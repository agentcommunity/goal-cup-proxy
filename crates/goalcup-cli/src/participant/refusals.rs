//! The refusals of `goalcup start` and `goalcup continue`, each with what to do first.

use super::*;
use std::path::Path;

/// `goalcup start` while a run of the same harness records on this computer: they share one home.
///
/// Most often this is a second start by mistake, so the first words say that nothing is needed;
/// replacing the run is a choice, said last, never the first thing to do.
///
/// `replaces_competition_run`: this start is a competition start and a competition run of this
/// account is open on this computer, so a new start would replace it. A practice or test start
/// never replaces a competition run, and does not say so.
pub fn home_busy(harness: &str, replaces_competition_run: bool) -> String {
    // A start that would replace the open competition run is never advised: it ends that run.
    let instead = if replaces_competition_run {
        "Do not start again: a new start replaces your current competition run.".to_string()
    } else {
        format!(
            "To start a new run instead: exit {harness} there first, then run goalcup start again."
        )
    };
    format!(
        "Your run is already going in another terminal. Nothing to do here: {harness} is already \
         recording on this computer. To see it: goalcup status. {instead} Nothing was created."
    )
}

/// `goalcup continue` while a run of the same harness records on this computer.
pub fn still_recording(harness: &str, run_id: &str) -> String {
    format!(
        "Use the terminal where {harness} runs. If it is gone, wait a minute and run \
         {CONTINUE_HINT} again: run {run_id} is still recording on this computer, and only one \
         {harness} run can record at a time. Nothing was changed."
    )
}

/// A chat-mode harness, and no terminal held in the foreground. `then` says what was left.
pub fn needs_terminal(harness: &str, then: &str) -> String {
    format!(
        "Run the command directly in a terminal window: not in the background, and not with its \
         output sent to a file. {harness} runs in this terminal, so goalcup must hold the \
         foreground of a terminal. {then}"
    )
}

/// What the folder checks call the folder `goalcup start` is run from, the run's workspace.
pub const PROJECT_FOLDER: &str = "project folder";

/// A new run's project folder holds more than the starting kit (`extra`, sorted): the run must
/// start in a new, empty folder of its own.
pub fn project_not_empty(project: &Path, extra: &[String]) -> String {
    let shown: Vec<&str> = extra.iter().take(3).map(String::as_str).collect();
    let more = if extra.len() > shown.len() {
        format!(" and {} more", extra.len() - shown.len())
    } else {
        String::new()
    };
    format!(
        "Make a new, empty folder for this run, put your .gitignore and .env.local in it, and run \
         goalcup start there. This folder ({}) also holds {}{more}. A run starts in a folder \
         with nothing but .gitignore and .env files in it. Nothing was created.",
        project.display(),
        shown.join(", ")
    )
}

/// The model service the harness's own config names is one Goal Cup cannot record (D62).
/// `why` is [`crate::model_host::Unrecordable::sentence`]; `id` is the connector's.
pub fn cannot_record_model_service(why: &str, id: &str) -> String {
    format!(
        "{why} Goal Cup records only an https model service that is not on this computer. Run \
         goalcup setup --harness {id}, choose such a service there, then run the command again."
    )
}

/// The participant's repository has commits, so a competition or test run cannot use it: by
/// GitHub's size in KB (`kb`), or `None` when the size is not known.
pub fn repository_not_empty(repo: &str, kb: Option<u64>) -> String {
    let size = kb.map(|kb| format!(" ({kb} KB)")).unwrap_or_default();
    format!(
        "Create a new repository with no README, license or .gitignore, then run goalcup start \
         again and name it (or add --repo <owner>/<name>). The repository {repo} is not \
         empty{size}, and a new run needs a new empty one."
    )
}

/// A folder of a run is on a filesystem a run cannot keep its records on (`name`): what to do
/// first, with no full stop at the end. The project folder is made anew on another volume, the
/// others move with the configuration folder, beside which the records are kept.
pub fn unsupported_filesystem(what: &str, path: &str, name: &str) -> String {
    let how = if what == PROJECT_FOLDER {
        "for example: make your project folder on that volume and run goalcup start there"
    } else {
        "for example: set GOALCUP_CONFIG_DIR to a folder on that volume, and the records are kept \
         beside it"
    };
    format!(
        "move the {what} {path} to an ext4, XFS or APFS volume ({how}): it is on {name}, which a \
         run cannot keep its records on"
    )
}

/// `goalcup start` or `goalcup continue` found a folder on such a filesystem (`why` is one or
/// more [`unsupported_filesystem`]): a warning, and the command goes on.
pub fn filesystem_warning(why: &str) -> String {
    format!(
        "Warning: {why}. This command goes on anyway and does not refuse the run, but goalcup setup \
         fails its check for it."
    )
}

/// The server answered `goalcup start` or `goalcup continue` with a `what` (a run, a part of
/// the run) goalcup cannot use. The answer itself goes to the log, never to the participant.
pub fn server_answer_unusable(server: &str, what: &str) -> String {
    format!(
        "Run the command again, and write to {SUPPORT_ADDRESS} if it keeps happening. The Goal \
         Cup server at {server} answered with a {what} that goalcup cannot use. Nothing was \
         started."
    )
}

/// `goalcup continue` with no record of the run on this computer.
pub fn not_started_here(run_id: Option<&str>) -> String {
    let which = run_id.map_or_else(|| "The run".to_string(), |id| format!("Run {id}"));
    format!(
        "Run {CONTINUE_HINT} on the computer that started the run. {which} {NOT_STARTED_HERE}, \
         and a run can be continued only there. Nothing was changed."
    )
}

/// `goalcup continue` of a run that was never opened on the server.
pub fn local_only(run_id: &str) -> String {
    format!(
        "Start a new run with goalcup start. Run {run_id} was recorded on this computer only, \
         never on the Goal Cup server, so it cannot be continued. Nothing was changed."
    )
}

/// `goalcup continue` of a run the server says is over.
pub fn run_over(run_id: &str) -> String {
    format!(
        "Nothing to continue: run {run_id} is over. Its time is up, or a newer run on your \
         account replaced it. Nothing was started."
    )
}

/// The command that ends a run's last part and sends it, which works without the clone.
fn end_last_part(entry_id: &str) -> String {
    format!("goalcup seal {entry_id} --yes, then goalcup upload")
}

/// `goalcup continue` of a run whose clone is not where the run left it: moved, deleted, or on a
/// disk that is not mounted. The agent's work is in that clone, so the next part needs it there;
/// the part already recorded does not, and can still be ended and sent.
pub fn clone_missing(run_id: &str, entry_id: &str, workspace: &Path) -> String {
    format!(
        "Put the run's folder back at {} (a link to it there is fine) and run {CONTINUE_HINT} \
         again. If it is lost, end the run's last part so it still reaches the Goal Cup server: \
         {}. The agent goes on with its work in that folder, and nothing is there. Write to \
         {SUPPORT_ADDRESS} and name run {run_id} if you need help. Nothing was changed.",
        workspace.display(),
        end_last_part(entry_id)
    )
}

/// Why a run's folder that is there is not its clone, for [`not_a_clone`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotAClone<'a> {
    /// A file, not a folder.
    NotAFolder,
    /// A folder with no `.git` directory: removed, or a `.git` file or link in its place.
    NoGitDirectory,
    /// A clone inside another Git repository, which a run's clone may not be.
    InsideRepository(&'a Path),
}

/// `goalcup continue` of a run whose folder is there but is not its clone any more. Its Git
/// history is what the next part goes on from, and is not there, so the only way on is to end the
/// last part, which still reaches the server.
pub fn not_a_clone(run_id: &str, entry_id: &str, workspace: &Path, why: NotAClone<'_>) -> String {
    let ws = workspace.display();
    let what = match why {
        NotAClone::NotAFolder => format!("{ws} is a file, not the run's folder"),
        NotAClone::NoGitDirectory => format!(
            "the run's folder {ws} is there, but it has no .git directory: it was removed, or is \
             a file or a link now"
        ),
        NotAClone::InsideRepository(outer) => format!(
            "the run's folder {ws} is inside another Git repository, {}",
            outer.display()
        ),
    };
    format!(
        "End the run's last part so it reaches the Goal Cup server: {}. Run {run_id} cannot go \
         on: {what}, so it is not the run's clone any more. If you moved things, put the clone \
         back as it was and run {CONTINUE_HINT} again. Nothing was changed.",
        end_last_part(entry_id)
    )
}

/// `goalcup continue` of a run whose folder this user cannot look in.
pub fn clone_unreadable(workspace: &Path, why: &str) -> String {
    format!(
        "Make {} readable to you again, then run {CONTINUE_HINT} again. goalcup cannot look in \
         the run's folder ({why}), so it cannot tell whether the run's clone is there. Nothing \
         was changed.",
        workspace.display()
    )
}

/// `goalcup continue` while the last part's host still holds the run's reservation.
pub fn last_part_closing(run_id: &str) -> String {
    format!(
        "Try again in a minute. The last part of run {run_id} is still being closed. Nothing was \
         changed."
    )
}

/// `goalcup continue` of a run that was continued on another computer.
pub fn continued_elsewhere(run_id: &str, latest: &str, ours: &str) -> String {
    format!(
        "Continue this run only on the computer that started it. The server's latest part of run \
         {run_id} (part {latest}) is not the one this computer ended last (part {ours}), so it \
         was continued from somewhere else. Nothing was started."
    )
}
