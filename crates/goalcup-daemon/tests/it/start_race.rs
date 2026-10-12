//! Two `goalcup-daemon`s started at once on a fresh artifact root: the loser refuses at the
//! registry lock, by that sentence, and never with "No such file or directory".
//!
//! Before the fix the loser's `openat(O_CREAT)` of `registry.redb` could fail with `ENOENT`
//! on APFS while the winner created the same name (8 of 10 hand-run races). The CLI only
//! waits out the lock sentence, so that exit looked like a broken configuration.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// The sentence `Registry::from_file` refuses with when another process holds the lock.
const LOCK: &str = "another daemon holds the registry";

/// Fresh roots per test run. One race lost to `ENOENT` was the common case before the fix,
/// so a handful of rounds makes a regression all but certain to show.
const ROUNDS: usize = 5;

fn start(config: &Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_goalcup-daemon"))
        .arg("--config")
        .arg(config)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the daemon binary runs")
}

fn stop(child: &mut Child) {
    // SAFETY: two integers; the pid is this test's own child, not yet reaped.
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    let _ = child.wait();
}

#[test]
fn the_loser_of_a_start_race_on_a_fresh_root_refuses_at_the_registry_lock() {
    let connectors = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../connectors");
    for round in 0..ROUNDS {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let config = root.join("daemon.toml");
        std::fs::write(
            &config,
            format!(
                "artifact_root    = {:?}\nconnectors_dir   = {:?}\nconfig_dir       = {:?}\n\
                 workspace_parent = {:?}\ncontrol_addr     = \"127.0.0.1:0\"\n",
                root.join("art"),
                connectors,
                root.join("cfg"),
                root.join("runs"),
            ),
        )
        .unwrap();
        assert!(
            !root.join("art").exists(),
            "the artifact root starts absent"
        );

        let mut pair = [start(&config), start(&config)];
        // The loser exits on its own; the winner runs until it is stopped.
        let began = Instant::now();
        let loser = loop {
            if let Some(i) = (0..2).find(|&i| pair[i].try_wait().unwrap().is_some()) {
                break i;
            }
            if began.elapsed() > Duration::from_secs(20) {
                pair.iter_mut().for_each(stop);
                panic!("round {round}: neither daemon refused within 20 s");
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let winner = 1 - loser;
        // Give the other a moment: two exits would mean neither won.
        std::thread::sleep(Duration::from_millis(300));
        let winner_alive = pair[winner].try_wait().unwrap().is_none();
        stop(&mut pair[winner]);

        let mut said = String::new();
        use std::io::Read as _;
        pair[loser]
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut said)
            .unwrap();
        let status = pair[loser].wait().unwrap();
        assert_eq!(
            status.code(),
            Some(1),
            "round {round}: the loser refuses: {said}"
        );
        assert!(
            !said.contains("No such file or directory"),
            "round {round}: the loser met ENOENT, not the lock: {said}"
        );
        assert!(
            said.contains(LOCK),
            "round {round}: the loser names the registry lock: {said}"
        );
        assert!(
            winner_alive,
            "round {round}: the other daemon is the winner"
        );
    }
}
