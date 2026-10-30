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

/// The footer of `goalcup --help` (PRD-10 section 11).
pub fn after_help() -> String {
    format!(
        "\
Steps:
  1. goalcup setup                                   once, until it says Ready.
  2. goalcup start --practice                         a 10-minute try, no GitHub (optional)
  3. goalcup start --repo <you>/<repo> --prompt FILE  your run, in the start window; FILE outside the folder
While it runs: goalcup status (in another terminal). If status says so: goalcup recover.
If the agent stopped early: goalcup continue.
Guide: https://github.com/agentcommunity/goal-cup-proxy/blob/main/docs/PARTICIPANT-GUIDE.md
Help: {SUPPORT_ADDRESS}
Exit codes: 0 done; 1 refused, and you can fix it; 3 the command could not do its job."
    )
}

/// PRD-10 section 1, "File connectors in competition".
pub const FILE_FLAGS_COMPETE: &str = "--connector and --hosts are for practice and test runs. \
A competition run uses the connectors built into goalcup.";

/// `GOALCUP_SERVER` holds something a run cannot use.
pub const BAD_SERVER: &str = "GOALCUP_SERVER must be off, an https:// address, or an http:// \
address on this computer (127.0.0.1, ::1 or localhost).";

/// No absolute folder to keep runs in: never a folder relative to where goalcup was started.
pub const NO_HOME: &str = "goalcup cannot tell where to keep your runs: HOME is not set to a full \
path. Set HOME, or GOALCUP_HOME, to a folder outside your project.";

/// A command whose body is not in this build yet.
pub fn not_built(command: &str) -> String {
    format!("{command} is not built yet")
}
