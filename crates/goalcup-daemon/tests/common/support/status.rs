//! One `StatusInputs`, one `EventStatus` and one started `Entry`.
//!
//! The status document is `goalcup_protocol::control::StatusPayload` — the daemon defines
//! no second shape — so the fixture builds that type and nothing else. `health_status.rs`
//! reads all of it; `control_api.rs` reads the payload half, which is why it lives here
//! rather than inline in one test file.

use goalcup_daemon::arming::GateState;
use goalcup_daemon::registry::{Entry, RunKeys};
use goalcup_daemon::status::StatusInputs;
use goalcup_protocol::control::EventStatus;
use goalcup_protocol::events::HealthState;
use goalcup_protocol::manifest::ConnectorManifest;
use std::sync::OnceLock;

/// An entry with a clock: started, with a deadline the status can count down to.
///
/// `Entry::new` touches no filesystem, so the fixture needs no temporary directory; the
/// paths it carries are strings the payload copies and nothing opens.
pub fn started_entry() -> Entry {
    let mut entry = Entry::new(
        "entry-0001",
        "claude-code-cli",
        std::path::PathBuf::from("/tmp/goalcup/ws/entry-0001"),
        std::path::PathBuf::from("/tmp/goalcup/artifacts/entry-0001"),
        &"ab".repeat(32),
        "2.1.259",
        goalcup_daemon::config::ReceiptBindingPolicy::Legacy,
        goalcup_daemon::registry::ExpectedProcessHandoff::NotRequired,
        RunKeys::generate(),
    );
    entry.state = goalcup_daemon::registry::EntryState::Started;
    entry.health = HealthState::Online;
    entry.session_id = Some("95addb64".to_string());
    entry.started_at = Some("2026-09-03T07:42:00Z".to_string());
    entry.deadline_at = Some("2026-09-10T07:42:00Z".to_string());
    entry.deadline_unix = Some(20_000.0);
    entry.events_uploaded = 206;
    entry
}

/// The shipped Claude manifest, once, so `StatusInputs` has something to borrow that
/// outlives the call. `StatusInputs` holds a `&ConnectorManifest`, and a fixture that
/// built one on the stack could not return the struct that borrows it.
fn claude_manifest() -> &'static ConnectorManifest {
    static MANIFEST: OnceLock<ConnectorManifest> = OnceLock::new();
    MANIFEST.get_or_init(|| super::manifest("claude-code-cli"))
}

pub fn event_status() -> EventStatus {
    EventStatus {
        t0: "2026-09-04T00:00:00Z".to_string(),
        start_window_closes_at: "2026-09-05T00:00:00Z".to_string(),
        publication_closes_at: "2026-09-12T00:00:00Z".to_string(),
        rules_version: "2026-09-03".to_string(),
    }
}

/// What the daemon knows about `entry` at one moment, against the shipped manifest.
pub fn status_inputs(entry: &Entry) -> StatusInputs<'_> {
    StatusInputs {
        entry,
        manifest: Some(claude_manifest()),
        arming: GateState::Started,
        health: HealthState::Online,
        events_uploaded: 206,
        events_pending: 0,
        last_sync_at: Some("2026-09-04T08:59:55Z".to_string()),
        now: 10_000.0,
        event: event_status(),
        cycles: goalcup_protocol::control::CycleErrors::default(),
        polls_capped: 0,
        cursor_restore_failures: 0,
        skipped_replayed: 0,
        buffer_gap_at: None,
        files_capped: 0,
        harness_unverified: false,
    }
}
