# Connector schema 1

What may `connectors/<id>/harness.toml` contain? This is the public field and validation
reference for harness authors and reviewers. The procedure for adding a harness is
[HARNESS-ONBOARDING](HARNESS-ONBOARDING.md).

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
executable; arguments and goal; configure commands; environment; home; CA; identity;
native source; argument guard; termination; hosts; redaction; delivery independence; cross-field
rules; then each `capture.platform` entry, in the fixed order `linux-x86_64`, `linux-aarch64`,
`macos-aarch64`, `macos-x86_64`, whatever order the file uses. The first refusal is deterministic. Unknown keys and
unknown closed values use the same message: a newer schema needs a newer `goalcup`, while a
misspelling is refused rather than ignored. The retired executable pin keys (`sha256`, `version`)
are the exception: they are refused before deserialization with their own sentence (D53).

All public schema structs and enums are non-exhaustive. A manifest cannot select its load origin:

- `Origin::Pinned` is an in-tree or pin-checked connector. It refuses an interpreter as the
  executable and refuses an absolute, home-relative, parent-relative or expanding path in an
  argument. There is no per-connector list in Rust (D51): `capture.args`, `goal_flag` and
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
| `capture.args` | required | 1–64 strings (0–64 with `interactive`); each at most 4096 bytes and without control characters. |
| `capture.goal_flag` | optional, default absent | One long option, at most 32 bytes, matching `--[a-z][a-z0-9-]*`; no `=` or value. Refused with `interactive`. |
| `capture.interactive` | optional, default `false` | Boolean. `true` runs the harness in its chat mode in the participant's terminal, where the participant types the prompt (D57); see below. |
| `capture.resume_args` | optional, default `[]` | Strings appended after `capture.args` when a run is continued; each held to every `args` rule, and `args` and `resume_args` together hold at most 64. |
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
| `capture.native` | defaults to `[]`, validation requires exactly one | One `[[capture.native]]` table. |
| `capture.argv_guard` | optional, default absent | Harness-specific pre-parser argument refusals. |
| `capture.termination` | optional; effective defaults below | How the owned harness is stopped. |
| `capture.hosts` | defaults to `[]`, validation requires 1–16 (with `providers`) | Intercepted model hosts and their recorded paths. |
| `capture.providers` | optional, default `[]` | Unique names from the shared provider table; each adds that provider's host row. See below. |

The goal has exactly one slot: either `capture.args` ends in the literal `--` and the goal is the
next single argument, or `capture.goal_flag` is present and carries the goal as
`--flag=<goal>`, never both. Every argument, every `resume_args` entry and `goal_flag` passes the
core deny policy and the optional argument guard.

With `interactive = true` (D57) there is no goal slot: the participant types the prompt in the
harness. The profile then takes no `goal_flag`, its `args` need not end in `--` and may be
empty, and its `identity.source` must be `none`. With `false`, the default, every
rule above holds unchanged. `resume_args` may be set either way; `goalcup continue` appends it
after `args`.

### Configure commands

`capture.configure` (D61) is for a harness whose native source needs a setting that has no
environment variable (Hermes writes `sessions/session_<id>.json` only with
`sessions.write_json_snapshots` on, and keeps writing it after a compaction only with
`compression.in_place` off). Each entry is an argument list for the harness's own
`capture.executable.program`. Only a chat-mode profile (`interactive = true`) takes it: the
commands run with the environment, and in the home, that `goalcup prep` opens the harness with.

```toml
configure = [
  ["config", "set", "sessions.write_json_snapshots", "true"],
  ["config", "set", "compression.in_place", "false"],
]
```

- **When.** `goalcup prep` runs them in the harness home after the participant's own setup, once
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
  it ended); start launches nothing and says to run `goalcup prep --harness <id>` again.
- **What it may change.** Only how the harness stores its session (capture and storage
  settings), in the harness home. Never how the agent behaves: not the model, the provider, the
  tools, the prompts, or whether compression is on. Core refuses an argument that has `model`,
  `provider`, `base_url`, `api_key`, `key`, `token` or `secret` as a `.`-separated part (compared
  in any case, leading dashes ignored: `model.provider`, `--model` and `auth.Token` are refused;
  `max_tokens` is not). The `args` rules also refuse the endpoint, credential, proxy and trust
  spellings, a profile switch and the connector's own guarded options. Review is the control for
  the rest.

Argument templates (`args` and `resume_args`) admit only `{workspace}` and `{session_id}`.
`capture.env` value templates also admit `{config_home}` (the isolated harness home) and
`{run_home}` (the run's private `HOME`), so a harness with a second state variable can point it
inside them (`DATA_DIR = "{config_home}/data"`). The value rules still apply around a token: no
leading `/` or `~`, no `$`, backtick or `://`, and no `..` component. `{{` and `}}` render literal
braces. Any other token or unmatched brace is refused. `{session_id}` has the identity rules
described below.

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
keeps it instead (D59): one empty directory per connector, made by the tracker and kept across
runs, held to the rules of a shared config home. Runs on it are serialized; the daemon refuses it
at the participant's `HOME`, at a `real_dirs` path or anywhere but directly under
`<config_dir>/capture-homes`; the baseline lists it in place of the config home; a missing
`{run_home}` native root is created as a `{config_home}` root is; a `{run_home}` native root with
identity `none` needs `since_run_start = true`. It is refused with
`capture.home.env`: such a harness shares its config home instead.

## Credential and CA

The tracer never sees, forwards or stores a participant's provider key (D61). A
`[capture.credential]` table, in the base profile or in a `[capture.platform.<key>]` override, is
refused with "a connector may not pass a provider key; the harness keeps its own login in its
home". A harness that reads its key from an environment variable only is set up with its own
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
| own variable | `extend` | the recording proxy's CA alone | its own roots plus the proxy (`NODE_EXTRA_CA_CERTS`, `CODEX_CA_CERTIFICATE`) |
| own variable | `replace` | the recording proxy's CA alone | the proxy only, so it fails closed (`HERMES_CA_BUNDLE`, D41) |
| shared row | `extend` only | the system's trusted roots plus the proxy's CA | the real roots plus the proxy |

A `replace` variable fails closed: if the harness skips `HTTPS_PROXY` and connects straight to its
provider, the handshake fails, so no model call goes unrecorded. **A shared row gives that up.**
Its file must hold the real roots, or every tool child's TLS would break, so a connection the
harness makes past the proxy still verifies and is not recorded
([CHEAT-CHECKLIST](CHEAT-CHECKLIST.md#anchoring-and-the-agents-reach)). So a builder names a shared
row only when the harness has no CA variable of its own. When the harness has a harness-specific
variable that replaces its roots, name that one with `mode = "replace"`. A shared row is never
named in a `replace` profile.

The shared-row file is built when the run starts from the OS trust store (macOS: the System Roots
keychain; Linux: the distribution bundle, such as `/etc/ssl/certs/ca-certificates.crt`). It holds
certificates only, the proxy CA first, and is written with mode 0600 into the run's private `ca/`
directory. A start with no readable trust store is refused. Apple's `/usr/bin/python3` links
LibreSSL, which ignores `SSL_CERT_FILE`; a Python harness from Homebrew, python.org or uv honours
it.

Each name is also held to the shared environment-name rules (no provider, git, npm, OpenSSL or
tracker namespace, no credential or proxy word). `SSL_CERT_DIR`, `CURL_CA_BUNDLE` and
`NODE_TLS_REJECT_UNAUTHORIZED` are refused in every field. A CA variable is refused in
`capture.env` and `capture.home.env`: `capture.ca` is the one field that sets it. A new mode or a
new name shape is a core policy and DECISIONS change.

## Identity

| Field | Required/default | Contract |
|---|---|---|
| `capture.identity.source` | required | Closed values: `preassigned`, `stdout_json`, `stderr_line`, `none`. An `interactive` profile takes `none` only. |
| `capture.identity.binding` | required except for `none` | Closed values: `at_launch`, `deferred`. |
| `capture.identity.match_field` | `stdout_json` only; required there | Dot path of 1–4 nonempty `[A-Za-z0-9_]+` segments. |
| `capture.identity.match_value` | `stdout_json` only; required there | 1–64 bytes, no control character. |
| `capture.identity.id_field` | `stdout_json` only; required there | Same dot-path grammar as `match_field`. |
| `capture.identity.wait_seconds` | `stdout_json` only; default 120 | 1–600: how long the launch waits for the identity event before recording the launch without one. |
| `capture.identity.prefix` | `stderr_line` only; required there | 1–32 printable ASCII bytes. |
| `capture.identity.grammar` | `stderr_line` only; required there | Closed values: `timestamp-hex`, `token`. |

`timestamp-hex` means `\d{8}_\d{6}_[0-9a-f]{6}`. `token` means 1–128 characters from
`[A-Za-z0-9_-]`; a lowercase UUID is a token. The only admitted source/binding pairs are:

| Source | Binding |
|---|---|
| `preassigned` | `at_launch` |
| `stdout_json` | `at_launch` |
| `stderr_line` | `deferred` |
| `none` | absent |

JSON selector fields are forbidden on all other sources; stderr fields are forbidden on all other
sources. `none` takes no binding. Widening this table is a core change with an offline harness-kit
case, but remains schema 1 because it makes a formerly refused combination valid.

## `[[capture.native]]`

Exactly one native row is required.

| Field | Required/default | Contract |
|---|---|---|
| `capture.native[].name` | required | 1–64 bytes of `[A-Za-z0-9._-]`. |
| `capture.native[].root` | required | `{config_home}` or `{run_home}` alone, or that token plus `/` and at most eight relative components; each component is 1–64 bytes of ASCII `[A-Za-z0-9._-]` and is neither `.` nor `..`. |
| `capture.native[].pattern` | required | 1–1024 bytes, relative, at most eight `/`-separated components. |
| `capture.native[].form` | required | Closed values: `append_only`, `snapshot`. `sqlite` is explicitly a core capability, not schema 1. |
| `capture.native[].protocol_hint` | optional, default absent | `append_only` only; 1–64 bytes matching `^[a-z][a-z0-9.-]{0,63}$`. |
| `capture.native[].since_run_start` | optional, default `false` | Boolean. `true` reads only the files new or written since the run started (D57). |

Pattern components admit `[A-Za-z0-9._-]`, `*`, `?` and the special `{id}` token. Empty, `.` or
`..` components, recursive `**`, other braces and absolute paths are refused. `{id}` appears
exactly once when the identity source names a session, and not at all when the source is `none`.
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
token and ends scanning. The guard is applied to every `args` and `resume_args` item and to `goal_flag`. It can only
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

One to 16 unique host rows are required, counting the rows `capture.providers` adds.

A model provider's host row is written once, in the shared provider table
`crates/goalcup-protocol/providers.toml`, which is compiled into the tracer. A connector names
rows there with `capture.providers = ["anthropic"]`. The loader appends each named row to
`capture.hosts`, after the connector's own rows, and every host rule below applies to it. An
unknown or repeated name is refused, and so is a host that the connector also writes itself. A new
provider is a reviewed row in that file, added from a measured run.

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

## `[participant]`

Optional. Core messages name a harness only through this table, so no core code holds a harness
name. When it is absent, messages use the connector `id`. Unknown keys are refused, as everywhere.
Both strings are printed to a terminal, so they are held to plain text.

| Field | Required/default | Contract |
|---|---|---|
| `participant.display_name` | required when the table is present | The name as a person says it ("Claude Code"). 1–40 characters. |
| `participant.first_screen` | optional, default absent | What the harness shows first and what to do about it, shown just before the harness opens. 1–200 characters. Leave it out when the harness asks nothing. |

Both fields are an allow-list: printable ASCII only (0x20 to 0x7E), no leading or trailing space,
and no two spaces in a row. That refuses every control character (ESC and so any ANSI sequence,
CR, LF, tab, DEL, the C1 range) and every non-ASCII character, so no invisible, bidirectional,
tag or odd-space character gets through. `harness_conformance.rs` also requires every shipped
connector to declare a `display_name`.

```toml
[participant]
display_name = "Claude Code"
first_screen = "Claude Code may ask whether you trust this folder. Choose Yes."
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
and the tracer's own proxy credential (`proxy_credential`, the secret in the launch's
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

1. `preassigned` requires `{session_id}` in `capture.args`. `{session_id}` anywhere in `args`,
   `resume_args` or an environment value requires `preassigned`.
2. (Retired by D61: the credential rules. A `[capture.credential]` table is refused.)
3. `capture.home.seed`, `dirs` and an explicit `per_run_dir` require `config_home = "per_run"`.
4. With `identity.source = "none"`, the native root must be `{run_home}/...` with a run home
   that is not `shared`, or `{config_home}/...` with a per-run config home, or the native
   source must declare
   `since_run_start = true`. A shared unscoped source read from before the run could read another
   run's session and is refused.
5. The native root plus pattern, compared case-insensitively with `{id}` treated as `*`, cannot
   match a seed destination. A `{run_home}` root is checked both with the config home coincident
   with the run home and nested below `per_run_dir`.
6. `capture.home.env` (when present) and all `capture.ca.env` names are pairwise
   distinct case-insensitively. No `capture.env` key may duplicate one.
7. `SSL_CERT_FILE` and `REQUESTS_CA_BUNDLE` are named only with `capture.ca.mode = "extend"`.
8. Authentication and intercepted host sets are disjoint; every intercepted host has at least one
   scoped path.
9. A native hint is allowed only on `append_only`; a prefix path cannot be `/`.
10. A manifest is delivery-agnostic: no argument, resume argument, goal flag, environment key/value, host or auth
    host may name Goal Cup, its reserved environment namespace or a tracker host. There is no
    schema field for a server, upload or delivery credential.
11. `run_home = "shared"` is refused with `capture.home.env` (D59).

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
tracer build's provider table, which is compiled in and changes only in a reviewed core PR.

Schema evolution and pin evolution are separate. A new optional field whose absence preserves
current behaviour, a widened bound, a new closed value or a newly admitted cross-field pair may
remain schema 1. Removing or renaming a field, changing a default, or narrowing a bound requires
schema 2, migration of every harness manifest and regenerated directory pins. Tightening a core
policy row is not a schema bump, but it needs a DECISIONS entry and migrates every connector it
would reject in the same PR.

## Examples

Read the shipped connectors. Conformance tests load each as `Origin::Pinned` and compare its
directory with its pin.

- [`connectors/hermes-cli/harness.toml`](../connectors/hermes-cli/harness.toml): chat mode with a
  `configure` command, a `snapshot` native source read from the run's start, an argv guard and a
  reserved prefix. Its headless form, with `stderr_line` identity, is frozen at
  [`headless-connectors/hermes-cli`](../crates/goalcup-protocol/tests/fixtures/headless-connectors/hermes-cli/harness.toml)
  for the tests of that recipe.
- [`connectors/codex-cli/harness.toml`](../connectors/codex-cli/harness.toml): chat mode
  (`interactive`), shared config home, `since_run_start`, `native_pid` termination, extra
  redaction patterns and per-platform env.
- [`connectors/claude-code-cli/harness.toml`](../connectors/claude-code-cli/harness.toml): chat
  mode with a `keychain_link` run home.
