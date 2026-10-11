//! The stderr tee of a Hermes capture child (D40, D41).
//!
//! Hermes prints one line, `session_id: <id>`, on stderr after its turn: the identity of the one
//! session file the daemon may read (`docs/qualification/m2-hermes-readiness.md`, "Session
//! identity carrier"). This module reads that line from the child's real stderr, forwards every
//! byte of both streams to the participant's terminal unchanged, and scans both streams for
//! fragments of the provider key as they pass.
//!
//! # What is read, and how strictly
//!
//! Only **stderr** is read for the identity. A line counts when it is complete (ended by `\n`,
//! with no `\r`), at most [`LINE_LIMIT`] bytes, and exactly
//! `session_id: ` + eight digits, `_`, six digits, `_`, six lower-case hexadecimal digits. Any
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
//! child is never blocked or failed by this process's own output, and the marker and the key
//! scan still run. The identity is read before a chunk is forwarded, so a slow terminal cannot
//! delay it.

use crate::CliError;
use crate::key_scan::FragmentMatcher;
use crate::launch::{
    ChildSecret, IdentityReader, Spawned, StderrLineRule, StdoutIdentityScan, StdoutJsonRule,
};
use goalcup_protocol::control::FINISH_COMMIT_RESERVE;
use goalcup_protocol::harness::IdGrammar;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Instant;

/// The one scope message of a stdout JSON identity read by the key-scanning tee: the bound id,
/// or `None`. Not a waitable tee: [`CaptureStderr::finish`] reports the stream's end.
pub type ScopeReceiver = std::sync::mpsc::Receiver<Option<String>>;

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
    /// Tees still running (stderr's, and stdout's for a capture child).
    open: usize,
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

/// The running tees of one capture child, and what they have seen.
pub struct CaptureStderr {
    shared: Arc<Shared>,
    hit: Arc<AtomicBool>,
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

    /// `true` once every stream was read to its end and forwarded; `false` at `deadline` with a
    /// stream still open (a descendant may hold it, or the destination's back-pressure may hold
    /// the final bytes). A read that failed is an error, reported the same on every call.
    pub fn finish(&self, deadline: Instant) -> std::io::Result<bool> {
        let mut state = self.shared.lock();
        while state.open > 0 {
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

    /// Whether any byte seen on either stream completed a fragment of the key. Live, and never
    /// reset: a fragment that was seen is a fact about the run.
    pub fn key_hit(&self) -> bool {
        self.hit.load(Ordering::Acquire)
    }
}

/// Tee one stream: forward it to `writer`, scan it with `matcher`, and read the session marker
/// from it. This is the stderr half of [`tee_marked_capture`], public so the grammar can be
/// tested on any byte source.
pub fn tee_marked(
    reader: impl Read + Send + 'static,
    writer: impl Write + Send + 'static,
    matcher: Option<FragmentMatcher>,
) -> std::io::Result<CaptureStderr> {
    tee_marked_with_rule(
        reader,
        writer,
        matcher,
        StderrLineRule {
            prefix: "session_id: ".into(),
            grammar: IdGrammar::TimestampHex,
        },
    )
}

/// [`tee_marked`] using the identity prefix and grammar declared by the harness manifest.
pub fn tee_marked_with_rule(
    reader: impl Read + Send + 'static,
    writer: impl Write + Send + 'static,
    matcher: Option<FragmentMatcher>,
    rule: StderrLineRule,
) -> std::io::Result<CaptureStderr> {
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            open: 1,
            ..State::default()
        }),
        changed: Condvar::new(),
    });
    let hit = Arc::new(AtomicBool::new(false));
    spawn_tee(
        "capture-stderr",
        reader,
        writer,
        matcher,
        Arc::clone(&shared),
        Arc::clone(&hit),
        Some(rule),
    )?;
    Ok(CaptureStderr { shared, hit })
}

/// Take both pipes off a released Hermes child and tee them: stdout to this process's stdout and
/// stderr to its stderr, byte for byte, each scanned for fragments of `secret` (a matcher of its
/// own per stream), stderr also read for the session marker.
///
/// Started on a child the gate has already released (D40), never a spawner. A tee that cannot
/// start ends the child it could not forward, which is registered, so the seal answers for it.
pub fn tee_marked_capture(
    spawned: &mut Spawned,
    secret: Option<&ChildSecret>,
) -> Result<CaptureStderr, CliError> {
    tee_marked_capture_with_rule(
        spawned,
        secret,
        Some(StderrLineRule {
            prefix: "session_id: ".into(),
            grammar: IdGrammar::TimestampHex,
        }),
    )
}

/// Tee and key-scan both piped streams, optionally reading a manifest identity rule on stderr.
pub fn tee_marked_capture_with_rule(
    spawned: &mut Spawned,
    secret: Option<&ChildSecret>,
    rule: Option<StderrLineRule>,
) -> Result<CaptureStderr, CliError> {
    tee_marked_capture_to_with_rule(
        spawned,
        secret,
        std::io::stdout(),
        std::io::stderr(),
        None,
        rule,
    )
    .map(|(_, tee)| tee)
}

/// Tee and key-scan both piped streams, and read the manifest's stdout JSON identity from the
/// same stdout reader the key scan reads (a keyed `stdout_json` profile). Without a stdout
/// rule this is [`tee_marked_capture_with_rule`] and the first value is `None`. The first value
/// is the bare scope receiver, not a [`crate::launch::CaptureStdout`]: there is no separate stdout tee to wait
/// on. [`CaptureStderr::finish`] reports both streams' end, and [`CaptureStderr::key_hit`]
/// covers both.
pub fn tee_marked_capture_with_identity(
    spawned: &mut Spawned,
    secret: Option<&ChildSecret>,
    stdout_rule: Option<StdoutJsonRule>,
    stderr_rule: Option<StderrLineRule>,
) -> Result<(Option<ScopeReceiver>, CaptureStderr), CliError> {
    tee_marked_capture_to_with_rule(
        spawned,
        secret,
        std::io::stdout(),
        std::io::stderr(),
        stdout_rule,
        stderr_rule,
    )
}

/// [`tee_marked_capture_with_identity`] with the two destinations named, for tests.
pub fn tee_marked_capture_with_identity_to(
    spawned: &mut Spawned,
    secret: Option<&ChildSecret>,
    stdout_rule: Option<StdoutJsonRule>,
    stderr_rule: Option<StderrLineRule>,
    stdout_to: impl Write + Send + 'static,
    stderr_to: impl Write + Send + 'static,
) -> Result<(Option<ScopeReceiver>, CaptureStderr), CliError> {
    tee_marked_capture_to_with_rule(
        spawned,
        secret,
        stdout_to,
        stderr_to,
        stdout_rule,
        stderr_rule,
    )
}

/// Take and forward only stderr while reading its manifest identity rule. This is the valid
/// schema-1 `stderr_line` case with no provider credential, so stdout remains inherited.
pub fn tee_stderr_capture_with_rule(
    spawned: &mut Spawned,
    rule: StderrLineRule,
) -> Result<CaptureStderr, CliError> {
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
    tee_marked_with_rule(stderr, std::io::stderr(), None, rule)
        .map_err(|error| fail(spawned, error.to_string()))
}

/// [`tee_marked_capture`] with the two destinations named, so a test can read what was forwarded.
pub fn tee_marked_capture_to(
    spawned: &mut Spawned,
    secret: Option<&ChildSecret>,
    stdout_to: impl Write + Send + 'static,
    stderr_to: impl Write + Send + 'static,
) -> Result<CaptureStderr, CliError> {
    tee_marked_capture_to_with_rule(
        spawned,
        secret,
        stdout_to,
        stderr_to,
        None,
        Some(StderrLineRule {
            prefix: "session_id: ".into(),
            grammar: IdGrammar::TimestampHex,
        }),
    )
    .map(|(_, tee)| tee)
}

fn tee_marked_capture_to_with_rule(
    spawned: &mut Spawned,
    secret: Option<&ChildSecret>,
    stdout_to: impl Write + Send + 'static,
    stderr_to: impl Write + Send + 'static,
    stdout_rule: Option<StdoutJsonRule>,
    rule: Option<StderrLineRule>,
) -> Result<(Option<ScopeReceiver>, CaptureStderr), CliError> {
    // A second, deliberate `Child::kill` on a child the gate released, beside `tee_scoped_capture`'s:
    // a tee that cannot forward ends the registered child it could not forward, and the seal that
    // follows answers for it (the termination sweep matches strings, so this is named here).
    let fail = |spawned: &mut Spawned, what: &str, error: String| {
        let _ = spawned.child.kill();
        let _ = spawned.child.wait();
        CliError::Refused(format!(
            "the harness output could not be forwarded ({what}: {error}), so the harness was \
             ended."
        ))
    };
    // Both pipes come with the `MarkedAndScanned` choice; a child spawned without them is ended
    // here rather than left running with output nothing reads.
    let (Some(stdout), Some(stderr)) = (spawned.child.stdout.take(), spawned.child.stderr.take())
    else {
        return Err(fail(
            spawned,
            "pipes",
            "the harness's stdout and stderr were not both piped".into(),
        ));
    };
    // One matcher per reader: stdout's tee, stderr's tee, and, only with a stdout identity
    // rule, the identity matcher, which must not bind an id holding a fragment of the key.
    let make = |secret: Option<&ChildSecret>| {
        secret
            .map(|secret| FragmentMatcher::new(secret.expose()))
            .transpose()
    };
    let matchers = match (
        make(secret),
        make(secret),
        make(stdout_rule.as_ref().and(secret)),
    ) {
        (Ok(a), Ok(b), Ok(c)) => (a, b, c),
        (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => {
            return Err(fail(spawned, "key scan", e.to_string()));
        }
    };
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            open: 2,
            ..State::default()
        }),
        changed: Condvar::new(),
    });
    let hit = Arc::new(AtomicBool::new(false));
    // One reader of stdout: with a stdout identity rule, what it reads is fed to the identity
    // matcher first, then scanned for key fragments and forwarded by the same tee.
    let (stdout, scope): (Box<dyn Read + Send>, _) = match stdout_rule {
        Some(rule) => {
            let (sender, scope) = std::sync::mpsc::channel();
            let mut scan = StdoutIdentityScan::new(rule, sender);
            if let Some(matcher) = matchers.2 {
                scan = scan.with_key_scan(matcher, Arc::clone(&hit));
            }
            (Box::new(IdentityReader::new(stdout, scan)), Some(scope))
        }
        None => (Box::new(stdout), None),
    };
    if let Err(e) = spawn_tee(
        "capture-stdout",
        stdout,
        stdout_to,
        matchers.0,
        Arc::clone(&shared),
        Arc::clone(&hit),
        None,
    ) {
        return Err(fail(spawned, "stdout", e.to_string()));
    }
    if let Err(e) = spawn_tee(
        "capture-stderr",
        stderr,
        stderr_to,
        matchers.1,
        Arc::clone(&shared),
        Arc::clone(&hit),
        rule,
    ) {
        // The stdout tee ends with the child.
        return Err(fail(spawned, "stderr", e.to_string()));
    }
    Ok((scope, CaptureStderr { shared, hit }))
}

fn spawn_tee(
    name: &str,
    reader: impl Read + Send + 'static,
    writer: impl Write + Send + 'static,
    matcher: Option<FragmentMatcher>,
    shared: Arc<Shared>,
    hit: Arc<AtomicBool>,
    marker_rule: Option<StderrLineRule>,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let marking = marker_rule.is_some();
            let result = run(reader, writer, matcher, &shared, &hit, marker_rule.as_ref());
            let mut state = shared.lock();
            if marking {
                state.stderr_ended = true;
            }
            state.open = state.open.saturating_sub(1);
            if let Err(e) = result {
                state.error.get_or_insert((e.kind(), e.to_string()));
            }
            drop(state);
            shared.changed.notify_all();
        })
        .map(|_| ())
}

/// What is written in place of output that held a fragment of the key.
const WITHHELD: &[u8] = b"[goalcup: output withheld: a fragment of the provider key]\n";

fn run(
    mut reader: impl Read,
    mut writer: impl Write,
    mut matcher: Option<FragmentMatcher>,
    shared: &Shared,
    hit: &AtomicBool,
    marker_rule: Option<&StderrLineRule>,
) -> std::io::Result<()> {
    let mut line: Vec<u8> = Vec::new();
    let mut overlong = false;
    let mut forwarding = true;
    // With a matcher, the last `overlap` bytes read are held back until more has been read, so a
    // fragment a later read completes has not already been sent. At the end of the stream the
    // tail is flushed.
    let overlap = matcher.as_ref().map_or(0, FragmentMatcher::overlap);
    let mut held: Vec<u8> = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    let send = |writer: &mut dyn Write, bytes: &[u8], forwarding: &mut bool| {
        if *forwarding && (writer.write_all(bytes).is_err() || writer.flush().is_err()) {
            *forwarding = false;
        }
    };
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => {
                // The tail the matcher held back: it completed no fragment, so it is forwarded.
                send(&mut writer, &held, &mut forwarding);
                crate::key_scan::wipe(&mut held);
                return Ok(());
            }
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        let chunk = &buf[..n];
        if let Some(rule) = marker_rule {
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
        }
        let Some(matcher) = &mut matcher else {
            send(&mut writer, chunk, &mut forwarding);
            continue;
        };
        held.extend_from_slice(chunk);
        if matcher.contains(&held) {
            // A fragment is in what is held, or is completed by what just arrived: none of it is
            // forwarded (a screen, a transcript or a `tee`'d log is outside the scanned run
            // directory), and the destination is told something was withheld.
            hit.store(true, Ordering::Release);
            crate::key_scan::wipe(&mut held);
            held.clear();
            send(&mut writer, WITHHELD, &mut forwarding);
            continue;
        }
        let ready = held.len().saturating_sub(overlap);
        send(&mut writer, &held[..ready], &mut forwarding);
        crate::key_scan::wipe(&mut held[..ready]);
        held.drain(..ready);
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
