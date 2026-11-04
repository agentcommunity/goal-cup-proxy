# Lane I: Multi-version connectors: [[versions]] overlays, version in the record, verified list
**Wave:** 2   **Depends on:** E, F (consumed by name below); H.3 then H.5 merged before I.1 (order below); C merged first for the two `pipeline/mod.rs` and `watchdog/mod.rs` hunks; G.22 for I.6   **Branch:** fix/version-overlays
**Covers findings:** versions-4, versions-8 (record half), versions-10, versions-11, versions-3 (Rust reader half), MULTI-VERSION-PLAN tasks 10, 11, 12, 14, 15, 17; arch-8 version half (dropped)
**Owns files:** crates/goalcup/src/connector/schema.rs (wave 2: after E's and H.3's one field each), connector/mod.rs (`plan`, `Refusal`, `LaunchPlan`, `load`), connectors/*.toml except hermes-cli.toml (header comments, `checked`, `[[versions]]`, `version` keys; not `[usage]`), connectors/opencode2-cli.toml (deleted), crates/goalcup/src/pipeline/boot.rs `start_event`, crates/goalcup/src/pipeline/harness_watch.rs (new), crates/goalcup/tests/connector_versions.rs (new), crates/goalcup/tests/fixtures/opencode-1.0/{opencode-cli,opencode2-cli}.toml (new, frozen copies), docs/harnesses/opencode-cli.md:3-5
**Shared files (only these parts):**
- connector/version.rs: delete F's `same_program`, `untested_say`, `unchecked_say` and their tests; tests/fixtures/version-lines.tsv: the `opencode2-cli` row's first column becomes `opencode-cli`.
- connector/launch.rs: none (`build_env`, `finish_env`, `harness_version` reused unchanged).
- cli/start.rs: `Row` and `tier_line` (F.4's 431-445 and its tests), `lines.extend(connector.first_screen…)` (289), the StartFacts literal (one field `version_row`, near 380).
- cli/continue_run.rs: the id check 141-150, `first_screen` (289), the version compare 282-288 moved above the StartFacts call.
- cli/setup.rs: 124-138 (`VersionMismatch` branch gone; `c.update_off`/`c.no_prompts` read from `plan.effective`).
- cli/text.rs: the `VersionMismatch` arm (F's 179-182) deleted; `tier_line` (F's 1158-1175); new `harness_changed`, `native_missing_redirect`.
- cli/status.rs:169-171 (`native_missing` arm) plus one `harness_changed` arm.
- watchdog/mod.rs (lane C's): `ToRecorder::Seal` (1023-1026) one field; the `after` thread (497-508) one call; the two `ToRecorder::Seal {..}` literals (441, 519); `StartFacts` one field (after 150).
- pipeline/mod.rs (lane C's): the `Launch` arm (979-982) and the 1 s loop (one `if let Some(note) = watch.check(now)` line); the `Seal` arm passes `version_read`.
- pipeline/seal.rs:52-60 (`SealAsk.version_read`) and 106-110 (one note beside `outlived`).
- native/mod.rs:222-224 (`missing_note` gains `redirect`); connector/mod.rs `NativeSource` (owned).
- goalcup-format/src/connectors.rs (E's): one `pub const RENAMED`; goalcup-verify/src/usage/hints.rs (E's): `usage()` looks through `RENAMED`, outer parse struct gains `checked`.
- goalcup-verify/src/usage/mod.rs (D's): `CallTable` three fields and `start()` 365-372; usage/out.rs (D's): the top-level `json!` at 114-122; usage/native.rs (E's): 171-173 (count a non-JSON line).
- scripts/pad/table:19-22 (G's): drop `"opencode2-cli"` from `CONNECTOR_IDS`; scripts/pad/harnesses/opencode2-cli.toml (G's): deleted; connectors/tiers.toml: the `[opencode2-cli]` block deleted.
- docs/PARTICIPANT-GUIDE.md:464 (F's row) deleted; docs/RECORD-FORMAT.md (D's): the `start` key list and the notes list, one line each for `harness_semver`, `version_row`, `version_read`, `harness_changed`, `native_missing.redirect`.
**Produces (for other lanes):**
- Connector keys: `checked = "<version>"` (optional; must `version::parse` to itself) and `[[versions]]` rows. Row keys, exactly: `match` (required, `version::matches` spec), `say`, and the overrides `args`, `resume`, `env`, `unset`, `host_hints`, `secret_files`, `no_prompts`, `update_off`, `native`, `first_screen`, `before`, `after`. Any other key (`id`, `name`, `program`, `home`, `ca_env`, `ca_settings`, `mask`, `refuse_if_set`, `usage`, `checked`, `versions`, `hint_env`, `skills_dir`, `end_note`) fails the load with serde's `unknown field` and the line.
- Merge: the first row whose `match` holds wins. A list or string replaces the base value; `env` merges key by key; `first_screen = ""` clears it. The merged connector passes the same `check()` as the base, error keys prefixed `versions[<i>].`. Unread version: no row applies. Order rows most specific first; `connector_builtin` proves every built-in row is the first match for at least one fixture line.
- `schema::VersionRow` (pub fields as above, `#[serde(rename = "match")] pub spec: String`), `impl Connector { pub fn for_version(&self, v: Option<&version::HarnessVersion>) -> (Connector, Option<String>) }` (effective connector, applied `match` text).
- `LaunchPlan.effective: Connector` (the merged connector; read `first_screen`, `no_prompts`, `update_off` from it), `LaunchPlan.version_row: Option<String>`. `Refusal::VersionMismatch` and `Connector.{version, version_notes}` are gone.
- Record (additive JSON, no format bump): `start` gains `harness_semver` (parsed text or null) and `version_row` (or null); notes `{"what":"version_read","line","version","before"}` (seal, continue), `{"what":"harness_changed","program","before":{size,mtime_ns,ino},"after":{..}}`, and `native_missing` gains `"redirect": "<home var>"` (or null). `os` is already in `start` (boot.rs:247).
- `goalcup_format::connectors::RENAMED: &[(&str, &str)] = &[("opencode2-cli", "opencode-cli")]`, honoured by `connector::load` and the verifier's `usage()`.
- For lane H: Hermes gets no row (H-hermes.md: the CA problem is macOS-wide since 0.21.6, fixed by base `ca_settings`). The general syntax, for any later harness release that changes a key:
  ```toml
  checked = "2.2.2"
  args = ["--standalone"]
  [[versions]]
  match = "1.x"        # most specific row first; spec grammar is version::matches
  args = []
  say = "<optional line said at start and recorded>"
  ```
**Consumes:** H.3: `Connector.ca_settings` (base only; refused in rows). H.5: Hermes `version_notes` deleted (this lane does not edit connectors/hermes-cli.toml). E: `goalcup_format::connectors::CONNECTORS`, `schema::Connector.usage: toml::Table`, hints.rs `parse`/`usage`. F: `version::{HarnessVersion, parse, matches}`, the fixture TSV, `text::tier_line` (replaced here), `LaunchPlan.version_notes`. G.22: `[[<id>.builds]]` tables `{version, os}` in tiers.toml (last 5 per os). C: `StartFacts` after C.11 (no `build`/`server_time`/`local_wall`). D: owns usage/mod.rs and out.rs; this lane adds fields only.

## Dropped
- G.24's `ca_env` row for Hermes 0.22.6 (coordinator question): **no CA key in rows, and no Hermes row.** H-hermes.md shows that no variable reaches Hermes model calls on macOS, on any version since 0.21.6. It is an OS split, fixed by H.3's base key `ca_settings`. Both `ca_env` and `ca_settings` are therefore refused inside `[[versions]]` (I.1 test). Trust setup is the same for every build, and a row can never weaken it. G.24 is superseded by H.5.
- MULTI-VERSION-PLAN task 13 (Hermes row): lane H, and not needed.
- `gap` key in a row and the `tier_said`/`tier_basis` start keys (MULTI-VERSION-PLAN): the row `say` is already recorded as a `version_note` note and `version_row` names the rules; the tier line is derivable from tiers.toml at that build. Less code.
- Overlap warning at load time: a loader warning has nowhere to go. Replaced by the I.1 test that each built-in row is the first match of a fixture line.
- `harness_changed` on `upload/beat.rs` (brief): the upload thread cannot write to the record. The stat runs on the recorder's existing 1 s control loop (pipeline/mod.rs:976), every 300 s, with no exec.
- Version read in `cli/seal.rs` (brief): that is the manual `goalcup seal`, which has no harness env. The read runs in the watchdog's existing `after` thread (watchdog/mod.rs:497-508), after the harness has exited. It travels in `ToRecorder::Seal` and is written by pipeline/seal.rs beside `outlived`.
- Unknown-shape counts per reader: only non-JSON lines are counted (`lines_unread`). Drift of a JSON shape shows as `reason: no_native` next to `harness_semver` and `native_checked_on`.
- arch-8 version half: nothing left after D (shared tables) and E (connector data).
- versions-5/x2 (the Hermes CA problem): lane H (H.5 supersedes G.24).
- Copilot `checked`: its connector has no version header (copilot-cli.toml:1). It gets no `checked`, so 14 connectors carry one, not 16.

## Review focus
1. A row that adds `env.OPENAI_BASE_URL`, a `secret_files` path outside `{home}`, or `env.HOME` must fail the load like the base would (I.1 `a_row_is_checked_like_the_base`).
2. An `opencode2-cli` run started on 1.0.0 must still `continue`; today continue_run.rs:144 refuses any id change (I.2 `an_opencode2_run_continues_as_opencode`).
3. OpenCode with an unread version gets the 2.x base and is told so; it is never refused (I.2).
4. The program stat must follow an npm symlink retarget and never act on a change, only note it once per change (I.4 test).
5. `first_screen = ""` in a row must clear the base line, not print an empty line (I.2 golden test, 1.x has none).

### Task I.1: `checked`, `[[versions]]` and the merge; version keys removed   (M)
**Files:** Modify connector/schema.rs:15-66 (fields, `VersionRow`, delete `VersionNote`), :216-221 (`parse` checks each merged row), connector/mod.rs:60-68, 102-144, 211-341, connector/version.rs (delete F's three fns), cli/text.rs (VersionMismatch arm), cli/setup.rs:124-138, cli/start.rs:289, cli/continue_run.rs:289, docs/PARTICIPANT-GUIDE.md:464; Create crates/goalcup/tests/connector_versions.rs; Modify crates/goalcup/tests/connector_launch.rs (F's VersionMismatch tests), cli_start.rs:302-365 (`[[version_notes]]` → `[[versions]]`)
**Interfaces:** Produces `VersionRow`, `Connector::for_version`, `LaunchPlan.{effective, version_row}` (header).
- [ ] Step 1: Write failing tests in `connector_versions.rs` (helper `c(rows) = load_text(&format!("id = \"t-cli\"\nname = \"T\"\nprogram = \"t\"\nargs = [\"base\"]\nenv = {{ A = \"1\" }}\nca_env = [\"T_CA\"]\nfirst_screen = \"Hi.\"\n{rows}"))`, `v = |s| version::parse(s)`):
  ```rust
  // first_match_wins_and_merges
  let k = c("[[versions]]\nmatch = \"1.2.3\"\nargs = [\"exact\"]\n[[versions]]\nmatch = \"1.x\"\nargs = []\nenv = { B = \"2\" }\nfirst_screen = \"\"\n").unwrap();
  let (e, row) = k.for_version(v("t 1.2.3").as_ref());
  assert_eq!((e.args.clone(), row.as_deref()), (vec!["exact".to_string()], Some("1.2.3")));
  let (e, row) = k.for_version(v("1.9.0").as_ref());
  assert_eq!(row.as_deref(), Some("1.x"));
  assert!(e.args.is_empty() && e.first_screen.is_none());
  assert_eq!(e.env.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>(), ["A=1", "B=2"]);
  assert_eq!(e.ca_env, ["T_CA"]);
  let (e, row) = k.for_version(None);
  assert_eq!((e.args.clone(), row, e.first_screen.as_deref()), (vec!["base".to_string()], None, Some("Hi.")));
  // a_row_refuses_the_fixed_keys
  for key in ["home = { X = \"{home}\" }", "ca_env = [\"X\"]", "ca_settings = []", "mask = []", "refuse_if_set = []", "usage = {}", "id = \"x\"", "program = \"y\"", "checked = \"1.0\"", "hint_env = []"] {
      let err = c(&format!("[[versions]]\nmatch = \"1.x\"\n{key}\n")).unwrap_err().to_string();
      assert!(err.contains("unknown field"), "{key}: {err}");
  }
  // a_row_is_checked_like_the_base
  assert!(matches!(c("[[versions]]\nmatch = \"1.x\"\nenv = { OPENAI_BASE_URL = \"x\" }\n"), Err(ConnectorError::ModelAddress { key, .. }) if key == "versions[0].env.OPENAI_BASE_URL"));
  assert!(matches!(c("[[versions]]\nmatch = \"1.x\"\nunset = []\nenv = { HOME = \"x\" }\n"), Err(ConnectorError::Invalid { key, .. }) if key == "versions[0].env.HOME"));
  assert!(matches!(c("[[versions]]\nmatch = \"1.x\"\nsecret_files = [\"/etc/passwd\"]\n"), Err(ConnectorError::Invalid { .. })));
  // checked_must_be_a_version
  assert!(c("checked = \"soon\"\n").is_err() && c("checked = \"2.2.2\"\n").is_ok());
  // every_builtin_row_is_reached_by_a_fixture_line: for each (id, _) in builtin::CONNECTORS with rows,
  // each row index i is the first matching row for at least one fixture line of that id (TSV column 1).
  ```
  In `connector_launch.rs` replace F's opencode/`fakev` VersionMismatch asserts with `the_unread_version_uses_the_base_and_says_so`: a test connector `checked = "2.0.0"` plus one row `match = "1.x"`, fake program `exit 1` → `plan.refusal == None`, `plan.version_row == None`, `plan.version_notes == ["goalcup could not read fakeharness --version, so it used the rules for T 2.0.0."]`. In cli_start.rs, `start_with_note` writes `[[versions]]\nmatch = \"1.2.3\"\nsay = \"Tester 1.2.3 loses calls.\"\n`; the three tests keep their asserts.
- [ ] Step 2: Run `cargo nextest run -p goalcup --test connector_versions --test connector_launch --test cli_start`; expect compile errors (`for_version`, `version_row` missing).
- [ ] Step 3: Implement.
  - schema.rs: drop `version`, `version_notes`, `VersionNote`. Add `pub checked: Option<String>` and `pub versions: Vec<VersionRow>`. `VersionRow` is `#[serde(deny_unknown_fields)]` with `Option<_>` for every override.
  - `for_version`: clone, then the first row with `matches(v, &row.spec)`.
  - `parse`: after `check(&c)`, it runs `check` on `c.apply(i)` for every row and prefixes the error key with `versions[i].`. `checked` must equal `version::parse(checked).text`.
  - mod.rs `plan`: probe with `build_env(connector)` as today, then `let (c, version_row) = connector.for_version(parsed.as_ref())`. `harness_env`, `argv`, hints, `update_off`, `native` and `before`/`after` come from `c`; `ca_env` and H.3's `ca_settings` cannot differ between `c` and `connector`. Home folders are made from `connector.home` (rows cannot change it).
  - Row `say` and the unread line go to `version_notes` (recorded as `version_note` notes, mod.rs:275-278, unchanged). The unread line is pushed only when `!connector.versions.is_empty()`, and names `checked`, else "the newest version it knows".
  - Delete `held`, `exact`, `Refusal::VersionMismatch`, F's `same_program`/`untested_say`/`unchecked_say`, the text.rs arm, the setup.rs branch and PARTICIPANT-GUIDE.md:464.
  - start.rs:289 and continue_run.rs:289 read `plan.effective.first_screen`; setup.rs:136-138 reads `plan.effective`.
- [ ] Step 4: Run `cargo nextest run -p goalcup --test connector_versions --test connector_launch --test cli_start --test connector_builtin` and `cargo test -p goalcup --lib connector::version`; expect PASS. `git grep -n "version_notes\]\]\|VersionMismatch\|same_program" crates connectors docs/PARTICIPANT-GUIDE.md` prints nothing.
- [ ] Step 5: Commit: `git add crates/goalcup docs/PARTICIPANT-GUIDE.md; git commit -m "feat(connector): [[versions]] rows pick the rules by version; no version refuses a run"`

### Task I.2: OpenCode is one connector   (M, after I.1)
**Files:** Create tests/fixtures/opencode-1.0/opencode-cli.toml, opencode2-cli.toml (`git show 70eecdfb:connectors/<f>` with the `version =` line removed and E's `[usage]` table kept); Modify connectors/opencode-cli.toml (whole), goalcup-format/src/connectors.rs (`RENAMED`), connector/mod.rs `load` (186-209), cli/continue_run.rs:141-150, goalcup-verify/src/usage/hints.rs `usage()`, tests/fixtures/version-lines.tsv (one cell), connectors/tiers.toml (`[opencode2-cli]` block), scripts/pad/table:22, docs/harnesses/opencode-cli.md:3-5; Delete connectors/opencode2-cli.toml, scripts/pad/harnesses/opencode2-cli.toml; Test connector_versions.rs, cli_continue.rs
**Interfaces:** Produces `goalcup_format::connectors::RENAMED`.
- [ ] Step 1: Write failing tests.
  - `opencode_fold_keeps_each_version_s_keys` (connector_versions.rs): `keys(c) = format!("{:?}", (&c.args, &c.resume, &c.home, &c.env, &c.ca_env, &c.unset, &c.host_hints, &c.secret_files, &c.native, &c.no_prompts, &c.update_off, &c.first_screen, &c.before, &c.after, &c.mask, &c.usage))`. For `(line, old)` in `[("1.19.5", "opencode-cli.toml"), ("opencode v2.2.2", "opencode2-cli.toml")]`: `keys(&load("opencode-cli")?.for_version(version::parse(line).as_ref()).0) == keys(&schema::parse(fixture(old))?)`. Also `for_version(None)` and `parse("opencode v3.0.0")` give the 2.2.2 keys with row `None`; `load("opencode2-cli", None)?.id == "opencode-cli"`.
  - `an_opencode2_run_continues_as_opencode` (cli_continue.rs): edit the fixture run's `server.json` connector id to `opencode2-cli` with source `builtin`; `continue` with a fake `opencode` printing `opencode v2.2.2` must not print `text::file_changed(..)`, and the new entry's start event has `connector == "opencode-cli"`.
  - In E's hints.rs table test, keep `wire_ids(Some("opencode2-cli"), ..)` and add `assert_eq!(j("opencode2-cli"), Some((true, false)))`.
- [ ] Step 2: Run `cargo nextest run -p goalcup --test connector_versions opencode --test cli_continue opencode2` and `cargo test -p goalcup-verify --lib usage::hints::tests`; expect FAIL: no row `1.x`, and `file_changed` printed.
- [ ] Step 3: Implement.
  - `connectors/opencode-cli.toml`:
    - The base is today's opencode2-cli body (`args = ["--standalone"]`, its `first_screen`, `host_hints`, `secret_files`, no `no_prompts`), with `checked = "2.2.2"`.
    - Then one row: `[[versions]]\nmatch = "1.x"   # checked on 1.19.5`, with `args = []`, `first_screen = ""` and 1.19.5's four `host_hints`, four `secret_files` and its `no_prompts`.
    - The header comment merges both sources lines.
  - `RENAMED`: `load` maps the id through it before the lookup. continue_run.rs:144 compares `connector.id` with the renamed `sj.connector.id`. `usage()` looks up the renamed id.
  - Delete the two files, the tiers block and the `CONNECTOR_IDS` entry. Change the TSV cell. docs/harnesses/opencode-cli.md:3-5 says one connector `opencode-cli`, 2.x base and a 1.x row.
- [ ] Step 4: Run `cargo nextest run --workspace --locked` and `python3 -m unittest discover -s scripts/pad -p 'test_*.py'`; expect PASS. `git grep -n opencode2 -- ':!docs/plans' ':!docs/archive' ':!docs/superpowers' ':!crates/*/tests'` lists only `RENAMED`, docs/RECORD-FORMAT.md:565 (D: says "and `opencode2-cli` before 1.1") and docs/harnesses/README.md:24, docs/HARNESS-SETUP.md:93 (lane L.8 rewrites them; listed in the PR).
- [ ] Step 5: Commit: `git add -A connectors crates scripts/pad/table scripts/pad/harnesses docs/harnesses/opencode-cli.md; git commit -m "feat(connector): OpenCode 1 and 2 are one connector with a 1.x row; old opencode2-cli runs continue"`

### Task I.3: The version in the start event and at continue   (S, PARALLEL-OK with I.2)
**Files:** Modify pipeline/boot.rs:237-258, watchdog/mod.rs `StartFacts` (+1 field), cli/start.rs StartFacts literal, cli/continue_run.rs:282-288 (moved above the facts call); Test cli_start.rs, cli_continue.rs
**Interfaces:** `StartFacts.version_row: Option<String>` (`#[serde(default)]`).
- [ ] Step 1: Write failing tests. cli_start.rs `start_with_note` also returns the start event:
  - `the_start_event_names_the_build_and_the_row`: version `1.2.3` gives `start["harness_semver"] == "1.2.3"` and `start["version_row"] == "1.2.3"`; `1.2.4` gives `"1.2.4"` and `null`; a script printing nothing (exit 1) gives both `null`.
  - cli_continue.rs `a_changed_version_at_continue_is_a_note`: the fake prints `tester 1.2.3` at start and `tester 1.2.4` at continue. The second entry has exactly one note `{"what":"version_read","line":"tester 1.2.4","version":"1.2.4","before":"1.2.3"}`, and the screen still shows `text::updated(..)`.
- [ ] Step 2: Run `cargo nextest run -p goalcup --test cli_start start_event_names --test cli_continue changed_version`; expect FAIL (keys absent).
- [ ] Step 3: Implement. `start_event` adds `"harness_semver": f.harness_version.as_deref().and_then(version::parse).map(|v| v.text)` and `"version_row": f.version_row`. start.rs sets `version_row: plan.version_row.clone()`. continue_run pushes the note into `plan.notes` before the StartFacts call when the parsed texts differ; `before` is the previous entry's parsed text or null. cli/recover.rs leaves `version_row` `None` (the first segment's start event holds it).
- [ ] Step 4: Run the Step 2 command; expect PASS.
- [ ] Step 5: Commit: `git add crates/goalcup; git commit -m "feat(record): start names the parsed build and the version row; continue notes a changed build"`

### Task I.4: Read at seal, stat on the loop   (S, PARALLEL-OK with I.2)
**Files:** Create pipeline/harness_watch.rs (`pub(crate) mod harness_watch;` in pipeline/mod.rs); Modify pipeline/mod.rs (field beside `harness` at 938, `Launch` arm 979-982, loop one line, `Seal` arm 991), watchdog/mod.rs (`ToRecorder::Seal` + literals 441, 519; `after` thread 497-508), pipeline/seal.rs:52-60, 106-110, cli/text.rs, cli/status.rs:169; Test harness_watch.rs `mod tests`, watchdog_end.rs
**Interfaces:** `pub(crate) struct HarnessWatch`, `HarnessWatch::new(program: &Path, now: Mono) -> HarnessWatch`, `fn check(&mut self, now: Mono) -> Option<serde_json::Value>`, `const EVERY_NS: u64 = 300 * 1_000_000_000`. `ToRecorder::Seal { reason, outlived, #[serde(default, skip_serializing_if = "Option::is_none")] version_read: Option<String> }`. `text::harness_changed(name: &str) -> String`.
- [ ] Step 1: Write failing tests.
  - harness_watch.rs `a_change_is_noted_once_and_a_symlink_is_followed`:
    - Setup: a tempdir with file `a` (3 bytes), file `b` (5 bytes), symlink `prog -> a`; `w = HarnessWatch::new(prog, Mono(0))`.
    - `w.check(Mono(EVERY_NS - 1)) == None`.
    - Retarget `prog -> b`: `w.check(Mono(EVERY_NS))` is `Some(n)` with `n["what"] == "harness_changed"`, `n["before"]["size"] == 3`, `n["after"]["size"] == 5`.
    - `w.check(Mono(2 * EVERY_NS)) == None`.
    - The program removed: `w.check(Mono(3 * EVERY_NS))` has `n["after"] == Value::Null`, then `None` again.
  - watchdog_end.rs `seal_records_a_version_read_after_exit`: a fake harness that prints `fake 1.0.0` for `--version`, then rewrites itself to print `fake 1.1.0` while running. The sealed record holds one note `what == "version_read"` with `version == "1.1.0"` and `before == "1.0.0"`. A harness that does not change writes none.
  - status.rs unit test: a `harness_changed` note gives `text::harness_changed("Tester")` == `"Tester changed on disk during the run (an update?). goalcup keeps recording; the record shows when."`.
- [ ] Step 2: Run `cargo test -p goalcup --lib pipeline::harness_watch` and `cargo nextest run -p goalcup --test watchdog_end version_read`; expect compile error, then FAIL.
- [ ] Step 3: Implement.
  - The stat uses `std::fs::metadata` (follows links): `size`, `mtime` in ns and `ino`. A change updates the baseline. The recorder makes the watch on `Launch` from `HarnessFacts.program`; the loop calls `check(self.rec.now())` and writes `Some` with `self.rec.event`.
  - The watchdog's `after` thread returns `launch::harness_version(&program, &env)` after the helpers. It is sent as `version_read` only when `version::parse` of it differs from `plan.harness_version`'s. Recover sends `None`.
  - seal.rs writes `{"ev":"note","what":"version_read","line","version","before"}` beside `outlived`. status.rs adds the `harness_changed` arm.
- [ ] Step 4: Run the Step 2 commands, then `cargo nextest run -p goalcup --test watchdog_end --test watchdog_recover`; expect PASS.
- [ ] Step 5: Commit: `git add crates/goalcup; git commit -m "feat(record): read the version after the harness exits; note a program change on disk"`

### Task I.5: Tier line lists the verified builds   (S, after G.22; PARALLEL-OK with I.2-I.4)
**Files:** Modify cli/start.rs:431-445 (`Row`, `tier_line`) and tests 461-469+, cli/text.rs `tier_line`
**Interfaces:** `pub fn tier_line(name: &str, version: Option<&str>, tier: &str, builds: &[(String, String)], host_os: &str) -> String` (replaces F's six-argument form). `Row { tier, version, #[serde(default)] os: String, #[serde(default)] builds: Vec<Build> }`, `Build { version: String, os: String }`. With no `builds`, a non-empty `(version, os)` is the one build.
- [ ] Step 1: Write failing tests in start.rs `mod tests` (`b = [("0.22.6","darwin-arm64"),("0.22.5","darwin-arm64"),("0.22.5","linux-x86_64")]`, all `"verified"`):
  - `("Hermes", Some("0.22.5"), b, "macos")` == `"Hermes 0.22.5: verified with goalcup on 0.22.5 (macOS)."`
  - `(Some("0.22.7"), "macos")` == `"Hermes 0.22.7: not verified on 0.22.7; verified with goalcup on 0.22.6, 0.22.5 (macOS). goalcup records it the same way; goalcup status says if model requests are missing."`
  - `(Some("0.22.6"), "linux")` == `"Hermes 0.22.6: not verified on 0.22.6; verified with goalcup on 0.22.5 (Linux). goalcup records it the same way; goalcup status says if model requests are missing."`
  - `(Some("0.22.6"), b[..2], "linux")` == `"Hermes 0.22.6: verified with goalcup on 0.22.6 (macOS), not on Linux. goalcup records it the same way; goalcup status says if model requests are missing."`
  - `(None, "macos")` == `"Hermes: verified with goalcup on 0.22.6, 0.22.5 (macOS); the version here could not be read, so goalcup cannot say it is that build."`
  - `tiers_file_parses` also parses a `[[x.builds]]` table.
- [ ] Step 2: Run `cargo test -p goalcup --lib cli::start::tests`; expect FAIL (arity).
- [ ] Step 3: Implement. Builds on the host OS come first; with none, use every build with its own OS label. The OS label rule is F.4's.
- [ ] Step 4: Run `cargo test -p goalcup --lib cli::start::tests`; expect PASS.
- [ ] Step 5: Commit: `git add crates/goalcup/src/cli/start.rs crates/goalcup/src/cli/text.rs; git commit -m "feat(cli): the tier line names the verified builds near yours"`

### Task I.6: Verifier says the build, the rules and what it could not read   (S, PARALLEL-OK)
**Files:** Modify goalcup-verify/src/usage/mod.rs (`CallTable` near 276, `start()` 365-372, `Leg`/totals), usage/native.rs:171-173, usage/out.rs:114-122, usage/hints.rs (outer struct `checked`, `pub(super) fn checked(connector: Option<&str>) -> Option<&'static str>`); Test crates/goalcup-verify/tests/check_qwen_chats.rs
**Interfaces:** `CallTable.{harness_semver: Option<String>, version_row: Option<String>, native_checked_on: Option<String>, lines_unread: u64}`; JSON top level `harness_semver`, `version_row`, `native_checked_on`; `coverage.lines_unread`.
- [ ] Step 1: Write failing test `a_qwen_record_names_build_rules_and_unread_lines`: the existing Qwen fixture with `edit_event(.., "start", 0, |v| { v["connector"] = "qwen-cli".into(); v["harness_semver"] = "0.27.0".into(); v["version_row"] = Value::Null; })` and one extra line `not json\n` appended to `MAIN_FILE`. `call_table(&dir)` has `harness_semver == Some("0.27.0")`, `version_row == None`, `native_checked_on == Some("0.26.4")`, `lines_unread == 1`, and the native rows are unchanged. A start event without the keys gives `None`, `None`.
- [ ] Step 2: Run `cargo nextest run -p goalcup-verify --test check_qwen_chats names_build`; expect compile error.
- [ ] Step 3: Implement. Read the two keys like `harness_version` (printable ASCII, at most 64 bytes). `native_checked_on = hints::checked(connector)`. native.rs counts the `let Ok(v) = .. else` branch.
- [ ] Step 4: Run `cargo nextest run -p goalcup-verify`; expect PASS (`usage_run1` unchanged apart from the three new null keys and `lines_unread: 0`; update its expected JSON in the same commit).
- [ ] Step 5: Commit: `git add crates/goalcup-verify; git commit -m "feat(verify): usage names the build, the version row, the reader's checked build and unread lines"`

### Task I.7: A redirect that did not take effect is said   (S, PARALLEL-OK)
**Files:** Modify connector/mod.rs `NativeSource` (+`#[serde(default)] pub redirect: Option<String>`) and `plan` native rows (285-296), native/mod.rs:222-224, cli/status.rs:169-171, cli/text.rs (`native_missing_redirect`); Test connector_versions.rs, native/mod.rs tests
**Interfaces:** `redirect` = the `home` variable whose filled value is the longest path prefix of the row's filled `root` (ties: first name in order), `None` for `{workspace}` roots. `text::native_missing_redirect(row: &str, name: &str, var: &str) -> String`.
- [ ] Step 1: Write failing tests.
  - `the_three_redirects_are_named`: `plan(load("claude-code-cli"))` row `transcripts` gives `redirect == Some("CLAUDE_CONFIG_DIR")`; codex `sessions` gives `"CODEX_HOME"`; droid `sessions` gives `"FACTORY_HOME_OVERRIDE"`; crush `db` gives `None`.
  - native/mod.rs: `missing_note` for a source with redirect gives `{"ev":"note","what":"native_missing","row":"transcripts","redirect":"CLAUDE_CONFIG_DIR"}`.
  - status text: `text::native_missing_redirect("transcripts", "Claude Code", "CLAUDE_CONFIG_DIR")` == `"goalcup has found no transcripts file for Claude Code in its goalcup home. If this build no longer reads CLAUDE_CONFIG_DIR, its sessions go to your usual folder and are not recorded; model requests still are."`.
- [ ] Step 2: Run `cargo nextest run -p goalcup --test connector_versions redirects` and `cargo test -p goalcup --lib native::`; expect FAIL.
- [ ] Step 3: Implement. `warn_missing` keeps its meaning: the redirect check is that row firing. status.rs picks `native_missing_redirect` when the note has `redirect`.
- [ ] Step 4: Run the Step 2 commands and `cargo nextest run -p goalcup --test cli_status`; expect PASS.
- [ ] Step 5: Commit: `git add crates/goalcup; git commit -m "feat(native): a missing session row names the home variable the build may have stopped reading"`

### Task I.8: Version facts are data   (S, after I.1)
**Files:** Modify connectors/{amp,claude-code,cline,codex,crush,droid,gemini,goose,kilo,kimi-code,openclaw,qwen}-cli.toml headers (opencode in I.2), docs/RECORD-FORMAT.md (the start key list and the notes list); Test connector_versions.rs
- [ ] Step 1: Write failing test `checked_is_set_where_a_header_named_a_build`: the ids of built-ins with `checked` are the 14 ids other than `copilot-cli`, and their values are `amp 0.0.1793275344-g17bf52, claude-code 2.1.312, cline 3.0.74, codex 0.165.0, crush 0.99.0, droid 0.251.0, gemini 0.66.0, goose 1.57.0, hermes 0.22.5 (written by lane H in H.5, see notes), kilo 7.8.25, kimi-code 2.4.1, openclaw 2026.10.9, opencode 2.2.2, qwen 0.26.4`.
- [ ] Step 2: Run `cargo nextest run -p goalcup --test connector_versions checked_is_set`; expect FAIL.
- [ ] Step 3: In each file, add `checked = "<v>"` after `program`. Its header comment keeps the sources and drops the build number ("# Claude Code: values checked against the vendor's docs on 30 Oct 2026."). RECORD-FORMAT.md gets one line per new key and note (header **Produces**).
- [ ] Step 4: Run the test; then `git grep -nE "^# .*[0-9]+\.[0-9]+\.[0-9]+" connectors/*.toml` prints only model-hosts/tiers lines; expect PASS.
- [ ] Step 5: Commit: `git add connectors docs/RECORD-FORMAT.md crates/goalcup/tests/connector_versions.rs; git commit -m "chore(connectors): the checked build is a key, not a comment"`

## Notes for the index
- Merge order: I.1 → I.8 and I.2. I.3, I.4, I.6 and I.7 go in any order after I.1. I.5 waits for G.22. No PAD task here.
- Merge order with lane H (both edit schema.rs and mod.rs in wave 2): E → F → **H.3** (adds `ca_settings` to `struct Connector`, the check loop and `plan()` after 301) → **H.5** (deletes Hermes `version_notes`, which I.1's schema can no longer load) → **I.1** (rebased on both; `VersionRow` gets no `ca_settings` field) → the rest of I. H.4 and H.6/H.7 do not touch these files' I hunks.
- Lane H request: in H.5 add `checked = "0.22.5"` to connectors/hermes-cli.toml and drop the number from its header comment. I.8's test expects it, and this lane does not edit that file. If H.5 lands without it, I.8 drops hermes from the list.
- G.24 is superseded (H.5); there is no CA key in any row. G owns the opencode2 pad file and `CONNECTOR_IDS`: I.2 deletes them, so tell G. Lane L.8 documents the row syntax from **Produces** and rewrites harnesses/README.md:24 and HARNESS-SETUP.md:93. Lane C's watchdog/pipeline hunks merge before I.4.
- Owner questions: (1) G.22 keeps the last 5 builds per OS, not per harness: fine? (2) A `goalcup recover` segment records `version_row: null`: fine?
