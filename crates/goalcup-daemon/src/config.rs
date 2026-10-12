//! The daemon's per-machine configuration file.
//!
//! This is the only thing the daemon invents that is not already in `goalcup-protocol`.
//! Everything below is either a path root the daemon resolves templates against, a cadence,
//! or a threshold; nothing here is a rule, and nothing here is read by the verifier.

use crate::DaemonError;
use crate::submission_types::CaptureResourceLimitsV1;
use goalcup_protocol::submission::{
    EffectiveSubmissionLimitsV1, LimitProfileV1, SubmissionProfileV1,
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

/// `#[serde(deny_unknown_fields)]`, and that attribute is the whole mechanism behind the
/// two "cut fields are load errors" assertions in `tests/config_load.rs`: without it, a
/// config still setting `unix_socket` or `foreign_scan_interval_ms` would load and be
/// silently ignored — the participant would believe a transport was configured and a
/// scan scheduled, and neither would exist.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DaemonConfig {
    /// Where the daemon writes. **The only tree it writes to outside a tracker-created
    /// workspace**: never `$HOME`, never a harness config directory, never `spikes/`.
    pub artifact_root: PathBuf,
    /// Where the shipped `manifest.toml` files live.
    pub connectors_dir: PathBuf,
    /// Where `control.json` and the status-cache mirrors live. Separate from
    /// `artifact_root` because the CLI and the MCP server read it, and neither may see a
    /// capture buffer.
    pub config_dir: PathBuf,
    /// Where `create_workspace` puts a workspace when the caller does not choose one.
    pub workspace_parent: PathBuf,
    /// The control API's listen address. `validate()` refuses anything but loopback.
    pub control_addr: SocketAddr,
    /// Where signed chunks go. **Loopback cleartext, or refused**: nothing in this build
    /// terminates TLS, so `validate()` refuses `https` by name and refuses `http` to any host
    /// but 127.0.0.0/8, `localhost` or `[::1]`.
    /// The rule itself lived on the V1 `receipt::split_url`, which connected; D48 removed it,
    /// so nothing in this tree connects to this URL now.
    /// Omit this and `server_public_key` together for a service-free v5 installation.
    #[serde(default)]
    pub upload_url: String,
    /// The Ed25519 key this daemon verifies every `StartReceipt` and `AckCheckpoint`
    /// against, hex. Written into `trace.toml` as `server_public_key`. In development it
    /// is `goalcup_protocol::receipt::PLACEHOLDER_SERVER_PUBLIC_KEY_HEX`; in a signed release it is the
    /// event backend's. `validate()` refuses anything that is not 64 hex characters,
    /// because an unparseable key would make every acknowledgement silently unverifiable.
    ///
    /// **The development key is a published placeholder; the daemon says so and does not
    /// refuse it.** Its seed is a constant in this repository, so anybody can sign a receipt
    /// and an acknowledgement under it and a run anchored to it is anchored to nothing. The
    /// verifier reports that as V13's `placeholder_server_key` and the devserver banners it on
    /// every start; the daemon prints one line on stderr at startup
    /// ([`DaemonConfig::placeholder_key_notice`]). It does not refuse, because refusing would
    /// break every development run in this tree. Read as: a `server_public_key` equal to
    /// `goalcup_protocol::receipt::PLACEHOLDER_SERVER_PUBLIC_KEY_HEX` (the key of the
    /// devserver's `DEV_SERVER_SEED`) names a trace that a verifier will
    /// flag, and only `--dev` suppresses the flag.
    /// Omitted together with `upload_url`, this is empty and only local v5 entries are admitted.
    #[serde(default)]
    pub server_public_key: String,

    /// The receipt contract copied onto each newly-created registry row.
    /// Existing rows retain their stored policy across restart and configuration changes.
    #[serde(default)]
    pub receipt_binding: ReceiptBindingPolicy,

    /// Tailing cadence. 250 ms is the measured one: spike 5 kept up with 2,371 records.
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,
    #[serde(default = "default_upload_interval_ms")]
    pub upload_interval_ms: u64,
    /// The most envelopes one signed chunk carries.
    #[serde(default = "default_upload_chunk_max")]
    pub upload_chunk_max: usize,
    #[serde(default = "default_status_interval_ms")]
    pub status_interval_ms: u64,
    /// Silence from every store for this long is `HealthState::CaptureStalled`.
    #[serde(default = "default_stall_after_seconds")]
    pub stall_after_seconds: f64,
    /// Not read: kept so an existing config that sets it still loads under
    /// `deny_unknown_fields`.
    #[serde(default = "default_silence_window_seconds")]
    pub silence_window_seconds: f64,
    /// An unacknowledged head this old is `HealthState::UploadLagging`.
    #[serde(default = "default_ack_lag_seconds")]
    pub ack_lag_seconds: f64,
    /// The **second** step of the arming-timeout precedence chain documented once on
    /// `manifest::Arming`: connector default, then this per-machine override, then
    /// `WorkspaceOpts.arming_timeout_seconds` from the CLI. `None` means "leave the
    /// connector's number alone", which is the normal case — so it is the default, and a
    /// machine that sets it is deliberately overriding every connector at once.
    #[serde(default)]
    pub arming_timeout_seconds: Option<u64>,
    /// How often an `EventBody::Heartbeat` is chained. `ConnectorManifest.heartbeat_seconds`
    /// overrides it per connector. Absence of heartbeats over a window is what makes
    /// downtime a positive statement in the verdict rather than an absence.
    #[serde(default = "default_heartbeat_seconds")]
    pub heartbeat_seconds: u64,
    /// How far the server's `acked_at` may diverge from local time before a `clock_skew`
    /// health detail is chained.
    #[serde(default = "default_clock_skew_seconds")]
    pub clock_skew_seconds: f64,
    /// The free-space floor on the artifact-root volume below which the daemon degrades
    /// to `HealthState::DiskPressure` — *before* `append` starts failing, because a
    /// buffer that runs out of disk mid-run tells the participant nothing.
    #[serde(default = "default_disk_reserve_bytes")]
    pub disk_reserve_bytes: u64,

    /// The round's own dates, when this machine has been told them.
    ///
    /// Absent is the normal case and the default. What it costs is on [`EventConfig`]: with
    /// no `[event]` section every signed status document reports the entry's own timeline in
    /// the competition's fields, and `StatusPayload::event_status_source` says `fallback` so
    /// a reader can tell the two apart.
    #[serde(default)]
    pub event: Option<EventConfig>,
}

/// `[event]` — the competition-wide dates the signed status document carries.
///
/// # Why this is configuration and not something the daemon can work out
///
/// `EventStatus` is the one part of the status document an agent may trust: it asks the MCP
/// server when the start window closes and is answered with a number signed by the run key.
/// The daemon has no way to know that number, so these three fields are how a machine is
/// told, and `event_status_source` on the status document is how a reader learns whether it
/// was. Without them the document would assert three numbers nobody had supplied.
///
/// `publication_closes_at` is deliberately not here. It is the entry's own `deadline_at`,
/// which the start receipt fixes and which is immutable; a per-machine file that could
/// name a later one would be a participant choosing their own deadline.
///
/// All three are required together. Two of the three would leave the document mixing a
/// configured date with an invented one under one `event_status_source`, and there is no
/// reading of that a participant could act on.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventConfig {
    /// When the round opens, RFC 3339 with a `Z`: `2026-10-01T00:00:00Z`.
    pub t0: String,
    /// The last moment an entry may start. `validate()` refuses a value below `t0`.
    pub start_window_closes_at: String,
    /// The rulebook this round is judged under — the organiser's string, not this build's
    /// version, which says which binary wrote the document and nothing about which rules apply.
    pub rules_version: String,
}

/// Which optional provenance bindings a newly-created entry requires on its start receipt.
///
/// This selects a receipt contract only. It does not activate cutoff, publication, or a
/// questionnaire presentation service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReceiptBindingPolicy {
    #[default]
    Legacy,
    NewEvent {
        questionnaire_version: String,
    },
    /// An explicitly configured V4 run. No default selects this arm: all receipt,
    /// submission, and resource inputs must be present before an entry can latch them.
    WorkspaceSubmission {
        questionnaire_version: String,
        submission_profile: SubmissionProfileV1,
        limit_profile: LimitProfileV1,
        effective_limits: EffectiveSubmissionLimitsV1,
        capture_limits: CaptureResourceLimitsV1,
    },
}

impl<'de> Deserialize<'de> for ReceiptBindingPolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
        enum ClosedPolicy {
            // A struct variant rather than a unit variant: serde ignores extra map fields
            // on an internally tagged unit variant even under `deny_unknown_fields`.
            Legacy {},
            NewEvent {
                questionnaire_version: String,
            },
            WorkspaceSubmission {
                questionnaire_version: String,
                submission_profile: SubmissionProfileV1,
                limit_profile: LimitProfileV1,
                effective_limits: EffectiveSubmissionLimitsV1,
                capture_limits: CaptureResourceLimitsV1,
            },
        }
        Ok(match ClosedPolicy::deserialize(deserializer)? {
            ClosedPolicy::Legacy {} => Self::Legacy,
            ClosedPolicy::NewEvent {
                questionnaire_version,
            } => Self::NewEvent {
                questionnaire_version,
            },
            ClosedPolicy::WorkspaceSubmission {
                questionnaire_version,
                submission_profile,
                limit_profile,
                effective_limits,
                capture_limits,
            } => Self::WorkspaceSubmission {
                questionnaire_version,
                submission_profile,
                limit_profile,
                effective_limits,
                capture_limits,
            },
        })
    }
}

impl ReceiptBindingPolicy {
    fn validate_config(&self) -> Result<(), DaemonError> {
        if let Self::NewEvent {
            questionnaire_version,
        }
        | Self::WorkspaceSubmission {
            questionnaire_version,
            ..
        } = self
        {
            goalcup_protocol::provenance::validate_questionnaire_version(questionnaire_version)
                .map_err(|_| {
                    DaemonError::Config(
                        "receipt_binding.questionnaire_version is not a valid bounded version"
                            .to_string(),
                    )
                })?;
        }
        if let Self::WorkspaceSubmission {
            limit_profile,
            effective_limits,
            capture_limits,
            ..
        } = self
        {
            if limit_profile.id.is_empty()
                || limit_profile.id.len() > 256
                || limit_profile.id.bytes().any(|byte| byte.is_ascii_control())
            {
                return Err(DaemonError::Config(
                    "receipt_binding.limit_profile.id must be bounded non-control text".to_string(),
                ));
            }
            for (name, value) in [
                ("object_count", effective_limits.object_count),
                ("total_bytes", effective_limits.total_bytes),
                ("pre_cutoff_drain", effective_limits.pre_cutoff_drain),
                ("process_walk", effective_limits.process_walk),
            ] {
                if value == 0 {
                    return Err(DaemonError::Config(format!(
                        "receipt_binding.effective_limits.{name} must be greater than zero"
                    )));
                }
            }
            capture_limits.validate_for_config().map_err(|reason| {
                DaemonError::Config(format!("receipt_binding.capture_limits {reason}"))
            })?;
        }
        Ok(())
    }
}

impl EventConfig {
    /// The two dates, in seconds, or the reason they cannot be read.
    ///
    /// `parse_z` is the protocol's own reader, the same one every comparison of two of these
    /// strings goes through, so a date this daemon accepts is a date the status document's
    /// readers can parse.
    fn window(&self) -> Result<(u64, u64), DaemonError> {
        let read = |name: &str, value: &str| {
            goalcup_protocol::control::parse_z(value).ok_or_else(|| {
                DaemonError::Config(format!(
                    "[event] {name} = {value:?} is not an RFC 3339 instant this build can \
                     read ({{YYYY-MM-DDTHH:MM:SSZ}}): the status document is SIGNED, and a \
                     date its readers cannot parse is a signature over nothing they can use"
                ))
            })
        };
        Ok((
            read("t0", &self.t0)?,
            read("start_window_closes_at", &self.start_window_closes_at)?,
        ))
    }
}

fn default_poll_interval_ms() -> u64 {
    250
}
fn default_upload_interval_ms() -> u64 {
    2_000
}
fn default_upload_chunk_max() -> usize {
    256
}
fn default_status_interval_ms() -> u64 {
    1_000
}
fn default_stall_after_seconds() -> f64 {
    900.0
}
fn default_silence_window_seconds() -> f64 {
    60.0
}
fn default_ack_lag_seconds() -> f64 {
    300.0
}
fn default_heartbeat_seconds() -> u64 {
    60
}
fn default_clock_skew_seconds() -> f64 {
    120.0
}
fn default_disk_reserve_bytes() -> u64 {
    256 << 20
}

impl DaemonConfig {
    /// The legacy service is configured only by supplying both existing fields.
    /// `validate` separately refuses partial pairs and validates their URL and key.
    pub(crate) fn has_service(&self) -> bool {
        !self.upload_url.is_empty() && !self.server_public_key.is_empty()
    }

    /// The one stderr line the daemon prints at startup when `server_public_key` is the
    /// published placeholder, or `None` for any other key (and for none at all).
    ///
    /// Compared without regard to hex case. The text names what the placeholder costs: the
    /// seed is public, so every receipt and acknowledgement under it can be forged, until the
    /// project server's real key is configured (V1.1). A line, not a refusal: see the field.
    pub fn placeholder_key_notice(&self) -> Option<String> {
        self.server_public_key
            .eq_ignore_ascii_case(goalcup_protocol::receipt::PLACEHOLDER_SERVER_PUBLIC_KEY_HEX)
            .then(|| {
                "server_public_key is the published placeholder key: its seed is public, so \
                 every receipt and acknowledgement under it can be forged until the project \
                 server's key replaces it (V1.1)"
                    .to_string()
            })
    }

    /// Parse without touching the filesystem. `validate()` is a separate call, so a
    /// caller can load a config in order to report what is wrong with it.
    pub fn from_toml_str(s: &str) -> Result<DaemonConfig, DaemonError> {
        toml::from_str(s).map_err(|e| DaemonError::Toml(e.to_string()))
    }

    /// Read and parse `path`. The read is `map_err`'d with the path, not `?`'d: a daemon
    /// that says "No such file or directory" and not which one is a daemon nobody can
    /// configure.
    pub fn load(path: &Path) -> Result<DaemonConfig, DaemonError> {
        let text = std::fs::read_to_string(path).map_err(|source| DaemonError::Io {
            path: path.display().to_string(),
            source,
        })?;
        DaemonConfig::from_toml_str(&text)
    }

    /// What `validate` refuses.
    ///
    /// - **Loopback only.** Three control routes are unauthenticated by design
    ///   (`is_public_route`), and what makes that safe is that the socket is not
    ///   reachable off the machine. Stated here as a refusal so it is an enforced
    ///   property rather than a hope.
    /// - **A complete service pair, or neither field for local v5 capture.** Partial
    ///   configuration cannot activate legacy creation or recovery.
    /// - **A parsable supplied server key.** An unparseable one would make every `StartReceipt`
    ///   and `AckCheckpoint` silently unverifiable — the daemon would accept whatever
    ///   arrived and record it as acknowledged.
    /// - **An upload url this build can honestly reach**. Cleartext
    ///   `http` only to a loopback host, and `https` refused by name rather than silently
    ///   downgraded by the client that has to speak to it. Checked here, at load, because
    ///   a machine configured to ship a run's evidence over a network in the clear should
    ///   learn that before the run and not after it.
    /// - **A workspace parent that is not entangled with the artifact root**. Agents
    ///   enumerate the workspace's parent directory on their first tool call, so
    ///   a `workspace_parent` that contains — or sits inside — the artifact root puts the
    ///   run's own evidence in the agent's field of view.
    pub fn validate(&self) -> Result<(), DaemonError> {
        self.receipt_binding.validate_config()?;
        if !self.control_addr.ip().is_loopback() {
            return Err(DaemonError::Config(format!(
                "control_addr {} is not loopback: the control API's three unauthenticated \
                 read routes are safe only because the socket is not reachable off this machine",
                self.control_addr
            )));
        }

        if self.upload_url.is_empty() != self.server_public_key.is_empty() {
            return Err(DaemonError::Config(
                "upload_url and server_public_key must be supplied together, or both omitted \
                 for local v5 capture"
                    .to_string(),
            ));
        }
        if self.has_service() {
            let key = &self.server_public_key;
            if key.len() != 64 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(DaemonError::Config(format!(
                    "server_public_key must be 64 hex characters (an Ed25519 public key), and this \
                     one is {} character(s): an unparseable key would make every acknowledgement \
                     silently unverifiable",
                    key.len()
                )));
            }
        }

        for (name, p) in [
            ("artifact_root", &self.artifact_root),
            ("connectors_dir", &self.connectors_dir),
            ("config_dir", &self.config_dir),
            ("workspace_parent", &self.workspace_parent),
        ] {
            if !p.is_absolute() {
                return Err(DaemonError::Config(format!(
                    "{name} must be an absolute path, and it is {}: a daemon that resolves a \
                     root against its own working directory writes somewhere different \
                     depending on who started it",
                    p.display()
                )));
            }
        }

        if nests(&self.workspace_parent, &self.artifact_root) {
            return Err(DaemonError::Config(format!(
                "workspace_parent {} and artifact_root {} are nested: an agent enumerates its \
                 workspace's parent on its first tool call, so the run's own evidence would be \
                 in its field of view (R3)",
                self.workspace_parent.display(),
                self.artifact_root.display()
            )));
        }

        if self.poll_interval_ms == 0 {
            return Err(DaemonError::Config(
                "poll_interval_ms must be greater than zero".to_string(),
            ));
        }
        if self.upload_chunk_max == 0 {
            return Err(DaemonError::Config(
                "upload_chunk_max must be greater than zero: a chunk carrying no envelopes \
                 never drains the buffer"
                    .to_string(),
            ));
        }
        if self.has_service()
            && !self.upload_url.starts_with("http://")
            && !self.upload_url.starts_with("https://")
        {
            return Err(DaemonError::Config(format!(
                "upload_url must be an http or https URL, and it is {:?}",
                self.upload_url
            )));
        }
        // The round's dates, when this machine has been told them. Both readable, and in
        // the one order that means anything: a start window that closed before the round
        // opened is a window no entry could have started in, and the daemon would sign that
        // statement for every participant who asked.
        if let Some(event) = &self.event {
            let (t0, closes) = event.window()?;
            if t0 > closes {
                return Err(DaemonError::Config(format!(
                    "[event] t0 {} is after start_window_closes_at {}: no entry could start \
                     in that window, and this daemon signs the numbers it is given",
                    event.t0, event.start_window_closes_at
                )));
            }
        }

        Ok(())
    }
}

/// Whether either path is the other, or contains the other. Component-wise, never a
/// substring test: `/tmp/gc/artifacts-old` does not sit inside `/tmp/gc/artifacts`.
///
/// Compared twice, because D5 is a statement about the directory the agent actually
/// enumerates and not about how the two roots were spelled. First lexically, so
/// `/tmp/other/../gc/artifacts/ws` is recognised as sitting inside `/tmp/gc/artifacts`
/// even when nothing on either path exists yet; then, as far as the filesystem will
/// resolve them, so a `workspace_parent` symlinked into the artifact root is recognised
/// too. The two passes disagree, and that is the point: a `..` that crosses a symlink
/// does not mean what it reads as, so the lexical answer for `link/../ws` is not the
/// directory the agent gets. Neither pass is a refinement of the other, which is why
/// both run and why either one finding an entanglement is enough. This refuses, and a
/// refusal that is occasionally too eager costs a participant one edited path, where a
/// miss puts the run's own evidence in the agent's field of view.
///
/// `pub(crate)` because [`crate::workspace::check_artifact_placement`] applies it a
/// second time, to the two *concrete* directories of one entry rather than to the
/// machine-wide roots. That check has clauses this one does not — it names the
/// direction and it refuses a sibling — but the symlink pass belongs here and is
/// reused, not copied.
pub(crate) fn nests(a: &Path, b: &Path) -> bool {
    if overlaps(&lexically_normalized(a), &lexically_normalized(b)) {
        return true;
    }
    overlaps(
        &resolved_as_far_as_possible(a),
        &resolved_as_far_as_possible(b),
    )
}

/// One direction or the other, on paths already normalised by the caller.
fn overlaps(a: &Path, b: &Path) -> bool {
    a == b || a.starts_with(b) || b.starts_with(a)
}

/// Resolve `.` and `..` textually, touching nothing on disk. A purely lexical pass is
/// wrong in general — `..` after a symlink does not mean what it reads as — which is
/// exactly why it is only half of `nests()`; on its own it is the half that works
/// against a root that does not exist yet, which is the normal case on first run.
fn lexically_normalized(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for component in p.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match out.components().next_back() {
                // `/..` is `/`: there is nothing above a root to walk up to.
                Some(Component::RootDir) | Some(Component::Prefix(_)) => {}
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                // A leading `..` on a relative path is meaningful and is kept. Absolute
                // paths are all `validate()` ever passes here, so this is defensive.
                _ => out.push(component.as_os_str()),
            },
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

/// `canonicalize` the longest prefix of `p` that exists — which is what follows
/// symlinks — and re-attach the components below it. Canonicalising the whole path
/// would fail on a root the daemon has not created yet and leave the symlink
/// unresolved; resolving the prefix still catches the case that matters, where the
/// link is a directory somewhere above.
///
/// The prefix is taken from `p` **as written**, never from its lexical normalisation,
/// and that ordering is the whole correctness of this function. `..` after a symlink
/// walks up from the link's *target*, not from where the link sits: with
/// `/tmp/gc/link -> /tmp/gc/artifacts/inner`, the kernel resolves `/tmp/gc/link/../ws`
/// to `/tmp/gc/artifacts/ws`, inside the artifact root. Normalising first collapses that
/// `..` against `link` and yields `/tmp/gc/ws`, a directory that has nothing to do with
/// the one the agent would enumerate — so the entanglement is erased before the
/// filesystem is ever asked. Handing the raw path to `canonicalize` lets the kernel
/// resolve the `..` itself, with the symlink in view.
///
/// Only the components past the longest existing prefix are resolved lexically, and by
/// construction they name nothing on disk, so no symlink can be hiding under them. The
/// canonical base has none left in it either, which is what makes popping for `..`
/// sound here where it was not sound on the raw path.
fn resolved_as_far_as_possible(p: &Path) -> PathBuf {
    use std::path::Component;
    let components: Vec<Component> = p.components().collect();
    // Longest prefix first: the whole path when every component of it exists.
    for cut in (1..=components.len()).rev() {
        let mut prefix = PathBuf::new();
        for component in &components[..cut] {
            prefix.push(component.as_os_str());
        }
        let Ok(base) = std::fs::canonicalize(&prefix) else {
            continue;
        };
        let mut out = base;
        for component in &components[cut..] {
            match component {
                Component::CurDir => {}
                // `pop()` on a root is a no-op, which is the wanted `/..` is `/`.
                Component::ParentDir => {
                    out.pop();
                }
                other => out.push(other.as_os_str()),
            }
        }
        return out;
    }
    // Nothing on this path exists. The lexical comparison is all there is.
    lexically_normalized(p)
}

#[cfg(test)]
mod workspace_submission_tests {
    use super::ReceiptBindingPolicy;
    use crate::submission_types::CaptureResourceLimitsV1;
    use goalcup_protocol::submission::{
        EffectiveSubmissionLimitsV1, LimitProfileV1, SubmissionProfileV1,
    };

    #[test]
    fn v4_requires_an_explicit_complete_profile_and_limits() {
        let policy: ReceiptBindingPolicy = toml::from_str(
            r#"
                mode = "workspace_submission"
                questionnaire_version = "goal-cup-2026-v1"
                submission_profile = "workspace_snapshot_v1"

                [limit_profile]
                id = "goal-cup-v1-local"

                [effective_limits]
                object_count = 4096
                total_bytes = 1073741824
                pre_cutoff_drain = 5000
                process_walk = 64

                [capture_limits]
                version = 1
                namespace_nodes = 8192
                inventory_bytes = 8388608
                manifest_bytes = 8388608
            "#,
        )
        .expect("an explicit V4 policy parses");
        assert_eq!(
            policy,
            ReceiptBindingPolicy::WorkspaceSubmission {
                questionnaire_version: "goal-cup-2026-v1".to_string(),
                submission_profile: SubmissionProfileV1::WorkspaceSnapshotV1,
                limit_profile: LimitProfileV1 {
                    id: "goal-cup-v1-local".to_string(),
                },
                effective_limits: EffectiveSubmissionLimitsV1 {
                    object_count: 4096,
                    total_bytes: 1_073_741_824,
                    pre_cutoff_drain: 5000,
                    process_walk: 64,
                },
                capture_limits: CaptureResourceLimitsV1 {
                    version: 1,
                    namespace_nodes: 8192,
                    inventory_bytes: 8_388_608,
                    manifest_bytes: 8_388_608,
                },
            }
        );

        for missing in [
            "submission_profile = \"workspace_snapshot_v1\"",
            "[limit_profile]\nid = \"goal-cup-v1-local\"",
            "[effective_limits]",
            "process_walk = 64",
            "[capture_limits]",
            "manifest_bytes = 8388608",
        ] {
            let incomplete = r#"
mode = "workspace_submission"
questionnaire_version = "goal-cup-2026-v1"
submission_profile = "workspace_snapshot_v1"
[limit_profile]
id = "goal-cup-v1-local"
[effective_limits]
object_count = 4096
total_bytes = 1073741824
pre_cutoff_drain = 5000
process_walk = 64
[capture_limits]
version = 1
namespace_nodes = 8192
inventory_bytes = 8388608
manifest_bytes = 8388608
            "#
            .replace(missing, "");
            assert!(
                toml::from_str::<ReceiptBindingPolicy>(&incomplete).is_err(),
                "missing {missing:?} must not fall back to an implicit qualifying value",
            );
        }

        for (field, invalid) in [
            ("version = 1", "version = 2"),
            ("namespace_nodes = 8192", "namespace_nodes = 0"),
            ("namespace_nodes = 8192", "namespace_nodes = 1000001"),
            ("inventory_bytes = 8388608", "inventory_bytes = 67108865"),
            ("manifest_bytes = 8388608", "manifest_bytes = 67108865"),
        ] {
            let invalid: ReceiptBindingPolicy = toml::from_str(
                &r#"
mode = "workspace_submission"
questionnaire_version = "goal-cup-2026-v1"
submission_profile = "workspace_snapshot_v1"
[limit_profile]
id = "goal-cup-v1-local"
[effective_limits]
object_count = 4096
total_bytes = 1073741824
pre_cutoff_drain = 5000
process_walk = 64
[capture_limits]
version = 1
namespace_nodes = 8192
inventory_bytes = 8388608
manifest_bytes = 8388608
            "#
                .replace(field, invalid),
            )
            .expect("the shape parses before semantic validation");
            assert!(
                invalid.validate_config().is_err(),
                "invalid capture resource limit {invalid:?} must refuse before entry creation",
            );
        }
    }
}
