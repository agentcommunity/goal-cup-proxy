//! `goalcup seal`: the self-check scan, the daemon's manifest, and where the daemon
//! exported the trace.
//!
//! Three ownerships meet in this command and none of them is the CLI's. The **daemon's**
//! manifest is the record; D22 makes the kill the daemon's, because at the
//! deadline the participant's terminal is long gone; and `TraceExport` makes the trace
//! directory the daemon's, which is why this command writes nothing and there is no
//! `goalcup export`. What the CLI owns is the self-check the PRD promises: scan the
//! workspace, exclude the union the daemon excluded, and print the differences.
//!
//! Two spellings this file takes deliberately, both precedented:
//!
//! * `support::walk("crates/goalcup-cli/src")` rather than `walk("src")` — `walk` joins the
//!   repository root, so `walk("src")` would sweep a directory that does not exist and
//!   assert nothing.
//! * The entry is `m.info().entry_id` rather than a literal. `cfg_with_entry` writes the
//!   record under the id the mock's `WorkspaceInfo` carries, and the mock's seal answers
//!   under the same one, so a literal here would be a test about `read_entry`'s refusal.
//!
//! # The mirror is checked against the walker it mirrors
//!
//! Checking `scan_workspace` against a manifest somebody typed proves the mirror agrees with
//! this file and not that it agrees with `goalcup_daemon::workspace::list_workspace`. The
//! property the whole command exists for is that an HONEST run — nothing written between the
//! two walks — prints zero differences. `goalcup-daemon` is a DEV-dependency of this crate
//! for that one reason: `an_honest_run_over_one_tree_prints_no_difference_at_all` runs both
//! walkers over one tree and asserts `compare` is empty. The shipped binary links none of it,
//! which `the_shipped_binary_links_no_daemon` asserts two ways.

use crate::common::support;

use goalcup_cli::commands::seal::{dry_run_with, seal_with, verify_command};
use goalcup_cli::seal::{compare, compare_parts, exclusion_set, scan_workspace, trace_dir};
use goalcup_protocol::control::ControlError;
use goalcup_protocol::control::FinishReportV1;
use goalcup_protocol::provenance::{
    Rfc3339Utc, Sha256Digest, TerminationMethodV1, TerminationOutcomeV1,
};
use goalcup_protocol::seal::{SealManifest, SealedFile, is_excluded};
use goalcup_protocol::submission::{SubmissionCapturedV1, SubmissionOutcomeV1};
use std::path::Path;

#[test]
fn exclusion_is_the_one_shared_function_at_its_current_anchored_semantics() {
    let p = |s: &str| vec![s.to_string()];
    assert!(!is_excluded("src/main.rs", &p(".git")));
    assert!(is_excluded(".git/HEAD", &p(".git")));
    assert!(
        !is_excluded("a/b/.git/HEAD", &p(".git")),
        "root-anchored since protocol C3: `mkdir src/.git` hid a whole subtree"
    );
    assert!(
        is_excluded("a/b/.git/HEAD", &p("**/.git")),
        "a connector opts in and means it"
    );
    assert!(is_excluded(
        ".opencode/node_modules/x/y.js",
        &p(".opencode/node_modules")
    ));
    assert!(!is_excluded(
        ".opencode/plugin/goalcup-tracker.ts",
        &p(".opencode/node_modules")
    ));
    assert!(
        !is_excluded("targeted.py", &p("target")),
        "never a substring test"
    );
}

#[test]
fn the_scan_hashes_what_is_there_and_reads_a_symlink_the_way_the_daemon_reads_one() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    std::fs::create_dir_all(ws.join(".claude")).unwrap();
    std::fs::write(ws.join("hello.py"), b"print(1)\n").unwrap();
    std::fs::write(ws.join(".claude/settings.json"), b"{}").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("hello.py", ws.join("inside")).unwrap();

    let ex = exclusion_set(
        &support::connector("claude-code-cli").manifest,
        &[".claude/settings.json".to_string()],
    );
    let m = scan_workspace(ws, &ex, "entry-0001", support::NOW).unwrap();
    assert_eq!(
        (m.entry_id.as_str(), m.sealed_at.as_str()),
        ("entry-0001", support::NOW),
        "a scan discovers three of the five fields; the other two are given to it"
    );
    assert_eq!(m.workspace, ws.display().to_string());
    let paths: Vec<&str> = m.files.iter().map(|f| f.path.as_str()).collect();
    assert!(paths.contains(&"hello.py"));
    assert!(
        !paths.contains(&".claude/settings.json"),
        "the tracker's own file is excluded"
    );
    assert!(
        m.excluded.iter().any(|p| p == ".claude/settings.json"),
        "and disclosed"
    );
    assert_eq!(
        m.files,
        {
            let mut s = m.files.clone();
            s.sort_by(|a, b| a.path.cmp(&b.path));
            s
        },
        "sorted, so a diff is stable"
    );
    #[cfg(unix)]
    {
        // The daemon opens the link's target, and a mirror that read one name differently would
        // turn an honest run into a difference report — so this opens the target too.
        let here = m.files.iter().find(|f| f.path == "hello.py").unwrap();
        let link = m
            .files
            .iter()
            .find(|f| f.path == "inside")
            .expect("recorded by its own path, not collapsed into its target's");
        assert_eq!(
            (link.bytes, link.sha256.as_str()),
            (here.bytes, here.sha256.as_str()),
            "the target's bytes, exactly as `goalcup_daemon::workspace::measure` reads them"
        );
    }
    // A link with nothing at the far end is listed with the absent digest and is not an
    // error — the daemon's `measure` answers a failed open the same way.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("nowhere-at-all", ws.join("dangling")).unwrap();
        let m = scan_workspace(ws, &ex, "entry-0001", support::NOW).unwrap();
        let d = m.files.iter().find(|f| f.path == "dangling").unwrap();
        assert_eq!((d.bytes, d.sha256.as_str()), (0, goalcup_cli::seal::ABSENT));
    }
    // The first argument borrows `…path()` because clippy's `unnecessary_to_owned` refuses a
    // copy; the borrow is sound: the `TempDir` temporary lives to the end of this statement,
    // so the directory is still on disk while the scan is walking it.
    assert!(
        scan_workspace(
            tempfile::tempdir().unwrap().path(),
            &[],
            "entry-0002",
            support::NOW
        )
        .unwrap()
        .files
        .is_empty(),
        "an empty workspace is a manifest, not an error"
    );
}

/// **The property `goalcup seal`'s self-check exists for.** Two walkers, one tree, no
/// difference — with nothing written between them, because nothing is written between them.
///
/// Everything else in this file checks the mirror against a
/// document a person typed. This runs `goalcup_cli::seal::scan_workspace` and
/// `goalcup_daemon::workspace::list_workspace` over ONE tree and asserts `compare` says
/// nothing at all — which is the only assertion that fails when the two implementations
/// drift, and the only one that would have caught the symlink reading.
///
/// The tree holds the four shapes that made the two walks disagree or could: nested
/// directories, a directory the exclusion set prunes, a symlink to a file (the daemon opens
/// the target, and so does the CLI), and a file larger than one 64 KiB hash chunk (the CLI
/// streams a file, so a multi-chunk file is where a mis-chunked digest would show).
#[test]
fn an_honest_run_over_one_tree_prints_no_difference_at_all() {
    use goalcup_daemon::workspace::list_workspace;

    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    std::fs::create_dir_all(ws.join("a/b/c")).unwrap();
    std::fs::create_dir_all(ws.join(".git/objects")).unwrap();
    std::fs::create_dir_all(ws.join(".claude")).unwrap();
    std::fs::write(ws.join("top.py"), b"print(1)\n").unwrap();
    std::fs::write(ws.join("empty.txt"), b"").unwrap();
    std::fs::write(ws.join("a/one.md"), b"one\n").unwrap();
    std::fs::write(ws.join("a/b/two.md"), b"two\n").unwrap();
    std::fs::write(ws.join("a/b/c/three.md"), b"three\n").unwrap();
    // Excluded, and pruned: neither walk may descend into it.
    std::fs::write(ws.join(".git/HEAD"), b"ref: refs/heads/main\n").unwrap();
    std::fs::write(ws.join(".git/objects/blob"), b"\x00\x01\x02").unwrap();
    std::fs::write(ws.join(".claude/settings.json"), b"{}").unwrap();
    // Two and a bit hash chunks, with a byte pattern that is not the same in every chunk —
    // a digest computed over the wrong chunk boundaries would still differ.
    let big: Vec<u8> = (0..(64 * 1024 * 2 + 777))
        .map(|i| (i % 251) as u8)
        .collect();
    std::fs::write(ws.join("a/big.bin"), &big).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("../top.py", ws.join("a/link-to-top")).unwrap();
        std::os::unix::fs::symlink("nowhere-at-all", ws.join("a/dangling")).unwrap();
    }

    let manifest = support::connector("claude-code-cli").manifest;
    let tracker_files = vec![".claude/settings.json".to_string()];
    // Both walks get the one exclusion set, so a difference report is about the workspace
    // and not about the set.
    let mine = exclusion_set(&manifest, &tracker_files);

    let local = scan_workspace(ws, &mine, "entry-0001", support::NOW).unwrap();
    let sealed = list_workspace(ws, "entry-0001", support::NOW, &mine).unwrap();

    assert_eq!(
        compare(&local, &sealed),
        Vec::<String>::new(),
        "an honest run: two walks over one tree, and nothing to report"
    );
    assert_eq!(local, sealed, "and the documents themselves are the same");

    // The tree really did exercise what it claims to. A vacuous agreement — two walkers that
    // both found nothing — would pass the assertion above and prove nothing.
    let paths: Vec<&str> = local.files.iter().map(|f| f.path.as_str()).collect();
    for expected in [
        "top.py",
        "empty.txt",
        "a/one.md",
        "a/b/c/three.md",
        "a/big.bin",
    ] {
        assert!(paths.contains(&expected), "{expected} in {paths:?}");
    }
    #[cfg(unix)]
    assert!(paths.contains(&"a/link-to-top") && paths.contains(&"a/dangling"));
    assert!(
        !paths.iter().any(|p| p.starts_with(".git/")),
        "the excluded directory was pruned by both: {paths:?}"
    );
    assert_eq!(
        local
            .files
            .iter()
            .find(|f| f.path == "a/big.bin")
            .map(|f| f.bytes),
        Some(big.len() as u64),
        "the streamed digest counted every chunk"
    );
}

/// The dev-dependency above is a TEST dependency, and the binary a participant installs
/// links none of it.
///
/// Two assertions, because either one alone is weak. `src/` naming the daemon at all is the
/// thing that would put it in the shipped graph, and the manifest placing it under
/// `[dev-dependencies]` is what keeps `cargo build --offline` — run with no features and no
/// dev-dependencies — from building it.
#[test]
fn the_shipped_binary_links_no_daemon() {
    for entry in support::walk("crates/goalcup-cli/src") {
        let src = std::fs::read_to_string(&entry).unwrap();
        // Comments name it all over this crate — the seal's whole job is to mirror
        // `goalcup_daemon::workspace`, and saying so is the point. CODE naming it is what
        // would put it in the shipped graph, so the comment lines are dropped first.
        for line in src.lines().filter(|l| !l.trim_start().starts_with("//")) {
            assert!(
                !line.contains("goalcup_daemon"),
                "{entry:?} names the daemon crate in code: {line}"
            );
        }
    }
    let toml = std::fs::read_to_string("Cargo.toml").unwrap();
    let dev = toml.find("[dev-dependencies]").expect("a dev section");
    let named = toml
        .find("\ngoalcup-daemon = ")
        .expect("the dev-dependency itself");
    assert!(
        named > dev,
        "goalcup-daemon is declared before [dev-dependencies], which ships it"
    );
}

/// A difference report nobody can read is not information. Every line names the path, and
/// says which of the two documents holds what — the participant's own scan, or the record.
#[test]
fn compare_names_the_path_and_says_which_side_holds_what() {
    let f = |p: &str, bytes: u64, h: &str| SealedFile {
        path: p.to_string(),
        bytes,
        sha256: h.repeat(32),
    };
    let local = SealManifest {
        entry_id: "entry-0001".to_string(),
        sealed_at: support::NOW.to_string(),
        workspace: "/w".to_string(),
        files: vec![f("both.py", 9, "aa"), f("only-here.md", 1, "bb")],
        excluded: vec![".git".to_string()],
    };
    let sealed = SealManifest {
        files: vec![f("both.py", 4, "cc"), f("only-sealed.md", 2, "dd")],
        ..local.clone()
    };
    let lines = compare(&local, &sealed);
    assert_eq!(
        lines.len(),
        3,
        "three files disagree and nothing else does: {lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.contains("both.py") && l.contains("this scan") && l.contains("seal")),
        "a file both hold, with both readings on the line: {lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.contains("only-here.md") && l.contains("this scan found it")),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.contains("only-sealed.md") && l.contains("seal lists it")),
        "{lines:?}"
    );
    assert!(
        compare(&local, &local).is_empty(),
        "two walks that agree produce no report at all"
    );
}

#[test]
fn seal_calls_the_daemon_once_prints_the_difference_and_points_at_the_trace_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_sealable(&tmp, "claude-code-cli");
    m.make_sealed_manifest_differ_on("hello.py");
    let entry = m.info().entry_id;
    let cfg = support::cfg_with_entry(tmp.path(), &m.info());
    let root = std::path::PathBuf::from(m.info().artifact_root);
    let dir = trace_dir(&root);
    // Every name under the WHOLE artifact root, with its size and its modification time, so a
    // rewrite with identical bytes and a file written beside the trace rather than in it both
    // show.
    let before = support::artifact_snapshot(&root);
    assert!(
        !before.is_empty(),
        "the daemon exported something to leave alone"
    );
    // The mtime half of that is as fine-grained as the filesystem under the tempdir; the
    // name-and-size half holds everywhere.
    let mut out = Vec::new();
    seal_with(
        &m,
        &support::connectors_dir(),
        &cfg,
        &entry,
        false,
        &mut out,
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();

    assert_eq!(m.calls().iter().filter(|c| *c == "seal").count(), 1);
    assert!(
        text.contains("hello.py"),
        "a difference is information, not a refusal"
    );
    assert!(
        text.contains(&dir.display().to_string()),
        "the daemon exported it; say where"
    );
    assert!(
        text.contains("goalcup-verify"),
        "one verifier binary, and the command to run it"
    );

    // The CLI wrote nothing under the artifact root: the same names, sizes and mtimes.
    assert_eq!(support::artifact_snapshot(&root), before);
}

/// A completed V4 response names immutable retained bytes. Reopening the live workspace
/// after that response is both unnecessary and unsafe: the agent may already have changed
/// or removed it, and a retry must report the durable result rather than resnapshot it.
#[test]
fn workspace_finish_reports_the_daemon_result_without_reopening_the_live_workspace() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_sealable(&tmp, "claude-code-cli");
    let info = m.info();
    let entry = info.entry_id.clone();
    let cfg = support::cfg_with_entry(tmp.path(), &info);
    let immutable_trace = tmp.path().join("immutable-v4-trace");
    m.set_workspace_finish(FinishReportV1 {
        entry_id: entry.clone(),
        trace_dir: immutable_trace.display().to_string(),
        termination: TerminationOutcomeV1::Confirmed {
            requested_at: Rfc3339Utc::try_from("2026-09-14T00:00:02Z").unwrap(),
            confirmed_at: Rfc3339Utc::try_from("2026-09-14T00:00:03Z").unwrap(),
            method: TerminationMethodV1::ProcessSignal,
        },
        submission: SubmissionOutcomeV1::Captured(SubmissionCapturedV1 {
            manifest_sha256: Sha256Digest::try_from("ab".repeat(32)).unwrap(),
        }),
    });
    std::fs::remove_dir_all(&info.workspace).expect("the live workspace is gone after finish");

    let mut out = Vec::new();
    seal_with(
        &m,
        &support::connectors_dir(),
        &cfg,
        &entry,
        false,
        &mut out,
    )
    .expect("the immutable V4 response does not depend on the live workspace");
    let text = String::from_utf8(out).unwrap();

    assert_eq!(m.calls().iter().filter(|call| *call == "seal").count(), 1);
    assert!(text.contains("workspace submission captured"), "{text}");
    assert!(
        text.contains(&immutable_trace.display().to_string()),
        "{text}"
    );
    assert!(
        !text.contains("this scan") && !text.contains("difference"),
        "a completed V4 result must not describe a new live-workspace scan: {text}",
    );
}

#[test]
fn dry_run_seals_nothing_and_an_already_sealed_entry_is_reported_not_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_sealable(&tmp, "codex-cli");
    let entry = m.info().entry_id;
    let cfg = support::cfg_with_entry(tmp.path(), &m.info());
    support::set_cached_connector(&cfg, &entry, "codex-cli");
    let mut out = Vec::new();
    seal_with(&m, &support::connectors_dir(), &cfg, &entry, true, &mut out).unwrap();
    assert!(!m.calls().contains(&"seal".to_string()));
    assert!(String::from_utf8(out).unwrap().contains("hello.py"));

    // A dry run still needs the cached record: it has no other source for the workspace.
    let err = seal_with(
        &m,
        &support::connectors_dir(),
        &support::empty_cfg(),
        &entry,
        true,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert_eq!(err.exit_code(), 1);

    let m = support::mock_sealable(&tmp, "codex-cli");
    m.seal_once_then_conflict();
    seal_with(
        &m,
        &support::connectors_dir(),
        &cfg,
        &entry,
        false,
        &mut Vec::new(),
    )
    .unwrap();
    let mut out = Vec::new();
    seal_with(
        &m,
        &support::connectors_dir(),
        &cfg,
        &entry,
        false,
        &mut out,
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("already sealed at"));
    assert!(
        text.contains("deadline") || text.contains("earlier"),
        "say WHICH seal they are looking at, so nobody thinks their early finish was lost"
    );
}

/// `busy` is the daemon holding the entry for one asynchronous call, and the seal asks on
/// its own four-attempt dephased schedule before it is reported.
///
/// The seal must not inherit the generic three-attempt schedule: its own bounded policy is
/// what escapes a 200 ms capture cadence while preserving a full slow-call reservation.
/// Both ends of the rule are here: one `busy` is absorbed and the seal succeeds, and a daemon
/// that is STILL busy when its reservation is spent is rung 1 with the daemon's own words in
/// it. Its one-second reservation envelope holds the 550 ms planned schedule plus ordinary
/// scheduler overhead, so this real command path reaches all four bounded calls.
#[test]
fn a_busy_seal_is_asked_again_and_a_standing_busy_is_rung_one() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_sealable(&tmp, "claude-code-cli");
    let entry = m.info().entry_id;
    let cfg = support::cfg_with_entry(tmp.path(), &m.info());

    m.busy_once("seal");
    let mut out = Vec::new();
    seal_with(
        &m,
        &support::connectors_dir(),
        &cfg,
        &entry,
        false,
        &mut out,
    )
    .unwrap();
    assert_eq!(
        m.calls().iter().filter(|c| *c == "seal").count(),
        2,
        "the transient was ridden out rather than reported"
    );
    assert!(String::from_utf8(out).unwrap().contains("sealed at"));

    let m = support::mock_sealable(&tmp, "claude-code-cli");
    m.fail_always(
        "seal",
        ControlError::Busy("the pump is mid-cycle on this entry".to_string()),
    );
    let err = seal_with(
        &m,
        &support::connectors_dir(),
        &cfg,
        &entry,
        false,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert_eq!(
        err.exit_code(),
        1,
        "actionable: wait a moment and run it again"
    );
    let said = err.to_string();
    assert!(
        said.contains("the pump is mid-cycle on this entry"),
        "the daemon's own words, not a sentence the CLI invented: {said}"
    );
    assert!(
        said.contains("no seal manifest was returned"),
        "state only what this command observed: {said}"
    );
    assert!(
        said.contains("may still be completing a seal"),
        "a Busy response does not prove that another asynchronous seal did not commit: {said}"
    );
    assert_eq!(
        m.calls().iter().filter(|c| *c == "seal").count(),
        goalcup_cli::retry::SEAL_BUSY_ATTEMPTS,
        "the real command reaches all four bounded seal asks"
    );
}

/// An entry the daemon had already sealed still gets a comparison, against the manifest that
/// seal actually produced.
///
/// A bare local scan would be a list of files with nothing to hold it against, in the one
/// case where the self-check matters most — the daemon sealed at the deadline, and
/// everything written since is a difference the participant has a right to see. So the
/// conflict branch reads `<artifact_root>/trace/seal.json`, read-only, and runs the same
/// comparison the success branch runs.
#[test]
fn an_already_sealed_entry_is_compared_against_the_seal_the_daemon_exported() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_sealable(&tmp, "claude-code-cli");
    let info = m.info();
    let entry = info.entry_id.clone();
    let cfg = support::cfg_with_entry(tmp.path(), &info);
    let root = std::path::PathBuf::from(&info.artifact_root);

    // What the daemon sealed, hours ago: a file the workspace no longer holds. The workspace
    // holds `hello.py`, which that seal never listed.
    let mut tracker_files = info.workspace_relative.clone();
    tracker_files.sort();
    let excluded = exclusion_set(
        &support::connector("claude-code-cli").manifest,
        &tracker_files,
    );
    support::plant_exported_seal(
        &info.artifact_root,
        &SealManifest {
            entry_id: entry.clone(),
            sealed_at: support::NOW.to_string(),
            workspace: info.workspace.clone(),
            files: vec![SealedFile {
                path: "gone-since.md".to_string(),
                bytes: 3,
                sha256: "ab".repeat(32),
            }],
            excluded: excluded.clone(),
        },
    );
    let before = support::artifact_snapshot(&root);

    // The mock answers the first seal and a `conflict` on every one after it, so the second
    // call below is the already-sealed path a participant meets after the deadline.
    seal_with(
        &m,
        &support::connectors_dir(),
        &cfg,
        &entry,
        false,
        &mut Vec::new(),
    )
    .unwrap();
    let mut out = Vec::new();
    seal_with(
        &m,
        &support::connectors_dir(),
        &cfg,
        &entry,
        false,
        &mut out,
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();

    assert!(text.contains("already sealed at"));
    assert!(
        text.contains("gone-since.md") && text.contains("seal lists it"),
        "the exported seal was read and compared, not just the local scan printed: {text}"
    );
    assert!(
        text.contains("hello.py") && text.contains("this scan found it"),
        "and the other direction: {text}"
    );
    assert_eq!(
        support::artifact_snapshot(&root),
        before,
        "read-only: the comparison opened that file and wrote nothing"
    );

    // A trace with no readable seal in it is said out loud and is not a refusal: the entry is
    // sealed either way. `mock_sealable` writes `{}` there, which is exactly that case.
    let m = support::mock_sealable(&tmp, "claude-code-cli");
    let cfg2 = support::cfg_with_entry(tmp.path(), &m.info());
    let entry2 = m.info().entry_id;
    seal_with(
        &m,
        &support::connectors_dir(),
        &cfg2,
        &entry2,
        false,
        &mut Vec::new(),
    )
    .unwrap();
    let mut out = Vec::new();
    seal_with(
        &m,
        &support::connectors_dir(),
        &cfg2,
        &entry2,
        false,
        &mut out,
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("could not be read") && text.contains("this machine's scan of"),
        "say why there is nothing to compare against, and still show the scan: {text}"
    );
}

/// A `seal.json` belonging to a DIFFERENT entry is refused by name, not compared.
///
/// The conflict branch reads `<artifact_root>/trace/seal.json` at a path built by
/// pure arithmetic over the `artifact_root` the entry cache holds — a file on the
/// participant's own disk. An artifact root copied from another run, or two records pointed at
/// one, puts a stranger's manifest under this entry's path, and every line of the difference
/// report is then wrong in a way the participant cannot see: the only thing that would have
/// told them is the `entry_id` inside the document nobody compared.
#[test]
fn a_seal_manifest_from_another_entry_is_refused_rather_than_compared() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_sealable(&tmp, "claude-code-cli");
    let info = m.info();
    let entry = info.entry_id.clone();
    let cfg = support::cfg_with_entry(tmp.path(), &info);

    // Somebody else's seal, under this entry's path, and otherwise perfectly well formed:
    // it parses, it lists a file, and nothing but the id says it is not this run's.
    support::plant_exported_seal(
        &info.artifact_root,
        &SealManifest {
            entry_id: "entry-from-another-run".to_string(),
            sealed_at: support::NOW.to_string(),
            workspace: info.workspace.clone(),
            files: vec![SealedFile {
                path: "someone-elses.md".to_string(),
                bytes: 3,
                sha256: "cd".repeat(32),
            }],
            excluded: Vec::new(),
        },
    );

    // The first seal is answered; every one after it is a `conflict`, which is the branch
    // that reads the file.
    seal_with(
        &m,
        &support::connectors_dir(),
        &cfg,
        &entry,
        false,
        &mut Vec::new(),
    )
    .unwrap();
    let mut out = Vec::new();
    seal_with(
        &m,
        &support::connectors_dir(),
        &cfg,
        &entry,
        false,
        &mut out,
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();

    assert!(
        text.contains("is entry entry-from-another-run's")
            && text.contains(&format!("not entry {entry}'s")),
        "the refusal names both ids, because which document was read is the whole finding: {text}"
    );
    assert!(
        !text.contains("someone-elses.md"),
        "not one line of a stranger's manifest is presented as this entry's record: {text}"
    );
    assert!(
        text.contains("this machine's scan of") && text.contains("hello.py"),
        "and the local scan is still shown, on its own: {text}"
    );
}

/// `--dry-run` needs no daemon, so it must not need a client — and `main.rs` must not build
/// one before it gets there.
///
/// A `goalcup seal --dry-run` on a machine with no daemon running must not refuse at
/// `HttpDaemonClient::from_config_dir`, telling a participant asking "what is in my
/// workspace" that the tracker was unreachable — about a question the tracker is never
/// asked.
#[test]
fn a_dry_run_answers_on_a_machine_with_no_control_json_at_all() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_sealable(&tmp, "claude-code-cli");
    let entry = m.info().entry_id;
    let cfg = support::cfg_with_entry(tmp.path(), &m.info());
    assert!(
        goalcup_cli::daemon::HttpDaemonClient::from_config_dir(&cfg).is_err(),
        "no `control.json` here: a client cannot be built at all, which is the point"
    );
    let mut out = Vec::new();
    dry_run_with(&support::connectors_dir(), &cfg, &entry, &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("a dry run") && text.contains("hello.py"));
    assert!(!m.calls().contains(&"seal".to_string()));
}

/// The pasted line: quoted where the path has a space in it, and `--dev` iff the run was one.
///
/// A development run's trace names a connector manifest this build does not pin, and
/// `goalcup-verify` fails it outright without the flag — so a line printed without it would
/// not work for the one participant most likely to paste it verbatim. The flag
/// comes off `EntryRecord::dev`, written by `goalcup start --dev`, and never off the trace:
/// the trace is the document being checked, and a document must not choose how strictly it
/// is read.
#[test]
fn the_verify_line_is_quoted_and_carries_dev_only_for_a_dev_run() {
    let spaced = Path::new("/Users/a b/goalcup/entry-0001/trace");
    let strict = verify_command(spaced, false);
    assert!(
        strict.contains("'/Users/a b/goalcup/entry-0001/trace'")
            || strict.contains("\"/Users/a b/goalcup/entry-0001/trace\""),
        "a path with a space is quoted so the line can be pasted as it stands: {strict}"
    );
    assert!(!strict.contains("--dev"), "{strict}");
    let dev = verify_command(spaced, true);
    assert!(dev.starts_with("goalcup-verify --dev "), "{dev}");
    assert!(dev.ends_with(&strict["goalcup-verify ".len()..]), "{dev}");

    // End to end: the record says which run this was, and the printed line follows it.
    let tmp = tempfile::tempdir().unwrap();
    for (dev, expect) in [(false, false), (true, true)] {
        let m = support::mock_sealable(&tmp, "claude-code-cli");
        let entry = m.info().entry_id;
        let cfg = support::cfg_with_entry(tmp.path(), &m.info());
        support::set_cached_dev(&cfg, &entry, dev);
        let mut out = Vec::new();
        seal_with(
            &m,
            &support::connectors_dir(),
            &cfg,
            &entry,
            false,
            &mut out,
        )
        .unwrap();
        let text = String::from_utf8(out).unwrap();
        assert_eq!(
            text.contains("goalcup-verify --dev"),
            expect,
            "dev={dev}: {text}"
        );
        assert!(text.contains("goalcup-verify"), "{text}");
    }
}

/// A difference report bounds its exclusion lines and its file lines separately, so two
/// hundred of the first cannot hide the one that matters.
///
/// One bound over the concatenation would spend the whole listing on exclusions — which
/// is what an edited connector manifest produces — and print not one file. `compare_parts`
/// is the seam: the kinds are kept apart, and `compare` still
/// concatenates them in reading order for everything that wants one list.
#[test]
fn exclusion_lines_and_file_lines_are_bounded_apart() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_sealable(&tmp, "claude-code-cli");
    m.make_sealed_manifest_differ_on("hello.py");
    let entry = m.info().entry_id;
    let cfg = support::cfg_with_entry(tmp.path(), &m.info());

    // Far more exclusions on this machine's side than the listing holds, and none of them
    // touching `hello.py`. They reach the scan through the record's `workspace_relative`,
    // which is the daemon's own list of what it installed — the field the exclusion union is
    // built from.
    let mut record = goalcup_cli::entries::read_entry(&cfg, &entry).unwrap();
    record.workspace_relative = (0..40).map(|i| format!(".tracker/f{i:03}")).collect();
    goalcup_cli::entries::write_entry(&cfg, &record).unwrap();

    let mut out = Vec::new();
    seal_with(
        &m,
        &support::connectors_dir(),
        &cfg,
        &entry,
        false,
        &mut out,
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("… and"),
        "the exclusion listing was bounded: {text}"
    );
    assert!(
        text.contains("hello.py"),
        "and the file difference still reached the screen: {text}"
    );

    // The seam itself: three kinds, kept apart, and `compare` is their concatenation.
    let f = |p: &str, h: &str| SealedFile {
        path: p.to_string(),
        bytes: 1,
        sha256: h.repeat(32),
    };
    let local = SealManifest {
        entry_id: "e".to_string(),
        sealed_at: support::NOW.to_string(),
        workspace: "/w".to_string(),
        files: vec![f("x.py", "aa")],
        excluded: vec![".git".to_string()],
    };
    let sealed = SealManifest {
        workspace: "/other".to_string(),
        files: vec![f("x.py", "bb")],
        excluded: vec![".hg".to_string()],
        ..local.clone()
    };
    let parts = compare_parts(&local, &sealed);
    assert!(parts.workspace.is_some());
    assert_eq!(parts.exclusions.len(), 2);
    assert_eq!(parts.files.len(), 1);
    assert_eq!(parts.len(), 4);
    assert_eq!(compare(&local, &sealed), parts.lines());
}

/// `<trace>/seal.json` is read with a bound on it, and a file past the bound is said out
/// loud rather than read into memory whole.
///
/// Every file this crate reads from outside itself is capped at
/// `config::CONTROL_FILE_MAX_BYTES` for one reason — the size of a document this process did
/// not write is not this process's to trust — and this one is read from an `artifact_root`
/// the ENTRY CACHE names, which is a file on the participant's own disk.
///
/// The seal itself still succeeds: an exported manifest this side could not read is not a
/// refusal, it is a comparison that cannot be made, and the local scan is printed on its own
/// with a sentence saying so. That is the same path an unreadable file already took.
#[test]
fn the_exported_seal_manifest_is_read_under_a_cap_and_a_larger_one_is_said_out_loud() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_sealable(&tmp, "claude-code-cli");
    let entry = m.info().entry_id;
    let cfg = support::cfg_with_entry(tmp.path(), &m.info());
    let dir = trace_dir(std::path::Path::new(&m.info().artifact_root));
    std::fs::create_dir_all(&dir).unwrap();
    let cap = goalcup_cli::config::CONTROL_FILE_MAX_BYTES;
    // One byte past the cap, which is the boundary the reader is written against.
    let padding = " ".repeat(usize::try_from(cap).unwrap() + 1);
    std::fs::write(
        dir.join("seal.json"),
        format!("{{\"comment\":\"{padding}\"}}"),
    )
    .unwrap();

    // The exported manifest is what an ALREADY SEALED entry is compared against: the daemon
    // answers `409` with no manifest on it, so the file on disk is the only record there is.
    m.seal_once_then_conflict();
    seal_with(
        &m,
        &support::connectors_dir(),
        &cfg,
        &entry,
        false,
        &mut Vec::new(),
    )
    .unwrap();
    let mut out = Vec::new();
    seal_with(
        &m,
        &support::connectors_dir(),
        &cfg,
        &entry,
        false,
        &mut out,
    )
    .expect("a manifest this side cannot read is not a refusal");
    let said = String::from_utf8(out).unwrap();
    assert!(
        said.contains(&cap.to_string()),
        "the cap is named, so a participant can tell this from a corrupt file: {said}"
    );
    assert!(
        said.contains("this machine's alone"),
        "and the scan below it is said to stand on its own: {said}"
    );
}

/// Sealing a run whose harness never started says so, before the seal.
///
/// Without it the seal prints a cheerful "0 files, 3 exclusions applied" and a
/// `goalcup-verify` line that then exits 3 with no verdict at all. The seal is still the
/// right way to end an entry — it has to end somehow, and the manifest of an empty workspace
/// is a true statement about it — but the participant is owed the sentence saying what it
/// will be worth WHILE stopping is still a choice they have, and that means before the call
/// rather than after it.
///
/// Asked of the daemon rather than of the local record: binding a session is the daemon's
/// event, and nothing this machine wrote to disk knows whether one happened.
#[test]
fn sealing_a_run_that_never_started_warns_that_the_verdict_will_be_a_fail() {
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_sealable(&tmp, "claude-code-cli");
    m.set_session_id(None);
    let entry = m.info().entry_id;
    let cfg = support::cfg_with_entry(tmp.path(), &m.info());

    let mut out = Vec::new();
    seal_with(
        &m,
        &support::connectors_dir(),
        &cfg,
        &entry,
        false,
        &mut out,
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("no session") && text.contains("no_session"),
        "the warning names the state and the finding it becomes: {text}"
    );
    let warning = text.find("no session").unwrap();
    let sealed = text.find("sealed at").expect("the seal still happened");
    assert!(
        warning < sealed,
        "the warning comes BEFORE the seal, while stopping is still a choice: {text}"
    );

    // …and a bound run is told nothing of the sort.
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_sealable(&tmp, "claude-code-cli");
    let entry = m.info().entry_id;
    let cfg = support::cfg_with_entry(tmp.path(), &m.info());
    let mut out = Vec::new();
    seal_with(
        &m,
        &support::connectors_dir(),
        &cfg,
        &entry,
        false,
        &mut out,
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(
        !text.contains("no_session"),
        "a run that captured something is not warned about one that did not: {text}"
    );
}

/// H-15: `goalcup seal` on an entry whose trace is torn. The tracker holds the row as a capture
/// but has no status and no open capture, and answers the CLI's legacy seal `malformed`. That was
/// reported as "malformed control response" at rung 3; it is a safe refusal at rung 1 that says
/// what happened, that nothing was changed, and what to do next.
#[test]
fn a_torn_entry_is_refused_with_what_happened_and_nothing_is_sealed() {
    use goalcup_cli::CliError;

    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_sealable(&tmp, "claude-code-cli");
    let entry = m.info().entry_id;
    let cfg = support::cfg_with_entry(tmp.path(), &m.info());
    m.fail_always(
        "status",
        ControlError::NotFound("no status for that entry".into()),
    );
    m.fail_always(
        "seal",
        ControlError::Malformed("a v5 capture entry is sealed with a capture seal request".into()),
    );
    let before = support::artifact_snapshot(std::path::Path::new(&m.info().artifact_root));

    let mut out = Vec::new();
    let error = seal_with(
        &m,
        &support::connectors_dir(),
        &cfg,
        &entry,
        false,
        &mut out,
    )
    .expect_err("a torn entry cannot be sealed");

    assert!(matches!(error, CliError::Refused(_)), "{error:?}");
    assert_eq!(
        error.exit_code(),
        1,
        "a refusal, not a CLI that could not do its job"
    );
    let said = error.to_string();
    assert!(
        said.contains(&format!("Entry {entry} cannot be sealed")),
        "{said}"
    );
    assert!(said.contains("torn or unreadable"), "{said}");
    assert!(
        said.contains("Nothing was sealed, written or repaired"),
        "{said}"
    );
    assert!(said.contains("preserved"), "{said}");
    assert!(
        said.contains("goalcup-verify --capture-integrity <the entry's trace directory>"),
        "the one command to read what survives: {said}"
    );
    assert!(said.contains(&format!("a folder named {entry}")), "{said}");
    assert!(said.contains("start a new run"), "{said}");
    assert!(
        !said.contains("malformed control response"),
        "the misleading wording is gone: {said}"
    );
    assert_eq!(
        support::artifact_snapshot(std::path::Path::new(&m.info().artifact_root)),
        before,
        "nothing under the artifact root was touched"
    );

    // A `malformed` the status read gives no reason to read as a torn entry stays what it was.
    let tmp = tempfile::tempdir().unwrap();
    let m = support::mock_sealable(&tmp, "claude-code-cli");
    let entry = m.info().entry_id;
    let cfg = support::cfg_with_entry(tmp.path(), &m.info());
    m.fail_always("seal", ControlError::Malformed("something else".into()));
    let error = seal_with(
        &m,
        &support::connectors_dir(),
        &cfg,
        &entry,
        false,
        &mut Vec::new(),
    )
    .expect_err("malformed");
    assert!(
        matches!(error, CliError::Control(ControlError::Malformed(_))),
        "{error:?}"
    );
    assert_eq!(error.exit_code(), 3);
}
