//! The trace directory descriptor. `goalcup-verify <trace-dir>` reads `trace.toml`
//! from that directory; every descriptor path is an opaque descendant read through one
//! pinned directory handle. Tests materialize historical fixtures before verification.

use crate::VerifierError;
use crate::budget::RetentionBudget;
use crate::chain::{ChainError, VerifiedChain};
use crate::legs::sha256_hex;
use crate::trace_fs::{TracePath, TraceRoot};
use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::receipt::StartReceipt;
use goalcup_protocol::seal::SealManifest;
use serde::Deserialize;
use std::path::Path;

const TRACE_DESCRIPTOR_MAX_BYTES: u64 = 1024 * 1024;
const CONNECTOR_MANIFEST_MAX_BYTES: u64 = 4 * 1024 * 1024;
const FIXED_JSON_MAX_BYTES: u64 = 16 * 1024 * 1024;
const RAW_BODY_MAX_BYTES: u64 = 64 * 1024 * 1024;
/// Raw API request/response bodies are retained JSON values, not a streaming source. Keep
/// their object count independently finite even when each individual file is tiny.
pub(crate) const RAW_BODY_MAX_COUNT: usize = 4_096;
const LEGACY_TEXT_COMPAT_MAX_BYTES: u64 = 256 * 1024 * 1024;
const ACK_LINE_MAX_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreKind {
    /// The harness's own file: one JSON record per line. The verifier chains it.
    RawJsonl,
    /// The tailer's output: `{t, src, seq, prev, hash, line}` per line.
    ChainJsonl,
}

#[derive(Debug, Clone)]
pub struct Leg1Spec {
    pub name: String,
    pub kind: StoreKind,
    pub path: TracePath,
    /// True when the store is deliberately outside the tracked session, so V10 has
    /// something real to partition away.
    pub foreign: bool,
    /// True when this file was altered after capture — a committed fixture that went
    /// through `spikes/redact.py` or a structural scrub — so its recorded hashes no
    /// longer recompute from its bytes. Set only for a fixture whose own
    /// `fixtures/README.md` documents the scrub. A chain the daemon wrote never carries
    /// it, and is recomputed.
    pub scrubbed: bool,
    /// A re-read from offset 0 on this store is **expected** — a resume the trace author
    /// can point at — so V9 must not read its `TailNotice` as a rewritten store. Defaults
    /// to `false`, so an unexplained truncation is the default reading and a benign one
    /// has to be declared.
    pub expected_reread: bool,
}

#[derive(Debug, Clone)]
pub struct Leg2Spec {
    pub path: TracePath,
}

#[derive(Debug, Clone)]
pub struct RawSpec {
    pub name: String,
    pub path: TracePath,
}

/// One loaded `[[raw]]` body: the hash that links it into `extra.goalcup.raw`, and the
/// filename stem that joins it to the transcript record carrying the same `requestId`.
#[derive(Debug, Clone)]
pub struct RawBody {
    pub name: String,
    /// The file name without its extension, e.g. `rawbody-goal-turn.response`.
    pub stem: String,
    pub sha256: String,
    pub bytes: u64,
    pub value: serde_json::Value,
}

/// A loaded descriptor. Every participant-selected file name is an opaque validated
/// relative path and the pinned root is private to this module.
#[derive(Debug, Clone)]
pub struct TraceDescriptor {
    pub connector: String,
    pub entry_id: String,
    pub workspace: String,
    pub tracked_sessions: Vec<String>,
    pub harness_version: String,
    pub connector_manifest: TracePath,
    pub leg1: Vec<Leg1Spec>,
    pub leg2: Option<Leg2Spec>,
    pub raw: Vec<RawSpec>,
    pub chain: Option<TracePath>,
    pub receipt: Option<TracePath>,
    pub baseline: Option<TracePath>,
    pub expected_reread: bool,
    pub acks: Option<TracePath>,
    pub seal: Option<TracePath>,
    pub server_public_key: Option<String>,
    pub(crate) root: TraceRoot,
    pub(crate) submission_cache: std::sync::Arc<
        std::sync::OnceLock<(
            goalcup_protocol::provenance::Sha256Digest,
            Result<goalcup_protocol::submission::SubmissionManifestV1, String>,
        )>,
    >,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DescriptorDoc {
    connector: String,
    entry_id: String,
    workspace: String,
    tracked_sessions: Vec<String>,
    harness_version: String,
    connector_manifest: String,
    #[serde(default)]
    leg1: Vec<Leg1Doc>,
    #[serde(default)]
    leg2: Option<Leg2Doc>,
    #[serde(default)]
    raw: Vec<RawDoc>,
    #[serde(default)]
    chain: Option<String>,
    #[serde(default)]
    receipt: Option<String>,
    #[serde(default)]
    baseline: Option<String>,
    #[serde(default)]
    expected_reread: bool,
    #[serde(default)]
    acks: Option<String>,
    #[serde(default)]
    seal: Option<String>,
    #[serde(default)]
    server_public_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Leg1Doc {
    name: String,
    kind: StoreKind,
    path: String,
    #[serde(default)]
    foreign: bool,
    #[serde(default)]
    scrubbed: bool,
    #[serde(default)]
    expected_reread: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Leg2Doc {
    path: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDoc {
    name: String,
    path: String,
}

#[cfg(all(test, unix))]
mod load_test_seams {
    use std::cell::RefCell;

    type Hook = Box<dyn FnOnce()>;

    thread_local! {
        static AFTER_ROOT: RefCell<Option<Hook>> = RefCell::new(None);
        static AFTER_DECODE: RefCell<Option<Hook>> = RefCell::new(None);
    }

    pub(super) fn install(
        after_root: impl FnOnce() + 'static,
        after_decode: impl FnOnce() + 'static,
    ) {
        AFTER_ROOT.with(|slot| {
            assert!(
                slot.replace(Some(Box::new(after_root))).is_none(),
                "after-root trace test seam already installed"
            );
        });
        AFTER_DECODE.with(|slot| {
            assert!(
                slot.replace(Some(Box::new(after_decode))).is_none(),
                "after-decode trace test seam already installed"
            );
        });
    }

    pub(super) fn after_root() {
        AFTER_ROOT.with(|slot| {
            if let Some(hook) = slot.borrow_mut().take() {
                hook();
            }
        });
    }

    pub(super) fn after_decode() {
        AFTER_DECODE.with(|slot| {
            if let Some(hook) = slot.borrow_mut().take() {
                hook();
            }
        });
    }
}

impl TraceDescriptor {
    pub fn load(dir: &Path) -> Result<TraceDescriptor, VerifierError> {
        let root = TraceRoot::open(dir)?;
        #[cfg(all(test, unix))]
        load_test_seams::after_root();
        let descriptor = TracePath::parse("trace.toml")?;
        let text = root.read_text_limited(&descriptor, TRACE_DESCRIPTOR_MAX_BYTES)?;
        let doc: DescriptorDoc = toml::from_str(&text).map_err(|source| VerifierError::Toml {
            path: "trace.toml".into(),
            source,
        })?;
        if doc.raw.len() > RAW_BODY_MAX_COUNT {
            return Err(VerifierError::RawBodyCountTooLarge {
                found: doc.raw.len(),
                limit: RAW_BODY_MAX_COUNT,
            });
        }
        let mut raw_names = std::collections::BTreeSet::new();
        let mut raw_paths = std::collections::BTreeSet::new();
        for raw in &doc.raw {
            if !raw_names.insert(raw.name.clone()) {
                return Err(VerifierError::DuplicateRaw {
                    field: "name",
                    value: raw.name.clone(),
                });
            }
            if !raw_paths.insert(raw.path.clone()) {
                return Err(VerifierError::DuplicateRaw {
                    field: "path",
                    value: raw.path.clone(),
                });
            }
        }
        let t = TraceDescriptor {
            connector: doc.connector,
            entry_id: doc.entry_id,
            workspace: doc.workspace,
            tracked_sessions: doc.tracked_sessions,
            harness_version: doc.harness_version,
            connector_manifest: TracePath::parse(doc.connector_manifest)?,
            leg1: doc
                .leg1
                .into_iter()
                .map(|spec| {
                    Ok(Leg1Spec {
                        name: spec.name,
                        kind: spec.kind,
                        path: TracePath::parse(spec.path)?,
                        foreign: spec.foreign,
                        scrubbed: spec.scrubbed,
                        expected_reread: spec.expected_reread,
                    })
                })
                .collect::<Result<_, VerifierError>>()?,
            leg2: doc
                .leg2
                .map(|spec| TracePath::parse(spec.path).map(|path| Leg2Spec { path }))
                .transpose()?,
            raw: doc
                .raw
                .into_iter()
                .map(|spec| {
                    Ok(RawSpec {
                        name: spec.name,
                        path: TracePath::parse(spec.path)?,
                    })
                })
                .collect::<Result<_, VerifierError>>()?,
            chain: doc.chain.map(TracePath::parse).transpose()?,
            receipt: doc.receipt.map(TracePath::parse).transpose()?,
            baseline: doc.baseline.map(TracePath::parse).transpose()?,
            expected_reread: doc.expected_reread,
            acks: doc.acks.map(TracePath::parse).transpose()?,
            seal: doc.seal.map(TracePath::parse).transpose()?,
            server_public_key: doc.server_public_key,
            root,
            submission_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
        };
        // Validate every declared file name before any optional branch can skip it.
        if t.chain.is_none() && t.leg1.is_empty() {
            return Err(VerifierError::Trace(
                "a trace must declare either `chain` or at least one [[leg1]] store".into(),
            ));
        }
        if t.chain.is_none() && t.leg2.is_none() {
            return Err(VerifierError::Trace(
                "a trace must declare either `chain` or a [leg2] table".into(),
            ));
        }
        if t.chain.is_some() && (!t.leg1.is_empty() || t.leg2.is_some()) {
            return Err(VerifierError::ChainDeclaresStores {
                leg1: t.leg1.len(),
                leg2: t.leg2.is_some(),
            });
        }
        if let Some(k) = &t.server_public_key
            && (k.len() != 64 || !k.chars().all(|c| c.is_ascii_hexdigit()))
        {
            return Err(VerifierError::Trace(
                "server_public_key must be 64 hex characters".into(),
            ));
        }
        #[cfg(all(test, unix))]
        load_test_seams::after_decode();
        Ok(t)
    }

    pub fn manifest_text(&self) -> Result<String, VerifierError> {
        self.root
            .read_text_limited(&self.connector_manifest, CONNECTOR_MANIFEST_MAX_BYTES)
    }
    pub fn leg1_text(&self, spec: &Leg1Spec) -> Result<String, VerifierError> {
        self.root
            .read_text_limited(&spec.path, LEGACY_TEXT_COMPAT_MAX_BYTES)
    }
    pub fn leg2_text(&self, spec: &Leg2Spec) -> Result<String, VerifierError> {
        self.root
            .read_text_limited(&spec.path, LEGACY_TEXT_COMPAT_MAX_BYTES)
    }
    pub fn load_leg1(&self, spec: &Leg1Spec) -> Result<crate::legs::Leg1Store, VerifierError> {
        let mut budget = RetentionBudget::standard();
        self.load_leg1_with_budget(spec, &mut budget)
    }
    pub(crate) fn load_leg1_with_budget(
        &self,
        spec: &Leg1Spec,
        budget: &mut RetentionBudget,
    ) -> Result<crate::legs::Leg1Store, VerifierError> {
        let reader = self.root.open_reader(&spec.path)?;
        match spec.kind {
            StoreKind::RawJsonl => crate::legs::Leg1Store::from_raw_jsonl_reader_with_budget(
                &spec.name,
                spec.path.display(),
                reader,
                budget,
            ),
            StoreKind::ChainJsonl => crate::legs::Leg1Store::from_chain_jsonl_reader_with_budget(
                &spec.name,
                spec.path.display(),
                reader,
                spec.scrubbed,
                budget,
            ),
        }
    }
    pub fn load_leg2(&self, spec: &Leg2Spec) -> Result<crate::legs::Leg2Stream, VerifierError> {
        let mut budget = RetentionBudget::standard();
        self.load_leg2_with_budget(spec, &mut budget)
    }
    pub(crate) fn load_leg2_with_budget(
        &self,
        spec: &Leg2Spec,
        budget: &mut RetentionBudget,
    ) -> Result<crate::legs::Leg2Stream, VerifierError> {
        crate::legs::Leg2Stream::from_reader_with_budget(
            spec.path.display(),
            self.root.open_reader(&spec.path)?,
            budget,
        )
    }

    pub fn load_raw(&self) -> Result<Vec<RawBody>, VerifierError> {
        let mut budget = RetentionBudget::standard();
        self.load_raw_with_budget(&mut budget)
    }

    pub(crate) fn load_raw_with_budget(
        &self,
        budget: &mut RetentionBudget,
    ) -> Result<Vec<RawBody>, VerifierError> {
        let mut out = Vec::new();
        for spec in &self.raw {
            let bytes = self
                .root
                .read_bytes_limited(&spec.path, RAW_BODY_MAX_BYTES)?;
            budget.retain(spec.path.display(), bytes.len() as u64)?;
            let value = serde_json::from_slice(&bytes).map_err(|source| VerifierError::Json {
                path: spec.path.display().to_string(),
                line: 0,
                source,
            })?;
            out.push(RawBody {
                name: spec.name.clone(),
                stem: spec.path.file_stem(),
                sha256: sha256_hex(&bytes),
                bytes: bytes.len() as u64,
                value,
            });
        }
        Ok(out)
    }

    pub fn is_legacy(&self) -> bool {
        self.chain.is_none()
    }

    pub fn load_chain(&self) -> Result<Option<VerifiedChain>, VerifierError> {
        let mut budget = RetentionBudget::standard();
        self.load_chain_with_budget(&mut budget)
    }
    pub(crate) fn load_chain_with_budget(
        &self,
        budget: &mut RetentionBudget,
    ) -> Result<Option<VerifiedChain>, VerifierError> {
        let Some(path) = &self.chain else {
            return Ok(None);
        };
        crate::chain::load_and_verify_reader_with_budget(
            path.display(),
            self.root.open_reader(path)?,
            crate::chain::CHAIN_LINE_MAX_BYTES,
            budget,
        )
        .map(Some)
        .map_err(|e| match e {
            ChainError::EvidenceTooLarge { path, line, limit } => VerifierError::EvidenceTooLarge {
                path: format!("{path} line {line}"),
                limit,
            },
            ChainError::AggregateEvidenceTooLarge {
                path,
                bytes,
                records,
                byte_limit,
                record_limit,
            } => VerifierError::AggregateEvidenceTooLarge {
                path,
                bytes,
                records,
                byte_limit,
                record_limit,
            },
            ChainError::Io { path, source } => VerifierError::Io { path, source },
            ChainError::Json { path, line, source } => VerifierError::Json { path, line, source },
            ChainError::Empty { path } => VerifierError::Trace(format!("chain {path} is empty")),
        })
    }
    pub fn load_receipt(&self) -> Result<Option<StartReceipt>, VerifierError> {
        json_file(self.receipt.as_ref(), &self.root)
    }
    pub fn load_baseline(&self) -> Result<Option<SealManifest>, VerifierError> {
        Ok(json_file(self.baseline.as_ref(), &self.root)?.map(SealManifest::sorted))
    }
    pub fn load_acks(&self) -> Result<Vec<AckCheckpoint>, VerifierError> {
        let mut budget = RetentionBudget::standard();
        self.load_acks_with_budget(&mut budget)
    }
    pub(crate) fn load_acks_with_budget(
        &self,
        budget: &mut RetentionBudget,
    ) -> Result<Vec<AckCheckpoint>, VerifierError> {
        match &self.acks {
            None => Ok(Vec::new()),
            Some(path) => jsonl(path, &self.root, budget),
        }
    }
    pub fn load_seal(&self) -> Result<Option<SealManifest>, VerifierError> {
        Ok(json_file(self.seal.as_ref(), &self.root)?.map(SealManifest::sorted))
    }

    /// Read the fixed V4 private submission layout through this descriptor's retained root.
    /// No descriptor-controlled path selects this artifact.
    pub(crate) fn load_submission(
        &self,
        captured: &goalcup_protocol::submission::SubmissionCapturedV1,
    ) -> Result<goalcup_protocol::submission::SubmissionManifestV1, VerifierError> {
        let cached = self.submission_cache.get_or_init(|| {
            (
                captured.manifest_sha256.clone(),
                crate::submission::verify_submission(&self.root, captured)
                    .map_err(|error| error.to_string()),
            )
        });
        if cached.0 != captured.manifest_sha256 {
            return Err(VerifierError::Trace(
                "multiple finalizations name different submission manifests".into(),
            ));
        }
        cached.1.clone().map_err(VerifierError::Trace)
    }
}

/// One whole-file JSON document, or `None` when the descriptor names no path for it.
fn json_file<T: serde::de::DeserializeOwned>(
    path: Option<&TracePath>,
    root: &TraceRoot,
) -> Result<Option<T>, VerifierError> {
    let Some(path) = path else {
        return Ok(None);
    };
    let bytes = root.read_bytes_limited(path, FIXED_JSON_MAX_BYTES)?;
    Ok(Some(serde_json::from_slice(&bytes).map_err(|source| {
        VerifierError::Json {
            path: path.display().to_string(),
            line: 0,
            source,
        }
    })?))
}

/// One JSON value per line, with the line number in any parse error.
fn jsonl<T: serde::de::DeserializeOwned>(
    path: &TracePath,
    root: &TraceRoot,
    budget: &mut RetentionBudget,
) -> Result<Vec<T>, VerifierError> {
    use std::io::{BufRead, Read};

    let mut reader = std::io::BufReader::new(root.open_reader(path)?);
    let mut out = Vec::new();
    let mut line_number = 0usize;
    loop {
        let mut line = Vec::new();
        let read = (&mut reader)
            .take(ACK_LINE_MAX_BYTES + 1)
            .read_until(b'\n', &mut line)
            .map_err(|source| VerifierError::Io {
                path: path.display().to_string(),
                source,
            })?;
        if read == 0 {
            break;
        }
        line_number += 1;
        if u64::try_from(read).unwrap_or(u64::MAX) > ACK_LINE_MAX_BYTES {
            return Err(VerifierError::EvidenceTooLarge {
                path: format!("{} line {line_number}", path.display()),
                limit: ACK_LINE_MAX_BYTES,
            });
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let value = serde_json::from_slice(&line).map_err(|source| VerifierError::Json {
            path: path.display().to_string(),
            line: line_number,
            source,
        })?;
        budget.retain(path.display(), u64::try_from(read).unwrap_or(u64::MAX))?;
        out.push(value);
    }
    Ok(out)
}

#[cfg(all(test, unix))]
mod submission_tests {
    use super::TraceDescriptor;
    use goalcup_protocol::submission::SubmissionCapturedV1;
    use std::os::unix::fs::PermissionsExt;

    const MANIFEST: &str = r#"{"profile":"workspace_snapshot_v1","limit_profile":{"id":"macos-v1"},"effective_limits":{"object_count":1,"total_bytes":1,"pre_cutoff_drain":1,"process_walk":1},"entry_id":"entry-1","receipt_sha256":"1111111111111111111111111111111111111111111111111111111111111111","cutoff_seq":9,"cutoff_chain_hash":"2222222222222222222222222222222222222222222222222222222222222222","finish_reason":"manual","entries":[],"excluded":[],"copy_started_at":"2026-09-14T00:00:00Z","copy_completed_at":"2026-09-14T00:00:01Z","readback_verified_at":"2026-09-14T00:00:02Z","object_count":0,"total_bytes":0}"#;

    #[test]
    fn trace_descriptor_reads_submission_from_the_fixed_private_layout() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("trace.toml"),
            "connector = \"codex-cli\"\nentry_id = \"entry-1\"\nworkspace = \"/ws\"\ntracked_sessions = [\"s1\"]\nharness_version = \"0\"\nconnector_manifest = \"manifest.toml\"\nchain = \"chain.jsonl\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("submission/objects/sha256")).unwrap();
        let path = dir.path().join("submission/manifest.json");
        std::fs::write(&path, MANIFEST).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let descriptor = TraceDescriptor::load(dir.path()).unwrap();
        let captured = SubmissionCapturedV1 {
            manifest_sha256: "7ac7ec62a21ef87e0566e4b1ee34c65beea960129ee13e1623eb788ded792e1e"
                .try_into()
                .unwrap(),
        };

        let manifest = descriptor.load_submission(&captured).unwrap();

        assert_eq!(manifest.entry_id, "entry-1");
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::path::{Path, PathBuf};

    const OPTIONAL_FILES: [&str; 4] = ["receipt.json", "baseline.json", "acks.jsonl", "seal.json"];

    #[test]
    fn a_sparse_oversized_descriptor_is_rejected_before_whole_file_allocation() {
        let fixture = tempfile::tempdir().unwrap();
        let descriptor = std::fs::File::create(fixture.path().join("trace.toml")).unwrap();
        descriptor.set_len(1024 * 1024 + 1).unwrap();
        drop(descriptor);

        assert!(matches!(
            TraceDescriptor::load(fixture.path()),
            Err(VerifierError::EvidenceTooLarge { path, limit })
                if path == "trace.toml" && limit == 1024 * 1024
        ));
    }

    fn copy_compliant_files(root: &Path) {
        std::fs::create_dir(root).unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/compliant");
        for name in [
            "trace.toml",
            "manifest.toml",
            "chain.jsonl",
            "receipt.json",
            "baseline.json",
            "acks.jsonl",
            "seal.json",
        ] {
            std::fs::copy(source.join(name), root.join(name)).unwrap();
        }
    }

    fn write_legacy_files(root: &Path) {
        std::fs::create_dir(root).unwrap();
        std::fs::create_dir(root.join("nested")).unwrap();
        std::fs::write(
            root.join("trace.toml"),
            "connector = \"claude-code-cli\"\n\
             entry_id = \"trusted-entry\"\n\
             workspace = \"/trusted-workspace\"\n\
             tracked_sessions = [\"trusted-session\"]\n\
             harness_version = \"2.1.259\"\n\
             connector_manifest = \"manifest.toml\"\n\
             receipt = \"receipt.json\"\n\
             baseline = \"baseline.json\"\n\
             acks = \"acks.jsonl\"\n\
             seal = \"seal.json\"\n\
             [[leg1]]\n\
             name = \"main\"\n\
             kind = \"raw_jsonl\"\n\
             path = \"leg1.jsonl\"\n\
             [leg2]\n\
             path = \"leg2.jsonl\"\n\
             [[raw]]\n\
             name = \"request\"\n\
             path = \"nested/raw.json\"\n",
        )
        .unwrap();
        std::fs::write(root.join("manifest.toml"), "trusted manifest").unwrap();
        std::fs::write(
            root.join("leg1.jsonl"),
            "{\"type\":\"trusted_leg1\",\"identity\":\"trusted\"}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("leg2.jsonl"),
            "{\"t\":1.0,\"path\":\"/TrustedEvent\",\"body\":{\"identity\":\"trusted\"}}\n",
        )
        .unwrap();
        std::fs::write(root.join("nested/raw.json"), "{\"identity\":\"trusted\"}").unwrap();

        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/compliant");
        for name in OPTIONAL_FILES {
            std::fs::copy(source.join(name), root.join(name)).unwrap();
        }
    }

    #[test]
    fn sparse_whole_documents_hit_typed_limits_before_allocation() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("trace");
        write_legacy_files(&root);

        std::fs::OpenOptions::new()
            .write(true)
            .open(root.join("manifest.toml"))
            .unwrap()
            .set_len(CONNECTOR_MANIFEST_MAX_BYTES + 1)
            .unwrap();
        let trace = TraceDescriptor::load(&root).unwrap();
        assert!(matches!(
            trace.manifest_text(),
            Err(VerifierError::EvidenceTooLarge { path, limit })
                if path == "manifest.toml" && limit == CONNECTOR_MANIFEST_MAX_BYTES
        ));

        std::fs::write(root.join("manifest.toml"), "trusted manifest").unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(root.join("nested/raw.json"))
            .unwrap()
            .set_len(RAW_BODY_MAX_BYTES + 1)
            .unwrap();
        assert!(matches!(
            trace.load_raw(),
            Err(VerifierError::EvidenceTooLarge { path, limit })
                if path == "nested/raw.json" && limit == RAW_BODY_MAX_BYTES
        ));

        std::fs::write(root.join("nested/raw.json"), "{}").unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(root.join("receipt.json"))
            .unwrap()
            .set_len(FIXED_JSON_MAX_BYTES + 1)
            .unwrap();
        assert!(matches!(
            trace.load_receipt(),
            Err(VerifierError::EvidenceTooLarge { path, limit })
                if path == "receipt.json" && limit == FIXED_JSON_MAX_BYTES
        ));
    }

    #[test]
    fn raw_bodies_are_unique_and_spend_the_retention_budget_before_deserializing() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("trace");
        write_legacy_files(&root);
        let descriptor = root.join("trace.toml");
        let original = std::fs::read_to_string(&descriptor).unwrap();
        std::fs::write(
            &descriptor,
            format!("{original}[[raw]]\nname = \"response\"\npath = \"nested/response.json\"\n"),
        )
        .unwrap();
        std::fs::write(root.join("nested/response.json"), "{} ").unwrap();
        let trace = TraceDescriptor::load(&root).unwrap();
        let mut budget = RetentionBudget::with_limits(u64::MAX, 1);
        assert!(matches!(
            trace.load_raw_with_budget(&mut budget),
            Err(VerifierError::AggregateEvidenceTooLarge {
                path,
                records: 2,
                record_limit: 1,
                ..
            }) if path == "nested/response.json"
        ));

        std::fs::write(
            &descriptor,
            format!("{original}[[raw]]\nname = \"request\"\npath = \"nested/response.json\"\n"),
        )
        .unwrap();
        assert!(matches!(
            TraceDescriptor::load(&root),
            Err(VerifierError::DuplicateRaw { field: "name", value }) if value == "request"
        ));
        std::fs::write(
            &descriptor,
            format!("{original}[[raw]]\nname = \"response\"\npath = \"nested/raw.json\"\n"),
        )
        .unwrap();
        assert!(matches!(
            TraceDescriptor::load(&root),
            Err(VerifierError::DuplicateRaw { field: "path", value }) if value == "nested/raw.json"
        ));
    }

    #[test]
    fn many_valid_small_ack_lines_hit_the_typed_retention_budget() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("trace");
        copy_compliant_files(&root);
        let trace = TraceDescriptor::load(&root).unwrap();
        let mut budget = crate::budget::RetentionBudget::with_limits(1024 * 1024, 2);
        assert!(matches!(
            trace.load_acks_with_budget(&mut budget),
            Err(VerifierError::AggregateEvidenceTooLarge {
                path,
                records: 3,
                record_limit: 2,
                ..
            }) if path == "acks.jsonl"
        ));
    }

    fn write_outside_marker_tree(root: &Path) {
        std::fs::create_dir(root).unwrap();
        std::fs::create_dir(root.join("nested")).unwrap();
        std::fs::write(
            root.join("trace.toml"),
            "connector = \"claude-code-cli\"\n\
             entry_id = \"OUTSIDE_SENTINEL\"\n\
             workspace = \"/outside\"\n\
             tracked_sessions = [\"outside\"]\n\
             harness_version = \"0\"\n\
             connector_manifest = \"manifest.toml\"\n\
             chain = \"chain.jsonl\"\n",
        )
        .unwrap();
        for name in [
            "manifest.toml",
            "chain.jsonl",
            "receipt.json",
            "baseline.json",
            "acks.jsonl",
            "seal.json",
        ] {
            std::fs::write(root.join(name), "OUTSIDE_SENTINEL").unwrap();
        }
        std::fs::write(
            root.join("leg1.jsonl"),
            "{\"type\":\"outside_leg1\",\"identity\":\"OUTSIDE_SENTINEL\"}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("leg2.jsonl"),
            "{\"t\":2.0,\"path\":\"/OUTSIDE_SENTINEL-cmd\",\"body\":{\"identity\":\"OUTSIDE_SENTINEL\"}}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("nested/raw.json"),
            "{\"identity\":\"OUTSIDE_SENTINEL\"}",
        )
        .unwrap();
    }

    fn replace_selected_name(root: &Path, moved: &Path, outside: &Path) {
        std::fs::rename(root, moved).unwrap();
        symlink(outside, root).unwrap();
    }

    #[test]
    fn selected_root_handle_is_retained_before_the_fixed_descriptor_read() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("trace");
        let moved = parent.path().join("moved-trace");
        let outside = parent.path().join("outside");
        write_legacy_files(&root);
        write_outside_marker_tree(&outside);

        let hook_root = root.clone();
        let hook_moved = moved.clone();
        let hook_outside = outside.clone();
        load_test_seams::install(
            move || replace_selected_name(&hook_root, &hook_moved, &hook_outside),
            || {},
        );

        let trace = TraceDescriptor::load(&root).unwrap();
        assert_eq!(trace.entry_id, "trusted-entry");
        assert_eq!(trace.manifest_text().unwrap(), "trusted manifest");
    }

    fn assert_every_reader_uses_retained_root(root: PathBuf, moved: PathBuf, outside: PathBuf) {
        let hook_root = root.clone();
        load_test_seams::install(
            || {},
            move || replace_selected_name(&hook_root, &moved, &outside),
        );

        let trace = TraceDescriptor::load(&root).unwrap();
        assert!(!trace.manifest_text().unwrap().contains("OUTSIDE_SENTINEL"));
        assert!(trace.load_receipt().unwrap().is_some());
        assert!(trace.load_baseline().unwrap().is_some());
        assert!(!trace.load_acks().unwrap().is_empty());
        assert!(trace.load_seal().unwrap().is_some());
        if let Some(chain) = trace.load_chain().unwrap() {
            assert!(!chain.envelopes.is_empty());
            assert_eq!(chain.entry_id, "compliant-0001");
        } else {
            assert!(
                trace
                    .leg1_text(&trace.leg1[0])
                    .unwrap()
                    .contains("trusted_leg1")
            );
            let leg1 = trace.load_leg1(&trace.leg1[0]).unwrap();
            assert_eq!(leg1.records.len(), 1);
            assert_eq!(leg1.records[0].value["identity"], "trusted");
            assert!(
                trace
                    .leg2_text(trace.leg2.as_ref().unwrap())
                    .unwrap()
                    .contains("TrustedEvent")
            );
            let leg2 = trace.load_leg2(trace.leg2.as_ref().unwrap()).unwrap();
            assert_eq!(leg2.posts.len(), 1);
            assert_eq!(leg2.posts[0].event, "TrustedEvent");
            assert_eq!(leg2.posts[0].transport(), "http");
            assert_eq!(leg2.posts[0].body["identity"], "trusted");
            let raw = trace.load_raw().unwrap();
            assert_eq!(raw.len(), 1);
            assert_eq!(raw[0].value, serde_json::json!({"identity": "trusted"}));
        }
    }

    #[test]
    fn every_evidence_reader_retains_the_root_handle_after_descriptor_decode() {
        let legacy = tempfile::tempdir().unwrap();
        let legacy_root = legacy.path().join("trace");
        let legacy_moved = legacy.path().join("moved-trace");
        let legacy_outside = legacy.path().join("outside");
        write_legacy_files(&legacy_root);
        write_outside_marker_tree(&legacy_outside);
        assert_every_reader_uses_retained_root(legacy_root, legacy_moved, legacy_outside);

        let modern = tempfile::tempdir().unwrap();
        let modern_root = modern.path().join("trace");
        let modern_moved = modern.path().join("moved-trace");
        let modern_outside = modern.path().join("outside");
        copy_compliant_files(&modern_root);
        write_outside_marker_tree(&modern_outside);
        assert_every_reader_uses_retained_root(modern_root, modern_moved, modern_outside);
    }

    #[test]
    fn descendant_ancestor_swap_after_decode_is_refused() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("trace");
        let outside = parent.path().join("outside");
        write_legacy_files(&root);
        write_outside_marker_tree(&outside);

        let trusted_nested = root.join("trusted-nested");
        let selected_nested = root.join("nested");
        let outside_nested = outside.join("nested");
        load_test_seams::install(
            || {},
            move || {
                std::fs::rename(&selected_nested, &trusted_nested).unwrap();
                symlink(&outside_nested, &selected_nested).unwrap();
            },
        );

        let trace = TraceDescriptor::load(&root).unwrap();
        let error = trace.load_raw().unwrap_err();
        assert!(error.to_string().contains("io nested/raw.json"), "{error}");
        assert!(!error.to_string().contains("OUTSIDE_SENTINEL"), "{error}");
    }
}
