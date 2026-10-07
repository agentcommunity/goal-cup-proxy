//! The plumbing every control-API gate test shares: the two child processes, the two
//! loopback clients, the configuration an operator writes, and the waits that let a test
//! assert a CONDITION the daemon's own cadences produce rather than a duration.
//!
//! Shared by every connector's gate test.
//! Nothing connector-specific lives here: the record and post shapes each
//! harness writes belong to the test that drives that harness, because they are the thing
//! under test.
#![allow(dead_code)]

use goalcup_protocol::control::{
    CONTROL_API_VERSION, ROUTE_ENTRIES, SignedStatus, WorkspaceInfo, WorkspaceOpts,
};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// How far below the run the server's clock is set. See the module header.
pub const CLOCK_MARGIN: u64 = 600;
/// The longest this test waits for any one of the daemon's cadences to come round.
pub const PATIENCE: Duration = Duration::from_secs(20);
/// The longest either child process is given to print a line this test is waiting for.
/// Generous, because it is only ever reached when the child is not going to print at all.
pub const CHILD_PATIENCE: Duration = Duration::from_secs(60);

/// The round this gate's daemon is told it is running, through its `[event]` section. The
/// dates are the organiser's in a real deployment; here they exist so the configured path
/// through `status::event_status_for` is the one exercised end to end, and so a signed
/// status carrying the FALLBACK is a failure rather than the shape nobody looked at.
pub const ROUND_T0: &str = "2026-10-01T00:00:00Z";
pub const ROUND_CLOSES: &str = "2026-10-08T00:00:00Z";
pub const ROUND_RULES: &str = "rules-2026-10-01";
// ------------------------------------------------------------------ the two processes

/// The binary `name`, beside the one cargo built for this package.
pub fn sibling(name: &str) -> PathBuf {
    let me = PathBuf::from(env!("CARGO_BIN_EXE_goalcup-devserver"));
    let path = me.with_file_name(name);
    assert!(
        path.is_file(),
        "{} is not built. This test drives the shipped binaries, which \
         `cargo test --workspace --offline` builds; one crate at a time does not.",
        path.display()
    );
    path
}

/// A child's piped stream, read on its own thread and handed over one line at a time —
/// under a deadline.
///
/// `BufReader::lines()` straight off a child pipe blocks forever when the child never
/// prints, and a child that refuses its configuration prints nothing at all: a one-line
/// refusal on the child's *other* stream would hang the run until something outside cargo
/// killed it, with no message about which process it was waiting for. Every read here fails
/// instead, naming what it wanted.
///
/// The reader thread outlives the test's interest on purpose. It keeps consuming after the
/// receiver is gone, so a child writing more than a pipe buffer's worth is never blocked by
/// nobody reading it.
pub struct ChildLines {
    pub rx: std::sync::mpsc::Receiver<String>,
}

impl ChildLines {
    pub fn spawn<R: Read + Send + 'static>(stream: R) -> ChildLines {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut listening = true;
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                if listening {
                    listening = tx.send(line).is_ok();
                }
            }
        });
        ChildLines { rx }
    }

    /// The next line, or a failure naming `what` this test was waiting for.
    pub fn next_line(&self, what: &str) -> String {
        match self.rx.recv_timeout(CHILD_PATIENCE) {
            Ok(line) => line,
            Err(e) => panic!("waited {CHILD_PATIENCE:?} for {what} and got {e}"),
        }
    }

    /// The first line `f` answers for, within the same deadline over the whole search: a
    /// child that prints an endless stream of lines this test does not want is as stuck as
    /// one that prints none.
    pub fn find(&self, what: &str, f: impl Fn(&str) -> Option<String>) -> String {
        let deadline = Instant::now() + CHILD_PATIENCE;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left) {
                Ok(line) => {
                    if let Some(found) = f(&line) {
                        return found;
                    }
                }
                Err(e) => panic!("waited {CHILD_PATIENCE:?} for {what} and got {e}"),
            }
        }
    }
}

/// A stand-in process owned by a control-API gate until its expected seal has reaped it.
///
/// The normal path still calls [`Self::release_after_reap`] after observing the daemon's
/// termination.  `Drop` covers only an assertion or unwind before that point, because a
/// dropped [`Child`] continues running otherwise.
pub struct ChildGuard {
    child: Option<Child>,
    reaped: bool,
}

impl ChildGuard {
    pub fn new(child: Child) -> Self {
        Self {
            child: Some(child),
            reaped: false,
        }
    }

    pub fn id(&self) -> u32 {
        self.child
            .as_ref()
            .expect("a released fixture child has no pid")
            .id()
    }

    /// Observe an exit on the same `Child` the test owns, recording that it was reaped.
    pub fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        let result = self
            .child
            .as_mut()
            .expect("a released fixture child cannot be waited on")
            .try_wait()?;
        self.reaped |= result.is_some();
        Ok(result)
    }

    /// Release the guard only after the gate has explicitly observed the expected reap.
    pub fn release_after_reap(mut self) {
        assert!(
            self.reaped,
            "a gate may release its fixture child only after observing its reap"
        );
        self.child.take();
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

pub struct Devserver {
    pub child: Child,
    pub url: String,
}

impl Devserver {
    pub fn start(clock: &str) -> Devserver {
        Self::start_with_args(clock, &[])
    }

    /// Start the devserver with the explicit new-event receipt contract used by the V4
    /// workspace-submission gate. The production server owns this choice; the test pins it
    /// on the process command line so the receipt the daemon binds is a real signed V4 input.
    pub fn start_workspace(clock: &str) -> Devserver {
        Self::start_with_args(
            clock,
            &[
                "--receipt-mode",
                "new-event",
                "--questionnaire-version",
                "goal-cup-2026-v1",
            ],
        )
    }

    fn start_with_args(clock: &str, extra: &[&str]) -> Devserver {
        let mut child = Command::new(env!("CARGO_BIN_EXE_goalcup-devserver"))
            .args(["--addr", "127.0.0.1:0", "--clock", clock])
            .args(extra)
            .stderr(Stdio::piped())
            .spawn()
            .expect("the devserver starts");
        let stderr = child.stderr.take().expect("stderr is piped");
        // The banner comes first and the bound address after it, both on stderr by design:
        // there is no quiet mode, because an operator who cannot see what the key is worth
        // is the one failure that crate can cause. `ChildLines` drains the rest of the
        // stream for the life of the process, so a full pipe never blocks those writes.
        let lines = ChildLines::spawn(stderr);
        let url = lines.find("where the devserver is listening", |line| {
            line.strip_prefix("listening on ").map(str::to_string)
        });
        Devserver { child, url }
    }

    pub fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Devserver {
    fn drop(&mut self) {
        self.stop();
    }
}

pub struct DaemonProcess {
    pub child: Child,
    pub control_addr: String,
    pub control_file: PathBuf,
}

impl DaemonProcess {
    /// Start it exactly as `README.md` says to, and read the three facts it prints.
    pub fn start(config: &Path) -> DaemonProcess {
        // `{home}` is deliberately the participant's real home in production, because the
        // foreign scan and config allowlist must inspect files the tracker did not write. A
        // gate, however, must never inspect its developer's real home: its temporary config
        // parent is the complete test machine. Canonicalising first keeps `HOME` absolute,
        // matching `participant_home`'s production contract.
        let config_parent = config.parent().expect("the test config has a parent");
        let participant_home = std::fs::canonicalize(config_parent)
            .expect("the test config parent canonicalises")
            .join("participant-home");
        std::fs::create_dir_all(&participant_home).expect("the isolated participant home exists");
        let mut child = Command::new(sibling("goalcup-daemon"))
            .arg("--config")
            .arg(config)
            .arg("--foreground")
            .env("HOME", &participant_home)
            .stdout(Stdio::piped())
            .spawn()
            .expect("the daemon starts");
        let stdout = child.stdout.take().expect("stdout is piped");
        let lines = ChildLines::spawn(stdout);
        let control_addr = lines.next_line("the daemon's control address");
        let control_file = PathBuf::from(lines.next_line("the daemon's control file path"));
        DaemonProcess {
            child,
            control_addr,
            control_file,
        }
    }

    /// The CLI's bearer, read out of `control.json` the way `goalcup doctor` reads it.
    pub fn bearer(&self) -> String {
        let text = std::fs::read_to_string(&self.control_file).expect("control.json");
        let parsed: serde_json::Value = serde_json::from_str(&text).expect("control.json is JSON");
        parsed["token"]
            .as_str()
            .expect("control.json carries the bearer")
            .to_string()
    }

    pub fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for DaemonProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The configuration file an operator writes, with every required field and nothing else.
///
/// The cadences are turned down so the daemon's own loop comes round inside a test rather
/// than inside a run: this is the file README documents, with four numbers changed. The
/// three health thresholds are pushed past any clock this test reaches — no check in the
/// verifier reads `EventBody::Health`, and leaving them at their defaults would chain a
/// `CaptureStalled` for a run that lasted five seconds.
pub fn write_config(root: &Path, upload_url: &str) -> PathBuf {
    let connectors = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../connectors");
    let connectors = std::fs::canonicalize(connectors).expect("the shipped connectors");
    let path = root.join("goalcup-daemon.toml");
    let text = format!(
        r#"
artifact_root = "{artifacts}"
connectors_dir = "{connectors}"
config_dir = "{config}"
workspace_parent = "{workspaces}"
control_addr = "127.0.0.1:0"
upload_url = "{upload_url}"
server_public_key = "{key}"

poll_interval_ms = 100
upload_interval_ms = 200
status_interval_ms = 100
stall_after_seconds = 1000000000.0
silence_window_seconds = 1000000000.0
ack_lag_seconds = 1000000000.0
clock_skew_seconds = 1000000000.0
disk_reserve_bytes = 0

[event]
t0 = "{t0}"
start_window_closes_at = "{closes}"
rules_version = "{rules}"
"#,
        t0 = ROUND_T0,
        closes = ROUND_CLOSES,
        rules = ROUND_RULES,
        artifacts = root.join("holder").display(),
        connectors = connectors.display(),
        config = root.join("config").display(),
        workspaces = root.join("workspaces").display(),
        key = goalcup_devserver::dev_server_public_key_hex(),
    );
    std::fs::write(&path, text).expect("the configuration file");
    path
}

/// The same operator configuration as [`write_config`], with every V4 workspace-submission
/// field explicit. Keeping this additive avoids making the legacy API gates silently change
/// receipt contracts while giving the real V4 gate a complete, signed input shape.
pub fn write_workspace_submission_config(root: &Path, upload_url: &str) -> PathBuf {
    let path = write_config(root, upload_url);
    let mut text = std::fs::read_to_string(&path).expect("the base configuration file");
    text.push_str(
        r#"

[receipt_binding]
mode = "workspace_submission"
questionnaire_version = "goal-cup-2026-v1"
submission_profile = "workspace_snapshot_v1"

[receipt_binding.limit_profile]
id = "goal-cup-v1-local"

[receipt_binding.effective_limits]
object_count = 4096
total_bytes = 1073741824
pre_cutoff_drain = 5000
process_walk = 64

[receipt_binding.capture_limits]
version = 1
namespace_nodes = 8192
inventory_bytes = 8388608
manifest_bytes = 8388608
"#,
    );
    std::fs::write(&path, text).expect("the V4 configuration file");
    path
}

// ------------------------------------------------------------------ the two clients

/// One response, read to EOF under `Connection: close`.
pub struct Res {
    pub status: u16,
    pub body: String,
}

impl Res {
    /// The transient half of `409`: the daemon's body says `busy`. See [`Api::post`].
    pub fn is_busy(&self) -> bool {
        self.status == 409
            && serde_json::from_str::<serde_json::Value>(&self.body)
                .ok()
                .and_then(|v| v.get("error")?.get("code")?.as_str().map(str::to_string))
                .as_deref()
                == Some("busy")
    }
}

/// The control API, with the bearer `control.json` carried.
pub struct Api {
    pub addr: String,
    pub token: String,
}

impl Api {
    /// `POST`, retried while the daemon answers the body code `busy`.
    ///
    /// **`busy` and not the status `409`**. Two opposite answers share that status:
    /// `busy` is an entry out of the entry map for the length of one pump — a call that
    /// would have to hold the slot across it is told "mid-upload" rather than made to wait,
    /// and with an upload cadence of 200 ms a caller meets that constantly — while
    /// `conflict` is the entry itself saying no, and will still be saying no in ten seconds.
    /// Retrying on the status would spend the whole patience budget against a genuine refusal
    /// before reporting it. **This is the rule every client follows: retry on `busy`,
    /// report `conflict`**, and it is written down here because this is where it is
    /// demonstrated.
    pub fn post(&self, path: &str, body: &str) -> Res {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let r = self.post_once(path, body);
            if !r.is_busy() || Instant::now() >= deadline {
                return r;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    pub fn post_once(&self, path: &str, body: &str) -> Res {
        let version = CONTROL_API_VERSION.to_string();
        request(
            &self.addr,
            "POST",
            path,
            &[
                ("Authorization", &format!("Bearer {}", self.token)),
                ("X-GoalCup-Control-Version", &version),
            ],
            body,
        )
    }

    pub fn post_body<B: serde::Serialize>(&self, path: &str, body: &B) -> Res {
        self.post(path, &serde_json::to_string(body).expect("serialises"))
    }

    pub fn post_json<B: serde::Serialize, T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> T {
        let r = self.post_body(path, body);
        assert_eq!(r.status, 200, "{path}: {}", r.body);
        serde_json::from_str(&r.body).expect("the contract's type")
    }

    pub fn get(&self, path: &str) -> Res {
        let version = CONTROL_API_VERSION.to_string();
        request(
            &self.addr,
            "GET",
            path,
            &[
                ("Authorization", &format!("Bearer {}", self.token)),
                ("X-GoalCup-Control-Version", &version),
            ],
            "",
        )
    }

    pub fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str, why: &str) -> T {
        let r = self.get(path);
        assert_eq!(r.status, 200, "{why}: {}", r.body);
        serde_json::from_str(&r.body).expect("the contract's type")
    }
}

/// The hook receiver, with this run's own bearer (D24) and the counter a hook stamps.
pub struct Receiver {
    pub addr: String,
    pub token: String,
}

impl Receiver {
    pub fn post(&self, event: &str, seq: u64, body: serde_json::Value) {
        let r = request(
            &self.addr,
            "POST",
            &format!("/{event}"),
            &[
                ("X-GoalCup-Token", &self.token),
                ("X-GoalCup-Event", event),
                ("X-GoalCup-Seq", &seq.to_string()),
                ("X-GoalCup-Source", "hook"),
            ],
            &body.to_string(),
        );
        assert_eq!(
            r.status, 200,
            "the receiver refused a {event} post: {}",
            r.body
        );
    }
}

/// A few lines of HTTP/1.1 over `std::net::TcpStream`, for the reason every other harness
/// in this workspace has its own: there is no client crate here, and a dependency added so a
/// test can speak the protocol it already serves is a dependency the workspace then carries.
pub fn request(addr: &str, method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> Res {
    let mut stream =
        std::net::TcpStream::connect(addr).unwrap_or_else(|e| panic!("connect to {addr}: {e}"));
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).expect("write the head");
    stream.write_all(body.as_bytes()).expect("write the body");
    let mut raw = Vec::new();
    let _ = stream.read_to_end(&mut raw);
    let text = String::from_utf8_lossy(&raw).to_string();
    let (head, body) = match text.split_once("\r\n\r\n") {
        Some((h, b)) => (h, b.to_string()),
        None => (text.as_str(), String::new()),
    };
    let status = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .expect("a status line");
    Res { status, body }
}

/// The public status document, off the route a browser polls, with its signature checked
/// against the key the API handed the CLI.
pub fn status(api: &Api, info: &WorkspaceInfo) -> SignedStatus {
    let signed: SignedStatus = api.get_json(
        &format!("{ROUTE_ENTRIES}/{}/status", info.entry_id),
        "the status route",
    );
    assert!(
        signed.verify_with(&info.run_public_key).is_ok(),
        "the status document verifies under the run key"
    );
    signed
}

/// Wait for one of the daemon's cadences to produce a fact, and answer it.
///
/// A condition, never a duration: the daemon runs its own loop in another process, and a
/// `sleep` long enough to be reliable on a loaded machine is a test that costs that long
/// every time it passes.
pub fn wait_for<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + PATIENCE;
    loop {
        if let Some(value) = f() {
            return value;
        }
        assert!(
            Instant::now() < deadline,
            "waited {PATIENCE:?} for {what} and it did not happen"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Wait until the chain holds `records` leg-1 records and `posts` leg-2 posts.
///
/// The failure prints the chain a line at a time, because the two ways this can go wrong
/// look identical from a counter: nothing was captured, and something was captured under a
/// source or a kind the count did not expect.
pub fn wait_for_chain(trace_dir: &Path, records: usize, posts: usize) {
    let deadline = Instant::now() + PATIENCE;
    let mut seen = (0, 0);
    loop {
        seen = chained(trace_dir).unwrap_or(seen);
        if seen.0 >= records && seen.1 >= posts {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the chain holds {} leg-1 records and {} leg-2 posts, not {records} and {posts}.\n{}",
            seen.0,
            seen.1,
            summarise(trace_dir)
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// The chain as one line per link: sequence, source, kind. No payloads — a failure message
/// that quoted a record would put the run's own text into a test log.
pub fn summarise(trace_dir: &Path) -> String {
    std::fs::read_to_string(trace_dir.join(goalcup_protocol::export::CHAIN_FILE))
        .unwrap_or_default()
        .lines()
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap_or_default();
            format!("  seq {} {} {}", v["seq"], v["source"], v["body"]["kind"])
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// How many leg-1 records and leg-2 posts the chain holds.
pub fn chained(trace_dir: &Path) -> Option<(usize, usize)> {
    let text =
        std::fs::read_to_string(trace_dir.join(goalcup_protocol::export::CHAIN_FILE)).ok()?;
    let mut records = 0;
    let mut posts = 0;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        // `EventBody` is adjacently tagged and NOT flattened, so the discriminant is
        // `body.kind`. `Link` is flattened, which is why `seq` above is top-level.
        match v["body"]["kind"].as_str() {
            Some("leg1_record") => records += 1,
            Some("leg2_post") => posts += 1,
            _ => {}
        }
    }
    Some((records, posts))
}

/// The sequence of the last link in the chain, which after the seal is the seal link.
pub fn chain_head(trace_dir: &Path) -> Option<u64> {
    let text =
        std::fs::read_to_string(trace_dir.join(goalcup_protocol::export::CHAIN_FILE)).ok()?;
    let last = text.lines().rfind(|l| !l.trim().is_empty())?;
    // `Link` is `#[serde(flatten)]`ed into the envelope, so `seq` is a top-level key.
    serde_json::from_str::<serde_json::Value>(last).ok()?["seq"].as_u64()
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

pub fn parse_secs(at: &str) -> u64 {
    goalcup_protocol::control::parse_z(at).unwrap_or_else(|| panic!("{at} is an instant"))
}

// ------------------------------------------------------------------ the synthetic harness
//
// The record and post shapes, parameterised by the session the API minted
// rather than by a constant — which is the one thing that differs from `support::roundtrip`,
// whose entry and session are its own.

pub fn append(path: &Path, line: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("the parent directory");
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("the store opens for append");
    writeln!(file, "{line}").expect("the line is appended");
}

/// The options `goalcup start` posts, with the manifest digest read off the shipped file:
/// the daemon refuses a manifest that is not the one in the release, so a
/// plausible hex string would test the refusal instead of the route.
pub fn opts(connector_id: &str) -> WorkspaceOpts {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../connectors")
        .join(connector_id);
    let bytes = std::fs::read(dir.join("manifest.toml")).expect("the shipped manifest");
    // No connector-directory digest is sent: the daemon hashes its own connectors directory
    // and compares that against the pins itself. These gates point it at the shipped one, so
    // the value it computes is the pinned value and the receipt it asks for claims it.
    WorkspaceOpts {
        connector_id: connector_id.to_string(),
        manifest_sha256: goalcup_daemon::workspace::sha256_hex(&bytes),
        workspace_parent: None,
        attach: false,
        deadline_hours: 168,
        grant_token: "grant-api-gate".to_string(),
        arming_timeout_seconds: None,
        // The number the CLI's preflight measured, which the daemon copies onto the entry
        // row rather than probing for itself.
        harness_version: Some("0.0.0-gate".to_string()),
    }
}

/// How many leg-1 records of one store the chain holds.
///
/// `chained` counts every store together, which is the right question for "has the tick
/// caught up". This is the other one: which STORE a record came from is the whole fact a
/// partition test is about.
///
/// The store is read off `Envelope::source`, which is the store name for a leg-1 record —
/// `EventBody` is adjacently tagged, so its own fields sit under `body.content` rather than
/// beside `body.kind`, and `source` is the copy at the top level.
pub fn leg1_records(trace_dir: &Path, store: &str) -> usize {
    let text = std::fs::read_to_string(trace_dir.join(goalcup_protocol::export::CHAIN_FILE))
        .unwrap_or_default();
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| {
            v["body"]["kind"].as_str() == Some("leg1_record") && v["source"].as_str() == Some(store)
        })
        .count()
}

/// Today, as `{yyyy}`, `{mm}` and `{dd}` render — the same three components
/// `control::today` fills, derived the same way (the date half of an RFC 3339 instant), so
/// a store template's date-partitioned directory can be built from a test.
pub fn today() -> (String, String, String) {
    let now = goalcup_protocol::control::format_z(now_unix());
    let date = now.split('T').next().unwrap_or("1970-01-01");
    let mut parts = date.split('-');
    (
        parts.next().unwrap_or("1970").to_string(),
        parts.next().unwrap_or("01").to_string(),
        parts.next().unwrap_or("01").to_string(),
    )
}
