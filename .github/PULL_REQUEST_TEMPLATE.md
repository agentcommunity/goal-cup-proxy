<!-- What does this change, and why? One concern per PR. -->

## Checklist

- [ ] Every commit is signed off (`git commit -s`, DCO). See CONTRIBUTING.md.
- [ ] I ran the gate: `cargo build`, `cargo fmt --all --check`,
      `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
      Platform and Rust version used:
- [ ] No credential, token, real transcript or trace is in this PR, its commits, or its description.
- [ ] Any committed fixture went through `spikes/redact.py` and I reviewed it by eye. I recorded
      the harness version. Nothing under `spikes/*/out/`, `harnesses/` or `target/` is committed.
- [ ] If I touched `connectors/`, I ran `sh scripts/pin-manifests.sh` in the same commit.
- [ ] The change sets no model base URL and writes no app-wide config (`~/.claude`, `~/.codex`,
      `~/.hermes`); tests that launch a harness use a private HOME.

## Harness PR only

Follow `skills/adding-a-capture-harness` and `docs/HARNESS-ONBOARDING.md`. The schema reference is
`docs/CONNECTOR-SCHEMA.md`; the surrounding M2 contract is
`docs/CONTRACTS.md#harness-contract-v1`. A code owner reviews against the numbered list in
`docs/CONNECTOR-CONTRIBUTING.md#reviewing-a-connector-pr`; each box below is the author's half of
that list, so say where the reviewer can check it.

- [ ] Capability worksheet attached or linked (UNKNOWN where unknown), and
      `docs/qualification/m2-<name>-readiness.md` with the exact harness version, install inventory
      hashes and the harness's argv pre-scan source (or "none") (items 2, 16).
- [ ] The pin script and conformance tests were run as in
      `docs/CONNECTOR-CONTRIBUTING.md#add-or-change-a-connector`. The pin diff is one line of
      `crates/goalcup-protocol/pins/connector-dirs.txt` (item 12).
- [ ] If two harness PRs conflict on the pins, I re-ran the pin script on the merged tree instead
      of hand-merging the generated data.
- [ ] Exact version and executable SHA-256, and the official artifact they came from; `program` is
      the harness's own entry point (item 4).
- [ ] Each file under `files/`, `args`, `env` value, seed, host, native pattern and CA or credential
      variable is justified in this description, with no model base URL or redirect (items 1 to 11,
      13).
- [ ] Kit case under `crates/goalcup-cli/tests/fixtures/harness-kit/<id>/` and its output. Fixtures
      went through `spikes/redact.py` (item 15).
- [ ] The diff touches only `connectors/<id>/`, one line of
      `crates/goalcup-protocol/pins/connector-dirs.txt`, `docs/qualification/` and the kit case;
      no `.rs` file is changed.
- [ ] No hidden policy change (item 17).
- [ ] Claim sought: see `docs/HARNESS-ONBOARDING.md` section 8. Without a maintainer live row it
      is "synthetic tested".
- [ ] Proposed STATUS or LEDGER text is in this description (do not edit those files).
