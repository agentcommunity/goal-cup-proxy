//! The stderr tee that carries a `stderr_line` harness's session identity.
//!
//! The harness prints one `session_id: <id>` line on stderr after its turn (the rule here is
//! `support::stderr_marker_rule`). The tee forwards every byte of stderr unchanged and reads
//! that one line under a strict grammar. Everything here uses in-memory readers, a socket pair,
//! or a `/bin/sh` child.

use crate::common::support;
use goalcup_cli::capture_tee::{NATIVE_FINISH_MIN, marker_wait_end, tee_stderr_capture_with_rule};
use goalcup_cli::launch::{ScopeMarker, Spawned, StderrLineRule, tee_marked_with_rule};
use goalcup_protocol::control::FINISH_COMMIT_RESERVE;
use goalcup_protocol::harness::IdGrammar;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const ID: &str = "20260930_141500_a1b2c3";
const OTHER: &str = "20260930_141501_ffeedd";
const PATIENCE: Duration = Duration::from_secs(10);

/// A reader that hands out its chunks one `read` at a time, then EOF.
struct Chunks(std::collections::VecDeque<Vec<u8>>);

impl Chunks {
    fn new(chunks: &[&[u8]]) -> Chunks {
        Chunks(
            chunks
                .iter()
                .filter(|c| !c.is_empty())
                .map(|c| c.to_vec())
                .collect(),
        )
    }
}

impl Read for Chunks {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self.0.pop_front() {
            None => Ok(0),
            Some(mut chunk) => {
                let n = chunk.len().min(buf.len());
                buf[..n].copy_from_slice(&chunk[..n]);
                if n < chunk.len() {
                    self.0.push_front(chunk.split_off(n));
                }
                Ok(n)
            }
        }
    }
}

/// A writer the test can read back, shareable with the tee's thread.
#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A destination that is closed: every write fails.
struct Closed;

impl Write for Closed {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
    }
}

fn marker(chunks: &[&[u8]]) -> ScopeMarker {
    let tee = tee_marked_with_rule(
        Chunks::new(chunks),
        Sink::default(),
        support::stderr_marker_rule(),
    )
    .unwrap();
    assert!(
        tee.finish(Instant::now() + PATIENCE).unwrap(),
        "the stream ended"
    );
    tee.marker_until(Instant::now())
}

fn line(id: &str) -> Vec<u8> {
    format!("session_id: {id}\n").into_bytes()
}

#[test]
fn marker_rule_honors_custom_prefix_and_token_grammar() {
    let rule = StderrLineRule {
        prefix: "run: ".into(),
        grammar: IdGrammar::Token,
    };
    let tee = tee_marked_with_rule(
        Chunks::new(&[b"session_id: ignored\nrun: Token_9-ok\n"]),
        Sink::default(),
        rule,
    )
    .unwrap();
    assert!(tee.finish(Instant::now() + PATIENCE).unwrap());
    assert_eq!(
        tee.marker_until(Instant::now()),
        ScopeMarker::One("Token_9-ok".into())
    );
}

#[test]
fn token_marker_accepts_exactly_128_bytes() {
    let token = format!("A{}_z", "9".repeat(125));
    assert_eq!(token.len(), 128);
    let input = format!("run: {token}\n");
    let tee = tee_marked_with_rule(
        Chunks::new(&[input.as_bytes()]),
        Sink::default(),
        StderrLineRule {
            prefix: "run: ".into(),
            grammar: IdGrammar::Token,
        },
    )
    .unwrap();
    assert!(tee.finish(Instant::now() + PATIENCE).unwrap());
    assert_eq!(tee.marker_until(Instant::now()), ScopeMarker::One(token));
}

#[test]
fn token_marker_refuses_empty_overlong_and_punctuation() {
    let rule = StderrLineRule {
        prefix: "run: ".into(),
        grammar: IdGrammar::Token,
    };
    let overlong = "x".repeat(129);
    let input = format!("run: \nrun: bad.dot\nrun: {overlong}\n");
    let tee =
        tee_marked_with_rule(Chunks::new(&[input.as_bytes()]), Sink::default(), rule).unwrap();
    assert!(tee.finish(Instant::now() + PATIENCE).unwrap());
    assert_eq!(tee.marker_until(Instant::now()), ScopeMarker::Absent);
}

/// A marker split across reads, at every byte boundary, is still found once.
#[test]
fn a_marker_split_across_reads_is_found() {
    let whole = [b"warning: something\n".to_vec(), line(ID)].concat();
    for cut in 0..=whole.len() {
        let (a, b) = whole.split_at(cut);
        assert_eq!(
            marker(&[a, b]),
            ScopeMarker::One(ID.to_string()),
            "cut at {cut}"
        );
    }
    // And one byte at a time.
    let singles: Vec<&[u8]> = whole.chunks(1).collect();
    assert_eq!(marker(&singles), ScopeMarker::One(ID.to_string()));
}

/// The same ID twice is one marker.
#[test]
fn the_same_id_twice_is_one_marker() {
    let twice = [line(ID), b"between\n".to_vec(), line(ID)].concat();
    assert_eq!(marker(&[&twice]), ScopeMarker::One(ID.to_string()));
}

/// Two different IDs conflict, in either order, and the conflict is final.
#[test]
fn two_different_ids_conflict_and_stay_conflicting() {
    let ab = [line(ID), line(OTHER)].concat();
    let ba = [line(OTHER), line(ID)].concat();
    let aba = [line(ID), line(OTHER), line(ID)].concat();
    for (what, bytes) in [("ab", ab), ("ba", ba), ("aba", aba)] {
        assert_eq!(marker(&[&bytes]), ScopeMarker::Conflicting, "{what}");
    }
}

/// No line at all is no marker.
#[test]
fn no_marker_is_absent() {
    assert_eq!(marker(&[]), ScopeMarker::Absent);
    assert_eq!(
        marker(&[b"just some stderr\nand more\n"]),
        ScopeMarker::Absent
    );
}

/// A line outside the grammar, an overlong line, an unterminated final line and a
/// carriage return are all ignored, and a valid line beside them still counts.
#[test]
fn a_line_outside_the_grammar_is_ignored() {
    let overlong = {
        let mut v = format!("session_id: {ID}").into_bytes();
        v.extend(std::iter::repeat_n(b'x', 5000));
        v.push(b'\n');
        v
    };
    let bad: Vec<(&str, Vec<u8>)> = vec![
        (
            "short date",
            b"session_id: 2026093_141500_a1b2c3\n".to_vec(),
        ),
        (
            "short time",
            b"session_id: 20260930_14150_a1b2c3\n".to_vec(),
        ),
        (
            "upper hex",
            b"session_id: 20260930_141500_A1B2C3\n".to_vec(),
        ),
        ("non hex", b"session_id: 20260930_141500_a1b2cz\n".to_vec()),
        ("short hex", b"session_id: 20260930_141500_a1b2c\n".to_vec()),
        (
            "long hex",
            b"session_id: 20260930_141500_a1b2c3d\n".to_vec(),
        ),
        ("no space", b"session_id:20260930_141500_a1b2c3\n".to_vec()),
        (
            "two spaces",
            b"session_id:  20260930_141500_a1b2c3\n".to_vec(),
        ),
        (
            "leading space",
            b" session_id: 20260930_141500_a1b2c3\n".to_vec(),
        ),
        (
            "trailing space",
            b"session_id: 20260930_141500_a1b2c3 \n".to_vec(),
        ),
        (
            "prefix text",
            b"x session_id: 20260930_141500_a1b2c3\n".to_vec(),
        ),
        (
            "suffix text",
            b"session_id: 20260930_141500_a1b2c3 extra\n".to_vec(),
        ),
        (
            "capital S",
            b"Session_id: 20260930_141500_a1b2c3\n".to_vec(),
        ),
        (
            "carriage return",
            b"session_id: 20260930_141500_a1b2c3\r\n".to_vec(),
        ),
        ("unterminated", format!("session_id: {ID}").into_bytes()),
        ("overlong", overlong.clone()),
        (
            "nul inside",
            b"session_id: 20260930_141500\0a1b2c3\n".to_vec(),
        ),
    ];
    for (what, bytes) in &bad {
        assert_eq!(marker(&[bytes]), ScopeMarker::Absent, "{what}");
        // Beside a good line it neither conflicts nor hides it.
        let with_good = [bytes.clone(), b"\n".to_vec(), line(ID)].concat();
        assert_eq!(
            marker(&[&with_good]),
            ScopeMarker::One(ID.to_string()),
            "{what} beside a good line"
        );
    }
}

/// An overlong line is skipped to its newline without growing a buffer without bound: ten
/// megabytes without a newline, then a good line.
#[test]
fn an_endless_line_is_bounded_and_a_later_line_still_counts() {
    let junk = vec![b'z'; 1 << 20];
    let mut chunks: Vec<&[u8]> = (0..10).map(|_| junk.as_slice()).collect();
    let good = [b"\n".to_vec(), line(ID)].concat();
    chunks.push(&good);
    assert_eq!(marker(&chunks), ScopeMarker::One(ID.to_string()));
}

/// Every byte is forwarded, unchanged, in order, whatever it is.
#[test]
fn bytes_are_forwarded_verbatim() {
    let mut payload: Vec<u8> = (0..=255u8).collect();
    payload.extend_from_slice(b"\r\n\0\xff\xfe not utf-8 \n");
    payload.extend(line(ID));
    payload.extend((0..200_000u32).map(|i| (i % 251) as u8));
    for size in [1usize, 7, 4096, 65_536] {
        let chunks: Vec<&[u8]> = payload.chunks(size).collect();
        let sink = Sink::default();
        let tee = tee_marked_with_rule(
            Chunks::new(&chunks),
            sink.clone(),
            support::stderr_marker_rule(),
        )
        .unwrap();
        assert!(tee.finish(Instant::now() + PATIENCE).unwrap());
        assert_eq!(*sink.0.lock().unwrap(), payload, "chunk size {size}");
    }
}

/// A destination that is closed turns forwarding into draining: the marker is still read, and
/// the tee still completes.
#[test]
fn a_closed_destination_does_not_stop_the_reading() {
    let tee = tee_marked_with_rule(
        Chunks::new(&[b"noise\n", &line(ID)]),
        Closed,
        support::stderr_marker_rule(),
    )
    .unwrap();
    assert!(tee.finish(Instant::now() + PATIENCE).unwrap());
    assert_eq!(
        tee.marker_until(Instant::now()),
        ScopeMarker::One(ID.to_string())
    );
}

/// A descendant holds stderr open, so there is no EOF. The wait still ends at its
/// deadline, and the answer is decided from the lines seen so far.
#[test]
fn the_wait_decides_from_the_lines_seen_when_stderr_is_held_open() {
    let (mut writer, reader) = std::os::unix::net::UnixStream::pair().unwrap();
    let tee = tee_marked_with_rule(reader, Sink::default(), support::stderr_marker_rule()).unwrap();
    writer.write_all(b"progress\n").unwrap();
    writer.write_all(&line(ID)).unwrap();
    writer.flush().unwrap();
    let started = Instant::now();
    let decided = tee.marker_until(started + Duration::from_millis(600));
    let took = started.elapsed();
    assert_eq!(decided, ScopeMarker::One(ID.to_string()));
    assert!(
        took >= Duration::from_millis(500) && took < Duration::from_secs(5),
        "waited {took:?} for a deadline of 600ms"
    );
    // The completion confirms nothing: the stream is still open.
    assert!(
        !tee.finish(Instant::now() + Duration::from_millis(100))
            .unwrap()
    );
    // A second, different ID is a later fact; once the stream ends the question sees both.
    writer.write_all(&line(OTHER)).unwrap();
    writer.flush().unwrap();
    drop(writer);
    assert!(tee.finish(Instant::now() + PATIENCE).unwrap());
    assert_eq!(tee.marker_until(Instant::now()), ScopeMarker::Conflicting);
}

/// The wait returns as soon as the stream ends: a harness that closed stderr is not waited for.
#[test]
fn the_wait_ends_at_eof_not_at_its_deadline() {
    let tee = tee_marked_with_rule(
        Chunks::new(&[&line(ID)]),
        Sink::default(),
        support::stderr_marker_rule(),
    )
    .unwrap();
    let started = Instant::now();
    let decided = tee.marker_until(started + Duration::from_secs(30));
    assert_eq!(decided, ScopeMarker::One(ID.to_string()));
    assert!(started.elapsed() < Duration::from_secs(10));
}

/// A marker on stdout is never used. Only a child's stderr is read for the identity.
#[test]
fn a_marker_on_stdout_is_never_used() {
    let script = format!("printf 'session_id: {ID}\\n'; printf 'warn\\n' >&2");
    let (spawned, tee) = tee_a_real_child(&script);
    assert!(spawned.success());
    assert!(tee.finish(Instant::now() + PATIENCE).unwrap());
    assert_eq!(tee.marker_until(Instant::now()), ScopeMarker::Absent);
}

/// The capture tee takes stderr off a real child and finds the marker on it.
#[test]
fn the_capture_tee_reads_a_real_childs_stderr() {
    let script = format!("printf 'out\\n'; printf 'session_id: {ID}\\n' >&2");
    let (spawned, tee) = tee_a_real_child(&script);
    assert!(spawned.success());
    assert!(tee.finish(Instant::now() + PATIENCE).unwrap());
    assert_eq!(
        tee.marker_until(Instant::now()),
        ScopeMarker::One(ID.to_string())
    );
}

/// Run `script` under `/bin/sh` with stderr piped to the capture tee and stdout discarded, and
/// wait for it.
fn tee_a_real_child(
    script: &str,
) -> (
    std::process::ExitStatus,
    goalcup_cli::capture_tee::CaptureStderr,
) {
    let mut command = std::process::Command::new("/bin/sh");
    command
        .args(["-c", script])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    let child = command.spawn().unwrap();
    let mut spawned = Spawned {
        child,
        program: "/bin/sh".to_string(),
    };
    let tee = tee_stderr_capture_with_rule(&mut spawned, support::stderr_marker_rule()).unwrap();
    let status = spawned.child.wait().unwrap();
    (status, tee)
}

/// Review amendment (c): the wait for the marker ends the commit reserve and the daemon's native
/// minimum before the finish's deadline, so the daemon's native read is left a share of its drain.
/// With less than that left it ends now, and the answer is then decided from the lines seen.
#[test]
fn the_marker_wait_ends_before_the_drain_share_does() {
    let deadline = Instant::now() + Duration::from_secs(10);
    let end = marker_wait_end(deadline);
    assert_eq!(end + FINISH_COMMIT_RESERVE + NATIVE_FINISH_MIN, deadline);
    // The drain share (what the daemon calls `drain_end`) is the reserve before the deadline.
    let drain_end = deadline - FINISH_COMMIT_RESERVE;
    assert!(drain_end - end >= NATIVE_FINISH_MIN);

    // Less than the reserve and the minimum left: no wait at all.
    let nearly = Instant::now() + Duration::from_millis(500);
    assert!(marker_wait_end(nearly) <= Instant::now());

    // A held-open stderr with a marker already seen is answered at that end, not at EOF.
    let (mut writer, reader) = std::os::unix::net::UnixStream::pair().unwrap();
    let tee = tee_marked_with_rule(reader, Sink::default(), support::stderr_marker_rule()).unwrap();
    writer.write_all(&line(ID)).unwrap();
    let begun = Instant::now();
    let decided = tee.marker_until(marker_wait_end(
        begun + FINISH_COMMIT_RESERVE + NATIVE_FINISH_MIN + Duration::from_millis(300),
    ));
    assert_eq!(decided, ScopeMarker::One(ID.to_string()));
    assert!(begun.elapsed() < Duration::from_secs(5));
    drop(writer);
}
