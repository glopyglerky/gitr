//! Adapter from the complete vendored Rbit planner to a narrow Gitr execution
//! profile. Parse support is deliberately broader than supported execution.
use super::*;
use rbit_workflow::{
    Deadline, Limits, RunContext,
    expr::Template,
    spec::{JobTemplate, StepAction},
};
use std::time::Duration;

fn templates(
    bytes: &[u8],
    commit: &str,
    deadline: &Deadline,
) -> Result<(Vec<JobTemplate>, RunContext)> {
    let limits = Limits {
        max_bytes: 256 * 1024,
        max_depth: 32,
        max_nodes: 16_384,
        max_aliases: 64,
        max_jobs: MAX_TASKS,
        max_combinations: MAX_TASKS,
        max_run_jobs: MAX_TASKS,
        max_timeout_minutes: 1,
    };
    deadline.check()?;
    let workflow = rbit_workflow::parse(bytes, &limits, deadline)?;
    let on = &workflow.on;
    ensure!(
        on.workflow_dispatch
            .as_ref()
            .is_some_and(|dispatch| dispatch.inputs.is_empty())
            && on.push.is_none()
            && on.pull_request.is_none()
            && on.schedule.is_empty()
            && on.workflow_call.is_none()
            && on.workflow_run.is_none()
            && on.repository_dispatch.is_none()
            && on.other.is_empty(),
        "this execution profile requires only workflow_dispatch with no inputs"
    );
    ensure!(
        workflow.run_name.is_none()
            && workflow.env.is_empty()
            && workflow.defaults == rbit_workflow::spec::RunDefaults::default()
            && workflow.permissions.is_none()
            && workflow.concurrency.is_none(),
        "workflow env, defaults, permissions, run-name and concurrency are not supported by this profile"
    );
    for job in &workflow.jobs {
        ensure!(
            job.condition.is_none()
                && job.permissions.is_none()
                && job.env.is_empty()
                && job.defaults == rbit_workflow::spec::RunDefaults::default()
                && job.outputs.is_empty()
                && job.timeout_minutes.is_none()
                && job.continue_on_error.is_none()
                && job.concurrency.is_none()
                && job.container.is_none()
                && job.services.is_empty()
                && job.environment.is_none()
                && job.call.is_none(),
            "job {} requests unsupported execution semantics",
            job.id
        );
    }
    let mut context = RunContext::default();
    context.github.insert("sha".into(), commit.into());
    context
        .github
        .insert("event_name".into(), "workflow_dispatch".into());
    let jobs = rbit_workflow::plan(&workflow, &context, &limits, deadline)?;
    ensure!(
        !jobs.is_empty() && jobs.len() <= MAX_TASKS,
        "workflow must expand to 1..32 jobs"
    );
    for job in &jobs {
        ensure!(
            !job.deferred && job.calls.is_empty() && !job.continue_on_error,
            "deferred/reusable workflows and continue-on-error are not supported"
        );
        ensure!(
            job.runs_on == ["gitr-wasmi-v1"],
            "job {} must select gitr-wasmi-v1",
            job.job_id
        );
        ensure!(
            job.steps.len() == 1,
            "job {} must contain exactly one bounded Wasm step",
            job.job_id
        );
        let step = &job.steps[0];
        ensure!(
            step.condition.is_none()
                && step.shell.is_none()
                && step.working_directory.is_none()
                && step.env.is_empty()
                && step.continue_on_error.is_none()
                && step.timeout_minutes.is_none(),
            "step requests unsupported execution semantics"
        );
        let StepAction::Action(action) = &step.action else {
            bail!(
                "only the reserved gitr/wasm@v1 action is supported; shell/built-in actions are not executed"
            );
        };
        ensure!(
            action.uses == "gitr/wasm@v1"
                && action.with.len() == 2
                && action.with.contains_key("module")
                && action.with.contains_key("path"),
            "expected gitr/wasm@v1 with exactly module and path inputs"
        );
    }
    Ok((jobs, context))
}

pub(super) fn prepare(
    repo: &gix::Repository,
    id: &str,
    revision: &str,
    path: &str,
) -> Result<Prepared> {
    key(id)?;
    // Resolve exactly once, then every input is read against that immutable ID.
    let workflow = SnapshotInput::from_git(repo, revision, path)?;
    let deadline = Deadline::after(Duration::from_secs(2));
    let (jobs, context) = templates(&workflow.bytes, &workflow.commit, &deadline)?;
    let mut groups: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for (index, job) in jobs.iter().enumerate() {
        groups
            .entry(&job.job_id)
            .or_default()
            .push(child_id(id, index));
    }
    let mut tasks = Vec::new();
    let mut inputs = Vec::new();
    let mut total = workflow.bytes.len();
    for (index, job) in jobs.iter().enumerate() {
        deadline.check()?;
        let StepAction::Action(action) = &job.steps[0].action else {
            bail!("validated workflow action changed");
        };
        let contexts = context.contexts().with("matrix", job.matrix.clone()).with("strategy", serde_json::json!({
            "fail-fast":job.strategy.fail_fast, "max-parallel":job.strategy.max_parallel.unwrap_or(job.strategy.job_total),
            "job-index":job.strategy.job_index,"job-total":job.strategy.job_total
        }));
        let module = Template::parse(&action.with["module"])?.render(&contexts)?;
        oid(&module)?;
        let path = Template::parse(&action.with["path"])?.render(&contexts)?;
        let source = SnapshotInput::from_git(repo, &workflow.commit, &path)?;
        total = total
            .checked_add(source.bytes.len())
            .context("workflow input byte overflow")?;
        ensure!(
            total <= 8 * MAX_BYTES,
            "workflow aggregate inputs exceed 8 MiB"
        );
        let mut needs = Vec::new();
        for need in &job.needs {
            needs.extend(
                groups
                    .get(need.as_str())
                    .context("dependency expands to no executable jobs")?
                    .iter()
                    .cloned(),
            );
        }
        needs.sort();
        needs.dedup();
        tasks.push(Task {
            action_id: child_id(id, index),
            job_key: job.job_id.clone(),
            name: job.name.clone(),
            matrix: job.matrix.clone().into(),
            needs,
            fail_fast: job.strategy.fail_fast,
            max_parallel: job
                .strategy
                .max_parallel
                .unwrap_or(job.strategy.job_total)
                .min(MAX_TASKS as u32),
            spec: Spec {
                profile: MANAGED_PROFILE.into(),
                module_sha256: module,
                commit_oid: source.commit,
                blob_oid: source.blob,
                path: source.path,
                input_sha256: digest(&source.bytes),
            },
        });
        inputs.push(source.bytes);
    }
    let plan = Plan {
        id: id.into(),
        planner: PLANNER.into(),
        commit_oid: workflow.commit,
        workflow_blob: workflow.blob,
        workflow_path: workflow.path,
        workflow_sha256: digest(&workflow.bytes),
        tasks,
    };
    validate(&plan)?;
    deadline.check()?;
    Ok(Prepared {
        plan,
        workflow: workflow.bytes,
        inputs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn yaml(extra: &str, step: &str) -> Vec<u8> {
        format!("on: workflow_dispatch\njobs:\n  check:\n    runs-on: gitr-wasmi-v1\n{extra}    steps:\n{step}\n").into_bytes()
    }
    fn wasm_step() -> String {
        format!(
            "      - uses: gitr/wasm@v1\n        with:\n          module: '{}'\n          path: source.rs",
            "a".repeat(64)
        )
    }
    #[test]
    fn upstream_planner_expands_matrix_and_preserves_dependency_keys() {
        let source = format!(
            "on: workflow_dispatch\njobs:\n  check:\n    runs-on: gitr-wasmi-v1\n    strategy:\n      matrix:\n        file: [a.rs, b.rs]\n      max-parallel: 1\n    steps:\n{}\n  follow:\n    needs: check\n    runs-on: gitr-wasmi-v1\n    steps:\n{}\n",
            wasm_step(),
            wasm_step()
        );
        let (jobs, _) = templates(source.as_bytes(), &"1".repeat(40), &Deadline::NONE).unwrap();
        assert_eq!(jobs.len(), 3);
        assert_eq!(jobs[0].strategy.max_parallel, Some(1));
        assert_eq!(jobs[2].needs, ["check"]);
    }
    #[test]
    fn unsupported_semantics_fail_instead_of_being_ignored() {
        for extra in [
            "    env: {PASSWORD: plaintext}\n",
            "    permissions: write-all\n",
            "    if: always()\n",
            "    timeout-minutes: 60\n",
            "    continue-on-error: true\n",
            "    container: ubuntu\n",
            "    environment: production\n",
        ] {
            assert!(
                templates(&yaml(extra, &wasm_step()), &"1".repeat(40), &Deadline::NONE).is_err(),
                "accepted {extra}"
            );
        }
        for step in [
            "      - run: echo unsafe",
            "      - uses: actions/checkout@v4",
            "      - uses: owner/other@main",
        ] {
            assert!(templates(&yaml("", step), &"1".repeat(40), &Deadline::NONE).is_err());
        }
        let source = String::from_utf8(yaml("", &wasm_step()))
            .unwrap()
            .replace("workflow_dispatch", "push");
        assert!(templates(source.as_bytes(), &"1".repeat(40), &Deadline::NONE).is_err());
    }
    #[test]
    fn budgets_cycles_and_unrepresentable_deadlines_fail_closed() {
        assert!(
            templates(
                &vec![b'x'; 256 * 1024 + 1],
                &"1".repeat(40),
                &Deadline::NONE
            )
            .is_err()
        );
        assert!(
            templates(
                &yaml("    needs: check\n", &wasm_step()),
                &"1".repeat(40),
                &Deadline::NONE
            )
            .is_err()
        );
        assert!(
            templates(
                &yaml("", &wasm_step()),
                &"1".repeat(40),
                &Deadline::after(Duration::MAX)
            )
            .is_err()
        );
        let values = (0..33).map(|n| n.to_string()).collect::<Vec<_>>().join(",");
        assert!(
            templates(
                &yaml(
                    &format!("    strategy:\n      matrix:\n        n: [{values}]\n"),
                    &wasm_step()
                ),
                &"1".repeat(40),
                &Deadline::NONE
            )
            .is_err()
        );
    }
}
