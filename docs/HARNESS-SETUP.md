# Harness setup

What `goalcup` does to a harness's environment and private home. The participant's steps are in the
[guide](PARTICIPANT-GUIDE.md#3-set-up); each harness's facts are in its [sheet](harnesses/README.md);
every connector key is in [CONNECTOR-SCHEMA](CONNECTOR-SCHEMA.md).

## The private home

`$GOALCUP_HOME` is, in order: the `GOALCUP_HOME` variable; on macOS `~/Library/Application Support/goalcup`;
else `$XDG_CONFIG_HOME/goalcup` or `~/.config/goalcup`.

A harness runs in `$GOALCUP_HOME/homes/<id>`, `<id>` the connector id, never in
`~/.claude`, `~/.codex` or any app-wide config. The folders are made with mode 0700. The connector's `home` map points the harness's own variables into it
(`CODEX_HOME`, `CLAUDE_CONFIG_DIR`, `HERMES_HOME`, `HOME` and `XDG_*` for Amp, and so on). One run holds
a home's lock (`<home>.goalcup.lock`); a second start on the same home is refused.

Where a connector's `home` sets `HOME` (Amp), goalcup writes `$HOME/.profile` putting the handed `PATH` back
after a login shell resets it.

## `goalcup setup`

```sh
goalcup setup --harness <id>      # --yes: ask and open nothing
goalcup setup --record-host HOST  # add a model host to $GOALCUP_HOME/record-hosts.txt, nothing else
```

Without `--harness` it uses the harness saved in `setup.json`, or asks.
With a terminal and no `--yes`, it:

1. Prints the harness name, version and path; prints any `version_notes` line that holds.
2. Opens the harness once in its private home, for sign-in. Quit it to go on.
3. Writes the connector's `update_off` rows.
4. Asks one question, then writes the `no_prompts` rows on yes.
5. Saves the harness id and the rows written in `$GOALCUP_HOME/setup.json`.
6. Prints one warning line each for: sleep lock, server unreachable, `git` missing, the WSL folder.

With `--yes` it writes both row lists and opens nothing. Without a terminal and without `--yes` it
writes nothing and checks only. A row already holding its value is left alone; a row it cannot write
is a line, not a failure. Rows are written only into the private home, never through a link, keeping
the other keys (`.json`, `.toml`, `.yaml`, `.yml`).

Sign-in, an API key or a gateway is set inside that home as without goalcup. A model host goalcup does
not know is added with `--record-host`; a host a harness's config file names (the connector's
`host_hints`) is picked up by itself.

## What goalcup sets at launch

The harness's environment is the participant's, with:

| Variable | Value |
|---|---|
| `HTTPS_PROXY`, `https_proxy` | `http://127.0.0.1:<port>`, the recording proxy |
| `NO_PROXY`, `no_proxy` | `127.0.0.1,localhost,::1` |
| `NODE_USE_ENV_PROXY` | `1` |
| `NODE_EXTRA_CA_CERTS` | the run CA, `run-ca.pem` |
| `SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE` | `bundle.pem`: the run CA, then the OS roots |
| each name in the connector's `ca_env` | the run CA, `run-ca.pem` |
| each key of the connector's `home` | its template under the private home |
| each key of the connector's `env` | its fixed value (auto-update switches and the like) |

Both CA files are in the part's folder. Removed first: `HTTPS_PROXY`,
`HTTP_PROXY`, `ALL_PROXY`, `NO_PROXY` (both cases), `NODE_EXTRA_CA_CERTS`, `SSL_CERT_FILE`,
`REQUESTS_CA_BUNDLE`, and the connector's `unset` names. `HTTP_PROXY` is never set. `SSL_CERT_DIR`,
`CURL_CA_BUNDLE` and `NODE_TLS_REJECT_UNAUTHORIZED` are never touched, so a value the participant
exported passes through. goalcup never sets a model base URL. `GOALCUP_TIME_LEFT_FILE` is set by `start`.

The harness gets the `PATH` of the terminal that ran `goalcup start`. `goalcup` must be on it for the agent
to run `goalcup time-left`.

Before launch the connector's `refuse_if_set` files are read: a proxy or CA value there refuses the start,
naming the file and key. The goal-cup skill is copied into `skills_dir` when the connector has one. The connector's `first_screen` line is printed before the harness opens. `before` and
`after` commands (Cline `hub stop`, Hermes `gateway stop -y`) run around the harness.

## What setup writes, per harness

From `connectors/*.toml`. "Prompts" is the `no_prompts` rows; "Update" the `update_off` rows (an env switch
in `env` is set at launch instead). A blank means the connector has none: for Amp, Cline, Crush, Gemini, Goose,
OpenClaw and OpenCode 2 the facts sheet says what to set yourself.

| Id | Home variable | Prompts (file: key = value) | Update |
|---|---|---|---|
| `claude-code-cli` | `CLAUDE_CONFIG_DIR` | `settings.json`: `permissions.defaultMode = bypassPermissions`, `skipDangerousModePermissionPrompt = true` | env |
| `codex-cli` | `CODEX_HOME` | `config.toml`: `approval_policy = never`, `sandbox_mode = danger-full-access` | `config.toml`: `check_for_update_on_startup = false` |
| `hermes-cli` | `HERMES_HOME` | `config.yaml`: `approvals.mode = off` | `config.yaml`: `updates.check = false` |
| `amp-cli` | `HOME`, `XDG_*` | | env |
| `copilot-cli` | `COPILOT_HOME` | `settings.json`: `defaultPermissionMode = allow-all` | env |
| `droid-cli` | `FACTORY_HOME_OVERRIDE` | `.factory/settings.json`: `sessionDefaultSettings.autonomyLevel = high` | env |
| `gemini-cli` | `GEMINI_CLI_HOME` | | `.gemini/settings.json`: `general.enableAutoUpdate = false` |
| `kilo-cli` | `XDG_*` | `.config/kilo/opencode.json`: `permission = allow` | env |
| `kimi-code-cli` | `KIMI_CODE_HOME` | `config.toml`: `default_permission_mode = auto` | env |
| `opencode-cli` | `XDG_*` | `.config/opencode/opencode.json`: `permission = allow` | env |
| `qwen-cli` | `QWEN_HOME` | `settings.json`: `tools.approvalMode = yolo` | `settings.json`: `general.enableAutoUpdate = false` |
| `cline-cli`, `crush-cli`, `goose-cli`, `openclaw-cli`, `opencode2-cli` | see the connector | | env, where the connector has one |

Files are under the private home (`$GOALCUP_HOME/homes/<id>`).

## Hermes

Before and after the harness, goalcup runs `hermes gateway stop -y` (`connectors/hermes-cli.toml`). Hermes
0.22.6 is said not to reach its model through goalcup; the connector prints a note for it
(`version_notes`). The first-run setup screen and its telemetry question are in the
[sheet](harnesses/hermes-cli.md).
