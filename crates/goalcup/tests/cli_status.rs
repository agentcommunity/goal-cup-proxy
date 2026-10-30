//! `goalcup status` (PRD-10 section 4): the recording state from facts in the table's order, one
//! problem line per cause, the note lines, pid liveness, and `status --json` passing the three files
//! through. Facts and the contract word `PROBLEM:` only; every sentence is compared with `text.rs`.

use std::path::Path;
use std::process::{Command, Output};

use goalcup::cli::status::{Ctx, Facts, State, block, decide, problems};
use goalcup::cli::{self, ConnectorSource, EntryFacts, RunMode, ServerJson, text};
use goalcup::pipeline::counts::TunnelHost;
use goalcup::pipeline::status::{
    AfterSeal, DurableStatus, HarnessStatus, Losing, Mode, Off, STATUS_V, Status, Written,
};
use goalcup::watchdog::Exit;
use serde_json::{Value, json};

const T0: u64 = 1_794_045_600; // 2026-11-07T10:00:00Z
const RUN: &str = "r0123456789abcdef";
const ENTRY: &str = "e0123456789abcdef";

fn status(mode: Mode) -> Status {
    Status {
        v: STATUS_V,
        updated: "2026-11-07T10:00:00Z".into(),
        pid: 0,
        start_id: 0,
        boot: 0,
        port: 41_000,
        mode,
        harness: None,
        written: Written::default(),
        last_request_at: None,
        last_native_at: None,
        losing: Losing::default(),
        off: Off::default(),
        notes: Vec::new(),
        held_bytes: 0,
        held_peak: 0,
        guard: 0,
        writer_idle_ms: 0,
        durable: DurableStatus::default(),
        deadline: None,
        sealing: None,
        seal: None,
        cut_files: None,
        outlived: None,
        after_seal: None,
    }
}

fn facts(mode: Mode, recorder: bool, watchdog: bool, harness: bool, age: u64) -> Facts {
    Facts {
        mode,
        recorder,
        watchdog,
        harness,
        updated: T0,
        off_since: None,
        losing: Losing::default(),
        writer_idle_ms: 0,
        now: T0 + age,
    }
}

fn cx(harness_alive: bool, now: u64) -> Ctx<'static> {
    Ctx {
        name: "Agent",
        run: RUN,
        harness_alive,
        deadline: Some(T0 + 86_400),
        now,
    }
}

#[test]
fn state_rows_in_table_order() {
    let sealed = decide(&facts(Mode::Sealed, false, false, false, 999));
    assert_eq!(sealed, State::Sealed { at: T0 });
    for mode in [Mode::Recording, Mode::Losing, Mode::Null] {
        assert_eq!(
            decide(&facts(mode, false, true, true, 5)),
            State::Restarting { since: T0 }
        );
        assert_eq!(
            decide(&facts(mode, false, true, true, 31)),
            State::OffRestarting { since: T0 }
        );
    }
    let recover = decide(&facts(Mode::Recording, false, false, true, 5));
    assert_eq!(recover, State::Recover { since: T0 });
    let line = text::state_line(&recover, RUN);
    assert!(line.starts_with(text::PROBLEM) && line.ends_with("goalcup recover"));
    for (age, want) in [
        (60, State::Continue { since: T0 }),
        (121, State::Stale { since: T0 }),
    ] {
        let got = decide(&facts(Mode::Recording, false, false, false, age));
        assert_eq!(got, want);
        let line = text::state_line(&got, RUN);
        assert!(line.starts_with(text::PROBLEM));
        assert!(line.ends_with(&format!("goalcup continue {RUN}")), "{line}");
    }
    let mut null = facts(Mode::Null, true, true, true, 3);
    null.off_since = Some(T0 - 600);
    assert_eq!(decide(&null), State::Null { since: T0 - 600 });
    assert_eq!(
        decide(&facts(Mode::Recording, true, true, true, 11)),
        State::NotAnswering { since: T0 }
    );
    let mut losing = facts(Mode::Losing, true, true, true, 1);
    losing.losing = Losing {
        streams_in_gap: 2,
        dropped_bytes: 12_000_000,
    };
    let got = decide(&losing);
    assert_eq!(
        got,
        State::Losing {
            streams: 2,
            bytes: 12_000_000
        }
    );
    assert!(text::state_line(&got, RUN).contains(&text::size_words(12_000_000)));
    let mut slow = facts(Mode::WriterSlow, true, true, true, 1);
    slow.writer_idle_ms = 14_500;
    assert_eq!(decide(&slow), State::WriterSlow { secs: 14 });
    assert_eq!(
        decide(&facts(Mode::Recording, true, true, true, 1)),
        State::Recording
    );
    assert!(!text::state_line(&State::Recording, RUN).starts_with(text::PROBLEM));
}

/// Null mode never names a retry count: the only number besides its time is the 5-minute cap.
#[test]
fn null_line_has_no_retry_count() {
    let state = State::Null { since: T0 };
    let line = text::state_line(&state, RUN).replace(&text::local_time(T0, true), "");
    assert!(line.starts_with(text::PROBLEM));
    let digits: String = line.chars().filter(char::is_ascii_digit).collect();
    assert_eq!(digits, "5", "{line}");
}

fn count_problems(s: &str) -> usize {
    s.matches(text::PROBLEM).count()
}

#[test]
fn one_problem_line_per_cause() {
    let mut st = status(Mode::Recording);
    let tunnel = json!({"what": "long_tunnel", "host": "files.example.net", "why": "upload", "up": 3_000_000});
    let ca = json!({"what": "ca_refused", "host": "pinned.example.com"});
    st.notes = vec![tunnel.clone(), ca.clone(), tunnel, ca];
    st.written.counts.tunnel_hosts.insert(
        "pinned.example.com".into(),
        TunnelHost {
            conns: 3,
            up: 10,
            down: 20,
            listed: Some("tls_refused".into()),
        },
    );
    for (alive, state) in [
        (true, State::Recording),
        (true, State::Recover { since: T0 }),
        (false, State::Continue { since: T0 }),
        (false, State::Stale { since: T0 }),
    ] {
        let lines = problems(&st, &state, &cx(alive, T0 + 5));
        let want = 2 + usize::from(state.is_problem());
        assert_eq!(
            lines
                .iter()
                .filter(|l| l.starts_with(text::PROBLEM))
                .count(),
            want,
            "{lines:#?}"
        );
        let shown = block(&st, &state, None, &cx(alive, T0 + 5), &[false]);
        assert_eq!(count_problems(&shown), want, "{shown}");
    }
}

#[test]
fn every_note_gives_its_line() {
    let mut st = status(Mode::Losing);
    st.notes = vec![
        json!({"what": "native_missing", "row": "transcript"}),
        json!({"what": "native_other_device", "row": "sessions", "path": "data/sessions"}),
        json!({"what": "long_tunnel", "host": "stream.example.net", "why": "stream", "secs": 720}),
        json!({"what": "long_tunnel", "host": "up.example.net", "why": "upload", "up": 3_000_000}),
        json!({"what": "ca_refused", "host": "refuses.example.com"}),
        json!({"what": "parse_stopped", "conn": 7, "reason": "head", "at": "2026-11-07T09:10:00Z"}),
        json!({"what": "local_model", "host": "127.0.0.1:11434"}),
        json!({"what": "fd_limit", "limit": 256}),
        json!({"what": "brand_new_note"}),
    ];
    st.written.counts.tunnel_hosts.insert(
        "proxy.example.com".into(),
        TunnelHost {
            conns: 1,
            up: 1,
            down: 1,
            listed: Some("upstream_untrusted".into()),
        },
    );
    st.written.counts.mask_failclosed_bytes = 2_000_000;
    let lines = problems(&st, &State::Recording, &cx(true, T0));
    let want = [
        text::native_missing("transcript", "Agent"),
        text::native_other_device("data/sessions", "Agent"),
        text::long_tunnel("stream.example.net", Some(12), 0),
        text::long_tunnel("up.example.net", None, 3_000_000),
        text::listed_tunnel("refuses.example.com", "tls_refused"),
        text::parse_stopped(None, Some(T0 - 3000)),
        text::local_model("127.0.0.1:11434"),
        text::fd_limit(Some(256)),
        text::unknown_note("brand_new_note"),
        text::listed_tunnel("proxy.example.com", "upstream_untrusted"),
        text::fail_closed(2_000_000),
    ];
    for w in &want {
        assert!(lines.contains(w), "missing {w}\nin {lines:#?}");
    }
    assert_ne!(
        text::listed_tunnel("h", "upstream_untrusted"),
        text::listed_tunnel("h", "tls_refused")
    );
    let tunnel = text::long_tunnel("up.example.net", None, 1);
    let (seal, cont) = (
        tunnel.find("goalcup seal").unwrap(),
        tunnel.find("goalcup continue").unwrap(),
    );
    assert!(seal < cont, "{tunnel}");
}

#[test]
fn request_silence_lines() {
    let mut st = status(Mode::Recording);
    st.harness = Some(HarnessStatus {
        pid: 1,
        started: "2026-11-07T10:00:00Z".into(),
        start_id: 1,
        exit: None,
    });
    let quiet = text::quiet_proxy("Agent");
    let none = text::NO_REQUEST_YET.to_owned();
    assert!(!problems(&st, &State::Recording, &cx(true, T0 + 200)).contains(&none));
    assert!(problems(&st, &State::Recording, &cx(true, T0 + 301)).contains(&none));
    st.written.model_requests = 4;
    st.last_request_at = Some("2026-11-07T10:00:00Z".into());
    st.last_native_at = Some("2026-11-07T10:09:00Z".into());
    assert!(problems(&st, &State::Recording, &cx(true, T0 + 601)).contains(&quiet));
    assert!(!problems(&st, &State::Recording, &cx(true, T0 + 599)).contains(&quiet));
    assert!(!problems(&st, &State::Recording, &cx(false, T0 + 601)).contains(&quiet));
}

#[test]
fn after_seal_and_agent_exited() {
    let mut st = status(Mode::Sealed);
    st.harness = Some(HarnessStatus {
        pid: 1,
        started: "2026-11-07T08:00:00Z".into(),
        start_id: 1,
        exit: Some(Exit::Code(0)),
    });
    st.after_seal = Some(AfterSeal {
        bytes: 0,
        hosts: vec!["api.example.com".into()],
    });
    let state = State::Sealed { at: T0 };
    let c = cx(false, T0);
    let exited = text::agent_exited(
        "Agent",
        Exit::Code(0),
        &text::end_words(T0 + 86_400, T0),
        RUN,
    );
    let after = text::after_seal("Agent", 2_000_000, &["api.example.com".into()]);
    let lines = problems(&st, &state, &c);
    assert!(lines.contains(&exited) && !lines.contains(&after));
    st.after_seal.as_mut().unwrap().bytes = 2_000_000;
    assert!(problems(&st, &state, &c).contains(&after));
}

#[test]
fn liveness() {
    let me = std::process::id();
    let start = cli::proc_start_id(me).expect("own start id");
    assert!(!cli::goalcup_alive(me), "a test binary is not goalcup");
    assert!(cli::harness_alive(me, start));
    assert!(!cli::harness_alive(me, start + 1));
    let mut child = Command::new("true").spawn().unwrap();
    let dead = child.id();
    child.wait().unwrap();
    assert!(!cli::goalcup_alive(dead) && !cli::harness_alive(dead, start));
    assert!(!cli::goalcup_alive(0) && !cli::harness_alive(0, 0));

    let dir = tempfile::tempdir().unwrap();
    assert!(!cli::watchdog_alive(dir.path()));
    std::fs::create_dir(dir.path().join("rec")).unwrap();
    let pid_file = dir.path().join("rec/watchdog.pid");
    std::fs::write(&pid_file, format!("{me} {start}\n")).unwrap();
    assert_eq!(cli::watchdog_pid(dir.path()), Some((me, start)));
    assert!(
        !cli::watchdog_alive(dir.path()),
        "alive, but not a goalcup binary"
    );
    for odd in ["", "12", "12 x", "12 3 4", "x 3"] {
        std::fs::write(&pid_file, odd).unwrap();
        assert_eq!(cli::watchdog_pid(dir.path()), None, "{odd:?}");
    }
}

/// A temp home with one run and one part.
struct Home {
    goalcup: tempfile::TempDir,
    user: tempfile::TempDir,
}

impl Home {
    fn new() -> Home {
        Home {
            goalcup: tempfile::tempdir().unwrap(),
            user: tempfile::tempdir().unwrap(),
        }
    }

    fn entry(&self) -> std::path::PathBuf {
        self.goalcup.path().join("runs").join(RUN).join(ENTRY)
    }

    fn write(&self, name: &str, value: &Value) {
        let dir = if name == "server.json" {
            self.entry().parent().unwrap().to_owned()
        } else {
            self.entry()
        };
        std::fs::create_dir_all(self.entry()).unwrap();
        std::fs::write(dir.join(name), serde_json::to_vec(value).unwrap()).unwrap();
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_goalcup"))
            .args(args)
            .env("GOALCUP_SERVER", "off")
            .env("GOALCUP_HOME", self.goalcup.path())
            .env("HOME", self.user.path())
            .env("XDG_CONFIG_HOME", self.user.path())
            .output()
            .unwrap()
    }
}

fn server_json() -> ServerJson {
    ServerJson {
        v: 1,
        server: None,
        run: RUN.into(),
        mode: RunMode::Practice,
        deadline: None,
        repo: None,
        connector: ConnectorSource {
            id: "agent".into(),
            name: "Agent".into(),
            source: "builtin".into(),
        },
        hosts_file: None,
        entries: vec![EntryFacts {
            id: ENTRY.into(),
            harness_version: None,
            resumed: false,
        }],
    }
}

fn stdout_json(out: &Output) -> Value {
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn json_passes_the_three_files_through() {
    let home = Home::new();
    let st = serde_json::to_value(status(Mode::Recording)).unwrap();
    let run = serde_json::to_value(server_json()).unwrap();
    home.write("status.json", &st);
    home.write("server.json", &run);
    let got = stdout_json(&home.run(&["status", "--json"]));
    assert_eq!(got, json!({"status": st, "upload": null, "run": run}));
    let upload = json!({"state": "done", "some_key_status_does_not_know": [1, 2]});
    home.write("upload.json", &upload);
    for args in [
        &["status", "--json"][..],
        &["status", ENTRY, "--json"],
        &["status", RUN, "--json"],
    ] {
        let got = stdout_json(&home.run(args));
        assert_eq!(
            got,
            json!({"status": st, "upload": upload, "run": run}),
            "{args:?}"
        );
    }
    let plain = home.run(&["status"]);
    assert_eq!(plain.status.code(), Some(0));
    let shown = String::from_utf8(plain.stdout).unwrap();
    assert!(!shown.contains(text::FINISHED), "{shown}");
}

#[test]
fn no_run_and_unreadable_status() {
    let home = Home::new();
    for args in [&["status"][..], &["status", "--json"]] {
        assert_eq!(home.run(args).status.code(), Some(1));
    }
    home.write("server.json", &serde_json::to_value(server_json()).unwrap());
    std::fs::write(home.entry().join("status.json"), b"{not json").unwrap();
    for args in [&["status"][..], &["status", "--json"]] {
        let out = home.run(args);
        assert_eq!(out.status.code(), Some(3));
        assert!(out.stdout.is_empty());
    }
    assert_eq!(
        home.run(&["status", "e00000000000000ff"]).status.code(),
        Some(1)
    );
    assert!(Path::new(&home.entry()).is_dir());
}
