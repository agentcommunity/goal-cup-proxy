# goalcup 1.0 build review, 2026-11-01 21:59 UTC

Independent reviewer, read only. Integration head `88329c47` (fetched 21:47Z). Read: the previous review (18:54Z), the ledger
to its last line (21:02Z), every lane log changed since 18:54Z (`masking.log` to 21:54Z, `connectors.log` to 19:58Z,
`verify.log` to 19:42Z, `pad.log` to 19:25Z, `pad-model.log`), `mrq/` (one `.held`, three `.res`), the three new gate logs,
CI on the new commits, the new reports (`gw2`, `gw3`, `hermes-parent`), `status/STATUS-REPORT.md` (19:04Z), the backlog
(20:32Z) and the stocktake (05:13Z, stale). Code read: the diff `a31a45e4..88329c47`, branch `v1.0/prd-05-masking` at
`0ae09590` (`mask/values.rs`, `mask/mod.rs`, the three CLI call sites, `pipeline/mod.rs:805-917`, `watchdog/mod.rs:210-330,
663-681`), branch `v1.0/prd-12-pad` at `f4047dd0` (`check.py`, `pad-run`, `table`, `hermes-cli.toml`), `cli/text.rs:315-330`,
`.github/workflows/release.yml`, PRD-00 done items, PRD-14 G5.

## 1 True state

1. Head `88329c47` (20:03Z). Three merges since review 5, each gate fast PASS (428, 425, 451 s) and CI green: `8eee2a70`
   PRD-08 slice 11 (Hermes subagents from delegation task logs), `9e5306ac` PRD-12 model pin, `88329c47` PRD-11 Hermes
   `state.db` drop. No row has run on any of them: `/root/gc-night/builds/` is empty.
2. Core rows passing all six on an integration build: still 4 of 9. gw3 (on `a31a45e4` plus an uncommitted local patch)
   shows the gateway works: Codex gateway all six PASS; Claude Code gateway smoke and full all but C3, which the owner made a
   PASS with note at 20:32Z (rig not changed yet). Hermes direct and gateway failed C3 only inside `state.db`, now dropped; no
   Hermes row since. Hermes dodger never run.
3. PRD-05 exact-value masking: the control-line blocker is fixed (worst start line 17.5 to 23.7 KB of 64 KiB), call sites
   switched (OWN-10), breaker stress clean at 21:54Z (40 and 200 streams of 1.5 MB, 32 values, 0 raw, 0 lost). Not merged: the
   lane stopped at 21:54Z on one docs finding (D2). The PRD-12 C3 package `192532` is `.held` to merge with it.
4. Phase 5: no full row for any of the 12 other harnesses, ever; `connectors/tiers.toml` says "not verified" for all 16.
5. Done items: 2, 3, 4 met. 1 at 4 of 9. 5 and 6 open. 7: no `server/` change since the staging deploy `e626de8f`, so a
   same-commit redeploy is a formality. 8 by tests only. 9 provisional.
6. Lanes: masking stopped (one docs finding). Pad waits on the package. Connectors, verify and pad-model finished. Participant
   idle since 14:21Z with the PRD-10 text seam unowned; core since 08:28Z; docs since 08:59Z. Last ledger line 21:02Z.
7. Spend: model about 27 USD billed (25.7 before, plus 1.18 gateway billed for gw3; pad estimate about 29). Gateway credit
   left 13.82 USD. Sandbox minutes not measured; still no `spend` line.
8. Time at 21:59Z: H+59.5. About 44 hours to the proposed freeze (Tuesday 3 Nov 18:00Z, still not adopted in the ledger);
   about 98 hours to the event switch (6 Nov 00:00Z).

## 2 Changes since last review (18:54Z)

- Owner: 18:56Z Hermes re-decided: keep `state.db` only until a reader of subagent links from other files lands, else ship
  "subagents not verified", said plainly. 19:17Z and 19:39Z W5: Vercel AI Gateway on a paid team, the DeepSeek key added as
  BYOK. 19:31Z: every Claude Code test runs on the pinned cheap model (conductor error: full1 ran on the default, 7.33 USD in
  22 minutes). 20:32Z: a deltas-only shape hit with 0 known hits is C3 PASS with a stated-loss note. 20:35Z: sandbox rule
  (one build per integration commit, gates only on real merges, at most 2 pad boxes, batches on a frozen head, no probe boxes).
- Conductor acted on review 5 at 18:56Z: corrected the Hermes premise, put PRD-05 on HOLD until the blocker fix and the C3
  package, relabelled 15:06Z, held and then discarded `182615`. Investigation `hermes-parent` (19:03Z): no other Hermes file
  carries session-id links; manifests and task logs show 5 of 5 subagents in full3 and full4.
- Merged: PRD-08 slice 11 (`HermesTask` and `HermesManifest` readers in `usage/hints.rs` and `usage/native.rs`, coverage text
  in `legs.rs`); PRD-12 pin (a `model` and `env` per case in the harness files, `pinned_model` in the row, a warning line on a
  mismatch, rates for the pinned ids); PRD-11 drop (state row gone from `connectors/hermes-cli.toml`, first-screen text, facts
  sheet section 4, OWN-9 test flip).
- Runs: gw2 19:20 to 19:33Z (every call 403, free tier, 0 USD); gw3 19:36 to 20:30Z (gateway capture works on 4 rows).
- PRD-05: `c5237a10` (dedupe and cut before send, `.env` values first, model-id and host-like values refused), then six fix
  rounds: JSON-escaped and HTML-safe forms (11 to 15 forms), lossy UTF-8 key files, call sites to `launch_values`
  (`d1cab71e`), recover reads `/proc/<pid>/environ` (`0ae09590`, Linux only).
- PRD-12 package (`f4047dd0`): planted shaped values moved to names the gate refuses (`PAD_OPENAI` and so on) so the shape
  rules stay tested; with exact values on, a plain hit of `PAD_SECRET_PLAIN` is a FAIL (review 5 item 4 closed); a
  masked-upstream PASS only for the harness file's `redacts_kinds`, and it does not count for `--release-check` until the owner
  confirms.
- Review 5's top 5: step 1 nearly done ((a) and (b) built, (c) participant text not started, (d) not run). Step 2 done. Step 3
  (phase 5) not started. Step 4 partly (15:06Z relabelled, lane branch noted; NIGHT-LOG, MORNING, OWNER-DECISIONS, `spend`
  line not done). Step 5 done (gateway works).

## 3 Discrepancies between claims and evidence

1. **"No GAP was written, no byte was lost and the relay was not blocked" (masking 21:02Z; PRD-05 section 11 on the branch)
   holds for running streams, not for boot.** The recorder builds its masker before it takes the listener and before `ready`
   (`pipeline/mod.rs:820`, then `:917`). The watchdog gives the first boot 10 s (`watchdog/mod.rs:223` `READY_NS`), then kills
   it and the harness is never started (exit 3). Measured build with 32 values of 256 B: 2.0 to 6.4 s in a box. Every restart
   boot rebuilds before it accepts, so a kill gap grows by the build time, while done item 2 bounds kill gaps at 2 s. The CLI
   also compiles every value once more only to check the rules (`start.rs:226`, `continue_run.rs:200`, `recover.rs:202`),
   though a value can never make `Masker::new` fail. Resident memory is 238 MB at 32 x 256 B against a 64 MiB target. The pad's
   `.env.local` has one gated value, so no pad row will ever show any of this.
2. **Owner rule 18:56Z: "if no other file carries the links, Hermes ships 'subagents not verified'".** The investigation
   answered that no other file carries them. The build now passes `subagents_present` for Hermes when every manifest task has a
   task log, and the first screen says subagents "are shown by their task logs". The PRD-16 note calls this proof "weak by
   design"; the investigation called it "close to tautological". It may be a fair reading, but the owner was not asked.
3. **"Codex via gateway PASS all six" (gw3)** ran on the model in an uncommitted patch. The merged pin (`codex-cli.toml`
   `[gateway] model`, `gateway/codex-config.toml:3`) is a different OpenAI model, priced 4 to 20 times higher, that no gateway
   row has run. None of the gw3 passes is on an integration build.
4. **Participant text will be wrong once PRD-05 merges.** `cli/text.rs:320-330` (section 12) still says only "your project's
   GitHub token" is masked by value and lists "a line of `cat .env`" as recordable; README.md:240 and PARTICIPANT-GUIDE.md:413-416
   say the same. The "N key values were not added to masking." line exists nowhere in `crates/` on either branch, so a
   participant whose values pass the 32 cut is never told (point 6). The PRD-10 seam (masking 18:25Z and 19:19Z) has no lane.
5. **The masking "final round" added new code** (`process_env`, 21:33Z) and then needed another round. Since the 20:35Z cost
   rule the lane ran about 6 boxes in 75 minutes for fix and re-check rounds whose findings were mostly nits. The lane has now
   stopped on a docs finding; nothing merges unless the conductor acts.
6. **A clean Hermes row may still not PASS C3.** In gw3 the planted `private_key` lines were never masked anywhere. The
   package counts a kind outside `redacts_kinds = ["openai_key", "github_token"]` with no MASK as "not exercised"
   (INCONCLUSIVE), and a masked-upstream PASS needs an owner yes to count for the release. If so, this is the fifth time a
   ledger expectation and `check.py` differ.
7. **Record.** NIGHT-LOG.md ends at 13:01Z, MORNING.md is the 09:59Z interim, OWNER-DECISIONS.md is unchanged since 30 Oct;
   six owner answers or rules since 17:44Z exist only in the ledger. `status/STATUS-REPORT.md` (19:04Z) says the gateway is
   blocked and Hermes `state.db` is copied, both no longer true. The backlog still lists V1 as open, but `honesty.rs:384`
   already skips zero-byte tunnels. D1 is queued "after the package", yet the held package edits the same `check.py` lines and
   still marks that case INCONCLUSIVE.
8. **A merge ran in the primary checkout again** (verify merger, 19:09Z, "cd failed"); it sits detached at `786d68c5`. Sixth
   time.

## 4 Top 5 next steps, in order

**The single most important step is 1.** Under the owner's batch rule every remaining row, phase 5 included, waits for this
head. The work is done except a docs nit and two small point 3 changes.

1. **Merge the masking package tonight, in one window. Who: conductor (merges, docs nit), core or masking lane (two small
   changes), pad lane (fold). Size: about 1.5 hours, two gates.**
   (a) Take D2 as a docs fix by the conductor or as a listed nit. Do not start another review round.
   (b) Point 3, before or right after the merge: the three CLI checks call `Masker::new(&plan.mask_rules, Vec::new())` (only
   the rules can fail), and the first-boot ready wait covers the build (raise `READY_NS` to about 30 s). Write the restart-gap
   cost into PRD-02 and PRD-05 as a stated cost with the measured number. Only if a Mac measure exceeds about 2 s: build the
   value forms in the mask worker after `ready` (it is off the relay).
   (c) Fold D1 (deltas-only PASS with note), D2 (the gateway rate at the cheap tier; without it the estimate is 3 times the
   bill and A2 would hit its estimate cap early) and D3 into `192532` before it merges. One gate and one build, not two.
   (d) Then freeze PRD-05: no new forms, no new readers.
2. **One build, first batch on the new head. Who: conductor dispatches pad agents. Size: about 1.5 hours, 2 boxes, about 3
   USD.** Claude Code direct smoke on the pinned model (confirms `pre_keys` with a model set, the recorded model id,
   `mask_values.workspace` at least 1, a `run_secret` MASK, plain hits 0) and Hermes direct full (first row without `state.db`:
   C2 from task logs, C3). Before it, at no cost: replay the package's `check.py` on the full3, full4 and gw3 Hermes folders to
   see which C3 lines a clean Hermes row will get (section 3 item 6), and settle the rule before spending. Then Hermes dodger
   full and Hermes gateway smoke.
3. **Phase 5 batches right after. Who: conductor, pad agents; the verify lane in parallel on V2 (OpenClaw orphans). Size: about
   6 hours of 2 boxes, about 10 to 15 USD.** Order: Gemini, Copilot, OpenCode, Kilo, Qwen, opencode2; then Goose, OpenClaw,
   Crush, Cline and Kimi with their known causes stated. Write `tiers.toml` from the rows (done item 5). These rows need not
   rerun on the final head (PRD-16 section 6).
4. **Participant text and the record. Who: participant lane (PRD-10 `text.rs`), docs lane (guide, README, SECURITY), conductor
   (record). Size: about 1.5 hours.** Section 12 text for exact values; the "N key values were not added to masking." line on
   start, `goalcup status` and the end screen; the Hermes line. Scribe NIGHT-LOG, refresh MORNING, add the `spend` line with box
   minutes, write the freeze time into the ledger, refresh or remove STATUS-REPORT.md.
5. **macOS now, not at the release. Who: conductor; owner for one Mac run. Size: one manual `release.yml` run (GitHub runners,
   no sandbox, about 1 hour) and 30 minutes of owner time.** In 1.0 the 24 `cfg(target_os = "macos")` blocks have never
   compiled: CI is `ubuntu-24.04` only, and `release.yml` last ran for 0.7.0 on 26 Oct. A manual start builds and runs the
   Rust guards on both Mac targets and publishes nothing (`release.yml:11-12`). That is PRD-14 G5 brought forward. Then one Mac
   smoke by the owner before the freeze.

Then: the freeze; the final core set (9 rows and the Codex kill row) on the frozen head with gate full; A1 to A4 once the caps
are answered; PAD-RESULTS.md, ACCEPTANCE.md and the final MORNING.md.

## 5 Cut, defer, reorder

- **Cut:** further PRD-05 review rounds and forms; F2 (mixed nested encoding) stays a named loss. The recover `/proc` reader
  can stay (the breaker passed it), but no more recover work.
- **Cut:** Hermes extras (more manifest checks, shell hooks, a wire-side link).
- **Reorder:** D1 to D3 into the held package. G5 (Mac guards) before the freeze. Phase 5 starts on the masking head, not
  after the freeze.
- **Decide once:** the Codex gateway model. Keep the merged OpenAI pin and run it, or return to the cheaper model gw3 proved.
  Not both.
- **Defer as stated costs:** masker speed steps (dispatch table, pool) and the 238 MB, unless a Mac measure says otherwise.
- **Keep deferred:** source 4, wire-learned values, joining deltas in the recorder; the rest of PRD-02 slice C part 2, PRD-09
  `usage-job.sh`, PRD-14 `e2e_run.rs` and G2, the 5 post-cut gaps, and the harness items that wait on the owner.
- **Keep cut:** per-merge replan commits, provenance rewording, more PRD-13 doc rounds, rig features beyond C3.

## 6 Risks

1. **Point 3, recorder boot with many long values (medium to high, new).** Section 3 item 1. Past 10 s the harness is not
   started; restarts add seconds to each gap. A small group of participants, a severe effect, and invisible to the pad.
2. **Point 7, macOS (high, under-weighted).** No 1.0 Mac compile or run. Most participants probably use Macs. The Linux
   paths have Mac branches that nothing has run (`spawn.rs:253` uses `ps`; `clock.rs:72` calls its Mac boot id "unverified");
   the recover `/proc` reader is a stated loss there.
3. **Point 7, breadth (high).** 12 harnesses at 0 full rows, 44 hours to the freeze, 2 pad boxes at a time: about 6 to 8 hours
   of pad time with little room for fix loops. Any slip in step 1 eats it.
4. **Point 2 (medium to high).** Hermes subagents are now proven by task logs only, not tied to calls; the owner's wording is
   open. No row has had more than 5 subagents; A1 and A2 wait on caps.
5. **Point 4 (medium).** Hermes task logs are free text and their content is "not measured" (facts sheet section 7);
   headerless PEM lines are never masked; OpenCode and OpenClaw keep SQLite copies of the same kind (section 12 names a
   database read whole through `{db}`; keep it that way).
6. **Cost (medium).** The pad estimate prices the gateway cheap model at 3 times the bill until D2, and the 10 USD per-run cap
   is checked on the estimate. Gateway credit 13.82 USD; an empty balance reads 403 and INCONCLUSIVE. Sandbox minutes are not
   measured, so the 75 and 150 USD sandbox lines (PRD-00 defaults) cannot fire.
7. **Point 6 (medium).** After the merge the participant text understates masking and never shows the cut count (section 3
   item 4).
8. **Process (medium).** The owner's record is 9 to 12 hours behind; lanes stop on nits and wait for the conductor; the
   primary checkout takes merges; the freeze time is not adopted; a stale status report sits beside the reviews.

## 7 Owner asks pending

1. **New: Hermes subagents.** No other file carries the links. The build shows each subagent by its task log, not tied to
   calls, and passes C2 on that. Confirm this meets the 18:56Z rule, or ship "subagents not verified".
2. **New: the masked-upstream C3 PASS** (a harness that redacts tool output itself; today Hermes). Yes or no. Without a yes
   such a row does not count for the release check.
3. **New: per-run cap for long runs** (19:31Z proposal: 15 USD per 2-hour run on the cheap model) and whether A1 and A2 run
   before Friday.
4. **New: the freeze time** (proposed Tuesday 3 Nov 18:00Z), one Mac run by the owner before it, and an OK to run `release.yml`
   by hand (GitHub Mac runner minutes, nothing published).
5. **New: masker cost.** Accept as stated (up to 238 MB and a few seconds at start with 32 values of 256 B), or lower the guard
   to 16 values.
6. **Record in OWNER-DECISIONS.md:** 18:04Z sources 1 to 3 and the Stripe rules; 18:56Z Hermes; 19:17Z and 19:39Z gateway;
   19:31Z model pin; 20:32Z deltas-only; 20:35Z sandbox rule. Agents may not edit it.
7. **Gateway credit:** 13.82 USD left; top up before the final gateway rows and A3 and A4.
8. **Still open:** S6 scope (a planted fake value stops only that harness's rows); the Gemini CLI 0.66 subagent Shell prompt; W1,
   rotate the two real keys named at 07:24Z; rung 5 setup L3; the size overruns (Rust src about 2.2 times PRD-00, PRD-05 and
   PRD-08 far over); the `gsk_` and `sgamp_` rules; PRD-11 setup switches; a new REPORT_AT; run 1's prompt in a file; the cron
   email on `failed`; question 12; the release steps (production deploy and secret removal, merge to main, tag).

Answered since review 5: Hermes (18:56Z), W5 gateway (19:17Z, 19:39Z), deltas-only (20:32Z). New owner rules: model pin
(19:31Z), sandbox use (20:35Z).
