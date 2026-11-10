//! Reserved `filter-branch` command pending a qualified native implementation.
//!
//! The donor's embedded shell engine was excluded because its inherited source
//! provenance is unresolved. Reject before reading or changing repository state.

use sley::Result;

pub(crate) fn cmd_filter_branch(args: &[String]) -> Result<()> {
    if matches!(args, [help] if help == "-h" || help == "--help") {
        eprintln!("usage: git filter-branch [options] [--] [revision-list]");
        eprintln!(
            "filter-branch is unavailable in this Gitr build; its native replacement is not qualified"
        );
        return Err(crate::cli_exit(129));
    }
    eprintln!(
        "fatal: filter-branch is unavailable in this Gitr build; its native replacement is not qualified"
    );
    Err(crate::cli_exit(128))
}
