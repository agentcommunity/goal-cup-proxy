# Working in this repository

Goal Cup records what a coding agent actually did during a run. A tracker captures the agent's own
session files and its model traffic into a sealed, hash-linked chain that anyone can verify
offline. For agents and people. What is next: [`TODO.md`](TODO.md).

## Read

- [`README.md`](README.md): build, the gate, run.
- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md): how the parts fit.
- [`docs/DECISIONS.md`](docs/DECISIONS.md): frozen; treat the entries as constraints.
- [`docs/RULES.md`](docs/RULES.md): the participant contract.
- [`docs/CHEAT-CHECKLIST.md`](docs/CHEAT-CHECKLIST.md) (read before trusting a Pass) and
  [`docs/TERMINATION.md`](docs/TERMINATION.md) (what finish and kill claim).
- [`docs/GLOSSARY.md`](docs/GLOSSARY.md): the words; use them exactly.
- Adding a harness: [`skills/adding-a-capture-harness/SKILL.md`](skills/adding-a-capture-harness/SKILL.md),
  [`docs/HARNESS-ONBOARDING.md`](docs/HARNESS-ONBOARDING.md).

Old plans, the ledger and the qualification records are in git history at `6494030`.

## The gate

Run before any commit, in this order:

```
cargo build
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
python3 -m unittest discover -s scripts
python3 scripts/check-links.py --root .
```

`cargo build` comes first on purpose ([README](README.md#the-gate) says why). Maintainers may add
`--offline` after one `cargo fetch --locked`. CI (`check`) is a small Linux smoke, not the gate.

## Hard rules

- **Never set a model base URL on any harness.** The only wire path is a loopback forward proxy
  through process-scoped `HTTPS_PROXY`; it never changes the harness base URL (D36).
- **Never write into `~/.claude`, `~/.codex`, `~/.hermes` or any app-wide config.** The tracker
  creates isolated homes. A test that launches a harness uses a private HOME.
- **Connector manifests are pinned.** An edit under `connectors/` regenerates the pins with
  `sh scripts/pin-manifests.sh` in the same commit, or every start refuses.
- **Never commit** anything under `spikes/*/out/`, `harnesses/` or `target/`. A committed fixture
  goes through `spikes/redact.py` and is reviewed by eye.

## Ground rules

- Record raw, interpret later: the daemon captures bytes, the verifier judges them, and the daemon
  never depends on the verifier (D3). Keep the daemon small.
- Rust edition 2024, stable toolchain. Questions about rules or scope go to the maintainer, Balazs
  Nemethi, not to a guess.

## Definition of done

1. The gate above passes.
2. One reviewer reads the PR once.
3. `README.md` or `TODO.md` is updated if behaviour changed.
