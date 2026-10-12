# goal-cup

How do I build, gate, install and run the tracker? For contributors and agents. Updated: 2026-10-12.

**Goal Cup** is an open-source tracker that records what a coding agent actually did during a run.
It captures the harness's own session files (the native leg) and its model traffic (the wire leg)
into one sealed, hash-linked chain that `goalcup-verify` checks offline. The aim is
**One Prompt. One Agent. 168 Hours.** ([docs/VISION.md](docs/VISION.md)); that sentence is the
goal, not a claim about this code.

## Status

Where the project stands, what is measured and what is open: [STATUS.md](STATUS.md). The ticket
list is [LEDGER.md](LEDGER.md).

v0.5.0 is published as source only, with no attached files: see the [v0.5.0
release](https://github.com/agentcommunity/goal-cup-tracer/releases/tag/v0.5.0) and [the v0.5.1
cleanup record](docs/qualification/v0.5.1-public-release-cleanup.md). There is no downloadable
binary, installer, staged bundle, registry publication, signature, notarization or qualification.
Built binaries print `0.1.0`: the package version is independent of source release tags. The source
has exactly three pinned schema-1 connector directories: `codex-cli`, `claude-code-cli` and
`hermes-cli` (capture-only). The separate private `m2-pilot` macOS packaging allowlist
([packaging/README.md](packaging/README.md)) selects only Codex and Claude; a source pin does not
prove package inclusion. Short rows are measured; long runs are not qualified (records:
[docs/qualification/README.md](docs/qualification/README.md)). `v1/trunk` and its `v0.1.0` tag are
private; the V1 code that v0.5.0 kept behind `v1-compat` was removed (D48) and lives on in the
`v0.5.0` tag. Terms: [docs/GLOSSARY.md](docs/GLOSSARY.md).

## Build

Prerequisites:

- **Rust 1.99 or later** (edition 2024, `let` chains). `rust-toolchain.toml` asks for `stable`.
- **A supported platform.** macOS is supported and is the only measured target. Linux is
  best-effort and untested: the workspace builds and a small CI smoke check runs there, but the
  daemon and CLI test suites are not run there (they spawn process trees and hang small machines)
  and a `goalcup start` capture is refused, because the chain's disk reservation is implemented
  only for macOS APFS. Windows is planned; its code stubs say "not yet supported" and
  `goalcup-platform` refuses to compile on it today.
- **A C compiler** (`ring` compiles C and assembly).
- **`shasum` or `sha256sum`**, because a test runs `scripts/pin-manifests.sh --print`, and
  **`python3`** if you change `connectors/` and regenerate the pins.

Plain `cargo build` works: cargo fetches the locked dependencies the first time. The optional
maintainer `--offline` convention is under [the gate](#the-gate); `cargo fmt` reads no registry
and takes no `--offline` flag. Never create a `.cargo/config.toml`.

```
cargo build
```

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
`test-hooks`-off library. The gate result of the current merge is recorded in
[STATUS.md](STATUS.md); this file carries no count. Ignored tests, the by-hand suites and the CI
smoke check are in [docs/OPERATING.md](docs/OPERATING.md#building-and-verifying).

The four commands gate the M2 capture product, which is the whole build: the V1/v4
transcript-and-hook path was removed (D48), so there is no second feature set to gate. The build
still parses the retained `manifest.toml` compatibility files in the Codex and Claude connector
directories at startup; removing that parser dependency is ticket OS-29, not completed work.

Narrower runs use the same runner, for example `cargo test -p goalcup-daemon`. Add
`--test <file>` for one integration file. **CLI tests spawn the sibling
`target/debug/goalcup-daemon`**, so run `cargo build --workspace` before a narrow
`-p goalcup-cli` run.

## Run from source or stage a private candidate

Nothing below is a `cargo run`: the commands are written as a participant types them, with the
binaries on `PATH`. The workspace builds `goalcup`, `goalcup-daemon`, `goalcup-verify` and
`goalcup-mcp` (`goalcup-devserver` is development-only).

```
cargo build --release
export PATH="$PWD/target/release:$PATH"
export GOALCUP_CONNECTORS_DIR="$PWD/connectors"
```

`GOALCUP_CONNECTORS_DIR` matters only for a binary copied off the build tree; the discovery order
is in [docs/OPERATING.md](docs/OPERATING.md#current-m2-operating-boundary). Connector directories
are pinned; edit one only with `sh scripts/pin-manifests.sh` in the same commit. The macOS staging
and assisted-install tooling in [packaging/README.md](packaging/README.md) is not a released
installer or staged artifact. The assisted install adds a persistent launchd user agent that
restarts after a crash and at every login; `uninstall-assisted.sh` removes it.

## Run a capture

A capture run records one headless harness session for one goal through the recording proxy. Every
run gets a fresh `HOME`; Codex and Claude use isolated shared `CODEX_HOME`/`CLAUDE_CONFIG_DIR`
(Claude's run home has the checked Keychains link), while Hermes gets a per-run `HERMES_HOME` seeded
only with declared files. It needs a running daemon (`goalcup-daemon --config <FILE>`), and
`GOALCUP_CONFIG_DIR` must name the directory the daemon's `config_dir` names. A LocalOnly daemon
needs five keys and no server:

```toml
artifact_root    = "<install>/var/artifacts"
connectors_dir   = "<install>/share/goalcup/connectors"
config_dir       = "<install>/etc"
workspace_parent = "<install>/var/workspaces"
control_addr     = "127.0.0.1:0"
```

Omit `upload_url` and `server_public_key`: the daemon admits only local v5 entries (D48).
Every other key is in [OPERATING](docs/OPERATING.md#running-the-daemon).

Put the goal in a text file in a directory of its own, outside any repository: the run's
directory is created beside it. Then:

```
goalcup start --harness <codex-cli|claude-code-cli|hermes-cli> --goal-file <PATH>
goalcup-verify --capture-integrity <trace>
```

`start` launches the harness, waits for it, seals the entry and prints the harness exit code,
the termination, the `trace:` directory and the exact `verify:` line to run. Optional flags:
`--capture-reserve-bytes <BYTES>` (chain bytes reserved before launch) and
`--capture-claude-allowed-tool <RULE>` (one bounded Claude Bash rule for this launch). The
harness's environment is built from nothing: of the shell's own variables only `USER`,
`LOGNAME` and `TMPDIR` reach it, so an exported `ANTHROPIC_BASE_URL`, `OPENAI_BASE_URL` or
`*_PROXY` is dropped, not inherited (`start` does not refuse it). The tracker never sets a base
URL; the only proxy the harness is given is the tracker's own loopback capture proxy, and a
connector cannot add a `*_URL` name through `[capture.env]` or `[home.env]`.

**Integrity exit codes** of `goalcup-verify --capture-integrity`: 0 complete and valid, 1
incomplete or unsealed, 2 invalid (a torn tail included), 3 no usable report. A LocalOnly seal
with exit 0 means local evidence is complete; it is not an anchored or qualified result. The
research report is `goalcup-verify --research-json <trace> --output <REPORT.ndjson>`
([docs/research/m2-report-format.md](docs/research/m2-report-format.md)).

**Hermes** needs a provider key, which only the owner loads, through the reviewed launch seam
and never pasted into a shell:

```
/bin/sh -p scripts/with-deepseek-key.sh <ENV_FILE> start --harness hermes-cli --goal-file <PATH>
```

`<ENV_FILE>` is a mode 0600 file holding one `DEEPSEEK_API_KEY=` line. The script runs exactly
that command and refuses any other. This run is owner-run and the agent never opens the key file
(D41). The procedure and the measured row are in
[docs/qualification/m2-hermes-reference-row.md](docs/qualification/m2-hermes-reference-row.md).

## More

| For | Read |
|---|---|
| How the parts fit | [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) |
| Daemon config, every CLI flag and exit code, MCP server, verifier, fixtures, pins | [docs/OPERATING.md](docs/OPERATING.md) |
| Adding a harness | [docs/HARNESS-ONBOARDING.md](docs/HARNESS-ONBOARDING.md) |
| Which record proves what | [docs/qualification/README.md](docs/qualification/README.md) |
| What a participant agrees to | [docs/RULES.md](docs/RULES.md) |
| Settled decisions | [docs/DECISIONS.md](docs/DECISIONS.md) |
| Working here (agents and people) | [AGENTS.md](AGENTS.md) |

`harnesses/` holds local, isolated harness installs for testing (git-ignored except
`install.sh` and its README). `harnesses/env.sh` does not exist in a fresh clone: it is generated
by `harnesses/install.sh`, which you need only for the spikes and the ignored real-harness
tests, never to build or run the gate. Source `harnesses/env.sh` before any real-harness work.

## Contributing

Harness connectors are welcome: start with [CONTRIBUTING.md](CONTRIBUTING.md) and
[docs/HARNESS-ONBOARDING.md](docs/HARNESS-ONBOARDING.md). No DCO sign-off is needed; a human
reads and answers for each PR. Security issues go to goalhack@agentcommunity.org, never a public
issue, or use GitHub private vulnerability reporting ([SECURITY.md](SECURITY.md)).
Everyone taking part follows the [Code of Conduct](CODE_OF_CONDUCT.md).

## License

Apache License 2.0 ([LICENSE](LICENSE), [NOTICE](NOTICE)). Copyright 2026 AgentCommunity. Notices
required by a binary distribution have not yet been collected and must be generated before the
first binary release.
