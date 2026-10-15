# TODO

The single tracker. Done means: the fast gate passes, one reviewer reads the PR once, and README or
this file is updated if behaviour changed ([AGENTS.md](AGENTS.md)). Ticket IDs below (F-04, H-20,
...) are names only. [CHEAT-CHECKLIST](docs/CHEAT-CHECKLIST.md) uses the same names.

## Dates (UTC)

| When | What |
|---|---|
| 23 to 24 Oct 2026 | First tagged release and the Homebrew tap ([packaging/README.md](packaging/README.md)) |
| 26 Oct | Freeze: `sh scripts/gate.sh full` passes. Fixes only after this |
| ~28 Oct | Dry run with two or three friendly teams. Fixes only |
| 7 Nov 00:00 | Start window opens. It lasts 24 hours. Each run then has 168 hours |

Owner decisions of 16 Oct 2026:

- Each participant's prompt becomes public at the latest at the end of the event.
- The second-harness rule stays: harness A must not start harness B (D10).
- Support contact: goalcup@agentcommunity.org.
- Windows means WSL2 for the event ([README](README.md#windows-use-wsl2)). Native Windows later.
- Undecided: the demo and questionnaire deadline, either 24 hours after the run or a fixed date
  around 15 Nov. [owner to confirm]

## Before the freeze

1. **Measure.** The 10-minute matrix (`scripts/matrix.sh`) and the 8-hour runs against the test
   server, on macOS and Linux ([OPERATING](docs/OPERATING.md)).
2. **Docs.** The [participant's guide](docs/PARTICIPANT-GUIDE.md), the
   [setup skill](skills/goal-cup-setup/SKILL.md), RULES and README as the code runs them.
3. **Release.** `sh scripts/gate.sh full`, the first tag, the tap. Neither the tag nor the tap
   exists yet.

## Parked

- Native Windows.
- An admin page. For 7 Nov, organizers use the Supabase table view.
- Signing and notarizing the macOS tarballs. Homebrew does not quarantine what it downloads, so the
  brew path runs without them.
- Full registration and team system.
- New harnesses: builders bring them ([HARNESS-ONBOARDING](docs/HARNESS-ONBOARDING.md)).
- Dead code to delete (each is unused outside tests; check before deleting):
  - the daemon's `ReceiptBindingPolicy` / `[receipt_binding]`, `server_public_key` and
    `upload_url` config fields (they drive nothing, D48);
  - `DaemonClient::current_entry` and `GET /v0/entries/current` (no `goalcup` command calls them);
  - `export::DESCRIPTOR_FILE` and the resume sweep of `trace.toml` temporaries, once the CLI test
    fixture `mock_sealable` (`crates/goalcup-cli/tests/common/support/mod.rs`) stops naming it.
- Speed up the two P1 redaction tests that each run over 30 s
  (`capture_redaction::a_masked_key_around_the_hold_bound_is_never_retained`,
  `capture_redaction_extra::seeded_random_chunking_matches_the_oracle_with_extra_patterns`). They
  floor the fast gate.

## After the event

Each of these changes a wire or touches several lanes, for little gain now.

- Delete `goalcup_protocol::control::WorkspaceInfo`. It is test-only; the code uses
  `CaptureWorkspaceInfo`.
- Rename the capture-create request's `reserve_bytes` to a quota name and drop the always-0
  `reserved_bytes` from the capture status (D40). Both are control and status wire changes.
- Move the remaining libc process, clock and termination code from `goalcup-protocol`
  (`process.rs`, `clock.rs`) and `goalcup-daemon` (`terminate/*`) into `goalcup-platform`, as a pure
  move, before the native Windows port.

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
- **H-28 (before any research access):** Anthropic usage objects pass through the private research
  observations. Allowlist token fields and strip plan, credit and billing data (D26) before anything
  is published or read from R2.
- **T24:** the release tarballs are unsigned and carry only the Codex and Claude connectors
  (`scripts/package-release.sh`).
- **H-26:** when a stop cannot be confirmed (`Unresolved`), record the final pushed Git commit as a
  "last known state" marker. Open question: which branch counts when the work is not on main.
- **Linux:** the fast gate passes in Docker (arm64). The reservation works on ext4 and XFS. A run
  with no login launches both measured builds (arm64 and x86_64), fails on auth and seals. On a
  4-vCPU x86_64 runner the daemon's `capture_redaction::seeded_long_inputs_*` tests hit the 180 s
  timeout and the CLI's `subreaper::linux::tests::the_bound_reports_what_keeps_arriving` stops
  before its bound. Not proven: termination of a fork-and-leave wrapper against a real harness
  ([TERMINATION](docs/TERMINATION.md)), a logged-in harness capture on Linux (where each harness
  keeps its login there is not checked), the end-to-end suites on x86_64, a real Linux host, and
  long runs.
