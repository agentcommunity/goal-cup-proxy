# Contributing

How do I send a change to Goal Cup, and what will a maintainer ask for? For outside
contributors and their agents. Updated: 2026-10-10.

Harness connectors are data contributions. Everything else (bug fixes, docs and tests) is
welcome too; open an issue first for anything large. Read [AGENTS.md](AGENTS.md) once: its
rules bind people and agents alike. How `main` is protected and which checks are required is in
[REPOSITORY-GOVERNANCE](docs/REPOSITORY-GOVERNANCE.md).

## From fork to pull request

The upstream repository is `https://github.com/agentcommunity/goal-cup-tracer.git` and every
pull request targets its `main` branch. Fork it on GitHub under your own account, then:

```
git clone https://github.com/<your-account>/goal-cup-tracer.git
cd goal-cup-tracer
git remote add upstream https://github.com/agentcommunity/goal-cup-tracer.git
git fetch upstream
git switch -c <topic> upstream/main
# edit, then run the gate below
git commit -s -m "<area>: <what changed>"
git push -u origin <topic>
```

Then open a pull request from `<your-account>:<topic>` into `agentcommunity/goal-cup-tracer:main`
on GitHub and fill in the template. Use your own name and email: you certify your own work, and
nobody else can sign off for you. Keep a PR to one concern.

## Sign off every commit (DCO)

Contributions are accepted under the [Apache License 2.0](LICENSE) and need a
[Developer Certificate of Origin](https://developercertificate.org/) sign-off. Add it with
`git commit -s`, which appends `Signed-off-by: Your Name <you@example.com>`. The name and email
must match the commit author. A pull request with an unsigned commit is not merged.

Forgot the sign-off? If it is your latest commit and you have not pushed it yet, run
`git commit --amend --no-edit --signoff`. For a commit you already pushed, or several commits,
follow the instructions on the failing DCO check's details page. Only rewrite and force-push
your own fork branch, never someone else's.

## The gate

Build prerequisites are in [README, Build](README.md#build): Rust 1.89 or later, a C compiler,
`shasum` or `sha256sum`, and `python3` when you regenerate connector pins. macOS is the
supported, measured platform; Linux is best-effort.

Every change passes these four commands, in this order
([why the first is not redundant](README.md#the-gate)):

```
cargo build
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

A change under `crates/goalcup-{daemon,cli,devserver}`, or to code those crates share with V1,
also passes the `v1-compat` gate from the [README](README.md#the-gate), again build first:

```
cargo build --workspace --features v1-compat
cargo clippy --workspace --all-targets --features v1-compat -- -D warnings
cargo test --workspace --features v1-compat
```

Plain cargo works; `--offline` is an optional maintainer convention. Report the exact command
exits, test counts, platform and Rust version in the PR.

## Checks on your pull request

Two checks are expected on a pull request: `check` (the `ci` workflow's one Linux job) and
`DCO` (the GitHub DCO App). Whether each one runs and is required today is recorded in
[REPOSITORY-GOVERNANCE](docs/REPOSITORY-GOVERNANCE.md); until its read-back is recorded, a
maintainer checks sign-off and the gate by hand.

`check` is a Linux smoke, not the gate. It skips the `goalcup-daemon` and `goalcup-cli` test
suites (they spawn real process trees and hang small runners), so a green `check` does not run
the harness kit. Maintainers re-run the gate, and the kit for a connector PR, on macOS.

## Adding a harness

Start with [adding-a-capture-harness](skills/adding-a-capture-harness/SKILL.md),
[HARNESS-ONBOARDING](docs/HARNESS-ONBOARDING.md), and the connector procedure in
[CONNECTOR-CONTRIBUTING](docs/CONNECTOR-CONTRIBUTING.md). The source tree currently has three
connector directories and three pins: `codex-cli`, `claude-code-cli` and `hermes-cli`.
Their `harness.toml` files are harness-contract schema 1; the directory pin is generated data,
not a release or package publication record.

A common data-only harness PR contains:

- `connectors/<id>/harness.toml` and its reviewed `files/**` seed data;
- the pin diff produced by `sh scripts/pin-manifests.sh` (see the note below);
- a readiness record under `docs/qualification/m2-<name>-readiness.md` and, when a maintainer
  has run a real row, links to that row's qualification evidence (a source edit alone is not
  qualification);
- a new generic harness-kit fixture case under
  `crates/goalcup-cli/tests/fixtures/harness-kit/<case>/` containing `harness.toml`, `stand-in.py`
  and `expect.toml` (plus only reviewed declared seeds) for this connector;
- evidence that the current baseline harness kit passed on macOS after a workspace build (the four
  baseline cases plus the new connector case):
  `cargo build`, then `cargo test -p goalcup-cli --test harness_contract_kit`.
- the focused schema and generated-pin checks:
  `cargo test -p goalcup-protocol --test harness_conformance --test connector_pins`.

The harness kit test (`harness_contract_kit`) is macOS-only. On Linux it compiles to zero tests,
so a Linux "pass" proves nothing about your connector; say so if you have no Mac, and a
maintainer runs it.

Pin note: for a new schema-1 connector the script changes exactly one line of
`crates/goalcup-protocol/pins/connector-dirs.txt`. Editing `codex-cli` or `claude-code-cli`,
which still keep a V1 `manifest.toml`, also changes their line in
`crates/goalcup-verifier/pins/connector-manifests.txt` when that `manifest.toml` changes.
Commit what the script writes; never hand-edit or hand-merge either file.

The current baseline kit cases are `controlled-workload`, `synthetic-preassigned`,
`synthetic-stderr-token` and `synthetic-stdout-json`; the new connector fixture is an additional
case, not a replacement. All use stand-ins, a test-owned proxy and private homes; they do not
prove provider access, package publication, a release, or a qualified live harness. A passing
kit is "synthetic tested"; a qualified claim needs a maintainer's live row and its own record.
A new connector must use an admitted schema-1 policy row and identity pair. A new policy row
(CA, credential, identity, native-store form or interpreter) is a separately reviewed core PR
with a DECISIONS row. A semantic decoder is a separately reviewed verifier PR. Do not add `.rs`
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

## Maintainer-owned files

`STATUS.md`, `LEDGER.md` and `DECISIONS.md` are maintainer-owned state trackers. Do not edit
them in an outside PR: they conflict on every merge. Put proposed text in the PR description and
a maintainer lands it.

Paths listed in [`.github/CODEOWNERS`](.github/CODEOWNERS) (connectors, pins, the harness
contract, launch, proxy, redaction, packaging, `.github/` and `SECURITY.md`) may be changed in a
PR, but each such PR needs a code owner's approval.

## Pull requests

Fill in the template: its universal checks apply to every PR, and the harness section only to
connector PRs. Do not paste a credential, token or real transcript anywhere in a PR, issue or
comment; if you did by accident, say so privately (see [SECURITY.md](SECURITY.md)) and rotate it.

## Security and conduct

Report vulnerabilities privately, not in an issue: [SECURITY.md](SECURITY.md). Everyone follows
the [Code of Conduct](CODE_OF_CONDUCT.md).
