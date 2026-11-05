//! PRD-02 section 8.1 (ledger R3b, points 3 and 4): `watchdog::recover_with` after the watchdog
//! and then the recorder died while the harness runs. The state is built by hand: a minted CA and its key
//! file, a recorder started on a listener with the start line, `ready`, a shell harness sent as
//! `launch`, one TLS request served, then the recorder SIGKILLed and everything closed.

mod common;
mod watchdog_share;

use common::pki::HTTP11;
use common::recorder::records;
use common::upstream::{Script, Upstream};
use goalcup::cli::{self, proc_start_id, read_json, status_json, watchdog_pid};
use goalcup::connector::{HostList, builtin};
use goalcup::proxy::ca::RunCa;
use goalcup::watchdog::{self, HarnessFacts, NotRecovered, StartFacts, ToRecorder, spawn};
use goalcup_format::{GapReason, Record};
use serde_json::{Value, json, to_value};
use std::io::BufReader;
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use watchdog_share::{ENTRY, RUN, connect_tls, exchange, facts, loopback_only, until};

struct Dead {
    tmp: tempfile::TempDir,
    _up: Upstream,
    entry_dir: PathBuf,
    port: u16,
    harness: Child,
    facts: StartFacts,
    /// The run CA key's PEM, to look for where it must never be.
    key: String,
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
        std::env::set_var(
            "GOALCUP_TEST_UPSTREAM",
            format!("api.openai.com={}", up.addr()),
        );
        std::env::set_var("GOALCUP_TEST_EXTRA_ROOTS", up.pki().roots_file(tmp.path()));
    }
    let home = tmp.path().join("g");
    let entry_dir = cli::entry_dir(&home, RUN, ENTRY);
    let rec = entry_dir.join("rec");
    std::fs::create_dir_all(&rec).unwrap();
    std::fs::set_permissions(&rec, std::fs::Permissions::from_mode(0o700)).unwrap();
    let hosts = HostList::from_file(builtin::HOSTS.as_bytes(), "model-hosts.toml").unwrap();
    let ca = RunCa::mint(&hosts.ca_subtrees()).unwrap();
    ca.write_files(&entry_dir).unwrap();
    let key = rec.join(watchdog::KEY_FILE);
    let key_pem = ca.export().key;
    std::fs::write(&key, &key_pem).unwrap();
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();

    let marker = tmp.path().join("signalled");
    let script = format!(
        "trap ': > {}' TERM INT HUP USR1 QUIT; while :; do sleep 0.05; done",
        marker.display()
    );
    let harness = Command::new("/bin/sh")
        .args(["-c", &script])
        .spawn()
        .unwrap();
    let pid = harness.id();
    let launched = HarnessFacts {
        pid,
        program: "/bin/sh".into(),
        started: "2026-11-01T00:00:00Z".into(),
        start_id: proc_start_id(pid).unwrap(),
    };
    let mut facts = facts(
        &entry_dir,
        tmp.path(),
        &hosts,
        to_value(ca.export()).unwrap(),
    );
    let (listener, addr) = common::ports::held();
    let (to, theirs) = UnixStream::pair().unwrap();
    let mut recorder = spawn::recorder(&common::bin(), false, &listener, &theirs).unwrap();
    drop(theirs);
    assert!(watchdog::send_line(
        &to,
        &ToRecorder::Start(Box::new(facts.clone()))
    ));
    let mut from = BufReader::new(to.try_clone().unwrap());
    let ready = watchdog::read_line::<Value>(&mut from).flatten().unwrap();
    assert_eq!(ready["line"], "ready");
    assert!(watchdog::send_line(
        &to,
        &ToRecorder::Launch(launched.clone())
    ));
    assert!(get(&entry_dir, addr.port()).ends_with(b"hi"));
    // A boot that dies before its first LINK leaves its number to the next.
    let counts = entry_dir.join("counts.jsonl");
    until("a link", || {
        std::fs::metadata(&counts).ok().filter(|m| m.len() > 0)
    });
    let status = || read_json::<Value>(&status_json(&entry_dir));
    until("the port in status.json", || {
        status().filter(|s| s["port"] == addr.port())
    });
    drop(from);
    let mut held = Some((to, listener));
    if !keep {
        recorder.kill().unwrap();
        recorder.wait().unwrap();
        held = None;
    }
    facts.harness = Some(launched);
    Dead {
        tmp,
        key: key_pem,
        _up: up,
        entry_dir,
        port: addr.port(),
        harness,
        facts,
        recorder,
        held,
    }
}

/// One GET through the proxy on `port` to the listed host, its leaf verified against
/// `run-ca.pem` only: the answer's bytes.
fn get(entry_dir: &Path, port: u16) -> Vec<u8> {
    let mut tls = connect_tls(entry_dir, port);
    exchange(
        &mut tls,
        b"GET / HTTP/1.1\r\nHost: api.openai.com\r\n\r\n",
        b"hi",
    )
}

/// Every file under `dir`, recursively.
fn files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        match p.is_dir() {
            true => out.extend(files(&p)),
            false => out.push(p),
        }
    }
    out
}

/// R3b: recover reopens the same port with the same CA under a detached watchdog, writes one
/// `recorder_down` gap with cause `watchdog_lost`, never touches the harness, and seals
/// `harness_exit` once the harness is gone, removing the key file.
#[test]
fn watchdog_recover_restarts_the_recorder_only() {
    let mut d = setup(false);
    let try_it = |d: &Dead| watchdog::recover_with(&d.entry_dir, d.facts.clone(), &common::bin());
    assert_eq!(
        try_it(&d),
        Ok(watchdog::Recovered {
            port: d.port,
            late: false
        })
    );
    loopback_only(d.port);
    assert!(get(&d.entry_dir, d.port).ends_with(b"hi"));
    let (pid, start_id) = watchdog_pid(&d.entry_dir).expect("rec/watchdog.pid");
    assert_ne!(pid, std::process::id());
    assert!(cli::alive(pid, start_id), "the new watchdog runs");
    assert_eq!(try_it(&d), Err(NotRecovered::WatchdogAlive));
    assert!(d.harness.try_wait().unwrap().is_none(), "the harness ended");
    assert!(!d.tmp.path().join("signalled").exists(), "signalled");
    // Point 4: the key file stays 0600 while the entry lives.
    let key = d.entry_dir.join("rec").join(watchdog::KEY_FILE);
    let mode = std::fs::metadata(&key)
        .expect("the key file")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);

    d.harness.kill().unwrap();
    d.harness.wait().unwrap();
    let status = until("the seal", || {
        read_json::<Value>(&status_json(&d.entry_dir)).filter(|s| !s["seal"].is_null())
    });
    assert_eq!(status["seal"]["reason"], "harness_exit", "{status}");
    assert!(!key.exists());
    assert_eq!(try_it(&d), Err(NotRecovered::Sealed));
    // Each `recorder_down` gap with the mono of the last LINK before it: the last durable point.
    let (mut link, mut down) = (0, Vec::new());
    for r in records(&d.entry_dir) {
        match r {
            Record::Link { mono_ns, .. } => link = mono_ns,
            Record::Gap {
                reason: GapReason::RecorderDown,
                start_mono,
                end_mono,
                tail,
                ..
            } => down.push((
                link,
                start_mono,
                end_mono,
                serde_json::from_slice::<Value>(&tail).unwrap(),
            )),
            _ => {}
        }
    }
    assert_eq!(down.len(), 1, "{down:?}");
    let (durable, start, end, tail) = &down[0];
    assert_eq!(tail["cause"], "watchdog_lost", "{down:?}");
    assert!(durable <= start && start < end, "{down:?}");
    // Point 4: the key's PEM body is in no file of the entry (parts, index, goalcup.log).
    let body: String = d.key.lines().filter(|l| !l.starts_with("-----")).collect();
    let line = d.key.lines().nth(1).unwrap().as_bytes();
    assert!(body.len() > 40 && line.len() > 40);
    for f in files(d.tmp.path()) {
        let bytes = std::fs::read(&f).unwrap();
        let flat: Vec<u8> = bytes.iter().copied().filter(|b| *b != b'\n').collect();
        let found = bytes.windows(line.len()).any(|w| w == line)
            || flat.windows(body.len()).any(|w| w == body.as_bytes());
        assert!(!found, "the CA key is in {}", f.display());
    }
    // And in no decoded record, since the part files are compressed.
    for r in records(&d.entry_dir) {
        let bytes = match &r {
            Record::Data { bytes, .. } => bytes,
            Record::Event { json, .. } => json,
            Record::Open { meta, .. } => meta,
            Record::End { tail, .. } | Record::Gap { tail, .. } => tail,
            _ => continue,
        };
        assert!(
            !bytes.windows(line.len()).any(|w| w == line),
            "the CA key is in a record"
        );
    }
}

/// R3b refusals: a live recorder holding the lock is left alone, a taken port, a missing key and
/// a gone harness refuse, and none of them starts a watchdog or binds the port.
#[test]
fn watchdog_recover_refusals_start_nothing() {
    let mut d = setup(true);
    let try_it = |d: &Dead| watchdog::recover_with(&d.entry_dir, d.facts.clone(), &common::bin());
    assert_eq!(try_it(&d), Err(NotRecovered::RecordingOn));
    assert!(d.recorder.try_wait().unwrap().is_none(), "touched");
    assert!(watchdog_pid(&d.entry_dir).is_none());
    d.recorder.kill().unwrap();
    d.recorder.wait().unwrap();
    d.held = None;
    until("the port is free", || spawn::bind_exact(d.port).ok());

    let taken = spawn::bind_exact(d.port).unwrap();
    assert_eq!(try_it(&d), Err(NotRecovered::PortTaken));
    drop(taken);
    std::fs::remove_file(d.entry_dir.join("rec").join(watchdog::KEY_FILE)).unwrap();
    assert_eq!(try_it(&d), Err(NotRecovered::KeyMissing));
    // Recover starts the binary the entry recorded (so it pins nothing in this HOME), and a
    // refusal leaves that record alone.
    let pin = d.entry_dir.join("rec").join(watchdog::PIN_FILE);
    std::fs::write(&pin, common::bin().as_os_str().as_encoded_bytes()).unwrap();
    let pinned = watchdog::recover(&d.entry_dir, d.facts.clone());
    assert_eq!(pinned, Err(NotRecovered::KeyMissing));
    assert_eq!(std::fs::read(&pin).unwrap(), common::bin().as_os_str().as_encoded_bytes());
    assert!(watchdog_pid(&d.entry_dir).is_none());
    assert!(spawn::bind_exact(d.port).is_ok(), "port left bound");
    d.harness.kill().unwrap();
    let _ = d.harness.wait();
    assert_eq!(try_it(&d), Err(NotRecovered::HarnessGone));
    assert!(watchdog_pid(&d.entry_dir).is_none());
}

/// Point 3 under recover: a first recorder that ends at boot (here a mask rule its masker
/// refuses, so every normal boot exits) is reported late, not as nothing started. The watchdog
/// keeps running, its restart loop takes the recorder on into null mode, the port stays held, the
/// harness is never touched, and its exit still seals.
#[test]
fn watchdog_recover_keeps_a_late_boot() {
    let mut d = setup(false);
    let mut facts = d.facts.clone();
    facts.mask_rules = json!([{"kind": "run_secret", "header_prefix": "x-a"}]);
    let got = watchdog::recover_with(&d.entry_dir, facts, &common::bin());
    assert_eq!(
        got,
        Ok(watchdog::Recovered {
            port: d.port,
            late: true
        })
    );
    let (pid, start_id) = watchdog_pid(&d.entry_dir).expect("rec/watchdog.pid");
    assert!(cli::alive(pid, start_id), "the new watchdog runs");
    let status = || read_json::<Value>(&status_json(&d.entry_dir));
    until("null mode", || status().filter(|s| s["mode"] == "null"));
    loopback_only(d.port);
    assert!(cli::alive(pid, start_id), "the new watchdog runs");
    assert!(d.harness.try_wait().unwrap().is_none(), "the harness ended");
    assert!(!d.tmp.path().join("signalled").exists(), "signalled");

    d.harness.kill().unwrap();
    d.harness.wait().unwrap();
    let sealed = until("the seal", || status().filter(|s| !s["seal"].is_null()));
    assert_eq!(sealed["seal"]["reason"], "harness_exit", "{sealed}");
}

/// Owner's Mac one-hour run (MAC-LEARNINGS 7g): `goalcup seal` on a recovered watchdog seals with
/// the harness still running, and says so instead of "goalcup seal stopped".
#[test]
fn watchdog_recover_seal_says_the_harness_was_left() {
    let mut d = setup(false);
    let got = watchdog::recover_with(&d.entry_dir, d.facts.clone(), &common::bin());
    assert!(got.is_ok(), "{got:?}");
    let out = Command::new(common::bin())
        .args(["seal", "--yes"])
        .env("GOALCUP_HOME", d.tmp.path().join("g"))
        .env_remove(cli::time_left::FILE_ENV)
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{said}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(said.contains("is still running"), "{said}");
    assert!(!said.contains("goalcup seal stopped"), "{said}");
    assert!(d.harness.try_wait().unwrap().is_none(), "the harness ended");
    assert!(!d.tmp.path().join("signalled").exists(), "signalled");
    d.harness.kill().unwrap();
    d.harness.wait().unwrap();
}

/// Review focus 5: the recovered watchdog holds the harness home lock (`flock`, a second open
/// in this process conflicts) until it ends, so a `goalcup start` of the same harness is refused.
#[test]
fn watchdog_recover_holds_the_home_lock() {
    let mut d = setup(false);
    let got = watchdog::recover_with(&d.entry_dir, d.facts.clone(), &common::bin());
    assert!(got.is_ok(), "{got:?}");
    let homes = d.tmp.path().join("g/homes");
    std::fs::create_dir_all(&homes).unwrap();
    let f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(homes.join("fake.goalcup.lock"))
        .unwrap();
    assert!(matches!(
        f.try_lock(),
        Err(std::fs::TryLockError::WouldBlock)
    ));

    d.harness.kill().unwrap();
    d.harness.wait().unwrap();
    until("the seal", || {
        read_json::<Value>(&status_json(&d.entry_dir)).filter(|s| !s["seal"].is_null())
    });
    until("the lock freed", || f.try_lock().ok());
}

/// A run recorded under an old id (`opencode2-cli`) has its home there: the recovered watchdog
/// locks that home, not the one of the id the connector reloads as.
#[test]
fn watchdog_recover_locks_the_recorded_home() {
    let mut d = setup(false);
    let run_dir = d.entry_dir.parent().unwrap().to_path_buf();
    let server = json!({"v": 1, "server": null, "run": RUN, "mode": "practice", "deadline": null,
        "repo": null, "connector": {"id": "opencode2-cli", "name": "OpenCode", "source": "builtin"},
        "hosts_file": null, "entries": []});
    std::fs::write(cli::server_json(&run_dir), server.to_string()).unwrap();
    d.facts.connector["id"] = json!("opencode-cli");
    let got = watchdog::recover_with(&d.entry_dir, d.facts.clone(), &common::bin());
    assert!(got.is_ok(), "{got:?}");
    let homes = d.tmp.path().join("g/homes");
    std::fs::create_dir_all(&homes).unwrap();
    let open = |name: &str| {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(homes.join(name))
            .unwrap()
    };
    let old = open("opencode2-cli.goalcup.lock");
    until("the recorded home locked", || {
        matches!(old.try_lock(), Err(std::fs::TryLockError::WouldBlock)).then_some(())
    });
    assert!(open("opencode-cli.goalcup.lock").try_lock().is_ok());

    d.harness.kill().unwrap();
    d.harness.wait().unwrap();
    until("the seal", || {
        read_json::<Value>(&status_json(&d.entry_dir)).filter(|s| !s["seal"].is_null())
    });
}
