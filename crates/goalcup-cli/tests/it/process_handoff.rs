//! D22: what the CLI hands the daemon after a launch, and the one thing it never does with
//! the process it launched.

use crate::common::support;

use crate::common::table_lock;

use goalcup_protocol::process::{descendants, find_named, snapshot};

#[test]
fn discovery_finds_the_grandchild_the_npm_wrapper_hides() {
    // M0 §2.5: `kill -9` on the wrapper left the native binary holding the thread-writer
    // lock, and the next resume silently became a new thread.
    //
    // The pid file is a path inside a temporary DIRECTORY rather than a `NamedTempFile`,
    // because `support::wait_for_file` waits for a path to exist and a `NamedTempFile`
    // exists from the moment it is created — the wait would return on an empty file and the
    // parse below would be a race this test lost on a busy machine.
    let _lock = table_lock::TableLock::acquire();
    let tmp = tempfile::tempdir().unwrap();
    let pidfile = tmp.path().join("grandchild.pid");
    let mut child = std::process::Command::new("sh")
        .arg(support::fixture("wrapper-fork.sh"))
        .arg(&pidfile)
        .spawn()
        .unwrap();
    support::wait_for_file(&pidfile);
    let grandchild: u32 = std::fs::read_to_string(&pidfile)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let table = snapshot().unwrap();
    assert!(descendants(child.id(), &table).contains(&grandchild));
    assert_eq!(find_named(child.id(), "sleep", &table), Some(grandchild));
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn this_crate_never_terminates_anything() {
    // The one invariant whose violation is silent: two killers with different grace
    // periods racing at the deadline. The kill is `goalcup_daemon::terminate`.
    //
    // `support::walk` takes a path relative to the repository root, so the crate's own source
    // directory is spelled out.
    //
    // The signal check carries five exemptions, and each is written down rather than
    // glossed.
    //
    // `preflight/probe.rs`: the version probe spawns the harness with `--version` in a group
    // of its own and `killpg`s that group at its own ten-second deadline. That child is the
    // probe's, it is dead before `start` has decided anything, and it is not the launched
    // harness — which is the process this rule is about.
    //
    // `commands/start.rs`: the abandon path signals the group of a harness the daemon was
    // never successfully told about, on a start that is failing. Nothing will measure that
    // process, nothing will seal what it writes, and the workspace it is running in is about
    // to be deleted — `commands::start::launch`'s own header is where that reasoning lives.
    // It is not a deadline, it has no grace period and it reads no clock: the terminator this
    // rule is about is `goalcup_daemon::terminate`, at 168 hours, and a second one racing it
    // is the failure whose symptom is silence.
    //
    // `released_child.rs` (H-10 4b, the root ruling under D40): when a seal raced in between
    // a capture child's durable registration and its release, the launcher ends the child it
    // has just released, with a grace and a SIGKILL, so that no harness runs on a sealed entry.
    // It is the one exemption that reads a clock, which is why it is its own file and the
    // exemption is by name: it signals only this process's own unreaped `Child` and that
    // child's native descendant bound by creation identity, never anything recorded, and
    // `goalcup_daemon::terminate` stays the only terminator of a recorded run.
    //
    // `subreaper.rs` (Linux only): `goalcup start` is a child subreaper, and after its root's
    // `wait` has returned and before the seal it sweeps what the harness left behind, which the
    // kernel reparented to it: SIGTERM, the connector's grace, SIGKILL, each one reaped. It
    // signals only this process's own unreaped children that started no earlier than the root
    // and are not the daemon it started. The sweep is printed, never chained. In a run that ends
    // on its own the root is reaped first, so the daemon's termination answers `AlreadyStopped`
    // and walks nothing. A standalone `goalcup seal` while the harness runs is the one overlap:
    // the daemon is mid-grace when the root dies, the members reparent to the launcher, and the
    // sweep signals the same processes. That overlap is safe: both escalate on the handle's
    // `grace_seconds`, the sweep's clock starts no earlier than the daemon's first SIGTERM, the
    // daemon signals through pidfds, and the sweep signals only its own unreaped children, so
    // neither can reach a recycled pid and nothing is SIGKILLed before the daemon's own grace.
    //
    // `harness_version.rs` (D53): the version probe, `<program> <version_args>`, runs once
    // before the harness is spawned and is never a recorded run: it is not registered, carries
    // no goal and no key, and leads a process group of its own. When it outlives its timeout
    // the launcher SIGKILLs that group while its own unreaped leader still holds the group id,
    // and reaps the leader, so it cannot reach a recycled pid or anything the daemon terminates.
    // The connector's `capture.configure` commands (D61) are ended by the same helper
    // (`wait_or_end_group`), for the same reasons: each runs the harness's own program once
    // before the launch, unregistered, with no goal and no key, in a group of its own.
    //
    // The `start.rs` exemption stays ONE file: a second file that signals a launched harness
    // would have to be exempted here as well, by name.
    //
    // A NEW file that signals is still caught, which is the whole point of the sweep.
    //
    // The sweep asserts what it walked before it asserts anything about the contents:
    // `support::walk` answers an empty vector for a directory that is not there, and a
    // `for` over nothing passes every assertion inside it. A rename that moved this crate's
    // source would otherwise turn this test green and silent.
    let sources = support::walk("crates/goalcup-cli/src");
    assert!(
        !sources.is_empty(),
        "the sweep walked no file at all: `crates/goalcup-cli/src` is not where this \
         crate's source lives any more"
    );
    assert!(
        sources
            .iter()
            .any(|p| p.file_name().and_then(|n| n.to_str()) == Some("launch.rs")),
        "the sweep did not reach `launch.rs`, which is the file that spawns the harness: \
         {sources:?}"
    );
    for entry in sources {
        let s = std::fs::read_to_string(&entry).unwrap();
        assert!(!s.contains("/bin/kill"), "{entry:?} signals a process");
        assert!(
            !s.contains("fn terminate"),
            "{entry:?} defines a terminator"
        );
        // The five files that signal, and the reason each one is allowed to. Only the probe half
        // of `preflight/{mod,gates,probe}.rs` kills anything, so the exemption is matched on the
        // tail of the path, and `preflight/gates.rs` is NOT exempt — a `SIGKILL` appearing there
        // is the sweep doing its job.
        let exempt = [
            "preflight/probe.rs",
            "commands/start.rs",
            "released_child.rs",
            "src/subreaper.rs",
            "src/harness_version.rs",
        ]
        .iter()
        .any(|tail| entry.ends_with(tail));
        if exempt {
            continue;
        }
        assert!(
            !s.contains("SIGKILL") && !s.contains("SIGTERM"),
            "{entry:?}"
        );
    }
}
