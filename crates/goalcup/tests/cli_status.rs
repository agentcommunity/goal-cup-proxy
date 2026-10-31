//! `goalcup status` (PRD-10 section 4): the recording state from facts in the table's order, one
//! problem line per cause, the note lines, pid liveness, and `status --json` passing the three files
//! through. Facts and the contract word `PROBLEM:` only; every sentence is compared with `text.rs`.

mod cli_common;
mod common;

use std::process::{Command, Output};

use cli_common::{ENTRY, RUN, T0, live, part, put, server_json, status, with};
use common::home::TempHome;
use goalcup::cli::status::{State, State as S, block, decide, problems};
use goalcup::cli::{self, text};
use goalcup::pipeline::status::{Mode, Mode as M};
use serde_json::{Value, json};

/// Each row of the table from a fixture, in its order: a dead recorder with a live watchdog, both
/// dead with the harness alive (`recover`) or gone (`continue`), past the deadline (`seal`), null
/// mode with no retry count, losing. `PROBLEM:` starts exactly the rows whose line names a command,
/// that line names no other one, and only a part that records is called `recording` (point 6).
#[test]
fn state_rows_in_table_order() {
    let (e, null) = (json!({}), json!({"off": {"since": "2026-11-07T09:50:00Z"}}));
    let losing = json!({"losing": {"streams_in_gap": 2, "dropped_bytes": 12_000_000}});
    let slow = json!({"writer_idle_ms": 14_500});
    let cont = format!("goalcup continue {RUN}");
    // (mode, patch, alive: r recorder w watchdog h harness, age in s, state, the line's command)
    #[rustfmt::skip]
    let mut rows = vec![
        (M::Sealed, &e, "", 999, S::Sealed { at: T0 }, None),
        (M::Recording, &e, "h", 5, S::Recover { since: T0 }, Some("goalcup recover")),
        (M::Recording, &e, "", 60, S::Continue { since: T0 }, Some(cont.as_str())),
        (M::Recording, &e, "", 121, S::Stale { since: T0 }, Some(cont.as_str())),
        (M::Recording, &e, "", 86_400, S::Over { since: T0 }, Some("goalcup seal")),
        (M::Recording, &e, "h", 86_400, S::Over { since: T0 }, Some("goalcup seal")),
        (M::Null, &null, "rwh", 3, S::Null { since: T0 - 600 }, Some("")),
        (M::Recording, &e, "rwh", 11, S::NotAnswering { since: T0 }, None),
        (M::Losing, &losing, "rwh", 1, S::Losing { streams: 2, bytes: 12_000_000 }, None),
        (M::WriterSlow, &slow, "rwh", 1, S::WriterSlow { secs: 14 }, None),
        (M::Recording, &e, "rwh", 1, S::Recording, None),
    ];
    for mode in [M::Recording, M::Losing, M::Null] {
        rows.push((mode, &e, "wh", 5, S::Restarting { since: T0 }, None));
        rows.push((mode, &e, "wh", 31, S::OffRestarting { since: T0 }, None));
    }
    for (mode, p, alive, age, want, command) in rows {
        let has = |c: char| alive.contains(c);
        let cx = live(has('r'), has('w'), has('h'), age);
        let got = decide(&with(mode, p.clone()), &cx);
        assert_eq!(got, want, "{mode:?} {alive} {age}");
        let line = text::state_line(&got, RUN);
        assert_eq!(line.starts_with(text::PROBLEM), command.is_some(), "{line}");
        if let Some(command) = command.filter(|c| !c.is_empty()) {
            assert!(line.ends_with(command), "{line}");
            for other in ["goalcup recover", "goalcup continue", "goalcup seal"] {
                assert_eq!(line.contains(other), command.starts_with(other), "{line}");
            }
        }
        let records = matches!(want, S::Recording | S::Losing { .. } | S::WriterSlow { .. });
        assert_eq!(got.word() == S::Recording.word(), records, "{got:?}");
        match got {
            S::Losing { bytes, .. } => assert!(line.contains(&text::size_words(bytes))),
            // Null mode names no retry count: the only number beside its time is the 5-minute cap.
            S::Null { since } => {
                let rest = line.replace(&text::local_time(since, true), "");
                let digits: String = rest.chars().filter(char::is_ascii_digit).collect();
                assert_eq!(digits, "5", "{line}");
            }
            _ => {}
        }
    }
}

/// One `PROBLEM:` per cause, in the lines and in the block: two notes given twice and a listed
/// host also named by a note make two, the state line one more.
#[test]
fn one_problem_line_per_cause() {
    let tunnel = json!({"what": "long_tunnel", "host": "a.example.net", "why": "upload", "up": 3});
    let ca = json!({"what": "ca_refused", "host": "pinned.example.com"});
    let listed = json!({"conns": 3, "up": 10, "down": 20, "listed": "tls_refused"});
    let st = with(
        Mode::Recording,
        json!({"notes": [tunnel, ca, tunnel, ca],
            "written": {"tunnel_hosts": {"pinned.example.com": listed}}}),
    );
    for (harness, state) in [
        (true, State::Recording),
        (true, State::Recover { since: T0 }),
        (false, State::Continue { since: T0 }),
        (false, State::Stale { since: T0 }),
    ] {
        let cx = live(true, true, harness, 5);
        let want = 2 + usize::from(state.is_problem());
        let lines = problems(&st, &state, &cx);
        let marked = lines.iter().filter(|l| l.starts_with(text::PROBLEM));
        assert_eq!(marked.count(), want, "{lines:#?}");
        let shown = block(&st, &state, None, None, &cx, &[state.word()]);
        assert_eq!(shown.matches(text::PROBLEM).count(), want, "{shown}");
    }
}

/// Every note of section 4 gives its line, as do a listed tunnel, fail-closed bytes, bytes after
/// the seal and an exited agent; the `long_tunnel` fix names `seal` before `continue`.
#[test]
fn every_note_gives_its_line() {
    let st = with(
        Mode::Losing,
        json!({"notes": [
            {"what": "native_missing", "row": "transcript"},
            {"what": "native_other_device", "row": "sessions", "path": "data/sessions"},
            {"what": "long_tunnel", "host": "stream.example.net", "why": "stream", "secs": 720},
            {"what": "long_tunnel", "host": "up.example.net", "why": "upload", "up": 3_000_000},
            {"what": "ca_refused", "host": "refuses.example.com"},
            {"what": "parse_stopped", "conn": 7, "reason": "head", "at": "2026-11-07T09:10:00Z"},
            {"what": "local_model", "host": "127.0.0.1:11434"},
            {"what": "fd_limit", "limit": 256},
            {"what": "brand_new_note"}],
        "written": {"mask_failclosed_bytes": 2_000_000, "tunnel_hosts": {"proxy.example.com":
            {"conns": 1, "up": 1, "down": 1, "listed": "upstream_untrusted"}}},
        "after_seal": {"bytes": 2_000_000, "hosts": ["api.example.com"]},
        "harness":
            {"pid": 1, "started": "2026-11-07T08:00:00Z", "start_id": 1, "exit": {"code": 0}}}),
    );
    let lines = problems(&st, &State::Recording, &live(true, true, true, 0));
    let exit = goalcup::watchdog::Exit::Code(0);
    for w in [
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
        text::after_seal("Agent", 2_000_000, &["api.example.com".into()]),
        text::agent_exited("Agent", exit, &text::end_words(T0 + 86_400, T0), RUN),
    ] {
        assert!(lines.contains(&w), "missing {w}\nin {lines:#?}");
    }
    let tunnel = text::long_tunnel("up.example.net", None, 1);
    let at = |cmd: &str| tunnel.find(cmd).unwrap();
    assert!(at("goalcup seal") < at("goalcup continue"), "{tunnel}");
}

/// The two silence lines, each only past its time and only while the harness runs.
#[test]
fn request_silence_lines() {
    let harness = json!({"pid": 1, "started": "2026-11-07T10:00:00Z", "start_id": 1});
    let none = with(Mode::Recording, json!({"harness": harness}));
    let quiet = with(
        Mode::Recording,
        json!({"harness": harness, "written": {"model_requests": 4},
            "last_request_at": "2026-11-07T10:00:00Z", "last_native_at": "2026-11-07T10:09:00Z"}),
    );
    let (no_req, quiet_line) = (text::no_request_yet(), text::quiet_proxy("Agent"));
    for (st, alive, age, line, shown) in [
        (&none, true, 200, &no_req, false),
        (&none, true, 301, &no_req, true),
        (&quiet, true, 599, &quiet_line, false),
        (&quiet, true, 601, &quiet_line, true),
        (&quiet, false, 601, &quiet_line, false),
    ] {
        let lines = problems(st, &State::Recording, &live(true, true, alive, age));
        assert_eq!(lines.contains(line), shown, "{line} {alive} {age}");
    }
}

/// A pid is alive by `kill(pid, 0)` and its start id alone, never by its program file, which
/// PRD-02's not-dumpable recorder and watchdog close to the participant.
#[test]
fn liveness() {
    let me = std::process::id();
    let start = cli::proc_start_id(me).expect("own start id");
    let mut child = Command::new("true").spawn().unwrap();
    let dead = child.id();
    child.wait().unwrap();
    assert!(cli::alive(me, start) && !cli::alive(me, start + 1));
    assert!(!cli::alive(dead, start) && !cli::alive(0, 0));
    let dir = tempfile::tempdir().unwrap();
    assert!(!cli::watchdog_alive(dir.path()));
    let pid_file = dir.path().join("rec/watchdog.pid");
    std::fs::create_dir(dir.path().join("rec")).unwrap();
    for (id, alive) in [(start, true), (start + 1, false)] {
        std::fs::write(&pid_file, format!("{me} {id}\n")).unwrap();
        assert_eq!(cli::watchdog_alive(dir.path()), alive);
    }
    std::fs::write(&pid_file, format!("{me}")).unwrap();
    assert_eq!(cli::watchdog_pid(dir.path()), None);
}

fn goalcup(home: &TempHome, args: &[&str]) -> Output {
    let mut cmd = Command::new(common::bin());
    home.apply(&mut cmd).args(args).env("GOALCUP_SERVER", "off");
    cmd.output().unwrap()
}

fn json_out(out: Output) -> Value {
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{err}");
    serde_json::from_slice(&out.stdout).unwrap()
}

/// Exit 1 with no run or an unknown part, 3 with an unreadable `status.json`; `--json` passes the
/// three files through whole for every way of naming the part; a recording part never reads as
/// finished.
#[test]
fn status_command_reads_the_files() {
    let home = TempHome::new();
    let part = part(&home.goalcup_home());
    for args in [&["status"][..], &["status", "--json"]] {
        assert_eq!(goalcup(&home, args).status.code(), Some(1));
    }
    let run = server_json();
    put(part.parent().unwrap(), "server.json", &run);
    std::fs::create_dir_all(&part).unwrap();
    std::fs::write(part.join("status.json"), b"{not json").unwrap();
    for args in [&["status"][..], &["status", "--json"]] {
        let out = goalcup(&home, args);
        assert_eq!(out.status.code(), Some(3));
        assert!(out.stdout.is_empty());
    }
    let unknown = goalcup(&home, &["status", "e00000000000000ff"]);
    assert_eq!(unknown.status.code(), Some(1));
    assert!(part.is_dir());
    let st = serde_json::to_value(status(Mode::Recording)).unwrap();
    put(&part, "status.json", &st);
    let got = json_out(goalcup(&home, &["status", "--json"]));
    assert_eq!(got, json!({"status": st, "upload": null, "run": run}));
    let upload = json!({"state": "done", "some_key_status_does_not_know": [1, 2]});
    put(&part, "upload.json", &upload);
    for name in [None, Some(ENTRY), Some(RUN)] {
        let args: Vec<&str> = ["status"]
            .into_iter()
            .chain(name)
            .chain(["--json"])
            .collect();
        let got = json_out(goalcup(&home, &args));
        let want = json!({"status": st, "upload": upload, "run": run});
        assert_eq!(got, want, "{args:?}");
    }
    let plain = goalcup(&home, &["status"]);
    assert_eq!(plain.status.code(), Some(0));
    assert!(!String::from_utf8_lossy(&plain.stdout).contains(text::FINISHED));
}
