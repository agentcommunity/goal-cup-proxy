//! What `goalcup status` says of a run: where it is, its problems, and what to do next.

use super::*;
use crate::account::device::RunMode;

/// The heading above the technical block `goalcup status` keeps below its participant block.
pub const STATUS_DETAILS: &str = "Details:";

/// The prefix of every problem line. Scripts and the guide read it.
pub const PROBLEM: &str = "PROBLEM:";

/// How far a run has come.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// The agent runs and the run records.
    Recording,
    /// The agent has exited; the run is still open.
    AgentExited,
    /// The run's time is up and goalcup is finishing it.
    Finishing,
    /// The run is over and its host finished, but its `seal.json` was never written.
    NotSealed,
    /// The run is sealed; part of it is not uploaded.
    Sealed,
    /// The run is sealed and all of it is uploaded.
    Uploaded,
}

/// What the agent's process is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Agent<'a> {
    Running,
    /// It ended by itself; `how` is `exit code 1`.
    Exited {
        how: &'a str,
    },
    /// goalcup ended it because the run is over.
    Stopped,
    /// goalcup ended it because the disk is almost full.
    StoppedForDisk,
}

/// How the upload stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadNow {
    /// The run is recorded on this computer only.
    NotUsed,
    /// Nothing has been sent yet.
    NotYet,
    /// It works and every record written is acknowledged; `ago` is the seconds since the last
    /// send.
    UpToDate { ago: Option<u64> },
    /// It works, and records wait for the next timed send; `waiting` is how many (`None`: not
    /// known), `ago` the seconds since the last send.
    Sending {
        ago: Option<u64>,
        waiting: Option<u64>,
    },
    /// It fails now; goalcup keeps trying.
    Retrying { ago: Option<u64> },
    /// goalcup gave up on it.
    GaveUp,
    /// The run is sealed and not all of it is sent.
    Unfinished,
    /// All of it is sent.
    Done,
}

/// The facts of the first lines of `goalcup status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusHeader<'a> {
    /// The harness's display name.
    pub harness: &'a str,
    /// The run's mode; `None` for a run recorded on this computer only.
    pub mode: Option<RunMode>,
    /// `None`: this computer has no state for the run yet.
    pub phase: Option<Phase>,
    /// The run's end ([`ends_line`]), for a run on a server.
    pub ends: Option<String>,
    pub agent: Agent<'a>,
    pub upload: UploadNow,
    /// How many problem lines follow.
    pub problems: usize,
    /// Nothing has written the run's state for a while: the state below may be old.
    pub stale: bool,
    /// The run ended before its time (a newer run replaced it, or the server closed it).
    pub early: bool,
    /// With no state: the run started a moment ago, so its state is still to come.
    pub young: bool,
}

/// How a run's end is said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    /// The run is open; the time left is said.
    Open,
    /// Its time is up.
    Over,
    /// It was closed before its time.
    Early,
}

/// The line that says when the run ends (`Ends: Sat 14 Nov, 10:12 CET (09:12 UTC), 95 h 10 min
/// left.`), or when it ended. `left` is what is left, for a run that is open.
pub fn ends_line(deadline_at: &str, left: Option<u64>, end: End) -> String {
    match end {
        End::Open => format!("Ends: {}.", end_words(deadline_at, left)),
        End::Over => format!("Ended: {}.", end_words(deadline_at, None)),
        End::Early => "Ended early: a newer run replaced it, or the server closed it.".to_string(),
    }
}

fn run_kind(mode: Option<RunMode>) -> &'static str {
    match mode {
        Some(RunMode::Practice) => "practice run",
        Some(RunMode::Competition) => "competition run",
        Some(RunMode::Test) => "organizer test run",
        None => "run (recorded on this computer only)",
    }
}

/// The first lines of `goalcup status`: the run, its state, its end, the agent, the upload and the
/// number of problems.
pub fn status_header(h: &StatusHeader<'_>) -> Vec<String> {
    let name = h.harness;
    let state = match (h.phase, h.mode) {
        (None, _) if h.young => "no run state yet".to_string(),
        (None, _) => "no run state, so it did not start".to_string(),
        (Some(Phase::Recording), _) if h.stale => "state not updated lately".to_string(),
        (Some(Phase::Recording), _) => "recording".to_string(),
        (Some(Phase::AgentExited), _) => format!("{name} has exited, the run is still open"),
        (Some(Phase::Finishing), _) if h.stale && h.early => {
            "the run has ended, goalcup stopped before it finished the run".to_string()
        }
        (Some(Phase::Finishing), _) if h.stale => {
            "time is up, goalcup stopped before it finished the run".to_string()
        }
        (Some(Phase::Finishing), _) if h.early => {
            "the run has ended, goalcup is finishing the run".to_string()
        }
        (Some(Phase::Finishing), _) => "time is up, goalcup is finishing the run".to_string(),
        (Some(Phase::NotSealed), _) => "finished, but not sealed".to_string(),
        (Some(Phase::Sealed), None) => "sealed on this computer".to_string(),
        (Some(Phase::Sealed), Some(_)) => "sealed, not all of it is uploaded yet".to_string(),
        (Some(Phase::Uploaded), _) => "sealed and uploaded".to_string(),
    };
    let mut lines = vec![format!("Your {} with {name}: {state}.", run_kind(h.mode))];
    if let Some(ends) = &h.ends {
        lines.push(format!("  {ends}"));
    }
    if h.phase.is_none() {
        lines.push(if h.young {
            "  goalcup writes the run state a few seconds after the run starts.".to_string()
        } else {
            "  goalcup wrote no state for this run, so the start did not finish.".to_string()
        });
        return lines;
    }
    let state_old = h.stale && matches!(h.phase, Some(Phase::Recording | Phase::Finishing));
    lines.push(match &h.agent {
        _ if state_old => format!("  {name}: not known, the state is not being updated."),
        Agent::Running => format!("  {name}: running."),
        Agent::Exited { how } => format!("  {name}: exited ({how})."),
        Agent::Stopped => format!("  {name}: stopped."),
        Agent::StoppedForDisk => format!("  {name}: stopped (the disk is almost full)."),
    });
    lines.push(format!(
        "  Upload: {}.",
        match h.upload {
            UploadNow::NotUsed => "not used, this run stays on this computer".to_string(),
            UploadNow::NotYet => "nothing sent yet".to_string(),
            UploadNow::UpToDate { ago: Some(ago) } =>
                format!("up to date (last sent {} ago)", crate::run::age_words(ago)),
            UploadNow::UpToDate { ago: None } => "up to date".to_string(),
            UploadNow::Sending { ago, waiting } => sending_words(ago, waiting),
            UploadNow::Retrying { ago: Some(ago) } => format!(
                "not up to date, goalcup keeps trying (last sent {} ago)",
                crate::run::age_words(ago)
            ),
            UploadNow::Retrying { ago: None } => "not up to date, goalcup keeps trying".to_string(),
            UploadNow::GaveUp => "stopped, see the problem below".to_string(),
            UploadNow::Unfinished =>
                "not finished, part of the run is only on this computer".to_string(),
            UploadNow::Done => "done".to_string(),
        }
    ));
    lines.push(match h.problems {
        0 => "  Problems: none.".to_string(),
        1 => "  Problems: 1, below.".to_string(),
        n => format!("  Problems: {n}, below."),
    });
    lines
}

/// The upload while records wait for the next timed send: `working, sent every 30 minutes
/// (last sent 16 min ago; 3201 records wait for the next send)`.
fn sending_words(ago: Option<u64>, waiting: Option<u64>) -> String {
    let every = crate::upload::uploader::Policy::default()
        .interval
        .as_secs()
        / 60;
    let mut said = Vec::new();
    if let Some(ago) = ago {
        said.push(format!("last sent {} ago", crate::run::age_words(ago)));
    }
    match waiting {
        Some(1) => said.push("1 record waits for the next send".to_string()),
        Some(n) => said.push(format!("{n} records wait for the next send")),
        None => {}
    }
    let mut text = format!("working, sent every {every} minutes");
    if !said.is_empty() {
        text.push_str(&format!(" ({})", said.join("; ")));
    }
    text
}

/// What `goalcup status` tells the participant to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next<'a> {
    /// The run records and nothing is wrong.
    NothingEnds,
    /// The run is done, or nothing can be done for it.
    Nothing,
    /// The agent can be started again.
    Continue { run_id: &'a str },
    /// Part of the run is not uploaded.
    Upload,
    /// The problem lines say what to do; the run goes on meanwhile.
    Problems,
    /// goalcup is still at work.
    Wait,
    /// goalcup stopped before it ended the run: seal it, then send it (`upload`: a run on a
    /// server).
    Seal { upload: bool },
    /// The run never started; start a new one.
    Restart {
        connector_id: &'a str,
        mode: Option<RunMode>,
    },
}

/// The last line of the participant block of `goalcup status`.
pub fn status_next(next: &Next<'_>) -> String {
    match next {
        Next::NothingEnds => "Next: nothing to do. The run ends by itself.".to_string(),
        Next::Nothing => "Next: nothing to do.".to_string(),
        // A command ends its line with no full stop: it is copied as it stands.
        Next::Continue { run_id } => format!("Next: run {CONTINUE_HINT} {run_id}"),
        Next::Upload => "Next: run goalcup upload".to_string(),
        Next::Seal { upload: true } => "Next: run goalcup seal, then goalcup upload".to_string(),
        Next::Seal { upload: false } => "Next: run goalcup seal".to_string(),
        Next::Restart { connector_id, mode } => {
            let flag = match mode {
                Some(RunMode::Practice) => " --practice",
                Some(RunMode::Test) => " --test",
                Some(RunMode::Competition) | None => "",
            };
            format!(
                "Next: this run did not start. Run goalcup start --harness {connector_id}{flag}"
            )
        }
        Next::Problems => {
            "Next: do what the problem lines above say. The run goes on meanwhile.".to_string()
        }
        Next::Wait => "Next: wait a minute, then run goalcup status again.".to_string(),
    }
}

/// `goalcup status` on a computer that has not started a run. The command comes first; the phrase
/// "no run has been started from this machine" is read by tests.
pub fn no_run_yet() -> String {
    "Run goalcup setup, then goalcup start --harness <id> --practice: no run has been started from \
     this machine yet. Nothing to show."
        .to_string()
}

/// What a problem line says about restarting the agent: the fix, or the fact that there is none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Restart<'a> {
    /// `goalcup continue` can restart it.
    Continue { run_id: &'a str },
    /// The run is over.
    Over,
    /// The run is open but cannot be continued.
    Cannot,
}

/// The run's facts that a problem line names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Who<'a> {
    pub harness: &'a str,
    pub run_id: &'a str,
    /// The run's end ([`end_words`]), when it has one.
    pub until: Option<&'a str>,
    /// The harness's section of the setup guide (its connector's `participant.setup_section`).
    pub guide: &'a str,
}

/// goalcup has not written the run's state for `silent` (words).
pub fn problem_state_stale(silent: &str, restart: Restart<'_>) -> String {
    let fix = match restart {
        Restart::Continue { run_id } => format!("To go on, run: {CONTINUE_HINT} {run_id}"),
        // What finishes it (a seal, an upload, or nothing) is the Next line's to say: only status
        // knows whether this part is sealed already.
        Restart::Over => {
            "The run is over, so it cannot be continued. Do what the Next line says.".to_string()
        }
        Restart::Cannot => "This run cannot be continued.".to_string(),
    };
    format!(
        "goalcup has not updated this run for {silent}. The computer may have slept or restarted. \
         {fix}"
    )
}

pub fn problem_recording_gaps() -> String {
    format!(
        "the recording has gaps: part of what happened was not recorded. Keep the run going. If \
         this keeps happening, write to {SUPPORT_ADDRESS}."
    )
}

pub fn problem_recording_failed(run_id: &str) -> String {
    format!("the recording failed. Write to {SUPPORT_ADDRESS} and name run {run_id}.")
}

/// What a network problem line adds: the network, or a firewall holding goalcup's connections.
pub const CHECK_NETWORK: &str =
    "Check the network: it may be down, or a firewall such as Little Snitch is holding goalcup.";

/// The upload is behind. `over`: the run is sealed or the upload gave up, so the fix is the
/// command; otherwise goalcup is still trying.
pub fn problem_upload(ago: &str, why: &str, over: bool) -> String {
    if over {
        format!(
            "part of the run is not uploaded (last upload {ago} ago: {why}). Nothing is lost; it \
             waits on this computer. {CHECK_NETWORK} Then run: goalcup upload"
        )
    } else {
        format!(
            "nothing was uploaded for {ago} ({why}). Nothing is lost; it waits on this computer. \
             {CHECK_NETWORK}"
        )
    }
}

/// The server keeps refusing the run's calls with `status` (D64): a server-side fault the
/// organisers fix. The run records on and asks again by itself; the participant does nothing.
pub fn problem_server_refuses(status: u16) -> String {
    format!(
        "the server refuses this run's uploads (HTTP {status}). Recording continues on this \
         computer and nothing is lost; it is sent once the server takes it again. Nothing to do."
    )
}

/// Heartbeats keep failing; `silent` is how long the heartbeats have been failing, in words ([`crate::run::age_words`]).
pub fn problem_heartbeats(silent: &str, why: &str) -> String {
    format!("the server has not heard from this run for about {silent} ({why}). {CHECK_NETWORK}")
}

/// The one `goalcup status` line about this computer: free disk, what goalcup's own processes
/// use (`vitals`), and whether goalcup paused the agent for the disk. `None` when nothing of it
/// is known.
pub fn this_computer(vitals: &crate::upload::api::HeartbeatVitals) -> Option<String> {
    let human = crate::run::reserve::human;
    let mut parts = Vec::new();
    if let Some(free) = vitals.disk_free_bytes {
        parts.push(format!("{} of disk free", human(free)));
    }
    match (vitals.rss_bytes, vitals.cpu_ms) {
        (Some(rss), Some(cpu_ms)) => parts.push(format!(
            "goalcup uses {} of memory, {} of CPU time so far",
            human(rss),
            crate::run::age_words(cpu_ms / 1000)
        )),
        (Some(rss), None) => parts.push(format!("goalcup uses {} of memory", human(rss))),
        (None, Some(cpu_ms)) => parts.push(format!(
            "goalcup used {} of CPU time so far",
            crate::run::age_words(cpu_ms / 1000)
        )),
        (None, None) => {}
    }
    if vitals.agent_paused.is_some() {
        parts.push("agent paused for disk".to_string());
    }
    (!parts.is_empty()).then(|| format!("  This computer: {}.", parts.join("; ")))
}

pub fn problem_disk(free: &str) -> String {
    format!("only {free} of disk is free. Free some space {FREE_OUTSIDE}.")
}

/// The agent is paused because the disk is nearly full (D65), on the terminal and in `goalcup
/// status`. The words are the owner's; where to free space comes from the participant review
/// (A7).
pub fn problem_disk_paused() -> &'static str {
    "The disk is almost full, so your agent is paused. Free some disk space outside your run's \
     folders, and do not delete anything the agent made; it resumes by itself. If your agent \
     seems stuck afterwards, type keep going."
}

/// The paused agent was resumed because `free` is free again (D65).
pub fn disk_resumed(free: &str) -> String {
    format!("disk: {free} is free again, so your agent was resumed.")
}

/// The paused agent was resumed because the free disk space could not be read (D65): a pause
/// must not outlast what it is for.
pub fn disk_resumed_unread() -> &'static str {
    "disk: if the disk is nearly full, free some space outside your run's folders. Goal Cup \
     Proxy could not read the free space, so your agent was resumed."
}

/// The words every silent-native-leg line carries, whatever the harness and the count. Tests and
/// scripts look for them.
pub const NATIVE_SILENT_MARK: &str = "session file yet";

/// The native leg is silent: the recording proxy recorded `requests` requests to the model
/// service and `harness` has written no session file Goal Cup could record. The run goes on;
/// `guide` is the harness's section of the setup guide. Said once per start or continue (in
/// `host.log` while the harness owns the terminal) and kept by `goalcup status` until a session
/// file is recorded.
pub fn problem_native_silent(harness: &str, requests: u64, guide: &str) -> String {
    let requests = match requests {
        1 => "1 request".to_string(),
        n => format!("{n} requests"),
    };
    format!(
        "Goal Cup has recorded {requests} to the model service but no {harness} \
         {NATIVE_SILENT_MARK}. Your run is still recording the model traffic; keep the agent \
         running. Check that {harness} is the supported version and set up as the guide says: \
         {guide}"
    )
}

pub fn problem_sleep(why: &str) -> String {
    format!("this computer may go to sleep ({why}). Keep it on mains power with the lid open.")
}

/// The agent ended by itself, or the disk stopped it, and the run is not over.
pub fn problem_agent_exited(
    who: &Who<'_>,
    how: &str,
    disk_stop: bool,
    restart: Restart<'_>,
) -> String {
    let h = who.harness;
    let until = who
        .until
        .map(|until| format!(" {STILL_OPEN} {until}."))
        .unwrap_or_default();
    match restart {
        Restart::Continue { run_id } if disk_stop => format!(
            "{h} was stopped: the disk is almost full and uploads are failing.{until} Free some \
             disk space {FREE_OUTSIDE}, then run: {CONTINUE_HINT} {run_id}"
        ),
        Restart::Continue { run_id } => {
            format!("{h} has exited ({how}).{until} To go on, run: {CONTINUE_HINT} {run_id}")
        }
        Restart::Over => {
            format!("{h} has exited ({how}) and the run is over. Nothing to continue.")
        }
        Restart::Cannot => format!(
            "{h} has exited ({how}) and this run cannot be continued. If you did not expect \
             this, write to {SUPPORT_ADDRESS}."
        ),
    }
}

/// The agent finished normally and the run cannot be continued: a normal end, not a problem.
pub fn agent_finished(harness: &str, over: bool) -> String {
    if over {
        format!("{harness} finished, and the run is over.")
    } else {
        format!("{harness} finished, and this run cannot be continued.")
    }
}
