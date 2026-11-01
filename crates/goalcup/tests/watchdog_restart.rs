//! PRD-02 section 8 steps 2 and 8, section 9.1 (point 3; PRD-17 L-C12, L-P1, L-P2):
//! `watchdog::run_with` with a `sh` harness waiting on a stop file and a recorder wrapper that
//! holds boot 2 for 3 s before it execs the goalcup binary. The one listener the watchdog holds
//! is on 127.0.0.1 only; a connection made while no recorder runs waits in its backlog and is
//! served by the next boot on the same port; the next boot's leaf verifies against the
//! `run-ca.pem` written before the kill, and the decrypted bytes are in the record. Linux only.
#![cfg(target_os = "linux")]

mod common;

use common::pki::{HTTP11, provider};
use common::recorder::records;
use common::upstream::{Script, Upstream};
use goalcup::cli;
use goalcup::connector::{ConnectorRef, HomeLock, HostList, LaunchPlan, NativeSources, builtin};
use goalcup::proxy::ca::{RUN_CA_FILE, certificates};
use goalcup::watchdog::{self, StartFacts};
use goalcup_format::{GapReason, Record};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

const RUN: &str = "r0123456789abcdef";
const ENTRY: &str = "e0123456789abcdef";
const WAIT: Duration = Duration::from_secs(90);

/// Waits until `f` gives `Some`, at most [`WAIT`].
fn until<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    let start = Instant::now();
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// L-C12: the one socket listening on `port` is bound to 127.0.0.1 (none on IPv6).
fn loopback_only(port: u16) {
    let tail = format!(":{port:04X}");
    let mut listening = Vec::new();
    for file in ["/proc/net/tcp", "/proc/net/tcp6"] {
        for l in std::fs::read_to_string(file)
            .unwrap_or_default()
            .lines()
            .skip(1)
        {
            let f: Vec<&str> = l.split_whitespace().collect();
            if f.len() > 3 && f[3] == "0A" && f[1].ends_with(&tail) {
                listening.push(f[1].to_string());
            }
        }
    }
    assert_eq!(listening, [format!("0100007F{tail}")], "port {port}");
}

#[test]
fn watchdog_restart_keeps_port_backlog_and_ca() {
    let up =
        Upstream::start(Script::ok(b"restart-p2-answer").with_tls(&["api.openai.com"], &[HTTP11]));
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path().join("m");
    std::fs::create_dir_all(&d).unwrap();
    // SAFETY: nextest runs each test in its own process; the upstream's thread reads no env.
    unsafe {
        std::env::set_var("GOALCUP_SERVER", "off");
        std::env::set_var(
            "GOALCUP_TEST_UPSTREAM",
            format!("api.openai.com={}", up.addr()),
        );
        std::env::set_var("GOALCUP_TEST_EXTRA_ROOTS", up.pki().roots_file(&d));
    }
    let home = tmp.path().join("g");
    let entry_dir = cli::entry_dir(&home, RUN, ENTRY);
    std::fs::create_dir_all(&entry_dir).unwrap();
    let server = json!({"v": 1, "server": null, "run": RUN, "mode": "practice", "deadline": null,
        "repo": null, "connector": {"id": "fake", "name": "Fake", "source": "builtin"},
        "hosts_file": null, "entries": [{"id": ENTRY, "harness_version": null, "resumed": false}]});
    let run_dir = cli::run_dir(&home, RUN);
    std::fs::write(cli::server_json(&run_dir), server.to_string()).unwrap();

    // Boot 2 notes its start, waits 3 s, notes its exec: the restart window is known.
    let (dd, exe) = (d.display(), env!("CARGO_BIN_EXE_goalcup"));
    let wrapper = d.join("recorder.sh");
    let text = format!(
        "#!/bin/sh\nn=$(( $(cat {dd}/boots 2>/dev/null || echo 0) + 1 )); echo $n > {dd}/boots\n\
         if [ $n = 2 ]; then : > {dd}/second; sleep 3; : > {dd}/second_exec; fi\nexec \"{exe}\" \"$@\"\n"
    );
    std::fs::write(&wrapper, text).unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();

    let hosts = HostList::from_file(builtin::HOSTS.as_bytes(), "model-hosts.toml").unwrap();
    let connector = ConnectorRef {
        id: "fake".into(),
        name: "Fake".into(),
        sha256: "0".repeat(64),
        source: Default::default(),
    };
    let harness = format!("while [ ! -e {dd}/stop ]; do sleep 0.05; done");
    let facts: StartFacts = serde_json::from_value(json!({"run": RUN, "entry": ENTRY,
        "segment": 0, "entry_dir": entry_dir, "local_wall": 0, "workspace": d, "keep": true,
        "connector": connector, "hosts": hosts, "hosts_sha256": hosts.sha256(),
        "native": NativeSources::default(), "mask_rules": [], "values": [],
        "ca": {"cert": "", "key": ""}, "notes": []}))
    .unwrap();
    let plan = LaunchPlan {
        connector,
        harness_version: None,
        program: PathBuf::from("/bin/sh"),
        argv: vec!["-c".into(), harness],
        env: BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
        home: d.join("home"),
        lock: HomeLock::default(),
        before: Vec::new(),
        after: Vec::new(),
        ca_env: Vec::new(),
        hosts,
        native: NativeSources::default(),
        mask_rules: Vec::new(),
        secret_values: Vec::new(),
        notes: Vec::new(),
        unread: Vec::new(),
        log: Vec::new(),
        refusal: None,
    };

    let (dir, m) = (entry_dir.clone(), d.clone());
    let driver = std::thread::spawn(move || drive(&dir, &m));
    let _ = watchdog::run_with(plan, facts, &wrapper);
    driver.join().unwrap();
    drop(up);

    let st = until("the sealed status.json", || {
        cli::read_json::<Value>(&cli::status_json(&entry_dir)).filter(|s| !s["seal"].is_null())
    });
    assert_eq!(st["seal"]["reason"], "harness_exit", "{st}");
    let (mut down, mut data) = (0, Vec::new());
    for r in records(&entry_dir) {
        match r {
            Record::Gap {
                reason: GapReason::RecorderDown,
                ..
            } => down += 1,
            Record::Data { bytes, .. } => data.extend(bytes),
            _ => {}
        }
    }
    assert_eq!(down, 1);
    // L-P2: the request and the answer through the next boot are in the record, decrypted.
    for want in [&b"restart-p2-request"[..], b"restart-p2-answer"] {
        assert!(
            data.windows(want.len()).any(|w| w == want),
            "{:?} not recorded",
            String::from_utf8_lossy(want)
        );
    }
}

/// Kills boot 1 mid-run, connects in the restart window, and talks HTTPS through the next boot.
fn drive(dir: &Path, m: &Path) {
    let status = || cli::read_json::<Value>(&cli::status_json(dir));
    let (first, port) = until("boot 0", || {
        let s = status()?;
        Some((
            s["pid"].as_u64().filter(|p| *p > 0)?,
            s["port"].as_u64()? as u16,
        ))
    });
    loopback_only(port);
    let pem = std::fs::read(dir.join(RUN_CA_FILE)).unwrap();
    // A boot that dies before its first LINK leaves its number to the next, so wait for one.
    until("a link", || {
        std::fs::metadata(dir.join("counts.jsonl"))
            .ok()
            .filter(|f| f.len() > 0)
    });
    // SAFETY: a signal to one pid, the recorder's.
    unsafe { libc::kill(first as libc::pid_t, libc::SIGKILL) };
    // The watchdog starts boot 2 once it has seen boot 1 end.
    until("boot 2 started", || m.join("second").exists().then_some(()));

    // L-P1: no recorder runs now; the kernel takes the connection into the held backlog.
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    s.write_all(b"CONNECT api.openai.com:443 HTTP/1.1\r\nHost: api.openai.com:443\r\n\r\n")
        .unwrap();
    assert!(
        !m.join("second_exec").exists(),
        "boot 2 ran before the connection was queued"
    );
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut b = [0u8];
        assert_eq!(s.read(&mut b).unwrap(), 1, "the proxy closed the CONNECT");
        head.push(b[0]);
    }
    assert!(head.starts_with(b"HTTP/1.1 200"), "{head:?}");

    // L-P2: the next boot's leaf verifies against the `run-ca.pem` read before the kill.
    let mut roots = rustls::RootCertStore::empty();
    for c in certificates(&pem) {
        roots.add(c.into()).unwrap();
    }
    let mut config = rustls::ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = vec![HTTP11.to_vec()];
    let name = rustls::pki_types::ServerName::try_from("api.openai.com").unwrap();
    let conn = rustls::ClientConnection::new(Arc::new(config), name).unwrap();
    let mut t = rustls::StreamOwned::new(conn, s);
    let req = "POST /v1/responses HTTP/1.1\r\nHost: api.openai.com\r\nConnection: close\r\nContent-Length: 18\r\n\r\nrestart-p2-request";
    t.write_all(req.as_bytes()).expect("the leaf verifies");
    let mut answer = Vec::new();
    let mut buf = [0u8; 4096];
    while !answer.ends_with(b"restart-p2-answer") {
        match t.read(&mut buf) {
            Ok(n) if n > 0 => answer.extend_from_slice(&buf[..n]),
            _ => break,
        }
    }
    let answer = String::from_utf8_lossy(&answer);
    assert!(
        answer.starts_with("HTTP/1.1 200") && answer.ends_with("restart-p2-answer"),
        "{answer}"
    );

    let after = until("boot 1's status", || {
        let s = status()?;
        (s["boot"].as_u64()? >= 1 && s["pid"].as_u64()? != first).then_some(s)
    });
    assert_eq!(after["port"].as_u64(), Some(u64::from(port)), "{after}");
    loopback_only(port);
    std::fs::write(m.join("stop"), "").unwrap();
}
