//! The stderr tee that carries Hermes' own session identity.
//!
//! Hermes prints one `session_id: <id>` line on stderr after its turn. The tee forwards every
//! byte of both streams unchanged, scans them for the key's fragments, and reads that one line
//! under a strict grammar. Everything here uses in-memory readers, a socket pair, or a
//! `/bin/sh` child; the synthetic key is `secret_support::KEY`.

use crate::common::secret_support;
use goalcup_cli::capture_tee::{NATIVE_FINISH_MIN, marker_wait_end};
use goalcup_cli::key_scan::FragmentMatcher;
use goalcup_cli::launch::{
    ScopeMarker, Spawned, StderrLineRule, tee_marked, tee_marked_capture, tee_marked_with_rule,
};
use goalcup_protocol::control::FINISH_COMMIT_RESERVE;
use goalcup_protocol::harness::IdGrammar;
use secret_support::KEY;
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
    let tee = tee_marked(Chunks::new(chunks), Sink::default(), None).unwrap();
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
        None,
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
        None,
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
    let tee = tee_marked_with_rule(
        Chunks::new(&[input.as_bytes()]),
        Sink::default(),
        None,
        rule,
    )
    .unwrap();
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
        let tee = tee_marked(Chunks::new(&chunks), sink.clone(), None).unwrap();
        assert!(tee.finish(Instant::now() + PATIENCE).unwrap());
        assert_eq!(*sink.0.lock().unwrap(), payload, "chunk size {size}");
    }
}

/// A destination that is closed turns forwarding into draining: the marker is still read, and
/// the tee still completes.
#[test]
fn a_closed_destination_does_not_stop_the_reading() {
    let tee = tee_marked(Chunks::new(&[b"noise\n", &line(ID)]), Closed, None).unwrap();
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
    let tee = tee_marked(reader, Sink::default(), None).unwrap();
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
    let tee = tee_marked(Chunks::new(&[&line(ID)]), Sink::default(), None).unwrap();
    let started = Instant::now();
    let decided = tee.marker_until(started + Duration::from_secs(30));
    assert_eq!(decided, ScopeMarker::One(ID.to_string()));
    assert!(started.elapsed() < Duration::from_secs(10));
}

/// The tee's key scan: a fragment of the key anywhere in either stream is a live hit, however
/// the reads cut it, and a clean stream is not one.
#[test]
fn the_tee_reports_a_key_fragment_split_across_reads() {
    let masked = format!(
        "Authorization: Bearer {}...{}\n",
        &KEY[..8],
        &KEY[KEY.len() - 4..]
    );
    for cut in [1usize, 5, 12, 30] {
        let (a, b) = masked.as_bytes().split_at(cut);
        let matcher = FragmentMatcher::new(KEY.as_bytes()).unwrap();
        let tee = tee_marked(Chunks::new(&[a, b]), Sink::default(), Some(matcher)).unwrap();
        assert!(tee.finish(Instant::now() + PATIENCE).unwrap());
        assert!(tee.key_hit(), "cut at {cut}");
    }
    let matcher = FragmentMatcher::new(KEY.as_bytes()).unwrap();
    let tee = tee_marked(
        Chunks::new(&[b"nothing to see\n", &line(ID)]),
        Sink::default(),
        Some(matcher),
    )
    .unwrap();
    assert!(tee.finish(Instant::now() + PATIENCE).unwrap());
    assert!(!tee.key_hit());
}

/// A marker on stdout is never used. Only a child's stderr is read for the identity.
#[test]
fn a_marker_on_stdout_is_never_used() {
    let secret = {
        let mut env = std::collections::BTreeMap::new();
        env.insert("DEEPSEEK_API_KEY".to_string(), KEY.to_string());
        let profile = hermes_profile();
        goalcup_cli::launch::child_secret_for_harness(&profile, &env)
            .unwrap()
            .unwrap()
    };
    let script = format!("printf 'session_id: {ID}\\n'; printf 'warn\\n' >&2");
    let mut command = std::process::Command::new("/bin/sh");
    command
        .args(["-c", &script])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let child = command.spawn().unwrap();
    let mut spawned = Spawned {
        child,
        program: "/bin/sh".to_string(),
    };
    let tee = tee_marked_capture(&mut spawned, Some(&secret)).unwrap();
    let status = spawned.child.wait().unwrap();
    assert!(status.success());
    assert!(tee.finish(Instant::now() + PATIENCE).unwrap());
    assert_eq!(tee.marker_until(Instant::now()), ScopeMarker::Absent);
    assert!(!tee.key_hit());
}

/// The wrapper takes both pipes off a real child, finds the marker on its stderr, and sees a
/// key fragment on its stdout.
#[test]
fn the_capture_tee_reads_a_real_childs_stderr_and_scans_both_streams() {
    let secret = {
        let mut env = std::collections::BTreeMap::new();
        env.insert("DEEPSEEK_API_KEY".to_string(), KEY.to_string());
        goalcup_cli::launch::child_secret_for_harness(&hermes_profile(), &env)
            .unwrap()
            .unwrap()
    };
    // The child prints a masked form of the key it is not given; the tee scans for it anyway.
    let masked = format!("{}...{}", &KEY[..8], &KEY[KEY.len() - 4..]);
    let script = format!("printf 'out %s\\n' '{masked}'; printf 'session_id: {ID}\\n' >&2");
    let mut command = std::process::Command::new("/bin/sh");
    command
        .args(["-c", &script])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let child = command.spawn().unwrap();
    let mut spawned = Spawned {
        child,
        program: "/bin/sh".to_string(),
    };
    let tee = tee_marked_capture(&mut spawned, Some(&secret)).unwrap();
    spawned.child.wait().unwrap();
    assert!(tee.finish(Instant::now() + PATIENCE).unwrap());
    assert_eq!(
        tee.marker_until(Instant::now()),
        ScopeMarker::One(ID.to_string())
    );
    assert!(tee.key_hit(), "the fragment on stdout is a live hit");
}

fn hermes_profile() -> goalcup_protocol::harness::CaptureSpec {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../connectors");
    goalcup_cli::connector::load_harness(&dir, "hermes-cli", false)
        .unwrap()
        .loaded
        .manifest
        .capture
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
    let tee = tee_marked(reader, Sink::default(), None).unwrap();
    writer.write_all(&line(ID)).unwrap();
    let begun = Instant::now();
    let decided = tee.marker_until(marker_wait_end(
        begun + FINISH_COMMIT_RESERVE + NATIVE_FINISH_MIN + Duration::from_millis(300),
    ));
    assert_eq!(decided, ScopeMarker::One(ID.to_string()));
    assert!(begun.elapsed() < Duration::from_secs(5));
    drop(writer);
}

// ---------------------------------------------------------------------------------------
// A fragment the tee sees is not forwarded to the terminal.
// ---------------------------------------------------------------------------------------

/// Every form of the key a leak could take on a screen.
fn forms() -> Vec<String> {
    let n = KEY.len();
    vec![
        KEY.to_string(),
        KEY[..8].to_string(),
        KEY[n - 8..].to_string(),
        format!("{}***{}", &KEY[..8], &KEY[n - 4..]),
        format!("{}...{}", &KEY[..6], &KEY[n - 4..]),
    ]
}

fn assert_no_form(what: &str, bytes: &[u8]) {
    let text = String::from_utf8_lossy(bytes);
    for form in forms() {
        assert!(!text.contains(&form), "{what}: the output shows `{form}`");
    }
}

const WITHHELD: &str = "[goalcup: output withheld: a fragment of the provider key]";

/// With a matcher the tee holds back the bytes a fragment could still be completed by, and
/// flushes them at the end of the stream: clean output is still forwarded byte for byte.
#[test]
fn a_matching_tee_still_forwards_clean_output_verbatim() {
    let mut payload: Vec<u8> = (0..=255u8).collect();
    payload.extend_from_slice(b"\r\n\0\xff\xfe not utf-8 \n");
    payload.extend(line(ID));
    payload.extend((0..200_000u32).map(|i| (i % 251) as u8));
    // A near miss is not a fragment.
    payload.extend_from_slice(b" sk-abcdefgh and sk-xxxx...yyyy ");
    for size in [1usize, 7, 4096, 65_536] {
        let chunks: Vec<&[u8]> = payload.chunks(size).collect();
        let sink = Sink::default();
        let matcher = FragmentMatcher::new(KEY.as_bytes()).unwrap();
        let tee = tee_marked(Chunks::new(&chunks), sink.clone(), Some(matcher)).unwrap();
        assert!(tee.finish(Instant::now() + PATIENCE).unwrap());
        assert!(!tee.key_hit(), "chunk size {size}");
        assert_eq!(*sink.0.lock().unwrap(), payload, "chunk size {size}");
        assert_eq!(
            tee.marker_until(Instant::now()),
            ScopeMarker::One(ID.to_string())
        );
    }
}

/// A fragment never reaches the destination, however the reads cut it: not whole, not as a prefix
/// or a mask, and not as a part held and sent later. The bytes read with it (at most one read and
/// the held tail) are withheld too, a fixed line says something was, the hit is recorded, and the
/// output that follows still arrives.
#[test]
fn a_fragment_is_withheld_however_the_reads_cut_it() {
    let tail = "z".repeat(300);
    for form in forms() {
        let text = format!("start of the output\nthe provider said: {form}.\n{tail}\nTHE-END\n");
        for size in [1usize, 2, 3, 5, 8, 13, 64, 4096] {
            let chunks: Vec<&[u8]> = text.as_bytes().chunks(size).collect();
            let sink = Sink::default();
            let matcher = FragmentMatcher::new(KEY.as_bytes()).unwrap();
            let tee = tee_marked(Chunks::new(&chunks), sink.clone(), Some(matcher)).unwrap();
            assert!(tee.finish(Instant::now() + PATIENCE).unwrap());
            let sent = sink.0.lock().unwrap().clone();
            assert_no_form(&format!("`{form}` cut every {size}"), &sent);
            assert!(tee.key_hit(), "`{form}` cut every {size}");
            let shown = String::from_utf8_lossy(&sent);
            assert!(shown.contains(WITHHELD), "{shown}");
            if size <= 64 {
                assert!(
                    shown.ends_with("THE-END\n"),
                    "output after the hit still arrives (cut every {size}): {shown}"
                );
            }
        }
    }
}

/// The wrapper over a real child: the key printed in pieces on stdout and stderr reaches neither
/// destination, and the marker on stderr is still read.
#[test]
fn a_real_childs_two_streams_are_withheld_before_they_reach_the_terminal() {
    use goalcup_cli::launch::{child_secret_for_harness, tee_marked_capture_to};
    let secret = {
        let mut env = std::collections::BTreeMap::new();
        env.insert("DEEPSEEK_API_KEY".to_string(), KEY.to_string());
        child_secret_for_harness(&hermes_profile(), &env)
            .unwrap()
            .unwrap()
    };
    let n = KEY.len();
    let script = format!(
        "printf 'out one %s' '{a}'; printf '%s done\\n' '{b}'; \
         printf 'err one %s' '{a}' >&2; printf '%s done\\n' '{b}' >&2; \
         printf 'Incorrect API key provided: {c}.\\n' >&2; printf 'session_id: {ID}\\n' >&2",
        a = &KEY[..4],
        b = &KEY[4..],
        c = format_args!("{}***{}", &KEY[..8], &KEY[n - 4..]),
    );
    let mut command = std::process::Command::new("/bin/sh");
    command
        .args(["-c", &script])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut spawned = Spawned {
        child: command.spawn().unwrap(),
        program: "/bin/sh".to_string(),
    };
    let (out, err) = (Sink::default(), Sink::default());
    let tee = tee_marked_capture_to(&mut spawned, Some(&secret), out.clone(), err.clone()).unwrap();
    spawned.child.wait().unwrap();
    assert!(tee.finish(Instant::now() + PATIENCE).unwrap());
    assert!(tee.key_hit());
    for (what, sink) in [("stdout", &out), ("stderr", &err)] {
        let sent = sink.0.lock().unwrap().clone();
        assert_no_form(what, &sent);
        assert!(String::from_utf8_lossy(&sent).contains(WITHHELD), "{what}");
    }
    assert_eq!(
        tee.marker_until(Instant::now()),
        ScopeMarker::One(ID.to_string())
    );
}

// ---------------------------------------------------------------------------------------
// A stderr marker that holds a fragment of the key is never bound (D50).
// ---------------------------------------------------------------------------------------

fn token_rule() -> StderrLineRule {
    StderrLineRule {
        prefix: "scope: ".into(),
        grammar: IdGrammar::Token,
    }
}

/// The `token` grammar's alphabet is a key's, so a keyed child can print `scope: <key>`. That
/// id would be the daemon's deferred scope, written into the chain: a marker whose line holds
/// any key-scan form is not bound, however the reads cut it and whatever marker follows, and it
/// is a key hit. Its bytes are withheld from the terminal as any fragment is.
#[test]
fn a_marker_holding_a_key_form_is_never_bound() {
    let n = KEY.len();
    let ids = [
        KEY.to_string(),
        KEY[n - 8..].to_string(),
        format!("run_{}_0001", &KEY[n - 8..]),
    ];
    for id in &ids {
        for after in ["", "scope: clean_one\n"] {
            let input = format!("before\nscope: {id}\n{after}after\n");
            for size in [1usize, 7, 4096] {
                let chunks: Vec<&[u8]> = input.as_bytes().chunks(size).collect();
                let sink = Sink::default();
                let matcher = FragmentMatcher::new(KEY.as_bytes()).unwrap();
                let tee = tee_marked_with_rule(
                    Chunks::new(&chunks),
                    sink.clone(),
                    Some(matcher),
                    token_rule(),
                )
                .unwrap();
                assert!(tee.finish(Instant::now() + PATIENCE).unwrap());
                let what = format!(
                    "marker of {} bytes, then {after:?}, cut every {size}",
                    id.len()
                );
                assert_eq!(
                    tee.marker_until(Instant::now()),
                    ScopeMarker::Absent,
                    "{what}"
                );
                assert!(tee.key_hit(), "{what}");
                assert_no_form(&what, &sink.0.lock().unwrap());
            }
        }
    }
}

/// The same marker rule with the key matcher still binds a clean marker, and an unkeyed tee
/// (no matcher) binds what it always bound.
#[test]
fn a_clean_marker_still_binds_on_a_keyed_tee() {
    let matcher = FragmentMatcher::new(KEY.as_bytes()).unwrap();
    let tee = tee_marked_with_rule(
        Chunks::new(&[b"scope: Token_9-ok\n"]),
        Sink::default(),
        Some(matcher),
        token_rule(),
    )
    .unwrap();
    assert!(tee.finish(Instant::now() + PATIENCE).unwrap());
    assert_eq!(
        tee.marker_until(Instant::now()),
        ScopeMarker::One("Token_9-ok".into())
    );
    assert!(!tee.key_hit());
}

/// The launcher's keyed `stderr_line` path over a real child: `scope: <key>` on stderr binds
/// nothing, is a hit, and no fragment reaches either destination.
#[test]
fn a_real_childs_key_bearing_stderr_marker_binds_nothing() {
    use goalcup_cli::launch::{child_secret_for_harness, tee_marked_capture_with_identity_to};
    let secret = {
        let mut env = std::collections::BTreeMap::new();
        env.insert("DEEPSEEK_API_KEY".to_string(), KEY.to_string());
        child_secret_for_harness(&hermes_profile(), &env)
            .unwrap()
            .unwrap()
    };
    let n = KEY.len();
    for id in [KEY, &KEY[n - 8..]] {
        let script = format!("printf 'hello\\n'; printf 'scope: {id}\\n' >&2");
        let mut command = std::process::Command::new("/bin/sh");
        command
            .args(["-c", &script])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut spawned = Spawned {
            child: command.spawn().unwrap(),
            program: "/bin/sh".to_string(),
        };
        let (out, err) = (Sink::default(), Sink::default());
        let (scope, tee) = tee_marked_capture_with_identity_to(
            &mut spawned,
            Some(&secret),
            None,
            Some(token_rule()),
            out.clone(),
            err.clone(),
        )
        .unwrap();
        assert!(scope.is_none(), "no stdout rule, no scope receiver");
        spawned.child.wait().unwrap();
        assert!(tee.finish(Instant::now() + PATIENCE).unwrap());
        assert_eq!(tee.marker_until(Instant::now()), ScopeMarker::Absent);
        assert!(tee.key_hit());
        for (what, sink) in [("stdout", &out), ("stderr", &err)] {
            assert_no_form(what, &sink.0.lock().unwrap());
        }
        assert_eq!(*out.0.lock().unwrap(), b"hello\n");
    }
}
