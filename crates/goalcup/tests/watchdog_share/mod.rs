//! Helpers the `watchdog_*` test binaries share (PRD-02). Each includes it as
//! `mod watchdog_share;` next to `mod common;`, so unused items are allowed here.

#![allow(dead_code)]

use crate::common::pki::{HTTP11, provider};
use crate::common::recorder::records;
use crate::common::upstream::Upstream;
use goalcup::cli;
use goalcup::connector::{ConnectorRef, HomeLock, HostList, LaunchPlan, NativeSources, builtin};
use goalcup::proxy::ca::{RUN_CA_FILE, certificates};
use goalcup::watchdog::StartFacts;
use goalcup_format::{GapReason, Record};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The longest wait of a `watchdog_*` test: a hang bound only.
pub const WAIT: Duration = Duration::from_secs(90);

/// Waits until `f` gives `Some`, at most [`WAIT`].
pub fn until<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    let start = Instant::now();
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The run and entry ids of a `watchdog_*` test entry.
pub const RUN: &str = "r0123456789abcdef";
pub const ENTRY: &str = "e0123456789abcdef";

/// Start facts for the test entry in `entry_dir`: a fake built-in connector, `hosts`, no mask
/// rules or values, the CA `ca` (a `CaPem` as JSON) and every optional fact left out.
pub fn facts(entry_dir: &Path, workspace: &Path, hosts: &HostList, ca: Value) -> StartFacts {
    serde_json::from_value(json!({"run": RUN, "entry": ENTRY, "segment": 0,
        "entry_dir": entry_dir, "local_wall": 0, "workspace": workspace, "keep": true,
        "connector": {"id": "fake", "name": "Fake", "sha256": "0".repeat(64), "source": "builtin"},
        "hosts": hosts, "hosts_sha256": hosts.sha256(), "native": NativeSources::default(),
        "mask_rules": [], "values": [], "ca": ca, "notes": []}))
    .unwrap()
}

/// The pid is gone: no process, or a zombie (Linux `/proc/<pid>/stat`, elsewhere `ps`).
pub fn gone(pid: u32) -> bool {
    let state = if cfg!(target_os = "linux") {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .map(|s| s[s.rfind(')').unwrap() + 1..].trim_start().to_string())
            .unwrap_or_default()
    } else {
        let p = pid.to_string();
        let out = Command::new("ps").args(["-o", "stat=", "-p", &p]).output();
        String::from_utf8_lossy(&out.unwrap().stdout)
            .trim()
            .to_string()
    };
    state.is_empty() || state.starts_with('Z')
}

/// L-C12: the one socket listening on `port` is bound to 127.0.0.1 (none on IPv6). Linux reads
/// `/proc/net/tcp{,6}`; elsewhere `lsof`, which names the socket once per process holding it.
pub fn loopback_only(port: u16) {
    let mut listening = Vec::new();
    if !cfg!(target_os = "linux") {
        let i = format!("-iTCP:{port}");
        let out = Command::new("lsof")
            .args(["-nP", &i, "-sTCP:LISTEN", "-Fn"])
            .output();
        let out = String::from_utf8_lossy(&out.unwrap().stdout).into_owned();
        listening.extend(out.lines().filter(|l| l.starts_with('n')).map(String::from));
        listening.sort();
        listening.dedup();
        assert_eq!(listening, [format!("n127.0.0.1:{port}")], "port {port}");
        return;
    }
    let tail = format!(":{port:04X}");
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

/// The CONNECT a test sends to the proxy for the listed host.
pub const CONNECT: &[u8] =
    b"CONNECT api.openai.com:443 HTTP/1.1\r\nHost: api.openai.com:443\r\n\r\n";

/// After [`CONNECT`] was written on `tcp`: its `200` head, then TLS whose leaf must verify
/// against `pem` (a `run-ca.pem`) only.
pub fn tls_over(
    mut tcp: TcpStream,
    pem: &[u8],
) -> rustls::StreamOwned<rustls::ClientConnection, TcpStream> {
    tcp.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut b = [0u8];
        assert_eq!(tcp.read(&mut b).unwrap(), 1, "the proxy closed the CONNECT");
        head.push(b[0]);
    }
    assert!(head.starts_with(b"HTTP/1.1 200"), "{head:?}");
    let mut roots = rustls::RootCertStore::empty();
    for c in certificates(pem) {
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
    rustls::StreamOwned::new(conn, tcp)
}

/// [`CONNECT`] and [`tls_over`] on a new connection to `port`, verified against the entry's `run-ca.pem`.
pub fn connect_tls(
    entry_dir: &Path,
    port: u16,
) -> rustls::StreamOwned<rustls::ClientConnection, TcpStream> {
    let pem = std::fs::read(entry_dir.join(RUN_CA_FILE)).unwrap();
    let mut tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
    tcp.write_all(CONNECT).unwrap();
    tls_over(tcp, &pem)
}

/// Writes `request` and reads until the answer ends with `tail` or the stream ends.
pub fn exchange(
    tls: &mut rustls::StreamOwned<rustls::ClientConnection, TcpStream>,
    request: &[u8],
    tail: &[u8],
) -> Vec<u8> {
    tls.write_all(request)
        .expect("the leaf verifies against run-ca.pem");
    let (mut out, mut buf) = (Vec::new(), [0u8; 4096]);
    while !out.ends_with(tail) {
        match tls.read(&mut buf) {
            Ok(n) if n > 0 => out.extend_from_slice(&buf[..n]),
            _ => break,
        }
    }
    out
}

/// The test folder: marker files in `d`, the entry folder (its run's `server.json` written, no
/// server), a plan whose harness is `sh` waiting for `d/stop`, its start facts and the recorder
/// wrapper.
pub struct Setup {
    pub _tmp: tempfile::TempDir,
    pub d: PathBuf,
    pub entry_dir: PathBuf,
    pub plan: LaunchPlan,
    pub recorder: PathBuf,
    pub facts: StartFacts,
}

/// [`Setup`] with the wrapper `wrapper` writes in `d` (most use [`script`]).
pub fn setup_with(wrapper: impl FnOnce(&Path) -> PathBuf) -> Setup {
    // SAFETY: set before any thread of this test process reads env; sound only because nextest
    // runs each test in its own process.
    unsafe { std::env::set_var("GOALCUP_SERVER", "off") };
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path().join("m");
    std::fs::create_dir_all(&d).unwrap();
    let home = tmp.path().join("g");
    let entry_dir = cli::entry_dir(&home, RUN, ENTRY);
    std::fs::create_dir_all(&entry_dir).unwrap();
    let server = json!({"v": 1, "server": null, "run": RUN, "mode": "practice", "deadline": null,
        "repo": null, "connector": {"id": "fake", "name": "Fake", "source": "builtin"},
        "hosts_file": null, "entries": [{"id": ENTRY, "harness_version": null, "resumed": false}]});
    let run_dir = cli::run_dir(&home, RUN);
    std::fs::write(cli::server_json(&run_dir), server.to_string()).unwrap();
    let plan = plan(&d);
    let facts = facts(&entry_dir, &d, &plan.hosts, json!({"cert": "", "key": ""}));
    let recorder = wrapper(&d);
    Setup {
        _tmp: tmp,
        d,
        entry_dir,
        plan,
        recorder,
        facts,
    }
}

/// Points the proxy's `api.openai.com` at `up`, trusting its roots (files in `d`).
pub fn tls_upstream(up: &Upstream, d: &Path) {
    // SAFETY: as in [`setup_with`]; the upstream's thread reads no env.
    unsafe {
        std::env::set_var(
            "GOALCUP_TEST_UPSTREAM",
            format!("api.openai.com={}", up.addr()),
        );
        std::env::set_var("GOALCUP_TEST_EXTRA_ROOTS", up.pki().roots_file(d));
    }
}

/// A recorder wrapper `name` in `d`: it counts boots in `boots` and logs each one's number,
/// uptime and arguments in `boot_log`, runs `body` (which sees the boot number as `$n`), then
/// execs the binary.
pub fn script(d: &Path, name: &str, body: &str) -> PathBuf {
    let (dd, exe) = (d.display(), env!("CARGO_BIN_EXE_goalcup"));
    let text = format!(
        "#!/bin/sh
         read up _ < /proc/uptime
         n=$(( $(cat {dd}/boots 2>/dev/null || echo 0) + 1 )); echo $n > {dd}/boots
         echo \"$n $up $*\" >> {dd}/boot_log
         {body}
         exec \"{exe}\" \"$@\""
    );
    let path = d.join(name);
    std::fs::write(&path, text).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// The plan: a fake built-in connector, the built-in hosts, and `sh` as the harness, waiting
/// for `d/stop`.
pub fn plan(d: &Path) -> LaunchPlan {
    let harness = format!("while [ ! -e {}/stop ]; do sleep 0.05; done", d.display());
    LaunchPlan {
        connector: ConnectorRef {
            id: "fake".into(),
            name: "Fake".into(),
            sha256: "0".repeat(64),
            source: Default::default(),
        },
        harness_version: None,
        program: PathBuf::from("/bin/sh"),
        argv: vec!["-c".into(), harness],
        env: BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
        home: d.join("home"),
        lock: HomeLock::default(),
        before: Vec::new(),
        after: Vec::new(),
        ca_env: Vec::new(),
        hosts: HostList::from_file(builtin::HOSTS.as_bytes(), "model-hosts.toml").unwrap(),
        native: NativeSources::default(),
        mask_rules: Vec::new(),
        secret_values: Vec::new(),
        notes: Vec::new(),
        version_notes: Vec::new(),
        version_row: None,
        effective: Default::default(),
        unread: Vec::new(),
        log: Vec::new(),
        refusal: None,
    }
}

/// Writes `stop` in its folder when dropped, so a driver that panics still ends the harness
/// and the test fails on its assert instead of hanging.
pub struct StopOnDrop(pub PathBuf);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::write(self.0.join("stop"), "");
    }
}

/// The `recorder_down` gaps as (start_mono, end_mono, tail).
pub fn down_gaps(entry_dir: &Path) -> Vec<(u64, u64, Value)> {
    down_gaps_and_data(entry_dir).0
}

/// The `recorder_down` gaps as (start_mono, end_mono, tail), and every DATA byte in order.
pub fn down_gaps_and_data(entry_dir: &Path) -> (Vec<(u64, u64, Value)>, Vec<u8>) {
    let (mut down, mut data) = (Vec::new(), Vec::new());
    for r in records(entry_dir) {
        match r {
            Record::Gap {
                reason: GapReason::RecorderDown,
                start_mono,
                end_mono,
                tail,
                ..
            } => down.push((start_mono, end_mono, serde_json::from_slice(&tail).unwrap())),
            Record::Data { bytes, .. } => data.extend(bytes),
            _ => {}
        }
    }
    (down, data)
}

/// Whether `want` is somewhere in `data`.
pub fn has(data: &[u8], want: &[u8]) -> bool {
    data.windows(want.len()).any(|w| w == want)
}
