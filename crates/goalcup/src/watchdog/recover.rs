//! `goalcup recover` (PRD-02 section 8.1, ledger R3b, point 3: recording goes on when the
//! watchdog died while the harness runs): a new watchdog is put back on the same port with the
//! same run CA, and the harness is never touched.

use super::{
    Boot, CaPem, Cause, KEY_FILE, Prev, StartFacts, ToRecorder, catch_seal, no_core, not_dumpable,
    send_line, spawn, watch, write_pid,
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
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How long the caller waits for the new watchdog's report: its `ready` wait and some.
const REPORT: Duration = Duration::from_nanos(super::READY_NS + 2 * super::SEC);

/// The recorder is back on `port`, under a new watchdog in its own session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recovered {
    pub port: u16,
}

/// Why nothing was started. PRD-10 words each (its section 3.2, ask N9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotRecovered {
    /// The watchdog in `rec/watchdog.pid` lives: it restarts the recorder itself.
    WatchdogAlive,
    /// `status.json` says sealed: nothing is left to record.
    Sealed,
    /// A recorder holds `rec/lock`: recording is on, and it is left alone.
    RecordingOn,
    /// The harness in `facts.harness` is gone: `goalcup continue` or `goalcup upload` then.
    HarnessGone,
    /// `rec/run-ca.key` or `run-ca.pem` is missing, unreadable or does not load.
    KeyMissing,
    /// The port in `status.json` is unknown or another program holds it.
    PortTaken,
    /// The new recorder gave no `ready` in time. Its watchdog goes on: the recorder is not ended,
    /// and the restart loop takes it on.
    NoReady,
}

/// [`recover_with`] with this binary as the recorder.
pub fn recover(entry_dir: &Path, facts: StartFacts) -> Result<Recovered, NotRecovered> {
    let exe = std::env::current_exe().map_err(|_| NotRecovered::NoReady)?;
    recover_with(entry_dir, facts, &exe)
}

/// Section 8.1: the checks and the bind here, then `recorder_exe` started as a new process in its
/// own session (the hidden recorder entry, told by a `recover` first line to be the watchdog; no
/// fork, so any thread of the caller is harmless). That watchdog starts the recorder (`prev`
/// `{"cause":"watchdog_lost"}`), reports whether it gave `ready` in time with one byte, and then
/// watches the harness in the shared loop.
pub fn recover_with(
    entry_dir: &Path,
    mut facts: StartFacts,
    recorder_exe: &Path,
) -> Result<Recovered, NotRecovered> {
    let status: Option<serde_json::Value> = read_json(&status_json(entry_dir));
    if status.as_ref().is_some_and(|s| s["mode"] == "sealed") {
        return Err(NotRecovered::Sealed);
    }
    if cli::watchdog_alive(entry_dir) {
        return Err(NotRecovered::WatchdogAlive);
    }
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
    let port = status.and_then(|s| u16::try_from(s.get("port")?.as_u64()?).ok());
    let port = port.filter(|p| *p != 0).ok_or(NotRecovered::PortTaken)?;
    let listener = spawn::bind_exact(port).map_err(|_| NotRecovered::PortTaken)?;
    drop(probe);
    (facts.ca, facts.entry_dir) = (pem, entry_dir.to_path_buf());
    facts.prev = Some(Prev::Cause(Cause::WatchdogLost));

    let (mut report, theirs) = UnixStream::pair().map_err(|_| NotRecovered::NoReady)?;
    // Never waited on: it outlives the caller, which leaves at once.
    let _watchdog = spawn::recorder(recorder_exe, false, &listener, &theirs)
        .map_err(|_| NotRecovered::NoReady)?;
    drop((theirs, listener));
    if !send_line(&report, &ToRecorder::Recover(Box::new(facts))) {
        return Err(NotRecovered::NoReady);
    }
    let _ = report.set_read_timeout(Some(REPORT));
    let mut byte = [0u8];
    match report.read(&mut byte) {
        Ok(1) if byte[0] == 1 => Ok(Recovered { port }),
        _ => Err(NotRecovered::NoReady),
    }
}

/// The new watchdog, in the process [`recover_with`] started: `listener` on fd 3, `report` on fd
/// 4. Section 8 step 1 for itself, the recorder started and whether its `ready` came in time
/// reported with one byte, then the shared loop. A boot slow to `ready` is not ended: the restart
/// loop and null mode take it from there, as for any boot after a kill. It keeps the recorder
/// entry's `oom_score_adj` 500: under memory pressure it ends before the harness, the recorder
/// records on, and recover can run again.
pub(crate) fn run(facts: StartFacts, listener: TcpListener, mut report: UnixStream) -> ExitCode {
    let dir = facts.entry_dir.clone();
    not_dumpable();
    fds::raise(Target::Harness);
    no_core();
    catch_seal();
    if write_pid(&dir.join("rec")).is_err() {
        line(&dir, "watchdog_pid_failed", &[]);
    }
    let exe = std::env::current_exe().unwrap_or_default();
    let recorder_pid = Arc::new(Mutex::new(0));
    let Ok(mut boot) = Boot::start(&exe, &listener, &facts, &recorder_pid, false) else {
        let _ = report.write_all(&[0]);
        line(&dir, "watchdog_failed", &[("why", Fact::Word("recorder_spawn"))]);
        return ExitCode::from(super::NOT_STARTED);
    };
    let ready = boot.wait_ready(false).is_ok();
    let _ = report.write_all(&[u8::from(ready)]);
    drop(report);
    let late = [("ready", Fact::Word("late"))];
    line(&dir, "watchdog_recovered", if ready { &[] } else { &late });
    let pid = facts.harness.as_ref().map(|h| h.pid);
    let _ = watch(boot, facts, None, (&exe, &listener, &recorder_pid), pid);
    ExitCode::SUCCESS
}
