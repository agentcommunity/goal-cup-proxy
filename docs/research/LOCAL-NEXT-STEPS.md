# Local next steps (after the 20 to 21 Oct cloud session)

Status 22 Oct: the [local test day](results/2026-10-22-local-test-day.md) ran steps 1 to 7 on the
owner's Mac and Docker, against staging. Ticks below carry a "(22 Oct: ...)" note. What needs a
GitHub token, a Windows machine or the owner is still open.

What the owner and the local agent do on their own machines, in order, to take branch
`smart/zen-gates-d9tmbh` from "built and unit tested in a cloud container" to "measured and
released". Until 22 Oct nothing here had run against real Docker, staging, GitHub or a real harness. The
[local test day](LOCAL-TEST-DAY.md) is the plan for doing steps 2 to 7 in one day. Record every
result as you go (step 9). Background: the [readiness plan](../plans/2026-10-20-event-readiness.md)
and the [local campaign](LOCAL-CAMPAIGN.md).

## 1. Review and merge

- [x] Draft PR [#226](https://github.com/agentcommunity/goal-cup-proxy/pull/226) from
      `smart/zen-gates-d9tmbh` to `main`. CI (`check`) is green. Ignore the DCO check: no sign-off
      is required (D47).
- [ ] One reviewer reads it once (AGENTS.md "Done"), ticks the PR boxes, marks it ready and merges
      after step 2 passes. Main areas: D64 (errors never stop a run), D65 (disk pause), D66 (time
      left, no self-seal), parallel test runs, size limit, vitals, judged push, rename.
- [x] Pull it locally and build: `cargo fetch --locked`, `cargo build --offline --workspace`.
      (22 Oct: `cargo build` debug and release OK on the Mac.)

## 2. Gate as a normal user (not root)

- [ ] `sh scripts/gate.sh fast`, then `sh scripts/gate.sh full`, and `sh scripts/linux-check.sh`.
      (22 Oct: `full` as a normal user on the Mac passed 2230 of 2231. The one failure,
      `m2_cli_lifecycle::a_run_holds_one_reserve_file_and_releases_it_at_the_end`, was a wrong test
      assertion, fixed in ea5c30f9. `fast` passed after each change since. `full` was not rerun
      after the fix, and `linux-check.sh` did not run.)
- [x] The cloud container ran as root, so 7 permission tests always failed there. On 21 Oct they
      passed in the cloud as a non-root user, except the m2_proxy one (the container has no IPv6).
      Confirm here:
      `upload_chunks::retries` (2), `launch::tests::a_session_is_a_native_file_written_since_the_run_started`,
      `m2_proxy_integration::start_refuses_invalid_configuration_before_touching_the_sink`,
      `dir_modes::an_owner_denying_umask…`, `capture_native::finish_reports_discovery_it_could_not_complete`,
      `harness_contract_kit::every_kit_case_seals_and_matches_expectations`.
      (22 Oct: all in the Mac `full` gate, which failed only the reserve test above.)
- [x] These full-tier tests failed in the cloud only because `*.localhost` did not resolve there; with
      a hosts entry they passed. Check them here:
      `capture_crash_campaign::torn_tail_is_preserved_and_nonresumable`,
      `capture_crash_campaign::daemon_exit_preserves_prefix_and_honest_recovery_limit`,
      `capture_crash_campaign::a_standalone_seal_that_commits_first_keeps_the_proxys_loss`,
      `launcher_sweep::the_launcher_sweeps_an_orphan_forked_after_the_root_died_before_the_seal`.
      (22 Oct: same Mac `full` run, no failure.)
- [x] macOS: the new `machine::process_usage` code was only type-checked, and a cloud cross-check
      needs the macOS SDK. Build and run its test on the Mac.
      (22 Oct: built and tested in the Mac `full` gate; `goalcup status` showed disk, memory and CPU
      in every cell asked.)
- [ ] `npm --prefix server/worker test` (251). Three clock races were fixed on 21 Oct; it then
      passed repeatedly in the cloud, also under load.
- [ ] `capture_lifecycle::a_seal_whose_registry_write_was_lost_is_committed_without_a_second_terminal`
      failed once in the cloud fast gate under load and passed 3 of 3 alone: watch it for flakiness.
- [ ] Running the gate inside Docker: add `127.0.0.1 api.e00.localhost auth.e00.localhost` to the
      container's `/etc/hosts` and make sure it has IPv6 on loopback (README, "The gate").

## 3. Database migrations (website repo): one shared database

One file in `server/worker/sql/pending/`: `20261021100000_goal_cup_tracer_event_readiness.sql`
(the five changes of 21 Oct merged: parallel test runs, size limit, vitals and disk email, judged
push, notice wording). Apply it **before** deploying the Worker that needs it.

- [x] Run `sh server/worker/scripts/check-sql.sh` first: it applies the mirror and the file on a
      throwaway Postgres in one transaction and smoke-tests the functions. It passed in the cloud
      on 21 Oct (Postgres 16). The file is apply-once: a second apply fails and rolls back.
      (22 Oct: PASS on Postgres 17.9. It needed `LC_ALL=C` on macOS; the script sets it now, ea5c30f9.)
- [x] Staging and production share ONE database: applying a file applies it to both at once.
      There is no "staging first" for SQL. The old-signature wrappers keep the live production
      Worker working.
      (22 Oct: the file was applied once to the shared database, with the owner's approval. So
      production has it too. `pnpm db:check` passed after.)
- [x] Copy each into the website repo `supabase/migrations/`; the lead agent applies them once,
      the owner approves. Regenerate the mirror (command in its header) and check it matches.
      (22 Oct: the mirror regenerated from website `main` plus the file equals
      `server/worker/sql/goal-cup-tracer-upload.sql`. The website branch
      `goalcup/event-readiness-migration` is committed but NOT pushed: see "Open since 22 Oct".)
- [x] Then deploy the staging Worker and test it.
      (22 Oct: deployed from this branch; the test day ran against it.)
- [ ] Then deploy the production Worker with the 26 Oct point release.

## 4. Server settings that could not be changed from the cloud

- [ ] `GITHUB_TOKEN` secret on production and staging (`wrangler secret put GITHUB_TOKEN`, and
      `--env staging`). Now required: the judged-push read uses GitHub's activity API.
- [ ] Check on staging with a real repo: the activity API answers with the token's permission, how
      soon a push appears, and how long history is kept.
- [x] `TEST_APPROVAL_KEY` on staging only (exists). Keep the key file local, `chmod 600`.
      (22 Oct: rotated; the new key is in the owner's `~/.goalcup-secrets/`, mode 600.)
- [ ] Test members for parallel cells: either rely on the parallel-test-runs migration (then set
      `"test_runs_parallel": true` in the campaign config) or add synthetic members (SQL in LOCAL-CAMPAIGN).
- [x] Deploy the staging Worker after its migrations; `npx wrangler tail --env staging` while testing.
      (22 Oct: deployed. For the long cells staging ran with `PRACTICE_RUN_MINUTES=120`, a `--var`
      on deploy; the next normal deploy restores 10. `wrangler tail` use is not recorded.)
- [ ] Website: approval page wording for test runs (no longer "this replaces your current run"),
      and the `disk_full` email text. The 3 h / 6 h idle emails still fire while an agent is paused
      for disk: decide whether that is fine.
- [x] `RUN_MAX_COMPRESSED_BYTES` is in `wrangler.toml` (64 GiB). Change only if measurements say so.
      (22 Oct: no change. WP-0 projects about 80 to 445 MB compressed for 168 h; the limit is far away.)
- [ ] Event ops (later): `EVENT_ID` switch on 6 Nov, deploy freeze 7 to 15 Nov, `MAINTENANCE=on` as
      the safe switch, a named on-call person per shift.

## 5. Machines and test kit

- [ ] Mac: `caffeinate -dimsu` during long runs, mains power, auto-updates off, allow the rebuilt
      `goalcup` in Little Snitch, `brew install zstd` if missing.
- [ ] Docker Desktop: raise its disk limit (several cells, about 3 to 5 GB each); cells use named
      volumes (ext4), never bind mounts on the Mac disk.
- [ ] WSL2: files inside the Linux file system (not `/mnt/c`), Windows Update restarts paused, WSL
      idle shutdown off, mains power.
- [ ] `python3 scripts/campaign.py check CONFIG`, then `build` once. `check` passed in the cloud; `build`
      could not run there (its container did not trust the sandbox proxy), so it is untested.
      (22 Oct: `build` refused before the host sign-ins; its order is fixed, ea5c30f9. The cells ran
      through the campaign scripts. A clean `check` then `build` result is not recorded.)
- [x] Sign in once per cell: `campaign.py login CONFIG <cell>` (Codex rotates its login token, so
      never share or copy a login between cells). Cheapest settings are set by the scripts:
      Codex `gpt-5.6-luna` at `low`, Claude Code `haiku`, thinking off.
      (22 Oct: nine cells signed in, one login each, never copied: mac-codex, mac-claude, mac-amp,
      mac-hermes, mac-disk, lx-codex, lx-claude, lx-amp, lx-hermes. Hermes needs no browser sign-in.)
- [ ] Sandbox for participants: run `goalcup doctor` in one sandbox and send the output. Without
      `goalcup prep` first, `doctor` checks every harness and exits 1 unless all are installed. goalcup refuses
      overlayfs, btrfs, ZFS, NFS, SMB, tmpfs and FUSE; a container sandbox may need an ext4 or XFS volume.

## 6. Measure what the cloud could not

- [x] Claude Code on Linux: where the login file lands (the Docker login logs the names).
      (22 Oct: it keeps its login under its config home; `linux-matrix.sh login` logs the file names.)
- [ ] Codex: does the installed version accept effort `minimal` for Luna?
- [x] Amp subagents: one practice run whose prompt makes Amp use a subagent (for example "use a
      Task subagent to count the files in this repository"). Check that the subagent's messages appear
      in the decoded record (`goalcup-verify --research-json`). Today this is unmeasured
      ([AMP-FOLLOWER](AMP-FOLLOWER.md)).
      (22 Oct: mac-amp 120-minute cell, 11,740 records, valid, 0 gaps. Subagents run on the parent
      thread and carry `parentToolUseId`. The decoder now carries that id and decodes `Task`
      results, 89703603. The subagents used `claude-opus-5-5` while the thread was in mode `low`.)
- [ ] Hermes and Amp: is there a skills folder they load? Until measured, only Codex and Claude Code
      get the goal-cup skill (and its `goalcup time-left` line).
- [ ] Inside each harness: the agent runs `goalcup time-left` (answers from the server's count), and
      `goalcup seal` refuses inside the harness but works from another terminal.
      (22 Oct: in the 10-minute smokes `time-left` answered and `seal` was refused inside the
      harness for mac-codex, mac-amp, mac-hermes, lx-hermes, and later mac-claude and lx-claude.
      lx-codex got `goalcup: not found`; fixed in fix plan v5, not rerun. `seal` from another
      terminal is not recorded.)
- [x] Disk pause (D65): on a small volume (`hdiutil create -size 6g -fs APFS …` on the Mac), fill the
      disk: the agent pauses below 1 GiB free, resumes above 3 GiB. (22 Oct: mac-disk, 9 GiB APFS
      image, PASS: paused below 1 GiB within 35 s, resumed above 3 GiB within 24 s; a pause held
      through the deadline and the run still sealed.)
- [ ] D65 disk email arrives once (not checked on 22 Oct).
- [ ] Vitals: `goalcup status` shows disk, memory and CPU; staging heartbeats store them.
      (22 Oct: `status` showed them in every cell asked. Stored heartbeats are not checked.)

## 7. Runs

- [x] 10-minute smoke of each cell type (Mac host, Docker Codex, Docker Claude).
      (22 Oct: eight staging practice smokes. Mac: Codex, Claude Code, Amp, Hermes. Docker: Codex,
      Claude Code, Amp, Hermes. After fix plans v2 and v4, every harness has a capture with real
      agent work on both surfaces (Docker Amp in the 120-minute cell). Details: [results](results/2026-10-22-local-test-day.md).)
- [ ] 10-minute smoke of WSL2 (22 Oct: not run, no Windows machine).
- [ ] WP-0 measurements, no paid run: `scripts/research-measure.py` on the 18 and 19 Oct traces and
      `scripts/research-staging.py` on staging. Paste the output (aggregates only) back.
      (22 Oct: `research-measure.py` ran on the six 120-minute server copies and the five 18 to
      19 Oct Codex traces; aggregates are in the results file. No 18 to 19 Oct Claude Code, Amp or
      Hermes trace exists locally. `research-staging.py` is not recorded.)
- [ ] Ladder on staging: 2 h, 8 h, 24 h, with the drills in LOCAL-CAMPAIGN.
      (22 Oct: the 2 h step ran in six cells, with network cut, pause and disk drills; all sealed
      and uploaded, server copies valid with 0 gaps. 8 h and 24 h not run.)
- [ ] By 27 Oct: start the 168 h run, using the 26 Oct point release.

## 8. Release

- [ ] Point release around 26 Oct: `scripts/release.sh prepare <x.y.z>`, merge, full gate and
      `linux-check.sh` on merged `main`, then `release.sh tag <x.y.z>`. Verify the published installer
      in a fresh prefix.
- [x] Rename step 4: rename the GitHub repo to `goal-cup-proxy`, then let an agent update the URLs
      and test that old install and release links redirect (22 Oct: old `releases/latest/download/`
      and `releases/download/v0.6.1/` URLs answer 301 to `goal-cup-proxy`).
- [ ] Rename step 5 (database schema): decide before or after the event.

## 9. Record everything

- [x] One private results folder per day: campaign `summary.json`/`summary.md`, cell reports, sampler
      CSVs, measure and staging outputs, logs. Raw traces and logins stay private and out of Git.
      (22 Oct: `~/goalcup-campaign/` on the Mac holds cells, results and FINDINGS.md.)
- [ ] After each stage, update `docs/research/coverage-matrix.md` and `TODO.md` with measured facts
      (versions, platforms, pass lines met or missed). Send only aggregates to a cloud agent.
      (22 Oct: both updated for the test day, in the commit that ticks this file.)

## Open since 22 Oct

From the [local test day](results/2026-10-22-local-test-day.md) and its fix plans.

- [ ] Token-gated cells. The owner has not made the fine-grained GitHub token for the test
      repositories yet. Without it no test run, no full-flow cell and no judged-commit check ran.
      Still to run: test-run cells with drills through `campaign.py`, full-flow cells on the
      `gc-test-*` repositories, and the check of GitHub's activity API on staging.
- [ ] Push the website migration PR branch `goalcup/event-readiness-migration`. The website
      pre-push hook needs the owner's `SERGIO_OWNER_CHECKOUT` and `SERGIO_OWNER_PIN`.
- [ ] WSL2 cells through a Windows VM (or a Windows machine). WSL2 did not run on 22 Oct. The
      Windows side needs its own local agent, or SSH into WSL2 (LOCAL-TEST-DAY).
- [ ] Sign-in expiry during a run. Claude Code stalled when its token had expired at start (fix
      plan v6, Goal Cup's lock collided with Claude's refresh lock; fixed, not rerun). Measure
      Codex and Amp the same way: start or `continue` with an expired token, and an expiry in the
      middle of a long run. Codex is not fully ruled out in plan v6.
- [ ] Fix plan v9 owner review (draft, not merged): `goalcup mcp`, a read-only `time_left` tool
      the agent calls whatever its PATH. Approve, change or drop before merge.
- [ ] Rerun after fix plans v5 to v7, none rerun yet: Docker Codex answers `goalcup time-left`
      (v5); a Claude Code cell with an expired token starts alone (v6); a Mac Claude Code cell
      with the v7 settings gets its prompt.
- [ ] Rerun the macOS stop on mac-codex and mac-amp: it should seal `Confirmed`, not
      `Unresolved(ProcessAuthorityUnavailable)` (fix plan v2 K10, merged, not remeasured).
- [ ] A 40-minute practice cell on the merged kit ends with a PASS row verified against its
      server copy (fix plan v3 done-when). The 120-minute rows said FAIL for a kit reason, now fixed.
- [ ] Longer runs. Agents finished the 2 h workloads in 5 to 7 minutes and idled; use
      `campaign-long-workload.txt`. The 168 h sizes are linear projections from idle 2 h runs.
- [ ] Restore staging `PRACTICE_RUN_MINUTES=10` with the next normal deploy (it is 120 now).

## Open owner decisions

- Prep's filesystem check fails on overlayfs and btrfs, but the run itself never refused them (only
  the 3 GiB reservation decides). Warn instead of fail? Test one sandbox with `goalcup doctor` and a
  practice run first.
- Dry runs: RULES opens a dry-run window, but no doc says what a dry run is (a practice run, or a
  full-length run that does not count?). Define it. ([B9](PARTICIPANT-REVIEW-2026-10-21.md))
- Finish ideas in [HOW-A-RUN-ENDS](HOW-A-RUN-ENDS.md): judged-commit email, freezing the judged
  commit, hours-worked fact. Before 4 Nov or after?

- Hermes and Amp time-left reach: the guide now tells their users to mention `goalcup time-left`
  in their prompt. Also measure a skills folder for them, or leave it at that?
- Goal Cup's run HOME writes `.profile` with the PATH it handed the harness (fix plan v5 Q1),
  instead of `-c allow_login_shell=false` in the Codex connector. Plan v5 asks the owner to confirm.
- Docker `goalcup status` prints the PROBLEM "no sleep inhibitor on this machine" in every
  container. Reword it or drop it when there is no display session? (Plan v3.)
- Who reviews `StorageFull` gaps (RULES says judges never see raw traces)?
- Known limits to accept or fix: a `goalcup seal` typed elsewhere while the agent is paused for disk
  ends it with a hard kill; a `continue` refused with 410 still marks the run over.
