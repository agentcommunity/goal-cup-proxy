# Add a harness

**Being rewritten.** The steps below are 0.7 text; the 1.0 connector keys are in
[CONNECTOR-SCHEMA](CONNECTOR-SCHEMA.md).

**For contributors.** The one guide for adding a coding-agent harness to Goal Cup Proxy, any
harness, not only the ones that ship. It is for a harness team and for the coding agent that team
uses. Every step has a command, its expected output and a stop rule. To set up a harness that
already ships for your own run, read [HARNESS-SETUP](HARNESS-SETUP.md) instead: that page is for
participants. The contributor skill
([adding-a-capture-harness](../skills/adding-a-capture-harness/SKILL.md)) points here and holds no
steps of its own. Three other documents complete it:

- [CONNECTOR-SCHEMA](CONNECTOR-SCHEMA.md): every connector key and the rule that refuses it.
  Its [connector contract](CONNECTOR-SCHEMA.md#the-connector-contract) is what your connector may set.
- [docs/harnesses](harnesses/README.md): one facts sheet per harness.
- [writing-a-decoder](../skills/adding-a-capture-harness/writing-a-decoder.md): a separate
  verifier change. A connector needs no decoder.

<a id="quick-path"></a>

## Quick path: add and test a connector

Four steps. Nothing here needs a Rust build until step 3, and nothing needs an organizer.

1. [Check capture support](#row-0), then write the connector: a short [facts
   sheet](../skills/adding-a-capture-harness/templates/harness-facts.md) with the required facts
   (where it signs in, that it honours `HTTPS_PROXY`, the session file path, the setting that stops
   it asking for permission, the version command), and `harness.toml` from the [chat-mode
   template](../skills/adding-a-capture-harness/templates/connector-harness.toml). Unknown
   isolation, auth or transport facts, or a required core change, mean **facts only**; that is still
   a useful contribution. No Rust code belongs in an ordinary connector PR.
2. [Try it](#test-a-connector) with the installed release: point `GOALCUP_CONNECTORS_DIR` at a
   folder holding only your connector, then run `goalcup start --harness <id> --dev --practice`.
   Anyone with a Goal Cup account may do this. A `--dev` run never competes.
3. Run `sh scripts/check-connector.sh <id>`. A new connector fails its goldens step because its
   golden files are not there yet: that is expected, say so in the PR, and do not regenerate them.
   Every other step must pass.
4. Open the PR with the facts sheet. A maintainer regenerates the goldens, runs the gate, and ships
   the connector in the next release.

A passing stand-in check is synthetic evidence. [A real practice run and event
qualification](#getting-a-harness-into-the-event) are separate. A connector can capture an
unfamiliar format before it has a semantic decoder.

The [research target](RESEARCH-TARGET.md) defines the coverage to establish with a real harness.
Check context-visible tool calls/results and subagent conversations separately. Full independent
tool-output/spill capture is not required. Reuse shared wire decoders where the protocol fits;
native formats may need their own decoder. Record unmeasured coverage explicitly.
[Local next steps](research/LOCAL-NEXT-STEPS.md) gives the current validation sequence.
The [coverage matrix](research/coverage-matrix.md) records measured examples and limits; the
[harness-neutral replay helper](research/replay-comparison.md) compares local and retrieved
traces after the separate stored-byte fetch.

<a id="test-a-connector"></a>

## Test a connector

**First try, with no Rust build.** Install the latest release
([guide](PARTICIPANT-GUIDE.md#1-install)) and run `goalcup setup` once if you have not. Then give
`goalcup` a folder that holds only your connector. `--dev` refuses a directory that a build also
pins, so it must not hold the shipped connectors:

```sh
mkdir -p "$HOME/my-connector"
cp -R connectors/<id> "$HOME/my-connector/<id>"      # from a checkout, or write it there
export GOALCUP_CONNECTORS_DIR="$HOME/my-connector"
mkdir "$HOME/my-connector-run" && cd "$HOME/my-connector-run"   # an empty folder
goalcup setup --harness <id> --dev
goalcup start --harness <id> --dev --practice
```

Setup opens the harness once so you sign in and set it not to ask for permission. The start asks for
the usual browser approval. Any member of the Goal Cup may run `--dev`; the server opens it as a
practice or test run, and a `--dev` run never competes: it is refused as a competition run, and
`goalcup` refuses `--dev` on a competition start before the approval. The run needs no repository
and no token. Type a small task, let the harness answer, and let the server's practice deadline
stop, seal and upload the run, or end it early with `goalcup seal` from another terminal. A practice
run catches a harness that writes no session file within about three minutes. Keep the config and
the trace for diagnosis. `goalcup status` in another terminal shows progress.

**Check it.** From the repository root, after the one-time setup in step 1 below (Rust, 3 GB of
`target/`):

```sh
sh scripts/check-connector.sh <id>
```

For a new connector expect `check-connector: <id>: 4 passed, 2 failed, 0 skipped` on macOS, or
`5 passed, 1 failed, 1 skipped` on Linux. The failures are the goldens lines only: `<id>.json` and
`<id>.daemon.json` do not exist yet, and a maintainer writes them when the PR merges. Ignore the
checker's hint to regenerate them. Any other failure is yours to fix. The checker regenerates the
pin and tests the shared stand-ins; it does not run your installed harness or prove real model
traffic. Do not run `scripts/gate.sh`: the maintainer does, and CI does not run the CLI tests on a
PR.

**A fuller local smoke from a checkout** (for a maintainer, or an author who wants the daemon's own
trace):

```sh
export PATH="$PWD/target/debug:$PATH"
export GOALCUP_CONNECTORS_DIR="$PWD/connectors"
goalcup_test_root=$(mktemp -d "$HOME/goalcup-connector.XXXXXX")
export GOALCUP_CONFIG_DIR="$goalcup_test_root/config"
mkdir "$goalcup_test_root/launch"
cd "$goalcup_test_root/launch"
goalcup setup --harness <id>
goalcup doctor --harness <id>
goalcup start --harness <id>
```

Do not log this fresh config into the server for a local smoke. In setup, sign into the harness in
its isolated home and exit normally; the login `[todo]` step is expected locally.
In start, type a small task, let the harness respond, then exit normally. Goal Cup Proxy seals the run.
A pinned connector needs no `--dev`. In another terminal with
the same config, `goalcup status` shows progress and `goalcup seal` ends the run early.

Copy the exact `trace:` path printed by start/seal. With a new output file outside that trace:

```sh
goalcup-verify --capture-integrity <trace>
goalcup-verify --research-json <trace> --output "$goalcup_test_root/report.ndjson"
```

Integrity exits: **0** complete/valid, **1** incomplete/unsealed, **2** invalid, **3** no usable
report. Local-only anchoring is expected without a server. Read research output privately: check
that native evidence and real model request/response traffic were retained, inspect gaps and
undecoded formats, and record task success and termination separately. Integrity exit 0 alone
can occur with no model traffic. Unknown semantics may honestly be capture-only. Neither real
traces nor research NDJSON belongs in a public PR; even reports can contain excerpts.

**Server delivery is the `--dev --practice` run above.** It opens on the Goal Cup server like any
practice run, so the same run proves delivery. A practice run needs no GitHub repository and no
token. Let the server's practice deadline stop, seal and upload the run. Use `goalcup entries` and
`goalcup upload <entry>` if delivery is pending, verify the resulting trace, and check signed
acknowledgement through the terminal. See
[event qualification](#getting-a-harness-into-the-event) for what to send with the PR.

## What Goal Cup Proxy does

Goal Cup Proxy starts one harness for a participant and records two things:

- the declared native session artifacts the harness writes for that run, which may contain a
  transcript or only metadata (as with Amp);
- its model traffic, through a local proxy set by `HTTPS_PROXY` for that one process only. The
  recording proxy certificate is trusted through the harness's CA variable (for example
  `NODE_EXTRA_CA_CERTS` or `SSL_CERT_FILE`), set for that process and its children only.

At the end Goal Cup Proxy seals both into one hash-linked trace. `goalcup-verify --capture-integrity
<trace>` checks it.

Goal Cup Proxy never sets a model base URL, never writes app-wide config (`~/.claude`, `~/.codex`,
`~/.hermes`), and never prompts the agent. Harness homes are isolated from the user's normal
stores; a connector may keep its private home across serialized runs. The harness's own sign-in/setup stores its login there. Goal Cup Proxy never
sees, forwards or stores a provider key (D61). Sign-in hosts are tunnelled and not recorded.

A harness is data. A contribution has no `.rs` file:

- `connectors/<id>/harness.toml` and its seed files (`files/**`);
- one line in `crates/goalcup-protocol/pins/connector-dirs.txt`;
- the facts sheet `docs/harnesses/<id>.md`.

No test is written for a new connector. The connector contract runs for every folder under
`connectors/` and reads the connector's own facts from its `harness.toml`. Its static half
(`crates/goalcup-cli/tests/it/connector_contract.rs`) checks the pin, the hosts, the argv
against the connector's own `argv_guard`, the launch environment, the CA files, the goal
placement and the per-run home, and checks that the connector's own home variable, reserved
prefixes and guarded options are refused. Its dynamic half
(`crates/goalcup-cli/tests/full/connector_contract.rs`) runs the connector once through the real
binaries with one stand-in, in chat mode, and checks the argv, the environment, the
recorded session file and the sealed trace.

Arguments, the goal flag, the model name and `capture.env` are data. Generic rules shared by all
connectors check them. Three lists stay in core code, because each is a security boundary: the
CA modes and CA-name shape, the redaction forms, and the native forms. A new entry in one of them is a separate core change (see
[Do I need a core change?](#do-i-need-a-core-change)).

<a id="row-0"></a>

## Check first: what Goal Cup Proxy cannot capture

Read this before you build anything. If your harness matches an item under "Cannot be captured",
stop and send the facts sheet alone (status `facts only`). A facts sheet is a complete
contribution.

Cannot be captured:

| Item | Why | What it would take |
|---|---|---|
| A database session store the harness writes while the run goes (SQLite, LevelDB) | `native.form` admits `append_only`, `snapshot` and `final_snapshot`. A `final_snapshot` is a copy taken once, after the harness has exited, so a database the harness keeps writing during the run is refused: a copy taken mid-run is not a faithful record | A read-only native form for a live database. A store the harness only settles at exit can use `final_snapshot` |
| HTTP/2-only or gRPC model traffic | The recording proxy speaks HTTP/1.1 and WebSocket upgrades; an `h2`-only client cannot connect | An `h2` proxy path and a decoder for its framing |
| A local or loopback model | The recording proxy never sees loopback traffic. A connector that names a loopback or single-label host is refused | A loopback relay Goal Cup Proxy records |
| A desktop or IDE app | Goal Cup Proxy starts one CLI from argv | Not planned |
| No chat mode in a terminal (one-shot or headless only) | Goal Cup Proxy runs every harness in chat mode, where the participant types the prompt (D57) | Not planned |

Known gaps. The connector works, but the record is partial. Write each one in the facts sheet:

- Subagents with their own transcript files. Only the files a declared `[[capture.native]]` row
  matches are recorded. A subagent file no row matches is seen only as model traffic.
- A per-run native ceiling. All native rows of one run share 32 open streams and 1024 tracked
  files (the defaults). An append-only file past the 32nd waits until the finish, and a file past
  the 1024th is not tracked. Each loss is marked as a streamless gap, not dropped silently. A
  connector with several append-only rows can reach this in a long run. The Claude facts sheet
  (`harnesses/claude-code-cli.md`, section 11) states the numbers.
- A binary that updates itself with no flag or variable to stop that. The daemon hashes the build
  at launch. An update during the run is not recorded.
- A model service the participant switches to inside a running session. The host is read from
  the harness's config at each start and continue (`model_host_from`, D62), so the new service is
  recorded from the next `goalcup continue`.

## What you need

- A Mac (Apple silicon or Intel), or Linux. Every step runs on both.
- The harness installed (note the version you measure), a GitHub account for the PR, and the
  latest Goal Cup Proxy release ([guide](PARTICIPANT-GUIDE.md#1-install)). That is all the first
  try needs.
- For `check-connector.sh` only: Rust from [rustup](https://rustup.rs) (1.99 or later), the Xcode
  command line tools and `python3`. If Homebrew also installed Rust, `which cargo` must print
  `~/.cargo/bin/cargo`. A network for the first build only, and about 3 GB of disk for `target/`.

## Stop rules

- **Cannot be captured**: stop at step 0. Contribute the facts sheet alone.
- **Needs a core change**: stop at step 3. Open an issue that names the table row and links the
  facts sheet. Write no connector. The sheet with `Status: facts only` is the PR.
- **Unknown fact** in transport, session storage, isolation or auth, or session identity: write
  "unknown", set `Status: facts only`, and stop.
- **A key or token in a file, argv, log, facts sheet, PR or chat**: never. Never open or print a
  key file. The kit uses an invented key.
- **Real `HOME` touched**: stop and report it. Every probe uses a private `HOME`.

## Steps

`<id>` is the connector id: lowercase, `^[a-z][a-z0-9-]{1,47}$`, usually `<name>-cli`. `<program>`
is the harness executable. Run every command from the repository root.

0. **Can it be captured?** Read [Check first](#row-0). Stop on any item under "Cannot be
   captured".
1. **Set up.** Fork the repository and branch as in [CONTRIBUTING](../CONTRIBUTING.md), and read
   [AGENTS.md](../AGENTS.md) once. The Rust setup below is needed only for step 9, so you can write
   and try the connector first. Then, online, once per checkout:
   ```sh
   cargo nextest --version || cargo install cargo-nextest --locked
   cargo fetch --locked
   cargo build --offline
   ```
   Expect `Finished` and exit 0. After this the gate runs cargo with `--offline`. Warm timings:
   `check-connector.sh` about 1 to 1.5 minutes, `gate.sh fast` about 2.5 minutes. Keep one cargo
   target directory per checkout.
2. **Fill the facts sheet.** Measure with a private `HOME` and a harness home under a temp
   directory, against the exact installed version.
   ```sh
   cp skills/adding-a-capture-harness/templates/harness-facts.md docs/harnesses/<id>.md
   ```
   Keep every heading and its order. Give every fact a source (a docs link, or "measured on
   version X") or write "unknown". Write no secret, token or personal path (`$HOME`, not
   `/Users/name`). The worked example is [codex-cli.md](harnesses/codex-cli.md); the rules and the
   `Status:` values are in [docs/harnesses](harnesses/README.md). The sections map to the
   connector:

   | Facts sheet section | Lands in `harness.toml` |
   |---|---|
   | 1. Run one prompt, in chat mode first | `interactive`, `[capture].args`, `resume_args`, `continuation` |
   | 2. Install and version | `[capture.executable]` |
   | 3. Where it keeps things | `[capture.home]`, `[[capture.native]].root` |
   | 4. Session format and identity | `[[capture.native]]` `pattern` and `form`, `[capture.identity]` |
   | 5. Auth | `auth_hosts`, `[capture.home]` `run_home` and `config_home` (the harness's own login, in its home) |
   | 6. Network | `[capture.ca]`, `model_host_from` (or, for one fixed service, `[[capture.hosts]]`) |
   | 7. Process model | `[capture.termination]`, `[capture.argv_guard]` |
   | 8. Things Goal Cup cannot isolate | nothing; it stays in the sheet |

   Section 1 describes the harness's own chat in a terminal. Ask early for the two facts that most often need a core change: the
   CA variable the harness honours (section 6, [row 7](#do-i-need-a-core-change)), and how a chat
   sign-in is kept without an environment key (section 5, row 14). Ask where the harness saves the
   model service the participant picks (section 6): the config file and the key of its URL. That
   is `capture.model_host_from`, the default for any harness whose model service is the
   participant's choice (D62): Goal Cup Proxy records that one host and keeps no provider list. A
   fixed `[[capture.hosts]]` row is only for a harness that can reach one service.

   Stop on an unknown in section 4, 5 or 6, or in the macOS column of section 3.
3. **Do I need a core change?** Walk every row of [the table below](#do-i-need-a-core-change).
   Stop on any "core change" row.
4. **Record how to get the version.** Builds are recorded, not pinned (D53): Goal Cup Proxy hashes
   what `program` resolves to and records the digests. Find the file the CLI will run and the
   version command:
   ```sh
   python3 -c 'import os,shutil,sys; print(os.path.realpath(shutil.which(sys.argv[1])))' <program>
   <program> --version
   ```
   Expect one absolute path, then the version on the first line of stdout. Put the arguments in
   `[capture.executable] version_args` (`["--version"]`) and the path and version in section 2 of
   the sheet. If there is no version command, or it needs a network or a login, leave
   `version_args` out; the trace then says `unreported`. Never write `version` or `sha256`: the
   loader refuses both.
5. **Write the connector.**
   ```sh
   mkdir -p connectors/<id>
   cp skills/adding-a-capture-harness/templates/connector-harness.toml connectors/<id>/harness.toml
   ```
   Replace every `<PLACEHOLDER>` and delete unused options. The template is chat mode
   (`interactive = true`), the only mode: the harness opens in the participant's terminal and the
   participant types the prompt. No goal slot, identity `none`, and `since_run_start = true` on the
   native row. Every shipped connector uses it. [CONNECTOR-SCHEMA](CONNECTOR-SCHEMA.md)
   lists every 1.0 key. The sign-in must last across runs, so pick the home by one
   question, "does the harness have a variable that moves its config?":
   - **Yes** (like `CODEX_HOME`): set `home.env`, `config_home = "shared"`, `run_home = "empty"`
     (or `"keychain_link"`), a `{config_home}/...` native root, and `skills_dir` if the harness
     reads skills.
   - **No** (like Amp): leave out `home.env` and `skills_dir`, set `run_home = "shared"` and
     `config_home = "per_run"`, and use a `{run_home}/...` native root. With `run_home =
     "empty"` the private `HOME` is fresh each run and the sign-in is lost every run.

   Fill `[participant]` too. `display_name` is required. `setup_section` names the harness's
   section in [HARNESS-SETUP](HARNESS-SETUP.md), the participant's setup guide. In the same PR,
   add to that guide a row in "Where the settings live", a `### <Harness name>` section under
   "Your harness" with the settings a 168-hour run needs (from the facts sheet's "Run without
   permission prompts" section), a `### <Harness name>: the time-left tool` section, and a row
   in each numbered table you have a fact for. List the permission setting as `[[participant.no_prompts]]` and where the
   harness keeps its MCP servers as `[participant.time_left_tool]`. Goal Cup recommends both and
   writes neither (D67).

   A Node harness keeps `NODE_USE_ENV_PROXY = "1"` in `capture.env` (D59): without it Node's
   HTTP client ignores `HTTPS_PROXY` and the run records no model traffic. The kit cannot see
   that, and step 9 catches it only for a `node` script on `PATH`.

   The closest complete example is [connectors/codex-cli](../connectors/codex-cli/harness.toml)
   (chat mode). [connectors/hermes-cli](../connectors/hermes-cli/harness.toml) is chat mode too,
   with a `configure` command: a harness whose session file needs a setting that has no
   environment variable names its own command for it, and setup and start run it in the harness
   home (D61). Seeds go in
   `connectors/<id>/files/` and `capture.home.seed` must name each one; the loader refuses any other
   file in the folder. Field rules: [CONNECTOR-SCHEMA](CONNECTOR-SCHEMA.md). Check that no
   placeholder is left outside comments:
   ```sh
   grep -nE '^[^#]*<[^>]+>' connectors/<id>/harness.toml
   ```
   Expect no output.
6. **Regenerate the pin.** `sh scripts/pin-manifests.sh`. Expect no output and exit 0. Then
   `git diff --stat -- crates/*/pins` must show `crates/goalcup-protocol/pins/connector-dirs.txt`
   with one line added for `<id>`, and nothing else. Run it again after every edit under
   `connectors/<id>/`. Never edit a pin by hand.
7. **Goldens: not yours.** `crates/goalcup-cli/tests/fixtures/harness-golden/<id>.json` and
   `<id>.daemon.json` are written by a maintainer when your PR merges, so leave them out. The
   maintainer runs, one command each:
   ```sh
   GOALCUP_REGEN_HARNESS_GOLDEN=1 cargo test --offline -p goalcup-cli --test it harness_contract_golden::
   GOALCUP_REGEN_HARNESS_GOLDEN=1 cargo test --offline -p goalcup-daemon --test it harness_contract_golden::
   ```
   expects a new `<id>.json` and `<id>.daemon.json` (the second only on a Mac) and no other
   change, since the shared goldens name no connector.
8. **No kit case.** A connector needs no test or kit case of its own: the connector contract
   runs every connector through one stand-in that reads its `harness.toml` (the program, `args`,
   the native root and pattern, `skills_dir`, the CA variables, the identity). Go to step 9.
9. **Run the connector check.** `sh scripts/check-connector.sh <id>`. It runs files, pins, schema,
   both goldens, and the kit with the connector contract (both halves), on macOS and on Linux. For a new connector expect the last line
   `check-connector: <id>: 4 passed, 2 failed, 0 skipped` on macOS, and `5 passed, 1 failed, 1
   skipped` on Linux. The failures are the goldens lines, which step 7 explains. A kit or contract failure prints only `FAIL kit` in the summary. Read
   the Rust assertion above it: it names the connector, `left` is what the run captured and
   `right` is what the connector's `harness.toml` predicts.
   Fix the field that the `right` side came from.
10. **The gate is the maintainer's.** You need not run `sh scripts/gate.sh fast`; CI does not run
    the CLI tests on a PR, so the maintainer runs it (and `gate full` before a release).
11. **Review your own diff** against the [reviewer checklist](#reviewing-a-connector-pr).
    For a real trial, use the [first try](#test-a-connector) (`--dev --practice`) or the
    fresh-config local smoke there. Plain `start` with a logged-in config can select competition
    mode. Put no real trace or private research report in the PR.
12. **Open the PR** (next section).

Fast loop while fixing schema errors: `cargo test --offline -p goalcup-protocol --test it --
harness_conformance:: connector_pins::`.

<a id="what-goes-in-the-pr"></a>

## What goes in the PR

Exactly these, which `git status --short` must confirm:

- `connectors/<id>/` (the `harness.toml` and `files/**` seeds);
- the one new line in `crates/goalcup-protocol/pins/connector-dirs.txt`;
- `docs/harnesses/<id>.md`;
- the harness's section in `docs/HARNESS-SETUP.md` (step 5), named by `setup_section`.

In the description paste the last line of step 9, name anything restored or not run, say
which parts an agent wrote, and say that a person read the whole diff. Claim only what passed. The
kit shows that Goal Cup Proxy works with a stand-in shaped like the harness. It does not show that
the real harness ran. A maintainer replies within 7 days and runs the checks again on a Mac.

## When something fails

| Symptom | Where to look |
|---|---|
| A field or value is refused | [CONNECTOR-SCHEMA](CONNECTOR-SCHEMA.md), then step 3 |
| `declares id ..., which is not the folder name` | `id` must equal the folder name |
| The pin test fails after an edit | Run step 6 again and commit what it writes |
| A golden is missing (a new connector) | Expected. A maintainer writes it on merge (step 7) |

<a id="do-i-need-a-core-change"></a>

## Do I need a core change?

Each row is a closed rule in
[`capture_policy.rs`](../crates/goalcup-protocol/src/capture_policy.rs) or
[`harness.rs`](../crates/goalcup-protocol/src/harness.rs) (`check_cross_field`). Field limits are
in [CONNECTOR-SCHEMA](CONNECTOR-SCHEMA.md). Check [row 0](#row-0) first.

"Core change" means stop. Open an issue that names the row and links your facts sheet. Write no
connector until a maintainer lands the change (with a [DECISIONS](DECISIONS.md) entry when policy
moves). Never spell around a rule.

Row numbers stay fixed so other pages can cite them; retired rows are left out.

| # | Rule | Data-only fits when | Otherwise |
|---|---|---|---|
| 1 | Provider key | the harness's own login stores it in its home; Goal Cup Proxy passes none (D61) | not possible: see row 14 |
| 4 | Session identity | `none`: this run's session files are found without an id (a per-run root, or `since_run_start`) | core change |
| 5 | Home variable | `home.env` is `<WORD>_HOME` or `<WORD>_CONFIG_DIR`, or is left out | core change |
| 6 | XDG | the harness falls back to `$HOME` and needs no XDG variable | core change |
| 7 | CA variable | the harness honours its own variable ending `_CA_CERTIFICATE`, `_CA_CERTIFICATES`, `_CA_CERT`, `_CA_CERTS`, `_CA_BUNDLE` or `_CA_FILE`, or else `SSL_CERT_FILE` or `REQUESTS_CA_BUNDLE` | core change |
| 8 | Native form | the own-session file is appended JSONL or rewritten whole | core change |
| 9 | Native rows | zero to eight local native artifact sources, one per row (D74); each may be metadata-only and need not be a transcript. With none, the wire is the only record | fits |
| 10 | Executable | one entry-point file (a binary, or a script with a shebang), not an interpreter | fits |
| 11 | Endpoint names | you set no endpoint variable at all | issue, not blocking |
| 12 | Other refusals | no refused env name, argv path or custom regex is needed | core change |
| 13 | Argv and env | always, when no generic rule refuses them | refused by row 12 |
| 14 | Chat-mode sign-in | the harness keeps its sign-in in its own config home (a login command, or the macOS Keychain) | core change: chat mode cannot pass an env-only API key |
| 15 | Participant-selected model host without a config-home variable | `model_host_from` needs `home.env` and `config_home = "shared"` | facts only/core change; a shared run HOME with per-run config can use fixed host rows only |

The security lists (row 7 and the redaction regex in row 12) are closed on purpose. A CA mode
is a trust statement about the participant's machine, so the two modes and the CA-name shape are
core; the CA variable's name is connector data. A redaction regex runs inside the signed
daemon.

1. **Provider key.** A connector passes no provider key and Goal Cup Proxy stores none (D61):
   `[capture.credential]` is refused. The harness's own login keeps the key in its home.
4. **Session identity.** A chat connector takes `source = "none"`: Goal Cup Proxy reads no session
   id. The native root is private to the run, or the native row sets `since_run_start`, so every
   session file new or written since the start is this run's. Runs on a shared home are
   serialized, so that is safe (D57).
5. **Home variable.** `^[A-Z][A-Z0-9]*_(HOME|CONFIG_DIR)$`. Other tools' variables (`CARGO_HOME`,
   `NPM_*`) are refused. A harness with no such variable leaves `home.env` out. It then keeps its
   state under the private run `HOME`, so its native `root` is `{run_home}/...`, `config_home` is
   `per_run`, and it takes no seeds or `dirs`. A login in that home lasts one run, so such a harness
   keeps its login in the macOS Keychain (`run_home =
   "keychain_link"`), or uses `run_home = "shared"`.
6. **XDG.** Every `XDG_*` name is refused in `capture.env` and as `home.env`. The launch
   environment starts empty, so the harness uses `$HOME`, the private run home.
7. **CA variable.** Prefer the harness's own variable. It is handed the recording proxy CA alone:
   `mode = "extend"` when it adds to the harness's roots, `mode = "replace"` when it replaces
   them. A replace-mode variable fails closed: a connection that skips the recording proxy does not
   verify. Name `SSL_CERT_FILE` or `REQUESTS_CA_BUNDLE` (`mode = "extend"`) only when the harness
   has no variable of its own. Tool children read those too, so the run hands them the system's
   roots plus the recording proxy CA, and a direct connection by the harness then succeeds unrecorded
   ([CHEAT-CHECKLIST](CHEAT-CHECKLIST.md#anchoring-and-the-agents-reach)). `SSL_CERT_DIR`,
   `CURL_CA_BUNDLE` and `NODE_TLS_REJECT_UNAUTHORIZED` stay refused. See
   [CONNECTOR-SCHEMA](CONNECTOR-SCHEMA.md#load-checks).
8. **Native form.** `append_only`, `snapshot` or `final_snapshot`. A `final_snapshot` is a database or
   other store copied once, after the harness has exited; it is never read while the run goes. A
   database the harness writes during the run, polled live, is refused. `protocol_hint` is refused on
   `snapshot` and `final_snapshot`.
9. **Zero to eight native rows (D74).** Schema 1 admits a connector with none, which records the
   wire only, and up to eight rows. Each row needs its own `name`, and no two rows may share a
   `root` and `pattern`.
10. **Executable.** There is no `sha256` (D53). The CLI resolves `program` on `PATH`, follows
    symlinks, and runs the regular file at the end. Any build runs. A pinned `program` cannot be an
    interpreter (`python3`, `node`, `sh`, `env`). A shebang script is fine.
11. **Endpoint names.** Names ending `_URL`, `_BASE`, `_HOST`, `_HOSTNAME`, `_ADDR`, `_ADDRESS`,
    `_SERVER`, `_ORIGIN` or `_REGISTRY`, or containing `ENDPOINT`, are refused, and so is any value
    with `://`. An endpoint variable with another name is not refused by name. Never set it, record
    it in the sheet and open an issue.
12. **Other refusals.** Examples: `NO_PAGER` (the `PAGER` suffix rule), a variable that loads code
    (`PYTHONBREAKPOINT`), an absolute path in `args`, an argument that turns certificate
    verification off or names a command, hook or server, a host that is `localhost`, `*.localhost`,
    single-label or numeric, and a redaction row that is a regex rather than a typed form. In
    `capture.env`: any `*_HOME`, `*_CONFIG_DIR` or `*_CONFIG_HOME` name (the harness's own goes in
    `capture.home.env`) and any name under the connector's `reserved_env_prefixes`.
13. **Argv and env.** No list of reviewed arguments or variables exists. A refused value is
    refused with the rule's reason at step 9. What no rule can see is the reviewer's job: a dropped
    safety flag, or a value that turns self-update back on. Write in the sheet why each flag and
    variable is there. How much the harness may do (model, sandbox, permissions) is the
    participant's choice. A connector does not limit it.
14. **Chat-mode sign-in.** No profile takes a `[capture.credential]` (D61), and the chat
    launch passes only `PATH`, `TERM`, `COLORTERM`, `LANG` and `SHELL` from the participant's
    shell. So a harness that reads its provider key only from an environment variable (many
    "any model" CLIs do) cannot run. It fits only if its own login command stores the key inside
    its isolated config home, which `config_home = "shared"` keeps across runs.

<a id="reviewing-a-connector-pr"></a>

## Reviewing a connector PR

A connector is trusted executable surface. The pin catches a change to the directory after a build.
It does not make a careless seed safe. A code owner reads every field and every seed.
`harness_conformance` refuses many invalid values, but validation is not review.

1. **Seeds.** Read every `files/**` file as code. Check commands, hooks, plugins, helpers, URLs,
   endpoint settings, MCP servers, profiles and permissions. Nothing may redirect the model
   endpoint or run outside the run.
2. **`capture.args`.** No endpoint, settings indirection, native-store switch-off or absolute
   path. If argv is pre-scanned, `argv_guard` must cover every option the harness acts on, with
   abbreviations.
3. **`capture.env`.** Justify every value. `SHELL` is only `/bin/zsh`, `/bin/bash` or `/bin/sh`. No
   flag that capture needs may be dropped: one session file kept, self-update off
   (`DISABLE_AUTOUPDATER = "0"` turns it back on). No argument or variable may name a command, a
   hook, an MCP server, a host or a file outside the run. Look for telemetry exporters and code
   loaders.
4. **`capture.executable`.** `program` is the harness entry point, not an interpreter or a moving
   updater wrapper. `version_args` only asks for the version.
5. **Hosts.** With `model_host_from`: the file and key are where the harness saves the
   participant's choice (a source or a measurement in the facts sheet), the file is its config and
   not a key file, and `hosts` lists nothing the config already names. A fixed row: only the
   provider, the narrowest evidence paths and a real protocol hint. No registry or unrelated host. Auth hosts are sign-in and refresh tunnels and cannot supply model
   evidence. The protocol hint is declared, never inferred.
6. **Native capture.** The root is inside the run or config home. `{id}` scopes a shared store. The
   pattern cannot match request dumps, credential stores or seed files.
7. **Redaction.** Plan, billing, quota and key fields seen in fixtures are covered by the admitted
   recognizers. Committed fixtures went through `spikes/redact.py` and were read by eye. They hold
   no real prompt, trace or key.
8. **Home and secrets.** A private `HOME` every time. `shared` is justified and serialized.
   Per-run seeds hold no credential, URL or user data, and use safe modes. The connector passes
   no API key (D61).
9. **Identity.** `source = "none"`, and the native row reads only this run's files (a per-run
   root, or `since_run_start` on a shared home).
10. **Pin, goldens, kit.** Re-run `sh scripts/pin-manifests.sh` and confirm one pin-line diff.
    Regenerate the goldens (step 7) and confirm the diff only adds the new connector. Run the
    gate, the kit and the connector contract after a workspace build. A
    policy row or a decoder belongs in its own PR.

What each manifest field must establish:

| Field | The reviewer must establish |
|---|---|
| `capture.args`, executable | chat-mode argv and `resume_args`, entry point, version |
| `capture.home`, `env` | private HOME and config home, real state dirs, seed contents and modes |
| `capture.ca` | variable names and process scope; no secret value; no app-wide trust; the CA variable and its mode cite a source or a measurement in the facts sheet, and an own-name variable is not one that tool children also read |
| `capture.model_host_from` | the file and key the harness saves the participant's model service in; a config file, never a key file |
| `capture.hosts`, `auth_hosts` | provider host, narrow evidence paths, protocol hint |
| `capture.native` | one local native artifact source, its form, root, pattern and scope; say whether it is a transcript or metadata, and exclude request dumps and credential stores |
| `capture.identity` | `none`, and how the native row keeps to this run's files |
| `redaction.extra_patterns` | plan, billing, quota and key-fragment coverage |

## Claims

| Finding | Permitted conclusion |
|---|---|
| Unknown transport, storage, isolation, auth or identity | Blocked. Keep "unknown" and stop |
| No request or response recorded | Not qualified. Check admission, host, path and auth tunnel |
| HTTP/2, unsupported body, or missing native source | Report unsupported or partial. Keep the bytes and gaps |
| Bytes recorded, semantics unknown | Capture-only for the stated coverage. No decoder claim |
| Gap, stale recorder or unknown accounting | Report incomplete. Never call it clean |
| Task fails while the recorder passes, or the reverse | Report each outcome separately |

A passing kit is **synthetic tested** only. A maintainer may run one short real run with their own
subscription or login, with a private launch directory and the reviewed connector. A contributor
never opens a key file. That run is evidence for that build of the harness, not a release or a general
qualification.

## Getting a harness into the event

A harness joins by being merged and shipped in a release (D70). Releases continue during the cup,
so there is no cutoff: a merged connector ships in the next point release. A connector runs without
`--dev` only when a release pins it. Before that it runs only with `--dev`, which never competes.
A player has to install the
release that carries the connector before they start a run. Point releases keep
`CONTROL_API_VERSION`, so a running run is never broken by an update. To get yours in:

1. Do the [first try](#test-a-connector): one `--dev --practice` run by you, sealed and uploaded.
   Verify the sealed trace with `goalcup-verify --capture-integrity <trace>`. Send that integrity
   report (never the transcript or research NDJSON) to goalcup@agentcommunity.org or attach it to
   the PR.
2. A maintainer does one practice run of the harness too, before it ships.
3. Model access for testing: if the harness lets you pick the model provider, the organizers can
   supply test API access. If the model comes only through the harness's own subscription and there
   are no free credits, the organizers ask you how to test it.

The harness ships in the next release once both runs pass.

## Reach us

Open an issue on [goal-cup-proxy](https://github.com/agentcommunity/goal-cup-proxy/issues) for
anything public, including "this step did not work". For anything private, or a security problem,
email goalcup@agentcommunity.org ([SECURITY](../SECURITY.md)). The maintainer is Balazs Nemethi.
