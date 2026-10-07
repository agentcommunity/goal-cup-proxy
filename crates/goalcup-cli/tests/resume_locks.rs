//! `goalcup resume`: the lock that stops a relaunch and what the participant is told to do
//! about it, the daemon's own refusals on the right rungs, and the relaunch that hands the
//! new process back.
//!
//! # Why the fixtures are not the shipped connectors directory
//!
//! Every test below whose subject is the relaunched **argv** uses
//! [`support::connectors_dir_with_a_resume_that_exits`], which is the shipped connector with
//! `[launch] program` rewritten and **`[resume] args` untouched**. `goalcup resume` spawns
//! the resume recipe and waits for it: against the shipped `program` that is `claude` or
//! `codex`, which hangs on a machine where the harness is installed and fails
//! on a machine where it is not. The argv under test — the thing those assertions are about
//! — is still the shipped rulebook's.
//!
//! The two tests whose subject is a child that must be **stopped** use
//! [`support::connectors_dir_with_a_resume_that_outlives_a_kill`] instead, and its own item
//! says why it is the one fixture here that rewrites `[resume] args`.
//!
//! The tests that never reach a launch at all — every lock refusal — use the shipped
//! directory unchanged, because their subject is the shipped `lock_paths[]`.
//!
//! # Why several tests hold [`support::one_interrupt_at_a_time`]
//!
//! One of them raises a real `SIGINT` at this process, and the flag that handler sets is
//! process-global. `cargo test` runs a binary's tests on a pool of threads, so without the
//! lock a sibling in the middle of its own relaunch would read that flag and kill a child
//! nobody interrupted.
//!
//! # Why the entry id is read off the mock
//!
//! `MockDaemonClient::armed_ok` names its entry `entry-mock-0001` and
//! `support::cfg_with_entry` writes the record under the id the `WorkspaceInfo` carries, so
//! a hard-coded id would name a record no fixture in this tree writes and every test would
//! assert the "this machine has no record of that entry" refusal instead of the thing it is
//! about.
#![cfg(feature = "v1-compat")]

mod support;

use goalcup_cli::commands::resume::{LockRoots, check_locks, resume_with};
use goalcup_protocol::control::{ControlError, ProcessHandoffCancelRequest, ResumeRequest};
use goalcup_protocol::manifest::KillStrategy;
use goalcup_protocol::process::{ProcessHandle, ProcessRegistration};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The mock, the configuration directory holding its record, and the entry id both are
/// about — the three every test here opens with.
///
/// `set_cached_connector` is the one correction: `cfg_with_entry` writes `claude-code-cli`
/// because `WorkspaceInfo` carries no connector id, and `resume` is the first command that
/// reads the field — it is how it finds the manifest holding `lock_paths[]` and `[resume]`.
fn resumable(
    tmp: &tempfile::TempDir,
    id: &str,
) -> (goalcup_cli::daemon::MockDaemonClient, PathBuf, String) {
    let m = support::mock_resumable(tmp, id);
    let entry_id = m.info().entry_id;
    let cfg = support::cfg_with_entry(tmp.path(), &m.info());
    support::set_cached_connector(&cfg, &entry_id, id);
    (m, cfg, entry_id)
}

/// Wait until a concurrent command has crossed a named mock boundary.  The test fake records
/// before its optional barrier, so this is a deterministic "first CLI owns the section"
/// witness rather than a sleep that happens to work on one machine.
fn wait_for_call(m: &goalcup_cli::daemon::MockDaemonClient, name: &str) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if m.calls().iter().any(|call| call == name) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("the mock never received {name} within 3s: {:?}", m.calls());
}

/// One machine may have one non-preview resume critical section per entry.  The first call
/// has reached `/resume` and is intentionally paused before it receives an answer; the
/// second must fail locally rather than opening (or later cancelling) another permit.
#[test]
fn a_second_local_resume_is_refused_while_the_first_holds_the_entry_lock() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "claude-code-cli");
    let m = Arc::new(m);
    let connectors = support::connectors_dir_with_a_resume_that_exits("claude-code-cli");
    let release = tmp.path().join("release-first-resume");
    m.block_next_resume_until(&release);

    let first_m = Arc::clone(&m);
    let first_cfg = cfg.clone();
    let first_connectors = connectors.clone();
    let first_entry = entry_id.clone();
    let first = std::thread::spawn(move || {
        resume_with(
            &*first_m,
            &first_connectors,
            &first_cfg,
            &first_entry,
            &support::resume_options(),
            &mut Vec::new(),
        )
    });
    wait_for_call(&m, "resume");

    let second = resume_with(
        &*m,
        &connectors,
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    );

    // Always free the test barrier before asserting, including when `second` completed a
    // second relaunch and the assertion below fails.
    std::fs::write(&release, b"go").unwrap();
    let first = first.join().expect("the first resume thread did not panic");

    let err = second.expect_err("a live local resume owns this entry's critical section");
    assert_eq!(err.exit_code(), 4, "a local resume lock is a gate: {err}");
    assert!(
        format!("{err}").contains("another local `goalcup resume"),
        "{err}"
    );
    assert!(
        first.is_ok(),
        "the owning resume still completes: {first:?}"
    );
    assert_eq!(
        m.calls()
            .iter()
            .filter(|call| call.as_str() == "resume")
            .count(),
        1,
        "the refused successor never opened a second daemon permit: {:?}",
        m.calls()
    );
}

/// An open permit without this machine's durable pre-POST marker may belong to another CLI.
/// It is not a crash this invocation is entitled to repair, so it must be left alone.
#[test]
fn an_unmarked_open_resume_is_refused_without_cancelling_another_cli() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "claude-code-cli");
    m.set_resume_handoff(2, "open_resume");
    m.fail_always(
        "resume",
        ControlError::Conflict("the test must never open another permit".to_string()),
    );

    let err = resume_with(
        &m,
        &support::connectors_dir_with_a_resume_that_exits("claude-code-cli"),
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect_err("only a successor holding this machine's recovery marker may cancel open");

    assert_eq!(
        err.exit_code(),
        1,
        "an unknown open permit is an actionable refusal"
    );
    assert!(format!("{err}").contains("another local CLI"), "{err}");
    assert!(
        !m.calls().contains(&"cancel_process_handoff".to_string()),
        "the command must not cancel a permit it cannot identify as its crashed predecessor: {:?}",
        m.calls()
    );
    assert!(
        !m.calls().contains(&"resume".to_string()),
        "nothing else is posted after the unknown open permit: {:?}",
        m.calls()
    );
}

/// The one safe recovery case: the prior local CLI wrote its intent before posting generation
/// one, then died after the daemon opened generation two.  The successor gets the released OS
/// lock, matches exactly that next generation, cancels it, and removes the marker.
#[test]
fn a_marked_open_resume_is_cancelled_only_by_the_lock_successor() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "claude-code-cli");
    m.set_resume_handoff(2, "open_resume");
    let mut record = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    record.resume_recovery = Some(goalcup_cli::entries::ResumeRecoveryIntent {
        expected_generation: 1,
    });
    goalcup_cli::entries::write_entry(&cfg, &record).unwrap();
    // The fixture's default response permits the fresh post-cancellation launch.  A
    // successful run makes the test cover both exact recovery and consumption of its marker.
    resume_with(
        &m,
        &support::connectors_dir_with_a_resume_that_exits("claude-code-cli"),
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect("the exact marker authorizes recovery and a fresh launch");

    assert_eq!(
        m.calls()
            .iter()
            .filter(|call| call.as_str() == "cancel_process_handoff")
            .count(),
        1,
        "only the exact persisted predecessor is cancelled: {:?}",
        m.calls()
    );
    let repaired = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    assert_eq!(repaired.process_generation, 2);
    assert!(
        repaired.resume_recovery.is_none(),
        "a successful cancellation consumes the local recovery permission"
    );
}

/// A daemon can accept `/resume` before a bad response header/body makes the client return
/// `Malformed`.  That is no safer than a disconnected transport: the durable marker must
/// survive until the lock successor previews and cancels exactly the permit this call opened.
#[test]
fn a_malformed_resume_reply_keeps_marker_for_successor_recovery() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "claude-code-cli");
    m.accept_next_resume_then_malformed();
    let connectors = support::connectors_dir_with_a_resume_that_exits("claude-code-cli");

    let err = resume_with(
        &m,
        &connectors,
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect_err("the accepted resume's malformed reply reaches the caller as an error");
    assert_eq!(err.exit_code(), 3, "{err}");

    let after_error = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    assert_eq!(
        after_error
            .resume_recovery
            .as_ref()
            .map(|intent| intent.expected_generation),
        Some(2),
        "the pre-POST intent must outlive an ambiguous response"
    );
    let open = m.resume_preview_info();
    assert_eq!(open.process_generation, 3);
    assert_eq!(open.process_handoff_phase, "open_resume");

    resume_with(
        &m,
        &connectors,
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect("the successor owns the released local lock and repairs its predecessor");

    assert_eq!(
        m.cancelled_process_handoff_generations(),
        vec![3],
        "only the exact N+1 permit from the ambiguous request may be cancelled"
    );
    let repaired = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    assert!(repaired.resume_recovery.is_none());
    assert!(repaired.pending_process_registration.is_none());
}

/// The durable marker is not a blanket cancellation capability: it authorizes only the
/// generation immediately after the pre-POST CAS value.  A later open permit may belong to
/// another recovery attempt and must remain untouched.
#[test]
fn a_marker_for_another_generation_cannot_cancel_an_open_resume() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "claude-code-cli");
    m.set_resume_handoff(2, "open_resume");
    let mut record = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    record.resume_recovery = Some(goalcup_cli::entries::ResumeRecoveryIntent {
        expected_generation: 0,
    });
    goalcup_cli::entries::write_entry(&cfg, &record).unwrap();

    let err = resume_with(
        &m,
        &support::connectors_dir_with_a_resume_that_exits("claude-code-cli"),
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect_err("a marker for generation zero is not authority over open generation two");

    assert!(format!("{err}").contains("another local CLI"), "{err}");
    assert!(
        !m.calls().contains(&"cancel_process_handoff".to_string()),
        "a mismatched marker cannot be used as a general cancellation credential: {:?}",
        m.calls()
    );
    assert!(
        !m.calls().contains(&"resume".to_string()),
        "a mismatched marker stops before another CAS: {:?}",
        m.calls()
    );
}

/// A lock file where `codex-cli`'s `lock_paths[]` says one goes: named after the session, in
/// the harness home `goalcup start` recorded. `{config_dir}` is that directory and no other.
///
/// `contents` is the file's own bytes, and it is a parameter because it is the ONLY
/// place a holder can come from. The spike found
/// `$CODEX_HOME/thread-writer-locks/<thread>.lock` beside a directory-wide
/// `.coordination.lock` and never opened either, so the CLI reads one as a holder only when
/// the whole file is a decimal pid, and every test below says which case it is testing by
/// what it writes here.
fn write_a_lock(m: &goalcup_cli::daemon::MockDaemonClient, name: &str, contents: &str) -> PathBuf {
    let locks = Path::new(&m.resume_info().harness_home).join("thread-writer-locks");
    std::fs::create_dir_all(&locks).unwrap();
    let lock = locks.join(name);
    std::fs::write(&lock, contents.as_bytes()).unwrap();
    lock
}

#[cfg(unix)]
#[test]
fn a_fifo_lock_is_classified_without_blocking_for_a_writer() {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt as _;

    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "codex-cli");
    support::set_cached_session_id(&cfg, &entry_id, &m.resume_info().session_id);
    let locks = Path::new(&m.resume_info().harness_home).join("thread-writer-locks");
    std::fs::create_dir_all(&locks).unwrap();
    let lock = locks.join(format!("{}.lock", m.resume_info().session_id));
    let native = CString::new(lock.as_os_str().as_bytes()).unwrap();
    // SAFETY: the CString is NUL terminated and names an owned fixture path.
    assert_eq!(unsafe { libc::mkfifo(native.as_ptr(), 0o600) }, 0);

    let rec = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    let started = Instant::now();
    let findings = check_locks(
        &support::connector("codex-cli").manifest,
        &LockRoots::from_entry(&rec),
    )
    .unwrap();
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].path, lock.display().to_string());
    assert_eq!(findings[0].holder_pid, None);
}

#[cfg(unix)]
#[test]
fn a_lock_symlink_never_supplies_a_holder_pid() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "codex-cli");
    support::set_cached_session_id(&cfg, &entry_id, &m.resume_info().session_id);
    let locks = Path::new(&m.resume_info().harness_home).join("thread-writer-locks");
    std::fs::create_dir_all(&locks).unwrap();
    let outside = tmp.path().join("outside-pid");
    std::fs::write(&outside, std::process::id().to_string()).unwrap();
    let lock = locks.join(format!("{}.lock", m.resume_info().session_id));
    symlink(&outside, &lock).unwrap();

    let rec = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    let findings = check_locks(
        &support::connector("codex-cli").manifest,
        &LockRoots::from_entry(&rec),
    )
    .unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].holder_pid, None);
    assert_eq!(
        std::fs::read_to_string(outside).unwrap(),
        std::process::id().to_string()
    );
}

/// The per-thread lock, named after the BOUND thread the way the spike found it — and the
/// record moved to the same id, because that is the run this file belongs to.
///
/// `codex-cli`'s `lock_paths[]` names `{config_dir}/thread-writer-locks/
/// {session_id}.lock`, and the id it renders on this side comes from the entry record:
/// the daemon's MINTED id until a resume writes the BOUND one back, and the bound one
/// afterwards. Every test that puts a held lock in front of the check is about a harness
/// that is holding this run's thread, so the record says so.
/// `the_first_resume_reads_the_minted_id_and_the_daemon_refuses_instead` is the other state,
/// and it sets nothing.
fn write_the_thread_writer_lock(
    m: &goalcup_cli::daemon::MockDaemonClient,
    cfg: &Path,
    entry_id: &str,
    contents: &str,
) -> PathBuf {
    support::set_cached_session_id(cfg, entry_id, &m.resume_info().session_id);
    write_a_lock(m, &format!("{}.lock", m.resume_info().session_id), contents)
}

/// The holder is read out of the lock file and never off the entry cache.
///
/// The cache is deliberately made to say something else — a pid that is certainly dead —
/// A finding that took the cache's pid would produce "no live holder" on a held lock, and the
/// mirror-image arrangement (a recycled pid in the cache, nothing in the file) would tell the
/// participant to kill a process that had nothing to do with their run.
#[test]
fn a_live_lock_holder_is_refused_before_the_resume_is_ever_posted() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "codex-cli");
    write_the_thread_writer_lock(&m, &cfg, &entry_id, &format!("{}\n", std::process::id()));
    support::set_cached_last_pid(&cfg, &entry_id, support::a_pid_that_is_certainly_dead());

    let rec = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    let findings = check_locks(
        &support::connector("codex-cli").manifest,
        &LockRoots::from_entry(&rec),
    )
    .expect("the shipped codex-cli lock path names only tokens a resume can resolve");
    assert_eq!(findings.len(), 1);
    assert!(findings[0].holder_alive);
    assert_eq!(findings[0].holder_pid, Some(std::process::id()));
    assert!(
        findings[0].path.starts_with(&rec.harness_home),
        "{{config_dir}} is the harness home the record carries, not an env var"
    );

    let err = resume_with(
        &m,
        &support::connectors_dir(),
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert_eq!(err.exit_code(), 4, "a gate refusal, not a verdict");
    let msg = format!("{err}");
    assert!(msg.contains("silently becomes a new thread"), "{msg}");
    assert!(
        msg.contains(&std::process::id().to_string()),
        "say which pid is holding it: {msg}"
    );
    assert!(
        msg.contains(&findings[0].path),
        "and which file it is holding: {msg}"
    );
    // **No `kill` command.** The pid came out of a file under
    // `harness_home`, a directory the measured harness owns and can write with a shell tool,
    // and `is_alive` only asks whether SOMETHING answers to that number — so a pasteable
    // `kill N` is an instruction the agent under measurement can aim. What is printed
    // instead is the pid, what the process table calls it, and the ask to recognise it.
    assert!(
        !msg.contains("kill "),
        "no kill command is ever printed: {msg}"
    );
    assert!(
        msg.contains("if that is your harness, stop it"),
        "the participant is asked to recognise the process, not to run a command: {msg}"
    );
    let named = findings[0]
        .holder_command
        .clone()
        .expect("this process is in this machine's own process table");
    assert!(
        msg.contains(&named),
        "and the command line `ps` reports for that pid is beside it: {msg}"
    );
    assert!(
        !msg.contains("rm "),
        "and a `rm` is never offered while something is still writing: {msg}"
    );
    assert!(
        !msg.contains(&rec.last_pid.unwrap().to_string()),
        "the cache's own pid is not evidence about a file and is never named: {msg}"
    );
    // The daemon answers a bare `Conflict` with no findings on it. The check runs first so
    // the participant is told which pid holds the lock.
    assert!(
        !m.calls().contains(&"resume".to_string()),
        "nothing was posted"
    );
}

#[test]
fn an_oversized_pid_file_is_a_lock_but_never_names_a_holder() {
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "codex-cli");
    let lock = write_the_thread_writer_lock(&m, &cfg, &entry_id, "");
    std::fs::OpenOptions::new()
        .write(true)
        .open(&lock)
        .unwrap()
        .set_len(33)
        .unwrap();

    let rec = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    let findings = check_locks(
        &support::connector("codex-cli").manifest,
        &LockRoots::from_entry(&rec),
    )
    .unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].holder_pid, None);
    assert!(!findings[0].holder_alive);
}

/// This run's own lock file is the finding, and `.coordination.lock` is not.
///
/// The spike found two shapes under `$CODEX_HOME/thread-writer-locks/`: the per-thread
/// `<thread>.lock`, which Codex holds while the thread is open and removes on a clean exit,
/// and a directory-wide `.coordination.lock` — 0 bytes, no holder, `lsof` empty — which it
/// makes at startup and leaves behind. `lock_paths[]` names the per-thread file, so a run
/// that ended properly does not refuse its own resume over the residue, and the holder comes
/// out of that file rather than off the entry cache, which would stamp every file in the
/// directory with one pid.
#[test]
fn only_this_runs_own_lock_file_is_a_finding_and_the_coordination_file_is_not() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "codex-cli");
    let dead = support::a_pid_that_is_certainly_dead();
    let named = write_the_thread_writer_lock(&m, &cfg, &entry_id, &format!("{dead}\n"));
    // What a clean `/quit` leaves behind, and what a resume must walk straight past.
    let coordination = write_a_lock(&m, ".coordination.lock", "");
    // A live pid in the CACHE, which must reach no finding.
    support::set_cached_last_pid(&cfg, &entry_id, std::process::id());

    let rec = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    let manifest = support::connector("codex-cli").manifest;
    let findings = check_locks(&manifest, &LockRoots::from_entry(&rec)).unwrap();
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].path, named.display().to_string());
    assert_eq!(findings[0].holder_pid, Some(dead));
    assert!(!findings[0].holder_alive);

    // And with this run's own lock gone — a clean quit — the coordination file that is still
    // there holds nothing.
    std::fs::remove_file(&named).unwrap();
    assert!(
        coordination.exists(),
        "the harness left it, and nothing removes it"
    );
    assert!(
        check_locks(&manifest, &LockRoots::from_entry(&rec))
            .unwrap()
            .is_empty(),
        "a clean quit leaves `.coordination.lock` and no lock"
    );
}

/// The other state the record can be in: a FIRST resume, before the bound id is written back.
///
/// The honest edge. `goalcup start` writes the id the daemon MINTED, and only the
/// end of a resume replaces it with the BOUND one, so on the first resume of a Codex run this
/// side renders `<minted>.lock` — a name no harness makes — and finds nothing to name. The
/// daemon renders the bound id and refuses the POST on its own reading, which is why the
/// module header calls it the authority. What is lost in this one case is the pid in the
/// message; the relaunch is still stopped.
///
/// **This test asserts THIS side only** — that the CLI reports nothing and leaves the lock
/// where it is. The daemon's half, the 409 it answers over the bound id, is
/// `crates/goalcup-daemon/tests/control_api.rs`; the mock here is never asked for it.
#[test]
fn the_first_resume_reads_the_minted_id_and_the_daemon_refuses_instead() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "codex-cli");
    // The lock the harness is really holding, named after the BOUND thread — and the record
    // left exactly as `goalcup start` wrote it.
    let held = write_a_lock(
        &m,
        &format!("{}.lock", m.resume_info().session_id),
        &format!("{}\n", std::process::id()),
    );
    let rec = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    assert_ne!(
        rec.session_id,
        m.resume_info().session_id,
        "the mock models a harness that mints its own thread; without that this test is empty"
    );
    assert!(
        check_locks(
            &support::connector("codex-cli").manifest,
            &LockRoots::from_entry(&rec)
        )
        .unwrap()
        .is_empty(),
        "the minted id names no file, so this side has nothing to report"
    );
    assert!(
        held.exists(),
        "and the lock the daemon will refuse over stands"
    );
}

/// An EMPTY lock DIRECTORY is not a lock, and a resume over it goes ahead.
///
/// A harness that creates its lock directory at startup and never removes it would have every
/// resume refused by a rule that refused on the PATH EXISTING. The daemon and this side share
/// one rule (`manifest::lock_path_is_held`), and this asserts both shapes of it on a
/// directory-shaped `lock_paths[]`; no shipped connector names a directory (`codex-cli` names
/// the per-thread FILE, and the two tests above are its shapes).
#[test]
fn an_empty_lock_directory_holds_nothing_and_a_file_in_it_holds_everything() {
    // One connector, and it is named rather than looped: `codex-cli` names the per-thread FILE
    // and `claude-code-cli` declares no lock at all, so the directory shape is kept on a copy
    // of Codex's manifest whose lock is a directory.
    {
        let id = "codex-cli";
        let tmp = tempfile::tempdir().unwrap();
        let (m, cfg, entry_id) = resumable(&tmp, id);
        let mut manifest = support::connector(id).manifest;
        manifest.lock_paths = vec!["{config_dir}/state/locks/".to_string()];
        let rec = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
        let roots = LockRoots::from_entry(&rec);

        // Nothing on disk at all.
        assert!(
            check_locks(&manifest, &roots).unwrap().is_empty(),
            "{id}: no lock directory, no locks"
        );

        // The directory the harness makes at startup, empty.
        let template = manifest
            .lock_paths
            .first()
            .expect("this connector names one");
        let dir = PathBuf::from(
            template
                .replace("{config_dir}", &m.resume_info().harness_home)
                .replace("{home}", &m.resume_info().harness_home),
        );
        std::fs::create_dir_all(&dir).unwrap();
        assert!(
            check_locks(&manifest, &roots).unwrap().is_empty(),
            "{id}: an empty lock directory is a directory the harness made, not a lock"
        );

        // A directory INSIDE it is not a lock file either.
        std::fs::create_dir_all(dir.join("a-directory")).unwrap();
        assert!(
            check_locks(&manifest, &roots).unwrap().is_empty(),
            "{id}: a nested directory is not a lock file"
        );

        // One file in it, and the resume is refused with that file named.
        let lock = dir.join("held.lock");
        std::fs::write(&lock, "").unwrap();
        let findings = check_locks(&manifest, &roots).unwrap();
        assert_eq!(findings.len(), 1, "{id}: {findings:?}");
        assert_eq!(findings[0].path, lock.display().to_string());

        // And the protocol helper — the ONE rule both sides apply — says the same thing
        // about the same directory.
        let rendered = format!("{}/", dir.display());
        assert!(goalcup_protocol::manifest::lock_path_is_held(&rendered));
        std::fs::remove_file(&lock).unwrap();
        assert!(!goalcup_protocol::manifest::lock_path_is_held(&rendered));
    }
}

/// The daemon refuses first and its refusals are not all the same refusal.
/// `busy` is transient, `conflict` on a lock is a gate, `conflict` on a sealed entry is over.
///
/// The rungs: `busy` is 1 (the tracker's own problem, and the same number `goalcup seal`
/// gives it), a lock is 4 (a gate, and the move is out in the world), a seal is 1 (over, and
/// there is nothing to do).
#[test]
fn the_daemons_own_refusals_land_on_the_right_rungs_of_the_ladder() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let connectors = support::connectors_dir_with_a_resume_that_exits("codex-cli");

    // Busy: the bounded retry carries it, and the relaunch happens.
    let (m, cfg, entry_id) = resumable(&tmp, "codex-cli");
    m.busy_once("resume");
    resume_with(
        &m,
        &connectors,
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect("one retry is enough for a pump that has finished");
    let requests = m.resume_requests();
    assert_eq!(
        requests.len(),
        2,
        "the Busy CAS was asked exactly once again"
    );
    assert_eq!(
        requests[0], requests[1],
        "the retry never mints a second resume generation"
    );
    assert_eq!(
        m.process_registrations().len(),
        1,
        "one resumed child is handed off after the CAS succeeds"
    );

    // Still busy when the retry is spent: the entry is held, not broken, and the
    // participant's move is to wait and run the command again.
    //
    // Rung 1, and rung 1 on `goalcup seal` too: one fact on two rungs is a number a script
    // branching on it cannot read. A daemon holding an entry has decided nothing about the
    // harness, the workspace
    // or the locks, so it is not a gate's verdict; it is the tracker's own problem, and
    // running the command again is what clears it.
    //
    // `fail_always` and not two `fail_next`s: the mock holds ONE armed refusal per method,
    // so arming `busy` twice arms it once and `retry_busy`'s third attempt would succeed.
    let (m, cfg, entry_id) = resumable(&tmp, "codex-cli");
    m.fail_always("resume", ControlError::Busy("mid-pump".into()));
    let err = resume_with(
        &m,
        &connectors,
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert_eq!(
        err.exit_code(),
        1,
        "still busy: wait and run it again — the same rung `goalcup seal` puts it on"
    );
    assert!(
        format!("{err}").contains("after up to 9 attempts"),
        "an interrupted or budget-limited retry must not claim all nine requests ran: {err}"
    );
    assert!(
        !m.calls().contains(&"register_process".to_string()),
        "and nothing was launched"
    );

    // Conflict naming a lock the CLI's own check did not see: still a gate, exit 4. The
    // daemon's own sentence, as `free_of_held_locks` writes it — path after the colon.
    let (m, cfg, entry_id) = resumable(&tmp, "codex-cli");
    m.fail_next(
        "resume",
        ControlError::Conflict(
            "entry e cannot be resumed while the harness's lock is still on disk: \
             /x/thread-writer-locks/s.lock"
                .into(),
        ),
    );
    let err = resume_with(
        &m,
        &connectors,
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert_eq!(err.exit_code(), 4);
    assert!(format!("{err}").contains("thread-writer-locks"));
    assert!(
        !format!("{err}").contains("Stop whatever is holding"),
        "this side's check ran first and found nothing, so it has no advice to append: {err}"
    );

    // The same refusal, with a path that has the word in it. The split reads the daemon's
    // sentence — everything before the first colon — and never a substring of a path, or
    // this lands on rung 1 where there is nothing the participant can do.
    let (m, cfg, entry_id) = resumable(&tmp, "codex-cli");
    m.fail_next(
        "resume",
        ControlError::Conflict(
            "entry e cannot be resumed while the harness's lock is still on disk: \
             /Users/x/sealed-runs/thread-writer-locks/s.lock"
                .into(),
        ),
    );
    let err = resume_with(
        &m,
        &connectors,
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert_eq!(err.exit_code(), 4, "a path is not a seal: {err}");

    // A sealed entry is a different refusal, and exit 1: nothing will unseal it. The
    // daemon's own words again — `resume_entry` answers "entry {id} is sealed: …".
    let (m, cfg, entry_id) = resumable(&tmp, "codex-cli");
    m.fail_next(
        "resume",
        ControlError::Conflict(
            "entry e is sealed: `EventBody::Seal` closed its chain at 2026-09-05T00:00:00Z".into(),
        ),
    );
    let err = resume_with(
        &m,
        &connectors,
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert_eq!(err.exit_code(), 1);
    assert!(format!("{err}").to_lowercase().contains("sealed"));
}

#[test]
fn a_busy_resume_preview_retries_without_duplicate_generation_or_child() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "codex-cli");
    m.busy_once("resume_preview");

    resume_with(
        &m,
        &support::connectors_dir_with_a_resume_that_exits("codex-cli"),
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect("the preview retries after the upload releases the entry");

    assert_eq!(
        m.calls()
            .iter()
            .filter(|call| *call == "resume_preview")
            .count(),
        2
    );
    assert_eq!(m.resume_requests().len(), 1, "one generation was minted");
    assert_eq!(
        m.process_registrations().len(),
        1,
        "one resumed child was handed off"
    );
}

#[test]
fn unreachable_preview_and_resume_cas_are_each_one_shot_before_spawn() {
    let _serial = support::one_interrupt_at_a_time();
    for method in ["resume_preview", "resume"] {
        let tmp = tempfile::tempdir().unwrap();
        let (m, cfg, entry_id) = resumable(&tmp, "codex-cli");
        m.fail_always(method, ControlError::Unreachable("daemon absent".into()));

        let error = resume_with(
            &m,
            &support::connectors_dir_with_a_resume_that_exits("codex-cli"),
            &cfg,
            &entry_id,
            &support::resume_options(),
            &mut Vec::new(),
        )
        .expect_err("an ambiguous entry call is never replayed");

        assert_eq!(error.exit_code(), 1, "{method}");
        assert_eq!(
            m.calls()
                .iter()
                .filter(|call| call.as_str() == method)
                .count(),
            1,
            "{method} Unreachable was retried"
        );
        assert!(
            m.process_registrations().is_empty(),
            "{method} spawned a child"
        );
        assert_eq!(
            m.resume_requests().len(),
            usize::from(method == "resume"),
            "only the CAS path offered one generation"
        );
    }
}

/// A lock nobody holds is still a refusal, and the participant is handed the `rm`.
///
/// The relaunch does NOT go ahead: the daemon's `free_of_held_locks` refuses on the path
/// existing and never asks whether a holder is alive, so a warning that promised a relaunch
/// would be taken away by the very next call. What the CLI is for here is the half the daemon
/// cannot do — say which file, say that nothing holds it, and print the one command that
/// clears it — and then stop.
#[test]
fn a_stale_lock_is_refused_before_the_post_and_the_rm_is_printed() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "codex-cli");
    let dead = support::a_pid_that_is_certainly_dead();
    let lock = write_the_thread_writer_lock(&m, &cfg, &entry_id, &format!("{dead}\n"));

    let rec = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    let findings = check_locks(
        &support::connector("codex-cli").manifest,
        &LockRoots::from_entry(&rec),
    )
    .expect("the shipped codex-cli lock path resolves");
    assert!(
        !findings[0].holder_alive,
        "the normal case after a hard kill"
    );

    let mut out = Vec::new();
    let err = resume_with(
        &m,
        &support::connectors_dir_with_a_resume_that_exits("codex-cli"),
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut out,
    )
    .unwrap_err();
    assert_eq!(
        err.exit_code(),
        4,
        "the same rung as a live holder: the daemon refuses on either: {err}"
    );
    let msg = format!("{err}");
    assert!(msg.contains(&lock.display().to_string()), "{msg}");
    assert!(
        msg.contains(&format!("rm {}", lock.display())),
        "the exact command that clears it: {msg}"
    );
    assert!(
        !msg.contains("goes ahead"),
        "and never a promise the daemon is about to take back: {msg}"
    );
    assert!(lock.exists(), "which this command prints and never runs");
    assert!(
        !m.calls().contains(&"resume".to_string()),
        "nothing was posted"
    );
    assert!(
        String::from_utf8(out).unwrap().is_empty(),
        "and the relaunch never began"
    );
}

/// "Recording. Leave this running." is not the last word. The fixture harness this test
/// relaunches EXITS AT ONCE, so the line would promise a run that was already over: the exit
/// is named, and beside it what the tracker holds.
#[test]
fn a_relaunch_that_dies_at_once_reports_its_exit_and_what_the_tracker_holds() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "codex-cli");
    let mut out = Vec::new();
    resume_with(
        &m,
        &support::connectors_dir_with_a_resume_that_exits("codex-cli"),
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut out,
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("Recording. Leave this running."),
        "still said, because it is true of the minute a TUI is up: {text}"
    );
    assert!(
        text.contains("the harness exited:"),
        "and the exit is the line after it: {text}"
    );
    assert!(
        text.contains("were caught up before this relaunch"),
        "with what the tracker has, so the participant is not left guessing: {text}"
    );
    assert!(
        text.contains(&format!("goalcup seal {entry_id}")),
        "and the two moves that are left: {text}"
    );
}

#[test]
fn a_resume_with_no_lock_on_disk_relaunches_with_the_same_pins() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "codex-cli");

    // Not `--print-launch`: that flag stops before the spawn and the registration this
    // test is about — the same contract `goalcup start` already
    // holds. The session id in the relaunched argv is asserted below, off the handle the
    // daemon actually received, which is the stronger check anyway.
    let o = support::resume_options();
    let mut out = Vec::new();
    resume_with(
        &m,
        &support::connectors_dir_with_a_resume_that_exits("codex-cli"),
        &cfg,
        &entry_id,
        &o,
        &mut out,
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("resume"),
        "the closing line names the command that continues this entry: {text}"
    );
    assert!(
        text.contains("keep going"),
        "D8, to the terminal and never into the session: {text}"
    );
    // The daemon clears its handle when it answers `resume`. A relaunch that
    // does not hand back a new one leaves the daemon measuring, and terminating, the
    // pre-crash pid.
    assert!(
        m.calls().contains(&"register_process".to_string()),
        "the relaunched process is registered, or the seal kills nothing"
    );
    assert_eq!(
        goalcup_cli::entries::read_entry(&cfg, &entry_id)
            .unwrap()
            .last_pid,
        m.last_registered_pid(),
        "the cache follows the handle"
    );
    // And what was handed over describes the RESUME recipe, not the start one: the daemon's
    // `same_process` compares a live command line against `argv`, so a handle carrying the
    // start argv would describe a process that is not running.
    let handed = m.last_registered_handle().expect("a handle was registered");
    let expected_generation = m.resume_preview_info().process_generation;
    assert_eq!(
        m.resume_requests(),
        vec![ResumeRequest {
            expected_generation,
        }],
        "the sole mutating resume names the generation from the authoritative preview"
    );
    assert_eq!(
        m.process_registrations(),
        vec![ProcessRegistration {
            generation: m.resume_info().process_generation,
            handle: handed.clone(),
        }],
        "the post-CAS process handoff is the complete typed request the daemon received"
    );
    assert_eq!(
        handed.argv.first().map(String::as_str),
        Some("resume"),
        "codex-cli's `[resume]` recipe is positional and `resume` is its first word: {:?}",
        handed.argv
    );
    assert!(handed.argv.contains(&m.resume_info().session_id));
    assert!(
        !handed.argv.iter().any(|a| a == "--session-id"),
        "and never the `[launch]` recipe's creating flag"
    );
    // D22 and `launch`'s three modes: a TUI is never put in a process group of its own,
    // because a group that is not the terminal's foreground group loses the tty on its first
    // read. Every shipped connector is `Tui`.
    assert_eq!(
        handed.pgid, None,
        "a TUI relaunch carries no pgid, exactly as its launch does"
    );
}

/// A cached process registration is the one recovery body the CLI may replay.  It must be
/// offered before the connector is even loaded: that accepted child can legitimately own the
/// connector's lock, and a fresh CAS would create a second process authority beside it.
#[test]
fn a_pending_registration_replays_before_the_lock_check_without_a_fresh_resume_cas() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "codex-cli");
    let registration = ProcessRegistration {
        generation: 41,
        handle: ProcessHandle {
            pid: 424_242,
            pgid: None,
            native_pid: None,
            native_binary: None,
            native_process_start: None,
            process_start: None,
            strategy: KillStrategy::NativePid,
            grace_seconds: 5,
            launched_at: "2026-09-11T00:00:00Z".to_string(),
            program: "/usr/bin/true".to_string(),
            argv: vec!["true".to_string()],
            env_keys: Vec::new(),
            home_seed: None,
        },
    };
    let mut cached = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    cached.process_generation = registration.generation;
    cached.pending_process_registration = Some(registration.clone());
    goalcup_cli::entries::write_entry(&cfg, &cached).unwrap();

    // A live holder is a normal-resume refusal.  It is deliberately present here to prove
    // that the exact replay returns before any lock inspection or fresh resume CAS.
    let lock =
        write_the_thread_writer_lock(&m, &cfg, &entry_id, &format!("{}\n", std::process::id()));
    assert!(lock.exists(), "the connector lock is genuinely on disk");

    let mut out = Vec::new();
    resume_with(
        &m,
        &support::connectors_dir(),
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut out,
    )
    .expect("the durable process body is replayed before the lock can refuse it");

    assert_eq!(m.calls(), vec!["register_process"]);
    assert_eq!(m.process_registrations(), vec![registration.clone()]);
    assert!(
        m.resume_requests().is_empty(),
        "recovery did not spend a fresh resume generation"
    );
    assert!(
        m.process_handoff_cancellations().is_empty(),
        "an acknowledged replay is already owned by the daemon"
    );
    let repaired = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    assert_eq!(repaired.process_generation, registration.generation);
    assert_eq!(repaired.last_pid, Some(registration.handle.pid));
    assert_eq!(repaired.pending_process_registration, None);
    assert!(
        lock.exists(),
        "replay never treated a live connector lock as disposable"
    );
    assert!(
        String::from_utf8(out)
            .unwrap()
            .contains("recovered the previously issued process handoff"),
        "the command tells the participant that the existing child, not a new resume, won"
    );
}

#[test]
fn the_tail_is_caught_up_before_any_relaunch() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "claude-code-cli");
    let mut out = Vec::new();
    resume_with(
        &m,
        &support::connectors_dir_with_a_resume_that_exits("claude-code-cli"),
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut out,
    )
    .unwrap();
    let calls = m.calls();
    assert_eq!(
        calls.iter().position(|c| c == "resume_preview"),
        Some(0),
        "the authoritative generation is read before the CAS"
    );
    assert_eq!(
        calls.iter().position(|c| c == "resume"),
        Some(1),
        "R16 and D9: after the read-only authority check, the gap is closed before any relaunch"
    );
    assert!(
        calls.iter().position(|c| c == "resume")
            < calls.iter().position(|c| c == "register_process")
    );
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("caught up"), "{text}");
    assert!(
        text.contains(&m.resume_info().caught_up_events.to_string()),
        "and how many: {text}"
    );
}

#[test]
fn each_connector_relaunches_with_its_own_resume_recipe_and_never_the_start_one() {
    let _serial = support::one_interrupt_at_a_time();
    for (id, must, must_not) in [
        ("claude-code-cli", "--resume", "--session-id"),
        ("codex-cli", "resume", "--session-id"),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let (m, cfg, entry_id) = resumable(&tmp, id);
        let mut o = support::resume_options();
        o.print_launch = true;
        let mut out = Vec::new();
        resume_with(
            &m,
            &support::connectors_dir_with_a_resume_that_exits(id),
            &cfg,
            &entry_id,
            &o,
            &mut out,
        )
        .unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains(must), "{id} must resume with {must}: {text}");
        assert!(!text.contains(must_not), "{id} must not carry {must_not}");
    }
    // A connector with no lock_paths has nothing to check and refuses nothing.
    assert!(
        support::connector("claude-code-cli")
            .manifest
            .lock_paths
            .is_empty()
    );

    // Only four of the nine PATH_TOKENS may appear in a lock path — the three roots plus
    // `session_id`, which `LockRoots` carries and which `codex-cli`'s per-thread
    // lock needs — and every shipped manifest keeps to them. A fifth would be an
    // Internal error naming the token, never a silent glob that reports every run as free.
    for id in ["claude-code-cli", "codex-cli"] {
        for p in &support::connector(id).manifest.lock_paths {
            for tok in support::brace_tokens(p) {
                assert!(
                    ["config_dir", "home", "workspace", "session_id"].contains(&tok.as_str()),
                    "{id}: {{{tok}}} has no source in ResumeInfo"
                );
                assert!(goalcup_protocol::manifest::PATH_TOKENS.contains(&tok.as_str()));
            }
        }
    }
}

/// Both readings of the two DIRECTORIES — the record `goalcup start` wrote and the answer
/// the daemon just gave — say the same thing, or the relaunch would go into a directory the
/// lock check never looked at. The session id is the daemon's alone; see the body.
#[test]
fn the_entry_cache_and_the_daemons_answer_agree_about_the_two_directories() {
    // The second half of this test relaunches, which is the window the file's header is
    // about.
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "codex-cli");
    let rec = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    let from_entry = LockRoots::from_entry(&rec);
    let from_resume = LockRoots::from_resume(&m.resume_info());
    assert_eq!(from_entry.harness_home, from_resume.harness_home);
    assert_eq!(from_entry.workspace, from_resume.workspace);
    // **The session id is NOT one of them.** The cache holds the id
    // the daemon MINTED at creation; `ResumeInfo.session_id` is the id the arming gate
    // BOUND. Only `claude-code-cli` pins the minted id at launch, so on a Codex run the
    // harness mints its own and the two differ — which is why `[resume]` renders
    // `{session_id}` off `ResumeInfo`. Comparing them would refuse every Codex resume at rung
    // 1 with nothing relaunched.
    assert_ne!(
        from_entry.session_id, from_resume.session_id,
        "the mock models the daemon: the bound id is not the minted one"
    );
    // Not `--print-launch`: the cache-follows-the-daemon assertion below needs the
    // registration step 5 writes, and that flag stops before it.
    let o = support::resume_options();
    resume_with(
        &m,
        &support::connectors_dir_with_a_resume_that_exits("codex-cli"),
        &cfg,
        &entry_id,
        &o,
        &mut Vec::new(),
    )
    .expect("a bound session id that differs from the minted one is not a disagreement");
    assert_eq!(
        goalcup_cli::entries::read_entry(&cfg, &entry_id)
            .unwrap()
            .session_id,
        m.resume_info().session_id,
        "and the cache follows the daemon: the next resume selects the session that is \
         actually being written to"
    );
}

/// Two spellings of ONE directory are not a disagreement; two directories still are.
///
/// The daemon answers the CANONICAL workspace
/// everywhere, so both readings agree as plain strings on every run started by this build.
/// A cache written before that — or by a daemon that has not been upgraded — can hold the
/// spelling the participant asked for, and under a symlinked parent (`/var` is a link to
/// `/private/var` on macOS, which is where a temporary directory lands) that is the same
/// directory under two names. Refusing it would strand a resume over a name.
///
/// The second half is the half that has to keep working: a workspace that is genuinely a
/// different directory is still refused, and the refusal still says which of the three roots
/// the two readings disagree about.
#[cfg(unix)]
#[test]
fn a_workspace_reached_through_a_symlink_is_the_same_workspace() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "codex-cli");
    let connectors = support::connectors_dir_with_a_resume_that_exits("codex-cli");
    let mut o = support::resume_options();
    o.print_launch = true;

    // The daemon answers the real directory; the record spells it through a symlink.
    let real = PathBuf::from(m.resume_info().workspace);
    let link = tmp.path().join("linked-workspace");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let mut rec = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    rec.workspace = link.display().to_string();
    goalcup_cli::entries::write_entry(&cfg, &rec).unwrap();
    resume_with(&m, &connectors, &cfg, &entry_id, &o, &mut Vec::new())
        .expect("one directory under two names is one workspace");

    // A different directory, which exists — so this is the comparison failing and not a
    // `canonicalize` that could not resolve either side.
    let elsewhere = tmp.path().join("somewhere-else");
    std::fs::create_dir_all(&elsewhere).unwrap();
    rec.workspace = elsewhere.display().to_string();
    goalcup_cli::entries::write_entry(&cfg, &rec).unwrap();
    let err = resume_with(&m, &connectors, &cfg, &entry_id, &o, &mut Vec::new())
        .expect_err("a different directory is a different workspace");
    let msg = format!("{err}");
    assert!(
        msg.contains("different workspace"),
        "the refusal names the root the two readings disagree about: {msg}"
    );
}

/// A concurrent daemon operation can make the cancellation route Busy. The cancellation is
/// idempotent at one exact generation, so the CLI retries that body rather than returning a
/// fresh resume permit or trying to launch a second child.
///
/// Triggered here through the directory-disagreement refusal above rather than a held
/// `{server_port}`, which would need a port fixture.
///
/// The disagreement has to land on the CAS answer specifically, and not on the read-only
/// preview moments before it: a preview-only disagreement is refused before any permit is
/// issued, with nothing to cancel. [`goalcup_cli::daemon::MockDaemonClient::
/// disagree_workspace_on_next_resume`] says a different workspace on that one answer for
/// exactly that reason.
#[test]
fn a_busy_post_resume_cancellation_retries_the_same_generation_and_repairs_the_cache() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "codex-cli");
    let connectors = support::connectors_dir_with_a_resume_that_exits("codex-cli");

    // A different directory, which exists — so it is the workspace comparison that refuses
    // the resume, not a `canonicalize` that could not resolve either side. This machine's
    // own record is left untouched; the preview the CLI reads first must still agree with
    // it, or the refusal lands before the CAS ever opens a permit.
    let elsewhere = tmp.path().join("somewhere-else");
    std::fs::create_dir_all(&elsewhere).unwrap();
    m.disagree_workspace_on_next_resume(&elsewhere.display().to_string());

    m.busy_once("cancel_process_handoff");

    let error = resume_with(
        &m,
        &connectors,
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect_err("a different workspace is still refused under Busy cancellation contention");
    assert_eq!(error.exit_code(), 1);
    assert!(format!("{error}").contains("different workspace"));

    let expected_generation = m.resume_info().process_generation;
    let cancellation = ProcessHandoffCancelRequest {
        expected_generation,
        request_digest: None,
    };
    assert_eq!(
        m.process_handoff_cancellations(),
        vec![cancellation.clone(), cancellation.clone()],
        "Busy contention replays the exact cleanup request, never a newer generation"
    );
    assert_eq!(
        m.resume_requests(),
        vec![ResumeRequest {
            expected_generation: m.resume_preview_info().process_generation,
        }],
        "cleanup contention did not make a second resume CAS"
    );
    assert!(m.process_registrations().is_empty());
    let repaired = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    assert_eq!(repaired.process_generation, expected_generation);
    assert!(repaired.pending_process_registration.is_none());
}

/// A lock path naming a token a resume cannot resolve is the CLI's own bug — exit 3, naming
/// the token and the connector — and never a glob that quietly matches nothing.
#[test]
fn a_lock_path_naming_a_token_no_resume_can_resolve_is_internal_and_names_it() {
    let tmp = tempfile::tempdir().unwrap();
    // The mock authors the record and is asked nothing else: `check_locks` is called
    // directly here and never reaches the wire.
    let (_, cfg, entry_id) = resumable(&tmp, "codex-cli");
    let rec = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    let mut c = support::connector("codex-cli");
    // `artifact_root` is in `PATH_TOKENS`, so `validate()` accepts it and only this check
    // catches it. `ResumeInfo` carries one; `LockRoots` deliberately does not.
    c.manifest.lock_paths = vec!["{artifact_root}/locks/".to_string()];
    let err = check_locks(&c.manifest, &LockRoots::from_entry(&rec)).unwrap_err();
    assert_eq!(err.exit_code(), 3);
    let msg = format!("{err}");
    assert!(msg.contains("artifact_root"), "{msg}");
    assert!(msg.contains("codex-cli"), "{msg}");
}

/// A lock DIRECTORY that is there and cannot be read is not "no locks".
///
/// A `read_dir` error that was a `continue` would be the same answer as a directory that does
/// not exist — so a machine where the harness home had become unreadable would clear every
/// relaunch this side, and the participant would meet the daemon's own refusal with no
/// explanation from the half that is supposed to give one.
#[cfg(unix)]
#[test]
fn a_lock_directory_that_cannot_be_read_is_internal_and_never_no_locks() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "codex-cli");
    // A lock DIRECTORY, planted on a copy of Codex's manifest.
    let mut manifest = support::connector("codex-cli").manifest;
    manifest.lock_paths = vec!["{config_dir}/state/locks/".to_string()];
    let locks = Path::new(&m.resume_info().harness_home).join("state/locks");
    std::fs::create_dir_all(&locks).unwrap();
    std::fs::set_permissions(&locks, std::fs::Permissions::from_mode(0o000)).unwrap();

    let rec = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    let err = check_locks(&manifest, &LockRoots::from_entry(&rec)).unwrap_err();
    // Put it back before any assertion, so a failure here does not leave a directory the
    // test harness cannot clean up.
    std::fs::set_permissions(&locks, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(err.exit_code(), 3, "{err}");
    let msg = format!("{err}");
    assert!(msg.contains(&locks.display().to_string()), "{msg}");
    let _ = entry_id;
}

/// The same question in the FILE shape. `codex-cli` names one lock file, and a `stat` of it
/// inside a directory this process cannot enter fails with a permission error rather than
/// `NotFound`. Answering "free" there would be the silent pass M0 §2.5 measured the cost of:
/// `lock_path_is_held` reads anything but `NotFound` as HELD, the same way its directory
/// branch does, and the relaunch is refused rather than cleared.
#[cfg(unix)]
#[test]
fn a_lock_file_this_process_cannot_stat_is_held_and_never_no_locks() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "codex-cli");
    let lock = write_the_thread_writer_lock(&m, &cfg, &entry_id, "");
    let locks = lock
        .parent()
        .expect("the directory it sits in")
        .to_path_buf();
    std::fs::set_permissions(&locks, std::fs::Permissions::from_mode(0o000)).unwrap();

    let rec = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    let findings = check_locks(
        &support::connector("codex-cli").manifest,
        &LockRoots::from_entry(&rec),
    );
    // Put it back before any assertion, so a failure here does not leave a directory the
    // test harness cannot clean up.
    std::fs::set_permissions(&locks, std::fs::Permissions::from_mode(0o755)).unwrap();
    let findings = findings.expect("a file path needs no directory listing to be answered");
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].path, lock.display().to_string());
    assert_eq!(
        findings[0].holder_pid, None,
        "a file this process cannot read names no holder, and is still a lock"
    );
}

/// The interrupt window on a resume: the child goes, and the entry does not.
///
/// A Ctrl-C
/// between the spawn and `register_process` is the one window where a relaunched harness is
/// running that nothing will ever measure, so it must be stopped — and it is the window
/// `goalcup start` UNMAKES the entry in, which a resume must never do. The entry it is
/// resuming is a run that has already been recorded: abandoning it would delete a workspace
/// and a chain over a keystroke.
///
/// The `SIGINT` is real, and raised at this process, because the handler being installed at
/// all is half of what is under test. [`support::one_interrupt_at_a_time`] is why that does
/// not reach the siblings in this binary.
#[cfg(unix)]
#[test]
fn a_ctrl_c_before_the_hand_off_stops_the_child_and_abandons_nothing() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "claude-code-cli");
    let before = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    let (connectors, survived) =
        support::connectors_dir_with_a_resume_that_outlives_a_kill("claude-code-cli");

    // The handler first, or the raise below ends this process the way an uninstalled SIGINT
    // ends every process.
    goalcup_cli::arming::watch_for_interrupt();
    goalcup_cli::arming::raise_sigint_for_tests();
    let err = resume_with(
        &m,
        &connectors,
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .unwrap_err();
    goalcup_cli::arming::clear_interrupt_for_tests();

    assert_eq!(
        err.exit_code(),
        1,
        "an interrupt is the participant's own doing, not a gate's refusal: {err}"
    );
    assert!(
        format!("{err}").contains("interrupted"),
        "and it says which one it was: {err}"
    );
    assert!(
        m.calls().contains(&"resume".to_string()),
        "the window under test is the one AFTER the tail was caught up and a child was \
         spawned, so this must not have refused earlier: {:?}",
        m.calls()
    );
    assert!(
        !m.calls().contains(&"register_process".to_string()),
        "the daemon was handed nothing it would terminate later"
    );
    assert_eq!(
        m.process_handoff_cancellations(),
        vec![ProcessHandoffCancelRequest {
            expected_generation: m.resume_info().process_generation,
            request_digest: None,
        }],
        "a child stopped locally after `/resume` returns the issued permit"
    );
    assert!(
        m.abandoned().is_empty(),
        "a RESUME never abandons: the run it is picking up is already recorded"
    );
    let after = goalcup_cli::entries::read_entry(&cfg, &entry_id)
        .expect("and this machine's record of it is still there");
    assert_eq!(after.last_pid, before.last_pid, "untouched, not rewritten");
    support::never_appears(&survived);
}

#[cfg(unix)]
#[test]
fn a_pre_post_resume_interrupt_retains_an_unresolved_native_cleanup_latch() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "codex-cli");
    let pidfile = std::path::Path::new(&m.resume_info().workspace).join("pid.txt");
    goalcup_cli::arming::watch_for_interrupt();
    goalcup_cli::arming::raise_sigint_for_tests();
    let error = resume_with(
        &m,
        &support::connectors_dir_with_an_unresolved_tui_native_resume("codex-cli"),
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect_err("the pre-POST interrupt is returned");
    goalcup_cli::arming::clear_interrupt_for_tests();
    assert!(format!("{error}").contains("interrupted"));
    assert!(!m.calls().contains(&"register_process".to_string()));

    let native_pid: u32 = std::fs::read_to_string(pidfile)
        .expect("the unresolved native process exists")
        .trim()
        .parse()
        .unwrap();
    assert!(goalcup_protocol::process::is_alive(native_pid));
    let path = goalcup_cli::entries::entry_path(&cfg, &entry_id);
    let raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(
        raw.get("pending_unresolved_native_cleanup")
            .and_then(|value| value.get("origin")),
        Some(&serde_json::Value::String("resume".to_string()))
    );
    assert!(raw.get("pending_process_cleanup").is_some());
    assert_eq!(m.resume_requests().len(), 1);
    assert_eq!(m.process_handoff_cancellations().len(), 1);

    std::process::Command::new("kill")
        .args(["-KILL", &native_pid.to_string()])
        .status()
        .unwrap();
    support::wait_until_gone(native_pid);
}

/// The other end of the same window: the daemon refused the hand-off.
///
/// Past this call the
/// daemon owns the process; short of it, nothing does — so a refusal here has to take the
/// child with it, must say what the daemon said (there is nothing else to go on), and still
/// must not abandon the entry.
#[test]
fn a_refused_hand_off_stops_the_child_names_the_refusal_and_abandons_nothing() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "claude-code-cli");
    let before = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    m.fail_always(
        "register_process",
        ControlError::Conflict("a handle already stands for this entry".into()),
    );
    let w = support::watched(&m);
    let (connectors, survived) =
        support::connectors_dir_with_a_resume_that_outlives_a_kill("claude-code-cli");

    let err = resume_with(
        &w,
        &connectors,
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert_eq!(err.exit_code(), 1, "the daemon refusing, on its own rung");
    assert!(
        format!("{err}").contains("a handle already stands for this entry"),
        "the daemon's own words are the whole of what is known: {err}"
    );
    let handed = w
        .registered_handle()
        .expect("the daemon was offered a process before it refused");
    let offered = m.process_registrations();
    assert_eq!(offered.len(), 1, "a non-Busy refusal is not replayed");
    assert_eq!(
        offered[0].generation,
        m.resume_info().process_generation,
        "the offered child is bound to the generation `/resume` issued"
    );
    assert_eq!(
        offered[0].handle, handed,
        "the cancellation digest is computed over the same termination authority offered"
    );
    assert_eq!(
        m.process_handoff_cancellations(),
        vec![ProcessHandoffCancelRequest {
            expected_generation: offered[0].generation,
            request_digest: Some(offered[0].canonical_handle_digest(&entry_id)),
        }],
        "a refused prepared handoff is cleaned up only by its exact authenticated digest"
    );
    support::wait_until_gone(handed.pid);
    support::never_appears(&survived);
    assert!(m.abandoned().is_empty(), "a resume never abandons");
    let after = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    assert_eq!(
        after.last_pid, before.last_pid,
        "and the pid is only written back once the daemon has taken the handle"
    );
    assert!(
        after.pending_process_registration.is_none(),
        "a definite Conflict must not leave an exact replay body for a child just stopped"
    );
    assert!(
        after.resume_recovery.is_none(),
        "the cancellation that closed this command's permit also consumes its recovery marker"
    );
}

#[test]
fn an_unresolved_tui_native_resume_refusal_stays_a_manual_cleanup_blocker() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "codex-cli");
    m.fail_always(
        "register_process",
        ControlError::Conflict("the replacement was definitely refused".into()),
    );
    let watched = support::watched(&m);
    let pidfile = std::path::Path::new(&m.resume_info().workspace).join("pid.txt");
    watched.answer_register_only_after(&pidfile);

    resume_with(
        &watched,
        &support::connectors_dir_with_an_unresolved_tui_native_resume("codex-cli"),
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect_err("the definite refusal is returned");

    let handle = watched
        .registered_handle()
        .expect("the wrapper was offered");
    let native_pid: u32 = std::fs::read_to_string(&pidfile)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_eq!(handle.pgid, None);
    assert_eq!(
        handle.native_pid, None,
        "bounded discovery intentionally missed"
    );
    support::wait_until_gone(handle.pid);
    assert!(goalcup_protocol::process::is_alive(native_pid));
    let path = goalcup_cli::entries::entry_path(&cfg, &entry_id);
    let raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(
        raw.get("pending_unresolved_native_cleanup")
            .and_then(|value| value.get("origin")),
        Some(&serde_json::Value::String("resume".to_string()))
    );
    assert!(raw.get("pending_process_registration").is_none());
    assert!(raw.get("pending_process_cleanup").is_some());

    let registrations_before = m.process_registrations().len();
    let resumes_before = m.resume_requests().len();
    resume_with(
        &watched,
        &support::connectors_dir(),
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect_err("persisted unresolved cleanup cannot relaunch");
    assert_eq!(m.process_registrations().len(), registrations_before);
    assert_eq!(m.resume_requests().len(), resumes_before);

    std::process::Command::new("kill")
        .args(["-KILL", &native_pid.to_string()])
        .status()
        .unwrap();
    support::wait_until_gone(native_pid);
}

/// A verified process refusal still leaves a resume generation open until its exact
/// cancellation succeeds. The rejected registration must move to the non-replayable cleanup
/// latch before cancellation or Child termination; the pre-POST recovery marker retains the
/// one generation transition a successor may reconcile.
#[test]
fn a_failed_definite_refusal_cancellation_stops_the_child_and_keeps_cleanup_and_recovery() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "claude-code-cli");
    let before = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    m.fail_always(
        "register_process",
        ControlError::Conflict("the daemon refused this replacement child".into()),
    );
    m.fail_always(
        "cancel_process_handoff",
        ControlError::Unreachable("the cancellation response was lost".into()),
    );
    let (connectors, _survived) =
        support::connectors_dir_with_a_resume_that_outlives_a_kill("claude-code-cli");

    let error = resume_with(
        &m,
        &connectors,
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect_err("the refusal still reaches the participant when its cleanup cannot confirm");
    assert!(format!("{error}").contains("the daemon refused this replacement child"));

    let offered = m.process_registrations();
    assert_eq!(offered.len(), 1, "the definite refusal is not retried");
    let registration = offered[0].clone();
    let alive_after_failed_cancellation =
        goalcup_protocol::process::is_alive(registration.handle.pid);
    if alive_after_failed_cancellation {
        std::process::Command::new("kill")
            .args(["-TERM", &registration.handle.pid.to_string()])
            .status()
            .unwrap();
        support::wait_until_gone(registration.handle.pid);
    }
    assert!(
        !alive_after_failed_cancellation,
        "an unconfirmed cancellation must not orphan this invocation's child"
    );
    assert!(
        !m.process_handoff_cancellations().is_empty(),
        "the exact cancellation was attempted before reporting the original refusal"
    );
    assert!(
        m.abandoned().is_empty(),
        "resume never abandons a recorded run"
    );
    let pending = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    assert_eq!(
        pending.process_generation, before.process_generation,
        "a failed cancellation does not make the unconfirmed generation authoritative locally"
    );
    assert_eq!(
        pending.last_pid, before.last_pid,
        "the definitely rejected child is never retained as local process authority"
    );
    assert_eq!(
        pending.pending_process_registration, None,
        "a definite refusal leaves no rejected registration for a successor to replay"
    );
    assert_eq!(
        pending.pending_process_cleanup,
        Some(registration),
        "the non-replayable cleanup identity survives until cancellation is confirmed"
    );
    assert_eq!(
        pending.pending_process_cleanup_origin,
        Some(goalcup_cli::entries::ProcessCleanupOrigin::Resume)
    );
    assert_eq!(
        pending.resume_recovery,
        Some(goalcup_cli::entries::ResumeRecoveryIntent {
            expected_generation: m.resume_requests()[0].expected_generation,
        }),
        "the exact resume transition remains available for successor reconciliation"
    );
}

#[test]
fn a_cleanup_cache_write_failure_still_stops_the_retained_resume_child() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "claude-code-cli");
    m.fail_always(
        "register_process",
        ControlError::Conflict("the replacement was definitely refused".into()),
    );
    let watched = support::watched(&m);
    watched.break_entry_cache_after_register(&cfg);

    resume_with(
        &watched,
        &support::connectors_dir_with_a_resume_that_outlives_a_kill("claude-code-cli").0,
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect_err("the cleanup-only transition could not be persisted");
    let handed = watched.registered_handle().expect("the child was offered");
    let alive = goalcup_protocol::process::is_alive(handed.pid);
    if alive {
        std::process::Command::new("kill")
            .args(["-KILL", &handed.pid.to_string()])
            .status()
            .unwrap();
        support::wait_until_gone(handed.pid);
    }
    assert!(!alive, "the retained resume Child is terminated and waited");
    assert!(
        m.process_handoff_cancellations().is_empty(),
        "without a durable cleanup-only transition, the successor's recovery row stays authoritative"
    );

    std::fs::remove_file(cfg.join("entries")).unwrap();
    std::fs::rename(cfg.join("entries-held-by-test"), cfg.join("entries")).unwrap();
    let record = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    assert!(record.pending_process_registration.is_some());
    assert_eq!(
        record.pending_process_registration_origin,
        Some(goalcup_cli::entries::ProcessCleanupOrigin::Resume)
    );
    assert!(record.resume_recovery.is_some());
}

#[test]
fn an_ambiguous_resume_handoff_keeps_the_child_and_replays_without_cancellation() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "claude-code-cli");
    let w = support::watched(&m);
    w.malform_accepted_process_replies_until_released();
    let (connectors, survived) =
        support::connectors_dir_with_a_resume_that_outlives_a_kill("claude-code-cli");

    let error = resume_with(
        &w,
        &connectors,
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect_err("a malformed response cannot prove the issued child was refused");
    assert_eq!(error.exit_code(), 3);
    assert_eq!(
        m.process_registrations().len(),
        1,
        "the malformed response followed durable admission of the exact child"
    );
    let handed = w.registered_handle().unwrap();
    assert!(goalcup_protocol::process::is_alive(handed.pid));
    assert!(m.process_handoff_cancellations().is_empty());
    assert!(
        goalcup_cli::entries::read_entry(&cfg, &entry_id)
            .unwrap()
            .pending_process_registration
            .is_some(),
        "the exact body survives for replay"
    );

    w.release_accepted_process_reply();
    resume_with(
        &w,
        &connectors,
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect("a later resume replays the pending body before another CAS");
    assert_eq!(
        w.registered_handles().last(),
        Some(&handed),
        "the later acknowledgement was for the original durable registration"
    );
    assert_eq!(m.process_registrations().len(), 1);
    assert_eq!(
        m.resume_requests().len(),
        1,
        "only the first invocation spent a CAS"
    );
    assert!(m.process_handoff_cancellations().is_empty());
    assert!(
        goalcup_cli::entries::read_entry(&cfg, &entry_id)
            .unwrap()
            .pending_process_registration
            .is_none()
    );

    std::process::Command::new("kill")
        .args(["-TERM", &handed.pid.to_string()])
        .status()
        .unwrap();
    support::wait_until_gone(handed.pid);
    support::never_appears(&survived);
}

/// The first invocation cannot know whether any request byte reached the daemon, so it
/// leaves the child and exact registration pending.  A later explicit pre-mutation refusal
/// settles that question: the cached body must become non-replayable before cleanup and the
/// one issued resume generation must be cancelled by digest. A successor has no retained
/// Child handle, so an exact-live instance becomes an explicit blocker rather than a PID.
#[test]
fn an_ambiguous_handoff_then_definite_replay_refusal_retires_replay_and_requires_safe_cleanup() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "claude-code-cli");
    let predecessor_pid = goalcup_cli::entries::read_entry(&cfg, &entry_id)
        .unwrap()
        .last_pid;
    m.fail_always(
        "register_process",
        ControlError::Unreachable("the first process request had no readable response".into()),
    );
    let w = support::watched(&m);
    let (connectors, survived) =
        support::connectors_dir_with_a_resume_that_outlives_a_kill("claude-code-cli");

    resume_with(
        &w,
        &connectors,
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect_err("the first response remains admission-ambiguous");
    let pending = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    let registration = pending
        .pending_process_registration
        .clone()
        .expect("the exact body survives ambiguity");
    assert!(goalcup_protocol::process::is_alive(registration.handle.pid));
    assert!(survived.exists(), "the ambiguous child remains alive");
    assert!(m.process_handoff_cancellations().is_empty());

    m.fail_always(
        "register_process",
        ControlError::Conflict("the exact request was proven absent before mutation".into()),
    );
    let error = resume_with(
        &w,
        &connectors,
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect_err("the later definite refusal remains the participant-visible result");
    assert!(
        format!("{error}").contains("proven absent before mutation")
            || format!("{error}").contains("cleanup must be verified")
    );
    assert!(
        goalcup_protocol::process::is_alive(registration.handle.pid),
        "a later CLI process never signals a cached numeric pid"
    );
    assert!(
        survived.exists(),
        "the marker predates the later definite response and proves ambiguity kept the child"
    );
    assert_eq!(
        m.process_handoff_cancellations(),
        vec![ProcessHandoffCancelRequest {
            expected_generation: registration.generation,
            request_digest: Some(registration.canonical_handle_digest(&entry_id)),
        }],
        "cleanup closes exactly the permit and digest that produced the pending child"
    );
    let repaired = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    assert!(repaired.pending_process_registration.is_none());
    assert!(repaired.resume_recovery.is_none());
    assert_eq!(
        repaired.last_pid, predecessor_pid,
        "the candidate never replaces the daemon-owned predecessor before a 204"
    );
    assert_eq!(
        repaired.pending_process_cleanup,
        Some(registration.clone()),
        "the non-replayable marker blocks relaunch until the cached child is verified gone"
    );
    std::process::Command::new("kill")
        .args(["-KILL", &registration.handle.pid.to_string()])
        .status()
        .unwrap();
    support::wait_until_gone(registration.handle.pid);
}

/// A cached pid is not a retained OS process handle.  If that number now names a different
/// process instance, cleanup must make the rejected body non-replayable, treat the original
/// instance as gone, and never signal the stranger.
#[test]
fn definite_replay_cleanup_refuses_to_signal_a_recycled_pid() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "claude-code-cli");
    let mut stranger = support::ChildGuard::new(
        std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .expect("the stranger starts"),
    );
    let mut record = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    let predecessor_pid = record.last_pid;
    let handle = ProcessHandle {
        pid: stranger.id(),
        pgid: None,
        native_pid: None,
        native_binary: None,
        native_process_start: None,
        process_start: Some("a different process instance".to_string()),
        strategy: KillStrategy::NativePid,
        grace_seconds: 0,
        launched_at: "2026-09-11T00:00:00Z".to_string(),
        program: "/bin/sleep".to_string(),
        argv: vec!["/bin/sleep".to_string(), "30".to_string()],
        env_keys: Vec::new(),
        home_seed: None,
    };
    let registration = ProcessRegistration {
        generation: m.resume_info().process_generation,
        handle,
    };
    record.pending_process_registration = Some(registration.clone());
    record.resume_recovery = Some(goalcup_cli::entries::ResumeRecoveryIntent {
        expected_generation: registration.generation.saturating_sub(1),
    });
    goalcup_cli::entries::write_entry(&cfg, &record).unwrap();
    m.fail_always(
        "register_process",
        ControlError::Conflict("the exact request was absent before mutation".into()),
    );

    let error = resume_with(
        &m,
        &support::connectors_dir(),
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect_err("a mismatched process identity cannot be cleaned automatically");
    assert!(!format!("{error}").is_empty());
    assert!(
        goalcup_protocol::process::is_alive(stranger.id()),
        "the recycled pid was never signalled"
    );
    let raw: serde_json::Value = serde_json::from_slice(
        &std::fs::read(cfg.join("entries").join(format!("{entry_id}.json"))).unwrap(),
    )
    .unwrap();
    assert!(
        raw.get("pending_process_registration").is_none(),
        "a definitively rejected body is never replayed again"
    );
    assert_eq!(raw["last_pid"], serde_json::json!(predecessor_pid));
    assert!(
        raw.get("pending_process_cleanup").is_none(),
        "a mismatched creation identity proves the original candidate is gone"
    );
    stranger.reap();
}

/// The durable unresolved-native latch is stronger than a later process-table snapshot.
/// Here the cached wrapper pid now names a different `sleep` instance, while another
/// `sleep` models the native child that escaped the original wrapper. A definite replay
/// refusal must retire the body without recomputing the latch from the recycled pid.
#[test]
fn cached_refusal_preserves_an_unresolved_native_latch_across_same_name_pid_reuse() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "claude-code-cli");
    let mut recycled = support::ChildGuard::new(
        std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .expect("the same-name recycled process starts"),
    );
    let mut orphan = support::ChildGuard::new(
        std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .expect("the undiscovered native survivor starts"),
    );
    let mut record = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    let registration = ProcessRegistration {
        generation: record.process_generation,
        handle: ProcessHandle {
            pid: recycled.id(),
            pgid: None,
            native_pid: None,
            native_binary: Some("sleep".to_string()),
            native_process_start: None,
            process_start: Some("the original wrapper instance is gone".to_string()),
            strategy: KillStrategy::NativePid,
            grace_seconds: 0,
            launched_at: "2026-09-11T00:00:00Z".to_string(),
            program: "/bin/sh".to_string(),
            argv: vec!["/bin/sh".to_string()],
            env_keys: Vec::new(),
            home_seed: None,
        },
    };
    let unresolved = goalcup_cli::entries::UnresolvedNativeCleanup {
        origin: goalcup_cli::entries::ProcessCleanupOrigin::Initial,
        native_binary: "sleep".to_string(),
    };
    record.pending_process_registration = Some(registration.clone());
    record.pending_process_registration_origin =
        Some(goalcup_cli::entries::ProcessCleanupOrigin::Initial);
    record.pending_unresolved_native_cleanup = Some(unresolved.clone());
    goalcup_cli::entries::write_entry(&cfg, &record).unwrap();
    m.fail_always(
        "register_process",
        ControlError::Conflict("the cached body was definitely refused".into()),
    );

    resume_with(
        &m,
        &support::connectors_dir(),
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect_err("the cached body is retired into unresolved cleanup");

    assert!(goalcup_protocol::process::is_alive(recycled.id()));
    assert!(goalcup_protocol::process::is_alive(orphan.id()));
    assert!(m.process_handoff_cancellations().is_empty());
    let retained = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    assert!(retained.pending_process_registration.is_none());
    assert_eq!(retained.pending_process_cleanup, Some(registration));
    assert_eq!(
        retained.pending_unresolved_native_cleanup,
        Some(unresolved.clone()),
        "the existing marker is preserved byte-for-byte, not inferred from a recycled pid"
    );

    let registrations = m.process_registrations().len();
    resume_with(
        &m,
        &support::connectors_dir(),
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect_err("unresolved cleanup blocks the next invocation");
    assert_eq!(m.process_registrations().len(), registrations);
    assert!(m.resume_requests().is_empty());
    assert!(m.process_handoff_cancellations().is_empty());
    assert_eq!(
        goalcup_cli::entries::read_entry(&cfg, &entry_id)
            .unwrap()
            .pending_unresolved_native_cleanup,
        Some(unresolved)
    );

    recycled.reap();
    orphan.reap();
}

/// Unlike a refusal, a genuine exact acknowledgement transfers the registered handle to
/// the daemon. That protocol boundary intentionally retires the speculative local latch.
#[test]
fn cached_exact_ack_retires_the_speculative_unresolved_native_latch() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "claude-code-cli");
    let mut record = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    record.pending_process_registration = Some(ProcessRegistration {
        generation: record.process_generation,
        handle: ProcessHandle {
            pid: u32::MAX - 20,
            pgid: None,
            native_pid: None,
            native_binary: Some("sleep".to_string()),
            native_process_start: None,
            process_start: Some("the previously offered wrapper".to_string()),
            strategy: KillStrategy::NativePid,
            grace_seconds: 0,
            launched_at: "2026-09-11T00:00:00Z".to_string(),
            program: "/bin/sh".to_string(),
            argv: vec!["/bin/sh".to_string()],
            env_keys: Vec::new(),
            home_seed: None,
        },
    });
    record.pending_process_registration_origin =
        Some(goalcup_cli::entries::ProcessCleanupOrigin::Initial);
    record.pending_unresolved_native_cleanup =
        Some(goalcup_cli::entries::UnresolvedNativeCleanup {
            origin: goalcup_cli::entries::ProcessCleanupOrigin::Initial,
            native_binary: "sleep".to_string(),
        });
    goalcup_cli::entries::write_entry(&cfg, &record).unwrap();

    resume_with(
        &m,
        &support::connectors_dir(),
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect("the daemon acknowledges the exact cached handoff");

    let repaired = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    assert!(repaired.pending_process_registration.is_none());
    assert!(
        repaired.pending_unresolved_native_cleanup.is_none(),
        "an exact daemon ACK, unlike a refusal, transfers lifecycle authority"
    );
    assert_eq!(m.process_registrations().len(), 1);
    assert!(m.resume_requests().is_empty());
    assert!(m.process_handoff_cancellations().is_empty());
}

/// A failed spawn syscall leaves no child authority. Even when cancellation of the issued
/// resume permit is temporarily unconfirmed, the speculative native guard must be removed
/// so the next command reaches recovery instead of returning at the guard.
#[test]
fn resume_spawn_error_clears_the_guard_and_prioritizes_permit_recovery() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "codex-cli");
    m.fail_next(
        "cancel_process_handoff",
        ControlError::Unreachable("the first cancellation response was unavailable".into()),
    );
    let connectors = support::connectors_dir_with_an_unspawnable_tui_native("codex-cli");

    resume_with(
        &m,
        &connectors,
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect_err("the missing executable cannot spawn");

    assert!(m.process_registrations().is_empty());
    assert_eq!(m.resume_requests().len(), 1);
    assert_eq!(m.process_handoff_cancellations().len(), 1);
    let retained = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    assert!(retained.pending_process_registration.is_none());
    assert!(retained.pending_process_cleanup.is_none());
    assert!(
        retained.pending_unresolved_native_cleanup.is_none(),
        "the spawn error proves the speculative child never existed"
    );
    assert!(
        retained.resume_recovery.is_some(),
        "the unconfirmed permit cancellation remains the next command's first authority"
    );

    let open_generation = retained
        .resume_recovery
        .as_ref()
        .unwrap()
        .expected_generation
        .checked_add(1)
        .unwrap();
    m.set_resume_handoff(open_generation, "open_resume");
    m.fail_always(
        "resume",
        ControlError::Conflict("stop after the recovery cancellation".into()),
    );
    let calls_before = m.calls().len();
    let second = resume_with(
        &m,
        &connectors,
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect_err("the fixture stops after repairing the old permit");
    assert!(format!("{second}").contains("stop after the recovery cancellation"));
    assert_eq!(m.process_registrations().len(), 0);
    assert_eq!(m.process_handoff_cancellations().len(), 2);
    let second_calls = &m.calls()[calls_before..];
    assert_eq!(
        &second_calls[..3],
        ["resume_preview", "cancel_process_handoff", "resume_preview"],
        "the old permit is repaired before any new resume CAS or spawn"
    );
}

/// A live numeric pid with no readable creation identity is not absence evidence. Cleanup
/// stays fail-closed and never signals it, even after the resume permit is settled.
#[test]
fn cleanup_with_an_unknown_live_process_identity_remains_a_blocker() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "claude-code-cli");
    let mut child = support::ChildGuard::new(
        std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .expect("the identity-unknown fixture starts"),
    );
    let mut record = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    let registration = ProcessRegistration {
        generation: m.resume_info().process_generation,
        handle: ProcessHandle {
            pid: child.id(),
            pgid: None,
            native_pid: None,
            native_binary: None,
            native_process_start: None,
            process_start: None,
            strategy: KillStrategy::NativePid,
            grace_seconds: 0,
            launched_at: "2026-09-11T00:00:00Z".to_string(),
            program: "/bin/sleep".to_string(),
            argv: vec!["/bin/sleep".to_string(), "30".to_string()],
            env_keys: Vec::new(),
            home_seed: None,
        },
    };
    record.pending_process_cleanup = Some(registration.clone());
    goalcup_cli::entries::write_entry(&cfg, &record).unwrap();

    let error = resume_with(
        &m,
        &support::connectors_dir(),
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect_err("unknown identity cannot complete cached cleanup");
    assert!(format!("{error}").contains("could not be proven absent"));
    assert!(goalcup_protocol::process::is_alive(child.id()));
    let retained = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    assert_eq!(retained.pending_process_cleanup, Some(registration));
    assert!(retained.pending_process_registration.is_none());
    child.reap();
}

#[test]
fn cleanup_does_not_clear_when_the_wrapper_is_gone_but_exact_native_child_lives() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "claude-code-cli");
    let mut native = support::ChildGuard::new(
        std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .expect("the escaped native fixture starts"),
    );
    let mut record = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    let registration = ProcessRegistration {
        generation: record.process_generation,
        handle: ProcessHandle {
            pid: u32::MAX - 11,
            pgid: None,
            native_pid: Some(native.id()),
            native_binary: Some("sleep".to_string()),
            native_process_start: goalcup_protocol::process::process_start_identity(native.id()),
            process_start: Some("the wrapper is already gone".to_string()),
            strategy: KillStrategy::NativePid,
            grace_seconds: 0,
            launched_at: "2026-09-11T00:00:00Z".to_string(),
            program: "/bin/sh".to_string(),
            argv: vec!["/bin/sh".to_string()],
            env_keys: Vec::new(),
            home_seed: None,
        },
    };
    record.pending_process_cleanup = Some(registration.clone());
    record.pending_process_cleanup_origin =
        Some(goalcup_cli::entries::ProcessCleanupOrigin::Initial);
    goalcup_cli::entries::write_entry(&cfg, &record).unwrap();

    let error = resume_with(
        &m,
        &support::connectors_dir(),
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect_err("the exact native instance remains a cleanup blocker");
    assert!(format!("{error}").contains("same live process"));
    assert!(goalcup_protocol::process::is_alive(native.id()));
    let retained = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    assert_eq!(retained.pending_process_cleanup, Some(registration));
    assert!(m.process_registrations().is_empty());
    assert!(m.resume_requests().is_empty());

    native.reap();
}

#[test]
fn cleanup_clears_a_recycled_native_pid_without_signalling_the_stranger() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "claude-code-cli");
    let mut stranger = support::ChildGuard::new(
        std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .expect("the recycled native pid fixture starts"),
    );
    let mut record = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    let registration = ProcessRegistration {
        generation: record.process_generation,
        handle: ProcessHandle {
            pid: u32::MAX - 12,
            pgid: None,
            native_pid: Some(stranger.id()),
            native_binary: Some("sleep".to_string()),
            native_process_start: Some("a different native process instance".to_string()),
            process_start: Some("the wrapper is gone".to_string()),
            strategy: KillStrategy::NativePid,
            grace_seconds: 0,
            launched_at: "2026-09-11T00:00:00Z".to_string(),
            program: "/bin/sh".to_string(),
            argv: vec!["/bin/sh".to_string()],
            env_keys: Vec::new(),
            home_seed: None,
        },
    };
    record.pending_process_cleanup = Some(registration);
    record.pending_process_cleanup_origin =
        Some(goalcup_cli::entries::ProcessCleanupOrigin::Initial);
    goalcup_cli::entries::write_entry(&cfg, &record).unwrap();
    m.fail_always(
        "resume",
        ControlError::Conflict("stop after recycled cleanup".into()),
    );

    resume_with(
        &m,
        &support::connectors_dir(),
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect_err("the fixture stops after cleanup");
    assert!(goalcup_protocol::process::is_alive(stranger.id()));
    let repaired = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    assert!(repaired.pending_process_cleanup.is_none());
    assert!(m.process_registrations().is_empty());

    stranger.reap();
}

/// A fixture child is owned by the test even when an assertion unwinds before the normal
/// success-path reap.  The guard is deliberately exercised through an unwind so a future
/// refactor cannot quietly turn it back into end-of-test cleanup.
#[test]
fn fixture_child_guard_reaps_a_standin_during_unwind() {
    let child = std::process::Command::new("/bin/sleep")
        .arg("30")
        .spawn()
        .expect("the guard fixture starts");
    let pid = child.id();

    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = support::ChildGuard::new(child);
        panic!("exercise the guard's unwind path");
    }));

    assert!(panicked.is_err(), "the fixture reached its forced unwind");
    support::wait_until_gone(pid);
}

/// The replay-to-cleanup cache transition is the crash boundary. A successor finishes the
/// exact cancellation before doing anything else and never reposts the rejected body.
#[test]
fn a_persisted_cleanup_marker_resumes_cleanup_without_reposting() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "claude-code-cli");
    let mut record = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    let registration = ProcessRegistration {
        generation: m.resume_info().process_generation,
        handle: ProcessHandle {
            pid: u32::MAX - 1,
            pgid: None,
            native_pid: None,
            native_binary: None,
            native_process_start: None,
            process_start: Some("a process that is already gone".to_string()),
            strategy: KillStrategy::NativePid,
            grace_seconds: 0,
            launched_at: "2026-09-11T00:00:00Z".to_string(),
            program: "/bin/false".to_string(),
            argv: vec!["/bin/false".to_string()],
            env_keys: Vec::new(),
            home_seed: None,
        },
    };
    record.last_pid = None;
    record.pending_process_registration = None;
    record.pending_process_cleanup = Some(registration.clone());
    record.resume_recovery = Some(goalcup_cli::entries::ResumeRecoveryIntent {
        expected_generation: registration.generation.saturating_sub(1),
    });
    goalcup_cli::entries::write_entry(&cfg, &record).unwrap();
    m.fail_always(
        "resume",
        ControlError::Conflict("stop after cleanup, before another launch".into()),
    );

    let error = resume_with(
        &m,
        &support::connectors_dir(),
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect_err("the later resume refusal stops the test after cleanup");
    assert!(format!("{error}").contains("stop after cleanup"));
    assert!(m.process_registrations().is_empty());
    assert_eq!(
        m.process_handoff_cancellations(),
        vec![ProcessHandoffCancelRequest {
            expected_generation: registration.generation,
            request_digest: Some(registration.canonical_handle_digest(&entry_id)),
        }]
    );
    let repaired = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    assert!(repaired.pending_process_registration.is_none());
    assert!(repaired.pending_process_cleanup.is_none());
    assert!(
        repaired.resume_recovery.is_some(),
        "the following independent resume attempt may leave its own recovery marker"
    );
}

/// A start-side crash can leave only an Initial cleanup latch. It has no resume permit to
/// cancel: once the exact child is confirmed gone, a successor clears the latch before its
/// ordinary resume work and never sends the rejected registration again.
#[test]
fn an_initial_cleanup_marker_clears_without_process_cancel_or_replay() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "claude-code-cli");
    let mut record = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    let registration = ProcessRegistration {
        generation: record.process_generation,
        handle: ProcessHandle {
            pid: u32::MAX - 3,
            pgid: None,
            native_pid: None,
            native_binary: None,
            native_process_start: None,
            process_start: Some("already-gone-initial-child".to_string()),
            strategy: KillStrategy::NativePid,
            grace_seconds: 0,
            launched_at: "2026-09-11T00:00:00Z".to_string(),
            program: "/bin/false".to_string(),
            argv: vec!["/bin/false".to_string()],
            env_keys: Vec::new(),
            home_seed: None,
        },
    };
    record.pending_process_cleanup = Some(registration);
    record.pending_process_cleanup_origin =
        Some(goalcup_cli::entries::ProcessCleanupOrigin::Initial);
    goalcup_cli::entries::write_entry(&cfg, &record).unwrap();
    m.fail_always(
        "resume",
        ControlError::Conflict("stop after initial cleanup".into()),
    );

    let error = resume_with(
        &m,
        &support::connectors_dir(),
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect_err("the fixture stops after cleanup");
    assert!(format!("{error}").contains("stop after initial cleanup"));
    assert!(m.process_registrations().is_empty());
    assert!(m.process_handoff_cancellations().is_empty());
    let repaired = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    assert!(repaired.pending_process_cleanup.is_none());
    assert!(repaired.pending_process_cleanup_origin.is_none());
}

#[test]
fn an_unconfirmed_cached_cleanup_blocks_relaunch_and_retains_only_recovery_authority() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "claude-code-cli");
    let mut record = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    let registration = ProcessRegistration {
        generation: m.resume_info().process_generation,
        handle: ProcessHandle {
            pid: u32::MAX - 2,
            pgid: None,
            native_pid: None,
            native_binary: None,
            native_process_start: None,
            process_start: Some("gone".to_string()),
            strategy: KillStrategy::NativePid,
            grace_seconds: 0,
            launched_at: "2026-09-11T00:00:00Z".to_string(),
            program: "/bin/false".to_string(),
            argv: vec!["/bin/false".to_string()],
            env_keys: Vec::new(),
            home_seed: None,
        },
    };
    record.last_pid = None;
    record.pending_process_cleanup = Some(registration.clone());
    record.resume_recovery = Some(goalcup_cli::entries::ResumeRecoveryIntent {
        expected_generation: registration.generation.saturating_sub(1),
    });
    goalcup_cli::entries::write_entry(&cfg, &record).unwrap();
    m.fail_always(
        "cancel_process_handoff",
        ControlError::Unreachable("the exact cancellation outcome is unknown".into()),
    );

    let error = resume_with(
        &m,
        &support::connectors_dir(),
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect_err("unconfirmed cleanup blocks all later resume work");
    assert!(format!("{error}").contains("cancellation outcome is unknown"));
    assert!(m.process_registrations().is_empty());
    assert!(m.resume_requests().is_empty());
    let retained = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    assert!(retained.pending_process_registration.is_none());
    assert_eq!(retained.pending_process_cleanup, Some(registration));
    assert!(retained.resume_recovery.is_some());
    assert_eq!(retained.last_pid, None);
}

#[test]
fn an_accepted_resume_reply_lost_through_retry_exhaustion_replays_without_cancellation() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "claude-code-cli");
    let predecessor_pid = goalcup_cli::entries::read_entry(&cfg, &entry_id)
        .unwrap()
        .last_pid;
    let w = support::watched(&m);
    w.lose_accepted_process_replies_until_released();
    let (connectors, survived) =
        support::connectors_dir_with_a_resume_that_outlives_a_kill("claude-code-cli");

    let error = resume_with(
        &w,
        &connectors,
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect_err("lost responses exhaust the bounded replay envelope");
    assert_eq!(error.exit_code(), 1);
    assert_eq!(m.resume_requests().len(), 1);
    let offered = w.registered_handles();
    assert_eq!(
        offered.len(),
        goalcup_cli::retry::PROCESS_BUSY_ATTEMPTS,
        "the exact accepted body was replayed in-command until the bounded limit"
    );
    assert!(
        offered.windows(2).all(|pair| pair[0] == pair[1]),
        "each bounded replay retained the exact issued child authority"
    );
    assert_eq!(m.process_registrations().len(), 1);
    assert!(m.process_handoff_cancellations().is_empty());

    let handed = offered.last().cloned().unwrap();
    assert!(goalcup_protocol::process::is_alive(handed.pid));
    assert!(
        survived.exists(),
        "the admitted child lived through the whole exhausted replay envelope"
    );
    let pending = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
    assert_eq!(
        pending.last_pid, predecessor_pid,
        "an ambiguous candidate does not replace the predecessor before its 204"
    );
    assert_eq!(
        pending.pending_process_registration,
        Some(ProcessRegistration {
            generation: m.resume_info().process_generation,
            handle: handed.clone(),
        })
    );

    w.release_accepted_process_reply();
    resume_with(
        &w,
        &connectors,
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut Vec::new(),
    )
    .expect("the later invocation replays before another resume CAS");
    assert_eq!(
        w.registered_handles().last(),
        Some(&handed),
        "the later acknowledgement was for the same exhausted replay body"
    );
    assert_eq!(m.process_registrations().len(), 1);
    assert_eq!(m.resume_requests().len(), 1);
    assert!(m.process_handoff_cancellations().is_empty());
    assert!(
        goalcup_cli::entries::read_entry(&cfg, &entry_id)
            .unwrap()
            .pending_process_registration
            .is_none()
    );

    std::process::Command::new("kill")
        .args(["-TERM", &handed.pid.to_string()])
        .status()
        .unwrap();
    support::wait_until_gone(handed.pid);
}

/// The cached page URL is checked before the daemon is asked anything, and a record naming
/// something else stops the resume where nothing has been spent on it.
///
/// This is the same field `goalcup ui` refuses: the record is a file on the participant's
/// own disk, and the last
/// line of a resume hands that string to a browser. Checked at step 1 rather than at the
/// open, because the open is past the relaunch and past the hand-off — a refusal there would
/// leave a registered harness running under a command that returned an error.
#[test]
fn a_cached_page_url_that_is_not_a_loopback_page_stops_the_resume_before_the_post() {
    // `resume_with` installs the process-global interrupt handler before it reads anything,
    // so even a test that never reaches a relaunch is inside the window this file's header
    // is about — see [`support::one_interrupt_at_a_time`].
    let _serial = support::one_interrupt_at_a_time();
    for bad in [
        "https://evil.example/v0/ui?entry=entry-mock-0001",
        "http://127.0.0.1/v0/ui?entry=entry-mock-0001",
        "http://127.0.0.1@evil.example:8080/v0/ui",
        "file:///etc/passwd",
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let (m, cfg, entry_id) = resumable(&tmp, "codex-cli");
        let mut rec = goalcup_cli::entries::read_entry(&cfg, &entry_id).unwrap();
        rec.status_url = bad.to_string();
        goalcup_cli::entries::write_entry(&cfg, &rec).unwrap();
        let err = resume_with(
            &m,
            &support::connectors_dir_with_a_resume_that_exits("codex-cli"),
            &cfg,
            &entry_id,
            &support::resume_options(),
            &mut Vec::new(),
        )
        .expect_err("a page this CLI will not open is not one it resumes behind");
        assert_eq!(err.exit_code(), 1, "{bad}");
        assert!(
            format!("{err}").contains(bad),
            "the refusal names it: {err}"
        );
        assert!(
            m.calls().is_empty(),
            "and nothing was asked of the tracker: {bad}"
        );
    }
}

/// What the tracker last held a handle for is on the screen.
///
/// `ResumeInfo.last_process` is the one thing on the resume answer that says what the
/// stopped run WAS, so a participant must be able to see it.
#[test]
fn the_resume_names_the_handle_the_tracker_last_held() {
    let _serial = support::one_interrupt_at_a_time();
    let tmp = tempfile::tempdir().unwrap();
    let (m, cfg, entry_id) = resumable(&tmp, "codex-cli");
    m.set_last_process_pid(424242);
    let mut out = Vec::new();
    resume_with(
        &m,
        &support::connectors_dir_with_a_resume_that_exits("codex-cli"),
        &cfg,
        &entry_id,
        &support::resume_options(),
        &mut out,
    )
    .unwrap();
    let said = String::from_utf8(out).unwrap();
    assert!(
        said.contains("424242") && said.contains("last handle"),
        "the pid the tracker last held a handle for is named: {said}"
    );
}
