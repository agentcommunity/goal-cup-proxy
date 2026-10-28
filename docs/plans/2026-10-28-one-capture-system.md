# One capture system implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every harness records through one rule set: model host from the connector's defaults or the
harness's own config, an optional chat file, every sealed connection listed and uploaded, and an
evidence label on each run.

**Architecture:** Additive schema-1 fields on `capture.model_host_from` (`via`, `absent`) and a
widened native bound; the CLI reader resolves one indirection and falls back to the connector's host
rows; the recording proxy keeps each tunnel's host, port, times and bytes, writes them at finish to a
side file sent with the seal; the verifier adds `analysis.evidence`. No v5 record changes.

**Tech Stack:** Rust 2024 (toolchain 1.99), cargo-nextest, TOML/JSON/YAML readers in
`goalcup-cli`, the upload Worker in TypeScript (`server/worker`, `node --test`), Vercel Sandbox for
measured runs.

**Spec:** [docs/plans/2026-10-28-one-capture-system-spec.md](2026-10-28-one-capture-system-spec.md)

## Global Constraints

- Goal Cup Proxy never sets a model base URL, never reads or forwards a key, and builds the harness
  environment from nothing (D1, D36, D61). The reader reads one URL value; a key file stays refused
  by the extension rule (`.yaml`, `.yml`, `.json`, `.toml` only).
- Schema 1 only: every connector field is optional and its absence reproduces today's behaviour
  (CONTRACTS "Schema 1 stays for a new optional field…a widened bound").
- The v5 capture chain, `seal.json` and `baseline.json` are frozen (CONTRACTS v5 freeze). The tunnel
  list is a side artifact with its own tag `goalcup-tunnels-v1`, outside the chain.
- `providers.toml` is frozen: no row added or removed (CONNECTOR-SCHEMA promise 6).
- No harness names in core code (`no_family_literals.rs`).
- Every edit under `connectors/` reruns `sh scripts/pin-manifests.sh` in the same commit; harness
  goldens regenerate with `GOALCUP_REGEN_HARNESS_GOLDEN=1`.
- One focused test per new or changed behaviour; never delete a P1 to P7 test; no golden snapshots
  of internals. Measured sandbox runs are the real proof.
- `sh scripts/gate.sh fast` passes once per PR before it is called merge-ready, run on the sandbox
  gate machine as user `ubuntu` (not root, IPv6 present).
- Docs: plain English, no em dashes, never write the next version number or a `vX.Y.Z` tag
  (`scripts/check-version.py`). Commits: Conventional Commits with the session's trailers.
- Never commit traces, keys, `spikes/*/out/`, `harnesses/` or `target/`. Keys reach a sandbox only as
  a copied 0600 file, never in argv or output.
- Sandbox test model: `deepseek/deepseek-v4.1-flash` through `https://ai-gateway.vercel.sh`. A 429 is a
  rate limit: wait and retry.
- Do not touch `connectors/opencode*` or `connectors/openclaw*` (another session owns them).

## Review Focus

1. A Claude Code participant with a plain subscription login and no `env.ANTHROPIC_BASE_URL`: start
   must not be refused, and `api.anthropic.com` is recorded as today (Task 3, Task 4 tests).
2. Codex with `model_provider = "openai"` (built-in, no `model_providers.openai` table) or no
   `model_provider` at all: absent falls back to `chatgpt.com` (Task 3 test).
3. A present but unrecordable value (company LiteLLM on `http://localhost:4000`, a sign-in host) stays
   refused with the fixed D62 sentence even under `absent = "defaults"`; absent covers only missing
   (Task 3 test).
4. An agent that opens thousands of tunnels (npm install loops): the list stays bounded at 10,000 rows
   with a dropped count, and finish does not stall (Task 6 test).
5. A seal sent to an old server that ignores `tunnels_jsonl`, and a retried seal: both stay 200, and
   the conflict rule still depends only on seal and baseline (Task 7 test).

---

### Task 0: Sandbox gate machine and two experiments (no code)

**Files:** none committed. Notes go to the scratchpad and later into Task 8's results update.

- [x] **Step 1: Create the gate machine.** `sandbox create --name gc-gate --vcpus 8 --timeout 3h`
  (Pro). Confirm 8 vCPUs in `sandbox ls`; if refused, Pro is not active yet: use 4 vCPUs, 40 min.
- [x] **Step 2: Provision as `ubuntu`.** rustup with toolchain 1.99.0 (clippy, rustfmt),
  `cargo install cargo-nextest --locked`, `sudo sh -c 'echo "127.0.0.1 api.e00.localhost auth.e00.localhost" >> /etc/hosts'`,
  check `nc`, `ps`, IPv6 on `lo`, `TMPDIR` on XFS.
- [x] **Step 3: Baseline gate on `origin/main`.** Clone, `cargo fetch --locked`, `sh scripts/gate.sh fast`.
  Expected: last line `gate fast: PASS in <N>s`. A FAIL here is an environment problem: fix before any PR.
- [x] **Step 4: Snapshot.** Delete any key file, then `sandbox snapshot gc-gate`. Record the snapshot id.
  Every later gate run forks from it (`sandbox fork <snapshot>`), pulls the PR branch, runs the gate.
- [x] **Step 5: Experiment E1, Codex key from `.env`.** In a fork, install the native Codex. Write
  `$CODEX_HOME/config.toml` with `model_provider = "vercel"`, `model = "deepseek/deepseek-v4.1-flash"`,
  `[model_providers.vercel] base_url = "https://ai-gateway.vercel.sh/codex/v1"`, `env_key = "AI_GATEWAY_API_KEY"`,
  `wire_api = "responses"`, and the key in `$CODEX_HOME/.env`. Run `env -i HOME=$HOME PATH=$PATH CODEX_HOME=$CODEX_HOME codex exec "say ok"`.
  Expected: a model reply. Record whether it used a WebSocket (`RUST_LOG=codex_core=debug`). If no
  reply, stop and report: Task 4's Codex row and Task 9's Codex run depend on it. Delete the key file
  after the experiment; never snapshot a sandbox that holds one.

### Task 1: The chat file is optional (zero native rows)

**Files:**
- Modify: `crates/goalcup-protocol/src/harness/check.rs:995-1013` (`check_native`)
- Modify: `crates/goalcup-protocol/src/harness/schema.rs:488-490` (doc comment)
- Modify: `crates/goalcup-daemon/src/runtime/capture/native_plan.rs:16` (`native_plan`)
- Test: `crates/goalcup-protocol/tests/it/harness_schema.rs:323`, `crates/goalcup-daemon/src/runtime/capture/native_plan.rs` (unit)

**Interfaces:**
- Produces: a connector with `capture.native = []` loads; `native_plan(...)` returns `Ok(vec![])` for it.

- [x] **Step 1: Flip the refusal test.** Rename `no_native_source_is_refused` to
  `a_harness_without_a_native_source_is_admitted`: remove the `[[capture.native]]` rows from `codex()`
  and call `admitted(&t, PINNED)`. Keep `two_native_sources_are_refused` unchanged.
- [x] **Step 2: Add the daemon unit test** `a_capture_without_a_native_source_plans_nothing`:
  build the capture from the codex fixture with `native` cleared, call
  `native_plan(&capture, run_home, &config_home, since)`, assert `Ok(v)` with `v.is_empty()`.
- [x] **Step 3: Run both.** `cargo nextest run --offline -p goalcup-protocol -E 'test(native_source)'`
  and `-p goalcup-daemon -E 'test(native_source)'`. Expected: both FAIL.
- [x] **Step 4: Implement.** In `check_native`, `[] => return Ok(())`; the per-row checks run only for
  `[one]`. In `native_plan`, `let Some(native) = capture.native.first() else { return Ok(Vec::new()) };`.
  Doc comment at schema.rs:488: "Zero or one entry in schema 1 (an array so that 1..=N is a later
  additive widening)."
- [x] **Step 5: Run both again.** Expected: PASS. Then `cargo nextest run --offline -p goalcup-cli -E 'test(resume) | test(native)'`. Expected: PASS (`has_native_session` already returns `Ok(true)` for no rows, `launch.rs:663`).
- [x] **Step 6: Commit.** `feat(schema): a connector may declare no native session file`

### Task 2: `model_host_from` gains `via` and `absent`; `env` is no longer a deny word

**Files:**
- Modify: `crates/goalcup-protocol/src/harness/schema.rs:783-816`
- Modify: `crates/goalcup-protocol/src/harness/check.rs:228-284` (`config_value_problem`), `:1133-1168` (`check_model_host_from`, `check_hosts`)
- Test: `crates/goalcup-protocol/tests/it/harness_schema.rs` (next to `a_model_host_from_is_held_to_its_rules`, :3955)

**Interfaces:**
- Produces:
  ```rust
  pub struct ModelHostFrom {
      pub file: String,
      pub key: String,
      #[serde(default)] pub via: Option<String>,
      #[serde(default)] pub absent: ModelHostAbsent,
  }
  #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
  #[serde(rename_all = "lowercase")]
  pub enum ModelHostAbsent { #[default] Refuse, Defaults }
  pub const MODEL_HOST_VIA_PART: &str = "*";
  ```
  `MODEL_HOST_DENIED_WORDS` without `"env"`.

- [x] **Step 1: Write the test** `a_model_host_from_names_one_indirection_and_a_fallback`, using the
  `with(file, key)` helper pattern from :3955 plus a `via`/`absent` setter:
  - admitted: `("settings.json", "env.ANTHROPIC_BASE_URL")` with `absent = "defaults"` and the codex host row kept;
  - admitted: `("config.toml", "model_providers.*.base_url")` with `via = "model_provider"`;
  - refused `["capture.model_host_from.key", "*"]`: `"model_providers.*.base_url"` without `via`;
  - refused `["capture.model_host_from.key", "one"]`: `"a.*.*.b"` with `via`;
  - refused `["capture.model_host_from.via"]`: `via = "x.*"` and `via = "auth.provider"` (deny word);
  - refused `["capture.hosts", "absent"]`: `absent = "defaults"` with `capture.hosts = []` and no providers;
  - refused `["capture.model_host_from.absent"]`: `absent = "sometimes"` (closed set, serde error).
- [x] **Step 2: Run it.** `cargo nextest run --offline -p goalcup-protocol -E 'test(indirection)'`. Expected: FAIL (unknown field `via`).
- [x] **Step 3: Implement the fields and checks.** Remove `"env"` from `MODEL_HOST_DENIED_WORDS`.
  In `config_value_problem`, accept a key part equal to `MODEL_HOST_VIA_PART` only when the caller
  passes `allow_via_part = true`, and require exactly one such part then. `check_model_host_from`
  validates `via` with `config_value_problem(field "capture.model_host_from.via", &from.file, via, …, allow_via_part = false)`.
  `check_hosts`: `least = 1` when `model_host_from` is `None` or `absent == Defaults`; message
  "must hold 1 to 16 hosts, or none with `capture.model_host_from` and `absent = \"refuse\"`".
- [x] **Step 4: Run it, then the whole crate.** `cargo nextest run --offline -p goalcup-protocol`. Expected: PASS, including the old deny-word cases (`auth.json`, `api-keys.yaml`, `model.api_key`, `providers.x.token`).
- [x] **Step 5: Commit.** `feat(schema): model_host_from may follow one indirection and fall back to the connector's hosts`

### Task 3: The reader follows `via` and falls back

**Files:**
- Modify: `crates/goalcup-cli/src/model_host.rs:128-195` (`for_run`, `check`, `read`)
- Test: `crates/goalcup-cli/src/model_host.rs` unit tests (:466+), `crates/goalcup-cli/tests/it/model_host_from.rs`

**Interfaces:**
- Consumes: `ModelHostFrom { file, key, via, absent }`, `ModelHostAbsent`, `MODEL_HOST_VIA_PART` (Task 2).
- Produces: `pub fn read(home: &Path, from: &ModelHostFrom, auth_hosts: &[String]) -> Result<Option<ModelHost>, Unrecordable>`;
  `check` and `for_run` keep their signatures (`Result<Option<ModelHost>, _>`) and flatten it.
  `ModelHost.from` reads "`<resolved key>` in <file>" (e.g. "`model_providers.vercel.base_url` in config.toml").

- [x] **Step 1: Write unit tests** in model_host.rs. `ModelHostFrom` is `#[non_exhaustive]` in another
  crate: build each one with `toml::from_str::<ModelHostFrom>(r#"file = "…" key = "…" …"#)`.
  - `an_indirection_reads_the_named_table`: TOML `model_provider = "vercel"` and
    `[model_providers.vercel] base_url = "https://ai-gateway.vercel.sh/codex/v1"` →
    `Ok(Some(m))`, `m.host == "ai-gateway.vercel.sh"`, `m.base_path == "/codex/v1"`, `m.from` contains `model_providers.vercel.base_url`.
  - `a_missing_value_falls_back_only_when_asked`: empty JSON `{}` with `absent = Defaults` → `Ok(None)`;
    the same with `Refuse` → `Err(u)` with `u.reason` containing "is not set".
  - `a_builtin_provider_without_a_table_falls_back`: `model_provider = "openai"`, no table, `Defaults` → `Ok(None)`.
  - `a_present_unrecordable_value_is_refused_even_with_defaults`: `env.ANTHROPIC_BASE_URL = "http://localhost:4000"` with `Defaults` → `Err`, reason "it is plain http, which goes around the recorder".
  - `a_via_value_that_is_not_a_name_is_refused`: `model_provider = "../x"` → `Err`, reason contains "is not a provider name".
- [x] **Step 2: Run them.** `cargo nextest run --offline -p goalcup-cli -E 'test(indirection) | test(falls_back) | test(even_with_defaults) | test(not_a_name)'`. Expected: FAIL.
- [x] **Step 3: Implement in `read`.** If `via` is set, read it with `read_value`; a missing value →
  missing; a value not matching `[A-Za-z0-9_-]{1,64}` → `Err(Unrecordable { shown: "an address Goal Cup cannot read", reason: "`{via}` in {file} names `{v}`, which is not a provider name" })`.
  Replace the `*` part of `key` with it. A missing key: `absent == Defaults` → `Ok(None)`, else the
  existing "is not set" refusal. A present value goes through the existing refusal list unchanged.
  Update every caller of `read` (`check`, and tests under `tests/it/`) to the `Option` return.
- [x] **Step 4: Integration test** `a_missing_setting_records_the_connector_hosts` in
  `tests/it/model_host_from.rs`: like `the_model_host_from_the_harness_config_is_recorded` (:126) but
  keep the fake connector's host row, set `absent = "defaults"`, write no config; run `headless_run`;
  assert the connector host is opened (`wire_opens`) and the start succeeded.
- [x] **Step 5: Run.** `cargo nextest run --offline -p goalcup-cli -E 'binary(it) & test(model_host)'`. Expected: PASS, old tests included.
- [x] **Step 6: Commit.** `feat(cli): read the model host through one indirection, else the connector's hosts`

### Task 4: Claude Code, Codex and Hermes connectors

**Files:**
- Modify: `connectors/claude-code-cli/harness.toml:34-36`, `connectors/codex-cli/harness.toml:32` and `:114-117`, `connectors/hermes-cli/harness.toml` (`[capture]`)
- Modify: `crates/goalcup-protocol/pins/connector-dirs.txt` (by script), `crates/goalcup-cli/tests/fixtures/harness-golden/*.json` (by regen)
- Modify: `skills/adding-a-capture-harness/templates/connector-harness.toml:70-76,188-191`
- Test: `crates/goalcup-cli/tests/it/model_host_from.rs`

**Interfaces:**
- Consumes: Task 2 fields, Task 3 reader.

- [x] **Step 1: Write the test** `the_shipped_claude_and_codex_rows_fall_back_without_a_setting`: load
  the shipped `claude-code-cli` and `codex-cli` connectors with the same `load_dir` call that
  `reads_model_host` uses (`tests/it/model_host_from.rs:25`), pointed at the checkout's `connectors/`
  directory, and a `Rig` for the config dir. Write no `settings.json` /
  `config.toml`, call `goalcup_cli::model_host::check(&rig.cfg, &connector)`; assert `Ok(None)` for both.
  Then write `settings.json` `{"env":{"ANTHROPIC_BASE_URL":"https://ai-gateway.vercel.sh/claude-code"}}`
  and assert `Ok(Some(m))` with `m.host == "ai-gateway.vercel.sh"`, `m.base_path == "/claude-code"`.
- [x] **Step 2: Run it.** Expected: FAIL (connectors have no `model_host_from`).
- [x] **Step 3: Edit the connectors.**
  - claude-code-cli: `model_host_from = { file = "settings.json", key = "env.ANTHROPIC_BASE_URL", absent = "defaults" }`; keep `providers = ["anthropic"]`; comment: the gateway the participant's own `settings.json` names (D74).
  - codex-cli: `model_host_from = { file = "config.toml", key = "model_providers.*.base_url", via = "model_provider", absent = "defaults" }`; keep the `chatgpt.com` row.
  - hermes-cli: `auth_hosts = ["portal.nousresearch.com", "auth.openai.com", "auth.x.ai"]` (refresh hosts, `docs/harnesses/hermes-cli.md` section 9). `model_host_from` unchanged (`absent` stays `refuse`).
  - template: document `via` and `absent`; drop `env` from the deny-word list.
- [x] **Step 4: Re-pin and regenerate.** `sh scripts/pin-manifests.sh`; `GOALCUP_REGEN_HARNESS_GOLDEN=1 cargo nextest run --offline -p goalcup-cli -p goalcup-daemon -E 'test(golden)'`. Inspect the golden diff: only the three connectors' host and model-host lines change.
- [x] **Step 5: Run.** `cargo nextest run --offline -p goalcup-cli -p goalcup-protocol -E 'test(model_host) | test(pin) | test(connector_contract)'`. Expected: PASS.
- [x] **Step 6: Commit.** `feat(connectors): Claude Code and Codex record the gateway their settings name`

### Task 5: Evidence label and wire request range in the research summary

**Files:**
- Modify: `crates/goalcup-verifier/src/capture_analysis/report.rs:34-40,280-360`
- Modify: `crates/goalcup-verifier/src/capture_analysis/session.rs:172-210,536-568` (`ObservationCounts`, `tally`, `observe`)
- Modify: `crates/goalcup-verifier/tests/fixtures/m2-report-bytes/*.ndjson` (by `REGENERATE_REPORT_BYTES=1`)
- Modify: `docs/research/m2-report-format.md:117,135`
- Test: `report.rs` unit (:424), `crates/goalcup-verifier/tests/it/m2_research_report.rs`

**Interfaces:**
- Produces: `fn evidence(native: u64, wire_streams: u64, wire_requests: u64) -> &'static str` returning
  in this order: `wire_requests > 0` → `"wire_and_native"` if `native > 0` else `"wire_only"`;
  `wire_streams > 0` → `"wire_undecoded"`; `native > 0` → `"native_only"`; else `"none"`.
  `ObservationCounts` gains `wire_first: Option<String>`, `wire_last: Option<String>` (min and max of
  `observation.observed.first.wall` / `.last.wall` over wire model requests).
  Summary keys: `analysis.evidence` (string) and `analysis.cross_leg.wire_request_range` (`{"first","last"}` or `null`).
  `RESEARCH_SCHEMA_VERSION = 5`.

- [x] **Step 1: Unit test** `the_evidence_label_follows_decoded_requests_first`: assert the five
  outputs for `(1,1,1)`, `(0,1,1)`, `(0,1,0)`, `(1,0,0)`, `(0,0,0)` and `(1,1,0) == "wire_undecoded"`.
- [x] **Step 2: Integration test** `each_run_carries_an_evidence_label`: `codex_records()` (native, no
  wire) → `"native_only"`; `openai_records()` → `"wire_only"` and `wire_request_range.first <= last`;
  `mixed_records()` → `"wire_and_native"`. Update `mixed_observations_end_in_one_bound_summary` to `schema_version == 5`.
- [x] **Step 3: Run.** `cargo nextest run --offline -p goalcup-verifier -E 'test(evidence)'`. Expected: FAIL.
- [x] **Step 4: Implement.** `evidence()` next to `cross_leg_state`; copy the wall times in `observe`
  before the sink takes the observation; add both keys; bump the version and its doc comment at :34-39.
  No key may contain `call`, `model`, `total`, `duration` (guards at :1060, :1587, :1668).
- [x] **Step 5: Regenerate the byte fixtures.** `REGENERATE_REPORT_BYTES=1 cargo nextest run --offline -p goalcup-verifier -E 'test(bytes)'`; inspect: only `schema_version`, `evidence` and `wire_request_range` change.
- [x] **Step 6: Run the crate.** `cargo nextest run --offline -p goalcup-verifier`. Expected: PASS.
- [x] **Step 7: Docs and commit.** m2-report-format.md: "Schema 5 added `analysis.evidence` and `analysis.cross_leg.wire_request_range`." Commit `feat(verifier): label each run's evidence`.

### Task 6: Keep each tunnel and write the list at finish

**Files:**
- Modify: `crates/goalcup-cli/src/proxy/connect.rs:303-338,424-458,532,562,646-662`
- Modify: `crates/goalcup-cli/src/proxy/mod.rs:596-640,718-790,856-880,1036-1044`
- Create: `crates/goalcup-cli/src/proxy/tunnels.rs`
- Modify: `crates/goalcup-cli/src/commands/start/finish.rs:412-430`
- Test: `crates/goalcup-cli/src/proxy/tunnels.rs` (unit), `crates/goalcup-cli/tests/it/model_host_from.rs` (`other_hosts_stay_opaque_tunnels`, :161)

**Interfaces:**
- Produces:
  ```rust
  // connect.rs
  pub struct Tunneled { pub host: String, pub port: u16, pub reason: TunnelReason, pub end: TunnelEnd,
                        pub opened: SystemTime, pub closed: SystemTime }
  // tunnels.rs
  pub const TUNNELS_FORMAT: &str = "goalcup-tunnels-v1";
  pub const MAX_TUNNEL_NOTES: usize = 10_000;
  pub struct TunnelNote { pub host: String, pub port: u16, pub reason: &'static str,
                          pub opened: SystemTime, pub closed: SystemTime,
                          pub up_bytes: Option<u64>, pub down_bytes: Option<u64> }
  pub struct TunnelNotes { /* Mutex<Vec<TunnelNote>>, dropped: AtomicU64 */ }
  impl TunnelNotes { pub fn push(&self, note: TunnelNote); pub fn snapshot(&self) -> (Vec<TunnelNote>, u64); }
  pub fn file_name(entry_id: &str) -> String;            // "tunnels-<entry_id>.jsonl"
  pub fn write(path: &Path, entry_id: &str, notes: &[TunnelNote], dropped: u64) -> std::io::Result<()>;
  pub fn summary_line(notes: &[TunnelNote], dropped: u64) -> Option<String>;
  ```
  `ProxyHealth` gains `tunnels: Vec<TunnelNote>` and `tunnels_dropped: u64`.
  Reason strings: `AuthHost` → `"sign_in"`, `NotApproved` → `"not_model_host"`, `SniAbsent` → `"fail_open_no_sni"`,
  `SniMismatch` → `"fail_open_sni_mismatch"`, `NotTls` → `"fail_open_not_tls"`, `ClientHelloUnparsed` → `"fail_open_unparsed"`.
  File lines: first `{"format":"goalcup-tunnels-v1","entry_id":…}`, then one per note
  `{"host","port","reason","opened","closed","up_bytes","down_bytes"}` (RFC 3339 UTC times, bytes `null`
  when the tunnel ended in an error), last `{"dropped":N}`. Mode 0600, `O_NOFOLLOW`, written once.
  Summary line: `sealed hosts: <host>:<port> x<n> <KiB> KiB, …` for the five hosts with most bytes,
  then ` (+<m> more, list in tunnels-<entry_id>.jsonl)` when more exist; `None` when there are none.

- [x] **Step 1: Unit tests** in tunnels.rs:
  - `notes_stop_at_the_bound_and_count_the_rest`: push `MAX_TUNNEL_NOTES + 5` → snapshot has `MAX_TUNNEL_NOTES` rows and `dropped == 5`.
  - `the_file_is_tagged_one_line_per_tunnel_and_private`: write three notes; first line `format == TUNNELS_FORMAT`, three note lines, last `{"dropped":0}`, file mode `0o600`.
  - `the_summary_names_the_biggest_hosts`: notes for `github.com:443` (2 MiB) and `registry.npmjs.org:443` (300 KiB) → line starts `sealed hosts: github.com:443 x1 2048 KiB, registry.npmjs.org:443 x1 300 KiB`.
- [x] **Step 2: Extend the integration test** `other_hosts_stay_opaque_tunnels` (:161): after the run,
  read `run_dir(cfg, run_id)/tunnels-<entry>.jsonl` and assert it names the tunnelled test host with reason `not_model_host` and `up_bytes > 0`; assert the finish output has a `sealed hosts:` line.
- [x] **Step 3: Run.** `cargo nextest run --offline -p goalcup-cli -E 'test(tunnel) | test(opaque_tunnels)'`. Expected: FAIL.
- [x] **Step 4: Implement.** In `connect.rs` carry `target.port` into `Decision::Tunnel` and set
  `opened`/`closed` around `run_tunnel`. In `mod.rs` add `tunnels: Arc<TunnelNotes>` to `Shared`, push
  in the `Admission::Tunneled(tunnel)` arm (:1036) next to the existing counter bump, fill
  `ProxyHealth` in `Shared::health()`. In `finish.rs` after `proxy_admission_line` (:415): print
  `summary_line`, then `tunnels::write(&run_dir(cfg, run_id).join(file_name(entry_id)), …)`; a write
  error is logged with `host_log::log_only` and never stops the seal.
- [x] **Step 5: Run.** Same command, then `cargo nextest run --offline -p goalcup-cli`. Expected: PASS.
- [x] **Step 6: Commit.** `feat(proxy): list every sealed connection with host, port, times and bytes`

### Task 7: Upload the tunnel list with the seal

**Files:**
- Modify: `crates/goalcup-cli/src/upload/mod.rs:47-60` (`UploadRecord`), where the record is created at start
- Modify: `crates/goalcup-cli/src/upload/api.rs:263-269` (`SealRequest`), `crates/goalcup-cli/src/upload/uploader/finish.rs:208-233`
- Modify: `server/worker/src/runs.ts:27,515-578,580-592`
- Modify: `docs/UPLOAD-API.md:322-336`, `docs/CONTRACTS.md` (Side artifacts)
- Test: `crates/goalcup-cli/src/upload/` unit tests, `server/worker/test/integration.test.ts` ("seal" block, :426)

**Interfaces:**
- Consumes: `tunnels::file_name(entry_id)` (Task 6).
- Produces: `UploadRecord.tunnels_file: Option<String>` (`#[serde(default)]`, absolute path);
  `SealRequest.tunnels_jsonl: Option<String>` (`#[serde(default, skip_serializing_if = "Option::is_none")]`);
  server object key `traces/<event>/<run>/<entry>/tunnels-<sha256>.jsonl`.

- [x] **Step 1: Client unit test** `the_seal_carries_the_tunnel_list_when_it_exists`: an `UploadRecord`
  with `tunnels_file` pointing at a written file → the built `SealRequest` has `tunnels_jsonl == Some(contents)`;
  with no file → `None`, and the serialized JSON has no `tunnels_jsonl` key. An old record JSON without
  the field still deserializes.
- [x] **Step 2: Server test** in the "seal" block: `test("a seal may carry the tunnel list; it never changes the conflict rule", …)`:
  seal with `{...docs, tunnels_jsonl: '{"format":"goalcup-tunnels-v1"}\n'}` → 200 and the R2 object
  `.../tunnels-<sha>.jsonl` exists in the test env's TRACES fake; a repeat without `tunnels_jsonl` → 200;
  `tunnels_jsonl: 5` → 400 `malformed`.
- [x] **Step 3: Run.** `cargo nextest run --offline -p goalcup-cli -E 'test(tunnel_list)'`; `cd server/worker && npm test`. Expected: FAIL.
- [x] **Step 4: Implement the client.** Set `tunnels_file` where the record is created
  (`run_dir(cfg, run_id).join(tunnels::file_name(entry_id))`); in `seal()` read it with `read_document`
  (16 MiB bound) when present.
- [x] **Step 5: Implement the server.** In `sealRun`, if `b.tunnels_jsonl !== undefined`, `takeDocument(b, "tunnels_jsonl")`;
  after the seal and baseline puts, `putDocument(ctx, \`${prefix}/tunnels-${sha}.jsonl\`, …, "application/x-ndjson")`
  (add the content-type parameter, default `"application/json"`). No DB change. The repeat/conflict
  check stays on seal and baseline only. `SEAL_BODY_MAX = 3 * SEAL_DOC_MAX + JSON_BODY_MAX`.
- [x] **Step 6: Run.** Same commands. Expected: PASS.
- [x] **Step 7: Docs and commit.** UPLOAD-API seal body gains the optional field; CONTRACTS lists
  `tunnels-<entry>.jsonl` (`goalcup-tunnels-v1`) under side artifacts: not hash-bound, never read by
  the v5 verifier. Commit `feat(upload): send the sealed-connection list with the seal`.

### Task 8: Rules, decision and docs

**Files:**
- Modify: `docs/DECISIONS.md` (append D74 after :1101; status rows :40)
- Modify: `README.md:34-40,112-135`, `SECURITY.md:29-45`, `docs/RULES.md:216-224,281-299,320-335`
- Modify: `docs/CONNECTOR-SCHEMA.md:22-23,193,262-276,615,678-748`
- Modify: `docs/HARNESS-SETUP.md:73-106`, `docs/CHEAT-CHECKLIST.md:44-48,92-97`
- Modify: `crates/goalcup-protocol/providers.toml` (header only), `docs/research/VERCEL-COMPAT.md` (banner + §2 table), `docs/research/results/2026-10-vercel-gateway.md` (decisions section), `docs/README.md` (active plans)

- [x] **Step 1: D74.** Title "The model host is the connector's default, or the one address the harness's own config names." Body bullets: defaults first; `via` and `absent`; `env` leaves the deny words; zero native rows; tunnels listed and uploaded as `goalcup-tunnels-v1`; evidence labels; `providers.toml` frozen. Supersedes D62's missing-value refusal, its `env` deny word and its one-key form; D1, D36, D61 stand. Status rows: `| D62 | partly superseded | D74 (…) |`, `| D63 | current | |`, `| D74 | current | |`.
- [x] **Step 2: README and SECURITY.** README "How it works" gains "### How Goal Cup Proxy records" with the four rules from the spec, verbatim. README :36 and SECURITY :40-45 replace "Goal Cup never sees it" with the token sentence from the spec and a link to `crates/goalcup-cli/src/proxy/http.rs`.
- [x] **Step 3: RULES.** :287 key sentence → the token sentence. :299 "Every token your agent saw is captured" → "Every request your agent sent to the model service Goal Cup opened is captured; a call that bypasses `HTTPS_PROXY` is not." :320-335 keep "address and bytes" and add port, times and reason, uploaded with the seal. Add "What a label is worth": `wire_and_native`, `wire_only`, `wire_undecoded`, `native_only` count; `none` does not. :220 remove `--local` from OpenClaw's banned list. No other advice added.
- [x] **Step 4: CONNECTOR-SCHEMA, HARNESS-SETUP, CHEAT-CHECKLIST, template text.** Native "zero or one"; `via`/`absent` rows and rules; Claude and Codex gateway recipes (Claude `settings.json` `env.ANTHROPIC_BASE_URL` plus `ANTHROPIC_AUTH_TOKEN`; Codex `config.toml` provider plus the key in `$CODEX_HOME/.env`); CHEAT-CHECKLIST: settings also come from `settings.local.json`, workspace `.claude/settings.json` and managed settings, so a gateway named there tunnels and shows in the sealed-hosts list.
- [x] **Step 5: Freeze and banners.** providers.toml header: "Frozen (D74): no new rows; a connector lists its default hosts in its own `[[capture.hosts]]`." VERCEL-COMPAT: "Superseded by D74 for Claude Code and Codex" banner and corrected §2 rows. Results file: the decisions and this plan's link. docs/README.md lists both plan files as active.
- [x] **Step 6: Check and commit.** `python3 scripts/check-links.py` (0 broken) and `python3 scripts/check-version.py` (pass). Commit `docs: one capture rule set (D74)`.

### Task 9: Measured runs, reviews, merge

- [x] **Step 1: Per PR, fresh reviewers.** Two reviewer subagents per PR, given the diff and the spec
  only: one for correctness against the spec and Review Focus, one holistic (simplicity, docs truth,
  no over-testing). Fix blocking findings; reply to each.
- [x] **Step 2: Gate per PR.** Fork the Task 0 snapshot, check out the PR branch,
  `sh scripts/gate.sh fast`. Expected: `gate fast: PASS`. For Task 7 also `cd server/worker && npm ci && npm test`.
- [x] **Step 3: Build the integrated branch in a sandbox** (`cargo build --release --offline`, `GOALCUP_CONNECTORS_DIR` set) and run local runs with a fresh `GOALCUP_CONFIG_DIR`, prompt "Build a chess game in Python.", about three minutes each:
  - Claude Code, `settings.json` env `ANTHROPIC_BASE_URL=https://ai-gateway.vercel.sh/claude-code`, model pinned to `deepseek/deepseek-v4.1-flash` for every alias. Pass: `integrity valid`, `analysis.evidence == "wire_and_native"`, `cross_leg.wire_model_requests > 0`, `ai-gateway.vercel.sh` absent from the sealed-hosts line.
  - Codex, provider `vercel` with the key in `$CODEX_HOME/.env`. Pass: the same, evidence `wire_and_native`.
  - Hermes, built-in `ai-gateway` provider. Pass: evidence `wire_only`, unchanged from 28 Oct.
  - Each run's `tunnels-<entry>.jsonl` lists hosts with bytes; none is the model host.
- [x] **Step 4: Merge** in this order: Task 1 PR, Task 5 PR, Task 6+7 PR, Task 2-4 PR, Task 8 PR. Rebase each on the new `main`; rerun the gate only when the rebase changed Rust files. Re-pin with `sh scripts/pin-manifests.sh` on the merged tree if the OpenCode/OpenClaw PR landed first.
- [ ] **Step 5: Tell the OpenCode/OpenClaw session** the new connector shape (`via`, `absent`, zero native rows, `prefix_paths` for `/v3/ai/`), and that its connectors should start on it.
- [x] **Step 6: Update the results file** with the measured runs; delete sandboxes and their snapshots, except the gate snapshot until the release.

### Task 10: Owner steps (not for agents)

- [ ] Deploy the Worker (`server/worker`) with the `tunnels_jsonl` change before participants upload.
- [ ] Laptop runs: Claude Code with a subscription login and no base URL (evidence `wire_and_native`, `api.anthropic.com` recorded); Codex with a ChatGPT login (`chatgpt.com` recorded); Codex with an OpenAI API key (is `api.openai.com` tunnelled? decides a future row).
- [ ] Release: 0.7.1 is merged but not tagged. Decide whether this work ships as the next patch after tagging 0.7.1, or folds into it, then `scripts/release.sh prepare` and `tag`.
