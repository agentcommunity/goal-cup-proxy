# TODO

The single tracker. The order of work to 6 November is the
[run-lifecycle plan](docs/superpowers/plans/2026-10-14-run-lifecycle.md) (its owner decisions win
over the [design](docs/superpowers/specs/2026-10-14-run-lifecycle-design.md) and
[SERVER-DESIGN](docs/SERVER-DESIGN.md) where they differ). Old plans, the ledger and the
qualification records are in git history at `6494030`; ticket IDs below (F-04, H-20, ...) name
tickets in the retired `LEDGER.md` there.

Done means: the fast gate passes, one reviewer reads the PR once, and README or this file is
updated if behaviour changed ([AGENTS.md](AGENTS.md)).

## Steps

Lanes P, R, W, A and T of the plan have landed: the device-flow login and run credential,
`goalcup prep`, chat mode, rotating chain parts and pruning, the run reservation, `goalcup
continue`, the deadline stop and the Worker's emails. Production runs at
`https://ingest.agentcommunity.org`. What is left, in order:

1. **D1, measure:** the 10-minute matrix (`scripts/matrix.sh`) and the 8-hour runs against the
   test server, on macOS and Linux ([OPERATING](docs/OPERATING.md)).
2. **D2, docs:** the [participant's guide](docs/PARTICIPANT-GUIDE.md), the
   [setup skill](skills/goal-cup-setup/SKILL.md), and RULES and README as the code runs them.
3. **Freeze, 26 Oct:** `sh scripts/gate.sh full`, then the first tagged release and the Homebrew
   tap ([packaging/README.md](packaging/README.md)); neither exists yet.
4. **168-hour dry run, 28 Oct – 4 Nov,** with two or three friendly teams. Fixes only.
5. **Start window, 6 Nov.**

## Parked

- Native Windows: wanted soon after 6 Nov. For the event, Windows means WSL2
  ([README](README.md#windows-use-wsl2)).
- An admin page: for 6 Nov organizers use the Supabase table view.
- Signing and notarizing the macOS release tarballs: Homebrew does not quarantine what it
  downloads, so the brew path runs without them ([packaging/README.md](packaging/README.md)).
- New harnesses (builders bring them: [HARNESS-ONBOARDING](docs/HARNESS-ONBOARDING.md))
- Full registration and teams system
- Qualification records
- Delete the daemon's `ReceiptBindingPolicy` / `[receipt_binding]` config: its only reader,
  `Registry::store_receipt`, is gone.
- Delete daemon `server_public_key` / `upload_url`, or wire them to the server: since D48 and
  the removal of the placeholder notice nothing reads the key.
- Delete `DaemonClient::current_entry` and the daemon's `GET /v0/entries/current`: no `goalcup`
  command calls it.
- Delete `export::DESCRIPTOR_FILE` and the resume's sweep of `trace.toml` temporaries once the
  CLI test fixture `mock_sealable` (`crates/goalcup-cli/tests/common/support/mod.rs`) stops
  naming the file. No writer produces it.
- Speed up the two P1 redaction tests that each run over 30 s
  (`capture_redaction::a_masked_key_around_the_hold_bound_is_never_retained`,
  `capture_redaction_extra::seeded_random_chunking_matches_the_oracle_with_extra_patterns`); they
  floor the fast gate.

## After 6 Nov

Wait until the event is over: each one changes a wire or touches several lanes for little gain now.

- Delete `goalcup_protocol::control::WorkspaceInfo`. It is test-only (the code uses
  `CaptureWorkspaceInfo`); its uses are in protocol tests (`control_shapes.rs`,
  `control_consumer.rs`) and CLI test files.
- Rename the capture-create request's `reserve_bytes` to a quota name and drop the always-0
  `reserved_bytes` from the capture status. Chains are quota-only since D40's amendment; both are
  control and status wire changes, so the CLI changes with them.
- Move the remaining libc process, clock and termination code from `goalcup-protocol`
  (`process.rs`, `clock.rs`) and `goalcup-daemon` (`terminate/*`) into `goalcup-platform`, a pure
  move before the native Windows port.

## Known issues

Carried over from the old ledger's open tickets; only those that bear on the steps above. (H-11,
the chain reservation held after seal, is closed: chains are quota-only and each run holds one
reservation, D40 as amended.)

- **T25 (steps 1, 4):** fault coverage still open: a literal ENOSPC on a disposable volume, a
  live-receiver TCP close, and RSS growth over a long run.
- **F-04 (steps 1, 4):** a torn tail is kept and never repaired (the V1 `repair_seal` was removed
  under OS-31). `goalcup continue` closes such a segment at its last whole record and starts a new
  one; the torn segment says it is incomplete.
- **H-10 residual (steps 1, 4):** a CLI crash after release plus a raced manual seal can leave a
  harness running on a sealed entry.
- **F-16 / E02 (step 1):** the cross-check matcher that scored the early unattended run failed 2
  of 6 and must be replaced (start-anchored match) before an eight-hour result is scored.
- **F-07 (step 1):** a base64url run of 16 KiB or more redacts the rest of a one-line JSON
  body. Measured false on the pilot path; re-measure on the eight-hour chain.
- **H-20 (step 1):** redaction residuals: invalid UTF-8 inside a key body leaves the key raw, and a
  lone `BEGIN` redacts to the end of the record. Fix before any upload surface sees raw bytes.
- **H-28 (before any research access):** Anthropic usage objects pass through the private research
  observations; allowlist token fields structurally and strip plan, credit and billing data (D26)
  before anything is published or read from R2.
- **T24 (step 3):** the release tarballs are unsigned and carry only the Codex and Claude
  connectors (`scripts/package-release.sh`, H-21). (H-19, the persistent launchd agent an evidence install left behind, is closed:
  the daemon is run-scoped and nothing is installed as a service.)
- **H-26 (step 4):** when a stop cannot be confirmed (`Unresolved`), record the final pushed Git
  commit as a "last known state" marker. Open question: which branch counts when the work is not
  on main.
- **Linux (steps 1, 4):** builds and passes the fast gate in Docker; the reservation works on ext4
  and XFS; a run with no login launches the measured arm64 and x86_64 builds, fails on auth and
  seals. Not proven: termination of a fork-and-leave wrapper (the `goalcup start` subreaper sweep
  is measured on a synthetic harness only, [TERMINATION.md](docs/TERMINATION.md));
  a logged-in harness capture; the end-to-end suites and the harness builds on x86_64; a real Linux
  host; long runs ([README, Linux](README.md#linux)).
