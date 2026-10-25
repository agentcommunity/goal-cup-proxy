//! A run that crosses several platform stops (docs/research/RESTARTS-DESIGN.md). A cloud sandbox
//! may stop a session every 24 hours: the harness and `goalcup start` are killed, and at times the
//! whole machine, daemon included, with the disk kept. After each stop the participant runs
//! `goalcup continue`. This file drives that with the chat-mode stand-in harness, a real
//! `goalcup-daemon` process (the binary starts it, so the test can kill it), and the upload
//! server's stand-in on loopback.

use crate::common::m2_cli_support::{Launcher, Rig};
use crate::common::upload_server::{ServerOptions, UploadServer};
use goalcup_protocol::capture::{CaptureRecord, CaptureSource, GapReason};
use goalcup_protocol::capture_status::AnchorState;
use std::path::{Path, PathBuf};

/// A test build's disk sizes: the reservation, no margin, a 50 ms watch.
const DISK: (&str, &str) = ("GOALCUP_TEST_DISK", "4194304,0,50");
/// The uploader's cadence a test can watch, with a heartbeat every 100 ms. The fifth number is the
/// wrap-up: a part not all sent within 10 s of its seal is left for `goalcup upload`.
const BEATING: (&str, &str) = (
    "GOALCUP_TEST_UPLOAD_POLICY",
    "200,25,100,400,10000,268435456,3000,100",
);

/// Ends every daemon started over this rig's configuration, however the test ends, so none is
/// left for its idle grace.
struct Reaper(PathBuf);

impl Drop for Reaper {
    fn drop(&mut self) {
        for pid in daemons_for(&self.0) {
            // SAFETY: two integers; the pid was just seen running this test's daemon.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
    }
}

/// The pids of every `goalcup-daemon --config <config>` on this machine, matched on whole
/// arguments. None when `ps` cannot be run: this also runs in [`Reaper`]'s drop, which must not
/// panic.
fn daemons_for(config: &Path) -> Vec<i32> {
    let Some(table) = std::process::Command::new("ps")
        .args(["-axww", "-o", "pid=,command="])
        .output()
        .ok()
    else {
        return Vec::new();
    };
    let config = config.to_string_lossy();
    String::from_utf8_lossy(&table.stdout)
        .lines()
        .filter_map(|line| {
            let mut tokens = line.split_whitespace();
            let pid = tokens.next()?.parse().ok()?;
            let argv: Vec<&str> = tokens.collect();
            let daemon = argv
                .first()
                .is_some_and(|exe| Path::new(exe).file_name() == Some("goalcup-daemon".as_ref()));
            let ours = argv
                .windows(2)
                .any(|pair| pair[0] == "--config" && pair[1] == config);
            (daemon && ours).then_some(pid)
        })
        .collect()
}

fn daemon_config(rig: &Rig) -> PathBuf {
    rig.cfg.join(goalcup_cli::daemon::spawn::DAEMON_CONFIG_NAME)
}

/// The one daemon of this rig, waited for.
fn the_daemon(rig: &Rig) -> i32 {
    let mut found = Vec::new();
    wait_until("one daemon", || {
        found = daemons_for(&daemon_config(rig));
        found.len() == 1
    });
    found[0]
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !done() {
        assert!(
            std::time::Instant::now() < deadline,
            "{what} never happened"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

fn kill9(pid: i32) {
    // SAFETY: two integers; the pid is one this test started or was told by its stand-in.
    unsafe { libc::kill(pid, libc::SIGKILL) };
}

fn harness(rig: &Rig, name: &str) -> String {
    rig.harness_file(name)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn agent_pid(rig: &Rig) -> i32 {
    harness(rig, "tui.pid").parse().expect("the agent's pid")
}

/// The entry's whole local chain: its chain files joined in order.
fn local_chain(rig: &Rig, entry_id: &str) -> Vec<u8> {
    let mut joined = Vec::new();
    for part in goalcup_cli::upload::prune::parts_now(&rig.trace(entry_id)).unwrap() {
        joined.extend(std::fs::read(&part.path).unwrap());
    }
    joined
}

/// The server's copy of an entry's chain.
fn server_copy(server: &UploadServer, entry_id: &str) -> Vec<u8> {
    let mut chunks = server.chunks_of(entry_id);
    chunks.sort_by_key(|c| c.offset);
    let mut joined = Vec::new();
    for chunk in chunks {
        assert_eq!(chunk.offset, joined.len() as u64, "chunks leave no hole");
        let raw = zstd::decode_all(std::fs::read(&chunk.body_path).unwrap().as_slice()).unwrap();
        joined.extend_from_slice(&raw);
    }
    joined
}

/// Every byte of the trace's native streams, joined.
fn native_bytes(records: &[CaptureRecord]) -> String {
    use base64::Engine;
    let natives: Vec<_> = records
        .iter()
        .filter_map(|r| match r {
            CaptureRecord::Open {
                stream,
                source: CaptureSource::Native,
                ..
            } => Some(stream.clone()),
            _ => None,
        })
        .collect();
    let mut bytes = Vec::new();
    for r in records {
        if let CaptureRecord::Segment {
            stream, data_b64, ..
        } = r
            && natives.contains(stream)
        {
            bytes.extend(
                base64::engine::general_purpose::STANDARD
                    .decode(data_b64)
                    .unwrap(),
            );
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Wait until the entry's chain holds the typed `line` from the harness's session file.
fn wait_recorded(rig: &Rig, entry_id: &str, line: &str) {
    let text = format!(r#""text":"{line}""#);
    wait_until(&format!("{line} in the chain of {entry_id}"), || {
        native_bytes(&rig.read_trace(entry_id).1).contains(&text)
    });
}

/// The stream-less gaps of a trace, by reason.
fn gaps(records: &[CaptureRecord]) -> Vec<GapReason> {
    records
        .iter()
        .filter_map(|r| match r {
            CaptureRecord::Gap { reason, .. } => Some(*reason),
            _ => None,
        })
        .collect()
}

/// A run crosses two platform stops. At "hour 24" the session cap kills `goalcup start` and the
/// agent with SIGKILL and the detached daemon lives on; at "hour 48" the machine powers off:
/// `goalcup start`, the agent and the daemon are killed, and the disk is kept. Each time the
/// participant types `goalcup continue`; at the end they seal the part from another terminal.
///
/// The promises: the run is one run with three segments, each on the server whole, sealed and
/// signed; the gaps each stop leaves are named and nothing is torn; no byte recorded before a
/// stop is lost or changed; the deadline never moves; the clone is the same one, with one
/// approval; every restarted harness is given its resume arguments; the agent can read the
/// time left again after the power-off.
#[test]
fn a_run_crosses_two_platform_stops_and_each_continue_keeps_it_whole() {
    let rig = Rig::chat_run_scoped();
    let _reaper = Reaper(daemon_config(&rig));
    let server = UploadServer::start(&rig.root.join("server"), ServerOptions::default());
    rig.prepare_for(&server.url());

    // Segment 0: the prompt.
    let mut first = rig.start_on_pty(&[DISK, BEATING]);
    first.wait_shown("tui: ready");
    first.type_line("one");
    first.wait_shown("tui: noted one");
    let runs = server.run_ids();
    assert_eq!(runs.len(), 1, "{runs:?}");
    let run_id = runs[0].clone();
    let started = goalcup_cli::run::read(&rig.cfg, &run_id).unwrap();
    let deadline_at = started.deadline_at.clone().expect("a deadline");
    let entry0 = server.entry_ids(&run_id)[0].clone();
    wait_recorded(&rig, &entry0, "one");
    let daemon = the_daemon(&rig);
    // From here the server stores no chunk until after the second stop: what segment 0 and 1
    // record waits on this machine, across both stops.
    server.fail_next_chunks(u64::MAX);

    // Stop 1, the session cap: `goalcup start` and the agent are killed. Nothing is sealed.
    kill9(first.child.id() as i32);
    kill9(agent_pid(&rig));
    first.close();
    first.wait();
    let before0 = local_chain(&rig, &entry0);
    assert_eq!(
        daemons_for(&daemon_config(&rig)),
        [daemon],
        "the daemon lives on"
    );

    let mut second = rig.continue_on_pty(&[DISK, BEATING], None);
    second.wait_shown("tui: ready");
    let shown = second.shown();
    assert!(
        shown.contains("was still open, so it was sealed now"),
        "{shown}"
    );
    assert!(
        harness(&rig, "tui.argv").ends_with("resume\n--last"),
        "{}",
        harness(&rig, "tui.argv")
    );
    second.type_line("two");
    second.wait_shown("tui: noted two");
    let entry1 = server.entry_ids(&run_id)[1].clone();
    wait_recorded(&rig, &entry1, "two");

    // Stop 2, the machine powers off: `goalcup start`, the agent and the daemon are killed.
    // The disk is kept. All three are frozen first, the daemon before the rest, so none of them
    // sees another go: as at a power cut.
    for pid in [daemon, second.child.id() as i32, agent_pid(&rig)] {
        // SAFETY: two integers; each pid is this test's own or its stand-in's.
        unsafe { libc::kill(pid, libc::SIGSTOP) };
    }
    kill9(second.child.id() as i32);
    kill9(agent_pid(&rig));
    kill9(daemon);
    second.close();
    second.wait();
    wait_until("the daemon gone", || {
        daemons_for(&daemon_config(&rig)).is_empty()
    });
    let before1 = local_chain(&rig, &entry1);
    let time_left = goalcup_cli::run::time_left::file_path(&rig.cfg, &run_id);
    let stale = std::fs::read_to_string(&time_left).ok();
    // Neither segment is on the server whole: both wait here.
    assert!(server.chunks_of(&entry1).is_empty());
    assert_ne!(server_copy(&server, &entry0), local_chain(&rig, &entry0));
    server.fail_next_chunks(0);

    let mut third = rig.continue_on_pty(&[DISK, BEATING], None);
    third.wait_shown("tui: ready");
    assert_ne!(the_daemon(&rig), daemon, "a new daemon");
    assert!(
        harness(&rig, "tui.argv").ends_with("resume\n--last"),
        "{}",
        harness(&rig, "tui.argv")
    );
    third.type_line("three");
    third.wait_shown("tui: noted three");
    let entry2 = server.entry_ids(&run_id)[2].clone();
    wait_recorded(&rig, &entry2, "three");
    // The agent can ask the time left again, and the run ends when it did.
    let deadline_line = format!(
        "deadline_at={}",
        goalcup_protocol::control::format_z(
            goalcup_protocol::control::parse_z(&deadline_at).unwrap()
        )
    );
    wait_until("a fresh time-left file", || {
        std::fs::read_to_string(&time_left)
            .is_ok_and(|now| Some(&now) != stale.as_ref() && now.contains(&deadline_line))
    });

    // The participant seals the part from another terminal.
    let sealed = rig
        .chat_command(
            &[],
            Launcher::Args(vec!["seal".into(), entry2.clone(), "--yes".into()]),
        )
        .output()
        .unwrap();
    assert!(
        sealed.status.success(),
        "{}{}",
        String::from_utf8_lossy(&sealed.stdout),
        String::from_utf8_lossy(&sealed.stderr)
    );
    let status = third.wait();
    let shown = third.shown();
    assert!(status.success(), "{status:?}\n{shown}");
    assert!(
        shown.contains(goalcup_cli::participant::STILL_OPEN),
        "{shown}"
    );

    // One run, one approval, one clone, three segments in order, here and on the server.
    let record = goalcup_cli::run::read(&rig.cfg, &run_id).unwrap();
    let entries: Vec<String> = record.entries.iter().map(|e| e.entry_id.clone()).collect();
    assert_eq!(entries, [entry0.clone(), entry1.clone(), entry2.clone()]);
    assert_eq!(server.entry_ids(&run_id), entries);
    assert_eq!(server.run_ids(), std::slice::from_ref(&run_id));
    assert_eq!(server.device_requests().len(), 1, "one approval");
    assert_eq!(rig.clones(), std::slice::from_ref(&started.workspace));
    assert_eq!(record.workspace, started.workspace);
    // The deadline never moved: the record, and every segment's signed receipt.
    assert_eq!(record.deadline_at.as_deref(), Some(deadline_at.as_str()));
    for entry in &entries {
        let receipt = server.receipt_of(entry).expect("a receipt");
        assert_eq!(receipt.claims.deadline_at, deadline_at, "{entry}");
        assert_eq!(
            receipt.claims.workspace,
            started.workspace.display().to_string()
        );
    }

    // Nothing recorded before a stop was lost or changed: the chain then is the start of the
    // chain now.
    assert!(local_chain(&rig, &entry0).starts_with(&before0));
    assert!(local_chain(&rig, &entry1).starts_with(&before1));

    // Each segment is whole on the server, sealed, signed and ended; the stops are named gaps,
    // and nothing is torn.
    let options = goalcup_verifier::VerifyOptions {
        extra_server_keys: vec![server.public_key_hex()],
        ..Default::default()
    };
    // Segment 0 was sealed by the first continue with the gap its killed host left, segment 1
    // by the second with the gap its killed daemon left (one reopen), and segment 2 is clean.
    for (entry, typed, exit, gap) in [
        (&entry0, "one", 1, vec![GapReason::IngestLost]),
        (&entry1, "two", 1, vec![GapReason::IngestLost]),
        (&entry2, "three", 0, vec![]),
    ] {
        // `wait_until` allows 60 s, but the real limit is the wrap-up in [`BEATING`]
        // (`WrapUp::after`, 10 s): what a part's host has not sent by then it never sends.
        wait_until(&format!("{entry} all sent"), || {
            server.seal_of(entry).is_some()
                && server_copy(&server, entry) == local_chain(&rig, entry)
        });
        assert!(
            server.heartbeats_of(entry).iter().any(|h| h.final_),
            "{entry}: a final heartbeat"
        );
        let mut records = Vec::new();
        let report = goalcup_verifier::capture_analysis::reader::scan_capture_trace(
            &rig.trace(entry),
            &options,
            &mut |record| {
                records.push(record.clone());
                Ok(())
            },
        )
        .unwrap();
        assert!(report.sealed, "{entry}: {:?}", report.findings);
        assert_eq!(report.exit_code(), exit, "{entry}: {:?}", report.findings);
        assert_eq!(gaps(&records), gap, "{entry}");
        assert_eq!(report.anchor, AnchorState::Signed, "{entry}");
        assert!(
            native_bytes(&records).contains(&format!(r#""text":"{typed}""#)),
            "{entry}: what was typed in it"
        );
    }
}

/// The run's clone is gone when the participant continues: a sandbox whose work folder was on a
/// disk that did not survive the stop, or a folder moved away. The continue is refused before
/// anything changes, here or on the server, and names the folder and the seal that ends the last
/// part without it; with the folder back in place the next continue goes on in it. (Before, the
/// missing folder was made again, empty, and the agent went on in it with none of its work.)
#[test]
fn a_continue_whose_clone_is_gone_is_refused_and_changes_nothing() {
    let rig = Rig::chat();
    let server = UploadServer::start(&rig.root.join("server"), ServerOptions::default());
    rig.prepare_for(&server.url());
    let mut first = rig.start_on_pty(&[DISK, BEATING]);
    first.wait_shown("tui: ready");
    first.type_line("one");
    first.wait_shown("tui: noted one");
    first.type_line("/exit");
    assert!(first.wait().success(), "{}", first.shown());
    let run_id = server.run_ids()[0].clone();
    let record = goalcup_cli::run::read(&rig.cfg, &run_id).unwrap();
    let (workspace, last_part) = (record.workspace, record.entries[0].entry_id.clone());
    let away = rig.root.join("moved-away");
    std::fs::rename(&workspace, &away).unwrap();
    let (agent, folders) = (harness(&rig, "tui.pid"), rig.run_dirs());

    let mut refused = rig.continue_on_pty(&[DISK, BEATING], None);
    let status = refused.wait();
    let said = refused.shown();
    assert_eq!(status.code(), Some(1), "{said}");
    assert!(said.contains(&workspace.display().to_string()), "{said}");
    assert!(said.contains("Nothing was changed"), "{said}");
    // The other way on, when the clone is lost: the part already recorded still reaches the
    // server, since a seal needs no clone.
    assert!(
        said.contains(&format!(
            "goalcup seal {last_part} --yes, then goalcup upload"
        )),
        "{said}"
    );
    assert!(!workspace.exists(), "the folder was not made again");
    assert_eq!(
        server.entry_ids(&run_id).len(),
        1,
        "no segment was asked for"
    );
    assert_eq!(harness(&rig, "tui.pid"), agent, "no agent was started");
    assert_eq!(rig.run_dirs(), folders);

    std::fs::rename(&away, &workspace).unwrap();
    let mut again = rig.continue_on_pty(&[DISK, BEATING], None);
    again.wait_shown("tui: ready");
    again.type_line("/exit");
    assert!(again.wait().success(), "{}", again.shown());
    assert_eq!(server.entry_ids(&run_id).len(), 2);
    assert_eq!(rig.clones(), [workspace]);
}
