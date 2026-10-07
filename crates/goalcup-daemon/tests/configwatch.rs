//! D6 and D21: the bytes of every file the tracker wrote, the hash a
//! shim pins into itself and reports on every post, and the `config_allowlist[]` of
//! app-wide files the tracker did not write.
//!
//! The two halves are deliberately asymmetric and every test here turns on that asymmetry:
//! a tracker file is put back (D6, "watched, not locked"), an app-wide file is reported and
//! left exactly as the participant left it (D21, AGENTS.md).
#![cfg(feature = "v1-compat")]

use goalcup_daemon::configwatch::ConfigWatch;
use goalcup_daemon::workspace::{ABSENT, AllowlistedFile, sha256_hex};

mod support;

/// Both halves in one poll, so the asymmetry is asserted against one
/// `ConfigWatch` rather than against two that could have been built differently.
#[test]
fn a_tracker_file_is_restored_and_an_app_wide_file_is_only_reported() {
    let t = tempfile::tempdir().unwrap();
    let tracked = support::tracker_file(t.path(), ".codex/hooks.json", b"{\"a\":1}", true);
    let mine = t.path().join("home/.claude/settings.json");
    support::write(&mine, b"{\"model\":\"opus\"}");
    let allow = AllowlistedFile {
        path: mine.clone(),
        relative_path: "{home}/.claude/settings.json".to_string(),
        sha256: sha256_hex(b"{\"model\":\"opus\"}"),
        mtime_unix: Some(1.0),
        existed: true,
    };
    let mut w = ConfigWatch::new(std::slice::from_ref(&tracked), &[allow]);
    assert!(w.poll().unwrap().is_empty());

    support::write(&tracked.absolute_path, b"{\"a\":2}");
    support::write(&mine, b"{\"model\":\"sonnet\"}");
    let d = w.poll().unwrap();
    assert_eq!(d.len(), 2);
    let tracker = d.iter().find(|d| d.path.ends_with("hooks.json")).unwrap();
    assert!(
        tracker.restored,
        "D6: watched, not locked — put back and say so"
    );
    assert!(
        tracker.arming_lost,
        "restoring a Codex hook file invalidates its trust hash"
    );
    assert_eq!(std::fs::read(&tracked.absolute_path).unwrap(), b"{\"a\":1}");
    assert_eq!(tracker.expected_sha256, tracked.sha256);
    assert_eq!(tracker.observed_sha256, sha256_hex(b"{\"a\":2}"));
    assert!(!tracker.detected_at.is_empty());

    let app_wide = d
        .iter()
        .find(|d| d.path.ends_with("settings.json"))
        .unwrap();
    assert!(
        !app_wide.restored,
        "Global Constraint 4: never write outside our own trees"
    );
    assert!(!app_wide.arming_lost);
    assert_eq!(std::fs::read(&mine).unwrap(), b"{\"model\":\"sonnet\"}");
}

/// A deleted tracker file is a deviation with the absent digest, and it comes back —
/// including the directory the deletion took with it.
#[test]
fn a_deleted_tracker_file_is_a_deviation_and_is_restored() {
    let t = tempfile::tempdir().unwrap();
    let f = support::tracker_file(t.path(), ".codex/hooks.json", b"{\"hooks\":[]}", true);
    let mut w = ConfigWatch::new(std::slice::from_ref(&f), &[]);
    assert!(w.poll().unwrap().is_empty());

    std::fs::remove_dir_all(t.path().join(".codex")).unwrap();
    let d = w.poll().unwrap();
    assert_eq!(d.len(), 1);
    assert_eq!(
        d[0].observed_sha256, ABSENT,
        "an absent file has no hash; zeros say so"
    );
    assert!(d[0].restored);
    assert!(d[0].arming_lost);
    assert_eq!(std::fs::read(&f.absolute_path).unwrap(), b"{\"hooks\":[]}");
    assert!(
        w.poll().unwrap().is_empty(),
        "a restored file deviates once, not on every poll after it"
    );
}

/// The Codex shim's own bytes are covered by no pinned digest anywhere — it pins its
/// peer's hash, and a file cannot carry its own — so the watch has to hold the shim as a
/// watched file in its own right. `restore_invalidates_trust = false` there: putting the
/// shim back does not touch the hash Codex trusts, which is `hooks.json`'s.
#[test]
fn the_shim_beside_the_hook_file_is_itself_watched_and_restored() {
    let t = tempfile::tempdir().unwrap();
    let peer = support::tracker_file(t.path(), ".codex/hooks.json", b"{\"hooks\":[]}", true);
    let shim = support::tracker_file(
        t.path(),
        ".codex/hooks/hook-post.sh",
        b"#!/bin/sh\ntrue\n",
        false,
    );
    let mut w = ConfigWatch::new(&[peer, shim.clone()], &[]);
    assert_eq!(w.watched().len(), 2);
    assert!(w.poll().unwrap().is_empty());

    support::write(&shim.absolute_path, b"#!/bin/sh\nexit 0\n");
    let d = w.poll().unwrap();
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].path, ".codex/hooks/hook-post.sh");
    assert!(d[0].restored);
    assert!(
        !d[0].arming_lost,
        "the shim's own bytes are covered by no trust hash"
    );
    assert_eq!(
        std::fs::read(&shim.absolute_path).unwrap(),
        b"#!/bin/sh\ntrue\n"
    );
}

/// `harness_owned`, both halves in one poll: the harness's own file is not on the watch at
/// all, and the hook file beside it is watched exactly as before.
///
/// The codex `config.toml` is harness-owned: Codex writes
/// `[tui.model_availability_nux]` into its config on every launch, so a watched copy would
/// be restored, rewritten and chained once per launch for the life of the connector — and a
/// chained `ConfigDeviation` no traced write explains is an `unattributed_config_change`
/// Fail at V9 (`crates/goalcup-verifier/tests/checks_v6_v11.rs`,
/// `a_removed_codex_hook_file_is_still_an_unattributed_config_change`, holds that half).
/// Taking the file off the watch must not take anything else off it, which is what the
/// second half here measures: the hook file is rewritten in the same poll and still
/// deviates, is still restored, and still loses arming.
#[test]
fn a_harness_owned_file_is_off_the_watch_and_the_hook_file_beside_it_is_not() {
    let t = tempfile::tempdir().unwrap();
    let hooks = support::tracker_file(t.path(), ".codex/hooks.json", b"{\"hooks\":[]}", true);
    let seeded = b"check_for_update_on_startup = false\n";
    let config_path = t.path().join("home/config.toml");
    support::write(&config_path, seeded);
    let config = goalcup_daemon::workspace::TrackerFile {
        relative_path: "config.toml".to_string(),
        absolute_path: config_path.clone(),
        scope: goalcup_protocol::manifest::InstallScope::HarnessHome,
        sha256: sha256_hex(seeded),
        contents: String::from_utf8(seeded.to_vec()).unwrap(),
        mode: goalcup_daemon::workspace::DEFAULT_MODE,
        restore_invalidates_trust: false,
        harness_owned: true,
    };
    let mut w = ConfigWatch::new(&[hooks.clone(), config], &[]);
    assert_eq!(
        w.watched().len(),
        1,
        "the harness's own file is not a watched file"
    );
    assert!(w.poll().unwrap().is_empty());

    // What Codex does on its first launch, and what an agent does to the hooks.
    let after = b"check_for_update_on_startup = false\n\n[tui.model_availability_nux]\ngpt = 1\n";
    support::write(&config_path, after);
    support::write(&hooks.absolute_path, b"{}");

    let d = w.poll().unwrap();
    assert_eq!(d.len(), 1, "one deviation, and it is not the config: {d:?}");
    assert_eq!(d[0].path, ".codex/hooks.json");
    assert!(d[0].restored);
    assert!(d[0].arming_lost);
    assert_eq!(
        std::fs::read(&hooks.absolute_path).unwrap(),
        b"{\"hooks\":[]}"
    );
    assert_eq!(
        std::fs::read(&config_path).unwrap(),
        after,
        "never watched means never written back"
    );
    assert!(
        w.poll().unwrap().is_empty(),
        "and the counter Codex wrote is still not a deviation on the poll after it"
    );
}

/// D21: an app-wide file is reported once per change and
/// not once per poll for the rest of the run, and it is never written back.
#[test]
fn an_app_wide_file_is_reported_once_per_change_and_never_written_back() {
    let t = tempfile::tempdir().unwrap();
    let mine = t.path().join("home/.claude/settings.json");
    support::write(&mine, b"{\"model\":\"opus\"}\n");
    let baseline = AllowlistedFile {
        path: mine.clone(),
        relative_path: "{home}/.claude/settings.json".to_string(),
        sha256: sha256_hex(b"{\"model\":\"opus\"}\n"),
        mtime_unix: Some(1_788_421_320.0),
        existed: true,
    };
    let mut w = ConfigWatch::new(&[], std::slice::from_ref(&baseline));
    assert!(w.poll().unwrap().is_empty(), "unchanged, so nothing to say");

    let edited = b"{\"model\":\"opus\",\"hooks\":{}}\n";
    support::write(&mine, edited);
    let d = w.poll().unwrap();
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].expected_sha256, baseline.sha256);
    assert_eq!(d[0].observed_sha256, sha256_hex(edited));
    assert!(!d[0].restored);
    assert_eq!(
        std::fs::read(&mine).unwrap(),
        edited,
        "the participant's file is EXACTLY as they left it"
    );

    assert!(w.poll().unwrap().is_empty(), "one event per change");
    support::write(&mine, b"{}\n");
    let again = w.poll().unwrap();
    assert_eq!(again.len(), 1, "and again on the next change");
    assert_eq!(
        again[0].expected_sha256, baseline.sha256,
        "the launch baseline is what a deviation is measured against, still"
    );
}

/// `~/.claude/settings.local.json` not existing at launch and existing an hour later is a
/// participant adding app-wide hooks the tracker never wrote — the case this clause is for.
#[test]
fn an_allowlisted_file_that_appears_mid_run_is_reported_and_not_deleted_again() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("home/.claude/settings.local.json");
    let baseline = AllowlistedFile {
        path: path.clone(),
        relative_path: "{home}/.claude/settings.local.json".to_string(),
        sha256: ABSENT.to_string(),
        mtime_unix: None,
        existed: false,
    };
    let mut w = ConfigWatch::new(&[], std::slice::from_ref(&baseline));
    assert!(
        w.poll().unwrap().is_empty(),
        "still absent, still nothing to say"
    );

    support::write(&path, b"{\"hooks\":{\"UserPromptSubmit\":[]}}\n");
    let d = w.poll().unwrap();
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].expected_sha256, ABSENT);
    assert!(!d[0].restored, "and it is NOT deleted again");
    assert!(path.is_file());
}

/// The write side of Global Constraint 4, as a property of the vector rather than as a
/// promise in a comment: `restore` is true for exactly the files the tracker wrote, whose
/// absolute paths are a scope root joined with a manifest-declared relative path, and false
/// for every allowlisted file — which is the only kind that can sit anywhere at all.
#[test]
fn only_files_the_tracker_wrote_are_ever_written_back() {
    let t = tempfile::tempdir().unwrap();
    let tracked = support::tracker_file(t.path(), ".codex/hooks.json", b"{}", true);
    let outside = t.path().join("elsewhere/etc/passwd");
    support::write(&outside, b"root\n");
    let allow = AllowlistedFile {
        path: outside.clone(),
        relative_path: "{home}/etc/passwd".to_string(),
        sha256: sha256_hex(b"root\n"),
        mtime_unix: None,
        existed: true,
    };
    let w = ConfigWatch::new(std::slice::from_ref(&tracked), &[allow]);
    for f in w.watched() {
        assert_eq!(
            f.restore,
            f.path == tracked.absolute_path,
            "{} is restorable only if the tracker wrote it",
            f.path.display()
        );
        assert!(
            f.restore || f.expected_bytes.is_empty(),
            "a file that is never restored carries no bytes to restore it with"
        );
        assert!(
            !f.restore_invalidates_trust || f.restore,
            "the two flags can never both apply to one file"
        );
    }
}

/// A restore after a deletion puts back the file the tracker installed, permission bits
/// and all. The Codex shim is 0o755, and recreating it at the default mode leaves a shim
/// the harness cannot execute — capture stopping in silence, which is the failure D6 chose
/// restoring over locking to avoid.
#[cfg(unix)]
#[test]
fn a_restored_shim_is_still_executable() {
    use std::os::unix::fs::PermissionsExt;
    let t = tempfile::tempdir().unwrap();
    let shim = support::tracker_file(t.path(), ".codex/hooks/hook-post.sh", b"#!/bin/sh\n", false);
    std::fs::set_permissions(&shim.absolute_path, std::fs::Permissions::from_mode(0o755)).unwrap();

    let mut w = ConfigWatch::new(std::slice::from_ref(&shim), &[]);
    assert!(w.poll().unwrap().is_empty());
    std::fs::remove_file(&shim.absolute_path).unwrap();
    assert_eq!(w.poll().unwrap().len(), 1);

    let mode = std::fs::metadata(&shim.absolute_path)
        .unwrap()
        .permissions()
        .mode()
        & 0o7777;
    assert_eq!(mode, 0o755, "a shim Codex cannot execute captures nothing");
}

// ---------------------------------------------------------------------------------------
// Restore safety and what a deviation may carry.
// ---------------------------------------------------------------------------------------

/// `fs::write` follows a symlink, so a
/// tracker path replaced by a link to a file outside the three roots would have the daemon
/// writing the shim's bytes — receiver token and all — into the participant's `~/.zshrc`.
/// The link is the deviation; the target is never touched.
#[cfg(unix)]
#[test]
fn a_tracker_path_replaced_by_a_symlink_is_never_written_through() {
    let t = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let victim = outside.path().join(".zshrc");
    std::fs::write(&victim, b"export PATH=/usr/bin\n").unwrap();

    let f = support::tracker_file(t.path(), ".codex/hooks.json", b"{\"hooks\":[]}", true);
    let mut w = ConfigWatch::new(std::slice::from_ref(&f), &[]);
    assert!(w.poll().unwrap().is_empty());

    std::fs::remove_file(&f.absolute_path).unwrap();
    std::os::unix::fs::symlink(&victim, &f.absolute_path).unwrap();

    let d = w.poll().unwrap();
    assert_eq!(d.len(), 1);
    assert!(d[0].restored);
    assert_eq!(
        std::fs::read(&victim).unwrap(),
        b"export PATH=/usr/bin\n",
        "a write must never leave the workspace, the artifact root and the harness home"
    );
    assert!(
        !std::fs::symlink_metadata(&f.absolute_path)
            .unwrap()
            .file_type()
            .is_symlink(),
        "the link is replaced by the real file"
    );
    assert_eq!(std::fs::read(&f.absolute_path).unwrap(), b"{\"hooks\":[]}");
}

/// A link to nothing is the same escape one `touch` away: `fs::write` through a dangling
/// link creates the target. The link goes, the file comes back, the target never appears.
#[cfg(unix)]
#[test]
fn a_dangling_symlink_in_a_tracker_path_is_replaced_by_the_real_file() {
    let t = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let never = outside.path().join("never-created");

    let f = support::tracker_file(t.path(), ".codex/hooks.json", b"{\"hooks\":[]}", true);
    let mut w = ConfigWatch::new(std::slice::from_ref(&f), &[]);
    assert!(w.poll().unwrap().is_empty());

    std::fs::remove_file(&f.absolute_path).unwrap();
    std::os::unix::fs::symlink(&never, &f.absolute_path).unwrap();

    let d = w.poll().unwrap();
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].observed_sha256, ABSENT);
    assert!(d[0].restored);
    assert!(
        !never.exists(),
        "a dangling link is not a licence to create"
    );
    assert_eq!(std::fs::read(&f.absolute_path).unwrap(), b"{\"hooks\":[]}");
}

/// A directory where a tracker file belongs makes every write fail forever. It is inside
/// the entry's own roots by construction, so it is removed and the file put back.
#[test]
fn a_directory_where_a_tracker_file_belongs_is_removed_and_the_file_restored() {
    let t = tempfile::tempdir().unwrap();
    let f = support::tracker_file(t.path(), ".codex/hooks.json", b"{\"hooks\":[]}", true);
    let mut w = ConfigWatch::new(std::slice::from_ref(&f), &[]);
    assert!(w.poll().unwrap().is_empty());

    std::fs::remove_file(&f.absolute_path).unwrap();
    std::fs::create_dir_all(f.absolute_path.join("nested")).unwrap();
    std::fs::write(f.absolute_path.join("nested/x"), b"x").unwrap();

    let d = w.poll().unwrap();
    assert_eq!(d.len(), 1);
    assert!(d[0].restored);
    assert!(f.absolute_path.is_file());
    assert_eq!(std::fs::read(&f.absolute_path).unwrap(), b"{\"hooks\":[]}");
}

/// A restore that cannot be performed is a deviation with `restored = false`, not an error
/// that aborts the poll. An aborted poll discards every
/// deviation already collected and blinds the watch for the rest of the run.
#[cfg(unix)]
#[test]
fn a_restore_that_cannot_be_written_is_a_deviation_and_the_poll_keeps_going() {
    use std::os::unix::fs::PermissionsExt;
    let t = tempfile::tempdir().unwrap();
    let stuck = support::tracker_file(t.path(), "locked/hooks.json", b"{\"a\":1}", false);
    let ok = support::tracker_file(t.path(), ".codex/hooks.json", b"{\"b\":2}", true);
    let mut w = ConfigWatch::new(&[stuck.clone(), ok.clone()], &[]);
    assert!(w.poll().unwrap().is_empty());

    std::fs::remove_file(&stuck.absolute_path).unwrap();
    let locked = t.path().join("locked");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
    support::write(&ok.absolute_path, b"{\"b\":3}");

    let d = w.poll().unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(
        d.len(),
        2,
        "one un-restorable file does not blind the watch"
    );
    let failed = d
        .iter()
        .find(|d| d.path.ends_with("locked/hooks.json"))
        .unwrap();
    assert!(!failed.restored, "say so rather than claim a restore");
    assert!(
        !failed.arming_lost,
        "nothing was restored, so nothing was lost"
    );
    let restored = d.iter().find(|d| d.path == ".codex/hooks.json").unwrap();
    assert!(restored.restored);
    assert_eq!(
        std::fs::read(&ok.absolute_path).unwrap(),
        b"{\"b\":2}",
        "the files after the failure are still watched and still restored"
    );
    assert_eq!(w.restore_failures(), 1, "degraded, and countably so");
}

/// `ConfigDeviation.diff` is never filled: the Codex shim's content
/// is `TOKEN="{receiver_token}"`. `ChainWriter::event` does not redact, so a diff is a
/// direct path from a config file's bytes into the chain.
#[test]
fn a_chained_deviation_never_carries_the_bytes_of_a_file() {
    let t = tempfile::tempdir().unwrap();
    let shim = support::tracker_file(
        t.path(),
        ".codex/hooks/hook-post.sh",
        b"#!/bin/sh\nTOKEN=\"s3cr3t-receiver-token\"\n",
        false,
    );
    let mine = t.path().join("home/.claude/settings.json");
    support::write(&mine, b"{\"model\":\"opus\"}\n");
    let allow = AllowlistedFile {
        path: mine.clone(),
        relative_path: "{home}/.claude/settings.json".to_string(),
        sha256: sha256_hex(b"{\"model\":\"opus\"}\n"),
        mtime_unix: None,
        existed: true,
    };
    let mut w = ConfigWatch::new(std::slice::from_ref(&shim), &[allow]);
    assert!(w.poll().unwrap().is_empty());

    support::write(&shim.absolute_path, b"#!/bin/sh\n");
    support::write(&mine, b"{\"model\":\"sonnet\"}\n");
    let d = w.poll().unwrap();
    assert_eq!(d.len(), 2);
    for one in &d {
        assert!(
            one.diff.is_empty(),
            "a deviation carries digests, never bytes: {:?}",
            one.diff
        );
    }
}

/// `chmod -x hook-post.sh` stops capture and changes no byte, so a
/// digest comparison alone is silent about it. The mode a file had at construction is part
/// of what the daemon installed.
#[cfg(unix)]
#[test]
fn a_tracker_file_that_lost_its_executable_bit_is_a_deviation_and_is_restored() {
    use std::os::unix::fs::PermissionsExt;
    let t = tempfile::tempdir().unwrap();
    let shim = support::tracker_file(t.path(), ".codex/hooks/hook-post.sh", b"#!/bin/sh\n", false);
    std::fs::set_permissions(&shim.absolute_path, std::fs::Permissions::from_mode(0o755)).unwrap();

    let mut w = ConfigWatch::new(std::slice::from_ref(&shim), &[]);
    assert!(w.poll().unwrap().is_empty());

    std::fs::set_permissions(&shim.absolute_path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let d = w.poll().unwrap();
    assert_eq!(d.len(), 1, "a shim that cannot run captures nothing");
    assert_eq!(
        d[0].observed_sha256, d[0].expected_sha256,
        "not one byte changed; the bits did"
    );
    assert!(d[0].restored);
    let mode = std::fs::metadata(&shim.absolute_path)
        .unwrap()
        .permissions()
        .mode()
        & 0o7777;
    assert_eq!(mode, 0o755);
    assert!(w.poll().unwrap().is_empty(), "one event per change");
}

/// An allowlisted file chained by its absolute path would put the
/// participant's real home directory in the chain. It is chained relative to the token its
/// `config_allowlist[]` entry is rooted at, the way a tracker file is chained relative to
/// its scope root.
#[test]
fn an_allowlisted_file_is_chained_relative_to_its_root_token() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let workspace = t.path().join("ws");
    let settings = home.join(".claude/settings.json");
    support::write(&settings, b"{\"model\":\"opus\"}\n");

    let manifest = support::manifest("claude-code-cli");
    let vars = support::path_vars(&workspace, &home);
    let allow = goalcup_daemon::workspace::baseline_allowlist(&manifest, &vars).unwrap();

    let mut w = ConfigWatch::new(&[], &allow);
    assert!(w.poll().unwrap().is_empty());

    support::write(&settings, b"{\"model\":\"sonnet\"}\n");
    let d = w.poll().unwrap();
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].path, "{home}/.claude/settings.json");
    assert!(
        !d[0].path.contains(&t.path().to_string_lossy().to_string()),
        "a chained path names a token and not a participant's disk: {}",
        d[0].path
    );
}

/// A symlinked ancestor redirects a write as surely as a symlinked leaf does:
/// `symlink_metadata` does not follow the final component, but the kernel follows every one
/// before it, so `.codex` turned into a link to a directory outside the roots would have
/// put the shim's bytes there. The link goes; the outside directory keeps its files.
#[cfg(unix)]
#[test]
fn a_symlinked_parent_directory_does_not_carry_a_restore_out_of_the_roots() {
    let t = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let elsewhere = outside.path().join("etc");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::write(elsewhere.join("theirs"), b"theirs\n").unwrap();

    let f = support::tracker_file(t.path(), ".codex/hooks.json", b"{\"hooks\":[]}", true);
    let mut w = ConfigWatch::new(std::slice::from_ref(&f), &[]);
    assert!(w.poll().unwrap().is_empty());

    std::fs::remove_dir_all(t.path().join(".codex")).unwrap();
    std::os::unix::fs::symlink(&elsewhere, t.path().join(".codex")).unwrap();

    let d = w.poll().unwrap();
    assert_eq!(d.len(), 1);
    assert!(d[0].restored);
    assert!(
        !elsewhere.join("hooks.json").exists(),
        "nothing was written through the linked directory"
    );
    assert_eq!(
        std::fs::read(elsewhere.join("theirs")).unwrap(),
        b"theirs\n"
    );
    assert!(t.path().join(".codex").is_dir());
    assert!(
        !std::fs::symlink_metadata(t.path().join(".codex"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(std::fs::read(&f.absolute_path).unwrap(), b"{\"hooks\":[]}");
}

/// A check-then-write restore is not enough: an ancestor can be exchanged after the
/// pathname walk and before `create_dir_all`/open resolves it. The synchronization seam
/// makes that narrow race an ordinary, reproducible control-flow path.
#[cfg(unix)]
#[test]
fn an_ancestor_swapped_after_validation_cannot_receive_restored_bytes() {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::mpsc;
    use std::time::Duration;

    let t = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let redirected = outside.path().join("redirected");
    std::fs::create_dir_all(&redirected).unwrap();
    let sentinel = redirected.join("sentinel");
    std::fs::write(&sentinel, b"outside bytes\n").unwrap();
    std::fs::set_permissions(&sentinel, std::fs::Permissions::from_mode(0o640)).unwrap();
    let listing_before: Vec<_> = std::fs::read_dir(&redirected)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();

    let f = support::tracker_file(t.path(), ".codex/hooks.json", b"{\"hooks\":[]}", true);
    let mut w = ConfigWatch::new(std::slice::from_ref(&f), &[]);
    assert!(w.poll().unwrap().is_empty());
    std::fs::remove_file(&f.absolute_path).unwrap();

    let (reached_tx, reached_rx) = mpsc::sync_channel(0);
    let (resume_tx, resume_rx) = mpsc::sync_channel(0);
    ConfigWatch::pause_next_restore(f.absolute_path.clone(), reached_tx, resume_rx);
    let poll = std::thread::spawn(move || w.poll().unwrap());
    reached_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("restore validated its original ancestor");
    std::fs::rename(t.path().join(".codex"), t.path().join("saved-codex")).unwrap();
    std::os::unix::fs::symlink(&redirected, t.path().join(".codex")).unwrap();
    resume_tx.send(()).unwrap();
    let deviations = poll.join().unwrap();

    assert_eq!(deviations.len(), 1);
    assert!(
        !deviations[0].restored,
        "a swapped ancestor is a visible failed restore"
    );
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"outside bytes\n");
    assert_eq!(
        std::fs::metadata(&sentinel).unwrap().permissions().mode() & 0o7777,
        0o640,
        "outside permissions are unchanged too"
    );
    let listing_after: Vec<_> = std::fs::read_dir(&redirected)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(listing_after, listing_before, "nothing appeared outside");
}

/// Two independent watches can reach the deterministic restore point concurrently, and
/// registering the second must not drop the first.
#[cfg(unix)]
#[test]
fn two_path_keyed_restore_pauses_can_be_registered_together() {
    use std::sync::mpsc;
    use std::time::Duration;

    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let first_file =
        support::tracker_file(first.path(), ".codex/hooks.json", b"{\"first\":true}", true);
    let second_file = support::tracker_file(
        second.path(),
        ".codex/hooks.json",
        b"{\"second\":true}",
        true,
    );
    let mut first_watch = ConfigWatch::new(std::slice::from_ref(&first_file), &[]);
    let mut second_watch = ConfigWatch::new(std::slice::from_ref(&second_file), &[]);
    assert!(first_watch.poll().unwrap().is_empty());
    assert!(second_watch.poll().unwrap().is_empty());
    std::fs::remove_file(&first_file.absolute_path).unwrap();
    std::fs::remove_file(&second_file.absolute_path).unwrap();

    let (first_reached_tx, first_reached_rx) = mpsc::sync_channel(0);
    let (first_resume_tx, first_resume_rx) = mpsc::sync_channel(0);
    let (second_reached_tx, second_reached_rx) = mpsc::sync_channel(0);
    let (second_resume_tx, second_resume_rx) = mpsc::sync_channel(0);
    ConfigWatch::pause_next_restore(
        first_file.absolute_path.clone(),
        first_reached_tx,
        first_resume_rx,
    );
    ConfigWatch::pause_next_restore(
        second_file.absolute_path.clone(),
        second_reached_tx,
        second_resume_rx,
    );

    let first_poll = std::thread::spawn(move || first_watch.poll().unwrap());
    let second_poll = std::thread::spawn(move || second_watch.poll().unwrap());
    let first_reached = first_reached_rx.recv_timeout(Duration::from_secs(2));
    let second_reached = second_reached_rx.recv_timeout(Duration::from_secs(2));
    let _ = first_resume_tx.send(());
    let _ = second_resume_tx.send(());
    let first_deviations = first_poll.join().unwrap();
    let second_deviations = second_poll.join().unwrap();

    assert!(
        first_reached.is_ok(),
        "the first path-keyed registration was dropped: {first_reached:?}"
    );
    assert!(
        second_reached.is_ok(),
        "the second path-keyed registration was dropped: {second_reached:?}"
    );
    assert!(first_deviations[0].restored);
    assert!(second_deviations[0].restored);
}

/// A transient open failure is not evidence that an ancestor is an obstruction. The old
/// recovery branch recursively removed the directory and all participant data below it before
/// recreating the tracker path. This drives the real poll/restore path with only the syscall
/// result injected.
#[cfg(unix)]
#[test]
fn an_unexpected_ancestor_open_error_preserves_the_existing_subtree() {
    let t = tempfile::tempdir().unwrap();
    let f = support::tracker_file(t.path(), ".codex/hooks.json", b"{\"hooks\":[]}", true);
    let participant_file = t.path().join(".codex/project/keep.txt");
    support::write(&participant_file, b"participant data\n");
    let mut watch = ConfigWatch::new(std::slice::from_ref(&f), &[]);
    assert!(watch.poll().unwrap().is_empty());
    std::fs::remove_file(&f.absolute_path).unwrap();

    ConfigWatch::fail_next_restore_ancestor_open(
        f.absolute_path.clone(),
        std::io::ErrorKind::Interrupted,
    );
    let deviations = watch.poll().unwrap();

    assert!(
        participant_file.exists(),
        "an unclassified ancestor-open error recursively deleted unrelated data"
    );
    assert_eq!(
        std::fs::read(&participant_file).unwrap(),
        b"participant data\n"
    );
    assert_eq!(deviations.len(), 1);
    assert!(
        !deviations[0].restored,
        "the failed restore remains visible instead of replacing an existing directory"
    );
    assert_eq!(watch.restore_failures(), 1);
    assert!(!f.absolute_path.exists());
}

/// Replacing a hard link must replace only the tracker directory entry. The linked outside
/// inode keeps both its bytes and permissions while the watcher restores a fresh local file.
#[cfg(unix)]
#[test]
fn a_hard_linked_tracker_leaf_does_not_mutate_the_outside_file_on_restore() {
    use std::os::unix::fs::PermissionsExt;

    let t = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let sentinel = outside.path().join("sentinel");
    std::fs::write(&sentinel, b"outside bytes\n").unwrap();
    std::fs::set_permissions(&sentinel, std::fs::Permissions::from_mode(0o640)).unwrap();
    let f = support::tracker_file(t.path(), ".codex/hooks.json", b"{\"hooks\":[]}", true);
    let mut w = ConfigWatch::new(std::slice::from_ref(&f), &[]);
    assert!(w.poll().unwrap().is_empty());
    std::fs::remove_file(&f.absolute_path).unwrap();
    std::fs::hard_link(&sentinel, &f.absolute_path).unwrap();

    let deviations = w.poll().unwrap();
    assert_eq!(deviations.len(), 1);
    assert!(deviations[0].restored);
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"outside bytes\n");
    assert_eq!(
        std::fs::metadata(&sentinel).unwrap().permissions().mode() & 0o7777,
        0o640
    );
    assert_eq!(std::fs::read(&f.absolute_path).unwrap(), b"{\"hooks\":[]}");
}

/// A mode-only deviation also replaces its pinned directory entry rather than chmodding an
/// inode that a participant linked into the tracker tree from elsewhere.
#[cfg(unix)]
#[test]
fn a_hard_linked_tracker_leaf_does_not_chmod_the_outside_file() {
    use std::os::unix::fs::PermissionsExt;

    let t = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let expected = b"#!/bin/sh\n";
    let sentinel = outside.path().join("sentinel.sh");
    std::fs::write(&sentinel, expected).unwrap();
    std::fs::set_permissions(&sentinel, std::fs::Permissions::from_mode(0o644)).unwrap();
    let f = support::tracker_file(t.path(), ".codex/hooks/hook-post.sh", expected, false);
    std::fs::set_permissions(&f.absolute_path, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut w = ConfigWatch::new(std::slice::from_ref(&f), &[]);
    assert!(w.poll().unwrap().is_empty());
    std::fs::remove_file(&f.absolute_path).unwrap();
    std::fs::hard_link(&sentinel, &f.absolute_path).unwrap();

    let deviations = w.poll().unwrap();
    assert_eq!(deviations.len(), 1);
    assert!(deviations[0].restored);
    assert_eq!(std::fs::read(&sentinel).unwrap(), expected);
    assert_eq!(
        std::fs::metadata(&sentinel).unwrap().permissions().mode() & 0o7777,
        0o644
    );
    assert_eq!(
        std::fs::metadata(&f.absolute_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o755
    );
}

/// A FIFO must be treated as a deviation, never opened through `File::open` where a writer
/// can make the watcher block forever. Restore removes it through the pinned parent and puts
/// the expected regular file back.
#[cfg(unix)]
#[test]
fn a_tracker_fifo_is_refused_without_blocking_and_replaced() {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let t = tempfile::tempdir().unwrap();
    let f = support::tracker_file(t.path(), ".codex/hooks.json", b"{\"hooks\":[]}", true);
    let mut w = ConfigWatch::new(std::slice::from_ref(&f), &[]);
    assert!(w.poll().unwrap().is_empty());
    std::fs::remove_file(&f.absolute_path).unwrap();
    let fifo = CString::new(f.absolute_path.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);

    let deviations = w.poll().unwrap();
    assert_eq!(deviations.len(), 1);
    assert!(deviations[0].restored);
    assert!(std::fs::metadata(&f.absolute_path).unwrap().is_file());
    assert_eq!(std::fs::read(&f.absolute_path).unwrap(), b"{\"hooks\":[]}");
}

/// App-wide allowlist observations have the same untrusted-leaf contract as tracker files:
/// they report a replaced leaf but never follow it, block on it, or write it back. A later
/// healthy path still has to be observed in the same poll.
#[cfg(unix)]
#[test]
fn app_wide_allowlist_leaves_are_nonblocking_nofollow_bounded_and_report_later_paths() {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::FileTypeExt;
    use std::sync::mpsc;
    use std::time::Duration;

    let temp = tempfile::tempdir().unwrap();
    let fifo = temp.path().join("fifo.json");
    let fifo_link = temp.path().join("fifo-link.json");
    let oversized = temp.path().join("oversized.json");
    let later = temp.path().join("later.json");
    for path in [&fifo, &fifo_link, &oversized, &later] {
        support::write(path, b"expected");
    }
    let allowlisted = [&fifo, &fifo_link, &oversized, &later]
        .into_iter()
        .map(|path| AllowlistedFile {
            path: path.clone(),
            relative_path: format!("{{home}}/{}", path.file_name().unwrap().to_string_lossy()),
            sha256: sha256_hex(b"expected"),
            mtime_unix: Some(1.0),
            existed: true,
        })
        .collect::<Vec<_>>();
    let mut watch = ConfigWatch::new(&[], &allowlisted);
    assert!(watch.poll().unwrap().is_empty());

    std::fs::remove_file(&fifo).unwrap();
    let fifo_name = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo_name.as_ptr(), 0o600) }, 0);
    std::fs::remove_file(&fifo_link).unwrap();
    std::os::unix::fs::symlink(&fifo, &fifo_link).unwrap();
    std::fs::write(
        &oversized,
        vec![b'x'; goalcup_daemon::MAX_FILE_BYTES as usize + 1],
    )
    .unwrap();
    support::write(&later, b"changed later");

    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || tx.send(watch.poll().unwrap()).unwrap());
    let deviations = rx
        .recv_timeout(Duration::from_secs(2))
        .expect("special allowlist leaves must not block polling");
    assert_eq!(deviations.len(), 4);
    assert!(deviations.iter().all(|deviation| !deviation.restored));
    assert!(
        deviations
            .iter()
            .any(|deviation| deviation.path.ends_with("later.json"))
    );
    assert!(
        std::fs::symlink_metadata(&fifo)
            .unwrap()
            .file_type()
            .is_fifo()
    );
    assert!(
        std::fs::symlink_metadata(&fifo_link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

/// Mode-only repair has the same ancestor authority as a byte restore. It must not chmod a
/// regular outside file after the watched directory has been renamed and replaced.
#[cfg(unix)]
#[test]
fn an_ancestor_swap_cannot_redirect_mode_only_repair() {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::mpsc;
    use std::time::Duration;

    let t = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let redirected = outside.path().join("redirected");
    std::fs::create_dir_all(&redirected).unwrap();
    let outside_hook = redirected.join("hook-post.sh");
    std::fs::write(&outside_hook, b"outside hook\n").unwrap();
    std::fs::set_permissions(&outside_hook, std::fs::Permissions::from_mode(0o640)).unwrap();

    let f = support::tracker_file(t.path(), ".codex/hooks/hook-post.sh", b"#!/bin/sh\n", false);
    std::fs::set_permissions(&f.absolute_path, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut w = ConfigWatch::new(std::slice::from_ref(&f), &[]);
    assert!(w.poll().unwrap().is_empty());
    std::fs::set_permissions(&f.absolute_path, std::fs::Permissions::from_mode(0o644)).unwrap();

    let (reached_tx, reached_rx) = mpsc::sync_channel(0);
    let (resume_tx, resume_rx) = mpsc::sync_channel(0);
    ConfigWatch::pause_next_restore(f.absolute_path.clone(), reached_tx, resume_rx);
    let poll = std::thread::spawn(move || w.poll().unwrap());
    reached_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    std::fs::rename(t.path().join(".codex"), t.path().join("saved-codex")).unwrap();
    std::os::unix::fs::symlink(&redirected, t.path().join(".codex")).unwrap();
    resume_tx.send(()).unwrap();
    let deviations = poll.join().unwrap();

    assert_eq!(deviations.len(), 1);
    assert!(!deviations[0].restored);
    assert_eq!(std::fs::read(&outside_hook).unwrap(), b"outside hook\n");
    assert_eq!(
        std::fs::metadata(&outside_hook)
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o640
    );
}

/// A `relative_path` that begins with `/` names no scope root, and the watch says so
/// rather than inventing one.
///
/// `render_tracker_files` builds every `absolute_path` as `root.join(relative_path)`, and
/// `Path::join` with an absolute argument throws the root away: the file then sits wherever
/// the leading slash pointed, and popping the relative components back off the result
/// returns a directory that was never a scope root of this entry. That directory is what
/// `clear_the_way` would then be entitled to remove things inside, outside the three roots
/// this crate writes to. With no root it removes nothing and refuses anything that
/// is not already a plain file, which is the safe reading of a path this daemon cannot place.
#[test]
fn a_relative_path_with_a_leading_slash_names_no_scope_root() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("hooks.json");
    support::write(&path, b"{}");
    let absolute = goalcup_daemon::workspace::TrackerFile {
        relative_path: "/hooks.json".to_string(),
        absolute_path: path,
        scope: goalcup_protocol::manifest::InstallScope::Workspace,
        sha256: sha256_hex(b"{}"),
        contents: "{}".to_string(),
        mode: goalcup_daemon::workspace::DEFAULT_MODE,
        restore_invalidates_trust: false,
        harness_owned: false,
    };
    let mut w = ConfigWatch::new(&[absolute], &[]);
    assert_eq!(
        w.watched()[0].restore_root,
        None,
        "an absolute path is not a path relative to anything"
    );
    let deviations = w.poll().unwrap();
    assert_eq!(deviations.len(), 1);
    assert!(
        !deviations[0].restored,
        "an infallible constructor reports unavailable restore authority on poll"
    );
    assert_eq!(std::fs::read(t.path().join("hooks.json")).unwrap(), b"{}");

    // …and the ordinary spelling still divides its absolute path exactly.
    let ordinary = support::tracker_file(t.path(), ".codex/hooks.json", b"{}", false);
    let w = ConfigWatch::new(&[ordinary], &[]);
    assert_eq!(
        w.watched()[0].restore_root.as_deref(),
        Some(t.path()),
        "two components popped off leaves the root the tracker wrote into"
    );

    for relative_path in [".codex/./hooks.json", ".codex/../hooks.json"] {
        let malformed = goalcup_daemon::workspace::TrackerFile {
            relative_path: relative_path.to_string(),
            absolute_path: t.path().join(".codex/hooks.json"),
            scope: goalcup_protocol::manifest::InstallScope::Workspace,
            sha256: sha256_hex(b"{}"),
            contents: "{}".to_string(),
            mode: goalcup_daemon::workspace::DEFAULT_MODE,
            restore_invalidates_trust: false,
            harness_owned: false,
        };
        assert!(
            ConfigWatch::new(&[malformed], &[]).watched()[0]
                .restore_root
                .is_none(),
            "{relative_path:?} is not an exact suffix of its absolute path"
        );
    }
}
