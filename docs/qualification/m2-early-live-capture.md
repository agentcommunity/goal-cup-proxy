# E01: early experimental capture of both real harnesses

> **Historical (2026-10-02).** E01 used the non-shipping experiment runner with unreserved storage. Current: [m2-real-agent-progress.md](m2-real-agent-progress.md); index: [README](README.md).

> **What this is:** the E01 compatibility experiment on branch `m2/e01-live`. It extends the
> non-shipping E00 runner (`crates/goalcup-cli/examples/m2_capture_experiment.rs`) with T02's
> measured launch recipes and runs Codex CLI and Claude Code through the real capture
> components: the T13 proxy, the T04 streaming redactor, the T05 native adapter, the T03 v5
> writer and the T18 integrity reader.
>
> **What it is not:** an installed pilot, a reserved pilot, a signed anchor, an upload or a
> product CLI.
> - Storage is **experimental, unreserved quota-only accounting**: `physical_reservation=false`,
>   `reserved_bytes=0`. The quota is an admission ceiling, never a reservation. **The installed
>   pilot needs the mandatory physical reservation (T03R/T01); nothing here stands in for it.**
> - The anchor is `local_only`. No receipt, ACK, devserver or upload exists.
> - Every number below was measured on this host in the run named. None is copied from a target.

## 1. Verdict

Candidate: code `94e540e`, measured on `444185e` (docs only after the code).
- Runner `target/release/examples/m2_capture_experiment`, sha256 `495adae7…ee0ab1`.
- `goalcup-verify` sha256 `6df96af7…7f31bd89`.
- Both harness rows ran on this one binary, never rebuilt.

| Row | Result | Evidence |
|---|---|---|
| `codex_live_smoke`, Codex CLI 0.153.0, `gpt-5.6-luna` low | **PASS**: exit 0 in 32.8 s; `result.txt` exact; literal `goalcup-verify` **0** (`valid`); tool call and output inspectable in **wire+native**; retained = source | `codex-20260928T032110Z`, §7.2 |
| `claude_live_smoke`, Claude Code 2.1.283, `haiku` low | **PASS**: exit 0 in 7.0 s; `result.txt` exact; verify **0** (`valid`); **wire+native**; 6 e-mail spans redacted, nothing else | `claude-20260928T032648Z`, §7.2 |
| Codex, first attempt | **FAIL, environmental (most likely)**: no admission completed; verify **1**. Most likely the network filter's decision was not yet effective | `codex-20260928T031203Z`, §7.3 |
| Claude, first attempt | **FAIL, environmental (most likely)**: the same signature for the new destination `api.anthropic.com`; verify **1** | `claude-20260928T032205Z`, §7.3 |
| NameConstraints CA accepted (T09 review row a) | **yes** for both harnesses | §7.4 |
| Proxy credential sent from the `HTTPS_PROXY` URL on CONNECT (T09 review row b) | **yes** for both, **0 × `407`** | §7.4 |
| App-wide writes attributable to a run | **none within the §4.4 listing roots**; `config.toml` sha256 unchanged in every row. `~/Library/Keychains`, which the Claude recipe links into its `HOME`, was **not** a listing root (§7.7) | §7.7 |

**E01's go criteria are met on this candidate** for both rows:
- the output was checked externally;
- wire and native capture are real and independent;
- the local finish is truthful (`AlreadyStopped`, every tracked descendant gone, verify 0);
- no unexplained loss (0 gaps);
- no secret or credential shape in any decoded retained stream (§7.8);
- no transport corruption (0 transport failures, 0 decode gaps).

Wire/native disagreements are reported unchanged in §7.6.

## 2. What E01 adds to the runner

All additions live in `crates/goalcup-cli/examples/m2_capture_support/`; no library, Cargo,
profile, daemon or shipped-CLI file changed. The E00 interface
(`docs/qualification/m2-component-capture.md` §6) is unchanged except where stated.

| Addition | Argument / mode | Behaviour |
|---|---|---|
| Late-bound native scope (E00-R3 I1) | `--scope-from-stdout-event TYPE:FIELD`, `--scope-wait-secs N` (120); `{late_scope}` in a native source's `scope` | Workload stdout goes through a tee: every byte is written on to `{out}/workload.stdout` (or the runner's stdout with `--workload-stdio inherit`); complete lines up to 1 MiB are scanned until the first JSON object with `"type"` = TYPE and a string FIELD. That value (checked `[A-Za-z0-9_-]{1,128}`) starts a second T05 adapter over the deferred sources. Not seen within N s, or never: the deferred sources never start, and a required one is a `SourceChanged` gap (exit 1). Codex: `thread.started:thread_id` |
| Descendant stop (E00-R3 I2) | always | The workload is spawned as a **session leader** (`setsid`; its pgid is still its pid). While it runs, every 100 ms, the tracker adds each process in its session (started no earlier than it) and each child of it or of a tracked process, keyed by pid **and start time**. The stop procedure sends SIGTERM to the group and to every live tracked descendant (and to foreign groups whose leader is one), waits 10 s, then SIGKILL, 5 s. `AlreadyStopped` only when the workload exited by itself and nothing tracked survived; `Confirmed` only when everything signalled is gone; otherwise `Unresolved`. With the stdout tee, a pipe still held after all that is `Unresolved` too |
| Pre-spawn refusal (E00-R3 m7, T02 §5/§8) | exit **6** | Empty or white-space-only goal, NUL, more than 65 536 bytes, not UTF-8, or an exec-size bound above `ARG_MAX` (T02 §5's formula plus the image path, with the proxy URL and ports at their longest). Checked before the output directory exists |
| Executable pin | `--pin-sha256 HEX` | sha256 of the workload executable must match, else exit 4 before anything exists |
| Version pin | `--pin-version-output TEXT` | `EXE --version` with the scoped environment and **no proxy/CA variables**, stdin `/dev/null`, cwd `{out}`, 10 s bound; trimmed stdout must equal TEXT, else exit 4 |
| Per-run `HOME` | `--run-home none\|empty\|keychain-link`, placeholder `{home}` = `{out}/home` | `empty`: a 0700 empty directory (Codex). `keychain-link`: exactly `Library/Keychains` → the user's `~/Library/Keychains` from the password database (Claude). Listed after the run in `summary.json` `run_home_after` |
| Workspace isolation | `--require-isolated-workspace` | Refuses (exit 4) a workspace under `/tmp`, `/private/tmp`, `/var/folders`, `/private/var/folders` or `$TMPDIR`, or with any ancestor holding `.git`, `AGENTS.md`, `AGENTS.override.md`, `CLAUDE.md`, `CLAUDE.local.md`, `.claude`, `.codex` or `.agents`. The workspace path must also contain no `"`, `\` or control character (the Codex `projects` override quotes it) |
| Goal-dispatch window | `--dispatch-deadline-secs N` (0 = off) | If the proxy has intercepted no approved-host connection N s after spawn, the runner asks the workload to stop (T02 §6: neither harness exits on its own when transport fails) |
| Pre-registered measurement | `m2_capture_experiment inspect TRACE [--call-token T] [--output-token T]` | §5 |
| Side-effect evidence | `m2_capture_experiment sidefx list …` / `sidefx diff …` | Metadata only (§4.4) |
| Driver | `examples/m2_capture_support/e01_run.sh dry\|codex\|claude` | The literal procedure of one run (§4) |
| Dry-run workload | `examples/m2_capture_support/fake_harness.sh` | §6 |

`summary.json` gains `launch` (program, sha256, pins, version output, exec-size bound,
`ARG_MAX`, run-home mode, isolation scan), `late_scope`, `late_native`,
`late_native_producer`, `dispatch`, `run_home_after`, and in `workload`: `descendants_gone`,
`tracker` (every tracked descendant: pid, pgid, `comm`, first seen, own group, signalled,
alive at end) and `stdout_tee`. `startup.json` gains `launch`.

**Deviation from T02, disclosed:** T02 launched through Python `Popen` in the measuring
shell's session. The runner makes the harness a session leader with no controlling
terminal. Both recipes are headless with stdin `/dev/null`, so no terminal input is
involved; the live rows are the first measurement of the recipes this way.

## 3. Decisions taken from evidence

### 3.1 Hosts and paths

| Harness | Approved host: capture rule | Evidence | Opaque tunnels |
|---|---|---|---|
| Codex 0.153.0, ChatGPT subscription (`auth_mode chatgpt`) | `chatgpt.com:/backend-api/codex/responses:exact:openai-responses`. Covers the WebSocket upgrade `GET` (the default transport) and the HTTP `POST` fallback on the same path | 2026-09-24 probe: 13 decrypted connections, all `chatgpt.com`; 1 WebSocket upgrade on this path. `spikes/wire-pty/REPORT.md` §4(g), §5: model calls are `response.create` frames on this path, or `POST` to it without WebSockets. T02 §6: Codex falls back from WebSockets to HTTPS on transport failure | `auth.openai.com` as the auth host (token refresh; tunnelled on 2026-09-24). Everything else, e.g. `ab.chatgpt.com` (seen tunnelled in T02 §6), is not approved |
| Claude Code 2.1.283, claude.ai subscription | `api.anthropic.com:/v1/messages:exact:anthropic-messages`. The matcher ignores the query (`?beta=true`); `/v1/messages/count_tokens` and `/api/...` are relayed, not captured | 2026-09-24 probe: one `POST /v1/messages`, gzip SSE. `spikes/wire-pty/REPORT.md` §4(g): the model-call path is `/v1/messages` | `platform.claude.com` as the auth host (the bundle's OAuth `TOKEN_URL`); telemetry and every other host are not approved |

- `api.openai.com` is **not** approved for Codex. The 2026-09-24 probe intercepted it and
  recorded nothing there for Codex. A connection to it would be counted in
  `tunneled_not_approved`, which the proxy cannot attribute to a host.
- Other paths on an approved host are TLS-terminated and relayed, but not captured. That
  includes Codex's plugin catalogue, 32.8 MB in the wire-pty spike.
- Upstream roots are `webpki`, the Mozilla roots compiled into the proxy. T09 offers no
  system-roots source; this is a report item for hosts behind TLS inspection, not a problem
  here.
- No model base URL is set anywhere, and no provider variable is passed to either harness.

### 3.2 CA variables: both extend trust, so they point at the proxy CA alone

| Variable | Replace or extend? | Evidence |
|---|---|---|
| `CODEX_CA_CERTIFICATE` (Codex 0.153.0) | **extends** | (a) 2026-09-24, first Codex attempt: with the variable set to the probe CA only, the refresh went through an *undecrypted* tunnel to `auth.openai.com` and the server answered (`refresh_token_reused`). So Codex validated the real `auth.openai.com` chain with the variable set. (b) The pinned binary's `http-client/src/custom_ca.rs` strings show it loading native root certificates (counting their errors) and registering the custom bundle's certificates in a rustls root store. That they share one store is inferred from the strings; the source was not read |
| `NODE_EXTRA_CA_CERTS` (Claude 2.1.283) | **extends** | The pinned bundle logs "Appended extra certificates from NODE_EXTRA_CA_CERTS" after loading its bundled and system roots, and its embedded runtime appends the variable's certificates to the default CA list |

So no combined bundle is built. `SSL_CERT_FILE`, which replaces trust, is not set. Tool
subprocesses therefore do not trust the proxy CA. The goal needs no network tool, and Codex
tool commands have no network (`network_access: false`, T02 §3.1).

### 3.3 Where runs live

Each run's `--out` is `/Users/Shared/goalcup-m2-e01/<run>`. The root was created 0700, and
every live run (Phase B) was launched under `umask 077`. No run left a group- or world-readable entry (checked
with `find -perm`). The root was removed after relocation (§7.9).
- `$HOME` holds `.claude`, `.codex` and `.agents`, so it cannot be an ancestor of the
  workspace.
- `/tmp` and `$TMPDIR` are writable by the Codex sandbox, so capture cannot live there.
- `/Users/Shared`, `/Users` and `/` hold none of the markers (the runner's scan is in every
  `startup.json`).

Raw originals are copied afterwards into the ignored `spikes/m2-first-release/out/early/`
(`git check-ignore`: `.gitignore:12 spikes/*/out/`).

## 4. The recipes as run

The procedure is `crates/goalcup-cli/examples/m2_capture_support/e01_run.sh <mode>`, after
`. harnesses/env.sh` (main checkout). One invocation is one run, serialized. It:
1. writes the goal (§4.1);
2. takes the side-effect listing;
3. runs the runner under `/usr/bin/time -l`;
4. takes the listing again;
5. runs the literal `goalcup-verify --capture-integrity`, `inspect` and `facts`;
6. diffs the listings, attributing paths that name the run UUID, the session/thread ID or the
   run name;
7. checks the output bytes from outside (`ls`, `stat`, `shasum`, `od`; read-only).

Each run's exact argv is saved as `command.txt` in its evidence directory, and binary and
host identity as `identity.txt`.

### 4.1 Goal (both harnesses, 334 bytes, sha256 `532eb310…d348fdea`)

```text
Create a file named result.txt in the current working directory whose entire content is exactly the 13 bytes m2-capture-ok with no trailing newline. Then read result.txt back using a tool and check that its content is exactly m2-capture-ok. Then reply with the single word done. Do not ask questions and do not wait for further input.
```

It is delivered once, automatically, as the argv element after `--`. Stdin is `/dev/null`.
It is retained only as the redacted Goal stream.

### 4.2 Common runner arguments

```text
--quota-bytes 1073741824 --require-isolated-workspace --dispatch-deadline-secs 120
--workload-timeout-secs 600 --workload-stdio files
--workload-env HOME={home} --workload-env PATH=/usr/bin:/bin:/usr/sbin:/sbin
--workload-env SHELL=/bin/zsh --workload-env NO_PROXY=127.0.0.1,localhost
--workload-env no_proxy=127.0.0.1,localhost
--workload-env-inherit USER --workload-env-inherit LOGNAME --workload-env-inherit TMPDIR
```

The runner adds `HTTPS_PROXY`/`https_proxy` = `http://goalcup:<secret>@127.0.0.1:<port>`.
The secret is held in memory only. It also adds the CA variable, pointing at the per-launch,
name-constrained proxy CA. Nothing else is inherited.

### 4.3 Per harness (T02 launch-recipes §8, copied)

**Codex CLI 0.153.0**

```text
--profile-id e01-codex-cli-0.153.0-t02 --harness-version 'codex-cli 0.153.0'
--continuation 'none: codex exec one turn, approval_policy=never, workspace-write without network, stdin /dev/null'
--capture chatgpt.com:/backend-api/codex/responses:exact:openai-responses
--auth-host auth.openai.com --upstream-roots webpki
--native-source 'name=codex-rollout,root=$GOALCUP_HARNESSES/home/codex/sessions,pattern=*/*/*/rollout-*-{id}.jsonl,form=append,hint=codex-jsonl,scope={late_scope}'
--scope-from-stdout-event thread.started:thread_id --scope-wait-secs 120
--ca-env CODEX_CA_CERTIFICATE
--pin-sha256 a29d9e86eef88cbbd69f97ce8c590b1d0a287c8f77424f5eef226b883d7eaa22
--pin-version-output 'codex-cli 0.153.0'
--run-home empty --workload-env CODEX_HOME=$GOALCUP_HARNESSES/home/codex
-- $GOALCUP_HARNESSES/node/lib/node_modules/@openai/codex/node_modules/@openai/codex-darwin-arm64/vendor/aarch64-apple-darwin/bin/codex
   exec --json -m gpt-5.6-luna -c 'model_reasoning_effort="low"' -c 'approval_policy="never"'
   -c check_for_update_on_startup=false -c 'projects={ "{workspace}" = { trust_level = "trusted" } }'
   -s workspace-write --skip-git-repo-check -C {workspace} -- {goal}
```

**Claude Code 2.1.283**

```text
--profile-id e01-claude-code-2.1.283-t02 --harness-version 'claude-code 2.1.283'
--continuation 'none: claude -p one turn, acceptEdits, permission prompts denied, stdin /dev/null'
--capture api.anthropic.com:/v1/messages:exact:anthropic-messages
--auth-host platform.claude.com --upstream-roots webpki
--native-source 'name=claude-transcript,root=$GOALCUP_HARNESSES/home/claude/projects,pattern=*/{id}.jsonl,form=append,hint=claude-code-jsonl,scope={run_uuid}'
--native-source 'name=claude-subagents,root=…/projects,pattern=*/{id}/subagents/*.jsonl,form=append,hint=claude-code-jsonl,scope={run_uuid},required=false'
--native-source 'name=claude-tool-results,root=…/projects,pattern=*/{id}/tool-results/*,form=snapshot,scope={run_uuid},required=false'
--ca-env NODE_EXTRA_CA_CERTS
--pin-sha256 d8cb1e5c79684cc12a8bfc813e3a2073406921b6245744b3009be3ab5651d21e
--run-home keychain-link --workload-env CLAUDE_CONFIG_DIR=$GOALCUP_HARNESSES/home/claude
--workload-env DISABLE_AUTOUPDATER=1 --workload-env ENABLE_CLAUDEAI_MCP_SERVERS=false
--workload-env CLAUDE_CODE_SKIP_PLUGIN_MCP_SERVERS=1
-- $HOME/.local/share/claude/versions/2.1.283 -p --model haiku --effort low
   --permission-mode acceptEdits --permission-prompts none --session-id {run_uuid}
   --output-format json --strict-mcp-config --mcp-config '{"mcpServers":{}}' -- {goal}
```

- **Stop criteria:** the harness exits after its one turn. Otherwise the dispatch deadline
  (120 s with no approved-host connection) or the 600 s workload limit starts the stop
  procedure (§2).
- **Not allowed during a measured run:** later input, approval, login repair, continuation
  prompts, workspace edits and manual rescue.

### 4.4 Side-effect listing

Before and after each run, `sidefx list` records name, kind, size and mtime (**never
contents**) for:
- `~/.claude`, `~/.claude.json`, `~/Library/Caches/claude-cli-nodejs`, `~/.local/state/claude`,
  `~/.cache/claude`, `~/.local/share/claude`, `~/.config/claude`,
  `~/Library/Application Support/Claude`, `~/.codex`, `~/.agents`;
- the harness's isolated home;
- the regular files directly in `~`.

It also records the sha256 of `$GOALCUP_HARNESSES/home/codex/config.toml`. Symlinks are never
followed. This is T02 §2's method, except that T02 also polled `~/.local/state/claude/locks`
every 50 ms, and E01 does not. T02 measured that the keychain-link `HOME` writes no version
lock anywhere. Unattributed changes are host activity unless shown otherwise. The same
host churn T02 described is visible in the dry run (§6).

## 5. Pre-registered measurement (orchestrator ruling E00-I1)

`inspect` reads the chain only through the T18 reader (`scan_capture_trace`, validated
records). It reports two things.

**Per stream.** For each stream it gives the source, label, host, path, hint and wire identity;
source and retained bytes; audit spans by kind; replaced source bytes; whether a replacement
ran to the end of the captured stream; gaps and the end reason. It also totals these per
source.

**Tool-field inspectability.** This is field *location*, not decoding.
- A tool call is a JSON member `"type": K` with K one of `tool_use`, `server_tool_use`,
  `custom_tool_call`, `function_call` or `local_shell_call`.
- A tool output is one with `tool_result`, `custom_tool_call_output`, `function_call_output`
  or `local_shell_call_output`.
- The field counts as inspectable only when the retained bytes from that member to the next
  such member (at most 16 KiB) hold the expected token: `result.txt` for a call,
  `m2-capture-ok` for an output. A replaced value therefore never counts.

The verdict per field is `wire+native`, `wire`, `native` or `neither`.

**Criteria fixed before the live runs:**
- A row whose tool-call or tool-output field is `neither` fails that criterion. It triggers the
  scoped T04 refinement named in ruling E00-I1.
- A request body or message replaced from a non-secret base64url or hex value is recorded as
  a **known limitation, not a pass**, with its audit kind and extent.
- Wire/native disagreements are reported unchanged.

## 6. Phase A verification

**Tests.** `cargo test -p goalcup-cli --offline --test m2_component_capture` passes **33/33**:
the 20 E00 tests unchanged, plus 13 E01 tests.
- 9 were added in Phase A.
- 4 were added in fix round 1 after the E01 checkpoint review (R3-I1, R3-I2, R3-m1), including
  `env_probe_child`, which runs only when re-executed by its parent test.
- Fix round 1 changed the test file only; the measured runner code is untouched.

| Test | Oracle |
|---|---|
| `descendant_in_its_own_process_group_is_stopped_and_counted` | A helper in its own group after a natural exit, after an **immediate** exit (the dry-run regression), and at a deadline. It is tracked, signalled and gone by `kill(pid, 0)` = `ESRCH`; the outcome is `Confirmed`, never `AlreadyStopped` |
| `stdout_scan_binds_the_first_matching_event_and_tees_every_byte` | The first match is handed back once; a non-matching type, non-JSON and an overlong (> 1 MiB) matching line are skipped; the file is byte-identical to what was printed |
| `goal_and_launch_refusals_exit_6_before_anything_exists` | Empty, white space (U+3000 included), NUL, 65 537 B, non-UTF-8 and a 1.1 MB environment value give exit 6; no output directory and no workload. 65 536 B is accepted |
| `pins_isolation_and_run_home` | A wrong or malformed sha256 gives exit 4 with nothing created. A wrong version gives exit 4 with no chain. A temporary root and ancestor `AGENTS.md`/`.claude` are refused. The run home holds exactly `Library/Keychains` → `~/Library/Keychains` |
| `late_scope_binds_native_capture_to_the_announced_session_only` | The announced session's file is captured byte-equal; a decoy session in the same store is not; the helper stop is `Confirmed`; exit 0; `inspect` says `native` for both fields |
| `late_scope_never_announced_is_a_visible_loss` | Outcome `not_seen`, one `source_changed` gap, exit 1 |
| `dispatch_deadline_stops_a_workload_that_never_reaches_the_proxy` | Stopped by SIGTERM after the 1 s deadline |
| `inspect_locates_tool_fields_and_ignores_replaced_values` | Spaced and compact members count; a redacted output and escaped (in-string) members do not |
| `sidefx_diff_reports_changes_by_metadata_only` | Created, deleted, modified and attributable paths, a config sha256 change, symlinks not followed, no content in the listing |
| `workload_environment_is_built_from_nothing_and_stdin_is_dev_null` (+ `env_probe_child`) | The test re-executes itself with `OPENAI_BASE_URL`, `ANTHROPIC_BASE_URL`, `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `CLAUDECODE`, `CODEX_HOME` and a sentinel in its environment, and a pipe on stdin. `/usr/bin/env` as the workload sees exactly `{A, PATH}` (direct spawn) and exactly `{A, HTTPS_PROXY, X, https_proxy}` (through the runner, `X` = the proxy CA path); no name holds `BASE_URL`, `API_KEY`, `ANTHROPIC`, `OPENAI`, `CLAUDE` or `CODEX`. The workload's `/dev/stdin` is the same device as `/dev/null` |
| `term_ignoring_descendant_is_killed_after_the_grace` | A SIGTERM-ignoring helper in its own group outlives the workload. It is tracked, signalled and gone by `ESRCH`; the outcome is `Confirmed`, with confirmation ≥ 10 s after the request; the wall time lies between 10 and 15 s |
| `stdout_pipe_held_by_an_untracked_process_is_unresolved` | The workload hands its stdout descriptor to the test over a Unix socket (`SCM_RIGHTS`, in flight, never read) and exits. Nothing tracked is alive, yet the tee is `detached` and the outcome is `Unresolved`, not `AlreadyStopped` |

**Mutations.** Evidence is in `spikes/m2-first-release/out/early/driver/mutations/`; each mutation was reverted.
Every one was killed:

| Mutant | What it changed | Killed by |
|---|---|---|
| M1 | no descendant ever alive | 2 tests |
| M2 | tee drops bytes after the match | the tee test |
| M3 | a white-space goal accepted | the refusal test |
| M4 | late scope widened to `*` | the late-scope test (the decoy is captured) |
| M5 | `ARG_MAX` check doubled | the refusal test |
| M6 | inspect ignores the token | the inspect test |
| M7 | isolation ignores temporary roots | the pins test |
| M8 | session scan off | the descendant test, on the immediate-exit case |
| MF1 (fix round 1) | `.env_clear()` removed from the workload spawn | the environment test |
| MF2 | `.stdin(Stdio::null())` removed | the environment test (stdin was the parent's pipe) |
| MF3 | no SIGKILL to tracked descendants, at both `drain_all` call sites (review mutant A) | the TERM-ignoring test (`Unresolved` after 15 s; the helper was killed by the test's guard) |
| MF4 | the held-pipe rule ignored | the held-stdout test |

One more fix-round-1 variant **survived**: MF3a, which guarded only the first of the two SIGKILL
call sites. The in-loop re-signal still delivers SIGKILL, so the behaviour is intact and the
mutant is equivalent. Logs are in `spikes/m2-first-release/out/early/driver/mutations-fix1/`.

**Gate on the tree committed as `94e540e`.** Logs are in
`spikes/m2-first-release/out/early/driver/gate-phase-a/`.

The gate (01:58–02:00:56 +07) and the release runner (linked 01:57:28 +07) were produced from
the pre-commit working tree, which was then committed unchanged as `94e540e` (02:01:10 +07).
The dry run `dry-20260927T185728Z` accordingly records the same runner sha256 with
`candidate=af510be` and `status_clean=11`.

Identity check (fix round 1): a fresh release build of the example from the same sources into
a scratch target directory gave a binary of the same size (10 516 896 B). It differs from the
evidence binary in **48 bytes only**:
- 16 bytes at offset 1944–1959, the `LC_UUID`;
- 32 bytes at 10 435 360–10 435 391, inside the code signature (`LC_CODE_SIGNATURE`
  `dataoff` 10 435 200).

`crates/`, `Cargo.toml` and `Cargo.lock` are unchanged between `94e540e` and this check. So
the measured binary is the committed code.

| Command | Exit |
|---|---|
| `cargo build --offline` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo clippy --workspace --all-targets --offline -- -D warnings` | 0 |
| `cargo test -p goalcup-cli --offline` | 0 (481 passed, 0 failed, 4 ignored) |

**Fix-round-1 gate** (test file and this document only). Logs are in
`spikes/m2-first-release/out/early/driver/gate-fix1/`.

| Command | Exit |
|---|---|
| `cargo build --offline` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo clippy --workspace --all-targets --offline -- -D warnings` | 0 |
| `cargo test -p goalcup-cli --offline`, first run | 101 |
| `cargo test -p goalcup-cli --offline`, full rerun | **0** (485 passed, 0 failed, 4 ignored; `m2_component_capture` 33/33) |

The first run hit the known intermittent `finish_matrix::signed_deadline_finishes_without_cli`
`Busy` ("mid-upload") failure while a scratch release build loaded the machine. Rerun in
isolation, that test passed.

**Dry run** (`e01_run.sh dry`, run `dry-20260927T190116Z`):
- Candidate `94e540e`, clean tree. The runner binary was built just before that commit, from the same code (see the identity check above).
- Runner `target/release/examples/m2_capture_experiment`, sha256 `495adae7…ee0ab1`.
- `goalcup-verify` sha256 `6df96af7…7f31bd89`.
- Host: macOS 26.6.2 (25G83), arm64, APFS `/System/Volumes/Data`.

`fake_harness.sh` stands in for `codex exec --json`, through the controlled endpoint (no
model call). It:
- prints `thread.started`;
- appends a rollout with a tool call and its output to a *fake* `CODEX_HOME`;
- posts one Responses-shaped body through `HTTPS_PROXY` and makes one auth-host request;
- writes and reads `result.txt`;
- leaves behind a helper that ignores SIGTERM, in a group of its own, holding stdout.

| Measured | Value |
|---|---|
| Pins | sha256 `f3c161c6…fb125c` matched; `--version` = `fake-harness 0.153.0-e01` |
| Exec-size bound / `ARG_MAX` | 2 004 / 1 048 576 |
| Isolation | 0 markers over 5 ancestors; not under a temporary root |
| Late scope | `bound` 254 ms after spawn; rollout captured 478 B source = 478 B retained |
| Dispatch | first approved-host interception *observed by the runner's 250 ms poll* at 254 ms after spawn; the chain's first connection record is 31 ms after spawn |
| Proxy | 2 accepted, 1 intercepted, 1 auth-host tunnel, **0** × `407`, 0 admission failures, clean drain, no capture loss |
| Workload | exit 0 after 58 ms; 11 descendants tracked (the helper `bash` and its `sleep`s), all in their own group; SIGTERM ignored; SIGKILL after the 10 s grace; **`Confirmed`** (requested 19:01:20.478Z, confirmed 19:01:30.492Z); 0 alive at end; tee completed (103 B, matched) |
| Output check (external) | `result.txt` 13 bytes, `m2-capture-ok`, **exact** |
| Integrity (library and literal `goalcup-verify`) | **0**, `valid`, sealed, 0 gaps, 20 records, 15 224 B, `local_only`, `key_source none` |
| `inspect` | tool call `wire+native`, tool output `wire+native`; 0 audit spans; retained = source for goal (334), native (478), wire request (241) and response (241) |
| Side effects | `config.toml` sha256 unchanged (`cc20ba96…`). 4 created and 1 modified path, all attributable, all in the fake home. 6 unattributed modified paths, all in the Claude desktop app's cache, cookies and sentry scope: host churn of the kind T02 §2 describes |
| Resources | runner peak RSS 18 350 080 B; largest reaped descendant peak RSS (`RUSAGE_CHILDREN` `ru_maxrss`) 5 357 568 B; verifier 3 670 016 B in 0.44 s; runner 10.24 s wall, of which the 10 s is the helper's TERM grace |

An earlier dry run on the pre-commit tree **caught a real bug**:
- The fake forked its helper and exited within 87 ms, between two parent-chain scans.
- The helper was re-parented to launchd unseen, and the runner claimed `AlreadyStopped`
  while it lived.
- The tee also reported nothing, because the helper still held stdout.

The session-leader tracking, the pipe-held rule and the immediate-exit test case were added
for that. The leftover helper was killed by hand. The run is kept as
`spikes/m2-first-release/out/early/dryrun/dry-20260927T185142Z`.

## 7. Live rows (Phase B, 2026-09-28)

### 7.1 Procedure

- **Setup outside the measured runs.** The maintainer allowed the runner binary in the host's
  network filter. `/Users/Shared/goalcup-m2-e01` was created 0700 under `umask 077`.
- **Isolated auth readiness** (recipe environment, no proxy):
  - `codex login status`: exit 0, "Logged in using ChatGPT".
  - `claude auth status`: exit 0, `loggedIn: true`, `authMethod: claude.ai`, subscription
    `max`.
  - No login or repair was attempted.
- **Each row** is `. harnesses/env.sh; umask 077; e01_run.sh codex|claude`. Runs were
  serialized and used no variable outside the recipe. The literal argv is each run's
  `command.txt` (§4).
- **Host:** macOS 26.6.2 (25G83), Darwin 25.6.0 arm64, APFS `/System/Volumes/Data`.
- **Harness identities:**
  - Codex: native binary sha256 `a29d9e86…d7eaa22`, `--version` = `codex-cli 0.153.0`.
  - Claude: `versions/2.1.283`, sha256 `d8cb1e5c…5651d21e`.
- **Goal:** §4.1, 334 B, sha256 `532eb310…d348fdea`.

| Run (UTC start) | Measured? | Runner / literal verify exit | Result |
|---|---|---|---|
| `codex-20260928T031203Z` | yes | 1 / 1 | environmental negative (§7.3) |
| `codex-20260928T032110Z` | yes | **0 / 0** | **PASS** |
| `claude-20260928T032205Z` | yes | 1 / 1 | environmental negative (§7.3) |
| `claude-20260928T032648Z` | yes | **0 / 0** | **PASS** |

The orchestrator decided both reruns after the negatives: same candidate, no code change.
The non-measured diagnostics (§7.5) do not count toward any row.

### 7.2 The two passing rows

| | Codex CLI 0.153.0 (`codex-20260928T032110Z`) | Claude Code 2.1.283 (`claude-20260928T032648Z`) |
|---|---|---|
| Configured / observed model | `gpt-5.6-luna` low / `turn_context` `gpt-5.6-luna`, effort `low`; `model` on every wire `response.create` = `gpt-5.6-luna` | `haiku` low / `claude-haiku-4-5-20251001` in every native assistant record and both wire requests. Effort is not observable natively (T02) |
| Goal dispatch | Interception *observed by the runner's 250 ms poll* at 505 ms after spawn (the late scope was bound on the same tick); the chain's first connection record is 0.436 s after spawn. WebSocket upgrade 0.74 s after spawn; the goal-bearing `response.create` is WS message 7 | Interception observed by the 250 ms poll at 756 ms; the chain's first connection record is 0.676 s after spawn. First `POST /v1/messages` 2.03 s after spawn |
| Goal delivered once (T02 §3.3 rule, native) | primary **1**, echo 1, guard 0; on the wire, the goal is in exactly **1** client WS message | primary **1** (`parentUuid: null`, `promptSource: sdk`), echo 1 (`queue-operation`), guard 0; on the wire, in request 1 of 2 |
| Tools actually used (native, same on the wire) | 4 × `exec` (code mode): `tools.apply_patch` adds `result.txt` (14 B with a newline) → `od`/`sed` read → `truncate -s 13` + `od` → a bash byte-count check that prints `m2-capture-ok`. Each has a `custom_tool_call_output` | `Write {file_path: result.txt, content: "m2-capture-ok"}` and `Read {file_path: result.txt}`, issued in parallel in one response; results "File created successfully…" and `1\tm2-capture-ok` |
| Output (external, read-only) | `result.txt` 13 B `m2-capture-ok`, **exact** | **exact** |
| Workload exit / duration | 0 / 32.771 s (runner 33.35 s wall, `time -l`) | 0 / 7.047 s (runner 7.66 s wall). Stdout `success`, `num_turns` 3, `end_turn`, `result: done`, `permission_denials: []` |
| Descendants / termination | 6 tracked: `codex-code-mode-host`, `zsh`, `wc`, `git` and `xcodebuild` in groups of their own; one `codex` helper in the workload group. All exited by themselves → `AlreadyStopped` | 3 tracked (`git`, `xcodebuild`, `security`), all gone → `AlreadyStopped` |
| Native scope | late-bound from `thread.started` in **505 ms**; rollout captured, 65 914 B = file size, **byte-identical** to the session file (sha256 `aaabf074…fd5f1`) | pre-assigned `--session-id`; transcript 177 329 B = file size. The retained copy differs from the file only on the 2 lines holding an e-mail address (4 redaction markers). No subagent or tool-result files |
| Wire capture | the WebSocket upgrade exchange (`GET`, `101`, empty bodies); **6** client messages (68 423 B) and **448** server messages (718 884 B); 6 `response.create`, 6 `response.completed` | **2** `POST /v1/messages` (86 897 B source), both `200` `text/event-stream`, with 2 SSE bodies (10 865 B), each ending `message_stop` |
| Proxy (T13 health) | 17 accepted, **16 intercepted**, 1 not-approved tunnel, 0 auth tunnels, **0 × `407`**, 0 admission failures, 16 closed `interrupted` (no `close_notify`, routine), clean drain, no capture loss, 0 transport failures | 14 accepted, **12 intercepted**, 1 auth-host tunnel (`platform.claude.com`), 1 not-approved tunnel, **0 × `407`**, 0 admission failures, 8 closed complete and 4 interrupted, clean drain, no capture loss, 0 transport failures |
| Literal `goalcup-verify --capture-integrity` | exit **0**: `valid`, `local_only`, sealed, 1 426 records, 2 184 731 B, 0 gaps, not incomplete, no torn tail, `key_source none`, no findings; terminal `already_stopped` | exit **0**: `valid`, `local_only`, sealed, 61 records, 409 146 B, 0 gaps, the same flags; terminal `already_stopped` |
| Storage | 2 184 731 B used of a 1 GiB quota, **reserved 0**, `physical_reservation=false` | 409 146 B, reserved 0 |
| Resources | runner peak RSS 19 988 480 B; largest reaped descendant peak RSS (`RUSAGE_CHILDREN` `ru_maxrss`: one process, not the tree's sum, and not descendants the runner never reaped) 234 176 512 B; verifier 4 751 360 B in 0.01 s | runner 19 693 568 B; largest reaped descendant 288 030 720 B; verifier 4 554 752 B, under 0.01 s |
| Run home after | empty | exactly `Library/Keychains` → `~/Library/Keychains` |

Capture gaps and context limits: none recorded (0 gaps, 0 stream-less losses) in either row. No
context limit was approached. Codex's `turn.completed` reported 62 545 input tokens (51 968
cached) and 800 output tokens (357 of them reasoning).

### 7.3 Environmental negative rows (retained as measured)

| | `codex-20260928T031203Z` | `claude-20260928T032205Z` |
|---|---|---|
| Pins / scope | sha256 and `codex-cli 0.153.0` matched; late scope bound in 5 295 ms | sha256 matched; pre-assigned session |
| Proxy | 26 accepted, **24 `admission_failed`**, 0 intercepted, 0 tunnels (auth or other), 0 × `407` | 37 accepted, **35 `admission_failed`**, 0 intercepted, 0 tunnels, 0 × `407` |
| Harness symptom | stdout "Reconnecting… 2/5 … 5/5 (request timed out)"; stderr MCP fetches to `chatgpt.com` failed; the rollout recorded the goal 30 s after `task_started` (T09's admission deadline is 30 s) | `system api_error` "Connection dropped (ECONNRESET)", `isSSLError: false`, retries 1–4 of 10, the first 39 s after spawn (spawn 03:22:07.231Z, first error 03:22:46.550Z) |
| Stop | the dispatch deadline hit at 120 s → SIGTERM → `Confirmed` (no descendants) | the dispatch deadline → SIGTERM (exit 143) → `Confirmed`; 15 descendants (`security`, `sh`, `git`, helper instances), all gone |
| Verify | exit **1**: `incomplete`, sealed, 14 records (53 734 B), 2 `shutdown_deadline` gaps (2 connections unsettled at the stop) | exit **1**: `incomplete`, sealed, 18 records (98 713 B), 2 `shutdown_deadline` gaps |
| Native | 33 017 B, byte-identical to the file; the goal is the primary user message (primary 1, echo 1); no model reply | 64 356 → 64 346 B (4 e-mail spans); goal primary 1 |
| Output / tools | `result.txt` absent; no tool ran | absent; no tool ran |
| Runner wall | 131.61 s | 133.93 s |

**Diagnosis — most likely, not proven:** the network filter's decision for the runner binary
was not yet effective for a destination it had not reached before (`chatgpt.com` at 03:12Z,
`api.anthropic.com` at 03:22Z).
- It is an inference. The proxy's counters do not classify admission failures, and do not
  name the host of a failed CONNECT (§9).
- "Every admission failed, including the plain tunnels" is inferred from 0 completed tunnels
  next to 24 and 35 failures, and 0 × `407`; the failed hosts were not observed.
- Counter-evidence to a strict per-destination theory: in the Claude PASS, this binary's first
  connection to `platform.claude.com` (the auth-host tunnel) completed.
- A few minutes later the identical binary tunnelled to, and intercepted, each host (§7.5), and
  the reruns passed.
- This setup lies outside the measured run. No code or recipe changed between a negative and
  its rerun.

### 7.4 T09 review rows (per harness)

| Harness | Name-constrained per-launch CA accepted? | Proxy credential from the `HTTPS_PROXY` URL sent on CONNECT? |
|---|---|---|
| Codex 0.153.0 (`CODEX_CA_CERTIFICATE`) | **Yes.** 16 connections completed the harness-side TLS handshake with the per-launch leaf; the WebSocket upgraded and 454 messages were captured | **Yes.** 17 admissions accepted, **0 × `407`**, so the credential was present on the first CONNECT |
| Claude 2.1.283 (`NODE_EXTRA_CA_CERTS`) | **Yes.** 12 connections intercepted, 2 model exchanges `200` | **Yes.** 14 accepted, **0 × `407`** |

Both CA variables extend trust (§3.2). The live rows are consistent with that: tunnels to
not-approved and auth hosts completed while the variable pointed at the proxy CA alone. The
proxy cannot see whether the TLS inside a tunnel succeeded.

### 7.5 Non-measured diagnostics (classification only; they count toward no row)

These were ad-hoc runner invocations. **No `command.txt` or `identity.txt` was saved** for
them: their driver directories hold only `goal.txt`, `runner.stdout`, `runner.stderr` and
`out-path`.
- The claims "same binary" and "the Codex recipe plus `RUST_LOG`" rest on the session
  transcript, not on a retained argv.
- As expected, the `diag-codex-log` `recipe_digest` (`60a2c04b…`) differs from the measured
  Codex rows' (`fa0ea975…`).

| Probe (UTC) | What | Result |
|---|---|---|
| `diag-tunnel` 03:15:03 | `/usr/bin/curl https://chatgpt.com/`, a pure tunnel through the runner | 1 not-approved tunnel, TLS to the real host, HTTP 403 |
| `diag-intercept` 03:15:20 | `chatgpt.com` approved (a capture path nothing requests), curl trusting the per-launch CA | intercepted, upstream webpki TLS and the name-constrained leaf OK, HTTP/1.1, 36 ms |
| `diag-codex-log` 03:18:24 | the Codex recipe **plus `RUST_LOG`**, 45 s deadline | Codex exit 0 in 20.6 s, `result.txt` exact; 17 accepted, 16 intercepted, 1 tunnel, 0 × `407`; verify 0; captured 5 + 282 WS messages and native 57 171 B. **Not a measured row** (non-recipe variable) |
| `diag-anthropic-tunnel` 03:25:04 | curl `https://api.anthropic.com/` as a pure tunnel (and `chatgpt.com`) | 2 tunnels; HTTP 404 and 403 |
| `diag-anthropic-intercept` 03:25:25 | `api.anthropic.com` approved, curl trusting the per-launch CA | intercepted, HTTP/1.1, 40 ms |

### 7.6 Pre-registered measurement (§5) and wire/native disagreements

**Retained vs source, per source:**

| Row | Source | Streams | Source B | Retained B | Ratio | Audit kinds | Replaced to end? |
|---|---|---|---|---|---|---|---|
| Codex | goal | 1 | 334 | 334 | 1.0 | — | — |
| Codex | native | 1 | 65 914 | 65 914 | 1.0 | — | — |
| Codex | wire request / response (the WS upgrade) | 1 / 1 | 0 / 0 | 0 / 0 | — | — | — |
| Codex | `ws_client` | 6 | 68 423 | 68 423 | 1.0 | — | — |
| Codex | `ws_server` | 448 | 718 884 | 718 884 | 1.0 | — | — |
| Claude | goal | 1 | 334 | 334 | 1.0 | — | — |
| Claude | native | 1 | 177 329 | 177 319 | 0.99994 | `email` × 4 (74 source B) | no |
| Claude | `wire_request` | 2 | 86 897 | 86 892 | 0.99994 | `email` × 2 in request 1 (37 source B) | no |
| Claude | `wire_response` | 2 | 10 865 | 10 865 | 1.0 | — | — |

- No replacement started at a base64url or hex value, and none ran to the end of a body. The
  only redactions are real e-mail addresses. **The fail-closed over-redaction limitation of
  E00-I1 did not occur on this traffic.**
- The Codex reasoning items did not reach the 16 KiB `encrypted_content` case: the stateful
  WebSocket continuations carry only the new item (below).

**Tool-field inspectability:**

| Row | Tool call | Tool output | Where |
|---|---|---|---|
| Codex | **wire+native** | **wire+native** | Wire: `response.output_item.done` `custom_tool_call` in `ws_server`, and `custom_tool_call_output` in the next `ws_client` `response.create`. Native: rollout `response_item` records |
| Claude | **wire+native** | **wire+native** | Wire: SSE `tool_use` blocks with `input_json_delta` (the path and content split across deltas), and `tool_result` in request 2. Native: transcript `tool_use`/`tool_result` |

**Wire/native disagreements** (reported unchanged; semantics for T20/T21):

- **Codex.**
  - The wire has 6 model calls where the rollout has one turn.
  - **WS message 0** is a warm-up `response.create` with `generate: false`. It carries
    `additional_tools` and a developer message holding the 17 730-character base instructions
    (natively `session_meta.base_instructions`), and no user content.
  - **WS message 7** chains to the warm-up's own response (`previous_response_id` = the
    warm-up's `resp_…`). It carries **3 input items**, the counterparts of rollout lines 3, 4
    and 7:
    - the developer message (skills/apps/plugins instructions);
    - the user message with `recommended_plugins` and `environment_context`;
    - the goal.
  - **The 4 continuations** after it each carry `previous_response_id` and one
    `custom_tool_call_output` only. The WebSocket is stateful, as `spikes/wire-pty/REPORT.md`
    §5 found.
- **Claude 2.1.283.**
  - Request 2 is a **threaded continuation**: `thread: {type: continue, previous_message_id}`
    with 1 message holding the two `tool_result`s and **no `tools`**. It does not carry the
    full context. A wire-only decoder must follow the thread from request 1
    (`thread.type: create`).
  - The wire shows Write and Read as **parallel** `tool_use` blocks in one response. The
    transcript records them as two assistant records, and stdout reports `num_turns` 3.
  - The Read `tool_result` sent on the wire carries an appended harness `<system-reminder>`
    block (a token-budget line) that the native `tool_result` (transcript line 22) does not
    have. Natively, the same `<total_tokens>` text is recorded as a separate
    `total_tokens_reminder` attachment (line 24), right after that result. So the injection
    is visible in both sources, in a different shape.

### 7.7 Side effects (metadata-only listing, §4.4)

| Row | `config.toml` sha256 | Attributable changes | Isolated-home changes | Other changes (unattributed) |
|---|---|---|---|---|
| Codex PASS | unchanged `cc20ba96…` | 1: the rollout in the isolated `CODEX_HOME` | 40 entries: sessions, the SQLite state/log/goal/queue/memory/thread-history files, `models_cache.json`, `cache`, `plugins`, `shell_snapshots`, `thread-writer-locks`, `tmp` (T02 §7.3's list) | `~/.claude.json`, `~/.claude/backups` (1 created, 1 rotated) and `~/.claude/skills/synced` (3): this measuring session's own Claude Code config and account sync. `~/.codex/logs_2.sqlite-wal`: the ChatGPT app. 9 Claude desktop app paths. The harness's `HOME` was the empty per-run directory, so it could not resolve these paths |
| Claude PASS | unchanged | 3: the project slug directory, the transcript and `memory` in the isolated `CLAUDE_CONFIG_DIR` | 22 entries: `.claude.json`, `.oauth_refresh.lock.owner`, `backups`, `cache`, `plugins`, `projects`, `sessions`, `skills` (account sync) | 3 Claude desktop app paths only |
| Codex negative | unchanged | 3: rollout, shell snapshot, thread lock (SIGTERM residue, T02 §7.3) | 32 | 19 desktop app; `~/.codex` WAL plus a shell snapshot of another thread (`01a0e5fe…`, started about 90 s before the run); 1 `~/.claude` |
| Claude negative | unchanged | 4: slug directory, transcript, `memory`, `telemetry/1p_failed_events.<session>…json` | 11 | 23 desktop app paths |

No app-wide path **within the §4.4 listing roots** was attributable to any run. Nothing
under the user's app-wide directories or the isolated homes was modified by hand.

**Not covered: `~/Library/Keychains`.** The Claude recipe links it into the per-run `HOME`
(the T02 trade-off), but it was not a listing root. In the Claude PASS:
- 1 auth-host tunnel (`platform.claude.com`) completed;
- `.oauth_refresh.lock.owner` was **deleted** in the isolated `CLAUDE_CONFIG_DIR`.

Both are consistent with an OAuth token refresh, and a refresh would write the login keychain.
Whether the keychain was written was **not measured** here. T02 checked its own runs; E01 did
not repeat that.

### 7.8 Secret and credential scan

Every retained stream of all four measured rows was base64-decoded and scanned, together with
the runner's `startup.json`, `summary.json`, `health.json`, `integrity.json` and
`workload.stdout`/`.stderr`. Patterns: the proxy credential URL `goalcup:<32 hex>@`,
`Proxy-Authorization`, `Bearer <token>`, `sk-ant-…`, `sk-…`, JWTs and `refresh_token` values.
**0 hits.**

The retained Claude chain still holds identifiers that T04 does not treat as secrets:
- the organization UUID in a transcript attachment;
- account and device identifiers in the request `metadata`.

The raw chains therefore stay in the ignored `out/` directory.

### 7.9 Artifacts and cleanup

- **Raw originals** are in the git-ignored `spikes/m2-first-release/out/early/`
  (`git check-ignore`: `.gitignore:12 spikes/*/out/`):
  - `live/<run>/`: every measured and diagnostic run directory (trace, summaries, workload
    stdout/stderr, CA certificate);
  - `live/native/<run>/`: copies of the harness session files;
  - `driver/<run>/`: `command.txt`, `identity.txt`, runner and verify output with
    `time -l`, `inspect.json`, `facts.json`, `sidefx-*.json`, `output-check.txt`;
  - `driver/analysis/`: the row analysis and sanitizer scripts and their outputs;
  - `dryrun/` and `authcheck/`.
- **Copies were verified:** each move compared sha256 manifests of every file and symlink
  target before the source was removed. `/Users/Shared/goalcup-m2-e01` has been deleted.
- **Sanitized excerpts** (committed) are in `spikes/m2-first-release/fixtures/early/`:
  - `codex-native-excerpt.jsonl` (14 records);
  - `codex-wire-excerpt.jsonl` (13 WS messages);
  - `claude-native-excerpt.jsonl` (7);
  - `claude-wire-excerpt.jsonl` (2 requests, 2 SSE responses);
  - `provenance.json`, which binds each excerpt by sha256 to its source session file or
    `chain.jsonl`.
- **What the excerpts keep and drop:** instructions, tool schemas, the environment context,
  the system prompt, request metadata, attachments and all IDs are dropped; the goal, tool
  calls and outputs, and the models are kept.
- **Checks on the excerpts:** `spikes/redact.py` made 0 redactions. The needle scan against
  the real e-mail, organization, account and device values found 0 hits. There are 0 `@` and
  0 UUID-shaped strings. Every record was read.

## 8. Network preflight (before `LIVE GO`)

The host runs Little Snitch. Freshly built binaries could not reach upstream in T02 §6. The
proxy runs inside the runner process, so every upstream and tunnel connection of a harness
leaves from:

- `<worktree>/target/release/examples/m2_capture_experiment`
- sha256 `495adae7181497145e0a214358818ec96930a0b19b586d74499d642cf0ee0ab1`
- ad-hoc linker signature, identifier `m2_capture_experiment-571628671df194bc`, CDHash `cfdd2a4510b0e7804ab5eabc229680bd067ed4a0`

The harnesses themselves connect only to `127.0.0.1`.
- No firewall setting was inspected or changed by the agent; the maintainer added the allow rule (§7.1).
- Rebuilding the runner changes the hash, and may invalidate an allow rule.
- The only self-check run was the loopback dry run above.

## 9. Limits and open items

- **Tracker window.** A descendant that calls `setsid` itself *and* whose parent exits within
  one 100 ms scan is not found.
- **The pipe-held rule is the only backstop, and only with the tee.** The rule that makes a
  still-held stdout pipe `Unresolved` is now tested (fix round 1). It applies only when
  `--scope-from-stdout-event` routes stdout through the tee, as in the Codex recipe. The
  Claude recipe writes stdout straight to a file, so it has no pipe backstop for the tracker
  window.
- **Proxy health is not broken down by class.** Its counters do not split admission failures
  (client TLS vs upstream) or name tunnel hosts. The CA-acceptance row is therefore read from
  interceptions that completed with captured streams, and the credential row from the `407`
  count next to accepted admissions. This is a library limitation, reported, not worked
  around.
- **Shared isolated homes.** Both homes are shared stores (T02 §7). The native scope
  partitions by session file.
- **Personal data.** The Claude transcript carries `userEmail` and `organizationUuid`
  attachments (T02 §3.2). Raw traces stay in ignored `out/` directories. Committed excerpts
  go through `spikes/redact.py` and eye review.
- **Headless recipes only.** No TTY/TUI was measured (T02 §9).
- **Per-destination filter decisions.** The host's network filter decided per destination, so
  the first run to reach a new provider host failed (§7.3). A future qualification should
  confirm the allow rules before the measured runs, outside them.
- **`umask 077`.** The rows ran under it, as the orchestrator required. Files the harnesses
  created in the isolated homes are therefore owner-only; T02 did not specify a umask.
- **n = 1 passing run per harness,** on one loaded machine. The four measured rows and the
  diagnostics are all there is.
- **Stateful continuations.** The Claude threaded continuation (`thread.previous_message_id`)
  and the Codex stateful WebSocket both mean a wire-only reader must replay from the first
  request of a thread or connection. That is T20/T21 input, not an E01 failure.
