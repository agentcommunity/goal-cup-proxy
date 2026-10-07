//! Capture-time partitioning (D20).
//!
//! D20 is unambiguous: the daemon "applies the connector's `store[].partition_key` before
//! hashing or shipping and discards foreign rows without persisting them". So this module
//! runs before redaction and before any hash, and [`Partitioner::admit`] answers with an
//! [`Outcome`] and nothing else — no rewritten record, no buffered copy, no second list of
//! "records we dropped". A discard is a counter. The output type cannot carry a foreign
//! row, which is what makes "a foreign row never reaches a later stage" a property of the
//! signature rather than of a caller's discipline.
//!
//! **No interpretation.** Every arm below compares a field on the record against either a
//! constant the manifest carries or a value another record taught. Whether the comparison
//! *means* the participant ran a second session in the workspace is `goalcup-verifier`'s
//! judgement (V4, V10), never this module's.
//!
//! **Per store, not per run.** [`tracked_for`] answers per store because the stores compare
//! different things: Codex's `history` store partitions on `session_id`, so a workspace path
//! in its tracked list would never match a thread UUID and would discard every record.
//!
//! **Bounded memory.** A partitioner holds the tracked values and, per strategy, one short
//! string per file or per aggregate it has learned about. It never holds a record, and
//! `admit` borrows the one it judges, so a poll batch larger than any single file's cap
//! streams through without a copy.
//!
//! # Poll order is part of the contract
//!
//! A verdict is reached once. [`Partitioner::admit`] borrows the record, answers, and the
//! caller drops the foreign bytes on the spot — D20 leaves nothing to re-offer, so a row
//! judged `UnknownAggregate` because its teaching record had not arrived yet is lost for
//! the rest of the run, even though every later row of that same aggregate is kept.
//!
//! **The obligation is the caller's, every cycle:** poll the `teach_store` — or, for
//! a store whose `teach_store` is `None`, the self-teaching store itself — and run
//! [`teach`] over its batch before calling `admit` on any store that learns from it. On
//! OpenCode that reads: poll `event`, `admit` + [`teach`] each of its rows, and only then
//! admit `message` and `part`. Within one cycle the same holds row by row, because
//! `session.created.1` and the first `message` row of a session can land in the same poll.
//! Doing it the other way round costs the head of every session, silently, as a counter.

use crate::DaemonError;
use crate::tailer::CapturedRecord;
use goalcup_protocol::manifest::{PartitionSpec, Store};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Why a record was not kept. Each reason names the strategy that reached it, so a caller
/// reporting a count never has to guess which comparison failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscardReason {
    /// `FileFirstRecord`: this file has not produced a valid deciding record yet. Its raw
    /// bytes cannot be attributed, so they are discarded while leaving the file free for
    /// the first later record that does carry a valid deciding value.
    UndecidedFile,
    /// `FileFirstRecord`: this file's deciding record named a value that is not tracked,
    /// so the whole file is foreign (Codex's per-thread rollout).
    ForeignFile,
    /// `AggregateLearned`: the row's aggregate has been taught a value, and that value is
    /// not tracked (OpenCode's shared `opencode.db`).
    ForeignAggregate,
    /// `Direct`: the row carries its own key and it is not one of the tracked values —
    /// or it carries no key at all, which is not a reason to keep it.
    ForeignValue,
    /// `AggregateLearned`: nothing has taught this row's aggregate yet, or the row names
    /// no aggregate. Not the same thing as foreign, and counted separately, because a
    /// store that discards every row for want of a teaching record is a capture defect
    /// and a store that discards rows belonging to another session is capture working.
    UnknownAggregate,
}

/// What [`Partitioner::admit`] decided. There is deliberately no third variant: a record
/// the daemon cannot judge is judged by the rule its strategy states, not deferred into a
/// holding pen the rest of the pipeline would have to know about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Keep,
    Discard(DiscardReason),
}

/// One store's partitioner. Built from that store's own `partition` spec and its own
/// tracked values ([`tracked_for`]).
pub struct Partitioner {
    /// `None` is a store the manifest declares with an `unpartitioned_reason`: its bytes
    /// are not shared with another session, so there is nothing to compare and everything
    /// is kept.
    spec: Option<PartitionSpec>,
    tracked: BTreeSet<String>,
    /// `FileFirstRecord`: source path → the first valid deciding value this process read.
    file_values: BTreeMap<String, String>,
    /// `AggregateLearned`: aggregate id → the value a teaching record carried, whether
    /// that record arrived through `admit` (this store teaches itself) or through
    /// [`teach`] (another store teaches it).
    aggregate_values: BTreeMap<String, String>,
    /// `Direct` only: this store may take the first session id it sees as the tracked one,
    /// while it tracks nothing. Off at construction; [`Partitioner::teach_on_first_sight`]
    /// says who turns it on and on what ground.
    teach_first: bool,
    /// The id a first record was allowed to teach, kept so binding can reconcile against
    /// it. Written once and never overwritten — a second id is a foreign thread, not a
    /// correction.
    taught: Option<String>,
    discarded: u64,
    unknown_aggregates: u64,
}

impl Partitioner {
    pub fn new(spec: Option<PartitionSpec>, tracked: Vec<String>) -> Partitioner {
        Partitioner {
            spec,
            tracked: tracked.into_iter().collect(),
            file_values: BTreeMap::new(),
            aggregate_values: BTreeMap::new(),
            teach_first: false,
            taught: None,
            discarded: 0,
            unknown_aggregates: 0,
        }
    }

    /// Build one binding-local partitioner from its declared parent. A JSONL child has the
    /// parent's record grammar but owns independent learned state. A spill is a whole-file
    /// blob and cannot execute a JSON-record partition grammar; the exact session-scoped
    /// source binding is its only supported authority.
    pub(crate) fn for_binding(
        spec: Option<&PartitionSpec>,
        tracked: Vec<String>,
        whole_file: bool,
        binding_name: &str,
    ) -> Result<Partitioner, DaemonError> {
        if whole_file && spec.is_some() {
            return Err(DaemonError::Refused(format!(
                "derived spill binding `{binding_name}` cannot apply its parent's record partition grammar to whole-file blobs"
            )));
        }
        Ok(Partitioner::new(spec.cloned(), tracked))
    }

    /// Judge one record. Keeps nothing, copies nothing, and rewrites nothing.
    pub fn admit(&mut self, record: &CapturedRecord) -> Outcome {
        let outcome = self.judge(record);
        if let Outcome::Discard(reason) = outcome {
            self.discarded += 1;
            if reason == DiscardReason::UnknownAggregate {
                self.unknown_aggregates += 1;
            }
        }
        outcome
    }

    /// How many records this partitioner discarded. A count is all a discard leaves
    /// behind: D20 forbids persisting the row itself.
    pub fn discarded(&self) -> u64 {
        self.discarded
    }

    /// How many of those discards were for want of a teaching record. A subset of
    /// [`Partitioner::discarded`], not a second total.
    pub fn unknown_aggregates(&self) -> u64 {
        self.unknown_aggregates
    }

    /// Reconstruct a `FileFirstRecord` decision from bytes still present below a restored
    /// cursor. This uses the same first-valid-value rule as [`admit`](Self::admit), but it
    /// does not count or return the historical record: restart replay is source-derived
    /// partition state, never a second persistence path for participant content.
    pub(crate) fn reconstruct_file_value(
        &mut self,
        source_path: &str,
        value: &serde_json::Value,
    ) -> bool {
        let Some(PartitionSpec::FileFirstRecord {
            select_path,
            select_value,
            value_path,
        }) = self.spec.as_ref()
        else {
            return false;
        };
        learn_file_value(
            &mut self.file_values,
            source_path,
            value,
            select_path,
            select_value,
            value_path,
        )
    }

    /// A truncate or inode replacement starts a new file generation at the same path. Its
    /// first valid header must decide again before any of the new generation can be kept.
    pub(crate) fn forget_file(&mut self, source_path: &str) {
        self.file_values.remove(source_path);
    }

    /// Drop partial restart reconstruction when any saved file in the store cannot be
    /// read safely. The runtime will then restore none of that store's cursor and replay
    /// every available file from zero.
    pub(crate) fn clear_file_decisions(&mut self) {
        self.file_values.clear();
    }

    /// Re-seed a `Direct` partitioner whose key is the session id, once the session binds.
    /// Clears the learned per-file and per-aggregate caches: a verdict reached against an
    /// empty tracked set was defaulted, not judged.
    ///
    /// **Scope, stated because clearing has a cost.** This is for the `Direct` store whose
    /// tracked value is the session id and is therefore unknown until the harness names
    /// its session. Calling it mid-run on an `AggregateLearned` store drops the values
    /// taught so far, and an aggregate whose `session.created.1` row has already gone by
    /// is then `UnknownAggregate` for the rest of the run — so the runtime re-seeds at
    /// binding time, when the caches hold only pre-binding defaults, and not later.
    ///
    /// **Same spelling obligation as [`tracked_for`]**: the values are
    /// compared byte for byte, so a workspace path passed here is the one
    /// [`std::fs::canonicalize`] returns, with no trailing slash. `/var/folders/…` where
    /// the harness writes `/private/var/folders/…` re-seeds the store into discarding the
    /// participant's own records.
    ///
    /// **It does not clear [`Partitioner::taught`].** A value a first record taught is a
    /// fact about what the store read, not a guess this call corrects; the binding compares
    /// the two and chains the difference. Clearing it here would hide exactly the
    /// disagreement the reconciliation exists to report.
    pub fn set_tracked(&mut self, tracked: Vec<String>) {
        self.tracked = tracked.into_iter().collect();
        self.file_values.clear();
        self.aggregate_values.clear();
    }

    pub fn tracked(&self) -> &BTreeSet<String> {
        &self.tracked
    }

    /// Let a `Direct` store take the first session id it sees as its tracked one, while it
    /// tracks nothing. Idempotent, and a no-op in effect on the other two
    /// strategies: they compare a workspace path `tracked_for` already knew at
    /// construction, so their tracked set is never empty and this arm is never reached.
    ///
    /// **Why a `Direct` store cannot simply wait for the binding.** `admit` judges once and
    /// D20 leaves nothing to re-offer, so a record read before the tracked set is seeded is
    /// lost for the rest of the run. Codex's `SessionStart` fires at the first prompt, which
    /// is the same moment that prompt is written to `history.jsonl` (leg 1, D2), and the tick
    /// drains the stores before it binds, so that record would meet an empty tracked set and be
    /// discarded as `ForeignValue`.
    ///
    /// **Why the first id seen is the participant's.** Not an interpretation of the record:
    /// the workspace is created empty and the pre-arm scan refuses a harness home that
    /// already holds foreign sessions, so at the moment arming succeeds there is no session
    /// in scope but the one about to start. That premise is the runtime's — it owns the
    /// workspace, the scan and the arming gate — which is why this is a switch the runtime
    /// throws at arming time and not a default this module could assume at construction,
    /// when a store may still be polled before the entry is armed.
    ///
    /// **It teaches, it does not decide.** The id is reported by [`Partitioner::taught`] so
    /// that `bind_start` can compare it against the id the harness actually named. A
    /// mismatch is a fact for the chain, never a silent re-teach: this method never
    /// overwrites what it learned, and a second thread's id is judged foreign like any
    /// other.
    pub fn teach_on_first_sight(&mut self) {
        self.teach_first = true;
    }

    /// The session id a first record taught under [`Partitioner::teach_on_first_sight`],
    /// for the binding to reconcile against. `None` when nothing has been taught — either
    /// the mode was never turned on, or no record has carried a key yet.
    pub fn taught(&self) -> Option<&str> {
        self.taught.as_deref()
    }

    fn judge(&mut self, record: &CapturedRecord) -> Outcome {
        // Destructured rather than matched through `&self.spec`, so that the strategy is
        // read from one field while the learned caches are written through another.
        let Partitioner {
            spec,
            tracked,
            file_values,
            aggregate_values,
            teach_first,
            taught,
            ..
        } = self;
        let Some(spec) = spec.as_ref() else {
            return Outcome::Keep;
        };
        match spec {
            // The record carries its own key.
            PartitionSpec::Direct { path } => match str_at(&record.value, path) {
                Some(v) if tracked.contains(v.as_ref()) => Outcome::Keep,
                // The first id seen by an armed store, while it tracks nothing, is the
                // tracked one — see [`Partitioner::teach_on_first_sight`] for the premise
                // and for who is entitled to assert it. Taught, reported, and then
                // compared against like any other tracked value, so the thread after it
                // is judged and not admitted.
                //
                // `taught.is_none()` is required alongside `tracked.is_empty()`: a
                // `Direct` store can be re-seeded through `set_tracked` with an empty
                // vec (see its doc comment — `taught` is deliberately not cleared there),
                // which leaves `tracked` empty again while a value has already been
                // taught. Without this guard the next record would re-teach and silently
                // overwrite what `bind_start` already reconciled against; `taught` is a
                // fact reported once, not a cache the tracked set's emptiness can reopen.
                Some(v) if *teach_first && tracked.is_empty() && taught.is_none() => {
                    let id = v.into_owned();
                    *taught = Some(id.clone());
                    tracked.insert(id);
                    Outcome::Keep
                }
                // An absent field is not permission to keep the row. Before the session
                // binds — and before arming, where nothing entitles this store to call the
                // first id it sees the participant's — the tracked set is empty and this
                // discards everything, which is the point of `set_tracked`: a defaulted
                // verdict is not a judged one.
                _ => Outcome::Discard(DiscardReason::ForeignValue),
            },

            // The file's first matching record decides for the whole file.
            //
            // The manifest wording is "the file's first matching record decides for the whole file
            // (Codex `session_meta`)". What the condition below requires is the first matching
            // record that also carries a string at `value_path`. A `session_meta` with no `cwd`
            // names no value, so it cannot decide anything; treating it as the decider would mean
            // deciding on `None`, and the only readings of that are "keep everything" or "discard
            // everything", neither of which the file's own bytes support. So a valueless header
            // leaves the file undecided and a later header that does carry a value decides it —
            // which is not the case the `None` arm below guards, where a value was already learned
            // and a second header must not overwrite it.
            PartitionSpec::FileFirstRecord {
                select_path,
                select_value,
                value_path,
            } => {
                learn_file_value(
                    file_values,
                    &record.source_path,
                    &record.value,
                    select_path,
                    select_value,
                    value_path,
                );
                match file_values.get(&record.source_path) {
                    Some(v) if tracked.contains(v) => Outcome::Keep,
                    Some(_) => Outcome::Discard(DiscardReason::ForeignFile),
                    // A file that has produced no deciding record has no owner. Keeping
                    // those bytes would persist participant content before capture-time
                    // partitioning had attributed it; a later foreign header cannot undo
                    // a chain link or upload. The bytes leave only the bounded discard
                    // counter, while a later valid header remains free to decide the file.
                    None => Outcome::Discard(DiscardReason::UndecidedFile),
                }
            }

            // One record type teaches an aggregate's value; every other row inherits it.
            PartitionSpec::AggregateLearned {
                aggregate_path,
                teach_select_path,
                teach_select_value,
                value_path,
                teach_store,
            } => {
                // A teaching record reaches this store's own `admit` only when the
                // manifest says the teaching record lives here (`teach_store = None`,
                // OpenCode's `event` table). When it names another store, the value
                // arrives through [`teach`], and looking for a `session.created.1` in
                // `message` would be waiting on a record type that table never holds.
                if teach_store.is_none()
                    && str_at(&record.value, teach_select_path).as_deref()
                        == Some(teach_select_value.as_str())
                    && let (Some(aggregate), Some(value)) = (
                        str_at(&record.value, aggregate_path),
                        str_at(&record.value, value_path),
                    )
                {
                    // First teaching record wins, for the same reason the first
                    // `session_meta` decides a file: an aggregate already learned is not
                    // re-decided by a later row.
                    aggregate_values
                        .entry(aggregate.into_owned())
                        .or_insert_with(|| value.into_owned());
                }
                let Some(aggregate) = str_at(&record.value, aggregate_path) else {
                    return Outcome::Discard(DiscardReason::UnknownAggregate);
                };
                match aggregate_values.get(aggregate.as_ref()) {
                    Some(v) if tracked.contains(v) => Outcome::Keep,
                    Some(_) => Outcome::Discard(DiscardReason::ForeignAggregate),
                    None => Outcome::Discard(DiscardReason::UnknownAggregate),
                }
            }
        }
    }
}

/// Learn only the first matching record that carries a string value. Shared by live admit
/// and source-prefix restart reconstruction so those two paths cannot disagree about what
/// constitutes a valid decider.
fn learn_file_value(
    file_values: &mut BTreeMap<String, String>,
    source_path: &str,
    value: &serde_json::Value,
    select_path: &str,
    select_value: &str,
    value_path: &str,
) -> bool {
    if file_values.contains_key(source_path)
        || str_at(value, select_path).as_deref() != Some(select_value)
    {
        return false;
    }
    let Some(value) = str_at(value, value_path) else {
        return false;
    };
    file_values.insert(source_path.to_string(), value.into_owned());
    true
}

/// The values a store's strategy compares against — **not** one list for every store.
///
/// `Direct` compares the session id the record carries, so before the session binds there
/// is nothing to compare and the list is empty. The other two strategies resolve to a
/// directory, which is the tracked workspace. A store with no `partition` spec asks for
/// nothing (its manifest entry carries an `unpartitioned_reason` instead).
///
/// # The caller must hand in the canonical workspace path
///
/// Values are compared **byte-exactly**, here and in every arm [`Partitioner::admit`] runs,
/// because normalising a path would be interpretation and this module does none. That puts
/// the whole obligation on the seeding site: `workspace` must be the path
/// [`std::fs::canonicalize`] returns, with no trailing slash. On macOS the difference is
/// routine rather than exotic — a tempdir is handed out as `/var/folders/…` while the
/// harness records `/private/var/folders/…` in its own `cwd`, and `/ws/e1` and `/ws/e1/`
/// are two different strings. Seeded with the wrong spelling, a `FileFirstRecord` or
/// `AggregateLearned` store discards the participant's own records as foreign, counting
/// them and keeping nothing. `an_uncanonicalised_tracked_workspace_makes_a_private_var_cwd_foreign`
/// pins that outcome so the contract cannot be read as advice.
pub fn tracked_for(
    spec: Option<&PartitionSpec>,
    workspace: &Path,
    session_id: Option<&str>,
) -> Vec<String> {
    match spec {
        None => Vec::new(),
        Some(PartitionSpec::Direct { .. }) => {
            session_id.map(|s| vec![s.to_string()]).unwrap_or_default()
        }
        Some(PartitionSpec::FileFirstRecord { .. })
        | Some(PartitionSpec::AggregateLearned { .. }) => {
            vec![workspace.display().to_string()]
        }
    }
}

/// Feed a record from `store` to every partitioner whose `AggregateLearned.teach_store`
/// names that store.
///
/// OpenCode's `message` and `part` tables inherit a directory taught by a
/// `session.created.1` row in the `event` table: `session.created.1` is an `event` row and
/// those two tables never see one, so without this they partition on a record type they
/// can never observe and discard the whole session. This is the daemon half of the
/// strategy the manifest describes.
///
/// Nothing is interpreted: the teaching record's own store says where its aggregate id
/// sits (`event` rows carry `aggregate_id`, while the `message` and `part` rows that
/// inherit from them carry `session_id`), and each learner's own spec says which record
/// teaches and which field it teaches from.
///
/// **Call this before admitting the learners, every cycle** (the module
/// docs state the contract in full). A `message` or `part` row admitted ahead of the
/// `event` row that teaches its aggregate is discarded as `UnknownAggregate` and never
/// offered again: `admit` judges once and D20 forbids holding the bytes for a second look.
/// So the poll loop drains and teaches from the `teach_store` first, and only then admits
/// the stores that name it — not once at startup, but on every poll, because a session's
/// `session.created.1` and its first `message` row can arrive in the same batch.
pub fn teach(
    partitioners: &mut BTreeMap<String, Partitioner>,
    store: &str,
    record: &CapturedRecord,
    stores: &[Store],
) {
    let Some(PartitionSpec::AggregateLearned {
        aggregate_path: teacher_aggregate_path,
        ..
    }) = stores
        .iter()
        .find(|s| s.name == store)
        .and_then(|s| s.partition.as_ref())
    else {
        return;
    };
    let Some(aggregate) = str_at(&record.value, teacher_aggregate_path) else {
        return;
    };

    for learner in partitioners.values_mut() {
        let Partitioner {
            spec,
            aggregate_values,
            ..
        } = learner;
        let Some(PartitionSpec::AggregateLearned {
            teach_select_path,
            teach_select_value,
            value_path,
            teach_store,
            ..
        }) = spec.as_ref()
        else {
            continue;
        };
        // `teach_store = None` is the store that teaches itself; it learned this row in
        // `admit` and is not taught twice here.
        if teach_store.as_deref() != Some(store) {
            continue;
        }
        if str_at(&record.value, teach_select_path).as_deref() != Some(teach_select_value.as_str())
        {
            continue;
        }
        let Some(value) = str_at(&record.value, value_path) else {
            continue;
        };
        aggregate_values
            .entry(aggregate.to_string())
            .or_insert_with(|| value.into_owned());
    }
}

/// A dotted field read as a string, descending **through** a JSON-encoded string node.
///
/// A field that is absent, or present as a number, an object or `null`, is `None`:
/// partition values are identifiers and paths, and coercing something else into one would
/// be an interpretation.
///
/// **Why this is not [`crate::tailer::dotted_json`].** OpenCode's
/// `event.data` is a SQLite **TEXT** column holding JSON, and the tailer keeps a TEXT
/// column as a `Value::String` (`sqlite.rs`'s `json_of`) rather than guessing that some
/// text is structure. So on a live run the shipped manifest's
/// `value_path = "data.info.directory"` meets a string at `data`, a lookup that stops
/// there resolves nothing, `teach` never fires, and every `event`, `message` and `part`
/// row of the entry is discarded as `UnknownAggregate` — the whole OpenCode capture, with
/// only a counter left of it. Committed proof:
/// `spikes/opencode/fixtures/leg1-event-main.jsonl` line 1 carries
/// `"data": "{\"sessionID\": …, \"info\": {…}}"`.
///
/// So when path components remain and the node reached is a string, it is parsed as JSON
/// and the walk continues inside it. This is **lookup only**: the parse is local to the
/// read, the record is never rewritten, and a string that is not JSON simply yields
/// `None` — the same answer a wrong path already gives. `dotted_json` itself is left
/// alone; the ordinal cursor that uses it reads a number off a line the tailer already
/// parsed and has no encoded column to cross.
pub(crate) fn str_at<'a>(value: &'a serde_json::Value, path: &str) -> Option<Cow<'a, str>> {
    if path.is_empty() {
        return None;
    }
    let (head, rest) = match path.split_once('.') {
        Some((head, rest)) => (head, Some(rest)),
        None => (path, None),
    };
    // Object keys only, exactly as `dotted_json` has it: an index into an array is not a
    // field name, and no manifest writes one.
    let node = value.get(head)?;
    match rest {
        None => node.as_str().map(Cow::Borrowed),
        Some(rest) => match node {
            serde_json::Value::String(encoded) => {
                let parsed: serde_json::Value = serde_json::from_str(encoded).ok()?;
                str_at(&parsed, rest).map(|found| Cow::Owned(found.into_owned()))
            }
            _ => str_at(node, rest),
        },
    }
}
