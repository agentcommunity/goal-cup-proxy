//! `goalcup start --goal-file` of a keyed connector (a child-only provider key, a per-run home,
//! a goal flag, a session id on stderr bound at the finish), through the real daemon in process,
//! the real proxy and sender inside `start_with`, and a fake harness (`fixtures/m2-fake-keyed.sh`)
//! that writes its own session snapshot and prints its identity on stderr. The connector is the
//! fixture `tests/fixtures/connectors/keyed-per-run`, never a shipped one.
//!
//! Every key here is the synthetic one in `secret_support`. Nothing reads a credential file or
//! resolves a real `HOME`; every directory is a temporary one.
//!
//! macOS and Linux (the trace folder must be on APFS, ext4 or XFS). Every test holds the
//! process-table lock: each runs a daemon and spawns a harness.

use crate::common::m2_cli_support;
use crate::common::secret_support;
use crate::common::support;

use crate::common::table_lock;

use goalcup_cli::CliError;
use goalcup_cli::key_scan::{FragmentMatcher, ScanOutcome};
use goalcup_protocol::capture::{CaptureRecord, CaptureSource, GapReason};
use goalcup_protocol::control::{CaptureSealError, CaptureSealRequest, ControlError, DaemonClient};
use m2_cli_support::{Hooked, Rig};
use secret_support::KEY;
use std::path::PathBuf;
use std::time::Duration;

const GOAL: &str = "say hi, then stop";
const RESERVE: u64 = 4 * 1024 * 1024;

fn joined<T>(handle: std::thread::ScopedJoinHandle<'_, T>) -> T {
    handle.join().expect("the start thread panicked")
}

fn matcher() -> FragmentMatcher {
    FragmentMatcher::new(KEY.as_bytes()).unwrap()
}

/// Run `goalcup start` of the keyed connector to its end.
fn run(
    rig: &Rig,
    client: &Hooked<'_>,
) -> (
    Result<goalcup_cli::commands::start::StartOutcome, CliError>,
    String,
) {
    let goal = rig.goal(GOAL);
    std::thread::scope(|s| joined(rig.start(s, client, rig.keyed_options(&goal, RESERVE), None)))
}

fn run_dir(rig: &Rig) -> PathBuf {
    let dirs = rig.run_dirs();
    assert_eq!(dirs.len(), 1, "{dirs:?}");
    dirs[0].clone()
}

fn verdict(rig: &Rig) -> String {
    std::fs::read_to_string(run_dir(rig).join("key-scan.txt")).expect("a verdict file")
}

fn gaps(records: &[CaptureRecord], reason: GapReason) -> usize {
    records
        .iter()
        .filter(|r| matches!(r, CaptureRecord::Gap { stream: None, reason: g, .. } if *g == reason))
        .count()
}

fn native_labels(records: &[CaptureRecord]) -> Vec<String> {
    records
        .iter()
        .filter_map(|r| match r {
            CaptureRecord::Open {
                source: CaptureSource::Native,
                metadata,
                ..
            } => Some(metadata.label.clone()),
            _ => None,
        })
        .collect()
}

fn session_id(rig: &Rig) -> String {
    rig.harness_file("session-id")
        .expect("the fake wrote its session id")
        .trim()
        .to_string()
}

/// End to end: the launcher reads the identity off stderr after the harness exits, stops the
/// proxy first, carries the identity on the finish request, and the run ends with its own
/// session snapshot captured, no gap, and a `clean` verdict. The child saw the key,
/// the private homes and the loopback proxy, and the key is in nothing this machine kept.
#[test]
fn a_keyed_run_ends_with_its_snapshot_captured_and_a_clean_verdict() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::keyed();
    rig.with_env("DEEPSEEK_API_KEY", KEY);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let (result, out) = run(&rig, &client);
    let outcome = result.unwrap_or_else(|e| panic!("{e}\n{out}"));
    assert!(
        outcome.exit_status.expect("a harness ran").success(),
        "{out}"
    );

    let entry = client.created().entry_id;
    let id = session_id(&rig);
    let seals = client.book().seals.clone();
    assert_eq!(seals.len(), 1);
    assert_eq!(seals[0].native_scope.as_deref(), Some(id.as_str()));
    assert!(seals[0].remaining_finish_ms.is_some());

    let (report, records) = rig.read_trace(&entry);
    assert_eq!(native_labels(&records).len(), 1, "{records:#?}");
    assert!(
        native_labels(&records)[0].contains(&format!("session_{id}.json")),
        "{:?}",
        native_labels(&records)
    );
    assert_eq!(gaps(&records, GapReason::SourceChanged), 0, "{records:#?}");
    assert!(!report.coverage_incomplete, "{:?}", report.findings);

    // The verdict, and the run directory's shape.
    let text = verdict(&rig);
    assert_eq!(text.lines().next(), Some("clean"), "{text}");
    // The participant is told the scan came out clean, not left to infer it from silence.
    assert!(out.contains("key scan: clean ("), "{out}");
    assert!(out.contains("harness exited: exit code 0."), "{out}");
    // H-18a: the verdict is bound to what was scanned, and a retention step can recompute it.
    assert!(text.contains("\ntrace-chain bytes="), "{text}");
    let current = goalcup_cli::key_scan::check_verdict(&run_dir(&rig));
    assert!(current.is_clean(), "{current}\n{text}");
    let run = run_dir(&rig);
    assert!(
        run.join(format!("keyed/sessions/session_{id}.json"))
            .is_file()
    );
    let seeds = support::fixture_connectors_dir().join("keyed-per-run/files");
    for name in ["config.yaml", "auth.json"] {
        assert_eq!(
            std::fs::read(run.join("keyed").join(name)).unwrap(),
            std::fs::read(seeds.join(name)).unwrap(),
            "{name}"
        );
    }

    // What the child was started with.
    let argv = rig.harness_file("argv").unwrap();
    assert_eq!(argv, format!("chat\n-Q\n--query={GOAL}\n"));
    let env = rig.harness_file("env").unwrap();
    let has = |line: &str| env.lines().any(|l| l == line);
    assert!(
        has(&format!("DEEPSEEK_API_KEY={KEY}")),
        "the child has the key"
    );
    assert!(has(&format!("KEYED_HOME={}", run.join("keyed").display())));
    assert!(has(&format!("HOME={}", run.join("home").display())));
    assert!(has("TOOL_HOME_MODE=profile"));
    assert!(
        env.lines()
            .any(|l| l.starts_with("HTTPS_PROXY=http://goalcup:") && l.contains("@127.0.0.1:")),
        "{env}"
    );
    // `goalcup start` hands the harness's own CA variable the proxy's CA alone (D41), in
    // replace mode too: one certificate, never the roots file a shared row gets.
    assert!(env.lines().any(|l| l.starts_with("KEYED_CA_BUNDLE=")));
    let handed = rig
        .harness_file("ca.pem")
        .expect("the fake copied its CA file");
    assert_eq!(
        goalcup_cli::ca_bundle::certificates(handed.as_bytes()).len(),
        1,
        "the own CA variable names a file of exactly one certificate"
    );
    assert!(
        !env.lines()
            .any(|l| l.starts_with("OPENAI") || l.contains("BASE_URL")),
        "{env}"
    );

    // The key is in nothing this machine kept, and nothing the CLI printed.
    let m = matcher();
    for (what, text) in [
        ("the CLI's own output", out.clone()),
        ("the entry records", entries(&rig)),
        ("the trace records", format!("{records:?}")),
    ] {
        assert!(!m.contains(text.as_bytes()), "{what} shows the key");
    }
    // The order the design fixes: the launch is recorded, then the finish (proxy stopped first).
    let names = client.names();
    let at = |name: &str| names.iter().position(|n| n == name).unwrap();
    assert!(
        at("record_capture_launch") < at("seal_capture"),
        "{names:?}"
    );
}

/// Every `entries/*.json` this machine wrote, as one string.
fn entries(rig: &Rig) -> String {
    let mut text = String::new();
    for entry in std::fs::read_dir(rig.cfg.join("entries"))
        .into_iter()
        .flatten()
        .flatten()
    {
        text.push_str(&std::fs::read_to_string(entry.path()).unwrap());
    }
    text
}

/// A missing or unusable key is refused before the daemon is asked for anything, and
/// nothing is created: no entry, no run directory, no harness.
#[test]
fn a_missing_or_unusable_key_refuses_before_the_daemon_is_asked() {
    for (what, key) in [
        ("missing", None),
        ("quoted", Some(format!("\"{KEY}\""))),
        ("short", Some("sk-short".to_string())),
    ] {
        let _lock = table_lock::TableLock::acquire();
        let rig = Rig::keyed();
        if let Some(key) = &key {
            rig.with_env("DEEPSEEK_API_KEY", key);
        }
        let http = rig.client();
        let client = Hooked::new(&http, &rig);
        let (result, _out) = run(&rig, &client);
        let error = result.expect_err(what);
        assert!(matches!(error, CliError::Refused(_)), "{what}: {error:?}");
        assert_eq!(error.exit_code(), 1, "{what}");
        assert!(
            error.to_string().contains("DEEPSEEK_API_KEY"),
            "{what}: {error}"
        );
        if let Some(key) = &key {
            assert!(!error.to_string().contains(key.as_str()), "{what}: echoed");
        }
        assert!(
            client.names().is_empty(),
            "{what}: the daemon was asked: {:?}",
            client.names()
        );
        assert!(rig.run_dirs().is_empty(), "{what}");
        assert!(!rig.harness_started(), "{what}");
    }
}

/// On the wire: a goal file inside a Git work tree is refused before the daemon is
/// asked for anything.
#[test]
fn a_goal_file_inside_a_git_work_tree_refuses_before_anything_is_created() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::keyed();
    rig.with_env("DEEPSEEK_API_KEY", KEY);
    let goal = rig.goal(GOAL);
    std::fs::create_dir(goal.parent().unwrap().join(".git")).unwrap();
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let (result, _out) = std::thread::scope(|s| {
        joined(rig.start(s, &client, rig.keyed_options(&goal, RESERVE), None))
    });
    let error = result.expect_err("a work tree is refused");
    assert!(matches!(error, CliError::Refused(_)), "{error:?}");
    assert!(error.to_string().contains("Git work tree"), "{error}");
    assert!(client.names().is_empty(), "{:?}", client.names());
    // The goal's own directory holds the `.git` this test made, and no run directory.
    assert!(
        rig.run_dirs().iter().all(|d| !d
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("goalcup-"))),
        "{:?}",
        rig.run_dirs()
    );
    assert!(!rig.harness_started());
}

/// A request dump holding a key fragment fails the row. The verdict names the file and
/// no fragment, the command exits 1 with a fixed sentence, and the run directory stays.
#[test]
fn a_planted_request_dump_fails_the_row_and_the_verdict_holds_no_fragment() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::keyed();
    rig.with_env("DEEPSEEK_API_KEY", KEY);
    rig.tell(
        "steps.sh",
        "tail4=$(printf %s \"$DEEPSEEK_API_KEY\" | tail -c 4)\n\
         printf '{\"headers\":{\"Authorization\":\"Bearer %.8s...%s\"}}' \"$DEEPSEEK_API_KEY\" \"$tail4\" \
         > \"$KEYED_HOME/sessions/request_dump_x.json\"\n",
    );
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let (result, out) = run(&rig, &client);
    let error = result.expect_err("a fragment fails the row");
    assert!(matches!(error, CliError::Refused(_)), "{error:?}");
    assert_eq!(error.exit_code(), 1);
    let said = error.to_string();
    assert!(said.contains("key-scan.txt"), "{said}");
    assert!(said.contains("stays local"), "{said}");
    assert!(!matcher().contains(said.as_bytes()) && !matcher().contains(out.as_bytes()));
    // The finish itself completed: this is the scan's verdict, not a failed seal.
    assert_eq!(client.book().seals.len(), 1);

    let text = verdict(&rig);
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("hit"), "{text}");
    let paths: Vec<&str> = lines.collect();
    assert!(
        paths
            .iter()
            .any(|p| p.ends_with("keyed/sessions/request_dump_x.json")),
        "{text}"
    );
    assert!(
        !matcher().contains(text.as_bytes()),
        "the verdict holds a fragment"
    );
    assert!(
        run_dir(&rig)
            .join("keyed/sessions/request_dump_x.json")
            .is_file()
    );
}

/// End to end: the first eight bytes of the key straddle the boundary between two segment
/// records of the session snapshot. No record's text holds them, so a scan of the trace file's
/// text is not a hit, and the decoded stream is: the row fails.
#[test]
fn a_fragment_split_across_two_trace_segments_fails_the_row() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::keyed();
    rig.with_env("DEEPSEEK_API_KEY", KEY);
    // A session snapshot of one segment (64 KiB) and a bit: the key's first eight bytes start
    // four bytes before the first segment ends.
    rig.tell(
        "steps.sh",
        "touch \"$out/no-snapshot\"\n\
         { printf '%s' '{\"pad\":\"'; head -c 65524 /dev/zero | tr '\\0' ' '; \
         printf '%.8s' \"$DEEPSEEK_API_KEY\"; head -c 200 /dev/zero | tr '\\0' ' '; printf '%s' '\"}'; } > \"$snap\"\n",
    );
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let (result, out) = run(&rig, &client);
    let error = result.expect_err(&format!("a split fragment fails the row\n{out}"));
    assert!(matches!(error, CliError::Refused(_)), "{error:?}");
    let text = verdict(&rig);
    assert!(text.starts_with("hit\n"), "{text}");
    assert!(
        text.lines().any(|l| l.ends_with("trace/chain.jsonl")),
        "{text}"
    );

    // The decoded streams of the trace hold it. (Whether the trace's text alone would also show
    // it depends on where a flush tick cut the segments, so that is not asserted: the split case
    // itself is `key_scan_run.rs`, on exact cuts.)
    let entry = client.created().entry_id;
    let trace = rig.trace(&entry);
    let decoded = goalcup_cli::key_scan::scan_trace(&matcher(), &trace);
    assert!(
        matches!(decoded, ScanOutcome::Hit { .. }),
        "the decoded streams of the trace show the fragment: {decoded}"
    );
}

/// End to end: a harness that never prints its identity ends the run with exactly one
/// deferred-scope gap and no native stream, and the finish request carried no scope.
#[test]
fn no_marker_means_one_gap_and_a_finish_with_no_scope() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::keyed();
    rig.with_env("DEEPSEEK_API_KEY", KEY);
    rig.tell("no-marker", "");
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let (result, out) = run(&rig, &client);
    result.unwrap_or_else(|e| panic!("{e}\n{out}"));
    assert_eq!(client.book().seals[0].native_scope, None);
    let (report, records) = rig.read_trace(&client.created().entry_id);
    assert!(native_labels(&records).is_empty());
    assert_eq!(gaps(&records, GapReason::SourceChanged), 1, "{records:#?}");
    assert!(report.coverage_incomplete);
    assert_eq!(verdict(&rig).lines().next(), Some("clean"), "no key leaked");
}

/// Two different identities are a conflict, never resolved by picking one.
#[test]
fn two_conflicting_markers_send_no_scope() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::keyed();
    rig.with_env("DEEPSEEK_API_KEY", KEY);
    rig.tell(
        "steps.sh",
        "echo 'session_id: 20261002_000000_000001' >&2\n",
    );
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let (result, out) = run(&rig, &client);
    result.unwrap_or_else(|e| panic!("{e}\n{out}"));
    assert_eq!(client.book().seals[0].native_scope, None);
    let (_, records) = rig.read_trace(&client.created().entry_id);
    assert!(native_labels(&records).is_empty());
    assert_eq!(gaps(&records, GapReason::SourceChanged), 1, "{records:#?}");
}

/// A daemon that does not know the scope field refuses the finish as malformed. The
/// row is not clean, the verdict says the scan did not complete, and the standalone seal that
/// follows (which sends no scope) finishes the run with the deferred gap.
#[test]
fn a_finish_the_daemon_refuses_leaves_an_incomplete_verdict_and_a_standalone_seal_finishes() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::keyed();
    rig.with_env("DEEPSEEK_API_KEY", KEY);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    client.book().seal_fault = Some(|| {
        CaptureSealError::MaybeAdmitted(ControlError::Malformed(
            "unknown field `native_scope`".into(),
        ))
    });
    let (result, _out) = run(&rig, &client);
    let error = result.expect_err("the finish was refused");
    assert!(
        matches!(error, CliError::Refused(_) | CliError::Control(_)),
        "{error:?}"
    );
    assert_eq!(
        client.book().seals[0].native_scope.as_deref(),
        Some(session_id(&rig).as_str())
    );
    let text = verdict(&rig);
    assert!(text.starts_with("incomplete\n"), "{text}");
    assert!(!text.starts_with("clean"));

    let entry = client.created().entry_id;
    http.seal_capture(
        &entry,
        &CaptureSealRequest {
            proxy: None,
            remaining_finish_ms: None,
            native_scope: None,
        },
        Duration::from_secs(60),
    )
    .expect("the standalone seal commits");
    let (_, records) = rig.read_trace(&entry);
    assert!(native_labels(&records).is_empty());
    assert_eq!(gaps(&records, GapReason::SourceChanged), 1, "{records:#?}");
}

/// The CLI's half: a keyless `stdout_json` finish never carries a scope, and writes no key
/// verdict (it holds no provider key to scan for).
#[test]
fn a_keyless_stdout_identity_finish_never_carries_a_scope_or_a_verdict() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::stdout_identity();
    rig.tell("stop", "");
    let goal = rig.goal(GOAL);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let (result, out) =
        std::thread::scope(|s| joined(rig.start(s, &client, rig.options(&goal, RESERVE), None)));
    result.unwrap_or_else(|e| panic!("{e}\n{out}"));
    let seals = client.book().seals.clone();
    assert_eq!(seals.len(), 1);
    assert_eq!(seals[0].native_scope, None);
    assert!(!run_dir(&rig).join("key-scan.txt").exists());
}

/// Copy a trace directory to a fresh place the test may damage.
fn copy_trace(from: &std::path::Path) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let to = dir.path().join("trace");
    std::fs::create_dir(&to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        if entry.file_type().unwrap().is_file() {
            std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
        }
    }
    (dir, to)
}

/// The verifier stops reading a chain at the first bad
/// record and still answers `Ok`, so a decoded scan that only checks for an error calls a trace it
/// read a prefix of `clean`. Whatever stops the read before the end of `chain.jsonl` is
/// `incomplete`: a torn tail, a malformed line, a line over the envelope bound.
#[test]
fn a_trace_the_verifier_stops_reading_partway_scans_incomplete_never_clean() {
    use goalcup_cli::key_scan::scan_trace;
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::keyed();
    rig.with_env("DEEPSEEK_API_KEY", KEY);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let (result, out) = run(&rig, &client);
    result.unwrap_or_else(|e| panic!("{e}\n{out}"));
    let trace = rig.trace(&client.created().entry_id);
    let m = matcher();

    // The control: the undamaged trace is read to its end and is clean.
    let (_keep, whole) = copy_trace(&trace);
    assert!(
        scan_trace(&m, &whole).is_clean(),
        "{}",
        scan_trace(&m, &whole)
    );

    let chain = std::fs::read(trace.join("chain.jsonl")).unwrap();
    let lines: Vec<&[u8]> = chain.split_inclusive(|b| *b == b'\n').collect();
    assert!(lines.len() > 4, "{} lines", lines.len());
    let damaged = |name: &str, bytes: Vec<u8>| {
        let (keep, copy) = copy_trace(&trace);
        std::fs::write(copy.join("chain.jsonl"), bytes).unwrap();
        let outcome = scan_trace(&m, &copy);
        assert!(
            matches!(outcome, ScanOutcome::Incomplete { .. }),
            "{name}: {outcome}"
        );
        drop(keep);
    };
    // A torn tail: an unterminated line after the last record.
    let mut torn = chain.clone();
    torn.extend_from_slice(b"{\"torn");
    damaged("torn tail", torn);
    // A malformed line in the middle: everything after it is unread.
    let mut bad = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if i == 3 {
            bad.extend_from_slice(b"this is not an envelope\n");
        }
        bad.extend_from_slice(line);
    }
    damaged("malformed line", bad);
    // A line over the envelope bound.
    let mut huge = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if i == 3 {
            huge.extend(std::iter::repeat_n(b'x', 3 * 1024 * 1024));
            huge.push(b'\n');
        }
        huge.extend_from_slice(line);
    }
    damaged("oversized line", huge);
}

/// A run that ends before its finish (here, a registration the daemon
/// refuses, so the parked harness never runs) still leaves a verdict, and it is `incomplete`: a
/// retention step that finds no file, or any word but `clean`, retains nothing.
#[test]
fn a_run_that_ends_before_its_finish_leaves_an_incomplete_verdict() {
    use m2_cli_support::RegistrationFault;
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::keyed();
    rig.with_env("DEEPSEEK_API_KEY", KEY);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    client.book().registration = Some(RegistrationFault::Refused);
    let (result, _out) = run(&rig, &client);
    result.expect_err("the registration was refused");
    assert!(!rig.harness_started(), "the parked harness never ran");
    let text = verdict(&rig);
    assert!(text.starts_with("incomplete\n"), "{text}");
}

/// A verdict that cannot be written fails the row, so a clean run is never
/// reported by a command that left no file saying so.
#[test]
fn a_verdict_that_cannot_be_written_fails_the_row() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::keyed();
    rig.with_env("DEEPSEEK_API_KEY", KEY);
    // A directory where the verdict file goes: the write is refused.
    rig.tell("steps.sh", "mkdir \"$KEYED_HOME/../key-scan.txt\"\n");
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let (result, out) = run(&rig, &client);
    let error = result.expect_err(&format!("an unwritten verdict fails the row\n{out}"));
    assert!(matches!(error, CliError::Refused(_)), "{error:?}");
    assert!(error.to_string().contains("key-scan.txt"), "{error}");
}

/// A stdout that goes away at the seal report: every write before the report succeeds, and the
/// first write of the report's first line, and every one after it, fails with a broken pipe.
struct StdoutGoneAtTheSealReport(std::sync::Arc<std::sync::Mutex<String>>);

impl std::io::Write for StdoutGoneAtTheSealReport {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let mut seen = self.0.lock().unwrap();
        if seen.contains("finished locally")
            || String::from_utf8_lossy(bytes).contains("finished locally")
        {
            seen.push_str("<broken>");
            return Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe));
        }
        seen.push_str(&String::from_utf8_lossy(bytes));
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A seal that has committed stays committed when this command's own stdout is gone (a pager
/// quit, a closed pipe): the run is reported as it ended, its trace is sealed, and the key scan
/// reads the trace it was always going to read, not `incomplete` (CONTRACTS: this command's own
/// output failing ends nothing).
#[test]
fn a_committed_seal_stays_committed_when_stdout_fails() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::keyed();
    rig.with_env("DEEPSEEK_API_KEY", KEY);
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let goal = rig.goal(GOAL);
    let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let result = std::thread::scope(|s| {
        joined(rig.start_writing(
            s,
            &client,
            rig.keyed_options(&goal, RESERVE),
            StdoutGoneAtTheSealReport(seen.clone()),
        ))
    });
    let shown = seen.lock().unwrap().clone();
    assert!(
        shown.contains("<broken>"),
        "the writer was never cut: {shown}"
    );
    let outcome =
        result.unwrap_or_else(|e| panic!("a sealed run exits as the harness did: {e}\n{shown}"));
    assert!(outcome.exit_status.expect("a harness ran").success());

    let entry = client.created().entry_id;
    let (report, _records) = rig.read_trace(&entry);
    assert!(!report.coverage_incomplete, "{:?}", report.findings);
    let text = verdict(&rig);
    assert_eq!(text.lines().next(), Some("clean"), "{text}");
    assert!(
        rig.trace(&entry).join("seal.json").is_file(),
        "the committed seal is on disk"
    );
}

/// A keyed harness that prints a masked form of its key on stderr: its output is tee'd and
/// scanned live, and the fragment fails the row.
#[test]
fn a_masked_key_on_the_harness_stderr_is_a_live_hit() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::keyed();
    rig.with_env("DEEPSEEK_API_KEY", KEY);
    // The harness prints a masked form of its key on stderr: the live scan must see it.
    rig.tell(
        "steps.sh",
        "tail4=$(printf %s \"$DEEPSEEK_API_KEY\" | tail -c 4)\n\
         printf 'echo %.8s***%s\\n' \"$DEEPSEEK_API_KEY\" \"$tail4\" >&2\n",
    );
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let (result, out) = run(&rig, &client);
    let error = result.expect_err(&format!("the live fragment fails the row\n{out}"));
    assert!(matches!(error, CliError::Refused(_)), "{error:?}");
    let text = verdict(&rig);
    assert!(text.starts_with("hit\n"), "{text}");
    assert!(text.contains("harness's own output"), "{text}");
}

/// A goal file in a synced folder is refused before anything is created.
#[test]
fn a_goal_file_in_a_synced_folder_refuses_before_anything_is_created() {
    let _lock = table_lock::TableLock::acquire();
    let rig = Rig::keyed();
    rig.with_env("DEEPSEEK_API_KEY", KEY);
    let dir = rig.root.join("Dropbox/goals");
    std::fs::create_dir_all(&dir).unwrap();
    let goal = dir.join("goal.txt");
    std::fs::write(&goal, GOAL).unwrap();
    let http = rig.client();
    let client = Hooked::new(&http, &rig);
    let (result, _out) = std::thread::scope(|s| {
        joined(rig.start(s, &client, rig.keyed_options(&goal, RESERVE), None))
    });
    let error = result.expect_err("a synced folder is refused");
    assert!(matches!(error, CliError::Refused(_)), "{error:?}");
    assert!(error.to_string().contains("synced"), "{error}");
    assert!(client.names().is_empty(), "{:?}", client.names());
    assert!(!rig.harness_started());
}
