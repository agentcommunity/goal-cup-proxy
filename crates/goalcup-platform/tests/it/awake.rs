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

/// A directory holding a fake `systemd-inhibit`, removed on drop.
#[cfg(target_os = "linux")]
struct FakeInhibitor(PathBuf);

#[cfg(target_os = "linux")]
impl FakeInhibitor {
    /// Write `body` as an executable `#!/bin/sh` script named `systemd-inhibit`.
    fn new(body: &str) -> Self {
        use std::io::Write as _;
        use std::os::unix::fs::PermissionsExt as _;
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "goalcup-awake-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let tool = dir.join("systemd-inhibit");
        {
            // The handle is closed before the script is ever executed.
            let mut file = std::fs::File::create(&tool).unwrap();
            file.write_all(format!("#!/bin/sh\n{body}\n").as_bytes())
                .unwrap();
        }
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        Self(dir)
    }

    fn tool(&self) -> PathBuf {
        self.0.join("systemd-inhibit")
    }

    /// `hold_with` using this fake. The tests run as threads of one process, so another thread
    /// can fork while this script's write handle is open and make the exec fail with ETXTBSY;
    /// that one error, and no other, is retried (about 5 tries, 20 ms apart).
    fn hold(&self) -> Result<SleepLock, String> {
        let which = found(self.tool());
        let mut tries = 1;
        loop {
            match hold_with("why", &which, false) {
                Err(why) if why.contains("Text file busy") && tries < 5 => {
                    tries += 1;
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                other => return other,
            }
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for FakeInhibitor {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(target_os = "linux")]
fn found(tool: PathBuf) -> impl Fn(&str) -> Option<PathBuf> {
    move |_: &str| Some(tool.clone())
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

/// A host that grants the lock: the fake skips its own options and runs the command, as the real
/// tool does once the lock is taken. The lock was asked for on a thread that is gone by the time
/// it is checked.
#[cfg(target_os = "linux")]
#[test]
fn a_granted_lock_made_on_a_short_lived_thread_outlives_it() {
    let fake = FakeInhibitor::new(
        r#"while [ $# -gt 0 ]; do case "$1" in --*) shift ;; *) break ;; esac; done
exec "$@""#,
    );
    let fake = std::sync::Arc::new(fake);
    let lock = std::thread::spawn({
        let fake = fake.clone();
        move || fake.hold()
    })
    .join()
    .unwrap()
    .unwrap_or_else(|why| panic!("a granted lock must be an Ok: {why}"));
    let pid = lock.pid();
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

/// A host where the tool is installed but cannot take the lock (no bus, policy denial) fails after
/// a delay, without ever running the command. That must be an Err, however late the tool gives up.
#[cfg(target_os = "linux")]
#[test]
fn an_inhibitor_that_fails_late_is_an_err() {
    let fake = FakeInhibitor::new("sleep 0.4\nexit 1");
    let err = match fake.hold() {
        Ok(_) => panic!("a tool that never ran its command must not count as a hold"),
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
    // An Ok is only returned once the lock is demonstrably held, so nothing here may be lost.
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
