# Working in this repository

This repository holds Goal Cup Proxy, a research recorder for running coding agents across harnesses and operating
systems. It records declared model traffic and native session artifacts into a sealed hash chain,
with offline verification and versioned research decoders. Goal Cup's one-prompt, 168-hour contest
uses that evidence and adds server upload and event rules. Current support is macOS and Linux;
Windows uses WSL2, with real-machine validation pending and native Windows planned.
This file is the map for agents and people. Open work is in [`TODO.md`](TODO.md).

## The seven points

What we build toward, from [VISION.md](VISION.md). Every change is judged against these.

1. **Record every model call** the harness makes, to any provider, for the whole run.
2. **Record every subagent,** however many and however long.
3. **Never block or slow the harness.** If something cannot be recorded, note a gap and keep going.
4. **Never record secrets.**
5. **Upload everything,** so the server can rebuild the full story by ID and time.
6. **Tell the participant plainly** what was recorded.
7. **Work with the harnesses people actually use,** run the way they use them.

Less code is better. Cut before you add. Ready by Friday 6 November 2026.

Read first: [VISION.md](VISION.md), then [DECISIONS](docs/DECISIONS.md) from "Owner decisions in
force". While the 1.0 build runs, also the [ledger](docs/plans/2026-10-30-v1.0/OWNER-DECISIONS.md),
which wins over every PRD, then the [PRD index](docs/plans/2026-10-30-v1.0/PRD-00-index.md).

## Map

| Path | What it is |
|---|---|
| `crates/goalcup-protocol` | Every byte definition: chain, v5 records, control API types, harness schema (`harness.rs`), trust tables (`capture_policy.rs`), connector pins. No rule logic. |
| `crates/goalcup-daemon` | `goalcup-daemon`: ingest, redaction, chain writer, native-file capture, finish, process termination. Small on purpose. |
| `crates/goalcup-cli` | `goalcup` (eleven commands: `start`, `continue`, `status`, `time-left`, `mcp`, `entries`, `seal`, `doctor`, `login`, `setup` (alias `prep`), `upload`). Launches the harness, runs the recording proxy, uploads. |
| `crates/goalcup-verifier` | `goalcup-verify` and the decoders. Every judgement about the bytes lives here. |
| `crates/goalcup-platform` | Unix-only OS code shared by the others: disk reservation, capability-based file access, machine facts, sleep lock. |
| `connectors/<id>/` | One harness = one `harness.toml` plus seed files. Data, not code. Pinned by digest. |
| `server/worker` | The upload server: a Cloudflare Worker in TypeScript. |
| `scripts/` | The gate, pins, connector check, link check, matrix, release and research tools. One line each, and which ones CI and the gate run: [`scripts/README.md`](scripts/README.md). |
| `skills/` | Agent skills: `adding-a-capture-harness` (for contributors), `goal-cup-setup` (for participants), `goal-cup` (copied into the harness home). |
| `docs/` | Docs by audience: [`docs/README.md`](docs/README.md). Finished work and old narratives: `docs/archive/`. |
| `packaging/` | Release packaging and `install.sh`. |
| `spikes/`, `harnesses/` | Old measurements (read-only fixtures) and local harness installs (git-ignored). |

The dependency rule: the daemon never links the verifier, and the CLI never links the daemon.
Both are checked by tests.

## Read next

1. [`README.md`](README.md): build, install, run, the gate.
2. [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md): how the parts fit, and the one glossary (run,
   part, segment, chain file, seal, receipt, ack).
3. [`docs/DECISIONS.md`](docs/DECISIONS.md): current rules and historical decisions. Code comments
   cite them as `D<number>`; the status table at the top says which are current, superseded or
   retired.
4. [`docs/RULES.md`](docs/RULES.md): the participant contract.
5. [`docs/CHEAT-CHECKLIST.md`](docs/CHEAT-CHECKLIST.md): read before you trust a verifier Pass.
   [`docs/TERMINATION.md`](docs/TERMINATION.md): what `Confirmed` does and does not claim.
6. Adding a harness: [`skills/adding-a-capture-harness/SKILL.md`](skills/adding-a-capture-harness/SKILL.md),
   then [`Add and test a connector`](docs/HARNESS-ONBOARDING.md#quick-path).
7. Wire contracts: [`docs/CONTRACTS.md`](docs/CONTRACTS.md), [`docs/CONNECTOR-SCHEMA.md`](docs/CONNECTOR-SCHEMA.md),
   [`docs/UPLOAD-API.md`](docs/UPLOAD-API.md). Running it: [`docs/OPERATING.md`](docs/OPERATING.md).

## The gate

```
sh scripts/gate.sh fast   # before every commit
sh scripts/gate.sh full   # before a release
```

Setup and what each tier runs: [README](README.md#the-gate). CI (`check`) is a small Linux smoke,
not the gate. After the one-time `cargo fetch --locked` and `cargo install cargo-nextest --locked`
setup, project build and gate Cargo commands run with `--offline`.

## Hard rules

- **Never set a model base URL on any harness.** The only wire path is the loopback recording
  proxy, reached through the harness's process-scoped `HTTPS_PROXY` (D1, D81). The test pad
  configures a harness the way a participant would (for example pointing it at DeepSeek or a gateway,
  in goalcup's isolated home); goalcup's own launch code never does.
- **Goal Cup Proxy records and never manages the agent.** No prompting, no auto "keep going", no
  finish tool. The participant types the prompt and may type only "keep going". One exception: D66,
  the agent may ask `goalcup time-left` how much time is left (pull only, nothing is pushed). A full
  disk gets no special handling: the recorder restarts, then runs in null mode, and the harness keeps
  working (D78).
- **No harness names in core code.** A harness is data (a connector) plus shared modules.
- **Never write into `~/.claude`, `~/.codex`, `~/.hermes` or any app-wide config.** Goal Cup Proxy
  makes isolated homes. A test that launches a harness uses a private `HOME`.
- **Never commit** anything under the repository root's `/pad-out/` or `/harnesses/`, under
  `spikes/*/out/`, or any `target/`, a key, or a real run's record. A committed fixture is masked
  and reviewed by eye.
- **Record raw, interpret later.** The recorder captures bytes. The verifier and the usage reader
  judge them. Do not put rule logic in the recorder (D3).

## Tests

One test per behaviour you add or change. Test the promise, not the shape: no golden snapshots of
internal structures, and no assertions on doc text or message wording unless the text is the
contract. Every behaviour that serves one of the seven points has a test, and that test is not
deleted while the point stands, or is replaced in the same change by a test with the same name prefix
that asserts the same promise.

## Done

1. `sh scripts/gate.sh fast` passes. `full` too before a release.
2. At least two independent reviewers read each diff during the 1.0 build (ledger B2); after it, one
   reviewer reads the PR once.
3. `README.md`, the participant guide or `TODO.md` is updated if behaviour changed.

Rust edition 2024, stable toolchain (`rust-toolchain.toml`). Questions about rules or scope go to
the maintainer, Balazs Nemethi, not to a guess.
