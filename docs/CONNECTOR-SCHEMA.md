# Connector schema

A harness is one data file, `connectors/<id>.toml`, and every harness goes through the same code. A
new connector is its TOML file plus one line in `crates/goalcup/src/connector/builtin.rs`, the one
source file that names connector files; `crates/goalcup/tests/connector_builtin.rs` fails when a file
is missing from that list or its `id` differs. Tiers live in
[`connectors/tiers.toml`](../connectors/tiers.toml), not in a connector. Nothing is pinned. The
`start` event names the connector (`connector`, `connector_source`, `connector_sha256`) and the host
list's hash (`hosts_sha256`). `goalcup start`, `setup` and `continue` are not built yet: the rules
below are what the connector loader and launch plan do, and a run applies them once `start` is built.

## The connector contract

The keys below are what a connector may set and rely on. `id`, `name` and `program` are required
and not empty; every other key is optional. An unknown key, at the top or in a row, is refused at
load. Templates: `{home}` is the harness's isolated home, `$GOALCUP_HOME/homes/<id>`; `{workspace}`
is the participant's project folder.

| Key | Type | Default | What it does |
|---|---|---|---|
| `id` | string `[a-z0-9-]{1,32}` | | The connector id: `goalcup start --harness <id>` |
| `name` | string | | Shown to the participant |
| `program` | string | | The executable, found on the participant's `PATH` |
| `version` | string | none | Text the first non-empty line of `program --version` must contain, else the start refuses, naming the program. A version that cannot be read (no output, a failed exit, 5 s without an exit) refuses nothing |
| `args` | string list | `[]` | Arguments for `goalcup start`. Template `{workspace}` only |
| `resume` | string list | `[]` | Appended to `args` for `goalcup continue`. Template `{workspace}` only |
| `home` | map, variable to template | empty | Variables pointed into the isolated home; each value starts with `{home}`. The folders are created with mode 0700 |
| `env` | map, variable to string | empty | Fixed variables (auto-update switches, daemon modes), set last |
| `ca_env` | string list | `[]` | The harness's own CA variables. Each is pointed at the run CA |
| `unset` | string list | `[]` | Participant variables removed before launch, for those that beat `HTTPS_PROXY` |
| `refuse_if_set` | list of `{file, keys}` | `[]` | A value at any of `keys` in `file` refuses the start, naming the file and key, never the value. `null`, `""`, `[]` and `{}` are unset; `false` is a value |
| `host_hints` | list of `{file, key}` | `[]` | Config file keys whose `https` URL values add the host to the `config` list. A loopback value gives a `local_model` note instead; any other value that adds nothing is a log line naming the file and key |
| `hint_env` | string list | `[]` | Participant variables read as `host_hints` values are. Only the names listed |
| `secret_files` | string list | `[]` | The harness's own key files. The default build reads none of them for masking |
| `before`, `after` | list of string lists | `[]` | Arguments without the program: the watchdog runs `program` followed by each, before launch and after the harness exits |
| `native` | list of `{name, root, pattern, form, warn_missing}` | `form = "append"`, `warn_missing = true` | Session-file rows; `name` is unique in the connector. `root` starts with `{home}` or `{workspace}`; `pattern` is relative to it, `/`-separated, with no `.`, `..`, empty component, brace or NUL, and `**` only as a whole component. A pattern ending in `**` is refused. `form` is `append` (read as it grows), `snapshot` (read whole when it changes) or `final` (a database, copied as a set). `warn_missing = false` silences the "no session file yet" line for a row that may stay empty. The first row that matches a file reads it |
| `mask` | list of `{kind, json_field, value}` or `{kind, header_prefix}` | `[]` | Connector mask rules, one form per rule. `kind` matches `^[a-z][a-z0-9_]{0,47}$` and names every marker the rule writes. `json_field` matches `^[A-Za-z0-9_-]{1,64}$` and masks `"field": value`, with `value` either `string` (a non-empty string without escapes) or `scalar` (such a string, else a bare token such as a number). `header_prefix` matches `^[a-z][a-z0-9-]{0,63}$` and masks the value of every header whose name is the prefix followed by `[a-z0-9-]+` |
| `first_screen` | string | none | One line printed just before the harness opens: a first-run question to expect |
| `no_prompts` | list of `{file, key, value}` | `[]` | Settings `goalcup setup` writes into the isolated home after its one question, so the harness never stops to ask approval |
| `update_off` | list of `{file, key, value}` | `[]` | An auto-update switch that exists only as a setting. Written into the isolated home with no question, again at every `goalcup start` and `continue`. Where an env switch exists, it goes in `env` |
| `skills_dir` | template | none | Where the goal-cup skill is installed, under `{home}`. Absent: no skill is installed |

A key is dotted (`env.HTTPS_PROXY`); a part may hold `*`, matching any key or list index. A hint or
refusal file is read by extension: `.json` and `.jsonc` as JSON with comments and trailing commas,
`.toml`, `.yaml` and `.yml` as block YAML, else as `KEY=value` lines; a link, a file over 1 MiB or
one that cannot be read or parsed is skipped, and the plan names it. Settings rows write `.json`,
`.toml` and `.yaml` files only, never through a link, keeping the other keys; `"true"` and `"false"`
become the format's boolean, any other value a string. A row not written is a log line, not a refusal.

## Load checks

Besides the table's types, patterns and templates, the loader checks these rules and nothing else.
A parse error names the file and the line; a rule error names the file and the key.

- **No model address.** goalcup never sets a model base URL. A connector is refused when an `env`
  name contains `BASE_URL`, `API_BASE`, `ENDPOINT`, `_HOST` or `PROXY` (any case), or when an
  `env`, `no_prompts` or `update_off` value contains `://`. This check runs first, so `HTTPS_PROXY`
  in `env` gets this error.
- `env` names no variable the code owns: the proxy and CA variables it removes and sets
  (`HTTPS_PROXY`, `HTTP_PROXY`, `ALL_PROXY`, `NO_PROXY` in both cases, `NODE_EXTRA_CA_CERTS`,
  `SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE`), `NODE_USE_ENV_PROXY`, `HOME`, the three that turn TLS checks
  off (`SSL_CERT_DIR`, `CURL_CA_BUNDLE`, `NODE_TLS_REJECT_UNAUTHORIZED`), and none in `home`, `ca_env`
  or `unset`. `ca_env` names none of these and no `home` key; a `home` key none but `HOME`.
- Paths stay inside: every component after the leading template is not empty, `.` or `..`, and holds
  no brace or NUL. A `refuse_if_set` or `host_hints` file and a `secret_files` entry start with
  `{home}` or `{workspace}`; a `no_prompts` or `update_off` file and `skills_dir` start with `{home}`,
  so nothing is written into the participant's project. Each may hold `*` only in its last component.

## The host list

[`connectors/model-hosts.toml`](../connectors/model-hosts.toml) names the hosts decrypted and
recorded byte for byte for every harness; every other host is a tunnel. It has five arrays, each
optional, and no other key. `provider`, `gateway` and `vendor` hold the hosts, the array name being
each host's `kind`; `never` holds hosts never decrypted, whatever another list says; `pinned` holds
names or patterns of hosts known to pin their certificate, which marks a listed host pinned (a
`pinned` name in no other array is listed with kind `other`). goalcup's own server
(`agentcommunity.org` and its subdomains, any name holding `goalcup`, and the server a run uploads
to) is `never` in code. A pattern is an exact name, `*.a.b` (one or more labels before `a.b`), or one
`*` inside one label with at least two labels after it. A name outside `never` that fails the host
check refuses the file. The `start` event holds the lists and the file's hash.

The hosts in force come from `shared` (this file, or a `--hosts` file), `participant`
(`$GOALCUP_HOME/record-hosts.txt`, written by `goalcup setup --record-host`) and `config` (the
hints), the last two with kind `other`. `never` answers first, then the shared, participant and
config lists in that order; a host in none is a tunnel. One host check serves every source: the name
is lowercased; labels of 1 to 63 letters, digits or `-`, at most 253 characters, at least two labels,
no trailing dot, no IP literal or numeric spelling of an address, no `localhost` form, no backslash,
space or control byte, not goalcup's server. `participant` takes an exact name or one `*.` before at
least two labels, `config` exact names only. A participant line that fails is skipped, a config value
that fails is a log line, and a `never` host from either is left out with a log line.

## Loading a file

`goalcup start` and `goalcup continue` take `--connector <file.toml>` and `--hosts <file.toml>`, with
the same parser and checks; the `start` event gives `connector_source = "file"`. Both flags are for
practice and test runs only: without `--practice` or `--test`, the start refuses before anything is
read. `continue` has no `--practice` or `--test`: its two flags must name the same files, unchanged,
as the run's first entry (none if it used none). **(next slice, ruled in PRD-10, not PRD-11: `goalcup
continue` is not built yet, so nothing is read)**

## Example

An excerpt of [`connectors/claude-code-cli.toml`](../connectors/claude-code-cli.toml):

```toml
id = "claude-code-cli"
name = "Claude Code"
program = "claude"
home = { CLAUDE_CONFIG_DIR = "{home}" }
native = [{ name = "transcripts", root = "{home}/projects", pattern = "**/*.jsonl" }]
mask = [{ kind = "anthropic_billing_header", header_prefix = "x-anthropic-billing-" }]
```
