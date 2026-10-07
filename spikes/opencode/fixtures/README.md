# Fixtures — spike 5, opencode (launch), run 2026-09-03

Redacted copies of the artifacts from spike run `run-20260903T111741Z`
(opencode **1.18.27**, isolated `harnesses/node/bin/opencode`, isolated
`XDG_{DATA,CONFIG,STATE,CACHE}_HOME` under `harnesses/home/opencode/`,
macOS 26.5.2 arm64, provider **openai** via ChatGPT subscription OAuth).
Tracked session `ses_f99007e8cffeavJ0Q8UZiiiOiT`, child session
`ses_f98ff0dadffeIz8ptSZM9TyKjp`.
The supporting private historical record is intentionally omitted from this public OSS cut.

Every file went through `spikes/redact.py` (pattern-based) **and** a structural
scrub (`out/run-20260903T111741Z/analysis/make_fixtures.py`) for the things the
regexes cannot see. Both logs are below. All files were then reviewed by eye with
a residual grep (`.claude/skills`, `.agents/skills`, `available_skills`, `gAAAAA`,
the operator's name and handle, the A9 session id and directory) — the only
surviving hits are the 48-character prefixes of provider-encrypted reasoning
blobs, the scrub placeholders themselves, and one path to the **public**
`superpowers/brainstorming` skill the agent actually loaded, which is transcript
evidence.

## Files

| file | sha256 (first 16) | bytes | what it is |
|---|---|---|---|
| `leg1-event-main.jsonl` | `9f0efc0b2c97339d` | 472,681 | leg 1, the tracked session's 388 `event` rows exactly as the tailer saw them, hash chain fields included. Contains the kill/resume boundary (seq 255 → 256) |
| `leg1-event-child.jsonl` | `2b29f5c55706de64` | 47,485 | leg 1, the child (subagent) session's 42 `event` rows; `aggregate_id` restarts at seq 0 |
| `db-message-part-main.jsonl` | `757633efa8160f3c` | 126,364 | the final `message` (39) + `part` (124) rows for the tracked session, from `db-dump/` |
| `db-child-session.jsonl` | `a5c57ba7d527ae67` | 10,065 | the child session's `session` row (with `parent_id`) plus its 4 messages and 13 parts. **The subagent's own prompt is a `role=user` message with no `synthetic` marker** — the A2 trap |
| `hooks.jsonl` | `364c366bfe1b166c` | 1,505,754 | leg 2, all **681** posts from the folder-scoped shim, including the two `plugin.loaded`, the 12 `chat.message` and the single `chat.message.blocked` |
| `hook-system-transform.json` | `b3b498c86babb06a` | 12,606 | one full `experimental.chat.system.transform` payload — OpenCode's stock 22,915-char system prompt for the `build` agent, with the operator's skill catalogue scrubbed (see below) |
| `hook-messages-transform.json` | `83f6accdae0a2e84` | 16,537 | one full `experimental.chat.messages.transform` payload: the whole model input for the goal turn. Note `input` is `{}` — the hook carries **no** sessionID |
| `goalcup-tracker.ts` | `05dd936634803472` | 9,822 | the folder-scoped shim, **unmodified**. Full sha256 `05dd936634803472cd3b7589abd6f67d87e48c9492ae4989693838c2f8e1b455` — identical to `config-hashes.txt`, i.e. the workspace's only file at launch |

Total 2.1 MB.

## Redaction logs

`spikes/redact.py` writes one log per file. **Every log is empty**: none of the
key, token, JWT, private-key or email patterns matched anything in any opencode
event row, message row, hook post or the shim source. As on spike 2, the leak
risk on this harness is structural, not pattern-shaped.

## Structural scrub (what the patterns could not catch)

Recorded in `out/run-20260903T111741Z/analysis/fixture-scrub-log.json`
(git-ignored with the rest of `out/`); **572 edits, 5,143,325 characters
removed**:

| where | field | why |
|---|---|---|
| every `system.transform` payload, and `chat.params` `output.options.instructions` | `<available_skills>…</available_skills>` | **13,131 of the 22,915-char system prompt** is the operator's private skill catalogue: 34 skills read from `/Users/team/.claude/skills` (19) and `/Users/team/.agents/skills` (14), with names, descriptions and absolute paths. Replaced by a placeholder carrying the length and sha256 |
| every message/part/event row and hook post | `reasoningEncryptedContent` | provider-encrypted reasoning blobs, 3–4.6 KB each, opaque. Truncated to the first 48 chars so the field's presence stays visible |
| `skill` tool output (part row, event row, `tool.execute.after`) | the skill body | 11,297 chars of a third-party skill document the agent loaded from the operator's host skills dir. The path and the fact of the load are kept; the document is not |
| the `bash` output of the truncation probe | `output`, `metadata.output` | head + tail of the `seq 1 60000` digit dump. The truncation markers, `metadata.truncated` and `metadata.outputPath` are kept — they are the evidence |
| `chat.message` post for the A7 message | `prompt` | the 4,100-char `x` filler, truncated to 120 chars. `promptLength: 4100` is the evidence |
| `hooks.jsonl` only | `body.messages` on `messages.transform`, `body.system` on `system.transform` | the whole model input / system array rides on **every** request. Replaced by `{count, bytes, sha256}` markers; one full example of each is kept as its own file above |
| `hooks.jsonl` only | `provider` on `chat.params` / `chat.headers` | the 17,183-byte provider catalogue, byte-identical on all 62 posts. Kept whole once, marked with `{id, models, bytes, sha256}` thereafter |

Absolute home paths (`/Users/team/…`) are kept deliberately: the isolated-home
layout, the workspace `directory` and the `outputPath` of the spill file are the
things under test.

OpenCode's own stock system prompt (9,784 chars before the skills block) is kept
in full: it is stock, and the fact that
`experimental.chat.system.transform` hands it over verbatim is the row-5 finding.

## Not committed

`spikes/opencode/out/` is git-ignored and stays local. In particular:

- `out/run-20260903T111741Z/store-copy/opencode.db` (+ `-wal`, `-shm`) — a
  read-only copy of the isolated database. It contains the isolated login's
  OpenAI OAuth material in the `account` / `control_account` / `credential`
  tables. `snapshot.sh` copies nothing and skips `auth.json` even from hashing;
  `collect.sh` copies the database on purpose and says so in its header. Nothing
  from `store-copy/` was used to build these fixtures — they come from
  `db-dump/`, which dumps only the 13 non-credential tables.
- `out/run-20260903T111741Z/leg1/capture.jsonl` — 2,371 records, the full leg-1
  capture across all nine sources including the A9 session's rows.
- `out/run-20260903T111741Z/sse/event.ndjson` — 1,392 SSE data lines, the
  independent second copy of the bus.
- `out/run-20260903T111741Z/hooks.jsonl` — the unscrubbed 7.0 MB leg 2.
- `out/run-20260903T111741Z/openapi.json` — the server's own `GET /doc`.
