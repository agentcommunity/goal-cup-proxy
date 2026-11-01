//! PRD-02 section 3, the "Writer stall" row (PRD-17 L-R2): the real recorder under
//! `watchdog::run_with`, a `sh` harness waiting on a stop file, and a recorder wrapper that holds
//! boot 0's writer with `GOALCUP_TEST_WRITER_STALL` (boot 0 only: the hook is read on every boot).
//! A decrypted exchange is served while the writer is held (point 3), its records wait, and after
//! 60 s with no write the recorder exits 75. The watchdog starts the next boot on the same port,
//! which writes one `recorder_down` gap with cause `writer_stalled` and serves and records the
//! next exchange. Linux only.
#![cfg(target_os = "linux")]

mod common;

mod watchdog_share;

use common::pki::HTTP11;
use common::recorder::records;
use common::upstream::{Script, Upstream};
use goalcup::cli;
use goalcup::connector::{ConnectorRef, HomeLock, HostList, LaunchPlan, NativeSources, builtin};
use goalcup::watchdog;
use goalcup_format::{GapReason, Record};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;
use watchdog_share::{ENTRY, RUN, connect_tls, exchange, facts, loopback_only, until};

/// When boot 0's writer is held, after its start: past its first LINK.
const HOLD_FROM_MS: u64 = 2_000;

#[test]
fn a_stalled_writer_exits_75_and_the_next_boot_names_it() {
    let script = Script::ok(b"stall-answer").with_tls(&["api.openai.com"], &[HTTP11]);
    let up = Upstream::start(script);
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
    std::fs::write(
        cli::server_json(&cli::run_dir(&home, RUN)),
        server.to_string(),
    )
    .unwrap();

    // The first recorder process holds its writer past the 60 s stall exit; later ones do not.
    let (dd, exe) = (d.display(), env!("CARGO_BIN_EXE_goalcup"));
    let wrapper = d.join("recorder.sh");
    let text = format!(
        "#!/bin/sh\nn=$(( $(cat {dd}/boots 2>/dev/null || echo 0) + 1 )); echo $n > {dd}/boots\n\
         if [ $n = 1 ]; then export GOALCUP_TEST_WRITER_STALL={HOLD_FROM_MS}:75000; fi\n\
         exec \"{exe}\" \"$@\"\n"
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
    let facts = facts(&entry_dir, &d, &hosts, json!({"cert": "", "key": ""}));
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
    let (mut down, mut data) = (Vec::new(), Vec::new());
    for r in records(&entry_dir) {
        match r {
            Record::Gap {
                reason: GapReason::RecorderDown,
                start_mono,
                end_mono,
                tail,
                ..
            } => down.push((
                start_mono,
                end_mono,
                serde_json::from_slice::<Value>(&tail).unwrap(),
            )),
            Record::Data { bytes, .. } => data.extend(bytes),
            _ => {}
        }
    }
    // One gap over the dead span, naming the stall exit; no null mode.
    assert_eq!(down.len(), 1, "{down:?}");
    assert!(down[0].0 < down[0].1, "{down:?}");
    assert_eq!(down[0].2["cause"], "writer_stalled", "{down:?}");
    assert_eq!(down[0].2["prev"], json!({"code": 75}), "{down:?}");
    // The exchange through the next boot is in the record, decrypted.
    for want in [&b"stall-request-2"[..], b"stall-answer"] {
        assert!(
            data.windows(want.len()).any(|w| w == want),
            "{:?} not recorded",
            String::from_utf8_lossy(want)
        );
    }
}

/// One POST through the proxy on `port`; asserts the upstream's answer came back.
fn post(dir: &Path, port: u16, body: &[u8]) {
    let mut t = connect_tls(dir, port);
    let mut req = format!(
        "POST /v1/responses HTTP/1.1\r\nHost: api.openai.com\r\nConnection: close\r\n\
         Content-Length: {}\r\n\r\n",
        body.len()
    )
    .into_bytes();
    req.extend_from_slice(body);
    let answer = exchange(&mut t, &req, b"stall-answer");
    let answer = String::from_utf8_lossy(&answer);
    assert!(
        answer.starts_with("HTTP/1.1 200") && answer.ends_with("stall-answer"),
        "{answer}"
    );
}

/// Talks through boot 0 while its writer is held, waits for the next boot on the same port,
/// talks through it, and stops the harness.
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
    // Boot 0 must write a LINK before the hold, or the next boot reuses its number.
    until("a link", || {
        std::fs::metadata(dir.join("counts.jsonl"))
            .ok()
            .filter(|f| f.len() > 0)
    });
    std::thread::sleep(Duration::from_millis(HOLD_FROM_MS + 500));
    // Point 3: the harness is served while the writer is stuck; its records now wait.
    post(dir, port, b"stall-request-1");
    until("records waiting", || {
        (status()?["writer_idle_ms"].as_u64()? > 0).then_some(())
    });

    let after = until("the next boot's status", || {
        let s = status()?;
        (s["boot"].as_u64()? >= 1 && s["pid"].as_u64()? != first).then_some(s)
    });
    assert_eq!(after["port"].as_u64(), Some(u64::from(port)), "{after}");
    loopback_only(port);
    post(dir, port, b"stall-request-2");
    std::fs::write(m.join("stop"), "").unwrap();
}
