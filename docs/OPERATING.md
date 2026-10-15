# Operating the tracker

Updated: 2026-10-15

How do I build, configure and run the binaries in full detail: the daemon config, the CLI flags
and exit codes, the verifier, the committed traces,
fixtures and connector pins? For participants and contributors. The short path (build, gate,
install, one capture run) is in [../README.md](../README.md).

## Contents

- [Building and verifying](#building-and-verifying): prerequisites, the gate, ignored tests, [CI](#ci)
- [Installing the participant binaries](#installing-the-participant-binaries)
- [Running the daemon](#running-the-daemon)
- [Running a capture with the CLI](#running-a-capture-with-the-cli)
- [Running the verifier](#running-the-verifier)
- [Pinning connectors](#pinning-connectors)
- [The matrix and the edge cases](#the-matrix-and-the-edge-cases): `scripts/matrix.sh`, the server-copy check, and the by-hand cases

## Current M2 operating boundary

v0.5.0 is published as source only
(cleanup record). Binary releases are built by `.github/workflows/release.yml` from a `v*` tag
that equals the workspace version, and installed with Homebrew
([packaging/README.md](../packaging/README.md)); they are unsigned and unnotarized, and no
qualification exists. Until a version is tagged, built binaries print `0.1.0`. The source
contains exactly three pinned schema-1 connector directories: `codex-cli`, `claude-code-cli` and
`hermes-cli`. A release tarball carries `codex-cli` and `claude-code-cli`, as committed, under
`share/goalcup/connectors`; `hermes-cli` stays maintainer-only (H-21). macOS is the only platform
measured with a real harness. On Linux the workspace builds, the chain's disk reservation works on
ext4 and XFS and refuses other filesystems, and the synthetic contract-kit captures seal and
verify in Docker; no real harness has been captured there ([README, Linux](../README.md#linux)).

Connector discovery checks `$GOALCUP_CONNECTORS_DIR`, `<exe>/../share/goalcup/connectors`, then
`<exe>/../../connectors`, beside the path `goalcup` was started by and then beside its resolved
path, skipping a candidate that holds a link. It considers immediate child directories with a regular `harness.toml`,
validates every discovered schema-1 profile, then selects by declared id. Every run gets a fresh
`HOME`: Codex and Claude use shared isolated config homes (Claude links the checked Keychains
directory); Hermes gets a seeded per-run `HERMES_HOME`.

Control API v7 requires the version on mutating requests and binds creation to the daemon's loaded
connector-directory digest. Pre-allocation refusals happen before daemon entry/artifact/workspace
writes, but CLI run/home/CA/seed preparation may already exist and is cleaned. After restart an
unfinished v5 capture has status/seal-only authority; other authenticated mutators refuse before
side effects, and an unfinished launched capture receives one `IngestLost` gap.

The V1 execution path was removed (D48; the `v0.5.0` tag keeps it), and the Codex and Claude
`manifest.toml` files with it (OS-29): connectors load from `harness.toml` alone. The schema-1
synthetic contract kit covers exactly six cases: `controlled-workload` (no identity,
append-only), `synthetic-preassigned` (launch identity, append-only),
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
  `rust-toolchain.toml` pins `1.99.0`, which rustup installs on first use; the release workflow
  builds with the same pin.
- **macOS or Linux.** `goalcup-platform` refuses to compile on anything else
  (`crates/goalcup-platform/src/lib.rs`). The V1 claim is one macOS target; Linux is
  measured in Docker, not claimed ([README, Linux](../README.md#linux)), and CI runs a smoke
  check (see below).
- **A C compiler.** `ring` compiles C and assembly.
- **`shasum` or `sha256sum`**, because the test suite runs `scripts/pin-manifests.sh --print`
  (`crates/goalcup-protocol/tests/it/connector_pins.rs`), and **`python3`** if you change
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
  process trees that hang or kill a 2-core Linux runner. CI runs the reservation tests on x86_64
  ext4; the daemon and CLI suites run on Linux in Docker via `scripts/linux-check.sh`, not in CI.
- `.github/workflows/linux-x86_64.yml` is manual only (`workflow_dispatch`, never on a push or
  pull request). It runs `scripts/linux-check.sh` on an x86_64 `ubuntu-24.04` runner with
  `LINUX_CHECK_HARNESSES=1`, which ends with `scripts/linux-harness-smoke.sh`: the measured x86_64
  Codex and Claude Code builds launched with no login to the auth step. The suites above run
  there because a public repository's hosted runner has 4 vCPUs and they run in Docker with
  `--init`; how long a run takes is not yet measured. A second job, `harness-smoke`, runs the
  smoke alone on the runner after a release build, so a suite that fails on that host does not
  hide the harness result. Start it by hand.
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

— or copy the three binaries somewhere already on it, and **set `$GOALCUP_CONNECTORS_DIR` to
this checkout's `connectors/`**:

```
export GOALCUP_CONNECTORS_DIR="$PWD/connectors"
```

That second variable is not optional for a copied binary, and it is the one thing about
installing this tree that surprises people. The connector directories (`harness.toml` and
seeds) are the **rulebook** — how a harness is launched and captured, with their digests pinned
in `goalcup-protocol` — so they live beside the binary and never under a directory the
participant writes. `goalcup` looks for them at `$GOALCUP_CONNECTORS_DIR`, then at
`<exe dir>/../share/goalcup/connectors` for an installed layout, then at
`<exe dir>/../../connectors`, which is what makes `target/release/goalcup` find this
repository's `connectors/` with nothing exported at all. Both are tried beside the path
`goalcup` was started by, then beside its resolved path, so a link in `~/bin` to
`target/release/goalcup` finds them too. A binary copied to `~/bin` matches none of them and
refuses, naming the variable. `cargo install` lands in the same place
for the same reason.

`$GOALCUP_CONFIG_DIR` is the third, and it must agree with the daemon (next section).

### Running the daemon

You normally never start it yourself. `goalcup start`, `seal`, `status` and `entries` start
`goalcup-daemon --config $GOALCUP_CONFIG_DIR/daemon.toml --exit-when-idle` when no daemon answers
on `control.json` (the README's [daemon](../README.md#the-daemon) note says how), and
`goalcup start` writes a default `daemon.toml` the first time: the keys below, with
`artifact_root` and `workspace_parent` beside the configuration directory
(`<config dir>-artifacts`, `<config dir>-runs`). Edit that file to change them.

The daemon takes one required argument, a configuration file, and runs in the foreground
under whatever started it. There is no daemonisation, no pidfile and no logger; what it
would have logged is counted on the signed status document instead. With `--exit-when-idle`
it exits 0 once no capture is unfinished and no authenticated control request has arrived for
30 seconds, removing its `control.json` first; without it, it runs until a signal.

```
goalcup-daemon --config <FILE> [--foreground] [--exit-when-idle]
goalcup-daemon --config <FILE> --print-addresses
goalcup-daemon --version
```

A minimal LocalOnly configuration, with every field that has no default. Paths must be absolute:

```toml
# The registry, every entry's artifact directory and trace: the ONLY tree the daemon writes
# outside a workspace. BESIDE `config_dir`, never inside it: the CLI reads that one.
artifact_root = "/Users/you/goalcup-artifacts"
# Schema-1 connector directories. In a checkout, this repository's `connectors/`.
connectors_dir = "/Users/you/src/goal-cup/connectors"
# `control.json`. It MUST be the directory the CLI looks in (the
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
keeps loading. Nothing in this tree uploads or checks a receipt (D48), so the pair changes
nothing. Omit both.

`$GOALCUP_CONFIG_DIR` overrides **both sides at once**: exported, the CLI
reads that directory instead of the platform default, and a daemon started with the same value
in `config_dir` writes its `control.json` there. That is the whole of how a second, isolated
tracker is run on one machine — a test, a second round, a development build beside an
installed one — and it is why nothing in this tree needs to move `$HOME` to do it. The CLI
starts a daemon from `daemon.toml` in that same directory, so the default it writes always
agrees. A hand-written `config_dir` that names another directory does not: the started daemon
writes `control.json` where the CLI is not looking, and `goalcup start` refuses, naming
`daemon.log`.

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
| 0 | Stopped on SIGTERM or SIGINT, or idle under `--exit-when-idle`. Neither seals an entry. After a restart an unfinished capture has status and seal authority only. Also `--print-addresses` when it read `control.json`, and `--version`. |
| 1 | Refused, in one sentence on `stderr` prefixed `goalcup-daemon:` and never a panic. The configuration file could not be read or did not validate; the daemon would not start (`control.json` among the things it writes); the async runtime or the signal handlers would not install; or `--print-addresses` found no `control.json`. Every refusal that is about a file names that file. |
| 2 | clap refused the arguments (`--config` missing, an unknown flag). |

### Running a capture with the CLI

A capture run is one command, and the verify line it prints; `goalcup start` starts the daemon
for it (above). Every command reads `$GOALCUP_CONFIG_DIR`, or the platform default beside it, to
find that daemon.

```
goalcup start --harness <id> --goal-file <PATH>   # launches, waits, seals, prints `verify:`
goalcup status                                     # what the signed document says
goalcup entries                                    # every run the registry holds
goalcup-verify --capture-integrity <trace-dir>     # the line `start` printed
```

Run the `verify:` line `start` printed, not one typed from memory. The integrity exit codes are
in the [README](../README.md#run-a-capture).

`goalcup doctor` checks, per connector, that the connector files are the pinned ones and finds
the harness executable on `PATH` that `goalcup start` would run (hashed, never started). Any build
passes: builds are recorded, not pinned (D53). A harness that is not installed fails it.
`goalcup doctor --harness <id>` scopes the report **and the exit code** to one connector.
Without it, `doctor` reports every shipped connector and exits 1 if any of them failed —
including one you have no intention of running, which is the honest answer to "is this
machine healthy" and the wrong one to "can I start this harness".

**The rest of the flags.** `goalcup <cmd> --help` carries each of them:

| Flag | What it does |
|---|---|
| `goalcup start --capture-reserve-bytes <BYTES>` | The chain bytes the daemon reserves on disk before the launch. |
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
set of exit codes. The V1 commands `auth`, `ui` and `resume` are gone (D48); typing one is a
usage error (exit 1).

`goalcup`'s own exit codes, which `goalcup --help` also carries:

| Exit | Meaning |
|---|---|
| 0 | The command did what it was asked. |
| 1 | Refused, and you can fix it: no daemon to ask (`seal` with no `daemon.toml`) or none could be started, a tracker still busy with the entry after the retry is spent, an entry that is already sealed, a mistyped sub-command, a preflight check that said no. Also `start` after its seal, on Linux, when the launcher's sweep left a process the harness started still running at its bound, or could not read `/proc` after it had signalled one: stderr names the pids, and the run is sealed and its trace unchanged ([TERMINATION.md](TERMINATION.md#ruled-on-linux-confirmed-excludes-fork-and-leave-wrappers)). |
| 2 | **Never this binary's.** It is "invalid evidence", and only `goalcup-verify` leaves under it. |
| 3 | The CLI could not do its job: a file it could not read, an answer it could not parse. |
| 4 | Not left by a capture run: it belonged to the removed V1 gates (D48). |

**What `--dev` and `--dev-key` mean.** Both say the same thing: this is a development run,
and its result is not a judged entry's.

| Flag | What it admits |
|---|---|
| `goalcup start --dev` | A connector directory this build does not pin: an edited manifest, a hand-written one, a connector under development. Without the flag that is a pre-allocation refusal before daemon entry/artifact/workspace creation; CLI preparation may already exist and is cleaned. The run says so on stdout and the entry record records it. |
| `goalcup-verify --dev-key <hex>` | See **Running the verifier** below: a server key this build does not pin, for one run. |

Neither is a way to make a failing entry pass. Neither suppresses a `chain_broken`, and a
verdict that used one says so in its own document.

### Running the verifier

```
goalcup-verify --capture-integrity <trace> [--dev-key <hex>]
goalcup-verify --research-json <trace> --output <report.ndjson> [--dev-key <hex>]
```

`<trace>` is a protocol-v5 capture trace directory (`chain.jsonl` and the files beside it).
`--capture-integrity` prints the integrity report on stdout and nothing else;
`--research-json` writes the private research report to a new `*.ndjson` file outside the
trace and leaves stdout empty. Everything meant for a person — notices, errors, the summary
line — is stderr. A pre-v5 trace (a `trace.toml` beside a protocol-v1 chain) is not read: it
exits 3.

| Exit | Meaning |
|---|---|
| 0 | Complete, valid local evidence (the anchor may stay `local_only`) |
| 1 | Incomplete or unsealed coverage (for the research report: incomplete or partial) |
| 2 | Invalid evidence, a torn tail included |
| 3 | No usable report: the trace could not be read, its chain is not v5, or the invocation was refused |

This build pins no server key, so a receipt or acknowledgement never verifies on its own: an
untrusted signature is `receipt_key_not_accepted` or `ack_signature_unverified`, and the anchor
stays `local_only`. `--dev-key <hex>` (repeatable, 64 hex
characters) accepts acknowledgement and receipt signatures under that Ed25519 public key for
one run; the report records `key_source: "supplied"` so a development run can never be mistaken
for a production one.

### Pinning connectors

```
sh scripts/pin-manifests.sh
```

It recomputes the whole-directory pin table, `CONNECTOR_DIR_SHA256S`, built from the data file
`crates/goalcup-protocol/pins/connector-dirs.txt` by `crates/goalcup-protocol/src/pins.rs` (a
malformed file does not compile): exactly the Codex, Claude and Hermes whole-directory digests,
with seeds and templates under `files/` included. It is a source integrity allowlist, not a
package manifest. The loader hashes each connector directory and refuses a pinned connector
whose bytes moved, and a v5 entry binds the digest at create and in
`Launch.connector_dir_sha256`. `goalcup doctor` runs the CLI's own copy of the same comparison
over the CLI's own installation.

The table uses the connector ids `harness.toml` declares. Run it in the
same commit as any change under `connectors/`;
`sh scripts/pin-manifests.sh && git diff --exit-code` is what proves the pins match.
`sh scripts/pin-manifests.sh --print` writes nothing and prints the directory rows — the mode
`crates/goalcup-protocol/tests/it/connector_pins.rs` reads to prove the shell and
`goalcup_protocol::connector_dir::connector_dir_sha256` hash the same stream.

## The matrix and the edge cases

Does a run of this connector, on this machine, against this server, record, stop on the server's
deadline, seal and upload a copy the server agrees with? `scripts/matrix.sh` answers that for one
connector in one command and prints one row. The by-hand cases below answer the same question for
the things that go wrong in a week: a lost network, a sleeping laptop, a dead agent, a full disk, a
reboot, a closed window.

### `scripts/matrix.sh`

```
scripts/matrix.sh <connector> <server-url> [--keep] [--log FILE] [--timeout SECONDS]
scripts/matrix.sh --check-copy <config-dir> [run-id]
```

It makes a temporary configuration directory, work folder and `HOME`, runs `goalcup login` and
`goalcup start --practice` in them (the chat-mode harness under a pseudo-terminal that the script
types one prompt into), waits for the tracer's own stop at the server's practice deadline (10
minutes on staging and production, `PRACTICE_RUN_MINUTES` on a Worker you run), and deletes the
temporary directory. It writes the details to a log (`$TMPDIR/goalcup-matrix-<connector>-<time>.log`,
or `--log`) and prints the log path and exactly one row:

```
<connector> <os>-<arch> <harness_version> PASS|FAIL <date>
```

`<os>` is `macos`, `linux` or `wsl2`; `<arch>` is `arm64` or `x86_64`; `<harness_version>` is the
line the harness reported to the tracker (`unreported` when it did not), spaces replaced by `_`.
A FAIL names its first reason on stderr and in the log. PASS means all of these held:

1. both approvals (the login, then the run) were given, the run opened, and the harness was up;
2. the run ended at its deadline: the tracer printed its `deadline:` line, and the upload line
   `uploaded: … and the seal`;
3. `goalcup-verify --capture-integrity <trace>` exited 0 with `integrity: valid` and `anchor:
   signed` (the server's acknowledgement of the last record verified);
4. the server's copy agrees with the local chain (next section).

**The server copy.** The upload API has no call that returns the stored chain, so the script does not
download it. It asks `GET /v1/runs/{id}?entry_id=…` with the run's own credential (the file the
tracker saved, handed to `curl` as a header on its standard input) and requires the server's answer
to equal the local chain the verifier has just checked end to end: `offset` equal to the bytes of
`chain.jsonl`, `next_seq` to the record count, `head` and the last acknowledgement's `hash` to the
verifier's `validated_head`, the committed end (`final`) to the last record, `status: sealed` and
`stop: true`. The server stores a chunk only if its SHA-256 matches the one the tracker sent, only
at the offset, previous hash and sequence it holds, and signs an acknowledgement of the chunk's
last record, which the verifier checks. So equal positions mean the server holds a chain the tracker
sent, in order, up to the record the local chain ends with. What this does not prove is the stored
bytes themselves; the server's verify job over the reassembled copy does that, and until
it runs there is nothing to download to check them (SERVER-DESIGN, section 4).

A server whose signing key this build does not pin (a local Worker, staging) fails the first
`goalcup-verify` with exit 2 and `receipt_key_not_accepted`. The script then runs it again with
`--dev-key` set to `MATRIX_SERVER_KEY`, else the public key in the run's own receipt, and logs
that the verification borrowed its trust from this invocation (`key_source: "supplied"`).

**Rules the script keeps.** The approval key comes from `GOALCUP_TEST_APPROVAL_KEY` only and the
member from `GOALCUP_TEST_MEMBER_ID`; the script takes the key out of its environment at once, so
no child (`goalcup`, the daemon, the harness) ever has it, and sends it to `curl` on standard input. It is not an argument, a file or
a log line, and the script has no `set -x` or verbose `curl`. With no key set, it prints the
approval address and code that `goalcup` printed, and waits while you approve in a browser as a
member. It never reads or writes the real goalcup configuration directory, `~/.codex`, `~/.claude`
or `~/.hermes`. It refuses `https://ingest.agentcommunity.org`
unless given `--allow-production`, and nothing here ever passes it.

**Targets.**

- **A local Worker** (nothing to ask anyone): `cd server/worker && npm ci && cp .dev.vars.example
  .dev.vars`, then start `wrangler dev` with a short practice run, for example
  `node node_modules/wrangler/bin/wrangler.js dev --env-file .dev.vars --var DB_BACKEND:local --var
  PRACTICE_RUN_MINUTES:2 --port 8787` (`npm run dev` is the same without the short run), and in
  another terminal
  `GOALCUP_TEST_APPROVAL_KEY=dev-approval-key sh scripts/matrix.sh m2-fake-tui http://localhost:8787`.
  Stop the Worker by its process id when done.
- **Staging** (`https://goalcup-ingest-staging.b-9a5.workers.dev`): needs staging's test approval
  key and a member id it may approve as. Load the key into the environment in the same command that
  runs the script, so it is never typed, printed or stored:
  `GOALCUP_TEST_APPROVAL_KEY="$(cat <file holding the key>)" GOALCUP_TEST_MEMBER_ID=<member id> sh
  scripts/matrix.sh <connector> <staging url>`. Never `cat` that file on its own or `echo` the
  variable. The run lasts the 10 practice minutes plus the clone and the upload, about 12 minutes.
  On a Mac with a per-app firewall such as Little Snitch, the freshly built `goalcup` is held until
  someone allows that exact path (`target/<profile>/goalcup`): the row is then a FAIL whose reason
  is `goalcup login exited 1: … could not be reached … connection timed out`, while `curl` to the
  same address works. Allow the binary and run it again.
- **Production**: never. The script refuses it.

**Connectors.** `m2-fake-tui` is a connector the script writes for itself around the test suite's
fake terminal UI (`crates/goalcup-cli/tests/fixtures/m2-fake-tui.sh`), admitted with `--dev`: no
harness, no model and no login, so a PASS proves the tracker, the daemon, the pseudo-terminal
handling, the server and the verifier, and nothing about any real harness. A shipped connector
id (`codex-cli`, `claude-code-cli`) runs that harness for real and needs it installed and logged in
inside a prepared configuration directory of its own: `GOALCUP_CONFIG_DIR=<dir> goalcup prep`
(never the default directory), then `MATRIX_CONFIG_DIR=<dir> sh scripts/matrix.sh codex-cli <url>`.
The script types `MATRIX_PROMPT` once, `MATRIX_PROMPT_DELAY` seconds (30) after the tracker says the
harness is up; a real harness's start-up differs, so the first real-harness rows are watched. Only the
fake has been run through the script so far (below).

**Other environment.** `MATRIX_REPO_URL` (default the public empty repository
`https://github.com/agentcommunity/goalcup-matrix-empty`: the practice clone needs network access to
GitHub, and a practice clone is anonymous and installs no push helper, so the script's `github.json`
holds a well-formed fake token that is never sent anywhere), `MATRIX_BIN_DIR`, `--keep` (leave the
temporary directory, which holds the trace, for inspection), `--timeout` (1800 s). It needs `curl`
7.55 or later, `python3` and `script(1)`; on Linux it runs the harness as `script -qec` (util-linux),
on macOS as `script -q /dev/null`.

**`--check-copy`.** For a run you made by hand, `sh scripts/matrix.sh --check-copy <config-dir>
[run-id]` verifies every entry (segment) of the run locally and compares each with the server's
position for that entry (`?entry_id=`), using the run's credential from the configuration
directory. It starts, approves and changes nothing, and it works on any run in a configuration
directory you give it, including a run still recording. Entries still uploading may differ until they
catch up: run it again. A segment the tracker had to close from the tail of its chain (after a
reboot or a kill) verifies with exit 1, incomplete coverage, and that is reported on its line, not
hidden; the server's copy must still equal the local chain.

### The edge cases, by hand

Each case is a run you start yourself and break on purpose. Unless the case says otherwise:

- **Run.** Practice runs last 10 minutes, which is enough for the quick cases (marked below).
  The long ones need an organizer test run, `goalcup start --harness <id> --test --dev` with the
  `--dev` only for an unpinned connector, which is a full 168-hour run, opened only by the test
  operators the server lists, and deleted from the server afterwards with
  `server/worker/scripts/delete-test-runs.sh`. Use the staging server for these
  (`goalcup login --server https://goalcup-ingest-staging.b-9a5.workers.dev`), in a configuration
  directory of its own (`GOALCUP_CONFIG_DIR`), after `goalcup prep`, never on production.
- **Watch it from a second terminal** with the same `GOALCUP_CONFIG_DIR`: `goalcup status` (the
  run's state, with a `PROBLEM:` line for anything the participant must act on) and `goalcup entries`.
  The chat window itself prints nothing until the harness exits.
- **Check the server copy** after the case, once any upload has caught up:
  `sh scripts/matrix.sh --check-copy "$GOALCUP_CONFIG_DIR"`. It exits 0 and prints
  `check-copy <os>-<arch> <run id> PASS <date>` when every entry matches. FAIL names the entry and the
  field that differs (the log has the server's answers).
- **Real harness.** A case marked *real harness* means the fake cannot show it (it makes no model
  calls and writes no native session format the verifier decodes). Run those with a logged-in
  Codex or Claude Code. The fake (`m2-fake-tui`, through a `--dev` start on a prepared directory with
  the connector directory in `GOALCUP_CONNECTORS_DIR`) is enough where marked *fake ok*.
- **Pass and fail.** A case passes when the tracer did what its "Expect" says and the check above
  exits 0. Anything else is a failure; file it with the exact `goalcup status` text, the log, and
  the run id against the lane that owns the part that misbehaved (the CLI, the daemon, the Worker).

**1. Thirty minutes offline** (*fake ok* for the upload and heartbeat part; *real harness* for the
provider-outage lines). Setup: an organizer test run, harness started, prompt typed. Run: turn the
network off for 35 minutes (macOS: `networksetup -setairportpower en0 off`, or unplug; Linux:
`nmcli networking off` or `sudo ip link set <interface> down`), then back on. Expect: recording
continues and the chat window shows nothing new. After 15 minutes `goalcup status` shows `PROBLEM:
the last N heartbeats to the server failed: …`; after 30 minutes without an acknowledged upload,
`PROBLEM: last upload N min ago: …`. A real harness's provider connections that fail coalesce into
outage windows that the finish reports as `upstream_outages=N` on the `proxy admission:` line
and one `proxy upstream outage:` line each. Back online, the uploader retries (60 s backoff,
doubling to 15 minutes, with jitter) and catches up; both `PROBLEM` lines go away. Server copy: while
offline, `GET` shows the server behind the local chain, which is the expected state; after
catch-up `--check-copy` passes. Fail if the run stops or the tracer exits, recording reports gaps
that the offline period does not explain, or the server is still behind 25 minutes after the
network returned.

**2. Laptop sleep** (*fake ok*). Setup: an organizer test run. Run: put the machine to sleep for
30 minutes (macOS: `pmset sleepnow` or close the lid; Linux: `systemctl suspend`) and wake it. Expect:
the tracker holds a sleep lock while it records (`goalcup status` says `PROBLEM: the machine may
sleep: …` if it could not), but a lid closed on battery still sleeps the machine. After wake the
harness continues, heartbeats resume within 5 minutes, the uploader catches up, and the deadline is
counted on a clock that keeps running during sleep (a run whose deadline passed while asleep stops
at wake with the `deadline:` line). Server copy: `--check-copy` passes. Fail if the tracer exits or
hangs, `goalcup status` calls the recording healthy while the verifier reports a gap that
`goalcup status` does not mention, or the server stays behind.

**3. Agent killed, then `continue`** (*fake ok*; quick: a practice run works if you continue within
its 10 minutes). Run: from a second terminal kill the harness process, not `goalcup`
(`pgrep -f <program>`, `kill -9 <pid>`). Expect: `goalcup start` prints `harness exited: ended by
signal 9.`, then `The agent's process ended (…). Your run stays open until <deadline>; run
`goalcup continue` to restart it.`, keeps uploading the segment until it is caught up, and exits 0;
`goalcup status` shows `PROBLEM: the agent's process exited (…): run goalcup continue`. Then
`goalcup continue` from the folder you started in: no approval is asked, the harness reopens its last
session, `goalcup entries` lists a second entry, and the run's deadline did not move. Server copy:
`--check-copy` shows two entries, both matching, the first with its committed end (`final`). Fail
if `continue` asks for approval, the deadline moves, or either entry's server position differs.

**4. Disk nearly full** (*fake ok*; needs a small volume and the case 1 outage). The run's chain
and its disk reserve stay at 3 GiB together and give space back when something else fills the disk;
the run stops cleanly only when the disk is nearly full, the reserve has nothing left to give, and no
upload has been acknowledged for 30 minutes. Setup: a volume of about 5 GiB for the configuration
directory and its sibling `-artifacts` and `-runs` directories (a start needs 1 GiB beyond the 3 GiB
reserve): macOS `hdiutil create -size 5g -fs APFS -volname gcsmall /tmp/gcsmall.dmg` and `hdiutil
attach` it; Linux `truncate -s 5G /tmp/gc.img && mkfs.ext4 -F /tmp/gc.img` and a loop mount (ext4 and
XFS only: the reservation refuses other file systems). Put `GOALCUP_CONFIG_DIR` on it. Run: start an
organizer test run, take the network down, and fill the volume with a junk file down to under 256 MB
free (`dd if=/dev/zero of=<volume>/junk bs=1m`). Expect: first `PROBLEM: disk: N free beyond the
reservation`; after 30 minutes offline, the run stops itself with the reason "the disk is nearly full
and uploads are failing"; the harness is ended and the segment sealed locally, and the run stays
open (a full disk ends a segment, not the run): `goalcup status` says `run goalcup continue`. Delete the junk file, restore the network, `goalcup continue`. Server
copy: `--check-copy` passes for both entries once the uploads finish. Fail if the chain is torn
(`goalcup-verify` exit 2), the run does not stop and recording fails instead, or the full volume is
left holding a half-written record.

**5. Reboot, then `continue`** (*fake ok*). Run: during an organizer test run, restart the machine
(a hard power-off is the harsher case; do both on separate runs). Expect: nothing runs after the
reboot. `goalcup continue` from the run folder's parent closes the segment the dead daemon left from
the tail of its chain (an unfinished capture gets one `IngestLost` gap and no repair of a torn tail),
sends what it can, asks the server for the next segment and reopens the harness. The first
segment's `goalcup-verify --capture-integrity` exits 1 (incomplete coverage; a hard power-off may
leave a torn tail, which is exit 2 and must be named as such); the second is clean. Server copy:
`--check-copy` shows the first entry with its committed end, matching the closed chain, and the
second matching. Fail if `continue` is refused with the run still open and the deadline ahead, or
the first entry's server position is not the closed chain's.

**6. Tokens run out** (*real harness*). Run: start a run with an account whose model quota is spent
(or switch the harness's model to one the account may not use), and type the prompt. Expect: the
harness reports its own error and sits idle; the tracker keeps recording and heartbeating without a
`PROBLEM` line (an idle agent is not a tracker fault), and the run goes on to its deadline.
Heartbeats report `model_requests` that stops growing; on the server, for a competition run, that
produces the `idle_3h` and `idle_6h` notices (never for a practice or test run). Typing `keep going` once
the quota is back is allowed and is labelled a resume. Server copy: `--check-copy` passes.
Fail if the tracker stops, treats the idle agent as a recording failure, or sends the harness
anything.

**7. Window closed, then `continue`** (*fake ok*; a practice run works). Run: start inside a
terminal window, then close the window with the harness running. Expect: the shell hangs up, the
kernel hangs up the foreground group, the harness and `goalcup start` end; nothing is left running
(`pgrep -f goalcup` shows no `goalcup start`). From a new window: `goalcup status` names the exited
agent and `run goalcup continue`; `goalcup continue` opens the next segment as in case 3. Server
copy: `--check-copy` passes for both entries. Fail if a process of the old run survives, or the
first segment cannot be sealed or sent.

**8. A human message other than `keep going`** (*real harness*, since only a decoded native session
shows who typed what). Run: a practice or test run; after the prompt, type another instruction to the
harness (`please also add tests`), then `keep going`. Expect: the tracker prints nothing and blocks
nothing (it never relays, edits or blocks input). After the run, `goalcup-verify --research-json
<trace> --output /tmp/report.ndjson` writes a report whose summary line has `assistance.evidence:
observed` and `positive_native_markers` of at least 1, which counts the extra message as an
intervention. A message the decoder cannot attribute is shown as unexplained, not dropped. Server
copy: `--check-copy` passes. Fail if the message is missing from the trace, the report says
`none_observed`, or the tracker interfered.

**9. The deadline stop** (*fake ok*; this is what the matrix script does). Run: `sh
scripts/matrix.sh m2-fake-tui <server>`. Expect: the tracer stops the harness at the deadline by the
machine's clock (`deadline: the run's time is up…`), seals, sends the final heartbeat, uploads the
rest and the seal (`uploaded: … and the seal.`), and prints `harness termination: Confirmed`. Server: `status:
sealed`, `stop: true`, `final` equal to the last record. Pass: the row says PASS. For a real harness
use `MATRIX_CONFIG_DIR`.

**10. A second start replaces the first** (*fake ok*). A competition start by an account replaces
its other open competition run; a practice start replaces nothing. Run: with two configuration
directories (`GOALCUP_CONFIG_DIR=a`, `b`, two terminals) logged in as the same member on a server
with an open competition event (staging's dry-run event; the runs are 168 hours, so end the second
with `goalcup seal` when done), start a competition run in `a`, then one in `b`. Expect: `b` opens a
new run; within one heartbeat (5 minutes) `a` prints `a newer run on your account replaced this one`,
stops its agent, seals and sends its final heartbeat; `GET` for the first run carries `ends_at` equal
to the second run's `official_start`, and a chunk past the first run's committed end is refused with
`410`. Server copy: `--check-copy a` and `--check-copy b` pass (the first run is over, the second
open). Negative check: two practice starts replace nothing. Fail if the first run keeps recording
past 5 minutes after the second opened, or both stay open.

**The long runs.** Two 8-hour runs, one on macOS and one on Linux, on staging with a real harness and
an organizer test run: start it, leave it, and at the end `goalcup seal`, then `--check-copy`. File
what `goalcup status` showed at any point against the lane that owns it.

### Recorded rows

One row per matrix run, newest last. A row from the fake connector proves the tracker, server and
verifier on that platform and says nothing about a harness.

| Row | Server | Notes |
|---|---|---|
| `m2-fake-tui linux-arm64 matrix-fake-tui_1.0.0 PASS 2026-10-15` | staging | **Fake harness.** In Docker on an arm64 Mac (privileged, TMPDIR on a loop-mounted ext4 volume, the workspace built in the container). Run `rcdb57542b0748b97`, 10-minute practice run. Server copy: `sealed`, `stop`, offset 8090 = local `chain.jsonl` bytes, `next_seq` 12 = 12 records, `head` and last acknowledgement = the verifier's `validated_head`, `final` = record 11; `goalcup-verify` exit 0 under the receipt's key (`key_source: supplied`), anchor signed. |
| `m2-fake-tui macos-arm64 matrix-fake-tui_1.0.0 PASS 2026-10-15` | local Worker (`wrangler dev`, 2-minute practice run) | **Fake harness.** Same checks, same result shape (offset 8086, 12 records). Not run on staging from macOS yet: Little Snitch holds the new `goalcup` build's connections until its path is allowed (see Targets above). |
