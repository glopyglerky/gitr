//! `grit whatchanged` — like `git log` but shows raw diff output.
//!
//! Equivalent to `git log --raw --no-merges` with the root commit's diff shown.
//! The per-commit walk and rendering are delegated to the `diff-tree` machinery
//! (`diff_tree::run_whatchanged`) so the output is byte-for-byte identical to
//! `git diff-tree --pretty` applied to each non-merge commit.

use anyhow::Result;

/// Run the `whatchanged` command from the raw CLI argument list.
pub fn run(argv: &[String]) -> Result<()> {
    // Upstream Git refuses to run the deprecated `whatchanged` command unless
    // `--i-still-use-this` is given; the test harness passes it. Filter the flag
    // out and forward the remaining options to the diff-tree walker.
    let mut rest: Vec<String> = Vec::with_capacity(argv.len());
    let mut opted_in = false;
    for arg in argv {
        if arg == "--i-still-use-this" {
            opted_in = true;
            continue;
        }
        rest.push(arg.clone());
    }

    if !opted_in {
        eprintln!("'git whatchanged' is nominated for removal.");
        eprintln!();
        eprintln!(
            "hint: You can replace 'git whatchanged <opts>' with:\n\
             hint:\tgit log <opts> --raw --no-merges\n\
             hint: Or make an alias:\n\
             hint:\tgit config set --global alias.whatchanged 'log --raw --no-merges'\n"
        );
        eprintln!();
        eprintln!("If you still use this command, here's what you can do:");
        eprintln!();
        eprintln!("- read https://git-scm.com/docs/BreakingChanges.html");
        eprintln!("- check if anyone has discussed this on the mailing");
        eprintln!("  list and if they came up with something that can");
        eprintln!("  help you: https://lore.kernel.org/git/?q=git%20whatchanged");
        eprintln!("- send an email to <git@vger.kernel.org> to let us");
        eprintln!("  know that you still use this command and were unable to");
        eprintln!("  determine a suitable replacement");
        eprintln!();
        anyhow::bail!("refusing to run without --i-still-use-this");
    }

    crate::commands::diff_tree::run_whatchanged(&rest)
}
