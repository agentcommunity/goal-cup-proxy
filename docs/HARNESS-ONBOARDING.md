# Harness onboarding

How does a contributor establish a truthful schema-1 connector claim? For a fresh contributor
or agent preparing harness data. This guide separates connector data, policy, native capture and
semantic decoding; [CONNECTOR-CONTRIBUTING](CONNECTOR-CONTRIBUTING.md) owns the reviewer checklist
and real-row procedure. Updated: 2026-10-11.

Read [CONTRIBUTING](../CONTRIBUTING.md), [STATUS](../STATUS.md), [GLOSSARY](GLOSSARY.md), and
DECISIONS D1, D40, D41, D42 and D43 first. The source tree currently has exactly three connector
directories and pins: `codex-cli`, `claude-code-cli` and `hermes-cli`. A source directory, pin or
synthetic result is not a package, release or qualification record.

## 1. Capability worksheet

Fill this from the exact candidate's source and install metadata before editing a profile. Write
`UNKNOWN`, never a guess. Phase 1 stops: record `BLOCKED`, preserve the evidence and do not wire
the connector if any required fact is UNKNOWN in transport; native storage or scope; isolation
or auth; or identity source or binding. A literal unsupported answer is a blocker too.

| Capability | Record | Where it lands |
|---|---|---|
| Executable | resolved entry point, version, SHA-256, and provenance if evidenced | `[capture.executable]` |
| Headless launch | exact argv, goal placement and continuation | `[capture].args`, `goal_flag` or `--` |
| Transport | provider/auth hosts, paths, observed transport/body encoding, CA variable | readiness evidence; related manifest data is `[capture.ca]`, `hosts`, `auth_hosts` |
| Isolation/auth | HOME, config-home variable, real state dirs, credential lifecycle and scope | `[capture.home]`, `credential`, `env` |
| Native storage/scope | JSONL/snapshot, own-session root/pattern and cross-run binding | `[[capture.native]]` |
| Identity source/binding | where the own-session ID appears and when it binds | `[capture.identity]` |
| Semantics | event shapes and an existing decoder, or explicit unknown/capture-only | verifier claim; no decoder in this PR |
| Privacy | possible secrets in headers, bodies, seeds and native files | redaction and key-fragment review |

Definitions are in [harness.rs](../crates/goalcup-protocol/src/harness.rs),
[CONTRACTS harness contract v1](CONTRACTS.md#harness-contract-v1),
[capture_policy.rs](../crates/goalcup-protocol/src/capture_policy.rs),
[launch.rs](../crates/goalcup-cli/src/launch.rs),
[native_scope.rs](../crates/goalcup-daemon/src/runtime/capture/native_scope.rs), and the
[Decoder v1 API](../crates/goalcup-verifier/src/capture_analysis/api.rs).

## 2. Schema-1 seam map

`connectors/<id>/harness.toml` is harness-contract schema 1 (`schema_version = 1`), the complete
data recipe for one connector. The loader requires one native source and an admitted identity pair.
It accepts native `append_only` or `snapshot`; the manifest carries hosts, paths and protocol
hints, not a transport field. Transport and body encoding are required readiness evidence, and
an UNKNOWN there is a Phase 1 blocker.
SQLite is refused by schema 1 and needs a separate core decision. Optional extra redaction rules
must use admitted recognizers.

| Need | Contribution boundary |
|---|---|
| Existing argv, host/path, CA, credential, home, native form or identity pair | connector data plus regenerated directory pin |
| New CA/credential/identity/native-store/interpreter policy row | separate core PR with a DECISIONS row |
| Meaning of retained bytes | separate verifier decoder PR; a connector may be capture-only |
| Exact candidate readiness and a live result | readiness/qualification evidence, never an inferred claim |

The common data-only PR is `connectors/<id>/` (manifest and reviewed seeds), its generated line
in `crates/goalcup-protocol/pins/connector-dirs.txt`, the regenerated harness goldens, a new
generic harness-kit fixture case under `crates/goalcup-cli/tests/fixtures/harness-kit/<case>/`
containing `harness.toml`, `stand-in.py` and `expect.toml`, a readiness record, and evidence that
the current baseline kit passes. Do not add `.rs` files, policy rows or decoder code to that PR.
The tests read the connector list from `connectors/`, so a new folder needs no test edit.

## 3. Generic harness-contract kit

The kit is the current schema-1 acceptance fixture, not a provider or release test. Build the
workspace before running it:

```text
cargo build
cargo test -p goalcup-cli --test harness_contract_kit
```

On macOS the current baseline suite runs six synthetic stand-ins, each with `harness.toml`,
`stand-in.py` and `expect.toml`; a connector PR adds a new fixture case for its own path:

- `controlled-workload`: no identity binding and an append-only native file;
- `synthetic-preassigned`: launch-assigned identity and `{id}`-scoped append-only file;
- `synthetic-stderr-token`: deferred `stderr_line` identity and append-only file;
- `synthetic-credential-stderr`: the same with a credential. Its `expect.toml` names
  `credential_env`; the runner hands an invented value through the child-secret path, then
  checks the stand-in received it and that no fragment is in the trace, run directory or output;
- `synthetic-stdout-json`: launch-assigned `stdout_json` identity and a snapshot, including a
  seeded file and argv guard;
- `synthetic-credential-stdout-json`: a keyed `stdout_json` case; one stdout reader binds the
  identity and scans for the key.

The kit uses a test-owned proxy, `Origin::Dev`, private homes and Python stand-ins. It does not
read a participant home, provider credential or package registry. A passing kit proves only
synthetic schema-1 wiring and isolation checks. It does not prove provider access, a published
package, a release, a semantic decoder or a qualified live row.

## 4. Evidence and privacy

Record the exact candidate executable version and SHA-256 only when the source or an official
artifact proves them. Package/dependency provenance, publication and release state are separate
facts; write `UNKNOWN` when their evidence is absent. Keep raw output in ignored directories.
Committed fixtures go through `spikes/redact.py` and an eye review; never commit keys, real
transcripts, `harnesses/`, `target/` or `spikes/*/out/`.

Every probe, test and row uses a private HOME and harness home. Never set a model base URL, copy
an app-wide config or trust root, or retain a credential. Auth hosts tunnel opaquely. The key
value never enters a manifest, argv, log, record or evidence. A new provider key or CA name is a
policy change, not a connector-data shortcut.

## 5. Prepare a contribution

The ordered steps with expected output are in the
[skill](../skills/adding-a-capture-harness/SKILL.md); `sh scripts/check-connector.sh <id>` runs
the pin, schema, golden and kit checks below in one command.

- [ ] Worksheet has source-linked values, or explicit UNKNOWN/blockers; Phase 1 has no UNKNOWN in
  transport, native storage/scope, isolation/auth or identity source/binding.
- [ ] `harness.toml` uses schema 1 and only admitted policy and identity values; seeds are safe.
- [ ] `sh scripts/pin-manifests.sh` was run and the intended connector pin is the only pin diff.
- [ ] `cargo test -p goalcup-protocol --test harness_conformance --test connector_pins` passed.
- [ ] The harness goldens were regenerated on macOS, so the daemon's live goldens are written too:
  `GOALCUP_REGEN_HARNESS_GOLDEN=1 cargo test -p goalcup-cli --test harness_contract_golden`, then
  the same with `-p goalcup-daemon`. The JSON diff under
  `crates/goalcup-cli/tests/fixtures/harness-golden/` is in the PR: new files and rows for the
  new connector, and no change to another connector's values.
- [ ] The workspace build ran before `harness_contract_kit`; all discovered cases passed (the six
  baseline cases plus the new connector case).
- [ ] Readiness evidence names source revision, candidate path/version/hash, transport/native/
  isolation/auth/identity facts and remaining unknown semantics without secrets.
- [ ] If a maintainer ran a real row, the PR links the exact record; otherwise claim only
  synthetic tested. Do not edit STATUS, LEDGER or DECISIONS in an outside PR.
- [ ] A human read the whole diff, AI assistance is disclosed, and the connector reviewer
  checklist is complete.

Anyone may send a connector PR. A maintainer reviews trusted connector data and acknowledges a
PR within 7 days; acknowledgement is not a promise to merge or qualify it. An outside contributor
stops after the data, readiness, generic kit and review gates. A live row uses the maintainer's
own subscription or key; the contributor never opens the key file.

## 6. Diagnose and claim only what passed

| Finding | Permitted conclusion |
|---|---|
| Unknown readiness fact | Phase 1 blocked; preserve UNKNOWN and stop |
| Pin/version/preflight refusal | compare installed bytes; source is not an install |
| Unsupported transport/body or no response | unsupported/partial capture; not qualified |
| Missing or unbound native source | wire-only partial; no own-session claim |
| Gap, stale recorder or unknown accounting | incomplete; never call it clean |
| Bytes retained but semantics unknown | capture-only for stated coverage; no decoder claim |
| Task fails while recorder passes, or reverse | report task and recorder outcomes separately |

Claim levels are **planned**, **implemented**, **installed**, **synthetic tested**, **short real
capture-qualified** and **short decoded-qualified**. A synthetic kit result is the named fixture
only. Qualification requires its own source-linked evidence and maintainer review; nothing here
implies restart reliability, long autonomy, delivery, package publication or release.

## 7. Handoff record

Copy this into the readiness or qualification record. Never include secret values or fingerprints.

```text
Claim sought / actual outcome:
Source revision; dirty inputs:
Candidate executable path/version/SHA-256; package provenance (or UNKNOWN):
Connector directory pin; harness-contract schema:
Transport hosts/paths/version/body; native storage/form/scope:
Isolation roots (metadata only); auth variable names and lifecycle:
Identity source/binding and crosscheck; semantic decoder or UNKNOWN:
Generic kit command, build/test exits and all discovered case results (six baseline plus the new
connector case):
Real-row ENTRY/RUN_DIR/TRACE and start/status/integrity/research/checker exits (if any):
Independent result bytes and hash (if any):
Redaction, gaps, unknowns, private artifact paths and deferred coverage:
Reviewer, permitted claim and qualification evidence link (or UNKNOWN):
```
