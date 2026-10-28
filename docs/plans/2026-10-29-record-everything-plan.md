# Record everything we safely can: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every trace holds every AI call and answer of the main agent and its subagents, with
headers and every session file, through one rule set that 10 to 15 harnesses and custom ones fit.

**Architecture:** The recording proxy records every exchange on an opened host (path rules only
pick the decoder) and adds a credential-masked header stream per exchange; connectors list every
model host their harness can use and up to 8 session-file rows; the start path warns instead of
refusing except where recording is impossible; the proxy restarts itself; all new data enters the
v5 chain as new streams that older verifiers skip.

**Tech Stack:** Rust 2024 (1.99), cargo-nextest, zstd 0.13.3, the upload Worker in TypeScript
(`node --test`), Vercel Sandbox for gates, Tenki for the 168-hour run.

**Spec:** [2026-10-29-record-everything-spec.md](2026-10-29-record-everything-spec.md) (rules 1 to 17).
Background: [one capture system plan](2026-10-28-one-capture-system.md) and its
[decision ledger](2026-10-28-one-capture-system-ledger.md); new decisions go in that ledger from L30.

## Global Constraints

- Never set a model base URL; never read, pass or store a key (D1, D36, D61). Keys reach a sandbox
  only as a 0600 file and go into the harness's own settings there; never in argv, output or a commit.
- Protocol v5 is frozen (D54): no new record type, field or `CaptureSource`. New data is a new
  stream with an existing source and a new `protocol_hint`. The v5 corpus and the verifiers built
  from v0.5.0, v0.7.0 and `main` must read every new trace as `valid`.
- Schema 1 only grows additively; every widened bound is refused by older binaries, and
  CONNECTOR-SCHEMA says so.
- Header values are masked when the name is a credential name or the value looks like a key:
  names `authorization`, `proxy-authorization`, `cookie`, `set-cookie`, `x-api-key`, `api-key`,
  `x-goog-api-key`, and any name containing `key`, `token`, `secret`, `password` or `auth`; values
  through the daemon's existing redactor. Mask text: `[masked]`.
- Refuse only plain-http and loopback model addresses (D62). Everything else warns and records.
- Every connector edit reruns `sh scripts/pin-manifests.sh` in the same commit; goldens regenerate
  with `GOALCUP_REGEN_HARNESS_GOLDEN=1`; report bytes with `REGENERATE_REPORT_BYTES=1`.
- No harness names in core code (`no_family_literals.rs`). Docs plain English, no em dashes, no
  future version numbers (`scripts/check-version.py`).
- One focused test per behaviour; `sh scripts/gate.sh fast` passes on the sandbox gate snapshot
  before a PR is merge-ready; measured runs are the proof.
- Test models: Codex GPT-6.1 Luma low, DeepSeek latest flash (`-1017`) low thinking, Claude
  Haiku low thinking, the same model for subagents; free gateway models for small checks only.

## Review Focus

1. A crafted base path (`/x/../claude-code`, `%2e%2e`, a backslash) never hides calls: it is
   refused at start, and every request on the host is recorded anyway (Tasks 1, 2).
2. A token in a request or response body on an opened host (`"refresh_token":"…"`) is masked in
   the chain (Task 1, Step 2).
3. A subagent's calls and its transcript are both in the trace: a Claude run with 3 subagents has
   3 subagent transcript streams and 4 distinct conversations on the wire (Task 5, Task 13).
4. A gateway user on a network that blocks the default host starts and continues (Task 4).
5. The proxy dying mid-run does not end the agent's network: the next request still goes through
   and is recorded (Task 10).

---

### Task 1: Record every request on an opened host

**Files:**
- Modify: `crates/goalcup-cli/src/proxy/http.rs:394-420` (`select`), `:258-310` (`relay_requests`)
- Modify: `crates/goalcup-cli/src/model_host.rs:79-106` (`approved`: drop the comment "never records every request on the host"; keep hints)
- Test: `crates/goalcup-cli/tests/it/m2_proxy_integration.rs`

**Interfaces:**
- Produces: every exchange on an approved host gets an `ExchangeCapture`; `ExchangeMeta.protocol_hint`
  is `metadata::select(policy, path).and_then(|p| p.protocol_hint.clone())`. `metadata::select`
  keeps its signature and is now only the hint lookup. Counters: `record_recorded_request` for
  every exchange; `record_generation_request` when the hint is set or the method is `POST` (unchanged rule).

- [ ] **Step 1: Write the failing test** `every_request_on_an_opened_host_is_recorded`: fake model
  host approved with one hinted path `/v1/messages`; the client sends `POST /v1/messages`,
  `GET /v1/models`, `POST /x/other`. Assert three request streams and three response streams in
  the chain; only the first carries hint `anthropic-messages`; `recorded_requests == 3`.
- [ ] **Step 2: Write the failing test** `a_token_in_a_body_on_an_opened_host_is_masked`:
  `POST /oauth/token` with body `{"refresh_token":"rt-0123456789abcdefghij"}`; assert the chain
  bytes do not contain `rt-0123456789abcdefghij`.
- [ ] **Step 3: Run** `cargo nextest run --offline -p goalcup-cli -E 'test(every_request_on_an_opened_host) | test(token_in_a_body)'`. Expected: the first FAILS (only 1 stream), the second PASSES (redactor already covers it; keep it as the pin).
- [ ] **Step 4: Implement** in `http.rs` `select`: return `Some(ExchangeCapture{..})` for every exchange that passes `metadata::is_method`, with the hint from the lookup above. WebSocket capture follows the same decision (Handoff at `:519`).
- [ ] **Step 5: Run** the two tests, then `cargo nextest run --offline -p goalcup-cli -E 'test(proxy) | test(model_host) | test(opaque_tunnels)'`. Expected: PASS; update any assertion that an unlisted path is not recorded.
- [ ] **Step 6: Commit** `feat(proxy): record every request on a host Goal Cup opens`

### Task 2: A model address must be clean and never Goal Cup's own

**Files:**
- Modify: `crates/goalcup-cli/src/model_host.rs:201-305` (`read`), `:596` (`parse_url`)
- Modify: `crates/goalcup-protocol/src/capture_policy.rs:123` (`TRACKER_HOSTS`), `:1045` (`capture_path_problem` becomes `pub`)
- Modify: `crates/goalcup-cli/src/commands/start/phases.rs:107-121` (loopback-by-resolution check)
- Test: unit tests in `model_host.rs`; `crates/goalcup-cli/tests/it/model_host_from.rs`

**Interfaces:**
- Produces: `pub fn capture_path_problem(path: &str) -> Option<&'static str>`;
  `TRACKER_HOSTS = &["agentcommunity.org"]`; `fn resolves_only_to_loopback(host: &str, port: u16) -> bool` in `crates/goalcup-cli/src/proxy/preflight.rs`.

- [ ] **Step 1: Write failing unit tests:**
  `an_unclean_base_path_is_refused`: `https://ai-gateway.vercel.sh/x/../claude-code`,
  `https://g.example.com/a/%2e%2e/v1`, `https://g.example.com/a\b` → `Err(u)`, `u.reason` contains
  "unusual path"; `https://ai-gateway.vercel.sh/claude-code` and `/codex/v1` → `Ok(Some(_))`.
  `the_event_server_is_never_a_model_host`: `https://ingest.agentcommunity.org/v1` → `Err`.
- [ ] **Step 2: Write the failing test** `a_name_that_resolves_to_this_computer_is_refused`
  (`resolves_only_to_loopback("localtest.me", 443)` with an injected resolver returning `127.0.0.1` → true; a resolve error → false).
- [ ] **Step 3: Run** `cargo nextest run --offline -p goalcup-cli -E 'test(unclean_base_path) | test(event_server) | test(resolves_to_this_computer)'`. Expected: FAIL.
- [ ] **Step 4: Implement:** in `read`, after `parse_url`, refuse when `capture_path_problem(path)` is `Some`, or the raw path contains `%2e`/`%2E`, `\`, or a byte a WHATWG parser percent-encodes; reason text "names an address with an unusual path, which Goal Cup cannot record safely. Write the address plainly, for example https://ai-gateway.vercel.sh/claude-code". Refuse `is_tracker_host`. In `prepare`, before the probe, refuse the config host when `resolves_only_to_loopback` with the existing loopback sentence.
- [ ] **Step 5: Run** the tests and `-E 'test(model_host)'`. Expected: PASS.
- [ ] **Step 6: Commit** `fix: refuse unclean gateway paths, the event server and names that resolve here`

### Task 3: The upload reads only its own tunnel list

**Files:**
- Modify: `crates/goalcup-cli/src/upload/uploader/finish.rs:236-281` (`seal_request`, `read_document`)
- Modify: `server/worker/src/runs.ts:517-603` (`sealRun`, `keepTunnels`)
- Test: `finish.rs` unit tests (:283); `server/worker/test/integration.test.ts:477-498`

**Interfaces:**
- Produces: `fn read_tunnels(path: &Path, run_dir: &Path, entry_id: &str) -> Option<String>`: opens with `O_NOFOLLOW`, requires a regular file named `tunnels-<entry_id>.jsonl` directly in `run_dir`, at most 8 MiB, first line exactly `{"format":"goalcup-tunnels-v1","entry_id":"<entry_id>"}`; else `None` plus one `host_log::log_only` line. The Worker refuses a `tunnels_jsonl` whose first line is not that object with 400 `malformed`.

- [ ] **Step 1: Write failing tests:** `a_linked_tunnel_list_is_not_sent` (symlink to a file with
  `SECRET-BYTES` → `tunnels_jsonl == None`); `a_list_for_another_entry_is_not_sent`;
  Worker `test("a tunnel list must start with its own format line", …)` → 400.
- [ ] **Step 2: Run** `cargo nextest run --offline -p goalcup-cli -E 'test(tunnel_list) | test(linked_tunnel)'` and `cd server/worker && npm test`. Expected: FAIL.
- [ ] **Step 3: Implement** `read_tunnels` and use it in `seal_request`; keep `read_document` for seal/baseline unchanged. Worker: check the first line in `sealRun` before `keepTunnels`.
- [ ] **Step 4: Run** the same commands. Expected: PASS.
- [ ] **Step 5: Commit** `fix(upload): send only the tunnel list this entry wrote`

### Task 4: Probe only what the run needs

**Files:**
- Modify: `crates/goalcup-cli/src/commands/start/phases.rs:107-121` (`prepare`)
- Modify: `crates/goalcup-cli/src/commands/prep.rs:996-1030` (`machine_step`)
- Modify: `crates/goalcup-cli/src/home.rs:423` (0700: fix the mode of a home Goal Cup created)
- Test: `crates/goalcup-cli/tests/it/prep.rs`, `start_launch.rs`

**Interfaces:**
- Consumes: `model_host::for_run` (`Option<ModelHost>`).
- Produces: when `for_run` is `Some`, `endpoints_reachable([model host])` is the hard check;
  `upstreams_reachable(profile.hosts)` failures become one line via `commands::say`:
  "warning: <host>:443 could not be reached; your settings use <gateway>, so the run goes on." Prep
  treats default and sign-in hosts the same way (`[warn]`). When `for_run` is `None`, today's behaviour.

- [ ] **Step 1: Write failing tests:** `a_gateway_user_whose_network_blocks_the_default_host_starts`
  (fake gateway reachable, default host unreachable: start succeeds and the output has the
  warning line); `an_unreachable_gateway_still_refuses` (the existing refusal text naming the gateway);
  `a_home_goal_cup_made_with_a_loose_mode_is_fixed` (home created 0755 → start sets 0700 and goes on).
- [ ] **Step 2: Run** `cargo nextest run --offline -p goalcup-cli -E 'test(blocks_the_default_host) | test(unreachable_gateway) | test(loose_mode)'`. Expected: FAIL.
- [ ] **Step 3: Implement** as in Interfaces. In `home.rs`, a home under `<cfg>/capture-homes` owned by this user gets `chmod 0700` and one log line; any other unsafe home keeps today's refusal.
- [ ] **Step 4: Run** the tests and `-E 'test(prep) | test(start_launch)'`. Expected: PASS.
- [ ] **Step 5: Commit** `fix(start): warn, not refuse, for a host the run does not use`

### Task 5: Up to 8 session-file rows; Claude subagents captured

**Files:**
- Modify: `crates/goalcup-protocol/src/harness/check.rs:1029` (`check_native`), `:1375-1508` (`check_cross_field`)
- Modify: `crates/goalcup-protocol/src/harness/schema.rs:488-490` (doc: "Zero to eight entries")
- Modify: `crates/goalcup-daemon/src/runtime/capture/native_plan.rs:18-65` (`native_plan`: every row)
- Modify: `connectors/claude-code-cli/harness.toml:92-98`, pins, goldens
- Create: `crates/goalcup-cli/tests/fixtures/schema-1-connectors/schema1-two-native/harness.toml`
- Modify: `docs/CONNECTOR-SCHEMA.md`, `docs/HARNESS-ONBOARDING.md:413,449-450`, `skills/adding-a-capture-harness/templates/connector-harness.toml:166-178`
- Test: `crates/goalcup-protocol/tests/it/harness_schema.rs:341`, daemon `capture/tests.rs`, `tests/full/connector_contract.rs`

**Interfaces:**
- Produces: `pub const MAX_NATIVE_ROWS: usize = 8;` in `harness/check.rs` (equal to the daemon's
  `MAX_NATIVE_SOURCES`); rows need unique `name`s and distinct `(root, pattern)`; every
  cross-field rule runs per row. `native_plan` returns one `NativeSourceSpec` per row.
- Claude rows added: `name="subagents" root="{config_home}/projects" pattern="*/*/subagents/agent-*.jsonl" form="append_only" protocol_hint="claude-code-jsonl" since_run_start=true warn_missing=false`;
  `name="subagent-meta" … pattern="*/*/subagents/agent-*.meta.json" form="snapshot" since_run_start=true warn_missing=false`;
  `name="tool-results" … pattern="*/*/tool-results/*" form="snapshot" since_run_start=true warn_missing=false`.

- [ ] **Step 1: Write failing tests:** `up_to_eight_native_sources_are_admitted` (3 rows load);
  `nine_native_sources_are_refused`; `two_rows_with_one_name_are_refused`;
  daemon `every_native_row_is_planned` (3 rows → 3 specs); full contract run of `schema1-two-native` records both streams and seals `valid`.
- [ ] **Step 2: Run** `cargo nextest run --offline -p goalcup-protocol -p goalcup-daemon -E 'test(native_source) | test(native_row)'`. Expected: FAIL.
- [ ] **Step 3: Implement** the loop over rows in `check_native` and `check_cross_field` (replace `.first()` at `:1397`), and in `native_plan`.
- [ ] **Step 4: Add the Claude rows**; `sh scripts/pin-manifests.sh`; regenerate goldens; inspect: only the Claude connector lines change.
- [ ] **Step 5: Run** `cargo nextest run --offline -p goalcup-protocol -p goalcup-daemon -p goalcup-cli -E 'test(native) | test(connector_contract) | test(golden) | test(pin)'`. Expected: PASS.
- [ ] **Step 6: Commit** `feat: several session-file rows; Claude subagent transcripts captured`

### Task 6: Copy a harness database once, after the harness exits

**Files:**
- Modify: `crates/goalcup-protocol/src/harness/schema.rs:152-160` (`NativeForm` gains `FinalSnapshot = "final_snapshot"`)
- Modify: `crates/goalcup-daemon/src/native_capture.rs` (`NativeStoreForm`; `finish` at `:642` reads final-snapshot files once)
- Modify: `connectors/hermes-cli/harness.toml` (row `name="state" root="{config_home}" pattern="state.db*" form="final_snapshot" warn_missing=false`), pins, goldens
- Test: daemon `native_capture` unit tests; `crates/goalcup-cli/tests/it/hermes_offline_key.rs`

**Interfaces:**
- Produces: a `final_snapshot` row is never polled during the run; at `NativeCapture::finish`,
  after the harness exited, each matching file is read once into its own stream
  (`state.db`, `state.db-wal`, `state.db-shm`), within `NATIVE_FINISH_BYTES` (64 MiB);
  over the budget writes the existing `Gap` and the seal goes on.

- [ ] **Step 1: Write failing tests:** `a_final_snapshot_file_is_read_only_at_finish` (file
  written during the run: no stream until finish, then exactly one stream with the final bytes);
  `a_final_snapshot_over_budget_leaves_a_gap`.
- [ ] **Step 2: Run** `cargo nextest run --offline -p goalcup-daemon -E 'test(final_snapshot)'`. Expected: FAIL.
- [ ] **Step 3: Implement** the form and the finish read; schema check: `final_snapshot` takes no `protocol_hint`.
- [ ] **Step 4: Add the Hermes row**; re-pin; goldens. **Run** `-p goalcup-daemon -p goalcup-protocol -p goalcup-cli -E 'test(final_snapshot) | test(hermes) | test(golden)'`. Expected: PASS.
- [ ] **Step 5: Commit** `feat: copy a harness database once after the harness exits`

### Task 7: A header stream per exchange

**Files:**
- Modify: `crates/goalcup-cli/src/proxy/http.rs:745-830` (`Fields::parse` keeps the raw head), the request open (`:343-355`) and response open (`:478-503`)
- Create: `crates/goalcup-cli/src/proxy/headers.rs`
- Test: `headers.rs` unit tests; `crates/goalcup-verifier/tests/it/` a v5 compatibility test; `m2_proxy_integration.rs`

**Interfaces:**
- Produces: `pub const HEADERS_HINT: &str = "http-head-v1";`
  `pub fn masked_head(head: &[u8]) -> Vec<u8>`: the start line and each header line as received,
  values replaced by `[masked]` for the Global Constraints names; the daemon redactor then runs
  as on every stream. For every recorded exchange the proxy opens one `WireRequest` and one
  `WireResponse` stream with `protocol_hint = Some(HEADERS_HINT)`, the exchange's `WireIdentity`,
  writes the masked head and ends it.

- [ ] **Step 1: Write failing tests:** `a_credential_header_value_is_masked`
  (`authorization`, `x-api-key`, `x-goog-api-key`, `cookie`, `x-my-auth-token` → `[masked]`;
  `user-agent: claude-cli/2.1.312`, `anthropic-version`, `x-request-id` kept);
  `every_recorded_exchange_has_header_streams` (integration: 2 exchanges → 4 header streams, no
  `Bearer` value in the chain); `an_old_verifier_reads_a_trace_with_header_streams`
  (verifier report `integrity == "valid"`, `analysis.undecoded` lists `http-head-v1`).
- [ ] **Step 2: Run** `cargo nextest run --offline -p goalcup-cli -p goalcup-verifier -E 'test(header)'`. Expected: FAIL.
- [ ] **Step 3: Implement** `masked_head` and the two opens per exchange, copying the 101 empty-stream pattern at `http.rs:493-503`.
- [ ] **Step 4: Run** the tests and the v5 corpus `-p goalcup-verifier -E 'test(v5_corpus)'`. Expected: PASS. Check by hand that `goalcup-verify` built from the latest release tag reads one new trace as `valid`.
- [ ] **Step 5: Commit** `feat(proxy): keep request and response headers, credentials masked`

### Task 8: The label counts model requests only

**Files:**
- Modify: `crates/goalcup-verifier/src/capture_analysis/report.rs:362-400` (`Legs::of`, `evidence`), `operations.rs:88`
- Modify: `docs/RULES.md` ("What a label is worth"), `docs/research/m2-report-format.md`, report-byte fixtures
- Test: `report.rs` unit tests

**Interfaces:**
- Produces: `wire_streams` counts only request `Open`s whose hint is set and is not `http-head-v1`
  or `goalcup-tunnels-v1`, or whose method is `POST` (the proxy's own rule).

- [ ] **Step 1: Write the failing test** `a_lone_get_and_header_streams_do_not_outrank_native`: native activity, one `GET` stream, header streams, no decoded request → `"native_only"`.
- [ ] **Step 2: Run** `cargo nextest run --offline -p goalcup-verifier -E 'test(lone_get)'`. Expected: FAIL.
- [ ] **Step 3: Implement**; RULES: "native" means model work a native decoder read; Hermes and Amp chat files have no decoder, so their runs label `wire_only` or `none`, and a `none` run with sealed chat files is reviewed, not thrown out.
- [ ] **Step 4: Run** `REGENERATE_REPORT_BYTES=1 cargo nextest run --offline -p goalcup-verifier`; inspect: only `evidence` changes where expected. Expected: PASS.
- [ ] **Step 5: Commit** `fix(verifier): count only model requests in the evidence label`

### Task 9: Every model host a harness can use

**Files:**
- Modify: `connectors/codex-cli/harness.toml:120-123` (add host `api.openai.com`, `evidence_paths=["/v1/responses"]`, hint `openai-responses`), pins, goldens
- Modify: `docs/harnesses/codex-cli.md`, `docs/HARNESS-SETUP.md` (Codex API key; Claude API key in `settings.json` `env.ANTHROPIC_API_KEY`; pin the subagent model when using a gateway)
- Test: `crates/goalcup-cli/tests/it/model_host_from.rs` (`the_shipped_claude_and_codex_rows_fall_back_without_a_setting`)

- [ ] **Step 1: Extend the test:** the shipped Codex connector approves `api.openai.com` with hint `openai-responses` on `/v1/responses`. **Run** `-E 'test(shipped_claude_and_codex)'`. Expected: FAIL.
- [ ] **Step 2: Add the row**; re-pin; goldens. **Run** again. Expected: PASS.
- [ ] **Step 3: Docs**, then **Commit** `feat(connectors): Codex records an API-key login`

### Task 10: The proxy restarts itself on the same port

**Files:**
- Modify: `crates/goalcup-cli/src/proxy/mod.rs:213` (`start`), `:282`/`:920` (`accept_loop`), `:338` (`ProxyHandle`)
- Test: `crates/goalcup-cli/tests/it/m2_proxy_integration.rs`

**Interfaces:**
- Produces: `accept_loop` runs under a supervisor in `ProxyHandle`: if the accept task ends
  without a stop, the supervisor rebinds `local_addr` (same port) and respawns `accept_loop` with
  the same `Arc<Shared>`; at most 5 restarts per run, each one a `host_log` line and a run-state
  problem "the recorder restarted N time(s)". A test hook `ProxyHandle::kill_accept_for_test()` behind `#[cfg(test)]` or a test feature.

- [ ] **Step 1: Write the failing test** `a_dead_accept_loop_is_restarted_on_the_same_port`: kill the accept task; a new request through the same proxy URL succeeds and is recorded.
- [ ] **Step 2: Run** `cargo nextest run --offline -p goalcup-cli -E 'test(accept_loop_is_restarted)'`. Expected: FAIL.
- [ ] **Step 3: Implement** the supervisor.
- [ ] **Step 4: Run** the test and `-E 'test(proxy)'`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(proxy): restart the recorder on the same port if it dies`

### Task 11: What the participant sees

**Files:**
- Modify: `crates/goalcup-cli/src/commands/start/finish.rs:369-430` (recording line), `proxy/tunnels.rs:142` (`summary_line` wording), `commands/start/phases.rs:68` (`CHAT_MODE_GOAL`), `participant/refusals.rs:95-98`, `commands/prep.rs:857-863`
- Test: `tests/it/start_launch.rs`; `tests/full/connector_contract.rs:346`, `tests/full/m2_synthetic_vertical.rs:155` (goal text)

**Interfaces:**
- Produces: at finish, after the admission line, one line: `Recording works: <n> model call(s) recorded from <host>[, <host>].` or, when `n == 0`, the existing `no_model_calls_block`. Summary line: `sealed hosts (passed through unread): <host>:<port> x<n> <KiB> KiB, …`, `size unknown` for a host whose tunnels all ended cut. `CHAT_MODE_GOAL` = "chat mode (D57): the harness was given no goal; the participant typed the prompt in the harness's own terminal. It is recorded in the model requests and, when the harness keeps one, in its session file." A refused model address for a file-based connector ends: "Edit or delete `<key>` in `<full path>`; without it Goal Cup records <default hosts>."

- [ ] **Step 1: Write failing tests:** `a_practice_run_says_recording_works` (fake model host, 2 calls → the line with `2 model call(s)`); `a_refused_address_says_which_file_to_edit` (full path and key in the text).
- [ ] **Step 2: Run** `cargo nextest run --offline -p goalcup-cli -E 'test(recording_works) | test(which_file_to_edit)'`. Expected: FAIL.
- [ ] **Step 3: Implement** the texts; update the two goal-text tests.
- [ ] **Step 4: Run** `-E 'test(start_launch) | test(prep) | test(tunnel)'` and `-p goalcup-cli --test full -E 'test(connector_contract) | test(m2_synthetic_vertical)'`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(cli): say plainly that recording works, and which file to edit`

### Task 12: Seams, operations and docs

**Files:**
- Test: `crates/goalcup-cli/src/model_host.rs` tests (hints meet decoders); `tests/it/upload_server.rs` (seal carries the list); `tests/it/restarts.rs` (continue keeps two lists)
- Modify: `scripts/fetch-run.sh:142-277` (also fetch `<prefix>/tunnels.jsonl` when present, no sha check, missing is fine)
- Modify: `docs/OPERATING.md`, `server/worker/README.md` (deploy the Worker before the client release; `MIN_COMPETITION_VERSION`), `docs/CONTRACTS.md:541` (no client sends `protocol_version`: say the Worker's version gate is `MIN_COMPETITION_VERSION` only), `docs/SERVER-DESIGN.md:78` (the `tunnels.jsonl` object), `docs/RULES.md` (what a trace holds: everything the agent sent and saw, headers without credentials, file contents and paths on your machine), `docs/PARTICIPANT-GUIDE.md` (the recording line and the sealed-hosts line), `TODO.md:80,93`, `crates/goalcup-verifier/src/capture_analysis/registry.rs` (`synthetic_only`, `measured`: the Claude Code 2.1.312 gateway run), `docs/OPERATING.md` (harness and version per entry are in `entries.connector_id` and `entries.harness_version`; models used per run are in the research report's per-decoder model counts)

- [ ] **Step 1: Write the three seam tests:** `every_hinted_path_is_read_by_its_decoder` (bases `/claude-code`, `/codex/v1`, `/v1`, empty: each hinted path from `ModelHost::approved()` is accepted by the decoder for its hint); `the_seal_carries_this_entrys_tunnel_list`; `continue_keeps_one_tunnel_list_per_entry`.
- [ ] **Step 2: Run** `cargo nextest run --offline -p goalcup-cli -E 'test(hinted_path_is_read) | test(seal_carries) | test(one_tunnel_list_per_entry)'`. Expected: PASS where the code is right; fix the code only if one fails.
- [ ] **Step 3: fetch-run.sh** change; run it against a local fixture if the script has one, else by reading.
- [ ] **Step 4: Docs**; `python3 scripts/check-links.py` (0 broken), `python3 scripts/check-version.py`.
- [ ] **Step 5: Commit** `test,docs: seams, deploy order and what a trace holds`

### Task 13: Measured runs, then the long runs

- [ ] **Step 1: Gate each PR** on the sandbox gate snapshot (`sh scripts/gate.sh fast`; worker tests when `server/` changes). Two fresh reviewers per PR (correctness, holistic) given the diff, this plan and the spec.
- [ ] **Step 2: Integrated build** (main after Tasks 1 to 12), in a new cloud session with `OPENAI_API_KEY`, `DEEPSEEK_API_KEY`, `ANTHROPIC_API_KEY`, `AI_GATEWAY_API_KEY`. Prompt "Build a working copy of PayPal … use subagents", 15 minutes each, practice mode:
  - Claude Code, API key, Haiku low (main and subagents).
  - Codex, API key, GPT-6.1 Luma low (main and subagents).
  - Hermes, DeepSeek flash.
  - Claude Code and Codex through the gateway (free model), short.
  Pass for each: integrity `valid`; "Recording works" line; `analysis.evidence` `wire_and_native` (Hermes `wire_only`); every subagent transcript on disk is in the chain byte for byte; distinct wire conversations equal 1 + subagents; header streams present with no credential value; the model hosts absent from the sealed-hosts line.
- [ ] **Step 3: Negative runs:** `/x/../claude-code` base path refused at start; a blocked default host with a gateway starts; killing the proxy's accept task mid-run (test build) keeps recording.
- [ ] **Step 4: Merge** in this order: Task 3, Task 2, Task 1, Task 8, Task 7, Task 4, Task 9, Task 5, Task 6, Task 10, Task 11, Task 12. Rebase each; rerun the gate when Rust changed.
- [ ] **Step 5: Long runs:** 24 hours and 168 hours on Tenki with the funded key, Claude Code and Codex, from the merged `main`; a `goalcup continue` after any box stop; record cost, trace size per hour, memory, upload lag; rebuild the trace from R2 with `fetch-run.sh` and verify it.
- [ ] **Step 6: Ledger and results file** updated with every measured number.

### Task 14: Owner steps (not for agents)

- [ ] Deploy the Worker (Task 3 adds a check) before any client from `main` seals.
- [ ] Prepare and tag the release; set `MIN_COMPETITION_VERSION`.
- [ ] Fund the long-run key; give the Tenki access; laptop subscription runs ([LAPTOP-LOGIN-RUNS](../research/LAPTOP-LOGIN-RUNS.md)).
- [ ] Decide whether a custom connector (`--dev`) may compete, flagged (today D70 refuses it).

## Next plan (separate): the harness batch

10 to 15 harnesses as connectors on this shape, each with a facts sheet, `scripts/check-connector.sh`
and a 15-minute measured run meeting Task 13 Step 2's pass list. Written after Tasks 1, 5 and 9 merge.

## After the event

v6 (raw bytes instead of base64; the tunnel list and headers as first-class records), the server
flag engine and heartbeat-head check, the stitcher, smart transfer, a generic connector.
