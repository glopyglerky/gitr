//! Storage-policy and build coordination. Never executes repository-provided scripts.
use std::{
    env, fs,
    path::Path,
    process::{Command, ExitCode},
};
use toml::Value;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const OMDB_REPOSITORY: &str = "https://github.com/glopyglerky/omdb";
const OMDB_REVISION: &str = "4c7c6c35d61372d0cd538ef3824a2f47eed2c4d5";
const OMDB_VERSION: &str = "4.5.0";
const RETIRED: &[&str] = &[
    "rust/gitr-store/",
    "rust/gitr-surreal-probe/",
    "src/actions_remote.rs",
    "ci/gitr-store-",
    "ci/gitr-surreal-",
    ".github/workflows/gitr-store-qualification.yml",
    ".github/workflows/gitr-surreal-qualification.yml",
];
fn forbidden(name: &str) -> bool {
    name.to_ascii_lowercase()
        .replace('_', "-")
        .starts_with("surreal")
}
fn manifest(v: &Value) -> Result<()> {
    if let Some(table) = v.as_table() {
        for (key, item) in table {
            if ["dependencies", "dev-dependencies", "build-dependencies"].contains(&key.as_str()) {
                let deps = item
                    .as_table()
                    .ok_or("dependency section must be a table")?;
                for (alias, spec) in deps {
                    if forbidden(alias)
                        || spec
                            .get("package")
                            .and_then(Value::as_str)
                            .is_some_and(forbidden)
                    {
                        return Err(format!("retired dependency: {alias}").into());
                    }
                }
            } else {
                manifest(item)?;
            }
        }
    }
    Ok(())
}
fn lock(v: &Value) -> Result<Vec<(String, String)>> {
    let mut packages = Vec::new();
    for p in v
        .get("package")
        .and_then(Value::as_array)
        .ok_or("missing lock packages")?
    {
        let name = p
            .get("name")
            .and_then(Value::as_str)
            .ok_or("missing package name")?;
        if forbidden(name) {
            return Err(format!("retired locked dependency: {name}").into());
        }
        packages.push((
            name.into(),
            p.get("version")
                .and_then(Value::as_str)
                .ok_or("missing version")?
                .into(),
        ));
    }
    Ok(packages)
}
fn pin(m: &Value, l: &Value) -> Result<()> {
    manifest(m)?;
    let spec = m
        .get("dependencies")
        .and_then(|d| d.get("redb"))
        .ok_or("redb required")?;
    if spec.get("optional").and_then(Value::as_bool) == Some(true) {
        return Err("redb must be required".into());
    }
    let version = spec
        .as_str()
        .or_else(|| spec.get("version").and_then(Value::as_str))
        .ok_or("missing redb version")?;
    if version != "=4.5.0" {
        return Err("retain reviewed exact redb 4.5.0 pin".into());
    }
    let versions: Vec<_> = lock(l)?
        .into_iter()
        .filter(|(n, _)| n == "redb")
        .map(|(_, v)| v)
        .collect();
    if versions != ["4.5.0"] {
        return Err("redb lock pin mismatch".into());
    }
    Ok(())
}
fn git_first(m: &Value, l: &Value) -> Result<()> {
    manifest(m)?;
    let engine = m
        .get("dependencies")
        .and_then(|d| d.get("sley-cli"))
        .and_then(|d| d.get("path"))
        .and_then(Value::as_str);
    if engine != Some("vendor/sley/crates/sley-cli") {
        return Err("the active Git engine must use the recorded Sley source import".into());
    }
    let packages = lock(l)?;
    if !packages.iter().any(|(name, _)| name == "sley-cli") {
        return Err("Sley is missing from the active dependency lock".into());
    }
    if packages.iter().any(|(name, _)| name == "grit-git") {
        return Err("GPL-2.0-only grit-git cannot be linked with Apache-2.0 Sley".into());
    }
    omdb_pin(m, l)?;
    Ok(())
}
fn omdb_pin(m: &Value, l: &Value) -> Result<()> {
    let spec = m
        .get("dependencies")
        .and_then(|dependencies| dependencies.get("omdb"))
        .ok_or("the snapshot catalog requires the pinned omdb dependency")?;
    for (key, expected) in [
        ("package", "redb"),
        ("version", "=4.5.0"),
        ("git", OMDB_REPOSITORY),
        ("rev", OMDB_REVISION),
    ] {
        if spec.get(key).and_then(Value::as_str) != Some(expected) {
            return Err(format!("omdb must retain the reviewed {key} pin").into());
        }
    }
    if spec.get("optional").and_then(Value::as_bool) == Some(true)
        || spec.get("default-features").and_then(Value::as_bool) == Some(false)
    {
        return Err("omdb must be required and retain its default std profile".into());
    }
    for key in [
        "path",
        "branch",
        "tag",
        "registry",
        "registry-index",
        "workspace",
    ] {
        if spec.get(key).is_some() {
            return Err(format!("omdb cannot override its reviewed source with {key}").into());
        }
    }
    if let Some(features) = spec.get("features") {
        if features
            .as_array()
            .ok_or("omdb features must be an array")?
            .iter()
            .any(|feature| feature.as_str() != Some("std"))
        {
            return Err(
                "omdb experimental or additional features need separate qualification".into(),
            );
        }
    }

    // Another alias or a target/dev dependency could unify unreviewed features.
    // The catalog owns the one direct declaration of this database package.
    fn declarations(value: &Value, expected: &Value) -> Result<usize> {
        let Some(table) = value.as_table() else {
            return Ok(0);
        };
        let mut count = 0;
        for (key, item) in table {
            if ["dependencies", "dev-dependencies", "build-dependencies"].contains(&key.as_str()) {
                for (alias, dependency) in item
                    .as_table()
                    .ok_or("dependency section must be a table")?
                {
                    if alias == "omdb"
                        || alias == "redb"
                        || dependency.get("package").and_then(Value::as_str) == Some("redb")
                    {
                        if alias != "omdb" || dependency != expected {
                            return Err("unreviewed additional redb/omdb declaration".into());
                        }
                        count += 1;
                    }
                }
            } else {
                count += declarations(item, expected)?;
            }
        }
        Ok(count)
    }
    if declarations(m, spec)? != 1 {
        return Err("omdb must have one direct dependency declaration".into());
    }
    if let Some(features) = m.get("features").and_then(Value::as_table) {
        for feature in features.values() {
            for forwarded in feature.as_array().ok_or("features must be arrays")? {
                if let Some(name) = forwarded.as_str()
                    && (name.starts_with("omdb/") || name.starts_with("omdb?/"))
                    && name != "omdb/std"
                    && name != "omdb?/std"
                {
                    return Err("unreviewed forwarded omdb feature".into());
                }
            }
        }
    }

    let packages: Vec<_> = l
        .get("package")
        .and_then(Value::as_array)
        .ok_or("missing lock packages")?
        .iter()
        .filter(|package| package.get("name").and_then(Value::as_str) == Some("redb"))
        .collect();
    let expected_source = format!("git+{OMDB_REPOSITORY}?rev={OMDB_REVISION}#{OMDB_REVISION}");
    if packages.len() != 1
        || packages[0].get("version").and_then(Value::as_str) != Some(OMDB_VERSION)
        || packages[0].get("source").and_then(Value::as_str) != Some(expected_source.as_str())
    {
        return Err("omdb lock must contain exactly the reviewed fork revision and version".into());
    }
    Ok(())
}
fn parse(p: &Path) -> Result<Value> {
    Ok(fs::read_to_string(p)?.parse()?)
}
fn active_path(name: &str) -> Result<()> {
    if RETIRED.iter().any(|p| name.starts_with(p)) {
        return Err(format!("retired active path: {name}").into());
    }
    Ok(())
}
fn policy(root: &Path) -> Result<()> {
    let output = Command::new("git")
        .args([
            "ls-files",
            "-z",
            "--",
            "Cargo.toml",
            "Cargo.lock",
            "src",
            "crates",
            "vendor",
            "ci",
            "xtask",
            ".github/workflows",
        ])
        .current_dir(root)
        .output()?;
    if !output.status.success() {
        return Err("git inventory failed".into());
    }
    for name in std::str::from_utf8(&output.stdout)?
        .split('\0')
        .filter(|s| !s.is_empty())
    {
        active_path(name)?;
        let path = root.join(name);
        match path.file_name().and_then(|s| s.to_str()) {
            Some("Cargo.toml") => manifest(&parse(&path)?)?,
            Some("Cargo.lock") => {
                lock(&parse(&path)?)?;
            }
            _ => {}
        }
    }
    git_first(
        &parse(&root.join("Cargo.toml"))?,
        &parse(&root.join("Cargo.lock"))?,
    )?;
    let archived = root.join("Documentation/gitr/archive/database-first");
    pin(
        &parse(&archived.join("Cargo.toml"))?,
        &parse(&archived.join("Cargo.lock"))?,
    )?;
    println!(
        "storage policy: Sley Git engine; pinned omdb snapshot catalog; archived redb 4.5.0 preserved; no active Surreal dependencies or grit-git linkage"
    );
    Ok(())
}
// This is a tracked-file and mode inventory, not a parser proving the absence
// of native dependencies or foreign programs inside Rust strings. Sley retains
// documented shell compatibility adapters and generated hook/filter fixtures.
fn audit() -> Result<()> {
    let output = Command::new("git")
        .args(["ls-files", "-s", "-z"])
        .output()?;
    if !output.status.success() {
        return Err("git inventory failed".into());
    }
    println!("mode\tblob\tclass\tpath");
    for record in std::str::from_utf8(&output.stdout)?
        .split('\0')
        .filter(|r| !r.is_empty())
    {
        let (meta, name) = record.split_once('\t').ok_or("invalid index record")?;
        let mode = meta.split_whitespace().next().ok_or("missing mode")?;
        let blob = meta.split_whitespace().nth(1).ok_or("missing blob")?;
        if mode != "100644" {
            return Err(format!("review executable/symlink/submodule: {name}").into());
        }
        let path = Path::new(name);
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let class = match ext {
            "rs" if name.starts_with("vendor/sley/") => "rust/compatibility-fixtures",
            "rs" => "rust",
            "md" | "log" | "txt" | "csv" | "tsv" | "tap" => "prose/evidence",
            "toml" | "lock" | "json" | "yaml" | "yml" | "upstream" => "declarative/data",
            "" | "1" => {
                if ![
                    "COPYING",
                    "LGPL-2.1",
                    "LICENSE",
                    "LICENSE-MIT",
                    "LICENSE-APACHE",
                    ".gitignore",
                    ".gitattributes",
                    ".editorconfig",
                    ".mailmap",
                ]
                .contains(&path.file_name().and_then(|n| n.to_str()).unwrap_or(""))
                {
                    return Err(format!("unclassified extensionless path: {name}").into());
                }
                "notice/metadata"
            }
            _ => return Err(format!("unclassified source/data: {name}").into()),
        };
        let bytes = fs::read(path)?;
        if bytes.starts_with(b"#!") && ext != "rs" {
            return Err(format!("foreign shebang: {name}").into());
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
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn value(s: &str) -> Value {
        s.parse().unwrap()
    }
    #[test]
    fn retired_paths() {
        for p in RETIRED {
            assert!(active_path(p).is_err());
        }
        active_path("src/actions.rs").unwrap();
    }
    #[test]
    fn accepted_redb() {
        pin(
            &value("[dependencies]\nredb='=4.5.0'"),
            &value("[[package]]\nname='redb'\nversion='4.5.0'"),
        )
        .unwrap();
    }
    #[test]
    fn forbidden_alias_optional_target_workspace() {
        for section in [
            "dependencies",
            "dev-dependencies",
            "build-dependencies",
            "workspace.dependencies",
            "target.'cfg(unix)'.dependencies",
        ] {
            for spec in [
                "surrealdb='4'",
                "db={package='surrealkv',optional=true,version='1'}",
            ] {
                assert!(manifest(&value(&format!("[{section}]\n{spec}"))).is_err());
            }
        }
    }
    #[test]
    fn forbidden_transitive_lock() {
        assert!(lock(&value("[[package]]\nname='surrealdb-core'\nversion='4'")).is_err());
    }
    #[test]
    fn exact_required_matching_pin() {
        let l = value("[[package]]\nname='redb'\nversion='4.5.0'");
        for spec in ["'4.5'", "'=4.6.0'", "{version='=4.5.0',optional=true}"] {
            assert!(pin(&value(&format!("[dependencies]\nredb={spec}")), &l).is_err());
        }
        assert!(
            pin(
                &value("[dependencies]\nredb='=4.5.0'"),
                &value("[[package]]\nname='redb'\nversion='4.4.0'")
            )
            .is_err()
        );
    }
    #[test]
    fn sley_build_requires_recorded_engine_and_separate_old_composition() {
        let manifest = omdb_manifest();
        let accepted = omdb_lock();
        git_first(&manifest, &accepted).unwrap();
        for name in ["grit-git", "surrealdb"] {
            let mut changed = accepted.clone();
            changed["package"]
                .as_array_mut()
                .unwrap()
                .push(value(&format!("name='{name}'\nversion='1.0.0'")));
            assert!(git_first(&manifest, &changed).is_err());
        }
        let mut changed = manifest.clone();
        changed["dependencies"]["sley-cli"] = Value::String("0.13".into());
        assert!(git_first(&changed, &accepted).is_err());
        assert!(
            git_first(
                &manifest,
                &value("[[package]]\nname='other'\nversion='1.0.0'")
            )
            .is_err()
        );
    }

    fn omdb_manifest() -> Value {
        value(&format!(
            "[dependencies]\nsley-cli={{path='vendor/sley/crates/sley-cli'}}\n\
             omdb={{package='redb',version='=4.5.0',git='{OMDB_REPOSITORY}',rev='{OMDB_REVISION}'}}"
        ))
    }

    fn omdb_lock() -> Value {
        value(&format!(
            "[[package]]\nname='sley-cli'\nversion='0.13.0'\n\
             [[package]]\nname='redb'\nversion='{OMDB_VERSION}'\n\
             source='git+{OMDB_REPOSITORY}?rev={OMDB_REVISION}#{OMDB_REVISION}'"
        ))
    }

    #[test]
    fn omdb_rejects_registry_or_different_fork_even_at_the_same_version() {
        let accepted = omdb_lock();
        for (key, replacement) in [
            ("package", "other"),
            ("git", "https://github.com/cberner/redb"),
            ("rev", "9273718b3ba5282580ca9035b0fb95b5bf13988f"),
            ("version", "4.5"),
        ] {
            let mut manifest = omdb_manifest();
            manifest["dependencies"]["omdb"][key] = Value::String(replacement.into());
            assert!(omdb_pin(&manifest, &accepted).is_err(), "accepted {key}");
        }
        let mut registry = accepted.clone();
        registry["package"][1]["source"] =
            Value::String("registry+https://github.com/rust-lang/crates.io-index".into());
        assert!(omdb_pin(&omdb_manifest(), &registry).is_err());
        let mut duplicate = accepted.clone();
        let another = duplicate["package"][1].clone();
        duplicate["package"].as_array_mut().unwrap().push(another);
        assert!(omdb_pin(&omdb_manifest(), &duplicate).is_err());
        let mut wrong_version = accepted.clone();
        wrong_version["package"][1]["version"] = Value::String("4.6.0".into());
        assert!(omdb_pin(&omdb_manifest(), &wrong_version).is_err());
    }

    #[test]
    fn omdb_requires_std_and_rejects_unqualified_features_or_source_overrides() {
        for extra in [
            "optional=true",
            "default-features=false",
            "features=['experimental-api-5']",
            "features=['std','experimental-niches']",
            "features=['logging']",
            "path='vendor/other'",
            "branch='master'",
            "tag='v4.5.0'",
            "registry='other'",
            "workspace=true",
        ] {
            let mut manifest = omdb_manifest();
            let additions = value(extra);
            manifest["dependencies"]["omdb"]
                .as_table_mut()
                .unwrap()
                .extend(additions.as_table().unwrap().clone());
            assert!(
                omdb_pin(&manifest, &omdb_lock()).is_err(),
                "accepted {extra}"
            );
        }
        let mut explicit_std = omdb_manifest();
        explicit_std["dependencies"]["omdb"]
            .as_table_mut()
            .unwrap()
            .insert(
                "features".into(),
                value("features=['std']")["features"].clone(),
            );
        omdb_pin(&explicit_std, &omdb_lock()).unwrap();
    }

    #[test]
    fn omdb_cannot_gain_experimental_features_through_other_root_declarations() {
        for addition in [
            "[dev-dependencies]\nalternate={package='redb',version='4.5',features=['experimental-api-5']}",
            "[target.'cfg(unix)'.dependencies]\nredb='4.5'",
            "[features]\npreview=['omdb/experimental-api-5']",
            "[features]\npreview=['omdb?/experimental-api-5']",
        ] {
            let mut manifest = omdb_manifest();
            manifest
                .as_table_mut()
                .unwrap()
                .extend(value(addition).as_table().unwrap().clone());
            assert!(
                omdb_pin(&manifest, &omdb_lock()).is_err(),
                "accepted {addition}"
            );
        }
        let mut missing = omdb_manifest();
        missing["dependencies"]
            .as_table_mut()
            .unwrap()
            .remove("omdb");
        assert!(git_first(&missing, &omdb_lock()).is_err());
    }
}
