# Lane G: Test pad and release gate
**Wave:** 1 (Tasks G.1 to G.14), 2 (Tasks G.20 to G.24, the version matrix)   **Depends on:** lane F for Task G.5 only (`scripts/pad/harness_version.py`); wave 2 also on lane I (overlays)   **Branch:** fix/pad-release
**Covers findings:** cr-scripts-x1 = versions-x1, cr-scripts-1, -3, -4, -7, -8, -9, -10, -11, -12, versions-3 (wave 2), versions-12, versions-x2 (pad side), versions-x3, MULTI-VERSION-PLAN tasks 1 (pad time), 4, 16, oss-3 (rows path only), oss-4, oss-2 (pad run files only)
**Owns files:** `scripts/pad/*` (not `harness_version.py`), `scripts/release.sh`, `scripts/fetch-run.sh`, `scripts/test_fetch_run.py`, `.github/workflows/release.yml`, `connectors/tiers.toml` (generated), `scripts/gate.sh` line 55 only, new `scripts/pad/rows/`
**Shared files (only these parts):** `scripts/gate.sh:49-58` (the python3.11 probe); `scripts/README.md:28` (the `table` line, one sentence)
**Produces (for other lanes):**
- Rows live at `scripts/pad/rows/pad-rows.jsonl` and `scripts/pad/rows/PAD-RESULTS.md` (`table.ROWS_REL`, `table.RESULTS`). Lane docs must not move or delete the old `docs/plans/2026-10-30-v1.0/pad-rows.jsonl`; Task G.1 moves it with `git mv`.
- `connectors/tiers.toml` rows gain `os = "<pad os id>"` (for example `darwin-arm64`; same string as a row's `os`). Rust's `Row {tier, version}` ignores it today; the owner of `tier_line` (text.rs) may read it. Wave 2 adds `[[<id>.builds]]` tables, also ignored by the current reader.
- Row field `pad_only: list[str]` (why strings) written by `check.py`; a non-empty list means the row never counts.
- Harness-file key `pad_only = [{ match = "<spec>", why = "<text>", files = { "<name>" = "<src>" } }]` inside `[direct]` or `[gateway]`.
- Env var `PAD_VERSION` given to every `version_cmd` and `install` (wave 1: version_cmd only).
**Consumes:** `scripts/pad/harness_version.py`: `parse(line: str) -> str | None`, `matches(text: str, spec: str) -> bool` (lane F). Lane I: `[[versions]]` overlays (no CA keys in rows; see index ruling 1).

## Dropped
- cr-scripts-2, -5, -6: `packaging/install.sh`, not in this lane's files.
- oss-2 (`mask_corpus` part) and oss-3 (moving `docs/plans`, `docs/research`, `docs/archive`): other lanes; this lane only frees the rows path and the three run files.
- versions-x2 "table refuses verified for versions in `version_notes`": replaced by `pad_only` (G.5), because Hermes `version_notes` goes away in lane I.

## Review focus
1. The tiers re-render must read harness files (`not_supported`, `not_verified`, `tier_note`, `[gateway]`) from the checked commit, not the working tree. Test in G.4.
2. `tier_for` and `release_check` must agree: a row set the release check accepts must render `verified`, and one it refuses must not. Today Claude (gateway FAIL, release-exempt) and Hermes (C3 INCONCLUSIVE "not exercised") disagree. Test in G.2.
3. A committed row without the `pad_only` key (all current rows) counts. They are replaced by the final run, but the behaviour is pinned. Test in G.5.
4. `goalcup_home` sequential runs: a Stop between cases must skip the later case, not start it. Test in G.6.
5. `PAD_STAGING_URL` must not point the pad at production: refuse a host whose first label lacks "staging". Test in G.12.

### Task G.1: Rows to a stable path, first test file   (size S)
**Files:** Modify `scripts/pad/table:4-14,199-202`, `scripts/README.md:28`; `git mv docs/plans/2026-10-30-v1.0/pad-rows.jsonl scripts/pad/rows/pad-rows.jsonl`; `git mv docs/plans/2026-10-30-v1.0/PAD-RESULTS.md scripts/pad/rows/PAD-RESULTS.md`; Create `scripts/pad/test_table_release.py`
**Interfaces:** Produces `table.ROWS_REL == "scripts/pad/rows/pad-rows.jsonl"`, `table.RESULTS == ROOT / "scripts/pad/rows/PAD-RESULTS.md"`. The test file defines helpers used by G.2 to G.5: `load(name, file)` (copy from `test_table_note.py`), `mkrepo(tmp, rows, tiers_text=None, harness=None) -> sha` (git init in a temp dir, commit rows at `table.ROWS_REL`, patch `table.ROOT`), `green_rows(base_sha) -> list[dict]` (8 core tier rows: claude `direct` and `dodger`, codex and hermes `direct`, `gateway`, `dodger`; one codex `direct` full row with `variant=["kill-recorder@3"]` and `exec={"h2-parallel.sh": "PASS", "long-tunnel.sh": "PASS"}`; every row `result=PASS`, `goalcup="1.0.0+"+base_sha`, `os="darwin-arm64"`, `harness_version` claude `2.1.312`, codex `0.165.0`, hermes `0.22.5`).
- [ ] Step 1: Write the failing test `PathAndPass.test_green_rows_pass_at_the_stable_path`: `mkrepo` with `green_rows`; assert `table.ROWS_REL == "scripts/pad/rows/pad-rows.jsonl"`, `table.release_check(sha) == 0`; `PathAndPass.test_missing_rows_file`: repo without the file, assert return 1 and stdout `no committed scripts/pad/rows/pad-rows.jsonl at <sha>`.
- [ ] Step 2: Run `python3 -m unittest discover -s scripts/pad -p 'test_table_release.py' -k stable_path`; expect FAIL (`ROWS_REL` still points into `docs/plans`).
- [ ] Step 3: Set `ROWS_REL` and `RESULTS` to the new paths, run both `git mv`, change the README sentence to name `scripts/pad/rows/`.
- [ ] Step 4: Run `python3 -m unittest discover -s scripts/pad -p 'test_table_release.py'` and then `python3 scripts/pad/table --release-check HEAD`; expect tests PASS and `release check: pass at <sha>`.
- [ ] Step 5: Commit `git add -A scripts/pad/table scripts/pad/rows scripts/pad/test_table_release.py scripts/README.md docs/plans/2026-10-30-v1.0; git commit -m "pad: pad-rows.jsonl and PAD-RESULTS.md move to scripts/pad/rows"`.

### Task G.2: One rule for "counts", shared by tiers and release check   (size M)
**Files:** Modify `scripts/pad/table:60-85` (`tier_for`), `scripts/pad/table:135-178` (`release_check`); Test `scripts/pad/test_table_release.py`
**Interfaces:** Produces module-level `passed(r) -> bool` (moved out of `release_check`, unchanged logic), `counts(r) -> bool` (`passed(r) and not g(r.get("facts"), "masked_upstream") and not r.get("pad_only")`), `NO_TIER_ROW = {("claude-code-cli", "gateway")}` (the owner's 3 Nov exemption, used by both). `tier_for(cid, rows, lines=None)`: `lines` is `harness_lines` when `None` (looked up at call time so `mock.patch.object(table, "harness_lines")` keeps working).
- [ ] Step 1: Write failing tests: `Agreement.test_claude_gateway_fail_does_not_block_claude` (`green_rows` plus a newer claude gateway row with `result="FAIL"`; assert `table.tier_for("claude-code-cli", rows)[0] == "verified"` and `release_check(sha) == 0`); `test_hermes_c3_not_exercised_counts` (hermes rows `result="INCONCLUSIVE"`, `checks.C3="INCONCLUSIVE"`, others PASS, `why=["C3 INCONCLUSIVE: exercise not done"]`; assert `tier_for("hermes-cli", rows)[0] == "verified"`); `test_masked_upstream_blocks_both` (one hermes row `facts.masked_upstream=True`; assert tier `not verified` with reason containing `masked upstream` and `release_check` output containing `C3 passed as masked upstream`); `test_exec_verdict_missing` (drop `exec` from the kill row; output contains `exec h2-parallel.sh verdict PASS: missing`).
- [ ] Step 2: Run `python3 -m unittest discover -s scripts/pad -p 'test_table_release.py' -k Agreement`; expect FAIL on the first two (`not verified`).
- [ ] Step 3: Move `passed` to module level, add `counts` and `NO_TIER_ROW`; `tier_for` skips `NO_TIER_ROW` cases and uses `counts` instead of `result != "PASS"` (reason text `"<case> masked upstream, not confirmed by the owner"` at rank 2 when `masked_upstream`); `release_check`'s loop and `need` default `ok` use the same two names; `tier_row` stays the selector.
- [ ] Step 4: Run the same command plus `python3 -m unittest discover -s scripts/pad -p 'test_table_note.py'`; expect PASS.
- [ ] Step 5: Commit `git add scripts/pad/table scripts/pad/test_table_release.py; git commit -m "table: tiers and release check share one counts rule"`.

### Task G.3: Tier rows carry the OS   (size S)   PARALLEL-OK after G.2
**Files:** Modify `scripts/pad/table:29-31,60-85,100-125`; `scripts/pad/test_table_note.py:27-33`; Test `scripts/pad/test_table_release.py`
**Interfaces:** `tier_for(...) -> tuple[str, str, str, str]` = `(tier, version, os, reason)`; `render_tiers` writes `os = "<os>"` between `version` and `reason`; "case rows on different versions" becomes "case rows on different versions or systems" when the set of `(harness_version, os)` pairs has more than one member; a case row without `os` gives `"no system on a case row"`.
- [ ] Step 1: Write failing tests `TierOs.test_verified_names_the_os` (hermes rows `os="darwin-arm64"` → `tier_for` equals `("verified", "0.22.5", "darwin-arm64", "")`), `test_two_systems_do_not_merge` (direct row `linux-x86_64`, others `darwin-arm64` → `("not verified", "", "", "case rows on different versions or systems")`), `test_render_has_os_line` (`render_tiers` output contains `os = "darwin-arm64"\nreason`).
- [ ] Step 2: Run `python3 -m unittest discover -s scripts/pad -p 'test_table_release.py' -k TierOs`; expect FAIL (tuple length 3).
- [ ] Step 3: Implement; update the unpacking in `render_tiers`, `results_md` (two sites) and the expected tuple in `test_table_note.py` to the 4-tuple.
- [ ] Step 4: Run `python3 -m unittest discover -s scripts/pad -p 'test_*.py' -k Tier`; expect PASS.
- [ ] Step 5: Commit `git add scripts/pad/table scripts/pad/test_table_note.py scripts/pad/test_table_release.py; git commit -m "table: a tier row names its OS"`.

### Task G.4: Release check re-renders tiers.toml; regenerate it   (size M)
**Files:** Modify `scripts/pad/table:55-58` (`harness_lines`), `:60-70` (`has_gateway`), `:135-178` (`release_check`), `:182-205` (`main`); regenerate `connectors/tiers.toml`; Test `scripts/pad/test_table_release.py`
**Interfaces:** `harness_lines(cid, sha=None) -> list[str]` (with `sha`: `git show <sha>:scripts/pad/harnesses/<cid>.toml`, `[]` if absent); `tier_for` and `has_gateway` pass `lines=lambda c: harness_lines(c, sha)` through. `release_check` compares `git show <sha>:connectors/tiers.toml` with `render_tiers({cid: tier_for(cid, rows, lines) for cid in CONNECTOR_IDS})`. `main()` with no `pad-out` rows renders from the committed `ROWS_REL` file when it exists (the seed only when neither exists).
- [ ] Step 1: Write failing tests `TiersDiff.test_matching_tiers_pass`, `test_seed_tiers_fail` (commit the all-"not verified" seed; assert return 1 and output contains `connectors/tiers.toml differs from the rows; run scripts/pad/table and commit it`), `test_missing_tiers_file` (output `connectors/tiers.toml missing at <sha>`), `test_harness_files_read_at_the_commit` (commit a hermes harness file with `tier_note = "n1"` and tiers rendered with that note; then change the working-tree file to `n2` without committing; assert return 0), `test_main_renders_from_committed_rows` (empty `pad-out`, rows file present: `table.main([])` writes tiers where codex is `verified`).
- [ ] Step 2: Run `python3 -m unittest discover -s scripts/pad -p 'test_table_release.py' -k TiersDiff`; expect FAIL (seed tiers pass today).
- [ ] Step 3: Implement as above; the diff item text is exactly the two strings in the tests.
- [ ] Step 4: Run `python3 -m unittest discover -s scripts/pad -p 'test_*.py'`; then `python3 scripts/pad/table` (renders from the committed rows), `git diff --stat connectors/tiers.toml`, `python3 scripts/pad/table --release-check HEAD` after committing; expect PASS and `release check: pass`. Also run `cargo nextest run -p goalcup tiers_file_parses`; expect PASS (the Rust reader ignores `os`).
- [ ] Step 5: Commit `git add scripts/pad/table scripts/pad/test_table_release.py connectors/tiers.toml; git commit -m "release check: tiers.toml must equal the rows' render; regenerate it"`.

### Task G.5: `pad_only` config never counts   (size M)   PAD for the Hermes rerun in G.14
**Files:** Modify `scripts/pad/pad-run:313-340` (setup), `:1122` (meta-box keys), `scripts/pad/check.py:932-934`, `scripts/pad/table` (`counts`, from G.2), `scripts/pad/harnesses/hermes-cli.toml:34-41`; Create `scripts/pad/harnesses/direct/hermes-ca.yaml`, `scripts/pad/harnesses/gateway/hermes-ca.yaml`; Modify `scripts/pad/harnesses/direct/hermes-config.yaml:11-16`, `scripts/pad/harnesses/gateway/hermes-config.yaml:11-16` (delete the CA block); Test `scripts/pad/test_pad_run_case.py`, `scripts/pad/test_table_release.py`
**Interfaces:** Consumes `harness_version.matches(text, spec)`. Harness file: `[direct]` and `[gateway]` each get `pad_only = [{ match = "0.22.6", why = "providers.goalcup-ca ssl_ca_cert, Hermes 0.22.6 ignores SSL_CERT_FILE (hermes-agent#148241)", files = { "config.yaml" = "direct/hermes-ca.yaml" } }]`. `Run.setup` merges each matching entry's files after the normal files and sets `self.st["pad_only"] = [why, ...]` (else `[]`). `check.py` row gets `pad_only=meta.get("pad_only") or []`.
- [ ] Step 1: Write failing tests: in `test_pad_run_case.py` `PadOnly.test_ca_block_only_on_0_22_6` (fake `sh` returns `Hermes Agent v0.22.5 (2026.9.1)` then `v0.22.6`; assert the merged `config.yaml` has no `goalcup-ca` for 0.22.5 and `st["pad_only"] == []`; for 0.22.6 it has `goalcup-ca` and `st["pad_only"]` has one string); in `test_table_release.py` `PadOnlyRows.test_pad_only_row_never_counts` (hermes rows with `pad_only=["x"]` → tier `not verified`, reason contains `pad-only config`, `release_check` output contains `hermes-cli direct full: result PASS, pad-only config`), `test_legacy_row_without_key_counts` (rows with no `pad_only` key → `verified`).
- [ ] Step 2: Run `python3 -m unittest discover -s scripts/pad -p 'test_pad_run_case.py' -k PadOnly` and `-p 'test_table_release.py' -k PadOnlyRows`; expect FAIL.
- [ ] Step 3: Implement the setup merge (spec match only through `harness_version.matches`), the `meta-box` key, the `check.py` key, the `counts` reason strings `"<case> pad-only config: <why>"`.
- [ ] Step 4: Run `python3 -m unittest discover -s scripts/pad -p 'test_*.py'`; expect PASS.
- [ ] Step 5: Commit `git add scripts/pad scripts/pad/harnesses; git commit -m "pad: pad_only config voids a row as evidence; Hermes CA block marked"`.

### Task G.6: Signed-in home runs one case at a time   (size S)   PARALLEL-OK
**Files:** Modify `scripts/pad/pad:1071-1074`; Test `scripts/pad/test_pad.py` (class `RunAll`)
**Interfaces:** `run_all(s, pad, harness, out_root, cases)` unchanged signature; `max_workers=1` when `s.get("goalcup_home")`.
- [ ] Step 1: Write failing test `RunAll.test_signed_in_home_runs_cases_in_order`: patch `pad.Run` with a class whose `walk` records start and end into a list with a 0.05 s sleep; `s={"goalcup_home": "/h"}`, cases `["direct","dodger"]`; assert the log is `["s direct","e direct","s dodger","e dodger"]`. `test_without_home_runs_together`: same without the key, assert the second `s` comes before the first `e`. `test_stop_skips_later_case`: `pad.STOP.set()` inside the first walk; assert the fake's second `walk` is still called once and sees `STOP.is_set()` (the real `walk` then skips every step); clear `STOP` in `tearDown`.
- [ ] Step 2: Run `python3 -m unittest discover -s scripts/pad -p 'test_pad.py' -k RunAll`; expect FAIL on the first test.
- [ ] Step 3: Implement `ThreadPoolExecutor(max_workers=1 if s.get("goalcup_home") else None)`.
- [ ] Step 4: Run `python3 -m unittest discover -s scripts/pad -p 'test_pad.py'`; expect PASS.
- [ ] Step 5: Commit `git add scripts/pad/pad scripts/pad/test_pad.py; git commit -m "pad: cases sharing a signed-in home run one after another"`.

### Task G.7: Release workflow refuses off-main tags and runs the pad check   (size S)   PARALLEL-OK
**Files:** Modify `.github/workflows/release.yml:100-115` (job `release`, before `publish`), `:7-10` (comment)
**Interfaces:** none. Two steps in job `release`, after checkout (which gets `fetch-depth: 0`): `git fetch --no-tags origin main` then `git merge-base --is-ancestor "$GITHUB_SHA" origin/main` (message `::error::tag commit is not on origin/main`), then `python3 scripts/pad/table --release-check "$GITHUB_SHA"`. The job already runs only for `push`. Build runs first; a bad tag costs four builds but publishes nothing. (Alternative: a separate first job; not chosen to keep one place.)
- [ ] Step 1: Verify the gap: `grep -c "is-ancestor\|release-check" .github/workflows/release.yml`; expect `0`.
- [ ] Step 2: Check the ancestor command by hand on a scratch clone: `git merge-base --is-ancestor $(git rev-parse HEAD) origin/main; echo $?` on a commit off main; expect `1`.
- [ ] Step 3: Add the two steps and the comment line.
- [ ] Step 4: Run `grep -c "is-ancestor\|release-check" .github/workflows/release.yml` (expect `2` or more) and `actionlint .github/workflows/release.yml` if installed (else `python3 -c "import yaml,sys; yaml.safe_load(open(sys.argv[1]))" .github/workflows/release.yml`).
- [ ] Step 5: Commit `git add .github/workflows/release.yml; git commit -m "release.yml: tag must be on origin/main and pass the pad release check"`.

### Task G.8: `release.sh prepare` never leaves a half-changed tree   (size S)   PARALLEL-OK
**Files:** Modify `scripts/release.sh:96-113`; Create `scripts/pad/test_release_sh.py` (lives in `scripts/pad/` because `gate.sh` discovers tests there; moving it to the named list at `gate.sh:85` is an owner question)
**Interfaces:** `restore()` becomes `trap restore EXIT` armed right after `git switch -q --detach origin/main`, disarmed (`trap - EXIT`) after the success `echo`; it runs `git checkout -q -- Cargo.toml Cargo.lock`, removes `Cargo.toml.new`, `git checkout -q "$start"`. The branch is created after the commit: `git commit` on the detached head, then `git switch -q -c "$branch"`.
- [ ] Step 1: Write failing test `ReleasePrepare.test_failing_commit_restores_the_tree`: temp bare `origin` and a clone with `Cargo.toml` (`version = "1.0.0"`), `scripts/release.sh` copied in, a fake `cargo` (exit 0) and `gh` on `PATH`, `.git/hooks/pre-commit` that exits 1; run `sh scripts/release.sh prepare 1.0.1`; assert non-zero exit, `git status --porcelain` empty, `git symbolic-ref --short HEAD` equals the start branch, `git branch --list 'release/*'` empty. `test_failing_cargo_restores` the same with `cargo` exiting 1.
- [ ] Step 2: Run `python3 -m unittest discover -s scripts/pad -p 'test_release_sh.py'`; expect FAIL on the commit case (branch `release/v1.0.1` left).
- [ ] Step 3: Implement the trap and the reorder.
- [ ] Step 4: Run the same command; expect PASS. Run `sh -n scripts/release.sh`.
- [ ] Step 5: Commit `git add scripts/release.sh scripts/pad/test_release_sh.py; git commit -m "release.sh: prepare restores the tree on any failure"`.

### Task G.9: Pad hardening: tar filter, atomic queue file, box gets tracked files only   (size S)   PARALLEL-OK
**Files:** Modify `scripts/pad/pad:153-156` (`extract`), `:251-262` (`downgrade_if_429`), `:684` (push-dir of `HERE`); Test `scripts/pad/test_pad.py`
**Interfaces:** `extract(tar, dest)` raises `Refused("python has no tar data filter (3.11.4 or later needed)")` when `tarfile` lacks `data_filter`. `downgrade_if_429` writes `queue.json.tmp` then `os.replace(tmp, path)`. New `stage_pad(dest, repo=REPO) -> list[str]`: copies `git ls-files -z -- scripts/pad` into `dest`, skipping paths under `scripts/pad/rows/`, `scripts/pad/fixtures/`, `scripts/pad/runs/` and files named `test_*.py`; returns the relative paths. Line 684 pushes `stage_pad`'s folder instead of `HERE`.
- [ ] Step 1: Write failing tests: `ExtractRefuses.test_no_data_filter` (`mock.patch.object(pad, "tarfile", types.SimpleNamespace(open=tarfile.open))` → `assertRaises(pad.Refused)`); `QueueAtomic.test_replace_is_used` (spy on `os.replace`; a 429-heavy `row.json`; assert called once with `(path + ".tmp", path)` and `queue.json` reads `{"downgraded": ["deepseek"]}`); `StagePad.test_untracked_and_rows_stay_out` (temp git repo with tracked `scripts/pad/pad-run`, `scripts/pad/rows/pad-rows.jsonl`, `scripts/pad/test_x.py`, untracked `scripts/pad/notes.txt`; assert result `["scripts/pad/pad-run"]`).
- [ ] Step 2: Run `python3 -m unittest discover -s scripts/pad -p 'test_pad.py' -k "ExtractRefuses or QueueAtomic or StagePad"`; expect FAIL.
- [ ] Step 3: Implement the three changes.
- [ ] Step 4: Run `python3 -m unittest discover -s scripts/pad -p 'test_pad.py'`; expect PASS.
- [ ] Step 5: Commit `git add scripts/pad/pad scripts/pad/test_pad.py; git commit -m "pad: refuse unfiltered tar, atomic queue.json, box gets tracked files only"`.

### Task G.10: `fetch-run.sh` quotes its curl config   (size S)   PARALLEL-OK
**Files:** Modify `scripts/fetch-run.sh:202-206`; Test `scripts/test_fetch_run.py` (fake curl at `:45-60`, new test beside `:339`)
**Interfaces:** local helper `cq(s) -> str` returns `'"' + s with \ , ", \n, \r, \t escaped as \\ , \", \n, \r, \t + '"'` (curl config rules); used for `url`, `output` and `header`.
- [ ] Step 1: Write failing test `test_an_outdir_with_quote_and_backslash_is_read_by_curl`: out dir named `we"ird\dir` under the temp root; the fake curl unescapes the `output = "..."` line with the same four escapes and writes there; assert the entry file exists and the run exits 0.
- [ ] Step 2: Run `python3 -m unittest scripts/test_fetch_run.py -k quote_and_backslash`; expect FAIL (fake curl writes to a garbled path).
- [ ] Step 3: Implement `cq` and use it in `config`.
- [ ] Step 4: Run `python3 -m unittest scripts/test_install.py scripts/test_fetch_run.py`; expect PASS.
- [ ] Step 5: Commit `git add scripts/fetch-run.sh scripts/test_fetch_run.py; git commit -m "fetch-run: escape the curl config"`.

### Task G.11: `gate.sh` no longer installs packages   (size S)   PARALLEL-OK
**Files:** Modify `scripts/gate.sh:55-58`
**Interfaces:** the `dnf` branch is deleted; the missing line becomes `missing="$missing python3.11(install it, for example: sudo dnf install python3.11)"`. The pad image already installs python3.11 (`scripts/pad/image:20`), so the build box is unaffected.
- [ ] Step 1: Verify: `grep -c sudo scripts/gate.sh`; expect `1`.
- [ ] Step 2: Edit lines 55-58 as above.
- [ ] Step 3: Run `grep -c sudo scripts/gate.sh` (expect `0` outside the message text: `grep -n "sudo -n" scripts/gate.sh` prints nothing) and `sh -n scripts/gate.sh`.
- [ ] Step 4: Run `PATH=/usr/bin:/bin sh scripts/gate.sh fast 2>&1 | head -3` on a machine with no python3.11 only if one exists; otherwise skip.
- [ ] Step 5: Commit `git add scripts/gate.sh; git commit -m "gate.sh: never sudo; say how to install python3.11"`.

### Task G.12: Staging host out of the source; signed-in home as "~/..."   (size S)   PARALLEL-OK
**Files:** Modify `scripts/pad/pad:28-30` (`SERVERS`), `:336-341` (`load_settings`), `scripts/pad/pad.toml:2`; `scripts/pad/runs/core-claude-code-signin.toml:2,9`, `core-codex-signin.toml:2,8`, `kill-codex-signin.toml:7`; Test `scripts/pad/test_pad_run_case.py` (`SignedInHome`), `scripts/pad/test_pad.py`
**Interfaces:** `staging_url() -> str` reads `PAD_STAGING_URL`, requires `https://<host>` whose first host label contains `staging`, else `Usage("PAD_STAGING_URL must name the staging Worker (https, a host with 'staging' in its first label)")`. `SERVERS["staging"]` is looked up through it. `goalcup_home` is `os.path.expanduser`-ed before the absolute check; run files say `goalcup_home = "~/gc-signin/home"`.
- [ ] Step 1: Write failing tests: `test_goalcup_home_tilde_is_expanded` (`HOME=/u` patched; `load_settings(dict(ROW, goalcup_home="~/gc/home"), PADCFG)["goalcup_home"] == "/u/gc/home"`; `"gc/home"` still raises `Usage`); `StagingUrl.test_unset_refuses`, `test_production_looking_host_refused` (`https://goalcup-ingest.example.workers.dev`), `test_staging_host_accepted` (`https://goalcup-ingest-staging.example.workers.dev`).
- [ ] Step 2: Run `python3 -m unittest discover -s scripts/pad -p 'test_pad*.py' -k "tilde or StagingUrl"`; expect FAIL.
- [ ] Step 3: Implement; edit the three run files and the `pad.toml` comment ("the staging address comes from PAD_STAGING_URL").
- [ ] Step 4: Run `python3 -m unittest discover -s scripts/pad -p 'test_*.py'` and `grep -rn "/Users/\|b-9a5" scripts/pad --include=* -I | grep -v pycache`; expect PASS and no hits.
- [ ] Step 5: Commit `git add scripts/pad; git commit -m "pad: staging URL from PAD_STAGING_URL, signed-in home as ~/..."`.

### Task G.13: OpenCode pad file follows the requested version   (size S)   PARALLEL-OK
**Files:** Modify `scripts/pad/pad-run:314`, `scripts/pad/harnesses/opencode-cli.toml:13`, `opencode2-cli.toml` (same line); Test `scripts/pad/test_pad_run_case.py`
**Interfaces:** `Run.setup` runs `version_cmd` with env `PAD_VERSION=<settings version>`. Both OpenCode `version_cmd` use `v=${PAD_VERSION:-<pin>}` for the link target and fall back to `opencode --version` when `$HOME/.opencode/$v` is absent.
- [ ] Step 1: Write failing test `VersionCmdEnv.test_version_cmd_sees_pad_version`: fake `padrun.sh` records `kw["env"]`; settings `version="1.18.0"`; assert `env["PAD_VERSION"] == "1.18.0"`.
- [ ] Step 2: Run `python3 -m unittest discover -s scripts/pad -p 'test_pad_run_case.py' -k VersionCmdEnv`; expect FAIL (`KeyError`).
- [ ] Step 3: Pass `env=dict(os.environ, PAD_VERSION=s["version"])` at `pad-run:314`; edit the two harness lines.
- [ ] Step 4: Run `python3 -m unittest discover -s scripts/pad -p 'test_*.py'`; and `PAD_VERSION=9.9.9 bash -c "$(sed -n 's/^version_cmd = .\(.*\).$/\1/p' scripts/pad/harnesses/opencode-cli.toml)"` on a machine without OpenCode prints a "not found" line, not a link error.
- [ ] Step 5: Commit `git add scripts/pad; git commit -m "pad: OpenCode version_cmd follows PAD_VERSION"`.

### Task G.14: Rows for the release freeze   (size L, PAD)
**Files:** Modify `scripts/pad/rows/pad-rows.jsonl`, `scripts/pad/rows/PAD-RESULTS.md`, `connectors/tiers.toml` (all generated by `scripts/pad/table`)
**Interfaces:** none. Runs once, at the code freeze, on the final `v1.1/integration` build (owner: one release-row run). Queue after every other PAD task.
- [ ] Step 1: Build the freeze commit with `sh scripts/gate.sh full`; expect `gate: pass`.
- [ ] Step 2: Run the core rows: `scripts/pad/pad scripts/pad/runs/core-claude-code-signin.toml`, `core-codex-signin.toml`, `core-hermes.toml` (Hermes at the pin 0.22.5, no `pad_only`), `kill-codex-signin.toml`, plus the Claude and Codex gateway rows; expect no `setup FAIL` in any dodger row (the G.6 check in a real home).
- [ ] Step 3: Run Hermes 0.22.6 direct, gateway, dodger (rows carry `pad_only`; they show as evidence of the failure, not as a tier).
- [ ] Step 4: Run `python3 scripts/pad/table`; expect `tiers.toml` shows Claude and Codex `verified` with `os = "darwin-arm64"`, Hermes `verified` at 0.22.5 if its rows pass, else `not verified` with the reason.
- [ ] Step 5: Commit `git add scripts/pad/rows connectors/tiers.toml; git commit -m "pad: release rows and tiers on the freeze commit"`, then `python3 scripts/pad/table --release-check HEAD`; expect `release check: pass at <sha>`. If Hermes 0.22.5 is not green, ship it "not verified" (tiers say so; the check still passes because the check lists only what the harness file's `release_versions` names, see G.22).

## Wave 2: the pad version matrix (MULTI-VERSION-PLAN task 16)
Shape: `versions = ["0.22.5", "0.22.6"]` in a pad settings file; one run per (case, version); `{version}` templated into the harness file's `install` and `version_cmd`; one row per (harness, version, os); `table` turns rows into a list of verified (version, os) builds. goalcup never refuses a version; the list is only what the pad proved.

### Task G.20: `versions` setting expands to runs   (size M)
**Files:** Modify `scripts/pad/pad:48-52` (`KEYS`), `:320-375` (`load_settings`), `:1063-1074` (`case_list`, `run_all`); Test `scripts/pad/test_pad.py`
**Interfaces:** `load_settings`: `versions` is a non-empty list of strings matching `[0-9][0-9A-Za-z.+-]*`, not with `version`; stored as `s["versions"]`. `case_list(s, harness) -> (list[tuple[str, str]], bool)` returns `(case, version)` pairs, version `harness["version"]` when no list. `Run.__init__` gets `version`; its settings copy has `version=<v>` and `version_required=True` when a list was given; `os` and entry names stay unique (the run folder already holds a time stamp and id).
- [ ] Step 1: Write failing tests `Matrix.test_pairs` (`versions=["0.22.5","0.22.6"]`, `case="all"` → 6 pairs, order case-major), `test_version_and_versions_conflict` (`Usage`), `test_bad_version_string` (`"1; rm"` → `Usage`), `test_no_list_gives_the_pin`.
- [ ] Step 2: Run `python3 -m unittest discover -s scripts/pad -p 'test_pad.py' -k Matrix`; expect FAIL.
- [ ] Step 3: Implement; `run_all` takes pairs (the sequential rule of G.6 still applies).
- [ ] Step 4: Run `python3 -m unittest discover -s scripts/pad -p 'test_*.py'`; expect PASS.
- [ ] Step 5: Commit `git add scripts/pad/pad scripts/pad/test_pad.py; git commit -m "pad: versions list expands to one run per case and version"`.

### Task G.21: `{version}` templates and the install step   (size M, PAD)
**Files:** Modify `scripts/pad/pad:~565-640` (step list), `scripts/pad/pad-run` (new `step_install`, `setup` required-version check), `scripts/pad/image:36-44`, every `scripts/pad/harnesses/*.toml` `install` and `version_cmd` that names its pin; Test `scripts/pad/test_pad_run_case.py`, `scripts/pad/test_pad.py`
**Interfaces:** `render(cmd, version) -> str` replaces `{version}` (version already matched the G.20 pattern, so no shell metacharacter). `step_list` inserts `install` before `setup` when `where == "sandbox"` and `version != harness["version"]`; `LIMITS["install"] = 10`. `image` renders every `install` and `version_cmd` with the pin. With `version_required` and `where == "here"`, a setup that reads another version FAILs with `requested 0.22.6, found 0.22.5; install it, for example: <rendered install>` (the pad never installs on the owner's Mac). `PAD_VERSION` is also in the `install` environment.
- [ ] Step 1: Write failing tests: `Install.test_step_list_adds_install_for_other_versions` (sandbox, version differs → `["install", "setup", ...]`; same version → no install step); `test_render` (`render("x@{version}", "1.2.3") == "x@1.2.3"`); in `test_pad_run_case.py` `test_required_version_mismatch_fails_on_mac` (found `0.22.5`, requested `0.22.6` → `Step("FAIL", ...)` containing `requested 0.22.6, found 0.22.5`).
- [ ] Step 2: Run the same classes with `-k Install`; expect FAIL.
- [ ] Step 3: Implement; convert each harness file's pin in `install`/`version_cmd` to `{version}` (the `version` key stays the pinned default).
- [ ] Step 4: Run `python3 -m unittest discover -s scripts/pad -p 'test_*.py'`; then one PAD run: a settings file with `versions = ["0.22.6"]`, Hermes, direct, micro, in a box; expect an `install` step `ok` and `harness_version` `0.22.6` in the row.
- [ ] Step 5: Commit `git add scripts/pad; git commit -m "pad: {version} templates, install step for other versions"`.

### Task G.22: Tiers become a list of verified (version, os) builds   (size M)
**Files:** Modify `scripts/pad/table:29-31,60-100` (`render_tiers`, `tier_for`), `release_check`; Test `scripts/pad/test_table_release.py`
**Interfaces:** `builds(cid, rows, lines) -> list[tuple[str, str]]`: for each `(harness_version, os)` pair among that connector's counting rows, the pair is verified when every required case (the G.2 rule: `CASES` less `NO_TIER_ROW`, less `gateway` without a gateway route) has a counting `full` fault-free row for exactly that pair; sorted by numeric version descending, last 5 per os. `tier_for` keeps its 4-tuple (newest verified build, or the not-verified reason), so the old reader still works. `render_tiers` appends `[[<id>.builds]]` tables (`version`, `os`) after each connector. Harness file key `release_versions = ["0.22.5"]` (default: the pin); `release_check` requires, per core harness, a verified build for every listed version on `darwin-arm64` instead of the single latest row set.
- [ ] Step 1: Write failing tests `Builds.test_two_versions_two_systems` (hermes rows for 0.22.5 and 0.22.6 on darwin, 0.22.5 on linux → three builds, newest first within an os), `test_incomplete_build_is_not_listed` (0.22.6 lacks `dodger` → absent), `test_last_five_per_os`, `test_render_has_builds_tables` (substring `[[hermes-cli.builds]]\nversion = "0.22.6"`), `test_release_versions_listed_in_harness_file_are_required` (harness file `release_versions = ["0.22.5","0.22.6"]`, only 0.22.5 rows → output contains `hermes-cli 0.22.6: no verified build`).
- [ ] Step 2: Run `python3 -m unittest discover -s scripts/pad -p 'test_table_release.py' -k Builds`; expect FAIL.
- [ ] Step 3: Implement; keep `tier_for`'s tuple, add `builds`.
- [ ] Step 4: Run `python3 -m unittest discover -s scripts/pad -p 'test_*.py'` and `cargo nextest run -p goalcup tiers_file_parses`; expect PASS.
- [ ] Step 5: Commit `git add scripts/pad/table scripts/pad/test_table_release.py; git commit -m "table: tiers list verified (version, os) builds"`.

### Task G.23: Matrix rows for Hermes and the core harnesses   (size L, PAD)
**Files:** Create `scripts/pad/runs/matrix-hermes.toml` (`versions = ["0.22.5","0.22.6"]`, `case = "all"`, `mode = "full"`); Modify `scripts/pad/rows/*`, `connectors/tiers.toml` (generated)
**Interfaces:** none. Runs once, with G.14, after the freeze.
- [ ] Step 1: Write failing test `test_matrix_hermes_loads` in `test_pad_acceptance_runs.py`: `pad.load_settings(read_toml(runs/matrix-hermes.toml), PADCFG)` gives `versions == ["0.22.5", "0.22.6"]` and `case_list` returns 6 pairs. Run `python3 -m unittest discover -s scripts/pad -p 'test_pad_acceptance_runs.py' -k matrix_hermes`; expect FAIL (no file). Then create the file by copying `core-hermes.toml` and adding the `versions` line.
- [ ] Step 2: Run `scripts/pad/pad scripts/pad/runs/matrix-hermes.toml` (6 runs: 3 cases by 2 versions, sandbox, linux-x86_64 rows) and the same file with `where = "here"` on the Mac for the versions installed there; expect rows with `harness_version` 0.22.5 and 0.22.6.
- [ ] Step 3: Run `python3 scripts/pad/table`; expect Hermes `[[hermes-cli.builds]]` for each (version, os) whose three cases pass.
- [ ] Step 4: Run `python3 scripts/pad/table --release-check HEAD` after the commit; expect `release check: pass`.
- [ ] Step 5: Commit `git add scripts/pad/runs/matrix-hermes.toml scripts/pad/rows connectors/tiers.toml; git commit -m "pad: Hermes version matrix rows"`.

### Task G.24: DROPPED (index ruling 1: Hermes uses H.3 `ca_settings`; H.1 deletes the block)
**Files:** Modify `scripts/pad/harnesses/hermes-cli.toml` (delete both `pad_only` entries), delete `scripts/pad/harnesses/direct/hermes-ca.yaml` and `gateway/hermes-ca.yaml`
**Interfaces:** Consumes lane I's `[[versions]] match = "0.22.6"` overlay with `ca_env` in `connectors/hermes-cli.toml`, so goalcup itself gives Hermes 0.22.6 the CA bundle.
- [ ] Step 1: Write failing check `HermesNoPadOnly.test_no_pad_only_left` in `test_pad_run_case.py`: `read_toml("harnesses/hermes-cli.toml")` has no `pad_only` key in `direct` or `gateway`.
- [ ] Step 2: Run `python3 -m unittest discover -s scripts/pad -p 'test_pad_run_case.py' -k HermesNoPadOnly`; expect FAIL.
- [ ] Step 3: Remove the entries and files.
- [ ] Step 4: Run the test, then one PAD run `versions = ["0.22.6"]` direct full; expect a row with `pad_only: []` that reaches the model (C1 PASS). If it does not, the overlay is wrong: stop and report to lane I.
- [ ] Step 5: Commit `git add scripts/pad; git commit -m "pad: Hermes 0.22.6 runs without pad-only config"`.

## Notes for the index
- PAD tasks: G.14, G.21 (one box run), G.23, G.24. G.5's Hermes rerun happens inside G.14.
- Merge order inside the lane: G.1 first (path), then G.2, G.3, G.4, G.5; G.6 to G.13 any time.
- Owner questions: (1) `PAD_STAGING_URL` env var, no default in the repo: OK? (2) May `test_release_sh.py` sit in `scripts/pad/` or should `gate.sh:85`'s named list get it? (3) Should the release require Hermes 0.22.6 verified, or only 0.22.5 (G.22 `release_versions`)? (4) Claude's gateway exemption: should `tier_note` say so in the tier text? (5) Pad never installs a harness version on the owner's Mac (G.21): agreed?
- Cross-lane risks: lane docs moving `docs/plans` (G.1 must land first); lane F's `harness_version.py` name and `matches` signature (G.5, wave 2); the Rust owner of `tier_line` reading `os` and `builds`; lane I's Hermes overlay (G.24).

## Coordinator addition (index ruling 2)
G.20 also adds the pad settings key `here_path`: a list of absolute directories put first on PATH for one `where = "here"` run (validated in `load_settings`: list of absolute paths that exist; error "here_path is a list of absolute directories"), so Mac rows can run a harness build installed under `/Users/team/dev/harness-installs/<id>-<version>/bin`. Test `test_pad.py::HerePath` (accepts a list of existing dirs; refuses a relative path; pad-run puts the dirs before PAD_BIN on PATH). Lane H.7 consumes it. Also skip `rec/goalcup` (lane C.2 pinned exe) when the pad copies a run folder.
