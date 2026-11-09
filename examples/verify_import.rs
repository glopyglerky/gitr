//! Validate the historical Grit import's local bytes and complete inventory.
//!
//! Run from the repository: `lustr run --example verify_import --locked`.
//! `--refresh` updates only local hashes, modification flags, local rename
//! evidence, and added-file records. Upstream identities and notices are never
//! rewritten. This verifies recorded provenance; it does not fetch or establish
//! the authenticity of the upstream revision independently.

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::Command,
};

const IMPORT: &str = "vendor/grit";
const MANIFEST: &str = "upstream-source.json";
const REPOSITORY: &str = "https://github.com/gitbutlerapp/grit";
const REVISION: &str = "e197103087b1110db2d629c4c269ef9258508169";
const OLD_INCLUDE: &str = "grit-git/src/commands/_submodule_run_update_inner.rs.inc";
const NEW_INCLUDE: &str = "grit-git/src/commands/_submodule_run_update_inner.rs";

fn main() -> Result<()> {
    let mut refresh = false;
    for argument in std::env::args_os().skip(1) {
        if argument == "--refresh" && !refresh {
            refresh = true;
        } else if argument == "--help" || argument == "-h" {
            println!("verify_import [--refresh]\nValidate exact import inventory and local hashes; --refresh records reviewed local changes without changing upstream identities.");
            return Ok(());
        } else {
            bail!("unexpected or repeated argument: {}", argument.to_string_lossy());
        }
    }

    let root = repository_root()?;
    let import = root.join(IMPORT);
    let manifest_path = checked_file(&root, &format!("{IMPORT}/{MANIFEST}"))?;
    let original = fs::read(&manifest_path).context("read import manifest")?;
    let mut document: Value = serde_json::from_slice(&original).context("parse import manifest")?;
    ensure!(document.is_object(), "manifest must be a JSON object");
    ensure!(document["schema_version"].as_u64() == Some(1), "unsupported import manifest schema");
    ensure!(document["repository"].as_str() == Some(REPOSITORY), "unexpected upstream repository");
    ensure!(document["revision"].as_str() == Some(REVISION), "unexpected upstream revision; a new donor requires a reviewed provenance update");

    let inventory = git_inventory(&root)?;
    let mut claimed = BTreeSet::new();
    let mut upstream_paths = BTreeSet::new();
    let mut hashes = BTreeMap::new();
    let mut modified = 0;

    let exclusions = document["excluded_package_files"].as_array()
        .context("excluded_package_files must be an array")?;
    let mut excluded = BTreeSet::new();
    for entry in exclusions {
        let path = entry.as_str().context("excluded package path must be a string")?;
        relative_path(path)?;
        ensure!(excluded.insert(path.to_owned()), "duplicate excluded path: {path}");
    }

    let originals = document["files"].as_array_mut().context("files must be an array")?;
    ensure!(!originals.is_empty(), "the original source inventory is empty");
    for entry in originals.iter_mut() {
        let record = entry.as_object_mut().context("original file record must be an object")?;
        let upstream_path = field(record, "upstream_path")?.to_owned();
        relative_path(&upstream_path)?;
        ensure!(upstream_paths.insert(upstream_path.clone()), "duplicate upstream path: {upstream_path}");
        ensure!(!excluded.contains(&upstream_path), "excluded upstream file is also imported: {upstream_path}");
        let upstream_hash = field(record, "upstream_sha256")?.to_owned();
        hash_value(&upstream_hash, 64, "upstream_sha256")?;
        let mut local_path = field(record, "path")?.to_owned();
        if refresh && upstream_path == OLD_INCLUDE && local_path == OLD_INCLUDE {
            ensure!(!import.join(OLD_INCLUDE).try_exists()?, "both old include provenance and the old local path remain; resolve the rename explicitly");
            checked_file(&root, &format!("{IMPORT}/{NEW_INCLUDE}"))?;
            local_path = NEW_INCLUDE.to_owned();
            record.insert("path".to_owned(), Value::String(local_path.clone()));
        }
        claim(&mut claimed, &local_path)?;
        let actual = local_hash(&root, &local_path)?;
        // Original license notices are byte-for-byte redistributions, not
        // ordinary local implementation patches.
        if is_license_notice(&local_path) {
            ensure!(actual == upstream_hash, "original license notice changed: {local_path}");
        }
        let changed = actual != upstream_hash;
        if refresh {
            record.insert("local_sha256".to_owned(), Value::String(actual.clone()));
            record.insert("modified".to_owned(), Value::Bool(changed));
        } else {
            verify_local_hash(record, &local_path, &actual)?;
            ensure!(record.get("modified").and_then(Value::as_bool) == Some(changed), "incorrect or missing modified flag: {local_path}");
        }
        modified += usize::from(changed);
        hashes.insert(local_path, actual);
    }
    let original_count = originals.len();

    let notices = document["supplementary_notices"].as_array()
        .context("supplementary_notices must be an array")?;
    for entry in notices {
        let record = entry.as_object().context("supplementary notice must be an object")?;
        let local_path = field(record, "path")?;
        claim(&mut claimed, local_path)?;
        ensure!(field(record, "repository")?.starts_with("https://"), "notice repository must be an HTTPS origin");
        relative_path(field(record, "upstream_path")?)?;
        hash_value(field(record, "revision")?, 40, "notice revision")?;
        hash_value(field(record, "upstream_blob")?, 40, "notice upstream_blob")?;
        let expected = field(record, "sha256")?;
        hash_value(expected, 64, "notice sha256")?;
        let actual = local_hash(&root, local_path)?;
        ensure!(actual == expected, "supplementary license notice changed: {local_path}");
        hashes.insert(local_path.to_owned(), actual);
    }
    let notice_count = notices.len();

    if refresh && document.get("added_files").is_none() {
        document.as_object_mut().context("manifest must be an object")?
            .insert("added_files".to_owned(), Value::Array(Vec::new()));
    }
    let additions = document["added_files"].as_array_mut().context("added_files must be an array")?;
    for entry in additions.iter_mut() {
        let record = entry.as_object_mut().context("added file record must be an object")?;
        let local_path = field(record, "path")?.to_owned();
        claim(&mut claimed, &local_path)?;
        let actual = local_hash(&root, &local_path)?;
        if refresh {
            record.insert("local_sha256".to_owned(), Value::String(actual.clone()));
        } else {
            verify_local_hash(record, &local_path, &actual)?;
        }
        hashes.insert(local_path, actual);
    }
    if refresh {
        for local_path in inventory.difference(&claimed).cloned().collect::<Vec<_>>() {
            let actual = local_hash(&root, &local_path)?;
            additions.push(json!({"path": local_path.clone(), "local_sha256": actual.clone()}));
            claimed.insert(local_path.clone());
            hashes.insert(local_path, actual);
        }
        additions.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    }
    let added_count = additions.len();
    let missing: Vec<_> = claimed.difference(&inventory).collect();
    let unrecorded: Vec<_> = inventory.difference(&claimed).collect();
    ensure!(missing.is_empty(), "manifest files absent from Git inventory: {missing:?}");
    ensure!(unrecorded.is_empty(), "unrecorded import files: {unrecorded:?}; review the additions then run --refresh");

    // Reject concurrent editing during the audit instead of writing evidence
    // for a mixture of before/after states.
    ensure!(inventory == git_inventory(&root)?, "import inventory changed during validation");
    for (path, expected) in &hashes {
        ensure!(&local_hash(&root, path)? == expected, "file changed during validation: {path}");
    }
    ensure!(fs::read(&manifest_path)? == original, "manifest changed during validation");
    if refresh {
        let mut encoded = serde_json::to_vec_pretty(&document)?;
        encoded.push(b'\n');
        let mut temporary = tempfile::NamedTempFile::new_in(&import)?;
        temporary.write_all(&encoded)?;
        temporary.as_file().sync_all()?;
        temporary.persist(&manifest_path).context("publish refreshed manifest")?;
        #[cfg(unix)]
        File::open(&import)?.sync_all()?;
    }
    println!("{} {original_count} original files ({modified} modified), {added_count} added files and {notice_count} supplementary notices; exact inventory and local SHA-256 hashes match.", if refresh { "Refreshed" } else { "Validated" });
    Ok(())
}

fn field<'a>(record: &'a Map<String, Value>, name: &str) -> Result<&'a str> {
    let value = record.get(name).and_then(Value::as_str)
        .with_context(|| format!("missing or non-string field: {name}"))?;
    ensure!(!value.is_empty(), "empty field: {name}");
    Ok(value)
}

fn hash_value(value: &str, length: usize, field: &str) -> Result<()> {
    ensure!(value.len() == length && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)), "invalid lowercase hexadecimal {field}");
    Ok(())
}

fn relative_path(value: &str) -> Result<&Path> {
    ensure!(!value.is_empty() && !value.contains(['\\', '\0', ':']), "unsafe import path: {value:?}");
    ensure!(value.split('/').all(|part| !part.is_empty() && part != "." && part != ".."), "non-canonical import path: {value:?}");
    let path = Path::new(value);
    ensure!(path.components().all(|part| matches!(part, Component::Normal(_))), "absolute or traversing import path: {value:?}");
    Ok(path)
}

fn excluded_local_path(path: &str) -> bool {
    path == MANIFEST || path.split('/').any(|part| part == "target")
}

fn claim(claimed: &mut BTreeSet<String>, local_path: &str) -> Result<()> {
    relative_path(local_path)?;
    ensure!(!excluded_local_path(local_path), "manifest includes its own file or a build target: {local_path}");
    ensure!(claimed.insert(local_path.to_owned()), "duplicate local import path: {local_path}");
    Ok(())
}

fn is_license_notice(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name == "COPYING" || name.starts_with("LICENSE")
}

fn checked_file(root: &Path, relative: &str) -> Result<PathBuf> {
    let path = relative_path(relative)?;
    let mut current = root.to_path_buf();
    let mut parts = path.components().peekable();
    while let Some(part) = parts.next() {
        current.push(part.as_os_str());
        let metadata = fs::symlink_metadata(&current)
            .with_context(|| format!("inspect import path {}", current.display()))?;
        ensure!(!metadata.file_type().is_symlink(), "symlink in import path: {}", current.display());
        if parts.peek().is_some() {
            ensure!(metadata.is_dir(), "non-directory import ancestor: {}", current.display());
        } else {
            ensure!(metadata.is_file(), "import entry is not a regular file: {}", current.display());
        }
    }
    Ok(current)
}

fn local_hash(root: &Path, relative: &str) -> Result<String> {
    let path = checked_file(root, &format!("{IMPORT}/{relative}"))?;
    let mut file = File::open(&path).with_context(|| format!("open {}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).with_context(|| format!("read {}", path.display()))?;
        if count == 0 { break; }
        digest.update(&buffer[..count]);
    }
    checked_file(root, &format!("{IMPORT}/{relative}"))?;
    Ok(digest.finalize().iter().map(|byte| format!("{byte:02x}")).collect())
}

fn verify_local_hash(record: &Map<String, Value>, path: &str, actual: &str) -> Result<()> {
    let expected = field(record, "local_sha256")?;
    hash_value(expected, 64, "local_sha256")?;
    ensure!(expected == actual, "local SHA-256 mismatch: {path}");
    Ok(())
}

fn git() -> Command {
    let mut command = Command::new("git");
    command.args(["--no-pager", "-c", "core.fsmonitor=false"]);
    for name in ["GIT_DIR", "GIT_WORK_TREE", "GIT_COMMON_DIR", "GIT_INDEX_FILE", "GIT_NAMESPACE"] {
        command.env_remove(name);
    }
    command
}

fn repository_root() -> Result<PathBuf> {
    let output = git().args(["rev-parse", "--show-toplevel"]).output()
        .context("run Git to locate repository")?;
    ensure!(output.status.success(), "Git could not locate repository: {}", String::from_utf8_lossy(&output.stderr));
    let root = std::str::from_utf8(&output.stdout).context("repository path is not UTF-8")?
        .trim_end_matches(['\n', '\r']);
    ensure!(!root.is_empty(), "Git returned an empty repository path");
    fs::canonicalize(root).context("resolve repository root")
}

fn git_inventory(root: &Path) -> Result<BTreeSet<String>> {
    let output = git().arg("-C").arg(root)
        .args(["ls-files", "-z", "--cached", "--others", "--exclude-standard", "--", IMPORT])
        .output().context("enumerate tracked and untracked import files")?;
    ensure!(output.status.success(), "Git inventory failed: {}", String::from_utf8_lossy(&output.stderr));
    let mut paths = BTreeSet::new();
    for bytes in output.stdout.split(|byte| *byte == 0).filter(|bytes| !bytes.is_empty()) {
        let path = std::str::from_utf8(bytes).context("import filename is not UTF-8")?;
        let local = path.strip_prefix("vendor/grit/").context("Git returned a path outside the import")?;
        relative_path(local)?;
        if excluded_local_path(local) { continue; }
        checked_file(root, path)?;
        ensure!(paths.insert(local.to_owned()), "duplicate Git inventory path (possibly an unmerged index): {local}");
    }
    // Upstream ignores lockfiles; the imported workspace's resolved build
    // closure is still mandatory provenance, even before it is force-added.
    checked_file(root, &format!("{IMPORT}/Cargo.lock"))?;
    paths.insert("Cargo.lock".to_owned());
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_traversal_and_noncanonical_paths() {
        for path in ["", "/etc/passwd", "../outside", "a/../b", "a//b", "./a", "a\\b", "C:/a", "a/", "a\0b"] {
            assert!(relative_path(path).is_err(), "{path:?}");
        }
        assert!(relative_path("grit-lib/src/odb/redb_store.rs").is_ok());
    }

    #[test]
    fn rejects_duplicate_claims_and_manifest_recursion() {
        let mut paths = BTreeSet::new();
        assert!(claim(&mut paths, "LICENSE").is_ok());
        assert!(claim(&mut paths, "LICENSE").is_err());
        assert!(claim(&mut paths, MANIFEST).is_err());
        assert!(claim(&mut paths, "grit-lib/target/build/generated.rs").is_err());
    }
}
