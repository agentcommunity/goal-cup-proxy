# Lane K: Open-source readiness and canary
**Wave:** 1 (K.1 to K.5), then 3 (K.6, K.7)   **Depends on:** none for wave 1; wave 3 needs every other lane merged into `v1.1/integration` and lane G's move of `pad-rows.jsonl` / `PAD-RESULTS.md`   **Branch:** fix/oss
**Covers findings:** oss-1, oss-2, oss-3, oss-6 (dropped), oss-7 (dropped), oss-8 (dropped), arch-9, docs-3, MULTI-VERSION-PLAN task 8
**Owns files:** `NOTICE`, `AGENTS.md` line 73 only, `crates/goalcup/tests/mask_corpus/**`, `crates/goalcup-verify/tests/usage_fixtures/**` (extension: 12 lines, data only, no other lane edits it), `scripts/check-links.py`, `scripts/test_check_links.py`, `docs/harnesses/{codex,claude-code,amp}-cli.md` (the `HARNESS-SETUP` anchor only), new `scripts/pad/canary`, new `scripts/pad/test_canary.py`, `docs/OPERATING.md` (one new section before `## Bad days`)
**Shared files (only these parts):** `scripts/gate.sh` (two added `step` lines, K.7); wave 3 only: the dangling references the deletion leaves in `README.md` (lines 232-235), `AGENTS.md` (26-27, 40), `TODO.md` (8, 25, 41, 47, 50, 53, 58-59, 82-84), `docs/DECISIONS.md`, `docs/harnesses/*.md` (Pad result line), `docs/LOCAL-RESEARCH-HANDOFF.md`, `docs/RESEARCH-TARGET.md`, `docs/UPLOAD-API.md:576`
**Produces (for other lanes):** `scripts/pad/canary` (exit 0 nothing new, 1 new build, 2 lookup failed); the `archive` branch on origin; a gate that fails on `/Users/<name>` and `/home/<name>` outside an allowlist and on a broken relative link or anchor in tracked Markdown
**Consumes:** lane G: `connectors/tiers.toml` rendered as `[<id>]` with `verified = [{ version = "...", os = "..." }, ...]` (K.4 also reads the old `version = "..."` string, so it works before and after G); G moves `ROWS_REL` and `RESULTS` in `scripts/pad/table:13-14` out of `docs/plans` and cleans the 3 signin run files (`core-claude-code-signin.toml`, `core-codex-signin.toml`, `kill-codex-signin.toml`, each `goalcup_home = "/Users/team/gc-signin/home"`)

## Dropped
- oss-6 CHANGELOG: `release.yml:131` already runs `gh release create --generate-notes`; a second file is a ritual to keep in step.
- oss-6 SUPPORT: `.github/ISSUE_TEMPLATE/config.yml` already carries the "Help with your run" contact link and `README.md:241` the issues link.
- oss-6 cargo-deny: `scripts/third-party-licenses.py` fails the release on a licence outside `ALLOWED`, and `.github/dependabot.yml` already covers advisories weekly. A CI job is also off the table (lean CI).
- oss-7 signed releases: after the event. `SECURITY.md` and `packaging/README.md:36` already say the binaries are unsigned and not notarized; the plan adds nothing before the event.
- oss-8 secret-scan allowlist: no scanner runs in this repo (gate, CI, hooks). The only hit of `sk-[A-Za-z0-9_-]{24,}` in tracked code is the ordinary word string at `mask_rules.rs:593`; test keys are built from pieces (`mask_corpus/common.rs` header). An allowlist for a scanner that does not exist is config for nothing. If the owner turns on GitHub push protection, add the allowlist then.
- oss-1 repo-name half: refuted in the report (`goal-cup-proxy` is canonical).
- oss-4 staging host: cosmetic, single owner, not in this lane's list.

## Review focus
1. The scrub changes a byte count, so a masker test that checks offsets or sizes fails quietly later. K.3 compares the corpus byte count before and after; the replacement is the same length (`team` to `user`).
2. A username survives in another spelling (the `-Users-team-...` Claude project-folder form, a JSON-escaped path). K.3 greps both forms, plus the bare word `team`, after the scrub.
3. The canary finds the registry unreachable and looks like "nothing new" during the event. K.4 tests that a failed lookup prints `lookup failed` and exits 2, and that a new build elsewhere still exits 1.
4. `tiers.toml` has no verified build yet (today every row is `version = ""`). K.4 tests that this reports the latest build as NEW and does not crash.
5. The deletion leaves a live document linking into a removed folder, or `scripts/pad/table` reading a deleted file. K.6 ends with the link check clean and `table --release-check` run once against HEAD.

---

### Task K.1: NOTICE and AGENTS.md:73 name real folders   (size S, PARALLEL-OK)
**Files:** Modify `NOTICE:9-13`, `AGENTS.md:73`
**Interfaces:** none
- [ ] Step 1: Write the failing check: `git grep -n "spikes" -- NOTICE AGENTS.md` must print nothing; today it prints `NOTICE:11` and `AGENTS.md:73`.
- [ ] Step 2: Run it, expect FAIL: `git grep -n "spikes" -- NOTICE AGENTS.md` prints those two lines.
- [ ] Step 3: Implement. In `NOTICE` replace the bullet "The directories spikes/*/fixtures ..." so it names `crates/goalcup/tests/mask_corpus`, `crates/goalcup-verify/tests/usage_fixtures` and `scripts/pad/fixtures` as the redacted captures. Drop the sentence "see each fixtures README" (no such README exists: `find crates scripts/pad/fixtures -iname 'README*'` is empty) and say only what is true: system prompts and tool descriptions are trimmed, names and quoted text stay with their owners. In `AGENTS.md:73` delete "`spikes/*/out/` (until PRD-13 deletes `spikes/`), or" so the line lists `/pad-out/`, `/harnesses/`, `target/`, a key, or a real run's record.
- [ ] Step 4: Run, expect PASS: `git grep -n "spikes" -- NOTICE AGENTS.md` prints nothing, and `for d in crates/goalcup/tests/mask_corpus crates/goalcup-verify/tests/usage_fixtures scripts/pad/fixtures; do test -d $d || echo MISSING $d; done` prints nothing.
- [ ] Step 5: Commit: `git add NOTICE AGENTS.md; git commit -m "docs: NOTICE and AGENTS name the real fixture folders"`

### Task K.2: Fix the three HARNESS-SETUP anchors   (size S, PARALLEL-OK)
**Files:** Modify `docs/harnesses/amp-cli.md:75`, `docs/harnesses/claude-code-cli.md:74`, `docs/harnesses/codex-cli.md:81`
**Interfaces:** none
- [ ] Step 1: Write the failing check: `python3 scripts/check-links.py 2>&1 | grep -E "harnesses/(amp|claude-code|codex)-cli.md"` must print nothing.
- [ ] Step 2: Run it, expect FAIL: three lines, `../HARNESS-SETUP.md#amp`, `#claude-code`, `#codex` "(no such anchor)".
- [ ] Step 3: Implement. In each of the three lines change the target to `../HARNESS-SETUP.md#what-setup-writes-per-harness` (the table at `docs/HARNESS-SETUP.md:74` is the per-harness section; there are no per-harness headings, so adding headings would be more text for the same fact).
- [ ] Step 4: Run, expect PASS: the Step 1 command prints nothing. (The check as a whole still reports the archive-folder links; K.7 wires it after they are gone.)
- [ ] Step 5: Commit: `git add docs/harnesses/amp-cli.md docs/harnesses/claude-code-cli.md docs/harnesses/codex-cli.md; git commit -m "docs: point the three setup links at the per-harness table"`

### Task K.3: Scrub the maintainer's home path from the test corpora   (size M, PARALLEL-OK)
**Files:** Modify every file under `crates/goalcup/tests/mask_corpus/**` and `crates/goalcup-verify/tests/usage_fixtures/**` that holds `Users/team` or `Users-team`; no Rust file changes.
**Interfaces:** none
- [ ] Step 1: Record the baseline. Run `git ls-files -z crates/goalcup/tests/mask_corpus crates/goalcup-verify/tests/usage_fixtures | xargs -0 cat | wc -c > $SCRATCH/corpus-bytes.before` and `cargo nextest run -p goalcup --test mask_corpus --test mask_rules --test mask_values` and `cargo nextest run -p goalcup-verify`. Expect all PASS before any edit (if not, stop: the lane starts from a red tree).
- [ ] Step 2: The failing check: `git grep -c -E "Users[/-]team" -- crates/goalcup/tests/mask_corpus crates/goalcup-verify/tests/usage_fixtures` must print nothing (exit 1). Run it, expect FAIL: it lists 12 files in `mask_corpus` and 2 in `usage_fixtures` (about 4,600 hits; the report's 1,846 counts only `mask_corpus` lines).
- [ ] Step 3: Implement with one throwaway command, not committed: `git ls-files -z crates/goalcup/tests/mask_corpus crates/goalcup-verify/tests/usage_fixtures | xargs -0 perl -pi -e 's#Users([/-])team#Users$1user#g'`. Same length, so every offset holds. It covers `/Users/team/...` and the Claude project-folder form `-Users-team-goalcup-spikes-...`.
- [ ] Step 4: Run, expect PASS, all four: (a) the Step 2 grep prints nothing; (b) `git grep -n -w team -- crates/goalcup/tests/mask_corpus crates/goalcup-verify/tests/usage_fixtures` prints nothing; (c) `git ls-files -z crates/goalcup/tests/mask_corpus crates/goalcup-verify/tests/usage_fixtures | xargs -0 cat | wc -c` equals `$SCRATCH/corpus-bytes.before`; (d) the three nextest commands of Step 1 PASS again. If (d) fails, a test depended on the literal path: stop and report which test; do not edit Rust in this lane.
- [ ] Step 5: Commit: `git add crates/goalcup/tests/mask_corpus crates/goalcup-verify/tests/usage_fixtures; git commit -m "test: scrub the maintainer home path from the corpora"`

### Task K.4: The canary   (size M, PARALLEL-OK)
**Files:** Create `scripts/pad/canary` (executable, `#!/usr/bin/env python3`, stdlib, `tomllib`), `scripts/pad/test_canary.py`
**Interfaces:**
- Consumes: `connectors/tiers.toml` (shape above under Consumes). No import from lane F's `harness_version.py`: a local one-line `norm` keeps this lane independent.
- Produces: `SOURCES = {"claude-code-cli": ("npm", "@anthropic-ai/claude-code"), "codex-cli": ("npm", "@openai/codex"), "amp-cli": ("npm", "@ampcode/cli"), "hermes-cli": ("github", "NousResearch/hermes-agent")}` (CORE harnesses plus Amp, the sources named in `scripts/pad/harnesses/*.toml` `install` and `hermes-cli.toml:13`); `def norm(text) -> str` (first `\d+(\.\d+)+(-[0-9A-Za-z.]+)?` token, so `v0.22.6` and `0.0.1793664053-ga6248e (2026-11-03)` both normalise); `def known(tiers: dict, cid: str) -> list[tuple[str, str]]` ((version, os) pairs; `[]` when none; reads `verified`, else a non-empty `version` as `(version, "?")`); `def latest(cid: str, run) -> str` where `run(argv: list[str]) -> str` is the only network seam (`npm view <pkg> version`, or `curl -fsS https://api.github.com/repos/<repo>/releases/latest` read as JSON `tag_name`); `def report(tiers, run) -> tuple[list[str], int]`. CLI: `scripts/pad/canary [--tiers PATH] [--only ID]`, prints the lines, exits with the code.
- Output line, one per harness: `claude-code-cli latest 2.1.320 verified 2.1.312 (macos) NEW`, or `... verified none NEW`, or `... known`, or `claude-code-cli lookup failed: <reason>`. Exit 1 if any NEW, else 2 if any lookup failed, else 0.
- [ ] Step 1: Write the failing tests in `scripts/pad/test_canary.py`, loading `canary` the way `test_table_note.py` loads `table` (SourceFileLoader). A fake `run` returns canned strings; no network. Tests, each with these exact values:
  - `test_new_build_exits_1`: tiers `claude-code-cli` verified `[{version="2.1.312", os="macos"}]`, npm answers `2.1.320` -> line ends `NEW`, code 1.
  - `test_known_build_exits_0`: same tiers, npm answers `2.1.312` -> `known`, code 0.
  - `test_other_os_still_known`: verified `[{version="2.1.312", os="linux"}]`, answer `2.1.312` -> `known`.
  - `test_amp_suffix_matches`: verified `0.0.1793664053-ga6248e`, npm answers `0.0.1793664053-ga6248e` -> `known`; npm answers `0.0.1793664099-gb1c2d3` -> `NEW`.
  - `test_hermes_v_prefix`: GitHub JSON `{"tag_name": "v0.22.6"}`, verified `0.22.5` -> latest shown `0.22.6`, `NEW`.
  - `test_nothing_verified_is_new`: tiers `[codex-cli] version = ""` (today's seed) -> `verified none NEW`, code 1.
  - `test_lookup_failure_exits_2`: `run` raises for `codex-cli` only, the others known -> a `codex-cli lookup failed` line, code 2.
  - `test_failure_does_not_hide_new`: `run` raises for `codex-cli`, `claude-code-cli` is new -> code 1 and both lines printed.
- [ ] Step 2: Run, expect FAIL: `python3.11 -m unittest scripts/pad/test_canary.py` (or `python3` when it is 3.11 or later) fails with `FileNotFoundError` for `scripts/pad/canary`.
- [ ] Step 3: Implement `scripts/pad/canary` to the signatures above. `report` loops `SOURCES` in order, never raises (a lookup exception becomes the `lookup failed` line), and returns the code. Header comment: runs on the maintainer's Mac, Python 3.11 for `tomllib`, needs `npm` and `curl` on PATH.
- [ ] Step 4: Run, expect PASS: `python3.11 -m unittest scripts/pad/test_canary.py` (8 tests), then `python3.11 -m unittest discover -s scripts/pad -p 'test_*.py'` (the gate's own command, so the new file joins by name with no `gate.sh` edit).
- [ ] Step 5: Live check once, by hand: `python3.11 scripts/pad/canary` prints four lines with a real latest version each; record the output in the PR, not in a file. After lane G merges, repeat against its rendered `tiers.toml`; if `verified` has another shape, change only `known`.
- [ ] Step 6: Commit: `git add scripts/pad/canary scripts/pad/test_canary.py; git commit -m "pad: canary reports new harness builds"`

### Task K.5: Event-window runbook in OPERATING.md   (size S)
**Files:** Modify `docs/OPERATING.md`: insert a new `## New harness builds during the event` section directly before `## Bad days` (line 218). Other lanes edit other parts of this file; touch nothing else.
**Interfaces:** Consumes the exact command and exit codes of K.4.
- [ ] Step 1: Write the failing check: `grep -c "scripts/pad/canary" docs/OPERATING.md` must print at least 2. Run it, expect FAIL: prints 0.
- [ ] Step 2: Run it (it is the check above), expect `0`.
- [ ] Step 3: Implement the section, short, in this order:
  - What it is: goalcup never refuses a build; the canary only tells the maintainer that a build exists that no pad row has verified.
  - Run: `python3.11 scripts/pad/canary` on the maintainer Mac every 2 hours for Claude Code, Codex and Hermes, hourly for Amp (about 7 npm builds a day, `scripts/pad/harnesses/amp-cli.toml` header). Exit 0 nothing to do; 1 a new build, named; 2 a lookup failed, run it again before assuming nothing is new.
  - Classify a new build within 2 hours (PAD, serial, one Mac): install that exact build from the `install` line of its `scripts/pad/harnesses/<id>.toml` with the version changed, run `python3.11 scripts/pad/pad scripts/pad/runs/micro.toml` for that harness. Amp has no pad key (`key_kind = "none"`): install the build by hand and run it as in `docs/harnesses/amp-cli.md` section 1.
  - Three outcomes: records fine, nothing to announce; wire fine and the native reader drifts, note it and re-verify after the event (the raw wire record is the evidence); model unreachable or a prompt blocks, publish the downgrade line (the `install` line of the last verified build) on the event page.
  - Never disqualify or refuse on a version. Affected entries are found by connector and the version word in the `start` event, then flagged.
- [ ] Step 4: Run, expect PASS: `grep -c "scripts/pad/canary" docs/OPERATING.md` prints 2 or more, and `python3 scripts/check-links.py 2>&1 | grep "docs/OPERATING.md"` prints nothing.
- [ ] Step 5: Commit: `git add docs/OPERATING.md; git commit -m "docs: runbook for new harness builds during the event"`

---
Wave 3 starts here. Run K.6 and K.7 only after every other lane's PR is merged into `v1.1/integration` and the full gate has passed once on it. They run in this order, by one agent, in the `fix/oss` worktree rebased onto `v1.1/integration`.

### Task K.6: Move the history folders to an `archive` branch   (size L)
**Files:** Delete `docs/plans/`, `docs/research/`, `docs/archive/`, `docs/superpowers/` (325 files, about 8.5 MB; includes this plan folder). Modify the live references listed under Shared files.
**Interfaces:** Consumes lane G: `grep -n "docs/plans" scripts/pad/table` prints nothing, and `scripts/pad/table` reads `pad-rows.jsonl` and writes `PAD-RESULTS.md` outside the four folders. Produces: origin branch `archive` holding every deleted file at the tip it was deleted from.
- [ ] Step 1: Write the failing check: `git grep -n -E "docs/(plans|research|archive|superpowers)" -- . ':!crates/goalcup/tests/mask_corpus' ':!server/worker/sql' ':!docs/plans' ':!docs/research' ':!docs/archive' ':!docs/superpowers'` must print nothing, and `python3 scripts/check-links.py` must exit 0. Also confirm live content in the folders is already gone: `docs/research/NOTIFICATIONS.md` is linked from `docs/UPLOAD-API.md:576` and `server/worker/sql/goal-cup-tracer-upload.sql:2515`; if the owner wants it live, `git mv docs/research/NOTIFICATIONS.md docs/NOTIFICATIONS.md` first and fix those two (owner question 1).
- [ ] Step 2: Run both, expect FAIL: about 30 reference lines (README, AGENTS, TODO, DECISIONS, 15 sheet "Pad result" lines, two pad harness comments) and about 70 broken links.
- [ ] Step 3: Implement, in this order, one action each:
  1. `git branch archive HEAD` then `git push origin archive` (public branch; the content is already public in history, so nothing new is exposed; owner question 2).
  2. `git rm -r -q docs/plans docs/research docs/archive docs/superpowers`.
  3. Run `python3 scripts/check-links.py`; for every reported link whose target resolved into a removed folder, rewrite it to `https://github.com/agentcommunity/goal-cup-proxy/blob/archive/<repo-relative path>` with a throwaway script in `$SCRATCH` that reads the checker's `file:line: target` output (keeps the meaning, no hand edits to `docs/DECISIONS.md`).
  4. By hand, the non-link mentions: `README.md:232-235` and `AGENTS.md:26-27,40` say the history is on the `archive` branch and drop "Plans are in docs/plans"; delete `TODO.md:82-84` (the history purge is this task) and fix `TODO.md` 8, 25, 41, 47, 50, 53, 58-59 the same way; the 15 sheet "Pad result" lines point at the new location lane G chose for `PAD-RESULTS.md`; the two `scripts/pad/harnesses` comments name the archive branch.
  5. Leave the `server/worker/sql` comment paths (comments in an applied migration; changing them changes a file the Worker lane owns).
- [ ] Step 4: Run, expect PASS: the Step 1 grep prints nothing; `python3 scripts/check-links.py` exits 0 (the two non-archive broken anchors `docs/LOCAL-RESEARCH-HANDOFF.md:135 OPERATING.md#the-matrix` and `docs/RESEARCH-TARGET.md:48 ../TODO.md#wp0-measure` are fixed here by removing the link, or the file if `docs/RESEARCH-TARGET.md` is on the owner's purge list at `TODO.md:82`); `python3.11 scripts/pad/table --release-check HEAD` runs without `FileNotFoundError` (it may report missing rows; it must not read a deleted path); `git ls-tree -r origin/archive --name-only | grep -c "^docs/plans/"` is above 250.
- [ ] Step 5: Commit: `git add -A docs README.md AGENTS.md TODO.md scripts/pad/harnesses; git commit -m "docs: move plans, research and archive to the archive branch"`

### Task K.7: Gate two guards: links and personal paths   (size S)
**Files:** Modify `scripts/gate.sh`, add two `step` lines after the pad unittest step (line 78). Nothing else in that file.
**Interfaces:** Consumes K.6 (links clean) and lane G (signin run files clean).
- [ ] Step 1: Write the failing check: plant a probe. Create an untracked-then-added file `probe-path.txt` containing `/Users/alice/x`, then run `sh scripts/gate.sh fast`; after Step 3 it must FAIL on the path step and name the file. Also add a bad relative link `[x](nope.md)` to a scratch copy of `README.md` to prove the link step fails (do both, remove both after).
- [ ] Step 2: Run the two probes against the current gate, expect both to PASS the gate (no guard exists): that is the failure of this task.
- [ ] Step 3: Implement the two lines (decision for arch-9 and docs-3: keep and wire `check-links.py`; it runs in 0.8 s, stdlib only, and the stranger's first view is the README links; `scripts/test_check_links.py` stays a by-hand test, not in the gate's named list, because it guards none of the seven points):
  - `step python3 scripts/check-links.py`
  - `step sh -c "! git grep -n -E '/(Users|home)/[A-Za-z0-9_.-]+' | grep -v -E '/(Users|home)/(Shared|placeholder|user|p|you|h|u|x|runs|participant|secret-root|hermes|opencode|openclaw|claude|codex)([^A-Za-z0-9_.-]|$)'"`
  The allowlist is exactly the names that `git grep -o` finds today after K.3 (checked in Step 4); a new name is a deliberate edit of this line.
- [ ] Step 4: Run, expect PASS: with the probes removed, `sh scripts/gate.sh fast` ends `gate fast: PASS`; with each probe present it ends `gate fast: FAIL` and the step output names the probe. Before this, `git grep -o -h -E '/(Users|home)/[A-Za-z0-9_.-]+' | sort -u` shows only the allowlisted names.
- [ ] Step 5: Commit: `git add scripts/gate.sh; git commit -m "gate: check links and personal paths"`

## Owner questions
1. Is `docs/research/NOTIFICATIONS.md` live (UPLOAD-API.md:576 says it lists every participant message)? If yes it moves to `docs/` before K.6; this plan assumes nothing else in the four folders is live.
2. OK to push a public `archive` branch (K.6 step 3.1)?
3. K.4 reads Hermes from GitHub releases (`NousResearch/hermes-agent`, the repo of the pad install line). Confirm that is the build participants get, not a PyPI package.
