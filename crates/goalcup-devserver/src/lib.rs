//! The dev-only signer of receipts and acknowledgements.
//!
//! The daemon, the CLI and the verifier's round trip all need a server that says something
//! real: a receipt that `StartReceipt::verify_with` accepts, an acknowledgement whose
//! signature covers `acked_at`, and a refusal for a chunk nobody signed. A mock that signs
//! every acknowledgement with sixty-four zeros cannot pass V12 on a trace this system
//! produced, and leaves the whole anchor argument untestable.
//!
//! **A separate crate, on purpose.** It must not link into the shipped daemon binary, and
//! it is the one place allowed to depend on `goalcup-verifier` — as a dev-dependency (D3),
//! which is what makes the round trip expressible without the daemon ever seeing the
//! verifier.
//!
//! **What this server actually checks** is the whole point of it:
//!
//!   * the entry's run key is the one the RECEIPT named, so an entry has one participant
//!     and not whichever one uploaded first;
//!   * a chunk is refused unless a receipt was issued for its entry;
//!   * the envelopes a chunk carries are re-hashed link by link with the protocol crate's
//!     own `Envelope::expected_hash`, are contiguous over the range the claims name,
//!     continue from the hash this server last acknowledged, and end at the head the claims
//!     name — so `head` and `last_seq` are computed here rather than taken on the client's
//!     word. An acknowledgement that was not earned that way is not signed at all.
//!   * a receipt's `started_at` is this server's clock and its `deadline_at` is that plus
//!     the server's configured qualification window, both overwriting whatever the request
//!     asked for. A participant who could name their own start could name their own deadline.
//!
//! **It holds no run.** The envelopes a chunk carries are verified and then dropped; what
//! stays per entry is the receipt, the run key, and the last acknowledgement — five short
//! strings, whatever the length of the chain.
#![forbid(unsafe_code)]

mod router;

pub use router::dev_router;

use ed25519_dalek::{Signer, SigningKey};
use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::chain::GENESIS;
use goalcup_protocol::chunk::Chunk;
use goalcup_protocol::control::{format_z, parse_z};
use goalcup_protocol::envelope::EventBody;
use goalcup_protocol::provenance::{canonical_run_public_key, validate_questionnaire_version};
use goalcup_protocol::receipt::{StartReceipt, StartReceiptClaims};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};

/// DEVELOPMENT ONLY. This seed is public, in this repository, on purpose: it exists so
/// V12, V13 and the anchor timeline can be exercised without an event backend.
///
/// **`[5u8; 32]`: the placeholder IS the dev key.** Its verifying key is
/// `goalcup_verifier::PLACEHOLDER_SERVER_PUBLIC_KEY`, the single entry on
/// `SERVER_PUBLIC_KEYS`, and it is what the committed compliant fixture signs with
/// (`crates/goalcup-verifier/tests/common/compliant.rs`). Using the same seed here is the
/// whole point: a run this server signed passes ONLY under `--dev`, because a strict run
/// resolves `KeySource::Pinned` and V13 says `placeholder_server_key` — everybody holding
/// this repository can sign under it. The thing that makes such a run mean something is
/// the release gate: replacing the pinned key with the organiser's, after which no
/// devserver-signed trace verifies at all. One dev key, one Flag, one gate — rather than a
/// second public seed that would need its own suppression story.
pub const DEV_SERVER_SEED: [u8; 32] = [5u8; 32];

/// Printed on start and returned on `GET /v0/dev/banner`, so no operator can mistake
/// this for the event backend.
pub const DEV_BANNER: &str =
    "goalcup-devserver — DEVELOPMENT ONLY. Its key is public. Entries it signs are not entries.";

/// The default entry window, in seconds: 168 hours. A receipt's `deadline_at` is its
/// `started_at` plus the server's configured window, computed here, so the deadline is not a
/// number the participant chose. Qualification runs can select a shorter positive window;
/// old constructors retain this default.
pub const ENTRY_WINDOW_SECONDS: u64 = 604_800;

/// The receipt contract this server process signs for its entire lifetime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiptAuthority {
    Legacy,
    NewEvent { questionnaire_version: String },
}

impl ReceiptAuthority {
    pub fn new_event(questionnaire_version: impl Into<String>) -> Result<Self, DevError> {
        let authority = Self::NewEvent {
            questionnaire_version: questionnaire_version.into(),
        };
        authority.validate()?;
        Ok(authority)
    }

    fn validate(&self) -> Result<(), DevError> {
        if let Self::NewEvent {
            questionnaire_version,
        } = self
        {
            validate_questionnaire_version(questionnaire_version).map_err(|_| {
                DevError::BadReceiptAuthority(
                    "questionnaire_version is not a valid bounded version",
                )
            })?;
        }
        Ok(())
    }

    fn binding_for(
        &self,
        claims: &StartReceiptClaims,
        sidecar_run_public_key: &str,
    ) -> Result<(String, Option<String>, Option<String>), DevError> {
        match self {
            Self::Legacy => {
                if claims.run_public_key.is_some() || claims.questionnaire_version.is_some() {
                    return Err(DevError::BadReceiptBinding(
                        "legacy mode requires both new claims to be absent",
                    ));
                }
                let sidecar = canonical_run_key(sidecar_run_public_key)?;
                Ok((sidecar, None, None))
            }
            Self::NewEvent {
                questionnaire_version,
            } => {
                let claimed_key =
                    claims
                        .run_public_key
                        .as_deref()
                        .ok_or(DevError::BadReceiptBinding(
                            "new-event mode requires run_public_key",
                        ))?;
                let claimed_version =
                    claims
                        .questionnaire_version
                        .as_deref()
                        .ok_or(DevError::BadReceiptBinding(
                            "new-event mode requires questionnaire_version",
                        ))?;
                let sidecar = canonical_bound_run_key(sidecar_run_public_key)?;
                let claimed_key = canonical_bound_run_key(claimed_key)?;
                validate_questionnaire_version(claimed_version).map_err(|_| {
                    DevError::BadReceiptBinding("questionnaire_version is malformed")
                })?;
                if claimed_key != sidecar {
                    return Err(DevError::BadReceiptBinding(
                        "the claimed run key does not match the request sidecar",
                    ));
                }
                if claimed_version != questionnaire_version {
                    return Err(DevError::BadReceiptBinding(
                        "the claimed questionnaire version does not match this service",
                    ));
                }
                Ok((
                    sidecar.clone(),
                    Some(sidecar),
                    Some(questionnaire_version.clone()),
                ))
            }
        }
    }
}

/// The default signing key: `SigningKey::from_bytes(&DEV_SERVER_SEED)`. Nothing here
/// GENERATES a key — see `with_seed`, which is the only other way in.
pub fn dev_server_signing_key() -> SigningKey {
    SigningKey::from_bytes(&DEV_SERVER_SEED)
}

/// Equal to `goalcup_verifier::PLACEHOLDER_SERVER_PUBLIC_KEY`, asserted in `placeholder_seed.rs`.
pub fn dev_server_public_key_hex() -> String {
    hex::encode(dev_server_signing_key().verifying_key().to_bytes())
}

/// The body of `POST /v0/entries/{id}/receipt`: the claims the daemon asserts, and the
/// per-run public key it will sign its uploads with.
///
/// The key always rides beside the claims because it is the upload-verification input. In
/// new-event mode the same key is also present in the receipt claims, and the server
/// canonicalizes and compares the two before signing. In legacy mode those additive claims
/// stay absent, preserving the historical signed bytes. In both modes the accepted sidecar
/// key is pinned to the entry and every chunk is checked against it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptRequest {
    pub claims: StartReceiptClaims,
    /// Hex-encoded 32-byte Ed25519 verifying key.
    pub run_public_key: String,
}

/// Why the dev server refused.
///
/// There is no variant for "something failed internally": serialising a document of strings
/// and integers is not a failure this server can reach, and a variant for it would put a
/// case in every caller's `match` that no test can produce. Such a failure is reported as
/// `BadSignature` — no signature was produced, which is the fact the caller has to act on —
/// with the underlying error in the sentence.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DevError {
    /// The URL names one entry and the document another, so there is no entry under that
    /// path to file the document under. A real backend answers 404 for the same reason.
    #[error("this server has no entry {0} under that path")]
    UnknownEntry(String),
    /// A chunk arrived for an entry no receipt was ever issued for. An acknowledgement over
    /// such an entry would be a server signature over a run nobody declared — and there
    /// would be no run key to check the upload against, which is the concrete reason this
    /// is a refusal and not a convention.
    #[error("entry {0} has no start receipt: an entry is opened by a receipt, then uploaded to")]
    NoReceipt(String),
    /// A second request changed a claim or run key on an entry that already has an immutable
    /// receipt. Exact retries replay that receipt; this refusal prevents a participant from
    /// swapping the bound entry or key while retaining the first server decision.
    #[error("entry {0} already has a start receipt for different claims")]
    AlreadyIssued(String),
    /// The receipt request named a run key that is not a usable Ed25519 public key. Refused
    /// here, at the one moment it can be: pinning it would mean every later chunk failing
    /// with a cryptography error over a key nobody could have signed with.
    #[error("the run key is not a usable Ed25519 public key: {0}")]
    BadRunKey(String),
    #[error("the receipt authority is invalid: {0}")]
    BadReceiptAuthority(&'static str),
    #[error("the receipt binding is invalid: {0}")]
    BadReceiptBinding(&'static str),
    /// The chunk did not verify against the entry's run key: a key the receipt did not
    /// name, a signature that does not stand, or a `chunk_id` that does not name the range
    /// the document carries.
    #[error("the chunk was not acknowledged: {0}")]
    BadSignature(String),
    /// The chunk carried no envelopes. The claims alone are the client's word about a range
    /// it did not ship, and an acknowledgement over them would anchor nothing.
    #[error("the chunk for entry {0} carries no envelopes: there is nothing to acknowledge")]
    EmptyChunk(String),
    /// The envelopes the chunk carries do not make the chain the claims describe. Every
    /// sentence names what was expected and what arrived, because "chain invalid" is not a
    /// finding a client can act on.
    #[error("entry {entry_id}: {reason}")]
    ChainMismatch { entry_id: String, reason: String },
    /// The test hook. A closed laptop is the normal path (D9), so a daemon has to be
    /// testable against a server that is simply not there.
    #[error("this server is offline")]
    Offline,
    /// The server's clock does not read as an RFC-3339 instant, so a deadline cannot be
    /// computed from it. Reachable only through `set_clock`, which takes a string
    /// deliberately — an acknowledgement carries it verbatim — but a receipt's
    /// `deadline_at` is arithmetic and arithmetic needs a number.
    #[error("the server clock reads {0}, which is not an RFC-3339 instant")]
    Clock(String),
}

/// What this server remembers about one entry. Deliberately small: no envelopes, no chain,
/// no run.
#[derive(Debug)]
struct Entry {
    /// The one receipt, which is replayed for an exact retry and whose `chain_head` is read
    /// when the first chunk arrives. A second request that changes a bound claim is a 409.
    receipt: StartReceipt,
    /// The entry's run key, named by the receipt request and pinned here. Every chunk for
    /// this entry is checked against it; a chunk under a second key is a mismatch rather
    /// than a second opinion.
    run_public_key: String,
    /// The last acknowledgement this server signed for the entry, and the `chunk_id` that
    /// earned it. Two jobs, one field:
    ///
    ///   * `(seq, hash)` is the anchor — where the acknowledged chain ends, and therefore
    ///     what the next chunk has to continue from;
    ///   * the `chunk_id` is the idempotency key. A retry of the chunk just acknowledged is
    ///     the same document and gets the same acknowledgement back, with the same
    ///     `acked_at` and therefore the same signature. Earlier chunk ids are NOT kept:
    ///     with prev continuity in force, a re-upload of an older range cannot be
    ///     acknowledged anyway, so a table of them would grow forever to answer requests
    ///     that are refused on their own merits.
    last: Option<Acked>,
}

#[derive(Debug)]
struct Acked {
    chunk_id: String,
    ack: AckCheckpoint,
}

/// Everything the server remembers: one small record per entry, and the two test hooks.
#[derive(Debug, Default)]
struct State {
    entries: BTreeMap<String, Entry>,
    offline: bool,
    /// The clock a test set. `None` is the wall clock.
    clock: Option<String>,
}

impl State {
    /// The clock, as a string, for the fields that carry it verbatim. `set_clock` accepts a
    /// spelling `parse_z` would refuse, and an acknowledgement records it as it was set —
    /// a test that wants an unreadable timestamp is entitled to one.
    fn now_string(&self) -> String {
        self.clock.clone().unwrap_or_else(now_z)
    }

    /// The clock, as seconds, for the fields computed from it. A clock nothing can read is
    /// `DevError::Clock` rather than a guess.
    fn now_secs(&self) -> Result<u64, DevError> {
        match &self.clock {
            Some(set) => parse_z(set).ok_or_else(|| DevError::Clock(set.clone())),
            None => Ok(wall_clock_secs()),
        }
    }
}

fn format_protocol_timestamp(
    label: &'static str,
    field: &'static str,
    seconds: u64,
) -> Result<String, DevError> {
    let formatted = format_z(seconds);
    if parse_z(&formatted) != Some(seconds) {
        return Err(DevError::Clock(format!(
            "{label} timestamp cannot round-trip through protocol format_z/parse_z ({field}): {formatted}"
        )));
    }
    Ok(formatted)
}

/// A server that actually signs.
pub struct DevServer {
    key: SigningKey,
    authority: ReceiptAuthority,
    qualification_window_seconds: u64,
    qualification_window_configured: bool,
    state: Mutex<State>,
}

impl Default for DevServer {
    fn default() -> DevServer {
        DevServer::new()
    }
}

impl DevServer {
    /// Signs with `DEV_SERVER_SEED`, i.e. the placeholder. Every fixture bar
    /// one uses this.
    pub fn new() -> DevServer {
        DevServer::from_key(dev_server_signing_key(), ReceiptAuthority::Legacy)
    }

    /// `--key-seed <hex>`: sign with some other 32-byte seed. The ONLY code path in this
    /// crate that touches a key the verifier does not pin, and the only reason a caller
    /// ever needs `--dev-key` / `extra_server_keys`. A run signed this way records
    /// `key_source: "supplied"` and V13 raises `server_key_unpinned`, by design. The V1
    /// `tests/dev_key.rs` exercised that path until D48 removed it with the V1 tests.
    pub fn with_seed(seed: [u8; 32]) -> DevServer {
        DevServer::from_key(SigningKey::from_bytes(&seed), ReceiptAuthority::Legacy)
    }

    /// Use the placeholder signing key with an independently configured receipt contract.
    pub fn with_authority(authority: ReceiptAuthority) -> Result<DevServer, DevError> {
        authority.validate()?;
        Ok(DevServer::from_key(dev_server_signing_key(), authority))
    }

    /// Build the development signer with a short, server-owned qualification window. This is
    /// the organizer-only seam used by V1's automatic deadline journey; participant receipt
    /// fields are still overwritten by [`Self::issue_receipt`].
    pub fn with_qualification_window(seconds: u64) -> Result<DevServer, DevError> {
        Self::with_seed_and_authority_and_qualification_window(
            DEV_SERVER_SEED,
            ReceiptAuthority::Legacy,
            seconds,
        )
    }

    /// Variant of [`Self::with_qualification_window`] for an independently configured
    /// receipt authority. The default `with_authority` constructor remains unchanged.
    pub fn with_authority_and_qualification_window(
        authority: ReceiptAuthority,
        seconds: u64,
    ) -> Result<DevServer, DevError> {
        authority.validate()?;
        Self::from_key_with_window(dev_server_signing_key(), authority, seconds, true)
    }

    /// Use an explicit signing seed and independently configured receipt contract.
    pub fn with_seed_and_authority(
        seed: [u8; 32],
        authority: ReceiptAuthority,
    ) -> Result<DevServer, DevError> {
        authority.validate()?;
        Ok(DevServer::from_key(
            SigningKey::from_bytes(&seed),
            authority,
        ))
    }

    /// Variant of [`Self::with_seed_and_authority`] for a selected qualification window.
    pub fn with_seed_and_authority_and_qualification_window(
        seed: [u8; 32],
        authority: ReceiptAuthority,
        seconds: u64,
    ) -> Result<DevServer, DevError> {
        authority.validate()?;
        Self::from_key_with_window(SigningKey::from_bytes(&seed), authority, seconds, true)
    }

    /// The one constructor the other two go through. Nothing in this crate GENERATES a
    /// key: it is the seed above or a seed the caller named, and there is no third way in.
    fn from_key(key: SigningKey, authority: ReceiptAuthority) -> DevServer {
        // Existing constructors preserve the historical 168-hour behavior exactly.
        Self::from_key_with_window(key, authority, ENTRY_WINDOW_SECONDS, false)
            .expect("the historical default qualification window is positive")
    }

    fn from_key_with_window(
        key: SigningKey,
        authority: ReceiptAuthority,
        qualification_window_seconds: u64,
        qualification_window_configured: bool,
    ) -> Result<DevServer, DevError> {
        if qualification_window_seconds == 0 {
            return Err(DevError::BadReceiptAuthority(
                "qualification window must be strictly positive",
            ));
        }
        Ok(DevServer {
            key,
            authority,
            qualification_window_seconds,
            qualification_window_configured,
            state: Mutex::new(State::default()),
        })
    }

    pub fn public_key_hex(&self) -> String {
        hex::encode(self.key.verifying_key().to_bytes())
    }

    /// The development warning plus this process's immutable receipt mode.
    pub fn banner(&self) -> String {
        format!("{DEV_BANNER} {}", self.receipt_mode_banner())
    }

    /// The mode-specific part of the startup and HTTP banner.
    pub fn receipt_mode_banner(&self) -> String {
        match &self.authority {
            ReceiptAuthority::Legacy => "Receipt mode: legacy.".to_string(),
            ReceiptAuthority::NewEvent {
                questionnaire_version,
            } => format!("Receipt mode: new-event ({questionnaire_version})."),
        }
    }

    /// `POST /v0/entries/{id}/receipt` — opens the entry.
    ///
    /// Three things happen here and nowhere else. The run key the request names is pinned
    /// to the entry, so the entry has one participant from its first moment rather than
    /// from whoever uploads first. `started_at` is taken from this server's clock and
    /// `deadline_at` is set to that plus this server's configured qualification window, both
    /// **overwriting** whatever the request asked for: a participant who names their own
    /// start names their own deadline, and success after dispatch being immutable is
    /// exactly the rule that forbids it. With the qualification-window constructor, an exact
    /// retry for the same entry and bound claims returns the original signed receipt,
    /// including its original server-owned times. Existing default constructors preserve
    /// their one-shot refusal. In either mode, a second request that changes a bound claim is
    /// refused, under the same run key as under a different one, whatever the clock says.
    ///
    /// **Every previously established non-time claim is signed exactly as it arrived**,
    /// including `connector_dir_sha256`. Legacy signs the two additive binding slots as
    /// absent. New-event reconstructs them from the canonical accepted sidecar key and the
    /// server's immutable configured version. This server verifies the run key, not the
    /// connector: it holds no connector directory, ships no pins, and could not recompute
    /// that digest if it wanted to. Its signature is what makes the daemon's claim
    /// non-repudiable, and `goalcup-verifier`'s V13 is what decides whether the digest is
    /// one this build ships.
    pub fn issue_receipt(&self, request: ReceiptRequest) -> Result<StartReceipt, DevError> {
        let mut state = self.state();
        if state.offline {
            return Err(DevError::Offline);
        }
        let entry_id = request.claims.entry_id.clone();
        let (run_public_key, run_public_key_claim, questionnaire_version) = self
            .authority
            .binding_for(&request.claims, &request.run_public_key)?;
        if let Some(entry) = state.entries.get(&entry_id) {
            let existing = &entry.receipt.claims;
            let retry_matches = existing.connector_id == request.claims.connector_id
                && existing.workspace == request.claims.workspace
                && existing.session_id == request.claims.session_id
                && existing.manifest_sha256 == request.claims.manifest_sha256
                && existing.chain_head == request.claims.chain_head
                && existing.connector_dir_sha256 == request.claims.connector_dir_sha256
                && entry.run_public_key == run_public_key
                && existing.run_public_key.as_deref() == run_public_key_claim.as_deref()
                && existing.questionnaire_version.as_deref() == questionnaire_version.as_deref();
            if self.qualification_window_configured && retry_matches {
                // The participant timing fields are deliberately omitted from this comparison:
                // the server owns them, and a retry must replay the one signed decision even if
                // the request was reconstructed with different local timestamps.
                return Ok(entry.receipt.clone());
            }
            return Err(DevError::AlreadyIssued(entry_id));
        }
        let started = state.now_secs()?;
        let deadline = started
            .checked_add(self.qualification_window_seconds)
            .ok_or_else(|| {
                DevError::Clock(format!(
                    "timestamp addition overflow at {started} + {} seconds",
                    self.qualification_window_seconds
                ))
            })?;
        let requested = request.claims;
        let claims = StartReceiptClaims {
            entry_id: requested.entry_id,
            connector_id: requested.connector_id,
            workspace: requested.workspace,
            session_id: requested.session_id,
            started_at: format_protocol_timestamp("started", "started_at", started)?,
            deadline_at: format_protocol_timestamp("deadline", "deadline_at", deadline)?,
            manifest_sha256: requested.manifest_sha256,
            chain_head: requested.chain_head,
            connector_dir_sha256: requested.connector_dir_sha256,
            run_public_key: run_public_key_claim,
            questionnaire_version,
        };
        let receipt = StartReceipt::sign(claims, &self.key)
            .map_err(|e| DevError::BadSignature(format!("the receipt was not signed: {e}")))?;
        state.entries.insert(
            entry_id,
            Entry {
                receipt: receipt.clone(),
                run_public_key,
                last: None,
            },
        );
        Ok(receipt)
    }

    /// `POST /v0/entries/{id}/chunks` — verifies the upload and, if it stands, signs an
    /// acknowledgement over what the server itself computed.
    ///
    /// In order:
    ///
    ///  1. the entry must have a receipt, or there is no run key and no declared run
    ///     ([`DevError::NoReceipt`]);
    ///  2. `chunk.verify_with(entry.run_public_key)` — the receipt's key, which also
    ///     re-derives `chunk_id` from the range before the signature is even looked at;
    ///  3. a retry of the chunk just acknowledged returns that acknowledgement unchanged;
    ///  4. [`verify_envelopes`] re-hashes the carried chain, which is where `head` and
    ///     `last_seq` stop being the client's word;
    ///  5. only then is an `AckCheckpoint` built over `(entry_id, last_seq, last_hash,
    ///     now)` and signed.
    ///
    /// **There is no `AckCheckpoint::sign`, and this crate does not add one.**
    /// `StartReceipt` has a `sign` because the receipt's claims are a separate type; an
    /// acknowledgement is four fields and its own signature, so signing it is done here:
    /// build the `AckCheckpoint` with `server_signature: String::new()`, call
    /// `signing_bytes()`, sign those bytes with the dev key, and hex-encode the result
    /// into `server_signature`. Four lines in the one crate that holds a server key,
    /// rather than a signing method on a protocol type every daemon links.
    ///
    /// The envelopes are dropped once they have been checked. A dev server that kept them
    /// would hold the run; what it keeps instead is the last `(seq, hash)`, which is all
    /// the next chunk has to be checked against.
    pub fn accept_chunk(&self, chunk: &Chunk) -> Result<AckCheckpoint, DevError> {
        let mut state = self.state();
        if state.offline {
            return Err(DevError::Offline);
        }
        let entry_id = chunk.claims.entry_id.clone();
        let acked_at = state.now_string();
        let entry = state
            .entries
            .get_mut(&entry_id)
            .ok_or_else(|| DevError::NoReceipt(entry_id.clone()))?;

        chunk
            .verify_with(&entry.run_public_key)
            .map_err(|e| DevError::BadSignature(e.to_string()))?;

        // The retry of the chunk this entry was last acknowledged at: the same document, so
        // the same answer, rather than a second acknowledgement at a later time.
        if let Some(last) = &entry.last
            && last.chunk_id == chunk.claims.chunk_id
        {
            return Ok(last.ack.clone());
        }

        let (expected_first_seq, expected_prev, genesis_head) = match &entry.last {
            Some(last) => (last.ack.seq + 1, last.ack.hash.clone(), None),
            None => (
                0,
                GENESIS.to_string(),
                Some(entry.receipt.claims.chain_head.clone()),
            ),
        };
        let last_hash = verify_envelopes(
            chunk,
            &entry_id,
            expected_first_seq,
            &expected_prev,
            genesis_head.as_deref(),
        )?;

        let mut ack = AckCheckpoint {
            entry_id: entry_id.clone(),
            // The range the SERVER walked, not the range the claims asserted — they are
            // equal by the checks above, and this is the one of the two that was computed.
            seq: chunk.claims.first_seq + (chunk.envelopes.len() as u64 - 1),
            hash: last_hash,
            acked_at,
            server_signature: String::new(),
        };
        let bytes = ack.signing_bytes().map_err(|e| {
            DevError::BadSignature(format!("the acknowledgement was not signed: {e}"))
        })?;
        ack.server_signature = hex::encode(self.key.sign(&bytes).to_bytes());

        entry.last = Some(Acked {
            chunk_id: chunk.claims.chunk_id.clone(),
            ack: ack.clone(),
        });
        Ok(ack)
    }

    /// The highest sequence this server has acknowledged for an entry.
    pub fn acked_seq(&self, entry_id: &str) -> Option<u64> {
        self.acked_head(entry_id).map(|(seq, _)| seq)
    }

    /// The anchor: the sequence AND the chain hash this server last acknowledged. Both
    /// halves, because a sequence on its own is not enough — two
    /// different chains reach seq 3, and only one of them continues this entry's.
    pub fn acked_head(&self, entry_id: &str) -> Option<(u64, String)> {
        self.state()
            .entries
            .get(entry_id)
            .and_then(|e| e.last.as_ref())
            .map(|a| (a.ack.seq, a.ack.hash.clone()))
    }

    /// Test hook: make the server unreachable. Every entry point answers `Offline` and
    /// signs nothing, so an outage moves no anchor.
    pub fn set_offline(&self, offline: bool) {
        self.state().offline = offline;
    }

    /// Test hook: move its clock. `acked_at` is inside an acknowledgement's signature, so
    /// a server whose clock a test cannot set cannot produce the anchor timeline V12
    /// reads, and `started_at` is the clock too. The string is used verbatim where a
    /// field records it, including a spelling `parse_z` would refuse — a test that wants an
    /// unreadable timestamp is entitled to one, and a receipt asked for under such a clock
    /// is refused with `DevError::Clock` rather than dated by a guess.
    pub fn set_clock(&self, rfc3339: &str) {
        self.state().clock = Some(rfc3339.to_string());
    }

    /// The state, through a poisoned lock as readily as a healthy one.
    ///
    /// `unwrap_or_else(PoisonError::into_inner)` rather than `unwrap`: a panic in one test
    /// thread must not turn every later call into a second panic, and there is no
    /// invariant here that a half-finished write could break — each method holds the lock
    /// across one decision and the insertions that follow from it.
    fn state(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Bind the envelopes a chunk carries to the range, the head and the chain the server has
/// already acknowledged. Returns the hash of the last envelope: the head the acknowledgement
/// will name.
///
/// `Chunk::verify_with` deliberately checks none of this — its own documentation assigns the
/// obligation here, to "the side that has the chain to check them against". Without it a
/// chunk is a signed assertion that some chain reached some hash, and the server's
/// acknowledgement says only that the participant said so; `VISION.md`'s "hash-chained and
/// acknowledged by the server as they happen" is then false, because nothing the server
/// signed is bound to the chain at all.
///
/// Every hash comes from [`Envelope::expected_hash`], which is `link_hash(prev,
/// chain_bytes())` — the protocol crate's one definition of what a link covers. Nothing
/// here computes a digest.
///
/// `genesis_head` is `Some(chain_head)` when this is the entry's FIRST chunk: seq 0 then has
/// to be the baseline, and its hash has to be the one the receipt witnessed.
fn verify_envelopes(
    chunk: &Chunk,
    entry_id: &str,
    expected_first_seq: u64,
    expected_prev: &str,
    genesis_head: Option<&str>,
) -> Result<String, DevError> {
    let refuse = |reason: String| DevError::ChainMismatch {
        entry_id: entry_id.to_string(),
        reason,
    };
    let claims = &chunk.claims;
    if chunk.envelopes.is_empty() {
        return Err(DevError::EmptyChunk(entry_id.to_string()));
    }
    if claims.last_seq < claims.first_seq {
        return Err(refuse(format!(
            "the chunk claims seqs {}..={}, which runs backwards",
            claims.first_seq, claims.last_seq
        )));
    }
    if claims.first_seq != expected_first_seq {
        return Err(refuse(format!(
            "the chunk starts at seq {}, and the next unacknowledged sequence is {expected_first_seq}",
            claims.first_seq
        )));
    }
    let claimed_len = claims.last_seq - claims.first_seq + 1;
    if chunk.envelopes.len() as u64 != claimed_len {
        return Err(refuse(format!(
            "the chunk claims seqs {}..={} ({claimed_len} links) and carries {}",
            claims.first_seq,
            claims.last_seq,
            chunk.envelopes.len()
        )));
    }

    let mut prev = expected_prev.to_string();
    for (offset, envelope) in chunk.envelopes.iter().enumerate() {
        let seq = claims.first_seq + offset as u64;
        if envelope.entry_id != entry_id {
            return Err(refuse(format!(
                "the link at seq {seq} belongs to entry {}",
                envelope.entry_id
            )));
        }
        if envelope.link.seq != seq {
            return Err(refuse(format!(
                "the link in position {offset} of the range says seq {}, and the range says {seq}",
                envelope.link.seq
            )));
        }
        if envelope.link.prev != prev {
            return Err(refuse(format!(
                "the link at seq {seq} extends {}, and the chain this server acknowledged \
                 reached {prev}",
                envelope.link.prev
            )));
        }
        let expected = envelope
            .expected_hash(&prev)
            .map_err(|e| refuse(format!("the link at seq {seq} does not serialise: {e}")))?;
        if envelope.link.hash != expected {
            return Err(refuse(format!(
                "the link at seq {seq} records hash {}, and its own content hashes to \
                 {expected}",
                envelope.link.hash
            )));
        }
        prev = envelope.link.hash.clone();
    }

    if let Some(chain_head) = genesis_head {
        // The entry's first chunk. Seq 0 is the baseline, and the baseline is the link
        // the start receipt witnessed: if the two disagree, either the workspace the run
        // declared is not the workspace it uploaded, or the receipt belongs to another run.
        let first = &chunk.envelopes[0];
        if !matches!(first.body, EventBody::Baseline { .. }) {
            return Err(refuse(
                "seq 0 is the R3 baseline or the chain does not start where the receipt says \
                 it does"
                    .to_string(),
            ));
        }
        if first.link.hash != chain_head {
            return Err(refuse(format!(
                "seq 0 hashes to {}, and the receipt's chain_head witnesses {chain_head}",
                first.link.hash
            )));
        }
    }

    if claims.head != prev {
        return Err(refuse(format!(
            "the chunk claims head {}, and its last link hashes to {prev}",
            claims.head
        )));
    }
    Ok(prev)
}

/// A run key as 64 lowercase hex characters, having checked it decodes to a point on the
/// curve. Stored canonically so the entry's pinned key has one spelling; `Chunk::verify_with`
/// compares case-insensitively either way.
fn canonical_run_key(run_public_key: &str) -> Result<String, DevError> {
    let bytes: [u8; 32] = hex::decode(run_public_key)
        .map_err(|e| DevError::BadRunKey(e.to_string()))?
        .try_into()
        .map_err(|_| {
            DevError::BadRunKey("a key is 32 bytes, i.e. 64 hex characters".to_string())
        })?;
    ed25519_dalek::VerifyingKey::from_bytes(&bytes)
        .map_err(|e| DevError::BadRunKey(e.to_string()))?;
    Ok(hex::encode(bytes))
}

/// The stronger canonical binding grammar for new-event receipts. Kept apart
/// from `canonical_run_key` so legacy receipt issuance retains its historical acceptance.
fn canonical_bound_run_key(run_public_key: &str) -> Result<String, DevError> {
    canonical_run_public_key(run_public_key).map_err(|e| DevError::BadRunKey(e.to_string()))
}

/// The wall clock in seconds. A clock somehow before the epoch reads as the epoch rather
/// than panicking; nothing downstream would believe either number.
fn wall_clock_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The wall clock, RFC-3339 UTC, whole seconds — the spelling every other timestamp in
/// this protocol uses.
fn now_z() -> String {
    format_z(wall_clock_secs())
}
