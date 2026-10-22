//! `goalcup entries` — every run the daemon's registry holds.
//!
//! `GET /v0/entries` lists the open runs and the sealed rows alike, so a run finished ten
//! seconds ago, or a week ago, is still here. `main` starts the run-scoped daemon from
//! `daemon.toml` when none answers, so the list does not depend on a daemon happening to be up.
//! Each line names the part id `goalcup status <id>` takes and the run id `goalcup continue
//! <run-id>` takes. With no `daemon.toml` no daemon was ever started here, and the answer is the
//! empty list, with no daemon started for it.

use crate::CliError;
use crate::commands::status::{put, wire_word};
use crate::retry::retry_busy;
use goalcup_protocol::control::DaemonClient;
use std::io::Write;
use std::path::Path;

/// `goalcup entries [--json]`. The run of each part is found in `cfg`'s run records.
///
/// `--json` is the summaries as the daemon sent them, one array, so a caller parses the
/// control API's own shape rather than this rendering. The prose below it is for a reader.
pub fn entries_with(
    client: &dyn DaemonClient,
    cfg: &Path,
    json: bool,
    out: &mut dyn Write,
) -> Result<(), CliError> {
    let live = retry_busy(|| client.entries())?;
    render_entries(cfg, &live, json, out)
}

/// The list, as `entries_with` prints it. `main` hands it an empty list when no daemon was
/// ever started from this machine.
pub fn render_entries(
    cfg: &Path,
    live: &[goalcup_protocol::control::EntrySummary],
    json: bool,
    out: &mut dyn Write,
) -> Result<(), CliError> {
    if json {
        return put(out, &format!("{}\n", serde_json::to_string_pretty(&live)?));
    }
    for line in crate::participant::entries_intro(live.is_empty()) {
        put(out, &format!("{line}\n"))?;
    }
    if live.is_empty() {
        put(out, "  (none)\n")?;
    }
    for e in live {
        // One line per part, and the ids first: they are what every other command is given. A
        // part with no run record on this computer has no run id to give.
        let run_id = crate::run::of_entry(cfg, &e.entry_id)
            .ok()
            .flatten()
            .map_or_else(|| "-".to_string(), |run| run.run_id);
        put(
            out,
            &format!(
                "  {}  {}  {}  {}  {}\n",
                e.entry_id,
                run_id,
                e.connector_id,
                state_words(e),
                e.workspace
            ),
        )?;
    }
    Ok(())
}

/// What a run is doing, in the participant's words.
fn state_words(e: &goalcup_protocol::control::EntrySummary) -> String {
    use goalcup_protocol::control::ArmingState;
    use goalcup_protocol::events::HealthState;
    match (&e.health, &e.arming) {
        (HealthState::Sealed, _) => "sealed".to_string(),
        (HealthState::SealingRecoveryNeeded, _) => "needs attention".to_string(),
        (HealthState::Online, ArmingState::Armed) => "not started".to_string(),
        (HealthState::Online, ArmingState::Started) => "recording".to_string(),
        // A word this build does not know: the wire's own, never a guess.
        (HealthState::Unknown, _) => wire_word(&e.health),
    }
}
