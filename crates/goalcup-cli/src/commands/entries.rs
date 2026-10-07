//! `goalcup entries` — the runs this daemon is carrying right now.
//!
//! # It is the live list, and it says so
//!
//! `GET /v0/entries` walks the daemon's in-memory entry map, and a sealed entry is evicted
//! from that map once its last acknowledgement lands — while the signed status route keeps
//! answering for it out of the frozen status cache. The two surfaces disagree by design,
//! and the disagreement is exactly the one a participant would misread: a run they finished
//! ten seconds ago is not in this list, and nothing about that means it is gone.
//!
//! So the list is headed as what it is, and every rendering of it — the empty one
//! especially — names `goalcup status <entry>` as the way to reach a run that has finished.
//! A finished run is reached by id, from the same cache `goalcup status` reads its pinned
//! key out of.

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
    put(out, "the runs this daemon is carrying right now:\n\n")?;
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
        "\nA run leaves this list when it is sealed and its last event has been \
         acknowledged; its signed status is kept and does not go with it. Reach a run that \
         has finished by id: `goalcup status <id>`.\n",
    )
}
