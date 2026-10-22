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

/// The line the launcher prints for this workload, exactly: the member, which forks its orphan
/// only once it has received SIGTERM and so is alive at the sweep's first pass, and that orphan.
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
