//! A real daemon with no receipt service behind it, the v5 capture control calls, and
//! readers for what a capture entry leaves on disk.
//!
//! The configured upload URL names a loopback listener that is bound and never accepted on:
//! [`Rig::upload_dials`] counts every connection made to it, so "needs no server" is an
//! observation rather than the absence of a stub.

use super::support::control::{
    EmptyControlHarness, Response, empty_control_harness_on, restart_empty_control,
};
use super::table_lock::TableLock;
use goalcup_daemon::config::DaemonConfig;
use goalcup_protocol::capture::{
    CaptureFrame, CaptureRecord, CaptureTime, FrameEvent, GapReason, IngestMessage, StreamKey,
    encode_ingest_message,
};
use goalcup_protocol::control::{
    CaptureGoalRequest, CaptureLaunchRequest, CaptureSealRequest, CaptureWorkspaceInfo,
    CaptureWorkspaceRequest, ProxyCounters, ROUTE_CAPTURE_WORKSPACES, SUFFIX_ABANDON,
    SUFFIX_CAPTURE_GOAL, SUFFIX_CAPTURE_LAUNCH, SUFFIX_PROCESS, SUFFIX_SEAL, SUFFIX_STATUS,
    SignedStatus, entry_route,
};
use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::process::ProcessRegistration;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// A small chain quota: 4 MiB, the 1 MiB terminal allowance included.
pub const RESERVE: u64 = 4 * 1024 * 1024;

/// An empty daemon whose upload URL is the trap.
pub struct Rig {
    h: EmptyControlHarness,
    upload_trap: std::net::TcpListener,
}

impl std::ops::Deref for Rig {
    type Target = EmptyControlHarness;

    fn deref(&self) -> &EmptyControlHarness {
        &self.h
    }
}

impl Rig {
    pub async fn start() -> Rig {
        Rig::start_with(|_| {}).await
    }

    /// [`Rig::start`] with one edit to the configuration before the daemon starts.
    pub async fn start_with(edit: impl FnOnce(&mut DaemonConfig)) -> Rig {
        let upload_trap = std::net::TcpListener::bind("127.0.0.1:0").expect("a loopback port");
        upload_trap
            .set_nonblocking(true)
            .expect("a non-blocking trap");
        let upload_url = format!("http://{}", upload_trap.local_addr().unwrap());
        // Nothing but the trap listens at the upload URL.
        let h = empty_control_harness_on(&upload_url, edit).await;
        Rig { h, upload_trap }
    }

    /// Stop this daemon and start another over the same root and configuration.
    pub async fn restart(self) -> Rig {
        Rig {
            h: restart_empty_control(self.h).await,
            upload_trap: self.upload_trap,
        }
    }

    /// Stop this daemon, put rows into its registry, and start another over the same root.
    /// The start's own answer is returned: a refusal is not a panic.
    pub async fn restart_planting(
        self,
        plant: impl FnOnce(&goalcup_daemon::registry::Registry),
    ) -> Result<Rig, goalcup_daemon::DaemonError> {
        let h = super::support::control::restart_empty_control_planting(self.h, plant).await?;
        Ok(Rig {
            h,
            upload_trap: self.upload_trap,
        })
    }

    /// A workspace path under this rig that does not exist yet.
    pub fn workspace(&self, name: &str) -> PathBuf {
        self.root.join("workspaces").join(name)
    }

    pub async fn create(&self, profile_id: &str, workspace: &Path, reserve_bytes: u64) -> Response {
        self.create_request(&self.request(profile_id, workspace, reserve_bytes))
            .await
    }

    /// The per-run `HOME` every create request of this rig names.
    pub fn run_home(&self) -> PathBuf {
        self.root.join("run-home")
    }

    /// A create request with this rig's run home.
    pub fn request(
        &self,
        profile_id: &str,
        workspace: &Path,
        reserve_bytes: u64,
    ) -> CaptureWorkspaceRequest {
        let connector_dir_sha256 = std::fs::read_dir(&self.config.connectors_dir)
            .unwrap()
            .flatten()
            .find_map(|child| {
                let dir = child.path();
                // A symlinked child points outside the tree and must never be opened: a test
                // plants a FIFO there. Directory order decides whether it is reached before
                // the profile's own directory (ext4 hashes names, APFS sorts them).
                if !child.file_type().ok()?.is_dir() {
                    return None;
                }
                let text = std::fs::read_to_string(dir.join("harness.toml")).ok()?;
                let manifest =
                    goalcup_protocol::harness::HarnessManifest::from_toml_str(&text).ok()?;
                (manifest.id == profile_id)
                    .then(|| goalcup_protocol::connector_dir::connector_dir_sha256(&dir).unwrap())
            })
            .unwrap_or_else(|| "00".repeat(32));
        CaptureWorkspaceRequest {
            workspace: workspace.display().to_string(),
            profile_id: profile_id.to_string(),
            connector_dir_sha256,
            reserve_bytes,
            run_home: self.run_home().display().to_string(),
            config_home: self.root.join("config-home").display().to_string(),
            entry_id: None,
            native_since: None,
        }
    }

    pub async fn create_request(&self, request: &CaptureWorkspaceRequest) -> Response {
        self.client
            .post_body(ROUTE_CAPTURE_WORKSPACES, request)
            .await
    }

    /// `POST .../process` with `registration`.
    pub async fn register(&self, entry_id: &str, registration: &ProcessRegistration) -> Response {
        self.client
            .post_body(&entry_route(entry_id, SUFFIX_PROCESS), registration)
            .await
    }

    /// `POST .../seal` with a capture seal request carrying only `proxy`.
    pub async fn seal(&self, entry_id: &str, proxy: Option<ProxyCounters>) -> Response {
        self.seal_with(
            entry_id,
            &CaptureSealRequest {
                proxy,
                remaining_finish_ms: None,
                native_scope: None,
            },
        )
        .await
    }

    /// `POST .../seal` with `request` as it stands: the remaining budget and the native scope
    /// included.
    pub async fn seal_with(&self, entry_id: &str, request: &CaptureSealRequest) -> Response {
        self.client
            .post_body(&entry_route(entry_id, SUFFIX_SEAL), request)
            .await
    }

    /// `POST .../abandon` with no body.
    pub async fn abandon(&self, entry_id: &str) -> Response {
        self.client
            .post(&entry_route(entry_id, SUFFIX_ABANDON))
            .await
    }

    /// A created entry for `profile_id` with a fresh workspace and [`RESERVE`].
    pub async fn created(&self, profile_id: &str) -> CaptureWorkspaceInfo {
        let workspace = self.workspace(&format!("{profile_id}-{}", unique()));
        let response = self.create(profile_id, &workspace, RESERVE).await;
        assert_eq!(response.status(), 200, "create: {}", response.body());
        response.json().expect("a CaptureWorkspaceInfo")
    }

    pub async fn goal(&self, entry_id: &str, bytes: &[u8]) -> Response {
        let request = CaptureGoalRequest {
            bytes: bytes.to_vec(),
        };
        self.client
            .post_body(&entry_route(entry_id, SUFFIX_CAPTURE_GOAL), &request)
            .await
    }

    /// The goal stream of an accepted goal.
    pub async fn goal_accepted(&self, entry_id: &str, bytes: &[u8]) -> StreamKey {
        let response = self.goal(entry_id, bytes).await;
        assert_eq!(response.status(), 200, "goal: {}", response.body());
        response.json().expect("a StreamKey")
    }

    pub async fn launch(&self, entry_id: &str, request: &CaptureLaunchRequest) -> Response {
        self.client
            .post_body(&entry_route(entry_id, SUFFIX_CAPTURE_LAUNCH), request)
            .await
    }

    /// The public status route, as `goalcup status` reads it: no bearer.
    pub async fn status(&self, entry_id: &str) -> SignedStatus {
        let response = self
            .client
            .get_no_token(&entry_route(entry_id, SUFFIX_STATUS))
            .await;
        assert_eq!(response.status(), 200, "status: {}", response.body());
        response.json().expect("a SignedStatus")
    }

    pub fn artifact_dir(&self, entry_id: &str) -> PathBuf {
        self.config.artifact_root.join("artifacts").join(entry_id)
    }

    pub fn trace_dir(&self, entry_id: &str) -> PathBuf {
        self.artifact_dir(entry_id).join("trace")
    }

    /// Every chain line, decoded.
    pub fn chain(&self, entry_id: &str) -> Vec<Envelope> {
        super::support::read_chain(&self.trace_dir(entry_id))
    }

    /// The entry's capture records now.
    pub fn records(&self, entry_id: &str) -> Vec<CaptureRecord> {
        self.chain(entry_id)
            .into_iter()
            .filter_map(|envelope| match envelope.body {
                EventBody::Capture { record } => Some(record),
                _ => None,
            })
            .collect()
    }

    /// The entry's capture records, once `ready` holds for them (10 s at most).
    pub async fn records_when(
        &self,
        entry_id: &str,
        ready: impl Fn(&[CaptureRecord]) -> bool,
    ) -> Vec<CaptureRecord> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let records = self.records(entry_id);
            if ready(&records) {
                return records;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the chain never reached the expected state: {records:#?}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    /// Connections made to the configured upload URL since the rig started.
    pub fn upload_dials(&self) -> usize {
        let mut dials = 0;
        while self.upload_trap.accept().is_ok() {
            dials += 1;
        }
        dials
    }
}

/// A launch request naming `goal_stream`, observed on a launcher clock of its own.
pub fn launch_request(goal_stream: &StreamKey) -> CaptureLaunchRequest {
    CaptureLaunchRequest {
        goal_stream: goal_stream.clone(),
        spawned_at: CaptureTime {
            wall: "2026-09-29T12:00:00.000000000Z".to_string(),
            mono_ns: 1,
            clock_id: "launcher-clock".to_string(),
        },
        harness_version: "0.153.0".to_string(),
        continuation: "exec: no input after the goal".to_string(),
        native_scope: Some("test-session".into()),
    }
}

/// 32 lowercase hex digits, fresh per call.
pub fn fresh_id() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn unique() -> String {
    fresh_id()[..12].to_string()
}

/// A sleeping child this test owns, killed on drop. It holds the process-table guard for as
/// long as it exists, so it is created on the test's own thread.
pub struct Child(pub std::process::Child, TableLock);

impl Child {
    pub fn sleeping() -> Child {
        let lock = TableLock::acquire();
        Child(super::support::spawn_harness(), lock)
    }

    /// Its registration at `generation`, naming the Codex profile's native binary.
    pub fn registration(&self, generation: u64) -> ProcessRegistration {
        let mut handle = super::support::handle_for_now(self.0.id());
        handle.native_binary = Some("codex".to_string());
        ProcessRegistration { generation, handle }
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// One producer epoch over the entry's ingest socket, framed as the capture sender frames it.
pub struct Wire {
    socket: std::net::TcpStream,
    pub producer: String,
    seq: u64,
}

impl Wire {
    /// Connect and send the handshake for `producer`.
    pub fn connect(info: &CaptureWorkspaceInfo, producer: &str) -> Wire {
        let mut socket = std::net::TcpStream::connect(info.ingest_addr).expect("connect to ingest");
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .unwrap();
        let body = serde_json::to_vec(&serde_json::json!({
            "entry_id": info.entry_id,
            "producer": producer,
            "token": info.ingest_secret,
        }))
        .unwrap();
        socket
            .write_all(&(body.len() as u32).to_be_bytes())
            .unwrap();
        socket.write_all(&body).unwrap();
        Wire {
            socket,
            producer: producer.to_string(),
            seq: 0,
        }
    }

    /// The producer's next frame on `stream`.
    pub fn send(&mut self, stream: u64, event: FrameEvent) {
        let frame = CaptureFrame {
            stream: StreamKey {
                producer: self.producer.clone(),
                stream,
            },
            seq: self.seq,
            observed: CaptureTime {
                wall: "2026-09-29T12:00:00.000000000Z".to_string(),
                mono_ns: self.seq,
                clock_id: self.producer.clone(),
            },
            event,
        };
        self.seq += 1;
        self.item(&IngestMessage::Frame(frame));
    }

    /// `Close { final_seq }` naming the last frame sent, then end of stream.
    pub fn close(self) {
        let final_seq = self.seq.checked_sub(1);
        self.close_claiming(final_seq);
    }

    /// `Close` claiming `final_seq`, then end of stream; waits until the receiver hangs up.
    pub fn close_claiming(mut self, final_seq: Option<u64>) {
        let close = IngestMessage::Close {
            producer: self.producer.clone(),
            final_seq,
        };
        self.item(&close);
        self.hang_up();
    }

    /// End of stream with no `Close`; waits until the receiver hangs up.
    pub fn hang_up(mut self) {
        let _ = self.socket.shutdown(std::net::Shutdown::Write);
        let mut rest = Vec::new();
        let _ = self.socket.read_to_end(&mut rest);
    }

    fn item(&mut self, message: &IngestMessage) {
        let framed = encode_ingest_message(message).expect("a valid ingest message");
        self.socket.write_all(&framed).expect("the receiver reads");
    }
}

/// A Codex-family rig with its required scoped rollout directory prepared. The connectors
/// directory is returned to outlive the rig.
pub async fn native_rig() -> (Rig, PathBuf, tempfile::TempDir) {
    let connectors = dev_connectors(|_| {});
    let dir = connectors.path().to_path_buf();
    let rig = Rig::start_with(move |config| config.connectors_dir = dir).await;
    let native_dir = rig.root.join("config-home/sessions/2026/09/30");
    std::fs::create_dir_all(&native_dir).unwrap();
    std::fs::write(
        native_dir.join("rollout-fixture-test-session.jsonl"),
        b"{\"type\":\"native\"}\n",
    )
    .unwrap();
    (rig, native_dir, connectors)
}

/// The unknown non-shipping family preserves the controlled workload's run-HOME evidence.
pub async fn controlled_native_rig() -> (Rig, PathBuf, tempfile::TempDir) {
    let connectors = tempfile::tempdir().unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../goalcup-cli/tests/fixtures/m2-controlled-workload/harness.toml");
    let harness = std::fs::read_to_string(source)
        .unwrap()
        .replace("id = \"m2-controlled-workload\"", "id = \"capture-dev\"")
        .replace("@WORKLOAD@", "/synthetic/workload.py")
        .replace("@CONFIG@", "/synthetic/config.json");
    let connector = connectors.path().join("capture-dev");
    std::fs::create_dir_all(&connector).unwrap();
    std::fs::write(connector.join("harness.toml"), harness).unwrap();
    let dir = connectors.path().to_path_buf();
    let rig = Rig::start_with(move |config| config.connectors_dir = dir).await;
    let native_dir = rig.run_home().join("native");
    std::fs::create_dir_all(&native_dir).unwrap();
    (rig, native_dir, connectors)
}

/// How many descriptors this process, which runs the rig's daemon, holds open on `path`'s file.
/// Each is measured with `fstat`: a `stat` through `/dev/fd` names the descriptor filesystem's
/// device, not the file's.
pub fn descriptors_on(path: &Path) -> usize {
    use std::os::unix::fs::MetadataExt;
    let file = std::fs::metadata(path).expect("the file exists");
    let descriptors: Vec<i32> = std::fs::read_dir("/dev/fd")
        .expect("this process's descriptors")
        .filter_map(|fd| fd.ok()?.file_name().to_str()?.parse().ok())
        .collect();
    descriptors
        .into_iter()
        .filter(|&fd| {
            let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
            // SAFETY: `fstat` fills `stat` for an open descriptor and fails for any other; it
            // is read only after a success.
            unsafe {
                libc::fstat(fd, stat.as_mut_ptr()) == 0 && {
                    let stat = stat.assume_init();
                    // `st_dev` is an i32 on macOS and a u64 on Linux.
                    #[allow(clippy::unnecessary_cast)]
                    let same_device = stat.st_dev as u64 == file.dev();
                    same_device && stat.st_ino == file.ino()
                }
            }
        })
        .count()
}

/// A connectors directory holding the two shipped connectors and `capture-dev`: the Codex
/// manifest under that id, without foreign scans, with `edit` applied.
pub fn dev_connectors(edit: impl FnOnce(&mut String)) -> tempfile::TempDir {
    let connectors = tempfile::tempdir().unwrap();
    for shipped in ["codex-cli", "claude-code-cli"] {
        super::support::copy_tree(
            &super::support::shipped_connector_dir(shipped),
            &connectors.path().join(shipped),
        );
    }
    let text = std::fs::read_to_string(
        super::support::shipped_connector_dir("codex-cli").join("harness.toml"),
    )
    .unwrap();
    let dir = connectors.path().join("capture-dev");
    std::fs::create_dir_all(&dir).unwrap();
    let mut rendered = text.replacen("id = \"codex-cli\"", "id = \"capture-dev\"", 1);
    edit(&mut rendered);
    std::fs::write(dir.join("harness.toml"), rendered).unwrap();
    connectors
}

/// `LocalFinish` records in the chain: one for a sealed entry, none for a refused finish.
pub fn local_finishes(records: &[CaptureRecord]) -> usize {
    records
        .iter()
        .filter(|r| matches!(r, CaptureRecord::LocalFinish { .. }))
        .count()
}

/// Stream-less gaps with `reason`, in chain order.
pub fn streamless(records: &[CaptureRecord], reason: GapReason) -> usize {
    records
        .iter()
        .filter(|r| matches!(r, CaptureRecord::Gap { stream: None, reason: g, .. } if *g == reason))
        .count()
}
