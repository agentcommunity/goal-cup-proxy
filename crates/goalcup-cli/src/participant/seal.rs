//! What `goalcup seal` asks and says.

use super::*;
use std::path::Path;

/// `goalcup seal` run inside the agent's session, which never ends its own run.
pub fn seal_inside_harness() -> &'static str {
    "To end the run early, a person runs goalcup seal from another terminal. goalcup seal does \
     not run inside the agent's session, so the agent cannot end its own run."
}

/// What `goalcup seal` asks before it stops the agent. `until` is the run's end ([`end_words`])
/// while the run stays open.
pub fn seal_question(harness: &str, until: Option<&str>) -> String {
    match until {
        Some(until) => format!(
            "This stops {harness} now. Your run stays open until {until}. Type yes to stop it."
        ),
        None => format!("This stops {harness} now. Type yes to stop it."),
    }
}

/// The answer to [`seal_question`] was not yes.
pub fn seal_not_confirmed() -> String {
    "Nothing was stopped: the answer was not yes. To stop it without this question: goalcup \
     seal --yes"
        .to_string()
}

/// `goalcup seal` with no terminal to ask in, and no `--yes`.
pub fn seal_needs_terminal() -> &'static str {
    "To stop the agent from a script, run: goalcup seal --yes. goalcup seal asks before it stops \
     the agent, and there is no terminal here to ask in. Nothing was stopped."
}

/// The facts of what `goalcup seal` says once it is done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealDone<'a> {
    pub harness: &'a str,
    pub part_id: &'a str,
    /// The part's seal was written.
    pub sealed: bool,
    /// The run and its end, when `goalcup continue` can start the next part.
    pub go_on: Option<(&'a str, &'a str)>,
    /// The run's end while it stays open (it may still be unable to go on).
    pub open_until: Option<&'a str>,
    /// The log the technical lines went to.
    pub details: Option<&'a Path>,
}

/// What `goalcup seal` says once the agent is stopped: plain words, the next step last.
pub fn seal_done(d: &SealDone<'_>) -> Vec<String> {
    let h = d.harness;
    let mut lines = vec![if d.sealed {
        format!("{h} was stopped, and this part of your run is sealed.")
    } else {
        format!(
            "{h} was stopped, but this part of your run could not be sealed: the disk may be \
             full. Free some space {FREE_OUTSIDE}, then run: goalcup seal {}",
            d.part_id
        )
    }];
    if let Some(log) = d.details {
        lines.push(format!("Details for support are in {}.", log.display()));
    }
    match (d.go_on, d.open_until) {
        (Some((run_id, until)), _) => {
            lines.push(format!("{STILL_OPEN} {until}."));
            lines.push(format!(
                "To go on: {CONTINUE_HINT} {run_id}. If your work is done, nothing else is needed."
            ));
        }
        (None, Some(until)) => {
            lines.push(format!("The run ends by itself at {until}. {NOTHING_MORE}"))
        }
        (None, None) => lines.push(NOTHING_MORE.to_string()),
    }
    lines
}
