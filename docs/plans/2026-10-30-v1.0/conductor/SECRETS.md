# Secrets the new environment needs (names only)

Never write a value into any file in this repository. Set these in the container environment settings before the session starts. Check each by name only: print "set" or "missing", never the value. Run `python3 keyscan.py <dir>` over anything before committing it; it needs the values in its own environment and exits 3 when none are set, so a quiet run in a shell without the secrets proves nothing.

## The 14 names

- `VERCEL_TOKEN`: `box.mjs` and the pad's `sbx.mjs` (create, run, delete sandboxes). Nothing runs without it.
- `VERCEL_TEAM_ID`: same tools; selects the team.
- `VERCEL_PROJECT_ID`: same tools; selects the project that owns the boxes.
- `CLAUDE_API_KEY`: pad rows with `key = "claude"` (Claude Code direct).
- `OPENAI_API_KEY`: pad rows with `key = "openai"` (Codex direct).
- `DEEPSEEK_API_KEY`: pad rows with `key = "deepseek"` (Hermes and other DeepSeek rows).
- `GEMINI_API_KEY`: pad rows with `key = "gemini"` (Gemini CLI).
- `AI_GATEWAY_API_KEY`: pad rows with `key = "gateway"` (Vercel AI Gateway). Must belong to the funded team.
- `DATABASE_URL`: read-only role `goalcup_trace_reader` (SELECT on the tracer entries, chunks, events, runs and heartbeats tables). Used only inside the short-lived index box (`dbfetch/`).
- `CLOUDFLARE_ACCOUNT_ID`: staging Worker deploy and `scripts/fetch-run.sh` object reads (R2).
- `CLOUDFLARE_API_TOKEN`: same two uses as the account id.
- `GOALCUP_TEST_APPROVAL_KEY`: pad staging runs (approval of the test member), read by `scripts/pad/pad`.
- `GOALCUP_TEST_MEMBER_ID`: pad staging runs (the test member), read by `scripts/pad/pad`.
- `GH_TOKEN`: conductor git pushes to `v1.0/*` branches. Never enters a box; unset in every pad box and for rows that have no test repo.

The list is `keyscan.NAMES`; `test/test_docs.sh` fails when the two differ.

## Also needed (not secrets)

- Push access to `v1.0/*` branches of `agentcommunity/goal-cup-proxy` (nothing else is pushed).
- Network access to the Vercel API. Full outbound access is needed in practice: GitHub, the sandbox API, npm and crates registries, the Worker staging host, R2, the AI Gateway host and each model host. If all traffic goes through `HTTPS_PROXY`, the sandbox patch in `handoff-head.md` is required.
- Prerequisite programs: Node 22 with npm, `python3.11` (for `padrun.sh`), `python3` 3.9 or later (standard library only; enough for `keyscan.py` and `rowinfo.py`), `git`, `gh`, `flock`, `tar`, `npx wrangler`. Check with `node --version`, `python3.11 --version`, `git --version`, `gh --version`, `command -v flock tar`.

## Rules

- Keys reach a box only through `sbx.mjs put-env` (value read inside node, written 0600). Never on a command line, never in a file in the repo.
- Unset `GH_TOKEN`, `DATABASE_URL`, `CLOUDFLARE_*` and the approval key in every pad box (`padbox.sh` does this with `env -u`).
- The conductor does not keep `~/.pad-keys/`; a session policy blocked it.
- The conductor cannot reach Postgres directly; it reaches the database only through a short-lived sandbox that holds `DATABASE_URL` at mode 0600, removed after the query, and the box is deleted (owner approved 1 November, 04:45Z).
- Two real keys (Gemini, Claude) were named at 07:24Z on 1 November as candidates for rotation; this is an open owner ask.
- Do not compile or test the product on the conductor. Builds and tests run only in sandboxes.
