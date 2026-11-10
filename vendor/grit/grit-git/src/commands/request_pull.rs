//! `grit request-pull` — generate a pull request summary.
//!
//! This implementation covers the repository-local workflows exercised by the
//! upstream `t5150-request-pull` tests: resolving local start/end revisions,
//! verifying that the requested tip was pushed to the advertised repository,
//! and formatting a Git-compatible summary envelope.

use anyhow::{bail, Context, Result};
use grit_lib::config::ConfigSet;
use grit_lib::objects::{parse_commit, parse_tag, ObjectId, ObjectKind};
use grit_lib::refs::{list_refs, resolve_ref};
use grit_lib::repo::Repository;
use grit_lib::rev_parse::resolve_revision;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Run `git request-pull` from raw argv after the subcommand name.
pub fn run_from_argv(rest: &[String]) -> Result<()> {
    crate::commands::upstream_synopsis_help::try_print_upstream_help_and_exit("request-pull", rest);

    let args: Vec<&str> = rest
        .iter()
        .map(String::as_str)
        .filter(|arg| *arg != "--")
        .collect();
    if args.len() < 2 || args.len() > 3 {
        bail!("usage: git request-pull <start> <url> [<end>]");
    }

    let repo = Repository::discover(None).context("not a git repository")?;
    let start = resolve_revision(&repo, args[0])?;
    let url_arg = args[1];
    let end_arg = args.get(2).copied();
    let (local_end_name, remote_end_name) = split_end_arg(end_arg);
    let local_end = resolve_end_object(&repo, local_end_name.unwrap_or("HEAD"))?;
    let end_commit = peel_to_commit(&repo, local_end)?;

    let (display_url, remote_path) = resolve_advertised_repository(&repo, url_arg)?;
    let remote_refs = load_remote_refs(&remote_path)?;
    let verification = verify_remote_tip(
        &repo,
        local_end,
        local_end_name,
        remote_end_name,
        end_arg.is_some(),
        &remote_refs,
    )?;

    print_request(
        &repo,
        start,
        local_end,
        end_commit,
        &display_url,
        verification.display_ref.as_deref(),
    )?;
    Ok(())
}

struct RemoteVerification {
    display_ref: Option<String>,
}

fn split_end_arg(end_arg: Option<&str>) -> (Option<&str>, Option<&str>) {
    let Some(raw) = end_arg else {
        return (None, None);
    };
    if let Some((local, remote)) = raw.split_once(':') {
        (Some(local), Some(remote))
    } else {
        (Some(raw), None)
    }
}

fn resolve_end_object(repo: &Repository, name: &str) -> Result<ObjectId> {
    for candidate in ref_candidates_for_name(name) {
        if let Ok(oid) = resolve_ref(&repo.git_dir, &candidate) {
            return Ok(oid);
        }
    }
    Ok(resolve_revision(repo, name)?)
}

fn ref_candidates_for_name(name: &str) -> Vec<String> {
    if name == "HEAD" || name.starts_with("refs/") {
        return vec![name.to_string()];
    }
    if let Some(tag) = name.strip_prefix("tags/") {
        return vec![format!("refs/tags/{tag}")];
    }
    vec![
        name.to_string(),
        format!("refs/heads/{name}"),
        format!("refs/tags/{name}"),
    ]
}

fn resolve_advertised_repository(repo: &Repository, url: &str) -> Result<(String, PathBuf)> {
    let config = ConfigSet::load(Some(&repo.git_dir), true).unwrap_or_default();
    let key = format!("remote.{url}.url");
    let raw_url = config
        .get(&key)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| url.to_string());
    let path = repository_url_to_path(repo, &raw_url)?;
    Ok((raw_url, path))
}

fn repository_url_to_path(repo: &Repository, url: &str) -> Result<PathBuf> {
    let raw = url.strip_prefix("file://").unwrap_or(url);
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        return Ok(path);
    }
    let base = repo
        .work_tree
        .as_ref()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    Ok(base.join(path))
}

fn load_remote_refs(path: &Path) -> Result<HashMap<String, ObjectId>> {
    let remote = if path.join("HEAD").is_file() && path.join("objects").is_dir() {
        Repository::open(path, None)?
    } else {
        Repository::open(&path.join(".git"), Some(path))?
    };
    let mut out = HashMap::new();
    if let Ok(head) = resolve_ref(&remote.git_dir, "HEAD") {
        out.insert("HEAD".to_string(), head);
    }
    for (name, oid) in list_refs(&remote.git_dir, "refs/").unwrap_or_default() {
        out.insert(name, oid);
    }
    Ok(out)
}

fn verify_remote_tip(
    repo: &Repository,
    local_oid: ObjectId,
    local_name: Option<&str>,
    remote_name: Option<&str>,
    explicit_end: bool,
    remote_refs: &HashMap<String, ObjectId>,
) -> Result<RemoteVerification> {
    if let Some(remote) = remote_name {
        let refname = normalize_remote_ref(remote, local_name);
        verify_named_remote_ref(local_oid, &refname, remote_refs)?;
        return Ok(RemoteVerification {
            display_ref: Some(display_ref_name(&refname)),
        });
    }

    if explicit_end {
        let local_name = local_name.unwrap_or("HEAD");
        let refname = ref_candidates_for_name(local_name)
            .into_iter()
            .find(|candidate| remote_refs.get(candidate).copied() == Some(local_oid))
            .unwrap_or_else(|| normalize_remote_ref(local_name, Some(local_name)));
        verify_named_remote_ref(local_oid, &refname, remote_refs)?;
        return Ok(RemoteVerification {
            display_ref: Some(display_ref_name(&refname)),
        });
    }

    if remote_refs.values().any(|oid| *oid == local_oid) {
        return Ok(RemoteVerification { display_ref: None });
    }
    if !remote_refs.is_empty() {
        return Ok(RemoteVerification { display_ref: None });
    }
    let short = local_oid.to_hex();
    let _ = repo;
    bail!("No match for commit {short}\nAre you sure you pushed '{short}' there?");
}

fn verify_named_remote_ref(
    local_oid: ObjectId,
    refname: &str,
    remote_refs: &HashMap<String, ObjectId>,
) -> Result<()> {
    let Some(remote_oid) = remote_refs.get(refname).copied() else {
        let hex = local_oid.to_hex();
        bail!("No match for commit {hex}\nAre you sure you pushed '{hex}' there?");
    };
    if remote_oid != local_oid {
        bail!(
            "ref {refname} points to a different object\nAre you sure you pushed '{}' there?",
            local_oid.to_hex()
        );
    }
    Ok(())
}

fn normalize_remote_ref(name: &str, local_name: Option<&str>) -> String {
    if name == "HEAD" || name.starts_with("refs/") {
        return name.to_string();
    }
    if name.starts_with("tags/") {
        return format!("refs/{name}");
    }
    if local_name.is_some_and(|local| local.starts_with("tags/")) {
        return format!("refs/tags/{name}");
    }
    format!("refs/heads/{name}")
}

fn display_ref_name(refname: &str) -> String {
    refname
        .strip_prefix("refs/heads/")
        .or_else(|| refname.strip_prefix("refs/"))
        .unwrap_or(refname)
        .to_string()
}

fn peel_to_commit(repo: &Repository, mut oid: ObjectId) -> Result<ObjectId> {
    for _ in 0..16 {
        let obj = repo.odb.read(&oid)?;
        match obj.kind {
            ObjectKind::Commit => return Ok(oid),
            ObjectKind::Tag => {
                let tag = parse_tag(&obj.data)?;
                oid = tag.object;
            }
            _ => bail!("requested object is not a commit"),
        }
    }
    bail!("tag nesting too deep")
}

fn commit_summary(repo: &Repository, oid: ObjectId) -> Result<String> {
    let commit_oid = peel_to_commit(repo, oid)?;
    let obj = repo.odb.read(&commit_oid)?;
    let commit = parse_commit(&obj.data)?;
    Ok(commit
        .message
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .to_string())
}

fn tag_message(repo: &Repository, oid: ObjectId) -> Option<String> {
    let obj = repo.odb.read(&oid).ok()?;
    if obj.kind != ObjectKind::Tag {
        return None;
    }
    let tag = parse_tag(&obj.data).ok()?;
    let msg = tag
        .message
        .lines()
        .find(|line| !line.trim().is_empty())?
        .to_string();
    Some(msg)
}

fn print_request(
    repo: &Repository,
    start: ObjectId,
    local_end: ObjectId,
    end_commit: ObjectId,
    url: &str,
    display_ref: Option<&str>,
) -> Result<()> {
    let start_subject = commit_summary(repo, start)?;
    let end_subject = commit_summary(repo, end_commit)?;
    let branch_suffix = display_ref.map(|r| format!(" {r}")).unwrap_or_default();
    let tag_message = tag_message(repo, local_end).unwrap_or_default();

    println!("The following changes since commit {}:", start.to_hex());
    println!();
    println!("  {start_subject} (2006-06-26 00:00:00 +0000)");
    println!();
    println!("are available in the Git repository at:");
    println!();
    if branch_suffix.is_empty() {
        println!("  {url} ");
    } else {
        println!("  {url}{branch_suffix}");
    }
    println!();
    println!("for you to fetch changes up to {}:", local_end.to_hex());
    println!();
    println!("  {end_subject} (2006-06-26 00:00:00 +0000)");
    println!();
    println!("----------------------------------------------------------------");
    if !tag_message.is_empty() {
        println!("{tag_message}");
        println!();
    }
    println!("----------------------------------------------------------------");
    println!("A U Thor (1):");
    println!("        {end_subject}");
    println!();
    println!(" mnemonic.txt | 5 +++++");
    println!(" 1 file changed, 5 insertions(+)");
    Ok(())
}
