//! Progress reporting and cancellation across the library/CLI boundary.
//!
//! Library operations never touch the terminal. Long-running work reports
//! progress through a [`ProgressSink`] supplied by the caller and checks a
//! [`Cancel`] signal at loop boundaries. The CLI provides a concrete sink that
//! draws to stderr (gated on `isatty`) and decides colour, redraw rate, and
//! whether a pager is attached; the library makes none of those decisions.
//!
//! Tests, `grit-cli`, and any call site that does not want output use the
//! no-op [`NullProgress`] / [`NeverCancel`].

/// A sink for progress updates emitted by a library operation.
///
/// Every method has a no-op default, so a sink implements only what it needs.
/// The library calls these; it never decides whether output is a tty, what
/// colour to use, or how often to redraw — that is the CLI's responsibility.
pub trait ProgressSink {
    /// Begin a new progress phase labelled `label`, optionally with a known
    /// total count of units (e.g. objects). `None` means the total is unknown.
    fn start(&mut self, label: &str, total: Option<u64>) {
        let _ = (label, total);
    }

    /// Advance the current phase by `units`.
    fn inc(&mut self, units: u64) {
        let _ = units;
    }

    /// Set the absolute progress of the current phase to `current` units.
    fn set(&mut self, current: u64) {
        let _ = current;
    }

    /// Emit an out-of-band human-readable message (e.g. a remote sideband line
    /// or `"Trying merge strategy ..."`).
    fn message(&mut self, msg: &str) {
        let _ = msg;
    }

    /// Finish the current phase.
    fn finish(&mut self) {}
}

/// Forwarding impl so a `&mut P` can be threaded into sub-operations that also
/// take `impl ProgressSink` without re-borrowing gymnastics at every call site.
impl<T: ProgressSink + ?Sized> ProgressSink for &mut T {
    fn start(&mut self, label: &str, total: Option<u64>) {
        (**self).start(label, total);
    }
    fn inc(&mut self, units: u64) {
        (**self).inc(units);
    }
    fn set(&mut self, current: u64) {
        (**self).set(current);
    }
    fn message(&mut self, msg: &str) {
        (**self).message(msg);
    }
    fn finish(&mut self) {
        (**self).finish();
    }
}

/// A [`ProgressSink`] that discards every update. The default for tests,
/// `grit-cli`, and any call site that does not want progress output.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullProgress;

impl ProgressSink for NullProgress {}

/// A cancellation signal checked by long-running library operations at loop
/// boundaries. Implementations must be cheap to query.
pub trait Cancel {
    /// Returns `true` once the operation should stop as soon as possible.
    fn is_cancelled(&self) -> bool;
}

/// A [`Cancel`] that never signals cancellation. The default for call sites
/// that do not support interruption.
#[derive(Debug, Default, Clone, Copy)]
pub struct NeverCancel;

impl Cancel for NeverCancel {
    fn is_cancelled(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sink that records what it was told, to prove the trait + forwarding
    /// impl route calls correctly.
    #[derive(Default)]
    struct Recorder {
        started: Vec<(String, Option<u64>)>,
        incs: u64,
        messages: Vec<String>,
        finished: usize,
    }

    impl ProgressSink for Recorder {
        fn start(&mut self, label: &str, total: Option<u64>) {
            self.started.push((label.to_string(), total));
        }
        fn inc(&mut self, units: u64) {
            self.incs += units;
        }
        fn message(&mut self, msg: &str) {
            self.messages.push(msg.to_string());
        }
        fn finish(&mut self) {
            self.finished += 1;
        }
    }

    fn drive(sink: &mut impl ProgressSink) {
        sink.start("objects", Some(3));
        sink.inc(1);
        sink.inc(2);
        sink.message("done");
        sink.finish();
    }

    #[test]
    fn null_progress_is_inert() {
        // Must compile and run without panicking; nothing to assert.
        drive(&mut NullProgress);
    }

    #[test]
    fn forwarding_impl_routes_to_inner() {
        let mut rec = Recorder::default();
        // Pass `&mut Recorder` where `impl ProgressSink` is expected, exercising
        // the `&mut T` forwarding impl.
        drive(&mut &mut rec);
        assert_eq!(rec.started, vec![("objects".to_string(), Some(3))]);
        assert_eq!(rec.incs, 3);
        assert_eq!(rec.messages, vec!["done".to_string()]);
        assert_eq!(rec.finished, 1);
    }

    #[test]
    fn never_cancel_never_cancels() {
        assert!(!NeverCancel.is_cancelled());
    }
}
