# Connector schema 1

What may `connectors/<id>/harness.toml` contain? This is the public field and validation
reference for harness authors and reviewers. The procedure for adding a harness is
[HARNESS-ONBOARDING](HARNESS-ONBOARDING.md).

## The connector contract

Any harness can be added, not only the four that ship. A connector is data: one folder, no Rust.
This section is what a connector may rely on, and what Goal Cup Proxy promises not to break
while it reads schema 1 (proposed as [D68](DECISIONS.md#d68-the-connector-contract-is-locked-at-schema-1);
the owner approves it). The rest of this page is the field-by-field detail.

### What a connector may rely on

- **Schema version.** `schema_version = 1`. This build reads `{1}` only. Any other value, an
  unknown key and an unknown closed value are refused with one sentence: a newer schema needs a
  newer `goalcup`, and a misspelling is never ignored ([Parse and load](#parse-and-load)).
- **Fields.** Required: `schema_version`, `id` (the folder name), and in `[capture]`:
  `interactive = true`, `continuation` and `args`; `[capture.executable]` with `program`;
  `[capture.home]` with `run_home`, `config_home` and `real_dirs` (1 to 4 paths);
  `[capture.ca]` with `env` and `mode`; `[capture.identity]` with `source = "none"`; and a model
  host (one to 16 `[[capture.hosts]]` or `providers` rows, each host row with `host` and at least
  one path, or `model_host_from`, which with `absent = "defaults"` needs one row too). Some pairs
  are required together, such as `home.env` with a `{config_home}` root
  ([Cross-field rules](#cross-field-rules)).
  Optional, each with the default its table below gives: zero or one `[[capture.native]]` (with
  `name`, `root`, `pattern` and `form`), `[participant]`, `[launch_prep]`,
  `resume_args`, `configure`, `auth_hosts`, `env`, `platform`, `reserved_env_prefixes`, `argv_guard`,
  `termination`, `providers`, `model_host_from`, the home's `skills_dir`, `per_run_dir`, `dirs`
  and `seed`, `version_args`, and `[redaction]`. The one-screen list is
  [What a chat connector writes](#what-a-chat-connector-writes).
- **The launch environment.** The harness starts from an empty environment
  (`crates/goalcup-cli/src/launch.rs`, `build_launch`). It gets exactly:
  - from the participant's shell: `PATH` (or `/usr/bin:/bin:/usr/sbin:/sbin` when there is none),
    `TERM`, `COLORTERM`, `LANG`, `SHELL`, every `LC_*` name the env rules admit, and `USER`,
    `LOGNAME`, `TMPDIR`. Nothing else: no proxy, base URL, key or token of theirs;
  - the connector's `capture.env`, with the running platform's `capture.platform.<key>.env` over
    it, rendered. An entry here wins over the same name from the shell;
  - `HOME`, the run's home, and the connector's `capture.home.env` pointed at its config home;
  - nothing of the participant's git login, for any run. No `SSH_AUTH_SOCK`, `GIT_CONFIG_GLOBAL`,
    `GH_CONFIG_DIR`, `XDG_RUNTIME_DIR` or `DBUS_SESSION_BUS_ADDRESS` reaches the harness
    ([D71](DECISIONS.md#d71-the-project-folder-is-the-workspace-the-agent-pushes-with-the-participants-one-repo-token),
    which ends the D69 pass-throughs). Every launch also sets `GIT_TERMINAL_PROMPT=0`, `GIT_CONFIG_NOSYSTEM=1` (the system
    git config can name a keychain helper) and `GIT_SSH_COMMAND` as an `ssh` that reads no config
    and offers no key; a connector cannot name a `GIT_` variable. The agent pushes with the token in
    the project folder's `.env.local`, and sets its own git identity in its repository;
  - for a recorded launch: `HTTPS_PROXY` and `https_proxy` (the loopback recording proxy, with
    a per-run credential), `NO_PROXY` and `no_proxy` (`127.0.0.1,localhost`), each `capture.ca.env`
    name pointed at the file its row names ([Credential and CA](#credential-and-ca)),
    `GOALCUP_IN_HARNESS=1` (`goalcup seal` refuses under it) and `GOALCUP_TIME_LEFT_FILE` (the
    run's time-left file, D66).

  `goalcup setup`, `capture.configure` and `[launch_prep]` commands get the same environment with
  no proxy, no CA variable and no `GOALCUP_*` variable: they record nothing. The `version_args` probe gets only
  `PATH` and a throwaway `HOME`. The harness runs in the run's workspace, on the participant's
  terminal (stdin, stdout and stderr inherited). Its argv is `args`, then `resume_args` on a
  `goalcup continue` that finds a session file in its native store (or cannot read the store,
  and so asks the harness for its last session). A connector never sets `GOALCUP_*`, a proxy, a
  CA variable, `HOME` or `PATH` itself: each is refused in `capture.env`, among other names
  ([Arguments and environment policy](#arguments-and-environment-policy)).
- **Homes.** `config_home = "shared"` is `<config dir>/capture-homes/<id>/`, kept across runs,
  and runs on it are serialized; `per_run` is made fresh in the run folder. `run_home` is
  `empty` (a fresh private `HOME` each run), `keychain_link` (a fresh `HOME` whose one entry links
  `Library/Keychains` to the participant's, for a macOS Keychain login) or `shared` (one kept
  `HOME` per connector at `<config dir>/capture-homes/<id>.run-home/`, D59). Every run `HOME`
  also gets a `.profile` that restores `PATH` for a login shell; in a shared run `HOME` Goal
  Cup's own `.profile` is rewritten each run, and any other `.profile` there is kept. With `skills_dir`, the goal-cup skill is put in `<config home>/<skills_dir>/goal-cup/`
  before each launch. See [`[capture.home]` and seeds](#capturehome-and-seeds).
- **Native roots and templates.** A native root is `{config_home}/...` or `{run_home}/...`; a
  missing `{config_home}` root (and a `{run_home}` root of a shared run home) is created mode 0700.
  `since_run_start = true` records every matching file new or written since the run started.
  Argument templates admit `{workspace}`; `capture.env` values also admit `{config_home}` and
  `{run_home}`.
- **Hosts.** Only the hosts the connector declares are opened and recorded: its `capture.hosts`
  rows, the rows its `providers` names from the shared table (`providers.toml`), and, with
  `model_host_from`, the host its own config names, read at each start and continue (through one
  `via` indirection, and with `absent = "defaults"` the connector's rows alone when the config
  names none). Every other host is an opaque tunnel, listed at finish and sent with the seal
  (D74). `auth_hosts` are always tunnels.
- **`[participant]`.** `display_name`, `first_screen`, `setup_section`, `no_prompts` and
  `time_left_tool`. Core messages name a harness only through these. Goal Cup Proxy reads the
  `no_prompts` and `time_left_tool` files and never writes them (D67).
- **Decoder hooks.** A connector names its byte format with `protocol_hint`, on a host row or an
  `append_only` native row, in the open grammar `^[a-z][a-z0-9.-]{0,63}$` (`hints.rs`). It
  needs no decoder. A hint that no decoder accepts is recorded as capture-only: the bytes are
  sealed, `goalcup doctor` and the research report name the hint, and the integrity verdict is
  unchanged (D43, D45). A decoder is a separate verifier PR: a `DecoderV1` registered in
  `DecoderRegistry::builtin()` whose `accepts` sees the whole `StreamOpen` (source, hint, content
  type) ([writing-a-decoder](../skills/adding-a-capture-harness/writing-a-decoder.md)). Adding
  one changes no connector and no pin. `model_host_from` paths carry the built-in wire hints
  (see below).

### The compatibility promise

While Goal Cup Proxy reads schema 1:

1. A connector that loads today keeps loading, under the same rules (`Origin::Pinned` once it is
   pinned, `Origin::Dev` under `--dev`). The one exception is a tightened core policy row (a
   deny list or trust table, for safety): it follows [the deprecation rule](#the-deprecation-rule),
   with notice and a warning before it refuses, unless the owner rules that waiting is unsafe.
2. Every field above keeps its meaning and its default. A closed value is not removed.
3. The launch keeps setting every name listed above with the same meaning, and takes nothing
   more from the participant's shell than that list. A later Goal Cup variable goes under
   `GOALCUP_`, which a connector cannot use.
4. The home locations (`capture-homes/<id>/` for a shared config home,
   `capture-homes/<id>.run-home/` for a shared run `HOME`) and the native root templates keep their meaning,
   so a participant's login and settings survive an upgrade.
5. A declared hint is recorded as declared. A new decoder never needs a connector change.
6. A shipped `providers` row is not removed or narrowed. The table is frozen (D74): no row is
   added either, and a connector lists its default hosts in its own `[[capture.hosts]]`.

Schema 1 may still grow: a new optional field whose absence keeps today's behaviour, a wider
bound, a new closed value or a newly admitted pair of values. None of those breaks a connector.
A connector that uses a widened bound (zero native rows, D74) needs a binary that admits it; an
older binary refuses it with its own message.

The promise is held by tests. Every shipped connector, and five frozen fixtures for harnesses
nobody ships (`crates/goalcup-cli/tests/fixtures/schema-1-connectors/`), take the connector
contract. The fixtures are a plain chat shape (`schema1-chat`); a shared config home with
`model_host_from`, `providers`, every `[participant]` field, `configure`, platform env, a
reserved prefix, an argv guard, `native_pid` and redaction rows (`schema1-optional`); a per-run
config home with `per_run_dir`, `dirs` and seeds, a `keychain_link` run home, a shared CA row
and `prefix_paths` (`schema1-per-run`); one host row and no native row, so the wire is the only
record (`schema1-wire-only`); and the required fields only, with no home variable and
a shared run `HOME` (`schema1-minimal`, which pins the defaults). They load, take the launch checks
(`tests/it/connector_contract.rs`) and run once with a stand-in through the real binaries
(`tests/full/connector_contract.rs`; the fixtures through `--dev`). A connector made from the
[template](../skills/adding-a-capture-harness/templates/connector-harness.toml) loads with no
Rust change (`goalcup-protocol` `connector_template.rs`), and no core string literal names a
harness (`no_family_literals.rs`). A change that makes one of these fail is a break of the
contract, not a test to update.

### The deprecation rule

A field, value, default or launch variable is retired only like this:

1. **A DECISIONS entry** names it, why it goes, what replaces it and the release that stops
   reading it. A tightened core policy row (a deny list, a trust table) follows the same rule; it
   may skip the notice and the warning when the owner rules that waiting is unsafe.
2. **Notice.** It keeps working for at least one released `goalcup` after the entry, and at
   least 14 days. Nothing is retired between an event's last release and the end of its runs.
3. **Warning.** During the notice, `goalcup doctor` and `scripts/check-connector.sh` say that the
   connector uses it and what to write instead. (No connector field is deprecated today, so this
   warning does not exist yet: the first deprecation adds it.)
4. **Schema bump.** Removing or renaming a field, changing a default or narrowing a bound is
   `schema_version = 2`. The same PR migrates every connector under `connectors/`, regenerates
   the pins, makes the schema-1 refusal name the change, adds schema-2 fixtures, and turns the
   schema-1 fixtures into checks that schema 1 is refused with that sentence. A tightened policy
   row keeps schema 1: its PR migrates every connector under `connectors/` it would refuse. A
   participant's connector outside the repository is covered only by the notice and the
   warning, which is why they come first.

Before this contract, three removals happened inside schema 1 with no notice: the executable pin
keys (D53), `[capture.credential]` (D61) and headless mode (21 Oct). Each is refused with a
sentence that names the field and says what to do. The rule above applies from D68 on.

### Pinned, or `--dev` only

A connector runs without `--dev` only when this `goalcup` build pins its folder
([`connector-dirs.txt`](../crates/goalcup-protocol/pins/connector-dirs.txt), compiled in). So a
participant's own harness needs its connector PR merged and a `goalcup` release that pins it.
Releases continue during the cup (D70), so a merged connector ships in the next point release
([event qualification](HARNESS-ONBOARDING.md#getting-a-harness-into-the-event)). An unpinned
connector runs only through `goalcup start --dev`. That load (`Origin::Dev`,
`harness/check.rs`) relaxes three rules: the interpreter refusal for `program`; the argument
path rules (an absolute, home-relative, parent-relative or expanding path in `args` or
`resume_args`), with the word rules skipped for an argument that is an absolute path; and the
loopback and single-label host refusal for `hosts` and `auth_hosts`. Every other rule holds,
and it refuses a directory whose id or digest a build pins. The start says "This run is a development run and its results are not a judged entry's", and the local
entry record says `dev`. A `--dev` run never competes (D70). The CLI refuses `--dev` on a competition start
before the approval, sends `dev` when it opens or continues a server run, and the server refuses
a `dev` competition run. Any member may run `--dev` for a practice or test run; those count for
nothing. The server stores the connector id and directory digest and compares neither with a
list. The verifier's research report labels the run `Dev/unpinned` (`run.connector`), and the
integrity verdict does not change.

To be pinned, a connector also needs what `harness_conformance.rs` checks for every folder under
`connectors/`, beyond the schema: a `[participant]` table with `display_name`, a `setup_section`
whose heading is in [HARNESS-SETUP](HARNESS-SETUP.md), and an `id` equal to its folder name.

## What a chat connector writes

Every shipped connector runs its harness in chat mode (D57), and a new one does too. This is all
a chat connector writes. The [template](../skills/adding-a-capture-harness/templates/connector-harness.toml)
has each line with its comment. The full reference below has every rule.

| Field | What to write | Example |
|---|---|---|
| `schema_version`, `id` | `1`, and the folder name | `id = "codex-cli"` |
| `[participant]` | `display_name`; `first_screen` only if the harness asks something first; `no_prompts` when a setting in its config stops its permission prompts (D67); `setup_section`, its section in [HARNESS-SETUP](HARNESS-SETUP.md); `time_left_tool`, where its MCP servers are listed | `display_name = "Codex"` |
| `capture.interactive`, `continuation` | `true`, and the fixed chat sentence | `continuation = "chat: the participant types the prompt and may type keep going"` |
| `capture.args`, `resume_args` | Options to open the chat (often none), and how to reopen the last session | `args = []`, `resume_args = ["--continue"]` |
| `[capture.executable]` | The program name on `PATH`, and its version option | `program = "claude"`, `version_args = ["--version"]` |
| `[capture.env]` and `[capture.platform.<key>.env]` | `SHELL`, `NODE_USE_ENV_PROXY = "1"` for a Node harness, the self-update switch; `SHELL = "/bin/bash"` on Linux | `DISABLE_AUTOUPDATER = "1"` |
| `[capture.home]` | `run_home`, `config_home`, the harness's home variable, its real folders, `skills_dir` | `config_home = "shared"`, `env = "CODEX_HOME"` |
| `[capture.ca]` | The CA variable the harness reads, and `extend` or `replace` | `env = ["NODE_EXTRA_CA_CERTS"]`, `mode = "extend"` |
| `[capture.identity]` | Always `source = "none"` | `source = "none"` |
| `[[capture.native]]` | Zero or one. The session files: `name`, `root`, `pattern`, `form`, `protocol_hint`, `since_run_start = true` | `root = "{config_home}/sessions"` |
| Model host | Default host rows (`[[capture.hosts]]` or `providers`), plus `model_host_from` when the harness's config may name another service | `model_host_from = { file = "settings.json", key = "env.ANTHROPIC_BASE_URL", absent = "defaults" }` |
| `capture.auth_hosts` | Sign-in hosts, tunnelled and never recorded | `auth_hosts = ["auth.openai.com"]` |
| `[capture.termination]` | `process_group`, or `native_pid` for an npm wrapper; a grace in seconds | `strategy = "process_group"`, `grace_seconds = 10` |
| `[redaction]` | Optional: plan, quota and billing fields the harness sends | `{ kind = "codex_plan_type", json_field = "plan_type", value = "string" }` |

Only when needed: `capture.configure` (a setting with no environment variable, as Hermes has),
`[launch_prep]` (a harness that keeps a process of its own between launches or trusts a CA file
only through its own config, as Hermes 0.22 does),
`capture.argv_guard` and `capture.reserved_env_prefixes` (a harness that scans its own options or
has a safe-mode variable). A connector never writes `goal_flag`, an identity `binding` or
selector, or `[capture.credential]`: each is refused. Seeds (`capture.home.seed`) are allowed but no shipped
connector uses them.

The executable authority is
[`harness.rs`](../crates/goalcup-protocol/src/harness.rs). Its
[`capture_policy.rs`](../crates/goalcup-protocol/src/capture_policy.rs) trust tables,
[`hints.rs`](../crates/goalcup-protocol/src/hints.rs) hint grammar and
[`pins.rs`](../crates/goalcup-protocol/src/pins.rs) pin admission rule are part of this contract.
The one-test-per-refusal table is
[`harness_schema.rs`](../crates/goalcup-protocol/tests/it/harness_schema.rs). If this reference and
those sources disagree, the sources refuse the manifest.

## Parse and load

`harness.toml` is UTF-8 TOML. The loader first reads `schema_version` from a generic table and refuses a
`[capture.credential]` table there (D61), then
deserializes every table with unknown fields denied, appends the `capture.providers` rows to
`capture.hosts`, then validates in this fixed order:
executable; arguments and goal; configure commands; launch prep; environment; home; CA; identity;
native source; argument guard; termination; model host; hosts; redaction; delivery independence; cross-field
rules; then each `capture.platform` entry, in the fixed order `linux-x86_64`, `linux-aarch64`,
`macos-aarch64`, `macos-x86_64`, whatever order the file uses. The first refusal is deterministic. Unknown keys and
unknown closed values use the same message: a newer schema needs a newer `goalcup`, while a
misspelling is refused rather than ignored. The retired executable pin keys (`sha256`, `version`)
are the exception: they are refused before deserialization with their own sentence (D53).

All public schema structs and enums are non-exhaustive. A manifest cannot select its load origin:

- `Origin::Pinned` is an in-tree or pin-checked connector. It refuses an interpreter as the
  executable and refuses an absolute, home-relative, parent-relative or expanding path in an
  argument. There is no per-connector list in Rust (D51): `capture.args` and
  `capture.env` are held to the generic rules below, and a new connector, variable or argument,
  a model name included, is a data change reviewed in its PR. An unpinned id is refused at the
  pin gate and runs only under `--dev`.
- `Origin::Dev` is an unpinned directory under `GOALCUP_CONNECTORS_DIR` selected through
  `--dev`. It relaxes those two refusals and the loopback-host refusal. Environment values, seeds,
  trust tables, all bounds and all cross-field rules remain strict.

The CLI and daemon both use the same loader and each validates again at its own trust boundary.

## Top level and `[capture]`

An omitted required field is a TOML error. “Default” below means the value applied when an
optional field or table is absent.

| Field | Required/default | Contract |
|---|---|---|
| `schema_version` | required | Integer `1`; this build's closed supported set is `{1}`. |
| `id` | required | 2–48 bytes; `^[a-z][a-z0-9-]{1,47}$`. |
| `capture` | required table | The complete launch-and-capture profile below. |
| `participant` | optional, default absent | `[participant]`: the harness's display name and first-screen hint for messages to the participant; see below. |
| `redaction` | optional, default absent | Connector-selected rows from the core redaction mirror. |
| `launch_prep` | optional, default absent | `[launch_prep]`: a CA file and commands run in the harness home before each launch, and commands run after the harness exits (D72). See [Launch prep](#launch-prep). |
| `capture.args` | required | 0–64 strings in chat mode; each at most 4096 bytes and without control characters. |
| `capture.goal_flag` | refused | Headless mode's goal option; refused as chat mode only (see below). |
| `capture.interactive` | required: `true` | Boolean. `true` runs the harness in its chat mode in the participant's terminal, where the participant types the prompt (D57). `false`, or leaving it out, is refused as chat mode only (see below). |
| `capture.resume_args` | optional, default `[]` | Strings appended after `capture.args` when a run is continued and already has a file in one of its `[[capture.native]]` stores, or has no native row; otherwise the harness opens the normal way. Each is held to every `args` rule, and `args` and `resume_args` together hold at most 64. |
| `capture.configure` | optional, default `[]` | `interactive` only. At most 4 commands, each 1–16 arguments of 1–256 bytes. Each argument is held to every `args` rule and takes no template (`{` and `}` are refused). See [Configure commands](#configure-commands). |
| `capture.continuation` | required | Non-blank, one line, 1–512 bytes; declarative provenance for unattended continuation. |
| `capture.auth_hosts` | optional, default `[]` | At most 16 unique lowercase DNS hosts; always opaque tunnels. |
| `capture.env` | optional, default `{}` | At most 32 string-to-string entries; the child-only process environment additions. |
| `capture.reserved_env_prefixes` | optional, default `[]` | At most 4 names of the form `^[A-Z][A-Z0-9_]{0,30}_$`; `capture.env` may set no name under them. See the environment policy below. |
| `capture.executable` | required table | The executable name on `PATH`, and how to ask it its version. |
| `capture.platform` | optional, default `{}` | Per-platform env entries; see below. |
| `capture.home` | required table | Run `HOME`, isolated harness home and any seeds. |
| `capture.credential` | refused (D61) | A connector may not pass a provider key; the harness keeps its own login in its home. |
| `capture.ca` | required table | The harness's CA variables and which file they get. |
| `capture.identity` | required table | How this run's native-session identity is obtained and bound. |
| `capture.native` | optional, default `[]` | Zero or one `[[capture.native]]` table (D74). |
| `capture.argv_guard` | optional, default absent | Harness-specific pre-parser argument refusals. |
| `capture.termination` | optional; effective defaults below | How the owned harness is stopped. |
| `capture.hosts` | defaults to `[]`, validation requires 1–16 (with `providers`), or 0–16 with `model_host_from` and `absent = "refuse"` | Intercepted model hosts and their recorded paths. |
| `capture.providers` | optional, default `[]` | Unique names from the shared provider table; each adds that provider's host row. See below. |
| `capture.model_host_from` | optional, default absent | `{ file = "<path>", key = "<dotted key>" }`, with optional `via = "<dotted key>"` and `absent = "refuse"` (default) or `"defaults"`: where the harness's own config names its model service. Requires `config_home = "shared"`. See [The model host from the harness's config](#the-model-host-from-the-harnesss-config). |

With `interactive = true` (D57) there is no goal slot: the participant types the prompt in the
harness. Its `args` may be empty, and its `identity.source` is `none`. Every argument and every `resume_args` entry passes the core deny policy and the
optional argument guard. `goalcup continue` appends `resume_args` after `args`.

**Headless mode was removed (21 Oct, simplification plan A1).** It gave the harness one goal as an
argument and named a session through the `preassigned`, `stdout_json` or `stderr_line` identity
sources. A manifest that still says `interactive = false` (or leaves it out), or writes
`goal_flag`, one of those sources or any of their identity fields (`binding`, `match_field`,
`match_value`, `id_field`, `prefix`, `grammar`, `wait_seconds`), is refused with one "chat mode
only" message that names the field. It stays `schema_version = 1`.

### Configure commands

`capture.configure` (D61) is for a harness whose native source needs a setting that has no
environment variable (Hermes writes `sessions/session_<id>.json` only with
`sessions.write_json_snapshots` on, and keeps writing it after a compaction only with
`compression.in_place` off). Each entry is an argument list for the harness's own
`capture.executable.program`. Only a chat-mode profile (`interactive = true`) takes it: the
commands run with the environment, and in the home, that `goalcup setup` opens the harness with.

```toml
configure = [
  ["config", "set", "sessions.write_json_snapshots", "true"],
  ["config", "set", "compression.in_place", "false"],
]
```

- **When.** `goalcup setup` runs them in the harness home after the participant's own setup, once
  the harness it opened has exited 0. `goalcup start` and `goalcup continue` run them again before
  each launch, after the goal-cup skill is put in place, because the participant may have changed
  the setting since. So each command must be safe to run again.
- **How.** In order, with exactly the environment the harness gets from prep (the run's private
  `HOME`, the harness home variable, the connector's `capture.env`, the chat-mode pass-through) and
  no proxy and no CA variable: a configure command records nothing. Stdin is `/dev/null`, each
  leads a process group of its own, and each has 60 seconds. Stdout and stderr are appended to
  `<cfg>/prep.log` (prep) or the run's `host.log` (start); nothing reaches the terminal or the
  trace.
- **Failure.** A command that exits non-zero, is ended by a signal or outlives its 60 seconds
  stops at once. Prep marks the home not ready (`[fail] harness_home`, naming the command and how
  it ended); start launches nothing and says to run `goalcup setup --harness <id>` again.
- **What it may change.** Only how the harness stores its session (capture and storage settings), in
  the harness home. Never how the agent behaves: not the model, the provider, the tools, the
  prompts, or whether compression is on. Core refuses an argument that has `model`, `provider`,
  `base_url`, `api_key`, `key`, `token` or `secret` as a `.`- or `=`-separated part (compared in any
  case, leading dashes ignored: `model.provider`, `--model`, `--model=x`, `provider=x` and
  `auth.Token` are refused; `max_tokens` is not). The `args` rules also refuse the endpoint,
  credential, proxy and trust spellings, a profile switch and the connector's own guarded options.
  Review is the control for the rest.

### Launch prep

`[launch_prep]` (D72, schema 1, additive) is for a harness that keeps a process of its own
between launches (Hermes 0.22's gateway, one per home, which keeps the environment of whoever
started it) or that trusts a CA file only through its own config (Hermes 0.22 on macOS reads
`HERMES_CA_BUNDLE` no more; a provider entry's `ssl_ca_cert` works). It is a top-level table.

```toml
[launch_prep]
ca_file = "recorder-ca.pem"
commands = [
  ["config", "set", "providers.recorder.base_url", "{model_base_url}"],
  ["config", "set", "providers.recorder.ssl_ca_cert", "{ca_file}"],
  ["gateway", "stop", "-y"],
]
after_exit = [["gateway", "stop", "-y"]]
```

- **`ca_file`** (optional). One file name in the config home, of `[A-Za-z0-9._-]` (at most 64
  bytes), ending in `.pem`, that does not name the tracker. At every start and continue goalcup
  writes the run's recording-proxy CA there, alone, as PEM, mode 0600, through a new temporary
  file and a rename, so a link at that name is replaced and never followed. If that write fails,
  the start is refused.
- **`commands`** (optional, default `[]`). At most 4, each 1–16 arguments of 1–256 bytes, held
  to every `capture.configure` rule, with one exception. Two templates exist, and each must be a
  whole argument: `{model_base_url}` (the URL `capture.model_host_from` read: `https://`, the
  host, `:port` unless 443, and the path; never a userinfo or a query) and `{ca_file}` (the
  absolute path of `ca_file`). A template is admitted only right after its key: an argument with
  no `=` whose last `.`-separated part is exactly `base_url` (before `{model_base_url}`) or
  `ssl_ca_cert` (before `{ca_file}`). That key is then admitted although the deny lists refuse its
  word (`baseurl`, `cacert`), and only that word: `model.base_url` is still refused for `model`,
  and `providers.x.base_url=evil.example` or `--base-url` is refused as before. `{model_base_url}` needs
  `capture.model_host_from`, and `{ca_file}` needs `ca_file`. Any other brace is refused.
- **When and how.** At every `goalcup start` and `goalcup continue`, after `capture.configure`
  and before the launch, never at `goalcup setup`: the CA file first, then each command in order,
  run as configure commands run (the harness home, no proxy, stdin `/dev/null`, 60 seconds each,
  output to `host.log`).
- **Failure.** A command that cannot start, ends non-zero, is ended by a signal or outlives its
  60 seconds does not stop the run: one line in `host.log` names it in its template form (never
  with the URL or the path filled in) and the launch goes on. The harness's own output, which also
  goes to `host.log`, may repeat the URL or the path. An older harness may lack a
  command (Hermes 0.19.0 has no `-y` on `gateway stop`).
- **`after_exit`** (optional, default `[]`, added 26 Oct 2026, D72). Commands run each time
  the harness exits, as `commands` run but with 15 seconds each. After its own quit, a crash or a
  signal they run before goalcup waits for the recording proxy to drain and seals the part. After
  goalcup's stop at the deadline they run once the proxy was stopped within its short bound and
  the part sealed, and only end what the harness left. Held to every `capture.configure` rule,
  with no template at all (the two above included). A failure is one line in `host.log` and never
  stops the run; goalcup then stops the proxy within its short bound (2 seconds) instead of
  waiting on the open connections, and tells the participant in one line only if it cut one. Hermes stops its gateway here, which
  otherwise keeps the proxy's connections open.
- **What it may change.** Only the harness's trust of the recording proxy for the URL its own
  config already names, and processes of its own that would keep an old environment. The pair
  rule means the only value a `base_url` or `cacert` key can get is goalcup's own: the URL the
  participant chose, and the run's CA (D1 stands: where the harness sends its calls does not
  change).

Argument templates (`args` and `resume_args`) admit only `{workspace}`.
`capture.env` value templates also admit `{config_home}` (the isolated harness home) and
`{run_home}` (the run's private `HOME`), so a harness with a second state variable can point it
inside them (`DATA_DIR = "{config_home}/data"`). The value rules still apply around a token: no
leading `/` or `~`, no `$`, backtick or `://`, and no `..` component. `{{` and `}}` render literal
braces. Any other token or unmatched brace is refused (`{session_id}` included: no run names
a session).

### Executable

| Field | Required/default | Contract |
|---|---|---|
| `capture.executable.program` | required | Bare `PATH` name, 1–64 bytes of `[A-Za-z0-9._-]`; no path. |
| `capture.executable.version_args` | optional, default `[]` | Empty, or exactly one of `--version`, `-v`, `-V`, `version`: the probe runs before the capture, with no proxy and no record. |

No build is pinned (D53). `goalcup start` resolves `program` on `PATH`, follows the symlinks above
the file, refuses a symlink or anything but a regular file at the end, and runs it; the CLI
reads none of its bytes, and a file replaced between resolve and release is simply a different
build. The daemon records the build that ran in `Launch.harness_build`. With `version_args`,
the launcher runs `<program> <version_args>` once before the harness, with an empty environment
except `PATH` and a throwaway `HOME` beside the run's own (made for the probe and removed after
it, so whatever the probe writes never reaches the run's `HOME`), stdin `/dev/null` and a ten-second timeout, and records
the first line of its stdout (trimmed, printable ASCII; a longer line is cut to its first 64
bytes, never refused) as `Launch.harness_version`; without them, or when that fails, it records
`unreported`. The version
is reported by the harness, not proven.

`capture.executable.sha256` and `capture.executable.version` (and the same keys in a
`capture.platform.<key>` table) are retired: a manifest that still carries one is refused with
one sentence telling the author to delete the line.

A pinned connector's `program` cannot be an interpreter or launcher named by
`INTERPRETER_PROGRAMS`. Matching is ASCII-case-insensitive and also catches a listed name with a
numeric version suffix, such as `python3.13`. The current table includes shells, `env`, privilege
and command wrappers, script runtimes, package launchers, `git`, `find`, `make`, Java and
PowerShell. A development kit case may use one.

### Per-platform environment

`[capture.platform.<key>]` may add env entries for one platform. Nothing else differs between
platforms; the executable is the same `program` everywhere.

| Field | Required/default | Contract |
|---|---|---|
| `<key>` | closed set | `linux-x86_64`, `linux-aarch64`, `macos-aarch64` or `macos-x86_64`: Rust's `std::env::consts::OS` and `ARCH`, joined by `-`. Any other key is refused. |
| `capture.platform.<key>.env` | optional, default `{}` | Entries added to `capture.env`; one with the same name replaces the base entry. |

Any other field inside an override, such as `program`, `args`, `home` or `hosts`, is refused as
not part of schema 1. The running platform's entry applies to `goalcup start`. Without one,
including on a platform outside the closed set, the base `capture.env` applies.

Each entry is validated as the whole recipe it resolves to: the base table with that platform's
env applied. Every rule in this document applies to that recipe, including the env deny lists,
value rules, bounds and cross-field rules. An override cannot set a variable, value or name that
the base table could not. A refusal names `capture.platform.<key>` (or `.env`) and keeps the base
rule's reason.

```toml
[capture.executable]
program = "codex"
version_args = ["--version"]

[capture.env]
SHELL = "/bin/zsh"

[capture.platform.linux-aarch64.env]
SHELL = "/bin/bash"
```

### Arguments and environment policy

`capture.args` refuses endpoint or provider selection, credentials, proxy or trust overrides,
telemetry export, code injection, settings/profile indirection, remote execution and switches
that disable the native session store. Under `Origin::Pinned`, it also refuses a path that:

- starts with `/` or `~`, or has either immediately after `=`, quote, `:`, `,`, `[`, `(`, space
  or `@`;
- attaches `/` or `~` to a one-to-four-character short option;
- has a `..` component; or
- contains a shell-style `$NAME` or `${NAME}` expansion.

That `@` delimiter is the R-1 boundary: `@/tmp/args` is treated as an `@file` response-file
path and refused for a pinned connector. It is not a new template form. `Origin::Dev` skips this
argument-path check so an offline stand-in can receive an absolute fixture path.

Each `capture.env` key is 1–128 bytes, begins with an ASCII letter or `_`, and otherwise contains
only ASCII letters, digits or `_`. Names compare uppercase for policy. The validator refuses:

- endpoint, proxy, credential and provider names (the one proxy name admitted is
  `NODE_USE_ENV_PROXY`, D59);
- the launch-pinned `HOME`, `PATH`, `USER`, `LOGNAME` and `TMPDIR`;
- CA variable names (the shared rows and the CA-name endings) and global trust overrides;
- injection/startup variables and names ending in command suffixes such as `_COMMAND`, `_CMD`,
  `EDITOR`, `PAGER`, `ASKPASS`, `_WRAPPER` or `_RUNNER`;
- injection and provider prefixes, including `DYLD_`, `LD_`, `OTEL_`, `CLAUDE_CODE_USE_`,
  `GIT_`, `NPM_CONFIG_`, `PIP_`, `XDG_`, `GOALCUP_` and the cloud-provider prefixes;
- any `*_HOME`, `*_CONFIG_DIR` or `*_CONFIG_HOME` name: a harness's own relocator is
  `capture.home.env`, and any other would move some program's state out of the run's isolation;
- any name pinned by this manifest's home or CA fields; and
- any name under one of this connector's `capture.reserved_env_prefixes` (up to 4, each
  `^[A-Z][A-Z0-9_]{0,30}_$`): the harness's own namespace, declared as data, so that switches
  such as Hermes's `HERMES_SAFE_MODE` cannot silently stop capture. No harness is named in the
  policy itself.

R-3 is an intentional over-refusal: `NO_PAGER` ends in `PAGER`, so schema 1 refuses it even when
the harness treats it as a boolean. A harness that needs that name requires a reviewed core policy
change; spelling around the rule in a connector is not allowed.

Each environment value is at most 1024 bytes and contains no control character. `SHELL` is the
only path-valued exception and must be exactly `/bin/zsh`, `/bin/bash` or `/bin/sh`.
`NODE_USE_ENV_PROXY` must be exactly `1`: it names no endpoint, it makes a Node harness honour the
`HTTPS_PROXY` the launch pins (D59). Every other
value is refused if it starts with `/` or `~`, contains `$`, a backtick or `://`, or has a `..`
path component. `Origin::Dev` does not relax environment values.

## `[capture.home]` and seeds

| Field | Required/default | Contract |
|---|---|---|
| `capture.home.run_home` | required | Closed values: `empty`, `keychain_link`, `shared`. `shared` only without `capture.home.env` (D59). |
| `capture.home.config_home` | required | Closed values: `shared`, `per_run`. |
| `capture.home.env` | optional, default absent | 1–64 bytes; `^[A-Z][A-Z0-9_]{0,63}$`, then the home-variable policy below. |
| `capture.home.per_run_dir` | optional; effective default `harness-home` | `per_run` only; 1–32 bytes; `^[a-z0-9][a-z0-9-]{0,31}$`. |
| `capture.home.dirs` | optional, default `[]` | `per_run` only; at most 8 relative paths, created mode `0700`. |
| `capture.home.seed` | optional, default `[]` | `per_run` only; at most 16 seed rows. |
| `capture.home.real_dirs` | required | 1–4 paths of the form `{user_home}/<relative>`; the daemon refuses a config home at or below them. |
| `capture.home.skills_dir` | optional, default absent | The directory under the config home where the harness reads its skills; a relative path of at most four components. Declarative: the daemon does not use it. |
| `capture.home.seed[].file` | required | Source below connector `files/` and the same destination below the config home. |
| `capture.home.seed[].mode` | optional, default `0600` | Closed values: `0600`, `0400`. |

Each `dirs`, seed `file`, `skills_dir` and `real_dirs` relative suffix has at most four, four,
four and eight path components respectively. A component is 1–64 bytes of `[A-Za-z0-9._-]` and is not `.` or `..`.
Seed and directory paths are refused case-insensitively if they name startup files, a `*.pth`
file, credential or tool-loaded directories, or `.config/git`. Seed destinations are unique.
Each seed file must exist, be regular and be at most 65,536 bytes. The directory walk reads and
hashes its bytes once; the per-run-home writer uses those captured bytes rather than reopening the
connector. Under every origin a seed's bytes must be UTF-8 text and, outside full-line `#` or `//`
comments, hold no URL (`://`), no backslash, none of the endpoint, `url`, `host`, provider,
proxy, trust, keylog, telemetry and `notify` spellings (compared lowercase with `-` and `_`
dropped), and no run of 32 or more `[A-Za-z0-9_-]` characters, the shape of a key or token, a
UUID or a git revision (`capture_policy::seed_content_problem`). It is a spelling check; the
reviewer still reads every seed.

`capture.home.env` must have the shape of one harness's own home variable: one uppercase word
followed by `_HOME` or `_CONFIG_DIR`, such as `CODEX_HOME`, `HERMES_HOME` or
`CLAUDE_CONFIG_DIR`. Other tools' home/config/cache variables, `SHELL`, and the shared name-deny
rules are refused.

Without `capture.home.env` the launch points no variable at the config home, so the harness keeps
its state under the private run `HOME`. Then `config_home` must be `per_run`, `seed` and `dirs`
must be empty, and the native `root` must be `{run_home}/...`: anything else would be a home the
harness never reads, admitted to record nothing.

That run `HOME` is fresh each run, so a login made before the run is lost. `run_home = "shared"`
keeps it instead (D59): one empty directory per connector, made by Goal Cup Proxy and kept across
runs, held to the rules of a shared config home. Runs on it are serialized; the daemon refuses it
at the participant's `HOME`, at a `real_dirs` path or anywhere but directly under
`<config_dir>/capture-homes`; the baseline lists it in place of the config home; a missing
`{run_home}` native root is created as a `{config_home}` root is; a `{run_home}` native root with
identity `none` needs `since_run_start = true`. It is refused with
`capture.home.env`: such a harness shares its config home instead.

Every run `HOME` gets one file from Goal Cup Proxy, whatever the connector: `.profile`, mode
`0600`, exporting the `PATH` the launch hands the harness (shell-quoted). A harness that runs its
commands in a login shell (Codex's `/bin/sh -lc`) has them run `/etc/profile`, which on Debian and
Ubuntu resets `PATH`; this puts the participant's back (D57). In a shared run home, a `.profile`
Goal Cup wrote (its first line says so) is rewritten each run, so the latest run's `PATH` wins;
any other `.profile` there (the participant's own, or a link) is kept as it is.

## Credential and CA

A connector passes no provider key, and Goal Cup Proxy stores none (D61). The harness's access
token is forwarded unread and never written ([README](../README.md#the-access-token)). A `[capture.credential]` table, in the base profile or in a
`[capture.platform.<key>]` override, is refused with "a connector may not pass a provider key;
the harness keeps its own login in its home". A harness that reads its key from an environment variable only is set up with its own
login command, which stores the key in its home.

| Field | Required/default | Contract |
|---|---|---|
| `capture.ca.env` | required | 1–2 unique CA variable names (below). |
| `capture.ca.mode` | required | Closed values: `extend`, `replace`. What the harness's own variable does with the file. |

A CA variable name is one of:

- the harness's own variable: upper case, ending `_CA_CERTIFICATE`, `_CA_CERTIFICATES`,
  `_CA_CERT`, `_CA_CERTS`, `_CA_BUNDLE` or `_CA_FILE` (`NODE_EXTRA_CA_CERTS`,
  `CODEX_CA_CERTIFICATE`, `HERMES_CA_BUNDLE`). The name is connector data; no core change.
- a shared row: `SSL_CERT_FILE` (OpenSSL) or `REQUESTS_CA_BUNDLE` (Python `requests`). Every tool
  child reads these too.

The launch points each name at a file made for the run:

| Name | Mode | The file | The harness then trusts |
|---|---|---|---|
| own variable | `extend` | the recording proxy's CA alone | its own roots plus the recording proxy (`NODE_EXTRA_CA_CERTS`, `CODEX_CA_CERTIFICATE`) |
| own variable | `replace` | the recording proxy's CA alone | the recording proxy only, so it fails closed (`HERMES_CA_BUNDLE`, D41) |
| shared row | `extend` only | the system's trusted roots plus the recording proxy's CA | the real roots plus the recording proxy |

A `replace` variable fails closed: if the harness skips `HTTPS_PROXY` and connects straight to its
provider, the handshake fails, so no model call goes unrecorded. **A shared row gives that up.**
Its file must hold the real roots, or every tool child's TLS would break, so a connection the
harness makes past the recording proxy still verifies and is not recorded
([CHEAT-CHECKLIST](CHEAT-CHECKLIST.md#anchoring-and-the-agents-reach)). So a builder names a shared
row only when the harness has no CA variable of its own. When the harness has a harness-specific
variable that replaces its roots, name that one with `mode = "replace"`. A shared row is never
named in a `replace` profile.

The shared-row file is built when the run starts from the OS trust store (macOS: the System Roots
keychain; Linux: the distribution bundle, such as `/etc/ssl/certs/ca-certificates.crt`). It holds
certificates only, the recording proxy CA first, and is written with mode 0600 into the run's private `ca/`
directory. A start with no readable trust store is refused. Apple's `/usr/bin/python3` links
LibreSSL, which ignores `SSL_CERT_FILE`; a Python harness from Homebrew, python.org or uv honours
it.

Each name is also held to the shared environment-name rules (no provider, git, npm, OpenSSL or
Goal Cup namespace, no credential or proxy word). `SSL_CERT_DIR`, `CURL_CA_BUNDLE` and
`NODE_TLS_REJECT_UNAUTHORIZED` are refused in every field. A CA variable is refused in
`capture.env` and `capture.home.env`: `capture.ca` is the one field that sets it. A new mode or a
new name shape is a core policy and DECISIONS change.

## Identity

| Field | Required/default | Contract |
|---|---|---|
| `capture.identity.source` | required | `none`, the one value. `preassigned`, `stdout_json` and `stderr_line` were headless mode's and are refused as chat mode only. |

Every other identity field (`binding`, `match_field`, `match_value`, `id_field`, `wait_seconds`,
`prefix`, `grammar`) is refused as chat mode only.

## `[[capture.native]]`

Zero or one native row (D74). A connector with none records the wire only, and `goalcup
continue` appends its `resume_args` as when a session file exists.

| Field | Required/default | Contract |
|---|---|---|
| `capture.native[].name` | required | 1–64 bytes of `[A-Za-z0-9._-]`. |
| `capture.native[].root` | required | `{config_home}` or `{run_home}` alone, or that token plus `/` and at most eight relative components; each component is 1–64 bytes of ASCII `[A-Za-z0-9._-]` and is neither `.` nor `..`. |
| `capture.native[].pattern` | required | 1–1024 bytes, relative, at most eight `/`-separated components. |
| `capture.native[].form` | required | Closed values: `append_only`, `snapshot`. `sqlite` is explicitly a core capability, not schema 1. |
| `capture.native[].protocol_hint` | optional, default absent | `append_only` only; 1–64 bytes matching `^[a-z][a-z0-9.-]{0,63}$`. |
| `capture.native[].since_run_start` | optional, default `false` | Boolean. `true` reads only the files new or written since the run started (D57). |
| `capture.native[].warn_missing` | optional, default `true` | Boolean. `false` keeps the "no session file" `PROBLEM:` line quiet, for a harness whose supported versions include ones that write no such file (D72). The file is still read whenever it is written. |

Pattern components admit `[A-Za-z0-9._-]`, `*` and `?`. Empty, `.` or `..` components,
recursive `**`, braces (`{id}` included: no run names a session) and absolute paths are refused.
Globs do not cross `/`.

With `since_run_start = true`, the launcher may send the run's start time (`native_since`, Unix
seconds) when it creates the capture; without it the daemon reads from the entry's own create
time, which the entry's registry row keeps. A file the
pattern matches is admitted once the later of its modification and change times, in whole
seconds, is at or after that start (the change time catches a file whose modification time was
set back); an older file is not tracked and does not count against the file bound until it is
written. A start more than 300 seconds ahead of the daemon's clock is refused. Once admitted a file stays, and it is recorded whole from offset 0. Every admitted file is
recorded: none is refused and none is preferred. This is safe on a shared config home because runs
against it are serialized.

A native root under the config home (`{config_home}/...`) that does not exist yet is created when
the capture is created, the config home itself too if it is missing: one directory at a time,
mode `0700`, never through a link. A component that exists as a symlink or as anything but a
directory refuses the creation. A `{run_home}` root is created this way only under a shared run
home (`run_home = "shared"`, D59).

An unknown but syntactically valid `protocol_hint` is admitted and means capture-only unless a
registered decoder accepts the complete `StreamOpen`. A snapshot cannot carry a hint. The same
hint grammar applies to wire hosts.

## `[capture.argv_guard]`

All fields default when the optional table is present.

| Field | Default | Contract |
|---|---|---|
| `capture.argv_guard.long` | `[]` | At most 16 names; each 1–32 bytes matching `^[a-z][a-z0-9-]{0,31}$`, without leading `--`. |
| `capture.argv_guard.short` | `[]` | At most 16 entries; each one ASCII letter or digit. |
| `capture.argv_guard.short_with_value` | `[]` | Same bound and grammar as `short`. |
| `capture.argv_guard.long_abbreviations` | `false` | Boolean. When true, any prefix of a guarded long name is guarded, including an ambiguous prefix. |

A guarded `--name` or `--name=value` is refused. A single-dash cluster is scanned left to right;
a guarded short option is refused, while a `short_with_value` option consumes the rest of the
token and ends scanning. The guard is applied to every `args` and `resume_args` item. It can only
make the core argument policy stricter.

## `[capture.termination]`

If the table is absent, the effective strategy is `process_group`, `native_binary` is absent and
the grace is 10 seconds.

| Field | Default | Contract |
|---|---|---|
| `capture.termination.strategy` | `process_group` | Closed values: `process_group`, `native_pid`. |
| `capture.termination.native_binary` | absent | Required for `native_pid`, refused for `process_group`; 1–32 bytes of `[A-Za-z0-9._-]`. |
| `capture.termination.grace_seconds` | `10` | Integer 1–60. |

## `[[capture.hosts]]` and `capture.providers`

One to 16 unique host rows are required, counting the rows `capture.providers` adds. A profile
with `capture.model_host_from` and `absent = "refuse"` may list none: its model host comes from
the harness's config at each start.

A model provider's host row is written once, in the shared provider table
`crates/goalcup-protocol/providers.toml`, which is compiled into `goalcup`. A connector names
rows there with `capture.providers = ["anthropic"]`. The loader appends each named row to
`capture.hosts`, after the connector's own rows, and every host rule below applies to it. An
unknown or repeated name is refused, and so is a host that the connector also writes itself. The
table is frozen (D74): it keeps `anthropic` and `deepseek`, and a new connector lists its default
hosts in its own `[[capture.hosts]]`.

| Field | Required/default | Contract |
|---|---|---|
| `capture.hosts[].host` | required | Lowercase DNS name, at most 253 bytes; labels are 1–63 bytes of `[a-z0-9-]`, without edge hyphens. |
| `capture.hosts[].evidence_paths` | optional, default `[]` | Exact recorded paths. |
| `capture.hosts[].prefix_paths` | optional, default `[]` | Recorded path prefixes; `/` is refused as too broad. |
| `capture.hosts[].protocol_hint` | optional, default absent | The open 1–64-byte hint grammar used by native sources. |

A host has 1–32 paths across the two arrays. Paths are unique across both arrays, start with `/`,
are at most 1024 bytes, contain no query or fragment, no ASCII byte from `0x00` through `0x20`,
and no `0x7f` byte, `.` segment or `..` segment. This is a literal byte rule; it does not classify
non-ASCII Unicode whitespace or control characters. Hosts have no scheme, port, userinfo, wildcard
or trailing dot and cannot be IPv4 literals. For a pinned connector a host and an authentication
host also cannot be `localhost`, `*.localhost`, `localhost.localdomain` or a single-label name:
a capture launch sets `NO_PROXY=127.0.0.1,localhost`, so loopback traffic is never recorded
(`capture_policy::local_host_problem`). Kit cases and the controlled workload (`Origin::Dev`)
keep their `*.localhost` stand-ins. An authentication host and intercepted host cannot
overlap; model hosts belong in `capture.hosts`, not `auth_hosts`.

## The model host from the harness's config

`capture.model_host_from = { file = "config.yaml", key = "model.base_url" }` is the default for a
harness whose model service is the participant's choice (D62, D74). The participant picks the
service in the harness, and the harness saves its address in its own config. There is no provider
list. The shipped forms:

| Connector | `capture.model_host_from` | Without a value |
|---|---|---|
| `hermes-cli` | `{ file = "config.yaml", key = "model.base_url" }` | refused |
| `claude-code-cli` | `{ file = "settings.json", key = "env.ANTHROPIC_BASE_URL", absent = "defaults" }` | `api.anthropic.com` (`providers = ["anthropic"]`) |
| `codex-cli` | `{ file = "config.toml", key = "model_providers.*.base_url", via = "model_provider", absent = "defaults" }` | `chatgpt.com` (its own host row) |

- **What is read.** `goalcup start` and each `goalcup continue` open `file` in the shared config
  home (`<cfg>/capture-homes/<id>/`), before anything is created, and read the one string at
  `key`. The file is never read during the run. The format follows the extension: `.yaml` or
  `.yml` (block mappings, read by a small lookup in core), `.json` or `.toml`. Nothing else in
  the file is kept or logged.
- **One indirection (`via`).** With `via`, the string at `via` names one table, and it replaces
  the one `*` part of `key`: `model_provider = "vercel"` with `key =
  "model_providers.*.base_url"` reads `model_providers.vercel.base_url`. One indirection, never
  two. A `via` value that is not a plain name (`[A-Za-z0-9_-]`, 1–64 bytes) is refused at start,
  without repeating the value.
- **A missing value (`absent`).** Missing means: no file, an empty file, no key, no `via` value,
  no table of that name, an empty or white-space value, or YAML the small lookup cannot follow.
  With `absent = "refuse"`, the default, it is refused at start (below). With `absent =
  "defaults"` it is not an error: the segment opens the connector's host rows only, so the
  connector must list at least one, and `host.log` says so (below). Only a missing value falls
  back.
- **A file that cannot be read is refused under either setting**, since it may name a service
  that would then pass unrecorded: a link (never followed), not a regular file, over 1 MiB,
  unreadable by the user, not UTF-8, or JSON or TOML its parser refuses (plain JSON only: no
  comments or trailing commas). The refusal names the file's path, the problem and the fix: "Goal
  Cup cannot read `<path>`, where `<display name>` keeps its model service setting: the file
  `<problem>`. `<fix>`. Then run the command again."
- **With a value, the default hosts and that address both open.** The configured address opens
  beside the connector's host rows, never instead of them (ledger L11).
- **What is recorded.** The value must be an `https` URL. Its host joins the intercepted hosts
  for that segment only, with the run's port; its path is the base under which requests are
  recorded. `<base>/chat/completions`, `<base>/messages` and `<base>/responses` (and the same under
  `<base>/v1`) carry the `openai-chat-completions`, `anthropic-messages` and `openai-responses`
  hints; every other path under the base is recorded with no hint. A host the connector also lists
  in `capture.hosts` keeps that row. Every other host stays an opaque tunnel, counted in
  `proxy admission:`, listed in the run's `tunnels-<entry_id>.jsonl` and never opened.
- **What is logged.** Only `model service: <host>:<port> (<key> in <file>)`, in the run's
  `host.log`, where `<key>` is the resolved key (`model_providers.vercel.base_url`). When the
  run falls back, the line is `model service: the connector's default hosts (<key> not set in
  <file>)`, with `via` written out: ``(`model_providers.*.base_url` through `model_provider` not
  set in config.toml)``. A userinfo (`user:pass@`), the path and the query are never logged or
  recorded.
- **Refused at start, and not ready at prep.** A missing file, key or `via` value (with
  `absent = "refuse"`); a value that is not an `http` or `https` URL; an authority (the part
  before the path) with a backslash, a space or a control character, which a URL parser reads as
  another host or refuses; plain `http` (only `HTTPS_PROXY` is set, so it would pass around the
  recorder); a loopback name; an address literal; a single-label name; a host that names Goal Cup,
  or the project's domain `agentcommunity.org` or a subdomain of it (its event server is Goal
  Cup's own host); a sign-in host; and a path the recorder cannot follow (a `.` or `..` segment, a
  `%2e` escape, a backslash, a quote, an angle bracket, a backtick, a brace or a non-ASCII byte).
  A name that resolves only to this computer (to a loopback address, to `0.0.0.0`, to `::`, or to
  an IPv4-mapped loopback address) is refused too, from a DNS lookup bounded by the preflight
  budget. The refusal names the value (`<scheme>://<host>:<port>` only), the reason, the setting
  and the way to change it: "`<display name>` is set to use `<value>`, which Goal Cup cannot
  record: `<reason>` (set by `<key>` in `<file>`). Goal Cup records only an https model service
  that is not on this computer. Run goalcup setup --harness `<id>`, choose such a service there,
  then run the command again." Nothing is created. The name check runs at three points, each a
  point-in-time lookup: `goalcup start` before anything is created, `goalcup continue` before it
  closes an earlier segment, and `goalcup prep`. A name whose DNS changes to this computer after
  its check is not seen; the gap is recorded in the one-capture-system ledger (L30).
  `goalcup setup` makes the same checks: while the value cannot be recorded the home is not
  ready, and prep says why and opens the harness again. Only the address the config names is
  checked; a login the harness refreshes at another host is not (D62).
- **Schema rules.** `file` is a relative path of at most 4 components with a `.yaml`, `.yml`,
  `.json` or `.toml` extension. `key` is 1–8 names of `[A-Za-z0-9_-]` (1–64 bytes each) joined
  with `.`; with `via` it holds exactly one `*` part, which is not its last, and without `via`
  none. `via` follows the `key` rules, with no `*`. `absent` is the closed set `"refuse"`,
  `"defaults"`. No `.`, `-` or `_` separated part of the file's name, the key or `via` may be
  `auth`, `credential(s)`, `secret(s)`, `token(s)`, `key(s)`, `api_key`, `apikey` or `password`,
  in any case: Goal Cup Proxy reads a service address, never a key file or a key. `env` is allowed (D74), for Claude Code's
  `env.ANTHROPIC_BASE_URL`; the same rule holds for the `participant.no_prompts` and
  `time_left_tool` keys, which share the check (ledger L15).
- **A switch mid-run.** A service the participant switches to inside a running session is not
  intercepted until the next segment. The participant guide says: exit the harness and run
  `goalcup continue`.

## `[participant]`

Optional. Core messages name a harness only through this table, so no core code holds a harness
name. When it is absent, messages use the connector `id`. Unknown keys are refused, as everywhere.
Both strings are printed to a terminal, so they are held to plain text.

| Field | Required/default | Contract |
|---|---|---|
| `participant.display_name` | required when the table is present | The name as a person says it ("Claude Code"). 1–40 characters. |
| `participant.first_screen` | optional, default absent | What the harness shows first and what to do about it, shown just before the harness opens. 1–200 characters. Leave it out when the harness asks nothing. |
| `participant.no_prompts` | optional, default `[]` | Up to 4 `{ file, key, values }` rows (D67): a setting in the harness's own config that makes it run without permission prompts, and the 1–4 values (as text, `true` for a boolean; the first is the one recommended) that mean so. `file` and `key` follow the `capture.model_host_from` rules (a `.yaml`, `.yml`, `.json` or `.toml` file in the shared config home, a dotted key, neither naming a key file or a key); requires `config_home = "shared"`; no row twice. `goalcup setup` reads them and, for each one not set to a listed value, recommends it in its harness home step with a link to the harness's `setup_section`. `docs/harnesses/<id>.md` must have a "Run without permission prompts" section. Goal Cup never writes them, and prep never fails for them. |
| `participant.setup_section` | optional, default absent | The anchor of the harness's section in [HARNESS-SETUP](HARNESS-SETUP.md), the participant's setup guide (`setup_section = "codex"` for `### Codex`). 1–40 lower-case letters and digits joined by single `-`. `goalcup setup` links there on its harness step ("Before a long run, read ...") and in a `no_prompts` recommendation; without it the link names the guide alone. `harness_conformance.rs` requires every shipped connector to name one, and the guide to have that heading. |
| `participant.time_left_tool` | optional, default absent | `{ file, key, pass_env }` (fix plan v9): where the participant adds `goalcup mcp`, the time-left MCP tool, in the harness's own config. `file` and `key` follow the `no_prompts` rules, but `file` is relative to the home `goalcup setup` opens (the shared config home, else the shared run `HOME`; one of the two is required). `key` is the file's table of MCP servers: dotted table names in TOML, one top-level key written as it is in JSON and YAML (`amp.mcpServers` is one JSON key). `pass_env` says how the harness hands the server `GOALCUP_TIME_LEFT_FILE`: `inherited` (default; its whole environment), `env_vars` (only the names an `env_vars` list gives) or `env_template` (only an `env` map, whose `${VAR}` values it fills). `goalcup setup` prints the lines for the file's type, with goalcup's absolute path, on its harness step. Goal Cup never writes them (D67). |

Both strings (and each `no_prompts` value) are an allow-list: printable ASCII only (0x20 to 0x7E), no leading or trailing space,
and no two spaces in a row. That refuses every control character (ESC and so any ANSI sequence,
CR, LF, tab, DEL, the C1 range) and every non-ASCII character, so no invisible, bidirectional,
tag or odd-space character gets through. `harness_conformance.rs` also requires every shipped
connector to declare a `display_name`.

```toml
[participant]
display_name = "Claude Code"
first_screen = "Claude Code may ask whether you trust this folder. Choose Yes."
setup_section = "claude-code"

[participant.time_left_tool]
file = ".claude.json"
key = "mcpServers"
pass_env = "inherited"

[[participant.no_prompts]]
file = "settings.json"
key = "permissions.defaultMode"
values = ["bypassPermissions"]
```

## `[redaction]`

`redaction.extra_patterns` defaults to `[]`, which is the shared redaction set alone. Each inline
row is one rule. A row names a form and the connector's own field or header; it never carries a
regex. The recorder derives the regex and its bounded streaming recognizer from the form.

| Field | Contract |
|---|---|
| `kind` | Required. `^[a-z][a-z0-9_]{0,47}$`. Every marker and audit span of the rule names it. |
| `json_field` | The JSON field whose value is removed. 1–64 bytes of `[A-Za-z0-9_-]`. Needs `value`. |
| `value` | `object`, `string` or `scalar`: the value form of `json_field`. |
| `header_prefix` | Lowercase `PREFIX…: value` text, wherever it is recorded. `^[a-z][a-z0-9-]{0,63}$`. Alone. |

A row has either `json_field` with `value`, or `header_prefix` alone. A row with a `regex`, with
both forms or with neither is refused when the file is parsed. The regex each form stands for
(`harness::ExtraPattern::regex`):

| Form | Regex | Span |
|---|---|---|
| `json_field = "F", value = "object"` | `"F"\s*:\s*(\{(?:[^{}]\|\{(?:[^{}]\|\{[^{}]*\})*\})*\})` | the object, nested at most three deep |
| `json_field = "F", value = "string"` | `"F"\s*:\s*"([^"\\]+)"` | the string's interior |
| `json_field = "F", value = "scalar"` | `"F"\s*:\s*("[^"\\]*"\|[^,}\s]+)` | a string, else a bare token such as a number |
| `header_prefix = "P"` | `P[a-z0-9-]+:\s*[^\r\n"]+` | the whole `name: value` text |

`header_prefix` is not a header filter: no HTTP header is ever recorded. It matches the text
`PREFIX…: value` anywhere in a recorded body or file, not only at the start of a line or a word,
and only in lowercase: `X-Vendor-Billing: …` is not matched, and no error says so. The match
ends at a CR, an LF or a `"`. A short prefix over-matches: `x-` also strips `max-age: 3600`.

What a form does not remove: `string` leaves a value with a backslash escape raw; `object` leaves
a value nested four deep raw, and stops early at a brace inside a string; a quoted `scalar` that
holds both an escape and a space is cut at the space; and no form matches a field inside a
JSON-encoded string (`\"plan_type\"`). Pick the form with these limits in view, and check the
harness's real bytes against it.

A header the harness puts inside a JSON body (`"headers": {"x-codex-turn-state": "…"}`) is a
JSON field, not a header: use `json_field` with the header name, which the field grammar admits.
`header_prefix` only matches `name: value` text.

The shared set already covers what every harness holds: provider keys, tokens, e-mail addresses,
and the recording proxy's credential (`proxy_credential`, the secret in the launch's
`HTTPS_PROXY`). A connector never names those. On top of the patterns, the daemon redacts the
exact value of the run's proxy credential in every form `goalcup_protocol::run_secret::encoded_forms`
derives, as `<redacted:run_secret>` (D60).

A JSON-field marker keeps the record JSON: the key and colon stay, and a value that was not a
string gets a quoted marker. A new form is a core PR: it needs a recognizer in the daemon's
streaming redactor, reviewed at the hold bound, and a variant of `harness::RedactionRule`.

```toml
[redaction]
extra_patterns = [
  { kind = "codex_plan_type", json_field = "plan_type", value = "string" },
  { kind = "anthropic_billing_header", header_prefix = "x-anthropic-billing-" },
]
```

## Cross-field rules

In addition to the local rules above, all of these apply:

1. (Retired with headless mode: the `preassigned` and `{session_id}` rule.)
2. (Retired by D61: the credential rules. A `[capture.credential]` table is refused.)
3. `capture.home.seed`, `dirs` and an explicit `per_run_dir` require `config_home = "per_run"`.
4. With `identity.source = "none"`, the native root must be `{run_home}/...` with a run home
   that is not `shared`, or `{config_home}/...` with a per-run config home, or the native
   source must declare
   `since_run_start = true`. A shared unscoped source read from before the run could read another
   run's session and is refused.
5. The native root plus pattern, compared case-insensitively, cannot
   match a seed destination. A `{run_home}` root is checked both with the config home coincident
   with the run home and nested below `per_run_dir`.
6. `capture.home.env` (when present) and all `capture.ca.env` names are pairwise
   distinct case-insensitively. No `capture.env` key may duplicate one.
7. `SSL_CERT_FILE` and `REQUESTS_CA_BUNDLE` are named only with `capture.ca.mode = "extend"`.
8. Authentication and intercepted host sets are disjoint; every intercepted host has at least one
   scoped path.
9. A native hint is allowed only on `append_only`; a prefix path cannot be `/`.
10. A manifest is delivery-agnostic: no argument, resume argument, goal flag, environment key/value, host or auth
    host may name Goal Cup, its reserved environment namespace or a Goal Cup host. There is no
    schema field for a server, upload or delivery credential.
11. `run_home = "shared"` is refused with `capture.home.env` (D59).
12. `capture.model_host_from` with `absent = "defaults"` needs at least one host row, its own or
    from `providers` (D74).

## Folder/id/digest admission

Discovery examines sorted immediate child directories of the connector root. A child without
`harness.toml` is skipped. An immediate-child symlink, a symlinked connector root, or a
non-regular `harness.toml` is refused. The hashing walk refuses every symlink, device, socket,
non-UTF-8 name and other unhashable entry.

The walk captures and hashes the same `harness.toml` and seed bytes the loader parses and later
writes. A connector directory may contain only:

- `harness.toml`; and
- `files/<path>` named by `capture.home.seed`.

A leftover V1 `manifest.toml` is refused like any other unnamed file (OS-29).

Every declared seed must exist, and an unnamed file is refused.

For a discovered folder, `connector_folder_admission` compares three captured facts with the
compiled pin table: immediate folder name, declared `id`, and whole-directory digest. If none
names any pin row, the connector is unpinned. If any one names a pinned connector, all three must
name the same pin row and the digest must match exactly; otherwise loading refuses. Thus a
byte-identical pinned directory under an alias is not a second trusted connector. The manifest
cannot claim `Origin::Dev`, and a direct development load also refuses any pinned id or any
pinned digest.

## Pin evolution

[`connector-dirs.txt`](../crates/goalcup-protocol/pins/connector-dirs.txt) is compiled into the
daemon. It contains optional leading comments followed by at least one sorted, unique line of
`<id> <64-lowercase-hex>`, with exactly one space, LF endings and a final newline. Comments after
the first pin, blank lines, uppercase hex and malformed rows fail the build.

Never type a directory digest. After any edit under `connectors/`, run:

```sh
sh scripts/pin-manifests.sh
```

The script hashes every regular file in the directory using the canonical directory-digest
layout. The protocol tests require one pin per connector folder and one folder per pin. A pin is
a post-build tamper check, not protection from a malicious merge; review remains the trust
control. Concurrent connector changes regenerate the file on the combined tree rather than
hand-merging hashes.

A `capture.providers` row lives in `providers.toml`, outside `connectors/<id>/`, so the directory
digest does not cover it. The effective host set of a pinned connector is its digest plus the
`goalcup` build's provider table, which is compiled in and changes only in a reviewed core PR.

Schema evolution and pin evolution are separate. A pin changes with every connector edit; the
schema changes only as [the compatibility promise](#the-compatibility-promise) and
[the deprecation rule](#the-deprecation-rule) say: what may stay schema 1, and what needs
schema 2.

## Examples

Read the shipped connectors. Conformance tests load each as `Origin::Pinned` and compare its
directory with its pin. The five frozen
[schema-1 fixtures](../crates/goalcup-cli/tests/fixtures/schema-1-connectors/) are complete
connectors for harnesses nobody ships, one per shape ([what each covers](#the-compatibility-promise)).

- [`connectors/hermes-cli/harness.toml`](../connectors/hermes-cli/harness.toml): chat mode with a
  `configure` command, its model host read from its own `config.yaml` (`model_host_from`, no
  `hosts`), a `snapshot` native source read from the run's start, an argv guard and a reserved
  prefix.
- [`connectors/codex-cli/harness.toml`](../connectors/codex-cli/harness.toml): chat mode
  (`interactive`), shared config home, `since_run_start`, `native_pid` termination, extra
  redaction patterns and per-platform env.
- [`connectors/claude-code-cli/harness.toml`](../connectors/claude-code-cli/harness.toml): chat
  mode with a `keychain_link` run home.
- [`connectors/amp-cli/harness.toml`](../connectors/amp-cli/harness.toml): chat mode with a
  shared run home and no home variable (`run_home = "shared"`, a `{run_home}` native root).
