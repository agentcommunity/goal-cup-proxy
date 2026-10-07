//! The start receipt: asked for once, verified twice, and stored only if it survives both
//! (D16).
//!
//! # The two checks, and why the second one exists
//!
//! [`StartReceipt::verify_with`] answers "did the key this build pins sign this document".
//! That is necessary and it is not enough: it says nothing about *what* was signed. A
//! server that signs a receipt naming a `deadline_at` a month out, or another entry's id,
//! or a `chain_head` that is not the link the daemon actually wrote, produces a document
//! that verifies perfectly and describes a run that did not happen.
//!
//! So [`accept_receipt`] compares every claim, field by field, against the claims the
//! daemon asserted, and names the first field that differs. The deadline is the case that
//! motivates it: the server owns `started_at` and `deadline_at` (a participant who names
//! their own start names their own deadline), so the daemon takes both values from the
//! verified receipt rather than re-deriving a fixed duration. A server that quietly
//! extends the window is then caught here, at the moment of issue, rather than by the
//! verifier once the run is already over.
//!
//! # Asked for exactly once
//!
//! The server enforces "success after dispatch is immutable" as a 409, so this
//! client requests once and reports the refusal rather than retrying into it. The request
//! carries the daemon's claims **and** its per-run public key. The key always travels beside
//! the claims as the server's upload-verification input. A legacy receipt leaves its two
//! additive binding claims absent; a new-event receipt repeats the canonical key inside its
//! signed claims beside the questionnaire version. The server compares that claim to the
//! sidecar before it pins the accepted key to the entry, so every later chunk is checked
//! against that key and no other.
//!
//! # The transport
//!
//! HTTP/1.1 over `std::net::TcpStream`, inside `spawn_blocking`. This workspace has no
//! client crate, and adding one — or the `io-util` feature an async client would need on
//! the tokio the daemon shares — for one request per run is a dependency the whole
//! workspace then carries.
//!
//! **Bounded on all three of connect, read and write** ([`NETWORK_TIMEOUT`]). A blocking
//! socket has no deadline of its own but the OS's, so a server that
//! accepted the connection and then said nothing held the `spawn_blocking` worker — and
//! with it the one request a run gets — for as long as the kernel felt like it. The whole
//! of the daemon's outbound protocol is this one call; it is the one that must not hang.
//!
//! **Cleartext is loopback-only.** Nothing in this build speaks TLS, so there are exactly two
//! honest answers to a configured endpoint: a loopback host, where plaintext never leaves
//! the machine, or a refusal. An `https://` url is refused by name rather than being quietly
//! downgraded, and an `http://` url naming anything but 127.0.0.0/8, `localhost` or `[::1]`
//! is refused rather than shipping a run's claims and its public key over somebody's network
//! in the clear. [`DaemonConfig::validate`](crate::config::DaemonConfig::validate) applies
//! the same rule to `upload_url` at load, so the refusal arrives before a run does.
//!
//! **Nothing here logs.** The request body carries a public key and the response carries a
//! signature; neither is secret, but this crate holds the per-run secrets in the same
//! process, and a module that can write a request out is a module that will one day write the
//! wrong one.

use crate::arming::ArmingGate;
use crate::config::ReceiptBindingPolicy;
use crate::registry::{Entry, EntryState, Registry, StartPathIdentity};
use crate::{DaemonError, MAX_FILE_BYTES};
use goalcup_protocol::control::{format_z, parse_z};
use goalcup_protocol::provenance::{canonical_run_public_key, validate_questionnaire_version};
use goalcup_protocol::receipt::{StartReceipt, StartReceiptClaims};

/// The entry window, in seconds: 168 hours.
///
/// Restated here rather than imported, because the crate that computes it
/// (`goalcup_devserver::ENTRY_WINDOW_SECONDS`) is a dev dependency — it holds a server key
/// and must not link into the shipped daemon binary (D3). The two numbers are pinned equal
/// by `tests/arming_receipt.rs`, which asks a real server for a receipt and checks the
/// deadline this constant derives against the one that server computed.
pub const ENTRY_WINDOW_SECONDS: u64 = 604_800;

/// How long connect, read and write each get before the request is an error.
///
/// Ten seconds, on all three, because the failure being bounded is the same one: a peer
/// that is reachable and unresponsive. A blocking `TcpStream` carries no deadline of its
/// own, so without these a receipt request against a listener that accepts and never
/// answers never returns at all — and it is the only request the run makes.
pub const NETWORK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// The client for one entry's receipt request.
pub struct ReceiptClient {
    /// The upload base, e.g. `http://127.0.0.1:8080`. Loopback and cleartext, or refused:
    /// see [`split_url`].
    upload_url: String,
    timeout: std::time::Duration,
}

impl ReceiptClient {
    pub fn new(upload_url: impl Into<String>) -> ReceiptClient {
        ReceiptClient::with_timeout(upload_url, NETWORK_TIMEOUT)
    }

    /// The same client on a shorter clock. Exists so a test can prove the timeout without
    /// waiting out the shipped one; every caller in `src/` uses [`ReceiptClient::new`].
    pub fn with_timeout(
        upload_url: impl Into<String>,
        timeout: std::time::Duration,
    ) -> ReceiptClient {
        ReceiptClient {
            upload_url: upload_url.into(),
            timeout,
        }
    }

    /// Called only in state `start_signal_seen`. Posts the claims the daemon asserts —
    /// entry, connector, workspace, session, `started_at`, `deadline_at`, the manifest
    /// digest, the chain head at the moment of the first prompt, and the digest of the
    /// connector directory the workspace was rendered from — beside the per-run public key,
    /// and returns what the server signed.
    ///
    /// `started_at` and `deadline_at` are sent and then **overwritten by the server**; what
    /// comes back is its clock. That is the point of asking: a start a participant could
    /// name is a deadline they could name. [`accept_receipt`] is what checks the answer.
    ///
    /// Called once. A second request for the same entry is a 409, reported as
    /// [`DaemonError::Http`] carrying the status, and is never retried into.
    pub async fn request(
        &self,
        claims: &StartReceiptClaims,
        run_public_key: &str,
    ) -> Result<StartReceipt, DaemonError> {
        let body = serde_json::to_string(&serde_json::json!({
            "claims": claims,
            "run_public_key": run_public_key,
        }))?;
        let (authority, prefix) = split_url(&self.upload_url)?;
        let path = format!("{prefix}/v0/entries/{}/receipt", claims.entry_id);
        let timeout = self.timeout;
        let (status, text) =
            tokio::task::spawn_blocking(move || post(&authority, &path, &body, timeout))
                .await
                .map_err(|e| {
                    DaemonError::Http(format!("the receipt request did not run: {e}"))
                })??;
        if status != 200 {
            return Err(DaemonError::Http(format!(
                "the receipt server answered {status}: {}",
                text.trim()
            )));
        }
        Ok(serde_json::from_str(&text)?)
    }
}

/// Verifies against the key the `DaemonConfig` pins and checks that every claim is the
/// daemon's own, so a server that rewrites `deadline_at` is caught here rather than by the
/// verifier once the run is over.
///
/// The signature is checked first: a receipt nobody this build trusts signed is refused
/// before its contents are read at all, so a forged document cannot steer the error message
/// by choosing which field to differ in.
///
/// `claims` is what the daemon asserts the receipt should say. For the seven fields the
/// daemon owns that is what it sent; for the two the server owns it is the signed
/// `started_at` and `deadline_at` returned by the receipt service. Callers must not replace
/// those values with participant-requested times or a locally re-derived duration.
pub fn accept_receipt(
    receipt: &StartReceipt,
    claims: &StartReceiptClaims,
    pinned_server_key: &str,
) -> Result<(), DaemonError> {
    receipt.verify_with(pinned_server_key).map_err(|e| {
        DaemonError::Refused(format!(
            "the start receipt for entry {} was not accepted: {e}",
            claims.entry_id
        ))
    })?;
    let got = &receipt.claims;
    // Every field, in declaration order, so the first difference reported is the first one
    // a reader of the struct would look for.
    let differences: [(&str, &str, &str); 9] = [
        ("entry_id", got.entry_id.as_str(), claims.entry_id.as_str()),
        (
            "connector_id",
            got.connector_id.as_str(),
            claims.connector_id.as_str(),
        ),
        (
            "workspace",
            got.workspace.as_str(),
            claims.workspace.as_str(),
        ),
        (
            "session_id",
            got.session_id.as_str(),
            claims.session_id.as_str(),
        ),
        (
            "started_at",
            got.started_at.as_str(),
            claims.started_at.as_str(),
        ),
        (
            "deadline_at",
            got.deadline_at.as_str(),
            claims.deadline_at.as_str(),
        ),
        (
            "manifest_sha256",
            got.manifest_sha256.as_str(),
            claims.manifest_sha256.as_str(),
        ),
        (
            "chain_head",
            got.chain_head.as_str(),
            claims.chain_head.as_str(),
        ),
        // The ninth, and the only optional one. A server that dropped the claim would leave
        // a receipt whose signature is perfectly good and which says nothing about the
        // connector directory — V13's `connector_dir_unbound`, over a run the
        // daemon believed it had bound. Compared as the two words below rather than as
        // `Option`s so the refusal reads like the other eight; `(absent)` is not a digest,
        // so it cannot collide with one.
        (
            "connector_dir_sha256",
            got.connector_dir_sha256.as_deref().unwrap_or("(absent)"),
            claims.connector_dir_sha256.as_deref().unwrap_or("(absent)"),
        ),
    ];
    for (field, found, expected) in differences {
        if found != expected {
            return Err(DaemonError::Refused(format!(
                "the start receipt for entry {} says {field} = {found}, and this daemon \
                 asserted {expected}: a receipt is a signature over the run's own claims, \
                 so one the daemon did not make is not stored",
                claims.entry_id
            )));
        }
    }
    validate_signed_receipt_times(receipt, &claims.entry_id)?;
    let got_binding = validated_binding_pair(got, &claims.entry_id)?;
    let expected_binding = validated_binding_pair(claims, &claims.entry_id)?;
    if got_binding != expected_binding {
        return Err(DaemonError::Refused(format!(
            "the start receipt for entry {} does not carry the run-key and questionnaire \
             binding this daemon asserted, so it is not stored",
            claims.entry_id
        )));
    }
    Ok(())
}

/// Validate the signed interval after its signature and identity have been checked. The
/// server owns both timestamps, so the daemon records exactly these signed values and never
/// accepts an interval that is malformed or already closed at its own start.
fn validate_signed_receipt_times(
    receipt: &StartReceipt,
    entry_id: &str,
) -> Result<(), DaemonError> {
    let started = parse_z(&receipt.claims.started_at).ok_or_else(|| {
        DaemonError::Refused(format!(
            "the start receipt for entry {entry_id} carries an unreadable signed started_at"
        ))
    })?;
    let deadline = parse_z(&receipt.claims.deadline_at).ok_or_else(|| {
        DaemonError::Refused(format!(
            "the start receipt for entry {entry_id} carries an unreadable signed deadline_at"
        ))
    })?;
    if deadline <= started {
        return Err(DaemonError::Refused(format!(
            "the start receipt for entry {entry_id} carries deadline_at {}, which is not strictly after started_at {}",
            receipt.claims.deadline_at, receipt.claims.started_at
        )));
    }
    Ok(())
}

fn validated_binding_pair(
    claims: &StartReceiptClaims,
    entry_id: &str,
) -> Result<Option<(String, String)>, DaemonError> {
    match (
        claims.run_public_key.as_deref(),
        claims.questionnaire_version.as_deref(),
    ) {
        (None, None) => Ok(None),
        (Some(key), Some(version)) => {
            let key = canonical_run_public_key(key).map_err(|_| {
                DaemonError::Refused(format!(
                    "the start receipt for entry {entry_id} carries a malformed run-key binding"
                ))
            })?;
            validate_questionnaire_version(version).map_err(|_| {
                DaemonError::Refused(format!(
                    "the start receipt for entry {entry_id} carries a malformed questionnaire binding"
                ))
            })?;
            Ok(Some((key, version.to_string())))
        }
        _ => Err(DaemonError::Refused(format!(
            "the start receipt for entry {entry_id} carries only half of the run-key and \
             questionnaire binding"
        ))),
    }
}

/// Derive the optional claims from the entry's persisted policy and actual run key.
pub(crate) fn binding_claims_for_entry(
    entry: &Entry,
) -> Result<(Option<String>, Option<String>), DaemonError> {
    match entry.receipt_binding() {
        ReceiptBindingPolicy::Legacy => Ok((None, None)),
        ReceiptBindingPolicy::NewEvent {
            questionnaire_version,
        }
        | ReceiptBindingPolicy::WorkspaceSubmission {
            questionnaire_version,
            ..
        } => {
            validate_questionnaire_version(questionnaire_version).map_err(|_| {
                DaemonError::Refused(format!(
                    "entry {} has an invalid stored questionnaire binding policy",
                    entry.entry_id
                ))
            })?;
            let key = canonical_run_public_key(&entry.keys.public_key_hex()?).map_err(|_| {
                DaemonError::Refused(format!(
                    "entry {} has no usable run key for its receipt binding",
                    entry.entry_id
                ))
            })?;
            Ok((Some(key), Some(questionnaire_version.clone())))
        }
    }
}

/// The registry's answer to whether this row owns a start receipt mirror.
pub(crate) enum PersistedReceipt {
    NoAuthoritativeStart,
    Authoritative {
        receipt: Box<StartReceipt>,
        compact_bytes: Vec<u8>,
    },
}

/// Revalidate the complete persisted start tuple against the pinned organizer key and the
/// validated seq-zero chain hash. Capture-closed recovery calls this before it acquires any
/// tailer or buffer authority; ordinary runtime rebuild uses the same check below.
pub(crate) fn validate_authoritative_receipt<'a>(
    entry: &'a Entry,
    genesis_hash: &str,
    pinned_server_key: &str,
) -> Result<&'a StartReceipt, DaemonError> {
    let (receipt, session_id, started_at, deadline_at, deadline_unix) = match (
        entry.receipt(),
        entry.session_id.as_deref(),
        entry.started_at.as_deref(),
        entry.deadline_at.as_deref(),
        entry.deadline_unix,
    ) {
        (
            Some(receipt),
            Some(session_id),
            Some(started_at),
            Some(deadline_at),
            Some(deadline_unix),
        ) => (receipt, session_id, started_at, deadline_at, deadline_unix),
        _ => {
            return Err(DaemonError::Refused(format!(
                "entry {} has a partial or contradictory persisted start tuple",
                entry.entry_id
            )));
        }
    };

    let (run_public_key, questionnaire_version) = binding_claims_for_entry(entry)?;
    let expected = StartReceiptClaims {
        entry_id: entry.entry_id.clone(),
        connector_id: entry.connector_id.clone(),
        workspace: entry.workspace.display().to_string(),
        session_id: session_id.to_string(),
        started_at: started_at.to_string(),
        deadline_at: deadline_at.to_string(),
        manifest_sha256: entry.manifest_sha256.clone(),
        chain_head: genesis_hash.to_string(),
        connector_dir_sha256: entry.connector_dir_sha256.clone(),
        run_public_key,
        questionnaire_version,
    };
    accept_receipt(receipt, &expected, pinned_server_key)?;
    // The receipt is the authority for both timestamps. `accept_receipt` has already
    // verified its signature, exact persisted string claims, and strict signed ordering;
    // only the separately persisted numeric mirror needs checking here. Re-deriving a
    // deadline from `started_at` would turn a valid server qualification window into the
    // legacy seven-day window during restart recovery.
    let signed_deadline_unix = parse_z(&receipt.claims.deadline_at).ok_or_else(|| {
        DaemonError::Refused(format!(
            "entry {} has no usable signed deadline instant",
            entry.entry_id
        ))
    })? as f64;
    if started_at != receipt.claims.started_at
        || deadline_at != receipt.claims.deadline_at
        || deadline_unix != signed_deadline_unix
    {
        return Err(DaemonError::Refused(format!(
            "entry {} has persisted start timing that does not match its signed receipt",
            entry.entry_id
        )));
    }
    Ok(receipt)
}

/// Validate the persisted start tuple against the already checked seq-zero chain hash.
pub(crate) fn persisted_receipt(
    entry: &Entry,
    genesis_hash: &str,
    pinned_server_key: &str,
) -> Result<PersistedReceipt, DaemonError> {
    // A corroborated pre-start row may already own its session id. The signed start
    // tuple begins with the receipt and timestamps; only `Started` requires the session
    // alongside that complete tuple below.
    let has_no_start_timestamps =
        entry.started_at.is_none() && entry.deadline_at.is_none() && entry.deadline_unix.is_none();

    if entry.state != EntryState::Started && entry.receipt().is_none() && has_no_start_timestamps {
        return Ok(PersistedReceipt::NoAuthoritativeStart);
    }

    if entry.state != EntryState::Started
        && matches!(entry.receipt_binding(), ReceiptBindingPolicy::Legacy)
        && has_no_start_timestamps
        && let Some(receipt) = entry.receipt()
    {
        receipt.verify_with(pinned_server_key).map_err(|e| {
            DaemonError::Refused(format!(
                "the legacy pre-start receipt for entry {} was not accepted: {e}",
                entry.entry_id
            ))
        })?;
        if receipt.claims.entry_id != entry.entry_id {
            return Err(DaemonError::Refused(format!(
                "the legacy pre-start receipt names entry {}, not registry entry {}",
                receipt.claims.entry_id, entry.entry_id
            )));
        }
        if receipt.claims.run_public_key.is_some() || receipt.claims.questionnaire_version.is_some()
        {
            return Err(DaemonError::Refused(format!(
                "the legacy pre-start receipt for entry {} carries a bound receipt pair",
                entry.entry_id
            )));
        }
        return Ok(PersistedReceipt::NoAuthoritativeStart);
    }

    if entry.state != EntryState::Started {
        return Err(DaemonError::Refused(format!(
            "entry {} has a partial or contradictory persisted start tuple",
            entry.entry_id
        )));
    }
    let receipt = validate_authoritative_receipt(entry, genesis_hash, pinned_server_key)?;
    Ok(PersistedReceipt::Authoritative {
        receipt: Box::new(receipt.clone()),
        compact_bytes: serde_json::to_vec(receipt)?,
    })
}

/// The deadline the entry window puts on `started_at`, or `None` when `started_at` is not
/// an RFC-3339 instant. The rule, not a number the receipt gets to assert.
pub fn deadline_for(started_at: &str) -> Option<String> {
    parse_z(started_at).map(|secs| format_z(secs.saturating_add(ENTRY_WINDOW_SECONDS)))
}

/// The one transition that starts a run: the verified receipt is stored, and `started_at`,
/// `deadline_at` and [`EntryState::Started`] are written **together**.
///
/// `asserted` is what the daemon sent. This function first checks those claims with the
/// server's signed `started_at` and `deadline_at`. The registry then
/// re-reads the row in its write transaction and rebuilds every row-owned claim from that
/// row; only the receipt request's seq-0 genesis link remains caller-supplied. A server
/// response or same-id replacement that moved either set of facts is refused and the entry
/// is untouched — [`Registry::commit_start`] writes nothing on a refusal, and
/// `Entry.receipt` has no unchecked writer.
///
/// **Why one transaction.** The goal-length gate keys on
/// `EntryState::Armed | EntryState::StartSignalSeen` — the states before `started_at`
/// exists. An entry carrying a `started_at` while still reading as pre-goal would have the
/// goal-length rule applied to every later prompt of the run; one reading as `Started` with
/// no `started_at` would report a deadline it cannot compute. The `debug_assert` below says
/// so in the build that can check it.
///
/// **Why the gate is a parameter.** A rule that lives only in a caller's discipline is a rule
/// the next caller forgets. The gate is asked whether it may start before anything is
/// written, and moved to [`crate::arming::GateState::Started`] after — so an entry cannot
/// acquire a `started_at` that no leg-1 record corroborated, whatever the caller does, and
/// the gate cannot be left saying `start_signal_seen` about a run the registry says has begun.
///
/// **Why the registry is asked as well.** The gate is one process's
/// memory of one arming window; the entry on disk is what every other reader of this run
/// agrees about. A fresh gate is three calls to build, so trusting only the in-memory one
/// would let a caller re-start an entry the registry already says is `Started` — moving
/// `started_at` forward under a held receipt, which defeats the receipt's immutability from
/// inside the daemon — or stamp a start onto an entry that was never armed, or one already
/// `Sealed` or `Refused`. So the entry is read first and only `Armed | StartSignalSeen`
/// proceeds; every other state is a named refusal and nothing is written.
///
/// **The session is the gate's**. `asserted.session_id` is ignored: the
/// receipt is checked against the session id of the leg-1 record that corroborated the
/// start, so a run is recorded under the session the daemon watched write rather than one
/// a caller named. A caller that requested its receipt with any other session gets a
/// refusal here naming `session_id`.
///
/// **The artifact identity predates the request.** `expected_paths` is captured from the
/// runtime entry that built the claims and opened the trace before the network await. It
/// travels unchanged into the registry transaction, so looking up a same-id replacement
/// after the server answers cannot adopt that replacement's artifact or trace directory.
pub fn record_start(
    registry: &Registry,
    entry_id: &str,
    gate: &mut ArmingGate,
    receipt: &StartReceipt,
    asserted: &StartReceiptClaims,
    expected_paths: &StartPathIdentity,
    pinned_server_key: &str,
) -> Result<Entry, DaemonError> {
    if !gate.may_stamp_started() {
        return Err(DaemonError::Refused(format!(
            "entry {entry_id} was not started: {}",
            gate.on_screen()
        )));
    }
    let entry = registry.get(entry_id)?.ok_or_else(|| {
        DaemonError::Missing(format!(
            "entry {entry_id} is not in the registry, so there is nothing to start"
        ))
    })?;
    if !matches!(entry.state, EntryState::Armed | EntryState::StartSignalSeen) {
        return Err(DaemonError::Refused(format!(
            "entry {entry_id} is {:?} in the registry, and a start is recorded only from \
             `Armed` or `StartSignalSeen`: an entry that has already started holds a \
             receipt whose `started_at` R5 makes immutable, and one that never armed has no \
             window a start could belong to",
            entry.state
        )));
    }
    let session_id = gate.session_id().ok_or_else(|| {
        DaemonError::Refused(format!(
            "entry {entry_id} was not started: the gate corroborated a start with no \
             session behind it, and the receipt names the session the record arrived in"
        ))
    })?;
    let started_at = receipt.claims.started_at.clone();
    let deadline_at = receipt.claims.deadline_at.clone();
    let (run_public_key, questionnaire_version) = binding_claims_for_entry(&entry)?;
    let expected = StartReceiptClaims {
        started_at: started_at.clone(),
        deadline_at: deadline_at.clone(),
        // The gate's, never the caller's: see the note on the session above.
        session_id: session_id.to_string(),
        run_public_key,
        questionnaire_version,
        ..asserted.clone()
    };
    accept_receipt(receipt, &expected, pinned_server_key)?;
    let stored = registry.commit_start(
        entry_id,
        receipt,
        &expected,
        expected_paths,
        session_id,
        pinned_server_key,
    )?;
    // The gate moves with the registry, in the same call, so the two cannot disagree about
    // whether the run has begun. It cannot refuse here — `may_stamp_started` was true above
    // and nothing between then and now observes anything — but the `?` is what makes that
    // a checked claim rather than an asserted one.
    gate.mark_started()?;
    debug_assert!(
        stored.started_at.is_some() && stored.state == EntryState::Started,
        "`started_at` and EntryState::Started are stamped in one transaction: Task 10's \
         length gate keys on the states before `started_at` exists"
    );
    debug_assert_eq!(
        gate.state().to_control(),
        goalcup_protocol::control::ArmingState::Started,
        "the gate and the registry name the same state at the moment `started_at` is stamped"
    );
    Ok(stored)
}

/// `("host:port", "path prefix")` from a base URL. The prefix is `""` for a bare origin.
///
/// **This is the cleartext rule, and it is the whole of it**.
///
/// `https` is refused rather than silently downgraded: this client speaks plaintext
/// HTTP/1.1, and a participant who configured a real endpoint is told what is missing rather
/// than left to read a connection error. A client that terminates TLS is a dependency, a
/// certificate store and a trust decision, and this one carries none of them.
///
/// `http` is accepted only for a loopback host: 127.0.0.0/8, `localhost`, or `[::1]`.
/// Plaintext to one of those never leaves the machine. Plaintext to anything else puts a
/// run's claims and its per-run public key on somebody's network in the clear, and does it
/// silently, because a request that succeeds says nothing about how it travelled. A url
/// with no scheme at all is an authority — the way a loopback dev server is usually
/// configured — and is held to exactly the same rule, because it is exactly as cleartext.
///
/// [`crate::config::DaemonConfig::validate`] calls this on `upload_url`, so the refusal
/// arrives when the daemon loads its config rather than when a run asks for its receipt.
pub(crate) fn split_url(url: &str) -> Result<(String, String), DaemonError> {
    let rest = match url.strip_prefix("http://") {
        Some(rest) => rest,
        None if url.starts_with("https://") => {
            return Err(DaemonError::Http(format!(
                "the upload url {url} is https, and nothing in this build speaks TLS: TLS \
                 termination lands with M4 packaging, and until it does an endpoint is \
                 either loopback cleartext or unreachable — it is not silently downgraded"
            )));
        }
        None => url,
    };
    let (authority, path) = match rest.find('/') {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, ""),
    };
    if authority.is_empty() {
        return Err(DaemonError::Http(format!(
            "the upload url {url} names no host"
        )));
    }
    if !is_loopback_authority(authority) {
        return Err(DaemonError::Http(format!(
            "the upload url {url} is cleartext http to {authority}, which is not loopback: \
             this build has no TLS, so a run's claims and its per-run public key would \
             travel a network in the clear. Loopback (127.0.0.0/8, localhost, [::1]) is \
             what cleartext is for; anything else waits for TLS in M4 packaging"
        )));
    }
    Ok((
        authority.to_string(),
        path.trim_end_matches('/').to_string(),
    ))
}

/// Whether `host[:port]` names this machine. `localhost` by name, and everything else by
/// parsing the host as an address rather than by matching its text: `127.0.0.1` is not the
/// only loopback address, and `127.0.0.1.example.com` is not one at all.
fn is_loopback_authority(authority: &str) -> bool {
    // `[::1]:8080` — the brackets are what make the colons in an IPv6 literal unambiguous.
    let host = match authority.strip_prefix('[') {
        Some(rest) => match rest.split_once(']') {
            Some((host, _)) => host,
            None => return false,
        },
        None => match authority.split_once(':') {
            Some((host, _)) => host,
            None => authority,
        },
    };
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(v4)) => v4.is_loopback(),
        Ok(std::net::IpAddr::V6(v6)) => v6.is_loopback(),
        Err(_) => false,
    }
}

/// One request, one response: `(status, body)`.
///
/// `Connection: close` so the body ends at EOF and this needs no chunked decoder, and the
/// read is capped at [`MAX_FILE_BYTES`] so a server that answers forever cannot be an
/// unbounded allocation in a daemon whose memory is bounded by constraint.
///
/// **Every blocking call has a deadline**. `connect_timeout` rather than
/// `connect`, and a read and a write timeout on the socket that survives them: the failure
/// they bound is one peer, reachable and silent, holding the only request a run makes and
/// the `spawn_blocking` worker under it. The address is resolved first because
/// `connect_timeout` takes an address and not a name — a host that resolves to several is
/// tried in the order the resolver gave, each on its own clock.
pub(crate) fn post(
    authority: &str,
    path: &str,
    body: &str,
    timeout: std::time::Duration,
) -> Result<(u16, String), DaemonError> {
    use std::io::{Read, Write};
    use std::net::ToSocketAddrs;
    let io = |source| DaemonError::Io {
        path: format!("{authority}{path}"),
        source,
    };
    let addrs: Vec<std::net::SocketAddr> = authority.to_socket_addrs().map_err(io)?.collect();
    let mut connected = None;
    let mut last = None;
    for addr in &addrs {
        match std::net::TcpStream::connect_timeout(addr, timeout) {
            Ok(stream) => {
                connected = Some(stream);
                break;
            }
            Err(e) => last = Some(e),
        }
    }
    let Some(mut stream) = connected else {
        return Err(match last {
            Some(e) => io(e),
            None => DaemonError::Http(format!("{authority} resolves to no address")),
        });
    };
    stream.set_read_timeout(Some(timeout)).map_err(io)?;
    stream.set_write_timeout(Some(timeout)).map_err(io)?;
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).map_err(io)?;
    stream.write_all(body.as_bytes()).map_err(io)?;
    let mut raw = Vec::new();
    stream
        .take(MAX_FILE_BYTES)
        .read_to_end(&mut raw)
        .map_err(io)?;
    let text = String::from_utf8_lossy(&raw).into_owned();
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| {
            DaemonError::Http(format!(
                "the receipt server's answer to {path} carries no status line"
            ))
        })?;
    let body = text.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");
    Ok((status, body.to_string()))
}

#[cfg(test)]
mod persisted_receipt_tests {
    use super::*;
    use crate::config::ReceiptBindingPolicy;
    use crate::registry::{ExpectedProcessHandoff, Registry, RunKeys, StartPathIdentity};
    use goalcup_devserver::{DevServer, ReceiptRequest};
    use goalcup_protocol::receipt::StartReceiptClaims;
    use std::path::PathBuf;

    fn run_public_key() -> String {
        hex::encode(
            ed25519_dalek::SigningKey::from_bytes(&[9u8; 32])
                .verifying_key()
                .to_bytes(),
        )
    }

    #[test]
    fn signed_short_deadline_survives_persisted_receipt_restart_for_participant_timing_attempts() {
        let server = DevServer::with_qualification_window(2)
            .expect("the positive qualification window builds");
        server.set_clock("2026-09-01T00:00:00Z");
        let pinned = goalcup_devserver::dev_server_public_key_hex();
        let signed_started_at = "2026-09-01T00:00:00Z";
        let signed_deadline_at = "2026-09-01T00:00:02Z";
        let signed_deadline_unix =
            parse_z(signed_deadline_at).expect("the organizer-signed deadline parses") as f64;
        let legacy_deadline_unix = (parse_z(signed_started_at)
            .expect("the organizer-signed start parses")
            + ENTRY_WINDOW_SECONDS) as f64;

        for (entry_id, participant_started_at, participant_deadline_at) in [
            (
                "persisted-short-deadline-shorten",
                signed_started_at,
                "2026-09-01T00:00:01Z",
            ),
            (
                "persisted-short-deadline-extend",
                "1999-01-01T00:00:00Z",
                "2036-10-01T00:00:00Z",
            ),
        ] {
            let dir = tempfile::tempdir().expect("the registry directory builds");
            let path = Registry::default_path(dir.path());
            let registry = Registry::open(&path).expect("the registry opens");
            let workspace = format!("/ws/{entry_id}");
            let mut entry = crate::registry::Entry::new(
                entry_id,
                "claude-code-cli",
                PathBuf::from(&workspace),
                dir.path().join("artifacts").join(entry_id),
                &"ab".repeat(32),
                "0.0.0-test",
                ReceiptBindingPolicy::Legacy,
                ExpectedProcessHandoff::NotRequired,
                RunKeys::generate(),
            );
            entry.state = EntryState::Armed;
            registry.put(&entry).expect("the entry stores");
            let expected_paths = StartPathIdentity::from_entry(&entry);
            let claims = StartReceiptClaims {
                entry_id: entry_id.to_string(),
                connector_id: entry.connector_id.clone(),
                workspace: workspace.clone(),
                session_id: "s1".to_string(),
                started_at: participant_started_at.to_string(),
                deadline_at: participant_deadline_at.to_string(),
                manifest_sha256: entry.manifest_sha256.clone(),
                chain_head: "cd".repeat(32),
                connector_dir_sha256: None,
                run_public_key: None,
                questionnaire_version: None,
            };
            let receipt = server
                .issue_receipt(ReceiptRequest {
                    claims,
                    run_public_key: run_public_key(),
                })
                .expect("the development server signs the organizer deadline");
            let stored = registry
                .commit_start(
                    entry_id,
                    &receipt,
                    &receipt.claims,
                    &expected_paths,
                    "s1",
                    &pinned,
                )
                .expect("the signed deadline stores");
            assert_eq!(stored.started_at.as_deref(), Some(signed_started_at));
            assert_eq!(stored.deadline_at.as_deref(), Some(signed_deadline_at));
            assert_eq!(stored.deadline_unix, Some(signed_deadline_unix));
            assert_ne!(stored.deadline_unix, Some(legacy_deadline_unix));
            drop(registry);

            let restarted = Registry::open(&path).expect("the registry restarts");
            let reloaded = restarted
                .get(entry_id)
                .expect("the restarted entry reads")
                .expect("the restarted entry remains present");
            let persisted = persisted_receipt(&reloaded, &receipt.claims.chain_head, &pinned)
                .expect("the signed short deadline survives restart recovery");
            let PersistedReceipt::Authoritative {
                receipt: recovered,
                compact_bytes,
            } = persisted
            else {
                panic!("a started row must retain an authoritative receipt");
            };
            assert_eq!(recovered.claims.started_at, signed_started_at);
            assert_eq!(recovered.claims.deadline_at, signed_deadline_at);
            assert_eq!(
                parse_z(signed_deadline_at),
                Some(parse_z(signed_started_at).unwrap() + 2)
            );
            assert_eq!(reloaded.started_at.as_deref(), Some(signed_started_at));
            assert_eq!(reloaded.deadline_at.as_deref(), Some(signed_deadline_at));
            assert_eq!(reloaded.deadline_unix, Some(signed_deadline_unix));
            assert_eq!(compact_bytes, serde_json::to_vec(&*recovered).unwrap());
        }
    }
}
