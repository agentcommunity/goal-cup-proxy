//! The key-fragment scan of a whole run, and the verdict it leaves.
//!
//! A mask split across two segment records of one stream is
//! invisible to a scan of the trace file's text, because each record's base64 holds only part
//! of it. The run scan therefore decodes each stream's segments in order and scans the
//! concatenation. Every key here is synthetic.

mod secret_support;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use goalcup_cli::key_scan::{
    Binding, FragmentMatcher, ScanOutcome, StreamScan, VerdictCheck, check_verdict, scan_run,
    scan_tree, write_verdict, write_verdict_bound,
};
use goalcup_protocol::capture::StreamKey;
use secret_support::KEY;
use std::fs;
use std::path::{Path, PathBuf};

fn matcher() -> FragmentMatcher {
    FragmentMatcher::new(KEY.as_bytes()).unwrap()
}

fn stream(producer: &str, n: u64) -> StreamKey {
    StreamKey {
        producer: producer.to_string(),
        stream: n,
    }
}

/// A 6/4 mask of the key, one of the shapes a 401 echo can take.
fn mask() -> String {
    format!("{}...{}", &KEY[..6], &KEY[KEY.len() - 4..])
}

/// The review's reproduction: a mask cut at bytes 8, 10 and 12 across two segments. Each
/// segment's base64 holds only part of it, so the text of the trace is clean, and the decoded
/// stream is not.
#[test]
fn a_mask_split_across_two_segments_is_found_in_the_decoded_stream_only() {
    let lead = "..{\"echo\":\"";
    let whole = format!("{lead}{}\"}}..", mask());
    let inside = lead.len() + 1..lead.len() + mask().len();
    for cut in 1..whole.len() {
        let (a, b) = whole.as_bytes().split_at(cut);
        // What the trace file holds: each segment record's own base64.
        let text = format!(
            "{{\"data_b64\":\"{}\"}}\n{{\"data_b64\":\"{}\"}}\n",
            STANDARD.encode(a),
            STANDARD.encode(b)
        );
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("trace");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("chain.jsonl"), &text).unwrap();
        let m = matcher();
        let by_text = scan_tree(&m, &[&root], 1 << 20);
        let mut scan = StreamScan::new(&m);
        let s = stream("p", 1);
        scan.segment(&s, a);
        scan.segment(&s, b);
        assert!(
            scan.hit(),
            "cut at {cut}: the decoded stream holds the mask"
        );
        // Whenever the cut falls inside the mask, no segment holds all of it and the text of
        // the trace is clean. That is the case this scan exists for.
        assert_eq!(
            by_text.is_clean(),
            inside.contains(&cut),
            "cut at {cut}: text scan {by_text}"
        );
    }
}

/// Other streams' segments between two of a stream's do not break its carry, and two streams
/// never form a match between them.
#[test]
fn segments_of_one_stream_are_joined_across_other_streams_and_never_across_streams() {
    let whole = mask();
    let (a, b) = whole.as_bytes().split_at(7);
    let m = matcher();

    let mut scan = StreamScan::new(&m);
    let (one, two) = (stream("p", 1), stream("p", 2));
    assert!(!scan.segment(&one, a));
    assert!(!scan.segment(&two, b"unrelated bytes from another stream"));
    assert!(scan.segment(&one, b), "the carry survived the other stream");
    assert!(scan.hit());

    // The halves in different streams are not one mask.
    let mut scan = StreamScan::new(&m);
    scan.segment(&one, a);
    scan.segment(&two, b);
    assert!(!scan.hit(), "two streams were joined");

    // The same stream id under another producer is another stream.
    let mut scan = StreamScan::new(&m);
    scan.segment(&stream("p", 1), a);
    scan.segment(&stream("q", 1), b);
    assert!(!scan.hit());
}

/// A clean stream, however it is cut, never hits, and a hit is sticky.
#[test]
fn a_clean_stream_is_clean_and_a_hit_is_sticky() {
    let m = matcher();
    let mut scan = StreamScan::new(&m);
    let s = stream("p", 1);
    let text = "an ordinary answer with sk-abcdefgh and a short sk-xxxx...yyyy mask".repeat(50);
    for chunk in text.as_bytes().chunks(13) {
        assert!(!scan.segment(&s, chunk));
    }
    assert!(!scan.hit());
    assert!(scan.segment(&s, KEY.as_bytes()));
    scan.segment(&s, &[b'.'; 4096]);
    assert!(scan.hit(), "a hit stays");
}

fn run_tree() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let run = dir.path().join("goalcup-run");
    let trace = dir.path().join("trace");
    for sub in [
        run.join("hermes/sessions"),
        run.join("home"),
        run.join("workspace"),
        trace.clone(),
    ] {
        fs::create_dir_all(sub).unwrap();
    }
    fs::write(run.join("hermes/config.yaml"), "sessions: {}\n").unwrap();
    fs::write(trace.join("chain.jsonl"), "{}\n").unwrap();
    (dir, run, trace)
}

fn write(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

/// The scan half: a planted fragment in a request dump is a hit that names the
/// file relative to its root and nothing else; the same tree without it is clean.
#[test]
fn a_planted_request_dump_is_a_hit_and_a_clean_run_is_clean() {
    let (_dir, run, trace) = run_tree();
    let m = matcher();
    let clean = scan_tree(&m, &[&run, &trace], 1 << 20);
    assert!(clean.is_clean(), "{clean}");

    write(
        &run.join("hermes/sessions/request_dump_x.json"),
        format!(
            "{{\"headers\":{{\"Authorization\":\"Bearer {}\"}}}}",
            mask()
        )
        .as_bytes(),
    );
    match scan_tree(&m, &[&run, &trace], 1 << 20) {
        ScanOutcome::Hit { relative_paths } => {
            let shown: Vec<String> = relative_paths
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            assert_eq!(shown, ["goalcup-run/hermes/sessions/request_dump_x.json"]);
        }
        other => panic!("{other}"),
    }
}

/// The run scan over a trace directory that is not a v5 trace is not clean: a scan that could
/// not read the trace says so, and is never read as a pass.
#[test]
fn a_trace_that_cannot_be_read_is_incomplete_never_clean() {
    let (_dir, run, trace) = run_tree();
    let m = matcher();
    // `chain.jsonl` here is `{}`: not a protocol-v5 chain.
    let outcome = scan_run(&m, &[&run], &trace, 1 << 20);
    assert!(!outcome.is_clean(), "{outcome}");
    assert!(
        matches!(outcome, ScanOutcome::Incomplete { .. }),
        "{outcome}"
    );
    // A missing trace directory is the same.
    let outcome = scan_run(&m, &[&run], &run.join("no-such-trace"), 1 << 20);
    assert!(
        matches!(outcome, ScanOutcome::Incomplete { .. }),
        "{outcome}"
    );
    // An empty list of roots to scan is not a pass either.
    let outcome = scan_run(&m, &[], &trace, 1 << 20);
    assert!(!outcome.is_clean(), "{outcome}");
}

/// A hit anywhere wins over an incomplete scan elsewhere: both fail the row, and a hit names
/// its file.
#[test]
fn a_hit_in_the_run_wins_over_an_unreadable_trace() {
    let (_dir, run, trace) = run_tree();
    write(&run.join("home/leak.txt"), mask().as_bytes());
    let m = matcher();
    match scan_run(&m, &[&run], &trace, 1 << 20) {
        ScanOutcome::Hit { relative_paths } => {
            assert!(
                relative_paths.iter().any(|p| p.ends_with("home/leak.txt")),
                "{relative_paths:?}"
            );
        }
        other => panic!("{other}"),
    }
}

/// The verdict file says one word and relative paths, mode 0600, and never a byte of a form.
#[test]
fn the_verdict_file_holds_a_word_and_relative_paths_only() {
    let (_dir, run, _trace) = run_tree();
    let read = || fs::read_to_string(run.join("key-scan.txt")).unwrap();

    write_verdict(&run, &ScanOutcome::Clean).unwrap();
    assert_eq!(read(), "clean\n");

    let hit = ScanOutcome::Hit {
        relative_paths: vec![
            PathBuf::from("goalcup-run/hermes/sessions/request_dump_x.json"),
            PathBuf::from("trace/chain.jsonl"),
        ],
    };
    write_verdict(&run, &hit).unwrap();
    assert_eq!(
        read(),
        "hit\ngoalcup-run/hermes/sessions/request_dump_x.json\ntrace/chain.jsonl\n"
    );

    let incomplete = ScanOutcome::Incomplete {
        reason: "a symlink was not followed",
    };
    write_verdict(&run, &incomplete).unwrap();
    assert_eq!(read(), "incomplete\na symlink was not followed\n");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(run.join("key-scan.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
    for text in [read(), format!("{hit}"), format!("{hit:?}")] {
        assert!(!text.contains(KEY) && !text.contains(&KEY[..8]), "{text}");
    }
}

/// H-18a: a `clean` verdict is bound to what was scanned. It stays a first line that scripts read
/// as before, and `check_verdict` recomputes the trace head and the scanned file list: any change
/// after the scan makes the verdict stale, and a stale verdict never reads clean.
#[test]
fn a_verdict_is_bound_to_the_trace_and_run_dir_it_scanned() {
    let (_dir, run, trace) = run_tree();
    let bound = Binding::capture(&run, Some(&trace));
    write_verdict_bound(&run, &ScanOutcome::Clean, &bound).unwrap();
    let text = fs::read_to_string(run.join("key-scan.txt")).unwrap();
    assert_eq!(text.lines().next(), Some("clean"), "{text}");
    assert!(text.contains("\ntrace-chain bytes=3 sha256="), "{text}");
    assert!(text.contains("\nscanned-files count="), "{text}");
    assert!(!text.contains(KEY) && !text.contains(&KEY[..8]), "{text}");

    let current = check_verdict(&run);
    assert_eq!(
        current,
        VerdictCheck::Current {
            verdict: "clean".to_string()
        },
        "{current}"
    );
    assert!(current.is_clean());

    // The scan's own verdict file is not part of what it scanned.
    write_verdict_bound(&run, &ScanOutcome::Clean, &bound).unwrap();
    assert!(check_verdict(&run).is_clean());

    // The trace changes after the scan: same length, other bytes, then a longer chain.
    fs::write(trace.join("chain.jsonl"), "{x}").unwrap();
    let stale = check_verdict(&run);
    assert!(matches!(stale, VerdictCheck::Stale { .. }), "{stale}");
    assert!(!stale.is_clean());
    assert!(stale.to_string().contains("trace"), "{stale}");
    fs::write(trace.join("chain.jsonl"), "{}\n").unwrap();
    assert!(check_verdict(&run).is_clean(), "restored, current again");
    fs::write(trace.join("chain.jsonl"), "{}\n{}\n").unwrap();
    assert!(!check_verdict(&run).is_clean());
    fs::write(trace.join("chain.jsonl"), "{}\n").unwrap();

    // A file added to, resized in or removed from the run dir after the scan.
    write(&run.join("hermes/sessions/request_dump_late.json"), b"{}");
    let stale = check_verdict(&run);
    assert!(matches!(stale, VerdictCheck::Stale { .. }), "{stale}");
    assert!(stale.to_string().contains("file list"), "{stale}");
    fs::remove_file(run.join("hermes/sessions/request_dump_late.json")).unwrap();
    assert!(check_verdict(&run).is_clean());
    fs::write(run.join("hermes/config.yaml"), "sessions: {a: 1}\n").unwrap();
    assert!(!check_verdict(&run).is_clean());
    fs::write(run.join("hermes/config.yaml"), "sessions: {}\n").unwrap();
    fs::remove_file(run.join("hermes/config.yaml")).unwrap();
    assert!(!check_verdict(&run).is_clean());
    fs::write(run.join("hermes/config.yaml"), "sessions: {}\n").unwrap();

    // The trace directory itself gone: not clean either.
    fs::remove_dir_all(&trace).unwrap();
    assert!(!check_verdict(&run).is_clean());
}

/// A run-directory file rewritten after the scan at the very same size (a planted fragment in
/// place of the bytes that were scanned) makes the verdict stale: size alone does not bind it.
#[test]
fn a_same_size_rewrite_of_a_run_dir_file_makes_the_verdict_stale() {
    let (_dir, run, trace) = run_tree();
    let bound = Binding::capture(&run, Some(&trace));
    write_verdict_bound(&run, &ScanOutcome::Clean, &bound).unwrap();
    assert!(check_verdict(&run).is_clean());

    let file = run.join("hermes/config.yaml");
    let before = fs::read(&file).unwrap();
    // The modification time is read to the nanosecond; a pause keeps a coarse clock honest.
    std::thread::sleep(std::time::Duration::from_millis(20));
    let mut other = before.clone();
    other[0] = if other[0] == b'x' { b'y' } else { b'x' };
    assert_eq!(other.len(), before.len());
    fs::write(&file, &other).unwrap();
    let stale = check_verdict(&run);
    assert!(matches!(stale, VerdictCheck::Stale { .. }), "{stale}");
    assert!(stale.to_string().contains("file list"), "{stale}");
    assert!(!stale.is_clean());

    // The verdict's first line is unchanged by any of this.
    let text = fs::read_to_string(run.join("key-scan.txt")).unwrap();
    assert_eq!(text.lines().next(), Some("clean"), "{text}");
}

/// A verdict with no binding, no file, or a word that is not `clean` never reads as a current
/// clean verdict.
#[test]
fn an_unbound_missing_or_edited_verdict_is_never_clean() {
    let (_dir, run, trace) = run_tree();
    assert_eq!(check_verdict(&run), VerdictCheck::Missing);
    write_verdict(&run, &ScanOutcome::Clean).unwrap();
    assert_eq!(check_verdict(&run), VerdictCheck::Unbound);
    assert!(!check_verdict(&run).is_clean());

    let bound = Binding::capture(&run, Some(&trace));
    let hit = ScanOutcome::Hit {
        relative_paths: vec![PathBuf::from(
            "goalcup-run/hermes/sessions/request_dump_x.json",
        )],
    };
    write_verdict_bound(&run, &hit, &bound).unwrap();
    let text = fs::read_to_string(run.join("key-scan.txt")).unwrap();
    assert!(text.starts_with("hit\ngoalcup-run/hermes/sessions/request_dump_x.json\ntrace "));
    let check = check_verdict(&run);
    assert_eq!(
        check,
        VerdictCheck::Current {
            verdict: "hit".to_string()
        }
    );
    assert!(!check.is_clean(), "a current hit is a hit");

    // A run with no trace (the finish did not complete) is bound too.
    let none = Binding::capture(&run, None);
    write_verdict_bound(&run, &ScanOutcome::Incomplete { reason: "x" }, &none).unwrap();
    assert!(matches!(check_verdict(&run), VerdictCheck::Current { .. }));
    assert!(!check_verdict(&run).is_clean());

    // A binding that does not parse is no binding.
    fs::write(run.join("key-scan.txt"), "clean\ntrace nonsense\n").unwrap();
    assert!(!check_verdict(&run).is_clean());
}
