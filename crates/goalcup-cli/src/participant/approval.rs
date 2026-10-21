// --------------------------------------------------------------------------------- approval

use super::*;
use crate::account::device::RunMode;

/// What a browser approval is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Approving<'a> {
    /// `goalcup login`: this computer, once.
    Login,
    /// One run of `harness` (its display name).
    Run { mode: RunMode, harness: &'a str },
}

/// The lines before the browser opens: which approval this is, and that two are normal.
pub fn approval_intro(what: Approving<'_>) -> Vec<String> {
    match what {
        Approving::Login => vec![
            "Connect this computer to your Goal Cup account.".to_string(),
            "You do this once per computer. Each run asks for its own approval later; that is \
             normal."
                .to_string(),
        ],
        Approving::Run { mode, harness } => vec![
            match mode {
                RunMode::Practice => {
                    format!("Approve this practice run of {harness} in your browser.")
                }
                RunMode::Competition => {
                    format!("Approve your competition run of {harness} in your browser.")
                }
                RunMode::Test => {
                    format!("Approve this organizer test run of {harness} in your browser.")
                }
            },
            "Every run needs its own approval, so this is normal: login connected this computer, \
             and this approval starts this one run."
                .to_string(),
        ],
    }
}

/// The lines that show the code and the address, once the browser was asked to open it:
/// `opened` is whether a browser program started. The line with the code ends
/// `shows the code XXXX-XXXX.`: `scripts/matrix.sh` reads it.
pub fn approval_code(
    user_code: &str,
    address: &str,
    opened: bool,
    expires_in_secs: u64,
) -> Vec<String> {
    let mut lines = if opened {
        vec![
            format!(
                "Your browser opened the approval page. Check that the page shows the code {user_code}."
            ),
            format!("If you do not see it, open this address: {address}"),
        ]
    } else {
        vec![format!(
            "Open this address in your browser: {address} and check that the page shows the code {user_code}."
        )]
    };
    lines.push(format!(
        "Waiting for you to approve… (the code expires in {} min; press Ctrl-C to cancel)",
        expires_in_secs.div_ceil(60).max(1)
    ));
    lines
}

/// Said before the browser opens for a competition run while another competition run of the
/// account is open on this computer: approving replaces it.
pub fn replaces_open_run(started: &str) -> String {
    format!(
        "You already have a competition run open (started {started}). Approving replaces it: \
         that run stops, and only the new run counts."
    )
}

/// The end of a login: approved, what it means, and what to do next. `server` is named only when
/// it is not the default one.
pub fn login_done(handle: &str, server: Option<&str>) -> Vec<String> {
    let mut lines = vec![
        format!("Approved. This computer is connected to your Goal Cup account ({handle})."),
        "You do this once per computer. Each run asks for its own approval later. That is normal."
            .to_string(),
    ];
    if let Some(server) = server {
        lines.push(format!("Runs upload to {server}."));
    }
    lines.push("Next: goalcup prep".to_string());
    lines
}

/// Said once a run's approval came back.
pub const APPROVED: &str = "Approved.";

/// Said before the participant's repository is cloned for a new run.
pub const CLONING: &str = "Cloning your repository…";

/// A denied approval.
pub fn approval_denied() -> String {
    format!(
        "{RUN_AGAIN} to start over. The approval was denied in the browser. Nothing was created."
    )
}

/// An approval code that ran out.
pub fn approval_expired() -> String {
    format!(
        "{RUN_AGAIN} for a new code. The code expired before it was approved. Nothing was created."
    )
}

/// Ctrl-C while the approval was waited for.
pub fn approval_interrupted() -> String {
    format!(
        "{RUN_AGAIN} when you are ready. It was interrupted while it waited for your approval. \
         Nothing was created, and no run was opened."
    )
}

/// The server could not be reached to ask for an approval.
pub fn server_unreachable(server: &str, why: &str) -> String {
    format!(
        "Check your connection, then run the command again. The Goal Cup server at {server} \
         could not be reached ({why}). Nothing was created."
    )
}
