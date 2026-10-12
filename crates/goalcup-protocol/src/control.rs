//! The local control API between `goalcup-cli` and `goalcup-daemon`.
//!
//! This module is the contract. The daemon implements the server side; the
//! CLI implements `DaemonClient`. Neither crate owns it, so it lives here.
//!
//! Transport in V4 is HTTP-only at `http://127.0.0.1:<port>`, with the loopback port read
//! from `<config_dir>/control.json`. Every response carries `Content-Length`; there is no
//! chunked encoding and no keep-alive requirement, so a small client is enough.
//! Every request carries `Authorization: Bearer <control_token>` **except the two public
//! routes `is_public_route` names** — `GET /v0/ui` and `GET /v0/entries/{id}/status`, which
//! are unauthenticated, loopback-only and read-only.
//! Write the middleware from `is_public_route`, never from this sentence.
//!
//! Every request, public or not, carries exactly one `Host` that names the listener itself:
//! `127.0.0.1:<port>`, `localhost:<port>` or `[::1]:<port>`, the port being the one in
//! `control.json`. Any other `Host` is refused `401 unauthorized` before the public/bearer
//! split, so a page reaching the daemon through DNS rebinding reads nothing. A client that
//! builds its URL from `control.json` sends a matching `Host` without trying.

use crate::ProtocolError;
use crate::events::HealthState;
use crate::manifest::InstallScope;
use crate::process::ProcessHandle;
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
pub const CONTROL_API_VERSION: u32 = 5;

pub const ROUTE_WORKSPACES: &str = "/v0/workspaces";
pub const ROUTE_ENTRIES: &str = "/v0/entries";
pub const ROUTE_ENTRIES_CURRENT: &str = "/v0/entries/current";
pub const ROUTE_HEALTH: &str = "/v0/health";
pub const ROUTE_UI: &str = "/v0/ui";
/// `POST` a [`CaptureWorkspaceRequest`]: create a protocol-v5 local capture entry.
pub const ROUTE_CAPTURE_WORKSPACES: &str = "/v0/capture-workspaces";

/// A status document older than this is reported as **stale** — by the MCP server
/// (`goalcup-mcp`), by `goalcup status`, and by the bundled page's `render()`. One number,
/// here, because a threshold that two surfaces disagree about is a threshold that tells the
/// participant two different stories about the same daemon.
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
pub const SUFFIX_ARM: &str = "arm";

/// See [`SUFFIX_ARM`].
pub const SUFFIX_START_SIGNAL: &str = "start-signal";

/// See [`SUFFIX_ARM`]. The one suffix that is also a PUBLIC route.
pub const SUFFIX_STATUS: &str = "status";

/// See [`SUFFIX_ARM`].
pub const SUFFIX_PROCESS: &str = "process";

/// See [`SUFFIX_ARM`].  Returns an issued resume permit before a replacement child is
/// accepted; it is deliberately separate from `abandon`, which may only delete unstarted
/// entries.
pub const SUFFIX_PROCESS_CANCEL: &str = "process/cancel";

/// See [`SUFFIX_ARM`].
pub const SUFFIX_ABANDON: &str = "abandon";

/// See [`SUFFIX_ARM`].
pub const SUFFIX_RESUME: &str = "resume";

/// See [`SUFFIX_ARM`]. The read-only launch-pin answer the V1 `resume --print-launch` read.
pub const SUFFIX_RESUME_PREVIEW: &str = "resume-preview";

/// See [`SUFFIX_ARM`].
pub const SUFFIX_SEAL: &str = "seal";

/// See [`SUFFIX_ARM`]. A v5 entry's one-shot goal ([`CaptureGoalRequest`]).
pub const SUFFIX_CAPTURE_GOAL: &str = "capture/goal";

/// See [`SUFFIX_ARM`]. A v5 entry's launch, recorded after spawn ([`CaptureLaunchRequest`]).
pub const SUFFIX_CAPTURE_LAUNCH: &str = "capture/launch";

/// The long-poll deadline `wait_start_signal` puts on the wire.
pub const QUERY_TIMEOUT_MS: &str = "timeout_ms";

/// The most the daemon will hold one start-signal long poll open for, whatever
/// [`QUERY_TIMEOUT_MS`] asks for.
///
/// **A contract and not a daemon detail.** The daemon clamps every request to it, so a CLI
/// asking for a connector's whole arming window (300 s for Codex) would get a `408` at sixty
/// seconds and read it as the window closing. A CLI cannot size its polls against a number
/// it cannot see, so the number lives here, in the crate both halves share, and the daemon
/// imports it.
///
/// Sixty seconds is well inside every connector's `arming.timeout_seconds` (Claude 120,
/// Codex 300), which is what makes a loop of these meet the gate's own answer rather than
/// this bound.
pub const MAX_START_SIGNAL_WAIT_MS: u64 = 60_000;

/// `/v0/entries/{id}/{suffix}`.
pub fn entry_route(entry_id: &str, suffix: &str) -> String {
    format!("{ROUTE_ENTRIES}/{entry_id}/{suffix}")
}

/// `/v0/entries/{id}/start-signal?timeout_ms=<ms>`, built once so the client and the
/// server agree on the parameter's name, its unit and its rounding.
///
/// Saturates at `u64::MAX` milliseconds rather than wrapping, because a wrapped deadline
/// is a long-poll that returns instantly and reads as "the signal never came" — the one
/// failure a participant cannot tell apart from a harness that never armed.
pub fn start_signal_route(entry_id: &str, timeout: Duration) -> String {
    let ms = u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX);
    format!(
        "{}?{QUERY_TIMEOUT_MS}={ms}",
        entry_route(entry_id, SUFFIX_START_SIGNAL)
    )
}

/// The authentication ruling above, as one function.
///
/// `true` for exactly the two routes the ruling names as unauthenticated, loopback-only
/// and read-only, and `false` for everything else. (`/v0/assets/*` was a third until nothing
/// served it: a prefix with no route behind it is public surface for no reader.) The daemon's auth middleware skips the
/// token on a `true`; the CLI's `HttpDaemonClient` omits the `Authorization` header on a
/// `true`. **Both call this function**, so the server and the client cannot drift into
/// disagreeing about which routes need a token.
///
/// `/v0/entries/current` is deliberately **not** public: reading one entry's signed status
/// when you already know its id is accepted, enumeration is not.
///
/// A path carrying a `.` or `..` segment is never public. `/v0/ui/../entries` starts with a
/// public route and resolves, at every normalising client and proxy between here and the
/// socket, to an authed one — so matching on the unresolved text would hand the authed
/// surface a free pass. (The rule was written for the `/v0/assets/` prefix, whose
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
    if path == ROUTE_UI {
        return true;
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

// ------------------------------------------------------------------ requests

/// `Debug` is written out below rather than derived: this struct holds a live credential.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceOpts {
    pub connector_id: String,
    /// SHA-256 of the connector manifest the CLI loaded, so the daemon refuses a
    /// manifest that is not the one in the signed release.
    ///
    /// Of the **file's** bytes, and it stays that way: this is the digest the daemon puts
    /// on the arming receipt's `manifest_sha256` claim, and the verifier recomputes it
    /// from the `manifest.toml` the trace carries. A trace does not carry the rest of the
    /// connector directory, so this is the one digest the two sides can both take.
    ///
    /// **There is deliberately no `connector_dir_sha256` beside it.** The whole-directory
    /// digest — the one that covers the hook bodies under `files/` — is computed by the
    /// **daemon**, over its own `connectors_dir`, at workspace creation, and compared
    /// against `pins::CONNECTOR_DIR_SHA256S`. A digest the CLI sent would be a digest
    /// the checked process supplied, which checks nothing: the daemon is what renders those
    /// bodies and then guards them, so it is what has to hash them. It travels from there
    /// onto the start receipt (`StartReceiptClaims::connector_dir_sha256`), where V13 reads
    /// it. The CLI keeps its own directory check for `goalcup doctor`, where it answers a
    /// question about the CLI's own installation.
    pub manifest_sha256: String,
    /// Where the workspace directory is created. `None` means the daemon's default,
    /// which is never an ancestor of the artifact root.
    pub workspace_parent: Option<String>,
    /// Desktop attach mode: arm the folder and wait rather than expecting a launch.
    pub attach: bool,
    /// **Nothing reads this, and a daemon must not honour it**.
    ///
    /// The window is a RULE, not a preference: 168 hours, and the clock never pauses. The
    /// V1 daemon derived the deadline from the development server's entry window and
    /// compared its re-derivation against the receipt the server signed; D48 removed that
    /// receipt path, and the current daemon sets no deadline from this field. So the number a
    /// CLI puts here reaches no deadline anywhere. The shipped CLI always sends the constant
    /// `DEFAULT_DEADLINE_HOURS = 168`.
    ///
    /// It stays on the wire because removing a field breaks version skew in the direction
    /// that matters — an older daemon deserialising a newer CLI's request. It is said HERE,
    /// and not only in the CLI that sends it, because this is where the next implementer
    /// looks: a daemon that honoured this in good faith would turn a participant's number
    /// into the run's window.
    pub deadline_hours: u32,
    /// The grant the V1 `goalcup auth` stored (D48 removed the command). The V1 daemon
    /// exchanged it with the server.
    pub grant_token: String,
    /// Third and last step of the arming-timeout precedence chain documented on
    /// `manifest::Arming`: manifest default, daemon-config override, CLI flag. `None`
    /// leaves the first two in force.
    pub arming_timeout_seconds: Option<u64>,
    /// What the CLI's own D12 preflight MEASURED the harness at, seconds before this call.
    ///
    /// The daemon runs no version probe of its own — `detection.version_command` is the
    /// CLI's to run, and the CLI has just run it to decide whether this machine is inside
    /// `tested_version_range` at all. Without it the signed status document would say
    /// `harness: unknown` about a harness `goalcup doctor` had printed a version for
    /// seconds earlier.
    ///
    /// `None` means genuinely not probed — the binary is not on `PATH`, or it would not
    /// answer — and the status document then says nothing rather than an empty string, which
    /// is a measurement of nothing dressed as one.
    ///
    /// `#[serde(default)]`, so a CLI older than this field still creates a workspace: the
    /// two halves are installed separately, and a daemon that refused the older request
    /// would turn a cosmetic gap into a broken machine.
    #[serde(default)]
    pub harness_version: Option<String>,
}

/// Every field as a derive would print it, except `grant_token`, which prints
/// `<redacted>`. `WorkspaceOpts` reaches the daemon's request log and is interpolated
/// into `ControlError::PreflightFailed`; a `{:?}` that carried the grant would hand a
/// reader the credential the V1 `goalcup auth` stored, which is the one that exchanges for
/// everything else.
impl std::fmt::Debug for WorkspaceOpts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkspaceOpts")
            .field("connector_id", &self.connector_id)
            .field("manifest_sha256", &self.manifest_sha256)
            .field("workspace_parent", &self.workspace_parent)
            .field("attach", &self.attach)
            .field("deadline_hours", &self.deadline_hours)
            .field("grant_token", &"<redacted>")
            .field("arming_timeout_seconds", &self.arming_timeout_seconds)
            .field("harness_version", &self.harness_version)
            .finish()
    }
}

// ------------------------------------------------------------------ responses

/// One file the **daemon** rendered and wrote. The CLI writes none of
/// these; it is told where they landed so it can pass `--settings` on the command line
/// and so `goalcup seal` can exclude exactly what the tracker put in the workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledFile {
    /// Absolute path as written.
    pub path: String,
    pub scope: InstallScope,
    /// Path relative to `scope`'s root, forward slashes.
    pub relative_path: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    pub entry_id: String,
    /// The daemon-issued initial process generation.  It becomes usable only after `arm`;
    /// carrying it at creation lets the CLI build one typed handoff without inventing a
    /// nonce or querying mutable state between arm and spawn.
    pub process_generation: u64,
    /// The empty directory the participant works in. Outside any Git worktree.
    ///
    /// **The canonical spelling** — what `std::fs::canonicalize` returns for it (D16).
    /// This is the ONE path this run is known by: the daemon stores it on the
    /// registry row, renders every `{workspace}` and `{slug}` from it, compares every hook
    /// post's `cwd` against it, and answers the same string later on
    /// [`ResumeInfo::workspace`]. A caller that caches this value and hands it back — a
    /// resume's lock roots, a seal's self-check — compares equal, which the requested
    /// spelling did not under a workspace parent reached through a symlink.
    pub workspace: String,
    /// The spelling the participant ASKED for: `workspace_parent` joined with the entry
    /// id, before it was resolved. Under a symlinked parent — the default shape on macOS,
    /// where `/var` is a link to `/private/var` — it differs from [`Self::workspace`], and
    /// otherwise it is the same string.
    ///
    /// **For display only.** Nothing compares it to anything: an older daemon answers no
    /// such field, so `serde(default)` makes it the empty string there, and a caller that
    /// routed on it would route on `""`. Show it when a participant needs to see the name
    /// they typed; use [`Self::workspace`] for everything else.
    #[serde(default)]
    pub workspace_requested: String,
    /// Buffer, logs, baselines, receiver state. Never an ancestor of the workspace.
    pub artifact_root: String,
    /// The dedicated harness home for this run's process-scoped pins (D5).
    pub harness_home: String,
    /// Hex Ed25519 verifying key for this run's signed status.
    pub run_public_key: String,
    pub status_url: String,
    pub status_cache_path: String,
    /// One line of plain text the Claude status line `cat`s. Never holds a secret.
    pub status_line_path: String,
    /// The session id the launch recipe pins where the harness accepts one.
    pub session_id: String,
    /// The port the daemon reserved for a `Server`-mode launch, if the connector needs one.
    pub server_port: Option<u16>,
    /// Every file the daemon rendered from the connector's `hook_install[]`, in
    /// manifest order. The CLI reads the `ArtifactRoot`-scoped ones to build launch argv.
    pub installed_files: Vec<InstalledFile>,
    /// The `Workspace`-scoped subset, workspace-relative and sorted: exactly what the
    /// seal excludes and the public entry discloses (`RULES.md`).
    pub workspace_relative: Vec<String>,
    pub created_at: String,
}

/// The observed arming signal. Distinct from `manifest::StartSignal`, which only says
/// whether one is required; always qualify the name at the use site.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartSignal {
    pub entry_id: String,
    /// The connector's `arming.signal` string, echoed back.
    pub signal: String,
    pub session_id: Option<String>,
    pub observed_at: String,
    /// `leg1` or `leg2`: which leg witnessed it.
    pub source: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArmingState {
    Armed,
    StartSignalSeen,
    Started,
    /// The gate will never arm — a start signal that never arrived, a timeout, or an
    /// occupied harness home. Only a pre-D48 (v0.5.0) daemon writes it. Last in the enum so
    /// no existing wire value shifts meaning; a reader that does not know this variant
    /// reports it as an unrecognised string rather than as a live state it can misjudge as
    /// still pending.
    ///
    /// A machine reader must be able to tell "will never arm" from "still waiting to arm"
    /// without parsing prose: `detail` carries the reason in words for a human reader, and
    /// this variant carries the fact.
    Refused,
}

/// Where the [`EventStatus`] in a status document came from.
///
/// The daemon has no way to KNOW the round's dates; it is told them, in its per-machine
/// configuration's `[event]` section, or it is not. A document is signed either way, so this
/// is what lets a reader tell "the organiser said the window closes then" from "this is the
/// entry's own timeline standing in for one" — which is the difference between a number an
/// agent may act on and a number the daemon made up.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventStatusSource {
    /// The dates were configured on the machine running the daemon.
    Config,
    /// No round was configured, so the fields carry the entry's own timeline. The default,
    /// and the reading a document written by a build that predates this field gets.
    #[default]
    Fallback,
}

/// The competition-wide facts, signed with the entry so an agent asking the time
/// through the MCP server gets a number it can trust.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventStatus {
    pub t0: String,
    pub start_window_closes_at: String,
    pub publication_closes_at: String,
    pub rules_version: String,
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
    /// The connector's `arming.recipe` while the gate is unmet; `None` once it clears.
    pub arming_recipe: Option<String>,
    /// The health state in plain words, for the status page and the terminal (D25).
    pub detail: String,
    pub event: EventStatus,
    /// Whether [`StatusPayload::event`] was configured or is the entry's own timeline.
    ///
    /// `#[serde(default)]`, like the two counters below and for the same reason: the cache
    /// directory outlives an upgrade, and a document a reader refuses is worse than one
    /// whose provenance reads `fallback` — which is what a document written before this
    /// field existed in fact carried.
    #[serde(default)]
    pub event_status_source: EventStatusSource,
    /// What the daemon's own four cadences lost while this entry was open.
    ///
    /// `#[serde(default)]` so a cached document written by a build that did not have this
    /// field still loads: the cache directory outlives an upgrade, and a status a reader
    /// refuses is worse than one whose counters read zero.
    #[serde(default)]
    pub cycles: CycleErrors,
    /// Hook posts that never reached this entry's chain.
    ///
    /// A post that arrives while the entry is held by an asynchronous call is queued rather
    /// than refused, and the queue is bounded so a stuck pump cannot grow it without limit.
    /// Anything over the bound is lost, and so is a post the chain then refused — both are
    /// the same fact from a reader's side, and this is the only place either surfaces.
    #[serde(default)]
    pub posts_dropped: u64,
    /// Hook posts that arrived for a run this daemon had already sealed and evicted.
    ///
    /// Daemon-level, unlike [`StatusPayload::posts_dropped`], because by then the entry is
    /// out of the map and there is no per-entry counter left to hold it. A participant whose
    /// harness is still posting after the seal has a harness the kill did not reach, and
    /// this is the only place that says so.
    #[serde(default)]
    pub posts_dropped_after_seal: u64,
    /// Reported hook pins beyond the memo's bound that were counted and chained nowhere.
    ///
    /// The shim reports the hook-set hash it is running; the daemon remembers a bounded
    /// number of distinct ones. A peer varying the value cannot grow that memo, and what it
    /// costs is counted here rather than dropped silently.
    #[serde(default)]
    pub pins_dropped: u64,
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
    /// Hex Ed25519 verifying key: the run key from `WorkspaceInfo.run_public_key`.
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

    /// Verify against the run key the **caller** pinned — `WorkspaceInfo.run_public_key`,
    /// which the CLI and the MCP server both hold from `create_workspace`.
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
pub struct ResumeInfo {
    pub entry_id: String,
    /// The newly opened process generation.  `resume` consumed the caller's expected
    /// generation atomically, so this is the only generation its relaunch may register.
    pub process_generation: u64,
    pub session_id: String,
    /// How many events the daemon caught up before answering.
    pub caught_up_events: u64,
    pub workspace: String,
    pub artifact_root: String,
    pub harness_home: String,
    pub status_line_path: String,
    /// `None` means the connector reserves no server port (`Tui`/`Attach` launch modes).
    /// `Some` is the port reserved at workspace creation and remembered on the registry
    /// row, returned here so a resume renders the same `{server_port}`; it is
    /// remembered, not re-reserved, so a preflight must check it is still free.
    pub server_port: Option<u16>,
    /// The process the previous launch registered, so the CLI can tell a stale lock
    /// from a live one before relaunching (D22, D9).
    pub last_process: Option<ProcessHandle>,
}

/// The immutable launch pins the V1 `goalcup resume --print-launch` needed. D48 removed
/// that command, and the daemon now refuses the preview route.
///
/// Unlike [`ResumeInfo`], obtaining this answer does not poll the capture receiver or
/// clear the registered process handle. Keep it deliberately limited to the seven values a
/// launch template consumes: adding a resume-only operational field here would make the
/// preview endpoint look like a non-mutating form of `resume`, which it is not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumePreviewInfo {
    pub entry_id: String,
    /// The authoritative, read-only process generation.  A local cache may have missed a
    /// successful cancellation response, so a normal resume obtains this before deciding
    /// which generation to compare-and-swap.
    pub process_generation: u64,
    /// Diagnostic only: the daemon's durable phase for that generation.  The CLI does not
    /// derive permission from its cache or from this string; the mutating resume CAS remains
    /// authoritative.
    pub process_handoff_phase: String,
    pub session_id: String,
    pub workspace: String,
    pub artifact_root: String,
    pub harness_home: String,
    pub status_line_path: String,
    pub server_port: Option<u16>,
}

/// The sole body accepted by `POST /resume`.  A bare or empty request would let an old
/// client consume a replacement slot without proving which registered generation it saw.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResumeRequest {
    pub expected_generation: u64,
}

/// The sole body accepted by `POST /process/cancel`.  `request_digest` is absent only when
/// no child reached `Prepared`; a prepared child can be returned only by the client whose
/// exact canonical process identity it is.  The digest contains no seed contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessHandoffCancelRequest {
    pub expected_generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_digest: Option<String>,
}

/// The durable generation after a cancellation.  It is never decremented or reused; this
/// answer is what lets a CLI repair its cache before reporting its original local failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessHandoffCancelInfo {
    pub process_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonHealth {
    pub control_api_version: u32,
    pub daemon_version: String,
    pub uptime_seconds: u64,
    pub entries_open: u32,
    pub upload_ok: bool,
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
    /// Serialized chain bytes reserved before the entry is ready, the terminal allowance
    /// included ([`crate::capture::DEFAULT_CAPTURE_RESERVE_BYTES`] unless measured otherwise).
    pub reserve_bytes: u64,
    /// The absolute per-run `HOME` the launcher creates and gives the harness; the daemon
    /// reads `{home}/` stores only for controlled-workload families.
    pub run_home: String,
    /// The absolute tracker-isolated harness configuration home, shared across runs.
    pub config_home: String,
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
    /// The chain bytes measured as physically allocated: the requested amount.
    pub reserved_bytes: u64,
    /// The daemon's loopback ingest endpoint.
    pub ingest_addr: std::net::SocketAddr,
    /// This entry's launch-scoped ingest credential, held in memory only: never argv,
    /// environment, disk or logs.
    pub ingest_secret: String,
    /// The fresh producer ID the proxy's first ingest epoch uses.
    pub producer_id: String,
    /// The run key every status document of this entry is signed with; the caller pins it
    /// and verifies against it, as with [`WorkspaceInfo::run_public_key`].
    pub run_public_key: String,
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
            .finish()
    }
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
    /// The harness version the launcher measured.
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
/// local finish budget, H-10 4a)").
pub const LOCAL_CAPTURE_FINISH_BUDGET: Duration = Duration::from_secs(10);

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
    #[error("arming timed out: {detail}")]
    ArmingTimeout { detail: String },
    #[error("preflight failed: {0}")]
    PreflightFailed(String),
    #[error("unsupported connector: {0}")]
    UnsupportedConnector(String),
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
            ControlError::ArmingTimeout { .. } => "arming_timeout",
            ControlError::PreflightFailed(_) => "preflight_failed",
            ControlError::UnsupportedConnector(_) => "unsupported_connector",
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
            "arming_timeout" => ControlError::ArmingTimeout {
                detail: message.to_string(),
            },
            "preflight_failed" => ControlError::PreflightFailed(message.to_string()),
            "unsupported_connector" => ControlError::UnsupportedConnector(message.to_string()),
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

/// Whether a failed process registration can safely be treated as not admitted.
///
/// A [`ControlError::Malformed`] alone does not answer that question: it can be a daemon's
/// framed `400` validation refusal before the handler touches process authority, or a client
/// that could not interpret an acknowledgement after the daemon persisted it.  Transport
/// implementations preserve that origin here; in-process clients use the conservative
/// default below.
#[derive(Debug)]
pub enum ProcessRegistrationError {
    DefinitelyRefused(ControlError),
    PossiblyAdmitted(ControlError),
}

impl ProcessRegistrationError {
    pub fn may_have_been_admitted(&self) -> bool {
        matches!(self, Self::PossiblyAdmitted(_))
    }

    pub fn into_error(self) -> ControlError {
        match self {
            Self::DefinitelyRefused(error) | Self::PossiblyAdmitted(error) => error,
        }
    }

    /// The trait's legacy `ControlError` result has no response-origin information.  Preserve
    /// safety for transport and post-Prepare outcomes. Ordinary `Busy` is explicitly
    /// pre-admission; `ProcessAdmissionBusy` carries the post-mutation retry case.
    pub fn from_unclassified(error: ControlError) -> ProcessRegistrationError {
        match error {
            error @ (ControlError::Unreachable(_)
            | ControlError::Malformed(_)
            | ControlError::Internal(_)
            | ControlError::ProcessAdmissionUncertain(_)
            | ControlError::ProcessAdmissionBusy(_)) => Self::PossiblyAdmitted(error),
            error => Self::DefinitelyRefused(error),
        }
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
    /// Create the entry, the empty workspace and the artifact root, **and write the
    /// connector's folder-scoped config**. The rendered paths and hashes come
    /// back on `WorkspaceInfo`; the CLI writes nothing itself.
    fn create_workspace(
        &self,
        harness: &str,
        opts: &WorkspaceOpts,
    ) -> Result<WorkspaceInfo, ControlError>;
    /// Move the entry to `armed`: config hashes recorded, receiver token live (D16).
    fn arm(&self, entry_id: &str) -> Result<(), ControlError>;
    /// Long-poll for the connector's `arming.signal` for this workspace (D16).
    fn wait_start_signal(
        &self,
        entry_id: &str,
        timeout: Duration,
    ) -> Result<StartSignal, ControlError>;
    /// The signed status cache for one entry (D25).
    fn status(&self, entry_id: &str) -> Result<SignedStatus, ControlError>;
    /// Every entry this daemon knows about.
    fn entries(&self) -> Result<Vec<EntrySummary>, ControlError>;
    /// Catch the tail up to the present, then report what a relaunch needs.
    fn resume(&self, entry_id: &str, request: &ResumeRequest) -> Result<ResumeInfo, ControlError>;
    /// Read only the pinned launch values needed to render a relaunch line. This does not
    /// catch up capture state or clear a registered process handle.
    fn resume_preview(&self, entry_id: &str) -> Result<ResumePreviewInfo, ControlError>;
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
    /// Register a process while retaining whether a failure might follow durable admission.
    ///
    /// The default keeps existing in-process implementations safe: their legacy
    /// [`ControlError`] does not say whether `Malformed` came from a verified daemon refusal
    /// or a response the client could not parse.  An HTTP client overrides this with the
    /// response origin it observed on the wire.
    fn register_process_with_admission(
        &self,
        entry_id: &str,
        request: &crate::process::ProcessRegistration,
    ) -> Result<(), ProcessRegistrationError> {
        self.register_process(entry_id, request)
            .map_err(ProcessRegistrationError::from_unclassified)
    }
    /// Return an issued resume generation before a replacement process was accepted.  This
    /// is an authenticated administrative recovery transition, not a bearer-derived nonce.
    fn cancel_process_handoff(
        &self,
        entry_id: &str,
        request: &ProcessHandoffCancelRequest,
    ) -> Result<ProcessHandoffCancelInfo, ControlError>;
    /// `goalcup doctor` needs one call that answers "is the daemon there".
    fn health(&self) -> Result<DaemonHealth, ControlError>;
    /// `goalcup status` with no entry argument, and the status page's default view.
    fn current_entry(&self) -> Result<Option<String>, ControlError>;
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
