# Architecture

How do the M2 recorder's parts fit, and where is each guarantee tested? For contributors on
`m2/wire`. Updated: 2026-10-12. Normative interfaces: [CONTRACTS]; terms: [glossary](GLOSSARY.md).

## 1. Scope

- **M2** (this branch): a local recorder. One harness runs headless on one goal; its model
  traffic (wire leg) and its own session file (native leg) go into one protocol-v5 chain, sealed
  locally. Scope: [D37](DECISIONS.md#d37-m2-autonomous-runs-research-evidence-and-open-contributions);
  constraints D36–D41 are in [DECISIONS](DECISIONS.md).
- **Status:** v0.5.0 is published as source only. Later `v*` tags attach unsigned binary
  tarballs, installed with Homebrew ([packaging/README.md](../packaging/README.md)). No signing,
  notarization or qualification claim is made. The qualification records are in git
  history at `6494030`.
- **V1** lives on `v1/trunk`. Its hook leg is not in this tree
  ([D48](DECISIONS.md#d48-m2-the-v1-path-leaves-the-tree)), and neither are the verifier's
  V1 checks for pre-v5 traces: `goalcup-verify` reads protocol v5 only.

## 2. Crates and the dependency rule

Five workspace members ([Cargo.toml](../Cargo.toml)).

| Crate | Ships | `[dependencies]` (workspace crates) | `[dev-dependencies]` |
|---|---|---|---|
| `goalcup-platform` | library | none | none |
| `goalcup-protocol` | library | platform | none |
| `goalcup-verifier` | `goalcup-verify` | protocol | none |
| `goalcup-daemon` | `goalcup-daemon` | protocol, platform | itself |
| `goalcup-cli` | `goalcup` | protocol, verifier, platform | itself, daemon |

The rule ([D3](DECISIONS.md)): **the daemon never links the verifier; the CLI never links the
daemon**. Both hold for `[dependencies]`; dev-dependencies cross them for tests only. The CLI
links the verifier on purpose. Checked by `the_structural_bans_hold_at_the_dependency_level`
([dependency_bans.rs](../crates/goalcup-cli/tests/it/dependency_bans.rs), `cargo tree`)
and `the_shipped_binary_links_no_daemon` ([seal_scan.rs](../crates/goalcup-cli/tests/it/seal_scan.rs)).

### One feature set: the capture product

The build is M2: D48 deleted the V1 execution path (behind `v1-compat` since D42); the `v0.5.0`
tag keeps it. Connectors load from `harness.toml` alone; the V1 `manifest.toml` files went with OS-29.
The daemon opens a registry written by either line and refuses only V1 rows that still owe work.

## 3. Roles

| Part | Does | Code |
|---|---|---|
| `goalcup start --goal-file` | Preflight; creates the entry; runs the proxy **in process**; gates and launches the harness; finishes; key scan | [commands/start.rs] (`capture`, five phases) |
| `goalcup seal` | Standalone v5 finish of an entry | [commands/seal.rs](../crates/goalcup-cli/src/commands/seal.rs) |
| Proxy | Loopback CONNECT proxy; relays bytes unchanged, tees decoded bodies as frames | [proxy/](../crates/goalcup-cli/src/proxy/mod.rs) |
| Daemon | Ingest, per-producer redaction, chain append, native capture, finish, termination | [runtime/capture/][rc] |
| Verifier | v5 integrity reader and research report with five built-in decoders (two native: `codex-native`, `claude-code-native`; three wire: `openai-responses`, `anthropic-messages`, `openai-chat-completions`) (library and `goalcup-verify`; no CLI relay since D48) | [capture_analysis/](../crates/goalcup-verifier/src/capture_analysis/mod.rs) |

`goalcup` has five commands (`start --goal-file`, `status`, `entries`, `seal`, `doctor`); a v5
entry cannot resume ([C5 amendments][cont]).
Control API v6 requires the version on mutating requests; create binds the directory digest.

The open protocol-hint grammar validates metadata only. `DecoderRegistry::builtin()` and each
decoder's `accepts` decide which retained formats are decoded. The wire decoders share framing,
accounting and D45 close classification; Anthropic keeps its provider-specific state.

## 4. The two legs

On M2 the legs are **native + wire**; the V1 hook leg is deleted (D2 note in
[DECISIONS](DECISIONS.md)). Neither leg erases a gap in the other.

| | Wire leg | Native leg |
|---|---|---|
| Source | Model traffic to the connector's declared hosts | The harness's own session file |
| How | Process-scoped `HTTPS_PROXY` + per-launch CA in the profile's `ca_env`; no base URL is ever set ([D1](DECISIONS.md) note) | Bytes read under an isolated root through no-follow capabilities |
| Producer | Proxy epochs over loopback IPC ([sender](../crates/goalcup-cli/src/proxy/ingest.rs) → [receiver](../crates/goalcup-daemon/src/ingest.rs)) | [native_capture.rs](../crates/goalcup-daemon/src/native_capture.rs), submitting to the owner directly |
| Scope | `[[capture.hosts]]` hosts and `evidence_paths` | One source per connector, from its `harness.toml` `[[capture.native]]` row ([native_scope.rs](../crates/goalcup-daemon/src/runtime/capture/native_scope.rs) `native_plan`): Codex `sessions/*/*/*/rollout-*-{id}.jsonl`, Claude `projects/*/{id}.jsonl`, Hermes `sessions/session_{id}.json` (snapshot) |
| Identity | Physical connection id + exchange, WS `message_index` ([C1]) | Codex: first `thread.started` on stdout; Claude: preassigned session id; Hermes: stderr `session_id:` line, bound at finish |
| On loss | `Loss` frames and gaps (§6); the relay never stops | One stream-less `SourceChanged` gap; the harness never stops ([C5 T24N amendment][cont]) |

## 5. Entry lifecycle

CLI phases are in [commands/start.rs]; daemon finish stages are in
[finish.rs](../crates/goalcup-daemon/src/runtime/capture/finish.rs).

1. **Prepare:** refuse unsafe profile, pin, executable, credential or home identity before allocation (D39).
2. **Create/reserve:** after CLI run/home/CA/seed setup, daemon compares D46 before its allocation or
   writes; refusal cleans CLI prep, success writes Baseline/reservation, starts proxy and records Goal.
3. **Spawn gate:** park before `exec`; after durable registration, re-open the exact pinned path
   with `O_NOFOLLOW|O_NONBLOCK`, stream-hash a regular file, then release; no snapshot/inode claim.
4. **Run:** append `Launch`, tee identity output and capture native plus wire legs.
5. **Finish budget:** stop the proxy, drain, persist loss, stop owners, walk the workspace and write
   `LocalFinish` plus `seal.json`; a refused finish writes no terminal.
6. **Termination:** SIGTERM, connector grace, SIGKILL, sweep and reconfirm run outside that budget.
7. **Seal, scan, verify:** bind the seal and termination, run any key scan, then verify (§6).

[CONTRACTS C6][cont] pins spawn/finish; [TERMINATION](TERMINATION.md) pins process ownership. A restart
reopens an unfinished v5 entry under `RecoveredStatusSealOnly`: status and seal remain authoritative,
abandon still works for an entry that never launched and handed nothing off, every other
authenticated mutator refuses before side effects, and a launched entry gets one `IngestLost` gap.
An entry that never launched and handed nothing off is abandoned by the idle loop after
`UNLAUNCHED_LIMIT` (15 minutes). The launcher reconnects finish once.

## 6. Chain, records and verdicts

| Concept | What it is | Pointer |
|---|---|---|
| Chain | `chain.jsonl`: append-only, hash-linked, byte-canonical v5 envelopes; one writer per entry | [writer.rs](../crates/goalcup-daemon/src/runtime/capture/writer.rs), [C1] |
| Producer, epoch, stream | A frame sender; one admitted lifetime; one payload sequence. Stream 0 carries `Connection`, `Recovery`, `Loss` | [C1], [C2], [sink.rs](../crates/goalcup-daemon/src/runtime/capture/sink.rs) |
| Records | `Open`, `Segment` (redacted; base64 after redaction), `End`, `Gap`, `Connection`, `Recovery`, `ProducerClosed`, `Launch`, `Assistance`, `LocalFinish` | [capture.rs](../crates/goalcup-protocol/src/capture.rs) |
| Gap | A chained statement of loss with a `GapReason`; the trace is then incomplete, never clean | [D9](DECISIONS.md), [C1] |
| Loss (H-16) | Stream-0 proxy frame for traffic relayed but not captured: fail-open tunnel (at the decision), opaque fallback, full capture table, unsettled cut, over-bound open. Chained as a stream-less `Gap` on the producer's clock | [cont], "C1/C3 proxy loss" |
| Recovery | A new wire epoch replacing a lost predecessor at a fresh request or message boundary; nothing is replayed | [C2] |
| Baseline / seal binding | `baseline.json` digest, file count and entry equal the seq-0 claim; `LocalFinish` binds the canonical `seal.json` digest | [cont], "C1 bounded state", "C6 local finish" |
| Anchor | `AnchorState::LocalOnly` (no server, receipt or ACK) or `Signed`; LocalOnly alone never lowers exit 0 | [C4], [C6] |

**One orphan rule.** A `Recovery` predecessor is an *orphan* when the chain holds no record under
its name (writer, reader); receiver and sink test the stand-in "admitted no frame but `Loss`". An
orphan Recovery supersedes every other open wire producer, a named one only that producer; the
writer refuses any later record of a superseded producer and a second Recovery of one. Table in
[runtime/capture/mod.rs][rc]; contract in [cont], H-16 ¶4.

The reader and `DecodeSession` share the bounded Recovery-lineage calculation: capped seen-producer
bookkeeping and supersession selection, including repeated Recovery and the 127/128 producer edges.
They are not one state machine: the reader validates the whole trace, while the session is
nonvalidating and routes per stream. Shipped producers allocate separate ids for native/goal and
wire classes; explicit rejection of invalid mixed-source producers is H-29.

**Freeze.** v5 records freeze at the `v0.5.0` tag: after it any new record field, optional or
not, is v6, because a released verifier must keep reading every v5 trace and every record denies
unknown fields. `FrameEvent` is governed by `CONTROL_API_VERSION`; receipt, ack and chunk files
carry their own domain tags; the reader corpus `crates/goalcup-verifier/tests/fixtures/v5-corpus/`
is the guard; a server refuses an unknown `protocol_version` with `client_newer`. Rule:
[CONTRACTS](CONTRACTS.md) ("The v5 format freeze"); `capture_status.rs` is outside it.

**Integrity exits** ([reader.rs][rdr] `exit_code`): **0** valid, sealed, coverage complete;
**1** incomplete (a gap, unsealed, a Recovery, no Launch, a missing manifest); **2** invalid,
including a torn tail (kept, never repaired; the entry cannot resume, [C2]); **3** no usable
report. Research output: `goalcup-verify --research-json`, format in
[m2-report-format.md](research/m2-report-format.md).

Integrity, anchor, key-source and terminal status facts are typed Rust values. The full research
summary is assembled JSON, not one typed Rust document. Typed names may be idiomatic while their
serialized words remain pinned by research-report schema 1.

## 7. The proxy

- **CONNECT** on loopback, authenticated by a per-launch credential in the proxy URL, which is
  never forwarded ([connect.rs](../crates/goalcup-cli/src/proxy/connect.rs)).
- **TLS only for declared hosts** whose SNI matches; auth hosts, other hosts and mismatched
  ClientHellos tunnel opaquely. Upstream TLS is always validated; the CA is name-constrained to
  the approved hosts and its key never leaves memory ([tls.rs][tls]).
- **HTTP/1.1 and WebSocket only** (ALPN `http/1.1` or none; no HTTP/2). One gzip/deflate coding
  and `permessage-deflate` are undone; any other is an `UnsupportedEncoding` gap ([http.rs][http],
  [websocket.rs][ws]).
- **No header is recorded:** bytes are forwarded unchanged; only selected bodies and allowlisted
  metadata become frames ([metadata.rs](../crates/goalcup-cli/src/proxy/metadata.rs), [C1]).
- **Capture is a side channel:** a refused frame stops capture, never the relay; each loss is
  chained as a `Loss` when it begins (§6). Bounded by `MAX_OPEN_CONNECTIONS`,
  `MAX_CONNECTION_TASKS` and `MAX_CAPTURE_QUEUE_BYTES`; no ACK, no replay ([C2], [C3]).
- **Outages:** a provider-host connect re-resolves and retries until 5/6 of the admission deadline;
  upstream sockets get TCP keepalive, no idle timeout. Failed connects coalesce into outage
  windows printed at finish, not chained ([outage.rs](../crates/goalcup-cli/src/proxy/outage.rs)).

## 8. Redaction

Runs in the **daemon owner, one `StreamRedactor` per producer, before encoding, hashing and
disk** ([redact/stream.rs][rs]; rules in [redact.rs](../crates/goalcup-daemon/src/redact.rs);
[D36](DECISIONS.md), [D39](DECISIONS.md)).

| Rule class | Behaviour |
|---|---|
| Hard: shared set + pinned connector `extra_patterns` | First rule wins an overlap |
| Soft: `masked_key` (D41) | Loses every overlap; can only add redaction |
| Block: `private_key` (PEM) | Absorbs and merges with every overlapping claim; an unterminated `BEGIN` redacts to the end of the record |

**Hold:** at most `MAX_HELD_BYTES` (16 KiB) undecided bytes per stream. Past it the stream
**fails closed**: from the earliest undecided byte everything is replaced, up to the next safe
record terminator or the stream's end. Below the bound, output equals whole-stream redaction
whatever the chunking. A connector regex without a pinned recognizer is refused. Best effort;
residuals (F-07, invalid UTF-8 inside a key body, a lone `BEGIN`): [checklist](CHEAT-CHECKLIST.md).

## 9. Connectors, homes, pins and the contract kit

- Discovery checks `$GOALCUP_CONNECTORS_DIR`, installed `share/goalcup/connectors`, then the source
  tree. It validates every immediate child with a regular `harness.toml` before selecting by id.
- The source tree and OSS cut contain exactly three pinned directories: `codex-cli`,
  `claude-code-cli`, `hermes-cli`. A release tarball carries `codex-cli` and `claude-code-cli` as
  committed, under `share/goalcup/connectors` ([packaging/README.md](../packaging/README.md)).
- Every run gets a fresh `HOME`. Codex and Claude use shared isolated config homes (Claude adds the
  checked Keychains link); Hermes uses a seeded per-run `HERMES_HOME`, child-only
  `DEEPSEEK_API_KEY`, replace-mode `HERMES_CA_BUNDLE`, deferred identity and a fail-closed key scan.
- The six synthetic contract-kit cases are `controlled-workload` (no identity, append-only),
  `synthetic-preassigned` (launch identity), `synthetic-stderr-token` (deferred identity) and
  `synthetic-stdout-json` (two snapshots), the last two each with a keyed `synthetic-keyed-*`
  twin. Each requires `--dev`; wrong-id decoys appear in all but `controlled-workload`.
- Directory pins come from `pin-manifests.sh` (`connector-dirs.txt`; no V1 manifest exists); executables
  carry version/SHA-256 pins. Same-UID executable and non-atomic connector-namespace races remain
  accepted residuals; see [SECURITY](../SECURITY.md).

## 10. Where each guarantee is tested

Test paths are under `crates/`. The "Record" column names the measurement that backed the row; those records are in git history at
`6494030` (`docs/qualification/`).

| Guarantee | Tests | Record |
|---|---|---|
| D3: daemon links no verifier; CLI links no daemon | `goalcup-cli/tests/it/dependency_bans.rs`, `goalcup-cli/tests/it/seal_scan.rs` | none |
| Spawn gate: no exec before durable registration | `goalcup-cli/tests/it/capture_gate.rs`, `goalcup-cli/tests/m2_cli_lifecycle.rs`, `goalcup-cli/tests/full/capture_crash_campaign.rs` | H-10 4c |
| Finish budget: 8 s + 2 s, latch, termination outside | `goalcup-daemon/tests/full/capture_finish_budget.rs`, `goalcup-daemon/src/runtime/capture/tests.rs`, `goalcup-cli/tests/full/capture_failure_campaign.rs` | H-10 4c |
| Termination contract | `goalcup-daemon/tests/it/terminate_sim.rs`, `goalcup-daemon/tests/full/process_kill.rs` | [TERMINATION](TERMINATION.md) |
| Loss frames make a lossy run incomplete | `goalcup-cli/tests/it/m2_emitter.rs`, `goalcup-cli/tests/it/m2_proxy_integration.rs`, `goalcup-daemon/tests/it/capture_ingest.rs`, `goalcup-cli/tests/m2_cli_lifecycle.rs`, `goalcup-cli/tests/full/capture_crash_campaign.rs` | [CONTRACTS][cont], H-16 ¶8 |
| One orphan rule (sink, receiver, writer, reader) | `goalcup-daemon/src/runtime/capture/tests.rs`, `goalcup-daemon/tests/it/capture_ingest.rs`, `goalcup-verifier/tests/it/m2_capture_integrity.rs` | T23 (CP-A4) |
| Integrity exits 0/1/2/3; baseline and seal binding | `goalcup-verifier/tests/it/m2_capture_integrity.rs` | T23 |
| Streaming = whole-stream redaction; 16 KiB fail closed | `goalcup-daemon/tests/it/capture_redaction.rs`, `goalcup-daemon/tests/it/capture_redaction_extra.rs`, `goalcup-daemon/tests/it/redaction.rs` | E00 |
| Reservation on the real chain file | `goalcup-protocol/tests/capture_reserved_storage.rs` | T01, T25 storage |
| Native scope, no symlink, deferred Hermes scope | `goalcup-daemon/tests/it/capture_native.rs`, `goalcup-daemon/tests/it/capture_deferred_scope.rs` | 3c rows, Hermes row |
| TLS and CONNECT, HTTP and WebSocket relay | `goalcup-cli/tests/it/m2_tls_connect.rs`, `goalcup-cli/tests/it/m2_http_relay.rs`, `goalcup-cli/tests/it/m2_websocket_relay.rs` | E00 |
| Frozen wire contracts; connector pins | `goalcup-protocol/tests/it/m2_contract.rs`, `goalcup-protocol/tests/it/harness_schema.rs`, `goalcup-protocol/tests/it/harness_loader.rs`, `goalcup-protocol/tests/it/harness_conformance.rs`, `goalcup-protocol/tests/it/connector_pins.rs` | none |
| v5 create, goal, launch, status through the daemon | `goalcup-daemon/tests/it/capture_lifecycle.rs` | T23 |
| Profile launch, executable pin, env refusals | `goalcup-cli/tests/it/harness_launch_plan.rs`, `goalcup-cli/tests/it/start_launch.rs` | T02 |
| Recorded V1 `capture_only` keys ignored; Hermes row decoded by exact hint | `goalcup-cli/tests/it/capture_only_refusal.rs`, `goalcup-verifier/tests/it/m2_hermes_capture_only.rs` | Hermes row |
| Hermes key child-only, private home, key scan | `goalcup-cli/tests/it/hermes_child_secret.rs`, `goalcup-cli/tests/it/hermes_per_run_home.rs`, `goalcup-cli/tests/it/key_scan.rs`, `goalcup-cli/tests/it/key_scan_run.rs`, `goalcup-cli/tests/it/capture_stderr_tee.rs`, `goalcup-cli/tests/it/loader_script.rs`, `goalcup-cli/tests/full/m2_hermes_lifecycle.rs`, `goalcup-cli/tests/it/hermes_offline_key.rs` | Hermes row |
| Crash, torn tail, restart | `goalcup-cli/tests/full/capture_crash_campaign.rs` | T25 crash |
| Whole vertical on a synthetic workload | `goalcup-cli/tests/full/m2_synthetic_vertical.rs` | T23 |
| Six schema-1 contract-kit cases | `goalcup-cli/tests/full/harness_contract_kit.rs` | none |
| Research report | `goalcup-verifier/tests/it/m2_research_report.rs` | 3c rows |

## 11. Not here

Deferred deliberately: durable ingest replay, automatic proxy process restart, torn-tail repair,
separate blobs/object routes, a new journal backend, universal lineage/assistance classification,
aggregate unique totals, continuous process/workspace/binary surveys, PTY/input relay, and
desktop/Windows/Linux qualification,
public publishing, and production service operation.
Optional signed M2 upload/anchoring and rich browser/MCP presentation are post-pilot packets with
their own gates. Basic existing hashes/signatures remain; new evidence-comparison machinery can
follow a recorder that works. The supporting private historical record is intentionally omitted
from this public OSS cut. Operating the binaries: [OPERATING](OPERATING.md).

[CONTRACTS]: CONTRACTS.md
[C1]: CONTRACTS.md#c1-limits-and-representation-t00
[C2]: CONTRACTS.md#c2-submission-privacy-and-storage
[C3]: CONTRACTS.md#c3-transport-and-native-seams
[C4]: CONTRACTS.md#c4-status-and-interpretation
[C5]: CONTRACTS.md#c5-explicit-activation-and-serial-integration-ownership
[C6]: CONTRACTS.md#c6-explicit-local-lifecycle-and-verified-report-facts
[cont]: CONTRACTS.md#continuation-amendments-2026-09-29
[commands/start.rs]: ../crates/goalcup-cli/src/commands/start.rs
[tls]: ../crates/goalcup-cli/src/proxy/tls.rs
[http]: ../crates/goalcup-cli/src/proxy/http.rs
[ws]: ../crates/goalcup-cli/src/proxy/websocket.rs
[rs]: ../crates/goalcup-daemon/src/redact/stream.rs
[rc]: ../crates/goalcup-daemon/src/runtime/capture/mod.rs
[rdr]: ../crates/goalcup-verifier/src/capture_analysis/reader.rs
