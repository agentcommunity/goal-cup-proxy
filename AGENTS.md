# Working in this repository

This repository holds Goal Cup Proxy (`goalcup`), the recorder for Goal Cup runs. One process on the
participant's machine records every model request a coding agent's harness makes, through a loopback
proxy, plus the harness's own session files, and writes every loss as a gap with a start and an end.
It uploads the record while the run goes on. `goalcup-verify` checks a record and reads usage from
it later. Linux and macOS (arm64, tested on the owner's Mac); Windows through WSL2, untested. Open work is in
[TODO.md](TODO.md).

## The seven points

The seven points are in [VISION.md](VISION.md#the-seven-points). Every change is judged against them.

Less code is better. Cut before you add. Ready one week before the start window opens.

Read first: [VISION.md](VISION.md), then [ARCHITECTURE](docs/ARCHITECTURE.md), then
[DECISIONS](docs/DECISIONS.md) from "Owner decisions in force". While the 1.0 build runs, also the
[ledger](docs/plans/2026-10-30-v1.0/OWNER-DECISIONS.md), which wins over every PRD, then the
[PRD index](docs/plans/2026-10-30-v1.0/PRD-00-index.md).

## Map

| Path | What it is |
|---|---|
| `crates/goalcup-format` | The record format (GCR1): records, links, part writer, reader, dump. No async, no TLS. Spec: [RECORD-FORMAT](docs/RECORD-FORMAT.md). |
| `crates/goalcup` | `goalcup`: the eight commands, the watchdog, the recorder process (proxy, session-file reader, masking, writer, uploader) and the connector loader. |
| `crates/goalcup-verify` | `goalcup-verify`: `check`, `dump`, `secrets`, `usage`. Every judgement about the bytes lives here. |
| `connectors/` | One `<id>.toml` per harness, `model-hosts.toml` (the one host list) and `tiers.toml` (written by the test pad). Data: the connector files are compiled into both binaries, the two list files into `goalcup`. |
| `server/worker` | The upload server, a Cloudflare Worker. It stores bytes and interprets nothing. |
| `scripts/` | The gate, release, `fetch-run.sh` and the test pad (`scripts/pad/`): [scripts/README.md](scripts/README.md). |
| `skills/` | `goal-cup` (copied into the harness home) and `adding-a-capture-harness` (contributors). |
| `docs/` | By reader: the [docs map](README.md#docs-map). Plans: `docs/plans/`. History: `docs/archive/`, `docs/research/`. |
| `packaging/` | Release packaging and `install.sh`. |

The dependency rule: the recorder never links the verifier, and the verifier never links the
recorder. They share only `goalcup-format`. No test guards this yet; the manifests are the only
check.

Core code is `crates/goalcup` and `crates/goalcup-format`. The usage reader in
`crates/goalcup-verify` reads harness session formats after the run, chosen by connector id.

## The gate

One script, two tiers: `sh scripts/gate.sh fast` before every merge (format, clippy, every test, the
Worker and script tests) and `sh scripts/gate.sh full` at phase exits and before a release (also the
ignored end-to-end tests and the release build). CI runs `fast` on every pull request to main and
every push to main or `v1.0/integration`. Run it as a normal user, not root. What each tier runs and
what it needs: [README](README.md#the-gate).

## Hard rules

- **Never set a model base URL on any harness.** The only wire path is the loopback recording proxy,
  reached through the harness's process-scoped `HTTPS_PROXY` (D1, D81). The test pad configures a
  harness the way a participant would (for example pointing it at DeepSeek or a gateway, in
  goalcup's isolated home); goalcup's own launch code never does. One exception (owner, 4 Nov
  2026): a connector's `ca_settings` trust entry names the real provider address, never the proxy.
- **Goal Cup Proxy records and never manages the agent.** No prompting, no auto "keep going", no
  finish tool. The participant types the prompt and may type only "keep going". One exception: D66,
  the agent may ask `goalcup time-left` how much time is left (pull only, nothing is pushed). A full
  disk gets no special handling: the recorder restarts, then runs in null mode, and the harness
  keeps working (D78).
- **No harness names in core code.** A harness is data (a connector) plus shared modules.
- **Never write into `~/.claude`, `~/.codex`, `~/.hermes` or any app-wide config.** Goal Cup Proxy
  makes isolated homes. A test that launches a harness uses a private `HOME`.
- **Never commit** anything under the repository root's `/pad-out/` or `/harnesses/`, any
  `target/`, a key, or a real run's record.
  A committed fixture is masked and reviewed by eye.
- **Record raw, interpret later.** The recorder captures bytes. The verifier and the usage reader
  judge them. Do not put rule logic in the recorder (D3).

## Tests

One test per behaviour you add or change. Test the promise, not the shape: no golden snapshots of
internal structures, and no assertions on doc text or message wording unless the text is the
contract. Every behaviour that serves one of the seven points has a test, and that test is not
deleted while the point stands unless it is replaced in the same change by a test with the same name
prefix that asserts the same promise. Where they are:
[README](README.md#the-seven-points-and-their-tests).

## Done

1. `sh scripts/gate.sh fast` passes. `full` too before a release.
2. At least two independent reviewers read each diff during the 1.0 build (ledger B2); after it, one
   reviewer reads the PR once.
3. `README.md`, the participant guide or `TODO.md` is updated if behaviour changed.

Rust edition 2024, stable toolchain (`rust-toolchain.toml`). Questions about rules or scope go to
the maintainer, Balazs Nemethi, not to a guess.
