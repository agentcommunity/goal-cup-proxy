# Operating the tracker

Updated: 2026-10-12

How do I build, configure and run the binaries in full detail: the daemon config, the CLI flags
and exit codes, the MCP server, the verifier, the committed traces,
fixtures and connector pins? For participants and contributors. The short path (build, gate,
install, one capture run) is in [../README.md](../README.md).

## Contents

- [Building and verifying](#building-and-verifying): prerequisites, the gate, ignored tests, [CI](#ci)
- [Installing the participant binaries](#installing-the-participant-binaries)
- [Running the daemon](#running-the-daemon)
- [Running a capture with the CLI](#running-a-capture-with-the-cli)
- [The MCP server, `goalcup-mcp`](#the-mcp-server-goalcup-mcp)
- [Running the verifier](#running-the-verifier)
- [The committed traces](#the-committed-traces)
- [Regenerating fixtures](#regenerating-fixtures)
- [Pinning connectors](#pinning-connectors)

## Current M2 operating boundary

v0.5.0 is published as source only
(cleanup record). No downloadable binary,
installer, staged bundle, registry publication, signature, notarization or qualification exists.
Built binaries print `0.1.0`: the package version is independent of source release tags. The source
contains exactly three pinned schema-1 connector directories: `codex-cli`, `claude-code-cli` and
`hermes-cli`. The separate private macOS packaging allowlist (`m2-pilot` in
[packaging/README.md](../packaging/README.md)) stages only Codex and Claude, copying each selected
directory whole, including seeds and its retained V1 `manifest.toml`. A source pin is not
evidence of package inclusion. macOS is the only measured platform. Linux builds, but a
`goalcup start` capture is refused there: the chain's disk reservation is implemented only for
macOS APFS.

Connector discovery checks `$GOALCUP_CONNECTORS_DIR`, `<exe>/../share/goalcup/connectors`, then
`<exe>/../../connectors`. It considers immediate child directories with a regular `harness.toml`,
validates every discovered schema-1 profile, then selects by declared id. Every run gets a fresh
`HOME`: Codex and Claude use shared isolated config homes (Claude links the checked Keychains
directory); Hermes gets a seeded per-run `HERMES_HOME`.

Control API v5 requires the version on mutating requests and binds creation to the daemon's loaded
connector-directory digest. Pre-allocation refusals happen before daemon entry/artifact/workspace
writes, but CLI run/home/CA/seed preparation may already exist and is cleaned. After restart an
unfinished v5 capture has status/seal-only authority; other authenticated mutators refuse before
side effects, and an unfinished launched capture receives one `IngestLost` gap.

The V1 execution path was removed (D48; the `v0.5.0` tag keeps it). The build still parses the
retained Codex and Claude `manifest.toml` files at startup. Full removal is ticket OS-29 and is not
complete. The schema-1 synthetic contract kit covers exactly six cases: `controlled-workload` (no
identity, append-only), `synthetic-preassigned` (launch identity, append-only),
`synthetic-stderr-token` (deferred identity, append-only), `synthetic-keyed-stderr` (the same
with an invented child-only `DEEPSEEK_API_KEY` and a clean key scan), `synthetic-stdout-json` (launch
identity, two snapshot generations), and `synthetic-keyed-stdout-json` (the same with an
invented child-only key, its identity read from the stdout the key scan reads). All require `--dev`
and exclude a wrong-id decoy where applicable.

## Building and verifying

Prerequisites:

- **Rust 1.99 or later.** `Cargo.toml` sets `rust-version = "1.99"`, the latest stable release
  when v0.5.1 was cut. The code itself needs less: edition 2024 needs 1.85, it uses `let` chains,
  and the locked dependencies declare at most 1.88. Development and CI use 1.99.0.
  `rust-toolchain.toml` asks for `stable`.
- **macOS or Linux.** `goalcup-platform` refuses to compile on anything else
  (`crates/goalcup-platform/src/lib.rs`). The V1 claim is one macOS target; Linux is a
  smoke check (see CI below).
- **A C compiler.** `ring` compiles C and assembly.
- **`shasum` or `sha256sum`**, because the test suite runs `scripts/pin-manifests.sh --print`
  (`crates/goalcup-verifier/tests/it/checks_v13.rs`), and **`python3`** if you change
  `connectors/` and regenerate the pins (`scripts/pin-manifests.sh`).

`cargo test --workspace` is the whole suite, every crate at once. "Green" means
`sh scripts/gate.sh fast` passes (and `full` before a release), not the suite alone. One
crate at a time, or one module of one crate, is the same runner narrowed:

```
cargo test -p goalcup-daemon                       # one crate
cargo test -p goalcup-daemon --test it control_api::  # one integration test module
```

On the maintainer's machine every cargo command carries `--offline`, so none of them can
fetch: `cargo fetch --locked` runs once first, and every later command works from the cache.
Add the flag to any command in this file for the same guarantee.

One narrowing does not work from a clean tree: the CLI tests drive the shipped
`goalcup-daemon` and `goalcup-verify` binaries, and only `--workspace` builds them. They say
so, by name, rather than skipping.

The gate a change has to pass before it is committed is `sh scripts/gate.sh fast`; a release
passes `sh scripts/gate.sh full` ([README, The gate](../README.md#the-gate) has the tiers). Both
run, in this order:

```
cargo build --offline --workspace
cargo fmt --all --check
cargo clippy --offline --workspace --all-targets -- -D warnings
cargo nextest run --offline --workspace   # fast: -E 'not (binary(full) | binary(m2_cli_lifecycle))'
cargo test --offline --workspace --doc
```

`cargo build` is step zero and it is not redundant. The other steps all build
dev-dependencies, and `goalcup-daemon` depends on itself in its dev list with the
`test-hooks` feature on — cargo unifies features within a package, so **the shipped,
`test-hooks`-off build of the library is compiled by none of them**. A `#[cfg(not(feature =
"test-hooks"))]` mistake, or a non-gated reference to a gated field, passes the last three
and breaks the binary that ships.

The gate (and `cargo test --workspace`) skips every `#[ignore]` test (child halves, fixture regenerators,
real-process and live-harness campaigns, vertical suites). List them with
`rg '^\s*#\[ignore' crates` and run one named suite at a time with its documented setup. The
default gate is not proof that ignored coverage ran.

The D3 dependency check, `the_structural_bans_hold_at_the_dependency_level`, lives in
`dependency_bans.rs` and runs in the default gate.

The synthetic vertical suite also runs by hand after building the workspace binaries:

```sh
cargo test -p goalcup-cli --offline --features test-support --test full m2_synthetic_vertical:: -- --ignored --test-threads=1
```

`--features test-support` is written out because `tests/common/support/mod.rs` needs it, even though
the crate's own dev-list already turns it on.

Nothing in the suite reaches the network. The one listener the daemon serves on `m2/wire`, the
control API, is refused a non-loopback address by `DaemonConfig::validate`. (On `v1/trunk` the hook receiver is a
second listener under the same rule.)

### CI

`.github/workflows/ci.yml` is a Linux smoke check, **not the gate**. Its one job, `check`
(workflow `ci`), runs on `ubuntu-24.04` (20-minute timeout) for every pull request into and push
to `main` or `v1/trunk`, and manual starts. No path filter, so branch protection can require it.

The `scope` step runs `scripts/ci-scope.py` on the event's two commits. It gives `rust=false`
only when every changed path is Markdown under `docs/`, at the root, or the PR template; any
other change, a new branch, a manual start or a failed comparison gives `rust=true`. Every run
runs the `test_ci_scope.py` and `test_check_links.py` suites and `scripts/check-links.py --root
.`. With `rust=true` it also runs build, fmt, clippy `-D warnings` and `cargo test --workspace
--exclude goalcup-daemon --exclude goalcup-cli`, in that order. Any failing step fails the job.

Every runner minute costs something on every pull request. Keep this file small:
- Do not add branches, jobs, a build matrix, or the daemon and CLI suites. Those suites spawn
  process trees that hang or kill a 2-core Linux runner, and Linux is best-effort, not a claim.
- A green `ci` run proves nothing about the macOS target. `sh scripts/gate.sh fast` above (and `full` before a
  release), plus the ignored synthetic vertical suite, runs by hand before a merge.

### Installing the participant binaries

Nothing below is a `cargo run`. Every command in this file is written as a participant types
it, which means `goalcup`, `goalcup-daemon` and `goalcup-verify` are on
`PATH` — and a clean checkout has none of them there. One build makes the workspace binaries:

```
cargo build --release
```

Then either put that directory in front of `PATH` for the session:

```
export PATH="$PWD/target/release:$PATH"
```

— or copy the four binaries somewhere already on it, and **set `$GOALCUP_CONNECTORS_DIR` to
this checkout's `connectors/`**:

```
export GOALCUP_CONNECTORS_DIR="$PWD/connectors"
```

That second variable is not optional for a copied binary, and it is the one thing about
installing this tree that surprises people. The connector manifests are the **rulebook** —
the human rule, the exclusion catalogue, the tool classes, with their digests pinned in the
verifier — so they live beside the binary and never under a directory the participant
writes. `goalcup` looks for them at `$GOALCUP_CONNECTORS_DIR`, then at
`<exe dir>/../share/goalcup/connectors` for an installed layout, then at
`<exe dir>/../../connectors`, which is what makes `target/release/goalcup` find this
repository's `connectors/` with nothing exported at all. A binary copied to `~/bin` matches
none of the three and refuses, naming the variable. `cargo install` lands in the same place
for the same reason.

`$GOALCUP_CONFIG_DIR` is the third, and it must agree with the daemon (next section).

### Running the daemon

The daemon takes one required argument, a configuration file, and runs in the foreground
under whatever started it. There is no daemonisation, no pidfile and no logger; what it
would have logged is counted on the signed status document instead.

```
goalcup-daemon --config <FILE> [--foreground]
goalcup-daemon --config <FILE> --print-addresses
goalcup-daemon --version
```

A minimal LocalOnly configuration, with every field that has no default. Paths must be absolute:

```toml
# The registry, every entry's artifact directory and trace: the ONLY tree the daemon writes
# outside a workspace. BESIDE `config_dir`, never inside it: the CLI and MCP read that one.
artifact_root = "/Users/you/goalcup-artifacts"
# Schema-1 connector directories. In a checkout, this repository's `connectors/`.
connectors_dir = "/Users/you/src/goal-cup/connectors"
# `control.json` and the status-cache mirrors. It MUST be the directory the CLI looks in (the
# CLI finds the daemon only through `control.json`): `~/Library/Application Support/goalcup`
# on macOS, `$XDG_CONFIG_HOME/goalcup` (else `~/.config/goalcup`) elsewhere. Never a harness's.
config_dir = "/Users/you/Library/Application Support/goalcup"
# Default workspace parent. Never nested with `artifact_root`: an agent lists its workspace's
# parent on its first tool call, so nesting would show it the run's evidence (R3).
workspace_parent = "/Users/you/goalcup-runs"
# The control API's listen address. Loopback, or the daemon refuses to start.
control_addr = "127.0.0.1:7787"
```

`upload_url` and `server_public_key` are still parsed, both or neither, so a V1 configuration
keeps loading. Nothing in this tree uploads or checks a receipt (D48), so the pair changes only
one thing: when `server_public_key` is the published placeholder (the verifying key of the
published seed `[5u8; 32]`), the daemon prints one stderr line saying its seed is
public. Omit both.

`$GOALCUP_CONFIG_DIR` overrides **both sides at once**: exported, the CLI and `goalcup-mcp`
read that directory instead of the platform default, and a daemon started with the same value
in `config_dir` writes its `control.json` there. That is the whole of how a second, isolated
tracker is run on one machine — a test, a second round, a development build beside an
installed one — and it is why nothing in this tree needs to move `$HOME` to do it. Set one
without the other and the CLI looks in one directory while the daemon writes to another, and
every command answers "no daemon is running".

With no `[event]` section the signed status document reports the entry's own timeline in the
competition's fields and marks it `event_status_source: "fallback"`, so a reader can tell a
date the organiser set from one standing in for it.

The daemon prints two lines on stdout and nothing else: the control address and the path of
the `control.json` it wrote. `--print-addresses` prints the same two from that file **without
starting anything**: it opens no registry, so it works while a daemon is up and holding the
lock, which is exactly when it is worth running.
The bearer in that file is read and never printed.

| Exit | Meaning |
|---|---|
| 0 | Stopped on SIGTERM or SIGINT. A signal ends the process; it does not seal the entry. After a restart an unfinished capture has status and seal authority only. Also `--print-addresses` when it read `control.json`, and `--version`. |
| 1 | Refused, in one sentence on `stderr` prefixed `goalcup-daemon:` and never a panic. The configuration file could not be read or did not validate; the daemon would not start (`control.json` among the things it writes); the async runtime or the signal handlers would not install; or `--print-addresses` found no `control.json`. Every refusal that is about a file names that file. |
| 2 | clap refused the arguments (`--config` missing, an unknown flag). |

### Running a capture with the CLI

With a daemon up (above), a capture run is one command, and the verify line it prints. Every
command reads `$GOALCUP_CONFIG_DIR`, or the platform default beside it, to find that daemon.

```
goalcup start --harness <id> --goal-file <PATH>   # launches, waits, seals, prints `verify:`
goalcup status                                     # what the signed document says
goalcup entries                                    # the runs this daemon holds
goalcup-verify --capture-integrity <trace-dir>     # the line `start` printed
```

Run the `verify:` line `start` printed, not one typed from memory. The integrity exit codes are
in the [README](../README.md#run-a-capture).

`goalcup doctor --harness <id>` scopes the report **and the exit code** to one connector.
Without it, `doctor` reports every shipped connector and exits 1 if any of them failed —
including one you have no intention of running, which is the honest answer to "is this
machine healthy" and the wrong one to "can I start this harness".

**The rest of the flags.** `goalcup <cmd> --help` carries each of them:

| Flag | What it does |
|---|---|
| `goalcup start --capture-reserve-bytes <BYTES>` | The chain bytes the daemon reserves on disk before the launch. |
| `goalcup start --capture-claude-allowed-tool <RULE>` | One bounded Claude Bash command rule for this launch only. |
| `goalcup seal --dry-run` | Walk this machine's workspace and print the self-check (what is there and what the exclusions take out) and seal nothing. It makes no call at all, so it answers on a machine whose daemon is not running. |
| `--json` on `status`, `entries` and `doctor` | The signed document itself, the summaries as one array, and the report as one document, instead of a page of text. |

**Network outages.** Before anything is created, `goalcup start` makes one TCP connect to port
443 of each provider host the connector intercepts, all within 10 s (loopback hosts are skipped).
A host that does not answer refuses the run, naming the host, the failure (DNS, refused, timed
out, not permitted) and this binary's path: a per-app firewall such as Little Snitch holds a
newly built binary's connections until someone allows that exact path. During a run, each
provider-host connect is retried, re-resolving the name, with a 250 ms to 2 s backoff until 25 s
of the 30 s admission deadline, and is never a replayed request; any other host (the agent's own
pip, npm, git or curl traffic) gets one round and fails at once, as before. Upstream sockets get TCP keepalive
(60 s idle, 15 s interval, 4 probes) and no idle timeout. Provider-host connects that still fail
coalesce into outage windows: the finish adds `upstream_outages=N` to the `proxy admission:` line
and one `proxy upstream outage:` line per window (first and last failure, count, recovery time).
The frozen v5 chain has no record kind for an outage, so the windows are not in the trace. Nothing
is said to the harness. The Claude connector sets `CLAUDE_CODE_RETRY_WATCHDOG=1` so a dropped
connection is retried for hours rather than about ten times; Codex 0.153.0 has no retry key that
does not require a custom provider with a base URL, so its budget stays its own.

`goalcup --help` lists five commands (`start`, `status`, `entries`, `seal`, `doctor`) and says
where verification is. There is no `goalcup verify`: one verifier, one set of flags, one
set of exit codes. `auth`, `ui` and `resume` were V1 commands; they are hidden and refuse at
rung 1, naming D48.

`goalcup`'s own exit codes, which `goalcup --help` also carries:

| Exit | Meaning |
|---|---|
| 0 | The command did what it was asked. |
| 1 | Refused, and you can fix it: no daemon running, a tracker still busy with the entry after the retry is spent, an entry that is already sealed, a mistyped sub-command, a preflight check that said no. |
| 2 | **Never this binary's.** It is a `Fail` verdict, and only `goalcup-verify` leaves under it. |
| 3 | The CLI could not do its job: a file it could not read, an answer it could not parse. |
| 4 | Not left by a capture run: it belonged to the removed V1 gates (D48). |

**What `--dev` means, on each of the two binaries that carry it.** Both say the same thing:
this is a development run, and its result is not a judged entry's.

| Flag | What it admits |
|---|---|
| `goalcup start --dev` | A connector directory this build does not pin: an edited manifest, a hand-written one, a connector under development. Without the flag that is a pre-allocation refusal before daemon entry/artifact/workspace creation; CLI preparation may already exist and is cleaned. The run says so on stdout and the entry record records it. |
| `goalcup-verify --dev <dir>` | See **Running the verifier** below: an unpinned rulebook becomes a Flag rather than a Fail, and the placeholder-server-key Flag is suppressed. |

None of them is a way to make a failing entry pass. `--dev` never suppresses a `chain_broken`,
and a verdict that used it says so in its own document.

### The MCP server, `goalcup-mcp`

The other half of M1c. It is the tracker's read-only status, offered to the agent under
measurement over MCP on stdio: three tools, no socket, and nothing it can change. It reads
the signed status documents this machine has cached and checks each one against the run key
`goalcup start` pinned in the entry record — never the key written inside the document being
checked — so an answer it gives is a fact the tracker signed, and a refusal is not a fact
about the run at all.

**It is shipped and never auto-registered.** A participant who wants their agent to have
these tools registers this binary themselves; nothing in `goalcup start` does it for them.
Registration is whatever the harness's own MCP configuration looks like, and the two things
it needs are the command and the configuration directory:

```
goalcup-mcp [--config-dir <DIR>]
```

`--config-dir` defaults to `$GOALCUP_CONFIG_DIR`, else the same platform default `goalcup`
uses — so a machine where the CLI already works needs no argument. In Claude Code's own
`.mcp.json` shape that is:

```json
{
  "mcpServers": {
    "goalcup": { "command": "goalcup-mcp", "args": [] }
  }
}
```

The three tools:

| Tool | Answers |
|---|---|
| `goalcup_event_status` | The round's timeline as the last signed document carries it: `t0`, when the start window closes, when publication closes, the rules version — and `source`, which says `config` when an organiser set those dates on this machine and `fallback` when the fields are this entry's own timeline standing in for a round nobody configured. |
| `goalcup_run_status` | How one run is doing: arming state, health, seconds remaining, when the tracker last synced, and whether that document is stale. Takes an optional `entry_id` and defaults to the most recently started run. `arming: "refused"` appears only in a status a pre-D48 (`v0.5.0`) daemon wrote; a capture entry has no arming gate (D48). |
| `goalcup_entries` | Every run this machine holds a **verified** status document for. `unverified` counts the cached documents that did not check out and are therefore not in the list. |

`crates/goalcup-mcp/SKILL.md` is the description an agent is given, and it carries the tool
manifest byte for byte — `crates/goalcup-mcp/tests/protocol.rs` compares the two, so a tool
renamed in one and not the other is a test failure rather than a skill describing a server
that is not running. It also says when NOT to call these tools, which is the half a skill
usually leaves out.

Exit codes are `0` on a clean end of input and `1` on a refusal, usage errors included —
`2` is a `Fail` verdict in this tree and is nobody else's number.

### Running the verifier

```
goalcup-verify <trace-dir> [--json <out>] [--dev] [--dev-key <hex>]
```

`<trace-dir>` is the directory holding `trace.toml`. Without `--json` the verdict JSON is
stdout and nothing else is; with `--json <path>` the verdict goes to the file and stdout
stays empty. Everything meant for a person — notices, errors, the summary line — is
stderr.

| Exit | Meaning |
|---|---|
| 0 | `Pass` |
| 1 | `Flag` |
| 2 | `Fail` |
| 3 | No verdict could be produced at all (the trace could not be read). A trace that could not be read is not a trace that failed. |

**A run whose harness never started is a `Fail`, not a 3.** Its trace names no
`tracked_sessions`, and the verifier reports that as a `no_session` Fail with a verdict
document, because "nothing was captured" is a real thing to say about a run and somebody has
to be able to file it. `goalcup seal` warns before it seals that this is what the verdict
will be, while stopping is still a choice.

`--dev` reports a connector manifest this build does not pin as a `Flag` instead of a
`Fail`. `--dev-key <hex>` (repeatable, 64 hex characters) accepts acknowledgement and
receipt signatures under that Ed25519 public key for one run; the verdict records
`key_source: "supplied"` so a development run can never be mistaken for a production one.

### The committed traces

| Trace directory | Exits |
|---|---|
| `crates/goalcup-verifier/tests/traces/claude-code-cli/` | 2 (`fail`) |
| `crates/goalcup-verifier/tests/traces/codex-cli/` | 2 (`fail`) |
| `crates/goalcup-verifier/tests/fixtures/adversarial/` | 2 (`fail`) |
| `crates/goalcup-verifier/tests/fixtures/compliant/` | 0 (`pass`) |

The two M0 traces are deliberately non-compliant scripts, so `fail` is the expected
result, not a defect — each has an `expected-verdict.json` beside it that the suite
compares against. The adversarial trace carries planted tamper evidence. The compliant
trace is the one entry that is clean end to end.

`v1/trunk` has a third M0 trace, `traces/opencode/`, which the `m2/wire` deletion pass removes
with the connector (`4039138`, which also deleted the live `traces/opencode-live/` capture).
On `m2/wire` the `codex-cli` trace still fails, and its findings also carry the interim
`<environment_context>` goal and one more V3 flag (wire-primary spec §11.2), labelled
`INTERIM` in its `expected-verdict.json`.

### Regenerating fixtures

Both generators are `#[ignore]`d **and** gated on `GOALCUP_REGEN=1`, so neither
`cargo test` nor `cargo test -- --include-ignored` can overwrite a committed fixture by
accident:

```
GOALCUP_REGEN=1 cargo test -p goalcup-verifier --test it -- --ignored adversarial::regenerate
GOALCUP_REGEN=1 cargo test -p goalcup-verifier --test it -- --ignored compliant::regenerate
```

Read the diff before committing it.

### Pinning connectors

```
sh scripts/pin-manifests.sh
```

It recomputes the schema-1 whole-directory pin table and the transitional V1-manifest table.
These are source integrity allowlists, not package manifests:

* `CONNECTOR_MANIFEST_SHA256S`, in `crates/goalcup-verifier/src/lib.rs` — exactly the retained
  Codex and Claude `manifest.toml` digests. This is what the V1 arming receipt claimed and what
  V13's `manifest_not_pinned` compares, because the `manifest.toml` a trace carries is the only
  byte of a connector a verifier can **recompute** after the fact.
* `CONNECTOR_DIR_SHA256S`, built from the data file `crates/goalcup-protocol/pins/connector-dirs.txt`
  by `crates/goalcup-protocol/src/pins.rs` (a malformed file does not compile) and re-exported
  from the verifier — exactly the Codex, Claude and Hermes whole-directory digests, with seeds and
  templates under `files/` included. The manifest digest cannot see an edited seed file, so the
  loader hashes each connector directory and refuses a pinned connector whose bytes moved, and a
  v5 entry binds the digest at create and in `Launch.connector_dir_sha256`. On a V1 (v4) trace,
  V13 reads the digest back from the start receipt the V1 daemon wrote (that daemon path was deleted
  by D48), and **only out of a receipt whose signature verified**; otherwise a participant could
  self-sign a receipt naming the pinned digest and buy silence: `connector_dir_unpinned` (Fail, and
  Pass-level under `--dev`) when the claim is not in this table, `connector_dir_unbound` (Flag) when
  a verified receipt carries no claim at all. A trace whose receipt did not verify, or that carries
  none, gets no directory finding: `receipt_invalid`, `receipt_unverified`, `receipt_missing` or
  `legacy_trace` is already the larger statement. `goalcup doctor` runs the CLI's own copy of the
  same comparison over the CLI's own installation.

Both tables use declared connector ids; schema 1 uses `harness.toml`, while the compatibility table
uses `manifest.toml`. Run it in the same commit as any change
under `connectors/`;
`sh scripts/pin-manifests.sh && git diff --exit-code` is what proves the pins match.
`sh scripts/pin-manifests.sh --print` writes nothing and prints
the applicable manifest and directory rows — the mode the Rust tests read to
prove the shell and `goalcup_protocol::manifest::connector_dir_sha256` hash the same
stream.
