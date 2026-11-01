//! `goalcup recover` (PRD-02 section 8.1, ledger R3b): the watchdog died while the harness runs, so
//! a new one is put back on the same port with the same run CA, and the harness is never touched.

use super::{
    Boot, CaPem, Cause, KEY_FILE, Prev, StartFacts, catch_seal, no_core, not_dumpable, spawn,
    watch, write_pid,
};
use crate::cli::log::{Fact, line};
use crate::cli::{self, read_json, status_json};
use crate::pipeline::fds::{self, Target};
use crate::proxy::ca::{RUN_CA_FILE, RunCa};
use std::fs::{self, TryLockError};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How long the caller waits for the detached watchdog's report: its 10 s `ready` wait and some.
const REPORT: Duration = Duration::from_secs(12);

/// The recorder is back on `port`, under a detached watchdog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recovered {
    pub port: u16,
}

/// Why nothing was started. PRD-10 words each (its section 3.2, ask N9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotRecovered {
    /// A recorder holds `rec/lock`: recording is on, and it is left alone.
    RecordingOn,
    /// The harness in `facts.harness` is gone: `goalcup continue` or `goalcup upload` then.
    HarnessGone,
    /// `rec/run-ca.key` or `run-ca.pem` is missing, unreadable or does not load.
    KeyMissing,
    /// The port in `status.json` is unknown or another program holds it.
    PortTaken,
    /// The new recorder gave no `ready` in time.
    NoReady,
}

/// [`recover_with`] with this binary as the recorder.
///
/// The caller must start no thread before calling it: it `fork`s, and the child goes on as the
/// watchdog.
pub fn recover(entry_dir: &Path, facts: StartFacts) -> Result<Recovered, NotRecovered> {
    let exe = std::env::current_exe().map_err(|_| NotRecovered::NoReady)?;
    recover_with(entry_dir, facts, &exe)
}

/// Section 8.1: the checks and the bind here, then a forked watchdog in its own session starts
/// the recorder (`prev` `{"cause":"watchdog_lost"}`), reports `ready` on a pipe and runs the
/// shared loop hands off the harness. Same rule as [`recover`]: no thread before the call.
pub fn recover_with(
    entry_dir: &Path,
    mut facts: StartFacts,
    recorder_exe: &Path,
) -> Result<Recovered, NotRecovered> {
    let rec = entry_dir.join("rec");
    // A probe only: held through the checks, released before anything starts.
    let probe = fs::File::open(rec.join("lock")).ok();
    if let Some(Err(TryLockError::WouldBlock)) = probe.as_ref().map(fs::File::try_lock) {
        return Err(NotRecovered::RecordingOn);
    }
    let harness = facts.harness.as_ref();
    if !harness.is_some_and(|h| cli::alive(h.pid, h.start_id)) {
        return Err(NotRecovered::HarnessGone);
    }
    let pem = CaPem {
        cert: fs::read_to_string(entry_dir.join(RUN_CA_FILE)).unwrap_or_default(),
        key: fs::read_to_string(rec.join(KEY_FILE)).unwrap_or_default(),
    };
    if pem.key.is_empty() || RunCa::import(&pem).is_err() {
        return Err(NotRecovered::KeyMissing);
    }
    let status: Option<serde_json::Value> = read_json(&status_json(entry_dir));
    let port = status.and_then(|s| u16::try_from(s.get("port")?.as_u64()?).ok());
    let port = port.filter(|p| *p != 0).ok_or(NotRecovered::PortTaken)?;
    let listener = spawn::bind_exact(port).map_err(|_| NotRecovered::PortTaken)?;
    drop(probe);
    (facts.ca, facts.entry_dir) = (pem, entry_dir.to_path_buf());
    facts.prev = Some(Prev::Cause(Cause::WatchdogLost));

    let (mut report, tell) = UnixStream::pair().map_err(|_| NotRecovered::NoReady)?;
    // SAFETY: the caller started no thread (documented above), so the child holds no lock
    // another thread took and may go on as an ordinary process.
    match unsafe { libc::fork() } {
        -1 => Err(NotRecovered::NoReady),
        0 => {
            drop(report);
            detached(facts, &listener, recorder_exe, tell);
            // SAFETY: ends the child here, never running the caller's code after `recover`.
            unsafe { libc::_exit(0) }
        }
        _ => {
            drop((tell, listener));
            let _ = report.set_read_timeout(Some(REPORT));
            let mut byte = [0u8];
            match report.read(&mut byte) {
                Ok(1) if byte[0] == 1 => Ok(Recovered { port }),
                _ => Err(NotRecovered::NoReady),
            }
        }
    }
}

/// The forked watchdog: section 8 step 1 for itself, the recorder started and its `ready`
/// reported with one byte, then the shared loop hands off.
fn detached(facts: StartFacts, listener: &TcpListener, exe: &Path, mut tell: UnixStream) {
    let dir = facts.entry_dir.clone();
    // SAFETY: integers and a constant path only.
    unsafe {
        libc::setsid();
        let null = libc::open(c"/dev/null".as_ptr(), libc::O_RDWR);
        for fd in 0..3 {
            libc::dup2(null, fd);
        }
        if null > 2 {
            libc::close(null);
        }
    }
    not_dumpable();
    fds::raise(Target::Harness);
    no_core();
    catch_seal();
    if write_pid(&dir.join("rec")).is_err() {
        line(&dir, "watchdog_pid_failed", &[]);
    }
    let recorder_pid = Arc::new(Mutex::new(0));
    let boot = Boot::start(exe, listener, &facts, &recorder_pid, false);
    let ready = boot.ok().and_then(|mut b| b.wait_ready().is_ok().then_some(b));
    let _ = tell.write_all(&[u8::from(ready.is_some())]);
    drop(tell);
    let Some(boot) = ready else {
        line(&dir, "watchdog_failed", &[("why", Fact::Word("no_ready"))]);
        return;
    };
    line(&dir, "watchdog_recovered", &[]);
    let pid = facts.harness.as_ref().map(|h| h.pid);
    let _ = watch(boot, facts, None, (exe, listener, &recorder_pid), pid);
}
