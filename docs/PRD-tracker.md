# PRD — /goal Cup tracker (daemon, connectors, verifier)

> **V1 requirements (2026-10-02).** This PRD describes V1 on `v1/trunk`; on `m2/wire` the hook leg is
> gone and the wire leg replaces it. Current: [public M2 architecture](ARCHITECTURE.md#4-the-two-legs), [DECISIONS](DECISIONS.md).

## V1 foundation revision (2026-09-13)

Public V1 scope and current status are recorded in [STATUS](../STATUS.md#v1-on-v1trunk). The
supporting private historical record is intentionally omitted from this public OSS cut.

V1 defines two fresh short-duration Codex CLI launch-mode qualification journeys on one exact macOS target: a manual-finish recovery journey and a separate automatic signed-short-deadline journey. Once implemented, together they require start, interruption, crash/restart, recovery, resume, process termination, immutable workspace artifact capture, and offline verification. A real 168-hour run is explicitly not required for V1.

V1.1 owns additional CLI and Desktop surfaces, Linux and Windows, continuous Git provenance, Git-aware selection/history, participant-side automatic publication, production organizer services and keys, self-service packages/signing, presentation operations, broad qualification, and the later 168-hour exercise. M2 independently owns wire and PTY capture.

### What does not hold on `m2/wire`

This PRD describes V1 on `v1/trunk` and `v0.1.0`. On `m2/wire` these parts do not hold
as written: leg 2 as a hook or plugin stream (§3 goal 1, R1's hook endpoint, R5's blocking hook,
R7, R9's hook config, R14's `hooks_silent`, V9's hook-config events); the two-legged human
rule and leg reconciliation (§3 goal 5, V2, V3's excluded-twin join); the manifest rows `arming.recipe`, `hook_silence_policy`,
`hot_disable`, `receiver.token` and `leg2_partition.path`; and OpenCode as a shipped
connector (§7 M1). "M2 … does not replace the transcript/hook fallback" (§7) is
overtaken there: the transcript stays as the fallback, the hook leg does not. The M2 authority is
the included [public architecture](ARCHITECTURE.md#1-scope), with current first-release framing in
[D37](DECISIONS.md#d37-m2-autonomous-runs-research-evidence-and-open-contributions). The earlier
wire-primary specification is intentionally omitted from this public OSS cut.

Status: overall product requirements first written 2026-09-03 and corrected in place since. The revision above assigns each requirement to V1, V1.1, or M2; an older unlabelled broad requirement is not permission to expand V1. Constraints come from `DECISIONS.md`; harness facts from `CAPTURE-RESEARCH.md`, whose section 8 lists what in `harness-research/` the spikes overturned. `SYSTEM-PLAN.md` section 4 is the pre-M1 long-form design and this document supersedes it where the two disagree; sections 5 to 7 there (backend, website, compliance) are covered nowhere else.

## 1. Problem

The full event must let participants support a 168-hour autonomous run on their own machine with reviewable evidence that no human instruction entered the agent after the original prompt, that the workspace started empty, and that the published product is the sealed result. Once implemented and qualified, V1 proves the same lifecycle at short duration without making that duration or publication claim. Participants use subscription-authenticated harnesses: V1 does not intercept model traffic; M2 may later add only the passive process-scoped wire path defined by D28. Organizers must eventually verify hundreds of entries across many harnesses with a small team, and the community must be able to add harnesses before the start.

## 2. Users

- **Participant**: installs one binary, runs one command, opens their harness, types the goal, walks away. Wants clear status, a working "back online" after outages, and the ability to self-check compliance before the deadline.
- **Organizer / reviewer**: runs the verifier over every sealed entry, reads only what is flagged, decides.
- **Connector contributor**: adds a harness by writing a manifest, recording fixtures, and writing a parser that passes the public conformance suite.
- **The agent**: may query remaining time through a CLI command or the shipped MCP server. Is otherwise never addressed.

## 3. Goals

1. Capture leg 1 (harness's persisted transcript) and leg 2 (hook or plugin stream) for every supported harness, hash-chained, redacted with a log, encrypted locally, uploaded continuously, acknowledged by the server.
2. Bind an entry to exactly one fresh session in exactly one tracker-created folder; the tracker may read only declared shared/home-wide harness stores, solely to partition the bound session and discard foreign rows before persistence. Foreign ordinary sessions are not retained, chained, or uploaded.
3. Start the clock on the first typed message; seal by workspace manifest at the deadline.
4. Survive crashes and outages: supervised daemon, encrypted write-ahead buffer, catch-up from last acknowledgement, visible recovery.
5. Provide an open-source verifier that normalizes to ATIF, applies the per-harness human-message rule, reconciles the two legs, classifies gaps, attributes config deviations and sealed files, and runs identically on the server and on the participant's machine.
6. Make adding a harness a data-plus-parser task with a public conformance suite.

## 4. Non-goals (V1 foundation)

Additional harness promotion; Desktop; Windows/Linux; continuous Git provenance or workspace hashing; participant-side Git publication; production organizer services or keys; self-service signing/notarization; presentation operations; a real 168-hour qualification; tracker-injected reminders or auto-continue; cross-harness subagent linking; a large native GUI; Rekor anchoring; kernel monitoring or attestation; cloud or remote execution modes. V1.1 and M2 routing is defined above; listing an item here does not discard its existing code or evidence.

## 5. Functional requirements

### 5.1 Daemon (`goalcup-daemon`)

- R1. V1 runs as one supervised user service on the claimed macOS target and restarts automatically. It exposes a loopback control API for the CLI and a token-authenticated local hook endpoint; unauthenticated posts are dropped without persisting. V1.1 adds systemd, Windows service operation, native transports, install/upgrade/uninstall, and their platform matrices.
- R2. Holds an entry registry: workspace path, harness, manifest hash, session binding, signed receipts, per-run keys, health.
- R3. Creates the workspace: new directory outside any Git worktree, zero entries, then writes the connector's folder-scoped tracker config, records expected hashes, records the baseline manifest excluding those files. The tracker's artifact root (buffer, logs, baselines, receiver state) lives in a tracker-owned directory that is **not** an ancestor of the workspace and not adjacent to `$HOME`, because agents enumerate the parent directory on their first tool call. Config baselines hash credential files; they never copy them.
- R4. Launch mode: spawns the harness per manifest recipe with process-scoped pins (env, flags, dedicated home or profile), following `kill.strategy` for later termination. Attach mode: arms the folder and waits for a new session whose working directory equals the workspace and whose start source is a fresh start. Both modes then wait for the connector's `arming.signal`; the daemon holds `armed` until it arrives, surfaces `arming.recipe` on screen where a human step exists (Codex app `/hooks` trust), and fails the start on `arming.timeout`.
- R5. Binds the entry on the first submitted prompt observed through leg 2 or leg 1, validates length through a blocking hook where the harness supports it, then requests the signed start receipt. The receipt is requested only in state `start_signal_seen`; where `start_signal.required` is false (Claude Code, Claude Desktop) the launch the daemon performed plus the first transcript record is the signal. Failure before dispatch leaves no entry; success after dispatch is immutable.
- R6. Tailers: append-only JSONL by path and inode, directory-of-files, SQLite with read-only WAL-safe access. Each store is read through its declared `cursor.type` (`ordinal`, `rowid`, `sqlite_pk`, `session_seq`, `inode_offset`) and never through a cursor named in `unsafe_cursors[]`; rowid on OpenClaw and the global `seq` on OpenCode are unsafe. Each store applies its `partition_key` before hashing or shipping. Watch subagent, archive and spill locations named in the manifest.
- R7. Routes hook posts to entries by working directory and per-run token; discards posts for unbound directories without persisting them.
- R8. Partitions every home-wide or agent-wide store at capture time and discards foreign rows before they enter the chain (Codex first `session_meta.cwd`; OpenCode `session.directory`; Hermes `sessions.cwd`; OpenClaw `ctx.sessionKey`; Claude Desktop raw bodies `metadata.user_id.session_id`; Cursor `meta.json` `cwd`). Watches all installed harness stores during a run only to detect a foreign harness session inside a bound workspace; emits a flag event; persists nothing from unbound sessions.
- R9. Config watch: any deviation of a tracker-written file from its expected bytes produces an event with timestamp and diff, then the file is restored. The watch covers the folder-scoped hook config, the hook-config hash the connector pins into its shim (so a hot-reload deletion surfaces on the next post as `config_mismatch`), and the `config_allowlist[]` of app-wide files whose deviation is meaningful. Restoring a Codex hook file invalidates its trust hash, so a restore there raises `arming_lost` rather than assuming capture continues.
- R10. Redaction: public pattern set applied before persistence; every altered span recorded in a redaction log shipped with the chain, so hashes are computed over redacted bytes consistently.
- R11. Integrity: per-line hash chain, monotonic sequence per entry, per-run Ed25519 signature on checkpoints, server acknowledgements stored locally, gap detection.
- R12. Buffer: encrypted write-ahead store (`redb` + `age`, per-run key). Acknowledged chunks deleted per documented policy.
- R13. Upload: continuous, idempotent, resumes from last acknowledgement after any outage. Thresholds generous. Recovery is a first-class state: "back online, N events uploaded."
- R14. Health states: online, upload_lagging, connection_lost, agent_exited, tracker_interrupted, capture_stalled, hooks_silent, config_restored, deadline_reached, plus `arming_pending`, `arming_lost`, `version_changed`, `config_mismatch`, and (added 2026-09-04) `store_replaced` — a tailer saw a truncation or an inode change, which is the on-disk signature of a rewritten transcript — and `disk_pressure`, so the buffer degrades visibly instead of failing on a full disk, and (added by dogfood O6, 2026-09-07) `sealed`, a state of the status DOCUMENT rather than a rung of the health ladder — nothing chains it, because a health event chained after the seal would be a link past the last link, and without it a sealed run went on showing a countdown. Sixteen states. Every transition is chained as a `Health` event, not only cached locally: the signed status cache is a participant surface, and a trace that holds no record that capture degraded cannot be reviewed for it. `hooks_silent` is evaluated against the connector's `hook_silence_policy` per event, not as a blanket rule, with "leg 2 silent while leg 1 grows" as the universal fallback and a short evaluation window where the harness can hot-disable (0.7 s on OpenClaw). `goalcup status --json` reports them from a signed local cache with last-sync time.
- R15. Finish: one durable `SealingIntent` closes admission and fixes the cutoff instant; drain
  already-admitted evidence to a bounded point; write the cutoff including that drain; bind its
  returned sequence/hash to the intent; reconcile the intent and chain before continuing; then
  terminate the managed process tree, freeze eligible workspace bytes into the private immutable
  V1 artifact, read it back, and write terminal finalization. Manual and deadline finish share
  this path and a retry cannot reopen the chain or resnapshot newer bytes. The daemon never
  mutates Git. V1.1 adds continuous repository observation, Git-aware eligibility/history,
  canonical tree construction, isolated publication, and remote readback.
- R16. Resume: `goalcup resume <entry>` catches up the tail, checks the connector's `lock_paths[]` before relaunching (a Codex resume that cannot take the thread-writer lock silently becomes a new thread), then relaunches the same session in the harness's resume mode with the same pins.
- R17. V1 writes no app-wide harness configuration. V1.1 Desktop/elevated-review surfaces may use only a declared `app_wide_exception` under D21, bracketed, reversed where possible, and disclosed. Never inject text into the agent.
- R18. Participant feedback (D25). The daemon serves one static status page on its local control endpoint and `goalcup start` opens it in the default browser once the workspace is armed; `goalcup ui` reopens it. The page polls the signed status cache and shows arming state, time remaining, last sync, events uploaded, the health state in plain words, and the harness-specific arming step where one exists (the Codex app). It is bundled with the release, carries a link to the Agent Community site and the sponsor list, and loads nothing from the network. The daemon also emits an OS notification at each state change (armed, clock started, back online with N events uploaded, one hour left, sealed). In launch mode on Claude Code the same status text is set as the session status line through the `--settings` file. The first-prompt hook refuses an unarmed workspace with a human-readable reason where the harness can block. None of this ever reaches the agent's context.

### 5.2 CLI (`goalcup`)

Eight sub-commands: `auth`, `start --harness <id> [--attach]`, `status [--json]`, `ui`, `entries`, `resume <entry>`, `seal <entry>`, `doctor`. Output is plain text plus JSON. Error messages go to the participant, never into the agent conversation.

**Verification is not among them.** It ships as its own binary, `goalcup-verify <trace-dir>`, so the daemon never links the verifier (D3) and there is one set of flags and one set of exit codes for it; `goalcup doctor --verify <dir>` relays that verdict's exit code for a participant who wants one command. Earlier wording here listed a ninth sub-command, `verify`; the CLI's own `--help` says there is none.

`seal` is the eighth and was added after M1 planning: R15 makes sealing a daemon responsibility, but the participant needs an early-finish path that prints what the daemon reports. It is not the only seal — the daemon also seals automatically at `deadline_at`, as `RULES.md` promises.

### 5.3 Agent-facing extras

`goalcup-mcp`: read-only tools `goalcup_event_status`, `goalcup_run_status`, `goalcup_entries` reading the signed cache; reports last sync when offline. A small skill file describing when to call them. Both shipped, neither auto-registered.

### 5.4 Connector manifest

Declarative (TOML). Fields: id, family, detection (binary, version command, tested range), launch recipe, arm recipe, stores to tail (path templates, mode, sequence key, subagent and spill locations), hook install recipe (folder-scoped files, shim reference), blocking prompt-length hook availability, human-message rule id, capture tier. Manifests and shims are hashed into the signed release; the server only accepts allowlisted hashes.

M0 proved fifteen further fields necessary. Schema v0 carries all of them:

| Field | What it declares |
|---|---|
| `arming.signal`, `arming.timeout`, `arming.recipe` | the event that proves leg 2 is live, how long to wait, and the human arming step where one exists |
| `start_signal.required` | whether `started_at` may be stamped without that signal (false on Claude, true on the Codex app) |
| `store[].cursor.type`, `unsafe_cursors[]` | `ordinal` / `rowid` / `sqlite_pk` / `session_seq` / `inode_offset`, and the cursors that must never be used |
| `store[].partition_key` | the field that separates tracked rows from foreign ones, applied at capture time |
| `human_rule.id`, `human_rule.exclusions[]` | the positive test plus the ordered list of harness-authored user-slot content and the required scoping clauses |
| `human_rule.byte_exact_field`, `normalisation.trailing_newline` | which leg-2 field is byte-exact, and whether one trailing newline is stripped before comparison |
| `join_key` | how leg 1 and leg 2 tool calls are paired where tool ids do not match (`turn_id` plus order on Codex) |
| `kill.strategy`, `lock_paths[]` | `process_group` / `native_pid` / `wrapper_unsafe`, and the locks whose absence means a degraded resume |
| `config_allowlist[]` | the named app-wide files whose deviation is meaningful, so the app-wide check is checkable on GUI harnesses |
| `app_wide_exception` | declared file, key, expected diff shape, reversal procedure, public disclosure string |
| `hook_silence_policy`, `hot_disable` | per event `expected` / `conditional(<reason>)` / `never_silent`, and `none` / `restart` / `hot(<seconds>)` |
| `blocking.capability` | `blocking` / `best_effort` / `record_only`, and where a blocked prompt still persists |
| `version.self_updating`, `version.sample_interval`, `version.record_field` | self-updating harnesses, how often to sample, and which field records the version per record |
| `aux_calls[]` | known auxiliary model calls (title generators, suggestion agents) so "every API call accounted for" does not misfire — **Removed from schema v0 in the M1a fix wave (2026-09-04):** four rows written across three connectors and no reader anywhere, and M2 freezes the schema. Held absent by `aux_calls_is_gone_from_the_schema_and_from_every_manifest` in `crates/goalcup-protocol/tests/manifest_load.rs`. The requirement below is the measurement that motivated it and stands as written. |
| `workspace_exclusions[]`, `archive_sources[]`, `receiver.token`, `tier`, `tested_version_range` | seal exclusions, stores where records leave the primary table, the per-run receiver token, the measured tier from D23, and the version range preflight enforces |

M1 and the dogfood runs added six more, so the list above is no longer the whole schema. `crates/goalcup-protocol/src/manifest.rs` is the schema itself — `ConnectorManifest` is `deny_unknown_fields`, so a manifest that names something not in it does not load:

| Field | What it declares |
|---|---|
| `home_seed` | what `goalcup start` and `goalcup resume` READ out of the participant's own harness configuration into the per-run harness home before the harness is spawned: candidate source files and the keys to merge, the whole files to copy out of `from_dir`, the subset of keys without which the start is refused, and the `trust_field`/`trust_keys` entry marking this run's own workspace trusted (D27) |
| `tool_classes` | which tool names count as a write for seal attribution, and the subset of those that run a shell command — a shell call explains a file only when the file's workspace-relative path appears in the command text (D11) |
| `foreign_scan[]` | where to look for another installed harness's sessions: harness, path glob, the field holding the session's working directory and the field holding its id. The paths are deliberately outside the per-run pins, because an unpinned installation under the participant's real home is where a second session in a bound workspace would land (R8) |
| `redaction.extra_patterns[]` | connector-declared secret patterns beyond the shared set (`SHARED` in `crates/goalcup-daemon/src/redact.rs`), each a `kind` and a `regex` (R10) |
| `heartbeat_seconds` | a per-connector override of the daemon's heartbeat interval, which the verifier reads back when it classifies a hole in the heartbeat series. Absent means the daemon's own default |
| `leg2_partition.path` | which leg-2 field names the workspace a post belongs to. **Schema only in M1**: nothing reads it, and it exists so that M3 — where one harness posts for several workspaces — does not have to reshape a schema M2 has frozen |

Three sections the list above folded into prose are first-class tables of their own in schema v0: `[preflight]` (`banned_env`, `required_paths`, `free_ports`), `[resume]` (`args`, `session_arg_style`), and `[[hook_install]]` — one entry per folder-scoped file the **daemon** renders and writes, carrying `path`, `template`, `scope`, `mode`, `optional`, `restore_invalidates_trust`, `pin_hash_as` (the sibling file's hash, pinned into the shim so a hot-reload deletion surfaces as `config_mismatch`) and `harness_owned` (the harness's own file: write it if it is absent, then never watch it, never restore it and never chain a deviation for it — added for `$CODEX_HOME/config.toml`, whose per-model counter no manifest can pre-write, with the bounded blind spot that opens written down in `docs/CHEAT-CHECKLIST.md`).

### 5.5 Verifier (`goalcup-verifier`)

- V1. Parsers per harness from raw stores to ATIF v1.8, raw files hash-linked from `extra`.
- V2. Human-message rule per harness (see `harness-research/README.md`), producing the ordered list of human messages. The rule is two-legged: a leg-2 post counts as human only when it pairs with a leg-1 record that passes the connector's positive test, is not on `human_rule.exclusions[]`, and satisfies the scoping clauses (no parent session, no delegate marker, not a subagent thread source, past the `subagent_history_start_ordinal` cut, session directory equals the workspace). A leg-1 user-slot record with no corroborating post is **judged, not dropped**: V2 runs over `messages ∪ unwitnessed`, so such a record that is not `keep going` is a **Fail** (`unwitnessed_not_keep_going`) and one that is `keep going` is a Flag (`unwitnessed_keep_going`) — the text is compliant, the capture is not. V3 reports the missing post separately as `leg1_without_leg2_post`, a Flag. (Corrected 2026-09-05: this sentence said "is a flag, not a human message", which the code has never done — judging only the witnessed records is how attack 1b, typing an instruction inside the hot-reload window, would classify itself out of the one rule the whole event rests on.)
- V3. Checks: original prompt recovered and within 4,000 characters; every later human message equals `keep going`; leg 1 and leg 2 agree on every human message and on request or message identifiers where issued; session continuity (fresh start once, resumes accounted for, no second root agent, no foreign harness, no traced invocation of known harness binaries); model identity and changes reported or `unknown`; subagent lineage resolves; version drift recorded; config deviations attributed to a traced tool call or not; gaps classified clean or dirty with the appended lines extracted; sealed files attributed to traced edits, writes, or shell commands.

  *Where each sub-clause lives in the code* (added 2026-09-05; the checks are `V1`…`V13` in `crates/goalcup-verifier/src/checks/`, and this list is one PRD item while the implementation is thirteen, so the mapping is written down rather than inferred): original prompt recovered and within 4,000 characters — **V1**; every later human message equals `keep going` — **V2**; the two legs agree on every human message — **V3**; and on request or message identifiers where issued — **V3** (`cross_leg_identifiers*`, the id spaces) and **V8** (the pairing itself, per id or per turn); session continuity, fresh start once, resumes accounted for, no second root agent, no foreign harness — **V4**; no traced invocation of known harness binaries — **M2**, unimplemented; model identity and changes reported or `unknown` — **V5**; subagent lineage resolves — **M2**: the parsers build `subagent_trajectories` and no check asserts the lineage; version drift recorded — **V5** (folded there because a mid-run change of what was answering is one question whether the model or the binary moved); config deviations attributed to a traced tool call or not — **V9**; gaps classified clean or dirty with the appended lines extracted — classified in `gaps.rs` and rendered by **V4**; sealed files attributed to traced edits, writes, or shell commands — `attribution.rs`, raised by `verify_trace` **outside any check** (`unexplained_seal_file`, `read_only_attribution`, `bulk_attribution`) because a seal belongs to no single check, with **V13** deciding whether the R3 baseline is bound tightly enough to exempt anything.
- V4. Output: a verdict document with deterministic results, evidence references, and a review queue of flags. Same output on server and offline.
- V5. Conformance suite: fixtures per harness fed through the parser with assertions for each check above.
- V6. Harness-authored user-slot content is classified, not counted: `<task-notification>`, `Continue from where you left off.`, the `/model` triple, sidechain and suggestion-agent payloads, `<recommended_plugins>` with `<environment_context>` concatenated, `# AGENTS.md instructions for `, task-tool and delegate prompts, `synthetic: true` messages, cron and compaction prefixes, `subagent_announce`, `main_session_restart_recovery`, blocked-message notices, `stop.followup_message`. The suite asserts each one is excluded on the connector's fixtures.
- V7. Trailing-newline normalisation: exactly one trailing newline is stripped before any byte comparison, and comparison uses `human_rule.byte_exact_field`. Wire-versus-typed divergence (Hermes `api_content`, OpenClaw `llm_input.prompt` timestamp prefix, the Cursor `<user_query>` wrapper, which is substring and not equality) is never tamper evidence on its own.
- V8. Request and response pairing where the harness issues identifiers, and through `join_key` where it does not: on Codex the two legs use *mostly* disjoint tool identifier spaces — `exec`-family calls are `exec-<uuid>` on leg 2 and `call_<id>` on leg 1, while the `collaboration*` tools share `call_` ids across both legs — and one leg-1 call can be several hook pairs, so the join goes through turn identity and ordering rather than through tool ids. (Corrected 2026-09-04 against `spikes/codex-cli/fixtures/`: the earlier claim that the two spaces "never match" is false; `call_FFBna0WHdpjVD98X321CRmft` and `call_jlSnFwWNLyjkVsMhs93Sh0Fv` appear on both legs.)
- V9. Hook-config change events: a `config_mismatch` post, a restored file, or a Codex trust invalidation is reconciled against the traced tool calls and reported with the window during which capture may have been degraded.
- V10. Foreign-session partitioning: no record in the chain fails its store's `partition_key`, and a foreign session detected inside a bound workspace is reported (a second Claude slug under the same config dir, a standalone CLI session in a tracked Codex workspace, a second OpenClaw session key).
- V11. Token accounting is derived, not enforced: the verifier sums the usage fields every harness already writes (Claude raw-body `usage`, Codex `token_count` events, OpenCode message tokens, Hermes `session_model_usage`, OpenClaw and Cursor usage fields) into a per-entry, per-model summary for the entry page and the research release, cross-checks leg-1 totals against raw bodies where both exist, and strips plan, credit and billing fields before anything is published. No rule depends on a token number.
- V12. Anchor reconciliation. Every `AckCheckpoint` carries a server signature over `(entry_id, seq, hash, acked_at)`, verified against a server public key pinned in the signed release and in the trace descriptor; the chain hash at that sequence must equal the checkpoint's hash. A mismatch, or no record at that sequence, is `ack_chain_mismatch`; a signature that does not verify is `ack_signature_invalid`; a trace with no acknowledgements at all is `no_anchor`, and every gap in it is dirty. Without this check the chain is only self-consistent, and `VISION.md`'s "acknowledged by the server as they happen" is not enforced anywhere. (Added 2026-09-04 from the adversarial M1 review.) `all_checks()` returns thirteen: V1 to V12 above and V13, which decides whether the R3 baseline is bound tightly enough to exempt a sealed file from attribution. Three further findings are raised by `verify_trace` outside any check — `unexplained_seal_file`, `read_only_attribution`, `bulk_attribution` — because a seal belongs to no single check.

## 6. Non-functional

- Rust. V1 provides an assisted, checksummed macOS bundle containing the CLI, daemon, verifier, MCP server, and exact pinned Codex connector; it makes no OS-signing/notarization claim. V1.1 adds the publisher, signed self-service packages, native services, and allowlisted production build hashes per supported OS.
- Daemon overhead negligible against a running agent: tailing and hashing must not stall on multi-hundred-MB session files (Codex rollouts have hit 700 MB).
- Raw bodies and spill files over 168 hours can reach tens of GB per entry; the buffer streams, never holds a run in memory.
- Privacy statements in `RULES.md` and `VISION.md` are requirements, not aspirations.

## 7. Milestones

**M0 — Capture spikes. Done 2026-09-03.** Nine spikes run (dsh prep only). Results: [M0 findings](M0-FINDINGS.md), the [public write-up](CAPTURE-RESEARCH.md#21-nine-spikes-one-per-harness-and-mode), and the included redacted fixtures described in its [public file map](CAPTURE-RESEARCH.md#71-where-things-live). Detailed per-harness reports are intentionally omitted from this public OSS cut. Tiers as measured are D23.

**M1 — Three `full`-tier connectors, end to end.** Claude Code CLI first, then Codex CLI and OpenCode as the second and third connectors: those three are the only ones M0 measured at `full`, and each exercises a different store shape (append-only JSONL plus raw bodies; append-only rollout with `ordinal`; home-wide SQLite).

- `goalcup-protocol`: envelope, start receipt, acknowledgement checkpoint, config-deviation and version-change events, seal manifest, manifest schema v0 with the fields in 5.4, filled in for the three connectors.
- `goalcup-daemon`: entry registry; workspace creation with the artifact root outside it (R3); launch with pins and `kill.strategy`; the arming state machine `armed` to `start_signal_seen` to `started_at`, with a refusal path and on-screen text (R4, R5); tailers behind a per-store cursor abstraction covering `inode_offset`, `ordinal` and `rowid` (R6); capture-time partitioning that discards foreign rows before hashing (R8); token-authenticated hook endpoint (R1, D24); config watch including the pinned hook-config hash (R9); hash chain, redaction log, encrypted buffer, upload to the mock server with catch-up; health states including `arming_pending`, `arming_lost` and `config_mismatch` (R14).
- `goalcup-cli`: `auth`, `start`, `status --json`, `ui`, `entries`, `resume`, `seal`, `doctor` — eight sub-commands, with verification as the separate `goalcup-verify` binary (D3, 5.2); the daemon's status endpoint and the bundled status page with OS notifications (R18).
- `goalcup-verifier`: parsers for the three harnesses to ATIF v1.8; the two-legged human rule with each connector's exclusion list (V2, V6); trailing-newline normalisation (V7); leg join by `turn_id` and order on Codex (V8); gap classification; partition and foreign-session checks (V10).
- `goalcup-mcp`: read-only status tools over the signed cache.
- **Acceptance:** `goalcup-verify` produces the **expected verdict** for each committed fixture set of Claude Code CLI, Codex CLI and OpenCode — stated explicitly per fixture in the M1 plans and committed as an `expected-verdict.json` beside each trace descriptor — byte-identically offline and against the mock server.

  "Expected" is not "clean". All three M0 fixture sets are **deliberately non-compliant scripts**: they contain a subagent request, a `write long.py` instruction, an `also add a README` request, a `what model are you?` question, the Codex `going` typo and a 4,100-character paste. Each must therefore yield `Fail` with exit code 2 and the named findings, and a run that turned any of them green would mean the rule had been broken, not the milestone reached. The earlier wording, "passes", is not used, because a future executor reading it would try to fix the fixtures — and the fixture is the measurement.

  **Plus one adversarial fixture set**, built deterministically from the Claude Code fixture and committed under `crates/goalcup-verifier/tests/fixtures/adversarial/`: a chain whose prefix was rewritten below the last acknowledgement, a truncated store, a planted file explained only by a read call, and a heartbeat hole. `goalcup-verify` must produce exactly four findings on it — `ack_chain_mismatch`, `store_rewritten`, `unexplained_seal_file` and a dirty gap. The three M0 sets exercise the human rule; this one exercises the tamper evidence, which is the other half of what `VISION.md` promises and which no M0 fixture reaches, because they carry no acknowledgements and no seal.

**V1 foundation — historical on `m2/wire`.** The short Codex CLI/macOS lifecycle and immutable
workspace artifact below describe the V1 `v1/trunk` line, not current M2 work. [D42](DECISIONS.md#d42-m2-open-source-v05-v1-path-behind-a-feature-data-only-harnesses-open-harness-prs)
keeps that V1/v4 path behind `v1-compat`, off by default and outside the v0.5 public surface.
Current M2 scope and status are recorded in the [public architecture](ARCHITECTURE.md#1-scope) and
[STATUS](../STATUS.md#what-m2-is).

**V1.1 — deferred product and event expansion.** Additional CLI connectors, continuous Git provenance
and publication, Desktop, native Linux/Windows, production organizer services and keys, signed
self-service packages, presentation operations, and later duration readiness remain outside the
current M2 boundary.

**M2 — current recorder line.** M2 is the local native-plus-wire recorder described by the [public
architecture](ARCHITECTURE.md#1-scope) and [STATUS](../STATUS.md#what-m2-is). It is source-only and
does not make a V1 completion or public qualification claim; the V1 hook leg is not on M2
([two-leg boundary](ARCHITECTURE.md#4-the-two-legs)).

## 8. Historical V1 acceptance context

The 2026-09-13 specification and delivery plan are private historical records intentionally omitted
from this public OSS cut; they are not operative acceptance authority. For this public cut, the V1
summary in this PRD, the included [STATUS V1 section](../STATUS.md#v1-on-v1trunk), [D42](DECISIONS.md#d42-m2-open-source-v05-v1-path-behind-a-feature-data-only-harnesses-open-harness-prs),
and the [public architecture](ARCHITECTURE.md#1-scope) are the authoritative public summary and
boundary; no private plan is an operative dependency.

Historically, the V1 scenario was an assisted participant installing the checksummed macOS bundle
without building Rust, running `doctor`, starting the exact pinned Codex CLI connector, binding one
prompt, exercising capture, interrupting and recovering the daemon, resuming the same session,
finishing manually, and verifying the immutable artifact. A second fresh entry reached an automatic
signed short deadline with no participant CLI present and qualified only with confirmed termination,
cutoff, artifact capture, and identical offline/development-round-trip verification. Explicit
termination uncertainty was retained only as a negative case and could not qualify or yield a clean
finalization/verdict. No 168-hour wait was required and no production or broader-platform claim
followed.

## 9. Open items for M1

M0 closed the questions listed at the end of `harness-research/README.md`; what remains is `M0-FINDINGS.md` section 6, ordered. Each item is marked with where it stands at the v1 candidate; `STATUS.md` is the live tracker and this list is not:

1. **Closed in M1.** Claude Code CLI vertical slice against spike 4's fixtures: `command` `SessionStart` shim, two-legged human rule, `(session_id, source)` dedupe for the double-fired resume, raw-body classifier on `cc_prompt_id`, `cc_is_subagent` and `cc_prev_req`.
2. **Closed in M1** (`crates/goalcup-daemon/src/partition.rs`, `cursor.rs`). Capture-time partitioning and the per-store cursor abstraction. Fix the stub tailer's rowid cursor before it becomes daemon code, and lock the sequence counter (two duplicate seqs observed on spike 9).
3. **Closed in M1** (`crates/goalcup-daemon/src/arming.rs`). The arming-gate state machine, with its refusal path and on-screen text.
4. **Half closed.** Manifest schema v0 with the 5.4 fields exists and is `deny_unknown_fields`; it is filled in for the three connectors M1 ships, not for the seven measured ones. The remaining four are M3.
5. **Closed in M1** (`crates/goalcup-daemon/src/receiver.rs`; `hook_silence_policy` declared in all three shipped manifests). Receiver authentication and the per-connector hook-silence policy tables.
6. **Decided, not done.** dsh is unsupported for v1 (D23) and parked (`STATUS.md`). The question below is what a later spike would have to settle first. Run the spike for real or leave the harness unsupported. Decide `latest` 0.1.1-rc.2 against `next` 0.1.2-rc.1 first; headless has no resume, so the browser surface is the only mode and A9 cannot pass cleanly.
7. **Open.** Close the four never-exercised legs: spill paths on Claude Code and Hermes, the permission and approval leg everywhere, compaction and rewind everywhere, a mid-stream kill on Claude Code.
8. **Open, M3** (the Codex desktop app has no v1 connector). Conformance gaps: `SubagentStart` and `SubagentStop`, `SessionStart` on reopen, `requirements.toml` managed hooks as the cleaner organiser mechanism.
9. **Open, M3** (Cursor has no v1 connector). A protobuf reader for the `store.db` blobs, and a TTY pass to confirm the interactive human witness now that `-p` is banned.
10. **Partly closed.** OpenCode's TUI recipe was driven live on 2026-09-07 and holds end to end (`docs/dogfood/2026-09-07/`); Hermes, OpenClaw and Cursor are untouched. TUI passes for OpenCode, Hermes, OpenClaw and Cursor: every one of those spikes was orchestrator-driven and cannot speak to what a participant sees on screen.
11. **Open.** Update `docs/harness-research/README.md`: the Cursor row predates spike 9 and the dsh row names a version that is not the one installed.
12. **Open.** Rewrite the per-harness research files the spikes found wrong: `codex-and-cursor.md`, `claude-code.md` sections 4 and 8, `deepseek-harness.md` section 5.
