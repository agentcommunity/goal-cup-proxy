//! The real installed Hermes, offline, with a synthetic key.
//!
//! These tests run the installed `hermes-agent` 0.19.0 against a **test-owned fake CONNECT
//! proxy** and nothing else. They are `#[ignore]`d and run by name:
//!
//! ```text
//! GOALCUP_TEST_HERMES_EXE=/path/to/hermes \
//!   cargo test --offline -p goalcup-cli --test it hermes_offline_key:: -- --ignored --test-threads=1
//! ```
//!
//! `GOALCUP_TEST_HERMES_EXE` must name the installed `hermes-agent` executable. There is no
//! default path; with the variable unset every test here fails and says so.
//!
//! What makes them safe to run:
//!
//! * The key is `sk-` and 32 random hex digits, generated per run and held in memory. No test
//!   here reads any credential file or any real key, and none sets a variable from the
//!   environment it runs in.
//! * The child's environment is the production capture plan's (built from nothing): a private
//!   `HOME` and `HERMES_HOME` inside a temporary run directory, `HTTPS_PROXY` pointing at the
//!   fake proxy's loopback port, `HERMES_CA_BUNDLE` pointing at the test's own CA, and the
//!   synthetic key handed through the production `ChildSecret` on the production gate. Before
//!   the child starts, the test asserts all of that.
//! * The fake proxy dials nothing. It answers `CONNECT api.deepseek.com:443` itself, terminates
//!   TLS with a leaf the test CA signed, and refuses every other target with 403, so no byte
//!   leaves the machine.
//! * The proxy compares the `Authorization` header with the synthetic key in memory and keeps
//!   only the boolean.
//! * Nothing is created under the real `HOME`; the test checks that `~/.hermes` is exactly as
//!   it was.
//!
//! What they prove (D41, readiness record "Native snapshot activation and collection"):
//!
//! * the corrected `auth.json` seed keeps the credential pool from copying the environment key,
//!   so the file is byte-identical after a run in which the key reached the request;
//! * a 4xx makes Hermes write `request_dump_*.json` holding a key fragment, and the key scan
//!   finds it (the positive control for the scan);
//! * D41's literal two-key seed does *not* hold: the pool seeds a fingerprinted entry (the
//!   deliberate RED case that documents the correction).

use crate::common::m2_common;

use goalcup_cli::key_scan::{FragmentMatcher, ScanOutcome, scan_tree};
use goalcup_cli::launch::{
    CAPTURE_PATH, CaptureLaunch, Pipes, ScopeMarker, build_capture_launch,
    child_secret_for_harness, resolve_harness_executable, spawn_gated,
    tee_marked_capture_with_rule,
};
use goalcup_protocol::template::TemplateVars;
use m2_common::{TestPki, provider, read_head};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const RUN_BOUND: Duration = Duration::from_secs(120);

/// The Hermes executable under test, taken only from `GOALCUP_TEST_HERMES_EXE`. There is no
/// default path: an unset variable fails the test with the way to set it.
fn hermes_exe() -> PathBuf {
    let Some(path) = std::env::var_os("GOALCUP_TEST_HERMES_EXE").filter(|value| !value.is_empty())
    else {
        panic!(
            "GOALCUP_TEST_HERMES_EXE is not set. Point it at the installed hermes-agent \
             executable, for example: GOALCUP_TEST_HERMES_EXE=/path/to/hermes cargo test \
             --offline -p goalcup-cli --test it hermes_offline_key:: -- --ignored --test-threads=1"
        );
    };
    PathBuf::from(path)
}

fn connector_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../connectors")
}

/// `sk-` and 32 random hex digits, from this process's generator and nothing else.
fn synthetic_key() -> String {
    format!("sk-{:032x}", rand::random::<u128>())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Reply {
    Ok200,
    Unauthorized401,
}

/// What the fake proxy saw, as counts and target names. Never a header value.
#[derive(Default)]
struct Seen {
    connects: Mutex<Vec<String>>,
    /// The `model` value of each request body: what the run asked for (D41), and nothing else
    /// of the body.
    models: Mutex<Vec<String>>,
    requests: AtomicUsize,
    key_matched: AtomicUsize,
    key_mismatched: AtomicUsize,
}

struct FakeProxy {
    addr: std::net::SocketAddr,
    seen: Arc<Seen>,
    // Dropped last: it owns the runtime the listener runs on.
    _runtime: tokio::runtime::Runtime,
}

impl FakeProxy {
    fn start(pki: &TestPki, key: &str, reply: Reply) -> FakeProxy {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let (chain, leaf_key) = pki.leaf(&["api.deepseek.com"]);
        let mut tls = rustls::ServerConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(chain, leaf_key)
            .unwrap();
        tls.alpn_protocols = vec![b"http/1.1".to_vec()];
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(tls));
        let listener = runtime
            .block_on(TcpListener::bind("127.0.0.1:0"))
            .expect("a loopback listener");
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Seen::default());
        let expected = Arc::new(format!("Bearer {key}").into_bytes());
        let masked = Arc::new(format!("{}***{}", &key[..8], &key[key.len() - 4..]));
        let task_seen = Arc::clone(&seen);
        runtime.spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                let (acceptor, seen, expected, masked) = (
                    acceptor.clone(),
                    Arc::clone(&task_seen),
                    Arc::clone(&expected),
                    Arc::clone(&masked),
                );
                tokio::spawn(async move {
                    let _ = serve(tcp, acceptor, seen, expected, masked, reply).await;
                });
            }
        });
        FakeProxy {
            addr,
            seen,
            _runtime: runtime,
        }
    }

    fn connects(&self) -> Vec<String> {
        self.seen.connects.lock().unwrap().clone()
    }
}

async fn serve(
    mut tcp: TcpStream,
    acceptor: tokio_rustls::TlsAcceptor,
    seen: Arc<Seen>,
    expected: Arc<Vec<u8>>,
    masked: Arc<String>,
    reply: Reply,
) -> std::io::Result<()> {
    let (head, _) = read_head(&mut tcp).await;
    let first = head
        .split(|b| *b == b'\r')
        .next()
        .map(|l| String::from_utf8_lossy(l).into_owned())
        .unwrap_or_default();
    let target = first
        .strip_prefix("CONNECT ")
        .and_then(|rest| rest.split(' ').next())
        .unwrap_or("")
        .to_string();
    seen.connects.lock().unwrap().push(target.clone());
    if target != "api.deepseek.com:443" {
        tcp.write_all(b"HTTP/1.1 403 Forbidden\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
            .await?;
        return tcp.shutdown().await;
    }
    tcp.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await?;
    let mut tls = acceptor.accept(tcp).await?;
    let (head, mut body) = read_head(&mut tls).await;
    seen.requests.fetch_add(1, Ordering::SeqCst);
    // Compared in memory, as a boolean. The header's text is never kept.
    let presented = header(&head, "authorization");
    if presented.as_deref() == Some(expected.as_slice()) {
        seen.key_matched.fetch_add(1, Ordering::SeqCst);
    } else {
        seen.key_mismatched.fetch_add(1, Ordering::SeqCst);
    }
    // The body, so the client is not left writing into a closed socket.
    if let Some(length) = header(&head, "content-length")
        .and_then(|v| String::from_utf8_lossy(&v).trim().parse::<usize>().ok())
    {
        let mut chunk = vec![0u8; 16 * 1024];
        while body.len() < length {
            let n = tls.read(&mut chunk).await?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..n]);
        }
    } else if header(&head, "transfer-encoding").is_some() {
        let mut chunk = vec![0u8; 16 * 1024];
        while !body.ends_with(b"0\r\n\r\n") {
            let n = tls.read(&mut chunk).await?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..n]);
        }
    }
    let text = String::from_utf8_lossy(&body);
    if let Some(model) = request_model(&text) {
        seen.models.lock().unwrap().push(model);
    }
    let streaming = text.contains("\"stream\":true") || text.contains("\"stream\": true");
    let response = match reply {
        Reply::Unauthorized401 => {
            let body = format!(
                r#"{{"error":{{"message":"Incorrect API key provided: {masked}. You can find your API key at https://platform.deepseek.com/api_keys.","type":"authentication_error","param":null,"code":"invalid_request_error"}}}}"#
            );
            format!(
                "HTTP/1.1 401 Unauthorized\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            )
        }
        Reply::Ok200 if streaming => {
            let first = r#"{"id":"chatcmpl-test","object":"chat.completion.chunk","created":1,"model":"deepseek-v4-flash","choices":[{"index":0,"delta":{"role":"assistant","content":"hello from the fake proxy"},"finish_reason":null}]}"#;
            let last = r#"{"id":"chatcmpl-test","object":"chat.completion.chunk","created":1,"model":"deepseek-v4-flash","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
            format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\n\r\ndata: {first}\n\ndata: {last}\n\ndata: [DONE]\n\n"
            )
        }
        Reply::Ok200 => {
            let body = r#"{"id":"chatcmpl-test","object":"chat.completion","created":1,"model":"deepseek-v4-flash","choices":[{"index":0,"message":{"role":"assistant","content":"hello from the fake proxy"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
            format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            )
        }
    };
    tls.write_all(response.as_bytes()).await?;
    let _ = tls.shutdown().await;
    Ok(())
}

/// The first `"model": "<value>"` in a JSON request body, with nothing else of it.
fn request_model(body: &str) -> Option<String> {
    let at = body.find("\"model\"")?;
    let rest = body[at + "\"model\"".len()..]
        .trim_start()
        .strip_prefix(':')?;
    let rest = rest.trim_start().strip_prefix('"')?;
    Some(rest[..rest.find('"')?].to_string())
}

/// A header's value, matched case-insensitively.
fn header(head: &[u8], name: &str) -> Option<Vec<u8>> {
    head.split(|b| *b == b'\n').skip(1).find_map(|line| {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let colon = line.iter().position(|b| *b == b':')?;
        line[..colon]
            .eq_ignore_ascii_case(name.as_bytes())
            .then(|| line[colon + 1..].trim_ascii().to_vec())
    })
}

/// What the real `~/.hermes` looks like: present or absent, and its entry names and mtimes.
/// Names and times only, never contents.
fn real_hermes_listing() -> String {
    let Some(home) = std::env::var_os("HOME") else {
        return "no HOME".to_string();
    };
    let dir = Path::new(&home).join(".hermes");
    let Ok(meta) = std::fs::symlink_metadata(&dir) else {
        return "absent".to_string();
    };
    let mut lines = vec![format!("present {:?}", meta.modified().ok())];
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            lines.push(format!(
                "{} {:?}",
                e.file_name().to_string_lossy(),
                e.metadata().and_then(|m| m.modified()).ok()
            ));
        }
    }
    lines.sort();
    lines.join("\n")
}

struct Run {
    status: std::process::ExitStatus,
    marker: ScopeMarker,
    key_hit: bool,
    dir: PathBuf,
    proxy: FakeProxy,
    key: String,
    _tmp: tempfile::TempDir,
}

/// One real Hermes run: the production plan, gate, secret and tee, against the fake proxy.
/// `auth_seed` replaces the shipped seed when given.
fn run_hermes(reply: Reply, auth_seed: Option<&[u8]>) -> Run {
    let exe = hermes_exe();
    assert!(
        exe.is_file(),
        "the installed Hermes is not at {}",
        exe.display()
    );
    let before = real_hermes_listing();

    let tmp = tempfile::tempdir().unwrap();
    let dir = std::fs::canonicalize(tmp.path())
        .unwrap()
        .join("goalcup-run");
    for sub in ["", "home", "ca", "workspace", "tmp"] {
        std::fs::create_dir_all(dir.join(sub)).unwrap();
    }
    let connector =
        goalcup_cli::connector::load_harness(&connector_dir(), "hermes-cli", false).unwrap();
    let profile = connector.loaded.manifest.capture.clone();
    let loaded = goalcup_protocol::harness::load_dir(
        &connector.dir,
        goalcup_protocol::harness::Origin::Pinned,
    )
    .unwrap();
    let hermes_home = goalcup_cli::per_run_home::create_per_run_home(
        &dir,
        &loaded.manifest.capture.home,
        &loaded.seeds,
    )
    .expect("the per-run home");
    if let Some(seed) = auth_seed {
        std::fs::write(hermes_home.join("auth.json"), seed).unwrap();
    }

    let key = synthetic_key();
    let pki = TestPki::new("goal cup test CA");
    let ca = dir.join("ca/proxy-ca.pem");
    std::fs::write(&ca, pki.root_pem()).unwrap();
    let proxy = FakeProxy::start(&pki, &key, reply);
    let proxy_url = format!(
        "http://goalcup:{:032x}@127.0.0.1:{}",
        rand::random::<u128>(),
        proxy.addr.port()
    );

    let resolved = resolve_harness_executable(&exe).expect("the installed Hermes");
    let parent: BTreeMap<String, String> = [
        ("USER", "participant"),
        ("LOGNAME", "participant"),
        ("TMPDIR", &*format!("{}/", dir.join("tmp").display())),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    let mut vars = TemplateVars::new();
    vars.set("workspace", dir.join("workspace").to_str().unwrap());
    vars.set("session_id", "00000000-0000-4000-8000-000000000000");
    let plan = build_capture_launch(
        &connector,
        &vars,
        &dir.join("workspace"),
        &CaptureLaunch {
            goal: Some(b"hi"),
            executable: &resolved,
            run_home: &dir.join("home"),
            config_home: &hermes_home,
            proxy_url: &proxy_url,
            ca_certificate: &ca,
            ca_bundle: None,
            parent_env: &parent,
        },
    )
    .expect("the capture plan");

    // Before anything starts: the child's environment is exactly the plan's, the proxy it is
    // pointed at is the test's own loopback listener, and the real homes are not in it.
    let proxy_port = format!("127.0.0.1:{}", proxy.addr.port());
    for name in ["HTTPS_PROXY", "https_proxy"] {
        assert!(
            plan.env_set[name].ends_with(&format!("@{proxy_port}")),
            "{name} does not point at the test proxy"
        );
    }
    assert!(plan.env_base.is_empty(), "the child inherits nothing");
    assert_eq!(plan.env_set["HERMES_CA_BUNDLE"], ca.to_str().unwrap());
    assert_eq!(plan.env_set["PATH"], CAPTURE_PATH);
    for (name, under) in [("HOME", "home"), ("HERMES_HOME", "hermes")] {
        assert_eq!(
            Path::new(&plan.env_set[name]),
            dir.join(under),
            "{name} is the run's private directory"
        );
    }
    assert!(plan.env_set.keys().all(|k| !k.contains("API_KEY")));
    assert!(plan.env_set.keys().all(|k| !k.ends_with("_BASE_URL")));

    let secret = child_secret_for_harness(
        &profile,
        &BTreeMap::from([("DEEPSEEK_API_KEY".to_string(), key.clone())]),
    )
    .unwrap()
    .unwrap();
    let matcher = FragmentMatcher::new(key.as_bytes()).unwrap();

    let gated = spawn_gated(
        &plan,
        Pipes {
            stdout: true,
            stderr: true,
        },
        Duration::from_secs(10),
        Some(&secret),
    )
    .expect("the gate");
    let mut spawned = gated.release().expect("released");
    let tee = tee_marked_capture_with_rule(
        &mut spawned,
        Some(&secret),
        Some(crate::common::support::stderr_marker_rule()),
    )
    .unwrap();
    drop(secret);
    let until = Instant::now() + RUN_BOUND;
    let status = loop {
        if let Some(status) = spawned.child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= until {
            let _ = spawned.child.kill();
            let _ = spawned.child.wait();
            panic!("Hermes did not exit within {RUN_BOUND:?}");
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(
        tee.finish(Instant::now() + Duration::from_secs(10))
            .unwrap()
    );
    let marker = tee.marker_until(Instant::now() + Duration::from_secs(2));
    let key_hit = tee.key_hit();

    assert_eq!(
        real_hermes_listing(),
        before,
        "the real ~/.hermes changed while Hermes ran"
    );
    drop(matcher);
    Run {
        status,
        marker,
        key_hit,
        dir,
        proxy,
        key,
        _tmp: tmp,
    }
}

fn scan(run: &Run) -> ScanOutcome {
    let matcher = FragmentMatcher::new(run.key.as_bytes()).unwrap();
    scan_tree(&matcher, &[&run.dir], 256 << 20)
}

fn sessions(run: &Run) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(run.dir.join("hermes/sessions"))
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

fn shipped_seed() -> Vec<u8> {
    std::fs::read(connector_dir().join("hermes-cli/files/auth.json")).unwrap()
}

/// The 200 variant: the corrected seed leaves `auth.json` byte-identical although the key
/// reached the request; the run is clean under the scan; one session snapshot carries the
/// marker's id; nothing reached anything but the fake proxy.
#[test]
#[ignore = "runs the real installed Hermes against a fake CONNECT proxy; run by name"]
fn the_corrected_seed_keeps_the_key_out_of_the_store_while_it_reaches_the_request() {
    let run = run_hermes(Reply::Ok200, None);
    let connects = run.proxy.connects();
    eprintln!(
        "offline 200: exit={:?} connects={connects:?} requests={} matched={} mismatched={} \
         sessions={:?} marker={:?} models={:?}",
        run.status.code(),
        run.proxy.seen.requests.load(Ordering::SeqCst),
        run.proxy.seen.key_matched.load(Ordering::SeqCst),
        run.proxy.seen.key_mismatched.load(Ordering::SeqCst),
        sessions(&run),
        run.marker,
        run.proxy.seen.models.lock().unwrap(),
    );
    assert!(
        run.proxy.seen.key_matched.load(Ordering::SeqCst) >= 1,
        "the synthetic key never reached an outgoing request"
    );
    assert_eq!(
        run.proxy.seen.key_mismatched.load(Ordering::SeqCst),
        0,
        "a request carried some other credential"
    );
    assert!(
        connects.iter().any(|c| c == "api.deepseek.com:443"),
        "{connects:?}"
    );
    // The provider, and at most the model catalogue, which has no switch and
    // is accepted and recorded. Not PyPI (the update check), and above all not GitHub (a scanner
    // binary downloaded at run time and run with the key in its environment).
    for host in &connects {
        assert!(
            host == "api.deepseek.com:443" || host == "models.dev:443",
            "an unexpected host was contacted: {host} in {connects:?}"
        );
    }
    // No scanner exists anywhere in the run: nothing was fetched, nothing was run.
    let mut stack = vec![run.dir.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            assert!(!name.contains("tirith"), "found {}", entry.path().display());
            if entry.file_type().unwrap().is_dir() {
                stack.push(entry.path());
            }
        }
    }
    assert!(
        !run.dir.join("hermes/bin").exists(),
        "a binary directory was made"
    );
    assert!(run.status.success(), "{:?}", run.status);
    // Byte-identical, and no pool.
    let auth = std::fs::read(run.dir.join("hermes/auth.json")).unwrap();
    assert_eq!(auth, shipped_seed(), "auth.json changed");
    assert!(!String::from_utf8_lossy(&auth).contains("credential_pool"));
    // The run's own session, named by the marker.
    let ScopeMarker::One(id) = &run.marker else {
        panic!("no session marker: {:?}", run.marker);
    };
    assert!(
        sessions(&run).contains(&format!("session_{id}.json")),
        "{:?}",
        sessions(&run)
    );
    assert!(!run.key_hit, "a key fragment passed on stdout or stderr");
    let outcome = scan(&run);
    assert!(
        outcome.is_clean(),
        "the scan of the run directory: {outcome}"
    );
}

/// The 401 variant: a 4xx makes Hermes write a request dump with a key fragment, and the
/// fragment scan finds it. The positive control for the scan, on the residue Hermes really
/// leaves.
#[test]
#[ignore = "runs the real installed Hermes against a fake CONNECT proxy; run by name"]
fn a_401_leaves_a_request_dump_and_the_fragment_scan_finds_it() {
    let run = run_hermes(Reply::Unauthorized401, None);
    eprintln!(
        "offline 401: exit={:?} connects={:?} matched={} sessions={:?}",
        run.status.code(),
        run.proxy.connects(),
        run.proxy.seen.key_matched.load(Ordering::SeqCst),
        sessions(&run),
    );
    assert!(run.proxy.seen.key_matched.load(Ordering::SeqCst) >= 1);
    assert!(
        sessions(&run)
            .iter()
            .any(|n| n.starts_with("request_dump_")),
        "no request dump: {:?}",
        sessions(&run)
    );
    match scan(&run) {
        ScanOutcome::Hit { relative_paths } => {
            assert!(
                relative_paths
                    .iter()
                    .any(|p| p.to_string_lossy().contains("request_dump_")),
                "the hit is not in the request dump: {relative_paths:?}"
            );
        }
        other => panic!("the scan did not find the request dump's fragment: {other}"),
    }
}

/// The deliberate RED case: D41's literal seed
/// (`{"version":1,"providers":{}}`) does not suppress the environment source, so the pool copies
/// the key into a fingerprinted entry and the byte-identical oracle fails. This test passes by
/// asserting the failure's shape, which is how the correction stays enforced.
#[test]
#[ignore = "runs the real installed Hermes against a fake CONNECT proxy; run by name"]
fn d41s_literal_seed_fails_the_byte_identical_oracle() {
    let literal = br#"{"version":1,"providers":{}}"#;
    let run = run_hermes(Reply::Ok200, Some(literal));
    let auth = std::fs::read(run.dir.join("hermes/auth.json")).unwrap();
    assert_ne!(auth, literal.to_vec(), "the literal seed was left alone");
    let text = String::from_utf8_lossy(&auth).into_owned();
    assert!(text.contains("credential_pool"), "{text}");
    assert!(text.contains("deepseek"), "{text}");
    assert!(text.contains("secret_fingerprint"), "{text}");
    // The fingerprint is the first 16 hex digits of the key's SHA-256.
    let digest = Sha256::digest(run.key.as_bytes());
    let fingerprint: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
    assert!(
        text.contains(&fingerprint),
        "the stored fingerprint is not the key's"
    );
    // And the scan sees it in auth.json.
    match scan(&run) {
        ScanOutcome::Hit { relative_paths } => assert!(
            relative_paths
                .iter()
                .any(|p| p.to_string_lossy().ends_with("auth.json")),
            "{relative_paths:?}"
        ),
        other => panic!("the scan missed the fingerprint: {other}"),
    }
}
