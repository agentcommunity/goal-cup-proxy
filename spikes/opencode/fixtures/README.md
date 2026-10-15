# Fixtures — spike 5, opencode (launch), run 2026-09-03

Redacted copies of the artifacts from spike run `run-20260903T111741Z`
(opencode **1.18.27**, isolated `harnesses/node/bin/opencode`, isolated
`XDG_{DATA,CONFIG,STATE,CACHE}_HOME` under `harnesses/home/opencode/`,
macOS 26.5.2 arm64, provider **openai** via ChatGPT subscription OAuth).
Tracked session `ses_f99007e8cffeavJ0Q8UZiiiOiT`, child session
`ses_f98ff0dadffeIz8ptSZM9TyKjp`.

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

## Redaction