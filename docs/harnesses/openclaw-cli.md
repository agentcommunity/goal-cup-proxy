# OpenClaw harness facts

Connector id: `openclaw-cli` · Facts for version: `2026.10.8` · Written by: Goal Cup team (agent-written, measured in a Linux cloud container) · Last checked: 2026-10-28
Public disclosure: yes. No keys, tokens or personal paths. Every probe used a private `HOME` and an invented key.
Status: facts only. OpenClaw keeps its transcripts only in SQLite, which schema 1 cannot capture
([Cannot be captured](../HARNESS-ONBOARDING.md#row-0), "A database session store"). Its
executable is a Node script (row 10), and its own proxy settings override `HTTPS_PROXY` (section 8).

Source words: "measured" means run on Linux x86_64, OpenClaw 2026.10.8 (dc2523e) on Node 26.12.1,
2026-10-28, with a private `HOME`, through a local test proxy; "vendor docs" means the OpenClaw
docs source (`github.com/openclaw/openclaw`, `docs/`, 2026-10-27; docs.openclaw.ai was not
reachable from the container); "unknown" means not measured or read. "Measured in a run" means a
local Goal Cup run (goalcup 0.7.0, `--dev`, the unpinned proof connector) in a Vercel Sandbox,
Linux x86_64, OpenClaw 2026.10.8 on Node 24.22.1, 2026-10-28, with real Gateway and DeepSeek
replies (see the [results file](../research/results/2026-10-opencode-openclaw.md)).

## 1. Run one prompt, in chat mode first

- Chat mode: `openclaw chat`, an alias for `openclaw tui --local` (help text). Plain
  `openclaw tui` connects to a Gateway daemon (`ws://127.0.0.1:18789`) instead. Driven under
  `goalcup start` in tmux: it records (measured in a run).
- One-shot mode (used only for the probes below): `openclaw agent --local --agent main --message "<prompt>"`.
- [RULES](../RULES.md) lists `--local`, `agent exec`, `--profile`, `--dev`, `proxy` and incognito
  threads as modes that switch capture off. That was measured on 2026.8.2 against the Gateway
  plugin design; whether `chat` (`tui --local`) is acceptable under chat mode (D57) is open.

## 2. Install and record a version

- Official installer: `curl -fsSL https://openclaw.ai/install.sh | bash -s -- --no-onboard --no-prompt`.
  It installs the npm package globally. It needs Node 24.22.1+ or 26.12.1+; with Node 22 and nvm
  present it stops and asks for `nvm install 26` (measured).
- Binary: `openclaw` on the Node bin path, a `#!/usr/bin/env node` script (measured), so a pinned
  connector is refused as an interpreter (row 10); `--dev` accepts it.
- Version: `openclaw --version` prints `OpenClaw 2026.10.8 (dc2523e)` (measured).
- Providers are plugins installed from npm: `openclaw plugins install @openclaw/vercel-ai-gateway-provider`
  and `@openclaw/deepseek-provider` (vendor docs; measured, into `$HOME/.openclaw/npm/`).

## 3. Where it keeps things, per OS

| | macOS | Linux | Windows |
|---|---|---|---|
| Config dir | unknown | `$HOME/.openclaw/openclaw.json` (JSON5; measured) | unknown |
| Session / transcript store | unknown | `$HOME/.openclaw/agents/<agent>/agent/openclaw-agent.sqlite` (+ `-wal`, `-shm`, lock files), table `transcript_events` (measured) | unknown |
| Logs / cache | unknown | `/tmp/openclaw/openclaw-YYYY-MM-DD.log` by default, outside `HOME` (measured); `logging.file` moves it (measured); `$HOME/.openclaw/state/openclaw.sqlite`; `$HOME/.cache/openclaw` | unknown |
| Env vars that move these | | `OPENCLAW_HOME` (replaces `HOME` for all OpenClaw paths), `OPENCLAW_STATE_DIR`, `OPENCLAW_CONFIG_PATH`, `OPENCLAW_PROFILE` (vendor docs) | |

## 4. Session format and identity

- Shared SQLite per agent: 69 tables; `transcript_events` held 54 rows after the probes (measured).
  `agents/<id>/sessions/` holds only archives and the legacy `sessions.json` (vendor docs); none
  was written here (measured). No JSONL transcript file exists any more (vendor docs, measured).
- Session ids are UUIDs, in log lines `sessionId=<uuid>` (measured).
- This is the stop rule, as for OpenCode.

## 5. Auth

- A key in `$HOME/.openclaw/.env` (`AI_GATEWAY_API_KEY=`, `DEEPSEEK_API_KEY=`) works with no
  environment variable: the invented key was sent as `x-api-key` to the Gateway (measured). Vendor
  docs recommend that file for provider keys.
- Model auth profiles live in the agent SQLite file (`auth_profile_store`); `auth-profiles.json`
  is legacy (vendor docs). `openclaw models auth paste-token --provider <id>` and
  `openclaw onboard --non-interactive --accept-risk ...` also store keys (vendor docs; the onboard
  form takes the key in argv, so it was not used).
- A variable that moves config and login: `OPENCLAW_HOME` matches the allowed form
  `^[A-Z][A-Z0-9]*_(HOME|CONFIG_DIR)$` (row 5). Not measured as a connector home.

## 6. Network

- CA: `NODE_EXTRA_CA_CERTS` adds to the roots (mode `extend`). Set, TLS to the test proxy
  succeeded; unset, `UNABLE_TO_VERIFY_LEAF_SIGNATURE`; `SSL_CERT_FILE` alone was ignored (all
  measured with a private test CA).
- `HTTPS_PROXY`: honoured on its own (measured). Node: yes; a connector keeps
  `NODE_USE_ENV_PROXY = "1"` (D59).
- Its own proxy: `OPENCLAW_PROXY_URL` and `proxy.proxyUrl` in `openclaw.json` both work, and both
  win over `HTTPS_PROXY` (measured: with `HTTPS_PROXY` set to one proxy, every request went to the
  other). The key is checked before the env (vendor docs). See section 8.
- Model address: the Vercel and DeepSeek plugins carry their URL; `openclaw.json` names only the
  model (`agents.defaults.model.primary = "vercel-ai-gateway/poolside/laguna-s-2.1-free"`,
  measured). A URL is in the file only with `models.providers.<id>.baseUrl` (vendor docs). So
  `model_host_from` has nothing to read by default.
- Wire, measured with an invented key: Gateway `POST https://ai-gateway.vercel.sh/v1/messages`
  (Anthropic Messages). It retries 8 times with 30 s waits on a reset. Same with a real key
  (measured in a run); on a 429 it retries after 27 s, then 60 s (up to 9 times, its log).
  DeepSeek `deepseek/deepseek-flash`: `POST https://api.deepseek.com/chat/completions` (measured
  in a run). `config set ... deepseek/deepseek-v4-flash` is refused as "Unknown model" (measured;
  that likely explains the first session's Gateway call). After
  `openclaw models list --refresh --provider deepseek` the catalog lists `deepseek/deepseek-flash`
  and `deepseek/deepseek-v4-pro`.
- Requests carry `HTTP-Referer: https://openclaw.ai` and `X-Title: OpenClaw` (vendor docs).
- Other hosts: `telemetry.openclaw.ai` daily update check, off with `OPENCLAW_NO_AUTO_UPDATE=1`
  or `update.checkOnStart: false`; npm and ClawHub for plugins (vendor docs).

## 7. Process model

Embedded agent in the CLI process for `--local`/`chat`; a separate Gateway daemon otherwise (vendor
docs). The daemon keeps its own environment, like the Hermes gateway (D72): unmeasured. Exit
(measured in a run): Escape, then `/exit`. While a turn waits on retries ("finishing context"),
Ctrl-C and `/exit` are refused; `goalcup seal --yes` stopped it, with termination logged as
`Unresolved` (`ProcessAuthorityUnavailable`) and no process left. SIGTERM, subagents: unmeasured.

## 8. Things Goal Cup cannot isolate

- **Proxy override.** `proxy.proxyUrl` in `openclaw.json` or `OPENCLAW_PROXY_URL` in
  `$HOME/.openclaw/.env` sends model traffic to another proxy and past the recorder (measured).
  Goal Cup does not read either today.
- Logs default to `/tmp/openclaw/`, shared by every `HOME` (measured).
- The managed browser and raw sockets bypass the proxy (vendor docs).

## 9. Quirks and open questions

- Is `openclaw chat` (`tui --local`) a chat mode Goal Cup accepts, given the RULES list? It
  records (measured in a run); the rules question is still open.
- With `logging.file` set into the home, the log is written during a run and the native row finds
  it (measured in a run).

## Run without permission prompts

No-approval host exec is the default (`tools.exec.mode` unset; vendor docs say the default is
`full`). Explicit: `openclaw config set tools.exec.mode full`, or `openclaw exec-policy preset yolo`
(vendor docs). Approvals live in `$HOME/.openclaw/state/openclaw.sqlite` (vendor docs), not in a
file a connector's `no_prompts` row could read. Whether chat mode then asks nothing: unmeasured.
