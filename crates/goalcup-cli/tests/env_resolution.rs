//! Every test in this crate that mutates the process environment, and nothing else.
//!
//! # Why this is its own binary
//!
//! `std::env::set_var` is `unsafe` in edition 2024 because the environment is one table
//! shared by every thread in the process and the C library reads it without a lock. Cargo
//! runs the `#[test]`s in one binary on a pool of threads, and `contract_gate.rs` spawns
//! `goalcup` with `Command::output()` — which walks that same table to build the child's
//! environment. A `set_var` on one thread while another is mid-spawn is undefined
//! behaviour, not a flake that shows up as a wrong answer: it is a read of a pointer the
//! writer freed. That is what this file exists to make impossible.
//!
//! Two rules keep it true, and both are load-bearing:
//!
//! 1. **This binary spawns nothing and reads no environment off any other thread.** There
//!    is exactly one `#[test]` below; it calls the cases in order, so no second test thread
//!    exists while a variable is being written.
//! 2. **No other test file in this crate calls `set_var` or `remove_var`.** A test that
//!    needs a made-up environment either belongs here, or — better — calls
//!    `config::config_dir_from`, which takes the lookup as an argument and touches nothing
//!    process-wide. The four platform branches are unit-tested that way in
//!    `src/config.rs`; what is left here is the one thing that route cannot check, which is
//!    that the shipped `config_dir()` really is wired to the real environment.

mod m2_profile_support;

use goalcup_cli::config;

/// The one test. The cases run in order, on this thread, with the environment restored
/// after each — see the module header for why there is not one `#[test]` per case.
#[test]
fn config_dir_reads_the_real_environment() {
    config_dir_from_keeps_the_cli_refusal_at_its_boundary();
    the_override_is_honoured_exactly_as_given();
    an_empty_override_is_unset_and_the_default_is_never_a_harness_home();
    a_capture_launch_takes_nothing_hostile_from_the_real_environment();
}

fn config_dir_from_keeps_the_cli_refusal_at_its_boundary() {
    let error = match config::config_dir_from(|_| None) {
        Ok(path) => panic!("an empty lookup resolved {}", path.display()),
        Err(error) => error.to_string(),
    };
    assert!(error.contains(config::CONFIG_DIR_ENV), "{error:?}");
    assert!(
        error.contains("LOCALAPPDATA") && error.contains("USERPROFILE"),
        "the Windows fallback variables are actionable at the CLI boundary: {error:?}"
    );
}

/// `$GOALCUP_CONFIG_DIR` wins, and the path comes back byte for byte.
///
/// Not canonicalised: on macOS `/var` is a link to `/private/var`, so a caller that handed
/// over a temporary directory and got a different string back would have to know that to
/// compare the two.
fn the_override_is_honoured_exactly_as_given() {
    let tmp = tempfile::tempdir().unwrap();
    let restore = set(config::CONFIG_DIR_ENV, Some(tmp.path().as_os_str()));
    let cfg = config::config_dir().unwrap();
    assert_eq!(
        cfg,
        tmp.path(),
        "${} is what the shipped config_dir() reads",
        config::CONFIG_DIR_ENV
    );
    restore();
}

/// An exported-but-empty `$GOALCUP_CONFIG_DIR` is what a shell script that meant to compute
/// a path and computed nothing leaves behind. Honouring it would resolve the tracker's
/// state to the process's working directory, which during a run is the participant's own
/// repository — so it is unset, and the platform default answers instead.
///
/// And whatever that default is on this machine, it is the tracker's own directory and
/// never a harness's: not `~/.claude`, not `~/.codex`, not `~/.opencode`. That promise is
/// the whole reason this module exists.
fn an_empty_override_is_unset_and_the_default_is_never_a_harness_home() {
    let roots = tempfile::tempdir().unwrap();
    let home = roots.path().join("home");
    let local_app_data = roots.path().join("local-app-data");
    let user_profile = roots.path().join("user-profile");
    let restore_override = set(config::CONFIG_DIR_ENV, Some(std::ffi::OsStr::new("")));
    let restore_home = set("HOME", Some(home.as_os_str()));
    // Removed and not set: on a machine where the developer exports it, leaving it would
    // send the non-macOS default somewhere outside the $HOME this case just pinned.
    let restore_xdg = set("XDG_CONFIG_HOME", None);
    let restore_local_app_data = set("LOCALAPPDATA", Some(local_app_data.as_os_str()));
    let restore_user_profile = set("USERPROFILE", Some(user_profile.as_os_str()));
    let cfg = config::config_dir().unwrap();
    restore_user_profile();
    restore_local_app_data();
    restore_xdg();
    restore_home();
    restore_override();

    let expected_root = if cfg!(target_os = "windows") {
        &local_app_data
    } else {
        &home
    };

    assert!(
        cfg.starts_with(expected_root),
        "an empty override is unset, so the default under the pinned platform root answers: {}",
        cfg.display()
    );
    assert!(
        cfg.ends_with(config::DEFAULT_CONFIG_DIR_NAME),
        "the platform default ends in {}, and it is {}",
        config::DEFAULT_CONFIG_DIR_NAME,
        cfg.display()
    );
    for harness_dir in [".claude", ".codex", ".opencode"] {
        assert!(
            !cfg.iter().any(|part| part == harness_dir),
            "the tracker borrows no harness home, and it resolved to {}",
            cfg.display()
        );
    }
    // Resolving creates nothing: a `goalcup doctor` on a machine where nothing has ever
    // started must not leave the next run a different answer.
    assert!(
        !cfg.exists(),
        "resolving created {} — it resolves and does not create",
        cfg.display()
    );
}

/// The M2 capture launch is handed the tracker's REAL environment (`process_env`, as
/// `goalcup start` reads it) with a hostile proxy, bypass, CA and base URL exported in it.
/// None of them reaches the plan — its environment is built from nothing and its own pins
/// win — and the real environment still holds exactly what was exported: the helper reads
/// it and changes nothing. Nothing is spawned; the "executable" is only hashed.
fn a_capture_launch_takes_nothing_hostile_from_the_real_environment() {
    use goalcup_cli::launch::{CAPTURE_NO_PROXY, build_capture_launch, process_env};
    use m2_profile_support as s;
    let l = s::Layout::new();
    let exe = s::fake_harness(&l.bin);
    let c = s::repinned("codex-cli", &exe);
    let pinned = s::pinned(&c, &exe);
    let hostile = [
        ("HTTPS_PROXY", "http://evil.example:3128"),
        ("NO_PROXY", "*"),
        ("CODEX_CA_CERTIFICATE", "/evil/ca.pem"),
        ("SSL_CERT_FILE", "/evil/roots.pem"),
        ("OPENAI_BASE_URL", "http://evil.example"),
        ("GOALCUP_T14_UNRELATED", "kept out"),
    ];
    let restores: Vec<_> = hostile
        .iter()
        .map(|(k, v)| set(k, Some(std::ffi::OsStr::new(v))))
        .collect();
    let parent = process_env();
    let proxy = s::proxy_url();
    let planned = build_capture_launch(
        &c,
        &s::vars(&l),
        &l.workspace,
        &s::launch(&l, b"goal", &pinned, &proxy, &parent),
    );
    let still: Vec<Option<String>> = hostile.iter().map(|(k, _)| std::env::var(k).ok()).collect();
    for restore in restores.into_iter().rev() {
        restore();
    }
    let p = planned.unwrap();
    for ((k, v), now) in hostile.iter().zip(still) {
        assert_eq!(
            now.as_deref(),
            Some(*v),
            "`{k}` in the real environment changed"
        );
    }
    assert_eq!(p.env_set["HTTPS_PROXY"], proxy);
    assert_eq!(p.env_set["NO_PROXY"], CAPTURE_NO_PROXY);
    assert_eq!(
        p.env_set["CODEX_CA_CERTIFICATE"],
        l.ca.to_str().unwrap(),
        "the launch's CA pin, not the exported one"
    );
    for k in ["SSL_CERT_FILE", "OPENAI_BASE_URL", "GOALCUP_T14_UNRELATED"] {
        assert!(
            !p.env_set.contains_key(k),
            "`{k}` was carried into the capture launch"
        );
    }
}

/// Set `name` to `value` (or remove it), returning the closure that puts it back.
///
/// SAFETY, for both calls below: this binary has one `#[test]`, it runs the cases in order
/// on one thread, and nothing here spawns a child or starts a thread. No other thread can
/// be reading the environment while either call is in flight.
fn set(name: &str, value: Option<&std::ffi::OsStr>) -> impl FnOnce() + use<> {
    let name = name.to_string();
    let previous = std::env::var_os(&name);
    match value {
        Some(value) => unsafe { std::env::set_var(&name, value) },
        None => unsafe { std::env::remove_var(&name) },
    }
    move || match previous {
        Some(previous) => unsafe { std::env::set_var(&name, previous) },
        None => unsafe { std::env::remove_var(&name) },
    }
}
