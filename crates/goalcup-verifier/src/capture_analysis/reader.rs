//! The bounded, streaming protocol-v5 capture-integrity reader. It populates the verified
//! facts used by verifier reports.
//!
//! [`scan_capture_trace`] reads one trace directory through one retained, no-follow
//! `tracecap` root, the capability the trace writer uses. It reads the fixed names the
//! writer produces: `chain.jsonl` (required), `baseline.json`, plus `receipt.json`,
//! `acks.jsonl` and `seal.json` when they are present. There is no `trace.toml` and no
//! connector manifest, because a v5 capture trace has neither. This reader refuses a chain
//! whose first record is not v5.
//!
//! **Bounded.** One chain line is held at a time. Each line is read into a reused buffer
//! capped at `MAX_ENVELOPE_BYTES` (newline included) before it is decoded. Base64 is
//! decoded only after its encoded length is checked. The retained state is also bounded:
//! at most `MAX_PRODUCERS` producer summaries (a few flags each), `MAX_OPEN_STREAMS` live
//! streams and `MAX_OPEN_CONNECTIONS` live connections, at most `MAX_GOAL_STREAMS` launch goal
//! streams and as many completed goal streams, plus one baseline claim, one terminal and one
//! cutoff.
//! No other completed stream is kept and there is no per-record index, so a longer run
//! grows none of it. Acknowledgements stream alongside the chain in the same pass, one line
//! at a time.
//!
//! **Checks, in order, per line:** the line bound and its newline, the declared version
//! (before the body is decoded), the line being exactly the envelope's canonical
//! serialization, sequence, prior hash and recomputed canonical hash, then
//! the v5 phase and terminal rules. Those come from the public
//! [`goalcup_protocol::export::ChainPhaseReplay`] and are never re-derived here. After that
//! come one entry, the capture-record semantics this module owns, the receipt's witnessed
//! head, and any ACKs at this sequence.
//!
//! **What the callback means.** `emit` receives each capture record only after that record
//! has been validated individually. It is not a verdict on the trace. A trace can still be
//! invalid (a later break, a torn tail, a manifest mismatch) or incomplete (a gap, no
//! terminal). Only the final [`CaptureIntegrityReport`] decides whole-trace trust. A
//! callback error ends the scan as an I/O error, and no report is produced.

use super::lineage::Lineage;
use super::wire::SegmentView;
use crate::SERVER_PUBLIC_KEYS;
use crate::VerifyOptions;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use goalcup_platform::tracecap::{RefusalKind, TraceCapError, TraceDir};
use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::capture::{
    CaptureFrame, CaptureMeta, CaptureRecord, CaptureSource, CaptureTime, ConnectionEvent,
    EndReason, FrameEvent, GapReason, M2_PROTOCOL_VERSION, MAX_ENVELOPE_BYTES,
    MAX_METADATA_STRING_BYTES, MAX_OPEN_CONNECTIONS, MAX_OPEN_STREAMS, MAX_SEGMENT_BYTES,
    StreamKey, is_capture_id,
};
use goalcup_protocol::capture_status::AnchorState;
use goalcup_protocol::chain::GENESIS;
use goalcup_protocol::control::parse_z;
use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::export::{
    ACKS_FILE, BASELINE_FILE, ChainPhaseReplay, ExportError, RECEIPT_FILE, SEAL_FILE, TerminalKind,
    chain_parts, check_terminal_manifest, manifest_sha256, part_file_name,
};
use goalcup_protocol::provenance::{Rfc3339Utc, Sha256Digest, TerminationOutcomeV1};
use goalcup_protocol::receipt::StartReceipt;
use goalcup_protocol::seal::{GitState, SealManifest};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, BufRead, BufReader, Read};
use std::path::Path;

/// The capture-integrity report document schema version. 2 added the deadline facts
/// (`records_after_deadline`, `last_seq_at_or_before_deadline`, `last_ack_at`). 3 added the
/// identity (`entry_id`, `profile_id`) and `first_failure`. Schema 1 and 2 documents still
/// parse, those fields reading as absent.
pub(crate) const CAPTURE_INTEGRITY_SCHEMA_VERSION: u32 = 3;

/// The most characters of a recorded Git reason the report repeats.
const GIT_UNREADABLE_CHARS: usize = 200;
/// At most this many distinct finding codes are retained; the rest are counted into one
/// final `findings_omitted:<n>` code.
pub const MAX_FINDING_CODES: usize = 128;
/// Launch goal streams, and separately completed goal streams, per entry. It kept the value of
/// `MAX_PRODUCERS` when that bound was raised (D58), so these verdicts did not change.
const MAX_GOAL_STREAMS: usize = 128;

/// Whole-trace evidence integrity. Serialized words are frozen by capture-integrity schemas 1 and 2
/// and reused unchanged by the research report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CaptureIntegrity {
    Valid,
    Incomplete,
    Invalid,
}

/// Provenance of the key that verified this v5 trace's receipt and acknowledgements:
/// `pinned` is this build's allowlist (`SERVER_PUBLIC_KEYS`), `supplied` a key the caller
/// passed for this run (`goalcup-verify --dev-key`), `none` no key at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum KeySource {
    Pinned,
    Supplied,
    None,
}

/// A signed acknowledgement position the reader verified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedAckPosition {
    pub seq: u64,
    pub hash: String,
    pub acked_at: String,
}

/// A server-owned receipt clock the reader verified. Never fabricated when absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedReceiptClock {
    pub started_at: String,
    pub deadline_at: String,
}

/// Which record ended the trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureTerminalKind {
    LocalFinish,
    SignedFinalization,
}

/// The verified terminal record of a trace. `LocalFinish` retains its full `CaptureTime`;
/// a signed finalization carries wall-clock evidence only, so `observed` is `None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureTerminalFacts {
    pub seq: u64,
    pub hash: String,
    pub observed: Option<CaptureTime>,
    pub observed_at: String,
    pub kind: CaptureTerminalKind,
    pub reason: Option<String>,
    pub termination: Option<TerminationOutcomeV1>,
    pub seal_manifest_sha256: Option<String>,
}

/// One acknowledgement line: an entry id, two numbers, a timestamp and a signature.
const ACK_LINE_MAX_BYTES: usize = 4 * 1024;
/// A start receipt is a handful of bounded claims and a signature.
const RECEIPT_MAX_BYTES: u64 = 64 * 1024;
/// The fixed manifest a terminal binds. It is a whole document, so it is bounded as the
/// legacy fixed-JSON reader bounds one.
const SEAL_MAX_BYTES: u64 = 16 * 1024 * 1024;
/// Padded standard base64 of `MAX_SEGMENT_BYTES`: the encoded bound checked before decoding.
const MAX_SEGMENT_B64_BYTES: usize = MAX_SEGMENT_BYTES.div_ceil(3) * 4;

/// The verified integrity of one retained v5 capture trace.
///
/// `integrity` is `valid`, `invalid` or `incomplete`. `anchor` is `signed` only when a
/// start receipt verified under an accepted key and bound this chain. A local trace is
/// `local_only`, which is a normal pilot outcome and never a claim of signed authenticity.
/// `verified_ack`, `receipt_clock` and `terminal` are populated only after validation;
/// absence is `None`, never a fabricated value. The report states evidence status. It
/// does not guess delivery or live network state, and an ACK behind the terminal is
/// progress, not complete delivery.
///
/// **Identity** (`entry_id`, `profile_id`). They let a batch join a report to the server's rows
/// without carrying the directory name on the side. Both are read from the chain's validated
/// prefix, so they are as good as the chain, and `None` when the prefix does not reach them.
///
/// **First failure** (`first_failure`). Where the scan first found a problem, so a judge need not
/// infer it from `validated_records`. See [`FirstFailure`].
///
/// **Deadline facts** (`records_after_deadline`, `last_seq_at_or_before_deadline`). They compare
/// each record's `recorded_at` with the receipt's `deadline_at`. `recorded_at` is the participant
/// machine's own clock: the chain hash covers it, but nothing makes it true, and a participant who
/// sets the clock back can make `records_after_deadline` read 0. So they are a hint for a judge,
/// not proof, and they never change the exit. The trusted time is the server's: `last_ack_at`
/// (equal to `verified_ack.acked_at`, signed with the server key) set against
/// `receipt_clock.deadline_at`, and the server's own chunk acknowledgements.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureIntegrityReport {
    pub schema_version: u32,
    pub integrity: CaptureIntegrity,
    pub anchor: AnchorState,
    /// A terminal record was validated and bound: a local finish to its fixed `seal.json`,
    /// or a signed finalization to its verified receipt and cutoff.
    pub sealed: bool,
    /// Chain records (envelopes, baseline and terminal included) in the validated prefix.
    pub validated_records: u64,
    /// Bytes of that prefix, newlines included.
    pub validated_bytes: u64,
    /// The link hash of the last validated record.
    pub validated_head: Option<String>,
    /// Gap records in the validated prefix. Gap history is sticky: nothing later clears it.
    pub gap_count: u64,
    /// A gap, a producer recovery (explicit loss) or a producer epoch left open (unknown
    /// tail) means the retained coverage is not complete.
    pub coverage_incomplete: bool,
    /// The chain ends in an unterminated line. That is invalid evidence, and the
    /// validated prefix is reported as it stands.
    pub torn_tail: bool,
    /// Which key checked the receipt and ACK signatures: `pinned`, `supplied` or `none`.
    pub key_source: KeySource,
    pub findings: Vec<String>,
    /// Why the workspace's Git state could not be read at the baseline or the seal (D56), one
    /// sentence each, when it could not. Either makes the report incomplete.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub git_unreadable: Vec<String>,
    pub verified_ack: Option<VerifiedAckPosition>,
    pub receipt_clock: Option<VerifiedReceiptClock>,
    pub terminal: Option<CaptureTerminalFacts>,
    /// Validated chain records whose `recorded_at` is after the verified receipt's `deadline_at`.
    /// Not a verdict: see "Deadline facts" on the struct. `None` without a verified, head-bound
    /// receipt clock, never a guess.
    ///
    /// What it counts depends on the run. A run the participant finished early has 0. A run cut at
    /// its deadline has the cutoff and the finalization here, and any drain records, unless they fall
    /// in the deadline's own second: that second counts as before. So 0, 1, 2 or more all occur,
    /// and the count alone says little. Read `last_seq_at_or_before_deadline`.
    #[serde(default)]
    pub records_after_deadline: Option<u64>,
    /// The highest seq whose `recorded_at` is at or before `deadline_at` (whole seconds, so the
    /// deadline's own second counts). `None` without a verified, head-bound receipt clock, or when
    /// no record is that early. A record whose `recorded_at` does not parse is in neither count, so
    /// the two need not add up to `validated_records`.
    #[serde(default)]
    pub last_seq_at_or_before_deadline: Option<u64>,
    /// The `acked_at` of the last verified acknowledgement: the server's signed time. The same
    /// value as `verified_ack.acked_at`, repeated here beside the other deadline facts.
    #[serde(default)]
    pub last_ack_at: Option<String>,
    /// The entry the validated prefix belongs to: the `entry_id` every validated record carries,
    /// read from the first record that passed every check. `None` when no record validated.
    #[serde(default)]
    pub entry_id: Option<String>,
    /// The `profile_id` (connector id) of the first validated `Launch` record. `None` when the
    /// prefix has no Launch, as in a chain pruned past it, or when no record validated.
    #[serde(default)]
    pub profile_id: Option<String>,
    /// The first thing that kept the report from `valid`. `None` for a valid report, and for an
    /// incomplete one whose only cause is coverage (`gap_recorded`, `producer_recovered`,
    /// `producer_unclosed`), which are facts about the capture and not failures of the scan.
    #[serde(default)]
    pub first_failure: Option<FirstFailure>,
}

/// The first problem the scan found, in the order the scan looks: the receipt first (before the
/// chain, so `seq` is `None`), then the chain record by record in seq order, then the documents
/// and whole-chain facts (`seq` `None`). So a bad receipt hides a bad record at seq 5 here; both
/// are still in `findings`. An `invalid` report names its first violation; an
/// `incomplete` one names its first missing or unproven piece (`chain_unsealed`,
/// `seal_manifest_missing`, `chain_pruned_locally` and the like). The scan stops at the first
/// bad record, so the code is also the reason `validated_records` stops where it does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirstFailure {
    /// The chain position the scan was at. For a record that failed it is the seq that record
    /// should have had (so the seq of the first bad record, which is not in `validated_records`).
    /// A chain that starts above byte 0 (pruned locally) has no trusted position for its first
    /// record, so a failure on that record is `None` too.
    /// `None` for a problem in a document and not in the chain: the receipt, `baseline.json`,
    /// `seal.json`, or a fact only the whole chain shows (`launch_missing`, `chain_unsealed`).
    pub seq: Option<u64>,
    pub code: String,
}

impl CaptureIntegrityReport {
    /// The canonical local integrity exit. 2 is invalid evidence. A torn tail is always
    /// invalid and takes precedence over the unsealed exit. 1 is incomplete or unsealed
    /// coverage. 0 is complete valid evidence, and local-only anchoring alone never fails
    /// it. Exit 3 ("no usable report") belongs to the caller, because a report that exists
    /// has usable content.
    pub fn exit_code(&self) -> u8 {
        if self.torn_tail || self.integrity == CaptureIntegrity::Invalid {
            2
        } else if self.integrity == CaptureIntegrity::Incomplete
            || !self.sealed
            || self.coverage_incomplete
        {
            1
        } else {
            0
        }
    }
}

/// Scan one v5 capture trace directory, handing each individually validated capture record
/// to `emit`, and return the whole-trace report.
///
/// A rotated chain is read part by part as one stream (D55); a part missing between two others
/// is `chain_part_missing`. A chain whose first parts were removed on this machine is read from
/// its first surviving record: its links, terminal and acknowledgements are checked, the
/// capture-record rules that need the removed records are not, and the report is incomplete
/// with `chain_pruned_locally`, because only the server's copy can be judged whole.
///
/// `Err` means no usable report. That covers an unreadable or replaced trace artifact (the
/// no-follow containment policy refuses symbolic links and hard links), a chain whose
/// first record is not protocol v5 (this build reads no other), an I/O failure, or
/// `emit` failing. Evidence problems are never `Err`: they are findings in an `invalid` or
/// `incomplete` report.
pub fn scan_capture_trace(
    dir: &Path,
    options: &VerifyOptions,
    emit: &mut dyn FnMut(&CaptureRecord) -> io::Result<()>,
) -> io::Result<CaptureIntegrityReport> {
    let root = TraceDir::open_selected(dir).map_err(|error| trace_error("trace root", error))?;
    let keys = AcceptedKeys::new(options);
    let mut scan = Scan::new(&keys);
    scan.load_receipt(&root)?;
    let parts = match chain_parts(dir) {
        Ok(parts) => parts,
        Err(error) if error.kind() == io::ErrorKind::InvalidData => {
            scan.fail("chain_part_missing");
            return Ok(scan.report());
        }
        Err(error) => return Err(error),
    };
    let no_chain = || io::Error::new(io::ErrorKind::NotFound, "the trace has no chain.jsonl");
    let first = parts.first().ok_or_else(no_chain)?.start;
    let mut chain = Vec::with_capacity(parts.len());
    for part in &parts {
        chain.push(open_file(&root, &part_file_name(part.start))?.ok_or_else(no_chain)?);
    }
    if first > 0 {
        scan.pruned_before(first);
    }
    let mut acks = open_file(&root, ACKS_FILE)?.map(AckStream::new);
    scan.read_chain(chain, acks.as_mut(), emit)?;
    // What follows is about the documents and the chain as a whole, not one record.
    scan.at = None;
    if scan.chain_clean {
        scan.finish_acks(acks.as_mut())?;
        scan.bind_baseline(&root)?;
        scan.bind_terminal(&root)?;
    }
    Ok(scan.report())
}

/// [`scan_capture_trace`] with the default options: no extra server keys.
///
/// Naming `VerifyOptions` is reserved to the verifier binary and `goalcup doctor`: a walk of
/// `goalcup-cli/src` in `goalcup-cli/tests/full/preflight_doctor.rs` fails on any other file that
/// does. This wrapper lets the launcher's key scan, which only reads a trace's records, read
/// one with the defaults without naming the type.
pub fn scan_capture_trace_default(
    dir: &Path,
    emit: &mut dyn FnMut(&CaptureRecord) -> io::Result<()>,
) -> io::Result<CaptureIntegrityReport> {
    scan_capture_trace(dir, &VerifyOptions::default(), emit)
}

/// The report and its serialized document together: the one seam the binary prints and a
/// test compares, so the two cannot drift.
pub fn capture_integrity_document(
    dir: &Path,
    options: &VerifyOptions,
) -> io::Result<(CaptureIntegrityReport, String)> {
    let report = scan_capture_trace(dir, options, &mut |_| Ok(()))?;
    let text = serde_json::to_string_pretty(&report).map_err(io::Error::other)?;
    Ok((report, text))
}

/// One capture-record rule a record broke. Codes are stable report vocabulary.
type Violation = &'static str;

/// A trace capability failure as I/O. An I/O failure keeps its kind; a refused artifact is
/// `InvalidData` with a `trace: ` message, worded as the verifier's own reader worded it.
fn trace_error(name: &str, error: TraceCapError) -> io::Error {
    let refused = |message: String| io::Error::new(io::ErrorKind::InvalidData, message);
    match error {
        TraceCapError::Io { source, .. } => {
            io::Error::new(source.kind(), format!("{name}: {source}"))
        }
        TraceCapError::Refused {
            reason: RefusalKind::NotRegularFile,
            ..
        } => refused(format!("trace: trace path {name:?} is not a regular file")),
        TraceCapError::Refused {
            reason: RefusalKind::MultipleLinks,
            ..
        } => refused(format!("trace: trace path {name:?} is hard linked")),
        TraceCapError::Refused {
            reason: RefusalKind::UnsupportedRoot,
            ..
        } => refused(format!(
            "trace: selected {name} is empty or contains an interior NUL"
        )),
        other => refused(format!("trace: {name}: {other}")),
    }
}

/// Open one fixed trace name through the retained root. `None` only when it does not exist;
/// a replaced (symlinked, hard-linked, non-regular) artifact is an error, never followed.
fn open_file(root: &TraceDir, name: &str) -> io::Result<Option<std::fs::File>> {
    root.read_regular(name)
        .map_err(|error| trace_error(name, error))
}

enum Fixed {
    Absent,
    TooLarge,
    Bytes(Vec<u8>),
}

/// A whole fixed document, read through the retained root and bounded before it is buffered.
fn read_fixed(root: &TraceDir, name: &str, limit: u64) -> io::Result<Fixed> {
    let Some(file) = open_file(root, name)? else {
        return Ok(Fixed::Absent);
    };
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Ok(Fixed::TooLarge);
    }
    Ok(Fixed::Bytes(bytes))
}

/// The keys this run accepts a signature from: every key this build pins, then the keys
/// the caller supplied (`--dev-key`), pinned first so a pinned key is never reported as
/// borrowed. A v5 capture trace has no descriptor, so there is no descriptor key to admit.
struct AcceptedKeys<'a> {
    keys: Vec<(&'a str, KeySource)>,
}

impl<'a> AcceptedKeys<'a> {
    fn new(options: &'a VerifyOptions) -> Self {
        let mut keys: Vec<(&'a str, KeySource)> = SERVER_PUBLIC_KEYS
            .iter()
            .map(|key| (*key, KeySource::Pinned))
            .collect();
        for key in &options.extra_server_keys {
            if !keys
                .iter()
                .any(|(known, _)| known.eq_ignore_ascii_case(key))
            {
                keys.push((key.as_str(), KeySource::Supplied));
            }
        }
        Self { keys }
    }

    fn named(&self, public_key: &str) -> Option<(&'a str, KeySource)> {
        self.keys
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(public_key))
            .copied()
    }

    fn verifying(&self, ack: &AckCheckpoint) -> Option<(&'a str, KeySource)> {
        self.keys
            .iter()
            .find(|(key, _)| ack.verify(key).is_ok())
            .copied()
    }
}

/// Distinct finding codes, bounded.
#[derive(Debug, Default)]
struct Findings {
    codes: Vec<String>,
    omitted: u64,
}

impl Findings {
    fn add(&mut self, code: &str) {
        if self.codes.iter().any(|known| known == code) {
            return;
        }
        if self.codes.len() < MAX_FINDING_CODES {
            self.codes.push(code.to_string());
        } else {
            self.omitted = self.omitted.saturating_add(1);
        }
    }

    fn finish(mut self) -> Vec<String> {
        if self.omitted > 0 {
            self.codes
                .push(format!("findings_omitted:{}", self.omitted));
        }
        self.codes
    }
}

/// The two envelope fields admitted before the body is decoded, so an unknown future body
/// reads as an unsupported version rather than as a parse failure.
#[derive(Deserialize)]
struct VersionSeq {
    protocol_version: u32,
    seq: u64,
}

fn export_code(error: &ExportError) -> Violation {
    match error {
        ExportError::UnsupportedProtocolVersion { .. } => "protocol_version_unsupported",
        ExportError::InvalidCaptureEnvelope { .. } => "capture_envelope_invalid",
        _ => "chain_phase_invalid",
    }
}

/// A start receipt that verified under an accepted key.
struct VerifiedReceipt {
    receipt: StartReceipt,
    digest: Sha256Digest,
    /// Its `chain_head` names a link of the validated chain.
    head_bound: bool,
    /// Its entry and clocks hold. False once any binding failed.
    bound: bool,
}

/// The cutoff a signed finalization closes, kept from the one pass.
struct CutoffFacts {
    receipt_sha256: Sha256Digest,
    deadline_at: String,
}

struct Scan<'k> {
    keys: &'k AcceptedKeys<'k>,
    findings: Findings,
    invalid: bool,
    /// An evidence fact left the coverage or the terminal unproven (not a violation).
    incomplete: bool,
    torn_tail: bool,
    /// The chain was read to its end with no chain-level violation.
    chain_clean: bool,
    /// The chain's first parts were removed on this machine (D55).
    pruned: bool,
    /// The seq of the first record read: 0, or the first surviving one of a pruned chain.
    first_seq: u64,
    validated_records: u64,
    validated_bytes: u64,
    head: String,
    entry_id: Option<String>,
    replay: ChainPhaseReplay,
    capture: CaptureState,
    /// The seq-0 baseline's claim about `baseline.json`: its digest and file count.
    baseline: Option<(String, u64)>,
    terminal: Option<Envelope>,
    cutoff: Option<CutoffFacts>,
    receipt_present: bool,
    receipt: Option<VerifiedReceipt>,
    /// The key that verified the receipt or, without one, the first ACK.
    anchor: Option<(&'k str, KeySource)>,
    acks_failed: bool,
    verified_ack: Option<VerifiedAckPosition>,
    sealed: bool,
    terminal_facts: Option<CaptureTerminalFacts>,
    git_unreadable: Vec<String>,
    /// The bound baseline recorded a clone's Git state, read or not.
    baseline_git: bool,
    /// The receipt's `deadline_at` in whole seconds, once its clocks read as ordered.
    deadline_secs: Option<u64>,
    records_after_deadline: u64,
    last_seq_at_or_before_deadline: Option<u64>,
    /// The profile of the first validated Launch.
    profile_id: Option<String>,
    /// The chain position of the record being read, for `first_failure`. `None` outside the
    /// chain pass, and for a pruned chain's first record, whose position is only its own claim.
    at: Option<u64>,
    /// The entry of the first record that passed every check, for the report's `entry_id`.
    validated_entry: Option<String>,
    /// The first `fail` (an invalid verdict) and the first `short` (an incomplete one).
    first_invalid: Option<FirstFailure>,
    first_incomplete: Option<FirstFailure>,
}

impl<'k> Scan<'k> {
    fn new(keys: &'k AcceptedKeys<'k>) -> Self {
        Self {
            keys,
            findings: Findings::default(),
            invalid: false,
            incomplete: false,
            torn_tail: false,
            chain_clean: false,
            pruned: false,
            first_seq: 0,
            validated_records: 0,
            validated_bytes: 0,
            head: GENESIS.to_string(),
            entry_id: None,
            replay: ChainPhaseReplay::new(),
            capture: CaptureState::default(),
            baseline: None,
            terminal: None,
            cutoff: None,
            receipt_present: false,
            receipt: None,
            anchor: None,
            acks_failed: false,
            verified_ack: None,
            sealed: false,
            terminal_facts: None,
            git_unreadable: Vec::new(),
            baseline_git: false,
            deadline_secs: None,
            records_after_deadline: 0,
            last_seq_at_or_before_deadline: None,
            profile_id: None,
            at: None,
            validated_entry: None,
            first_invalid: None,
            first_incomplete: None,
        }
    }

    /// The parts before logical byte `start` were removed on this machine: offsets stay
    /// logical, and the phase is the one an open chain had where they end.
    fn pruned_before(&mut self, start: u64) {
        self.pruned = true;
        self.validated_bytes = start;
        self.replay = ChainPhaseReplay::after_pruned_prefix();
        self.short("chain_pruned_locally");
    }

    fn fail(&mut self, code: &str) {
        self.invalid = true;
        if self.first_invalid.is_none() {
            self.first_invalid = Some(FirstFailure {
                seq: self.at,
                code: code.to_string(),
            });
        }
        self.findings.add(code);
    }

    fn short(&mut self, code: &str) {
        self.incomplete = true;
        if self.first_incomplete.is_none() {
            self.first_incomplete = Some(FirstFailure {
                seq: self.at,
                code: code.to_string(),
            });
        }
        self.findings.add(code);
    }

    /// The optional start receipt: absent is normal local capture; present, it must verify
    /// under an accepted key and carry readable, ordered clocks. Its entry and witnessed
    /// head are bound as the chain streams.
    ///
    /// A zero-length `receipt.json` is treated as absent. An empty optional service
    /// artifact is normal local capture, not a malformed one.
    fn load_receipt(&mut self, root: &TraceDir) -> io::Result<()> {
        let bytes = match read_fixed(root, RECEIPT_FILE, RECEIPT_MAX_BYTES)? {
            Fixed::Absent => return Ok(()),
            Fixed::Bytes(bytes) if bytes.is_empty() => return Ok(()),
            Fixed::TooLarge => {
                self.receipt_present = true;
                self.fail("receipt_too_large");
                return Ok(());
            }
            Fixed::Bytes(bytes) => bytes,
        };
        self.receipt_present = true;
        let Ok(receipt) = serde_json::from_slice::<StartReceipt>(&bytes) else {
            self.fail("receipt_malformed");
            return Ok(());
        };
        let Some((key, source)) = self.keys.named(&receipt.public_key) else {
            self.fail("receipt_key_not_accepted");
            return Ok(());
        };
        if receipt.verify_with(key).is_err() {
            self.fail("receipt_signature_invalid");
            return Ok(());
        }
        // A run key the receipt names that is not a usable key is the receipt's fault, and is
        // reported as one, not as a Launch field later.
        if let Some(run_key) = &receipt.claims.run_public_key
            && goalcup_protocol::provenance::canonical_run_public_key(run_key).is_err()
        {
            self.fail("receipt_malformed");
            return Ok(());
        }
        self.anchor = Some((key, source));
        let Ok(digest) = receipt.canonical_digest() else {
            self.fail("receipt_malformed");
            return Ok(());
        };
        let claims = &receipt.claims;
        let clocks = Rfc3339Utc::try_from(claims.started_at.as_str()).is_ok()
            && Rfc3339Utc::try_from(claims.deadline_at.as_str()).is_ok()
            && matches!(
                (parse_z(&claims.started_at), parse_z(&claims.deadline_at)),
                (Some(started), Some(deadline)) if started <= deadline
            );
        if !clocks {
            self.fail("receipt_clock_invalid");
        } else {
            self.deadline_secs = parse_z(&claims.deadline_at);
        }
        self.receipt = Some(VerifiedReceipt {
            receipt,
            digest,
            head_bound: false,
            bound: clocks,
        });
        Ok(())
    }

    /// Read the chain's parts, in order, as one stream.
    fn read_chain(
        &mut self,
        parts: Vec<std::fs::File>,
        mut acks: Option<&mut AckStream>,
        emit: &mut dyn FnMut(&CaptureRecord) -> io::Result<()>,
    ) -> io::Result<()> {
        let limit = MAX_ENVELOPE_BYTES as u64;
        let mut parts = parts.into_iter();
        let Some(first) = parts.next() else {
            self.chain_clean = true;
            return Ok(());
        };
        let mut reader = BufReader::new(first);
        let mut line = Vec::with_capacity(16 * 1024);
        loop {
            line.clear();
            // The position a failure here is reported at: the seq this record should have.
            self.at = if self.pruned && self.validated_records == 0 {
                None
            } else {
                Some(self.first_seq + self.validated_records)
            };
            let read = (&mut reader).take(limit).read_until(b'\n', &mut line)?;
            if read == 0 {
                match parts.next() {
                    Some(next) => {
                        reader = BufReader::new(next);
                        continue;
                    }
                    None => {
                        self.chain_clean = true;
                        return Ok(());
                    }
                }
            }
            if line.last() != Some(&b'\n') {
                if read as u64 == limit {
                    // Refused at the bound; the rest of the line is never read.
                    self.fail("envelope_too_large");
                } else {
                    self.torn_tail = true;
                    self.fail("torn_tail");
                }
                return Ok(());
            }
            line.pop();
            let Ok(header) = serde_json::from_slice::<VersionSeq>(&line) else {
                self.fail("record_malformed");
                return Ok(());
            };
            if self.validated_records == 0 && header.protocol_version != M2_PROTOCOL_VERSION {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "chain.jsonl begins with a protocol v{} record, not a protocol v5 \
                         capture chain, and this verifier reads no other",
                        header.protocol_version
                    ),
                ));
            }
            if let Err(error) = self
                .replay
                .check_version(header.seq, header.protocol_version)
            {
                self.fail(export_code(&error));
                return Ok(());
            }
            let Ok(env) = serde_json::from_slice::<Envelope>(&line) else {
                self.fail("record_malformed");
                return Ok(());
            };
            // The link hash covers the decoded content only, so any other spelling of the
            // line (an unknown key, whitespace, a carriage return) is bytes nothing covers.
            if !serde_json::to_vec(&env).is_ok_and(|canonical| canonical == line) {
                self.fail("record_not_canonical");
                return Ok(());
            }
            if self.pruned && self.validated_records == 0 {
                // The links before this one were removed: its own claim is the position, and
                // its hash is still recomputed over that `prev`.
                self.first_seq = env.link.seq;
                self.head.clone_from(&env.link.prev);
            }
            let seq = self.first_seq + self.validated_records;
            if env.link.seq != seq {
                self.fail("link_seq_mismatch");
                return Ok(());
            }
            if env.link.prev != self.head {
                self.fail("link_prev_mismatch");
                return Ok(());
            }
            if !env
                .expected_hash(&self.head)
                .is_ok_and(|hash| hash == env.link.hash)
            {
                self.fail("link_hash_mismatch");
                return Ok(());
            }
            if let Err(error) = self.replay.push(&env) {
                self.fail(export_code(&error));
                return Ok(());
            }
            match &self.entry_id {
                None => {
                    let mismatch = self.receipt.as_mut().is_some_and(|receipt| {
                        let differs = receipt.receipt.claims.entry_id != env.entry_id;
                        if differs {
                            receipt.bound = false;
                        }
                        differs
                    });
                    if mismatch {
                        // Not a stop: the chain is still read on.
                        self.fail("receipt_entry_mismatch");
                    }
                    self.entry_id = Some(env.entry_id.clone());
                }
                Some(entry) if *entry != env.entry_id => {
                    self.fail("entry_id_mismatch");
                    return Ok(());
                }
                Some(_) => {}
            }
            // A pruned chain's records name streams and producers its removed parts opened.
            if !self.pruned
                && let EventBody::Capture { record } = &env.body
                && let Err(code) = self.capture.apply(record)
            {
                self.fail(code);
                return Ok(());
            }
            if let EventBody::Capture {
                record:
                    CaptureRecord::Launch {
                        run_public_key: Some(launch_key),
                        ..
                    },
            } = &env.body
                && let Some(receipt_key) = self
                    .receipt
                    .as_ref()
                    .and_then(|r| r.receipt.claims.run_public_key.as_deref())
                && goalcup_protocol::provenance::canonical_run_public_key(receipt_key).as_deref()
                    != Ok(launch_key.as_str())
            {
                self.fail("record_field_invalid");
                return Ok(());
            }

            // Individually validated.
            self.validated_records += 1;
            self.validated_bytes += read as u64;
            self.head.clone_from(&env.link.hash);
            if self.validated_entry.is_none() {
                self.validated_entry = Some(env.entry_id.clone());
            }
            if self.profile_id.is_none()
                && let EventBody::Capture {
                    record: CaptureRecord::Launch { profile_id, .. },
                } = &env.body
            {
                self.profile_id = Some(profile_id.clone());
            }
            if let (Some(deadline), Some(stamped)) = (self.deadline_secs, env.recorded_at_secs()) {
                if stamped > deadline {
                    self.records_after_deadline += 1;
                } else {
                    self.last_seq_at_or_before_deadline = Some(seq);
                }
            }
            if let Some(receipt) = &mut self.receipt
                && receipt.receipt.claims.chain_head == env.link.hash
            {
                receipt.head_bound = true;
            }
            if let EventBody::Baseline {
                manifest_sha256,
                file_count,
            } = &env.body
            {
                self.baseline = Some((manifest_sha256.clone(), *file_count));
            }
            if let EventBody::CutoffReachedV1(cutoff) = &env.body {
                self.cutoff = Some(CutoffFacts {
                    receipt_sha256: cutoff.receipt_sha256.clone(),
                    deadline_at: cutoff.deadline_at.as_str().to_string(),
                });
            }
            if let Some(acks) = acks.as_deref_mut() {
                self.acks_at(acks, seq, &env.link.hash)?;
            }
            if let EventBody::Capture { record } = &env.body {
                emit(record)?;
            }
            if self.terminal.is_none() && self.replay.terminal().is_some() {
                self.terminal = Some(env);
            }
        }
    }

    /// Consume every acknowledgement naming `seq`, the link just validated.
    fn acks_at(&mut self, acks: &mut AckStream, seq: u64, hash: &str) -> io::Result<()> {
        while !self.acks_failed {
            if let Err(code) = acks.fill()? {
                self.acks_failed = true;
                self.fail(code);
                break;
            }
            let Some(ack) = acks.pending.as_ref() else {
                break;
            };
            if ack.seq > seq {
                break;
            }
            let ack = acks.pending.take().expect("checked above");
            if self.pruned && ack.seq < self.first_seq {
                // It acknowledges a record removed on this machine: nothing here to check.
                continue;
            }
            if ack.seq < seq {
                // Monotonic ACKs are consumed at their own link; one behind it is out of order.
                self.acks_failed = true;
                self.fail("ack_not_monotonic");
                break;
            }
            if let Err(code) = self.check_ack(&ack, hash) {
                self.acks_failed = true;
                self.fail(code);
                break;
            }
            self.verified_ack = Some(VerifiedAckPosition {
                seq: ack.seq,
                hash: ack.hash,
                acked_at: ack.acked_at,
            });
        }
        Ok(())
    }

    fn check_ack(&mut self, ack: &AckCheckpoint, hash: &str) -> Result<(), Violation> {
        if self.entry_id.as_deref() != Some(ack.entry_id.as_str()) {
            return Err("ack_entry_mismatch");
        }
        if ack.hash != hash {
            return Err("ack_hash_mismatch");
        }
        match self.anchor {
            Some((key, _)) => ack.verify(key).map_err(|_| "ack_signature_invalid"),
            None => match self.keys.verifying(ack) {
                Some(found) => {
                    self.anchor = Some(found);
                    Ok(())
                }
                None => Err("ack_signature_unverified"),
            },
        }
    }

    /// After a clean chain: an ACK still pending names a head the chain does not hold.
    fn finish_acks(&mut self, acks: Option<&mut AckStream>) -> io::Result<()> {
        let Some(acks) = acks else {
            return Ok(());
        };
        if self.acks_failed {
            return Ok(());
        }
        match acks.fill()? {
            Err(code) => self.fail(code),
            Ok(()) if acks.pending.is_some() => self.fail("ack_beyond_chain"),
            Ok(()) => {}
        }
        Ok(())
    }

    /// Bind `baseline.json` to the seq-0 baseline claim: the digest of the manifest it
    /// holds, its file count and its entry. Absent or empty is a missing artifact
    /// (incomplete); anything else that does not agree is invalid.
    fn bind_baseline(&mut self, root: &TraceDir) -> io::Result<()> {
        let Some((digest, file_count)) = self.baseline.take() else {
            return Ok(());
        };
        let bytes = match read_fixed(root, BASELINE_FILE, SEAL_MAX_BYTES)? {
            Fixed::Absent => {
                self.short("baseline_manifest_missing");
                return Ok(());
            }
            Fixed::Bytes(bytes) if bytes.is_empty() => {
                self.short("baseline_manifest_missing");
                return Ok(());
            }
            Fixed::TooLarge => {
                self.fail("baseline_manifest_too_large");
                return Ok(());
            }
            Fixed::Bytes(bytes) => bytes,
        };
        let Ok(manifest) = serde_json::from_slice::<SealManifest>(&bytes) else {
            self.fail("baseline_manifest_malformed");
            return Ok(());
        };
        let bound = manifest_sha256(&manifest).is_ok_and(|own| own == digest)
            && manifest.files.len() as u64 == file_count
            && self.entry_id.as_deref() == Some(manifest.entry_id.as_str());
        if !bound {
            self.fail("baseline_manifest_mismatch");
        } else {
            self.baseline_git = manifest.git.is_some();
            self.git_read(&manifest, "baseline");
        }
        Ok(())
    }

    /// A bound manifest whose clone Git could not read: the trace is incomplete, and the report
    /// says why (D56). The reason is the daemon's sentence, at most [`GIT_UNREADABLE_CHARS`]
    /// characters of it, control characters replaced.
    fn git_read(&mut self, manifest: &SealManifest, at: &str) {
        if let Some(GitState::Unreadable { unreadable }) = &manifest.git {
            self.short(&format!("{at}_git_unreadable"));
            let why: String = unreadable
                .chars()
                .take(GIT_UNREADABLE_CHARS)
                .map(|c| if c.is_control() { '?' } else { c })
                .collect();
            self.git_unreadable.push(format!(
                "the workspace's Git state could not be read at the {at}: {why}"
            ));
        }
    }

    /// Bind the validated terminal to what it claims. A local finish binds its fixed
    /// `seal.json`, with no receipt prerequisite. A signed finalization binds its verified
    /// receipt and its cutoff. The two terminal modes are never interchangeable. The V1 seal
    /// terminal, which the phase replay still admits, ends no v5 capture chain: no v5 writer
    /// produces one, so a v5 chain that ends in one is invalid.
    fn bind_terminal(&mut self, root: &TraceDir) -> io::Result<()> {
        // A pruned chain may have lost the link a receipt witnessed.
        if let Some(receipt) = &self.receipt
            && !receipt.head_bound
            && !self.pruned
        {
            self.fail("receipt_chain_head_unbound");
        }
        let Some(terminal) = self.replay.terminal() else {
            self.short("chain_unsealed");
            return Ok(());
        };
        let Some(env) = self.terminal.take() else {
            self.short("chain_unsealed");
            return Ok(());
        };
        let bound = match terminal.kind {
            TerminalKind::LocalFinish => self.bind_fixed_manifest(root, &env)?,
            TerminalKind::FinalizationV1 => self.bind_signed_finalization(&env),
            TerminalKind::LegacySeal => {
                self.fail("chain_phase_invalid");
                false
            }
        };
        if bound {
            self.terminal_facts = terminal_facts(&env);
            self.sealed = self.terminal_facts.is_some();
        }
        Ok(())
    }

    /// A zero-length `seal.json` is reported as missing (incomplete), like an absent one.
    /// `write_local_finish` chains the terminal before it creates and writes the fixed
    /// file, so a crash in between leaves exactly that.
    fn bind_fixed_manifest(&mut self, root: &TraceDir, env: &Envelope) -> io::Result<bool> {
        let bytes = match read_fixed(root, SEAL_FILE, SEAL_MAX_BYTES)? {
            Fixed::Absent => {
                self.short("seal_manifest_missing");
                return Ok(false);
            }
            Fixed::Bytes(bytes) if bytes.is_empty() => {
                self.short("seal_manifest_missing");
                return Ok(false);
            }
            Fixed::TooLarge => {
                self.fail("seal_manifest_too_large");
                return Ok(false);
            }
            Fixed::Bytes(bytes) => bytes,
        };
        let Ok(manifest) = serde_json::from_slice::<SealManifest>(&bytes) else {
            self.fail("seal_manifest_malformed");
            return Ok(false);
        };
        if check_terminal_manifest(env, &manifest).is_err() {
            self.fail("seal_manifest_mismatch");
            return Ok(false);
        }
        self.git_read(&manifest, "seal");
        // A clone's seal records its Git state, or why it could not: a walk instead (an older
        // daemon, or a removed `.git`) leaves the clone's work unproven (D56).
        if self.baseline_git && manifest.git.is_none() {
            self.short("seal_git_missing");
        }
        Ok(true)
    }

    fn bind_signed_finalization(&mut self, env: &Envelope) -> bool {
        if !matches!(env.body, EventBody::FinalizationV1(_)) {
            self.fail("chain_phase_invalid");
            return false;
        }
        let Some(receipt) = &self.receipt else {
            if !self.receipt_present {
                // A signed-mode terminal whose receipt is not retained cannot be proven.
                self.short("signed_terminal_receipt_missing");
            }
            return false;
        };
        let Some(cutoff) = &self.cutoff else {
            self.fail("chain_phase_invalid");
            return false;
        };
        let mut faults: Vec<Violation> = Vec::new();
        if cutoff.receipt_sha256 != receipt.digest {
            faults.push("cutoff_receipt_mismatch");
        }
        if cutoff.deadline_at != receipt.receipt.claims.deadline_at {
            faults.push("cutoff_deadline_mismatch");
        }
        let proven = receipt.bound && receipt.head_bound;
        for fault in &faults {
            self.fail(fault);
        }
        faults.is_empty() && proven
    }

    fn report(mut self) -> CaptureIntegrityReport {
        // Every entry carries a Launch naming a goal stream that ended Complete, the two in
        // either order. Without that the run is unproven, never forged. A pruned chain's
        // records are not judged here, and it is already incomplete.
        if !self.pruned && self.capture.launch_goals.is_empty() {
            self.short("launch_missing");
        } else if !self
            .capture
            .launch_goals
            .is_subset(&self.capture.goal_complete)
        {
            self.short("launch_goal_unbound");
        }
        let unclosed = self.capture.has_unclosed_producer();
        if self.capture.gaps > 0 {
            self.findings.add("gap_recorded");
        }
        if self.capture.recoveries > 0 {
            self.findings.add("producer_recovered");
        }
        // Informational: an epoch lost in flight. The recovery alone already makes
        // coverage incomplete, so this never changes the exit.
        if self.capture.unrecorded_recoveries > 0 {
            self.findings.add("recovery_names_unrecorded_producer");
        }
        if unclosed {
            self.findings.add("producer_unclosed");
        }
        let coverage_incomplete = self.capture.gaps > 0 || self.capture.recoveries > 0 || unclosed;
        let receipt_clock = self
            .receipt
            .as_ref()
            .filter(|r| r.bound && r.head_bound)
            .map(|r| VerifiedReceiptClock {
                started_at: r.receipt.claims.started_at.clone(),
                deadline_at: r.receipt.claims.deadline_at.clone(),
            });
        if let Some((_, KeySource::Supplied)) = self.anchor {
            self.findings.add("server_key_unpinned");
        }
        let integrity = if self.invalid {
            CaptureIntegrity::Invalid
        } else if self.incomplete || !self.sealed || coverage_incomplete {
            CaptureIntegrity::Incomplete
        } else {
            CaptureIntegrity::Valid
        };
        // The deadline counts mean something only against a receipt that verified and bound.
        let deadline_facts = receipt_clock.is_some();
        let last_ack_at = self.verified_ack.as_ref().map(|ack| ack.acked_at.clone());
        // An invalid report names its first violation, an incomplete one its first missing piece.
        let first_failure = match integrity {
            CaptureIntegrity::Invalid => self.first_invalid.take(),
            CaptureIntegrity::Incomplete => self.first_incomplete.take(),
            CaptureIntegrity::Valid => None,
        };
        CaptureIntegrityReport {
            schema_version: CAPTURE_INTEGRITY_SCHEMA_VERSION,
            integrity,
            anchor: if receipt_clock.is_some() {
                AnchorState::Signed
            } else {
                AnchorState::LocalOnly
            },
            sealed: self.sealed,
            validated_records: self.validated_records,
            validated_bytes: self.validated_bytes,
            validated_head: (self.validated_records > 0).then(|| self.head.clone()),
            gap_count: self.capture.gaps,
            coverage_incomplete,
            torn_tail: self.torn_tail,
            key_source: self.anchor.map_or(KeySource::None, |(_, source)| source),
            findings: self.findings.finish(),
            git_unreadable: self.git_unreadable,
            verified_ack: self.verified_ack,
            receipt_clock,
            terminal: self.terminal_facts,
            records_after_deadline: deadline_facts.then_some(self.records_after_deadline),
            last_seq_at_or_before_deadline: self
                .last_seq_at_or_before_deadline
                .filter(|_| deadline_facts),
            last_ack_at,
            entry_id: self.validated_entry,
            profile_id: self.profile_id,
            first_failure,
        }
    }
}

/// The verified facts of a validated, bound terminal envelope.
fn terminal_facts(env: &Envelope) -> Option<CaptureTerminalFacts> {
    let (observed, observed_at, kind, reason, termination, seal_manifest_sha256) = match &env.body {
        EventBody::Capture {
            record:
                CaptureRecord::LocalFinish {
                    observed,
                    reason,
                    termination,
                    seal_manifest_sha256,
                },
        } => (
            Some(observed.clone()),
            observed.wall.clone(),
            CaptureTerminalKind::LocalFinish,
            Some(reason.clone()),
            Some(termination.clone()),
            Some(seal_manifest_sha256.clone()),
        ),
        EventBody::FinalizationV1(finalization) => {
            let termination = finalization.termination().clone();
            (
                None,
                env.recorded_at.clone(),
                CaptureTerminalKind::SignedFinalization,
                None,
                Some(termination),
                None,
            )
        }
        _ => return None,
    };
    Some(CaptureTerminalFacts {
        seq: env.link.seq,
        hash: env.link.hash.clone(),
        observed,
        observed_at,
        kind,
        reason,
        termination,
        seal_manifest_sha256,
    })
}

/// `acks.jsonl`, one bounded line at a time, checked for shape and monotonicity as it is
/// read. Matching each ACK to its chain link is the scan's.
struct AckStream {
    reader: BufReader<std::fs::File>,
    line: Vec<u8>,
    pending: Option<AckCheckpoint>,
    previous: Option<(u64, u64)>,
    finished: bool,
}

impl AckStream {
    fn new(file: std::fs::File) -> Self {
        Self {
            reader: BufReader::new(file),
            line: Vec::with_capacity(ACK_LINE_MAX_BYTES),
            pending: None,
            previous: None,
            finished: false,
        }
    }

    /// Load the next acknowledgement unless one is pending or the stream ended. The outer
    /// `Err` is I/O; the inner one is an evidence fault.
    fn fill(&mut self) -> io::Result<Result<(), Violation>> {
        if self.pending.is_some() || self.finished {
            return Ok(Ok(()));
        }
        self.line.clear();
        let read = (&mut self.reader)
            .take(ACK_LINE_MAX_BYTES as u64)
            .read_until(b'\n', &mut self.line)?;
        if read == 0 {
            self.finished = true;
            return Ok(Ok(()));
        }
        if self.line.last() != Some(&b'\n') {
            return Ok(Err(if read == ACK_LINE_MAX_BYTES {
                "ack_too_large"
            } else {
                "ack_torn_tail"
            }));
        }
        self.line.pop();
        let Ok(ack) = serde_json::from_slice::<AckCheckpoint>(&self.line) else {
            return Ok(Err("ack_malformed"));
        };
        let Some(acked_at) = Rfc3339Utc::try_from(ack.acked_at.as_str())
            .ok()
            .and(ack.acked_at_secs())
        else {
            return Ok(Err("ack_time_invalid"));
        };
        if let Some((seq, at)) = self.previous
            && (ack.seq < seq || acked_at < at)
        {
            return Ok(Err("ack_not_monotonic"));
        }
        self.previous = Some((ack.seq, acked_at));
        self.pending = Some(ack);
        Ok(Ok(()))
    }
}

/// A bounded display string: at most 1 KiB and no control bytes.
fn bounded_text(value: &str) -> bool {
    value.len() <= MAX_METADATA_STRING_BYTES && !value.bytes().any(|b| b.is_ascii_control())
}

fn check_time(time: &CaptureTime) -> Result<(), Violation> {
    time.validate().map_err(|_| "record_field_invalid")
}

/// Reuse the protocol's frame validators for a record's own fields (identifier formats, the
/// stream-zero rule, ALPN, metadata bounds and source/wire consistency) instead of
/// restating them. The sequence number is not retained in a record and is not checked.
fn check_frame(
    stream: &StreamKey,
    observed: &CaptureTime,
    event: FrameEvent,
) -> Result<(), Violation> {
    CaptureFrame {
        stream: stream.clone(),
        seq: 0,
        observed: observed.clone(),
        event,
    }
    .validate()
    .map_err(|_| "record_field_invalid")
}

/// What the reader keeps about one producer epoch: a high-water stream ID and a few flags.
#[derive(Debug, Default)]
struct ProducerState {
    /// The highest stream ID this producer opened. IDs increase and are never reused.
    high_stream: u64,
    /// A ProducerClosed record closed this epoch cleanly.
    closed: bool,
    /// A Recovery superseded this epoch: explicit loss.
    lost: bool,
    /// This epoch began with a Recovery record.
    recovered: bool,
    /// A wire producer: its first record is a Recovery, or it recorded a
    /// Connection. Native and goal producers never are.
    wire: bool,
    /// Records that each consumed exactly one of this producer's frames (Recovery,
    /// Connection, Open, End). Bytes frames can merge into segments and are not counted.
    /// This gives a lower bound only, never a reconstruction of ingress sequences.
    frames: u64,
}

#[derive(Debug, Default)]
struct StreamState {
    next_part: u64,
    next_offset: u64,
    /// A gap was recorded on this stream since its last segment, so the next segment may
    /// start past the covered source range.
    gap_pending: bool,
    /// It opened with `source: Goal`.
    goal: bool,
}

/// The capture-record semantics this reader owns, kept to bounded live state.
#[derive(Debug, Default)]
struct CaptureState {
    lineage: Lineage,
    producers: BTreeMap<String, ProducerState>,
    streams: BTreeMap<StreamKey, StreamState>,
    /// Active connection identity to the producer currently observing it.
    connections: BTreeMap<String, String>,
    gaps: u64,
    recoveries: u64,
    /// Recoveries whose previous producer had no earlier record.
    unrecorded_recoveries: u64,
    /// The goal stream each Launch names.
    launch_goals: BTreeSet<StreamKey>,
    /// Goal streams that ended Complete.
    goal_complete: BTreeSet<StreamKey>,
    /// Reused decode buffer for one segment.
    decoded: Vec<u8>,
}

/// Remember `key`, admitting a new one only under the per-entry limit.
fn remember(
    set: &mut BTreeSet<StreamKey>,
    key: &StreamKey,
    limit: Violation,
) -> Result<(), Violation> {
    if !set.contains(key) {
        if set.len() >= MAX_GOAL_STREAMS {
            return Err(limit);
        }
        set.insert(key.clone());
    }
    Ok(())
}

/// A producer's state, admitting a new producer under the per-entry limit.
fn admit<'a>(
    producers: &'a mut BTreeMap<String, ProducerState>,
    lineage: &mut Lineage,
    id: &str,
) -> Result<&'a mut ProducerState, Violation> {
    if !producers.contains_key(id) {
        if !is_capture_id(id) {
            return Err("record_field_invalid");
        }
        if !lineage.note(id, false) {
            return Err("producer_limit_exceeded");
        }
        producers.insert(id.to_string(), ProducerState::default());
    }
    Ok(producers.get_mut(id).expect("admitted above"))
}

/// The state of the producer epoch a record claims to come from: admitted, and neither
/// closed nor superseded.
fn owner<'a>(
    producers: &'a mut BTreeMap<String, ProducerState>,
    lineage: &mut Lineage,
    id: &str,
) -> Result<&'a mut ProducerState, Violation> {
    let state = admit(producers, lineage, id)?;
    if state.closed {
        return Err("record_after_producer_closed");
    }
    if state.lost {
        return Err("record_from_lost_producer");
    }
    Ok(state)
}

impl CaptureState {
    fn has_unclosed_producer(&self) -> bool {
        self.producers.values().any(|p| !p.closed && !p.lost)
    }

    /// The capture-record rules, one method per record kind.
    fn apply(&mut self, record: &CaptureRecord) -> Result<(), Violation> {
        match record {
            CaptureRecord::ProducerClosed {
                producer,
                observed,
                final_seq,
            } => self.producer_closed(producer, observed, *final_seq),
            CaptureRecord::Connection {
                producer,
                connection,
                observed,
                event,
            } => self.connection(producer, connection, observed, event),
            CaptureRecord::Recovery {
                previous_producer,
                producer,
                observed,
                reason,
            } => self.recovery(previous_producer, producer, observed, *reason),
            CaptureRecord::Open {
                stream,
                observed,
                source,
                metadata,
            } => self.open(stream, observed, *source, metadata),
            CaptureRecord::Segment {
                stream,
                part,
                source_offset,
                source_len,
                first,
                last,
                data_b64,
                audit,
            } => {
                let view = SegmentView {
                    part: *part,
                    first,
                    last,
                    audit: &audit.spans,
                    source_offset: *source_offset,
                    source_len: *source_len,
                };
                self.segment(stream, &view, data_b64)
            }
            CaptureRecord::End {
                stream,
                observed,
                reason,
            } => self.end(stream, observed, *reason),
            CaptureRecord::Gap {
                stream,
                observed,
                reason,
                lost_bytes,
            } => self.gap(stream.as_ref(), observed, *reason, *lost_bytes),
            CaptureRecord::Launch { goal_stream, .. } => {
                check_launch(record)?;
                remember(&mut self.launch_goals, goal_stream, "launch_limit_exceeded")
            }
            CaptureRecord::Assistance { observed, action } => {
                check_time(observed)?;
                if action.is_empty() || !bounded_text(action) {
                    return Err("record_field_invalid");
                }
                Ok(())
            }
            // Its fields, position and envelope time are the phase replay's to judge.
            CaptureRecord::LocalFinish { .. } => Ok(()),
        }
    }

    fn producer_closed(
        &mut self,
        producer: &str,
        observed: &CaptureTime,
        final_seq: Option<u64>,
    ) -> Result<(), Violation> {
        check_time(observed)?;
        if self.producers.get(producer).is_some_and(|p| p.closed) {
            return Err("producer_closed_twice");
        }
        if self.streams.keys().any(|k| k.producer == producer) {
            return Err("producer_closed_with_open_streams");
        }
        let state = owner(&mut self.producers, &mut self.lineage, producer)?;
        // A receiver attestation: `final_seq` is the last accepted frame sequence,
        // None only for zero frames. The persisted records give a lower bound.
        let consistent = match final_seq {
            None => state.frames == 0,
            Some(seq) => seq.checked_add(1).is_none_or(|count| count >= state.frames),
        };
        if !consistent {
            return Err("producer_closed_final_seq_inconsistent");
        }
        state.closed = true;
        self.connections.retain(|_, owner| owner != producer);
        Ok(())
    }

    fn connection(
        &mut self,
        producer: &str,
        connection: &str,
        observed: &CaptureTime,
        event: &ConnectionEvent,
    ) -> Result<(), Violation> {
        check_frame(
            &StreamKey {
                producer: producer.to_string(),
                stream: 0,
            },
            observed,
            FrameEvent::Connection {
                connection: connection.to_string(),
                event: event.clone(),
            },
        )?;
        let state = owner(&mut self.producers, &mut self.lineage, producer)?;
        state.frames = state.frames.saturating_add(1);
        state.wire = true;
        self.lineage.note(producer, true);
        let recovered = state.recovered;
        match event {
            ConnectionEvent::Open { already_open, .. } => {
                // A current observation of a surviving socket, allowed only in an
                // epoch that began with a Recovery. It is not reconstructed history.
                if *already_open && !recovered {
                    return Err("connection_reobserved_outside_recovery");
                }
                if self.connections.contains_key(connection) {
                    return Err("connection_already_open");
                }
                if self.connections.len() >= MAX_OPEN_CONNECTIONS {
                    return Err("connection_limit_exceeded");
                }
                self.connections
                    .insert(connection.to_string(), producer.to_string());
            }
            ConnectionEvent::UpgradedWebSocket => {
                if self.connections.get(connection).map(String::as_str) != Some(producer) {
                    return Err("connection_not_open");
                }
            }
            ConnectionEvent::Close { .. } => {
                if self.connections.get(connection).map(String::as_str) != Some(producer) {
                    return Err("connection_not_open");
                }
                self.connections.remove(connection);
            }
        }
        Ok(())
    }

    fn recovery(
        &mut self,
        previous_producer: &str,
        producer: &str,
        observed: &CaptureTime,
        reason: GapReason,
    ) -> Result<(), Violation> {
        check_frame(
            &StreamKey {
                producer: producer.to_string(),
                stream: 0,
            },
            observed,
            FrameEvent::Recovery {
                previous_producer: previous_producer.to_string(),
                reason,
            },
        )?;
        // The fresh epoch's first frame is its Recovery.
        if self.producers.contains_key(producer) {
            return Err("recovery_producer_not_fresh");
        }
        // A previous producer with no earlier record was lost in flight
        // (`Queued` is not persisted). It is admitted here as a lost zero-record
        // producer and counts toward MAX_PRODUCERS.
        if let Some(previous) = self.producers.get(previous_producer) {
            // Only a wire producer is ever recovered; a native or goal producer is not.
            if !previous.wire {
                return Err("recovery_names_non_wire_producer");
            }
            if previous.closed {
                return Err("recovery_after_producer_closed");
            }
            if previous.lost {
                return Err("producer_recovered_twice");
            }
        }
        let supersession = self.lineage.supersedes(previous_producer, producer);
        if !supersession.admitted {
            if supersession.predecessor_fits {
                // Preserve the reader's sequential admission behavior: an existing
                // predecessor, or an unseen predecessor that consumes the last slot,
                // is marked lost before admitting the fresh epoch fails. At a completely
                // full table an unseen predecessor does not fit, so nothing changes.
                if supersession.unrecorded {
                    self.producers
                        .insert(previous_producer.to_string(), ProducerState::default());
                }
                for id in &supersession.producers {
                    if let Some(state) = self.producers.get_mut(id)
                        && !state.closed
                    {
                        state.lost = true;
                    }
                }
                if supersession.unrecorded {
                    self.unrecorded_recoveries = self.unrecorded_recoveries.saturating_add(1);
                }
            }
            return Err("producer_limit_exceeded");
        }
        admit(&mut self.producers, &mut self.lineage, previous_producer)?;
        for id in &supersession.producers {
            if let Some(state) = self.producers.get_mut(id)
                && !state.closed
            {
                state.lost = true;
            }
        }
        if supersession.unrecorded {
            self.unrecorded_recoveries = self.unrecorded_recoveries.saturating_add(1);
        }
        let fresh = admit(&mut self.producers, &mut self.lineage, producer)?;
        fresh.recovered = true;
        fresh.wire = true;
        fresh.frames = 1;
        // Producer loss invalidates the superseded epochs' stream keys and
        // connection contexts; surviving sockets are re-observed by the fresh
        // epoch, never transferred. Only this method sets `lost`, and a lost producer
        // can hold nothing afterwards, so only those superseded here still do.
        self.streams
            .retain(|k, _| !supersession.producers.contains(&k.producer));
        self.connections
            .retain(|_, owner| !supersession.producers.contains(owner));
        self.recoveries = self.recoveries.saturating_add(1);
        Ok(())
    }

    fn open(
        &mut self,
        stream: &StreamKey,
        observed: &CaptureTime,
        source: CaptureSource,
        metadata: &CaptureMeta,
    ) -> Result<(), Violation> {
        check_frame(
            stream,
            observed,
            FrameEvent::Open {
                source,
                metadata: metadata.clone(),
            },
        )?;
        let state = owner(&mut self.producers, &mut self.lineage, &stream.producer)?;
        if stream.stream <= state.high_stream {
            return Err("stream_id_not_increasing");
        }
        if self.streams.len() >= MAX_OPEN_STREAMS {
            return Err("open_stream_limit_exceeded");
        }
        state.high_stream = stream.stream;
        state.frames = state.frames.saturating_add(1);
        self.streams.insert(
            stream.clone(),
            StreamState {
                goal: source == CaptureSource::Goal,
                ..StreamState::default()
            },
        );
        Ok(())
    }

    fn segment(
        &mut self,
        stream: &StreamKey,
        view: &SegmentView<'_>,
        data_b64: &str,
    ) -> Result<(), Violation> {
        let (first, last) = (view.first, view.last);
        check_time(first)?;
        check_time(last)?;
        if first.clock_id == last.clock_id && first.mono_ns > last.mono_ns {
            return Err("segment_time_reversed");
        }
        let state = self.streams.get_mut(stream).ok_or("stream_not_open")?;
        if view.part != state.next_part {
            return Err("segment_part_out_of_order");
        }
        let end = view
            .source_offset
            .checked_add(view.source_len)
            .ok_or("segment_range_invalid")?;
        if view.source_offset < state.next_offset {
            return Err("segment_source_overlap");
        }
        if view.source_offset > state.next_offset && !state.gap_pending {
            return Err("segment_source_unexplained_hole");
        }
        if data_b64.len() > MAX_SEGMENT_B64_BYTES {
            return Err("segment_data_too_large");
        }
        self.decoded.clear();
        STANDARD
            .decode_vec(data_b64.as_bytes(), &mut self.decoded)
            .map_err(|_| "segment_data_not_base64")?;
        if self.decoded.len() > MAX_SEGMENT_BYTES {
            return Err("segment_data_too_large");
        }
        // Audit spans are absolute source offsets inside this segment's own range.
        for span in view.audit {
            if span.start > span.end
                || span.start < view.source_offset
                || span.end > end
                || span.kind.is_empty()
                || !bounded_text(&span.kind)
            {
                return Err("segment_audit_out_of_range");
            }
        }
        // Unredacted sanitized bytes are the source bytes they claim to cover.
        if view.audit.is_empty() && self.decoded.len() as u64 != view.source_len {
            return Err("segment_length_mismatch");
        }
        state.next_part = view.part.checked_add(1).ok_or("segment_range_invalid")?;
        state.next_offset = end;
        state.gap_pending = false;
        Ok(())
    }

    fn end(
        &mut self,
        stream: &StreamKey,
        observed: &CaptureTime,
        reason: EndReason,
    ) -> Result<(), Violation> {
        check_frame(stream, observed, FrameEvent::End { reason })?;
        let state = self.streams.remove(stream).ok_or("stream_not_open")?;
        if state.goal && reason == EndReason::Complete {
            remember(
                &mut self.goal_complete,
                stream,
                "goal_stream_limit_exceeded",
            )?;
        }
        if let Some(state) = self.producers.get_mut(&stream.producer) {
            state.frames = state.frames.saturating_add(1);
        }
        Ok(())
    }

    fn gap(
        &mut self,
        stream: Option<&StreamKey>,
        observed: &CaptureTime,
        reason: GapReason,
        lost_bytes: Option<u64>,
    ) -> Result<(), Violation> {
        match stream {
            None => check_time(observed)?,
            Some(key) => {
                check_frame(key, observed, FrameEvent::Gap { reason, lost_bytes })?;
                // A loss marker may name a stream of a lost or closed epoch, but
                // only one that was actually opened.
                let opened = self
                    .producers
                    .get(&key.producer)
                    .is_some_and(|p| key.stream <= p.high_stream);
                if !opened {
                    return Err("gap_on_unknown_stream");
                }
                if let Some(state) = self.streams.get_mut(key) {
                    state.gap_pending = true;
                }
            }
        }
        self.gaps = self.gaps.saturating_add(1);
        Ok(())
    }
}

/// The field rules of a `Launch` record. Its goal stream is remembered by the caller.
fn check_launch(record: &CaptureRecord) -> Result<(), Violation> {
    let CaptureRecord::Launch {
        observed,
        goal_stream,
        profile_id,
        profile_digest,
        harness_version,
        recorder_revision,
        continuation,
        allowed_tool_rule,
        process_handoff_digest,
        run_public_key,
        connector_dir_sha256,
        harness_build,
    } = record
    else {
        return Ok(());
    };
    check_time(observed)?;
    if !is_capture_id(&goal_stream.producer) || goal_stream.stream == 0 {
        return Err("record_field_invalid");
    }
    if ![
        profile_id,
        profile_digest,
        harness_version,
        recorder_revision,
        continuation,
    ]
    .iter()
    .all(|value| bounded_text(value))
    {
        return Err("record_field_invalid");
    }
    // The tool allowance is held to shape only, for every profile: both present,
    // bounded text and SHA-256 hex, or both absent.
    match (allowed_tool_rule, process_handoff_digest) {
        (None, None) => {}
        (Some(rule), Some(digest))
            if bounded_text(rule) && Sha256Digest::try_from(digest.as_str()).is_ok() => {}
        _ => return Err("record_field_invalid"),
    }
    // Contract S1: both optional fields are valid absent forever. A present
    // key is canonical lowercase hex of a usable Ed25519 point (agreement with a
    // receipt's key is checked where the receipt is in hand, in `read_chain`); a
    // present directory digest is canonical lowercase SHA-256 hex and is never
    // compared against a pin here.
    if let Some(key) = run_public_key {
        let canonical = goalcup_protocol::provenance::canonical_run_public_key(key);
        if canonical.as_deref() != Ok(key.as_str()) {
            return Err("record_field_invalid");
        }
    }
    if let Some(digest) = connector_dir_sha256
        && Sha256Digest::try_from(digest.as_str()).is_err()
    {
        return Err("record_field_invalid");
    }
    // D53: the observed build is recorded, never judged here. Each present digest is
    // canonical SHA-256 hex, and the platform is bounded text.
    if let Some(build) = harness_build {
        let digests_ok = [&build.program_sha256, &build.image_sha256]
            .into_iter()
            .flatten()
            .all(|digest| Sha256Digest::try_from(digest.as_str()).is_ok());
        if !digests_ok || build.platform.is_empty() || !bounded_text(&build.platform) {
            return Err("record_field_invalid");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn findings_keep_distinct_codes_and_count_what_they_omit() {
        let mut findings = Findings::default();
        findings.add("a");
        findings.add("a");
        for n in 0..MAX_FINDING_CODES + 5 {
            findings.add(&format!("code_{n}"));
        }
        let codes = findings.finish();
        assert_eq!(codes.len(), MAX_FINDING_CODES + 1);
        assert_eq!(codes[0], "a");
        assert_eq!(codes.last().unwrap(), "findings_omitted:6");
    }

    /// The encoded bound is exactly a full segment's padded base64. Padding lets one or two
    /// extra bytes encode to the same length, so the decoded length is bounded as well.
    #[test]
    fn a_segment_is_bounded_before_and_after_decoding() {
        use goalcup_protocol::capture::{CaptureMeta, CaptureSource};
        use goalcup_protocol::redaction::RedactionAudit;

        assert_eq!(
            STANDARD.encode(vec![0u8; MAX_SEGMENT_BYTES]).len(),
            MAX_SEGMENT_B64_BYTES
        );
        let stream = StreamKey {
            producer: "a1b2c3d4e5f60718293a4b5c6d7e8f90".to_string(),
            stream: 1,
        };
        let at = CaptureTime {
            wall: "2026-09-27T12:00:01Z".to_string(),
            mono_ns: 1,
            clock_id: stream.producer.clone(),
        };
        let mut state = CaptureState::default();
        state
            .apply(&CaptureRecord::Open {
                stream: stream.clone(),
                observed: at.clone(),
                source: CaptureSource::Native,
                metadata: CaptureMeta {
                    wire: None,
                    label: "rollout.jsonl#1".to_string(),
                    protocol_hint: None,
                    method: None,
                    host: None,
                    path: None,
                    status: None,
                    content_type: None,
                },
            })
            .unwrap();
        let segment = |bytes: usize| CaptureRecord::Segment {
            stream: stream.clone(),
            part: 0,
            source_offset: 0,
            source_len: bytes as u64,
            first: at.clone(),
            last: at.clone(),
            data_b64: STANDARD.encode(vec![b'x'; bytes]),
            audit: RedactionAudit { spans: vec![] },
        };
        // Same encoded length as a full segment: refused after decoding.
        assert_eq!(
            STANDARD.encode(vec![0u8; MAX_SEGMENT_BYTES + 1]).len(),
            MAX_SEGMENT_B64_BYTES
        );
        assert_eq!(
            state.apply(&segment(MAX_SEGMENT_BYTES + 1)),
            Err("segment_data_too_large")
        );
        // Longer encoded text: refused before any decoding.
        assert_eq!(
            state.apply(&segment(MAX_SEGMENT_BYTES + 3)),
            Err("segment_data_too_large")
        );
        assert_eq!(state.apply(&segment(MAX_SEGMENT_BYTES)), Ok(()));
    }

    /// The distinct-producer bound at its edges, reached through a smaller lineage limit: a
    /// trace of `MAX_PRODUCERS` producers takes seconds to build (D58). Each sequence's last
    /// record is the one judged.
    #[test]
    fn producer_bound_edges_keep_the_sequential_rule() {
        use goalcup_protocol::capture::GapReason;

        const LIMIT: usize = 4;
        let id = |n: usize| format!("{n:032x}");
        let (live, in_flight, fresh) = (id(800), id(801), id(802));
        let at = |clock: &str| CaptureTime {
            wall: "2026-09-27T12:00:01Z".to_string(),
            mono_ns: 1,
            clock_id: clock.to_string(),
        };
        let closed = |count: usize| -> Vec<CaptureRecord> {
            (1..=count)
                .map(|n| CaptureRecord::ProducerClosed {
                    producer: id(n),
                    observed: at("daemon"),
                    final_seq: None,
                })
                .collect()
        };
        let wire_open = CaptureRecord::Connection {
            producer: live.clone(),
            connection: id(900),
            observed: at(&live),
            event: ConnectionEvent::Open {
                alpn: None,
                already_open: false,
            },
        };
        let recovery = |previous: &str| CaptureRecord::Recovery {
            previous_producer: previous.to_string(),
            producer: fresh.clone(),
            observed: at(&fresh),
            reason: GapReason::IngestLost,
        };
        let run = |records: Vec<CaptureRecord>, last: Result<(), Violation>| {
            let mut state = CaptureState {
                lineage: Lineage::with_limit(LIMIT),
                ..CaptureState::default()
            };
            let (judged, before) = records.split_last().unwrap();
            for record in before {
                state.apply(record).unwrap();
            }
            assert_eq!(state.apply(judged), last);
            state
        };
        let with = |mut records: Vec<CaptureRecord>, more: &[&CaptureRecord]| {
            records.extend(more.iter().map(|r| (*r).clone()));
            records
        };

        // One more distinct producer than the limit, completed epochs included.
        run(closed(LIMIT), Ok(()));
        run(closed(LIMIT + 1), Err("producer_limit_exceeded"));

        // An unseen predecessor and the fresh producer fit exactly in the last two slots.
        let state = run(with(closed(LIMIT - 2), &[&recovery(&in_flight)]), Ok(()));
        assert_eq!(state.unrecorded_recoveries, 1);

        // One slot left: the unrecorded predecessor takes it and the fresh producer does not
        // fit, but the recovery still names an unrecorded producer.
        let state = run(
            with(closed(LIMIT - 1), &[&recovery(&in_flight)]),
            Err("producer_limit_exceeded"),
        );
        assert_eq!(state.unrecorded_recoveries, 1);

        // With a live wire epoch among the admitted, the failed recovery supersedes it first,
        // so finalization adds no `producer_unclosed` to the limit error.
        let state = run(
            with(closed(LIMIT - 2), &[&wire_open, &recovery(&in_flight)]),
            Err("producer_limit_exceeded"),
        );
        assert_eq!(state.unrecorded_recoveries, 1);
        assert!(!state.has_unclosed_producer());

        // A full table admits nothing, so no unrecorded predecessor is noted.
        let state = run(
            with(closed(LIMIT), &[&recovery(&in_flight)]),
            Err("producer_limit_exceeded"),
        );
        assert_eq!(state.unrecorded_recoveries, 0);

        // A full table with a recorded live predecessor: it is marked lost before the fresh
        // admission fails.
        let state = run(
            with(closed(LIMIT - 1), &[&wire_open, &recovery(&live)]),
            Err("producer_limit_exceeded"),
        );
        assert_eq!(state.unrecorded_recoveries, 0);
        assert!(!state.has_unclosed_producer());
    }
}
