//! Select the native Git CLI while retaining Gitr's isolated component profile.
use std::ffi::OsString;

const EXTENSIONS: &[&str] = &["actions", "assets", "db", "doctor", "lfs"];

/// Standard commands use the imported Rust Git implementation. The explicit
/// `--isolated-git` selector retains the previous gix command contract, including
/// bounded reads, local-only configuration and mandatory expected-old ref writes.
/// Only a global selector is removed; a filename or `-C` value is left intact.
pub(crate) fn select(args: &mut Vec<OsString>) -> bool {
    let mut index = 1;
    while let Some(arg) = args.get(index) {
        if arg == "--isolated-git" {
            args.remove(index);
            return false;
        }
        if arg == "--native-git" {
            return true;
        }
        if matches!(arg.to_str(), Some("-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace" | "--config-env" | "--attr-source")) {
            index += 2;
            continue;
        }
        let Some(arg) = arg.to_str() else {
            return true;
        };
        if matches!(arg, "--help" | "-h" | "--version" | "-v") {
            return true;
        }
        if arg.starts_with('-') {
            index += 1;
            continue;
        }
        // Preserve the already qualified strict-CAS plumbing interface while
        // the donor's full update-ref transaction modes are being migrated.
        if arg == "update-ref" { return false; }
        return !EXTENSIONS.contains(&arg);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(items: &[&str]) -> Vec<OsString> {
        items.iter().map(OsString::from).collect()
    }

    #[test]
    fn preserves_extension_routes() {
        for command in EXTENSIONS {
            assert!(!select(&mut args(&["gitr", "-C", "repo", command])));
        }
        assert!(select(&mut args(&["gitr", "--help"])));
    }

    #[test]
    fn standard_commands_use_native_engine() {
        for command in ["init", "add", "commit", "clone", "fetch", "push", "merge", "rebase", "stash", "log"] {
            assert!(select(&mut args(&["gitr", "-C", "repo", command])));
        }
        assert!(select(&mut args(&["gitr", "--native-git", "log"])));
        assert!(select(&mut args(&["gitr", "-c", "user.name=Test", "commit"])));
        assert!(!select(&mut args(&["gitr", "update-ref", "refs/heads/main", "new", "old"])));
    }

    #[test]
    fn isolated_selector_is_removed_only_before_the_command() {
        let mut invocation = args(&["gitr", "-C", "repo", "--isolated-git", "status"]);
        assert!(!select(&mut invocation));
        assert_eq!(invocation, args(&["gitr", "-C", "repo", "status"]));
        assert!(select(&mut args(&["gitr", "-C", "--isolated-git", "status"])));
        assert!(select(&mut args(&["gitr", "hash-object", "--no-filters", "--isolated-git"])));
    }
}
