# Goalcup 1.0 test coverage (3 November)

What is tested, how, and where; then what is not. Linux rows ran on earlier builds (B0 to H6) in Vercel
Sandbox; Mac rows ran on the owner's Mac (macOS 26.6, M4 Pro) on `mac-test/v1.0`. Evidence:
[ACCEPTANCE.md](ACCEPTANCE.md) (Linux), [MAC-LEARNINGS.md](MAC-LEARNINGS.md) (Mac). Nothing has run on a
frozen head yet: every release row is still owed.

## How things are tested

| Kind | What it is | Where it ran |
|---|---|---|
| Gate full | fmt, clippy `-D warnings`, nextest (669 Rust tests), 30 script tests, 172 pad tests, 3 end-to-end tests, release build | Linux (cloud boxes, every train); macOS (owner's Mac, after the review fixes) |
| Pad mini row | `scripts/pad/pad`, 5 to 10 minutes, smoke prompt, one subagent asked for, key from `~/.pad-keys`, staging, checks C1 to C6, then fetch and server compare | Linux (B0, H2); macOS (12 harnesses) |
| Pad core row | 20 minutes, full prompt, `full` mode, direct / gateway / dodger cases | Linux only (f7048ed8, H5) |
| Pad Paynest row | 15 minutes, payments prompt asking for many subagents | Linux only (H5, H6) |
| One-hour run | `a2` / `a3` / `a5`: kills at 10 and 30 min, watchdog kill and `goalcup recover` at 45, keep going | Linux; best result INCONCLUSIVE |
| One-hour from scratch | release archive through `install.sh`, `goalcup setup`, `goalcup start --test` approved on agentcommunity.org, Paynest prompt, recorder kill at 20, watchdog kill and `goalcup recover` at 40, seal, server copy checked | macOS (Hermes 0.22.6, DeepSeek): INCONCLUSIVE, see MAC-LEARNINGS |
| Login row | by hand: `goalcup setup` sign-in in the private home, `goalcup start --test`, staging test approval, seal, `goalcup-verify check --server` on the fetched copy | macOS (Codex, Claude Code, Amp) |
| Replay | the verifier re-run on recorded rows after a reader change | Linux records (subagent readers), macOS record (Amp reader) |

## Harnesses

| Harness | Linux | macOS | Not tested |
|---|---|---|---|
| Claude Code | core rows PASS (key: direct, gateway, dodger); one-hour INC | login row PASS (subscription, Haiku, 2/2 subagents) | key rows on macOS (Claude key has no credit for Claude Code); one-hour PASS |
| Codex | core rows PASS (f7048ed8); H5 direct/dodger did no work (no credit) | login row PASS (ChatGPT, 2/2 subagents) | key rows on macOS; one-hour PASS |
| Hermes | core rows C3 INC (ask 4) | 0.22.6 mini row: all but C3 PASS, with the pad's CA workaround; one-hour from scratch INC (570 requests, 10 subagents, both kills named) | 0.22.6 without the workaround (fails: hermes-agent#148241); 15 subagents with 10 at once |
| Kilo, Qwen, Kimi Code, Goose, OpenCode, OpenCode 2, Crush | Paynest PASS | mini row PASS | 20-minute and many-subagent rows on macOS |
| Gemini CLI | INC (stalled on an approval screen) | mini row PASS (API key) | Google sign-in; many subagents |
| Copilot | records; no subagent link | records; no subagent link | a subagent reader |
| OpenClaw | INC (no tool use) | C3 FAIL (restated git password, stated loss) | a clean C3 |
| Amp | none | login row: records fully; reader links 2/2 subagents, main-thread usage | subagent tokens (Amp does not send them); a pinned version that lasts |
| Cline, Factory Droid | none | none (Cline needs a sign-in the pad cannot do) | everything |

## Promises (the seven points)

| Point | Tested | Not tested |
|---|---|---|
| 1 Every model call | C1 on every row that did work, both systems | one-hour PASS; Windows |
| 2 Every subagent | 10 harnesses linked on Linux; 8 pad + 3 login harnesses on macOS | many subagents on macOS; Copilot; Amp subagent tokens |
| 3 Never block | recorder kills and recover in 6 one-hour runs on Linux; on macOS a network drop (upload caught up), a recorder kill (789 ms gap) and a watchdog kill with `goalcup recover` | kill-gap bound under load (PASS 1 of 3); seal after a recover (MAC-LEARNINGS 7g, 7h) |
| 4 No secrets | C3 and keyscan on every row; stated losses: split echo, restated git password | a server-side scrub (not built) |
| 5 Upload | C4 server copy equals local on Linux and macOS rows (staging) | production Worker; production smoke |
| 6 Tell the participant | C6 on every row; version notes said at start (test only) | the newcomer docs run (PRD-15); a live version note |
| 7 Real harnesses | 15 harnesses listed, 12 with rows | Cline, Factory; Windows |

## Not tested at all

- **Windows (WSL2).** No row, no gate.
- **The release path.** `release.yml`, `install.sh` from a real GitHub release, the production Worker, a production smoke run. (`install.sh` from a local archive ran on macOS.)
- **The frozen head.** No freeze yet.
- **A one-hour run that passes,** on either system. The macOS one-hour run was driven by hand, not by `pad-run` (its macOS port of the fault steps has unit tests only).
- **Gateway and dodger cases on macOS** (the gateway key is over its budget).
- **A competition run from scratch** (a public repo, not `--test`), and the newcomer docs read by someone new.
