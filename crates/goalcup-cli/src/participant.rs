//! What the participant reads during a run: the approval, the launch, the harness's exit, the
//! deadline, `goalcup continue`, and the refusals of a start or a continue. All of it is here,
//! as plain functions of the facts, so it is worded in one place and tested in one place.
//!
//! The rules (the UX plan, section 0):
//!
//! - Plain words and short sentences. No rule codes, and no internal words (entry, segment,
//!   chain, seal.json, daemon) in a participant block.
//! - A harness is named by its manifest's `participant.display_name` only
//!   ([`crate::connector::HarnessConnector::display_name`]), never by a name in this file.
//! - A block ends with what to do next, or with "Nothing more to do". A refusal says what to do
//!   first, then why.
//! - Times are the participant's local time with its zone, and the time left comes from the
//!   server's count when there is one ([`crate::run::TimeLeft`]).
//!
//! Some words are a contract: scripts and tests read them. They are the `pub const`s below, and
//! the technical lines `goalcup start` prints after the harness exits (`deadline:`,
//! `harness exited:`, `uploaded: … and the seal`), which stay as they were under
//! [`details_heading`]. A test asserts these constants and the facts (a run id, a command, a
//! time), never a whole sentence.

mod approval;
mod ending;
mod launch;
mod refusals;
mod seal;
mod status;
mod times;
mod upload;

pub use approval::{
    APPROVED, Approving, approval_code, approval_denied, approval_expired, approval_interrupted,
    approval_intro, login_done, replaces_open_run, server_unreachable,
};
pub use ending::{
    Ending, UPLOAD_DONE, UPLOADING_REST, after_upload, details_heading, ending_block,
};
pub use launch::{Launch, Opens, RUN_GOES_ON, launch_block, prompt_block};
pub use refusals::{
    GITIGNORE_LINES, PROJECT_FOLDER, cannot_record_model_service, continued_elsewhere,
    dev_competes, env_not_ignored, filesystem_warning, home_busy, last_part_closing, local_only,
    needs_terminal, not_started_here, project_beside_goalcup, project_inside_repository,
    project_missing, project_not_a_folder, project_not_empty, project_unreadable,
    repository_not_empty, run_over, server_answer_unusable, still_recording, token_line_form,
    token_line_missing, unsupported_filesystem,
};
pub use seal::{
    SealDone, seal_done, seal_inside_harness, seal_needs_terminal, seal_not_confirmed,
    seal_question,
};
pub use status::{
    Agent, CHECK_NETWORK, End, NATIVE_SILENT_MARK, Next, PROBLEM, Phase, Restart, STATUS_DETAILS,
    StatusHeader, UploadNow, Who, agent_finished, disk_resumed, disk_resumed_unread, ends_line,
    no_run_yet, problem_agent_exited, problem_disk, problem_disk_paused, problem_heartbeats,
    problem_native_silent, problem_recording_failed, problem_recording_gaps,
    problem_server_refuses, problem_sleep, problem_state_stale, problem_upload, status_header,
    status_next, this_computer,
};
pub use times::{
    clock_words, end_words, left_words, local_clock, no_open_run_time_left, run_seconds_left,
    seconds_left, time_left_unknown,
};
pub use upload::{
    entries_intro, upload_finished, upload_incomplete, upload_local_run, upload_no_run,
    upload_not_here, upload_not_opened, upload_not_sealed, upload_nothing_left, upload_refused,
    upload_seal_missing, upload_waiting,
};

/// The ready mark: the last line before a chat-mode harness takes the terminal.
/// `scripts/matrix.sh`, `scripts/linux-harness-smoke.sh` and the launch tests wait for it.
pub const READY_MARK: &str = "Recording in chat mode";

/// The command that restarts a run's agent.
pub const CONTINUE_HINT: &str = "goalcup continue";

/// The only words the participant may type after the one prompt.
pub const KEEP_GOING: &str = "keep going";

/// The first words of the block that says a run goes on after its harness exited.
pub const STILL_OPEN: &str = "Your run is still open until";

/// The first words of the block at the deadline.
pub const TIME_IS_UP: &str = "Time is up.";

/// The heading above the technical lines printed after the harness exits.
pub const DETAILS_HEADING: &str = "Details for support";

/// The end of every block after which the participant need do nothing.
pub const NOTHING_MORE: &str = "Nothing more to do.";

/// The line `goalcup continue` says while the earlier part of the run is sent beside it.
pub const EARLIER_PART_UPLOADING: &str = "The earlier part of this run uploads in the background.";

/// What a refusal of `goalcup continue` on another computer says.
pub const NOT_STARTED_HERE: &str = "was not started on this computer";

/// What a refusal says when the approval code must be asked for again.
pub const RUN_AGAIN: &str = "Run the command again";

/// Where to free disk space, and what never to delete: what the agent made is the run's work.
pub const FREE_OUTSIDE: &str =
    "outside your run's folders, and do not delete anything the agent made";

/// Where a participant writes when something unexpected happened.
pub const SUPPORT_ADDRESS: &str = "goalcup@agentcommunity.org";

#[cfg(test)]
mod tests;
