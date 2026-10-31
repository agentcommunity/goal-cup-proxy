//! What the watchdog binds and starts (PRD-02 section 8 steps 2, 4 and 7, section 10): the listen
//! socket, the control pair, the recorder and the harness.
//!
//! Both sockets carry `FD_CLOEXEC`; only the recorder's `pre_exec` clears it, on fd 3 and fd 4, so
//! the harness never inherits them (a harness holding the listener keeps the port alive after
//! goalcup is gone). Every `pre_exec` makes only async-signal-safe calls.

use crate::cli::{NULL_FLAG, RECORDER_SUBCOMMAND};
use crate::pipeline::fds::{self, Target};
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

/// Binds `127.0.0.1:port` with `SO_REUSEADDR` (so `goalcup recover` can reopen a port whose closed
/// connections sit in TIME_WAIT), backlog 4,096 and `FD_CLOEXEC`. A port that is taken or refused
/// gives way to port 0.
pub fn bind(port: u16) -> io::Result<TcpListener> {
    match listen(port) {
        Err(_) if port != 0 => listen(0),
        bound => bound,
    }
}

fn listen(port: u16) -> io::Result<TcpListener> {
    let socket = Socket::new(Domain::IPV4, Type::STREAM, None)?;
    socket.set_cloexec(true)?;
    socket.set_reuse_address(true)?;
    socket.bind(&SocketAddr::from(([127, 0, 0, 1], port)).into())?;
    socket.listen(BACKLOG)?;
    Ok(socket.into())
}

/// The control socket pair, one JSON line each way. The standard library opens both ends with
/// `FD_CLOEXEC` (`SOCK_CLOEXEC` on Linux).
pub fn control_pair() -> io::Result<(UnixStream, UnixStream)> {
    UnixStream::pair()
}

/// Starts `exe RECORDER_SUBCOMMAND [NULL_FLAG]`, stdin from `/dev/null`, in a session of its own
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
    command.arg(RECORDER_SUBCOMMAND).stdin(Stdio::null());
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
