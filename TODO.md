# TODO

The single tracker. Five steps, in order. Everything else is parked. Old plans, the ledger and the
qualification records are in git history at `6494030`; ticket IDs below (H-11, F-07, ...) name
tickets in the retired `LEDGER.md` there.

Done means: the fast gate passes, one reviewer reads the PR once, and README or this file is
updated if behaviour changed ([AGENTS.md](AGENTS.md)).

## Steps

1. **8-hour Codex run.** One unattended Codex CLI run of eight hours on macOS, sealed and checked
   with `goalcup-verify --capture-integrity`.
2. **Hourly upload to our storage.** R2 plus a tiny upload endpoint. A per-participant upload
   token. The tracker uploads every hour; once the upload is acknowledged it prunes local data
   older than 2 hours; it retries until the connection returns.
3. **One admin page.** Entries, first prompt, last upload, outage flags. Researchers read R2
   directly.
4. **Signed macOS binaries.** Sign and notarize the Homebrew release tarballs
   ([packaging/README.md](packaging/README.md)), which ship unsigned.
5. **168-hour dry run** with two or three friendly teams.

## Parked

- Windows (WSL2 documented, untested: [README](README.md#windows-use-wsl2))
- New harnesses (builders bring them: [HARNESS-ONBOARDING](docs/HARNESS-ONBOARDING.md))
- Full registration and teams system
- Qualification records
- Delete the daemon's `ReceiptBindingPolicy` / `[receipt_binding]` config: its only reader,
  `Registry::store_receipt`, is gone.
- Delete daemon `server_public_key` / `upload_url`, or wire them to the server: since D48 and
  the removal of the placeholder notice nothing reads the key.
- Delete `DaemonClient::current_entry` and the daemon's `GET /v0/entries/current`: no `goalcup`
  command calls it.
- Delete the protocol's pre-v5 trace surface the removed V1 verifier was the reader of:
  `TraceExport::{write_descriptor, write_manifest}`, `TraceDescriptorDoc`, `DESCRIPTOR_FILE`,
  `MANIFEST_FILE`, the V1 `human_rule` types in `manifest.rs`, and the parts of
  `tests/export_roundtrip.rs` that only exercise them. No shipped binary calls them.
- Speed up the two P1 redaction tests that each run over 30 s
  (`capture_redaction::a_masked_key_around_the_hold_bound_is_never_retained`,
  `capture_redaction_extra::seeded_random_chunking_matches_the_oracle_with_extra_patterns`); they
  floor the fast gate.

## Known issues

Carried over from the old ledger's open tickets; only those that bear on the five steps.

- **H-11 (steps 1, 5):** the chain reservation stays allocated after seal; only grow-then-shrink
  frees APFS blocks and that has a torn-tail window. A separate-reservation design is due before
  any long run (D40).
- **T25 (steps 1, 5):** fault coverage still open: a literal ENOSPC on a disposable volume, a
  live-receiver TCP close, and RSS growth over a long run.
- **F-04 (step 1):** a torn tail is kept and never repaired (the V1 `repair_seal` was removed under
  OS-31); the entry cannot resume.
- **H-10 residual (step 1):** a CLI crash after release plus a raced manual seal can leave a
  harness running on a sealed entry.
- **F-16 / E02 (step 1):** the cross-check matcher that scored the early unattended run failed 2
  of 6 and must be replaced (start-anchored match) before an eight-hour result is scored.
- **F-07 (steps 1, 2):** a base64url run of 16 KiB or more redacts the rest of a one-line JSON
  body. Measured false on the pilot path; re-measure on the eight-hour chain.
- **H-20 (step 2):** redaction residuals: invalid UTF-8 inside a key body leaves the key raw, and a
  lone `BEGIN` redacts to the end of the record. Fix before any upload surface sees raw bytes.
- **H-28 (steps 2, 3):** Anthropic usage objects pass through the private research observations;
  allowlist token fields structurally and strip plan, credit and billing data (D26) before
  anything is published or read from R2.
- **T24 (step 4):** the release tarballs are unsigned and carry only the Codex and Claude
  connectors (`scripts/package-release.sh`, H-21). (H-19, the persistent launchd agent an evidence install left behind, is closed:
  the daemon is run-scoped and nothing is installed as a service.)
- **H-26 (step 3):** when a stop cannot be confirmed (`Unresolved`), record the final pushed Git
  commit as a "last known state" marker. Open question: which branch counts when the work is not
  on main.
- **Linux (steps 1, 5):** builds and passes the fast gate in Docker; the reservation works on ext4
  and XFS. Not proven: termination of a fork-and-leave wrapper (a child-subreaper fix is due before
  the 168-hour dry run, [TERMINATION.md](docs/TERMINATION.md)); a real harness capture (the shipped
  connectors pin macOS binaries); the end-to-end suites on x86_64; a real Linux host; long runs
  ([README, Linux](README.md#linux)).
