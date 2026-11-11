// SPDX-License-Identifier: MIT OR Apache-2.0
//! Git source composition and build coordination.
use std::{
    collections::BTreeSet,
    env, fs,
    path::Path,
    process::{Command, ExitCode},
};
use toml::Value;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
type Package = (String, String, String);
const COMPAT_PATH: &str = "compat/git";
const COMPAT_REVISION: &str = "e9019fcafe0040228b8631c30f97ae1adb61bcdc";
const REGISTRY: &str = "registry+https://github.com/rust-lang/crates.io-index";

fn parse(path: &Path) -> Result<Value> {
    Ok(fs::read_to_string(path)?.parse()?)
}

fn packages(value: &Value) -> Result<BTreeSet<Package>> {
    let mut result = BTreeSet::new();
    for package in value
        .get("package")
        .and_then(Value::as_array)
        .ok_or("missing lock packages")?
    {
        let field = |name| {
            package.get(name).and_then(Value::as_str)
                .ok_or_else(|| format!("missing package {name}"))
        };
        result.insert((
            field("name")?.into(),
            field("version")?.into(),
            package.get("source").and_then(Value::as_str).unwrap_or("").into(),
        ));
    }
    Ok(result)
}

// The donor lock is covered by verify_import's pinned file inventory. Every
// additional application dependency needs an explicit source-scope review.
fn approved_packages(root: &Path) -> Result<BTreeSet<Package>> {
    for (path, expected) in [
        ("vendor/sley/Cargo.lock", "3219431ebac4b8b3cea13c8cdff45ede19366e3e"),
        ("xtask/Cargo.lock", "5f8713c1f68b37c7a1af10fba004445c49cfcd92"),
    ] {
        let output = Command::new("git").args(["hash-object", "--", path])
            .current_dir(root).output()?;
        if !output.status.success() || String::from_utf8(output.stdout)?.trim() != expected {
            return Err(format!("reviewed dependency inventory changed: {path}").into());
        }
    }
    let mut approved = packages(&parse(&root.join("vendor/sley/Cargo.lock"))?)?;
    approved.extend(packages(&parse(&root.join("xtask/Cargo.lock"))?)?);
    approved.insert(("rit".into(), "0.2.0".into(), "".into()));
    approved.insert(("sha2".into(), "0.11.1".into(), REGISTRY.into()));
    Ok(approved)
}

fn manifest(value: &Value, approved: &BTreeSet<Package>) -> Result<()> {
    if let Some(table) = value.as_table() {
        for (key, item) in table {
            if ["dependencies", "dev-dependencies", "build-dependencies"].contains(&key.as_str()) {
                for (alias, spec) in item.as_table().ok_or("dependency section must be a table")? {
                    let package = spec.get("package").and_then(Value::as_str).unwrap_or(alias);
                    if !approved.iter().any(|(name, _, _)| name == package) {
                        return Err(format!("dependency outside reviewed Git composition: {alias}").into());
                    }
                }
            } else {
                manifest(item, approved)?;
            }
        }
    }
    Ok(())
}

fn lock(value: &Value, approved: &BTreeSet<Package>) -> Result<()> {
    for package in packages(value)? {
        if !approved.contains(&package) {
            return Err(format!("locked package outside reviewed Git composition: {} {}", package.0, package.1).into());
        }
    }
    Ok(())
}

fn git_engine(value: &Value) -> Result<()> {
    let engine = value.get("dependencies")
        .and_then(|dependencies| dependencies.get("sley-cli"))
        .and_then(|spec| spec.get("path"))
        .and_then(Value::as_str);
    if engine != Some("vendor/sley/crates/sley-cli") {
        return Err("the Git engine must use the recorded Sley source import".into());
    }
    Ok(())
}

fn gitlink(name: &str, mode: &str, revision: &str) -> Result<bool> {
    if mode != "160000" {
        return Ok(false);
    }
    if name != COMPAT_PATH || revision != COMPAT_REVISION {
        return Err(format!("unreviewed source connection: {name} {revision}").into());
    }
    Ok(true)
}

fn index(root: &Path) -> Result<String> {
    let output = Command::new("git").args(["ls-files", "-s", "-z"])
        .current_dir(root).output()?;
    if !output.status.success() {
        return Err("Git source inventory failed".into());
    }
    Ok(String::from_utf8(output.stdout)?)
}

fn policy(root: &Path) -> Result<()> {
    let approved = approved_packages(root)?;
    let records = index(root)?;
    let mut connections = 0;
    for record in records.split('\0').filter(|record| !record.is_empty()) {
        let (meta, name) = record.split_once('\t').ok_or("invalid index record")?;
        let mut fields = meta.split_whitespace();
        let mode = fields.next().ok_or("missing mode")?;
        let revision = fields.next().ok_or("missing object identity")?;
        if gitlink(name, mode, revision)? {
            connections += 1;
            continue;
        }
        let path = root.join(name);
        match path.file_name().and_then(|name| name.to_str()) {
            Some("Cargo.toml") => manifest(&parse(&path)?, &approved)?,
            Some("Cargo.lock") => lock(&parse(&path)?, &approved)?,
            _ => {}
        }
    }
    if connections != 1 {
        return Err("the reviewed Git compatibility source connection is required".into());
    }
    let output = Command::new("git").args([
        "config", "--file", ".gitmodules", "--get-regexp", "^submodule\\.",
    ]).current_dir(root).output()?;
    if !output.status.success() {
        return Err("missing Git source connection metadata".into());
    }
    let mut settings = String::from_utf8(output.stdout)?.lines()
        .map(str::to_owned).collect::<Vec<_>>();
    settings.sort();
    let mut expected = vec![
        "submodule.git.path compat/git".to_owned(),
        "submodule.git.url https://github.com/git/git.git".to_owned(),
    ];
    expected.sort();
    if settings != expected {
        return Err("Git source connection metadata differs from the reviewed source".into());
    }
    git_engine(&parse(&root.join("Cargo.toml"))?)?;
    if !packages(&parse(&root.join("Cargo.lock"))?)?.iter().any(|(name, _, _)| name == "sley-cli") {
        return Err("Sley is missing from the application dependency lock".into());
    }
    println!("Git composition verified: reviewed native engine, dependencies and pinned compatibility source");
    Ok(())
}

// This inventory classifies tracked source. Imported Git is a separately
// licensed source connection; native dependencies and generated shell fixtures
// remain covered by their source and license records.
fn audit() -> Result<()> {
    let records = index(Path::new("."))?;
    println!("mode\tblob\tclass\tpath");
    for record in records.split('\0').filter(|record| !record.is_empty()) {
        let (meta, name) = record.split_once('\t').ok_or("invalid index record")?;
        let mut fields = meta.split_whitespace();
        let mode = fields.next().ok_or("missing mode")?;
        let blob = fields.next().ok_or("missing blob")?;
        if gitlink(name, mode, blob)? {
            println!("{mode}\t{blob}\tupstream-git-compatibility\t{name}");
            continue;
        }
        if mode != "100644" {
            return Err(format!("review executable or symlink: {name}").into());
        }
        let path = Path::new(name);
        let ext = path.extension().and_then(|extension| extension.to_str()).unwrap_or("");
        let class = match ext {
            "rs" if name.starts_with("vendor/sley/") => "rust/compatibility-fixtures",
            "rs" => "rust",
            "md" | "log" | "txt" | "csv" | "tsv" | "tap" => "prose/evidence",
            "toml" | "lock" | "json" | "yaml" | "yml" | "upstream" => "declarative/data",
            "" | "1" => {
                if ![
                    "COPYING", "LGPL-2.1", "LICENSE", "LICENSE-MIT", "LICENSE-APACHE",
                    ".gitignore", ".gitattributes", ".editorconfig", ".mailmap", ".gitmodules",
                ].contains(&path.file_name().and_then(|name| name.to_str()).unwrap_or("")) {
                    return Err(format!("unclassified extensionless path: {name}").into());
                }
                "notice/metadata"
            }
            _ => return Err(format!("unclassified source/data: {name}").into()),
        };
        let bytes = fs::read(path)?;
        if bytes.starts_with(b"#!") && ext != "rs" {
            return Err(format!("unreviewed executable source: {name}").into());
        }
        println!("{mode}\t{blob}\t{class}\t{name}");
    }
    Ok(())
}
fn run(c: &mut Command) -> Result<()> {
    if !c.status()?.success() {
        return Err("command failed".into());
    }
    Ok(())
}
fn qualify() -> Result<()> {
    let root = env::var("BLUEFE_ROOT")?;
    let root = Path::new(&root);
    if !root.is_absolute() || !root.is_dir() {
        return Err("BLUEFE_ROOT must name the reviewed absolute bundle".into());
    }
    let bin = |name: &str| {
        root.join("bin")
            .join(format!("{name}{}", env::consts::EXE_SUFFIX))
    };
    let version = Command::new(bin("rustc")).arg("-vV").output()?;
    if !version.status.success()
        || !String::from_utf8_lossy(&version.stdout)
            .contains("commit-hash: 7005dc02c9fb6674380eade443022bbc04215276")
    {
        return Err("wrong produced compiler".into());
    }
    print!("{}", String::from_utf8_lossy(&version.stdout));
    for args in [
        vec!["build", "--locked", "--all-features", "--all-targets"],
        vec!["test", "--locked", "--all-features", "--all-targets"],
        vec!["test", "--locked", "--doc", "--all-features"],
        vec![
            "test",
            "--manifest-path",
            "vendor/sley/Cargo.toml",
            "--locked",
            "--lib",
            "-p",
            "sley-core",
            "-p",
            "sley-object",
            "-p",
            "sley-index",
            "-p",
            "sley-refs",
            "-p",
            "sley-odb",
            "-p",
            "sley-pack",
            "-p",
            "sley-protocol",
            "-p",
            "sley-worktree",
        ],
        vec![
            "test",
            "--manifest-path",
            "vendor/sley/Cargo.toml",
            "--locked",
            "-p",
            "sley-cli",
            "--test",
            "upload_archive",
        ],
        vec!["run", "--locked", "--example", "verify_import"],
        vec!["test", "--manifest-path", "xtask/Cargo.toml", "--locked"],
    ] {
        run(Command::new(bin("lustr"))
            .arg("--bluefe-root")
            .arg(root)
            .args(args)
            .env("RIT_BACKEND", "native")
            .env("RUSTC", bin("rustc"))
            .env("RUSTDOC", bin("rustdoc"))
            .env(
                "CARGO_ENCODED_RUSTFLAGS",
                format!("--sysroot\x1f{}", root.display()),
            )
            .env("RUSTDOCFLAGS", format!("--sysroot {}", root.display()))
            .env("CARGO_BUILD_JOBS", "2"))?;
    }
    policy(Path::new("."))?;
    audit()
}
fn main() -> ExitCode {
    let result = match env::args().nth(1).as_deref() {
        Some("policy") => policy(Path::new(".")),
        Some("qualify") => qualify(),
        Some("audit") => audit(),
        _ => Err("usage: rit-xtask policy|qualify|audit".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(text: &str) -> Value {
        text.parse().unwrap()
    }

    fn approved() -> BTreeSet<Package> {
        BTreeSet::from([("engine".into(), "1.0.0".into(), REGISTRY.into())])
    }

    #[test]
    fn dependency_aliases_and_nested_sections_need_review() {
        for section in ["dependencies", "dev-dependencies", "build-dependencies",
            "workspace.dependencies", "target.'cfg(unix)'.dependencies"] {
            manifest(&value(&format!("[{section}]\nname={{package='engine',version='1'}}")), &approved()).unwrap();
            for spec in ["unreviewed='1'", "engine={package='unreviewed',version='1',optional=true}"] {
                assert!(manifest(&value(&format!("[{section}]\n{spec}")), &approved()).is_err());
            }
        }
    }

    #[test]
    fn transitive_versions_and_sources_need_review() {
        lock(&value(&format!("[[package]]\nname='engine'\nversion='1.0.0'\nsource='{REGISTRY}'")), &approved()).unwrap();
        for package in [
            "name='unreviewed'\nversion='1.0.0'",
            "name='engine'\nversion='2.0.0'",
            "name='engine'\nversion='1.0.0'\nsource='git+https://example.invalid/source'",
        ] {
            assert!(lock(&value(&format!("[[package]]\n{package}")), &approved()).is_err());
        }
    }

    #[test]
    fn only_exact_reviewed_git_source_connection_is_allowed() {
        assert!(gitlink(COMPAT_PATH, "160000", COMPAT_REVISION).unwrap());
        assert!(gitlink("other", "160000", COMPAT_REVISION).is_err());
        assert!(gitlink(COMPAT_PATH, "160000", "0000000000000000000000000000000000000000").is_err());
        assert!(!gitlink("README.md", "100644", COMPAT_REVISION).unwrap());
    }

    #[test]
    fn application_requires_the_imported_native_engine() {
        git_engine(&value("[dependencies]\nsley-cli={path='vendor/sley/crates/sley-cli'}")).unwrap();
        assert!(git_engine(&value("[dependencies]\nsley-cli='0.13'")).is_err());
    }
}
