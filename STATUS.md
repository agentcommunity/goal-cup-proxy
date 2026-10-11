# STATUS: where Goal Cup stands

Updated: 2026-10-11. v0.5.0 is published as source only: the [v0.5.0
release](https://github.com/agentcommunity/goal-cup-tracer/releases/tag/v0.5.0) has no attached
files, and its tag, commit and tree are in [the v0.5.1 cleanup
record](docs/qualification/v0.5.1-public-release-cleanup.md). Built binaries print `0.1.0`: the
package version is independent of source release tags. Commit hashes and branch names cited here
refer to private development history not included in this repository. Tickets:
[LEDGER.md](LEDGER.md). How the parts fit: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). The words:
[docs/GLOSSARY.md](docs/GLOSSARY.md).

## What m2 is

`m2/wire` is the M2 recorder line. A participant runs `goalcup start --goal-file` and a coding agent
works inside an isolated home while the tracker records the native and wire legs into one
hash-linked v5 chain. Delivery is LocalOnly: no server, receipt or acknowledgement. v0.5.0 is
source-only: the tree has three connector directories and the separate private `m2-pilot` macOS
packaging allowlist selects/stages two. There is no downloadable binary or installer artifact,
staged bundle, registry publication, signing, notarization or qualification claim; a future Homebrew
installer is a separate later packaging track. The same-UID local-attacker residual remains out of
scope. The pilot is private, unsigned and not released. Nothing is qualified.

The phrase `two connector profiles` is not a connector-count claim: a profile is one connector
manifest's `[capture]` table, while the pilot selects/stages two connectors.

Since D42 (2026-10-04) the V1/v4 transcript-and-hook path compiles only with the `v1-compat` cargo
feature, off by default. OS-11 is merged; the seam refactors are OS-12 and are explicitly
post-tag.

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
- **Verifier.** Integrity reader with exits 0/1/2/3 and five built-in decoders: two native
  (`codex-native`, `claude-code-native`) and three wire (`openai-responses`,
  `anthropic-messages`, `openai-chat-completions`). The Chat decoder has synthetic coverage only;
  the earlier Hermes/DeepSeek live row was not re-decoded.
- **PH-V verifier harmonization.** Final lane `1059d16b`, merged as `b4954f64`: wire decoder
  versions Responses 2 / Anthropic 3 / Chat 2; shared framing, aggregate budget and close
  classification; registry-owned selection; the closed ten observation kinds; shared bounded
  Recovery-lineage calculation; typed integrity/anchor/key-source/terminal facts; and
  presentation-only OS-15 connector labels. Independent review approved exact tip `1059d16b`;
  focused evidence is 341 passed, 0 failed, 1 ignored, with fmt, shared-wire, default/v1 workspace
  no-run and all-target clippy exits 0. W1 at `9fc730e6` also covers it; no live re-decode or
  qualification is claimed.
- **Control and launchd hardening.** C1 merged as `b93cd27d`: the bound listener's Host is
  enforced before routing, only status and version are public, the forgeable placeholder key is
  named at startup, and the launchd agent is crash-only/throttled with owned-label teardown. These
  are code and static-policy facts; no launchd execution was measured.
- **Harness-contract path.** PH-A `cf05d4fe`, S4a/S4b `b3c4211d`, S5 `9b29ce10`, S6
  `b047d854` and C2-0700 `31e9f04f` are merged and covered by the W1 gate. The offline kit has
  four synthetic cases, including two data-only case commits. This is not a qualified row.
- **Connectors.** The three connector directories are `codex-cli`, `claude-code-cli`, and
  capture-only `hermes-cli`; the two selected/staged connectors are Codex and Claude (D40, D41,
  T14, H-18).
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
  release (OS-25) are recorded. OS-23 outsider Gemini and OS-24 fresh clone: evidence not yet
  linked. The v0.5.1 cleanup merged as `328ffa9`; OS-07 settings are applied
  ([record](docs/qualification/v0.5.1-public-release-cleanup.md#public-checks-and-settings)).
- **Deferred after `v0.5.0`:** OS-12, OS-13, OS-26, OS-27, OS-28 and OS-29 (remove the default
  build's retained V1 manifests parsing dependency), plus H-28 and H-29.
- **Other open work:** H-11, H-12, H-04, F-16, H-18, H-20 through H-26 and older OS tickets remain
  unless their own evidence closes them; this boundary does not silently mark them done.

## Gate

The last recorded whole-tree gate is W1, **PASS** on private integration head `9fc730e6`, before
the public cut. It does not validate public commit `d2b3206` or v0.5.1, and no whole-tree gate
result for `d2b3206` is linked here. W1 counts and identities, and the v0.5.1 local results, are in
[the cleanup record](docs/qualification/v0.5.1-public-release-cleanup.md#local-validation).

Every lane keeps focused failing-first/acceptance proof and independent review. The integrator
runs V -> D -> C once on each whole-wave head. A timing failure seen only under load is rerun
alone; only an isolated reproduction blocks.

## V1 on `v1/trunk`

- V1 is the macOS/Codex CLI foundation, pinned at `7f01234` (tag `v0.1.0`); trunk `main` is
  at `0c5f477`.
- Acceptance criteria 1 to 6 are met at the pin; criterion 7 settles on the owner's read.
- No 168-hour run is part of V1. It is not event-ready, not production-anchored, not public.
- The `v1/trunk` hook leg and thirteen checks are not on m2.
  The supporting private historical record is intentionally omitted from this public OSS cut.
