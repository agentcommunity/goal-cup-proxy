# Review of the four finishing plans: correctness and rules

Reviewer: independent, read-only (no build, box, push or edit). Read on `origin/v1.0/conductor-handover` at `e9dfd3b7` (plans, conductor docs and tools), `origin/v1.0/integration` at `04044320` (the code the plans touch) and `origin/v1.0/prd-05-masking-exact-trie-matcher` at `21419832` (the matcher). Line numbers below are from those commits.

Summary of what holds: nearly every file path, function, line range, test helper and command the plans name exists where they say (checked: `scripts/pad/pad` `KEYS:48`, `load_settings:275`, `apply_kill_minutes:329`, `step_list:444`, `step_fetch:685`, `meta.json:836-845`, `main:947`; `pad-run` `merge_config:48`, `setup:252-290`, `exit_harness:566-583`; `check.py:214`, `:387`, `:640-648`, `:867-881`; `table` `tier_for:60`, `release_check:126`; `test_pad.py` helpers; `selftest` writing `pad-out/selftest.json` with `commit`, `all_ran`, `exit`; `release.sh:62-70`; `release.yml:58`, four build targets and a `release` job that skips on dispatch; `wrangler.toml:36-42`; `start.rs:26`; `legs.rs:446`; `fetch-run.sh` heredoc `BEGIN READ ONLY` to `ROLLBACK`; `sbx.mjs` commands; on the matcher branch `RunSecrets::new:254`, `build:289`, `find_all:450`, `spawn:482`, `LiteralWalk:491`, `exact_candidates:392`, `Policy::claims:782` with `base = self.len() - self.secrets.len()`, `mask_bench.rs:70/237/318`, PRD-05 `:537` and `:1206`; all draft decision times appear at a line start of the committed ledger). The kill-minute mapping (`apply_kill_minutes`) only fires when both `@8` and `@10` are present, so A2/A5 keep `[10, 30]` and `kill-codex.toml` maps to `[3, 5]` as the tests expect. The `merge_config` TOML/JSON paths give exactly what the `KeyFileMerge` tests assert. The findings below are what does not hold.

## Blockers (the slice box or the wave cannot pass as written)

### B1. The box cannot see `origin/v1.0/integration`, so `red-green.sh` has no base
- Plan, task, step: handover Task 10 Step 4; long-runs Task 5 Step 2.
- What is wrong: both run `bash $C/test/red-green.sh origin/v1.0/integration` inside the box, and the handover plan asserts "the bundle carries `origin/v1.0/integration` as `refs/remotes/b/...`". It does not. `box.mjs` bundles only one temporary ref (`conductor/box.mjs:52-54`: `update-ref refs/box/job-...` then `git bundle create bundle tmpRef`), and the box fetches `+refs/*:refs/remotes/b/*` (`:64`), so the only name in the box is `refs/remotes/b/box/job-...`. Ancestor commits are in the bundle (so the matcher plan's checkout of `04044320` works), but no branch name is. The fallback "takes the merge base of HEAD with it when the name is missing" cannot run without the ref either.
- Correction: resolve the base on the conductor and put the sha in the job: `BASE=$(git -C "$WT" rev-parse origin/v1.0/integration)` then `bash $C/test/red-green.sh $BASE`. Specify `red-green.sh BASE` as taking a commit (sha or name) and iterating `git rev-list --reverse BASE..HEAD`. Same change in long-runs Task 5 Step 2 (`rev-parse origin/v1.0/integration` in `$GC_WT/wave-a-check`). Fix the false sentence in the plan.

### B2. "One gate on the combined head" is not what the queue steps do
- Plan, task, step: mini Task 8 Step 4 (the lane writes its own `.req`); long-runs Task 5 Step 3 ("two `.req` files"); matcher Task 3 Step 4 ("queues it with the other code-wave lane branches through `mergeq.sh`"); README Wave 2 and the box budget ("1 gate").
- What is wrong: `mergeq.sh` claims each `.req` and runs `merge.sh` for it alone (`conductor/mergeq.sh:20-31`), and every `merge.sh` run is one gate box (`merge.sh:50`). Four requests are four gates, and the first lane to write a `.req` is merged on its own as soon as `mq-driver.sh` ticks. The train mechanism exists but is not used: `merge.sh` takes a comma-joined branch list and merges all of them before the one gate (`merge.sh:26-30`).
- Correction: in all three plans, lanes write NO `.req` for the code wave; they push and log `ready for the train`. The conductor writes ONE request `{"branch": "v1.0/prd-05-masking-exact-trie-matcher,v1.0/prd-12-mini-rig,v1.0/prd-12-release-rows,v1.0/conductor-longsum", "message": "code-wave train: ...", "refresh_lock": false}`. Bisect = re-queue a comma-joined subset. The same applies to the fix train (mini Task 11 Step 3). The handover branch stays its own single request (handover Task 10 Step 7 is right).

### B3. `install.sh --skip-npm` exits 5 whenever there is no `node_modules`, and two tests hit that
- Plan, task, step: handover Task 3 (Interfaces: `install.sh` "runs `patch-sandbox.sh DIR/node_modules`"; `patch-sandbox.sh` "exit 5 ... when ... the file is missing"); Task 8 Step 1 (`test_handoff.sh`: `install.sh --skip-npm` from `$C`, and `t_restore_keeps_scribed` from a fresh worktree `$d/new/$C`); Task 10 (`run-all.sh` runs `test_*.sh` in name order).
- What is wrong: `test_handoff.sh` sorts before `test_install.sh`, so when it runs in the clean clone `$C/node_modules` does not exist (only `test_install.sh` creates the fake SDK there). `install.sh --skip-npm` copies nothing, then `patch-sandbox.sh "$d/tools/node_modules"` finds no `base-client.js` and exits 5, install fails, and `t_handoff_bundle`, `t_handoff_trailer`, `t_handoff_keyscan_refuses`, `t_restore_keeps_scribed`, `t_restore_never_overwrites` all fail. `t_restore_keeps_scribed` fails even in isolation (a git worktree has no `node_modules`).
- Correction: specify in Task 3 Interfaces: "with `--skip-npm` and no `DIR/node_modules/@vercel/sandbox`, `install.sh` prints `patch skipped: no node_modules` and exits 0; the patch runs only when the SDK is present". Add that case to `test_install.sh` (`t_skip_npm_without_sdk_ok`). Expected counts become `test_install 7`.

### B4. `t_no_model_ids` fails on the file the owner's format requires to name model classes
- Plan, task, step: handover Task 9 Step 1 (`t_no_model_ids` greps `NEW-SESSION-PROMPT.md` for `haiku|opus|sonnet`); Task 9 Interfaces (`NEW-SESSION-PROMPT.md` rewrite) and the owner's "EXECUTION AS WORKFLOWS, MODEL PER TASK" (WRITING-PLANS.md:109-114) and "Claude Code tests use Haiku only" (committed prompt lines 7 and 16).
- What is wrong: the prompt must tell the new conductor which model class runs which task and that Claude Code rows run Haiku; the committed prompt says "sonnet", "haiku" and "Haiku" (2 matching lines today). As specified, the test is red at Task 9 Step 4 and the slice box fails, or the prompt drops an owner rule to pass.
- Correction: exempt `NEW-SESSION-PROMPT.md` from `t_no_model_ids` (as the plan already exempts the owner-quote draft), or word the classes as "most capable / mid-size / smallest" and point to `scripts/pad/harnesses/claude-code-cli.toml:24-30` for the pin. Make the same exemption explicit in the README for `docs/superpowers/plans/*` (see N1).

### B5. `t_handoff_trailer` always fails
- Plan, task, step: handover Task 8 Step 1: `git -C "$d/repo" log -1 --format=%B origin/v1.0/integration | tail -1 | grep -qx "Claude-Session: $S"`.
- What is wrong: `--format=%B` prints the body with its trailing newline and git adds the record separator, so the last line is empty (verified on this repository: `tail -1 | od -c` prints a lone `\n`). `grep -qx` never matches.
- Correction: `git -C "$d/repo" log -1 --format='%(trailers:key=Claude-Session,valueonly)' origin/v1.0/integration | grep -qx "$S"`, or `--format=%B | grep -qx "Claude-Session: $S"`.

## Fix (wrong or missing, does not stop the first box)

### F1. Two layouts for the same files: the plan's `ledger/`, `reports/`, top-level tools versus the committed `state/` and `tools/`
- Plan, task, step: handover File Structure, Task 1 (modifies `$C/merge.sh` etc. at the top level, creates `$C/mq-driver.sh` "from `/root/gc-tools`"), Task 8 (`handoff.sh` copies to `$C/ledger/<lane>.log`, `$C/reports/<folder>.txt`, `$C/FINISH-PLAN.md`), Task 9 (`t_draft_cites_ledger` reads `$C/ledger/integrator.log`); Task 1 test `t_no_root_paths` globs `$C/*.sh $C/*.mjs $C/*.js $C/*.cjs $C/*.py $C/dbfetch/*.sh`.
- What is wrong: commit `9606cf4a` already put the full tool set in `$C/tools/` (box.mjs, gc-lane.js, merge.sh, mergeq.sh, mq-driver.sh, padrun.sh, padbox.sh, dbfetch-idx.sh, dbfetch-install.sh, keyscan.py, scrub.py, package.json, package-lock.json, patched-base-client.js, proxy-preload.cjs, handoff.sh, handoff-head.md, scribe.sh, build-snapshot.id) with byte-identical copies of six of them at the top of `$C`, the ledger at `$C/state/lanes/*.log(.scribed)`, reports at `$C/state/reports/*.txt`, and `state/FINISH-PLAN.md`, `state/reviews/LATEST.md`, `state/next-pad-verify.md`, `state/STATUS-REPORT.md`. TOOLS.md:3 calls `tools/` "the complete set". The plan never mentions `state/` or `tools/`, so after Task 8 the repo holds the ledger and every tool twice, `t_no_root_paths` passes while `tools/*.sh` still carry `/root/...` and the old session URL, and `TOOLS.md` points readers at the stale copies.
- Correction: decide once in the File Structure. Recommended: keep the plan's layout and add to Task 8 Step 3 `git rm -r $C/tools $C/state` with `state/lanes/*` becoming `ledger/`, `state/reports/*` becoming `reports/`, `state/FINISH-PLAN.md`, `state/reviews/LATEST.md`, `state/next-pad-verify.md` moving to the plan's names (a `git mv`, so history stays); rewrite TOOLS.md:3 and :27-35 (the manual install steps are replaced by `install.sh`). Extend `t_no_root_paths` to `find "$C" -type f \( -name '*.sh' -o -name '*.mjs' -o -name '*.js' -o -name '*.cjs' -o -name '*.py' \) -not -path '*/node_modules/*'`.

### F2. `rowinfo.py` exists only on the old machine
- Plan, task, step: mini Task 9 Interfaces ("Consumes ... `/root/gc-night/gw3/rowinfo.py`") and Row procedure step 2 (`python3 /root/gc-night/gw3/rowinfo.py <run folder>`), used by every row of Tasks 10, 11 and the long-runs plan's reading.
- What is wrong: it is not in `$C/tools/` on the branch (`git ls-tree -r origin/v1.0/conductor-handover | grep -i rowinfo` prints nothing) and no handover task copies it. A fresh container cannot read a row the way the plan says.
- Correction: handover Task 5 gains `$C/rowinfo.py` (copied, no literal paths), listed in Task 8's `handoff.sh` copy list, `install.sh`'s copy and `t_no_root_paths`; mini Task 9 calls `python3 "$GC_TOOLS/rowinfo.py"`.

### F3. The owner-decisions draft stops before the Hermes `state.db` outcome and leaves a superseded line standing
- Plan, task, step: `conductor/owner-decisions-draft.md` lines 8 and 9 (18:19Z: "The PRD-11 state.db drop branch is NOT merged (parked ...)"; 18:56Z: "Investigation running"); handover Task 9 Interfaces (the draft's time list ends the Hermes thread at 18:56Z).
- What is wrong: the ledger continues: 19:04Z (no other Hermes file carries parent links; delegation `manifest.json` and `task-<n>.log` show every subagent), 19:42Z (`state.db` drop UNPARKED under the 18:56Z rule), 20:12Z (merged as `88329c47`: "finish Hermes state.db drop"), 22:03Z (C2 passes on task logs). STATE.md:9 and mini Task 10 ("first proof of the `state.db` drop") rely on the merge. A reader of the draft alone, which "binds" the new session per NEW-SESSION-PROMPT.md:11, concludes `state.db` is still captured.
- Correction: add one draft line "19:04Z finding and 19:42Z: under the 18:56Z rule, no other file carries the links, so the `state.db` drop was unparked and merged at 20:12Z (`88329c47`); Hermes subagents come from task logs (22:03Z)". Mark the 18:19Z line "superseded by 18:56Z and 19:42Z". Add 19:42Z to the Task 9 time list and to `t_draft_cites_ledger`'s expectations.

### F4. Wrong ledger citation for the Codex gateway pin
- Plan, task, step: mini Task 5 cause text and `test_codex_data` comment ("anthropic/claude-haiku-5.5, owner 22:27Z"); handover Task 9 Interfaces ("the four answered owner asks (22:05Z, 22:03Z, 22:27Z, 20:32Z)").
- What is wrong: every 22:27Z ledger line is the masking merge (`gate`, `merge`, "MASKING PACKAGE MERGED"). The pin is the 22:00Z conductor line ("Codex gateway model to be pinned to the tested anthropic/claude-haiku-5.5"), resting on the owner's 19:17Z (gateway rows on the pinned cheap model) and the 20:27Z PASS. It is not itself an owner answer.
- Correction: cite "19:17Z owner rule, 22:00Z conductor note, 20:27Z gw3 PASS" in Task 5 and the test comment; in Task 9 replace 22:27Z with 22:00Z or drop it from "owner asks".

### F5. The matcher plan runs the in-repo `box.mjs`, which has no `node_modules`
- Plan, task, step: matcher Task 2 Step 3 and Task 4 Step 4: `node docs/plans/2026-10-30-v1.0/conductor/box.mjs run ...`; also WRITING-PLANS.md:94 (project note) and matcher Tech Stack.
- What is wrong: `box.mjs:11` does `import { Sandbox } from '@vercel/sandbox'` at load; the repo copy has no `node_modules` (`.gitignore`), so the command fails before any box. The runnable copy is the installed one (`/root/gc-tools/box.mjs`, `$GC_TOOLS` after the handover plan), as the mini and long-runs plans use.
- Correction: `node "$GC_TOOLS/box.mjs" run --repo <wt> --ref ... --script /root/gc-night/jobs/trie-box.sh --minutes 30` (and `--vcpus 4`: a release build of one crate does not need 8).

### F6. An em dash in the long-runs plan
- Plan, task, step: long-runs Task 10 Step 2, line 308: `` `grep -c '—' ACCEPTANCE.md` ``.
- What is wrong: the rule in the same plan ("no em dashes") and the handover `t_no_em_dash` pattern; the other plans use the byte form.
- Correction: `grep -c $'\xe2\x80\x94' docs/plans/2026-10-30-v1.0/ACCEPTANCE.md` is 0.

### F7. The README's open-decision list drops the ask that can refuse the tag
- Plan, task, step: README "Owner decisions still open" (13 items) against STATE.md asks 6, 16 and 17; long-runs Task 8 Steps 3 and 7.
- What is wrong: `table --release-check` rejects any relied-on row whose `facts.masked_upstream` is set (`scripts/pad/table:142-146`), and Hermes direct is a T8 release blocker. STATE.md ask 6 ("does a masked-upstream C3 pass count for the release check?") is not in the README list. Also missing: ask 16 (copy the draft into `OWNER-DECISIONS.md`) and ask 17 (Amp and Factory accounts, which decides two "not verified" rows). With exact-value masking the planted `.env` values should now be masked by goalcup itself, which would clear `masked_upstream`, but no plan step checks that on the first Hermes row.
- Correction: add asks 6, 16 and 17 to the README list; in mini Task 10's Hermes row add "expected `facts.masked_upstream` false (the plants are masked by exact value), else owner ask 6 decides the release row".

### F8. TOOLS.md describes a hold mechanism the queue does not have
- Plan, task, step: TOOLS.md "Merge queue protocol" ("A hold is a `.held` file or a removed request; the tick must skip held branches") and lesson 6; handover Task 1 (modifies `mergeq.sh` without adding it).
- What is wrong: `mergeq.sh` has no `held` logic at all (grep), so a `.held` file does nothing; only removing the request holds it. A new conductor following TOOLS.md would believe a branch is held while the tick merges it (the exact failure lesson 2 and 6 record).
- Correction: either add to handover Task 1 "mergeq skips `<id>.req` when `<id>.held` or `<branch-with-slashes-as-underscores>.held` exists" with test `t_mergeq_skips_held`, or change TOOLS.md to "a hold is the request moved to `mrq/held/`".

### F9. The new-session prompt tells a fresh container to merge the tools branch itself
- Plan, task, step: NEW-SESSION-PROMPT.md step 1 ("if `v1.0/conductor-handover` is not yet on integration, merge it first with a merge commit"); handover Global Constraints ("The tool commits reach integration only through `merge.sh` (they are not docs-only)").
- What is wrong: the branch carries `.sh`, `.py`, `.mjs` files, so by the plan's own rule it needs the gate; a fresh container has no tools installed yet to run `merge.sh`, and a local merge pushed to integration would skip the gate (lesson 5).
- Correction: "If the handover is not on integration, install the tools from the handover branch, then queue its merge through `merge.sh` (one gate); never push a local merge."

### F10. H1 is built from the primary checkout without a fetch
- Plan, task, step: mini Task 9 Step 0 (`--repo /home/user/goal-cup-proxy --ref <H1>`) before Step 1's fetch; `box.mjs:50-55` writes and deletes a temp ref in that repository.
- What is wrong: `<H1>` is the train head just pushed by the gate box; the primary checkout only knows it after `git fetch`, so `rev-parse <H1>^{commit}` fails. TOOLS.md lesson 7 also says to keep hands off that checkout (a temp ref is not the working tree, but the plan should say which repo it touches).
- Correction: `git -C /home/user/goal-cup-proxy fetch -q origin` first, or run Step 0 after Step 1 with `--repo /root/gc-wt/mini --ref HEAD`.

### F11. `t_restore_keeps_scribed` and `fresh-clone.sh` depend on F1's layout
- Plan, task, step: handover Task 8 Step 1 and Task 10 Interfaces (`fresh-clone.sh` asserts `$HOME/gc-night/lanes/integrator.log` exists when `$C/ledger/` exists, else prints `no ledger yet`).
- What is wrong: with the committed `state/lanes/` already present but `ledger/` absent, the slice box prints `no ledger yet` and asserts nothing, so the restore path is never proven against the real ledger until after the merge.
- Correction: resolve F1 before the slice box so `$C/ledger/` exists at Task 10 Step 4 and the restore assert runs.

## Nits

- N1. The plans state "no model names or ids in repo docs" and then name `sonnet`, `haiku`, `claude-haiku-5-5`, `anthropic/claude-haiku-5.5`, `deepseek/deepseek-v4.1-flash`, `gpt-6.1-sol` in prose (mini line 16, long-runs line 19 and Task 7 Step 4, every Execution table). The owner's format requires model classes per task, so say so once in the README ("plans name model classes under the owner's format; model ids appear only as references to pad data files") and prefer `scripts/pad/harnesses/<id>.toml` references over bare ids.
- N2. Handover Task 10 `red-green.sh` "runs the one test file that commit added": Task 1's red commit adds `test/t.sh` and `test/test_lib.sh`; say "the `test/test_*` file".
- N3. Handover Task 9 "rulings `OWN-1` to `OWN-11`, next `OWN-12`": the ledger has OWN-1, 2, 3, 5 to 11 (no OWN-4).
- N4. Task 9 says each draft line "ends with its ledger time"; the committed draft starts with it. Pick one so `t_draft_cites_ledger` and readers agree.
- N5. Mini Task 1 Interfaces: say `cap_usd` is "a number above 0, not a bool" (the test includes `("cap_usd", True)`; `True` is an `int` in Python).
- N6. STATE.md "Next step" says "run `snaps.mjs`"; it is not in `tools/` (TOOLS.md:25); the handover plan replaces it with `box.mjs snaps`.
- N7. Matcher Global Constraints: "The quote cut keeps a value's own `"` (and the `\` before it) raw" contradicts the passing tests `json_masked` (`mask_values.rs:514-532`) and `a_value_with_html_bytes_is_masked_in_html_safe_json` (`:600-627`), which assert the quoted forms fully starred; the plan's Review Focus 1 test asserts the same. The sentence is wrong, not the test; reword or drop.
- N8. Mini Task 7 cites `cline-cli.toml:9-12` for the "Not verified" comment; it is lines 5-10, and `not_verified` is line 20.
- N9. Mini Task 12 Step 2 runs `python3.11 scripts/pad/table` on the conductor; it is not a test, but it is project code writing `connectors/tiers.toml` on the conductor. Say it is the pad's conductor half (as `pad` with `where = "sandbox"` is), or run it in the Task 8 Step 2 box.
- N10. No plan or `SECRETS.md`/`TOOLS.md` lists the conductor's non-secret prerequisites: Node 22 with npm, `python3.11` (for `padrun.sh`; `GC_PYTHON` exists but the default is 3.11), `python3` 3.9+ with stdlib, `git`, `gh`, `flock`, `tar`, `npx wrangler` (long-runs Task 8 Step 8). Add a line to `SECRETS.md` or `install.sh --check`.
- N11. Handover Task 2 `box.mjs run` keeps `--vcpus` default 8 "unchanged gate size"; the owner's rule is "smallest box size". Fine for gates; say that slice boxes pass `--vcpus 2` or `4` (the matcher plan does not, see F5).

## The seven points: planned evidence

| Point | Where the plans prove it |
|---|---|
| 1 Record every model call | C1 on every composite row (mini Task 10 table), X1 in the one-hour runs (long-runs Task 7 Step 5), N7 (long-runs Task 9 Step 2), the A5 `h2-parallel.sh` exec verdict (Task 8 Step 5) |
| 2 Record every subagent | C2 on every row, Hermes by task logs with the stated note (`legs.rs:446`), X2 with 15 or more subagents and 10 at once (`longsum.py`, Task 4) |
| 3 Never block or slow the harness | X4 kill gaps under 5 s and the recover lines "same harness pid and port" (Task 7 Step 5); the matcher measured off the relay (matcher Task 2). No row times the harness itself; the stream rate (6 MB/s against the 50 MB/s target) is README open decision 11 for the owner |
| 4 Never record secrets | C3 on every row, deltas-only stated loss per 20:32Z, N6 box with every real key as known values (Task 9 Step 1), `keyscan.py` before every copy-out and commit, the `.env` plants masked by exact value |
| 5 Upload everything | C4 `server_equals_local` (`check.py:387`), `wait-upload ... final: sealed`, X5, X6 |
| 6 Tell the participant plainly | C6 and X10 (end screen equals the verifier), `connectors/tiers.toml` with a stated reason per harness (Tasks 2 and 8) |
| 7 Work with the harnesses people use | 13 or 14 harnesses in one wave on pinned versions, vendor docs re-read (T7), core set on that day's versions after the tag (Task 13) |

All seven have evidence planned; point 3 has the weakest (no latency figure from a real row), which the owner accepted in substance at 22:05Z and 23:12Z.

## Rules check

- Tests never on the conductor: holds in every plan (N9 is the one grey area). Keyscan, `snaps`, `spend` are API calls, not tests.
- Secrets: every key path goes through `put-env`; `GH_TOKEN`, `DATABASE_URL`, `CLOUDFLARE_*`, approval key unset for boxes (`pad:35-38`, `padbox.sh`, dbfetch `run.sh`). The committed `keyscan.py` checks 9 names and skips `.py`; the plan's 14-name scanner that reads dotfiles and `.py` fixes exactly the gaps the Review Focus names.
- Push rules: only `v1.0/*`, merge commits, no force, no `main`, tags or production; the release steps are the owner's (long-runs Task 12). `handoff.sh` and `scribe.sh` push docs directly, allowed by TOOLS.md lesson 5. B2 is the one place the gate rule is weaker than the plans claim.
- Staging only: every run file and `padrun.sh` row targets `server = "staging"`; `wrangler deploy --env staging` (`OPERATING.md:78-82`); `EVENT_ID` and the production deploy are owner items.
- Haiku for Claude Code: `claude-code-cli.toml:25-30` pins every alias to Haiku in both cases; A2 is Claude Code direct on the `claude` key; the Codex gateway row on Haiku is pad data under the owner's 19:17Z rule (F4 fixes the citation).
- Box use: one slice box per lane, trains, one build per head, up to 8 row boxes plus index boxes; README open decision 1 asks the owner about the 16-box peak. B2 is the gap.
- Exact-value sources 1 to 3 only: matcher plan Global Constraints and Task 4 do not add a source; `is_usable` and `MAX_*` match `values.rs:46-75`.
- Owner decisions: the draft is reflected except F3 (Hermes outcome) and F4 (citation); the 22:05Z "Aho-Corasick" wording is correctly raised as matcher open decision 2; "before the 2-hour runs" is correctly read as the 23:12Z one-hour runs; `OWNER-DECISIONS.md` is never edited by any step.

## Does the handover let a fresh container work?

Yes, once B3, B4, B5 (the slice box), F1, F2 (rowinfo.py), F9 and N10 are fixed. What would still exist only on the old machine after the plan: nothing in the tools; the lane logs and reports are copied; FINISH-PLAN.md and WRITING-PLANS.md are committed under `state/` today (F1 decides the final place). The plans themselves still reference `/root/gc-night/...` paths in prose (fine for this session) but every script path goes through `lib.sh` and `install.sh` after Task 1 to 4.
