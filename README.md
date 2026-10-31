# Goal Cup Proxy

Goal Cup Proxy (`goalcup`) records what a coding agent does during a run. It starts your agent's
harness in your terminal and writes down every model request the harness makes, through a proxy on
your own computer, plus the harness's own session files. Every loss is written as a gap with a
start and an end, never skipped. It uploads the record to the Goal Cup server while the run goes on.
Everything runs on your machine, and anyone can check a record later with `goalcup-verify`.

Its first use is the /goal Cup: one prompt, one agent, 168 hours. The organizer is
[Agent Community](https://agentcommunity.org). Sign-up, the rules summary, the demo video and the
questionnaire are on the event site, [agentcommunity.org/goalcup](https://agentcommunity.org/goalcup).

Agents working in this repository: read [AGENTS.md](AGENTS.md) first.

## Quick start for participants

The full walk-through, with every message you will see, is the
[participant's guide](docs/PARTICIPANT-GUIDE.md). The contract is [RULES](docs/RULES.md).
The harnesses goalcup records are listed in [docs/harnesses/README.md](docs/harnesses/README.md).

1. **Install** the latest release (macOS or Linux; on Windows, inside WSL2):

   ```sh
   curl -fsSL https://github.com/agentcommunity/goal-cup-proxy/releases/latest/download/install.sh | sh
   goalcup --version
   ```

   It checks the download against the release's `SHA256SUMS` and installs into `~/.local/bin`
   and `~/.local/share/goalcup`, with no sudo. Run it again to upgrade, only with no run open.
   For one particular release, add `-s -- --version <x.y.z>` after `sh`.
2. **Set up**: `goalcup setup`. It asks which harness you use, opens it once in its own isolated
   home so you can sign in and set it never to stop for approval, and checks this machine. Run it
   until it says `Ready.` ([the guide, step 3](docs/PARTICIPANT-GUIDE.md#3-set-up)).
3. **Practice** (optional): in an empty folder, `goalcup start --practice`. About ten minutes, no
   GitHub, never counts. Do as many as you like
   ([the guide, step 5](docs/PARTICIPANT-GUIDE.md#5-a-practice-run)).
4. **Make the repository, the token and the folder**, in this order. Create a new, empty, public
   GitHub repository (no README, `.gitignore` or licence). Make a fine-grained token for that one
   repository: Contents read and write, expiry 30 days. Make a new folder holding `.env.local`
   (exactly one line, `GH_TOKEN=<token>`) and a `.gitignore` with the line `.env*`. Write your
   prompt in a file outside that folder, for example `prompt.txt`. Details:
   [the guide, step 6](docs/PARTICIPANT-GUIDE.md#6-your-run).
5. **Start** inside the start window, in that folder:
   `goalcup start --repo <you>/<repo> --prompt prompt.txt`. goalcup checks the folder and the
   repository before it asks for the browser approval. Your first message is the push lines and
   your prompt in one block; paste it as the first thing you send. After it, the only message you
   may ever type is "keep going". Delete the token after your run.

**During the run.**

- The server is the clock. The run ends 168 hours after the server's official start.
- Check on it from a second terminal with `goalcup status`. Its `Next:` line says what to do.
- Your agent may run `goalcup time-left` to ask how much time is left. goalcup never sends anything
  to the agent: no reminders, no "keep going", no restarts.
- If the agent stops early (a crash, a reboot, a closed window), run `goalcup continue`.
- No server error stops a run. Recording goes on, and uploads are retried with longer waits.
- If the disk fills, goalcup cannot write. Your agent keeps working, `goalcup status` says recording
  is off, and goalcup retries by itself. Free space and recording starts again. The time it was off
  is a gap in the record.
- Keep the machine on mains power, awake and online, and keep the terminal open.

**The end.** At the deadline goalcup stops the agent, finishes the record and uploads the rest. The
end screen says what was written and what was lost. What is judged is the last push to your
repository's default branch that GitHub received before the deadline. Your agent must push its own
work; goalcup never pushes. Then hand in your demo video and questionnaire on the event site
within 24 hours ([the end](docs/PARTICIPANT-GUIDE.md#7-the-end)).

## Commands

A **run** is your one 168-hour attempt. A **part** is one `start` or one `continue` inside it. Every
flag is in `goalcup <command> --help`. Exit codes: 0 done; 1 refused, and you can fix it; 3 the
command could not do its job.

| Command | What it does |
|---|---|
| `goalcup setup [--harness <ID>] [--yes] [--record-host <HOST>]` | Pick your harness, sign in to it once, check this machine. `--yes` asks nothing and opens nothing. `--record-host` adds a model host goalcup records. |
| `goalcup start [--harness <ID>] [--practice \| --test] [--repo <OWNER/NAME>] [--prompt <FILE>] [--connector <FILE>] [--hosts <FILE>]` | Start a run: open your harness in the current folder and record it. A real run needs `--repo`. `--test` is for organizers. `--connector` and `--hosts` load a file and are for practice and test runs. |
| `goalcup continue [<RUN>] [--connector <FILE>] [--hosts <FILE>]` | Start your agent again in the same run, after it stopped early. |
| `goalcup recover [<RUN>]` | Restart recording when `status` says recording stopped and your agent still runs. |
| `goalcup status [<PART>] [--json]` | Show how the run is doing, its parts and a `Next:` line. |
| `goalcup seal [<PART>] [--yes]` | End a part now: stop recording, finish its record files and upload them. |
| `goalcup upload [<PART>]` | Send what is not uploaded yet. Safe to repeat. |
| `goalcup time-left` | Show how much time the run has left. The agent may run it too. |

`goalcup-verify` checks a record after the run. Every judgement about the bytes lives here.

| Command | What it does |
|---|---|
| `goalcup-verify check <PATH> [--json] [--live] [--server] [--local <ENTRY_DIR>] [--known <FILE>]` | Prove an entry, a run or a fetched folder whole, and name every loss. `--json` prints the report ([RECORD-FORMAT](docs/RECORD-FORMAT.md)). |
| `goalcup-verify dump <PATH> [--sid <ID>] [--kind <KIND>] [--text]` | One JSON line per record. Payload only with `--text`. |
| `goalcup-verify secrets [--known <FILE>] <PATHS>...` | Scan records and raw files for known secret values and secret shapes. |
| `goalcup-verify usage <PATH> [--json] [--out <OUT>]` | Read tokens, models and subagents from a record. |

Exit codes of `goalcup-verify`: 0 whole; 1 honest but incomplete (every loss named, or not sealed);
2 a failure; 3 no usable report (the input cannot be read or the command line is wrong). Exit 0
says the record is consistent, not that the run was good.

## How it works

```
participant types the prompt
        |
        v
harness --HTTPS_PROXY--> proxy (loopback, in the recorder) --> listed model hosts
   |                          |  other hosts: tunnel, one conn record
   | session files            v
   +---------------> held-bytes guard --> masker --> writer --> rec-*.gcr.zst part files
                                                                     |
                                                                     v
                                       uploader: chunks, heartbeat  --> Goal Cup server (Worker)

goalcup-verify reads the part files, or the server's copy, after the run.
```

- **One process.** `goalcup start` is the watchdog: it holds the proxy's port, starts the harness
  and restarts the recorder at once if it dies. The recorder process does all the recording. No
  recorded byte crosses a process boundary.
- **Relay first, record second.** The proxy forwards bytes, then offers a copy. If a memory guard is
  full, the copy is dropped and the loss is written as a gap. The harness is never slowed.
- **Headers are recorded with credentials masked.** Secrets are masked before any byte reaches disk.
- **One host list.** `connectors/model-hosts.toml` names the model hosts goalcup opens. Every other
  connection passes through closed, noted only as host, port, bytes and times.

A gateway is one more HTTPS address. If your harness uses Vercel AI Gateway, OpenRouter or
Cloudflare AI Gateway, goalcup records it exactly like a direct key: same proxy, same record. A
model host goalcup does not know is added once with `goalcup setup --record-host <host>`. Some
harnesses talk only to their vendor (Amp, Copilot with GitHub's models): goalcup records what
the harness sends to the vendor; the vendor's own model calls happen on its servers. A harness
that ignores goalcup's proxy is "not supported": goalcup records its session files only, not its
model calls, and says so.

The record format is [RECORD-FORMAT](docs/RECORD-FORMAT.md). The parts, the terms and the data
flow are [ARCHITECTURE](docs/ARCHITECTURE.md).

## Platforms

| Platform | Status |
|---|---|
| macOS | The owner's Mac run is pending. Until it passes, macOS is not confirmed. |
| Linux x86_64 | Tested in sandboxes. |
| Linux arm64 | Built, not yet tested. |
| Windows | Use WSL2 (below). |

Known issues: [TODO](TODO.md#known-issues). On a machine with no browser, set `GOALCUP_NO_BROWSER=1`:
the approval address is printed instead.

<a id="windows-use-wsl2"></a>**Windows: use WSL2.** In PowerShell run `wsl --install -d Ubuntu`,
then do everything inside Ubuntu as on Linux. Keep every folder a run uses on the Linux filesystem
(`~/...`), never on `/mnt/c`. goalcup cannot keep Windows awake, so set Windows sleep to Never for
the run.

## Add a harness

A harness is data, not code: one `connectors/<id>.toml`, one pad file and a facts sheet. No `.rs`
file and no test of your own. The test pad is the one way to add it.

1. Read the harness's current docs and fill the facts sheet (`docs/harnesses/<id>.md`).
2. Write `connectors/<id>.toml`. Copy a neighbour such as `connectors/hermes-cli.toml`. Every key is
   in [CONNECTOR-SCHEMA](docs/CONNECTOR-SCHEMA.md).
3. Write `scripts/pad/harnesses/<id>.toml` and run the pad on it.
4. Open the pull request with the run's `row.json`.

Start with the [adding-a-capture-harness skill](skills/adding-a-capture-harness/SKILL.md). The
harnesses that exist are in [docs/harnesses/README.md](docs/harnesses/README.md).

## For developers

<a id="build"></a>You need Rust 1.99 (`rust-toolchain.toml` selects it; install [rustup](https://rustup.rs)), a C
compiler (macOS: the Xcode command line tools; Debian or Ubuntu: `build-essential`) and `python3`.

```sh
cargo fetch --locked
cargo build --release --locked
export PATH="$PWD/target/release:$PATH"
```

### The gate

One script, two tiers. Its last line is `gate <tier>: PASS in <N>s` or `gate <tier>: FAIL`. CI runs
`sh scripts/gate.sh fast` on every pull request and every push to `main` and `v1.0/integration`.

```sh
cargo install cargo-nextest --locked   # once per machine
npm ci --prefix server/worker          # once per checkout (Node 22.18 or later)
sh scripts/gate.sh fast                # every change, before it is merged
sh scripts/gate.sh full                # phase exits, before a release
```

- **fast:** `cargo fmt --check`, `cargo clippy -D warnings`, every Rust test under cargo-nextest, the
  doctests, the Worker tests and the script tests.
- **full:** fast, plus every ignored test (the end-to-end run), the release build, and a check that
  no test hook text is in the shipped `goalcup`.
- **Run it as a normal user, not root.** Some permission tests always fail as root, so the gate
  stops at once. `GOALCUP_GATE_ALLOW_ROOT=1` overrides this, for a throwaway container.
- It needs `cargo-nextest`, `zstd`, `python3`, `git`, `npm` and Python 3.11 or later for the pad
  tests, and names every missing one before it starts. More: [scripts](scripts/README.md).

**The dependency rule.** The recorder never links the verifier, and the verifier never links the
recorder. They share only `goalcup-format`. Other hard rules (no base URL set by goalcup, no
harness names in core code, masking before disk) are in [AGENTS.md](AGENTS.md).

### The seven points and their tests

Each point has tests. A test that guards a point is never deleted while the point stands; it may
move tier ([AGENTS.md](AGENTS.md#tests)). The test pad's six checks (C1 to C6, PAD-RESULTS) prove
the same points on real harnesses.

| Point | Real-run proof | Guard tests |
|---|---|---|
| 1 Every model request | Pad C1 | `proxy_connect.rs`, `proxy_ws.rs` |
| 2 Every subagent | Pad C2 | `native_walk.rs`, `native_tail.rs`, `native_finish.rs`, `native_resume.rs` |
| 3 Never block | Recorder kill test | none merged yet |
| 4 No secrets | Pad C3 | `mask_stream.rs`, `mask_rules.rs`, `secrets_scan.rs` |
| 5 Upload everything | Pad C4 | `upload_chunks.rs`, `server_compare.rs`, Worker `test/integration.test.ts` |
| 6 Tell plainly | Pad C6 | `check_invariants.rs`, `cli_finish.rs`, `upload_beat.rs` |
| 7 Real harnesses | Pad tiers | `connector_builtin.rs`, `connector_launch.rs`, `rule_code.rs`, `scripts/test_install.py` |
| Usage | Pad C5 | `usage_run1.rs` |
| Format | Pad | `golden.rs`, `recovery.rs`, `check_invariants.rs` |
| All together | Done items 1 and 2 | none merged yet |

### Releasing

The version is written only in the workspace `Cargo.toml`. `sh scripts/release.sh prepare <x.y.z>`
opens the version pull request. After it merges, run `sh scripts/gate.sh full` on that commit, then
`scripts/pad/table --release-check <commit>`, then `sh scripts/release.sh tag <x.y.z>`
([details](packaging/README.md)). Release notes: [GitHub releases](https://github.com/agentcommunity/goal-cup-proxy/releases).

## Docs map

Every doc, grouped by who reads it.

| You are | Start with |
|---|---|
| A participant | [PARTICIPANT-GUIDE](docs/PARTICIPANT-GUIDE.md), [RULES](docs/RULES.md), [SECURITY](SECURITY.md), [harnesses](docs/harnesses/README.md) |
| An organizer | [OPERATING](docs/OPERATING.md), [UPLOAD-API](docs/UPLOAD-API.md), [server/worker](server/worker/README.md), [packaging](packaging/README.md) |
| A contributor | [AGENTS](AGENTS.md), [VISION](VISION.md), [CONTRIBUTING](CONTRIBUTING.md), [ARCHITECTURE](docs/ARCHITECTURE.md), [RECORD-FORMAT](docs/RECORD-FORMAT.md), [DECISIONS](docs/DECISIONS.md), [TODO](TODO.md) |
| A harness builder | [skill](skills/adding-a-capture-harness/SKILL.md), [CONNECTOR-SCHEMA](docs/CONNECTOR-SCHEMA.md), [facts sheets](docs/harnesses/README.md), [scripts](scripts/README.md) |
| Evidence | [the first two-hour run](docs/research/results/2026-10-30-two-hour-run.md) |

Plans are in `docs/plans/`. Finished work and old narratives are in `docs/archive/` and
`docs/research/`; do not plan from them.

## Support, security and license

- Participant support: goalcup@agentcommunity.org. Paste your `goalcup status` output and run id,
  never your GitHub token.
- Public problems: [GitHub issues](https://github.com/agentcommunity/goal-cup-proxy/issues).
- Security problems: report privately to goalcup@agentcommunity.org or through GitHub private
  vulnerability reporting ([SECURITY.md](SECURITY.md)). Contributing: [CONTRIBUTING.md](CONTRIBUTING.md),
  [Code of Conduct](CODE_OF_CONDUCT.md). Maintainer: Balazs Nemethi.

Apache License 2.0 ([LICENSE](LICENSE), [NOTICE](NOTICE)). Copyright 2026 AgentCommunity.
