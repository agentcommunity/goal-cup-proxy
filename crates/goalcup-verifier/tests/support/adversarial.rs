//! Build the adversarial trace deterministically from the committed Claude Code fixture.
//!
//! Reads `spikes/claude-code-cli/fixtures/` and writes a self-contained trace directory.
//! **Nothing under `spikes/` is ever written**.
//!
//! **This is a pure chain trace.** Every byte of evidence goes through
//! `goalcup_protocol::export::TraceExport` into one `chain.jsonl`: leg-1 records as
//! `EventBody::Leg1Record` envelopes, leg-2 posts as `EventBody::Leg2Post` envelopes,
//! heartbeats, the tail notice, and the seal as the last link. The descriptor is written by
//! the exporter and declares no `[[leg1]]` store and no `[leg2]` table, because there are
//! none: a store the descriptor names beside a chain is an off-chain side channel, and a
//! fixture that had one would be measuring a path the daemon does not produce.
//!
//! Four things are planted, and each one is the on-disk residue of a specific attack a
//! motivated participant would run on their own machine:
//!
//!   1. **A rewritten prefix below the last acknowledgement.** One record inside the
//!      acknowledged range has its text replaced and the chain is built over the result,
//!      so the trace recomputes cleanly end to end — and the server's hash at the
//!      acknowledged sequence no longer matches. It is the attack the whole `VISION.md`
//!      sentence rests on. Finding: `ack_chain_mismatch` (V12).
//!   2. **A truncated store.** A `TailNotice { kind: Truncated }` whose `offset_before` is
//!      the store's own length at the moment it was truncated and whose `offset_after` is
//!      0, no traced tool call naming the path, and no expected re-read declared anywhere.
//!      The attack: kill the daemon, work, truncate the store, restart. Finding:
//!      `store_rewritten` (V9).
//!   3. **A planted file nothing in the trace explains.** `planted.txt` appears in the
//!      seal and no traced call names it at all — not a write, and not even a read. Under a
//!      basename substring match a file like this would attribute cleanly to whatever call
//!      happened to mention a similar name. Finding:
//!      `unexplained_seal_file`, raised by `verify_trace` from `seal_attribution`.
//!
//!      Note what is deliberately **not** planted here: a read-class call naming
//!      `planted.txt`. That would make the file `Attribution::ReadOnly` and raise
//!      `read_only_attribution` (Flag) instead, which is a different — weaker — finding.
//!      The fixture isolates the `Fail` one, and `expected-verdict.json` names
//!      `read_only_attribution` in `must_not_contain` so the two can never be confused.
//!   4. **A heartbeat hole with records appended in it.** Heartbeats stop for twenty-one
//!      minutes (last one 07:44:00Z, next one 08:05:00Z) and **40** of `main`'s records
//!      carry harness timestamps inside the hole, with the
//!      acknowledgement stream stopping first. Finding: `dirty_gap` (V4).
//!
//!      The claim is a harness-timestamp claim on purpose, and `seq` could not make it.
//!      This generator chains all 165 leg-1 records first and the daemon's own eight
//!      heartbeats after them, so the 40 sit at chain seq 90-143 — below both the beat that
//!      closes the hole (seq 220) and the one that reopens it (seq 221), with nothing at
//!      all between those two in sequence. Read by `seq` the hole is empty; read by the
//!      harness's own clock the 40 are inside it. That is the reading
//!      `gaps::records_in_window` takes for an entry-chain store, and this fixture is what
//!      exercises it.
//!
//! **The chain starts with an empty baseline.** Seq 0 is an `EventBody::Baseline` link
//! over a `SealManifest` with no files: the workspace this run started in held nothing,
//! which is what makes `planted.txt` a planted file rather than one the directory already
//! had. It is written through `TraceExport::write_baseline`, so the digest it chains is
//! the digest of the bytes in `baseline.json`, and the descriptor names that file. Every
//! other link sits one seq higher, so `CLEAN_ACK_SEQ`, `ANCHOR_SEQ` and
//! `REWRITE_AFTER_SEQ` are chain sequences and the store index behind each one is looked
//! up rather than assumed.
//!
//! **Two leg-1 stores, not one.** `main` and `subagent` both appear in the chain, exactly
//! as the M0 claude-code-cli descriptor declares them, because the harness really wrote
//! both and a trace that hides one is not the trace under attack — it is a differently
//! incomplete one. Chaining only `main` would produce a spurious `leg2_tool_without_leg1_call`:
//! the hook stream posts a tool response whose call was made inside the subagent's own
//! file, so dropping that file would invent a leg-agreement failure with nothing to do with the
//! four planted attacks. The `a9-other-folder.jsonl` probe is deliberately absent
//! — it is the M0 set's foreign-session exhibit and belongs to V10's story, not to this one.
//!
//! **Every number in this file is derived, never invented.** The
//! heartbeat cursors are counts of records the daemon had chained; the `TailNotice`'s
//! `offset_before` is the store's real length in bytes; every link hash is stamped by
//! `TraceExport`; and both acknowledgements carry link hashes read back out of a chain.

use ed25519_dalek::Signer;
use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::chain::Link;
use goalcup_protocol::control::{format_z, parse_z};
use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::events::TailNoticeKind;
use goalcup_protocol::export::{
    ACKS_FILE, BASELINE_FILE, CHAIN_FILE, ExportError, SEAL_FILE, TraceDescriptorDoc, TraceExport,
};
use goalcup_protocol::seal::{SealManifest, SealedFile};
use std::path::Path;

use super::compliant::{records_cursor, remove_generator_output, repo_root, server_key};

/// Historical fixture bytes are a protocol-v1 compatibility control, independent of the
/// protocol version used by new exports.
const FIXTURE_PROTOCOL_VERSION: u32 = goalcup_protocol::LEGACY_PROTOCOL_VERSION;
/// The rewrite targets the first **assistant** record at or after this chain sequence.
///
/// `main` is chained first, above the baseline at seq 0, so a main record's chain seq is
/// its position in the harness's own file plus one. `doctor` is handed that base rather
/// than assuming it, so this constant keeps meaning a chain sequence however many links
/// the daemon writes before the transcript. Deliberately not a user-slot record: rewriting
/// a typed prompt would
/// change the human message list and V3's byte comparison, and this fixture must isolate
/// the *anchor* failure. The attack it models — quietly editing what the agent said or
/// did, below the last acknowledgement — is the more realistic one anyway.
pub const REWRITE_AFTER_SEQ: u64 = 30;
/// The last acknowledged sequence. The rewrite sits below it, which is the whole point.
pub const ANCHOR_SEQ: u64 = 100;
/// The acknowledgement below the rewrite, which must still reconcile: without it the
/// fixture would only show that *an* acknowledgement failed, not that the failure starts
/// exactly where the participant's edit does.
pub const CLEAN_ACK_SEQ: u64 = 20;
/// Seconds; matches `heartbeat_seconds` in the connector manifest. `classify` calls a
/// window a hole once it runs longer than `2 * HEARTBEAT`, so the planted hole is asserted
/// against this rather than against a hand-counted number of minutes.
pub const HEARTBEAT: u64 = 60;
/// An arbitrary but *fixed* inode for the synthetic store. Nothing reads its value; what
/// matters is that `inode_before == inode_after`, which is what makes the notice a
/// truncation rather than a replacement. It is a constant so it is obviously synthetic
/// rather than a measurement someone might try to reconcile with a real machine.
pub const SYNTHETIC_INODE: u64 = 8_612_004;
/// The entry this whole trace — descriptor, acknowledgements, envelopes and seal — belongs
/// to. One constant, because `verify_acks` rejects an acknowledgement whose `entry_id` is
/// not the trace's, and a typo would look exactly like the replay attack that gate exists
/// to catch.
pub const ENTRY_ID: &str = "adversarial-0001";
/// When the participant truncated `main`: the `TailNotice`'s `at` and the envelope's own
/// `recorded_at`, named once so the two cannot drift into two different moments.
const TRUNCATED_AT: &str = "2026-09-03T08:07:30Z";
/// When the entry was sealed — after the last heartbeat, because the seal closes the chain.
const SEALED_AT: &str = "2026-09-03T08:10:00Z";
/// When the workspace was measured: the moment the run's directory was created, which is
/// the timestamp the M0 capture's own workspace path carries. The baseline's `sealed_at`
/// and therefore the seq-0 link's `recorded_at`. It is a daemon envelope, so V13's window
/// scan does not measure it — only leg-1 records and leg-2 posts are placed in the window.
const BASELINE_AT: &str = "2026-09-03T07:30:54Z";
/// The run's workspace. Named once: the baseline, the seal and the descriptor all state
/// it, and V13 compares the descriptor's against the receipt's — a typo in one of three
/// copies would read as the substituted frame this fixture is not modelling.
const WORKSPACE: &str = "/Users/team/goalcup-spikes/ws-claude-code-cli-20260903T073054Z";
/// The files this trace no longer holds. A regenerated directory must not keep a retired
/// input beside the chain that replaced it: `trace.toml` names none of them, so a reader
/// who found one would have to guess whether the verifier had read it.
const RETIRED: [&str; 4] = [
    "main.jsonl",
    "subagent.jsonl",
    "hooks.jsonl",
    "events.jsonl",
];

/// One leg-1 record as the daemon chained it.
struct Observed {
    /// The harness's own line, byte for byte — doctored, for the one record the
    /// participant rewrote.
    line: String,
    /// The record's own timestamp, where the harness wrote one. This is what places the
    /// record inside or outside a heartbeat hole.
    harness_at: Option<u64>,
    /// When the daemon observed it, which is the envelope's `recorded_at`.
    observed_at: u64,
}

/// Read one store's lines and date each one twice: by the harness's clock, and by the
/// daemon's.
///
/// **The `observed_at` carve-out.** 46 of `main`'s 148 records carry no timestamp of their
/// own, and every envelope carries a `recorded_at` — a daemon that chained a record knows
/// when it did so, even when the record says nothing about when it was written. So a
/// record with no harness timestamp is dated one second after the last harness timestamp
/// the daemon had seen (or, before it has seen one, one second after the store's first).
/// That is a **daemon fact, not a harness byte**: it is this generator standing in for a
/// tailer's own clock, and nothing downstream reads it as the harness's word. The rule
/// that decides holes reads the harness timestamp and falls back to the chain's own order,
/// never to this field — see `gaps::records_in_window`.
fn observe(lines: &[String]) -> Vec<Observed> {
    let harness: Vec<Option<u64>> = lines
        .iter()
        .map(|l| {
            serde_json::from_str::<serde_json::Value>(l)
                .ok()
                .and_then(|v| {
                    v.get("timestamp")
                        .and_then(|t| t.as_str())
                        .and_then(parse_z)
                })
        })
        .collect();
    let first = harness.iter().flatten().copied().next();
    let mut last: Option<u64> = None;
    let mut out = Vec::with_capacity(lines.len());
    for (line, harness_at) in lines.iter().zip(harness.iter()) {
        let observed_at = match harness_at {
            Some(t) => {
                last = Some(*t);
                *t
            }
            None => last.or(first).expect("the store has a timestamped record") + 1,
        };
        out.push(Observed {
            line: line.clone(),
            harness_at: *harness_at,
            observed_at,
        });
    }
    out
}

/// How many of this store's records the daemon had chained when it wrote the beat at
/// `beat`.
///
/// Strictly before, never at: a record stamped in the beat's own second may not have
/// reached the chain by the time the beat was written, and a cursor must not claim a record
/// the daemon may not yet have had. This is the only source of a cursor number in this
/// file, and it is a **count of records** rather than a byte offset because a chain-derived
/// store is not a file with an offset into it — the unit is the one
/// `gaps::records_in_window` counts, so both sides of the cursor comparison measure the
/// same thing.
fn cursor_at(records: &[Observed], beat: u64) -> usize {
    records.iter().filter(|r| r.observed_at < beat).count()
}

/// Every non-empty line of one committed store, in order.
fn store_lines(name: &str) -> Vec<String> {
    let path = repo_root()
        .join("spikes/claude-code-cli/fixtures")
        .join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect()
}

/// Append one envelope to the entry's chain and return the link the writer stamped.
///
/// The link is `TraceExport`'s to stamp, never this generator's: a fixture that hand-rolled
/// `seq`, `prev` and `hash` would be asserting agreement with a formula it wrote itself
/// rather than reading the one the daemon's writer uses.
fn push(export: &mut TraceExport, source: &str, recorded_at: &str, body: EventBody) -> Link {
    let mut env = Envelope {
        protocol_version: FIXTURE_PROTOCOL_VERSION,
        entry_id: ENTRY_ID.to_string(),
        source: source.to_string(),
        recorded_at: recorded_at.to_string(),
        link: Link {
            seq: 0,
            prev: String::new(),
            hash: String::new(),
        },
        body,
        redaction: None,
    };
    export
        .append_chain(&mut env)
        .expect("the chain writer stamps every link")
}

/// Chain one store's records and return each one's link hash, in seq order.
///
/// `source` is `"daemon"` for every one of them: `source` names the component that emitted
/// the line, and the daemon's tailer is what read these files. The store is named in the
/// body, which is where `chain::load_and_verify` reads it from.
fn chain_records(export: &mut TraceExport, store: &str, records: &[Observed]) -> Vec<Link> {
    records
        .iter()
        .map(|r| {
            push(
                export,
                "daemon",
                &format_z(r.observed_at),
                EventBody::Leg1Record {
                    store: store.to_string(),
                    // A line-oriented store's payload is the harness's own line as a JSON
                    // *string*. There is no `{path,text}` wrapper anywhere.
                    payload: serde_json::Value::String(r.line.clone()),
                },
            )
        })
        .collect()
}

/// The hash of the link this store's records carry at chain sequence `seq`.
///
/// Looked up rather than indexed. A store's records no longer start at seq 0 — the
/// baseline does — so `links[seq]` would silently name the record one place too early, and
/// an acknowledgement built that way would fail for a reason that has nothing to do with
/// the planted rewrite.
fn hash_at(links: &[Link], seq: u64) -> String {
    links
        .iter()
        .find(|l| l.seq == seq)
        .unwrap_or_else(|| panic!("no link at chain seq {seq}"))
        .hash
        .clone()
}

/// The `SealManifest` this run's workspace started from: nothing.
///
/// The whole fixture rests on it. `planted.txt` is an attack because the workspace was
/// empty when the run was declared; against a baseline that already listed it, it would be
/// a file the directory came with, and `unexplained_seal_file` would be wrong to raise.
fn empty_baseline() -> SealManifest {
    SealManifest {
        entry_id: ENTRY_ID.to_string(),
        sealed_at: BASELINE_AT.to_string(),
        workspace: WORKSPACE.to_string(),
        files: Vec::new(),
        excluded: Vec::new(),
    }
    .sorted()
}

/// The doctored `main`, and the chain sequence of the record the participant rewrote.
///
/// The same record shape, different content. A reviewer reading the chain sees
/// an ordinary line; only the acknowledgement disagrees. The target is the first assistant
/// record at or after `REWRITE_AFTER_SEQ` — never a user-slot record, so the human-message
/// list is untouched and the fixture isolates the anchor failure.
///
/// `base` is the chain sequence this store's first record carries — 1, above the baseline
/// at seq 0 — so `REWRITE_AFTER_SEQ`, `CLEAN_ACK_SEQ` and `ANCHOR_SEQ` are all read in one
/// space and the three assertions in `generate` compare like with like.
///
/// One honest limitation, recorded so nobody later reads it as an oversight: `serde_json`
/// is taken without `preserve_order`, so re-serialising the record sorts its keys and the
/// doctored line is visibly reordered against the source fixture. A real participant would
/// preserve key order. That costs this fixture nothing — the verifier hashes bytes and has
/// no opinion about key order, so the acknowledgement at `ANCHOR_SEQ` fails for exactly the
/// same reason either way — and it buys a reviewer a doctored line they can find by eye.
fn doctor(lines: &[String], base: u64) -> (Vec<String>, u64) {
    let mut out = Vec::with_capacity(lines.len());
    let mut rewritten_at: Option<u64> = None;
    for (i, line) in lines.iter().enumerate() {
        let seq = base + i as u64;
        let v: serde_json::Value = serde_json::from_str(line).expect("valid record");
        let is_assistant = v.get("type").and_then(|t| t.as_str()) == Some("assistant");
        if rewritten_at.is_none() && seq >= REWRITE_AFTER_SEQ && is_assistant {
            rewritten_at = Some(seq);
            let mut v = v.clone();
            v["message"]["content"] = serde_json::json!([{
                "type": "text",
                "text": "[rewritten by the participant after the fact]"
            }]);
            out.push(serde_json::to_string(&v).unwrap());
        } else {
            out.push(line.clone());
        }
    }
    (
        out,
        rewritten_at.expect("the Claude fixture has assistant records"),
    )
}

/// The link hashes the entry's chain would have carried had the participant not rewritten
/// anything: the honest bytes, through the same writer, in a directory that is thrown away.
///
/// Built rather than asserted, because a link hash is a function of every link below it —
/// the two chains agree up to the rewrite and diverge for ever after — and the only honest
/// way to know the hash the server signed at `ANCHOR_SEQ` is to run the honest bytes
/// through `TraceExport`, exactly as the daemon would have.
fn honest_links(records: &[Observed]) -> Vec<Link> {
    let scratch = tempfile::tempdir().expect("a scratch directory for the honest chain");
    let mut export = TraceExport::create(scratch.path()).expect("open the scratch export");
    // The same baseline, first, byte for byte. A link hash is a function of every link
    // below it, so an honest chain that skipped seq 0 would diverge from the real one at
    // the very first record and the comparison below would prove nothing.
    export
        .write_baseline_for_protocol(FIXTURE_PROTOCOL_VERSION, &empty_baseline())
        .expect("chain the honest baseline");
    chain_records(&mut export, "main", records)
}

fn remove_previous_outputs(out: &Path) -> std::io::Result<()> {
    // Keep trace.toml: TraceExport owns atomic descriptor replacement. These are the
    // generator's exact current and retired outputs, never a directory sweep.
    for name in [CHAIN_FILE, ACKS_FILE, BASELINE_FILE, SEAL_FILE] {
        remove_generator_output(out, name)?;
    }
    for name in RETIRED {
        remove_generator_output(out, name)?;
    }
    Ok(())
}

/// Write the whole adversarial trace directory into `out`. Returns nothing: every file is
/// a pure function of the committed input, so the caller compares bytes rather than
/// trusting a return value.
pub fn generate(out: &Path) {
    std::fs::create_dir_all(out).expect("create the output directory");
    remove_previous_outputs(out).expect("remove the generator's previous outputs");
    let mut export = TraceExport::create(out).expect("open the trace export");

    // ---- the baseline, chained first: seq 0, and empty, because this run's workspace
    // started with nothing in it. That is what makes `planted.txt` below a planted file
    // rather than one the directory came with. `write_baseline` chains the link and writes
    // the file, so the digest in the chain is the digest of the bytes in `baseline.json`
    // and a participant cannot rewrite the list afterwards without moving seq 0 — the link
    // a start receipt's `chain_head` witnesses.
    let (_, baseline) = export
        .write_baseline_for_protocol(FIXTURE_PROTOCOL_VERSION, &empty_baseline())
        .expect("chain and write the baseline");
    assert_eq!(baseline.seq, 0, "the baseline is the genesis link");
    // The chain sequence this store's first record carries. Read off the writer rather
    // than assumed, so a link added above the transcript later moves every derived number
    // with it instead of silently invalidating three constants.
    let base = export.head().expect("the baseline is chained").0 + 1;

    // ---- leg 1: the Claude transcript, with one record inside the anchor rewritten.
    let source = store_lines("main-session.jsonl");
    let (doctored, rewritten_at) = doctor(&source, base);
    let main = observe(&doctored);
    // The store's length at the moment the participant truncated it: the sum of the bytes
    // the harness actually wrote, not a number chosen to look plausible.
    let store_len: u64 = main.iter().map(|r| r.line.len() as u64 + 1).sum();
    assert!(
        main.len() as u64 + base > ANCHOR_SEQ,
        "the source store has {} records from chain seq {base}, not enough to reach \
         ANCHOR_SEQ = {ANCHOR_SEQ}",
        main.len()
    );
    assert!(
        rewritten_at < ANCHOR_SEQ,
        "the rewrite must sit BELOW the last acknowledgement, or the fixture proves nothing"
    );
    assert!(
        CLEAN_ACK_SEQ < rewritten_at,
        "the reconciling acknowledgement must sit below the rewrite, or it would fail too"
    );
    let honest = honest_links(&observe(&source));
    let doctored_links = chain_records(&mut export, "main", &main);

    // The fixture's whole claim, asserted at generation time so a change to the input
    // cannot quietly turn the adversarial trace into an honest one: the chain agrees with
    // the server below the rewrite and disagrees above it. Both sides are looked up by
    // chain sequence, which is the space the two acknowledgements name.
    assert_eq!(
        hash_at(&doctored_links, CLEAN_ACK_SEQ),
        hash_at(&honest, CLEAN_ACK_SEQ),
        "seq {CLEAN_ACK_SEQ} is below the rewrite, so it must still match"
    );
    assert_ne!(
        hash_at(&doctored_links, ANCHOR_SEQ),
        hash_at(&honest, ANCHOR_SEQ),
        "seq {ANCHOR_SEQ} is above the rewrite, so it must NOT match"
    );

    // ---- leg 1, second store: the subagent's own file, chained verbatim. Nothing was
    // tampered with here; it is in the chain because the harness wrote it, and because the
    // hook stream posts a tool response whose call lives in this file. Dropping it invents
    // a `leg2_tool_without_leg1_call` that has nothing to do with the planted attacks.
    let subagent = observe(&store_lines("subagent-a8ca2ea8d4a8a1673.jsonl"));
    // Every subagent record is timestamped, so every one of them is placed by the harness's
    // own clock and none of them falls back to chain order.
    assert!(
        subagent.iter().all(|r| r.harness_at.is_some()),
        "the subagent store is expected to be fully timestamped"
    );
    chain_records(&mut export, "subagent", &subagent);

    // ---- leg 2: the hook stream, chained verbatim. The receiver's own path and body are
    // what a `Leg2Post` envelope carries; the transport metadata the M0 capture recorded
    // beside them (method, headers) has no home in the protocol and is not invented one
    // here. `received_at` becomes the envelope's clock, to the second — see `format_z`.
    for line in store_lines("hooks.jsonl") {
        let raw: serde_json::Value = serde_json::from_str(&line).expect("a hook post");
        let at = raw
            .get("t")
            .and_then(|t| t.as_f64())
            .expect("every hook post carries its receipt time") as u64;
        push(
            &mut export,
            "hooks",
            &format_z(at),
            EventBody::Leg2Post {
                // The receiver path verbatim, leading slash and any `-cmd` suffix
                // included: the suffix is what tells the two transports apart, and
                // duplicate pairing depends on it.
                event: raw
                    .get("path")
                    .and_then(|p| p.as_str())
                    .expect("every hook post names a path")
                    .to_string(),
                payload: raw.get("body").cloned().unwrap_or(serde_json::Value::Null),
                // This fixture puts the raw path in `event` (above), so it needs no
                // separate one; a chain the daemon writes carries the event's own name
                // there and the path here.
                path: None,
                receiver_meta: None,
            },
        );
    }

    // ---- the daemon's own stream: heartbeats with a hole, then the tail notice.

    // Heartbeats every minute from 07:40 to 07:44, then a twenty-one-minute hole
    // (07:44:00Z → 08:05:00Z), then heartbeats again.
    let before_hole: Vec<String> = (0..5u64)
        .map(|minute| format!("2026-09-03T07:{:02}:00Z", 40 + minute))
        .collect();
    let after_hole: Vec<String> = (0..3u64)
        .map(|minute| format!("2026-09-03T08:{:02}:00Z", 5 + minute))
        .collect();
    // The hole is measured from the strings actually written, through the same parser the
    // classifier uses, rather than eyeballed from the comment above: `classify` only calls
    // a window a hole once it runs longer than two heartbeat intervals, and a fixture whose
    // hole quietly stopped being one would still pass every other assertion here.
    let hole_seconds = parse_z(&after_hole[0]).expect("the first beat after the hole parses")
        - parse_z(before_hole.last().unwrap()).expect("the last beat before the hole parses");
    assert!(
        hole_seconds > 2 * HEARTBEAT,
        "the hole is {hole_seconds}s and has to exceed two {HEARTBEAT}s heartbeat intervals"
    );

    // Every beat reports both stores, because the daemon tailed both, and each cursor is
    // `cursor_at` over the records this generator just chained — never a number chosen to
    // look plausible.
    //
    // …and in between, twenty-one minutes of nothing, during which the harness kept
    // writing: 40 of `main`'s records carry timestamps after 07:44:00Z, and the daemon saw
    // none of them until it came back.
    for ts in before_hole.iter().chain(after_hole.iter()) {
        let beat = parse_z(ts).expect("a heartbeat timestamp this generator wrote parses");
        push(
            &mut export,
            "daemon",
            ts,
            EventBody::Heartbeat {
                ts: ts.clone(),
                stores_seen: vec![
                    records_cursor("main", cursor_at(&main, beat), "rowid"),
                    records_cursor("subagent", cursor_at(&subagent, beat), "rowid"),
                ],
                receiver_ok: true,
            },
        );
    }
    // The store was truncated below the saved offset, and nothing the agent
    // did explains it. `offset_before` is the store's real length in bytes and
    // `offset_after` is 0, so the drop is unambiguous. The test recomputes the expected
    // number from this envelope rather than repeating a literal.
    push(
        &mut export,
        "daemon",
        TRUNCATED_AT,
        EventBody::TailNotice {
            store: "main".to_string(),
            path: "{config_dir}/projects/ws/95addb64.jsonl".to_string(),
            kind: TailNoticeKind::Truncated,
            offset_before: store_len,
            offset_after: 0,
            // The inode is unchanged: truncated in place, not replaced. `FileReplaced` is
            // the other `TailNoticeKind` and this fixture is deliberately not that one.
            inode_before: Some(SYNTHETIC_INODE),
            inode_after: Some(SYNTHETIC_INODE),
            at: TRUNCATED_AT.to_string(),
        },
    );

    // ---- the seal: one file the agent wrote, one nothing in the trace explains. Chained
    // last, by `write_seal`, which closes the chain — no record may follow a seal, and the
    // writer is what enforces that rather than the order of the lines above.
    let seal = SealManifest {
        entry_id: ENTRY_ID.to_string(),
        sealed_at: SEALED_AT.to_string(),
        workspace: WORKSPACE.to_string(),
        files: vec![
            SealedFile {
                path: "hello.py".to_string(),
                bytes: 4,
                sha256: "aa".repeat(32),
            },
            // Nothing in the trace names this file at all.
            SealedFile {
                path: "planted.txt".to_string(),
                bytes: 3,
                sha256: "cc".repeat(32),
            },
        ],
        excluded: vec![".claude/settings.json".to_string()],
    }
    .sorted();
    export
        .write_seal_for_protocol(FIXTURE_PROTOCOL_VERSION, &seal)
        .expect("chain and write the seal");
    // Stated as a refusal rather than as a comment: the seal is the last link, and this is
    // the writer saying so about the chain that was just written.
    let mut after_the_seal = Envelope {
        protocol_version: FIXTURE_PROTOCOL_VERSION,
        entry_id: ENTRY_ID.to_string(),
        source: "daemon".to_string(),
        recorded_at: SEALED_AT.to_string(),
        link: Link {
            seq: 0,
            prev: String::new(),
            hash: String::new(),
        },
        body: EventBody::Leg1Record {
            store: "main".to_string(),
            payload: serde_json::Value::String("{}".to_string()),
        },
        redaction: None,
    };
    assert!(
        matches!(
            export.append_chain(&mut after_the_seal),
            Err(ExportError::Sealed { .. })
        ),
        "the seal closes the chain: nothing may be appended after it"
    );

    // ---- acknowledgements: what the server durably stored, signed with the fixed key.
    //
    // Both name a position in the entry's one chain, which is what an acknowledgement
    // names. The clean one carries the hash the chain actually holds; the one at
    // ANCHOR_SEQ carries the hash the honest chain would have held there — the record as
    // the server saw it, before the participant edited it.
    let key = server_key();
    for (seq, hash) in [
        (CLEAN_ACK_SEQ, hash_at(&doctored_links, CLEAN_ACK_SEQ)),
        (ANCHOR_SEQ, hash_at(&honest, ANCHOR_SEQ)),
    ] {
        let mut a = AckCheckpoint {
            entry_id: ENTRY_ID.to_string(),
            seq,
            hash,
            acked_at: "2026-09-03T07:45:00Z".to_string(),
            server_signature: String::new(),
        };
        a.server_signature = hex::encode(key.sign(&a.signing_bytes().unwrap()).to_bytes());
        export.append_ack(&a).expect("append the acknowledgement");
    }

    // ---- the descriptor, written by the exporter. No `[[leg1]]`, no `[leg2]`: this trace
    // declares one chain, and everything either leg holds is in it.
    export
        .write_descriptor(&TraceDescriptorDoc {
            connector: "claude-code-cli".to_string(),
            entry_id: ENTRY_ID.to_string(),
            workspace: WORKSPACE.to_string(),
            tracked_sessions: vec!["95addb64-9dac-4046-8ef6-8aa53fa0c7f1".to_string()],
            harness_version: "2.1.259".to_string(),
            connector_manifest: "../../../../../connectors/claude-code-cli/manifest.toml"
                .to_string(),
            chain: Some(CHAIN_FILE.to_string()),
            receipt: None,
            baseline: Some(BASELINE_FILE.to_string()),
            acks: Some(ACKS_FILE.to_string()),
            seal: Some(SEAL_FILE.to_string()),
            // The verifying key of the seed `[5u8; 32]`, which is the one entry in the
            // verifier's own `SERVER_PUBLIC_KEYS` allowlist. So this run is
            // `key_source = "pinned"`: the signatures are genuinely checked, and the
            // acknowledgement that contradicts the chain is evidence rather than a
            // self-signed note.
            server_public_key: Some(hex::encode(key.verifying_key().to_bytes())),
            // Not an expected re-read: that is the point. A truncation the trace cannot
            // account for is what V9 reports as `store_rewritten`.
            expected_reread: false,
        })
        .expect("write the descriptor");
}

/// The files `generate` writes, in a fixed order, for byte-comparison.
pub const FILES: [&str; 5] = [
    "trace.toml",
    "chain.jsonl",
    "acks.jsonl",
    "baseline.json",
    "seal.json",
];
