# Spike 7 fixtures — Hermes Agent 0.19.0, launch mode, 2026-09-03

Source run: `spikes/hermes/out/run-20260903T122253Z` (git-ignored).
Workspace: `/Users/team/goalcup-spikes/ws-hermes-20260903T122253Z`.
Provider `openai-codex` (ChatGPT subscription), `api_mode=codex_responses`,
models `gpt-5.6-sol` → `gpt-5.6-luna` (mid-session `/model` switch).

Everything here went through `spikes/redact.py` and then an eye review.
**Every file produced 0 pattern redactions** — the empty
`*.redactions.jsonl` logs next to each output are the proof, not an omission.
That is expected: this provider's OAuth token never appears in a hook payload
(Hermes builds the client from `auth.json`, and `pre_api_request.request`
carries only `method` + `body`, never headers with credentials). It was
verified by hand afterwards: zero matches for `Authorization`, `Bearer`,
`access_token`, `refresh_token`, `chatgpt_account_id`, `id_token`, `cookie`;
the only `api_key` hits are a log line reading
`check_fn check_web_api_key returned False`, and the only `sk-` hits are
fragments of the path `.../ta` + `sk-0.log`.

## Structural scrubs applied

| What | Decision |
|---|---|
| `SOUL.md` | **Kept verbatim.** It is Hermes' stock default persona, created by Hermes itself on first run; it contains nothing personal. It is part of the system-prompt bytes, so the fixture set needs it. |
| `system-prompt-root-session.txt` | **Kept verbatim.** `sessions.system_prompt` for the root session, 10,287 bytes. Checked for identity: no name, no email, no account id, no host-specific skill list (the isolated `$HERMES_HOME/skills/` was empty, unlike OpenCode's spike-5 finding). The only absolute paths are the workspace and `$HERMES_HOME`, which every other spike's fixtures also carry. |
| `hooks-leg2.jsonl` | **Full stream, unfiltered**, all 112 posts from both processes. Includes the provider's `reasoning` items with `encrypted_content` blobs — those are the provider's own opaque reasoning envelopes, not credentials, and dropping them would misrepresent what `request_messages` actually contains. |
| `leg1-capture.jsonl` | **Full tailer output**, all 531 records including the 477 `log` records, so the running SHA-256 chain in the file verifies end to end. Verify with: chain starts at 64 zeros, each record's `prev` == the previous record's `hash`. |
| `probe-oneshot` hook stream | **Omitted.** Byte-identical in shape to `probe-z-hooks.jsonl` (same 8 posts, same order); `-z` and `--oneshot` are one argparse argument. `probes-summary.txt` carries its counts. |
| `probe-safe-mode` hook stream | **Omitted because it is empty** — zero posts is the finding. `probes-summary.txt` records `hook posts: 0` next to the new `sessions` row. |
| store copy / `auth.json` / `.env` / OAuth files | **Never copied anywhere.** `collect.sh` deletes them from its own copy; `snapshot.sh` only ever hashes non-secret files. Nothing credential-bearing exists under `out/` to redact. |

## Files

| sha256 | bytes | file | what it proves |
|---|---|---|---|
| `77e4b605a57bed8863a62c55d49e39a141412ac3967fd08718be979c1402d1f5` | 1,821,142 | `hooks-leg2.jsonl` | leg 2: 112 posts, 17 hook names, both processes, the `/goalcup.violation` A7 post, `subagent_start`/`stop`, the injected `request_messages` |
| `9df18e69ee0c1191aa0aed348f713892eb3451a281815f8b74a07cb95f645dd9` | 344,820 | `leg1-capture.jsonl` | leg 1: 531 hash-chained records, 0 chain breaks |
| `863dccc3e5d95e6cae68421c886133269a0d38623e4c2e6b394c163f562408a4` | 67,548 | `db-sessions.jsonl` | final `sessions`, incl. `parent_session_id`, `cwd`, `system_prompt` |
| `96053747071de81104e4f3eaf1780e987866e42e6051ceaa2406c84bbb6dd46f` | 59,600 | `db-messages.jsonl` | final `messages`; row 24 is the `api_content != content` injection, row 26 the 4,100-char A7 message |
| `abf3b6dec8848b5ff228540ac964469066517cfe2e90e9ee185dc1afd4357fe5` | 4,634 | `db-session_model_usage.jsonl` | the only in-DB record of the `/model` switch, plus the `title_generation` auxiliary call |
| `80614497939a12960c00b2eb1bfca69221a79999dd9455861a650b65062b5bc0` | 4,048 | `db-async_delegations.jsonl` | the delegate_task dispatch record |
| `fb5c1a095152c6f8e1bee7859887cde01a2ac1cf6a33794d8fa9b9c2a8ceacc1` | 10,287 | `system-prompt-root-session.txt` | the ONLY byte-exact copy of the system prompt on `codex_responses` |
| `2765a846e1bb371d78d3b93b403dfb0f8d1ba1a9895edb5f608367abfe81194d` | 513 | `SOUL.md` | the persona block inside that system prompt |
| `0f30e161f5c2e3096a5dc1535426a5cf4db424c7c5dbaf9fa2891514814f50b5` | 621 | `delegation-live/manifest.json` | A4 side channel |
| `37e927bce49a8dd95859798dc21d0bebc516972e17fafd17c8618d42c52cd63a` | 1,984 | `delegation-live/task-0.log` | A4 live child transcript |
| `ba4a1a4761ff7e7110c8f0b313c15c4fcee56077d25967c9bf0ccdfa5a8ca646` | 48,064 | `probe-z-hooks.jsonl` | P1: `-z` loads plugins and fires the full hook set |
| `f264d90df5f5d1aeecd0810e2e006557ed5030f0651b00bbeae9339683fccaec` | 2,336 | `probes-summary.txt` | P1/P2/P3 counts and the new `sessions` rows |
| `f4e47913511fee5cc64581300f788ef639419be69205c73bf8f3abc3e0a8a7f9` | 14,069 | `goalcup-tracker/__init__.py` | the shim source, exactly as loaded |
| `d45df1cd6ec1bc76f0602a198fcf462165dd245c4bdafc9858f0ae0479348390` | 809 | `goalcup-tracker/plugin.yaml` | its manifest |
| `30abee9c5a01b7486c5c3dfb24fd90ec45c71bf17d5da28fbc5d6349638ddd9c` | 55 | `hermes-home-config.yaml.before` | the one app-wide-shaped write, before |
| `a8ef8500125599acb5d2fcb7de0418c96b5791ad749db78302fbfcb372eb1ff2` | 110 | `hermes-home-config.yaml.after` | …and after |
| `b38cbc6657f570bac653b95592fe5274b0591fd1e3200fc868342f7081c72b1c` | 14,499 | `CHECK.txt` | the assertion output this REPORT is written from |

Total: 2.3 MB.
