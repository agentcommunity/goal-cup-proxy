# Plan: multi-version harness support, assurance first

Lane: plan-assurance. Read-only review of /Users/team/dev/worktrees/goal-cup-tracer/mac-test-v1.0 (release/v1.0-rows).
Angle: how we KNOW a harness version works, and what we do when a harness ships a breaking build while the
competition runs. Plan, not code.

Dates that set the plan (docs/RULES.md:37-52): release 6 Nov 12:00; start window 7 Nov 00:00 to 8 Nov 00:00;
runs last 168 h, so the last run ends by 15 Nov 00:00. A breaking harness release between 7 and 15 Nov is likely:
Amp ships about 7 builds a day and its server refuses old builds (MAC-LEARNINGS 7c), Copilot and Droid ship daily,
Kilo several times a day (PRD-11:912).

Count check: there are 16 connector files, not 17 (connectors/ also holds model-hosts.toml and tiers.toml;
connector/builtin.rs:8-25 lists 16).

## 1. Problem today (what we can and cannot prove)

What we know about a version today comes from one place only: pad rows. Everything else is a claim.

- Evidence: docs/plans/2026-10-30-v1.0/pad-rows.jsonl has rows for 13 connectors, each on exactly ONE version and
  ONE OS (all darwin-arm64): claude 2.1.312, codex 0.165.0, copilot 1.0.105, crush 0.99.0, gemini 0.66.0,
  goose 1.57.0, hermes 0.22.6, kilo 7.8.25, kimi 2.4.1, openclaw 2026.10.9, opencode 1.19.5, opencode2 2.2.2,
  qwen 0.26.4. No rows for amp, cline, droid. No Linux rows.
- The shipped verdict ignores that evidence: connectors/tiers.toml is still the seed (every row "not verified",
  version "", "not yet run on the pad") and is compiled in (cli/start.rs:26). The release check never re-renders
  tiers from the rows (GUARDED excludes tiers.toml, scripts/pad/table:17-18; release_check table:128-178). (versions-x1)
- The verdict model holds one exact version per harness and no OS: tier_for keeps only the latest row per case and
  marks the harness not verified when case rows differ in version (table:82-88); row["os"] is never read. (versions-3, x3)
- Six parsers disagree on what "the version" is: connector prefix with v-strip (connector/mod.rs:237-240),
  version_notes exact (mod.rs:251-257), tier/setup first digit-led word without v-strip (start.rs:442-443,
  setup.rs:127-130), pad regex (pad-run:314), raw line on the wire and in the verifier (runs.ts:144,
  goalcup-verify usage/mod.rs:368). Hermes "v0.22.6" and OpenCode 2 "v2.2.2" parse to None in the tier line and fall
  into the plain "verified" arm (text.rs:1167); Copilot "1.0.105." never equals "1.0.105" (text.rs:1161). (versions-1, -2)
- The pad passes a configuration participants do not have: every Hermes row is 0.22.6, made green by the
  providers.goalcup-ca.ssl_ca_cert block in scripts/pad/harnesses/direct/hermes-config.yaml:11-16, while
  connectors/hermes-cli.toml:42-44 tells 0.22.6 users they cannot reach their model. Hermes is CORE (table:16), so the
  release check leans on rows a participant cannot reproduce. Also unproven: that block defines a provider
  `goalcup-ca` while model.provider is `deepseek`; nobody has shown which route the green rows actually used. (versions-5, x2)
- The pad file pin and the rows disagree: scripts/pad/harnesses/hermes-cli.toml:10 pins 0.22.5, rows say 0.22.6,
  because pad-run:312-319 runs any installed version (ee5849c2). The pin is no longer the tested version.
- Version reading fails exactly when it matters: launch.rs:148-171 keeps only a successful exit within 5 s; Hermes on
  a fresh HERMES_HOME exits 1 before its dependencies install (docs/harnesses/hermes-cli.md:20), so the 0.22.6 note
  never fires on a first run, and a `version` pin is skipped silently when unread (mod.rs:242, cr-watchdog-5).
- No mid-run evidence: RULES.md:207-209 promises "samples the version, records a change as an event"; the code probes
  once per start/continue (mod.rs:234) and continue only prints text::updated (continue_run.rs:282-288). Amp, Crush and
  Goose have no update-off switch at all (no update_off/auto-update env in connectors/amp-cli.toml, crush-cli.toml,
  goose-cli.toml). (versions-9, docs-2)
- No way to tell a participant anything after release without a new binary: tiers.toml, version_notes and connectors
  are include_str! (start.rs:26, builtin.rs:5). The open answer carries no notes (cli/approve.rs:97-101, Opened is
  run/entry/deadline/token). A new binary is itself dangerous mid-run: install.sh renames the old binary away
  (install.sh:326-328, 208), and on Linux every recorder respawn then fails and the watchdog loops forever
  (watchdog/mod.rs:569-580; cr-watchdog-1, arch-x1).
- The verifier cannot say "this is a version problem": native readers are keyed by connector id only
  (goalcup-verify usage/hints.rs:30-51), unknown lines are dropped (usage/native.rs:171-197), and a format change shows
  as the generic NoNative reason (legs.rs:18). (versions-10)
- Setting/env redirects are written whatever version runs and never checked afterwards (CLAUDE_CODE_DISABLE_AGENT_VIEW,
  permissions.defaultMode, FACTORY_HOME_OVERRIDE, CODEX_HOME ...). (versions-11)

## 2. Design

Five principles, each one checkable.

A. "Verified" is a set of evidence tuples, not a single string. A tuple is (connector id, version word, os, case,
   pad config hash, goalcup commit, result). The verdict a participant sees is computed from tuples, never typed.
B. One version word. A single rule turns a `--version` line into a version word, and every consumer (connector
   matching, notes, tier line, setup, pad, table, server, verifier) uses that rule. The rule is pinned by a fixture of
   real `--version` lines, one or more per connector, that both the Rust and the Python test read.
C. Three honest answers, never "verified" by default. For the participant's version word W:
   - exact: a passing tuple exists for W on this OS family -> "verified with goalcup on W (macOS)".
   - near: W is not tested but shares the connector's band key with a verified version (band = same major.minor by
     default, a connector can say `band = "major"`) -> "tested on 2.1.312, not on 2.1.318; recording usually works
     within 2.1. goalcup status shows if model requests are missing."
   - untested / unread -> today's "not verified" line plus the version word, or "version could not be read".
   No range syntax: bands come from evidence plus one key. A range claim nobody tested is what we are removing.
D. Pad green must equal participant green. A pad row counts toward a verdict only if the harness ran with what a
   participant gets from goalcup (connector + documented participant steps). Pad config that goalcup does not apply
   is declared in the pad harness file (`pad_only = "why"`) and turns that row into "not evidence".
E. The server speaks last. The open answer (POST /v1/runs, also continue) may carry `notes`: plain lines the server
   chooses for (connector id, version word, os). That is the one channel that reaches a 1.0.0 binary during the
   competition. It never refuses a build (RULES.md:208 "refuses no build"); it informs and it flags.

## 3. Schema, CLI and record changes

### Record (GCR1 start event, pipeline/boot.rs:251) and open body (approve.rs, runs.ts:144)
- Keep `harness_version` (raw line, 64 units).
- Add `harness_version_word`: the parsed word or null. Server stores it per entry beside the raw line, so SQL never
  parses (no seventh parser).
- Add `tier_said`: the exact tier line shown, plus `tier_basis` ("exact" | "near" | "untested" | "unread"), so an
  organizer can see what the participant was told.
- Add `program`: {path relative to HOME, size, mtime} of the resolved program (stat only, no hashing, no exec).
- New event `harness_changed` {before, after} when a cheap stat of the same path changes during the run (sample on
  the existing 5-minute upload tick; no `--version` exec mid-run, because Hermes-style first-run installs make exec
  unsafe). This keeps the RULES.md:208 promise with no harness impact (point 3).
- End event: `harness_version_end`, one `--version` probe after the harness has exited (the fresh-home Hermes case
  reads fine by then). Continue already sends a version per entry.
- Server: notes it sent are stored on the entry (`notes_sent`), so the record says what goalcup and the server told
  the participant.

### Connector (connectors/<id>.toml, connector/schema.rs:15-48), after the competition
- Add ordered `[[when]]` tables: `version = "2."` (version-word prefix, the one rule), then any of: args, env, unset,
  home, ca_env, host_hints, no_prompts, update_off, secret_files, first_screen, native, `say` (note). First match wins;
  a key in the table replaces the base key. Unread version uses the base and says so.
- Cut `version` (hard refusal) and `version_notes`: `[[when]] say` replaces notes; folding OpenCode removes the only
  users of the refusal.
- Add optional `band = "minor" | "major"` (default minor) for the near verdict.
- Cut the version named in the header comment; the version facts are the rows, rendered into docs.

### tiers.toml (written by scripts/pad/table)
Replace `{tier, version, reason}` with a list per connector:
```
[[claude-code-cli]]
version = "2.1.312"
os = "darwin-arm64"
cases = ["direct", "gateway", "dodger"]
rows_at = "2026-11-02"
```
plus `reason` only for "not supported" or "no key". Order newest first. Compiled-in copy stays as the offline
fallback (GOALCUP_SERVER=off); the Worker serves the same file and computes the server note from it.

### Pad harness files (scripts/pad/harnesses/<id>.toml)
- `version` -> `versions = ["0.22.5", "0.22.6"]` (the band rows to run), `install` templated with `{version}`, each
  version in its own prefix (OpenCode already does this, opencode-cli.toml:13).
- `latest_cmd`: how to ask the registry for the newest build (npm view, GitHub releases, pip index), for the canary.
- `pad_only = "<why>"` on any [direct]/[gateway] files block that applies config goalcup does not.

### CLI
- `goalcup setup` and `start` print the version word and the three-way verdict.
- `goalcup setup --harness <id>` with an unreadable version prints how to read it by hand (no new flag).
- No new commands. `goalcup-verify check` prints the harness version per entry and, on NoNative or unknown lines, the
  versions its reader was checked on (constant beside each reader) and the count of lines of an unknown shape.

## 4. What to cut

- connector `version` key and the Refusal::VersionMismatch path (connector/mod.rs:237-248, text.rs:179-182), after the
  OpenCode fold.
- `version_notes` (replaced by `[[when]] say`).
- opencode2-cli as a separate connector after 15 Nov: fold into opencode-cli with `[[when]] version = "2."`. A plain
  alias does not work: continue_run.rs:141-149 loads the stored id and refuses when the loaded `connector.id` differs,
  so the fold waits until every event run's continue window is closed (after 15 Nov), and no alias is kept.
- Version words in connector header comments and in docs/harnesses/README.md's hand-copied table (README.md:36-37).
- The pad's single `version` pin and the "(pinned)"/"(pad file: X)" wording in pad-run:323.
- The pad-only Hermes CA block, or make it a participant step; it cannot stay as silent pad advantage.
- The duplicated `tier_line` digit parser (start.rs:442, setup.rs:128).

## 5. Migration of the 16 connectors

Mechanical, no behaviour change, done after 15 Nov except where marked:
1. All 16: drop the version from the header comment; add the real `--version` line(s) to the fixture file
   (crates/goalcup/tests/fixtures/version-lines.tsv: id, line, expected word). Before Friday for the parser.
2. hermes-cli: `[[version_notes]]` -> `[[when]] version = "0.22.6" say = ...`; if rollout step 1 (section 7) proves a
   participant route, the overlay also carries that setting.
3. opencode-cli + opencode2-cli: one file, base = 1.x, `[[when]] version = "2."` with args, host_hints, secret_files,
   no_prompts = [], first_screen. Verifier hints.rs arms keyed by "opencode2-cli" stay so event records still verify
   (the record names the old id); new records use the version word.
4. amp-cli, crush-cli, goose-cli: no update-off switch exists; keep the default minor band and give them the
   canary's highest frequency. Amp is "latest track only" since its server refuses old builds.
5. claude-code-cli, codex-cli, droid-cli and the rest: no overlay until a breaking build appears. An overlay is added
   only with a pad row that proves it (rule enforced by review, see tests).
6. Pad files (16): `version` -> `versions`, `install` templated, `latest_cmd` added.

## 6. Tests (how each claim is proved)

- Parser fixture: one Rust test and one Python test (scripts/pad/test_pad.py) read the same fixture; every connector
  must have at least one line; real lines include "Hermes Agent v0.22.6 (2026.9.24)", "opencode v2.2.2",
  "GitHub Copilot CLI 1.0.105.". Fails if any consumer parses differently.
- tier_line unit tests for exact / near / untested / unread / other-OS, with v-prefixed and dotted lines.
- table tests: two versions of one harness both verified; mixed OS; a `pad_only` row does not count; tiers.toml is
  rendered deterministically from rows.
- release check: re-render tiers from committed rows at the tag and fail on any difference from connectors/tiers.toml
  at the tag (closes versions-x1).
- Overlay tests (post): first-match-wins, key replace, unread uses base; loader refuses `home`, `ca_env`, `mask`,
  `refuse_if_set` inside a `[[when]]` (adopted from plan-data: the probe ran with `home`, and a row must never weaken
  masking, point 4). Golden test: folded opencode-cli at "2.2.2" equals today's opencode2-cli keys.
- Server: notes chosen by (connector, word, os); text is plain, length-capped, control characters stripped; an empty
  notes list changes nothing; continue gets notes too.
- Effect checks (post, M): after the harness exits, `goalcup status` checks the redirects took effect (files under
  {home} for home vars, the settings file still holds the written key) and logs a `redirect_unused` note. Pad check
  C-row asserts it.
- Pad matrix (data): for each harness, rows per (version in `versions`, os in {darwin-arm64, linux-x86_64}, case).
  Minimum to call a band verified: full direct row on both OS families for CORE; micro row for the rest.

## 7. Rollout

### Before Friday 6 Nov 12:00 (only S items; the release test is live, so each lands through the normal gate on main,
not in the test folder)
1. Hermes truth (pad time, no code): run hermes-cli full direct/gateway/dodger rows on 0.22.5, the version the connector
   recommends and the pad file pins, with the ssl_ca_cert block removed for 0.22.5. If 0.22.5 is green, those rows
   are the CORE evidence; mark 0.22.6 `pad_only`. If not, say so in the release notes; do not ship "verified".
2. One parser + fixture; tier_line and setup use it (fixes versions-1/-2). S.
3. table: honour `pad_only`; add `os` to tier rows and print "(macOS)" in the line; release check compares rendered
   tiers. Then regenerate tiers.toml from the committed rows so 1.0.0 ships the evidence, not the seed. S.
4. Open-answer `notes` (client prints up to 5 plain lines; Worker reads a small advisories JSON shipped with the
   Worker deploy). S client + S server. Owner go/no-go: it is the only lever that reaches 1.0.0 binaries during the
   window. If no-go, the fallback channel is the event page and participant email, and the plan says so.
5. install.sh refuses while a run is live (cr-scripts lane owns it); required so a 1.0.x hotfix cannot hang Linux runs.
6. scripts/pad/canary (script only, not in the binary): reads `latest_cmd` from pad files, prints new versions versus
   scripts/pad/latest.json, runs a micro row for each new version. S.
7. Docs: one "Harness versions" section in CONNECTOR-SCHEMA.md stating today's truth (two OpenCode connectors refuse
   the other major; everything else runs any build; tiers are pad evidence for one version and OS); per harness, the
   exact install command for the verified version (from the pad install strings), so a downgrade is one paste. S.

### During the competition (7-15 Nov): freeze the binary, run the process
Breaking-update runbook:
1. Detect: canary every 2 h on the maintainer Mac (keys are there) and every 6 h on the Linux image. Amp hourly.
2. Classify within 2 h from the micro row and the harness's own error log (MAC-LEARNINGS method 1-4):
   - Records fine -> append the tuple to tiers (Worker deploy only), server note becomes "verified".
   - Wire fine, native reader drift -> note "model calls are recorded; per-thread usage may be read later". No
     participant action. Fix the verifier later: GCR1 files are raw, verify can re-run after 15 Nov.
   - Model unreachable / prompts block / redirect lost -> server note with the exact downgrade command and "do not
     update during your run"; post the same on the event page.
3. Runs already on the bad build: query entries by (connector_id, harness_version_word); organizer flag, never a
   disqualification (RULES.md:208). Tell them `goalcup seal` then reinstall the good build and `goalcup continue`.
4. A goalcup hotfix (1.0.x) only for data a server note cannot cover, and only with step 5 of the pre-Friday list in
   place. Tag through release.sh, never a hand-pushed tag (cr-scripts-4).
5. Log each event in the ops notes: version, time seen, class, note text, runs affected.

### After 15 Nov
Overlays and the OpenCode fold, cut `version`/`version_notes`, tiers list format and server tier answer, stat sampling
and `harness_changed`, end-of-run version probe, verifier reader versions and unknown-line counts, redirect effect
checks, Linux rows in the default matrix, pad files to `versions`.

## 8. Risks

- Pre-Friday code in a live release test: every item above is S and gated; if the gate is red on Friday, items 2-4
  drop and the docs item states the limits instead.
- Server notes are a remote text channel into the participant's terminal: plain text only, cap length and count,
  strip control and escape characters, audit the advisories file in git.
- "near" can be wrong (a minor release that breaks): the line says "usually" and points at goalcup status; the canary
  exists to turn near into exact or into a warning within hours.
- Canary cost and noise: micro rows cost a few cents of model calls each; Amp's build rate means up to ~7 rows a day.
  Batch by hour.
- Overlays grow into code in TOML: allow only the listed keys, first match wins, no nesting, no expressions.
- Stat sampling false positives (npm touches files): report a change, never act on it.
- Hermes: if neither 0.22.5 nor a participant route for 0.22.6 is green, a CORE harness has no evidence on release day.
  That must be said plainly, not hidden by pad config.

## 9. Ordered task list

| # | Task | Size | When |
|---|------|------|------|
| 1 | Hermes 0.22.5 full rows on the pad; prove which route the 0.22.6 rows used; `pad_only` on the CA block | M (pad time) | before Fri |
| 2 | One version-word fn + shared fixture (Rust + Python), used by tier_line, setup, connector matchers, pad-run | S | before Fri |
| 3 | table: `pad_only`, `os` per tier row, release check re-renders and compares tiers | S | before Fri |
| 4 | Regenerate connectors/tiers.toml from committed rows | S | before Fri |
| 5 | Open/continue answer `notes` + Worker advisories JSON + client print | S+S | before Fri (owner go/no-go) |
| 6 | install.sh refuses during a live run (with cr-scripts) | S | before Fri |
| 7 | scripts/pad/canary + `latest_cmd` in 16 pad files + latest.json | S | before Fri |
| 8 | Docs: "Harness versions" contract + downgrade command per harness | S | before Fri |
| 9 | Runbook in OPERATING.md; canary cron on Mac and Linux image | S | Fri |
| 10 | Record fields: version word, tier_said/basis, program stat, notes_sent | M | after 15 Nov |
| 11 | harness_changed via stat on upload tick; end-of-run version probe | M | after 15 Nov |
| 12 | Connector `[[when]]` overlays; cut `version`, `version_notes` | L | after 15 Nov |
| 13 | Fold opencode2-cli into opencode-cli (no alias: continue refuses a changed id); keep verifier arms for old records | M | after 15 Nov |
| 14 | tiers list format (version x os x cases), near verdict with `band`; server computes the tier note | M | after 15 Nov |
| 15 | Verifier: reader "checked on" versions, unknown-line counts, version in NoNative reason | M | after 15 Nov |
| 16 | Redirect effect checks in status and pad | M | after 15 Nov |
| 17 | Pad matrix: `versions` list, templated install, Linux rows for CORE in the default run | M | after 15 Nov |

## 10. Where this differs from other plans

Read at the end: plan-data.md and plan-runtime.md (plus the versions lane "Direction").

Agree with both: one parser with a shared fixture used by Rust and the pad; `[[when]]`/`[[versions]]` overlays with
first match wins; cut `version` and `version_notes`; tiers as a (version, os) list written from rows; the release check
re-renders tiers and diffs; no tested_range field; the OpenCode fold waits until after the event because
continue_run.rs:141-149 refuses a changed connector id (plan-data found this; I corrected my alias idea).

Differences:
1. Server notes before Friday. Both plans defer any server-supplied text to after the event. I put a minimal
   `notes` field in the open/continue answer before Friday (owner go/no-go), because 7-15 Nov is exactly when a
   breaking build lands and a compiled-in binary cannot be told anything; a 1.0.x hotfix is the only alternative and
   install.sh mid-run hangs Linux runs (arch-x1). Plain text, capped, no refusal. If owner says no, the runbook falls
   back to the event page and email, and that limit is written down.
2. Refusal. plan-data keeps `status = "broken"` that refuses a start. I agree with plan-runtime: never refuse on version
   (RULES.md:208 "refuses no build", point 7). A known-broken build gets a note and an organizer flag.
3. Mid-run sampling. plan-runtime re-runs `--version` in a background thread (60 s, then every 10 min). I avoid
   exec mid-run: a stat of the resolved program on the upload tick catches a self-update, and one `--version` after the
   harness exits gives the end version. Exec inside the isolated home can start installs (Hermes) or touch state.
4. Evidence rule. Neither plan has `pad_only`: a pad row that relied on config goalcup does not apply must not count
   as evidence. That is the root of versions-x2, and it recurs whenever the pad works around a harness bug.
5. Canary. plan-data calls a nightly latest lane infrastructure and defers it; plan-runtime puts it post-event. Mine is
   a small script (registry query + micro row) run by cron on the maintainer Mac during 7-15 Nov, when it matters most;
   after the event it can become a nightly job.
6. Near verdict. plan-runtime uses comparator ranges in `[[when]] versions = ">=0.22.6, <0.22.7"`. I keep matching to
   the one prefix rule (as plan-data's `match`) plus a `band` key for the near tier line; fewer grammar rules to test.
