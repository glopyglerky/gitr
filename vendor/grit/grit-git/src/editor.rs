//! Editor resolution matching upstream Git's `git_editor()` in `editor.c`.
//!
//! Order: `GIT_EDITOR` → `core.editor` → `VISUAL` (only when the terminal is not dumb) →
//! `EDITOR`. If the terminal is dumb (`TERM` unset or `dumb`) and no editor was chosen from
//! those sources, returns [`None`] (Git would not fall back to `vi`). Otherwise returns
//! [`Some`] with the resolved command, defaulting to `vi`.

use grit_lib::config::ConfigSet;

/// Matches Git's `is_terminal_dumb()`: true when `TERM` is unset or equals `"dumb"`.
#[must_use]
pub(crate) fn is_terminal_dumb() -> bool {
    match std::env::var("TERM") {
        Ok(t) => t == "dumb",
        Err(_) => true,
    }
}

fn env_editor_candidate(key: &str, for_launch: bool) -> Option<String> {
    let v = std::env::var(key).ok()?;
    let t = v.trim();
    if t.is_empty() {
        return None;
    }
    // `launch_specified_editor` treats an explicit `GIT_EDITOR=:` as a no-op. The test harness
    // also sets `EDITOR=:` / `VISUAL=:` globally; when actually launching an editor, ignore
    // those placeholders so a later fake editor can win.
    if for_launch && t == ":" && matches!(key, "EDITOR" | "VISUAL") {
        return None;
    }
    Some(v)
}

/// Resolve the editor command like Git's `git_editor()`.
///
/// `for_launch`: when `true`, treat `EDITOR` / `VISUAL` values of `:` as unset (harness
/// placeholders). When `false` (`git var`), preserve them so output matches upstream Git.
///
/// Returns [`None`] only when the terminal is dumb and no editor was found in the
/// environment or config (Git then errors with "Terminal is dumb, but EDITOR unset").
#[must_use]
pub(crate) fn resolve_git_editor(config: &ConfigSet, for_launch: bool) -> Option<String> {
    let terminal_is_dumb = is_terminal_dumb();

    if let Some(e) = env_editor_candidate("GIT_EDITOR", for_launch) {
        return Some(e);
    }
    if let Some(e) = config.get("core.editor") {
        let t = e.trim();
        if !t.is_empty() {
            return Some(e);
        }
    }
    if !terminal_is_dumb {
        if let Some(v) = env_editor_candidate("VISUAL", for_launch) {
            return Some(v);
        }
    }
    if let Some(e) = env_editor_candidate("EDITOR", for_launch) {
        return Some(e);
    }
    if terminal_is_dumb {
        return None;
    }
    // Upstream `git_editor()` always falls back to DEFAULT_EDITOR when the terminal
    // is not dumb, even when stdin is not a TTY (`git var GIT_EDITOR`, t7005).
    Some("vi".to_owned())
}

/// Resolve an editor for commit-style launches while preserving `EDITOR=:` as a no-op.
///
/// The test harness exports `VISUAL=:` as a placeholder, but individual commands commonly set
/// `EDITOR=...` to select a test editor. Git ignores that placeholder for launches, while still
/// treating an explicit `EDITOR=:` as a valid no-op editor.
#[must_use]
pub(crate) fn resolve_commit_launch_editor(config: &ConfigSet) -> Option<String> {
    let terminal_is_dumb = is_terminal_dumb();

    if let Some(e) = env_editor_candidate("GIT_EDITOR", false) {
        return Some(e);
    }
    if let Some(e) = config.get("core.editor") {
        let t = e.trim();
        if !t.is_empty() {
            return Some(e);
        }
    }
    if !terminal_is_dumb {
        if let Some(v) = env_editor_candidate("VISUAL", true) {
            return Some(v);
        }
    }
    if let Some(e) = env_editor_candidate("EDITOR", false) {
        return Some(e);
    }
    if terminal_is_dumb {
        return None;
    }
    // Match upstream `git_editor()`: once the terminal is not dumb and nothing is
    // configured, fall back to DEFAULT_EDITOR ("vi") unconditionally. Git does not
    // consult `isatty()` here, so neither do we (t7005 "Using vi" runs without a TTY).
    Some("vi".to_owned())
}
