//! Where `goalcup start` says things: the terminal, or `<run_dir>/host.log` while a chat-mode
//! harness owns the terminal (D57).
//!
//! A chat-mode harness shares the participant's terminal with the host. A line the host writes
//! there while the harness draws lands in the middle of its screen, so between the launch line
//! and the harness's exit the host writes **nothing** to the terminal. This module is the one
//! switch that makes that true: every message of a run goes through it, the stdout report
//! through [`Routed`] and every stderr line through [`say`], whichever thread says it (the
//! uploader, the disk watch, the seal's retries). While [`Quiet`] holds, each goes to
//! `host.log` instead (append, mode 0600, beside `state.json`); while a log is open, what is
//! printed is also appended there, so the log holds the whole run.
//!
//! When the harness exits, the host speaks again only if the terminal is still there: after a
//! hangup ([`goalcup_platform::terminal::hangup_seen`]) it stays quiet and the rest goes to the
//! log. A write to a terminal that is gone fails with `EIO` (or `EPIPE`), and while a log is
//! open such a failure is swallowed: it must not end the host, which has a seal and an upload
//! to finish. A headless run never opens a log, and its output is exactly what it was.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The log's name in the run's own directory (`<cfg>/runs/<run_id>/`).
pub const HOST_LOG_NAME: &str = "host.log";

struct Sink {
    log: Option<std::fs::File>,
    /// Nothing goes to the terminal.
    quiet: bool,
}

/// Process-global: the uploader and the disk watch speak from threads of their own.
static SINK: Mutex<Sink> = Mutex::new(Sink {
    log: None,
    quiet: false,
});

fn sink() -> std::sync::MutexGuard<'static, Sink> {
    SINK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The panic hook a [`Quiet`] replaced, which it chains to and puts back.
type Hook = std::sync::Arc<Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Sync + Send + 'static>>;

/// The host is quiet while this is held: see the module header. Dropping it closes the log,
/// puts the previous panic hook back and lets the host speak again.
pub struct Quiet {
    path: PathBuf,
    previous_hook: Hook,
}

impl std::fmt::Debug for Quiet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Quiet").field("path", &self.path).finish()
    }
}

/// Go quiet, and open `<run_dir>/host.log` for what would have been said. A log that cannot
/// be opened leaves the host quiet all the same: the terminal belongs to the harness, and the
/// run's state file still says how it is.
pub fn quiet(run_dir: &Path) -> Quiet {
    let path = run_dir.join(HOST_LOG_NAME);
    let log = open_log(&path).ok();
    {
        let mut sink = sink();
        sink.log = log;
        sink.quiet = true;
    }
    // A panic on any thread (the uploader's, the disk watch's) is a message like any other:
    // to the log while the host is quiet, and through the hook it replaced otherwise.
    let previous_hook: Hook = std::sync::Arc::new(std::panic::take_hook());
    let chained = std::sync::Arc::clone(&previous_hook);
    std::panic::set_hook(Box::new(move |info| {
        if !panic_to_log(info) {
            chained(info);
        }
    }));
    Quiet {
        path,
        previous_hook,
    }
}

/// Write a panic to the log if the host is quiet, and answer whether it was handled. The sink is
/// tried for a moment rather than waited on: a panic while the sink is held on this very thread
/// must not deadlock, and a quiet host never prints the panic instead.
fn panic_to_log(info: &std::panic::PanicHookInfo<'_>) -> bool {
    for _ in 0..100 {
        let mut sink = match SINK.try_lock() {
            Ok(sink) => sink,
            Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                std::thread::sleep(std::time::Duration::from_millis(1));
                continue;
            }
        };
        if !sink.quiet {
            return false;
        }
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("<unnamed>");
        if let Some(log) = sink.log.as_mut() {
            let _ = writeln!(log, "goalcup: thread '{name}' panicked: {info}");
        }
        return true;
    }
    // The sink stayed held: say nothing rather than write on the harness's terminal.
    true
}

fn open_log(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.append(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path)
}

impl Quiet {
    /// The harness has exited: speak again if the terminal is still there, and stay quiet if
    /// it hung up. Answers whether the host speaks.
    pub fn harness_exited(&self) -> bool {
        let there = !goalcup_platform::terminal::hangup_seen();
        sink().quiet = !there;
        there
    }

    /// Where the log is.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Quiet {
    fn drop(&mut self) {
        {
            let mut sink = sink();
            sink.log = None;
            sink.quiet = false;
        }
        // Ours goes, and the one it replaced is back (wrapped once: a hook is not `Clone`). Not
        // while this thread unwinds: the hook cannot be changed then, and trying would abort.
        if std::thread::panicking() {
            return;
        }
        let _ = std::panic::take_hook();
        let previous = std::sync::Arc::clone(&self.previous_hook);
        std::panic::set_hook(Box::new(move |info| previous(info)));
    }
}

/// One `goalcup: <line>` on stderr, or in the log while the host is quiet. Never fails and
/// never panics: there is nothing to tell about a line that could not be told.
pub fn say(line: &str) {
    say_raw(&format!("goalcup: {line}"));
}

/// [`say`] for a line that already names who says it.
pub fn say_raw(line: &str) {
    let mut sink = sink();
    if let Some(log) = sink.log.as_mut() {
        let _ = writeln!(log, "{line}");
    }
    if !sink.quiet {
        let _ = writeln!(std::io::stderr(), "{line}");
    }
}

/// A command's stdout report, routed through the same switch: written to `inner` unless the
/// host is quiet, and also appended to the log while one is open. While a log is open a failed
/// write to `inner` (the terminal closed under it) is swallowed.
pub struct Routed<'a> {
    inner: &'a mut dyn Write,
}

impl<'a> Routed<'a> {
    pub fn new(inner: &'a mut dyn Write) -> Routed<'a> {
        Routed { inner }
    }
}

impl Write for Routed<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let (quiet, logging) = {
            let mut sink = sink();
            let logging = match sink.log.as_mut() {
                Some(log) => {
                    let _ = log.write_all(buf);
                    true
                }
                None => false,
            };
            (sink.quiet, logging)
        };
        if quiet {
            return Ok(buf.len());
        }
        // Whole, so the log (which took `buf` whole) and the terminal never disagree.
        match self.inner.write_all(buf) {
            Ok(()) => Ok(buf.len()),
            Err(_) if logging => Ok(buf.len()),
            Err(e) => Err(e),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let logging = sink().log.is_some();
        match self.inner.flush() {
            Err(_) if logging => Ok(()),
            other => other,
        }
    }
}
