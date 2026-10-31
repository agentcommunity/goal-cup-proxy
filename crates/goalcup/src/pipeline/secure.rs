//! What a recorder boot does before it reads anything (PRD-02 section 9.2 step 1): no core dump
//! can hold a recorded byte or the CA key, the kernel kills the recorder before the harness under
//! memory pressure, and a panic never prints its message.

use crate::cli::log::{self, Fact};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// The unused `prctl` arguments, as the `unsigned long` the kernel reads.
#[cfg(target_os = "linux")]
const Z: libc::c_ulong = 0;

/// Core dumps off (`RLIMIT_CORE` 0; on Linux also `PR_SET_DUMPABLE` 0) and, on Linux,
/// `oom_score_adj` 500. Every failure is ignored: recording goes on without it.
pub fn harden() {
    let zero = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `zero` is a valid rlimit for the call.
    let _ = unsafe { libc::setrlimit(libc::RLIMIT_CORE, &zero) };
    #[cfg(target_os = "linux")]
    {
        // Before PR_SET_DUMPABLE: a process that is not dumpable no longer owns its /proc files.
        let _ = std::fs::write("/proc/self/oom_score_adj", "500");
        // SAFETY: PR_SET_DUMPABLE reads its integer arguments and changes no memory.
        let _ = unsafe { libc::prctl(libc::PR_SET_DUMPABLE, Z, Z, Z, Z) };
    }
}

/// Where [`exit`] and the panic hook write their `goalcup.log` line, once [`panic_hook`] set it.
static LOG_DIR: OnceLock<PathBuf> = OnceLock::new();

/// A panic hook that writes a fixed text and the panic's file and line to stderr and to
/// `goalcup.log` in `entry_dir` (seam S11), never the payload: an `expect` or a `panic!` could
/// format a recorded value or a key. It also sets where [`exit`] logs.
pub fn panic_hook(entry_dir: &Path) {
    let _ = LOG_DIR.set(entry_dir.to_path_buf());
    std::panic::set_hook(Box::new(|info| {
        let (file, line) = info.location().map_or(("", 0), |at| (at.file(), at.line()));
        let text = format!("goalcup: internal error at {file}:{line}\n");
        let _ = std::io::stderr().lock().write_all(text.as_bytes());
        if let Some(dir) = LOG_DIR.get() {
            let facts = [
                ("file", Fact::Path(file.into())),
                ("line", Fact::N(line.into())),
            ];
            log::line(dir, "internal_error", &facts);
        }
    }));
}

/// Ends the recorder with `code` after one fixed line, naming `cause`, to stderr and
/// `goalcup.log`, so the watchdog restarts it and the next boot's `recorder_down` gap names the
/// span. The first caller exits; a later one returns, since the process is already ending.
pub(crate) fn exit(code: i32, cause: &'static str) {
    static EXITING: AtomicBool = AtomicBool::new(false);
    if EXITING.swap(true, Ordering::SeqCst) {
        return;
    }
    let text = format!("goalcup recorder: {cause}; exiting to restart\n");
    let _ = std::io::stderr().lock().write_all(text.as_bytes());
    if let Some(dir) = LOG_DIR.get() {
        let facts = [("cause", Fact::Word(cause)), ("code", Fact::N(code as u64))];
        log::line(dir, "recorder_exit", &facts);
    }
    std::process::exit(code);
}
