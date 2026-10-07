//! D16's arming gate, as the participant meets it.
//!
//! **Every connector arms on its own transcript**. The daemon arms
//! an entry on the first chained transcript record, so there is no start signal for this CLI
//! to wait for: the launch plus that first record is the start. What is left for the CLI is
//! the one thing the daemon can still say before a run begins, which is that it has settled
//! the gate and will not arm (`control::ArmingState::Refused`: a harness home somebody else's
//! run is holding, for example). [`wait_foreground`] and [`watch_beside`] read that once,
//! off the signed status, and report it.
//!
//! The start-signal long poll (`DaemonClient::wait_start_signal`) is part of the daemon's
//! wire contract; this CLI does not make it.
//!
//! # The Ctrl-C handler is here too
//!
//! [`watch_for_interrupt`] and [`interrupted`] live beside the gate because the gate is where
//! a participant spends the moments a Ctrl-C is most likely to land in: attach mode stands at
//! it with an armed workspace and nothing running. The flag they carry is read by the whole
//! of `goalcup start`'s abandon window and not only by the gate — a run interrupted between
//! `create_workspace` and `register_process` has to leave nothing behind either way.
//!
//! **The window closes at `register_process`, and so does every read of the flag.** Past
//! that the harness owns the terminal and owns the Ctrl-C typed into it: a TUI child shares
//! this terminal, so an ordinary interrupt inside the harness latches this flag, and a gate
//! that still read it afterwards would report a fully armed, recorded run as "interrupted
//! before the run began" and ask the daemon to abandon a stamped entry. [`Watch`] is where
//! the two moments are told apart, and it is the only reader of the flag inside this file.

/// What the gate said, in the three answers it has.
///
/// **`Refused` is not a timeout.** `ArmingState` has a fourth state (`refused`, on the wire)
/// for a gate that will never arm (`goalcup_daemon::arming::RefusalReason`). A participant
/// told to wait longer for a state that cannot change would open a session that runs blind:
/// a session opened before arming is never recorded.
///
/// [`GateOutcome::NotRequired`] is every connector's ordinary case and not an absence of a
/// gate: the launch plus the first transcript record is the start, and the daemon is
/// already watching for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateOutcome {
    /// The daemon has settled that this gate will not arm, in its own words. Exit 4.
    Refused(String),
    /// Nothing to wait for: the first transcript record arms the entry.
    NotRequired,
    /// The gate was asked about while this process was being stopped — and only ever inside
    /// the abandon window, which is the only place [`Watch`] reads the flag. Rung 1: nothing
    /// refused this run, the participant stopped it.
    Interrupted,
}

// What the gate says to a participant (the daemon's settled-refusal reports, the foreground
// and beside-the-harness watchers) is the V1 path's alone (D42) and lives in `arming/gate.rs`,
// compiled with `v1-compat`. `GateOutcome` stays here because a local capture run reports
// `NotRequired` in the same `StartOutcome`.
#[cfg(feature = "v1-compat")]
mod gate;
#[cfg(feature = "v1-compat")]
pub use gate::*;

// ------------------------------------------------------------------ Ctrl-C

/// Set by the `SIGINT` handler and read by everything inside the abandon window.
///
/// A `static` and not a field, because a signal handler has nowhere else to put what it saw:
/// it may not allocate, it may not lock, and it may not call back into anything that does.
/// A relaxed store into an `AtomicBool` is the whole of what is allowed, and the whole of
/// what is needed.
static INTERRUPTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether this process has been asked to stop since [`watch_for_interrupt`] was called.
///
/// The gate reads it, and so does every step of `goalcup start` between `create_workspace`
/// and `register_process` — the window in which a failure leaves no entry behind. A Ctrl-C
/// inside that window has to unmake the entry, because the alternative is a workspace the
/// daemon is watching for a run nobody is doing.
pub fn interrupted() -> bool {
    INTERRUPTED.load(std::sync::atomic::Ordering::SeqCst)
}

/// Install the `SIGINT` handler, once per process.
///
/// **`sigaction` with no `SA_RESTART`, deliberately.** `signal()` on both platforms this
/// tree runs on installs a handler that restarts an interrupted read, which would leave a
/// Ctrl-C invisible until the blocked daemon call returned on its own. Without the flag the
/// blocking read returns `EINTR`, the client reports it, and the next look at [`interrupted`]
/// is the one that matters.
///
/// A crate like `ctrlc` would be the ordinary way to write this; `libc` is already in this
/// crate's graph for `killpg`, which is the other half of what an interrupted launch has to
/// do.
#[cfg(unix)]
pub fn watch_for_interrupt() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // SAFETY: `sigaction` reads the struct handed to it and writes nothing this process
        // owns; the struct is zeroed first so every field this platform has, named here or
        // not, is a defined value. The handler stores into a `static AtomicBool` and does
        // nothing else, which is the whole of what is async-signal-safe.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = on_sigint as *const () as libc::sighandler_t;
            libc::sigemptyset(&mut action.sa_mask);
            action.sa_flags = 0;
            libc::sigaction(libc::SIGINT, &action, std::ptr::null_mut());
        }
    });
}

/// A platform with no `sigaction` has nothing to install; [`interrupted`] simply never
/// answers `true`.
#[cfg(not(unix))]
pub fn watch_for_interrupt() {}

/// The handler itself: one atomic store, and nothing a signal handler may not do.
#[cfg(unix)]
extern "C" fn on_sigint(_signum: libc::c_int) {
    INTERRUPTED.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// Raise `SIGINT` at this process, so a test can meet the handler rather than the flag.
///
/// Behind `test-support` with the rest of this crate's fakes: nothing a participant installs
/// signals itself.
#[cfg(all(unix, feature = "test-support"))]
pub fn raise_sigint_for_tests() {
    // SAFETY: `raise` takes one integer, delivers to the calling thread, and returns before
    // this call does. The handler it reaches is the one above.
    let _ = unsafe { libc::raise(libc::SIGINT) };
}

/// Put the flag back, so what one `goalcup start` was told does not reach the next one.
///
/// The flag is process-global because a signal handler has nowhere else to put what it saw,
/// and a process that runs `start` twice — a test binary, or a later caller of this library
/// — must not have the second one refuse itself over the first one's Ctrl-C.
///
/// **Its callers hold a [`ClearOnDrop`] rather than calling it last.** A trailing call is a
/// promise about every path out of a function, and the
/// paths out of `start_with` and `resume_with` include a panic in a scoped watcher thread
/// and an early `?` a later edit adds without reading to the end. A guard keeps the promise
/// by construction.
pub fn clear_interrupt() {
    INTERRUPTED.store(false, std::sync::atomic::Ordering::SeqCst);
}

/// Clears the interrupt flag when it goes out of scope, whichever way its holder left.
///
/// Built by [`clear_on_drop`] and never by a caller, so the only way to hold one is to have
/// asked for it. It carries nothing: the flag it puts back is the `static` above.
pub struct ClearOnDrop(());

impl Drop for ClearOnDrop {
    fn drop(&mut self) {
        clear_interrupt();
    }
}

/// A guard that calls [`clear_interrupt`] on the way out. See it for why this is a guard.
///
/// `let _clear = arming::clear_on_drop();` — and not `let _ =`, which drops it immediately
/// and clears the flag before the command has read it once.
pub fn clear_on_drop() -> ClearOnDrop {
    ClearOnDrop(())
}

/// Put the flag back, so a test that raised one leaves the process as it found it.
#[cfg(feature = "test-support")]
pub fn clear_interrupt_for_tests() {
    clear_interrupt();
}
