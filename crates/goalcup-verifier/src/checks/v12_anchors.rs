//! V12: anchor reconciliation.
//!
//! Every other check in this crate reads evidence the participant produced on their own
//! machine. This one reads the single value they did not: the server's acknowledgement of
//! a chain prefix, signed by a key pinned outside their reach. `VISION.md` says an edit is
//! made visible because the two legs are hash-chained "and acknowledged by the server as
//! they happen" — this is the check that makes that sentence true rather than decorative.
//!
//! Note what this check does **not** rest on. The write-ahead buffer is encrypted with a
//! per-run key that lives on the participant's disk, because the daemon runs as the
//! participant (`age`, `RunKeys`). That is confidentiality against a third
//! party who takes the laptop; it is not integrity against the machine's owner. The
//! acknowledgements are the only integrity anchor there is.

use super::{Check, CheckContext, CheckResult, Evidence, fail, flag, pass};
use crate::chain::VerifiedChain;
use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::control::parse_z;
use goalcup_protocol::envelope::EventBody;

/// A first verified acknowledgement later than this many seconds after `started_at` is
/// a Flag: the evidence existed for a day before anyone outside the participant's
/// machine saw any of it.
pub const LATE_FIRST_ANCHOR_SECONDS: u64 = 86_400;
/// The same number as a span: a day with no verified acknowledgement in it — between two
/// of them, before the first, or after the last and before the run ended.
pub const MAX_UNANCHORED_SPAN_SECONDS: u64 = 86_400;

/// When the server saw this chain, read off the acknowledgements this run can stand
/// behind — and only those.
///
/// The whole point of the type: firewall the upload host for a week, fabricate the run,
/// connect an hour before the deadline, and every acknowledgement is genuine and every hash
/// reconciles. Without *when*, that reads clean. `acked_at` is inside the signature, so
/// there is a server-attested time to say it with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorTimeline {
    pub first_verified_at: Option<String>,
    pub last_verified_at: Option<String>,
    /// The longest run of wall-clock seconds with no verified acknowledgement: from
    /// `started_at` to the first when a receipt declared a start, between consecutive
    /// ones, and from the last to the end of the run when the trace says where that is.
    pub longest_unanchored_seconds: u64,
    /// The last span, stated separately because it is the one an hourly stream can still
    /// hide: anchor every hour for six days, stop, and seal on the seventh. Every span
    /// *between* anchors is an hour and the run is unwitnessed for a day. `None` when the
    /// trace says nothing about where it ended — a legacy trace with no chain and no seal
    /// — because a tail measured to a moment nobody declared is a number about nothing.
    pub tail_unanchored_seconds: Option<u64>,
    /// How many verified acknowledgements the timeline is actually built from: those with
    /// a readable `acked_at`. A verified acknowledgement whose timestamp does not parse
    /// contributes no anchor time, so counting it here would overstate the timeline. The
    /// headline count in `anchors_checked` is the reconciliation's own `verified.len()`
    /// and is reported separately; where the two differ, the check says so.
    pub verified: usize,
}

/// The timeline over `acks`, restricted to the indices in `verified`.
///
/// Two rules hold this together. **Only verified acknowledgements count.** One that
/// reconciles by hash but was never signature-checked — no pinned key — is not an anchor,
/// because a hash the participant read off their own chain is something they could have
/// written; `verify_acks` already puts such an acknowledgement in neither list, and this
/// reads `verified` alone. **Every time comes from the signed `acked_at`**, never from the
/// order the acknowledgements sit in a file: file order is the participant's to choose, so
/// the times are sorted here before any span is measured.
///
/// `ended_at` closes the timeline. Without it the measurement stopped at the last anchor,
/// which left the whole tail of a run unmeasured: anchoring hourly for six days and then
/// going dark until the seal produced a longest span of one hour. The end of a run is not
/// the participant's to assert — it is the seal link the chain carries — so the caller
/// resolves it (`run_end`) and passes `None` where the trace states nothing.
pub fn anchor_timeline(
    acks: &[AckCheckpoint],
    verified: &[usize],
    started_at: Option<u64>,
    ended_at: Option<u64>,
) -> AnchorTimeline {
    let mut times: Vec<(u64, &str)> = verified
        .iter()
        .filter_map(|&i| acks.get(i))
        .filter_map(|a| a.acked_at_secs().map(|secs| (secs, a.acked_at.as_str())))
        .collect();
    times.sort_unstable();

    // From the declared start to the first anchor, when a receipt declared one. Without a
    // receipt there is no attested beginning to measure from, and inventing one — the
    // first record's timestamp, say — would be measuring the participant's clock against
    // itself. So the span is then only what lies *between* acknowledgements.
    let mut longest = match (started_at, times.first()) {
        (Some(start), Some(&(first, _))) => first.saturating_sub(start),
        _ => 0,
    };
    for pair in times.windows(2) {
        longest = longest.max(pair[1].0.saturating_sub(pair[0].0));
    }
    // From the last anchor to the end of the run. `saturating_sub`, so a seal dated before
    // the acknowledgement that covers it reads as 0 rather than wrapping — that
    // disagreement is V13's to report, and it must not arrive here as a span of 500
    // million years.
    let tail = match (ended_at, times.last()) {
        (Some(end), Some(&(last, _))) => Some(end.saturating_sub(last)),
        _ => None,
    };
    longest = longest.max(tail.unwrap_or(0));

    AnchorTimeline {
        first_verified_at: times.first().map(|&(_, at)| at.to_string()),
        last_verified_at: times.last().map(|&(_, at)| at.to_string()),
        longest_unanchored_seconds: longest,
        tail_unanchored_seconds: tail,
        verified: times.len(),
    }
}

/// Where the run ends, for the tail span alone.
///
/// The order is not a preference, it is a question of who wrote the value. The chain's
/// `Seal` link is written by `TraceExport::write_seal` and covered by the link hash; the
/// `seal.json` behind `cx.seal` is a file at a participant-named path. Where both exist
/// the chained one wins, because a participant who wanted a short tail would edit the
/// file, not the chain. Where there is a chain and no seal link the run did not seal, and
/// the last link the chain carries is the last moment the daemon is on record for. Where
/// there is no chain at all — a legacy M0 trace — the declared seal is all there is, and
/// with no seal either the tail is not measured rather than guessed.
fn run_end(cx: &CheckContext) -> Option<u64> {
    match cx.chain {
        Some(chain) => chained_seal_at(chain)
            .and_then(|at| parse_z(&at))
            .or_else(|| chain.envelopes.last().and_then(|e| e.recorded_at_secs())),
        None => cx.seal.and_then(|s| parse_z(&s.sealed_at)),
    }
}

/// The `sealed_at` of the chain's own seal link, or `None` when it carries none.
fn chained_seal_at(chain: &VerifiedChain) -> Option<String> {
    let seq = chain.seal_link?;
    chain.envelopes.iter().find_map(|e| match &e.body {
        EventBody::Seal { sealed_at, .. } if e.link.seq == seq => Some(sealed_at.clone()),
        _ => None,
    })
}

/// The `anchor_timeline` finding's sentence. `reconciled` is the reconciliation's own
/// `verified.len()`, which is larger than `timeline.verified` exactly when a verified
/// acknowledgement carries an `acked_at` that does not parse — a difference the reader is
/// told about rather than left to infer from two numbers that never appear together.
fn timeline_sentence(timeline: &AnchorTimeline, reconciled: usize, has_key: bool) -> String {
    let unreadable = reconciled.saturating_sub(timeline.verified);
    let unplaced = if unreadable == 0 {
        String::new()
    } else {
        format!(
            "; {unreadable} verified acknowledgement(s) carry no readable time and are not \
             on this timeline"
        )
    };
    let tail = match timeline.tail_unanchored_seconds {
        Some(secs) => format!(", of which {secs}s is after the last one and before the run ended"),
        // Said rather than left out: a reader who does not know the tail was unmeasurable
        // would read the longest span as covering the whole run.
        None => ", and nothing here says where the run ended, so the span after the last \
                 acknowledgement is not measured"
            .to_string(),
    };
    match (&timeline.first_verified_at, &timeline.last_verified_at) {
        (Some(first), Some(last)) => format!(
            "the server saw this chain {} time(s) it can be held to: first at {first}, last \
             at {last}; the longest span with no verified acknowledgement is {}s{tail}{unplaced}",
            timeline.verified, timeline.longest_unanchored_seconds
        ),
        // The reason there is no timeline matters. With no recognised key nothing was
        // verified at all, so blaming unreadable timestamps would point a reader at the
        // acknowledgements when the thing missing is the key that would let anyone check
        // them.
        _ if reconciled == 0 && !has_key => "no pinned server key, so no acknowledgement was \
             verified and nothing here says when — or whether — a server saw any of this \
             chain"
            .to_string(),
        _ => format!(
            "no verified acknowledgement carries a readable time, so nothing here says \
             when the server saw this chain — only that some part of it reconciles{unplaced}"
        ),
    }
}

pub struct V12Anchors;

impl Check for V12Anchors {
    fn id(&self) -> &'static str {
        "V12"
    }
    fn title(&self) -> &'static str {
        "server acknowledgements reconciled against the chain"
    }
    fn run(&self, cx: &CheckContext) -> CheckResult {
        if cx.acks.is_empty() {
            return CheckResult::from_findings(
                self.id(),
                self.title(),
                vec![flag(
                    "no_anchor",
                    "this trace carries 0 acknowledgements, so no part of the chain is \
                     anchored to a value the participant did not produce; the chain \
                     recomputing cleanly says only that it is self-consistent"
                        .to_string(),
                    Vec::new(),
                )],
            );
        }

        // The resolved key, never `cx.trace.server_public_key`: the
        // descriptor is participant-writable, and `verify_trace` has already decided
        // whether this build recognises the key it names. When it does not, `server_key`
        // is `None` and the `anchor_unverified` flag below is exactly the right reading —
        // a signature checked against a key the participant chose is not checked.
        let key = cx.server_key;
        // Reconciled once, in `verify_trace`, and read here: a second `verify_acks` call
        // would be a second chance to disagree with the `verified` set `classify` anchored
        // on. V12 renders `mismatches` and the presence of a recognised key; `verified`
        // feeds `classify` and the anchor label below.
        let reconciliation = cx.acks_verified;
        let mismatches = &reconciliation.mismatches;
        let mut findings = Vec::new();

        for m in mismatches.iter() {
            // `ack_index` is a direct lookup, not a re-search by `seq` — which is not a
            // safe key: an acknowledgement can collide on `seq` with another store's
            // record, or be replayed from another entry's stream at the same sequence.
            let ack = &cx.acks[m.ack_index];

            // A validly signed acknowledgement for another entry,
            // replayed into this trace, is rejected here — before the signature and hash
            // checks below, and regardless of whether either would otherwise pass.
            if ack.entry_id != cx.trace.entry_id {
                findings.push(fail(
                    "ack_entry_mismatch",
                    format!(
                        "the acknowledgement at seq {} names entry `{}`, and this trace is \
                         entry `{}`; an acknowledgement from another entry cannot anchor \
                         this one, whether or not its hash happens to reconcile",
                        m.seq, ack.entry_id, cx.trace.entry_id
                    ),
                    vec![Evidence::at(
                        "acks",
                        m.seq,
                        None,
                        format!("entry `{}`, trace is `{}`", ack.entry_id, cx.trace.entry_id),
                    )],
                ));
                continue;
            }

            // `verify_acks` reports a failed signature the same way it reports a missing
            // record — `observed: None` — so distinguish the two here, where the key is
            // in hand, rather than widening the shared return type.
            let signature_bad = key.is_some_and(|k| ack.verify(k).is_err());
            if signature_bad {
                findings.push(fail(
                    "ack_signature_invalid",
                    format!(
                        "the acknowledgement at seq {} does not verify against the pinned \
                         server key; a hand-written acknowledgement stream is not an anchor",
                        m.seq
                    ),
                    vec![Evidence::at(
                        "acks",
                        m.seq,
                        Some(m.expected.clone()),
                        "signature",
                    )],
                ));
            } else {
                findings.push(fail(
                    "ack_chain_mismatch",
                    match &m.observed {
                        Some(o) => format!(
                            "the server acknowledged {} at seq {}, and the chain holds {o}; \
                             the chain was rewritten below an acknowledged position",
                            m.expected, m.seq
                        ),
                        None => format!(
                            "the server acknowledged {} at seq {}, and the chain has no \
                             record at that sequence; the chain was rebuilt shorter than it was",
                            m.expected, m.seq
                        ),
                    },
                    vec![Evidence::at(
                        "acks",
                        m.seq,
                        Some(m.expected.clone()),
                        m.observed
                            .clone()
                            .unwrap_or_else(|| "no record".to_string()),
                    )],
                ));
            }
        }

        // The honest count. `acks.len() - mismatches.len()` would count an acknowledgement
        // that reconciled by hash but could not be signature-checked — no pinned key — as
        // one the check stood behind, and `verify_acks` deliberately puts that
        // acknowledgement in neither list. `verified.len()` is the number this check can
        // defend, and the sentence says "verified" rather than "matched" so a reader is not
        // told a stronger thing than was established.
        let verified = reconciliation.verified.len();
        let unchecked = if key.is_none() {
            "; signatures unchecked"
        } else {
            ""
        };
        // "Anchored" is a claim about `verified`, not about every supplied ack — a
        // mismatched or unverifiable one must not be cited as having anchored anything.
        // With nothing verified there is no anchored sequence to report, so the label says
        // what it actually is: the highest sequence anyone *claimed*, verified or not.
        let (anchor_label, anchor_seq) = match reconciliation
            .verified
            .iter()
            .map(|&i| cx.acks[i].seq)
            .max()
        {
            Some(seq) => ("highest anchored sequence", seq),
            None => (
                "highest supplied sequence",
                cx.acks.iter().map(|a| a.seq).max().unwrap_or(0),
            ),
        };
        findings.push(pass(
            "anchors_checked",
            // "mismatches", not "mismatched": the word "matched" must not appear anywhere
            // in this sentence, and `mismatched` contains it.
            format!(
                "{} acknowledgement(s) reconciled, {verified} verified, {} mismatches\
                 {unchecked}; {anchor_label} {anchor_seq}",
                cx.acks.len(),
                mismatches.len(),
            ),
        ));

        // When the server saw the chain, not only that it did. `started_at`
        // comes from the receipt's claims — the one start statement the participant did
        // not sign — and is `None` on a trace that carries no receipt, which V13 reports
        // in its own right. There is deliberately no second source for it.
        let started_at = cx.receipt.and_then(|r| parse_z(&r.claims.started_at));
        let ended_at = run_end(cx);
        let timeline = anchor_timeline(cx.acks, &reconciliation.verified, started_at, ended_at);
        findings.push(pass(
            "anchor_timeline",
            timeline_sentence(&timeline, verified, key.is_some()),
        ));

        // Two triggers, two codes: a reader of the queue has to tell a run that started
        // unwitnessed from one that went dark in the middle or at the end, and
        // `late_first_anchor` would be a false statement about the second and third.
        let late_first = started_at
            .zip(timeline.first_verified_at.as_deref().and_then(parse_z))
            .filter(|&(start, first)| first > start.saturating_add(LATE_FIRST_ANCHOR_SECONDS));
        if let (Some((start, first)), Some(at)) = (late_first, &timeline.first_verified_at) {
            findings.push(flag(
                "late_first_anchor",
                format!(
                    "the first verified acknowledgement is {at}, {}s after the run's \
                     declared start and more than the {LATE_FIRST_ANCHOR_SECONDS}s this \
                     verifier will read as anchored-as-it-happened; for that window nothing \
                     outside the participant's own machine saw any of this evidence, and a \
                     run fabricated inside it is acknowledged exactly as one that happened",
                    first.saturating_sub(start)
                ),
                vec![Evidence {
                    store: "acks".to_string(),
                    // The sequence of the earliest verified acknowledgement by its signed
                    // time, not the lowest sequence: the two need not be the same record,
                    // and this finding is about time.
                    seq: reconciliation
                        .verified
                        .iter()
                        .filter_map(|&i| cx.acks.get(i))
                        .filter_map(|a| a.acked_at_secs().map(|secs| (secs, a.seq)))
                        .min()
                        .map(|(_, seq)| seq),
                    sha256: None,
                    note: format!("first verified acknowledgement at {at}"),
                }],
            ));
        }

        if timeline.longest_unanchored_seconds > MAX_UNANCHORED_SPAN_SECONDS {
            // Whether the longest run is the tail decides which record a reviewer should
            // look at, so the evidence names the last acknowledgement only when the tail
            // is what triggered — and stays silent about a sequence otherwise rather than
            // citing one the hole does not open at.
            let tail_is_longest =
                timeline.tail_unanchored_seconds == Some(timeline.longest_unanchored_seconds);
            let last_seq = reconciliation
                .verified
                .iter()
                .filter_map(|&i| cx.acks.get(i))
                .filter_map(|a| a.acked_at_secs().map(|secs| (secs, a.seq)))
                .max()
                .map(|(_, seq)| seq);
            findings.push(flag(
                "long_unanchored_span",
                format!(
                    "the longest run with no verified acknowledgement is {}s, longer than \
                     {MAX_UNANCHORED_SPAN_SECONDS}s{}; for that window nothing outside the \
                     participant's own machine saw any of this evidence, and a run \
                     fabricated inside it is acknowledged exactly as one that happened",
                    timeline.longest_unanchored_seconds,
                    if tail_is_longest {
                        ", and it is the stretch between the last acknowledgement and the \
                         end of the run"
                    } else {
                        ""
                    }
                ),
                vec![Evidence {
                    store: "acks".to_string(),
                    seq: if tail_is_longest { last_seq } else { None },
                    sha256: None,
                    note: match (&timeline.last_verified_at, tail_is_longest) {
                        (Some(at), true) => {
                            format!("last verified acknowledgement at {at}, then nothing")
                        }
                        _ => format!(
                            "longest unanchored run {}s",
                            timeline.longest_unanchored_seconds
                        ),
                    },
                }],
            ));
        }

        if key.is_none() {
            findings.push(flag(
                "anchor_unverified",
                "no recognised server key applies to this trace, so the acknowledgement \
                 signatures were carried and not checked; the hashes reconcile, the \
                 signer does not"
                    .to_string(),
                Vec::new(),
            ));
        }

        CheckResult::from_findings(self.id(), self.title(), findings)
    }
}
