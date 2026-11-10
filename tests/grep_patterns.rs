// SPDX-License-Identifier: MIT OR Apache-2.0
//! Pattern files contain bytes, not the CLI's internal argv escape encoding.
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

fn run(program: &Path, cwd: &Path, args: &[&str], input: Option<&[u8]>) -> Output {
    let mut command = Command::new(program);
    command
        .env_clear()
        .env("PATH", cwd.join("empty-path"))
        .env("HOME", cwd)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", cwd.join("absent-config"))
        .current_dir(cwd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    if let Some(input) = input {
        child.stdin.take().unwrap().write_all(input).unwrap();
    } else {
        drop(child.stdin.take());
    }
    child.wait_with_output().unwrap()
}

#[test]
fn binary_pattern_file_and_stdin_preserve_nul_and_unicode_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::create_dir(root.join("empty-path")).unwrap();
    let oracle = std::env::var_os("RIT_REFERENCE_GIT")
        .or_else(|| std::env::var_os("GITR_REFERENCE_GIT"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/usr/bin/git".into());
    assert!(oracle.is_absolute() && oracle.is_file());

    for (mode, pattern, literal, decoy) in [
        (
            "-P",
            b"\0\xee\x83\xbf\n".as_slice(),
            b"x\0\xee\x83\xbf\n".as_slice(),
            b"x\xff\n".as_slice(),
        ),
        ("-F", b"\xff\n", b"x\xff\n", b"x\xef\xbf\xbd\n"),
        ("-F", b"\xee\x83\xbf\n", b"x\xee\x83\xbf\n", b"x\xff\n"),
    ] {
        fs::write(root.join("pattern"), pattern).unwrap();
        fs::write(root.join("literal"), literal).unwrap();
        fs::write(root.join("decoy"), decoy).unwrap();
        for source in ["pattern", "-"] {
            // Interleave an argv pattern before the file pattern to verify that
            // byte preservation follows the correct expression-token index.
            let args = [
                "grep",
                "--no-index",
                "-a",
                "-l",
                mode,
                "-e",
                "never matches",
                "-f",
                source,
                "--",
                "literal",
                "decoy",
            ];
            let input = (source == "-").then_some(pattern);
            let expected = run(&oracle, root, &args, input);
            assert!(expected.status.success(), "oracle: {expected:?}");
            assert_eq!(expected.stdout, b"literal\n");
            let actual = run(Path::new(env!("CARGO_BIN_EXE_rit")), root, &args, input);
            assert!(actual.status.success(), "candidate: {actual:?}");
            assert_eq!(actual.stdout, expected.stdout, "{mode} {source}");
            assert_eq!(actual.stderr, expected.stderr, "{mode} {source}");
        }
    }
}
