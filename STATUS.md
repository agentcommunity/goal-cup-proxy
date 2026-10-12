# STATUS: where Goal Cup stands

Updated: 2026-10-12. v0.5.0 is published as source only: the [v0.5.0
release](https://github.com/agentcommunity/goal-cup-tracer/releases/tag/v0.5.0) has no attached
files, and its tag, commit and tree are in [the v0.5.1 cleanup
record](docs/qualification/v0.5.1-public-release-cleanup.md). Built binaries print `0.1.0`: the
package version is independent of source release tags. Hashes and branch names cited here are
private development history, except commits from PR #1 onward, which are on this repository's
`main`. Tickets: [LEDGER.md](LEDGER.md). How the parts fit:
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). The words: [docs/GLOSSARY.md](docs/GLOSSARY.md).

## What m2 is

`m2/wire` is the M2 recorder line. A participant runs `goalcup start --goal-file` and a coding agent
works inside an isolated home while the tracker records the native and wire legs into one
hash-linked v5 chain. Delivery is LocalOnly: no server, receipt or acknowledgement. v0.5.0 is
source-only: the tree has three connector directories and the separate private `m2-pilot` macOS
packaging allowlist selects/stages two. There is no downloadable binary or installer artifact,
staged bundle, registry publication, signing, notarization or qualification claim; a future Homebrew
installer is a separate later packaging track. The same-UID local-attacker residual remains out of
scope. The pilot is private, unsigned and not released. Nothing is qualified. `Two connector
profiles` is not a connector count: a profile is one manifest's `[capture]` table.

D42 (2026-10-04) put the V1/v4 transcript-and-hook path behind a `v1-compat` cargo feature; D48
(2026-10-11) deleted it, so the tree has one feature set and one gate (OS-30). The `v0.5.0` tag
keeps the V1 code. The seam refactors are OS-12 and are explicitly post-tag.

## Built

Detail and the test for each guarantee: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

- **Chain and reservation.** v5 writer, physical reservation on the chain file, local finish and
  `seal` (T03, T03R, T15); one finish budget plus a pre-exec spawn gate (H-10).
- **Daemon.** Ingest receiver, own-session native capture, orphan and Recovery rules, Loss
  frames chained as gaps (T05, T06, H-09, H-16), in `runtime/capture/`.
- **Proxy.** In-process CONNECT/TLS, HTTP/1.1 and WebSocket relay with streaming redaction before
  hash or disk, fail-closed past the 16 KiB hold (T04, T09–T13).
- **CLI.** `goalcup start --goal-file` and `seal` over v5, participant output in plain words,
  the key loader for Hermes (T16, H-14, H-15).
- **Verifier.** Integrity exits 0/1/2/3; decoders `codex-native`, `claude-code-native`,
  `openai-responses`, `anthropic-messages`, `openai-chat-completions` (Chat: synthetic only).
- **PH-V verifier harmonization.** Merged `b4954f64` (lane `1059d16b`): decoder versions
  Responses 2 / Anthropic 3 / Chat 2, shared framing and budgets, typed facts, OS-15 labels.
  Focused evidence and W1 cover it; no live re-decode or qualification is claimed.
- **Control and launchd hardening.** C1 `b93cd27d`: Host enforced before routing, only status and
  version public, crash-only throttled launchd agent. Code and static facts; nothing measured.
- **Harness-contract path.** PH-A `cf05d4fe`, S4a/S4b `b3c4211d`, S5 `9b29ce10`, S6
  `b047d854` and C2-0700 `31e9f04f` are merged and covered by the W1 gate. The offline kit now
  has six synthetic cases, two of them keyed (PR #8). This is not a qualified row.
- **Onboarding (2026-10-12).** Entry point: the [skill](skills/adding-a-capture-harness/SKILL.md)
  and `sh scripts/check-connector.sh <id>` (PR #7). The goldens derive the connector list, so a
  data-only connector needs no `.rs` edit (PR #4; proven for the CLI half, H-42). Three
  [trial runs](docs/research/harness-onboarding-trial.md), all by the skill's author (OS-23).
- **Amp core (2026-10-12).** D49 and D50 are in force; `*_URL` names are refused in connector env
  and dropped from inherited ones (PR #8, [amp-core-prep](docs/research/amp-core-prep.md)). No Amp
  connector. Linux script fixes and the [portability note](docs/research/platform-portability.md)
  (design, not evidence) are PR #6.
- **Connectors.** `codex-cli`, `claude-code-cli` and capture-only `hermes-cli`; Codex and Claude
  are the two selected/staged (D40, D41, T14, H-18).
- **Pilot.** Private unsigned three-binary install, source `3c542aa5` (T24).

## Measured

"Measured" means a linked record binds the result to an exact source and binary identity.

| Row | Source | Result | Record |
|---|---|---|---|
| T23 Part B, one 30-minute synthetic load | `3c542aa5` | Driver PASS, 18 assertions true; verifier exit 1 as designed (sealed, incomplete, LocalOnly) | [synthetic vertical](docs/qualification/m2-synthetic-vertical.md) |
| 3c paired tiny, Codex 0.153.0 and Claude 2.1.283 | `3c542aa5` | Both PASS: valid sealed LocalOnly, zero gaps, result `m2-capture-ok` | [real-agent progress](docs/qualification/m2-real-agent-progress.md) |
| Useful Claude (CSV summary) | `3c542aa5` | PASS, 85.01 s, 456 records, zero gaps | same |
| Useful Codex (CSV summary) | `70e8d129` | PASS, 170.28 s, 15,925 records, zero gaps | same |
| C3 partner-guide literal tiny paths | `70e8d129` | Codex and Claude PASS, zero gaps; fresh install, signal and teardown branches not exercised | same |
| T25 storage and accounting slice (4 rows) | `74d07df9..a35c93c8` | 4/4 PASS on real binaries, no production defect | [failure campaign](docs/qualification/m2-failure-campaign.md) |
| T25 crash and torn-tail slice (4 rows) | `d4e9ed43..8ce2b8d2` | 4/4 PASS on real binaries, no production defect | [crash campaign](docs/qualification/m2-crash-campaign.md) |
| H-10 finish budget and spawn gate (9 real-binary rows) | `cfed3b09`..`5a170e38`, merged `c20d9e92` | 9/9 PASS on debug builds; release drain 138.05 ms and 141.96 ms | [H-10 record](docs/qualification/m2-h10-finish-and-spawn-gate.md) |
| Hermes row 1 (0.19.0 on DeepSeek) | `8e1d8344` | PASS: verifier exit 0, 0 gaps, key scan clean, `sum.py` prints 55 | [Hermes row](docs/qualification/m2-hermes-reference-row.md) |

Failed rows stay on record: the d8 installed Codex and Claude rows
([live capture](docs/qualification/m2-live-capture.md)) and the `6d92adc6` default Codex row.
The cause of the upstream-connect failures on those rows is UNKNOWN.

## Not qualified, open

By LEDGER ID. No eight-hour, 168-hour, production, public or acknowledged-delivery claim exists.

- **Qualification:** T25 (rest), T26, T27, T28.
- **v0.5 foundations:** C1, OS-14 through OS-21 and S-08/OS-15 are merged. W1 covers code
  through private head `9fc730e6`; the later S8 documentation merges had focused evidence only.
- **v0.5 cut and proof:** the public cut (OS-01, OS-06), its link check (OS-22) and the source
  release (OS-25) are recorded; OS-23 and OS-24 evidence is not yet linked. D48 (OS-30, PR #5)
  deleted the V1 path. The v0.5.1 cleanup is merged and OS-07 settings are applied
  ([record](docs/qualification/v0.5.1-public-release-cleanup.md#applied-2026-10-11)).
- **Deferred after `v0.5.0`:** OS-12, OS-13, OS-26, OS-27, OS-28, OS-29 (remove the build's
  retained V1 manifests parsing dependency), OS-31 (D48 devserver follow-up), H-28 and H-29.
- **Amp (in flight):** [BLOCKED](docs/qualification/m2-amp-readiness.md); no `connectors/amp-cli`
  until the owner's live probe and rulings H-34 to H-38. H-39 and H-40 are optional hardening,
  not started; H-41 (executable pin shape) is decided before Amp's connector.
- **Other open work:** H-11, H-12, H-04, F-16, H-18, H-20 through H-24, H-26, H-42, H-43 and
  older OS tickets stay open until their own evidence closes them.

## Gate

**PASS** on macOS 26.6.2 arm64, rustc/cargo 1.99.0, at `d2c42de` (the Rust head of
`post-v060-review-fixes`; later commits change Markdown and `check-connector.sh` and its test):
build, fmt, clippy exit 0; `cargo test --workspace` 2,307 passed, 0 failed, 40 ignored, 141
targets. By-hand suites not run. One earlier run under load failed `process_kill`'s 5-second
settle wait; the rerun alone passed. The pre-cut W1 gate (`9fc730e6`, before D48) is in
[the cleanup record](docs/qualification/v0.5.1-public-release-cleanup.md#local-validation).
The integrator runs the one [gate](README.md#the-gate) on each whole-wave head; a timing failure
seen only under load is rerun alone, and only an isolated reproduction blocks.

## V1 on `v1/trunk`

- V1 is the macOS/Codex CLI foundation, pinned at `7f01234` (tag `v0.1.0`); the private V1
  repository's `main` is at `0c5f477`.
- Acceptance criteria 1 to 6 are met at the pin; criterion 7 settles on the owner's read.
- No 168-hour run is part of V1. It is not event-ready, not production-anchored, not public.
- The `v1/trunk` hook leg is not in this tree (D48); the verifier still ships the thirteen V1
  checks for v4 traces.
  The supporting private historical record is intentionally omitted from this public OSS cut.
