# STATUS: where Goal Cup stands

Updated: 2026-10-07. The merged integration head is `cc52f967` (full hash
`cc52f967f40e1f3e6aba8f52a7f3572df26d242f`). The tested integration head is `9fc730e6`
(full hash `9fc730e6b4631700b400f347835de147cd528dbe`). The five S8 documentation merges after
that test are merged evidence, not a new whole-tree gate. Tickets:
[LEDGER.md](LEDGER.md). How the parts fit: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). The words:
[docs/GLOSSARY.md](docs/GLOSSARY.md). No public-repository push, cut publication or tag is
authorized.

## What m2 is

`m2/wire` is the M2 recorder line. A participant runs `goalcup start --goal-file` and a coding
agent works inside an isolated home while the tracker records the native and wire legs into one
hash-linked v5 chain. Delivery is LocalOnly: no server, receipt or acknowledgement. The approved
v0.5 medium is source-only: the tree has three connector directories and the private pilot has
two selected/staged connectors. There is no downloadable binary or installer artifact, staged
bundle, registry publication, signing, notarization or qualification claim; a future Homebrew
installer is a separate later packaging track. The same-UID
local-attacker residual remains out of scope. The pilot is private, unsigned and not released.
Nothing is qualified.

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
- **v0.5 pre-tag foundations:** C1, OS-14 through OS-21 and S-08/OS-15 are merged. W1 covers
  code through the tested head above; S8 documentation has focused evidence but awaits Step11's
  exact-head whole-tree gate.
- **v0.5 cut and proof:** OS-22 cut-link repair, OS-23 outsider Gemini, OS-24 fresh clone, then
  OS-25 exact-head evidence and explicit owner GO.
- **Explicitly post-tag:** OS-12, OS-13, OS-26, OS-27, OS-28 and OS-29 (remove the default
  build's retained V1 manifests parsing dependency), plus H-28 and H-29. They remain ticketed work
  but do not block `v0.5.0`.
- **Other open work:** H-11, H-12, H-04, F-16, H-18, H-20 through H-26 and older OS tickets remain
  unless their own evidence closes them; this boundary does not silently mark them done.

## Gate

The W1 full gate is **PASS** on exact tested integration head `9fc730e6` (full hash
`9fc730e6b4631700b400f347835de147cd528dbe`): 22 commands, no timing waivers and all exits zero.
Default workspace tests: 2,300 passed, 0 failed, 40 ignored. `v1-compat` workspace tests:
3,038 passed, 0 failed, 43 ignored. The failure and crash campaigns passed 6/6 and 12/12;
the default synthetic set passed 8/8 and the v1 real-daemon set 3/3. Evidence is local, not in git:
`target/v05-wave1-9fc730e6b463/gate-summary.md` in the `m2-wire` worktree. This records the gate
only; it does not claim a tag, release, publication or live qualification.

For v0.5, every lane keeps focused failing-first/acceptance proof and independent review. The
integrator runs V -> D -> C once on each whole-wave head. A timing failure seen only under load is
rerun alone; only an isolated reproduction blocks. After the final cut proofs, the exact proposed tag
head needs current evidence. No push, public cut publication or `v0.5.0` tag occurs without the
owner's explicit GO.

## V1 on `v1/trunk`

- V1 is the macOS/Codex CLI foundation, pinned at `7f01234` (tag `v0.1.0`); trunk `main` is
  at `0c5f477`.
- Acceptance criteria 1 to 6 are met at the pin; criterion 7 settles on the owner's read.
- No 168-hour run is part of V1. It is not event-ready, not production-anchored, not public.
- The `v1/trunk` hook leg and thirteen checks are not on m2.
  The supporting private historical record is intentionally omitted from this public OSS cut.
