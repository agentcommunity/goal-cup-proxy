//! The capture spawn gate (D40, CONTRACTS.md C6 capture spawn gate) and the CLI's ending of a child it just released.
//!
//! Real processes, nothing else: a `/bin/sh` script is the "harness", a symlink to `/bin/sleep`
//! is its "native binary", and every assertion reads the process table or a file the script
//! wrote. The gate's own interleavings with a daemon are `m2_cli_lifecycle.rs` (in-process rig)
//! and `capture_crash_campaign.rs` (real binaries); this file pins the two pieces that need no
//! daemon: `spawn_gated`/`release`/`abort`, and `end_released_child`.
//!
//! Tests here run one at a time (the process-table `TableLock`, and the `serial` group under
//! nextest): each forks, and a fork taken while another test
//! holds a freshly written script open is how a "text file busy" is made.

use crate::common::table_lock::TableLock;

use goalcup_cli::CliError;
use goalcup_cli::launch::{
    GatedSpawn, LaunchPlan, ReleasedChildEnd, Spawned, end_released_child, gated_handle,
    launched_at_now, spawn_gated,
};
use goalcup_protocol::manifest::{Kill, KillStrategy, LaunchMode};
use goalcup_protocol::process::{ProcessHandle, is_alive, process_start_identity};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const NATIVE: &str = "m2native";

/// How long a pid or file may take to appear or vanish.
const PATIENCE: Duration = Duration::from_secs(15);

struct World {
    dir: tempfile::TempDir,
    _lock: TableLock,
}

impl World {
    fn new() -> World {
        let lock = TableLock::acquire();
        let dir = tempfile::tempdir().unwrap();
        // The "native binary": `sleep` under another name, which is what a process table calls
        // it. (A copy of a platform binary is killed by code signing; a link is not.)
        std::os::unix::fs::symlink("/bin/sleep", dir.path().join(NATIVE)).unwrap();
        World { dir, _lock: lock }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// A plan that runs `script` under `/bin/sh`, with `$OUT` naming this world's directory and
    /// `$NATIVE` the "native binary".
    fn plan(&self, script: &str, native_binary: Option<&str>, grace: u64) -> LaunchPlan {
        let script_path = self.path("harness.sh");
        std::fs::write(&script_path, script).unwrap();
        let mut env = BTreeMap::new();
        env.insert("PATH".into(), "/usr/bin:/bin".into());
        env.insert("OUT".into(), self.dir.path().display().to_string());
        env.insert("NATIVE".into(), self.path(NATIVE).display().to_string());
        let argv = vec![script_path.display().to_string(), "chat".to_string()];
        LaunchPlan {
            program: "/bin/sh".into(),
            argv,
            env_set: env,
            env_remove: Vec::new(),
            env_base: BTreeMap::new(),
            cwd: self.dir.path().to_path_buf(),
            mode: LaunchMode::Tui,
            kill: Kill {
                strategy: if native_binary.is_some() {
                    KillStrategy::NativePid
                } else {
                    KillStrategy::ProcessGroup
                },
                native_binary: native_binary.map(str::to_string),
                grace_seconds: grace,
            },
        }
    }

    fn gated(&self, script: &str) -> (LaunchPlan, GatedSpawn) {
        let plan = self.plan(script, None, 1);
        let gated = spawn_gated(&plan, PATIENCE).expect("a parked child");
        (plan, gated)
    }

    fn ran(&self) -> bool {
        self.path("ran").exists()
    }
}

fn wait_for(what: &str, mut condition: impl FnMut() -> bool) {
    let until = Instant::now() + PATIENCE;
    while !condition() {
        assert!(Instant::now() < until, "waited {PATIENCE:?} for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn read_pid(path: &Path) -> u32 {
    wait_for(&format!("{}", path.display()), || {
        std::fs::read_to_string(path).is_ok_and(|t| t.trim().parse::<u32>().is_ok())
    });
    std::fs::read_to_string(path)
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

fn command_of(pid: u32) -> String {
    let out = std::process::Command::new("/bin/ps")
        .args(["-ww", "-o", "command=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn kill9(pid: u32) {
    // SAFETY: two integers; every caller passes a child this test spawned and has not reaped.
    assert_eq!(unsafe { libc::kill(pid as i32, libc::SIGKILL) }, 0);
}

const RAN: &str = "echo ran > \"$OUT/ran\"\nexec /bin/sleep 30\n";

// ------------------------------------------------------------------ the gate

#[test]
fn gate_eof_never_execs() {
    let w = World::new();
    let (_, gated) = w.gated(RAN);
    let pid = gated.pid;
    assert!(process_start_identity(pid).is_some(), "the child is parked");
    // The launcher goes away without releasing: its end of the gate closes.
    drop(gated);
    wait_for("the parked child to end at EOF", || {
        process_start_identity(pid).is_none()
    });
    assert!(!w.ran(), "a child that read EOF never exec'd the harness");
}

#[test]
fn gate_abort_reaps_without_exec() {
    let w = World::new();
    let (_, gated) = w.gated(RAN);
    let pid = gated.pid;
    gated.abort();
    assert!(
        process_start_identity(pid).is_none(),
        "abort returns only after the child is ended and reaped"
    );
    assert!(!w.ran());
}

#[test]
fn gate_release_execs_with_stable_pid_and_start_identity() {
    let w = World::new();
    let (_, gated) = w.gated(RAN);
    let pid = gated.pid;
    let parked_start = process_start_identity(pid).expect("a parked identity");
    let this_binary = std::env::current_exe().unwrap();
    let this_binary = this_binary.file_name().unwrap().to_string_lossy();
    assert!(
        command_of(pid).contains(&*this_binary),
        "parked, it is still a copy of this process: {}",
        command_of(pid)
    );
    let mut spawned = gated.release().expect("released");
    assert_eq!(spawned.child.id(), pid, "the registered pid is the harness");
    wait_for("the harness to run", || w.ran());
    // The script writes `ran` just before its own `exec`, so `ran` alone does not say the exec
    // has happened (under load the shell was seen between the two). Wait for the exec itself:
    // the process table naming the harness's command, which is what is asserted.
    let exec_by = Instant::now() + PATIENCE;
    while !command_of(pid).contains("/bin/sleep 30") {
        assert!(
            Instant::now() < exec_by,
            "after the release it is the harness (the script's exec), but after {PATIENCE:?} the \
             process is: {}",
            command_of(pid)
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        process_start_identity(pid).as_deref(),
        Some(parked_start.as_str()),
        "exec does not change the creation identity the daemon was handed"
    );
    assert_eq!(spawned.program, "/bin/sh");
    kill9(pid);
    let _ = spawned.child.wait();
}

#[test]
fn gated_handle_has_no_native_hint() {
    let w = World::new();
    let plan = w.plan(RAN, Some(NATIVE), 7);
    let gated = spawn_gated(&plan, PATIENCE).unwrap();
    let handle = gated_handle(&plan, &gated, &launched_at_now()).unwrap();
    assert_eq!(handle.pid, gated.pid);
    assert_eq!(handle.native_pid, None);
    assert_eq!(handle.native_process_start, None);
    assert_eq!(handle.native_binary.as_deref(), Some(NATIVE));
    assert_eq!(handle.strategy, KillStrategy::NativePid);
    assert_eq!(handle.grace_seconds, 7);
    assert_eq!(handle.program, "/bin/sh");
    assert_eq!(
        handle.process_start,
        process_start_identity(gated.pid),
        "the fork-time identity is registered while parked"
    );
    assert_eq!(handle.argv, plan.argv, "the handle records the plan's argv");
    gated.abort();
}

#[test]
fn parked_child_death_is_never_released() {
    let w = World::new();
    let plan = w.plan(RAN, None, 1);
    // A ready timeout far longer than the assertion below: only EOF or EPIPE can end it early.
    let gated = spawn_gated(&plan, Duration::from_secs(60)).unwrap();
    let pid = gated.pid;
    kill9(pid);
    wait_for("the killed child to die", || !is_alive(pid));
    let began = Instant::now();
    let refused = gated.release().err();
    assert!(
        matches!(refused, Some(CliError::Internal(_))),
        "a dead child is never released: {refused:?}"
    );
    assert!(
        began.elapsed() < Duration::from_secs(5),
        "the answer came from the closed gate, not the 60 s acknowledgement wait: {:?}",
        began.elapsed()
    );
    assert!(!w.ran(), "nothing exec'd");
    assert!(
        process_start_identity(pid).is_none(),
        "the dead child was reaped by the failed release"
    );
}

#[test]
fn pid_read_ends_on_child_death_or_fork_failure() {
    let w = World::new();
    // A set-up failure inside the child (its working directory is missing) ends the spawn
    // thread before any pid is written. That is seen through the thread, not by waiting out the
    // 60 s pid timeout: the launcher still holds the child's end, so there is no EOF.
    let mut plan = w.plan(RAN, None, 1);
    plan.cwd = w.path("no-such-directory");
    let began = Instant::now();
    let refused = spawn_gated(&plan, Duration::from_secs(60)).err();
    assert!(
        matches!(refused, Some(CliError::Io { .. })),
        "the spawn failure is the answer: {refused:?}"
    );
    assert!(
        began.elapsed() < Duration::from_secs(5),
        "{:?}",
        began.elapsed()
    );
    assert!(!w.ran());

    // A death after the pid is seen as EOF, because the launcher dropped the child's end once
    // the pid arrived; with the end retained the release would wait out its whole 60 s.
    let plan = w.plan(RAN, None, 1);
    let gated = spawn_gated(&plan, Duration::from_secs(60)).unwrap();
    kill9(gated.pid);
    wait_for("the killed child to die", || !is_alive(gated.pid));
    let began = Instant::now();
    assert!(gated.release().is_err());
    assert!(
        began.elapsed() < Duration::from_secs(5),
        "{:?}",
        began.elapsed()
    );
}

// ------------------------------------------------------------------ ending a released child

/// A released gated child and its handle, registered the way `capture()` does.
fn released(w: &World, script: &str, native: Option<&str>, grace: u64) -> (ProcessHandle, Spawned) {
    let plan = w.plan(script, native, grace);
    let gated = spawn_gated(&plan, PATIENCE).unwrap();
    let handle = gated_handle(&plan, &gated, &launched_at_now()).unwrap();
    let spawned = gated.release().unwrap();
    (handle, spawned)
}

#[test]
fn end_released_child_re_resolves_native_descendant() {
    let w = World::new();
    // The wrapper is not started far enough to have forked its native binary when the child is
    // released: it ignores SIGTERM until it has, then forks the native binary (default
    // SIGTERM), and waits on it. Ending it must find the native child on a later poll, not only
    // in a first snapshot, or the native child outlives the root.
    let script = "trap '' TERM\n\
                  touch \"$OUT/ready\"\n\
                  sleep 0.8\n\
                  trap - TERM\n\
                  \"$NATIVE\" 300 &\n\
                  echo $! > \"$OUT/native.pid\"\n\
                  wait\n";
    let (handle, spawned) = released(&w, script, Some(NATIVE), 5);
    let root = spawned.child.id();
    wait_for("the wrapper to be running", || w.path("ready").exists());
    let began = Instant::now();
    let end = end_released_child(spawned.child, &handle);
    let took = began.elapsed();
    let native = read_pid(&w.path("native.pid"));
    match &end {
        ReleasedChildEnd::Ended { termed, killed } => {
            assert_eq!(termed, &vec![root, native], "{end:?}");
            assert!(killed.is_empty(), "{end:?}");
        }
        other => panic!("{other:?}"),
    }
    assert!(!is_alive(native), "the native child was found and ended");
    assert!(!is_alive(root));
    assert!(
        took < Duration::from_secs(5),
        "ended as soon as the native child appeared, not at the grace: {took:?}"
    );
}

#[test]
fn end_released_child_signals_an_already_running_native_child_first() {
    let w = World::new();
    let script = "\"$NATIVE\" 300 &\n\
                  echo $! > \"$OUT/native.pid\"\n\
                  wait\n";
    let (handle, spawned) = released(&w, script, Some(NATIVE), 5);
    let root = spawned.child.id();
    let native = read_pid(&w.path("native.pid"));
    let end = end_released_child(spawned.child, &handle);
    match &end {
        ReleasedChildEnd::Ended { termed, killed } => {
            assert_eq!(
                termed,
                &vec![native, root],
                "the native child first: {end:?}"
            );
            assert!(killed.is_empty(), "{end:?}");
        }
        other => panic!("{other:?}"),
    }
    assert!(!is_alive(native));
    assert!(!is_alive(root));
}

#[test]
fn end_released_child_native_never_seen_is_native_unverified() {
    let w = World::new();
    // The connector names a native binary; this wrapper never starts one. The root ends, but
    // nothing proves there was no native child, so the answer is never `Ended`.
    let (handle, spawned) = released(&w, "exec /bin/sleep 300\n", Some(NATIVE), 5);
    let root = spawned.child.id();
    let end = end_released_child(spawned.child, &handle);
    assert_eq!(
        end,
        ReleasedChildEnd::NativeUnverified {
            termed: vec![root],
            killed: vec![]
        }
    );
    assert!(!is_alive(root));
}

#[test]
fn end_released_child_on_exited_root_signals_nothing() {
    let w = World::new();
    let (handle, spawned) = released(&w, "exit 0\n", None, 5);
    let root = spawned.child.id();
    wait_for("the harness to exit by itself", || !is_alive(root));
    assert_eq!(
        end_released_child(spawned.child, &handle),
        ReleasedChildEnd::AlreadyGone
    );
}

#[test]
fn end_released_child_kills_a_term_ignoring_root_after_exactly_the_grace() {
    let w = World::new();
    let script = "trap '' TERM\ntouch \"$OUT/ready\"\nwhile :; do sleep 1; done\n";
    let (handle, spawned) = released(&w, script, None, 2);
    let root = spawned.child.id();
    wait_for("the harness to ignore TERM", || w.path("ready").exists());
    let began = Instant::now();
    let end = end_released_child(spawned.child, &handle);
    let took = began.elapsed();
    assert_eq!(
        end,
        ReleasedChildEnd::Ended {
            termed: vec![root],
            killed: vec![root]
        }
    );
    assert!(
        took >= Duration::from_secs(2),
        "the grace is never cut: {took:?}"
    );
    assert!(
        took < goalcup_protocol::process::termination_bound(2),
        "and the whole ending is inside the termination bound: {took:?}"
    );
    assert!(!is_alive(root));
}

// ------------------------------------------------------------------ what the launcher says

#[test]
fn the_refusal_never_says_nothing_is_running_unless_the_outcome_does() {
    let ended = ReleasedChildEnd::Ended {
        termed: vec![7],
        killed: vec![],
    };
    let gone = ReleasedChildEnd::AlreadyGone;
    let unverified = ReleasedChildEnd::NativeUnverified {
        termed: vec![7],
        killed: vec![],
    };
    let still = ReleasedChildEnd::StillRunning { pids: vec![7] };
    for (end, clean) in [
        (&ended, true),
        (&gone, true),
        (&unverified, false),
        (&still, false),
    ] {
        let refusal = end.refusal("e1");
        let announcement = end.announcement("e1");
        assert!(
            refusal.contains("sealed before its launch could be recorded"),
            "{refusal}"
        );
        assert_eq!(
            announcement.contains("ended the harness it had just started"),
            clean,
            "{announcement}"
        );
        assert_eq!(
            refusal.contains("no longer running") || refusal.contains("already exited"),
            clean,
            "{refusal}"
        );
        assert!(
            !refusal.contains("nothing is running"),
            "the old unconditional claim is gone: {refusal}"
        );
    }
    // The may-be-running cases say so, and point at the outcome line above.
    assert!(
        unverified.refusal("e1").contains("may still be running")
            && unverified.refusal("e1").contains("see the line above"),
        "{}",
        unverified.refusal("e1")
    );
    assert!(
        unverified
            .announcement("e1")
            .contains("tried to end the harness"),
        "{}",
        unverified.announcement("e1")
    );
    assert!(still.refusal("e1").contains("STILL RUNNING"));
}

#[test]
fn end_released_child_on_exited_root_with_a_named_native_binary_is_unverified() {
    let w = World::new();
    // The wrapper left by itself; a native child it forked may be reparented and unfindable.
    let (handle, spawned) = released(&w, "exit 0\n", Some(NATIVE), 5);
    let root = spawned.child.id();
    wait_for("the harness to exit by itself", || !is_alive(root));
    assert_eq!(
        end_released_child(spawned.child, &handle),
        ReleasedChildEnd::NativeUnverified {
            termed: vec![],
            killed: vec![]
        }
    );
}

#[test]
fn a_parked_child_that_died_is_refused_in_its_own_words() {
    let w = World::new();
    let plan = w.plan(RAN, None, 1);
    let gated = spawn_gated(&plan, PATIENCE).unwrap();
    kill9(gated.pid);
    wait_for("the killed child to die", || !is_alive(gated.pid));
    let refused = gated_handle(&plan, &gated, &launched_at_now())
        .expect_err("a dead parked child has no identity to register");
    let text = refused.to_string();
    assert!(
        matches!(refused, CliError::Internal(_)) && text.contains("never ran"),
        "{text}"
    );
    assert!(!text.contains("numeric termination authority"), "{text}");
    gated.abort();
}

/// b4a249eb's ordering, pinned by source because the behaviour is timing-shaped: the child's
/// creation identity is read before anything that takes time. A harness that exits at once is a
/// zombie within milliseconds and has no identity to read.
#[test]
fn the_creation_identity_is_read_before_the_native_probe() {
    let source = include_str!("../../src/launch.rs");
    let (_, rest) = source
        .split_once("\nfn build_handle(")
        .expect("build_handle");
    let body = rest.split("\n}\n").next().expect("its body");
    let identity = body
        .find("process_start_identity(pid)")
        .expect("build_handle reads the creation identity");
    let probe = body
        .find("native()")
        .expect("and then runs the native probe");
    assert!(identity < probe, "the identity must come first");
    // And the probe is handed in lazily by `handle`, never evaluated before the call.
    assert!(source.contains("|| native_identity_of(p, pid)"));
}
