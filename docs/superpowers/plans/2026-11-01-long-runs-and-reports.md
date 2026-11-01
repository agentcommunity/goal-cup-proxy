# Long Runs, Release and Big Tests Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prove goalcup 1.0 at length with three one-hour composite runs in parallel (Claude Code on Haiku, Codex, Hermes on DeepSeek), freeze one commit, add only the release rows the release check still lacks, write `PAD-RESULTS.md`, `connectors/tiers.toml`, `ACCEPTANCE.md`, `MORNING.md` and the `NIGHT-LOG.md` catch-up, hand the owner the release checklist, then run the big tests (core set, extra harness rows, A4, A1) on the released `v1.0.0`.

**Architecture:** Wave A is the only code: row identity in the pad (`goalcup`, `os`, `pad_trusted_at`, which the release check needs), a tier note, the committed run files, and one conductor tool (`longsum.py`). It rides one merge train and gets one box. Wave L runs A2, A5 and A3 at once on the shared H2 build; A5 also carries the Codex kill pair's job and both exec scripts, so one composite row stands in for the old kill row and exec row. Waves F, R, O are run tasks with exact settings, commands, evidence and stop rules, on one shared build per head (`$GC_NIGHT/builds/<commit8>/`); reports are written only from rows and ledger lines.

**Tech Stack:** Python 3.11 (pad, `tomllib`) and Python 3.9 stdlib (`check.py`, `table`, `longsum.py`), `scripts/pad/pad`, `sbx.mjs`, the handover plan's conductor tools (`box.mjs`, `padrun.sh`, `keyscan.py`, `scribe.sh`, `handoff.sh`, `merge.sh`), `gh` for `release.yml`, `npx wrangler` for staging.

**Spec:** `/root/gc-night/finish/FINISH-PLAN.md` (sections 2, 3 "Wave 2" to "Wave 4", 5) with the OWNER-APPROVED EXECUTION SHAPE and EXECUTION AS WORKFLOWS sections of `/root/gc-night/finish/WRITING-PLANS.md` (which override it), `docs/plans/2026-10-30-v1.0/PRD-16-orchestration-acceptance.md` sections 7, 9 to 14 and N1 to N10, `PRD-00-index.md` "Definition of done" and "Morning report template", `PRD-12-test-pad.md` sections 3.1, 8, 9, 12, the owner answers in `/root/gc-night/lanes/integrator.log` (19:31Z, 19:34Z, 20:32Z, 20:35Z, 22:03Z, 22:05Z, 23:12Z).

## Global Constraints

- PT is PST, UTC-8. Freeze: **Tue 3 Nov 18:00Z (Tue 10:00 PT)**, owner ask 6. After the freeze only fixes for failed release rows merge, each named in the ledger.
- Owner shape: one-hour composite runs, not two hours, for Claude Code on Haiku (A2, direct), Codex (A5, direct), Hermes on DeepSeek (A3, gateway): recorder kills at minutes 10 and 30, `goalcup recover` at 45, 15 or more subagents, **cap 15 USD each** (owner ask 5), all three in parallel on the shared build. Then the release (owner steps). Then the big tests on the released version. Every other row keeps `cap_usd = 10` (`pad.toml:3`).
- `cap_usd` per row comes from the harness mini-test plan's Task 1 (`row_pad`, `OVERRIDES`); this plan only sets it in run files.
- Box rules: one shared build per head (`$GC_NIGHT/builds/<commit8>/` holding `goalcup`, `goalcup-verify`, `commit`; H1 and H2 are built by the mini-test plan's Task 9 Step 0 job); up to 8 row boxes at once, plus each row's short-lived index box; per key at most 2 runs at once, DeepSeek 6 (`pad.toml:5-6`, enforced by the pad's queue); no probe boxes; gate only on real merges; smallest box, short timeouts; reviews and reports never start a box. Tests never run on the conductor machine.
- Claude Code test runs use Haiku 5.5 only (direct `claude-haiku-5-5`, gateway `anthropic/claude-haiku-5.5`; `scripts/pad/harnesses/claude-code-cli.toml:24-29`). Check the pin in the first five minutes of every Claude Code row (`watch/spend.jsonl` model keys).
- Keys only through `sbx.mjs put-env` (0600). No key on any command line, in any file, log or commit. `GH_TOKEN`, `DATABASE_URL`, `CLOUDFLARE_*`, `GOALCUP_TEST_APPROVAL_KEY`, `GOALCUP_TEST_MEMBER_ID` never enter a pad box (`scripts/pad/pad:33-37`); `padrun.sh` unsets `GH_TOKEN`.
- Push only `v1.0/*` branches, merge commits only, never force. No production, `main`, tags or website database writes; the conductor deploys to staging only (`docs/OPERATING.md:78-82`). Release steps are the owner's.
- Never edit `VISION.md` or `OWNER-DECISIONS.md`; draft owner lines go to `docs/plans/2026-10-30-v1.0/conductor/owner-decisions-draft.md`.
- Never commit `pad-out/`, the night dir's `runs/` folders, `dbfetch/idx/`, `.env*` or key files. Run `keyscan.py` (0 hits, 14 keys checked) before every commit of a report.
- Docs: plain English, no em dashes, no model names or ids, no planted value. Every claim links to a row, a gate line, a CI run or a commit (PRD-16 section 13).
- Exact-value masking reads sources 1 to 3 only, held in memory, never written. Deltas-only shape hits are PASS with "stated loss: shape in streamed reply" (20:32Z). Hermes C2 passes on task logs with "subagents shown by their task logs, not tied to individual calls" (22:03Z).
- Commit trailers: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` and `Claude-Session: https://claude.ai/code/session_01V2hUmG4Ccnfp953KQMAUS6`. Each code task commits its test alone first, subject ending ` [red]`, then the implementation.
- Paths: `$C = docs/plans/2026-10-30-v1.0/conductor`; `$GC_TOOLS`, `$GC_NIGHT`, `$GC_WT` as the handover plan's `lib.sh` defines them (`/root/gc-tools`, `/root/gc-night`, `/root/gc-wt`). The main checkout `/home/user/goal-cup-proxy` is never edited; worktrees are added from it.
- The release check (`scripts/release.sh:62-70` runs `scripts/pad/table --release-check origin/main`, `table:126-167`) needs, committed in `pad-rows.jsonl` at `main`, trusted PASS rows on the same guarded tree: 9 core tier rows (fault-free `full`, `direct`, `gateway`, `dodger` for each of the three core harnesses), one Codex direct `full` row with a `kill-recorder` step, and both exec verdicts PASS.

## Review Focus

1. Every real row carries `goalcup: null`, `os: null`, `pad_trusted_at: null` (`scripts/pad/check.py:878`, `scripts/pad/pad:839`; seen in `/root/gc-night/full4` rows), so `table --release-check` can never pass and every tier reads "pad not trusted". Expect: rows name `1.0.0+<commit>`, the box OS and the trusted commit. Tests: Task 1 `test_goalcup_id_reads_workspace_version`, `test_trusted_at_*`, `test_row_takes_identity_from_meta`.
2. A5 is the release check's Codex kill row and exec row at once; a run file that drops an exec step, or gains a `connector copy`, leaves the release check without them. Expect: the A5 plan holds both kill steps, both exec steps and no `connector copy`. Test: Task 3 `test_a5_carries_release_items`.
3. A build folder whose `commit` file is stale or from another head makes rows claim a commit they did not run. Expect: `goalcup` is null unless the folder name is a prefix of the file's 40-hex commit. Test: Task 1 `test_build_commit_must_match_folder_name`.
4. Under composite load a plain kill gap of 2 to 5 s is FAIL in `check.py` (`check.py:640-648`) but PASS under PRD-16 X4. Expect: `longsum.py` judges X4 with the 5 s bound and says so; the row's `faults` slot stays as `check.py` wrote it (so a FAIL there costs the release check A5's kill row: Task 8 Step 2 fallback). Tests: Task 4 `test_x4_allows_5s_under_load`, `test_x4_fails_over_5s`.
5. A run sealed at the 15 USD cap before 15 subagents ran reads as an X2 FAIL. Expect: "INCONCLUSIVE: sealed at the spend cap", and the report asks for a higher cap. Test: Task 4 `test_over_cap_marks_x2_inconclusive`.

---

## File Structure

| Path | Responsibility | Task |
|---|---|---|
| `scripts/pad/pad` (functions near `pinned_model` `:68`; `meta.json` dict `:836-845`) | `goalcup`, `os`, `pad_commit` in `meta.json` | 1 |
| `scripts/pad/check.py` (`:867-881` evaluate) | `trusted_at(repo, pad_commit)`; row identity from meta | 1 |
| `scripts/pad/table` (`:60-87` tier_for, `:106-124` results_md), `harnesses/hermes-cli.toml` | `tier_note` on verified tiers | 2 |
| `scripts/pad/runs/a1.toml`, `a2.toml`, `a3.toml` (modify); `a5.toml`, `core-claude-code.toml`, `core-codex.toml`, `core-hermes.toml`, `kill-codex.toml` (new) | every run this plan starts, committed so `ACCEPTANCE.md` can say how to repeat it | 3 |
| `scripts/pad/test_pad_identity.py`, `test_table_note.py`, `test_pad_acceptance_runs.py` (new) | Wave A pad tests | 1, 2, 3 |
| `$C/longsum.py`, `$C/test/test_longsum.py` (new) | cost per hour, X2/X3/X4 numbers, live projection, `ACCEPTANCE.md` block | 4 |
| `docs/plans/2026-10-30-v1.0/pad-rows.jsonl`, `PAD-RESULTS.md`, `connectors/tiers.toml` (written by `scripts/pad/table`) | the table | 8 |
| `docs/plans/2026-10-30-v1.0/ACCEPTANCE.md` (new), `MORNING.md`, `NIGHT-LOG.md`, `HANDOFF.md` | the reports | 10, 11, 13 |

Branches: `v1.0/prd-12-release-rows` (Tasks 1, 2, 3; PRD-12 owns `scripts/pad/**`), `v1.0/conductor-longsum` (Task 4). `v1.0/prd-12-release-table` (Task 8) carries the table outputs; `tiers.toml` is compiled into `goalcup` (`crates/goalcup/src/cli/start.rs:26`), so it goes through `merge.sh`, never a docs-only push.

---

## Wave A: code (Sun 1 Nov evening PT to Mon 2 Nov, rides the code-wave merge train, so it is in H1)

Rows made before Task 1 merges carry no `pad_commit` and can never be trusted.

### Task 1: Row identity: goalcup, os, pad_trusted_at

**Files:**
- Modify: `scripts/pad/pad` (near `:68`; `meta.json` dict `:836-845`), `scripts/pad/check.py:867-881`
- Test: `scripts/pad/test_pad_identity.py`

**Interfaces:**
- Produces (`pad`): `build_commit(build_dir: str) -> str | None`: the first line of `<build_dir>/commit` when it is 40 lowercase hex and the folder's basename is either not 7 to 40 hex characters or a prefix of it, else None. `goalcup_id(repo: str, commit: str | None) -> str | None`: `"<v>+<commit>"`, `<v>` from the first `^version = "(.*)"$` line of `git -C repo show <commit>:Cargo.toml` (the `release.yml:58` rule), None on any failure. `os_id(where: str) -> str`: `"linux-x86_64"` for `sandbox`, else `platform.system().lower() + "-" + platform.machine()`. `meta.json` gains `goalcup`, `os`, `pad_commit` (`git -C REPO rev-parse HEAD` at the run's start).
- Produces (`check.py`): `REPO` (three `dirname`s of `__file__`); `trusted_at(repo: str, pad_commit: str | None) -> str | None`: reads `<repo>/pad-out/selftest.json` and `<repo>/pad-out/trust.json`; returns `selftest["commit"]` only when `selftest["exit"] == 0`, `selftest["all_ran"] is True`, `trust["commit"] == selftest["commit"]`, `trust["read"]` is a non-empty string, `pad_commit` is set, and `git -C repo diff --quiet <trusted> <pad_commit> -- scripts/pad crates/goalcup-verify` exits 0. `evaluate` copies `goalcup` and `os` from meta and sets `pad_trusted_at=trusted_at(REPO, meta.get("pad_commit"))`, replacing `os=None, goalcup=None` at `:878`. Re-running `python3.11 scripts/pad/check.py <run dir>` re-stamps a row after trust is granted (Tasks 6, 8).
- `trust.json` (written by hand, Task 6): `{"commit": "<40 hex>", "read": "<HH:MM>Z first-output read done at <commit8> by <reviewer lane>"}`.

- [ ] **Step 1: Write the failing test** `scripts/pad/test_pad_identity.py` (load `pad` and `check.py` as `test_pad.py:22-28`; `git_repo()`: temp dir, `git init -q`, test author env, files `Cargo.toml` with `[workspace.package]\nversion = "1.0.0"\n`, `scripts/pad/x.py`, `crates/goalcup-verify/y.rs`, `README.md`, one commit; returns `(dir, sha)`)
```python
def test_build_commit_must_match_folder_name(self):
    c = "ab" * 20
    for name, want in (("abababab", c), ("cdcdcdcd", None), ("build", c)):
        d = os.path.join(tmp, name); os.makedirs(d); open(os.path.join(d, "commit"), "w").write(c + "\n")
        self.assertEqual(pad.build_commit(d), want)
    self.assertIsNone(pad.build_commit(os.path.join(tmp, "missing")))
    bad = os.path.join(tmp, "bad"); os.makedirs(bad); open(os.path.join(bad, "commit"), "w").write("main\n")
    self.assertIsNone(pad.build_commit(bad))
def test_goalcup_id_reads_workspace_version(self):
    repo, sha = git_repo()
    self.assertEqual(pad.goalcup_id(repo, sha), "1.0.0+" + sha)
    self.assertIsNone(pad.goalcup_id(repo, None)); self.assertIsNone(pad.goalcup_id(repo, "0" * 40))
def test_os_id(self):
    self.assertEqual(pad.os_id("sandbox"), "linux-x86_64")
def test_trusted_at_same_and_unrelated_change(self):
    repo, c1 = git_repo(); write_trust(repo, c1, exit=0, all_ran=True, read="12:00Z first-output read done")
    c2 = commit_change(repo, "README.md")
    self.assertEqual(check.trusted_at(repo, c1), c1); self.assertEqual(check.trusted_at(repo, c2), c1)
def test_trusted_at_refuses(self):
    repo, c1 = git_repo()
    c3 = commit_change(repo, "scripts/pad/x.py")
    for kw in (dict(exit=3, all_ran=False, read="r"), dict(exit=0, all_ran=True, read=""),
               dict(exit=0, all_ran=True, read="r", trust_commit="f" * 40)):
        write_trust(repo, c1, **kw); self.assertIsNone(check.trusted_at(repo, c1))
    write_trust(repo, c1, exit=0, all_ran=True, read="r")
    self.assertIsNone(check.trusted_at(repo, c3)); self.assertIsNone(check.trusted_at(repo, None))
def test_row_takes_identity_from_meta(self):
    row = evaluate({"meta.json": {"connector": "codex-cli", "case": "direct", "mode": "full", "goalcup": "1.0.0+" + "ab" * 20, "os": "linux-x86_64"}})
    self.assertEqual((row["goalcup"], row["os"], row["pad_trusted_at"]), ("1.0.0+" + "ab" * 20, "linux-x86_64", None))
```
(`write_trust(repo, commit, exit, all_ran, read, trust_commit=None)` writes both JSON files under `repo/pad-out/`; `commit_change(repo, rel)` appends a line, commits, returns the sha; `evaluate` is `test_pad.py:36-43`.)
- [ ] **Step 2: Commit the red test** (`test(pad): row identity and trust [red]`). Expected in the slice box (Task 5): FAIL with `has no attribute 'build_commit'`.
- [ ] **Step 3: Implement** `build_commit`, `goalcup_id`, `os_id` in `pad`; in `step_check` add `goalcup=goalcup_id(REPO, build_commit(build))`, `os=os_id(s["where"])`, `pad_commit=<HEAD read in __init__>` to the `meta.json` dict (`build` is the folder `step_setup` resolves at `:600`; keep it on `self`). `REPO`, `trusted_at` and the `evaluate` change in `check.py`.
- [ ] **Step 4: Commit** (`feat(pad): rows name their goalcup commit, OS and trusted pad commit`). Slice-box command: `python3.11 -m unittest discover -s scripts/pad -p 'test_*.py'`. Expected: all pad tests OK.

### Task 2: Tier note for "verified with a note"

**Files:**
- Modify: `scripts/pad/table:60-87` (`tier_for`), `:106-124` (`results_md`); `scripts/pad/harnesses/hermes-cli.toml` (one top-level line)
- Test: `scripts/pad/test_table_note.py`

**Interfaces:**
- Produces: harness-file line `tier_note = "<text>"` (read like `not_verified`, `table:68`). `tier_for` returns `("verified", version, note)` for a verified harness with a note, `("verified", version, "")` without; a note never changes another tier. `tiers.toml` keeps `tier = "verified"` (`start.rs:431-438` compares that word) and puts the note in `reason`. `PAD-RESULTS.md` summary cell: `verified with a note: <note>`.
- Data: `hermes-cli.toml` gains `tier_note = "subagents shown by their task logs, not tied to individual calls"`.

- [ ] **Step 1: Write the failing test** `scripts/pad/test_table_note.py` (load `table`; three rows `{"connector": "hermes-cli", "case": c, "mode": "full", "variant": [], "result": "PASS", "pad_trusted_at": "c" * 40, "harness_version": "0.22.5", "started": "2026-11-03T19:0%dZ" % i, "checks": {k: "PASS" for k in table.CHECKS}}` for `c` in `CASES`)
```python
NOTE = "subagents shown by their task logs, not tied to individual calls"
def test_verified_carries_the_note(self):
    with mock.patch.object(table, "harness_lines", return_value=['tier_note = "%s"' % NOTE, "[gateway]"]):
        self.assertEqual(table.tier_for("hermes-cli", ROWS), ("verified", "0.22.5", NOTE))
        md = table.results_md(ROWS, {cid: table.tier_for(cid, ROWS) for cid in table.CONNECTOR_IDS})
    self.assertIn("verified with a note: " + NOTE, md)
def test_note_never_hides_a_failure(self):
    rows = ROWS[:2] + [dict(ROWS[2], result="FAIL", checks=dict(ROWS[2]["checks"], C3="FAIL"), why=["C3 known hit"])]
    with mock.patch.object(table, "harness_lines", return_value=['tier_note = "%s"' % NOTE, "[gateway]"]):
        t = table.tier_for("hermes-cli", rows)
    self.assertEqual(t[0], "not verified"); self.assertNotIn(NOTE, t[2])
def test_no_note_is_plain_verified(self):
    with mock.patch.object(table, "harness_lines", return_value=["[gateway]"]):
        self.assertEqual(table.tier_for("hermes-cli", ROWS), ("verified", "0.22.5", ""))
```
- [ ] **Step 2: Commit the red test** (`test(pad): tier note [red]`). Expected in the slice box: FAIL on `test_verified_carries_the_note`.
- [ ] **Step 3: Implement** the `tier_note` read in `tier_for`, the summary cell in `results_md`, the Hermes line.
- [ ] **Step 4: Commit** (`feat(pad): verified tiers may carry a stated note`). Slice-box command: `python3.11 -m unittest discover -s scripts/pad -p 'test_table_note.py'`. Expected: `Ran 3 tests ... OK`.

### Task 3: The committed run files

**Files:**
- Modify: `scripts/pad/runs/a1.toml` (add `cap_usd = 15`), `a2.toml`, `a3.toml`
- Create: `scripts/pad/runs/a5.toml`, `core-claude-code.toml`, `core-codex.toml`, `core-hermes.toml`, `kill-codex.toml`
- Test: `scripts/pad/test_pad_acceptance_runs.py`

**Interfaces:**
- Consumes: `cap_usd` setting (mini-test plan Task 1).
- Produces, exact bodies (each with a one-line comment naming its PRD-16 id):
```toml
# a2.toml: A2, Claude Code direct, one-hour composite (owner 1 Nov 23:12Z)
harness = "claude-code-cli"
key = "claude"
case = "direct"
mode = "full"
minutes = 60
keep_going = 3
cap_usd = 15
prompt = "scripts/pad/prompts/many-subagents.txt"
steps = ["kill-recorder@10", "kill-recorder@30", "kill-watchdog-recover@45", "keep-going"]
```
  `a3.toml` (A3 at one hour): `harness = "hermes-cli"`, `key = "deepseek"`, `case = "gateway"`, otherwise as `a2.toml`. `a5.toml` (new id A5, Codex direct composite, also the release check's Codex kill row and exec row): as `a2.toml` with `harness = "codex-cli"`, `key = "openai"`, `steps = ["kill-recorder@10", "exec h2-parallel.sh@15", "exec long-tunnel.sh@20", "kill-recorder@30", "kill-watchdog-recover@45", "keep-going"]`. `core-<h>.toml`: `harness`, `key` (`claude`, `openai`, `deepseek`), `case = "all"`, `mode = "full"`, nothing else. `kill-codex.toml` (fallback only, Task 8 Step 2): Codex direct full, `steps = ["kill-recorder@8", "kill-recorder@10"]` (moved to 3 and 5 by `kill_minutes`, `codex-cli.toml:19`). `a4.toml` unchanged (cap 10).

- [ ] **Step 1: Write the failing test** `scripts/pad/test_pad_acceptance_runs.py` (load `pad`; `PADCFG = pad.read_toml(HERE + "/pad.toml")`; `harness(s)` = `pad.read_toml(HERE/harnesses/<s["harness"]>.toml)`; `plan(name)` = `load_settings(read_toml(runs/<name>))` then `apply_kill_minutes` with the harness file; `kills(s)` as `test_pad_kill_times.py:30-31`)
```python
def test_composite_runs(self):
    for name in ("a2.toml", "a3.toml", "a5.toml"):
        s = plan(name)
        self.assertEqual((kills(s), s["minutes"], s["cap_usd"], s["keep_going"]), ([10, 30], 60, 15, 3))
        self.assertIn("kill-watchdog-recover@45", s["steps"]); self.assertTrue(s["prompt"].endswith("many-subagents.txt"))
def test_a5_carries_release_items(self):
    s = plan("a5.toml"); steps = [r for _, _, r in s["plan"]]
    self.assertEqual((s["harness"], s["case"], s["mode"]), ("codex-cli", "direct", "full"))
    self.assertIn("exec h2-parallel.sh@15", steps); self.assertIn("exec long-tunnel.sh@20", steps)
    self.assertFalse(any("connector copy" in r for r in steps))
def test_a1_two_hours_cap_15(self):
    s = plan("a1.toml"); self.assertEqual((s["minutes"], s["cap_usd"], s["case"]), (120, 15, "direct"))
def test_core_rows_give_tiers(self):
    for h in ("claude-code", "codex", "hermes"):
        s = plan("core-%s.toml" % h)
        self.assertEqual((s["case"], s["mode"], s["minutes"]), ("all", "full", 20))
        self.assertEqual([x for x in pad.step_list(s, harness(s)) if x.split("@")[0] in pad.FAULTS], [])
def test_codex_kill_fallback_uses_harness_minutes(self):
    self.assertEqual(kills(plan("kill-codex.toml")), [3, 5])
```
- [ ] **Step 2: Commit the red test** (`test(pad): composite and release run files [red]`). Expected in the slice box: FAIL with `No such file` for `a5.toml`.
- [ ] **Step 3: Write the run files.**
- [ ] **Step 4: Commit** (`feat(pad): one-hour composite runs, core and fallback kill run files`). Slice-box command: `python3.11 -m unittest discover -s scripts/pad -p 'test_*.py'`. Expected: all OK.

### Task 4: longsum.py, cost per hour and the X-line numbers

**Files:**
- Create: `$C/longsum.py`, `$C/test/test_longsum.py`

**Interfaces:**
- Consumes: a pad run folder (`meta.json` with `steps` as `[step, result, "H:MM", saw]`, `over_cap`; `row.json` `facts`; `watch/spend.jsonl`; `verify/usage/<entry>.json` `threads`), and `check.py`'s `Run(folder, meta).gaps()` (`check.py:163,214-222`) loaded from `<repo>/scripts/pad/check.py`.
- Produces:
  - `RUN1 = {"minutes": 49, "input": 117_400_000, "output": 410_600, "uploaded_mb": 405}` (PRD-16 X8, X9).
  - `step_min(meta: dict, prefix: str) -> float | None`: minutes of the first step line starting with `prefix`.
  - `hours(meta: dict) -> float`: `(step_min(meta, "seal") - step_min(meta, "start")) / 60`, else `meta["minutes"] / 60`.
  - `max_parallel(threads: list) -> int`: most threads with a non-null `parent` whose `[first, last]` overlap at one instant.
  - `summary(run_dir: str, check_mod, min_subagents: int = 15, gap_limit_ms: int = 5000) -> dict` with keys `hours, usd, over_cap, usd_per_hour, tokens_per_hour {input, cached, output}, raw_mb_per_hour, compressed_mb_per_hour, rss_peak_mb, wire_requests, subagents, threads, max_parallel, gaps [{cause, ms}], x2, x3, x4` (MB is 10^6 bytes; rates rounded to 0.1). `x2`: `"PASS"` when `subagents >= min_subagents` and `max_parallel >= 10`; `"INCONCLUSIVE: sealed at the spend cap"` when short and `over_cap`; else `"INCONCLUSIVE: <n> subagents, <m> at once"`. `x3`: `"FAIL"` on any `memory_guard` gap, else `"PASS"`. `x4`: with `k` the count of `kill-recorder@` steps and `w` 1 if a `kill-watchdog-recover@` step ran: `"PASS"` when there are exactly `k + w` `recorder_down` gaps, the first `k` by start each under `gap_limit_ms`, every gap start before end, no `null_mode` gap, and every `why` line of the row that starts with `faults` is about the 2 s bound (contains `not under 2 s` or `(limit 2 s)`); else `"FAIL: <first reason>"`.
  - `live(run_dir: str, planned_minutes: float) -> dict`: start from the folder name's `YYYYmmddTHHMMSSZ`; from the last `watch/spend.jsonl` line: `minutes`, `usd`, `usd_per_hour`, `projected_usd`.
  - `markdown(s: dict, baseline: dict | None = RUN1) -> str`: rows `| Measure | This run | Run 1 |` for hours, USD, USD per hour, input and output tokens per hour, raw and compressed MB per hour, recorder RSS peak, subagents and most at once, gaps, X2, X3, X4. Never prints a model name.
  - CLI: `python3 longsum.py RUN_DIR [--live MINUTES] [--json] [--min-subagents N] [--gap-limit-ms N] [--repo DIR]` prints `markdown(summary(...))`, the `summary` dict with `--json`, the `live` dict with `--live`; `--repo` defaults to the nearest parent of `RUN_DIR` holding `scripts/pad/check.py`; exit 2 on a folder without `row.json` (or without `watch/spend.jsonl` for `--live`).

- [ ] **Step 1: Write the failing test** `$C/test/test_longsum.py` (unittest; `fixture(tmp, **over)` writes a run folder: `meta.json` steps `start 0:01`, `seal 1:01`, `kill-recorder@10`, `kill-recorder@30`, `kill-watchdog-recover@45` as step lines, `over_cap false`, `entry "e1"`; `row.json` facts tokens input 6_000_000 cached 30_000_000 output 120_000, `raw_bytes 2_000_000_000`, `compressed_bytes 50_000_000`, `subagents 16`, `threads 17`, `wire_requests 900`, `spend_usd_est 6.0`, `recorder_rss_peak_mb 180.0`, `models ["claude-haiku-5-5"]`; usage threads: 10 with parent overlapping 14:10 to 14:40, 1 with parent at 15:00 to 15:05, 1 without parent; `stub(GAPS)` is a `check_mod` whose `Run(folder, meta).gaps()` returns `(GAPS, {})`)
```python
def test_per_hour(self):
    s = longsum.summary(fixture(t), stub([gap(10, 900), gap(30, 1200), gap(46, 40000)]))
    self.assertEqual((s["hours"], s["usd_per_hour"], s["tokens_per_hour"]["input"], s["raw_mb_per_hour"], s["compressed_mb_per_hour"]),
                     (1.0, 6.0, 6000000.0, 2000.0, 50.0))
def test_max_parallel(self):
    self.assertEqual(longsum.summary(fixture(t), stub([]))["max_parallel"], 10)
def test_x4_allows_5s_under_load(self):
    d = fixture(t, why=["faults: a recorder_down gap of 3100 ms, not under 2 s"])
    self.assertEqual(longsum.summary(d, stub([gap(10, 3100), gap(30, 900), gap(46, 40000)]))["x4"], "PASS")
def test_x4_fails_over_5s(self):
    self.assertTrue(longsum.summary(fixture(t), stub([gap(10, 5200), gap(30, 900), gap(46, 40000)]))["x4"].startswith("FAIL"))
    self.assertTrue(longsum.summary(fixture(t), stub([gap(10, 900), gap(46, 40000)]))["x4"].startswith("FAIL"))
def test_x3_memory_guard_fails(self):
    self.assertEqual(longsum.summary(fixture(t), stub([gap(10, 900, cause="memory_guard")]))["x3"], "FAIL")
def test_over_cap_marks_x2_inconclusive(self):
    s = longsum.summary(fixture(t, subagents=8, over_cap=True), stub([]))
    self.assertEqual(s["x2"], "INCONCLUSIVE: sealed at the spend cap")
def test_live_projection(self):
    d = live_fixture(t, "20261102T200000Z-pad-abc123", [("2026-11-02T20:30:00+00:00", 4.0)])
    self.assertEqual(longsum.live(d, 60), {"minutes": 30.0, "usd": 4.0, "usd_per_hour": 8.0, "projected_usd": 8.0})
def test_markdown_has_run1_and_no_model_id(self):
    md = longsum.markdown(longsum.summary(fixture(t), stub([])))
    self.assertIn("| Raw MB per hour | 2000.0 | 495.9 |", md); self.assertNotIn("haiku", md)
```
(`gap(minute, ms, cause="recorder_down")` builds `{"cause", "start", "end", "ms"}` at 2026-11-02T14:00Z plus `minute`.)
- [ ] **Step 2: Commit the red test** on `v1.0/conductor-longsum` (`test(conductor): longsum [red]`). Expected in the slice box: FAIL with `No module named 'longsum'`.
- [ ] **Step 3: Implement `$C/longsum.py`** (standard library, Python 3.9; loads `check.py` with `importlib.machinery.SourceFileLoader`; prints no value from `calls.log` or panes).
- [ ] **Step 4: Commit** (`feat(conductor): longsum, cost per hour and X lines from a run folder`). Slice-box command: `python3 -m unittest discover -s $C/test -p 'test_longsum.py'`. Expected: `Ran 8 tests ... OK`.

### Task 5: The slice box and the merge train

- [ ] **Step 1: One local test merge (conductor, no box).** `git -C /home/user/goal-cup-proxy worktree add -q --detach "$GC_WT/wave-a-check" origin/v1.0/integration && git -C "$GC_WT/wave-a-check" merge -q --no-ff origin/v1.0/prd-12-release-rows origin/v1.0/conductor-longsum -m "check: wave A"` (never pushed; the mini-test plan's rig slice must already be on integration, or merged here too, for `cap_usd`).
- [ ] **Step 2: The one slice box.** Job: `set -e; py=$(command -v python3.11 || command -v python3); [ -f $C/test/red-green.sh ] && bash $C/test/red-green.sh origin/v1.0/integration; $py -m unittest discover -s scripts/pad -p 'test_*.py'; python3 -m unittest discover -s $C/test -p 'test_longsum.py'`. Run: `node "$GC_TOOLS/box.mjs" run --repo "$GC_WT/wave-a-check" --ref HEAD --script "$J" --minutes 15 --vcpus 2`. Expected: one `RED-OK` per red commit (4), both unittest runs `OK`, `[box] exit 0`. On a failure: all findings fixed in one batch, one more box (the one fix round).
- [ ] **Step 3: Join the code-wave train** (two `.req` files in `$GC_NIGHT/mrq/`, `refresh_lock: false`; one gate on the combined head, bisect on failure). Expected: `.res` `"exit": 0` for both; ledger `merge` lines. Then `git worktree remove --force "$GC_WT/wave-a-check"`.

---

## Wave L: three one-hour composite runs on H2 (Mon 2 Nov 15:00 PT, 23:00Z, about 1.5 hours wall)

H2 is the head after the composite 10-minute wave's single fix round (that wave's plan); its build is `$GC_NIGHT/builds/<H2 8>/` with `$GC_NIGHT/trust/selftest-<H2 8>.json`, made once by that plan's Task 11 Step 4 (H2 = H1 and the H1 pair when the fix list was empty).

### Task 6: Trust the pad at H2

- [ ] **Step 1: Selftest rides the H2 build, no extra box** (the mini-test plan's build job runs `scripts/pad/selftest`). Expected in `$GC_NIGHT/trust/selftest-<H2 8>.json`: `"exit": 0`, `"all_ran": true`. If not: the failure goes to PRD-12, rows run untrusted, and the Task 8 Step 2 box's selftest is the next chance (rows re-stamped then).
- [ ] **Step 2: First-output read (reviewer lane, read-only, no box).** A reviewer who did not write the pad reads the selftest output, the first composite-wave row on H2 (on H1 when `scripts/pad` and `crates/goalcup-verify` did not change) and the first kill step lines of A2 (`table.txt`, `row.json`, `report.txt`, `status.json`, `calls.log`) against the PRD-12 section 12 list, using the `verify/` JSON the row holds; a `goalcup-verify dump` still needed is one more line in the Task 8 Step 2 box. Output: one night-log line per finding or `<HH:MM>Z | pad reviewer PRD-12 | decision | first-output read done at <H2 8>`. Stop rule: any finding blocks trust; route it to PRD-12; rows keep running and are re-stamped once trust holds.
- [ ] **Step 3: Write trust and re-stamp.** `printf '{"commit": "%s", "read": "%s"}\n' <H2 40> "<that log line>" > $GC_NIGHT/trust/trust-<H2 8>.json`; copy both files to `<worktree>/pad-out/selftest.json` and `trust.json` in every H2 worktree; `python3.11 scripts/pad/check.py <dir>` for each H2 row folder. Expected: `pad_trusted_at` equals H2 on every row whose `pad_commit` has the same `scripts/pad` and `crates/goalcup-verify` tree.

### Task 7: A2, A5, A3 at once

**Settings:** `scripts/pad/runs/a2.toml` (Claude Code direct, claude key), `a5.toml` (Codex direct, OpenAI key), `a3.toml` (Hermes gateway, DeepSeek model, gateway key). Three run boxes plus index boxes; one run per key.

- [ ] **Step 1: Preconditions (no box).** Owner answered asks 5 (cap 15) and 9 (gateway credit); trie matcher and Wave A merged into H2; Task 6 Step 1 done; `node "$GC_TOOLS/box.mjs" spend --since <now-1h>` shows no live `pad-` box.
- [ ] **Step 2: Worktree.**
```bash
git -C /home/user/goal-cup-proxy worktree add -q --detach "$GC_WT/long" <H2>
mkdir -p "$GC_WT/long/pad-out" && ln -s "$GC_NIGHT/builds/<H2 8>" "$GC_WT/long/pad-out/build"
cp "$GC_NIGHT/trust/selftest-<H2 8>.json" "$GC_WT/long/pad-out/selftest.json"   # trust.json once Task 6 Step 3 holds
```
- [ ] **Step 3: Start all three (each in the background).** For `r` in `a2 a5 a3`: `GC_PAD_WT=$GC_WT/long bash "$GC_TOOLS/padrun.sh" "$GC_WT/long/scripts/pad/runs/$r.toml" "$GC_NIGHT/long"`; ledger `run` line per row with the settings name and H2.
- [ ] **Step 4: Watch every 5 minutes (no box).** `python3 "$GC_TOOLS/longsum.py" <run dir> --live 60`. Expected: first `watch/spend.jsonl` line within 6 minutes; its model keys equal the harness file's pin for the case (A2 `claude-haiku-5-5`, A5 `gpt-6.1-sol`, A3 the `[gateway]` model `deepseek/deepseek-v4.1-flash`).
Stop rules: a model key other than the pin in the first spend line: `kill -TERM $(pgrep -f "scripts/pad/pad .*/$r.toml")`, ledger `stop`, fix the pin, one rerun. `projected_usd` above 22.5 or a box that will not stop: W4 call. No new `usd` reading for 15 minutes: read `calls.log`; a dead box is `sandbox_lost`, one rerun. The pad's own seal at 15 USD is not a failure (PRD-16 section 9).
- [ ] **Step 5: Judge each row (no box).** `python3 "$GC_TOOLS/longsum.py" <run dir> --json > $GC_NIGHT/long/<r>-summary.json` and `python3 "$GC_TOOLS/longsum.py" <run dir> > $GC_NIGHT/long/<r>-summary.md`, kept for Task 10.
Expected PASS evidence per row: `PAD_EXIT` in the log; `row.json` C1 to C6 PASS (Hermes C2 on task logs; deltas-only C3 note allowed); `x2` PASS (15 or more subagents, 10 at once); `x3` PASS; `x4` PASS (two plain kills under 5 s, one recover gap) and `check.py`'s recover lines (same harness pid and port, both recover outcomes, `harness_exit` or `deadline` seal) not FAIL; C4 `server_equals_local` (X6); `secret_hits` 0 (X7); `unattributed` 0 (X8); end screen equals verifier (X10, C6). A5 also: `exec/h2-parallel.sh.txt` and `exec/long-tunnel.sh.txt` present (verdicts in Task 8 Step 4).
Stop rules: INCONCLUSIVE reruns once; a quiet kill (no work in flight) is INCONCLUSIVE, not FAIL. A FAIL goes to a fix lane per named cause; the fix joins one pre-freeze fix train (one gate, counted in the Budget only if it happens); S1 at three attempts on one cause, then W7. X2 short because the harness spawned fewer: INCONCLUSIVE for load, rerun only with the owner's word.
- [ ] **Step 6: Clean up and log.** `node scripts/pad/sbx.mjs list --prefix pad-` in `$GC_WT/long` shows none of these boxes; `node "$GC_TOOLS/box.mjs" spend --since <start> --ledger`; one ledger `run` line per row with result, USD, USD per hour and `x2`/`x3`/`x4`.

---

## Wave F: freeze and release rows (Tue 3 Nov 18:00Z to Wed 4 Nov 12:00Z)

### Task 8: Freeze, one gate-and-build box, release rows, table, staging, Mac

- [ ] **Step 1: Freeze line (18:00Z).** HF = `origin/v1.0/integration` after the last train. Ledger: `18:00Z | integrator conductor | decision | FREEZE at <HF 8>: only fixes for failed release rows merge from now, each named here`.
- [ ] **Step 2: One box: gate full, release build, selftest.** Job: `set -e; sh scripts/gate.sh full; git rev-parse HEAD > target/release/commit; $(command -v python3.11 || command -v python3) scripts/pad/selftest --bin target/release`. Run: `node "$GC_TOOLS/box.mjs" run --repo /home/user/goal-cup-proxy --ref <HF> --script "$J" --minutes 40 --out target/release/goalcup:$GC_NIGHT/builds/<HF 8>/goalcup --out target/release/goalcup-verify:$GC_NIGHT/builds/<HF 8>/goalcup-verify --out target/release/commit:$GC_NIGHT/builds/<HF 8>/commit --out pad-out/selftest.json:$GC_NIGHT/trust/selftest-<HF 8>.json`. Expected: `gate full: PASS in <N>s`, `selftest: all controls ran as wanted`, `[box] exit 0`, three files in `builds/<HF 8>/`; ledger `gate` line; CI `check` green on HF (`gh run list --branch v1.0/integration --limit 1`). Trust at HF: if `git diff --quiet <H2> <HF> -- scripts/pad crates/goalcup-verify`, `trust-<HF 8>.json` names HF and quotes the H2 read line; else a new read (Task 6 Step 2) on the first HF row before Step 5.
Stop rules: gate FAIL: the failing step to its owner; one fix train; a new HF; the freeze time stands. Box fails to start three times in 10 minutes: S8.
- [ ] **Step 3: What the release check still lacks (no box).** Worktree `$GC_WT/final` at HF, `pad-out/build` linked to `builds/<HF 8>`, `selftest.json` and `trust.json` copied in. Re-stamp and import every earlier row whose `meta.json` `pad_commit` has the same `scripts/pad` and `crates/goalcup-verify` tree as HF (`$GC_WT/final/scripts/pad/check.py <dir>`, then copy only `row.json` to `$GC_WT/final/pad-out/imported/<n>/row.json`); this brings A2, A5, A3 (`$GC_WT/long/pad-out`) and the composite-wave rows (`$GC_WT/mini/pad-out`) in. Run `python3 scripts/pad/table`, commit locally (never pushed) and run `python3 scripts/pad/table --release-check HEAD`. Its list is the work of Step 4; nothing else runs.
- [ ] **Step 4: Run only the missing items, all at once, with `padrun.sh` into `$GC_NIGHT/final`.** A missing `<core harness> <case> full`: `core-<h>.toml` (all three cases; a copy in the run worktree with `case` set when only one case is missing). A missing or failed Codex kill row or exec item: if A5 FAILed only on `check.py`'s 2 s kill bound, `kill-codex.toml` for the kill row; otherwise rerun A5 (Task 7 Steps 3 to 6 in `$GC_WT/final`). A line `<commit> differs from <HF> under crates/goalcup...`: rerun the named composite runs on HF (Task 7 Steps 3 to 6, up to 45 USD); under `connectors/<id>` only: that harness's run. Expected evidence: each core row PASS with all six checks (Hermes C1 by the totals form, C2 on task logs); each dodger row `listed_hosts_decrypted: named`, verifier exit 1, end screen "not recorded" (PRD-16 N1); every row `goalcup` = `1.0.0+<HF>`. Stop rules as Task 7 Step 5. A post-freeze fix that changes `crates/` or `connectors/`: new HF, one new build box (Step 2), and Steps 3 and 4 again. A second failure on the same cause: W7.
- [ ] **Step 5: Exec verdicts (watcher, read-only).** The watcher reads A5's (or its rerun's) `exec/h2-parallel.sh.txt` (ALPN h2, four `protocol 2` lines, curl exit 0) and `h2-dump.txt` from Task 9 Step 1, which runs first (one `conn` with `alpn: "h2"`, four decoded pairs, zero `h2_unsupported`), and `exec/long-tunnel.sh.txt` (curl exit 0 or 28 at 90 s, bytes above 0, the tunnel `conn` whole), writes `exec/<script>.verdict` as `PASS <one line why>` or `FAIL <why>`, then `python3.11 scripts/pad/check.py <A5 run dir>`. Expected: `row.json` `exec` `{"h2-parallel.sh": "PASS", "long-tunnel.sh": "PASS"}` (done item 9).
- [ ] **Step 6: Tier notes, table, commit.** For each harness whose newest full rows pass with a stated note, add its `tier_note` line (`scripts/pad/harnesses/<id>.toml`), e.g. `"stated loss: shape in streamed reply"`. Re-import the rows of Steps 4 and 5, then `cd $GC_WT/final && python3 scripts/pad/table`. Expected: `table: <n> of <n> rows read; wrote connectors/tiers.toml, docs/plans/2026-10-30-v1.0/pad-rows.jsonl, PAD-RESULTS.md`; 16 tables in `tiers.toml`, each `verified` (version named, maybe a note), `not verified` with a reason, or `not supported`. Branch `v1.0/prd-12-release-table` from HF with those files and any `tier_note` lines; `python3 "$GC_TOOLS/keyscan.py" "$GC_WT/final/connectors" "$GC_WT/final/docs" "$GC_WT/final/scripts/pad/harnesses"` 0 hits; queue (gate fast). The merged head is HR.
- [ ] **Step 7: N5.** `git -C "$GC_WT/final" checkout -q --detach <HR> && python3 scripts/pad/table --release-check <HR>`. Expected: `release check: pass at <HR>`, exit 0. Then in a throwaway worktree at HR a scratch commit appending one comment line to `crates/goalcup/src/main.rs` (never pushed): `python3 scripts/pad/table --release-check HEAD` exits 1 naming `crates/goalcup/src/main.rs`. Ledger `check` line with both exits; remove the scratch worktree.
- [ ] **Step 8: Staging from HF (done item 7).** In a worktree at HF: `cd server/worker && npm ci --silent && npx wrangler deploy --env staging --tag <HF 8>` with `CLOUDFLARE_ACCOUNT_ID` and `CLOUDFLARE_API_TOKEN` in the environment only. Expected: a new staging version id; ledger `deploy` line. Read-only N7 half: `npx wrangler deployments list` (no `--env`) unchanged since 1 Nov (or the owner checks).
- [ ] **Step 9: macOS build (owner ask 7 must be yes).** Skip if a `release.yml` run on a commit with the same `crates/` tree is already green (cite it). Else `gh workflow run release.yml --repo agentcommunity/goal-cup-proxy --ref v1.0/integration`; `gh run list --workflow release.yml --limit 1 --json databaseId,headSha,status` (headSha must be HF or HR); `gh run watch <id> --exit-status`. Expected: four `build` jobs green (`aarch64-apple-darwin`, `x86_64-apple-darwin`, both Linux), job `release` skipped. Stop rule: a Mac job red: the failing step to its owning PRD (likely `spawn.rs` `ps` or `clock.rs:74-90` boot id); one fix, one rerun.

### Task 9: N6, N7, N9 and the newcomer run

- [ ] **Step 1: One N6 box after the last Task 8 Step 4 row has ended, before Task 8 Step 5.** `node scripts/pad/sbx.mjs create --name pad-n6-<HHMMSS> --minutes 40 --snapshot $(cat scripts/pad/image.id) --vcpus 2`; `push-dir` each of `$GC_WT/final/pad-out`, `$GC_WT/long/pad-out`, `$GC_WT/mini/pad-out`, `$GC_NIGHT/lanes`, and a folder holding `NIGHT-LOG.md` and `git log -p --since=2026-10-30 origin/v1.0/integration` saved as one file; `put ~/bin/goalcup-verify --file $GC_NIGHT/builds/<HF 8>/goalcup-verify --mode 755`; `put-env ~/k/<n> --env <VAR>` for `CLAUDE_API_KEY`, `OPENAI_API_KEY`, `DEEPSEEK_API_KEY`, `GEMINI_API_KEY`, `AI_GATEWAY_API_KEY`; `exec` a script: `cat ~/k/* > ~/known && rm -rf ~/k; ~/bin/goalcup-verify secrets --known ~/known <every pushed folder> > ~/n6.json; rc=$?; rm -f ~/known; ~/bin/goalcup-verify dump <A5's rec-kept folder> > ~/h2-dump.txt; exit $rc`; `get ~/n6.json`, `get ~/h2-dump.txt` (`keyscan.py` on it before anyone reads it); `rm` the box from an EXIT trap. All `sbx.mjs` calls with `GH_TOKEN`, `DATABASE_URL`, `CLOUDFLARE_*` and the approval key unset. Ledger `decision` line naming the box. Expected: `n6.json` 0 hits, exit 0; then `python3 "$GC_TOOLS/keyscan.py" <the same local paths>`: `keyscan: 0 hit(s), 14 keys checked, unset: none`. Stop rule: any hit is W1: seal and remove every pad box, no new run (S6), owner call.
- [ ] **Step 2: N7.** `grep -rl 'ingest.agentcommunity.org' $GC_WT/final/pad-out $GC_WT/long/pad-out $GC_WT/mini/pad-out --include='goalcup*.log'` prints nothing; Task 8 Step 8's production list unchanged. Ledger line.
- [ ] **Step 3: N9.** `node scripts/pad/sbx.mjs list --prefix pad-` lists no live box; `gh repo list --limit 200 --json name -q '.[].name' | grep '^gc-pad-'` prints only repos a cleanup line kept; `find $GC_WT/final/pad-out $GC_WT/long/pad-out $GC_WT/mini/pad-out \( -name '*.key' -o -path '*/keys/*' \) -type f` prints nothing; `node "$GC_TOOLS/box.mjs" spend --since 2026-11-01T23:00:00Z --ledger`. Each leftover gets a ledger line.
- [ ] **Step 4: Newcomer run (PRD-15 acceptance 1), skip if the docs plan already ran it on HF.** One fresh lane given only `README.md`, `docs/PARTICIPANT-GUIDE.md` and the staging `GOALCUP_SERVER`, driving one pad box through `sbx.mjs exec` (build from `builds/<HF 8>`, Claude Code on Haiku, about 15 minutes, a short prompt), the conductor approving; the box removed at the end. Expected: a sealed, uploaded practice run, and the lane's list of every place the docs did not say what to do. Stop: 30 minutes wall.

---

## Wave R: reports before the release (Wed 4 Nov 06:00Z to 18:00Z, Wed 10:00 PT)

### Task 10: ACCEPTANCE.md

**Files:** Create `docs/plans/2026-10-30-v1.0/ACCEPTANCE.md` (docs-only; PRD-16 owns it).

- [ ] **Step 1: Write it.** One section per run, in the order A2, A5, A3, then A4 and A1 as "runs after the release, Task 13" until they have run: the prompt as run (`scripts/pad/prompts/many-subagents.txt` text, or `full`; run 1's own prompt only if the owner agreed, ask 14); the settings file path and text; the commit; the row (run id, entry id, `pad-rows.jsonl` line); C1 to C6; the `longsum.py` markdown block; X1, X5, X6, X7, X8, X10 one line each from the row and `fetch.json`; "To repeat": the `padrun.sh` command. One line: the composite runs replace PRD-16's A2 and A3 lengths, and A5 stands in for the separate Codex kill and exec rows, by owner rule 23:12Z.
- [ ] **Step 2: Check and push.** `grep -c '—' ACCEPTANCE.md` is 0; `grep -E 'claude-|gpt-|deepseek-|gemini-'` prints nothing; `python3 "$GC_TOOLS/keyscan.py" <file>` 0 hits; `GC_SESSION=https://claude.ai/code/session_01V2hUmG4Ccnfp953KQMAUS6 bash "$GC_TOOLS/handoff.sh"`.

### Task 11: MORNING.md, NIGHT-LOG.md, HANDOFF.md

- [ ] **Step 1: Catch up the log.** `GC_SESSION=https://claude.ai/code/session_01V2hUmG4Ccnfp953KQMAUS6 bash "$GC_TOOLS/scribe.sh"`. Expected: `scribed <n> lines`; the last `NIGHT-LOG.md` line time is at or after the last line of every `$GC_NIGHT/lanes/*.log`.
- [ ] **Step 2: Rewrite MORNING.md** from the PRD-00 "Morning report template" (`PRD-00-index.md:895-990`), only from rows, ledger lines, CI runs and commits. Line 1: `Ready to merge v1.0/integration at <HR>` only if done items 1 to 9 all hold, else `Not ready: <the one main reason>`; the acceptance line (A2, A3, A5 done; A1, A4 after the release) next to it. Directly under the verdict: "Your decisions", every open owner ask with the default used (FINISH-PLAN section 5 numbering), the size overrun (about 39.9k Rust lines against 18.1k), key rotation. Then the template's sections; "Your steps" is Task 12's checklist; "Spend and resources" from `pad-out/spend.jsonl` totals and the `box.mjs spend` ledger lines; the newcomer run beside done item 6; the macOS line (Task 8 Step 9 run link, the owner's smoke in Task 12 item 6); "After the release": Task 13.
- [ ] **Step 3: N8, one fresh reader (read-only lane, no box).** It reads only `MORNING.md` and times itself. Expected: verdict, blockers and the owner's next three steps stated within 10 minutes. Otherwise cut detail into `ACCEPTANCE.md` and repeat once.
- [ ] **Step 4: Push.** Em-dash, model-id and keyscan checks as Task 10 Step 2; `handoff.sh` (it also refreshes `HANDOFF.md`). Remove `/root/gc-night/status/STATUS-REPORT.md` from the reading list in `HANDOFF.md`.

---

## Wave O: release, then big tests (Wed 4 Nov 22:00Z to Fri 6 Nov)

### Task 12: Owner release checklist (owner only; copied into MORNING.md "Your steps")

- [ ] 1. Read `MORNING.md`; answer its decisions or accept the defaults.
- [ ] 2. Read-only: no real run open on production (PRD-07 section 12 step 3 query).
- [ ] 3. Merge `v1.0/integration` at HR into `main` as a merge commit. `main` is `4b226de2` at `0.7.0`; the merge brings `1.0.0`, so no `release.sh prepare`. If `main` moved since `4b226de2` under a guarded path, the conductor reruns Task 8 Steps 3 and 4 on the merge result first (on your word), else the release check refuses.
- [ ] 4. `sh scripts/gate.sh full` on `main` in a build box (or the conductor: `box.mjs run --ref origin/main`, on your word).
- [ ] 5. `sh scripts/release.sh tag 1.0.0` (runs `scripts/pad/table --release-check` on `origin/main` first, `release.sh:62-70`).
- [ ] 6. `release.yml` publishes four targets; run `install.sh` from `releases/latest` on Linux and your Mac; one pad run on the Mac (`where = "here"`, PRD-12 P9), noting the masker boot time (target under about 2 s, else the value build moves after `ready`).
- [ ] 7. Production Worker from your own device, from the tagged commit: `cd server/worker && npx wrangler deploy`; `MIN_COMPETITION_VERSION = "1.0.0"` goes live with it; then `npx wrangler secret delete RECEIPT_SIGNING_KEY`.
- [ ] 8. Production smoke: one practice run sealed, fetched, checked.
- [ ] 9. Deferred checks: PRD-14 G2, PRD-12 P8.
- [ ] 10. `EVENT_ID = "goalcup-cup-2026"` between 6 Nov 00:00Z and 7 Nov 00:00Z (Thu 16:00 PT to Fri 16:00 PT), `server/worker/wrangler.toml:36-42`, your deploy.
- [ ] 11. Rotate the test keys after the last test run (ask 13).

### Task 13: Big tests on the released version (conductor, on the owner's word, staging only, Thu 5 Nov)

- [ ] **Step 1: Build.** If `git diff --quiet <HF> v1.0.0 -- crates/ server/worker/src/ connectors/ ':(exclude)connectors/tiers.toml' Cargo.toml Cargo.lock rust-toolchain.toml`, reuse `builds/<HF 8>`; else one build box of `v1.0.0`. Worktree `$GC_WT/big` at `v1.0.0`, as Task 8 Step 3.
- [ ] **Step 2: Core set and extra harness rows, all at once into `$GC_NIGHT/big`.** `core-claude-code.toml`, `core-codex.toml`, `core-hermes.toml` (9 rows) on that day's harness versions (pin bumps only in the run worktree, never committed); A4 (`a4.toml`, Kilo gateway, 20 minutes: C1 to C6 and N3, `ai-gateway.vercel.sh` decrypted in every `conn`); and one `full` row (a run-worktree copy of `runs/full.toml` with `harness`, `key` and `case = "direct"`, or `"gateway"` where the harness has no direct case) per phase 5 harness whose composite 10-minute row passed (up to 8). About 6 boxes at once, per-key queue. Expected and stop rules as Task 8 Step 4. A new failure is a `1.0.1` fix forward, reported to the owner; tiers change only in `1.0.1`.
- [ ] **Step 3: A1, after Step 2's OpenAI rows end.** `scripts/pad/runs/a1.toml` (Codex direct, OpenAI key alone, 120 minutes, keep going after 3 idle minutes, cap 15 USD); watch as Task 7 Step 4 with `--live 120`. Expected: C1 to C6, X1 to X10; X2 at least 20 subagent threads; X8 and X9 per hour against run 1 (`longsum.py` run 1 column). Stop rules as Task 7.
- [ ] **Step 4: Report.** Append A4 and A1 sections to `ACCEPTANCE.md`, a "Big tests on 1.0.0" block to `MORNING.md` (rows, results, any `1.0.1` items); checks and push as Task 10 Step 2; Task 9 Step 3 (N9) again.

---

## Budget

| Wave | Boxes | Builds | Gates | Rows | Model USD (likely / cap) |
|---|---|---|---|---|---|
| A | 1 slice box (2 with its fix round) | 0 | share of the code-wave train | 0 | 0 |
| L | 6 (3 rows and their index boxes) | 0 (H2 shared) | 0 | 3 | about 15 / 45 |
| F | about 22: 1 gate-build box, the missing release rows (likely the 9 core rows, 18 boxes), 1 N6 box, 1 newcomer box, 1 table-train gate | 1 (HF) | 2 (gate full on HF, table train) | about 9 | about 10 / 90 |
| Through release | about 30 | 1 | 2 | about 12 | about 25 / 135 |
| O (after the release) | about 38 (9 core rows, A4, up to 8 extra rows, A1); about 20 if Task 13 drops the core set (Open decision 1) | 0 or 1 | owner's | about 19 (10) | about 25 / 200 |

The whole finish (README in this folder) adds the handover, code-wave and mini-test plans; a Wave L FAIL adds one pre-freeze fix train and a post-freeze fix one new HF build.

## Open decisions

1. **Core set before or after the release (owner).** The owner's shape runs the core set on the released version, but `release.sh tag` refuses without 9 trusted core tier rows on the released tree, and the composite runs cannot be tier rows (they carry faults). So the core rows the release check lacks run on HF before the tag (Task 8 Step 4), and the core set runs again on `v1.0.0` with that day's harness versions (Task 13). If the owner wants the core set once, Task 13 drops it.
2. **Cap 15 USD for each one-hour composite run (owner ask 5).** Asked for 2-hour runs; the plan keeps the same cap for one hour. Review 1254 suggested 40 to 50 USD; with Claude Code on Haiku 15 should hold.
3. **Phase 5 tiers (conductor, owner).** A tier needs `full` rows for direct, gateway (if any) and dodger. The extra harness rows of Task 13 run after the tag, so phase 5 harnesses ship in 1.0.0 as "not verified: no full rows" unless the composite 10-minute wave already ran them `full` on the HF pad tree; their tiers can move in `1.0.1`.
4. **Kill gap bound (PRD-12, PRD-02).** `check.py` fails any plain kill gap of 2 s or more; X4 allows 5 s under load. `longsum.py` judges X4; `check.py` is left alone, so an A5 kill gap of 2 to 5 s costs the release check its Codex kill row and Task 8 Step 4 runs `kill-codex.toml` (10 minutes). A PRD-12 per-run bound would remove that fallback.
5. **Freeze time (owner ask 6), Mac runner minutes and a Mac smoke slot (ask 7), gateway credit (ask 9)** gate Waves L and F.

## Execution

Lane script: `docs/plans/2026-10-30-v1.0/conductor/gc-lane.js`, one lane per independent task group. Code lanes get no box per fix round; reviewers are read-only; each slice gets one box at its end (Task 5).

- **Parallel from the start (Mon 2 Nov):** lane `prd-12-release-rows` (Tasks 1, 2, 3 in order, one branch) and lane `conductor-longsum` (Task 4). Both join the code-wave merge train (Task 5), which also carries the mini-test rig slice.
- **Waits on H2** (composite 10-minute wave's fix round merged and built): Task 6 Step 1 rides that build; Task 7 runs three lanes at once (`long-a2`, `long-a5`, `long-a3`); Task 6 Steps 2 and 3 run beside them.
- **Waits on the freeze (Tue 3 Nov 18:00Z) and Task 7:** Task 8 Steps 1 to 4 in order; Task 8 Steps 8 and 9 in parallel with Step 3 onward; Task 9 Step 1 after Task 8 Step 4's rows end, then Task 8 Steps 5 to 7; Task 9 Steps 2 to 4 in parallel with Task 8 Step 6.
- **Waits on HR:** Tasks 10 and 11 (one docs lane, in order).
- **Owner:** Task 12. **Waits on the tag `v1.0.0`:** Task 13, Steps 2 and 3 as row lanes.

| Task | Work | Model |
|---|---|---|
| 1, 2, 3 | Write tests, implement, commit | sonnet |
| 4 | Write tests, implement `longsum.py` | sonnet |
| 1 to 4 | Two read-only reviewers per lane | default (most capable) |
| 5 | Slice box, fix round, merge-train assembly and bisect | sonnet |
| 5 | Queue ticks and `.res` waits | haiku |
| 6 Step 2 | First-output read (trust gate) | default (most capable) |
| 6 Steps 1, 3; 7 | Start, watch, judge and clean up the three composite runs (one lane each) | sonnet |
| 7 Step 4 | Five-minute `--live` ticks and spend lines | haiku |
| 7, 8 | Fix lanes for a named FAIL cause in recorder, masker or proxy code | default (most capable) |
| 7, 8 | Fix lanes for pad, connector or rig data causes | sonnet |
| 8 Steps 1 to 4, 6 to 9 | Freeze, gate-build box, release-check list, missing rows, table, N5, staging, macOS run | sonnet |
| 8 Step 5 | Exec verdicts (watcher) | sonnet |
| 8 Step 6, 9 Step 1 | `keyscan.py` runs, N6 box | haiku (keyscan), sonnet (N6 box) |
| 9 Steps 2, 3 | N7 and N9 greps and leftovers | haiku |
| 9 Step 4 | Newcomer lane | sonnet |
| 10, 11 | `ACCEPTANCE.md`, `MORNING.md`, `HANDOFF.md` | sonnet |
| 11 Step 1 | `scribe.sh` catch-up | haiku |
| 11 Step 3 | N8 fresh reader | sonnet |
| before HR | Final whole-branch review of `v1.0/integration` | default (most capable) |
| whole plan | Three-hourly reviewer | default (most capable) |
| 13 | Big-test row lanes, results, report lines | sonnet |
| 13 | Watch ticks, N9 again | haiku |

## Self-review

1. Spec coverage: three one-hour composite runs, one direct or gateway each, cap 15, in parallel on H2 (Tasks 3, 7); kills at 10 and 30, recover at 45, 15 or more subagents (Tasks 3, 4, 7); freeze, gate full and one build on HF (Task 8); release rows limited to what `--release-check` lists, the Codex kill row and exec verdicts folded into A5 (Tasks 3, 8); N5 (Task 8 Step 7); staging and macOS (Task 8); N6, N7, N9, newcomer (Task 9); tiers and PAD-RESULTS (Tasks 2, 8); ACCEPTANCE (Task 10); MORNING, N8, NIGHT-LOG, HANDOFF (Task 11); owner release steps before the big tests (Task 12); core set, A4, extra harness rows and A1 on `v1.0.0` (Task 13); Execution with a model per task. Gaps closed: rows never carried `goalcup` or `pad_trusted_at` (Task 1). Removed as owned elsewhere: the per-row cap and the Codex gateway pin (mini-test plan Tasks 1 and 5).
2. Step scan: code steps name file, signature and pinned values; run steps give settings, command, evidence and stop rules.
3. Type consistency: `build_commit`, `goalcup_id`, `os_id`, `trusted_at`, `tier_note`, `summary`, `live`, `markdown`, the run file names, `cap_usd` and `$GC_NIGHT/builds/<commit8>/commit` match across tasks and the mini-test plan.
4. Review Focus: each of the five has a test in its owning task (Tasks 1, 3, 1, 4, 4).
5. Proportion: no bodies except one run file and the test assertions; two tasks and two run files cut; the length added is the Execution section.
