# Fixtures — spike 3, codex-app (attach), run 2026-09-03

Redacted copies of the artifacts from spike run
`spikes/codex-app/out/run-20260903T081322Z` (ChatGPT.app 26.825.31414 / Codex
Framework 151.0.7922.174 / embedded codex-cli 0.150.0-alpha.12.2, macOS 26.5.2
arm64). Two runs are represented: the **main run** — tracked thread
`01a06657-c53d-7201-afa1-cefdaac2edf5`, subagent
`01a06658-688f-7790-8124-1635bbceddc8`, no hooks — and **probe P3**, thread
`01a0666d-87ec-7d12-b578-6d79f921376b`, run after the hooks were trusted from the
standalone Codex CLI, on the self-updated app binary 0.151.0-alpha.7.1.
The supporting private historical record is intentionally omitted from this public OSS cut.

Every rollout went through a structural scrub (`../make_fixtures.py`) and then
`spikes/redact.py`, and was reviewed by eye. **No `redact.py` pattern matched
anything in this spike** — all five `.redactions.jsonl` logs are empty. The four
copied or generated files (`tracker-*`, `hooks.jsonl`, and the two `config-toml-*.diff`)
do not go through `redact.py` and have no log. Every
removal was structural and is listed below.

## Files

| file | sha256 | bytes | what it is |
|---|---|---|---|
| `rollout-main-01a06657.jsonl` | `eea1e2de…6da40` | 112,686 | the tracked thread, 130 records, `ordinal` 0–129. Holds all seven scripted human messages, the quit/reopen (no marker, ordinals 73→74), the model switch, the accepted 4,101-char prompt |
| `rollout-subagent-01a06658.jsonl` | `b661609b…5d95e` | 23,128 | the `count_hello_lines` subagent (nickname `Faraday`), 29 records. Two `session_meta` records: its own, then the parent's replayed verbatim. `subagent_history_start_ordinal: 14` |
| `rollout-a9-foreign-01a0665e-excerpt.jsonl` | `e69d1dee76bcfcf6df9b097a6a447b0d75bf50214763a3f0a39ac39bb77d5206` | 5,752 | the A9 parallel-session probe in a sanitized foreign workspace. **Excerpt only**: `session_meta` + the `say hello` turn (ordinals 0, 1, 7–13). Records 2–6, the injected context for an unrelated project, were never copied out |
| `hooks.jsonl` | `e3b0c442…52b855` | **0** | leg 2 of the **main run**. Empty, and that is the finding: zero hook posts in an 11-minute run. See the note below |
| `rollout-probe-01a0666d.jsonl` | `cbab881f…52318` | 47,014 | **probe P3** leg 1, 51 records. Holds the `SessionStart` turn, the blocked over-length turn (`task_started` → `turn_context` → `task_complete` with `last_agent_message: null`, **no** `role=user` record) and the `hello2.py` turn |
| `probe-hooks.jsonl` | `090d64e9…b2a91` | 15,216 | **probe P3** leg 2, all 13 posts across 6 event types, including the CLI thread's `SessionEnd`, the full 4,101-char blocked prompt, and three `Bash` `tool_input`/`tool_response` pairs |
| `config-toml-cli-trust.diff` | `dc02eea3…149be` | 3,132 | the twelve `[hooks.state…] trusted_hash` tables the CLI's `/hooks` trust wrote, `cfb6a96b…` → `534befa1…`, `$HOME`-relative. Append-only, hashes only, no secret |
| `tracker-hooks.json` | `c621a3cea2c38a21219a6102aede45fd55cece2ab48fcb4917f05f3bd8413dbf` | 4,198 | the folder-scoped `.codex/hooks.json` the tracker wrote, **unmodified**; this hash is the one quoted in the report |
| `tracker-hook-post.sh` | `5c2766a3cdea28c34b3d451ac2b6a4a380f492cf4761ac76ba215398545c55f4` | 926 | the shim, **unmodified**, byte-identical to spike 2's |
| `config-toml-trust.diff` | `6dd1241b…f43e7` | 2,144 | the witness diff of `~/.codex/config.toml` **across the main run only**, 08:14:07Z vs 08:36:41Z, paths rewritten to `$HOME`. **The file itself is not committed** — it holds the operator's MCP server commands, URLs and env blocks |

Total 214,147 bytes (plus five empty `.redactions.jsonl` logs).

## The empty `hooks.jsonl`

It is committed at zero bytes deliberately. The receiver
(`spikes/stub-receiver.py`, port 7777, `--block-over 4000`) opened this file in
append mode at **08:13:30Z** — that creation is the only surviving evidence in
the artifact root that the receiver process started, because `recv.sh` logs to
its terminal and no `recv.log` was written. The pre-run `curl` ping described in
`PROTOCOL.md` step 1 left no line, so **liveness at the moment of the run is not
proven from the artifacts**; the report treats that as a caveat and does not
lean on leg 2's silence alone. What is independently proven is the cause: the
app version under test ships no hooks review surface, `~/.codex/config.toml`
gained **zero** `hooks.state` / `trusted_hash` entries during the run, and
untrusted hooks are skipped silently (spike 2, on the CLI).

**Probe P3 closes it from the other side.** The same receiver, the same
`hooks.json` (same hash), the same workspace — but with the trust granted from
the standalone CLI's `/hooks` — produced `probe-hooks.jsonl` with 13 posts. So
the receiver, the shim and the hook file were all fine; only the trust was
missing. The two files side by side are the evidence for the tier change.

## Structural scrub

Logged to `out/run-20260903T081322Z/analysis/fixture-scrub-log.json` (git-ignored
with the rest of `out/`); 55 edits, of which 14 are probe P3's. Each placeholder states the original length,
because on this harness the length and position of an injected block are the
evidence, not its text.

| file | field / block | edits | chars removed | why |
|---|---|---|---|---|
| main, subagent, a9 | `session_meta.base_instructions.text` | 4 | 70,920 | stock Codex base instructions, identical in every rollout |
| main | `response_item/message` `role=developer` `<app-context>` (ord 2) | 1 | 46,153 | **embeds `~/.codex/memories/memory_summary.md`** — the operator's private projects, clients and preferences — plus the skill listing with absolute paths and the collaboration/apps/plugins instruction blocks |
| subagent | same block (ord 3) | 1 | 46,153 | same, replayed into the subagent |
| main | `response_item/message` `role=developer` `<skills_instructions>` (ord 76) | 1 | 22,147 | the operator's skill inventory with absolute paths |
| main | `response_item/message` `role=developer` `<model_switch>` (ord 90) | 1 | 17,685 of 17,885 | stock re-issued system prompt; first 200 chars kept because the marker is the A6 evidence |
| main, subagent | `response_item/message` `role=user` `<recommended_plugins>` | 2 | 7,070 of 2×3,735 | the operator's available connectors, and the `<environment_context>` block concatenated into the **same** `role=user` message; first 200 chars kept because the shape is the A2 evidence |
| main, subagent | `world_state.state.host_skills.body` | 3 | 66,312 | the same private skill inventory, again |
| main | `response_item/custom_tool_call_output.output` (ord 16) | 1 | 23,571 | tool output quoting the operator's local `SKILL.md` files |
| main | `event_msg/item_completed.item.{stdout,aggregated_output,formatted_output}` (ord 15) | 3 | 67,413 | three further copies of the same output in the app's UI event stream |
| main, subagent, a9, probe | `event_msg/token_count.rate_limits` | 22 | 7,172 | `plan_type`, credit balance, quota `used_percent`, window and reset time |
| a9 | `session_meta.cwd` | 1 | 34 | the foreign workspace path, replaced with the established private-project-path marker |
| a9 | `turn_context.cwd` and `turn_context.workspace_roots[]` | 2 | 68 | the same foreign workspace path in runtime context; the participant-home root remains visible |
| a9 | `session_meta.git.repository_url` | 1 | 35 | the foreign repository URL, replaced with the established private-repository-URL marker |
| probe | `<app-context>` (ord 2), `<recommended_plugins>` (ord 5), `world_state.host_skills.body`, `session_meta.base_instructions.text` | 4 | 89,941 | the same four blocks as the main run, same reasons |
| probe | `custom_tool_call_output.output` + `item.{stdout,aggregated_output,formatted_output}` (the `SKILL.md` read) | 4 | 90,984 | four copies of one tool output quoting the operator's skill files |
| `probe-hooks.jsonl` | `PostToolUse.tool_response` | 1 | 22,471 | the leg-2 copy of that same output |
| main | `function_call spawn_agent.arguments.message` | 1 | 252 of 312 | Fernet-encrypted subagent task payload; opaque in the original too, truncated for size |
| subagent | `agent_message.content[].encrypted_content` | 1 | 252 of 312 | the same payload as delivered |
| a9 | records at `ordinal` 2–6 | 1 | 62,434 | excerpt only: the injected context for an unrelated project was not copied out of `~/.codex` at all |

## Kept deliberately

- **Absolute home paths** (`/Users/team/…`), including the workspace path, the
  `~/.codex/visualizations/<date>/<thread-id>` root and the exec commands that
  read `/Users/team/.agents/skills/superpowers/*/SKILL.md`. The store layout and
  the fact that the agent read the operator's global skills unprompted on its
  first tool call are both under test.
- The A9 excerpt's `session_meta.git` block is kept with its repository URL
  scrubbed; its commit hash and branch remain. This is the evidence that the
  parallel session reported git metadata, while the tracker workspace was not a
  git repo, so **its** `session_meta` carries no `git` block at all.
- The 4,101-char over-length prompt at `ordinal` 103 in full. It is `x` padding
  behind an `OVERLENGTH-TEST-A7` marker and it is the A7 evidence.
- The `cli_version` values as written: `0.150.0-alpha.12.2` in the two tracked
  rollouts, `0.151.0-alpha.7.1` in the A9 excerpt taken seven minutes later and in
  the probe thread. The app updated itself mid-run; see the report's Surprise 1.
- In `probe-hooks.jsonl`: the whole 4,101-char blocked prompt (it is `x` padding
  behind an `OVERLENGTH-TEST-A7` marker, and it is the A7 evidence — the text
  exists in leg 2 and nowhere in leg 1), the three `Bash` `tool_input.command`
  strings, the two short `tool_response` values, and both `Stop` posts'
  `last_assistant_message` including the design question that explains the
  missing `hello2.py`.
- In `config-toml-cli-trust.diff`: every added value verbatim. They are sha256
  hashes of the tracker's own hook definitions, and the point of the fixture is
  that a reader can recompute them.

## Probe P3 leg-1 provenance

The `stub-tailer.py` process was already stopped when P3 ran, so
`rollout-probe-01a0666d.jsonl` is **not** covered by the leg-1 hash chain. It was
copied read-only out of `~/.codex/sessions/2026/09/03/` after the fact into
`out/run-20260903T081322Z/store-copy/` (sha256 `6f48df13…f5d8e`, 51 lines) and
redacted from there. A11 is not asserted for the probe, and the report says so.

## Not committed

`spikes/codex-app/out/` is git-ignored and must be deleted — see
`out/run-20260903T081322Z/CLEANUP.txt`. In particular `leg1/capture.jsonl`
(23.2 MB) is 97.7% unrelated conversations from this machine, and it was not
edited: editing it would break the hash chain that A11 verifies.
