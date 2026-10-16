//! The stderr tee of a `stderr_line` capture child (D40).
//!
//! A `stderr_line` harness prints one line, its manifest's `prefix` and then an id, on stderr
//! after its turn: the identity of the one session file the daemon may read (a headless Hermes
//! printed `session_id: <id>`). This module reads that line from the child's real stderr and
//! forwards every byte of it to the participant's terminal unchanged.
//!
//! # What is read, and how strictly
//!
//! Only **stderr** is read for the identity. A line counts when it is complete (ended by `\n`,
//! with no `\r`), at most [`LINE_LIMIT`] bytes, and exactly the rule's prefix and one id of its
//! grammar (`timestamp-hex`: eight digits, `_`, six digits, `_`, six lower-case hexadecimal
//! digits; `token`: 1 to 128 of `[A-Za-z0-9_-]`). Any
//! other line is ignored, and an overlong one is skipped to its newline without being held. The
//! same ID twice is one marker; two different IDs are [`ScopeMarker::Conflicting`], and the
//! conflict is final. A marker on stdout is never read: stdout carries the model's answer, which
//! anything may write.
//!
//! # The wait
//!
//! [`CaptureStderr::marker_until`] waits for the end of stderr or for its deadline, whichever
//! comes first, and then decides **from the lines seen so far**. It does not need an end of
//! stream to answer: a descendant that holds stderr open must not hold up the finish, and a
//! marker already printed is a fact whether or not the stream has closed.
//!
//! # Back-pressure and a closed terminal
//!
//! A destination that cannot take bytes (a closed terminal) turns forwarding into draining: the
//! child is never blocked or failed by this process's own output, and the marker is still read.
//! The identity is read before a chunk is forwarded, so a slow terminal cannot delay it.

use crate::CliError;
use crate::launch::{Spawned, StderrLineRule};
use goalcup_protocol::control::FINISH_COMMIT_RESERVE;
use goalcup_protocol::harness::IdGrammar;
use std::io::{Read, Write};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Instant;

/// The longest stderr line looked at for the marker.
pub const LINE_LIMIT: usize = 4096;

/// What the daemon's native read is left of the finish when the launcher stops waiting for the
/// marker: the daemon's native budget is the rest of its drain share, which ends
/// [`FINISH_COMMIT_RESERVE`] before the deadline, so a wait that ran to the drain share's end
/// would leave the read nothing and a valid identity would still end in a gap.
pub const NATIVE_FINISH_MIN: std::time::Duration = std::time::Duration::from_secs(1);

/// The last instant the launcher waits for the marker, given the one finish `deadline`: the
/// commit reserve and [`NATIVE_FINISH_MIN`] before it. When less than that remains, now: the
/// answer is then decided from the lines already seen.
pub fn marker_wait_end(deadline: Instant) -> Instant {
    deadline
        .checked_sub(FINISH_COMMIT_RESERVE + NATIVE_FINISH_MIN)
        .unwrap_or_else(Instant::now)
}

/// What the stderr stream said about this run's session identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeMarker {
    /// Exactly one distinct identity, printed once or more.
    One(String),
    /// No line in the grammar.
    Absent,
    /// Two different identities. Never resolved by picking one.
    Conflicting,
}

#[derive(Default)]
struct State {
    first: Option<String>,
    conflicting: bool,
    /// Stderr ended: nothing more will be read from it.
    stderr_ended: bool,
    error: Option<(std::io::ErrorKind, String)>,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The running stderr tee of one capture child, and what it has seen.
pub struct CaptureStderr {
    shared: Arc<Shared>,
}

impl CaptureStderr {
    /// Wait for the end of stderr or `until`, then answer from the lines seen so far.
    pub fn marker_until(&self, until: Instant) -> ScopeMarker {
        let mut state = self.shared.lock();
        while !state.stderr_ended {
            let now = Instant::now();
            if now >= until {
                break;
            }
            state = self
                .shared
                .changed
                .wait_timeout(state, until - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        if state.conflicting {
            ScopeMarker::Conflicting
        } else if let Some(id) = &state.first {
            ScopeMarker::One(id.clone())
        } else {
            ScopeMarker::Absent
        }
    }

    /// `true` once stderr was read to its end and forwarded; `false` at `deadline` with it still
    /// open (a descendant may hold it, or the destination's back-pressure may hold the final
    /// bytes). A read that failed is an error, reported the same on every call.
    pub fn finish(&self, deadline: Instant) -> std::io::Result<bool> {
        let mut state = self.shared.lock();
        while !state.stderr_ended {
            let now = Instant::now();
            if now >= deadline {
                return Ok(false);
            }
            state = self
                .shared
                .changed
                .wait_timeout(state, deadline - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        match &state.error {
            Some((kind, what)) => Err(std::io::Error::new(*kind, what.clone())),
            None => Ok(true),
        }
    }
}

/// Tee one stream: forward it to `writer` and read the session marker the manifest's `rule`
/// declares (its prefix and grammar) from it. Public so the grammar can be tested on any byte
/// source.
pub fn tee_marked_with_rule(
    reader: impl Read + Send + 'static,
    writer: impl Write + Send + 'static,
    rule: StderrLineRule,
) -> std::io::Result<CaptureStderr> {
    let shared = Arc::new(Shared {
        state: Mutex::new(State::default()),
        changed: Condvar::new(),
    });
    let tee_shared = Arc::clone(&shared);
    std::thread::Builder::new()
        .name("capture-stderr".into())
        .spawn(move || {
            let result = run(reader, writer, &tee_shared, &rule);
            let mut state = tee_shared.lock();
            state.stderr_ended = true;
            if let Err(e) = result {
                state.error.get_or_insert((e.kind(), e.to_string()));
            }
            drop(state);
            tee_shared.changed.notify_all();
        })?;
    Ok(CaptureStderr { shared })
}

/// Take and forward only stderr while reading its manifest identity rule; stdout remains
/// inherited.
pub fn tee_stderr_capture_with_rule(
    spawned: &mut Spawned,
    rule: StderrLineRule,
) -> Result<CaptureStderr, CliError> {
    // A deliberate `Child::kill` on a child the gate released: a tee that cannot forward ends
    // the registered child it could not forward, and the seal that follows answers for it (the
    // termination sweep matches strings, so this is named here).
    let fail = |spawned: &mut Spawned, error: String| {
        let _ = spawned.child.kill();
        let _ = spawned.child.wait();
        CliError::Refused(format!(
            "the harness output could not be forwarded (stderr: {error}), so the harness was \
             ended."
        ))
    };
    let Some(stderr) = spawned.child.stderr.take() else {
        return Err(fail(spawned, "the harness's stderr was not piped".into()));
    };
    tee_marked_with_rule(stderr, std::io::stderr(), rule)
        .map_err(|error| fail(spawned, error.to_string()))
}

fn run(
    mut reader: impl Read,
    mut writer: impl Write,
    shared: &Shared,
    rule: &StderrLineRule,
) -> std::io::Result<()> {
    let mut line: Vec<u8> = Vec::new();
    let mut overlong = false;
    let mut forwarding = true;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        let chunk = &buf[..n];
        for &byte in chunk {
            if byte == b'\n' {
                if !overlong && let Some(id) = marker_id(rule, &line) {
                    record(shared, id);
                }
                line.clear();
                overlong = false;
            } else if !overlong {
                if line.len() == LINE_LIMIT {
                    line.clear();
                    overlong = true;
                } else {
                    line.push(byte);
                }
            }
        }
        if forwarding && (writer.write_all(chunk).is_err() || writer.flush().is_err()) {
            forwarding = false;
        }
    }
}

fn record(shared: &Shared, id: String) {
    let mut state = shared.lock();
    match &state.first {
        None => state.first = Some(id),
        Some(first) if *first != id => state.conflicting = true,
        Some(_) => {}
    }
    drop(state);
    shared.changed.notify_all();
}

/// The identity on `line` (without its newline), if it is a marker line.
fn marker_id(rule: &StderrLineRule, line: &[u8]) -> Option<String> {
    let id = line.strip_prefix(rule.prefix.as_bytes())?;
    let grammar = match rule.grammar {
        IdGrammar::TimestampHex => {
            id.len() == 22
                && id[..8].iter().all(u8::is_ascii_digit)
                && id[8] == b'_'
                && id[9..15].iter().all(u8::is_ascii_digit)
                && id[15] == b'_'
                && id[16..]
                    .iter()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
        }
        IdGrammar::Token => {
            (1..=128).contains(&id.len())
                && id
                    .iter()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        }
        _ => false,
    };
    grammar.then(|| String::from_utf8_lossy(id).into_owned())
}
