# Working in this repository

Goal Cup is a one-prompt, 168-hour coding-agent contest. This repository holds the tracer. It
launches a participant's coding agent (the harness), records what the agent does into a sealed
hash chain, uploads the chain to the Goal Cup server, and ships a verifier that anyone can run
offline. This file is the map for agents and people. Open work is in [`TODO.md`](TODO.md).

## Map

| Path | What it is |
|---|---|
| `crates/goalcup-protocol` | Every byte definition: chain, v5 records, control API types, harness schema (`harness.rs`), trust tables (`capture_policy.rs`), connector pins. No rule logic. |
| `crates/goalcup-daemon` | `goalcup-daemon`: ingest, redaction, chain writer, native-file capture, finish, process termination. Small on purpose. |
| `crates/goalcup-cli` | `goalcup` (nine commands: `start`, `continue`, `status`, `entries`, `seal`, `doctor`, `login`, `prep`, `upload`). Launches the harness, runs the recording proxy, uploads. |
| `crates/goalcup-verifier` | `goalcup-verify` and the decoders. Every judgement about the bytes lives here. |
| `crates/goalcup-platform` | Unix-only OS code shared by the others: disk reservation, capability-based file access, machine facts, sleep lock. |
| `connectors/<id>/` | One harness = one `harness.toml` plus seed files. Data, not code. Pinned by digest. |
| `server/worker` | The upload server: a Cloudflare Worker in TypeScript. |
| `scripts/` | `gate.sh`, `pin-manifests.sh`, `check-connector.sh`, `check-links.py`, `matrix.sh`, release scripts. |
| `skills/` | Agent skills: `adding-a-capture-harness` (for contributors), `goal-cup-setup` (for participants), `goal-cup` (copied into the harness home). |
| `docs/`, `packaging/` | Docs; release packaging and `install.sh`. |
| `spikes/`, `harnesses/` | Old measurements (read-only fixtures) and local harness installs (git-ignored). |

The dependency rule: the daemon never links the verifier, and the CLI never links the daemon.
Both are checked by tests.

## Read next

1. [`README.md`](README.md): build, install, run, the gate.
2. [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md): how the parts fit, and the terms.
3. [`docs/DECISIONS.md`](docs/DECISIONS.md): rules that still bind the code. Code comments cite
   them as `D<number>`. Treat them as constraints.
4. [`docs/RULES.md`](docs/RULES.md): the participant contract.
5. [`docs/CHEAT-CHECKLIST.md`](docs/CHEAT-CHECKLIST.md): read before you trust a verifier Pass.
   [`docs/TERMINATION.md`](docs/TERMINATION.md): what `Confirmed` does and does not claim.
6. Adding a harness: [`skills/adding-a-capture-harness/SKILL.md`](skills/adding-a-capture-harness/SKILL.md),
   then [`docs/HARNESS-ONBOARDING.md`](docs/HARNESS-ONBOARDING.md).
7. Wire contracts: [`docs/CONTRACTS.md`](docs/CONTRACTS.md), [`docs/CONNECTOR-SCHEMA.md`](docs/CONNECTOR-SCHEMA.md),
   [`docs/UPLOAD-API.md`](docs/UPLOAD-API.md). Running it: [`docs/OPERATING.md`](docs/OPERATING.md).

## The gate

```
sh scripts/gate.sh fast   # before every commit
sh scripts/gate.sh full   # before a release
```

Setup and what each tier runs: [README](README.md#the-gate). CI (`check`) is a small Linux smoke,
not the gate. Every cargo command is `--offline`; run `cargo fetch --locked` once first.

## Hard rules

- **Never set a model base URL on any harness.** The only wire path is the loopback recording
  proxy, reached through the harness's process-scoped `HTTPS_PROXY` (D1, D36).
- **The tracer records and never manages the agent.** No prompting, no auto "keep going", no
  finish tool. The participant types the prompt and may type only "keep going".
- **No harness names in core code.** A harness is data (a connector) plus shared modules.
- **Never write into `~/.claude`, `~/.codex`, `~/.hermes` or any app-wide config.** The tracer
  makes isolated homes. A test that launches a harness uses a private `HOME`.
- **Connector edits regenerate the pins** with `sh scripts/pin-manifests.sh` in the same commit,
  or every start refuses.
- **Never commit** anything under `spikes/*/out/`, `harnesses/` or `target/`. A committed
  fixture goes through `spikes/redact.py` and is reviewed by eye.
- **Record raw, interpret later.** The daemon captures bytes. The verifier judges them. Do not
  put rule logic in the daemon (D3).

## Tests

One test per behaviour you add or change. Test the promise, not the shape: no golden snapshots of
internal structures, and no assertions on doc text or message wording unless the text is the
contract. Never delete a test in the promises list ([P1 to P7](README.md#the-promises)).

## Done

1. `sh scripts/gate.sh fast` passes. `full` too before a release.
2. One reviewer reads the PR once.
3. `README.md` or `TODO.md` is updated if behaviour changed.

Rust edition 2024, stable toolchain (`rust-toolchain.toml`). Questions about rules or scope go to
the maintainer, Balazs Nemethi, not to a guess.
