use goalcup_platform::awake::{SleepLock, hold, hold_with};
use std::path::PathBuf;

fn alive(pid: u32) -> bool {
    // SAFETY: signal 0 only probes for the existence of the process.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
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
