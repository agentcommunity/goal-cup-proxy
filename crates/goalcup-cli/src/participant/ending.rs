//! The end of a part of a run on this computer: what happened, and what to do now.

use super::*;
use std::path::Path;

/// The heading above the technical lines printed once the harness has exited.
pub fn details_heading(host_log: Option<&Path>) -> String {
    match host_log {
        Some(log) => format!("{DETAILS_HEADING} (also in {}):", log.display()),
        None => format!("{DETAILS_HEADING}:"),
    }
}

/// The words that end [`helper_not_stopped`], for a test to find it.
pub const OPEN_CALLS_CUT: &str = "so it stopped recording its open calls now.";

/// Said once the recording proxy's stop is done, when the connector's after-exit commands (D72)
/// did not stop what the harness left running and the stop, bounded short instead of waiting on
/// connections such a process would hold until its own idle exit, cut at least one.
pub fn helper_not_stopped(harness: &str) -> String {
    format!("goalcup could not stop {harness}'s background helper, {OPEN_CALLS_CUT}")
}

/// What a practice run that is sealed and uploaded points to: the real run.
pub const PRACTICE_NEXT: &str =
    "Next: your real run, inside the start window: goalcup start --harness <your harness>.";

/// Said before the wait for the rest of the upload, which can take minutes.
pub const UPLOADING_REST: &str =
    "Uploading the rest of the run… keep this terminal open until it says the upload is done.";

/// Said once the rest of the run is uploaded, with its seal.
pub const UPLOAD_DONE: &str = "Upload done.";

/// How a run on a server came to its end on this computer, for [`ending_block`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ending<'a> {
    /// The harness exited, and `goalcup continue` can restart it.
    Open {
        run_id: &'a str,
        /// How the harness exited (`exit code 1`).
        how: &'a str,
        /// The run's end ([`end_words`]).
        until: String,
    },
    /// Goal Cup Proxy stopped the harness at the run's deadline.
    TimeUp { practice: bool },
    /// A newer run on the account replaced this one.
    Replaced,
    /// The harness exited after the run's time was up.
    Over { how: &'a str },
    /// The harness exited, and it cannot be restarted in the same run (this machine does not
    /// hold the run's credential).
    NoRestart { how: &'a str, until: String },
}

/// The block printed at the seal, before the wrap-up upload: what happened, and, for a run that
/// goes on, what to do now. `goalcup continue` works from this moment (the home and the run's
/// reserve are let go at the seal), so it is said before the wait, not after it.
pub fn ending_block(harness: &str, ending: &Ending<'_>) -> Vec<String> {
    let h = harness;
    let mut lines = vec![String::new()];
    match ending {
        Ending::Open { run_id, how, until } => {
            lines.push(format!("{h} has exited ({how})."));
            lines.push(format!("{STILL_OPEN} {until}."));
            lines.push(format!(
                "To go on, run: {CONTINUE_HINT} {run_id} (you can do it in another terminal now)."
            ));
            lines.push(
                "If your work is done, you need do nothing. The run ends by itself.".to_string(),
            );
        }
        Ending::TimeUp { practice: true } => lines.push(format!(
            "{TIME_IS_UP} Your practice run is over. {h} was stopped."
        )),
        Ending::TimeUp { practice: false } => lines.push(format!("{TIME_IS_UP} {h} was stopped.")),
        Ending::Replaced => {
            lines.push(format!(
                "This run was replaced by a newer run on your account. {h} was stopped."
            ));
            lines.push("Only the newer run counts.".to_string());
        }
        Ending::Over { how } => lines.push(format!(
            "{h} has exited ({how}). Your run is over: its time is up."
        )),
        Ending::NoRestart { how, until } => {
            lines.push(format!(
                "{h} has exited ({how}). It cannot be restarted in the same run."
            ));
            lines.push(format!("The run ends by itself at {until}."));
        }
    }
    lines
}

/// The last lines, once the wrap-up upload is over. `all_sent`: everything of this part of the
/// run is on the server, with its seal. `server_closed`: the server said, while it took the upload,
/// that the run is sealed or incomplete, so a run that was to go on cannot be continued.
pub fn after_upload(ending: &Ending<'_>, all_sent: bool, server_closed: bool) -> Vec<String> {
    let mut lines = Vec::new();
    let unsent =
        "Part of the run is not uploaded yet. Next: when you are online, run: goalcup upload";
    match ending {
        Ending::Open { .. } if server_closed => {
            lines.push("The server has closed this run; it cannot be continued.".to_string());
            lines.push(if all_sent {
                format!("{UPLOAD_DONE} {NOTHING_MORE}")
            } else {
                unsent.to_string()
            });
        }
        Ending::Open { .. } => lines.push(if all_sent {
            format!("{UPLOAD_DONE} Nothing more to do in this terminal.")
        } else {
            unsent.to_string()
        }),
        Ending::TimeUp { practice } => {
            if all_sent {
                // The pointer to the real run is a next step, so it replaces "Nothing more to
                // do": the two would contradict each other. It is given only when the upload is
                // complete.
                if *practice {
                    lines.push("Your practice run is sealed and uploaded.".to_string());
                    lines.push(PRACTICE_NEXT.to_string());
                } else {
                    lines.push(format!("Your run is sealed and uploaded. {NOTHING_MORE}"));
                }
            } else {
                lines.push("Your run is sealed on this computer.".to_string());
                lines.push(unsent.to_string());
            }
        }
        Ending::Replaced | Ending::Over { .. } | Ending::NoRestart { .. } => {
            lines.push(if all_sent {
                format!("{UPLOAD_DONE} {NOTHING_MORE}")
            } else {
                unsent.to_string()
            })
        }
    }
    lines
}
