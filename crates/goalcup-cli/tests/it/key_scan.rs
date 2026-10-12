//! The key-fragment scanner (D41). Every key here is synthetic.
//!
//! The oracle for each form is built here from the key by hand, and base64 is encoded by the
//! `base64` crate over a stream the form is embedded in, so the scanner's own alignment
//! arithmetic is checked against a real encoder and not against itself.

use crate::common::secret_support;
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE, URL_SAFE_NO_PAD};
use goalcup_cli::CliError;
use goalcup_cli::key_scan::{FragmentMatcher, ScanOutcome, StreamScan, scan_tree};
use goalcup_protocol::capture::StreamKey;
use secret_support::KEY;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

/// The separators a mask is enumerated with.
const SEPARATORS: [&str; 4] = ["...", "\u{2026}", "***", "\\u2026"];

fn matcher() -> FragmentMatcher {
    FragmentMatcher::new(KEY.as_bytes()).expect("a usable key")
}

fn fingerprint() -> String {
    Sha256::digest(KEY.as_bytes())[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Every plain form, named, derived here and not by the scanner.
fn plain_forms() -> Vec<(String, Vec<u8>)> {
    let n = KEY.len();
    let mut forms = vec![
        ("full".to_string(), KEY.as_bytes().to_vec()),
        ("prefix8".to_string(), KEY.as_bytes()[..8].to_vec()),
        ("suffix8".to_string(), KEY.as_bytes()[n - 8..].to_vec()),
        ("sha256-16".to_string(), fingerprint().into_bytes()),
        (
            "SHA256-16".to_string(),
            fingerprint().to_uppercase().into_bytes(),
        ),
    ];
    for prefix in [4usize, 6, 8] {
        for sep in SEPARATORS {
            forms.push((
                format!("mask{prefix}{sep}"),
                format!("{}{sep}{}", &KEY[..prefix], &KEY[n - 4..]).into_bytes(),
            ));
        }
    }
    forms
}

/// Shapes only the window rule covers: other separators and widths between the first four
/// and the last four bytes, including a three-byte gap.
fn window_forms() -> Vec<String> {
    let n = KEY.len();
    vec![
        format!("{}~~~{}", &KEY[..4], &KEY[n - 4..]),
        format!("{}{}", &KEY[..4], &KEY[n - 4..]),
        format!("{}0123456789abcdef{}", &KEY[..4], &KEY[n - 4..]),
        format!("{}?????{}", &KEY[..8], &KEY[n - 4..]),
        format!("{} [masked] {}", &KEY[..4], &KEY[n - 4..]),
    ]
}

fn write(root: &Path, relative: &str, bytes: &[u8]) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn tree() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("run");
    fs::create_dir(&root).unwrap();
    (dir, root)
}

fn hit_paths(outcome: ScanOutcome) -> Vec<String> {
    match outcome {
        ScanOutcome::Hit { relative_paths } => relative_paths
            .into_iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
        other => panic!("expected a hit, got {other:?}"),
    }
}

const BOUND: u64 = 64 << 20;

/// Plain: each form planted in a nested file among ordinary bytes names that file
/// and only that file.
#[test]
fn every_plain_form_is_found_in_a_nested_file() {
    let m = matcher();
    let forms: Vec<(String, Vec<u8>)> = plain_forms()
        .into_iter()
        .chain(
            window_forms()
                .into_iter()
                .map(|f| (format!("window {f}"), f.into_bytes())),
        )
        .collect();
    for (name, form) in forms {
        let (_keep, root) = tree();
        write(&root, "a/clean.txt", b"nothing to see here\n");
        let mut body = b"{\"x\":\"some ordinary text ".to_vec();
        body.extend_from_slice(&form);
        body.extend_from_slice(b" and more\"}\n");
        write(&root, "hermes/sessions/nested/dump.json", &body);
        write(&root, "z/other.txt", b"also clean\n");
        assert_eq!(
            hit_paths(scan_tree(&m, &[&root], BOUND)),
            ["run/hermes/sessions/nested/dump.json"],
            "{name}"
        );
    }
}

/// Encoded: every form inside a base64 segment, in both alphabets, at all three
/// alignments and every tail length, the way `data_b64` carries it in the chain.
#[test]
fn every_form_is_found_inside_base64_at_every_alignment() {
    let m = matcher();
    let mut forms = plain_forms();
    forms.extend(
        window_forms()
            .into_iter()
            .map(|f| (format!("window {f}"), f.into_bytes())),
    );
    for (name, form) in forms {
        let is_window = name.starts_with("window");
        for lead in 0..3usize {
            for tail in 0..3usize {
                let mut stream = vec![0xC7u8; lead + 1];
                stream.extend_from_slice(&form);
                stream.extend(std::iter::repeat_n(0x3Cu8, tail + 1));
                // The unknown bytes on both sides are not padding-aligned on purpose.
                for (alphabet, encoded) in [
                    ("std", STANDARD.encode(&stream)),
                    ("url", URL_SAFE.encode(&stream)),
                    ("url-nopad", URL_SAFE_NO_PAD.encode(&stream)),
                ] {
                    let (_keep, root) = tree();
                    let line =
                        format!("{{\"type\":\"segment\",\"data_b64\":\"{encoded}\",\"part\":0}}\n");
                    write(&root, "trace/chain.jsonl", line.as_bytes());
                    let outcome = scan_tree(&m, &[&root], BOUND);
                    // The window rule is for plain text; its base64 would need the gap
                    // bytes to align, so only the plain forms are claimed here.
                    if is_window {
                        continue;
                    }
                    assert_eq!(
                        hit_paths(outcome),
                        ["run/trace/chain.jsonl"],
                        "{name} lead {lead} tail {tail} {alphabet}"
                    );
                }
            }
        }
    }
}

/// A clean tree is clean, including every near miss the design names:
/// a bare four-character suffix, a seven-character prefix, a short digest, a mask with the
/// wrong tail. (The eight-byte prefix is a form in its own right, so the near miss is a
/// six-byte one.)
#[test]
fn a_clean_tree_and_its_near_misses_are_clean() {
    let m = matcher();
    let n = KEY.len();
    let (_keep, root) = tree();
    write(&root, "a.txt", b"hello world\n");
    write(&root, "deep/er/b.json", br#"{"model":"deepseek-v4-flash"}"#);
    write(&root, "suffix4.txt", &KEY.as_bytes()[n - 4..]);
    write(
        &root,
        "suffix4-in-text.txt",
        format!("the id ends {} here", &KEY[n - 4..]).as_bytes(),
    );
    write(&root, "prefix7.txt", &KEY.as_bytes()[..7]);
    write(&root, "digest15.txt", &fingerprint().into_bytes()[..15]);
    write(
        &root,
        "wrong-tail.txt",
        format!("{}...{}", &KEY[..6], "zzzz").as_bytes(),
    );
    write(
        &root,
        "far-tail.txt",
        format!("{}{}{}", &KEY[..4], ".".repeat(40), &KEY[n - 4..]).as_bytes(),
    );
    write(&root, "binary.bin", &[0u8, 255, 1, 254, 2, 253]);
    write(&root, "empty.txt", b"");
    assert_eq!(scan_tree(&m, &[&root], BOUND), ScanOutcome::Clean);
}

/// What cannot be read is not clean. A symlink, an unreadable file, a special file
/// and a tree over the byte bound are each `Incomplete`, and a hit still wins over them.
#[test]
fn an_unreadable_symlinked_special_or_oversized_tree_is_incomplete() {
    use std::os::unix::fs::PermissionsExt;
    let m = matcher();

    // A symlink.
    let (_keep, root) = tree();
    write(&root, "real.txt", b"clean\n");
    std::os::unix::fs::symlink(root.join("real.txt"), root.join("link.txt")).unwrap();
    assert!(
        matches!(scan_tree(&m, &[&root], BOUND), ScanOutcome::Incomplete { reason } if reason.contains("symlink"))
    );

    // A symlink that points outside the tree is never read.
    let (keep, root) = tree();
    let outside = keep.path().join("outside.txt");
    fs::write(&outside, KEY).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("escape.txt")).unwrap();
    assert!(matches!(
        scan_tree(&m, &[&root], BOUND),
        ScanOutcome::Incomplete { .. }
    ));

    // An unreadable file.
    let (_keep, root) = tree();
    write(&root, "secret.txt", b"clean\n");
    fs::set_permissions(root.join("secret.txt"), fs::Permissions::from_mode(0o000)).unwrap();
    let readable_anyway = fs::read(root.join("secret.txt")).is_ok();
    let outcome = scan_tree(&m, &[&root], BOUND);
    fs::set_permissions(root.join("secret.txt"), fs::Permissions::from_mode(0o600)).unwrap();
    if !readable_anyway {
        assert!(
            matches!(outcome, ScanOutcome::Incomplete { .. }),
            "{outcome:?}"
        );
    }

    // An unreadable directory.
    let (_keep, root) = tree();
    write(&root, "locked/inside.txt", b"clean\n");
    fs::set_permissions(root.join("locked"), fs::Permissions::from_mode(0o000)).unwrap();
    let readable_anyway = fs::read_dir(root.join("locked")).is_ok();
    let outcome = scan_tree(&m, &[&root], BOUND);
    fs::set_permissions(root.join("locked"), fs::Permissions::from_mode(0o700)).unwrap();
    if !readable_anyway {
        assert!(
            matches!(outcome, ScanOutcome::Incomplete { .. }),
            "{outcome:?}"
        );
    }

    // A special file, never opened (a FIFO would block a read).
    let (_keep, root) = tree();
    write(&root, "ok.txt", b"clean\n");
    let made = std::process::Command::new("mkfifo")
        .arg(root.join("pipe"))
        .status()
        .unwrap();
    assert!(made.success());
    assert!(matches!(
        scan_tree(&m, &[&root], BOUND),
        ScanOutcome::Incomplete { .. }
    ));

    // The byte bound: a tree one byte over it, and the same tree at the bound.
    let (_keep, root) = tree();
    write(&root, "a.txt", &[b'a'; 600]);
    write(&root, "b.txt", &[b'b'; 600]);
    assert!(matches!(
        scan_tree(&m, &[&root], 1199),
        ScanOutcome::Incomplete { reason } if reason.contains("bound")
    ));
    assert_eq!(scan_tree(&m, &[&root], 1200), ScanOutcome::Clean);

    // A hit is reported even where the scan is also incomplete.
    let (_keep, root) = tree();
    write(&root, "dump.json", KEY.as_bytes());
    std::os::unix::fs::symlink(root.join("dump.json"), root.join("link")).unwrap();
    assert_eq!(hit_paths(scan_tree(&m, &[&root], BOUND)), ["run/dump.json"]);
}

/// Two roots (the run directory and the trace directory) are both scanned, and a hit names
/// the root it was under.
#[test]
fn several_roots_are_scanned_and_a_hit_names_its_root() {
    let m = matcher();
    let keep = tempfile::tempdir().unwrap();
    let run = keep.path().join("run");
    let trace = keep.path().join("trace");
    fs::create_dir(&run).unwrap();
    fs::create_dir(&trace).unwrap();
    write(&run, "a.txt", b"clean\n");
    write(
        &trace,
        "chain.jsonl",
        &[b"{\"k\":\"", KEY.as_bytes(), b"\"}"].concat(),
    );
    assert_eq!(
        hit_paths(scan_tree(&m, &[&run, &trace], BOUND)),
        ["trace/chain.jsonl"]
    );
    assert_eq!(scan_tree(&m, &[&run], BOUND), ScanOutcome::Clean);
}

/// Nothing the matcher or an outcome prints contains the key or any form of it, and
/// the refusal of an unusable key never echoes it.
#[test]
fn debug_and_display_never_contain_a_form() {
    let m = matcher();
    let (_keep, root) = tree();
    write(&root, "dump.json", KEY.as_bytes());
    let hit = scan_tree(&m, &[&root], BOUND);
    let clean = ScanOutcome::Clean;
    let incomplete = ScanOutcome::Incomplete { reason: "x" };

    let mut shown = vec![format!("{m:?}")];
    for outcome in [&hit, &clean, &incomplete] {
        shown.push(format!("{outcome:?}"));
        shown.push(format!("{outcome}"));
        shown.push(outcome.verdict().to_string());
    }
    let mut forms: Vec<Vec<u8>> = plain_forms().into_iter().map(|(_, f)| f).collect();
    forms.extend(window_forms().into_iter().map(String::into_bytes));
    for form in forms {
        let encoded = [
            STANDARD.encode(&form),
            URL_SAFE_NO_PAD.encode(&form),
            String::from_utf8_lossy(&form).into_owned(),
        ];
        for text in &shown {
            for needle in &encoded {
                // Forms shorter than eight bytes cannot appear (none is), and a verdict word
                // is never a substring of one.
                assert!(!text.contains(needle.as_str()), "{text:?} contains a form");
            }
        }
    }
    assert!(!format!("{m:?}").contains(KEY));

    for bad in [
        &b"short"[..],
        b"has a space in it, twenty+ bytes",
        "caf\u{e9}-caf\u{e9}-caf\u{e9}-caf\u{e9}".as_bytes(),
    ] {
        let error = FragmentMatcher::new(bad).expect_err("an unusable key");
        assert!(matches!(error, CliError::Refused(_)));
        assert!(
            !error
                .to_string()
                .contains(&String::from_utf8_lossy(bad).into_owned()),
            "the refusal echoed the key: {error}"
        );
    }
}

/// A stream scan finds a form wherever its segments split, and a clean stream stays clean. One
/// form, every split, three segments.
#[test]
fn a_form_split_across_chunks_is_found() {
    let stream = StreamKey {
        producer: "producer-1".to_string(),
        stream: 1,
    };
    let forms: Vec<Vec<u8>> = plain_forms()
        .into_iter()
        .map(|(_, f)| f)
        .chain(window_forms().into_iter().map(String::into_bytes))
        .collect();
    let m = matcher();
    for form in forms {
        let mut input = b"prefix bytes ".to_vec();
        input.extend_from_slice(&form);
        input.extend_from_slice(b" suffix bytes");
        for first in 0..=input.len() {
            for second in first..=input.len() {
                let mut scan = StreamScan::new(&m);
                let hits = [&input[..first], &input[first..second], &input[second..]]
                    .map(|chunk| scan.segment(&stream, chunk));
                assert!(
                    hits.iter().any(|h| *h) && scan.hit(),
                    "{:?} split at {first}/{second}",
                    String::from_utf8_lossy(&form)
                );
            }
        }
    }
    let mut scan = StreamScan::new(&m);
    for chunk in [&b"nothing "[..], b"here ", b"at all"] {
        assert!(!scan.segment(&stream, chunk));
    }
    assert!(!scan.hit());
    scan.segment(&stream, KEY.as_bytes());
    assert!(scan.hit());
    // Another stream's bytes are never joined to this one's: a form cut across two streams is
    // not a form.
    let other = StreamKey {
        producer: "producer-1".to_string(),
        stream: 2,
    };
    let mask = format!("{}...{}", &KEY[..6], &KEY[KEY.len() - 4..]);
    let (head, tail) = mask.as_bytes().split_at(8);
    let mut scan = StreamScan::new(&m);
    assert!(!scan.segment(&stream, head));
    assert!(!scan.segment(&other, tail));
    assert!(!scan.hit());
    let mut scan = StreamScan::new(&m);
    assert!(!scan.segment(&stream, head));
    assert!(scan.segment(&stream, tail), "the same stream joins them");
}

/// A key too short to scan for safely, or not one printable token, is refused.
#[test]
fn an_unusable_key_is_refused() {
    assert!(FragmentMatcher::new(&[b'a'; 19]).is_err());
    assert!(FragmentMatcher::new(&[b'a'; 20]).is_ok());
    assert!(FragmentMatcher::new(b"aaaaaaaaaa aaaaaaaaaaaa").is_err());
    assert!(FragmentMatcher::new(b"aaaaaaaaaa\naaaaaaaaaaaa").is_err());
}

/// A scan that had nothing to scan is not a clean scan. An empty root
/// list, an empty directory and a tree of empty directories are `Incomplete`, so a caller
/// that built its roots wrongly cannot read a pass out of it.
#[test]
fn a_scan_with_nothing_to_scan_is_not_clean() {
    let m = matcher();
    assert!(matches!(
        scan_tree(&m, &[], BOUND),
        ScanOutcome::Incomplete { reason } if reason.contains("nothing")
    ));
    let (_keep, root) = tree();
    assert!(matches!(
        scan_tree(&m, &[&root], BOUND),
        ScanOutcome::Incomplete { reason } if reason.contains("nothing")
    ));
    fs::create_dir_all(root.join("a/b/c")).unwrap();
    assert!(matches!(
        scan_tree(&m, &[&root], BOUND),
        ScanOutcome::Incomplete { .. }
    ));
    // One empty file is something that was read, and read clean.
    write(&root, "a/empty.txt", b"");
    assert_eq!(scan_tree(&m, &[&root], BOUND), ScanOutcome::Clean);
}

/// `Clean` is the only outcome that is clean, and says so by name, so a
/// caller does not have to write `!matches!(outcome, Hit { .. })` and read `Incomplete` as a
/// pass.
#[test]
fn only_clean_is_clean() {
    assert!(ScanOutcome::Clean.is_clean());
    assert!(!ScanOutcome::Incomplete { reason: "x" }.is_clean());
    assert!(
        !ScanOutcome::Hit {
            relative_paths: vec![PathBuf::from("a")]
        }
        .is_clean()
    );
}
