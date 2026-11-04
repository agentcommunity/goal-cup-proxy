# Pad results

Written by scripts/pad/table (PRD-12 section 9) from pad-rows.jsonl. Do not edit by hand.

| Harness | Tier | Version | direct | gateway | dodger | Kill row | Date |
|---|---|---|---|---|---|---|---|
| amp-cli | not verified: no key | - | - | - | - | - | - |
| claude-code-cli | not verified: no full gateway row | - | PASS | - | - | - | 2026-11-04 |
| cline-cli | not verified: needs sign-in (no provider config file or env route; the auth command puts the key in argv) | - | - | - | - | - | - |
| codex-cli | not verified: no full direct row | - | - | PASS | - | - | 2026-11-04 |
| copilot-cli | not verified: no full direct row | - | - | - | - | - | 2026-11-02 |
| crush-cli | not verified: no full direct row | - | - | - | - | - | 2026-11-02 |
| droid-cli | not verified: no key | - | - | - | - | - | - |
| gemini-cli | not verified: no full direct row | - | - | - | - | - | 2026-11-03 |
| goose-cli | not verified: no full direct row | - | - | - | - | - | 2026-11-02 |
| hermes-cli | not verified: direct C3 stated loss: shape in streamed reply: bearer, generic_secret_assignment in eea0526888253d7b2/0.163/deltas, eea0526888253d7b2/0.588/deltas | - | INCONCLUSIVE | INCONCLUSIVE | INCONCLUSIVE | - | 2026-11-03 |
| kilo-cli | not verified: no full direct row | - | - | - | - | - | 2026-11-02 |
| kimi-code-cli | not verified: no full direct row | - | - | - | - | - | 2026-11-02 |
| openclaw-cli | not verified: no full direct row | - | - | - | - | - | 2026-11-02 |
| opencode | not verified: no full direct row | - | - | - | - | - | 2026-11-02 |
| qwen-cli | not verified: no full direct row | - | - | - | - | - | 2026-11-02 |

## claude-code-cli, runs

| Started | Case | Mode | Variant | Result | C1 | C2 | C3 | C4 | C5 | C6 | Wire / harness calls | Subagents | Tokens in / out | Gaps | Why |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 2026-11-04T00:32:50+00:00 | direct | full | - | PASS | PASS | PASS | PASS | PASS | PASS | PASS | 124 / 90 | 5 | 61398 / 281724 | - | - |
| 2026-11-03T21:02:47+00:00 | direct | full | - | FAIL | PASS | INCONCLUSIVE | PASS | FAIL | INCONCLUSIVE | PASS | None / None | 0 | None / None | - | C3 PASS: stated loss: shape in streamed reply: generic_secret_assignment in ed92698b9d59dd415/0.188/deltas, ed92698b9d59dd415/0.255/deltas, ed92698b9d59dd415/0.286/deltas; C4 FAIL: server copy not fetched; C2 INCONCLUSIVE: no usage summary; C5 INCONCLUSIVE: no usage summary |

## codex-cli, runs

| Started | Case | Mode | Variant | Result | C1 | C2 | C3 | C4 | C5 | C6 | Wire / harness calls | Subagents | Tokens in / out | Gaps | Why |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 2026-11-04T00:32:50+00:00 | gateway | full | - | PASS | PASS | PASS | PASS | PASS | PASS | PASS | 321 / 317 | 7 | 609746 / 525060 | - | - |

## copilot-cli, runs

| Started | Case | Mode | Variant | Result | C1 | C2 | C3 | C4 | C5 | C6 | Wire / harness calls | Subagents | Tokens in / out | Gaps | Why |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 2026-11-02T23:20:21+00:00 | direct | micro | - | INCONCLUSIVE | PASS | INCONCLUSIVE | INCONCLUSIVE | PASS | PASS | PASS | 13 / None | 0 | 30200 / 2774 | - | C3 INCONCLUSIVE: not exercised of github_token: no MASK; C3 INCONCLUSIVE: not exercised of jwt: no MASK; C3 INCONCLUSIVE: not exercised of url_password: no MASK; C2 INCONCLUSIVE: no thread with a parent |

## crush-cli, runs

| Started | Case | Mode | Variant | Result | C1 | C2 | C3 | C4 | C5 | C6 | Wire / harness calls | Subagents | Tokens in / out | Gaps | Why |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 2026-11-02T23:02:13+00:00 | direct | micro | - | PASS | PASS | PASS | PASS | PASS | PASS | PASS | 17 / None | 1 | 19741 / 1434 | - | - |

## gemini-cli, runs

| Started | Case | Mode | Variant | Result | C1 | C2 | C3 | C4 | C5 | C6 | Wire / harness calls | Subagents | Tokens in / out | Gaps | Why |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 2026-11-03T00:02:04+00:00 | direct | micro | - | PASS | PASS | PASS | PASS | PASS | PASS | PASS | 10 / 10 | 1 | 39772 / 1303 | - | - |

## goose-cli, runs

| Started | Case | Mode | Variant | Result | C1 | C2 | C3 | C4 | C5 | C6 | Wire / harness calls | Subagents | Tokens in / out | Gaps | Why |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 2026-11-02T23:48:42+00:00 | direct | micro | - | PASS | PASS | PASS | PASS | PASS | PASS | PASS | 14 / None | 1 | 17596 / 3136 | - | - |

## hermes-cli, runs

| Started | Case | Mode | Variant | Result | C1 | C2 | C3 | C4 | C5 | C6 | Wire / harness calls | Subagents | Tokens in / out | Gaps | Why |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 2026-11-03T20:53:56+00:00 | gateway | full | - | INCONCLUSIVE | PASS | PASS | INCONCLUSIVE | PASS | PASS | PASS | 349 / None | 5 | 404089 / 472557 | - | C3 PASS: stated loss: shape in streamed reply: generic_secret_assignment, masked_key in e67c5e01586a09bf6/0.209/deltas, e67c5e01586a09bf6/0.221/deltas, e67c5e01586a09bf6/0.256/deltas, e67c5e01586a09bf6/0.312/deltas, e67c5e01586a09bf6/0.90/deltas; C3 PASS: masked upstream, before the wire: no MASK of openai_key, github_token; C3 INCONCLUSIVE: not exercised; shape only in joined streamed replies (stated loss) of google_key: no MASK; C3 INCONCLUSIVE: not exercised; shape only in joined streamed replies (stated loss) of jwt: no MASK; C3 INCONCLUSIVE: not exercised; shape only in joined streamed replies (stated loss) of private_key: no MASK; subagents shown by their task logs, not tied to calls |
| 2026-11-03T20:23:09+00:00 | dodger | full | - | INCONCLUSIVE | PASS | PASS | INCONCLUSIVE | PASS | PASS | PASS | 0 / None | 5 | None / None | pinned 403 | C3 INCONCLUSIVE: secret exercise not done: no MASK of a planted shape; subagents shown by their task logs, not tied to calls |
| 2026-11-03T20:01:17+00:00 | direct | full | - | INCONCLUSIVE | PASS | PASS | INCONCLUSIVE | PASS | PASS | PASS | 325 / None | 5 | 421668 / 442992 | - | C3 PASS: stated loss: shape in streamed reply: bearer, generic_secret_assignment in eea0526888253d7b2/0.163/deltas, eea0526888253d7b2/0.588/deltas; C3 PASS: masked upstream, before the wire: no MASK of openai_key, github_token; C3 INCONCLUSIVE: not exercised; shape only in joined streamed replies (stated loss) of google_key: no MASK; C3 INCONCLUSIVE: not exercised; shape only in joined streamed replies (stated loss) of jwt: no MASK; C3 INCONCLUSIVE: not exercised; shape only in joined streamed replies (stated loss) of private_key: no MASK; subagents shown by their task logs, not tied to calls |
| 2026-11-03T19:33:23+00:00 | gateway | full | - | INCONCLUSIVE | INCONCLUSIVE | INCONCLUSIVE | INCONCLUSIVE | INCONCLUSIVE | INCONCLUSIVE | INCONCLUSIVE | 2 / None | 0 | None / None | - | C3 INCONCLUSIVE: secret exercise not done: no MASK of a planted shape; C1 INCONCLUSIVE: provider_refused: 401 or 403 from the model host; C2 INCONCLUSIVE: provider_refused: 401 or 403 from the model host; C4 INCONCLUSIVE: provider_refused: 401 or 403 from the model host; C5 INCONCLUSIVE: provider_refused: 401 or 403 from the model host; C6 INCONCLUSIVE: provider_refused: 401 or 403 from the model host; faults INCONCLUSIVE: provider_refused: 401 or 403 from the model host |
| 2026-11-03T19:33:23+00:00 | dodger | full | - | INCONCLUSIVE | PASS | PASS | INCONCLUSIVE | PASS | PASS | PASS | 0 / None | 5 | None / None | pinned 328 | C3 INCONCLUSIVE: secret exercise not done: no MASK of a planted shape; subagents shown by their task logs, not tied to calls |
| 2026-11-03T19:33:23+00:00 | direct | full | - | INCONCLUSIVE | PASS | PASS | INCONCLUSIVE | PASS | PASS | PASS | 374 / None | 5 | 444717 / 539716 | - | C3 PASS: stated loss: shape in streamed reply: generic_secret_assignment, masked_key, url_password in ef0a922e7b6e0860a/0.1195/deltas, ef0a922e7b6e0860a/0.154/deltas, ef0a922e7b6e0860a/0.162/deltas, ef0a922e7b6e0860a/0.532/deltas, ef0a922e7b6e0860a/0.864/deltas, ef0a922e7b6e0860a/0.89/deltas; C3 PASS: masked upstream, before the wire: no MASK of openai_key, github_token; C3 INCONCLUSIVE: not exercised; shape only in joined streamed replies (stated loss) of google_key: no MASK; C3 INCONCLUSIVE: not exercised; shape only in joined streamed replies (stated loss) of jwt: no MASK; C3 INCONCLUSIVE: not exercised; shape only in joined streamed replies (stated loss) of private_key: no MASK; subagents shown by their task logs, not tied to calls |
| 2026-11-03T19:31:31+00:00 | gateway | full | - | INCONCLUSIVE | INCONCLUSIVE | INCONCLUSIVE | INCONCLUSIVE | INCONCLUSIVE | INCONCLUSIVE | INCONCLUSIVE | None / None | 0 | None / None | - | C3 INCONCLUSIVE: no scan of the local record and the run's files; C3 INCONCLUSIVE: the run certificate key is not in the known values; C3 INCONCLUSIVE: no MASK counts by rule in verify/secrets.json; C1 INCONCLUSIVE: the harness did no work (commits since start None, model requests None, tool turn none; work is a commit, or 10 model requests with a tool-using turn); C2 INCONCLUSIVE: the harness did no work (commits since start None, model requests None, tool turn none; work is a commit, or 10 model requests with a tool-using turn); C4 INCONCLUSIVE: the harness did no work (commits since start None, model requests None, tool turn none; work is a commit, or 10 model requests with a tool-using turn); C5 INCONCLUSIVE: the harness did no work (commits since start None, model requests None, tool turn none; work is a commit, or 10 model requests with a tool-using turn); C6 INCONCLUSIVE: the harness did no work (commits since start None, model requests None, tool turn none; work is a commit, or 10 model requests with a tool-using turn); faults INCONCLUSIVE: the harness did no work (commits since start None, model requests None, tool turn none; work is a commit, or 10 model requests with a tool-using turn) |
| 2026-11-03T19:31:31+00:00 | dodger | full | - | INCONCLUSIVE | INCONCLUSIVE | INCONCLUSIVE | INCONCLUSIVE | INCONCLUSIVE | INCONCLUSIVE | INCONCLUSIVE | None / None | 0 | None / None | - | C3 INCONCLUSIVE: no scan of the local record and the run's files; C3 INCONCLUSIVE: the run certificate key is not in the known values; C3 INCONCLUSIVE: no MASK counts by rule in verify/secrets.json; C1 INCONCLUSIVE: the harness did no work (commits since start None, model requests None, tool turn none; work is a commit, or 10 model requests with a tool-using turn); C2 INCONCLUSIVE: the harness did no work (commits since start None, model requests None, tool turn none; work is a commit, or 10 model requests with a tool-using turn); C4 INCONCLUSIVE: the harness did no work (commits since start None, model requests None, tool turn none; work is a commit, or 10 model requests with a tool-using turn); C5 INCONCLUSIVE: the harness did no work (commits since start None, model requests None, tool turn none; work is a commit, or 10 model requests with a tool-using turn); C6 INCONCLUSIVE: the harness did no work (commits since start None, model requests None, tool turn none; work is a commit, or 10 model requests with a tool-using turn); faults INCONCLUSIVE: the harness did no work (commits since start None, model requests None, tool turn none; work is a commit, or 10 model requests with a tool-using turn) |
| 2026-11-03T19:31:31+00:00 | direct | full | - | INCONCLUSIVE | INCONCLUSIVE | INCONCLUSIVE | INCONCLUSIVE | INCONCLUSIVE | INCONCLUSIVE | INCONCLUSIVE | None / None | 0 | None / None | - | C3 INCONCLUSIVE: no scan of the local record and the run's files; C3 INCONCLUSIVE: the run certificate key is not in the known values; C3 INCONCLUSIVE: no MASK counts by rule in verify/secrets.json; C1 INCONCLUSIVE: the harness did no work (commits since start None, model requests None, tool turn none; work is a commit, or 10 model requests with a tool-using turn); C2 INCONCLUSIVE: the harness did no work (commits since start None, model requests None, tool turn none; work is a commit, or 10 model requests with a tool-using turn); C4 INCONCLUSIVE: the harness did no work (commits since start None, model requests None, tool turn none; work is a commit, or 10 model requests with a tool-using turn); C5 INCONCLUSIVE: the harness did no work (commits since start None, model requests None, tool turn none; work is a commit, or 10 model requests with a tool-using turn); C6 INCONCLUSIVE: the harness did no work (commits since start None, model requests None, tool turn none; work is a commit, or 10 model requests with a tool-using turn); faults INCONCLUSIVE: the harness did no work (commits since start None, model requests None, tool turn none; work is a commit, or 10 model requests with a tool-using turn) |
| 2026-11-02T22:38:03+00:00 | direct | micro | - | INCONCLUSIVE | PASS | PASS | INCONCLUSIVE | PASS | PASS | PASS | 22 / None | 1 | 35497 / 8692 | - | C3 PASS: stated loss: shape in streamed reply: masked_key, url_password in ee9b5ed5c5e8eeaff/0.108/deltas, ee9b5ed5c5e8eeaff/0.140/deltas, ee9b5ed5c5e8eeaff/0.78/deltas; C3 PASS: masked upstream, before the wire: no MASK of openai_key, github_token; C3 INCONCLUSIVE: not exercised; shape only in joined streamed replies (stated loss) of google_key: no MASK; C3 INCONCLUSIVE: not exercised; shape only in joined streamed replies (stated loss) of jwt: no MASK; C3 INCONCLUSIVE: not exercised; shape only in joined streamed replies (stated loss) of private_key: no MASK; subagents shown by their task logs, not tied to calls |
| 2026-11-02T22:31:18+00:00 | direct | micro | - | FAIL | PASS | INCONCLUSIVE | INCONCLUSIVE | FAIL | INCONCLUSIVE | PASS | None / None | 0 | None / None | - | C3 PASS: stated loss: shape in streamed reply: masked_key in e9190e7222bef7d92/0.131/deltas, e9190e7222bef7d92/0.78/deltas; C3 PASS: masked upstream, before the wire: no MASK of openai_key, github_token; C3 INCONCLUSIVE: not exercised; shape only in joined streamed replies (stated loss) of google_key: no MASK; C3 INCONCLUSIVE: not exercised; shape only in joined streamed replies (stated loss) of jwt: no MASK; C3 INCONCLUSIVE: not exercised; shape only in joined streamed replies (stated loss) of private_key: no MASK; C4 FAIL: server copy not fetched; C2 INCONCLUSIVE: no usage summary; C5 INCONCLUSIVE: no usage summary |
| 2026-11-02T22:23:17+00:00 | direct | micro | - | FAIL | PASS | INCONCLUSIVE | INCONCLUSIVE | FAIL | INCONCLUSIVE | PASS | None / None | 0 | None / None | - | C3 PASS: stated loss: shape in streamed reply: masked_key in e7b4a945683ee47b9/0.90/deltas; C3 PASS: masked upstream, before the wire: no MASK of openai_key, github_token; C3 INCONCLUSIVE: not exercised; shape only in joined streamed replies (stated loss) of google_key: no MASK; C3 INCONCLUSIVE: not exercised; shape only in joined streamed replies (stated loss) of jwt: no MASK; C3 INCONCLUSIVE: not exercised; shape only in joined streamed replies (stated loss) of private_key: no MASK; C4 FAIL: server copy not fetched; C2 INCONCLUSIVE: no usage summary; C5 INCONCLUSIVE: no usage summary |
| 2026-11-02T22:10:03+00:00 | direct | micro | - | INCONCLUSIVE | INCONCLUSIVE | INCONCLUSIVE | INCONCLUSIVE | INCONCLUSIVE | INCONCLUSIVE | INCONCLUSIVE | None / None | 0 | None / None | - | C3 INCONCLUSIVE: secret exercise not done: no MASK of a planted shape; C1 INCONCLUSIVE: the harness did no work (commits since start 0, model requests 0, tool turn none; work is a commit, or 10 model requests with a tool-using turn); C2 INCONCLUSIVE: the harness did no work (commits since start 0, model requests 0, tool turn none; work is a commit, or 10 model requests with a tool-using turn); C4 INCONCLUSIVE: the harness did no work (commits since start 0, model requests 0, tool turn none; work is a commit, or 10 model requests with a tool-using turn); C5 INCONCLUSIVE: the harness did no work (commits since start 0, model requests 0, tool turn none; work is a commit, or 10 model requests with a tool-using turn); C6 INCONCLUSIVE: the harness did no work (commits since start 0, model requests 0, tool turn none; work is a commit, or 10 model requests with a tool-using turn); faults INCONCLUSIVE: the harness did no work (commits since start 0, model requests 0, tool turn none; work is a commit, or 10 model requests with a tool-using turn) |
| 2026-11-02T21:32:14+00:00 | direct | micro | - | INCONCLUSIVE | INCONCLUSIVE | INCONCLUSIVE | INCONCLUSIVE | INCONCLUSIVE | INCONCLUSIVE | INCONCLUSIVE | None / None | 0 | None / None | - | C3 INCONCLUSIVE: secret exercise not done: no MASK of a planted shape; C1 INCONCLUSIVE: the harness did no work (commits since start 0, model requests 0, tool turn none; work is a commit, or 10 model requests with a tool-using turn); C2 INCONCLUSIVE: the harness did no work (commits since start 0, model requests 0, tool turn none; work is a commit, or 10 model requests with a tool-using turn); C4 INCONCLUSIVE: the harness did no work (commits since start 0, model requests 0, tool turn none; work is a commit, or 10 model requests with a tool-using turn); C5 INCONCLUSIVE: the harness did no work (commits since start 0, model requests 0, tool turn none; work is a commit, or 10 model requests with a tool-using turn); C6 INCONCLUSIVE: the harness did no work (commits since start 0, model requests 0, tool turn none; work is a commit, or 10 model requests with a tool-using turn); faults INCONCLUSIVE: the harness did no work (commits since start 0, model requests 0, tool turn none; work is a commit, or 10 model requests with a tool-using turn) |

## hermes-cli, fault and check runs

| Started | Case | Mode | Variant | Result | C1 | C2 | C3 | C4 | C5 | C6 | Wire / harness calls | Subagents | Tokens in / out | Gaps | Why |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 2026-11-03T20:54:50+00:00 | direct | micro | exec h2-parallel.sh@1 exec long-tunnel.sh@2 (h2-parallel.sh PASS) (long-tunnel.sh PASS) | INCONCLUSIVE | PASS | INCONCLUSIVE | INCONCLUSIVE | INCONCLUSIVE | PASS | PASS | 7 / None | 0 | 12751 / 1890 | - | C3 INCONCLUSIVE: not exercised of openai_key: no MASK; C3 INCONCLUSIVE: not exercised of github_token: no MASK; C3 INCONCLUSIVE: not exercised of google_key: no MASK; C3 INCONCLUSIVE: not exercised of jwt: no MASK; C3 INCONCLUSIVE: not exercised of private_key: no MASK; C4 INCONCLUSIVE: server off: local pass at best; no local check failed (keep from the settings); C2 INCONCLUSIVE: no thread with a parent |
| 2026-11-03T20:49:44+00:00 | direct | micro | exec h2-parallel.sh@1 exec long-tunnel.sh@2 (h2-parallel.sh pending) (long-tunnel.sh pending) | INCONCLUSIVE | PASS | INCONCLUSIVE | INCONCLUSIVE | INCONCLUSIVE | PASS | PASS | 8 / None | 0 | 13529 / 1357 | - | C3 PASS: stated loss: shape in streamed reply: masked_key in efc7dcce0dc23b026/0.87/deltas, efc7dcce0dc23b026/0.93/deltas; C3 PASS: masked upstream, before the wire: no MASK of openai_key, github_token; C3 INCONCLUSIVE: not exercised; shape only in joined streamed replies (stated loss) of google_key: no MASK; C3 INCONCLUSIVE: not exercised; shape only in joined streamed replies (stated loss) of jwt: no MASK; C3 INCONCLUSIVE: not exercised; shape only in joined streamed replies (stated loss) of private_key: no MASK; C4 INCONCLUSIVE: server off: local pass at best; no local check failed (keep from the settings); C2 INCONCLUSIVE: no thread with a parent |
| 2026-11-03T20:44:34+00:00 | direct | micro | exec h2-parallel.sh@1 exec long-tunnel.sh@2 (h2-parallel.sh pending) (long-tunnel.sh pending) | INCONCLUSIVE | PASS | INCONCLUSIVE | INCONCLUSIVE | INCONCLUSIVE | PASS | PASS | 6 / None | 0 | 13008 / 948 | - | C3 PASS: stated loss: shape in streamed reply: masked_key in e5c0084991b4f7970/0.87/deltas; C3 PASS: masked upstream, before the wire: no MASK of openai_key, github_token; C3 INCONCLUSIVE: not exercised; shape only in joined streamed replies (stated loss) of google_key: no MASK; C3 INCONCLUSIVE: not exercised; shape only in joined streamed replies (stated loss) of jwt: no MASK; C3 INCONCLUSIVE: not exercised; shape only in joined streamed replies (stated loss) of private_key: no MASK; C4 INCONCLUSIVE: server off: local pass at best; no local check failed (keep from the settings); C2 INCONCLUSIVE: no thread with a parent |

## kilo-cli, runs

| Started | Case | Mode | Variant | Result | C1 | C2 | C3 | C4 | C5 | C6 | Wire / harness calls | Subagents | Tokens in / out | Gaps | Why |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 2026-11-02T22:44:25+00:00 | direct | micro | - | PASS | PASS | PASS | PASS | PASS | PASS | PASS | 10 / None | 1 | 26502 / 1992 | - | - |

## kimi-code-cli, runs

| Started | Case | Mode | Variant | Result | C1 | C2 | C3 | C4 | C5 | C6 | Wire / harness calls | Subagents | Tokens in / out | Gaps | Why |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 2026-11-02T22:56:33+00:00 | direct | micro | - | PASS | PASS | PASS | PASS | PASS | PASS | PASS | 11 / 11 | 1 | 25059 / 2629 | - | - |

## openclaw-cli, runs

| Started | Case | Mode | Variant | Result | C1 | C2 | C3 | C4 | C5 | C6 | Wire / harness calls | Subagents | Tokens in / out | Gaps | Why |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 2026-11-02T23:55:01+00:00 | direct | micro | - | FAIL | PASS | INCONCLUSIVE | FAIL | PASS | PASS | PASS | 29 / None | 0 | 88971 / 16047 | source_changed 1 | C3 FAIL: 26 hits in the local record and the run's files; C3 FAIL: secrets fail in the verifier report: hits: 23 known, 3 shape, 0 glued_shape; 0 MASK spans over 16 KiB; C3 FAIL: hits of openai_key: no MASK; C3 FAIL: hits of github_token: no MASK; C3 FAIL: hits of google_key: no MASK; C3 FAIL: hits of jwt: no MASK; C2 INCONCLUSIVE: no thread with a parent; faults FAIL: named loss other than the expected gaps: source_changed |

## opencode, runs

| Started | Case | Mode | Variant | Result | C1 | C2 | C3 | C4 | C5 | C6 | Wire / harness calls | Subagents | Tokens in / out | Gaps | Why |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 2026-11-02T23:14:14+00:00 | opencode2-cli direct | micro | - | PASS | PASS | PASS | PASS | PASS | PASS | PASS | 12 / None | 1 | 12336 / 2636 | - | - |
| 2026-11-02T23:08:04+00:00 | opencode-cli direct | micro | - | PASS | PASS | PASS | PASS | PASS | PASS | PASS | 11 / None | 1 | 15753 / 3317 | - | - |

## qwen-cli, runs

| Started | Case | Mode | Variant | Result | C1 | C2 | C3 | C4 | C5 | C6 | Wire / harness calls | Subagents | Tokens in / out | Gaps | Why |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 2026-11-02T22:50:35+00:00 | direct | micro | - | PASS | PASS | PASS | PASS | PASS | PASS | PASS | 15 / 13 | 1 | 69291 / 4383 | - | - |
