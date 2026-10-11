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

| # | Change | Readiness item | Status on `claude/v060-amp-core-prep` |
|---|---|---|---|
| 1 | Refuse `*_URL` env names | core change 1 | implemented |
| 2 | Admit `AMP_API_KEY` | core change 2 | implemented; needs D49 approved |
| 3 | Kit credential support | core change 3 | implemented; macOS kit not run |
| 4 | Credential with `stdout_json` identity | core change 5 | implemented; needs D50 approved |
| - | Private `TMPDIR` per capture launch | core change 4 | not started (optional hardening) |
| - | Settings seed under the run home | core change 6 | not started (only if needed) |
| - | Key fingerprint file, native store | core change 7 | owner decisions, below |

**Merge gate.** Changes 2 and 4 contradict D43 ruling 3 as in force. Their DECISIONS entries are
in [DECISIONS](../DECISIONS.md) itself as proposals, D49 (admit `AMP_API_KEY`) and D50 (amend D43
ruling 3 for credential + `stdout_json` via one stdout reader), each marked "Status: Proposed,
owner approval required before merge." The code commits for changes 2 and 4 must not merge
until the owner approves those entries, and any pull request carrying them carries the entries
too. Change 3's `synthetic-credential-stdout-json` kit case depends on change 4 and waits with
it.

Each numbered change is one commit on this branch, meant to be reviewed as its own pull request,
smallest first. Changes 1, 2 and 4 were built test first: the new tests failed before the
change and passed after it. Change 3 is test code for the macOS-only kit, so it could only be
type-checked here. On this Linux host the macOS-only suites (the harness-contract kit, the
end-to-end and Hermes offline-key tests) were not run.

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
- **The CLI's D23 predicate.** `is_denied` in [launch.rs](../../crates/goalcup-cli/src/launch.rs),
  which `scrubbed_env` and `probe_env` use to strip inherited variables, is widened the same way
  (`*_URL`, was `*_BASE_URL`). `scrubbed_env` serves only the V1 launch, but `probe_env` runs in
  the default build: `goalcup doctor` probes a V1 manifest's harness version through it. Test:
  `the_version_probe_runs_the_harness_with_no_base_url_and_no_proxy_in_its_environment` in
  `preflight_doctor.rs` now plants `AMP_URL`.
- **Status.** Implemented on this branch.

## 2. Admit `AMP_API_KEY` as a credential

- **What.** `CREDENTIAL_ENV` gains `AMP_API_KEY`. The value grammar is unchanged and shared:
  `child_secret_named` in [launch.rs](../../crates/goalcup-cli/src/launch.rs) accepts 20 to 256
  bytes of `[A-Za-z0-9_-]` for every credential name. Under `capture.env` the name stays refused
  ("would carry a credential"). A credential always requires a per-run config home, and an
  identity of `stderr_line` or, with change 4 (D50), `stdout_json` + `at_launch`.
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

### DECISIONS entry

Proposed as [D49](../DECISIONS.md#d49-m2-proposed-admit-amp_api_key-as-a-child-credential), not
in force. This change must not merge until the owner approves it.

## 3. Teach the generic kit a credential

- **What.** A kit case may name `credential_env` in `expect.toml`. The runner in
  [harness_contract_kit.rs](../../crates/goalcup-cli/tests/harness_contract_kit.rs) then sets
  that variable in `goalcup start`'s own environment to an invented value (`KIT_CREDENTIAL`,
  shared through `tests/support/mod.rs`), so the launcher's existing child-secret path hands it
  to the stand-in. After the run it checks that the stand-in received exactly that value, that
  the launcher reported
  `key scan: clean`, and that the key scan finds no fragment in the sealed trace or the run
  directory and the forwarded stdout and stderr hold none. A new case,
  `synthetic-credential-stderr`, uses the admitted `DEEPSEEK_API_KEY` with `stderr_line`
  identity, so it is testable before any Amp policy lands.
- **The marker.** The stand-in proves which value it received by writing the SHA-256 of
  `kit-marker:` + the value to its marker, outside the run directory. That is a salted SHA-256
  of an invented key: it is not a key-scan form (the scan's form is the SHA-256 of the bare
  value), so it proves delivery without planting a fragment. It is safe only because the key is
  invented. Never reuse the scheme with a real key: a salted digest of a real key is a key
  fingerprint, which D41 forbids persisting.
- **No collisions.** The daemon writes the case id into the sealed chain, so the credential's
  key-scan forms must appear in no case id or fixture. The first invented value began with
  `kit-synt`, which every `kit-synthetic-*` id holds, so both keyed cases would have failed their
  own key scan. The value is now a random-looking token with no such overlap, and
  [harness_kit_credential.rs](../../crates/goalcup-cli/tests/harness_kit_credential.rs), which
  runs on Linux too, asserts that no key-scan form of it (first or last eight bytes, masks, the
  window rule, the SHA-256 prefix and their base64 renderings) occurs in any file or directory
  name under `tests/fixtures/harness-kit/` or in `connectors/`, and that the marker digest is
  not one either.
- **Why.** Readiness row "Synthetic kit for a credential profile": the kit had no credential
  field and supplied no synthetic key, so a keyed connector case could not run.
- **Files.** `harness_contract_kit.rs`, `harness_kit_credential.rs`, `tests/support/mod.rs`;
  the new case under
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
    before handing the bytes on. `CaptureStdout` is unchanged: it is always a waitable tee.
  - [capture_tee.rs](../../crates/goalcup-cli/src/capture_tee.rs): the private two-stream tee
    takes an optional `StdoutJsonRule`; with one, its stdout tee reads through `IdentityReader`.
    New `tee_marked_capture_with_identity` (and `_to`, for tests) returns the bare scope
    receiver (`ScopeReceiver`, not a `CaptureStdout`, so it cannot be mistaken for a tee to wait
    on) and the usual `CaptureStderr`, whose `finish` and `key_hit` cover both streams. It builds
    the identity matcher's own key matcher only when there is a stdout rule.
  - [start.rs](../../crates/goalcup-cli/src/commands/start.rs) `register_and_release`: a keyed
    child always goes through that one call; an unkeyed child keeps the separate stdout scope
    tee and stderr tee. `StreamPlan` and `LaunchScope` are unchanged.
  - [harness.rs](../../crates/goalcup-protocol/src/harness.rs) `check_cross_field`: the
    `stdout_json` refusal for a credential is removed.
- **Behaviour.** The identity is read when bytes are read, before the key scan decides whether
  to forward them, so a stdout echo of the key is still withheld (`[goalcup: output withheld:
  ...]`), sets the live hit, and fails the row through the existing key verdict. The identity
  still binds the native scope. The bound id is written into the chain as the scope, so for a
  keyed launch the identity matcher holds its own key-fragment matcher: when the matching
  event's line or decoded id holds any key-scan form, it binds nothing (the unbound path an
  unusable id takes) and records a key hit. For a keyed launch its line buffer is overwritten
  before each clear and on drop, as the tee's own buffer is; an unkeyed scan just clears it.
  No v5 record, control API or verifier change.
- **Why.** Readiness row "Credential with `stdout_json` identity": Amp documents a stdout
  `system`/`init` event with `session_id` as its identity carrier. Needed only if the owner's
  live probe confirms that carrier.
- **Tests.** `harness_schema.rs`:
  `a_credential_with_stdout_json_identity_is_admitted_with_a_per_run_home` (replaces the refusal
  test; a shared home is still refused). New
  [credential_stdout_identity.rs](../../crates/goalcup-cli/tests/credential_stdout_identity.rs),
  Linux-runnable: a Python child with the synthetic key prints an Amp-shaped init event; clean
  output is forwarded byte for byte with the identity bound, and a stdout key echo is a hit and
  withheld while the identity still binds. An init event whose `session_id` is the key, holds
  its last eight bytes, or spells the key in JSON `\u` escapes binds nothing, is a key hit, and
  forwards no fragment. `daemon_client.rs`'s gate sweep names the new call. A
  macOS kit case, `synthetic-credential-stdout-json`, drives the whole `goalcup start` path
  (not run here).
- **Risk.** Medium. It changes the launcher's tee wiring for keyed children, the one area D41
  depends on. Hermes's path (keyed, `stderr_line`) now goes through the new call with no stdout
  rule, which is the old code path. The kit, the end-to-end tests and the Hermes offline-key
  test are macOS-only and were not run here.
- **Status.** Implemented on this branch; macOS runs pending.

### DECISIONS entry

Proposed as
[D50](../DECISIONS.md#d50-m2-proposed-amend-d43-ruling-3-for-stdout_json-identity), an
amendment to D43 ruling 3, not in force. This change must not merge until the owner approves it.

## Owner decisions that remain

These are not code changes this branch can make. Each needs the owner's ruling, and most need
the live probe first.

- **Key fingerprint file.** Amp writes `$HOME/.local/share/amp/.legacy-state-owner.json` with
  an `identityKey` that is a deterministic function of the access token (readiness row
  "Key-derived file"). D41 and the plan forbid persisting a key fingerprint. The key-fragment
  scan cannot see it: it matches fragments and the SHA-256 prefix, and this derivation is
  UNKNOWN. Options:
  - (a) find an Amp setting or seed that stops the write, proven byte for byte as D41 did for
    Hermes;
  - (b) amend D41 to confine the file to the uncommitted per-run directory and exclude it from
    any retained artifact, with a native pattern that cannot match it;
  - (c) treat Amp as blocked until Amp stops writing it.
- **No local own-session store.** If the live probe finds that Amp keeps its sessions only on
  its server, schema 1 has no fit, because it requires exactly one `[[capture.native]]` row. The
  readiness record lists the options: (a) stop, Amp stays blocked; (b) a core PR and DECISIONS
  row admitting a connector with no native row, at a wire-only or capture-only claim, whose wire
  leg is Amp's own protocol to `ampcode.com` with no decoder; (c) a new server-relay harness
  design. A wrapper that writes a native file from stdout or an export stays forbidden. Change 4
  makes a stdout identity possible; it does not make stdout native capture.
- **Managed settings.** Amp reads machine-wide `/etc/ampcode/managed-settings.json` (macOS:
  under `Application Support/ampcode`, full path UNKNOWN) and fetches workspace settings and
  plugins from its server. Goal Cup cannot isolate either. The owner rules whether a row must
  record their presence (metadata only), refuse to start when the machine-wide file exists, or
  disclose them as residuals.
- **Token grammar.** Change 2 assumes Amp tokens fit 20 to 256 bytes of `[A-Za-z0-9_-]`. The
  live probe must confirm it before D49 is approved.
- **Identity carrier.** Change 4 matters only if the live probe shows the stdout `init` event
  arrives before the first model request and names the session a native file is scoped by.
