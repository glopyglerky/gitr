use gitr_test_support::{guest, Guest};
use super::*;
use std::sync::{Arc, Barrier};
use std::thread;

fn success_module() -> Vec<u8> {
    guest(Guest::Echo)
}
fn input(bytes: &[u8]) -> SnapshotInput {
    SnapshotInput {
        commit: "1".repeat(40),
        blob: "2".repeat(40),
        path: "source.rs".into(),
        bytes: bytes.to_vec(),
    }
}
fn database() -> (tempfile::TempDir, ActionDb, String) {
    let temp = tempfile::tempdir().unwrap();
    let db = ActionDb::open_path(&temp.path().join("state.redb"), true).unwrap();
    let module = db.install(&success_module()).unwrap();
    (temp, db, module)
}

#[test]
fn stored_wasm_and_inputs_survive_reopen_and_repeat_returns_original_receipt() {
    let (temp, db, module) = database();
    db.submit("build", &module, input(b"source bytes\0\xff"))
        .unwrap();
    drop(db);
    let db = ActionDb::open_path(&temp.path().join("state.redb"), false).unwrap();
    let receipt = db.run_once("build").unwrap();
    assert_eq!(receipt.status, Status::Passed);
    assert_eq!(db.artifact("build").unwrap(), b"source bytes\0\xff");
    assert_eq!(db.run_once("build").unwrap(), receipt);
    assert_eq!(
        db.submit("build", &module, input(b"source bytes\0\xff"))
            .unwrap(),
        receipt
    );
    assert_eq!(receipt.attempt, 1);
}

#[test]
fn idempotency_rejects_changed_inputs_modules_and_snapshot_identity() {
    let (_temp, db, module) = database();
    db.submit("key", &module, input(b"one")).unwrap();
    assert!(db.submit("key", &module, input(b"two")).is_err());
    let mut other_commit = input(b"one");
    other_commit.commit = "3".repeat(40);
    assert!(db.submit("key", &module, other_commit).is_err());
    let other_module = db.install(&guest(Guest::Empty)).unwrap();
    assert!(db.submit("key", &other_module, input(b"one")).is_err());
    assert_eq!(db.get("key").unwrap().status, Status::Queued);
}

#[test]
fn live_claims_are_exclusive_and_expired_workers_are_fenced() {
    let (_temp, db, module) = database();
    db.submit("job", &module, input(b"v1")).unwrap();
    let old = db.claim("job", || Ok(100)).unwrap().unwrap();
    assert!(db.claim("job", || Ok(101)).unwrap().is_none());
    let new = db.claim("job", || Ok(100 + LEASE_MS)).unwrap().unwrap();
    assert_eq!(new.attempt, old.attempt + 1);
    assert!(
        db.finish(&old, Ok((0, b"stale".to_vec())), || Ok(101 + LEASE_MS))
            .is_err()
    );
    assert!(db.artifact("job").is_err());
    let result = db
        .finish(&new, Ok((0, b"winner".to_vec())), || Ok(101 + LEASE_MS))
        .unwrap();
    assert_eq!(result.status, Status::Passed);
    assert_eq!(db.artifact("job").unwrap(), b"winner");
    assert!(
        db.finish(&old, Ok((0, b"stale".to_vec())), || Ok(102 + LEASE_MS))
            .is_err()
    );
}

#[test]
fn cancellation_fences_running_work_without_rewriting_terminal_results() {
    let (_temp, db, module) = database();
    db.submit("job", &module, input(b"data")).unwrap();
    let claim = db.claim("job", || Ok(0)).unwrap().unwrap();
    assert_eq!(db.cancel("job").unwrap().status, Status::Cancelled);
    assert!(db.finish(&claim, Ok((0, vec![1])), || Ok(1)).is_err());
    assert!(db.artifact("job").is_err());
    assert!(db.claim("job", || Ok(LEASE_MS * 2)).unwrap().is_none());
    db.submit("passed", &module, input(b"data")).unwrap();
    let result = db.run_once("passed").unwrap();
    assert_eq!(db.cancel("passed").unwrap(), result);
}

#[test]
fn only_one_of_concurrent_local_handles_can_claim() {
    let (temp, db, module) = database();
    db.submit("race", &module, input(b"data")).unwrap();
    let path = temp.path().join("state.redb");
    let handles: Vec<_> = (0..4)
        .map(|_| ActionDb::open_path(&path, false).unwrap())
        .collect();
    let barrier = Arc::new(Barrier::new(handles.len()));
    let threads: Vec<_> = handles
        .into_iter()
        .map(|db| {
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                db.claim("race", || Ok(0)).unwrap().is_some()
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
fn abandoned_claim_recovers_after_reopen_and_another_job_can_write() {
    let (temp, db, module) = database();
    db.submit("abandoned", &module, input(b"a")).unwrap();
    let old = db.claim("abandoned", || Ok(0)).unwrap().unwrap();
    // A claimed guest has no database write transaction: another job can submit.
    db.submit("independent", &module, input(b"b")).unwrap();
    assert_eq!(db.run_once("independent").unwrap().status, Status::Passed);
    drop(db);
    let db = ActionDb::open_path(&temp.path().join("state.redb"), false).unwrap();
    assert!(
        db.claim("abandoned", || Ok(LEASE_MS - 1))
            .unwrap()
            .is_none()
    );
    let new = db.claim("abandoned", || Ok(LEASE_MS)).unwrap().unwrap();
    assert!(new.attempt > old.attempt);
}

#[test]
fn failure_and_trap_discard_partial_outputs() {
    for kind in [Guest::Failure(7), Guest::Trap] {
        let code = guest(kind);
        let (_temp, db, _) = database();
        let module = db.install(&code).unwrap();
        db.submit("fail", &module, input(b"")).unwrap();
        let receipt = db.run_once("fail").unwrap();
        assert_eq!(receipt.status, Status::Failed);
        assert!(receipt.artifact_sha256.is_none());
        assert!(db.artifact("fail").is_err());
        let tx = db.db.begin_read().unwrap();
        assert!(
            tx.open_table(OBJECTS)
                .unwrap()
                .get(digest(b"partial").as_str())
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn fuel_limits_infinite_guest_loops() {
    let code = guest(Guest::Spin);
    assert!(execute(&code, vec![]).is_err());
}

#[test]
fn memory_growth_and_guest_host_buffers_are_bounded() {
    let grow = guest(Guest::Grow);
    assert!(execute(&grow, vec![]).is_err());
    for (pointer, length) in [(-1, 1), (0, -1), (65536, 1), (0, MAX_BYTES as i32 + 1)] {
        let code = guest(Guest::Buffer(pointer,length));
        assert!(execute(&code, vec![]).is_err());
    }
}

#[test]
fn capability_imports_start_functions_and_wrong_exports_are_rejected() {
    for kind in [Guest::Wasi,Guest::Network,Guest::WrongImport,Guest::Start,Guest::WrongExport,Guest::Huge] {
        assert!(prepare(&guest(kind)).is_err(), "accepted {kind:?}");
    }
    assert!(prepare(b"not wasm").is_err());
    assert!(prepare(&vec![0; MAX_BYTES + 1]).is_err());
}

#[test]
fn host_call_and_output_budgets_are_enforced() {
    let calls = guest(Guest::Calls);
    assert!(execute(&calls, vec![]).is_err());
    let output = guest(Guest::Output);
    assert!(execute(&output, vec![]).is_err());
}

#[test]
fn artifact_integrity_and_invalid_control_inputs_fail_closed() {
    let (_temp, db, module) = database();
    db.submit("job", &module, input(b"original")).unwrap();
    db.run_once("job").unwrap();
    let id = db.get("job").unwrap().artifact_sha256.unwrap();
    let tx = db.db.begin_write().unwrap();
    tx.open_table(OBJECTS)
        .unwrap()
        .insert(id.as_str(), b"corrupt".as_slice())
        .unwrap();
    tx.commit().unwrap();
    assert!(db.artifact("job").is_err());
    for invalid in ["", "../job", "a/b", "a\n"] {
        assert!(db.get(invalid).is_err());
    }
    assert!(
        serde_json::from_str::<Request>(
            r#"{"operation":"status","job":"job","database":"/etc/passwd"}"#
        )
        .is_err()
    );
    assert!(bounded_read(&vec![0; 33 * 1024][..], 32 * 1024).is_err());
}
