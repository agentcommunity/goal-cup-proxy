# Fixtures — spike 2, codex-cli (launch), run 2026-09-03

Redacted copies of the artifacts from spike run `ws-20260903T064347Z`
(codex-cli **0.153.0**, isolated install `harnesses/node/bin/codex`, isolated
`CODEX_HOME=harnesses/home/codex`, macOS 26.5.2 arm64). Tracked thread
`01a06614-8f11-79a1-abb9-0b0f45b7f104`.
The supporting private historical record is intentionally omitted from this public OSS cut.

Every file went through `spikes/redact.py` (pattern-based) **and** a structural
scrub (`out/ws-20260903T064347Z/analysis/make_fixtures.py`) for the things the
regexes cannot see. Both logs are listed below. All files were then reviewed by
eye with a residual-content grep (`Skill roots`, `.agents/skills/<private>`,
`plan_type`, the operator's email and handle) — the only surviving hits are the
string `plan_type` inside my own scrub placeholder and the paths of the
**public** `superpowers` plugin skills the agent actually read, which are
transcript evidence.

## Files

| file | sha256 (first 16) | bytes | what it is |
|---|---|---|---|
| `rollout-main-01a06614.jsonl` | `04441b2e0d18be76` | 312,967 | the tracked thread's whole rollout, 161 records, **including the turns appended after `codex resume`** |
| `rollout-subagent-01a06615.jsonl` | `e3bd02af1960b1c7` | 75,750 | the subagent thread, 39 records; carries the parent link in `session_meta` |
| `hooks.jsonl` | `334d72982ed71e55` | 72,435 | leg 2, all 45 posts (post 1 is the operator's `/Ping` smoke test) |
| `history-excerpt.jsonl` | `900823778bbb4040` | 1,446 | all 11 `$CODEX_HOME/history.jsonl` records — the third witness, and the only place the hook-**blocked** 4,100-char prompt survives for the tracked thread |
| `wrong-cwd-01a06616-excerpt.jsonl` | `103fa336d0306cf4` | 2,310 | 2 records from the mis-launched session proving it enumerated the tracker's artifact root (`hooks.jsonl`, `leg1/capture.jsonl`, `baseline/files/auth.json`) |
| `tracker-hooks.json` | `afa41eb4c78d8e76` | 4,514 | the folder-scoped `.codex/hooks.json` the tracker wrote, **unmodified**. Full sha256 `afa41eb4c78d8e769e164cb84b87712b9c0cb83695112fd0ed546f187a8824e0` — this is the post-restructure hash quoted in the report |
| `tracker-hook-post.sh` | `5c2766a3cdea28c3` | 926 | the shim, **unmodified**. Full sha256 `5c2766a3cdea28c34b3d451ac2b6a4a380f492cf4761ac76ba215398545c55f4` |
| `probes/p1-trusted-exec.stderr.txt` | `4c873f29b54607e7` | 6,309 | P1: `codex exec` with **already-trusted** hooks and no bypass flag — the `hook: <Event>` lines are Codex's own trace |
| `probes/hooks-p1-trusted.jsonl` | `129cc77fafe445f0` | 10,424 | the 6 posts P1 produced |
| `probes/p1b-untrusted-exec.stderr.txt` | `f19aea49a5f72675` | 6,555 | P1b: same command, **untrusted** hooks. No `hook:` line, no warning, exit 0 |
| `probes/hooks-p1b-untrusted.jsonl` | (empty) | 0 | P1b produced zero posts. The empty file is the evidence |
| `probes/p2-nogit-exec.stderr.txt` | `943deb412f7ae279` | 6,517 | P2: non-git cwd, `--skip-git-repo-check --dangerously-bypass-hook-trust` |
| `probes/hooks-p2-nogit.jsonl` | `85b0b21e9ac8f401` | 10,375 | the 6 posts P2 produced — hooks **are** discovered outside a git repo |

Total 524 KB.

## Redaction logs

`spikes/redact.py` writes one log per file. **Every log is empty**: none of the
key, token, JWT, private-key or email patterns matched anything in any Codex
rollout, hook post or probe output. That is a finding in itself — the leak risk
on this harness is structural, not pattern-shaped, which is why the scrub below
exists.

## Structural scrub (what the patterns could not catch)

Recorded in `out/ws-20260903T064347Z/analysis/fixture-scrub-log.json` (git-ignored
with the rest of `out/`); **29 edits**, 151,364 characters removed:

| file | field | count | chars | removed |
|---|---|---|---|---|
| `rollout-main-01a06614.jsonl` | `payload.rate_limits` on every `token_count` event | 15 | 4,890 | the operator's ChatGPT account state: `plan_type: "pro"`, credit balance, quota `used_percent`, `resets_at`. Present on **every** turn |
| `rollout-main-01a06614.jsonl` | `response_item.message[role=developer]` | 2 | 27,255 | the `<skills_instructions>` blocks: the operator's private skill catalogue with names, descriptions and absolute paths under `/Users/team/.agents/skills` |
| `rollout-main-01a06614.jsonl` | `world_state.state.host_skills.body` | 2 | 21,012 | the same catalogue again, in a second place |
| `rollout-main-01a06614.jsonl` | `function_call[spawn_agent].arguments.message` | 1 | 264 | the parent→subagent instruction. It is a Fernet-style ciphertext (`gAAAAA…`), not plaintext; truncated to 48 chars so the fixture carries a marker rather than a blob |
| `rollout-subagent-01a06615.jsonl` | same three fields | 6 | 48,919 | the subagent rollout replays the parent's context, so it repeats all of the above |
| `history-excerpt.jsonl` | `text` | 2 | 7,960 | the 4,100-char A7 filler, truncated to 120 chars. Size only; the record's existence is the evidence |
| `wrong-cwd-01a06616-excerpt.jsonl` | `custom_tool_call_output.output` | 1 | 40,064 | kept only the first 620 chars — the file listing that names the tracker's artifacts. The rest was the repo's own documentation, which does not belong in a fixture |

Absolute home paths (`/Users/team/…`) are kept deliberately: the `CODEX_HOME`
layout and the `cwd` values in hook payloads are the thing under test.

`session_meta.base_instructions.text` (the ~12 KB stock Codex system prompt) is
kept: it is stock, and its presence in `session_meta` is a capture fact worth
having in a fixture.

## Not committed

`spikes/codex-cli/out/` is git-ignored and stays local. In particular:

- `out/ws-20260903T064347Z/leg1/capture.jsonl` (563 records, two hash chains) —
  the leg-1 capture, unedited.
- `out/ws-20260903T064347Z/baseline/files/auth.json` — the A10 baseline copied
  the isolated home's **ChatGPT OAuth token** into the artifact root, where the
  mis-launched session then listed it by name (see
  `wrong-cwd-01a06616-excerpt.jsonl`). It is not a fixture and must never become
  one; the report proposes moving A10 baselines out of the artifact root.
- `out/ws-20260903T064347Z/store-copy/` — read-only copies of the four rollout
  files, `history.jsonl`, `session_index.jsonl` and `state_5.sqlite`.
- `$CODEX_HOME/shell_snapshots/01a06614-….sh` — 591 KB dump of the operator's
  zsh functions and aliases, written by Codex per session. No secrets matched,
  but it is not a fixture.
