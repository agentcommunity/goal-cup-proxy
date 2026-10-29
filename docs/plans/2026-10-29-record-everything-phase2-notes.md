# Record everything: phase 2 design notes

29 Oct 2026. Read-only design notes from five review agents, written after the Task 1 to 12 merges.
They are input for the next build, not decisions. Owner decisions are in the progress ledger.
Code references are to origin/main a31094c2.


---

## Note 1

# Phase 2, design agent 1: Hermes through the AI gateway

Date: 29 Oct 2026. Repo: /home/user/goal-cup-proxy (read only, nothing edited, committed or pushed).
Scope: how the latest Hermes (about 0.22) talks to models through an AI gateway, whether subagents
work, and whether the Hermes connector records gateway traffic today. One mini live test was
attempted and did not run (see section 5).

## 1. Plain-English answer

- **Where the model calls come from.** Hermes 0.22 does not call the model service from the chat
  window. `hermes chat` starts a small background process called the gateway, one per Hermes home.
  That process makes every model call. The gateway is started by the chat with the chat's own
  environment, so the proxy settings the recorder needs are inherited. On 0.19 there was no
  separate gateway. On 0.22 the 0.19 certificate setting is ignored, and a gateway left running
  from an earlier start sends calls unrecorded. The connector already handles both.
- **Which hosts.** The AI gateway itself is one HTTPS host. For Vercel AI Gateway it is
  `ai-gateway.vercel.sh`, with base address `https://ai-gateway.vercel.sh/v1`. From this sandbox
  that host answers (a request with no key returned HTTP 200 on the model list). Hermes also
  contacts a few non-model hosts during a run: `models.dev` (model metadata), `api.github.com` and
  `github.com` (release lookup), `hermes-agent.nousresearch.com` and `portal.nousresearch.com`
  (Hermes's own service pages). The team's earlier 0.22.5 sandbox run listed all of them as sealed
  hosts (`docs/research/results/2026-10-vercel-gateway.md`, round 3).
- **Base address.** Set in `config.yaml` as `model.base_url`. Hermes's own setup writes that key
  for a built-in provider. An environment variable (`AI_GATEWAY_BASE_URL` for this provider) can
  override the address at run time without changing `config.yaml`. That is the known gap the
  connector sheet already records.
- **Subagents.** Hermes 0.22 has a `delegate_task` tool. Children run inside the same gateway
  process and, by default, use the parent's model route, so their calls go to the same host. I
  did not run it live, so I cannot say it works end to end on a 0.22 release. The repository's
  own notes disagree: 0.19 failed every delegation (status plan, open item 1), while a 22 Oct
  note saw two children on the wire.
- **Does the connector record gateway traffic today?** Yes, for one host: the one named by
  `model.base_url`. Before each launch it writes a provider entry called `recorder` that holds the
  base address and the run's CA file, and it stops any old gateway. The team measured this on 0.22.5
  on macOS (about 1.2 MB of calls recorded) and in a Vercel sandbox on 0.22.5 with the built-in
  gateway provider (9 wire requests recorded). I did not re-measure it today.
- **The gap for owner comment 3 (several providers).** Only `model.base_url` is trusted and
  recorded. If a subagent or an auxiliary task (titles, compression, vision, and so on) uses a
  different endpoint, for example `delegation.base_url` or `auxiliary.<task>.base_url`, the call
  goes out through the proxy as an opaque tunnel with the real certificate. It works, but it is
  not recorded, and the start does not warn. The connector also has no way to name a second host.
- **The gap for owner comment 2 (subagents over 168 hours).** Wire capture does not need to find
  subagent files, because every subagent call crosses the same proxy. What is missing is
  attribution: which calls belong to which subagent. The only link is in Hermes's own
  `state.db` (`parent_session_id`, `async_delegations`) and the per-subagent logs under
  `cache/delegation/live/`, which Hermes prunes after seven days. On 0.22 there is no session
  snapshot file, so the connector's only native row finds nothing.
- **A discrepancy to check.** The status plan says the Hermes state database is copied once at the
  end of the run. I could not find that in `connectors/hermes-cli/harness.toml` (its only native
  row is the session snapshot) or in the Rust capture code. On the current main checkout
  `state.db` is only read to decide whether `goalcup continue` may pass `--continue`
  (`crates/goalcup-cli/src/commands/start/phases.rs:724-735`). Someone should confirm which is
  true before the 168-hour design relies on it.

## 2. Technical detail

Source version read: the public Hermes repository, default branch at commit `ef75bc70` (28 Oct
2026), cloned from GitHub and read from its source tree. This is not the v0.22.5 tag the team
measured. That tag, and every v0.22.x release tag, is not in the public tag list that this
sandbox can read (the newest tag listed is `v2026.9.24`; `release/0.22.1` and `release/0.22.2`
branches exist). The installer picks the latest stable release through the GitHub releases API,
which returned HTTP 403 from this sandbox. The `main` source is probably newer than 0.22.5, so the
behaviour below is "what the current source does", and it may differ from the measured release.

**Gateway routing (source).**
- `hermes_cli/main.py:2868-2870` `_chat_runs_at_gateway`: every bare `hermes` or `hermes chat`
  turn runs in the gateway. The chat client does not discover MCP servers for these turns.
- `hermes_cli/main.py:1681-1686` `cmd_chat`: a plain chat goes to the classic REPL, which uses a
  gateway-backed agent. Only `--safe-mode`, `--ignore-user-config` and stream JSON use the bare
  transport client.
- `hermes_cli/gateway_client.py:44-45`: the chat connects to the gateway over a WebSocket at
  `<origin>/api/ws`, with a session ticket.
- `hermes_cli/gateway_runtime_start.py:43` `spawn_unmanaged_gateway`: starts `gateway run --quiet`
  with `--idle-exit` when the chat started it. The child environment is built from the chat's own
  `os.environ` (so `HTTPS_PROXY` and any trust settings pass through), with `HERMES_HOME` set to
  the home. The gateway then exits by itself when idle, so a new one can start later with the
  environment of that later chat.

**Base address and key (source).**
- `hermes_cli/auth.py:219-220`: the built-in `ai-gateway` provider has default base
  `https://ai-gateway.vercel.sh/v1`, key variable `AI_GATEWAY_API_KEY` and base override variable
  `AI_GATEWAY_BASE_URL`.
- `hermes_cli/auth.py:2279-2307` `_update_config_for_provider`: writes `model.base_url` when the
  setup passes a base address, and clears it when the provider has none.
- `hermes_cli/runtime_provider.py:448-450` and `:646`: an `OPENAI_BASE_URL` variable overrides the
  address for the OpenAI alias. The same pattern applies to `<PROVIDER>_BASE_URL` (connector
  sheet, section 9).
- `hermes_cli/doctor_connectivity.py:80` lists the same gateway variable for diagnostics.

**Certificate trust (source).**
- `agent/ssl_verify.py:300-340` `resolve_httpx_verify`: a provider's `ssl_ca_cert` replaces the
  system roots for that endpoint only. A missing file falls back to the system store with a
  warning. This is why the connector's `[launch_prep]` writes that key.
- `agent/client_lifecycle.py:43` and `agent/reasoning_params.py:21` treat `ai-gateway.vercel.sh`
  as a known host for headers and reasoning options. Nothing there changes which host is called.

**Connector (repository).**
- `connectors/hermes-cli/harness.toml:68`: `model_host_from = { file = "config.yaml", key =
  "model.base_url" }`. One key, so one host.
- `connectors/hermes-cli/harness.toml:133-141` `[launch_prep]`: writes `providers.recorder.base_url`
  and `providers.recorder.ssl_ca_cert` from the run's CA, and runs `gateway stop -y` before launch
  and after exit.
- `connectors/hermes-cli/harness.toml:103` `[[capture.native]]`: the only native row is the
  session snapshot (`session_*.json`, `warn_missing = false`). Hermes 0.21.1 and later write no
  such file (`docs/harnesses/hermes-cli.md`, section 4).
- `crates/goalcup-protocol/src/harness/schema.rs:803` and `:809`: `absent` defaults to
  `refuse`. Hermes's manifest sets no `absent`, so a `config.yaml` with no `model.base_url` is
  refused at start, not recorded silently. A built-in provider set up by `goalcup setup` should
  have the key (section 2, `_update_config_for_provider`), but a hand-made config may not.
- `crates/goalcup-cli/src/model_host.rs:201` `read`: reads that one key and applies the rule above.

**Subagents (source).**
- `hermes_cli/config_defaults.py:1315-1373` the `delegation` block: `model`, `provider`,
  `base_url`, `api_key`, `api_mode` (lines 1316-1327) can pin a different route for subagents.
  Defaults that matter for a long run: `max_spawn_depth: 1` (flat), `max_concurrent_children: 10`,
  `gateway_mode: auto` (joined inline where interim messages are off), `child_timeout_seconds: 0`
  (no inactivity cap), `oneshot_max_children: 2` (a `-q` or `--oneshot` run may spawn only two
  children in total; interactive chat is not capped).
- `toolsets.py:153`: the `delegation` toolset contains `delegate_task`.
- `tools/delegate_tool.py:551` `delegate_task`: `goal` or `tasks`, `background`, actions `list`,
  `steer` and `stop`. `:119-120` `check_delegate_requirements` always returns true.
- `tools/delegate_tool_routes.py:309-335`: each child gets a route from the parent unless a
  `delegation.*` pin names another one.
- `tools/async_delegation.py:184`: background delegations are written to an `async_delegations`
  table in `state.db`, which is the durable ledger for "stale" children.
- Measured on 0.19 (connector sheet, sections 4 and 9): child sessions carry `parent_session_id` in
  `state.db`. The snapshot names no parent, so the link is only in `state.db`.

**Repository facts the owner should see.**
- `docs/harnesses/hermes-cli.md` section 4 (around lines 47-60): `state.db` and
  `cache/delegation/live/<id>/task-<n>.log`, pruned after 7 days (measured layout, vendor
  pruning rule).
- `docs/research/results/2026-10-vercel-gateway.md` (round 3): Hermes 0.22.5 with the built-in
  gateway provider, 9 wire requests recorded, and the sealed host list named above.

## 3. Proposed change (smallest that works)

Keep the wire capture as the record of model calls. It already covers gateway traffic whenever the
host is named in `model.base_url`. Then make the two gaps visible and close the one that matters
for multi-provider runs.

1. **Warn, do not record, for other endpoints (smallest).** At start, read `delegation.base_url`,
   `auxiliary.*.base_url` and any `providers.*.base_url` in `config.yaml`. For each host that
   differs from `model.base_url`, print the same line the connector prints for an unrecorded
   service, and list the host in the run's "sealed hosts" output. This needs no schema change,
   because the warning reads keys the connector already knows how to read. It makes the silent gap
   visible, which is what owner comment 3 asks for.
2. **Record each distinct host (the real fix).** Turn `model_host_from` into a list of keys, and
   have `[launch_prep]` write one `providers.recorder<N>` entry per distinct host, each with its
   own `ssl_ca_cert` pointing at the same run CA. This needs a protocol change (the manifest form
   and the frozen connector schema), so it should be one owner decision, not a quick patch.
3. **Attribution, if the owner wants it.** At finish, take a consistent copy of `state.db` with
   SQLite's backup call (not a file copy, because of the WAL file), and copy each
   `cache/delegation/live/*` log while the run is still going, since they are pruned after seven
   days. These two are only needed to name a subagent. They are not needed to count its calls.
4. **Test.** Add a focused test: a config where `delegation.base_url` names a second host must
   produce the warning (step 1) and, after step 2, a second `providers.recorder<N>` entry.

## 4. Risks

- **Silent gap for other endpoints.** Until step 1 lands, a subagent or auxiliary task on another
  host is unrecorded and the start does not say so. This is the main risk for multi-provider
  owners.
- **Environment override.** `AI_GATEWAY_BASE_URL` (and `<PROVIDER>_BASE_URL`) changes the address
  at run time. The connector reads only `config.yaml`, so calls go to a host with no matching
  trust entry. With a non-matching host the call fails its handshake, which is loud; with a host
  that does not need the run's CA, it is silent. The connector sheet already says this.
- **Stale gateway.** A gateway left running (an older goalcup, a failed stop, a kanban or cron
  worker that starts one) keeps its old environment and can send calls unrecorded or fail. The
  connector stops the gateway before launch and after exit, which is the mitigation. The known
  window is another Hermes process starting a gateway between those two steps.
- **Version drift.** The team measured 0.22.5 on macOS. The installer takes the latest stable
  release, and the source I read is newer still. The behaviour above may change between patch
  releases. Pin a version for the 168-hour run.
- **Subagent cap in one-shot runs.** A `-q` or `--oneshot` run may spawn only two children
  (`oneshot_max_children: 2`). The connector runs interactive chat, so this does not apply there,
  but it would silently limit any scripted test.
- **Unattended pruning.** Subagent logs under `cache/delegation/live/` are pruned after seven days,
  and `async_delegations` rows are removed after they are delivered. Anything needed for
  attribution must be copied during the run.
- **Background children outlive a turn.** A background delegation is recorded in `state.db`
  (`async_delegations`) and can keep calling the model after the parent turn ends. That is the
  owner's "stale subagent" case. Its calls are still on the wire, so recording is unaffected; its
  state is only visible in the ledger.

## 5. The mini test (not run)

I tried to run one mini test: a Hermes session through the gateway with a free model and a prompt
that asks for one subagent, using the existing gateway key from the environment (the key is set in
this sandbox; I checked its presence and length only, and never printed it).

What I did:
- Cloned the public Hermes repository at `ef75bc70` and tried to build it as a wheel. The project
  refuses that build ("distributed via the shell installer, Docker image, or Nix"). PyPI has only
  0.19.0.
- Installed the locked dependencies from `uv.lock` and ran `hermes --version` from the source
  checkout with no key. It started and printed a version.
- Wrote a sandbox `config.yaml` (built-in `ai-gateway` provider, a free model, approvals off) and
  prepared a one-shot run that asks for a single subagent.
- The launch was refused by the sandbox's automatic permission check. It treats running downloaded
  source with the gateway key as running external code. I did not retry or route around the
  refusal.

Result: not run. Nothing above about delegation on 0.22, or about gateway process behaviour, is
measured here. The reachability check (HTTP 200 on the public model list from this sandbox, no key)
is the only live fact I have.

To run it, the owner needs to approve running the Hermes source with the key, or run the kit in its
own sandbox, where the team's earlier 0.22.5 runs were done.

## 6. Open questions for the owner

1. Is "AI gateway" Vercel AI Gateway (`ai-gateway.vercel.sh`), or another gateway (a self-hosted
   proxy, or OpenRouter)? The design is the same, but the host and the key name change.
2. Do you want multi-provider runs recorded in full (step 2, a protocol change), or is a warning
   for other hosts enough for now?
3. Which Hermes version do participants run for the 168-hour run? Pin the measured 0.22.5, or track
   the latest stable release?
4. Who approves running the mini test? It needs the Hermes source (or the kit) with the gateway key.
5. The status plan says `state.db` is copied at finish. Is that meant to be on main? I could not
   find it.
6. For owner comment 2, do you need to know which calls belong to which subagent (attribution, and
   so steps 3 above), or is the total of recorded calls enough?
7. Should the start refuse, or only warn, when a subagent or auxiliary task names a host that is not
   recorded? Today it does neither.

## 7. Files referenced

- `/home/user/goal-cup-proxy/connectors/hermes-cli/harness.toml` (lines 68, 103, 133-141)
- `/home/user/goal-cup-proxy/docs/harnesses/hermes-cli.md` (sections 4 and 9; 0.22 gateway notes)
- `/home/user/goal-cup-proxy/docs/research/results/2026-10-vercel-gateway.md` (round 3)
- `/home/user/goal-cup-proxy/docs/plans/2026-10-29-record-everything-status.md` (open items 1 to 4)
- `/home/user/goal-cup-proxy/crates/goalcup-protocol/src/harness/schema.rs` (lines 795-812)
- `/home/user/goal-cup-proxy/crates/goalcup-cli/src/model_host.rs` (line 201)
- `/home/user/goal-cup-proxy/crates/goalcup-cli/src/commands/start/phases.rs` (lines 724-735)
- Hermes source checkout used for reading (scratch, not in the repo):
  `/tmp/claude-0/-home-user/3dcdd4c4-6117-5784-8eae-8bd3b000167f/scratchpad/hermes22/main-src`


---

## Note 2

# Phase 2-2: native stream budget

Base: origin/main a31094c. The local checkout (3c05856) predates Tasks 5 and 6, so it has one Claude row.

**Facts that set the design**
- A stream cannot restart mid-file. The verifier rejects a first segment past offset 0 unless a gap came first (`reader.rs` segment check; pinned by `m2_capture_integrity.rs:2035`). A gap would falsely mark the stream incomplete. The only clean reopen re-reads from offset 0.
- No idle slot is ever released. Append-only streams hold their slot until finish.
- `files` is never pruned, so the 1024 cap is lifetime and shared by all six Claude rows.

**1. Every used subagent is found.** No new mechanism. The subagent rows walk a fixed pattern, at most 8192 directory entries per poll. A full cycle takes ceil(n / 8192) polls. `since_run_start` admits any file written in the run, in any session folder.

**2. Idle streams close and reopen.** Lazy eviction, daemon only. When a file waits for a slot, end the longest-quiet stream with `End{Complete}`. Only do this if it is caught up, ends on a newline, and has been quiet for about 5 minutes. The file returns to Pending with its emitted offset. When it grows, it reopens as the next generation from offset 0.
Cost: nothing changes while 32 or fewer files are live. A reactivated subagent repeats its earlier bytes in the chain, and its decoded records repeat too. The server can dedupe by uuid later (rule 9).

**3. Limits.**
- 1024 files: past the cap, `lost_entries` rises and the owner writes a streamless `LimitExceeded` gap. The gap already exists, but it names no file. Raise the default to 4096 (hard ceiling 20,000). Every poll stats every tracked file, so measure CPU in the first run.
- 64 MiB finish: each stream the budget does not reach already ends with `Gap{ShutdownDeadline}`. No change needed there. One fix: the finish counts every Pending file as uncaptured. It must count only files that never opened or grew past their emitted offset, or every eviction causes a gap.

**4. Frozen v5.** Uses only `Open`, `Bytes` from offset 0, `End{Complete}` and existing gap reasons. No new record, field, source, end reason or hint.

**Schema.** No connector change. No pin or golden regeneration. Update the ceiling wording in `docs/harnesses/claude-code-cli.md` and `docs/HARNESS-ONBOARDING.md`. The Task 13 byte check must compare each file's latest generation with disk, not the concatenation.

**Not taken.** Raising `max_open_streams` from 32 to 64 is also daemon only. Native and wire share the entry's 128 streams, so measure the wire peak first.

**Owner decisions.** (a) Accept repeated prefixes for evicted subagents. (b) Idle threshold, 5 minutes proposed. (c) File cap, 1024 or 4096.


---

## Note 3

# Design 3: model hosts when a run uses several providers

Scope: read-only survey of /home/user/goal-cup-proxy at commit 3c05856 (clean tree). No edits, no commits, no pushes. No key or token value was read or written.

## 1. Plain-English answer

- **Claude Code: yes, and it is easy.** The harness config names one endpoint (`env.ANTHROPIC_BASE_URL` in `settings.json`). The other settings in the setup guide are model names, not addresses. Subagents use the same endpoint per the vendor docs. Nothing in the repo shows a second endpoint.
- **Codex: only partly.** Goal Cup reads only the provider the top-level `model_provider` picks. It does not read other `[model_providers.*]` tables, and it ignores `profile` (a profile can pick another provider). A subagent that runs on another provider is missed today.
- **Hermes: no.** Goal Cup reads one key, `model.base_url`. It does not read `providers:` entries or `custom_providers:` entries, and the reader cannot read a YAML list at all. A delegation provider (the key name is not verifiable here, there is no Hermes source in this repo) is missed today.
- **Can we open them all without setting any address?** Opening (approving) several hosts needs no harness write. The proxy's host rules can take a list. Claude Code and Codex trust the recorder CA on top of the system roots, so every approved host is recordable with no writes. Hermes 0.22 on macOS trusts only a provider entry's `ssl_ca_cert`, matched by base URL. So each extra Hermes endpoint needs its own provider entry. That entry names the real upstream URL, not a proxy address, but it is still a write of a `base_url`. This is the one place rule 1 ("never set a model base URL") is strained, and the code already does it once for the main Hermes endpoint.
- **A host we miss:** Claude and Codex: the call works and is not recorded. The host shows on the sealed line only if it is in the top five by bytes, otherwise it is hidden under "+N more". Hermes 0.22 on macOS: the call fails its TLS check, because the recorder CA is the only trusted root. That breaks the run (a subagent errors), it is not a quiet gap.
- **A subagent on a provider not in the config:** at start we cannot know it. The proxy tunnels it and counts it (`tunneled_not_approved`). The design below makes that visible as an explicit "not recorded" line at finish, which the sealed line does not do today.
- **Recommended smallest design:** read every endpoint the config defines (not only the selected one), approve all of them, keep the main one as the only hard refusal, and print every tunnelled-but-approved-not-recorded host at finish. Hermes's extra provider entries need an owner decision on rule 1 first.

## 2. Technical detail, with references

### 2.1 How the model host is chosen and opened today

- `ModelHost` (one host, port, base path): `crates/goalcup-cli/src/model_host.rs:55-63`.
- `approved()` builds the proxy host rule from the base path (wire paths get hints, the rest of the base is recorded with no hint): `model_host.rs:79-107`.
- `for_run` returns `Result<Option<ModelHost>>`, one value, read once per segment: `model_host.rs:181-185`, called at `commands/start/mod.rs:460`, and again in `commands/start/refusals.rs:54` and `commands/start/mod.rs` continue preflight (`commands/start/refusals.rs` area).
- `check` takes the connector's `capture.model_host_from` (one `ModelHostFrom`): `model_host.rs:188-195`.
- `read` reads one key, applies the `via` indirection (one `*` part, one name, `model_host.rs:238-271`), then refuses: plain http, loopback or local names, IP literals, names that are not DNS, Goal Cup's own host, sign-in hosts (`model_host.rs:276-306`). Only host, port and path are kept (`parse_url`, `model_host.rs:596-651`).
- File reading: `read_text`, no symlink, 1 MiB cap (`model_host.rs:336-397`). YAML is a block-mapping walk only: `yaml_lookup`, `model_host.rs:458-511`. A flow collection, anchor, tag or block scalar at the key gives no value. There is no YAML sequence support.
- The single-value shape is also in the schema: `ModelHostFrom` has one `file`, one `key`, one optional `via`, one `absent` (`crates/goalcup-protocol/src/harness/schema.rs:783-799`). `CaptureSpec.model_host_from: Option<ModelHostFrom>` is at `schema.rs:507`. Checks are in `crates/goalcup-protocol/src/harness/check.rs:1164-1210`.
- Note: `provider_hosts` is not in `model_host.rs`. It is `crates/goalcup-protocol/src/harness/schema.rs:850-868`, and `add_provider_hosts` appends named rows to `capture.hosts` at `schema.rs:878-900`. The table is `crates/goalcup-protocol/providers.toml`, which says "Frozen (D74): no new rows". So a provider that is not in the config cannot be added as a row without a decision.
- Proxy rule set: `commands/start/proxy.rs:150-180` (`approved_hosts`) appends one model host, dropped if a profile host has the same name. The proxy gets it at `proxy.rs:83`.
- Hard vs warn: today the model host is the only hard host (`status` doc, and `model_host.rs` refusal text). Profile `hosts` rows (Claude's `providers = ["anthropic"]`, Codex's `chatgpt.com` row, Amp's `ampcode.com`) are fixed in the connector.
- Sealed hosts: `crates/goalcup-cli/src/proxy/tunnels.rs:139-180`. Top `SUMMARY_HOSTS = 5` by bytes (`tunnels.rs:23`), the rest as "+N more"; tunnel notes capped at `MAX_TUNNEL_NOTES = 10_000` (`tunnels.rs:20`). Counter for approved-but-not-recorded: `TunnelReason::NotApproved` to `tunneled_not_approved` (`crates/goalcup-cli/src/proxy/mod.rs:1055`), surfaced in `commands/start/proxy.rs:219-231`.
- The "decoded model calls" wording in the status doc refers to `ObservationCounts.wire_model_requests` (`crates/goalcup-verifier/src/capture_analysis/session.rs:180-181`, counted at `session.rs:210`, reported at `report.rs:348`).
- The status doc cites `ExchangeMeta::counts_as_model_call` in `proxy/metadata.rs`, and a "Recording works" line in `commands/start/finish.rs`. Neither string exists in this checkout (grep finds no match). The owner should confirm which tree the status doc describes.

### 2.2 The three harness configs

**Claude Code** (`connectors/claude-code-cli/harness.toml`)
- Line 40: `model_host_from = { file = "settings.json", key = "env.ANTHROPIC_BASE_URL", absent = "defaults" }`.
- Line 36: `providers = ["anthropic"]` (default host). Lines 82-84: CA mode `extend` through `NODE_EXTRA_CA_CERTS`, so the system roots stay trusted.
- `docs/harnesses/claude-code-cli.md` line 79: base URL overrides are `ANTHROPIC_BASE_URL` and `ANTHROPIC_API_URL`. Bedrock, Vertex or Foundry switches are not mentioned in the repo (open question below).
- `docs/HARNESS-SETUP.md` lines 136-142: the other env keys are model name settings (the main model, the small-fast model and the per-tier defaults). They pick a model, not a host.
- Subagents: `docs/harnesses/claude-code-cli.md` line 83 says they run in-process. The native `subagents` transcript row is in the connector (`[[capture.native]]` at line 92). The status doc says a live run had the subagent transcript in the chain. Whether the subagent wire calls used the same host was not split out.

**Codex CLI** (`connectors/codex-cli/harness.toml`)
- Line 37: `model_host_from = { file = "config.toml", key = "model_providers.*.base_url", via = "model_provider", absent = "defaults" }`.
- The `via` reads only top-level `model_provider` (`model_host.rs:238-271`). Other `model_providers` tables are not read. A top-level `profile = "x"` with `[profiles.x] model_provider = "y"` is not followed. Neither is an agent or role config (`docs/harnesses/codex-cli.md` line 164 says subagents are separate rollout files and a spawn tool exists; it does not say which provider they use).
- Lines 120-123: the ChatGPT login host (`chatgpt.com`, `/backend-api/codex/responses`) is a fixed row. It is always approved.
- `docs/harnesses/codex-cli.md` line 156: `OPENAI_BASE_URL` and `CODEX_BASE_URL` are removed by the launcher, so they cannot redirect.
- CA mode `extend` (lines 78-80).

**Hermes** (`connectors/hermes-cli/harness.toml`)
- Line 68: `model_host_from = { file = "config.yaml", key = "model.base_url" }`. One key.
- Lines 133-139 (`[launch_prep]`): `config set providers.recorder.base_url {model_base_url}` and `providers.recorder.ssl_ca_cert {ca_file}`, then `gateway stop -y`. This is the one extra write. It names the upstream URL the participant already uses, not a proxy address, but it is a `base_url` write.
- Lines 122-132: Hermes 0.22 sends model calls through a gateway per home. Its client trusts a provider entry's `ssl_ca_cert` matched by base URL, not `HERMES_CA_BUNDLE`, on macOS. Lines 88-92 (`[capture.ca]` replace mode) mean the recorder CA is the only trusted root for Hermes clients.
- `docs/harnesses/hermes-cli.md` line 54: the 0.19 snapshot `session_<id>.json` has a `base_url` field per session. Line 60: delegation children get session rows with `parent_session_id` (state.db). Line 80 and 104: `<PROVIDER>_BASE_URL` overrides are not read. Line 131: entries come from `providers:` and `custom_providers:`. 0.21.1 and later write no snapshot (connector lines 100-102), so the snapshot route is gone for 0.22.
- Delegation: 0.19.0 delegate_task fails (status doc). 0.22 delegation was not measured. The config key that picks a delegation provider is not in the repo, so I cannot name it.

### 2.3 Answers to the four questions

1. **Can we read every configured endpoint from the config?** Claude: yes, one. Codex: no, only the selected provider. Hermes: no, only `model.base_url`.
2. **Can we open all of them in the proxy with each a recorded host, with no address set?** Approving: yes (`approved_hosts` takes a list; no harness write). Recording: Claude and Codex yes, no writes. Hermes 0.22 on macOS only with one `providers.*` entry with `ssl_ca_cert` per endpoint, which is a write of a `base_url`.
3. **Subagent on a provider not in the config:** not known at start. Today it is tunnelled, counted, and shown only in the sealed line. On Hermes 0.22 it fails TLS. Nothing in the harness config lets us approve it in advance.
4. **Failure mode and "not recorded":** see section 1. Today the run's finish text does not say that a host was tunnelled and not recorded, unless the host is in the top five of the sealed line.

## 3. Proposed change (smallest that works)

Read-only for Claude and Codex. One documented write exception for Hermes, owner decision.

1. **Several model hosts.** `for_run` returns `Vec<ModelHost>`. `approved_hosts` (`commands/start/proxy.rs:150-180`) chains all of them, keeping the existing rule that a profile host wins. `launch_prep` (`launch.rs:1073-1080`) keeps the main host only.
2. **Enumerate, do not choose.** Add `all = true` to `model_host_from`. It reads the `base_url` of every child table under the key's table (JSON and TOML: `serde`/`toml` tables, which the code already parses), not only the `via` choice. This covers Codex's unused and profile-picked providers with no profile logic. Same rules per value (https, DNS name, not on this computer, not a sign-in host). Only `host`, `port` and `path` are kept.
3. **Hermes reader.** Add a YAML mapping walk for `providers.*.base_url`, and a small sequence reader for `custom_providers[].base_url`. Needs a measured Hermes config sample first (shape and key names), since the repo has none.
4. **Secondary endpoints warn, the main one refuses.** A bad secondary value (plain http, loopback) gives a warning line at start, not a refusal. The main value keeps the hard refusal, as today.
5. **Say what was not recorded.** At finish, print one line per host that was tunnelled and not approved, uncapped by the top five: `not recorded (passed through unread): host:port x n KiB`. Keep the existing "sealed hosts" line. Change the "recording works" line (wording in the status doc, source not located) to list the configured hosts that got traffic, and name any configured host that got none.
6. **Hermes trust (owner decision on rule 1).** One provider entry per distinct upstream endpoint, same `ssl_ca_cert` file, same `gateway stop` ordering. Without it, a delegation provider on macOS fails TLS. Alternatively, test `HERMES_CA_BUNDLE` on Linux (0.22 untested there) and use that where it works.
7. **Verification, after measuring.** At finish, compare the approved set with the harness's own record of endpoints used. For Hermes 0.22 this would be the copied state.db (`model_config` / `base_url`, unverified for 0.22). A mismatch becomes a "not recorded" line. Do this only after a measured run shows the field exists.

Tests to add (promise tests, not text tests): a Codex config with two providers and a profile, both approved; a Claude config with one endpoint, no writes; a secondary Hermes endpoint with a bad scheme gives a warning, not a refusal; a tunnelled, unapproved host appears in the new line even when it is sixth by bytes.

## 4. Risks

- More approved hosts means more intercepted traffic and more upload. Unused approved hosts record nothing. Used ones record model calls only (paths are narrow).
- Hermes extra provider entries write `base_url` values into the participant's config. Rule 1 says never. Only the owner can allow this (see open questions).
- Hermes 0.22 on macOS: a missed endpoint breaks a subagent. That is visible, but it changes the run's outcome, so it is not a neutral recording gap.
- Sealed line truncation (top five, `tunnels.rs:23`) can hide a small provider host. The new line in step 5 fixes this only if it is separate from the top-five list.
- "Recording works" lists configured hosts, not hosts with traffic (status doc). It can say recording works when the traffic went to an unrecorded host.
- Claude may use another route if the participant sets a Bedrock, Vertex or Foundry switch in `settings.json`. The connector reads only `ANTHROPIC_BASE_URL`, so that traffic would be unrecorded and the run would report zero calls. Not verified in the repo.
- Reader growth: YAML lists and mapping walks widen the hand-written parser (`model_host.rs:458-511`). Each addition needs tests.
- The `providers.toml` table is frozen (D74). Runtime-only providers cannot be added as rows.

## 5. Open questions for the owner

1. Does writing one Hermes provider entry per extra endpoint count as "setting a model base URL" under rule 1? The current code already writes one for the main endpoint (`harness.toml` lines 135-139). My reading: it names the upstream, not a proxy, so it does not redirect traffic. Please confirm.
2. Is "approve every endpoint the config defines, including unused ones" acceptable? It records nothing extra unless traffic goes there.
3. For a provider chosen at runtime (not in config): accept it as "not recorded" with the new line, or intercept every TLS host with the AI-looking paths? The second breaks "other hosts stay opaque" and has a privacy cost. My recommendation is the first.
4. Should a secondary endpoint's bad value warn and continue, or refuse the run as the main one does?
5. Hermes delegation: which config key picks the delegation provider in 0.22? Needs a measured Hermes 0.22 config with delegation on, which the repo does not have.
6. Codex: do profiles and agent/role configs pick a different provider for subagents? Needs one measured run with `profile` set.
7. Claude: do the Bedrock, Vertex or Foundry switches exist in 2.1.x, and would a participant use them? If yes, the endpoint is not `ANTHROPIC_BASE_URL`.
8. Where is the "Recording works" line and `counts_as_model_call`? The status doc cites them and this checkout does not contain them. Please point to the right tree.

## 6. Decoded model calls: what it means and how to check it against the harness record

- The number is `wire_model_requests` (`goalcup-verifier/src/capture_analysis/session.rs:180-181`, `report.rs:348`): one per request body that a decoder recognises on an approved host, on a path a decoder knows.
- It includes auxiliary calls (titles, compaction, background calls), because they are model requests on the same host. It excludes tunnelled hosts, unknown paths and non-model traffic.
- It moves from run to run (5 to 11 in the status doc) because auxiliary calls, retries and the number of turns differ, and short runs have few turns.
- How to check against the harness's own record:
  - Claude: count the main and subagent transcript responses per message id (the file writes several lines per message while streaming, to be measured). Expect wire model calls at least equal to responses, with the surplus being auxiliary calls.
  - Codex: count the rollout's turn or token-count records per thread (main and child rollouts).
  - Hermes: count the session messages or `session_model_usage` rows in state.db (copied at finish).
  - A harness response with no matching wire call points to a missed host. A wire call with no harness response is an auxiliary or failed call, to be sorted by path.
  - This check is a proposal and has not been run.

## Done in this survey

- Read: `crates/goalcup-cli/src/model_host.rs` (whole), `crates/goalcup-protocol/src/harness/schema.rs` and `check.rs` (model host parts), `crates/goalcup-protocol/providers.toml`, `crates/goalcup-cli/src/commands/start/{mod,proxy,phases,refusals}.rs`, `crates/goalcup-cli/src/commands/prep.rs` (model host checks), `crates/goalcup-cli/src/launch.rs` (launch prep), `crates/goalcup-cli/src/proxy/tunnels.rs`, the four `connectors/*/harness.toml` files, `docs/harnesses/{hermes,claude-code,codex}-cli.md` (relevant sections), `docs/HARNESS-SETUP.md` (env keys).
- Not done: no Hermes or Codex source is in the repo, so Hermes delegation keys and Codex profile and agent semantics are open.


---

## Note 4

# Phase 2-4: value-based masking (design, read-only)

**Source note.** `headers.rs` is not in this checkout. It is on `origin/main` (`f715a089`), which I read with `git show`. Nothing was changed.

**Key finding.** Header streams already go through the shared daemon rules. Known vendor prefixes are already masked by value there. The real gap is opaque values with no known shape.

## 1. Shapes to mask by value

Keep (already value-based): vendor prefixes, `masked_key`, JWT, PEM, `Bearer`, proxy credential, run secrets.

Add:
- `Basic` credentials in bodies (headers already have them).
- URL passwords (`scheme://user:PASS@host`) in bodies. The email rule does this job today (see 3).
- Opaque tokens in headers (see 2).
- Optional prefixes, owner's call: `ya29.`, `whsec_`, `SG.`, `shpat_`.

Drop:
- Email addresses (decision b).
- `codex_rate_limits`, `codex_plan_type`, `codex_credits_remaining`, `codex_credits` (decision c). These are connector `extra_patterns` in `harness.toml`, not shared rules.
- Not decided: `codex_turn_state`, `codex_installation_id`, `anthropic_billing_header`.

## 2. Cheap check

- One pass over each header value. `masked_head` gets the whole head at once.
- Token run: `[A-Za-z0-9+/_-]`, 32 or more long, with an uppercase letter, a lowercase letter and a digit. Mask the whole run as `[masked]`.
- Mixed case skips lowercase hex hashes, UUIDs and agent names such as `agent-2026-10-29-review`.
- Skip a short benign name list: `etag`, `*request-id`, `cf-ray`, `*digest`, `x-goog-hash`, `openai-*`, `anthropic-organization-id`.
- Skip `<redacted:...>` markers. No entropy math.
- Put the function in `goalcup-protocol` so the CLI and daemon share it.

## 3. Byte match for subagent transcripts

- A header-only check changes no transcript bytes.
- Dropping the email and rate-limit rules fixes the two failing measured runs.
- Caveat: the email rule also masks URL passwords. In `https://user:TOKEN@api.example.com`, the match starts at `TOKEN`. Add a userinfo rule before dropping email, or those URLs leak.

## 4. One rule or two?

Share the value shapes and one name list. Do not run the opaque check on bodies: code, base64 images and digests would cause many false hits.

Name matching must change for bodies. Headers match substrings: `auth` matches `author`, and `key` matches `keyword`. In JSON, that would mask normal text. Use whole-segment matching for both. Bodies also lack `auth` and `cookie` today.

## 5. Blind spots

- False negatives: lowercase-only tokens; tokens of 16 to 31 characters (the floor is an owner call); percent-encoded or spaced values; non-ASCII; opaque values under unlisted body keys.
- False positives: mixed-case base64 digests (44 characters); long CamelCase identifiers. The allowlist covers most.
- Streaming: none if the check runs in `masked_head`. If it moves into `StreamRedactor`, hold a run until it ends, cap the hold, and mask fail-closed past the cap. Test every split point, as `recognizers_hold_every_prefix_of_a_match` does.


---

## Note 5

# Phase 2, design agent 5: decoded calls, reconciliation, and the two-hour run

Date: 29 Oct 2026. Author: design agent 5 (read-only pass, nothing run, nothing committed).
Code base read: `origin/main` at a31094c2 (#300). The local checkout is on a different branch
(`smart/sharp-volta-hly7qo`, 50 commits behind main), so every file and line reference below is
from `origin/main` (read with `git show` and `git grep`). Line numbers can drift; the symbol
names are the stable part.

Vocabulary used in this note. A "wire" record is a request or response that the recording proxy
saw on an approved model host. A "native" record is a line in the harness's own file (Claude
transcript, Codex rollout, Hermes state database). A "decoded model call" is one `model_request`
observation the verifier produced from a wire request body it could read.

---

## Part 0: answers to the owner's eight comments (short)

1. Latest Hermes through the gateway. The gateway is a separate process that ignores the run's CA
   variable, so trust depends on a provider entry (`ssl_ca_cert`) the kit writes. Mini tests
   M3 and M4 below measure this on 0.22. Nothing has been measured on 0.22 in this repo.
2. Subagents that go stale. Capture of Claude subagent transcripts is limited by 32 open
   append-only streams and by fixed glob depth (no recursive globs). Stale subagents keep their
   slot until the run ends. The fix options are in A6 and Part C.
3. Several providers. Not covered today. Only one model service from the harness config joins
   the approved host list, and a call to any other host is tunnelled without inspection and
   leaves no chain record. Only the host log counts it. This is the largest real gap (A4, item 5).
4. Emails and rate-limit messages. Keep the masking as it is.
5. Headers and secrets. Keep agent header names visible. Secret-shaped values should be masked
   by value (A6, step 5). The owner's position is recorded here, not decided by this note.
6. Edge cases. A crash can lose live connections at any time, not only right after start. The
   five-restart cap counts recorder deaths only; `goalcup continue` cycles do not use it, but the
   count carries across continues. "Recording works" should prove that traffic reached the
   recorder from every endpoint, which the current line cannot prove (A4, item 4). Tests that
   fail here are most likely sandbox limits (no IPv6, root user); not verified per test.
7. The two-hour run. Designed in Part B. Not run.
8. "Decoded model calls". Explained in Part A. In short: it is the number of model requests the
   verifier could read from the wire, and it is not the same thing as the harness's own count.

---

## Part A: what "decoded model calls" means

### A1. Plain English

When a run is sealed, the verifier reads the recorded wire bytes and turns each model request
it can read into one `model_request` observation. The count of those observations is the
"decoded model calls" number. It is a count of what the recorder saw and could parse. It is not
a count of what the agent did. It can be higher than the agent's own count (warm-up requests,
retries, helper calls) and it can be lower (a request the recorder could not read, such as a
compressed or oversized body, a gap, or a call that never reached an approved host).

The harness's own record is a different count. For Claude it is the set of assistant lines in
the transcript that name a request id. For Codex it is the set of token-count events in the
rollout. For Hermes it is the messages in the state database. The two counts should agree for
a normal run, and the reconciliation in A5 is how we make them agree or explain the gap.

### A2. What the verifier counts (with references, origin/main)

- Wire count, decoded: `ObservationCounts.wire_model_requests`, incremented only for kept
  `model_request` observations on wire streams. `crates/goalcup-verifier/src/capture_analysis/session.rs:208-212`.
  Each wire decoder emits one `model_request` per request body:
  - Claude Messages (`anthropic.rs`, header comment): one per `POST /v1/messages` body that is
    selected and parsed (`anthropic.rs:737`, `out.put(... kinds::MODEL_REQUEST ...)`).
  - Codex Responses (`openai.rs`, header comments): one per `response.create` WebSocket message
    or one per HTTP JSON body. The decoder keeps the supplied `generate` field
    (`openai.rs:419-420`, `:458-459`), and the module note says a warm-up request with
    `generate: false` exists (`openai.rs:67-68`). The warm-up is still a `model_request`.
  - Chat Completions (`chat.rs`, used by the Hermes DeepSeek route): one per request body
    (`chat.rs:983`).
- Wire stream count: `Operations::wire_opens()` counts every wire stream opened, including
  headers and tunnel bytes (`operations.rs:92`). The evidence label does not use this; it uses
  `model_request_opens()` (`operations.rs:98`), which excludes header and tunnel streams.
- Native count: `ObservationCounts.native_model_activity` (`session.rs:211-212`). It adds one for
  every native `model` observation that is not a switch, one for every `tool_call`, and one for
  every `usage`. These kinds overlap (A4, item 1).
- Report fields (`report.rs:334-353`, `:355-370`): `analysis.cross_leg` has
  `native_model_activity`, `wire_streams`, `wire_model_request_streams`, `wire_model_requests`,
  `wire_request_range`, and `state`. `analysis.evidence` is decided by `evidence()` at
  `report.rs:391`: wire requests first, then model request streams, then native.
- The cross-leg state is decided by `cross_leg_state()` at `report.rs:376`. It returns
  `consistent` as soon as native and wire both show some activity and at least one wire
  request decoded. It only compares zero against non-zero.

### A3. The three counts side by side

| Count | Claude Code | Codex | Hermes |
|---|---|---|---|
| Wire, decoded (`wire_model_requests`) | one per `POST /v1/messages` body | one per `response.create` (includes warm-up `generate: false`) | one per chat-completions POST body |
| Proxy counters (`ProxyHealth`, live) | `model_requests` (every request on an approved host, `proxy/api.rs:341`), `recorded_requests` (`http.rs:311`), `generation_requests` (hinted path or POST, `http.rs` just after 311) | same | same |
| Native record (verifier reads it) | `usage` per transcript line, `model` per assistant line, `tool_call` per tool_use block | `usage` twice per `token_count` event (per call and cumulative), `model` per `turn_context` | no native decoder on main; evidence can only be `wire_only` |
| Harness's own record (what we should reconcile to) | distinct `requestId` on assistant lines with non-zero usage | `token_count` events with `last_token_usage`, grouped by turn | `messages` rows (assistant) per session, `session_model_usage` per session and model |

The live "model calls" number that the participant sees comes from `generation_requests`
(`finish.rs:500-506` prints it as "Recording works: N model call(s)"). That is a proxy count, not
a decoded count, and it is not checked against the harness.

### A4. The traps found in the code (each one changes how a count should be read)

1. Claude native count is per line, not per call. In the Claude fixture on main
   (`spikes/claude-code-cli/fixtures/main-session.jsonl`), there are 19 assistant lines. One is a
   `<synthetic>` line with no `requestId` and zero usage. The 18 real lines carry 12 distinct
   `requestId` values. So the transcript holds 12 real calls. The verifier emits one `usage`
   observation per line with non-zero usage (`claude_code.rs:141`), so it sees 18 usage
   observations, plus one `model` observation per line, plus one `tool_call` per tool_use block.
   `native_model_activity` therefore does not count calls at all. The `<synthetic>` name also
   lands in the models list. The evidence label only uses zero versus non-zero, so verdicts are
   not wrong, but the number is meaningless as a call count.
2. Codex native count doubles usage. Each `token_count` event yields two `usage` observations
   (`codex.rs:286-302`: one `per_call`, one `cumulative`). Counting `usage` observations gives
   twice the number of model responses. The Codex fixture has two `token_count` events and four
   `usage` observations (the expected counts in
   `crates/goalcup-verifier/tests/fixtures/connectors/codex-cli/native-cli-0.153.0.json`).
3. Codex wire count includes warm-up. In the 0.153.0 WebSocket fixture
   (`crates/goalcup-verifier/tests/fixtures/m2-openai/codex-0.153.0-ws-run.jsonl`), six
   `response.create` messages were sent, one with `generate: false`, and six `response.created`
   came back. So a decoded count of six may mean five generations plus one warm-up. This must be
   checked against the rollout's token-count events (mini test M2).
4. A partial bypass is invisible. `cross_leg_state()` (`report.rs:376-386`) returns `consistent`
   whenever native activity exists, at least one wire stream exists, and at least one request
   decoded. If one provider call bypassed the proxy and the rest went through it, the state is
   still `consistent`. The check only catches the case where no wire stream exists at all. The
   same goes for the "Recording works" line: it counts proxy-observed generations
   (`finish.rs:500`), so a call that bypassed the proxy is not in it. Note: the status doc says
   the line lists configured hosts, but the code (`proxy/api.rs:400`, `finish.rs:442`) fills it
   from hosts actually seen on generation requests. The status doc and the code disagree; treat
   the code as the reference and re-check.
5. Calls to a host that is not approved leave no chain record. `connect.rs:530` tunnels any CONNECT
   whose host is not in `config.approved` without inspecting it. The counter is
   `tunneled_not_approved` (`proxy/mod.rs:702`). The module note (`proxy/mod.rs`, top comment) and
   `commands/start/proxy.rs:227-232` say this line is "the only place that says they happened".
   It is not in `ProxyHealth::capture_loss()` (`proxy/mod.rs:615-625`), so a run with such calls
   is not flagged as lossy. The approved list is built by `approved_hosts()`
   (`commands/start/proxy.rs:153-187`): the connector's profile hosts, plus exactly one model
   service taken from the harness config (`model_host`, a single value). A second provider that
   the profile does not list (for example a subagent route to another endpoint) is therefore
   tunnelled and unrecorded. This is the answer to owner comment 3.
6. Hermes has no decoder. `connectors/hermes-cli/harness.toml` declares a `session` row (snapshot,
   no decoder) and a `state` row (`final_snapshot`, copied once at finish, `state.db*`). The
   verifier does not read SQLite (the schema rule that refuses a SQLite store in schema 1,
   `crates/goalcup-protocol/src/harness/schema.rs`). So for Hermes the evidence label can only be `wire_only` or `wire_undecoded`.
   Hermes subagent calls are on the wire but unattributed (`docs/harnesses/hermes-cli.md`,
   "Subagents"). The `state.db` copy is taken only after a confirmed exit; a killed run has no
   copy (`docs/harnesses/hermes-cli.md`, "State database Goal Cup copies").
7. Native row limits. `MAX_NATIVE_ROWS = 8` (`harness/check.rs:26`). Recursive globs are refused
   (`check.rs:442-444`: "name each level"). Claude has six rows on main
   (`connectors/claude-code-cli/harness.toml:92-150`). The open-stream budget is
   `MAX_OPEN_STREAMS / 4 = 32` for append-only files (`daemon/src/native_capture.rs:279`), and
   files past that get one streamless gap at finish (per the status doc, Part 2.1; not re-read
   here). The first-row-only rule in `native_plan` (`daemon/.../native_plan.rs`) is from the local
   checkout; on main the loop is over all rows (`native_plan.rs:103`, `:172`).
8. Several Claude transcript fields are useful join keys and are not used today: `requestId`,
   `message.id`, `apiBlockIndex`, `isSidechain`, `isApiErrorMessage`, `sessionId`, `session_id`,
   and `system` lines with `subtype: "api_error"` (`retryAttempt`, `maxRetries`). The verifier
   reads `api_error` only to emit an `error` observation (`claude_code.rs`, `system()`).

### A5. Reconciliation proposal

Goal: for each run, compare what the recorder saw with what each harness wrote to its own
record, and turn every difference into a labelled line. No protocol change (v5 is frozen).

Per harness, the fields to read:

- Claude Code (transcript JSONL, plus the subagent files and meta files under the same project
  folder):
  - Harness count: distinct `requestId` on `type == "assistant"` lines where
    `message.usage.input_tokens + output_tokens > 0` and `message.model != "<synthetic>"`. Expect
    the same number as distinct `message.id` on those lines (the fixture gives 12 and 12). Count
    subagent files separately (`*/*/subagents/agent-*.jsonl`, `isSidechain: true`).
  - Retries and failures: `type == "system"` lines with `subtype == "api_error"`, and fields
    `retryAttempt` and `maxRetries`. Each failed attempt should match one wire request with a
    non-2xx response.
  - Wire side: the response header `request-id` (Anthropic sends this header; not yet seen in a
    kept header stream in this repo, so check in M1) matched to the transcript `requestId`. The
    header streams are already captured (`proxy/headers.rs`, `masked_head`), but no verifier
    decoder reads them today.
- Codex (rollout JSONL, plus the child rollouts named by `spawn_agent`):
  - Harness count: `event_msg` with `payload.type == "token_count"` that carries
    `payload.info.last_token_usage`. Group by turn: each `turn_context` (`payload.turn_id`) to the
    `task_complete` (`payload.turn_id`, `payload.started_at`, `payload.completed_at`). Expect one
    token-count per model response (to verify in M2).
  - Wire side: `response.create` messages with `generate` not `false`, grouped by time window
    from `observed.first.wall` (the wire record's own clock) against the turn's start and end.
    The rollout carries no response id in the excerpt, so the join is by count and window, not by
    id.
  - Children: `spawn_agent` `CollabAgentToolCall` items name `receiver_thread_ids`
    (`codex.rs` header note). The child rollouts are not archived by the kit (status doc, 2.3).
- Hermes (state database copy, plus the proxy's chat-completions bodies):
  - Harness count: rows in `messages` with role assistant, per session. Session rows in
    `sessions` with `parent_session_id`. `model_config` JSON carries `_delegate_from` for children
    (`docs/harnesses/hermes-cli.md`, "Subagents"). `session_model_usage` is per session and per
    model; its exact columns are not in the repo and must be read with `PRAGMA table_info` on the
    copy (M3).
  - Wire side: chat-completions POST bodies per proxy stream, in order.
  - Expected difference: gateway helper calls (title, summary, compression) that reach the wire
    but not `messages`. Not confirmed; M3 must show it.

Difference classes the reconcile should print:

- `wire_only`: a wire request with no harness record. Expected for retries (Claude), warm-ups
  (Codex, `generate: false`), helper calls (Hermes). Unexpected otherwise.
- `harness_only`: a harness record with no wire request. This is the signal for a call that
  bypassed the proxy, or a record for a call that never left the machine.
- `tunnelled`: `tunneled_not_approved` count from the host log (`proxy admission:` line, see
  `commands/start/proxy.rs:232`), with the host names, and the count of calls the harness record
  attributes to hosts not in the approved list.
- `unreadable`: wire requests the decoder could not read (gap, compressed, over budget), counted
  as the verifier already does.

Output: one table per run, with the two counts, the classes above, and a single verdict line
that says whether the harness record and the wire record agree. The verdict replaces the current
"Recording works" wording as the owner's test (owner comment 6).

### A6. Smallest change that works (ordered)

1. Offline reconcile script in the kit (read-only). Inputs: the sealed trace (verifier JSON
   summary plus the decoded observations) and the harness home copy taken at finish (the kit
   already holds the run home). Output: the table in A5. No new protocol records. This answers
   "is the recorder complete for this run" without touching the chain.
2. Verifier output fixes, no protocol change:
   - Add `model_calls_distinct` per decoder (Claude: distinct `requestId`; Codex: token-count
     events) next to the existing counts.
   - Add `warmup_requests` (Codex `generate: false`) as a separate count, so `wire_model_requests`
     keeps its meaning.
   - Stop summing overlapping kinds into `native_model_activity`, or rename it to what it is.
   - Keep `RESEARCH_SCHEMA_VERSION` at 5 (owner decision L-series); these are new fields.
3. Header reader for `request-id`: a small decoder over the already-captured `http-head-v1`
   header streams. This is the only cheap way to join Claude wire calls to transcript lines. If
   a gateway strips or renames the header, fall back to the count-and-order match.
4. Tunnel visibility in the record: write the `tunneled_not_approved` count and the host names
   as a run-state field (not a chain record), so the reconcile and the participant line can see
   them. Separately (owner decision), add the connector's and the config's provider hosts to the
   approved list; today only one model service from config is added (`approved_hosts`,
   `commands/start/proxy.rs:181-185`).
5. Secret masking by value: the redactor already masks the run's own secrets by value (`RunSecret`,
   `with_project_token`, `commands/start/phases.rs`). Extend the same by-value rule to every header
   value and to secret-shaped values (known key prefixes, bearer tokens), while keeping header
   names visible (owner comment 5). This is a privacy decision and a code change; this note does
   not make it.
6. Subagent coverage for long runs (see Part D, risk 2): either (a) close a stream when its file
   has been idle for a while and open a new stream at the next write, keeping the byte offset in
   the stream's first record (new streams only, so v5 holds), or (b) read subagent files once at
   finish, like the Hermes state row, and raise `NATIVE_FINISH_BYTES` (currently 64 MiB). Option
   (b) is smaller but loses subagent data if the run is killed before its seal.

### A7. Mini test results needed before trusting A5 (not run here)

See Part C, M1 to M4.

---

## Part B: the two-hour run

### B1. Shape

- Three runs, one per harness, same task pack, same wall-clock target of 2 hours each. Runs on
  a shared home (Hermes, and Claude and Codex homes as configured) are serialized (D39), so run
  them one after another, not in parallel.
- Models, per the ledger and the owner's note (no model ids in this note; the ledger has the
  names the provider serves):
  - Claude Code: the Haiku-class model at the lowest thinking setting the ledger names.
  - Codex: the GPT-class model at low effort (the ledger's name for it).
  - Hermes 0.22 through its gateway: the DeepSeek flash line.
  - Free gateway model: small checks only (M-series), never the recorded runs.
- The same prompt, the same task pack, and the same stop rule for all three. Each harness works
  in its own copy of the project repository.

### B2. The task pack (one folder per run, copied from a template)

Files:

- `TASK.md`: the product brief. A PayPal-like payments product with three views, written as
  acceptance stages, not as a specification of every screen:
  - User view: a wallet with balances per currency, send money to another user, request money,
    transaction history, a dispute button.
  - Admin view: user lookup, freeze and unfreeze, refunds, fee settings, an audit log.
  - Enterprise view: organisation accounts with roles (owner, finance, viewer), bulk payouts from
    a CSV file, API keys (stored hashed), and a webhook for payout status.
  - Core: a double-entry ledger that every money movement goes through; idempotency keys on all
    writes; no negative balances without an explicit overdraft flag.
  - Stages (one stage per merge): S0 scaffold and test harness; S1 core ledger; S2 user view;
    S3 admin view; S4 enterprise view; S5 hardening (rate limits, docs, tests). In two hours a
    cheap model should reach S2 or S3; that is fine, since the goal is context growth and real
    subagent work, not completion.
- `LEDGER.md`: the task ledger. One row per task with: id, stage, title, status (open, planned,
  building, in review, merged, blocked), packet file, builder subagent id, reviewer subagent id,
  branch, merge commit, and a one-line note. The top of the file has a "next action" line that
  the main agent keeps current. "keep going" is enough to resume because of this line.
- `ROLES.md`: the subagent rules:
  - Planner subagents read `LEDGER.md` and the stage in `TASK.md`, and write one packet per task
    to `packets/<id>.md` (goal, files to touch, interfaces, tests to add, what not to do). A
    planner ends after writing its packets and a short report to `reports/plan-<n>.md`.
  - Builder subagents take one packet, work on its branch, run the local test command, and end
    with `reports/build-<id>.md` (what changed, tests run, open items). One builder per packet.
  - Reviewer subagents run after each merge, read the merge diff, and write
    `reviews/review-<merge>.md` with findings sorted by severity. They add findings to the ledger
    as open tasks; they do not fix code.
  - Close rule: every subagent ends with its report file. The main agent records each subagent id
    in the ledger when it starts and when it ends. Do not reuse a subagent across packets. A
    subagent that has not ended by the next checkpoint is listed as stale in the ledger.
  - Concurrency cap: at most three subagents alive at once (the 32 open-stream limit is far
    away at this size; the cap keeps the run readable).
- `CHECKPOINTS.md`: what the operator does and when (see B5).
- `project/`: the empty repository with `LEDGER.md`, `TASK.md`, `ROLES.md`, a test command, and
  the `AGENTS.md` for the project, which repeats the ROLES rules for the main agent.

### B3. Subagent pattern per stage (what we expect to see)

- Per stage: one planner (or two) to write packets, then up to three builders in parallel, then
  one reviewer after each merge.
- Expected count over two hours: 15 to 40 subagent transcripts (a rough guess; the mini test M7
  measures it). That is near the 32 open-stream budget, so the 32 limit is a real risk for this
  run, not just for a 168-hour run.

### B4. Keep-going: how the run keeps working without the proxy managing the agent

Rule (AGENTS.md, Hard rules): the proxy records and never manages the agent. The participant types
the prompt and may type only "keep going". So:

- The proxy does not prompt, nudge or restart the agent. Nothing in this design changes that.
- The main agent is told in `TASK.md` to keep working until `goalcup time-left` reports zero or
  the ledger has no open rows, and to end its turn only when it has no open work it can do. When
  it stops early, the next "keep going" resumes it, because the ledger's next-action line says
  what to do.
- The operator (a person, not the product) watches the terminal and types "keep going" when the
  agent has stopped and time is left. Checking every 10 minutes is enough for a cheap model that
  works in long turns. This is the design I recommend for the first run.
- Option B, only with owner approval: a small test-operator script outside the product that sends
  exactly "keep going" to the terminal when the screen has been idle for N minutes and
  `goalcup time-left` is above zero. This is automation of the participant's one allowed action,
  so it needs an explicit owner decision (open question 1). Do not build it before that decision.
- Time: the main agent may run `goalcup time-left` (D66, pull only). The run ends at the time
  limit; the operator then stops the harness the normal way and the run seals.

### B5. Checkpoints (all operator-side, none managed by the proxy)

- Start: commit the task pack and the empty project, tag `cp-0`. Record the `goalcup start` output.
- After every merge: the main agent commits and tags `cp-<n>` and updates the ledger. The operator
  sees the tag in git.
- Every 15 minutes: the operator writes `goalcup status` and `goalcup time-left` output to the run
  log, and copies `LEDGER.md` to the run folder with a timestamp. These are operator notes, not
  chain records.
- End: the agent stops at the time limit; the operator runs the normal finish. The run's sealed
  trace and the copies are the evidence set.

### B6. Pass criteria for a run (what "good" means)

- The sealed trace verifies, integrity valid, sealed, no unexplained loss.
- The reconcile table (A5) shows no `harness_only` lines and no `tunnelled` calls to hosts the
  harness used for model work. `wire_only` lines are each explained (retry, warm-up, helper).
- At least three stage merges, at least one builder and one reviewer subagent per stage, every
  subagent with a closing report.
- Wall-clock time of the run within 5 minutes of two hours.
- Context: the main agent's transcript passes 100K tokens at least once (this is the reason for a
  two-hour run; if it does not, the run did not test long context).

### B7. Cost estimate per run

Prices (checked this session with the skill's cached table; check the live pricing page on the
day, because the cache is dated 6 Oct 2026):

- Haiku-class model: $0.10 per million input tokens and $0.50 per million output tokens for a
  prompt of up to 100K tokens; $0.50 input and $2.50 output above 100K. Cache read and cache
  write prices are not in the cached table; read them from the live pricing page before the run.
- Codex (GPT-class, low effort) and Hermes (DeepSeek flash line): not checked in this pass. Read
  their live price pages on the day. Use the same formula below.

Formula per run: input tokens times input price, plus output tokens (including thinking) times
output price. Cache reads and writes replace the input term for cached prefixes.

Example for one two-hour Claude run (assumptions, to be replaced by M7 measurements):

- About 400 model calls across main and subagents.
- Average prompt 70K tokens (under the 100K line), so 28M input tokens; output including thinking
  about 1.5K per call, so about 0.6M output tokens.
- Without any cache benefit: 28M times $0.10 is about $2.80, plus 0.6M times $0.50 is about $0.30,
  so about $3.10. Add half again for retries and long turns: about $5. Set a per-run cap of $10.
- If prompts pass 100K, the rate jumps five times on input. The same run at 120K average is about
  $26. The 100K line is the single biggest cost lever in this design: keep the main agent
  compacted below it, and keep subagent prompts small.

Codex and Hermes runs: same shape. Expect the cost to be set by the price of the model at the
chosen effort and by how often prompts pass each provider's long-prompt tier. Free gateway model:
$0 but not a valid recorded run (owner rule: small checks only).

Mini tests are cheap by comparison (roughly 20 minutes each, Part C).

---

## Part C: mini tests (to run before the two-hour run; none run here)

- M1, Claude Code, Haiku-class, 10 minutes. One subagent spawn. Check: (a) a kept header stream
  for the response carries `request-id`; (b) the transcript `requestId` matches it; (c) the
  subagent transcript is in the chain; (d) distinct requestId equals the count of wire model
  requests with 2xx. Proves the Claude join key works through this proxy.
- M2, Codex, Luma-class at low effort, 10 minutes. Check: warm-up `generate: false` count against
  token-count events; child rollout path for a `spawn_agent` call; that each turn has one
  token-count event. Proves the Codex count rule.
- M3, Hermes 0.22 through the gateway, DeepSeek flash line, 15 minutes. Check: (a) whether any
  model call reaches the wire through the gateway with the run's trust (the proxy's record for
  the gateway's POSTs); (b) `PRAGMA table_info` for `messages`, `sessions`, `session_model_usage`
  on the copy; (c) whether a delegation call works on 0.22 (the status doc says it failed in
  0.19); (d) whether helper calls reach the wire without a `messages` row.
- M4, Hermes, same run as M3: a second provider entry in the Hermes config pointing at another
  host, one call through it. Check: the `tunneled_not_approved` line in the host log, and that no
  chain record names that host. Proves owner comment 3 on the current code.
- M5, partial bypass test (any harness, 5 minutes): make one call from the harness's own shell to
  the model host without the proxy variable set. Check: the reconcile flags `harness_only` (after
  A5 exists), and the current `cross_leg` state still says `consistent`. Proves trap A4, item 4.
- M6, recorder death test (5 minutes): kill the accept loop once at 1 minute and once at 20
  minutes into a run. Check: the restart line, `recorder_restarts` carried across a
  `goalcup continue`, and what is lost (open connections only). Answers owner comment 6 with
  evidence rather than reading.
- M7, 20-minute dry run of the task pack on the Haiku-class model: planner, two builders, one
  reviewer. Check: subagent count per 20 minutes (extrapolate to two hours), prompt size growth
  (is 100K reached?), keep-going frequency, and the cost per run from the usage lines. This is the
  measurement behind B3 and B7.

Free gateway model: use only for M5 (plumbing) and for checking the kit scripts.

---

## Part D: risks

1. Join key missing: the `request-id` header may be absent or rewritten by a gateway. Then the
   Claude reconcile falls back to counts and order, which is weaker.
2. Subagent streams: the 32 open append-only streams can be used up in a two-hour run with 15 to
   40 subagents. Past that, transcripts are lost to one gap record (status doc, open item 2). A
   168-hour run will hit this many times. Fix options are in A6, step 6.
3. Finish budget: the Hermes and subagent final snapshots share `NATIVE_FINISH_BYTES` (64 MiB).
   Long transcripts with tool output can exceed it; the overflow becomes a gap.
4. Redaction changes bytes: email and rate-limit masks change subagent copies, so any reconcile
   that compares bytes will differ from disk. The reconcile must compare identifiers (requestId,
   message ids, counts), not bytes.
5. Warm-ups and helpers: Codex warm-ups and Hermes helper calls make the wire count higher than
   the harness count. Every such case must be labelled, or the reconcile will look broken.
6. Unapproved providers: a run that uses a second provider not in the profile records nothing for
   those calls. The only evidence is a host-log counter. Multi-provider participants are exactly
   this case.
7. Hermes gateway trust: the gateway keeps the environment of whoever started it and ignores the
   run's CA variable (`connectors/hermes-cli/harness.toml`, `launch_prep` note). A gateway left
   over from an earlier run can bypass the proxy; the kit stops it before launch, but this is
   unverified on 0.22.
8. Keep-going automation: any automated "keep going" sender changes the participant rule and needs
   owner approval first.
9. Cost tail: prompts above 100K tokens cost five times more on input. A forgotten compaction can
   make a run's cost jump with no error.
10. Time: a two-hour run on one machine uses the machine for two hours; disk growth of the trace
    and the sleep lock need a check (not measured here).
11. Sandbox limits: some tests fail only here (root user, no IPv6). Do not use those failures to
    judge the design.

---

## Part E: open questions for the owner

1. Keep-going: may the test team use a scripted "keep going" sender for the two-hour and 168-hour
   runs, or does the participant type it by hand every time? The rule in AGENTS.md says the
   participant types it, so this needs a decision.
2. Multi-provider: should every model host the harness config names join the approved list
   (a change to `approved_hosts`), or should an unlisted host stop the run at start? Today it is
   tunnelled and unrecorded.
3. Subagent coverage for the 168-hour run: idle-close and reopen (new streams, more code), or
   finish-time snapshot (less code, loses data on a killed run), or raise the 32-stream budget?
4. Secret masking by value: go ahead with value masking for every header value and secret-shaped
   value (owner comment 5 says secrets must not be recorded), and what counts as secret-shaped?
5. Recording-works wording: replace the current line with the reconcile verdict once it exists, or
   keep both?
6. Cost caps: is a $10 per-run cap for the Claude run acceptable, and who approves a run that goes
   above it?
7. Price checks: may the team read the live price pages for the Codex and Hermes models on the day
   (this note did not check them)?
8. Restart cap: keep five recorder restarts per run, as a board decision? (Owner comment 6.)
9. Claude subagent transcripts: are the wider patterns (`subagents/workflows/...`) worth keeping
   as separate rows, given the 8-row limit?
10. The status doc and the code disagree about the "Recording works" hosts line (A4, item 4).
    Which one is right is a fact check, not an owner decision, but it should be settled before the
    next status update.

