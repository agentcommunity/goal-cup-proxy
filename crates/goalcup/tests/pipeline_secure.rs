//! PRD-02 section 9.2 step 1 (point 4): a recorder boot turns core dumps off and its panic hook
//! never prints the panic's message. The test runs itself again as a child that hardens, panics
//! with a marker, and reports its limits; the parent checks the child.

use goalcup::pipeline::secure;
use std::process::Command;

const CHILD: &str = "PIPELINE_SECURE_CHILD";
const MARKER: &str = "panic-payload-marker-5c1e";

/// The unused `prctl` arguments, as the `unsigned long` the kernel reads.
#[cfg(target_os = "linux")]
const Z: libc::c_ulong = 0;

fn child() -> ! {
    secure::harden();
    secure::panic_hook();
    let mut lim = libc::rlimit {
        rlim_cur: 1,
        rlim_max: 1,
    };
    // SAFETY: `lim` is a valid out pointer for the call.
    unsafe { libc::getrlimit(libc::RLIMIT_CORE, &mut lim) };
    println!("core={}", lim.rlim_cur);
    #[cfg(target_os = "linux")]
    {
        // SAFETY: PR_GET_DUMPABLE takes no argument and changes nothing.
        let dumpable = unsafe { libc::prctl(libc::PR_GET_DUMPABLE, Z, Z, Z, Z) };
        println!("dumpable={dumpable}");
    }
    let value = String::from(MARKER);
    panic!("a value that must never be printed: {value}");
}

#[test]
fn boot_hardening_and_panic_hook() {
    if std::env::var_os(CHILD).is_some() {
        child();
    }
    let exe = std::env::current_exe().expect("own test binary");
    let out = Command::new(exe)
        .args(["boot_hardening_and_panic_hook", "--exact", "--nocapture"])
        .env(CHILD, "1")
        .output()
        .expect("child runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(!out.status.success());
    assert!(stdout.lines().any(|l| l == "core=0"), "{stdout}");
    #[cfg(target_os = "linux")]
    assert!(stdout.lines().any(|l| l == "dumpable=0"), "{stdout}");
    let at = stderr
        .split("pipeline_secure.rs:")
        .nth(1)
        .expect("the hook names the file");
    assert!(at.starts_with(|c: char| c.is_ascii_digit()), "no line");
    assert!(!stdout.contains(MARKER));
    assert!(!stderr.contains(MARKER));
}
