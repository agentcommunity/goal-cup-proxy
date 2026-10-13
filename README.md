# goal-cup

Goal Cup records what a coding agent does in one run: it launches the agent on your goal, records
its model traffic and its own session files, and seals them into one hash-chained trace that
`goalcup-verify` checks offline. It is the tracker for the /goal Cup ([docs/VISION.md](docs/VISION.md)).
Until the server signs acknowledgements of the record, it is local only: the chain catches a stray
edit or corruption, but whoever controls the machine can edit a record, recompute the hashes after
it and re-seal, and it still verifies ([checklist](docs/CHEAT-CHECKLIST.md)).

## Install

**With Homebrew** (macOS and Linux):

```sh
brew install agentcommunity/tap/goalcup
```

This installs the three binaries (`goalcup`, `goalcup-daemon`, `goalcup-verify`) and the pinned
connectors they read. Releases and the tap are described in
[packaging/README.md](packaging/README.md); until the first release is published, build from
source.

<a id="build"></a>**From source** (Rust 1.99 or later, [rustup](https://rustup.rs); on macOS the
Xcode command line tools). From the repository root, build and put the three binaries on your
`PATH`:

```sh
cargo build --release
export PATH="$PWD/target/release:$PATH"
```

## Quickstart

**You need** a Mac (Apple silicon, or Intel: no Intel run is measured yet; on Apple
silicon install the arm64 Goal Cup and harness, not an Intel build under Rosetta), or Linux
(x86_64 or arm64) with the trace folder on ext4 or XFS ([Linux](#linux)); Goal Cup [installed](#install); and a Claude or ChatGPT subscription.
Install Claude Code or Codex CLI (the recipes were measured at Claude Code 2.1.283 and Codex CLI
0.153.0). Builds are recorded, not pinned: `goalcup start` runs the build it finds on `PATH` and
the trace records its digests and the version it reports. Choose where the tracker keeps its state:

```sh
export GOALCUP_CONFIG_DIR="$HOME/goalcup"
```

There is no daemon to install or keep running: `goalcup start` starts `goalcup-daemon` for the
run, and the daemon exits once every run is sealed. The first `start` writes
`$GOALCUP_CONFIG_DIR/daemon.toml`, which keeps traces in `~/goalcup-artifacts`; edit it to move
them ([the daemon](#the-daemon)).

Claude Code uses your normal Claude login. Codex signs in once to the tracker's own Codex home:
`mkdir -p -m 700 "$GOALCUP_CONFIG_DIR/capture-homes/codex-cli"`, then
`CODEX_HOME="$GOALCUP_CONFIG_DIR/capture-homes/codex-cli" codex login`.

Then run:

```sh
mkdir ~/goal-1 && echo "Build a to-do app in Python." > ~/goal-1/goal.txt
goalcup doctor --harness claude-code-cli
goalcup start --harness claude-code-cli --goal-file ~/goal-1/goal.txt
goalcup-verify --capture-integrity <trace>    # copy the verify: line start printed
```

`start` runs the agent until it exits, seals the run, and prints its `trace:` folder and the
`verify:` line. Meanwhile, from another terminal with the same `GOALCUP_CONFIG_DIR`,
`goalcup status` shows how it is doing and `goalcup seal` stops it early. Claude Code and Codex
runs share one tracker home per harness, so a second `start` of the same harness while one is
recording is refused; run them one at a time.
Verify exit 0 means the record is complete and valid ([all exit codes](#run-a-capture)). What you
agree to as a participant: [docs/RULES.md](docs/RULES.md).

**Adding a harness?** Start at [skills/adding-a-capture-harness](skills/adding-a-capture-harness/SKILL.md).

Status: v0.5.0 is published as source; binary releases come from
[release.yml](.github/workflows/release.yml) once a version is tagged. Open work:
[TODO.md](TODO.md). Terms: [docs/GLOSSARY.md](docs/GLOSSARY.md). Contributors running the tests also need `shasum` and
`python3`; never create a `.cargo/config.toml`.

## Linux

The [quickstart](#quickstart) works on Linux, apart from the three things below: how the harness
is installed, the configuration directory and the Claude Code login. The harness build is
recorded, not pinned, on Linux as on macOS. Linux is
measured less than macOS, and no logged-in harness run has happened on Linux yet; the lists below
say what was and was not measured.

**You need** Rust 1.99 or later, a C compiler (on Debian or Ubuntu, `build-essential`), and the
trace folder (`artifact_root` in `daemon.toml`) on ext4 or XFS. btrfs, ZFS, NFS, tmpfs, overlayfs
and FUSE are refused before anything runs, with this message:
`the trace folder must be on APFS (macOS) or ext4 or XFS (Linux); move artifact_root in daemon.toml`.
Check the folder, or its parent before the first run, with `stat -f -c %T ~/goalcup-artifacts`
(or `stat -f -c %T ~`): `xfs` is XFS, and `ext2/ext3` is what GNU `stat` prints for ext4. The
other tools the tests need are in [the gate](#the-gate).

**What differs from the macOS quickstart:**

- **Installing the harness on Linux.** What was measured is the native binary from the npm
  platform package, placed first on `PATH` (a symlink on `PATH` to it is fine). Codex:
  `@openai/codex@0.153.0-linux-x64` or `-linux-arm64`, file
  `package/vendor/<x86_64|aarch64>-unknown-linux-musl/bin/codex`. Claude Code:
  `@anthropic-ai/claude-code-linux-x64@2.1.283` or `-linux-arm64`, file `package/claude`; these
  are the glibc builds; the `-musl` (Alpine) builds were not measured. Other install routes are
  not measured: `npm i -g` puts the npm wrapper on `PATH`, which `goalcup start` then runs and
  records in place of the binary, and Claude Code's native installer on Linux was not tried.
- **The configuration directory.** With `GOALCUP_CONFIG_DIR` unset it is `$XDG_CONFIG_HOME/goalcup`,
  else `~/.config/goalcup`; an empty `$XDG_CONFIG_HOME` counts as unset
  (`crates/goalcup-cli/src/config.rs`). The quickstart sets `GOALCUP_CONFIG_DIR` and skips this.
- **The Claude Code login.** Linux has no Keychain, and the Claude Code
  connector links only the macOS Keychains directory into the run. Its login is expected in its
  config directory, so sign in once to the tracker's own:
  `mkdir -p -m 700 "$GOALCUP_CONFIG_DIR/capture-homes/claude-code-cli"`, then
  `CLAUDE_CONFIG_DIR="$GOALCUP_CONFIG_DIR/capture-homes/claude-code-cli" claude`.
  **This is not yet measured:** where Claude Code 2.1.283 keeps its login on Linux has not been
  checked, and nor has the Codex login location (the Codex command above is unchanged).

**Measured, in Docker** (rust:1.99, Linux 7.0.12, arm64): the full workspace builds and the fast
gate passes; the reservation reserves on ext4 and XFS and refuses overlayfs, tmpfs and FUSE; the
six synthetic contract-kit captures seal and verify. `sh scripts/linux-check.sh` reproduces this
from a Mac ([what it runs](#the-gate)). On x86_64, CI on ubuntu-24.04 runs the reservation tests on
ext4 and they pass; the end-to-end suites were run only on arm64.

With the arm64 builds and no login, measured in Docker (when the connectors still pinned
builds): `goalcup doctor` passes the executable check; `goalcup start` launches the binary;
the harness fails on auth and gets no model reply (Codex receives 401 Unauthorized, Claude Code
prints `Not logged in`, and both exit 1); the run seals; and
`goalcup-verify --capture-integrity` exits 0. Neither binary left a child running after it
exited on this path. The smoke needed network access: before it creates anything, `goalcup start`
checks that the provider host answers, and with no network it refuses the run.

The same no-login path was measured with the x86_64 builds on a GitHub-hosted ubuntu-24.04 runner
(the `harness-smoke` job of the manual `linux-x86_64` workflow, `scripts/linux-harness-smoke.sh`,
2026-10-13): both binaries run, stop at auth and exit 1, both runs seal, verify exits 0, and no
harness process is left ([facts sheets](docs/harnesses/README.md)).

**Not proven:**

- Termination of an orphan the process walk cannot see, such as a wrapper that forks a child and
  leaves, by the daemon alone. On Linux the daemon's `Confirmed` can still be claimed while such
  an orphan runs. `goalcup start` is a child subreaper there and sweeps what the harness left
  behind after it exits and before the seal; measured in Docker on a synthetic harness only. A
  process still running at the sweep's bound is named on stderr and `goalcup start` exits 1
  after the seal. It sweeps nothing if `goalcup start` itself dies, and a `goalcup seal` from
  another terminal can report `all_gone` while the sweep is still running
  ([TERMINATION.md](docs/TERMINATION.md#ruled-on-linux-confirmed-excludes-fork-and-leave-wrappers)).
- A successful, logged-in harness run on Linux, on either architecture.
- The end-to-end suites on x86_64: on the 4-vCPU runner `linux-check.sh` stops in its fast-gate
  step on two timing-sensitive tests. Any real Linux host or distribution kernel (only Docker
  Desktop's and the hosted runner's were run).
- Long runs, and termination against a real harness's process tree on Linux.

## Windows: use WSL2

**Not yet tested on a real Windows machine.** Run the tracker inside WSL2, as Linux:

1. In PowerShell: `wsl --install -d Ubuntu`.
2. Inside Ubuntu, install Rust, the build tools and the harness as in [Linux](#linux), then build
   and run the tracker there.
3. Keep the goal folder, the run folder and `artifact_root` under the Linux filesystem (`~/...`),
   never under `/mnt/c`: that is a Windows drive (drvfs), and `artifact_root` there is refused by
   the filesystem check. Keep the goal and run folders on the Linux filesystem too. WSL2's own root
   is ext4.

Native Windows is planned, not built.

## The gate

One script, two tiers. The last line it prints is `gate <tier>: PASS in <N>s` or
`gate <tier>: FAIL`. See [AGENTS.md](AGENTS.md) for the rules that bind every contributor.

```
sh scripts/gate.sh fast   # every change, before it is committed
sh scripts/gate.sh full   # before a release
```

Once per machine:

```
cargo install cargo-nextest --locked
cargo fetch --locked
export CARGO_TARGET_DIR=$HOME/.cache/goalcup-target   # optional: one build shared by every worktree
```

Share one target directory only when gates in different worktrees never run at the same time:
the CLI tests run the `target/debug` binaries by path, so a concurrent gate would test another
worktree's build.

The gate runs cargo with `--offline`, which is why the one `cargo fetch --locked` comes first;
`--offline` on an empty cache fails.

- **fast** runs `cargo build --offline --workspace`, `cargo fmt --all --check`,
  `cargo clippy --offline --workspace --all-targets -- -D warnings`, then every test under
  cargo-nextest (one process per test, in parallel) except the slow tier, then the doctests.
  It aims at under a minute warm.
- **full** is the release check. It runs everything fast runs plus the slow tier, the CLI's
  `#[ignore]`d end-to-end tests (`m2_synthetic_vertical`, `capture_failure_campaign`), the scripts'
  unit tests (`python3 -m unittest discover -s scripts`) and the link check
  (`python3 scripts/check-links.py --root .`). Nothing else runs the slow tier, CI included, so
  run `full` before every tag or it rots unseen.
- Without cargo-nextest the gate says so and falls back to `cargo test`, which runs every tier.

On Linux the gate needs `nc`, procps `ps` (`/bin/ps`; busybox `ps` is not supported),
`*.localhost` resolving to loopback (systemd-resolved or nss-myhostname), and `TMPDIR` on ext4
or XFS (capture refuses overlayfs, tmpfs, btrfs and FUSE). `sh scripts/linux-check.sh` runs the
Linux part in Docker, after the `cargo fetch --locked` above: the registry check, `gate.sh fast`,
the capture-reservation tests on ext4 and XFS, the CLI's `full` and `m2_cli_lifecycle` suites,
the `#[ignore]`d `m2_synthetic_vertical` and `capture_failure_campaign` suites, and the daemon's
`full` suite. With `LINUX_CHECK_HARNESSES=1` it then runs `scripts/linux-harness-smoke.sh`,
which fetches the measured harness builds for the machine's architecture from npm and launches
each with no login to the auth step (it needs network). It is not `gate.sh full`: the other
crates' `full` binaries, the scripts' unit tests and the link check are not run there. It has
been measured on Docker Desktop on an arm64 Mac only; on x86_64 it runs by hand in the
`linux-x86_64` workflow ([OPERATING](docs/OPERATING.md)), whose `harness-smoke` job has passed.

Each crate's integration tests are modules of two binaries: `tests/it/` (fast) and `tests/full/`
(slow). The slow tier is every `full` binary plus the CLI's standalone `m2_cli_lifecycle`
(about 100 s). A few suites that change process state (umask, environment, the panic hook) stay
standalone binaries in `tests/`. Suites that serialise through an in-process lock are put in
one-at-a-time test groups in `.config/nextest.toml`.

`cargo build` is step zero and is not redundant: the other steps build dev-dependencies, which
turn on `goalcup-daemon`'s `test-hooks` feature, so none of them compiles the shipped,
`test-hooks`-off library. **CLI tests also spawn the sibling `target/debug/goalcup-daemon`**, so
a stale daemon would be tested without it. This file carries no test count. Ignored tests, the
by-hand suites and the CI smoke check are in
[docs/OPERATING.md](docs/OPERATING.md#building-and-verifying).

The gate covers the M2 capture product, which is the whole build: the V1/v4
transcript-and-hook path was removed (D48), so there is no second feature set to gate. Every
connector loads from its `harness.toml` alone; the V1 `manifest.toml` files and their startup
parser were removed (OS-29).

Narrower runs: `cargo nextest run --offline -p goalcup-daemon`, or one module with
`cargo test --offline -p goalcup-daemon --test it control_api::`. Run
`cargo build --offline --workspace` before a narrow `-p goalcup-cli` run.

### The promises

Tests guard what a participant relies on. A test guarding one of these is never deleted; it may
only move tier.

| | Promise | Where the tests are |
|---|---|---|
| P1 | No key fragment leaks | cli key scan, stderr tee, child-secret suites; daemon redaction suites |
| P2 | The chain catches a stray edit (an edit with recomputed hashes re-seals; see the checklist) | protocol chain golden and round trips; verifier capture integrity, v5 corpus, containment |
| P3 | Capture policy refuses endpoints, proxies, credentials and bad args | protocol harness schema and conformance; cli launch plan, env resolution, dependency bans |
| P4 | Connectors load and pins match | protocol connector pins and loader; cli shipped connectors |
| P5 | The control API authenticates | daemon control API |
| P6 | The proxy captures faithfully | cli TLS, HTTP and WebSocket relay; verifier decoders |
| P7 | One end-to-end capture per harness, and termination | cli lifecycle suites and contract kit; daemon capture lifecycle and process kill |

## Run a capture

The reference behind the quickstart. Every flag is in `goalcup <command> --help`; the daemon's
other config keys are in [OPERATING](docs/OPERATING.md#running-the-daemon).

- **Where things go.** The run's folder (`workspace/`, the harness's fresh `HOME`) is created
  beside the goal file, so keep the goal file in an empty folder outside any repository. The
  sealed trace is under the daemon's `artifact_root`.
- **What the harness sees.** Its environment is built from nothing: of your shell's variables only
  `USER`, `LOGNAME`, `TMPDIR` and, for a keyed connector, its one API key reach it. An exported
  `ANTHROPIC_BASE_URL`, `OPENAI_BASE_URL` or `*_PROXY` is dropped, not inherited. The tracker
  never sets a model base URL; the only proxy the harness gets is the tracker's own recorder on
  your machine.
- <a id="the-daemon"></a>**The daemon.** `goalcup start` and `goalcup seal` use the daemon that
  answers on `$GOALCUP_CONFIG_DIR/control.json`, and start one when none does:
  `goalcup-daemon --config $GOALCUP_CONFIG_DIR/daemon.toml --exit-when-idle`, found beside
  `goalcup` or on `PATH`, in a session of its own (closing the terminal does not stop it), with
  an environment built from nothing (`HOME`, `USER`, `LOGNAME`, `TMPDIR` and a system `PATH`;
  none of your API keys or proxy variables), and its output in
  `$GOALCUP_CONFIG_DIR/daemon.log`. It stays up while any run is unsealed and exits 30 seconds
  after the last one is sealed. `goalcup status` and `goalcup entries` start one too from
  `daemon.toml` and read its registry (sealed runs included): there is no status copy on disk.
  With no run on record they answer without starting anything. If a run outlives its daemon (a
  crash, a reboot), the daemon any of these commands starts reopens the run for status and seal
  only, a run that had launched carries the lost interval as one gap, and that daemon **stays up
  until `goalcup seal`** finishes the run rather than exiting after 30 seconds. A run whose
  `goalcup start` died before the launch, with nothing handed off, is abandoned after 15 minutes. While `daemon.toml` is the
  unedited default, `start` keeps its `connectors_dir` naming the connectors it loaded (binaries
  move); an edited file that names other connectors is refused, naming the file. A
  `daemon.toml` you wrote for an earlier build belongs at `$GOALCUP_CONFIG_DIR/daemon.toml`;
  otherwise the first `start` writes a default with other roots. The configuration's keys are in
  [OPERATING](docs/OPERATING.md#running-the-daemon).
- **A binary copied out of `target/release`** cannot find the connectors on its own: export
  `GOALCUP_CONNECTORS_DIR="<checkout>/connectors"`. An installed one (Homebrew, or an unpacked
  release tarball) finds them at `<bin>/../share/goalcup/connectors`.
- **Hermes** (`hermes-cli`) needs a provider key and is run by the maintainer only, through
  `/bin/sh -p scripts/with-deepseek-key.sh <ENV_FILE> start --harness hermes-cli --goal-file <PATH>`
  with a mode 0600 file holding one `DEEPSEEK_API_KEY=` line. Never paste the key into a shell.

**Exit codes of `goalcup-verify --capture-integrity`:** 0 complete and valid, 1 incomplete or not
sealed, 2 invalid (including a torn last record), 3 no usable report. Exit 0 says the local record
is complete and consistent; it is not a judged or server-confirmed result, and a local-only record
can be rewritten and re-sealed so that it still exits 0. `goalcup-verify --research-json <trace>
--output <REPORT.ndjson>` writes the private research report.

## More

| For | Read |
|---|---|
| How the parts fit | [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) |
| Daemon config, every CLI flag and exit code, verifier, fixtures, pins | [docs/OPERATING.md](docs/OPERATING.md) |
| Adding a harness | [skills/adding-a-capture-harness](skills/adding-a-capture-harness/SKILL.md), then [docs/HARNESS-ONBOARDING.md](docs/HARNESS-ONBOARDING.md) |
| What is next | [TODO.md](TODO.md) |
| What a participant agrees to | [docs/RULES.md](docs/RULES.md) |
| Settled decisions | [docs/DECISIONS.md](docs/DECISIONS.md) |
| Working here (agents and people) | [AGENTS.md](AGENTS.md) |

`harnesses/` holds local, isolated harness installs for testing (git-ignored except
`install.sh` and its README). `harnesses/env.sh` does not exist in a fresh clone: it is generated
by `harnesses/install.sh`, which you need only for the spikes and the ignored real-harness
tests, never to build or run the gate. Source `harnesses/env.sh` before any real-harness work.

## Contributing

Harness connectors are welcome: start with [CONTRIBUTING.md](CONTRIBUTING.md) and
[skills/adding-a-capture-harness](skills/adding-a-capture-harness/SKILL.md). No DCO sign-off is
needed; a human reads and answers for each PR. Security issues go to goalhack@agentcommunity.org,
never a public issue, or use GitHub private vulnerability reporting ([SECURITY.md](SECURITY.md)).
Everyone taking part follows the [Code of Conduct](CODE_OF_CONDUCT.md).

## License

Apache License 2.0 ([LICENSE](LICENSE), [NOTICE](NOTICE)). Copyright 2026 AgentCommunity. Notices
required by a binary distribution have not yet been collected and must be generated before the
first binary release.
