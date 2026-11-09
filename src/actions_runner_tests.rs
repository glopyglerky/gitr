use gitr_test_support::{guest, Guest};
use super::*;
use crate::actions::{ActionDb, SnapshotInput, Status};

fn fixture_command(name: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--ignored",
            "--exact",
            &format!("actions::runner::tests::{name}"),
            "--nocapture",
        ])
        .env("GITR_WORKER_FIXTURE", "1");
    command
}
fn active_fixture() -> bool {
    std::env::var("GITR_WORKER_FIXTURE").as_deref() == Ok("1")
}

#[test]
fn framed_request_roundtrip_and_limits_reject_before_payload_allocation() {
    let encoded = encode_request(1, b"module\0", b"input\xff").unwrap();
    let decoded = read_request(&mut encoded.as_slice()).unwrap();
    assert_eq!(decoded.module, b"module\0");
    assert_eq!(decoded.bytes, b"input\xff");
    assert_eq!(
        decoded.identity,
        request_identity(1, b"module\0", b"input\xff")
    );
    for end in 0..encoded.len() {
        assert!(read_request(&mut &encoded[..end]).is_err());
    }
    let mut bad = encoded.clone();
    bad[9..13].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(read_request(&mut bad.as_slice()).is_err());
    let mut bad = encoded;
    *bad.last_mut().unwrap() ^= 1;
    assert!(read_request(&mut bad.as_slice()).is_err());
    assert!(encode_request(2, b"", b"").is_err());
    assert!(encode_request(0, b"module", b"input").is_err());
    assert!(encode_request(1, &vec![0; MAX_BYTES + 1], b"").is_err());
}

#[test]
fn output_binds_the_exact_request_and_discards_failed_or_malformed_results() {
    let identity = request_identity(1, b"a", b"bc");
    assert_ne!(identity, request_identity(1, b"ab", b"c"));
    let valid = response(identity, 1, 0, b"artifact\0\xff").unwrap();
    assert_eq!(
        decode_response(&valid, 1, identity).unwrap(),
        (0, b"artifact\0\xff".to_vec())
    );
    assert!(decode_response(&valid, 1, [0; 32]).is_err());
    assert!(decode_response(&valid, 0, identity).is_err());
    for end in 0..valid.len() {
        assert!(decode_response(&valid[..end], 1, identity).is_err());
    }
    let mut bad = valid.clone();
    bad.push(0);
    assert!(decode_response(&bad, 1, identity).is_err());
    let mut bad = valid;
    *bad.last_mut().unwrap() ^= 1;
    assert!(decode_response(&bad, 1, identity).is_err());
    assert!(response(identity, 1, 7, b"partial").is_err());
    assert!(decode_response(&response(identity, 2, 0, b"").unwrap(), 1, identity).is_err());
    assert_eq!(
        decode_response(&response(identity, 1, -1, b"").unwrap(), 1, identity).unwrap(),
        (-1, vec![])
    );
}

#[test]
fn deadline_stops_a_child_that_never_reads_a_full_pipe_and_reaps_it() {
    let start = Instant::now();
    let error = supervise(
        fixture_command("fixture_no_read"),
        vec![0; 2 * MAX_BYTES],
        Duration::from_millis(150),
        || Ok(true),
    )
    .unwrap_err();
    assert!(error.to_string().contains("deadline"), "{error}");
    assert!(start.elapsed() < Duration::from_secs(5));
}

#[test]
fn cancellation_stops_a_running_child_without_waiting_for_its_output() {
    let start = Instant::now();
    let mut polls = 0;
    let error = supervise(
        fixture_command("fixture_no_read"),
        vec![0; 2 * MAX_BYTES],
        Duration::from_secs(4),
        || {
            polls += 1;
            Ok(polls < 5)
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("no longer current"), "{error}");
    assert!(start.elapsed() < Duration::from_secs(3));
}

#[test]
fn oversized_output_is_not_buffered_indefinitely_or_treated_as_success() {
    let start = Instant::now();
    assert!(
        supervise(
            fixture_command("fixture_large_output"),
            vec![],
            Duration::from_secs(3),
            || Ok(true)
        )
        .is_err()
    );
    assert!(start.elapsed() < Duration::from_secs(5));
}

#[test]
fn successful_stdout_does_not_count_until_the_child_exits_successfully() {
    assert!(
        supervise(
            fixture_command("fixture_failed_exit"),
            vec![],
            Duration::from_secs(3),
            || Ok(true)
        )
        .is_err()
    );
}

#[test]
fn worker_command_does_not_inherit_credentials_or_repository_working_directory() {
    let result = fixture_command("fixture_environment_parent")
        .env("GITR_API_TOKEN", "must-not-reach-child")
        .env("GIT_CONFIG_COUNT", "123")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("SANITIZATION_PASSED"));
}

#[test]
fn malformed_worker_response_becomes_failed_receipt_without_artifact() {
    let temp = tempfile::tempdir().unwrap();
    let db = ActionDb::open_path(&temp.path().join("state.redb"), true).unwrap();
    let module = guest(Guest::Empty);
    let module = db.install(&module).unwrap();
    db.submit(
        "job",
        &module,
        SnapshotInput {
            commit: "1".repeat(40),
            blob: "2".repeat(40),
            path: "source".into(),
            bytes: b"input".to_vec(),
        },
    )
    .unwrap();
    // This Rust test binary is NOT a Gitr worker: its successful 'zero tests'
    // stdout must fail the binary protocol, not become a successful artifact.
    let db =
        db.with_ephemeral_runner(EphemeralRunner::new(std::env::current_exe().unwrap()).unwrap());
    let receipt = db.run_once("job").unwrap();
    assert_eq!(receipt.status, Status::Failed);
    assert_eq!(receipt.attempt, 1);
    assert!(receipt.artifact_sha256.is_none());
    assert!(db.artifact("job").is_err());
    assert_eq!(db.run_once("job").unwrap(), receipt);
}

#[test]
fn ephemeral_runner_configuration_cannot_silently_extend_the_execution_budget() {
    assert!(EphemeralRunner::new("gitr").is_err());
    let runner = EphemeralRunner::new(std::env::current_exe().unwrap()).unwrap();
    assert!(runner.clone().with_timeout(Duration::ZERO).is_err());
    assert!(runner.clone().with_timeout(Duration::MAX).is_err());
    assert!(runner.with_timeout(Duration::from_millis(1)).is_ok());
}

// Controlled subprocess fixtures, invoked by the parent tests above. They return
// immediately when accidentally selected without the fixture environment marker.
#[test]
#[ignore = "controlled process fixture invoked by parent tests"]
fn fixture_no_read() {
    if !active_fixture() {
        return;
    }
    thread::sleep(Duration::from_secs(120));
}
#[test]
#[ignore = "controlled process fixture invoked by parent tests"]
fn fixture_large_output() {
    if !active_fixture() {
        return;
    }
    let _ = io::stdout().write_all(&vec![0; 4 * MAX_BYTES]);
    thread::sleep(Duration::from_secs(120));
}
#[test]
#[ignore = "controlled process fixture invoked by parent tests"]
fn fixture_failed_exit() {
    if !active_fixture() {
        return;
    }
    io::stdout()
        .write_all(b"looks successful but is not a receipt")
        .unwrap();
    std::process::exit(7);
}
#[test]
#[ignore = "controlled process fixture invoked by parent tests"]
fn fixture_environment_parent() {
    if !active_fixture() {
        return;
    }
    assert_eq!(
        std::env::var("GITR_API_TOKEN").unwrap(),
        "must-not-reach-child"
    );
    let directory = tempfile::tempdir().unwrap();
    let mut command = sanitized_command(&std::env::current_exe().unwrap(), directory.path());
    command
        .args([
            "--ignored",
            "--exact",
            "actions::runner::tests::fixture_environment_child",
            "--nocapture",
        ])
        .env("GITR_WORKER_FIXTURE", "1");
    let output = supervise(command, vec![], Duration::from_secs(3), || Ok(true)).unwrap();
    assert!(String::from_utf8_lossy(&output).contains("SANITIZATION_PASSED"));
    println!("SANITIZATION_PASSED");
}
#[test]
#[ignore = "controlled process fixture invoked by parent tests"]
fn fixture_environment_child() {
    if !active_fixture() {
        return;
    }
    assert!(std::env::var_os("GITR_API_TOKEN").is_none());
    assert!(std::env::var_os("GIT_CONFIG_COUNT").is_none());
    assert!(std::env::var_os("HOME").is_none());
    assert!(fs::read_dir(".").unwrap().next().is_none());
    println!("SANITIZATION_PASSED");
}
