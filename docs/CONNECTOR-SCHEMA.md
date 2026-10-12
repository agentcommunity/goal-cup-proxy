# Connector schema 1

What may `connectors/<id>/harness.toml` contain? This is the public field and validation
reference for harness authors and reviewers.
Updated: 2026-10-11.

The executable authority is
[`harness.rs`](../crates/goalcup-protocol/src/harness.rs). Its
[`capture_policy.rs`](../crates/goalcup-protocol/src/capture_policy.rs) trust tables,
[`hints.rs`](../crates/goalcup-protocol/src/hints.rs) hint grammar and
[`pins.rs`](../crates/goalcup-protocol/src/pins.rs) pin admission rule are part of this contract.
The one-test-per-refusal table is
[`harness_schema.rs`](../crates/goalcup-protocol/tests/it/harness_schema.rs). If this reference and
those sources disagree, the sources refuse the manifest.

## Parse and load

`harness.toml` is UTF-8 TOML. The loader first reads `schema_version` from a generic table, then
deserializes every table with unknown fields denied, then validates in this fixed order:
executable; arguments and goal; environment; home; credential; CA; identity; native source;
argument guard; termination; hosts; redaction; delivery independence; cross-field rules. The
first refusal is deterministic. Unknown keys and unknown closed values use the same message: a
newer schema needs a newer `goalcup`, while a misspelling is refused rather than ignored.

All public schema structs and enums are non-exhaustive. A manifest cannot select its load origin:

- `Origin::Pinned` is an in-tree or pin-checked connector. It refuses an interpreter as the
  executable and refuses an absolute, home-relative, parent-relative or expanding path in an
  argument.
- `Origin::Dev` is an unpinned directory under `GOALCUP_CONNECTORS_DIR` selected through
  `--dev`. It relaxes only those two refusals. Environment values, seeds, trust tables, all
  bounds and all cross-field rules remain strict.

The CLI and daemon both use the same loader and each validates again at its own trust boundary.

## Top level and `[capture]`

An omitted required field is a TOML error. “Default” below means the value applied when an
optional field or table is absent.

| Field | Required/default | Contract |
|---|---|---|
| `schema_version` | required | Integer `1`; this build's closed supported set is `{1}`. |
| `id` | required | 2–48 bytes; `^[a-z][a-z0-9-]{1,47}$`. |
| `capture` | required table | The complete launch-and-capture profile below. |
| `redaction` | optional, default absent | Connector-selected rows from the core redaction mirror. |
| `capture.args` | required | 1–64 strings; each at most 4096 bytes and without control characters. |
| `capture.goal_flag` | optional, default absent | One long option, at most 32 bytes, matching `--[a-z][a-z0-9-]*`; no `=` or value. |
| `capture.continuation` | required | Non-blank, one line, 1–512 bytes; declarative provenance for unattended continuation. |
| `capture.auth_hosts` | optional, default `[]` | At most 16 unique lowercase DNS hosts; always opaque tunnels. |
| `capture.env` | optional, default `{}` | At most 32 string-to-string entries; the child-only process environment additions. |
| `capture.executable` | required table | Exact executable name, measured version and byte digest. |
| `capture.home` | required table | Run `HOME`, isolated harness home and any seeds. |
| `capture.credential` | optional, default absent | One core-allowlisted child credential variable. |
| `capture.ca` | required table | One CA mode and its core-allowlisted variables. |
| `capture.identity` | required table | How this run's native-session identity is obtained and bound. |
| `capture.native` | defaults to `[]`, validation requires exactly one | One `[[capture.native]]` table. |
| `capture.argv_guard` | optional, default absent | Harness-specific pre-parser argument refusals. |
| `capture.termination` | optional; effective defaults below | How the owned harness is stopped. |
| `capture.hosts` | defaults to `[]`, validation requires 1–16 | Intercepted model hosts and their recorded paths. |

The goal has exactly one slot: either `capture.args` ends in the literal `--` and the goal is the
next single argument, or `capture.goal_flag` is present and carries the goal as
`--flag=<goal>`, never both. Every argument and `goal_flag` passes the core deny policy and the
optional argument guard.

Argument and `capture.env` value templates admit only `{workspace}`, `{port}` and
`{session_id}`. `{{` and `}}` render literal braces. Any other token or unmatched brace is
refused. `{session_id}` has the identity rules described below.

### Executable

| Field | Required/default | Contract |
|---|---|---|
| `capture.executable.program` | required | Bare `PATH` name, 1–64 bytes of `[A-Za-z0-9._-]`; no path. |
| `capture.executable.version` | required | 1–32 bytes; `^[0-9][0-9A-Za-z.+-]{0,31}$`. |
| `capture.executable.sha256` | required | Exactly 64 lowercase hexadecimal characters. |

A pinned connector's `program` cannot be an interpreter or launcher named by
`INTERPRETER_PROGRAMS`. Matching is ASCII-case-insensitive and also catches a listed name with a
numeric version suffix, such as `python3.13`. The current table includes shells, `env`, privilege
and command wrappers, script runtimes, package launchers, `git`, `find`, `make`, Java and
PowerShell. A development kit case may use one, but its executable digest is still checked by the
runtime.

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

- endpoint, proxy, credential and provider names;
- the launch-pinned `HOME`, `PATH`, `USER`, `LOGNAME` and `TMPDIR`;
- CA registry names and global trust overrides;
- injection/startup variables and names ending in command suffixes such as `_COMMAND`, `_CMD`,
  `EDITOR`, `PAGER`, `ASKPASS`, `_WRAPPER` or `_RUNNER`;
- injection, provider and harness-reserved prefixes, including `DYLD_`, `LD_`, `OTEL_`,
  `CLAUDE_CODE_USE_`, `GIT_`, `NPM_CONFIG_`, `PIP_`, `XDG_`, `GOALCUP_`, the cloud-provider
  prefixes and `HERMES_`; and
- any name pinned by this manifest's home, credential or CA fields.

R-3 is an intentional over-refusal: `NO_PAGER` ends in `PAGER`, so schema 1 refuses it even when
the harness treats it as a boolean. A harness that needs that name requires a reviewed core policy
change; spelling around the rule in a connector is not allowed.

Each environment value is at most 1024 bytes and contains no control character. `SHELL` is the
only path-valued exception and must be exactly `/bin/zsh`, `/bin/bash` or `/bin/sh`. Every other
value is refused if it starts with `/` or `~`, contains `$`, a backtick or `://`, or has a `..`
path component. `Origin::Dev` does not relax environment values.

## `[capture.home]` and seeds

| Field | Required/default | Contract |
|---|---|---|
| `capture.home.run_home` | required | Closed values: `empty`, `keychain_link`. |
| `capture.home.config_home` | required | Closed values: `shared`, `per_run`. |
| `capture.home.env` | required | 1–64 bytes; `^[A-Z][A-Z0-9_]{0,63}$`, then the home-variable policy below. |
| `capture.home.per_run_dir` | optional; effective default `harness-home` | `per_run` only; 1–32 bytes; `^[a-z0-9][a-z0-9-]{0,31}$`. |
| `capture.home.dirs` | optional, default `[]` | `per_run` only; at most 8 relative paths, created mode `0700`. |
| `capture.home.seed` | optional, default `[]` | `per_run` only; at most 16 seed rows. |
| `capture.home.real_dirs` | required | 1–4 paths of the form `{user_home}/<relative>`; the daemon refuses a config home at or below them. |
| `capture.home.seed[].file` | required | Source below connector `files/` and the same destination below the config home. |
| `capture.home.seed[].mode` | optional, default `0600` | Closed values: `0600`, `0400`. |

Each `dirs`, seed `file` and `real_dirs` relative suffix has at most four, four and eight path
components respectively. A component is 1–64 bytes of `[A-Za-z0-9._-]` and is not `.` or `..`.
Seed and directory paths are refused case-insensitively if they name startup files, a `*.pth`
file, credential or tool-loaded directories, or `.config/git`. Seed destinations are unique.
Each seed file must exist, be regular and be at most 65,536 bytes. The directory walk reads and
hashes its bytes once; the per-run-home writer uses those captured bytes rather than reopening the
connector.

`capture.home.env` must have the shape of one harness's own home variable: one uppercase word
followed by `_HOME` or `_CONFIG_DIR`, such as `CODEX_HOME`, `HERMES_HOME` or
`CLAUDE_CONFIG_DIR`. Other tools' home/config/cache variables, `SHELL`, and the shared name-deny
rules are refused.

## Credential and CA

| Field | Required/default | Contract |
|---|---|---|
| `capture.credential.env` | required when the optional table exists | Closed registry: `DEEPSEEK_API_KEY`, `AMP_API_KEY`. The value: 20 to 256 bytes of `[A-Za-z0-9_-]`. |
| `capture.ca.env` | required | 1–2 unique names from the CA registry. |
| `capture.ca.mode` | required | Closed values: `extend`, `replace`; every name must have this mode in the registry. |

The CA registry is exactly:

| Variable | Mode |
|---|---|
| `CODEX_CA_CERTIFICATE` | `extend` |
| `NODE_EXTRA_CA_CERTS` | `extend` |
| `HERMES_CA_BUNDLE` | `replace` |

`SSL_CERT_FILE`, `SSL_CERT_DIR`, `CURL_CA_BUNDLE` and `REQUESTS_CA_BUNDLE` remain refused because
they replace trust for every tool child. A new CA variable or credential name is a core policy and
DECISIONS change, not a connector field value.

## Identity

| Field | Required/default | Contract |
|---|---|---|
| `capture.identity.source` | required | Closed values: `preassigned`, `stdout_json`, `stderr_line`, `none`. |
| `capture.identity.binding` | required except for `none` | Closed values: `at_launch`, `deferred`. |
| `capture.identity.match_field` | `stdout_json` only; required there | Dot path of 1–4 nonempty `[A-Za-z0-9_]+` segments. |
| `capture.identity.match_value` | `stdout_json` only; required there | 1–64 bytes, no control character. |
| `capture.identity.id_field` | `stdout_json` only; required there | Same dot-path grammar as `match_field`. |
| `capture.identity.prefix` | `stderr_line` only; required there | 1–32 printable ASCII bytes. |
| `capture.identity.grammar` | `stderr_line` only; required there | Closed values: `timestamp-hex`, `token`. |

`timestamp-hex` means `\d{8}_\d{6}_[0-9a-f]{6}`. `token` means 1–128 characters from
`[A-Za-z0-9_-]`; a lowercase UUID is a token. The only admitted source/binding pairs are:

| Source | Binding | Credential |
|---|---|---|
| `preassigned` | `at_launch` | refused |
| `stdout_json` | `at_launch` | admitted, and requires `config_home = "per_run"` |
| `stderr_line` | `deferred` | admitted, and requires `config_home = "per_run"` |
| `none` | absent | refused |

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

Pattern components admit `[A-Za-z0-9._-]`, `*`, `?` and the special `{id}` token. Empty, `.` or
`..` components, recursive `**`, other braces and absolute paths are refused. `{id}` appears
exactly once when the identity source names a session, and not at all when the source is `none`.
Globs do not cross `/`.

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
token and ends scanning. The guard is applied to every `args` item and to `goal_flag`. It can only
make the core argument policy stricter.

## `[capture.termination]`

If the table is absent, the effective strategy is `process_group`, `native_binary` is absent and
the grace is 10 seconds.

| Field | Default | Contract |
|---|---|---|
| `capture.termination.strategy` | `process_group` | Closed values: `process_group`, `native_pid`. |
| `capture.termination.native_binary` | absent | Required for `native_pid`, refused for `process_group`; 1–32 bytes of `[A-Za-z0-9._-]`. |
| `capture.termination.grace_seconds` | `10` | Integer 1–60. |

## `[[capture.hosts]]`

One to 16 unique host rows are required.

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
or trailing dot and cannot be IPv4 literals. An authentication host and intercepted host cannot
overlap; model hosts belong in `capture.hosts`, not `auth_hosts`.

## `[redaction]`

`redaction.extra_patterns` defaults to `[]`. Each inline row has two required fields:

| Field | Contract |
|---|---|
| `redaction.extra_patterns[].kind` | 1–48 bytes; `^[a-z][a-z0-9_]{0,47}$`. |
| `redaction.extra_patterns[].regex` | Must byte-equal one of the four core streaming recognizer sources below. |

```text
"rate_limits"\s*:\s*(\{(?:[^{}]|\{(?:[^{}]|\{[^{}]*\})*\})*\})
"plan_type"\s*:\s*"([^"\\]+)"
"credits_remaining"\s*:\s*("[^"\\]*"|[^,}\s]+)
x-anthropic-billing-[a-z0-9-]+:\s*[^\r\n"]+
```

A connector cannot introduce a regex. A new recognizer is a core PR that changes both the
daemon's streaming redactor and `EXTRA_PATTERN_SOURCES`.

## Cross-field rules

In addition to the local rules above, all of these apply:

1. `preassigned` requires `{session_id}` in `capture.args`. `{session_id}` anywhere in `args` or
   an environment value requires `preassigned`.
2. A credential requires `stderr_line` plus `deferred`, or `stdout_json` plus `at_launch`,
   identity and a `per_run` config home. One stdout reader feeds the key-fragment scan and the
   `stdout_json` identity matcher.
3. `capture.home.seed`, `dirs` and an explicit `per_run_dir` require `config_home = "per_run"`.
4. With `identity.source = "none"`, the native root must be `{run_home}/...`, or
   `{config_home}/...` with a per-run config home. A shared unscoped source could read another
   run's session and is refused.
5. The native root plus pattern, compared case-insensitively with `{id}` treated as `*`, cannot
   match a seed destination. A `{run_home}` root is checked both with the config home coincident
   with the run home and nested below `per_run_dir`.
6. `capture.home.env`, `capture.credential.env` and all `capture.ca.env` names are pairwise
   distinct case-insensitively. No `capture.env` key may duplicate one.
7. Every CA variable's registry mode equals the single declared `capture.ca.mode`.
8. Authentication and intercepted host sets are disjoint; every intercepted host has at least one
   scoped path.
9. A native hint is allowed only on `append_only`; a prefix path cannot be `/`.
10. A manifest is delivery-agnostic: no argument, goal flag, environment key/value, host or auth
    host may name Goal Cup, its reserved environment namespace or a tracker host. There is no
    schema field for a server, upload or delivery credential.

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

Schema evolution and pin evolution are separate. A new optional field whose absence preserves
current behaviour, a widened bound, a new closed value or a newly admitted cross-field pair may
remain schema 1. Removing or renaming a field, changing a default, or narrowing a bound requires
schema 2, migration of every harness manifest and regenerated directory pins. Tightening a core
policy row is not a schema bump, but it needs a DECISIONS entry and migrates every connector it
would reject in the same PR.

## Full checked example

This is the complete tracked
[`hermes-cli/harness.toml`](../connectors/hermes-cli/harness.toml), not a permissive sketch. The
focused `harness_conformance` and `connector_pins` tests load these bytes as `Origin::Pinned`,
validate every field and compare the whole connector directory with its compiled pin.

```toml
# Hermes Agent 0.19.0, capture only (D40, D41). Provenance: docs/qualification/m2-hermes-readiness.md.
schema_version = 1
id = "hermes-cli"

[capture]
# hermes chat -Q --provider deepseek --model deepseek-v4-flash --query=<goal>
args = ["chat", "-Q", "--provider", "deepseek", "--model", "deepseek-v4-flash"]
goal_flag = "--query"
continuation = "none: hermes chat -Q one turn, session JSON snapshot on, stdin /dev/null"

[capture.executable]
program = "hermes"
version = "0.19.0"
sha256 = "0444572f0bce8a805473fbcaa431beed399818528758b454fbf24825a51be325"

[capture.env]
TERMINAL_HOME_MODE = "profile"

[capture.home]
run_home = "empty"
config_home = "per_run"
env = "HERMES_HOME"
per_run_dir = "hermes"
dirs = ["home"]
real_dirs = ["{user_home}/.hermes"]
seed = [
  { file = "config.yaml" },
  { file = "auth.json" },
  { file = ".update_check" },
]

[capture.credential]
env = "DEEPSEEK_API_KEY"

[capture.ca]
env = ["HERMES_CA_BUNDLE"]
mode = "replace"

[capture.identity]
source = "stderr_line"
binding = "deferred"
prefix = "session_id: "
grammar = "timestamp-hex"

# Only the run's own snapshot; never a request_dump_* file (D41). No native decoder yet.
[[capture.native]]
name = "session"
root = "{config_home}/sessions"
pattern = "session_{id}.json"
form = "snapshot"

# Hermes scans raw argv for -p/--profile, --tui, --cli and -h before argparse, which abbreviates.
[capture.argv_guard]
long = ["profile", "tui", "cli", "help"]
short = ["p", "h"]
short_with_value = ["p", "z", "m", "t", "r", "s", "q", "c"]
long_abbreviations = true

[capture.termination]
strategy = "process_group"
grace_seconds = 10

[[capture.hosts]]
host = "api.deepseek.com"
evidence_paths = ["/v1/chat/completions"]
protocol_hint = "openai-chat-completions"
```

The canonical file remains the source: if its measured recipe changes, update this checked example
in the same documentation change rather than treating the copy as another connector definition.
