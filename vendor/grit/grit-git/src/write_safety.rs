//! The product entry point exposes reference writers only after CAS migration.
//! The imported standalone donor remains available for upstream conformance
//! work; its command inventory is not Gitr's qualified capability inventory.
use anyhow::{bail, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

static GITR_PRODUCT: AtomicBool = AtomicBool::new(false);
static GITR_VERSION: OnceLock<&'static str> = OnceLock::new();

pub(crate) fn enable(version: &'static str) {
    let _ = GITR_VERSION.set(version);
    GITR_PRODUCT.store(true, Ordering::Relaxed);
}
pub(crate) fn is_product() -> bool { GITR_PRODUCT.load(Ordering::Relaxed) }
pub(crate) fn product_version() -> Option<&'static str> { GITR_VERSION.get().copied() }

const PENDING_REFERENCE_MIGRATION: &[&str] = &[
    "am", "backfill", "bisect", "checkout", "switch", "cherry-pick", "revert",
    "clone", "fast-import", "fetch", "fetch-pack", "push", "send-pack", "pull",
    "merge", "notes", "rebase", "receive-pack", "replace", "replay", "history",
    "reset", "stash", "submodule", "symbolic-ref", "update-ref", "worktree",
    "gc", "maintenance", "pack-refs", "repack", "prune", "refs", "scalar",
];

pub(crate) fn check(command: &str, args: &[String]) -> Result<()> {
    if !is_product() { return Ok(()); }
    // Help is permitted, but only a sole help option. A filename or another
    // operand named --help must never turn off the mutation boundary.
    if args.len() == 1 && matches!(args[0].as_str(), "-h" | "--help" | "--help-all") { return Ok(()); }
    check_operation(command, args)
}

fn check_operation(command: &str, args: &[String]) -> Result<()> {
    let pending = PENDING_REFERENCE_MIGRATION.contains(&command)
        || (command == "remote" && args.iter().find(|arg| !arg.starts_with('-'))
            .is_some_and(|arg| arg.as_str() != "get-url"))
        || (command == "reflog" && args.iter().any(|arg| matches!(arg.as_str(), "delete" | "drop" | "expire" | "write")));
    if pending {
        bail!("Gitr has imported '{command}' source, but its reference publication is not yet qualified for atomic expected-old updates; this operation is disabled before mutation (see Documentation/gitr/rust-git-import.md)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unqualified_reference_writers_fail_closed() {
        for command in PENDING_REFERENCE_MIGRATION { assert!(check_operation(command, &[]).is_err(), "{command}"); }
        assert!(check_operation("remote", &["remove".into(), "origin".into()]).is_err());
        assert!(check_operation("remote", &["--verbose".into(), "remove".into(), "origin".into()]).is_err());
        assert!(check_operation("reflog", &["expire".into(), "--all".into()]).is_err());
        for command in ["commit", "branch", "tag", "log", "status", "hash-object"] { check_operation(command, &[]).unwrap(); }
        check_operation("remote", &["get-url".into(), "origin".into()]).unwrap();
    }
}
