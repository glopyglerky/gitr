use super::*;
use std::sync::{Arc, Barrier};

fn database() -> (tempfile::TempDir, ActionDb, String) {
    let dir = tempfile::tempdir().unwrap();
    let db = ActionDb::open_path(&dir.path().join("state.redb"), true).unwrap();
    let module = db.install(&wat::parse_str(r#"(module (memory (export "memory") 1) (func (export "run") (result i32) i32.const 0))"#).unwrap()).unwrap();
    (dir, db, module)
}
fn prepared(id: &str, module: &str, count: usize) -> Prepared {
    let workflow = b"committed workflow fixture".to_vec();
    let inputs: Vec<_> = (0..count)
        .map(|index| format!("source {index}").into_bytes())
        .collect();
    let tasks = inputs
        .iter()
        .enumerate()
        .map(|(index, bytes)| Task {
            action_id: child_id(id, index),
            job_key: format!("job{index}"),
            name: format!("Job {index}"),
            matrix: serde_json::json!({}),
            needs: vec![],
            fail_fast: true,
            max_parallel: 1,
            spec: Spec {
                profile: MANAGED_PROFILE.into(),
                module_sha256: module.into(),
                commit_oid: "1".repeat(40),
                blob_oid: format!("{index:040x}"),
                path: format!("source{index}.rs"),
                input_sha256: digest(bytes),
            },
        })
        .collect();
    Prepared {
        plan: Plan {
            id: id.into(),
            planner: PLANNER.into(),
            commit_oid: "1".repeat(40),
            workflow_blob: "2".repeat(40),
            workflow_path: ".gitr/workflows/check.yml".into(),
            workflow_sha256: digest(&workflow),
            tasks,
        },
        workflow,
        inputs,
    }
}

#[test]
fn direct_actions_cannot_bypass_dependencies_and_terminal_retries_are_identical() {
    let (_dir, db, module) = database();
    let mut p = prepared("dag", &module, 2);
    p.plan.tasks[1].needs = vec![child_id("dag", 0)];
    db.submit_workflow(p).unwrap();
    let blocked = db.run_once(&child_id("dag", 1)).unwrap();
    assert_eq!(blocked.status, Status::Queued);
    assert_eq!(blocked.attempt, 0);
    let done = db.workflow_run("dag").unwrap();
    assert_eq!(done.status, RunStatus::Passed);
    assert!(done.jobs.iter().all(|j| j.attempt == 1));
    assert_eq!(db.workflow_run("dag").unwrap(), done);
    assert_eq!(db.workflow_cancel("dag").unwrap(), done);
}

#[test]
fn run_identity_is_immutable_and_survives_database_reopen() {
    let (dir, db, module) = database();
    let first = db.submit_workflow(prepared("run", &module, 2)).unwrap();
    assert_eq!(
        db.submit_workflow(prepared("run", &module, 2)).unwrap(),
        first
    );
    let mut different = prepared("run", &module, 2);
    different.plan.tasks[0].needs = vec![child_id("run", 1)];
    assert!(db.submit_workflow(different).is_err());
    assert_eq!(db.workflow_status("run").unwrap(), first);
    drop(db);
    let reopened = ActionDb::open_path(&dir.path().join("state.redb"), false).unwrap();
    assert_eq!(reopened.workflow_status("run").unwrap(), first);
    assert_eq!(
        reopened.workflow_run("run").unwrap().status,
        RunStatus::Passed
    );
}

#[test]
fn cancellation_fences_old_workers_and_propagates_without_guest_execution() {
    let (_dir, db, module) = database();
    let mut p = prepared("cancel", &module, 2);
    p.plan.tasks[1].needs = vec![child_id("cancel", 0)];
    db.submit_workflow(p).unwrap();
    let claim = db
        .claim(&child_id("cancel", 0), || Ok(100))
        .unwrap()
        .unwrap();
    let result = db.workflow_cancel("cancel").unwrap();
    assert_eq!(result.status, RunStatus::Cancelled);
    assert!(result.jobs.iter().all(|j| j.status == Status::Cancelled));
    assert!(db.finish(&claim, Ok((0, vec![1])), || Ok(101)).is_err());
    assert_eq!(db.workflow_run("cancel").unwrap(), result);
}

#[test]
fn failure_atomically_fences_matrix_siblings_and_dependent_jobs() {
    let (_dir, db, module) = database();
    let mut p = prepared("matrix", &module, 3);
    for task in &mut p.plan.tasks[..2] {
        task.job_key = "checks".into();
        task.max_parallel = 2;
    }
    p.plan.tasks[2].needs = vec![child_id("matrix", 0), child_id("matrix", 1)];
    db.submit_workflow(p).unwrap();
    let first = db
        .claim(&child_id("matrix", 0), || Ok(10))
        .unwrap()
        .unwrap();
    let second = db
        .claim(&child_id("matrix", 1), || Ok(10))
        .unwrap()
        .unwrap();
    db.finish(&first, Ok((1, b"discard".to_vec())), || Ok(11))
        .unwrap();
    let result = db.workflow_status("matrix").unwrap();
    assert_eq!(result.status, RunStatus::Failed);
    assert_eq!(result.jobs[0].status, Status::Failed);
    assert_eq!(result.jobs[1].status, Status::Cancelled);
    assert_eq!(result.jobs[2].status, Status::Cancelled);
    assert_eq!(result.jobs[2].attempt, 0);
    assert!(db.finish(&second, Ok((0, vec![])), || Ok(12)).is_err());
}

#[test]
fn disabled_fail_fast_allows_independent_siblings_but_never_failed_dependencies() {
    let (_dir, db, module) = database();
    let mut p = prepared("independent", &module, 3);
    for task in &mut p.plan.tasks[..2] {
        task.job_key = "checks".into();
        task.fail_fast = false;
    }
    p.plan.tasks[2].needs = vec![child_id("independent", 0)];
    db.submit_workflow(p).unwrap();
    let first = db
        .claim(&child_id("independent", 0), || Ok(10))
        .unwrap()
        .unwrap();
    db.finish(&first, Ok((1, vec![])), || Ok(11)).unwrap();
    assert_eq!(
        db.get(&child_id("independent", 1)).unwrap().status,
        Status::Queued
    );
    assert_eq!(
        db.get(&child_id("independent", 2)).unwrap().status,
        Status::Cancelled
    );
    assert_eq!(
        db.workflow_run("independent").unwrap().jobs[1].status,
        Status::Passed
    );
}

#[test]
fn concurrent_matrix_claims_respect_max_parallel_across_database_handles() {
    let (dir, db, module) = database();
    let mut p = prepared("parallel", &module, 4);
    for task in &mut p.plan.tasks {
        task.job_key = "checks".into();
    }
    db.submit_workflow(p).unwrap();
    let handles: Vec<_> = (0..4)
        .map(|_| ActionDb::open_path(&dir.path().join("state.redb"), false).unwrap())
        .collect();
    let barrier = Arc::new(Barrier::new(4));
    let threads: Vec<_> = handles
        .into_iter()
        .enumerate()
        .map(|(index, db)| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                db.claim(&child_id("parallel", index), || Ok(100))
                    .unwrap()
                    .is_some()
            })
        })
        .collect();
    assert_eq!(
        threads
            .into_iter()
            .map(|t| t.join().unwrap())
            .filter(|v| *v)
            .count(),
        1
    );
}

#[test]
fn expired_claim_recovery_changes_generation_and_old_completion_cannot_publish() {
    let (dir, db, module) = database();
    db.submit_workflow(prepared("recovery", &module, 1))
        .unwrap();
    let old = db
        .claim(&child_id("recovery", 0), || Ok(100))
        .unwrap()
        .unwrap();
    drop(db);
    let db = ActionDb::open_path(&dir.path().join("state.redb"), false).unwrap();
    assert!(
        db.claim(&old.id, || Ok(100 + LEASE_MS - 1))
            .unwrap()
            .is_none()
    );
    let new = db.claim(&old.id, || Ok(100 + LEASE_MS)).unwrap().unwrap();
    assert_eq!(new.attempt, old.attempt + 1);
    assert!(
        db.finish(&old, Ok((0, b"stale".to_vec())), || Ok(101 + LEASE_MS))
            .is_err()
    );
    db.finish(&new, Ok((0, b"winner".to_vec())), || Ok(101 + LEASE_MS))
        .unwrap();
    assert_eq!(db.artifact(&old.id).unwrap(), b"winner");
}

#[test]
fn colliding_standalone_action_is_not_adopted_or_overwritten() {
    let (_dir, db, module) = database();
    let id = child_id("collision", 1);
    let existing = db
        .submit(
            &id,
            &module,
            SnapshotInput {
                commit: "1".repeat(40),
                blob: "2".repeat(40),
                path: "other.rs".into(),
                bytes: b"other".to_vec(),
            },
        )
        .unwrap();
    assert!(
        db.submit_workflow(prepared("collision", &module, 2))
            .is_err()
    );
    assert!(db.get(&child_id("collision", 0)).is_err());
    assert_eq!(db.get(&id).unwrap(), existing);
    assert!(db.workflow_status("collision").is_err());
}

#[test]
fn failure_after_partial_object_writes_rolls_back_the_entire_workflow() {
    let (_dir, db, module) = database();
    let p = prepared("rollback", &module, 2);
    let source_id = p.plan.workflow_sha256.clone();
    let corrupt_id = p.plan.tasks[1].spec.input_sha256.clone();
    let tx = db.db.begin_write().unwrap();
    tx.open_table(OBJECTS)
        .unwrap()
        .insert(corrupt_id.as_str(), b"corrupt".as_slice())
        .unwrap();
    tx.commit().unwrap();
    assert!(db.submit_workflow(p).is_err());
    assert!(db.object(&source_id).is_err());
    assert!(db.get(&child_id("rollback", 0)).is_err());
    assert!(db.workflow_status("rollback").is_err());
    let tx = db.db.begin_read().unwrap();
    assert!(
        tx.open_table(META)
            .unwrap()
            .get("workflow_schema")
            .unwrap()
            .is_none()
    );
}

#[test]
fn managed_profile_and_ownership_are_mandatory_even_without_planner_feature() {
    let (_dir, db, module) = database();
    db.submit_workflow(prepared("owner", &module, 1)).unwrap();
    let id = child_id("owner", 0);
    assert_ne!(db.get(&id).unwrap().spec.profile, PROFILE);
    let tx = db.db.begin_write().unwrap();
    tx.open_table(OWNERS).unwrap().remove(id.as_str()).unwrap();
    tx.commit().unwrap();
    assert!(db.run_once(&id).is_err());
    let mut bytes = Vec::new();
    assert!(db.export(&mut bytes).is_err());
    assert!(bytes.is_empty());
}

#[test]
fn snapshot_export_retains_workflow_plan_ownership_and_exact_source() {
    let (dir, db, module) = database();
    db.submit_workflow(prepared("export", &module, 2)).unwrap();
    let result = db.workflow_run("export").unwrap();
    let mut first = Vec::new();
    db.export(&mut first).unwrap();
    let records: Vec<serde_json::Value> = first
        .split(|b| *b == b'\n')
        .filter(|b| !b.is_empty())
        .map(|b| serde_json::from_slice(b).unwrap())
        .collect();
    assert_eq!(records[0]["format"], "gitr-action-export-v2");
    assert_eq!(records.last().unwrap()["workflows"], 1);
    assert_eq!(
        records
            .iter()
            .filter(|r| r["kind"] == "workflow_owner")
            .count(),
        2
    );
    let stored = records.iter().find(|r| r["kind"] == "workflow").unwrap();
    assert_eq!(stored["record"]["plan_sha256"], result.plan_sha256);
    drop(db);
    let db = ActionDb::open_path(&dir.path().join("state.redb"), false).unwrap();
    let mut second = Vec::new();
    db.export(&mut second).unwrap();
    assert_eq!(first, second);
}

#[test]
fn malformed_or_cyclic_stored_plans_fail_closed() {
    let (_dir, db, module) = database();
    let mut p = prepared("cycle", &module, 2);
    p.plan.tasks[0].needs = vec![child_id("cycle", 1)];
    p.plan.tasks[1].needs = vec![child_id("cycle", 0)];
    assert!(db.submit_workflow(p).is_err());
    let mut p = prepared("oversized", &module, 1);
    p.plan.tasks[0].name = "x".repeat(MAX_RUN_BYTES + 1);
    assert!(db.submit_workflow(p).is_err());
}
