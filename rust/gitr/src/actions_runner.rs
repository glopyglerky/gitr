//! Disposable Wasmi processes with bounded binary IPC and parent-loss shutdown.
//!
//! This is process separation, NOT a kernel-enforced filesystem/network sandbox.
//! The trusted worker executable uses the existing no-WASI Wasmi capability set.
//! Only the supervisor owns redb, claims and publication. Linkers may explicitly
//! choose this runner; the library's default embedded interpreter remains usable.
use super::{MAX_BYTES, bounded_read, execute, prepare};
use anyhow::{Context, Result, bail, ensure};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::mpsc::{self, TryRecvError},
    thread,
    time::{Duration, Instant},
};

pub const WORKER_ARGUMENT: &str = "__gitr-wasm-worker-v1";
const REQUEST_MAGIC: &[u8; 8] = b"GITRWK1\0";
const RESPONSE_MAGIC: &[u8; 8] = b"GITRWR1\0";
const RESPONSE_HEADER: usize = 8 + 32 + 1 + 4 + 4 + 32;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);
const WORKER_LIFETIME: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(10);

/// A trusted absolute Gitr executable, never selected by repository configuration.
/// Each validation/execution launches a fresh process and fresh Wasmi instance.
#[derive(Clone, Debug)]
pub struct EphemeralRunner {
    executable: PathBuf,
    timeout: Duration,
}

impl EphemeralRunner {
    pub fn new(executable: impl AsRef<Path>) -> Result<Self> {
        let path = executable.as_ref();
        ensure!(path.is_absolute(), "worker executable must be absolute");
        let executable = fs::canonicalize(path).context("resolve trusted worker executable")?;
        ensure!(
            executable.is_file(),
            "worker executable must be a regular file"
        );
        Ok(Self {
            executable,
            timeout: DEFAULT_TIMEOUT,
        })
    }

    pub fn current_executable() -> Result<Self> {
        Self::new(std::env::current_exe()?)
    }

    /// The deadline may be tightened, never extended past the reviewed profile.
    pub fn with_timeout(mut self, timeout: Duration) -> Result<Self> {
        ensure!(
            !timeout.is_zero() && timeout <= DEFAULT_TIMEOUT,
            "invalid worker timeout"
        );
        self.timeout = timeout;
        Ok(self)
    }

    pub fn validate(&self, module: &[u8]) -> Result<()> {
        self.invoke(0, module, &[], || Ok(true)).map(|_| ())
    }

    pub fn execute(&self, module: &[u8], input: &[u8]) -> Result<(i32, Vec<u8>)> {
        self.execute_while_current(module, input, || Ok(true))
    }

    pub(super) fn execute_while_current(
        &self,
        module: &[u8],
        input: &[u8],
        is_current: impl FnMut() -> Result<bool>,
    ) -> Result<(i32, Vec<u8>)> {
        self.invoke(1, module, input, is_current)
    }

    fn invoke(
        &self,
        mode: u8,
        module: &[u8],
        input: &[u8],
        is_current: impl FnMut() -> Result<bool>,
    ) -> Result<(i32, Vec<u8>)> {
        let request = encode_request(mode, module, input)?;
        let identity = request_identity(mode, module, input);
        // No checkout, repository file, token or database path enters this cwd.
        // TempDir removes it after the child has been reaped on normal/error paths.
        let directory = tempfile::Builder::new()
            .prefix("gitr-wasm-worker-")
            .tempdir()?;
        let mut command = sanitized_command(&self.executable, directory.path());
        command.arg(WORKER_ARGUMENT);
        let bytes = supervise(command, request, self.timeout, is_current)?;
        decode_response(&bytes, mode, identity)
    }
}

fn sanitized_command(executable: &Path, directory: &Path) -> Command {
    let mut command = Command::new(executable);
    command.env_clear().current_dir(directory);
    // Windows uses SystemRoot to locate operating-system components. No PATH,
    // HOME, Git configuration, proxy, credential or loader override is copied.
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", root);
    }
    command
}

fn request_identity(mode: u8, module: &[u8], input: &[u8]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(REQUEST_MAGIC);
    hash.update([mode]);
    hash.update((module.len() as u32).to_be_bytes());
    hash.update((input.len() as u32).to_be_bytes());
    hash.update(module);
    hash.update(input);
    hash.finalize().into()
}

fn encode_request(mode: u8, module: &[u8], input: &[u8]) -> Result<Vec<u8>> {
    ensure!(
        mode <= 1 && (mode == 1 || input.is_empty()),
        "invalid worker operation"
    );
    ensure!(
        module.len() <= MAX_BYTES && input.len() <= MAX_BYTES,
        "worker input exceeds limit"
    );
    let mut bytes = Vec::with_capacity(49 + module.len() + input.len());
    bytes.extend_from_slice(REQUEST_MAGIC);
    bytes.push(mode);
    bytes.extend_from_slice(&(module.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&(input.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&request_identity(mode, module, input));
    bytes.extend_from_slice(module);
    bytes.extend_from_slice(input);
    Ok(bytes)
}

fn read_array<const N: usize>(reader: &mut impl Read) -> Result<[u8; N]> {
    let mut bytes = [0; N];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

struct Input {
    mode: u8,
    identity: [u8; 32],
    module: Vec<u8>,
    bytes: Vec<u8>,
}
fn read_request(reader: &mut impl Read) -> Result<Input> {
    ensure!(
        &read_array::<8>(reader)? == REQUEST_MAGIC,
        "invalid worker protocol"
    );
    let mode = read_array::<1>(reader)?[0];
    let module_len = u32::from_be_bytes(read_array(reader)?) as usize;
    let input_len = u32::from_be_bytes(read_array(reader)?) as usize;
    ensure!(
        mode <= 1 && (mode == 1 || input_len == 0),
        "invalid worker operation"
    );
    ensure!(
        module_len <= MAX_BYTES && input_len <= MAX_BYTES,
        "worker input exceeds limit"
    );
    let identity = read_array::<32>(reader)?;
    let mut module = vec![0; module_len];
    let mut bytes = vec![0; input_len];
    reader.read_exact(&mut module)?;
    reader.read_exact(&mut bytes)?;
    ensure!(
        identity == request_identity(mode, &module, &bytes),
        "worker input integrity failure"
    );
    Ok(Input {
        mode,
        identity,
        module,
        bytes,
    })
}

fn response(identity: [u8; 32], outcome: u8, code: i32, output: &[u8]) -> Result<Vec<u8>> {
    ensure!(output.len() <= MAX_BYTES, "worker output exceeds limit");
    ensure!(
        (outcome == 1 && code == 0) || output.is_empty(),
        "failed worker output must be discarded"
    );
    let mut bytes = Vec::with_capacity(RESPONSE_HEADER + output.len());
    bytes.extend_from_slice(RESPONSE_MAGIC);
    bytes.extend_from_slice(&identity);
    bytes.push(outcome);
    bytes.extend_from_slice(&code.to_be_bytes());
    bytes.extend_from_slice(&(output.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&Sha256::digest(output));
    bytes.extend_from_slice(output);
    Ok(bytes)
}

fn decode_response(bytes: &[u8], mode: u8, identity: [u8; 32]) -> Result<(i32, Vec<u8>)> {
    ensure!(
        bytes.len() <= RESPONSE_HEADER + MAX_BYTES,
        "worker output exceeds limit"
    );
    let mut reader = io::Cursor::new(bytes);
    ensure!(
        &read_array::<8>(&mut reader)? == RESPONSE_MAGIC,
        "invalid worker response"
    );
    ensure!(
        read_array::<32>(&mut reader)? == identity,
        "worker response does not match request"
    );
    let outcome = read_array::<1>(&mut reader)?[0];
    let code = i32::from_be_bytes(read_array(&mut reader)?);
    let length = u32::from_be_bytes(read_array(&mut reader)?) as usize;
    let expected = read_array::<32>(&mut reader)?;
    ensure!(
        length <= MAX_BYTES && bytes.len() == RESPONSE_HEADER + length,
        "invalid worker output framing"
    );
    let output = &bytes[RESPONSE_HEADER..];
    let actual: [u8; 32] = Sha256::digest(output).into();
    ensure!(actual == expected, "worker output integrity failure");
    ensure!(
        outcome != 2,
        "guest validation, trap or resource limit failure"
    );
    ensure!(
        (mode == 0 && outcome == 0 && code == 0 && output.is_empty())
            || (mode == 1 && outcome == 1 && (code == 0 || output.is_empty())),
        "invalid worker outcome"
    );
    Ok((code, output.to_vec()))
}

struct Reap(Child);
impl Drop for Reap {
    fn drop(&mut self) {
        // Drop alone does not stop/wait for a Rust Child. This guard handles all
        // supervisor failures and unwinding before scoped I/O threads are joined.
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

fn supervise(
    mut command: Command,
    request: Vec<u8>,
    timeout: Duration,
    mut is_current: impl FnMut() -> Result<bool>,
) -> Result<Vec<u8>> {
    ensure!(is_current()?, "worker claim is no longer current");
    let deadline = Instant::now()
        .checked_add(timeout)
        .context("worker deadline overflow")?;
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    thread::scope(|scope| {
        // Child must be scoped INSIDE this closure: kill/reap on every exit before
        // thread::scope waits for a blocked pipe reader/writer.
        let mut child = Reap(command.spawn().context("start ephemeral worker")?);
        let mut stdin = child.0.stdin.take().context("worker stdin unavailable")?;
        let stdout = child.0.stdout.take().context("worker stdout unavailable")?;
        let (sent, received) = mpsc::sync_channel(1);
        let writer = scope.spawn(move || {
            let result = stdin
                .write_all(&request)
                .and_then(|()| stdin.flush())
                .map(|()| stdin);
            let _ = sent.send(result);
        });
        let (sent, output) = mpsc::sync_channel(1);
        let reader = scope.spawn(move || {
            let _ = sent.send(bounded_read(stdout, RESPONSE_HEADER + MAX_BYTES));
        });
        let mut lifeline = None;
        let mut reply = None;
        let status: ExitStatus = loop {
            if lifeline.is_none() {
                match received.try_recv() {
                    Ok(result) => lifeline = Some(result.context("send worker request")?),
                    Err(TryRecvError::Disconnected) => {
                        bail!("worker input thread ended without a result")
                    }
                    Err(TryRecvError::Empty) => (),
                }
            }
            if reply.is_none() {
                match output.try_recv() {
                    Ok(result) => reply = Some(result.context("read bounded worker response")?),
                    Err(TryRecvError::Disconnected) => {
                        bail!("worker output thread ended without a result")
                    }
                    Err(TryRecvError::Empty) => (),
                }
            }
            ensure!(
                Instant::now() < deadline,
                "ephemeral worker exceeded its deadline"
            );
            ensure!(is_current()?, "worker claim is no longer current");
            if let Some(status) = child.0.try_wait()? {
                break status;
            }
            thread::sleep(POLL.min(deadline.saturating_duration_since(Instant::now())));
        };
        ensure!(status.success(), "ephemeral worker exited unsuccessfully");
        // Reap first, then pipe EOF is guaranteed for this no-subprocess worker.
        child.0.wait()?;
        if lifeline.is_none() {
            lifeline = Some(received.recv()?.context("send worker request")?);
        }
        if reply.is_none() {
            reply = Some(output.recv()?.context("read bounded worker response")?);
        }
        drop(lifeline);
        writer
            .join()
            .map_err(|_| anyhow::anyhow!("worker input thread panicked"))?;
        reader
            .join()
            .map_err(|_| anyhow::anyhow!("worker output thread panicked"))?;
        reply.context("worker did not return a response")
    })
}

/// Hidden CLI entry: exactly one request, no Git discovery and no database open.
/// The parent keeps stdin open after the frame as a liveness channel. EOF, extra
/// bytes or a channel error stop the worker, including compilation. A separate
/// watchdog caps orphan lifetime even if the liveness thread cannot run promptly.
pub fn worker_stdio() -> Result<()> {
    thread::Builder::new()
        .name("gitr-worker-watchdog".into())
        .spawn(|| {
            thread::sleep(WORKER_LIFETIME);
            std::process::exit(124);
        })?;
    let input = read_request(&mut io::stdin().lock())?;
    thread::Builder::new()
        .name("gitr-worker-parent".into())
        .spawn(|| {
            let mut byte = [0];
            loop {
                match io::stdin().read(&mut byte) {
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    _ => std::process::exit(125),
                }
            }
        })?;
    let result = if input.mode == 0 {
        prepare(&input.module).map(|_| (0, Vec::new()))
    } else {
        execute(&input.module, input.bytes)
    };
    let bytes = match result {
        Ok((code, output)) => response(
            input.identity,
            input.mode,
            code,
            if code == 0 { &output } else { &[] },
        )?,
        Err(_) => response(input.identity, 2, 0, &[])?,
    };
    let mut stdout = io::stdout().lock();
    stdout.write_all(&bytes)?;
    stdout.flush()?;
    Ok(())
}

#[cfg(test)]
#[path = "actions_runner_tests.rs"]
mod tests;
