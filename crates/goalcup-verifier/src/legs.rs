//! Loading the two legs. Leg 1 is the harness's persisted store; leg 2 is the hook or
//! plugin post stream (DECISIONS D2). Neither is modified here.

use crate::VerifierError;
use crate::budget::RetentionBudget;
use crate::trace::StoreKind;
use goalcup_protocol::chain::{Chain, link_hash};
use sha2::{Digest, Sha256};

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// When a raw harness record carries its own wall-clock time, read it: the gap classifier
/// needs to place a record inside or outside a hole in the heartbeat series, and a record
/// that cannot be placed in time is a record no clean/dirty judgement can be made about.
///
/// Three shapes cover the measured harnesses: Claude Code's RFC 3339 `timestamp`, Codex's
/// RFC 3339 `timestamp`, and OpenCode's numeric `time_created` (milliseconds). Anything
/// else yields `None`, which the classifier treats as "cannot be placed" and never as
/// "outside the hole".
///
/// `pub(crate)` because `gaps::records_in_window` asks the same question of a
/// chain-derived record: the envelope's `recorded_at` is the *daemon's* observation, and
/// the harness's own clock — which is what a hole has to be measured against — lives here,
/// in the record. One reader for both, so a store loaded from a file and the same store
/// lifted out of the chain are placed in time by the same three shapes.
pub(crate) fn record_time(v: &serde_json::Value) -> Option<f64> {
    if let Some(s) = v.get("timestamp").and_then(|t| t.as_str())
        && let Some(secs) = goalcup_protocol::control::parse_z(s)
    {
        return Some(secs as f64);
    }
    if let Some(ms) = v.get("time_created").and_then(|t| t.as_u64()) {
        return Some(ms as f64 / 1000.0);
    }
    None
}

/// Walk a dotted path into a JSON value. `dotted(v, "payload.thread_source")`.
pub fn dotted<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut cur = value;
    for segment in path.split('.') {
        cur = cur.get(segment)?;
    }
    Some(cur)
}

/// A dotted field read as a string, descending **through** a JSON-encoded string node —
/// the same walk `goalcup_daemon::partition`'s private `str_at` runs at capture time.
///
/// **Why this exists beside [`dotted`] rather than inside it.** `dotted` returns a
/// borrowed `&'a Value`, which every one of its ~80 call sites relies on to hand a
/// borrowed `&str`/`bool`/`u64` back to their own caller. A node reached by parsing a
/// JSON-encoded string is a fresh, owned `Value` with no relation to `'a`, so honouring
/// it would mean `dotted` could no longer return a plain borrow — a signature change that
/// ripples through every parser in this crate. This function is additive instead: same
/// walk, but it stops at a string leaf (there is nothing else V10's `Direct` partition
/// path ever reads) and returns [`std::borrow::Cow`], which owns the parsed bytes only
/// when the walk actually crossed one.
///
/// **The divergence this closes (V10).** A `store[].partition = { strategy = "direct", path
/// = "…" }` field is compared against the tracked session by
/// [`crate::checks::v10_partition`] using exactly this reader, so the verifier's sweep can
/// never disagree with what the daemon partitioned on at capture time.
///
/// Object keys only, exactly as `dotted`: an index into an array is not a field name.
/// `None` for an absent field, a non-string leaf, or a string leaf that is not valid JSON
/// where more path remains.
pub fn dotted_through_strings<'a>(
    value: &'a serde_json::Value,
    path: &str,
) -> Option<std::borrow::Cow<'a, str>> {
    if path.is_empty() {
        return None;
    }
    let (head, rest) = match path.split_once('.') {
        Some((head, rest)) => (head, Some(rest)),
        None => (path, None),
    };
    let node = value.get(head)?;
    match rest {
        None => node.as_str().map(std::borrow::Cow::Borrowed),
        Some(rest) => match node {
            serde_json::Value::String(encoded) => {
                let parsed: serde_json::Value = serde_json::from_str(encoded).ok()?;
                dotted_through_strings(&parsed, rest)
                    .map(|s| std::borrow::Cow::Owned(s.into_owned()))
            }
            _ => dotted_through_strings(node, rest),
        },
    }
}

#[derive(Debug, Clone)]
pub struct Leg1Record {
    pub store: String,
    pub seq: u64,
    pub prev: String,
    pub hash: String,
    /// Seconds since the epoch: the tailer's `t` for a `ChainJsonl` record, or the
    /// harness's own timestamp field for a `RawJsonl` one. `None` when the record carries
    /// no time at all, which the gap classifier reads as "cannot be placed" rather than as
    /// "outside the window".
    pub recorded_at: Option<f64>,
    /// The decoded record: the harness's own JSON for `RawJsonl`, the `line` object
    /// for `ChainJsonl`.
    pub value: serde_json::Value,
    /// The exact bytes the hash was taken over, where they are available.
    pub raw: Vec<u8>,
}

/// Which sequence space a store's `Leg1Record.seq` lives in.
///
/// This is a fact about where the records came from, not a field anybody declares:
/// `StoreKind` is a descriptor field the participant writes, and reading the space off it
/// would pool the daemon's entry chain with the verifier's own per-store counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeqSpace {
    /// The entry's one chain. Directly comparable with any envelope's `seq` and with
    /// an acknowledgement's.
    EntryChain,
    /// The verifier's own per-store counter over a legacy M0 fixture. Comparable with
    /// nothing outside its own store.
    LocalPerStore,
}

#[derive(Debug, Clone)]
pub struct Leg1Store {
    pub name: String,
    /// What the descriptor said this file was. It decides how the file is *read*; it does
    /// not decide how a record is placed in time or which sequences it may be compared with
    /// — `space` does.
    pub kind: StoreKind,
    /// Which sequence space `Leg1Record.seq` lives in, and therefore how a record can be
    /// placed in a window. A chain-derived store carries the daemon's own chain sequences,
    /// which are directly comparable with an envelope's `seq`. Every store a *descriptor*
    /// declares — `raw_jsonl` and `chain_jsonl` alike — is chained or re-read here, per
    /// store, so its sequences mean nothing outside that store and are never compared with
    /// an envelope's. The two spaces are never pooled.
    pub space: SeqSpace,
    pub records: Vec<Leg1Record>,
    /// Bytes on the final line that did not parse and were not newline-terminated.
    ///
    /// A hard kill is the exact scenario this tracker exists to record, and what it leaves
    /// on disk is a half-written last line: the harness appends whole records, so bytes
    /// with no terminator after them are a write that did not finish. A hard error — exit
    /// 3, no verdict — would hand a participant a way to turn a Fail into a crash by
    /// killing the harness mid-append, so it is a fact the store carries and V13 renders as
    /// the `truncated_tail` Flag. Every complete record before it still parses and is still
    /// in `records`; only the unfinished bytes are set aside.
    ///
    /// `None` on a chain-derived store (`from_chain`): the chain has its own tail, read
    /// once by `chain::load_and_verify` and carried on `VerifiedChain::truncated_tail`, so
    /// a chain trace's tail is reported once rather than by every store lifted out of it.
    pub truncated_tail: Option<u64>,
    /// Declared by the trace descriptor: this file was altered after capture, so a
    /// hash mismatch is the scrub and not tamper evidence.
    scrubbed: bool,
}

const LEG_LINE_MAX_BYTES: u64 = 16 * 1024 * 1024;

/// Read one physical JSONL line without allowing its buffer to grow past the evidence policy.
fn next_bounded_line(
    reader: &mut impl std::io::BufRead,
    path: &str,
    line: usize,
    limit: u64,
) -> Result<Option<(Vec<u8>, bool, u64)>, VerifierError> {
    use std::io::{BufRead, Read};

    let mut bytes = Vec::new();
    let read = (&mut *reader)
        .take(limit.saturating_add(1))
        .read_until(b'\n', &mut bytes)
        .map_err(|source| VerifierError::Io {
            path: path.to_string(),
            source,
        })?;
    if read == 0 {
        return Ok(None);
    }
    if u64::try_from(read).unwrap_or(u64::MAX) > limit {
        return Err(VerifierError::EvidenceTooLarge {
            path: format!("{path} line {line}"),
            limit,
        });
    }
    let terminated = bytes.last() == Some(&b'\n');
    if terminated {
        bytes.pop();
    }
    Ok(Some((
        bytes,
        terminated,
        u64::try_from(read).unwrap_or(u64::MAX),
    )))
}

impl Leg1Store {
    /// One store's worth of leg-1 records lifted out of the entry chain.
    ///
    /// `kind` is `StoreKind::ChainJsonl` — it is the daemon's chained stream — with
    /// `space: SeqSpace::EntryChain` and `scrubbed: false`: a chain the daemon wrote is
    /// always recomputed, and `chain::load_and_verify` is what recomputes it.
    pub fn from_chain(name: &str, records: Vec<Leg1Record>) -> Leg1Store {
        Leg1Store {
            name: name.to_string(),
            kind: StoreKind::ChainJsonl,
            space: SeqSpace::EntryChain,
            records,
            // The chain's own tail is read once, by `chain::load_and_verify`, and reported
            // once from `VerifiedChain`. A store lifted out of it has no file of its own.
            truncated_tail: None,
            scrubbed: false,
        }
    }
}

#[cfg(test)]
impl Leg1Store {
    /// Build a `Leg1Store` directly from constructed records, for a check's unit test
    /// that needs `CheckContext.stores` populated without loading a fixture file —
    /// `scrubbed` is private to this module by design (a hash mismatch means something
    /// different depending on it), so a check's own test module cannot build one with a
    /// struct literal.
    pub(crate) fn from_records(
        name: &str,
        kind: StoreKind,
        space: SeqSpace,
        records: Vec<Leg1Record>,
    ) -> Leg1Store {
        Leg1Store {
            name: name.to_string(),
            kind,
            space,
            records,
            truncated_tail: None,
            scrubbed: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkReport {
    pub adjacent_linked: usize,
    pub broken: usize,
    pub seq_gaps: usize,
    /// How many records were recomputed from their own bytes.
    pub recomputed: usize,
    /// How many of those disagreed with the recorded hash.
    pub recompute_mismatches: usize,
    /// The descriptor's override: mismatches here are the documented scrub.
    pub scrubbed: bool,
    /// Computed: every record was recomputed and every one matched.
    pub recomputable: bool,
}

impl Leg1Store {
    /// Parse one JSON record per line after the rooted reader has supplied its bytes.
    pub(crate) fn from_raw_jsonl_reader_with_budget(
        name: &str,
        path: &str,
        reader: impl std::io::Read,
        budget: &mut RetentionBudget,
    ) -> Result<Leg1Store, VerifierError> {
        let mut reader = std::io::BufReader::new(reader);
        let mut chain = Chain::new();
        let mut records = Vec::new();
        let mut truncated_tail = None;
        let mut line_number = 0usize;
        while let Some((line, terminated, physical_bytes)) =
            next_bounded_line(&mut reader, path, line_number + 1, LEG_LINE_MAX_BYTES)?
        {
            line_number += 1;
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let value: serde_json::Value = match serde_json::from_slice(&line) {
                Ok(v) => v,
                Err(source) => {
                    if !terminated {
                        truncated_tail = Some(line.len() as u64);
                        continue;
                    }
                    return Err(VerifierError::Json {
                        path: path.to_string(),
                        line: line_number,
                        source,
                    });
                }
            };
            budget.retain(path, physical_bytes)?;
            let link = chain.try_append(&line).map_err(|error| {
                VerifierError::Trace(format!("legacy chain writer refused a record: {error}"))
            })?;
            let recorded_at = record_time(&value);
            records.push(Leg1Record {
                store: name.to_string(),
                seq: link.seq,
                prev: link.prev,
                hash: link.hash,
                recorded_at,
                value,
                raw: line,
            });
        }
        Ok(Leg1Store {
            name: name.to_string(),
            kind: StoreKind::RawJsonl,
            // The verifier chained this file itself, from 1, in this function: those
            // sequences are its own bookkeeping and mean nothing outside this store.
            space: SeqSpace::LocalPerStore,
            records,
            truncated_tail,
            scrubbed: false,
        })
    }

    /// Parse a tailer chain after the rooted reader has supplied its bytes.
    pub fn from_chain_jsonl_reader(
        name: &str,
        path: &str,
        reader: impl std::io::Read,
        scrubbed: bool,
    ) -> Result<Leg1Store, VerifierError> {
        let mut budget = RetentionBudget::standard();
        Self::from_chain_jsonl_reader_with_budget(name, path, reader, scrubbed, &mut budget)
    }

    pub(crate) fn from_chain_jsonl_reader_with_budget(
        name: &str,
        path: &str,
        reader: impl std::io::Read,
        scrubbed: bool,
        budget: &mut RetentionBudget,
    ) -> Result<Leg1Store, VerifierError> {
        #[derive(serde::Deserialize)]
        struct Envelope {
            t: Option<f64>,
            seq: u64,
            prev: String,
            hash: String,
            line: serde_json::Value,
        }
        let mut reader = std::io::BufReader::new(reader);
        let mut records = Vec::new();
        let mut truncated_tail = None;
        let mut line_number = 0usize;
        while let Some((line, terminated, physical_bytes)) =
            next_bounded_line(&mut reader, path, line_number + 1, LEG_LINE_MAX_BYTES)?
        {
            line_number += 1;
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let env: Envelope = match serde_json::from_slice(&line) {
                Ok(e) => e,
                Err(source) => {
                    if !terminated {
                        truncated_tail = Some(line.len() as u64);
                        continue;
                    }
                    return Err(VerifierError::Json {
                        path: path.to_string(),
                        line: line_number,
                        source,
                    });
                }
            };
            budget.retain(path, physical_bytes)?;
            // The bytes the daemon hashed: the canonical serialisation of `line`.
            // Kept so `link_report` can recompute rather than take the record's word.
            let raw = serde_json::to_vec(&env.line).unwrap_or_default();
            records.push(Leg1Record {
                store: name.to_string(),
                seq: env.seq,
                prev: env.prev,
                hash: env.hash,
                recorded_at: env.t,
                value: env.line,
                raw,
            });
        }
        Ok(Leg1Store {
            name: name.to_string(),
            kind: StoreKind::ChainJsonl,
            // A legacy fixture's own tailer sequences, not this entry's chain: the M0
            // captures predate the daemon, and their `seq` is comparable with nothing
            // outside the file it came from.
            space: SeqSpace::LocalPerStore,
            records,
            truncated_tail,
            scrubbed,
        })
    }

    /// Structural linkage plus recomputation. `recomputable` is a *result*, not a
    /// declaration: it is true when every record was recomputed from its own bytes and
    /// every one matched. `scrubbed` is the descriptor's explicit override for a
    /// committed fixture that was altered after capture; it forces `recomputable` false
    /// and keeps the mismatches out of `broken`, so a documented scrub never reads as
    /// tamper evidence and an undocumented one always does.
    pub fn link_report(&self) -> LinkReport {
        let mut adjacent_linked = 0usize;
        let mut broken = 0usize;
        let mut seq_gaps = 0usize;
        for w in self.records.windows(2) {
            if w[0].seq.checked_add(1) == Some(w[1].seq) {
                if w[1].prev == w[0].hash {
                    adjacent_linked += 1;
                } else {
                    broken += 1;
                }
            } else {
                seq_gaps += 1;
            }
        }

        let mut recomputed = 0usize;
        let mut recompute_mismatches = 0usize;
        for r in &self.records {
            if r.raw.is_empty() {
                continue;
            }
            recomputed += 1;
            if link_hash(&r.prev, &r.raw) != r.hash {
                recompute_mismatches += 1;
            }
        }
        if !self.scrubbed {
            broken += recompute_mismatches;
        }

        LinkReport {
            adjacent_linked,
            broken,
            seq_gaps,
            recomputed,
            recompute_mismatches,
            scrubbed: self.scrubbed,
            recomputable: !self.scrubbed && recomputed > 0 && recompute_mismatches == 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Leg2Post {
    pub index: usize,
    pub received_at: f64,
    /// Canonical event name: the receiver path with its leading slash and any
    /// `-cmd` transport suffix removed, so a harness posting the same event through
    /// two sources yields one event name.
    pub event: String,
    /// The raw receiver path, kept verbatim. `event` deliberately erases the `-cmd` suffix;
    /// the transport it names is what tells the *same submission arriving twice* apart from
    /// *a submission replayed twice*. Erasing both would make the duplicate-pairing defence
    /// unreachable.
    pub path: String,
    pub body: serde_json::Value,
}

impl Leg2Post {
    /// Which hook source carried this post. Claude Code fires every prompt over both the
    /// http hook and the `-cmd` shim, and the shim's path carries the `-cmd` suffix; a
    /// harness with one source posts everything over `"http"`.
    pub fn transport(&self) -> &'static str {
        if self.path.ends_with("-cmd") {
            "cmd"
        } else {
            "http"
        }
    }
}

#[derive(Debug, Clone)]
pub struct Leg2Stream {
    pub posts: Vec<Leg2Post>,
}

impl Leg2Stream {
    pub fn from_reader(
        path: &str,
        reader: impl std::io::Read,
    ) -> Result<Leg2Stream, VerifierError> {
        let mut budget = RetentionBudget::standard();
        Self::from_reader_with_budget(path, reader, &mut budget)
    }

    pub(crate) fn from_reader_with_budget(
        path: &str,
        reader: impl std::io::Read,
        budget: &mut RetentionBudget,
    ) -> Result<Leg2Stream, VerifierError> {
        #[derive(serde::Deserialize)]
        struct Raw {
            t: f64,
            path: String,
            #[serde(default)]
            body: serde_json::Value,
        }
        let mut reader = std::io::BufReader::new(reader);
        let mut posts = Vec::new();
        let mut line_number = 0usize;
        while let Some((line, _terminated, physical_bytes)) =
            next_bounded_line(&mut reader, path, line_number + 1, LEG_LINE_MAX_BYTES)?
        {
            line_number += 1;
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let raw: Raw = serde_json::from_slice(&line).map_err(|source| VerifierError::Json {
                path: path.to_string(),
                line: line_number,
                source,
            })?;
            budget.retain(path, physical_bytes)?;
            let event = raw
                .path
                .trim_start_matches('/')
                .trim_end_matches("-cmd")
                .to_string();
            posts.push(Leg2Post {
                index: line_number - 1,
                received_at: raw.t,
                event,
                path: raw.path,
                body: raw.body,
            });
        }
        Ok(Leg2Stream { posts })
    }

    pub fn by_event<'a>(&'a self, name: &str) -> Vec<&'a Leg2Post> {
        self.posts.iter().filter(|p| p.event == name).collect()
    }
}

/// Every `EventBody::HomeSeeded` in the chain, in chain order.
///
/// **A reader and never a judge.** It parses the record the daemon chained and hands it to
/// the verdict as [`crate::verdict::Verdict::carried_settings`]; nothing here compares it
/// against anything, raises a finding, or moves `overall`. Which of their own settings a
/// participant carried into a run is theirs to decide, and this is the sentence that says
/// which ones they were.
///
/// At most one per launch that registered a process, so a run resumed twice normally carries
/// three, in launch order. An empty answer is a chain that carries none — an older chain, a
/// connector with no `[home_seed]`, or a launch whose seeding failed and therefore claimed
/// nothing — and never a claim that nothing was seeded.
pub fn carried_settings(
    events: &[goalcup_protocol::envelope::Envelope],
) -> Vec<goalcup_protocol::events::HomeSeedRecord> {
    events
        .iter()
        .filter_map(|e| match &e.body {
            goalcup_protocol::envelope::EventBody::HomeSeeded(r) => Some(r.clone()),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod retention_tests {
    use super::{Leg1Store, Leg2Stream};
    use crate::VerifierError;
    use crate::budget::RetentionBudget;

    #[test]
    fn many_small_legacy_records_share_the_typed_retention_budget() {
        let mut budget = RetentionBudget::with_limits(1024, 1);
        let error = Leg1Store::from_raw_jsonl_reader_with_budget(
            "main",
            "leg1.jsonl",
            std::io::Cursor::new(b"{}\n{}\n"),
            &mut budget,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            VerifierError::AggregateEvidenceTooLarge {
                path,
                records: 2,
                record_limit: 1,
                ..
            } if path == "leg1.jsonl"
        ));

        let mut budget = RetentionBudget::with_limits(4096, 1);
        let posts = b"{\"t\":1.0,\"path\":\"/Stop\"}\n{\"t\":2.0,\"path\":\"/Stop\"}\n";
        let error = Leg2Stream::from_reader_with_budget(
            "leg2.jsonl",
            std::io::Cursor::new(posts),
            &mut budget,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            VerifierError::AggregateEvidenceTooLarge {
                path,
                records: 2,
                record_limit: 1,
                ..
            } if path == "leg2.jsonl"
        ));
    }
}
