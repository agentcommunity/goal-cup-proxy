# OpenClaw harness facts

Connector id: `openclaw-cli` · Facts for version: `2026.10.9` · Written by: Goal Cup team · Last checked: 2026-10-31
Public disclosure: yes. Facts come from OpenClaw's public docs and source and a run on 2026.10.8 in a Linux container on 28 Oct 2026. No keys, tokens or personal paths are included.
Status: shipped connector ([`connectors/openclaw-cli.toml`](../../connectors/openclaw-cli.toml))

Source words: "docs" is [docs.openclaw.ai](https://docs.openclaw.ai/help/environment) as read on 2026-10-31; "measured" is the 28 Oct run on 2026.10.8 with a private home and a local test proxy; "source" is the public repository one commit set ahead of `v2026.10.9`; "unknown" is not measured or read. Nothing was run on 2026.10.9.

## 1. Run one prompt, in chat mode first

- `openclaw chat` is `openclaw tui --local`: the embedded runtime, no Gateway ([docs](https://docs.openclaw.ai/cli/tui)). Plain `openclaw tui` connects to a Gateway instead and is not used. Exit: Escape, then `/exit` (measured). While a turn waits on retries, Ctrl-C and `/exit` are refused.
- The connector has no `resume` form. Whether `chat` has a continue flag: unknown.
- [RULES](../RULES.md) names `--local`, `agent exec`, `--profile`, `--dev` and `proxy` modes as ones that switch recording off; `chat` implies `--local`, and the rules question is open.

## 2. Install and record a version

- `curl -fsSL https://openclaw.ai/install.sh | bash -s -- --no-onboard --no-prompt`, which installs the npm package. It needs Node 24.22.1 or later (or 26.12.1 or later) (measured).
- The executable is a `#!/usr/bin/env node` script. `openclaw --version` prints `OpenClaw 2026.10.9 (<commit>)`. About two to three releases a week (npm, 29 Oct).

## 3. Where it keeps things, per OS

| | macOS | Linux | Windows |
|---|---|---|---|
| Config dir | `~/.openclaw/openclaw.json` (docs) | `$HOME/.openclaw/openclaw.json`, JSON5 (measured) | unknown |
| Session store | unknown | `agents/<agent>/agent/openclaw-agent.sqlite` and `state/openclaw.sqlite` under `.openclaw`; no JSONL (measured) | unknown |
| Logs / cache | unknown | `/tmp/openclaw/openclaw-YYYY-MM-DD.log`, outside the home (measured) | unknown |
| Env vars that move these | | `OPENCLAW_HOME`, `OPENCLAW_STATE_DIR`, `OPENCLAW_CONFIG_PATH`, `OPENCLAW_PROFILE` | |

`OPENCLAW_HOME` "replaces the system home directory" for OpenClaw's own path defaults, so state lands in `.openclaw/` under it ([docs](https://docs.openclaw.ai/help/environment)). The connector sets it and unsets the other three, which would move state out of the isolated home.

## 4. Session format and identity

- One SQLite file per agent plus one shared state file, each with `-wal` and `-shm`; the connector copies both rows at the end as a final set. Transcript rows are in the `transcript_events` table (measured). Session ids are UUIDs, in log lines as `sessionId=<uuid>`.
- Subagents are background runs in their own sessions, keyed `agent:<agentId>:subagent:<uuid>` ([docs](https://docs.openclaw.ai/tools/subagents)). Defaults: 8 concurrent runs and 5 children per session ([docs](https://docs.openclaw.ai/tools/subagents/operations)); these are OpenClaw's caps, not ours.

## 5. Auth

- Provider keys sit in `$HOME/.openclaw/.env` (for example `DEEPSEEK_API_KEY=`); the file works with no environment variable (measured). Auth profiles are in the agent database.
- A variable that moves config and login together: yes, `OPENCLAW_HOME`. The connector sets it, so sign-in and keys stay in the isolated home.

## 6. Network

- CA: `NODE_EXTRA_CA_CERTS` adds to the roots; `SSL_CERT_FILE` alone was ignored (measured). Node: yes, so the launch sets `NODE_USE_ENV_PROXY=1`. `HTTPS_PROXY` is honoured, and `NO_PROXY` too ([docs](https://docs.openclaw.ai/security/network-proxy)).
- Its own proxy wins: `proxy.proxyUrl` beats `OPENCLAW_PROXY_URL`, and in a run both beat `HTTPS_PROXY` (measured). goalcup unsets `OPENCLAW_PROXY_URL` and refuses the start if `proxy.proxyUrl`, `env.OPENCLAW_PROXY_URL` or `env.vars.OPENCLAW_PROXY_URL` is set in `openclaw.json`, or any of `OPENCLAW_PROXY_URL`, `OPENCLAW_STATE_DIR`, `OPENCLAW_CONFIG_PATH`, `OPENCLAW_PROFILE` in `.env` or `~/.config/openclaw/gateway.env`. The refusal names file and key, never the value.
- A hand-written `openclaw.json` is JSON5. One that does not parse as JSON is named in `unread`, so its proxy keys are not checked and the participant is told.
- DeepSeek posts to `api.deepseek.com/chat/completions`; no model host is named in a file by default, so there are no host hints. An unknown model id was refused and the first session went to the gateway default (measured).
- Raw `net`, `tls` and `http2` sockets, native addons and non-OpenClaw child processes "may bypass" Node-level routing, and loopback goes direct ([docs](https://docs.openclaw.ai/security/network-proxy)).

## 7. Process model

- `chat` runs the agent inside the CLI process, with no Gateway. Local mode "refuses to start while a Gateway or another embedded writer owns that state" ([docs](https://docs.openclaw.ai/cli/tui)).
- The Gateway daemon is not started or stopped by goalcup. `openclaw gateway stop` acts on the account's native service (a systemd unit, a launchd agent), which `OPENCLAW_HOME` does not scope ([docs](https://docs.openclaw.ai/cli/gateway/service), [environment](https://docs.openclaw.ai/help/environment)). The connector has no `before` or `after`.

## 8. Things Goal Cup cannot isolate

- Subagents not recorded on the wire when they run in a Gateway started outside goalcup: that daemon keeps its own environment, so its model calls skip the run's proxy. In `chat` a spawn likely fails or reaches such a Gateway (source, unverified by run).
- Logs under `/tmp/openclaw`, shared by every home.
- The managed browser and raw sockets (section 6).

## 9. Quirks and open questions

- `OPENCLAW_NO_AUTO_UPDATE=1` turns off update applies ([docs](https://docs.openclaw.ai/help/environment)); the connector sets it.
- Unknown: the session after a home switch, whether a Gateway can be run for one home only, SIGTERM and exit codes.

## Run without permission prompts

No-approval host exec is the default (`tools.exec.mode` unset; docs say `full`). Approvals live in the state database, not a file a `no_prompts` row can write, so the connector has none. Whether chat then asks nothing is unmeasured.

## Pad result

Not yet run on the pad. Expected tier: not verified: subagents need the Gateway, which goalcup does not start. [`connectors/tiers.toml`](../../connectors/tiers.toml) is the record.
