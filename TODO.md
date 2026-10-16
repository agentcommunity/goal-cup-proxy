# TODO

The single tracker. Done means: the fast gate passes, one reviewer reads the PR once, and README or
this file is updated if behaviour changed ([AGENTS.md](AGENTS.md)). Ticket IDs below (F-04, H-20,
...) are names only. [CHEAT-CHECKLIST](docs/CHEAT-CHECKLIST.md) uses the same names.

## Dates (UTC)

| When | What |
|---|---|
| ~23 Oct 2026 | First tagged release, so dry runs use a real build. More releases follow as needed |
| From the first release | Dry runs whenever everything runs; 10-minute tests any time. No fixed dry-run date |
| 4 Nov 00:00 | Feature stop: after this, fixes only |
| 6 Nov 12:00 | Last release, including the last connectors (connector cut-off) |
| 7 Nov 00:00 | Start window opens. It lasts 24 hours. Each run then has 168 hours |

Owner decisions of 16 Oct 2026:

- Each participant's prompt becomes public at the latest at the end of the event.
- The second-harness rule stays: harness A must not start harness B (D10).
- Support contact: goalcup@agentcommunity.org.
- Windows means WSL2 for the event ([README](README.md#windows-use-wsl2)). Native Windows later.
- Security contact: goalcup@agentcommunity.org too.
- Emails: no throughput work here. The website is building one central mail system (hundreds of
  mails every 5 seconds); the Worker moves to it later.
- The lead agent applies production database migrations and Worker deploys (additive only, existing
  storage intact). The owner approves terminal prompts.
- The program version comes from one place and updates everywhere automatically (WP2).
- Install works on macOS, Linux and WSL2 with one command; Homebrew is not the main path (WP2).
- Time: the server is the clock. Every heartbeat answer carries the time left; the tracer counts
  down from that and never trusts the machine's own date (WP11).
- A harness is a plug-in module: its folder carries every harness-specific setting, and tests and
  release discover it. No harness name by hand anywhere else (WP5, WP12).
- The larger clean-ups run now, by parallel agents on separate files, before the feature stop:
  the first release has not made any compatibility promise yet.
- Demo and questionnaire: due 24 hours after each run's 168 hours end (a later start gets a later
  due time). They go through the website portal, not the tracer; the website may close it leniently.
- Harnesses: 15+ expected, Hermes Agent among them. All shipped connectors go to participants.
  Harnesses can still be added until the last release (6 Nov 12:00 UTC): the builder sends a
  practice-run verifier report, and the organizers run a 1-hour test run before it ships
  ([HARNESS-ONBOARDING](docs/HARNESS-ONBOARDING.md#getting-a-harness-into-the-event)). For testing,
  the owner has API access for harnesses whose model provider is free to choose; for a harness whose
  model comes only through its own subscription, ask the owner or the builder.
- The install command goes into RULES and the participant guide (WP2).
- Research data is stripped of personal and entity information (people, organisations, emails,
  accounts, repo and host identifiers) before any research group gets it. Researchers never get
  access to our database or R2; each group gets a cleaned export through custom, narrow access
  (clean-up 14). The design doc is kept privately; it is built after the event.
- No keyed runs: the tracer never sees, forwards or stores a provider key (D61). Every harness,
  Hermes included, keeps its own login in its home, and every run uploads under the same rules.
  #208 redacts the run's own proxy credential by exact value; #214 runs Hermes in chat mode from
  its own home; #215 removes the key-forwarding mechanism.
- Amp ships. Its metadata log is a native source, labelled as such. #206 waits for the owner's live
  run (the thread log is not yet measured in chat mode).
- Claude Code's folder-trust dialog: the participant answers it. The RULES exemption for that one
  answer is still to be written (UX-F).
- Every start needs a human click in the browser, also when it replaces a run. The test approval key
  exists on staging only.
- Wire-only core (a connector with no native leg) is deferred: not before 4 Nov.

## Work to the feature stop (4 Nov), in order

Review 12 (16 Oct) produced this list from six code reviews and three test reviews. Each item names
its source report, e.g. [C4-F4]; the reports are kept outside the repo (maintainer's machine,
`goal-cup-review12/`). Line numbers are at `3722360`, so re-grep before you edit. `cli/` means
`crates/goalcup-cli/`, and the same for the other crates. One PR per package; `sh scripts/gate.sh
fast` before every commit.

### WP0: Measure
Status (16 Oct): first real runs on macOS against staging (0.6.0). Codex (luna) recorded, stopped,
sealed and uploaded. Claude (haiku) stopped at the folder-trust dialog. Left:
- Mac Codex and Claude matrix rows still have to pass. They are blocked on a Little Snitch rule for
  the new build.
- Linux Codex sign-in needs the owner (Chrome must be signed in to OpenAI, or the owner enters the
  device code). Linux Claude and Amp need owner logins.
- Then: the 8-hour runs on macOS and Linux, Intel Mac, x86_64 Linux and WSL2, and the edge cases in
  [OPERATING](docs/OPERATING.md). The matrix is `scripts/matrix.sh` (Linux: `scripts/linux-matrix.sh`, #198).

WP0 findings (16 Oct, first real runs). The owner calls these real mini bugs to clean up. Numbers
are the order found.
1. Login approval page said "harness: none". Fixed: website #307.
2. Work folder inside Git refused late, path named twice, rule code in the text. Fixed: #204.
3. Two browser approvals look odd. Kept by design (login stores no credential; each run mints its own).
   Pages and terminal say why: #307, #205. The guide section is open (UX-F).
4. Terminal text not readable. Fixed: #205 (run), #209 (status, upload, entries).
5. The guide must say two approvals are normal. Open (UX-F).
6. "By this machine's clock" was stale. Fixed: #201, #205.
7. Codex prompt typed, not sent. Matrix fixed: #200. The launch text now says "press Enter": #205.
8. Test key still opened the browser. Fixed: #200 (`GOALCUP_NO_BROWSER`).
9. Already-approved code showed "Unable to load this request". Fixed: website #307 (one retry, plain states).
10. Codex stopped about 3 minutes after the deadline. Fixed: #201.
11. Claude folder-trust dialog on every run. Participant answers it. Hint in the manifest (#203) and
    launch text (#205); the matrix answers it (#200). The RULES exemption is open (UX-F).
12. Matrix expected `deadline:` on the terminal. Fixed: #200.
13. `seal_git_unreadable` after a long proxy wait. Fixed: #199.
14. Connections without host or bytes. By design (tunnels); the admission line now says so: #199.
15. Codex ids, credit balance and turn state kept clear. Fixed: #202 (plus the proxy credential rule).
16. `install.sh` ended "Next: goalcup doctor". Fixed: #204.
17. Silence during the clone and the wrap-up upload. Fixed: #205.
18. Launch line full of ids and paths. Fixed: #205 (details go to host.log).
19. Claude "No, exit" then `goalcup continue` may fail with no session. Open: measure in the matrix.
20. RULES forbids clicking after the start; the trust answer is one. Open (UX-F).
21. Status had no end time, time left or mode. Fixed: #209.
22. Second start of the same harness advised `goalcup seal`. Fixed: #205.
23. Help was developer-facing. Fixed: #204.
24. Page buttons "Approve/Deny" for every purpose. Fixed: website #307.
25. Replace warning had no start time. Fixed: website #307 (migration applied to prod), SQL mirror #207.
26. Prep opened the harness with no pause. Fixed: #204.
Also open: Amp (#206, waits for the owner's run). Keyed runs are gone (D61, #214, #215).
Participant-guide text is one pass at the end (UX-F): approvals, trust dialog, Little Snitch.

### WP1: Event window and event switch (server). Land first.
Status (16 Oct): done. #165 and website #305 merged; production migration applied (cup 7-8 Nov 00:00 UTC, dry run 23 Oct to 6 Nov 00:00); Workers Logs on with invocation logs off (#169); deployed to prod and staging. Left: the 6 Nov `EVENT_ID` switch (runbook in `server/worker/README.md`).
- The events row is 16 h early. `goalcup-cup-2026` opens 6 Nov 16:00 UTC. Add a new website migration (agentCommunity_PAGE `supabase/migrations/`) that sets opens `2026-11-07T00:00:00Z` and closes `2026-11-08T00:00:00Z` (staging=false), and opens the dry-run row from the first release to 6 Nov 00:00 UTC, so a dry run can start any day. Copy it into `server/worker/sql/goal-cup-tracer-upload.sql`. Fix the stale comment at `:1788-1789`. Drop `[CONFIRM]` at `server/worker/README.md:236-239`. [C6-01, docs]
- The `EVENT_ID` switch: `wrangler.toml:37-40` still says `goalcup-dryrun-2026`. Write a dated runbook step: "~30 Oct: set `goalcup-cup-2026`, `npm test`, `npx wrangler deploy`, open one practice and one test run, check the R2 prefix `traces/goalcup-cup-2026/`." Do it on 6 Nov with the last release. [C6-02]
- Turn on Workers Logs: `[observability] enabled = true` at top level and in `[env.staging]`, pinned in `test/config.test.ts`. [C6-04]
- Why: the wrong window refuses real starts after 7 Nov 16:00 UTC. A missed switch writes cup runs outside the 180-day bucket lock.
- Accept: `select id,start_window_opens,start_window_closes from goal_cup_tracer.events` shows the new values. `npm --prefix server/worker test` passes.

### WP2: Release pipeline shakedown. Start 17 Oct; four runners are untested.
Status (16 Oct): code done (#164: 0.6.0 single source, `install.sh`, every connector ships, `release.sh prepare`/`tag`, no Homebrew). Left: the first real tag and publish (WP10).
- Version, one source: workspace `Cargo.toml:13` says `0.1.0` while tag `v0.5.0` exists. Make the workspace version the only place a version is written; every crate, binary, report field (`verifier_version`, `tracer_version`, `DAEMON_VERSION`), the Worker's expected tracer version if any, the install script and the release notes read it. `scripts/release.sh <x.y.z>` sets it, updates `Cargo.lock`, commits and tags; `release.yml` refuses a tag that differs from it. Docs never write a version number by hand (they say "the latest release"); a gate test fails on any other hard-coded version string in the repo. Start at `0.6.0`. Also fix the stale `v0.5.1` at `:15` and "dev-only signer" at `:82`. [C5-F5, C6-05, C6-15, docs]
- Ship every connector. `scripts/package-release.sh:38` archives only codex-cli and claude-code-cli. Archive all of `connectors/`, with no list of names (Hermes ships too; H-21 is closed). Fix the comment at `:11-13`, `packaging/README.md:28,38-39`, and SKILL step 12 ("ships once merged"). [C3-F2, C6-06]
- Run `gh workflow run release.yml` by hand now. It publishes nothing. Fix what fails (`macos-15-intel`, `ubuntu-22.04-arm`, `third-party-licenses.py`). [C6-05]
- Install for every platform: an `install.sh` published with each release (`curl -fsSL https://github.com/agentcommunity/goal-cup-tracer/releases/latest/download/install.sh | sh`). It picks the build for macOS arm64/x86_64 or Linux arm64/x86_64 (WSL2 is Linux), checks the SHA-256, and installs to `~/.local/bin`. A download by `curl` is not quarantined on macOS, so unsigned builds run. Drop the Homebrew tap from the plan (maybe later, Mac-only nicety). Native Windows later gets an `install.ps1`. Update README, RULES, PARTICIPANT-GUIDE and the setup skill.
- Put the Worker tests in the gate: add `step npm --prefix server/worker test` in the `full` block of `scripts/gate.sh:89`, and assert it in `scripts/test_gate.py::test_full_runs_everything`. Keep it out of ci.yml, because CI stays lean. [C6-03, T3-F2]
- `.github/CODEOWNERS`: add `/server/ @nembal`. In `ci.yml:8-10,28-30`, drop `v1/trunk`. [C6-11, C6-15]
- Accept: the manual release run is green on all four targets, and `install.sh` installs it on macOS and Linux. The tarball holds every non-excluded `connectors/*`. `python3 -m pytest scripts/test_gate.py` passes.

### WP3: Participant-facing CLI and daemon fixes
Status (16 Oct): done (#159, #158).
- `goalcup status` with no id can mix two runs. `cli/src/commands/status.rs:154-158` takes the newest run, while `:221-224` pins the newest entry. Use `crate::run::of_entry(cfg, &record.entry_id)`. [C4-F4]
- Keep one definition of "run over". Replace `commands/start.rs:2472-2478` with `!on_server || crate::run::is_over(&record, Some(&keeper.snapshot()), now_unix())`. [C4-F1]
- `goalcup status` tells participants "nothing is uploaded" during an uploaded run. Reword `daemon/src/status.rs:118-121`. [C2-F6]
- Help text leads with `--goal-file`, which both chat connectors refuse (`main.rs:52`, `status.rs:227`). Also fix the config-dir table at `config.rs:13-22` (`grant.json`, "participant's token"). [C4-F5, C4-F13]
- `goalcup doctor` exits 1 for harnesses the participant never installed. Default to the harness in `prep.json` (`commands/doctor.rs:84-95`), and keep `--harness all`. Drop the unused `_err` (`:66,74`). [C4-F6]
- Operator errors: name the connector folder in `load_child` errors (`daemon/src/runtime/capture/harness.rs:28-29,103-116`). `eprintln!` reopen failures (`runtime/capture/mod.rs:771-790`, `CycleCounters::note`). Fix "three unauthenticated routes" (`config.rs:324,339-340`, `runtime/daemon.rs:94`). [C2-F4, C2-F5, C2-F10]
- Optional: `goalcup upload` with no id sends the newest run's unsent entries (`main.rs:208-212`). [C4-F7]
- Accept: a new `it/status_views.rs` case (practice, then competition, then continue the practice run). `gate.sh fast`.

### WP4: Deadline and long-run tests in chat mode
Status (16 Oct): done (#175).
- Server deadline in chat mode: `Rig::chat()` with `short_server(…, 8, …)` in `cli/tests/full/upload_e2e.rs`. The TUI stops, the terminal comes back, and the run seals and uploads. [T2-F2a]
- A continued segment: crash segment 0, then `continue`. Segment 1 must stop on the server's `stop`, and with the server away it must stop on `LocalDeadline::until` (`upload/uploader.rs:1420`, which has zero tests). Add a skewed-clock unit test. [T2-F2b, T2-§5]
- Real part rotation through the CLI: `set_chain_part_limits_for_test(4096, 1h)` plus `GOALCUP_TEST_DISK`, then prune, kill, continue and finish. Every record reaches the server exactly once. [T2-F3]
- A table test for the WSL browser opener (`platform/src/machine.rs:380-400`). [T2-F14]
- Accept: `cargo nextest run --offline -p goalcup-cli --test full upload_e2e` passes, and `scripts/linux-check.sh` passes.

### WP5: Harness onboarding for #4..#15. Land before the connector cut-off.
Status (16 Oct): done (#160, #172).
- The template and skill build headless (Hermes) connectors, but the product is chat mode (D57). Add a chat-mode block, on by default, to `skills/adding-a-capture-harness/templates/connector-harness.toml:14-22,41`. In SKILL `:71,:102`, point at codex-cli. The facts sheet starts in chat mode and asks for the CA variable and every provider host. [C3-F1, C3-F7, C3-F13]
- Chat-mode kit: replace the id list at `cli/tests/full/harness_contract_kit.rs:1069` (and the match at `:1005`) with every pinned `interactive = true` connector. Pass `KIT_NATIVE_FILE`, derived from `[[capture.native]]`. Drop the per-harness branches in `tests/fixtures/m2-fake-chat-harness.sh:18-33`. [C3-F1, T2-F1]
- Find connectors by discovery, not hard-coded ids. One `shipped_connector_ids()` helper replaces the lists at `cli/tests/shipped_connectors.rs:95,143,220,258,544` (compare `:258` to the pins file), `protocol/tests/it/harness_conformance.rs:35`, `full/preflight_doctor.rs:44,138,206,252` and `it/harness_launch_plan.rs:287`. Delete the per-harness recipe asserts (`shipped_connectors.rs:26-82`, `harness_launch_plan.rs:19-56`). Loop the generic rules over every connector. [T1-F4, T2-F1, T2-F12]
- The name ban must grow with `connectors/`. In `protocol/tests/it/no_family_literals.rs:19`, add the connector ids (strip `-cli`, match whole tokens). [T3-F9]
- `scripts/check-connector.sh:216-238` and SKILL `:159-160`: run the kit on Linux too, and skip only the daemon golden off macOS. Fix the header that says macOS only. [C3-F4, docs]
- `decision-table.md:25-27`: chat mode cannot pass an env-only API key. [C3-F6]
- Process: set a connector cut-off (~21 Oct). Pins are compiled in (`protocol/src/pins.rs:33`), so a later connector needs a point release. [C3-F3]
- Accept: a scratch chat connector under `connectors/` passes `sh scripts/check-connector.sh <id>` on macOS and Linux with no Rust edit. Remove it afterwards.

### WP6: Test infrastructure and flakes
Status (16 Oct): done (#168, #181, #194). The x86_64 workflow is green. `linux-check.sh` works from worktrees (#188, #191).
- Serial drift: add `launcher_sweep` to the cli serial regex (`.config/nextest.toml:41`). `TableLock::acquire()` panics when `NEXTEST_TEST_GROUP` is set and is not `serial` (`daemon/tests/common/support/table_lock.rs`). `capture_gate.rs:28` `ONE_AT_A_TIME` becomes `TableLock`. [T1-F2]
- Sleep races: use a ready-file handshake in `daemon/tests/full/capture_finish_budget.rs:767,810`, and poll the handler state at `:196`. [T1-F3]
- x86_64: the README says `gate fast` fails there because two timing tests time out. Fix both the same way (wait for the condition, not a fixed budget), then delete the README caveat. [docs, T1-F3]
- The fast tier runs the per-connector checks: the `scripts/gate.sh:57` filter admits `harness_contract_golden|harness_contract_kit|preflight_doctor`, and `scripts/test_gate.py:81` is updated. [T1-F1, T3-F6]
- Set `test = false` on the `goalcup` bin (`cli/Cargo.toml:118`) and the `goalcup-verify` bin (`verifier/Cargo.toml:40`), and add a daemon `[[bin]]` stanza with the same. [T1-F5]
- Accept: `gate.sh fast` passes on macOS and in `linux-x86_64.yml`. `scripts/linux-check.sh` passes.

### WP7: Safe deletes (pure deletion, before any of it becomes a compatibility promise)
Status (16 Oct): done (#161, #162, #167, #174), including the cross-family golden (#187). `hermes_offline_key.rs` stays (see below).
- Dead tests and fixtures: `daemon/tests/it/terminate_sim.rs:1307,1497`, `daemon/tests/fixtures/wrapper-exit-after-term.sh`, `protocol/tests/it/harness_schema.rs:3206-3219` with `tests/fixtures/harness/*.chat-mode.toml`, and the dead `harness_legacy_equal.rs` mention at `:13`. [T1-F7, T3-F8]
- The cross-family golden: the test at `cli/tests/full/harness_contract_golden.rs:942`, `cross-family.json`, `FAMILIES` (`:68-74`), `legacy_*` (`:685-706,905-940`), `streams_json` (`:512`), and the orphan `*.native.json` and `native-families.json`. Regenerate once with `GOALCUP_REGEN_HARNESS_GOLDEN=1` and review the diff. [C3, T2-F4, T3-F7]
- Daemon config: delete the 14 dead keys, `ReceiptBindingPolicy` and `submission_types.rs` (`config.rs:39-53,81-104,154-274`, `lib.rs:37`). Keep `deny_unknown_fields`. Update `tests/it/config_load.rs` and `docs/OPERATING.md:238-240`. [C2-F2]
- Daemon: `unread_stores` (`native_scope.rs:19,92,99`, `capture/mod.rs:1278-1280`) and `DAEMON_VERSION_TEXT` (`lib.rs:49-52`). [C2-F8, C2-F13]
- CLI: `CliError::Gate` and exit 4 (`lib.rs:117-121,141`) with the test at `contract_gate.rs:207`, `GateOutcome` (`arming.rs:34`, `start.rs:146,2681`), `run::state_path` (`run/mod.rs:600`), and the dangling link at `lib.rs:7`. [C4, T2-F8]
- Protocol: `ForeignScanRoot` (`manifest.rs:36-42`). Move the CLI ack writer into `export::append_ack_to`, then delete `export.rs:1349,1366,1375`. Swap the two `libc::flock` calls for `File::try_lock` (`start.rs:594`, `uploader.rs:362`). Platform uses `libc.workspace = true`. [C1-F11, C1-F5, C1-F3, C1-F15]
- Tests: keep `cli/tests/it/hermes_offline_key.rs`: it is the only real-Hermes P1 promise test, and Hermes ships (promise tests are never deleted). Delete the `upload_open.rs:1141` twin. Merge the two stale control-file tests (`contract_gate.rs:136`, `daemon_client.rs:412-433`) into one cross-crate test. Move `daemon/tests/it/contract_present.rs` into protocol `m2_contract.rs` so CI runs it. [T2-F10, T2-F9, T2-F7, T3-F10]
- Accept: `gate.sh fast` and `cargo build --offline` pass with no new warnings.

### WP8: Stale references in code comments and docs
Status (16 Oct): done (#157, #170, #173, #177, #178, #185, #186, #193 connector comments).
- `grep -rn CONTRACTS.md crates server` finds ~35 comments citing old section titles; update them. `uploader.rs:11,61` and `server/worker/src/chunks.ts:2` cite SERVER-DESIGN sections that no longer exist. `capture_policy.rs:90` cites a deleted Amp record, and `ack.rs:3` cites the deleted VISION.md. [docs]
- Comments in `connectors/*/harness.toml` cite missing docs. Fix them and run `sh scripts/pin-manifests.sh` in the same commit. This must land before the release cut, because pins are compiled in. [docs]
- Generic modules: `platform/src/lib.rs:1`, `filecap.rs:1`, `protocol/src/template.rs:3`, `manifest.rs:24-29`, `released_child.rs:9`, `handle_budget.rs:1,10`. [C1-F12]
- Worker README: delete `server/worker/README.md:117-168`, and fix `:12`, `:46-62` and `:206-208` (the cron has four steps). Add an operator runbook: maintenance, rollback, logs, the EVENT_ID switch, window changes, adding a test operator, test cleanup. [C6-07]
- Regenerate the SQL mirror with a one-line command written in its header. It is missing the staging-events migration, and `:10,17` cite a dead file. The retry advice at `delete-test-runs.sh:92` should say "run again without --older-than". [C6-10, C6-08]
- In `docs/OPERATING.md:362`, the verifier does pin the production key; drop `chain_broken` at `:338`. In `SECURITY.md`, replace goalhack@agentcommunity.org. [C5-F6, docs]
- Accept: `python3 scripts/check-links.py` passes. `grep -rn 'goalhack@\|VISION.md\|proposed-goalcup'` finds nothing. The pins test passes.

### WP9: Upload contract and verifier guards (organiser side)
Status (16 Oct): done (#166).
- SQL↔Worker parity test in `server/worker/test/supabase.test.ts`: every `#rpc` name and `p_` argument in `db-supabase.ts` exists in the SQL. [T3-F1]
- Shared signing vectors: commit `server/worker/test/vectors/signing.json`. `protocol/tests/it/signing_vectors.rs` asserts the same hex, and `sign.test.ts` reads that file. [T3-F3]
- In `verifier/tests/it/m2_capture_integrity.rs`, add the tamper-table test `line_level_tampering_names_its_finding`. Add a rotated-corpus test to `v5_corpus.rs`. [T3-F4, T3-F5]
- Deadline facts: in `verifier/src/capture_analysis/reader.rs` `Scan`, record `records_after_deadline`, `last_seq_at_or_before_deadline` and `last_ack_at`, with a schema bump. This is a fact, not an exit change. It may slip past the feature stop (the verifier is not on the participant path), but it must land before judging. [C5-F1]
- Add `findings: …` to the stderr summary (`bin/goalcup-verify.rs:109-113`; its pin is at `m2_capture_integrity.rs:2885`). Add a table of finding codes to OPERATING. [C5-F4, C5-F3b]
- Accept: `npm --prefix server/worker test` passes, and so does `cargo nextest run --offline -p goalcup-verifier -p goalcup-protocol`.

### WP11: The server is the clock
Status (16 Oct): done (#163, plus #175 for status and reserve).
- Today: the server stamps `official_start` and `deadline_at` at open, and each heartbeat answer
  says `stop` and repeats `deadline_at` (`server/worker/src/runs.ts:93-97`). Offline, the tracer
  counts the run's length on a clock that keeps running while the machine sleeps
  (`cli/src/upload/uploader.rs` `LocalDeadline`). The weak spot: a continued segment
  (`LocalDeadline::until`) subtracts the machine's own date, which the participant can change.
- Change: every heartbeat answer (and the open and continue answers) adds `seconds_left`, computed on
  the server. On each answer the tracer resets its countdown to `seconds_left` on the continuous
  clock. It never reads the machine's date for the deadline. During an outage it keeps counting
  down from the last answer; when the server comes back its answer wins.
- Accept: a stand-in test where the machine's date jumps a day back mid-run still stops on time;
  a chat-mode run stops on the server's `stop` (WP4).

### WP12: A harness is one plug-in folder
Status (16 Oct): done (#187, #189, #190, #192, #193). A scratch fifth connector passes every check with zero Rust edits.
- Everything harness-specific lives in `connectors/<id>/`: launch, chat mode, env, CA variables,
  redaction rules, provider hosts, native session files. Core reads it; core never names a harness.
- Move the redaction rules and the CA-variable list from Rust tables into manifest data
  (After-event items 3 and 4 below; signed daemon change, fine before the first release).
- Tests and the release find connectors by reading `connectors/` (WP5); a per-connector contract
  test runs for every folder (item 7 below).
- Clean out every hard-coded Codex, Claude Code and Hermes special case in Rust and tests; each of
  the three becomes an ordinary connector folder with its own contract test, like harness #15.
- A new harness can be added between the feature stop and the last release (6 Nov 12:00 UTC); it
  needs no core change, only its folder, its test and the 1-hour organizer run.
- Accept: a scratch connector folder for a fifth harness passes `check-connector.sh` and the contract
  test, ships in the release tarball, with zero Rust edits.

### WP10: Release cut (first ~23 Oct, last 6 Nov 12:00 UTC)
- Run `gate.sh full` once (including `upload_worker` and `npm test`), then `scripts/linux-check.sh`. Run `scripts/release.sh 0.6.0` and the release workflow.
- Accept: `install.sh` works on clean Apple Silicon and Intel Macs and on Linux arm64/x86_64. `goalcup doctor --harness all` lists every shipped connector. The release notes list the pinned ids.

## Feature stop

Until 4 Nov 00:00 UTC anything on this list may land. From 4 Nov, only fixes: bugs from dry runs and
10-minute tests, flakes and docs. Connectors may still land until the last release on 6 Nov 12:00
UTC. Every change passes `gate.sh fast`; Worker changes also pass `npm test`; every release passes
`gate.sh full` and `scripts/linux-check.sh`.

## Parked

- Native Windows.
- An admin page. For 7 Nov, organizers use the Supabase table view.
- Signing and notarizing the macOS tarballs. A `curl` download is not quarantined, so the
  install script runs them without it.
- Full registration and team system.
- New harnesses: builders bring them ([HARNESS-ONBOARDING](docs/HARNESS-ONBOARDING.md)).
- Speed up the two P1 redaction tests that each run over 30 s
  (`capture_redaction::a_masked_key_around_the_hold_bound_is_never_retained`,
  `capture_redaction_extra::seeded_random_chunking_matches_the_oracle_with_extra_patterns`). They
  floor the fast gate.

## Larger clean-ups (parallel agents, before the feature stop where safe)

Run these in parallel worktrees, two writers never on the same files. Items 3 and 7 are WP12.
Anything not done by 4 Nov moves to after the event.

Status (16 Oct): 3, 4, 7 and 9 done (WP12). 10 done (#183, #184); the Anthropic port to the shared tracker waits until after the event. 11 and 12 done. 1, 2 and 5 wait until after the first release.

1. **Run lifecycle module.** Add a persisted `enum StopReason` that falls back to the old strings, and `run/lifecycle.rs` `phase()`, read by status, reserve and finish. Add `HarnessState::Exited.code`. Move `RunMode` to `run/`. Rename the wire field `reserve_bytes` to `chain_quota_bytes` with an alias. [C4-F1, F10, F11]
2. **Split `start.rs` and `uploader.rs`.** `commands/start/` plus `crate::shared_home`, and `uploader.rs` in five parts, keeping `pub use`. After the move: an Undo guard, `StartCx`, `SealArgs`, `TestDoors` and shared helpers. [C4-F2, F3, F8, F9, F12]
3. **Redaction rules from manifest data.** Typed `{json_field, value}` and `{header_prefix}` forms. Delete `EXTRA_PATTERN_SOURCES`, `EXTRA_SHAPES` and `BILLING_PREFIX`. Merge `RedactionPattern` into `ExtraPattern`. Re-pin. [C2-F1, C3-F8, C1-F6]
4. **CA env registry from data.** A generic `SSL_CERT_FILE`/`REQUESTS_CA_BUNDLE` row backed by a file of system roots plus the proxy CA. `{config_home}` and `{run_home}` env tokens. A shared table of provider hosts. [C3-F7, F9, F13]
5. **libc → `goalcup-platform` (prep for native Windows).** Move process and clock out of protocol. Send signals from one place, with the `ProcessOs` seam kept. Adopt or delete `readcap`. Add `create_private_file`, the `machine::*` limits, SIGINT in `terminal`, and the `AsFd` API. Move `terminate/*`. Drop the BSD arms. [C1-F1, F2, F4, F7–F10, F14; C2-F12, F14]
6. **`crates/goalcup-testkit`.** It holds the wait, copy_tree, paths, TableLock and proc helpers and the shared and spike fixtures. Add a `protocol::testkit` for record builders. Merge the standalone binaries into `it`. Tier by binary and require nextest. Give the stand-in a clock `advance`. Keep one `FakeGithub`. [T1-F6, F8–F14; T3-F11; T2-F13]
7. **A contract test per connector.** A static half in `it/connector_contract.rs` and a dynamic half in `full/` with one manifest-driven stand-in. Then give Hermes- and Amp-named tests shape names, move engine tests to fixture connectors, and delete the test-only identity wrappers (`launch.rs:1498,1533,1572`, `capture_tee.rs:174,223,317`). [T2-§6, F11; T1-F4; T3-F13; C3-F5]
8. **Shared CLI↔Worker contract.** A `test/contract/cases.json` table run against both the stand-in and the Worker. Run the rule cases against Postgres loaded with the production SQL. Then drop one copy of the rules. Use one source for the run length. The DB picks the event by its window, and `EVENT_ID` goes. Add the Worker module seams. [T3-F1, F2; T2-F6; C6-09, 12, 14, 02]
9. **Restructure the goldens.** One shared deny-probe or policy table, with only per-connector differences in each file. The digest literal becomes a bool that says it matches the pin. Move the golden, kit and doctor tests into `it` and delete verifier `full`. [C3-F10, T3-F6, T1-F1]
10. **One file per verifier decoder format.** `native/{mod,codex,claude_code}.rs` with a `NativeFormat` trait. Put Anthropic on the shared tracker. Split `apply` per record kind. Remove the signed-finalization and legacy-seal paths. Report the undecoded inventory, trim the decoder prose, and add a real chat-completions fixture and per-connector sample replay. [C5-F7–F13, T3-F12]
11. **Observability and judging.** `scripts/fetch-run.sh` rebuilds a trace from the DB chunk index; needed before judging. Add `first_failure`, `entry_id` and `profile_id` to the report. Add `cycles` and `reopen_failed` to `DaemonHealth`. [C5-F2, F3a, F4; C2-F5]
12. **Control vocabulary at the next `CONTROL_API_VERSION`.** Drop `WorkspaceInfo`, `status_cache_path`, `[event]`, `/v0/entries/current`, the V1 registry fields, the unproduced `HealthState` variants (add `serde(other)`), `status_url` and the mock arming. The mock defaults to a capture status, and the legacy branch at `status.rs:266-370` goes with its six tests. [C1-F11, F13; C2-F3, F7, F9; C4; T2-F5]
13. **Product decisions.** Should the server hold the admitted connector digests (no release per harness)? Does headless capture stay or freeze? [C3-F3, F12]
14. **Research export (after the event).** A cleaning step strips people, organisations, emails, accounts, and repo and host identifiers, and strips plan, credit and billing data (H-28, D26). Each research group gets a cleaned export through custom, narrow access. Researchers never touch our database or R2. `fetch-run.sh` stays organiser-only.

## Keep as is

These look odd but are right. Do not "fix" them.

- The closed trust tables in `capture_policy.rs` (D43). Change their form later, not their closedness. The deny rows naming `CLAUDE_CODE_*`, `ANTHROPIC_` and `OPENAI_`. [C1, C3]
- Pins compiled in through `include_str!` with the const-eval parser, the folder/id/digest triple check, and `--dev` refusing pinned ids. [C1, C3]
- Loading every connector folder on create (fix the message, not the loop), and the best-effort restart lookup. [C2, C3]
- The stream redactor's hold-and-fail-closed design. The whole-record `Redactor` as its oracle. `spikes/redact.py` as an oracle. [C2, T3]
- Decoders keyed by `protocol_hint`, capture-only for an unknown hint, and no TOML native mapping. HTTP/1.1-only ALPN. [C3, C5]
- The `ProcessOs` seam, the strict `ProcessRegistration` Deserialize, and `KillStrategy::WrapperUnsafe` (wire). [C1]
- The exit ladders: cli `try_parse` exits 1, verifier exits 3, and `CliError::exit_code` is the single place a number is chosen. [C4, C5]
- The refusal order in `continue_run.rs`, seal never signalling, doctor tolerating a missing daemon, the `practice`/`test` bools, `GiveBack` with `pending-open.json`, and `LONGEST_RUN`. [C4]
- The v5 corpus (add cases, never regenerate to pass), `chain_golden` reading the spike fixtures, the envelope-bytes pin, and the serde-wording pin. [T3]
- The test layout: `it`/`full` binaries, nextest one-process-per-test, the self dev-dependency with a feature, the three process-global binaries, the child-half pattern, the kit cases, and independent m2 oracles. [T1, T2]
- Lean `ci.yml`, the manual `linux-x86_64.yml`, and the manual release run. [T1, C6]
- Worker: the staging block repeats every var and sets `routes = []`, prod `TEST_EVENT_ID`, the two-layer test fence, `/v1/health` before maintenance, a retried chunk answered first, and `db-local.ts`. [C6]
- The Rust upload stand-in until After-event step 8 decides. Negative-window sleeps (give them a helper, do not remove them). The 25 s SIGSTOP scenario. [T1, C6]
- `--dev-key` alongside the one pinned key, and the report-file rules. Harness names in comments are examples only. [C5, C2, C4]

## Open owner questions

- WP0 needs: a Little Snitch rule for the new build (Mac), a ChatGPT sign-in in Chrome or the device
  code (Linux Codex), and logins for Linux Claude and Amp. A live Amp run before #206 merges.

## Known issues

- **T25 (measure, dry run):** fault coverage still open: a literal ENOSPC on a disposable volume, a
  live-receiver TCP close, and RSS growth over a long run.
- **F-04:** a torn tail is kept and never repaired. `goalcup continue` closes such a segment at its
  last whole record and starts a new one. The torn segment says it is incomplete.
- **H-10 residual:** a CLI crash after release plus a raced manual seal can leave a harness running
  on a sealed entry.
- **F-16 / E02:** the cross-check matcher that scored the early unattended run failed 2 of 6. Replace
  it (start-anchored match) before an eight-hour result is scored.
- **F-07:** a base64url run of 16 KiB or more redacts the rest of a one-line JSON body. Measured
  false on the pilot path. Re-measure on the eight-hour chain.
- **H-20:** invalid UTF-8 inside a key body leaves the key raw, and a lone `BEGIN` redacts to the
  end of the record. Fix before any upload surface sees raw bytes.
- **H-28 (before any research access; clean-up 14):** Anthropic usage objects pass through the private research
  observations. Allowlist token fields and strip plan, credit and billing data (D26) before anything
  is published or read from R2.
- **T24:** the release tarballs are unsigned. They carry every connector since #164.
- **H-26:** when a stop cannot be confirmed (`Unresolved`), record the final pushed Git commit as a
  "last known state" marker. Open question: which branch counts when the work is not on main.
- **Little Snitch and updates:** each rebuilt `goalcup` binary has a new ad-hoc signature identifier
  (`goalcup-<hash>`), so Little Snitch blocks it again and the connection times out. The participant
  guide must say so (allow `goalcup` again after each update). Open (UX-F).
- **D62 OAuth-login services (Hermes):** a token refresh to a login host fails under the
  replace-mode `HERMES_CA_BUNDLE`, because the login host tunnels with its real certificate. So
  API-key services work, and OAuth-login services (Nous Portal, Codex-OAuth, xAI-OAuth) may not.
  Traced from source; measure with a Nous Portal or Codex-OAuth Hermes row in the test round
  ([hermes-cli.md](docs/harnesses/hermes-cli.md), section 9).
- **Linux:** the fast gate passes in Docker (arm64). The reservation works on ext4 and XFS. A run
  with no login launches both measured builds (arm64 and x86_64), fails on auth and seals. The
  x86_64 timing failures are fixed (39ba14f): the x86_64 fast gate passed 1933 tests on 16 Oct
  (run 38775727986). Not proven: termination of a fork-and-leave wrapper against a real harness
  ([TERMINATION](docs/TERMINATION.md)), a logged-in harness capture on Linux (where each harness
  keeps its login there is not checked), the end-to-end suites on x86_64, a real Linux host, and
  long runs.
