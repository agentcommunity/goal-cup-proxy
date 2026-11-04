# Brief for lane plan drafters (read all of it)

You write ONE lane of a parallel implementation plan for goalcup (Goal Cup recorder). You write a plan, not code.

## Where things are
- Repo checkout (read only for you): /Users/team/dev/worktrees/goal-cup-tracer/mac-test-v1.0 . Code equals origin/main 70eecdfb except scripts/pad and docs/plans. Read AGENTS.md and VISION.md first.
- Review report (the findings, with file:line): docs/superpowers/plans/2026-11-04-v1-fixes/review/REVIEW-REPORT.md
- Lane notes with more detail: same folder, <lane>.md (cr-proxy, cr-watchdog, cr-upload-server, cr-format-verify, cr-scripts, arch, usability, oss, docs, versions).
- Multi-version plan (merged, scored): same folder, MULTI-VERSION-PLAN.md. NOTE: its dates are stale. The event moved to end of November / early December 2026. Nothing has to ship by 6 Nov; the "1.1" tasks are now in scope before the event.
- Plan folder (write your file here): docs/superpowers/plans/2026-11-04-v1-fixes/

## Hard rules
- Write only your own lane file in the plan folder. Do not edit any other file. Do not run cargo, tests, builds, git checkout/stash/commit in that checkout (a live test run uses it). Do not touch tmux or pad processes. Reading files and git log/show/grep is fine.
- Never open .env, .env.local or ~/.pad-keys, never print a key.
- Do not spawn subagents.
- Verify every file:line you cite by reading the code; the review can be wrong. If a finding is wrong or already fixed, say so in one line under "Dropped" instead of planning work for it.

## Owner decisions (fixed)
- Event: end of November / early December 2026. No 1.0.0 tag before all lanes land; one code freeze, one release-row run at the end.
- Multi-version: goalcup never refuses a harness because of its version. It records the version it reads and tells the participant plainly. "Verified" is a list of (version, os) pairs computed from pad rows.
- Hermes: support several versions (at least 0.22.5 and 0.22.6) for participants, without pad-only config.
- Videos: two videos are required, one before the run (setup/plan) and one after (demo).
- CI stays lean: do not add new jobs/suites to .github/workflows/ci.yml. macOS coverage runs through scripts/gate.sh on the owner's Mac; a macOS CI job is an owner question, not a task.
- Less code is better. Cut before you add. Seven points in CLAUDE.md/VISION.md are the yardstick.

## How lanes run (parallel)
- Each lane runs in its own worktree (Worktrunk: /Users/team/dev/tools/worktrunk/bin/wt), branch `fix/<lane-id>`, cut from `v1.1/integration` (created from origin/main). Each lane merges back by PR into v1.1/integration. The full gate (scripts/gate.sh full, on the owner's Mac, PATH with ~/.cargo/bin first) runs once per merge.
- Lanes own files. Only edit files your lane owns (list in your lane brief). Where a file is SHARED, edit only the functions/line ranges named for your lane, so merges are clean.
- Pad rows (scripts/pad/pad runs of real harnesses) are a serial resource on one Mac (shared signed-in home, key budgets). Mark any task that needs pad rows with "PAD" so the index can queue them.

## Cross-lane contracts (use these exact names)
- Version parser (lane F): `crates/goalcup/src/connector/version.rs`
  - `pub struct HarnessVersion { pub text: String, pub nums: Vec<u64> }` (text like "0.22.6", no "v", no trailing dot)
  - `pub fn parse(line: &str) -> Option<HarnessVersion>` (first version-looking token in a `--version` line)
  - `pub fn matches(v: &HarnessVersion, spec: &str) -> bool` where spec is "0.22.6" (exact), "0.22" (prefix by numeric parts), or comma-joined bounds ">=0.22.5,<0.23".
  - Shared fixture `crates/goalcup/tests/fixtures/version-lines.tsv` (columns: connector_id, raw line, expected text or "-").
  - Python twin `scripts/pad/harness_version.py`: `parse(line) -> str | None`, `matches(text, spec) -> bool`, tested on the same TSV.
- Connector loading (lane E): `crates/goalcup/build.rs` globs `connectors/*.toml` and replaces the hand list in `connector/builtin.rs`; keep the existing public API of builtin.rs unchanged so callers do not move. `[usage]` connector table replaces connector-id literals in goalcup-verify/src/usage/hints.rs and usage/native.rs.
- Version overlays (lane I, wave 2): `[[versions]]` array in connector TOML, each with `match = "<spec>"` (version.rs syntax) plus override keys; first match wins; keys `home`, `ca_env`, `mask`, `refuse_if_set` are not allowed in a row. Replaces `version`, `version_notes`, VersionMismatch.
- Net/egress (lane B): `crates/goalcup/src/net.rs` with `pub struct Egress { .. }`, `pub fn load() -> Egress` (participant's saved upstream proxy + OS roots incl. /Library/Keychains/System.keychain on macOS + participant CA files), used by proxy upstream and upload::client::ServerClient.
- Tiers (lane G): `scripts/pad/table --release-check` re-renders tiers.toml from committed rows and fails on diff; rows marked `pad_only` never count as verified.

## Plan file format (writing-plans skill, follow exactly)
Start with:
```
# Lane <ID>: <name>
**Wave:** 1 or 2   **Depends on:** <lanes or none>   **Branch:** fix/<lane-id>
**Covers findings:** <every finding id you plan or drop>
**Owns files:** <list>   **Shared files (only these parts):** <list>
**Produces (for other lanes):** <exact names/signatures, or none>
**Consumes:** <exact names from other lanes, or none>
## Dropped
<finding id: one-line reason>   (or "none")
## Review focus
<up to 5 inputs/conditions no test below covers yet that would hurt a user; each gets a test added to its task>
```
Then tasks:
```
### Task <ID>.<n>: <name>   (size S/M/L, PAD if it needs pad rows, PARALLEL-OK if independent of the other tasks in this lane)
**Files:** Create/Modify (with line ranges)/Test
**Interfaces:** Consumes / Produces (exact signatures)
- [ ] Step 1: Write the failing test — test name + assertions as code with exact values
- [ ] Step 2: Run it, expect FAIL — exact command + expected failure
- [ ] Step 3: Implement — exact signature, file, pinned values; a body only for an algorithm the test does not determine
- [ ] Step 4: Run, expect PASS — exact command
- [ ] Step 5: Commit — git add <files>; git commit -m "<msg>"
```
Rules: one action per step; no "TBD", "handle edge cases", "add validation" lines; no full function bodies the signature and test already determine; doc-only tasks have a verification step (grep or link check) instead of a unit test. Test commands: Rust `cargo nextest run -p <crate> --test <file> <name>` (or `cargo test -p <crate> <name>` for unit tests in src), Python `python3 -m unittest scripts/pad/test_x.py -k <name>` style (check how the repo runs them in scripts/gate.sh), Worker tests as gate.sh runs them. Mark Linux-only behaviour.
Keep the file tight: a plan longer than the code it describes is a transcript. Order tasks so independent ones are marked PARALLEL-OK.

Return: a 5-line summary (task count, sizes, PAD tasks, cross-lane risks, open owner questions).
