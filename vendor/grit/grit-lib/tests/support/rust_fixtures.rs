//! Compiled fixture programs shared by otherwise independent integration tests.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

pub fn credential_helper(
    path: &Path,
    response: &str,
    record: Option<&Path>,
    record_arguments: bool,
    exit_code: i32,
) {
    std::fs::write(path.with_extension("response"), response).expect("credential response fixture");
    std::fs::write(path.with_extension("exit-code"), exit_code.to_string()).expect("helper exit code");
    if let Some(record) = record {
        std::fs::write(path.with_extension("record"), record.to_str().expect("UTF-8 fixture path"))
            .expect("credential record fixture");
    }
    if record_arguments {
        std::fs::write(path.with_extension("record-arguments"), []).expect("argument record fixture");
    }
    grit_test_support::compile_rust_fixture(path, include_str!("../fixtures/credential_helper.rs"))
        .expect("compile Rust credential helper");
}

pub fn fake_ssh(dir: &Path) -> PathBuf {
    let executable = dir.join("fake-ssh");
    grit_test_support::compile_rust_fixture(&executable, include_str!("../fixtures/fake_ssh.rs"))
        .expect("compile Rust SSH fixture");
    executable
}

pub fn forward_to(dir: &Path, name: &str, target: &Path) -> PathBuf {
    let executable = dir.join(name);
    std::fs::write(executable.with_extension("target"), target.to_str().expect("UTF-8 fixture path"))
        .expect("forwarded executable path");
    grit_test_support::compile_rust_fixture(&executable, include_str!("../fixtures/forward.rs"))
        .expect("compile Rust process fixture");
    executable
}

pub fn receive_pack(dir: &Path, name: &str, advertisement: &[u8], report: &[u8]) -> PathBuf {
    let executable = dir.join(name);
    std::fs::write(executable.with_extension("advertisement"), advertisement)
        .expect("receive-pack advertisement");
    std::fs::write(executable.with_extension("report"), report).expect("receive-pack report");
    grit_test_support::compile_rust_fixture(&executable, include_str!("../fixtures/receive_pack.rs"))
        .expect("compile Rust receive-pack fixture");
    executable
}

pub fn pre_receive_hook(path: &Path, record: Option<&Path>) {
    if let Some(record) = record {
        std::fs::write(path.with_extension("record"), record.to_str().expect("UTF-8 fixture path"))
            .expect("push-option record path");
    }
    grit_test_support::compile_rust_fixture(path, include_str!("../fixtures/pre_receive.rs"))
        .expect("compile Rust pre-receive hook");
}
