//! Keep repository inspection from executing configuration-supplied programs.
//!
//! Commands that deliberately change the worktree keep Git's hook/filter model.
//! Inspection fails explicitly when the effective configuration could launch a
//! program. This is a documented compatibility difference, never silently
//! returning an unfiltered result as if a required filter had run.
use anyhow::{bail, Result};
use grit_lib::config::ConfigSet;

const INSPECTION: &[&str] = &[
    "annotate", "blame", "cat-file", "check-attr", "check-ignore", "check-mailmap",
    "check-ref-format", "count-objects", "describe", "diff", "diff-files",
    "diff-index", "diff-tree", "for-each-ref", "grep", "hash-object", "log",
    "ls-files", "ls-tree", "merge-base", "name-rev", "rev-list", "rev-parse",
    "shortlog", "show", "show-ref", "status", "whatchanged",
];

pub(crate) fn check(command: &str) -> Result<()> {
    if !INSPECTION.contains(&command) {
        return Ok(());
    }
    let repository = grit_lib::rev_parse::discover_optional(None)?;
    let config = ConfigSet::load(repository.as_ref().map(|repo| repo.git_dir.as_path()), true)?;
    check_config(&config)?;
    if std::env::var_os("GIT_EXTERNAL_DIFF").is_some_and(|value| !value.is_empty()) {
        bail!("repository inspection does not execute GIT_EXTERNAL_DIFF");
    }
    Ok(())
}

fn check_config(config: &ConfigSet) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    for entry in config.entries().iter().rev() {
        if seen.insert(entry.key.as_str()) && executable(&entry.key, entry.value.as_deref()) {
            bail!("repository inspection does not execute configured helper '{}'", entry.key);
        }
    }
    Ok(())
}

fn executable(key: &str, value: Option<&str>) -> bool {
    let value = value.unwrap_or("").trim();
    if value.is_empty() {
        return false;
    }
    if key == "core.fsmonitor" || key == "core.pager" || key.starts_with("pager.") {
        return !matches!(value.to_ascii_lowercase().as_str(), "false" | "0" | "no" | "off" | "true" | "1" | "yes" | "on");
    }
    key == "diff.external"
        || (key.starts_with("diff.") && (key.ends_with(".textconv") || key.ends_with(".command")))
        || (key.starts_with("filter.") && [".clean", ".smudge", ".process"].iter().any(|suffix| key.ends_with(suffix)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_effective_programs_without_logging_their_contents() {
        let mut config = ConfigSet::new();
        config.add_command_override("diff.example.textconv", "secret-command").unwrap();
        let error = check_config(&config).unwrap_err().to_string();
        assert!(error.contains("diff.example.textconv"));
        assert!(!error.contains("secret-command"));
    }

    #[test]
    fn respects_disabled_monitor_and_overridden_program() {
        let mut config = ConfigSet::new();
        config.add_command_override("core.fsmonitor", "untrusted-hook").unwrap();
        config.add_command_override("core.fsmonitor", "false").unwrap();
        config.add_command_override("filter.example.required", "true").unwrap();
        check_config(&config).unwrap();
    }
}
