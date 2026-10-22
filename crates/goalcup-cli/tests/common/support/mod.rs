//! The shared test surface for `goalcup-cli`; a test file pulls it in with `mod support;`.
//!
//! A helper written twice under two names is two tests that agree today and drift tomorrow,
//! so a helper a second call site needs is added here rather than at the call site. A name
//! keeps its signature; a body may grow, and the item's own documentation says why.
//!
//! Rules the items follow:
//!
//! * A helper does what its signature says. There is no `mock(info)` here: `MockDaemonClient`
//!   builds its own entry record, and one that took an `info` and ignored it would lie
//!   about what the mock hands back.
//! * A fixture never runs a real harness. The shipped recipes launch `claude` and `codex`,
//!   which hang on a machine that has one installed and fail on one that has not, so a
//!   rewritten recipe runs `sh` or `true` instead, and
//!   [`clean_env_with_no_harness_installed`] keeps D12's version gate from measuring the
//!   developer's own harness.
//! * A rewritten connector no longer matches the manifest pin: `pin_state` calls it
//!   `Altered` and a start against it needs `--dev`. That is the door behaving as it
//!   should, not a weakening of the pin.
//! * Manifests are edited line by line, section by section, never through a TOML round
//!   trip, which would rewrite every comment in the shipped rulebook the fixtures keep.
//! * An environment starts empty ([`clean_env`] is `PATH` and nothing else) and a home is a
//!   throwaway directory, so no test inherits the developer's `GOALCUP_*` variables, harness
//!   home or credentials.
//! * A wait is on a condition, not a duration: [`PATIENCE`] is a ceiling reached only when
//!   the condition cannot come true, and a wait that gives up names what it waited for.
//!
//! `dead_code` is allowed throughout: every test binary in this directory compiles all of
//! this file, and no single one uses all of it.

#![allow(dead_code)]

use goalcup_protocol::control;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// One clock for every render. A test that formatted `now` would pass on the day it was
/// written and be a different test tomorrow.
pub const NOW: &str = "2026-09-05T00:00:00Z";

/// The longest any wait in this module gives a condition to come true. Generous, because it
/// is only ever reached when the condition is not going to come true at all.
pub const PATIENCE: Duration = Duration::from_secs(20);

// ------------------------------------------------------------------ the tree

/// The workspace root: `CARGO_MANIFEST_DIR/../..`.
pub fn repo_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

/// The shipped `connectors/` — in a checkout, this repository's.
pub fn connectors_dir() -> PathBuf {
    repo_root().join("connectors")
}

/// The shipped connector `id`, loaded from [`connectors_dir`] by its `harness.toml`.
pub fn connector(id: &str) -> goalcup_cli::connector::HarnessConnector {
    match goalcup_cli::connector::load_harness(&connectors_dir(), id, false) {
        Ok(c) => c,
        Err(e) => panic!("the shipped connector {id}: {e}"),
    }
}

/// The fixture connectors, one per launch shape: `tests/fixtures/connectors/<id>/`. An engine
/// test loads one of these, never a shipped connector, so a shipped manifest edit changes no
/// engine test. `chat-per-run` is the per-run-home shape; `chat-shared-home` is the shared-home
/// shape; both are chat mode (D57). No build pins them: they load through `--dev`.
pub fn fixture_connectors_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/connectors")
}

/// The fixture connector `id`, loaded the way `goalcup start --dev` loads it.
pub fn fixture_connector(id: &str) -> goalcup_cli::connector::HarnessConnector {
    match goalcup_cli::connector::load_harness(&fixture_connectors_dir(), id, true) {
        Ok(c) => c,
        Err(e) => panic!("the fixture connector {id}: {e}"),
    }
}

/// `tests/fixtures/connectors/<id>/` copied to `<root>/<folder>/` under the declared id `as_id`,
/// so a test can edit a copy. Returns the copy's folder.
pub fn copy_fixture_connector(id: &str, root: &Path, folder: &str, as_id: &str) -> PathBuf {
    let dir = root.join(folder);
    copy_tree(&fixture_connectors_dir().join(id), &dir);
    let path = dir.join("harness.toml");
    let text = std::fs::read_to_string(&path).unwrap();
    let declared = format!("id = \"{id}\"");
    assert!(text.contains(&declared), "{id}: no `{declared}`");
    std::fs::write(
        &path,
        text.replacen(&declared, &format!("id = \"{as_id}\""), 1),
    )
    .unwrap();
    dir
}

/// `connectors/<id>/` copied to `<tmp>/<id>/`, whole — the `harness.toml` and any `files/`
/// beside it — so a test can edit a copy without touching the pinned original.
pub fn copy_connector(tmp: &tempfile::TempDir, id: &str) {
    copy_tree(&connectors_dir().join(id), &tmp.path().join(id));
}

/// Every shipped connector copied under `dir`, which is created if it is not there.
///
/// [`copy_connector`] puts a single connector at the root of a `TempDir`; the layouts
/// `config::connectors_dir` looks for relative to the executable need the whole set several
/// directories down.
pub fn copy_connectors_into(dir: &Path) {
    if let Err(e) = std::fs::create_dir_all(dir) {
        panic!("{}: {e}", dir.display());
    }
    let shipped = connectors_dir();
    let entries = match std::fs::read_dir(&shipped) {
        Ok(entries) => entries,
        Err(e) => panic!("{}: {e}", shipped.display()),
    };
    for entry in entries.flatten() {
        if entry.path().is_dir()
            && let Some(name) = entry.path().file_name()
        {
            copy_tree(&entry.path(), &dir.join(name));
        }
    }
}

/// A connectors directory holding one connector whose `harness.toml` is **not** the one this
/// build pins: `connectors/<id>/`, copied, with a comment appended.
///
/// A comment and not a value, deliberately. What the release-hash allowlist compares is the
/// directory's bytes, so the smallest possible edit is the honest fixture: it proves the digest
/// is over the file rather than over a re-serialisation of the parsed manifest, and it leaves
/// a manifest that still parses and still validates — which is exactly the case a participant
/// produces, and exactly the case that would otherwise pass unnoticed.
///
/// Leaked like [`tmp_cfg`], so the caller gets a `PathBuf` without holding a guard.
pub fn connectors_dir_with_edited_harness(id: &str) -> PathBuf {
    let root = empty_cfg();
    copy_tree(&connectors_dir().join(id), &root.join(id));
    let harness = root.join(id).join("harness.toml");
    let text = match std::fs::read_to_string(&harness) {
        Ok(text) => text,
        Err(e) => panic!("{}: {e}", harness.display()),
    };
    let edited =
        format!("{text}\n# edited by a test: this harness.toml is no longer the pinned one\n");
    if let Err(e) = std::fs::write(&harness, edited) {
        panic!("{}: {e}", harness.display());
    }
    root
}

/// The harness config a connector's `capture.model_host_from` names, written into its shared
/// config home `home` (made 0700) with `url` at the key, in the format the file's extension
/// says (D62): what a participant's own setup of the harness leaves there.
#[cfg(unix)]
pub fn write_model_host_config(
    home: &Path,
    from: &goalcup_protocol::harness::ModelHostFrom,
    url: &str,
) {
    use std::os::unix::fs::PermissionsExt;
    let parts: Vec<&str> = from.key.split('.').collect();
    let (last, tables) = parts.split_last().unwrap();
    let text = match from.file.rsplit_once('.').map(|(_, e)| e) {
        Some("json") => {
            let mut value = serde_json::json!(url);
            for part in parts.iter().rev() {
                value = serde_json::json!({ *part: value });
            }
            value.to_string()
        }
        Some("toml") if tables.is_empty() => format!("{last} = \"{url}\"\n"),
        Some("toml") => format!("[{}]\n{last} = \"{url}\"\n", tables.join(".")),
        _ => {
            let mut text = String::new();
            for (depth, part) in tables.iter().enumerate() {
                text.push_str(&format!("{}{part}:\n", "  ".repeat(depth)));
            }
            text.push_str(&format!("{}{last}: {url}\n", "  ".repeat(tables.len())));
            text
        }
    };
    let path = home.join(&from.file);
    for dir in [home.parent().unwrap(), home, path.parent().unwrap()] {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    std::fs::write(&path, text).unwrap();
}

/// `from` copied to `to`, recursively, creating what it needs. File modes come along, which
/// is what keeps an executable shim executable.
pub fn copy_tree(from: &Path, to: &Path) {
    if let Err(e) = std::fs::create_dir_all(to) {
        panic!("{}: {e}", to.display());
    }
    let entries = match std::fs::read_dir(from) {
        Ok(entries) => entries,
        Err(e) => panic!("{}: {e}", from.display()),
    };
    for entry in entries.flatten() {
        let source = entry.path();
        let Some(name) = source.file_name() else {
            continue;
        };
        let target = to.join(name);
        if source.is_dir() {
            copy_tree(&source, &target);
        } else if let Err(e) = std::fs::copy(&source, &target) {
            panic!("{} -> {}: {e}", source.display(), target.display());
        }
    }
}

/// `tests/fixtures/<name>`, this crate's own.
pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

/// Every file under `<repo_root>/<rel>`, recursively, sorted.
pub fn walk(rel: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect(&repo_root().join(rel), &mut out);
    out.sort();
    out
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else {
            out.push(path);
        }
    }
}

// ------------------------------------------------------------------ config dirs

/// A configuration directory that outlives the test: a `TempDir` deliberately leaked, so a
/// helper can hand back a `PathBuf` without the caller having to keep a guard alive for it.
///
/// Leaked and not cleaned, because the alternative is a directory removed the moment this
/// function returns and a test that reads an empty path for reasons nothing in it explains.
/// The operating system's temporary directory is where it lands.
pub fn tmp_cfg() -> PathBuf {
    let dir = match tempfile::tempdir() {
        Ok(dir) => dir.keep(),
        Err(e) => panic!("a temporary configuration directory: {e}"),
    };
    if let Err(e) = std::fs::create_dir_all(dir.join("entries")) {
        panic!("the entries directory under {}: {e}", dir.display());
    }
    dir
}

/// [`tmp_cfg`] without the `entries/` directory: a configuration directory a `goalcup
/// start` has never run against.
pub fn empty_cfg() -> PathBuf {
    match tempfile::tempdir() {
        Ok(dir) => dir.keep(),
        Err(e) => panic!("a temporary configuration directory: {e}"),
    }
}

/// [`clean_env`] with a `PATH` no harness is on.
///
/// D12's version gate measures the harness **on the machine the test
/// is running on**: a developer with `codex` at 0.143.0 and a connector tested at `=0.153.0`
/// fails every command test that loads that connector, and the failure is about their
/// laptop rather than about the code. A binary that is not on `PATH` at all is the case the
/// gate answers with a `Warn` — "a machine without the harness installed is a report, not a
/// broken tracker" — so that is the case these tests ask for, deliberately and in one place.
///
/// It does not weaken the launch: `launch::spawn` resolves its program on **this process's**
/// `PATH`, not on the map handed to `start_with`, so a test whose recipe runs `sh` still
/// runs `sh`. The gate itself is tested where it belongs, against synthetic manifests, in
/// `tests/it/preflight_doctor.rs`.
pub fn clean_env_with_no_harness_installed() -> BTreeMap<String, String> {
    let mut env = clean_env();
    env.insert("PATH".to_string(), "/nonexistent".to_string());
    env
}

/// `PATH` and nothing else, ever. A child launched with the real environment inherits the
/// developer's `GOALCUP_*` variables, their harness home and their credentials, and a test
/// that passed because of one of those is a test about the machine it ran on.
pub fn clean_env() -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    let path = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_string());
    env.insert("PATH".to_string(), path);
    env
}

// ------------------------------------------------------------------ a start, mocked

/// A mock holding one armed entry for `id`, with `tmp` as its configuration directory.
///
/// It writes no `grant.json`: nothing has read one since D48 removed `goalcup auth`.
pub fn mock_armed(tmp: &tempfile::TempDir, id: &str) -> goalcup_cli::daemon::MockDaemonClient {
    goalcup_cli::daemon::MockDaemonClient::armed_ok(tmp.path(), id)
}

/// `tmp` as a configuration directory that already holds the record one `goalcup start`
/// would have left for `info`'s entry, and the path to it.
///
/// The pinned run key is what matters: `goalcup status` and `goalcup seal` are fresh processes,
/// the key they check a signed document against is the one `start` wrote down here, and a
/// test that handed them a configuration directory without this file would be testing the
/// refusal rather than the render. `MockDaemonClient::info` is that record.
pub fn cfg_with_entry(tmp: &Path, info: &goalcup_cli::entries::EntryRecord) -> PathBuf {
    if let Err(e) = goalcup_cli::entries::write_entry(tmp, info) {
        panic!("the cached entry record under {}: {e}", tmp.display());
    }
    tmp.to_path_buf()
}

/// A run's disk sizes for a test: 4 MiB held for the run, no start margin, a tick every 50 ms,
/// and chains limited to `quota` bytes. No test holds the 3 GiB a participant's run does.
pub fn disk_of(quota: u64) -> goalcup_cli::run::reserve::DiskPolicy {
    goalcup_cli::run::reserve::DiskPolicy {
        reserve_bytes: 4 << 20,
        start_margin: 0,
        free_floor: goalcup_cli::run::reserve::FREE_FLOOR,
        chain_quota: quota,
        tick: std::time::Duration::from_millis(50),
        // A test never pauses its harness for the machine's own disk.
        pause_below: 0,
        resume_above: 0,
    }
}

/// The opener every test start is given: no browser is ever started by a test.
pub fn never_open(_url: &str) -> bool {
    false
}

/// What one `goalcup start` was asked to do, with the flag a test must never leave on.
///
/// `dev` is `false`, so the default options are a judged run's — a test that wants the door
/// open opens it in one line at the call site that is about it. No `run_folder`, so the run is
/// made in this process's current directory; a test that starts one sets it.
pub fn start_options(id: &str) -> goalcup_cli::commands::start::StartOptions {
    goalcup_cli::commands::start::StartOptions {
        harness: id.to_string(),
        dev: false,
        run_folder: None,
        disk: disk_of(4 << 20),
        native_silence: Default::default(),
        mode: goalcup_cli::account::device::RunMode::Competition,
        open_url: never_open,
        upload_policy: Default::default(),
        sleep_lock: goalcup_platform::awake::hold,
    }
}

/// A mock holding one entry that is ready to be sealed, with `tmp` made into a
/// configuration directory the way [`mock_armed`] makes one.
///
/// It adds **nothing to the fake**: `armed_ok` already answers the first seal and a
/// `conflict` on every one after it, and `make_sealed_manifest_differ_on` and
/// `seal_once_then_conflict` cover the rest. What it adds is on disk, because the seal's
/// subject is a workspace and a directory the daemon exported:
///
/// * **One file in the workspace**, `hello.py`. Exactly one, deliberately: the mock's sealed
///   manifest starts empty, so a test that wants a disagreement asks for one by name and
///   reads a report with that disagreement in it and nothing else.
/// * **`<artifact_root>/trace`**, holding the four documents `TraceExport` writes. `goalcup
///   seal` writes not one byte in there, and the only way to assert that is against a
///   directory that already has bytes in it.
pub fn mock_sealable(tmp: &tempfile::TempDir, id: &str) -> goalcup_cli::daemon::MockDaemonClient {
    use goalcup_protocol::export::{CHAIN_FILE, DESCRIPTOR_FILE, RECEIPT_FILE, SEAL_FILE};

    let m = mock_armed(tmp, id);
    let info = m.info();
    put(&Path::new(&info.workspace).join("hello.py"), b"print(1)\n");
    let trace = Path::new(&info.artifact_root).join("trace");
    put(&trace.join(CHAIN_FILE), b"{\"seq\":1}\n");
    put(&trace.join(RECEIPT_FILE), b"{}\n");
    put(&trace.join(SEAL_FILE), b"{}\n");
    put(&trace.join(DESCRIPTOR_FILE), b"entry_id = \"mock\"\n");
    m
}

/// `bytes` at `path`, with the directories above it, or a panic naming the path.
fn put(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        panic!("{}: {e}", parent.display());
    }
    if let Err(e) = std::fs::write(path, bytes) {
        panic!("{}: {e}", path.display());
    }
}

// ------------------------------------------------------------------ the binaries

/// The binary `name`, beside the one cargo built for this package.
///
/// `CARGO_BIN_EXE_goalcup` is the only variable cargo sets for this crate. The others are
/// built by `cargo test --workspace --offline` — the gate command — and land in the same
/// directory, so they are resolved by name there. A missing one fails loudly naming the
/// command that builds it rather than skipping silently.
fn sibling(name: &str) -> PathBuf {
    let me = PathBuf::from(env!("CARGO_BIN_EXE_goalcup"));
    let path = me.with_file_name(name);
    assert!(
        path.is_file(),
        "{} is not built. This test drives the shipped binaries, which \
         `cargo test --workspace --offline` builds; one crate at a time does not.",
        path.display()
    );
    path
}

/// The `goalcup-daemon` binary.
pub fn daemon_binary() -> PathBuf {
    sibling("goalcup-daemon")
}

/// The `goalcup-verify` binary.
pub fn verify_binary() -> PathBuf {
    sibling("goalcup-verify")
}

// ------------------------------------------------------------------ waits and pids

/// Wait until `path` exists, or fail naming it.
pub fn wait_for_file(path: &Path) {
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline {
        if path.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("{} never appeared within {PATIENCE:?}", path.display());
}

/// `cargo tree -p <pkg> --offline`, so a task can assert what a crate does and does not
/// link rather than reading its manifest and hoping.
pub fn cargo_tree(pkg: &str) -> String {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let out = match std::process::Command::new(cargo)
        .args(["tree", "-p", pkg, "--offline"])
        .current_dir(repo_root())
        .output()
    {
        Ok(out) => out,
        Err(e) => panic!("`cargo tree -p {pkg} --offline`: {e}"),
    };
    assert!(
        out.status.success(),
        "`cargo tree -p {pkg} --offline` failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

// ------------------------------------------------------------------ a daemon that records

/// A listener on the loopback that answers a fixed script and remembers what it was asked.
///
/// The bearer rule is a WIRE property — `MockDaemonClient` is in-process and has no headers
/// to hold — so the only honest place to assert "public routes carry no token and every
/// other route carries one" is against a real socket and a real `HttpDaemonClient`. This is
/// that socket.
///
/// It speaks the smallest HTTP/1.1 a `ureq` request needs: one request per connection,
/// `Content-Length` on the way out, `Connection: close` so nothing is pooled, and — the one
/// detail that is not cosmetic — **the whole request body is read before a response is
/// written**. A server that answers early leaves those bytes in its receive queue and its
/// close sends an RST, which ureq 2.12 meets with an `.expect()` inside the library
/// (`commands::auth` documents the same hazard from the other side).
pub struct RecordingDaemon {
    base: String,
    log: std::sync::Arc<std::sync::Mutex<Recorded>>,
}

/// What the listener has been asked so far.
#[derive(Default)]
struct Recorded {
    /// The raw request target — path AND query, exactly as it arrived — and the
    /// `Authorization` header, or `None` where the request carried none.
    last: Option<(String, Option<String>)>,
    last_body: Option<String>,
    attempts: usize,
}

/// What a [`RecordingDaemon`] answers with.
enum Answer {
    /// `200` and an empty JSON object. Typed calls fail to parse it and this is deliberate:
    /// the tests that use it assert what went OUT, and a fixture body per route would be a
    /// second copy of the daemon's own answers.
    Empty,
    /// The same refusal every time, in the contract's `ErrorBody` shape.
    Refusal {
        status: u16,
        code: String,
        message: String,
    },
    /// `409`/`busy` once, then [`Answer::Empty`] — the transient the daemon raises on every
    /// mutating route while an entry is mid-pump.
    BusyOnce,
    /// `200` and a JSON string of this many bytes, for the client's response cap.
    Oversized(usize),
    /// `200`, a `Content-Length` far larger than the bytes that follow it, and then the
    /// close — a daemon that stopped mid-answer.
    Truncated,
    /// `200` and a body that is not UTF-8.
    NonUtf8,
    /// One exact wire response, including deliberately missing or repeated version headers.
    Raw {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
    /// A deliberately invalid 204 body that uses transfer framing and carries no
    /// `Content-Length`, for the client boundary that `ureq` normalizes away.
    RawChunked {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
}

/// One raw HTTP response assembled by the test listener. `None` deliberately omits
/// `Content-Length`, which is how the chunked-204 fixture reaches `ureq` unchanged.
type ScriptedResponse = (u16, Vec<(String, String)>, Vec<u8>, Option<usize>);

impl RecordingDaemon {
    /// `http://127.0.0.1:<port>`, with no trailing slash: a route is appended to it whole.
    pub fn base(&self) -> String {
        self.base.clone()
    }

    /// The raw target and the `Authorization` header of the last request served.
    pub fn last(&self) -> (String, Option<String>) {
        match self.log.lock() {
            Ok(log) => match &log.last {
                Some(last) => last.clone(),
                None => panic!("nothing has been asked of this recording daemon yet"),
            },
            Err(e) => panic!("the recording daemon's log: {e}"),
        }
    }

    /// The body of the last request, so a test can assert what went out.
    pub fn last_body(&self) -> Option<String> {
        match self.log.lock() {
            Ok(log) => log.last_body.clone(),
            Err(e) => panic!("the recording daemon's log: {e}"),
        }
    }

    /// Requests served, so a retry is counted rather than inferred.
    pub fn attempts(&self) -> usize {
        match self.log.lock() {
            Ok(log) => log.attempts,
            Err(e) => panic!("the recording daemon's log: {e}"),
        }
    }
}

/// A daemon that answers `200 {}` and remembers everything.
pub fn recording_daemon() -> RecordingDaemon {
    serve(Answer::Empty)
}

/// A daemon that answers this refusal, in the contract's `ErrorBody` shape, every time.
pub fn erroring_daemon(status: u16, code: &str, message: &str) -> RecordingDaemon {
    serve(Answer::Refusal {
        status,
        code: code.to_string(),
        message: message.to_string(),
    })
}

/// Answers `409`/`busy` once and `200` thereafter, for the retry.
pub fn recording_daemon_busy_once() -> RecordingDaemon {
    serve(Answer::BusyOnce)
}

/// A daemon whose `200` is larger than any answer a control route has.
///
/// The client refuses a response body over its cap rather than
/// reading one into memory whole, and that refusal has no other fixture. `bytes` is the
/// length of the JSON string it answers with, so a caller names a size on either side of
/// the cap and does not have to know how the body is framed.
pub fn oversized_daemon(bytes: usize) -> RecordingDaemon {
    serve(Answer::Oversized(bytes))
}

/// A daemon that promises a body and then stops partway through it.
///
/// It declares a `Content-Length` it does not send and
/// closes, which is the wire shape of a daemon killed mid-answer; the client must call that
/// `Malformed` and not `Unreachable`, because "the daemon is not reachable" is the sentence
/// a participant acts on by restarting a daemon that was in fact answering.
pub fn truncated_daemon() -> RecordingDaemon {
    serve(Answer::Truncated)
}

/// A daemon whose `200` is not UTF-8; see
/// [`truncated_daemon`] for why this is not an unreachable daemon either.
pub fn non_utf8_daemon() -> RecordingDaemon {
    serve(Answer::NonUtf8)
}

/// A daemon whose response is named byte-for-byte by a wire-contract test.
pub fn raw_response_daemon(
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
) -> RecordingDaemon {
    serve(Answer::Raw {
        status,
        headers,
        body,
    })
}

/// A response whose bytes are chunked without a `Content-Length`.  This is a separate raw
/// fixture because the normal listener deliberately writes lengths for every ordinary test.
pub fn raw_chunked_response_daemon(
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
) -> RecordingDaemon {
    serve(Answer::RawChunked {
        status,
        headers,
        body,
    })
}

/// One listener, one thread, one connection at a time.
fn serve(answer: Answer) -> RecordingDaemon {
    let listener = match std::net::TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(e) => panic!("a loopback listener for a recording daemon: {e}"),
    };
    let port = match listener.local_addr() {
        Ok(addr) => addr.port(),
        Err(e) => panic!("the recording daemon's own address: {e}"),
    };
    let log = std::sync::Arc::new(std::sync::Mutex::new(Recorded::default()));
    let served = std::sync::Arc::clone(&log);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let Some((_method, target, authorization, body)) = read_request(&mut stream) else {
                continue;
            };
            let count = match served.lock() {
                Ok(mut log) => {
                    log.attempts += 1;
                    log.last = Some((target, authorization));
                    log.last_body = body;
                    log.attempts
                }
                Err(_) => continue,
            };
            let (status, headers, body, declared) = scripted(&answer, count);
            let headers = headers
                .into_iter()
                .map(|(name, value)| format!("{name}: {value}\r\n"))
                .collect::<String>();
            let content_length = declared
                .map(|length| format!("Content-Length: {length}\r\n"))
                .unwrap_or_default();
            let head = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n\
                 {headers}{content_length}Connection: close\r\n\r\n",
            );
            use std::io::Write;
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&body);
            let _ = stream.flush();
        }
    });
    RecordingDaemon {
        base: format!("http://127.0.0.1:{port}"),
        log,
    }
}

/// The answer for the `count`-th request served: the status, the bytes that go on the wire,
/// and an optional `Content-Length` declared for them.
///
/// The declared length is a separate number rather than `body.len()` because one fixture
/// exists to make the two disagree: [`Answer::Truncated`] promises a body it does not send.
fn scripted(answer: &Answer, count: usize) -> ScriptedResponse {
    let whole = |status: u16, body: Vec<u8>| {
        let declared = body.len();
        (
            status,
            vec![(
                "x-goalcup-control-version".to_string(),
                goalcup_protocol::control::CONTROL_API_VERSION.to_string(),
            )],
            body,
            Some(declared),
        )
    };
    match answer {
        Answer::Empty => whole(200, b"{}".to_vec()),
        Answer::Refusal {
            status,
            code,
            message,
        } => whole(*status, error_body(code, message).into_bytes()),
        Answer::BusyOnce if count == 1 => whole(
            409,
            error_body("busy", "this entry is held by a call already running").into_bytes(),
        ),
        Answer::BusyOnce => whole(200, b"{}".to_vec()),
        Answer::Oversized(bytes) => whole(200, format!("\"{}\"", "x".repeat(*bytes)).into_bytes()),
        // Eleven bytes of a body that says it is four kibibytes long, and then the close.
        Answer::Truncated => (
            200,
            vec![(
                "x-goalcup-control-version".to_string(),
                goalcup_protocol::control::CONTROL_API_VERSION.to_string(),
            )],
            b"{\"entries\":".to_vec(),
            Some(4096),
        ),
        // A quoted JSON string whose two middle bytes are not UTF-8 at all.
        Answer::NonUtf8 => whole(200, vec![b'"', 0xff, 0xfe, b'"']),
        Answer::Raw {
            status,
            headers,
            body,
        } => (*status, headers.clone(), body.clone(), Some(body.len())),
        Answer::RawChunked {
            status,
            headers,
            body,
        } => {
            let mut chunked = format!("{:X}\r\n", body.len()).into_bytes();
            chunked.extend_from_slice(body);
            chunked.extend_from_slice(b"\r\n0\r\n\r\n");
            let mut headers = headers.clone();
            headers.push(("Transfer-Encoding".to_string(), "chunked".to_string()));
            (*status, headers, chunked, None)
        }
    }
}

/// The contract's failure shape, built by hand so a test names the wire and not a builder.
fn error_body(code: &str, message: &str) -> String {
    let body = control::ErrorBody {
        error: control::ErrorDetail {
            code: code.to_string(),
            message: message.to_string(),
            detail: serde_json::Value::Null,
        },
    };
    match serde_json::to_string(&body) {
        Ok(text) => text,
        Err(e) => panic!("the contract's own error shape: {e}"),
    }
}

/// The request target, the `Authorization` header and the body, or `None` on a connection
/// that carried no request at all.
fn read_request(
    stream: &mut std::net::TcpStream,
) -> Option<(String, String, Option<String>, Option<String>)> {
    use std::io::Read;

    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    // Byte at a time to the end of the headers, so not one byte of the body is consumed
    // before its length is known. Kilobytes, once per request.
    while !head.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte) {
            Ok(0) | Err(_) => return None,
            Ok(_) => head.push(byte[0]),
        }
    }
    let head = String::from_utf8_lossy(&head).to_string();
    let mut lines = head.lines();
    let request_line = lines.next()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next()?.to_string();
    let mut authorization = None;
    let mut length = 0usize;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("authorization") {
            authorization = Some(value.to_string());
        } else if name.eq_ignore_ascii_case("content-length") {
            length = value.parse().unwrap_or(0);
        }
    }
    // Every byte of it, before a single byte of the response goes out.
    let mut body = vec![0u8; length];
    if length > 0 && stream.read_exact(&mut body).is_err() {
        return None;
    }
    let body = (length > 0).then(|| String::from_utf8_lossy(&body).to_string());
    Some((method, target, authorization, body))
}

// ------------------------------------------------- a mock whose handle is readable

/// Wait until `pid` is out of the process table, or fail naming it.
///
/// A zombie counts as gone — `process::is_alive` says so — because the caller that owns the
/// `Child` reaps it, and an orphaned grandchild is reaped by init a moment after it dies.
pub fn wait_until_gone(pid: u32) {
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline {
        if !goalcup_protocol::process::is_alive(pid) {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("pid {pid} was still running {PATIENCE:?} after the command returned");
}

// ------------------------------------------------ a mock with the daemon's files in place

/// Every path under `root`, with its size and its modification time, sorted.
///
/// It covers the whole root rather than the exported trace alone, so a file written BESIDE
/// the trace is seen, and an mtime moves even when the bytes do not, so a file rewritten with
/// the same bytes is seen too.
///
/// A file whose metadata cannot be read is recorded as such rather than dropped, so a
/// helper that went blind cannot read as a directory that did not change.
pub fn artifact_snapshot(root: &Path) -> Vec<(String, String)> {
    let mut paths = Vec::new();
    collect(root, &mut paths);
    let mut out: Vec<(String, String)> = paths
        .iter()
        .map(|p| {
            let name = p
                .strip_prefix(root)
                .unwrap_or(p.as_path())
                .display()
                .to_string();
            let said = match std::fs::metadata(p) {
                Ok(m) => format!(
                    "{} bytes, mtime {:?}",
                    m.len(),
                    m.modified().ok().map(|t| t
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos())
                        .unwrap_or(0))
                ),
                Err(e) => format!("unreadable: {e}"),
            };
            (name, said)
        })
        .collect();
    out.sort();
    out
}

// ------------------------------------------------------------------ waiting on a cadence

/// Wait for one of the daemon's own cadences to produce a fact, and answer it. A condition,
/// never a duration: the daemon runs its own loop, and a `sleep` long enough to be reliable
/// on a loaded machine is a test that costs that long every time it passes.
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
