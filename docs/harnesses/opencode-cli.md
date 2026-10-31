# OpenCode harness facts

Connector ids: `opencode-cli` (OpenCode 1.x) and `opencode2-cli` (OpenCode 2.x) · Facts for versions: `1.19.5` and `2.2.2` · Written by: Goal Cup team · Last checked: 2026-10-31
Public disclosure: yes. Facts come from OpenCode's public docs, its public source and a run on 1.19.4 in a Linux container on 28 Oct 2026. No keys, tokens or personal paths are included.
Status: shipped connectors ([`opencode-cli.toml`](../../connectors/opencode-cli.toml), [`opencode2-cli.toml`](../../connectors/opencode2-cli.toml)). This one sheet covers both; no separate OpenCode 2 sheet is written.

Source words: "docs" is [opencode.ai/docs](https://opencode.ai/docs/cli/) for 1.x and [opencode.ai/v2/docs](https://opencode.ai/v2/docs/cli) for 2.x, read on 2026-10-31; "measured" is the 28 Oct run on 1.19.4; "source" is the public `dev` branch, which may be ahead of the 2.2.2 package; "unknown" is not measured or read. Nothing was run on 2.2.2.

## 1. Run one prompt, in chat mode first

- `opencode` with no argument starts the TUI in the current folder ([docs](https://opencode.ai/docs/cli/)). `opencode --continue` (`-c`) reopens the last session, `--session <id>` a named one. Both connectors use `--continue` to resume (2.x: not on its CLI page, taken from 1.x and the source).
- OpenCode 2 is started `opencode --standalone` (the connector's `args`): it "run[s] with a private server" instead of the shared background service ([docs](https://opencode.ai/v2/docs/cli)).
- `opencode run "<prompt>"` is the one-shot form; it is not chat mode and is not used. [RULES](../RULES.md) lists `--pure`, `OPENCODE_PURE=1`, `OPENCODE_DISABLE_PROJECT_CONFIG=1`, `--attach` and `share` as modes that switch recording off.

## 2. Install and record a version

- 1.x: `curl -fsSL https://opencode.ai/install | bash`. 2.x: `curl -fsSL https://opencode.ai/v2/install | bash` or `npm install -g @opencode/cli` ([docs](https://opencode.ai/v2/docs)). A mise install older than 2026.10.9 gets OpenCode 1.
- Both print the version with `opencode --version`. The two connectors share the program name and are told apart only by the version word-prefix rule: `"1."` for 1.x, `"2."` for 2.x. A mismatch refuses the start and names the program; the pad's first row of each is the proof on a real binary.
- 1.x releases about weekly, 2.x often twice a day (npm, 29 Oct); expect several builds in a run.

## 3. Where it keeps things, per OS

| | macOS | Linux | Windows |
|---|---|---|---|
| Config dir | unknown | `$HOME/.config/opencode/opencode.json` or `.jsonc` ([docs](https://opencode.ai/v2/docs/config)) | unknown |
| Session store | unknown | `$HOME/.local/share/opencode/opencode.db`, SQLite (measured, 1.x) | unknown |
| Logs / cache | unknown | `.local/share/opencode/log/`, `.cache/opencode/`, `.local/state/opencode/` (measured); `/tmp/opencode` | unknown |
| Env vars that move these | | `XDG_*`; `OPENCODE_DB` moves the database; `OPENCODE_CONFIG` the file ([docs](https://opencode.ai/docs/cli/)) | |

Both connectors set the four XDG folders under the isolated home and read `opencode*.db*` there as a final copy. A 2.x database at the same path is taken from a third-party write-up and unverified.

## 4. Session format and identity

- One SQLite database for every session (tables `session`, `message`, `part`, `event`, `credential` and more). No JSON session files (measured, 1.x). Session ids look like `ses_<26 chars>`.
- Subagents are child sessions in the same database with a parent id (1.x source, `task.ts`). They run in the same process, so they inherit the proxy and CA ([docs](https://opencode.ai/docs/agents/): General and Explore, through the Task tool or an `@` mention). Subagents through goalcup: unmeasured.

## 5. Auth

- `/connect` or `opencode auth login` stores a key in `$HOME/.local/share/opencode/auth.json`; a key may also come from the environment or from config (`apiKey`). A key in `auth.json` works with no environment variable (measured).
- A variable that moves config and login together: no. `OPENCODE_CONFIG_DIR` moves config only; `auth.json` follows `XDG_DATA_HOME`, so the connectors set the XDG folders.

## 6. Network

- CA: `NODE_EXTRA_CA_CERTS` adds to the roots, "for both proxy connections and direct API access" ([docs](https://opencode.ai/docs/network/)); `SSL_CERT_FILE` alone also worked (measured). Bun-compiled binary, so `NODE_USE_ENV_PROXY` is not needed; `HTTPS_PROXY` is honoured on its own.
- `NO_PROXY=localhost,127.0.0.1` is required: the TUI talks to a local HTTP server and "you must bypass the proxy for this connection to prevent routing loops" ([docs](https://opencode.ai/docs/network/)). goalcup passes it.
- Where the model host is named: 1.x `provider.*.options.baseURL`, 2.x `providers.*.settings.baseURL` ([docs](https://opencode.ai/v2/docs/providers)), in the global config, the workspace `opencode.json` or `.opencode/`. The connectors read those keys as host hints. With none set, a provider takes its URL from the catalog and its host must be on the shared list.
- DeepSeek is a native provider and posts to `api.deepseek.com/chat/completions` (measured). `opencode.ai` is listed as a gateway host and also serves the console page where a key is copied. `models.opencode.ai` and `registry.npmjs.org` are tunnels.
- An unknown model id silently fell back to a paid gateway model, and the default small model is a paid gateway model too (measured). Set a valid `model` and `small_model`.

## 7. Process model

- 1.x: one Bun process, tools as child shells, Ctrl-C twice ends the TUI (measured). Exit codes and SIGTERM: unmeasured.
- 2.x: by default the CLI finds or starts one shared background server per user, which "handles sessions, configuration, permissions, and tool execution" and keeps its own environment ([docs](https://opencode.ai/v2/docs/cli)). A service started earlier would call models outside the run's proxy; `--standalone` avoids it. Unverified on a real run.

## 8. Things Goal Cup cannot isolate

- Managed config in `/etc/opencode`, `/Library/Application Support/opencode` and `%ProgramData%\opencode`, and a remote `.well-known/opencode` config, merged above the user's.
- `/tmp/opencode`, shared by every home.
- A shared 2.x service already running for the participant's account, if `--standalone` did not apply.

## 9. Quirks and open questions

- Auto-update: 1.x never installs by itself since 1.19.0 and only notifies; `OPENCODE_DISABLE_AUTOUPDATE=1` ([docs](https://opencode.ai/docs/cli/)), which both connectors set. 2.x has an `update` setting, `"disable"`, `"notify"` (default) or `"auto"`, read from the global config only ([docs](https://opencode.ai/v2/docs/config)). Whether 2.x also reads the environment switch: source only.
- Unknown: whether `--continue` picks the right session after a home switch on 2.x.

## Run without permission prompts

1.x: `"permission": "allow"` in `opencode.json`; `external_directory`, `doom_loop` and `.env` reads default to `ask` otherwise ([docs](https://opencode.ai/docs/permissions/)). The `opencode-cli` connector's `no_prompts` row writes it. 2.x names the key `permissions`, an ordered rule list ([docs](https://opencode.ai/v2/docs/config)), so `opencode2-cli` has no row and the participant sets it.

## Pad result

Not yet run on the pad. Expected: 1.x likely verified, 2.x not verified until `--standalone` and the version rule are seen on a real binary. [`connectors/tiers.toml`](../../connectors/tiers.toml) is the record.
