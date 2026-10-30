# Connector schema

A harness is one data file, `connectors/<id>.toml`, and every harness goes through the same code. A
new connector is its TOML file plus one line in `crates/goalcup/src/connector/builtin.rs`, the one
source file that names connector files; `crates/goalcup/tests/connector_builtin.rs` fails when a file
is missing from that list or its `id` differs. The shared host list is `connectors/model-hosts.toml`.
Tiers live in [`connectors/tiers.toml`](../connectors/tiers.toml), not in a connector. Nothing is
pinned. The `start` event names the connector used (`connector`, `connector_source`,
`connector_sha256`) and the host list's hash (`hosts_sha256`).

Parts of the connector code are still stubs. A rule marked **(next slice)** is ruled in PRD-11 and
arrives with its next merge; until then the code does what the mark says.

## The connector contract

The keys below are what a connector may set and rely on. `id`, `name` and `program` are required
and not empty; every other key is optional. An unknown key, at the top or in a row, is refused at
load. Templates: `{home}` is the harness's isolated home,
`$GOALCUP_HOME/homes/<id>`; `{workspace}` is the participant's project folder.

| Key | Type | Default | What it does |
|---|---|---|---|
| `id` | string `[a-z0-9-]{1,32}` | | The connector id: `goalcup start --harness <id>` |
| `name` | string | | Shown to the participant |
| `program` | string | | The executable, found on the participant's `PATH` |
| `version` | string | none | Text the first line of `program --version` must contain, else the start refuses. **(next slice: the version is not read yet, so nothing is refused)** |
| `args` | string list | `[]` | Arguments for `goalcup start`. Template `{workspace}` only |
| `resume` | string list | `[]` | Appended to `args` for `goalcup continue`. Template `{workspace}` only |
| `home` | map, variable to template | empty | Variables pointed into the isolated home; each value starts with `{home}`. The folders are created with mode 0700 |
| `env` | map, variable to string | empty | Fixed variables (auto-update switches, daemon modes), set last |
| `ca_env` | string list | `[]` | The harness's own CA variables. Each is pointed at the run CA |
| `unset` | string list | `[]` | Participant variables removed before launch, for those that beat `HTTPS_PROXY` |
| `refuse_if_set` | list of `{file, keys}` | `[]` | A non-empty value at any of `keys` in `file` refuses the start, naming the file and key. **(next slice: nothing is read yet)** |
| `host_hints` | list of `{file, key}` | `[]` | Config file keys whose URL values add a host to the `config` list. **(next slice: nothing is read yet)** |
| `hint_env` | string list | `[]` | Participant variables whose URL values add a host to the `config` list. Only the names listed. **(next slice: nothing is read yet)** |
| `secret_files` | string list | `[]` | The harness's own key files. The default build reads none of them for masking |
| `before`, `after` | list of string lists | `[]` | Arguments without the program: the watchdog runs `program` followed by each, before launch and after the harness exits |
| `native` | list of `{name, root, pattern, form, warn_missing}` | `form = "append"`, `warn_missing = true` | Session-file rows; `name` is unique in the connector. `root` starts with `{home}` or `{workspace}`; `pattern` is relative to it, `/`-separated, with no `.`, `..`, empty component, brace or NUL, and `**` only as a whole component. A pattern ending in `**` is refused **(next slice: today it loads)**. `form` is `append` (read as it grows), `snapshot` (read whole when it changes) or `final` (a database, copied as a set). `warn_missing = false` silences the "no session file yet" line for a row that may stay empty. The first row that matches a file reads it |
| `mask` | list of `{kind, json_field, value}` or `{kind, header_prefix}` | `[]` | Connector mask rules, one form per rule. `kind` matches `^[a-z][a-z0-9_]{0,47}$` and names every marker the rule writes. `json_field` matches `^[A-Za-z0-9_-]{1,64}$` and masks `"field": value`, with `value` either `string` (a non-empty string without escapes) or `scalar` (such a string, else a bare token such as a number). `header_prefix` matches `^[a-z][a-z0-9-]{0,63}$` and masks the value of every header whose name is the prefix followed by `[a-z0-9-]+` |
| `first_screen` | string | none | One line printed just before the harness opens: a first-run question to expect |
| `no_prompts` | list of `{file, key, value}` | `[]` | Settings `goalcup setup` writes into the isolated home after its one question, so the harness never stops to ask approval |
| `update_off` | list of `{file, key, value}` | `[]` | An auto-update switch that exists only as a setting. Written into the isolated home with no question, again at every `goalcup start` and `continue`. Where an env switch exists, it goes in `env` |
| `skills_dir` | template | none | Where the goal-cup skill is installed, under `{home}`. Absent: no skill is installed |

**(Next slice: no `file` is read or written yet.)** Any `file` whose name ends `.jsonc` is read as
JSON with comments. For `no_prompts` and `update_off`, each `file` starts with `{home}`, since a
`{workspace}` row would write into the participant's project (today `{workspace}` also loads); a
`value` of `"true"` or `"false"` is written as the file format's boolean, any other value as a string.

## Load checks

Besides the types, patterns and templates in the table, the loader checks these rules and nothing
else. A parse error names the file and the line; a rule error names the file and the key.

- **No model address.** goalcup never sets a model base URL. A connector is refused when an `env`
  name contains `BASE_URL`, `API_BASE`, `ENDPOINT`, `_HOST` or `PROXY` (any case), or when an
  `env`, `no_prompts` or `update_off` value contains `://`. This check runs first, so `HTTPS_PROXY`
  in `env` gets this error.
- `env` names no variable the code owns: the proxy and CA variables it removes and sets
  (`HTTPS_PROXY`, `HTTP_PROXY`, `ALL_PROXY`, `NO_PROXY` in both cases, `NODE_EXTRA_CA_CERTS`,
  `SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE`), `NODE_USE_ENV_PROXY`, `HOME`, and the three that turn TLS
  checks off (`SSL_CERT_DIR`, `CURL_CA_BUNDLE`, `NODE_TLS_REJECT_UNAUTHORIZED`); and none named in
  `home`, `ca_env` or `unset`. `ca_env` names none of these and no `home` key. A `home` key names
  none of them except `HOME`.
- Paths stay inside: every component after the leading template is not empty, `.` or `..`, and holds
  no brace or NUL. Every `file` and every `secret_files` entry starts with `{home}` or `{workspace}`
  and may hold `*` only in its last component.

## The host list

[`connectors/model-hosts.toml`](../connectors/model-hosts.toml) names the hosts decrypted and
recorded byte for byte for every harness. Every other host is a tunnel. It has five arrays, each
optional, and no other key:

- `provider`, `gateway` and `vendor`: the hosts. The array name is each host's `kind`.
- `never`: hosts never decrypted, whatever another list says. goalcup's own server
  (`agentcommunity.org`, `*.agentcommunity.org` and the server a run uploads to) is added in code.
- `pinned`: names or patterns of hosts known to pin their certificate. A listed host that matches
  one is marked pinned; a `pinned` name in no other array is listed with kind `other`.

A pattern is an exact name, `*.a.b` (one or more labels before `a.b`), or one `*` inside one label.
The code reads this file, checks each `provider`, `gateway`, `vendor` and `pinned` name and puts the
lists and the file's hash in the `start` event. **(Next slice: no host is decrypted yet; every host is a tunnel.)**

**(Next slice)** The hosts in force come from three sources: `shared` (this file, or a `--hosts`
file), `participant` (`$GOALCUP_HOME/record-hosts.txt`, written by `goalcup setup --record-host`) and
`config` (the hints). Hosts from the last two get kind `other`. One host check serves every source:
lowercase, labels of 1 to 63 letters, digits or `-`, at most 253 characters, at least two labels, no
trailing dot, no IP literal or numeric spelling of an address, no `localhost` form, no backslash,
space or control byte, not a `never` host, not goalcup's server. Only the shared file takes the
pattern forms above; `participant` takes an exact name or one `*.` before at least two labels;
`config` takes exact names. Today the check is the label shape, the length and the two labels; a
shared name may hold `*` anywhere in any label, so the pattern forms are not checked at load.

## Loading a file

`goalcup start` and `goalcup continue` take `--connector <file.toml>` and `--hosts <file.toml>`, with
the same parser and checks; the `start` event gives `connector_source = "file"`. Both flags are for
practice and test runs only: without `--practice` or `--test`, the start refuses before anything is
read.

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
