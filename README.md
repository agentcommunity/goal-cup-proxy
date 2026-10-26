# Goal Cup Proxy

Goal Cup Proxy (`goalcup`) records what a coding agent does during a run. It launches the agent's
harness (Codex CLI, Claude Code, Hermes or Amp) in your terminal and records three things: the
harness's model traffic, the harness's own session files, and the state of the run's Git
repository. It seals them into one hash chain and uploads it to the Goal Cup server while the run
goes on. Everything runs on the participant's own machine. Anyone can check a sealed trace offline with
`goalcup-verify`.

Its first use is the /goal Cup: one prompt, one agent, 168 hours. The organizer is
[Agent Community](https://agentcommunity.org). Sign-up, the rules summary, the demo video and the
questionnaire are on the event site, [agentcommunity.org/goalcup](https://agentcommunity.org/goalcup).

Agents working in this repository: read [AGENTS.md](AGENTS.md) first.

## Quick start for participants

The full walk-through, with every message you will see, is the
[participant's guide](docs/PARTICIPANT-GUIDE.md). The contract is [RULES](docs/RULES.md). You can
also hand the setup to your own agent with the [goal-cup-setup skill](skills/goal-cup-setup/SKILL.md).

1. **Install** the latest release (macOS or Linux; on Windows, inside WSL2):

   ```sh
   curl -fsSL https://github.com/agentcommunity/goal-cup-proxy/releases/latest/download/install.sh | sh
   goalcup --version
   ```

   It checks the download against the release's `SHA256SUMS` and installs into `~/.local/bin`
   and `~/.local/share/goalcup`, with no sudo. Run it again to upgrade, only with no run open.
   If a start is refused right after an update, wait a minute (the old recorder exits when idle)
   and start again.
   For one particular release, add `-s -- --version <x.y.z>` after `sh`.
2. **Set up**: `goalcup setup`. Four steps: it connects this computer to your account, lists the
   harnesses and lets you pick one, opens it once in its own isolated home so you sign in (or
   paste your API key; Goal Cup never sees it) and set it never to stop for approval
   ([set up your harness for a 168-hour run](docs/HARNESS-SETUP.md)), and checks this machine.
   Run it until every step says `[ok]`. It needs no GitHub and keeps no key. For Hermes, the
   newest release from Hermes's own installer is fine with an API-key service such as DeepSeek;
   see [Hermes setup](docs/HARNESS-SETUP.md#hermes) for the setups that need 0.19.0.
3. **Practice** (optional): in an empty folder, `goalcup start --harness <id> --practice`. About
   ten minutes, no GitHub, no token, never counts. Do as many as you like.
4. **Make the repository, the token and the folder**, in this order. Create a new, empty, public
   GitHub repository (no README, `.gitignore` or licence). Make a fine-grained token for that one
   repository: Contents read and write, expiry 30 days. Make a new folder holding `.env.local`
   (exactly one line, `GH_TOKEN=<token>`) and a `.gitignore` with the line `.env*`. Details:
   [the guide, section 5](docs/PARTICIPANT-GUIDE.md#5-your-run).
5. **Start** inside the start window, in that folder:
   `goalcup start --harness <id> --repo <user>/<name>`. A dry run is open from 23 October, so you
   can start a rehearsal now ([the dry run](docs/PARTICIPANT-GUIDE.md#the-dry-run-optional)).
   `goalcup` checks the folder and the repository before it asks for the browser approval. The
   harness opens in the folder, and your agent runs `git init` and pushes with the token. Its own
   setup screens (sign-in, folder trust, model choice) come before your prompt, and answering them
   is fine. Delete the token after your run.
6. **Paste the block `goalcup` prints** (the repository, the token file, the git lines) at the
   start of your prompt, **type your one prompt** and press Enter. After it, the only message you
   may ever type is "keep going".

**During the run.**

- The server is the clock. The run ends 168 hours after the server's official start.
- Check on it from a second terminal with `goalcup status`. Its `Next:` line, above the `Details:`
  block, says what to do.
- Your agent may run `goalcup time-left`, or call the `time_left` tool of `goalcup mcp` if you
  added it to your harness, to ask how much time is left. Goal Cup Proxy never sends
  anything to the agent: no reminders, no "keep going", no restarts.
- If the agent stops early (a crash, a reboot, a closed window), run `goalcup continue`. Nobody
  restarts it for you.
- No server error stops a run. Recording goes on, and uploads are retried, with longer waits
  between tries.
- If the disk is nearly full (under 1 GiB free beyond the run's reservation), the agent is paused
  until space returns, then resumes by itself. Free space outside your run's folders.
- Keep the machine on mains power, awake and online, and keep the terminal open.

**The end.** At the deadline Goal Cup Proxy stops the agent, seals the record and uploads the
rest. You do nothing. What is judged is the last push to your repository's default branch that
GitHub received before the deadline, by GitHub's timestamp. Your agent must push its own work;
Goal Cup Proxy never pushes. Then hand in your demo video and questionnaire on the event site
within 24 hours. More: [the end](docs/PARTICIPANT-GUIDE.md#7-the-end).

## Commands

A **run** is your one 168-hour attempt. A **part** is one unbroken stretch of recording inside it;
most runs have one, and `goalcup continue` starts the next. Every flag is in `goalcup <command> --help`. Exit codes: 0 done; 1 refused, and you can fix it;
3 the command could not do its job.

| Command | What it does |
|---|---|
| `goalcup setup [--harness <ID>] [--json] [--dev]` | Check the four readiness steps. Never starts a run. `goalcup prep` is its old name. |
| `goalcup login [--server <URL>]` | Connect this computer to your Goal Cup account (once; `setup` does it for you). |
| `goalcup start --harness <ID> [--practice \| --test] [--repo <OWNER/NAME>] [--dev]` | Start a run: open the harness in the current folder and record it. Run it in the project folder (empty but for `.env.local` and `.gitignore`). A real run needs its own new, empty, public GitHub repository, given as `--repo <user>/<name>`; `start` checks it before the approval. `--test` is for organizers; `--dev` is for trying an unpinned connector and never competes. |
| `goalcup status [<part-id>] [--json]` | Show the run: recording state, time left, upload, problems, `Next:`. `--json` prints the signed status document, with this machine's `local` facts and a `problems` array (the `PROBLEM:` lines' sentences, left out when there are none) beside it. |
| `goalcup time-left` | Show how much time the run has left. The agent may run it too. |
| `goalcup mcp` | Serve the time left as an MCP tool (`time_left`) for the agent. The harness starts it; add it as in [HARNESS-SETUP](docs/HARNESS-SETUP.md#add-the-goal-cup-time-left-tool). |
| `goalcup continue [<run-id>]` | Start the agent again in the same run, on the same computer. |
| `goalcup upload [<part-id>]` | Send what is not uploaded yet. Safe to repeat. |
| `goalcup entries [--json]` | List the recorded parts of your runs on this computer. |
| `goalcup seal [<part-id>] [--yes]` | Stop the agent now and seal this part. The run stays open. |
| `goalcup doctor [--harness <ID> \| all] [--json]` | Check that this computer can record and each harness is installed. |

`goalcup-verify --capture-integrity <trace>` checks a sealed trace (0 complete and valid,
1 incomplete or not sealed, 2 invalid, 3 no usable report). Exit 0 says the record is consistent,
not that the run was good. `goalcup-verify --research-json <trace> --output <new.ndjson>` writes a
private research report. `goalcup-verify --stats <trace>… --output <new.json>` writes one run's
numbers (tokens, models, tool calls and errors, subagents, "keep going") and no text, with the same
exit codes. Every command and exit code: [OPERATING](docs/OPERATING.md).

## How it works

```
participant types the prompt
        |
        v
harness  --HTTPS_PROXY-->  recording proxy (in goalcup, loopback)  -->  model provider
   |                              | decoded bodies
   | session files                v
   +---------------------->  goalcup-daemon: redact, append to the hash chain
                                  | sealed at the end, with the Git commit and a
                                  | digest of uncommitted changes
                                  v
                  goalcup start uploads chunks and heartbeats  -->  Goal Cup server
                                                     (signs a start receipt and an ack per chunk)

goalcup-verify reads a sealed trace offline: integrity check, research report and run stats.
```

- **The wire leg.** The harness reaches its model through a local recording proxy, set by
  `HTTPS_PROXY` for that one process only. Goal Cup Proxy never sets a model base URL. Bytes are
  relayed unchanged; no header is recorded.
- **The native leg.** The daemon reads the harness's session files from an isolated home, never
  your own `~/.claude` or `~/.codex`.
- **The chain.** Secrets are removed before anything is written. Any detected loss is a gap in
  the chain, never an invented record. The chain catches a stray edit. Server acknowledgements,
  signed with a key pinned in the verifier, catch a rewrite of what was uploaded
  ([cheat checklist](docs/CHEAT-CHECKLIST.md)).
- <a id="the-daemon"></a>**The daemon.** There is nothing to install. `goalcup start` starts
  `goalcup-daemon` for the run. It exits 30 seconds after the last run is sealed, and logs to
  `$GOALCUP_CONFIG_DIR/daemon.log`. It stores raw bytes; the verifier makes every judgement later.

| Crate | Ships | What it holds |
|---|---|---|
| `goalcup-protocol` | library | Every byte definition: chain, records, control API, harness schema, connector pins. |
| `goalcup-daemon` | `goalcup-daemon` | Ingest, redaction, chain writer, session-file capture, seal, process termination. |
| `goalcup-cli` | `goalcup` | The eleven commands, the harness launch, the recording proxy, the upload. |
| `goalcup-verifier` | `goalcup-verify` | The integrity reader and the decoders. Every judgement about the bytes. |
| `goalcup-platform` | library | Unix OS code: disk reservation, safe file access, machine facts, sleep lock. |

Also: `connectors/<id>/` (one harness as data), `server/worker` (the upload server, a Cloudflare
Worker). The details and the glossary (run, part, segment, chain file, seal, receipt, ack):
[ARCHITECTURE](docs/ARCHITECTURE.md).

## Platforms

| Platform | Status |
|---|---|
| macOS, Apple silicon | Measured. Use the arm64 build of `goalcup` and of the harness. |
| macOS, Intel | Builds. No run measured. |
| Linux, arm64 | Docker gates and logged-in Codex and Hermes staging practices passed. Longer runs pending ([measurements](TODO.md#wp0-measure)). |
| Linux, x86_64 | Reservation tests and a no-login harness smoke pass on a hosted runner. Full suites not measured. |
| Windows | Use WSL2 (below). Real-machine validation is pending. Native Windows is planned. |

The trace folder must be on APFS, ext4 or XFS: `goalcup setup` fails its step otherwise, and
`start` and `continue` warn. On Linux, install the
harness as a native binary first on `PATH` (not an npm wrapper). On a machine with no browser, set
`GOALCUP_NO_BROWSER=1`: `login` and `start` print the approval address instead. Known issues:
[TODO](TODO.md#known-issues).

<a id="windows-use-wsl2"></a>**Windows: use WSL2.** In PowerShell run `wsl --install -d Ubuntu`,
then do everything inside Ubuntu as on Linux. Keep every folder a run uses on the Linux filesystem
(`~/...`), never on `/mnt/c`. Goal Cup Proxy refuses WSL1 and Windows-drive folders. It cannot
keep Windows awake, so set Windows sleep to Never for the run.

## Add a harness

A harness is data, not code: one `connectors/<id>/harness.toml` (chat mode), a pin line and a
facts sheet. No `.rs` file and no test of your own. Start with the
[adding-a-capture-harness skill](skills/adding-a-capture-harness/SKILL.md), then follow
[Add and test a connector](docs/HARNESS-ONBOARDING.md#quick-path). The short path:

1. **Check it can be captured** ([check first](docs/HARNESS-ONBOARDING.md#row-0)). The two facts
   that most often need a core change: the CA variable the harness reads, and how it keeps its
   sign-in without an environment key.
2. **Fill the facts sheet**: copy `skills/adding-a-capture-harness/templates/harness-facts.md` to
   `docs/harnesses/<id>.md`.
3. **Copy the template**: `skills/adding-a-capture-harness/templates/connector-harness.toml` to
   `connectors/<id>/harness.toml`. Fill the one-screen field table in
   [What a chat connector writes](docs/CONNECTOR-SCHEMA.md#what-a-chat-connector-writes). The
   closest example is [connectors/codex-cli](connectors/codex-cli/harness.toml).
4. **Pin and regenerate**: `sh scripts/pin-manifests.sh`, then the two golden commands in
   [Test a connector](docs/HARNESS-ONBOARDING.md#test-a-connector).
5. **Check**: `sh scripts/check-connector.sh <id>`, then `sh scripts/gate.sh fast`.
6. **Run it for real**: a local smoke run, then a practice run on the server
   (`goalcup start --harness <id> --practice`), checked with `goalcup-verify --capture-integrity`.

A harness can join the event at any time: releases continue during the cup, and a merged connector
ships in the next one. A maintainer does a practice run of it first ([getting a harness into the event](docs/HARNESS-ONBOARDING.md#getting-a-harness-into-the-event)).

## For developers

<a id="build"></a>**Build from source.** You need Rust 1.99 (`rust-toolchain.toml` selects it;
install [rustup](https://rustup.rs)), a C compiler (macOS: the Xcode command line tools; Debian or
Ubuntu: `build-essential`), `python3`, and `shasum` or `sha256sum`. From the repository root:

```sh
cargo fetch --locked             # once: download the dependencies
cargo build --release --offline
export PATH="$PWD/target/release:$PATH"
```

A binary copied out of `target/release` needs `GOALCUP_CONNECTORS_DIR="<checkout>/connectors"`.

**Try a local run without an account.** The harness must already be installed on this machine
(`goalcup doctor --harness claude-code-cli` checks). Use a fresh config, so nothing is uploaded:

```sh
export GOALCUP_CONFIG_DIR="$(mktemp -d "$HOME/goalcup-local.XXXXXX")"
goalcup setup --harness claude-code-cli   # sign in to the harness; the [todo] lines are fine locally
cd "$(mktemp -d "$HOME/goalcup-run.XXXXXX")"   # outside any Git repository
goalcup start --harness claude-code-cli   # type a prompt, then exit the harness to seal
```

Then run the `verify:` command it prints. Only one run per harness records at a time.

### The gate

One script, two tiers. Its last line is `gate <tier>: PASS in <N>s` or `gate <tier>: FAIL`.

```sh
cargo install cargo-nextest --locked   # once per machine
cargo fetch --locked                   # once per machine
sh scripts/gate.sh fast                # every change, before it is committed
sh scripts/gate.sh full                # before a release
```

- **fast:** the version check, `cargo build`, `cargo fmt --check`, `cargo clippy -D warnings`,
  every test except the slow tier (under cargo-nextest), and the doctests. A few minutes warm.
- **full:** fast, plus the slow tier, the CLI's ignored end-to-end tests, the Worker's tests
  (Node 22.18+ and `npm ci` in `server/worker`), the scripts' unit tests and the link check. Run
  it before every tag.
- **Run the gate as a normal user, not root.** Some permission tests always fail as root, so the
  gate stops at once when run as root. Set `GOALCUP_GATE_ALLOW_ROOT=1` to run it anyway, for
  example in a throwaway container, and expect those tests to fail.
- On Linux the gate also needs `nc`, procps `ps`, `*.localhost` on loopback, IPv6 on loopback and
  `TMPDIR` on ext4 or XFS. A plain Debian or Ubuntu container does not resolve `*.localhost`
  (it lacks `nss-myhostname`): add `127.0.0.1 api.e00.localhost auth.e00.localhost` to its
  `/etc/hosts`. If `/tmp` is tmpfs or btrfs and your home is ext4 or XFS, run this first:
  `export TMPDIR="$HOME/.cache/goalcup-tmp"; mkdir -p "$TMPDIR"`. CI (`check`) is a small Linux
  smoke, not the gate. More: [OPERATING](docs/OPERATING.md#building-and-verifying).

**The dependency rule.** The daemon never links the verifier, and the CLI never links the daemon.
Tests check both. Other hard rules (never a base URL, no harness names in core code, pins
regenerated with every connector edit) are in [AGENTS.md](AGENTS.md#hard-rules).

### The promises

A test that guards one of these is never deleted. It may move tier.

| | Promise | Where the tests are |
|---|---|---|
| P1 | No key fragment leaks | daemon redaction suites; cli connector contract and kit (no provider key reaches a harness, D61), Hermes offline key |
| P2 | The chain catches a stray edit | protocol chain golden; verifier capture integrity, v5 corpus, containment |
| P3 | Capture policy refuses endpoints, proxies, credentials and bad args | protocol harness schema and conformance; cli launch plan, dependency bans |
| P4 | Connectors load and pins match | protocol connector pins and loader; cli shipped connectors |
| P5 | The control API authenticates | daemon control API |
| P6 | The recording proxy captures faithfully | cli TLS, HTTP and WebSocket relay; verifier decoders |
| P7 | One end-to-end capture per harness, and termination | cli lifecycle suites and contract kit; daemon capture lifecycle and process kill |

### Releasing

The version is written only in the workspace `Cargo.toml` (`scripts/check-version.py` fails on
any other copy). `sh scripts/release.sh prepare <x.y.z>` opens the version PR; after it merges, run
the full gate and `sh scripts/linux-check.sh` on that commit, then `sh scripts/release.sh tag <x.y.z>`
([details](packaging/README.md#cutting-a-release)). Release notes: [GitHub releases](https://github.com/agentcommunity/goal-cup-proxy/releases).

## Docs map

Every doc, grouped by who reads it: [docs/README.md](docs/README.md). The key ones:

| You are | Start with |
|---|---|
| A participant | [PARTICIPANT-GUIDE](docs/PARTICIPANT-GUIDE.md), [HARNESS-SETUP](docs/HARNESS-SETUP.md), [RULES](docs/RULES.md), [SECURITY](SECURITY.md) |
| A harness builder | [skill](skills/adding-a-capture-harness/SKILL.md), [HARNESS-ONBOARDING](docs/HARNESS-ONBOARDING.md), [CONNECTOR-SCHEMA](docs/CONNECTOR-SCHEMA.md), [facts sheets](docs/harnesses/README.md) |
| A developer | [AGENTS](AGENTS.md), [ARCHITECTURE](docs/ARCHITECTURE.md), [DECISIONS](docs/DECISIONS.md), [CONTRACTS](docs/CONTRACTS.md), [TODO](TODO.md) |
| An organizer | [OPERATING](docs/OPERATING.md), [UPLOAD-API](docs/UPLOAD-API.md), [SERVER-DESIGN](docs/SERVER-DESIGN.md), [server/worker](server/worker/README.md) |
| A researcher | [RESEARCH-TARGET](docs/RESEARCH-TARGET.md), [CHEAT-CHECKLIST](docs/CHEAT-CHECKLIST.md), [coverage matrix](docs/research/coverage-matrix.md) |

## Support, security and license

- Participant support: goalcup@agentcommunity.org. Paste your `goalcup status` output and run id,
  never your GitHub token.
- Public problems: [GitHub issues](https://github.com/agentcommunity/goal-cup-proxy/issues).
- Security problems: report privately to goalcup@agentcommunity.org or through GitHub private
  vulnerability reporting ([SECURITY.md](SECURITY.md)). Contributing: [CONTRIBUTING.md](CONTRIBUTING.md),
  [Code of Conduct](CODE_OF_CONDUCT.md). Maintainer: Balazs Nemethi.

Apache License 2.0 ([LICENSE](LICENSE), [NOTICE](NOTICE)). Copyright 2026 AgentCommunity.
