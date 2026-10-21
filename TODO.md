# TODO

The single task list. Done means: the fast gate passes, one reviewer reads the PR once, and README or
this file is updated if behaviour changed ([AGENTS.md](AGENTS.md)). Ticket IDs below (F-04, H-20,
...) are names only. [CHEAT-CHECKLIST](docs/CHEAT-CHECKLIST.md) uses the same names.

## Research recorder validation

The [research target](docs/RESEARCH-TARGET.md) defines the research scope, and
[local next steps](docs/research/LOCAL-NEXT-STEPS.md) the next work.
The target is useful evidence of agent work across harnesses and operating systems, with explicit
coverage limits. Context-visible tool calls/results are sufficient; independently collecting full
tool-output/spill files is not required. Subagent conversations need separate coverage checks.
Existing event rules and privacy requirements remain in force.

Validate in three phases, without treating a lifecycle/integrity PASS as proof of research coverage:

1. Establish one reliable end-to-end path, reconciling known activity with retained evidence and
   decoded observations. Start with Codex using Luna at high reasoning effort for up to 30 minutes;
   Claude validation waits for available budget. Record versions, configuration and evidence.
2. Repeat that validation per harness, reusing shared wire decoders where protocols match and
   documenting native-format and subagent differences. Keep unmeasured platform cells explicit.
3. Establish endurance only after the shorter paths are reliable: interruptions, resumptions,
   rotation, capture fidelity and resource growth over longer runs. Native Windows remains
   planned; a WSL2 run is evidence for that Linux environment, not native Windows support.

These are validation targets. They do not mark the runtime or measurement gaps below as fixed.
The [coverage matrix](docs/research/coverage-matrix.md) distinguishes real activity, retrieval,
decoding and child coverage for each measured harness/platform cell.

The 18 and 19 Oct baseline runs (Codex Luna/high, a native child probe, Hermes and Amp
replays) are recorded in the [October archive](docs/archive/TODO-2026-10.md#research-baseline-runs-18-to-19-oct).

## Dates (UTC)

| When | What |
|---|---|
| Release cut | Version PR (if needed), merge under the `main` ruleset, full gate and separate `scripts/linux-check.sh` on merged `origin/main`, then tag |
| 23 Oct to 6 Nov 00:00 | Dry-run window; dry runs can start on any day |
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
- Time: the server is the clock. Every heartbeat answer carries the time left; Goal Cup Proxy counts
  down from that and never trusts the machine's own date (WP11).
- A harness is a plug-in module: its folder carries every harness-specific setting, and tests and
  release discover it. No harness name by hand anywhere else (WP5, WP12).
- Historical launch-planning note (16 Oct): larger clean-ups were scheduled before the feature stop.
  It is not a current compatibility or review policy; follow `AGENTS.md`, `CONTRIBUTING.md` and
  the current release checklist below.
- Demo and questionnaire: due 24 hours after each run's 168 hours end (a later start gets a later
  due time). They go through the website portal, not Goal Cup Proxy; the website may close it leniently.
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
- No keyed runs: Goal Cup Proxy never sees, forwards or stores a provider key (D61). Every harness,
  Hermes included, keeps its own login in its home, and every run uploads under the same rules.
  #208 redacts the run's own proxy credential by exact value; #214 runs Hermes in chat mode from
  its own home; #215 removes the key-forwarding mechanism.
- Amp's native source is its metadata log, labelled as such. Logged-in macOS chat through
  Goal Cup Proxy completed on 17 Oct with zero capture gaps. #206's pinned macOS staging practice also
  passed capture/seal/upload matching; process-tree termination remains `Unresolved` (below).
  Amp's native metadata and HTTP setup remain capture-only. The separately versioned thread
  decoder now interprets complete WebSocket conversation snapshots; hidden provider requests remain unobserved.
- Claude Code's folder-trust dialog: the participant answers it. RULES allows any start-up answer
  before the first prompt.
- Every start needs a human click in the browser, also when it replaces a run. The test approval key
  exists on staging only.
- Wire-only core (a connector with no native leg) is deferred: not before 4 Nov.

## Work to the feature stop (4 Nov), in order

The [event readiness plan](docs/plans/2026-10-20-event-readiness.md) (20 Oct) sets the
work from 20 Oct to 6 Nov: invariants, work packages WP-A to WP-H, the test ladder and the owner
decisions. The [simplification plan](docs/research/SIMPLIFICATION-2026-10-21.md) (21 Oct) lists the
low-risk clean-ups before the event and the larger ones after it. The Review 12 work packages
WP1 to WP9, WP11 and WP12 are done; their checklists and ticket IDs are in the
[October archive](docs/archive/TODO-2026-10.md#work-packages-finished-by-20-oct). One PR per
package; `sh scripts/gate.sh fast` before every commit.

Status (21 Oct, branch `smart/zen-gates-d9tmbh`, not yet released):
- WP-A done (D64): no server error stops a run or ends its upload. Follow-up: a `410` answer to
  `continue` still marks the run over; a bad deploy of that route would refuse continues.
- WP-B part 1 done: acknowledged parts are deleted at once when the disk is short. The quiet-time
  upload trigger waits for WP-0 gap data.
- WP-D done in the Worker, plus parallel test runs (test runs no longer replace each other). Both
  need the website migrations in `server/worker/sql/pending/`, applied before that Worker deploys.
  Check the website approval page wording for test runs.
- WP-E docs done (H-29 widened, login guidance, StorageFull rule). The verifier scan moves to the
  post-event export step (owner, 21 Oct): flagged traces are cleaned before anyone gets them.
- WP-F kit done: `scripts/campaign.py`, `matrix.sh --test --duration`, `linux-matrix.sh` cells,
  `research-measure.py`, `research-sample.py`, `research-staging.py`;
  [local campaign](docs/research/LOCAL-CAMPAIGN.md). Not yet run against real Docker or staging.
- WP-G done: E02 cross-check ported with unique bodies and a start-anchored match.
- T25 measured ([t25-disk-full](docs/research/t25-disk-full.md)): before WP-C, a full disk silently
  ended the recording.
- WP-C, small version (owner, 21 Oct): pause the agent below 1 GiB free, resume above 3 GiB (D65),
  no disk stop verdict, vitals in heartbeats and status, cleanup of sent parts after a run. The
  daemon recorder-hold rework is dropped: the pause comes first.
- WP-H steps 1 to 3 done (Goal Cup Proxy; the client sends proxy_version and tracer_version). Open: step 4 repo rename (owner, on GitHub), step 5 schema rename.

### WP0: Measure
Status (17 Oct): macOS and Linux Codex (luna/low) and Hermes (DeepSeek) passed staging
practice runs: deadline stop, seal, matching uploaded server position and zero-gap research
reports. Hermes native snapshots remain undecoded. Amp's pinned macOS task, capture and delivery
passed with zero gaps (#206); its trace does not confirm process-tree termination. Left:
- Claude (haiku) reached its weekly quota after the folder-trust dialog. Retry when quota returns;
  preserve the incomplete-stop trace and its gaps rather than count it as a successful task.
- Linux Amp and Claude task runs remain unmeasured; no Linux Amp login or capture claim.
- Then: the 8-hour runs on macOS and Linux, Intel Mac, x86_64 Linux and WSL2, and the edge cases in
  [OPERATING](docs/OPERATING.md). The matrix is `scripts/matrix.sh` (Linux: `scripts/linux-matrix.sh`, #198).

The 16 Oct WP0 findings list (27 small bugs, all fixed or kept by design) is in the
[October archive](docs/archive/TODO-2026-10.md#wp0-findings-16-oct).

### WP10: Release cut (next version; last scheduled release 6 Nov 12:00 UTC)
- If the version is not already on `origin/main`, run `scripts/release.sh prepare <x.y.z>` and
  merge its PR under the `main` ruleset. On merged, versioned `origin/main`, run
  `sh scripts/gate.sh full` and the separate `sh scripts/linux-check.sh`; then run
  `sh scripts/release.sh tag <x.y.z>`. `tag` does not run the full gate. A manual release workflow
  run only builds and tests artifacts; it does not publish.
- Accept: the tag workflow passes on all four targets and publishes four tarballs, `SHA256SUMS`,
  and `install.sh`; verify the published installer in a fresh private prefix. `goalcup doctor
  --harness all` checks every shipped connector and reports install status (it may exit 1 when an
  optional harness is absent). Report source version, tag and published assets as separate states.

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
- Done 21 Oct: the slow P1 redaction tests are split into parallel parts (slowest part about 9 s)
  and the daemon builds at opt-level 1 in test builds.

## Larger clean-ups (parallel agents, before the feature stop where safe)

Run these in parallel worktrees, two writers never on the same files. Items 3 and 7 are WP12.
Anything not done by 4 Nov moves to after the event.

Status (16 Oct): 3, 4, 7 and 9 done (WP12). 10 done (#183, #184); the Anthropic port to the shared `Tracker` waits until after the event. 11 and 12 done. 1, 2 and 5 wait until after the event.

1. **Run lifecycle module.** Add a persisted `enum StopReason` that falls back to the old strings, and `run/lifecycle.rs` `phase()`, read by status, reserve and finish. Add `HarnessState::Exited.code`. Move `RunMode` to `run/`. Rename the wire field `reserve_bytes` to `chain_quota_bytes` with an alias. [C4-F1, F10, F11]
2. **Split `start.rs` and `uploader.rs`.** `commands/start/` plus `crate::shared_home`, and `uploader.rs` in five parts, keeping `pub use`. After the move: an Undo guard, `StartCx`, `SealArgs`, `TestDoors` and shared helpers. [C4-F2, F3, F8, F9, F12]
3. **Redaction rules from manifest data.** Typed `{json_field, value}` and `{header_prefix}` forms. Delete `EXTRA_PATTERN_SOURCES`, `EXTRA_SHAPES` and `BILLING_PREFIX`. Merge `RedactionPattern` into `ExtraPattern`. Re-pin. [C2-F1, C3-F8, C1-F6]
4. **CA env registry from data.** A generic `SSL_CERT_FILE`/`REQUESTS_CA_BUNDLE` row backed by a file of system roots plus the recording proxy CA. `{config_home}` and `{run_home}` env tokens. A shared table of provider hosts. [C3-F7, F9, F13]
5. **libc → `goalcup-platform` (prep for native Windows).** Move process and clock out of protocol. Send signals from one place, with the `ProcessOs` seam kept. (`readcap` and `filecap` were deleted on 21 Oct.) Add `create_private_file`, the `machine::*` limits, SIGINT in `terminal`, and the `AsFd` API. Move `terminate/*`. Drop the BSD arms. [C1-F1, F2, F4, F7–F10, F14; C2-F12, F14]
6. **`crates/goalcup-testkit`.** It holds the wait, copy_tree, paths, TableLock and proc helpers and the shared and spike fixtures. Add a `protocol::testkit` for record builders. Merge the standalone binaries into `it`. Tier by binary and require nextest. Give the stand-in a clock `advance`. Keep one `FakeGithub`. [T1-F6, F8–F14; T3-F11; T2-F13]
7. **A contract test per connector.** A static half in `it/connector_contract.rs` and a dynamic half in `full/` with one manifest-driven stand-in. Then give Hermes- and Amp-named tests shape names, move engine tests to fixture connectors, and delete the test-only identity wrappers (`launch.rs:1498,1533,1572`, `capture_tee.rs:174,223,317`). [T2-§6, F11; T1-F4; T3-F13; C3-F5]
8. **Shared CLI↔Worker contract.** A `test/contract/cases.json` table run against both the stand-in and the Worker. Run the rule cases against Postgres loaded with the production SQL. Then drop one copy of the rules. Use one source for the run length. The DB picks the event by its window, and `EVENT_ID` goes. Add the Worker module seams. [T3-F1, F2; T2-F6; C6-09, 12, 14, 02]
9. **Restructure the goldens.** One shared deny-probe or policy table, with only per-connector differences in each file. The digest literal becomes a bool that says it matches the pin. Move the golden, kit and doctor tests into `it` and delete verifier `full`. [C3-F10, T3-F6, T1-F1]
10. **One file per verifier decoder format.** `native/{mod,codex,claude_code}.rs` with a `NativeFormat` trait. Put Anthropic on the shared `Tracker`. Split `apply` per record kind. Remove the signed-finalization and legacy-seal paths. Report the undecoded inventory, trim the decoder prose, and add a real chat-completions fixture and per-connector sample replay. [C5-F7–F13, T3-F12]
11. **Observability and judging.** `scripts/fetch-run.sh` rebuilds a trace from the DB chunk index; needed before judging. Add `first_failure`, `entry_id` and `profile_id` to the report. Add `cycles` and `reopen_failed` to `DaemonHealth`. [C5-F2, F3a, F4; C2-F5]
12. **Control vocabulary at the next `CONTROL_API_VERSION`.** Drop `WorkspaceInfo`, `status_cache_path`, `[event]`, `/v0/entries/current`, the V1 registry fields, the unproduced `HealthState` variants (add `serde(other)`), `status_url` and the mock arming. The mock defaults to a capture status, and the legacy branch at `status.rs:266-370` goes with its six tests. [C1-F11, F13; C2-F3, F7, F9; C4; T2-F5]
13. **Product decisions.** Should the server hold the admitted connector digests (no release per harness)? Headless capture: decided 21 Oct, removed ([simplification plan](docs/research/SIMPLIFICATION-2026-10-21.md), A1). A manifest with `interactive = false`, `goal_flag` or a session identity is refused as "chat mode only"; `--goal-file` and the deferred identity binding are gone. Left for the next `CONTROL_API_VERSION`: the unused `native_scope` field on the control finish request, and the scope-id support in the native capture it fed. [C3-F3, F12]
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

- WP0: the owner completed Linux Codex sign-in and Amp's private-home macOS sign-in. A rebuilt
  Mac binary may need a new Little Snitch approval. Claude remains deferred until quota returns;
  Linux Amp task coverage is still outstanding.

## Known issues

- **Amp macOS deadline termination:** the 17 Oct pinned practice passed capture and delivery,
  and the CLI observed exit after signal 9, but the trace records
  `Unresolved(ProcessAuthorityUnavailable)`, with no remaining targeted process and
  `observed_gone = false`. Investigate the authority-loss cause during longer tests; do not
  claim `Confirmed` process-tree termination. [Measured facts](docs/harnesses/amp-cli.md#pinned-staging-practice-17-oct-2026).

- **T25 (measure, dry run):** fault coverage still open: a live-receiver TCP close, and RSS growth
  over a long run. The literal ENOSPC is measured ([t25-disk-full](docs/research/t25-disk-full.md)).
- **F-04:** a torn tail is kept and never repaired. `goalcup continue` closes such a segment at its
  last whole record and starts a new one. The torn segment says it is incomplete.
- **H-10 residual:** a CLI crash after release plus a raced manual seal can leave a harness running
  on a sealed entry.
- **F-07:** a base64url run of 16 KiB or more redacts the rest of a one-line JSON body. Measured
  false on the pilot path. Re-measure on the eight-hour chain.
- **H-29 (known gap in promise P1; cheap path is WP-E):** a key the model echoes in its reply stream is
  recorded raw on the wire response leg. Every key shape, not only PEM. The model streams its reply
  in small pieces, with JSON framing between them. Every redaction rule needs the secret as one
  contiguous run of bytes, so no rule sees it. Reproduced with `spikes/redact.py`, which tests pin
  equal to the daemon: a split PEM `BEGIN`, `sk-ant-`, `sk-`, `AKIA` and `ghp_` are all stored raw,
  with zero redactions. Even a whole `sk-ant-api03-AbCd` with the rest in the next delta leaks,
  because the rule needs 20 or more contiguous characters (`crates/goalcup-daemon/src/redact.rs:197-198`).
  The same text is redacted in the native transcript and in the next request's history. Only the
  reply stream leaks. The harness's own login header is never stored: headers are skipped unread
  (`crates/goalcup-cli/src/proxy/http.rs:726`; pinned by
  `crates/goalcup-cli/tests/it/m2_http_relay.rs:662`). The agent can read the harness's login files
  and print them, so the model can echo them too (see
  [CHEAT-CHECKLIST](docs/CHEAT-CHECKLIST.md) "Redaction residuals").
  Owner decisions (20 and 21 Oct): for the event, the cheap path. The verifier scans the decoded,
  joined reply text and flags secret-shaped text by kind, count and reference, never the value. A
  flagged trace passes through, flagged, and is cleaned before anyone else gets it (research
  export, clean-up 14). After the event: a stream-aware redactor that joins the text inside each
  SSE or WebSocket event before matching. P1 has this known gap until that redactor lands. Raw R2
  objects under the 180-day lock cannot be rewritten, so participants rotating their login is what
  closes the exposure.
- **H-28 (before any research access; clean-up 14):** Anthropic usage objects pass through the private research
  observations. Allowlist token fields and strip plan, credit and billing data (D26) before anything
  is published or read from R2.
- **T24:** the release tarballs are unsigned. They carry every connector since #164.
- **H-26:** when a stop cannot be confirmed (`Unresolved`), record the final pushed Git commit as a
  "last known state" marker. Open question: which branch counts when the work is not on main.
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
