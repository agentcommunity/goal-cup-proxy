//! The verifier's one reader of `chain.jsonl`.
//!
//! It checks the two things the envelope stream cannot be trusted without:
//!
//!   * **Link integrity.** `Envelope::expected_hash` is recomputed for every link: a
//!     participant who rewrites a body and leaves the recorded `hash` alone breaks the
//!     chain there.
//!   * **Completeness.** A deleted line leaves every surviving hash recomputing correctly
//!     against its own stored `prev`; only the `seq` column says a record went missing, and
//!     it is compared.
//!
//! One pass answers both, and everything else the file has to say. A break is a **finding**
//! and never an error: a chain that stops holding at seq 40 still has 40 links a reviewer
//! can read, and a verifier that refused to produce a verdict would hand the participant a
//! way to turn a Fail into a crash. Unsupported versions and incompatible version/body pairs
//! instead form a hard retained-prefix boundary: no envelope, store, post, event, head,
//! window, or acknowledgement candidate above it exists to this build. `load_and_verify`
//! returns `Err` only when bytes inside that readable prefix cannot be parsed.
//!
//! Only the **first** break is reported. The second is a consequence of the first — every
//! link after a rewritten one is chained over a hash that moved — and listing both invites
//! a reviewer to count problems rather than read one.

use crate::budget::RetentionBudget;
use crate::legs::{Leg1Record, Leg1Store, Leg2Post, Leg2Stream};
use goalcup_protocol::chain::{GENESIS, link_hash};
use goalcup_protocol::envelope::{Envelope, EnvelopeVersionCompatibilityError, EventBody};
use goalcup_protocol::redaction::RedactionRecord;
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum ChainError {
    #[error("chain {path} line {line} exceeds the {limit}-byte verifier limit")]
    EvidenceTooLarge {
        path: String,
        line: usize,
        limit: u64,
    },
    #[error(
        "chain retention at {path} would reach {bytes} bytes/{records} records, over the \
         verifier limits of {byte_limit} bytes/{record_limit} records"
    )]
    AggregateEvidenceTooLarge {
        path: String,
        bytes: u64,
        records: u64,
        byte_limit: u64,
        record_limit: u64,
    },
    #[error("io {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("chain {path} line {line}: {source}")]
    Json {
        path: String,
        line: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("chain {path} is empty")]
    Empty { path: String },
}

/// The only claims read before a version gate. Serde ignores the opaque body here, so a
/// future body kind cannot turn an unsupported-version boundary into a typed parse failure.
#[derive(Deserialize)]
struct EnvelopeHeader {
    protocol_version: u32,
    seq: u64,
}

/// Where the link chain first stops holding. Reported once, at the first break: the
/// second break is a consequence of the first and listing both invites a reviewer to
/// count problems rather than read one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainBreak {
    pub seq: u64,
    pub reason: ChainBreakReason,
    pub expected: String,
    pub observed: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChainBreakReason {
    /// `entry_id` changed within one chain file. A foreign entry is never retained.
    EntryIdMismatch,
    /// `seq` did not follow the previous line's `seq + 1`.
    SeqOutOfOrder { expected: u64 },
    /// `prev` did not equal the previous line's `hash`.
    PrevMismatch,
    /// `hash` is not `link_hash(prev, chain_bytes())`.
    HashMismatch,
    /// A valid successor cannot be represented after this link.
    SequenceExhausted,
}

/// A semantic mismatch between an envelope version/body and its typed redaction audit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RedactionAuditIssueReason {
    AuditOnLegacyVersion,
    AuditOnNonLeg,
    EmptySpans,
    EmptyKind { span: usize },
    EmptyOrReversedSpan { span: usize },
    UnorderedOrOverlappingSpan { span: usize },
    LegacyLogInV2,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactionAuditIssue {
    pub seq: u64,
    pub store: String,
    pub reason: RedactionAuditIssueReason,
}

/// An exact legacy salted marker whose subject has no matching legacy log record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyRedactionLogMissing {
    pub seq: u64,
    pub store: String,
}

/// A supported protocol number paired with a body that version does not define. The
/// offending line and every later byte are excluded from the verified chain prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvelopeCompatibilityIssue {
    pub seq: u64,
    pub reason: EnvelopeVersionCompatibilityError,
}

#[derive(Debug, Clone)]
pub struct VerifiedChain {
    pub entry_id: String,
    /// The hash of seq 0. What a start receipt's `chain_head` claim must equal.
    pub genesis_link: String,
    /// The head of the retained supported, compatible prefix.
    pub head_seq: u64,
    pub head_hash: String,
    /// Only the verified, version/body-compatible prefix, in file and sequence order.
    pub envelopes: Vec<Envelope>,
    /// Leg-1 records in that prefix, split by the body's own `store`, each an entry-chain store. The
    /// harness line is unwrapped once, here, so no parser ever sees a wrapper.
    pub stores: Vec<Leg1Store>,
    pub posts: Leg2Stream,
    /// Every retained envelope that is neither a leg-1 record nor a leg-2 post: tail notices,
    /// config deviations, foreign sessions, heartbeats, restarts, health, redactions.
    pub events: Vec<Envelope>,
    pub redactions: Vec<RedactionRecord>,
    /// The seq of the `EventBody::Seal` link, or `None` when the chain carries none.
    /// The loader states the fact; V13 is what decides that its absence on a sealed
    /// trace is `seal_not_chained`. `Some(n)` where `n < head_seq` is left to V13 too —
    /// a seal that is not last means records were appended after it.
    pub seal_link: Option<u64>,
    /// The seq of the `EventBody::Baseline` link, or `None` when the chain carries none.
    /// The mirror of `seal_link`, at the other end of the run: the writer only ever puts
    /// a baseline at seq 0, so `Some(n)` where `n != 0` is a chain somebody edited. The
    /// loader states the fact and V13 is what reads it as `baseline_not_chained`.
    pub baseline_link: Option<u64>,
    /// The digest that link claims for `baseline.json`. Stated, never compared: V13
    /// recomputes the declared manifest's own digest and decides whether the two agree.
    pub baseline_digest: Option<String>,
    /// The first break, or `None`. Never an error: a broken chain still yields a verdict.
    pub break_at: Option<ChainBreak>,
    /// Bytes on the final line that did not parse and were not newline-terminated.
    pub truncated_tail: Option<u64>,
    /// The first envelope whose `protocol_version` this build does not support.
    pub unsupported_version: Option<(u64, u32)>,
    /// The first supported-version envelope whose body is incompatible with that version.
    pub incompatible_envelope: Option<EnvelopeCompatibilityIssue>,
    /// Semantic audit defects on retained V2/V3 inline-redaction evidence. Version/body
    /// redaction-shape mismatches stop at `incompatible_envelope` before reaching here.
    pub redaction_audit_issues: Vec<RedactionAuditIssue>,
    /// Exact legacy markers without a matching `RedactionRecord.target_seq`.
    pub legacy_redaction_log_missing: Vec<LegacyRedactionLogMissing>,
}

pub(crate) const CHAIN_LINE_MAX_BYTES: u64 = 16 * 1024 * 1024;

pub fn load_and_verify_text(path: &str, text: &str) -> Result<VerifiedChain, ChainError> {
    load_and_verify_reader(path, std::io::Cursor::new(text.as_bytes()))
}

pub fn load_and_verify_reader(
    path: &str,
    reader: impl std::io::Read,
) -> Result<VerifiedChain, ChainError> {
    let mut budget = RetentionBudget::standard();
    load_and_verify_reader_with_budget(path, reader, CHAIN_LINE_MAX_BYTES, &mut budget)
}

#[cfg(test)]
fn load_and_verify_reader_with_limit(
    path: &str,
    reader: impl std::io::Read,
    line_limit: u64,
) -> Result<VerifiedChain, ChainError> {
    let mut budget = RetentionBudget::standard();
    load_and_verify_reader_with_budget(path, reader, line_limit, &mut budget)
}

#[cfg(test)]
fn load_and_verify_reader_with_limits(
    path: &str,
    reader: impl std::io::Read,
    line_limit: u64,
    byte_limit: u64,
    record_limit: u64,
) -> Result<VerifiedChain, ChainError> {
    let mut budget = RetentionBudget::with_limits(byte_limit, record_limit);
    load_and_verify_reader_with_budget(path, reader, line_limit, &mut budget)
}

pub(crate) fn load_and_verify_reader_with_budget(
    path: &str,
    reader: impl std::io::Read,
    line_limit: u64,
    budget: &mut RetentionBudget,
) -> Result<VerifiedChain, ChainError> {
    use std::io::{BufRead, Read};

    let mut envelopes: Vec<Envelope> = Vec::new();
    let mut truncated_tail: Option<u64> = None;
    let mut entry_id = String::new();
    let mut genesis_link = GENESIS.to_string();
    let mut break_at: Option<ChainBreak> = None;
    let mut unsupported_version: Option<(u64, u32)> = None;
    let mut incompatible_envelope: Option<EnvelopeCompatibilityIssue> = None;
    let mut seal_link: Option<u64> = None;
    let mut baseline_link: Option<u64> = None;
    let mut baseline_digest: Option<String> = None;
    let mut stores: Vec<Leg1Store> = Vec::new();
    let mut posts: Vec<Leg2Post> = Vec::new();
    let mut events: Vec<Envelope> = Vec::new();
    let mut redactions: Vec<RedactionRecord> = Vec::new();
    let mut redaction_audit_issues: Vec<RedactionAuditIssue> = Vec::new();
    let mut legacy_marker_subjects: Vec<LegacyRedactionLogMissing> = Vec::new();

    let mut expected_seq = 0u64;
    let mut head_seq = 0u64;
    let mut prev = GENESIS.to_string();
    // Read one bounded physical line at a time. A partial final write remains distinguishable
    // because `read_until` tells us whether it found the newline terminator; no whole chain or
    // attacker-sized single line is allocated before that decision.
    let mut reader = std::io::BufReader::new(reader);
    let mut segment = Vec::new();
    let mut line_number = 0usize;
    loop {
        segment.clear();
        let read = (&mut reader)
            .take(line_limit.saturating_add(1))
            .read_until(b'\n', &mut segment)
            .map_err(|source| ChainError::Io {
                path: path.to_string(),
                source,
            })?;
        if read == 0 {
            break;
        }
        line_number = line_number.saturating_add(1);
        if u64::try_from(read).unwrap_or(u64::MAX) > line_limit {
            return Err(ChainError::EvidenceTooLarge {
                path: path.to_string(),
                line: line_number,
                limit: line_limit,
            });
        }
        let terminated = segment.last() == Some(&b'\n');
        if terminated {
            segment.pop();
        }
        if segment.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let header: EnvelopeHeader = match serde_json::from_slice(&segment) {
            Ok(header) => header,
            Err(_source) if !terminated => {
                truncated_tail = Some(segment.len() as u64);
                continue;
            }
            Err(source) => {
                return Err(ChainError::Json {
                    path: path.to_string(),
                    line: line_number,
                    source,
                });
            }
        };
        if !goalcup_protocol::is_supported_protocol_version(header.protocol_version) {
            unsupported_version = Some((header.seq, header.protocol_version));
            // Stop before typed decoding, link validation, or retaining any V4 bytes. The
            // suffix is outside this build's evidence contract too, even if it is malformed.
            break;
        }
        let env: Envelope = match serde_json::from_slice(&segment) {
            Ok(env) => env,
            Err(_source) if !terminated => {
                truncated_tail = Some(segment.len() as u64);
                continue;
            }
            Err(source) => {
                return Err(ChainError::Json {
                    path: path.to_string(),
                    line: line_number,
                    source,
                });
            }
        };
        if let Err(reason) = env.validate_version_body_compatibility() {
            incompatible_envelope = Some(EnvelopeCompatibilityIssue {
                seq: header.seq,
                reason,
            });
            break;
        }
        if envelopes.is_empty() {
            entry_id = env.entry_id.clone();
            genesis_link = env.link.hash.clone();
        } else if env.entry_id != entry_id {
            break_at = Some(ChainBreak {
                seq: env.link.seq,
                reason: ChainBreakReason::EntryIdMismatch,
                expected: entry_id.clone(),
                observed: env.entry_id.clone(),
            });
            // The entry id is part of the evidence boundary, not just another link
            // claim. Stop before auditing, classifying, or retaining this envelope so a
            // correctly linked foreign suffix cannot enter any derived store or event.
            break;
        } else if env.link.seq == 0 {
            genesis_link = env.link.hash.clone();
        }
        budget
            .retain(path, u64::try_from(read).unwrap_or(u64::MAX))
            .map_err(|limit| ChainError::AggregateEvidenceTooLarge {
                path: limit.path,
                bytes: limit.bytes,
                records: limit.records,
                byte_limit: limit.byte_limit,
                record_limit: limit.record_limit,
            })?;
        if let Some(reason) = redaction_audit_issue(&env) {
            redaction_audit_issues.push(RedactionAuditIssue {
                seq: env.link.seq,
                store: envelope_store(&env),
                reason,
            });
        }
        if env.redaction.is_none() && leg_payload_has_exact_legacy_marker(&env.body) {
            legacy_marker_subjects.push(LegacyRedactionLogMissing {
                seq: env.link.seq,
                store: envelope_store(&env),
            });
        }

        // The three link questions, in the order that makes the answer readable: a line
        // in the wrong position is not also a hash failure, and a hash recomputed over a
        // `prev` that is already wrong says nothing.
        let recomputed = env.chain_bytes().ok().map(|bytes| link_hash(&prev, &bytes));
        if break_at.is_none() {
            if env.link.seq != expected_seq {
                break_at = Some(ChainBreak {
                    seq: env.link.seq,
                    reason: ChainBreakReason::SeqOutOfOrder {
                        expected: expected_seq,
                    },
                    expected: expected_seq.to_string(),
                    observed: env.link.seq.to_string(),
                });
            } else if env.link.prev != prev {
                break_at = Some(ChainBreak {
                    seq: env.link.seq,
                    reason: ChainBreakReason::PrevMismatch,
                    expected: prev.clone(),
                    observed: env.link.prev.clone(),
                });
            } else {
                match &recomputed {
                    Some(hash) if *hash == env.link.hash => {}
                    // An envelope whose own bytes will not re-serialise cannot be
                    // link-checked. That is a break, not a load error: the file still
                    // yields a verdict, and a verifier that exited here would hand the
                    // participant a crash in place of a Fail.
                    other => {
                        break_at = Some(ChainBreak {
                            seq: env.link.seq,
                            reason: ChainBreakReason::HashMismatch,
                            expected: other
                                .clone()
                                .unwrap_or_else(|| "unrecomputable".to_string()),
                            observed: env.link.hash.clone(),
                        });
                    }
                }
            }
        }
        // The walk continues over what is on disk rather than over what should have been
        // there, so one break does not turn every later line into a second one.
        head_seq = env.link.seq;
        match env.link.seq.checked_add(1) {
            Some(next) => expected_seq = next,
            None => {
                if break_at.is_none() {
                    break_at = Some(ChainBreak {
                        seq: env.link.seq,
                        reason: ChainBreakReason::SequenceExhausted,
                        expected: "a representable successor".to_string(),
                        observed: "u64::MAX".to_string(),
                    });
                }
                // The first break remains authoritative. Keep a stable expected value for
                // any malformed suffix rather than wrapping the position to zero.
                expected_seq = u64::MAX;
            }
        }
        prev = env.link.hash.clone();

        match &env.body {
            EventBody::Leg1Record { store, payload } => {
                // Unwrapped once, here. A line-oriented store's payload is the harness's
                // own line as a JSON *string*; a SQLite store's is the row as an object.
                // Every parser downstream reads `value.get("type")` and must never see a
                // wrapper, so the string is decoded exactly once and exactly here.
                let value = match payload {
                    serde_json::Value::String(line) => serde_json::from_str(line)
                        .unwrap_or_else(|_| serde_json::Value::String(line.clone())),
                    other => other.clone(),
                };
                // The store is the body's `store`, never `env.source`. `source` names the
                // component that emitted the line — `"daemon"` for everything a daemon
                // observed, which is every leg-1 record a real tailer ships — so reading a
                // store name off it collapses every store in the entry into one called
                // `daemon`, and the descriptor's per-store rules (partition keys, cursors,
                // tail notices) then name stores the verifier does not have.
                let record = Leg1Record {
                    store: store.clone(),
                    seq: env.link.seq,
                    prev: env.link.prev.clone(),
                    hash: env.link.hash.clone(),
                    recorded_at: env.recorded_at_secs().map(|s| s as f64),
                    value,
                    // The bytes the hash was taken over are the envelope's
                    // `chain_bytes()`, not the harness line, so any bytes stored here
                    // would be a lie. `gaps::verify_acks` re-derives the hash for a
                    // chain-derived record from the envelope itself.
                    raw: Vec::new(),
                };
                match stores.iter_mut().find(|s| &s.name == store) {
                    Some(existing) => existing.records.push(record),
                    // First-appearance order, so the store list is a function of the
                    // file rather than of a hash map's iteration order.
                    None => stores.push(Leg1Store::from_chain(store, vec![record])),
                }
            }
            // `..`: `receiver_meta` is the receiver's own bookkeeping, carried raw. This
            // derived `Leg2Post` view drops it — the rest-pattern is deliberate, a later
            // field is additive by the same rule, and this arm reads the three it names —
            // and V4's `leg2_receiver_seq_gap` reads it straight off
            // `VerifiedChain::envelopes` below, which keeps every field of the raw
            // envelope, `receiver_meta` included.
            EventBody::Leg2Post {
                event,
                payload,
                path,
                ..
            } => {
                posts.push(Leg2Post {
                    index: posts.len(),
                    // `0.0` when the clock does not parse, matching how
                    // `Leg2Stream::from_file` treats a post with no usable `t`.
                    received_at: env.recorded_at_secs().map(|s| s as f64).unwrap_or(0.0),
                    event: event
                        .trim_start_matches('/')
                        .trim_end_matches("-cmd")
                        .to_string(),
                    // The receiver path verbatim, leading slash and `-cmd` suffix
                    // included: `Leg2Post::transport()` reads it to tell "cmd" from
                    // "http", which duplicate-pairing depends on.
                    //
                    // The chain carries it separately: the event name is the header's,
                    // which is the same word on both of Claude Code's transports, so a
                    // chain with only the event would name `http` for both and turn the
                    // second copy of every prompt into a `duplicate_pairing` Fail. An older
                    // chain without the field on every post falls back to the event, which
                    // is what those bytes mean.
                    path: path.clone().unwrap_or_else(|| event.clone()),
                    body: payload.clone(),
                });
            }
            body => {
                if env.protocol_version == goalcup_protocol::LEGACY_PROTOCOL_VERSION
                    && let EventBody::Redaction(r) = body
                {
                    // Only a v1 log can complete a v1 record's legacy audit pair. A v2
                    // Redaction body is retained in `events` and reported as invalid,
                    // but cannot erase the missing-evidence ambiguity it targets.
                    redactions.push(r.clone());
                }
                // The baseline, noted the way the seal is. A chain carrying two keeps
                // the first: the writer refuses to chain a second one, so a file with two
                // is one somebody edited, and the first is the one a receipt's
                // `chain_head` could have witnessed.
                if let EventBody::Baseline {
                    manifest_sha256, ..
                } = body
                    && baseline_link.is_none()
                {
                    baseline_link = Some(env.link.seq);
                    baseline_digest = Some(manifest_sha256.clone());
                }
                if matches!(body, EventBody::Seal { .. }) {
                    // Noted while the file is being walked rather than searched for
                    // later, so V13 can say "there is no seal in this chain" without a
                    // second pass. A second seal is the writer's error to refuse; the
                    // loader keeps the first, which is the one an acknowledgement below
                    // it anchors.
                    seal_link = seal_link.or(Some(env.link.seq));
                }
                events.push(env.clone());
            }
        }
        envelopes.push(env);
    }

    if envelopes.is_empty() && unsupported_version.is_none() && incompatible_envelope.is_none() {
        return Err(ChainError::Empty {
            path: path.to_string(),
        });
    }

    let legacy_targets: std::collections::BTreeSet<u64> =
        redactions.iter().map(|r| r.target_seq).collect();
    let legacy_redaction_log_missing = legacy_marker_subjects
        .into_iter()
        .filter(|subject| !legacy_targets.contains(&subject.seq))
        .collect();

    // The head is the last retained prefix link, break or no break. `prev` and
    // `expected_seq` are carried out of the incremental walk rather than looked up a second
    // time, so no unsupported or incompatible suffix can become a head claim.
    Ok(VerifiedChain {
        entry_id,
        genesis_link,
        head_seq,
        head_hash: prev,
        envelopes,
        stores,
        posts: Leg2Stream { posts },
        events,
        redactions,
        seal_link,
        baseline_link,
        baseline_digest,
        break_at,
        truncated_tail,
        unsupported_version,
        incompatible_envelope,
        redaction_audit_issues,
        legacy_redaction_log_missing,
    })
}

fn envelope_store(env: &Envelope) -> String {
    match &env.body {
        EventBody::Leg1Record { store, .. }
        | EventBody::Redaction(RedactionRecord { store, .. }) => store.clone(),
        _ => env.source.clone(),
    }
}

fn redaction_audit_issue(env: &Envelope) -> Option<RedactionAuditIssueReason> {
    if env.protocol_version == goalcup_protocol::LEGACY_PROTOCOL_VERSION {
        return env
            .redaction
            .is_some()
            .then_some(RedactionAuditIssueReason::AuditOnLegacyVersion);
    }
    if env.protocol_version < goalcup_protocol::INLINE_REDACTION_PROTOCOL_VERSION {
        return None;
    }
    if matches!(env.body, EventBody::Redaction(_)) {
        return Some(RedactionAuditIssueReason::LegacyLogInV2);
    }
    let Some(audit) = &env.redaction else {
        return None;
    };
    if !matches!(
        env.body,
        EventBody::Leg1Record { .. } | EventBody::Leg2Post { .. }
    ) {
        return Some(RedactionAuditIssueReason::AuditOnNonLeg);
    }
    if audit.spans.is_empty() {
        return Some(RedactionAuditIssueReason::EmptySpans);
    }
    let mut previous_end = 0u64;
    for (index, span) in audit.spans.iter().enumerate() {
        if span.kind.is_empty() {
            return Some(RedactionAuditIssueReason::EmptyKind { span: index });
        }
        if span.start >= span.end {
            return Some(RedactionAuditIssueReason::EmptyOrReversedSpan { span: index });
        }
        if index > 0 && span.start < previous_end {
            return Some(RedactionAuditIssueReason::UnorderedOrOverlappingSpan { span: index });
        }
        previous_end = span.end;
    }
    None
}

fn leg_payload_has_exact_legacy_marker(body: &EventBody) -> bool {
    match body {
        EventBody::Leg1Record { payload, .. } | EventBody::Leg2Post { payload, .. } => {
            value_has_exact_legacy_marker(payload)
        }
        _ => false,
    }
}

fn value_has_exact_legacy_marker(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(text) => text_has_exact_legacy_marker(text),
        serde_json::Value::Array(values) => values.iter().any(value_has_exact_legacy_marker),
        serde_json::Value::Object(values) => values.iter().any(|(key, value)| {
            text_has_exact_legacy_marker(key) || value_has_exact_legacy_marker(value)
        }),
        _ => false,
    }
}

fn text_has_exact_legacy_marker(text: &str) -> bool {
    const OPEN: &str = "<redacted:";
    let mut rest = text;
    while let Some(at) = rest.find(OPEN) {
        let from = &rest[at + OPEN.len()..];
        let Some(close) = from.find('>') else {
            return false;
        };
        let body = &from[..close];
        if let Some((kind, digest)) = body.rsplit_once(':')
            && !kind.is_empty()
            && digest.len() == 16
            && digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return true;
        }
        rest = &from[close + 1..];
    }
    false
}

#[cfg(test)]
mod resource_tests {
    use super::{
        ChainError, load_and_verify_reader_with_limit, load_and_verify_reader_with_limits,
    };

    #[test]
    fn an_oversized_chain_line_is_rejected_at_the_incremental_reader_boundary() {
        assert!(matches!(
            load_and_verify_reader_with_limit("chain.jsonl", std::io::Cursor::new(b"xxxxxxxxx"), 8),
            Err(ChainError::EvidenceTooLarge { path, line: 1, limit })
                if path == "chain.jsonl" && limit == 8
        ));
    }

    #[test]
    fn many_valid_small_chain_lines_hit_the_typed_retention_budget() {
        let bytes = include_bytes!("../tests/fixtures/compliant/chain.jsonl");
        assert!(matches!(
            load_and_verify_reader_with_limits(
                "chain.jsonl",
                std::io::Cursor::new(bytes),
                super::CHAIN_LINE_MAX_BYTES,
                u64::MAX,
                2,
            ),
            Err(ChainError::AggregateEvidenceTooLarge {
                path,
                records: 3,
                record_limit: 2,
                ..
            }) if path == "chain.jsonl"
        ));
    }

    #[test]
    fn cumulative_valid_chain_bytes_hit_the_typed_retention_budget() {
        let bytes = include_bytes!("../tests/fixtures/compliant/chain.jsonl");
        let mut lines = bytes.split_inclusive(|byte| *byte == b'\n');
        let first = lines.next().unwrap().len() as u64;
        let second = lines.next().unwrap().len() as u64;
        let attempted = first + second;
        assert!(matches!(
            load_and_verify_reader_with_limits(
                "chain.jsonl",
                std::io::Cursor::new(bytes),
                super::CHAIN_LINE_MAX_BYTES,
                attempted - 1,
                u64::MAX,
            ),
            Err(ChainError::AggregateEvidenceTooLarge {
                path,
                bytes,
                byte_limit,
                ..
            }) if path == "chain.jsonl" && bytes == attempted && byte_limit == attempted - 1
        ));
    }
}
