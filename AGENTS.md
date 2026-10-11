# Working in this repository

Goal Cup records what a coding agent actually did during a run: a tracker that captures the
agent's own session files and its model traffic into a sealed, hash-linked chain that anyone can
verify. This file is for agents (Claude Code, Codex, others) and people.

**Adding a harness?** Read [CONTRIBUTING.md](CONTRIBUTING.md), then
[`skills/adding-a-capture-harness`](skills/adding-a-capture-harness/SKILL.md) and
[`docs/HARNESS-ONBOARDING.md`](docs/HARNESS-ONBOARDING.md). Treat STATUS and LEDGER as
maintainer-owned: put proposed edits to them in your PR description.
Updated: 2026-10-11.

## Read in this order

1. [`STATUS.md`](STATUS.md): where things are, on one screen.
2. [`LEDGER.md`](LEDGER.md): the open tickets by ID. Change a ticket in the same commit as the work
   (maintainers).
3. [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md): how the parts fit.
4. [`docs/DECISIONS.md`](docs/DECISIONS.md): read the index; treat the entries as constraints.
5. [`README.md`](README.md): build, the four-command gate, run.
6. [`docs/GLOSSARY.md`](docs/GLOSSARY.md): the words. Use them exactly.

## When you need it

- [`docs/RULES.md`](docs/RULES.md): the participant contract.
- [`docs/TERMINATION.md`](docs/TERMINATION.md): what finish and kill claim.
- [`docs/CHEAT-CHECKLIST.md`](docs/CHEAT-CHECKLIST.md): read before trusting a Pass.
- [`docs/HARNESS-ONBOARDING.md`](docs/HARNESS-ONBOARDING.md): adding a harness.
- [`skills/adding-a-capture-harness`](skills/adding-a-capture-harness/SKILL.md): the phase order
  and gates for a new capture harness or decoder.
- [`docs/qualification/README.md`](docs/qualification/README.md): which record proves what.
- [`docs/CONTRACTS.md`](docs/CONTRACTS.md): the M2 interface contracts C1 to C6.
- [`docs/DOCS-STANDARD.md`](docs/DOCS-STANDARD.md): read before editing any doc.

## Rules that bind every contributor and every agent

- **Never set a model base URL on any harness.** The only wire path is a loopback forward proxy
  through process-scoped `HTTPS_PROXY` and a process CA. It records no header, credential or
  hash of one: only selected bodies and allowlisted metadata, redacted before hashing or disk
  (D36). It never changes the harness base URL.
- **Never write into `~/.claude`, `~/.codex`, `~/.hermes` or any app-wide config.** The tracker
  creates isolated homes and reads no app-wide harness configuration. The one link into the real
  home is Claude Code's run `HOME/Library/Keychains` to `~/Library/Keychains`, for its subscription
  login (D38). Participants use subscription auth.
- **A test that launches a harness uses a private HOME.** Check the real HOME before and after.
  A test that touches the real HOME has already broken this rule (LEDGER H-17).
- **Run the four-command gate before any commit**, in this order ([README](README.md#the-gate)
  says why `cargo build` comes first). Never create a `.cargo/config.toml`.
  ```
  cargo build
  cargo fmt --all --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
  ```
  Maintainers add `--offline` after one `cargo fetch --locked`, so that nothing can fetch. It is
  an optional convention: plain commands work, and `--offline` on an empty cache fails.
- **Connector directories and manifests are pinned.** An edit under `connectors/` regenerates
  both `crates/goalcup-protocol/pins/connector-dirs.txt` and
  `crates/goalcup-verifier/pins/connector-manifests.txt` with `sh scripts/pin-manifests.sh` in
  the same commit, or every start refuses.
- **Never commit** anything under `spikes/*/out/`, `harnesses/` or `target/`.

## Ground rules

- **Record raw, interpret later.** The daemon captures bytes and health. Parsing and rule checks
  live in the verifier. No rule logic in the daemon, and the daemon never depends on the verifier
  (D3). Keep the daemon small; it is the signed, hard-to-change piece.
- **Isolation.** Harnesses for testing live under `harnesses/` with their own homes
  (`harnesses/home/<harness>/`). Run `harnesses/install.sh`, then source the `env.sh` it writes.
  Never point a test at the real `~/.claude`, `~/.codex` or similar. If a harness cannot be
  scoped, say so; do not work around it.
- **Secrets.** Spike artifacts land in `spikes/<harness>/out/`, which is git-ignored, because raw
  transcripts hold keys and tokens. A committed fixture goes through `spikes/redact.py` first and
  is reviewed by eye.
- **Versions.** Record the exact harness version in every spike report and connector manifest.
  Formats change between releases.
- **Commits.** Commit docs, spike scripts and reports, redacted fixtures and Rust code. Rust is
  edition 2024 on the stable toolchain; the crates are in ARCHITECTURE.
- **Scope.** `m2/wire` is the M2 wire-plus-native recorder; V1 is the `v1/trunk` foundation.
  Desktop, Windows/Linux, continuous Git provenance, automatic publication, production
  signing and the 168-hour exercise are V1.1 and not started here. Continuous workspace hashing,
  tracker-injected reminders and cross-harness linking are out of scope. Interfaces:
  [`docs/CONTRACTS.md`](docs/CONTRACTS.md).

## Contact

Maintainer: Balazs Nemethi. Questions about rules or scope go to him, not to a guess.
