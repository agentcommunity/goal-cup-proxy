//! One run, driven through the `goalcup` BINARY, against the real daemon, the
//! real devserver and the real verifier: `start` → arm → the arming signal → `status` →
//! `seal` → `goalcup-verify`.
//!
//! # The symlinked workspace parent
//!
//! The daemon binds an entry to the workspace path as `std::fs::canonicalize` returns it,
//! not as configured. Under a workspace parent reached through a symlink — the default shape
//! of a macOS temporary directory, and of any `workspace_parent` under `/tmp`, `/var` or a
//! linked home — a daemon that bound the configured spelling would capture zero transcript
//! records, with no counter moving on any surface a participant has. So the fixture's root is
//! NOT canonicalised, which `api_gate.rs` does and which would hide exactly that defect. The
//! transcript guard is the chain wait at stage 4; stage 5 also retains the daemon's
//! `posts_dropped == 0` signed legacy field-shape assertion; V1 has no receiver path, so it is
//! not live no-loss proof.
//!
//! # What this proves that nothing else does
//!
//! `crates/goalcup-devserver/tests/api_gate.rs` already drives the daemon-to-verifier seam
//! over the shipped binaries — it POSTs `/v0/workspaces`, arms, registers a process, writes
//! the harness's records itself, seals and verifies. Everything it proves about the daemon
//! this file takes as given. What it does not cover is the participant: the argv `goalcup
//! start` renders, the manifest digest it sends, the entry cache it writes, the pinned run
//! key later processes read out of that cache, the harness that is STILL RUNNING when the
//! seal arrives, and the `goalcup-verify` line `goalcup seal` prints. So this test types the
//! commands instead of speaking HTTP — `env!("CARGO_BIN_EXE_goalcup")` with
//! `$GOALCUP_CONFIG_DIR` pointing at the daemon's own `config_dir`, which is the CLI's
//! canonical-location rule — and asserts the expected transcript-only Fail verdict and findings.
//!
//! # The harness outlives the seal, on purpose
//!
//! `goalcup start` runs here as a CHILD of this test and not as `.output()`, and
//! `$GOALCUP_FAKE_CLAUDE_LINGER` makes the fixture stay alive after it has written everything.
//! That is the real shape: the participant seals while their harness is open, `start` is
//! blocked in `child.wait()`, and the process ends because `Daemon::seal_entry` terminated it.
//! Stage 6 asserts that `start` came back, under zero, with the pid gone — which is the
//! hand-off working in the only direction a test can see it from.
//!
//! # The harness is a PATH shim, and the connector is the shipped one
//!
//! `tests/fixtures/fake-claude.sh` is installed as `claude` on the child's `PATH`, which is
//! the name `connectors/claude-code-cli/manifest.toml` declares. The connector directory is
//! this repository's, untouched — and it has to be: a rewritten manifest is a rulebook this
//! build does not pin, `manifest_not_pinned` is a **Flag even under `--dev`**, and `overall`
//! could then never be Pass. The fixture's header says the rest.
//!
//! # No `--clock` margin
//!
//! `api_gate.rs` opens the server's window ten minutes below the run. That is the wrong
//! thing to copy here: a record in `[started_at - 300s, started_at)` is a **Pass**
//! (`pre_start_records`, `ARMING_TOLERANCE_SECONDS`) and only one earlier than that is a
//! Flag. An honest real-time run needs no margin and passes; a copied 600 s offset would
//! move every record above `started_at` and test a shape no participant produces.
//!
//! # Why it is `#[ignore]`d
//!
//! It binds two loopback ports, spawns several processes and waits on the daemon's own
//! cadences. Run by name:
//!
//! ```text
//! cargo test -p goalcup-cli --offline --features test-support --test real_daemon_end_to_end -- --ignored
//! ```
//!
//! Its assertions are **staged** for the same reason: a failure names which stage broke
//! rather than arriving as one red end-to-end line with a dozen candidates.
#![cfg(feature = "v1-compat")]

mod support;

use goalcup_protocol::control::{ArmingState, ControlError, DaemonClient, ResumePreviewInfo};
use goalcup_verifier::{VerifyOptions, verify_trace_with_options};
use std::path::Path;
#[cfg(feature = "v1-compat")]
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// The connector this run uses, and the only one that ships a fixture harness.
const CONNECTOR: &str = "claude-code-cli";

/// The five transcript records and no hook posts `fake-claude.sh` produces. Named here so the
/// wait below asks for a CONDITION the daemon's poll cadence reaches rather than a duration.
const LEG1_RECORDS: usize = 5;
const LEG2_POSTS: usize = 0;

/// How long `goalcup start` is given to come back once the seal has terminated its harness.
///
/// The shipped `claude-code-cli` grace is five seconds and this fixture does not survive its
/// SIGTERM, so the real number is milliseconds. This is the bound that turns "the hand-off is
/// broken" from a hang into a failure that names itself.
const HANDOFF_PATIENCE: Duration = Duration::from_secs(20);

/// `goalcup start`, running as a child of this test: its whole process GROUP killed on
/// `Drop`, with both pipes drained by threads of their own.
///
/// Two hazards are avoided. A stage that fails before stage 6 would leave the child running —
/// with the harness it launched under it — because a `Child` that is dropped is not a child
/// that is killed. And a child that fills a 64 KiB pipe buffer while nothing is reading it
/// blocks in `write`, which is a hang no assertion in this file would name. `DaemonProcess`
/// has the kill-on-`Drop` half; this is the same guard, plus the drains.
///
/// A plain `Child::kill()` reaches only THIS process — `goalcup start` — and not the harness
/// it launches underneath, which is a grandchild of the test binary and not of `child`. With
/// `$GOALCUP_FAKE_CLAUDE_LINGER` set (see `tests/fixtures/fake-claude.sh`), that harness sits
/// in `while :; do sleep 1; done` long after `goalcup start` itself is gone, and normally
/// only `Daemon::seal_entry` reaches it — which never runs when an earlier stage panics.
/// `spawn` below puts `child` at the head of a process group of its own
/// (`process_group(0)`, the same move `launch::spawn` makes for a harness the daemon must
/// later reach), so `Drop` can `killpg` the group and take the harness with it.
struct RunningStart {
    child: Option<std::process::Child>,
    stdout: Option<std::thread::JoinHandle<Vec<u8>>>,
    stderr: Option<std::thread::JoinHandle<Vec<u8>>>,
}

/// What a finished `goalcup start` left behind.
struct Finished {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl RunningStart {
    /// Spawn `command`, at the head of its own process group, taking both pipes onto
    /// reader threads at once.
    ///
    /// `process_group(0)` (unix only — this whole file is) is what lets `Drop` reach the
    /// harness underneath `child` and not only `child` itself; see the struct's doc.
    fn spawn(command: &mut std::process::Command) -> RunningStart {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("the goalcup binary runs");
        let stdout = child.stdout.take().map(drain);
        let stderr = child.stderr.take().map(drain);
        RunningStart {
            child: Some(child),
            stdout,
            stderr,
        }
    }

    /// Wait up to `patience` for it to exit, then join both drains.
    ///
    /// A poll and not a blocking `wait`, so the timeout is this test's and the failure names
    /// itself — a `wait` that never returned would be a hung test binary instead.
    fn finish(mut self, patience: Duration) -> Finished {
        let deadline = Instant::now() + patience;
        let status = loop {
            let child = self.child.as_mut().expect("finished once");
            match child
                .try_wait()
                .expect("`goalcup start` could not be waited on")
            {
                Some(status) => break status,
                None => assert!(
                    Instant::now() < deadline,
                    "stage 6: the seal terminated the harness and `goalcup start` had still \
                     not returned {patience:?} later — the participant's terminal is the \
                     thing left hanging"
                ),
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        self.child = None;
        Finished {
            code: status.code(),
            stdout: joined(self.stdout.take()),
            stderr: joined(self.stderr.take()),
        }
    }

    /// Wait for a fixture marker while keeping an early CLI exit diagnostic instead of
    /// turning it into an unrelated missing-file timeout.
    #[cfg(feature = "v1-compat")]
    fn wait_for_file(&mut self, path: &Path, patience: Duration) {
        let deadline = Instant::now() + patience;
        loop {
            if path.exists() {
                return;
            }
            let child = self.child.as_mut().expect("a running command");
            if let Some(status) = child.try_wait().expect("query the goalcup child") {
                self.child = None;
                let stdout = joined(self.stdout.take());
                let stderr = joined(self.stderr.take());
                panic!(
                    "{} never appeared because the goalcup command exited {:?}.\nstdout:\n{}\nstderr:\n{}",
                    path.display(),
                    status.code(),
                    stdout,
                    stderr
                );
            }
            assert!(
                Instant::now() < deadline,
                "{} never appeared within {patience:?} while the goalcup command stayed open",
                path.display()
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

impl Drop for RunningStart {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            kill_group(&mut child);
            let _ = child.wait();
        }
        let _ = self.stdout.take().map(std::thread::JoinHandle::join);
        let _ = self.stderr.take().map(std::thread::JoinHandle::join);
    }
}

/// Kill `child`'s whole process group, not only `child`.
///
/// `RunningStart::spawn` gave `child` a group of its own with `process_group(0)`, so on
/// unix its pid IS that group's id and one `killpg` reaches `child` and every process it
/// spawned — the harness, and the harness's own linger loop, all at once. Falls back to a
/// plain `Child::kill` on a platform this test does not otherwise run on, which reaches
/// `child` alone.
#[cfg(unix)]
fn kill_group(child: &mut std::process::Child) {
    let Ok(pid) = i32::try_from(child.id()) else {
        return;
    };
    // SAFETY: `killpg` takes an integer process-group id, dereferences no memory this
    // process owns, and writes none. `pid` is positive and is `child`'s own pid, which
    // `process_group(0)` at spawn made equal to its group id — so this signals exactly the
    // group `spawn` created and never this test binary's own group.
    unsafe {
        libc::killpg(pid, libc::SIGKILL);
    }
}

#[cfg(not(unix))]
fn kill_group(child: &mut std::process::Child) {
    let _ = child.kill();
}

/// One pipe, read to its end on a thread of its own so the child never blocks writing.
fn drain<R: std::io::Read + Send + 'static>(mut pipe: R) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = pipe.read_to_end(&mut buf);
        buf
    })
}

/// A drain's bytes as text, or an empty string where the thread was already taken or panicked.
fn joined(handle: Option<std::thread::JoinHandle<Vec<u8>>>) -> String {
    match handle.map(std::thread::JoinHandle::join) {
        Some(Ok(bytes)) => String::from_utf8_lossy(&bytes).to_string(),
        _ => String::new(),
    }
}

#[cfg(feature = "v1-compat")]
fn ready_path(gate: &Path) -> PathBuf {
    let mut name = gate.as_os_str().to_os_string();
    name.push(".ready");
    PathBuf::from(name)
}

/// The one entry this daemon holds, as the daemon itself lists it.
///
/// Asked of the daemon rather than parsed out of `goalcup start`'s stdout, because `start` is
/// still running here: it is blocked on a harness that will not exit until the seal, and a
/// test reading its pipe for the entry line would be holding the pipe open for the rest of
/// the run.
fn the_one_entry(client: &dyn DaemonClient) -> String {
    let deadline = Instant::now() + support::PATIENCE;
    loop {
        let last = match client.entries() {
            Ok(entries) => match entries.as_slice() {
                [one] => return one.entry_id.clone(),
                other => format!("{} entries", other.len()),
            },
            Err(e) => format!("the entries route answered {e}"),
        };
        assert!(
            Instant::now() < deadline,
            "waited {:?} for `goalcup start` to create exactly one entry; last: {last}",
            support::PATIENCE
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Read the preview pins once the live daemon's transient upload slot is free.
///
/// The fixture deliberately pumps every 200 ms, so the CLI's production 200 ms retry can
/// phase-lock with that cadence. This test-only read uses a shorter interval and asserts on
/// the real daemon's response rather than weakening the snapshot to a cache-derived value.
fn preview_pins(client: &dyn DaemonClient, entry_id: &str) -> ResumePreviewInfo {
    let deadline = Instant::now() + support::PATIENCE;
    loop {
        match client.resume_preview(entry_id) {
            Ok(info) => return info,
            Err(ControlError::Busy(message)) => {
                assert!(
                    Instant::now() < deadline,
                    "waited {:?} for preview pins for {entry_id}; last busy response: {message}",
                    support::PATIENCE
                );
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(error) => panic!("the daemon refused preview pins for {entry_id}: {error}"),
        }
    }
}

/// The pid `goalcup start` registered for this run, out of the record it wrote after the
/// hand-off.
///
/// Waited for rather than read once: `register_process` and the record rewrite that follows
/// it race the harness's own first post, and this test reaches the record from the arming
/// side. A `None` here means the pid line has not been written yet, not that nothing was
/// launched — the entry armed, so something is running in that workspace.
fn the_launched_pid(cfg: &Path, entry_id: &str) -> u32 {
    let deadline = Instant::now() + support::PATIENCE;
    loop {
        let last = match goalcup_cli::entries::read_entry(cfg, entry_id) {
            Ok(rec) => match rec.last_pid {
                Some(pid) => return pid,
                None => "the record carries no pid yet".to_string(),
            },
            Err(e) => format!("the record could not be read: {e}"),
        };
        assert!(
            Instant::now() < deadline,
            "waited {:?} for `goalcup start` to record the pid it handed over; last: {last}",
            support::PATIENCE
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Wait until the local cache says the daemon acknowledged one exact process hand-off.
///
/// `last_pid` is written before the first `/process` request byte so a crash can recover the
/// body. It is therefore not, by itself, proof that the daemon accepted the process. The
/// pending body disappears only after the narrow 204 acknowledgement; pairing those facts
/// lets the finite-exit fixture stop naturally without racing PID reuse on macOS.
#[cfg(feature = "v1-compat")]
fn the_registered_record(
    cfg: &Path,
    entry_id: &str,
    previous_pid: Option<u32>,
) -> goalcup_cli::entries::EntryRecord {
    let deadline = Instant::now() + support::PATIENCE;
    loop {
        let last = match goalcup_cli::entries::read_entry(cfg, entry_id) {
            Ok(record)
                if record.last_pid.is_some()
                    && record.last_pid != previous_pid
                    && record.pending_process_registration.is_none()
                    && record.pending_process_cleanup.is_none() =>
            {
                return record;
            }
            Ok(record) => format!(
                "pid {:?}, generation {}, pending registration {}, pending cleanup {}",
                record.last_pid,
                record.process_generation,
                record.pending_process_registration.is_some(),
                record.pending_process_cleanup.is_some()
            ),
            Err(error) => format!("the record could not be read: {error}"),
        };
        assert!(
            Instant::now() < deadline,
            "waited {:?} for entry {entry_id}'s process hand-off acknowledgement; last: {last}",
            support::PATIENCE
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
#[ignore = "spawns the daemon and the devserver; run by name with -- --ignored"]
fn the_cli_drives_the_daemon_and_the_verifier_reads_what_it_wrote() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let root = tmp.path();

    // ---- Stage 0. The server the daemon uploads to. Its DEFAULT key IS
    // `PLACEHOLDER_SERVER_PUBLIC_KEY` — the single entry on `SERVER_PUBLIC_KEYS` — so this
    // trace resolves `KeySource::Pinned` with no `extra_server_keys` at all, and what keeps
    // that honest is V13's `placeholder_server_key`, suppressed here by `dev_mode` exactly as
    // `goalcup-verify --dev` suppresses it and NOT suppressed in a judged run.
    let dev = support::spawn_devserver();
    assert_eq!(
        dev.public_key_hex(),
        goalcup_verifier::PLACEHOLDER_SERVER_PUBLIC_KEY,
        "the devserver's default key is this build's pinned one"
    );

    let daemon = support::spawn_daemon(&support::daemon_binary(), root, &dev.url());
    let cfg = daemon.config_dir();
    // `goalcup start`'s first refusal is a machine that has never run `goalcup auth`, and no
    // device-code flow exists in this tree to run against: `goalcup-devserver` serves the
    // upload API and nothing else. A `--dev` start needs no grant (the fourth admission of
    // the one door), so none is written: this run proves the no-grant path on a real daemon.
    let bin = support::fake_claude_on_path(root);
    // …and the home that launch reads its `[home_seed]` out of. `support::goalcup` pins
    // `$HOME` to `<root>/home`, which `spawn_daemon` created empty; a start against an empty
    // one is REFUSED because the run would meet a theme picker and a login
    // screen instead of its goal. Signed in here with an invented account, never the
    // developer's.
    support::sign_in_the_run_home(root);

    // ---- Stage 1. The control API is up and answering, before anything else is blamed.
    let client = goalcup_cli::daemon::HttpDaemonClient::from_config_dir(&cfg)
        .expect("the daemon wrote a control.json this build can read");
    let health = client.health().expect("the control API must be reachable");
    assert_eq!(
        health.control_api_version,
        goalcup_protocol::control::CONTROL_API_VERSION
    );

    // ---- Stage 2. The run, typed, and LEFT RUNNING. `--no-open` because a test that opened
    // a browser would open the developer's; `--dev` because this is a development run and the
    // record, the seal's printed verify line and the verdict all have to say so;
    // `$GOALCUP_FAKE_CLAUDE_LINGER` because a harness that had already exited would let the
    // seal's kill pass without doing anything, and the hand-off is what stage 6 is about.
    let running = RunningStart::spawn(
        support::goalcup(&cfg, &bin, root)
            .args(["start", "--harness", CONNECTOR, "--dev", "--no-open"])
            .env("GOALCUP_FAKE_CLAUDE_LINGER", "1"),
    );
    let entry_id = the_one_entry(&client);

    // ---- Stage 3. The entry is durably Started with a bound session, and the signed status
    // says so against the key the ENTRY CACHE pinned — not against the key named inside the
    // document being checked. `wait_for_arming` deliberately waits through the final bind so
    // the byte-exact chain snapshot in stage 4 cannot race its autonomous health tail.
    support::wait_for_arming(&client, &entry_id);
    let rec = goalcup_cli::entries::read_entry(&cfg, &entry_id)
        .expect("`goalcup start` left a record of this run");
    assert!(
        rec.dev,
        "the run came through the one door and the record says so"
    );
    let signed = client.status(&entry_id).expect("the signed status route");
    signed
        .verify_with(&rec.run_public_key)
        .expect("the daemon signs with the key it handed the CLI");
    assert_eq!(
        signed.payload.arming,
        ArmingState::Started,
        "stage 3: the entry never reached a durable Started state: {:?} — {}",
        signed.payload.arming,
        signed.payload.detail
    );
    assert!(
        signed
            .payload
            .session_id
            .as_deref()
            .is_some_and(|session| !session.trim().is_empty()),
        "stage 3: Started lacked the bound session id: {:?}",
        signed.payload.session_id
    );

    // ---- Stage 4. The chain, before the seal, so "nothing was captured" is diagnosed as
    // itself rather than as "the verifier rejected the trace".
    //
    // **This is the symlink stage.** `spawn_daemon` points `workspace_parent` at a SYMLINK, so
    // the path the daemon was CONFIGURED with and the path `pwd -P` reports inside the
    // launched harness are two different strings for one directory. There is
    // one spelling on the wire — the canonical one — and these three lines are what says so:
    // the record `goalcup start` cached IS what `canonicalize` returns, and it is NOT the
    // spelling the daemon was configured with. `WorkspaceInfo.workspace_requested` carries
    // that second spelling and no process in this test ever sees it: `goalcup start` neither
    // prints nor caches it, which is the point of a display-only field, and
    // `goalcup-daemon/tests/control_api.rs` asserts its value where it exists.
    //
    // A daemon that bound the entry to the configured spelling would derive `claude_slug` from
    // one name while the transcript was written under the other: the `main` glob would match
    // nothing and transcript capture would be silently zero for a `cwd` that did not compare
    // equal. This test's first concern is exactly that. The chain wait below guards the
    // transcript, while stage 5 pins the retained signed legacy field shape: V1 has no receiver,
    // so the status hardcodes `posts_dropped` to zero rather than providing live loss evidence.
    let workspace = Path::new(&rec.workspace);
    let canonical = std::fs::canonicalize(workspace).expect("the workspace canonicalises");
    assert_eq!(
        canonical.as_path(),
        workspace,
        "the entry cache holds the CANONICAL workspace and nothing else (D16 round 2)"
    );
    // `spawn_daemon`'s own table: `workspaces` is the symlink the daemon is configured with,
    // and the daemon names a workspace `workspace_parent/<entry id>`.
    let requested = root.join("workspaces").join(&entry_id);
    assert_ne!(
        requested.as_path(),
        workspace,
        "the fixture's workspace parent is a symlink, so the requested spelling must differ \
         from the canonical one — otherwise this test is passing for a reason about the \
         machine it ran on"
    );
    let trace = goalcup_cli::seal::trace_dir(Path::new(&rec.artifact_root));
    support::wait_for_chain(&trace, LEG1_RECORDS, LEG2_POSTS);

    // ---- Stage 5. `goalcup status` in FRESH processes: the key each checks the document
    // against is the one `start` wrote into the entry cache, and nothing else on this machine
    // holds it.
    //
    // The first is README's walkthrough line, typed as README types it — no entry id at all,
    // resolved against "the newest run started from this machine".
    let default_status = support::goalcup(&cfg, &bin, root)
        .arg("status")
        .output()
        .expect("the goalcup binary runs");
    let default_out = String::from_utf8_lossy(&default_status.stdout).to_string();
    assert_eq!(
        default_status.status.code(),
        Some(0),
        "stage 5: bare `goalcup status` refused.\nstdout:\n{default_out}\nstderr:\n{}",
        String::from_utf8_lossy(&default_status.stderr)
    );
    assert!(
        default_out.contains(&entry_id),
        "stage 5: bare `goalcup status` resolved to some other run:\n{default_out}"
    );
    let status = support::goalcup(&cfg, &bin, root)
        .args(["status", &entry_id, "--json"])
        .output()
        .expect("the goalcup binary runs");
    assert_eq!(
        status.status.code(),
        Some(0),
        "stage 5: `goalcup status` refused: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    let doc: goalcup_protocol::control::SignedStatus =
        serde_json::from_slice(&status.stdout).expect("`--json` is one signed document");
    doc.verify_with(&rec.run_public_key)
        .expect("the document a fresh `goalcup status` printed verifies under the pinned key");
    assert_eq!(doc.payload.entry_id, entry_id);
    // Preserve the signed legacy field shape. Transcript-only V1 has no receiver path, so this
    // hardcoded zero is not live no-loss evidence.
    assert_eq!(
        doc.payload.posts_dropped, 0,
        "stage 5: the daemon dropped {} posts — {}",
        doc.payload.posts_dropped, doc.payload.detail
    );

    // ---- Stage 6. The seal, the export, the two lines it prints, and the kill.
    //
    // The pid first, while the harness is certainly still alive. It is read out of the record
    // `goalcup start` wrote after `register_process` — the same field `goalcup resume`'s lock
    // check reads — because after the kill there is nothing left to ask.
    let harness_pid = the_launched_pid(&cfg, &entry_id);
    assert!(
        goalcup_protocol::process::is_alive(harness_pid),
        "stage 6: the harness must still be running when the seal arrives, or the kill below \
         proves nothing"
    );
    // No entry id on the seal: README's other walkthrough line, and the same default
    // `goalcup status` just resolved.
    let sealed = support::goalcup(&cfg, &bin, root)
        .arg("seal")
        .output()
        .expect("the goalcup binary runs");
    let seal_out = String::from_utf8_lossy(&sealed.stdout).to_string();
    assert_eq!(
        sealed.status.code(),
        Some(0),
        "stage 6: `goalcup seal` refused.\nstdout:\n{seal_out}\nstderr:\n{}",
        String::from_utf8_lossy(&sealed.stderr)
    );
    assert!(
        seal_out.contains(&format!("trace:  {}", trace.display())),
        "the seal names the daemon's own export directory:\n{seal_out}"
    );
    // Byte for byte the line `end_to_end.rs` asserts against the mock, and the line run
    // below.
    let verify_line = format!("verify: goalcup-verify --dev {}", trace.display());
    assert!(
        seal_out.contains(&verify_line),
        "the seal prints the invocation this test then runs:\n{seal_out}"
    );
    // **The self-check, over one spelling of the workspace.** `goalcup seal` walks the path in
    // the entry cache and the daemon's manifest names the path the entry is bound to; under a
    // symlinked parent those would be two names for one directory, and the report would open
    // with a difference line about a workspace nothing had moved. They are one name, so every
    // file agrees and there is one sentence.
    assert!(
        seal_out.contains("agrees with it in every file"),
        "stage 6: the seal's self-check found differences:\n{seal_out}"
    );
    assert!(
        !seal_out.contains("disagrees with it in"),
        "stage 6: the seal's self-check found differences:\n{seal_out}"
    );
    for f in [
        "trace.toml",
        "chain.jsonl",
        "receipt.json",
        "baseline.json",
        "seal.json",
        "acks.jsonl",
    ] {
        assert!(
            trace.join(f).exists(),
            "stage 6: `TraceExport` must have written {f} into {}",
            trace.display()
        );
    }
    // **The hand-off, from the only side a test can watch it from.** The harness was alive
    // when the seal arrived; `Daemon::seal_entry` terminated it after writing the chain;
    // `goalcup start` was blocked in `child.wait()` and comes back because of that kill. All
    // three of those are one observation: the command this test left running exits, under
    // zero, with the pid it registered out of the process table.
    let finished = running.finish(HANDOFF_PATIENCE);
    let start_out = finished.stdout.clone();
    assert_eq!(
        finished.code,
        Some(0),
        "stage 6: `goalcup start` left under a non-zero number after a run that \
         sealed.\nstdout:\n{start_out}\nstderr:\n{}",
        finished.stderr
    );
    assert!(
        start_out.contains(&format!("entry {entry_id}")),
        "stage 6: `goalcup start` names the entry it created:\n{start_out}"
    );
    assert!(
        start_out.contains("Recording. Leave this running."),
        "stage 6: `goalcup start` said it was holding the run open:\n{start_out}"
    );
    support::wait_until_gone(harness_pid);
    // One more acknowledgement, so the seal link itself is anchored and no record sits above
    // the highest acknowledged sequence. The daemon's upload cadence does it; this waits for
    // the evidence rather than for a duration.
    support::wait_for_seal_ack(&trace);

    // ---- Stage 7. The transcript-only verdict, over the library and then over the binary.
    let verdict = verify_trace_with_options(
        &trace,
        VerifyOptions {
            dev_mode: true,
            ..VerifyOptions::default()
        },
    )
    .expect("a trace the daemon produced must at least be readable");
    assert_eq!(verdict.connector_id, CONNECTOR);
    assert_eq!(
        verdict.key_source,
        goalcup_verifier::KeySource::Pinned,
        "the devserver signs with the placeholder, which IS this build's pinned key"
    );
    assert_eq!(verdict.checks.len(), 13, "V1-V13");
    assert!(
        !verdict
            .checks
            .iter()
            .flat_map(|c| &c.findings)
            .any(|f| f.code == "chain_broken"),
        "the chain the daemon wrote must verify against the bytes the verifier recomputes"
    );
    // V1 no longer emits hook posts. Keep the live verifier checks visible: the transcript is
    // readable and chained, but its unpaired user slots and tool call must remain findings
    // rather than being hidden by this end-to-end contract.
    let mut review_findings = verdict
        .review_queue
        .iter()
        .map(|finding| (finding.code.as_str(), finding.severity))
        .collect::<Vec<_>>();
    review_findings.sort_unstable();
    let mut expected_review_findings = vec![
        (
            "leg1_call_without_leg2_post",
            goalcup_verifier::checks::Severity::Fail,
        ),
        (
            "leg1_without_leg2_post",
            goalcup_verifier::checks::Severity::Flag,
        ),
        (
            "leg1_without_leg2_post",
            goalcup_verifier::checks::Severity::Flag,
        ),
        (
            "unwitnessed_keep_going",
            goalcup_verifier::checks::Severity::Flag,
        ),
    ];
    expected_review_findings.sort_unstable();
    assert_eq!(
        review_findings, expected_review_findings,
        "stage 7: transcript-only capture must retain the exact pairing findings"
    );
    assert_eq!(
        verdict.overall,
        goalcup_verifier::checks::Severity::Fail,
        "stage 7: transcript-only capture must have a Fail verdict"
    );
    // Transcript-only V1 has no leg-2 human-message witnesses.
    assert_eq!(verdict.human_messages.len(), 0);

    // The binary produces the same document over the same directory. It `println!`s, so its
    // stdout carries a trailing newline `to_json()` does not.
    let out = std::process::Command::new(support::verify_binary())
        .arg(&trace)
        .arg("--dev")
        .output()
        .expect("goalcup-verify runs");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim_end(),
        verdict.to_json().expect("the verdict serialises"),
        "the printed line and the library door must produce one document"
    );
    assert_eq!(out.status.code(), Some(verdict.exit_code()));

    drop(daemon);
    drop(dev);
}

/// The print-only resume leg, through the binary, against the real daemon.
///
/// The in-process control test proves the registry and capture invariant directly. This
/// ignored test proves the participant-facing command keeps those bytes and its registered,
/// live harness intact; the seal afterwards must still reach that original handle.
#[test]
#[ignore = "spawns the daemon and the devserver; run by name with -- --ignored"]
fn print_launch_previews_a_real_resume_without_changing_the_run() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let root = tmp.path();
    let dev = support::spawn_devserver();
    let daemon = support::spawn_daemon(&support::daemon_binary(), root, &dev.url());
    let cfg = daemon.config_dir();
    let bin = support::fake_claude_on_path(root);
    support::sign_in_the_run_home(root);
    let client = goalcup_cli::daemon::HttpDaemonClient::from_config_dir(&cfg)
        .expect("the daemon wrote a control.json this build can read");

    let running = RunningStart::spawn(
        support::goalcup(&cfg, &bin, root)
            .args(["start", "--harness", CONNECTOR, "--dev", "--no-open"])
            .env("GOALCUP_FAKE_CLAUDE_LINGER", "1"),
    );
    let entry_id = the_one_entry(&client);
    support::wait_for_arming(&client, &entry_id);
    let record =
        goalcup_cli::entries::read_entry(&cfg, &entry_id).expect("the start wrote its entry cache");
    let trace = goalcup_cli::seal::trace_dir(Path::new(&record.artifact_root));
    support::wait_for_chain(&trace, LEG1_RECORDS, LEG2_POSTS);
    let harness_pid = the_launched_pid(&cfg, &entry_id);
    assert!(
        goalcup_protocol::process::is_alive(harness_pid),
        "the preview must start from a still-registered live harness"
    );
    let preview_before = preview_pins(&client, &entry_id);
    let cache_path = goalcup_cli::entries::entry_path(&cfg, &entry_id);
    let cache_before = std::fs::read(&cache_path).expect("the cache bytes");

    let preview = support::goalcup(&cfg, &bin, root)
        .args(["resume", &entry_id, "--print-launch", "--no-open"])
        .output()
        .expect("the goalcup binary runs");
    let preview_out = String::from_utf8_lossy(&preview.stdout).to_string();
    assert_eq!(
        preview.status.code(),
        Some(0),
        "the read-only preview refused.\nstdout:\n{preview_out}\nstderr:\n{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    assert!(preview_out.contains("--resume"), "{preview_out}");
    assert!(
        !preview_out.contains("caught up"),
        "a preview must not claim a capture catch-up: {preview_out}"
    );
    assert_eq!(
        std::fs::read(&cache_path).expect("the cache remains"),
        cache_before,
        "print-launch rewrote the participant's cache"
    );
    assert_eq!(
        preview_pins(&client, &entry_id),
        preview_before,
        "print-launch changed the daemon's launch pins or process handoff phase"
    );
    assert!(
        goalcup_protocol::process::is_alive(harness_pid),
        "print-launch stopped the harness it only needed to describe"
    );

    // If the preview had cleared the daemon's registered handle, this seal would leave the
    // lingerer alive. Its termination proves the original hand-off survived the preview.
    let sealed = support::goalcup(&cfg, &bin, root)
        .args(["seal", &entry_id])
        .output()
        .expect("the goalcup binary runs");
    assert_eq!(
        sealed.status.code(),
        Some(0),
        "seal after preview refused: {}",
        String::from_utf8_lossy(&sealed.stderr)
    );
    let finished = running.finish(HANDOFF_PATIENCE);
    assert_eq!(
        finished.code,
        Some(0),
        "{}\n{}",
        finished.stdout,
        finished.stderr
    );
    assert!(
        !goalcup_protocol::process::is_alive(harness_pid),
        "the seal after preview did not reach the original registered harness"
    );

    drop(daemon);
    drop(dev);
}

/// The resume leg, through the binary, against the real daemon: `start` → the harness exits
/// → `goalcup resume` → the daemon re-registers what it relaunched → `seal`.
///
/// The cached session id is the one the daemon MINTED at `create_workspace`, and
/// `ResumeInfo.session_id` is the one the arming gate BOUND; a resume must not refuse when
/// they differ. Only `claude-code-cli` pins the minted id at launch (`--session-id`), so on
/// Codex the harness mints its own and the two differ on every first resume.
///
/// **What this proves and what it does not.** Claude Code pins the id, so the two coincide
/// here and this test cannot see the difference; the mock, which answers a bound id that
/// differs by construction, is where that half is asserted
/// (`tests/resume_locks.rs`). What is here is the FLOW against a real daemon: a resume that
/// catches the tail up, relaunches from `[resume]`, hands the new process over, writes the
/// daemon's session id back into the cache, and leaves an entry that still seals.
///
/// The harness does NOT linger in this test: `$GOALCUP_FAKE_CLAUDE_LINGER` is unset. It
/// writes its records, waits only until the daemon has durably accepted its process hand-off,
/// then exits by itself and `goalcup start` comes back — which is the state a participant is
/// in when they reach for `goalcup resume`. The acknowledgement gate matters on macOS: an
/// unconstrained fixture can exit before `/process`, have its PID recycled immediately, and
/// correctly trip the process-instance guard instead of testing the intended resume flow.
#[cfg(feature = "v1-compat")]
#[test]
#[ignore = "spawns the daemon and the devserver; run by name with -- --ignored"]
fn a_resume_relaunches_the_harness_and_the_entry_still_seals() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let root = tmp.path();
    let dev = support::spawn_devserver();
    let daemon = support::spawn_daemon(&support::daemon_binary(), root, &dev.url());
    let cfg = daemon.config_dir();
    let bin = support::fake_claude_on_path(root);
    // The same signing-in as the test above, and for the same refusal — see there. Both
    // launches of this run seed from it: the start's and the resume's.
    support::sign_in_the_run_home(root);
    let client = goalcup_cli::daemon::HttpDaemonClient::from_config_dir(&cfg)
        .expect("the daemon wrote a control.json this build can read");

    // ---- Stage 1. The run, and the harness ends it by exiting on its own after the exact
    //      process hand-off has been acknowledged.
    let start_exit_gate = root.join("start-exit-after-registration");
    let mut running_start = RunningStart::spawn(
        support::goalcup(&cfg, &bin, root)
            .args(["start", "--harness", CONNECTOR, "--dev", "--no-open"])
            .env("GOALCUP_FAKE_CLAUDE_EXIT_GATE", &start_exit_gate),
    );
    running_start.wait_for_file(&ready_path(&start_exit_gate), support::PATIENCE);
    let entry_id = the_one_entry(&client);
    support::wait_for_arming(&client, &entry_id);
    let before = the_registered_record(&cfg, &entry_id, None);
    let trace = goalcup_cli::seal::trace_dir(Path::new(&before.artifact_root));
    support::wait_for_chain(&trace, LEG1_RECORDS, LEG2_POSTS);
    let first_capture = support::capture_counts(&trace)
        .expect("stage 1: the first launch wrote an exported capture snapshot");
    std::fs::write(&start_exit_gate, b"registered")
        .expect("release the finite start fixture after registration");
    let started = running_start.finish(HANDOFF_PATIENCE);
    assert_eq!(
        started.code,
        Some(0),
        "stage 1: `goalcup start` refused.\nstdout:\n{}\nstderr:\n{}",
        started.stdout,
        started.stderr
    );

    // ---- Stage 2. The resume, typed the way README types it.
    let resume_exit_gate = root.join("resume-exit-after-registration");
    let mut running_resume = RunningStart::spawn(
        support::goalcup(&cfg, &bin, root)
            .args(["resume", &entry_id, "--no-open"])
            .env("GOALCUP_FAKE_CLAUDE_EXIT_GATE", &resume_exit_gate),
    );
    running_resume.wait_for_file(&ready_path(&resume_exit_gate), support::PATIENCE);
    let after = the_registered_record(&cfg, &entry_id, before.last_pid);
    // The resume has to add a complete batch AFTER the snapshot, and its transcript records
    // must carry the fixture's distinct `*-resume-*` identities. Two aggregate batches alone
    // could be a replay of launch one's links.
    support::wait_for_resumed_capture(&trace, first_capture, LEG1_RECORDS, LEG2_POSTS);
    std::fs::write(&resume_exit_gate, b"registered")
        .expect("release the finite resume fixture after registration");
    let resumed = running_resume.finish(HANDOFF_PATIENCE);
    let resume_out = resumed.stdout.clone();
    assert_eq!(
        resumed.code,
        Some(0),
        "stage 2: `goalcup resume` refused.\nstdout:\n{resume_out}\nstderr:\n{}",
        resumed.stderr
    );
    assert!(
        resume_out.contains("caught up"),
        "stage 2: R16 — the tail is caught up before anything new is appended:\n{resume_out}"
    );
    assert!(
        resume_out.contains(goalcup_cli::commands::resume::KEEP_GOING),
        "stage 2: the one message the rules allow is on the screen:\n{resume_out}"
    );

    // ---- Stage 3. The daemon re-registered what the resume relaunched, and this machine's
    //      record followed both the pid and the daemon's own session id.
    assert!(
        after.last_pid.is_some() && after.last_pid != before.last_pid,
        "stage 3: the relaunched process was handed over and the record follows it: {:?} -> \
         {:?}",
        before.last_pid,
        after.last_pid
    );
    let signed = client.status(&entry_id).expect("the signed status route");
    signed
        .verify_with(&after.run_public_key)
        .expect("the daemon signs with the key the entry cache pinned");
    // Unconditional: a `None` here is the daemon answering a resumed run with no session
    // bound at all, which is a defect this stage exists to catch and not a shape the test
    // should quietly skip past.
    let session = signed.payload.session_id.as_ref().expect(
        "stage 3: the daemon's signed status names no session after a resume — nothing for \
         the next resume to select",
    );
    assert_eq!(
        &after.session_id, session,
        "stage 3: the cache holds the session the DAEMON is binding, which is what the next \
         resume selects"
    );

    // ---- Stage 4. The entry is still one entry, and it still seals.
    let sealed = support::goalcup(&cfg, &bin, root)
        .args(["seal", &entry_id])
        .output()
        .expect("the goalcup binary runs");
    let seal_out = String::from_utf8_lossy(&sealed.stdout).to_string();
    assert_eq!(
        sealed.status.code(),
        Some(0),
        "stage 4: `goalcup seal` refused after a resume.\nstdout:\n{seal_out}\nstderr:\n{}",
        String::from_utf8_lossy(&sealed.stderr)
    );
    assert!(
        seal_out.contains(&format!("trace:  {}", trace.display())),
        "stage 4: the seal names the daemon's own export directory:\n{seal_out}"
    );
    for f in ["trace.toml", "chain.jsonl", "receipt.json", "seal.json"] {
        assert!(
            trace.join(f).exists(),
            "stage 4: `TraceExport` must have written {f} into {}",
            trace.display()
        );
    }
    // The trace is only verifier-ready once the post-seal checkpoint is anchored. This is
    // the same content wait stage 7 uses for the first lifecycle E2E, applied here before
    // the resumed trace's transcript-only Fail verdict assertion.
    support::wait_for_seal_ack(&trace);

    // ---- Stage 5. What each launch carried in, in the sealed chain and then in the
    //      verdict.
    //
    // This is the only test in the tree where the record is written by the real CLI, chained
    // by the real daemon and read back out of a sealed trace — `home_seeding.rs` covers the
    // seeding function, `compliant.rs` covers the verdict over a planted chain, and neither
    // of them launches anything. **Two launches, two records**: the start's and the resume's,
    // which is what "one per launch" means on the one entry that has more than one.
    let seeded = home_seeded_in(&trace);
    assert_eq!(
        seeded.len(),
        2,
        "stage 5: one `HomeSeeded` link per launch — the start's and the resume's — and this \
         chain carries {}:\n{}",
        seeded.len(),
        support::summarise_chain(&trace)
    );

    // The names, against the shipped manifest rather than a list copied into this file: the
    // fixture home carries every key `[home_seed] keys` names, so a manifest that grows one
    // fails here until `support::signed_in_claude_json` grows it too.
    let spec = support::connector(CONNECTOR)
        .manifest
        .home_seed
        .expect("`claude-code-cli` seeds its home");
    for (nth, record) in seeded.iter().enumerate() {
        assert_eq!(record.connector, CONNECTOR);
        assert_eq!(
            record.from, "{home}/.claude.json",
            "stage 5: the record names the manifest's spelling and never the path it \
             resolved to on this machine"
        );
        assert_eq!(
            record.keys.iter().map(|k| &k.name).collect::<Vec<_>>(),
            spec.keys.iter().collect::<Vec<_>>(),
            "stage 5: launch {nth} carried the manifest's key list, in its order"
        );
        let value = |name: &str| {
            record
                .keys
                .iter()
                .find(|k| k.name == name)
                .unwrap_or_else(|| panic!("stage 5: `{name}` is on the manifest's list"))
                .value
                .clone()
        };
        assert_eq!(
            value("oauthAccount"),
            None,
            "stage 5: `oauthAccount` is an object holding an account — NAMED and never \
             quoted, in a chain that is published"
        );
        // …and the two shapes the rule does carry, so "named only" is a rule and not this
        // record being empty: the browser-tools switch is a boolean and the theme is a short
        // string.
        assert_eq!(
            value("claudeInChromeDefaultEnabled"),
            Some(serde_json::Value::Bool(true))
        );
        assert_eq!(value("theme"), Some(serde_json::json!("dark")));
        // The one entry the tracker DECIDED rather than copied, kept on a list of its own
        // — and named without this machine's workspace path.
        assert_eq!(
            record
                .written
                .iter()
                .map(|k| (k.name.clone(), k.value.clone()))
                .collect::<Vec<_>>(),
            spec.trust_keys
                .iter()
                .map(|k| (k.clone(), Some(serde_json::Value::Bool(true))))
                .collect::<Vec<_>>(),
            "stage 5: launch {nth} pre-answered the harness's per-project safety question \
             for this run's own workspace, and the record says so"
        );
        assert!(
            record.files.is_empty(),
            "stage 5: this connector copies no whole files"
        );
    }
    assert!(
        seeded[0].keys_unchanged.is_empty(),
        "stage 5: the start seeded a home that held nothing yet: {:?}",
        seeded[0].keys_unchanged
    );
    assert_eq!(
        seeded[1].keys_unchanged, spec.keys,
        "stage 5: the resume seeded the same home again, so every key was already held at \
         the value being written — each was written a second time, identically, and the \
         merge changed nothing"
    );

    // And the verdict says the same thing, which is the surface a reviewer of somebody else's
    // trace reads. The carried-settings list is a fact list, but the resumed trace as a whole
    // must retain the transcript-only pairing findings for both launches.
    let verdict = verify_trace_with_options(
        &trace,
        VerifyOptions {
            dev_mode: true,
            ..VerifyOptions::default()
        },
    )
    .expect("a trace the daemon produced must at least be readable");
    assert_eq!(
        verdict.carried_settings, seeded,
        "stage 5: `carried_settings` is the chain's own records, in launch order"
    );
    // The transcript-only V1 contract intentionally leaves the verifier's pairing findings in
    // the queue. Assert their codes instead of weakening the product checks to make the queue
    // look empty.
    let mut review_findings = verdict
        .review_queue
        .iter()
        .map(|finding| (finding.code.as_str(), finding.severity))
        .collect::<Vec<_>>();
    review_findings.sort_unstable();
    let mut expected_review_findings = vec![
        (
            "leg1_call_without_leg2_post",
            goalcup_verifier::checks::Severity::Fail,
        ),
        (
            "leg1_without_leg2_post",
            goalcup_verifier::checks::Severity::Flag,
        ),
        (
            "leg1_without_leg2_post",
            goalcup_verifier::checks::Severity::Flag,
        ),
        (
            "leg1_without_leg2_post",
            goalcup_verifier::checks::Severity::Flag,
        ),
        (
            "leg1_without_leg2_post",
            goalcup_verifier::checks::Severity::Flag,
        ),
        (
            "unwitnessed_keep_going",
            goalcup_verifier::checks::Severity::Flag,
        ),
        (
            "unwitnessed_keep_going",
            goalcup_verifier::checks::Severity::Flag,
        ),
        (
            "unwitnessed_keep_going",
            goalcup_verifier::checks::Severity::Flag,
        ),
    ];
    expected_review_findings.sort_unstable();
    assert_eq!(
        review_findings, expected_review_findings,
        "stage 5: resumed transcript-only capture must retain the exact pairing findings"
    );
    assert_eq!(
        verdict.overall,
        goalcup_verifier::checks::Severity::Fail,
        "stage 5: resumed transcript-only capture must have a Fail verdict"
    );
    let json = verdict.to_json().expect("the verdict serialises");
    assert!(
        !json.contains("participant@example.test"),
        "stage 5: the verdict quotes an address out of the participant's own file"
    );
    let records = serde_json::to_string(&verdict.carried_settings).expect("the records do");
    assert!(
        !records.contains(&root.display().to_string()),
        "stage 5: and the records name no path off this machine — the manifest's spelling \
         for the file read, and the trust keys without the workspace they were written for"
    );

    drop(daemon);
    drop(dev);
}

/// Every `HomeSeeded` record in a sealed trace's chain, in chain order.
///
/// Read out of `chain.jsonl` by hand rather than through the verifier, so stage 5's first
/// claim is about the bytes the daemon wrote: the verdict's own list is asserted against
/// this one afterwards, and a reader that went through the verifier for both would be
/// comparing one function with itself.
#[cfg(feature = "v1-compat")]
fn home_seeded_in(trace: &Path) -> Vec<goalcup_protocol::events::HomeSeedRecord> {
    let path = trace.join(goalcup_protocol::export::CHAIN_FILE);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|line| {
            let v: serde_json::Value = serde_json::from_str(line).ok()?;
            match v["body"]["kind"].as_str() {
                Some("home_seeded") => Some(
                    serde_json::from_value(v["body"]["content"].clone())
                        .unwrap_or_else(|e| panic!("a `home_seeded` link this build wrote: {e}")),
                ),
                _ => None,
            }
        })
        .collect()
}
