<!-- What does this change, and why? One concern per PR. -->

<!-- Required checks and review rules: docs/REPOSITORY-GOVERNANCE.md.
     The fork-to-PR steps and the gate: CONTRIBUTING.md. -->

## Every PR

- [ ] I, a human, read the whole diff and answer for it. Any AI assistance is disclosed here or
      in `Co-Authored-By:` trailers.
- [ ] I ran the gate: `cargo build`, `cargo fmt --all --check`,
      `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
      Platform and Rust version used:
- [ ] If I touched `crates/goalcup-{daemon,cli,devserver}` or code they share with V1, I also ran
      `cargo build --workspace --features v1-compat`, then the `v1-compat` clippy and test
      commands from CONTRIBUTING.md (or: not applicable).
- [ ] No credential, token, real transcript or trace is in this PR, its commits, or its description.
- [ ] Any committed fixture went through `spikes/redact.py` and I reviewed it by eye. Nothing under
      `spikes/*/out/`, `harnesses/` or `target/` is committed.
- [ ] The change sets no model base URL and writes no app-wide config (`~/.claude`, `~/.codex`,
      `~/.hermes`); tests that launch a harness use a private HOME.
- [ ] Outside contributors: I did not edit `STATUS.md`, `LEDGER.md` or `docs/DECISIONS.md`; any
      proposed text for them is in this description.

## Harness PR only (skip for other PRs)

Follow `skills/adding-a-capture-harness` and `docs/HARNESS-ONBOARDING.md`. The schema reference is
`docs/CONNECTOR-SCHEMA.md`; the surrounding M2 contract is
`docs/CONTRACTS.md#harness-contract-v1`. A code owner reviews against the numbered list (items 1
to 10) in `docs/CONNECTOR-CONTRIBUTING.md#reviewing-a-connector-pr`; each box below is the
author's half of that list, so say where the reviewer can check it.

- [ ] Capability worksheet attached or linked (UNKNOWN where unknown), and
      `docs/qualification/m2-<name>-readiness.md` with the exact harness version, install inventory
      hashes and the harness's argv pre-scan source (or "none") (item 2).
- [ ] I ran `sh scripts/pin-manifests.sh` in the same commit and the conformance tests as in
      `docs/CONNECTOR-CONTRIBUTING.md#add-or-change-a-connector`. The pin diff is one line of
      `crates/goalcup-protocol/pins/connector-dirs.txt`, plus the connector's line in
      `crates/goalcup-verifier/pins/connector-manifests.txt` only if I changed a V1
      `manifest.toml` in `codex-cli` or `claude-code-cli` (item 10).
- [ ] If two harness PRs conflict on the pins, I re-ran the pin script on the merged tree instead
      of hand-merging the generated data.
- [ ] Exact version and executable SHA-256, and the official artifact they came from; `program` is
      the harness's own entry point (item 4).
- [ ] Each file under `files/`, `args`, `env` value, seed, host, native pattern, identity source and
      CA or credential variable is justified in this description, with no model base URL or
      redirect (items 1 to 9).
- [ ] Kit case under `crates/goalcup-cli/tests/fixtures/harness-kit/<case>/` and the output of
      `cargo test -p goalcup-cli --test harness_contract_kit` after `cargo build`, run on macOS
      (on Linux it compiles to zero tests; say so if you could not run it). Fixtures went through
      `spikes/redact.py` (items 7 and 10).
- [ ] The diff touches only `connectors/<id>/`, the pin lines above, `docs/qualification/` and the
      kit case `<case>`; no `.rs` file is changed and no policy row or decoder is added (item 10).
- [ ] Claim sought: see `docs/HARNESS-ONBOARDING.md` section 6 ("Diagnose and claim only what
      passed"). Without a maintainer live row it is "synthetic tested".
