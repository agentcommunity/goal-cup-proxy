# Multi-version harness support: merged plan

Judge's merge of plan-data.md (base), plan-assurance.md and plan-runtime.md, with versions.md as the map.
Checkout read: /Users/team/dev/worktrees/goal-cup-tracer/mac-test-v1.0 (= origin/main 70eecdfb + pad/plans). Plan only.
Paths below are relative to that checkout. Line refs re-checked by the judge on 3 Nov.

Scores (1-10: seven points / least code / Friday for top 3 / honest / testable = avg):
- plan-data      8 / 9 / 8 / 7 / 9 = 8.2  (base: smallest schema, right timing on the OpenCode fold; loses on `status="broken"` refusal)
- plan-assurance 9 / 6 / 7 / 10 / 8 = 8.0 (grafted: `pad_only` evidence rule, never refuse, end-of-run read, stat on upload tick, canary, runbook)
- plan-runtime   8 / 5 / 6 / 9 / 7 = 7.0  (grafted: probe reads stderr, unread-version line wording, effect checks idea; cut: mid-run exec probes, ranges)

Top 3 harnesses = CORE in scripts/pad/table:16: claude-code-cli, codex-cli, hermes-cli.

## 1. Today: the pinned state

- One version per harness, one OS, typed by hand.
  - connectors/tiers.toml is `{tier, version, reason}`, still the phase-0 seed: every row "not verified". It is compiled in (cli/start.rs:26).
  - scripts/pad/table:82-88 calls a harness verified only when all its case rows share one version. `os` is never read.
  - The release check does not compare tiers.toml to the rows: GUARDED excludes it (table:17-18).
- Six version parsers disagree:
  - connector `version` prefix with v-strip (connector/mod.rs:237-240);
  - `version_notes` exact match (mod.rs:251-257);
  - tier/setup "first digit-led word", with no v-strip and trailing punctuation kept (start.rs:442, setup.rs:127-130);
  - the pad regex (scripts/pad/pad-run:314) plus a copilot sed (scripts/pad/harnesses/copilot-cli.toml:16);
  - the raw line on the server (runs.ts:144) and in the verifier (usage/mod.rs:368).
  - Bugs this causes:
    - Hermes "v0.22.6" and OpenCode "v2.2.2" parse to None, so they always show "verified" (text.rs:1158-1167).
    - Copilot "1.0.105." shows "verified on 1.0.105, not on 1.0.105."
- Behaviour is single-valued per connector (connector/schema.rs:15-48).
  - A breaking release means cloning the whole connector, as opencode-cli/opencode2-cli did. That clone is the only user of the `version` refusal.
  - The refusal says "Install that one" (text.rs:179-182), which goes against RULES.md:207-209 "refuses no build".
  - `goalcup setup` lists both OpenCode connectors whenever `opencode` is on PATH (setup.rs:250-255).
- Known-bad is text only, and the pad tests a config participants do not have.
  - connectors/hermes-cli.toml:42-44 tells Hermes 0.22.6 users they cannot reach their model.
  - All 17 committed Hermes rows are 0.22.6, and they pass only because of a pad-only `providers.goalcup-ca.ssl_ca_cert` block (scripts/pad/harnesses/direct/hermes-config.yaml:11-16).
  - Hermes is CORE.
- The probe fails when it matters.
  - launch.rs:146-171 drops stderr, waits 5 s at most, and accepts a successful exit only.
  - Hermes on a fresh home exits 1 (docs/harnesses/hermes-cli.md:20), so its 0.22.6 note never fires on a first run.
- The RULES promise is not kept. RULES.md:207-209 says goalcup "samples the version, records a change as an event". The code reads the version once per start/continue (mod.rs:234), and continue prints `text::updated` with no event (continue_run.rs:282-288).
- Nothing reaches a shipped binary.
  - Connectors, notes and tiers are `include_str!`.
  - The open answer carries no notes: approve.rs:97-101 `Opened` is run/entry/deadline/token only.
- Evidence is thin.
  - pad-rows.jsonl has claude-code 2.1.312 (2 rows), codex 0.165.0 (1 row) and hermes 0.22.6 (17 rows).
  - It has no amp, cline or droid rows, and no Linux rows.

## 2. Goal

Any installed harness version runs and is recorded (points 1, 3, 7). The participant is told in one true line what goalcup knows about their exact build on their OS (point 6). "Verified" is computed from pad rows a participant could reproduce, never typed. One connector per program; a version picks the rules. A harness breaking during the event (7-15 Nov) is caught and told within hours without blocking any run.

## 3. Design

1. **One version word.**
   - `connector::version::parse(line) -> Option<Version>`, in about 60 lines with no semver crate.
   - It takes the first word matching `^v?\d+(\.\d+)*([-+][0-9A-Za-z.-]+)?` after stripping a leading `(` and trailing `.,;:)`.
   - Comparison is numeric per component in u64, so Amp's 0.0.1793664053 fits. The suffix is ignored for order and kept for display.
   - Every consumer uses this one function: connector match, notes, tier line, setup, status, continue and the verifier.
   - The pad gets a 15-line Python port. Both the Rust and the Python tests read one fixture of real `--version` lines.
   - The raw line is still recorded unchanged.
2. **Never refuse on version.** Unknown, untested, unread and known-broken builds all run and are recorded.
   - Broken builds get a `say` line and a gap note.
   - An organizer flags them; none is disqualified (RULES.md:208).
   - plan-data's `status = "broken"` refusal is cut.
3. **Overlay rows in the connector (1.1).**
   - Ordered `[[versions]]` rows with `match`; the first match wins.
   - A list or string key replaces the base value; the `env` map merges key by key.
   - The loader refuses `id`, `name`, `program`, `home`, `ca_env`, `mask` and `refuse_if_set` inside a row. The probe ran with `home`/`ca_env`, and masking never varies by version (point 4).
   - Unread version: the base applies and the line says so.
4. **Evidence, not claims.**
   - tiers is a list of (version, os) entries rendered by `table` from committed rows.
   - A pad row counts only if the harness ran with what goalcup gives participants. Pad config that goalcup does not apply is declared with `pad_only = "<why>"` and voids the row as evidence.
   - The release check re-renders tiers and fails on any diff.
5. **One honest line.** Each case below gets one line; none says "Install".
   - exact (version and OS on the list): "verified with goalcup on V (macOS)."
   - other build: "not verified on V2; verified on V1, V3. goalcup records it the same way; goalcup status says if model requests are missing."
   - other OS.
   - unread: "version could not be read; goalcup used the rules for <checked>."
   - broken row: its `say` plus "The run is recorded."
6. **Version changes are recorded without exec mid-run.**
   - A `stat` of the resolved program (size, mtime, inode) on the existing 5-minute upload tick catches a self-update and writes `harness_changed`.
   - One `--version` read at seal, after the harness has exited, writes a `version_read` note when the version differs from start or was unread. This is the Hermes fresh-home case.
   - No background `--version` exec: Hermes-style first-run installs make it unsafe.
7. **Server notes (owner go/no-go, open question 1).**
   - The open/continue answer may carry `notes`: up to 5 plain lines, length-capped, control characters stripped, chosen by the Worker from (connector, version word, os).
   - Notes never refuse.
   - This is the only channel that reaches 1.0.0 binaries during 7-15 Nov.

## 4. Changes

### Connector schema (connectors/*.toml, connector/schema.rs:15-48), 1.1
- Add `checked = "X"`. It replaces the header comment (e.g. claude-code-cli.toml:1, hermes-cli.toml:1).
- Add `[[versions]]`:
  - `match` grammar: exact `"0.22.6"`, prefix `"1.x"`, comparisons `">=1.19, <2"`. No OR lists: use two rows.
  - Optional `say` and `gap`.
  - Allowed override keys: args, resume, env, unset, host_hints, secret_files, no_prompts, update_off, native, first_screen, before, after.
- Remove `version`, `version_notes`, `Refusal::VersionMismatch`, text.rs:179-182, and the `held`/`exact` closures (mod.rs:237-263).
- Loader warns when two rows' matches overlap on any fixture line. Document "most specific first".

### Crates
- goalcup:
  - connector/version.rs (new): parser and match.
  - connector/mod.rs: overlay merge as a serde Value merge before the typed deserialize, which keeps `deny_unknown_fields`.
  - launch.rs: the probe also reads stderr and accepts a parseable first line on a non-zero exit.
  - text.rs: tier_line becomes table-driven.
  - start.rs and setup.rs drop their own parse.
- goalcup-verify:
  - Print `harness_semver`/`version_row`.
  - The NoNative reason names the harness version and the version the reader was checked on (a constant beside each reader).
  - Count lines of an unknown shape instead of dropping them (usage/native.rs:171-197).
  - Readers stay keyed by connector id until a real format break.
- server/worker: an optional `notes` in the open/continue answer from a small advisories JSON committed beside the Worker. It also stores `harness_version_word` beside the raw line.

### Record (GCR1 start event, pipeline/boot.rs:251; additive JSON, no format bump)
- Start event gains `harness_semver` (or null), `version_row` (the applied `match`, or null), `tier_said` and `tier_basis` (exact|other_build|other_os|unread).
- New events:
  - `harness_changed` {before, after} from the stat;
  - `note what=version_read` at seal and continue;
  - gap words `version_unread` and the row `gap`.
- Server: `notes_sent` is stored on the entry.
- Verifier: confirm unknown event types are ignored in timeline.rs and integrity.rs before shipping.

### Pad (scripts/pad)
- pad-run and table use the shared parser. The regex (pad-run:314) and the copilot sed go.
- `pad_only` key on files blocks; `table` skips those rows as evidence.
- `table` writes tiers as a (version, os) list, keeping the last 5 per os, and `--release-check` diffs it.
- Pad harness files (1.1): `version` becomes `versions = [...]`, `install` is templated with `{version}`, and `latest_cmd` is added for the canary.
- `scripts/pad/canary` (script only): reads `latest_cmd`, compares against `scripts/pad/latest.json`, and runs a micro row per new build.

### Docs
- CONNECTOR-SCHEMA.md gets a "Harness versions" section: how a version is read, matched, recorded and verified. Fix :7 "Nothing is pinned".
- RULES.md:207-209 states what the code does now: "read at start, continue and seal; a change is recorded", or the stat once it lands.
- Per CORE harness: the exact install command for the verified build, so a downgrade is one paste.
- Fix the Amp version drift (docs/harnesses/README.md:26 vs scripts/pad/harnesses/amp-cli.toml:9).
- OPERATING.md: the breaking-update runbook (section 6).

## 5. What we cut
- `status = "broken"` refusal (plan-data). Point 7 and RULES.md:208 say record it and tell the participant.
- Mid-run `--version` exec: plan-runtime's 60 s late probe, the 10-min re-probe and the `ToRecorder::Version` control message. The stat plus the seal read cover it with no exec and no new channel.
- `tested` ranges (plan-runtime) and the `band`/"near" verdict (plan-assurance). Both make range claims nobody tested; the list of verified builds says enough.
- A signed remote compat file and remote behaviour. Notes are text only; behaviour changes need a release.
- Version-keyed readers, until a real transcript break appears.
- An OpenCode alias. continue_run.rs:141-150 refuses when the loaded `connector.id` differs from the stored one, so the fold waits until every event continue window has closed (after 15 Nov), and no alias is kept.
- Header version comments, the pad `version` pin, and the hand-copied tier table in docs/harnesses/README.md.

## 6. Phases

### Now (Tue 3 to Wed 4 Nov): truth and one parser. Each change lands on main through the normal gate, never in the live test folder.
- Hermes truth: run full direct/gateway/dodger rows on 0.22.5 without the CA block. Mark the 0.22.6 rows `pad_only`.
- Also measure whether 0.22.6 honours a CA env var (REQUESTS_CA_BUNDLE, HTTPX_CA_BUNDLE, SSL_CERT_DIR). If one works, it becomes the connector fix.
- One parser plus the shared fixture.
- Tier line and setup fixed.

### Before Friday (by Thu 5 Nov EOD, release 6 Nov 12:00)
- Re-render tiers from rows and add the release diff. Add "(macOS)".
- Unread-version wording.
- The OpenCode mismatch names the sibling id instead of "Install that one".
- Probe reads stderr.
- Docs.
- Canary script and runbook.
- Server notes only if the owner says go and it is green by Wed EOD.

### During the event (7-15 Nov): binary frozen, run the process
- Run the canary every 2 h on the maintainer Mac, Amp hourly.
- Classify a new build within 2 h:
  - records fine: add the tuple via a Worker deploy;
  - wire fine, native reader drift: a note only, and re-verify after 15 Nov because GCR1 is raw;
  - model unreachable or a prompt blocks: a note with the exact downgrade command, plus the event page.
- Query affected entries by (connector, version word). Flag them; never disqualify.

### After 1.0 (1.1, after 15 Nov)
- Overlays.
- The OpenCode fold.
- Record fields.
- The stat and the seal read.
- Verifier output.
- The tiers list.
- Pad `versions` matrix with Linux rows for CORE.
- Redirect effect checks: fold warn_missing into one "did CLAUDE_CONFIG_DIR / CODEX_HOME / FACTORY_HOME_OVERRIDE take effect" check, with a gap note.

## 7. Ordered tasks

| # | Task | Size | When |
|---|------|------|------|
| 1 | Hermes 0.22.5 full rows without the CA block; `pad_only` on the 0.22.6 block; measure the CA env vars on 0.22.6 | M (pad time) | now |
| 2 | `connector/version.rs` parse + `version-lines.tsv` fixture (every connector, real lines incl. "Hermes Agent v0.22.6 (2026.9.24)", "opencode v2.2.2", "GitHub Copilot CLI 1.0.105.", "2.1.312 (Claude Code)", Amp suffix); used by tier_line, setup and the existing version/notes checks; Python port in pad-run/table on the same fixture | S | now |
| 3 | tier_line table test (exact, other build, other OS, unread, v-prefix, trailing dot); add `os` to the tier row and say "(macOS)"; no "Install" anywhere | S | before Fri |
| 4 | table honours `pad_only`; `--release-check` re-renders and diffs tiers.toml; regenerate tiers.toml from committed rows (claude, codex, and hermes only if task 1 is green) | S | before Fri |
| 5 | Probe: read stderr and accept a parseable first line on non-zero exit; acceptance = Hermes fresh home on the Mac pad | S | before Fri |
| 6 | VersionMismatch text names the sibling id (`--harness opencode2-cli`); unread line names the versions it could not check | S | before Fri |
| 7 | Docs: CONNECTOR-SCHEMA "Harness versions", RULES:207-209 truth, downgrade commands for the CORE harnesses, Amp drift | S | before Fri |
| 8 | `scripts/pad/canary` + `latest_cmd` for CORE + Amp; runbook in OPERATING.md | S | before Fri |
| 9 | Server `notes` in the open/continue answer + advisories JSON + client print (cap, strip, test) | S+S | before Fri, owner go/no-go |
| 10 | Start-event fields, `version_read` at seal/continue, `harness_changed` via stat on the upload tick | M | 1.1 |
| 11 | `checked` + `[[versions]]` overlay + loader rules; remove `version`/`version_notes`/VersionMismatch; 16 headers to `checked` | M | 1.1 |
| 12 | Fold opencode2-cli into opencode-cli (base 2.x, row `1.x`), with a golden test proving identical keys; drop the builtin line, pad file and tiers row; keep the verifier hints.rs arm for event records | M | 1.1 |
| 13 | Hermes 0.22.6 row: the env fix from task 1, or a `say`+`gap` row | S | 1.1 |
| 14 | tiers as a (version, os) list, last 5; tier line lists nearby verified builds | S | 1.1 |
| 15 | Verifier: version in NoNative, reader "checked on", unknown-line counts | S | 1.1 |
| 16 | Pad matrix: `versions` list, templated install, Linux rows for CORE | M | 1.1 |
| 17 | Redirect effect check (fold warn_missing) for claude/codex/droid | M | 1.1 |

## 8. Risks
- **Code landing in a live release week.** Tasks 2-9 are S and gated. If the gate is red Thursday, ship tasks 1, 4 and 7 only (truth plus docs), and the docs state the limits.
- **Hermes has no participant-reproducible green row on release day.** If task 1 fails on 0.22.5, CORE Hermes ships "not verified". Say so plainly; do not lean on pad config.
- **Breaking build mid-event.** Without task 9 the only channels are the event page and email. A 1.0.x hotfix mid-run is dangerous: packaging/install.sh replaces the binary, and the watchdog respawns the recorder via current_exe (watchdog/mod.rs:269). This is cr-watchdog's critical (arch-x1). An installer refusal during a live run must land first.
- **Server notes are remote text in a terminal.** Plain text, capped count and length, control and escape characters stripped, advisories file reviewed in git.
- **Unread version on OpenCode 1 after the fold** gets the 2.x base keys and misses its host_hints. The line names the assumed version, `version_row=null` is recorded, and the seal read records the real version.
- **Stat false positives** (npm touching files): report the change, never act on it.
- **First-match order mistakes** in rows. The overlap warning covers this, plus "most specific first".

## 9. Open questions for the owner
1. Server `notes` before Friday (task 9): go or no-go? Go = a lever during 7-15 Nov; no-go = event page plus email only.
2. If Hermes 0.22.5 is not green without the pad block by Thursday, ship Hermes as "not verified" (recommended) or drop it from CORE?
3. Installer refusal during a live run (cr-scripts) and the recorder exe pinned per run (cr-watchdog): in 1.0.0, or accept the risk and forbid hotfix installs during the event?
4. Who runs the canary and the runbook 7-15 Nov, and from which machine (keys live on the maintainer Mac)?
5. RULES.md:207-209: rewrite to "read at start, continue and seal" now (recommended), or keep the sampling promise and ship the stat in 1.0.x?
