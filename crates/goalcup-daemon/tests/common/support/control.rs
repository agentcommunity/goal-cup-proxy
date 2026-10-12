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

use goalcup_daemon::config::DaemonConfig;
use goalcup_daemon::runtime::Daemon;
use goalcup_protocol::control::CONTROL_API_VERSION;
use goalcup_protocol::process::ProcessHandle;
use std::net::SocketAddr;
use std::path::PathBuf;

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
/// Both harnesses below hold one of these rather than each carrying its own copy of the
/// twenty lines that follow.
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

/// A daemon holding no entry, its control listener, and the bearer it minted.
///
/// `POST /v0/workspaces` is the call that *creates* the first entry, and
/// "failure before dispatch leaves no entry" is an assertion about the entry list being
/// empty afterwards — neither can be written against a harness that arrives with a started
/// entry already in it.
///
/// The `TempDir` and the upload target are held so the tree and the port outlive the
/// daemon.
pub struct EmptyControlHarness {
    pub daemon: Daemon,
    pub client: ControlClient,
    pub config: DaemonConfig,
    pub config_dir: PathBuf,
    pub control_token: String,
    /// The canonical root every path in this harness hangs off.
    pub root: PathBuf,
    _upload: Option<std::net::TcpListener>,
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
    let (upload, upload_url) = super::upload_target();
    let mut h = empty_control_harness_on(&upload_url, edit).await;
    h._upload = Some(upload);
    h
}

/// [`empty_control_harness_with`] with the `upload_url` the caller chose.
pub async fn empty_control_harness_on(
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
        _upload: None,
        _dir: dir,
    }
}

/// A daemon holding one **capture** entry (a v5 local capture, no V1 workspace), its control
/// listener and bearer. It is the fixture the auth, version and route-table tests use.
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
/// This one is for the case of a registry row in the shape an older daemon wrote it, taken
/// up by the current daemon. That row can only be planted after a create and can only be
/// rehydrated by a restart, so the fixture has to be the empty one.
///
/// The bearer is re-read because `Daemon::start` mints a fresh control token every time and
/// writes it into `control.json`; a harness holding the old one would get a `401` from every
/// authenticated route.
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
        _upload,
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
        _upload,
        _dir,
    })
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
