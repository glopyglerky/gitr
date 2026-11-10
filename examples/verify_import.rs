// SPDX-License-Identifier: MIT OR Apache-2.0
//! Verify the complete imported Sley tree against its recorded source inventory.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    error::Error,
    fs,
    path::{Component, Path, PathBuf},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn checked_relative(value: &str) -> Result<&Path> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(format!("invalid inventory path: {value}").into());
    }
    Ok(path)
}

fn collect(root: &Path, directory: &Path, files: &mut BTreeSet<String>) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let path = entry.path();
        if kind.is_dir() {
            if path
                .file_name()
                .is_some_and(|name| name == "target" || name == ".git")
            {
                continue;
            }
            collect(root, &path, files)?;
        } else if kind.is_file() {
            let relative = path
                .strip_prefix(root)?
                .to_str()
                .ok_or("non-UTF8 inventory path")?;
            files.insert(relative.replace('\\', "/"));
        } else {
            return Err(format!("unreviewed symlink or special file: {}", path.display()).into());
        }
    }
    Ok(())
}

fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing {key}").into())
}

fn main() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest: Value =
        serde_json::from_slice(&fs::read(root.join("Documentation/gitr/sley-import.json"))?)?;
    if text(&manifest, "revision")? != "a71654f50de24b3c750f0df5f1829db7342491f6"
        || text(&manifest, "license")? != "Apache-2.0"
    {
        return Err("unreviewed donor revision or license".into());
    }
    let donor = root.join(checked_relative(text(&manifest, "destination")?)?);
    let mut expected = BTreeSet::new();
    let mut patched = 0;
    let mut added = 0;
    for file in manifest
        .get("files")
        .and_then(Value::as_array)
        .ok_or("missing files")?
    {
        let name = text(file, "path")?;
        if !expected.insert(name.to_owned()) {
            return Err(format!("duplicate inventory entry: {name}").into());
        }
        let bytes = fs::read(donor.join(checked_relative(name)?))?;
        if digest(&bytes) != text(file, "sha256")? {
            return Err(format!("import hash mismatch: {name}").into());
        }
        if file.get("upstream_blob").is_some_and(Value::is_string) {
            let blob = text(file, "upstream_blob")?;
            let original = text(file, "upstream_sha256")?;
            if blob.len() != 40
                || !blob.bytes().all(|byte| byte.is_ascii_hexdigit())
                || original.len() != 64
                || !original.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(format!("invalid source identity: {name}").into());
            }
            if original != text(file, "sha256")? {
                if text(file, "change")?.trim().is_empty() {
                    return Err(format!("missing adaptation reason: {name}").into());
                }
                patched += 1;
            }
        } else {
            if text(file, "origin")?.trim().is_empty() {
                return Err(format!("missing local origin: {name}").into());
            }
            added += 1;
        }
    }
    let mut actual = BTreeSet::new();
    collect(&donor, &donor, &mut actual)?;
    if expected != actual {
        return Err(format!(
            "inventory differs: missing={:?}, unrecorded={:?}",
            expected.difference(&actual).collect::<Vec<_>>(),
            actual.difference(&expected).collect::<Vec<_>>()
        )
        .into());
    }
    let lock = fs::read_to_string(root.join("Cargo.lock"))?;
    if lock.contains("name = \"grit-git\"") {
        return Err("historical GPL-only engine is linked into the active lock".into());
    }
    // Cargo emits canonical package blocks. Permit only the reviewed omdb
    // fork, not a registry redb or a second database revision. xtask's TOML
    // policy separately checks the manifest alias, features and archived pin.
    let databases: Vec<_> = lock
        .split("[[package]]")
        .filter(|package| package.lines().any(|line| line.trim() == "name = \"redb\""))
        .collect();
    let source = "source = \"git+https://github.com/glopyglerky/omdb?rev=4c7c6c35d61372d0cd538ef3824a2f47eed2c4d5#4c7c6c35d61372d0cd538ef3824a2f47eed2c4d5\"";
    if databases.len() != 1
        || !databases[0]
            .lines()
            .any(|line| line.trim() == "version = \"4.5.0\"")
        || !databases[0].lines().any(|line| line.trim() == source)
    {
        return Err("active database must be the exact reviewed omdb fork revision".into());
    }
    println!(
        "Sley source verified: {} files, {patched} adapted, {added} added; exact donor revision and complete installed inventory",
        expected.len()
    );
    Ok(())
}
