# Local test day, 2026-10-22 (results, aggregates only)

Branch `smart/zen-gates-d9tmbh` (draft PR #226), the owner's Mac (arm64, 12 CPU, 24 GiB) and
Docker on it (12 CPU, 8 GiB). Staging only. Raw traces, logs and logins stay on the Mac under
`~/goalcup-campaign/` (cells, results, FINDINGS.md). Times are UTC. This file is updated during
the night; the newest state is at the bottom of "Cells".

## Setup done

- Build: `cargo build` debug and release OK.
- `gate.sh full` (Mac, normal user): 2230 of 2231 passed; the one failure,
  `m2_cli_lifecycle::a_run_holds_one_reserve_file_and_releases_it_at_the_end`, failed 13 of 13
  (the test asserted the reserve alone; the reserve is a floor for reserve plus chain). Fixed in
  ea5c30f9 (test asserts the promise). `gate.sh fast` PASS after each change since.
- `server/worker/scripts/check-sql.sh`: PASS on Postgres 17.9 (needed `LC_ALL=C` on macOS; the
  script sets it now, ea5c30f9).
- Migration `20261021100000_goal_cup_tracer_event_readiness.sql` applied once to the shared
  database with the owner's approval; `pnpm db:check` passed after. Website PR branch
  `goalcup/event-readiness-migration` (worktree `~/dev/worktrees/agentCommunity_PAGE/
  goalcup-event-readiness-migration`) is committed but NOT pushed: the website pre-push hook needs
  the owner's `SERGIO_OWNER_CHECKOUT`/`SERGIO_OWNER_PIN`. The mirror regenerated from website
  `main` plus the file equals `server/worker/sql/goal-cup-tracer-upload.sql`.
- Staging Worker deployed from this branch. Staging `TEST_APPROVAL_KEY` rotated (new key in the
  owner's `~/.goalcup-secrets/`, mode 600). For the overnight long cells staging runs with
  `PRACTICE_RUN_MINUTES=120` (a `--var` on deploy; the next normal deploy restores 10).
- Test repositories (public, empty, owner `nembal`): `gc-test-smoke-flow-1`, `gc-test-mac-amp-1`,
  `gc-test-mac-hermes-1`, `gc-test-lx-codex-flow-1`, `gc-test-lx-claude-flow-1`,
  `gc-test-spare-1..3`. Created private, made public with the owner's yes (Goal Cup refuses
  private repositories).
- Not done: the fine-grained GitHub token for the test repositories (owner); so no test run, no
  full-flow cell and no judged-commit check yet. WSL2: not run (no Windows machine).

## Sign-ins (one per cell, never copied)

| Cell | Where | Harness | Version | Model setting |
|---|---|---|---|---|
| mac-codex | Mac host | codex-cli | 0.156.1 | gpt-5.6-luna, low |
| mac-claude | Mac host | claude-code-cli | 2.1.299 | haiku, thinking off |
| mac-amp | Mac host | amp-cli | 0.0.1792209635-g24a9fe | mode low |
| mac-hermes | Mac host | hermes-cli | 0.19.0 | DeepSeek deepseek-v4-flash |
| mac-disk | Mac host, 9 GiB APFS image | hermes-cli | 0.19.0 | same |
| lx-codex | Docker | codex-cli | 0.153.0 | gpt-5.6-luna, low |
| lx-claude | Docker | claude-code-cli | 2.1.283 | haiku, thinking off |
| lx-amp | Docker | amp-cli | 0.0.1792209635-g24a9fe | mode low |
| lx-hermes | Docker | hermes-cli | 0.19.0 | DeepSeek deepseek-v4-flash |

Measured: Claude Code on Linux keeps its login under its config home (file names logged by
`linux-matrix.sh login`); Amp keeps its last mode in `~/.local/share/amp/session.json`
(`agentMode`) and its onboarding state there too (`neoWelcomeDismissed`); Amp's cheapest mode is
`low`. Hermes needs `model.base_url` in its own config.yaml (D62) and no browser sign-in.

## Cells

### 10-minute staging practice smokes (17:53 to 18:13)

Eight started at once; seven were refused with HTTP 429 (staging allows 5 device starts per
minute per address; each matrix run does a login and a start). Restarted 90 s apart: no 429.

| Cell | Row | Integrity | Server copy | Agent worked | time-left in harness | seal in harness |
|---|---|---|---|---|---|---|
| mac-codex | PASS | valid | equal | yes, 0 commits (sandbox) | answered | refused |
| mac-amp | PASS | valid | equal | yes, 19 commits | answered | refused |
| mac-hermes | PASS | valid | equal | yes | answered | refused |
| lx-hermes | PASS | valid | equal | yes, 44 tests | answered | refused |
| lx-codex | PASS (hollow) | valid | equal | **no** | - | - |
| lx-amp | PASS (hollow) | valid | equal | **no** | - | - |
| mac-claude | FAIL | incomplete (2 gaps) | - | **no** | - | - |
| lx-claude | FAIL | incomplete (2 gaps) | - | **no** | - | - |

`goalcup status` showed disk, memory and CPU in every cell asked. Docker adds a PROBLEM line "no
sleep inhibitor on this machine".

### Long cells (18:21 to 20:31, 120-minute staging practice runs)

Every run reached its deadline, sealed and uploaded. Every matrix row says FAIL for one kit reason:
after about 30 minutes acknowledged chain parts are pruned locally (by design) and `matrix.sh`
requires `chain.jsonl` ([plan v3](2026-10-22-fix-plan-v3.md) L1). The server copies were rebuilt
with `scripts/fetch-run.sh` and verified instead.

| Cell | Workload, drills | Server copy (records / bytes) | Integrity | Notes |
|---|---|---|---|---|
| lx-hermes | build task; network cut 10 min, pause 20 min | 2,650 / 73.5 MB | valid, 0 gaps | heartbeat failures 1-2 during the cut, 0 after; run state survived the pause; agent idle after ~7 min |
| mac-hermes | subagents | 2,751 / 55.9 MB | valid, 0 gaps | 2 delegate_task children, on the wire, unattributed |
| mac-amp | subagents | 11,740 / 9.9 MB | valid, 0 gaps | subagents on the parent thread with parentToolUseId; subagents used claude-opus-5-5 in mode low |
| lx-amp | build task; network cut 10 min | 21,115 / 20.9 MB | valid, 0 gaps | prompt received (welcome fixed); ACK covered the pre-cut tail 9.4 min after the cut |
| mac-codex | subagents, no git | 14,720 / 16.4 MB | valid, 0 gaps | 2 subagent rollouts, attributed; no approval stall without git |
| mac-disk | build task; D65 disk drill | 2,176 / 52.1 MB | valid, 0 gaps | **D65 PASS**: paused below 1 GiB within 35 s, resumed above 3 GiB within 24 s, a pause held through the deadline and the run still sealed |

Local tail part equals the server copy's tail in all six. Daemon and CLI memory and open files flat
in every cell. Uploads go on a 30-minute timer by design, so "ACK lag under 2 minutes" is not a
usable pass line (plan v3 L2). Agents finished in 5 to 7 minutes and idled: Hermes stops at
`agent.max_turns` (default 90); the workloads are too short for long cells (plan v3 L4).

### Reruns after fix plan v2 (22 Oct 00:43 to 00:57 UTC, 10-minute practice)

| Cell | Before v2 | After v2 |
|---|---|---|
| mac-claude | FAIL, 2 gaps, prompt lost to a dialog | **PASS**: integrity valid, 0 gaps, server copy equal, agent worked (3 commits, 25 tests), time-left answered, seal refused. First clean Claude Code capture. |
| lx-claude | FAIL, 2 gaps, no work | 0 gaps, server copy equal, prompt delivered, but Claude never answered it (1 small haiku call); model check FAILs the row. Cause under analysis. |
| lx-codex | hollow PASS, prompt never submitted | prompt delivered as one bracketed paste; Codex's sandbox cannot start in Docker (`bwrap: No permissions to create a new namespace`), Codex asks for escalation and waits: no work. |

The new "agent worked" row check passed lx-claude and lx-codex on title or probe calls: too
lenient. Root cause of both stalls: a harness permission prompt nobody may answer (Codex's
sandbox escalation; Claude Code 2.1.283 falling back to manual mode without
`skipDangerousModePermissionPrompt`). Fixed in [plan v4](2026-10-22-fix-plan-v4.md), with the
owner's decision D67 (Goal Cup recommends a no-prompt setup; participants configure it).

### Reruns after fix plan v4 (22 Oct 01:33 to 01:45 UTC)

| Cell | Result |
|---|---|
| lx-codex | **PASS**: no approval prompts, 2 commits, workspace and agent-worked checks pass, integrity valid, server copy equal, every turn gpt-5.6-luna low. But Codex's shell in the Docker cell could not find `goalcup` (`goalcup: not found`), so `time-left` did not run there. |
| lx-claude | **PASS**: no approval prompts, 58 tool uses, 6 commits, 0 gaps, server copy equal, every assistant message haiku, time-left answered, seal refused. |

Every harness now has a capture with real agent work on both surfaces (Mac: codex, claude, amp,
hermes; Docker: codex, claude, hermes, and amp in the 120-minute cell).

### Round 2: eight 120-minute cells (22 Oct 01:57 to 04:10 UTC, long workload, Docker drills)

Server copies fetched and verified for all eight: **7 valid with 0 gaps**; mac-amp incomplete with 2
gaps that are most likely a false loss in our proxy (a client that closes before its TLS greeting is
counted as fail-open; [plan v10](2026-10-22-fix-plan-v10.md)).

| Cell | Result | Notes |
|---|---|---|
| lx-codex | ran to deadline, 20 MB | network cut: ACK covered the pre-cut tail 12 min after the cut; `goalcup: not found` inside Codex (fixed in v5, proven below) |
| mac-codex | ran to deadline, 24 MB | **v4 proven on macOS** (no prompts, 8 commits); **K10 proven** (stop Confirmed); no leak (CLI spike = extra short-lived goalcup processes) |
| lx-claude | no work | stalled at the first token refresh (the v6 bug); pause drill recovered in 9 min |
| mac-claude | no work | auto-mode dialog took the prompt (the v7 bug) |
| lx-amp | 70 commits, 44 MB | pause drill: ACK caught up 2 min after unpause |
| mac-amp | 68 commits, 38 MB | 2 fail-open gaps (plan v10) |
| lx-hermes | 32 commits, 239 MB | network cut and pause recovered; agent finished in 20 min |
| mac-hermes | 105 commits, **1.19 GB** | the long workload held ~110 of 120 min; busy Hermes writes ~600 MB/h raw (~5-7 GB compressed per 168 h) |

All daemons' memory and open files flat; uploads on the 30-minute timer; every drill recovered inside
the 35-minute line.

### Proof cells on the final code (22 Oct 04:04 to 04:18 UTC, 10-minute practice)

| Cell | Proves | Result |
|---|---|---|
| mac-claude | v6 (token refresh at start), v7 (auto-mode offer) | **PASS**: no refresh error, no dialog, 14 commits, 0 gaps |
| lx-claude | v6, v7 in Docker | **PASS**: 16 commits, 0 gaps, new `<connector>.goalcup.lock` in use |
| lx-codex | v5 (`goalcup time-left` inside Codex's login shell) | **PASS**: time-left answered, seal refused |

### WP-0 measurements (`scripts/research-measure.py`, no paid run)

On today's six 120-minute server copies and the five 18-19 Oct Codex traces on disk (no 18-19 Oct
Claude, Amp or Hermes trace exists locally). Rates are over the measured span; 168 h figures are
linear **projections** from 2-hour runs whose agents idled after a few minutes, so a busy run can be
larger.

| Harness | Rate (2 h runs) | 168 h projection | zstd -3 ratio | Projected compressed |
|---|---|---|---|---|
| Codex | 8.2 MB/h | ~1.4 GB | 10.5x | ~130 MB |
| Amp | 4.9-10.4 MB/h | ~0.8-1.75 GB | ~10x | ~80-170 MB |
| Hermes | 26-37 MB/h | ~4.4-6.2 GB | 14-20x (52-57x with `--long=27`) | ~225-445 MB |

- Hermes is 3-7 times the others: over 95% is wire, because chat-completions requests resend the
  whole conversation each turn. The repeats sit far apart, so a long zstd window helps a lot.
- Server size limit (`RUN_MAX_COMPRESSED_BYTES`, 64 GiB) is far away for every harness.
- Local disk stays bounded: acknowledged parts are pruned after upload (seen in every long cell).
- Short 18-19 Oct Codex runs wrote about 46 MB in 5-8 minutes, mostly connection open/end records
  (about 17k each) at start-up; not a rate to project.

## Fixes made today (all merged into `smart/zen-gates-d9tmbh`)

| Plan | What it fixed | Proven live? |
|---|---|---|
| ea5c30f9 | reserve test asserted the wrong thing; campaign build order; check-sql locale | gate |
| [v1](2026-10-22-fix-plan.md) | kit token file for test cells; host model rules; Hermes reports its version | Hermes version yes; token path no (no token) |
| [v2](2026-10-22-fix-plan-v2.md) | **brotli/zstd replies decoded** (every Claude run had gaps); macOS exiting-root stop; bracketed paste; "agent worked" check; Claude/Amp first-run state; spaced starts | yes: Claude 0 gaps Mac and Docker |
| [v3](2026-10-22-fix-plan-v3.md) | long runs verified on their server copy; drill pass lines for the 30-minute upload; honest upload status; long workload; subagent attribution (Amp, Codex) | server copies verified by hand; matrix path not run |
| [v4](2026-10-22-fix-plan-v4.md) | no permission prompts in test cells; "agent worked" = workspace changed; Docker fetch settings; **D67** (participants set up their own harness) | yes: Docker Codex and Claude work |
| [v5](2026-10-22-fix-plan-v5.md) | Goal Cup's run home restores the PATH it handed the harness (Codex login shell on Linux lost `goalcup` and the participant's tools); skill backstop; kit installs goalcup like install.sh; two flaky tests | **yes**: Docker Codex answers time-left |
| [v6](2026-10-22-fix-plan-v6.md) | **Goal Cup's home lock collided with Claude Code's token-refresh lock**: a start or continue with an expired token stalled | **yes**: Mac and Docker Claude work after a refresh |
| [v7](2026-10-22-fix-plan-v7.md) | kit settles Claude Code's auto-mode offer | **yes** |
| [v8](2026-10-22-fix-plan-v8.md) | [`docs/HARNESS-SETUP.md`](../../HARNESS-SETUP.md); `goalcup prep` links to it | links and tests |
| [v10](2026-10-22-fix-plan-v10.md) | a client that closes before its TLS greeting is a client failure, not a loss (mac-amp's 2 gaps); fail-open counted by reason | unit and integration tests; live Amp check open |
| [v9](2026-10-22-fix-plan-v9.md) | `goalcup mcp`: time-left as an MCP tool the agent calls (owner-approved); prep prints the snippet | Claude Code registered it ("Connected"); no run called it yet |

Gates: `gate.sh fast` PASS after every merge (last on 30299446). `gate.sh full` on ad030750: all
2245 Rust tests pass; two flaky Python tests (fixed in v5). A code review of all changes found no
high-severity defect; six small findings are being fixed on `fix/review`.

## For the cloud agent

In priority order. Evidence is aggregates; raw traces stay on the owner's Mac. Findings list (local):
`~/goalcup-campaign/FINDINGS.md` on the owner's Mac.

1. **Plan v10 is merged**: a 2-hour Amp cell must now show 0 gaps (mac-amp had 2 false losses). **v9 live
   check**: one cell per harness with the `goalcup mcp` tool configured from prep's snippet
   (Codex/Amp/Hermes "not measured"). v5, v6, v7 and K10 are proven live.
2. **The competition path has never run.** Needs the owner's fine-grained token
   (`~/.goalcup-secrets/gc-test-token`, nembal/gc-test-* repos, public): test runs via
   `campaign.py` with drills, full-flow cells (pushes, `GOALCUP.md`, judged commit equals GitHub's
   last push before the deadline), staging `GITHUB_TOKEN` reading the repos.
3. **Codex in a container has no working sandbox** (`bwrap: No permissions to create a new
   namespace`); with Codex's defaults every command waits for an approval. D67: participants set
   `approval_policy`/`sandbox_mode` themselves; HARNESS-SETUP.md says how. Verify the guide's Codex
   section on a real Linux install.
4. **Long workloads end early with cheap models** (Hermes 32 commits in 20 min, then DONE.md),
   so drills often hit an idle chain. Use a task without a natural end for drill cells.
5. **Token expiry during a run** is proven only for Claude (refresh works mid-run). Codex and Amp:
   not measured.
6. **WSL2**: not run (no Windows machine). Needs a Windows 11 ARM VM with nested virtualization
   (the Mac is an M4 Pro) or a cloud Windows VM.
7. **Staging is set to `PRACTICE_RUN_MINUTES=120`** (deploy `--var`); a plain
   `wrangler deploy --env staging` restores 10.
8. **Owner items**: website PR nembal/agentcommunity_page#319 (migration file, already applied);
   review the new dependency `brotli-decompressor` 5.0.3 (BSD-3-Clause/MIT); D67 has a bullet added
   by an implementer (prep links the guide); Amp's Task subagents ran `claude-opus-5-5` in mode
   `low` (cost note in the guide).

## What still exists (kept on purpose; delete only what the owner names)

- **Docker volumes (logins and traces):** `goalcup-linux-matrix-codex-cli-lx-codex`,
  `goalcup-linux-matrix-claude-code-cli-lx-claude`, `goalcup-linux-matrix-amp-cli-lx-amp`,
  `goalcup-linux-matrix-hermes-cli-lx-hermes`, plus older `goalcup-linux-matrix-codex-cli` and
  `goalcup-linux-matrix-hermes-cli`, and the build volume `goalcup-linux-matrix-target-2732686530`.
- **Mac cells:** `~/goalcup-campaign/cells/mac-{codex,claude,amp,hermes}` (with `-artifacts`,
  `-work`), the disk-drill cell on the 9 GiB image `~/goalcup-campaign/gcsmall.dmg` (mounted at
  `/Volumes/gcsmall`), all results under `~/goalcup-campaign/results/`, the findings list
  `~/goalcup-campaign/FINDINGS.md`.
- **Secrets (owner's Mac, mode 600):** `~/.goalcup-secrets/staging-test-approval-key` (staging
  `TEST_APPROVAL_KEY` was rotated to it on 22 Oct). The GitHub token file does not exist yet.
- **GitHub:** public empty repos `nembal/gc-test-*` (8); website PR nembal/agentcommunity_page#319.
- **Staging:** every test and practice run of the day (run ids in the results above); the Worker is
  this branch's code with `PRACTICE_RUN_MINUTES=10`.
- **Worktrees** under `~/dev/worktrees/goal-cup-tracer/` for each fix branch (all merged).
