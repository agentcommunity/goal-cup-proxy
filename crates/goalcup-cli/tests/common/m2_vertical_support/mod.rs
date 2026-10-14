//! Support for `m2_synthetic_vertical.rs`: one test's world, every part of it a real process or
//! socket. The shipped `goalcup-daemon` binary; the `goalcup` binary of this test build, whose
//! one test-only input is `GOALCUP_TEST_UPSTREAM_ROOTS`; the controlled TLS endpoint,
//! served from this process; and the controlled workload
//! (`fixtures/m2-controlled-workload.py`) as the capture executable of a development connector
//! written from `fixtures/m2-controlled-workload/harness.toml`.
//!
//! What the workload sent is known from its own report, outside the recorder; what was
//! retained is read back from the chain file line by line. Every wait is bounded, and every
//! child is ended by its owner on drop.

mod wire;

mod endpoint;

use base64::Engine as _;
use endpoint::{API_HOST, AUTH_HOST, ControlledEndpoint, EndpointConfig};
use goalcup_protocol::capture::{
    CaptureMeta, CaptureRecord, CaptureSource, CaptureTime, EndReason, GapReason, StreamKey,
};
use goalcup_protocol::control::SignedStatus;
use goalcup_protocol::envelope::{Envelope, EventBody};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// The development connector's id.
pub const CONNECTOR: &str = "m2-controlled-workload";

/// The logical size a chain may grow to in every run but the small-quota ones.
pub const RESERVE: u64 = 64 * 1024 * 1024;

/// What the run itself holds on disk (its one reservation) in every test: far below the 3 GiB a
/// participant's run holds.
const RUN_HELD: u64 = 4 * 1024 * 1024;

/// `GOALCUP_TEST_DISK` for a run whose chains are limited to `quota` bytes.
fn disk_spec(quota: u64) -> String {
    format!(
        "{RUN_HELD},0,50,{},{quota}",
        goalcup_cli::run::reserve::FREE_FLOOR
    )
}

/// The one goal every run is given.
pub const GOAL: &str = "exchange the controlled traffic plan through the recording proxy";

/// The E02 sentinel's prefix, built at run time like the load's, so no source holds it.
pub fn sentinel_prefix() -> Vec<u8> {
    [&b"sk-"[..], b"ant-"].concat()
}

/// The ceiling on a wait for a process or file this module started.
const PATIENCE: Duration = Duration::from_secs(60);

/// One test's world.
pub struct Vertical {
    pub root: PathBuf,
    pub cfg: PathBuf,
    pub connectors: PathBuf,
    pub artifacts: PathBuf,
    /// The workload's report directory, outside everything the recorder reads.
    pub out: PathBuf,
    /// Every child's standard output and error, and every command's.
    pub logs: PathBuf,
    pub goal: PathBuf,
    upstream_pem: PathBuf,
    runtime: tokio::runtime::Runtime,
    endpoint: Option<ControlledEndpoint>,
    daemon: Option<Child>,
    /// The daemon's `upload_url` listens here and nothing may connect to it.
    upload_trap: std::net::TcpListener,
    _tmp: tempfile::TempDir,
}

/// The finish budget the vertical's `goalcup` processes work within, in milliseconds.
pub const TEST_FINISH_BUDGET_MS: u64 = 10_000;

impl Vertical {
    /// A daemon, an endpoint and a connector whose workload runs `mode`, with `extra` merged
    /// into its configuration.
    pub fn new(mode: &str, extra: Value) -> Vertical {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let dir = |name: &str| {
            let path = root.join(name);
            std::fs::create_dir_all(&path).unwrap();
            path
        };
        let (cfg, connectors, artifacts, out, logs) = (
            dir("cfg"),
            dir("connectors"),
            dir("artifacts"),
            dir("out"),
            dir("logs"),
        );
        let (workspaces, home) = (dir("workspaces"), dir("home"));
        let goal = dir("runs").join("goal.txt");
        std::fs::write(&goal, GOAL).unwrap();

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let endpoint = runtime
            .block_on(ControlledEndpoint::start(EndpointConfig {
                api_host: API_HOST.to_string(),
                auth_host: AUTH_HOST.to_string(),
                test_id: "t23-vertical".to_string(),
            }))
            .unwrap();
        let upstream_pem = root.join("upstream-ca.pem");
        std::fs::write(&upstream_pem, &endpoint.ca_pem).unwrap();

        let mut config = serde_json::json!({
            "mode": mode,
            "port": endpoint.port(),
            "test_id": "t23-vertical",
            "upstream_ca": upstream_pem,
            "out": out,
            "release": out.join("release"),
            "max_s": 150,
        });
        for (k, v) in extra.as_object().unwrap() {
            config[k] = v.clone();
        }
        let config_path = root.join("workload.json");
        std::fs::write(&config_path, config.to_string()).unwrap();
        write_connector(&connectors, &config_path);

        let upload_trap = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        upload_trap.set_nonblocking(true).unwrap();
        let daemon_toml = root.join("goalcup-daemon.toml");
        std::fs::write(
            &daemon_toml,
            format!(
                "artifact_root = \"{}\"\nconnectors_dir = \"{}\"\nconfig_dir = \"{}\"\n\
                 workspace_parent = \"{}\"\ncontrol_addr = \"127.0.0.1:0\"\n\
                 upload_url = \"http://{}\"\nserver_public_key = \"{}\"\n\
                 upload_interval_ms = 100\nstatus_interval_ms = 1000\n",
                artifacts.display(),
                connectors.display(),
                cfg.display(),
                workspaces.display(),
                upload_trap.local_addr().unwrap(),
                super::support::TEST_SERVER_PUBLIC_KEY_HEX,
            ),
        )
        .unwrap();
        let daemon = Command::new(super::support::daemon_binary())
            .arg("--config")
            .arg(&daemon_toml)
            .arg("--foreground")
            .env_clear()
            .env("HOME", &home)
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .stdin(Stdio::null())
            .stdout(log(&logs, "daemon.out"))
            .stderr(log(&logs, "daemon.err"))
            .spawn()
            .unwrap();
        let vertical = Vertical {
            root,
            cfg,
            connectors,
            artifacts,
            out,
            logs,
            goal,
            upstream_pem,
            runtime,
            endpoint: Some(endpoint),
            daemon: Some(daemon),
            upload_trap,
            _tmp: tmp,
        };
        // The daemon prints its control address and control file once it serves.
        wait_until("the daemon's startup lines", || {
            std::fs::read_to_string(vertical.logs.join("daemon.out"))
                .is_ok_and(|text| text.lines().count() >= 2)
        });
        vertical
    }

    /// `goalcup` with only this world's directories in its environment.
    fn goalcup(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_goalcup"));
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env("HOME", self.root.join("home"))
            .env(goalcup_cli::config::CONFIG_DIR_ENV, &self.cfg)
            .env(goalcup_cli::config::CONNECTORS_DIR_ENV, &self.connectors)
            // These tests cut a finish that cannot complete; the shipped budget is three
            // minutes (D56), a test's is ten seconds.
            .env(
                "GOALCUP_TEST_FINISH_BUDGET_MS",
                TEST_FINISH_BUDGET_MS.to_string(),
            )
            .stdin(Stdio::null());
        for key in ["USER", "LOGNAME", "TMPDIR"] {
            if let Ok(value) = std::env::var(key) {
                command.env(key, value);
            }
        }
        command
    }

    /// `goalcup start --harness m2-controlled-workload --dev --goal-file GOAL`, with chains
    /// limited to `reserve` bytes (`GOALCUP_TEST_DISK`), in a process group of its own, which
    /// the harness joins.
    pub fn start(&self, reserve: u64) -> Launcher {
        self.start_with(disk_spec(reserve), None)
    }

    /// [`Vertical::start`] with `GOALCUP_TEST_PAUSE_DIR` naming `pause_dir`: the launcher holds
    /// at each point whose `<point>.arm` file is there (see `commands/start.rs::pause_at`).
    pub fn start_paused(&self, reserve: u64, pause_dir: &Path) -> Launcher {
        self.start_with(disk_spec(reserve), Some(pause_dir))
    }

    /// [`Vertical::start_paused`] with a test build's wrap-up for a run without a server
    /// shortened to `wrap_up_ms` (`GOALCUP_TEST_LOCAL_WRAP_UP`).
    pub fn start_paused_short_wrap_up(
        &self,
        reserve: u64,
        pause_dir: &Path,
        wrap_up_ms: u64,
    ) -> Launcher {
        self.start_launcher(disk_spec(reserve), Some(pause_dir), Some(wrap_up_ms))
    }

    fn start_with(&self, disk: String, pause_dir: Option<&Path>) -> Launcher {
        self.start_launcher(disk, pause_dir, None)
    }

    fn start_launcher(
        &self,
        disk: String,
        pause_dir: Option<&Path>,
        wrap_up_ms: Option<u64>,
    ) -> Launcher {
        use std::os::unix::process::CommandExt;
        let mut command = self.goalcup();
        command
            .args(["start", "--harness", CONNECTOR, "--dev", "--goal-file"])
            .arg(&self.goal)
            .env("GOALCUP_TEST_DISK", disk)
            .env("GOALCUP_TEST_UPSTREAM_ROOTS", &self.upstream_pem);
        if let Some(dir) = pause_dir {
            command.env("GOALCUP_TEST_PAUSE_DIR", dir);
        }
        if let Some(ms) = wrap_up_ms {
            command.env("GOALCUP_TEST_LOCAL_WRAP_UP", ms.to_string());
        }
        let child = command
            .stdout(log(&self.logs, "start.out"))
            .stderr(log(&self.logs, "start.err"))
            .process_group(0)
            .spawn()
            .unwrap();
        Launcher {
            child,
            logs: self.logs.clone(),
        }
    }

    /// One `goalcup` command to completion, its output kept under `logs/`.
    fn run(&self, name: &str, args: &[&str]) -> (ExitStatus, String) {
        let output = self.goalcup().args(args).output().unwrap();
        let n = std::fs::read_dir(&self.logs).unwrap().count();
        std::fs::write(self.logs.join(format!("{n:03}-{name}.out")), &output.stdout).unwrap();
        std::fs::write(self.logs.join(format!("{n:03}-{name}.err")), &output.stderr).unwrap();
        (
            output.status,
            String::from_utf8_lossy(&output.stdout).into_owned(),
        )
    }

    /// `goalcup status ENTRY --json`, the signed document parsed.
    pub fn status(&self, entry: &str) -> SignedStatus {
        let (status, out) = self.run("status", &["status", entry, "--json"]);
        assert!(status.success(), "goalcup status: {status}\n{out}");
        serde_json::from_str(&out).unwrap()
    }

    /// `goalcup seal ENTRY`: its exit, what it printed and how long it took.
    pub fn seal(&self, entry: &str) -> (ExitStatus, String, Duration) {
        let began = Instant::now();
        let (status, out) = self.run("seal", &["seal", entry]);
        (status, out, began.elapsed())
    }

    /// `goalcup-verify --capture-integrity TRACE`: its exit and its report.
    pub fn verify(&self, entry: &str) -> (i32, Value) {
        let output = subprocess(super::support::verify_binary())
            .arg("--capture-integrity")
            .arg(self.trace(entry))
            .output()
            .unwrap();
        std::fs::write(self.logs.join("verify.out"), &output.stdout).unwrap();
        (
            output.status.code().unwrap(),
            serde_json::from_slice(&output.stdout).unwrap(),
        )
    }

    /// `goalcup-verify --research-json TRACE --output REPORT`: its exit and summary line.
    pub fn research(&self, entry: &str) -> (i32, Value) {
        let report = self.root.join("research.ndjson");
        let status = subprocess(super::support::verify_binary())
            .arg("--research-json")
            .arg(self.trace(entry))
            .arg("--output")
            .arg(&report)
            .status()
            .unwrap();
        let text = std::fs::read_to_string(&report).unwrap();
        let summary = serde_json::from_str(text.lines().last().unwrap()).unwrap();
        (status.code().unwrap(), summary)
    }

    /// The entry `goalcup start` recorded on this machine, once it has.
    pub fn entry_id(&self) -> String {
        wait_until("the entry record", || {
            goalcup_cli::entries::latest_entry(&self.cfg)
                .ok()
                .flatten()
                .is_some()
        });
        goalcup_cli::entries::latest_entry(&self.cfg)
            .unwrap()
            .unwrap()
            .entry_id
    }

    pub fn trace(&self, entry: &str) -> PathBuf {
        self.artifacts.join("artifacts").join(entry).join("trace")
    }

    pub fn chain(&self, entry: &str) -> Chain {
        Chain::read(&self.trace(entry).join("chain.jsonl"))
    }

    /// The run directory `goalcup start` made beside the goal file.
    pub fn run_dir(&self) -> Option<PathBuf> {
        std::fs::read_dir(self.goal.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .find(|p| p.is_dir())
    }

    /// The native file the workload wrote under its run `HOME`.
    pub fn native_file(&self) -> Vec<u8> {
        let dir = self.run_dir().expect("a run directory");
        std::fs::read(dir.join("home/native/e02-native.jsonl")).unwrap()
    }

    /// Wait for the workload to write `name` into its report directory.
    pub fn wait_for(&self, name: &str) {
        let path = self.out.join(name);
        wait_until(&format!("the workload's {name}"), || path.exists());
    }

    /// End the workload's hold or stream.
    pub fn release(&self) {
        std::fs::write(self.out.join("release"), b"").unwrap();
    }

    pub fn harness_pid(&self) -> u32 {
        self.wait_for("pid");
        std::fs::read_to_string(self.out.join("pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    }

    pub fn report(&self) -> Value {
        serde_json::from_slice(&std::fs::read(self.out.join("report.json")).unwrap()).unwrap()
    }

    /// The stream mode's exchanges so far, in the order they finished.
    pub fn events(&self) -> Vec<Value> {
        std::fs::read_to_string(self.out.join("events.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    /// Wait until the stream mode has finished `n` good echoes.
    pub fn wait_for_echoes(&self, n: usize) {
        wait_until(&format!("{n} echoes"), || {
            self.events()
                .iter()
                .filter(|e| e["lane"] == "echo" && e["ok"] == true)
                .count()
                >= n
        });
    }

    /// How many connections the daemon's `upload_url` accepted.
    pub fn upload_dials(&self) -> usize {
        let mut n = 0;
        while self.upload_trap.accept().is_ok() {
            n += 1;
        }
        n
    }

    pub fn daemon_pid(&self) -> i32 {
        self.daemon.as_ref().unwrap().id() as i32
    }

    /// The controlled endpoint's own request counters: what reached the upstream, counted
    /// outside the workload, the proxy and the recorder. Only the crash campaign reads it.
    pub fn endpoint_counters(&self) -> endpoint::EndpointCounters {
        self.endpoint.as_ref().expect("the endpoint").counters()
    }
}

fn subprocess(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin");
    command
}

impl Drop for Vertical {
    fn drop(&mut self) {
        // A workload a failed test left running: ended only while its pid still runs Python.
        if let Some(pid) = std::fs::read_to_string(self.out.join("pid"))
            .ok()
            .and_then(|p| p.trim().parse::<u32>().ok())
            && goalcup_protocol::process::snapshot().is_ok_and(|table| {
                table
                    .iter()
                    .any(|r| r.pid == pid && r.comm.to_lowercase().contains("python"))
            })
        {
            // SAFETY: two integers; the pid was just seen running Python.
            unsafe { libc::kill(pid as i32, libc::SIGKILL) };
        }
        if let Some(mut daemon) = self.daemon.take() {
            // SAFETY: two integers, a child this module started. CONT first, in case a test
            // stopped it and failed before continuing it.
            unsafe {
                libc::kill(daemon.id() as i32, libc::SIGCONT);
                libc::kill(daemon.id() as i32, libc::SIGTERM);
            }
            let until = Instant::now() + Duration::from_secs(15);
            while daemon.try_wait().ok().flatten().is_none() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(50));
            }
            let _ = daemon.kill();
            let _ = daemon.wait();
        }
        if let Some(endpoint) = self.endpoint.take() {
            self.runtime
                .block_on(endpoint.shutdown(Duration::from_secs(2)));
        }
    }
}

/// `goalcup start`, running.
pub struct Launcher {
    child: Child,
    logs: PathBuf,
}

impl Launcher {
    pub fn pid(&self) -> i32 {
        self.child.id() as i32
    }

    pub fn exited(&mut self) -> bool {
        self.child.try_wait().unwrap().is_some()
    }

    /// Its exit, within `within`.
    pub fn wait(&mut self, within: Duration) -> ExitStatus {
        let until = Instant::now() + within;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(
                Instant::now() < until,
                "goalcup start did not end within {within:?}\n{}",
                self.output()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// What it printed, both streams.
    pub fn output(&self) -> String {
        let read = |name| std::fs::read_to_string(self.logs.join(name)).unwrap_or_default();
        format!("{}{}", read("start.out"), read("start.err"))
    }

    /// `kill -9` of this process alone: the harness in its group is left running.
    pub fn kill9(&mut self) {
        // SAFETY: two integers, a child this module started and has not reaped.
        assert_eq!(unsafe { libc::kill(self.pid(), libc::SIGKILL) }, 0);
        self.child.wait().unwrap();
    }
}

impl Drop for Launcher {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// The fixture harness, with the workload and its configuration named.
fn write_connector(connectors: &Path, config: &Path) {
    // Schema 1 refuses tracker-named paths in launch arguments. Give the workload a neutral
    // private spelling while preserving its source location and relative helper layout through
    // symlinks. The vertical's file-leak sweep deliberately does not follow these links.
    let fixture_root = connectors
        .parent()
        .expect("the connectors root has a parent");
    let workload = fixture_root.join("a/b/c/d/controlled-workload.py");
    std::fs::create_dir_all(workload.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(
        super::support::fixture("m2-controlled-workload.py"),
        &workload,
    )
    .unwrap();
    std::os::unix::fs::symlink(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spikes"),
        fixture_root.join("spikes"),
    )
    .unwrap();
    let render = |name: &str| {
        let fixture = format!("m2-controlled-workload/{name}");
        std::fs::read_to_string(super::support::fixture(&fixture))
            .unwrap()
            .replace("@WORKLOAD@", &workload.display().to_string())
            .replace("@CONFIG@", &config.display().to_string())
    };
    let dir = connectors.join(CONNECTOR);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("harness.toml"), render("harness.toml")).unwrap();
}

fn log(dir: &Path, name: &str) -> std::fs::File {
    std::fs::File::create(dir.join(name)).unwrap()
}

/// Poll `condition` every 50 ms until it holds, or fail naming `what`.
pub fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let until = Instant::now() + PATIENCE;
    while !condition() {
        assert!(Instant::now() < until, "waited {PATIENCE:?} for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// `2026-09-30T12:34:56.123456789Z` as seconds since the epoch.
pub fn wall(text: &str) -> f64 {
    let frac = text
        .trim_end_matches('Z')
        .split_once('.')
        .map_or("0", |(_, f)| f);
    goalcup_protocol::control::parse_z(text).unwrap() as f64
        + format!("0.{frac}").parse::<f64>().unwrap()
}

/// This host's wall clock as seconds since the epoch.
pub fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

/// One chain line.
pub struct Line {
    pub seq: u64,
    pub recorded_at: String,
    pub protocol_version: u32,
    pub record: Option<CaptureRecord>,
}

/// A chain file as it stands on disk.
pub struct Chain {
    pub bytes: Vec<u8>,
    pub lines: Vec<Line>,
}

/// One stream, reassembled from its records.
pub struct Stream {
    pub source: CaptureSource,
    pub meta: CaptureMeta,
    /// The retained, redacted bytes.
    pub data: Vec<u8>,
    /// Source bytes the segments cover, before redaction.
    pub source_len: u64,
    pub audit: Vec<String>,
    pub end: Option<EndReason>,
    pub gaps: Vec<GapReason>,
    /// Each segment's chain seq, the capture time of its first byte, and its append time.
    pub segments: Vec<(u64, CaptureTime, String)>,
}

impl Chain {
    pub fn read(path: &Path) -> Chain {
        let bytes = std::fs::read(path).unwrap();
        // Complete lines only: a chain read while it is appended to may end mid-line.
        let lines = bytes
            .split_inclusive(|b| *b == b'\n')
            .filter(|l| l.ends_with(b"\n"))
            .map(|l| {
                let envelope: Envelope = serde_json::from_slice(l).unwrap();
                Line {
                    seq: envelope.link.seq,
                    recorded_at: envelope.recorded_at,
                    protocol_version: envelope.protocol_version,
                    record: match envelope.body {
                        EventBody::Capture { record } => Some(record),
                        _ => None,
                    },
                }
            })
            .collect();
        Chain { bytes, lines }
    }

    pub fn sha256(&self) -> String {
        goalcup_daemon::workspace::sha256_hex(&self.bytes)
    }

    pub fn records(&self) -> impl Iterator<Item = &CaptureRecord> {
        self.lines.iter().filter_map(|l| l.record.as_ref())
    }

    pub fn count(&self, pick: impl Fn(&CaptureRecord) -> bool) -> usize {
        self.records().filter(|r| pick(r)).count()
    }

    /// Every stream, keyed as the chain keys it.
    pub fn streams(&self) -> BTreeMap<StreamKey, Stream> {
        let mut streams = BTreeMap::new();
        for line in &self.lines {
            match &line.record {
                Some(CaptureRecord::Open {
                    stream,
                    source,
                    metadata,
                    ..
                }) => {
                    streams.insert(
                        stream.clone(),
                        Stream {
                            source: *source,
                            meta: metadata.clone(),
                            data: Vec::new(),
                            source_len: 0,
                            audit: Vec::new(),
                            end: None,
                            gaps: Vec::new(),
                            segments: Vec::new(),
                        },
                    );
                }
                Some(CaptureRecord::Segment {
                    stream,
                    source_len,
                    first,
                    data_b64,
                    audit,
                    ..
                }) => {
                    let s = streams.get_mut(stream).unwrap();
                    s.data.extend(
                        base64::engine::general_purpose::STANDARD
                            .decode(data_b64)
                            .unwrap(),
                    );
                    s.source_len += source_len;
                    s.audit.extend(audit.spans.iter().map(|a| a.kind.clone()));
                    s.segments
                        .push((line.seq, first.clone(), line.recorded_at.clone()));
                }
                Some(CaptureRecord::End { stream, reason, .. }) => {
                    streams.get_mut(stream).unwrap().end = Some(*reason);
                }
                Some(CaptureRecord::Gap {
                    stream: Some(stream),
                    reason,
                    ..
                }) => {
                    streams.get_mut(stream).unwrap().gaps.push(*reason);
                }
                _ => {}
            }
        }
        streams
    }

    /// Every producer that has a record of its own in the chain.
    pub fn producers(&self) -> std::collections::BTreeSet<String> {
        self.records()
            .filter_map(|r| match r {
                CaptureRecord::Open { stream, .. }
                | CaptureRecord::Segment { stream, .. }
                | CaptureRecord::End { stream, .. } => Some(stream.producer.clone()),
                CaptureRecord::Connection { producer, .. }
                | CaptureRecord::Recovery { producer, .. }
                | CaptureRecord::ProducerClosed { producer, .. } => Some(producer.clone()),
                _ => None,
            })
            .collect()
    }

    /// The producer of the proxy's first captured request: the launch's first wire epoch.
    pub fn first_wire_producer(&self) -> String {
        self.records()
            .find_map(|r| match r {
                CaptureRecord::Open {
                    stream,
                    source: CaptureSource::WireRequest,
                    ..
                } => Some(stream.producer.clone()),
                _ => None,
            })
            .expect("a captured request")
    }
}

impl Stream {
    /// Ended complete, with no gap.
    pub fn clean(&self) -> bool {
        self.end == Some(EndReason::Complete) && self.gaps.is_empty()
    }

    pub fn sha256(&self) -> String {
        goalcup_daemon::workspace::sha256_hex(&self.data)
    }
}

/// Every file under `dir`, with its bytes.
pub fn files_under(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let meta = std::fs::symlink_metadata(entry.path()).unwrap();
            if meta.is_dir() {
                stack.push(entry.path());
            } else if meta.is_file() {
                found.push((entry.path(), std::fs::read(entry.path()).unwrap()));
            }
        }
    }
    found
}

/// The proxy credential's shape wherever it could leak: userinfo in a proxy URL, its Basic
/// encoding, or the header that would carry it.
pub fn holds_proxy_credential(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes);
    text.match_indices("goalcup:").any(|(i, _)| {
        let rest = &text[i + "goalcup:".len()..];
        let hex = rest.chars().take_while(char::is_ascii_hexdigit).count();
        hex >= 16 && rest[hex..].starts_with('@')
    }) || text.contains("Z29hbGN1cDo")
        || text.to_ascii_lowercase().contains("proxy-authorization")
}

#[cfg(test)]
mod subprocess_environment_tests {
    use super::*;

    #[test]
    fn child_commands_receive_only_the_minimal_path() {
        let output = subprocess("/usr/bin/env").output().unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            "PATH=/usr/bin:/bin:/usr/sbin:/sbin\n"
        );
    }
}
