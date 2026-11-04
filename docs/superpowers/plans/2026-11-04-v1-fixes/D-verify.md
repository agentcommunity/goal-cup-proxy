# Lane D: Verifier and record format hardening
**Wave:** 1   **Depends on:** none (Task D.2 should merge first: it moves a shared file)   **Branch:** fix/verify
**Covers findings:** cr-format-verify-1, -2, -3, -4, -5 (not in the brief, same file as -9, planned), -6 (dropped), -7, -8, -9, -10, -11, -12, -13, arch-8
**Owns files:** crates/goalcup-verify/src/* except usage/hints.rs and usage/native.rs; crates/goalcup-format/src/*; their tests (crates/goalcup-verify/tests, crates/goalcup-format/tests); docs/RECORD-FORMAT.md; new `connectors/shapes.tsv` (moved, see D.2)
**Shared files (only these parts):**
- SHARED EDIT 1: `git mv crates/goalcup/src/mask/shapes.tsv connectors/shapes.tsv`; then crates/goalcup/src/mask/rules.rs:19 `include_str!("shapes.tsv")` becomes `include_str!("../../../../connectors/shapes.tsv")`; crates/goalcup/tests/mask_corpus/common.rs:31 `include_str!("../../src/mask/shapes.tsv")` becomes `include_str!("../../../../connectors/shapes.tsv")`; the comment at crates/goalcup/tests/mask_rules.rs:3 names the new path. Docs lane: docs/ARCHITECTURE.md:153 path.
- SHARED EDIT 2: crates/goalcup/src/mask/rules.rs:207-256 (`glued`, `ends_an_escape`) are deleted and replaced by `pub(crate) use goalcup_format::shared::glued;`; `GUARD_BYTES` (rules.rs:52) stays, set to `goalcup_format::shared::GUARD_BYTES`.
- SHARED EDIT 3: crates/goalcup/src/connector/hosts.rs:236-250 (`matches`) is deleted and replaced by `pub(crate) use goalcup_format::shared::host_matches as matches;`.
**Produces (for other lanes):** `goalcup_format::shared::{GUARD_BYTES: usize, glued(before: &[u8]) -> bool, host_matches(pattern: &str, name: &str) -> bool}`; path `connectors/shapes.tsv`; check ids `record_json`, `wall_sane` in `integrity::IDS`; `secrets` check present in every `check --json` entry. Lane E: the `reader(connector, path)` signature in usage/hints.rs stays as it is.
**Consumes:** lane E's `[usage]` table only inside hints.rs and native.rs; nothing from this lane's code.

## Dropped
- cr-format-verify-6: reader choice by harness version lives in usage/hints.rs and usage/native.rs (lane E) and the version rows (lane I). The version is already read (usage/mod.rs:368) and reported (usage/out.rs:115 `harness_version`). Under the owner decision (never refuse by version) the verifier adds nothing here.
- arch-8, the version half: lane I. This lane takes only the shared-data half (shapes.tsv, host list, masker constants).

## Review focus
1. A LINK or GAP with wall times near u64::MAX or far in the past: `check` finishes in under 2 s with at most 100000 timeline rows (D.1).
2. A tunnel to `api.openai.com` the record calls `not_listed`, or `sign_in` with 500 MiB moved (D.3).
3. A `check` on a clean entry holding a string shaped like a key: exit and text must show it (D.4).
4. A 64 MiB SSE `data:` stream without a newline, and a stream of 1M one-byte lines: scan stays linear and memory bounded (D.4).
5. A part file with no `.idx` is never deleted; a recovery after a failed write leaves files mode 0600 (D.9).

### Task D.1: Timeline bucket cap and wall/mono check (cr-format-verify-4, top priority)   (size S)
**Files:** Modify crates/goalcup-verify/src/timeline.rs:10-16 (add const), :107-125 (`finish`); crates/goalcup-verify/src/integrity.rs:67 (`IDS`), :620 (LINK arm). Test crates/goalcup-verify/tests/check_timeline.rs.
**Interfaces:** Produces `const MAX_BUCKETS: u64 = 100_000;` in timeline.rs and check id `wall_sane`.
- [ ] Step 1: Write the failing tests in check_timeline.rs. `forged_link_wall_does_not_blow_up_the_timeline`: build `Entry::new("tl-forge")`, `standard()`, `crash("killed", 60_000)`, `exchange(Complete)`, `count()`, `seal()`; `mutate` the first `Record::Link` after the recorder_down GAP to `wall_ns = u64::MAX`; run `check`; assert `rows(&v).len() <= 100_000`, the call returns inside `std::time::Instant` 5 s, and `result(&v,"wall_sane") == "named"`. Second test `a_link_wall_a_day_off_the_mono_clock_is_fine`: same entry, Link wall +86_400e9 ns; assert `result(&v,"wall_sane") == "pass"`.
- [ ] Step 2: Run `cargo nextest run -p goalcup-verify --test check_timeline forged_link` ; expect FAIL (the run times out or allocates; `wall_sane` is missing from the report).
- [ ] Step 3: Implement. In `Tally::finish` set `self.last = self.last.min(first.saturating_add(MAX_BUCKETS * BUCKET))` before `end` is computed, so the span loop and the tail bucket stay inside the cap. In integrity.rs add `wall_sane` to `IDS` and, in the LINK arm, raise Named "a LINK wall time is more than 400 days from its PART's wall plus mono delta" when `|(wall_ns - part_wall) - (mono_ns - part_mono)| > 400 days` in i128. Named, not Fail: a clock step after boot is legal.
- [ ] Step 4: Run `cargo nextest run -p goalcup-verify --test check_timeline` ; expect PASS.
- [ ] Step 5: Commit `git add crates/goalcup-verify; git commit -m "fix(verify): cap timeline buckets, check wall against mono"`.

### Task D.2: One home for shapes.tsv, glued and host_matches (arch-8, cr-format-verify-1 prerequisite)   (size M, merge first)
**Files:** Create crates/goalcup-format/src/shared.rs; Modify crates/goalcup-format/src/lib.rs (add `pub mod shared;`), crates/goalcup-verify/src/secrets.rs:22-33 and :267-268; the three SHARED EDITs above. Test crates/goalcup-format/tests/shared.rs (new).
**Interfaces:** Produces `goalcup_format::shared::{GUARD_BYTES, glued, host_matches}` with the bodies moved unchanged from mask/rules.rs:207-256 and connector/hosts.rs:236-250.
- [ ] Step 1: Write the failing test file crates/goalcup-format/tests/shared.rs: `glued(b"abc") == true`, `glued(b"") == false`, `glued(b"x\\u0041") == false` (escape), `glued(b"%41") == false`, `glued(b"\x1b[31m") == false`, `host_matches("*.openai.azure.com","a.openai.azure.com") == true`, `host_matches("*.openai.azure.com","openai.azure.com") == false`, `host_matches("bedrock-runtime.*.amazonaws.com","bedrock-runtime.us-east-1.amazonaws.com") == true`.
- [ ] Step 2: Run `cargo nextest run -p goalcup-format --test shared` ; expect FAIL (no module `shared`).
- [ ] Step 3: Create shared.rs with the moved bodies. Apply SHARED EDITs 1 to 3. In secrets.rs delete `ESCAPE`, `GUARD` and the `LazyLock` import if unused, change :267-268 to `let glued = *guard && shared::glued(&buf[..s]);` (the masker's own rule, so the two cannot drift), and `SHAPES` to `include_str!("../../../../connectors/shapes.tsv")`.
- [ ] Step 4: Run `cargo nextest run -p goalcup-format --test shared`, then `cargo nextest run -p goalcup-verify --test secrets_scan`, then `cargo nextest run -p goalcup --test mask_rules --test mask_corpus --test connector_builtin` ; expect PASS (the masker corpus is the proof the move changed nothing).
- [ ] Step 5: Commit `git add connectors/shapes.tsv crates; git commit -m "refactor: one home for shapes.tsv, glued and host_matches"`.

### Task D.3: Honesty reads the shared host list (cr-format-verify-1)   (size M)
**Files:** Create crates/goalcup-verify/src/hosts.rs; Modify crates/goalcup-verify/src/lib.rs, Cargo.toml (`toml.workspace = true`), honesty.rs:247-295 (`open`), :374-395 (`tunnel_end`), :104 area (`start` event reading). Test crates/goalcup-verify/tests/honesty_expected.rs.
**Interfaces:** Produces in hosts.rs `pub fn sha256() -> &'static str`, `pub fn kind(host: &str) -> Option<&'static str>` ("provider", "gateway", "vendor"), `pub fn never(host: &str) -> bool`, all over `include_str!("../../../connectors/model-hosts.toml")` parsed once in a `LazyLock`, matching with `shared::host_matches`.
- [ ] Step 1: Write failing tests in honesty_expected.rs. (a) `a_tunnel_to_a_provider_labelled_not_listed_fails`: a conn with meta `{"host":"api.openai.com","mode":"tunnel","reason":"tls_refused","listed":null}`, END `up:10,down:10`, no GAP; expect `result(&v,"honesty_listed")` equals `"fail"` (use the check id the existing tests use for listed hosts). (b) `a_sign_in_tunnel_to_a_provider_is_judged`: same host with `reason:"sign_in"`; expect `fail`. (c) `a_sign_in_tunnel_over_1_mib_is_named`: host `auth.openai.com`, reason sign_in, END `up:600000,down:600000`; expect `named`. (d) `a_sign_in_tunnel_to_a_never_host_is_still_free`: same host, `up:1000,down:1000`; expect `pass`. (e) `a_different_host_list_hash_is_a_visible_warn`: start event `hosts_sha256` set to 64 `a`; expect the listed check `warn` and the text output contains `host list`.
- [ ] Step 2: Run `cargo nextest run -p goalcup-verify --test honesty_expected tunnel` ; expect FAIL on (a), (b), (c), (e).
- [ ] Step 3: Implement. In `open` (Conn arm) set `listed |= hosts::kind(host).is_some_and(|k| k != "vendor")`, `model |= same`; derive `host_kind` from `hosts::kind` when the record has none. In `tunnel_end` replace `c.reason == "sign_in"` with `c.reason == "sign_in" && hosts::never(&c.host)`; a sign-in tunnel with `up + down > 1 MiB` raises Named. In `event` for `start`, compare `hosts_sha256` with `hosts::sha256()`; on a difference raise Warn on the listed check with why "the record's host list differs from this verifier's"; the build's list is added to, never removed from, the judgement.
- [ ] Step 4: Run `cargo nextest run -p goalcup-verify --test honesty_expected --test check_conn_bytes` ; expect PASS.
- [ ] Step 5: Commit `git add crates/goalcup-verify; git commit -m "fix(verify): judge listed hosts by model-hosts.toml, not the record's label"`.

### Task D.4: secrets runs in every check; scan stays linear (cr-format-verify-2, -5, -9)   (size M, PARALLEL-OK with D.3)
**Files:** Modify crates/goalcup-verify/src/report.rs:20-40 and :40 (`known` becomes `&Rules`), main.rs:127-150, secrets.rs:359-372 (SSE loop), :402 (`off + len`), :510-520 (`check`). Test crates/goalcup-verify/tests/secrets_scan.rs.
**Interfaces:** `report::build(input, now_ns, local, known: &Rules, files)`; `secrets::check(r: &Rules, e: &Entry, known_given: bool) -> Check`.
- [ ] Step 1: Write failing tests. (a) `plain_check_reports_a_secrets_check_and_fails_on_a_shape`: an entry holding a body with `sk-ant-api03-` plus 90 `A`; `check(&dir, &[])` returns exit 2 and `result(&v,"secrets") == "fail"`. (b) `plain_check_says_no_known_values_were_given`: clean entry; `item(&v,"secrets")["why"]` contains `shapes only`; result `pass`. (c) `one_big_sse_line_scans_in_linear_time`: a 32 MiB `data:` payload with no newline over `entry(..., true, ...)`; finishes in under 10 s and is not `fail`. (d) `a_million_tiny_lines_finish`: 1_000_000 `data:x\n` lines in 64 KiB DATA pieces; under 10 s. (e) `a_mask_with_off_near_u64_max_does_not_panic`: Record::Mask `off: u64::MAX - 1, len: 5`.
- [ ] Step 2: Run `cargo nextest run -p goalcup-verify --test secrets_scan plain_check` ; expect FAIL (no `secrets` entry).
- [ ] Step 3: Implement. main.rs passes `k.unwrap_or_else(Rules::shapes)`; `secrets::check` appends " (shapes only; no --known values)" to `why` when `!known_given`. Replace the per-line `drain` in the SSE branch with a cursor over `s.line` that drains once per DATA after the last newline; `Stream.line` is truncated to its last 1 MiB (a longer line is dropped with a Named `secrets` why note "an SSE line over 1 MiB was not parsed"). Replace `off + len` and `m.1 + len` with `saturating_add`.
- [ ] Step 4: Run `cargo nextest run -p goalcup-verify --test secrets_scan --test check_run1_shape` ; expect PASS.
- [ ] Step 5: Commit `git add crates/goalcup-verify; git commit -m "fix(verify): secrets check always runs, SSE scan linear"`.

### Task D.5: Loss-indicating warns become named and visible (cr-format-verify-3)   (size S, after D.3 and D.4)
**Files:** Modify crates/goalcup-verify/src/honesty.rs:377 (refusal after trust), :419 (conn_bytes warn), history.rs:716 ("text not found in any response"), report.rs:136 (text filter). Test crates/goalcup-verify/tests/check_conn_bytes.rs, check_history.rs.
- [ ] Step 1: Write failing tests. `conn_bytes_between_10_and_50_percent_unaccounted_is_named`: existing fixture in check_conn_bytes.rs with 20% unaccounted; assert `result == "named"` and exit 1 (currently `warn`, exit 0). `history_text_missing_is_named` in check_history.rs, same pattern. `a_warn_is_printed_in_the_text_form`: a `gap_reason_unknown` entry; stderr (from `run`) contains `gap_reason_unknown warn`.
- [ ] Step 2: Run `cargo nextest run -p goalcup-verify --test check_conn_bytes --test check_history` ; expect FAIL (result is `warn`).
- [ ] Step 3: Implement. The three sites raise `Res::Named`. report.rs:136 already prints `>= Res::Named`; change it to `>= Res::Warn`. The mask-over-1-MiB warn (honesty.rs:236), `gap_reason_unknown`, `record_kind_unknown` and history "from before the record" stay `warn`: they lose nothing. State this split in docs/RECORD-FORMAT.md (D.10).
- [ ] Step 4: Run `cargo nextest run -p goalcup-verify` ; expect PASS after updating every test that asserted the old `warn` (grep `"warn"` in crates/goalcup-verify/tests).
- [ ] Step 5: Commit `git add crates/goalcup-verify; git commit -m "fix(verify): loss-indicating warns are named and printed"`.

### Task D.6: Unparseable record JSON is a finding (cr-format-verify-7)   (size S, PARALLEL-OK)
**Files:** Modify crates/goalcup-verify/src/integrity.rs:67 (`IDS`), :406-411 (EVENT), :546, :657 (OPEN meta), END tail arm; honesty.rs:104 (`obj`). Test crates/goalcup-verify/tests/check_invariants.rs (FAILS table).
- [ ] Step 1: Write the failing test: add `("record_json", m_event_not_json)` and `("record_json", m_end_tail_not_json)` to `FAILS` (check_invariants.rs:222), builders that put `b"{not json"` in an EVENT and an END tail. A tail of `null` or empty bytes (what `Value::Null` writes) must stay `pass`: add `a_null_tail_is_fine`.
- [ ] Step 2: Run `cargo nextest run -p goalcup-verify --test check_invariants record_json` ; expect FAIL.
- [ ] Step 3: Implement one helper in integrity.rs `fn json_ok(b: &[u8]) -> bool` (empty, or parses as any `Value`); call it once per OPEN, GAP, END, EVENT at the top of `item` and `self.fail("record_json", "a record's JSON does not parse", at)`. Add `record_json` to `IDS`.
- [ ] Step 4: Run `cargo nextest run -p goalcup-verify --test check_invariants` ; expect PASS.
- [ ] Step 5: Commit `git add crates/goalcup-verify; git commit -m "fix(verify): record_json check for unparseable meta, tail and event JSON"`.

### Task D.7: wire_keeps_up cannot be masked by early wire-only calls (cr-format-verify-8)   (size M, PARALLEL-OK)
**Files:** Modify crates/goalcup-verify/src/legs.rs:219-242 (`Sweep::new`), :290 (use). Test crates/goalcup-verify/tests/check_legs.rs.
- [ ] Step 1: Write the failing test `early_wire_only_calls_do_not_hide_later_lost_calls`: harness times h = 5 calls at minutes 30..34; wire times w = 20 calls at minutes 0..4 (title and count_tokens style) and none later. Assert `wire_keeps_up` result is not `pass` (today the cumulative count reads 20 wire against 5 harness). Second test `a_clean_run_with_a_few_wire_only_calls_passes`: 3 wire-only calls at minute 0 plus matching pairs; `pass`.
- [ ] Step 2: Run `cargo nextest run -p goalcup-verify --test check_legs early_wire_only` ; expect FAIL (result `pass`).
- [ ] Step 3: Implement a window pass beside the cumulative sweep: for each 10 minute window (`10 * MIN`) count harness calls `hw` and wire calls `ww` (wire times shifted back by `LAG`); a window with `hw > ww + tol(hw)` marks `first/worst/at` as `Sweep` does now. The cumulative sweep stays as the second condition, so the result is the worse of the two.
- [ ] Step 4: Run `cargo nextest run -p goalcup-verify --test check_legs --test check_run1_shape` ; expect PASS.
- [ ] Step 5: Commit `git add crates/goalcup-verify; git commit -m "fix(verify): wire_keeps_up compares per window, not only cumulative"`.

### Task D.8: Hermes DB copy opened defensively (cr-format-verify-10)   (size S, PARALLEL-OK)
**Files:** Modify crates/goalcup-verify/src/usage/sqlite.rs:270-290. Test crates/goalcup-verify/tests/check_thread_db.rs.
- [ ] Step 1: Write the failing test `a_database_with_a_trigger_or_view_function_is_not_run`: a SQLite file (built with rusqlite in the test) holding a view that calls `load_extension` and a trigger; `usage` on it must return and the reader row count must equal the table's, with no side file created in `std::env::temp_dir()` after the call (list the dir before and after). Second: `the_copy_dir_is_unpredictable`: two calls get different directory names that contain 16 hex of randomness (read through a `#[cfg(test)] fn last_dir()`).
- [ ] Step 2: Run `cargo nextest run -p goalcup-verify --test check_thread_db a_database_with` ; expect FAIL (no random name).
- [ ] Step 3: Implement. Name the dir with 8 random bytes from `getrandom`-free `std::fs::File::open("/dev/urandom")` on unix (fall back to the nanosecond clock); open with `SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_NO_MUTEX`; run `PRAGMA trusted_schema=OFF` before `query_only`; `DbConfig::SQLITE_DBCONFIG_DEFENSIVE` set true. Keep the per-process copy (a WAL needs its own directory).
- [ ] Step 4: Run `cargo nextest run -p goalcup-verify --test check_thread_db --test usage_run1` ; expect PASS.
- [ ] Step 5: Commit `git add crates/goalcup-verify/src/usage/sqlite.rs crates/goalcup-verify/tests; git commit -m "fix(verify): open the Hermes DB copy defensively"`.

### Task D.9: PartWriter file modes; recover quarantines (cr-format-verify-11, -13)   (size S, PARALLEL-OK)
**Files:** Modify crates/goalcup-format/src/part.rs:112 (`create_dir_all`), :289-294 (`create`), :343-349 (delete). Test crates/goalcup-format/tests/recovery.rs.
- [ ] Step 1: Write failing tests. `part_and_index_files_are_private` (unix only, `#[cfg(unix)]`): after `PartWriter::open` in a fresh dir, `mode & 0o077 == 0` for dir, part file and idx. `a_part_file_without_an_index_is_set_aside_not_deleted`: delete the `.idx` of the only part file, call `recover`, assert the original file now exists as `<name>.gcr.zst.orphan` and `part_files(dir)` does not list it. Linux and macOS behave the same.
- [ ] Step 2: Run `cargo nextest run -p goalcup-format --test recovery private_ orphan` ; expect FAIL (mode 0644; file gone).
- [ ] Step 3: Implement. `OpenOptionsExt::mode(0o600)` on the two `create` calls; `DirBuilder::new().recursive(true).mode(0o700)` for the dir (existing dir untouched). Replace `fs::remove_file(p)` with `fs::rename(p, p.with_extension("zst.orphan"))` (index copy stays beside it).
- [ ] Step 4: Run `cargo nextest run -p goalcup-format` ; expect PASS (adjust `a_boot_with_no_index_line_is_deleted...` to check the orphan name).
- [ ] Step 5: Commit `git add crates/goalcup-format; git commit -m "fix(format): private part files, set aside part files without an index"`.

### Task D.10: Mutation and fuzz tests; format doc (cr-format-verify-12)   (size M, after D.1 to D.9)
**Files:** Modify crates/goalcup-verify/tests/check_invariants.rs (FAILS table), create crates/goalcup-verify/tests/fuzz_bytes.rs; Modify docs/RECORD-FORMAT.md:224-246 (invariants), :338-452 (report), :452-490 (secrets), :460 (path of shapes.tsv).
- [ ] Step 1: Write the failing tests. `m_part_prev_wrong`, `m_part_raw_offset_wrong`, `m_part_number_skips`, `m_part_entry_name_changes` (each mutates the second PART's field) with the existing `inv2` or the matching check id as expectation; `m_frame_sealed_then_open` (a frame with a seal then more bytes) expects `torn_tail` or `inv8`. fuzz_bytes.rs: 200 seeds (xorshift as recovery.rs does) of random bytes and of a valid part file with one random byte flipped, written as `rec/rec-000...gcr.zst`; `run(["check", dir])` must return exit 2 or 3, never 0 and never a signal, within 10 s each.
- [ ] Step 2: Run `cargo nextest run -p goalcup-verify --test check_invariants --test fuzz_bytes` ; expect FAIL where a mutation is not caught; fix the verifier at the exact check each failure names, one commit per catch (do not add speculative checks).
- [ ] Step 3: Update the doc: new ids `record_json`, `wall_sane`; `secrets` is in every entry; the warn/named split of D.5; the new path `connectors/shapes.tsv`; `part files without an .idx are renamed .orphan`.
- [ ] Step 4: Verify the doc: `grep -n "mask/shapes.tsv" docs/RECORD-FORMAT.md` returns nothing; `grep -c "record_json" docs/RECORD-FORMAT.md` is at least 1; run `cargo nextest run -p goalcup-verify -p goalcup-format` and expect PASS.
- [ ] Step 5: Commit `git add crates docs/RECORD-FORMAT.md; git commit -m "test(verify): part continuity mutations, byte fuzz; document new checks"`.
