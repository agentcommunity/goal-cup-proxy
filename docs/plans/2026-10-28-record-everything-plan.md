# Record everything we safely can: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Goal Cup Proxy is a research recorder, so every trace holds as much usable context as can
be taken safely: every request on a host the proxy opens, every model host a connector knows, the
harness's whole session folder, and no path that hides model traffic. The fixes from the 28 Oct
four-reviewer audit of the [one capture system](2026-10-28-one-capture-system.md) land with it.

**Architecture:** The proxy records every exchange on an opened host, not only its model paths;
a model host read from config must be a clean URL. Connectors gain the model hosts they were
missing and, after a measurement, several native rows so a harness's session folder (subagent
transcripts, full tool-output files) is captured. Start probes only what the run needs. The upload
reads its side file safely. No v5 record changes.

**Tech Stack:** Rust 2024 (1.99), cargo-nextest, the upload Worker in TypeScript (`node --test`),
Vercel Sandbox for the gate and measured runs.

**Builds on:** [one capture system plan](2026-10-28-one-capture-system.md), its
[spec](2026-10-28-one-capture-system-spec.md) and [decision ledger](2026-10-28-one-capture-system-ledger.md).
New decisions go into that ledger as L30 and on.

## Owner decisions (28 Oct)

1. **Record as much as possible.** The traces are research data: at least every AI call and
   response, in full. "Every request under the model path" becomes "every request on an opened
   host", for config hosts and the connector's own default hosts alike.
2. **Wire first, chat file as backup.** Amp and Hermes (0.21.1 and later) keep no local chat
   file, so the wire is the whole record for them. Nothing may assume a chat file exists.
3. **Whole session folder, after a measurement.** Record subagent transcripts and full
   tool-output files too, once a sandbox run shows how big they get (Task 9).
4. **Non-AI traffic stays sealed** (npm, GitHub, pip): decrypting it breaks tools that keep their
   own certificate list and would catch push tokens. Host, port, bytes and times stay listed.
5. **No terminal recording for 6 Nov.** The AI calls already carry what the agent saw.
6. **Repeated history is a server problem.** Each request resends the conversation; a later
   stitcher rebuilds one chat per agent from the raw bytes, which stay as recorded (they are in
   the sealed chain). Upload size is handled by compression (Task 10).
7. **Labels are a hint.** Real analysis happens on the server; the verifier's `analysis.evidence`
   only needs to be a decent first guess.

## Owner decisions on wording (28 Oct, taken)

- **Goal sentence (L21): yes.** `CHAT_MODE_GOAL` becomes "chat mode (D57): the harness was given
  no goal; the participant typed the prompt in the harness's own terminal. It is recorded in the
  model requests and, when the harness keeps one, in its session file." New runs only; the
  verifier and Worker never match this text.
- **Finish line: yes.** `sealed hosts (passed through unread): github.com:443 x2 24992 KiB, …`,
  with `size unknown` instead of `? KiB` for cut tunnels.

## Global Constraints

- Everything in the one capture system plan's Global Constraints still holds (never set a base URL,
  env from nothing, v5 frozen, schema 1 additive, pins regenerated, no harness names in core code,
  plain English docs without em dashes, no future version numbers, keys never in argv or output).
- A widened schema bound (several native rows) is additive for new binaries only; say so in
  CONNECTOR-SCHEMA (older binaries refuse it, as for zero rows and `via`/`absent`).
- Measured runs use free gateway models (L17) unless a `DEEPSEEK_API_KEY` is present in a new
  session's environment; then DeepSeek through Hermes and the gateway are preferred.
- One focused test per behaviour; the seam tests in Task 8 are the ones the audit found missing.

## Review Focus

1. A gateway base path an agent crafts (`/x/../claude-code`, `%2e%2e`, a backslash) can no longer
   hide model calls: either it is refused at start, or every request on that host is recorded
   anyway (Tasks 1, 2).
2. A gateway user on a network that blocks `api.anthropic.com` or `chatgpt.com` starts, sets up
   and continues normally (Task 3).
3. The upload can never send a file other than the tunnel list it wrote (Task 1).
4. Recording every request on an opened host keeps sign-in hosts sealed and never decrypts a host
   that was not opened before (Task 2).
5. A trace with the whole session folder stays within the disk reserve and upload budget measured
   in Task 9; a huge tool-output file is capped, never fatal (Task 9).

---

### Task 1: Close the security gaps the audit found

**Files:** `crates/goalcup-cli/src/model_host.rs` (`parse_url` ~:596, `read`),
`crates/goalcup-protocol/src/capture_policy.rs` (`TRACKER_HOSTS` :123, `capture_path_problem`
:1045), `crates/goalcup-cli/src/upload/uploader/finish.rs` (`seal_request` :236),
`server/worker/src/runs.ts` (`keepTunnels`), tests beside each.

- [ ] **Step 1: Clean base paths only.** In `read`, refuse a config URL whose path is not already
  in normal form: run `capture_path_problem` on it and also refuse `%2e`/`%2E`, `\`, and any
  character a WHATWG URL parser percent-encodes. Sentence: "`<key>` in <file> names a model
  address with an unusual path, which Goal Cup cannot record safely. Write the address plainly,
  for example https://ai-gateway.vercel.sh/claude-code." Unit test: `/x/../claude-code`,
  `/claude-code/%2e%2e/v1`, `/a\b` are refused; `/claude-code`, `/codex/v1` pass.
- [ ] **Step 2: Goal Cup's own server is never a model host.** `TRACKER_HOSTS = ["agentcommunity.org"]`;
  `read` refuses a host that `is_tracker_host` matches, with the existing D62 sentence. Unit test:
  `https://ingest.agentcommunity.org/v1` refused.
- [ ] **Step 3: A config host that is this computer is refused even when spelled as a name.**
  At start, resolve the config host once; if every address is loopback, refuse with the existing
  loopback sentence (`localtest.me`, `*.localhost.direct`). A resolve failure is not a refusal (the
  probe reports it). Unit test with a fake resolver.
- [ ] **Step 4: The upload reads only its own list.** `seal_request` opens `tunnels_file` with
  `O_NOFOLLOW`, requires a regular file named `tunnels-<entry_id>.jsonl` inside the run
  directory, and requires the first line to be
  `{"format":"goalcup-tunnels-v1","entry_id":"<entry_id>"}`; otherwise it sends no list and logs
  one host.log line. The Worker checks the same first line (400 `malformed` otherwise). Tests:
  a symlinked list is not sent; a list for another entry is not sent; the Worker refuses a body
  whose first line is wrong.
- [ ] **Step 5: Run** `cargo nextest run --offline -p goalcup-cli -p goalcup-protocol -E 'test(model_host) | test(tunnel)'`
  and `cd server/worker && npm test`. **Commit** `fix: refuse unclean gateway paths, the event server and links in the tunnel list`.

### Task 2: Every request on an opened host is recorded

**Files:** `crates/goalcup-cli/src/proxy/http.rs` (`select` :394), `crates/goalcup-cli/src/model_host.rs`
(`approved` :79), `crates/goalcup-cli/src/commands/start/proxy.rs` (`approved_hosts`), tests in
`crates/goalcup-cli/tests/it/m2_proxy_integration.rs` and `model_host_from.rs`.

**Interfaces:** an opened host records every request and response, whatever its path; per-path
protocol hints stay as today (they only pick a decoder). Sign-in hosts stay tunnels. A host that
was not opened before is not opened now.

- [ ] **Step 1: Test** `every_request_on_an_opened_host_is_recorded`: a fake model host opened
  from config; the harness sends `POST <base>/v1/messages`, `GET /v1/models` and
  `POST /other/path`; all three appear as wire streams, only the first with a hint.
- [ ] **Step 2: Test** `a_sign_in_host_stays_sealed_when_another_host_is_opened` (keep or extend
  the existing auth-host test).
- [ ] **Step 3: Implement** in `select`: an exchange on an opened host is captured whether or not
  its path matched a row; the hint lookup is unchanged. Check the generation counter at :305 still
  counts only model requests.
- [ ] **Step 4: Run** the proxy and model-host tests; the harness goldens
  (`GOALCUP_REGEN_HARNESS_GOLDEN=1` only if a golden records captured paths; inspect the diff).
- [ ] **Step 5: Commit** `feat(proxy): record every request on a host Goal Cup opens`.

### Task 3: Probe only what the run needs

**Files:** `crates/goalcup-cli/src/commands/start/phases.rs` (:108-118),
`crates/goalcup-cli/src/commands/prep.rs` (~:1003-1007), `continue` preflight, tests in
`tests/it/prep.rs` and `start_launch.rs`.

- [ ] **Step 1: Test** `a_gateway_user_whose_network_blocks_the_default_host_starts`: config names
  a reachable fake gateway; the connector's default host and sign-in host are unreachable; start
  and prep succeed and print one warning line naming the unreachable default host.
- [ ] **Step 2: Test** `an_unreachable_gateway_still_refuses`: the config gateway is unreachable;
  start is refused with the existing reachability sentence naming that host.
- [ ] **Step 3: Implement:** when `model_host::check` returns `Some`, the gateway probe is the hard
  check; default-host and sign-in-host probes become warnings. When it returns `None`, today's
  behaviour stays (the default hosts are the model service). Same in prep and `continue`.
- [ ] **Step 4: Commit** `fix(start): a gateway user is not refused for an unreachable default host`.

### Task 4: Codex with an API key is recorded

**Files:** `connectors/codex-cli/harness.toml`, `crates/goalcup-protocol/pins/connector-dirs.txt`
(by script), `docs/harnesses/codex-cli.md`, `docs/HARNESS-SETUP.md` (Codex section).

- [ ] **Step 1:** Add a `[[capture.hosts]]` row `host = "api.openai.com"` with `evidence_paths =
  ["/v1/responses"]`, hint `openai-responses`, comment "Codex signed in with an OpenAI API key
  (unmeasured; the laptop run confirms)". Re-pin; regenerate goldens; inspect.
- [ ] **Step 2: Test:** the shipped Codex connector opens `api.openai.com` and hints
  `/v1/responses` (extend `the_shipped_claude_and_codex_rows_fall_back_without_a_setting` or the
  connector contract test).
- [ ] **Step 3:** HARNESS-SETUP Codex: one line that an API-key login is recorded through
  `api.openai.com`. **Commit** `feat(connectors): Codex records an API-key login`.

### Task 5: The label stays a fair hint

**Files:** `crates/goalcup-verifier/src/capture_analysis/report.rs` (`evidence` :388),
`operations.rs` (`wire_opens` :88), `docs/RULES.md`, `docs/research/m2-report-format.md`, byte
fixtures (by `REGENERATE_REPORT_BYTES=1`).

- [ ] **Step 1:** `wire_undecoded` counts only generation-shaped wire streams: a hinted stream or
  a `POST`. A lone `GET` (model list, flags) no longer outranks `native_only`. Unit test with one
  `GET` and native activity: `native_only`.
- [ ] **Step 2:** RULES and m2-report-format: "native" means model work a native decoder read;
  Hermes and Amp chat files have no decoder, so their runs label `wire_only` or `none`, and a
  `none` run with sealed chat files is reviewed, not thrown out. The label is computed even when
  integrity fails; it is meaningful only next to a valid integrity check.
- [ ] **Step 3: Commit** `fix(verifier): a lone GET does not outrank native evidence`.

### Task 6: What the participant reads

**Files:** `crates/goalcup-cli/src/participant/refusals.rs` (:98), `commands/prep.rs` (~:863),
`model_host.rs` (`defaults_detail`), `proxy/tunnels.rs` (`summary_line`), `commands/start/phases.rs` (`CHAT_MODE_GOAL`).

- [ ] **Step 1:** For a connector whose model address is a key in a file (Claude Code, Codex), a
  refused value says what to do: "Edit or delete `<key>` in `<full path>`; without it Goal Cup
  records `<default hosts>`." Hermes keeps "choose a service in Hermes". Test: the refusal names
  the full path and the key.
- [ ] **Step 2:** The fallback line names the hosts it opens: `model service: api.anthropic.com:443
  (connector default; env.ANTHROPIC_BASE_URL not set in settings.json)`, and for Codex a set
  built-in provider is called set, not "not set".
- [ ] **Step 3:** the finish-line wording and the `CHAT_MODE_GOAL` text above; update the two tests
  that compare the goal constant (`tests/full/connector_contract.rs`, `tests/full/m2_synthetic_vertical.rs`).
- [ ] **Step 4: Commit** `fix(cli): refusals say which file and key to edit`.

### Task 7: Operations and stale docs

**Files:** `scripts/fetch-run.sh`, `docs/OPERATING.md`, `docs/SERVER-DESIGN.md` (:78),
`server/worker/README.md`, `TODO.md` (:80, :93), `docs/PARTICIPANT-GUIDE.md`, `docs/HARNESS-SETUP.md`
intro, `crates/goalcup-verifier/src/capture_analysis/registry.rs` (`synthetic_only`, `measured`),
`docs/CONNECTOR-SCHEMA.md` (older binaries refuse `via`/`absent`; deny words have no deprecation
window because no shipped connector uses them), `docs/plans/2026-10-28-one-capture-system.md`
(Task 10 release line).

- [ ] **Step 1:** `fetch-run.sh` also downloads `<seal prefix>/tunnels.jsonl` when present.
- [ ] **Step 2:** OPERATING and the Worker README: deploy order (Worker first, then the client
  release; a new client sealing against an old Worker loses that entry's list for good), the new
  R2 object, `MIN_COMPETITION_VERSION` after the release.
- [ ] **Step 3:** PARTICIPANT-GUIDE: what the `sealed hosts:` line means and that host names,
  intranet ones included, go to the event server with the seal. TODO.md stale lines. The registry
  records the measured Claude Code 2.1.312 gateway run and the `.../messages` rule.
- [ ] **Step 4:** Plan Task 10 release line: v0.7.0 is tagged (26 Oct); `main` holds D73 and D74
  on top; the next release is prepared with `scripts/release.sh prepare`.
- [ ] **Step 5:** `python3 scripts/check-links.py`, `python3 scripts/check-version.py`.
  **Commit** `docs: deploy order, the tunnel list for operators and participants`.

### Task 8: The three seams nobody tested

**Files:** `crates/goalcup-cli/src/model_host.rs` tests or a verifier integration test,
`crates/goalcup-cli/tests/it/upload_server.rs`, `restarts.rs` or `start_launch.rs`.

- [ ] **Step 1: Hints meet decoders.** For each base `/claude-code`, `/codex/v1`, `/v1`, `` (empty):
  take the paths `ModelHost::approved()` hints and assert the decoder for each hint accepts that
  path (the bug L27 found by hand).
- [ ] **Step 2: The seal carries the list.** A real start against the fake upload server: the seal
  POST body has `tunnels_jsonl` whose first line names the entry.
- [ ] **Step 3: Continue keeps two lists.** A run with one `continue` leaves two
  `tunnels-<entry>.jsonl` files, one per entry, each sent with its own seal.
- [ ] **Step 4: Commit** `test: hints meet decoders; the seal carries its list; continue keeps both`.

### Task 9: Measure, then record the whole session folder

**Measurement first (no code):** a sandbox run of Claude Code and Codex through the free gateway
(or DeepSeek if a key is present), prompt "Build a working copy of PayPal … use subagents", five
minutes. Results in the ledger: every file in the session folder with size and kind, what is
captured today, trace size, repetition, and compression (Task 10). Decide per harness which
files to add and the size cap.

**Files:** `crates/goalcup-protocol/src/harness/check.rs` (`check_native` bound 0..=1 to 0..=N,
N = 8), `schema.rs` doc, `crates/goalcup-daemon/src/runtime/capture/native_plan.rs` (several
rows), the native adapter (one stream per row), `connectors/claude-code-cli/harness.toml`
(rows for `*/*/subagents/*.jsonl` `append_only` and `*/*/tool-results/*` `snapshot` with a cap),
Codex rows per the measurement, pins, goldens, CONNECTOR-SCHEMA, a frozen fixture with two rows.

- [ ] **Step 1: Test** a connector with two native rows records both streams and seals; three
  overlapping patterns that match the same file are refused (one file, one stream).
- [ ] **Step 2: Implement** the widened bound and one native stream per row; each row keeps its
  own form, scope and hint rules; a per-file size cap on `snapshot` rows (default from the
  measurement, for example 8 MiB) records a cut marker instead of failing.
- [ ] **Step 3: Connectors** per the measurement; re-pin; goldens; the native-silent warning
  still watches only the main row.
- [ ] **Step 4: Measured run** again: subagent transcripts and spilled tool output are in the
  trace; the trace stays within the disk reserve.
- [ ] **Step 5: Commit** `feat: record the harness's whole session folder`.

### Task 10: Compress repeated history in the upload (after the measurement)

**Files:** `crates/goalcup-cli/src/upload/uploader/chunk.rs` (`ZSTD_LEVEL` :19, no long window
today), `scripts/fetch-run.sh` (`zstd -dcq`), any server-side reader of chunks.

- [ ] **Step 1:** If Task 9's measurement shows `--long=27` cuts the compressed chain clearly
  (for example by half), enable long-distance matching with window log 27. Window log 27 is the
  default decoder limit of both the zstd CLI and the Rust crate, so existing readers decode it
  without a flag; a larger window would need every reader changed, so do not go above 27.
- [ ] **Step 2: Test:** a chunk written with the long window decodes with the default decoder
  (`fetch-run.sh` path and the Rust reader); a test chain with repeated 1 MiB prefixes compresses
  smaller than at today's settings.
- [ ] **Step 3: Commit** `perf(upload): long-window compression for repeated history`.

### Task 11: Reviews, gates, measured runs, merge

- [ ] **Step 1:** Two fresh reviewers per PR (correctness, holistic), given the diff and this plan.
- [ ] **Step 2:** `sh scripts/gate.sh fast` on the sandbox gate snapshot per PR (worker tests when
  `server/` changes). Gate runner: the detached runner with the done marker (L18 notes).
- [ ] **Step 3:** Integrated sandbox runs: Claude Code, Codex and Hermes through the gateway; one
  crafted `/x/../claude-code` base path (refused at start); a Claude run that makes a `GET` on the
  gateway (recorded); a run with a blocked default host (starts).
- [ ] **Step 4:** Merge order: Task 1, Task 2, Task 3, Task 8, Task 5, Task 4, Task 6, Task 7,
  then Tasks 9 and 10 after the measurement. Update the ledger and results file.

### Task 12: Owner steps (not for agents)

- [ ] Deploy the Worker before any client from `main` seals (Task 1 adds a Worker check too).
- [ ] Prepare and tag the next release; set `MIN_COMPETITION_VERSION`.
- [ ] Laptop runs ([LAPTOP-LOGIN-RUNS](../research/LAPTOP-LOGIN-RUNS.md)), plus run 3 now confirms
  the new `api.openai.com` row; one real upload and `fetch-run.sh` of a practice run.

## Later (not before 6 Nov unless cheap)

- Refuse proxy-override keys a harness reads from its own settings (`env.HTTPS_PROXY`,
  `env.NO_PROXY`, `env.NODE_EXTRA_CA_CERTS` in Claude's `settings.json`): needs a new connector
  field.
- A per-host count of exchanges on opened hosts in the finish line (moot once Task 2 records them).
- The stitcher that rebuilds one chat per agent and subagent from repeated requests (server side).
- A label for "chat files recorded, no decoder" (Hermes, Amp, OpenCode, OpenClaw).
