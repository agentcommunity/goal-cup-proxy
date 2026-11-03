//! What the watchdog binds, starts and ends (PRD-02 section 8 steps 2, 4, 6, 7, 10 and 11, section
//! 10): the listen socket, the recorder, the harness, the helpers and the harness tree. The
//! control pair is `UnixStream::pair()`, which opens both ends with `FD_CLOEXEC` (`SOCK_CLOEXEC`
//! on Linux).
//!
//! Both sockets carry `FD_CLOEXEC`; only the recorder's `pre_exec` clears it, on fd 3 and fd 4, so
//! the harness never inherits them (a harness holding the listener keeps the port alive after
//! goalcup is gone). Every `pre_exec` makes only async-signal-safe calls.

use crate::cli::{NULL_FLAG, RECORDER_SUBCOMMAND};
use crate::pipeline::fds::{self, Target};
use crate::pipeline::status::Outlived;
use socket2::{Domain, Socket, Type};
use std::collections::BTreeMap;
use std::io;
use std::net::{SocketAddr, TcpListener};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};

/// The recorder's listen socket.
pub const LISTEN_FD: RawFd = 3;
/// The recorder's end of the control pair.
pub const CONTROL_FD: RawFd = 4;

/// The accept backlog; the kernel clamps it to `somaxconn`.
const BACKLOG: i32 = 4_096;

/// [`bind_exact`], and a port that is taken or refused gives way to port 0 (`goalcup continue`).
pub fn bind(port: u16) -> io::Result<TcpListener> {
    match bind_exact(port) {
        Err(_) if port != 0 => bind_exact(0),
        bound => bound,
    }
}

/// Binds `127.0.0.1:port` with `SO_REUSEADDR` (so `goalcup recover` can reopen a port whose closed
/// connections sit in TIME_WAIT), backlog 4,096 and `FD_CLOEXEC`, or returns the OS error: recover
/// reopens the same port or refuses.
pub fn bind_exact(port: u16) -> io::Result<TcpListener> {
    let socket = Socket::new(Domain::IPV4, Type::STREAM, None)?;
    socket.set_cloexec(true)?;
    socket.set_reuse_address(true)?;
    socket.bind(&SocketAddr::from(([127, 0, 0, 1], port)).into())?;
    socket.listen(BACKLOG)?;
    Ok(socket.into())
}

/// Starts `exe RECORDER_SUBCOMMAND [NULL_FLAG]`, stdin, stdout and stderr on `/dev/null` (its
/// lines go to `goalcup.log`, never onto a harness's screen), in a session of its own
/// (terminal signals and a closed window never reach it), with `listener` on fd 3, `control` on
/// fd 4, core dumps off and the soft open-file limit raised to the hard one.
///
/// Call it while fds 3 and 4 are taken in this process (the watchdog binds the listener and the
/// control pair first): otherwise the standard library's exec-error pipe may sit on fd 3 or 4 in
/// the child, and a failed exec would then show only as the child's exit.
pub fn recorder(
    exe: &Path,
    null: bool,
    listener: &TcpListener,
    control: &UnixStream,
) -> io::Result<Child> {
    let mut command = Command::new(exe);
    command
        .arg(RECORDER_SUBCOMMAND)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if null {
        command.arg(NULL_FLAG);
    }
    let (listen_fd, control_fd) = (listener.as_raw_fd(), control.as_raw_fd());
    // SAFETY: the closure runs in the forked child before `exec` and makes only async-signal-safe
    // calls: `setsid`, `fcntl`, `dup2` and `setrlimit` here, and `fds::raise`, which calls only
    // `getrlimit` and `setrlimit` on stack values and allocates nothing. `last_os_error` reads
    // errno into a value with no allocation.
    unsafe {
        command.pre_exec(move || {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            let mut control_fd = control_fd;
            if control_fd == LISTEN_FD {
                // Moved out of the way before the listener takes fd 3.
                control_fd = libc::fcntl(control_fd, libc::F_DUPFD_CLOEXEC, CONTROL_FD + 1);
                if control_fd == -1 {
                    return Err(io::Error::last_os_error());
                }
            }
            place(listen_fd, LISTEN_FD)?;
            place(control_fd, CONTROL_FD)?;
            let zero = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            libc::setrlimit(libc::RLIMIT_CORE, &zero);
            fds::raise(Target::Recorder);
            Ok(())
        });
    }
    command.spawn()
}

/// Puts `fd` at `at` without `FD_CLOEXEC`. `dup2` onto another number clears the flag on the new
/// descriptor; onto the same number it changes nothing, so the flag is cleared by hand.
///
/// SAFETY: async-signal-safe (`dup2`, `fcntl`); called only from `pre_exec`.
unsafe fn place(fd: RawFd, at: RawFd) -> io::Result<()> {
    // SAFETY: integers only.
    let ok = unsafe {
        if fd == at {
            let flags = libc::fcntl(at, libc::F_GETFD);
            flags != -1 && libc::fcntl(at, libc::F_SETFD, flags & !libc::FD_CLOEXEC) != -1
        } else {
            libc::dup2(fd, at) != -1
        }
    };
    if ok {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Starts the harness from the `LaunchPlan` fields: `program` with `argv`, exactly `env`, in
/// `cwd` if given. It stays in the watchdog's process group (no `setsid`, no `setpgid`): a TUI
/// outside the terminal's foreground group stops at its first read. Its soft open-file limit
/// becomes `max(soft, 4096)`, at most the hard limit; its core limit stays its own.
pub fn harness(
    program: &Path,
    argv: &[String],
    env: &BTreeMap<String, String>,
    cwd: Option<&Path>,
) -> io::Result<Child> {
    let mut command = Command::new(program);
    command.args(argv).env_clear().envs(env);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    // SAFETY: `fds::raise` calls only `getrlimit` and `setrlimit` on stack values and allocates
    // nothing, so it is async-signal-safe in the forked child.
    unsafe {
        command.pre_exec(|| {
            fds::raise(Target::Harness);
            Ok(())
        });
    }
    command.spawn()
}

/// Makes the watchdog a child subreaper on Linux (`PR_SET_CHILD_SUBREAPER`), so a process the
/// harness forks and leaves stays findable among the watchdog's children. Elsewhere, nothing. The
/// error is the OS's; the caller logs it and goes on.
pub fn subreaper() -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: `prctl(PR_SET_CHILD_SUBREAPER, 1)` takes integers only and touches no memory.
        if unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// How long a helper (`plan.before`, `plan.after`) may run, and how long the harness tree has
/// between SIGTERM and SIGKILL.
pub const BUDGET_NS: u64 = 10_000_000_000;

/// How often a wait looks again.
pub const TICK: std::time::Duration = std::time::Duration::from_millis(100);

/// Runs each entry as `program` followed by its arguments, with the harness's `env`, stdio on
/// `/dev/null`, one after the other, each ended (SIGKILL) after `budget_ns` of boot time. A
/// failure is logged with fixed words and the index, never an argument, and the next one runs.
pub fn helpers(
    program: &Path,
    entries: &[Vec<String>],
    env: &BTreeMap<String, String>,
    budget_ns: u64,
    entry_dir: &Path,
    step: &'static str,
) {
    use crate::cli::log::{Fact, line};
    use crate::pipeline::clock::Mono;
    for (i, args) in entries.iter().enumerate() {
        let index = ("index", Fact::N(i as u64));
        let spawned = Command::new(program)
            .args(args)
            .env_clear()
            .envs(env)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let mut child = match spawned {
            Ok(child) => child,
            Err(e) => {
                line(
                    entry_dir,
                    "helper_failed",
                    &[
                        ("step", Fact::Word(step)),
                        index,
                        ("io", Fact::Io(e.kind())),
                    ],
                );
                continue;
            }
        };
        let start = Mono::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if Mono::now().0.saturating_sub(start.0) < budget_ns => {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
            }
        };
        match status {
            Some(s) if s.success() => {}
            Some(s) => line(
                entry_dir,
                "helper_failed",
                &[("step", Fact::Word(step)), index, ("exit", Fact::Exit(s))],
            ),
            None => line(
                entry_dir,
                "helper_failed",
                &[
                    ("step", Fact::Word(step)),
                    index,
                    ("why", Fact::Word("timeout")),
                ],
            ),
        }
    }
}

/// One process as the tree walk sees it.
struct Proc {
    pid: u32,
    ppid: u32,
    zombie: bool,
}

/// Every process: Linux `/proc/*/stat`, macOS `ps -A -o pid=,ppid=,stat=`.
#[cfg(target_os = "linux")]
fn procs() -> Vec<Proc> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse().ok()) else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        // The name in brackets may hold spaces and `)`: fields after the last `)` start at 3.
        let Some(rest) = stat.rfind(')').map(|i| &stat[i + 1..]) else {
            continue;
        };
        let mut fields = rest.split_whitespace();
        let zombie = fields.next() == Some("Z");
        if let Some(ppid) = fields.next().and_then(|p| p.parse().ok()) {
            out.push(Proc { pid, ppid, zombie });
        }
    }
    out
}

#[cfg(not(target_os = "linux"))]
fn procs() -> Vec<Proc> {
    let Ok(out) = Command::new("ps")
        .args(["-A", "-o", "pid=,ppid=,stat="])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            Some(Proc {
                pid: f.next()?.parse().ok()?,
                ppid: f.next()?.parse().ok()?,
                zombie: f.next().is_some_and(|s| s.starts_with('Z')),
            })
        })
        .collect()
}

/// The harness tree: `harness` and its descendants by a parent walk, and on Linux the
/// watchdog's own children (orphans the subreaper took in) that started no earlier than the
/// harness. Never the watchdog, never a pid `spare` names (the recorder), never a zombie. Each
/// with its start id, so a reused pid is never taken for it.
fn tree(harness: (u32, u64), spare: &dyn Fn(u32) -> bool) -> Vec<(u32, u64)> {
    let me = std::process::id();
    let all = procs();
    let mut found = vec![harness.0];
    let mut i = 0;
    while i < found.len() {
        let parent = found[i];
        for p in &all {
            if p.ppid == parent && !found.contains(&p.pid) {
                found.push(p.pid);
            }
        }
        i += 1;
    }
    if cfg!(target_os = "linux") {
        for p in all.iter().filter(|p| p.ppid == me) {
            let late = crate::cli::proc_start_id(p.pid).is_some_and(|s| s >= harness.1);
            if late && !found.contains(&p.pid) {
                found.push(p.pid);
            }
        }
    }
    found
        .into_iter()
        .filter(|pid| *pid != me && !spare(*pid))
        .filter(|pid| !all.iter().any(|p| p.pid == *pid && p.zombie))
        .filter_map(|pid| {
            let start = crate::cli::proc_start_id(pid)?;
            (pid != harness.0 || start == harness.1).then_some((pid, start))
        })
        .collect()
}

/// Ends the harness tree ([`tree`]): SIGTERM to each, SIGKILL after 10 s of boot time to each
/// still alive. Signals go to single pids only, never a group, so the watchdog's own process
/// group is never signalled. The watchdog's own children among them, except the harness (its
/// caller reaps that one), are reaped here.
pub fn end_tree(harness: (u32, u64), spare: &dyn Fn(u32) -> bool) {
    use crate::pipeline::clock::Mono;
    let me = std::process::id();
    let send = |procs: &[(u32, u64)], signal: libc::c_int| {
        for (pid, _) in procs {
            if let Ok(p) = libc::pid_t::try_from(*pid) {
                // SAFETY: a single positive pid and a signal number; touches no memory.
                unsafe { libc::kill(p, signal) };
            }
        }
    };
    let targets = tree(harness, spare);
    send(&targets, libc::SIGTERM);
    let start = Mono::now();
    loop {
        let all = procs();
        let mut left = Vec::new();
        for &(pid, start_id) in &targets {
            let zombie = all.iter().any(|p| p.pid == pid && p.zombie);
            if zombie && pid != harness.0 && all.iter().any(|p| p.pid == pid && p.ppid == me) {
                let mut status = 0;
                // SAFETY: reaps one child of ours by pid; writes one integer.
                unsafe { libc::waitpid(pid as libc::pid_t, &mut status, libc::WNOHANG) };
            } else if !zombie && crate::cli::alive(pid, start_id) {
                left.push((pid, start_id));
            }
        }
        if left.is_empty() {
            return;
        }
        if Mono::now().0.saturating_sub(start.0) >= BUDGET_NS {
            send(&left, libc::SIGKILL);
            return;
        }
        std::thread::sleep(TICK);
    }
}

/// The Linux children of the watchdog that started no earlier than the harness and are still
/// alive at the seal, with their program names (`/proc/<pid>/comm`). None elsewhere or when
/// there are none.
pub fn outlived(harness_start: u64, spare: &dyn Fn(u32) -> bool) -> Option<Outlived> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let me = std::process::id();
    let programs: Vec<String> = procs()
        .into_iter()
        .filter(|p| p.ppid == me && !p.zombie && !spare(p.pid))
        .filter(|p| crate::cli::proc_start_id(p.pid).is_some_and(|s| s >= harness_start))
        .map(|p| {
            std::fs::read_to_string(format!("/proc/{}/comm", p.pid))
                .map(|c| c.trim_end().to_string())
                .unwrap_or_default()
        })
        .collect();
    (!programs.is_empty()).then_some(Outlived {
        count: programs.len() as u64,
        programs,
    })
}

/// One look at the harness: `waitpid(WNOHANG | WUNTRACED)`. A stopped harness is continued
/// (`SIGCONT` to its group, the watchdog's own, as [`super::shield::wait_resuming`] does) and
/// reads as running. This is the one signal the watchdog's own group gets, and an accepted
/// exception to "never signalled": `SIGCONT` only resumes, and a job-control stop holds the
/// harness's children too, which a `SIGCONT` to the pid alone would leave stopped. `Some` once it has exited; a lost child (`ECHILD`) reads as exited with no
/// status.
pub fn look(pid: u32) -> Option<Option<std::process::ExitStatus>> {
    use std::os::unix::process::ExitStatusExt;
    let p = libc::pid_t::try_from(pid).ok()?;
    let mut status = 0;
    // SAFETY: `waitpid` writes the one integer it is handed.
    let reaped = unsafe { libc::waitpid(p, &mut status, libc::WNOHANG | libc::WUNTRACED) };
    match reaped {
        0 => None,
        -1 if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted => None,
        -1 => Some(None),
        _ if libc::WIFSTOPPED(status) => {
            // SAFETY: integers only.
            unsafe {
                let group = libc::getpgid(p);
                libc::kill(if group > 0 { -group } else { p }, libc::SIGCONT);
            }
            None
        }
        _ => Some(Some(std::process::ExitStatus::from_raw(status))),
    }
}
