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

/// A new run's project folder is the workspace of run `run_id`, opened on a server and not over:
/// the folder is that run's, and a new run there would be the wrong way back to it.
/// `competition`: that run is a competition run, which a new competition run replaces when it
/// opens (a practice or test run is never replaced), and which needs a repository of its own.
pub fn project_of_open_run(run_id: &str, competition: bool) -> String {
    let new_run = if competition {
        "A new run needs a new folder and a new repository, and replaces this one."
    } else {
        "A new run needs a new folder."
    };
    format!(
        "This folder belongs to run {run_id}. To see it: goalcup status. If the agent has \
         exited: {CONTINUE_HINT}. {new_run} Nothing was created."
    )
}

/// A new run's project folder is one `service` syncs off this computer: it would upload the
/// folder's `.env.local`, and the token in it.
pub fn project_synced(service: &str) -> String {
    format!(
        "{service} syncs this folder and would upload your .env.local and token: use a folder \
         outside it, for example under ~/code. Nothing was created."
    )
}

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
/// `why` is [`crate::model_host::Unrecordable::sentence`]; `id` is the connector's. A value read
/// from the config file is fixed in that file: `edit` is its advice
/// ([`crate::model_host::Unrecordable::edit_sentence`]), and it ends the refusal. Without it, the
/// way is to choose another service through setup.
pub fn cannot_record_model_service(why: &str, id: &str, edit: Option<&str>) -> String {
    match edit {
        Some(edit) => format!(
            "{why} Goal Cup records only an https model service that is not on this computer. {edit}"
        ),
        None => format!(
            "{why} Goal Cup records only an https model service that is not on this computer. Run \
             goalcup setup --harness {id}, choose such a service there, then run the command again."
        ),
    }
}

/// The participant's repository has commits, so a competition or test run cannot use it: by
/// GitHub's size in KB (`kb`), or `None` when GitHub's list of commits said so (a README alone
/// can leave the size at 0). The first two sentences are the server's own refusal.
pub fn repository_not_empty(repo: &str, kb: Option<u64>) -> String {
    let size = kb.map(|kb| format!(" ({kb} KB)")).unwrap_or_default();
    format!(
        "The repository must be empty when the run opens. Create a new one with no README, \
         .gitignore or license, and start again. {repo} already has commits{size}; name the new \
         one when goalcup start asks, or add --repo <owner>/<name>."
    )
}

/// A new run's project folder is goalcup's own folder `root` (`what`), is inside it, holds it,
/// or sits beside it: the agent would see goalcup's records.
pub fn project_beside_goalcup(project: &Path, what: &str, root: &Path) -> String {
    format!(
        "Make a new folder for this run somewhere else, for example in your home folder, and run \
         goalcup start there. This folder ({}) is goalcup's {what} {}, is inside it, holds it, \
         or is next to it, and the agent must not see goalcup's own records. Nothing was created.",
        project.display(),
        root.display()
    )
}

/// A new run's project folder is inside a Git repository (`outer`): the agent's `git init` would
/// make a repository inside another one.
pub fn project_inside_repository(project: &Path, outer: &Path) -> String {
    format!(
        "Make a new folder for this run outside any Git repository, put your .gitignore and \
         .env.local in it, and run goalcup start there. This folder ({}) is inside the Git \
         repository {}. Nothing was created.",
        project.display(),
        outer.display()
    )
}

/// The two lines a project's `.gitignore` needs so that no `.env` file is ever committed.
pub const GITIGNORE_LINES: [&str; 2] = [".env", ".env.*"];

/// Some `.env` files of a new run's project folder (`uncovered`, sorted) are not covered by its
/// `.gitignore`: the agent's first `git add .` would publish them, with the token.
pub fn env_not_ignored(uncovered: &[String]) -> String {
    format!(
        "Add these two lines to the .gitignore in this folder, then run goalcup start again:\n  \
         {}\n  {}\n.gitignore does not cover {}, so the agent's first `git add .` would publish \
         it, and your token with it. Nothing was created.",
        GITIGNORE_LINES[0],
        GITIGNORE_LINES[1],
        uncovered.join(", ")
    )
}

/// A key file of a new run's project folder (`file`) names `GH_TOKEN` in a form the push line
/// cannot read, or more than once. The value is never shown.
pub fn token_line_form(file: &str) -> String {
    format!(
        "Write your token in {file} as one line, exactly GH_TOKEN=<your token>: no export, no \
         quotes, no spaces, and no second GH_TOKEN line in any .env file. The push line goalcup \
         prints reads only that form. Nothing was created."
    )
}

/// A real run's project folder has no `GH_TOKEN` line in any key file: the agent's first push
/// would fail after the approval. With no `.gitignore` there (`gitignore_missing`), the two lines
/// it needs are said too, so both are fixed before the next try.
pub fn token_line_missing(gitignore_missing: bool) -> String {
    let ignore = if gitignore_missing {
        format!(
            " Also make a .gitignore in this folder with these two lines:\n  {}\n  {}\n",
            GITIGNORE_LINES[0], GITIGNORE_LINES[1]
        )
    } else {
        " ".to_string()
    };
    format!(
        "Put your GitHub token in .env.local as one line, GH_TOKEN=<your token>.{ignore}Then \
         run goalcup start again. Nothing was created."
    )
}

/// `goalcup start --dev` for a competition run on a server: the server refuses it at open, so it
/// is refused here, before the approval.
pub fn dev_competes() -> String {
    "Add --practice (or --test) to run a connector this build does not pin. A --dev run never \
     enters the competition. Nothing was created."
        .to_string()
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

/// The command that ends a run's last part and sends it, which works without the run's folder.
fn end_last_part(entry_id: &str) -> String {
    format!("goalcup seal {entry_id} --yes, then goalcup upload")
}

/// `goalcup continue` of a run whose folder is not where the run left it: moved, deleted, or on
/// a disk that is not mounted. The agent's work is in that folder, so the next part needs it
/// there; the part already recorded does not, and can still be ended and sent.
pub fn project_missing(run_id: &str, entry_id: &str, workspace: &Path) -> String {
    format!(
        "Put the run's folder back at {} (a link to it there is fine) and run {CONTINUE_HINT} \
         again. If it is lost, end the run's last part so it still reaches the Goal Cup server: \
         {}. The agent goes on with its work in that folder, and nothing is there. Write to \
         {SUPPORT_ADDRESS} and name run {run_id} if you need help. Nothing was changed.",
        workspace.display(),
        end_last_part(entry_id)
    )
}

/// `goalcup continue` of a run whose folder's path now holds a file: the next part cannot go on
/// there, so the only way on is to put the folder back or end the last part.
pub fn project_not_a_folder(run_id: &str, entry_id: &str, workspace: &Path) -> String {
    format!(
        "Put the run's folder back at {ws} and run {CONTINUE_HINT} again, or end the run's last \
         part so it reaches the Goal Cup server: {}. Run {run_id} cannot go on: {ws} is a file, \
         not the run's folder. Nothing was changed.",
        end_last_part(entry_id),
        ws = workspace.display(),
    )
}

/// `goalcup continue` of a run whose folder this user cannot look in.
pub fn project_unreadable(workspace: &Path, why: &str) -> String {
    format!(
        "Make {} readable to you again, then run {CONTINUE_HINT} again. goalcup cannot look in \
         the run's folder ({why}), so it cannot tell whether it is there. Nothing was changed.",
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
