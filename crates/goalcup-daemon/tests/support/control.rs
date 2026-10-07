//! The control listener, the bearer it minted, and a client small
//! enough to send a path this workspace's own code would never construct.
//!
//! # Why the client is hand-written
//!
//! The workspace has no client crate, and a dependency added so a test can speak the
//! protocol it already serves is a dependency the workspace then carries.
//!
//! Here there is a second reason. The traversal cases are
//! `/v0/assets/../entries/e1/seal` and `/v0/assets/%252e%252e/…` — request lines that a
//! real HTTP client normalises, percent-decodes or refuses before they ever reach a
//! socket. This client writes the bytes of the request line verbatim, so what the daemon's
//! auth layer sees is what the test wrote. A test that could not send an un-normalised
//! path could not tell a layer that asks about the raw path apart from one that does not.

#![allow(dead_code)]

use super::daemon::DaemonHarness;
use goalcup_daemon::config::DaemonConfig;
use goalcup_daemon::runtime::Daemon;
use goalcup_protocol::control::{CONTROL_API_VERSION, WorkspaceOpts};
use goalcup_protocol::process::ProcessHandle;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

/// One response, read to EOF under `Connection: close` so nothing here decodes chunks.
pub struct Response {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Response {
    pub fn status(&self) -> u16 {
        self.status
    }

    pub fn body(&self) -> &str {
        &self.body
    }

    /// Header lookup, case-insensitive as HTTP is.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// Every value under one case-insensitive HTTP header name.  `header` above is enough
    /// for ordinary response fields, while control-version tests must prove the daemon emits
    /// exactly one value rather than merely finding a valid first one beside a duplicate.
    pub fn headers_named(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
            .collect()
    }

    /// The body as one of the contract's types.
    ///
    /// Synchronous: the body is already whole in memory by the time this returns —
    /// there is nothing left to await.
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_str(&self.body)
    }
}

/// The control listener's address and the CLI's bearer, and every way a test reaches it.
///
/// Separate from [`ControlHarness`]: that harness holds a daemon with one started
/// entry, and `POST /v0/workspaces` has to be exercised against a daemon holding none —
/// "failure before dispatch leaves no entry" is an assertion about an empty entry list, and
/// a fixture that came with an entry already in it could not make it. So both harnesses hold
/// one of these rather than each carrying its own copy of the twenty lines below.
pub struct ControlClient {
    pub addr: SocketAddr,
    pub token: String,
}

impl ControlClient {
    /// The client the CLI builds for `daemon`: its control address, and the bearer it
    /// wrote into `control.json` under `config_dir`. `Daemon::start` mints a fresh bearer
    /// every time, so a restarted daemon needs a new client.
    pub fn of(daemon: &Daemon, config_dir: &std::path::Path) -> ControlClient {
        let file =
            std::fs::read_to_string(config_dir.join(goalcup_daemon::control::CONTROL_FILE_NAME))
                .expect("the daemon wrote control.json");
        let parsed: serde_json::Value = serde_json::from_str(&file).expect("control.json is JSON");
        ControlClient {
            addr: daemon.control_addr(),
            token: parsed["token"]
                .as_str()
                .expect("control.json carries the CLI's bearer")
                .to_string(),
        }
    }

    /// `GET path` with the control bearer.
    pub async fn get(&self, path: &str) -> Response {
        self.request(
            "GET",
            path,
            Some(self.token.clone()),
            Vec::new(),
            String::new(),
        )
        .await
    }

    /// `GET path` with no `Authorization` header — what a browser opening the status page
    /// sends, and what every unauthenticated case here sends.
    pub async fn get_no_token(&self, path: &str) -> Response {
        self.request("GET", path, None, Vec::new(), String::new())
            .await
    }

    /// [`ControlClient::get_no_token`], named for the cases whose whole point is that the
    /// path reaches the daemon un-normalised.
    ///
    /// It is the same call: this client normalises nothing, which is exactly why the traversal
    /// cases can be written at all. The second name exists so a reader of
    /// `/v0/assets/%252e%252e/…` can see that the double encoding is deliberate rather
    /// than a typo a client would have cleaned up.
    pub async fn get_raw_no_token(&self, path: &str) -> Response {
        self.get_no_token(path).await
    }

    /// `POST path` with the bearer and an empty body — `arm` and `abandon`, which take
    /// their whole argument from the path.
    pub async fn post(&self, path: &str) -> Response {
        self.request(
            "POST",
            path,
            Some(self.token.clone()),
            control_version_header(),
            String::new(),
        )
        .await
    }

    /// `POST path` with the bearer and `body` as JSON.
    pub async fn post_body<B: serde::Serialize>(&self, path: &str, body: &B) -> Response {
        let text = serde_json::to_string(body).expect("the request body serialises");
        self.request(
            "POST",
            path,
            Some(self.token.clone()),
            control_version_header(),
            text,
        )
        .await
    }

    /// [`ControlClient::post_body`], insisting on a `200` and one of the contract's types.
    ///
    /// The status is asserted here rather than at every call site: a handler that answered a
    /// refusal would otherwise fail as "the contract's type did not parse", which names the
    /// symptom and hides the body that says why.
    pub async fn post_json<B: serde::Serialize, T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> T {
        let r = self.post_body(path, body).await;
        assert_eq!(r.status(), 200, "{path}: {}", r.body());
        r.json().expect("the contract's type")
    }

    /// One request, with the request line written byte for byte.
    pub async fn request(
        &self,
        method: &'static str,
        path: &str,
        token: Option<String>,
        extra: Vec<(String, String)>,
        body: String,
    ) -> Response {
        self.request_with_header_bytes(
            method,
            path,
            token,
            extra
                .into_iter()
                .map(|(name, value)| (name, value.into_bytes()))
                .collect(),
            body,
        )
        .await
    }

    /// [`ControlClient::request`] for a deliberate non-text header value.  A `String`
    /// cannot carry `0x80`, but `HeaderValue::to_str` must refuse it just as a real raw
    /// loopback client would; writing this request by bytes keeps that protocol boundary
    /// observable without adding an HTTP client dependency to the test tree.
    pub async fn request_raw_header_values(
        &self,
        method: &'static str,
        path: &str,
        token: Option<String>,
        extra: Vec<(String, Vec<u8>)>,
        body: String,
    ) -> Response {
        self.request_with_header_bytes(method, path, token, extra, body)
            .await
    }

    async fn request_with_header_bytes(
        &self,
        method: &'static str,
        path: &str,
        token: Option<String>,
        extra: Vec<(String, Vec<u8>)>,
        body: String,
    ) -> Response {
        let addr = self.addr.to_string();
        let path = path.to_string();
        tokio::task::spawn_blocking(move || {
            use std::io::{Read, Write};
            let mut stream =
                std::net::TcpStream::connect(&addr).expect("connect to the control listener");
            let mut head = format!(
                "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\
                 Content-Type: application/json\r\nContent-Length: {}\r\n",
                body.len()
            )
            .into_bytes();
            if let Some(token) = &token {
                head.extend_from_slice(format!("Authorization: Bearer {token}\r\n").as_bytes());
            }
            for (k, v) in &extra {
                head.extend_from_slice(k.as_bytes());
                head.extend_from_slice(b": ");
                head.extend_from_slice(v);
                head.extend_from_slice(b"\r\n");
            }
            head.extend_from_slice(b"\r\n");
            stream.write_all(&head).expect("write the head");
            let _ = stream.write_all(body.as_bytes());
            let mut raw = Vec::new();
            let _ = stream.read_to_end(&mut raw);
            parse(&String::from_utf8_lossy(&raw))
        })
        .await
        .expect("the blocking request finishes")
    }
}

/// Normal test calls model a supported V1 client.  Tests for missing/stale headers use
/// [`ControlClient::request`] deliberately, because that method writes exactly the supplied
/// headers and must remain able to model an old direct caller.
fn control_version_header() -> Vec<(String, String)> {
    vec![(
        "x-goalcup-control-version".to_string(),
        CONTROL_API_VERSION.to_string(),
    )]
}

/// A daemon with one started entry, its control listener, and the two tokens that must
/// never be the same string.
pub struct ControlHarness {
    inner: DaemonHarness,
    pub client: ControlClient,
    pub control_addr: SocketAddr,
    pub config_dir: PathBuf,
    /// The CLI's bearer, read back out of `control.json`.
    pub control_token: String,
    /// D24's per-run token, which lives in the workspace where the agent can read it.
    pub receiver_token: String,
    /// The key `SignedStatus::verify_with` is pinned against.
    pub run_public_key: String,
}

/// Field access on the daemon harness — `h.daemon`, `h.entry_id`, `h.trace_dir` — reads
/// through to the harness this one wraps, so a control test names the same fields every
/// other daemon test does.
impl std::ops::Deref for ControlHarness {
    type Target = DaemonHarness;

    fn deref(&self) -> &DaemonHarness {
        &self.inner
    }
}

/// Every request method reads through the one [`ControlClient`], so a test written against
/// the started-entry harness and a test written against the empty one send the same bytes.
///
/// Delegated rather than `Deref`'d: this type already dereferences to [`DaemonHarness`], so
/// `h.entry_id` and `h.daemon` keep reading through to the daemon fixture, and a second
/// `Deref` impl on one type does not compile.
impl ControlHarness {
    pub async fn get(&self, path: &str) -> Response {
        self.client.get(path).await
    }

    pub async fn get_no_token(&self, path: &str) -> Response {
        self.client.get_no_token(path).await
    }

    pub async fn get_raw_no_token(&self, path: &str) -> Response {
        self.client.get_raw_no_token(path).await
    }

    pub async fn post(&self, path: &str) -> Response {
        self.client.post(path).await
    }

    pub async fn post_body<B: serde::Serialize>(&self, path: &str, body: &B) -> Response {
        self.client.post_body(path, body).await
    }

    pub async fn post_json<B: serde::Serialize, T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> T {
        self.client.post_json(path, body).await
    }

    pub async fn request(
        &self,
        method: &'static str,
        path: &str,
        token: Option<String>,
        extra: Vec<(String, String)>,
        body: String,
    ) -> Response {
        self.client.request(method, path, token, extra, body).await
    }

    pub async fn request_raw_header_values(
        &self,
        method: &'static str,
        path: &str,
        token: Option<String>,
        extra: Vec<(String, Vec<u8>)>,
        body: String,
    ) -> Response {
        self.client
            .request_raw_header_values(method, path, token, extra, body)
            .await
    }
}

fn parse(text: &str) -> Response {
    let (head, body) = match text.split_once("\r\n\r\n") {
        Some((h, b)) => (h, b.to_string()),
        None => (text, String::new()),
    };
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .expect("a status line");
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    Response {
        status,
        headers,
        body,
    }
}

/// A daemon holding one started entry, with its status document published, so the one
/// public route that answers a document has a document to answer with.
#[cfg(feature = "v1-compat")]
pub async fn control_harness() -> ControlHarness {
    let inner = super::daemon::started_entry_with_work().await;
    // The status cadence, run once by hand: `Daemon::run_forever` is not driving this
    // harness, and the public status route reads what the last publication left.
    inner
        .daemon
        .refresh_statuses()
        .expect("the statuses publish");

    let config_dir = inner.config.config_dir.clone();
    let client = ControlClient::of(&inner.daemon, &config_dir);
    let entry = inner.entry();

    ControlHarness {
        control_addr: client.addr,
        control_token: client.token.clone(),
        client,
        receiver_token: entry.keys.receiver_token.clone(),
        run_public_key: entry.keys.public_key_hex().expect("the run public key"),
        config_dir,
        inner,
    }
}

/// [`super::daemon::restart_daemon`] with the control surface rebuilt around the second
/// daemon.
///
/// The bearer is re-read rather than carried over: `Daemon::start` mints a fresh control
/// token every time and writes it into `control.json`, so a harness that kept the first one
/// would get a `401` from every authenticated route and read it as a regression in the
/// route.
pub async fn restart_control(h: ControlHarness) -> ControlHarness {
    let ControlHarness {
        inner,
        receiver_token,
        run_public_key,
        ..
    } = h;
    let inner = super::daemon::restart_daemon(inner).await;
    let config_dir = inner.config.config_dir.clone();
    let client = ControlClient::of(&inner.daemon, &config_dir);
    ControlHarness {
        control_addr: client.addr,
        control_token: client.token.clone(),
        client,
        receiver_token,
        run_public_key,
        config_dir,
        inner,
    }
}

/// A daemon holding no entry, its control listener, and the bearer it minted.
///
/// `POST /v0/workspaces` is the call that *creates* the first entry, and
/// "failure before dispatch leaves no entry" is an assertion about the entry list being
/// empty afterwards — neither can be written against a harness that arrives with a started
/// entry already in it.
///
/// The `TempDir` and the dev server are held so the tree and the upload target outlive the
/// daemon; dropping either mid-test would make a tick fail for a reason no test caused.
pub struct EmptyControlHarness {
    pub daemon: Daemon,
    pub client: ControlClient,
    pub config: DaemonConfig,
    pub config_dir: PathBuf,
    pub control_token: String,
    /// The canonical root every path in this harness hangs off.
    pub root: PathBuf,
    _server: Arc<goalcup_devserver::DevServer>,
    _dir: tempfile::TempDir,
}

/// This harness has no second target to dereference to, so the client is reached directly.
impl std::ops::Deref for EmptyControlHarness {
    type Target = ControlClient;

    fn deref(&self) -> &ControlClient {
        &self.client
    }
}

pub async fn empty_control_harness() -> EmptyControlHarness {
    empty_control_harness_with(|_| {}).await
}

/// [`empty_control_harness`] with one edit applied to the configuration before the daemon
/// is started.
///
/// The edit is a closure rather than a second `config_at`: what a test wants to change is
/// one field, and a fixture that took seventeen arguments so a test could change one of them
/// would be a fixture every other test then had to read.
pub async fn empty_control_harness_with(
    edit: impl FnOnce(&mut DaemonConfig),
) -> EmptyControlHarness {
    let (server, upload_url) = super::devserver().await;
    empty_control_harness_on(server, &upload_url, edit).await
}

/// [`empty_control_harness_with`] against a receipt signer the caller started, at
/// `upload_url`.
pub async fn empty_control_harness_on(
    server: Arc<goalcup_devserver::DevServer>,
    upload_url: &str,
    edit: impl FnOnce(&mut DaemonConfig),
) -> EmptyControlHarness {
    let dir = tempfile::tempdir().expect("a temporary directory");
    // Canonical from the start, for the reason `started_entry_with_work` gives: on macOS a
    // tempdir is handed out as `/var/folders/…` while the harness records
    // `/private/var/folders/…`, and the partitioner compares the two byte for byte.
    let root = std::fs::canonicalize(dir.path()).expect("the tempdir canonicalises");
    let mut config = super::config_at(&root, upload_url);
    edit(&mut config);
    let daemon = Daemon::start(config.clone())
        .await
        .expect("the daemon starts");
    let config_dir = config.config_dir.clone();
    let client = ControlClient::of(&daemon, &config_dir);

    EmptyControlHarness {
        control_token: client.token.clone(),
        client,
        config_dir,
        config,
        root,
        daemon,
        _server: server,
        _dir: dir,
    }
}

/// An empty daemon under [`v4_binding`], with its receipts signed by a new-event signer for
/// the same questionnaire.
pub async fn v4_daemon(object_count: u64) -> EmptyControlHarness {
    let authority = goalcup_devserver::ReceiptAuthority::new_event("goal-cup-2026-v1")
        .expect("the V4 questionnaire authority is valid");
    let (server, upload_url) = super::devserver_with_authority(authority).await;
    empty_control_harness_on(server, &upload_url, |config| {
        config.receipt_binding = v4_binding(object_count);
    })
    .await
}

/// The V4 workspace-submission receipt binding with test limits: `object_count` caps the
/// out-of-window partition's records, and `total_bytes` (1 MiB) caps one record.
pub fn v4_binding(object_count: u64) -> goalcup_daemon::config::ReceiptBindingPolicy {
    goalcup_daemon::config::ReceiptBindingPolicy::WorkspaceSubmission {
        questionnaire_version: "goal-cup-2026-v1".to_string(),
        submission_profile: goalcup_protocol::submission::SubmissionProfileV1::WorkspaceSnapshotV1,
        limit_profile: goalcup_protocol::submission::LimitProfileV1 {
            id: "goal-cup-v1-test".to_string(),
        },
        effective_limits: goalcup_protocol::submission::EffectiveSubmissionLimitsV1 {
            object_count,
            total_bytes: 1_048_576,
            pre_cutoff_drain: 1_000,
            process_walk: 16,
        },
        capture_limits: goalcup_daemon::CaptureResourceLimitsV1 {
            version: 1,
            namespace_nodes: 1_024,
            inventory_bytes: 1_048_576,
            manifest_bytes: 1_048_576,
        },
    }
}

/// A daemon holding one **capture** entry (a v5 local capture, no V1 workspace), its control
/// listener and bearer. It is the fixture the auth, version and route-table tests use, because
/// those properties belong to the control layer both builds serve; the V1 entry in
/// [`ControlHarness`] exists only with `v1-compat`.
pub struct CaptureControlHarness {
    inner: EmptyControlHarness,
    pub entry_id: String,
    /// The per-run key `SignedStatus::verify_with` is pinned against.
    pub run_public_key: String,
    /// The capture ingest bearer, which must never equal the control token and never appear in
    /// a control response.
    pub ingest_secret: String,
}

impl std::ops::Deref for CaptureControlHarness {
    type Target = EmptyControlHarness;

    fn deref(&self) -> &EmptyControlHarness {
        &self.inner
    }
}

/// [`empty_control_harness`] plus one created `codex-cli` capture entry.
pub async fn capture_control_harness() -> CaptureControlHarness {
    let inner = empty_control_harness().await;
    let workspace = inner.root.join("workspaces").join("capture-control");
    let request = goalcup_protocol::control::CaptureWorkspaceRequest {
        workspace: workspace.display().to_string(),
        profile_id: "codex-cli".to_string(),
        connector_dir_sha256: goalcup_protocol::pins::pinned_connector_dir_sha256("codex-cli")
            .unwrap()
            .to_string(),
        reserve_bytes: 4 * 1024 * 1024,
        run_home: inner.root.join("run-home").display().to_string(),
        config_home: inner.root.join("config-home").display().to_string(),
    };
    let response = inner
        .client
        .post_body(
            goalcup_protocol::control::ROUTE_CAPTURE_WORKSPACES,
            &request,
        )
        .await;
    assert_eq!(response.status(), 200, "create: {}", response.body());
    let info: goalcup_protocol::control::CaptureWorkspaceInfo =
        response.json().expect("a CaptureWorkspaceInfo");
    CaptureControlHarness {
        entry_id: info.entry_id,
        run_public_key: info.run_public_key,
        ingest_secret: info.ingest_secret,
        inner,
    }
}

/// [`empty_control_harness`] restarted over the same tree: a second daemon, rehydrating
/// whatever rows the first one left.
///
/// The started-entry harness has [`restart_control`]; this one is for the case of a
/// registry row in the shape an older daemon wrote it, taken up by the current daemon.
/// That row can only be planted after a create and can only be rehydrated by a restart, so
/// the fixture has to be the empty one.
///
/// The bearer is re-read for [`restart_control`]'s reason: `Daemon::start` mints a fresh
/// control token every time and writes it into `control.json`.
pub async fn restart_empty_control(h: EmptyControlHarness) -> EmptyControlHarness {
    restart_empty_control_planting(h, |_| {})
        .await
        .expect("a second daemon starts over the same tree")
}

/// [`restart_empty_control`] with rows put into the stopped daemon's registry first, and the
/// start's own answer returned: a refusal is a result a test asserts on, not a panic.
pub async fn restart_empty_control_planting(
    h: EmptyControlHarness,
    plant: impl FnOnce(&goalcup_daemon::registry::Registry),
) -> Result<EmptyControlHarness, goalcup_daemon::DaemonError> {
    let EmptyControlHarness {
        daemon,
        config,
        root,
        _server,
        _dir,
        ..
    } = h;
    daemon.stop().await;
    {
        let path = goalcup_daemon::registry::Registry::default_path(&config.artifact_root);
        let registry = goalcup_daemon::registry::Registry::open(&path)
            .expect("the stopped daemon's registry opens");
        plant(&registry);
    }
    let daemon = Daemon::start(config.clone()).await?;
    let config_dir = config.config_dir.clone();
    let client = ControlClient::of(&daemon, &config_dir);
    Ok(EmptyControlHarness {
        control_token: client.token.clone(),
        client,
        config_dir,
        config,
        root,
        daemon,
        _server,
        _dir,
    })
}

/// The options `goalcup start` posts for `connector_id`, with the manifest digest read off
/// the shipped file rather than made up: the daemon refuses a manifest that is not the one
/// in the signed release, so a fixture carrying a plausible hex string would test
/// the refusal instead of the route.
///
/// There is no connector-directory digest to send. The daemon hashes its own connectors
/// directory and compares that against the pins itself, so the only way a test can move
/// that comparison is to point the daemon at a different directory —
/// [`super::write_config_with`] and [`shipped_connector_dir`] are what
/// `control_api.rs` builds such a case out of.
pub fn workspace_opts(connector_id: &str) -> WorkspaceOpts {
    WorkspaceOpts {
        connector_id: connector_id.to_string(),
        manifest_sha256: shipped_manifest_sha256(connector_id),
        workspace_parent: None,
        attach: false,
        deadline_hours: 168,
        grant_token: "gt-a-grant-goalcup-auth-stored".to_string(),
        arming_timeout_seconds: None,
        // What the CLI's preflight measured. A test daemon runs no probe, so
        // this is the value under test rather than something it discovers.
        harness_version: Some("2.1.259".to_string()),
    }
}

/// The digest of `connectors/<id>/manifest.toml` as it ships.
pub fn shipped_manifest_sha256(connector_id: &str) -> String {
    let path = shipped_connector_dir(connector_id).join("manifest.toml");
    goalcup_daemon::workspace::sha256_hex(&std::fs::read(&path).expect("the shipped manifest"))
}

/// The digest of the whole `connectors/<id>/` directory as it ships — the start-time pin,
/// which covers the hook templates under `files/` the manifest digest cannot see.
pub fn shipped_connector_dir_sha256(connector_id: &str) -> String {
    goalcup_protocol::connector_dir::connector_dir_sha256(&shipped_connector_dir(connector_id))
        .expect("the shipped connector directory hashes")
}

/// `from` copied to `to`, recursively, creating what it needs.
///
/// Exists so a test can hand the daemon a connectors directory of its own and edit one file
/// in it: the daemon hashes the directory it is configured with, so a tamper has to be a
/// tamper on disk rather than a field in a request.
pub fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).expect("the target directory is created");
    for entry in std::fs::read_dir(from)
        .expect("the source directory reads")
        .flatten()
    {
        let source = entry.path();
        let Some(name) = source.file_name() else {
            continue;
        };
        let target = to.join(name);
        if source.is_dir() {
            copy_tree(&source, &target);
        } else {
            std::fs::copy(&source, &target).expect("the file copies");
        }
    }
}

/// `connectors/<id>/`, from this crate's manifest directory.
pub fn shipped_connector_dir(connector_id: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../connectors")
        .join(connector_id)
}

/// The handle the CLI registers after a launch it did not put in its own process group: a
/// TUI inherits the participant's terminal, so `pgid` is `None` and the guard is the age
/// plus the argv.
pub fn process_handle() -> ProcessHandle {
    super::daemon::handle_for_now(std::process::id())
}

/// A test-only spelling of the typed wire document.  Production callers never infer this
/// number: it comes from `WorkspaceInfo` or the successful resume response.
pub fn process_registration(
    generation: u64,
    handle: ProcessHandle,
) -> goalcup_protocol::process::ProcessRegistration {
    goalcup_protocol::process::ProcessRegistration { generation, handle }
}
