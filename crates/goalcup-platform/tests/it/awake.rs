use goalcup_platform::awake::{SleepLock, hold, hold_with};
use std::path::PathBuf;

fn alive(pid: u32) -> bool {
    // SAFETY: signal 0 only probes for the existence of the process.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

/// The process state letter, or `None` when no such process exists. A zombie reads as `Z`.
fn state(pid: u32) -> Option<String> {
    if cfg!(target_os = "linux") {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // The command name sits in parentheses and may hold spaces; the state follows the last one.
        let after = &stat[stat.rfind(')')? + 1..];
        after.split_whitespace().next().map(str::to_string)
    } else {
        let out = std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (!text.is_empty()).then_some(text)
    }
}

/// Alive and not a zombie (`kill(pid, 0)` also succeeds for an unreaped zombie).
fn running(pid: u32) -> bool {
    state(pid).is_some_and(|s| !s.starts_with('Z'))
}

fn false_tool() -> PathBuf {
    ["/usr/bin/false", "/bin/false"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
        .expect("a `false` binary at /usr/bin or /bin")
}

#[test]
fn a_sleep_lock_dies_with_its_value() {
    let lock: SleepLock = match hold("goalcup awake test") {
        Ok(lock) => lock,
        Err(why) => {
            // Only a machine with no inhibitor (or a WSL host) may decline.
            if cfg!(target_os = "macos") {
                panic!("macOS ships caffeinate, hold must succeed: {why}");
            }
            return;
        }
    };
    let pid = lock.pid();
    assert!(alive(pid), "the inhibitor must be running while held");
    drop(lock);
    assert!(
        !alive(pid),
        "the inhibitor must be killed and reaped on drop"
    );
}

#[test]
fn no_inhibitor_is_an_err_not_a_panic() {
    let none = |_: &str| -> Option<PathBuf> { None };
    let err = match hold_with("why", &none, false) {
        Ok(_) => panic!("nothing to hold with must be an Err"),
        Err(e) => e,
    };
    assert_eq!(err, "no sleep inhibitor on this machine");

    let wsl = match hold_with("why", &none, true) {
        Ok(_) => panic!("WSL must be an Err"),
        Err(e) => e,
    };
    assert_eq!(
        wsl,
        "the Windows host decides when this machine sleeps; set its power settings"
    );
}

#[test]
fn an_inhibitor_that_exits_at_once_is_an_err() {
    let tool = false_tool();
    let fake = move |_: &str| -> Option<PathBuf> { Some(tool.clone()) };
    let err = match hold_with("why", &fake, false) {
        Ok(_) => panic!("a tool that exits at once must not count as a hold"),
        Err(e) => e,
    };
    assert!(
        err.starts_with("the sleep inhibitor exited at once"),
        "{err}"
    );
}

#[test]
fn a_lock_made_on_a_short_lived_thread_outlives_it() {
    let made = std::thread::spawn(|| hold("goalcup awake thread test"))
        .join()
        .unwrap();
    let lock = match made {
        Ok(lock) => lock,
        Err(why) => {
            if cfg!(target_os = "macos") {
                panic!("macOS ships caffeinate, hold must succeed: {why}");
            }
            eprintln!("skipped: no sleep inhibitor on this host ({why})");
            return;
        }
    };
    let pid = lock.pid();
    // The spawning thread is gone; a signal tied to it would have left a zombie by now.
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(
        running(pid),
        "the inhibitor must still be running (state {:?}) after the thread that asked ended",
        state(pid)
    );
    drop(lock);
    assert!(
        state(pid).is_none(),
        "the inhibitor must be gone and reaped after the drop"
    );
}
