# Contributing

How do I send a change to Goal Cup, and what will a maintainer ask for? For outside
contributors and their agents. Updated: 2026-10-07.

Harness connectors are data contributions. Everything else (bug fixes, docs and tests) is
welcome too; open an issue first for anything large. Read [AGENTS.md](AGENTS.md) once: its
rules bind people and agents alike.

## Sign off every commit (DCO)

Contributions are accepted under the [Apache License 2.0](LICENSE) and need a
[Developer Certificate of Origin](https://developercertificate.org/) sign-off. Add it with
`git commit -s`, which appends `Signed-off-by: Your Name <you@example.com>`. A pull request
with an unsigned commit is not merged.

## The gate

Every change passes these four commands, in this order ([why the first is not redundant](README.md#the-gate)):

```
cargo build
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Maintainers also run the clippy and test commands with `--features v1-compat`; follow that
gate when a change touches the daemon, CLI or devserver crates, or code they share with V1.
Plain cargo works; `--offline` is an optional maintainer convention. Rust 1.89 or later and
macOS are the supported, measured baseline. Report the exact command exits and test counts.

## Adding a harness

Start with [adding-a-capture-harness](skills/adding-a-capture-harness/SKILL.md),
[HARNESS-ONBOARDING](docs/HARNESS-ONBOARDING.md), and the connector procedure in
[CONNECTOR-CONTRIBUTING](docs/CONNECTOR-CONTRIBUTING.md). The source tree currently has three
connector directories and three pins: `codex-cli`, `claude-code-cli` and `hermes-cli`.
Their `harness.toml` files are harness-contract schema 1; the directory pin is generated data,
not a release or package publication record.

A common data-only harness PR contains:

- `connectors/<id>/harness.toml` and its reviewed `files/**` seed data;
- the one regenerated line for that directory in
  `crates/goalcup-protocol/pins/connector-dirs.txt`, produced by `sh scripts/pin-manifests.sh`;
- a readiness record under `docs/qualification/m2-<name>-readiness.md` and, when a maintainer
  has run a real row, links to that row's qualification evidence (a source edit alone is not
  qualification);
- a new generic harness-kit fixture case under
  `crates/goalcup-cli/tests/fixtures/harness-kit/<case>/` containing `harness.toml`, `stand-in.py`
  and `expect.toml` (plus only reviewed declared seeds) for this connector;
- evidence that the current baseline harness kit passed after a workspace build (the four baseline
  cases plus the new connector case):
  `cargo build`, then `cargo test -p goalcup-cli --test harness_contract_kit`.
- the focused schema and generated-pin checks:
  `cargo test -p goalcup-protocol --test harness_conformance --test connector_pins`.

The current baseline kit cases are `controlled-workload`, `synthetic-preassigned`,
`synthetic-stderr-token` and `synthetic-stdout-json`; the new connector fixture is an additional
case, not a replacement. All use stand-ins, a test-owned proxy and private homes; they do not
prove provider access, package publication, a release, or a qualified live harness. A new
connector must use an admitted schema-1 policy row and identity pair. A new policy row (CA,
credential, identity, native-store form or interpreter) is a separately reviewed core PR with
a DECISIONS row. A semantic decoder is a separately reviewed verifier PR. Do not add `.rs`
files to an ordinary data-only connector PR.

Record exact executable version and SHA-256 only when the source or an official artifact proves
them. Package provenance, a published package, a release, or qualification is UNKNOWN unless its
own evidence proves it; do not infer any of those from a connector directory, a pin, or a passing
synthetic test. Unknown transport, native storage/scope, isolation/auth, or identity
source/binding stops Phase 1; record the blocker instead of guessing.

Connectors are trusted code (a template, environment variable, seed or host entry can run or
leak things), so every connector PR needs code-owner review against the
[reviewer checklist](docs/CONNECTOR-CONTRIBUTING.md#reviewing-a-connector-pr). Never commit
real traces, keys, `target/`, `harnesses/` or `spikes/*/out/`; redact and review any committed
fixture with `spikes/redact.py`.

## STATUS.md, LEDGER.md and DECISIONS.md

These are maintainer-owned state trackers. Do not edit them in an outside PR: they conflict on
every merge. Put proposed text in the PR description and a maintainer lands it.

## Pull requests

Target `main`. Keep a PR to one concern. Fill in the PR template: DCO, redaction, readiness and
the harness checklist. Do not paste a credential, token or real transcript anywhere in a PR,
issue or comment; if you did by accident, say so privately (see [SECURITY.md](SECURITY.md)) and
rotate it.

## Security and conduct

Report vulnerabilities privately, not in an issue: [SECURITY.md](SECURITY.md). Everyone follows
the [Code of Conduct](CODE_OF_CONDUCT.md).
