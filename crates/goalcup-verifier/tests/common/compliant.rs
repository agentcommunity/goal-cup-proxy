//! Build the compliant trace deterministically. The pass path's fixture.
//!
//! Every other fixture in this crate measures something going wrong: the three M0 captures
//! are of a deliberately non-compliant script, and `adversarial.rs` plants four attacks. So
//! this file is what asserts that a run which broke **no** rule reads as a run that broke
//! no rule: it makes `Severity::Pass` and exit 0 reachable over real bytes rather than only
//! from a hand-built `Verdict` in a unit test.
//!
//! **A pure chain trace**, the same shape `adversarial.rs` produces and the same writer:
//! every byte of evidence goes through `goalcup_protocol::export::TraceExport` into one
//! `chain.jsonl`. Leg-1 records are `Leg1Record` envelopes, leg-2 posts are `Leg2Post`
//! envelopes, the baseline is the genesis link and the seal is the last one. The
//! descriptor declares no `[[leg1]]` store and no `[leg2]` table, because a store named
//! beside a chain is an off-chain side channel and `TraceDescriptor::load` refuses one.
//!
//! **Nothing is read from `spikes/`.** The adversarial fixture derives from a real capture
//! because its claim is about real bytes; this one's claim is about the *rules*, so every
//! record is written here, in full, where a reader can check that it is compliant by
//! reading it rather than by trusting a transcript.
//!
//! What the run is, end to end:
//!
//!   * The workspace starts with one file, `README.md`, and the baseline says so — at
//!     seq 0, which is the link the start receipt's `chain_head` witnesses.
//!   * The participant types the goal. Leg 1 records it, the hook stream posts it, and the
//!     two agree byte for byte on `prompt_id` (V1, V3, V7).
//!   * The agent answers with one `Write` call that creates `hello.py`. Leg 1 carries the
//!     `tool_use` block, leg 2 carries the `PreToolUse` post, and both name `toolu_01`, so
//!     the declared `prompt_id` join actually joins (V3, V8).
//!   * The participant types `keep going`, and that is the only thing they type after the
//!     goal (V2).
//!   * One harness-authored `task-notification` record occupies the user slot and fails
//!     the connector's positive test (`origin.kind` is not `human`), which keeps it out of
//!     the human list (V6), which names `failed_positive_test` as the reason. It is here on
//!     purpose: `user_slot_excluded` is a Pass-level finding, and a compliant fixture that
//!     never raised it would not show that the sentence survives.
//!   * The daemon reports in twice a day for the seven days of the entry window, and the
//!     server acknowledges every report. Its declared `heartbeat_seconds` is 60, so each
//!     half-day silence is a hole — and every one of them is clean: nothing was appended
//!     to any store inside it, the daemon's own cursor did not move, and an acknowledgement
//!     covers its far edge. That is D9's definition of accounted-for downtime, and
//!     `clean_gap` is a Pass-level finding this fixture exists to exhibit.
//!   * The entry seals. `hello.py` attributes to the traced `Write` call and `README.md` to
//!     the baseline, so the seal explains every file it holds.
//!
//! **Every number is derived, never invented.** The link hashes are `TraceExport`'s; the
//! heartbeat cursors are counts of the records this generator actually chained; each
//! acknowledgement carries a link hash read back out of the chain; and the receipt's
//! `manifest_sha256` is the digest of the manifest bytes written beside it.

use ed25519_dalek::{Signer, SigningKey};
use goalcup_protocol::ack::AckCheckpoint;
use goalcup_protocol::chain::Link;
use goalcup_protocol::control::{format_z, parse_z};
use goalcup_protocol::cursor::{CursorUnit, StoreCursor};
use goalcup_protocol::envelope::{Envelope, EventBody};
use goalcup_protocol::events::TailNoticeKind;
use goalcup_protocol::export::{
    ACKS_FILE, BASELINE_FILE, CHAIN_FILE, MANIFEST_FILE, RECEIPT_FILE, SEAL_FILE,
    TraceDescriptorDoc, TraceExport,
};
use goalcup_protocol::receipt::{StartReceipt, StartReceiptClaims};
use goalcup_protocol::seal::{SealManifest, SealedFile};
use serde_json::json;
use std::path::{Path, PathBuf};

/// The entry this whole trace belongs to. One constant: `verify_acks` rejects an
/// acknowledgement whose `entry_id` is not the trace's, and V13 compares the descriptor's
/// against the receipt's.
pub const ENTRY_ID: &str = "compliant-0001";
/// The run's workspace. Named once — the baseline, the seal, the receipt, the descriptor
/// and every leg-1 record's `cwd` all state it, and V10 fails a tracked session whose
/// directory is not this exact string.
pub const WORKSPACE: &str = "/ws/compliant-0001";
/// When the server declared the run started. The receipt's `started_at`, and the lower
/// edge of the window V13 places every record and post against.
pub const STARTED_AT: &str = "2026-09-01T00:00:00Z";
/// When the entry sealed: the seal's `sealed_at`, the receipt's `deadline_at`, and the
/// upper edge of that window. Seven days after the start, which is the entry window.
pub const SEALED_AT: &str = "2026-09-08T00:00:00Z";
/// Fixed seed, so the server key and every signature are the same on every machine.
///
/// **`[5u8; 32]` on purpose**: the verifying key of that seed is
/// `goalcup_verifier::PLACEHOLDER_SERVER_PUBLIC_KEY`, the single entry in
/// `SERVER_PUBLIC_KEYS` and the allowlist a default `verify_trace` run checks against.
/// Signing with any other seed would leave `key_source` as `none`, V12 would report
/// `anchor_unverified` and V13 `receipt_unverified` — both Flags — and this trace would
/// not reach Pass however compliant its contents were.
///
/// Because the seed is written here, a default run also raises V13's
/// `placeholder_server_key`: anybody holding this repository can sign under it, so a
/// `key_source` of `pinned` says only that the signature matched a key everyone has. That
/// Flag is suppressed by `--dev`, which is what the pass assertions in
/// `tests/it/compliant.rs` pass, and the committed `expected-verdict.json` records the
/// reading without it.
pub const SERVER_SEED: [u8; 32] = [5u8; 32];

#[derive(Debug, Clone, Copy)]
enum Generation {
    Current,
    Historical,
}

impl Generation {
    fn protocol_version(self) -> u32 {
        match self {
            Self::Current => goalcup_protocol::PROTOCOL_VERSION,
            Self::Historical => goalcup_protocol::LEGACY_PROTOCOL_VERSION,
        }
    }

    fn connector_dir(self) -> PathBuf {
        match self {
            Self::Current => repo_root().join("connectors").join(CONNECTOR),
            Self::Historical => PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/common/fixtures/compliant-connector-v1"),
        }
    }
}

/// The files either generation mode writes, in a fixed order, for byte-comparison.
pub const FILES: [&str; 7] = [
    "trace.toml",
    "chain.jsonl",
    "acks.jsonl",
    "receipt.json",
    "baseline.json",
    "seal.json",
    "manifest.toml",
];

/// The connector this run was judged by. Its manifest is copied in beside the chain (see
/// `write_manifest`), so the trace is self-contained. Current generation resolves to the
/// current pin; historical generation deliberately preserves the older input bytes.
const CONNECTOR: &str = "claude-code-cli";
/// Inside `tested_version_range = ">=2.1.259"`, so V5 reports no version drift.
const HARNESS_VERSION: &str = "2.1.259";
/// The one session this entry tracks. The receipt signs it, the descriptor lists it, and
/// every record carries it.
const SESSION_ID: &str = "compliant-session-0001";
/// The store the transcript is chained under. `main` is the name the connector manifest
/// gives the transcript, so the heartbeat cursors and any tail notice name a store the
/// verifier actually has.
const STORE: &str = "main";
/// The goal, under `MAX_PROMPT_CHARS`.
const GOAL: &str = "write hello.py that prints the current date";
/// Every human message after the goal must be exactly this (V2, RULES.md).
const KEEP_GOING: &str = "keep going";
/// The file the workspace started with, and its digest in the baseline.
///
/// Obviously synthetic, and deliberately so: this fixture ships no workspace, so nothing
/// hashes any bytes to produce it, and a realistic-looking digest would invite a reader to
/// go looking for the file it came from. What matters to `attribute_seal` is that the
/// baseline's entry and the seal's entry for `README.md` are the same string, which is why
/// there is one constant rather than two literals.
const README_SHA256: &str = "1111111111111111111111111111111111111111111111111111111111111111";
/// The file the agent wrote, and its digest in the seal. Different from the baseline's, so
/// `hello.py` cannot attribute as a file the workspace already held.
const HELLO_SHA256: &str = "2222222222222222222222222222222222222222222222222222222222222222";
/// How often the daemon reports in, and how often the server acknowledges. Twelve hours,
/// which is longer than twice the connector's declared `heartbeat_seconds` — so every
/// interval is a hole, and every hole is clean.
///
/// **Twelve-hourly rather than hourly.** An hourly cadence over a seven-day window would
/// put 167 `clean_gap` findings into the verdict, almost all of it one sentence repeated,
/// which would make the fixture's own document unreadable. The
/// property being exhibited is that a hole the trace positively accounts for reads as
/// clean; thirteen of them exhibit it exactly as well as a hundred and sixty-seven, and
/// twelve hours still clears V12's 24-hour limit on the span between verified
/// acknowledgements with half the interval to spare.
const BEAT_INTERVAL: u64 = 43_200;
/// How many reports the week holds: the first at `STARTED_AT` + one minute, then every
/// twelve hours, the last comfortably inside the window and less than a day before the
/// seal. Fourteen reports leave thirteen holes between consecutive reports, and a
/// fifteenth would fall past `SEALED_AT`.
const BEATS: u64 = 14;

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn server_key() -> SigningKey {
    SigningKey::from_bytes(&SERVER_SEED)
}

/// What this generator is asked to produce.
///
/// The compliant trace is `Plan::compliant()`. The two variants exist for one test —
/// `expected_reread_without_a_restart_does_not_downgrade_a_rewrite` — and they differ from
/// the compliant trace in exactly the two bytes the test is about: the descriptor's
/// `expected_reread`, and whether the chain carries a witness for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    /// The descriptor's top-level `expected_reread`, which is where a chain trace declares
    /// it — a chain descriptor has no `[[leg1]]` to carry the per-store flag.
    pub expected_reread: bool,
    /// Plant a `TailNotice { kind: Truncated }` for `main` above the heartbeats.
    pub truncation: bool,
    /// Chain a `DaemonRestart` immediately below that notice, which is the corroboration
    /// the verifier requires before `expected_reread` is honoured.
    pub restart: bool,
    /// Chain the launch's `EventBody::HomeSeeded` above the baseline, the
    /// way `register_process` does on a real start. `false` for the committed fixture, so
    /// its bytes and its verdict are unaffected.
    pub home_seeded: bool,
}

impl Plan {
    /// The compliant base shape: no truncation, nothing declared, nothing to corroborate.
    pub fn compliant() -> Plan {
        Plan {
            expected_reread: false,
            truncation: false,
            restart: false,
            home_seeded: false,
        }
    }
}

/// Write a current compliant trace into `out`.
///
/// It uses the current wire version and current shipped connector inputs.
pub fn generate(out: &Path) {
    generate_with(out, Plan::compliant(), Generation::Current);
}

/// Reproduce the committed protocol-v1 fixture from its frozen connector inputs.
pub fn generate_historical(out: &Path) {
    generate_with(out, Plan::compliant(), Generation::Historical);
}

/// What this run's launch carried out of the participant's own `~/.claude.json`.
///
/// The browser-tools switch is a boolean and carries its value; `oauthAccount` is an object
/// holding an account and is named only, which is the protocol's shape rule
/// (`SeededKey::new`) and not a choice this fixture makes. `written` is the one entry the
/// tracker decides rather than copies — this run's workspace, marked trusted.
pub fn home_seed_record() -> goalcup_protocol::events::HomeSeedRecord {
    use goalcup_protocol::events::{HomeSeedRecord, SeededKey};
    HomeSeedRecord {
        handoff_generation: None,
        handoff_request_digest: None,
        connector: CONNECTOR.to_string(),
        from: "{home}/.claude.json".to_string(),
        keys: vec![
            SeededKey::new("hasCompletedOnboarding", &json!(true)),
            SeededKey::new("theme", &json!("dark")),
            SeededKey::new("claudeInChromeDefaultEnabled", &json!(true)),
            SeededKey::new(
                "oauthAccount",
                &json!({"emailAddress": "someone@example.test"}),
            ),
        ],
        keys_unchanged: Vec::new(),
        written: vec![SeededKey::new("hasTrustDialogAccepted", &json!(true))],
        files: Vec::new(),
        files_kept: Vec::new(),
        files_missing: Vec::new(),
        wire_layout: Default::default(),
    }
}

/// Append one envelope and return the link the writer stamped.
///
/// The link is `TraceExport`'s to stamp, never this generator's: a fixture that hand-rolled
/// `seq`, `prev` and `hash` would assert agreement with a formula it wrote itself rather
/// than reading the one the daemon's writer uses.
fn push(
    export: &mut TraceExport,
    protocol_version: u32,
    source: &str,
    recorded_at: &str,
    body: EventBody,
) -> Link {
    let mut env = Envelope {
        protocol_version,
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

/// One leg-1 record, chained as the daemon's tailer would ship it: the harness's own line
/// as a JSON **string**, under the store the body names.
fn leg1(
    export: &mut TraceExport,
    protocol_version: u32,
    at: &str,
    line: &serde_json::Value,
) -> Link {
    push(
        export,
        protocol_version,
        "daemon",
        at,
        EventBody::Leg1Record {
            store: STORE.to_string(),
            payload: serde_json::Value::String(line.to_string()),
        },
    )
}

/// One leg-2 post, named by the receiver path verbatim — leading slash included, because
/// `Leg2Post::transport()` reads the path's `-cmd` suffix and duplicate pairing depends on
/// telling the two transports apart. This run posts everything over the http hook.
fn leg2(
    export: &mut TraceExport,
    protocol_version: u32,
    at: &str,
    event: &str,
    payload: serde_json::Value,
) -> Link {
    push(
        export,
        protocol_version,
        "hooks",
        at,
        EventBody::Leg2Post {
            event: event.to_string(),
            payload,
            path: None,
            receiver_meta: None,
        },
    )
}

/// The heartbeat cursor for one store: a count of records, declared as one.
///
/// `CursorUnit::Records` is not decoration. `gaps::cursor_delta` compares a cursor's
/// movement against a count of records in a window only under that unit, so a cursor
/// claiming any other unit would switch off the corroboration silently. `label` is the
/// prefix of the human `describe` line in the chain (`rowid:7`, `records:7`): it is part of
/// the chained bytes, so each generator keeps the spelling its committed fixture was
/// written with.
pub fn records_cursor(store: &str, position: usize, label: &str) -> StoreCursor {
    StoreCursor {
        store: store.to_string(),
        unit: CursorUnit::Records,
        position: Some(position as u64),
        describe: format!("{label}:{position}"),
    }
}

/// The baseline: what the workspace held when the run was declared.
///
/// One file, so the seal has something to attribute to the baseline as well as something to
/// attribute to a traced call — a fixture whose baseline was empty would exercise only half
/// of `attribute_seal`.
fn baseline() -> SealManifest {
    SealManifest {
        entry_id: ENTRY_ID.to_string(),
        sealed_at: STARTED_AT.to_string(),
        workspace: WORKSPACE.to_string(),
        files: vec![SealedFile {
            path: "README.md".to_string(),
            bytes: 12,
            sha256: README_SHA256.to_string(),
        }],
        excluded: Vec::new(),
    }
    .sorted()
}

/// The seal: what the workspace held at the deadline.
///
/// `README.md` is byte-identical to the baseline's entry, so it attributes as
/// `Attribution::Baseline`. `hello.py` is new, and the traced `Write` call names its full
/// path, so it attributes as `Attribution::Traced`. Neither raises a finding, which is the
/// point: every file in the seal is explained.
fn seal() -> SealManifest {
    SealManifest {
        entry_id: ENTRY_ID.to_string(),
        sealed_at: SEALED_AT.to_string(),
        workspace: WORKSPACE.to_string(),
        files: vec![
            SealedFile {
                path: "README.md".to_string(),
                bytes: 12,
                sha256: README_SHA256.to_string(),
            },
            SealedFile {
                path: "hello.py".to_string(),
                bytes: 41,
                sha256: HELLO_SHA256.to_string(),
            },
        ],
        excluded: vec![".claude/settings.json".to_string()],
    }
    .sorted()
}

/// One Claude Code transcript record in the user slot, as the harness writes them.
///
/// `origin.kind` and `promptSource` are the connector's own positive test, `cwd` is what
/// V10 measures the tracked session's directory by, and `promptId` is the `prompt_id` join
/// both legs stamp.
fn typed_prompt(at: &str, prompt_id: &str, text: &str) -> serde_json::Value {
    json!({
        "type": "user",
        "sessionId": SESSION_ID,
        "cwd": WORKSPACE,
        "version": HARNESS_VERSION,
        "timestamp": at,
        "promptId": prompt_id,
        "userType": "external",
        "promptSource": "typed",
        "origin": { "kind": "human" },
        "message": { "role": "user", "content": text },
    })
}

pub fn remove_generator_output(out: &Path, name: &str) -> std::io::Result<()> {
    match std::fs::remove_file(out.join(name)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn remove_previous_outputs(out: &Path) -> std::io::Result<()> {
    // Keep trace.toml: TraceExport owns atomic descriptor replacement. These are the
    // generator's exact stream and fixed-document outputs, never a directory sweep.
    for name in [
        CHAIN_FILE,
        ACKS_FILE,
        RECEIPT_FILE,
        BASELINE_FILE,
        SEAL_FILE,
        MANIFEST_FILE,
    ] {
        remove_generator_output(out, name)?;
    }
    Ok(())
}

/// The whole trace, under `plan`.
fn generate_with(out: &Path, plan: Plan, generation: Generation) {
    std::fs::create_dir_all(out).expect("create the output directory");
    remove_previous_outputs(out).expect("remove the generator's previous outputs");
    let mut export = TraceExport::create(out).expect("open the trace export");
    let protocol_version = generation.protocol_version();
    let connector_dir = generation.connector_dir();

    // ---- the rulebook, beside the evidence. Current generation reads the shipped
    // connector; historical generation reads the frozen three-file connector input. In
    // both cases the exact manifest bytes written beside the trace also enter its receipt.
    let manifest_bytes =
        std::fs::read(connector_dir.join("manifest.toml")).expect("the connector manifest");
    std::fs::write(out.join("manifest.toml"), &manifest_bytes).expect("copy the manifest");
    // The digest the receipt claims, taken over the bytes just written through the same
    // function `verify_trace` hashes the bytes it reads with — so the comparison V13 makes
    // is between one digest and itself rather than between two spellings of one.
    let manifest_sha256 = goalcup_verifier::legs::sha256_hex(&manifest_bytes);
    // And the digest of the whole connector directory, which the daemon takes over its own
    // `connectors/<id>/` at workspace creation and the server signs into the receipt. Taken
    // here the same way, over the selected connector input. No template byte is copied into
    // the trace: the point of the claim is that a trace carries `manifest.toml` and nothing
    // else of the connector, so the receipt is the only place the templates' digest can
    // appear.
    let connector_dir_sha256 =
        goalcup_protocol::connector_dir::connector_dir_sha256(&connector_dir)
            .expect("the connector directory hashes");
    // The interval the rulebook declares, read out of the bytes just written rather than
    // retyped here: `gaps::classify` calls a window a hole once it runs longer than twice
    // this number, so a fixture that hardcoded 60 would keep asserting against a constant
    // after the manifest moved.
    let heartbeat = goalcup_protocol::manifest::ConnectorManifest::from_toml_str(
        std::str::from_utf8(&manifest_bytes).expect("the shipped manifest is UTF-8"),
    )
    .expect("the shipped manifest parses")
    .heartbeat_seconds
    .expect("claude-code-cli declares a heartbeat interval");

    // ---- seq 0: the baseline, chained first. `write_baseline` chains the link and
    // writes the file, so the digest in the chain is the digest of the bytes in
    // `baseline.json` and the list cannot be rewritten afterwards without moving seq 0 —
    // the link the start receipt witnesses.
    let (_, genesis) = export
        .write_baseline_for_protocol(protocol_version, &baseline())
        .expect("chain and write the baseline");
    assert_eq!(genesis.seq, 0, "the baseline is the genesis link");

    // ---- what the launch seeded, chained where `register_process` puts it: after the
    // baseline and before the first record, because the CLI seeds, spawns, and hands the
    // daemon the process before the harness has written anything.
    if plan.home_seeded {
        push(
            &mut export,
            protocol_version,
            "daemon",
            "2026-09-01T00:00:05Z",
            EventBody::HomeSeeded(home_seed_record()),
        );
    }

    // ---- the work. Leg 1 and leg 2 alternate, as they do on a real run: the harness
    // writes the record, the hook posts it a moment later.
    leg1(
        &mut export,
        protocol_version,
        "2026-09-01T00:00:10Z",
        &typed_prompt("2026-09-01T00:00:10Z", "prompt-1", GOAL),
    );
    leg2(
        &mut export,
        protocol_version,
        "2026-09-01T00:00:11Z",
        "/UserPromptSubmit",
        json!({
            "session_id": SESSION_ID,
            "cwd": WORKSPACE,
            "prompt_id": "prompt-1",
            "prompt": GOAL,
        }),
    );
    // The agent's one turn: a `Write` naming the full path of the file the seal holds.
    leg1(
        &mut export,
        protocol_version,
        "2026-09-01T00:00:20Z",
        &json!({
            "type": "assistant",
            "sessionId": SESSION_ID,
            "cwd": WORKSPACE,
            "version": HARNESS_VERSION,
            "timestamp": "2026-09-01T00:00:20Z",
            "requestId": "req-1",
            "message": {
                "id": "msg-1",
                "role": "assistant",
                "model": "claude-opus-5",
                "usage": {
                    "input_tokens": 1_200,
                    "output_tokens": 340,
                    "cache_read_input_tokens": 0,
                },
                "content": [
                    { "type": "text", "text": "Writing hello.py." },
                    {
                        "type": "tool_use",
                        "id": "toolu_01",
                        "name": "Write",
                        "input": {
                            "file_path": format!("{WORKSPACE}/hello.py"),
                            "content": "from datetime import date\nprint(date.today())\n",
                        },
                    },
                ],
            },
        }),
    );
    leg2(
        &mut export,
        protocol_version,
        "2026-09-01T00:00:21Z",
        "/PreToolUse",
        json!({
            "session_id": SESSION_ID,
            "cwd": WORKSPACE,
            "tool_use_id": "toolu_01",
            "tool_name": "Write",
        }),
    );
    leg1(
        &mut export,
        protocol_version,
        "2026-09-01T00:00:30Z",
        &typed_prompt("2026-09-01T00:00:30Z", "prompt-2", KEEP_GOING),
    );
    leg2(
        &mut export,
        protocol_version,
        "2026-09-01T00:00:31Z",
        "/UserPromptSubmit",
        json!({
            "session_id": SESSION_ID,
            "cwd": WORKSPACE,
            "prompt_id": "prompt-2",
            "prompt": KEEP_GOING,
        }),
    );
    // The harness's own notification, in the user slot, excluded by the ordinary positive
    // test (`origin.kind` is not `human`; see D18) and reported by V6 as `user_slot_excluded`,
    // the Pass-level finding this record is here to raise.
    leg1(
        &mut export,
        protocol_version,
        "2026-09-01T00:00:40Z",
        &json!({
            "type": "user",
            "sessionId": SESSION_ID,
            "cwd": WORKSPACE,
            "version": HARNESS_VERSION,
            "timestamp": "2026-09-01T00:00:40Z",
            "userType": "external",
            "origin": { "kind": "task-notification" },
            "message": {
                "role": "user",
                "content": "<task-notification>the Write tool finished</task-notification>",
            },
        }),
    );
    // Four leg-1 records are chained, and the daemon's cursor says so from here on. It stops
    // moving once the work is chained, so every window sees an unmoved cursor over an
    // unchanged chain, and neither `cursor_mismatch` nor `cursor_ahead_of_chain` has anything
    // to say.
    let chained_records = 4usize;

    // ---- the week. One report every twelve hours, each acknowledged, from a minute
    // after the declared start. `assert` rather than a comment: the hole machinery only engages
    // above two heartbeat intervals, and a fixture whose windows quietly stopped being
    // holes would still pass every other assertion here while exhibiting no `clean_gap`.
    assert!(
        BEAT_INTERVAL > 2 * heartbeat,
        "a {BEAT_INTERVAL}s report interval has to exceed two {heartbeat}s heartbeat \
         intervals for the window between two reports to be a hole at all"
    );
    let start = parse_z(STARTED_AT).expect("the declared start parses");
    let sealed = parse_z(SEALED_AT).expect("the declared seal parses");
    let mut beats: Vec<(String, Link)> = Vec::new();
    for k in 0..BEATS {
        let at = format_z(start + heartbeat + k * BEAT_INTERVAL);
        let link = push(
            &mut export,
            protocol_version,
            "daemon",
            &at,
            EventBody::Heartbeat {
                ts: at.clone(),
                stores_seen: vec![records_cursor(STORE, chained_records, "records")],
                receiver_ok: true,
            },
        );
        beats.push((at, link));
    }
    let last_beat = parse_z(&beats.last().expect("at least one report").0)
        .expect("a timestamp this generator wrote parses");
    assert!(
        sealed.saturating_sub(last_beat) < 86_400,
        "the last report has to sit less than a day before the seal, or V12 reports \
         `long_unanchored_span` for the tail"
    );

    // ---- the variants, and only the variants. `plan.compliant()` chains neither.
    if plan.truncation {
        let at = format_z(last_beat + heartbeat);
        if plan.restart {
            // The corroboration the verifier requires: the tailer came back up, and a re-read
            // from offset 0 is what a restarting tailer does. It names the head it found,
            // which has to be the chain position immediately below itself or `classify`
            // raises `restart_head_mismatch`; both numbers are read off the writer.
            let (head_seq, head_hash) = export.head().expect("the chain has links");
            push(
                &mut export,
                protocol_version,
                "daemon",
                &at,
                EventBody::DaemonRestart {
                    at: at.clone(),
                    previous_head_seq: head_seq,
                    previous_head_hash: head_hash,
                },
            );
        }
        push(
            &mut export,
            protocol_version,
            "daemon",
            &at,
            EventBody::TailNotice {
                store: STORE.to_string(),
                path: "{config_dir}/projects/ws/compliant.jsonl".to_string(),
                kind: TailNoticeKind::Truncated,
                offset_before: 4_096,
                offset_after: 0,
                inode_before: Some(8_612_004),
                inode_after: Some(8_612_004),
                at: at.clone(),
            },
        );
    }

    // ---- the seal, chained last by `write_seal`, which closes the chain.
    let (_, seal_link) = export
        .write_seal_for_protocol(protocol_version, &seal())
        .expect("chain and write the seal");

    // ---- the acknowledgements: what the server durably stored, signed with the pinned
    // key. One per report, plus one over the seal link itself — which is what puts the
    // highest acknowledged sequence at the head of the chain, so no record and no hole
    // sits above the anchor.
    let key = server_key();
    let mut acked: Vec<(String, Link)> = beats;
    acked.push((SEALED_AT.to_string(), seal_link));
    for (at, link) in &acked {
        let mut a = AckCheckpoint {
            entry_id: ENTRY_ID.to_string(),
            seq: link.seq,
            hash: link.hash.clone(),
            acked_at: at.clone(),
            server_signature: String::new(),
        };
        a.server_signature = hex::encode(key.sign(&a.signing_bytes().unwrap()).to_bytes());
        export.append_ack(&a).expect("append the acknowledgement");
    }

    // ---- the start receipt: the one statement about this run the participant did not
    // write. `chain_head` is the genesis link — the baseline — so rewriting what the
    // workspace held at t0 moves the head the server signed.
    export
        .write_receipt(
            &StartReceipt::sign(
                StartReceiptClaims {
                    entry_id: ENTRY_ID.to_string(),
                    connector_id: CONNECTOR.to_string(),
                    workspace: WORKSPACE.to_string(),
                    session_id: SESSION_ID.to_string(),
                    started_at: STARTED_AT.to_string(),
                    deadline_at: SEALED_AT.to_string(),
                    manifest_sha256,
                    chain_head: genesis.hash.clone(),
                    connector_dir_sha256: Some(connector_dir_sha256),
                    run_public_key: None,
                    questionnaire_version: None,
                },
                &key,
            )
            .expect("sign the receipt"),
        )
        .expect("write the receipt");

    // ---- the descriptor. No `[[leg1]]`, no `[leg2]`: this trace declares one chain, and
    // everything either leg holds is in it.
    export
        .write_descriptor(&TraceDescriptorDoc {
            connector: CONNECTOR.to_string(),
            entry_id: ENTRY_ID.to_string(),
            workspace: WORKSPACE.to_string(),
            tracked_sessions: vec![SESSION_ID.to_string()],
            harness_version: HARNESS_VERSION.to_string(),
            connector_manifest: "manifest.toml".to_string(),
            chain: Some(CHAIN_FILE.to_string()),
            receipt: Some(RECEIPT_FILE.to_string()),
            baseline: Some(BASELINE_FILE.to_string()),
            acks: Some(ACKS_FILE.to_string()),
            seal: Some(SEAL_FILE.to_string()),
            server_public_key: Some(hex::encode(key.verifying_key().to_bytes())),
            expected_reread: plan.expected_reread,
        })
        .expect("write the descriptor");
}

/// The compliant trace with a truncation planted and `expected_reread` declared.
///
/// The whole corroboration check in one function. `expected_reread` is written by the
/// participant, so declaring it must not be enough to downgrade `store_rewritten`: with
/// `corroborated` false the chain carries no witness at all and the truncation stays a
/// Fail; with it true a `DaemonRestart` sits immediately below the notice and the same
/// truncation reads as `store_reread`.
pub fn with_truncation_and_expected_reread(out: &Path, corroborated: bool) {
    generate_with(
        out,
        Plan {
            expected_reread: true,
            truncation: true,
            restart: corroborated,
            home_seeded: false,
        },
        Generation::Current,
    );
}

/// The compliant trace with the launch's `[home_seed]` record chained.
///
/// A variant and not a change to the committed fixture, for the reason `Plan::home_seeded`
/// gives: an additive record must leave every older chain — the fixture included — hashing
/// and reading exactly as it did.
pub fn with_home_seed(out: &Path) {
    generate_with(
        out,
        Plan {
            home_seeded: true,
            ..Plan::compliant()
        },
        Generation::Current,
    );
}
