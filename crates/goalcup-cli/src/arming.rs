//! D16's arming gate, as the participant meets it.
//!
//! **Every connector arms on its own transcript**. The daemon arms
//! an entry on the first chained transcript record, so there is no start signal for this CLI
//! to wait for: the launch plus that first record is the start. The V1 gate
//! reads (`wait_foreground` and `watch_beside`), which read a settled refusal off the
//! signed status, were removed by D48, and the start-signal long poll with them.
//!
//! # The Ctrl-C handler is here too
//!
//! [`watch_for_interrupt`] and [`interrupted`] live beside the gate because the V1 gate was
//! where a participant spent the moments a Ctrl-C was most likely to land in. The flag they
//! carry is read by the whole of `goalcup start`'s abandon window and not only by a gate:
//! a run interrupted between `create_capture_workspace` and `register_process` has to leave
//! nothing behind either way.
//!
//! **The window closes when the harness is released, or when a Ctrl-C has been acted on**
//! ([`close_window`]). Past the release the harness owns the terminal and owns the Ctrl-C
//! typed into it: the child shares this terminal, so an ordinary interrupt meant for the
//! harness reaches this process too. Once the window is closed the flag reads false and the
//! handler records nothing, so the busy backoffs in `retry.rs` (the flag's other readers) run
//! their full schedule for the launch record, the seal and the abandon that a Ctrl-C itself
//! started.

// ------------------------------------------------------------------ Ctrl-C

/// The abandon window: [`OPEN`], [`INTERRUPTED`] by the `SIGINT` handler, or [`CLOSED`].
///
/// A `static` and not a field, because a signal handler has nowhere else to put what it saw:
/// it may not allocate, it may not lock, and it may not call back into anything that does.
/// One compare-and-swap on a lock-free atomic is the whole of what is allowed, and the whole
/// of what is needed. One state and not two flags, so a handler running on another thread can
/// never latch an interrupt after [`close_window`] closed the window.
static WINDOW: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(OPEN);
const OPEN: u8 = 0;
const INTERRUPTED: u8 = 1;
const CLOSED: u8 = 2;

/// Whether this process has been asked to stop since [`watch_for_interrupt`] was called.
///
/// The gate reads it, and so does every step of `goalcup start` between
/// `create_capture_workspace` and `register_process` — the window in which a failure leaves no
/// entry behind. A Ctrl-C inside that window has to unmake the entry, because the
/// alternative is a workspace the daemon is watching for a run nobody is doing.
pub(crate) fn interrupted() -> bool {
    WINDOW.load(std::sync::atomic::Ordering::SeqCst) == INTERRUPTED
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
pub(crate) fn watch_for_interrupt() {
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

/// One line typed on standard input, read so that a Ctrl-C ends the wait at once: the handler
/// above sets no `SA_RESTART`, but `std`'s own read retries on `EINTR`, so this polls the
/// descriptor and looks at [`interrupted`] between polls. `None` when the input ended, it could
/// not be read, or a Ctrl-C came (the caller asks [`interrupted`] which). A line is at most
/// 4 KiB.
#[cfg(unix)]
pub(crate) fn read_stdin_line() -> Option<String> {
    let mut line = Vec::new();
    loop {
        if interrupted() {
            return None;
        }
        let mut poll = libc::pollfd {
            fd: libc::STDIN_FILENO,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one `pollfd` this function owns, for its whole length.
        if unsafe { libc::poll(&mut poll, 1, 100) } <= 0 {
            // A timeout, or `EINTR` from the Ctrl-C: look at the flag and poll again.
            continue;
        }
        let mut byte = [0u8; 1];
        // SAFETY: reads at most one byte into a buffer of one byte this function owns.
        let read = unsafe { libc::read(libc::STDIN_FILENO, byte.as_mut_ptr().cast(), 1) };
        match read {
            1 => {
                line.push(byte[0]);
                if byte[0] == b'\n' || line.len() >= 4096 {
                    break;
                }
            }
            0 if line.is_empty() => return None,
            0 => break,
            _ if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted => {}
            _ => return None,
        }
    }
    Some(String::from_utf8_lossy(&line).into_owned())
}

/// The handler itself: one atomic store, and nothing a signal handler may not do.
#[cfg(unix)]
extern "C" fn on_sigint(_signum: libc::c_int) {
    let _ = WINDOW.compare_exchange(
        OPEN,
        INTERRUPTED,
        std::sync::atomic::Ordering::SeqCst,
        std::sync::atomic::Ordering::SeqCst,
    );
}

/// Close the abandon window: the harness was released and owns every later Ctrl-C, or a
/// Ctrl-C was acted on and the cleanup it started must not be cut short by it. From here
/// [`interrupted`] is false until [`clear_interrupt`] reopens the window for the next start.
pub(crate) fn close_window() {
    WINDOW.store(CLOSED, std::sync::atomic::Ordering::SeqCst);
}

/// What the `SIGINT` handler does, for a test that cannot send a real signal safely.
#[cfg(test)]
pub(crate) fn interrupt_for_test() {
    on_sigint(libc::SIGINT);
}

/// Put the flag back, so what one `goalcup start` was told does not reach the next one.
///
/// The flag is process-global because a signal handler has nowhere else to put what it saw,
/// and a process that runs `start` twice — a test binary, or a later caller of this library
/// — must not have the second one refuse itself over the first one's Ctrl-C.
///
/// **Its callers hold a [`ClearOnDrop`] rather than calling it last.** A trailing call is a
/// promise about every path out of a function, and the paths out of `start_with` include a
/// panic and an early `?` a later edit adds without reading to the end. A guard keeps the
/// promise by construction.
pub(crate) fn clear_interrupt() {
    WINDOW.store(OPEN, std::sync::atomic::Ordering::SeqCst);
}

/// Clears the interrupt flag when it goes out of scope, whichever way its holder left.
///
/// Built by [`clear_on_drop`] and never by a caller, so the only way to hold one is to have
/// asked for it. It carries nothing: the flag it puts back is the `static` above.
pub(crate) struct ClearOnDrop(());

impl Drop for ClearOnDrop {
    fn drop(&mut self) {
        clear_interrupt();
    }
}

/// A guard that calls [`clear_interrupt`] on the way out. See it for why this is a guard.
///
/// `let _clear = arming::clear_on_drop();` — and not `let _ =`, which drops it immediately
/// and clears the flag before the command has read it once.
pub(crate) fn clear_on_drop() -> ClearOnDrop {
    ClearOnDrop(())
}
