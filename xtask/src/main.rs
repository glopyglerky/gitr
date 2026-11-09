//! Storage-policy and build coordination. Never executes repository-provided scripts.
use std::{env, fs, path::Path, process::{Command, ExitCode}};
use toml::Value;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const RETIRED: &[&str] = &["rust/gitr-store/", "rust/gitr-surreal-probe/", "src/actions_remote.rs", "ci/gitr-store-", "ci/gitr-surreal-", ".github/workflows/gitr-store-qualification.yml", ".github/workflows/gitr-surreal-qualification.yml"];
fn forbidden(name: &str) -> bool {name.to_ascii_lowercase().replace('_',"-").starts_with("surreal")}
fn manifest(v: &Value) -> Result<()> {
    if let Some(table)=v.as_table() {for (key,item) in table {
        if ["dependencies","dev-dependencies","build-dependencies"].contains(&key.as_str()) {
            let deps=item.as_table().ok_or("dependency section must be a table")?;
            for (alias,spec) in deps {if forbidden(alias) || spec.get("package").and_then(Value::as_str).is_some_and(forbidden) {return Err(format!("retired dependency: {alias}").into());}}
        } else {manifest(item)?;}
    }}
    Ok(())
}
fn lock(v: &Value) -> Result<Vec<(String,String)>> {
    let mut packages=Vec::new();
    for p in v.get("package").and_then(Value::as_array).ok_or("missing lock packages")? {
        let name=p.get("name").and_then(Value::as_str).ok_or("missing package name")?;
        if forbidden(name) {return Err(format!("retired locked dependency: {name}").into());}
        packages.push((name.into(),p.get("version").and_then(Value::as_str).ok_or("missing version")?.into()));
    } Ok(packages)
}
fn pin(m: &Value,l: &Value) -> Result<()> {
    manifest(m)?;
    let spec=m.get("dependencies").and_then(|d|d.get("redb")).ok_or("redb required")?;
    if spec.get("optional").and_then(Value::as_bool)==Some(true) {return Err("redb must be required".into());}
    let version=spec.as_str().or_else(||spec.get("version").and_then(Value::as_str)).ok_or("missing redb version")?;
    if version!="=4.5.0" {return Err("retain reviewed exact redb 4.5.0 pin".into());}
    let versions:Vec<_>=lock(l)?.into_iter().filter(|(n,_)|n=="redb").map(|(_,v)|v).collect();
    if versions!=["4.5.0"] {return Err("redb lock pin mismatch".into());} Ok(())
}
fn parse(p: &Path)->Result<Value> {Ok(fs::read_to_string(p)?.parse()?)}
fn active_path(name:&str)->Result<()> {if RETIRED.iter().any(|p|name.starts_with(p)) {return Err(format!("retired active path: {name}").into());} Ok(())}
fn policy(root:&Path)->Result<()> {
    let output=Command::new("git").args(["ls-files","-z","--","Cargo.toml","Cargo.lock","src","crates","vendor","ci","xtask",".github/workflows"]).current_dir(root).output()?;
    if !output.status.success() {return Err("git inventory failed".into());}
    for name in std::str::from_utf8(&output.stdout)?.split('\0').filter(|s|!s.is_empty()) {
        active_path(name)?;
        let path=root.join(name);
        match path.file_name().and_then(|s|s.to_str()) {Some("Cargo.toml")=>manifest(&parse(&path)?)?,Some("Cargo.lock")=>{lock(&parse(&path)?)?;},_=>{}}
    }
    pin(&parse(&root.join("Cargo.toml"))?,&parse(&root.join("Cargo.lock"))?)?;
    println!("storage policy: redb 4.5.0; no active Surreal dependencies or migration paths");Ok(())
}
fn audit()->Result<()> {
    let output=Command::new("git").args(["ls-files","-s","-z"]).output()?;
    if !output.status.success() {return Err("git inventory failed".into());}
    println!("mode\tblob\tclass\tpath");
    for record in std::str::from_utf8(&output.stdout)?.split('\0').filter(|r|!r.is_empty()) {
        let (meta,name)=record.split_once('\t').ok_or("invalid index record")?;
        let mode=meta.split_whitespace().next().ok_or("missing mode")?;
        let blob=meta.split_whitespace().nth(1).ok_or("missing blob")?;
        if mode!="100644" {return Err(format!("review executable/symlink/submodule: {name}").into());}
        let path=Path::new(name);
        let ext=path.extension().and_then(|e|e.to_str()).unwrap_or("");
        let class=match ext {
            "rs"=>"rust",
            "md"|"log"|"txt"|"csv"=>"prose/evidence",
            "toml"|"lock"|"json"|"yaml"|"yml"|"upstream"=>"declarative/data",
            ""|"1"=>{if !["COPYING","LGPL-2.1","LICENSE","LICENSE-MIT","LICENSE-APACHE",".gitignore",".gitattributes",".editorconfig",".mailmap"].contains(&path.file_name().and_then(|n|n.to_str()).unwrap_or("")) {return Err(format!("unclassified extensionless path: {name}").into());} "notice/metadata"},
            _=>return Err(format!("unclassified source/data: {name}").into()),
        };
        let bytes=fs::read(path)?;
        if bytes.starts_with(b"#!") && ext!="rs" {return Err(format!("foreign shebang: {name}").into());}
        println!("{mode}\t{blob}\t{class}\t{name}");
    } Ok(())
}
fn run(c:&mut Command)->Result<()> {if !c.status()?.success() {return Err("command failed".into());} Ok(())}
fn qualify()->Result<()> {
    let root=env::var("BLUEFE_ROOT")?;let root=Path::new(&root);
    if !root.is_absolute() || !root.is_dir() {return Err("BLUEFE_ROOT must name the reviewed absolute bundle".into());}
    let bin=|name:&str|root.join("bin").join(format!("{name}{}",env::consts::EXE_SUFFIX));
    let version=Command::new(bin("rustc")).arg("-vV").output()?;
    if !version.status.success() || !String::from_utf8_lossy(&version.stdout).contains("commit-hash: 7005dc02c9fb6674380eade443022bbc04215276") {return Err("wrong produced compiler".into());}
    print!("{}",String::from_utf8_lossy(&version.stdout));
    for args in [vec!["build","--locked"],vec!["test","--locked","--all-features","--all-targets"],vec!["test","--locked","--no-default-features","--all-targets"],vec!["test","--locked","--doc","--all-features"],vec!["build","--locked","--all-features"],vec!["build","--manifest-path","crates/action-example/Cargo.toml","--locked","--release","--target","wasm32-unknown-unknown"],vec!["test","--manifest-path","xtask/Cargo.toml","--locked"],vec!["test","--manifest-path","vendor/rbit-workflow/Cargo.toml","--locked"]] {
        run(Command::new(bin("lustr")).arg("--bluefe-root").arg(root).args(args).env("RUSTC",bin("rustc")).env("RUSTDOC",bin("rustdoc")).env("CARGO_ENCODED_RUSTFLAGS",format!("--sysroot\x1f{}",root.display())).env("RUSTDOCFLAGS",format!("--sysroot {}",root.display())).env("CARGO_BUILD_JOBS","2"))?;
    }
    policy(Path::new("."))?;
    audit()
}
fn main()->ExitCode {let result=match env::args().nth(1).as_deref() {Some("policy")=>policy(Path::new(".")),Some("qualify")=>qualify(),Some("audit")=>audit(),_=>Err("usage: gitr-xtask policy|qualify|audit".into())};match result {Ok(())=>ExitCode::SUCCESS,Err(e)=>{eprintln!("{e}");ExitCode::FAILURE}}}
#[cfg(test)] mod tests {
    use super::*;
    fn value(s:&str)->Value{s.parse().unwrap()}
    #[test] fn retired_paths(){for p in RETIRED {assert!(active_path(p).is_err());}active_path("src/actions.rs").unwrap();}
    #[test] fn accepted_redb(){pin(&value("[dependencies]\nredb='=4.5.0'"),&value("[[package]]\nname='redb'\nversion='4.5.0'")).unwrap();}
    #[test] fn forbidden_alias_optional_target_workspace(){for section in ["dependencies","dev-dependencies","build-dependencies","workspace.dependencies","target.'cfg(unix)'.dependencies"] {for spec in ["surrealdb='4'","db={package='surrealkv',optional=true,version='1'}"] {assert!(manifest(&value(&format!("[{section}]\n{spec}"))).is_err());}}}
    #[test] fn forbidden_transitive_lock(){assert!(lock(&value("[[package]]\nname='surrealdb-core'\nversion='4'")).is_err());}
    #[test] fn exact_required_matching_pin(){let l=value("[[package]]\nname='redb'\nversion='4.5.0'");for spec in ["'4.5'","'=4.6.0'","{version='=4.5.0',optional=true}"] {assert!(pin(&value(&format!("[dependencies]\nredb={spec}")),&l).is_err());}assert!(pin(&value("[dependencies]\nredb='=4.5.0'"),&value("[[package]]\nname='redb'\nversion='4.4.0'")).is_err());}
}
