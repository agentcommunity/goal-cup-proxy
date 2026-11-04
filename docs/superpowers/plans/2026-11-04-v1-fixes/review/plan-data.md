# plan-data: multi-version harness support, data first

Angle: versions are data in the connector. Make the smallest schema change that lets any installed version run and be recorded honestly. Cut more than we add.
Checkout read: /Users/team/dev/worktrees/goal-cup-tracer/mac-test-v1.0 (read-only). Paths below are relative to it.

## 1. Problem today (cited)

1. **Five encodings, no single source** (arch-6, arch-8, versions-11):
   - (a) The connector `version` key is a prefix that refuses a mismatch (crates/goalcup/src/connector/schema.rs:19-20, connector/mod.rs:237-248). Only opencode-cli ("1.") and opencode2-cli ("2.") use it.
   - (b) `version_notes` is exact text that only warns (schema.rs:21-23, mod.rs:251-263). Only hermes-cli uses it (connectors/hermes-cli.toml:42-44).
   - (c) connectors/tiers.toml holds one `version` string per id. It is compiled in at cli/start.rs:26 and is still the seed today: every harness "not verified".
   - (d) scripts/pad/harnesses/<id>.toml `version` is the pad pin. Amp's pin, 0.0.1793664053-ga6248e, differs from its connector header.
   - (e) A header comment in each connector names the version it was checked against ("# Hermes Agent v0.22.5"). It is not data.
2. **Five parsers that disagree** (versions-2): mod.rs:237-240 (prefix, strips v), mod.rs:251-257 (exact, strips v), start.rs:442 and setup.rs:128 (first word that starts with a digit, no v-strip, keeps the trailing "."), scripts/pad/pad-run:314 (regex), and the raw line kept by the verifier and the server. Results:
   - Hermes "v0.22.6" and OpenCode "v2.2.2" parse to None, so the tier line says "verified" for any build (versions-1).
   - Copilot prints "1.0.105." and the line reads "verified on 1.0.105, not on 1.0.105." (text.rs:1161).
3. **No per-version behaviour** (versions-4). Every behavioural key is single-valued (schema.rs:24-48). A breaking release means cloning the whole connector: opencode2-cli differs from opencode-cli in about 6 keys (diff: `version`, args `--standalone`, host_hints key paths, secret_files, no_prompts, first_screen). Each clone also needs arms in goalcup-verify usage/hints.rs:68-73, a pad file, a tiers row and a docs sheet. `goalcup setup` lists both clones when `opencode` is on PATH (setup.rs:250-255).
4. **Known-bad is text only, and the pad tests a config participants do not have** (versions-5, versions-x2).
   - Hermes 0.22.6 gets a note saying "Use 0.22.5".
   - The pad passes on 0.22.6 only because of `providers.goalcup-ca.ssl_ca_cert` in scripts/pad/harnesses/direct/hermes-config.yaml:11-17.
   - Every committed hermes row is 0.22.6, and hermes is CORE (scripts/pad/table:16).
5. **Unread version means no rules at all** (versions-6, cr-watchdog-5). launch.rs:146-171 gives a 5 s probe, success exit only. Hermes on a fresh home exits 1, so its note never fires. An OpenCode pin is skipped silently (`(Some, Some)` only, mod.rs:241).
6. **Verified means one exact build on one OS** (versions-3, versions-x3). scripts/pad/table:82-88 keeps only the latest case rows and needs them all on one version. `os` is ignored. Nothing checks that tiers.toml matches the committed rows (versions-x1, GUARDED excludes it, table:17-18).
7. **Promise not kept**: docs/RULES.md:207-209 says the version is sampled and a change recorded as an event. The code reads it only at start/continue. continue_run.rs:282-288 prints a line but writes no event (docs-2).
8. **Docs contradict the code**: CONNECTOR-SCHEMA.md:7 says "Nothing is pinned", but OpenCode hard-refuses (docs-1). No doc says how a version is chosen, recorded and verified (docs-x1).

## 2. Design (one paragraph)

goalcup reads `program --version` once with the base env, as it does today. One parser turns that line into a version. The connector's ordered `[[versions]]` rows are tested against it, and the first match is applied as an overlay on the base connector. A row may also say `status = "broken"` with a `say`. With no match, or an unreadable version, the base connector runs: the version is unknown or untested, the run is recorded the same way, and the participant is told so in one line. The applied row's `match` and the parsed version go into the start event, so the server and verifier can see which rules recorded the part. Tiers become a list of (version, os) pairs that the pad verified, written from the committed rows and checked at release. Everything else that names a version (header comments, the pad pin, the `version`/`version_notes` keys, the opencode2 clone) is derived or cut.

The principles:
- **Never refuse an unknown version.** That is point 7: the version people actually installed. Only a row that someone measured as broken may refuse.
- **Overlay, not inheritance.** There is one base plus flat rows. Rows do not chain and do not include each other.
- **Reads `--version` exactly once per start/continue.** No new probes during the run. The only extras are a re-read at seal and at continue, which already exist or cost nothing.

## 3. Schema changes (connectors/*.toml)

### 3.1 New keys
```toml
checked = "2.2.2"            # the version this file was written and read against (replaces the header comment and the pad pin)

[[versions]]                 # ordered; the first row whose `match` holds is applied
match  = "1.x"               # grammar below
say    = "..."               # optional: shown at start/continue and recorded (replaces version_notes)
status = "ok"                # "ok" (default) | "broken": broken refuses the start with `say`
# Override keys, the same names and shapes as the base:
args = [...]; resume = [...]; env = {...}; unset = [...]
host_hints = [...]; secret_files = [...]; no_prompts = [...]; update_off = [...]
native = [...]; first_screen = "..."; before = [...]; after = [...]
```

### 3.2 Overlay rule
Two lines, and the doc states them:
- A list or string key replaces the base value.
- A map key (`env`) merges over the base, key by key.

These keys cannot be overridden, and the loader refuses them inside a row:
- `id`, `name`, `program`: identity.
- `home`, `ca_env`: the probe already ran with them, so changing them after the read would make the probe a lie.
- `mask`, `refuse_if_set`: secrets rules stay version-independent (point 4), so a row can never weaken masking.

### 3.3 `match` grammar
Smallest that covers the 16 connectors:
- `"0.22.6"` is exact.
- `"1.x"` or `"0.22.x"` is a prefix on whole components.
- `">=0.22.6"`, `"<2"` and `">=1.19, <2"` are comparisons, joined by a comma for AND.
- `match` may be a list for OR.
- Comparison is numeric per dot component with u64. That holds Amp's 0.0.1793664053 and openclaw's 2026.10.9.
- A pre-release or build suffix (`-ga6248e`, `+x`) is ignored for ordering and kept for display.

About 60 lines of Rust plus a table test. No semver crate: it rejects a leading v, two-part versions ("2", "1.0") and trailing punctuation, and it adds a dependency.

### 3.4 Removed keys
- `version` (prefix refusal) and `version_notes`.
- The loader refuses both with a pointer to `[[versions]]`. That is fine because connectors are compiled in, and an outside `--connector` file is non-competition only.

### 3.5 The one parser: `connector::version::parse(line) -> Option<Version>`
- It takes the first whitespace word matching `^v?\d+(\.\d+)*([-+][0-9A-Za-z.-]+)?`, after stripping trailing `.,;:)` and a leading `(`.
- It is used by the connector match, the tier line, setup, status and continue. All five parsers collapse into this one.
- Its test table holds every connector's real `--version` line, collected from MAC-LEARNINGS, docs/harnesses/*.md and the pad rows' raw harness_version:
  - "Hermes Agent v0.22.6 (2026.9.24)"
  - "GitHub Copilot CLI 1.0.105."
  - "opencode v2.2.2"
  - "2.1.312 (Claude Code)"
  - Amp's 0.0.N-gHASH

### 3.6 tiers.toml (generated, still compiled in)
```toml
[claude-code-cli]
tier = "verified"            # best tier over the list below; kept so text.rs stays a 3-arm match
verified = [ { version = "2.1.312", os = "darwin-arm64" }, { version = "2.1.316", os = "darwin-arm64" } ]
reason = ""
```
The tier line logic, in text.rs:1158 (same 3 arms, one more clause):
- On the list for this os: "verified with goalcup on V (this OS)."
- Same version, another OS: "verified on V on macOS, not yet on Linux."
- In range of the applied row, but not on the list: "X V: not verified on this build; verified on V1, V2. goalcup records it the same way."
- Unknown version: today's "not verified" line.

No "tested range" maths, so there is nothing new to keep true.

## 4. CLI / record / server changes

- **goalcup start / continue**: one new line when a row applied with `say`, which is today's version_notes line. One line when the version is unread and the connector has a `broken` row: "X's version could not be read, so goalcup could not check it against the versions known not to work: <list>." That fixes cr-watchdog-5 in words without refusing. There are no new flags. Keep `--connector` (non-competition) for testing an edited row.
- **goalcup setup**: show `name parsed-version (rules: <match> | base)`. With opencode folded, it lists one OpenCode.
- **goalcup status**: show the applied row next to version_note (status.rs:186 already shows notes).
- **Start event** (pipeline/boot.rs:251): add `"harness_semver": "2.2.2"` (parsed or null) and `"version_row": "1.x"` (the match text, or null for the base). Keep the raw `harness_version`. GCR1 records are JSON, so the change is additive and old readers ignore it. No format version bump.
- **Seal and continue re-read**: at seal (after the harness exits, so Hermes has its deps) and at continue, read `--version` again. If it differs from the start event, or was unread, write `{"ev":"note","what":"version_read","harness_version":...,"harness_semver":...}`. That keeps docs/RULES.md:207-209 honestly, at part granularity. Change the RULES text to "read at each start, continue and seal; a change is recorded". No sampling thread.
- **Server**: no schema change. POST /v1/runs keeps `harness_version` (runs.ts:144). The new fields ride in the uploaded records. Optional later: `harness_semver` column for operator queries.
- **goalcup-verify**: print harness_semver and version_row in usage output (usage/mod.rs:368 and out.rs:115 already carry harness_version). When a native reader yields NoNative, add "harness V, connector rows for <match>" to the reason (legs.rs:18). Readers stay keyed by connector id. Making them version-keyed is out of scope (see section 5).

## 5. What to cut

1. **opencode2-cli as a connector.** Fold it into opencode-cli: base = OpenCode 2 (what a new install gets), plus `[[versions]] match = "1.x"` carrying the 1.x host_hints, secret_files, no_prompts and args = []. 16 files become 15. This also removes:
   - its builtin.rs line (builtin.rs:22);
   - its tiers row;
   - its pad file, which becomes a `[versions."1.19.5"]` case in opencode-cli's pad file or a second pin;
   - the hints.rs arm (hints.rs:73), whose WIRE_IDS are identical;
   - the duplicate setup listing (versions-8, usability-11).
   - Timing: after the event, see section 8.
2. **`version` and `version_notes`** keys, the VersionMismatch refusal text (text.rs:179-182) and the `held`/`exact` closures (mod.rs:237-263). These are replaced by the parser and the `match` check.
3. **Header comments naming a version** in all 16 connectors. Each becomes `checked = "..."`.
4. **The pad's own version regex** (scripts/pad/pad-run:312-319) and the copilot `sed` (scripts/pad/harnesses/copilot-cli.toml:16). The pad reads harness_semver from the run's start event, which it already parses for its checks, so goalcup's parser is the only one.
5. **The pad `version` pin key.** The pad installs `checked` from the connector unless a `--version` override is given. `install` becomes a template with `{version}`, and `version_cmd` stays only for the OpenCode link dance, or goes once OpenCode is one connector.
6. **The pad-only Hermes workaround** (hermes-config.yaml:11-17). It either moves into the connector as a 0.22.6 row or is deleted, so that the pad tests the config participants actually get.
7. **The tiers "latest row only, all one version" rule** (table:82-88). It becomes "every version with a full PASS set per os".

Net: probably fewer lines of Rust than today. The parser and overlay (about 120) against the cut of the closures, the refusal path and the clone's tests. The Python shrinks too.

## 6. Migration of the 16 connector files

There are 16 built-in files today (builtin.rs:8-25); the brief's "17" counts something else. After the fold there are 15.

| Connector | Change |
|---|---|
| amp, claude-code, cline, codex, copilot, crush, droid, gemini, goose, kilo, kimi-code, openclaw, qwen | Header comment version becomes `checked = "..."`. No rows. Use the pad's pinned version where it differs from the header. Amp: use 0.0.1793664053-ga6248e, the one the reader was written against, and fix docs/harnesses/README.md:26. |
| hermes | `checked = "0.22.5"`. Add `[[versions]] match = "0.22.6"`. Option A (preferred if measured): add the CA setting as a no_prompts-style row. Option B: `status = "broken"`, `say` = today's text. Measure A first (task T6). |
| opencode + opencode2 | Fold into opencode-cli. Base is 2.x, from opencode2-cli.toml. Add `[[versions]] match = "1.x"` holding opencode-cli.toml's differing keys. `checked = "2.2.2"`. The 1.x row can carry its own `say` naming 1.19.5 as checked. |

About Option A for Hermes. The pad fix sets `ssl_ca_cert` on a provider it defines itself. A participant's provider is their own, so a single settings row with a fixed provider name does not reach it, and write_row (launch.rs:186-230) has no `*` support. Before adding wildcard writes (more code), check whether Hermes 0.22.6 reads a CA env var that goalcup could add through the row's `env`, for example REQUESTS_CA_BUNDLE or HTTPX_CA_BUNDLE. If one works, Option A is a one-line `env` overlay. If none does, ship Option B: refuse 0.22.6 with the clear text rather than start a run that cannot reach its model. That is more honest than the current warn-then-fail.

## 7. Tests

- **Parser table test** (`connector/version.rs`): every real `--version` line above, plus edge cases: empty, no digits, "v", date-only openclaw, Amp suffix, trailing ".".
- **Match grammar test**: exact, `.x`, ranges, lists, suffix ordering, u64 overflow.
- **Overlay test**: for each built-in connector with rows, load with each row's example version and assert the merged keys. Also assert that the loader refuses an overlay of `program`, `home`, `ca_env`, `mask` or `refuse_if_set`.
- **Golden fold test**: `opencode-cli` with version "1.19.5" yields exactly the keys of today's opencode-cli.toml, and with "2.2.2" exactly opencode2-cli.toml's (except id and name). Run it once against the pre-fold files, then delete the old files. This proves the fold changes no behaviour.
- **tier_line tests** (text.rs): on the list, other OS, other version, unknown, v-prefixed, trailing dot. These cover versions-1, versions-2 and versions-x3.
- **Pad**:
  - `table --release-check` re-renders tiers.toml from the committed pad-rows.jsonl at the tag and diffs it against connectors/tiers.toml at the tag (versions-x1, about 10 lines of Python).
  - A row whose start event's version_row differs from what the connector would pick fails C-something. This catches a pad config the participant does not have (versions-x2).
- **Continue test**: a part recorded with harness_semver A, continued with B, writes a `version_read` note, and the end screen says so.

## 8. Rollout

### Before Friday 6 Nov (event start; 1.0 is live, a release test is running)
Only changes that cannot break a 168-hour run or `continue` across an upgrade. continue_run.rs:144-150 refuses when the stored connector id no longer loads to the same id, so **the opencode fold must wait until after the event**. Folding mid-event strands every opencode2-cli run at `continue`.

- **T1 parser** (S): one parse function used by tier_line, setup and the existing `version`/`version_notes` checks. This fixes versions-1/2 with no schema change.
- **T2 tiers render and release-check diff** (S): run `table`, commit real tiers, add the diff to `--release-check`. Hermes cannot be "verified" on 0.22.6 through a pad-only workaround, so either drop the pad block and re-run hermes on 0.22.5, or keep tiers "not verified" for hermes. Decide with T6.
- **T3 docs truth** (S): CONNECTOR-SCHEMA.md:7 states the OpenCode prefix refusal. RULES.md:207-209 says read at start and continue. Add a "Harness versions" section, the contract, to CONNECTOR-SCHEMA.md (docs-x1).
- **T4 unread-version line** (S): when the version is unread and the connector has `version` or `version_notes`, say which versions it could not check (cr-watchdog-5, versions-6).
- **T6 measure Hermes 0.22.6 CA env** (S, measurement only): it decides Option A or B.

Each is a goalcup 1.0.x patch. Participants on 1.0.0 continue working, and the record is unchanged.

### After the event (1.1)
- **T5**: `[[versions]]`, `checked`, `status`, the overlay and the new start-event fields.
- **T7**: the hermes row.
- **T8**: the opencode fold. Server-side runs from the event stay valid, because the old id is only needed by `continue`, and continue windows are closed by then.
- **T9**: the pad changes (derive the pin, drop its parser, matrix of (harness, version) cases).
- **T10**: version_read at seal/continue.
- **T11**: verifier output.

Later and optional: fetch a signed known-bad list from the server at setup/start, so that a Hermes-0.22.6-style break warns without a goalcup release (versions lane, arch-6). I recommend against it for now: it adds a network dependency to start (point 3), and a 1.0.x patch release with one connector row is cheap.

## 9. Risks

- **Probe uses the base env.** A row cannot change where `--version` looks. That is acceptable: no harness today needs a different home to print its version. The loader refuses `home` in rows to keep this true.
- **Wrong row on an unread version.** For OpenCode after the fold, an unread version runs base (2.x) rules on a 1.x binary. Today the same case runs the connector the participant picked, so this is no worse. The T4 line names it, and the start event's `version_row` = null makes it visible to the verifier.
- **First match wins, so overlapping rows depend on order.** The loader warns when two rows' matches overlap on any of the version lines in the test table. Document "most specific first".
- **The tier list grows forever.** table keeps the last N=5 versions per os. That is enough to say "verified on a nearby build".
- **Refusing a broken version.** This is against point 7 in spirit. Allow `status = "broken"` only with a cited issue in `say`, and only for a measured break where the harness cannot reach its model through goalcup (the run would be useless anyway). Review rule in CONTRIBUTING.
- **Readers stay keyed by connector id.** A transcript format change still shows as NoNative. The new reason text names the version, which makes it diagnosable but does not fix it. Version-keyed readers (a `reader = "codex-rollout-v2"` key in a row) is a later step if a real break appears. Do not build it speculatively.
- **The mid-run self-update gap remains** for Amp, Crush and Goose, which have no update-off switch (versions-9). The seal re-read records that it happened. It cannot prevent it.

## 10. Ordered task list

| # | Task | Size | When |
|---|---|---|---|
| T1 | One `version::parse`, table-tested on all real lines; used by tier_line, setup and both existing checks | S | before Fri |
| T2 | Render tiers.toml from committed rows; `--release-check` diffs it; settle hermes 0.22.6 rows | S | before Fri |
| T3 | Docs: CONNECTOR-SCHEMA "Harness versions" contract; fix CONNECTOR-SCHEMA:7, harnesses/README:77, RULES:207-209, Amp version drift | S | before Fri |
| T4 | Unread version + connector has version rules: plain line naming the unchecked versions | S | before Fri |
| T6 | Measure whether Hermes 0.22.6 honours a CA env var (decides the row's content) | S | before Fri |
| T5 | Schema: `checked`, `[[versions]]` (match/say/status + overlay keys); loader rules; remove `version`/`version_notes`; start event `harness_semver`, `version_row` | M | 1.1 |
| T7 | hermes-cli 0.22.6 row (env fix or broken); delete pad-only provider block | S | 1.1 |
| T8 | Fold opencode2-cli into opencode-cli with golden test; drop builtin line, hints arm, pad file, tiers row, docs sheet | M | 1.1 (after event) |
| T9 | Pad: install `checked` by default with `--version` override; read harness_semver from the record; drop pad-run regex and copilot sed; tiers as (version, os) list, last 5 | M | 1.1 |
| T10 | Re-read version at seal and continue; `version_read` note event; end screen line | S | 1.1 |
| T11 | goalcup-verify: show harness_semver/version_row; NoNative reason names the version | S | 1.1 |
| T12 | Replace 16 header comments with `checked` (mechanical, with T5) | S | 1.1 |

## 11. Where I differ from other plans
No other plan-*.md existed in the review folder when this was written (checked at write time). Against the versions lane's direction list (versions.md "Direction"):
- **Agree:** one parser (1), overlay rows (3), and folding opencode2 (3).
- **Differ on (2):** no `tested_range` field. "Verified" stays a list of facts from the pad. The applied row's `match` already says which rules cover a version, so a second range would be a sixth encoding.
- **Differ on (5):** no sampling during the run. A re-read at seal and at continue keeps the promise at part granularity with no new thread.
- **Differ on (6):** no nightly `@latest` lane before the event. It is a good post-event pad job, but it is infrastructure.
- **Differ on (7):** no `doctor` check that redirects took effect. That is real (versions-10) but is a separate plan. A cheap version is: warn_missing on native rows already says when a home redirect failed.
- **Differ from arch-6 and the versions lane on a remote compat file:** defer. A patch release is the update channel.
- **Timing, an addition the others may miss:** the opencode fold must wait until after the event, because continue_run.rs:144-150 refuses a stored connector id that no longer loads.
