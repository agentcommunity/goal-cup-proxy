//! Gap classification (DECISIONS D9) and acknowledgement reconciliation.
//!
//! A clean gap is an outage: the heartbeat series has a hole longer than two intervals,
//! nothing was appended to any store inside it, and an acknowledgement covers its far
//! edge. A dirty gap is any hole that fails one of those, and the reviewer is told which.
//!
//! Both halves exist because downtime must be a *statement* in the verdict rather than an
//! absence from it. Without the heartbeat, a daemon that was killed for six hours leaves
//! the same trace as a daemon that was never needed.
//!
//! Three rules shape what is measured here:
//!
//!   * **Membership respects the sequence space a record lives in.**
//!     A `ChainJsonl` store carries the daemon's own chain sequences, so a record is
//!     inside a hole when its `seq` lies between the bounding envelopes' `seq`s — the
//!     strongest placement there is, and immune to the participant's clock. A `RawJsonl`
//!     store is chained by the verifier from 1, per store: those sequences collide across
//!     stores and mean nothing next to an envelope's, so they are never compared with
//!     one. Such a record is placed by `recorded_at` where it has one, and where it does
//!     not it is **unplaceable** — 46 of the 148 records in the claude-code-cli fixture
//!     carry no timestamp — which makes a hole `Dirty` under `unplaceable_records`
//!     rather than quietly empty. Timestamps still measure a hole's *duration*, because
//!     nothing else can, and a series that steps backwards in chain order says so.
//!   * **Anchoring is per hole, not per trace.** A hole is `Clean` only when its closing
//!     envelope sits at or below the highest acknowledged sequence. A quiet-looking hole
//!     beyond the last acknowledgement is `Dirty` with reason `unanchored`: nobody
//!     witnessed the window in which the daemon says nothing happened.
//!   * **The daemon's own corroboration is consumed, asymmetrically.**
//!     Each heartbeat carries every store's cursor. More records in the chain than the
//!     cursor moved is `cursor_mismatch` at `Fail`: the chain holds records the daemon
//!     never accounted for. The other direction is not symmetrical, because an honest
//!     daemon reading a home-wide store advances its cursor over rows a partition filter
//!     drops: a cursor ahead of the chain is **silent** for a store the manifest gives a
//!     `partition_key`, and `cursor_ahead_of_chain` at `Flag` for a store it does not.
//!
//!     **Neither direction is reachable unless the cursor counts records** (`StoreCursor`
//!     carries its `CursorUnit`). A number whose unit is not the window's unit is reported
//!     and never subtracted.
//!
//!     Within the records unit the `Flag` side still keys on **zero records with cursor
//!     movement**, not on the arithmetic `delta > records`, and the difference is
//!     deliberate: a partition filter can explain any amount of movement that still
//!     shipped something, and what it cannot explain is movement that produced *nothing*.
//!     The `Fail` side is the literal comparison, because in that direction the arithmetic
//!     holds: a cursor can never pass fewer records than the daemon chained. A
//!     `DaemonRestart` names the head it found, which must be the chain position
//!     immediately below the restart envelope itself, or it is `restart_head_mismatch`.
//!
//! The third outcome of `anchoring` is `NoAnchors`: no acknowledgements and no heartbeat
//! series at all — every M0 fixture, taken before the daemon existed. Nothing there can be
//! called a gap, and manufacturing one whole-chain dirty window would bury the finding
//! that matters. V12 states it once, as `no_anchor`.

use crate::chain::VerifiedChain;
use crate::checks::Severity;
use crate::legs::{Leg1Record, Leg1Store, SeqSpace};
use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::chain::link_hash;
use goalcup_protocol::control::parse_z;
use goalcup_protocol::cursor::StoreCursor;
use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::manifest::ConnectorManifest;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GapKind {
    Clean,
    Dirty,
}

/// One record inside a gap: where to find it, and what it hashes to.
///
/// **No excerpt.** A citation is a store name, a sequence and a hash; the reviewer opens
/// the store with those three. Do not add a text field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GapLine {
    pub store: String,
    pub seq: u64,
    pub sha256: String,
}

/// Why a gap is not clean. Rendered into findings by V4, each under its own code and at
/// its own severity — a `Flag` reason is a question for a reviewer, a `Fail` reason is a
/// window the trace cannot account for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum GapReason {
    /// The chain grew inside the hole.
    RecordsAppended,
    /// A `RawJsonl` store holds records that cannot be placed in or out of this hole:
    /// their sequences are the verifier's own per-store bookkeeping and they carry no
    /// timestamp. Fail closed — an unplaceable record may be inside.
    UnplaceableRecords { store: String, records: usize },
    /// The window closes beyond the highest acknowledged sequence, so no one witnessed
    /// it: a hole nobody was watching, or the stretch of chain above the anchor.
    Unanchored,
    /// The chain holds more records in this window than the daemon's own cursor moved
    /// over: records the daemon never accounted for.
    CursorMismatch {
        store: String,
        records: usize,
        /// The movement, in records. Always `Some`: a reason is only ever built from a
        /// comparison that was actually made, and a cursor whose unit is not `Records`
        /// produces no comparison. The field stays optional because it is the serialised
        /// shape a reader and a fixture already know.
        cursor_delta: Option<u64>,
    },
    /// The daemon's cursor moved over rows that never reached the chain, in a store the
    /// manifest gives no `partition_key` — so no filter explains the difference.
    CursorAheadOfChain {
        store: String,
        cursor_delta: Option<u64>,
    },
    /// A restart named a head the chain does not hold immediately below the restart.
    RestartHeadMismatch { claimed_seq: u64, observed_seq: u64 },
    /// The heartbeat timestamps step backwards in chain order. Under
    /// `BACKSTEP_GRACE_SECONDS` this is a `Flag` — an ordinary clock correction, and the
    /// two beats still bracket the same window; beyond it the window has no measurable
    /// duration and the gap is `Fail`.
    HeartbeatsOutOfOrder { seconds: u64 },
}

/// A backward wall-clock step this size or smaller between adjacent heartbeats is a
/// `Flag`: NTP corrections happen, and chain order is intact either way.
pub const BACKSTEP_GRACE_SECONDS: u64 = 5;

impl GapReason {
    /// How badly this reads. `Fail` on any reason makes the gap `Dirty`.
    pub fn severity(&self) -> Severity {
        match self {
            GapReason::RecordsAppended
            | GapReason::UnplaceableRecords { .. }
            | GapReason::Unanchored
            | GapReason::CursorMismatch { .. }
            | GapReason::RestartHeadMismatch { .. } => Severity::Fail,
            GapReason::CursorAheadOfChain { .. } => Severity::Flag,
            GapReason::HeartbeatsOutOfOrder { seconds } => {
                if *seconds > BACKSTEP_GRACE_SECONDS {
                    Severity::Fail
                } else {
                    Severity::Flag
                }
            }
        }
    }

    /// The finding code V4 raises for this reason on its own, where it raises one.
    pub fn code(&self) -> Option<&'static str> {
        match self {
            GapReason::UnplaceableRecords { .. } => Some("unplaceable_records"),
            GapReason::CursorMismatch { .. } => Some("cursor_mismatch"),
            GapReason::CursorAheadOfChain { .. } => Some("cursor_ahead_of_chain"),
            GapReason::RestartHeadMismatch { .. } => Some("restart_head_mismatch"),
            GapReason::HeartbeatsOutOfOrder { .. } => Some("heartbeats_out_of_order"),
            // Records above the anchor are the only way a gap is Dirty without being a
            // hole, and V4 renders `dirty_gap` for holes only — so without a code of its
            // own that gap would surface nowhere.
            GapReason::Unanchored => Some("unanchored_records"),
            // Said by the `dirty_gap` finding itself; it only ever arises inside a hole.
            GapReason::RecordsAppended => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gap {
    pub kind: GapKind,
    /// True when the two bounding envelopes are more than `2N` apart, or step backwards
    /// at all. A window that is punctual but whose cursors contradict the chain is still
    /// reported — as its reasons, not as downtime. False also for the gap that runs from
    /// the anchor to the head, which is unwitnessed work rather than an outage.
    pub is_hole: bool,
    /// For a heartbeat window, the chain sequences of the two envelopes that bound it. For
    /// a gap above the anchor, the **acknowledged** sequence and the head: the window runs
    /// from the last thing the server witnessed to the end of the chain.
    pub from_seq: u64,
    pub to_seq: u64,
    /// RFC 3339 bounds of the window, where they are known. A gap derived only from
    /// unacknowledged records carries no timestamps.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub to: Option<String>,
    pub appended: Vec<GapLine>,
    /// Empty exactly when the gap is `Clean` with nothing to say. A `Clean` gap may still
    /// carry `Flag`-severity reasons.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub reasons: Vec<GapReason>,
}

/// A proof-of-life envelope whose timestamp could not be read.
/// Reported as `heartbeat_unparseable`, never silently dropped: a beat the verifier cannot
/// place is a beat that cannot bound a hole, and that has to be visible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeartbeatNotice {
    pub seq: u64,
    pub sha256: String,
}

/// Every heartbeat or restart envelope whose timestamp does not parse.
pub fn unparseable_heartbeats(events: &[Envelope]) -> Vec<HeartbeatNotice> {
    let mut out: Vec<HeartbeatNotice> = events
        .iter()
        .filter(|env| {
            matches!(
                &env.body,
                EventBody::Heartbeat { .. } | EventBody::DaemonRestart { .. }
            ) && beat_of(env).is_none()
        })
        .map(|env| HeartbeatNotice {
            seq: env.link.seq,
            sha256: env.link.hash.clone(),
        })
        .collect();
    out.sort_by_key(|n| n.seq);
    out
}

/// One hole in a source's declared receiver counter.
///
/// `Leg2Post::receiver_meta` carries the receiver's own bookkeeping — `receiver::PostMeta`
/// serialised raw. This is the comparison over it, made once, over the full persisted
/// stream, independently of whatever the daemon's own in-memory counter concluded at post
/// time: a restart resets that counter, and a suppressed post cannot be caught by a check
/// reading only what survived.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiverSeqGap {
    pub source: String,
    pub expected: u64,
    pub observed: u64,
    /// The chain position of the post that broke contiguity, so a reviewer can open
    /// exactly this line.
    pub seq: u64,
    pub sha256: String,
}

/// The one shape this rule reads out of `receiver_meta`: `seq` and `source`. `anomalies`
/// and `reported_config_hash` — `receiver::PostMeta`'s other two fields — are ignored by
/// construction rather than named here, the same way `EnvelopeHeader` in `chain.rs` reads
/// only `protocol_version` and `seq` out of a whole envelope.
#[derive(Deserialize)]
struct ReceiverMetaSeqSource {
    seq: Option<u64>,
    source: String,
}

/// Per-source contiguity of the receiver's declared counter, read straight off the
/// chained envelope stream — `VerifiedChain::envelopes`, not the derived `Leg2Post` view,
/// which drops `receiver_meta` on purpose (chain.rs).
///
/// **`seq: None` takes no part in this rule.** Claude Code's native `http` transport
/// dispatches the post itself and has nowhere to run the counter
/// (`receiver::PostAnomaly::Uncounted`'s doc); absence is a stated fact about that source,
/// never a hole, and such a source never enters `last` below — so it can never be flagged.
///
/// **A hole is any declared value that is not `previous + 1`** — a skip, a replay, or a
/// value going backwards — for the same source, save for the one witnessed restart
/// described below. One reason, exactly as the contract asks for; a suppressed post and a
/// replayed one both read as "not contiguous" and both are worth a reviewer's look under
/// the one code this raises.
///
/// The walk continues over what the source declared, not over what it should have
/// declared, so one hole does not turn every later post from that source into a second one
/// (the same rule `chain.rs`'s own `SeqOutOfOrder` walk uses for the chain's own `seq`).
///
/// **The one forgiven discontinuity: a witnessed restart.** When the harness relaunches,
/// every hook source's counter is a fresh process's — the shim starts again at 1 — and a
/// rule that knew only contiguity would read that 1 as "a post between them is missing" and
/// fail every honest run that restarts. A reset is therefore forgiven when, and only when,
/// the chain itself carries a resume between the two posts — see [`resume_witness_seqs`]
/// for what counts as one, and [`witnessed_between`] for the interval.
///
/// Three edges of that forgiveness, each of which is the whole point of it:
///
///   * **Only exactly 1.** A fresh counter's first value is 1 and nothing else. Partial
///     loss after a resume still starts at k+1, and k+1 is still a hole. Forgiving every
///     reset would let a shim drop posts 5..9, declare 1, and walk away clean.
///   * **Only with a witness in that interval.** A reset to 1 with no resume between the
///     two posts stays a finding, and a resume emitted *below* the reset witnesses
///     nothing — otherwise a late-emitted marker launders an earlier drop.
///   * **Never on a `DaemonRestart`.** The shim's counter is a file under the artifact root
///     (`connectors/codex-cli/files/hook-post.sh`), so it survives the daemon dying: a
///     daemon restart does not reset it and must not excuse one. Claude Code's hooks mint
///     no instance id of their own either (`parsers/claude_code.rs`, `ConfigFact.shim`), so
///     "which launch is this" can only come from the resume markers in the chain — never
///     from the post itself.
pub fn receiver_seq_gaps(events: &[Envelope]) -> Vec<ReceiverSeqGap> {
    let resumes = resume_witness_seqs(events);
    // Per source: the last value it declared, and the chain position it declared it at —
    // the second is the lower edge of the interval a witness has to fall inside.
    let mut last: std::collections::BTreeMap<String, (u64, u64)> =
        std::collections::BTreeMap::new();
    let mut out = Vec::new();
    for env in events {
        let EventBody::Leg2Post {
            receiver_meta: Some(meta),
            ..
        } = &env.body
        else {
            continue;
        };
        let Ok(meta) = serde_json::from_value::<ReceiverMetaSeqSource>(meta.clone()) else {
            continue;
        };
        let Some(seq) = meta.seq else { continue };
        if let Some(&(previous, previous_at)) = last.get(&meta.source)
            && seq != previous + 1
            && !(seq == 1 && witnessed_between(&resumes, previous_at, env.link.seq))
        {
            out.push(ReceiverSeqGap {
                source: meta.source.clone(),
                expected: previous + 1,
                observed: seq,
                seq: env.link.seq,
                sha256: env.link.hash.clone(),
            });
        }
        last.insert(meta.source, (seq, env.link.seq));
    }
    out
}

/// Whether this envelope is a resume witness: a `Leg2Post` whose event is `SessionStart`
/// and whose payload declares `source: "resume"`.
///
/// This is the same predicate the harness parsers already build their resume markers from
/// — `parsers/claude_code.rs` (`by_event("SessionStart")`, `source == "resume"`) and
/// `parsers/codex.rs` — read here off the raw envelope stream rather than off the derived
/// leg-2 view, because that is the stream this rule walks and the one the counter's own
/// posts live on. Deliberately not deduped the way `claude_code.rs` dedupes its markers by
/// `(session_id, source)`: one resume firing `SessionStart` twice is two envelopes at two
/// chain positions, and a position is exactly what this rule needs.
fn is_resume_witness(env: &Envelope) -> bool {
    let EventBody::Leg2Post { event, payload, .. } = &env.body else {
        return false;
    };
    event == "SessionStart"
        && crate::legs::dotted(payload, "source").and_then(|v| v.as_str()) == Some("resume")
}

/// The chain positions of every resume witness in the stream, ascending, so an interval
/// query is a binary search rather than a re-scan per post.
fn resume_witness_seqs(events: &[Envelope]) -> Vec<u64> {
    let mut out: Vec<u64> = events
        .iter()
        .filter(|env| is_resume_witness(env))
        .map(|env| env.link.seq)
        .collect();
    out.sort_unstable();
    out
}

/// Is there a witness in `(after, through]`?
///
/// The upper bound is **inclusive**, and that is load-bearing: for Claude Code's
/// `project-settings` source the resume `SessionStart` is the seq-1 post, so an exclusive
/// bound would miss a post's own witness and flag every honest resume. The lower bound is
/// exclusive: the witness has to sit strictly above the previous post from this source, or
/// a resume that was already accounted for would keep excusing later resets.
fn witnessed_between(witnesses: &[u64], after: u64, through: u64) -> bool {
    let first_above = witnesses.partition_point(|w| *w <= after);
    witnesses.get(first_above).is_some_and(|w| *w <= through)
}

/// What the trace can be measured against at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchoring {
    /// At least one acknowledgement: a window can be called watched or unwatched.
    Anchored,
    /// A heartbeat series but no acknowledgement. Holes are visible, but nothing was
    /// witnessed by anyone other than the participant's own daemon, so every hole is
    /// dirty.
    Unanchored,
    /// Neither. There is nothing to classify, no gap is manufactured, and V12 reports
    /// `no_anchor` once.
    NoAnchors,
}

/// Which of the three the trace is in. Read by `classify` and by V12, so the
/// two can never disagree about whether the trace had an anchor.
pub fn anchoring(acks: &[AckCheckpoint], events: &[Envelope]) -> Anchoring {
    if !acks.is_empty() {
        Anchoring::Anchored
    } else if events.iter().any(|e| beat_of(e).is_some()) {
        Anchoring::Unanchored
    } else {
        Anchoring::NoAnchors
    }
}

enum BeatKind<'a> {
    Heartbeat { cursors: &'a [StoreCursor] },
    Restart { head_seq: u64, head_hash: &'a str },
}

/// One envelope in the proof-of-life series, placed by its chain sequence and dated by its
/// own timestamp.
struct Beat<'a> {
    seq: u64,
    prev: &'a str,
    secs: u64,
    ts: &'a str,
    kind: BeatKind<'a>,
}

fn beat_of(env: &Envelope) -> Option<Beat<'_>> {
    let (ts, kind) = match &env.body {
        EventBody::Heartbeat {
            ts, stores_seen, ..
        } => (
            ts.as_str(),
            BeatKind::Heartbeat {
                cursors: stores_seen,
            },
        ),
        EventBody::DaemonRestart {
            at,
            previous_head_seq,
            previous_head_hash,
        } => (
            at.as_str(),
            BeatKind::Restart {
                head_seq: *previous_head_seq,
                head_hash: previous_head_hash,
            },
        ),
        _ => return None,
    };
    parse_z(ts).map(|secs| Beat {
        seq: env.link.seq,
        prev: &env.link.prev,
        secs,
        ts,
        kind,
    })
}

fn cursor_for<'a>(kind: &BeatKind<'a>, store: &str) -> Option<&'a StoreCursor> {
    match kind {
        BeatKind::Heartbeat { cursors } => cursors.iter().find(|c| c.store == store),
        BeatKind::Restart { .. } => None,
    }
}

/// How far the cursor moved between two heartbeats, **in records**.
///
/// `Some` only when both cursors count records (`StoreCursor::records()`); every other unit
/// yields `None`, and `None` means there is no comparison to make. The daemon knows the
/// unit and says it, so the arithmetic happens only where it means something.
///
/// A cursor that went *backwards* reads as no movement, so records in that window are
/// reported against it.
fn cursor_delta(before: &StoreCursor, after: &StoreCursor) -> Option<u64> {
    match (before.records(), after.records()) {
        (Some(b), Some(a)) => Some(a.saturating_sub(b)),
        _ => None,
    }
}

/// Whether every record in this store can be placed in or out of a window.
///
/// An `EntryChain` store always can: its sequences are the daemon's own, and the window is
/// bounded by two envelopes from that same chain. A `LocalPerStore` store can only be
/// placed by its records' own timestamps, so one record without a timestamp makes the
/// whole store unplaceable for that window — the count of records inside it is not
/// knowable, and a cursor comparison against an unknown count would be invented evidence.
///
/// This switches on `space`, never on `StoreKind`: the kind is what a participant wrote in
/// a descriptor.
fn unplaceable_records(store: &Leg1Store) -> usize {
    match store.space {
        SeqSpace::EntryChain => 0,
        SeqSpace::LocalPerStore => store
            .records
            .iter()
            .filter(|r| r.recorded_at.is_none())
            .count(),
    }
}

/// The records of one store that fall strictly inside the window, in the only sequence
/// space that store's records live in. Sequence spaces are never pooled: a `LocalPerStore`
/// store's `seq` is the verifier's own per-store counter and is not comparable with an
/// envelope's.
///
/// On an `EntryChain` store the record's **own harness timestamp wins where it has one**,
/// and the chain sequence is the fallback for a record that carries no time at all. That
/// order is what D9's dirty gap is about. A record's position in the chain is a fact about
/// when the daemon got to it and in what order the exporter wrote it down, not about when
/// the harness wrote the record: the adversarial fixture chains all 165 of its leg-1
/// records first and the daemon's own eight heartbeats after them, so every record in it —
/// including the 40 written during the hole — sits below both heartbeats by `seq`, and
/// nothing at all lies between them in sequence. Placing records by sequence alone would
/// therefore report the one window the daemon could not see as empty, which is exactly the
/// window the participant chose to work in; it is the harness's own clock that puts those
/// 40 inside it. A record with no harness timestamp cannot be placed that way, and for it
/// the chain's own order is the weaker statement that remains available: it lies between
/// the two heartbeat envelopes or it does not.
fn records_in_window(store: &Leg1Store, a: &Beat, b: &Beat) -> Vec<GapLine> {
    let inside_by_time = |t: f64| -> bool { t > a.secs as f64 && t < b.secs as f64 };
    store
        .records
        .iter()
        .filter(|r| match store.space {
            SeqSpace::EntryChain => match crate::legs::record_time(&r.value) {
                Some(t) => inside_by_time(t),
                None => r.seq > a.seq && r.seq < b.seq,
            },
            SeqSpace::LocalPerStore => r.recorded_at.is_some_and(inside_by_time),
        })
        .map(gap_line)
        .collect()
}

/// Walk the proof-of-life series and classify every interval.
///
/// The heartbeat interval `N` and each store's `partition_key` are read from `manifest`:
/// the classifier needs the second to tell an honest partition-filtered cursor from a
/// cursor running ahead of the chain.
///
/// A hole longer than `2N` is a gap — two intervals, so one missed tick under load is not
/// an incident. An interval that is *not* a hole is still examined, because the daemon's
/// own cursors can contradict the chain inside a perfectly punctual series.
///
/// `verified` is `AckReconciliation::verified` — `verify_acks`'s own allowlist — and the
/// highest anchored sequence is read only from the acknowledgements whose index appears in
/// it. This is an allowlist, not a blocklist of known problems: "verified" means
/// signature-checked against a pinned `server_public_key`, so an acknowledgement with no
/// key pinned is never in this list even when its `(seq, hash)` reconciles perfectly
/// against the chain — that reconciling hash is exactly the self-consistent evidence the
/// participant can produce alone, and trusting it as an anchor would recreate the hole the
/// whole check exists to close. With `verified` empty, `high` is `0` and every record in
/// every store reads as `unanchored_records`.
///
/// A caller with nothing to say about verification passes an explicit index set of its own
/// choosing — `&[]` trusts nothing, `&[0, 1, ..]` trusts every supplied `ack`. This
/// module's own tests mostly do the latter: their acknowledgements are hand-built anchor
/// markers for heartbeat/hole logic, not real chain evidence, so there is nothing here for
/// them to verify against.
pub fn classify(
    stores: &[Leg1Store],
    acks: &[AckCheckpoint],
    events: &[Envelope],
    manifest: &ConnectorManifest,
    verified: &[usize],
) -> Vec<Gap> {
    let high = acks
        .iter()
        .enumerate()
        .filter(|(i, _)| verified.contains(i))
        .map(|(_, a)| a.seq)
        .max()
        .unwrap_or(0);
    let window = manifest
        .heartbeat_seconds
        .unwrap_or(60)
        .saturating_mul(2)
        .max(1);
    let partitioned = |name: &str| {
        manifest
            .store
            .iter()
            .find(|s| s.name == name)
            .is_some_and(|s| s.partition_key.is_some())
    };
    let mut gaps = Vec::new();

    // 1. Every consecutive pair in the proof-of-life series, in chain order.
    let mut beats: Vec<Beat> = events.iter().filter_map(beat_of).collect();
    beats.sort_by_key(|b| b.seq);
    for pair in beats.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        let mut reasons = Vec::new();

        let backstep = a.secs.saturating_sub(b.secs);
        if backstep > 0 {
            reasons.push(GapReason::HeartbeatsOutOfOrder { seconds: backstep });
        }
        // Any backstep makes this a hole. The forward difference saturates to zero when the
        // clock runs backwards, so letting the grace decide `is_hole` would switch the
        // whole hole machinery — `records_appended`, `unanchored`, `unplaceable` — off for
        // the price of a two-second rewind. The grace softens the severity of
        // `HeartbeatsOutOfOrder` itself and nothing else.
        let is_hole = b.secs.saturating_sub(a.secs) > window || backstep > 0;

        let mut appended: Vec<GapLine> = Vec::new();
        for store in stores {
            let placed = records_in_window(store, a, b);
            let unplaceable = unplaceable_records(store);
            let cursors = (
                cursor_for(&a.kind, &store.name),
                cursor_for(&b.kind, &store.name),
            );
            if unplaceable > 0 {
                // The count inside the window is unknown, so the cursor cannot be
                // reconciled against it — and skipping that silently is how one
                // timestampless record suppresses the corroboration it should have
                // triggered. Say so whenever a comparison was there to be made and had to
                // be dropped, and in any hole.
                if is_hole || matches!(cursors, (Some(_), Some(_))) {
                    reasons.push(GapReason::UnplaceableRecords {
                        store: store.name.clone(),
                        records: unplaceable,
                    });
                }
                appended.extend(placed);
                continue;
            }
            // The daemon's own corroboration, on every interval bounded by two heartbeats —
            // and only where the daemon's number counts the same thing the chain does. A
            // cursor whose unit is not `Records` produces no delta at all, so neither
            // direction of the comparison is reachable on evidence that never supported it.
            if let (Some(before), Some(after)) = cursors
                && let Some(moved) = cursor_delta(before, after)
            {
                if placed.len() as u64 > moved {
                    reasons.push(GapReason::CursorMismatch {
                        store: store.name.clone(),
                        records: placed.len(),
                        cursor_delta: Some(moved),
                    });
                } else if placed.is_empty() && moved > 0 && !partitioned(&store.name) {
                    // A partition-filtered store legitimately advances over rows it never
                    // ships; a store with no partition key has no such excuse.
                    reasons.push(GapReason::CursorAheadOfChain {
                        store: store.name.clone(),
                        cursor_delta: Some(moved),
                    });
                }
            }
            appended.extend(placed);
        }

        // A restart names the head it found: the chain position immediately below itself.
        if let BeatKind::Restart {
            head_seq,
            head_hash,
        } = b.kind
            && (head_seq != b.seq.saturating_sub(1) || head_hash != b.prev)
        {
            reasons.push(GapReason::RestartHeadMismatch {
                claimed_seq: head_seq,
                observed_seq: b.seq.saturating_sub(1),
            });
        }

        if is_hole && !appended.is_empty() {
            reasons.push(GapReason::RecordsAppended);
        }
        // Per-hole anchoring: quiet is only clean where someone was watching.
        if is_hole && b.seq > high {
            reasons.push(GapReason::Unanchored);
        }
        if !is_hole && reasons.is_empty() {
            continue;
        }
        if reasons.is_empty() {
            gaps.push(clean_gap(
                a.seq,
                b.seq,
                Some(a.ts.to_string()),
                Some(b.ts.to_string()),
            ));
            continue;
        }
        let dirty = reasons.iter().any(|r| r.severity() == Severity::Fail);
        gaps.push(Gap {
            kind: if dirty {
                GapKind::Dirty
            } else {
                GapKind::Clean
            },
            is_hole,
            from_seq: a.seq,
            to_seq: b.seq,
            from: Some(a.ts.to_string()),
            to: Some(b.ts.to_string()),
            appended,
            reasons,
        });
    }

    // 2. Records above the highest acknowledgement: unwatched by construction.
    //
    // Only when there *is* an anchor. With no acknowledgements at all, "above the last
    // ack" means "the entire chain", and calling that a gap would report every M0 fixture
    // as one enormous dirty window — which says nothing a reader can act on and buries the
    // real finding. The honest statement about an unanchored trace is V12's `no_anchor`,
    // and it is made once, there.
    //
    // The comparison here stays inside one store's sequence space, never pooling two: an
    // acknowledgement names a position in the chain it acknowledges, which is the same
    // chain `verify_acks` looks the record up in.
    if anchoring(acks, events) != Anchoring::Anchored {
        return gaps;
    }
    for store in stores {
        let appended: Vec<GapLine> = store
            .records
            .iter()
            .filter(|r| r.seq > high)
            .map(gap_line)
            .collect();
        if appended.is_empty() {
            continue;
        }
        gaps.push(Gap {
            kind: GapKind::Dirty,
            is_hole: false,
            // The window runs from the anchor — the last sequence the server witnessed —
            // to the head, so a reviewer reads "everything after this is unwitnessed"
            // rather than a range that silently starts one record late.
            from_seq: high,
            to_seq: appended.last().map(|l| l.seq).unwrap_or(high),
            from: None,
            to: None,
            appended,
            reasons: vec![GapReason::Unanchored],
        });
    }
    gaps
}

fn gap_line(r: &Leg1Record) -> GapLine {
    GapLine {
        store: r.store.clone(),
        seq: r.seq,
        sha256: r.hash.clone(),
    }
}

/// A gap with no appended lines and no reason to doubt it: the daemon was down, nothing
/// was written, and an acknowledgement covers the far edge. Called by `classify` for every
/// such hole — which is what makes downtime a positive statement rather than an absence.
pub fn clean_gap(from_seq: u64, to_seq: u64, from: Option<String>, to: Option<String>) -> Gap {
    Gap {
        kind: GapKind::Clean,
        is_hole: true,
        from_seq,
        to_seq,
        from,
        to,
        appended: Vec::new(),
        reasons: Vec::new(),
    }
}

/// One acknowledgement the chain does not corroborate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AckMismatch {
    pub seq: u64,
    /// Position of this acknowledgement in the `acks` slice `verify_acks` was given, so a
    /// caller — V12 — can look the exact `AckCheckpoint` back up without re-searching by
    /// `seq`, which is not a safe key: an acknowledgement can collide on `seq` with another
    /// store's record, or be replayed from another entry's stream entirely.
    pub ack_index: usize,
    /// The hash the server acknowledged.
    pub expected: String,
    /// The hash the chain actually holds at that sequence — `None` when no record exists
    /// there at all, which is the signature of a chain rebuilt shorter than it was, and
    /// also how a signature that does not verify (or an acknowledgement from another
    /// entry) is reported.
    pub observed: Option<String>,
}

/// What `verify_acks` establishes about an acknowledgement stream: the specific problems
/// (`mismatches`), and — a stricter, disjoint set — which acknowledgements this run can
/// stand behind as a genuine anchor (`verified`, by index into the `acks` slice given to
/// `verify_acks`).
///
/// The two are not complements. An acknowledgement whose `(seq, hash)` reconciles
/// perfectly, but with no `server_public_key` pinned to check its signature against, is in
/// **neither** list: it is not a mismatch (the chain does hold what it claims), but it is
/// not verified either (nothing here rules out the participant having written that hash
/// themselves — that is exactly the self-consistent evidence this reconciliation exists to
/// distrust). `classify` anchors on `verified` alone, never on "not a mismatch" — see its
/// own doc comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AckReconciliation {
    pub mismatches: Vec<AckMismatch>,
    /// Indices into the `acks` slice `verify_acks` was given. Ascending, one entry per
    /// acknowledgement that passed every gate below, signature included.
    pub verified: Vec<usize>,
}

/// The reconciliation of an empty acknowledgement stream: nothing mismatched, nothing
/// verified — exactly what `verify_acks` returns for `acks: &[]` (asserted below).
///
/// A `static` rather than a `const` or a `Default` impl because it has to be borrowable
/// for `'static`: `CheckContext` holds a `&AckReconciliation`, and a check-level test that
/// builds its context inside a helper function has no local to borrow from. `&CONST` would
/// be a temporary there — `Vec` has a destructor, so it is never promoted — and the borrow
/// would not outlive the function.
pub static NO_ACKS: AckReconciliation = AckReconciliation {
    mismatches: Vec::new(),
    verified: Vec::new(),
};

/// Reconcile every acknowledgement against the trace it claims to belong to, the server's
/// signature where a key is pinned, and the chain.
///
/// This is the check that makes `VISION.md`'s sentence true. Without it, `AckCheckpoint`
/// carries a hash and a signature that nothing ever reads, and a participant who re-chains
/// the entry from seq 1 with the instructions removed produces a trace that recomputes
/// cleanly and reports no gaps.
///
/// Four gates. The first one an acknowledgement fails puts it in `mismatches` and ends its
/// evaluation; passing all four puts it in `verified`. Failing only the last of them —
/// no `server_public_key` pinned to check a signature against — puts it in **neither**:
///
///   1. **`entry_id`.** A validly signed acknowledgement is signed over `{entry_id, seq, hash,
///      acked_at}` — the server never promises it is *this* trace's acknowledgement, only that
///      it once acknowledged some entry's chain at some sequence. An acknowledgement genuinely
///      signed for a different entry and replayed into this one is rejected here, regardless
///      of whether its hash happens to reconcile against this chain — a coincidence is not an
///      anchor.
///   2. **The `(seq, hash)` pair, across every source of links this trace has**, not `seq`
///      alone against whichever store happens to be first. On a chain trace that source is
///      `VerifiedChain.envelopes`, and the hash is **recomputed** from
///      `Envelope::chain_bytes()` rather than read off the line. On a legacy trace it is
///      the `[[leg1]]` stores, whose records are chained independently, so more than one
///      can legitimately hold a record at the same `seq`; a collision there is harmless
///      because the hash disambiguates it — this gate passes when *some* record at that
///      sequence carries exactly the hash the server signed. A `seq` that exists in one or
///      more sources but matches none of their hashes, and a `seq` that exists in none at
///      all, are both reported as `ack_chain_mismatch` by V12; `observed` carries a
///      representative hash in the first case and `None` in the second (also how a wrong
///      entry or a bad signature are reported: there is no `observed` hash to cite that
///      means anything).
///
///      A chain-derived store is deliberately **not** searched here. Its records carry the
///      envelope's recorded hash and no bytes to recompute it from, so matching against
///      one would be taking the chain's word for its own integrity; the envelope beside it
///      is the same link with the bytes attached, and that is what is compared.
///   3. **A `server_public_key` is pinned at all.** Without one there is nothing to check
///      the signature against, so an acknowledgement whose hash reconciled at gate 2 stops
///      here — reported nowhere as a problem, but never added to `verified` either
///      ("verified" means signature-verified against a pinned key, and a hash a participant
///      could have copied off their own chain is not that).
///   4. **The signature**, against `server_public_key`. A signature that fails to verify
///      is `ack_chain_mismatch`, `observed: None` — the same shape as a missing record,
///      since neither means anything the participant did not fully control.
pub fn verify_acks(
    chain: Option<&VerifiedChain>,
    stores: &[Leg1Store],
    acks: &[AckCheckpoint],
    server_public_key: Option<&str>,
    entry_id: &str,
) -> AckReconciliation {
    let mut mismatches = Vec::new();
    let mut verified = Vec::new();
    for (ack_index, ack) in acks.iter().enumerate() {
        if ack.entry_id != entry_id {
            mismatches.push(AckMismatch {
                seq: ack.seq,
                ack_index,
                expected: ack.hash.clone(),
                observed: None,
            });
            continue;
        }
        // The entry chain first: on a daemon-produced trace an acknowledgement names an
        // envelope's seq, and the head it names is as likely to be a heartbeat as a record.
        // Reconciling against leg-1 records alone would make that a guaranteed false
        // `ack_chain_mismatch` on a clean run.
        let mut candidates: Vec<String> = chain
            .into_iter()
            .flat_map(|c| c.envelopes.iter())
            .filter(|e| e.link.seq == ack.seq)
            // Recomputed from the envelope's own bytes, never read off the line.
            .filter_map(|e| e.expected_hash(&e.link.prev).ok())
            .collect();
        // Then the legacy per-store path, the only path when `chain` is
        // `None`. Compare the recorded hash and — where the bytes are available —
        // recompute it, so an acknowledgement cannot be matched against a hash the store
        // merely claims.
        let recomputed = |r: &Leg1Record| -> String {
            if r.raw.is_empty() {
                r.hash.clone()
            } else {
                link_hash(&r.prev, &r.raw)
            }
        };
        candidates.extend(
            stores
                .iter()
                .filter(|s| s.space == SeqSpace::LocalPerStore)
                .flat_map(|s| s.records.iter())
                .filter(|r| r.seq == ack.seq && r.hash == ack.hash)
                .map(recomputed),
        );
        // A store record whose recorded hash already differs from the acknowledgement is
        // filtered above, so it cannot match here; it still has to be citable as what the
        // chain actually holds at that sequence.
        let observed = candidates.first().cloned().or_else(|| {
            stores
                .iter()
                .filter(|s| s.space == SeqSpace::LocalPerStore)
                .flat_map(|s| s.records.iter())
                .find(|r| r.seq == ack.seq)
                .map(recomputed)
        });
        if !candidates.contains(&ack.hash) {
            mismatches.push(AckMismatch {
                seq: ack.seq,
                ack_index,
                expected: ack.hash.clone(),
                observed,
            });
            continue;
        }
        let Some(key) = server_public_key else {
            // Hash reconciles, nothing to check the signature against: not a mismatch, and
            // not verified either.
            continue;
        };
        if ack.verify(key).is_err() {
            mismatches.push(AckMismatch {
                seq: ack.seq,
                ack_index,
                expected: ack.hash.clone(),
                observed: None,
            });
            continue;
        }
        verified.push(ack_index);
    }
    mismatches.sort_by_key(|m| m.seq);
    AckReconciliation {
        mismatches,
        verified,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::support;

    /// `NO_ACKS` is not a hand-written stand-in: it is what the function itself returns
    /// for an empty stream, so a check reading it reads the truth.
    #[test]
    fn no_acks_is_what_verify_acks_returns_for_an_empty_stream() {
        assert_eq!(verify_acks(None, &[], &[], None, "e"), NO_ACKS);
    }
    use goalcup_protocol::chain::Link;
    use goalcup_protocol::cursor::CursorUnit;

    /// One `Leg2Post` envelope, at chain `seq`, whose `receiver_meta` declares `source`
    /// and the receiver's own counter `receiver_seq` (`None` for an uncounted source).
    fn leg2_post(seq: u64, source: &str, receiver_seq: Option<u64>) -> Envelope {
        post(seq, source, receiver_seq, "UserPromptSubmit", None)
    }

    /// The resume witness this rule reads: a `SessionStart` post whose payload declares
    /// `source: "resume"`. Written as an ordinary post so the fixture proves the witness
    /// is found on the same envelope stream every other post travels on — including when
    /// the witness is the post whose counter restarted.
    fn resume_post(seq: u64, source: &str, receiver_seq: Option<u64>) -> Envelope {
        post(seq, source, receiver_seq, "SessionStart", Some("resume"))
    }

    fn post(
        seq: u64,
        source: &str,
        receiver_seq: Option<u64>,
        event: &str,
        payload_source: Option<&str>,
    ) -> Envelope {
        let mut payload = serde_json::json!({});
        if let Some(s) = payload_source {
            payload["source"] = serde_json::Value::String(s.to_string());
        }
        env(
            seq,
            &format!("{:064}", seq.wrapping_sub(1)),
            EventBody::Leg2Post {
                event: event.to_string(),
                payload,
                path: None,
                receiver_meta: Some(serde_json::json!({
                    "reported_config_hash": null,
                    "seq": receiver_seq,
                    "source": source,
                    "anomalies": [],
                })),
            },
        )
    }

    #[test]
    fn contiguous_per_source_seq_raises_no_gap() {
        let events = vec![
            leg2_post(0, "hooks-http", Some(1)),
            leg2_post(1, "hooks-http", Some(2)),
            leg2_post(2, "hooks-http", Some(3)),
        ];
        assert_eq!(receiver_seq_gaps(&events), Vec::new());
    }

    #[test]
    fn a_skipped_seq_is_one_gap_naming_expected_and_observed() {
        let events = vec![
            leg2_post(0, "hooks-http", Some(1)),
            leg2_post(1, "hooks-http", Some(2)),
            // 3 was never delivered.
            leg2_post(2, "hooks-http", Some(4)),
        ];
        let gaps = receiver_seq_gaps(&events);
        assert_eq!(
            gaps,
            vec![ReceiverSeqGap {
                source: "hooks-http".to_string(),
                expected: 3,
                observed: 4,
                seq: 2,
                sha256: events[2].link.hash.clone(),
            }]
        );
    }

    /// Replay reads as a hole too: the contract is "contiguous", and a value that is not
    /// strictly the next one is not contiguous whichever direction it moved.
    #[test]
    fn a_replayed_seq_is_also_a_gap() {
        let events = vec![
            leg2_post(0, "hooks-http", Some(1)),
            leg2_post(1, "hooks-http", Some(2)),
            leg2_post(2, "hooks-http", Some(2)),
        ];
        let gaps = receiver_seq_gaps(&events);
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].expected, 3);
        assert_eq!(gaps[0].observed, 2);
    }

    /// One hole does not turn every later post from that source into a second finding:
    /// the walk continues from what was actually declared.
    #[test]
    fn one_hole_does_not_cascade_into_the_rest_of_the_source() {
        let events = vec![
            leg2_post(0, "hooks-http", Some(1)),
            leg2_post(1, "hooks-http", Some(5)),
            leg2_post(2, "hooks-http", Some(6)),
        ];
        let gaps = receiver_seq_gaps(&events);
        assert_eq!(gaps.len(), 1, "{gaps:?}");
        assert_eq!(gaps[0].expected, 2);
        assert_eq!(gaps[0].observed, 5);
    }

    /// A source that never sends a counter (Claude Code's native `http` transport)
    /// participates in no finding at all, however many posts it sends.
    #[test]
    fn a_source_with_no_declared_seq_is_never_flagged() {
        let events = vec![
            leg2_post(0, "http", None),
            leg2_post(1, "http", None),
            leg2_post(2, "http", None),
        ];
        assert_eq!(receiver_seq_gaps(&events), Vec::new());
    }

    /// Two sources are tracked independently: a hole in one says nothing about the other,
    /// and an uncounted source beside a counted one raises nothing for its own posts.
    #[test]
    fn sources_are_tracked_independently() {
        let events = vec![
            leg2_post(0, "hooks-http", Some(1)),
            leg2_post(1, "http", None),
            leg2_post(2, "hooks-http", Some(3)), // skipped 2
            leg2_post(3, "http", None),
        ];
        let gaps = receiver_seq_gaps(&events);
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].source, "hooks-http");
        assert_eq!(gaps[0].expected, 2);
        assert_eq!(gaps[0].observed, 3);
    }

    /// The guard. A counter that restarts at 1 with no resume anywhere between the two
    /// posts is still a hole — this is the laundering hole the forgiveness must not open. A
    /// shim that drops posts 5..9 and then declares 1 gets no cover from a rule that
    /// forgives every reset.
    #[test]
    fn a_reset_to_one_with_no_resume_witness_is_still_a_gap() {
        let events = vec![
            leg2_post(0, "hooks-http", Some(1)),
            leg2_post(1, "hooks-http", Some(2)),
            // A SessionStart that is not a resume witnesses nothing.
            post(2, "hooks-http", None, "SessionStart", Some("startup")),
            leg2_post(3, "hooks-http", Some(1)),
        ];
        let gaps = receiver_seq_gaps(&events);
        assert_eq!(gaps.len(), 1, "{gaps:?}");
        assert_eq!(gaps[0].expected, 3);
        assert_eq!(gaps[0].observed, 1);
        assert_eq!(gaps[0].seq, 3);
    }

    /// The honest restart the forgiveness exists for: the harness relaunched, the chain
    /// carries the resume, and the per-source counter starting again at 1 is explained.
    #[test]
    fn a_reset_to_one_witnessed_by_a_resume_is_forgiven() {
        let events = vec![
            leg2_post(0, "hooks-http", Some(1)),
            leg2_post(1, "hooks-http", Some(2)),
            resume_post(2, "hooks-cmd", Some(1)),
            leg2_post(3, "hooks-http", Some(1)),
            leg2_post(4, "hooks-http", Some(2)),
        ];
        assert_eq!(receiver_seq_gaps(&events), Vec::new());
    }

    /// The inclusive upper bound, and the reason for it: on Claude Code's
    /// `project-settings` source the resume `SessionStart` is the seq-1 post. An exclusive
    /// bound would miss its own witness and flag every honest resume.
    #[test]
    fn the_resume_post_witnesses_its_own_reset() {
        let events = vec![
            leg2_post(0, "project-settings", Some(1)),
            leg2_post(1, "project-settings", Some(2)),
            resume_post(2, "project-settings", Some(1)),
        ];
        assert_eq!(receiver_seq_gaps(&events), Vec::new());
    }

    /// Partial loss after a resume still starts at k+1, and k+1 is not 1: a resume
    /// forgives exactly the value a fresh counter produces and nothing else.
    #[test]
    fn a_resume_does_not_forgive_a_reset_to_anything_but_one() {
        let events = vec![
            leg2_post(0, "hooks-http", Some(1)),
            leg2_post(1, "hooks-http", Some(2)),
            resume_post(2, "hooks-cmd", Some(1)),
            leg2_post(3, "hooks-http", Some(4)),
        ];
        let gaps = receiver_seq_gaps(&events);
        assert_eq!(gaps.len(), 1, "{gaps:?}");
        assert_eq!(gaps[0].expected, 3);
        assert_eq!(gaps[0].observed, 4);
    }

    /// A resume emitted after the reset launders nothing: the witness must sit in the
    /// interval between the two posts, so a late marker cannot explain an earlier drop.
    #[test]
    fn a_resume_below_the_reset_does_not_witness_it() {
        let events = vec![
            leg2_post(0, "hooks-http", Some(1)),
            leg2_post(1, "hooks-http", Some(2)),
            leg2_post(2, "hooks-http", Some(1)),
            resume_post(3, "hooks-cmd", Some(1)),
        ];
        let gaps = receiver_seq_gaps(&events);
        assert_eq!(gaps.len(), 1, "{gaps:?}");
        assert_eq!(gaps[0].observed, 1);
        assert_eq!(gaps[0].seq, 2);
    }

    /// A resume witnessed once does not stay spent for the rest of the trace's sources —
    /// but it also does not forgive a second reset that happens above it with no second
    /// resume of its own.
    #[test]
    fn one_resume_does_not_forgive_a_later_unwitnessed_reset() {
        let events = vec![
            leg2_post(0, "hooks-http", Some(1)),
            resume_post(1, "hooks-cmd", Some(1)),
            leg2_post(2, "hooks-http", Some(1)),
            leg2_post(3, "hooks-http", Some(2)),
            leg2_post(4, "hooks-http", Some(1)),
        ];
        let gaps = receiver_seq_gaps(&events);
        assert_eq!(gaps.len(), 1, "{gaps:?}");
        assert_eq!(gaps[0].expected, 3);
        assert_eq!(gaps[0].observed, 1);
        assert_eq!(gaps[0].seq, 4);
    }

    /// A `DaemonRestart` is not a witness. The shim's counter is a file under the artifact
    /// root (`connectors/codex-cli/files/hook-post.sh`) and survives the daemon dying, so a
    /// daemon restart cannot explain a counter that went back to 1.
    #[test]
    fn a_daemon_restart_is_not_a_resume_witness() {
        let events = vec![
            leg2_post(0, "hooks-http", Some(1)),
            leg2_post(1, "hooks-http", Some(2)),
            env(
                2,
                &"0".repeat(64),
                EventBody::DaemonRestart {
                    at: "2026-09-04T00:00:02Z".to_string(),
                    previous_head_seq: 1,
                    previous_head_hash: format!("{:064}", 1),
                },
            ),
            leg2_post(3, "hooks-http", Some(1)),
        ];
        let gaps = receiver_seq_gaps(&events);
        assert_eq!(gaps.len(), 1, "{gaps:?}");
        assert_eq!(gaps[0].expected, 3);
        assert_eq!(gaps[0].observed, 1);
    }

    /// The first value a source ever declares is never a finding, witness or not: there is
    /// no previous value for it to be contiguous with.
    #[test]
    fn a_first_declaration_of_one_is_not_a_gap_and_needs_no_witness() {
        let events = vec![
            leg2_post(0, "hooks-http", Some(1)),
            leg2_post(1, "hooks-http", Some(2)),
        ];
        assert_eq!(receiver_seq_gaps(&events), Vec::new());
    }

    /// A `receiver_meta` this rule cannot parse this far (a shape older or stranger than
    /// `{seq, source}`) is not this rule's to report — it is skipped rather than treated
    /// as a hole.
    #[test]
    fn an_unparseable_receiver_meta_is_skipped_rather_than_flagged() {
        let events = vec![env(
            0,
            &"0".repeat(64),
            EventBody::Leg2Post {
                event: "UserPromptSubmit".to_string(),
                payload: serde_json::json!({}),
                path: None,
                receiver_meta: Some(serde_json::json!("not an object")),
            },
        )];
        assert_eq!(receiver_seq_gaps(&events), Vec::new());
    }

    fn env(seq: u64, prev: &str, body: EventBody) -> Envelope {
        Envelope {
            protocol_version: 1,
            entry_id: "e".to_string(),
            source: "daemon".to_string(),
            recorded_at: "2026-09-04T00:00:00Z".to_string(),
            link: Link {
                seq,
                prev: prev.to_string(),
                hash: format!("{seq:064}"),
            },
            body,
            redaction: None,
        }
    }

    /// A heartbeat carrying one typed cursor per store. `describe` is derived from the
    /// unit and the position, so the human string is present in every fixture below and
    /// read by nothing.
    fn beat(seq: u64, ts: &str, cursors: &[(&str, CursorUnit, u64)]) -> Envelope {
        env(
            seq,
            &"0".repeat(64),
            EventBody::Heartbeat {
                ts: ts.to_string(),
                stores_seen: cursors
                    .iter()
                    .map(|(store, unit, position)| StoreCursor {
                        store: store.to_string(),
                        unit: *unit,
                        position: Some(*position),
                        describe: format!("{unit:?}:{position}"),
                    })
                    .collect(),
                receiver_ok: true,
            },
        )
    }

    /// Six hours of silence between two chain positions.
    fn hole(
        cursors_before: &[(&str, CursorUnit, u64)],
        cursors_after: &[(&str, CursorUnit, u64)],
    ) -> Vec<Envelope> {
        vec![
            beat(10, "2026-09-04T08:00:00Z", cursors_before),
            beat(40, "2026-09-04T14:00:00Z", cursors_after),
        ]
    }

    /// One punctual minute between two chain positions.
    fn punctual(
        cursors_before: &[(&str, CursorUnit, u64)],
        cursors_after: &[(&str, CursorUnit, u64)],
    ) -> Vec<Envelope> {
        vec![
            beat(10, "2026-09-04T08:00:00Z", cursors_before),
            beat(40, "2026-09-04T08:01:00Z", cursors_after),
        ]
    }

    /// A store in the legacy per-store sequence space, built from records the test wrote
    /// itself. This is the space `verify_acks`'s per-store path measures.
    fn local_store(name: &str, records: Vec<Leg1Record>) -> Leg1Store {
        Leg1Store::from_records(
            name,
            crate::trace::StoreKind::RawJsonl,
            SeqSpace::LocalPerStore,
            records,
        )
    }

    fn store(name: &str, space: SeqSpace, records: &[(u64, Option<&str>)]) -> Leg1Store {
        Leg1Store::from_records(
            name,
            crate::trace::StoreKind::RawJsonl,
            space,
            records
                .iter()
                .map(|(seq, at)| Leg1Record {
                    store: name.to_string(),
                    seq: *seq,
                    prev: "0".repeat(64),
                    hash: format!("{seq:064}"),
                    recorded_at: at.map(|t| parse_z(t).expect("test timestamp") as f64),
                    value: serde_json::json!({"cwd": "/Users/real/ws"}),
                    raw: Vec::new(),
                })
                .collect(),
        )
    }

    fn ack(seq: u64) -> AckCheckpoint {
        AckCheckpoint {
            entry_id: "e".to_string(),
            seq,
            hash: "a".repeat(64),
            acked_at: "2026-09-04T00:00:00Z".to_string(),
            server_signature: String::new(),
        }
    }

    /// A `ChainJsonl` store's sequences are the daemon's, so a record with no timestamp of
    /// its own is still placed exactly.
    #[test]
    fn a_chain_store_places_records_by_chain_sequence() {
        let m = support::manifest("claude-code-cli");
        let inside = store("main", SeqSpace::EntryChain, &[(20, None), (30, None)]);
        let gaps = classify(&[inside], &[ack(100)], &hole(&[], &[]), &m, &[0]);
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].kind, GapKind::Dirty);
        assert_eq!(gaps[0].appended.len(), 2);
        assert!(gaps[0].reasons.contains(&GapReason::RecordsAppended));

        let outside = store("main", SeqSpace::EntryChain, &[(1, None), (99, None)]);
        let gaps = classify(&[outside], &[ack(100)], &hole(&[], &[]), &m, &[0]);
        assert_eq!(gaps[0].kind, GapKind::Clean);
        assert!(gaps[0].is_hole);
    }

    /// A chain-derived store whose records carry the harness's own `timestamp` field, and
    /// a daemon observation (`recorded_at`) that deliberately disagrees with it — which is
    /// what a catch-up after a restart looks like on disk.
    fn catch_up_store(records: &[(u64, Option<&str>, &str)]) -> Leg1Store {
        Leg1Store::from_records(
            "main",
            crate::trace::StoreKind::ChainJsonl,
            SeqSpace::EntryChain,
            records
                .iter()
                .map(|(seq, harness_at, observed_at)| Leg1Record {
                    store: "main".to_string(),
                    seq: *seq,
                    prev: "0".repeat(64),
                    hash: format!("{seq:064}"),
                    // The daemon's own clock, which is not what places the record.
                    recorded_at: Some(parse_z(observed_at).expect("test timestamp") as f64),
                    value: match harness_at {
                        Some(t) => serde_json::json!({"cwd": "/Users/real/ws", "timestamp": t}),
                        None => serde_json::json!({"cwd": "/Users/real/ws"}),
                    },
                    raw: Vec::new(),
                })
                .collect(),
        )
    }

    /// D9's dirty gap, on the trace shape that actually produces one: the daemon was dead
    /// for six hours, so it appended nothing during them, and the records it caught up on
    /// when it came back sit above the resuming heartbeat by `seq` while carrying the
    /// harness's own timestamps from inside the hole.
    ///
    /// Placing an entry-chain record by `seq` alone reports that window as empty — the one
    /// window nobody was watching, and the one the participant would choose to work in.
    #[test]
    fn a_chain_record_with_a_harness_timestamp_is_placed_by_that_timestamp() {
        let m = support::manifest("claude-code-cli");
        // Chained after the 14:00 beat (seq 40), written by the harness at 10:00 and
        // 12:00 — inside the 08:00–14:00 hole.
        let caught_up = catch_up_store(&[
            (50, Some("2026-09-04T10:00:00Z"), "2026-09-04T14:00:05Z"),
            (51, Some("2026-09-04T12:00:00Z"), "2026-09-04T14:00:06Z"),
        ]);
        let gaps = classify(&[caught_up], &[ack(100)], &hole(&[], &[]), &m, &[0]);
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].kind, GapKind::Dirty);
        assert_eq!(gaps[0].appended.len(), 2, "{:?}", gaps[0].reasons);
        assert!(gaps[0].reasons.contains(&GapReason::RecordsAppended));

        // And the other direction, which is what makes this a placement rule rather than
        // a way of finding more records: sequences squarely inside the window, harness
        // timestamps outside it. The harness's clock decides, so the hole is clean.
        let elsewhere = catch_up_store(&[
            (20, Some("2026-09-04T07:00:00Z"), "2026-09-04T07:00:01Z"),
            (30, Some("2026-09-04T20:00:00Z"), "2026-09-04T20:00:01Z"),
        ]);
        let gaps = classify(&[elsewhere], &[ack(100)], &hole(&[], &[]), &m, &[0]);
        assert_eq!(gaps[0].kind, GapKind::Clean, "{:?}", gaps[0].reasons);
        assert!(gaps[0].appended.is_empty());
    }

    /// The fallback, and the reason it is a fallback rather than "unplaceable": a chain
    /// record with no harness timestamp still has the daemon's own order, and the two
    /// bounding envelopes are links in that same chain.
    #[test]
    fn a_chain_record_without_a_harness_timestamp_is_placed_by_chain_seq() {
        let m = support::manifest("claude-code-cli");
        let inside = catch_up_store(&[(20, None, "2026-09-04T20:00:00Z")]);
        let gaps = classify(&[inside], &[ack(100)], &hole(&[], &[]), &m, &[0]);
        assert_eq!(gaps[0].kind, GapKind::Dirty);
        assert_eq!(
            gaps[0].appended.len(),
            1,
            "between seq 10 and seq 40, whatever the daemon's clock says"
        );
        // Never `unplaceable_records`: that reason belongs to a store whose sequences mean
        // nothing outside itself, and this store's are the entry's own.
        assert!(
            !gaps[0]
                .reasons
                .iter()
                .any(|r| matches!(r, GapReason::UnplaceableRecords { .. }))
        );

        let outside = catch_up_store(&[(99, None, "2026-09-04T10:00:00Z")]);
        let gaps = classify(&[outside], &[ack(100)], &hole(&[], &[]), &m, &[0]);
        assert_eq!(gaps[0].kind, GapKind::Clean, "{:?}", gaps[0].reasons);
    }

    /// A `RawJsonl` store is chained per store, from 1, by the verifier. Those sequences
    /// collide across stores and mean nothing next to an envelope's, so placement is by the
    /// record's own timestamp — and only by that.
    #[test]
    fn a_raw_store_places_records_by_timestamp_and_never_by_its_local_sequence() {
        let m = support::manifest("claude-code-cli");
        let mixed = store(
            "main",
            SeqSpace::LocalPerStore,
            &[
                (20, Some("2026-09-04T10:00:00Z")),
                (30, Some("2026-09-04T20:00:00Z")),
            ],
        );
        let gaps = classify(&[mixed], &[ack(100)], &hole(&[], &[]), &m, &[0]);
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].appended.len(), 1, "only the one inside the window");
        assert_eq!(gaps[0].appended[0].seq, 20);

        // Sequences that sit squarely between the bounding envelopes, timestamps that do
        // not: pooling the two spaces would call this dirty.
        let elsewhere = store(
            "main",
            SeqSpace::LocalPerStore,
            &[
                (20, Some("2026-09-04T07:00:00Z")),
                (30, Some("2026-09-04T20:00:00Z")),
            ],
        );
        let gaps = classify(&[elsewhere], &[ack(100)], &hole(&[], &[]), &m, &[0]);
        assert_eq!(gaps[0].kind, GapKind::Clean, "{:?}", gaps[0].reasons);
    }

    /// A raw record with no timestamp cannot be placed either way, and an unplaceable
    /// record inside a hole is exactly what tamper looks like.
    #[test]
    fn a_raw_record_with_no_timestamp_makes_a_hole_dirty() {
        let m = support::manifest("claude-code-cli");
        let unplaceable = store(
            "main",
            SeqSpace::LocalPerStore,
            &[(20, None), (30, Some("2026-09-04T20:00:00Z"))],
        );
        let gaps = classify(&[unplaceable], &[ack(100)], &hole(&[], &[]), &m, &[0]);
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].kind, GapKind::Dirty);
        assert_eq!(
            gaps[0].reasons,
            vec![GapReason::UnplaceableRecords {
                store: "main".to_string(),
                records: 1,
            }]
        );
    }

    /// Quiet is only clean where someone was watching.
    #[test]
    fn a_quiet_hole_beyond_the_last_acknowledgement_is_dirty() {
        let m = support::manifest("claude-code-cli");
        let empty = || store("main", SeqSpace::EntryChain, &[]);
        let early = classify(&[empty()], &[ack(20)], &hole(&[], &[]), &m, &[0]);
        assert_eq!(early.len(), 1);
        assert_eq!(early[0].kind, GapKind::Dirty);
        assert_eq!(early[0].reasons, vec![GapReason::Unanchored]);
        let covered = classify(&[empty()], &[ack(40)], &hole(&[], &[]), &m, &[0]);
        assert_eq!(covered[0].kind, GapKind::Clean);
        assert!(covered[0].reasons.is_empty());
    }

    /// The Fail direction: the chain holds more than the daemon read.
    #[test]
    fn more_records_than_the_cursor_moved_over_is_a_cursor_mismatch() {
        let m = support::manifest("claude-code-cli");
        let records = store(
            "main",
            SeqSpace::EntryChain,
            &[(20, None), (25, None), (30, None)],
        );
        let events = punctual(
            &[("main", CursorUnit::Records, 10)],
            &[("main", CursorUnit::Records, 11)],
        );
        let gaps = classify(&[records], &[ack(100)], &events, &m, &[0]);
        assert_eq!(
            gaps.len(),
            1,
            "punctual, but the daemon never saw two of them"
        );
        assert!(!gaps[0].is_hole);
        assert_eq!(gaps[0].kind, GapKind::Dirty);
        assert_eq!(
            gaps[0].reasons,
            vec![GapReason::CursorMismatch {
                store: "main".to_string(),
                records: 3,
                cursor_delta: Some(1),
            }]
        );
        assert_eq!(gaps[0].reasons[0].code(), Some("cursor_mismatch"));
        assert_eq!(gaps[0].reasons[0].severity(), Severity::Fail);
    }

    /// The silent direction: a partition-filtered store legitimately advances its cursor
    /// over rows it never ships.
    #[test]
    fn a_cursor_ahead_of_the_chain_is_silent_for_a_partitioned_store() {
        let m = support::manifest("codex-cli");
        assert!(
            m.store
                .iter()
                .find(|s| s.name == "rollout")
                .unwrap()
                .partition_key
                .is_some()
        );
        let events = punctual(
            &[("rollout", CursorUnit::Records, 10)],
            &[("rollout", CursorUnit::Records, 99)],
        );
        let empty = store("rollout", SeqSpace::EntryChain, &[]);
        assert!(classify(&[empty], &[ack(100)], &events, &m, &[0]).is_empty());
    }

    /// …and the Flag direction: a store with no partition key has no such excuse.
    #[test]
    fn a_cursor_ahead_of_the_chain_is_flagged_for_an_unpartitioned_store() {
        let m = support::manifest("claude-code-cli");
        assert!(
            m.store
                .iter()
                .find(|s| s.name == "main")
                .unwrap()
                .partition_key
                .is_none()
        );
        let events = punctual(
            &[("main", CursorUnit::Records, 0)],
            &[("main", CursorUnit::Records, 4096)],
        );
        let empty = store("main", SeqSpace::EntryChain, &[]);
        let gaps = classify(&[empty], &[ack(100)], &events, &m, &[0]);
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].kind, GapKind::Clean, "a question, not a verdict");
        assert_eq!(
            gaps[0].reasons,
            vec![GapReason::CursorAheadOfChain {
                store: "main".to_string(),
                cursor_delta: Some(4096),
            }]
        );
        assert_eq!(gaps[0].reasons[0].severity(), Severity::Flag);
    }

    /// An honest window — cursor moved, records arrived — is not a gap at all.
    #[test]
    fn a_cursor_that_kept_up_with_the_chain_says_nothing() {
        let m = support::manifest("claude-code-cli");
        let events = punctual(
            &[("main", CursorUnit::Records, 0)],
            &[("main", CursorUnit::Records, 900)],
        );
        let records = store("main", SeqSpace::EntryChain, &[(20, None), (25, None)]);
        assert!(classify(&[records], &[ack(100)], &events, &m, &[0]).is_empty());
    }

    /// A restart must name the chain position immediately below it.
    #[test]
    fn a_restart_naming_a_head_the_chain_does_not_hold_is_a_dirty_gap() {
        let m = support::manifest("claude-code-cli");
        let restart = |head_seq: u64, head_hash: String| {
            env(
                21,
                &format!("{:064}", 20),
                EventBody::DaemonRestart {
                    at: "2026-09-04T08:01:00Z".to_string(),
                    previous_head_seq: head_seq,
                    previous_head_hash: head_hash,
                },
            )
        };
        let empty = || store("main", SeqSpace::EntryChain, &[]);
        let honest = vec![
            beat(20, "2026-09-04T08:00:00Z", &[]),
            restart(20, format!("{:064}", 20)),
        ];
        assert!(classify(&[empty()], &[ack(50)], &honest, &m, &[0]).is_empty());

        let lying = vec![
            beat(20, "2026-09-04T08:00:00Z", &[]),
            restart(7, "f".repeat(64)),
        ];
        let gaps = classify(&[empty()], &[ack(50)], &lying, &m, &[0]);
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].kind, GapKind::Dirty);
        assert_eq!(
            gaps[0].reasons,
            vec![GapReason::RestartHeadMismatch {
                claimed_seq: 7,
                observed_seq: 20,
            }]
        );
    }

    /// A beat whose timestamp cannot be read is reported, not dropped.
    #[test]
    fn an_unparseable_heartbeat_is_reported() {
        let m = support::manifest("claude-code-cli");
        let events = vec![
            beat(10, "2026-09-04T08:00:00Z", &[]),
            beat(11, "yesterday afternoon", &[]),
        ];
        let notices = unparseable_heartbeats(&events);
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].seq, 11);
        assert_eq!(notices[0].sha256.len(), 64);
        // It also cannot bound a hole, which is exactly why it has to be reported.
        let empty = store("main", SeqSpace::EntryChain, &[]);
        assert!(classify(&[empty], &[ack(50)], &events, &m, &[0]).is_empty());
    }

    /// The grace softens the severity of a small backward clock step and nothing else. The
    /// window stays a hole, because the forward difference saturates to zero when the clock
    /// runs backwards and letting the grace clear `is_hole` would switch off
    /// `records_appended` and `unanchored` for the price of a two-second rewind.
    #[test]
    fn a_small_clock_correction_is_a_flag_but_still_bounds_a_hole() {
        let m = support::manifest("claude-code-cli");
        let empty = || store("main", SeqSpace::EntryChain, &[]);
        let nudged = vec![
            beat(10, "2026-09-04T08:00:04Z", &[]),
            beat(11, "2026-09-04T08:00:00Z", &[]),
        ];
        let gaps = classify(&[empty()], &[ack(50)], &nudged, &m, &[0]);
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].kind, GapKind::Clean, "nothing happened in it");
        assert!(gaps[0].is_hole, "the hole machinery stays on");
        assert_eq!(
            gaps[0].reasons,
            vec![GapReason::HeartbeatsOutOfOrder { seconds: 4 }]
        );
        assert_eq!(gaps[0].reasons[0].severity(), Severity::Flag);

        let rewound = vec![
            beat(10, "2026-09-04T09:00:00Z", &[]),
            beat(11, "2026-09-04T08:00:00Z", &[]),
        ];
        let gaps = classify(&[empty()], &[ack(50)], &rewound, &m, &[0]);
        assert_eq!(gaps[0].kind, GapKind::Dirty);
        assert!(gaps[0].is_hole);
        assert_eq!(gaps[0].reasons[0].severity(), Severity::Fail);
    }

    /// Two seconds of rewind must not buy a free window.
    #[test]
    fn a_two_second_rewind_with_records_in_it_is_a_dirty_gap_that_lists_them() {
        let m = support::manifest("claude-code-cli");
        let events = vec![
            beat(10, "2026-09-04T08:00:02Z", &[]),
            beat(40, "2026-09-04T08:00:00Z", &[]),
        ];
        let records = store("main", SeqSpace::EntryChain, &[(20, None), (30, None)]);
        let gaps = classify(&[records], &[ack(100)], &events, &m, &[0]);
        assert_eq!(gaps.len(), 1);
        assert!(gaps[0].is_hole);
        assert_eq!(gaps[0].kind, GapKind::Dirty);
        assert_eq!(gaps[0].appended.len(), 2, "the records are listed");
        assert!(gaps[0].reasons.contains(&GapReason::RecordsAppended));
        assert!(
            gaps[0]
                .reasons
                .contains(&GapReason::HeartbeatsOutOfOrder { seconds: 2 })
        );
    }

    /// An unplaceable record must not suppress the cursor check by making the whole store
    /// skip it in silence.
    #[test]
    fn an_unplaceable_record_says_so_even_in_a_punctual_window() {
        let m = support::manifest("claude-code-cli");
        let raw = store(
            "main",
            SeqSpace::LocalPerStore,
            &[(20, None), (30, Some("2026-09-04T08:00:30Z"))],
        );
        let events = punctual(
            &[("main", CursorUnit::Records, 0)],
            &[("main", CursorUnit::Records, 4096)],
        );
        let gaps = classify(&[raw], &[ack(100)], &events, &m, &[0]);
        assert_eq!(
            gaps.len(),
            1,
            "the skipped comparison is stated, not hidden"
        );
        assert!(!gaps[0].is_hole);
        assert_eq!(gaps[0].kind, GapKind::Dirty);
        assert_eq!(
            gaps[0].reasons,
            vec![GapReason::UnplaceableRecords {
                store: "main".to_string(),
                records: 1,
            }]
        );
        assert_eq!(gaps[0].reasons[0].code(), Some("unplaceable_records"));

        // With no cursor to compare and no hole, there is nothing to say.
        let raw = store("main", SeqSpace::LocalPerStore, &[(20, None)]);
        let events = punctual(&[], &[]);
        assert!(classify(&[raw], &[ack(100)], &events, &m, &[0]).is_empty());
    }

    /// The reason that carries the gap above the anchor has a code of its own, or V4
    /// renders it nowhere.
    #[test]
    fn records_above_the_anchor_carry_a_finding_code() {
        assert_eq!(GapReason::Unanchored.code(), Some("unanchored_records"));
        assert_eq!(GapReason::Unanchored.severity(), Severity::Fail);
        let m = support::manifest("claude-code-cli");
        let records = store("main", SeqSpace::EntryChain, &[(1, None), (2, None)]);
        let gaps = classify(&[records], &[ack(1)], &[], &m, &[0]);
        assert_eq!(gaps.len(), 1);
        assert!(!gaps[0].is_hole);
        assert_eq!(gaps[0].kind, GapKind::Dirty);
        assert_eq!(gaps[0].reasons, vec![GapReason::Unanchored]);
    }

    /// The M0 shape: no events, no acknowledgements. No gap is manufactured.
    #[test]
    fn no_anchors_produces_no_gap_at_all() {
        let m = support::manifest("claude-code-cli");
        assert_eq!(anchoring(&[], &[]), Anchoring::NoAnchors);
        let records = store("main", SeqSpace::LocalPerStore, &[(1, None), (2, None)]);
        assert!(classify(&[records], &[], &[], &m, &[]).is_empty());
    }

    /// `cursor_delta` subtracts records, and only records.
    #[test]
    fn a_cursor_delta_is_a_count_of_records_or_it_is_nothing() {
        let at = |unit, position| StoreCursor {
            store: "main".to_string(),
            unit,
            position: Some(position),
            describe: "unread".to_string(),
        };
        assert_eq!(
            cursor_delta(&at(CursorUnit::Records, 1), &at(CursorUnit::Records, 1)),
            Some(0)
        );
        assert_eq!(
            cursor_delta(&at(CursorUnit::Records, 1), &at(CursorUnit::Records, 9)),
            Some(8)
        );
        // Backwards: no movement, so records in the window are reported against it.
        assert_eq!(
            cursor_delta(&at(CursorUnit::Records, 9), &at(CursorUnit::Records, 1)),
            Some(0)
        );
        // Bytes, seconds and store-local keys are numbers with nothing to subtract them
        // from, in either order and however far they moved.
        for unit in [CursorUnit::Bytes, CursorUnit::Time, CursorUnit::Opaque] {
            assert_eq!(
                cursor_delta(&at(unit, 0), &at(unit, 4096)),
                None,
                "{unit:?}"
            );
            assert_eq!(
                cursor_delta(&at(CursorUnit::Records, 0), &at(unit, 4096)),
                None,
                "one comparable side is not a comparison: {unit:?}"
            );
        }
        // A `Records` cursor that names no number is no number.
        let numberless = StoreCursor {
            store: "main".to_string(),
            unit: CursorUnit::Records,
            position: None,
            describe: "no rows yet".to_string(),
        };
        assert_eq!(cursor_delta(&numberless, &at(CursorUnit::Records, 9)), None);
    }

    /// An acknowledgement resolves by its `(seq, hash)` **pair**, across every store — not
    /// by `seq` alone against whichever store happens to be checked first. Leg-1 stores are
    /// chained independently, so two of them legitimately holding a record at the same
    /// `seq` is ordinary, and the hash disambiguates the collision. Here the first store's
    /// record at seq 20 carries the wrong hash and the second's carries the right one; a
    /// first-store, first-match lookup would report this as a mismatch.
    #[test]
    fn an_ack_verifies_against_a_seq_that_collides_across_stores_when_only_the_second_matches() {
        let wrong_hash = "a".repeat(64);
        let right_hash = "b".repeat(64);
        let record_at = |store_name: &str, hash: &str| Leg1Record {
            store: store_name.to_string(),
            seq: 20,
            prev: "0".repeat(64),
            hash: hash.to_string(),
            recorded_at: None,
            value: serde_json::json!({}),
            raw: Vec::new(),
        };
        let first = local_store("main", vec![record_at("main", &wrong_hash)]);
        let second = local_store("hooks", vec![record_at("hooks", &right_hash)]);
        let ack = AckCheckpoint {
            entry_id: "e".to_string(),
            seq: 20,
            hash: right_hash.clone(),
            acked_at: "2026-09-04T00:00:00Z".to_string(),
            server_signature: String::new(),
        };
        assert!(
            verify_acks(
                None,
                &[first.clone(), second.clone()],
                std::slice::from_ref(&ack),
                None,
                "e"
            )
            .mismatches
            .is_empty(),
            "the second store's record at the colliding seq carries the acknowledged hash"
        );
        // Order must not matter: the match is found regardless of which store comes first.
        assert!(
            verify_acks(
                None,
                &[second, first],
                std::slice::from_ref(&ack),
                None,
                "e"
            )
            .mismatches
            .is_empty(),
            "resolution is not 'whichever store is first'"
        );

        // Neither store matches: reported once, citing a representative observed hash.
        let neither = AckCheckpoint {
            hash: "c".repeat(64),
            ..ack
        };
        let bad = verify_acks(
            None,
            &[
                local_store("main", vec![record_at("main", &wrong_hash)]),
                local_store("hooks", vec![record_at("hooks", &right_hash)]),
            ],
            &[neither],
            None,
            "e",
        );
        assert_eq!(bad.mismatches.len(), 1);
        assert_eq!(bad.mismatches[0].seq, 20);
        assert!(bad.mismatches[0].observed.is_some());
    }

    /// `AckCheckpoint::signing_bytes` includes `entry_id`, but a valid signature only
    /// promises the server once acknowledged some entry's chain at that sequence — never
    /// that it is this entry's. An acknowledgement whose `entry_id` does not match the
    /// trace's is a mismatch regardless of whether its `(seq, hash)` pair would otherwise
    /// reconcile.
    #[test]
    fn an_ack_from_the_wrong_entry_is_a_mismatch_even_when_the_hash_matches_exactly() {
        let hash = "d".repeat(64);
        let store = local_store(
            "main",
            vec![Leg1Record {
                store: "main".to_string(),
                seq: 5,
                prev: "0".repeat(64),
                hash: hash.clone(),
                recorded_at: None,
                value: serde_json::json!({}),
                raw: Vec::new(),
            }],
        );
        let wrong_entry = AckCheckpoint {
            entry_id: "another-entry".to_string(),
            seq: 5,
            hash,
            acked_at: "2026-09-04T00:00:00Z".to_string(),
            server_signature: String::new(),
        };
        let reconciliation = verify_acks(None, &[store], &[wrong_entry], None, "this-entry");
        assert_eq!(
            reconciliation.mismatches.len(),
            1,
            "the hash matches, the entry does not"
        );
        assert_eq!(reconciliation.mismatches[0].seq, 5);
        assert_eq!(reconciliation.mismatches[0].ack_index, 0);
        assert!(reconciliation.verified.is_empty());
    }
}
