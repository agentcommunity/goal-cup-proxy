# goal-cup

Goal Cup is a one-prompt, 168-hour coding-agent contest. This repository is the tracer. It
launches a participant's coding agent (the harness) on their machine, records the agent's model
traffic and its own session files, seals them into one hash-chained trace, and uploads the trace
to the Goal Cup server. `goalcup-verify` checks a trace offline.

The chain catches a stray edit and accidental corruption. A trace that never reached the server
can be edited, re-hashed and re-sealed, and it still verifies. A trace uploaded during the run
carries server acknowledgements signed with a key pinned in the verifier. Those catch a rewrite
of everything acknowledged ([cheat checklist](docs/CHEAT-CHECKLIST.md)).

Support: goalcup@agentcommunity.org. Participants: start with
[docs/PARTICIPANT-GUIDE.md](docs/PARTICIPANT-GUIDE.md). This README is for people who build, test
or change the tracer. Agents: start with [AGENTS.md](AGENTS.md).

## Install

**From a release** (macOS and Linux; on Windows, inside WSL2):

```sh
curl -fsSL https://github.com/agentcommunity/goal-cup-tracer/releases/latest/download/install.sh | sh
```

It picks the build for your machine (macOS or Linux, arm64 or x86_64), checks its SHA-256 against
the release's `SHA256SUMS`, and installs `goalcup`, `goalcup-daemon` and `goalcup-verify` into
`~/.local/bin` and the connectors into `~/.local/share/goalcup`. No sudo. It edits no shell
profile; if `~/.local/bin` is not on your `PATH` it says how to add it. Run it again to upgrade, or
add `-s -- --version <x.y.z>` after `sh` for a particular release. The builds are unsigned, and a
download by `curl` is not quarantined on macOS, so they run. There is no Homebrew tap; native
Windows comes later. The release workflow and the script:
[packaging/README.md](packaging/README.md).

<a id="build"></a>**From source.** You need Rust 1.99 (`rust-toolchain.toml` selects it; install
[rustup](https://rustup.rs)), a C compiler (macOS: the Xcode command line tools; Debian or
Ubuntu: `build-essential`), `python3` and `shasum` or `sha256sum`. From the repository root:

```sh
cargo build --release
export PATH="$PWD/target/release:$PATH"
```

This builds `goalcup`, `goalcup-daemon` and `goalcup-verify`. A binary copied out of
`target/release` cannot find the connectors: set `GOALCUP_CONNECTORS_DIR="<checkout>/connectors"`.

## Quickstart

A local run needs no login and uploads nothing. You need a supported platform ([Platforms](#platforms)),
a Claude or ChatGPT subscription, and Claude Code or Codex CLI installed. The tracer runs the build
it finds on `PATH` and records which build ran.

```sh
export GOALCUP_CONFIG_DIR="$HOME/goalcup"         # where the tracer keeps its state
goalcup prep --harness claude-code-cli            # sign in once in the tracer's own harness home (press Enter when it asks)
# then, from an empty folder outside any git repository:
goalcup doctor --harness claude-code-cli          # optional: checks the connector files, the harness on PATH and the daemon
goalcup start --harness claude-code-cli           # the harness opens here: answer its trust question, type your prompt, press Enter; exit the harness (Claude Code: /exit) to end the run
goalcup-verify --capture-integrity <trace>        # copy the `verify:` line that start printed
```

`prep` steps that need a login print `[todo]`. That is fine for a local run. A local run asks for no
browser approval; a run on the server asks for one at each `goalcup start`, and `goalcup login`
asks once ([why](docs/PARTICIPANT-GUIDE.md#2-log-in-goalcup-login)). There is no daemon to
install: `goalcup start` starts `goalcup-daemon` for the run. Both shipped chat-mode harnesses (Codex
CLI, Claude Code) refuse `--goal-file`; Hermes is headless and needs it. From another terminal with
the same `GOALCUP_CONFIG_DIR`, `goalcup status` shows the run and `goalcup seal` ends it early.
Only one run per harness can record at a time.

A run on the server needs `goalcup login`, `goalcup prep` and a practice run first
([participant's guide](docs/PARTICIPANT-GUIDE.md)).

## Platforms

| Platform | Status |
|---|---|
| macOS, Apple silicon | Measured. Use the arm64 build of the tracer and of the harness. |
| macOS, Intel | Builds. No run measured. |
| Linux, arm64 | Fast gate passes in Docker. No logged-in harness run measured. See [Linux](#linux). |
| Linux, x86_64 | Reservation tests and a no-login harness smoke pass on a hosted runner. The full suites are not measured. |
| Windows | Run the tracer in WSL2. See [Windows](#windows-use-wsl2). Native Windows is planned. |

The trace folder (`artifact_root` in `daemon.toml`) must be on APFS, ext4 or XFS. The tracer
refuses btrfs, ZFS, NFS, tmpfs, overlayfs and FUSE before it creates anything.

## Linux

The quickstart works on Linux with three differences:

- **Install the harness as a native binary first on `PATH`** (the npm platform package, not the
  `npm i -g` wrapper): the tracer runs and records whatever `PATH` resolves, so a wrapper is
  recorded in place of the binary.
- **The configuration directory** is `$XDG_CONFIG_HOME/goalcup`, else `~/.config/goalcup`
  (macOS: `~/Library/Application Support/goalcup`). `GOALCUP_CONFIG_DIR` overrides both.
- **A headless or remote machine** has no browser to open. Set `GOALCUP_NO_BROWSER=1` and `goalcup login` and
  `goalcup start` print the approval address and code and open nothing; open the address on any device.
- **The Claude Code login.** Linux has no Keychain. Sign in once with
  `goalcup prep --harness claude-code-cli`, which opens Claude Code in the tracer's own home.

The trace folder defaults to `<config dir>-artifacts`, beside the configuration directory. Check
its filesystem, or its parent's before the first run, with `stat -f -c %T <folder>` (`xfs` is XFS;
`ext2/ext3` is what GNU `stat` prints for ext4). On Linux `goalcup start` is a child subreaper and ends processes the
harness left behind before the seal ([TERMINATION](docs/TERMINATION.md)). What is and is not
measured on Linux: [TODO](TODO.md#known-issues).

## Windows: use WSL2

Native Windows is not supported yet. For the event, run everything inside WSL2 as Linux:

1. In PowerShell: `wsl --install -d Ubuntu`.
2. Inside Ubuntu, install Rust, the build tools and the harness as in [Linux](#linux).
3. Keep every folder a run writes to on the Linux filesystem (`~/...`), never on a Windows drive
   (`/mnt/c`).

On WSL the tracer refuses WSL1 (`goalcup prep`, `start` and `continue` say how to convert), and
refuses a work folder, `artifact_root`, configuration folder or run folder on a Windows-host
filesystem (9p, drvfs, virtiofs, CIFS, NTFS). It takes no sleep lock under WSL: set Windows sleep
to Never for the run. Not yet tested on a real Windows machine.

## The gate

One script, two tiers. Its last line is `gate <tier>: PASS in <N>s` or `gate <tier>: FAIL`.

```
sh scripts/gate.sh fast   # every change, before it is committed
sh scripts/gate.sh full   # before a release
```

Once per machine:

```
cargo install cargo-nextest --locked
cargo fetch --locked          # the gate runs cargo --offline
```

- **fast:** the version check (`scripts/check-version.py`: the version is written in the workspace
  `Cargo.toml` only), `cargo build --workspace`, `cargo fmt --all --check`, `cargo clippy
  --workspace --all-targets -- -D warnings`, every test except the slow tier (under
  cargo-nextest), then the doctests. About a minute warm.
- **full:** fast, plus the slow tier, the CLI's `#[ignore]`d end-to-end tests
  (`m2_synthetic_vertical`, `capture_failure_campaign`, `upload_worker`), the Worker's own tests
  (`npm --prefix server/worker test`), the scripts' unit tests and the link check. Run it before
  every tag: nothing else runs the slow tier. The Worker tests need Node 22.18+ and `npm ci` in
  `server/worker`.
- Without cargo-nextest the gate falls back to `cargo test`, which runs every tier.

`cargo build` comes first on purpose: the CLI tests spawn the sibling `target/debug/goalcup-daemon`,
so a stale daemon would be tested. If worktrees share `CARGO_TARGET_DIR`, never run two gates at
once. Integration tests are modules of two binaries per crate, `tests/it/` (fast) and
`tests/full/` (slow). A few suites that change process state stay standalone. Serial groups are in
`.config/nextest.toml`.

On Linux the gate also needs `nc`, procps `ps` (not busybox), `*.localhost` resolving to
loopback, and `TMPDIR` on ext4 or XFS. `sh scripts/linux-check.sh` runs the Linux part in Docker;
the manual `linux-x86_64` workflow runs it on x86_64. CI (`check`, `.github/workflows/ci.yml`) is a
small Linux smoke that skips the daemon and CLI suites. It is not the gate.
More: [docs/OPERATING.md](docs/OPERATING.md#building-and-verifying).

### The promises

A test that guards one of these is never deleted. It may move tier.

| | Promise | Where the tests are |
|---|---|---|
| P1 | No key fragment leaks | cli key scan, stderr tee, child-secret suites; daemon redaction suites |
| P2 | The chain catches a stray edit | protocol chain golden; verifier capture integrity, v5 corpus, containment |
| P3 | Capture policy refuses endpoints, proxies, credentials and bad args | protocol harness schema and conformance; cli launch plan, dependency bans |
| P4 | Connectors load and pins match | protocol connector pins and loader; cli shipped connectors |
| P5 | The control API authenticates | daemon control API |
| P6 | The proxy captures faithfully | cli TLS, HTTP and WebSocket relay; verifier decoders |
| P7 | One end-to-end capture per harness, and termination | cli lifecycle suites and contract kit; daemon capture lifecycle and process kill |

## Releasing

The workspace `version` in `Cargo.toml` is the only place a version is written; the gate fails on
any other copy. `main` is protected, so a release is two steps that never push `main`:
`sh scripts/release.sh prepare <x.y.z>` opens a pull request that bumps the version (it does
nothing if `origin/main` already has it), and, once that is merged, `sh scripts/release.sh tag
<x.y.z>` tags `origin/main` and pushes only the tag. The tag starts the release workflow, which
refuses a tag that differs from `Cargo.toml`. Details and how to try the build by hand:
[packaging/README.md](packaging/README.md#cutting-a-release).

## Run a capture

The reference behind the quickstart. Every flag is in `goalcup <command> --help`; config keys are
in [OPERATING](docs/OPERATING.md#running-the-daemon).

- **Where things go.** A run's folder (the harness `HOME`, and for a local run its `workspace/`) is
  made in the folder you run `goalcup start` from, or beside the goal file for a headless harness.
  The sealed trace is under the daemon's `artifact_root`. A run on the server works in a clone of
  the participant's GitHub repository instead.
- **What the harness sees.** Its environment is built from nothing. Of your shell only `USER`,
  `LOGNAME`, `TMPDIR` and, for a keyed connector, its one API key reach it. The tracer never sets a
  model base URL. The only proxy the harness gets is the tracer's own loopback recorder.
- <a id="the-daemon"></a>**The daemon.** `goalcup start` uses the daemon that answers on
  `$GOALCUP_CONFIG_DIR/control.json` and starts `goalcup-daemon --exit-when-idle` when none does. It
  is found beside `goalcup` or on `PATH`, runs in its own session with an environment built from
  nothing, and logs to `$GOALCUP_CONFIG_DIR/daemon.log`. It exits 30 seconds after the last run is
  sealed. If a run outlives its daemon, the next `status`, `entries` or `seal` starts a daemon that
  reopens it for status and seal only, and that daemon stays until `goalcup seal` finishes the run.
- **The server.** `goalcup login` approves the tracer in the browser. `goalcup start` then asks for
  approval of each run, clones the participant's repository, opens the run on the server and keeps
  the server's start receipt as `receipt.json`. While the run records, the CLI uploads the chain in
  chunks and sends heartbeats. Recording never waits on the network. After the stop it seals and
  uploads the rest for a bounded wrap-up; `goalcup upload` finishes the job later. With no entry id it sends the unsent entries of the newest run; `goalcup upload <entry>` sends one entry. The
  deadline is 168 hours after the server's official start, on the server's clock: at the deadline
  the CLI stops the agent and seals. Wire contract: [docs/UPLOAD-API.md](docs/UPLOAD-API.md).
  Server code: [server/worker](server/worker/README.md).
- **Hermes** (`hermes-cli`) is headless, needs a provider key, and is run by the maintainer only:
  `/bin/sh -p scripts/with-deepseek-key.sh <ENV_FILE> start --harness hermes-cli --goal-file <PATH>`
  with a mode 0600 file holding one `DEEPSEEK_API_KEY=` line. Never paste the key into a shell.

**`goalcup-verify --capture-integrity` exit codes:** 0 complete
and valid, 1 incomplete or not sealed, 2 invalid (including a torn last record), 3 no usable report.
Exit 0 says the record is complete and consistent. It is not a judged result.
`goalcup-verify --research-json <trace> --output <REPORT.ndjson>` writes the private research report.

## More

| For | Read |
|---|---|
| How the parts fit, and the terms | [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) |
| Daemon config, every command and exit code, the verifier, pins | [docs/OPERATING.md](docs/OPERATING.md) |
| Adding a harness | [skills/adding-a-capture-harness](skills/adding-a-capture-harness/SKILL.md), then [docs/HARNESS-ONBOARDING.md](docs/HARNESS-ONBOARDING.md) |
| What is next | [TODO.md](TODO.md) |
| What a participant agrees to | [docs/RULES.md](docs/RULES.md) |
| Rules that bind the code | [docs/DECISIONS.md](docs/DECISIONS.md) |
| Working here | [AGENTS.md](AGENTS.md), [CONTRIBUTING.md](CONTRIBUTING.md) |

`harnesses/` holds local, isolated harness installs for tests (git-ignored except `install.sh`).
`harnesses/env.sh` is generated by `harnesses/install.sh`. You need it only for real-harness tests,
never to build or run the gate.

## Contributing and license

Harness connectors are welcome: [CONTRIBUTING.md](CONTRIBUTING.md). Report security issues
privately ([SECURITY.md](SECURITY.md)). Everyone follows the [Code of Conduct](CODE_OF_CONDUCT.md).

Apache License 2.0 ([LICENSE](LICENSE), [NOTICE](NOTICE)). Copyright 2026 AgentCommunity. The
notices a binary distribution needs are generated into each release tarball
(`scripts/third-party-licenses.py`).
