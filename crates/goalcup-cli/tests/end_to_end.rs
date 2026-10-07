//! One session, one configuration directory, and every seam the earlier files prove one at
//! a time.
//!
//! Every other test in this crate drives one command against a mock built for it. This one
//! runs a whole run through a single mock and a single configuration directory — `start`,
//! the arming gate, the harness's own transcript, `status`, a `resume`, and `seal` — and asserts
//! the order the daemon was asked things in as ONE sequence, plus the on-disk state a
//! participant would actually see afterwards.
//!
//! # What it can prove and what it cannot
//!
//! The harness is real, as a process: the connector's `[launch]` program is a shell script
//! this crate ships, it is spawned by `launch::spawn` from the shipped argv with the
//! `--settings` file the daemon wrote, and it writes a transcript and a file into the
//! workspace.
//!
//! What is NOT real is the daemon. `MockDaemonClient` answers the control API in process:
//! nothing tails the transcript this fixture writes, and the `trace/` directory the seal names
//! is a path and not an export. **An assembled trace is a trace the test wrote**, so this file
//! verifies nothing and asserts that the seal's own output makes no verdict claim either.
//! `tests/real_daemon_end_to_end.rs` runs a fake harness against the real daemon and the real
//! verifier, and that is what proves a trace.
//!
//! Two more absences are named here because a reader will look for them in this file and not
//! find them. The fixture runs to completion before `start_with` returns, so the seal below
//! meets a process that has already exited and the daemon's terminate-at-seal is never asked
//! anything; and the run comes through `--dev`, so D12's version gate is never asked of a
//! real binary.
#![cfg(feature = "v1-compat")]

mod support;

use goalcup_cli::arming::GateOutcome;
use goalcup_cli::commands::{
    resume::resume_with, seal::seal_with, start::start_with, status::status_with,
};

#[test]
fn the_whole_flow_meets_at_every_seam_against_the_mock() {
    let tmp = tempfile::tempdir().unwrap();
    // A mock whose answer names the files the daemon wrote.
    let m = support::mock_with_daemon_files(&tmp, "claude-code-cli");
    let connectors = support::connector_with_fake_harness("claude-code-cli", &tmp);
    let info = m.info();

    // `--dev`, because the fixture rewrites `[launch] program` and a rewritten connector
    // directory is a rulebook this build does not pin. That is the one door behaving as it
    // should, and it is asserted below: the record says `dev` and the seal's verify line
    // carries the flag.
    let mut options = support::start_options("claude-code-cli");
    options.dev = true;

    // A throwaway `$HOME` carrying the keys the connector's `[home_seed]` requires. Claude
    // Code keeps its first-run state in the per-run configuration directory, so without them
    // this start refuses rather than launching into a theme picker and a login wall (D1).
    let home = support::home_with_a_signed_in_harness(tmp.path());

    let mut start_out = Vec::new();
    let outcome = start_with(
        &m,
        &connectors,
        tmp.path(),
        &support::clean_env_with_home(&home),
        &options,
        &mut start_out,
    )
    .unwrap();

    assert_eq!(outcome.entry_id, info.entry_id);
    assert!(matches!(outcome.gate, GateOutcome::NotRequired));
    assert_eq!(
        outcome.exit_status.map(|s| s.success()),
        Some(true),
        "the fake harness ran to completion: {}",
        String::from_utf8_lossy(&start_out)
    );

    // The daemon wrote the launch settings, and the agent's work is in the workspace.
    let ws = std::path::Path::new(&info.workspace);
    assert!(
        std::path::Path::new(&info.artifact_root)
            .join("settings-launch.json")
            .exists(),
        "the DAEMON wrote it (R3)"
    );
    assert!(
        ws.join("hello.py").exists(),
        "the fake harness did some work"
    );

    // Leg 1, under the project slug a harness whose cwd is this workspace writes to. The
    // slug is asked of the daemon's own `claude_slug` rather than spelled again here: the
    // fixture derives it from its cwd, the daemon derives it from the workspace string it
    // holds, and a test that wrote out a third spelling would agree with neither.
    //
    // It is asked about the CANONICAL workspace, and that is a real caveat rather than a
    // tidy-up. A shell recomputes its cwd physically when the environment it was handed
    // carries a stale `PWD`, and so does a real harness reporting `process.cwd()` — so on a
    // macOS temporary directory, where `/var` is a link to `/private/var`, the harness
    // writes under the physical slug while the daemon holds the logical path. Whether the
    // daemon's tailer finds it is a question about the DAEMON, asked in
    // `real_daemon_end_to_end.rs`; what this line asserts is that the fixture writes where a
    // real harness would.
    let physical = std::fs::canonicalize(ws).unwrap();
    let transcript = std::path::Path::new(&info.harness_home)
        .join("projects")
        .join(goalcup_daemon::paths::claude_slug(&physical))
        .join(format!("{}.jsonl", info.session_id));
    let records: Vec<serde_json::Value> = std::fs::read_to_string(&transcript)
        .unwrap_or_else(|e| panic!("{}: {e}", transcript.display()))
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let typed = records
        .iter()
        .find(|r| r["type"] == "user")
        .expect("the transcript carries the typed prompt");
    assert_eq!(typed["sessionId"], info.session_id.as_str());
    assert_eq!(typed["cwd"], physical.display().to_string().as_str());
    assert!(
        typed["promptId"].is_string(),
        "the join key a later leg pairs on"
    );

    // The key comes from the record `start_with` wrote a moment ago, not from a parameter
    // this test happened to have: that record is the seam under test.
    let rec = goalcup_cli::entries::read_entry(tmp.path(), &info.entry_id).unwrap();
    assert_eq!(rec.run_public_key, info.run_public_key);
    assert!(
        rec.dev,
        "the run came through the one door and the record says so"
    );
    let launched_pid = rec.last_pid.expect("the launch's pid was written back");
    // The daemon's disclosure list, carried into the record and read back out of it by the
    // seal. Claude has no tracker file in the workspace.
    assert!(
        rec.workspace_relative.is_empty(),
        "{:?}",
        rec.workspace_relative
    );

    let mut status_out = Vec::new();
    status_with(
        &m,
        tmp.path(),
        Some(&info.entry_id),
        true,
        support::NOW,
        &mut status_out,
    )
    .unwrap();
    let doc: goalcup_protocol::control::SignedStatus = serde_json::from_slice(&status_out).unwrap();
    assert!(doc.verify_with(&info.run_public_key).is_ok());

    // A resume mid-chain: the same entry, the same configuration directory, the `[resume]`
    // recipe rather than the `[launch]` one — and a NEW pid handed over, because a daemon
    // still measuring the pre-crash one would aim its seal-time terminate at a process that
    // is gone.
    let mut resume_out = Vec::new();
    resume_with(
        &m,
        &connectors,
        tmp.path(),
        &info.entry_id,
        &support::resume_options(),
        &mut resume_out,
    )
    .unwrap();
    let resumed = goalcup_cli::entries::read_entry(tmp.path(), &info.entry_id).unwrap();
    let resumed_pid = resumed
        .last_pid
        .expect("the relaunch's pid was written back");
    assert_ne!(
        resumed_pid, launched_pid,
        "the relaunch is a different process"
    );
    assert_eq!(
        m.last_registered_pid(),
        Some(resumed_pid),
        "the daemon was handed the relaunch, not the launch"
    );

    let mut dry_out = Vec::new();
    seal_with(
        &m,
        &connectors,
        tmp.path(),
        &info.entry_id,
        true,
        &mut dry_out,
    )
    .unwrap();
    let dry = String::from_utf8(dry_out.clone()).unwrap();
    assert!(dry.contains("hello.py"));
    // The exclusion set the seal applied, counted: `.claude` and `.git`, the connector's own
    // `workspace_exclusions`. The daemon's disclosure list is empty.
    assert!(
        dry.contains("2 exclusions applied"),
        "the connector's own exclusions reached the seal's exclusion set: {dry}"
    );
    // This test assembles no trace and verifies nothing: that is `real_daemon_end_to_end.rs`'s,
    // over the directory the real daemon exported.
    assert!(!dry.contains("expected-verdict"));

    // And the real seal, which is where the verifier line is printed.
    let mut seal_out = Vec::new();
    seal_with(
        &m,
        &connectors,
        tmp.path(),
        &info.entry_id,
        false,
        &mut seal_out,
    )
    .unwrap();
    let sealed = String::from_utf8(seal_out.clone()).unwrap();
    let trace = std::path::Path::new(&info.artifact_root).join("trace");
    assert!(
        sealed.contains(&format!("trace:  {}", trace.display())),
        "the seal names the daemon's own export directory: {sealed}"
    );
    assert!(
        sealed.contains(&format!("verify: goalcup-verify --dev {}", trace.display())),
        "the verify line is exactly what Task 16 will run, `--dev` and all: {sealed}"
    );
    assert!(!sealed.contains("expected-verdict"));

    // The whole run, as one sequence. `status` appears FOUR times and every one is
    // accounted for: the arming gate asks once before it decides there is nothing to wait
    // for (Claude Code supplies no hook-derived birth, D16), `goalcup status` asks once
    // more, `goalcup resume` first reads its immutable preview/generation before its one
    // mutating CAS, then asks once when its relaunched harness EXITS — the line that says
    // how it went and what the tracker holds, rather than a promise about a process that is
    // already gone — and `goalcup seal` asks once BEFORE it seals,
    // because a run that bound no session is a `no_session` Fail and the participant is told
    // so while stopping is still a choice. That last one is the last thing on
    // the wire before `seal`.
    assert_eq!(
        m.calls(),
        vec![
            "create_workspace",
            "arm",
            "register_process",
            "status",
            "status",
            "resume_preview",
            "resume",
            "register_process",
            "status",
            "status",
            "seal",
        ],
        "D22: the daemon learns about the process only once the folder is armed"
    );
    assert!(
        m.abandoned().is_empty(),
        "nothing failed, so nothing was unmade"
    );

    // No secret in anything a participant saw, across the whole run.
    let streams = [start_out, status_out, resume_out, dry_out, seal_out];
    for stream in &streams {
        let text = String::from_utf8_lossy(stream);
        assert!(
            !text.contains(support::GRANT_TOKEN),
            "the grant token reached a stream: {text}"
        );
    }
}
