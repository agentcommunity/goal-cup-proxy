# Operating the tracker

Updated: 2026-10-10

How do I build, configure and run the binaries in full detail: the daemon config, the CLI flags
and exit codes, the development walkthrough, the MCP server, the verifier, the committed traces,
fixtures and connector pins? For participants and contributors. The short path (build, gate,
install, one capture run) is in [../README.md](../README.md).

> **Historical sections.** The explicitly marked V1 walkthroughs below preserve the signed-start,
> receipt-binding and thirteen-check path measured on `v1/trunk`; they are not current M2
> instructions. For M2 use this file's current boundary, the README capture run and
> [ARCHITECTURE.md](ARCHITECTURE.md); where historical wording disagrees, those win.

## Contents

- [Building and verifying](#building-and-verifying): prerequisites, the gate, ignored tests, [CI](#ci)
- [Installing the participant binaries](#installing-the-participant-binaries)
- [Running the daemon](#running-the-daemon)
- [Running a session with the CLI](#running-a-session-with-the-cli)
- [A development run, end to end](#a-development-run-end-to-end)
- [The MCP server, `goalcup-mcp`](#the-mcp-server-goalcup-mcp)
- [Running the verifier](#running-the-verifier)
- [The committed traces](#the-committed-traces)
- [Regenerating fixtures](#regenerating-fixtures)
- [Pinning connectors](#pinning-connectors)

## Current M2 operating boundary

v0.5.0 is published as source only
([cleanup record](qualification/v0.5.1-public-release-cleanup.md)). No downloadable binary,
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

The default build excludes V1 execution but still parses the retained Codex and Claude
`manifest.toml` files at startup. Full removal is ticket OS-29 and is not complete. The
schema-1 synthetic contract kit covers exactly four cases: `controlled-workload` (no identity,
append-only), `synthetic-preassigned` (launch identity, append-only), `synthetic-stderr-token`
(deferred identity, append-only), and `synthetic-stdout-json` (launch identity, two snapshot
generations). All require `--dev` and exclude a wrong-id decoy where applicable.

## Building and verifying

Prerequisites:

- **Rust 1.89 or later.** The workspace is edition 2024 (`Cargo.toml`), which needs 1.85. It
  also uses `let` chains (for example in `crates/goalcup-cli/src/browser.rs`), and the locked
  dependencies `time` and `rcgen` declare 1.88, which is the highest any of them declares.
  Development uses 1.98.0; no older release has been tried. `rust-toolchain.toml` asks for
  `stable`.
- **macOS or Linux.** `goalcup-platform` refuses to compile on anything else
  (`crates/goalcup-platform/src/lib.rs`). The V1 claim is one macOS target; Linux is a
  smoke check (see CI below).
- **A C compiler.** `ring` compiles C and assembly; a `v1-compat` build also compiles the bundled
  SQLite that `rusqlite` carries (D42).
- **`shasum` or `sha256sum`**, because the test suite runs `scripts/pin-manifests.sh --print`
  (`crates/goalcup-verifier/tests/checks_v13.rs`), and **`python3`** if you change
  `connectors/` and regenerate the pins (`scripts/pin-manifests.sh`).

```
cargo build
cargo test --workspace
```

That second command is the whole suite, every crate at once, and it is what "green" means
here. One crate at a time, or one file of one crate, is the same runner narrowed:

```
cargo test -p goalcup-daemon                       # one crate
cargo test -p goalcup-daemon --test control_api    # one integration test file
```

On the maintainer's machine every cargo command carries `--offline`, so none of them can
fetch: `cargo fetch --locked` runs once first, and every later command works from the cache.
Add the flag to any command in this file for the same guarantee.

One narrowing does not work from a clean tree:
`crates/goalcup-devserver/tests/api_gate.rs` drives the shipped `goalcup-daemon` and
`goalcup-verify` binaries over loopback, and only `--workspace` builds them. It says so, by
name, rather than skipping.

The gate a change has to pass before it is committed is four commands, in this order:

```
cargo build
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

`cargo build` is step zero and it is not redundant. The other three all build
dev-dependencies, and `goalcup-daemon` depends on itself in its dev list with the
`test-hooks` feature on — cargo unifies features within a package, so **the shipped,
`test-hooks`-off build of the library is compiled by none of them**. A `#[cfg(not(feature =
"test-hooks"))]` mistake, or a non-gated reference to a gated field, passes the last three
and breaks the binary that ships.

`cargo test --workspace` skips every test carrying `#[ignore]`. That set is intentionally not a
stable inventory: it includes subprocess child halves, fixture regenerators, disposable-volume
checks, real-process campaigns, live-harness checks and full vertical suites. List the current set
from source with `rg '^\s*#\[ignore' crates`; run only a named suite with its documented feature,
environment and serialization requirements. Do not treat the default gate as proof that ignored
coverage ran, and do not run all ignored tests indiscriminately.

`crates/goalcup-cli/tests/real_daemon_end_to_end.rs` is one such manual suite. Its principal V1
compatibility journey drives the shipped CLI through `start`, arming, `status` and `seal` against
the real daemon, devserver and verifier. Because the V1 hook leg has been removed, its current
oracle is a readable, chained, transcript-only **Fail**, not Pass: one
`leg1_call_without_leg2_post` Fail, two `leg1_without_leg2_post` Flags and one
`unwitnessed_keep_going` Flag. It needs the `v1-compat` build (D42) and spawns the sibling daemon,
so build that feature set immediately before running the suite:

```
cargo build --workspace --features v1-compat
cargo test -p goalcup-cli --features test-support,v1-compat --test real_daemon_end_to_end -- --ignored --test-threads=1
```

The D3 dependency check, `the_structural_bans_hold_at_the_dependency_level`, lives in `dependency_bans.rs` and runs in both builds.

The synthetic vertical suite also runs by hand after building the workspace binaries:

```sh
cargo test -p goalcup-cli --offline --features test-support --test m2_synthetic_vertical -- --ignored --test-threads=1
```

`--features test-support` is written out rather than left to cargo. The crate's own dev-list
already names itself with that feature on, so it is on either way — but `tests/support/mod.rs`
does not compile without it, and a line that says which features it needs is a line that keeps
working when that dev-dependency is untangled.

Nothing in the suite reaches the network. `goalcup-daemon`'s tests upload to
`goalcup-devserver` on a loopback port they bind themselves, and the one listener the daemon
serves on `m2/wire`, the control API, is refused a non-loopback address by
`DaemonConfig::validate`. (On `v1/trunk` the hook receiver is a second listener under the
same rule.)

### CI

`.github/workflows/ci.yml` is a Linux smoke check, **not the gate**. Its one job, `check`
(workflow `ci`), runs on `ubuntu-24.04` with a 20-minute timeout for every pull request into
`main` or `v1/trunk`, every push to those branches, and manual starts. There is no path filter,
so every such pull request gets a completed `check` that branch protection can require. Agent
branches never start it.

The job's `scope` step runs `scripts/ci-scope.py` on the event's two commits (the pull
request's base and head, or the push's before and after). Its output, `rust=false`, skips the
Rust smoke only when every changed path is Markdown under `docs/`, Markdown at the repository
root, or `.github/PULL_REQUEST_TEMPLATE.md`. Any other path, a deleted or renamed code path, a
new branch, a manual start, or a comparison that fails gives `rust=true`. Every run then runs
the `test_ci_scope.py` and `test_check_links.py` suites and `scripts/check-links.py --root .`.
When `rust=true` it also runs `cargo build --workspace`, `cargo fmt --all --check`, `cargo
clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace --exclude
goalcup-daemon --exclude goalcup-cli`, in that order. A failure in any step fails the job.

Every runner minute costs something on every pull request. Keep this file small:

- Do not add branches, jobs, a build matrix, or the daemon and CLI suites. Those suites spawn
  process trees that hang or kill a 2-core Linux runner, and Linux is best-effort, not a claim.
- A green `ci` run proves nothing about the macOS target. The four-command gate above, plus
  the ignored real-daemon and synthetic vertical suites, runs by hand before a merge.

### Installing the participant binaries

Nothing below is a `cargo run`. Every command in this file is written as a participant types
it, which means `goalcup`, `goalcup-daemon`, `goalcup-verify`, and `goalcup-mcp` are on
`PATH` — and a clean checkout has none of them there. `goalcup-devserver` is development-only,
not a participant install target. One build makes the workspace binaries:

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

`$GOALCUP_SERVER` is the third one, and it belongs to the real V1 flow only (a `v1-compat` build; the
default build has no `goalcup auth`): `goalcup auth` reads it when `--server` is not given. A development run against a local pair never
authenticates, so it never needs it — see the development walkthrough below.

`$GOALCUP_CONFIG_DIR` is the fourth, and it is the one that has to agree with the daemon;
the next section is about that.

### Running the daemon

The daemon takes one required argument, a configuration file, and runs in the foreground
under whatever started it. There is no daemonisation, no pidfile and no logger; what it
would have logged is counted on the signed status document instead.

```
goalcup-daemon --config <FILE> [--foreground]
goalcup-daemon --config <FILE> --print-addresses
goalcup-daemon --version
```

A minimal configuration, with every field that has no default. Paths must be absolute:

```toml
# Where the daemon writes: the registry, every entry's artifact directory and its trace.
# The ONLY tree it writes to outside a tracker-created workspace. It sits BESIDE
# `config_dir` and never inside it: `config_dir` is the directory the CLI and the MCP
# server read, and neither may see a capture buffer.
artifact_root = "/Users/you/goalcup-artifacts"
# Schema-1 connector directories. In a checkout, this repository's `connectors/`.
connectors_dir = "/Users/you/src/goal-cup/connectors"
# `control.json` and the status-cache mirrors. Separate from `artifact_root`, above.
# **This must be the directory the CLI looks in**, because the CLI finds the running daemon
# by reading `control.json` out of it and nothing else tells it where the daemon is. So the
# value here is the CLI's own platform default: `~/Library/Application Support/goalcup` on
# macOS, `$XDG_CONFIG_HOME/goalcup` (else `~/.config/goalcup`) elsewhere. Never a harness's
# — not `~/.claude`, not `~/.codex`, not `~/.opencode`.
config_dir = "/Users/you/Library/Application Support/goalcup"
# Where a workspace is created when the caller does not choose one. It must not contain or
# sit inside `artifact_root`: an agent enumerates its workspace's parent on its first tool
# call, so a nested pair would put the run's own evidence in its field of view (R3).
workspace_parent = "/Users/you/goalcup-runs"
# The control API's listen address. Loopback, or the daemon refuses to start.
control_addr = "127.0.0.1:7787"
# Where signed chunks go. Loopback cleartext or refused: nothing in this build terminates
# TLS, so `https` is refused by name and `http` only to 127.0.0.0/8, `localhost` or `[::1]`.
# 7789 and not 7788: 7788 is the port the M0 OpenCode spike drove `opencode serve` on, and
# a walkthrough that puts the devserver there collides with a harness a reader may be running.
upload_url = "http://127.0.0.1:7789"
# The Ed25519 key every start receipt and acknowledgement is verified against, 64 hex.
# This is the devserver's published placeholder — REPLACE IT BEFORE RELEASE; see below.
server_public_key = "6e7a1cdd29b0b78fd13af4c5598feff4ef2a97166e3ca6f2e4fbfccd80505bf1"

# Everything below has a measured default and may be omitted. The round's own dates are
# the one section worth setting on a machine that is running the competition:
# [event]
# t0 = "2026-10-01T00:00:00Z"
# start_window_closes_at = "2026-10-08T00:00:00Z"
# rules_version = "rules-2026-10-01"
```

**That block is deliberately minimal, and minimal is not enough to measure a finish.** Without
an `[event]` section *and* a `[receipt_binding]` section with
`mode = "workspace_submission"`, the daemon takes the legacy fallback: the run still starts,
still seals, and still verifies `pass`, but the chain carries **no**
`FinalizationV1::WorkspaceSubmission` record, so the run's `TerminationOutcomeV1` never reaches
the trace at all and `checks/v13_binding.rs` has nothing to read. Three real journeys were spent
learning this. A working block is committed at
`docs/qualification/2026-09-18-v4-daemon.toml.example`. Its four paths are placeholders under
`/Users/you`, so set your own.

**Do not use `round dates: not configured (fallback)` as the tell.** Earlier text here, and
`docs/archive/v1/JOURNEY-RUNBOOK.md` §4b, said that line means "the run you are about to spend is measuring
nothing". It does not. Both journeys that qualified `v0.1.0` printed it *while taking the
workspace-submission path in full*: in the tagged `v0.1.0` binary the configured round is
clobbered at seal by `compose_capture_closed`, so every sealed entry prints it. `v1/trunk` fixed
that in `67184bc`; built from trunk, the line appears only when `[event]` was absent when the
entry was created, when the entry predates the fix, or when an open entry is rebuilt after a
restart with `[event]` removed (the rebuild reads the current config). It still says nothing
about receipt binding.
An operator following the old instruction would discard good runs. Until `goalcup status` and
`goalcup doctor` surface the receipt-binding mode directly
(`docs/superpowers/plans/2026-09-21-step6-findings.md` items 1-3), confirm the block is in the
daemon's own config file and check the chain for a `finalization_v1` record after the run.

**The `server_public_key` in that block is the published placeholder**, and it is worth
knowing what that means before you pin it. It is
`goalcup_verifier::PLACEHOLDER_SERVER_PUBLIC_KEY`, which is the verifying key of
`goalcup_devserver::DEV_SERVER_SEED`; `goalcup-devserver --print-key` prints it, and that
seed is a constant in this repository — so anybody can sign a receipt or an acknowledgement
under it, and a run anchored to it is anchored to nothing. The verifier says so, as V13's
`placeholder_server_key`, and only `--dev` suppresses that. The daemon does not: refusing
would break every development run in this tree. **Real organizer keys are V1.1 public-event
gates**, not V1 foundation qualification gates; this block is a development configuration and
says so here rather than in a comment a copy-paste drops.

`$GOALCUP_CONFIG_DIR` overrides **both sides at once**: exported, the CLI and `goalcup-mcp`
read that directory instead of the platform default, and a daemon started with the same value
in `config_dir` writes its `control.json` there. That is the whole of how a second, isolated
tracker is run on one machine — a test, a second round, a development build beside an
installed one — and it is why nothing in this tree needs to move `$HOME` to do it. Set one
without the other and the CLI looks in one directory while the daemon writes to another, and
every command answers "no daemon is running".

`crates/goalcup-daemon/tests/config_load.rs` parses that block, so the daemon's own loader
is what says it is still a configuration this build accepts.

With no `[event]` section the signed status document reports the entry's own timeline in the
competition's fields and marks it `event_status_source: "fallback"`, so a reader can tell a
date the organiser set from one standing in for it.

A local pair, which is what the tests do:

```
goalcup-devserver --addr 127.0.0.1:7789        # prints its key and address on stderr
goalcup-daemon --config ~/.goalcup/daemon.toml
```

The daemon prints two lines on stdout and nothing else: the control address and the path of
the `control.json` it wrote. (On `v1/trunk` there is a third, the hook receiver's address,
first.) `--print-addresses` prints the same two from that file **without starting anything** — it opens no registry, which is why it
works while a daemon is up and holding the lock, which is exactly when it is worth running.
The bearer in that file is read and never printed.

| Exit | Meaning |
|---|---|
| 0 | Stopped on SIGTERM or SIGINT. A signal ends the process; it does not seal the entry, which is resumed when the daemon comes back. Also `--print-addresses` when it read `control.json`, and `--version`. |
| 1 | Refused, in one sentence on `stderr` prefixed `goalcup-daemon:` and never a panic. The configuration file could not be read or did not validate; the daemon would not start (`control.json` among the things it writes); the async runtime or the signal handlers would not install; or `--print-addresses` found no `control.json`. Every refusal that is about a file names that file. |
| 2 | clap refused the arguments (`--config` missing, an unknown flag). |

### Running a session with the CLI

> **`v1-compat` build only (D42).** This section walks the V1 path: `goalcup auth`, a plain `goalcup start`,
> the arming gate, `resume`. The default build is the capture product: `goalcup start --harness <id>
> --goal-file <file>`, then `status` and `seal`, and `goalcup-verify --capture-integrity <trace-dir>`.
> Build the workspace with `--features v1-compat` to follow what is below.

With a daemon up (above), a judged run is four commands. Every one of them reads
`$GOALCUP_CONFIG_DIR`, or the platform default beside it, to find that daemon.

```
goalcup auth --server https://<the round's server>   # once per machine
goalcup start --harness claude-code-cli              # creates, arms, launches, waits
goalcup status                                       # what the signed document says
goalcup seal                                         # end the run and print the trace
goalcup-verify <trace-dir>                           # the line `seal` prints
```

That last line is printed by `seal` rather than typed from memory, and what it prints
depends on the run: a run started with `--dev` gets `goalcup-verify --dev <trace-dir>`,
because the trace it produced needs the flag, and a judged run gets the line above. **Run
the line `seal` printed**, not the one in this file.

**`goalcup seal` is how a run ends early, and it is not the only way one ends.** In the current
M1 implementation, the development server stamps a legacy fixed 168-hour `deadline_at`, and the
daemon re-derives that deadline from `started_at`; automatic sealing follows that legacy
deadline. The organizer-signed `deadline_at`, including a short signed deadline, **is present
now** — `goalcup-devserver --qualification-window-seconds` drives it, and a deadline has fired
to the second on a real Codex run and terminated the harness
(`docs/qualification/2026-09-17-journey-dogfood.md`). What that run did *not* do is record a
termination outcome, because it took the legacy receipt-binding path; see the `[receipt_binding]`
note under the configuration above. The eventual event's 168-hour policy remains V1.1. Those are the entry's two endings, and a
signal is neither: SIGTERM or SIGINT stops the daemon process and leaves the entry to be
resumed when it comes back. So a participant who never types `seal` still gets a sealed run
and a trace; typing it is how you stop before the deadline.

`goalcup auth` is what puts the grant on disk, and `goalcup start` refuses a machine that
has none — the daemon exchanges that token with the server, and a start without one fails at
the far end after an entry already exists. Nothing in this repository serves a device code
(`goalcup-devserver` is the upload API and nothing else), so **that flow needs the round's
real server**. What it leaves behind is one file, `grant.json`, at `0600` in the
configuration directory — `$GOALCUP_CONFIG_DIR/grant.json`, else
`~/Library/Application Support/goalcup/grant.json` on macOS and
`${XDG_CONFIG_HOME:-~/.config}/goalcup/grant.json` elsewhere. This is its shape:

```json
{
  "server": "https://<the round's server>",
  "grant_token": "<the secret this machine starts runs with>",
  "subject": "<who the server says you are>",
  "email_verified": true,
  "scopes": ["entry:create"],
  "expires_at": "2027-01-01T00:00:00Z",
  "stored_at": "2026-09-06T00:00:00Z"
}
```

It is documented so a participant can see what their machine is holding and delete it, not
so it can be hand-written: a token nothing issued is a token the server will not honour.
**A development run does not need one at all** — `goalcup start --dev` admits a machine that
never authenticated, and says so on stdout. The next section is that run, end to end.

`start` blocks: it launches the harness, hands the daemon the process (D22), and waits at the
arming gate and then for the harness itself. Leave it running. `status`, `seal` and `resume`
name the newest run started from this machine when no entry is given.

**Log the harness in once, outside the tracker, before your first run.** Every entry gets a
FRESH configuration directory — `CLAUDE_CONFIG_DIR = {harness_home}`, minted by the daemon
per entry — and Claude Code keeps its first-run state (`hasCompletedOnboarding`, `theme`,
`oauthAccount`) in `<config dir>/.claude.json`. A fresh one shows a theme picker and then
"Select login method", which is not something an unattended run can answer. So the connector
declares a `[home_seed]` section naming the keys, and `goalcup start` and `goalcup resume`
**read** them out of your own `~/.claude.json` and write them into the per-run home before
the harness is spawned, printing one line saying which file they read. A machine that has
never signed in is refused with the step to take, rather than launched into a login screen.
Among the copied keys is `oauthAccount`, which carries your own email address — this puts it
into the per-run home on your own machine, and it goes no further than that.

That read is not one of D21's app-wide exceptions and does not become one: D21 lists app-wide
**writes**, each with a reversal procedure, and nothing here writes a byte outside the run's
own harness home. The manifest has no field that could ask for a write, and the seeding code
opens no path under your home for writing.

Claude Code asks a second first-run question, per project: "Is this a project you created or
one you trust?", whose default answer is **No, exit**. No copied key can answer it — your
configuration was written before this run's workspace existed — so the same step marks THAT
ONE DIRECTORY trusted in the per-run file. It is a folder the tracker created itself, empty,
for this entry alone (R3), so it is the one directory whose trust question the tracker can
answer without speaking for anybody. No other path is named there.

Codex keeps its credential in `$CODEX_HOME/auth.json`, and `CODEX_HOME` is the per-run home
too — so the same section copies that ONE FILE whole, verbatim, mode `0600`, out of your own
`~/.codex`. It is your own credential, on your own machine, in a directory the tracker made
for your own run: nothing uploads it, nothing chains it and no trace names it. Nothing is
written back to `~/.codex`. If you would rather the tracker did not touch that file, sign in
against the run's own home instead —

```
CODEX_HOME=<the entry's harness home> codex login
```

— and the copy is skipped: a file the per-run home already holds is left exactly as it is,
and one that is not on the machine is a line on the screen rather than a refusal.

**Claude Code needs one more step, and it is yours to choose.** Measured on 2026-09-07: the
onboarding keys above remove the theme picker, the "Select login method" screen and the
per-project trust question, and they are **not the credential**. On macOS that lives in the
login keychain, and a run under a fresh `CLAUDE_CONFIG_DIR` does not inherit it — the harness
comes up in the tracked workspace, takes the prompt, and answers `Not logged in · Run
/login`. Two ways round it, both of which work and neither of which this document picks for
you:

- run `/login` once inside the launched Claude Code, per entry; or
- export `ANTHROPIC_API_KEY` in the shell that runs `goalcup start`, so the launched harness
  inherits it.

Whichever you choose, do it in the harness or in your own shell. **Never type a credential
into a tracked session's prompt** — that is a message, and messages are captured.

`goalcup start --print-launch` prints the launch line and runs nothing — and, since it has to
create an entry to render that line (which names the workspace, the artifact root and the
session the daemon minted; on `v1/trunk`, the receiver token too), it abandons that entry before it returns
and says so. `goalcup entries` afterwards lists nothing new.

`goalcup doctor --harness <id>` scopes the report **and the exit code** to one connector.
Without it, `doctor` reports every shipped connector and exits 1 if any of them failed —
including one you have no intention of running, which is the honest answer to "is this
machine healthy" and the wrong one to "can I start this harness".

**The rest of the flags**, none of which the four commands above need. `goalcup <cmd> --help`
carries each of them; they are listed here because a flag a reader has to discover from
`--help` is a flag they discover after they needed it.

| Flag | What it does |
|---|---|
| `goalcup start --attach` (`v1-compat` only) | R4's other mode: arm the folder and **launch nothing** — you open your own session in that workspace. The gate then blocks in the foreground and prints what it sees. On `m2/wire` it arms on the harness's first transcript record in that folder; on `v1/trunk` it also prints the connector's human arming step where there is one. |
| `goalcup start --arming-timeout <SECONDS>` | How long to wait for the arming signal (on `m2/wire`, the harness's first transcript record), for this run only. Third and last in the precedence chain: the connector's own default, then the daemon's configuration, then this. |
| `goalcup seal --dry-run` | Walk this machine's workspace and print the self-check — what is there and what the exclusions take out — and seal nothing. It makes no call at all, so it answers on a machine whose daemon is not running: "what is in my workspace" is a question the tracker was never going to be asked. |
| `goalcup auth --no-open` (`v1-compat` only) | Print the URL and the code instead of opening a browser. The headless machine's path. |
| `goalcup auth --poll-limit <N>` (`v1-compat` only) | How many times to ask the server whether the code has been entered yet. |
| `goalcup ui [ENTRY]` (`v1-compat` only) | Open that entry's status page in a browser; `--print` prints the URL instead. Defaults to the run this daemon is carrying. |
| `goalcup resume --print-launch` (`v1-compat` only) | Ask the authenticated, read-only `resume-preview` endpoint for the relaunch pins, print the line and stop: no capture tail is caught up, no registered handle is cleared, credentials and the local cache stay unchanged, and no harness is launched or registered. Unlike `start --print-launch` the existing entry is kept. |
| `--json` on `status`, `entries` and `doctor` | The signed document itself, the summaries as one array, and the report as one document — instead of a page of text. |
| `--no-open` on `start` and `resume` (`v1-compat` only) | Print the status URL rather than handing it to a browser. The URL is printed either way (R18). |

`goalcup --help` lists five commands by default — `start`, `status`, `entries`, `seal`, `doctor` —
and a `v1-compat` build adds `auth`, `ui` and `resume` (and a plain `start`). It says where
verification is. There is no `goalcup verify`:
one verifier, one set of flags, one set of exit codes, and it is the binary below.

`goalcup`'s own exit codes, which `goalcup --help` also carries:

| Exit | Meaning |
|---|---|
| 0 | The command did what it was asked. |
| 1 | Refused, and you can fix it: no daemon running, no grant, a tracker still busy with the entry after the retry is spent, an entry that is already sealed, a mistyped sub-command, a preflight check that said no. |
| 2 | **Never this binary's.** It is a `Fail` verdict, relayed unchanged from `goalcup-verify` by `goalcup doctor --verify` (a `v1-compat` build; the default build has no verify mode and never leaves under 2). A participant whose arming timed out must not read the same number as one whose entry did not verify. |
| 3 | The CLI could not do its job: a file it could not read, an answer it could not parse. |
| 4 | (`v1-compat` only) **A gate said no, and nothing else does.** Two of them, both meaning "something has to change before this can go ahead". The start signal never arrived within the window, or the daemon has settled that this gate will never arm — the rung a participant actually meets, and it means the harness never told the tracker it had started. And `goalcup resume` refusing because the harness's lock is still on disk: the run is fine, something is holding it open, and the refusal names the file and what is known about its holder (DOGFOOD §3 expects exactly this). |

**What `--dev` means, on each of the three flags that carry it.** All three say the same
thing: this is a development run, and its result is not a judged entry's.

| Flag | What it admits |
|---|---|
| `goalcup start --dev` | two things. A connector directory this build does not pin — an edited manifest, a hand-written one, a connector under development. **And a machine that has never run `goalcup auth`** (a `v1-compat` build; the default build has no grant), so a development run against a local pair needs no grant; the run carries an empty token and the server it uploads to is vouching for nobody. Without the flag each is a pre-allocation refusal before daemon entry/artifact/workspace creation; CLI preparation may already exist and is cleaned. The run names on stdout whichever of the two it actually used, the entry record records it, and `goalcup seal` puts `--dev` into the `goalcup-verify` line it prints, because the trace it produced needs it. |
| `goalcup doctor --verify-with-dev <dir>` | the same, on the verdict this command relays, plus the placeholder-key Flag. It requires `--verify` and sets nothing else. |
| `goalcup-verify --dev <dir>` | see **Running the verifier** below: an unpinned rulebook becomes a Flag rather than a Fail, and the placeholder-server-key Flag is suppressed. |

None of them is a way to make a failing entry pass. `--dev` never touches V1–V12, never
suppresses a `chain_broken`, and a verdict that used it says so in its own document.

### A development run, end to end

> **`v1-compat` build only (D42).** This section walks the V1 path: `goalcup auth`, a plain `goalcup start`,
> the arming gate, `resume`. The default build is the capture product: `goalcup start --harness <id>
> --goal-file <file>`, then `status` and `seal`, and `goalcup-verify --capture-integrity <trace-dir>`.
> Build the workspace with `--features v1-compat` to follow what is below.

The walkthrough above needs the round's server. This one needs nothing but this checkout,
and it is the one that is **executed, as written, before this file is committed**.

It types `--dev` twice, and each time for a different admission: `goalcup start --dev`
because a development machine has no grant, and `goalcup-verify --dev` because this pair
signs with the published placeholder key. Neither makes a failing entry pass, and both say
what they did on stderr.

**One directory holds the whole thing**, so nothing here touches a harness's own
configuration — not `~/.claude`, not `~/.codex`:

```
mkdir -p dev/cfg dev/artifacts dev/runs
cat > dev/daemon.toml <<EOF
artifact_root = "$PWD/dev/artifacts"
connectors_dir = "$PWD/connectors"
config_dir = "$PWD/dev/cfg"
workspace_parent = "$PWD/dev/runs"
control_addr = "127.0.0.1:7787"
upload_url = "http://127.0.0.1:7789"
server_public_key = "6e7a1cdd29b0b78fd13af4c5598feff4ef2a97166e3ca6f2e4fbfccd80505bf1"
EOF
export GOALCUP_CONFIG_DIR="$PWD/dev/cfg"
```

`config_dir` in that file and `$GOALCUP_CONFIG_DIR` in the shell are **the same directory**,
and that is the one thing here that cannot be fudged: the CLI finds the running daemon by
reading `control.json` out of it, and nothing else tells it where that daemon is.

Three terminals, each with `$GOALCUP_CONFIG_DIR` exported:

```
goalcup-devserver --addr 127.0.0.1:7789                    # 1. the signer
goalcup-daemon --config dev/daemon.toml                    # 2. the tracker
goalcup start --harness claude-code-cli --dev --no-open    # 3. the run — this one BLOCKS
```

The third prints its two banners, the entry, the workspace and the status page, and then
sits there:

```
--dev was typed: this run may use a rulebook this build does not pin, and its result is a
development run's rather than a judged entry's.
--dev also admits a machine that has never run `goalcup auth`: this run carries no grant, so
the server it uploads to is vouching for nobody.
entry e7cec8771302f0126
workspace: .../dev/runs/e7cec8771302f0126
status: http://127.0.0.1:7787/v0/ui?entry=e7cec8771302f0126
Recording. Leave this running.
```

Work in that workspace with the harness it launched. Then, from a fourth terminal:

```
goalcup status
goalcup seal
```

`seal` prints the trace directory and, on the next line, **the `goalcup-verify` line to
run** — with `--dev` on it, because this run had it. Run that line, not one from this file:

```
verify: goalcup-verify --dev .../dev/artifacts/artifacts/e7cec8771302f0126/trace
```

The current V1 verifier answers `"overall": "fail"` and exits 2. Its exact review queue is one
`leg1_call_without_leg2_post` Fail, two `leg1_without_leg2_post` Flags and one
`unwitnessed_keep_going` Flag, with no leg-2 human-message witness. That transcript-only result is
expected because the hook leg is gone; it is compatibility evidence, not a passing product verdict.
`goalcup start` itself still returns 0 when the daemon terminates the harness it was holding, which
is the process hand-off working.

**Two things will stop this walkthrough on a real machine, and both are the tracker
working.**

A `claude` on `PATH` is needed, because `claude-code-cli` is the connector this run names —
D12 measures its version before the workspace exists. With no harness installed, the fixture
this repository tests against stands in for one: `crates/goalcup-cli/tests/fixtures/fake-claude.sh`
copied onto `PATH` under the name `claude`. That is how the walkthrough is executed in the
gate, and it is the same path `cargo test ... --test real_daemon_end_to_end -- --ignored`
takes.

And a shell with `ANTHROPIC_BASE_URL`, or any `*_PROXY`, exported is refused before daemon
entry/artifact/workspace allocation. CLI run/home/CA/seed preparation may already exist and is
cleaned on that pre-allocation refusal:

```
goalcup: `goalcup start` refuses this run:
  FAIL  banned_env      `claude-code-cli` refuses a run with ANTHROPIC_BASE_URL set: these
                        change what the harness talks to or what it records, so a run under
                        one is not the run the trace describes.
        fix: unset ANTHROPIC_BASE_URL and run this again.
```

`unset` it and start again. The tracker never sets one of those itself, in any mode, and it
will not measure a machine that has.

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
| `goalcup_run_status` | How one run is doing: arming state, health, seconds remaining, when the tracker last synced, and whether that document is stale. Takes an optional `entry_id` and defaults to the most recently started run. `arming: "refused"` means the gate will never arm, which is the one state an agent must not sit and wait through. |
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
GOALCUP_REGEN=1 cargo test -p goalcup-verifier --test adversarial -- --ignored regenerate
GOALCUP_REGEN=1 cargo test -p goalcup-verifier --test compliant   -- --ignored regenerate
```

Read the diff before committing it.

### Pinning connectors

```
sh scripts/pin-manifests.sh
```

It recomputes the schema-1 whole-directory pin table and the transitional V1-manifest table.
These are source integrity allowlists, not package manifests:

* `CONNECTOR_MANIFEST_SHA256S`, in `crates/goalcup-verifier/src/lib.rs` — exactly the retained
  Codex and Claude `manifest.toml` digests. This is what the V1 arming receipt claims and what V13's
  `manifest_not_pinned` compares, because the `manifest.toml` a trace carries is the only
  byte of a connector a verifier can **recompute** after the fact.
* `CONNECTOR_DIR_SHA256S`, built from the data file `crates/goalcup-protocol/pins/connector-dirs.txt`
  by `crates/goalcup-protocol/src/pins.rs` (a malformed file does not compile) and re-exported
  from the verifier — exactly the Codex, Claude and Hermes whole-directory digests, with seeds and
  templates under `files/` included. The manifest digest cannot see an edited template, and the daemon renders those
  templates and then guards them, so the daemon hashes its own connectors directory at
  workspace creation, refuses a pinned connector whose bytes moved, and **claims the digest
  on the start receipt**. V13 reads it back there, and **only out of a receipt whose
  signature verified** — otherwise a participant could self-sign a receipt naming the pinned
  digest and buy silence: `connector_dir_unpinned` (Fail, and Pass-level under `--dev`) when
  the claim is not in this table, `connector_dir_unbound` (Flag) when a verified receipt
  carries no claim at all. A trace whose receipt did not verify, or that carries none, gets
  no directory finding — `receipt_invalid`, `receipt_unverified`, `receipt_missing` or
  `legacy_trace` is already the larger statement. `goalcup doctor` runs the CLI's own copy of
  the same comparison over the CLI's own installation.

Both tables use declared connector ids; schema 1 uses `harness.toml`, while the compatibility table
uses `manifest.toml`. Run it in the same commit as any change
under `connectors/`;
`sh scripts/pin-manifests.sh && git diff --exit-code` is what proves the pins match.
`sh scripts/pin-manifests.sh --print` writes nothing and prints
the applicable manifest and directory rows — the mode the Rust tests read to
prove the shell and `goalcup_protocol::manifest::connector_dir_sha256` hash the same
stream.
