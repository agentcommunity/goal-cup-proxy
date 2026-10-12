# goal-cup

Goal Cup records what a coding agent does in one run: it launches the agent on your goal, records
its model traffic and its own session files, and seals them into one tamper-evident trace that
`goalcup-verify` checks offline. It is the tracker for the /goal Cup ([docs/VISION.md](docs/VISION.md)).

## Quickstart

**You need** a Mac with Apple silicon, Rust 1.99 or later ([rustup](https://rustup.rs)), the Xcode
command line tools, and a Claude or ChatGPT subscription. Install the exact harness build Goal Cup
is pinned to, Claude Code 2.1.283 or Codex CLI 0.153.0: `goalcup start` refuses any other build.

<a id="build"></a>From the repository root, build, configure and start the daemon:

```sh
cargo build --release
export PATH="$PWD/target/release:$PATH"
mkdir -p ~/goalcup && cat > ~/goalcup/daemon.toml <<EOF
artifact_root    = "$HOME/goalcup/artifacts"
connectors_dir   = "$PWD/connectors"
config_dir       = "$HOME/goalcup/config"
workspace_parent = "$HOME/goalcup/runs"
control_addr     = "127.0.0.1:0"
EOF
export GOALCUP_CONFIG_DIR="$HOME/goalcup/config"
goalcup-daemon --config ~/goalcup/daemon.toml    # leave it running
```

Claude Code uses your normal Claude login. Codex signs in once to the tracker's own Codex home:
`mkdir -p -m 700 "$GOALCUP_CONFIG_DIR/capture-homes/codex-cli"`, then
`CODEX_HOME="$GOALCUP_CONFIG_DIR/capture-homes/codex-cli" codex login`.

In a second terminal, export the same `PATH` and `GOALCUP_CONFIG_DIR`, then run:

```sh
mkdir ~/goal-1 && echo "Build a to-do app in Python." > ~/goal-1/goal.txt
goalcup doctor --harness claude-code-cli
goalcup start --harness claude-code-cli --goal-file ~/goal-1/goal.txt
goalcup-verify --capture-integrity <trace>    # copy the verify: line start printed
```

`start` runs the agent until it exits, seals the run, and prints its `trace:` folder and the
`verify:` line. Meanwhile `goalcup status` shows how it is doing and `goalcup seal` stops it early.
Verify exit 0 means the record is complete and valid ([all exit codes](#run-a-capture)). What you
agree to as a participant: [docs/RULES.md](docs/RULES.md).

**Adding a harness?** Start at [skills/adding-a-capture-harness](skills/adding-a-capture-harness/SKILL.md).

Status: source only (v0.5.0); there is no binary release. Open work: [TODO.md](TODO.md). Terms:
[docs/GLOSSARY.md](docs/GLOSSARY.md). Contributors running the tests also need `shasum` and
`python3`; never create a `.cargo/config.toml`.

## The gate

A change passes these four commands, in this order, before it is committed. See
[AGENTS.md](AGENTS.md) for the rules that bind every contributor.

```
cargo build
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Maintainer variant: add `--offline` to the three cargo commands that take it, after one
`cargo fetch --locked`, so that nothing can fetch; `--offline` on an empty cache fails.

`cargo build` is step zero and is not redundant: the other three build dev-dependencies, which
turn on `goalcup-daemon`'s `test-hooks` feature, so none of them compiles the shipped,
`test-hooks`-off library. This file carries no test count. Ignored tests, the by-hand suites and the CI
smoke check are in [docs/OPERATING.md](docs/OPERATING.md#building-and-verifying).

The four commands gate the M2 capture product, which is the whole build: the V1/v4
transcript-and-hook path was removed (D48), so there is no second feature set to gate. The build
still parses the retained `manifest.toml` compatibility files in the Codex and Claude connector
directories at startup; removing that parser dependency is ticket OS-29, not completed work.

Narrower runs use the same runner, for example `cargo test -p goalcup-daemon`. Add
`--test <file>` for one integration file. **CLI tests spawn the sibling
`target/debug/goalcup-daemon`**, so run `cargo build --workspace` before a narrow
`-p goalcup-cli` run.

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
- **A binary copied out of `target/release`** cannot find the connectors on its own: export
  `GOALCUP_CONNECTORS_DIR="<checkout>/connectors"`. The macOS staging scripts in
  [packaging/README.md](packaging/README.md) are private tooling, not a released installer.
- **Hermes** (`hermes-cli`) needs a provider key and is run by the maintainer only, through
  `/bin/sh -p scripts/with-deepseek-key.sh <ENV_FILE> start --harness hermes-cli --goal-file <PATH>`
  with a mode 0600 file holding one `DEEPSEEK_API_KEY=` line. Never paste the key into a shell.

**Exit codes of `goalcup-verify --capture-integrity`:** 0 complete and valid, 1 incomplete or not
sealed, 2 invalid (including a torn last record), 3 no usable report. Exit 0 says the local record
is complete; it is not a judged or server-confirmed result. `goalcup-verify --research-json <trace>
--output <REPORT.ndjson>` writes the private research report.

## More

| For | Read |
|---|---|
| How the parts fit | [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) |
| Daemon config, every CLI flag and exit code, MCP server, verifier, fixtures, pins | [docs/OPERATING.md](docs/OPERATING.md) |
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
