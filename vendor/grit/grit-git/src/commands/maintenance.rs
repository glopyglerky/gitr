//! `git maintenance` — Git-compatible maintenance (`git/builtin/gc.c`).

use crate::explicit_exit::ExplicitExit;
use crate::grit_exe;
use crate::trace2_emit_child_start_json;
use crate::trace2_region_json;
use anyhow::{bail, Context, Result};
use grit_lib::config::{ConfigFile, ConfigScope, ConfigSet};
use grit_lib::objects::{parse_commit, ObjectId, ObjectKind};
use grit_lib::odb::Odb;
use grit_lib::repo::Repository;
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SchedulePriority {
    None = 0,
    Weekly = 1,
    Daily = 2,
    Hourly = 3,
}

impl SchedulePriority {
    fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "hourly" => Some(Self::Hourly),
            "daily" => Some(Self::Daily),
            "weekly" => Some(Self::Weekly),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Hourly => "hourly",
            Self::Daily => "daily",
            Self::Weekly => "weekly",
            Self::None => "none",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TaskId {
    Prefetch,
    LooseObjects,
    IncrementalRepack,
    GeometricRepack,
    Gc,
    CommitGraph,
    PackRefs,
    ReflogExpire,
    WorktreePrune,
    RerereGc,
}

impl TaskId {
    fn name(self) -> &'static str {
        match self {
            Self::Prefetch => "prefetch",
            Self::LooseObjects => "loose-objects",
            Self::IncrementalRepack => "incremental-repack",
            Self::GeometricRepack => "geometric-repack",
            Self::Gc => "gc",
            Self::CommitGraph => "commit-graph",
            Self::PackRefs => "pack-refs",
            Self::ReflogExpire => "reflog-expire",
            Self::WorktreePrune => "worktree-prune",
            Self::RerereGc => "rerere-gc",
        }
    }

    fn from_name(s: &str) -> Option<Self> {
        Some(match s {
            "prefetch" => Self::Prefetch,
            "loose-objects" => Self::LooseObjects,
            "incremental-repack" => Self::IncrementalRepack,
            "geometric-repack" => Self::GeometricRepack,
            "gc" => Self::Gc,
            "commit-graph" => Self::CommitGraph,
            "pack-refs" => Self::PackRefs,
            "reflog-expire" => Self::ReflogExpire,
            "worktree-prune" => Self::WorktreePrune,
            "rerere-gc" => Self::RerereGc,
            _ => return None,
        })
    }

    fn idx(self) -> usize {
        self as usize
    }
}

const N_TASK: usize = 10;

fn all_tasks() -> [TaskId; N_TASK] {
    [
        TaskId::Prefetch,
        TaskId::LooseObjects,
        TaskId::IncrementalRepack,
        TaskId::GeometricRepack,
        TaskId::Gc,
        TaskId::CommitGraph,
        TaskId::PackRefs,
        TaskId::ReflogExpire,
        TaskId::WorktreePrune,
        TaskId::RerereGc,
    ]
}

#[derive(Clone, Copy)]
struct MaintBits(u8);

impl MaintBits {
    const SCH: u8 = 1;
    const MAN: u8 = 2;

    fn new(bits: u8) -> Self {
        Self(bits)
    }
    fn has(self, m: u8) -> bool {
        self.0 & m != 0
    }
}

#[derive(Clone)]
struct Strategy {
    flags: [MaintBits; N_TASK],
    schedules: [SchedulePriority; N_TASK],
}

impl Strategy {
    fn empty() -> Self {
        Self {
            flags: [MaintBits::new(0); N_TASK],
            schedules: [SchedulePriority::None; N_TASK],
        }
    }

    fn geometric() -> Self {
        let mut s = Self::empty();
        let mut set = |t: TaskId, bits: u8, sch: SchedulePriority| {
            s.flags[t.idx()] = MaintBits::new(bits);
            s.schedules[t.idx()] = sch;
        };
        let both = MaintBits::SCH | MaintBits::MAN;
        set(TaskId::CommitGraph, both, SchedulePriority::Hourly);
        set(TaskId::GeometricRepack, both, SchedulePriority::Daily);
        set(TaskId::PackRefs, both, SchedulePriority::Daily);
        set(TaskId::RerereGc, both, SchedulePriority::Weekly);
        set(TaskId::ReflogExpire, both, SchedulePriority::Weekly);
        set(TaskId::WorktreePrune, both, SchedulePriority::Weekly);
        s
    }

    fn gc_only() -> Self {
        let mut s = Self::empty();
        s.flags[TaskId::Gc.idx()] = MaintBits::new(MaintBits::SCH | MaintBits::MAN);
        s.schedules[TaskId::Gc.idx()] = SchedulePriority::Daily;
        s
    }

    fn incremental() -> Self {
        let mut s = Self::empty();
        let mut sch = |t: TaskId, sc: SchedulePriority| {
            s.flags[t.idx()] = MaintBits::new(MaintBits::SCH);
            s.schedules[t.idx()] = sc;
        };
        sch(TaskId::CommitGraph, SchedulePriority::Hourly);
        sch(TaskId::Prefetch, SchedulePriority::Hourly);
        sch(TaskId::IncrementalRepack, SchedulePriority::Daily);
        sch(TaskId::LooseObjects, SchedulePriority::Daily);
        sch(TaskId::PackRefs, SchedulePriority::Weekly);
        s.flags[TaskId::Gc.idx()] = MaintBits::new(MaintBits::MAN);
        s
    }
}

fn parse_strategy(name: &str) -> Result<Strategy> {
    match name.to_ascii_lowercase().as_str() {
        "gc" => Ok(Strategy::gc_only()),
        "incremental" => Ok(Strategy::incremental()),
        "geometric" => Ok(Strategy::geometric()),
        "none" => Ok(Strategy::empty()),
        other => bail!("unknown maintenance strategy: '{other}'"),
    }
}

pub fn run_from_argv(rest: &[String]) -> Result<()> {
    if rest.len() == 1 && (rest[0] == "-h" || rest[0] == "--help") {
        println!("usage: git maintenance <subcommand> [<options>]");
        return Err(ExplicitExit {
            code: 129,
            message: String::new(),
        }
        .into());
    }
    if rest.is_empty() {
        eprintln!("error: need a subcommand");
        eprintln!("usage: git maintenance <subcommand> [<options>]");
        return Err(ExplicitExit {
            code: 129,
            message: String::new(),
        }
        .into());
    }

    match rest[0].as_str() {
        "run" => cmd_run(&rest[1..]),
        "is-needed" => cmd_is_needed(&rest[1..]),
        "register" => cmd_register(&rest[1..]),
        "unregister" => cmd_unregister(&rest[1..]),
        "start" => cmd_start(&rest[1..]),
        "stop" => cmd_stop(),
        other => {
            eprintln!("error: unknown subcommand: `{other}'");
            eprintln!("usage: git maintenance <subcommand> [<options>]");
            Err(ExplicitExit {
                code: 129,
                message: String::new(),
            }
            .into())
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SchedulerKind {
    Crontab,
    Systemd,
    Launchctl,
    Schtasks,
}

fn parse_scheduler_arg(s: &str) -> Result<SchedulerKind> {
    match s.to_ascii_lowercase().as_str() {
        "cron" | "crontab" => Ok(SchedulerKind::Crontab),
        "systemd" | "systemd-timer" => Ok(SchedulerKind::Systemd),
        "launchctl" => Ok(SchedulerKind::Launchctl),
        "schtasks" => Ok(SchedulerKind::Schtasks),
        _ => {
            eprintln!("error: unrecognized --scheduler argument '{s}'");
            Err(ExplicitExit {
                code: 129,
                message: String::new(),
            }
            .into())
        }
    }
}

fn systemctl_user_works() -> bool {
    let (_, cmd, ok) = scheduler_testing_lookup("systemctl");
    if ok && cmd != "systemctl" {
        return true;
    }
    Command::new("systemctl")
        .args(["--user", "list-timers"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Mirror of git's `get_schedule_cmd`. Returns `(from_env, command, is_available)`.
///
/// When `GIT_TEST_MAINT_SCHEDULER` is set but the requested `cmd` is not one of
/// the listed keys, the scheduler is reported as UNAVAILABLE (git sets
/// `*is_available = 0` and returns the original command unchanged). The `val !=
/// "false"` convention lets tests model a present-but-failing scheduler.
fn scheduler_testing_lookup(cmd: &str) -> (bool, String, bool) {
    let Ok(testing) = std::env::var("GIT_TEST_MAINT_SCHEDULER") else {
        return (false, cmd.to_string(), true);
    };
    for entry in testing.split(',') {
        let mut p = entry.splitn(2, ':');
        let Some(key) = p.next() else { continue };
        let Some(val) = p.next() else { continue };
        if key == cmd {
            return (true, val.to_string(), val != "false");
        }
    }
    // Set, but this scheduler is not listed: not available.
    (true, cmd.to_string(), false)
}

fn scheduler_available(sk: SchedulerKind) -> bool {
    let name = match sk {
        SchedulerKind::Crontab => "crontab",
        SchedulerKind::Systemd => "systemctl",
        SchedulerKind::Launchctl => "launchctl",
        SchedulerKind::Schtasks => "schtasks",
    };
    let (from_env, _, ok) = scheduler_testing_lookup(name);
    if from_env {
        return ok;
    }
    match sk {
        SchedulerKind::Crontab => {
            #[cfg(target_os = "macos")]
            {
                false
            }
            #[cfg(not(target_os = "macos"))]
            {
                Command::new("crontab")
                    .arg("-l")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .is_ok()
            }
        }
        SchedulerKind::Systemd => systemctl_user_works(),
        _ => true,
    }
}

fn resolve_auto_scheduler() -> SchedulerKind {
    #[cfg(target_os = "linux")]
    if systemctl_user_works() {
        return SchedulerKind::Systemd;
    }
    SchedulerKind::Crontab
}

fn cmd_stop() -> Result<()> {
    let repo = Repository::discover(None).context("not a git repository")?;
    let lock_dir = repo.git_dir.join("objects");
    let lock_path = lock_dir.join("schedule.lock");
    fs::create_dir_all(&lock_dir)?;
    let _lock = ScheduleLock::acquire(lock_path)?;
    update_background_schedule_locked(&repo, false, None)
}

fn cmd_start(rest: &[String]) -> Result<()> {
    let repo = Repository::discover(None).context("not a git repository")?;
    let mut sched_arg: Option<String> = None;
    let mut i = 0usize;
    while i < rest.len() {
        let a = &rest[i];
        if let Some(v) = a.strip_prefix("--scheduler=") {
            sched_arg = Some(v.to_string());
            i += 1;
        } else if a == "--scheduler" {
            let Some(v) = rest.get(i + 1) else {
                bail!("option `--scheduler` requires a value");
            };
            sched_arg = Some(v.clone());
            i += 2;
        } else if a == "--no-scheduler" {
            eprintln!("error: unknown option `--no-scheduler`");
            return Err(ExplicitExit {
                code: 129,
                message: String::new(),
            }
            .into());
        } else {
            bail!("unknown option: {a}");
        }
    }

    let raw = sched_arg.as_deref().unwrap_or("auto");
    let sk = if raw.eq_ignore_ascii_case("auto") {
        resolve_auto_scheduler()
    } else {
        parse_scheduler_arg(raw)?
    };

    // Acquire the schedule lock before doing anything else. Git checks scheduler
    // availability first, but doing the lock first lets us surface the
    // "another process is running" error even when an unrelated scheduler env
    // mock claims the requested scheduler is unavailable.
    let lock_dir = repo.git_dir.join("objects");
    let lock_path = lock_dir.join("schedule.lock");
    fs::create_dir_all(&lock_dir)?;
    let _lock = ScheduleLock::acquire(lock_path)?;

    if !scheduler_available(sk) {
        bail!(
            "fatal: {} scheduler is not available",
            scheduler_cmd_name(sk)
        );
    }

    update_background_schedule_locked(&repo, true, Some(sk))?;
    cmd_register(&[])
}

fn scheduler_cmd_name(sk: SchedulerKind) -> &'static str {
    match sk {
        SchedulerKind::Crontab => "crontab",
        SchedulerKind::Systemd => "systemctl",
        SchedulerKind::Launchctl => "launchctl",
        SchedulerKind::Schtasks => "schtasks",
    }
}

/// All schedulers in git's `scheduler_fn` iteration order (crontab, systemd,
/// launchctl, schtasks). `update_background_schedule` tears these down in this
/// order before enabling the selected one.
const ALL_SCHEDULERS: [SchedulerKind; 4] = [
    SchedulerKind::Crontab,
    SchedulerKind::Systemd,
    SchedulerKind::Launchctl,
    SchedulerKind::Schtasks,
];

struct ScheduleLock {
    path: PathBuf,
}

impl ScheduleLock {
    fn acquire(path: PathBuf) -> Result<Self> {
        match fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
        {
            Ok(mut f) => {
                let _ = writeln!(f, "{}", std::process::id());
                Ok(Self { path })
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                bail!(
                    "unable to create '{}.lock': File exists.\n\n\
Another scheduled git-maintenance(1) process seems to be running in this\n\
repository. Please make sure no other maintenance processes are running and\n\
then try again. If it still fails, a git-maintenance(1) process may have\n\
crashed in this repository earlier: remove the file manually to continue.",
                    path.display()
                );
            }
            Err(e) => Err(e.into()),
        }
    }
}

impl Drop for ScheduleLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Tear down every available scheduler except the selected one (in git's
/// `scheduler_fn` order), then enable the selected scheduler. On disable, tear
/// down all of them. Assumes the schedule lock is already held by the caller.
fn update_background_schedule_locked(
    repo: &Repository,
    enable: bool,
    sk: Option<SchedulerKind>,
) -> Result<()> {
    // First, remove schedules from every OTHER available scheduler.
    for &other in &ALL_SCHEDULERS {
        if enable && sk == Some(other) {
            continue;
        }
        if !scheduler_available(other) {
            continue;
        }
        scheduler_remove(repo, other)?;
    }

    if enable {
        if let Some(sk) = sk {
            scheduler_enable(repo, sk)?;
        }
    }
    Ok(())
}

fn scheduler_remove(repo: &Repository, sk: SchedulerKind) -> Result<()> {
    match sk {
        SchedulerKind::Crontab => crontab_clear(repo),
        SchedulerKind::Systemd => systemd_remove_units(),
        SchedulerKind::Launchctl => launchctl_remove_plists(repo),
        SchedulerKind::Schtasks => schtasks_remove_tasks(repo),
    }
}

fn scheduler_enable(repo: &Repository, sk: SchedulerKind) -> Result<()> {
    match sk {
        SchedulerKind::Crontab => crontab_install(repo),
        SchedulerKind::Systemd => systemd_setup_units(repo),
        SchedulerKind::Launchctl => launchctl_add_plists(repo),
        SchedulerKind::Schtasks => schtasks_add_tasks(repo),
    }
}

const FREQUENCIES: [&str; 3] = ["hourly", "daily", "weekly"];

/// Schedule minute: a fixed value under tests (git `get_random_minute`).
fn schedule_minute() -> u32 {
    if std::env::var("GIT_TEST_MAINT_SCHEDULER").is_ok() {
        13
    } else {
        0
    }
}

fn cmd_parts(name: &str) -> Vec<String> {
    let (_, cmdline, _) = scheduler_testing_lookup(name);
    cmdline.split_whitespace().map(|s| s.to_string()).collect()
}

fn run_cmd(argv: &[String]) -> Result<()> {
    if argv.is_empty() {
        return Ok(());
    }
    let st = Command::new(&argv[0])
        .args(&argv[1..])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| format!("failed to run {}", argv[0]))?;
    if !st.success() {
        bail!("scheduler command failed");
    }
    Ok(())
}

fn run_cmd_ignore(argv: &[String]) {
    if argv.is_empty() {
        return;
    }
    let _ = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn grit_exec_path() -> String {
    grit_exe::grit_executable().to_string_lossy().to_string()
}

const EXTRA_CONFIG: &[&str] = &["credential.interactive=false", "core.askPass=true"];

// --------------------------------------------------------------------------
// crontab backend
// --------------------------------------------------------------------------

const CRON_BEGIN: &str = "# BEGIN GIT MAINTENANCE SCHEDULE";
const CRON_END: &str = "# END GIT MAINTENANCE SCHEDULE";

fn crontab_read_without_git_region(parts: &[String]) -> String {
    let mut list_cmd = Command::new(&parts[0]);
    list_cmd.args(&parts[1..]).arg("-l");
    list_cmd.stdin(Stdio::null());
    let existing = list_cmd
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let mut out = String::new();
    let mut skip = false;
    for line in existing.lines() {
        if line == CRON_BEGIN {
            skip = true;
            continue;
        }
        if line == CRON_END {
            skip = false;
            continue;
        }
        if !skip {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn crontab_install(repo: &Repository) -> Result<()> {
    let parts = cmd_parts("crontab");
    if parts.is_empty() {
        bail!("failed to set up maintenance schedule");
    }

    let mut out = crontab_read_without_git_region(&parts);

    let exec_s = grit_exec_path();
    let work = repo
        .work_tree
        .as_deref()
        .unwrap_or(&repo.git_dir)
        .canonicalize()
        .unwrap_or_else(|_| repo.git_dir.clone());
    let work_s = work.to_string_lossy();
    let minute = schedule_minute();
    let extra = "-c credential.interactive=false -c core.askPass=true ";

    out.push_str(CRON_BEGIN);
    out.push('\n');
    out.push_str("# The following schedule was created by Git\n");
    out.push_str("# Any edits made in this region might be\n");
    out.push_str("# replaced in the future by a Git command.\n\n");
    out.push_str(&format!(
        "{minute} 1-23 * * * cd \"{work_s}\" && \"{exec_s}\" {extra}for-each-repo --keep-going --config=maintenance.repo maintenance run --schedule=hourly\n"
    ));
    out.push_str(&format!(
        "{minute} 0 * * 1-6 cd \"{work_s}\" && \"{exec_s}\" {extra}for-each-repo --keep-going --config=maintenance.repo maintenance run --schedule=daily\n"
    ));
    out.push_str(&format!(
        "{minute} 0 * * 0 cd \"{work_s}\" && \"{exec_s}\" {extra}for-each-repo --keep-going --config=maintenance.repo maintenance run --schedule=weekly\n"
    ));
    out.push('\n');
    out.push_str(CRON_END);
    out.push('\n');

    let tmp = repo.git_dir.join("objects").join("cron-edit.txt");
    fs::write(&tmp, &out)?;
    let mut install = Command::new(&parts[0]);
    install.args(&parts[1..]).arg(&tmp);
    let st = install.status().context("crontab install")?;
    let _ = fs::remove_file(&tmp);
    if !st.success() {
        bail!("failed to set up maintenance schedule");
    }
    Ok(())
}

fn crontab_clear(repo: &Repository) -> Result<()> {
    let parts = cmd_parts("crontab");
    if parts.is_empty() {
        return Ok(());
    }
    let out = crontab_read_without_git_region(&parts);
    let tmp = repo.git_dir.join("objects").join("cron-clear.txt");
    fs::write(&tmp, &out)?;
    let mut install = Command::new(&parts[0]);
    install.args(&parts[1..]).arg(&tmp);
    let _ = install.status();
    let _ = fs::remove_file(&tmp);
    Ok(())
}

// --------------------------------------------------------------------------
// systemd backend
// --------------------------------------------------------------------------

fn xdg_config_home_dir() -> PathBuf {
    if let Ok(x) = std::env::var("XDG_CONFIG_HOME") {
        if !x.is_empty() {
            return PathBuf::from(x);
        }
    }
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".config")
}

fn systemd_unit_path(name: &str) -> PathBuf {
    xdg_config_home_dir()
        .join("systemd")
        .join("user")
        .join(name)
}

fn systemd_write_service_template() -> Result<()> {
    let path = systemd_unit_path("git-maintenance@.service");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let exec = grit_exec_path();
    let extra: String = EXTRA_CONFIG
        .iter()
        .map(|c| format!("-c {c} "))
        .collect::<String>();
    let unit = format!(
        "# This file was created and is maintained by Git.\n\
# Any edits made in this file might be replaced in the future\n\
# by a Git command.\n\
\n\
[Unit]\n\
Description=Optimize Git repositories data\n\
\n\
[Service]\n\
Type=oneshot\n\
ExecStart=\"{exec}\" {extra}for-each-repo --keep-going --config=maintenance.repo maintenance run --schedule=%i\n\
LockPersonality=yes\n\
MemoryDenyWriteExecute=yes\n\
NoNewPrivileges=yes\n\
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6 AF_VSOCK\n\
RestrictNamespaces=yes\n\
RestrictRealtime=yes\n\
RestrictSUIDSGID=yes\n\
SystemCallArchitectures=native\n\
SystemCallFilter=@system-service\n"
    );
    fs::write(&path, unit)?;
    Ok(())
}

fn systemd_write_timer_file(freq: &str, minute: u32) -> Result<()> {
    let path = systemd_unit_path(&format!("git-maintenance@{freq}.timer"));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let pattern = match freq {
        "hourly" => format!("*-*-* 1..23:{minute:02}:00"),
        "daily" => format!("Tue..Sun *-*-* 0:{minute:02}:00"),
        "weekly" => format!("Mon 0:{minute:02}:00"),
        _ => bail!("unknown schedule frequency {freq}"),
    };
    let unit = format!(
        "# This file was created and is maintained by Git.\n\
# Any edits made in this file might be replaced in the future\n\
# by a Git command.\n\
\n\
[Unit]\n\
Description=Optimize Git repositories data\n\
\n\
[Timer]\n\
OnCalendar={pattern}\n\
Persistent=true\n\
\n\
[Install]\n\
WantedBy=timers.target\n"
    );
    fs::write(&path, unit)?;
    Ok(())
}

fn systemd_enable_unit(enable: bool, freq: &str, minute: u32) -> Result<()> {
    if enable {
        systemd_write_timer_file(freq, minute)?;
    }
    let mut c = cmd_parts("systemctl");
    if c.is_empty() {
        bail!("failed to set up maintenance schedule");
    }
    c.extend([
        "--user".into(),
        if enable { "enable" } else { "disable" }.into(),
        "--now".into(),
        format!("git-maintenance@{freq}.timer"),
    ]);
    if enable {
        run_cmd(&c)?;
    } else {
        run_cmd_ignore(&c);
    }
    Ok(())
}

fn systemd_setup_units(_repo: &Repository) -> Result<()> {
    let minute = schedule_minute();
    systemd_write_service_template()?;
    for freq in FREQUENCIES {
        systemd_enable_unit(true, freq, minute)?;
    }
    Ok(())
}

fn systemd_remove_units() -> Result<()> {
    let minute = schedule_minute();
    for freq in FREQUENCIES {
        let _ = systemd_enable_unit(false, freq, minute);
    }
    for freq in FREQUENCIES {
        let _ = fs::remove_file(systemd_unit_path(&format!("git-maintenance@{freq}.timer")));
    }
    let _ = fs::remove_file(systemd_unit_path("git-maintenance@.service"));
    Ok(())
}

// --------------------------------------------------------------------------
// launchctl backend (macOS)
// --------------------------------------------------------------------------

fn launchctl_service_name(freq: &str) -> String {
    format!("org.git-scm.git.{freq}")
}

fn launchctl_plist_path(name: &str) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home)
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{name}.plist"))
}

fn launchctl_uid() -> String {
    format!("gui/{}", grit_lib::ident_config::current_uid())
}

fn launchctl_boot(enable: bool, filename: &Path) {
    let mut c = cmd_parts("launchctl");
    if c.is_empty() {
        return;
    }
    c.push(if enable { "bootstrap" } else { "bootout" }.into());
    c.push(launchctl_uid());
    c.push(filename.to_string_lossy().to_string());
    run_cmd_ignore(&c);
}

fn launchctl_list_contains(name: &str) -> bool {
    let mut c = cmd_parts("launchctl");
    if c.is_empty() {
        return false;
    }
    c.push("list".into());
    c.push(name.to_string());
    Command::new(&c[0])
        .args(&c[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn launchctl_plist_contents(name: &str, freq: &str, minute: u32) -> String {
    let exec = grit_exec_path();
    let exec_dir = grit_exe::grit_executable()
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let extra: String = EXTRA_CONFIG
        .iter()
        .map(|c| format!("<string>-c</string>\n<string>{c}</string>\n"))
        .collect();
    let mut plist = String::new();
    plist.push_str(&format!(
        "<?xml version=\"1.0\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\"><dict>\n\
<key>Label</key><string>{name}</string>\n\
<key>ProgramArguments</key>\n\
<array>\n\
<string>{exec_dir}/git</string>\n\
<string>--exec-path={exec}</string>\n\
{extra}\
<string>for-each-repo</string>\n\
<string>--keep-going</string>\n\
<string>--config=maintenance.repo</string>\n\
<string>maintenance</string>\n\
<string>run</string>\n\
<string>--schedule={freq}</string>\n\
</array>\n\
<key>StartCalendarInterval</key>\n\
<array>\n"
    ));
    match freq {
        "hourly" => {
            for hour in 1..=23 {
                plist.push_str(&format!(
                    "<dict>\n<key>Hour</key><integer>{hour}</integer>\n<key>Minute</key><integer>{minute}</integer>\n</dict>\n"
                ));
            }
        }
        "daily" => {
            for weekday in 1..=6 {
                plist.push_str(&format!(
                    "<dict>\n<key>Weekday</key><integer>{weekday}</integer>\n<key>Hour</key><integer>0</integer>\n<key>Minute</key><integer>{minute}</integer>\n</dict>\n"
                ));
            }
        }
        _ => {
            plist.push_str(&format!(
                "<dict>\n<key>Weekday</key><integer>0</integer>\n<key>Hour</key><integer>0</integer>\n<key>Minute</key><integer>{minute}</integer>\n</dict>\n"
            ));
        }
    }
    plist.push_str("</array>\n</dict>\n</plist>\n");
    plist
}

fn launchctl_schedule_plist(freq: &str, minute: u32) -> Result<()> {
    let name = launchctl_service_name(freq);
    let filename = launchctl_plist_path(&name);
    if let Some(parent) = filename.parent() {
        fs::create_dir_all(parent)?;
    }
    let contents = launchctl_plist_contents(&name, freq, minute);

    // If the file already exists with identical contents and launchctl reports
    // the service as registered, there is nothing to do (git short-circuit).
    let already = fs::read_to_string(&filename)
        .map(|c| c == contents)
        .unwrap_or(false)
        && launchctl_list_contains(&name);
    if already {
        return Ok(());
    }

    fs::write(&filename, &contents)?;
    launchctl_boot(false, &filename);
    launchctl_boot(true, &filename);
    Ok(())
}

fn launchctl_add_plists(_repo: &Repository) -> Result<()> {
    let minute = schedule_minute();
    for freq in FREQUENCIES {
        launchctl_schedule_plist(freq, minute)?;
    }
    Ok(())
}

fn launchctl_remove_plists(_repo: &Repository) -> Result<()> {
    for freq in FREQUENCIES {
        let name = launchctl_service_name(freq);
        let filename = launchctl_plist_path(&name);
        launchctl_boot(false, &filename);
        let _ = fs::remove_file(&filename);
    }
    Ok(())
}

// --------------------------------------------------------------------------
// schtasks backend (Windows)
// --------------------------------------------------------------------------

fn schtasks_task_name(freq: &str) -> String {
    format!("Git Maintenance ({freq})")
}

fn schtasks_xml(freq: &str, minute: u32) -> String {
    let exec = grit_exec_path();
    let exec_dir = grit_exe::grit_executable()
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let extra: String = EXTRA_CONFIG
        .iter()
        .map(|c| format!("-c {c} "))
        .collect::<String>();
    let trigger = match freq {
        "hourly" => format!(
            "<StartBoundary>2020-01-01T01:{minute:02}:00</StartBoundary>\n\
<Enabled>true</Enabled>\n\
<ScheduleByDay>\n<DaysInterval>1</DaysInterval>\n</ScheduleByDay>\n\
<Repetition>\n<Interval>PT1H</Interval>\n<Duration>PT23H</Duration>\n<StopAtDurationEnd>false</StopAtDurationEnd>\n</Repetition>\n"
        ),
        "daily" => format!(
            "<StartBoundary>2020-01-01T00:{minute:02}:00</StartBoundary>\n\
<Enabled>true</Enabled>\n\
<ScheduleByWeek>\n<DaysOfWeek>\n<Monday />\n<Tuesday />\n<Wednesday />\n<Thursday />\n<Friday />\n<Saturday />\n</DaysOfWeek>\n<WeeksInterval>1</WeeksInterval>\n</ScheduleByWeek>\n"
        ),
        _ => format!(
            "<StartBoundary>2020-01-01T00:{minute:02}:00</StartBoundary>\n\
<Enabled>true</Enabled>\n\
<ScheduleByWeek>\n<DaysOfWeek>\n<Sunday />\n</DaysOfWeek>\n<WeeksInterval>1</WeeksInterval>\n</ScheduleByWeek>\n"
        ),
    };
    format!(
        "<?xml version=\"1.0\" ?>\n\
<Task version=\"1.4\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\">\n\
<Triggers>\n<CalendarTrigger>\n{trigger}</CalendarTrigger>\n</Triggers>\n\
<Principals>\n<Principal id=\"Author\">\n<LogonType>InteractiveToken</LogonType>\n<RunLevel>LeastPrivilege</RunLevel>\n</Principal>\n</Principals>\n\
<Settings>\n<MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>\n<Enabled>true</Enabled>\n<Hidden>true</Hidden>\n<UseUnifiedSchedulingEngine>true</UseUnifiedSchedulingEngine>\n<WakeToRun>false</WakeToRun>\n<ExecutionTimeLimit>PT72H</ExecutionTimeLimit>\n<Priority>7</Priority>\n</Settings>\n\
<Actions Context=\"Author\">\n<Exec>\n<Command>\"{exec_dir}\\headless-git.exe\"</Command>\n<Arguments>--exec-path=\"{exec}\" {extra}for-each-repo --keep-going --config=maintenance.repo maintenance run --schedule={freq}</Arguments>\n</Exec>\n</Actions>\n\
</Task>\n"
    )
}

fn schtasks_schedule_task(repo: &Repository, freq: &str, minute: u32) -> Result<()> {
    let name = schtasks_task_name(freq);
    // Git writes the XML to a mkstemp file `schedule_<freq>_XXXXXX` under the
    // common dir (`.git`) with NO extension; the test's mock copies it to
    // `<file>.xml` and then `ls .git/schedule_<freq>*.xml` must match exactly
    // that single copy.
    let xml_path = repo
        .git_dir
        .join(format!("schedule_{freq}_{}", std::process::id()));
    fs::write(&xml_path, schtasks_xml(freq, minute))?;
    let mut c = cmd_parts("schtasks");
    if c.is_empty() {
        bail!("failed to set up maintenance schedule");
    }
    c.extend([
        "/create".into(),
        "/tn".into(),
        name,
        "/f".into(),
        "/xml".into(),
        xml_path.to_string_lossy().to_string(),
    ]);
    let result = run_cmd(&c);
    // Git deletes the temp XML after schtasks consumes it.
    let _ = fs::remove_file(&xml_path);
    result
}

fn schtasks_add_tasks(repo: &Repository) -> Result<()> {
    let minute = schedule_minute();
    for freq in FREQUENCIES {
        schtasks_schedule_task(repo, freq, minute)?;
    }
    Ok(())
}

fn schtasks_remove_tasks(_repo: &Repository) -> Result<()> {
    for freq in FREQUENCIES {
        let name = schtasks_task_name(freq);
        let mut c = cmd_parts("schtasks");
        if c.is_empty() {
            continue;
        }
        c.extend(["/delete".into(), "/tn".into(), name, "/f".into()]);
        run_cmd_ignore(&c);
    }
    Ok(())
}

fn cmd_register(rest: &[String]) -> Result<()> {
    let mut config_file: Option<PathBuf> = None;
    let mut i = 0usize;
    while i < rest.len() {
        if rest[i] == "--config-file" {
            let Some(p) = rest.get(i + 1) else {
                bail!("option `--config-file` requires a value");
            };
            config_file = Some(PathBuf::from(p));
            i += 2;
        } else {
            bail!("unknown option: {}", rest[i]);
        }
    }

    let repo = Repository::discover(None).context("not a git repository")?;

    // A valueless `maintenance.repo` anywhere in the merged config makes git
    // print `error: missing value for 'maintenance.repo'`; register still
    // succeeds (exit 0).
    if maintenance_repo_has_missing_value(&repo) {
        eprintln!("error: missing value for 'maintenance.repo'");
    }

    let cfg_path = repo.git_dir.join("config");
    let content = fs::read_to_string(&cfg_path).unwrap_or_default();
    let mut local = ConfigFile::parse(&cfg_path, &content, ConfigScope::Local)?;
    local.set("maintenance.auto", "false")?;
    let has_strategy = local
        .entries
        .iter()
        .any(|e| e.key == "maintenance.strategy");
    if !has_strategy {
        local.set("maintenance.strategy", "incremental")?;
    }
    local.write()?;

    let maintpath = repo
        .work_tree
        .as_deref()
        .unwrap_or(&repo.git_dir)
        .canonicalize()
        .unwrap_or_else(|_| repo.git_dir.clone());
    let maintpath_s = maintpath.to_string_lossy().to_string();

    let global_path = config_file.unwrap_or_else(|| {
        grit_lib::config::global_config_paths_pub()
            .into_iter()
            .next()
            .unwrap_or_else(|| PathBuf::from("/tmp/.gitconfig"))
    });
    if let Some(parent) = global_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let g_content = fs::read_to_string(&global_path).unwrap_or_default();
    let mut global = ConfigFile::parse(&global_path, &g_content, ConfigScope::Global)?;
    let exists = global
        .entries
        .iter()
        .any(|e| e.key == "maintenance.repo" && e.value.as_deref() == Some(maintpath_s.as_str()));
    if !exists {
        global.add_value("maintenance.repo", &maintpath_s)?;
        global.write()?;
    }
    Ok(())
}

/// True when `maintenance.repo` is present in any reachable config scope with no
/// value (e.g. `[maintenance]\n\trepo`). Mirrors git's config parser emitting
/// `error: missing value for 'maintenance.repo'`.
fn maintenance_repo_has_missing_value(repo: &Repository) -> bool {
    let cfg = match ConfigSet::load(Some(&repo.git_dir), true) {
        Ok(c) => c,
        Err(_) => return false,
    };
    cfg.get_all_raw("maintenance.repo")
        .iter()
        .any(|v| v.is_none())
}

fn cmd_unregister(rest: &[String]) -> Result<()> {
    let mut config_file: Option<PathBuf> = None;
    let mut force = false;
    let mut i = 0usize;
    while i < rest.len() {
        match rest[i].as_str() {
            "--config-file" => {
                let Some(p) = rest.get(i + 1) else {
                    bail!("option `--config-file` requires a value");
                };
                config_file = Some(PathBuf::from(p));
                i += 2;
            }
            "--force" => {
                force = true;
                i += 1;
            }
            _ => bail!("unknown option: {}", rest[i]),
        }
    }

    let repo = Repository::discover(None).context("not a git repository")?;

    // A valueless `maintenance.repo` makes git print the error and exit 128
    // (unless --force, where it prints the error but still succeeds).
    if maintenance_repo_has_missing_value(&repo) {
        eprintln!("error: missing value for 'maintenance.repo'");
        if !force {
            return Err(ExplicitExit {
                code: 128,
                message: String::new(),
            }
            .into());
        }
    }

    let maintpath = repo
        .work_tree
        .as_deref()
        .unwrap_or(&repo.git_dir)
        .canonicalize()
        .unwrap_or_else(|_| repo.git_dir.clone());
    let maintpath_s = maintpath.to_string_lossy().to_string();

    let global_path = config_file.unwrap_or_else(|| {
        grit_lib::config::global_config_paths_pub()
            .into_iter()
            .next()
            .unwrap_or_else(|| PathBuf::from("/tmp/.gitconfig"))
    });
    if !global_path.exists() {
        if force {
            return Ok(());
        }
        bail!("repository '{}' is not registered", maintpath_s);
    }
    let g_content = fs::read_to_string(&global_path).unwrap_or_default();
    let mut global = ConfigFile::parse(&global_path, &g_content, ConfigScope::Global)?;
    let had = global
        .entries
        .iter()
        .any(|e| e.key == "maintenance.repo" && e.value.as_deref() == Some(maintpath_s.as_str()));
    if !had && !force {
        bail!("repository '{}' is not registered", maintpath_s);
    }
    let n = global.unset_matching("maintenance.repo", Some(&regex::escape(&maintpath_s)), true)?;
    if n == 0 && !force {
        bail!("repository '{}' is not registered", maintpath_s);
    }
    global.write()?;
    Ok(())
}

fn cmd_is_needed(rest: &[String]) -> Result<()> {
    let repo = Repository::discover(None).context("not a git repository")?;
    let cfg = ConfigSet::load(Some(&repo.git_dir), true).unwrap_or_default();
    let mut auto = false;
    let mut tasks: Vec<TaskId> = Vec::new();
    let mut i = 0usize;
    while i < rest.len() {
        if rest[i] == "--auto" {
            auto = true;
            i += 1;
        } else if let Some(t) = rest[i].strip_prefix("--task=") {
            let Some(id) = TaskId::from_name(t) else {
                bail!("'{t}' is not a valid task");
            };
            tasks.push(id);
            i += 1;
        } else {
            bail!("unknown option: {}", rest[i]);
        }
    }
    if tasks.is_empty() {
        tasks.extend(all_tasks());
    }
    if auto {
        for t in &tasks {
            if task_auto_needed(&repo, &cfg, *t)? {
                return Ok(());
            }
        }
        std::process::exit(1);
    }
    Ok(())
}

/// Return `path` as an absolute path, joining it onto the current working
/// directory when it is relative. Unlike `canonicalize`, this does not require
/// the path to exist and does not resolve symlinks, so it is safe for paths
/// inside a repository that is about to be removed.
fn absolutize_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}

fn cmd_run(rest: &[String]) -> Result<()> {
    let repo = Repository::discover(None).context("not a git repository")?;
    let cfg = ConfigSet::load(Some(&repo.git_dir), true).unwrap_or_default();

    let mut auto = false;
    let mut detach: Option<bool> = None;
    let mut quiet = !atty::is(atty::Stream::Stderr);
    let mut schedule: Option<SchedulePriority> = None;
    let mut selected: Vec<TaskId> = Vec::new();

    let mut i = 0usize;
    while i < rest.len() {
        let a = &rest[i];
        if a == "--auto" {
            auto = true;
            i += 1;
        } else if a == "--quiet" {
            quiet = true;
            i += 1;
        } else if a == "--no-quiet" {
            quiet = false;
            i += 1;
        } else if a == "--detach" {
            detach = Some(true);
            i += 1;
        } else if a == "--no-detach" {
            detach = Some(false);
            i += 1;
        } else if let Some(v) = a.strip_prefix("--schedule=") {
            schedule =
                Some(SchedulePriority::parse(v).ok_or_else(|| {
                    anyhow::anyhow!("fatal: unrecognized --schedule argument '{v}'")
                })?);
            i += 1;
        } else if let Some(t) = a.strip_prefix("--task=") {
            let Some(id) = TaskId::from_name(t) else {
                bail!("'{t}' is not a valid task");
            };
            if selected.contains(&id) {
                bail!("task '{t}' cannot be selected multiple times");
            }
            selected.push(id);
            i += 1;
        } else {
            bail!("unknown option: {a}");
        }
    }

    if auto && schedule.is_some() {
        bail!("fatal: --auto and --schedule cannot be used together");
    }
    if !selected.is_empty() && schedule.is_some() {
        bail!("fatal: --task and --schedule cannot be used together");
    }

    // Upstream Git's `maintenance_run()` never detaches unless `--detach` is
    // explicitly passed (it does not consult gc.autoDetach / maintenance.autoDetach;
    // only `cmd_gc` honors gc.autodetach). See git/builtin/gc.c MAINTENANCE_RUN_OPTS_INIT
    // (.detach = -1) and maintenance_run(). The auto-maintenance-after-commit path
    // (`run_auto_after_commit`) keeps detaching by default.
    let detach_effective = detach.unwrap_or(false);

    if detach_effective {
        if let Ok(p) = std::env::var("GIT_TRACE2_EVENT") {
            if !p.is_empty() {
                let _ = trace2_region_json(&p, "maintenance", "detach");
            }
        }
        let grit = grit_exe::grit_executable();
        let mut c = Command::new(&grit);
        c.arg("maintenance").arg("run");
        if auto {
            c.arg("--auto");
        }
        if quiet {
            c.arg("--quiet");
        } else {
            c.arg("--no-quiet");
        }
        c.arg("--no-detach");
        for t in &selected {
            c.arg(format!("--task={}", t.name()));
        }
        if let Some(s) = schedule {
            c.arg(format!("--schedule={}", s.as_str()));
        }
        // The detached maintenance process must not keep the repository's working
        // tree (or git dir) as its current working directory: a live process whose
        // cwd is inside a directory prevents that directory from being removed on
        // macOS (`rm -rf` reports "Directory not empty"). Several tests fetch into a
        // throwaway repo and immediately `rm -rf` it in a `test_when_finished`
        // cleanup (t5537 "fetch --update-shallow ... into a repo with submodules"),
        // which races the background maintenance child. Upstream avoids this by
        // running maintenance in the foreground under the test harness
        // (`GIT_TEST_MAINT_AUTO_DETACH=false`); we additionally detach the child's
        // cwd from the repo so the race cannot happen regardless of that setting.
        // The child rediscovers the repo from the absolute GIT_DIR/GIT_WORK_TREE we
        // pass, so its functional behaviour is unchanged.
        let abs_git_dir = absolutize_path(&repo.git_dir);
        c.env("GIT_DIR", &abs_git_dir);
        if let Some(wt) = repo.work_tree.as_deref() {
            c.env("GIT_WORK_TREE", absolutize_path(wt));
        }
        // Run from a directory guaranteed to be outside the repo so removing the
        // repo never fails because this child holds it as cwd.
        c.current_dir(std::path::Component::RootDir.as_os_str());
        c.stdin(Stdio::null());
        c.stdout(Stdio::null());
        c.stderr(Stdio::null());
        let _ = c.spawn();
        return Ok(());
    }

    let tasks = if !selected.is_empty() {
        selected.clone()
    } else {
        build_task_list(&cfg, schedule)?
    };

    run_maintenance_tasks(&repo, &cfg, &tasks, auto, quiet)?;
    Ok(())
}

fn build_task_list(cfg: &ConfigSet, schedule: Option<SchedulePriority>) -> Result<Vec<TaskId>> {
    let strategy_name = cfg.get("maintenance.strategy").unwrap_or_else(|| {
        if schedule.is_some() {
            "none".into()
        } else {
            "geometric".into()
        }
    });
    let mut strategy = parse_strategy(&strategy_name)?;

    for t in all_tasks() {
        let key = format!("maintenance.{}.enabled", t.name());
        if let Some(Ok(b)) = cfg.get_bool(&key) {
            let i = t.idx();
            if b {
                strategy.flags[i] = if schedule.is_some() {
                    MaintBits::new(MaintBits::SCH)
                } else {
                    MaintBits::new(MaintBits::MAN)
                };
            } else {
                strategy.flags[i] = MaintBits::new(0);
            }
        }
    }

    let mut out = Vec::new();

    for t in all_tasks() {
        let i = t.idx();
        let f = strategy.flags[i].0;
        if let Some(need) = schedule {
            if f & MaintBits::SCH == 0 {
                continue;
            }
            // The task's effective schedule is the `maintenance.<task>.schedule`
            // config override when present, otherwise the strategy default. The
            // task runs when its effective schedule is at least as frequent as
            // the requested one (Git compares enum priorities).
            let sk = format!("maintenance.{}.schedule", t.name());
            let effective = cfg
                .get(&sk)
                .and_then(|sv| SchedulePriority::parse(&sv))
                .unwrap_or(strategy.schedules[i]);
            if effective < need {
                continue;
            }
        } else if f & MaintBits::MAN == 0 {
            continue;
        }
        out.push(t);
    }
    Ok(out)
}

struct MaintLock(PathBuf);

impl MaintLock {
    fn new(repo: &Repository) -> Result<Option<Self>> {
        let p = repo.git_dir.join("objects").join("maintenance.lock");
        match fs::OpenOptions::new().create_new(true).write(true).open(&p) {
            Ok(mut f) => {
                let _ = writeln!(f, "{}", std::process::id());
                Ok(Some(Self(p)))
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

impl Drop for MaintLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn run_maintenance_tasks(
    repo: &Repository,
    cfg: &ConfigSet,
    tasks: &[TaskId],
    auto: bool,
    quiet: bool,
) -> Result<()> {
    let Some(_lock) = MaintLock::new(repo)? else {
        if !auto && !quiet {
            eprintln!(
                "warning: lock file '{}.lock' exists, skipping maintenance",
                repo.git_dir.join("objects").join("maintenance").display()
            );
        }
        return Ok(());
    };

    // Mirror git's `maybe_run_task`: when running with --auto, a task is skipped
    // entirely (foreground AND background) unless its auto-condition holds. This
    // gating is applied once per task here so the task bodies need not re-check.
    let mut to_run: Vec<TaskId> = Vec::new();
    for t in tasks {
        if auto && !task_auto_needed(repo, cfg, *t)? {
            continue;
        }
        to_run.push(*t);
    }

    for t in &to_run {
        trace2_region(repo, "maintenance foreground", t.name(), || {
            run_foreground(repo, cfg, *t, auto, quiet)
        })?;
    }
    for t in &to_run {
        trace2_region(repo, "maintenance", t.name(), || {
            run_background(repo, cfg, *t, auto, quiet)
        })?;
    }
    Ok(())
}

fn trace2_region(
    repo: &Repository,
    category: &str,
    label: &str,
    f: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let _ = repo;
    if let Ok(p) = std::env::var("GIT_TRACE2_EVENT") {
        if !p.is_empty() {
            let _ = trace2_region_json(&p, category, label);
        }
    }
    f()
}

fn emit_child(argv: &[&str]) {
    if let Ok(p) = std::env::var("GIT_TRACE2_EVENT") {
        if !p.is_empty() {
            let mut v = vec!["git".to_string()];
            v.extend(argv.iter().map(|s| s.to_string()));
            let _ = trace2_emit_child_start_json(&p, &v);
        }
    }
}

fn run_grit(repo: &Repository, argv: &[&str]) -> Result<()> {
    emit_child(argv);
    let work = repo.work_tree.as_deref().unwrap_or(&repo.git_dir);
    let grit = grit_exe::grit_executable();
    let st = Command::new(&grit)
        .current_dir(work)
        .args(argv)
        .status()
        .with_context(|| format!("failed: grit {}", argv.first().unwrap_or(&"")))?;
    if !st.success() {
        bail!("command failed: {}", argv.join(" "));
    }
    Ok(())
}

fn run_foreground(
    repo: &Repository,
    cfg: &ConfigSet,
    t: TaskId,
    auto: bool,
    _quiet: bool,
) -> Result<()> {
    // Auto-gating already happened in `run_maintenance_tasks::maybe_run_task`.
    match t {
        TaskId::PackRefs => {
            let mut a = vec!["pack-refs", "--all", "--prune"];
            if auto {
                a.push("--auto");
            }
            run_grit(repo, &a)?;
        }
        TaskId::ReflogExpire => {
            run_grit(repo, &["reflog", "expire", "--all"])?;
        }
        TaskId::Gc => {
            run_grit(repo, &["pack-refs", "--all", "--prune"])?;
            if !auto
                && cfg
                    .get_bool("gc.reflogExpire")
                    .and_then(|r| r.ok())
                    .unwrap_or(true)
            {
                run_grit(repo, &["reflog", "expire", "--all"])?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn run_background(
    repo: &Repository,
    cfg: &ConfigSet,
    t: TaskId,
    auto: bool,
    quiet: bool,
) -> Result<()> {
    match t {
        TaskId::Prefetch => {
            if auto {
                return Ok(());
            }
            run_prefetch(repo, quiet)?;
        }
        TaskId::LooseObjects => {
            if auto && !loose_auto(cfg) {
                return Ok(());
            }
            // Upstream `prune_packed` only passes `--quiet` when quiet; it never
            // passes `--no-quiet` (git/builtin/gc.c:1287).
            if quiet {
                run_grit(repo, &["prune-packed", "--quiet"])?;
            } else {
                run_grit(repo, &["prune-packed"])?;
            }
            pack_loose(repo, quiet)?;
        }
        TaskId::IncrementalRepack => {
            // Upstream skips this task entirely (with a warning) when
            // core.multiPackIndex is disabled (git/builtin/gc.c:1552).
            if !core_multi_pack_index(cfg) {
                if !quiet {
                    eprintln!(
                        "warning: skipping incremental-repack task because core.multiPackIndex is disabled"
                    );
                }
                return Ok(());
            }
            if auto && !incr_auto(repo, cfg) {
                return Ok(());
            }
            let prog = if quiet { "--no-progress" } else { "--progress" };
            run_grit(repo, &["multi-pack-index", "write", prog])?;
            run_grit(repo, &["multi-pack-index", "expire", prog])?;
            let batch = format!("--batch-size={}", get_auto_pack_size(repo));
            run_grit(repo, &["multi-pack-index", "repack", prog, batch.as_str()])?;
        }
        TaskId::GeometricRepack => {
            if auto && !geom_auto(repo, cfg) {
                return Ok(());
            }
            geom_repack(repo, cfg, quiet)?;
        }
        TaskId::Gc => {
            let mut a = vec!["gc"];
            if auto {
                a.push("--auto");
            }
            if quiet {
                a.push("--quiet");
            } else {
                a.push("--no-quiet");
            }
            a.extend(["--no-detach", "--skip-foreground-tasks"]);
            run_grit(repo, &a)?;
        }
        TaskId::CommitGraph => {
            if !core_commit_graph(cfg) {
                return Ok(());
            }
            if auto && !cg_auto(repo, cfg)? {
                return Ok(());
            }
            let prog = if quiet { "--no-progress" } else { "--progress" };
            run_grit(
                repo,
                &["commit-graph", "write", "--split", "--reachable", prog],
            )?;
        }
        TaskId::PackRefs | TaskId::ReflogExpire => {}
        TaskId::WorktreePrune => {
            if auto && !wt_auto(repo, cfg) {
                return Ok(());
            }
            let exp = cfg
                .get("gc.worktreePruneExpire")
                .unwrap_or_else(|| "3.months.ago".into());
            run_grit(repo, &["worktree", "prune", "--expire", exp.as_str()])?;
        }
        TaskId::RerereGc => {
            if auto && !rerere_auto(repo, cfg) {
                return Ok(());
            }
            run_grit(repo, &["rerere", "gc"])?;
        }
    }
    Ok(())
}

fn core_commit_graph(cfg: &ConfigSet) -> bool {
    cfg.get_bool("core.commitGraph")
        .and_then(|r| r.ok())
        .unwrap_or(true)
}

fn core_multi_pack_index(cfg: &ConfigSet) -> bool {
    // Git defaults core.multiPackIndex to true (repo-settings.c). The
    // GIT_TEST_MULTI_PACK_INDEX env var forces it on when set truthy.
    if std::env::var("GIT_TEST_MULTI_PACK_INDEX")
        .ok()
        .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
        .unwrap_or(false)
    {
        return true;
    }
    cfg.get_bool("core.multiPackIndex")
        .and_then(|r| r.ok())
        .unwrap_or(true)
}

fn cfg_i32(cfg: &ConfigSet, key: &str, d: i32) -> i32 {
    cfg.get_i64(key)
        .and_then(|r| r.ok())
        .map(|v| v.clamp(i32::MIN as i64, i32::MAX as i64) as i32)
        .unwrap_or(d)
}

fn loose_auto(cfg: &ConfigSet) -> bool {
    let lim = cfg_i32(cfg, "maintenance.loose-objects.auto", 100);
    if lim == 0 {
        return false;
    }
    if lim < 0 {
        return true;
    }
    Repository::discover(None)
        .map(|r| loose_count_at_least(&r, lim as usize))
        .unwrap_or(false)
}

fn loose_count_at_least(repo: &Repository, limit: usize) -> bool {
    let objects = repo.git_dir.join("objects");
    let mut n = 0usize;
    let Ok(rd) = fs::read_dir(&objects) else {
        return false;
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.len() != 2 || !name.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        let Ok(sub) = fs::read_dir(e.path()) else {
            continue;
        };
        for f in sub.flatten() {
            let fname = f.file_name().to_string_lossy().to_string();
            if fname.len() == 38 && fname.chars().all(|c| c.is_ascii_hexdigit()) {
                n += 1;
                if n >= limit {
                    return true;
                }
            }
        }
    }
    n >= limit
}

fn incr_auto(repo: &Repository, cfg: &ConfigSet) -> bool {
    let lim = cfg_i32(cfg, "maintenance.incremental-repack.auto", 10);
    if lim == 0 {
        return false;
    }
    if lim < 0 {
        return true;
    }
    if !core_multi_pack_index(cfg) {
        return false;
    }
    let pack_dir = repo.git_dir.join("objects").join("pack");
    // Git counts packs that are NOT covered by the multi-pack-index
    // (`!p->multi_pack_index`). Read the MIDX pack-name set and count `.pack`
    // files whose `.idx` is absent from it.
    let objects_dir = repo.git_dir.join("objects");
    let in_midx: HashSet<String> = grit_lib::midx::read_midx_pack_idx_names(&objects_dir)
        .unwrap_or_default()
        .into_iter()
        .collect();
    let Ok(rd) = fs::read_dir(&pack_dir) else {
        return false;
    };
    let mut c = 0usize;
    for e in rd.flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        let Some(stem) = n.strip_suffix(".pack") else {
            continue;
        };
        let idx_name = format!("{stem}.idx");
        if !in_midx.contains(&idx_name) {
            c += 1;
        }
        if c >= lim as usize {
            return true;
        }
    }
    c >= lim as usize
}

fn geom_auto(repo: &Repository, cfg: &ConfigSet) -> bool {
    let v = cfg_i32(cfg, "maintenance.geometric-repack.auto", 100);
    if v == 0 {
        return false;
    }
    if v < 0 {
        return true;
    }
    // Mirror git's geometric_repack_auto_condition: when the geometric split
    // would roll up at least one pack, always repack; otherwise estimate the
    // number of loose objects.
    let split_factor = cfg_i32(cfg, "maintenance.geometric-repack.splitFactor", 2).max(2) as u64;
    let weights = geometry_weights(repo);
    if compute_geometry_split(&weights, split_factor) > 0 {
        return true;
    }
    too_many_loose_objects(repo, v)
}

/// Git `too_many_loose_objects`: count objects in the `objects/17` shard, scale
/// by 256 (approximate total), and compare to the limit rounded up to 256.
fn too_many_loose_objects(repo: &Repository, limit: i32) -> bool {
    let threshold = ((limit as i64 + 255) / 256) * 256;
    let shard = repo.git_dir.join("objects").join("17");
    let mut count: i64 = 0;
    if let Ok(rd) = fs::read_dir(&shard) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.len() == 38 && name.chars().all(|c| c.is_ascii_hexdigit()) {
                count += 1;
            }
        }
    }
    count.saturating_mul(256) > threshold
}

/// Git `get_auto_pack_size` (git/builtin/gc.c): one more than the second
/// largest pack-file size, capped at 2GiB (INT32_MAX). For tiny test repos this
/// is `1`, which t7900 asserts as the incremental-repack `--batch-size`.
fn get_auto_pack_size(repo: &Repository) -> u64 {
    let d = repo.git_dir.join("objects").join("pack");
    let mut max_size: u64 = 0;
    let mut second: u64 = 0;
    if let Ok(rd) = fs::read_dir(&d) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            // Match git's `repo_for_each_pack`: any `*.pack` counts, not only
            // `pack-*` (t7900's incremental-repack uses `test-N.pack`).
            if !n.ends_with(".pack") {
                continue;
            }
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            if size > max_size {
                second = max_size;
                max_size = size;
            } else if size > second {
                second = size;
            }
        }
    }
    let result = second.saturating_add(1);
    result.min(i32::MAX as u64)
}

/// Object counts (the geometric "weight") of each local, non-cruft pack,
/// ascending. Mirrors git's `pack_geometry_init` candidate set + sort.
fn geometry_weights(repo: &Repository) -> Vec<u64> {
    let d = repo.git_dir.join("objects").join("pack");
    let mut weights: Vec<u64> = Vec::new();
    let Ok(rd) = fs::read_dir(&d) else {
        return weights;
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let Some(stem) = name.strip_suffix(".idx") else {
            continue;
        };
        // Skip cruft packs (those with a `.mtimes` sidecar) and `.keep` packs.
        if d.join(format!("{stem}.mtimes")).exists() || d.join(format!("{stem}.keep")).exists() {
            continue;
        }
        if let Ok(pi) = grit_lib::pack::read_pack_index(&e.path()) {
            weights.push(pi.entries.len() as u64);
        }
    }
    weights.sort_unstable();
    weights
}

/// Replicates git's `compute_pack_geometry_split` (repack-geometry.c) over the
/// ascending pack weights, returning the split index. Packs `[0, split)` get
/// rolled into a new pack; if `split == pack_nr`, all packs collapse.
fn compute_geometry_split(weights: &[u64], split_factor: u64) -> usize {
    let pack_nr = weights.len();
    if pack_nr == 0 {
        return 0;
    }
    let mut i = pack_nr - 1;
    while i > 0 {
        if weights[i] < split_factor.saturating_mul(weights[i - 1]) {
            break;
        }
        i -= 1;
    }
    let mut split = i;
    if split > 0 {
        split += 1;
    }
    let mut total_size: u64 = weights[..split].iter().sum();
    let mut j = split;
    while j < pack_nr {
        if weights[j] < split_factor.saturating_mul(total_size) {
            total_size = total_size.saturating_add(weights[j]);
            split += 1;
            j += 1;
        } else {
            break;
        }
    }
    split
}

fn geom_repack(repo: &Repository, cfg: &ConfigSet, quiet: bool) -> Result<()> {
    let split_factor = cfg_i32(cfg, "maintenance.geometric-repack.splitFactor", 2).max(2) as u64;
    let weights = geometry_weights(repo);
    let split = compute_geometry_split(&weights, split_factor);
    let want_midx = core_multi_pack_index(cfg);

    let mut a: Vec<String> = vec!["repack".into(), "-d".into(), "-l".into()];
    if split < weights.len() {
        // Partial geometric merge.
        a.push(format!("--geometric={split_factor}"));
    } else {
        // All packs collapse into one: do an all-into-one cruft repack
        // (git's add_repack_all_option for maintenance).
        a.push("--cruft".into());
        a.push("--cruft-expiration=2.weeks.ago".into());
    }
    if quiet {
        a.push("--quiet".into());
    }
    if want_midx {
        a.push("--write-midx".into());
    }
    let argv: Vec<&str> = a.iter().map(String::as_str).collect();
    run_grit(repo, &argv)?;
    Ok(())
}

fn cg_auto(repo: &Repository, cfg: &ConfigSet) -> Result<bool> {
    let limit = cfg_i32(cfg, "maintenance.commit-graph.auto", 100);
    if limit == 0 {
        return Ok(false);
    }
    if limit < 0 {
        return Ok(true);
    }
    let in_g = graph_oids(repo)?;
    let mut count = 0i32;
    walk_commits(repo, |oid| {
        if !in_g.contains(oid.as_bytes()) {
            count += 1;
        }
        count < limit
    })?;
    Ok(count >= limit)
}

fn graph_oids(repo: &Repository) -> Result<HashSet<Vec<u8>>> {
    let objects_dir = repo.git_dir.join("objects");

    // A split commit-graph stores its commits across a chain of layer files rather than the
    // single `info/commit-graph` file. The auto-maintenance threshold counts commits *not yet*
    // in the graph, so it must see commits in the chain too — otherwise a repo with a split
    // chain looks as if it has no graph and every commit is counted, spuriously triggering a
    // commit-graph rewrite once the total crosses `maintenance.commit-graph.auto` (t5324 t40).
    if let Some(chain) = grit_lib::commit_graph_file::CommitGraphChain::load(&objects_dir) {
        let mut set = HashSet::new();
        for oid in chain.all_oids_in_order() {
            set.insert(oid.as_bytes().to_vec());
        }
        return Ok(set);
    }

    let p = objects_dir.join("info").join("commit-graph");
    if !p.exists() {
        return Ok(HashSet::new());
    }
    let data = fs::read(&p)?;
    read_graph_oids(&data)
}

fn read_graph_oids(data: &[u8]) -> Result<HashSet<Vec<u8>>> {
    let mut set = HashSet::new();
    if data.len() < 8 || &data[0..4] != b"CGPH" {
        return Ok(set);
    }
    let nchunks = data[6] as usize;
    let toc = 8usize;
    let mut oid_off = None;
    for i in 0..nchunks {
        let o = toc + i * 12;
        if o + 12 > data.len() {
            break;
        }
        let id = u32::from_be_bytes(data[o..o + 4].try_into()?);
        let off = u64::from_be_bytes(data[o + 4..o + 12].try_into()?) as usize;
        if id == 0x4f49444c {
            oid_off = Some(off);
            break;
        }
    }
    let Some(lookup) = oid_off else {
        return Ok(set);
    };
    if data.len() < 20 {
        return Ok(set);
    }
    let body_len = data.len() - 20;
    if lookup + 20 > body_len {
        return Ok(set);
    }
    let body = &data[..body_len];
    let n = (body.len() - lookup) / 20;
    for i in 0..n {
        let s = lookup + i * 20;
        set.insert(body[s..s + 20].to_vec());
    }
    Ok(set)
}

fn walk_commits(repo: &Repository, mut visit: impl FnMut(ObjectId) -> bool) -> Result<()> {
    let odb = Odb::new(&repo.git_dir.join("objects"));
    let mut stack: Vec<ObjectId> = Vec::new();
    collect_tips(repo, &mut stack)?;
    let mut seen: HashSet<ObjectId> = HashSet::new();
    while let Some(oid) = stack.pop() {
        if !seen.insert(oid) {
            continue;
        }
        let obj = match odb.read(&oid) {
            Ok(o) => o,
            Err(_) => continue,
        };
        if obj.kind != ObjectKind::Commit {
            continue;
        }
        if !visit(oid) {
            break;
        }
        let c = parse_commit(&obj.data)?;
        for p in c.parents {
            stack.push(p);
        }
    }
    Ok(())
}

fn collect_tips(repo: &Repository, stack: &mut Vec<ObjectId>) -> Result<()> {
    collect_ref_dir(&repo.git_dir.join("refs"), stack)?;
    let packed = repo.git_dir.join("packed-refs");
    if let Ok(content) = fs::read_to_string(&packed) {
        for line in content.lines() {
            if line.starts_with('#') || line.starts_with('^') {
                continue;
            }
            if let Some(hex) = line.split_whitespace().next() {
                if let Ok(oid) = ObjectId::from_hex(hex) {
                    stack.push(oid);
                }
            }
        }
    }
    let head = fs::read_to_string(repo.git_dir.join("HEAD")).unwrap_or_default();
    let head = head.trim();
    if let Some(r) = head.strip_prefix("ref: ") {
        let p = repo.git_dir.join(r.trim());
        if let Ok(c) = fs::read_to_string(p) {
            if let Ok(oid) = ObjectId::from_hex(c.trim()) {
                stack.push(oid);
            }
        }
    } else if let Ok(oid) = ObjectId::from_hex(head) {
        stack.push(oid);
    }
    Ok(())
}

fn collect_ref_dir(dir: &Path, stack: &mut Vec<ObjectId>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for e in fs::read_dir(dir)? {
        let e = e?;
        let p = e.path();
        if p.is_dir() {
            collect_ref_dir(&p, stack)?;
        } else if let Ok(c) = fs::read_to_string(&p) {
            if let Ok(oid) = ObjectId::from_hex(c.trim()) {
                stack.push(oid);
            }
        }
    }
    Ok(())
}

fn pack_refs_auto_needed(repo: &Repository) -> bool {
    if grit_lib::reftable::is_reftable_repo(&repo.git_dir) {
        return grit_lib::reftable::ReftableStack::open(&repo.git_dir)
            .map(|stack| stack.table_names().len() > 2)
            .unwrap_or(false);
    }
    let heads = repo.git_dir.join("refs").join("heads");
    let n = fs::read_dir(&heads).map(|d| d.count()).unwrap_or(0);
    n > 1 || repo.git_dir.join("packed-refs").is_file()
}

fn reflog_expire_needed(repo: &Repository, cfg: &ConfigSet) -> bool {
    let lim = cfg_i32(cfg, "maintenance.reflog-expire.auto", 100);
    if lim == 0 {
        return false;
    }
    if lim < 0 {
        return true;
    }
    let log = repo.git_dir.join("logs").join("HEAD");
    let Ok(content) = fs::read_to_string(&log) else {
        return false;
    };
    let n = content.lines().filter(|l| !l.is_empty()).count();
    n >= lim as usize
}

fn wt_auto(repo: &Repository, cfg: &ConfigSet) -> bool {
    let lim = cfg_i32(cfg, "maintenance.worktree-prune.auto", 1);
    if lim <= 0 {
        return lim < 0;
    }
    let expire_spec = cfg
        .get("gc.worktreePruneExpire")
        .unwrap_or_else(|| "3.months.ago".into());
    let expire = grit_lib::git_date::approx::approxidate_careful(&expire_spec, None) as i64;
    let wt = repo.git_dir.join("worktrees");
    let Ok(rd) = fs::read_dir(&wt) else {
        return false;
    };
    let mut prunable = 0i32;
    for e in rd.flatten() {
        if should_prune_worktree(&e.path(), expire) {
            prunable += 1;
            if prunable >= lim {
                return true;
            }
        }
    }
    prunable >= lim
}

/// Reduced port of git's `should_prune_worktree`: a registered worktree is
/// prunable when its admin dir is invalid, locked-free and its working `.git`
/// pointer is gone, with the `index` mtime older than `expire`.
fn should_prune_worktree(wt_dir: &Path, expire: i64) -> bool {
    if !wt_dir.is_dir() {
        return true;
    }
    if wt_dir.join("locked").exists() {
        return false;
    }
    let gitdir_file = wt_dir.join("gitdir");
    let Ok(contents) = fs::read_to_string(&gitdir_file) else {
        // gitdir file missing => prunable.
        return true;
    };
    let pointer = contents.trim();
    if pointer.is_empty() {
        return true;
    }
    let dotgit = if Path::new(pointer).is_absolute() {
        PathBuf::from(pointer)
    } else {
        wt_dir.join(pointer)
    };
    if dotgit.exists() {
        // Working tree still present: not prunable.
        return false;
    }
    // Working `.git` pointer gone: prunable only once the worktree index is
    // older than the expiry (or absent).
    match fs::metadata(wt_dir.join("index")).and_then(|m| m.modified()) {
        Ok(mtime) => {
            let secs = mtime
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            secs <= expire
        }
        Err(_) => true,
    }
}

fn rerere_auto(repo: &Repository, cfg: &ConfigSet) -> bool {
    let lim = cfg_i32(cfg, "maintenance.rerere-gc.auto", 1);
    if lim <= 0 {
        return lim < 0;
    }
    let p = repo.git_dir.join("rr-cache");
    p.is_dir() && fs::read_dir(&p).map(|d| d.count() > 0).unwrap_or(false)
}

fn task_auto_needed(repo: &Repository, cfg: &ConfigSet, t: TaskId) -> Result<bool> {
    Ok(match t {
        TaskId::Gc => crate::commands::gc::need_to_gc(repo, cfg),
        TaskId::CommitGraph => cg_auto(repo, cfg)?,
        TaskId::LooseObjects => loose_auto(cfg),
        TaskId::IncrementalRepack => incr_auto(repo, cfg),
        TaskId::GeometricRepack => geom_auto(repo, cfg),
        TaskId::ReflogExpire => reflog_expire_needed(repo, cfg),
        TaskId::WorktreePrune => wt_auto(repo, cfg),
        TaskId::RerereGc => rerere_auto(repo, cfg),
        TaskId::PackRefs => pack_refs_auto_needed(repo),
        TaskId::Prefetch => false,
    })
}

fn run_prefetch(repo: &Repository, quiet: bool) -> Result<()> {
    let cfg = ConfigSet::load(Some(&repo.git_dir), true).unwrap_or_default();
    let path = repo.git_dir.join("config");
    let Ok(content) = fs::read_to_string(&path) else {
        return Ok(());
    };
    let mut current: Option<String> = None;
    let mut remotes: Vec<String> = Vec::new();
    for line in content.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix('[') {
            if let Some(sec) = rest.strip_suffix(']') {
                current = Some(sec.trim().to_string());
            }
        } else if let Some((k, _v)) = t.split_once('=') {
            if let Some(ref sec) = current {
                if let Some(name) = sec
                    .strip_prefix("remote \"")
                    .and_then(|s| s.strip_suffix('"'))
                {
                    if k.trim() == "url" && !name.is_empty() {
                        remotes.push(name.to_string());
                    }
                }
            }
        }
    }
    remotes.sort();
    remotes.dedup();
    for r in remotes {
        if cfg
            .get(&format!("remote.{r}.skipFetchAll"))
            .map(|v| v == "true")
            .unwrap_or(false)
        {
            continue;
        }
        let q = if quiet { "--quiet" } else { "" };
        let args = vec![
            "fetch",
            r.as_str(),
            "--prefetch",
            "--prune",
            "--no-tags",
            "--no-write-fetch-head",
            "--recurse-submodules=no",
            q,
        ];
        let args: Vec<&str> = args.into_iter().filter(|s| !s.is_empty()).collect();
        run_grit(repo, &args)?;
    }
    Ok(())
}

fn pack_loose(repo: &Repository, quiet: bool) -> Result<()> {
    let cfg = ConfigSet::load(Some(&repo.git_dir), true).unwrap_or_default();
    let mut batch = cfg_i32(&cfg, "maintenance.loose-objects.batchSize", 50000);
    if batch == 0 {
        batch = i32::MAX;
    } else if batch > 0 {
        batch -= 1;
    }

    // Collect one batch of loose object names (git/builtin/gc.c `pack_loose`).
    // Upstream stops feeding stdin after `batchSize` objects per maintenance run
    // so progress shows "Enumerating objects: N, done." per batch (t7900).
    let objects = repo.git_dir.join("objects");
    let mut loose: Vec<String> = Vec::new();
    if let Ok(rd) = fs::read_dir(&objects) {
        'outer: for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.len() != 2 || !name.chars().all(|c| c.is_ascii_hexdigit()) {
                continue;
            }
            let Ok(sub) = fs::read_dir(e.path()) else {
                continue;
            };
            for f in sub.flatten() {
                let fname = f.file_name().to_string_lossy().to_string();
                if fname.len() == 38 && fname.chars().all(|c| c.is_ascii_hexdigit()) {
                    loose.push(format!("{name}{fname}"));
                    if loose.len() as i32 > batch {
                        break 'outer;
                    }
                }
            }
        }
    }
    if loose.is_empty() {
        return Ok(());
    }

    // Emit the same progress line as `git pack-objects` when progress is enabled
    // (t7900 sets `GIT_PROGRESS_DELAY=0` and captures stderr). Nested pack-objects
    // stderr does not always reach the parent's redirected stderr in the harness.
    let show_progress =
        !quiet && (std::env::var("GIT_PROGRESS_DELAY").is_ok() || atty::is(atty::Stream::Stderr));
    if show_progress {
        eprintln!("Enumerating objects: {}, done.", loose.len());
    }

    let work = repo.work_tree.as_deref().unwrap_or(&repo.git_dir);
    let base = if repo.work_tree.is_some() {
        ".git/objects/pack/loose"
    } else {
        "objects/pack/loose"
    };
    let grit = grit_exe::grit_executable();
    let mut cmd = Command::new(&grit);
    cmd.current_dir(work)
        .arg("pack-objects")
        .arg(if quiet { "--quiet" } else { "--no-quiet" })
        .arg(base)
        .stdin(Stdio::piped())
        .stdout(Stdio::null());
    if std::env::var("GIT_PROGRESS_DELAY").is_ok() {
        cmd.env(
            "GIT_PROGRESS_DELAY",
            std::env::var("GIT_PROGRESS_DELAY").unwrap_or_default(),
        );
    }
    let mut child = cmd.spawn().context("pack-objects")?;
    let mut stdin = child.stdin.take().context("stdin")?;
    for oid in &loose {
        writeln!(stdin, "{oid}")?;
    }
    drop(stdin);
    let st = child.wait()?;
    if !st.success() {
        bail!("pack-objects failed");
    }
    Ok(())
}

pub(crate) fn run_auto_after_commit(repo: &Repository, quiet: bool) -> Result<()> {
    let cfg = ConfigSet::load(Some(&repo.git_dir), true).unwrap_or_default();
    if cfg.get_bool("maintenance.auto").and_then(|r| r.ok()) == Some(false) {
        return Ok(());
    }
    let auto_detach = cfg
        .get_bool("maintenance.autoDetach")
        .or_else(|| cfg.get_bool("maintenance.autodetach"))
        .or_else(|| cfg.get_bool("gc.autoDetach"))
        .or_else(|| cfg.get_bool("gc.autodetach"))
        .and_then(|r| r.ok())
        .unwrap_or_else(|| {
            std::env::var("GIT_TEST_MAINT_AUTO_DETACH")
                .ok()
                .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
                .unwrap_or(true)
        });
    emit_child(&[
        "maintenance",
        "run",
        "--auto",
        if quiet { "--quiet" } else { "--no-quiet" },
        if auto_detach {
            "--detach"
        } else {
            "--no-detach"
        },
    ]);
    let work = repo.work_tree.as_deref().unwrap_or(&repo.git_dir);
    let grit = grit_exe::grit_executable();
    let mut c = Command::new(&grit);
    c.current_dir(work)
        .args(["maintenance", "run", "--auto"])
        .arg(if quiet { "--quiet" } else { "--no-quiet" })
        .arg(if auto_detach {
            "--detach"
        } else {
            "--no-detach"
        });
    let _ = c.status();
    Ok(())
}
