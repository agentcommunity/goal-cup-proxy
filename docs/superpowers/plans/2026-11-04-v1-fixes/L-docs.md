# Lane L: Rules and participant docs
**Wave:** 1 (L.1-L.7, owner truths) and 3 (L.8, final sync)   **Depends on:** none for wave 1; A, B, C, E, F, G, I, J merged for L.8   **Branch:** fix/docs
**Covers findings:** docs-1, docs-2, docs-4, docs-8, docs-9, docs-10, PARTICIPANT-GUIDE:324 (proxy), event date, docs-7 (doc half, with E), docs-x1; drops docs-3, docs-5, docs-6, docs-x2
**Owns files:** VISION.md, CLAUDE.md, TODO.md, README.md (not Quick start 1-6, lane A), docs/RULES.md, docs/PARTICIPANT-GUIDE.md (not line 464, lane F), docs/CONNECTOR-SCHEMA.md (not the `[usage]` section, lane E), docs/HARNESS-SETUP.md, docs/LOCAL-RESEARCH-HANDOFF.md, docs/RESEARCH-TARGET.md, skills/goal-cup/SKILL.md, skills/adding-a-capture-harness/SKILL.md and templates/harness-facts.md
**Shared files (only these parts):** AGENTS.md line 22 (date) and lines 12-22 (seven points) ; docs/ARCHITECTURE.md lines 10-18 (seven points) ; docs/harnesses/README.md line 77 ; docs/research/coverage-matrix.md lines 6-7 (links after the move). `skills/adding-a-capture-harness/templates/pad-harness.toml` belongs to lane G.
**Produces (for other lanes):** one doc home per fact (table in L.6); the "Harness versions" section in docs/CONNECTOR-SCHEMA.md that lanes F and I link to; owner-question markers `OWNER-Q` (grep-able, all gone before the tag)
**Consumes:** lane B behaviour for the proxy line (L.5); lane F `matches()` spec syntax and lane I `[[versions]]` key list (L.8 only)

## Dropped
- docs-3: broken anchors and the unrun link checker are lane G/arch-9 (delete or wire `check-links.py`); the TODO.md:51 line is replaced inside L.1 if still present.
- docs-5: DECISIONS pruning is already tracked at TODO.md:85; the D74 citation is right (review report says so).
- docs-6: the `dev` field and `disk_full` are Worker and UPLOAD-API.md changes; lane J.
- docs-x2: repo-name half refuted (oss-1). The README install URL stays `goal-cup-proxy`; lane for oss-1 owns any rename.
- docs-7 code half: `build.rs` is lane E. Only the contributor text is here (L.7).

## Review focus
1. A doc says "never refuses" while the code still refuses OpenCode on a version prefix: wave-1 text merges into `v1.1/integration` before lane F, so L.8 greps that the refusal strings are gone from both code and docs.
2. A participant on a company network reads "Nothing to fix" (guide 324) while the harness cannot reach its model at all: L.5 separates "traffic passed unread" from "harness blocked".
3. A date left as the old 7 November in one of the six files that carry it: L.1 test greps the whole tree.
4. Moving two history docs breaks links from docs/research/coverage-matrix.md: L.4 greps for dead targets.
5. Two videos named in four places with different due times: L.2 greps every mention for the same wording.

### Task L.1: Event date and "ready by" line   (size S, PARALLEL-OK)
**Files:** Modify VISION.md:16,43 ; CLAUDE.md:16 ; AGENTS.md:22 ; docs/RULES.md:38,40 ; TODO.md "Release 1.0" section (lines 56-71) ; Test: grep
**Interfaces:** Produces `OWNER-Q date`: the exact start window is not in any file I may read. Never invent it.
- [ ] Step 1: Write the check (shell, saved in the task notes): `grep -rnE "6 November|7 November|8 November|Friday 6" VISION.md CLAUDE.md AGENTS.md README.md docs/RULES.md docs/PARTICIPANT-GUIDE.md TODO.md` must print nothing.
- [ ] Step 2: Run it, expect FAIL with six hits (VISION.md:16,43; CLAUDE.md:16; AGENTS.md:22; docs/RULES.md:38,40).
- [ ] Step 3: Implement. VISION.md:16 becomes "**Start window:** a 24-hour window at the end of November or the start of December 2026 (OWNER-Q date: exact day). Every entry gets its own 168 hours from its start." VISION.md:43 becomes "**Ready one week before the start window opens.**" CLAUDE.md:16 and AGENTS.md:22 end "Ready one week before the start window opens." RULES.md:38 drops "planned for 6 November, 12:00" and says "The release is published before the start window; it is not a cutoff, and releases may follow (D70)." RULES.md:40 says "The 24-hour start window opens on the date on the event site (OWNER-Q date) at 00:00 and closes 24 hours later." TODO.md "Release 1.0": retitle "Release 1.0 (one freeze, one release-row run, before the event)" and drop any 6 November wording.
- [ ] Step 4: Run the Step 1 command, expect no output; `grep -rn "OWNER-Q date" VISION.md docs/RULES.md` shows the two placeholders to resolve.
- [ ] Step 5: Commit `git add VISION.md CLAUDE.md AGENTS.md docs/RULES.md TODO.md; git commit -m "docs: event date moved to end of November"`

### Task L.2: Two required videos   (size S, PARALLEL-OK)
**Files:** Modify VISION.md:20 ; docs/RULES.md:407-410 ; README.md:10,68 ; docs/PARTICIPANT-GUIDE.md:27,406
**Interfaces:** none. Nothing in the tree says when the setup video is due or where it goes. Never invent either.
- [ ] Step 1: Write the check: `grep -rnE "demo video" VISION.md README.md docs/RULES.md docs/PARTICIPANT-GUIDE.md | grep -vi "setup video"` must print nothing, and `grep -rc "setup video" VISION.md README.md docs/RULES.md docs/PARTICIPANT-GUIDE.md` must be at least 1 in each.
- [ ] Step 2: Run it, expect FAIL (RULES.md:407, README.md:68, guide 27 and 406 name only the demo video).
- [ ] Step 3: Implement one wording, one home. RULES.md:407 (the home) reads: "**Two videos and a questionnaire.** The setup video (before the run, the goal, the harness, the model and the plan; due by OWNER-Q setup-due) and the demo video (after the run, showing the product; due 24 hours after your own 168 hours end) are both required. You hand in both, and the questionnaire, on the event site (OWNER-Q setup-upload: where the setup video goes), not through Goal Cup Proxy. Nothing is added to your repository for them." VISION.md:20 becomes "**Two short videos:** a setup video before the run and a demo after it (RULES)." README.md:68 and guide 27, 406 say "your two videos and the questionnaire" and link to RULES. README.md:10 says "the videos".
- [ ] Step 4: Run the Step 1 command, expect no output and the counts all at least 1; `grep -rn "OWNER-Q setup" docs/RULES.md` lists the two open questions.
- [ ] Step 5: Commit `git add VISION.md README.md docs/RULES.md docs/PARTICIPANT-GUIDE.md; git commit -m "docs: two required videos, setup before and demo after"`

### Task L.3: Version truth in RULES and the connector contract   (size M, PARALLEL-OK)
**Files:** Modify docs/RULES.md:207-209,345 ; docs/CONNECTOR-SCHEMA.md:6-8 (intro) and a new section "Harness versions" after the contract table ; docs/harnesses/README.md:77 ; skills/adding-a-capture-harness/SKILL.md:30-31 (wording only)
**Interfaces:** Produces the policy lanes F and I link to: goalcup reads the version at start, continue and seal and records it; it never refuses a harness for its version; a version with a known problem gets a note said at start; "Verified" is a list of (version, os) pairs computed from pad rows (lane G), not a connector field.
- [ ] Step 1: Write the check: `grep -nE "samples the version|Nothing is pinned|not pinned|architecture, CPU count, memory" docs/RULES.md docs/CONNECTOR-SCHEMA.md docs/harnesses/README.md` must print nothing, and `grep -c "Harness versions" docs/CONNECTOR-SCHEMA.md` must be 1.
- [ ] Step 2: Run it, expect FAIL with four hits (RULES.md:208, CONNECTOR-SCHEMA.md:7, harnesses/README.md:77, RULES.md:345) and count 0.
- [ ] Step 3: Implement. RULES.md:207-209: "Do not update the harness during the run. Goal Cup Proxy reads the harness version at start, at continue and at seal, records each, and refuses no version. If the version changed during the run, the record shows it and an organizer looks at it. That is a flag, not a disqualification." RULES.md:345 (machine facts, per `cli/start.rs:271-272`): "the harness build and two non-identifying facts about the machine (operating system and CPU count): no host name, user name or serial number." CONNECTOR-SCHEMA.md intro: delete "Nothing is pinned."; add section "Harness versions" with five bullets: where the version is read (first version-looking token of `program --version`, lane F parser); where it is recorded (`start`, `continue`, `seal` events, RECORD-FORMAT); that no version refuses a start; that a known problem is a `[[versions]]` note (lane I, syntax in L.8); that "verified" lives in `connectors/tiers.toml` as (version, os) pairs written from pad rows. harnesses/README.md:77: "builds are recorded, never refused."
- [ ] Step 4: Run the Step 1 command, expect no output and count 1.
- [ ] Step 5: Commit `git add docs/RULES.md docs/CONNECTOR-SCHEMA.md docs/harnesses/README.md skills/adding-a-capture-harness/SKILL.md; git commit -m "docs: multi-version truth, machine facts are os and cpus"`

### Task L.4: History docs out of docs/   (size S, PARALLEL-OK)
**Files:** Move docs/LOCAL-RESEARCH-HANDOFF.md and docs/RESEARCH-TARGET.md to docs/archive/ ; Modify docs/research/coverage-matrix.md:6-7 ; TODO.md:82-83 ; docs/archive/README line if one exists
**Interfaces:** none. `git mv` only; do not edit the moved text.
- [ ] Step 1: Write the check: `test ! -e docs/RESEARCH-TARGET.md -a ! -e docs/LOCAL-RESEARCH-HANDOFF.md -a -e docs/archive/RESEARCH-TARGET.md` and `grep -rn "\.\./RESEARCH-TARGET\|\.\./LOCAL-RESEARCH-HANDOFF" docs/research` prints nothing.
- [ ] Step 2: Run it, expect FAIL (both files at top level; coverage-matrix.md:6-7 links point there).
- [ ] Step 3: Implement: `git mv docs/RESEARCH-TARGET.md docs/LOCAL-RESEARCH-HANDOFF.md docs/archive/`; coverage-matrix.md:6-7 links become `../archive/...`; TODO.md:82-83 drops the two names from the purge list since they are already in archive.
- [ ] Step 4: Run the Step 1 command, expect success. Then `python3 scripts/check-links.py` if lane G keeps it, else `grep -rn "RESEARCH-TARGET\|LOCAL-RESEARCH-HANDOFF" --include=*.md . | grep -v "docs/archive\|docs/plans"` shows only the moved links.
- [ ] Step 5: Commit `git add -A docs TODO.md; git commit -m "docs: move two history docs to archive"`

### Task L.5: Company proxy line in the guide   (size S, PARALLEL-OK; text depends on lane B)
**Files:** Modify docs/PARTICIPANT-GUIDE.md:324 (table row) and :435 (limits bullet "a network that re-signs TLS is not supported")
**Interfaces:** Consumes lane B `net::load() -> Egress`: goalcup sends its own upstream traffic through the participant's saved upstream proxy and trusts OS roots plus participant CA files. Lane B's final wording is checked in L.8.
- [ ] Step 1: Write the check: `grep -n "company proxy" docs/PARTICIPANT-GUIDE.md` must show a row that has two outcomes, and `grep -c "proxy-only" docs/PARTICIPANT-GUIDE.md` must be at least 1.
- [ ] Step 2: Run it, expect FAIL (row 324 says only "Nothing to fix").
- [ ] Step 3: Implement before B lands: split row 324. "refused the recorder (it checks its own certificates)": "Nothing to fix; the traffic passed unread and the end screen names it." "its certificate is not one goalcup trusts, a company proxy": "The agent may not reach its model at all. Networks that only allow traffic through a company proxy are not supported yet; use another network. (OWNER-Q proxy: confirm the cut-off.)" Line 435 keeps "a network that re-signs TLS is not supported" until L.8 rewrites both rows to what lane B ships (upstream proxy variable honoured, company root CA read from the system).
- [ ] Step 4: Run the Step 1 command, expect two matching lines and count at least 1.
- [ ] Step 5: Commit `git add docs/PARTICIPANT-GUIDE.md; git commit -m "docs(guide): company proxy rows say what happens"`

### Task L.6: One home per fact, one platform table   (size M, PARALLEL-OK)
**Files:** Modify README.md:138-145 (the platform table, kept as home) and README.md:92 (check flags) ; docs/RULES.md:259 ; docs/PARTICIPANT-GUIDE.md (platform mentions in section 1) ; AGENTS.md:12-22 ; docs/ARCHITECTURE.md:10-18
**Interfaces:** Home table (every fact has exactly one doc that states it; the others link):

| Fact | Home | Others |
|---|---|---|
| Seven points | VISION.md | CLAUDE.md keeps the list because the harness loads it; AGENTS.md and ARCHITECTURE.md link |
| Platforms | README.md "Platforms" | RULES.md:259 and guide link |
| Video, questionnaire, due times | RULES.md | README, guide, VISION link |
| Harness version policy | CONNECTOR-SCHEMA.md "Harness versions" | RULES, guide, harnesses/README link |
| Machine facts | RULES.md:345 | guide section 9 links |

- [ ] Step 1: Write the check: `grep -c "Record every model call" VISION.md CLAUDE.md AGENTS.md docs/ARCHITECTURE.md` must give VISION 1, CLAUDE 1, AGENTS 0, ARCHITECTURE 0 ; `grep -c "harness-files" README.md` at least 1 ; `grep -n "Intel" docs/RULES.md README.md` shows hits only in README.md.
- [ ] Step 2: Run it, expect FAIL (AGENTS 1, ARCHITECTURE 1, README 0 for `--harness-files`, RULES has "Intel").
- [ ] Step 3: Implement. AGENTS.md:12-22 and ARCHITECTURE.md:10-18 replace the list with "The seven points are in [VISION.md](VISION.md#the-seven-points)." README.md:92 adds `[--harness-files <JSON>]` to the `check` row (flag text copied from `crates/goalcup-verify/src/legs.rs:518`: expects `[{"path": "<source>:<path>", "size": n}]`) and one sentence: "A plain `check` does not scan for secrets; run `goalcup-verify secrets` for that." README platform table gains rows for macOS Intel and Linux arm64 as the owner states them (OWNER-Q platforms: is Intel supported, tested or untested?); RULES.md:259 becomes "Platforms: see the README table. On Windows, use WSL2 for the event. Native Windows comes later."
- [ ] Step 4: Run the Step 1 command, expect the target counts and Intel only in README.md.
- [ ] Step 5: Commit `git add README.md docs/RULES.md docs/PARTICIPANT-GUIDE.md AGENTS.md docs/ARCHITECTURE.md; git commit -m "docs: one home per fact"`

### Task L.7: Contributor path text   (size S, PARALLEL-OK; final form after lane E)
**Files:** Modify skills/adding-a-capture-harness/SKILL.md:13,29-30 ; CONTRIBUTING.md:64-65,99 (lane that owns CONTRIBUTING makes the same edit; this task only edits the skill) ; docs/CONNECTOR-SCHEMA.md:5-7
**Interfaces:** Consumes lane E: `build.rs` globs `connectors/*.toml`; no `.rs` file is edited by a harness PR.
- [ ] Step 1: Write the check: `grep -rn "entry!" docs skills CONTRIBUTING.md` must print nothing.
- [ ] Step 2: Run it, expect FAIL (CONNECTOR-SCHEMA.md:5 names `entry!`; the skill names `builtin.rs`).
- [ ] Step 3: Implement. Until lane E merges, the text says what is true today (one `entry!` line, the only allowed `.rs` change). Lane E's merge flips it in L.8.
- [ ] Step 4: Run the Step 1 command after lane E merges (done in L.8); before that only check that the text names the `entry!` line in both skill and schema.
- [ ] Step 5: Commit `git add skills docs/CONNECTOR-SCHEMA.md; git commit -m "docs: contributor path states the one allowed .rs line"`

### Task L.8: Final docs sync after all code lanes   (size M, wave 3; PAD only for the version table, read from committed rows)
**Files:** Modify any file under Owns; read lane files A-cli-ux, B-net, C-lifecycle, E-connector-data, F-version-core, G-pad-release, I, J-server in this folder (not all existed when this was written; where one is missing, work from the finding id)
**Interfaces:** Consumes everything above.
- [ ] Step 1: Write the checklist as a shell script in the task notes (not committed). Each line is `claim | grep that must pass`:
  - docs-1, docs-2 (F, I): `! grep -rnE "refuses.*version|VersionMismatch|does not show <version>" docs README.md skills crates/goalcup/src` ; guide Appendix A has no version row ; CONNECTOR-SCHEMA key table lists `[[versions]]` and no `version_notes`; guide:464 is lane F's, check it reads without "install that version".
  - docs-4 (J, if the server collects videos): `grep -c "setup video" RULES.md README.md PARTICIPANT-GUIDE.md VISION.md` all at least 1 ; `OWNER-Q setup` answered.
  - docs-7 (E): `! grep -rn "entry!\|builtin.rs" docs skills CONTRIBUTING.md` ; skill 13 and 29-30 say a harness PR is a TOML file plus a facts page.
  - docs-9 (A): README Quick start commands equal `goalcup --help` and `goalcup-verify --help` output: for each flag in README `grep -c` against the help text; `--harness-files` present.
  - Proxy (B): guide rows 324 and 435 match B's shipped behaviour; if B shipped `net.rs` the "proxy-only networks are not supported" sentence is removed and replaced by the upstream-proxy sentence B's lane file gives.
  - C-lifecycle: every message the lane changes (`status`, `continue`, end screen, sleep/restart lines) is copied into the guide's tables 321-327 by `grep -F` of the exact string from `crates/goalcup/src`. The command: for each guide table cell in backticks, `grep -rF "<text>" crates/goalcup/src` must hit.
  - ARCHITECTURE:95 (arch-1, C): respawn and null-mode retry stated apart.
  - G: `connectors/tiers.toml` rows exist and guide section 4 (line 93) says what "Verified" means as (version, os) pairs; if every row is still "not verified", the guide says so plainly (usability finding, tiers seed).
  - J (UPLOAD-API, `dev`, `disk_full`): out of this lane; check only that README/guide do not name them.
  - Dates and counts: re-derive every figure in the changed docs from the tree (memory: numbers failed twice): `grep -rnE "[0-9]+ (harnesses|connectors|tests)" README.md docs/*.md` and compare with `ls connectors/*.toml | wc -l`.
  - OWNER-Q sweep: `grep -rn "OWNER-Q" . --include=*.md | grep -v docs/plans` must be empty before the tag; each hit is an answered owner question or the release is blocked.
- [ ] Step 2: Run the script on `v1.1/integration` after the last code lane merges; expect FAIL on every claim a lane changed late.
- [ ] Step 3: Edit the docs line by line for each failing claim; no new sections.
- [ ] Step 4: Run the script again, expect all lines pass; run `python3 scripts/check-links.py` if G kept it.
- [ ] Step 5: Commit `git add -A VISION.md CLAUDE.md TODO.md README.md docs skills; git commit -m "docs: final sync after code lanes"`

## Open owner questions
1. Exact start window date (L.1). 2. Setup video due time (L.2). 3. Where the setup video is handed in (L.2). 4. Is macOS Intel supported, tested or untested (L.6)? 5. Is "proxy-only networks not supported" the right cut-off until lane B lands (L.5)? 6. May AGENTS.md and ARCHITECTURE.md lose their copy of the seven points (L.6)?
