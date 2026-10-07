# Fixtures — spike 4, claude-code-cli (launch), run 2026-09-03

Redacted copies of the artifacts from run `run-20260903T073054Z` (standalone CLI
`~/.local/bin/claude` **2.1.259**, macOS Darwin 25.5.0, launch mode with
`CLAUDE_CONFIG_DIR=harnesses/home/claude`). Session
`95addb64-9dac-4046-8ef6-8aa53fa0c7f1`, pinned with `--session-id`.
The supporting private historical record is intentionally omitted from this public OSS cut.

Every file went through a structural scrub (`out/run-20260903T073054Z/analysis/make_fixtures.py`,
git-ignored with the rest of `out/`) **and** `spikes/redact.py`, in that order, and
was then reviewed by eye. Both logs are below.

## Files

| file | sha256 (first 16) | bytes | what it is |
|---|---|---|---|
| `main-session.jsonl` | `832f5d6f97b88d18` | 157,831 | the whole main transcript, 148 records, launch → hard kill → resume → `/exit` |
| `subagent-a8ca2ea8d4a8a1673.jsonl` | `aa54a7993395d07f` | 44,302 | the one real subagent (`general-purpose`), 17 records |
| `subagent-a8ca2ea8d4a8a1673.meta.json` | `4346f85ce5e211d6` | 131 | parent link: `agentType`, `description`, `toolUseId`, `spawnDepth` |
| `hooks.jsonl` | `953811f27dc7d300` | 79,248 | leg 2, all 50 hook posts with headers and receive times, from **both** hook sources |
| `tracker-settings.json` | `0f7aa0d17c0895d7` | 7,264 | the `--settings` file (17 events, `type: http`), **unmodified** — this hash is the config hash in the report |
| `project-settings.json` | `92328c425e7ff838` | 1,039 | the folder-scoped `<ws>/.claude/settings.json` discriminator (2 events, `type: command`), **unmodified** |
| `rawbody-goal-turn.request.json` | `6af9a30728207544` | 40,507 | API request for the goal turn (`f43f8464-…`) |
| `rawbody-goal-turn.response.json` | `3445f86d0d7cbb80` | 1,445 | its response (`req_011Ceg8TuCNL5CMWq1MQVz7j`) |
| `rawbody-killed-turn-orphan.request.json` | `24ee43fd6b8c15f8` | 57,018 | the request in flight when `kill -9` landed (`9a898513-…`): complete JSON, **no response file exists** |
| `rawbody-subagent.request.json` | `6eec2760438c5f9c` | 23,006 | the subagent's first request (`62b92d07-…`): `cc_is_subagent=true`, its own system prompt, 9 tools |
| `a9-other-folder.jsonl` | `c1a58d137f0b29d8` | 33,706 | the A9 parallel `claude -p "say hello"` transcript from `/Users/team/goalcup-spikes/a9-other`, same `CLAUDE_CONFIG_DIR`, no pins — proof it ran and produced **no** hook post and **no** raw body |

Total 447 KB (this file excluded). Both `tracker-settings.json` and `project-settings.json` are byte-identical
to the files the run used; their hashes match `out/.../config-hashes.txt`.

## Redaction logs (`spikes/redact.py`, pattern pass)

One log per file; an empty log means the patterns found nothing.

- `main-session.jsonl.redactions.jsonl` — 7 (email; 1 operator address, 6 `noreply@anthropic.com`)
- `subagent-a8ca2ea8d4a8a1673.jsonl.redactions.jsonl` — 3 (email)
- `a9-other-folder.jsonl.redactions.jsonl` — 1 (email)
- `rawbody-goal-turn.request.json`, `rawbody-killed-turn-orphan.request.json`,
  `rawbody-subagent.request.json` — 3 each (email)
- `hooks.jsonl`, `tracker-settings.json`, `project-settings.json`,
  `rawbody-goal-turn.response.json`, `subagent-…meta.json` — empty

Note that `hooks.jsonl` needed **no** redaction at all: hook payloads carry
`session_id`, `cwd`, `transcript_path`, `scratchpad_dir`, tool names and prompts,
and nothing account-identifying.

## Structural scrub (what the patterns cannot see)

23 edits, logged in `out/run-20260903T073054Z/analysis/fixture-scrub-log.json`.

| file(s) | field | removed |
|---|---|---|
| `main-session.jsonl`, `a9-other-folder.jsonl` | `attachment.prompt_snapshot.systemPrompt` (2 occurrences each) | 15 blocks / 10,851 chars (main) and 14 blocks / 10,334 chars (A9) of Claude Code system prompt. Replaced by a placeholder stating block count and length, because the length and position are the evidence, not the text. |
| same | `attachment.prompt_snapshot.tools` | 14 (main) / 13 (A9) full tool JSON schemas. Tool **names are kept** in the placeholder. |
| `main-session.jsonl`, `subagent-…jsonl`, `a9-other-folder.jsonl` | `attachment.session_context.context` | the operator-email context block |
| `main-session.jsonl` | 8 × `bridge-session` records | `bridgeSessionId`, `ownerAccountUuid`, `ownerOrganizationUuid` — account and organization identifiers |
| all three request bodies | the `role: "system"` mid-conversation message (`messages[1]`) | 15,414 chars (main) / 10,427 chars (subagent): environment block + deferred-tool list + agent listing + MCP instructions + skill listing, concatenated. Placeholder states the original length. |
| all three request bodies | `metadata.user_id` | `device_id` and `account_uuid`. **`session_id` is kept** — it is the evidence that the `--session-id` pin reaches the API body. |
| every file | free-text occurrences | the claude.ai cloud-session id (`session_01Bg…`, 15 hits in the main transcript, inside `remote_session_change` attachments and the git-attribution `<system-reminder>`) and the bridge-session id |

### Eye-review notes (things deliberately **kept**)

- **Absolute home paths** (`/Users/team/goalcup-spikes/…`,
  `/Users/team/dev/PROJECTS/AgentCommunity/goal-cup/harnesses/home/claude/…`) are kept:
  the project-slug rule, the isolated store layout and the `transcript_path` in every
  hook payload are exactly what is under test.
- **`attachment.skill_listing`** is kept unscrubbed. It lists **17 skills, all of them
  bundled with the CLI** (`design`, `dataviz`, `artifact-*`, `code-review`, `run`,
  `init`, `security-review`, …) and **no** user or project skill. That is itself a
  finding: with `CLAUDE_CONFIG_DIR` pointed at the isolated dir, the operator's own
  `~/.claude` skills, plugins and marketplace entries are not loaded. Contrast spike 1,
  where the same field leaked 82 private skills and had to be removed.
- **Thinking blocks** in the request bodies already arrive as
  `{"type":"thinking","thinking":"<REDACTED>","signature":"…"}` — the harness redacts the
  text and keeps the opaque signature. Left as-is.
- **`x-anthropic-billing-header` system blocks** are kept in full. `cc_version`,
  `cc_entrypoint`, `cc_is_subagent`, `cc_prompt_id` and `cc_prev_req` are the linkage
  the verifier needs for the session evidence. The supporting private B1/B5 analysis
  is intentionally omitted from this public OSS cut.
- **`atis-latch`** values are kept; they are per-session and carry no account data.

## Not committed

`spikes/claude-code-cli/out/` is git-ignored and stays local. It holds
`leg1/capture.jsonl` (the hash-chained leg-1 capture, 206 records), all 41 raw body
files, the `baseline/` and `after/` A10 snapshots, and `analysis/`. Nothing under
`harnesses/home/claude/` is committed either; in particular the isolated dir's
`shell-snapshots/snapshot-zsh-*.sh` (139 KB of the operator's exported shell
environment) and `paste-cache/570a2f6b06f0a350.txt` (the blocked 4,100-character
paste) are **not** fixtures. `snapshot.sh` never hashes or copies any file matching
`*credentials*`, and no credential file exists anywhere under `out/`.

## Vendor-prompt trim (2026-10-04, before the public release)

The raw request bodies carried Claude Code's system prompt and tool descriptions verbatim,
which is vendor text, not ours. Following the Cursor precedent (`spikes/cursor-cli/fixtures/
README.md`, `system_prompt_tail_with_host_skill`), these were trimmed in place and the JSON
structure was kept:

- every `system[i].text` longer than 1,500 chars keeps its first 1,500 chars;
- every `tools[i].description` longer than 300 chars keeps its first 300 chars;
- each cut is followed by `<scrubbed:vendor_prompt:FIELD:NNNNchars:sha256=...>`, carrying the
  original length and the full SHA-256 of the original string;
- tool names, `input_schema` objects, `messages`, `metadata`, `betas` and every other field are
  untouched, so the tool count, the system-block count and their order stay verifiable.

The `*.redactions.jsonl` offsets still describe the original, pre-redaction, pre-trim bytes and
were not rewritten. The sha256 values in the table above are for the trimmed files. The trim
was applied to the three `rawbody-*.request.json` files.
