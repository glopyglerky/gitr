//! Unix-only process helpers (FFI).

/// Returns whether process `pid` exists (same semantics as `kill(pid, 0)`).
///
/// On success of `kill`, the process exists (or we lack permission; treated as alive).
#[must_use]
pub fn pid_is_alive(pid: u32) -> bool {
    // `kill(pid, None)` performs the existence check without sending a signal,
    // exactly like `kill(pid, 0)`; `Ok` mirrors the C call returning 0.
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), None).is_ok()
}
