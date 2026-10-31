//! The end screen (PRD-10 section 5): what a sealed part wrote, what it lost, how long recording
//! was off and what is still on this computer. It prints only what the sealed `status.json` holds
//! (its `written` and `off` equal the seal's totals) and adds no counter (ledger L2), so the screen
//! and `goalcup-verify check` agree by construction. The upload line comes from `upload.json`
//! (seam S9).

use std::path::Path;

use super::status::by_count;
use super::{ServerJson, text};
use crate::pipeline::status::{Mode, Status};
use crate::upload::state::{self as up, UploadJson};
use crate::watchdog::SealReason;

/// The part is not sealed (or its `status.json` cannot be read): there is no end screen yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotSealed;

/// How many unlisted tunnel hosts are named on screen (rule 4).
const SHOWN_HOSTS: usize = 5;

/// The end screen of the part in `entry_dir`, from its `status.json`, `upload.json` and its run's
/// `server.json`.
pub fn end_screen(entry_dir: &Path) -> Result<String, NotSealed> {
    let st: Status = super::read_json(&super::status_json(entry_dir)).ok_or(NotSealed)?;
    let run_dir = entry_dir.parent().ok_or(NotSealed)?;
    let server: Option<ServerJson> = super::read_json(&super::server_json(run_dir));
    let run = run_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    screen(
        &st,
        server.as_ref(),
        up::read(entry_dir).as_ref(),
        &run,
        &super::goalcup_log(entry_dir),
        super::now(),
    )
}

/// The end screen as a function of the sealed `status.json`, `server.json`, `upload.json`, the run
/// id, the `goalcup.log` path and now (seconds since 1970). The one thing it reads besides those is
/// process liveness (`super::alive(st.pid, st.start_id)`): a live recorder still sends the wrap-up,
/// so the upload line names `goalcup upload` only when that process is gone. Whoever seals a dead
/// recorder's entry (`close_dead_entry`) must write that recorder's `pid` and `start_id`, not the
/// caller's own, or this screen would see it alive and drop the line.
pub fn screen(
    st: &Status,
    server: Option<&ServerJson>,
    upload: Option<&UploadJson>,
    run: &str,
    log: &Path,
    now: u64,
) -> Result<String, NotSealed> {
    let seal = st
        .seal
        .as_ref()
        .filter(|_| st.mode == Mode::Sealed)
        .ok_or(NotSealed)?;
    let name = server.map_or("your agent", |s| s.connector.name.as_str());
    let c = &st.written.counts;
    let mut head = Vec::new();
    if let Some(exit) = st.harness.as_ref().and_then(|h| h.exit) {
        head.push(text::has_exited(name, exit));
    }
    let deadline = server
        .and_then(|s| s.deadline.as_deref())
        .and_then(super::unix_of);
    if let Some(d) = deadline.filter(|d| *d > now) {
        head.push(text::still_open(&text::end_words(d, now)));
        head.push(text::continue_now(run));
    }
    if seal.reason == SealReason::Recovered {
        head.push(text::RECOVERED.to_owned());
    }
    // Rule 1 by construction: every line that names a loss goes through `loss`, and "Nothing was
    // lost." is the second line only when none did.
    let mut body = vec![match st.written.model_requests {
        0 => text::no_request_written(name),
        n => text::requests_written(n, &by_count(&c.hosts)),
    }];
    let mut lost = false;
    let mut loss = |body: &mut Vec<String>, line: String| {
        lost = true;
        body.push(line);
    };
    if c.req_holes > 0 || c.lost_bytes > 0 {
        loss(
            &mut body,
            text::whole_and_lost(c.req_whole, c.req_holes, c.lost_bytes),
        );
    }
    // Gaps no other line names: of unknown size beyond the off spans, or, with no other loss
    // line, gaps that lost no byte. `recorder_down` gaps are the off line's.
    let unknown = c.lost_unknown.saturating_sub(st.off.spans);
    let listed = c.tunnel_hosts.values().any(|t| t.listed.is_some());
    if unknown > 0 {
        loss(&mut body, text::gaps_line(unknown, true));
    } else if c.gaps > st.off.spans && c.lost_bytes == 0 && !listed {
        loss(&mut body, text::gaps_line(c.gaps - st.off.spans, false));
    }
    body.push(text::session_files(c.native_files, c.native_bytes));
    if st.off.spans > 0 || st.off.ms > 0 {
        loss(&mut body, text::off_total(st.off.spans, st.off.ms));
    }
    let mut unlisted = Vec::new();
    for (host, t) in &c.tunnel_hosts {
        match &t.listed {
            Some(reason) => loss(
                &mut body,
                text::tunnel_listed(host, reason, t.conns, t.up + t.down),
            ),
            None => unlisted.push((host.as_str(), t.conns, t.up + t.down)),
        }
    }
    unlisted.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(b.0)));
    if !unlisted.is_empty() {
        let more = unlisted.len().saturating_sub(SHOWN_HOSTS);
        unlisted.truncate(SHOWN_HOSTS);
        body.push(text::tunnels_unlisted(&unlisted, more));
    }
    if c.mask_failclosed_bytes > 0 {
        loss(&mut body, text::fail_closed(c.mask_failclosed_bytes));
    }
    if let Some(f) = st.cut_files.filter(|f| f.files > 0 || f.bytes > 0) {
        loss(&mut body, text::cut_files(f.files, f.bytes));
    }
    if let Some(a) = st.after_seal.as_ref().filter(|a| a.bytes > 0) {
        loss(&mut body, text::after_seal(name, a.bytes, &a.hosts));
    }
    if let Some(o) = st.outlived.as_ref().filter(|o| o.count > 0) {
        body.push(text::outlived(name, o.count, &o.programs));
    }
    if !lost {
        body.insert(1, text::NOTHING_LOST.to_owned());
    }
    let mut out = String::new();
    for l in &head {
        out += &format!("{l}\n");
    }
    if !head.is_empty() {
        out.push('\n');
    }
    out += &format!("{}\n", text::PART_RECORDED);
    for l in &body {
        out += &format!("  {l}\n");
    }
    // The wrap-up sends while the recorder lives: only then is the end screen not the last word.
    let end = !super::alive(st.pid, st.start_id);
    if let Some(u) = upload {
        out += &format!("{}\n", text::upload_line(u, now, end));
    }
    out += &format!("{}\n", text::details_at(log));
    Ok(out)
}

// What `start` prints once `watchdog::run` has returned (PRD-10 section 3 step 10).

/// How long a sealed `status.json` is waited for after the watchdog returns, in 100 ms steps.
const SEALED_WAIT: u32 = 30;

/// The longest the front shows the wrap-up: the recorder's own bound is 60 minutes (PRD-06 section 9).
const WRAP_UP_MAX: std::time::Duration = std::time::Duration::from_secs(61 * 60);

/// What `start` prints when the watchdog has returned: the end screen of the sealed part, with a
/// line first when the harness never started, or the start refusal when nothing was recorded at
/// all. `refused` is the watchdog's mask refusal, which has its own text. Only the details a
/// participant can act on; no value and no byte of the record.
pub fn closing(entry_dir: &Path, name: &str, run: &str, refused: bool) -> String {
    if refused {
        return format!("{}\n", text::mask_refused());
    }
    for _ in 0..SEALED_WAIT {
        if let Ok(screen) = end_screen(entry_dir) {
            let st: Option<Status> = super::read_json(&super::status_json(entry_dir));
            let gone = st.is_some_and(|s| s.harness.is_none());
            let first = gone.then(|| format!("{name} did not start.\n\n"));
            return format!("{}{screen}", first.unwrap_or_default());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let (log, support) = (
        text::details_at(&super::goalcup_log(entry_dir)),
        text::SUPPORT_ADDRESS,
    );
    if super::status_json(entry_dir).exists() {
        return format!(
            "goalcup could not read the final counts of this part. Run goalcup status, or write \
             to {support}.\n{log}\n"
        );
    }
    format!(
        "Goal Cup could not start recording, so {name} was not started. Run the command again. \
         If it fails again, write to {support} and name run {run}.\n{log}\n"
    )
}

/// The upload progress after the end screen (PRD-06 section 9): while the recorder lives and the
/// upload is going, one `Upload:` line whenever its state or the bytes waiting change, then the
/// last word. Ctrl-C ends this wait only; the recorder finishes alone.
pub fn wrap_up(entry_dir: &Path, say: &mut dyn FnMut(&str)) {
    let recorder = || -> bool {
        let st: Option<Status> = super::read_json(&super::status_json(entry_dir));
        st.is_some_and(|s| super::alive(s.pid, s.start_id))
    };
    if !recorder() {
        return;
    }
    let started = std::time::Instant::now();
    let mut shown = None;
    while recorder() && started.elapsed() < WRAP_UP_MAX {
        if let Some(u) = up::read(entry_dir) {
            let key = (u.state, u.waiting_bytes, u.final_);
            if shown != Some(key) {
                say(&text::upload_line(&u, super::now(), false));
                shown = Some(key);
            }
            if matches!(
                u.state,
                up::State::Local
                    | up::State::Done
                    | up::State::Closed
                    | up::State::Stuck
                    | up::State::Refused
            ) {
                return;
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    // The wrap-up is over: nothing sends this part by itself now.
    if let Some(u) = up::read(entry_dir) {
        say(&text::upload_line(&u, super::now(), true));
    }
}
