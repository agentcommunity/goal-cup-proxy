# Bring your harness to Goal Cup

For a team that builds a coding agent and wants it in Goal Cup. Read this in five minutes; hand
the detailed checklist to your own coding agent. Updated: 2026-10-14.

## What Goal Cup is

Goal Cup is an agent hackathon with one rule: one person gives one coding agent one prompt,
then leaves it alone for 168 hours. Judges score the product the agent ships. The organizers
need to know afterwards that no human helped, so every participant runs an open-source
**tracer** on their own Mac.

The tracer starts your harness on the participant's goal and records two things while it runs:

- the session files your harness writes for that run (its own transcript), and
- its model traffic, through a local proxy set (`HTTPS_PROXY`) for that one process only. The
  proxy's certificate is trusted by that process only, through your harness's own CA variable
  (for example `NODE_EXTRA_CA_CERTS`).

When the run ends, the tracer seals both into one hash-linked trace, which the organizers check
offline with `goalcup-verify --capture-integrity <trace>`. Traces are never published.

What the tracer never does:

- It never changes your harness's model base URL or any endpoint setting.
- It never writes app-wide config (`~/.claude`, `~/.codex`, your harness's own dot-folder).
- It gives every run its own private home folder, so the run does not touch the user's normal
  setup. An API key is handed to your process only, and the tracer checks that no piece of it
  reaches the trace. Sign-in hosts are passed through without being recorded.

<a id="row-0"></a>

## Check first: harnesses the tracer cannot capture yet

Read this in minute one.

**Stop here** if your harness matches one of these: no connector can work until the tracer
itself changes, so send the facts sheet alone (below).

- **No local session file.** The tracer records the transcript file your harness writes on disk
  for the run. Sessions kept only on your server have no fit.
- **A database session store.** The file must be appended JSONL or rewritten whole each turn.
  SQLite, LevelDB and other database stores are refused.
- **HTTP/2-only or gRPC model traffic.** The recording proxy speaks HTTP/1.1 (and WebSocket
  upgrades). A client that also offers HTTP/1.1 is fine; an `h2`-only client cannot connect.
- **A local or loopback model** (Ollama, LM Studio, anything on `localhost`). The proxy never
  sees loopback traffic, so a connector naming a loopback or single-label host is refused.
- **A desktop or IDE app.** The tracer starts one CLI from argv: in chat mode in the
  participant's own terminal (D57), or headless with one goal. A desktop or IDE app has no fit.

**Known gaps**: the connector works, but the record is partial. Say so in the facts sheet.

- **Subagents with their own transcript files.** Exactly one session file pattern is recorded;
  separate subagent files are seen only as model traffic.
- **A binary that updates itself** with no flag or variable to turn that off. The tracer hashes
  the build that ran (the daemon hashes the binary at the launch, D53; the CLI reads none of its
  bytes); an update during the run goes unrecorded.
- **More than 16 model hosts, or a wildcard host.** Traffic to a host the connector does not
  list passes through unrecorded.

## What we ask of you

1. **A connector PR.** A connector is a small folder, `connectors/<id>/`, with one data file,
   `harness.toml`, and any seed files it needs. It tells the tracer how to start your harness
   (in chat mode, or headless), where its session file lands, and which hosts carry model traffic. No Rust code:
   your arguments, model name and environment variables are data, checked by rules that apply to
   every connector. A few things stay closed lists in the tracer's code, because each is a
   security decision: the name of an API-key variable the tracer hands your process, a
   CA-certificate variable and a redaction pattern; new session-id carriers and session-file
   forms are core changes too. If you need a new one, the decision table says so at step 3 and
   we add it first.
2. **A public facts sheet**, `docs/harnesses/<id>.md`: where your harness keeps sessions,
   config and logins on macOS, Linux and Windows, how it authenticates and what it talks to.
   [Why it exists](harnesses/README.md).

If your harness does not fit the tracer yet (for example, sessions live only on your server),
send the facts sheet alone. It tells us exactly what to build, and it is a complete
contribution on its own.

## What you need

- A Mac with Apple silicon for the kit's real-harness step, which runs on macOS only. The event
  also runs on Linux and WSL2, and the kit's synthetic captures run on Linux in Docker
  ([README, Linux](../README.md#linux)).
- Rust through [rustup](https://rustup.rs) (1.99 or later), the Xcode command line tools, and
  `python3` (the one macOS ships is fine). If Homebrew also installed Rust, make sure
  `which cargo` shows `~/.cargo/bin/cargo`.
- A network connection for the first build only, and about 3 GB of disk for `target/`.
- Your harness installed (note the version you measured), and a GitHub account.

<a id="1-capability-worksheet"></a>

## Steps

The [skill](../skills/adding-a-capture-harness/SKILL.md) is the full checklist, written so your
coding agent can follow it step by step, with the expected output of every command. In short:

1. **Get the code and build it.** Fork
   [goal-cup-tracer](https://github.com/agentcommunity/goal-cup-tracer) on GitHub, then:
   ```sh
   git clone https://github.com/<you>/goal-cup-tracer.git && cd goal-cup-tracer
   cargo nextest --version || cargo install cargo-nextest --locked
   cargo fetch --locked
   cargo build
   ```
   This is the only step that needs the network; the checks afterwards run offline.
2. **Write the facts sheet.** Copy the template and fill it from your docs and from a run with
   a throwaway `HOME`. Write "unknown" rather than guess.
   [codex-cli.md](harnesses/codex-cli.md) is a filled example.
   ```sh
   mkdir -p docs/harnesses
   cp skills/adding-a-capture-harness/templates/harness-facts.md docs/harnesses/<id>.md
   ```
3. **Check the fit.** Walk the [decision table](../skills/adding-a-capture-harness/decision-table.md).
   If a row says "core PR", stop and open an issue with your facts sheet: we change the tracer
   first. Amp is the worked example there. (The [check-first list](#row-0) above is row 0.) Two early surprises: the session id must reach the
   tracer as an id flag, a JSON line on stdout or a fixed-prefix line on stderr (plain text on
   stdout is not enough), and a Python harness that trusts only `SSL_CERT_FILE` or
   `REQUESTS_CA_BUNDLE` needs its own CA variable added first.
4. **Write the connector.** Copy
   `skills/adding-a-capture-harness/templates/connector-harness.toml` to
   `connectors/<id>/harness.toml` and fill it in; `connectors/hermes-cli/` is the closest
   example. Builds are recorded, not pinned (D53): name the program, and if it prints its
   version on one line, the arguments that ask for it (`version_args = ["--version"]`). The
   tracer records the build's digests and that line in every trace.
5. **Add a test case.** Copy `skills/adding-a-capture-harness/templates/kit-case/` to
   `crates/goalcup-cli/tests/fixtures/harness-kit/<id>/`. It is a small Python stand-in that
   behaves like your harness, so the tracer can be tested without a network or a key.
6. **Regenerate the generated files.** `sh scripts/pin-manifests.sh` records the connector's
   hash; the skill gives the one command that refreshes the test snapshots ("goldens").
7. **Check it and open the PR** (next two sections).

## Check it locally

```sh
sh scripts/check-connector.sh <id>
sh scripts/gate.sh fast
```

The first runs every connector check: files, pin, schema, test snapshots, and the test kit with
your stand-in. On a Mac its last line reads `check-connector: <id>: 6 passed, 0 failed, 0 skipped`.
The second is the build, format, lint and test gate every change passes; its last line reads
`gate fast: PASS in <N>s`. On a warm build expect about a minute for the first and two to
three for the second. If the kit fails, the summary says only `FAIL kit`; the skill explains
how to read the test output above it.

A passing kit proves the tracer starts, isolates and records something shaped like your
harness. It does not prove your real harness works under the tracer. To try that on your own
account (a trial on a self-built tracer, not a judged entry): run `sh scripts/pin-manifests.sh`
and `cargo build` (the pins are compiled in, so repeat both after every connector edit), run one
short goal as in the [README quickstart](../README.md#quickstart) with `target/debug/goalcup`
and `--harness <id>`, and check it with
`target/debug/goalcup-verify --capture-integrity <trace>`. No `--dev` flag is needed: `--dev` is
only for a connector folder whose id this build does not pin. Never put anything from a real
trace in the PR; it holds your transcript.

## What goes in the PR

- `connectors/<id>/harness.toml` and any seed files under `connectors/<id>/files/`;
- the one new line in `crates/goalcup-protocol/pins/connector-dirs.txt`;
- the regenerated snapshots under `crates/goalcup-cli/tests/fixtures/harness-golden/`;
- the test case under `crates/goalcup-cli/tests/fixtures/harness-kit/<id>/`;
- `docs/harnesses/<id>.md`.

In the description, paste the last lines of both checks, and say which parts an agent wrote and
that a person on your team read the whole diff. No keys, tokens or real transcripts anywhere. A
maintainer replies within 7 days and runs the checks again on a Mac.

## What is not ready yet

- **No installer.** The tracer is source only; participants build it with cargo.
- **No server.** Traces stay on the participant's machine; upload to the organizers is
  planned, not built.
- **macOS only for a real capture.** Linux is measured in Docker with synthetic captures only,
  and Windows is not measured, which is why the facts sheet asks for their paths now.
- **The capture limits** in the [check-first list](#row-0) above.

## Reach us

Open an issue on [goal-cup-tracer](https://github.com/agentcommunity/goal-cup-tracer/issues)
for anything public, including "this step did not work". For anything private, or a security
problem, email goalhack@agentcommunity.org ([SECURITY](../SECURITY.md)). The maintainer is
Balazs Nemethi.
