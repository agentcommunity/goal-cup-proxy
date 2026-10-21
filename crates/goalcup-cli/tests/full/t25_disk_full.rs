//! T25: what a run does today on a **literal** `ENOSPC`, measured and not fixed (WP-C, I3).
//!
//! The other storage rows (`capture_failure_campaign`) spend a logical quota; these fill a real
//! filesystem. Each row runs the real `goalcup` (test build), `goalcup-daemon` and
//! `goalcup-verify` with the controlled workload of `m2_synthetic_vertical` as the harness, its
//! traffic going through the recording proxy, and fills the volume that holds the configuration
//! directory, the daemon's artifacts, the run directory and the reserve with `fallocate` from a
//! second process, the way the agent's own build would.
//!
//! The rows record what happens, with timings, into `$T25_OUT/<row>/timeline.jsonl` and assert
//! almost nothing: they are a measurement. Logs, the workload's report directory and the upload
//! stand-in's store are kept off the filled volume (`$T25_OUT`), so they survive the fill.
//!
//! Linux, root, ignored. Never run them by hand on a real disk: they fill whatever volume holds
//! `TMPDIR`. Run them through `scripts/t25-disk-full.sh`, which loop-mounts a fresh ext4 image
//! (`-m 0`, so root's reserved blocks do not hide the fill), points `TMPDIR` at it and `T25_OUT`
//! off it, and unmounts and deletes the image afterwards. Each row refuses a volume above 16 GiB.
//!
//! Test-build doors used (all documented in `src/main.rs`):
//! - `GOALCUP_TEST_DISK`: the shipped sizes (3 GiB reserve, 1 GiB start margin, 256 MiB floor,
//!   the shipped chain quota) with the watch tick at 1 s instead of 30 s;
//! - `GOALCUP_TEST_UPLOAD_POLICY`: a chunk every 5 s (or at 4 MiB), heartbeats every 10 s,
//!   retries 1 to 5 s, a 60 s wrap-up;
//! - `GOALCUP_TEST_UPSTREAM_ROOTS`, `GOALCUP_TEST_NO_BROWSER`, `GOALCUP_TEST_CLONE_SOURCE`,
//!   `GOALCUP_TEST_FINISH_BUDGET_MS` as in the other full rows.
//!
//! `UPLOAD_FAILING_AFTER` (30 min) has no test door, so the stop row with a server waits it out.

use crate::common::m2_cli_support::{FAKE_GITHUB_TOKEN, PREP_REPO};
use crate::common::m2_vertical_support::{CONNECTOR, Vertical};
use crate::common::support;
use crate::common::upload_server::{ServerOptions, UploadServer};
use serde_json::{Value, json};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;

/// Under the shipped 256 MiB floor, with room for a few minutes of the stream workload.
const UNDER_FLOOR: u64 = 128 * MIB;

/// The shipped disk sizes with a 1 s watch tick.
fn disk_spec() -> String {
    format!("{},{},1000,{}", 3 * GIB, GIB, 256 * MIB)
}

/// interval, poll, backoff min, backoff max, wrap-up (ms), eager bytes, put timeout, heartbeat.
const UPLOAD_POLICY: &str = "5000,1000,1000,5000,60000,4194304,30000,10000";

// ------------------------------------------------------------------------------------ evidence

/// One row's timeline, `$T25_OUT/<row>/timeline.jsonl`, also echoed to stderr.
struct Timeline {
    file: Mutex<std::fs::File>,
    t0: Instant,
}

impl Timeline {
    fn new(dir: &Path) -> Timeline {
        std::fs::create_dir_all(dir).unwrap();
        Timeline {
            file: Mutex::new(std::fs::File::create(dir.join("timeline.jsonl")).unwrap()),
            t0: Instant::now(),
        }
    }

    fn t(&self) -> f64 {
        (self.t0.elapsed().as_millis() as f64) / 1000.0
    }

    fn note(&self, what: &str, value: Value) {
        let line = json!({"t": self.t(), "what": what, "value": value});
        eprintln!("t25 {line}");
        let mut file = self.file.lock().unwrap();
        writeln!(file, "{line}").unwrap();
        file.flush().unwrap();
    }
}

// -------------------------------------------------------------------------------------- volume

fn statvfs(path: &Path) -> (u64, u64) {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: a valid C string and a zeroed out-parameter of the right type.
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    assert_eq!(unsafe { libc::statvfs(c.as_ptr(), &mut s) }, 0, "statvfs");
    (
        s.f_blocks as u64 * s.f_frsize as u64,
        s.f_bavail as u64 * s.f_frsize as u64,
    )
}

fn free(path: &Path) -> u64 {
    statvfs(path).1
}

/// The volume under `TMPDIR`, refused unless it is small: these rows fill it.
fn small_volume() -> PathBuf {
    let tmp = PathBuf::from(
        std::env::var_os("TMPDIR")
            .expect("TMPDIR names the loop volume (scripts/t25-disk-full.sh)"),
    );
    let (total, _) = statvfs(&tmp);
    assert!(
        total < 16 * GIB,
        "TMPDIR {} is on a {} GiB volume; these rows fill it. Run scripts/t25-disk-full.sh.",
        tmp.display(),
        total / GIB
    );
    tmp
}

fn out_dir(row: &str) -> PathBuf {
    let base = PathBuf::from(std::env::var_os("T25_OUT").expect("T25_OUT, off the volume"));
    let dir = base.join(row);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A second process filling the volume: every 200 ms it `fallocate`s files until the volume's
/// free space is at most `target`, halving each request that the volume refuses, down to one
/// block. `u64::MAX` pauses it.
struct Filler {
    dir: PathBuf,
    target: Arc<AtomicU64>,
    filled: Arc<AtomicU64>,
    done: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Filler {
    fn start(volume: &Path) -> Filler {
        let dir = volume.join("t25-fill");
        std::fs::create_dir_all(&dir).unwrap();
        let target = Arc::new(AtomicU64::new(u64::MAX));
        let filled = Arc::new(AtomicU64::new(0));
        let done = Arc::new(AtomicBool::new(false));
        let (d, t, f, stop) = (dir.clone(), target.clone(), filled.clone(), done.clone());
        let thread = std::thread::spawn(move || {
            let mut n = 0u64;
            while !stop.load(Ordering::SeqCst) {
                let want = t.load(Ordering::SeqCst);
                let mut have = free(&d);
                let mut ask = have.saturating_sub(want);
                while want != u64::MAX && have > want && ask >= 4096 {
                    n += 1;
                    let ok = Command::new("fallocate")
                        .arg("-l")
                        .arg(ask.to_string())
                        .arg(d.join(format!("fill-{n:06}")))
                        .stderr(Stdio::null())
                        .status()
                        .is_ok_and(|s| s.success());
                    if ok {
                        f.fetch_add(ask, Ordering::SeqCst);
                    } else {
                        let _ = std::fs::remove_file(d.join(format!("fill-{n:06}")));
                        ask /= 2;
                        continue;
                    }
                    have = free(&d);
                    ask = have.saturating_sub(want);
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        });
        Filler {
            dir,
            target,
            filled,
            done,
            thread: Some(thread),
        }
    }

    fn fill_to(&self, bytes: u64) {
        self.target.store(bytes, Ordering::SeqCst);
    }

    /// Stop filling and delete every fill file: the space comes back at once.
    fn release(&mut self) {
        self.target.store(u64::MAX, Ordering::SeqCst);
        self.done.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Drop for Filler {
    fn drop(&mut self) {
        self.release();
    }
}

// --------------------------------------------------------------------------------------- world

/// One row's world: the vertical on the volume, the stand-in and logs off it.
struct World {
    v: Vertical,
    out: PathBuf,
    workload_out: PathBuf,
    volume: PathBuf,
    server: Option<UploadServer>,
    server_store: PathBuf,
    server_port: Option<u16>,
    clone_source: PathBuf,
    log: Timeline,
}

impl World {
    fn new(row: &str, mode: &str, extra: Value, with_server: bool) -> World {
        let volume = small_volume();
        let out = out_dir(row);
        let workload_out = out.join("workload");
        std::fs::create_dir_all(&workload_out).unwrap();
        let mut config = json!({
            "out": workload_out,
            "release": workload_out.join("release"),
            "max_s": 3 * 3600,
        });
        for (k, val) in extra.as_object().unwrap() {
            config[k] = val.clone();
        }
        let v = Vertical::new_logging_to(mode, config, Some(out.join("logs")));
        let log = Timeline::new(&out);
        let server_store = out.join("server");
        let clone_source = v.root.join("github-source.git");
        let mut world = World {
            v,
            out,
            workload_out,
            volume,
            server: None,
            server_store,
            server_port: None,
            clone_source,
            log,
        };
        if with_server {
            world.server_up();
            world.prepare();
        }
        let (total, avail) = statvfs(&world.volume);
        world.log.note(
            "world",
            json!({"volume": world.volume, "root": world.v.root, "total": total,
                   "free": avail, "server": world.server.as_ref().map(UploadServer::url)}),
        );
        world
    }

    fn server_up(&mut self) {
        let opts = ServerOptions {
            port: self.server_port,
            ..ServerOptions::default()
        };
        let server = UploadServer::start(&self.server_store, opts);
        self.server_port = Some(server.port());
        self.server = Some(server);
    }

    fn server_down(&mut self) {
        if let Some(server) = self.server.take() {
            server.stop();
        }
    }

    /// `Rig::prepare_for` for this world: logged in to the stand-in, prepared for the
    /// controlled workload's connector.
    fn prepare(&self) {
        let cfg = &self.v.cfg;
        goalcup_cli::server::write_server_config(
            cfg,
            &goalcup_cli::server::ServerConfig {
                url: self.server.as_ref().unwrap().url(),
                account: Some("member-1".to_string()),
            },
        )
        .unwrap();
        let git = Command::new("git")
            .args(["init", "--quiet", "--bare"])
            .arg(&self.clone_source)
            .output()
            .unwrap();
        assert!(git.status.success(), "{git:?}");
        let work = self.v.root.join("work");
        std::fs::create_dir_all(&work).unwrap();
        goalcup_cli::account::github::GithubSetup::new(PREP_REPO, FAKE_GITHUB_TOKEN)
            .unwrap()
            .store(cfg)
            .unwrap();
        std::fs::write(
            goalcup_cli::prep_file::prep_path(cfg),
            serde_json::to_vec(&json!({
                "harness": CONNECTOR, "repo_url": PREP_REPO, "work_root": work,
            }))
            .unwrap(),
        )
        .unwrap();
    }

    /// `goalcup` with this world's environment and every test-build door these rows use.
    fn goalcup(&self) -> Command {
        let mut command = self.v.goalcup();
        command
            .env("GOALCUP_TEST_DISK", disk_spec())
            .env("GOALCUP_TEST_UPLOAD_POLICY", UPLOAD_POLICY)
            .env(
                "GOALCUP_TEST_UPSTREAM_ROOTS",
                self.v.root.join("upstream-ca.pem"),
            )
            .env("GOALCUP_TEST_NO_BROWSER", "1")
            .env(
                goalcup_cli::commands::start::TEST_CLONE_SOURCE,
                &self.clone_source,
            );
        command
    }

    /// A long-running `goalcup` command (`start`, `continue`), its output in `logs/<name>.*`.
    fn spawn(&self, name: &str, args: &[&str]) -> Running {
        use std::os::unix::process::CommandExt;
        let logs = self.out.join("logs");
        let child = self
            .goalcup()
            .args(args)
            .current_dir(self.v.goal.parent().unwrap())
            .stdout(std::fs::File::create(logs.join(format!("{name}.out"))).unwrap())
            .stderr(std::fs::File::create(logs.join(format!("{name}.err"))).unwrap())
            .process_group(0)
            .spawn()
            .unwrap();
        self.log.note("spawn", json!({"name": name, "args": args}));
        Running {
            child,
            name: name.to_string(),
            logs,
            began: Instant::now(),
        }
    }

    fn start(&self) -> Running {
        let goal = self.v.goal.display().to_string();
        self.spawn(
            "start",
            &[
                "start",
                "--harness",
                CONNECTOR,
                "--dev",
                "--goal-file",
                &goal,
            ],
        )
    }

    /// One short command to completion, recorded whole.
    fn run(&self, what: &str, command: Command) -> (Option<i32>, String, String) {
        run_recorded(&self.log, what, command)
    }

    fn goalcup_cmd(&self, what: &str, args: &[&str]) -> (Option<i32>, String, String) {
        let mut command = self.goalcup();
        command.args(args);
        self.run(what, command)
    }

    /// `goalcup status` as the participant sees it.
    fn status_text(&self) {
        self.goalcup_cmd("goalcup status", &["status"]);
    }

    /// The signed capture status of `entry`, quietly: `None` when the command fails.
    fn capture(&self, entry: &str) -> Option<Value> {
        let output = self
            .goalcup()
            .args(["status", entry, "--json"])
            .stdin(Stdio::null())
            .output()
            .ok()?;
        let doc: Value = serde_json::from_slice(&output.stdout).ok()?;
        Some(doc["payload"]["capture"].clone())
    }

    fn verify(&self, entry: &str) -> Option<i32> {
        verify_trace(&self.log, entry, &self.v.trace(entry))
    }

    fn entry(&self) -> String {
        self.v.entry_id()
    }

    fn entries(&self) -> Vec<String> {
        let (_, out, _) = self.goalcup_cmd("goalcup entries --json", &["entries", "--json"]);
        let mut ids = Vec::new();
        collect_entry_ids(&serde_json::from_str(&out).unwrap_or(Value::Null), &mut ids);
        ids
    }

    /// Everything the row looks at, in one line.
    fn snapshot(&self, phase: &str, entry: &str) -> Value {
        let chain = chain_state(&self.v.trace(entry));
        let capture = self.capture(entry);
        let (echo_ok, echo_bad, last_echo) = echoes(&self.workload_out);
        let state = run_states(&self.v.cfg);
        let server = self.server.as_ref().map(|s| {
            let runs = s.run_ids();
            let run = runs.last().cloned().unwrap_or_default();
            json!({"run": run, "position": format!("{:?}", s.position(&run)),
                   "chunks": s.chunks(&run).len()})
        });
        let snap = json!({
            "phase": phase,
            "free": free(&self.volume),
            "reserve_allocated": reserve_allocated(&self.v.root),
            "chain": chain,
            "capture": capture.as_ref().map(|c| json!({
                "run": c["run"], "capture": c["capture"], "gap_count": c["gap_count"],
                "used_bytes": c["storage"]["used_bytes"],
                "accounting_available": c["storage"]["accounting_available"],
                "durable_bytes": c["storage"]["durable"]["chain_bytes"],
            })),
            "workload": {"alive": workload_alive(&self.workload_out), "echo_ok": echo_ok,
                         "echo_failed": echo_bad, "last_echo_age_s": last_echo},
            "state": state,
            "server": server,
        });
        self.log.note("snapshot", snap.clone());
        snap
    }

    /// Snapshots every `every` until `until` holds or `within` passes; answers whether it held.
    fn watch(
        &self,
        phase: &str,
        entry: &str,
        every: Duration,
        within: Duration,
        until: impl Fn(&Value) -> bool,
    ) -> bool {
        let end = Instant::now() + within;
        while Instant::now() < end {
            let snap = self.snapshot(phase, entry);
            if until(&snap) {
                self.log.note("watch-held", json!({"phase": phase}));
                return true;
            }
            std::thread::sleep(every);
        }
        self.log.note("watch-timeout", json!({"phase": phase}));
        false
    }

    fn release_workload(&self) {
        std::fs::write(self.workload_out.join("release"), b"").unwrap();
        self.log.note("workload released", json!({}));
    }

    fn unrelease_workload(&self) {
        let _ = std::fs::remove_file(self.workload_out.join("release"));
    }

    /// The chain parts, the reserve and the trace directory listing, for the record.
    fn listing(&self, entry: &str) {
        let trace = self.v.trace(entry);
        let mut files: Vec<Value> = std::fs::read_dir(&trace)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| {
                let meta = e.metadata().ok();
                json!({"name": e.file_name().to_string_lossy(),
                       "len": meta.as_ref().map(|m| m.len()),
                       "allocated": meta.as_ref().map(|m| m.blocks() * 512)})
            })
            .collect();
        files.sort_by_key(|f| f["name"].as_str().unwrap_or("").to_string());
        self.log
            .note("trace listing", json!({"entry": entry, "files": files}));
    }
}

/// One short command to completion, recorded whole.
fn run_recorded(log: &Timeline, what: &str, mut command: Command) -> (Option<i32>, String, String) {
    let began = Instant::now();
    let output = command.stdin(Stdio::null()).output().unwrap();
    let (stdout, stderr) = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    );
    log.note(
        what,
        json!({"exit": output.status.code(), "secs": began.elapsed().as_secs_f64(),
               "stdout": stdout, "stderr": stderr}),
    );
    (output.status.code(), stdout, stderr)
}

/// The stand-in's signing key (its default seed), so a server-anchored trace is verified with
/// its receipt accepted and only the bytes' own findings remain.
fn standin_key_hex() -> String {
    let key = ed25519_dalek::SigningKey::from_bytes(&ServerOptions::default().signing_seed);
    hex::encode(key.verifying_key().to_bytes())
}

/// `goalcup-verify --capture-integrity TRACE --dev-key <stand-in key>`.
fn verify_trace(log: &Timeline, entry: &str, trace: &Path) -> Option<i32> {
    let mut command = Command::new(support::verify_binary());
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .arg("--capture-integrity")
        .arg(trace)
        .arg("--dev-key")
        .arg(standin_key_hex());
    let (code, _, _) = run_recorded(
        log,
        &format!("goalcup-verify --capture-integrity {entry}"),
        command,
    );
    code
}

fn collect_entry_ids(value: &Value, ids: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(id)) = map.get("entry_id")
                && !ids.contains(id)
            {
                ids.push(id.clone());
            }
            map.values().for_each(|v| collect_entry_ids(v, ids));
        }
        Value::Array(items) => items.iter().for_each(|v| collect_entry_ids(v, ids)),
        _ => {}
    }
}

/// A long-running `goalcup` command.
struct Running {
    child: Child,
    name: String,
    logs: PathBuf,
    began: Instant,
}

impl Running {
    fn output(&self) -> String {
        let read = |ext: &str| {
            std::fs::read_to_string(self.logs.join(format!("{}.{ext}", self.name)))
                .unwrap_or_default()
        };
        format!("{}{}", read("out"), read("err"))
    }

    fn exited(&mut self) -> Option<std::process::ExitStatus> {
        self.child.try_wait().unwrap()
    }

    /// Wait for it to end, at most `within`; record exit, time and output.
    fn finish(&mut self, log: &Timeline, within: Duration) -> Option<i32> {
        let end = Instant::now() + within;
        while Instant::now() < end {
            if let Some(status) = self.exited() {
                log.note(
                    &format!("{} exited", self.name),
                    json!({"exit": status.code(), "secs": self.began.elapsed().as_secs_f64(),
                           "output": self.output()}),
                );
                return status.code();
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        log.note(
            &format!("{} still running", self.name),
            json!({"after_s": within.as_secs(), "output": self.output()}),
        );
        None
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            // SAFETY: two integers; the group this module started.
            unsafe { libc::kill(-(self.child.id() as i32), libc::SIGKILL) };
            let _ = self.child.wait();
        }
    }
}

// ---------------------------------------------------------------------------------- readings

/// The chain parts' total length and whether the newest part ends on a whole line.
fn chain_state(trace: &Path) -> Value {
    let mut parts: Vec<PathBuf> = std::fs::read_dir(trace)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| goalcup_platform::tracecap::parse_part_name(n).is_some())
        })
        .collect();
    parts.sort();
    let total: u64 = parts
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .sum();
    let whole_tail = parts.last().and_then(|p| {
        let mut file = std::fs::File::open(p).ok()?;
        let len = file.metadata().ok()?.len();
        if len == 0 {
            return Some(true);
        }
        file.seek(SeekFrom::Start(len - 1)).ok()?;
        let mut byte = [0u8; 1];
        file.read_exact(&mut byte).ok()?;
        Some(byte[0] == b'\n')
    });
    json!({"parts": parts.len(), "bytes": total, "tail_whole": whole_tail})
}

fn reserve_allocated(root: &Path) -> Value {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.is_dir() {
                if path.file_name().is_some_and(|n| n == "t25-fill") {
                    continue;
                }
                stack.push(path);
            } else if meta.is_file()
                && path
                    .parent()
                    .and_then(Path::file_name)
                    .is_some_and(|n| n == "reserve")
            {
                found.push(json!({"len": meta.len(), "allocated": meta.blocks() * 512}));
            }
        }
    }
    Value::Array(found)
}

/// Every run `state.json` under `cfg`: the disk, upload and recording fields.
fn run_states(cfg: &Path) -> Value {
    let mut found = Vec::new();
    let mut stack = vec![cfg.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().is_some_and(|n| n == "state.json") {
                let doc: Value = std::fs::read(&path)
                    .ok()
                    .and_then(|b| serde_json::from_slice(&b).ok())
                    .unwrap_or(Value::Null);
                found.push(json!({"disk": doc["disk"], "upload": doc["upload"],
                                  "recording": doc["recording"], "stopped": doc["stopped"]}));
            }
        }
    }
    Value::Array(found)
}

/// Echo exchanges the workload finished well and badly, and the age of its newest event.
fn echoes(out: &Path) -> (usize, usize, Option<f64>) {
    let text = std::fs::read_to_string(out.join("events.jsonl")).unwrap_or_default();
    let (mut ok, mut bad, mut last) = (0, 0, None::<f64>);
    for line in text.lines() {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if event["lane"] == "echo" {
            if event["ok"] == true {
                ok += 1;
            } else {
                bad += 1;
            }
        }
        if let Some(end) = event["end"].as_f64().or(event["start"].as_f64()) {
            last = Some(last.map_or(end, |l: f64| l.max(end)));
        }
    }
    let now = crate::common::m2_vertical_support::now();
    (ok, bad, last.map(|l| ((now - l) * 10.0).round() / 10.0))
}

fn workload_alive(out: &Path) -> bool {
    std::fs::read_to_string(out.join("pid"))
        .ok()
        .and_then(|p| p.trim().parse::<i32>().ok())
        // SAFETY: signal 0 probes a pid and sends nothing.
        .is_some_and(|pid| unsafe { libc::kill(pid, 0) } == 0)
}

fn reserve_zero(snap: &Value) -> bool {
    snap["reserve_allocated"]
        .as_array()
        .is_some_and(|r| r.iter().all(|f| f["allocated"] == 0))
}

fn capture_is(snap: &Value, state: &str) -> bool {
    snap["capture"]["capture"] == state
}

// ---------------------------------------------------------------------------------------- rows

/// S1, S3, S4 with uploads working: fill to under the floor, then to zero, then free it all;
/// then end the harness and look at `status`, the verifier, `continue` and `seal`.
#[test]
#[ignore = "fills the volume under TMPDIR: run through scripts/t25-disk-full.sh"]
fn t25_enospc_with_uploads_working() {
    let w = World::new("uploads-working", "stream", json!({}), true);
    let mut start = w.start();
    let entry = w.entry();
    w.log.note("entry", json!({"entry": entry}));
    assert!(
        w.watch(
            "warmup",
            &entry,
            Duration::from_secs(5),
            Duration::from_secs(120),
            |s| {
                s["workload"]["echo_ok"].as_u64() > Some(100)
                    && s["server"]["chunks"].as_u64() > Some(2)
            }
        ),
        "the run never reached steady traffic\n{}",
        start.output()
    );
    w.status_text();
    w.listing(&entry);

    // S1: under the floor, uploads working.
    let filler = Filler::start(&w.volume);
    filler.fill_to(UNDER_FLOOR);
    w.log.note("S1 fill", json!({"target": UNDER_FLOOR}));
    w.watch(
        "S1",
        &entry,
        Duration::from_secs(2),
        Duration::from_secs(90),
        |s| reserve_zero(s) && s["free"].as_u64() < Some(256 * MIB),
    );
    w.watch(
        "S1-hold",
        &entry,
        Duration::from_secs(5),
        Duration::from_secs(30),
        |_| false,
    );
    w.status_text();
    w.log.note(
        "S1 start still running",
        json!({"running": start.exited().is_none(),
                                                 "filled": filler.filled.load(Ordering::SeqCst)}),
    );

    // S3: to zero, uploads still working.
    filler.fill_to(0);
    w.log.note("S3 fill", json!({"target": 0}));
    let failed = w.watch(
        "S3",
        &entry,
        Duration::from_secs(2),
        Duration::from_secs(300),
        |s| capture_is(s, "failed") || s["capture"].is_null(),
    );
    w.log.note("S3 capture failed", json!({"failed": failed}));
    w.watch(
        "S3-hold",
        &entry,
        Duration::from_secs(5),
        Duration::from_secs(60),
        |_| false,
    );
    w.status_text();
    w.listing(&entry);
    w.log.note(
        "S3 start still running",
        json!({"running": start.exited().is_none(),
                                                 "filled": filler.filled.load(Ordering::SeqCst)}),
    );

    // S4: free the space while the harness runs.
    let mut filler = filler;
    filler.release();
    w.log.note("S4 freed", json!({"free": free(&w.volume)}));
    w.watch(
        "S4",
        &entry,
        Duration::from_secs(5),
        Duration::from_secs(90),
        |_| false,
    );
    w.status_text();

    // End the harness; then what the participant has.
    w.release_workload();
    start.finish(&w.log, Duration::from_secs(300));
    w.status_text();
    w.snapshot("after-start", &entry);
    w.listing(&entry);
    w.verify(&entry);
    w.unrelease_workload();
    let mut cont = w.spawn("continue", &["continue"]);
    std::thread::sleep(Duration::from_secs(30));
    let entries = w.entries();
    w.log.note("entries", json!(entries));
    if let Some(next) = entries.iter().find(|e| **e != entry) {
        w.snapshot("continued", next);
    }
    w.status_text();
    w.release_workload();
    cont.finish(&w.log, Duration::from_secs(300));
    w.status_text();
    w.goalcup_cmd("goalcup seal", &["seal", "--yes"]);
    w.status_text();
    w.goalcup_cmd("goalcup upload", &["upload"]);
    w.status_text();
    for e in w.entries() {
        w.snapshot("final", &e);
        w.listing(&e);
        w.verify(&e);
    }
}

/// S3 and S4 on a chat-mode run, the kind `goalcup continue` can go on with: the fake TUI's
/// session file (its native source) is given blocks ahead of time (`fallocate --keep-size`), so
/// the agent's own appends still land while the volume is at zero and only the recorder meets
/// `ENOSPC`. Then the space is freed, the agent is killed, and `continue` opens the next
/// segment.
#[test]
#[ignore = "fills the volume under TMPDIR: run through scripts/t25-disk-full.sh"]
fn t25_enospc_chat_run_then_continue() {
    use crate::common::m2_cli_support::Rig;
    let volume = small_volume();
    let out = out_dir("chat-continue");
    let log = Timeline::new(&out);
    let rig = Rig::chat();
    let server = UploadServer::start(&out.join("server"), ServerOptions::default());
    rig.prepare_for(&server.url());
    let disk = disk_spec();
    let env = [
        ("GOALCUP_TEST_DISK", disk.as_str()),
        ("GOALCUP_TEST_UPLOAD_POLICY", UPLOAD_POLICY),
    ];
    let goalcup = || {
        let mut command = Command::new(env!("CARGO_BIN_EXE_goalcup"));
        command
            .env_clear()
            .env("PATH", rig.path())
            .env("GOALCUP_CONFIG_DIR", &rig.cfg)
            .env("GOALCUP_CONNECTORS_DIR", &rig.capture_connectors)
            .env("GOALCUP_TEST_NO_BROWSER", "1")
            .envs(env);
        for (key, value) in rig.extra_env.lock().unwrap().iter() {
            command.env(key, value);
        }
        command
    };
    let status_text = |what: &str| {
        let mut command = goalcup();
        command.arg("status");
        run_recorded(&log, what, command);
    };
    let capture = |entry: &str| -> Value {
        let output = goalcup()
            .args(["status", entry, "--json"])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        serde_json::from_slice::<Value>(&output.stdout)
            .map(|d| d["payload"]["capture"].clone())
            .unwrap_or(Value::Null)
    };
    let snap = |phase: &str, entry: &str| -> Value {
        let c = capture(entry);
        let value = json!({
            "phase": phase, "free": free(&volume),
            "reserve_allocated": reserve_allocated(&rig.root),
            "chain": chain_state(&rig.trace(entry)),
            "capture": {"run": c["run"], "capture": c["capture"], "gap_count": c["gap_count"],
                        "used_bytes": c["storage"]["used_bytes"],
                        "durable_bytes": c["storage"]["durable"]["chain_bytes"],
                        "accounting_available": c["storage"]["accounting_available"]},
            "state": run_states(&rig.cfg),
            "workload": {"alive": false, "echo_ok": 0, "echo_failed": 0, "last_echo_age_s": null},
            "server": {"chunks": server.run_ids().last().map(|r| server.chunks(r).len()),
                       "position": server.run_ids().last()
                           .map(|r| format!("{:?}", server.position(r)))},
        });
        log.note("snapshot", value.clone());
        value
    };
    log.note(
        "world",
        json!({"volume": volume, "root": rig.root, "free": free(&volume)}),
    );

    let mut first = rig.start_on_pty(&env);
    first.wait_shown("tui: ready");
    first.type_line("hello");
    first.wait_shown("tui: noted hello");
    let entry = goalcup_cli::entries::latest_entry(&rig.cfg)
        .unwrap()
        .unwrap()
        .entry_id;
    let sessions = rig.chat_home().join("sessions");
    let session = std::fs::read_dir(&sessions)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .next()
        .expect("the session file");
    let pre = Command::new("fallocate")
        .args(["--keep-size", "-l", "64MiB"])
        .arg(&session)
        .status()
        .unwrap();
    log.note(
        "entry",
        json!({"entry": entry, "session": session, "prealloc": pre.success()}),
    );
    // Some traffic so the chain is a few hundred records long.
    let line = "x".repeat(3000);
    for n in 0..50 {
        first.type_line(&format!("warm-{n}-{line}"));
    }
    first.wait_shown("tui: noted warm-49");
    std::thread::sleep(Duration::from_secs(3));
    snap("warm", &entry);

    let mut filler = Filler::start(&volume);
    filler.fill_to(0);
    log.note("S3 fill", json!({"target": 0}));
    std::thread::sleep(Duration::from_secs(3));
    snap("S3-full", &entry);
    let mut failed_at = None;
    for n in 0..200 {
        first.type_line(&format!("full-{n}-{line}"));
        std::thread::sleep(Duration::from_millis(100));
        if n % 10 == 9 {
            let s = snap("S3", &entry);
            if s["capture"]["capture"] == "failed" {
                failed_at = Some(n);
                break;
            }
        }
    }
    log.note("S3 capture failed", json!({"after_lines": failed_at}));
    first.type_line("after-failure");
    first.wait_shown("tui: noted after-failure");
    log.note("agent still answers", json!({}));
    status_text("goalcup status (full)");
    snap("S3-hold", &entry);

    filler.release();
    log.note("S4 freed", json!({"free": free(&volume)}));
    for n in 0..6 {
        first.type_line(&format!("freed-{n}-{line}"));
        std::thread::sleep(Duration::from_secs(5));
        snap("S4", &entry);
    }
    status_text("goalcup status (freed)");

    let pid: i32 = rig.harness_file("tui.pid").unwrap().trim().parse().unwrap();
    // SAFETY: two integers; the fake agent's own pid.
    unsafe { libc::kill(pid, libc::SIGKILL) };
    log.note("agent killed", json!({"pid": pid}));
    let began = Instant::now();
    let code = first.wait();
    log.note(
        "start exited",
        json!({"exit": code.code(), "secs": began.elapsed().as_secs_f64(),
               "output": first.shown()}),
    );
    status_text("goalcup status (after start)");
    snap("after-start", &entry);
    verify_trace(&log, &entry, &rig.trace(&entry));

    let mut second = rig.continue_on_pty(&env, None);
    let began = Instant::now();
    for _ in 0..600 {
        let shown = second.shown();
        if shown.contains("tui: ready") || shown.contains("goalcup:") {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let ready = second.shown().contains("tui: ready");
    log.note(
        "continue shown",
        json!({"ready": ready, "secs": began.elapsed().as_secs_f64(),
               "output": second.shown()}),
    );
    if ready {
        second.type_line("continued");
        second.wait_shown("tui: noted continued");
        std::thread::sleep(Duration::from_secs(3));
        if let Some(next) = goalcup_cli::entries::latest_entry(&rig.cfg)
            .unwrap()
            .map(|e| e.entry_id)
            .filter(|e| *e != entry)
        {
            snap("continued", &next);
        }
        second.type_line("/exit");
    }
    let code = second.wait();
    log.note(
        "continue exited",
        json!({"exit": code.code(), "output": second.shown()}),
    );
    status_text("goalcup status (after continue)");
    let mut seal = goalcup();
    seal.args(["seal", "--yes"]);
    run_recorded(&log, "goalcup seal", seal);
    status_text("goalcup status (after seal)");
    for run in server.run_ids() {
        for e in server.entry_ids(&run) {
            snap("final", &e);
            verify_trace(&log, &e, &rig.trace(&e));
            log.note(
                "server copy",
                json!({"entry": e, "position": format!("{:?}", server.position_of(&e)),
                       "sealed": server.seal_of(&e).is_some()}),
            );
        }
    }
}

/// S2 without a server: a run with nothing to upload counts its uploads as failing at once, so
/// this is the stop verdict's own path, reached in seconds. `T25_LOCAL_TARGET_MIB` sets the fill
/// (default 128 MiB, under the floor; 0 races the stop against a literal ENOSPC).
#[test]
#[ignore = "fills the volume under TMPDIR: run through scripts/t25-disk-full.sh"]
fn t25_stop_verdict_local_run() {
    let target = std::env::var("T25_LOCAL_TARGET_MIB")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(128)
        * MIB;
    let row = format!("local-stop-{}mib", target / MIB);
    let w = World::new(&row, "stream", json!({}), false);
    let mut start = w.start();
    let entry = w.entry();
    w.log.note("entry", json!({"entry": entry}));
    w.watch(
        "warmup",
        &entry,
        Duration::from_secs(5),
        Duration::from_secs(120),
        |s| s["workload"]["echo_ok"].as_u64() > Some(100),
    );
    w.status_text();
    let mut filler = Filler::start(&w.volume);
    filler.fill_to(target);
    w.log.note("S2 fill", json!({"target": target}));
    let end = Instant::now() + Duration::from_secs(300);
    while Instant::now() < end && start.exited().is_none() {
        w.snapshot("S2", &entry);
        std::thread::sleep(Duration::from_secs(2));
    }
    start.finish(&w.log, Duration::from_secs(300));
    w.snapshot("stopped-while-full", &entry);
    w.status_text();
    w.listing(&entry);
    w.verify(&entry);
    filler.release();
    w.log.note("freed", json!({"free": free(&w.volume)}));
    w.status_text();
    w.unrelease_workload();
    let mut cont = w.spawn("continue", &["continue"]);
    std::thread::sleep(Duration::from_secs(20));
    w.status_text();
    w.release_workload();
    cont.finish(&w.log, Duration::from_secs(300));
    w.goalcup_cmd("goalcup seal", &["seal", "--yes"]);
    w.status_text();
    for e in w.entries() {
        w.snapshot("final", &e);
        w.verify(&e);
    }
}

/// S2 with a server that goes away: a trickling harness (so the chain does not reach ENOSPC first),
/// the stand-in stopped, the volume under the floor, and a wait for the 30-minute failing-upload
/// window to stop the run. Then the server comes back, the space is freed, and `continue`.
#[test]
#[ignore = "fills the volume under TMPDIR and waits 30 min: run through scripts/t25-disk-full.sh"]
fn t25_stop_verdict_server_down() {
    // A trickle, not silence: an uploader with nothing waiting counts as working
    // (`uploader.rs`, "Nothing waiting is an upload with nothing to complain of"), so a silent
    // harness would never reach the verdict. One 256-byte SSE event a second keeps a little
    // waiting without spending the volume's last 128 MiB in 30 minutes.
    let mut w = World::new(
        "server-down-stop",
        "sparse",
        json!({"events": 4000, "interval_ms": 1000, "hold": true}),
        true,
    );
    let mut start = w.start();
    let entry = w.entry();
    w.log.note("entry", json!({"entry": entry}));
    w.watch(
        "warmup",
        &entry,
        Duration::from_secs(5),
        Duration::from_secs(120),
        |s| s["server"]["chunks"].as_u64() > Some(1),
    );
    std::thread::sleep(Duration::from_secs(15));
    w.snapshot("before-outage", &entry);
    w.server_down();
    w.log.note("server down", json!({}));
    let filler = Filler::start(&w.volume);
    filler.fill_to(UNDER_FLOOR);
    w.log.note("S2 fill", json!({"target": UNDER_FLOOR}));
    let end = Instant::now() + Duration::from_secs(40 * 60);
    let mut last_text = Instant::now();
    while Instant::now() < end && start.exited().is_none() {
        w.snapshot("S2", &entry);
        if last_text.elapsed() > Duration::from_secs(300) {
            w.status_text();
            last_text = Instant::now();
        }
        std::thread::sleep(Duration::from_secs(20));
    }
    start.finish(&w.log, Duration::from_secs(300));
    w.snapshot("stopped", &entry);
    w.status_text();
    w.listing(&entry);
    w.verify(&entry);
    let mut filler = filler;
    filler.release();
    w.server_up();
    w.log
        .note("server up, space freed", json!({"free": free(&w.volume)}));
    std::thread::sleep(Duration::from_secs(10));
    w.status_text();
    let mut cont = w.spawn("continue", &["continue"]);
    std::thread::sleep(Duration::from_secs(20));
    w.status_text();
    w.release_workload();
    cont.finish(&w.log, Duration::from_secs(300));
    w.goalcup_cmd("goalcup seal", &["seal", "--yes"]);
    w.status_text();
    for e in w.entries() {
        w.snapshot("final", &e);
        w.verify(&e);
    }
}
