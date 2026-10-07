//! The daemon's one writer of a trace directory.
//!
//! Every file a verifier reads is written here, and every one of them is written by
//! [`goalcup_protocol::export::TraceExport`]. This module defines **no export shape of its
//! own**. What a link covers is answered once, in [`Envelope::chain_bytes`], and the stamping
//! lives in `TraceExport::append_chain`, so **the daemon computes no hash** and the two
//! sides cannot drift.
//!
//! **One chain per entry, one writer per chain.** `ChainWriter` owns the entry's
//! `TraceExport` and nothing else in this crate holds one, so the sequence counter has
//! exactly one owner.
//!
//! **The order is fixed**: partition, then redact, then hash, then buffer.
//! [`ChainWriter::leg1`] redacts before it builds the envelope, and the envelope is what the
//! export hashes — a [`CapturedRecord`]'s `bytes` reach the redactor and stop there. The link
//! is computed and the line is on disk before the caller gets the envelope back to buffer, so
//! a restart resumes from a durable head.
//!
//! **The seal and the baseline are not events.** `EventBody::Seal` and
//! `EventBody::Baseline` are built by `TraceExport::write_seal` and
//! `TraceExport::write_baseline` and by nothing else:
//! `append_chain` refuses both bodies with `ExportError::ReservedBody`, and
//! [`ChainWriter::event`] refuses them earlier and in this crate's vocabulary. The
//! once-only and position rules stay properties of the export rather than conventions
//! every call site has to remember. This module re-implements neither — it delegates and
//! surfaces what comes back as a [`DaemonError`].
//!
//! **One altered record, one append.** Its redacted body and inline audit share one
//! envelope and one hash. An append failure therefore leaves neither half visible.

use crate::redact::{Redacted, Redactor};
use crate::tailer::CapturedRecord;
use crate::{DaemonError, now_rfc3339};
use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::chain::Link;
use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::export::{
    FixedTraceDocument, MANIFEST_FILE, PreparedTraceResume, TraceDescriptorDoc, TraceExport,
    TraceRoot,
};
use goalcup_protocol::manifest::ConnectorManifest;
use goalcup_protocol::provenance::{CutoffReachedV1, FinalizationV1};
use goalcup_protocol::receipt::StartReceipt;
use goalcup_protocol::redaction::RedactionAudit;
use goalcup_protocol::seal::SealManifest;
use std::path::{Path, PathBuf};

/// The `source` of everything the daemon observed about itself or the filesystem.
const DAEMON: &str = "daemon";

/// One entry's writer. Owns the entry's `TraceExport`; nothing else in this crate holds
/// one, so the sequence counter has exactly one owner.
#[derive(Debug)]
pub struct ChainWriter {
    entry_id: String,
    export: TraceExport,
    /// The run's redactor.
    redactor: Redactor,
    /// Test-only: the number of appends that may reach the export before one fails, and
    /// then every append after it. A **writer** failure — `DaemonError::Io` — which is
    /// what [`crate::runtime::EntryRuntime`]'s post drain stops on.
    ///
    /// Gated on `cfg(test)` or the off-by-default `test-hooks` feature, so no shipped
    /// build has a way to make an append fail.
    #[cfg(any(test, feature = "test-hooks"))]
    appends_before_failure: Option<u32>,
    /// Test-only: how many appends may land before one is refused *before it writes
    /// anything*, after which the writer is fit again. A **per-post** failure —
    /// `DaemonError::Refused` — and therefore the one the drain spends on a single record
    /// and carries on past. Same gate, same reason.
    #[cfg(any(test, feature = "test-hooks"))]
    refuse_append_at: Option<u32>,
    /// Test-only: fail the next acknowledgement before it reaches the export. Real
    /// failures are a full disk or a broken already-open stream.
    #[cfg(any(test, feature = "test-hooks"))]
    fail_next_ack: bool,
}

impl ChainWriter {
    /// `TraceExport::create`, which refuses a directory that already holds a non-empty
    /// `chain.jsonl` (`ExportError::ChainExists`, surfaced as `DaemonError::Export`):
    /// creating over an existing chain would write a second run's link 0 onto the end of
    /// the first run's links, one file that reads as one chain and is two.
    ///
    /// Fresh versus rebuilt intent is fixed by `EntryRuntime` construction; `open_trace`
    /// consumes that explicit state rather than inferring it from a path.
    pub fn create(
        entry_id: impl Into<String>,
        trace_dir: &Path,
        redactor: Redactor,
    ) -> Result<ChainWriter, DaemonError> {
        Ok(ChainWriter {
            entry_id: entry_id.into(),
            export: TraceExport::create(trace_dir)?,
            redactor,
            #[cfg(any(test, feature = "test-hooks"))]
            appends_before_failure: None,
            #[cfg(any(test, feature = "test-hooks"))]
            refuse_append_at: None,
            #[cfg(any(test, feature = "test-hooks"))]
            fail_next_ack: false,
        })
    }

    /// Fresh construction over a final trace root acquired by the entry's retained artifact
    /// capability. The supplied root's display path is diagnostic; its directory handle owns
    /// every export operation.
    pub fn create_from_root(
        entry_id: impl Into<String>,
        root: TraceRoot,
        redactor: Redactor,
    ) -> Result<ChainWriter, DaemonError> {
        Ok(ChainWriter {
            entry_id: entry_id.into(),
            export: TraceExport::create_from_root(root)?,
            redactor,
            #[cfg(any(test, feature = "test-hooks"))]
            appends_before_failure: None,
            #[cfg(any(test, feature = "test-hooks"))]
            refuse_append_at: None,
            #[cfg(any(test, feature = "test-hooks"))]
            fail_next_ack: false,
        })
    }

    /// Consume the already validated and caller-inspected resume capability. No path or
    /// independently supplied chain position can replace the root and append handle held
    /// by `prepared`.
    pub fn resume_prepared(
        entry_id: impl Into<String>,
        prepared: PreparedTraceResume,
        redactor: Redactor,
    ) -> Result<ChainWriter, DaemonError> {
        Ok(ChainWriter {
            entry_id: entry_id.into(),
            export: TraceExport::resume_prepared(prepared)?,
            redactor,
            #[cfg(any(test, feature = "test-hooks"))]
            appends_before_failure: None,
            #[cfg(any(test, feature = "test-hooks"))]
            refuse_append_at: None,
            #[cfg(any(test, feature = "test-hooks"))]
            fail_next_ack: false,
        })
    }

    /// `TraceExport::head()`, passed through: `None` before the first append, which is
    /// why `Entry.chain_seq` is an `Option` and not a `0`. "No links" and "the head is
    /// link 0" are two states and this type never collapses them.
    pub fn head(&self) -> Option<(u64, String)> {
        self.export.head()
    }

    /// The seq of the last link written, or `None` when nothing is chained yet.
    pub fn seq_written(&self) -> Option<u64> {
        self.head().map(|(seq, _)| seq)
    }

    /// The trace directory every file this writer produces sits in.
    pub fn root(&self) -> &Path {
        self.export.root()
    }

    /// The opaque identity of the trace root this writer has held since construction.
    /// A recovery intent records it before a terminal append, so a later pathname reopen
    /// cannot substitute a different directory carrying a plausible chain.
    pub fn root_identity(&self) -> Result<String, DaemonError> {
        Ok(self.export.root_identity()?)
    }

    /// Open a read-only handle matching this writer's retained chain identity. The
    /// concrete capability remains opaque outside protocol/platform; an unsafe acquisition
    /// poisons this writer and every later write or reader acquisition refuses.
    pub fn independent_chain_reader(
        &mut self,
    ) -> Result<impl std::io::Read + std::io::Seek + Send + 'static, DaemonError> {
        Ok(self.export.independent_chain_reader()?)
    }

    /// One leg-1 record: redacted and chained with an inline audit when something changed.
    ///
    /// `record.bytes` are the redactor's input and reach no hash: the link is taken over
    /// the envelope carrying the **redacted** payload.
    pub fn leg1(&mut self, record: &CapturedRecord) -> Result<Vec<Envelope>, DaemonError> {
        let out = self.redactor.apply(&record.bytes);
        let payload = leg1_payload(&record.payload, &out)?;
        let body = EventBody::Leg1Record {
            store: record.store.clone(),
            payload,
        };
        self.record(&record.store, body, out)
    }

    /// Everything else: notices, deviations, foreign sightings, heartbeats, restarts and
    /// chained health. **Not the seal, baseline, or legacy redaction log**: the first two
    /// bodies are [`ChainWriter::write_seal`]'s and [`ChainWriter::write_baseline`]'s to
    /// build, while new redaction evidence belongs inline on a leg envelope. A
    /// body of any of those kinds is `DaemonError::Refused` before it reaches the export.
    /// The export reserves seal and baseline too (`ExportError::ReservedBody`), so this is
    /// the near check rather than the only one: it fails one frame earlier and says so in
    /// this crate's vocabulary, and a chain with two seals — which is what the far check
    /// prevents for every other holder of a `TraceExport` — gives an acknowledgement two
    /// heads to anchor.
    ///
    /// Called after a seal it returns `DaemonError::Export(ExportError::Sealed { .. })`.
    pub fn event(&mut self, body: EventBody) -> Result<Envelope, DaemonError> {
        if matches!(
            body,
            EventBody::Seal { .. } | EventBody::Baseline { .. } | EventBody::Redaction(_)
        ) {
            return Err(DaemonError::Refused(
                "a seal and a baseline are chained by their dedicated writers, and new \
                 redaction evidence is chained inline on the altered leg envelope; none is \
                 an event a caller may hand over"
                    .to_string(),
            ));
        }
        self.append(DAEMON, &now_rfc3339(), body, None)
    }

    pub fn write_descriptor(&self, doc: &TraceDescriptorDoc) -> Result<PathBuf, DaemonError> {
        Ok(self.export.write_descriptor(doc)?)
    }

    pub fn write_receipt(&self, r: &StartReceipt) -> Result<PathBuf, DaemonError> {
        Ok(self.export.write_receipt(r)?)
    }

    /// Read one fixed trace document through the writer's retained root capability.
    pub fn read_fixed(
        &self,
        document: FixedTraceDocument,
    ) -> Result<Option<impl std::io::Read + std::io::Seek + Send + 'static>, DaemonError> {
        Ok(self.export.read_fixed(document)?)
    }

    /// Delegates to `TraceExport::write_baseline`, which chains `EventBody::Baseline` as
    /// seq 0 and only then writes `baseline.json` (the receipt's `chain_head` is the
    /// genesis link, so a bound baseline cannot be rewritten after the receipt). It must be
    /// the first call on a fresh export — any earlier `event`/`leg1` makes the
    /// export return `ExportError::BaselineNotFirst`, surfaced here as a `DaemonError`.
    /// `&mut self`: it chains a link.
    pub fn write_baseline(&mut self, m: &SealManifest) -> Result<(PathBuf, Link), DaemonError> {
        Ok(self.export.write_baseline(m)?)
    }

    /// Delegates to `TraceExport::write_seal`, which chains `EventBody::Seal` as the last
    /// link and only then writes `seal.json`. `&mut self` and the returned `Link` are the
    /// export's, passed through.
    pub fn write_seal(&mut self, m: &SealManifest) -> Result<(PathBuf, Link), DaemonError> {
        Ok(self.export.write_seal(m)?)
    }

    /// Delegates the V1 cutoff append to the protocol export, which owns the chain
    /// transition and cutoff validation. The writer supplies only its bound entry id.
    pub fn write_cutoff(
        &mut self,
        recorded_at: &str,
        cutoff: CutoffReachedV1,
    ) -> Result<Link, DaemonError> {
        Ok(self
            .export
            .write_cutoff(&self.entry_id, recorded_at, cutoff)?)
    }

    /// Delegates the V1 terminal finalization append to the protocol export, which owns
    /// the cutoff binding and terminal transition. The writer supplies only its bound entry id.
    pub fn write_finalization(
        &mut self,
        recorded_at: &str,
        finalization: FinalizationV1,
    ) -> Result<Link, DaemonError> {
        Ok(self
            .export
            .write_finalization(&self.entry_id, recorded_at, finalization)?)
    }

    /// Recovery-only replacement of the fixed seal document after this same writer has
    /// already chained the exact terminal envelope.
    pub fn repair_seal(
        &self,
        manifest: &SealManifest,
        terminal: &Envelope,
    ) -> Result<PathBuf, DaemonError> {
        Ok(self.export.repair_seal(manifest, terminal)?)
    }

    /// Appends to `acks.jsonl`. Called only after the acknowledgement verified against the
    /// pinned server key, so an unverified ack never reaches the trace.
    pub fn append_ack(&mut self, ack: &AckCheckpoint) -> Result<(), DaemonError> {
        #[cfg(any(test, feature = "test-hooks"))]
        if std::mem::take(&mut self.fail_next_ack) {
            return Err(DaemonError::Io {
                path: self.root().join("acks.jsonl").display().to_string(),
                source: std::io::Error::other("injected acknowledgement append failure"),
            });
        }
        Ok(self.export.append_ack(ack)?)
    }

    /// Make the next acknowledgement append fail before it reaches the export.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn fail_next_ack(&mut self) {
        self.fail_next_ack = true;
    }

    /// Copies the shipped `connectors/<id>/manifest.toml` into the trace directory
    /// byte-for-byte and returns its name **relative to the trace directory**, so
    /// `TraceDescriptorDoc.connector_manifest` names a relative file and the verifier's
    /// digest pin compares the bytes that judged the run. A trace that points at a
    /// manifest the participant edited fails `manifest_not_pinned`.
    ///
    /// The read is this crate's — the connector directory is the daemon's to know — and
    /// the write is `TraceExport::write_manifest`'s, like every other file a verifier
    /// reads. There is one `MANIFEST_FILE` constant, in the crate that owns the trace
    /// directory, so the name in the descriptor and the name on disk cannot drift apart.
    pub fn copy_manifest(&self, connector_dir: &Path) -> Result<String, DaemonError> {
        let from = connector_dir.join(MANIFEST_FILE);
        let bytes = std::fs::read(&from).map_err(|source| DaemonError::Io {
            path: from.display().to_string(),
            source,
        })?;
        self.export.write_manifest(&bytes)?;
        Ok(MANIFEST_FILE.to_string())
    }

    /// Chain one leg record. If redaction changed its bytes, the original offsets travel
    /// on this same envelope and are covered by this same link hash.
    fn record(
        &mut self,
        source: &str,
        body: EventBody,
        out: Redacted,
    ) -> Result<Vec<Envelope>, DaemonError> {
        let at = now_rfc3339();
        let redaction = (!out.spans.is_empty()).then_some(RedactionAudit { spans: out.spans });
        Ok(vec![self.append(source, &at, body, redaction)?])
    }

    /// Test-only: make the `n`-th following append (0-based) — and every append after
    /// it — fail as a `DaemonError::Io` before it reaches the export. See
    /// `appends_before_failure` for the gate.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn fail_append_after(&mut self, n: u32) {
        self.appends_before_failure = Some(n);
    }

    /// Test-only: let the writer append again — the counterpart of
    /// [`ChainWriter::fail_append_after`], so a test can say "and then the failure
    /// cleared" without rebuilding the writer off disk.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn clear_append_failure(&mut self) {
        self.appends_before_failure = None;
    }

    /// Test-only: refuse the next `n` appends with `DaemonError::Refused`, before
    /// anything is written. The **per-post** failure shape — nothing reached the chain,
    /// so the record that met it costs itself and nothing behind it.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn refuse_append_after(&mut self, n: u32) {
        self.refuse_append_at = Some(n);
    }

    /// Build the envelope this crate is allowed to build — `protocol_version`,
    /// `entry_id`, `source`, `recorded_at`, `body` — and hand it to the export, which
    /// stamps `seq`, `prev` and `hash` from its own counter. The `Link` handed over is
    /// left unstamped on purpose: a daemon that named a position would be keeping chain
    /// state of its own, and the export would refuse it as `OutOfOrder` the moment the
    /// two disagreed.
    fn append(
        &mut self,
        source: &str,
        at: &str,
        body: EventBody,
        redaction: Option<RedactionAudit>,
    ) -> Result<Envelope, DaemonError> {
        #[cfg(any(test, feature = "test-hooks"))]
        if let Some(left) = self.refuse_append_at {
            match left.checked_sub(1) {
                Some(n) => self.refuse_append_at = Some(n),
                None => {
                    self.refuse_append_at = None;
                    return Err(DaemonError::Refused(
                        "injected per-post refusal: nothing was written".to_string(),
                    ));
                }
            }
        }
        #[cfg(any(test, feature = "test-hooks"))]
        if let Some(left) = self.appends_before_failure.as_mut() {
            match left.checked_sub(1) {
                Some(n) => *left = n,
                None => {
                    return Err(DaemonError::Io {
                        path: "test".to_string(),
                        source: std::io::Error::other("injected append failure"),
                    });
                }
            }
        }
        let mut env = Envelope {
            protocol_version: goalcup_protocol::PROTOCOL_VERSION,
            entry_id: self.entry_id.clone(),
            source: source.to_string(),
            recorded_at: at.to_string(),
            link: Link {
                seq: 0,
                prev: String::new(),
                hash: String::new(),
            },
            body,
            redaction,
        };
        self.export.append_chain(&mut env)?;
        Ok(env)
    }
}

/// The redacted payload of a leg-1 record, in the shape the store ships.
///
/// The two shapes are the tailer's, mirrored rather than sniffed for: a line-oriented
/// store's `payload` is the harness's raw line as a JSON **string** and its `bytes` are
/// that line, so the redacted bytes are the redacted line; a SQLite store's `payload` is
/// the row as an object and its `bytes` are that payload's canonical JSON, so the
/// redacted bytes parse back into the object. There is no `{path,text}` wrapper on
/// either side.
///
/// With nothing removed the captured payload is shipped as it was captured: the redacted
/// bytes are then the input byte for byte, and re-deriving the value would be a
/// re-serialisation of what the harness wrote.
fn leg1_payload(
    captured: &serde_json::Value,
    out: &Redacted,
) -> Result<serde_json::Value, DaemonError> {
    if out.spans.is_empty() {
        return Ok(captured.clone());
    }
    match captured {
        // Lossy exactly where the tailer was lossy: under the regex crate's default
        // Unicode mode a match is valid UTF-8, so redaction never introduces a
        // replacement character the capture did not already carry.
        serde_json::Value::String(_) => Ok(serde_json::Value::String(
            String::from_utf8_lossy(&out.bytes).into_owned(),
        )),
        _ => Ok(serde_json::from_slice(&out.bytes)?),
    }
}

/// Canonical, private retention bytes for a tailer record that arrived outside the admitted
/// window. This deliberately reuses leg 1's payload reconstruction and connector redactor:
/// a late record must never create a second privacy or payload-shape contract.
///
/// `source_path`, `value`, and the raw redactor input are tailer bookkeeping only. They do not
/// enter the retained bytes; the caller stores this private versioned record separately from the
/// public chain.
pub(crate) fn canonical_out_of_window_leg1(
    record: &CapturedRecord,
    manifest: &ConnectorManifest,
) -> Result<Vec<u8>, DaemonError> {
    #[derive(serde::Serialize)]
    struct TailerRecord<'a> {
        version: u8,
        leg: &'static str,
        store: &'a str,
        payload: &'a serde_json::Value,
    }

    let mut redactor = Redactor::for_connector(manifest)?;
    let redacted = redactor.apply(&record.bytes);
    let payload = leg1_payload(&record.payload, &redacted)?;
    Ok(serde_json::to_vec(&TailerRecord {
        version: 1,
        leg: "tailer",
        store: &record.store,
        payload: &payload,
    })?)
}

#[cfg(test)]
mod tests {
    //! Append failures are injected under `cfg(test)` or the off-by-default `test-hooks`
    //! feature, so no shipped build carries a way to fail a write. The feature lets
    //! integration tests drive the same hook against the post drain.

    use super::*;
    use goalcup_protocol::export::CHAIN_FILE;

    type R = Result<(), Box<dyn std::error::Error>>;

    fn beat(ts: &str) -> EventBody {
        EventBody::Heartbeat {
            ts: ts.to_string(),
            stores_seen: Vec::new(),
            receiver_ok: true,
        }
    }

    /// One record with a secret in it, so the envelope has an inline audit.
    fn secret() -> serde_json::Value {
        serde_json::json!({"headers":{"authorization":"Bearer sk-ant-api03-AAAABBBBCCCCDDDD"}})
    }

    /// [`secret`] as a transcript record, so the altered-envelope tests run on leg 1.
    fn secret_record() -> Result<CapturedRecord, serde_json::Error> {
        let payload = secret();
        let bytes = serde_json::to_vec(&payload)?;
        Ok(tailer_record(payload, &bytes, "/tmp/session.jsonl"))
    }

    fn lines(dir: &Path) -> Result<usize, std::io::Error> {
        Ok(std::fs::read_to_string(dir.join(CHAIN_FILE))?
            .lines()
            .count())
    }

    fn tailer_record(
        payload: serde_json::Value,
        bytes: &[u8],
        source_path: &str,
    ) -> CapturedRecord {
        CapturedRecord {
            store: "messages".to_string(),
            source_path: source_path.to_string(),
            payload,
            value: serde_json::Value::Null,
            bytes: bytes.to_vec(),
        }
    }

    fn connector_with_tailer_secret_pattern() -> goalcup_protocol::manifest::ConnectorManifest {
        let mut manifest = goalcup_protocol::manifest::ConnectorManifest::from_toml_str(
            include_str!("../../../connectors/codex-cli/manifest.toml"),
        )
        .expect("the shipped Codex manifest is valid");
        manifest
            .redaction
            .extra_patterns
            .push(goalcup_protocol::manifest::RedactionPattern {
                kind: "tailer_test_secret".to_string(),
                regex: "tailer-secret-[A-Z0-9]+".to_string(),
            });
        manifest
    }

    #[test]
    fn out_of_window_tailer_bytes_are_canonical_private_and_connector_redacted() -> R {
        let manifest = connector_with_tailer_secret_pattern();
        let raw = r#"{"flavor":"tailer-secret-ABCDE12345"}"#;
        let record = tailer_record(
            serde_json::Value::String(raw.to_string()),
            raw.as_bytes(),
            "/Users/participant/private/session.jsonl",
        );

        let first = canonical_out_of_window_leg1(&record, &manifest)?;
        let second = canonical_out_of_window_leg1(&record, &manifest)?;

        assert_eq!(first, second, "identical input has identical private bytes");
        let retained: serde_json::Value = serde_json::from_slice(&first)?;
        assert_eq!(retained["version"], 1);
        assert_eq!(retained["leg"], "tailer");
        assert_eq!(retained["store"], "messages");
        assert!(retained.get("source_path").is_none());
        let text = std::str::from_utf8(&first)?;
        assert!(!text.contains("tailer-secret-ABCDE12345"));
        assert!(!text.contains("/Users/participant/private/session.jsonl"));
        assert!(text.contains("<redacted:tailer_test_secret>"));
        Ok(())
    }

    #[test]
    fn out_of_window_tailer_bytes_refuse_a_redacted_object_that_cannot_be_reconstructed() {
        let manifest = connector_with_tailer_secret_pattern();
        let record = tailer_record(
            serde_json::json!({"flavor": "tailer-secret-ABCDE12345"}),
            b"not-json tailer-secret-ABCDE12345",
            "/Users/participant/private/bad.sqlite",
        );

        assert!(
            canonical_out_of_window_leg1(&record, &manifest).is_err(),
            "a tailer object must retain the existing leg-1 typed reconstruction gate"
        );
    }

    /// An altered record and its audit are one append. Injecting failure at that append
    /// leaves no parseable altered record behind and does not poison later writes.
    #[test]
    fn a_failed_redacted_append_leaves_no_record_and_the_writer_can_continue() -> R {
        let t = tempfile::tempdir()?;
        let dir = t.path().join("trace");
        let mut w = ChainWriter::create("e1", &dir, Redactor::new())?;
        w.fail_append_after(0);
        assert!(
            w.leg1(&secret_record()?).is_err(),
            "the one altered-envelope append failed"
        );
        assert_eq!(lines(&dir)?, 0, "no half of the altered envelope landed");

        w.clear_append_failure();
        w.event(beat("2026-09-04T00:00:01Z"))?;
        assert_eq!(
            lines(&dir)?,
            1,
            "a pre-append failure leaves the writer fit"
        );
        Ok(())
    }

    /// The body and its audit are one link when the append lands.
    #[test]
    fn an_altered_record_is_one_envelope_with_inline_audit() -> R {
        let t = tempfile::tempdir()?;
        let dir = t.path().join("trace");
        let mut w = ChainWriter::create("e1", &dir, Redactor::new())?;
        let written = w.leg1(&secret_record()?)?;
        assert_eq!(written.len(), 1);
        assert!(written[0].redaction.is_some());
        assert_eq!(lines(&dir)?, 1);
        w.event(beat("2026-09-04T00:00:01Z"))?;
        assert_eq!(lines(&dir)?, 2);
        Ok(())
    }

    #[test]
    fn the_v2_writer_refuses_a_legacy_redaction_event() -> R {
        let t = tempfile::tempdir()?;
        let dir = t.path().join("trace");
        let mut w = ChainWriter::create("e1", &dir, Redactor::new())?;
        let legacy = EventBody::Redaction(goalcup_protocol::redaction::RedactionRecord {
            store: "main".to_string(),
            target_seq: 0,
            salt: "legacy".to_string(),
            spans: Vec::new(),
        });

        assert!(matches!(w.event(legacy), Err(DaemonError::Refused(_))));
        assert_eq!(lines(&dir)?, 0);
        Ok(())
    }

    #[test]
    fn finalization_boundaries_delegate_to_the_protocol_export() -> R {
        let t = tempfile::tempdir()?;
        let dir = t.path().join("trace");
        let mut w = ChainWriter::create("e1", &dir, Redactor::new())?;
        let receipt_sha256 = goalcup_protocol::provenance::Sha256Digest::try_from("a".repeat(64))?;

        let cutoff_link = w.write_cutoff(
            "2026-09-14T00:00:00Z",
            goalcup_protocol::provenance::CutoffReachedV1 {
                receipt_sha256: receipt_sha256.clone(),
                deadline_at: goalcup_protocol::provenance::Rfc3339Utc::try_from(
                    "2026-09-14T00:00:00Z",
                )?,
                pre_cutoff_drain: goalcup_protocol::provenance::PreCutoffDrainOutcomeV1::Complete,
            },
        )?;
        assert_eq!(cutoff_link.seq, 0);

        let finalization_link = w.write_finalization(
            "2026-09-14T00:00:01Z",
            goalcup_protocol::provenance::FinalizationV1::WorkspaceSubmission {
                receipt_sha256,
                termination: goalcup_protocol::provenance::TerminationOutcomeV1::Confirmed {
                    requested_at: goalcup_protocol::provenance::Rfc3339Utc::try_from(
                        "2026-09-14T00:00:01Z",
                    )?,
                    confirmed_at: goalcup_protocol::provenance::Rfc3339Utc::try_from(
                        "2026-09-14T00:00:02Z",
                    )?,
                    method: goalcup_protocol::provenance::TerminationMethodV1::AlreadyExited,
                },
                submission: goalcup_protocol::submission::SubmissionOutcomeV1::Incomplete {
                    reason: goalcup_protocol::submission::SubmissionFailureV1::StorageFailed,
                },
            },
        )?;
        assert_eq!(finalization_link.seq, 1);
        assert_eq!(w.seq_written(), Some(1));
        assert_eq!(lines(&dir)?, 2);
        Ok(())
    }
}
