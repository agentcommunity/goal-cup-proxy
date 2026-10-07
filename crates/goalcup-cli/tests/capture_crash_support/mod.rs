//! Support for `capture_crash_campaign.rs`: only what the crash and torn-tail campaign adds to
//! `m2_vertical_support` and to `capture_campaign_support` (status waits, delivery counts, the
//! owned-process helpers and the evidence `Row` both campaigns share). Nothing here edits or
//! shadows those modules, and nothing reaches the capture path except the real binaries, the
//! operating system and the filesystem.
//!
//! - [`Daemon`]: a second real `goalcup-daemon --foreground`, spawned by the test over the
//!   first one's private registry (the restart), optionally under an operating-system file
//!   size limit (the one partial-append fault the campaign injects, see `Daemon::spawn`);
//! - [`cli`]: `goalcup` run raw, so a refusal can be observed as what it is;
//! - byte retention: whole-trace hashes taken before and after every inspection.

use crate::capture_campaign_support::{kill9_owned_daemon, sha256};
use crate::m2_vertical_support::{Vertical, files_under, wait_until};
use std::collections::BTreeMap;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// A real `goalcup-daemon --foreground` the test owns, over the private roots of `v`.
pub struct Daemon {
    child: Option<Child>,
}

impl Daemon {
    /// Spawn it from the first daemon's own configuration file, so it opens the same private
    /// registry, artifact root and trace directories. Its output goes to
    /// `daemon-<tag>.out` / `.err` under the world's `logs/`.
    ///
    /// `file_size_limit` is the campaign's one partial-append seam. It sets the child's
    /// `RLIMIT_FSIZE` (hard and soft) and leaves `SIGXFSZ` ignored (an ignored disposition
    /// survives `exec`), in the child only, before it starts: the daemon itself is the
    /// shipped binary and runs its production code unchanged. A `write` that crosses the limit
    /// is then a short write of exactly the bytes up to it, and the next `write` fails
    /// `EFBIG`, which is what the chain writer's own failure path sees.
    pub fn spawn(v: &Vertical, tag: &str, file_size_limit: Option<u64>) -> Daemon {
        let log = |name: String| std::fs::File::create(v.logs.join(name)).unwrap();
        let mut command = Command::new(crate::support::daemon_binary());
        command
            .arg("--config")
            .arg(v.root.join("goalcup-daemon.toml"))
            .arg("--foreground")
            .env_clear()
            .env("HOME", v.root.join("home"))
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .stdin(Stdio::null())
            .stdout(log(format!("daemon-{tag}.out")))
            .stderr(log(format!("daemon-{tag}.err")));
        if let Some(limit) = file_size_limit {
            // SAFETY: the closure runs in the forked child before exec and calls only
            // async-signal-safe functions (`signal`, `setrlimit`).
            unsafe {
                command.pre_exec(move || {
                    libc::signal(libc::SIGXFSZ, libc::SIG_IGN);
                    let limit = libc::rlimit {
                        rlim_cur: limit,
                        rlim_max: limit,
                    };
                    if libc::setrlimit(libc::RLIMIT_FSIZE, &limit) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
        let child = command.spawn().expect("the daemon binary");
        let daemon = Daemon { child: Some(child) };
        // It prints its control address and control file once it serves.
        let out = v.logs.join(format!("daemon-{tag}.out"));
        wait_until("the restarted daemon's startup lines", || {
            std::fs::read_to_string(&out).is_ok_and(|text| text.lines().count() >= 2)
        });
        daemon
    }

    pub fn pid(&self) -> i32 {
        self.child.as_ref().expect("a daemon").id() as i32
    }

    /// `kill -9`, reaped.
    pub fn kill9(&mut self, v: &Vertical) {
        let pid = self.pid();
        kill9_owned_daemon(v, pid);
        if let Some(mut child) = self.child.take() {
            let _ = child.wait();
        }
    }
}

impl Drop for Daemon {
    // Best effort and never a panic: a panic while a test is already unwinding would abort
    // before the world's own drops end the launcher's workload and daemon.
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            // SAFETY: two integers, a child this module started and has not reaped. CONT first,
            // in case a test stopped it and failed before continuing it.
            unsafe {
                libc::kill(child.id() as i32, libc::SIGCONT);
                libc::kill(child.id() as i32, libc::SIGTERM);
            }
            let until = Instant::now() + Duration::from_secs(15);
            while child.try_wait().ok().flatten().is_none() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(50));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

// ------------------------------------------------------------------ the CLI, raw

/// `goalcup ARGS` with only this world's directories in its environment, to completion: its
/// exit, standard output and standard error. Unlike `Vertical::status` it asserts nothing, so
/// a refusal can be observed as what it is.
pub fn cli(v: &Vertical, args: &[&str]) -> (ExitStatus, String, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_goalcup"));
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("HOME", v.root.join("home"))
        .env(goalcup_cli::config::CONFIG_DIR_ENV, &v.cfg)
        .env(goalcup_cli::config::CONNECTORS_DIR_ENV, &v.connectors)
        .stdin(Stdio::null())
        .args(args);
    for key in ["USER", "LOGNAME", "TMPDIR"] {
        if let Ok(value) = std::env::var(key) {
            command.env(key, value);
        }
    }
    let output = command.output().expect("goalcup");
    (
        output.status,
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

// ------------------------------------------------------------------ bytes

/// Every file under `dir`, relative name to sha256, so a later look can prove no byte of the
/// trace moved.
pub fn tree_hashes(dir: &Path) -> BTreeMap<String, String> {
    files_under(dir)
        .into_iter()
        .map(|(path, bytes)| {
            (
                path.strip_prefix(dir).unwrap().display().to_string(),
                sha256(&bytes),
            )
        })
        .collect()
}

/// Run `inspection` and require that it left every byte of the trace directory exactly as it
/// was, including its length: no truncation, repair, replay or seal.
pub fn leaves_trace_unchanged<R>(trace: &Path, what: &str, inspection: impl FnOnce() -> R) -> R {
    let before = tree_hashes(trace);
    let result = inspection();
    let after = tree_hashes(trace);
    assert_eq!(before, after, "{what} changed the trace's bytes");
    result
}

/// The bytes after the last newline of a chain file: empty when the file ends on a line.
pub fn tail_after_last_newline(bytes: &[u8]) -> &[u8] {
    match bytes.iter().rposition(|b| *b == b'\n') {
        Some(at) => &bytes[at + 1..],
        None => bytes,
    }
}
