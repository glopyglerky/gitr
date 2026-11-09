//! Durable, repository-local workflow runs over the existing action database.
//!
//! The planner is optional, but claim gates compile into every new binary. Managed
//! jobs use a distinct execution profile so older binaries refuse to execute them.
//! No workflow operation grants a guest additional host capabilities.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

pub(super) const MANAGED_PROFILE: &str =
    "gitr-workflow-action-v1/wasmi-2.0.0/fuel-10000000/memory-16MiB/io-8MiB/calls-4096";
pub(super) const RUNS: TableDefinition<&str, &[u8]> = TableDefinition::new("actions.workflows.v1");
pub(super) const OWNERS: TableDefinition<&str, &str> =
    TableDefinition::new("actions.workflow-owners.v1");
pub const PLANNER: &str = "gitr-wasmi-workflow-v1/rbit-39f932a00b7aa35b324e06a9ff2343a57d4bed76";
pub const MAX_TASKS: usize = 32;
const MAX_RUN_BYTES: usize = 1024 * 1024;

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Plan a committed workflow without writing the database or executing code.
    #[cfg(feature = "workflows")]
    Plan {
        run: String,
        #[arg(long, default_value = "HEAD")]
        revision: String,
        #[arg(long)]
        path: String,
    },
    /// Atomically persist an immutable workflow and all its action jobs.
    #[cfg(feature = "workflows")]
    Submit {
        run: String,
        #[arg(long, default_value = "HEAD")]
        revision: String,
        #[arg(long)]
        path: String,
    },
    /// Drain ready jobs, or return when other workers hold the remaining work.
    Run { run: String },
    /// Read one consistent workflow/job snapshot.
    Status { run: String },
    /// Atomically cancel all unfinished jobs and fence their active workers.
    Cancel { run: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub action_id: String,
    pub job_key: String,
    pub name: String,
    pub matrix: serde_json::Value,
    pub needs: Vec<String>,
    pub fail_fast: bool,
    pub max_parallel: u32,
    pub spec: Spec,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub id: String,
    pub planner: String,
    pub commit_oid: String,
    pub workflow_blob: String,
    pub workflow_path: String,
    pub workflow_sha256: String,
    pub tasks: Vec<Task>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredRun {
    pub version: u32,
    pub plan_sha256: String,
    pub plan: Plan,
    pub cancelled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Pending,
    Running,
    Passed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Report {
    pub id: String,
    pub plan_sha256: String,
    pub status: RunStatus,
    pub plan: Plan,
    pub jobs: Vec<Job>,
}

#[cfg(feature = "workflows")]
#[path = "actions_workflow_plan.rs"]
mod planner;

#[cfg(any(feature = "workflows", test))]
struct Prepared {
    plan: Plan,
    workflow: Vec<u8>,
    inputs: Vec<Vec<u8>>,
}

fn child_id(run: &str, index: usize) -> String {
    format!("wf.{}.{index}", digest(run.as_bytes()))
}

fn validate(plan: &Plan) -> Result<()> {
    key(&plan.id)?;
    ensure!(
        plan.planner == PLANNER,
        "unsupported workflow planner profile"
    );
    ensure!(
        !plan.tasks.is_empty() && plan.tasks.len() <= MAX_TASKS,
        "invalid workflow task count"
    );
    oid(&plan.workflow_sha256)?;
    ensure!(
        [40, 64].contains(&plan.commit_oid.len())
            && plan.commit_oid.bytes().all(|c| c.is_ascii_hexdigit()),
        "invalid workflow commit identity"
    );
    ensure!(
        plan.workflow_blob.len() == plan.commit_oid.len()
            && plan.workflow_blob.bytes().all(|c| c.is_ascii_hexdigit()),
        "invalid workflow blob identity"
    );
    let ids: BTreeSet<_> = plan
        .tasks
        .iter()
        .map(|task| task.action_id.as_str())
        .collect();
    ensure!(
        ids.len() == plan.tasks.len(),
        "duplicate workflow action identity"
    );
    for (index, task) in plan.tasks.iter().enumerate() {
        ensure!(
            task.action_id == child_id(&plan.id, index),
            "invalid workflow action identity"
        );
        key(&task.job_key)?;
        ensure!(
            task.spec.profile == MANAGED_PROFILE && task.spec.commit_oid == plan.commit_oid,
            "workflow action provenance mismatch"
        );
        oid(&task.spec.module_sha256)?;
        oid(&task.spec.input_sha256)?;
        ensure!(
            (1..=MAX_TASKS as u32).contains(&task.max_parallel),
            "invalid workflow parallelism"
        );
        ensure!(
            task.needs.len() <= MAX_TASKS
                && task
                    .needs
                    .iter()
                    .all(|id| id != &task.action_id && ids.contains(id.as_str())),
            "invalid workflow dependency"
        );
        for sibling in plan
            .tasks
            .iter()
            .filter(|other| other.job_key == task.job_key)
        {
            ensure!(
                sibling.fail_fast == task.fail_fast && sibling.max_parallel == task.max_parallel,
                "inconsistent matrix strategy"
            );
        }
    }
    let mut visited = BTreeSet::new();
    for _ in 0..plan.tasks.len() {
        for task in &plan.tasks {
            if task.needs.iter().all(|id| visited.contains(id)) {
                visited.insert(task.action_id.clone());
            }
        }
    }
    ensure!(
        visited.len() == plan.tasks.len(),
        "workflow contains a dependency cycle"
    );
    ensure!(
        serde_json::to_vec(plan)?.len() <= MAX_RUN_BYTES,
        "workflow plan exceeds byte limit"
    );
    Ok(())
}

fn decode(bytes: &[u8], id: &str) -> Result<StoredRun> {
    ensure!(
        bytes.len() <= MAX_RUN_BYTES,
        "workflow record exceeds byte limit"
    );
    let run: StoredRun = serde_json::from_slice(bytes)?;
    ensure!(
        run.version == 1 && run.plan.id == id,
        "workflow record identity mismatch"
    );
    validate(&run.plan)?;
    ensure!(
        run.plan_sha256 == digest(&serde_json::to_vec(&run.plan)?),
        "workflow plan integrity failure"
    );
    Ok(run)
}
fn read_run(tx: &redb::WriteTransaction, id: &str) -> Result<StoredRun> {
    key(id)?;
    let table = tx.open_table(RUNS)?;
    let bytes = table.get(id)?.context("unknown workflow run")?;
    decode(bytes.value(), id)
}
fn write_run(tx: &redb::WriteTransaction, run: &StoredRun) -> Result<()> {
    let bytes = serde_json::to_vec(run)?;
    ensure!(
        bytes.len() <= MAX_RUN_BYTES,
        "workflow record exceeds byte limit"
    );
    tx.open_table(RUNS)?
        .insert(run.plan.id.as_str(), bytes.as_slice())?;
    Ok(())
}
fn check_job(task: &Task, job: &Job) -> Result<()> {
    ensure!(
        job.id == task.action_id && job.spec == task.spec,
        "managed action does not match its immutable workflow plan"
    );
    Ok(())
}
fn jobs_in(tx: &redb::WriteTransaction, run: &StoredRun) -> Result<BTreeMap<String, Job>> {
    run.plan
        .tasks
        .iter()
        .map(|task| {
            let job = read_job(tx, &task.action_id)?;
            check_job(task, &job)?;
            let owners = tx.open_table(OWNERS)?;
            ensure!(
                owners
                    .get(task.action_id.as_str())?
                    .map(|v| v.value() == run.plan.id)
                    .unwrap_or(false),
                "workflow ownership mismatch"
            );
            Ok((task.action_id.clone(), job))
        })
        .collect()
}
fn active(job: &Job) -> bool {
    matches!(job.status, Status::Queued | Status::Running)
}

/// Propagate failure/cancellation and matrix fail-fast in the same writer as the
/// originating change. Cancellation removes leases, fencing old completions.
fn settle(tx: &redb::WriteTransaction, run: &StoredRun) -> Result<()> {
    let original = jobs_in(tx, run)?;
    let mut jobs = original.clone();
    for _ in 0..=run.plan.tasks.len() {
        let mut changed = false;
        for task in &run.plan.tasks {
            let blocked = task
                .needs
                .iter()
                .any(|id| matches!(jobs[id].status, Status::Failed | Status::Cancelled));
            let sibling_failed = task.fail_fast
                && run.plan.tasks.iter().any(|other| {
                    other.job_key == task.job_key && jobs[&other.action_id].status == Status::Failed
                });
            let job = jobs
                .get_mut(&task.action_id)
                .context("missing managed action")?;
            if active(job) && (run.cancelled || blocked || sibling_failed) {
                job.status = Status::Cancelled;
                job.lease_until_ms = None;
                job.failure = Some(
                    if run.cancelled {
                        "workflow cancelled"
                    } else if blocked {
                        "workflow dependency did not pass"
                    } else {
                        "workflow matrix fail-fast"
                    }
                    .into(),
                );
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    for (id, job) in jobs {
        if job != original[&id] {
            write_job(tx, &job)?;
        }
    }
    Ok(())
}

fn owner(tx: &redb::WriteTransaction, job: &Job) -> Result<StoredRun> {
    ensure!(
        tx.open_table(META)?
            .get("workflow_schema")?
            .map(|v| v.value())
            == Some(1),
        "managed action requires supported workflow metadata"
    );
    let table = tx.open_table(OWNERS)?;
    let id = table
        .get(job.id.as_str())?
        .context("managed action has no workflow owner")?;
    let run = read_run(tx, id.value())?;
    let task = run
        .plan
        .tasks
        .iter()
        .find(|task| task.action_id == job.id)
        .context("managed action absent from workflow")?;
    check_job(task, job)?;
    Ok(run)
}

pub(super) fn claim_allowed(tx: &redb::WriteTransaction, job: &Job, now: u64) -> Result<bool> {
    if job.spec.profile == PROFILE {
        return Ok(true);
    }
    ensure!(
        job.spec.profile == MANAGED_PROFILE,
        "unsupported action execution profile"
    );
    let run = owner(tx, job)?;
    settle(tx, &run)?;
    let jobs = jobs_in(tx, &run)?;
    let task = run
        .plan
        .tasks
        .iter()
        .find(|task| task.action_id == job.id)
        .context("unknown managed action")?;
    if run.cancelled
        || !active(&jobs[&job.id])
        || task
            .needs
            .iter()
            .any(|id| jobs[id].status != Status::Passed)
    {
        return Ok(false);
    }
    let running = run
        .plan
        .tasks
        .iter()
        .filter(|other| other.job_key == task.job_key && other.action_id != job.id)
        .filter(|other| {
            jobs[&other.action_id].status == Status::Running
                && jobs[&other.action_id]
                    .lease_until_ms
                    .is_some_and(|until| now < until)
        })
        .count();
    Ok(running < task.max_parallel as usize)
}

pub(super) fn after_change(tx: &redb::WriteTransaction, job: &Job) -> Result<()> {
    if job.spec.profile == MANAGED_PROFILE {
        settle(tx, &owner(tx, job)?)?;
    }
    Ok(())
}

fn report(run: StoredRun, jobs: Vec<Job>) -> Report {
    let status = if jobs.iter().any(|j| j.status == Status::Running) {
        RunStatus::Running
    } else if jobs.iter().any(|j| j.status == Status::Queued) {
        RunStatus::Pending
    } else if jobs.iter().any(|j| j.status == Status::Failed) {
        RunStatus::Failed
    } else if run.cancelled || jobs.iter().any(|j| j.status == Status::Cancelled) {
        RunStatus::Cancelled
    } else {
        RunStatus::Passed
    };
    Report {
        id: run.plan.id.clone(),
        plan_sha256: run.plan_sha256,
        status,
        plan: run.plan,
        jobs,
    }
}

impl ActionDb {
    #[cfg(any(feature = "workflows", test))]
    fn submit_workflow(&self, prepared: Prepared) -> Result<Report> {
        let Prepared {
            plan,
            workflow,
            inputs,
        } = prepared;
        validate(&plan)?;
        ensure!(
            digest(&workflow) == plan.workflow_sha256 && workflow.len() <= 256 * 1024,
            "workflow source integrity failure"
        );
        ensure!(
            inputs.len() == plan.tasks.len(),
            "workflow inputs do not match plan"
        );
        let mut total = workflow.len();
        let mut checked = BTreeSet::new();
        for (task, bytes) in plan.tasks.iter().zip(&inputs) {
            ensure!(
                bytes.len() <= MAX_BYTES && digest(bytes) == task.spec.input_sha256,
                "workflow input integrity failure"
            );
            total = total
                .checked_add(bytes.len())
                .context("workflow input size overflow")?;
            ensure!(
                total <= 8 * MAX_BYTES,
                "workflow aggregate inputs exceed 8 MiB"
            );
            if checked.insert(task.spec.module_sha256.clone()) {
                prepare(&self.object(&task.spec.module_sha256)?)?;
            }
        }
        let run = StoredRun {
            version: 1,
            plan_sha256: digest(&serde_json::to_vec(&plan)?),
            plan,
            cancelled: false,
        };
        let mut tx = self.db.begin_write()?;
        tx.set_durability(Durability::Immediate)?;
        let version = tx
            .open_table(META)?
            .get("workflow_schema")?
            .map(|v| v.value());
        ensure!(
            version.is_none() || version == Some(1),
            "unsupported workflow schema"
        );
        let existing = {
            let runs = tx.open_table(RUNS)?;
            runs.get(run.plan.id.as_str())?
                .map(|v| decode(v.value(), &run.plan.id))
                .transpose()?
        };
        if let Some(existing) = existing {
            ensure!(
                existing.plan == run.plan,
                "workflow idempotency key belongs to a different plan"
            );
            drop(tx);
            return self.workflow_status(&run.plan.id);
        }
        // Check every identity before writing anything; any later error also
        // aborts the entire transaction, including object and ownership inserts.
        for task in &run.plan.tasks {
            ensure!(
                tx.open_table(JOBS)?.get(task.action_id.as_str())?.is_none(),
                "workflow action identity already exists"
            );
            ensure!(
                tx.open_table(OWNERS)?
                    .get(task.action_id.as_str())?
                    .is_none(),
                "workflow action owner already exists"
            );
        }
        put_object(&tx, &run.plan.workflow_sha256, &workflow)?;
        for (task, bytes) in run.plan.tasks.iter().zip(&inputs) {
            // Recheck installed object integrity at publication as well.
            let objects = tx.open_table(OBJECTS)?;
            let module = objects
                .get(task.spec.module_sha256.as_str())?
                .context("workflow module disappeared")?;
            ensure!(
                digest(module.value()) == task.spec.module_sha256,
                "workflow module integrity failure"
            );
            drop(module);
            drop(objects);
            put_object(&tx, &task.spec.input_sha256, bytes)?;
            write_job(
                &tx,
                &Job {
                    id: task.action_id.clone(),
                    spec: task.spec.clone(),
                    status: Status::Queued,
                    attempt: 0,
                    lease_until_ms: None,
                    artifact_sha256: None,
                    exit_code: None,
                    failure: None,
                },
            )?;
            tx.open_table(OWNERS)?
                .insert(task.action_id.as_str(), run.plan.id.as_str())?;
        }
        write_run(&tx, &run)?;
        tx.open_table(META)?.insert("workflow_schema", 1)?;
        tx.commit()?;
        self.workflow_status(&run.plan.id)
    }

    pub fn workflow_status(&self, id: &str) -> Result<Report> {
        key(id)?;
        let tx = self.db.begin_read()?;
        ensure!(
            tx.open_table(META)?
                .get("workflow_schema")?
                .map(|v| v.value())
                == Some(1),
            "workflow store not initialized"
        );
        let runs = tx.open_table(RUNS)?;
        let bytes = runs.get(id)?.context("unknown workflow run")?;
        let run = decode(bytes.value(), id)?;
        let table = tx.open_table(JOBS)?;
        let owners = tx.open_table(OWNERS)?;
        let mut jobs = Vec::new();
        for task in &run.plan.tasks {
            let bytes = table
                .get(task.action_id.as_str())?
                .context("workflow action missing")?;
            let job = serde_json::from_slice(bytes.value())?;
            check_job(task, &job)?;
            ensure!(
                owners
                    .get(task.action_id.as_str())?
                    .map(|v| v.value() == id)
                    .unwrap_or(false),
                "workflow owner missing"
            );
            jobs.push(job);
        }
        Ok(report(run, jobs))
    }

    pub fn workflow_run(&self, id: &str) -> Result<Report> {
        let mut previous = self.workflow_status(id)?;
        // No background daemon or busy wait. A caller can retry after external
        // workers finish or their durable claims expire.
        for _ in 0..=previous.jobs.len() {
            for task in &previous.plan.tasks {
                self.run_once(&task.action_id)?;
            }
            let current = self.workflow_status(id)?;
            if current == previous
                || !matches!(current.status, RunStatus::Pending | RunStatus::Running)
            {
                return Ok(current);
            }
            previous = current;
        }
        Ok(previous)
    }

    pub fn workflow_cancel(&self, id: &str) -> Result<Report> {
        let mut tx = self.db.begin_write()?;
        tx.set_durability(Durability::Immediate)?;
        let mut run = read_run(&tx, id)?;
        if jobs_in(&tx, &run)?.values().any(active) {
            run.cancelled = true;
            write_run(&tx, &run)?;
            settle(&tx, &run)?;
            tx.commit()?;
        } else {
            drop(tx);
        }
        self.workflow_status(id)
    }
}

pub(super) fn run(repo: &gix::Repository, command: Command) -> Result<()> {
    match command {
        #[cfg(feature = "workflows")]
        Command::Plan {
            run,
            revision,
            path,
        } => print_json(&planner::prepare(repo, &run, &revision, &path)?.plan),
        #[cfg(feature = "workflows")]
        Command::Submit {
            run,
            revision,
            path,
        } => {
            let prepared = planner::prepare(repo, &run, &revision, &path)?;
            print_json(&ActionDb::open(repo, true)?.submit_workflow(prepared)?)
        }
        Command::Status { run } => print_json(&ActionDb::open(repo, false)?.workflow_status(&run)?),
        Command::Cancel { run } => print_json(&ActionDb::open(repo, false)?.workflow_cancel(&run)?),
        Command::Run { run } => {
            let result = ActionDb::open_for_cli(repo, false)?.workflow_run(&run)?;
            print_json(&result)?;
            ensure!(
                result.status == RunStatus::Passed,
                "workflow has not passed; inspect durable job states"
            );
            Ok(())
        }
    }
}

#[path = "actions_workflow_export.rs"]
mod snapshot;
pub(super) use snapshot::{export_preflight, export_records, verify_export_job};
#[cfg(test)]
#[path = "actions_workflow_tests.rs"]
mod tests;
