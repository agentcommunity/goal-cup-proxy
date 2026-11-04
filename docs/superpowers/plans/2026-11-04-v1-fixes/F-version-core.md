# Lane F: One version parser, honest version lines
**Wave:** 1   **Depends on:** none   **Branch:** fix/version-core
**Covers findings:** versions-1, versions-2, versions-6, versions-x3, usability-11 (= versions-7), cr-watchdog-5, docs-1 (PARTICIPANT-GUIDE.md:464 only)
**Owns files:** crates/goalcup/src/connector/version.rs (new), crates/goalcup/tests/fixtures/version-lines.tsv (new), scripts/pad/harness_version.py (new), scripts/pad/test_harness_version.py (new)
**Shared files (only these parts):** connector/mod.rs (the `pub mod` lines 7-11, `Refusal` 60-68, 233-263, 275-277); connector/launch.rs 146-171 (`harness_version`); cli/start.rs 431-445 (`Row`, `tier_line`) and its `mod tests` 461-469; cli/text.rs 179-182 (VersionMismatch arm) and 1158-1175 (`tier_line`); cli/setup.rs 124-130; crates/goalcup/tests/connector_launch.rs 150-185 (`probe` and its asserts); scripts/pad/pad-run 314-315; scripts/pad/harnesses/copilot-cli.toml:16 (the sed is a parser); docs/PARTICIPANT-GUIDE.md:464
**Produces (for other lanes):**
- `crates/goalcup/src/connector/version.rs`:
  - `#[derive(Debug, Clone, PartialEq, Eq)] pub struct HarnessVersion { pub text: String, pub nums: Vec<u64> }`
  - `pub fn parse(line: &str) -> Option<HarnessVersion>`
  - `pub fn matches(v: &HarnessVersion, spec: &str) -> bool`
  - `pub fn same_program(program: &str) -> Vec<(&'static str, String)>` (built-in ids with that program and a `version` key, with the key)
  - `pub fn untested_say(program: &str, got: &str, want: &str) -> String`, `pub fn unchecked_say(program: &str, want: &str, others: &[(&'static str, String)]) -> String`
- `scripts/pad/harness_version.py`: `parse(line) -> str | None`, `matches(text, spec) -> bool`
- tiers.toml row key `os` (pad `os_id` form, e.g. `"darwin-arm64"`; missing or `""` = unknown), read by `tier_line`. Lane G writes it.
- `Refusal::VersionMismatch { program: String, want: String, got: String, sibling: String }` (lane I deletes it in wave 2).
**Consumes:** none. (Lane E keeps `builtin::CONNECTORS` and `connector::load`; `same_program` uses only those.)

Grammar, pinned for both languages:
- `parse`: per whitespace word, strip leading `(`, strip trailing chars in `.,;:)`, strip one leading `v`; the word must then fully match `\d+(\.\d+)+([-+][0-9A-Za-z.-]+)?` with every number fitting u64. First such word wins. `text` = the matched word (suffix kept), `nums` = the dotted numbers.
- `matches`: spec split on `,`, every part must hold, empty spec or an unparsable part is false. A part is an optional op (`>=`, `<=`, `>`, `<`, `=`) and a value; the value drops a trailing `x` and then trailing `.` (`"1."`, `"1.x"` -> `[1]`). With an op: compare `nums` to the value, shorter side padded with 0. Without: the value's numbers are a prefix of `nums` (so `"0.22.6"` is exact for 3-part builds, `"1."` never holds 10.0.0). Suffix never compared.

## Dropped
- docs-1, CONNECTOR-SCHEMA.md:7 half: not in this lane (docs lane); this lane fixes only the guide line.
- versions-6, the 5 s probe timeout vs Hermes' first-start install: unchanged here; the seal read (MULTI-VERSION-PLAN task 10) covers it.
- `scripts/pad/table`: no version parse to replace (verified: table:82-88 only compares row strings that pad-run wrote); its `os` and tiers render are lane G.
- `continue_run.rs:282-288` raw compare and goalcup-verify `usage/mod.rs:368`: they keep the raw line by design, not a parser.

## Review focus
1. A failed `--version` whose stderr names some other version ("needs Python 3.11.4") must not become the harness version (test in F.5).
2. `"1."` must not hold 10.x or 1x.y (b5a34483 regression), `"0.22.6"` must not hold 0.22.60 (test in F.1).
3. A number too large for u64 or a word like `8b18b55` must not parse (test in F.1).
4. OpenCode 3.x (no connector for it) must run, not be refused (owner: never refuse on version) (test in F.3).
5. Unread version on a version-keyed connector must say so and name the sibling (cr-watchdog-5) (test in F.3).

### Task F.1: version.rs and the shared fixture   (S, PARALLEL-OK)
**Files:** Create crates/goalcup/src/connector/version.rs, crates/goalcup/tests/fixtures/version-lines.tsv; Modify connector/mod.rs:7-11 (`pub mod version;`)
**Interfaces:** Produces `HarnessVersion`, `parse`, `matches`, `same_program`, `untested_say`, `unchecked_say` (above)
- [ ] Step 1: Write the fixture, tab-separated, `#` lines are comments. Rows (sources: goalcup start events in pad-out/*/verify/usage/*.json, docs):
  ```
  hermes-cli	Hermes Agent v0.22.6 (2026.9.24)	0.22.6
  hermes-cli	Hermes Agent v0.19.0 (2026.7.20)	0.19.0
  hermes-cli	no dependency environment is committed for this install; run hermes pm repair	-
  opencode2-cli	opencode v2.2.2	2.2.2
  opencode-cli	1.19.5	1.19.5
  copilot-cli	GitHub Copilot CLI 1.0.105.	1.0.105
  claude-code-cli	2.1.312 (Claude Code)	2.1.312
  codex-cli	codex-cli 0.165.0	0.165.0
  crush-cli	crush version v0.99.0	0.99.0
  openclaw-cli	OpenClaw 2026.10.9 (8b18b55)	2026.10.9
  kilo-cli	7.8.25	7.8.25
  gemini-cli	0.66.0	0.66.0
  goose-cli	1.57.0	1.57.0
  kimi-code-cli	2.4.1	2.4.1
  qwen-cli	0.26.4	0.26.4
  -	Node.js 99999999999999999999.1	-
  -		-
  ```
  Then add one `amp-cli` row from `amp --version | head -1` on the owner's Mac (`/opt/homebrew/bin/amp`), expected the build word with its `-g<hash>` suffix. droid-cli and cline-cli have no recorded line (no pad rows): add a row only if the binary is installed.
- [ ] Step 2: Write the failing tests in `version.rs` `mod tests`:
  - `fixture_lines_parse`: `include_str!("../../tests/fixtures/version-lines.tsv")`, every non-comment row: `parse(raw).map(|v| v.text)` equals expected (`"-"` = None); also assert `parse("Hermes Agent v0.22.6 (2026.9.24)").unwrap().nums == vec![0, 22, 6]`.
  - `specs_match`: with `v = |s| parse(s).unwrap()`: `matches(&v("2.2.2"), "2.")`, `!matches(&v("10.0.0"), "1.")`, `!matches(&v("1.19.5"), "2.")`, `matches(&v("1.19.5"), "1.x")`, `matches(&v("0.22.6"), "0.22.6")`, `!matches(&v("0.22.60"), "0.22.6")`, `matches(&v("0.22.6"), "0.22")`, `matches(&v("0.22.5"), ">=0.22.5,<0.23")`, `!matches(&v("0.23.0"), ">=0.22.5,<0.23")`, `matches(&v("1.0.105"), "=1.0.105")`, `!matches(&v("1.0.0"), "")`, `!matches(&v("1.0.0"), ">=x")`.
  - `opencode_ids_share_a_program`: `same_program("opencode") == vec![("opencode-cli", "1.".into()), ("opencode2-cli", "2.".into())]`; `same_program("claude").is_empty()`.
- [ ] Step 3: Run, expect FAIL: `cargo test -p goalcup --lib connector::version` -> compile error (module empty).
- [ ] Step 4: Implement the five functions in version.rs per the grammar above. `same_program` walks `builtin::CONNECTORS`, `connector::load(id, None)`, keeps rows with `program == program` and `version.is_some()`. The two says are pinned in F.3.
- [ ] Step 5: Run, expect PASS: `cargo test -p goalcup --lib connector::version`
- [ ] Step 6: Commit: `git add crates/goalcup/src/connector/version.rs crates/goalcup/src/connector/mod.rs crates/goalcup/tests/fixtures/version-lines.tsv; git commit -m "feat(connector): one harness version parser and a fixture of real --version lines"`

### Task F.2: Python twin, pad uses it   (S, after F.1 step 1)
**Files:** Create scripts/pad/harness_version.py, scripts/pad/test_harness_version.py; Modify scripts/pad/pad-run:314-315, scripts/pad/harnesses/copilot-cli.toml:16
**Interfaces:** Produces `parse(line) -> str | None`, `matches(text, spec) -> bool` (module uses `from __future__ import annotations`; pad runs on 3.11, Macs give 3.9)
- [ ] Step 1: Write the failing test `test_harness_version.py`: `TestFixture.test_fixture_lines_parse` reads `../../crates/goalcup/tests/fixtures/version-lines.tsv` relative to the test file and asserts `parse(raw) == (None if exp == "-" else exp)` per row; `TestMatches.test_specs_match` repeats the twelve `specs_match` cases from F.1 on text (`matches("2.2.2", "2.")` ...).
- [ ] Step 2: Run, expect FAIL: `python3.11 -m unittest discover -s scripts/pad -p 'test_harness_version.py'` -> ModuleNotFoundError harness_version.
- [ ] Step 3: Implement harness_version.py per the grammar (one regex on the stripped word).
- [ ] Step 4: Replace pad-run:314-315: run `version_cmd` as today, then `version = harness_version.parse(next((l for l in r.stdout.splitlines() if l.strip()), ""))` (`import harness_version` after the `sys.path.insert(0, PAD)` at line 14; the box gets it with the `~/pad` push, pad:684). copilot-cli.toml:16 becomes `version_cmd = "copilot --version"`.
- [ ] Step 5: Run, expect PASS: `python3.11 -m unittest discover -s scripts/pad -p 'test_*.py'` (as scripts/gate.sh:86)
- [ ] Step 6: Commit: `git add scripts/pad/harness_version.py scripts/pad/test_harness_version.py scripts/pad/pad-run scripts/pad/harnesses/copilot-cli.toml; git commit -m "fix(pad): read harness versions with the goalcup grammar, drop the regex and the copilot sed"`

### Task F.3: version checks use version.rs; a wrong OpenCode pick names the other id   (S)
**Files:** Modify connector/mod.rs:60-68, 233-263, 275-277; cli/text.rs:179-182; cli/setup.rs:124-126; docs/PARTICIPANT-GUIDE.md:464; Test crates/goalcup/tests/connector_launch.rs:150-185 (new test after it)
**Interfaces:** Consumes F.1. Produces `Refusal::VersionMismatch { program, want, got, sibling }`
- [ ] Step 1: Write the failing tests in connector_launch.rs:
  - change `probe` to return the plan; keep the `v2.1.0` asserts; replace the two `VersionMismatch` asserts with: `"echo fakeharness 1.9.0"` -> `refusal == None` and `version_notes == vec!["fakev 1.9.0 is not the version this connector was written for (2.x). goalcup records it the same way; goalcup status says if model requests are missing."]`.
  - new `a_wrong_opencode_pick_names_the_other_connector`: fake `opencode` in the temp dir printing `opencode v2.2.2`; `finished("opencode-cli", ..).refusal == Some(Refusal::VersionMismatch { program: "opencode".into(), want: "1.".into(), got: "2.2.2".into(), sibling: "opencode2-cli".into() })`; `text::refusal(&that) == "opencode --version shows 2.2.2; this connector is for opencode 1.x. Run: goalcup setup --harness opencode2-cli"`. Then the fake prints `boom` and exits 1 (fresh temp dir): `refusal == None` and `version_notes == vec!["goalcup could not read opencode --version, so it could not check this is opencode 1.x. If it is 2.x, run: goalcup setup --harness opencode2-cli"]`.
- [ ] Step 2: Run, expect FAIL: `cargo nextest run -p goalcup --test connector_launch` -> struct field errors, then assertion failures.
- [ ] Step 3: Implement mod.rs 237-263: `read = harness_version.as_deref().and_then(version::parse)`; `others` = `same_program(program)` without this id. `(Some(want), Some(v))` not matching: the first `others` row that matches -> `VersionMismatch`, none -> push `untested_say`. `(Some(want), None)` -> push `unchecked_say`. `version_notes` rows hold when `read` matches one of `versions`. The pushed says go into `version_notes`, so start, continue and setup print them and 275-277 records them as `note what=version_note`. Delete `held` and `exact`.
- [ ] Step 4: text.rs:179-182 to the pinned string (spec `"1."` shows as `1.x`); setup.rs:124-126 returns `text::refusal` for `VersionMismatch` as for `HomeLocked`; PARTICIPANT-GUIDE.md:464 becomes `| \`<program> --version shows <version>; this connector is for <program> <major>.x. Run: goalcup setup --harness <id>\` | Run the command it names. OpenCode 1 and 2 share the \`opencode\` program and have a connector each. |`
- [ ] Step 5: Run, expect PASS: `cargo nextest run -p goalcup --test connector_launch` and `grep -n "Install that version\|Install that one" docs/PARTICIPANT-GUIDE.md crates/goalcup/src` -> no output.
- [ ] Step 6: Commit: `git add crates/goalcup/src/connector/mod.rs crates/goalcup/src/cli/text.rs crates/goalcup/src/cli/setup.rs crates/goalcup/tests/connector_launch.rs docs/PARTICIPANT-GUIDE.md; git commit -m "fix(connector): never refuse a version; a wrong OpenCode pick names the other connector"`

### Task F.4: tier line reads the version and the OS   (S, after F.1)
**Files:** Modify cli/start.rs:431-445, cli/text.rs:1158-1175, cli/setup.rs:127-130; Test cli/start.rs `mod tests` 461-469
**Interfaces:** `pub fn tier_line(name: &str, version: Option<&str>, tier: &str, verified: &str, os: &str, host_os: &str) -> String` (text.rs); start.rs `tier_line(name, probed, id)` keeps its signature, so the call at 298-302 does not move. `Row` gains `#[serde(default)] os: String`.
- [ ] Step 1: Write the failing tests in start.rs `mod tests`:
  - `tier_line_reads_v_prefix_and_trailing_dot`: `super::tier_line("Hermes", Some("Hermes Agent v0.22.6 (2026.9.24)"), "hermes-cli").starts_with("Hermes 0.22.6: ")`; `super::tier_line("Copilot", Some("GitHub Copilot CLI 1.0.105."), "copilot-cli").starts_with("Copilot 1.0.105: ")`.
  - `tier_line_says_build_and_os` (text::tier_line):
    - `("Hermes", Some("0.22.6"), "verified", "0.22.6", "darwin-arm64", "macos")` == `"Hermes 0.22.6: verified with goalcup on 0.22.6 (macOS)."`
    - `(.., Some("0.22.7"), ..)` == `"Hermes 0.22.7: not verified on 0.22.7; verified with goalcup on 0.22.6 (macOS). goalcup records it the same way; goalcup status says if model requests are missing."`
    - `(.., Some("0.22.6"), .., "darwin-arm64", "linux")` == `"Hermes 0.22.6: verified with goalcup on 0.22.6 (macOS), not on Linux. goalcup records it the same way; goalcup status says if model requests are missing."`
    - `(.., None, ..)` == `"Hermes: verified with goalcup on 0.22.6 (macOS); the version here could not be read, so goalcup cannot say it is that build."`
    - `("Claude Code", Some("2.1.312"), "verified", "2.1.312", "", "linux")` == `"Claude Code 2.1.312: verified with goalcup on 2.1.312."`
    - not verified and not supported strings stay as today.
- [ ] Step 2: Run, expect FAIL: `cargo test -p goalcup --lib cli::start::tests` -> arity error, then "Hermes: not verified" for the v-prefix case.
- [ ] Step 3: Implement: start.rs parses `probed` and `row.version` with `version::parse` (a `verified` that does not parse is used as written) and passes `row.os` and `std::env::consts::OS`. OS label: the part before `-`, `darwin`/`macos` -> `macOS`, `linux` -> `Linux`, `""` -> no label and no OS compare, other -> as written. setup.rs:127-130 shows `version::parse(v).map_or(v, text)`.
- [ ] Step 4: Run, expect PASS: `cargo test -p goalcup --lib cli::start::tests`
- [ ] Step 5: Commit: `git add crates/goalcup/src/cli/start.rs crates/goalcup/src/cli/text.rs crates/goalcup/src/cli/setup.rs; git commit -m "fix(cli): tier line reads v-prefixed and dotted versions and names the OS"`

### Task F.5: probe reads stderr and a named version on a failed exit   (S, PAD for step 5, PARALLEL-OK with F.3/F.4)
**Files:** Modify connector/launch.rs:146-171; Test version.rs `mod tests`
**Interfaces:** `pub(crate) fn harness_version(program: &Path, vars: &BTreeMap<String, String>) -> Option<String>` unchanged; still returns the raw line (recorded unchanged).
- [ ] Step 1: Write the failing test `probe_reads_stderr_and_a_named_version_on_a_failed_exit` in version.rs: a `#!/bin/sh` script named `fakev` in a tempdir, empty vars, `crate::connector::launch::harness_version`:
  - `echo fakev 1.2.3` -> `Some("fakev 1.2.3")`
  - `echo fakev 2.0.0 >&2` -> `Some("fakev 2.0.0")`
  - `echo 'Fakev Agent v0.22.6 (2026.9.24)' >&2; exit 1` -> `Some("Fakev Agent v0.22.6 (2026.9.24)")`
  - `echo 'error: needs Python 3.11.4' >&2; exit 1` -> `None`
  - `echo fakev 1.2.3; exit 1` -> `Some("fakev 1.2.3")`
- [ ] Step 2: Run, expect FAIL: `cargo test -p goalcup --lib connector::version` -> the stderr cases give None.
- [ ] Step 3: Implement: pipe stderr as well (both non-blocking after exit, as stdout today); line = first non-empty stdout line, else first non-empty stderr line. Success exit: return it. Failed exit: return it only if `version::parse(line)` is Some and the line contains the program's file name, ASCII case-insensitive. Timeout unchanged (5 s).
- [ ] Step 4: Run, expect PASS: `cargo test -p goalcup --lib connector::version` and `cargo nextest run -p goalcup --test connector_launch`
- [ ] Step 5 (PAD acceptance, owner's Mac): `GOALCUP_HOME="$(mktemp -d)" GOALCUP_SERVER=off target/debug/goalcup setup --harness hermes-cli --yes` prints `Hermes Agent <x.y.z> (...)`; then the next queued `scripts/pad/pad scripts/pad/runs/core-hermes.toml` row has a non-null `harness_version` in its start event (pad-out/hermes-cli/*/verify/usage/*.json holds `"harness_version":null` 10 times today). If Hermes prints no version on that exit, write so in the PR: versions-6 then stays with the seal read (MULTI-VERSION-PLAN task 10).
- [ ] Step 6: Commit: `git add crates/goalcup/src/connector/launch.rs crates/goalcup/src/connector/version.rs; git commit -m "fix(connector): version probe reads stderr and a named version on a failed exit"`
