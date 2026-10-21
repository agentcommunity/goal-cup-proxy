# goal-cup

This repository holds Goal Cup Proxy, a recorder for research into running coding agents. Its target is to retain as
much useful evidence of agent work as possible across harnesses and macOS, Linux and Windows,
within explicit capture and privacy boundaries. Current platform support is listed below;
native Windows remains planned.

Goal Cup Proxy launches a coding agent (the harness), records declared model traffic and native
session artifacts, and seals them into one hash-chained trace. `goalcup-verify` checks a trace
offline and produces research observations through versioned decoders. Goal Cup's one-prompt,
168-hour contest is one use of that evidence; server runs upload to the Goal Cup server.

The [research target](docs/RESEARCH-TARGET.md) defines the intended evidence and acceptance
criteria. The [local research handoff](docs/LOCAL-RESEARCH-HANDOFF.md) records the next validation
work. Tool calls and results visible in the agent's context are the target; separate collection
of full tool-output files is not required. Subagent conversations are valuable evidence in their
own right, and their coverage must be measured for each harness.

The chain catches a stray edit and accidental corruption. A trace that never reached the server
can be edited, re-hashed and re-sealed, and it still verifies. A trace uploaded during the run
carries server acknowledgements signed with a key pinned in the verifier. Those catch a rewrite
of everything acknowledged ([cheat checklist](docs/CHEAT-CHECKLIST.md)).

Support: goalcup@agentcommunity.org. Participants: sign up at
[agentcommunity.org/goalcup](https://agentcommunity.org/goalcup), then start with
[docs/PARTICIPANT-GUIDE.md](docs/PARTICIPANT-GUIDE.md). This README is for people who build, test
or change Goal Cup Proxy. Agents: start with [AGENTS.md](AGENTS.md).
Adding a harness? Start with [Add and test a connector](docs/HARNESS-ONBOARDING.md#quick-path).

## Install

**From a release** (macOS and Linux; on Windows, inside WSL2):

```sh
curl -fsSL https://github.com/agentcommunity/goal-cup-tracer/releases/latest/download/install.sh | sh
```

It picks the build for your machine (macOS or Linux, arm64 or x86_64), checks its SHA-256 against
the release's `SHA256SUMS`, and installs `goalcup`, `goalcup-daemon` and `goalcup-verify` into
`~/.local/bin` and the connectors into `~/.local/share/goalcup`. No sudo. It edits no shell
profile; if `~/.local/bin` is not on your `PATH` it says how to add it. Run it again to upgrade
(never during a competition run), or add `-s -- --version <x.y.z>` after `sh` for a particular
release. The builds are unsigned, and a download by `curl` is not quarantined on macOS, so they
run. There is no Homebrew tap; native
Windows comes later. The release workflow and the script:
[packaging/README.md](packaging/README.md).
If no binary release is published yet, use the source-build steps below.

<a id="build"></a>**From source.** You need Rust 1.99 (`rust-toolchain.toml` selects it; install
[rustup](https://rustup.rs)), a C compiler (macOS: the Xcode command line tools; Debian or
Ubuntu: `build-essential`), `python3` and `shasum` or `sha256sum`. From the repository root:

```sh
cargo fetch --locked       # one-time dependency download
cargo build --release --offline
export PATH="$PWD/target/release:$PATH"
```

This builds `goalcup`, `goalcup-daemon` and `goalcup-verify`. A binary copied out of
`target/release` cannot find the connectors: set `GOALCUP_CONNECTORS_DIR="<checkout>/connectors"`.

## Quickstart

A local run needs no login and uploads nothing. You need a supported platform ([Platforms](#platforms)),
a shipped CLI harness installed with the account or model service it uses. All shipped connectors
are available to participants; `goalcup doctor --harness all` checks the shipped roster and its
install status. It can exit 1 when an optional shipped harness is not installed. Goal Cup Proxy
runs the build on `PATH` and records it. Use a fresh config for the local example below; a config
that has been logged into the server follows the server-run path instead.

```sh
export GOALCUP_CONFIG_DIR="$(mktemp -d "$HOME/goalcup-local.XXXXXX")"  # fresh local-only test state
goalcup prep --harness claude-code-cli            # sign in once in Goal Cup Proxy's own harness home (press Enter when it asks)
RUN_DIR="$(mktemp -d "$HOME/goalcup-run.XXXXXX")"
cd "$RUN_DIR"                                    # keep the run workspace outside any git repository
goalcup doctor --harness claude-code-cli          # optional: checks the connector files, the harness on PATH and the daemon
goalcup start --harness claude-code-cli           # the harness opens here: answer its trust question, type your prompt, press Enter; exit the harness (Claude Code: /exit) to end the run
# After start exits, run the exact `verify:` command it printed.
```

`prep` steps that need a login print `[todo]`. That is fine for a local run. A local run asks for no
browser approval; a run on the server asks for one at each `goalcup start`, and `goalcup login`
asks once ([why](docs/PARTICIPANT-GUIDE.md#2-log-in-goalcup-login)). There is no daemon to
install: `goalcup start` starts `goalcup-daemon` for the run. The shipped harnesses (Codex CLI,
Claude Code, Hermes, Amp) run in chat mode and refuse `--goal-file`. From another terminal with
the same `GOALCUP_CONFIG_DIR`, `goalcup status` shows the run and `goalcup seal` ends it early.
Only one run per harness can record at a time.

A run on the server needs `goalcup login`, `goalcup prep` and a practice run first
([participant's guide](docs/PARTICIPANT-GUIDE.md)). A competition or test `goalcup start` refuses
a repository that is not empty: each such run needs a new one (`goalcup prep --repo <url>`).

## Platforms

| Platform | Status |
|---|---|
| macOS, Apple silicon | Measured. Use the arm64 build of `goalcup` and of the harness. |
| macOS, Intel | Builds. No run measured. |
| Linux, arm64 | Docker gates and logged-in Codex/Hermes staging practices passed. Longer runs remain pending. See [Linux](#linux) and [current measurements](TODO.md#wp0-measure). |
| Linux, x86_64 | Reservation tests and a no-login harness smoke pass on a hosted runner. The full suites are not measured. |
| Windows | Run Goal Cup Proxy in WSL2. See [Windows](#windows-use-wsl2). Native Windows is planned. |

The trace folder (`artifact_root` in `daemon.toml`) must be on APFS, ext4 or XFS. Goal Cup Proxy
refuses btrfs, ZFS, NFS, tmpfs, overlayfs and FUSE before it creates anything. `goalcup prep`
checks the configuration folder, the trace folder and the work folder for this and says how to
move them.

## Linux

The quickstart works on Linux with three differences:

- **Install the harness as a native binary first on `PATH`** (the npm platform package, not the
  `npm i -g` wrapper): Goal Cup Proxy runs and records whatever `PATH` resolves, so a wrapper is
  recorded in place of the binary.
- **The configuration directory** is `$XDG_CONFIG_HOME/goalcup`, else `~/.config/goalcup`
  (macOS: `~/Library/Application Support/goalcup`). `GOALCUP_CONFIG_DIR` overrides both.
- **A headless or remote machine** has no browser to open. Set `GOALCUP_NO_BROWSER=1` and `goalcup login` and
  `goalcup start` print the approval address and code and open nothing; open the address on any device.
- **The Claude Code login.** Linux has no Keychain. Sign in once with
  `goalcup prep --harness claude-code-cli`, which opens Claude Code in Goal Cup Proxy's own home.

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

On WSL Goal Cup Proxy refuses WSL1 (`goalcup prep`, `start` and `continue` say how to convert), and
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
cargo fetch --locked          # one-time dependency download; project build and gate commands use --offline
```

- **fast:** the version check (`scripts/check-version.py`: the version is written in the workspace
  `Cargo.toml` only), `cargo build --workspace --offline`, `cargo fmt --all --check`, `cargo clippy
  --workspace --all-targets --offline -- -D warnings`, every test except the slow tier (under
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
| P1 | No key fragment leaks | daemon redaction suites; cli connector contract and kit (no provider key reaches a harness, D61), Hermes offline key |
| P2 | The chain catches a stray edit | protocol chain golden; verifier capture integrity, v5 corpus, containment |
| P3 | Capture policy refuses endpoints, proxies, credentials and bad args | protocol harness schema and conformance; cli launch plan, dependency bans |
| P4 | Connectors load and pins match | protocol connector pins and loader; cli shipped connectors |
| P5 | The control API authenticates | daemon control API |
| P6 | The recording proxy captures faithfully | cli TLS, HTTP and WebSocket relay; verifier decoders |
| P7 | One end-to-end capture per harness, and termination | cli lifecycle suites and contract kit; daemon capture lifecycle and process kill |

## Releasing

The workspace `version` in `Cargo.toml` is the only place a version is written; the gate fails on
any other copy. The active repository ruleset protects `main`, and the release script never pushes
it directly:
`sh scripts/release.sh prepare <x.y.z>` opens a pull request that bumps the version (it does
nothing if `origin/main` already has it). After the version PR is merged, run the full gate on that
versioned `origin/main` commit and the separate `sh scripts/linux-check.sh`, then
`sh scripts/release.sh tag <x.y.z>` tags that commit and pushes
only the tag. The tag starts the release workflow, which
refuses a tag that differs from `Cargo.toml`. Details and how to try the build by hand:
[packaging/README.md](packaging/README.md#cutting-a-release).

## Run a capture

The reference behind the quickstart. Every flag is in `goalcup <command> --help`; config keys are
in [OPERATING](docs/OPERATING.md#running-the-daemon).

- **Where things go.** A run's folder (including a local run's `workspace/`) is
  made in the folder you run `goalcup start` from, or beside the goal file for a headless harness.
  Its harness homes are isolated: connectors may keep a shared config home or shared `HOME`
  under the Goal Cup Proxy configuration directory for sign-in across serialized runs.
  The sealed trace is under the daemon's `artifact_root`. A run on the server works in a clone of
  the participant's GitHub repository instead.
- **What the harness sees.** Its environment is built from nothing. Of your shell only these reach
  it: `USER`, `LOGNAME` and `TMPDIR` for every harness, and for a chat-mode harness also `PATH`,
  `TERM`, `COLORTERM`, `LANG`, `SHELL` and every `LC_*` locale variable (a headless harness gets a
  fixed `PATH`). Never a provider key (D61): a harness keeps its own login in its home. Goal Cup Proxy
  never sets a model base URL. The only proxy the harness gets is the recording proxy, on
  loopback.
- <a id="the-daemon"></a>**The daemon.** `goalcup start` uses the daemon that answers on
  `$GOALCUP_CONFIG_DIR/control.json` and starts `goalcup-daemon --exit-when-idle` when none does. It
  is found beside `goalcup` or on `PATH`, runs in its own session with an environment built from
  nothing, and logs to `$GOALCUP_CONFIG_DIR/daemon.log`. It exits 30 seconds after the last run is
  sealed. If a run outlives its daemon, the next `status`, `entries` or `seal` starts a daemon that
  reopens it for status and seal only, and that daemon stays until `goalcup seal` finishes the run.
- **The server.** `goalcup login` approves Goal Cup Proxy in the browser. `goalcup start` then asks for
  approval of each run, clones the participant's repository, opens the run on the server and keeps
  the server's start receipt as `receipt.json`. While the run records, the CLI uploads the chain in
  chunks and sends heartbeats. Recording never waits on the network, and no server error stops the
  run or ends its upload, whatever the status: it is asked again with backoff, and a refusal that
  repeats is a `PROBLEM:` line in `goalcup status` (D64). Only a heartbeat answer that says `stop`
  or the run's own count to its deadline stops it. After the stop it seals and
  uploads the rest for a bounded wrap-up; `goalcup upload` finishes the job later. With no entry id it sends the unsent entries of the newest run; `goalcup upload <entry>` sends one entry. The
  deadline is 168 hours after the server's official start, on the server's clock: at the deadline
  the CLI stops the agent and seals. Wire contract: [docs/UPLOAD-API.md](docs/UPLOAD-API.md).
  Server code: [server/worker](server/worker/README.md).
- **Hermes** (`hermes-cli`) runs in chat mode from its own home: `goalcup prep` opens it there
  and its own setup keeps the provider, model and key. Goal Cup Proxy hands no harness a provider key
  (D61).

**`goalcup-verify --capture-integrity` exit codes:** 0 complete
and valid, 1 incomplete or not sealed, 2 invalid (including a torn last record), 3 no usable report.
Exit 0 says the record is complete and consistent. It is not a judged result.
`goalcup-verify --research-json "$TRACE_DIR" --output "$REPORT_PATH"` writes a private research
report; set those variables to an immutable trace directory and a new `.ndjson` path first.

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
