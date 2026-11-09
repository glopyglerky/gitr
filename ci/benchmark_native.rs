//! Reproducible selected-command benchmark. Compile this std-only helper with
//! the verified produced BlueFe compiler, not the stock orchestration compiler.
#![forbid(unsafe_code)]

use std::env;
use std::error::Error;
use std::ffi::OsString;
use std::fs::{self, File, FileTimes, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Output, Stdio};
use std::time::{Duration, Instant, UNIX_EPOCH};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const FILES: usize = 1_000;
const COMMITS: usize = 200;
const BLOB_BYTES: usize = 32 * 1024 * 1024;
const SAMPLES: usize = 7;
const SEED: u64 = 0x4255_4c4b_4749_5452;

struct Context {
    gitr: PathBuf,
    git: PathBuf,
    compiler: PathBuf,
    fixture: PathBuf,
    output: PathBuf,
}

struct Workload {
    name: &'static str,
    arguments: Vec<OsString>,
}

struct Sample {
    workload: &'static str,
    engine: &'static str,
    phase: &'static str,
    iteration: usize,
    position: usize,
    elapsed_ns: u128,
    stdout_bytes: usize,
    stderr_bytes: usize,
    exit_code: Option<i32>,
}

struct Summary {
    workload: &'static str,
    git_median_ns: u128,
    gitr_median_ns: u128,
}

fn json(value: &str) -> String {
    let mut result = String::from("\"");
    for character in value.chars() {
        match character {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            character if character.is_control() => result.push_str(&format!("\\u{:04x}", character as u32)),
            character => result.push(character),
        }
    }
    result.push('"');
    result
}

fn text_path(path: &Path) -> Result<&str> {
    path.to_str().ok_or_else(|| "benchmark paths must be valid UTF-8".into())
}

fn parse() -> Result<Context> {
    let arguments: Vec<_> = env::args_os().skip(1).collect();
    if arguments.len() != 10 {
        return Err("usage: benchmark-native --gitr /exe --git /exe --compiler-root /bundle --fixture /fresh-dir --output /fresh-dir".into());
    }
    let mut values = std::collections::BTreeMap::new();
    for pair in arguments.chunks_exact(2) {
        let name = pair[0].to_str().ok_or("invalid option name")?;
        if !matches!(name, "--gitr" | "--git" | "--compiler-root" | "--fixture" | "--output")
            || values.insert(name, PathBuf::from(&pair[1])).is_some()
        {
            return Err("unknown or duplicate benchmark option".into());
        }
    }
    let mut take = |name: &str| values.remove(name).ok_or_else(|| format!("missing {name}"));
    let context = Context {
        gitr: take("--gitr")?.canonicalize()?,
        git: take("--git")?.canonicalize()?,
        compiler: take("--compiler-root")?.canonicalize()?,
        fixture: take("--fixture")?,
        output: take("--output")?,
    };
    for path in [&context.gitr, &context.git, &context.compiler, &context.fixture, &context.output] {
        if !path.is_absolute() || path.components().any(|part| part == std::path::Component::ParentDir) {
            return Err("benchmark paths must be absolute without parent traversal".into());
        }
        text_path(path)?;
    }
    if !context.gitr.is_file() || !context.git.is_file() || context.gitr == context.git {
        return Err("benchmark requires distinct existing Gitr and reference Git executables".into());
    }
    if context.fixture.exists() || context.output.exists() {
        return Err("benchmark fixture and output must be fresh directories".into());
    }
    if context.fixture.starts_with(&context.output) || context.output.starts_with(&context.fixture) {
        return Err("keep benchmark output outside the fixture".into());
    }
    Ok(context)
}

/// Both measured programs receive the same small environment. An empty PATH
/// and absolute executable path prevent system-Git or shell-helper fallback.
fn command(executable: &Path, directory: &Path) -> Command {
    let mut command = Command::new(executable);
    command.current_dir(directory).env_clear()
        .env("PATH", "")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("TZ", "UTC")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_PAGER", "")
        .env("PAGER", "")
        .stdin(Stdio::null());
    command
}

fn success(output: Output, operation: &str) -> Result<Vec<u8>> {
    if !output.status.success() {
        return Err(format!("{operation} failed with {}: {}", output.status,
            String::from_utf8_lossy(&output.stderr)).into());
    }
    Ok(output.stdout)
}

fn git(context: &Context, repository: &Path, arguments: &[&str]) -> Result<Vec<u8>> {
    success(command(&context.git, repository).arg("--no-pager").args(arguments).output()?,
        &format!("fixture git {arguments:?}"))
}

fn write_fixture_file(path: &Path, bytes: &[u8], seconds: u64) -> Result<()> {
    let mut file = File::create(path)?;
    file.write_all(bytes)?;
    let date = UNIX_EPOCH + Duration::from_secs(seconds);
    file.set_times(FileTimes::new().set_modified(date).set_accessed(date))?;
    Ok(())
}

fn fixture(context: &Context) -> Result<(PathBuf, PathBuf)> {
    fs::create_dir_all(&context.fixture)?;
    let repository = context.fixture.join("repository");
    let empty_template = context.fixture.join("empty-template");
    let empty_hooks = context.fixture.join("empty-hooks");
    for directory in [&repository, &empty_template, &empty_hooks] {
        fs::create_dir(directory)?;
    }
    git(context, &repository, &["init", "--quiet", "--initial-branch=main", "--object-format=sha1",
        &format!("--template={}", empty_template.display())])?;
    for (key, value) in [
        ("core.autocrlf", "false"), ("core.filemode", "false"),
        ("core.fsmonitor", "false"), ("core.untrackedCache", "false"),
        ("gc.auto", "0"), ("maintenance.auto", "false"),
        ("commit.gpgSign", "false"), ("tag.gpgSign", "false"),
        ("color.ui", "false"), ("gitr.redbObjects", "false"),
    ] {
        git(context, &repository, &["config", key, value])?;
    }
    git(context, &repository, &["config", "core.hooksPath", text_path(&empty_hooks)?])?;
    let tracked = repository.join("files");
    fs::create_dir(&tracked)?;
    for index in 0..FILES {
        let content = format!("native benchmark tracked file {index:04}\n").repeat(8);
        write_fixture_file(&tracked.join(format!("file-{index:04}.txt")), content.as_bytes(), 1_700_000_000)?;
    }
    git(context, &repository, &["add", "--all"])?;
    let tree = String::from_utf8(git(context, &repository, &["write-tree"])?)?.trim().to_owned();
    let mut parent = String::new();
    for index in 0..COMMITS {
        let mut create = command(&context.git, &repository);
        create.args(["commit-tree", &tree, "-m", &format!("benchmark commit {index:04}")])
            .env("GIT_AUTHOR_NAME", "Gitr Native Benchmark")
            .env("GIT_AUTHOR_EMAIL", "benchmark@example.invalid")
            .env("GIT_COMMITTER_NAME", "Gitr Native Benchmark")
            .env("GIT_COMMITTER_EMAIL", "benchmark@example.invalid")
            .env("GIT_AUTHOR_DATE", format!("{} +0000", 1_700_000_000 + index))
            .env("GIT_COMMITTER_DATE", format!("{} +0000", 1_700_000_000 + index));
        if !parent.is_empty() {
            create.args(["-p", &parent]);
        }
        parent = String::from_utf8(success(create.output()?, "fixture commit-tree")?)?.trim().to_owned();
    }
    git(context, &repository, &["update-ref", "refs/heads/main", &parent])?;
    for index in 0..20 {
        let content = format!("modified tracked benchmark file {index:04}\n");
        write_fixture_file(&tracked.join(format!("file-{index:04}.txt")), content.as_bytes(), 1_700_000_001)?;
    }
    for index in 0..10 {
        write_fixture_file(&repository.join(format!("untracked-{index:04}.txt")), b"untracked\n", 1_700_000_001)?;
    }
    let blob = context.fixture.join("blob-32mib.bin");
    let mut file = File::create(&blob)?;
    let mut state = SEED;
    let mut buffer = [0_u8; 64 * 1024];
    for _ in 0..BLOB_BYTES / buffer.len() {
        for chunk in buffer.chunks_exact_mut(8) {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            chunk.copy_from_slice(&state.to_le_bytes());
        }
        file.write_all(&buffer)?;
    }
    file.sync_all()?;
    fs::write(context.output.join("fixture.json"), format!(
        "{{\"format_version\":1,\"tracked_files\":{FILES},\"modified_tracked_files\":20,\"untracked_files\":10,\"commits\":{COMMITS},\"history_shape\":\"linear commits sharing one tree\",\"head\":{},\"tree\":{},\"blob_bytes\":{BLOB_BYTES},\"blob_generator\":\"xorshift64 shifts 13,7,17; little-endian u64 output\",\"blob_seed\":\"{SEED:016x}\",\"redb_objects\":false,\"storage\":\"Git loose objects, no redb database\",\"tracked_mtime_epoch\":1700000000}}\n",
        json(&parent), json(&tree),
    ))?;
    fs::copy(repository.join(".git/config"), context.output.join("fixture-config.txt"))?;
    Ok((repository, blob))
}

fn capture_metadata(context: &Context) -> Result<()> {
    for (key, expected) in [("CARGO_PROFILE_RELEASE_OPT_LEVEL", "3"),
        ("CARGO_PROFILE_RELEASE_LTO", "false"), ("CARGO_PROFILE_RELEASE_CODEGEN_UNITS", "16"),
        ("CARGO_PROFILE_RELEASE_DEBUG", "0")]
    {
        if env::var(key).as_deref() != Ok(expected) {
            return Err(format!("benchmark profile provenance requires {key}={expected}").into());
        }
    }
    fs::create_dir_all(context.output.join("versions"))?;
    fs::create_dir_all(context.output.join("hardware"))?;
    fs::create_dir_all(context.output.join("outputs"))?;
    for (name, executable, argument) in [
        ("gitr", context.gitr.clone(), "--version"),
        ("reference-git", context.git.clone(), "--version"),
        ("produced-rustc", context.compiler.join("bin/rustc"), "-vV"),
        ("lustr", context.compiler.join("bin/lustr"), "--version"),
    ] {
        let output = success(command(&executable, &context.output).arg(argument).output()?, "version probe")?;
        fs::write(context.output.join("versions").join(format!("{name}.txt")), output)?;
    }
    for (source, name) in [("/proc/cpuinfo", "cpuinfo.txt"), ("/proc/meminfo", "meminfo.txt"),
        ("/etc/os-release", "os-release.txt")]
    {
        match fs::read(source) {
            Ok(bytes) => fs::write(context.output.join("hardware").join(name), bytes)?,
            Err(error) => fs::write(context.output.join("hardware").join(name), format!("unavailable: {error}\n"))?,
        }
    }
    let metadata = format!(
        "{{\"format_version\":1,\"application_commit\":{},\"github_run\":{},\"gitr\":{},\"reference_git\":{},\"compiler_bundle\":{},\"os\":{},\"architecture\":{},\"available_parallelism\":{},\"profile\":{{\"name\":\"release\",\"opt_level\":3,\"lto\":false,\"codegen_units\":16,\"debug\":0,\"all_features\":true,\"note\":\"CI measurement overrides; repository default release LTO remains thin\"}},\"environment\":{{\"cleared\":true,\"PATH\":\"\",\"LC_ALL\":\"C\",\"LANG\":\"C\",\"TZ\":\"UTC\",\"GIT_CONFIG_NOSYSTEM\":\"1\",\"GIT_CONFIG_GLOBAL\":\"/dev/null\",\"GIT_TERMINAL_PROMPT\":\"0\",\"GIT_OPTIONAL_LOCKS\":\"0\",\"GIT_PAGER\":\"\",\"PAGER\":\"\"}},\"timing\":\"Instant wall time including process startup and captured stdout/stderr; comparison and artifact writes excluded\",\"cache_policy\":\"No cache drops. Verification runs precede one warmup per executable, then seven alternating measurement rounds. Shared fixture and OS page cache are warm.\",\"redb_measured\":false,\"peak_rss_measured\":false,\"limitations\":\"Selected SHA-1 read workloads on a shared hosted Linux runner; no universal speed or memory claim; reference Git uses its packaged build configuration.\"}}\n",
        json(&env::var("GITHUB_SHA").unwrap_or_else(|_| "unknown".into())),
        json(&env::var("GITHUB_RUN_ID").unwrap_or_else(|_| "local".into())),
        json(text_path(&context.gitr)?), json(text_path(&context.git)?),
        json(text_path(&context.compiler)?), json(env::consts::OS), json(env::consts::ARCH),
        std::thread::available_parallelism().map_or(0, usize::from),
    );
    fs::write(context.output.join("metadata.json"), metadata)?;
    Ok(())
}

fn sample_json(sample: &Sample) -> String {
    format!("{{\"workload\":{},\"engine\":{},\"phase\":{},\"iteration\":{},\"order_position\":{},\"elapsed_ns\":{},\"elapsed_ms\":{:.6},\"stdout_bytes\":{},\"stderr_bytes\":{},\"exit_code\":{}}}",
        json(sample.workload), json(sample.engine), json(sample.phase), sample.iteration,
        sample.position, sample.elapsed_ns, sample.elapsed_ns as f64 / 1_000_000.0,
        sample.stdout_bytes, sample.stderr_bytes,
        sample.exit_code.map_or_else(|| "null".into(), |code| code.to_string()))
}

fn record_sample(context: &Context, samples: &[Sample], sample: &Sample) -> Result<()> {
    let mut csv = OpenOptions::new().append(true).open(context.output.join("samples.csv"))?;
    writeln!(csv, "{},{},{},{},{},{},{:.6},{},{},{}", sample.workload, sample.engine,
        sample.phase, sample.iteration, sample.position, sample.elapsed_ns,
        sample.elapsed_ns as f64 / 1_000_000.0, sample.stdout_bytes, sample.stderr_bytes,
        sample.exit_code.map_or_else(|| "signal".into(), |code| code.to_string()))?;
    let records = samples.iter().map(sample_json).collect::<Vec<_>>().join(",\n");
    fs::write(context.output.join("samples.json"), format!("[\n{records}\n]\n"))?;
    Ok(())
}

fn matching_output(context: &Context, label: &str, actual: &Output, expected: &Output) -> Result<()> {
    if !actual.status.success() || actual.stdout != expected.stdout || actual.stderr != expected.stderr {
        fs::write(context.output.join("outputs").join(format!("{label}.mismatch.stdout")), &actual.stdout)?;
        fs::write(context.output.join("outputs").join(format!("{label}.mismatch.stderr")), &actual.stderr)?;
        return Err(format!("{label}: exit status or output differs from reference Git; timings cannot qualify this workload").into());
    }
    Ok(())
}

fn median(samples: &[Sample], workload: &str, engine: &str) -> Result<u128> {
    let mut values: Vec<_> = samples.iter().filter(|sample| {
        sample.workload == workload && sample.engine == engine && sample.phase == "measured"
    }).map(|sample| sample.elapsed_ns).collect();
    if values.len() != SAMPLES || values.contains(&0) {
        return Err("incomplete benchmark samples or zero timer resolution".into());
    }
    values.sort_unstable();
    Ok(values[SAMPLES / 2])
}

fn execute(context: &Context) -> Result<()> {
    capture_metadata(context)?;
    let (repository, blob) = fixture(context)?;
    let workloads = [
        Workload { name: "hash-blob-32mib", arguments: vec!["hash-object".into(), "--no-filters".into(), blob.into_os_string()] },
        Workload { name: "status-1000-files", arguments: ["status", "--porcelain=v1", "-z"].map(OsString::from).to_vec() },
        Workload { name: "log-200-commits", arguments: ["log", "--format=%H", "-n", "200", "HEAD"].map(OsString::from).to_vec() },
        Workload { name: "ls-tree-1000-files", arguments: ["ls-tree", "-r", "-z", "--full-tree", "HEAD"].map(OsString::from).to_vec() },
    ];
    fs::write(context.output.join("samples.csv"), "workload,engine,phase,iteration,order_position,elapsed_ns,elapsed_ms,stdout_bytes,stderr_bytes,exit_code\n")?;
    let mut samples = Vec::new();
    let mut summaries = Vec::new();
    let mut commands = Vec::new();
    for (workload_index, workload) in workloads.iter().enumerate() {
        let invoke = |executable: &Path| {
            command(executable, &repository).arg("--no-pager").args(&workload.arguments).output()
        };
        let expected = invoke(&context.git)?;
        if !expected.status.success() || expected.stdout.is_empty() {
            return Err(format!("reference fixture verification failed for {}: {}", workload.name,
                String::from_utf8_lossy(&expected.stderr)).into());
        }
        let fixture_valid = match workload.name {
            "hash-blob-32mib" => expected.stdout.len() == 41
                && expected.stdout[..40].iter().all(u8::is_ascii_hexdigit)
                && expected.stdout[40] == b'\n',
            "status-1000-files" => expected.stdout.iter().filter(|byte| **byte == 0).count() == 30,
            "log-200-commits" => expected.stdout.iter().filter(|byte| **byte == b'\n').count() == COMMITS,
            "ls-tree-1000-files" => expected.stdout.iter().filter(|byte| **byte == 0).count() == FILES,
            _ => false,
        };
        if !fixture_valid {
            return Err(format!("reference output has the wrong fixture shape for {}", workload.name).into());
        }
        fs::write(context.output.join("outputs").join(format!("{}.expected.stdout", workload.name)), &expected.stdout)?;
        fs::write(context.output.join("outputs").join(format!("{}.expected.stderr", workload.name)), &expected.stderr)?;
        let actual = invoke(&context.gitr)?;
        matching_output(context, workload.name, &actual, &expected)?;
        fs::write(context.output.join("outputs").join(format!("{}.gitr-verified.stdout", workload.name)), &actual.stdout)?;
        commands.push(format!("{{\"workload\":{},\"arguments\":[{},{}]}}", json(workload.name),
            json("--no-pager"), workload.arguments.iter().map(|value| {
                value.to_str().map(json).ok_or("non-UTF8 benchmark argument")
            }).collect::<std::result::Result<Vec<_>, _>>()?.join(",")));
        fs::write(context.output.join("commands.json"), format!("[{}]\n", commands.join(",\n")))?;
        for iteration in 0..=SAMPLES {
            let order = if (iteration + workload_index) % 2 == 0 { ["git", "gitr"] } else { ["gitr", "git"] };
            for (position, engine) in order.into_iter().enumerate() {
                let executable = if engine == "git" { &context.git } else { &context.gitr };
                let started = Instant::now();
                let actual = invoke(executable)?;
                let elapsed_ns = started.elapsed().as_nanos();
                samples.push(Sample {
                    workload: workload.name, engine,
                    phase: if iteration == 0 { "warmup" } else { "measured" },
                    iteration, position: position + 1, elapsed_ns,
                    stdout_bytes: actual.stdout.len(), stderr_bytes: actual.stderr.len(),
                    exit_code: actual.status.code(),
                });
                record_sample(context, &samples, samples.last().ok_or("sample missing")?)?;
                matching_output(context, &format!("{}-{engine}-{iteration}", workload.name), &actual, &expected)?;
            }
        }
        summaries.push(Summary {
            workload: workload.name,
            git_median_ns: median(&samples, workload.name, "git")?,
            gitr_median_ns: median(&samples, workload.name, "gitr")?,
        });
    }
    let mut markdown = String::from("## Native command benchmark\n\nRelease build with opt-level 3, LTO off and 16 codegen units. The repository's default release profile still uses thin LTO. Exact exit status, stdout and stderr matched reference Git before timing and for every sample.\n\n| Workload | Git median ms | Gitr median ms | Gitr / Git |\n|---|---:|---:|---:|\n");
    let mut records = Vec::new();
    for summary in &summaries {
        let git_ms = summary.git_median_ns as f64 / 1_000_000.0;
        let gitr_ms = summary.gitr_median_ns as f64 / 1_000_000.0;
        let ratio = gitr_ms / git_ms;
        markdown.push_str(&format!("| {} | {git_ms:.3} | {gitr_ms:.3} | {ratio:.3} |\n", summary.workload));
        records.push(format!("{{\"workload\":{},\"git_median_ms\":{git_ms:.6},\"gitr_median_ms\":{gitr_ms:.6},\"gitr_over_git\":{ratio:.6}}}", json(summary.workload)));
    }
    markdown.push_str("\nA ratio above 1 means Gitr was slower. Each executable had one warmup and seven measured invocations per workload; order alternated. Verification and warmups populated the shared OS page cache. Timings include process startup and output capture. The fixture has 1,000 tracked files, 20 modified files, 10 untracked files, and 200 linear commits sharing one tree. These are SHA-1 reads from ordinary Git loose objects. Redb performance and peak memory were not measured. Hosted runner contention and the packaged Git build can affect these results.\n");
    fs::write(context.output.join("summary.md"), &markdown)?;
    fs::write(context.output.join("results.json"), format!(
        "{{\"format_version\":1,\"complete\":true,\"warmups_per_engine\":1,\"samples_per_engine\":{SAMPLES},\"ratio_definition\":\"gitr_median / git_median; greater than one means slower\",\"summaries\":[{}]}}\n", records.join(",\n")))?;
    print!("{markdown}");
    if let Some(path) = env::var_os("GITHUB_STEP_SUMMARY") {
        OpenOptions::new().create(true).append(true).open(path)?.write_all(markdown.as_bytes())?;
    }
    Ok(())
}

fn run() -> Result<()> {
    let context = parse()?;
    fs::create_dir_all(&context.output)?;
    let result = execute(&context);
    if let Err(error) = &result {
        fs::write(context.output.join("results.json"), format!(
            "{{\"format_version\":1,\"complete\":false,\"error\":{}}}\n", json(&error.to_string())))?;
    }
    result
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
