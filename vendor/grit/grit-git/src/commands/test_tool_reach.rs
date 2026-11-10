//! `test-tool reach` support used by upstream reachability tests.

use std::io::{self, BufRead};

use anyhow::{bail, Context, Result};
use grit_lib::merge_base::{
    ancestor_closure, branch_base_for_tip, independent_commits, is_ancestor,
    merge_bases_first_vs_rest,
};
use grit_lib::objects::ObjectId;
use grit_lib::repo::Repository;
use grit_lib::rev_parse::{peel_to_commit_for_merge_base, resolve_revision};

#[derive(Default)]
struct ReachInput {
    a: Option<ObjectId>,
    b: Option<ObjectId>,
    x: Vec<ObjectId>,
    y: Vec<ObjectId>,
}

/// Run `test-tool reach`.
///
/// # Parameters
///
/// - `args` - command arguments after `test-tool reach`.
///
/// # Errors
///
/// Returns an error when repository discovery, revision resolution, or commit
/// graph traversal fails.
pub fn run(args: &[String]) -> Result<()> {
    let subcmd = args
        .first()
        .ok_or_else(|| anyhow::anyhow!("usage: test-tool reach <subcommand>"))?;
    let repo = Repository::discover(None).context("not a git repository")?;
    let input = read_input(&repo)?;

    match subcmd.as_str() {
        "ref_newer" => {
            let a = require(input.a, "A")?;
            let b = require(input.b, "B")?;
            println!("ref_newer(A,B):{}", bool_int(is_ancestor(&repo, b, a)?));
        }
        "in_merge_bases" => {
            let a = require(input.a, "A")?;
            let b = require(input.b, "B")?;
            println!(
                "in_merge_bases(A,B):{}",
                bool_int(is_ancestor(&repo, a, b)?)
            );
        }
        "in_merge_bases_many" => {
            let a = require(input.a, "A")?;
            let hit = any_reachable_from(&repo, a, &input.x)?;
            println!("in_merge_bases_many(A,X):{}", bool_int(hit));
        }
        "is_descendant_of" => {
            let a = require(input.a, "A")?;
            let hit = any_ancestor_of(&repo, &input.x, a)?;
            println!("is_descendant_of(A,X):{}", bool_int(hit));
        }
        "get_branch_base_for_tip" => {
            let a = require(input.a, "A")?;
            let index = branch_base_for_tip(&repo, a, &input.x)?
                .map(|index| index as isize)
                .unwrap_or(-1);
            println!("get_branch_base_for_tip(A,X):{index}");
        }
        "get_merge_bases_many" => {
            let a = require(input.a, "A")?;
            let mut bases = merge_bases_first_vs_rest(&repo, a, &input.x)?;
            bases.sort();
            println!("get_merge_bases_many(A,X):");
            print_oids(&bases);
        }
        "reduce_heads" => {
            let mut reduced = independent_commits(&repo, &input.x)?;
            reduced.sort();
            println!("reduce_heads(X):");
            print_oids(&reduced);
        }
        "can_all_from_reach" => {
            println!(
                "can_all_from_reach(X,Y):{}",
                bool_int(all_reachable_from_any(&repo, &input.x, &input.y)?)
            );
        }
        "can_all_from_reach_with_flag" => {
            println!(
                "can_all_from_reach_with_flag(X,_,_,0,0):{}",
                bool_int(all_reachable_from_any(&repo, &input.x, &input.y)?)
            );
        }
        "commit_contains" => {
            let a = require(input.a, "A")?;
            let hit = any_ancestor_of(&repo, &input.x, a)?;
            println!("commit_contains(_,A,X,_):{}", bool_int(hit));
        }
        "get_reachable_subset" => {
            let mut reachable = reachable_subset(&repo, &input.x, &input.y)?;
            reachable.sort();
            println!("get_reachable_subset(X,Y)");
            print_oids(&reachable);
        }
        other => bail!("test-tool reach: unknown subcommand '{other}'"),
    }

    Ok(())
}

fn read_input(repo: &Repository) -> Result<ReachInput> {
    let mut input = ReachInput::default();
    for line in io::stdin().lock().lines() {
        let line = line?;
        if line.len() < 3 {
            continue;
        }
        let (prefix, spec) = line.split_at(2);
        let oid = resolve_revision(repo, spec)
            .and_then(|oid| peel_to_commit_for_merge_base(repo, oid))
            .with_context(|| format!("failed to resolve {spec}"))?;
        match prefix.as_bytes()[0] {
            b'A' => input.a = Some(oid),
            b'B' => input.b = Some(oid),
            b'X' => input.x.push(oid),
            b'Y' => input.y.push(oid),
            other => bail!("unexpected start of line: {}", other as char),
        }
    }
    Ok(input)
}

fn require(value: Option<ObjectId>, name: &str) -> Result<ObjectId> {
    value.ok_or_else(|| anyhow::anyhow!("test-tool reach: missing {name} input"))
}

fn bool_int(value: bool) -> u8 {
    u8::from(value)
}

fn print_oids(oids: &[ObjectId]) {
    for oid in oids {
        println!("{}", oid.to_hex());
    }
}

fn all_reachable_from_any(repo: &Repository, from: &[ObjectId], to: &[ObjectId]) -> Result<bool> {
    for &target in to {
        let mut reachable = false;
        for &tip in from {
            if is_ancestor(repo, target, tip)? {
                reachable = true;
                break;
            }
        }
        if !reachable {
            return Ok(false);
        }
    }
    Ok(true)
}

fn any_reachable_from(repo: &Repository, target: ObjectId, from: &[ObjectId]) -> Result<bool> {
    for &tip in from {
        if is_ancestor(repo, target, tip)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn any_ancestor_of(
    repo: &Repository,
    ancestors: &[ObjectId],
    descendant: ObjectId,
) -> Result<bool> {
    for &ancestor in ancestors {
        if is_ancestor(repo, ancestor, descendant)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn reachable_subset(
    repo: &Repository,
    from: &[ObjectId],
    to: &[ObjectId],
) -> Result<Vec<ObjectId>> {
    let mut from_closure = std::collections::HashSet::new();
    for &tip in from {
        from_closure.extend(ancestor_closure(repo, tip)?);
    }
    Ok(to
        .iter()
        .copied()
        .filter(|oid| from_closure.contains(oid))
        .collect())
}
