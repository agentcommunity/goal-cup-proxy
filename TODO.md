# TODO

The open list for goalcup 1.0. Decisions live in [DECISIONS](docs/DECISIONS.md). Done is defined in
[AGENTS.md](AGENTS.md#done).

## Open owner questions

The twelve questions are in [PRD-00](https://github.com/agentcommunity/goal-cup-proxy/blob/archive/docs/plans/2026-10-30-v1.0/PRD-00-index.md#owner-questions).
The build uses each default below until the owner answers.

1. Server and data. Default: no website database change; usage stats in R2, for organizers only.
2. Participant-facing changes. Default: yes to all five, including the whole environment for the
   harness minus proxy and CA variables; this touches D61's "forwards no key" clause.
3. Masking beyond option b. Default: no; only the run's `GH_TOKEN` is masked by exact value.
4. Rules and judging. Default: yes to all three; a tier does not decide eligibility.
5. Release order. Default: publish 1.0.0, then deploy production with the 1.0.0 floor.
6. Run 1's prompt. Default: not committed; without the file, A1 runs the pad's `full` prompt.
7. Hermes and per-call matching. Default: per call when `state.db` has usable times, else per session.
8. Short answers and the build budget. Default: the readings and report lines PRD-00 gives.
9. The watchdog's jobs. Default: the `start` or `continue` process, as PRD-00 lists.
10. Left guard class. Default: `[A-Za-z0-9_]`.
11. Rate-limit headers. Default: no exemption from the name rule.
12. One test-only build feature. Default: keep it, never in a release build.

- O1 ([ledger section 9](https://github.com/agentcommunity/goal-cup-proxy/blob/archive/docs/plans/2026-10-30-v1.0/OWNER-DECISIONS.md#9-open)), staging and the
  website database. Default: shared; no migrations, read-only use.
- O2, Copilot CLI and a subscription. Default: try the key; "not verified" if it fails.

## Still to decide

- Judging and result dates.
- Event capacity limits.
- Final judging rubric, shortlist threshold and appeal process.

## Known issues

- Linux x86_64 and macOS arm64 (owner's Mac, 3 Nov) are tested. macOS x86_64 and Linux arm64 are
  first built and tested by the release workflow at the owner's release (PRD-14 G5, after the merge).
  Windows (WSL2) is untested.
- Unmasked memory (held bytes, the run CA key) can still reach disk through swap on Linux
  ([PRD-00](https://github.com/agentcommunity/goal-cup-proxy/blob/archive/docs/plans/2026-10-30-v1.0/PRD-00-index.md#architecture)).
- The `zstd` command line tool may stop short on a live entry; read one with `goalcup-verify dump`
  ([RECORD-FORMAT](docs/RECORD-FORMAT.md#compression-chunks-and-crash-recovery)).
- H-28: usage objects in the record carry plan, credit and billing data. They must be stripped
  before any research export, which comes after the event (D26); nothing does it yet.
- H-29: a key the model echoes split across stream pieces is written unmasked
  ([PRD-05](https://github.com/agentcommunity/goal-cup-proxy/blob/archive/docs/plans/2026-10-30-v1.0/PRD-05-masking.md#risks-and-blind-spots)).
- F-07: a guarded key prefix right after a `-` inside a base64url run masks the rest of the run,
  failing closed past 16 KiB, up to the end of the message
  ([PRD-05](https://github.com/agentcommunity/goal-cup-proxy/blob/archive/docs/plans/2026-10-30-v1.0/PRD-05-masking.md#risks-and-blind-spots), question 10).
- PRD-11 hand-off: repoint the harness facts sheets in `docs/harnesses/` from
  [HARNESS-SETUP](docs/HARNESS-SETUP.md) to the [participant's guide](docs/PARTICIPANT-GUIDE.md#3-set-up),
  then delete HARNESS-SETUP. Until then it is a temporary duplicate and still
  describes the removed MCP command (D84).

## Release 1.0 (one freeze, one release-row run, before the event)

Coverage: [TEST-COVERAGE](https://github.com/agentcommunity/goal-cup-proxy/blob/archive/docs/plans/2026-10-30-v1.0/TEST-COVERAGE.md). Findings:
[MAC-LEARNINGS](https://github.com/agentcommunity/goal-cup-proxy/blob/archive/docs/plans/2026-10-30-v1.0/MAC-LEARNINGS.md).

Before release:
- 🔴 Freeze; release rows on the frozen head (Claude Code, Codex, Hermes), then `scripts/pad/table --release-check`. Hermes needs 0.22.5 and 0.22.6 both verified on macOS arm64 (only 0.22.6 has rows today).
- 🔴 Release path end to end: GitHub release, `install.sh` from it, production Worker smoke run.
- 🟡 A one-hour PASS. Best so far INC on both systems; misses were harness subagent counts, not loss.
  Option: set the subagent target to what harnesses really start (about 10).
- 🟡 Amp pin: bump when Amp's server refuses it.

After 1.0:
- Watchdog-only kill: nothing warns that the recorder has no restarter (MAC-LEARNINGS 7h).
- Restated passwords stay raw (OpenClaw, stated loss): server-side scrub, or `.git/config` remote
  passwords as exact values at launch (MAC-LEARNINGS 7a).
- Copilot and Droid subagent readers.
- `hermes gateway stop` helper times out on every run (MAC-LEARNINGS 3).
- Codex in its private home reads `~/.agents/skills` (MAC-LEARNINGS 7d).
- Cline and Factory Droid rows; Windows (WSL2).
- A competition run from scratch with a public repo; a newcomer reads the docs.
- RULES lists banned Cursor flags; there is no Cursor connector.

## After the merge to main

- The DECISIONS rewrite to 24 entries with a retired-numbers table (PRD-15). Each "This supersedes"
  line names every old entry its status rows point to (D3, D9, D37, D40, D43, D45, D46, D52, D53
  and D71 are not named today); D75, D76, D80, D81 and D84 go back to short prose.
- Hermes notes from D61 and D72 move into [its facts sheet](docs/harnesses/hermes-cli.md) (PRD-11).
- PRD-15's contributor run and organizer run.
- The judge view (PRD-08).
- PRD-14 G2 (sabotage) and G5 (the release workflow on four targets); PRD-12 P8 (contributor dry run).
- `stopped-runs.sql`, the organizer query for stopped runs (PRD-07).
- Following HTTP/2 PUSH_PROMISE, WebSocket over HTTP/2 (RFC 8441) (PRD-03); today each is a GAP
  (`h2_unsupported`), never silent. Compressed gRPC bodies are a GAP too (`unsupported_encoding`).
- The mask worker pool, only if PRD-02's throughput test shows masking is the slow stage (PRD-02).
