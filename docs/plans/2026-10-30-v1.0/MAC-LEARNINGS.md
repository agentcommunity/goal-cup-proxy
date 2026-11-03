# Mac learnings (2 to 3 November, owner's Mac)

macOS 26.6.2, M4 Pro. Worktree `mac-test-v1.0` at `origin/v1.0/integration` (`6c76e26a`). Rows run with
`where = "here"`, 5 minutes, staging. Fixes are made here and proved by a micro row, not sent to the cloud.

## How to work a harness

1. Run a 5-minute row. Stop it at once (`kill -TERM` the `goalcup start --test` pid; the pad ignores Ctrl-C)
   when there are no model calls after 3 minutes.
2. Read the harness's own error log in its private home before blaming the proxy. Hermes said
   "certificate is not trusted" in `errors.log`; the record only showed handshakes closing.
3. Prove the proxy side without the harness: the live proxy, the run's `bundle.pem`, a request with no key.
4. Fix in data (pad harness config, connector) before code.

## Findings

| # | What | Where | State |
|---|---|---|---|
| 1 | Hermes 0.22.6 drops `SSL_CERT_FILE` for provider clients: `resolve_httpx_verify()` returns a system-only context (`agent/ssl_verify.py:341`). Every model call fails TLS. Not Mac-specific. 0.22.5 is not affected. | Hermes upstream | Worked around in the pad config: `providers.goalcup-ca` with `ssl_ca_cert: ${env:SSL_CERT_FILE}`. Participants on 0.22.6 are still affected. Done: a Hermes connector version note says so at start and records it (no prepare step). The version can read "unknown" on a fresh 0.22.6 home (`--version` fails before packages install), and then no note shows. |
| 2 | Hermes 0.22.6 in a fresh home installs about 64 MB of Python packages on first start (more than 6 minutes). | Hermes | Noted. Every goalcup run is a fresh home; goalcup does not install anything for the harness. |
| 3 | `hermes gateway stop` helper times out on every run; the gateway started through `osascript` outlives the run. | goalcup connector / Hermes | Open. |
| 4 | The pad's fetch ran its database query in a Vercel index box. | `scripts/pad/pad` | Fixed: with `where = "here"`, `fetch-run.sh` runs its read-only query itself. |
| 5 | Gate full failed at clippy on macOS: Linux-only imports in `mask_values.rs` and `native_safety.rs`, and the `openpty` pointer type in `cli_setup.rs`. | tests | Fixed. |
| 6 | The 21 to 27 "failures" came from running plain `cargo test`: the proxy and guard tests set process env (`GOALCUP_TEST_UPSTREAM`, `..._EXTRA_ROOTS`, `..._WRITER_STALL`) and need one process per test. Under `cargo nextest` (the gate's runner) they pass. Exit 75 is `EXIT_WRITER_STALLED` from a leaked stall setting. No product bug. | tests | Use the gate or `cargo nextest run`, never plain `cargo test`. |
| 6a | Two real test fixes: `cli_approve` Ctrl-C test now holds `ONE` before it signals (a process-wide SIGINT hit other tests); `proxy_tunnel` peek waits up to 100 ms, since macOS loopback delivers a beat after `write` returns. | tests | Fixed. |
| 6b | `pad-run` read `/proc` in 8 places (alive, kill check, status samples, tree, environ, limits): the one-hour fault steps would break on macOS. Ported with `ps` / `lsof`; Linux path unchanged. Per-pid fd limits and the daemon scan stay Linux-only ("unknown" on macOS). | `scripts/pad/pad-run` | Fixed. Gate full PASS on the Mac after the review fixes (669 Rust, 30 script, 172 pad, 3 end-to-end, 354 s). |
| 7 | Browser tabs at `http://127.0.0.1/approve?code=BCDF-GHJK` came from the test suite (`cli_approve` against a fake server), not from pad runs: the test home did not set `GOALCUP_NO_BROWSER=1`, and macOS has `open`. | tests | Fixed once for every test process: `[env] GOALCUP_NO_BROWSER` in `.cargo/config.toml`. |
| 7a | OpenClaw C3 FAIL: its model restated the planted git-remote password in `reasoning_content`; OpenClaw resends that in every request. The `url_password` shape only matches inside `user:pw@`, and the password is not an exact-value source, so the bare copies stay raw. The tool-result copies were masked. Not macOS-specific: on Linux OpenClaw never ran tools. This is the open "bare git remote password" ask. | goalcup masker | Later (owner, 3 Nov): stated loss for 1.0. Candidate fix: read remote passwords from the project's `.git/config` as exact values at launch (`launch_values`, about 30 lines and a test); learning captures mid-run is the bigger option. |
| 7b | Login rows run by hand: the pad cannot sign in. `goalcup setup` signs in inside the private home; `goalcup start --test` needs an empty folder away from `GOALCUP_HOME`; staging approval by the test key. Staging's approval link is `agentcommunity.org/goalcup/approve` (correct). | process | Not kept in the repo. |
| 7c | Amp's server refuses the pinned 0.0.1792209635 ("needs to be updated"). Ran 0.0.1793664053. | Amp | Pin to bump. |
| 7d | Codex in its private home still reads the user's `~/.agents/skills` (outside `CODEX_HOME`). Read-only, but personal skills shape the run. | Codex / goalcup isolation | Noted. |
| 8 | goalcup's CA, leaf and bundle pass macOS SecTrust (truststore, `security verify-cert`) with all 51 host subtrees. | goalcup | Proven. |
| 9 | A network drop during a run: upload waited and caught up by itself, nothing lost. | goalcup | Proven. |

## Rows

| Harness | Version | C1 | C2 | C3 | C4 | C5 | C6 | Requests | Folder |
|---|---|---|---|---|---|---|---|---|---|
| Hermes | 0.22.6 | PASS | PASS | INC (ask 4) | PASS | PASS | PASS | 22 | `pad-out/hermes-cli/direct/20261102T223803Z-pad-1f238d/` |
| Kilo | 7.8.25 | PASS | PASS (1/1) | PASS | PASS | PASS | PASS | 10 | `pad-out/kilo-cli/direct/20261102T224425Z-pad-0b66f5/` |
| Qwen | 0.26.4 | PASS | PASS (1/1) | PASS, stated loss | PASS | PASS | PASS | 15 | `pad-out/qwen-cli/direct/20261102T225035Z-pad-5e229b/` |
| Kimi Code | 2.4.1 | PASS | PASS (1/1) | PASS | PASS | PASS | PASS | 11 | `pad-out/kimi-code-cli/direct/20261102T225633Z-pad-91edfe/` |
| Crush | 0.99.0 | PASS | PASS (1/1) | PASS | PASS | PASS | PASS | 17 | `pad-out/crush-cli/direct/20261102T230213Z-*` |
| OpenCode | 1.19.5 | PASS | PASS (1/1) | PASS, stated loss | PASS | PASS | PASS | 11 | `pad-out/opencode-cli/direct/20261102T230804Z-*` |
| OpenCode 2 | 2.2.2 | PASS | PASS (1/1) | PASS, stated loss | PASS | PASS | PASS | 12 | `pad-out/opencode2-cli/direct/20261102T231414Z-*` |
| Copilot | 1.0.105 | PASS | INC (no child thread) | INC (redacts plants itself) | PASS | PASS | PASS | 13 | `pad-out/copilot-cli/direct/20261102T232021Z-pad-4a1df8/` |
| Cline | 3.0.74 | skipped by the pad: needs sign-in (tier "not verified") | | | | | | 0 | |
| Goose | 1.57.0 | PASS | PASS (1/1) | PASS | PASS | PASS | PASS | 14 | `pad-out/goose-cli/direct/` |
| Gemini CLI | 0.66.0 | PASS | PASS (1/1) | PASS | PASS | PASS | PASS | 10 | `pad-out/gemini-cli/direct/`; no approval stall on the Mac |
| OpenClaw | 2026.10.9 | PASS | INC (no child) | FAIL (finding 7a) | PASS | PASS | PASS | 29 | `pad-out/openclaw-cli/direct/20261102T235501Z-pad-494977/` |

Subscription-login runs (by hand, staging, verified on the server copy with `goalcup-verify check --server`):

| Harness | Login | Requests | Subagents | Gaps | Secrets | Usage | Verdict |
|---|---|---|---|---|---|---|---|
| Codex 0.165.0 | ChatGPT | 125, chatgpt.com | 2/2 linked | 0 | pass (12 glued-shape warnings) | gpt-6.1-sol 57 calls | PASS |
| Claude Code 2.1.312 | Claude subscription, Haiku | 30, api.anthropic.com | 2/2 linked | 0 | pass (2 glued-shape warnings) | claude-haiku-5-5 21 calls | PASS |
| Amp 0.0.1793664053 | Amp account | 201, ampcode.com | linked from Task calls by the new Amp reader; Amp does not send subagent tokens | 0 | pass | main-thread turns only | not re-run with the reader |

Claude Code on macOS keeps its login in a Keychain entry named per `CLAUDE_CONFIG_DIR` (`Claude Code-credentials-<hash>`): the sign-in in goalcup's home left the user's own entry untouched. The older note in `docs/harnesses/claude-code-cli.md` section 5 is out of date. `goalcup seal` needs a terminal or `--yes`.

Queue spend: about 0.08 USD for 8 rows. Copilot's C2 matches Linux (no subagent reader); not a Mac issue.
