# versions lane: harness version support (map for the planners)

Checkout read: /Users/team/dev/worktrees/goal-cup-tracer/mac-test-v1.0 (= origin/main 70eecdfb + pad/plans).

## Top findings (one line each, most severe first)
- V1 important: tier line cannot read "v"-prefixed versions -> once a harness is "verified", Hermes ("Hermes Agent v0.22.6 (date)") and OpenCode 2 ("opencode v2.2.2") always print "verified with goalcup on X" even on a different/broken build. crates/goalcup/src/cli/start.rs:442-444 + text.rs:1161-1167 (None falls to the plain "verified" arm). Same parser hides the version in setup: cli/setup.rs:127-130.
- V2 important: Copilot prints "GitHub Copilot CLI 1.0.105." (trailing stop, docs/harnesses/copilot-cli.md:17); tier compare is raw `v != verified` (text.rs:1161) -> "verified on 1.0.105, not on 1.0.105." The pad strips the dot with sed (scripts/pad/harnesses/copilot-cli.toml:16), goalcup does not.
- V3 important: four different version parsers that disagree: connector `version` word-prefix with v-strip (connector/mod.rs:237-240), version_notes exact-with-v-strip (mod.rs:251-257), tier/setup "first word starting with a digit", no v-strip, punctuation kept (start.rs:442, setup.rs:128), pad regex `\d+\.\d+\.\d+[0-9A-Za-z.+-]*` (scripts/pad/pad-run:314). The pad writes tiers.toml from regex output, goalcup compares with its own parse. Fix: one `harness_semver(line)` fn in connector/, used everywhere, unit-tested on every connector's real --version line.
- V4 important: "verified" is ONE exact version per harness (connectors/tiers.toml `version = ""` string; scripts/pad/table:82-88 makes a harness "not verified" when case rows are on different versions). No range, no list, no history. With Amp ~7 builds/day, Copilot/Droid daily, Kilo several/day (PRD-11:912) a participant is almost never on the verified build.
- V5 important: the only per-version knob is `version_notes` (text only). Every behavioural field (args, env, ca_env, home, native patterns, no_prompts/update_off keys, host_hints keys, refuse_if_set keys, before/after) is single-valued per connector (connector/schema.rs:15-48). A breaking version means cloning the whole connector under a new id (opencode-cli / opencode2-cli), which also needs new arms in goalcup-verify usage/hints.rs:30-51,58-80,103-114, pad files, tiers rows. 0.7 had `[launch_prep]` (DECISIONS D72) which spanned Hermes 0.19-0.22; PRD-13:108 cut it.
- V6 important: Hermes 0.22.6 (CA ignored, every model call fails TLS, MAC-LEARNINGS #1) is handled by a note only; the working fix (`providers.goalcup-ca.ssl_ca_cert: ${env:SSL_CERT_FILE}`) lives only in the pad's config (scripts/pad/harnesses/direct/hermes-config.yaml:12-16), so the pad passes 0.22.6 (MAC-LEARNINGS rows) while participants on 0.22.6 record nothing usable. Pad tests a config participants do not have.
- V7 important: Hermes version reads "unknown" on a fresh isolated home (`--version` exits 1 before deps install; MAC-LEARNINGS #1, commit 678e261f/7a51ed5b, docs/harnesses/hermes-cli.md:20) -> the 0.22.6 note does not fire exactly when it matters (first run). Also 5 s probe timeout (connector/launch.rs:152) vs 6+ min first-start install.
- V8 important: VersionMismatch refusal says "Install that one" (cli/text.rs:179-182) instead of naming the sibling connector; PRD-11:148 and :921 promised "use connector X / points to opencode2-cli". `goalcup setup` lists BOTH opencode-cli and opencode2-cli as installed whenever `opencode` is on PATH (cli/setup.rs:250-255, no version check) -> 50% chance the participant picks the wrong one and is told to reinstall OpenCode.
- V9 important: RULES.md:207-209 promises "Goal Cup Proxy samples the version, records a change as an event". No code does: version is probed once per `start`/`continue` (connector/mod.rs:234); `continue` only prints text::updated (cli/continue_run.rs:282-288), no event. A mid-run self-update is invisible. Amp, Crush, Goose have no update_off/auto-update env at all (connectors/amp-cli.toml, crush-cli.toml, goose-cli.toml).
- V10 important: native readers in goalcup-verify are keyed by connector id only, never by harness_version (usage/hints.rs:30-51); unknown lines are dropped silently (usage/native.rs:171-197, rollout 466-517 `_ => {}`). A format change shows as Reason::NoNative "could not read the harness's own log" (legs.rs:18) with no mention that the harness version differs from the one the reader was written for (version only in doc comments: native.rs:8-9, hints.rs:130-133, rpc.rs:2).
- V11 important: settings/env keys that silently stop working after an update -> blocked or unrecorded runs, no detection: e.g. claude-code CLAUDE_CODE_DISABLE_AGENT_VIEW (background agents bypass recorder), `permissions.defaultMode` (prompt blocks harness: point 3), droid FACTORY_HOME_OVERRIDE "read in the binary, not in the docs" (droid-cli.toml:2), codex `check_for_update_on_startup`. write_settings writes the key whether or not the version reads it.
- V12 minor: version drift in the repo itself: Amp is 0.0.1793275344 in connector header + docs/harnesses/README.md:26, 0.0.1793664053-ga6248e in pad (scripts/pad/harnesses/amp-cli.toml:9), 0.0.1792209635 refused by Amp's server (MAC-LEARNINGS 7c); Hermes connector header 0.22.5 vs Mac release test on 0.22.6. Connector `# X.Y.Z` headers are comments, not data: CONTRIBUTING.md:94 "the version the connector names" has nothing to read.
- V13 minor: tiers.toml ships all "not verified" (seed 5dc9adf5); README harness table copies it by hand (docs/harnesses/README.md:36-37 "not regenerated").
- V14 minor: pad's opencode version_cmd re-points ~/.local/bin/opencode to the pinned prefix (scripts/pad/harnesses/opencode-cli.toml:13), so the 3 Nov "run any installed version" change (ee5849c2) never applies to OpenCode; pad image installs only pinned versions (2157b7c3), so "any version" means the Mac only.

## Map: every place a harness version is pinned, compared or assumed

### goalcup (crates/goalcup)
- connector/schema.rs:19-23 `version: Option<String>` (prefix), `version_notes: Vec<{versions, say}>`.
- connector/mod.rs:234-236 probe (`free` lock only), 237-248 prefix match -> Refusal::VersionMismatch, 251-263 exact notes, 275-277 `note` event `what=version_note`.
- connector/launch.rs:146-171 `program --version`, env = connector env incl. isolated home vars, stderr dropped, 5 s, first non-empty stdout line, success exit only.
- cli/start.rs:293-301 prints notes, `version_unread`, tier_line; 439-445 tier_line reads TIERS (compiled tiers.toml).
- cli/text.rs:179 mismatch wording; 1142 version_unread; 1158-1175 tier_line.
- cli/setup.rs:121-132 probe at setup, shows version (digit-first word) + notes.
- cli/continue_run.rs:211 wire_version, 282-288 "updated" line when entry versions differ, 293-297.
- cli/approve.rs:132 + wire_version: "unknown" when unread, cut to 64 UTF-16 units; server/worker/src/runs.ts:144 stores it (64).
- pipeline/boot.rs:251 start event `harness_version`; cli/recover.rs:223 carries it.
- cli/status.rs:186 shows version_note.
- Only users of `version`: opencode-cli "1.", opencode2-cli "2.". Only version_notes: hermes 0.22.6.

### connectors/*.toml (version assumptions are comments + implicit keys)
- Header comment names the version checked: amp 0.0.1793275344, claude 2.1.312, cline 3.0.74, codex 0.165.0, crush 0.99.0, droid 0.251.0, gemini 0.66.0, goose 1.57.0, hermes 0.22.5, kilo 7.8.25, kimi 2.4.1, openclaw 2026.10.9, opencode 1.19.5, opencode2 2.2.2, qwen 0.26.4; copilot none.
- Version-sensitive data in each: env names (CLAUDE_CODE_*, OPENCODE_DISABLE_AUTOUPDATE, FACTORY_HOME_OVERRIDE, QWEN_RUNTIME_DIR, KIMI_CODE_HOME, GOOSE_PATH_ROOT, GEMINI_CLI_HOME), args (codex --no-daemon, opencode2 --standalone, hermes chat/gateway stop -y), settings keys (no_prompts/update_off), host_hints keys (opencode1 provider.*.options.baseURL vs opencode2 providers.*.settings.baseURL), native root/pattern, mask json_field names (x-codex-*), ca_env (CODEX_CA_CERTIFICATE).
- tiers.toml: {tier, version (one string), reason}; all "not verified" today.
- model-hosts.toml: no version data; `pinned = []` is cert pinning, unrelated.

### goalcup-verify
- usage/hints.rs: readers by (connector id, row name, file name); WIRE_IDS header names per connector; join_of, thread_rpc (amp), reads_native list, thread_db (crush/goose table+column names, xxh3 hash).
- usage/native.rs: Codex rollout (session_meta, turn_context, event_msg/token_count, item_completed CollabAgentToolCall), Claude (assistant message.usage, requestId, agent- files), Gemini (type gemini, tokens), Qwen (assistant usageMetadata), Kimi (llm.request/usage.record/subagent.spawned), Hermes manifest/task-log paths. Version pinned in doc comments only.
- usage/rpc.rs Amp 0.0.1793664053 JSON-RPC shapes. usage/sqlite.rs Hermes state.db reader (connector no longer has a state row: dead path).
- usage/mod.rs:368 harness_version kept for output (out.rs:115), never used to pick or warn.

### Pad (scripts/pad)
- harnesses/<id>.toml: `version` (pin), `version_cmd`, `install` at exact version. pad:1091 requires both.
- pad-run:312-319 (ee5849c2, 3 Nov): any installed version runs; row records it; "(pinned)" or "(pad file: X)".
- table:82-88 verified only if all case rows share one version -> tiers.toml version.
- image.id: snapshot with all 16 pinned versions (2157b7c3).

### Docs
- CONNECTOR-SCHEMA.md:7 "Nothing is pinned"; :22-23 version and version_notes rules. Says builtin.rs is "the one source file that names connector files" but goalcup-verify hints.rs names connector ids too.
- RULES.md:207-209 (version sampling promise, not implemented).
- HARNESS-ONBOARDING.md: nothing about new versions of an existing harness.
- skills/adding-a-capture-harness/SKILL.md:30-34 version_notes + pinned pad version.
- CONTRIBUTING.md:94 pad row "from a run of the version the connector names".
- docs/harnesses/README.md table column "Harness (version checked)".
- TODO.md:66 Amp pin bump; :73 drop Hermes note later.

## What breaks when a harness updates (by layer)
- Wire (provider API shape): mostly provider-owned, robust; harness-specific headers in WIRE_IDS (x-opencode-*, x-codex-*, agent-session-id) and mask json_fields can be renamed.
- Transcript layout: native root/pattern + reader shapes (Codex rollout event names, Claude usage/requestId, Kimi wire types, Gemini tokens). Silent -> NoNative.
- Env vars: home redirects (CLAUDE_CONFIG_DIR, CODEX_HOME ...): a rename writes into the participant's real home and native capture misses (warn_missing gives a line for some rows).
- CA trust: Hermes 0.22.6 regression; Codex CODEX_CA_CERTIFICATE; Node NODE_EXTRA_CA_CERTS. Shows as TLS failures / tunnels.
- Flags: --no-daemon, --standalone, gateway stop -y (Hermes 0.19 had no -y).
- Model hosts: new default hosts become tunnels (not decrypted) -> calls missing from wire.
- Server-forced upgrades: Amp server refuses old builds (MAC-LEARNINGS 7c).

## What the participant sees today
- Same version as verified: "X: verified with goalcup on V." (after release rows; today all "not verified ... goalcup status says if model requests are missing").
- Different version: "X V2: verified with goalcup on V, not on V2." (no hint if likely fine); for v-prefixed versions: silently "verified on V".
- Version unreadable: "X's version could not be read, so the run starts without it." + upload "unknown".
- Prefix mismatch (OpenCode only): start refused, "Install that one".
- Known-bad version: the note text (Hermes 0.22.6), only when version readable.

## Version drift incidents in history
- b5a34483 (30 Oct): version check was substring, "1." matched OpenCode 2.1.x and "2." matched 1.12.x.
- 678e261f -> 7a51ed5b (1 Nov, same day flip): probe in real home wrote ~/.codex ~/.gemini ~/.hermes; probe in isolated home makes Hermes 0.22.5 exit 1 -> server refused null harness_version (cd69f744 sends "unknown").
- 2157b7c3: pad image rebuilt with all 16 pinned harnesses.
- e86b8d0a Mac test wave: Hermes 0.22.6 CA regression -> version note; Amp reader written against 0.0.1793664053.
- MAC-LEARNINGS 7c: Amp server refused pinned 0.0.1792209635.
- ee5849c2 (3 Nov): pad stops refusing unpinned versions.
- DECISIONS D72 (0.7): Hermes 0.19/0.21/0.22 differences handled by [launch_prep]; cut in 1.0 (PRD-13:108).
- 20b9d83d, a123d1a6: harness_version kept with spaces; cut to 64 units on the wire (real Hermes line 62).

## Direction for the multi-version plan (input for planners)
1. One version parser (semver-ish, v-strip, trailing punct) used by connector match, notes, tier, setup, pad.
2. tiers.toml -> per-harness list of verified versions + a `tested_range` (e.g. ">=2.1.300,<2.2"); tier line says "verified on 2.1.312; you have 2.1.318, inside the tested range" / "newer than tested".
3. Connector `[[versions]]` overlay tables: `match = ">=0.22.6"` with the same keys as the base (env, args, no_prompts, native, extra config rows like Hermes ssl_ca_cert). Fold opencode2 into opencode as an overlay; keep one id.
4. Readers: tolerate-and-count unknown lines; report "N lines of an unknown shape (harness V, reader written for W)".
5. Mid-run version sample (re-probe at each upload tick or on harness exec) and a `version_changed` event, or drop the RULES claim.
6. Pad: matrix of (harness, version) rows; table keeps per-version verdicts; nightly "latest" lane installs `@latest` to detect drift early.
7. A `goalcup doctor`/setup check that verifies the redirect env vars actually took effect (files appear under {home}) instead of trusting the version.

## Re other lanes
- arch: agree usage/native.rs keyed by connector id; add that the key ignores harness_version (V10).
- arch-6: agree and it matters most for versions: tiers.toml, connectors and version_notes are include_str! into the binary (cli/start.rs:26, connector/builtin.rs:5). A new harness release that breaks (Hermes 0.22.6 style) needs a goalcup release to warn anyone. Plan: fetch a signed version-compat file from the server at setup/start (fallback to built-in).
- arch-8: agree; same as V3/V5. One `[[versions]]` table in the connector as the single source.
- usability-11 / docs-1 / docs-2: agree (= V8 / V9). cr-scripts inbox: thanks, folded into pad map (image:43-45, table:31).

## Answers
- to cr-scripts (pad pinning facts): confirmed and used. Extra: pad-run:313 is a sixth version parser (regex \d+\.\d+\.\d+...), so pad rows store "2.1.312" while goalcup tier_line (cli/start.rs:442) takes the first digit-led word; they agree today only because no harness prints a leading v. table:82-84 has no OS dimension (all committed rows are darwin-arm64) -> see versions-x3.
- to cr-proxy (crush on macOS ignores SSL_CERT_FILE): disputed by evidence. docs/plans/2026-10-30-v1.0/pad-rows.jsonl crush-cli direct micro row, 2026-11-02T23:02, os darwin-arm64, pad_trusted_at null (no keychain trust): C1-C6 PASS, wire_requests 17, masks 22 per planted shape -> the calls were decrypted. Docs gap (crush-cli.md:44 says Linux only) stands; the "cannot trust the run CA" premise does not on this build (crush 0.99.0). Version/OS tiers still worth having.

## Round 2: cross-talk (checked in code)
- arch-7 / cr-watchdog-4 / usability-12: agree, and the gate does not close it: table --release-check (scripts/pad/table:128-178) checks rows and GUARDED diffs but GUARDED excludes connectors/tiers.toml (table:17-18) and nothing compares tiers.toml at the tag to what table renders from the committed rows. Today tiers.toml is still the phase-0 seed (only commit 5dc9adf5) while pad-rows.jsonl has 13 connectors. -> versions-x1.
- versions-x2 [important]: committed Hermes rows are all 0.22.6 (pad-rows.jsonl), the pad passes only because of hermes-config.yaml:13 ssl_ca_cert block, and connectors/hermes-cli.toml:43-44 tells participants 0.22.6 "cannot reach its model through goalcup". Once table writes tiers, a 0.22.6 participant sees both "Hermes 0.22.6: verified with goalcup on 0.22.6." and the cannot-reach note. release_check counts hermes-cli (CORE, table:16) on that version.
- versions-x3 [minor]: tier_for (table:82-89) ignores row["os"]; all rows are darwin-arm64, but the tier line is shown on Linux too.
- cr-format-verify-5 / arch-2: duplicate of versions-9.
- usability-11: duplicate of versions-7.
- docs-1: agree (CONNECTOR-SCHEMA.md:7 "Nothing is pinned" vs opencode version prefix refusal at connector/mod.rs:242).
- docs-2: duplicate of versions-8.
- cr-watchdog-5: agree, minor. connector/mod.rs:242 `_ => None` when the probe fails; OpenCode is the only connector with `version`, so an unreadable version runs the possibly wrong OpenCode connector silently. Fix with the one-connector overlay (direction 3).
- arch-6: agree.
