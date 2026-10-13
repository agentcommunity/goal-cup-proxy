//! One `StatusInputs`, one `EventStatus` and one started `Entry`.
//!
//! The status document is `goalcup_protocol::control::StatusPayload` — the daemon defines
//! no second shape — so the fixture builds that type and nothing else. `health_status.rs`
//! reads all of it; `control_api.rs` reads the payload half, which is why it lives here
//! rather than inline in one test file.

use goalcup_daemon::registry::{Entry, RunKeys};
use goalcup_daemon::status::StatusInputs;
use goalcup_protocol::capture_status::{
    AnalysisState, AnchorState, CaptureState, CaptureStorage, DeliveryState, M2Status, RunState,
};
use goalcup_protocol::control::{ArmingState, EventStatus};
use goalcup_protocol::events::HealthState;

/// A started entry.
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
    entry
}

pub fn event_status() -> EventStatus {
    EventStatus {
        t0: "2026-09-04T00:00:00Z".to_string(),
        start_window_closes_at: "2026-09-05T00:00:00Z".to_string(),
        publication_closes_at: "2026-09-12T00:00:00Z".to_string(),
        rules_version: "2026-09-03".to_string(),
    }
}

/// A recording capture's status projection.
pub fn recording_capture() -> M2Status {
    M2Status {
        run: RunState::Running,
        capture: CaptureState::Recording,
        delivery: DeliveryState::LocalOnly,
        analysis: AnalysisState::NotRun,
        anchor: AnchorState::LocalOnly,
        storage: CaptureStorage {
            quota_bytes: 1 << 20,
            reserved_bytes: 1 << 20,
            used_bytes: 4096,
            durable: None,
            accounting_available: true,
        },
        gap_count: 0,
        assistance_observed: false,
    }
}

/// What the daemon knows about `entry` at one moment.
pub fn status_inputs(entry: &Entry) -> StatusInputs<'_> {
    StatusInputs {
        entry,
        arming: ArmingState::Started,
        event: event_status(),
        cycles: goalcup_protocol::control::CycleErrors::default(),
        capture: recording_capture(),
    }
}
