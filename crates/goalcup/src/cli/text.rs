//! Every message the participant sees (PRD-10 section 13), as plain functions of facts.
//!
//! The three contract words are kept byte for byte, because the pad and scripts read them:
//! [`READY_MARK`], [`PROBLEM`] and the `shows the code <code>.` ending of [`approval_code`].
//! Later slices add the other messages here.

/// The start of the line that says the harness is being recorded.
pub const READY_MARK: &str = "Recording in chat mode";

/// The start of a status line the participant can act on or must report.
pub const PROBLEM: &str = "PROBLEM:";

/// Where a participant writes for help.
pub const SUPPORT_ADDRESS: &str = "goalcup@agentcommunity.org";

/// The approval line; it ends `shows the code <code>.`.
pub fn approval_code(code: &str) -> String {
    format!("Check that the approval page shows the code {code}.")
}
