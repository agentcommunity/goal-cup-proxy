//! PRD-02 section 3, the "Writer stall" row (PRD-17 L-R2, with the in-process cases named in the
//! PRD-02 amendment of this file): the real recorder under `watchdog::run_with`, a `sh` harness
//! waiting on a stop file, and a recorder wrapper that holds boot 0's writer with
//! `GOALCUP_TEST_WRITER_STALL` (boot 0 only: the hook is read on every boot). A decrypted
//! exchange is served while the writer is held (point 3), its records wait, and after 60 s with
//! no write the recorder exits 75. The watchdog starts the next boot on the same port, which
//! writes one `recorder_down` gap with cause `writer_stalled` over the lost span (the held
//! exchange inside it) and serves and records the next exchange. Linux only.
#![cfg(target_os = "linux")]

mod common;

mod watchdog_share;

use common::pki::HTTP11;
use common::upstream::{Script, Upstream};
use goalcup::cli;
use goalcup::pipeline::Mono;
use goalcup::watchdog;
use serde_json::{Value, json};
use std::path::Path;
use std::time::Duration;
use watchdog_share::{
    Setup, StopOnDrop, connect_tls, down_gaps_and_data, exchange, has, script, setup_with,
    tls_upstream, until,
};

/// When boot 0's writer is held, after its start: well past its first LINK (one tick, 1 s),
/// with room for a loaded box.
const HOLD_FROM_MS: u64 = 5_000;

#[test]
fn a_stalled_writer_exits_75_and_the_next_boot_names_it() {
    let up = Upstream::start(Script::ok(b"stall-answer").with_tls(&["api.openai.com"], &[HTTP11]));
    // The first recorder process holds its writer past the 60 s stall exit; later ones do not.
    let Setup {
        _tmp,
        d,
        entry_dir,
        plan,
        recorder,
        facts,
    } = setup_with(|d| {
        let body = format!(
            "if [ $n = 1 ]; then export GOALCUP_TEST_WRITER_STALL={HOLD_FROM_MS}:75000; fi"
        );
        script(d, "recorder.sh", &body)
    });
    tls_upstream(&up, &d);

    let (dir, m) = (entry_dir.clone(), d.clone());
    let driver = std::thread::spawn(move || drive(&dir, &m));
    let _ = watchdog::run_with(plan, facts, &recorder);
    let posted = driver.join().unwrap();
    drop(up);

    let st = until("the sealed status.json", || {
        cli::read_json::<Value>(&cli::status_json(&entry_dir)).filter(|s| !s["seal"].is_null())
    });
    assert_eq!(st["seal"]["reason"], "harness_exit", "{st}");
    let (down, data) = down_gaps_and_data(&entry_dir);
    // One gap over the dead span, naming the stall exit; no null mode.
    assert_eq!(down.len(), 1, "{down:?}");
    let (start, end, tail) = &down[0];
    assert_eq!(tail["cause"], "writer_stalled", "{down:?}");
    assert_eq!(tail["prev"], json!({"code": 75}), "{down:?}");
    // No drop without a GAP: the exchange served while the writer was held is in DATA, or the
    // gap starts at or before it (the last durable point of boot 0) and ends after it.
    if !has(&data, b"stall-request-1") {
        assert!(
            *start <= posted && posted < *end,
            "{down:?}, posted at {posted}"
        );
    }
    // The exchange through the next boot is in the record, decrypted.
    for want in [&b"stall-request-2"[..], b"stall-answer"] {
        assert!(
            has(&data, want),
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
/// talks through it, and stops the harness (also when it panics). Returns the record-clock time
/// just before the held exchange.
fn drive(dir: &Path, m: &Path) -> u64 {
    let _stop = StopOnDrop(m.to_path_buf());
    let status = || cli::read_json::<Value>(&cli::status_json(dir));
    let (first, port) = until("boot 0", || {
        let s = status()?;
        Some((
            s["pid"].as_u64().filter(|p| *p > 0)?,
            s["port"].as_u64()? as u16,
        ))
    });
    // Boot 0 must write a LINK before the hold, or the next boot reuses its number.
    until("a link", || {
        std::fs::metadata(dir.join("counts.jsonl"))
            .ok()
            .filter(|f| f.len() > 0)
    });
    // The wrapper's start is before the writer's, so this bounds the LINK's time after it.
    let boot0 = boot_log_uptime(m);
    let to_link = uptime() - boot0;
    assert!(
        to_link * 1000.0 < HOLD_FROM_MS as f64,
        "boot 0's first LINK came {to_link} s after its start, past the hold: a loaded box"
    );
    std::thread::sleep(Duration::from_millis(HOLD_FROM_MS + 500));
    // Point 3: the harness is served while the writer is stuck; its records now wait.
    let posted = Mono::now().ns();
    post(dir, port, b"stall-request-1");
    until("records waiting", || {
        (status()?["writer_idle_ms"].as_u64()? > 0).then_some(())
    });

    let after = until("the next boot's status", || {
        let s = status()?;
        (s["boot"].as_u64()? >= 1 && s["pid"].as_u64()? != first).then_some(s)
    });
    assert_eq!(after["port"].as_u64(), Some(u64::from(port)), "{after}");
    post(dir, port, b"stall-request-2");
    posted
}

/// Boot time now, in seconds (`/proc/uptime`, `CLOCK_BOOTTIME`).
fn uptime() -> f64 {
    let text = std::fs::read_to_string("/proc/uptime").unwrap();
    text.split_whitespace().next().unwrap().parse().unwrap()
}

/// The boot time at which the wrapper started boot 0, from its `boot_log`.
fn boot_log_uptime(m: &Path) -> f64 {
    let log = std::fs::read_to_string(m.join("boot_log")).unwrap();
    let first = log.lines().find(|l| l.starts_with("1 ")).unwrap();
    first.split_whitespace().nth(1).unwrap().parse().unwrap()
}
