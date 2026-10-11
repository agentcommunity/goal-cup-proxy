# Amp core preparation

Which core changes would let a reviewed Amp CLI connector land later, and what does each one
change? For maintainers reviewing these changes one at a time, and for whoever writes the Amp
connector after the owner's live probe.
Updated: 2026-10-11.

Status: **proposed**. Nothing here admits an Amp connector. No `connectors/amp-cli` exists, and
none may be added until the owner's live probe settles the UNKNOWN rows of the Amp readiness
record, `docs/qualification/m2-amp-readiness.md` (on branch `claude/v060-amp-readiness`, not yet
on this branch). The rows cited below are rows of that record's "Mapping to Goal Cup policy"
table and its "Core changes Amp would need" list.

## 1. Refuse `*_URL` env names as endpoints

- **What.** `env_name_denied` in
  [capture_policy.rs](../../crates/goalcup-protocol/src/capture_policy.rs) refuses any name that
  ends in `_URL` with "redirects an endpoint (D1)", as its last rule.
- **Why.** Readiness row "`AMP_URL` endpoint override": Amp reads `AMP_URL` as its server
  address, and the name passed the name rules. Only the value rule (`://`) stopped it.
- **Files.** `capture_policy.rs`; test in
  [harness_schema.rs](../../crates/goalcup-protocol/tests/harness_schema.rs)
  (`a_url_named_variable_is_refused_as_an_endpoint`).
- **Risk.** Low. The rule is broad (`*_URL`, not only `AMP_URL`). Every shipped connector, kit
  fixture and V1 `[launch.env]` was checked: none sets a `_URL` name. The rule runs last, so a
  name another rule already refuses (`AUTH_URL`, `GOALCUP_UPLOAD_URL`, `PIP_INDEX_URL`) keeps its
  reason, and the pinned harness goldens do not move. A future harness that needs a harmless
  `*_URL` switch would need a core change.
- **Status.** Implemented on this branch.

## 2. Admit `AMP_API_KEY` as a credential

- **What.** `CREDENTIAL_ENV` gains `AMP_API_KEY`. The value grammar is unchanged and shared:
  `child_secret_named` in [launch.rs](../../crates/goalcup-cli/src/launch.rs) accepts 20 to 256
  bytes of `[A-Za-z0-9_-]` for every credential name. Under `capture.env` the name stays refused
  ("would carry a credential"). A credential still requires a per-run config home and
  `stderr_line` identity.
- **Why.** Readiness row "`AMP_API_KEY`": only `DEEPSEEK_API_KEY` was admitted.
- **Files.** `capture_policy.rs`; the four harness goldens' `allowed_names`
  (`crates/goalcup-cli/tests/fixtures/harness-golden/*.json`);
  [CONNECTOR-SCHEMA](../CONNECTOR-SCHEMA.md). Tests:
  `amp_credential_is_admitted_only_in_credential_field` and the closed-list test in
  `harness_schema.rs`; [amp_child_secret.rs](../../crates/goalcup-cli/tests/amp_child_secret.rs)
  (child-only delivery, no echo in Debug, plan or refusal text, grammar bounds, and the key scan
  on a token with no `sk-` prefix).
- **Risk.** Amp's real token grammar is UNKNOWN. If a real token holds a byte outside
  `[A-Za-z0-9_-]` (a JWT's `.`, for example) or exceeds 256 bytes, every launch is refused with
  a name-only error, which fails closed. The grammar change would then belong in this PR. The
  plan's key loader script (`scripts/with-amp-key.sh`) is not written: it targets
  `--harness amp-cli`, which does not exist.
- **Status.** Implemented on this branch.

### Proposed DECISIONS row (maintainers own DECISIONS.md)

Proposed text, not in force:

> **D47. (M2) Amp access token admitted as a child credential.**
> **Decision (proposed, 2026-10-11).** `AMP_API_KEY` joins `capture_policy::CREDENTIAL_ENV`
> beside `DEEPSEEK_API_KEY` (amends D43 ruling 3's closed list; the list stays closed). It is
> handed to the harness child only, through the existing child-secret seam, under the shared
> grammar of 20 to 256 bytes of `[A-Za-z0-9_-]`. It is never a manifest value: under
> `capture.env` the name is refused. D43's pairing rules are unchanged: a credential needs a
> per-run config home and the identity pairs D43 admits. D41's key-fragment scan applies to the
> token unchanged. This row admits a name only; it admits no Amp connector, and the owner rules
> separately on Amp's on-disk key fingerprint (`.legacy-state-owner.json`).
> **Reconsider when** the owner's live probe shows a token outside the shared grammar, or Amp
> renames the variable.
> **Evidence.** `docs/qualification/m2-amp-readiness.md` (static: Amp help text names
> `AMP_API_KEY` as an access token that takes precedence over stored credentials).

## 3. Teach the generic kit a credential

- **What.** A kit case may name `credential_env` in `expect.toml`. The runner in
  [harness_contract_kit.rs](../../crates/goalcup-cli/tests/harness_contract_kit.rs) then sets
  that variable in `goalcup start`'s own environment to an invented value (`KIT_CREDENTIAL`), so
  the launcher's existing child-secret path hands it to the stand-in. After the run it checks
  that the stand-in received exactly that value (the stand-in writes a prefixed SHA-256 to its
  marker, outside the run directory, which is not a key-scan form), that the launcher reported
  `key scan: clean`, and that the key scan finds no fragment in the sealed trace or the run
  directory and the forwarded stdout and stderr hold none. A new case,
  `synthetic-credential-stderr`, uses the admitted `DEEPSEEK_API_KEY` with `stderr_line`
  identity, so it is testable before any Amp policy lands.
- **Why.** Readiness row "Synthetic kit for a credential profile": the kit had no credential
  field and supplied no synthetic key, so a keyed connector case could not run.
- **Files.** `harness_contract_kit.rs`; the new case under
  `crates/goalcup-cli/tests/fixtures/harness-kit/synthetic-credential-stderr/`; the kit case
  lists in [HARNESS-ONBOARDING](../HARNESS-ONBOARDING.md), [OPERATING](../OPERATING.md),
  [ARCHITECTURE](../ARCHITECTURE.md) and the
  [harness skill](../../skills/adding-a-capture-harness/SKILL.md).
- **Tests.** The existing `every_kit_case_seals_and_matches_expectations` and
  `every_kit_case_requires_the_dev_door` pick the case up from its directory. The kit is
  macOS-only and was **not run** here. On Linux it was type-checked with
  `cargo clippy -p goalcup-cli --tests --target aarch64-apple-darwin -- -D warnings`, the new
  `harness.toml` was validated as `Origin::Dev` by a throwaway test, and the stand-in's no-key
  branch exits 3 before touching its marker.
- **Risk.** Low; test-only. The first macOS run is the real proof. If the key scan or the
  credential path behaves differently on macOS, the case fails rather than passing silently.
- **Status.** Implemented on this branch; not yet run on macOS.

## 4. Credential with `stdout_json` identity

- **What.** One stdout reader now feeds both the key-fragment scan and the `stdout_json`
  identity matcher, and `check_cross_field` admits a credential with `stdout_json` identity. It
  still requires `config_home = "per_run"`, so the run-directory scan covers the config home.
  - [launch.rs](../../crates/goalcup-cli/src/launch.rs): the identity line matcher moves out
    of `stdout_tee_with_rule` into `StdoutIdentityScan` (same bounds: 64 complete lines of at
    most 1 MiB, one message). `IdentityReader` wraps a stdout pipe and feeds every read to it
    before handing the bytes on. `CaptureStdout` may be scope-only (`scope_only`), when the
    marked tee reports the stream's end.
  - [capture_tee.rs](../../crates/goalcup-cli/src/capture_tee.rs): the private two-stream tee
    takes an optional `StdoutJsonRule`; with one, its stdout tee reads through `IdentityReader`.
    New `tee_marked_capture_with_identity` (and `_to`, for tests) returns the scope receiver and
    the usual `CaptureStderr`, whose `finish` and `key_hit` cover both streams.
  - [start.rs](../../crates/goalcup-cli/src/commands/start.rs) `register_and_release`: a keyed
    child always goes through that one call; an unkeyed child keeps the separate stdout scope
    tee and stderr tee. `StreamPlan` and `LaunchScope` are unchanged.
  - [harness.rs](../../crates/goalcup-protocol/src/harness.rs) `check_cross_field`: the
    `stdout_json` refusal for a credential is removed.
- **Behaviour.** The identity is read when bytes are read, before the key scan decides whether
  to forward them, so a stdout echo of the key is still withheld (`[goalcup: output withheld:
  ...]`), sets the live hit, and fails the row through the existing key verdict. The identity
  still binds the native scope. No v5 record, control API or verifier change.
- **Why.** Readiness row "Credential with `stdout_json` identity": Amp documents a stdout
  `system`/`init` event with `session_id` as its identity carrier. Needed only if the owner's
  live probe confirms that carrier.
- **Tests.** `harness_schema.rs`:
  `a_credential_with_stdout_json_identity_is_admitted_with_a_per_run_home` (replaces the refusal
  test; a shared home is still refused). New
  [credential_stdout_identity.rs](../../crates/goalcup-cli/tests/credential_stdout_identity.rs),
  Linux-runnable: a Python child with the synthetic key prints an Amp-shaped init event; clean
  output is forwarded byte for byte with the identity bound, and a stdout key echo is a hit and
  withheld while the identity still binds. `daemon_client.rs`'s gate sweep names the new call. A
  macOS kit case, `synthetic-credential-stdout-json`, drives the whole `goalcup start` path
  (not run here).
- **Risk.** Medium. It changes the launcher's tee wiring for keyed children, the one area D41
  depends on. Hermes's path (keyed, `stderr_line`) now goes through the new call with no stdout
  rule, which is the old code path. The kit, the end-to-end tests and the Hermes offline-key
  test are macOS-only and were not run here.
- **Status.** Implemented on this branch; macOS runs pending.

### Proposed amendment to D43 ruling 3 (maintainers own DECISIONS.md)

Proposed text, not in force:

> **D43 ruling 3, amended (proposed, 2026-10-11).** A credential requires a per-run config home
> and either `stderr_line` + `deferred` or `stdout_json` + `at_launch` identity; `preassigned`
> and `none` stay refused. For a keyed `stdout_json` profile one stdout reader feeds both the
> key-fragment scan and the identity matcher: the identity is read as bytes arrive, and output
> holding a fragment is withheld and fails the row. Key scanning and identity are no longer
> separate tees on stdout; they share one. Evidence: `credential_stdout_identity.rs`, kit case
> `synthetic-credential-stdout-json`.
