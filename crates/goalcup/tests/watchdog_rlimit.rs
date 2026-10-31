//! PRD-02 section 10 and section 8 steps 2, 4 and 7 (points 3, 4 and 7, PRD-17 P6): the recorder
//! gets the hard open-file limit, no core dumps, fds 3 and 4 and a session of its own; the harness
//! gets `max(soft, 4096)`, its own core limit, none of our sockets, the watchdog's process group
//! and only the env it was given. A small shell script reports what it got. Linux only.
#![cfg(target_os = "linux")]

mod common;

use common::ports;
use goalcup::cli::{NULL_FLAG, RECORDER_SUBCOMMAND};
use goalcup::watchdog::spawn;
use std::collections::BTreeMap;
use std::net::{Ipv4Addr, TcpStream};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Child;

#[derive(Clone, Copy)]
enum Lim {
    Files,
    Core,
}

fn resource(lim: Lim) -> libc::__rlimit_resource_t {
    match lim {
        Lim::Files => libc::RLIMIT_NOFILE,
        Lim::Core => libc::RLIMIT_CORE,
    }
}

/// `(soft, hard)`.
fn limit(lim: Lim) -> (u64, u64) {
    let mut got = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `got` is a valid out pointer for the call.
    assert_eq!(unsafe { libc::getrlimit(resource(lim), &mut got) }, 0);
    (got.rlim_cur, got.rlim_max)
}

fn set_soft(lim: Lim, soft: u64) {
    let new = libc::rlimit {
        rlim_cur: soft,
        rlim_max: limit(lim).1,
    };
    // SAFETY: `setrlimit` only reads the struct.
    assert_eq!(unsafe { libc::setrlimit(resource(lim), &new) }, 0);
}

/// nextest runs each test in its own process: start from a 256 soft open-file limit (launchd's
/// default) and a core limit that is not 0 where the hard limit allows it.
fn low_start() {
    set_soft(Lim::Files, 256);
    set_soft(Lim::Core, limit(Lim::Core).1);
}

/// Writes the probe script into `dir`. It uses builtins and absolute paths only, since the
/// harness has no `PATH`. `fds` are the descriptor numbers it reports open or not.
fn probe(dir: &Path, fds: &[i32]) -> PathBuf {
    let out = dir.join("out");
    let fds: Vec<String> = fds.iter().map(i32::to_string).collect();
    let script = format!(
        "#!/bin/sh\n\
         o1=$(/bin/readlink /proc/$$/fd/1); o2=$(/bin/readlink /proc/$$/fd/2)\n{{\n\
         echo \"nofile $(ulimit -Sn)\"\necho \"hard $(ulimit -Hn)\"\necho \"core $(ulimit -c)\"\n\
         for fd in {fds}; do [ -e /proc/$$/fd/$fd ] && echo \"open $fd\"; done\n\
         [ -e /proc/$$/fd/3 ] && echo \"fd3 $(/bin/readlink /proc/$$/fd/3)\"\n\
         echo \"fd1 $o1\"\necho \"fd2 $o2\"\n\
         read -r stat < /proc/$$/stat; echo \"stat $stat\"\n\
         echo \"pid $$\"\necho \"cwd $(pwd -P)\"\n\
         for a in \"$@\"; do echo \"arg $a\"; done\n\
         }} > {out}\n/bin/cat /proc/$$/environ > {out}.env\n",
        fds = fds.join(" "),
        out = out.display(),
    );
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// What the probe saw.
struct Seen {
    lines: Vec<String>,
    env: Vec<String>,
}

impl Seen {
    fn read(dir: &Path, mut child: Child) -> Seen {
        let status = common::within(common::HANG, move || child.wait().unwrap());
        assert!(status.success(), "probe exited {status}");
        let out = dir.join("out");
        let env = std::fs::read(out.with_extension("env")).unwrap();
        Seen {
            lines: std::fs::read_to_string(&out)
                .unwrap()
                .lines()
                .map(str::to_owned)
                .collect(),
            env: env
                .split(|b| *b == 0)
                .filter(|v| !v.is_empty())
                .map(|v| String::from_utf8_lossy(v).into_owned())
                .collect(),
        }
    }

    fn one(&self, key: &str) -> &str {
        let first = self.all(key).first().copied();
        first.unwrap_or_else(|| panic!("no {key} in {:?}", self.lines))
    }

    fn n(&self, key: &str) -> u64 {
        match self.one(key) {
            "unlimited" => libc::RLIM_INFINITY,
            n => n.parse().unwrap(),
        }
    }

    fn all(&self, key: &str) -> Vec<&str> {
        let lead = format!("{key} ");
        self.lines
            .iter()
            .filter_map(|l| l.strip_prefix(&lead))
            .collect()
    }

    /// `(process group, session)` from `/proc/<pid>/stat`, read after the command name.
    fn group_and_session(&self) -> (u64, u64) {
        let stat = self.one("stat");
        let rest: Vec<&str> = stat[stat.rfind(')').unwrap() + 1..]
            .split_whitespace()
            .collect();
        (rest[2].parse().unwrap(), rest[3].parse().unwrap())
    }
}

fn fd_link(fd: i32) -> String {
    std::fs::read_link(format!("/proc/self/fd/{fd}"))
        .unwrap()
        .display()
        .to_string()
}

/// A listener from `spawn::bind`, on a port it had to pick itself.
fn listener() -> std::net::TcpListener {
    let (_held, addr) = ports::held();
    spawn::bind(addr.port()).unwrap()
}

#[test]
fn the_recorder_gets_the_hard_limit_no_core_its_fds_and_its_own_session() {
    low_start();
    for null in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let exe = probe(dir.path(), &[3, 4]);
        let listener = listener();
        let (_ours, theirs) = UnixStream::pair().unwrap();
        let child = spawn::recorder(&exe, null, &listener, &theirs).unwrap();
        let pid = u64::from(child.id());
        let seen = Seen::read(dir.path(), child);

        assert_eq!(seen.n("nofile"), seen.n("hard"), "soft open-file limit");
        assert_eq!(seen.n("hard"), limit(Lim::Files).1);
        assert_eq!(seen.one("core"), "0");
        assert_eq!(seen.all("open"), ["3", "4"]);
        assert_eq!(seen.one("fd3"), fd_link(listener.as_raw_fd()));
        // Nothing the recorder says lands on a harness's screen.
        assert_eq!((seen.one("fd1"), seen.one("fd2")), ("/dev/null", "/dev/null"));
        assert_eq!(seen.group_and_session(), (pid, pid), "its own session");
        let mut args = vec![RECORDER_SUBCOMMAND];
        if null {
            args.push(NULL_FLAG);
        }
        assert_eq!(seen.all("arg"), args);
    }
    // The raise ran in the child only.
    assert_eq!(limit(Lim::Files).0, 256);
}

#[test]
fn the_harness_gets_4096_none_of_our_sockets_our_group_and_only_its_env() {
    low_start();
    let dir = tempfile::tempdir().unwrap();
    let listener = listener();
    let (ours, theirs) = UnixStream::pair().unwrap();
    let ours_fds = [
        3,
        4,
        listener.as_raw_fd(),
        ours.as_raw_fd(),
        theirs.as_raw_fd(),
    ];
    let exe = probe(dir.path(), &ours_fds);
    let argv = ["one".to_owned(), "two words".to_owned()];
    let env = BTreeMap::from([("GC_PROBE".to_owned(), "yes".to_owned())]);
    let child = spawn::harness(&exe, &argv, &env, Some(dir.path())).unwrap();
    let seen = Seen::read(dir.path(), child);

    let (_, hard) = limit(Lim::Files);
    assert_eq!(seen.n("nofile"), 4_096.min(hard));
    assert_eq!(seen.n("hard"), hard);
    assert_eq!(seen.one("core") == "0", limit(Lim::Core).0 == 0);
    assert!(seen.all("open").is_empty(), "{:?}", seen.all("open"));
    // SAFETY: integers only.
    let (group, session) = unsafe { (libc::getpgrp(), libc::getsid(0)) };
    assert_eq!(
        seen.group_and_session(),
        (group as u64, session as u64),
        "the watchdog's process group"
    );
    assert_eq!(seen.env, ["GC_PROBE=yes"]);
    assert_eq!(seen.all("arg"), argv);
    assert_eq!(
        Path::new(seen.one("cwd")),
        dir.path().canonicalize().unwrap()
    );
    assert_eq!(limit(Lim::Files).0, 256);
}

#[test]
fn a_taken_port_gives_way_to_a_new_one() {
    let (held, addr) = ports::held();
    assert!(spawn::bind_exact(addr.port()).is_err(), "recover refuses");
    let bound = spawn::bind(addr.port()).unwrap();
    let at = bound.local_addr().unwrap();
    assert_eq!(at.ip(), Ipv4Addr::LOCALHOST);
    assert_ne!(at.port(), addr.port());
    assert_ne!(at.port(), 0);
    TcpStream::connect(at).unwrap();
    // SAFETY: integers only.
    let flags = unsafe { libc::fcntl(bound.as_raw_fd(), libc::F_GETFD) };
    assert_ne!(flags & libc::FD_CLOEXEC, 0, "FD_CLOEXEC on the listener");
    drop(held);
}

#[test]
fn a_port_with_connections_in_time_wait_is_bound_again() {
    let (held, addr) = ports::held();
    let mut client = TcpStream::connect(addr).unwrap();
    let (server, _) = held.accept().unwrap();
    // The listening side closes first, so its end of the connection sits in TIME_WAIT.
    drop(server);
    std::io::Read::read_to_end(&mut client, &mut Vec::new()).unwrap();
    drop(client);
    drop(held);

    let plain = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None).unwrap();
    let shown = plain.bind(&addr.into()).is_err();
    drop(plain);
    let bound = spawn::bind_exact(addr.port()).unwrap();
    assert_eq!(bound.local_addr().unwrap(), addr);
    if !shown {
        eprintln!("SKIPPED: no TIME_WAIT refusal without SO_REUSEADDR here; rebind not shown");
    }
}
