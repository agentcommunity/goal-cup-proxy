//! `goalcup status` (PRD-10 section 4): reads `status.json` (seam S4), `server.json` and, for
//! `--json` only, `upload.json`, and never talks to the recorder. It works the same while a part
//! records, after it ends and after a crash. It shows only what `status.json` holds as written,
//! with no counter of its own (ledger L2).

use std::process::ExitCode;

use serde_json::Value;

use super::{Found, ServerJson, fail, find, home, read_json, say, text};
use crate::pipeline::status::{Mode, Status};

/// The recording state, one row of the section 4 table each. Times are seconds since 1970.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Sealed {
        at: u64,
    },
    /// The recorder is gone, its watchdog lives, for up to 30 s.
    Restarting {
        since: u64,
    },
    /// The same, after 30 s.
    OffRestarting {
        since: u64,
    },
    /// Both gone, the harness lives: `goalcup recover`.
    Recover {
        since: u64,
    },
    /// Both gone, the harness too: `goalcup continue`.
    Continue {
        since: u64,
    },
    /// As `Continue`, and `status.json` is older than 2 minutes: the computer may have slept.
    Stale {
        since: u64,
    },
    /// Both gone and the run's time is up: `goalcup seal`, as `recover` and `continue` refuse.
    Over {
        since: u64,
    },
    Null {
        since: u64,
    },
    NotAnswering {
        since: u64,
    },
    Losing {
        streams: u64,
        bytes: u64,
    },
    WriterSlow {
        secs: u64,
    },
    Recording,
}

impl State {
    /// The state line starts with `PROBLEM:`: the participant must act or report.
    pub fn is_problem(&self) -> bool {
        matches!(
            self,
            State::Recover { .. }
                | State::Continue { .. }
                | State::Stale { .. }
                | State::Over { .. }
                | State::Null { .. }
        )
    }

    /// The word for the part this state is about, in the head and the parts line.
    pub fn word(&self) -> &'static str {
        match self {
            State::Sealed { .. } => "finished",
            State::Recording | State::Losing { .. } | State::WriterSlow { .. } => "recording",
            State::Restarting { .. } => "recording is restarting",
            State::NotAnswering { .. } => "recording is not answering",
            _ => "recording is off",
        }
    }
}

/// The recording state, in the section 4 order.
pub fn decide(st: &Status, cx: &Ctx<'_>) -> State {
    let updated = super::unix_of(&st.updated).unwrap_or(0);
    let age = cx.now.saturating_sub(updated);
    let since = match st.mode {
        Mode::Null => at(st.off.since.as_deref()).unwrap_or(updated),
        _ => updated,
    };
    let over = cx.deadline.is_some_and(|d| d <= cx.now);
    match st.mode {
        Mode::Sealed => State::Sealed { at: updated },
        _ if !cx.recorder && cx.watchdog && age <= 30 => State::Restarting { since },
        _ if !cx.recorder && cx.watchdog => State::OffRestarting { since },
        _ if !cx.recorder && over => State::Over { since },
        _ if !cx.recorder && cx.harness => State::Recover { since },
        _ if !cx.recorder && age > 120 => State::Stale { since },
        _ if !cx.recorder => State::Continue { since },
        Mode::Null => State::Null { since },
        _ if age > 10 => State::NotAnswering { since },
        Mode::Losing => State::Losing {
            streams: st.losing.streams_in_gap,
            bytes: st.losing.dropped_bytes,
        },
        Mode::WriterSlow => State::WriterSlow {
            secs: st.writer_idle_ms / 1000,
        },
        Mode::Recording => State::Recording,
    }
}

/// What the state and the block need beside `status.json`: names, liveness and times (seconds
/// since 1970).
#[derive(Clone, Debug)]
pub struct Ctx<'a> {
    /// The harness name from the connector.
    pub name: &'a str,
    pub run: &'a str,
    pub recorder: bool,
    pub watchdog: bool,
    pub harness: bool,
    /// The run's deadline, when there is one.
    pub deadline: Option<u64>,
    pub now: u64,
}

fn at(t: Option<&str>) -> Option<u64> {
    t.and_then(super::unix_of)
}

fn note_str<'a>(n: &'a Value, key: &str) -> Option<&'a str> {
    n.get(key).and_then(Value::as_str)
}

/// One problem or notice line per cause (section 4 table), from the facts `status.json` holds.
pub fn problems(st: &Status, state: &State, cx: &Ctx<'_>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |line: String| {
        if !out.contains(&line) {
            out.push(line);
        }
    };
    if state.is_problem() {
        add(text::state_line(state, cx.run));
    }
    let w = &st.written;
    let started = st.harness.as_ref().and_then(|h| super::unix_of(&h.started));
    if cx.harness && st.mode != Mode::Sealed {
        let (req, native) = (
            at(st.last_request_at.as_deref()),
            at(st.last_native_at.as_deref()),
        );
        match (req, native) {
            (Some(r), Some(n)) if cx.now.saturating_sub(r) >= 600 && n > r => {
                add(text::quiet_proxy(cx.name))
            }
            (None, _) if w.model_requests == 0 && started.is_some_and(|s| cx.now >= s + 300) => {
                add(text::no_request_yet())
            }
            _ => {}
        }
    }
    for n in &st.notes {
        let host = note_str(n, "host");
        let line = match note_str(n, "what") {
            Some("native_missing") => {
                text::native_missing(note_str(n, "row").unwrap_or("session"), cx.name)
            }
            Some("native_other_device") => {
                text::native_other_device(note_str(n, "path").unwrap_or("a folder"), cx.name)
            }
            Some("long_tunnel") => {
                let num = |k: &str| n.get(k).and_then(Value::as_u64);
                let minutes =
                    (note_str(n, "why") == Some("stream")).then(|| num("secs").unwrap_or(0) / 60);
                text::long_tunnel(host.unwrap_or("a host"), minutes, num("up").unwrap_or(0))
            }
            Some("ca_refused") => text::listed_tunnel(host.unwrap_or("a host"), "tls_refused"),
            Some("parse_stopped") => {
                text::parse_stopped(host, note_str(n, "at").and_then(super::unix_of))
            }
            Some("local_model") => text::local_model(host.unwrap_or("a local address")),
            Some("fd_limit") => text::fd_limit(n.get("limit").and_then(Value::as_u64)),
            Some(what) if !goalcup_format::NOTES.contains(&what) => text::unknown_note(what),
            _ => continue,
        };
        add(line);
    }
    for (host, t) in &w.counts.tunnel_hosts {
        if let Some(reason) = &t.listed {
            add(text::listed_tunnel(host, reason));
        }
    }
    if w.counts.mask_failclosed_bytes > 0 {
        add(text::fail_closed(w.counts.mask_failclosed_bytes));
    }
    if let Some(a) = st.after_seal.as_ref().filter(|a| a.bytes > 0) {
        add(text::after_seal(cx.name, a.bytes, &a.hosts));
    }
    let exit = st.harness.as_ref().and_then(|h| h.exit);
    let open = cx.deadline.filter(|d| *d > cx.now);
    if let (Some(e), Some(d), false) = (exit, open, state.is_problem()) {
        add(text::agent_exited(
            cx.name,
            e,
            &text::end_words(d, cx.now),
            cx.run,
        ));
    }
    out
}

/// Hosts by count, largest first.
pub fn by_count(hosts: &std::collections::BTreeMap<String, u64>) -> Vec<(&str, u64)> {
    let mut v: Vec<(&str, u64)> = hosts.iter().map(|(h, n)| (h.as_str(), *n)).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    v
}

/// The status block (section 4), the same shape in every state. `parts`: the word for each part.
pub fn block(
    st: &Status,
    state: &State,
    run: Option<&ServerJson>,
    cx: &Ctx<'_>,
    parts: &[&str],
) -> String {
    let w = &st.written;
    let h = st.harness.as_ref();
    let recording = if state.is_problem() {
        "Recording: off.".to_owned()
    } else {
        text::state_line(state, cx.run)
    };
    let mut lines = vec![text::status_head(run.map(|r| r.mode), cx.name, state)];
    if let Some(d) = cx.deadline {
        lines.push(text::ends_line(d, cx.now));
    }
    lines.push(text::harness_line(
        cx.name,
        cx.harness,
        h.and_then(|h| h.exit),
        h.is_some(),
    ));
    lines.push(recording);
    lines.push(text::written_line(
        w.model_requests,
        &by_count(&w.counts.hosts),
        w.counts.req_whole,
        w.counts.req_holes,
    ));
    lines.push(text::session_files_line(
        w.counts.native_files,
        w.counts.native_bytes,
    ));
    lines.push(text::off_line(st.off.spans, st.off.ms));
    lines.push(text::parts_line(parts));
    let problems = problems(st, state, cx);
    lines.push(text::problems_head(problems.len()));
    let mut out = String::new();
    for (i, l) in lines.iter().enumerate() {
        out += if i == 0 { "" } else { "  " };
        out += l;
        out.push('\n');
    }
    for p in problems {
        out += &format!("    {p}\n");
    }
    out
}

/// `goalcup status [PART] [--json]`: 0 shown, 1 no run yet, 3 `status.json` unreadable.
pub fn run(part: Option<String>, json: bool) -> ExitCode {
    let home = match home() {
        Ok(h) => h,
        Err(t) => return fail(t),
    };
    let Some(found) = find(&home, part.as_deref()) else {
        let line = part.map_or_else(|| text::NO_RUN_YET.to_owned(), |p| text::no_such_part(&p));
        return fail(&line);
    };
    let dir = found.entry_dir();
    let path = super::status_json(&dir);
    if json {
        let Some(status) = read_json::<Value>(&path) else {
            say(&text::status_unreadable(&path));
            return ExitCode::from(3);
        };
        let run: Option<Value> = read_json(&super::server_json(&found.run_dir));
        let upload: Option<Value> = read_json(&super::upload_json(&dir));
        let all = serde_json::json!({ "status": status, "upload": upload, "run": run });
        println!("{all}");
        return ExitCode::SUCCESS;
    }
    let Some(st) = read_json::<Status>(&path) else {
        say(&text::status_unreadable(&path));
        return ExitCode::from(3);
    };
    print!("{}", show(&found, &st, super::now()));
    ExitCode::SUCCESS
}

fn show(found: &Found, st: &Status, now: u64) -> String {
    let run = found.server.as_ref();
    let deadline = run
        .and_then(|r| r.deadline.as_deref())
        .or(st.deadline.as_deref());
    let cx = Ctx {
        name: run.map_or("your agent", |r| r.connector.name.as_str()),
        run: &found.run,
        recorder: super::alive(st.pid, st.start_id),
        watchdog: super::watchdog_alive(&found.entry_dir()),
        harness: st
            .harness
            .as_ref()
            .is_some_and(|h| super::alive(h.pid, h.start_id)),
        deadline: at(deadline),
        now,
    };
    let state = decide(st, &cx);
    let parts: Vec<&str> = found
        .entries
        .iter()
        .map(|e| {
            let st: Option<Status> = read_json(&super::status_json(&found.run_dir.join(e)));
            if st.is_some_and(|s| s.mode == Mode::Sealed) {
                "sealed"
            } else if *e == found.entry {
                state.word()
            } else {
                "not sealed"
            }
        })
        .collect();
    block(st, &state, run, &cx, &parts)
}
