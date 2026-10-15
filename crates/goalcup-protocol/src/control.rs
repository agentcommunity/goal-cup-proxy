//! The local control API between `goalcup-cli` and `goalcup-daemon`.
//!
//! This module is the contract. The daemon implements the server side; the
//! CLI implements `DaemonClient`. Neither crate owns it, so it lives here.
//!
//! Transport in V4 is HTTP-only at `http://127.0.0.1:<port>`, with the loopback port read
//! from `<config_dir>/control.json`. Every response carries `Content-Length`; there is no
//! chunked encoding and no keep-alive requirement, so a small client is enough.
//! Every request carries `Authorization: Bearer <control_token>` **except the one public
//! route `is_public_route` names** — `GET /v0/entries/{id}/status`, which is
//! unauthenticated, loopback-only and read-only.
//! Write the middleware from `is_public_route`, never from this sentence.
//!
//! Every request, public or not, carries exactly one `Host` that names the listener itself:
//! `127.0.0.1:<port>`, `localhost:<port>` or `[::1]:<port>`, the port being the one in
//! `control.json`. Any other `Host` is refused `401 unauthorized` before the public/bearer
//! split, so a page reaching the daemon through DNS rebinding reads nothing. A client that
//! builds its URL from `control.json` sends a matching `Host` without trying.

use crate::ProtocolError;
use crate::events::HealthState;
use crate::provenance::{PreCutoffDrainOutcomeV1, TerminationOutcomeV1};
use crate::seal::SealManifest;
use crate::submission::SubmissionOutcomeV1;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Domain tag prefixed to every signed status's signing bytes, in the shape
/// `chain::CHAIN_DOMAIN` established. Version-tagged, so a status's bytes can never be
/// presented as a receipt's or an acknowledgement's, and a v2 status can never be read as
/// a v1 one.
pub const STATUS_DOMAIN: &[u8] = b"goalcup-status-v1\0";

/// Bumped when a route or a field changes meaning. `DaemonHealth` reports it so the
/// CLI can refuse a daemon it does not understand rather than mis-reading its JSON.
/// V4 is deliberately not compatible with V3 clients: the seal route returns the daemon's
/// durable workspace-submission and process-termination outcomes, and a V3 CLI would
/// interpret that response as a live-scan manifest and could report bytes outside the
/// daemon's cutoff. This is a coordinated pre-release upgrade boundary, not support for
/// replacing a daemon under an already-running V3 CLI.
/// V5 adds the required connector-directory digest to capture workspace creation, so two
/// separately installed halves cannot silently create a run from different connector bytes.
/// This is only the local control boundary; it does not change `M2_PROTOCOL_VERSION`.
/// V6 removes the retired V1 hook-path shapes: `WorkspaceInfo`'s `server_port`,
/// `installed_files` and `workspace_relative`, the always-constant `StatusPayload` fields
/// `arming_recipe`, `posts_dropped`, `posts_dropped_after_seal` and `pins_dropped`, the
/// `arming_timeout` and `unsupported_connector` error codes, and `ProcessHandle.home_seed`.
/// An unknown connector is now refused `preflight_failed`.
/// V7 lets a capture create carry an upload server's `entry_id`, and adds `chain_head` and
/// `trace_dir` to its answer (docs/UPLOAD-API.md).
/// V8 drops the V1 vocabulary nothing produced: `WorkspaceInfo`, `GET /v0/entries/current`,
/// `StatusPayload.event` and `event_status_source`, the `ArmingState` values
/// `start_signal_seen` and `refused`, and every `HealthState` but `online`, `sealed` and
/// `sealing_recovery_needed`. The CLI and the daemon ship together, so a pair from two
/// releases is refused rather than bridged.
pub const CONTROL_API_VERSION: u32 = 8;

pub const ROUTE_ENTRIES: &str = "/v0/entries";
pub const ROUTE_HEALTH: &str = "/v0/health";
/// `POST` a [`CaptureWorkspaceRequest`]: create a protocol-v5 local capture entry.
pub const ROUTE_CAPTURE_WORKSPACES: &str = "/v0/capture-workspaces";

/// A status document older than this is reported as **stale** by `goalcup status`. One
/// number, here, so any later reader of the status document uses the same threshold.
pub const STALE_AFTER_SECONDS: u64 = 120;

/// Route suffixes, as constants rather than as string literals typed on both sides.
/// `entry_route(id, SUFFIX_SEAL)` on the client and the same call in
/// the server's router mean a typo is a compile error, not a 404 the other side reports
/// as "the daemon is not reachable".
///
/// They live HERE, in the crate neither the daemon nor the CLI owns, because a route
/// string spelled in two crates is a route string that drifts. The daemon's router
/// re-exports these names rather than defining its own: one definition site, or the
/// drift is back with a `pub use` in front of it.
///
/// The one suffix that is also a PUBLIC route.
pub const SUFFIX_STATUS: &str = "status";

/// See [`SUFFIX_STATUS`].
pub const SUFFIX_PROCESS: &str = "process";

/// See [`SUFFIX_STATUS`].
pub const SUFFIX_ABANDON: &str = "abandon";

/// See [`SUFFIX_STATUS`].
pub const SUFFIX_SEAL: &str = "seal";

/// See [`SUFFIX_STATUS`]. A v5 entry's one-shot goal ([`CaptureGoalRequest`]).
pub const SUFFIX_CAPTURE_GOAL: &str = "capture/goal";

/// See [`SUFFIX_STATUS`]. A v5 entry's launch, recorded after spawn ([`CaptureLaunchRequest`]).
pub const SUFFIX_CAPTURE_LAUNCH: &str = "capture/launch";

/// `/v0/entries/{id}/{suffix}`.
pub fn entry_route(entry_id: &str, suffix: &str) -> String {
    format!("{ROUTE_ENTRIES}/{entry_id}/{suffix}")
}

/// The authentication ruling above, as one function.
///
/// `true` for exactly the one route the ruling names as unauthenticated, loopback-only
/// and read-only, and `false` for everything else. (`/v0/assets/*` and the V1 page `/v0/ui`
/// were public too until nothing served them: a route with nothing behind it is public
/// surface for no reader.) The daemon's auth middleware skips the
/// token on a `true`; the CLI's `HttpDaemonClient` omits the `Authorization` header on a
/// `true`. **Both call this function**, so the server and the client cannot drift into
/// disagreeing about which routes need a token.
///
/// `/v0/entries` is deliberately **not** public: reading one entry's signed status when you
/// already know its id is accepted, enumeration is not.
///
/// A path carrying a `.` or `..` segment is never public. `/v0/entries/e1/status/../../seal`
/// carries the public route's text and resolves, at every normalising client and proxy
/// between here and the socket, to an authed one — so matching on the unresolved text would
/// hand the authed surface a free pass. (The rule was written for the `/v0/assets/` prefix, whose
/// `/v0/assets/../entries/e1/seal` was the sharpest case; the prefix is gone and the rule
/// stays.) The rule is stated as a refusal
/// rather than a normalisation: this function decides authentication, and a function that
/// decides authentication must not be the one that rewrites the path it is deciding about.
///
/// The refusal covers the percent-encoded spellings too: `/v0/entries/%2e%2e/status`
/// is the same traversal written in the alphabet a URL decoder speaks, and a decoder sits
/// between this function and the filesystem on every path that matters. A segment is
/// refused when its percent-decoding is `.` or `..`, and a path is refused outright when it
/// carries a `%` that is not a well-formed `%XX` escape — an undecodable path is one this
/// function cannot reason about, and the safe answer to "what does this resolve to" is not
/// "it is public".
pub fn is_public_route(path: &str) -> bool {
    // Query strings and fragments are not part of the routing decision.
    let path = path.split(['?', '#']).next().unwrap_or(path);
    for seg in path.split('/') {
        match percent_decode(seg) {
            // A `%` that does not open a valid escape: refuse rather than guess.
            None => return false,
            Some(decoded) => {
                if matches!(decoded.as_slice(), b"." | b"..") {
                    return false;
                }
            }
        }
    }
    // `/v0/entries/{id}/status` — and nothing else under `/v0/entries/`.
    match path.strip_prefix("/v0/entries/") {
        Some(rest) => match rest.strip_suffix("/status") {
            // A non-empty id with no further slash in it.
            Some(id) => !id.is_empty() && !id.contains('/'),
            None => false,
        },
        None => false,
    }
}

/// Percent-decode one path segment's `%XX` escapes, or `None` when a `%` is not followed
/// by two hex digits. Deliberately minimal and dependency-free: `is_public_route` only
/// needs to know whether a segment spells `.` or `..`, and the bytes it yields are never
/// used to open anything.
fn percent_decode(seg: &str) -> Option<Vec<u8>> {
    fn hex(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
    let bytes = seg.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hi = hex(*bytes.get(i + 1)?)?;
            let lo = hex(*bytes.get(i + 2)?)?;
            out.push((hi << 4) | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Some(out)
}

// ------------------------------------------------------------------ responses

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArmingState {
    /// Created, and the harness not launched yet.
    Armed,
    /// The harness was launched.
    Started,
}

/// What the daemon's own loop lost, per cadence, and what the last failure said.
///
/// The daemon runs four timers — the tick, the pump, the status write and the deadline
/// check — and none of them may end the process on a transient: a daemon that exited on the
/// first failure would take the capture with it. Without these counters a poll that failed
/// every cycle would look exactly like a quiet run from every surface a participant has.
///
/// **`last` never carries a filesystem path.** The status route is unauthenticated by
/// ruling and the status line is rendered into a terminal a screen recording may catch, so
/// the message is the daemon's own words with every path removed.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CycleErrors {
    /// Ticks — the ordered poll and the start binding — that ended in an error.
    pub tick: u64,
    /// Buffer drains that ended in an error.
    pub upload: u64,
    /// Status publications that ended in an error.
    pub status: u64,
    /// Deadline sweeps that ended in an error.
    pub deadline: u64,
    /// The last error any of the four raised, path-free. `None` while none has.
    pub last: Option<String>,
    /// When any of the four last completed a pass. `None` while none has.
    ///
    /// # One timestamp, not four counters
    ///
    /// Zero failure counts on a document the daemon re-signs every second read the same for
    /// a daemon whose loops are healthy and one whose loops have stopped. The question a
    /// participant asks is "are the loops turning?", and a timestamp answers it in one
    /// reading: a stamp from a second ago is a live daemon, an old stamp is a stopped one,
    /// and `None` is one that has not run yet. Per-cadence pass counts answer "how many
    /// times each" — a number nobody can act on, four fields wide, on a document and a
    /// status line that a participant reads.
    ///
    /// The four cadences share ONE stamp deliberately. They are the same daemon loop, and a
    /// per-cadence stamp would invite a reader to diff them; nothing in the daemon acts on
    /// the difference, and the tick is by far the most frequent, so the maximum is what
    /// "the loops are turning" means.
    ///
    /// `serde(default)`, so a cached document written before it reads `None` — which is
    /// what those bytes say: that daemon did not stamp.
    #[serde(default)]
    pub last_pass_at: Option<String>,
}

/// Read-only projection of the registry's durable V4 finalization intent.
///
/// This deliberately exposes only the three participant-actionable states, not a second
/// copy of the daemon's recovery journal. `Incomplete` carries whichever exact outcomes
/// were already committed when progress stopped; `Terminal` requires both outcomes that
/// the finalization link binds. The status publisher derives this exclusively from the
/// persisted intent, never from a live workspace scan or current process absence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkspaceFinalizationStatusV1 {
    // Struct variants keep `deny_unknown_fields` effective even for the fieldless state.
    Finishing {},
    Incomplete {
        pre_cutoff_drain: Option<PreCutoffDrainOutcomeV1>,
        termination: Option<TerminationOutcomeV1>,
        submission: Option<SubmissionOutcomeV1>,
    },
    Terminal {
        pre_cutoff_drain: PreCutoffDrainOutcomeV1,
        termination: TerminationOutcomeV1,
        submission: SubmissionOutcomeV1,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusPayload {
    pub entry_id: String,
    pub connector_id: String,
    pub workspace: String,
    pub session_id: Option<String>,
    pub harness_version: Option<String>,
    pub arming: ArmingState,
    pub health: HealthState,
    pub started_at: Option<String>,
    pub deadline_at: Option<String>,
    /// When the chain closed, from the entry's own row. `None` while the run is open.
    /// Additive: `health` reads `sealed` on exactly the documents that carry
    /// it, and `seconds_remaining` is `0` on them — a sealed run's countdown has stopped,
    /// and a surface that went on counting down was telling a participant to hurry over a
    /// run that had ended.
    #[serde(default)]
    pub sealed_at: Option<String>,
    pub seconds_remaining: Option<u64>,
    pub last_sync_at: Option<String>,
    pub events_uploaded: u64,
    pub events_pending: u64,
    /// The health state in plain words, for the status page and the terminal (D25).
    pub detail: String,
    /// What the daemon's own four cadences lost while this entry was open.
    ///
    /// `#[serde(default)]` so a cached document written by a build that did not have this
    /// field still loads: the cache directory outlives an upgrade, and a status a reader
    /// refuses is worse than one whose counters read zero.
    #[serde(default)]
    pub cycles: CycleErrors,
    /// How many times a tick stopped at the per-tick poll cap with a store still holding
    /// more (`MAX_POLLS_PER_TICK`).
    ///
    /// Cumulative for the run, not per tick: one capped tick is a busy moment and a hundred
    /// of them is a daemon that has been behind since the run started, and nothing in the
    /// chain tells the two apart — a capped tick looks exactly like a quiet one.
    #[serde(default)]
    pub polls_capped: u64,
    /// How many stores began this run at offset 0 because their saved position did not come
    /// back. On a resumed entry it equals the number of `Reread` witnesses chained before
    /// the first poll.
    #[serde(default)]
    pub cursor_restore_failures: u64,
    /// How many records the restart de-duplication budget was spent on: read again by this
    /// process, already in the chain from the last one, and therefore chained neither a
    /// second time nor counted as captured or discarded.
    ///
    /// A budget computed too wide is invisible from every other counter here, so this is the
    /// one place that residue surfaces.
    #[serde(default)]
    pub skipped_replayed: u64,
    /// The sequence number of the first hole in the write-ahead buffer, if the last pump
    /// found one and the chain could not answer for it.
    ///
    /// A reported fact and not a health state: a hole in the buffer is permanent, and the
    /// ladder has no rung for it. `None` is a buffer with no gap.
    #[serde(default)]
    pub buffer_gap_at: Option<u64>,
    /// How many polls of a whole-file store stopped at the seen-set's bound with files it
    /// had not captured still in front of it.
    ///
    /// Cumulative for the run, for the same reason `polls_capped` is. The store is perfectly
    /// readable and the daemon is simply out of room to remember another file, so nothing in
    /// the chain says it happened: the tailer's notice is a health signal that chains
    /// nothing, and every file past the bound is evidence that will not exist.
    #[serde(default)]
    pub files_capped: u64,
    /// The seal could not prove the managed harness process tree was fully and safely
    /// stopped when the chain ended. This covers both a guard/capability refusal that sent
    /// no signal and a best-effort numeric signal path that lacked instance-bound
    /// containment or an authenticated final proof.
    ///
    /// `false` on every run that has not sealed, and on a seal whose kill was verified. It
    /// is a fact about the ENDING, which is why it reaches a document only once the run has
    /// one.
    #[serde(default)]
    pub harness_unverified: bool,
    /// V1-V3 payloads omit this field byte-for-byte. V4 sets it only from the durable
    /// workspace intent so restart and the unauthenticated read-only route report the same
    /// finishing/incomplete/terminal fact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finalization: Option<WorkspaceFinalizationStatusV1>,
    /// The M2 capture projection. Every legacy payload omits it byte-for-byte:
    /// `None` is skipped on the wire and a document without the field decodes as `None`.
    /// Filled only by the pre-sign projection of a v5 entry, never by guessing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture: Option<crate::capture::M2Status>,
    /// A v5 entry's authenticated ingest producer IDs, from its receiver's own count (not
    /// the listener-wide handshake refusals). Absent from every legacy payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub producers_consumed: Option<u64>,
    /// A v5 entry's authenticated producer IDs its capture never admitted. With
    /// `producers_consumed`, this is what a launcher reads to learn that its producer was
    /// admitted before it spawns anything. Absent from every legacy payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admissions_refused: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedStatus {
    pub payload: StatusPayload,
    /// Hex Ed25519 verifying key: the run key from `CaptureWorkspaceInfo.run_public_key`.
    pub public_key: String,
    pub signature: String,
    /// When the daemon produced this document, RFC 3339 with a `Z`.
    pub cached_at: String,
}

impl SignedStatus {
    /// Canonical signing bytes:
    /// `STATUS_DOMAIN ++ {"cached_at":…,"payload":{…}}` with the payload's declaration
    /// order preserved by `serde_json`.
    pub fn signing_bytes(&self) -> Result<Vec<u8>, ProtocolError> {
        #[derive(Serialize)]
        struct Signed<'a> {
            cached_at: &'a str,
            payload: &'a StatusPayload,
        }
        let mut bytes = STATUS_DOMAIN.to_vec();
        bytes.extend_from_slice(&serde_json::to_vec(&Signed {
            cached_at: &self.cached_at,
            payload: &self.payload,
        })?);
        Ok(bytes)
    }

    pub fn sign(
        payload: StatusPayload,
        cached_at: String,
        key: &SigningKey,
    ) -> Result<SignedStatus, ProtocolError> {
        let mut s = SignedStatus {
            payload,
            public_key: hex::encode(key.verifying_key().to_bytes()),
            signature: String::new(),
            cached_at,
        };
        let sig: Signature = key.sign(&s.signing_bytes()?);
        s.signature = hex::encode(sig.to_bytes());
        Ok(s)
    }

    /// Verify against the run key the **caller** pinned — `CaptureWorkspaceInfo.run_public_key`,
    /// which the CLI holds from the entry record it pinned at creation.
    ///
    /// There is deliberately no `verify(&self)`. The status route is unauthenticated by
    /// ruling (`is_public_route`), so a local process that binds the control port first
    /// could serve a status signed with its own key; checking that document against the
    /// key the document itself carries would report a forged deadline as trusted. This
    /// method ignores `self.public_key` entirely — that field is a label, not a
    /// credential.
    pub fn verify_with(&self, run_public_key_hex: &str) -> Result<(), ProtocolError> {
        let pk: [u8; 32] = hex::decode(run_public_key_hex)
            .map_err(|e| ProtocolError::Signature(e.to_string()))?
            .try_into()
            .map_err(|_| ProtocolError::Signature("run key is not 32 bytes".into()))?;
        let sg: [u8; 64] = hex::decode(&self.signature)
            .map_err(|e| ProtocolError::Signature(e.to_string()))?
            .try_into()
            .map_err(|_| ProtocolError::Signature("signature is not 64 bytes".into()))?;
        let vk =
            VerifyingKey::from_bytes(&pk).map_err(|e| ProtocolError::Signature(e.to_string()))?;
        // `verify_strict`, not `verify`: it rejects non-canonical signature encodings and
        // small-order public keys, both of which let one signature verify under more than
        // one key.
        vk.verify_strict(&self.signing_bytes()?, &Signature::from_bytes(&sg))
            .map_err(|e| ProtocolError::Signature(e.to_string()))
    }

    /// Seconds between `cached_at` and `now`, both RFC 3339 with a trailing `Z`.
    /// `None` when either fails to parse, which the caller reports as "unknown"
    /// rather than as "fresh".
    pub fn stale_seconds(&self, now: &str) -> Option<u64> {
        let a = parse_z(&self.cached_at)?;
        let b = parse_z(now)?;
        Some(b.saturating_sub(a))
    }
}

/// Minimal RFC 3339 reader for `YYYY-MM-DDTHH:MM:SS[.fff]Z`, returning seconds since
/// 1970-01-01. Written out rather than pulling a date crate: the control API only ever
/// compares two of its own timestamps, and a wrong answer must be `None`, not a guess.
pub fn parse_z(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || *b.last()? != b'Z' {
        return None;
    }
    let num = |from: usize, to: usize| -> Option<i64> { s.get(from..to)?.parse::<i64>().ok() };
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, sec) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    // Days from civil (Howard Hinnant's algorithm), valid for the whole Gregorian range.
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
    let yoe = y2 - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let total = days * 86_400 + h * 3_600 + mi * 60 + sec;
    u64::try_from(total).ok()
}

/// The inverse of `parse_z`: seconds since 1970-01-01 as `YYYY-MM-DDTHH:MM:SSZ`.
///
/// Written out for the same reason `parse_z` is — this crate pulls no date library — using
/// Hinnant's civil-from-days, which is the algorithm `parse_z`'s days-from-civil block
/// inverts term for term.
///
/// **Sub-second precision is dropped by design.** `recorded_at` is read as a whole second
/// everywhere in this protocol: `parse_z` truncates, `Envelope::recorded_at_secs` returns
/// whole seconds, and a formatter that invented a fractional part would be claiming a
/// precision nothing downstream can check. A caller holding fractional time that matters
/// has to carry it somewhere other than this string.
pub fn format_z(unix_secs: u64) -> String {
    let days = (unix_secs / 86_400) as i64;
    let secs_of_day = unix_secs % 86_400;
    // Civil from days (Howard Hinnant), the exact inverse of the block above.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs_of_day / 3_600,
        (secs_of_day % 3_600) / 60,
        secs_of_day % 60
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntrySummary {
    pub entry_id: String,
    pub connector_id: String,
    pub workspace: String,
    pub arming: ArmingState,
    pub health: HealthState,
    pub started_at: Option<String>,
    pub deadline_at: Option<String>,
    pub sealed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonHealth {
    pub control_api_version: u32,
    pub daemon_version: String,
    pub uptime_seconds: u64,
    pub entries_open: u32,
    /// What the daemon's own loop lost, as the signed status document also carries it:
    /// counts and the last path-free message. The daemon counts here and does not judge.
    ///
    /// `serde(default)`, so an older daemon's answer still parses and reads as "nothing lost".
    #[serde(default)]
    pub cycles: CycleErrors,
    /// Ids of the capture entries this daemon failed to reopen at start. Such an entry has
    /// no status and routes nothing, so this is the one place an operator finds its id.
    /// Empty when none failed. `serde(default)` for the same reason as `cycles`.
    #[serde(default)]
    pub reopen_failed: Vec<String>,
}

/// The durable answer to a V4 workspace finalization request.
///
/// These are terminal facts recovered from the daemon's registry and trace. They are not
/// reconstructed by walking the live workspace, because that walk would occur after the
/// signed cutoff and could include bytes that never qualified for the submission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinishReportV1 {
    pub entry_id: String,
    pub trace_dir: String,
    pub termination: TerminationOutcomeV1,
    pub submission: SubmissionOutcomeV1,
}

/// Response body for `POST /v0/entries/{id}/seal`.
///
/// The legacy arm remains byte-for-byte compatible with the historical bare
/// [`SealManifest`]. A V4 workspace response is distinguished by its closed field set and
/// appears first so serde cannot discard workspace-only fields through the permissive
/// historical manifest shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SealResponse {
    Workspace(FinishReportV1),
    Legacy(SealManifest),
}

// ------------------------------------------------------------------ v5 local capture

/// `POST` [`ROUTE_CAPTURE_WORKSPACES`]: a protocol-v5 local capture entry — its workspace,
/// its physically reserved chain and its ingest authorization — with no server, grant or
/// receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureWorkspaceRequest {
    /// The absolute workspace directory, absent or empty and outside any Git worktree. The
    /// daemon creates it and knows the run by its canonical spelling.
    pub workspace: String,
    /// The connector whose `[capture]` profile the run is launched with.
    pub profile_id: String,
    /// The lowercase SHA-256 of the exact connector directory the launcher loaded. The daemon
    /// compares it with its own authoritative load before it allocates an entry or writes a byte.
    #[serde(deserialize_with = "lowercase_sha256")]
    pub connector_dir_sha256: String,
    /// The chain's logical byte quota, the terminal allowance included: admission accounting
    /// only, nothing is allocated ([`crate::capture::CHAIN_QUOTA_BYTES`] by default).
    pub reserve_bytes: u64,
    /// The absolute per-run `HOME` the launcher creates and gives the harness; the daemon
    /// reads `{home}/` stores only for controlled-workload families.
    pub run_home: String,
    /// The absolute tracker-isolated harness configuration home, shared across runs.
    pub config_home: String,
    /// The entry id the upload server issued for this run (`e` and 16 lowercase hex digits),
    /// which every record then carries. Absent for a local-only run: the daemon mints one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_id: Option<String>,
    /// The run's start, Unix seconds (UTC): a native source that declares `since_run_start`
    /// reads only the files modified or changed at or after it. Optional: without it the daemon
    /// reads from the entry's create time (D57); ignored by a source without `since_run_start`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_since: Option<u64>,
}

fn lowercase_sha256<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let digest = String::deserialize(deserializer)?;
    if digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        Ok(digest)
    } else {
        Err(serde::de::Error::custom(
            "connector_dir_sha256 must be 64 lowercase hex characters",
        ))
    }
}

/// The answer to [`CaptureWorkspaceRequest`]. `Debug` prints the ingest secret as
/// `<redacted>`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureWorkspaceInfo {
    pub entry_id: String,
    /// The generation `register_process` names for the harness this entry launches. It is
    /// accepted once, before `record_capture_launch`.
    pub process_generation: u64,
    /// The chain bytes physically reserved: always zero, since a chain is quota-only.
    pub reserved_bytes: u64,
    /// The daemon's loopback ingest endpoint.
    pub ingest_addr: std::net::SocketAddr,
    /// This entry's launch-scoped ingest credential, held in memory only: never argv,
    /// environment, disk or logs.
    pub ingest_secret: String,
    /// The fresh producer ID the proxy's first ingest epoch uses.
    pub producer_id: String,
    /// The run key every status document of this entry is signed with; the caller pins it
    /// and verifies against it with [`SignedStatus::verify_with`].
    pub run_public_key: String,
    /// The chain's head once the baseline is chained: the hash a start receipt signs.
    pub chain_head: String,
    /// The entry's trace directory, holding `chain.jsonl`.
    pub trace_dir: String,
}

impl std::fmt::Debug for CaptureWorkspaceInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CaptureWorkspaceInfo")
            .field("entry_id", &self.entry_id)
            .field("process_generation", &self.process_generation)
            .field("reserved_bytes", &self.reserved_bytes)
            .field("ingest_addr", &self.ingest_addr)
            .field("ingest_secret", &"<redacted>")
            .field("producer_id", &self.producer_id)
            .field("run_public_key", &self.run_public_key)
            .field("chain_head", &self.chain_head)
            .field("trace_dir", &self.trace_dir)
            .finish()
    }
}

/// Whether `id` is an entry id in the one shape this tree uses: `e` and 16 lowercase hex
/// digits. A server-issued id names a directory, so nothing else is accepted.
pub fn is_entry_id(id: &str) -> bool {
    id.len() == 17
        && id.starts_with('e')
        && id[1..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// `POST` [`SUFFIX_CAPTURE_GOAL`]: the initial goal, accepted once. The answer is the goal
/// stream's key, given only after the goal is redacted, appended and synced. `Debug` prints
/// the length, never the bytes.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureGoalRequest {
    /// The goal exactly as read: UTF-8, at most [`crate::capture::MAX_GOAL_BYTES`].
    pub bytes: Vec<u8>,
}

impl std::fmt::Debug for CaptureGoalRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CaptureGoalRequest")
            .field("bytes", &format_args!("<{} bytes>", self.bytes.len()))
            .finish()
    }
}

/// `POST` [`SUFFIX_CAPTURE_LAUNCH`], after the harness is spawned: the daemon appends the
/// `Launch` record, adding the profile identity and its own revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureLaunchRequest {
    /// The key the goal submission returned.
    pub goal_stream: crate::capture::StreamKey,
    /// When the launcher observed the spawn, on its own capture clock.
    pub spawned_at: crate::capture::CaptureTime,
    /// What the harness printed for its own `version_args`, or `unreported` (D53): reported by
    /// the harness, not proven. The build's digests are what the daemon records.
    pub harness_version: String,
    /// The profile's declarative continuation, as the launch resolved it.
    pub continuation: String,
    /// This run's native session identity; unavailable identity records a capture gap.
    #[serde(default)]
    pub native_scope: Option<String>,
}

/// The loopback proxy's raw health counters at stop. The daemon records one stream-less
/// gap per non-zero counter except `sink_refusals`, whose losses are already chained.
///
/// Since H-16 this is a confirmatory second copy, not the only carrier: the proxy chains each of
/// these losses as a stream-zero `Loss` frame when it happens, so a request that never reaches
/// the daemon, or loses a race to a standalone seal, no longer hides them. A handled request
/// therefore adds daemon-clocked gaps for losses already chained on the producer's clock; gap
/// counts count loss records, not losses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyCounters {
    pub tunneled_fail_open: u64,
    pub opaque_fallbacks: u64,
    pub capture_table_full: u64,
    pub cut_unsettled: u64,
    pub unterminated: u64,
    pub sink_refusals: u64,
}

/// The one local-finish budget of a v5 capture entry, end to end: the launcher's proxy stop,
/// the daemon's drain and commit, and the sender's and the scoped stdout's remainder. It
/// replaces the CLI's `CAPTURE_FINISH` and the daemon's `CAPTURE_DRAIN`. Termination time is
/// outside it: SIGTERM, the connector's grace, SIGKILL, the kill sweep and the reconfirm keep
/// TERMINATION.md's bound and are never cut (D40; CONTRACTS.md, "Amendment 2026-10-02 (C6
/// local finish budget, H-10 4a)"). Three minutes since D56, so a slow disk or a large clone
/// still seals; it was 10 s.
pub const LOCAL_CAPTURE_FINISH_BUDGET: Duration = Duration::from_secs(180);

/// The commit share of [`LOCAL_CAPTURE_FINISH_BUDGET`]. The drain stages (proxy stop, ingest
/// drain, native finish) end this long before the deadline, so a producer that never settles
/// is cut with a recorded gap and still leaves time to seal.
pub const FINISH_COMMIT_RESERVE: Duration = Duration::from_secs(2);

/// `POST /v0/entries/{id}/seal` for a v5 entry. `proxy` is `None` when no proxy report
/// exists, because the launcher that ran the proxy is gone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureSealRequest {
    pub proxy: Option<ProxyCounters>,
    /// What remained of the caller's one local-finish budget when this request was built,
    /// whole milliseconds rounded down. Absent: the full budget, which is what an older CLI
    /// gets. Above the budget: clamped to it. Zero: refused before any finish step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining_finish_ms: Option<u64>,
    /// The native session identity of a family that only learns it at the end of its turn
    /// (Hermes prints it on stderr), read by the launcher after the harness exited. Installed by
    /// this finish before the entry's owner stops, and ignored for every family whose scope was
    /// due at the launch. Absent: no identity was read, and a standalone `goalcup seal` never
    /// sends one. Additive; an older daemon refuses a body that carries it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_scope: Option<String>,
}

impl CaptureSealRequest {
    /// The budget this request grants the finish: its stated remainder clamped to
    /// [`LOCAL_CAPTURE_FINISH_BUDGET`], or the whole budget when it states none. A stated
    /// zero is zero, which the daemon refuses.
    pub fn remaining_finish(&self) -> Duration {
        self.remaining_finish_ms
            .map_or(LOCAL_CAPTURE_FINISH_BUDGET, |ms| {
                Duration::from_millis(ms).min(LOCAL_CAPTURE_FINISH_BUDGET)
            })
    }
}

/// Whether the local finish created `seal.json` beside its chained terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalSealOutcome {
    SealWritten,
    SealWriteFailed,
}

/// The local finish of a v5 entry, with no receipt, acknowledgement or signed deadline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureFinishReport {
    pub entry_id: String,
    pub trace_dir: String,
    /// The actual process outcome the terminal binds.
    pub termination: TerminationOutcomeV1,
    /// The chained `LocalFinish` link's sequence, or `None` when it was never appended.
    pub terminal_seq: Option<u64>,
    pub seal: LocalSealOutcome,
}

// ------------------------------------------------------------------ errors

#[derive(Debug, thiserror::Error)]
pub enum ControlError {
    #[error("the daemon is not reachable: {0}")]
    Unreachable(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    #[error("preflight failed: {0}")]
    PreflightFailed(String),
    #[error("daemon error: {0}")]
    Internal(String),
    #[error("malformed control response: {0}")]
    Malformed(String),
    #[error("this platform is not supported in M1: {0}")]
    Unsupported(String),
    /// The process route durably observed this exact generation and canonical handle
    /// digest before later reconciliation failed.  The request is neither rejected nor
    /// acknowledged: only an exact replay or cancellation may settle it.
    #[error("process admission may already be durable: {0}")]
    ProcessAdmissionUncertain(String),
    /// The exact process generation and digest are already durable as `Prepared`, but the
    /// runtime that must reconcile them is temporarily unavailable.  Like [`Self::Busy`]
    /// this is retryable and maps to 409; unlike ordinary Busy it is post-mutation, so a
    /// process-owning client must retain the child and immutable replay body.
    #[error("prepared process admission is waiting for its runtime: {0}")]
    ProcessAdmissionBusy(String),
    /// **The ordinary pre-admission refusal a client may retry.** The daemon is holding the entry for the
    /// length of one asynchronous call — its runtime is out of the entry map while a pump
    /// uploads — and the call would have had to hold a lock across that. Nothing about the
    /// entry says no; the same request a moment later is expected to succeed.
    ///
    /// Appended last, deliberately: the variants above it are the contract
    /// `HttpDaemonClient` is written against, and a variant inserted among them would
    /// renumber nothing on the wire but would reorder every generated table that walks this
    /// enum. The process route's post-Prepare counterpart is the explicitly ambiguous
    /// [`Self::ProcessAdmissionBusy`] above.
    ///
    /// It shares `409` with [`ControlError::Conflict`] and means the opposite of it.
    /// `conflict` is the entry answering: already sealed, already started, a lock still on
    /// disk. Retrying one of those is a client spinning against a state that will not
    /// change, which is what a single `409` for both would make every client do.
    /// **Retry on `busy`; report `conflict`.**
    #[error("the daemon is busy with this entry: {0}")]
    Busy(String),
    /// The v5 local finish ran out of its one budget before the named stage, wrote no
    /// terminal, and left the entry open. It shares `409` with `conflict` and `busy` and is
    /// neither: the entry is not sealed and the same call is not retried. The payload, and
    /// so the wire message, is exactly the refused stage's token (`blocking_permit`,
    /// `run_lock`, `owner_drain`, `termination`, `workspace_walk`, `local_finish`,
    /// `not_started`), which is why this variant displays as the bare token.
    ///
    /// Appended last, like [`Self::Busy`]: the variants above it are the contract
    /// `HttpDaemonClient` is written against.
    #[error("{0}")]
    FinishDeadline(String),
}

impl ControlError {
    pub fn code(&self) -> &'static str {
        match self {
            ControlError::Unreachable(_) => "unreachable",
            ControlError::NotFound(_) => "not_found",
            ControlError::Conflict(_) => "conflict",
            ControlError::Unauthorized(_) => "unauthorized",
            ControlError::PreflightFailed(_) => "preflight_failed",
            ControlError::Malformed(_) => "malformed",
            ControlError::Unsupported(_) => "unsupported",
            ControlError::ProcessAdmissionUncertain(_) => "process_admission_uncertain",
            ControlError::ProcessAdmissionBusy(_) => "process_admission_busy",
            ControlError::Busy(_) => "busy",
            ControlError::FinishDeadline(_) => "finish_deadline",
            ControlError::Internal(_) => "internal",
        }
    }

    /// An unrecognised code becomes `Internal`, never a silent success.
    pub fn from_code(code: &str, message: &str) -> ControlError {
        match code {
            "not_found" => ControlError::NotFound(message.to_string()),
            "conflict" => ControlError::Conflict(message.to_string()),
            "unauthorized" => ControlError::Unauthorized(message.to_string()),
            "preflight_failed" => ControlError::PreflightFailed(message.to_string()),
            "malformed" => ControlError::Malformed(message.to_string()),
            "unsupported" => ControlError::Unsupported(message.to_string()),
            "process_admission_uncertain" => {
                ControlError::ProcessAdmissionUncertain(message.to_string())
            }
            "process_admission_busy" => ControlError::ProcessAdmissionBusy(message.to_string()),
            "busy" => ControlError::Busy(message.to_string()),
            "finish_deadline" => ControlError::FinishDeadline(message.to_string()),
            _ => ControlError::Internal(message.to_string()),
        }
    }
}

impl From<ProtocolError> for ControlError {
    fn from(e: ProtocolError) -> ControlError {
        ControlError::Malformed(e.to_string())
    }
}

/// Whether a failed capture seal can have started the daemon's finish (H-10 4a).
///
/// The classification decides what may be replayed, so it is narrow: only a failure that is
/// provably before the finish began is [`Self::NotAdmitted`]. Everything else, a framed 400
/// and every 404 included (the finish can answer `not_found` after its owner stopped, and
/// `malformed` from inside it), is [`Self::MaybeAdmitted`] and is never replayed by the call
/// that got it. The capture finish never answers `busy`.
#[derive(Debug)]
pub enum CaptureSealError {
    /// Exactly `busy`, `unauthorized` (the auth middleware answers before the handler) and a
    /// connection the daemon never accepted, where nothing was written.
    NotAdmitted(ControlError),
    /// The daemon answered from inside the finish (`finish_deadline`, `conflict`, `internal`,
    /// a 400 or a 404) or may have: a transport failure after the request was written, a read
    /// timeout, an unreadable response.
    MaybeAdmitted(ControlError),
}

impl CaptureSealError {
    pub fn into_error(self) -> ControlError {
        match self {
            Self::NotAdmitted(error) | Self::MaybeAdmitted(error) => error,
        }
    }

    pub fn not_admitted(&self) -> bool {
        matches!(self, Self::NotAdmitted(_))
    }

    /// For clients with no transport origin to read (in-process ones): the two framed
    /// pre-handler codes are `NotAdmitted`, everything else `MaybeAdmitted`.
    pub fn from_unclassified(error: ControlError) -> CaptureSealError {
        match error {
            error @ (ControlError::Busy(_) | ControlError::Unauthorized(_)) => {
                Self::NotAdmitted(error)
            }
            error => Self::MaybeAdmitted(error),
        }
    }
}

/// The wire shape of every failure body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: ErrorDetail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorDetail {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub detail: serde_json::Value,
}

// ------------------------------------------------------------------ the trait

/// The whole surface between the CLI and the daemon. Implemented by `HttpDaemonClient` and by
/// `MockDaemonClient`; served by `goalcup-daemon`. Adding a method means editing all three.
pub trait DaemonClient {
    /// The signed status document for one entry (D25).
    fn status(&self, entry_id: &str) -> Result<SignedStatus, ControlError>;
    /// Every entry this daemon knows about.
    fn entries(&self) -> Result<Vec<EntrySummary>, ControlError>;
    /// Finalize the entry and return the daemon's durable result. Workspace entries never
    /// rescan live bytes in the CLI; legacy entries retain their historical manifest arm.
    fn seal(&self, entry_id: &str) -> Result<SealResponse, ControlError>;
    /// The harness is a TUI and must be a child of the participant's terminal, so the
    /// CLI spawns it and hands the daemon the pids to terminate at the deadline (D22).
    fn register_process(
        &self,
        entry_id: &str,
        request: &crate::process::ProcessRegistration,
    ) -> Result<(), ControlError>;
    /// `goalcup doctor` needs one call that answers "is the daemon there".
    fn health(&self) -> Result<DaemonHealth, ControlError>;
    /// The participant wants the run to stop early; also the path `start` takes when the
    /// arming gate times out and the entry must not exist. A v5 capture entry is abandoned
    /// only before its launch and before a process is registered: that removes its
    /// reservation, row, artifacts and still-empty workspace. After either, seal it.
    fn abandon(&self, entry_id: &str, reason: &str) -> Result<(), ControlError>;
    /// Create a v5 local capture entry: reserved chain, baseline and ingest authorization.
    fn create_capture_workspace(
        &self,
        _request: &CaptureWorkspaceRequest,
    ) -> Result<CaptureWorkspaceInfo, ControlError> {
        Err(capture_unsupported())
    }
    /// Submit a v5 entry's goal once. `Ok` only after it is redacted, appended and synced;
    /// any failure, including a lost answer, means the launch does not go ahead.
    fn record_capture_goal(
        &self,
        _entry_id: &str,
        _request: &CaptureGoalRequest,
    ) -> Result<crate::capture::StreamKey, ControlError> {
        Err(capture_unsupported())
    }
    /// Record a v5 entry's launch after the harness is spawned.
    fn record_capture_launch(
        &self,
        _entry_id: &str,
        _request: &CaptureLaunchRequest,
    ) -> Result<(), ControlError> {
        Err(capture_unsupported())
    }
    /// Finish a v5 entry locally, posting the proxy's counters. `read_timeout` is how long
    /// the caller waits for the answer: its remaining budget, the termination allowance and
    /// the connect slack (CONTRACTS.md, H-10 4a ¶6).
    fn seal_capture(
        &self,
        _entry_id: &str,
        _request: &CaptureSealRequest,
        _read_timeout: Duration,
    ) -> Result<CaptureFinishReport, CaptureSealError> {
        Err(CaptureSealError::NotAdmitted(capture_unsupported()))
    }
}

/// What a client without v5 local capture answers for its methods.
fn capture_unsupported() -> ControlError {
    ControlError::Unsupported("this client does not implement v5 local capture".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_credentials_and_goal_bytes_never_reach_debug_output() {
        let info = CaptureWorkspaceInfo {
            entry_id: "e1".to_string(),
            process_generation: 1,
            reserved_bytes: 1,
            ingest_addr: std::net::SocketAddr::from(([127, 0, 0, 1], 9)),
            ingest_secret: "ingest-secret-ingest-secret-ingest-secret".to_string(),
            producer_id: "0".repeat(32),
            run_public_key: "k".to_string(),
            chain_head: "h".to_string(),
            trace_dir: "/t".to_string(),
        };
        let printed = format!("{info:?}");
        assert!(!printed.contains("ingest-secret"), "{printed}");
        assert!(printed.contains("<redacted>"));

        let goal = CaptureGoalRequest {
            bytes: b"a goal with a secret".to_vec(),
        };
        assert_eq!(
            format!("{goal:?}"),
            "CaptureGoalRequest { bytes: <20 bytes> }"
        );
    }
}
