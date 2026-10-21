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

### Long cells (started about 18:35, 120-minute practice runs)

Running overnight; results are added here as they end: lx-hermes (network cut 10 min, pause
20 min), mac-hermes (subagents), mac-amp (subagents), lx-amp (network cut), mac-codex (subagents,
no git), mac-disk (D65 disk pause and resume).

## Fixes made today

- ea5c30f9: reserve test, campaign build order, check-sql locale.
- Fix plan v1 ([plan](2026-10-22-fix-plan.md), merged 9fec42e6): one GitHub token file for test
  cells (`scripts/kit_settle.py`), host cells get the cheapest-model rules and the model check,
  Hermes reports its version (verified live: "Hermes Agent v0.19.0"), docs say public repos.
- Fix plan v2 ([plan](2026-10-22-fix-plan-v2.md)): in progress in two worktrees
  (`fix/v2-product`, `fix/v2-kit`).

## For the cloud agent

In priority order. Evidence is aggregates; raw traces stay on the owner's Mac.

1. **Claude Code replies are brotli; the proxy cannot decode them (every Claude run "incomplete").**
   `api.anthropic.com` answers `content-encoding: br` when offered (curl, 22 Oct). The proxy decodes
   identity/gzip/deflate only (`crates/goalcup-cli/src/proxy/http.rs` `Decoder::for_codings`), so
   each reply is an `unsupported_encoding` gap (mac-claude and lx-claude: 2 gaps each, the replies to
   the start-up probe). Fix in plan v2 K5 (brotli-decompressor, plus zstd). Check that branch first.
2. **Codex cannot commit or push without a human approving.** Codex's default `workspace-write`
   sandbox makes `.git` read-only: `git add` failed with `Operation not permitted` on
   `.git/index.lock`; Codex then asked for approval and waited 8.7 of 10 minutes (mac-codex). The
   connector passes no sandbox option (D57); RULES let the participant type only "keep going".
   Owner decision: connector arguments, a RULES change allowing harness permission answers, or
   participant guidance. Without it, a Codex participant has no judged commit.
3. **A matrix row passes without any agent work.** lx-codex and lx-amp passed with 0 model turns.
   Plan v2 K7 adds the check; K6 (bracketed paste: Codex 0.153 on the Linux pty split the 1,443-char
   prompt into a 1,024-char paste plus text, and the Enter became a newline) and K8 (Claude Code's
   "Try the new fullscreen renderer?" dialog took the prompt; Amp welcome state) fix the causes.
4. **macOS: harness stop sealed Unresolved (ProcessAuthorityUnavailable) although it exited**
   (mac-codex, mac-amp). A root caught in the ~30-45 ms exiting window (`ps` state `E`,
   `proc_pidinfo` already failing) is refused as recycled in
   `crates/goalcup-daemon/src/terminate/authority.rs` (~394-406). Plan v2 K10.
5. **Kit**: campaign starts must be spaced (HTTP 429); `campaign.py login` settles only when prep
   exits 0 (prep exits 1 while any step is todo); Amp's version line holds a relative age.
6. Still to run when the owner is back: the token, then test-run cells with drills via
   `campaign.py`, full-flow cells on the gc-test repositories (judged commit), Claude Code after K5
   and K8, lx-codex after K6.
