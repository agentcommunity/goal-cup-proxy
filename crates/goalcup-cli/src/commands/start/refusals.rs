//! `goalcup start`'s and `goalcup continue`'s refusals: the sentences for an interrupted start,
//! and the checks made before anything is created.

use crate::CliError;
use crate::connector::HarnessConnector;
use crate::model_host::ModelHost;
use crate::preflight::Level;
use crate::proxy::connect::ConnectFailureKind;
use crate::proxy::preflight::{PREFLIGHT_BUDGET, Unreachable, endpoints_reachable};
use crate::run::reserve::artifact_root;
use std::collections::BTreeMap;
use std::path::Path;

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
    // Checked here, before anything is closed; the segment reads it again at its launch. A name
    // that resolves only to this computer is refused as `start` refuses it, behind the same
    // test-only offline seam.
    if let Some(model) = crate::model_host::for_run(cfg, connector)?
        && !offline_upstreams_for_test(env)
    {
        crate::model_host::refuse_if_on_this_computer(&model).map_err(|refused| {
            CliError::Refused(
                refused.refusal(connector.display_name(), &connector.loaded.manifest.id),
            )
        })?;
    }
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
/// address of a fake GitHub API. A test build without it does not ask the API. A shipped build
/// always asks GitHub.
#[cfg(feature = "test-support")]
pub const TEST_GITHUB_API: &str = "GOALCUP_TEST_GITHUB_API";

/// What a start that fails after the approval says once it has removed what it made (said once,
/// by `capture`, for every failure of that phase): the run directory and the run's credential
/// directory. It never removes the project folder (the participant's, the run's workspace), the
/// `<config>-runs` folder the run directory was made in, or the shared harness-home folder of a
/// connector under `<cfg>/capture-homes/` (one per connector, persistent, and reused by every run
/// of it).
pub(super) const CLEANED_UP: &str = "Nothing was started, and what this start made was removed.";

/// The folder checks over every folder a run writes to: the project folder (a run on a server's
/// workspace), the folder the run's own directory is made in, the artifact root and the
/// configuration folder.
/// WSL1, or a folder on a Windows drive under WSL2, is refused; a folder on a filesystem a run
/// cannot keep its records on is warned about on the terminal and the command goes on (`goalcup
/// prep` fails its step for it). See [`crate::folder_gate`]. `env` states the machine in a test
/// build only.
pub(crate) fn check_run_folders(
    env: &BTreeMap<String, String>,
    cfg: &Path,
    run_folder: &Path,
    project: Option<&Path>,
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
    if let Some(project) = project {
        folders.insert(0, (crate::participant::PROJECT_FOLDER, project));
    }
    host.refuse(&folders, untouched)?;
    if let Some(warning) = host.warn_filesystems(&folders) {
        crate::host_log::say(&warning);
    }
    Ok(())
}

/// A test build's door past the local refusal of `--dev` in the competition
/// ([`crate::participant::dev_competes`]): the test connectors are pinned by no build, so they
/// run through `--dev`, and most tests are competition runs. It stands in for a pinned connector;
/// the server's own refusal is not affected. A shipped build compiles none of this.
#[cfg(feature = "test-support")]
pub const TEST_DEV_COMPETES: &str = "GOALCUP_TEST_DEV_COMPETES";

#[cfg(feature = "test-support")]
pub(super) fn dev_competes_for_test(env: &BTreeMap<String, String>) -> bool {
    env.get(TEST_DEV_COMPETES).is_some_and(|v| v == "1")
}

#[cfg(not(feature = "test-support"))]
pub(super) fn dev_competes_for_test(_env: &BTreeMap<String, String>) -> bool {
    false
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

/// A test build's stated answers to the reachability probes `goalcup start` makes, as
/// `host=ok,host=unreachable` pairs. A host it does not name is probed for real, as a shipped
/// build probes every host. Lets a test state a network that blocks one host and reaches
/// another, which no test machine can state for itself.
#[cfg(feature = "test-support")]
pub const TEST_PROBE_ANSWERS: &str = "GOALCUP_TEST_PROBE_ANSWERS";

/// The reachability `goalcup start` asks of `endpoints`, in order: the first that does not answer
/// is the answer. A host a test build states an answer for ([`TEST_PROBE_ANSWERS`]) is answered
/// so; the rest are probed for real.
pub(super) fn reachable(
    env: &BTreeMap<String, String>,
    endpoints: &[(&str, u16)],
) -> Result<(), Unreachable> {
    let mut probed = Vec::new();
    for &(host, port) in endpoints {
        match stated_answer_for_test(env, host) {
            Some(true) => {}
            Some(false) => {
                return Err(Unreachable {
                    host: host.to_string(),
                    port,
                    kind: ConnectFailureKind::TcpRefused,
                    budget: PREFLIGHT_BUDGET,
                    binary: std::env::current_exe().ok(),
                });
            }
            None => probed.push((host, port)),
        }
    }
    endpoints_reachable(probed)
}

/// The answer a test build states for `host`: `Some(true)` for `ok`, `Some(false)` for any other
/// word, `None` when it states none.
#[cfg(feature = "test-support")]
fn stated_answer_for_test(env: &BTreeMap<String, String>, host: &str) -> Option<bool> {
    let stated = env.get(TEST_PROBE_ANSWERS)?;
    stated
        .split(',')
        .filter_map(|pair| pair.split_once('='))
        .find(|(name, _)| *name == host)
        .map(|(_, answer)| answer == "ok")
}

#[cfg(not(feature = "test-support"))]
fn stated_answer_for_test(_env: &BTreeMap<String, String>, _host: &str) -> Option<bool> {
    None
}

/// The one line `goalcup start` says when a default host the run does not use cannot be reached,
/// because the settings name a model service that answers: what happened, and what to do.
pub(super) fn default_host_warning(missing: &Unreachable, service: &ModelHost) -> String {
    format!(
        "warning: {}:{} could not be reached; your settings use {}, so the run goes on. The run does \
         not need that host. To use it, check the network or a firewall such as Little Snitch, \
         then start again.",
        missing.host,
        missing.port,
        service.authority()
    )
}
