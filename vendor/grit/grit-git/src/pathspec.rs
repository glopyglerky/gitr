//! CLI pathspec resolution helpers.
//!
//! The pure pathspec-resolution logic now lives in [`grit_lib::pathspec`]; the
//! items below are re-exported so existing `crate::pathspec::*` call sites keep
//! working. Only the CLI-local short-magic parser (used by `git clean`) remains
//! defined here.

pub use grit_lib::pathspec::{
    normalize_worktree_file_path, pathdiff, resolve_magic_pathspec, resolve_pathspec,
    resolve_pathspec_in_worktree, PathOutsideRepository,
};

#[derive(Debug, Default)]
pub(crate) struct PathspecMagic {
    pub(crate) icase: bool,
    pub(crate) prefix: Option<String>,
}

pub(crate) fn parse_magic(spec: &str) -> (PathspecMagic, &str) {
    let Some(rest) = spec.strip_prefix(":(") else {
        return (PathspecMagic::default(), spec);
    };
    let Some(close) = rest.find(')') else {
        return (PathspecMagic::default(), spec);
    };

    let (magic_part, tail_with_paren) = rest.split_at(close);
    let mut magic = PathspecMagic::default();
    for token in magic_part
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        if token.eq_ignore_ascii_case("icase") {
            magic.icase = true;
        } else if let Some(prefix) = token.strip_prefix("prefix:") {
            magic.prefix = Some(prefix.to_string());
        }
    }

    (magic, &tail_with_paren[1..])
}
