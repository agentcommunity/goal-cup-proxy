# Local next steps (after the 20 to 21 Oct cloud session)

What the owner and the local agent do on their own machines, in order, to take branch
`smart/zen-gates-d9tmbh` from "built and unit tested in a cloud container" to "measured and
released". Nothing here ran against real Docker, staging, GitHub or a real harness yet. Record every
result as you go (step 9). Background: the [readiness plan](../superpowers/plans/2026-10-20-event-readiness.md)
and the [local campaign](LOCAL-CAMPAIGN.md).

## 1. Review and merge

- [ ] Open a pull request from `smart/zen-gates-d9tmbh` to `main` (29 commits). One reviewer reads it
      once (AGENTS.md "Done"). Main areas: D64 (errors never stop a run), D65 (disk pause), D66
      (time left, no self-seal), parallel test runs, size limit, vitals, judged push, rename.
- [ ] Pull it locally and build: `cargo fetch --locked`, `cargo build --offline --workspace`.

## 2. Gate as a normal user (not root)

- [ ] `sh scripts/gate.sh fast`, then `sh scripts/gate.sh full`, and `sh scripts/linux-check.sh`.
- [ ] The cloud container ran as root, so 7 permission tests always failed there. They must pass here:
      `upload_chunks::retries` (2), `launch::tests::a_session_is_a_native_file_written_since_the_run_started`,
      `m2_proxy_integration::start_refuses_invalid_configuration_before_touching_the_sink`,
      `dir_modes::an_owner_denying_umask…`, `capture_native::finish_reports_discovery_it_could_not_complete`,
      `harness_contract_kit::every_kit_case_seals_and_matches_expectations`.
- [ ] These full-tier tests also failed in the cloud before and after the changes. Check them here:
      `capture_crash_campaign::torn_tail_is_preserved_and_nonresumable`,
      `capture_crash_campaign::daemon_exit_preserves_prefix_and_honest_recovery_limit`,
      `capture_crash_campaign::a_standalone_seal_that_commits_first_keeps_the_proxys_loss`,
      `launcher_sweep::the_launcher_sweeps_an_orphan_forked_after_the_root_died_before_the_seal`.
- [ ] macOS: the new `machine::process_usage` code was only type-checked. Run its test on the Mac.
- [ ] `npm --prefix server/worker test` (245). One run in five failed once under load in the cloud.
- [ ] `capture_lifecycle::a_seal_whose_registry_write_was_lost_is_committed_without_a_second_terminal`
      failed once in the cloud fast gate under load and passed 3 of 3 alone: watch it for flakiness.

## 3. Database migrations (website repo): one shared database

Five files in `server/worker/sql/pending/`, in this order. Apply **before** deploying the Worker
that needs them, or chunk uploads (size limit) and vitals heartbeats answer 502 until they land.

- [ ] `20261021100000_goal_cup_tracer_test_runs_parallel.sql` (test runs no longer replace)
- [ ] `20261021110000_goal_cup_tracer_run_size_limit.sql` (stored bytes, 64 GiB cap)
- [ ] `20261021120000_goal_cup_tracer_vitals_disk_notice.sql` (vitals, `disk_full` email)
- [ ] `20261021130000_goal_cup_tracer_final_push.sql` (judged push before the deadline)
- [ ] `20261021140000_goal_cup_tracer_notice_text.sql` (silence, disk and ended email wording)
- [ ] Staging and production share ONE database: applying a file applies it to both at once.
      There is no "staging first" for SQL. The old-signature wrappers keep the live production
      Worker working.
- [ ] First run all five on a throwaway Postgres loaded with the mirror (Docker `postgres:16`, stub
      `auth.users`), in order, inside one transaction.
- [ ] Copy each into the website repo `supabase/migrations/`; the lead agent applies them once,
      the owner approves. Regenerate the mirror (command in its header) and check it matches.
- [ ] Then deploy the staging Worker and test it; then the production Worker with the 26 Oct
      point release.

## 4. Server settings that could not be changed from the cloud

- [ ] `GITHUB_TOKEN` secret on production and staging (`wrangler secret put GITHUB_TOKEN`, and
      `--env staging`). Now required: the judged-push read uses GitHub's activity API.
- [ ] Check on staging with a real repo: the activity API answers with the token's permission, how
      soon a push appears, and how long history is kept.
- [ ] `TEST_APPROVAL_KEY` on staging only (exists). Keep the key file local, `chmod 600`.
- [ ] Test members for parallel cells: either rely on the parallel-test-runs migration (then set
      `"test_runs_parallel": true` in the campaign config) or add synthetic members (SQL in LOCAL-CAMPAIGN).
- [ ] Deploy the staging Worker after its migrations; `npx wrangler tail --env staging` while testing.
- [ ] Website: approval page wording for test runs (no longer "this replaces your current run"),
      and the `disk_full` email text. The 3 h / 6 h idle emails still fire while an agent is paused
      for disk: decide whether that is fine.
- [ ] `RUN_MAX_COMPRESSED_BYTES` is in `wrangler.toml` (64 GiB). Change only if measurements say so.
- [ ] Event ops (later): `EVENT_ID` switch on 6 Nov, deploy freeze 7 to 15 Nov, `MAINTENANCE=on` as
      the safe switch, a named on-call person per shift.

## 5. Machines and test kit

- [ ] Mac: `caffeinate -dimsu` during long runs, mains power, auto-updates off, allow the rebuilt
      `goalcup` in Little Snitch, `brew install zstd` if missing.
- [ ] Docker Desktop: raise its disk limit (several cells, about 3 to 5 GB each); cells use named
      volumes (ext4), never bind mounts on the Mac disk.
- [ ] WSL2: files inside the Linux file system (not `/mnt/c`), Windows Update restarts paused, WSL
      idle shutdown off, mains power.
- [ ] `python3 scripts/campaign.py check CONFIG`, then `build` once.
- [ ] Sign in once per cell: `campaign.py login CONFIG <cell>` (Codex rotates its login token, so
      never share or copy a login between cells). Cheapest settings are set by the scripts:
      Codex `gpt-5.6-luna` at `low`, Claude Code `haiku`, thinking off.
- [ ] Sandbox for participants: run `goalcup doctor` in one sandbox and send the output. goalcup refuses
      overlayfs, btrfs, ZFS, NFS, tmpfs and FUSE; a container sandbox may need an ext4 or XFS volume.

## 6. Measure what the cloud could not

- [ ] Claude Code on Linux: where the login file lands (the Docker login logs the names).
- [ ] Codex: does the installed version accept effort `minimal` for Luna?
- [ ] Hermes and Amp: is there a skills folder they load? Until measured, only Codex and Claude Code
      get the goal-cup skill (and its `goalcup time-left` line).
- [ ] Inside each harness: the agent runs `goalcup time-left` (answers from the server's count), and
      `goalcup seal` refuses inside the harness but works from another terminal.
- [ ] Disk pause (D65): on a small volume (`hdiutil create -size 6g -fs APFS …` on the Mac), fill the
      disk: the agent pauses below 1 GiB free, resumes above 3 GiB, the email arrives once.
- [ ] Vitals: `goalcup status` shows disk, memory and CPU; staging heartbeats store them.

## 7. Runs

- [ ] 10-minute smoke of each cell type (Mac host, Docker Codex, Docker Claude, WSL2).
- [ ] WP-0 measurements, no paid run: `scripts/research-measure.py` on the 18 and 19 Oct traces and
      `scripts/research-staging.py` on staging. Paste the output (aggregates only) back.
- [ ] Ladder on staging: 2 h, 8 h, 24 h, with the drills in LOCAL-CAMPAIGN.
- [ ] By 27 Oct: start the 168 h run, using the 26 Oct point release.

## 8. Release

- [ ] Point release around 26 Oct: `scripts/release.sh prepare <x.y.z>`, merge, full gate and
      `linux-check.sh` on merged `main`, then `release.sh tag <x.y.z>`. Verify the published installer
      in a fresh prefix.
- [ ] Rename step 4: rename the GitHub repo to `goal-cup-proxy`, then let an agent update the URLs
      and test that old install and release links redirect.
- [ ] Rename step 5 (database schema): decide before or after the event.

## 9. Record everything

- [ ] One private results folder per day: campaign `summary.json`/`summary.md`, cell reports, sampler
      CSVs, measure and staging outputs, logs. Raw traces and logins stay private and out of Git.
- [ ] After each stage, update `docs/research/coverage-matrix.md` and `TODO.md` with measured facts
      (versions, platforms, pass lines met or missed). Send only aggregates to a cloud agent.

## Open owner decisions

- Prep's filesystem check fails on overlayfs and btrfs, but the run itself never refused them (only
  the 3 GiB reservation decides). Warn instead of fail? Test one sandbox with `goalcup doctor` and a
  practice run first.
- Mid-run `/model` switch: the guide accepts it, RULES says only "keep going". Allow or forbid?
- Finish ideas in [HOW-A-RUN-ENDS](HOW-A-RUN-ENDS.md): judged-commit email, freezing the judged
  commit, hours-worked fact. Before 4 Nov or after?

- `GOALCUP.md` in the final version: required, or optional?
- Hermes and Amp time-left reach: measure a skills folder, or tell participants to mention
  `goalcup time-left` in their prompt?
- Who reviews `StorageFull` gaps (RULES says judges never see raw traces)?
- Known limits to accept or fix: a `goalcup seal` typed elsewhere while the agent is paused for disk
  ends it with a hard kill; a `continue` refused with 410 still marks the run over.
