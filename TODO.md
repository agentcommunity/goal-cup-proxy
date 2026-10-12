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
4. **Signed macOS installer.** Signed, replacing the unsigned private `m2-pilot`
   bundle ([packaging/README.md](packaging/README.md)).
5. **168-hour dry run** with two or three friendly teams.

## Parked

- Linux
- Windows
- New harnesses (builders bring them: [HARNESS-ONBOARDING](docs/HARNESS-ONBOARDING.md))
- Full registration and teams system
- Qualification records
- Decide: delete the unused `goalcup-devserver` crate (and its three test files): a dev-only
  receipt, acknowledgement and chunk signer. Check first whether the verifier's V12/V13 round
  trip still needs it; protocol `receipt.rs` and `ack.rs` stay either way, the verifier reads them.
- Delete `goalcup-protocol/src/chunk.rs` (and its unit tests) with the devserver's chunk route: no
  shipped binary reaches the chunk upload flow. Its integration tests went in the test diet, so
  `CHUNK_DOMAIN`'s distinctness from the receipt, acknowledgement and chain domains is untested
  now: if `chunk` stays, restore one distinctness assertion.
- Remove the manifest `launch`, `resume` and `banned_args` fields from the V1 manifest schema
  (`goalcup-protocol/src/manifest.rs`): parsed, read by no shipped binary (launch plans come from
  `harness.toml`). Their snapshot tests went in the test diet.
- Delete daemon `Registry::store_receipt` (test-hooks only, no caller left) with the
  `ReceiptBindingPolicy` / `[receipt_binding]` config that only it reads, and `Entry::receipt` if
  nothing else does.
- Delete daemon `server_public_key` / `upload_url` and `placeholder_key_notice`: since D48 the key's
  only consumer is a startup notice about receipts that no longer exist. Its test,
  `tests/it/placeholder_notice.rs`, goes with it.
- Delete the CLI's V1 daemon-client routes, now untested and unreachable from
  `goalcup start|seal|status|entries|doctor`: `HttpDaemonClient::{create_workspace, arm,
  wait_start_signal, resume, resume_preview, current_entry}`, `daemon::long_poll_read_timeout`,
  `MockDaemonClient::{set_signal_delay, withhold_signal}`, with the V1 methods of
  `goalcup_protocol::control::DaemonClient`.
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
- **H-19 / T24 (step 4):** an evidence install keeps a persistent launchd agent that no run script
  boots out; the pilot bundle is unsigned and `M2_PILOT_CONNECTORS` lists only Codex and Claude
  (H-21).
- **H-26 (step 3):** when a stop cannot be confirmed (`Unresolved`), record the final pushed Git
  commit as a "last known state" marker. Open question: which branch counts when the work is not
  on main.
