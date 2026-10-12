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

#[test]
#[cfg_attr(
    not(target_os = "linux"),
    ignore = "Linux only: the launcher is a child subreaper there; on macOS the kqueue witness covers this shape"
)]
fn the_launcher_sweeps_an_orphan_forked_after_the_root_died_before_the_seal() {
    let _lock = TableLock::acquire();
    let v = Vertical::new("orphan-after-root", json!({}));
    let mut start = v.start(RESERVE);
    let entry = v.entry_id();
    let exit = start.wait(Duration::from_secs(60));
    let output = start.output();
    let orphan: u32 = std::fs::read_to_string(v.out.join("orphan.pid"))
        .unwrap_or_else(|e| panic!("the member forked no orphan ({e})\n{output}"))
        .trim()
        .parse()
        .unwrap();
    let _owned = Orphan(
        orphan,
        goalcup_protocol::process::process_start_identity(orphan),
    );
    assert!(exit.success(), "{output}");
    assert!(
        !goalcup_protocol::process::is_alive(orphan),
        "orphan {orphan}, forked after the root died, outlived `goalcup start`\n{output}"
    );
    let swept = output
        .lines()
        .position(|line| {
            line.starts_with("swept ")
                && line.ends_with(" process(es) the harness left behind after it exited.")
        })
        .unwrap_or_else(|| panic!("no sweep line\n{output}"));
    let sealed = output
        .lines()
        .position(|line| line.contains(": finished locally;"))
        .unwrap_or_else(|| panic!("no local finish\n{output}"));
    assert!(swept < sealed, "the sweep precedes the seal\n{output}");
    let (code, integrity) = v.verify(&entry);
    assert_eq!(code, 0, "{integrity}");
}
