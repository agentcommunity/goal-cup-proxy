//! PRD-02 section 8 step 9 (point 3, ledger R3a): `watchdog::run_with` with a hand-built plan
//! whose harness is `sh` waiting on a stop file, and a recorder wrapper that runs the goalcup
//! binary but fails boots 2 and 3 at once. A first boot killed by SIGKILL and those two make a
//! crash loop: the next recorder runs `--null`, the proxy still answers, `status.json` says `null`
//! with `off.since`, the null recorder leaves on the `restart` line with exit 0 after 30 s of boot
//! time, and the next normal boot writes one `recorder_down` gap naming `null_mode` over the span.
//! A harness exit while the null recorder serves seals in null mode with that one gap; and after
//! a server `stop` the harness's connections are still answered until it exits.
//! Boot time here is `/proc/uptime` (`CLOCK_BOOTTIME`, the record clock). Linux only.
#![cfg(target_os = "linux")]

mod common;

use common::recorder::records;
use common::upstream::{Script, Upstream};
use goalcup::cli::{self, NULL_FLAG};
use goalcup::connector::{ConnectorRef, HomeLock, HostList, LaunchPlan, NativeSources, builtin};
use goalcup::watchdog::{self, CaPem, StartFacts};
use goalcup_format::{GapReason, Record};
use serde_json::{Value, to_value};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(90);
const RUN: &str = "r0123456789abcdef";
const ENTRY: &str = "e0123456789abcdef";

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

/// The wrapper's boot log: per boot `<n> <uptime> <args>`, and `<n> exit <code>` after a null one.
fn boot_log(d: &Path) -> Vec<Vec<String>> {
    let text = std::fs::read_to_string(d.join("boot_log")).unwrap_or_default();
    let words = |l: &str| l.split_whitespace().map(String::from).collect();
    text.lines().map(words).collect()
}

/// The uptime of boot `n`, in seconds, once it started.
fn started(d: &Path, n: &str) -> Option<f64> {
    let log = boot_log(d);
    let l = log.iter().find(|l| l[0] == n && l[1] != "exit")?;
    l[1].parse().ok()
}

/// Boots 2 and 3 exit 1 at once; a `--null` boot runs the binary as a child to note its exit;
/// every other boot is the binary itself (fds 3 and 4 pass through `exec`).
fn wrapper(d: &Path) -> PathBuf {
    let exe = env!("CARGO_BIN_EXE_goalcup");
    let body = format!(
        "case $n in 2|3) exit 1;; esac
         case \" $* \" in *' {NULL_FLAG} '*) \"{exe}\" \"$@\"; c=$?
           echo \"$n exit $c\" >> {dd}/boot_log; exit $c;; esac",
        dd = d.display()
    );
    script(d, "recorder.sh", &body)
}

/// Boot 1 gets a server `stop` (reason `admin`) on its control socket 4 s after it starts, as
/// its uploader's forwarder would send it; every boot is the binary itself.
fn stop_wrapper(d: &Path) -> PathBuf {
    stop_wrapper_then(d, "")
}

/// As [`stop_wrapper`], but every boot after the first exits 1 at once.
fn stop_crash_wrapper(d: &Path) -> PathBuf {
    stop_wrapper_then(d, "*) exit 1;;")
}

fn stop_wrapper_then(d: &Path, later: &str) -> PathBuf {
    let word = r#"{"line":"server","stop":true,"reason":"admin","seconds_left":null,"asked_at":0}"#;
    let body = format!("case $n in 1) ( sleep 4; printf '%s\\n' '{word}' >&4 ) & ;; {later} esac");
    script(d, "stop_recorder.sh", &body)
}

/// Every boot after the first, null ones too, exits 1 at once.
fn crash_wrapper(d: &Path) -> PathBuf {
    script(d, "crash_recorder.sh", "case $n in 1) ;; *) exit 1;; esac")
}

/// A recorder wrapper `name` in `d`: it counts boots in `boots` and logs each one's number,
/// uptime and arguments in `boot_log`, runs `body` (which sees the boot number as `$n`), then
/// execs the binary.
fn script(d: &Path, name: &str, body: &str) -> PathBuf {
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

fn plan(d: &Path) -> LaunchPlan {
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
        unread: Vec::new(),
        log: Vec::new(),
        refusal: None,
    }
}

fn facts(plan: &LaunchPlan, entry_dir: &Path, d: &Path) -> StartFacts {
    StartFacts {
        run: RUN.into(),
        entry: ENTRY.into(),
        segment: 0,
        entry_dir: entry_dir.to_path_buf(),
        deadline: None,
        server_time: None,
        local_wall: 0,
        workspace: d.to_path_buf(),
        connector: to_value(&plan.connector).unwrap(),
        harness_version: None,
        keep: true,
        build: None,
        hosts: to_value(&plan.hosts).unwrap(),
        hosts_sha256: plan.hosts.sha256().to_string(),
        native: to_value(&plan.native).unwrap(),
        mask_rules: serde_json::json!([]),
        values: serde_json::json!([]),
        ca: CaPem {
            cert: String::new(),
            key: String::new(),
        },
        prev: None,
        harness: None,
        null_since: None,
        notes: Vec::new(),
    }
}

/// A plain request to `up` through the proxy on `port`: a CONNECT tunnel, then one GET.
fn through_proxy(port: u64, up: std::net::SocketAddr) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", port as u16)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    write!(s, "CONNECT {up} HTTP/1.1\r\nHost: {up}\r\n\r\n").unwrap();
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut b = [0u8];
        assert_eq!(s.read(&mut b).unwrap(), 1, "the proxy closed the CONNECT");
        head.push(b[0]);
    }
    assert!(head.starts_with(b"HTTP/1.1 200"), "{head:?}");
    s.write_all(b"GET / HTTP/1.1\r\nHost: up\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut answer = String::new();
    let _ = s.read_to_string(&mut answer);
    answer
}

/// The test folder: marker files in `d`, the entry folder, the plan and the recorder wrapper.
struct Setup {
    _tmp: tempfile::TempDir,
    d: PathBuf,
    entry_dir: PathBuf,
    plan: LaunchPlan,
    recorder: PathBuf,
    facts: StartFacts,
}

fn setup() -> Setup {
    setup_with(wrapper)
}

fn setup_with(wrapper: fn(&Path) -> PathBuf) -> Setup {
    // SAFETY: set before any thread of this test process starts; sound only because nextest
    // runs each test in its own process.
    unsafe { std::env::set_var("GOALCUP_SERVER", "off") };
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path().join("m");
    std::fs::create_dir_all(&d).unwrap();
    let home = tmp.path().join("g");
    let entry_dir = cli::entry_dir(&home, RUN, ENTRY);
    std::fs::create_dir_all(&entry_dir).unwrap();
    let server = serde_json::json!({
        "v": 1, "server": null, "run": RUN, "mode": "practice", "deadline": null,
        "repo": null, "connector": {"id": "fake", "name": "Fake", "source": "builtin"},
        "hosts_file": null, "entries": [{"id": ENTRY, "harness_version": null, "resumed": false}],
    });
    std::fs::write(
        cli::server_json(&cli::run_dir(&home, RUN)),
        server.to_string(),
    )
    .unwrap();
    let (plan, recorder) = (plan(&d), wrapper(&d));
    let facts = facts(&plan, &entry_dir, &d);
    Setup {
        _tmp: tmp,
        d,
        entry_dir,
        plan,
        recorder,
        facts,
    }
}

/// Kills boot 1 after its first LINK, so boots 1 to 3 make a crash loop, and waits for the null
/// boot's `status.json`.
fn crash_to_null(dir: &Path) -> Value {
    let status = || cli::read_json::<Value>(&cli::status_json(dir));
    let first = until("boot 0", || {
        status()?.get("pid")?.as_u64().filter(|p| *p > 0)
    });
    // A boot that dies before its first LINK leaves its number to the next, so wait for one.
    until("a link", || {
        std::fs::metadata(dir.join("counts.jsonl"))
            .ok()
            .filter(|m| m.len() > 0)
    });
    // SAFETY: a signal to one pid, the first recorder's.
    unsafe { libc::kill(first as libc::pid_t, libc::SIGKILL) };
    until("null mode", || status().filter(|s| s["mode"] == "null"))
}

/// The `recorder_down` gaps as (start_mono, end_mono, tail).
fn down_gaps(entry_dir: &Path) -> Vec<(u64, u64, Value)> {
    records(entry_dir)
        .into_iter()
        .filter_map(|r| match r {
            Record::Gap {
                reason: GapReason::RecorderDown,
                start_mono,
                end_mono,
                tail,
                ..
            } => Some((start_mono, end_mono, serde_json::from_slice(&tail).unwrap())),
            _ => None,
        })
        .collect()
}

/// Point 3 and ledger R3a: a crash loop goes to null mode, which serves and retries a normal
/// boot after 30 s by the `restart` line; the sealed entry holds one `null_mode` gap over it.
#[test]
fn watchdog_null_crash_loop_serves_and_retries() {
    let up = Upstream::start(Script::ok(b"hi"));
    let Setup {
        _tmp,
        d,
        entry_dir,
        plan,
        recorder,
        facts,
    } = setup();

    let (dir, m, upstream) = (entry_dir.clone(), d.clone(), up.addr());
    let driver = std::thread::spawn(move || {
        let status = || cli::read_json::<Value>(&cli::status_json(&dir));
        let null = crash_to_null(&dir);
        assert!(null["off"]["since"].is_string(), "{null}");
        let answer = through_proxy(null["port"].as_u64().unwrap(), upstream);
        assert!(
            answer.starts_with("HTTP/1.1 200") && answer.ends_with("hi"),
            "{answer}"
        );
        until("the normal try", || started(&m, "5"));
        until("the try's status", || {
            status().filter(|s| s["mode"] != "null")
        });
        std::fs::write(m.join("stop"), "").unwrap();
    });
    let _ = watchdog::run_with(plan, facts, &recorder);
    driver.join().unwrap();
    drop(up);

    let log = boot_log(&d);
    assert!(
        log.iter()
            .any(|l| l[0] == "4" && l.contains(&NULL_FLAG.to_string()))
    );
    assert!(log.iter().any(|l| l[..] == ["4", "exit", "0"]), "{log:?}");
    let (null_at, try_at) = (started(&d, "4").unwrap(), started(&d, "5").unwrap());
    // The wrapper reads its uptime a little after the wait began: 0.1 s of slack.
    let waited = try_at - null_at;
    assert!((29.9..=40.0).contains(&waited), "{waited}");

    until("the sealed status.json", || {
        cli::read_json::<Value>(&cli::status_json(&entry_dir)).filter(|s| !s["seal"].is_null())
    });
    let down = down_gaps(&entry_dir);
    assert_eq!(down.len(), 1, "{down:?}");
    let (start, end, tail) = &down[0];
    assert_eq!(tail["cause"], "null_mode", "{tail}");
    assert!(start < end);
    let ns = |s: f64| (s * 1e9) as u64;
    assert!(*start <= ns(null_at) && *end >= ns(try_at), "{down:?}");
}

/// Section 11 "In null mode" (PRD-02 section 12): the harness exits while the null recorder
/// serves. The null recorder seals at once, no normal try follows, and the sealed entry holds one
/// `null_mode` gap, the seal with reason `harness_exit`, an unbroken link chain read whole by the
/// format reader, and an index that ends with its `R` line.
#[test]
fn watchdog_null_harness_exit_seals_in_null_mode() {
    let Setup {
        _tmp,
        d,
        entry_dir,
        plan,
        recorder,
        facts,
    } = setup();
    let (dir, m) = (entry_dir.clone(), d.clone());
    let driver = std::thread::spawn(move || {
        crash_to_null(&dir);
        std::fs::write(m.join("stop"), "").unwrap();
    });
    let _ = watchdog::run_with(plan, facts, &recorder);
    driver.join().unwrap();

    // The watchdog returned on the null recorder's `sealed`; no normal try was started.
    let log = boot_log(&d);
    assert!(started(&d, "5").is_none(), "{log:?}");
    let text = std::fs::read_to_string(cli::goalcup_log(&entry_dir)).unwrap();
    assert!(!text.contains("null_seal_left"));
    let st = cli::read_json::<Value>(&cli::status_json(&entry_dir)).unwrap();
    assert_eq!(st["mode"], "sealed", "{st}");
    assert_eq!(st["seal"]["reason"], "harness_exit", "{st}");
    assert!(!entry_dir.join("rec").join(watchdog::KEY_FILE).exists());

    let down = down_gaps(&entry_dir);
    assert_eq!(down.len(), 1, "{down:?}");
    assert_eq!(down[0].2["cause"], "null_mode", "{down:?}");
    assert!(down[0].0 < down[0].1);

    // The whole entry, every part file in order: every LINK recomputes, the seal event comes
    // right before the last LINK, and the last part file ends at its index's last line, an `R`.
    let rec = entry_dir.join("rec");
    let items: Vec<_> = goalcup_format::read::open_dir(&rec)
        .unwrap()
        .map(|i| i.unwrap())
        .collect();
    assert!(items.iter().all(|i| i.link_ok != Some(false)));
    assert!(matches!(
        items.last().map(|i| &i.rec),
        Some(Record::Link { .. })
    ));
    let Record::Event { json, .. } = &items[items.len() - 2].rec else {
        panic!("the seal event comes right before the last LINK");
    };
    let seal: Value = serde_json::from_slice(json).unwrap();
    assert_eq!(
        (&seal["ev"], &seal["reason"]),
        (&"seal".into(), &"harness_exit".into())
    );
    let parts = goalcup_format::read::part_files(&rec).unwrap();
    let last = parts.last().unwrap();
    let name = last.file_name().unwrap().to_str().unwrap();
    let idx = last.with_file_name(format!("{}.idx", name.trim_end_matches(".gcr.zst")));
    let line = *goalcup_format::read::read_index(&idx)
        .unwrap()
        .last()
        .unwrap();
    assert_eq!(line.kind, goalcup_format::IndexKind::R);
    assert_eq!(std::fs::metadata(last).unwrap().len(), line.c_end);
}

/// Point 3 and section 8 step 10: after a server `stop` the entry seals with `server_stop` and the
/// harness runs on. The recorder that sealed ends after its wrap-up, and the watchdog starts the
/// next at once on the sealed entry, so the harness's connections are still answered until it
/// exits, and the seal stays as it was.
#[test]
fn watchdog_after_a_server_stop_serves_until_the_harness_exits() {
    let up = Upstream::start(Script::ok(b"hi"));
    let Setup {
        _tmp,
        d,
        entry_dir,
        plan,
        recorder,
        facts,
    } = setup_with(stop_wrapper);
    let (dir, m, upstream) = (entry_dir.clone(), d.clone(), up.addr());
    let driver = std::thread::spawn(move || {
        let status = || cli::read_json::<Value>(&cli::status_json(&dir));
        let st = until("the server_stop seal", || {
            status().filter(|s| s["seal"]["reason"] == "server_stop")
        });
        until("the boot after the seal", || started(&m, "2"));
        let answer = through_proxy(st["port"].as_u64().unwrap(), upstream);
        assert!(
            answer.starts_with("HTTP/1.1 200") && answer.ends_with("hi"),
            "{answer}"
        );
        std::fs::write(m.join("stop"), "").unwrap();
    });
    let _ = watchdog::run_with(plan, facts, &recorder);
    driver.join().unwrap();
    drop(up);
    let st = cli::read_json::<Value>(&cli::status_json(&entry_dir)).unwrap();
    assert_eq!(st["seal"]["reason"], "server_stop", "{st}");
}

/// Point 3 and ledger R3: a null recorder that dies mid-run is replaced at once by another null
/// boot, so the harness's next connection is answered within seconds, not at the end of the
/// null wait.
#[test]
fn watchdog_null_killed_is_replaced_at_once() {
    let up = Upstream::start(Script::ok(b"hi"));
    let Setup {
        _tmp,
        d,
        entry_dir,
        plan,
        recorder,
        facts,
    } = setup();
    let (dir, m, upstream) = (entry_dir.clone(), d.clone(), up.addr());
    let driver = std::thread::spawn(move || {
        let null = crash_to_null(&dir);
        let port = null["port"].as_u64().unwrap();
        assert!(through_proxy(port, upstream).ends_with("hi"));
        let pid = null["pid"].as_u64().unwrap();
        // SAFETY: a signal to one pid, the null recorder's.
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
        until("boot 4's end", || {
            boot_log(&m)
                .iter()
                .find(|l| l[..2] == ["4", "exit"])
                .cloned()
        });
        let t = Instant::now();
        let answer = through_proxy(port, upstream);
        let waited = t.elapsed();
        std::fs::write(m.join("stop"), "").unwrap();
        (answer, waited)
    });
    let _ = watchdog::run_with(plan, facts, &recorder);
    let (answer, waited) = driver.join().unwrap();
    drop(up);
    assert!(answer.ends_with("hi"), "{answer}");
    assert!(waited < Duration::from_secs(5), "{waited:?}");
    let log = boot_log(&d);
    assert!(
        log.iter()
            .any(|l| l[0] == "5" && l.contains(&NULL_FLAG.to_string())),
        "{log:?}"
    );
}

/// Point 3 and section 8 step 10: after a server `stop`, boots on the sealed entry that die at
/// boot are spaced out (never one per tick) and never start null mode, and the seal stays.
#[test]
fn watchdog_sealed_entry_boots_are_spaced_and_never_null() {
    let Setup {
        _tmp,
        d,
        entry_dir,
        plan,
        recorder,
        facts,
    } = setup_with(stop_crash_wrapper);
    let (dir, m) = (entry_dir.clone(), d.clone());
    let driver = std::thread::spawn(move || {
        let status = || cli::read_json::<Value>(&cli::status_json(&dir));
        until("the server_stop seal", || {
            status().filter(|s| s["seal"]["reason"] == "server_stop")
        });
        until("the boot after the seal", || started(&m, "2"));
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(10) {
            if let Some(st) = status() {
                assert_eq!(st["seal"]["reason"], "server_stop", "{st}");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        std::fs::write(m.join("stop"), "").unwrap();
    });
    let _ = watchdog::run_with(plan, facts, &recorder);
    driver.join().unwrap();
    let log = boot_log(&d);
    assert!(
        !log.iter().any(|l| l.contains(&NULL_FLAG.to_string())),
        "{log:?}"
    );
    // Spaced 1, 2, 4, 8 s: about five boots in 10 s, where one per tick would be dozens.
    assert!((3..=8).contains(&log.len()), "{log:?}");
    let st = cli::read_json::<Value>(&cli::status_json(&entry_dir)).unwrap();
    assert_eq!(st["seal"]["reason"], "server_stop", "{st}");
}

/// Ledger R3a (locked): when the null boots also die at boot, their spacing never pushes the
/// normal try past the 30 s wait.
#[test]
fn watchdog_null_boots_dying_at_boot_keep_the_wait() {
    let Setup {
        _tmp,
        d,
        entry_dir,
        plan,
        recorder,
        facts,
    } = setup_with(crash_wrapper);
    let (dir, m) = (entry_dir.clone(), d.clone());
    let null = NULL_FLAG.to_string();
    let driver = std::thread::spawn(move || {
        let status = || cli::read_json::<Value>(&cli::status_json(&dir));
        let first = until("boot 0", || {
            status()?.get("pid")?.as_u64().filter(|p| *p > 0)
        });
        // SAFETY: a signal to one pid, the first recorder's.
        unsafe { libc::kill(first as libc::pid_t, libc::SIGKILL) };
        let first_null = until("the first null boot", || {
            let log = boot_log(&m);
            log.iter().find(|l| l.contains(&null))?[1].parse::<f64>().ok()
        });
        let next_try = until("the normal try", || {
            let log = boot_log(&m);
            let n = log.iter().position(|l| l.contains(&null))?;
            let t = log[n..].iter().find(|l| !l.contains(&null))?;
            t[1].parse::<f64>().ok()
        });
        std::fs::write(m.join("stop"), "").unwrap();
        next_try - first_null
    });
    let _ = watchdog::run_with(plan, facts, &recorder);
    let wait = driver.join().unwrap();
    assert!((29.0..=33.0).contains(&wait), "{wait} {:?}", boot_log(&d));
}
