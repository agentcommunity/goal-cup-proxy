# Connector schema

A harness is one data file, `connectors/<id>.toml`. Every harness goes through the same code. The
files are compiled into the binary by one list, `crates/goalcup/src/connector/builtin.rs`, the one
source file that names connector files: a new connector is its TOML file plus one line there, and
`tests/connector_builtin.rs` fails when a file is missing from the list or its `id` differs. The one
shared host list is `connectors/model-hosts.toml`. Tiers are not connector data: they live in
`connectors/tiers.toml` (see [the harness table](harnesses/README.md)). Nothing is pinned. The
`start` event names what was used: `connector`, `connector_source`, `connector_sha256` (SHA-256 of
the exact TOML bytes), `hosts` and `hosts_sha256`.

## Keys

`id`, `name` and `program` are required; every other key is optional. An unknown key, at the top or
in a row, is refused at load. Templates: `{home}` is the harness's isolated home,
`$GOALCUP_HOME/homes/<id>`; `{workspace}` is the participant's project folder.

| Key | Type | Default | What it does |
|---|---|---|---|
| `id` | string `[a-z0-9-]{1,32}` | | The connector id: `goalcup start --harness <id>` |
| `name` | string | | Shown to the participant |
| `program` | string | | The executable, found on the participant's `PATH` |
| `version` | string | none | Text the first line of `program --version` must contain, else the start refuses |
| `args` | string list | `[]` | Arguments for `goalcup start`. Template `{workspace}` only |
| `resume` | string list | `[]` | Appended to `args` for `goalcup continue`. Template `{workspace}` only |
| `home` | map, variable to template | empty | Variables pointed into the isolated home; each value starts with `{home}`. The folders are created with mode 0700 |
| `env` | map, variable to string | empty | Fixed variables (auto-update switches, daemon modes), set last |
| `ca_env` | string list | `[]` | The harness's own CA variables. Each is pointed at the run CA |
| `unset` | string list | `[]` | Participant variables removed before launch, for those that beat `HTTPS_PROXY` |
| `refuse_if_set` | list of `{file, keys}` | `[]` | A non-empty value at any of `keys` in `file` refuses the start, naming the file and key, never the value |
| `host_hints` | list of `{file, key}` | `[]` | Config file keys whose URL values add a host to the `config` list |
| `hint_env` | string list | `[]` | Participant variables whose URL values add a host to the `config` list. Only the names listed |
| `secret_files` | string list | `[]` | The harness's own key files. The default build reads none of them for masking |
| `before`, `after` | list of string lists | `[]` | Arguments without the program: the watchdog runs `program` followed by each, before launch and after the harness exits |
| `native` | list of `{name, root, pattern, form, warn_missing}` | `form = "append"`, `warn_missing = true` | Session-file rows. `root` starts with `{home}` or `{workspace}`; `pattern` is relative to it, `/`-separated, `**` a whole component. `form` is `append` (read as it grows), `snapshot` (read whole when it changes) or `final` (a database, copied as a set). `warn_missing = false` silences the "no session file yet" line for a row that may stay empty. The first row that matches a file reads it |
| `mask` | list of `{kind, json_field, value}` or `{kind, header_prefix}` | `[]` | Connector mask rules. `kind` matches `^[a-z][a-z0-9_]{0,47}$` and names every marker the rule writes. `json_field` matches `^[A-Za-z0-9_-]{1,64}$` and masks `"field": value`, with `value` either `string` (a non-empty string without escapes) or `scalar` (such a string, else a bare token such as a number). `header_prefix` matches `^[a-z][a-z0-9-]{0,63}$` and masks the value of every header whose name is the prefix followed by `[a-z0-9-]+` |
| `first_screen` | string | none | One line printed just before the harness opens: a first-run question to expect |
| `no_prompts` | list of `{file, key, value}` | `[]` | Settings `goalcup setup` writes into the isolated home after its one question, so the harness never stops to ask approval |
| `update_off` | list of `{file, key, value}` | `[]` | An auto-update switch that exists only as a setting. Written into the isolated home with no question, again at every `goalcup start` and `continue`. Where an env switch exists, it goes in `env` |
| `skills_dir` | template | none | Where the goal-cup skill is installed, under `{home}`. Absent: no skill is installed |

## Load checks

The loader checks these rules and nothing else. An error names the file and the key, never a value.

- The file is UTF-8 TOML; types match the table; no unknown key.
- `id` matches `[a-z0-9-]{1,32}`; `name` and `program` are not empty.
- **No model address.** goalcup never sets a model base URL. A connector is refused when an `env`
  name contains `BASE_URL`, `API_BASE`, `ENDPOINT`, `_HOST` or `PROXY` (any case), or when an
  `env`, `no_prompts` or `update_off` value contains `://`. The error says the connector sets a
  model address, which goalcup never does.
- `env` names no variable the code sets: `HTTPS_PROXY`, `HTTP_PROXY`, `ALL_PROXY`, `NO_PROXY` (both
  cases of each), `NODE_EXTRA_CA_CERTS`, `SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE`, `NODE_USE_ENV_PROXY`,
  `HOME`, `SSL_CERT_DIR`, `CURL_CA_BUNDLE`, `NODE_TLS_REJECT_UNAUTHORIZED`; and none named in `home`,
  `ca_env` or `unset`. `ca_env` names none of these and no `home` key. A `home` key names none of
  them except `HOME`.
- Paths stay inside: every component after the leading template is not empty, `.` or `..`, and holds
  no brace or NUL. `home` values and `skills_dir` start with `{home}`. Every `file` (in
  `refuse_if_set`, `host_hints`, `no_prompts`, `update_off`) and every `secret_files` entry starts
  with `{home}` or `{workspace}` and may hold `*` only in its last component. `args` and `resume`
  hold no template but `{workspace}`.
- `native` names are unique within the connector; each `root` starts with `{home}` or `{workspace}`;
  each `pattern` has no `.`, `..`, empty component, brace or NUL, and `**` only as a whole component.
- Each `mask` rule has `json_field` with `value`, or `header_prefix`, never both, in the forms above.

There are no row caps, no deny lists and no checks on harness names.

## Config file keys

A `key` in `refuse_if_set` and `host_hints` is a path into the config file. Dots separate the parts;
`*` matches any one key or array index; a part may hold `*` as a glob (`env.ANTHROPIC_*BASE_URL`).
The file's format follows its extension: `.json`, `.toml`, `.yaml` or `.yml`, and anything else is
read as dotenv (`KEY=value` lines). A file is opened without following a link (`O_NOFOLLOW`) and read
as UTF-8 with a BOM stripped. A link, or a file that cannot be read or parsed, is skipped with a log
line and one start line, never a refusal.

## The host list

[`connectors/model-hosts.toml`](../connectors/model-hosts.toml) names the hosts decrypted and
recorded byte for byte for every harness. Every other host is a tunnel. It has five arrays, each
optional, and no other key:

- `provider`, `gateway` and `vendor`: the hosts. The array name is each host's `kind`.
- `never`: hosts never decrypted, whatever another list says. goalcup's own server
  (`agentcommunity.org`, `*.agentcommunity.org` and the server a run uploads to) is added to `never`
  in code.
- `pinned`: names or patterns of hosts known to pin their certificate. A listed host that matches
  one is marked pinned; a `pinned` name in no other array is listed with kind `other`.

A pattern is an exact name, `*.a.b` (one or more labels before `a.b`), or one `*` inside one label.
The hosts in force for a run come from three sources: `shared` (this file, or a `--hosts` file),
`participant` (`$GOALCUP_HOME/record-hosts.txt`, written by `goalcup setup --record-host`) and
`config` (the hints above). Hosts from the last two get kind `other`. One host check serves every
source: a name is lowercased and needs at least two labels of 1 to 63 letters, digits or `-`, and
at most 253 characters in all. Only the shared file may use a `*`. A hint whose host is loopback or a
private address is not recorded; it adds one `local_model` note to the start instead.

## Loading a file

`goalcup start --connector <file.toml>` loads a connector from disk, and `--hosts <file.toml>` a
host list. `goalcup continue` takes the same two flags for a run whose first part used them. The
parser and the checks are the same; the `start` event then says `connector_source: "file"`, with the
file's hash. Both flags are for practice and test runs only: without `--practice` or `--test`, the
start refuses before anything is read.

## Example

The full file is [`connectors/claude-code-cli.toml`](../connectors/claude-code-cli.toml). An excerpt:

```toml
id = "claude-code-cli"
name = "Claude Code"
program = "claude"
resume = ["--continue"]
home = { CLAUDE_CONFIG_DIR = "{home}" }
unset = ["CLAUDE_CODE_CERT_STORE"]
host_hints = [
  { file = "{home}/settings.json", key = "env.ANTHROPIC_*BASE_URL" },
  { file = "{workspace}/.claude/settings.json", key = "env.ANTHROPIC_*BASE_URL" },
  { file = "{workspace}/.claude/settings.local.json", key = "env.ANTHROPIC_*BASE_URL" },
]
secret_files = ["{home}/.credentials.json"]
native = [
  { name = "tool-results", root = "{home}/projects", pattern = "**/tool-results/*", form = "snapshot", warn_missing = false },
  { name = "meta", root = "{home}/projects", pattern = "**/*.meta.json", form = "snapshot", warn_missing = false },
  { name = "transcripts", root = "{home}/projects", pattern = "**/*.jsonl" },
]
mask = [{ kind = "anthropic_billing_header", header_prefix = "x-anthropic-billing-" }]
skills_dir = "{home}/skills"
```
