//! On Linux `goalcup start` is a child subreaper and sweeps what the harness left behind after
//! it exited, before the seal (TERMINATION.md, "RULED: on Linux, `Confirmed` excludes
//! fork-and-leave wrappers"). The daemon's own walk cannot see such an orphan: the root is reaped
//! before the seal, so the daemon answers `AlreadyStopped` without walking, and the orphan was
//! never under a live root it could snapshot.
//!
//! The harness is the controlled workload in its `orphan-after-root` mode: the root starts a
//! member and exits, and only after the root's death does the member fork a detached
//! `sleep 300` and exit. The real `goalcup` binary runs it against the real daemon, as
//! `m2_synthetic_vertical` does.
//!
//! macOS has no child subreaper, and its kqueue witness covers this shape in the daemon, so the
//! test is ignored there.

use crate::common::m2_vertical_support;
use crate::common::table_lock;

use m2_vertical_support::{RESERVE, Vertical};
use serde_json::json;
use std::time::Duration;
use table_lock::TableLock;

/// The orphan this test made, ended when the test ends however it ends: only while its pid still
/// has the start identity read when the test first saw it, so a reused pid is never signalled.
struct Orphan(u32, Option<String>);

impl Drop for Orphan {
    fn drop(&mut self) {
        if self.1.is_some() && goalcup_protocol::process::process_start_identity(self.0) == self.1 {
            // SAFETY: two integers; the pid still has the start identity this test recorded.
            unsafe { libc::kill(self.0 as i32, libc::SIGKILL) };
        }
    }
}

/// The line the launcher prints for this workload: the member and its orphan, both alive when the
/// sweep first saw them (the member lingers 200 ms or more after the root's death).
const SWEPT_TWO: &str = "swept 2 process(es) the harness left behind after it exited.";

/// Run `v`'s `orphan-after-root` workload to its end under the real `goalcup start`, and hold the
/// launcher to the sweep: the orphan is gone when it returns, the sweep line precedes the seal's
/// report, and the test's own daemon is untouched. Returns the entry and what it printed.
fn swept_before_the_seal(v: &Vertical) -> (String, String) {
    let mut start = v.start(RESERVE);
    let exit = start.wait(Duration::from_secs(60));
    let output = start.output();
    assert!(exit.success(), "{output}");
    let entry = v.entry_id();
    let orphan: u32 = std::fs::read_to_string(v.out.join("orphan.pid"))
        .unwrap_or_else(|e| panic!("the member forked no orphan ({e})\n{output}"))
        .trim()
        .parse()
        .unwrap();
    let _owned = Orphan(
        orphan,
        goalcup_protocol::process::process_start_identity(orphan),
    );
    assert!(
        !goalcup_protocol::process::is_alive(orphan),
        "orphan {orphan}, forked after the root died, outlived `goalcup start`\n{output}"
    );
    assert!(
        goalcup_protocol::process::is_alive(v.daemon_pid() as u32),
        "the daemon did not survive the sweep\n{output}"
    );
    assert!(
        !output.contains("could not read its own children"),
        "{output}"
    );
    let swept = output
        .lines()
        .position(|line| line == SWEPT_TWO)
        .unwrap_or_else(|| panic!("no `{SWEPT_TWO}`\n{output}"));
    let sealed = output
        .lines()
        .position(|line| line.contains(": finished locally;"))
        .unwrap_or_else(|| panic!("no local finish\n{output}"));
    assert!(swept < sealed, "the sweep precedes the seal\n{output}");
    (entry, output)
}

#[test]
#[cfg_attr(
    not(target_os = "linux"),
    ignore = "Linux only: the launcher is a child subreaper there; on macOS the kqueue witness covers this shape"
)]
fn the_launcher_sweeps_an_orphan_forked_after_the_root_died_before_the_seal() {
    let _lock = TableLock::acquire();
    let v = Vertical::new("orphan-after-root", json!({}));
    let (entry, _) = swept_before_the_seal(&v);
    let (code, integrity) = v.verify(&entry);
    assert_eq!(code, 0, "{integrity}");
}

/// The same orphan under a `stdout_json` identity (Codex's shape), with a root that exits before
/// any scope line while the member holds its stdout. The launcher's scope wait sees the root exit
/// and reaps it before the launch is recorded; the sweep was armed before that wait, so it still
/// knows the root's start and sweeps.
#[test]
#[cfg_attr(
    not(target_os = "linux"),
    ignore = "Linux only: the launcher is a child subreaper there; on macOS the kqueue witness covers this shape"
)]
fn a_root_reaped_by_the_scope_wait_still_has_its_orphans_swept() {
    let _lock = TableLock::acquire();
    let v = Vertical::new("orphan-after-root", json!({"member_holds_stdout": true}));
    let manifest = v
        .connectors
        .join(m2_vertical_support::CONNECTOR)
        .join("harness.toml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    let none = "[capture.identity]\nsource = \"none\"\n";
    assert!(
        text.contains(none),
        "the workload connector's identity section moved"
    );
    let pattern = "pattern = \"native/*.jsonl\"";
    assert!(
        text.contains(pattern),
        "the workload connector's native store moved"
    );
    // A session-scoped identity needs a session-scoped native store; this run writes none.
    std::fs::write(
        &manifest,
        text.replace(
            none,
            "[capture.identity]\nsource = \"stdout_json\"\nbinding = \"at_launch\"\n\
             match_field = \"event.kind\"\nmatch_value = \"session.started\"\n\
             id_field = \"session.id\"\n",
        )
        .replace(pattern, "pattern = \"native/{id}.jsonl\""),
    )
    .unwrap();
    let (entry, output) = swept_before_the_seal(&v);
    let (_, integrity) = v.verify(&entry);
    assert_eq!(integrity["sealed"], true, "{integrity}\n{output}");
}
