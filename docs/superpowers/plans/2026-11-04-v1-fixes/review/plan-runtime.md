# Plan: multi-version harness support, runtime-first (lane: plan-runtime)

Angle: goalcup reads the installed harness version at `goalcup start`/`continue`, picks the connector variant that fits it, never refuses on version (points 3, 7), writes a gap note when it has to fall back, and tells the participant in one plain line what is and is not supported for their build (point 6). The pad and tiers feed this, they do not gate it.

Checkout read: /Users/team/dev/worktrees/goal-cup-tracer/mac-test-v1.0 (= origin/main 70eecdfb + pad/plans). PLAN ONLY, no code written.

## 1. Problem today (cited)

1. Version is probed once, then mostly ignored. `launch::harness_version` (crates/goalcup/src/connector/launch.rs:146-171) keeps the first stdout line of `program --version`, and only when the exit is 0 within 5 s. It drops stderr. Nothing parses it into a comparable value.
2. There are five parsers that disagree (versions-2):
   - the connector `version` word prefix with v-strip (connector/mod.rs:237-248);
   - version_notes, an exact word with no digit after it (mod.rs:251-263);
   - tier_line, the first digit-led word with no v-strip and punctuation kept (cli/start.rs:439-445, setup.rs:127-130);
   - the pad regex (scripts/pad/pad-run:314) plus a sed strip in pad files (scripts/pad/harnesses/copilot-cli.toml:16);
   - the verifier and server, which keep the raw line (cli/approve.rs:132 cuts it to 64 units).

   Results: Hermes "v0.22.6" and OpenCode "v2.2.2" read as no version, so they show as verified (text.rs:1158-1167). Copilot "1.0.105." shows "verified on 1.0.105, not on 1.0.105."
3. Version is a hard gate in one place and only text everywhere else. `version = "1."`/`"2."` refuses the start (mod.rs:242, Refusal::VersionMismatch, text.rs:179-182 "Install that one"), but only for OpenCode. An unread version skips the gate silently (cr-watchdog-5). Every other harness gets one fixed behaviour, whatever version is installed (versions-4).
4. A behavioural version difference means a cloned connector. opencode-cli.toml and opencode2-cli.toml differ only in version, args (`--standalone`), the host_hints key path (`provider.*.options.baseURL` vs `providers.*.settings.baseURL`), secret_files, no_prompts and first_screen. `goalcup setup` lists both whenever `opencode` is on PATH (setup.rs:250-255). The participant picks by hand (usability-11, versions-8).
5. A known-broken build gets words, not a fix. Hermes 0.22.6 has only `[[version_notes]]` (connectors/hermes-cli.toml:46-48). The pad passes 0.22.6 through a provider block that participants do not have (scripts/pad/harnesses/direct/hermes-config.yaml:11-17), and that block is participant-specific: it names `base_url: https://api.deepseek.com/v1` (versions-5, versions-x2).
6. Unread at the worst moment. Hermes exits 1 on a fresh isolated home before its deps install (versions-6, docs/harnesses/hermes-cli.md:20), so the 0.22.6 note never fires on the first run.
7. No mid-run sampling. RULES.md:207-209 promises "Goal Cup Proxy samples the version, records a change as an event". In the code, version is read once per start/continue (mod.rs:234). continue_run.rs:282-288 prints `text::updated` but records no event.
8. Support facts are compiled in and single-valued. tiers.toml is `include_str!`'d (cli/start.rs:26) with one `version` string per id. scripts/pad/table:82-88 drops a harness to "not verified" when its case rows are on different versions. There is no OS dimension (versions-x3), and the release check does not compare tiers.toml against the rows (versions-x1).
9. The verifier does not know the version. Readers are picked by connector id only (crates/goalcup-verify/src/usage/hints.rs:30-51), and unknown lines are dropped silently. A format change shows as a bare Reason::NoNative (versions lane V10).
10. Settings and env redirects are trusted blindly. CLAUDE_CONFIG_DIR, CODEX_HOME, FACTORY_HOME_OVERRIDE, CLAUDE_CODE_DISABLE_AGENT_VIEW, permissions.defaultMode and others are written whatever version runs. Nothing checks that they took effect (versions-10).
11. Adjacent: goalcup's own version can mix mid-run. The recorder respawns `current_exe()` by path (watchdog/mod.rs:269-270, 569). An install.sh replace mid-run hangs the respawn loop on Linux and mixes versions on macOS (arch-x1, cr-watchdog-1).

## 2. Design

### 2.1 One version value
- New `connector/version.rs`, about 60 lines: `Ver { parts: Vec<u64>, tail: String, raw: String }`.
- `parse(line)`:
  - Take the first whitespace word that, after stripping a leading `v`/`V` and trailing `.,;:)`, starts with digit(s) and contains a dot.
  - Split on `.` and keep numeric parts until the first non-digit. The rest is `tail` (`-ga6248e`, `-beta.1`).
  - Fits every real line in docs/harnesses/*.md: "Hermes Agent v0.22.6 (2026.9.24)" gives 0.22.6, "GitHub Copilot CLI 1.0.105." gives 1.0.105, Amp "0.0.1793664053-ga6248e" gives [0,0,1793664053] (u64 is enough), OpenClaw "2026.10.9", "opencode v2.2.2".
- Ranges: comma-joined comparators `>=0.22.6, <0.23`, `<2`, `=2.1.312`, and `*`. Compare numeric parts only (missing parts count as 0) and ignore tail. No caret or tilde, no pre-release rules. This is the one function used by connectors, tiers, setup, the pad (Python port of about 15 lines, tested against the same fixture file) and the verifier.
- The raw line is still recorded unchanged.

### 2.2 Probe that can arrive late
- Read stdout and stderr, and accept output on a non-zero exit when it parses. Several CLIs print the version before failing on deps. Whether Hermes 0.22.5/0.22.6 does needs checking on the Mac pad; it is task T2's acceptance test.
- Keep the 5 s limit at start, so the harness is never held back (point 3).
- If unread at start: the watchdog re-probes in a background thread once the harness has run for 60 s, then every 10 min until it reads one. Hermes' first-run deps install is done by then. The result goes through the control channel (2.5). A late read can only add notes and a `version` event. It never changes launch behaviour already applied; the next `continue` applies the right variant.

### 2.3 Connector variants: `[[when]]` overlays (replaces `version` and `version_notes`)

```toml
checked = "2.2.2"            # data, replaces the header comment; the version the base keys were checked on

[[when]]                     # applied in file order; every match applies; later wins
versions = "<2"
checked = "1.19.5"
args = []                    # any behavioural key from the base may appear; a list REPLACES the base list
host_hints = [ ... provider.*.options.baseURL ... ]
no_prompts = [ ... permission = allow ... ]
secret_files = [ ... ]

[[when]]
versions = ">=0.22.6, <0.22.7"
support = "broken"           # ok (default) | partial | broken
say = "Hermes 0.22.6 ignores the run CA for model calls (NousResearch/hermes-agent#148241). goalcup records the run, but model calls may show as failed. 0.22.5 works."
gap = "harness_ca_ignored"   # gap word written as a note event; goalcup status and end screen list it
```

- Keys allowed in `[[when]]` are the behavioural set: args, resume, env, home, ca_env, unset, refuse_if_set, host_hints, hint_env, secret_files, before, after, native, mask, first_screen, end_note, no_prompts, update_off, skills_dir. Plus `versions`, `checked`, `support`, `say`, `gap`. Not allowed: id, name, program (one connector = one program).
- Selection:
  - Version read: apply every matching `[[when]]`.
  - Version unread: apply the base only, which is by convention the newest checked version (fresh installs are mostly newest). Add gap `version_unread` and the line "X's version could not be read; goalcup set it up for X <checked>."
- One merge function in connector/mod.rs: about 40 lines, a serde Value merge of base TOML and overlay TOML before typed deserialize. That keeps `deny_unknown_fields` and every existing loader check on the merged result. The connector sha256 stays the sha of the file bytes. The start event adds which overlays applied (2.6).
- Nothing refuses on version. `Refusal::VersionMismatch` and its text are cut. This matches D12 "refuses none" and RULES.md:207 "refuses no build", and fixes docs-1.

### 2.4 Plain support line (one line, replaces tier_line)
Inputs: the parsed version, the matched overlay `support`, and the support data from 2.7. Each line is said once, at start, continue and setup (setup gains the same parser):
- exact verified: "Claude Code 2.1.312: verified with goalcup on this version."
- in tested range: "Claude Code 2.1.318: not checked on this exact build; goalcup recorded 2.1.300 to 2.1.316 without problems."
- newer than tested: "Claude Code 2.2.0: newer than any version goalcup has checked (last 2.1.316). The run is recorded; `goalcup status` says if model calls are missing."
- older than tested: same with "older".
- broken/partial overlay: its `say`, plus "The run is recorded." Never "Install X".
- unread: as in 2.2.
- not on this OS (2.7 has an os key): "verified on macOS only."

All of these sit in text.rs as one function with a table-driven test.

### 2.5 Mid-run version change (makes RULES.md:207-209 true)
- The watchdog already ticks (spawn::TICK). Every 30 s it `stat`s the resolved program path and its realpath (inode, size, mtime). That is cheap and spawns nothing.
- On a change, or for a late probe, it runs the probe in a thread (never on the tick loop), then sends a new `ToRecorder::Version { line: Option<String> }` (watchdog/mod.rs:1013 enum).
- The recorder writes an `ev: "version"` event and updates status.json `harness_version_now`. `goalcup status` and the end screen show "X updated itself during the run: A -> B (recorded)". No harness action is taken, because a variant cannot be swapped under a live process.
- An older recorder that gets an unknown control line ignores it (check how ToRecorder parses unknowns, see risk R4).

### 2.6 Record changes (additive JSON in existing GCR1 events, no framing change)
- The `start` event (pipeline/boot.rs:246-258) adds `harness_version_parsed` ("0.22.6" or null), `variant` (list of the applied overlays' `versions` strings), `support` (ok/partial/broken/unknown) and `checked`. `harness_version` stays the raw line.
- New event `{"ev":"version","raw":..,"parsed":..,"why":"late_read"|"changed"}`.
- Gap words gain `version_unread`, `version_untested` (outside the tested range) and the overlay `gap`.
- Upload: no API change. approve.rs keeps sending the raw line (64 units). The server can add a parsed column later; nothing needed before Friday.
- Verifier: `goalcup-verify` reads `harness_version_parsed` and `variant` from the start event and prints them. The reader pick becomes `reader(connector, version, path)`, and readers count unknown lines instead of dropping them. The report says "N lines of an unknown shape (harness 2.2.0; reader checked on 2.1.316)" instead of a bare NoNative. Old records without the new fields parse as before, since the fields are optional.

### 2.7 Support data: from one exact string to ranges, still in the binary
- tiers.toml becomes, per id:
  - `verified = ["2.1.312", "2.1.316"]` (every version with a passing row set),
  - `tested = ">=2.1.312, <=2.1.316"` (min..max verified),
  - `os = ["darwin-arm64"]`,
  - `reason`.
- scripts/pad/table writes it from pad-rows.jsonl grouped by (harness_version, os). This replaces the "all case rows on one version" rule (table:82-88).
- A release check re-renders tiers from the committed rows and fails on a diff (closes versions-x1).
- Remote overlay of support text: see 2.9, post-Friday only.

### 2.8 Effect checks instead of trusting the version (versions-10)
- A connector `[[expect]]` row: `{ path = "{home}/projects/**/*.jsonl", after = "first_model_call", gap = "home_redirect_ignored" }`. The recorder checks it once, 60 s after the first wire call. A miss writes a gap note with the version in it: "Claude Code 2.1.330 did not write its session under the goalcup home; CLAUDE_CONFIG_DIR may no longer be read by this version."
- This reuses the native tailer's existing glob code. The existing `warn_missing` is close; fold it into this.
- Point 6 is then honest even for versions nobody has tested.

### 2.9 Out of scope before Friday, candidates after
- Server-served support/`say` data (fetch at start with a 1 s timeout from the Worker, fall back to built-in). It would let a Hermes-0.22.6-style warning reach participants without a goalcup release. It is data only, never behaviour, because a behaviour change remains a release. This adds code and a trust surface, so it waits for evidence that releases are too slow.
- A nightly pad lane that installs `@latest` per harness, so drift shows within a day. It feeds `tested`.

## 3. What to cut
- The connector `version` key, `Refusal::VersionMismatch`, text.rs:179-182, and the "Install that version" entry in PARTICIPANT-GUIDE Appendix A.
- `version_notes` (folded into `[[when]] say`), along with `exact`/`held` closures in mod.rs:237-263.
- connectors/opencode2-cli.toml (folded into opencode-cli as `[[when]] versions = "<2"` for v1, base = v2). Keep `opencode2-cli` as a load alias for one release so scripts and records still resolve, then drop it.
- Header version comments in all 16 connector files (become `checked = ...`).
- The tier_line digit-word parser (start.rs:442), setup.rs:127-130 parse, the pad `sed` strip in copilot-cli.toml:16, and the pad's opencode version_cmd re-point (opencode-cli.toml:13 in scripts/pad/harnesses).
- The single `version` string in tiers.toml.
- Net: likely fewer lines in goalcup. One parser and one merge replace three matchers, a refusal and a duplicate connector.

## 4. Migration of the 16 connector files (the brief says 17; `ls connectors/*-cli.toml` gives 16, which become 15)
- 13 of them (amp, claude-code, cline, codex, copilot, crush, droid, gemini, goose, kilo, kimi-code, openclaw, qwen): mechanical. Header comment becomes `checked = "<x>"`. No `[[when]]`.
- Amp: pick ONE checked version. The repo has three (connector header 0.0.1793275344, pad pin 0.0.1793664053-ga6248e, 0.0.1792209635 refused by server). Use the pad pin, since the reader in usage/rpc.rs was written against it. Add `[[when]] versions = "<0.0.1793000000" support = "broken" say = "Amp's server refuses this build; update Amp."` for the server-forced floor.
- Hermes: `checked = "0.22.5"`, plus `[[when]] versions = ">=0.22.6, <0.22.7"` with `support = "broken"` and the say/gap. Moving the pad's CA fix into the connector is NOT a copy-paste. The pad block defines a whole provider with a deepseek base_url. A connector row would need a per-provider key such as `providers.*.ssl_ca_cert` that also covers built-in providers, and that is unverified. Task T9 tests it. If it works it becomes a settings row in the overlay and support goes to "ok"; if not, the pad must drop the block so pad green = participant green (versions-x2).
- OpenCode: merge as in section 3. Base = 2.2.2 keys, overlay `<2` = 1.19.5 keys. setup lists one "OpenCode". goalcup-verify hints.rs arms keyed "opencode2-cli" also accept "opencode-cli" with a version >= 2.
- Claude Code: add an `[[expect]]` for transcripts under {home} (CLAUDE_CONFIG_DIR). Codex: the same for CODEX_HOME sessions. Droid: the same for FACTORY_HOME_OVERRIDE. These three are where a silently ignored redirect loses native evidence.
- Pad files keep `version` as an install pin (that is install data, not support data) and the pad stops deciding support itself.

## 5. Tests
- version.rs: a fixture file `connectors/version-lines.txt` with every real `--version` line from docs/harnesses/*.md and pad rows, giving the expected parse. Used by the Rust unit test AND the pad's Python port, so the parsers cannot drift (fixes versions-2 for good).
- Range compare table test, including tails and missing parts.
- Overlay merge: for opencode, the merged `<2` connector must equal today's opencode-cli.toml field for field (load both, compare structs), and the base must equal today's opencode2-cli.toml except id. That proves the fold loses nothing.
- tests/connector_builtin.rs: each `[[when]]` must have a parseable `versions`, only allowed keys, and a non-empty `say` when support != ok.
- The support line: a table test over {exact, in range, newer, older, broken, unread, other OS}. Add a no-"Install" assertion (no refusal wording).
- Watchdog: a fake program whose `--version` output changes mid-run (a test script swapped on disk) must produce a `version` event within 2 ticks, and the harness pid must be untouched.
- Late probe: a fake program that exits 1 for its first N calls, then prints a version. The start says unread, and later a `version` event with why=late_read appears.
- Verifier: an old record without the new fields still verifies. A record with unknown native lines reports the count and versions.
- Pad: `table --release-check` fails when tiers.toml differs from re-rendered rows.

## 6. Rollout

Before Friday 6 Nov (3 days, release test live, hard shipping mode): only small, low-risk pieces that fix wrong participant text.
- T1 parser plus its use in tier_line, setup and notes (fixes versions-1, versions-2; S).
- T3 `version_unread`/broken wording with no "Install"; for OpenCode, change the mismatch text to name the sibling id ("use --harness opencode2-cli"), not cut the refusal yet (S).
- T5 tiers.toml written from the committed rows and the release check comparing them (versions-x1; S/M).
- T9 decision on Hermes 0.22.6: either the connector carries the fix or the pad drops it. Do not ship verified-on-0.22.6 while the note says it cannot record (versions-x2; S to decide).
- T11 adjacent goalcup-self-version safety (arch-x1): copy or hardlink the exe into the entry dir at watchdog start and spawn the recorder from that copy. Add a version check on the first control line. This one is critical on Linux if the install path can run mid-event (M; owner call).

After Friday (v1.1): T4 overlays plus the OpenCode fold, T6 mid-run sampling, T7 record fields and verifier reader-by-version, T8 expect rows, T10 nightly latest lane. Remote support data waits for evidence.

## 7. Risks
- R1 Overlay merge semantics ("lists replace") surprise connector authors who expect append. Mitigation: one rule, documented in CONNECTOR-SCHEMA.md, and the opencode equality test shows it.
- R2 Base = newest means an unread version on an old install gets new-version keys. For OpenCode 1 that means a wrong host_hints path, so config hosts are missed and calls show as tunnels. Mitigation: the gap note names the assumed version, and the late probe plus continue apply the right variant.
- R3 Late probe and mid-run stat must never block or slow the harness. Both run off the tick thread, with a 5 s limit, and at most one probe in flight.
- R4 Older records and older binaries: unknown event/control types must be ignored. timeline.rs:110 `_ => {}` suggests the verifier ignores unknown events. Confirm for integrity.rs and for the ToRecorder line parser before shipping T6.
- R5 Accepting output on a non-zero exit can read an error line that contains a dotted number. Mitigation: parse only the first non-empty line, and require a digit-led word with a dot.
- R6 Removing the OpenCode refusal means a participant on an untested OpenCode 3 gets the v2 keys and possibly wrong host hints. That is acceptable under point 3 and is said plainly by the support line ("newer than any version goalcup has checked").
- R7 `tested` ranges assume monotonic compatibility. A regression inside a range (Hermes 0.22.6 between 0.22.5 and a fixed 0.22.7) is handled only by a `broken` overlay, which someone has to write.

## 8. Ordered tasks (size)
1. T1 `connector/version.rs` parser, range compare and fixture file. Use it in tier_line, setup and notes, and port it to pad-run and table (S).
2. T2 Probe: read stderr too, accept a parseable line on a non-zero exit. Verify the Hermes fresh-home output on the Mac pad (S).
3. T3 Support line rewrite in text.rs (table-driven). Remove "Install that one"; for OpenCode, name the sibling id until T4 (S).
4. T5 table writes `verified`/`tested`/`os` from rows, and the release check compares tiers.toml to the re-rendered rows (M).
5. T9 Hermes 0.22.6: test `providers.*.ssl_ca_cert` (or equivalent) as a connector row. Ship it or drop the pad block (S investigation, S change).
6. T11 Recorder exe pinned per run plus a version handshake on the first control line (M; adjacent, owner call for pre-Friday).
7. T4 `[[when]]` overlays (merge, allowed keys, loader checks). Fold opencode2 into opencode with a load alias. Cut `version`, `version_notes` and VersionMismatch. Update CONNECTOR-SCHEMA.md and the guide (M).
8. T4b Migrate the 16 connector files to `checked =` plus overlays (Amp floor, Hermes broken) (S).
9. T7 Start-event fields, `version` event, gap words, and the verifier prints them. Readers keyed by (id, version), unknown-line counts (M).
10. T6 Watchdog stat sampling, late probe, ToRecorder::Version and status/end-screen line (M).
11. T8 `[[expect]]` effect checks (claude, codex, droid first), folding `warn_missing` (M).
12. T10 Nightly `@latest` pad lane feeding `tested` (M, post-event).
13. T12 Docs: one "Harness versions" section in CONNECTOR-SCHEMA.md, the source of truth that docs-x1 asks for. Fix RULES.md:207-209 to match T6, and README "version checked" to "versions checked" (S).

## 9. Where this differs from other plans

### Against plan-data.md (read at finish)

We agree on:
- one parser with a fixture table;
- overlays on one base;
- folding opencode2 into opencode;
- cutting `version`/`version_notes`;
- `checked =` replacing the header comments;
- tiers rendered from rows and checked at release;
- before Friday, only the parser, tiers and docs;
- deferring a remote compat file.

We differ on:
1. **Refusal.** plan-data keeps `status = "broken"` as a start refusal. I never refuse on version. A broken build is recorded with a gap note and plain text (points 3 and 7, and RULES.md:207 "refuses no build"). A run that cannot reach its model still records the failed attempts, which is evidence for the organizer. If the owner wants a refusal, limit it to a `support = "broken"` that has a cited issue, as plan-data says.
2. **Mid-run.** plan-data re-reads only at seal and continue. I add a late probe when the version is unread: one background probe 60 s into the run. That is the only way the Hermes 0.22.6 warning reaches a first-run participant while it still matters. Seal re-read as the minimal version: adopt it. Stat-based change sampling can move to post-event; the late probe should stay.
3. **Match semantics.** plan-data uses first-match-wins. I use all matches, later wins. **I concede: first-match-wins is simpler and cannot surprise anyone.** Adopt it, with the "most specific first" rule and an overlap warning.
4. **Keys a row may not override.** plan-data forbids `home`, `ca_env`, `mask` and `refuse_if_set` in a row. **I adopt this.** The probe ran with `home`, and a row must never weaken masking (point 4). This replaces my allowed-key list in 2.3.
5. **Tested range.** I keep a derived `tested` (min..max verified) for the "inside the checked range" line. plan-data shows the list only. It is derived data, not a sixth hand-kept encoding, but it is optional. Drop it if it costs more than the line is worth.
6. **The OpenCode alias.** plan-data is right that continue_run.rs:141-149 refuses when the loaded `connector.id != sj.connector.id`, so a plain alias that loads `opencode-cli` breaks `continue` of opencode2-cli runs. Either keep the stored id on alias load (the merged connector reports the requested id) or fold after the event, as plan-data says. Fold after the event, with no alias, is simpler. Adopt it.
7. **Hermes 0.22.6.** Add plan-data's cheaper option: test whether a CA env var (REQUESTS_CA_BUNDLE, HTTPX_CA_BUNDLE, or SSL_CERT_DIR) works as a row `env` before considering wildcard settings writes.
8. **Effect checks.** I keep `[[expect]]` (2.8) as a post-event item. plan-data defers it to warn_missing. Both agree it is not pre-Friday.

### Expected differences from other plans
These were written before any other plan was read:
  - vs a pad-first or matrix plan: here the pad only informs the text. It never decides whether a run starts, and runtime adapts without a pad row.
  - vs a remote-compat-file plan (versions lane, arch-6 reply): deferred. Behaviour comes from compiled overlays; only text could come from the server, and not before Friday.
  - vs keeping opencode2-cli as its own id: fold it. One program means one connector, chosen by version.
