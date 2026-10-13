# Connector contributions

How do I submit and review a schema-1 connector, and what evidence supports its claim? For a
contributor or agent adding harness data. Start with [HARNESS-ONBOARDING](HARNESS-ONBOARDING.md)
for the worksheet and phase gate; this file owns the connector checklist and tiny-row procedure.
Updated: 2026-10-12.

A connector launches and captures a harness. Its `harness.toml` is data: it declares the launch,
isolated home, provider/auth hosts and paths, protocol hints, native source and redaction
boundaries. A decoder is optional and is a separate verifier contribution. A connector does not
need a decoder to retain supported traffic.

## Add or change a connector

The source tree currently contains three connector directories and three generated pins:
`codex-cli`, `claude-code-cli` and `hermes-cli`. Their schema-1 files are the source facts; a
directory or pin does not by itself prove a package, release or qualified harness.

A common data-only PR carries exactly these concerns:

1. `connectors/<id>/harness.toml` (`schema_version = 1`) and reviewed `files/**` seeds. Use a
   shipped connector as a shape reference, but measure the new harness rather than copying its
   auth, CA, home or identity rules.
2. The regenerated directory pin. Run `sh scripts/pin-manifests.sh` after the edit; it rewrites
   the connector's line in `crates/goalcup-protocol/pins/connector-dirs.txt`. Never hand-edit or
   hand-merge that line.
3. The harness's public facts sheet `docs/harnesses/<id>.md`, filled from the
   [template](../skills/adding-a-capture-harness/templates/harness-facts.md) with a source for
   every fact (see [docs/harnesses](harnesses/README.md) and
   [HARNESS-ONBOARDING](HARNESS-ONBOARDING.md)). It replaces the old readiness record. Do not
   label a contributor's source or synthetic result qualified.
4. A new generic harness-kit fixture under
   `crates/goalcup-cli/tests/fixtures/harness-kit/<id>/` (named after the connector id), containing
   `harness.toml`, `stand-in.py` and `expect.toml` (plus only reviewed declared seeds). The case
   exercises the connector's schema-1 path; do not substitute an existing baseline case.
5. Evidence that the current baseline harness-kit suite passes. Build the workspace first, then
   run `cargo test -p goalcup-cli --test full harness_contract_kit::`; the six existing baseline cases
   are listed in the [skill, step 8](../skills/adding-a-capture-harness/SKILL.md#checklist).
   `sh scripts/check-connector.sh <id>` runs it with the pin, schema and golden checks.
6. The regenerated harness goldens. The tests read the connector list from `connectors/`, so
   they need no edit. On macOS, run
   `GOALCUP_REGEN_HARNESS_GOLDEN=1 cargo test -p goalcup-cli --test full harness_contract_golden::`
   and the same with `-p goalcup-daemon`, then commit the JSON diff under
   `crates/goalcup-cli/tests/fixtures/harness-golden/`: the new `<id>.json` and the new rows.

After a connector edit, also run
`cargo test -p goalcup-protocol --test it -- harness_conformance:: connector_pins::`; these
checks cover the schema and the generated directory pin. The
workspace build must precede the harness kit because the kit launches the built binaries.

The schema-1 loader admits one native source and the identity pairs exercised by the kit:
`none` with no binding, `preassigned` plus `at_launch`, `stdout_json` plus `at_launch`, or
`stderr_line` plus `deferred`. Native forms are `append_only` and `snapshot`; SQLite is refused
as a schema-1 native form and needs a separate core decision. The manifest carries hosts, paths
and protocol hints, not a transport field. Record the candidate's observed transport and body
encoding in the facts sheet (section 6); HTTP/2-only or unknown body encoding is a blocker, not a support
claim.

A new policy row (CA variable, credential name, identity pair, native-store form or interpreter)
is a separately reviewed core PR with a DECISIONS row. A new semantic decoder is a separately
reviewed verifier PR; use [writing-a-decoder](../skills/adding-a-capture-harness/writing-a-decoder.md).
Do not add `.rs` files, policy changes or decoder code to an ordinary connector-data PR.

## Readiness gate

Before editing the connector, fill the facts sheet (`docs/harnesses/<id>.md`)
from the exact candidate. Write `UNKNOWN`, never a guess. Phase 1 stops with a recorded blocker
if any required fact is UNKNOWN in transport; native storage or scope; isolation or auth; or
identity source or binding. A literal unsupported answer is also a blocker.

Record exact executable version and SHA-256 only when the source or an official artifact proves
them. Record package/source provenance only when directly evidenced; otherwise write UNKNOWN.
Do not turn a package label, source tree, directory digest, synthetic kit result or readiness
record into a release or qualification claim.

The child environment is built from declared values. Never set a provider base URL, write app-wide
settings or trust roots, copy a credential, or retain one (D1). Auth hosts tunnel opaquely. An
approved provider host is intercepted only on an `evidence_paths` or `prefix_paths` path; other
paths are forwarded and not retained. The protocol hint is declared data, never inferred.

| Source field | What the reviewer must establish |
|---|---|
| `capture.args`, executable | headless argv, goal placement, exact entry point, version and hash |
| `capture.home`, `env` | private HOME/config home, real state dirs, seed contents and modes |
| `capture.credential`, `capture.ca` | variable names and process scope; no secret values or app-wide trust |
| `capture.hosts`, `auth_hosts` | provider host, narrow evidence paths and protocol hint; the facts sheet records observed transport |
| `capture.native` | one own-session store, form, root/pattern and scope; no request dump or credential store |
| `capture.identity` | measured carrier, grammar/fields and binding to that own-session store |
| `redaction.extra_patterns` | plan, billing, quota and key-fragment coverage from admitted sources |

## Reviewing a connector PR

A connector is trusted executable surface. The pin catches directory tampering after a build; it
does not make a careless seed safe. A code owner reads every field and seed. `harness_conformance`
refuses many invalid values, but validation is not a substitute for review.

1. Read every `files/**` file as code. Check commands, hooks, plugins, helpers, URLs, endpoint
   settings, MCP servers, profiles and permissions. Nothing may redirect the model endpoint or
   execute outside the run.
2. Check `capture.args`: no endpoint, settings indirection, native-store disablement or absolute
   path. If argv is pre-scanned, `argv_guard` covers every acted option and abbreviation.
3. Check `capture.env`: justify every value; `SHELL` is only `/bin/zsh`, `/bin/bash` or `/bin/sh`.
   No Rust list names a connector's argv or env any more (D51): this step is the control for
   what the generic rules cannot see. Check that no flag capture needs was dropped (one headless
   turn, the session file kept, self-update off: `DISABLE_AUTOUPDATER = "0"` turns it back on),
   and that no argument or variable names a command, a hook, an MCP server, a host or a file
   outside the run. How much the harness may do (model, sandbox, permissions) is the
   participant's, not the connector's: a connector does not limit it.
   Look for telemetry exporters and code loaders. Do not request a `NO_PAGER` exception: the
   `PAGER` suffix rule refuses it.
4. Check `capture.executable`: `program` is the harness entry point, not an interpreter or moving
   updater wrapper, and `version_args` (if any) only asks for the version. There is no build
   digest to re-derive: builds are recorded, not pinned (D53).
5. Check hosts: only the provider, narrowest evidence paths and a real hint; no registry or
   unrelated host. Auth hosts are sign-in/refresh tunnels and cannot supply model evidence.
6. Check native capture: root is inside the run/config home, `{id}` scopes a shared store, and
   the pattern cannot match request dumps, future credential stores, seed files or key-fragment
   logs. Schema 1 has exactly one native source.
7. Check redaction: fixture-observed plan, billing, quota and key fields are covered by admitted
   recognizers. Redact committed fixtures, inspect them by eye, and omit real prompts, traces and
   keys.
8. Check home and secrets: private HOME every time; `shared` is justified and serialized; per-run
   seeds contain no credential, URL or user data and use safe modes. API keys stay child-only and
   out of homes, logs, records and evidence.
9. Check identity: source matches the measured carrier, `stderr_line` grammar is exact, and
   `stdout_json` start fields precede model text and escape model text inside events.
10. Re-run `sh scripts/pin-manifests.sh` and confirm one intended pin-line diff. Confirm the
    harness-golden diff only adds the new connector and changes no other connector's values.
    Run the generic kit after a workspace build. A policy row or decoder belongs in its own
    reviewed PR.

## Tiny real row

A maintainer may run one exact candidate with their own subscription or key after readiness and
review. The contributor never opens a key file. Use a fresh private launch directory and the
reviewed connector inputs; the goal is exactly the 13 ASCII bytes `m2-capture-ok` in
`result.txt`, with no newline, followed by a read-back tool call. Verify the result bytes
independently and retain the actual start/status/integrity/research/checker exits.

Use a bounded 600-second watchdog plus a 25-second finish allowance; never wrap the parent in
`timeout` or kill the parent. Before signaling, read only `last_pid` from the private entry record
and prove with `ps` that it is a direct child of this CLI. If the entry, child PID or ownership
proof is missing, record incomplete cleanup and send no signal. Otherwise send `TERM`, wait 10
seconds, and send `KILL` only while the same parent still owns the child. If the parent remains
active after the finish allowance, stop, record incomplete cleanup and run no verifier or further
row. For a key-bearing row, a clean key-fragment scan is mandatory before accepting evidence.

Check the selected request and response separately, the goal/launch/local-finish chain, and one
own-session native open plus scoped segments. A gap, missing or unbound native source, stale
recorder, unknown semantics, empty trace or failed task lowers the claim exactly as the guide
states. A live row is evidence for that candidate and source; it is not a package release or a
general qualification matrix.

## Claim boundary

| Finding | Permitted conclusion |
|---|---|
| Pin, version or preflight refusal | compare installed payload hashes; source is not an install |
| Unknown transport/storage/isolation/auth/identity | Phase 1 blocked; preserve UNKNOWN and stop |
| No request or response | capture is not qualified; inspect admission, host, path and auth tunnel |
| HTTP/2, unsupported body or missing native source | report unsupported/partial; keep bytes and gaps |
| Bytes retained, semantics unknown | capture-only for the stated coverage; no decoder claim |
| Gap, stale recorder or unknown accounting | report incomplete; never call it clean or qualified |
| Task fails while recorder passes, or the reverse | report each outcome separately |

Claim levels are planned, implemented, installed, synthetic tested, short real capture-qualified
and short decoded-qualified. A passing kit covering the six baseline cases plus the new connector
case is **synthetic tested** only. Qualification requires its own evidence and maintainer review;
nothing here implies restart reliability, long autonomy, delivery, package publication or release.
