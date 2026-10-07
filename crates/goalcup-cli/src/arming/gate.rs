//! The V1 arming gate's participant-facing half (D42): compiled with `v1-compat` only. See the
//! parent module for what the gate is and for the Ctrl-C handler a local capture run shares.

use super::{GateOutcome, interrupted};
use crate::connector::Connector;
use crate::retry::retry_busy;
use goalcup_protocol::control;
use goalcup_protocol::control::DaemonClient;

// ------------------------------------------------------------------ what the gate said

/// The gate's answer and the words that go with it.
///
/// `lines` exists because the two entry points below print at different moments. In attach
/// mode the terminal is free and [`wait_foreground`] writes them at once; in launch mode
/// the harness owns the terminal for the length of the run, so the same lines are carried
/// back and printed once the TUI has let go of the screen.
#[derive(Debug, Clone)]
pub struct GateReport {
    pub outcome: GateOutcome,
    /// The account of the outcome, one line per line, with no trailing newline on any of
    /// them. Empty for an interrupted gate, which the participant is already being told
    /// about by the command that abandons the entry.
    pub lines: Vec<String>,
}

/// Attach mode: the terminal is free, so print here.
///
/// `run_public_key` is the key this run's `create_workspace` handed back and `goalcup start`
/// pinned into the entry record; [`look`] checks the status document against it before a
/// word of that document decides anything.
pub fn wait_foreground(
    client: &dyn DaemonClient,
    c: &Connector,
    entry_id: &str,
    run_public_key: &str,
    out: &mut dyn std::io::Write,
) -> GateReport {
    let report = observe(client, c, entry_id, run_public_key, Watch::Foreground);
    for line in &report.lines {
        // A gate that could not print its own account is not a reason to change the
        // account: the outcome is what the caller acts on, and the caller reports the
        // stream that closed under its own rung (`commands::start`'s `stands`).
        let _ = writeln!(out, "{line}");
    }
    report
}

/// Launch mode: the TUI owns the terminal, so nothing is printed here; the lines travel
/// back on the report and the caller prints them when the TUI has finished with the screen.
pub fn watch_beside(
    client: &dyn DaemonClient,
    c: &Connector,
    entry_id: &str,
    run_public_key: &str,
) -> GateReport {
    observe(client, c, entry_id, run_public_key, Watch::Beside)
}

/// The one gate, under both entry points.
///
/// 1. **Has this process been interrupted, in the mode where that is this command's to act
///    on?** Then there is no account to give: the command is about to unmake the entry.
///    [`Watch::Beside`] answers `false` to this question always — see the module header.
/// 2. **Has the daemon settled this gate?** Then the daemon's own reason is the answer.
/// 3. Otherwise the entry arms on its first transcript record, and there is nothing to wait
///    for.
fn observe(
    client: &dyn DaemonClient,
    c: &Connector,
    entry_id: &str,
    run_public_key: &str,
    watch: Watch,
) -> GateReport {
    if watch.interrupted() {
        return GateReport {
            outcome: GateOutcome::Interrupted,
            lines: Vec::new(),
        };
    }
    let (workspace, settled) = look(client, entry_id, run_public_key);
    if let Some(detail) = settled {
        let lines = as_lines(&refusal_text(entry_id, &workspace, &detail));
        return GateReport {
            outcome: GateOutcome::Refused(detail),
            lines,
        };
    }
    GateReport {
        outcome: GateOutcome::NotRequired,
        lines: nothing_to_wait_for(c),
    }
}

/// Which of the two moments the gate is being watched in.
///
/// [`Watch::Foreground`] is attach mode: nothing has been registered, the abandon window is
/// open, and a Ctrl-C is the participant cancelling a run that has not begun.
/// [`Watch::Beside`] is launch mode, past `register_process`: the harness owns the terminal
/// and owns the Ctrl-C typed into it, so the flag is not read here at all.
enum Watch {
    Foreground,
    Beside,
}

impl Watch {
    /// Whether a Ctrl-C ends this wait — only inside the abandon window.
    fn interrupted(&self) -> bool {
        matches!(self, Watch::Foreground) && interrupted()
    }
}

/// What the entry's signed status says: the workspace it names, and the daemon's reason
/// where its arming state is [`control::ArmingState::Refused`].
///
/// A status this call could not read is not a refusal: the entry arms on its first
/// transcript record whatever this call learns.
///
/// **Two questions before either of them is believed, and a `no` to either is "no answer".**
/// The document is checked against `run_public_key` — the key
/// `create_workspace` handed back and `goalcup start` pinned into the entry record — and its
/// `payload.entry_id` is compared with the id that was asked for. `Some(detail)` here becomes
/// [`GateOutcome::Refused`], which `commands::start::verdict` turns into an abandon, so a
/// document off the one control route that is unauthenticated must not be able to
/// unmake an entry. The id check is the half a signature cannot cover: one run key signs
/// every document of the run, so another entry's payload verifies just as well as this one's.
fn look(
    client: &dyn DaemonClient,
    entry_id: &str,
    run_public_key: &str,
) -> (String, Option<String>) {
    match retry_busy(|| client.status(entry_id)) {
        Ok(status)
            if status.verify_with(run_public_key).is_ok()
                && status.payload.entry_id == entry_id =>
        {
            let refused = status.payload.arming == control::ArmingState::Refused;
            (
                status.payload.workspace.clone(),
                refused.then(|| status.payload.detail.clone()),
            )
        }
        _ => (String::new(), None),
    }
}

/// The ordinary case, in words that say what is happening rather than what is not.
fn nothing_to_wait_for(c: &Connector) -> Vec<String> {
    vec![
        format!(
            "{} arms on its own transcript: the launch plus the first transcript record is \
             the start signal, and the tracker is already watching for it.",
            c.manifest.id
        ),
        "Recording begins with your first prompt. There is nothing here to arm by hand."
            .to_string(),
    ]
}

/// The start signal, in the daemon's words (`goalcup_daemon::arming`'s `on_screen`).
pub const START_SIGNAL_IN_WORDS: &str = "the harness's first transcript record";

/// A tested artefact, because it is the most important thing the participant reads.
///
/// `reason` is the daemon's own words for a settled gate, and the wording says the gate will
/// not arm rather than asking for more patience — a settled gate that told a participant to
/// be patient would be telling them to spend a week on an entry that can never be verified.
///
/// **The word "wait" does not appear.** A session opened before arming runs
/// blind and silent for its whole life, so the one instruction that saves the entry — do not
/// type your goal into that folder — is on the screen.
///
/// The signal is named in the daemon's own words (`goalcup_daemon::arming`'s
/// `on_screen`): the first transcript record arms every entry, whatever a manifest's
/// `arming.signal` still says.
pub fn refusal_text(entry_id: &str, workspace: &str, reason: &str) -> String {
    let signal = START_SIGNAL_IN_WORDS;
    // A status this gate could not read leaves no path to name, and an empty pair of
    // quotation marks in the middle of the sentence that matters most is worse than a
    // description of the folder.
    let workspace = match workspace.trim() {
        "" => "the workspace this entry was created in",
        named => named,
    };
    let mut said = String::new();
    said.push_str(&format!("The tracker did not start your clock: {reason}\n"));
    said.push_str(&format!(
        "Entry {entry_id} will not arm, so {signal} will not start its clock and nothing \
         in {workspace} is being recorded.\n"
    ));
    said.push_str(
        "Do not type your goal into a session in that folder: a session opened before the \
         gate armed runs blind and silent for its whole life.\n",
    );
    said.push_str("This entry is being abandoned, so `goalcup start` can simply be run again.");
    said
}

/// One `String` per line, with no trailing newline on any of them — the shape
/// [`GateReport::lines`] promises, so a caller printing them with `writeln!` cannot double
/// the blank lines.
fn as_lines(text: &str) -> Vec<String> {
    text.lines().map(str::to_string).collect()
}
