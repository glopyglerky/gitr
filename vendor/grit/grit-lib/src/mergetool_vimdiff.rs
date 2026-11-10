//! Vimdiff tool names and declarative layout defaults.
//!
//! Gitr does not generate executable Vimscript. Built-in Vimdiff merge layouts
//! are unsupported; an explicitly configured mergetool command remains available.

/// Resolves the layout string for a merge tool name, matching `merge_cmd` in Git's vimdiff script.
///
/// * `tool` — e.g. `vimdiff`, `gvimdiff2`, `nvimdiff1`.
/// * `mergetool_layout` — value of `mergetool.<tool>.layout` when set.
/// * `vimdiff_layout_fallback` — value of `mergetool.vimdiff.layout` when variant-specific layout is unset.
#[must_use]
pub fn vimdiff_resolve_layout<'a>(
    tool: &str,
    mergetool_layout: Option<&'a str>,
    vimdiff_layout_fallback: Option<&'a str>,
) -> &'a str {
    if let Some(l) = mergetool_layout.filter(|s| !s.is_empty()) {
        return l;
    }
    if let Some(l) = vimdiff_layout_fallback.filter(|s| !s.is_empty()) {
        return l;
    }

    if tool.ends_with("vimdiff1") {
        return "@LOCAL,REMOTE";
    }
    if tool.ends_with("vimdiff2") {
        return "LOCAL,MERGED,REMOTE";
    }
    if tool.ends_with("vimdiff3") {
        return "MERGED";
    }

    if tool.contains("vimdiff") {
        return "(LOCAL,BASE,REMOTE)/MERGED";
    }

    "(LOCAL,BASE,REMOTE)/MERGED"
}

/// Executable name for a vimdiff-family merge tool (`vim`, `gvim`, `nvim`).
#[must_use]
pub fn vimdiff_executable_for_tool(tool: &str) -> Option<&'static str> {
    if tool.starts_with("nvimdiff") {
        return Some("nvim");
    }
    if tool.starts_with("gvimdiff") {
        return Some("gvim");
    }
    if tool.starts_with("vimdiff") {
        return Some("vim");
    }
    None
}
