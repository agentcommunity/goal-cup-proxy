//! Leg-1 capture: the polling readers that turn what a harness wrote into records
//! (D19).
//!
//! **Polling only.** No `notify`, no inotify, no FSEvents. A watch is a per-path kernel
//! resource with its own failure modes, and a tracker that silently stops receiving events
//! is worse than one that reads a little late.
//!
//! **Bounded memory.** No tailer holds a whole store. A line tailer reads at most
//! [`crate::MAX_POLL_BYTES`] out of one file in one poll and sets [`TailBatch::more_available`]
//! when there is more, so a 700 MB rollout is caught up over many polls rather than
//! in one allocation. A whole-file tailer stops at [`MAX_FILE_BYTES`] per file and says so.
//!
//! **No interpretation.** A tailer reports what it saw on disk — a shrunken file, a new
//! inode, a line it could not carry. Whether a shrunken file means a rewritten transcript
//! is `goalcup-verifier`'s judgement, and the two capacity notices exist precisely so that
//! a bound the daemon hit is never chained as evidence of one.

pub mod dir;
pub mod jsonl;
pub mod sqlite;

use crate::cursor::CursorState;
use crate::paths::{PathVars, resolve_template};
use crate::{DaemonError, MAX_FILE_BYTES};
use goalcup_protocol::envelope::EventBody;
use goalcup_protocol::events::TailNoticeKind;
use goalcup_protocol::manifest::{ConnectorManifest, CursorType, Store, StoreMode};

pub use crate::cursor::guard_unsafe;
pub use dir::DirTailer;
pub use jsonl::JsonlTailer;
pub use sqlite::SqliteTailer;

/// One record a harness persisted, as the tailer read it.
pub struct CapturedRecord {
    pub store: String,
    /// The file it came out of. This is the tailer's own bookkeeping, reported beside the
    /// record — it is deliberately not folded into `payload`, which is the harness's bytes
    /// and nothing else.
    pub source_path: String,
    /// The harness's own record, unwrapped: a JSON **string** holding the raw line for a
    /// line-oriented store, a JSON **object** holding the row for a SQLite store. There is
    /// no `{path,text}` wrapper anywhere in the system: with one, the verifier's Claude and
    /// Codex parsers would read `value.get("type")` off a wrapper that has no `type`, and
    /// every export would parse to zero sessions.
    pub payload: serde_json::Value,
    /// The dotted-field view the partitioner and the ordinal cursor read. For a line store
    /// this is the parsed line — `Value::Null` when the line is not JSON, which keeps the
    /// bytes in the chain without inventing a reading of them.
    pub value: serde_json::Value,
    /// The record's raw bytes, **for redaction only**. They enter no hash: the chain link is
    /// [`goalcup_protocol::envelope::Envelope::chain_bytes`] over the redacted payload, so
    /// nothing here is ever what a verifier recomputes.
    ///
    /// What they are for is giving the redactor the same text the payload carries, byte
    /// for byte. For a line-oriented store that is the harness's own line, exactly as it
    /// sits on disk, and re-serialising it from `value` would reformat what the harness
    /// wrote. A SQLite row has no bytes on disk at all, so there it is the canonical JSON
    /// of the payload the row ships as — the row itself for a flat store, `{db,table,row}`
    /// for an envelope store. It has to be the whole payload and not the bare row: an
    /// envelope carries the database path, which is under the participant's home
    /// directory, and a redactor handed anything narrower passes a secret through in the
    /// part of the payload it never saw.
    pub bytes: Vec<u8>,
}

/// Something the tailer observed about a store's files. Two of these are on-disk signatures
/// of a rewritten transcript; two are bounds the daemon itself hit; one is a health signal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TailNotice {
    /// The inode at the path changed: a different file, not a longer one.
    FileReplaced {
        store: String,
        path: String,
        inode_before: Option<u64>,
        inode_after: Option<u64>,
        offset_before: u64,
    },
    /// The file shrank below the saved offset.
    Truncated {
        store: String,
        path: String,
        offset_before: u64,
        len_now: u64,
    },
    /// The tailer went back to zero on a file it had already read past. A consequence, not
    /// a cause: V9 judges the notice that caused it and reads this one as the witness that
    /// the re-read was the tailer's own doing.
    Reread {
        store: String,
        path: String,
        offset_before: u64,
    },
    /// A capacity bound, not a rewrite signature: one line exceeded [`crate::MAX_LINE_BYTES`],
    /// the per-line cap, so `offset_after - offset_before` bytes were discarded.
    /// [`crate::MAX_POLL_BYTES`] is a different bound — how much of a file one poll reads — and
    /// exceeding that sets [`TailBatch::more_available`], which is not a notice at all.
    OverlongLine {
        store: String,
        path: String,
        offset_before: u64,
        offset_after: u64,
    },
    /// A capacity bound: one whole-file capture exceeded [`MAX_FILE_BYTES`], so `captured`
    /// bytes of `actual` reached the chain.
    OversizeFile {
        store: String,
        path: String,
        captured: u64,
        actual: u64,
    },
    /// The store could not be read at all. A health signal about the daemon's own reach,
    /// not a fact about the file's contents, so it chains no `TailNotice` envelope.
    StoreUnavailable {
        store: String,
        path: String,
        reason: String,
    },
}

impl TailNotice {
    pub fn store(&self) -> &str {
        match self {
            TailNotice::FileReplaced { store, .. }
            | TailNotice::Truncated { store, .. }
            | TailNotice::Reread { store, .. }
            | TailNotice::OverlongLine { store, .. }
            | TailNotice::OversizeFile { store, .. }
            | TailNotice::StoreUnavailable { store, .. } => store,
        }
    }

    pub fn path(&self) -> &str {
        match self {
            TailNotice::FileReplaced { path, .. }
            | TailNotice::Truncated { path, .. }
            | TailNotice::Reread { path, .. }
            | TailNotice::OverlongLine { path, .. }
            | TailNotice::OversizeFile { path, .. }
            | TailNotice::StoreUnavailable { path, .. } => path,
        }
    }

    /// The protocol kind. The two capacity notices map to their own kinds: chaining a
    /// 1 MiB overrun as `Truncated` would make V9 read a bound the daemon hit as a rewritten
    /// transcript, at Fail.
    pub fn kind(&self) -> Option<TailNoticeKind> {
        Some(match self {
            TailNotice::FileReplaced { .. } => TailNoticeKind::FileReplaced,
            TailNotice::Truncated { .. } => TailNoticeKind::Truncated,
            TailNotice::Reread { .. } => TailNoticeKind::Reread,
            TailNotice::OverlongLine { .. } => TailNoticeKind::OverlongLine,
            TailNotice::OversizeFile { .. } => TailNoticeKind::OversizeFile,
            TailNotice::StoreUnavailable { .. } => return None,
        })
    }

    /// The chained form. `None` for `StoreUnavailable`, which is a health signal and not an
    /// on-disk fact.
    ///
    /// The envelope carries one pair of offsets for every kind, so each variant says what
    /// its own pair means: for the two capacity notices the difference is the shortfall the
    /// chain does not carry, and for the two rewrite signatures it is where the tailer was
    /// and where the file left it.
    pub fn to_event(&self, at: String) -> Option<EventBody> {
        let (offset_before, offset_after, inode_before, inode_after) = match self {
            TailNotice::FileReplaced {
                inode_before,
                inode_after,
                offset_before,
                ..
            } => (*offset_before, 0, *inode_before, *inode_after),
            TailNotice::Truncated {
                offset_before,
                len_now,
                ..
            } => (*offset_before, *len_now, None, None),
            TailNotice::Reread { offset_before, .. } => (*offset_before, 0, None, None),
            TailNotice::OverlongLine {
                offset_before,
                offset_after,
                ..
            } => (*offset_before, *offset_after, None, None),
            TailNotice::OversizeFile {
                captured, actual, ..
            } => (*captured, *actual, None, None),
            TailNotice::StoreUnavailable { .. } => return None,
        };
        Some(EventBody::TailNotice {
            store: self.store().to_string(),
            path: self.path().to_string(),
            kind: self.kind()?,
            offset_before,
            offset_after,
            inode_before,
            inode_after,
            at,
        })
    }

    /// True only for `Truncated` and `FileReplaced` — the two that raise
    /// `HealthState::StoreReplaced`. A capacity bound is not a rewrite.
    pub fn is_rewrite(&self) -> bool {
        matches!(
            self,
            TailNotice::Truncated { .. } | TailNotice::FileReplaced { .. }
        )
    }
}

/// What one poll produced.
pub struct TailBatch {
    pub records: Vec<CapturedRecord>,
    pub notices: Vec<TailNotice>,
    /// The poll hit a byte cap and there is more to read. Not a notice: nothing went wrong,
    /// and the next poll continues from where this one stopped.
    pub more_available: bool,
    /// The poll stopped at the seen-file bound ([`dir::MAX_SEEN_FILES`]) with files it had
    /// not captured still in front of it.
    ///
    /// The notice beside it is a [`TailNotice::StoreUnavailable`], which chains nothing and
    /// says "store unavailable" about a store that is perfectly readable and merely full —
    /// so a caller cannot tell this bound from a database it could not open by reading the
    /// notices. This flag is what [`crate::runtime::PollReport::files_capped`] counts, and
    /// it is a bound rather than a fault, which is why it is a field here and not a sixth
    /// notice kind.
    pub files_capped: bool,
}

impl TailBatch {
    fn empty() -> TailBatch {
        TailBatch {
            records: Vec::new(),
            notices: Vec::new(),
            more_available: false,
            files_capped: false,
        }
    }
}

/// One store's reader.
///
/// `Send`, because the daemon polls its stores from a worker thread; not `Sync`, because a
/// tailer owns a mutable cursor and exactly one caller may poll it.
pub trait Tailer: Send {
    /// The store name, as the manifest spells it. Every record and notice carries it.
    fn name(&self) -> &str;
    /// Read what is new. Bounded by the byte caps above; a poll that hits one sets
    /// `more_available` rather than reading further.
    ///
    /// **A path this reader could not read is a [`TailNotice::StoreUnavailable`] in the
    /// batch, not an `Err`** — see [`store_unavailable`]. An `Err` here is for a reader that
    /// cannot answer at all; one unreadable file must not discard the records the same poll
    /// already took out of the other files a glob matched.
    fn poll(&mut self) -> Result<TailBatch, DaemonError>;
    /// Where it got to, for the heartbeat and for the buffer that persists it.
    fn cursor(&self) -> CursorState;
    /// Resume from a saved position. D19: a tailer that cannot restore re-reads from zero
    /// and produces a second complete chain of records it has already shipped — which is
    /// expected, is not corruption, and is exactly what `restore` exists to avoid.
    fn restore(&mut self, cursor: CursorState) -> Result<(), DaemonError>;

    /// Lower the per-store file bound so a test can reach it with a handful of files rather
    /// than twenty thousand — [`DirTailer::set_seen_cap`], reachable through the boxed
    /// reader an [`crate::runtime::EntryRuntime`] holds.
    ///
    /// A no-op by default: only the whole-file tailer keeps a file set, and the two
    /// line-oriented readers have no such bound to lower.
    #[cfg(any(test, feature = "test-hooks"))]
    fn set_seen_cap(&mut self, _cap: usize) {}
}

/// Build the reader for a store's own `path_template`.
///
/// This is one of the readers a store may declare — see [`tailers_for`], which is what a
/// caller building an entry's readers wants, because `subagent_path_template` and
/// `spill_path_template` are stores' paths too and a caller that only calls this one reads
/// none of them.
///
/// **Order matters and is fixed here.** The `SessionSeq` refusal happens first, before
/// [`guard_unsafe`] is called, so the message names the cut rather than an unrelated store
/// field. Both refusals fire on OpenCode's `event` store — it declares
/// `unsafe_cursors = ["event.seq"]` and `cursor_field_name(SessionSeq)` is `seq` — and a
/// guard-first order would answer the same call with the unsafe-cursor message, hiding the
/// fact that the tailer was cut rather than forbidden.
pub fn tailer_for(
    store: &Store,
    manifest: &ConnectorManifest,
    vars: &PathVars,
) -> Result<Box<dyn Tailer>, DaemonError> {
    if store.cursor.kind == CursorType::SessionSeq {
        return Err(DaemonError::Refused(format!(
            "store `{}` declares cursor type `session_seq`, and no tailer for it is built: \
             no M1 connector uses one and round 2 cut the implementation to M3",
            store.name
        )));
    }
    guard_unsafe(manifest, store)?;

    let pattern = resolve_template(&store.path_template, vars)?;
    reader(store, &store.name, pattern)
}

/// Which declaration produced a reader. This is retained as data so downstream admission
/// never has to infer a parent from a generated suffix or a source basename.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Primary,
    Subagent,
    Spill,
}

/// One fixed manifest declaration. A derived declaration deliberately has no resolved path:
/// fresh construction has no corroborated session and `{session_id}` must not become `*`
/// authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredBinding {
    name: String,
    parent_store: String,
    kind: SourceKind,
    template: String,
}

impl DeclaredBinding {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn parent_store(&self) -> &str {
        &self.parent_store
    }

    pub fn kind(&self) -> SourceKind {
        self.kind
    }

    pub fn template(&self) -> &str {
        &self.template
    }
}

/// One active declaration after its exact path authority has been resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceBinding {
    declaration: DeclaredBinding,
    resolved_pattern: String,
}

impl SourceBinding {
    pub fn name(&self) -> &str {
        self.declaration.name()
    }

    pub fn parent_store(&self) -> &str {
        self.declaration.parent_store()
    }

    pub fn kind(&self) -> SourceKind {
        self.declaration.kind()
    }

    pub fn resolved_pattern(&self) -> &str {
        &self.resolved_pattern
    }
}

/// A reader and the declaration that authorizes every record it can emit.
pub struct BoundTailer {
    pub binding: SourceBinding,
    pub reader: Box<dyn Tailer>,
}

/// The manifest grammar cannot spell an orphan: every production declaration derives its
/// parent from the store currently being traversed. Kept under `cfg(test)` rather than the
/// integration-test feature so no shipped or integration-facing API can construct one.
/// `runtime::entry` uses this only to prove its second-line parent lookup fails closed if
/// that static invariant is ever broken by a future internal construction path.
#[cfg(test)]
pub(crate) fn orphan_bound_tailer_for_runtime_test() -> BoundTailer {
    let declaration = DeclaredBinding {
        name: "orphan_subagent".to_string(),
        parent_store: "missing-parent".to_string(),
        kind: SourceKind::Subagent,
        template: "/test-orphan/{session_id}/child.jsonl".to_string(),
    };
    BoundTailer {
        binding: SourceBinding {
            declaration,
            resolved_pattern: "/test-orphan/session/child.jsonl".to_string(),
        },
        reader: Box::new(JsonlTailer::new(
            "orphan_subagent",
            "/test-orphan/session/child.jsonl".to_string(),
            None,
        )),
    }
}

/// Build and validate the fixed declaration roster without resolving a derived session path.
/// This runs during entry construction, before any start receipt can be requested.
pub fn binding_roster(manifest: &ConnectorManifest) -> Result<Vec<DeclaredBinding>, DaemonError> {
    let primary_names: std::collections::BTreeSet<&str> = manifest
        .store
        .iter()
        .map(|store| store.name.as_str())
        .collect();
    let primary_templates: std::collections::BTreeSet<&str> = manifest
        .store
        .iter()
        .map(|store| store.path_template.as_str())
        .collect();
    let mut names: std::collections::BTreeSet<String> = primary_names
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    let mut derived_templates: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();
    let mut out = Vec::new();

    for store in &manifest.store {
        out.push(DeclaredBinding {
            name: store.name.clone(),
            parent_store: store.name.clone(),
            kind: SourceKind::Primary,
            template: store.path_template.clone(),
        });
        for (kind, suffix, template) in [
            (
                SourceKind::Subagent,
                "subagent",
                store.subagent_path_template.as_ref(),
            ),
            (
                SourceKind::Spill,
                "spill",
                store.spill_path_template.as_ref(),
            ),
        ] {
            let Some(template) = template else {
                continue;
            };
            // Codex declares its subagent path as the primary rollout path. It remains one
            // primary reader, even while unbound, and is not a distinct derived declaration.
            if primary_templates.contains(template.as_str()) {
                continue;
            }
            if store.mode == StoreMode::Sqlite {
                return Err(DaemonError::Refused(format!(
                    "store `{}` is mode `sqlite` and declares a derived {suffix} path",
                    store.name
                )));
            }
            if !template
                .split(['/', '\\'])
                .any(|component| component == "{session_id}")
            {
                return Err(DaemonError::Refused(format!(
                    "store `{}` declares derived {suffix} path `{template}` without an exact session path component",
                    store.name
                )));
            }
            let name = format!("{}_{suffix}", store.name);
            if !names.insert(name.clone()) {
                return Err(DaemonError::Refused(format!(
                    "derived binding `{name}` collides with another declared store or binding"
                )));
            }
            if let Some(first) = derived_templates.insert(template.clone(), name.clone()) {
                return Err(DaemonError::Refused(format!(
                    "derived bindings `{first}` and `{name}` declare the same source template `{template}`"
                )));
            }
            out.push(DeclaredBinding {
                name,
                parent_store: store.name.clone(),
                kind,
                template: template.clone(),
            });
        }
    }
    Ok(out)
}

/// Resolve and construct all currently authorized readers in parent poll order. Primaries are
/// always active. Distinct derived declarations become active only with a safe exact session.
pub fn build_bound_tailers(
    declarations: &[DeclaredBinding],
    manifest: &ConnectorManifest,
    vars: &PathVars,
    parent_order: &[String],
) -> Result<Vec<BoundTailer>, DaemonError> {
    let primary_patterns = declarations
        .iter()
        .filter(|binding| binding.kind == SourceKind::Primary)
        .map(|binding| {
            resolve_template(&binding.template, vars).map(|pattern| (binding.name.clone(), pattern))
        })
        .collect::<Result<std::collections::BTreeMap<_, _>, _>>()?;
    if declarations
        .iter()
        .any(|binding| binding.kind != SourceKind::Primary)
        && let Some(session) = vars.session_id.as_deref()
    {
        validate_session_component(session)?;
    }

    let mut derived_patterns: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();
    let mut out = Vec::with_capacity(declarations.len());
    for parent in parent_order {
        for declaration in declarations
            .iter()
            .filter(|binding| binding.parent_store == *parent)
        {
            let store = manifest
                .store_named(&declaration.parent_store)
                .ok_or_else(|| {
                    DaemonError::Refused(format!(
                        "binding `{}` names unknown parent store `{}`",
                        declaration.name, declaration.parent_store
                    ))
                })?;
            let pattern = resolve_template(&declaration.template, vars)?;
            if declaration.kind != SourceKind::Primary {
                if vars.session_id.is_none() {
                    continue;
                }
                if pattern.contains("{session_id}") {
                    return Err(DaemonError::Refused(format!(
                        "derived binding `{}` retained unresolved session authority",
                        declaration.name
                    )));
                }
                if primary_patterns.values().any(|primary| primary == &pattern) {
                    return Err(DaemonError::Refused(format!(
                        "derived binding `{}` resolves to an active primary pattern `{pattern}`",
                        declaration.name
                    )));
                }
                if let Some(first) =
                    derived_patterns.insert(pattern.clone(), declaration.name.clone())
                {
                    return Err(DaemonError::Refused(format!(
                        "derived bindings `{first}` and `{}` resolve to the same source pattern `{pattern}`",
                        declaration.name
                    )));
                }
            }
            let reader = if declaration.kind == SourceKind::Primary {
                tailer_for(store, manifest, vars)?
            } else if declaration.kind == SourceKind::Spill {
                // A spill is one completed blob per file, not another JSONL transcript.
                // Its parent still supplies the declaration and partition authority, while
                // the child reader supplies the whole-file capture semantics.
                Box::new(DirTailer::new(&declaration.name, pattern.clone()))
            } else {
                guard_unsafe(manifest, store)?;
                reader(store, &declaration.name, pattern.clone())?
            };
            out.push(BoundTailer {
                binding: SourceBinding {
                    declaration: declaration.clone(),
                    resolved_pattern: pattern,
                },
                reader,
            });
        }
    }
    Ok(out)
}

fn validate_session_component(session: &str) -> Result<(), DaemonError> {
    let glob_or_separator = |ch: char| {
        matches!(ch, '/' | '\\' | '*' | '?' | '[' | ']' | '{' | '}' | ':') || ch.is_control()
    };
    if session.is_empty()
        || matches!(session, "." | "..")
        || std::path::Path::new(session).is_absolute()
        || session.chars().any(glob_or_separator)
    {
        return Err(DaemonError::Refused(format!(
            "bound session `{session}` is not a safe literal path component"
        )));
    }
    Ok(())
}

/// Compatibility helper for callers that want one parent's active readers. Declaration
/// validation and primary-pattern dedup remain manifest-wide.
pub type NamedTailer = (String, Box<dyn Tailer>);

/// Every reader one store declares: its own `path_template`, then its
/// `subagent_path_template`, then its `spill_path_template`.
///
/// Both extra templates are token-validated by the manifest loader against the same
/// allowlist as `path_template`, and both are read here. On Claude Code they are the whole
/// subagent transcript directory and the whole spilled `tool-results/` directory, which
/// `human_rule`'s `sidechain` exclusion and its `not_subagent_thread_source` scoping clause
/// both judge records from. A scoping clause with no records to scope is not a passing
/// check; it is a check nothing reaches.
///
/// **The derived names.** A manifest names one store, so the extra readers are named
/// `{store}_subagent` and `{store}_spill` — derived here, in one place, rather than left to
/// each caller. Nothing downstream reads a store name for meaning: the verifier's Claude
/// Code parser answers "is this a subagent's line?" from the record's own `isSidechain` and
/// `agentId` and says in its own doc comment that the store's name is never consulted, and
/// V10 skips a chained store its manifest does not name.
///
/// **The subagent reader is the parent store's own kind, not a directory reader.** The
/// subagent transcript is a transcript — a JSONL file of records, with the same cursor its
/// parent declares — and a whole-file capture of one would ship a multi-line file as a
/// single record whose `value` parses to `null`, which is exactly nothing for the sidechain
/// rules to read. The spill reader is a directory reader: a spill is whole files written
/// once and never appended to, which is what `mode = "dir"` means.
///
/// **A template that renders to a pattern already read is skipped.** Codex declares
/// `subagent_path_template` identical to its `rollout` store's `path_template` — its
/// subagent rollout is a file in the same dated directory, already inside the glob — and a
/// second reader over it would chain every record twice, which reads downstream as a
/// harness replaying its transcript over a run that did nothing of the kind.
///
/// **"Already read" means by any store, not only this one.** A manifest with a second store
/// already globbing the same directory — nothing rules that out, and one store's
/// `subagent_path_template` rendering to another store's `path_template` is the same
/// double-chaining Codex's own-pattern case guards against — would otherwise get a second
/// reader over bytes a sibling store already tails. So the comparison set is seeded from
/// every store the manifest declares, rendered under the same `vars` this call already
/// carries, not just `store`'s own pattern.
pub fn tailers_for(
    store: &Store,
    manifest: &ConnectorManifest,
    vars: &PathVars,
) -> Result<Vec<NamedTailer>, DaemonError> {
    let declarations = binding_roster(manifest)?;
    let order: Vec<String> = manifest
        .store
        .iter()
        .map(|store| store.name.clone())
        .collect();
    Ok(build_bound_tailers(&declarations, manifest, vars, &order)?
        .into_iter()
        .filter(|bound| bound.binding.parent_store() == store.name)
        .map(|bound| (bound.binding.name().to_string(), bound.reader))
        .collect())
}

/// One reader of the kind `store.mode` names, over `pattern`, under `name`. The name and
/// the pattern are arguments rather than read off the store, because a store has more than
/// one of each ([`tailers_for`]).
fn reader(store: &Store, name: &str, pattern: String) -> Result<Box<dyn Tailer>, DaemonError> {
    Ok(match store.mode {
        StoreMode::Jsonl => {
            // The ordinal cursor is a JSONL cursor with a second key, not a second reader:
            // the column names the field on the record, and the bytes are still read by
            // inode and offset.
            let ordinal_field = match store.cursor.kind {
                CursorType::Ordinal => store.cursor.column.clone(),
                _ => None,
            };
            Box::new(JsonlTailer::new(name, pattern, ordinal_field))
        }
        StoreMode::Dir => Box::new(DirTailer::new(name, pattern)),
        // `SqliteTailer::new` takes the store and reads its own name off it, so a derived
        // reader would come back carrying the parent's name — two tailers answering
        // `name()` with one string, and a chain whose records cannot be told apart.
        // Refused rather than silently built: no shipped connector declares a subagent or
        // spill path on a `mode = "sqlite"` store, and the day one does, the reader needs
        // its own name and this is where that is noticed.
        StoreMode::Sqlite if name != store.name => {
            return Err(DaemonError::Refused(format!(
                "store `{}` is mode `sqlite` and declares a second path template (`{name}`): \
                 a SQLite reader is named after its store and cannot carry a derived name",
                store.name
            )));
        }
        // The resolved template and not a glob expansion: a SQLite store names one
        // database file, and the file does not exist until the harness starts — a reader
        // built from what the glob matched today would be a reader for no store at all.
        StoreMode::Sqlite => Box::new(SqliteTailer::new(store, pattern)?),
    })
}

/// Read a dotted field off a JSON value: `payload.ordinal`, `session_meta.cwd`. Object keys
/// only — an index into an array is not a field name, and no manifest writes one. An empty
/// path is `None`, not the whole value: a store that declared no column asked for nothing.
pub fn dotted_json<'a>(v: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    if path.is_empty() {
        return None;
    }
    let mut cur = v;
    for segment in path.split('.') {
        cur = cur.get(segment)?;
    }
    Some(cur)
}

/// One path a poll could not read, as a notice the batch carries rather than an error that
/// discards it.
///
/// The contract `sqlite.rs` states for a database is the whole module's: **a store the
/// daemon cannot reach is a [`TailNotice::StoreUnavailable`], never an `Err`.** A glob
/// matches many files, `expand_globs` sorts them, and the two line-oriented readers move a
/// file's position as soon as they have parsed it — so an `?` on file *N* would throw away
/// the records already taken out of the files before it, whose cursor had already moved past
/// them. That would be the one way this daemon could lose a record the harness really wrote
/// with no witness on disk and none in the chain. With the notice, the batch and its other
/// files' progress reach the caller, the failing path keeps its position, and the next poll
/// retries it.
///
/// `doing` names the call, because "could not stat" and "could not open" are different
/// facts about the same path.
pub(crate) fn store_unavailable(
    store: &str,
    path: &str,
    doing: &str,
    source: &DaemonError,
) -> TailNotice {
    TailNotice::StoreUnavailable {
        store: store.to_string(),
        path: path.to_string(),
        reason: format!("could not {doing} this file: {source}"),
    }
}

/// One file's identity and size, or `None` when it is not there. A file that vanished
/// between the glob and the stat is normal — a harness rotates its own transcripts — and is
/// not an error.
pub(crate) struct FileFacts {
    pub inode: u64,
    pub len: u64,
    /// Nanoseconds since the epoch, negative before it. `i128` because a `SystemTime` may
    /// sit on either side of the epoch and the difference must not saturate at zero.
    pub mtime_nanos: i128,
    /// The device the inode is numbered on. An inode number is unique only within one
    /// filesystem, so "the same inode" is only "the same file" together with this.
    pub dev: u64,
}

pub(crate) fn stat(path: &std::path::Path) -> Result<Option<FileFacts>, DaemonError> {
    let md = match std::fs::metadata(path) {
        Ok(md) => md,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(DaemonError::Io {
                path: path.display().to_string(),
                source,
            });
        }
    };
    if !md.is_file() {
        return Ok(None);
    }
    Ok(Some(FileFacts {
        inode: inode_of(&md),
        dev: dev_of(&md),
        len: md.len(),
        mtime_nanos: mtime_nanos(&md),
    }))
}

#[cfg(unix)]
fn inode_of(md: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    md.ino()
}

/// Off unix there is no inode, so replacement is detected by the other two facts a
/// rotation changes. Stated rather than faked with a random number: a cursor that claimed
/// an inode it did not read would make every restart look like a replacement.
#[cfg(not(unix))]
fn inode_of(_md: &std::fs::Metadata) -> u64 {
    0
}

#[cfg(unix)]
fn dev_of(md: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    md.dev()
}

/// Same reasoning as [`inode_of`]: a constant, not an invention.
#[cfg(not(unix))]
fn dev_of(_md: &std::fs::Metadata) -> u64 {
    0
}

fn mtime_nanos(md: &std::fs::Metadata) -> i128 {
    let Ok(t) = md.modified() else {
        return 0;
    };
    match t.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_nanos() as i128,
        Err(e) => -(e.duration().as_nanos() as i128),
    }
}

/// Read at most `max` bytes from `path` starting at `offset`, into a fresh buffer. The
/// buffer is the only copy the tailer holds, and `max` is what bounds it: no `std::fs::read`
/// and no `read_to_string` anywhere in this module.
pub(crate) fn read_window(
    path: &std::path::Path,
    offset: u64,
    max: u64,
) -> Result<Vec<u8>, DaemonError> {
    use std::io::{Read, Seek, SeekFrom};
    let io = |source| DaemonError::Io {
        path: path.display().to_string(),
        source,
    };
    let mut f = std::fs::File::open(path).map_err(io)?;
    f.seek(SeekFrom::Start(offset)).map_err(io)?;
    // `MAX_FILE_BYTES` is the larger of the two caps, so it bounds the allocation for both
    // callers even when `max` comes from a length the filesystem reported.
    let mut buf = Vec::with_capacity(max.min(MAX_FILE_BYTES) as usize);
    f.take(max).read_to_end(&mut buf).map_err(io)?;
    Ok(buf)
}
