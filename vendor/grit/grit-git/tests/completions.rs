//! Tests for `grit-git completions <shell>` (the `clap_complete`-backed generator).
//!
//! The generator reconstructs the full clap command tree on demand (grit's hot
//! path uses manual pre-dispatch and never builds it). These tests guard that
//! reconstruction: every shell emits a non-empty, well-formed script, and the
//! fish output in particular carries subcommand names + descriptions and the
//! global options. Note: under `cargo test` (a debug build) clap's debug-time
//! argument assertions are live, so the generator must not panic — a command
//! whose `Args` trips an assertion degrades to name-only completion instead.

use std::process::Command;

const GRIT_BIN: &str = env!("CARGO_BIN_EXE_grit-git");

fn completions(shell: &str) -> std::process::Output {
    Command::new(GRIT_BIN)
        .args(["completions", shell])
        .output()
        .expect("failed to run grit-git completions")
}

#[test]
fn fish_completion_is_well_formed() {
    let out = completions("fish");
    assert!(out.status.success(), "exit: {:?}", out.status);
    let script = String::from_utf8(out.stdout).expect("utf8");

    // fish completion scaffolding clap_complete always emits for our binary.
    assert!(
        script.contains("complete -c grit-git"),
        "missing `complete -c grit-git` directives"
    );
    assert!(
        script.contains("__fish_grit_git_needs_command"),
        "missing fish subcommand helper"
    );

    // A representative subcommand with its `help -a` description.
    assert!(
        script.contains("Switch branches or restore working tree files"),
        "missing checkout description"
    );

    // Per-subcommand option carried through from the command's clap `Args`.
    assert!(
        script.contains("__fish_grit_git_using_subcommand checkout"),
        "missing checkout option completions"
    );

    // A grit global option (parsed by hand in main.rs, re-declared for the tree).
    assert!(
        script.contains("-l version"),
        "missing global --version option"
    );

    // The generator command completes itself.
    assert!(
        script.contains("completions"),
        "missing `completions` subcommand"
    );
}

#[test]
fn all_supported_shells_emit_nonempty_scripts() {
    for shell in ["bash", "zsh", "fish", "elvish", "powershell"] {
        let out = completions(shell);
        assert!(out.status.success(), "{shell}: exit {:?}", out.status);
        assert!(
            out.stdout.len() > 100,
            "{shell}: script suspiciously short ({} bytes)",
            out.stdout.len()
        );
        // No assertion panics or other noise should reach stderr.
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            !stderr.to_lowercase().contains("panic"),
            "{shell}: stderr contains a panic:\n{stderr}"
        );
    }
}

#[test]
fn unknown_shell_is_rejected() {
    let out = completions("tcsh");
    assert!(!out.status.success(), "tcsh should be rejected");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("unknown shell"),
        "expected an 'unknown shell' error, got: {stderr}"
    );
}

#[test]
fn missing_shell_argument_is_rejected() {
    let out = Command::new(GRIT_BIN)
        .arg("completions")
        .output()
        .expect("failed to run grit-git completions");
    assert!(!out.status.success(), "missing shell arg should fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("usage:"),
        "expected usage text, got: {stderr}"
    );
}
