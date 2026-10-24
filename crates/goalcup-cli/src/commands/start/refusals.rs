//! `goalcup start`'s and `goalcup continue`'s refusals: the sentences for an interrupted start,
//! and the checks made before anything is created.

use crate::CliError;
use crate::connector::HarnessConnector;
use crate::preflight::Level;
use crate::run::reserve::artifact_root;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One line, because a participant who pressed Ctrl-C is not reading a page.
///
/// Rung 1 and not rung 4: nothing refused this run, the participant stopped it, and a script
/// branching on the number must not read a cancellation as an arming gate that closed.
pub(crate) fn interrupted_refusal() -> CliError {
    CliError::Refused(
        "Run the command again when you are ready. It was interrupted before the run began, so \
         nothing is left behind."
            .to_string(),
    )
}

/// A Ctrl-C after the entry was registered and before the harness was released: the parked
/// child is ended without running, and the entry is sealed with no launch.
pub(super) fn interrupted_before_launch() -> CliError {
    CliError::Refused(
        "Run the command again when you are ready. It was interrupted before the harness \
         started, so nothing ran; what was made is sealed without a launch."
            .to_string(),
    )
}

/// `goalcup continue`'s refusals that need no server and change nothing, made before the
/// shared harness home is locked and before any earlier segment is sealed or sent: this must be
/// the foreground of a terminal, and the harness, its pin and its executable must be usable.
/// [`prepare`] asks them again of the segment it launches.
pub(crate) fn continue_preflight(
    cfg: &Path,
    connector: &HarnessConnector,
    dev: bool,
    env: &BTreeMap<String, String>,
) -> Result<(), CliError> {
    let profile = &connector.loaded.manifest.capture;
    holds_the_terminal(connector.display_name(), "Nothing was changed.")?;
    let pin = crate::preflight::harness_pin_gate(connector, dev);
    if pin.level == Level::Fail {
        return Err(CliError::Refused(format!(
            "`goalcup continue` refuses this run:\n{}",
            pin.render().trim_end()
        )));
    }
    crate::launch::capture_executable(connector, profile, env)?;
    // Checked here, before anything is closed; the segment reads it again at its launch.
    crate::model_host::for_run(cfg, connector)?;
    Ok(())
}

/// A chat-mode harness reads and draws on this terminal: `goalcup start`'s input must be a
/// terminal it holds in the foreground (or the harness's first read stops it with `SIGTTIN` and
/// the run records a frozen harness), and its output must be a terminal too (a harness drawing
/// into a file is not a chat). `name` is the harness's display name; `then` says what was left
/// behind.
pub(super) fn holds_the_terminal(name: &str, then: &str) -> Result<(), CliError> {
    use goalcup_platform::terminal::{is_foreground, is_terminal};
    if crate::chat_without_terminal()
        || is_foreground(libc::STDIN_FILENO) && is_terminal(libc::STDOUT_FILENO)
    {
        return Ok(());
    }
    Err(CliError::Refused(crate::participant::needs_terminal(
        name, then,
    )))
}

/// A test build's door to GitHub for a real run's repository (`run_repo::choose`): the base
/// address of a fake GitHub API. A test build without it does not ask the API, because its clone
/// comes from a local stand-in ([`super::TEST_CLONE_SOURCE`]), not from GitHub. A shipped build
/// always asks GitHub.
#[cfg(feature = "test-support")]
pub const TEST_GITHUB_API: &str = "GOALCUP_TEST_GITHUB_API";

/// What a start that fails after the approval says once it has removed what it made (said once,
/// by `capture`, for every failure of that phase): the run directory, the clone, and the run's
/// credential directory. It does not remove the work folder itself (the participant's folder,
/// left in place once created) or the shared harness-home folder of a connector under
/// `<cfg>/capture-homes/` (one per connector, persistent, and reused by every run of it).
pub(super) const CLEANED_UP: &str = "Nothing was started, and what this start made was removed.";

/// The folder a continued run's own folder is made in: `requested` (the folder `goalcup
/// continue` was run from), or this process's current directory. It is refused inside the work
/// folder the run's clone is in, for the reason a new run's is
/// ([`refuse_goal_folder_inside_work_root`]).
pub(crate) fn run_folder_for_continue(
    requested: Option<&Path>,
    workspace: &Path,
) -> Result<PathBuf, CliError> {
    let beside = match requested {
        Some(folder) => folder.to_path_buf(),
        None => std::env::current_dir().map_err(|source| CliError::Io {
            path: "the current directory".to_string(),
            source,
        })?,
    };
    // A run that works in the participant's project folder is continued from that folder; its
    // own directory goes in the configuration directory (`capture`).
    if let Some(work_root) = workspace.parent()
        && !beside.starts_with(workspace)
    {
        refuse_goal_folder_inside_work_root(work_root, &beside)?;
    }
    Ok(beside)
}

/// The work folder `goalcup prep` kept, when a server is set up (a run with none makes no clone).
pub(super) fn prepared_work_folder(cfg: &Path) -> Option<PathBuf> {
    crate::server::read_server_config(cfg).ok().flatten()?;
    let kept = crate::prep_file::read_prep_lenient(cfg).work_root;
    (!kept.is_empty()).then(|| PathBuf::from(kept))
}

/// The folder checks over every folder a run writes to: the work folder (when there is one), the
/// folder the run's own directory is made in, the artifact root and the configuration folder.
/// WSL1, or a folder on a Windows drive under WSL2, is refused; a folder on a filesystem a run
/// cannot keep its records on is warned about on the terminal and the command goes on (`goalcup
/// prep` fails its step for it). See [`crate::folder_gate`]. `env` states the machine in a test
/// build only.
pub(crate) fn check_run_folders(
    env: &BTreeMap<String, String>,
    cfg: &Path,
    run_folder: &Path,
    work_folder: Option<&Path>,
    untouched: &str,
) -> Result<(), CliError> {
    #[cfg(feature = "test-support")]
    let host = crate::folder_gate::Host::from_env(env);
    #[cfg(not(feature = "test-support"))]
    let host = {
        let _ = env;
        crate::folder_gate::Host::here()
    };
    let artifacts = artifact_root(cfg);
    let mut folders = vec![
        ("run folder", run_folder),
        ("artifact root", artifacts.as_path()),
        ("config folder", cfg),
    ];
    if let Some(work_folder) = work_folder {
        folders.insert(0, ("work folder", work_folder));
    }
    host.refuse(&folders, untouched)?;
    if let Some(warning) = host.warn_filesystems(&folders) {
        crate::host_log::say(&warning);
    }
    Ok(())
}

/// The run directory is made in the folder `goalcup start` is run from and holds the harness's
/// `HOME`; the clone is made in the work folder, under a name of its own. They cannot be the same
/// folder, but a folder *below* the work folder would put the run directory (and so the
/// harness's home) inside some other clone, or inside an older run's folder. So: `beside` may be
/// the work folder itself, or anywhere outside it, and is refused anywhere inside it.
pub(super) fn refuse_goal_folder_inside_work_root(
    work_root: &Path,
    beside: &Path,
) -> Result<(), CliError> {
    let beside = std::fs::canonicalize(beside);
    let root = std::fs::canonicalize(work_root).unwrap_or_else(|_| work_root.to_path_buf());
    if let Ok(beside) = beside
        && beside != root
        && beside.starts_with(&root)
    {
        return Err(CliError::Refused(crate::participant::inside_work_folder(
            work_root, &beside,
        )));
    }
    Ok(())
}

/// A test build's door past the provider-host probe, so the harness contract kit can run the
/// shipped connectors (whose hosts are real providers) with no network and reach no provider.
/// A shipped build compiles none of this, and always probes.
#[cfg(feature = "test-support")]
pub const TEST_OFFLINE_UPSTREAMS: &str = "GOALCUP_TEST_OFFLINE_UPSTREAMS";

#[cfg(feature = "test-support")]
pub(super) fn offline_upstreams_for_test(env: &BTreeMap<String, String>) -> bool {
    env.contains_key(TEST_OFFLINE_UPSTREAMS)
}

#[cfg(not(feature = "test-support"))]
pub(super) fn offline_upstreams_for_test(_env: &BTreeMap<String, String>) -> bool {
    false
}
