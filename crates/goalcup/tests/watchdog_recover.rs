//! PRD-02 section 8.1 (ledger R3b, point 3): `watchdog::recover_with` after the watchdog and then
//! the recorder died while the harness runs. The state is built by hand: a minted CA and its key
//! file, a recorder started on a listener with the start line, `ready`, a shell harness sent as
//! `launch`, one TLS request served, then the recorder SIGKILLed and everything closed. Linux only.
#![cfg(target_os = "linux")]

mod common;

use common::pki::HTTP11;
use common::recorder::records;
use common::upstream::{Script, Upstream};
use goalcup::cli::{self, proc_start_id, read_json, status_json, watchdog_pid};
use goalcup::connector::{ConnectorRef, HostList, NativeSources, builtin};
use goalcup::proxy::ca::{RUN_CA_FILE, RunCa, certificates};
use goalcup::watchdog::{self, CaPem, HarnessFacts, NotRecovered, StartFacts, ToRecorder, spawn};
use goalcup_format::{GapReason, Record};
use rustls::pki_types::{CertificateDer, ServerName};
use serde_json::{Value, json, to_value};
use std::io::{BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::net::UnixStream;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Waits until `f` gives `Some`, at most 60 s.
fn until<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    let start = Instant::now();
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(start.elapsed() < Duration::from_secs(60), "timed out: {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

struct Dead {
    _tmp: tempfile::TempDir,
    _up: Upstream,
    entry_dir: PathBuf,
    port: u16,
    harness: Child,
    facts: StartFacts,
    /// The hand-started recorder, still running when [`setup`] was asked to keep it, and its
    /// control end and listener.
    recorder: Child,
    held: Option<(UnixStream, TcpListener)>,
}

/// The hand-built state; the recorder is SIGKILLed unless `keep`.
fn setup(keep: bool) -> Dead {
    let tmp = tempfile::tempdir().unwrap();
    let up = Upstream::start(Script::ok(b"hi").with_tls(&["api.openai.com"], &[HTTP11]));
    // SAFETY: before any thread reads the environment; nextest runs one test per process.
    unsafe {
        std::env::set_var("GOALCUP_SERVER", "off");
        std::env::set_var("GOALCUP_TEST_UPSTREAM", format!("api.openai.com={}", up.addr()));
        std::env::set_var("GOALCUP_TEST_EXTRA_ROOTS", up.pki().roots_file(tmp.path()));
    }
    let (run, entry) = ("r0123456789abcdef", "e0123456789abcdef");
    let home = tmp.path().join("g");
    let entry_dir = cli::entry_dir(&home, run, entry);
    let rec = entry_dir.join("rec");
    std::fs::create_dir_all(&rec).unwrap();
    std::fs::set_permissions(&rec, std::fs::Permissions::from_mode(0o700)).unwrap();
    let hosts = HostList::from_file(builtin::HOSTS.as_bytes(), "model-hosts.toml").unwrap();
    let ca = RunCa::mint(&hosts.ca_subtrees()).unwrap();
    ca.write_files(&entry_dir).unwrap();
    let key = rec.join(watchdog::KEY_FILE);
    std::fs::write(&key, ca.export().key).unwrap();
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();

    let marker = tmp.path().join("signalled");
    let script = format!(
        "trap ': > {}' TERM INT HUP USR1 QUIT; while :; do sleep 0.05; done",
        marker.display()
    );
    let harness = Command::new("/bin/sh").args(["-c", &script]).spawn().unwrap();
    let pid = harness.id();
    let launched = HarnessFacts {
        pid,
        program: "/bin/sh".into(),
        started: "2026-11-01T00:00:00Z".into(),
        start_id: proc_start_id(pid).unwrap(),
    };
    let mut facts = StartFacts {
        run: run.into(),
        entry: entry.into(),
        segment: 0,
        entry_dir: entry_dir.clone(),
        deadline: None,
        server_time: None,
        local_wall: 0,
        workspace: tmp.path().to_path_buf(),
        connector: to_value(ConnectorRef {
            id: "fake".into(),
            name: "Fake".into(),
            sha256: "0".repeat(64),
            source: Default::default(),
        })
        .unwrap(),
        harness_version: None,
        keep: true,
        build: None,
        hosts: to_value(&hosts).unwrap(),
        hosts_sha256: hosts.sha256().to_string(),
        native: to_value(NativeSources::default()).unwrap(),
        mask_rules: json!([]),
        values: json!([]),
        ca: ca.export(),
        prev: None,
        harness: None,
        null_since: None,
        notes: Vec::new(),
    };
    let (listener, addr) = common::ports::held();
    let (to, theirs) = UnixStream::pair().unwrap();
    let mut recorder = spawn::recorder(&common::bin(), false, &listener, &theirs).unwrap();
    drop(theirs);
    assert!(watchdog::send_line(&to, &ToRecorder::Start(Box::new(facts.clone()))));
    let mut from = BufReader::new(to.try_clone().unwrap());
    let ready = watchdog::read_line::<Value>(&mut from).flatten().unwrap();
    assert_eq!(ready["line"], "ready");
    assert!(watchdog::send_line(&to, &ToRecorder::Launch(launched.clone())));
    assert!(get(&entry_dir, addr.port()).ends_with(b"hi"));
    // A boot that dies before its first LINK leaves its number to the next.
    let counts = entry_dir.join("counts.jsonl");
    until("a link", || std::fs::metadata(&counts).ok().filter(|m| m.len() > 0));
    let status = || read_json::<Value>(&status_json(&entry_dir));
    until("the port in status.json", || status().filter(|s| s["port"] == addr.port()));
    drop(from);
    let mut held = Some((to, listener));
    if !keep {
        recorder.kill().unwrap();
        recorder.wait().unwrap();
        held = None;
    }
    (facts.ca, facts.harness) = (CaPem { cert: String::new(), key: String::new() }, Some(launched));
    Dead { _tmp: tmp, _up: up, entry_dir, port: addr.port(), harness, facts, recorder, held }
}

/// One GET through the proxy on `port` to the listed host, its leaf verified against
/// `run-ca.pem` only: the answer's bytes.
fn get(entry_dir: &Path, port: u16) -> Vec<u8> {
    let mut tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
    tcp.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    tcp.write_all(b"CONNECT api.openai.com:443 HTTP/1.1\r\nHost: api.openai.com\r\n\r\n").unwrap();
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut b = [0u8];
        assert_eq!(tcp.read(&mut b).unwrap(), 1, "the proxy closed the CONNECT");
        head.push(b[0]);
    }
    assert!(head.starts_with(b"HTTP/1.1 200"));
    let mut store = rustls::RootCertStore::empty();
    for c in certificates(&std::fs::read(entry_dir.join(RUN_CA_FILE)).unwrap()) {
        store.add(CertificateDer::from(c)).unwrap();
    }
    let mut config = rustls::ClientConfig::builder_with_provider(common::pki::provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(store)
        .with_no_client_auth();
    config.alpn_protocols = vec![HTTP11.to_vec()];
    let name = ServerName::try_from("api.openai.com").unwrap();
    let conn = rustls::ClientConnection::new(Arc::new(config), name).unwrap();
    let mut tls = rustls::StreamOwned::new(conn, tcp);
    tls.write_all(b"GET / HTTP/1.1\r\nHost: api.openai.com\r\n\r\n").unwrap();
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    while !out.ends_with(b"hi") {
        let n = tls.read(&mut buf).expect("the leaf verifies against run-ca.pem");
        assert!(n > 0, "closed early: {}", String::from_utf8_lossy(&out));
        out.extend_from_slice(&buf[..n]);
    }
    out
}

/// R3b: recover reopens the same port with the same CA under a detached watchdog, writes one
/// `recorder_down` gap with cause `watchdog_lost`, never touches the harness, and seals
/// `harness_exit` once the harness is gone, removing the key file.
#[test]
fn watchdog_recover_restarts_the_recorder_only() {
    let mut d = setup(false);
    let got = watchdog::recover_with(&d.entry_dir, d.facts.clone(), &common::bin());
    assert_eq!(got, Ok(watchdog::Recovered { port: d.port }));
    assert!(get(&d.entry_dir, d.port).ends_with(b"hi"));
    let (pid, start_id) = watchdog_pid(&d.entry_dir).expect("rec/watchdog.pid");
    assert_ne!(pid, std::process::id());
    assert!(cli::alive(pid, start_id), "the detached watchdog runs");
    let h = d.facts.harness.clone().unwrap();
    assert!(cli::alive(h.pid, h.start_id));
    assert!(d.harness.try_wait().unwrap().is_none());
    assert!(!d._tmp.path().join("signalled").exists(), "the harness was signalled");

    d.harness.kill().unwrap();
    d.harness.wait().unwrap();
    let status = until("the seal", || {
        read_json::<Value>(&status_json(&d.entry_dir)).filter(|s| !s["seal"].is_null())
    });
    assert_eq!(status["seal"]["reason"], "harness_exit", "{status}");
    assert!(!d.entry_dir.join("rec").join(watchdog::KEY_FILE).exists());
    let down: Vec<(u64, u64, Value)> = records(&d.entry_dir)
        .into_iter()
        .filter_map(|r| match r {
            Record::Gap { reason: GapReason::RecorderDown, start_mono, end_mono, tail, .. } => {
                Some((start_mono, end_mono, serde_json::from_slice(&tail).unwrap()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(down.len(), 1, "{down:?}");
    assert_eq!(down[0].2["cause"], "watchdog_lost", "{down:?}");
    assert!(down[0].0 < down[0].1);
}

/// R3b refusals: a live recorder holding the lock is left alone, a missing key and a taken port
/// refuse, and none of them starts a watchdog or binds the port.
#[test]
fn watchdog_recover_refusals_start_nothing() {
    let mut d = setup(true);
    let try_it = |d: &Dead| watchdog::recover_with(&d.entry_dir, d.facts.clone(), &common::bin());
    assert_eq!(try_it(&d), Err(NotRecovered::RecordingOn));
    assert!(d.recorder.try_wait().unwrap().is_none(), "the live recorder was touched");
    assert!(watchdog_pid(&d.entry_dir).is_none());
    d.recorder.kill().unwrap();
    d.recorder.wait().unwrap();
    d.held = None;
    until("the port is free", || spawn::bind_exact(d.port).ok());

    let key = d.entry_dir.join("rec").join(watchdog::KEY_FILE);
    let aside = key.with_extension("aside");
    std::fs::rename(&key, &aside).unwrap();
    assert_eq!(try_it(&d), Err(NotRecovered::KeyMissing));
    std::fs::rename(&aside, &key).unwrap();

    let taken = spawn::bind_exact(d.port).unwrap();
    assert_eq!(try_it(&d), Err(NotRecovered::PortTaken));
    drop(taken);
    assert!(watchdog_pid(&d.entry_dir).is_none());
    assert!(spawn::bind_exact(d.port).is_ok(), "a refusal left the port bound");
    d.harness.kill().unwrap();
    let _ = d.harness.wait();
}
