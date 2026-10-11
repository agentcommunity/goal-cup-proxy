# Qualification records

Which evidence record proves what, on which source, and is it still current? For anyone about to
trust a Pass or cite a measurement. Updated: 2026-10-11.

A record is a measurement tied to an exact source, binary and harness identity. Bodies are not
rewritten: a failed row stays as it was and a newer row says what it supersedes. Numbers live in
the record, not here; the latest gate line is in [STATUS](../../STATUS.md), tickets in
[LEDGER](../../LEDGER.md). Nothing on m2 is **qualified** until T27 and T28 pass. Terms:
[GLOSSARY](../GLOSSARY.md). Read [CHEAT-CHECKLIST](../CHEAT-CHECKLIST.md) before trusting a Pass.

State values: **current** (describes this branch), **historical** (true when written, preserved),
**superseded by X** (read X instead).

## M2 (wire + native capture, branch `m2/wire`)

| Record | Date | Source | Claims | Result | State |
|---|---|---|---|---|---|
| [m2/reservation.md — Overall verdict](m2/reservation.md#overall-verdict) (T01) | 2026-09-27 | Private historical packet `docs/superpowers/plans/2026-09-25-m2-recorder/T01-reservation-proof.md`, probe `spikes/m2-reservation/reserve_probe.c`, script `spikes/m2-reservation/run.sh`, and `spikes/m2-reservation/RESULTS.md` are omitted from this public OSS cut. The supporting private historical record is intentionally omitted from this public OSS cut. | chain-file space reservation on APFS | PASS for `F_ALLOCATEALL\|F_ALLOCATEPERSIST`; plain `F_ALLOCATEALL` does not survive a remount | current (release of reservations deferred, D40, H-11) |
| [m2/launch-recipes.md](m2/launch-recipes.md) (T02, [raw runs](m2/launch-recipes/)) | 2026-09-27 | `m2/t02-launch`, base `19a52ea` | automatic-goal launch recipes, Codex and Claude | measured recipes and side effects; not recorder proof | current |
| [m2-component-capture.md](m2-component-capture.md) (E00) | 2026-09-27 | `m2/e00-capture`, `a4c9617e` | real proxy, redactor, native adapter, writer, reader on a controlled endpoint | capture matches traffic; deliberate loss reads exit 1 | historical |
| [m2-early-live-capture.md](m2-early-live-capture.md) (E01) | 2026-09-28 | `m2/e01-live` | both real harnesses through the real components | Codex and Claude smoke rows PASS, verify 0 | historical |
| [m2-early-unattended.md](m2-early-unattended.md) (E02) with [goal](m2-early-goal.txt), [criteria](m2-early-criteria.json) | 2026-09-28 | `m2/e02-unattended`, inputs frozen `edebac2` | unattended task (Part A), 30-minute load (Part B) | Part A: Claude finished, Codex partial; Part B frozen verdict FAIL (§8.1) | historical; Part B superseded by the T23 run in m2-synthetic-vertical |
| [m2-live-capture.md](m2-live-capture.md) (T24) | 2026-09-30 | `d8d5b1cf` | installed candidate, two real tasks | both tasks FAILED; native capture and sealing measured; cause not established | historical; superseded by the 3c rows |
| [m2-installed-codex.md](m2-installed-codex.md), [m2-installed-claude.md](m2-installed-claude.md) | 2026-09-30 | `d8d5b1cf` | the two T24 rows in detail | Codex TLS handshake EOF; Claude ECONNRESET; no result file | historical; superseded by the 3c rows |
| [m2-real-agent-progress.md](m2-real-agent-progress.md) (G-3) | 2026-10-01 | `70e8d129`, `3c542aa5` | paired tiny Codex and Claude rows, useful tasks, partner-guide paths, connect diagnostics | tiny rows PASS on both sources; useful Claude first row task FAIL, retry PASS; `6d92adc6` default Codex row FAIL | current |
| [m2-synthetic-vertical.md](m2-synthetic-vertical.md) (T23, G-2) | 2026-10-01 | `3c542aa5` | installed-path lifecycle on a synthetic workload; the full Part B run | eight tests; Part B PASS, expected verifier exit 1 | current |
| [m2-failure-campaign.md](m2-failure-campaign.md) (T25 Task 2) | 2026-10-01 | `dddfe678` | storage and accounting faults, four rows | PASS; T25 not complete | current |
| [m2-crash-campaign.md](m2-crash-campaign.md) (T25 Task 3) | 2026-10-01 | `a35c93c8` | crash and torn-tail, four rows | PASS; T25 not complete | current |
| [m2-h10-finish-and-spawn-gate.md](m2-h10-finish-and-spawn-gate.md) (H-10 4c, done) | 2026-10-02 | `m2/wire`, `cfed3b09`..`5a170e38`, merged `c20d9e92` | one finish budget; pre-exec spawn gate | real-binary rows PASS; substitutions and residuals declared | current |
| [m2-hermes-readiness.md](m2-hermes-readiness.md) | 2026-10-01, updated 10-02 | `hermes-agent==0.19.0` (source only) | frozen identity, launch, session, snapshot and transport contract | source-only contract; runtime gates covered by row 1 | current |
| [m2-hermes-reference-row.md](m2-hermes-reference-row.md) | 2026-10-02 | `m2/wire` `8e1d8344` | Hermes on DeepSeek, one live row, capture-only | PASS, sealed complete LocalOnly capture; clean key scan | current |
| [m2-amp-readiness.md](m2-amp-readiness.md) | 2026-10-11 | `@sourcegraph/amp@0.0.1791676897-g77c125` (key-free probe, Linux) | Phase 1 readiness for an Amp connector, Routes A and B | BLOCKED: transport, native store, auth and identity UNKNOWN; no connector | current |
| [v0.5.1-public-release-cleanup.md](v0.5.1-public-release-cleanup.md) | 2026-10-10, updated 10-11 | public v0.5.0 `d2b3206`; merged `b851a51` | release truth, CI scope, contribution review, Linux and macOS gates, applied settings | Linux CI tests and macOS full gate (Rust 1.98 and 1.99) exit 0; full Linux tests NOT RUN; settings read back | current |

M2 launch recipes and the early experiments describe how Codex and Claude start. For adding a
harness read [HARNESS-ONBOARDING](../HARNESS-ONBOARDING.md); for the run recipe read
[CONNECTOR-CONTRIBUTING](../CONNECTOR-CONTRIBUTING.md).

## V1 (macOS/Codex CLI foundation, branch `v1/trunk`, pin `7f01234`, tag `v0.1.0`)

Included V1 records remain in place and are historical for m2 work. Where a private historical record is omitted from this public OSS cut, its identity and status remain indexed here without a public link.

| Record | Date | Source | Claims | Result | State |
|---|---|---|---|---|---|
| [2026-09-15-codex-recon.md](2026-09-15-codex-recon.md) | 2026-09-15 | Lane Q0, real Codex CLI | five reconnaissance questions | a process outlived the sweep 3 of 3; not qualification | historical |
| [2026-09-16-linux-arm.md](2026-09-16-linux-arm.md) | 2026-09-16 | `cloud/linux-arm`, from `08f57d9` | first Linux run | pidfd delivery assertion passed; daemon cannot capture a submission on Linux | historical |
| 2026-09-16-review.md and 2026-09-16-review-evidence/ are private historical records omitted from this public OSS cut. The supporting private historical record is intentionally omitted from this public OSS cut. | 2026-09-16 | `cloud/review` | independent review of the 09-15/16 work | COMPLETE; found F0 (`Confirmed` over a live process); pin digests of `5061d74` correct | historical; cited by CHEAT-CHECKLIST |
| [2026-09-17-gate.md](2026-09-17-gate.md) | 2026-09-17 | `827e5c2`, tag `v1-gate-green-2026-09-17` | four-command gate | all four rc 0 | historical; superseded by the gate at the pin in 09-21 |
| 2026-09-17-journey-dogfood.md is private historical evidence retained internally and omitted from this public OSS cut. The supporting private historical record is intentionally omitted from this public OSS cut. | 2026-09-17 | `827e5c2` | real Codex journeys | lede corrected 09-19: defect fixed, three journeys pass; criterion 4 half-measured | historical; superseded by 2026-09-21-v1-evidence |
| [2026-09-18-f0-linux-residual.md](2026-09-18-f0-linux-residual.md) | 2026-09-18 | `v1/ci-forkbomb`, harvested 09-24 | F0 fixture on Linux | inside the documented residual window, and the window is wide | historical |
| [2026-09-19-cascade-finding-SUPERSEDED.md](2026-09-19-cascade-finding-SUPERSEDED.md) | 2026-09-19 | `ef7f309` | gate red from a kill-cascade race | mechanism disproven | superseded by cascade-correction |
| [2026-09-19-cascade-correction.md](2026-09-19-cascade-correction.md) | 2026-09-21 | `ef7f309` | the red is `/bin/ps` under contention | measured; no journey run | historical |
| [2026-09-21-v1-evidence.md](2026-09-21-v1-evidence.md) with [dir](2026-09-21-v1-evidence/) | 2026-09-21 | `7f01234`, tag `v0.1.0` | V1 criteria 1-7, one commit | gate rc 0; two journeys `overall: pass` with a dev signer; stated deviations | current for V1 only |
| [2026-09-18-v4-daemon.toml.example](2026-09-18-v4-daemon.toml.example) | 2026-09-18 | `v1/trunk` | placeholder v4 daemon config | a template, not a measurement | historical |

## Adding a record

1. Name the exact source, binaries and harness version, and keep raw output out of git
   (`spikes/*/out/`, `harnesses/`, `target/`): say "local, not in git" if you cite it.
2. State the claim and its limit in the first lines. Pass, Fail and Unknown are defined per task.
3. Add one row above. If it replaces an older record, put "superseded by" in the old row and one
   banner line after the old record's title: `> **Historical (YYYY-MM-DD).** why. Current: link.`
