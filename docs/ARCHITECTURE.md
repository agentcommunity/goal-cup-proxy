# Architecture

How do the parts of Goal Cup Proxy fit, and where is each guarantee tested? For contributors and
their agents. Wire formats are in [CONTRACTS](CONTRACTS.md); rules that bind the code are in
[DECISIONS](DECISIONS.md). Section 12 lists the terms. Use them exactly.

## 1. What Goal Cup Proxy is for

Goal Cup Proxy preserves evidence of running agent work for research across harnesses and operating
systems. It records declared native session artifacts and model traffic, seals them into a hash
chain, and leaves interpretation to versioned decoders. Goal Cup's one-prompt, 168-hour contest
uses this evidence for review and adds upload, deadline and participant rules.

The [research target](RESEARCH-TARGET.md) defines the desired evidence and how to test coverage;
[local next steps](research/LOCAL-NEXT-STEPS.md) sets the next validation work. The
[local research handoff](LOCAL-RESEARCH-HANDOFF.md) is history: how the research work was handed
over, not the work to do next.
Context-visible tool calls/results are sufficient without independently recording full tool
output or spill files. Subagent conversations are a separate research objective whose coverage
must be measured. These are targets, not assertions of universal capture. Platform status is in
the [README](../README.md#platforms): native Windows is planned; WSL2 is a Linux path awaiting
real Windows-machine validation.

Goal Cup Proxy guarantees:

1. **Bounded capture.** The harness's declared native artifacts and model traffic for the run.
   Detected capture loss is a gap in the chain, never an invented record. Activity outside the
   selected sources may be unobserved without a gap. Integrity completeness is not proof that
   every agent action was observed.
2. **Tamper evidence, not tamper proof.** The participant controls the machine. The chain, the
   server's signed receipt and acknowledgements, and the gaps make changes reviewable.
3. **Scope.** It records the bound session and workspace. It does not read other sessions,
   terminals, browser history or unrelated activity.
4. **Silence.** It never speaks to the agent. No reminders, no nudges, no input relay.

It is not an API proxy that changes where the harness connects, not a sandbox, not device
surveillance and not a judge. **Record raw, interpret later:** the daemon stores bytes, and every
judgement is made afterwards by open-source verifier code that anyone can re-run.

## 2. Crates and the dependency rule

| Crate | Ships | Depends on (workspace) | Dev-depends on |
|---|---|---|---|
| `goalcup-platform` | library | none | none |
| `goalcup-protocol` | library | platform | none |
| `goalcup-verifier` | `goalcup-verify` | protocol, platform | none |
| `goalcup-daemon` | `goalcup-daemon` | protocol, platform | itself |
| `goalcup-cli` | `goalcup` | protocol, verifier, platform | itself, daemon |

**The daemon never links the verifier. The CLI never links the daemon** (D3). The CLI links the
verifier on purpose. Tests: `the_structural_bans_hold_at_the_dependency_level`
([dependency_bans.rs](../crates/goalcup-cli/tests/it/dependency_bans.rs)) and
`the_shipped_binary_links_no_daemon` ([seal_scan.rs](../crates/goalcup-cli/tests/it/seal_scan.rs)).

Outside the workspace: `connectors/<id>/` (harness data) and `server/worker` (the upload server,
TypeScript).

## 3. The parts

| Part | Does | Code |
|---|---|---|
| `goalcup start` | Preflight; creates the entry; runs the recording proxy in process; gates and launches the harness; seals when it exits; uploads | [commands/start/](../crates/goalcup-cli/src/commands/start/) |
| `goalcup login`, `setup`, `continue`, `upload` | Device-flow login; machine and repository readiness; next segment of a run; finish an upload | [commands/](../crates/goalcup-cli/src/commands/), [account/](../crates/goalcup-cli/src/account/), [upload/](../crates/goalcup-cli/src/upload/) |
| `goalcup seal`, `status`, `entries`, `doctor` | End a run early; read status; list runs; check the machine | [commands/](../crates/goalcup-cli/src/commands/) |
| Recording proxy | Loopback CONNECT proxy. Relays bytes unchanged; tees decoded bodies as frames | [proxy/](../crates/goalcup-cli/src/proxy/mod.rs) |
| Daemon | Control API on loopback; ingest; per-producer redaction; chain append; native-file capture; finish; termination | [runtime/capture/](../crates/goalcup-daemon/src/runtime/capture/mod.rs) |
| Verifier | Integrity reader and research report. Six built-in decoders: native `codex-native` and `claude-code-native`; wire `openai-responses`, `anthropic-messages`, `openai-chat-completions`, and `amp-thread-jsonrpc` | [capture_analysis/](../crates/goalcup-verifier/src/capture_analysis/mod.rs) |
| Upload server | Checks the run credential, stores chunks in R2, sends the run emails | [server/worker](../server/worker/README.md), [UPLOAD-API](UPLOAD-API.md) |

The control API (version `CONTROL_API_VERSION`, now 12) requires its version on mutating requests.
Create binds the connector-directory digest, may carry the server's entry id, and carries the
run's own secret, the proxy credential (`run_secrets`, D60), which the daemon redacts by exact
value and holds in memory for that entry only.

## 4. A harness is data

A harness is `connectors/<id>/harness.toml` (harness-contract schema 1) plus seed files under
`files/`. Core code names no harness. Adding a harness adds a directory, the regenerated pin and
test data, and no `.rs` file ([CONNECTOR-SCHEMA](CONNECTOR-SCHEMA.md),
[HARNESS-ONBOARDING](HARNESS-ONBOARDING.md)).

The manifest says, for one harness:

- how to launch it: program, argv template, environment, chat mode (`interactive = true`),
  `resume_args`;
- which hosts the recording proxy decrypts (`[[capture.hosts]]`, each with a `protocol_hint`) and which CA
  variable the harness reads;
- which session files to read (`[[capture.native]]`: root, glob, `append_only`, `snapshot` or `final_snapshot` (a store copied once after the harness exits),
  `since_run_start`) and how the session is identified (`[capture.identity]`);
- the home layout (`[capture.home]`): shared or per-run config home, `run_home`, seeds;
- how to end it (`[capture.termination]`: `process_group` or `native_pid`, grace) and extra
  redaction patterns.

Shared modules do the rest: the loader and its trust tables (`harness.rs`, `capture_policy.rs`), the
launcher, the native-file reader, the recording proxy, the redactor and the decoders. A harness whose wire or
session format no decoder reads is still recorded. It is *capture-only* and decoded later when a
decoder is added.

- **Pins.** `crates/goalcup-protocol/pins/connector-dirs.txt` holds one SHA-256 per connector
  directory, written by `sh scripts/pin-manifests.sh`. Start refuses a directory that does not
  match. The CLI and the daemon each load the connector and must agree on the digest before an
  entry is created (D46). The pin stops a connector from smuggling in a base URL. Review is the
  control against a malicious merge ([SECURITY](../SECURITY.md), D51).
- **Builds are recorded, not pinned.** Goal Cup Proxy runs the harness that `program` resolves to on
  `PATH` and records `Launch.harness_build` (program digest, running-image digest, platform) and
  the version the harness prints (D53).
- **Discovery.** `$GOALCUP_CONNECTORS_DIR`, else `../share/goalcup/connectors` or
  `../../connectors` beside the binary. Release packaging discovers every committed
  [connector folder](../connectors); each has a [facts sheet](harnesses/README.md).
- **Homes.** Harness homes are isolated from the user's normal stores. Codex and Claude Code use a shared isolated config
  home (`CODEX_HOME`, `CLAUDE_CONFIG_DIR`) under `<config>/capture-homes/<id>`; runs on one are
  serialized by a lock (D39). Claude Code's run home links the macOS Keychains directory. Hermes
  shares one `HERMES_HOME` the same way; the participant's own Hermes setup keeps its provider and
  key there, and Goal Cup Proxy passes it none (D61). Amp instead keeps one isolated shared `HOME`
  across serialized runs. Other run homes are fresh, according to the connector. Every run home
  gets a `.profile` that exports the `PATH` the harness is handed, so a login shell whose
  `/etc/profile` resets `PATH` (Debian, Ubuntu) still finds the participant's tools and `goalcup`
  ([CONNECTOR-SCHEMA](CONNECTOR-SCHEMA.md)).
- **Contract kit.** Synthetic cases under `crates/goalcup-cli/tests/fixtures/harness-kit/` test the
  shared modules with stand-ins. Every connector runs through the manifest-driven shared stand-in
  (the connector contract), so a new connector adds no case. A passing kit says "synthetic
  tested", not "works with the real harness".

## 5. The two legs

Neither leg erases a gap in the other.

| | Wire leg | Native leg |
|---|---|---|
| Source | Model traffic to the connector's declared hosts | The harness's own session files |
| How | Process-scoped `HTTPS_PROXY` plus a per-launch CA in the connector's CA variable. No base URL is ever set (D1) | Bytes read under an isolated root through no-follow capabilities |
| Producer | Proxy epochs over loopback IPC ([sender](../crates/goalcup-cli/src/proxy/ingest.rs), [receiver](../crates/goalcup-daemon/src/ingest.rs)) | [native_capture.rs](../crates/goalcup-daemon/src/native_capture.rs), submitting to the owner directly |
| Scope | `[[capture.hosts]]` hosts and `evidence_paths` | `[[capture.native]]` rows. With `since_run_start`, every matching file new or written since the run began (D57) |
| On loss | `Loss` frames and gaps. The relay never stops | One stream-less `SourceChanged` gap. The harness never stops |

## 6. Entry lifecycle

CLI phases are in `commands/start/phases.rs` (`prepare`, `create_and_gate`, `register_and_release`,
`run_harness`, `finish_and_scan`). Daemon finish stages are in
[finish.rs](../crates/goalcup-daemon/src/runtime/capture/finish.rs).

1. **Prepare.** Refuse an unsafe profile, executable, credential or home before anything is
   allocated. On the server path: check the project folder and the repository, approve the run in the browser,
   then start the recorder and open the run (D71).
2. **Create.** The daemon compares connector digests (D46) and then writes `baseline.json` and the
   seq-0 `Baseline`. The chain is quota-only. The run host holds one disk reservation per run
   (D40 as amended). The recording proxy starts and the goal marker is recorded.
3. **Spawn gate.** The harness parks before `exec`. After the daemon durably registers it, the CLI
   re-opens the resolved program path with `O_NOFOLLOW|O_NONBLOCK`, requires a regular file and
   releases the harness. No binary bytes are read. No execution is bound to an inode.
4. **Run.** `Launch` is appended. The native and wire legs capture. The CLI uploads chunks and
   sends heartbeats.
5. **Stop the harness.** A standalone seal enters the daemon's entry run lock, then SIGTERM,
   the connector's grace, SIGKILL, sweep and reconfirm run with wire and native capture still live.
   This runs outside the capture budget ([TERMINATION](TERMINATION.md)); both the drain-share
   boundary and final deadline move by actual termination time. The launcher sees the child exit
   and stops its recording proxy; competing finishes share one terminal.
6. **Finish capture.** Close admission, drain the existing producers, persist explicit loss,
   stop the owner, read the workspace state (Git if the workspace is a clone, decided now; otherwise a
   file walk bounded by a file limit and the finish deadline, D71), then write
   `LocalFinish` plus `seal.json`. A refused finish writes no terminal and retains its termination
   outcome for retry. The capture budget is `LOCAL_CAPTURE_FINISH_BUDGET` (180 s) with a 2 s commit
   reserve (D40, D56). Control operations that can wait on the entry lock run on the blocking
   pool so ingest remains able to drain.
7. **Seal, verify.** Bind seal and termination, print the `verify:` line,
   upload the rest (the wrap-up).

A restart reopens an unfinished entry with status and seal authority only. Every other
authenticated mutator refuses. A launched entry gets one `IngestLost` gap. An entry that never
launched and handed nothing off is abandoned after `UNLAUNCHED_LIMIT` (15 minutes).
`goalcup continue` opens the next segment of a run as a new entry in the same project folder.

## 7. Chain, records and verdicts

| Concept | What it is |
|---|---|
| Chain | `chain.jsonl`: append-only, hash-linked, byte-canonical v5 envelopes, one writer per entry ([writer.rs](../crates/goalcup-daemon/src/runtime/capture/writer.rs)). Long runs rotate into chain files `chain.<start>.jsonl` at 256 MiB or 30 minutes (D55) |
| Envelope bodies | `Baseline`, `Seal`, `Capture` (the cutoff and signed-finalization bodies were removed on 21 Oct; a line carrying one is `record_malformed`, D54) |
| Capture records | `Open`, `Segment` (redacted, base64), `End`, `Gap`, `Connection`, `Recovery`, `ProducerClosed`, `Launch`, `Assistance`, `LocalFinish` ([capture.rs](../crates/goalcup-protocol/src/capture.rs)) |
| Gap | A chained statement of loss with a `GapReason`. A trace with a gap is incomplete, never clean |
| Loss | A stream-zero proxy frame for traffic relayed but not captured. Chained as a stream-less gap |
| Recovery | A new wire epoch replaces a lost one at a fresh request boundary. Nothing is replayed |
| Anchor | `LocalOnly` (no server receipt or acknowledgement) or `Signed`. `LocalOnly` alone never lowers exit 0 |

**The orphan rule.** A `Recovery` predecessor is an orphan when the chain holds no record under its
name. An orphan Recovery supersedes every other open wire producer. A named one supersedes only
that producer. The writer refuses any later record of a superseded producer. The rule is in the
writer, the reader, the sink and the receiver ([runtime/capture/mod.rs](../crates/goalcup-daemon/src/runtime/capture/mod.rs)).
Producer ID limits: `MAX_PRODUCERS` 65 536 per entry, `MAX_LIVE_PRODUCERS` 128 (D58).

**The v5 freeze.** Records freeze: any new field on a v5 record is v6, because every record denies
unknown fields. `FrameEvent` is governed by `CONTROL_API_VERSION`. The reader corpus
`crates/goalcup-verifier/tests/fixtures/v5-corpus/` guards the reader: a verifier change that makes a
valid v5 trace invalid needs a DECISIONS row (D43 ruling 7). The protocol carries v5 only (D54).

**Integrity exits** (`goalcup-verify --capture-integrity`, [reader.rs](../crates/goalcup-verifier/src/capture_analysis/reader.rs)):
0 valid, sealed, coverage complete; 1 incomplete (a gap, unsealed, a Recovery, no Launch, a
pruned chain); 2 invalid, including a torn tail (kept, never repaired); 3 no usable report. The
exit judges the record, not the run (D45). Research output: `goalcup-verify --research-json`
([m2-report-format.md](research/m2-report-format.md)). A server receipt or acknowledgement
verifies only under a key in `SERVER_PUBLIC_KEYS` (the production Worker's) or one passed with
`--dev-key`; the report then says `key_source: pinned` or `supplied`.

## 8. The recording proxy

- **CONNECT on loopback**, authenticated by a per-launch credential in the recording proxy URL. The
  credential is never forwarded ([connect.rs](../crates/goalcup-cli/src/proxy/connect.rs)).
- **TLS only for declared hosts** whose SNI matches. Authentication hosts, other hosts and
  mismatched ClientHellos tunnel opaquely. Upstream TLS is always validated. The CA is
  name-constrained to the approved hosts and its key never leaves memory ([tls.rs](../crates/goalcup-cli/src/proxy/tls.rs)).
- **HTTP/1.1 and WebSocket only** (no HTTP/2). One gzip, deflate, brotli or zstd
  coding and `permessage-deflate` are undone. Any other coding is an `UnsupportedEncoding` gap,
  named in `host.log`.
- **No header is recorded.** Bytes are forwarded unchanged. Only selected bodies and allowlisted
  metadata become frames ([metadata.rs](../crates/goalcup-cli/src/proxy/metadata.rs)).
- **Capture is a side channel.** A refused frame stops capture, never the relay. Each loss is
  chained when it begins. Bounded by `MAX_OPEN_CONNECTIONS`, `MAX_CONNECTION_TASKS` and
  `MAX_CAPTURE_QUEUE_BYTES`. No acknowledgement, no replay.
- **Outages.** Failed provider connects coalesce into outage windows printed at finish, not chained
  ([outage.rs](../crates/goalcup-cli/src/proxy/outage.rs)).

## 9. Redaction

Redaction runs in the daemon owner, one `StreamRedactor` per producer, before encoding, hashing
and disk ([redact/stream.rs](../crates/goalcup-daemon/src/redact/stream.rs), rules in
[redact.rs](../crates/goalcup-daemon/src/redact.rs); D36, D39).

| Rule class | Behaviour |
|---|---|
| Hard: shared set plus the connector's `extra_patterns` | First rule wins an overlap |
| Soft: `masked_key` (D41) | Loses every overlap. Can only add redaction |
| Block: `private_key` (PEM) | An `END` must start within 15 872 bytes of `BEGIN`, else the block is those bytes, up to the first later `BEGIN` in them. The parts inside JSON strings are redacted, and a part outside them holding a base64 run of 16 or more characters with a letter (not counting `e`/`E`) |

At most `MAX_HELD_BYTES` (16 KiB) of undecided bytes are held per stream. Past that the stream
**fails closed**: everything from the earliest undecided byte is replaced up to the next safe
terminator. Below the bound, output equals whole-stream redaction whatever the chunking. A
connector rule is a typed form with its own recognizer, never a regex. Redaction is best effort. The residuals are
in the [checklist](CHEAT-CHECKLIST.md). The recording proxy also strips credential headers by name.

## 10. Upload and the server

`goalcup start` uploads the chain to the Worker in compressed chunks while the run records and
keeps the server's acknowledgement of each in `acks.jsonl`. Recording never waits on the network,
and no server error stops the run or ends its upload (D64).
The server issues the entry id, the official start and the deadline. Parts the server acknowledged
are pruned locally two hours later, or at once when the parts alone pass the 3 GiB reservation or
the disk has under 5 GiB free beyond it (never the part being written or the one holding the last
acknowledged record); the verifier then reports `chain_pruned_locally` (incomplete, verify the
server copy). The deadline is 168 hours on the server's clock. At the deadline the CLI stops the
agent and seals. Details: [UPLOAD-API](UPLOAD-API.md), [server/worker](../server/worker/README.md).

## 11. Not here

Deliberately not built: durable ingest replay, automatic recording proxy restart, torn-tail repair, a PTY or
input relay, continuous workspace hashing, reminders injected by Goal Cup Proxy, cross-harness linking and
native Windows. Operating the binaries: [OPERATING](OPERATING.md).

## 12. Terms

This is the one glossary for the repository. Other docs link here. Use the words exactly.

**Runs and their pieces.** One concept, one name per audience:

| Concept | Participants say | Code and protocol say | What it is |
|---|---|---|---|
| The 168-hour attempt | run | run (`run_id`) | One participant's contest run on the server. It has one or more parts. |
| One unbroken stretch of recording | part | segment, stored as an entry (`entry_id`) | One harness, one workspace, one chain. `goalcup continue` starts the next one. |
| One file of a long chain | (not shown) | chain file | `chain.jsonl`, then `chain.<start>.jsonl` after each rotation at 256 MiB or 30 minutes (D55). Joined in order, they are one chain. |

"Segment" and "entry" name the same thing; new text says segment, and existing field names
(`entry_id`, `goalcup entries`) stay. A chain `Segment` record is a different thing: a body of
captured bytes. D55 and the finding codes `chain_part_missing` and `chain_pruned_locally` call
chain files "parts"; read that as chain files.

- **seal**: ending a segment: stop the harness, finish capture, write `LocalFinish` and `seal.json`.
- **receipt**: the server's signed start receipt for one segment: its facts and chain head, with its start and deadline.
- **ack** (acknowledgement): the server's signed note that it stored the chain up to one record.
- **harness**: the coding agent program (Codex CLI, Claude Code, Hermes, Amp).
- **connector**: `connectors/<id>/`, the pinned directory that says how to launch and capture one
  harness. Its identity is `connector_dir_sha256`. The loader is `harness::load_dir`.
- **harness manifest**: `connectors/<id>/harness.toml`, harness-contract schema 1. Parsed by
  `crates/goalcup-protocol/src/harness.rs`.
- **profile**: the `[capture]` table of the manifest. Its digest is `profile_sha256`.
- **`profile_id`**: the field `Launch.profile_id` holds a connector id (`hermes-cli`), not a digest.
- **origin**: loader authority, never a manifest field. `Pinned` is a pin-checked connector. `Dev`
  is an unpinned one chosen with `--dev`; it relaxes only the interpreter and argument-path
  refusals, and its run does not count as a competition entry.
- **shared capture home**: `<config>/capture-homes/<connector>`, the isolated config home that all
  runs of one connector use (D39).
- **native leg / wire leg**: section 5.
- **producer, epoch, stream**: a producer sends capture frames (the recording proxy, the native adapter). An
  epoch is one admitted lifetime of a producer; a restarted producer is a new epoch. A stream is one
  payload sequence in an epoch. Stream zero carries `Connection`, `Recovery` and `Loss`.
- **capture-only**: bytes recorded and not decoded, because no decoder accepts the stream or the
  harness has no decoder.
- **decoder, observation**: a verifier component that turns the retained bytes of the streams it
  accepts into observations. Trait `DecoderV1`; `DecoderRegistry::builtin()` lists six.
  `protocol_hint` is open, grammar-checked metadata that tells the registry which decoders to offer.
- **gap, Loss, Recovery**: section 7.
- **finish budget**: the one capture budget (180 s), split into a drain share and a 2 s commit
  reserve. Termination is outside it.
- **spawn gate**: the harness waits before `exec` until the daemon has durably registered it.
- **anchor**: the external binding of the chain head: `LocalOnly` or `Signed`.
- **run key**: the entry's per-run Ed25519 key. Its public half is `Launch.run_public_key`.
- **research-report schema** (now 5): the private NDJSON report format. Not the same as harness-contract
  schema 1 (`harness.toml`) or the decoder API version.

**Names kept from "Goal Cup Tracer" on purpose.** The product is Goal Cup Proxy. These
identifiers keep the old name, because changing them would break something that is already out:

- the server's database schema, `goal_cup_tracer`;
- `server/worker/sql/goal-cup-tracer-upload.sql`, the mirror of the website's migrations;
- the `tracer_version` field on the wire, which reports the `goalcup` version.

**The repository rename.** The repository is now `github.com/agentcommunity/goal-cup-proxy`. It
was `goal-cup-tracer`, and GitHub sends the old URL to the new one, so old install commands and
links still work. Every copy of the URL moved in one pull request: `packaging/install.sh` (tested
by `scripts/test_install.py`), the `AFTER_HELP` guide URL in `crates/goalcup-cli/src/main.rs`,
prep's setup guide link, and the URLs in the README, the rules, the guides and the skills. Archived
and dated documents keep the old URL as history. Never create a new repository under the old name:
that would end the redirect, and old install commands would fetch from it.

## 13. Where each guarantee is tested

Paths are under `crates/`.

| Guarantee | Tests |
|---|---|
| The daemon links no verifier; the CLI links no daemon | `goalcup-cli/tests/it/dependency_bans.rs`, `goalcup-cli/tests/it/seal_scan.rs` |
| Spawn gate: no exec before durable registration | `goalcup-cli/tests/it/capture_gate.rs`, `goalcup-cli/tests/m2_cli_lifecycle.rs`, `goalcup-cli/tests/full/capture_crash_campaign.rs` |
| Finish budget and termination outside it | `goalcup-daemon/tests/full/capture_finish_budget.rs`, `goalcup-daemon/src/runtime/capture/tests.rs`, `goalcup-cli/tests/full/capture_failure_campaign.rs` |
| Termination contract | `goalcup-daemon/tests/it/terminate_sim.rs`, `goalcup-daemon/tests/full/process_kill.rs`, `goalcup-cli/tests/full/launcher_sweep.rs` |
| Loss frames make a lossy run incomplete | `goalcup-cli/tests/it/m2_emitter.rs`, `goalcup-cli/tests/it/m2_proxy_integration.rs`, `goalcup-daemon/tests/it/capture_ingest.rs`, `goalcup-cli/tests/m2_cli_lifecycle.rs` |
| One orphan rule | `goalcup-daemon/src/runtime/capture/tests.rs`, `goalcup-daemon/tests/it/capture_ingest.rs`, `goalcup-verifier/tests/it/m2_capture_integrity.rs` |
| Integrity exits 0/1/2/3; baseline and seal binding | `goalcup-verifier/tests/it/m2_capture_integrity.rs`, `goalcup-verifier/tests/it/v5_corpus.rs` |
| Streaming equals whole-stream redaction; 16 KiB fail closed | `goalcup-daemon/tests/it/capture_redaction.rs`, `goalcup-daemon/tests/it/capture_redaction_extra.rs`, `goalcup-daemon/tests/it/redaction.rs` |
| The run's proxy credential redacted by exact value, wrapped and escaped forms included (D60) | `goalcup-protocol/src/run_secret.rs` (unit), `goalcup-daemon/tests/it/capture_redaction_run_secrets.rs`, `goalcup-cli/tests/full/per_run_lifecycle.rs` (`the_proxy_credential_a_harness_records_…`) |
| Native scope, no symlink, deferred scope | `goalcup-daemon/tests/it/capture_native.rs`, `goalcup-daemon/tests/it/capture_deferred_scope.rs` |
| TLS, CONNECT, HTTP and WebSocket relay | `goalcup-cli/tests/it/m2_tls_connect.rs`, `goalcup-cli/tests/it/m2_http_relay.rs`, `goalcup-cli/tests/it/m2_websocket_relay.rs` |
| Frozen wire contracts; connector pins | `goalcup-protocol/tests/it/m2_contract.rs`, `harness_schema.rs`, `harness_loader.rs`, `harness_conformance.rs`, `connector_pins.rs` (same folder) |
| v5 create, goal, launch, status through the daemon | `goalcup-daemon/tests/it/capture_lifecycle.rs` |
| Profile launch, executable resolution, env refusals | `goalcup-cli/tests/it/harness_launch_plan.rs`, `goalcup-cli/tests/it/start_launch.rs` |
| No provider key handed to a harness (D61), private home, stderr marker | `goalcup-cli/tests/it/connector_contract.rs`, `per_run_home.rs`, `capture_stderr_tee.rs`, `hermes_offline_key.rs` (same folder), `goalcup-cli/tests/full/per_run_lifecycle.rs`, `goalcup-protocol/tests/it/harness_schema.rs` |
| Every connector folder: the connector contract | `goalcup-cli/tests/it/connector_contract.rs` (static), `goalcup-cli/tests/full/connector_contract.rs` (one run each) |
| Crash, torn tail, restart | `goalcup-cli/tests/full/capture_crash_campaign.rs` |
| Whole vertical on a synthetic workload | `goalcup-cli/tests/full/m2_synthetic_vertical.rs` |
| Contract-kit cases | `goalcup-cli/tests/it/harness_contract_kit.rs` |
| Research report | `goalcup-verifier/tests/it/m2_research_report.rs` |
| Upload against the real Worker | `goalcup-cli/tests/full/upload_worker.rs`, `server/worker/test` |
