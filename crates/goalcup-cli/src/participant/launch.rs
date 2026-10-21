//! The block said just before the harness is launched.

use super::*;
use crate::account::device::RunMode;

/// The facts the launch block is made of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch<'a> {
    /// The harness's display name.
    pub harness: &'a str,
    /// The run's mode, for a run on a server; `None` for a run on this machine only.
    pub mode: Option<RunMode>,
    /// A first launch, or a `goalcup continue` and whether it picks up a session.
    pub opens: Opens,
    /// The run's end ([`end_words`]), for a run on a server.
    pub ends: Option<String>,
    /// The manifest's `participant.first_screen`.
    pub first_screen: Option<&'a str>,
}

/// How the harness opens for one part of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opens {
    /// The run's first launch.
    First,
    /// A `goalcup continue`: the harness picks up its last session.
    LastSession,
    /// A `goalcup continue` of a run where no saved session was found (the harness closed before
    /// the first message, or its home is made fresh for each part): the harness opens the normal
    /// way, with a new session.
    NewSession,
}

/// The first line of a continued run's launch block.
pub const RUN_GOES_ON: &str =
    "Your run goes on. No approval is needed: this computer kept the approval for this run.";

/// The block printed just before a chat-mode harness takes the terminal. Its last line starts
/// with [`READY_MARK`].
pub fn launch_block(l: &Launch<'_>) -> Vec<String> {
    let h = l.harness;
    let continued = l.opens != Opens::First;
    let mut lines = vec![String::new()];
    match (continued, l.mode) {
        (true, _) => lines.push(RUN_GOES_ON.to_string()),
        (false, Some(RunMode::Practice)) => lines
            .push("Your practice run is open. It does not count, and it never pushes.".to_string()),
        (false, Some(RunMode::Competition)) => {
            lines.push("Your competition run is open.".to_string())
        }
        (false, Some(RunMode::Test)) => {
            lines.push("TEST RUN: an organizer test run, not a competition entry.".to_string())
        }
        (false, None) => lines.push("This run is recorded on this computer only.".to_string()),
    }
    lines.push(match l.opens {
        Opens::First => format!("{h} opens now in this terminal."),
        Opens::LastSession => {
            format!("{h} opens now in this terminal and picks up its last session.")
        }
        Opens::NewSession => format!(
            "{h} opens now in this terminal with a new session: no saved session was found \
             for this run."
        ),
    });
    // What the harness shows first comes before what to type: the participant meets it first.
    if let Some(first) = l.first_screen {
        lines.push(first.to_string());
    }
    if l.opens == Opens::LastSession {
        lines.push(format!(
            "If the agent has stopped, type \"{KEEP_GOING}\" and press Enter. If you never sent \
             your prompt, send it now: it is still your one prompt."
        ));
    } else {
        lines.push(format!(
            "Type your one prompt and press Enter. After that you may only type \"{KEEP_GOING}\", \
             when the agent has stopped."
        ));
    }
    if let Some(ends) = &l.ends {
        lines.push(format!(
            "Your run ends at {ends}. You do not need to stop it."
        ));
    }
    lines.push(format!(
        "goalcup prints nothing here until {h} exits. To see the run, run goalcup status in \
         another terminal."
    ));
    lines.push(format!("{READY_MARK}: {h} opens now."));
    lines
}
