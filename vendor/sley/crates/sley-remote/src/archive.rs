// SPDX-License-Identifier: MIT OR Apache-2.0
//! Streaming archive requests over the existing SSH and git-daemon transports.

use std::io::Write;
use std::process::{Child, Command, ExitStatus};

use sley_config::GitConfig;
use sley_core::{GitError, Result};
use sley_protocol::UploadArchiveRequest;
use sley_transport::{RemoteTransport, RemoteUrl};

use crate::{ProgressSink, RemotePolicy};

/// Archive service commands own a separate process group so a failed peer's
/// descendants cannot retain pipes after only the immediate child is killed.
pub(crate) fn isolate_archive_process(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(not(unix))]
    let _ = command;
}

/// Call before reaping the group leader: its PID remains reserved until wait,
/// so this cannot signal a subsequently reused process group identifier.
pub(crate) fn terminate_archive_process(child: &mut Child) {
    #[cfg(unix)]
    if let Ok(raw) = i32::try_from(child.id())
        && raw > 1
        && let Some(pid) = rustix::process::Pid::from_raw(raw)
    {
        let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
    }
    let _ = child.kill();
}

/// Observe exit without reaping before cleaning the remaining owned group.
/// This preserves the leader's real exit status and prevents PID reuse while
/// descendants are signalled. Linux and macOS provide the required waitid
/// contract; other targets retain direct-child waits pending qualification.
pub(crate) fn wait_archive_process(child: &mut Child) -> std::io::Result<ExitStatus> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use rustix::process::{Pid, Signal, WaitId, WaitIdOptions, kill_process_group, waitid};
        let pid = i32::try_from(child.id())
            .ok()
            .filter(|raw| *raw > 1)
            .and_then(Pid::from_raw)
            .ok_or_else(|| std::io::Error::other("invalid archive child process identifier"))?;
        loop {
            match waitid(
                WaitId::Pid(pid),
                WaitIdOptions::EXITED | WaitIdOptions::NOWAIT,
            ) {
                Ok(Some(_)) => break,
                Ok(None) => continue,
                Err(rustix::io::Errno::INTR) => continue,
                Err(error) => return Err(error.into()),
            }
        }
        // The exited group leader is still waitable here, so its PID cannot
        // belong to an unrelated process, even if no descendants remain.
        let _ = kill_process_group(pid, Signal::KILL);
    }
    child.wait()
}

/// Repository and command policy are supplied by the caller. No local object
/// database is necessary: the server resolves the ref and generates the archive.
pub struct RemoteArchiveRequest<'a> {
    pub remote: &'a RemoteUrl,
    pub arguments: &'a [String],
    pub config: &'a GitConfig,
    pub policy: &'a RemotePolicy,
    /// An explicitly selected remote upload-archive command (`archive --exec`).
    pub service_command: Option<&'a str>,
}

/// Request an archive and copy it to the destination without buffering it whole.
/// SSH authentication remains with the configured SSH executable; git:// uses
/// the shared native socket / configured proxy connector.
pub fn archive_remote<W: Write + ?Sized>(
    request: RemoteArchiveRequest<'_>,
    writer: &mut W,
    progress: &mut dyn ProgressSink,
) -> Result<u64> {
    crate::protocol::check_transport_allowed(
        crate::protocol::transport_scheme_for_remote(request.remote),
        Some(request.config),
        &request.policy.transport,
    )
    .map_err(crate::protocol::transport_policy_git_error)?;
    let arguments = UploadArchiveRequest {
        arguments: request.arguments.to_vec(),
    };
    // Validate the complete bounded control request before opening a transport.
    // The archive itself is streamed and has no whole-payload allocation.
    if arguments.arguments.len() > 64 {
        return Err(GitError::Command(
            "upload-archive accepts at most 64 arguments".into(),
        ));
    }
    let _ = sley_protocol::encode_upload_archive_request(&arguments)?;
    match request.remote.transport {
        RemoteTransport::Ssh | RemoteTransport::Ext => crate::ssh::archive_via_ssh(
            request.remote,
            &arguments,
            request.config,
            request.service_command,
            writer,
            progress,
        ),
        RemoteTransport::Git => {
            if request
                .service_command
                .is_some_and(|command| command != "git-upload-archive")
            {
                return Err(GitError::Unsupported(
                    "git:// does not support a custom upload-archive command".into(),
                ));
            }
            crate::git::archive_via_git(
                request.remote,
                &arguments,
                request.config,
                writer,
                progress,
            )
        }
        RemoteTransport::Local | RemoteTransport::File => {
            let command = request.service_command.ok_or_else(|| {
                GitError::Unsupported("local archive requires an explicit service command".into())
            })?;
            crate::ssh::archive_via_local_command(
                &request.remote.path,
                &arguments,
                command,
                writer,
                progress,
            )
        }
        _ => Err(GitError::Unsupported(
            "remote archive requires SSH, git://, or a local repository".into(),
        )),
    }
}
