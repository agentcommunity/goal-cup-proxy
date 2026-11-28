# OpenClaw harness facts

Connector id: `openclaw-cli` · Facts for version: `2026.10.9` · Written by: Goal Cup team · Last checked: 2026-11-01
Public disclosure: yes. Facts come from OpenClaw's public docs and source and a run on 2026.10.8 in a Linux container on 28 Oct 2026. No keys, tokens or personal paths are included.
Status: shipped connector ([`connectors/openclaw-cli.toml`](../../connectors/openclaw-cli.toml))

Source words: "docs" is [docs.openclaw.ai](https://docs.openclaw.ai/help/environment) as read on 2026-10-31; "measured" is the 28 Oct run on 2026.10.8 with a private home and a local test proxy; "source" is the public repository one commit set ahead of `v2026.10.9`; "unknown" is not measured or read. Nothing was run on 2026.10.9.

## 1. Run one prompt, in chat mode first

- `openclaw chat` is `openclaw tui --local`: the embedded runtime, no Gateway ([docs](https://docs.openclaw.ai/cli/tui)). Plain `openclaw tui` connects to a Gateway instead and is not used. Exit: Escape, then `/exit` (measured). While a turn waits on retries, Ctrl-C and `/exit` are refused.
- The agent works in the project folder: the connector sets `OPENCLAW_WORKSPACE_DIR` ("Override the default agent workspace", [environment](https://docs.openclaw.ai/help/environment)) to `{workspace}`. A new workspace gets OpenClaw's bootstrap files (`AGENTS.md`, `SOUL.md`, `BOOTSTRAP.md` and others) and, with git installed, a `git init` ([agent workspace](https://docs.openclaw.ai/concepts/agent-workspace), read 2026-11-26), so they can reach the judged repository. An explicit `agents.defaults.workspace` in `openclaw.json` wins over `OPENCLAW_WORKSPACE_DIR` ([workspace and bootstrap](https://docs.openclaw.ai/gateway/config-agents/workspace-and-bootstrap)), and OpenClaw's onboarding writes one ([wizard reference](https://docs.openclaw.ai/reference/wizard), read 2026-11-26). If you ran `openclaw onboard` or `openclaw setup` in the isolated home, delete that key from `{home}/.openclaw/openclaw.json`, or the agent works in the folder it names. During `goalcup setup` the workspace is the isolated home. Not yet run on the pad.
- The connector has no `resume` form. Whether `chat` has a continue flag: unknown.
- [RULES](../RULES.md) lists OpenClaw incognito threads, `agent exec`, `--profile`, `--dev` and `proxy` as modes that switch capture off. The connector runs `chat` and none of those.

## 2. Install and record a version

- `curl -fsSL https://openclaw.ai/install.sh | bash -s -- --no-onboard --no-prompt`, which installs the npm package. It needs Node 24.22.1 or later (or 26.12.1 or later) (measured).
- The executable is a `#!/usr/bin/env node` script. `openclaw --version` prints `OpenClaw 2026.10.9 (<commit>)`. About two to three releases a week (npm, 29 Oct).

## 3. Where it keeps things, per OS

| | macOS | Linux | Windows |
|---|---|---|---|
| Config dir | `~/.openclaw/openclaw.json` (docs) | `$HOME/.openclaw/openclaw.json`, JSON5 (measured) | unknown |
| Session store | unknown | `agents/<agent>/agent/openclaw-agent.sqlite` and `state/openclaw.sqlite` under `.openclaw`; no JSONL (measured); neither is copied (section 4) | unknown |
| Logs / cache | unknown | `/tmp/openclaw/openclaw-YYYY-MM-DD.log`, outside the home (measured) | unknown |
| Env vars that move these | | `OPENCLAW_HOME`, `OPENCLAW_STATE_DIR`, `OPENCLAW_CONFIG_PATH`, `OPENCLAW_PROFILE` | |

`OPENCLAW_HOME` "replaces the system home directory" for OpenClaw's own path defaults, so state lands in `.openclaw/` under it ([docs](https://docs.openclaw.ai/help/environment)). The connector sets it and unsets the other three, which would move state out of the isolated home.

## 4. Session format and identity

- One SQLite file per agent plus one shared state file, each with `-wal` and `-shm`; The connector copies neither. Both hold OpenClaw's auth profiles as plain JSON (`auth_profile_store.store_json` in the agent database, `authProfiles.store` in the state database; 2026.10.9 package, read 26 Nov 2026), and no mask rule names their `key`, `access` and `refresh` fields. Model calls are recorded on the wire. Transcript rows are in the `transcript_events` table (measured). Session ids are UUIDs, in log lines as `sessionId=<uuid>`.
- Subagents are background runs in their own sessions, keyed `agent:<agentId>:subagent:<uuid>` ([docs](https://docs.openclaw.ai/tools/subagents)). Defaults: 8 concurrent runs and 5 children per session ([docs](https://docs.openclaw.ai/tools/subagents/operations)); these are OpenClaw's caps, not ours.

## 5. Auth

- Provider keys sit in `$HOME/.openclaw/.env` (for example `DEEPSEEK_API_KEY=`); the file works with no environment variable (measured). Auth profiles, API keys and OAuth tokens included, are in both databases ([docs](https://docs.openclaw.ai/concepts/oauth), read 26 Nov 2026), so the connector lists them as secret files and copies neither.
- A variable that moves config and login together: yes, `OPENCLAW_HOME`. The connector sets it, so sign-in and keys stay in the isolated home.

## 6. Network

- CA: `NODE_EXTRA_CA_CERTS` adds to the roots; `SSL_CERT_FILE` alone was ignored (measured). Node: yes, so the launch sets `NODE_USE_ENV_PROXY=1`. `HTTPS_PROXY` is honoured, and `NO_PROXY` too ([docs](https://docs.openclaw.ai/security/network-proxy)).
- Its own proxy wins: `proxy.proxyUrl` beats `OPENCLAW_PROXY_URL`, and in a run both beat `HTTPS_PROXY` (measured). goalcup unsets `OPENCLAW_PROXY_URL` and refuses the start if `proxy.proxyUrl`, `env.OPENCLAW_PROXY_URL` or `env.vars.OPENCLAW_PROXY_URL` is set in `openclaw.json`, or any of `OPENCLAW_PROXY_URL`, `OPENCLAW_STATE_DIR`, `OPENCLAW_CONFIG_PATH`, `OPENCLAW_PROFILE` in `.env` or `~/.config/openclaw/gateway.env`. The refusal names file and key, never the value.
- A hand-written `openclaw.json` is JSON5. goalcup reads it as JSON with comments and trailing commas; one with unquoted keys or single quotes does not parse and is skipped, so its proxy keys are not checked and no start line says so.
- DeepSeek posts to `api.deepseek.com/chat/completions`; a custom provider names its host at `models.providers.<id>.baseUrl` in `openclaw.json`, which the connector hints when the file parses as JSON with comments. A host in a file with unquoted keys or single quotes is not hinted: add it with `goalcup setup --record-host`. An unknown model id was refused and the first session went to the gateway default (measured).
- Raw `net`, `tls` and `http2` sockets, native addons and non-OpenClaw child processes "may bypass" Node-level routing, and loopback goes direct ([docs](https://docs.openclaw.ai/security/network-proxy)).

## 7. Process model

- `chat` runs the agent inside the CLI process, with no Gateway. Local mode "refuses to start while a Gateway or another embedded writer owns that state" ([docs](https://docs.openclaw.ai/cli/tui)).
- The Gateway daemon is not started or stopped by goalcup. `openclaw gateway stop` acts on the account's native service (a systemd unit, a launchd agent), which `OPENCLAW_HOME` does not scope ([docs](https://docs.openclaw.ai/cli/gateway/service), [environment](https://docs.openclaw.ai/help/environment)). The connector has no `before` or `after`. The connector's `first_screen` line asks the participant to stop a running Gateway, and to type `/new` at the start of a new run, because `chat` reattaches the rolling `main` session ([docs](https://docs.openclaw.ai/concepts/session), read 26 Nov 2026).

## 8. Things Goal Cup cannot isolate

- A value the model restates can stay unmasked (stated loss for 1.0): OpenClaw's model restated a git-remote password in its own reasoning content and then resent that in every later request; the masker's `url_password` shape only matches inside `user:pw@`, and the bare copy is not an exact-value source, so it is not caught (MAC-LEARNINGS.md row 7a on the `archive` branch).
- Subagents not recorded on the wire when they run in a Gateway started outside goalcup: that daemon keeps its own environment, so its model calls skip the run's proxy. In `chat` a spawn likely fails or reaches such a Gateway (source, unverified by run). The `first_screen` line says so.
- Logs under `/tmp/openclaw`, shared by every home.
- The managed browser and raw sockets (section 6).

## 9. Quirks and open questions

- `OPENCLAW_NO_AUTO_UPDATE=1` turns off update applies ([docs](https://docs.openclaw.ai/help/environment)); the connector sets it.
- Unknown: the session after a home switch, whether a Gateway can be run for one home only, SIGTERM and exit codes.

## Run without permission prompts

No-approval host exec is the default (`tools.exec.mode` unset; docs say `full`). Approvals live in the state database, not a file a `no_prompts` row can write, so the connector has none. Whether chat then asks nothing is unmeasured.

## Pad result

The tier, the tested version and the runs are in [`connectors/tiers.toml`](../../connectors/tiers.toml) and `scripts/pad/rows/PAD-RESULTS.md`; this sheet does not copy them.
