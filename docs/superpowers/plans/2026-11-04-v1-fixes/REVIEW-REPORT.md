# goalcup 1.0 review report

Ten review lanes (5 code review: proxy, watchdog, upload-server, format-verify, scripts; 5 other: arch, usability, oss, docs, versions) plus cross-talk rounds and a verify pass on every finding. Checkout: `/Users/team/dev/worktrees/goal-cup-tracer/mac-test-v1.0`. Paths below are relative to that checkout. Lane notes are in this folder.

## Verdict

Ship 1.0 with fixes. No finding survived verification as critical. The hot path keeps points 3 and 4: the relay writes first and never waits on recording, masking fails closed, the CA is constrained, the upload protocol is idempotent, and the record format and verifier are correct and well tested. What can still hurt a participant is at the edges. `start` opens a competition run for a harness that is not on PATH. The README `--prompt` example cannot work. Networks that need an upstream proxy or a corporate CA are not supported, and nothing tells the participant why. The release gate can ship a `tiers.toml` that does not match the pad rows, and the Hermes rows only pass with a pad-only CA workaround that participants do not have. macOS, where most participants are, has no automated lifecycle tests. Multi-version support is a plan, not 1.0 code. Do its "now" and "before Friday" steps, which are small and mostly about truth.

## Fix before release (by Thu 5 Nov)

No finding was confirmed critical. These important ones cost a participant their run, or ship a false claim, so land them before 6 Nov.

1. **usability-1: `start` spends the approval and opens the run for a harness that cannot launch.** `crates/goalcup/src/cli/start.rs` `check()` (163-233) never tests `plan.program.is_absolute()`. `setup.rs:76` does. Fix: in `check()`, refuse with `text::not_installed(&connector.name)` when the program does not resolve. Make `closing()` (`cli/finish.rs:207`) name the io kind instead of the bare "did not start".
2. **usability-5: the README quick-start `--prompt prompt.txt` always fails.** `README.md:41,44`. `folder.rs:31` resolves a relative path inside the project folder, so you get either `prompt_inside` or `prompt_missing`. Fix: use `~/goal-prompt.txt`, as `PARTICIPANT-GUIDE.md:213,218` already does. Optionally, make `prompt_missing` say that relative paths are taken inside the project folder.
3. **cr-scripts-x1 = versions-x1: the release gate never checks that `connectors/tiers.toml` matches the pad rows.** `scripts/pad/table:17-18` excludes the file, and `release_check` (128-178) never calls `render_tiers`. The tree has already drifted: tiers.toml is still the phase-0 seed (all "not verified"), but `pad-rows.jsonl` holds 30 rows for 13 connectors. The file is compiled in (`cli/start.rs:26`). Fix: in `release_check`, render tiers from the committed rows and fail on any diff against `git show <sha>:connectors/tiers.toml`. Then regenerate and commit tiers.toml. Also run `table --release-check` in `release.yml`, so a hand-pushed tag cannot skip the gate (cr-scripts-4).
4. **versions-x2 (with versions-5): release qualification for Hermes rests on 0.22.6 plus a pad-only CA block.** All 17 hermes rows are 0.22.6. They work only through `scripts/pad/harnesses/{direct,gateway}/hermes-config.yaml:11-16` (`providers.goalcup-ca.ssl_ca_cert`). Meanwhile `connectors/hermes-cli.toml:42-44` tells 0.22.6 users they cannot reach their model. The pad pins 0.22.5 (`scripts/pad/harnesses/hermes-cli.toml:10`), but no 0.22.5 rows are committed. Fix: run full direct, gateway and dodger rows on 0.22.5 without the CA block. Make `table` refuse "verified" for any version listed in `version_notes`. If 0.22.5 is not green by Thursday, ship Hermes as "not verified" (multi-version plan, task 1).
5. **cr-scripts-1: signed-in pad rows run `direct` and `dodger` at once in one `GOALCUP_HOME`.** `scripts/pad/pad:1071-1074` runs them in parallel with no `max_workers`. `core-claude-code-signin.toml` and `core-codex-signin.toml` use `case = "all"` with a fixed `goalcup_home`, and HomeLock refuses the second run. Evidence: `pad-out/claude-code-cli/dodger/20261103T210247Z-pad-e9261e` shows "setup FAIL". The release check depends on these rows. Fix: run the cases one after another when `goalcup_home` is set, or refuse `case="all"` together with `goalcup_home` in `load_settings`.
6. **arch-x2 (covers cr-proxy-2, cr-proxy-1, cr-upload-server-3): there is no egress layer.**
   - `connector/launch.rs:19-31` strips the participant's `HTTPS_PROXY`/`ALL_PROXY` and their CA variables.
   - `proxy/connect.rs:252-285` always dials upstream directly.
   - `proxy/ca.rs:352` reads only the System Roots keychain.
   - `upload/client.rs:211-228` uses webpki roots and `try_proxy_from_env(false)`.

   So on a proxy-only or TLS-inspected network, the harness, the recording and the upload all fail, and nothing says why. Minimum for 1.0: `goalcup start` and `setup` print a plain refusal or warning when `HTTPS_PROXY`/`ALL_PROXY` was set. Add one guide line naming proxy-only networks. Fix `PARTICIPANT-GUIDE.md:324`, which says "Nothing to fix" when in fact the harness is blocked. Full fix after the event: one `net` module (saved upstream proxy plus System.keychain plus the participant's CA files), used by both the proxy upstream and `ServerClient`. Keep the token safe by only using a proxy the participant configured.
7. **docs-4: VISION promises two videos, while the participant docs name one.** `VISION.md:20` says there is an hour-24 setup video. `docs/RULES.md:407-410`, `README.md:68` and `PARTICIPANT-GUIDE.md:27,406` name only the demo video. Owner decision: either add the hour-24 video (deadline and where to submit) to RULES and the guide, or change VISION.

Release action, not code: until cr-watchdog-3 is fixed, run `scripts/gate.sh fast` on the owner's Mac with the Linux gates lifted before tagging.

## Fix soon (important, confirmed)

- **cr-watchdog-3: no macOS tests for watchdog, recover or start.** These are all `#![cfg(target_os = "linux")]`: `crates/goalcup/tests/watchdog_{end,null,recover,restart,rlimit,shield}.rs`, `cli_recover.rs`, `lessons_launch.rs` and `pipeline_stall.rs`. CI runs only `ubuntu-24.04`. The macOS `procs()` (`ps -A`) and tree-end paths have no automated coverage. Fix: make the `/proc` helpers portable (`kill(pid,0)` plus `cli::proc_start_id`), then add a `macos-14` CI job for `watchdog_end`, `watchdog_recover` and `cli_recover`. Note: this extends the lean CI on purpose, so the owner must decide.
- **cr-format-verify-4: forged wall times make the timeline allocate about 3e7 rows.** `crates/goalcup-verify/src/timeline.rs:59-60,143-148`. `last` is the max of the PART/LINK `wall_ns`, and nothing checks it against mono (`integrity.rs:507,620-645`). The timeline runs even after integrity failures (`integrity.rs:268,284`). Fix: cap the bucket count (for example 100k). Add a wall/mono consistency check and clamp `last`.
- **docs-7: the contributor path says data only, but a connector PR fails the gate.** `CONTRIBUTING.md:64-65,99` and `skills/adding-a-capture-harness/SKILL.md:13,29-30` say a harness PR adds no `.rs` file. But `crates/goalcup/tests/connector_builtin.rs:32-39` fails unless a line is added to `connector/builtin.rs`. Fix: replace `builtin.rs` with a `build.rs` that globs `connectors/*.toml` (arch-2). Or say plainly that one `entry!` line is the only allowed `.rs` change and that the contributor adds it.
- **versions-4: no per-version behaviour.** `connector/schema.rs:15-47` allows a single value per key. A breaking harness release means cloning the connector (`opencode-cli` vs `opencode2-cli` differ in about 6 keys), plus verifier arms in `goalcup-verify/src/usage/hints.rs`. Fix: add `[[versions]]` overlays, as in MULTI-VERSION-PLAN tasks 11-12 (1.1).

## Minor

Lanes marked (unverified) were not put through the verify pass. Every other line below was verified, and its severity was lowered to minor.

**cr-proxy**
- cr-proxy-1: corporate or MDM roots in `/Library/Keychains/System.keychain` are ignored (`proxy/ca.rs:61,352-366`). This is a known, documented scope limit; see arch-x2.
- cr-proxy-3: a learned refusal expires after 600 s (`proxy/hosts.rs:19`), so a client that never trusts the run CA loses one connection every 10 minutes, and NoAlert closes are never learned. Back off after the second refusal. Note Crush on macOS in `docs/harnesses/crush-cli.md`.
- cr-proxy-x1: a known secret that the model echoes across SSE deltas is stored raw (`mask/rules.rs:565`). This is a documented, accepted loss (`PARTICIPANT-GUIDE.md:413-415`). Optional fix: run known-value matching over the joined deltas.
- cr-proxy-4 (unverified): the upstream TLS handshake has no deadline (`proxy/mod.rs:289,320`). Bound it at 60 s.
- cr-proxy-5 (unverified): null mode still parses and decodes every body (`proxy/mod.rs:390`). Pass parser `None` when the recorder sends nothing.

**cr-watchdog**
- cr-watchdog-1 + arch-x1: the respawn loop `watchdog/mod.rs:568-581` retries forever and stops watching the harness. Recorder restarts re-exec `current_exe` by path (`mod.rs:269`), so a forced upgrade mid-run mixes versions. Cap the retries, keep reaping the harness inside the loop, and spawn from a pinned copy or fd. Note: MULTI-VERSION-PLAN section 8 calls this "critical". Verification found the install guard (`install.sh:118-170`) blocks normal upgrades, so it is minor.
- cr-watchdog-2: after `goalcup recover`, the isolated home holds no HomeLock (`cli/recover.rs:195`, `watchdog/recover.rs:136-180`). Take the lock in `recover::run` for the life of `watch()`.
- cr-watchdog-5 (unverified): the OpenCode prefix pin is skipped when `--version` cannot be read (`connector/mod.rs:242`).
- cr-watchdog-6 (unverified): a closed terminal makes `println!` panic in `hand_over` (`cli/start.rs:138-143`). `shield::hangup_seen()` is never called.
- cr-watchdog-7 + arch-10 (unverified): `build` and `server_time` are always null in the start event (`cli/start.rs:375,382`, `pipeline/mod.rs:706,712`). Fill them or drop them.
- cr-watchdog-8 (unverified): orphans adopted by the Linux subreaper stay zombies until the end (`watchdog/spawn.rs:156`). Add `waitpid(-1, WNOHANG)` per tick.
- cr-watchdog-9 (unverified): the start-line size test does not cover 256-byte values or a `--hosts` file (`watchdog/mod.rs:1084`). Check against MAX_LINE before spawning.

**cr-upload-server**
- cr-upload-server-1: there is no storage ceiling or `stored_bytes` alert, and no rate limit on authenticated routes (`server/worker/src/chunks.ts:112-119`). This is an accepted trade-off. Add a cron alert, plus a ceiling on practice and test runs.
- cr-upload-server-3: `ServerClient` ignores the proxy and OS roots (`upload/client.rs:213-228`). This is deliberate, and it fails loudly. Add the guide line; see arch-x2.
- cr-upload-server-5: the absolute workspace path is sent and stored (`cli/approve.rs:131`, `runs.ts:142`) but is missing from the "What the server stores" list in the Worker README. Docs only; RULES already discloses paths.
- cr-upload-server-6 (unverified): expired `device_requests` rows are never purged (`sql/goal-cup-tracer-upload.sql:77`).
- cr-upload-server-7 (unverified): a lost poll reply loses the minted credential (`device.ts:103`). At least reword the 410.
- cr-upload-server-8 (unverified): a stored-chunk retry does not compare sha256, prev or first_seq (`chunks.ts:48`).
- cr-upload-server-9 (unverified): a 600 s PUT bound on 32 MiB chunks, with no shrink, starves slow uplinks (`upload/client.rs:18`).
- cr-upload-server-10 + docs-6 (unverified): the dead `dev` field and `disk_full` notice remain in the Worker and `UPLOAD-API.md:37,174-177`, and the credential row is not shape-checked (`db-supabase.ts:206`).
- cr-upload-server-11 (unverified): UPLOAD-API drift (`docs/UPLOAD-API.md:11`): "1.0 needs no SQL", 429 scope, Content-Length requirement.
- cr-upload-server-12 (unverified): `delete-test-runs.sh:118` gives destructive retry advice, and deletes DB rows before R2.

**cr-format-verify**
- cr-format-verify-1: the honesty checks trust the record's own host labels. The verifier never reads `model-hosts.toml`, and nothing checks `hosts_sha256` (`goalcup-verify/src/honesty.rs:260,381`).
- cr-format-verify-2: plain `check` runs no secret scan and emits no `skip` (`report.rs:40`). Emit `secrets: skip` or scan shapes by default.
- cr-format-verify-3: warns are hidden from the text report (`report.rs:136`). Filter `>= Res::Warn`.
- cr-format-verify-6 (unverified): malformed JSON in record bodies is read as `{}` (`integrity.rs:408,546,657,779`).
- cr-format-verify-7 (unverified): the SSE line buffer in secrets is O(N·L) and unbounded (`secrets.rs:366`).
- cr-format-verify-8 (unverified): `wire_keeps_up` is cumulative, so early wire surplus hides later lost calls (`legs.rs:235`).
- cr-format-verify-9 (unverified): unchecked add on MASK offsets (`secrets.rs:402`). Use `saturating_add`.
- cr-format-verify-10 (unverified): Hermes DB copies are opened without `trusted_schema=OFF` or defensive mode (`usage/sqlite.rs:288`).
- cr-format-verify-11 (unverified): part and index files take the umask mode (`goalcup-format/src/part.rs:289`). Set 0700/0600 explicitly.
- cr-format-verify-12 (unverified): recovery deletes a part file whose `.idx` is missing (`part.rs:343`). Rename it to `.orphan` instead.
- cr-format-verify-13 (unverified): no mutation tests for the PART chain rules or forged LINK walls, and no fuzz target (`tests/check_invariants.rs:225`).

**cr-scripts**
- cr-scripts-2: `install.sh` installs on INSTALL_UNSURE (`packaging/install.sh:157-176`). This is documented. Refuse only when status hangs or a 1.0+ binary prints JSON it cannot read.
- cr-scripts-3: `table --release-check` has no tests (`scripts/pad/table:128`). Add `test_table_release.py`, which covers the tiers diff from cr-scripts-x1.
- cr-scripts-4 (unverified): `release.yml:56` publishes any `v*` tag that matches Cargo.toml, wherever the commit is. Add a `merge-base --is-ancestor` check.
- cr-scripts-5 (unverified): the wget fallback has no `--https-only` or TLS floor (`install.sh:67`).
- cr-scripts-6 (unverified): the PATH hint is wrong for fish (`install.sh:222`).
- cr-scripts-7 (unverified): tar extraction with no filter on Python 3.11.0-3.11.3 (`scripts/pad/pad:153`).
- cr-scripts-8 (unverified): `queue.json` is rewritten in place (`pad:256`). Use tmp plus `os.replace`.
- cr-scripts-9 (unverified): `gate.sh:55` runs `sudo -n dnf install` silently.
- cr-scripts-10 (unverified): `release.sh prepare` leaves a half-changed tree on non-cargo failures (`release.sh:104`).
- cr-scripts-11 (unverified): `fetch-run.sh:202` writes the curl `-K` config without escaping.
- cr-scripts-12 (unverified): the pad pushes untracked files from `scripts/pad` into the box (`pad:684`).

**arch**
- arch-1: the null-mode retry cuts every live harness connection (`watchdog/mod.rs:795-808`, `pipeline/mod.rs:995`), and `docs/ARCHITECTURE.md:95` says the opposite. Fix the doc now, and drain before exit later.
- arch-2: per-harness knowledge sits in 5 places. The verifier hard-codes connector ids (`goalcup-verify/src/usage/hints.rs:30-133`, `usage/native.rs:92-95`). `connector/builtin.rs:8-25` is a hand-kept list.
- arch-4: the module graph is a full mesh, and `cli/` acts as the utility library (`pipeline/mod.rs:53,58`, `upload/beat.rs:38`).
- arch-5 (unverified): the recovered watchdog runs as the hidden `goalcup recorder` subcommand (`pipeline/mod.rs:799`).
- arch-6: harness version is encoded five ways, and `opencode2-cli` duplicates a whole connector (`connectors/opencode2-cli.toml`).
- arch-8 (unverified): the verifier `include_str`s `../../goalcup/src/mask/shapes.tsv` and copies masker constants (`goalcup-verify/src/secrets.rs:22-32`).
- arch-9 (unverified): `scripts/check-links.py` plus its test (3,021 lines) are run by nothing.

**usability**
- usability-2: `--harness` takes only the exact `-cli` id, and no error lists the valid ids (`connector/mod.rs:156-157,200-204`).
- usability-3: after a Ctrl-C once the run is open, the message says to run `start` again, which replaces the run. The right step is `continue`. Also, `first-message.txt` is never written (`cli/text.rs:281-286`, `approve.rs:157-159`).
- usability-4: no command reprints the first-message path once the TUI has painted (`cli/start.rs:402-428`).
- usability-6: window refusals arrive after browser approval, with a ".." double stop and a lowercase fragment (`approve.rs:124,141`, `text.rs:288`, `db-local.ts:115-116`).
- usability-7: `repo_not_empty` does not point to `goalcup continue` (`text.rs:308-312`).
- usability-8: PROBLEM lines with no action: Null (`text.rs:535-540`) and `no_request_yet` (`text.rs:620`).
- usability-9 (unverified): no lid or battery warning when `caffeinate` exists (`cli/awake.rs:26`).
- usability-10 (unverified): setup prints "Ready." after writing nothing with no tty, and never checks sign-in (`setup.rs:79`).
- usability-11 (from notes): a wrong OpenCode pick says "install that version" instead of naming the sibling id (`text.rs:176-182`). This is plan task 6.
- usability-13 (unverified): clap errors are cut to one line, which drops did-you-mean. `--test`, `--connector` and `--hosts` show in help (`main.rs:145`).
- usability-14 (unverified): `time-left` outside the harness says "unknown" between parts (`time_left.rs:43`).
- usability-15 (unverified): `continue` does not check the workspace folder (`continue_run.rs:45`).
- usability-16 (unverified): jargon, and three product names, in terminal text (`text.rs`).

**oss**
- oss-1: `NOTICE:9-13` cites `spikes/*/fixtures`, which does not exist. It should name `mask_corpus`, `usage_fixtures` and `scripts/pad/fixtures`. `AGENTS.md:73` is stale. (The repo-name half is refuted: `goal-cup-proxy` is canonical and the old URL redirects.)
- oss-2: 1,846 `/Users/team` paths in `crates/goalcup/tests/mask_corpus`. `goalcup_home = /Users/team/gc-signin/home` in 3 pad run files. Emails and account ids are already scrubbed.
- oss-3: `docs/plans` (275 files, about 7.9 MB), `docs/research`, `docs/superpowers` and `docs/archive` are tracked. Move `pad-rows.jsonl` first: `scripts/pad/table:13` `ROWS_REL` reads it.
- oss-4: the staging `*.b-9a5.workers.dev` host is in `scripts/pad/pad:30` and 6 docs. There is a single code owner. Cosmetic.
- oss-6 (unverified): no CHANGELOG, SUPPORT or cargo-deny.
- oss-7 (unverified): release artefacts are unsigned. Post-event.
- oss-8 (unverified): add a secret-scanning allowlist for test constants.

**docs**
- docs-1: "Nothing is pinned" (`docs/CONNECTOR-SCHEMA.md:7`) sits next to the OpenCode prefix refusal. `PARTICIPANT-GUIDE.md:464` says "install that version" with `<version>` = "1.".
- docs-2 (from notes; = versions-8): `RULES.md:207-209` says goalcup "samples the version, records a change as an event", but nothing does that. `RULES.md:345` machine facts vs `os`+`cpus` only. This is plan task 7.
- docs-3: broken anchors `#codex`, `#claude-code` and `#amp` (`docs/harnesses/{codex,claude-code,amp}-cli.md`). The stale `TODO.md:51`. `check-links.py` does check anchors but nothing runs it.
- docs-8 (unverified): `LOCAL-RESEARCH-HANDOFF.md` and `RESEARCH-TARGET.md` are history that sits at the top level.
- docs-9 (unverified): platform lists disagree (`RULES.md:259` vs `README.md:171-176`). `--harness-files` is missing from the README.
- docs-10 (unverified): participant docs repeat each other. The seven points are copied word for word in four files.
- docs-x2 (unverified): the repo-name half is refuted (see oss-1).

**versions**
- versions-1: `tier_line` does not strip a leading `v` (`cli/start.rs:442-443`), so a Hermes or OpenCode 2 version parses to None. This is latent while every tier is "not verified".
- versions-2: five version parsers disagree. Copilot "1.0.105." would print "not on 1.0.105.". Plan task 2.
- versions-3: tiers hold one exact build per harness, compiled in (`scripts/pad/table:29-31,79-88`).
- versions-5: see versions-x2 above.
- versions-6: a fresh Hermes home cannot read the version (`connector/launch.rs:148-171`). Interactive setup warms the home, so the impact is limited to `setup --yes`. Plan task 5.
- versions-10: nothing checks that the env and settings keys still take effect after an update. `native_missing` already covers the home redirects.
- versions-11 (unverified): Amp is named in 3 versions and the Hermes header says 0.22.5. Version facts are comments, not data.
- versions-12 (unverified): the "run any installed version" pad change does not apply to OpenCode (`scripts/pad/harnesses/opencode-cli.toml:13`).
- versions-x3 (unverified): tiers have no OS dimension, and every row is darwin-arm64 (`table:82`, `text.rs:1158`).

## Refuted

- cr-upload-server-2 (GitHub rate limit at open): the cron makes no GitHub calls until a week after starts. 2 calls per open over a 24 h window is far under 5000/h.
- oss-5 (no stranger path): a facts-only PR is already complete (`CONTRIBUTING.md:69-71`). The pad micro mode runs locally with no staging. The README opens with a tool description and lists the gate prerequisites.
- docs-5 (DECISIONS pruning, wrong D74 citation): D74 does define the evidence labels (`DECISIONS.md:1220-1224`). Keeping superseded entries is the stated policy. The shrink is already tracked at `TODO.md:85`.
- docs-x1 (no doc defines versions): `CONNECTOR-SCHEMA.md:22-23`, `RECORD-FORMAT.md:510` and `PARTICIPANT-GUIDE.md:96,464` define the field, the recording and the mismatch. The facts are spread out but do not conflict.
- oss-1, repo-name half: `goal-cup-proxy` is canonical (`DECISIONS.md:330`), and the old URL redirects. The NOTICE half stands.
- cr-watchdog-1, Linux ENOENT path: `current_exe` is resolved once to the real path, and `install.sh` moves the new binary to that same path, so a respawn after an upgrade succeeds.
- cr-upload-server-5 "privacy promise broken": RULES already discloses that paths are uploaded. Only the Worker README list is incomplete.

## Architecture

From `arch.md`. The core shape is sound: the watchdog holds the port, relay comes before recording, one writer and the masker run in one recorder, and the Worker only stores bytes. Top simplifications, all cut-first:

1. **Connector data as the single source (arch-2, arch-6, docs-7, versions-4).** `build.rs` globs `connectors/*.toml`, replacing `builtin.rs`. A `[usage]` table (thread headers, join, thread_db) replaces the id literals in `goalcup-verify/src/usage/hints.rs`. `[[versions]]` rows replace `version`, `version_notes`, the pad pin and the header comments. This is the structural change multi-version needs.
2. **One data-only crate or folder for shared tables (arch-8, cr-format-verify-1).** Put `connectors/*.toml`, `model-hosts.toml` and `shapes.tsv` where both binaries read them. Then the verifier can recompute host labels, and stops `include_str`ing into the recorder's source.
3. **Pull out leaf modules `run` and `ctl` (arch-4).** `run` holds paths, JSON, log and process identity. `ctl` holds ToRecorder/ToWatchdog, StartFacts and SealReason. This is mostly a file move, after 1.0.
4. **One `net` module for egress (arch-x2).** The same proxy and roots for the proxy upstream and for `ServerClient`.
5. **Cut dead weight.** Delete `scripts/check-links.py` and its test, or wire one into the gate (arch-9, docs-3). Remove the `dev` field, `disk_full`, the `abandoned` status, and the always-null `build`/`server_time` (arch-10, cr-upload-server-10). Give the recovered watchdog its own hidden subcommand (arch-5).
6. **Failure domain (arch-9 in notes, post-event).** The proxy, masker and uploader share the recorder process, so any recorder restart drops every in-flight harness stream. Consider keeping the relay alive across recorder restarts.
7. **Server DB (arch-5 in notes).** Integration tests run only on `db-local.ts`. The production SQL atomicity in the 4,468-line mirror is never tested here (cr-upload-server-4: run the real SQL under the same scenarios).

## Usability

The happy path is honest and plain. The dead ends that cost a first run are listed in priority order:
1. Missing harness found only after the run opened (usability-1, release).
2. README `--prompt` example (usability-5, release).
3. Wrong next step after Ctrl-C, and `repo_not_empty`, both point away from `goalcup continue` (usability-3, -7). Both are text fixes.
4. `--harness claude` gets no id list (usability-2). List the ids and accept the program name as an alias.
5. Window refusals come after the browser approval, with broken punctuation (usability-6).
6. PROBLEM lines with no action (usability-8), and the first-message path is never shown again (usability-4).
7. Every harness prints "not verified" with no next step (tiers seed). Say plainly that recording does not depend on it.

## Open-source readiness

Legal files, CI hygiene and secret hygiene are solid, and no real credentials were found. Before going public:
1. Scrub `/Users/team` from `mask_corpus` and the signin run files. Add a CI grep for `/Users/` and `/home/` (oss-2).
2. Move `docs/plans`, `docs/research`, `docs/superpowers` and `docs/archive` to an archive branch, after relocating `pad-rows.jsonl` and `ROWS_REL` (oss-3).
3. Fix the contributor gate failure (docs-7) and the NOTICE paths (oss-1).
4. Replace the staging host with a placeholder or an env var (oss-4).
5. Later: CHANGELOG, SUPPORT, cargo-deny, signed releases, a second code owner (oss-6, -7).

## Docs

Every number spot-checked against code matched. Fix before 6 Nov:
1. docs-4: decide on the videos.
2. docs-1 and docs-2: version truth. State that `version` is an OpenCode major-prefix guard. Rewrite `RULES.md:207-209` to "read at start, continue and seal". Fix `RULES.md:345` machine facts.
3. docs-7: the contributor path.
4. Correct `ARCHITECTURE.md:95` (null retry cuts connections, arch-1).
5. Correct `PARTICIPANT-GUIDE.md:324` (company proxy, arch-x2).
6. Add the proxy-only network line.

After that:
- docs-3: anchors, plus running `check-links.py`.
- docs-6: the `dev` field.
- docs-8: history docs out of `docs/`.
- docs-9: one platform table.
- docs-10: one home per fact.

## Multi-version

Full plan: `docs/superpowers/plans/2026-11-04-v1-fixes/review/MULTI-VERSION-PLAN.md`. Lane map: `versions.md`.
- Today: one exact build per harness in a compiled-in seed `tiers.toml`. Six version parsers disagree. Behaviour has one value per connector, so breaking releases are cloned (OpenCode). Hermes "passes" only with a pad-only workaround.
- Goal: any installed version runs and is recorded. One true line about the participant's exact build and OS. "Verified" is computed from reproducible pad rows. One connector per program, where the version picks the rules.
- Now and before Friday (tasks 1-8, all S except task 1): Hermes 0.22.5 truth rows. One `connector/version.rs` parser with a shared fixture. Tier line, setup and probe fixes. Release-check diff of tiers. Sibling-id mismatch text. Docs. Canary script and runbook for 7-15 Nov.
- 1.1 (tasks 10-17): `[[versions]]` overlays, the OpenCode fold, start-event version fields and a seal read, a (version, os) tiers list, verifier "checked on", a pad version matrix with Linux rows, redirect effect checks.
- Owner decisions (section 9): server `notes` go or no-go; Hermes "not verified" vs dropping it from CORE; installer refusal or a hotfix ban during the event; who runs the canary; RULES wording.

## Lane verdicts

| Lane | Verdict | One line |
|---|---|---|
| cr-proxy | With fixes | The hot path holds points 3 and 4. The gaps are corporate CA, upstream proxy, and the refusal re-probe. |
| cr-watchdog | With fixes | The lifecycle is carefully built. Remaining: respawn wedge, unlocked home after recover, no macOS coverage. |
| cr-upload-server | With fixes | The protocol and credentials are solid. Remaining: proxy/trust roots, a storage alert, real SQL tests. |
| cr-format-verify | With fixes | The format is correct. The verifier needs canonical host labels, an explicit secrets skip, visible warns, and a bounded timeline. |
| cr-scripts | Ready with fixes | Release and install are well tested. Remaining: the pad signin clash, release-check tests, the tiers diff. |
| arch | With fixes | Sound shape. Consolidate connector and version data. Fix the null-retry docs and tiers. |
| usability | With fixes | Honest happy path. First-day dead ends can cost a participant's only run. |
| oss | With fixes | Legal and CI are fine. Scrub personal paths, move internal plans, fix the contributor path. |
| docs | With fixes | Accurate on numbers. Fix the version-support text, the RULES sampling claim, the videos, and the contributor gate. |
| versions | With fixes | One pinned build per harness. Needs one parser, overlays, a version-list tiers file, and truthful pad rows. |
