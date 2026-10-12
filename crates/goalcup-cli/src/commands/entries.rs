//! `goalcup entries` — every run the daemon's registry holds.
//!
//! `GET /v0/entries` lists the open runs and the sealed rows alike, so a run finished ten
//! seconds ago, or a week ago, is still here. `main` starts the run-scoped daemon when none
//! answers, so the list does not depend on a daemon happening to be up. Each line names the
//! id `goalcup status <id>` takes.

use crate::CliError;
use crate::commands::status::{put, wire_word};
use crate::retry::retry_busy;
use goalcup_protocol::control::DaemonClient;
use std::io::Write;

/// `goalcup entries [--json]`.
///
/// `--json` is the summaries as the daemon sent them, one array, so a caller parses the
/// control API's own shape rather than this rendering. The prose below it is for a reader.
pub fn entries_with(
    client: &dyn DaemonClient,
    json: bool,
    out: &mut dyn Write,
) -> Result<(), CliError> {
    let live = retry_busy(|| client.entries())?;
    if json {
        return put(out, &format!("{}\n", serde_json::to_string_pretty(&live)?));
    }
    put(out, "the runs the tracker holds:\n\n")?;
    if live.is_empty() {
        put(out, "  (none)\n")?;
    }
    for e in &live {
        // One line per run, and the id first: it is what every other command is given.
        put(
            out,
            &format!(
                "  {}  {}  {}  {}  {}\n",
                e.entry_id,
                e.connector_id,
                wire_word(&e.arming),
                wire_word(&e.health),
                e.workspace
            ),
        )?;
    }
    put(
        out,
        "\nShow one run, sealed or not, by id: `goalcup status <id>`.\n",
    )
}
