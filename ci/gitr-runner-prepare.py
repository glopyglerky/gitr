#!/usr/bin/env python3
"""Prepare reviewed ephemeral runner wiring for qualification; never update refs."""
from pathlib import Path


def replace(path, old, new):
    path = Path(path)
    text = path.read_text()
    assert text.count(old) == 1, (str(path), text.count(old), old[:90])
    path.write_text(text.replace(old, new))


root = 'rust/gitr/src/'
replace(root+'actions.rs', 'pub struct ActionDb {\n    db: Database,\n}', '''pub struct ActionDb {
    db: Database,
    runner: Option<runner::EphemeralRunner>,
}''')
replace(root+'actions.rs', 'impl ActionDb {\n    pub fn open(', '''impl ActionDb {
    /// Choose disposable processes explicitly when linking Gitr into another host.
    /// The executable must be a trusted Gitr binary, not repository-provided code.
    pub fn with_ephemeral_runner(mut self, runner: runner::EphemeralRunner) -> Self {
        self.runner = Some(runner);
        self
    }

    fn open_for_cli(repo: &gix::Repository, create: bool) -> Result<Self> {
        Ok(Self::open(repo, create)?.with_ephemeral_runner(runner::EphemeralRunner::current_executable()?))
    }

    pub fn open(''')
replace(root+'actions.rs', 'Ok(Self { db })', 'Ok(Self { db, runner: None })')
replace(root+'actions.rs', '''        // Validation and eager compilation are deliberately outside a writer.
        prepare(bytes)?;''', '''        // Validation and eager compilation are deliberately outside a writer.
        if let Some(runner) = &self.runner { runner.validate(bytes)?; }
        else { prepare(bytes)?; }''')
replace(root+'actions.rs', '''        let module = self.object(&claim.spec.module_sha256)?;
        let input = self.object(&claim.spec.input_sha256)?;
        let result = execute(&module, input);''', '''        let result = (|| {
            let module = self.object(&claim.spec.module_sha256)?;
            let input = self.object(&claim.spec.input_sha256)?;
            if let Some(runner) = &self.runner {
                runner.execute_while_current(&module, &input, || {
                    let current = self.get(&claim.id)?;
                    Ok(current.status == Status::Running && current.attempt == claim.attempt
                        && current.spec == claim.spec
                        && current.lease_until_ms.is_some_and(|until| now_ms().is_ok_and(|now| now < until)))
                })
            } else { execute(&module, input) }
        })();''')
replace(root+'actions.rs', 'guest validation, trap or resource limit failure; output discarded', 'guest, worker, integrity or resource limit failure; output discarded')
replace(root+'actions.rs', '''    let db = ActionDb::open(repo, create)?;
    match request {''', '''    let db = ActionDb::open(repo, create)?;
    control_with_db(repo, &db, request)
}

/// Shared typed operations for hosts that explicitly configure their executor.
pub fn control_with_db(repo: &gix::Repository, db: &ActionDb, request: Request) -> Result<Job> {
    match request {''')
replace(root+'actions.rs', '''        return print_json(&control(&repo, serde_json::from_slice(&bytes)?)?);''', '''        let request: Request = serde_json::from_slice(&bytes)?;
        let db = ActionDb::open_for_cli(&repo, matches!(request, Request::Submit { .. }))?;
        return print_json(&control_with_db(&repo, &db, request)?);''')
replace(root+'actions.rs', 'let db = ActionDb::open(&repo, create)?;', 'let db = ActionDb::open_for_cli(&repo, create)?;')
path = Path(root+'actions.rs')
path.write_text(path.read_text() + '\n#[path = "actions_runner.rs"]\npub mod runner;\n')
replace(root+'actions_api.rs', 'Arc::new(ActionDb::open(repo, true)?)', 'Arc::new(ActionDb::open_for_cli(repo, true)?)')
replace(root+'actions_workflow.rs', 'let result = ActionDb::open(repo, false)?.workflow_run(&run)?;', 'let result = ActionDb::open_for_cli(repo, false)?.workflow_run(&run)?;')
replace(root+'main.rs', 'fn main() -> std::process::ExitCode {\n    match run(Cli::parse()) {', '''fn main() -> std::process::ExitCode {
    // The worker never parses repository options, discovers Git or opens redb.
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() == 2 && args[1] == actions::runner::WORKER_ARGUMENT {
        return match actions::runner::worker_stdio() {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(_) => std::process::ExitCode::FAILURE,
        };
    }
    match run(Cli::parse()) {''')
replace(root+'actions_runner.rs', '''        let mut command = Command::new(&self.executable);
        command.arg(WORKER_ARGUMENT).env_clear().current_dir(directory.path());
        // Windows uses SystemRoot to locate operating-system components. No PATH,
        // HOME, Git configuration, proxy, credential or loader override is copied.
        #[cfg(windows)]
        if let Some(root) = std::env::var_os("SystemRoot") { command.env("SystemRoot", root); }
        let bytes = supervise(command, request, self.timeout, is_current)?;''', '''        let mut command = sanitized_command(&self.executable, directory.path());
        command.arg(WORKER_ARGUMENT);
        let bytes = supervise(command, request, self.timeout, is_current)?;''')
replace(root+'actions_runner.rs', 'fn request_identity(', '''fn sanitized_command(executable: &Path, directory: &Path) -> Command {
    let mut command = Command::new(executable);
    command.env_clear().current_dir(directory);
    // Windows uses SystemRoot to locate operating-system components. No PATH,
    // HOME, Git configuration, proxy, credential or loader override is copied.
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") { command.env("SystemRoot", root); }
    command
}

fn request_identity(''')
print('Prepared ephemeral worker wiring; no runtime dependency, lockfile or data-format changes.')
