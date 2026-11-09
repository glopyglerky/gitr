//! Exercises the real Gitr worker executable, not an in-process VM mock.
use gitr::actions::runner::{EphemeralRunner, WORKER_ARGUMENT};
use sha2::{Digest, Sha256};
use std::{io::{Read, Write}, path::Path, process::{Command, Stdio}, thread, time::{Duration, Instant}};

fn runner() -> EphemeralRunner { EphemeralRunner::new(Path::new(env!("CARGO_BIN_EXE_gitr"))).unwrap() }
fn module() -> Vec<u8> {
    wat::parse_str(r#"(module
        (import "gitr_actions_v1" "input_len" (func $len (result i32)))
        (import "gitr_actions_v1" "input_read" (func $read (param i32 i32 i32) (result i32)))
        (import "gitr_actions_v1" "output_write" (func $write (param i32 i32) (result i32)))
        (memory (export "memory") 16)
        (func (export "run") (result i32)
            (drop (call $read (i32.const 0) (i32.const 0) (call $len)))
            (drop (call $write (i32.const 0) (call $len))) i32.const 0))"#).unwrap()
}

#[test]
fn actual_ephemeral_worker_validates_and_roundtrips_binary_without_a_repository() {
    let runner = runner();
    let module = module();
    runner.validate(&module).unwrap();
    for input in [vec![], b"committed bytes\0\xff".to_vec(), vec![42; 1024 * 1024]] {
        assert_eq!(runner.execute(&module, &input).unwrap(), (0, input));
    }
    assert!(runner.validate(b"not wasm").is_err());
}

#[test]
fn each_real_worker_starts_fresh_and_failed_guests_return_no_partial_output() {
    let fresh = wat::parse_str(r#"(module
        (import "gitr_actions_v1" "output_write" (func $write (param i32 i32) (result i32)))
        (memory (export "memory") 1) (global $counter (mut i32) (i32.const 0))
        (func (export "run") (result i32)
            (global.set $counter (i32.add (global.get $counter) (i32.const 1)))
            (i32.store8 (i32.const 0) (global.get $counter))
            (drop (call $write (i32.const 0) (i32.const 1))) i32.const 0))"#).unwrap();
    let runner = runner();
    assert_eq!(runner.execute(&fresh, &[]).unwrap(), (0, vec![1]));
    assert_eq!(runner.execute(&fresh, &[]).unwrap(), (0, vec![1]));
    let failed = wat::parse_str(r#"(module
        (import "gitr_actions_v1" "output_write" (func $write (param i32 i32) (result i32)))
        (memory (export "memory") 1) (data (i32.const 0) "partial")
        (func (export "run") (result i32)
            (drop (call $write (i32.const 0) (i32.const 7))) i32.const 9))"#).unwrap();
    assert_eq!(runner.execute(&failed, &[]).unwrap(), (9, vec![]));
}

#[test]
fn real_worker_retains_capability_denial_fuel_and_memory_limits() {
    let runner = runner();
    for text in [
        r#"(module (import "wasi_snapshot_preview1" "fd_write" (func)) (memory (export "memory") 1) (func (export "run") (result i32) i32.const 0))"#,
        r#"(module (memory (export "memory") 1) (func (export "run") (result i32) (drop (memory.grow (i32.const 10000))) i32.const 0))"#,
        r#"(module (memory (export "memory") 1) (func (export "run") (result i32) (loop $again (br $again)) i32.const 0))"#,
    ] { assert!(runner.execute(&wat::parse_str(text).unwrap(), &[]).is_err()); }
}

#[test]
fn worker_rejects_unframed_or_oversized_requests_without_creating_state() {
    let directory = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_gitr"))
        .arg(WORKER_ARGUMENT).current_dir(directory.path())
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let mut request = b"GITRWK1\0\x01".to_vec();
    request.extend_from_slice(&u32::MAX.to_be_bytes());
    request.extend_from_slice(&0u32.to_be_bytes());
    child.stdin.take().unwrap().write_all(&request).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty() && output.stderr.is_empty());
    assert!(directory.path().read_dir().unwrap().next().is_none());
}

#[test]
fn closing_the_parent_channel_cannot_leave_a_live_guest_behind() {
    let directory = tempfile::tempdir().unwrap();
    let wasm = wat::parse_str(r#"(module (memory (export "memory") 1)
        (func (export "run") (result i32) (loop $again (br $again)) i32.const 0))"#).unwrap();
    let mut identity = Sha256::new();
    identity.update(b"GITRWK1\0"); identity.update([1]);
    identity.update((wasm.len() as u32).to_be_bytes()); identity.update(0u32.to_be_bytes()); identity.update(&wasm);
    let mut request = b"GITRWK1\0\x01".to_vec();
    request.extend_from_slice(&(wasm.len() as u32).to_be_bytes());
    request.extend_from_slice(&0u32.to_be_bytes());
    request.extend_from_slice(&identity.finalize()); request.extend_from_slice(&wasm);
    let mut child = Command::new(env!("CARGO_BIN_EXE_gitr"))
        .arg(WORKER_ARGUMENT).current_dir(directory.path())
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    let mut channel = child.stdin.take().unwrap();
    channel.write_all(&request).unwrap(); channel.flush().unwrap();
    drop(channel);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if child.try_wait().unwrap().is_some() { child.wait().unwrap(); break; }
        if Instant::now() >= deadline {
            child.kill().unwrap(); child.wait().unwrap(); panic!("worker survived parent-channel loss");
        }
        thread::sleep(Duration::from_millis(10));
    }
    // A parent-channel exit races a naturally completed/fuel-limited guest; both
    // must leave no running process and neither has authority to publish to redb.
    let mut output = Vec::new(); child.stdout.take().unwrap().read_to_end(&mut output).unwrap();
    assert!(output.len() <= 81);
    assert!(directory.path().read_dir().unwrap().next().is_none());
}
