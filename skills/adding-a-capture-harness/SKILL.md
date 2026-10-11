---
name: adding-a-capture-harness
description: Use when adding or supporting a CLI coding-agent harness in Goal Cup capture, writing schema-1 connector data under connectors/, onboarding a harness with no connector, or deciding whether retained traffic needs a separate verifier decoder.
---

# Adding a capture harness

## Overview

A normal harness contribution is data-only: a schema-1 `harness.toml`, reviewed seeds, one
regenerated connector-directory pin, readiness evidence and the generic offline harness kit.
Policy rows and semantic decoders are separate reviewed changes. A synthetic kit pass is not a
live qualification, package publication or release claim.

## When to use

- A candidate CLI agent writes session data; probe whether its observed transport can be captured
  through `HTTPS_PROXY`. GUI apps and HTTP/2-only clients are blockers; record them and stop.
- A contributor needs to add or audit `connectors/<id>/harness.toml` (harness-contract schema 1).

## Phases: do not start a phase until the previous gate passes

1. **Probe.** Use a private `HOME`, XDG variables and harness home under a temp directory. Fill
   the [worksheet](../../docs/HARNESS-ONBOARDING.md#1-capability-worksheet) from source and
   install evidence. Phase 1 stops with `BLOCKED` on any `UNKNOWN` in transport, native
   storage/scope, isolation/auth, or identity source/binding; do not guess or wire around it.
2. **Record readiness.** Write `docs/qualification/m2-<name>-readiness.md` (`<name>` is the id
   without `-cli`). Record exact executable version/SHA-256, source and package provenance only
   when evidenced, transport, native store/scope, isolation/auth, identity and unknown semantics.
   An absent package, release or qualification record stays `UNKNOWN`.
3. **Wire data.** Add `connectors/<id>/harness.toml` with `schema_version = 1` and reviewed
   `files/**` seeds, plus a new generic fixture under
   `crates/goalcup-cli/tests/fixtures/harness-kit/<case>/` containing `harness.toml`, `stand-in.py`
   and `expect.toml` for this connector. Run `sh scripts/pin-manifests.sh` in the same change.
   Use only admitted policy rows and identity pairs. A new CA, credential, identity, native-store
   form or interpreter is a separate core PR with a DECISIONS row. Do not add `.rs` files here.
   Run `cargo test -p goalcup-protocol --test harness_conformance --test connector_pins`.
4. **Prove the generic kit offline.** Build first, then run on macOS:
   ```
   cargo build
   cargo test -p goalcup-cli --test harness_contract_kit
   ```
   The current baseline suite has four synthetic cases: `controlled-workload`,
   `synthetic-preassigned`, `synthetic-stderr-token` and `synthetic-stdout-json`; the new
   connector fixture is an additional case exercised by the same command. They use stand-ins, a
   test-owned proxy and private homes. They prove schema-1 wiring only; they never use a provider
   or real key.
5. **Review.** Run the [connector checklist](../../docs/CONNECTOR-CONTRIBUTING.md#reviewing-a-connector-pr)
   over every field and seed. Confirm redaction, private homes, no base URL, exact executable
   evidence and one intended pin diff. A decoder is a separate verifier PR; capture-only is
   an honest claim when bytes are retained but semantics are unknown.
6. **Live row.** A maintainer may run one exact candidate with their own subscription or key; the
   contributor never opens the key file. Follow [tiny real row](../../docs/CONNECTOR-CONTRIBUTING.md#tiny-real-row)
   and retain independent task output, request/response, own-session native evidence, privacy,
   integrity and all exits. Do not call this package publication, release or qualification unless
   its own evidence supports that exact claim.
7. **Record.** Link the readiness record and, if it exists, the maintainer's exact qualification
   evidence. Do not edit STATUS, LEDGER or DECISIONS from an outside PR. Claim capture-only unless
   a reviewed decoder accepts the retained streams: [writing-a-decoder](writing-a-decoder.md).

## Schema-1 quick reference

| Check | Schema-1 field or source rule |
|---|---|
| CA trust | `[capture.ca]` `env`/`mode`; use an admitted CA row, never system trust |
| Credential | `[capture.credential].env`; child-only secret; per-run home where required |
| Env, argv, seeds | `[capture.env]`, `args`, `home.seed`; deny startup/code-loader surprises |
| Native store | exactly one `[[capture.native]]`; `append_only` or `snapshot`; SQLite is a core decision |
| Identity | `[capture.identity]`; admitted pairs are none/no binding, preassigned/at-launch, stdout-json/at-launch, stderr-line/deferred |
| Key scan | [`crates/goalcup-cli/src/key_scan.rs`](../../crates/goalcup-cli/src/key_scan.rs); a hit fails the row |

The daemon keeps bytes and health; interpretation stays in the verifier (D3). Never set a model
base URL: route through the process-scoped proxy and CA. Auth hosts tunnel opaquely. The protocol
hint is declared data, never inference.

## Common mistakes

- Treating a connector directory, pin, package label or synthetic pass as a release or qualified
  result; write the exact evidence and keep missing facts UNKNOWN.
- Continuing after an UNKNOWN transport, native storage/scope, isolation/auth or identity fact;
  stop Phase 1 and record the blocker.
- Adding policy or decoder code to a data-only PR; make the separate reviewed core/verifier PR.
- Pointing a test or probe at real HOME; run in a child with private homes and assert real HOME is
  unchanged.
- Assuming a SQLite store fits native capture; schema 1 refuses it and needs a maintainer decision.
- Assuming a key stays in the environment; prove it never persists in the home, chain, logs or
  evidence.
- Merging because the kit passes; every connector still needs source-linked readiness and review.
