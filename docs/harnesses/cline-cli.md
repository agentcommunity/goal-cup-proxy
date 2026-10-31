# Cline CLI harness facts

Connector id: `cline-cli` · Facts for version: `3.0.74` · Written by: Goal Cup team · Last checked: 2026-10-31
Public disclosure: yes. Facts come from Cline's public docs, the `3.0.74` npm package and one build-box test of the hub command. No keys, tokens, account names or personal paths are included.
Status: shipped connector ([`connectors/cline-cli.toml`](../../connectors/cline-cli.toml))

Source words: "docs" is [docs.cline.bot](https://docs.cline.bot/cli/cli-reference) as read on 2026-10-31; "package" is the `3.0.74` Bun binary and its source strings; "box test" is the 30 Oct run that started a hub in one data folder and tried to stop it from another; "unknown" is not read or measured. No model call has been recorded through the proxy.

## 1. Run one prompt, in chat mode first

- `cline` with no argument starts a task or the interactive mode; `-i` (`--tui`) opens the terminal UI ([docs](https://docs.cline.bot/cli/cli-reference)).
- There is no continue form. `--id <session-id>` resumes a named session and `cline history` opens a picker (docs), but nothing reopens "the last one". The connector's `resume` is empty, so `goalcup continue` starts a new Cline session in the same home and says so. The earlier session's files stay in the home and are copied at the end.
- `cline --json "<prompt>"` runs once; auto-approve defaults to true (docs). It is not chat mode and is not used.

## 2. Install and record a version

- `npm install -g cline@<version>` pulls the platform package (`@cline/cli-linux-x64` and others), a Bun-compiled binary (package). `cline --version` prints the version; unmeasured form.
- `CLINE_NO_AUTO_UPDATE=1` is read by the binary though the docs do not list it; the binary sets it for its own children (package). The connector sets it. Release notes say updates install on exit (cli-v3.0.55), so a run may change the binary between `start` and the next `continue`; the `start` event records the version each time.

## 3. Where it keeps things, per OS

| | macOS | Linux | Windows |
|---|---|---|---|
| Config dir | `$HOME/.cline/` (`CLINE_DIR`) ([docs](https://docs.cline.bot/cli/cli-reference)) | same | `%USERPROFILE%\.cline\` |
| Session store | `data/sessions/<id>/` per session; SQLite index `data/db/sessions.db` | same | same |
| Logs / cache | `data/logs/cline.log`, `data/logs/hub-daemon.log` ([docs](https://docs.cline.bot/troubleshooting/networking-and-proxies)) | same | same |
| Env vars that move these | `CLINE_DIR`, `CLINE_DATA_DIR` (default `$CLINE_DIR/data`), `CLINE_LOG_PATH` | same | same |

The connector sets `CLINE_DIR` to the isolated home and `CLINE_DATA_DIR` to `data` under it. It also removes `CLINE_HUB_DISCOVERY_PATH`, `CLINE_SESSION_DATA_DIR`, `CLINE_DB_DATA_DIR` and `CLINE_PROVIDER_SETTINGS_PATH` from the participant's environment: each would move the hub record, sessions, index or key file out of the home (package).

## 4. Session format and identity

- One folder of message files per session, plus a SQLite index (`sessions.db`, with `-wal` and `-shm`) (docs). The connector reads the whole `data/sessions` tree as row `sessions` (form `append`, read as it grows) and copies the index as row `index` (form `final`, copied when the run ends). The file format inside a session folder: unknown, and goalcup does not read it.
- Subagents are started by the `spawn_agent` tool, on unless settings turn `enableSpawn` off (package). Whether each gets its own folder: unknown.
- The session id is the folder name (docs).

## 5. Auth

- Provider keys through `cline auth`, interactively or with `-p <provider> -k <key> -m <model>`; they are stored in `data/settings/providers.json` (docs; package). The connector lists that file as a secret file. A Cline account sign-in is also possible and uses `api.workos.com` and `app.cline.bot`.
- DeepSeek is a native provider id `deepseek` (package).
- A variable that moves config and login together: yes, `CLINE_DIR` with `CLINE_DATA_DIR`.

## 6. Network

- Proxy: `https_proxy`, `http_proxy`, `no_proxy`. The CLI "does not support SOCKS proxies, proxy autoconfiguration (PAC) scripts" ([docs](https://docs.cline.bot/troubleshooting/networking-and-proxies)). goalcup sets both `HTTPS_PROXY` and `https_proxy`.
- CA: `NODE_EXTRA_CA_CERTS`, a PEM file (docs). The connector has no `ca_env`: goalcup sets `NODE_EXTRA_CA_CERTS` to the run CA for every harness.
- Where the model host is named: `providers.*.settings.baseUrl` in `data/settings/providers.json`. A provider without it uses its built-in host, which must be on the shared list. `api.cline.bot` is listed as a gateway host; `api.workos.com` is on the never-decrypt list and passes as a tunnel.
- PostHog telemetry strings are in the binary (package); they are not model calls and are tunnels.

## 7. Process model

- Sessions can run in a local hub daemon (default port 25463, `CLINE_HUB_PORT`) that starts "when needed" and keeps the environment of whoever started it (docs). With `CLINE_SESSION_BACKEND_MODE=local`, which the connector sets, the session runs in process and starts no hub (package; box test).
- `cline hub stop` reads the hub record at `CLINE_DATA_DIR/locks/hub/production.json`, so it stops only the hub of that data folder. In the box test, a stop from another data folder left the hub running. The connector runs `hub stop` before and after the run.
- Exit codes: not documented.

## 8. Things Goal Cup cannot isolate

- A hub started outside goalcup for the participant's real `~/.cline`: it keeps its own environment, and goalcup's stop cannot reach it from the isolated home.
- Cline's remote and ACP session modes if the participant selects them.

## 9. Quirks and open questions

- Unknown: the file format of a session folder, whether subagents write their own folder, whether the Bun binary trusts the run CA through `NODE_EXTRA_CA_CERTS` on a real run, Windows paths.
- No `no_prompts` row: the docs name no settings key for approval and `--auto-approve` defaults to true.
- Unknown: whether a run shows `HTTPS_PROXY` and the run CA honoured and no hub started.

## Run without permission prompts

None needed: auto-approve defaults to `true` for the CLI (`--auto-approve`, [docs](https://docs.cline.bot/cli/cli-reference)); it is `false` only in ACP mode. The connector has no `no_prompts` row.

## Pad result

The tier, the tested version and the runs are in [`connectors/tiers.toml`](../../connectors/tiers.toml) and `PAD-RESULTS.md`; this sheet does not copy them.
