//! PRD-17 section 2.8 rows P3, P4 and P5, at the layer that exists before the watchdog loop:
//! `spawn::harness`, `Shield`, `Modes` and `wait_resuming`, under a pty the test makes itself
//! (`openpty`), because a build box runs tests under `sandbox exec`, which is not a terminal.
//!
//! The watchdog's part is played by this test binary run again as a helper (`launch_helper`, which
//! does nothing unless `GC_LAUNCH_ROLE` is set). The helper becomes a session leader whose
//! controlling terminal is the pty slave, so it and the harness it starts are the terminal's
//! foreground group, exactly as under a participant's shell. The test types on the master and reads
//! what the helper and the harness print. Every wait is a read with a deadline.
//!
//! Left out until the watchdog loop is on the branch: SC3 (nothing is written to the terminal while
//! the harness runs). Linux only.
#![cfg(target_os = "linux")]

use goalcup::watchdog::shield::{Modes, Shield, wait_resuming};
use goalcup::watchdog::spawn;
use std::collections::BTreeMap;
use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const ROLE: &str = "GC_LAUNCH_ROLE";
/// How long a test waits for the helper. Generous: a loaded box, never a synchronisation sleep.
const DEADLINE: Duration = Duration::from_secs(30);

// ---- the helper (runs inside the pty, as the watchdog's stand-in) ----

fn say(line: &str) {
    let mut out = std::io::stdout();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

fn harness_env() -> BTreeMap<String, String> {
    let path = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_string());
    BTreeMap::from([("PATH".to_string(), path)])
}

fn start(script: &str) -> Child {
    let argv = vec!["-c".to_string(), script.to_string()];
    spawn::harness(Path::new("/bin/sh"), &argv, &harness_env(), None).expect("harness starts")
}

fn termios() -> libc::termios {
    // SAFETY: a zeroed local that `tcgetattr` fills.
    unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        assert_eq!(libc::tcgetattr(0, &mut t), 0, "stdin is the pty");
        t
    }
}

fn pid_of(child: &Child) -> libc::pid_t {
    libc::pid_t::try_from(child.id()).expect("pid")
}

fn describe(status: std::process::ExitStatus) -> String {
    use std::os::unix::process::ExitStatusExt;
    match (status.code(), status.signal()) {
        (Some(code), _) => format!("code={code}"),
        (_, Some(signal)) => format!("signal={signal}"),
        _ => "unknown".to_string(),
    }
}

// `wait_resuming` reaps each harness; clippy cannot see that.
#[allow(clippy::zombie_processes)]
#[test]
fn launch_helper() {
    let Ok(role) = std::env::var(ROLE) else {
        return;
    };
    match role.as_str() {
        "tui" => {
            let child = start("read x; echo got=$x");
            let pid = pid_of(&child);
            // SAFETY: integers only.
            let (terminal, group, mine) =
                unsafe { (libc::tcgetpgrp(0), libc::getpgid(pid), libc::getpgrp()) };
            say(&format!(
                "foreground={}",
                terminal == group && group == mine
            ));
            let status = wait_resuming(child.id()).expect("wait");
            say(&format!("exit {}", describe(status)));
        }
        "modes" => {
            let modes = Modes::save(0).expect("stdin is a terminal");
            let before = termios().c_lflag;
            let raw = start("stty raw -echo");
            wait_resuming(raw.id()).expect("wait");
            let during = termios().c_lflag;
            say(&format!("raw_seen={}", during & libc::ICANON == 0));
            modes.restore().expect("restore");
            say(&format!("restored={}", termios().c_lflag == before));
            let stops = start("kill -STOP $$; exit 7");
            let status = wait_resuming(stops.id()).expect("wait");
            say(&format!("resumed {}", describe(status)));
        }
        "ctrl_c" => {
            // No shell in between: a shell may ignore the terminal's signals for its own reasons.
            let argv = vec!["120".to_string()];
            let child = spawn::harness(Path::new("/bin/sleep"), &argv, &harness_env(), None)
                .expect("start");
            let status = std::fs::read_to_string(format!("/proc/{}/status", child.id()))
                .expect("harness status");
            let mask = status
                .lines()
                .find_map(|line| line.strip_prefix("SigIgn:"))
                .expect("SigIgn line");
            say(&format!("ign={}", mask.trim()));
            let mut shield = Shield::take().expect("shield");
            say("shielded");
            let status = wait_resuming(child.id()).expect("wait");
            shield.harness_exited();
            say(&format!("harness {}", describe(status)));
            say("watchdog alive");
        }
        other => panic!("unknown role {other}"),
    }
}

// ---- the test side ----

struct Pty {
    master: OwnedFd,
    slave: OwnedFd,
}

fn pty() -> Pty {
    let (mut master, mut slave) = (0, 0);
    // SAFETY: two integers are written; the name and termios arguments may be null.
    let made = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    assert_eq!(made, 0, "openpty: {}", std::io::Error::last_os_error());
    // SAFETY: both descriptors were just opened and nothing else owns them.
    let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
    for fd in [&master, &slave] {
        // SAFETY: integers only.
        unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) };
    }
    Pty { master, slave }
}

/// The helper process; its whole process group is killed if a test ends early.
struct Helper {
    child: Child,
}

impl Drop for Helper {
    fn drop(&mut self) {
        // SAFETY: integers only; the helper is its own group leader.
        unsafe {
            libc::kill(
                -libc::pid_t::try_from(self.child.id()).unwrap(),
                libc::SIGKILL,
            )
        };
        let _ = self.child.wait();
    }
}

fn run_helper(role: &str, pty: &Pty) -> Helper {
    let exe = std::env::current_exe().expect("test binary");
    let stdio = || Stdio::from(pty.slave.try_clone().expect("dup slave"));
    let mut command = Command::new(exe);
    command
        .args([
            "launch_helper",
            "--exact",
            "--nocapture",
            "--test-threads",
            "1",
        ])
        .env(ROLE, role)
        .stdin(stdio())
        .stdout(stdio())
        .stderr(stdio());
    // SAFETY: only async-signal-safe calls (`setsid`, `ioctl`, `signal`) and no allocation. The
    // pty slave is already fd 0 here: it becomes the session's controlling terminal, and the
    // helper's group its foreground group. The terminal's signals start at their defaults.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            for signal in [libc::SIGINT, libc::SIGQUIT, libc::SIGTSTP] {
                libc::signal(signal, libc::SIG_DFL);
            }
            Ok(())
        });
    }
    Helper {
        child: command.spawn().expect("helper starts"),
    }
}

#[derive(Debug, PartialEq)]
enum Read {
    Matched,
    Eof,
    Timeout,
}

/// Reads the master into `out` until `until` holds, the pty closes (every holder of the slave is
/// gone) or the deadline passes.
fn pump(master: RawFd, out: &mut Vec<u8>, until: impl Fn(&str) -> bool) -> Read {
    let end = Instant::now() + DEADLINE;
    loop {
        if until(&String::from_utf8_lossy(out)) {
            return Read::Matched;
        }
        let Some(left) = end.checked_duration_since(Instant::now()) else {
            return Read::Timeout;
        };
        let mut poll = libc::pollfd {
            fd: master,
            events: libc::POLLIN,
            revents: 0,
        };
        let wait = i32::try_from(left.as_millis().min(200)).unwrap();
        // SAFETY: one valid pollfd.
        if unsafe { libc::poll(&mut poll, 1, wait) } <= 0 {
            continue;
        }
        let mut buf = [0u8; 4096];
        // SAFETY: `buf` is valid for its length.
        let n = unsafe { libc::read(master, buf.as_mut_ptr().cast(), buf.len()) };
        if n <= 0 {
            // EIO is how a master reports that the slave side is closed.
            return if until(&String::from_utf8_lossy(out)) {
                Read::Matched
            } else {
                Read::Eof
            };
        }
        out.extend_from_slice(&buf[..usize::try_from(n).unwrap()]);
    }
}

fn type_on(master: RawFd, bytes: &[u8]) {
    // SAFETY: `bytes` is valid for its length.
    let n = unsafe { libc::write(master, bytes.as_ptr().cast(), bytes.len()) };
    assert_eq!(
        n,
        isize::try_from(bytes.len()).unwrap(),
        "typed on the master"
    );
}

/// Runs the helper to the end of its output and returns what it printed.
fn finish(mut helper: Helper, pty: Pty) -> String {
    let Pty { master, slave } = pty;
    drop(slave);
    let mut out = Vec::new();
    let read = pump(master.as_raw_fd(), &mut out, |_| false);
    let text = String::from_utf8_lossy(&out).into_owned();
    assert_eq!(
        read,
        Read::Eof,
        "helper still running at the deadline:\n{text}"
    );
    let status = helper.child.wait().expect("wait helper");
    assert!(status.success(), "helper failed ({status}):\n{text}");
    text
}

/// P4: the harness has no process group of its own, so its first read from the terminal does not
/// raise `SIGTTIN`.
#[test]
fn a_tui_child_reads_the_terminal() {
    let pty = pty();
    let helper = run_helper("tui", &pty);
    // Typed ahead: the line discipline holds it until the child reads.
    type_on(pty.master.as_raw_fd(), b"Q7\n");
    let text = finish(helper, pty);
    assert!(text.contains("foreground=true"), "{text}");
    assert!(text.contains("got=Q7"), "{text}");
    assert!(text.contains("exit code=0"), "{text}");
}

/// P5: a harness that left the terminal raw does not leave it raw, and a harness that stopped
/// itself is continued.
#[test]
fn modes_restored_after_a_raw_exit() {
    let pty = pty();
    let helper = run_helper("modes", &pty);
    let text = finish(helper, pty);
    assert!(text.contains("raw_seen=true"), "{text}");
    assert!(text.contains("restored=true"), "{text}");
    assert!(text.contains("resumed code=7"), "{text}");
}

/// P3: ^C goes to the whole foreground group. The harness was started with the terminal's signals
/// at their defaults and dies of `SIGINT`; the shielded watchdog catches it and carries on.
#[test]
fn ctrl_c_reaches_the_harness_and_the_watchdog_lives() {
    let pty = pty();
    let helper = run_helper("ctrl_c", &pty);
    let master = pty.master.as_raw_fd();
    let mut seen = Vec::new();
    let read = pump(master, &mut seen, |t| t.contains("shielded"));
    assert_eq!(read, Read::Matched, "{}", String::from_utf8_lossy(&seen));
    type_on(master, &[0x03]);
    let text = finish(helper, pty);
    let text = format!("{}{text}", String::from_utf8_lossy(&seen));
    assert!(
        text.contains(&format!("harness signal={}", libc::SIGINT)),
        "{text}"
    );
    assert!(text.contains("watchdog alive"), "{text}");
    let mask = text
        .split("ign=")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or_else(|| panic!("harness printed no mask:\n{text}"));
    let ignored = u64::from_str_radix(mask.trim(), 16).expect("hex mask");
    for signal in [libc::SIGINT, libc::SIGQUIT, libc::SIGTSTP] {
        assert_eq!(
            ignored >> (signal - 1) & 1,
            0,
            "signal {signal} is ignored: {mask}\n{text}"
        );
    }
}
