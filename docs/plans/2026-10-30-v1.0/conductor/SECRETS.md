# Secrets the new environment needs (names only)

Never write a value into any file in this repository. Set these in the container environment settings before the session starts. Run `tools/keyscan.py <dir>` over anything before committing it (0 hits required).

| Name | Read by |
|---|---|
| `CLAUDE_API_KEY` | Pad `key = "claude"` rows (Claude Code direct); `scrub.py`, `keyscan.py` |
| `OPENAI_API_KEY` | Pad `key = "openai"` rows (Codex direct); `scrub.py`, `keyscan.py` |
| `DEEPSEEK_API_KEY` | Pad `key = "deepseek"` rows (Hermes and other DeepSeek rows) |
| `GEMINI_API_KEY` | Pad `key = "gemini"` rows (Gemini CLI) |
| `AI_GATEWAY_API_KEY` | Pad `key = "gateway"` rows (Vercel AI Gateway); `scrub.py`, `keyscan.py`. Must belong to the funded team |
| `VERCEL_TOKEN`, `VERCEL_TEAM_ID`, `VERCEL_PROJECT_ID` | `tools/box.mjs` and the pad's `sbx.mjs` (create, run, delete sandboxes). Nothing runs without them |
| `CLOUDFLARE_API_TOKEN`, `CLOUDFLARE_ACCOUNT_ID` | Staging Worker deploy, `fetch-run` object reads (R2) on the conductor; `scrub.py`, `keyscan.py` |
| `DATABASE_URL` | Read-only role `goalcup_trace_reader` (SELECT on `goal_cup_tracer` entries, chunks, events, runs, heartbeats). Used only inside the short-lived index box (`dbfetch-idx.sh`); `scrub.py`, `keyscan.py` |
| `GOALCUP_TEST_APPROVAL_KEY`, `GOALCUP_TEST_MEMBER_ID` | Pad staging runs (approval of the test member); `keyscan.py` |
| `GH_TOKEN` | Conductor git and pushes to `v1.0/*` branches. Never enters a box; unset in every pad box and for rows that have no test repo |

## Rules

- Keys reach a box only through `sbx.mjs put-env` (value read inside node, written 0600). Never on a command line, never in a file in the repo.
- Unset `GH_TOKEN`, `DATABASE_URL`, `CLOUDFLARE_*` and the approval key in every pad box (`padbox.sh` does this with `env -u`).
- The conductor does not keep `~/.pad-keys/`; a session policy blocked it.
- Two real keys (Gemini, Claude) were named at 07:24Z as candidates for rotation; this is an open owner ask.

## Network

Full outbound access is needed: GitHub, the sandbox API, npm and crates registries, the Worker staging host, R2, the AI Gateway host and each model host. In the old container all traffic went through `HTTPS_PROXY`, which is why the sandbox SDK needs the patch in TOOLS.md. The conductor cannot reach Postgres directly; it reaches the database only through a short-lived sandbox that holds `DATABASE_URL` at mode 0600, removed after the query, and the box is deleted (owner approved 1 November, 04:45Z).
