# Appendix O: harness access under "HTTPS_PROXY plus run CA"

Date: 29 Oct 2026 (checks ran from about 19:00 UTC to the late evening). Repo: `/home/user/goal-cup-proxy`.
Review only. No harness was run, nothing was built, no test was run, no key or secret was read.
This appendix merges six research parts (O1 to O6) and refreshes appendix D
(`docs/plans/2026-10-30-audit/D-harnesses.md`) and the DeepSeek part of appendix E
(`docs/plans/2026-10-30-audit/E-providers.md`) for the new design.

The design under test: goalcup starts the harness with `HTTPS_PROXY` pointing at a local recording proxy
and hands it a run CA. The harness must send its model calls through the proxy and trust the CA. goalcup
never sets a model base URL on any harness (`AGENTS.md:81`): it never changes where a harness sends
calls. For tests, the pad configures each harness the way a participant would (for example, pointing it
at DeepSeek). Test keys: Claude Code uses a Claude API key, Codex uses an OpenAI API key, and every
other harness that accepts its own key uses DeepSeek. Terminal capture is out of scope.

## Summary

Of the 15 harnesses, 13 can send their model calls through goalcup's proxy without goalcup touching a
base URL. Two cannot: Cursor CLI and Amp both run inference on their own servers, so the proxy sees only
their own protocol, never the provider request. Cursor also speaks HTTP/2 that its server can force,
which our HTTP/1.1-only proxy refuses. Of the 13, three are measured through a proxy already (Claude
Code on 2.1.312, OpenCode 1 and the OpenClaw main agent on 28 Oct) and a fourth has its proxy path
measured (Hermes; its CA path changes now that goalcup stops writing a `recorder` provider). Everything
else comes from source and docs and needs one pad run each. DeepSeek works natively or through a
documented route in 12 of the 15 (all but Cursor, Gemini CLI and Amp, where it can run only on Amp's
servers); Claude Code and Codex use their own vendors' keys in the test anyway. Four harnesses need their own account or key on top: Amp (Amp account),
Cursor (Cursor plan), Gemini CLI (paid Google key) and Factory Droid (free Factory account). The main
traps are not the proxy: they are CA variables that replace the system roots (`SSL_CERT_FILE` in Python,
Go and some Rust stacks, so goalcup must pass a full bundle), background daemons or gateways that keep
their own environment (Hermes, OpenCode 2, OpenClaw, Codex, Droid, Claude Code agent view), harness
settings that beat `HTTPS_PROXY` (OpenClaw, Qwen Code, Codex `.env`), and very fast release pace
(Amp about 7 builds a day, Droid and Copilot about one a day). DeepSeek is a good test provider:
HTTP/1.1 accepted, plain SSE, no pinning, cheap. The owner's figure of 0.12 in and 0.48 out per million
tokens is out of date: the page says 0.15 and 0.60 off-peak, double at weekday peak hours. A 10-minute
test is still about 0.10 dollars. If a harness fails, the best swap-ins are Kimi Code, Mistral Vibe
and Cline CLI.

Words used below:
- "source" = read in the harness code or shipped package at the named tag, never run.
- "binary" = read from `strings` of the shipped executable, never executed.
- "docs" = vendor docs fetched 29 Oct 2026. Most pages carry no date, so the fetch date is the date.
- "measured" = measured earlier by the team and written in this repo.
- "probe" = one `POST {}` with no key to a DeepSeek path on 29 Oct 2026; only the status line and the
  `server`/`via` headers were read.
- "unverified" = nobody has checked it by running it.
- Model names and ids are left out on purpose. DeepSeek sells two models today: "the cheap model" and
  "the large model". Recipes write `<cheap model id>`; copy it from the pricing page
  (https://api-docs.deepseek.com/quick_start/pricing) on the day.

How it was checked (from the parts):
- Versions and dates: npm registry `time` fields (`registry.npmjs.org/<pkg>`), PyPI JSON, `git ls-remote`
  tags, all fetched 29 Oct 2026.
- Source read in shallow, read-only clones: `openclaw/openclaw` at `cfb5923c` (30 Oct 2026 UTC),
  `anomalyco/opencode` branch `dev` at `085d6f7b` (29 Oct 2026; OpenCode 1 files at tag `v1.19.5`),
  https://github.com/QwenLM/qwen-code at `v0.26.4` (commit `715a550c`), https://github.com/charmbracelet/crush
  at `v0.99.0` (commit `e38655aa`, 29 Oct 2026) plus Catwalk `v3.0.1` and the npm wrapper
  `@charmland/crush@0.99.0`, Goose `v1.57.0`, Aider `v0.86.2`, Kilo `v7.8.25`, Hermes `v0.22.5` and
  `origin/main` at `67d47601bc`, Codex `rust-v0.165.0`.
- Packages read without running: Gemini CLI `@google/gemini-cli@0.66.0` npm tarball (`bundle/*.js`);
  Copilot CLI `@github/copilot-linux-x64@1.0.105` (one ELF, read with `strings`); Cursor CLI
  `2026.10.22-27dcf5f` linux x64 package; Factory Droid `@factory/cli-linux-x64@0.251.0` (`strings`); and
  the backup harness packages listed in section O.6.
- Factory Droid docs at https://docs.factory.com (the old `docs.factory.ai` now answers 308 to it).
- The `gh` search API was blocked in the sessions, so tracker checks used web search. "No known issue"
  means "none found", not "none exists".
- Facts sheets for OpenCode and OpenClaw exist only on branch `origin/opencode-openclaw`
  (`docs/harnesses/opencode-cli.md`, `docs/harnesses/openclaw-cli.md`, both measured 28 Oct 2026).
- Prior research built on: appendix D (`docs/plans/2026-10-30-audit/D-harnesses.md`: D1 to D3, D4.3.7
  Goose, D4.3.8 Aider, D4.3.9 Kilo, D4.4 runtime table, and `:1232-1360` for Amp, OpenCode and OpenClaw),
  appendix E (`E-providers.md`), and the connectors in `connectors/`.
- Proxy facts that matter for every harness: the proxy today is HTTP/1.1 only. It offers only `http/1.1`
  in ALPN, so a client that insists on `h2` is refused (`crates/goalcup-cli/src/proxy/tls.rs:26-31`). It
  does relay WebSocket upgrades (`crates/goalcup-cli/src/proxy/websocket.rs`). It forwards raw bytes and
  parses only HTTP framing (`crates/goalcup-cli/src/proxy/http.rs:1-12`).
- The 2-hour run report (`docs/research/results/2026-10-30-two-hour-run.md`) does not exist on any branch
  yet. Nothing here relies on it.

## O.1 Master table

"Proxy-ready" means the harness sends its model calls through `HTTPS_PROXY` and can trust the run CA
without goalcup setting a base URL. "Full bundle" means a PEM file holding the system roots plus the run
CA, because that variable replaces the default roots. Section numbers point to the per-harness sections.

| # | Harness | Version (date) | Runtime | Proxy-ready | CA method goalcup passes | Test key | DeepSeek route | Subagents | Session files | Auto-update switch | Verdict |
|---|---|---|---|---|---|---|---|---|---|---|---|
| H1 | Claude Code | 2.1.312 (27 Oct); stable 2.1.308 (22 Oct) | Bun native binary | **Yes** (measured on 2.1.312 through a gateway; direct with an API key unverified) | `NODE_EXTRA_CA_CERTS` (extends) | Claude API key | Anthropic-format endpoint `https://api.deepseek.com/anthropic` (not used) | Yes, in-process Agent tool; workflow agents maybe separate processes (unverified) | `~/.claude/projects/<p>/<session>.jsonl` plus `<session>/subagents/agent-<id>.jsonl` | `DISABLE_AUTOUPDATER=1`, `DISABLE_UPDATES=1` | Test first. Ready. |
| H2 | Codex CLI | 0.165.0 (27 Oct) | Rust (npm package is a Node wrapper) | **Likely** (HTTPS and CA clear in source; WebSocket path unmeasured) | `CODEX_CA_CERTIFICATE` (else `SSL_CERT_FILE`; extends) | OpenAI API key | Responses API at `https://api.deepseek.com` (new, corrects part O1, see O.8.4; not used) | Yes, threads in the same process | `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-*.jsonl`, one per thread | No self-install; `check_for_update_on_startup = false` | Test. Record the WebSocket path. |
| H3 | Hermes Agent | v0.22.5 (22 Oct) | Python 3.14 (uv); a gateway process makes the calls | **Likely** (proxy measured; CA through `SSL_CERT_FILE` unverified) | `SSL_CERT_FILE` = full bundle (replaces), or the OS store | DeepSeek | Native provider `deepseek` | Yes, in the gateway process (unmeasured on 0.22.5) | `$HERMES_HOME/state.db` (SQLite) plus `cache/delegation/live/` logs | No self-install; `updates.check: false` | Test. Stop any old gateway first. |
| H4 | OpenCode | 1.19.5 (28 Oct); OpenCode 2 is 2.2.2 (29 Oct) | Bun binary (2: Node launcher plus Bun) | **Yes** for 1 (measured 28 Oct); 2 likely only with `--standalone` | `NODE_EXTRA_CA_CERTS` (`SSL_CERT_FILE` also worked) | DeepSeek | Native provider `deepseek` (measured) | Yes, in-process Task tool (unmeasured through goalcup) | `~/.local/share/opencode/opencode.db` (SQLite) | 1 only notifies; `OPENCODE_DISABLE_AUTOUPDATE=1` | Test 1 first; 2 once with `--standalone`. |
| H5 | OpenClaw | 2026.10.9 (29 Oct) | Node `>=24.22.1 <25` or `>=26.12.1` | **Likely** for the main agent (measured 28 Oct); subagents unknown | `NODE_EXTRA_CA_CERTS` (`SSL_CERT_FILE` ignored) | DeepSeek | Plugin `@openclaw/deepseek-provider` (measured) | Yes, but spawn needs a Gateway daemon (source) | `~/.openclaw/agents/<agent>/agent/openclaw-agent.sqlite` | `OPENCLAW_NO_AUTO_UPDATE=1` | Test main agent; owner decides on the Gateway path. |
| H6 | Amp | 0.0.1793275344-g17bf52 (29 Oct) | Bun binary | **Yes for Amp's WebSocket; no for provider calls** (run on Amp's servers) | `NODE_EXTRA_CA_CERTS` | Amp access token, plus DeepSeek stored in Amp or Amp credits | Only as a server-side Custom URL connection (unverified) | Yes (`Task`), on the same WebSocket | None local; metadata log `~/.cache/amp/logs/threads/T-<uuid>.log` | `AMP_SKIP_UPDATE_CHECK=1` | Dodger. Records Amp's protocol only. |
| H7 | Cursor CLI | 2026.10.22-27dcf5f (22 Oct) | Bash launcher plus bundled Node | **No** (HTTP/2 bidi the server can force; inference on Cursor's servers) | `NODE_EXTRA_CA_CERTS` plus system store | Cursor account key | None (hidden `--base-url` path rejected in the public build, unverified) | Yes (server-side inference) | `~/.cursor/chats/<md5>/<agent>/store.db` plus `agent-transcripts/<parent>/subagents/` | `--disable-auto-update` (hidden) or `channel: "static"` (unverified) | Out of Edition 01. |
| H8 | Gemini CLI | 0.66.0 (27 Oct) | Node 20+; relaunches a child with the same env | **Likely** | `NODE_EXTRA_CA_CERTS` (launch env only) | Paid Gemini API key | None (Gemini format only; a translating gateway is not how people run it) | Yes, in-process | `~/.gemini/tmp/<id>/chats/*.jsonl`, subagents in `chats/<parent>/` | `general.enableAutoUpdate: false` plus `enableAutoUpdateNotification: false` | Only if the owner funds a Google key. Low priority. |
| H9 | GitHub Copilot CLI | 1.0.105 (28 Oct) | Node single-executable (Node 24.22.1) plus Rust addon | **Likely** (env proxy wins over settings; HTTP/1.1 default; CA layer unverified) | `NODE_EXTRA_CA_CERTS` and `SSL_CERT_FILE` = full bundle | DeepSeek | BYOK `anthropic` type at `https://api.deepseek.com/anthropic` (not `openai`); no GitHub login | Yes (process model unverified) | `~/.copilot/session-state/<id>/events.jsonl` | `COPILOT_AUTO_UPDATE=false` or `--no-auto-update` | Test. Best of its lane. |
| H10 | Goose | v1.57.0 (27 Oct) | Rust (reqwest, rustls) | **Likely** | `GOOSE_CA_CERT_PATH` (extends) plus `SSL_CERT_FILE` = full bundle | DeepSeek | Custom provider `custom_deepseek` (OpenAI format) | Yes, in-process `delegate`; 5 at once, 5-minute timeout | `~/.local/share/goose/sessions/sessions.db` (SQLite) | CLI does not self-update | Test. |
| H11 | Aider | 0.86.2 (12 Feb 2026), stale | Python (LiteLLM, httpx, requests) | **Likely** | `SSL_CERT_FILE` and `REQUESTS_CA_BUNDLE` = full bundle | DeepSeek | Native `deepseek/<id>` through LiteLLM (path `/beta`) | **No** | `.aider.chat.history.md` in the repo root | `--no-check-update` (needed: `--yes-always` accepts upgrades) | Low priority: no subagents, stale. |
| H12 | Kilo CLI | 7.8.25 tag (28 Oct); npm `latest` 7.8.18 | Bun binary, Node launcher | **Likely** | `NODE_EXTRA_CA_CERTS` or system store | DeepSeek | Native (models.dev catalog) | Yes, in-process `task` | `~/.local/share/kilo/kilo.db` (SQLite) | `KILO_DISABLE_AUTOUPDATE=1` (patches install silently by default) | Test. |
| H13 | Qwen Code | 0.26.4 (29 Oct) | Node 22+ | **Likely** (but a `proxy` setting or `--proxy` beats `HTTPS_PROXY`) | `NODE_EXTRA_CA_CERTS` | DeepSeek | Native preset (OpenAI format) | Yes, in-process `agent` tool | `~/.qwen/projects/<cwd>/chats/<id>.jsonl` plus `subagents/<id>/agent-*.jsonl` | `general.enableAutoUpdate: false` (no env switch) | Test. |
| H14 | Crush | 0.99.0 (29 Oct) | Go 1.27 | **Likely** | `SSL_CERT_FILE` = full bundle (or system store) | DeepSeek | Native Catwalk provider `deepseek` | Yes, read-only search sub-agents, in-process | `<project>/.crush/crush.db` (SQLite) | Binary does not self-update; `CRUSH_DISABLE_PROVIDER_AUTO_UPDATE=1` for the catalog | Test with a search prompt. |
| H15 | Factory Droid | 0.251.0 (27 Oct) | Bun 1.4.4 binary | **Likely** | `NODE_EXTRA_CA_CERTS` or `FACTORY_EXTRA_CA_CERTS` | DeepSeek plus free Factory account | `customModels` with `generic-chat-completion-api` at `https://api.deepseek.com/v1` | Yes, but reported broken on BYOK (issue 691) | `~/.factory/sessions/<slug>/<uuid>.jsonl` | npm builds off; `FACTORY_DROID_AUTO_UPDATE_ENABLED=false` | Test after the account; subagents may fail. |

One CA rule covers everyone: goalcup can pass both `NODE_EXTRA_CA_CERTS=<run CA>` (Node and Bun add it
to their roots) and `SSL_CERT_FILE=<full bundle>` (Python, Go and rustls-native-certs replace their
roots with it), plus `CODEX_CA_CERTIFICATE`, `GOOSE_CA_CERT_PATH` and `REQUESTS_CA_BUNDLE` where they
apply. Passing the run CA alone in `SSL_CERT_FILE` breaks every non-proxied or non-model host.

## O.2 Proxy dodgers and why

Full dodgers (the proxy can never see the provider call):
- **Cursor CLI (H7).** Cursor's servers build the prompt and call the provider. The CLI talks Connect
  RPC over HTTP/2 to `*.cursor.sh`, and the server can force HTTP/2 (`server_force_all_enabled`), which
  our HTTP/1.1 proxy refuses. The IDE ignores proxy settings with HTTP/2 on (staff confirmed 29 Oct 2026,
  https://forum.cursor.com/t/agent-ignores-http-proxy-and-https-proxy-when-http-2-is-enabled-bypasses-corporate-proxy/173619).
- **Amp (H6).** "Amp sends model requests from its own servers" (https://ampcode.com/docs/customize/model-routing).
  The proxy records Amp's thread WebSocket, never the DeepSeek or other provider request, for any key.

Conditional dodgers (a setting, a daemon or a mode routes around the proxy):
- **OpenClaw (H5):** `proxy.proxyUrl` and `OPENCLAW_PROXY_URL` beat `HTTPS_PROXY`; subagents run in a
  Gateway daemon with its own environment; raw `net`, `tls`, `http2` sockets and non-OpenClaw children
  "may bypass".
- **Qwen Code (H13):** a `proxy` key in settings or `--proxy` beats `HTTPS_PROXY`; its built-in
  `claude-code` and `codex` subagents start other harnesses.
- **OpenCode 2 (H4):** by default attaches to a shared background service that keeps the env it started
  with; `--standalone` fixes it.
- **Hermes (H3):** a gateway already running for the same `HERMES_HOME` makes calls with its own env,
  direct.
- **Codex (H2):** `$CODEX_HOME/.env` overwrites `HTTPS_PROXY`, `NO_PROXY` and `SSL_CERT_FILE`; a shared
  app-server daemon keeps its own env (`--no-daemon` avoids it).
- **Claude Code (H1):** a settings `env` block may set proxy values (whether it beats the launch env is
  unverified); the agent-view supervisor keeps the env of the shell that started it first; Bedrock,
  Vertex, Foundry, Mantle and the Claude apps gateway.
- **Factory Droid (H15):** a separately started `droid daemon` or the desktop app; cloud Droid Computers
  and Missions.
- **Gemini CLI (H8):** `--sandbox` sets its own `HTTPS_PROXY` for tools; remote A2A subagents.
- **Copilot CLI (H9):** `/delegate` to GitHub's cloud agent (needs sign-in); the voice runtime ignores
  proxy settings (issue #5245).
- **Crush (H14):** opt-in `CRUSH_CLIENT_SERVER=1` with an older `crush server`.
- No dodge found for model calls: Goose (H10), Aider (H11), Kilo (H12).
- Every harness: a participant who sets `NO_PROXY` to include the provider host.
- Backups: DeepSeek Harness reads proxy values from `$DSH_HOME/.env` after the env and sends telemetry
  direct; Cline's hub daemon may already run with another env; Plandex makes model calls on its server.

## O.3 Needs its own key or account

| Harness | What it needs | Can it use DeepSeek? |
|---|---|---|
| Claude Code (H1) | Claude API key (owner's plan; Claude Code has no other bring-your-own-key route in the test plan) | Yes through the Anthropic-format endpoint, but that is a base URL; not used |
| Codex CLI (H2) | OpenAI API key (owner's plan) | Yes since DeepSeek added a Responses API (O.5); not used |
| Amp (H6) | Amp account always (`amp login` or an `sgamp_` token); Amp credits or a key stored in Amp Model Routing | Only server-side; never on the local wire |
| Cursor CLI (H7) | Cursor account and plan (`CURSOR_API_KEY`) | No |
| Gemini CLI (H8) | Paid Gemini API key (Google AI Studio or Agent Platform) | No |
| Factory Droid (H15) | Free Factory account and `FACTORY_API_KEY` (a second secret to mask) on top of DeepSeek | Yes, for inference |

Every other harness (Hermes, OpenCode, OpenClaw, Copilot CLI, Goose, Aider, Kilo, Qwen Code, Crush) runs
on a DeepSeek key alone. Copilot needs no GitHub login with bring-your-own-key.

## O.4 Likely verified by Monday 2 Nov, and risky

This is a judgement from the facts below, not a promise. "Likely" means: measured before, or a clean
source path plus a native DeepSeek route plus in-process subagents.

Likely verified by Monday:
1. Claude Code (H1): measured on 2.1.312.
2. OpenCode 1 (H4): measured 28 Oct, native DeepSeek.
3. Codex CLI (H2): clean source path; the WebSocket recording is the one open check.
4. Hermes Agent (H3): proxy measured; needs the full-bundle CA and an old-gateway check.
5. Goose (H10).
6. Kilo CLI (H12).
7. Qwen Code (H13).
8. Crush (H14): the subagent prompt must ask for a search.
9. GitHub Copilot CLI (H9): the CA layer is unverified, but a failure would be loud, not silent.

Risky:
- OpenClaw (H5): subagents likely need a Gateway daemon, which changes how goalcup launches it.
- Factory Droid (H15): account gate, BYOK subagent bug (issue 691), daily releases, daemon.
- Aider (H11): proxy is easy, but there are no subagents and it is stale.
- Gemini CLI (H8): blocked until the owner funds a paid Google key.
- OpenCode 2 (H4): works only with `--standalone`; a participant without it may record nothing.
- Amp (H6): can be "verified" only as Amp's protocol, never as provider calls.
- Cursor CLI (H7): expected to fail; test only to prove it fails cleanly.

What could go wrong with this split: every harness ships fast (Copilot and Droid about daily, Amp about
7 builds a day), so a pass on Monday may not hold on 7 Nov; pin versions and re-check changelogs on 5 Nov.

## O.5 DeepSeek facts (29 Oct 2026)

### O.5.0 Short answer

1. **DeepSeek is a good test provider for the pad.** Two hosts only matter: `https://api.deepseek.com`
   (OpenAI format: Chat Completions and, new, the Responses API) and `https://api.deepseek.com/anthropic`
   (Anthropic Messages). Plain HTTPS, SSE streaming, HTTP/1.1 accepted (probe), no pinning, 1M context.
   Our HTTP/1.1-only proxy (`crates/goalcup-cli/src/proxy/tls.rs:26-30`) fits.
2. **The owner's price is low by about 20 percent.** The cheap model costs **0.15 in (cache miss) and
   0.60 out per million tokens off-peak**, and double that in peak hours (0.30 and 1.20). Cache hits cost
   0.003 off-peak. The owner's 0.12 and 0.48 match no tier on the page. It is still very cheap: a
   10-minute test is roughly 0.10 dollars off-peak (estimate, section O.5.2).
3. **Peak hours exist now.** Weekdays 01:00 to 04:00 and 06:00 to 10:00 UTC cost double. Schedule the
   overnight pad batches outside those hours, or accept twice the cost.
4. **Correction to part O1 (see O.8.4):** DeepSeek now supports the Responses API and publishes a Codex guide, so Codex is
   DeepSeek-ready (part O1 said "no, Responses only", sections H2.2 and H2.7). It does not change the plan:
   Codex is tested with the OpenAI key.
5. **One real outage this month:** 15 Oct 2026, about 00:11 to 05:04 UTC, API down. An overnight pad run
   will sometimes meet 429, 500 or 503 from DeepSeek. The pad must count those as recorded calls, not as
   recorder failures.
6. **Best swap-ins if one of the 15 fails:** (1) Kimi Code, (2) Mistral Vibe, (3) Cline CLI,
   (4) DeepSeek Harness (`dsh`), (5) Letta Code in local mode. All five run headless on Linux, take a
   DeepSeek key, and have subagents a prompt can ask for. Pi is very popular but has **no built-in
   subagents**. Roo Code is shut down, Continue CLI and OpenHands CLI look stale, Warp, Qoder and
   CodeBuddy need their own accounts, Plandex runs model calls on a server.

### O.5.1 Hosts, paths, formats

| Topic | Fact | Source |
|---|---|---|
| Hosts | OpenAI format base `https://api.deepseek.com`; Anthropic format base `https://api.deepseek.com/anthropic`. Fronted by CloudFront (`server: elb`, `via: ... cloudfront.net`) | https://api-docs.deepseek.com/quick_start/pricing (fetched 29 Oct 2026); probe |
| Chat Completions | `POST /chat/completions` in the docs. `POST /v1/chat/completions` also answers 401 without a key. Harness defaults differ: Hermes and one Cline entry use `.../v1`, CodeBuddy's guide uses `https://api.deepseek.com/v1/chat/completions` | https://api-docs.deepseek.com/ ; probe; https://api-docs.deepseek.com/quick_start/agent_integrations/workbuddy |
| Responses API | New. Base `https://api.deepseek.com`, "To meet the demand for Codex". Semantic SSE events with `sequence_number`; the stream ends with `response.completed`, `response.incomplete` or `response.failed`, **no `data: [DONE]`**. `/responses` and `/v1/responses` both answer 401 (probe), so the route exists | https://api-docs.deepseek.com/guides/responses_api ; probe |
| Anthropic Messages | `POST /anthropic/v1/messages`. `x-api-key` "Fully Supported"; `anthropic-version` and most of `anthropic-beta` ignored. DeepSeek's own Claude Code guide sets `ANTHROPIC_AUTH_TOKEN`, which sends `Authorization: Bearer`, so Bearer must work too (inferred, unverified) | https://api-docs.deepseek.com/guides/anthropic_api ; https://api-docs.deepseek.com/quick_start/agent_integrations/claude_code |
| Model names on the Anthropic route | Names that start with the Claude prefix are accepted: the top family maps to the large model, the others to the cheap model. Other unknown names give 400. So a harness on this route may send one model name and be served another | anthropic_api page above |
| Models | Two: the cheap model (vision, concurrency 2,500) and the large model (no vision, concurrency 500). Two older names "are still accepted, but the corresponding models have been retired" and bill at the cheap price. The cheap model was refreshed on 17 Oct 2026 | pricing page; https://api-docs.deepseek.com/news/news261017 |
| Context, output | Context 1M tokens for both; max output 384K. One request can be several MB of JSON | pricing page |
| Features | JSON output, tool calls, Responses API, Anthropic API, prefix completion (beta), FIM (beta, non-thinking only) on both models | pricing page |
| Streaming | SSE. Chat Completions ends with `data: [DONE]`. While a request waits, streaming responses carry `: keep-alive` SSE comment lines and non-streaming responses carry **empty lines before the JSON**. "If the request has not started inference after 10 minutes, the server will close the connection" | https://api-docs.deepseek.com/quick_start/rate_limit |
| Usage in streams | Chat Completions sends usage in a stream only if the client sets `stream_options.include_usage`; the harness decides. Usage fields: `prompt_tokens`, `completion_tokens`, `prompt_cache_hit_tokens`, `prompt_cache_miss_tokens`, `completion_tokens_details.reasoning_tokens`. Responses API: full usage in `response.completed` | https://api-docs.deepseek.com/api/create-chat-completion ; https://api-docs.deepseek.com/guides/kv_cache ; responses_api page |
| Tool calling | OpenAI `tools` with `tool_calls`, also in thinking mode. `strict` mode needs base `https://api.deepseek.com/beta`. Only the Anthropic and Responses formats accept tool calls inserted mid-conversation | https://api-docs.deepseek.com/guides/tool_calls |
| Reasoning output | Thinking is **on by default** (effort high). Chat Completions: `reasoning_content` next to `content` (in `message` and in stream `delta`); with tools, the client must send it back next turn. Anthropic route: `thinking` blocks (`budget_tokens` ignored, `output_config.effort` honoured). Responses route: `response.reasoning_text.delta` events. Toggle: `thinking: {type}` plus `reasoning_effort` (low, high, max) | https://api-docs.deepseek.com/guides/thinking_mode |
| Server-side tools | Web search and fetch run **on DeepSeek's servers** for Claude Code, Codex and DeepSeek's own harness. They show up only as `server_tool_use` / `web_search_tool_result` blocks or `web_search_call` events inside the model response. Free until 31 Oct 2026 16:00 UTC, then 0.75 dollars per 1,000 requests | pricing page; anthropic_api page; responses_api page |
| HTTP version | HTTP/2 offered, HTTP/1.1 accepted (probe: `HTTP/1.1 401` and `HTTP/2 401`). No `alt-svc: h3` seen. No WebSocket or gRPC model API | probe 29 Oct 2026 |
| Errors | 400, 401, 402 Insufficient Balance, 422, 429 Rate Limit Reached, 500, 503 Server Overloaded | https://api-docs.deepseek.com/quick_start/error_codes |

What this means for goalcup:
- The proxy forwards raw bytes and parses only HTTP framing (`crates/goalcup-cli/src/proxy/http.rs:1-12`),
  so keep-alive comments and leading empty lines pass through untouched. Recording is fine.
- The verifier already reads `reasoning_content` and `prompt_cache_hit_tokens`
  (`crates/goalcup-verifier/src/capture_analysis/chat.rs:134-142,625`,
  `crates/goalcup-verifier/src/capture_analysis/stats.rs:420`). For the owner's per-call token stats,
  streamed Chat Completions **without** `include_usage` carry no usage at all: the stats must say "not
  given" for those calls, not zero.
- The Responses stream has no `[DONE]`. Any "stream ended cleanly" test must accept a final
  `response.completed` event instead.
- On the Anthropic route the request's model name may not be the model that served it. Research stats
  should read the model from the response if present (unverified whether DeepSeek echoes the served
  model there).

### O.5.2 Prices per million tokens (USD)

| Item | Cheap model off-peak | Cheap model peak | Large model off-peak | Large model peak |
|---|---|---|---|---|
| Input, cache hit | 0.003 | 0.006 | 0.022 | 0.044 |
| Input, cache miss | **0.15** | 0.30 | 0.66 | 1.32 |
| Output | **0.60** | 1.20 | 1.98 | 3.96 |

Source: https://api-docs.deepseek.com/quick_start/pricing (fetched 29 Oct 2026). "Off-peak rates are half
of the peak rates. Peak hours are 01:00 - 04:00 and 06:00 - 10:00 UTC, Monday through Friday, excluding
Chinese public holidays." Weekends are off-peak. 7 and 8 Nov 2026 are a Saturday and Sunday.

The owner believes 0.12 in and 0.48 out. **That does not match the page.** Off-peak cache-miss input is
0.15 and output 0.60. Agent harnesses resend a long prefix, so most input is a cache hit (0.003). Rough
cost of one 10-minute pad test (estimate, not measured): 3M input tokens at 90 percent cache hits plus
60K output tokens = 0.3M x 0.15 + 2.7M x 0.003 + 0.06M x 0.60, about 0.09 dollars off-peak and 0.18 at
peak. Fifteen harnesses times a few retries stays under 10 dollars a night. Blind spot: a harness that
breaks the cache (some do, on every subagent) can cost 5 to 10 times more; still small.

### O.5.3 Rate limits, concurrency, outages

- Limits are **concurrency per account**, not per key: 2,500 open requests for the cheap model, 500 for
  the large one; over that gives 429. "A request counts as one concurrent connection from the time it is
  sent until the model response is complete." No published request-per-minute limit. Raising it is
  free on request. Source: https://api-docs.deepseek.com/quick_start/rate_limit.
- All pad tests on one account share that pool. Thousands of parallel subagents on the large model
  could hit 500. Not a problem for the pad.
- Status page (https://status.deepseek.com/, fetched 29 Oct 2026, script-rendered): "Everything is
  running smoothly"; uptime Aug to Oct 2026: cheap-model API 99.47 percent, large-model API 99.74 percent.
  This corrects the earlier note that the page showed no incidents (`docs/plans/2026-10-30-audit/E-providers.md:220`).
- **15 Oct 2026 outage:** web, app and both model APIs, 08:11 to 13:04 Beijing time (00:11 to 05:04 UTC),
  almost five hours. Sources: https://www.yicaiglobal.com/news/deepseek-suffers-nearly-five-hour-ai-service-outage-one-of-its-worst-this-year
  (15 Oct 2026); https://scmp.com/tech/big-tech/article/3371047/deepseek-offers-user-refunds-after-suffering-another-major-outage
  (refund as credits, secondary, unverified). Earlier partial outages on 2 Sep, 15 Sep and 12 Oct are
  reported by secondary sources only (unverified).
- What could go wrong: an overnight batch that meets an outage looks like 15 failed harnesses. The pad
  should check provider status codes first and mark such a run "provider down", not "proxy failed".

### O.5.4 Which harnesses list DeepSeek

DeepSeek's own "Agent Integrations" menu (fetched 29 Oct 2026, https://api-docs.deepseek.com/) lists:
DeepSeek Harness, Claude Code, Codex, OpenCode, OpenClaw, Hermes, Reasonix, WorkBuddy/CodeBuddy, Qoder.

| Harness | DeepSeek route | Kind | Source |
|---|---|---|---|
| Claude Code | `ANTHROPIC_BASE_URL=https://api.deepseek.com/anthropic`, key in `ANTHROPIC_AUTH_TOKEN` | Anthropic-compatible base URL | claude_code guide above; H1.2. Test uses the Claude key |
| Codex | `[model_providers.deepseek]` in `~/.codex/config.toml` plus a `~/.codex/models.json` catalog; DeepSeek ships a setup script. Its catalog sets `prefer_websockets: false` | Responses-compatible base URL (**new; corrects O1**) | https://api-docs.deepseek.com/quick_start/agent_integrations/codex . Test uses the OpenAI key |
| Hermes | native provider `deepseek` | native | H3.2 |
| OpenCode | native provider `deepseek` | native | H4.2 |
| OpenClaw | plugin `@openclaw/deepseek-provider` | native via plugin | H5.2 |
| Kilo CLI | native, models.dev catalog | native | H12.2 |
| Aider | `deepseek/<id>` through LiteLLM | native | H11.2 |
| Goose | custom OpenAI-compatible provider | custom | H10.2 |
| Copilot CLI | BYOK, Anthropic type at `/anthropic` | custom | H9.2, H9.7 |
| Crush | `provider add deepseek --type completions --base-url https://api.deepseek.com/v1` | custom | https://github.com/charmbracelet/crush/blob/v0.99.0/README.md#L756-L773 ; H14.2 |
| Qwen Code, Factory Droid | OpenAI-compatible custom provider | custom | H13.2, H15.2 |
| Amp, Cursor CLI, Gemini CLI | need their own account or key | none for the test | H6.7, H7.7, H8.7 |
| Backups: Cline, Pi, Letta Code, DeepSeek Harness, Reasonix | native | native | Part B |
| Backups: Kimi Code, Mistral Vibe, OpenHands, CodeBuddy, Qoder | OpenAI-compatible custom, or wizard | custom | Part B |

## O.6 Backup harnesses, ranked

Popularity hint: weekly downloads 21 to 27 Oct 2026 from `api.npmjs.org/downloads/point/2026-10-21:2026-10-27/<pkg>`
and `pypistats.org/api/packages/<pkg>/recent` (last_week). Downloads are not users.

### O.6.1 Ranked summary

| Rank | Harness (latest, date) | Weekly downloads | Runtime | Proxy-ready | DeepSeek-ready | Subagents | Swap-in verdict |
|---|---|---|---|---|---|---|---|
| 1 | Kimi Code `@moonshot-ai/kimi-code` 2.4.0 (28 Oct) | 111,441 (npm) | Node 22.19+ (npm route) | Likely | Yes (`openai` type) | Yes, `Agent` tool, one `wire.jsonl` per subagent | Best fit: JSONL per agent, proxy code in source, clean off switch |
| 2 | Mistral Vibe `mistral-vibe` 3.0.3 (26 Oct) | 661,523 (PyPI) | Python 3.12+ | Likely | Yes (generic `openai` provider) | Yes, `task` tool, in-process | Strong; very popular |
| 3 | Cline CLI `cline` 3.0.74 (26 Oct) | 86,058 | Bun binary | Likely | Yes (native `deepseek`) | Yes, `spawn_agent` on by default (source) | Good; hub daemon must be kept off |
| 4 | DeepSeek Harness `@deepseek-ai/dsh` 0.2.1-rc.5 (29 Oct) | 540,139 | Node | Likely | Yes (native) | Yes, `subagent` and `subagent_fork` tools | Good, but developer preview with release candidates |
| 5 | Letta Code `@letta-ai/letta-code` 0.34.31 (29 Oct) | 56,976 | Node 22.23.4+ or 24.22.1+ | Likely, only with `NODE_USE_ENV_PROXY=1` | Yes in local mode (native) | Yes, child processes | Usable; cloud backend is the default |
| 6 | OpenHands CLI `openhands` 1.16.0 (8 May) | 28,043 (PyPI) | Python 3.12, LiteLLM | Likely | Yes (LiteLLM) | Unverified in the CLI | Stale for 5 months |
| 7 | Pi `@earendil-works/pi-coding-agent` 1.4.0 (21 Oct) | 4,929,894 | Node 22.19+ | Yes (docs) | Yes (native) | **No built-in**; an example extension spawns child `pi` processes with `--no-session` | Only if a subagent-free harness is acceptable |
| 8 | CodeBuddy Code `@tencent-ai/codebuddy-code` 2.178.1 (29 Oct) | 105,189 | Node | Likely | Needs own account, then `models.json` | Yes (`subagent_type` in bundle) | Needs a Tencent account |
| 9 | Qoder CLI `@qoder-ai/qodercli` 1.1.81 (29 Oct) | 11,475 | Node 20+ | Unverified | Needs own account; DeepSeek via interactive `/model` wizard only | Unverified | Needs an account and a TTY to set up |
| 10 | Reasonix `reasonix` 2.54.0 (28 Oct) | 5,218 | Go 1.26.10 binary | Likely | Yes (DeepSeek only) | Yes, via skills `runAs: subagent` | Low use |
| 11 | Plandex cli/v2.2.1 (16 Jul) | n/a | Go CLI plus server | No for the CLI | Yes on the server | No | Model calls leave from the server |
| 12 | Warp agent CLI | n/a | closed | Unknown | Needs Warp account; BYOK only for three vendors, no DeepSeek | Unknown | No |
| 13 | Continue CLI `@continuedev/cli` 1.5.47 (18 Jun) | 5,527 | Node | Unknown | Custom | Unverified | No release in 4 months |
| 14 | Roo Code CLI cli-v0.1.17 (4 Mar, pre-release) | n/a | Node | n/a | n/a | n/a | Repository archived 15 May 2026: dead |

Not ranked: Auggie (`@augmentcode/auggie`, needs an Augment account), iFlow CLI (0.5.19, 25 Apr 2026,
no release since), Antigravity CLI (no DeepSeek route, see appendix D4).

### O.6.2 Per harness

**1. Kimi Code (Moonshot), the successor to the Python Kimi CLI.**
(1) `@moonshot-ai/kimi-code` 2.4.0, 28 Oct 2026; 6 npm releases since 29 Sep plus daily canary tags
(`git ls-remote https://github.com/MoonshotAI/kimi-code`). The Python `kimi-cli` 1.52.0 (22 Sep) is being
phased out (https://moonshotai.github.io/kimi-code/en/guides/migration). Install `npm i -g
@moonshot-ai/kimi-code@2.4.0` (Node 22.19+) or `curl -fsSL https://code.kimi.com/kimi-code/install.sh | bash`.
Headless: `kimi -p "<prompt>" --output-format stream-json`; `-p` always runs with auto permission and
rejects `--yolo` (https://moonshotai.github.io/kimi-code/en/reference/kimi-command).
(2) BYOK yes. Provider types `kimi`, `anthropic`, `openai`, `openai_responses`, `google-genai`, `vertexai`;
`openai` "including DeepSeek", and `reasoning_content` is handled. Config `$KIMI_CODE_HOME/config.toml`
(default `~/.kimi-code/`): `[providers.deepseek] type = "openai"`, `base_url = "https://api.deepseek.com/v1"`,
`api_key_env = "DEEPSEEK_API_KEY"`. It reads no shell key unless `api_key_env` is declared
(https://moonshotai.github.io/kimi-code/en/configuration/providers). Test key: DeepSeek.
(3) Node bundle. When a proxy variable is set it installs undici `EnvHttpProxyAgent` as the global
dispatcher, and it passes `NODE_USE_ENV_PROXY=1`, `HTTP(S)_PROXY` and `NO_PROXY` to child processes
(source, `dist/main.mjs`, `proxyEnvForChild`). CA: `NODE_EXTRA_CA_CERTS` (Node standard; not referenced in
the bundle, unverified). No HTTP/2, WebSocket model path or pinning found. Telemetry client is a no-op by
default in the SDK layer (source, unverified for the app). The managed Kimi service host is
`api.kimi.com`; not used with a DeepSeek provider.
(4) Subagents yes: tool `Agent` (and `AgentSwarm`), built-in `coder`, `explore`, `plan` subagents. They
run inside the same session state machine (source), so in-process (unverified).
(5) `sessions/<workspaceId>/<sessionId>/agents/main/wire.jsonl`, and **one directory per subagent**
(`agents/agent-0/wire.jsonl`); `session_index.jsonl`; OAuth credentials under `credentials/` (0600). An
API key written into `config.toml` would be a secret on disk; use `api_key_env`
(https://moonshotai.github.io/kimi-code/en/configuration/data-locations).
(6) Auto-update on by default. `KIMI_CODE_NO_AUTO_UPDATE=1` "fully disables automatic update behavior: no
check, no background install, no prompt, and no staged-swap" (source); also `[upgrade].auto_install` in `tui.toml`.
(7) Proxy-ready **likely**, DeepSeek-ready **yes**, subagents **yes**. Recipe: install 2.4.0 by npm; set
`KIMI_CODE_HOME=<pad>/kimi`, write the provider block above plus a default model alias for
`<cheap model id>` (field names for the model alias are unverified; check `kimi --help` and the
configuration reference); env: `DEEPSEEK_API_KEY`, `HTTPS_PROXY=http://127.0.0.1:<port>`,
`NODE_EXTRA_CA_CERTS=<run CA>`, `KIMI_CODE_NO_AUTO_UPDATE=1`. Prompt: "Use the Agent tool to start one
explore subagent that lists every Python file in this repo and counts their lines. Then write the total
to COUNT.txt yourself." Check: every `POST` to `api.deepseek.com` in the record matches a model turn in
`agents/*/wire.jsonl`; there are two agent directories; the key is masked; the server copy is whole.

**2. Mistral Vibe.**
(1) `mistral-vibe` 3.0.3, 26 Oct 2026; 6 releases since 29 Sep (PyPI; tags v3.0.0 to v3.0.3). Install
`uv tool install mistral-vibe==3.0.3` (Python 3.12+, GIL build). Headless: `vibe --prompt "<p>"
--auto-approve --output json --output-dir <dir>`, plus `--max-turns`, `--time-limit`
(https://raw.githubusercontent.com/mistralai/mistral-vibe/v3.0.3/README.md:294-335).
(2) BYOK yes. `$VIBE_HOME/config.toml` (default `~/.vibe/`): `[[providers]] name = "deepseek"`,
`api_base = "https://api.deepseek.com/v1"`, `api_key_env_var = "DEEPSEEK_API_KEY"`, `api_style = "openai"`,
`backend = "generic"`; `[[models]] name = "<cheap model id>"`, `provider = "deepseek"`, `alias = "ds"`;
`active_model = "ds"` (https://docs.mistral.ai/mistral-vibe/terminal/configuration). No DeepSeek example in
the docs: this exact block is unverified. Test key: DeepSeek.
(3) Python on httpx. Model calls use `env_proxy_transport`, which reads the proxy from the environment
(`getproxies()`) and honours `NO_PROXY` (source, `mistralai_vibe_local_harness/vibe/_http_transport.py:1-45`,
used by `adapters/generic/_execute.py`). TLS: certifi plus the OS store via truststore, and
`SSL_CERT_FILE` / `SSL_CERT_DIR` are **added** as extra roots (source `vibe/_ssl.py:14-47`; README:835-843).
So `SSL_CERT_FILE=<run CA>` alone should work (unverified). No HTTP/2 found in source. OpenTelemetry export
is off unless `enable_otel = true`.
(4) Subagents yes: `task` tool; built-in read-only `explore`; custom ones with `agent_type = "subagent"`.
In-process: no subprocess calls in `vibe/_subagents/` (source).
(5) One unified session store per session, journal segments `journal/<16 digits>.jsonl` (source
`vibe/_storage.py:96,140`); logs under `$VIBE_HOME/logs/`; keys in `$VIBE_HOME/.env`.
(6) Daily update check; `enable_update_checks = false` turns it off (README:1391-1405).
(7) Proxy-ready **likely**, DeepSeek-ready **yes**, subagents **yes**. Recipe: pin 3.0.3; `VIBE_HOME=<pad>/vibe`
with the config above plus `enable_update_checks = false`; env `DEEPSEEK_API_KEY`,
`HTTPS_PROXY`, `SSL_CERT_FILE=<run CA>`. Prompt: "Use the task tool to delegate to the explore subagent:
find every Python file and count its lines. Then write the total to COUNT.txt." Check: wire calls match
the journal; the subagent's turns appear (in the main journal or its own; layout unverified); key masked;
a first-run trust prompt for a folder with `.vibe/` does not block headless mode (unverified).

**3. Cline CLI.**
(1) `cline` 3.0.74, 26 Oct 2026; 33 npm versions since 29 Sep including nightlies. `npm i -g cline@3.0.74`
pulls `@cline/cli-linux-x64` (a Bun-compiled binary, source). Headless: `cline --json "<prompt>"`,
auto-approve defaults to true, `-t <seconds>` timeout (https://docs.cline.bot/cli/cli-reference).
(2) BYOK yes. Native provider id `deepseek`, base `https://api.deepseek.com`, env `DEEPSEEK_API_KEY`
(source, provider table in the binary). Non-interactive setup: `cline auth -p deepseek -k "$DEEPSEEK_API_KEY"
-m <cheap model id>`; that writes the key into the config directory (`CLINE_DIR`, default `~/.cline/`).
Test key: DeepSeek.
(3) Bun binary. Docs: "uses standard HTTP proxy environment variables"; custom CA via
`NODE_EXTRA_CA_CERTS` (PEM); HTTP proxies only, no SOCKS or PAC
(https://docs.cline.bot/troubleshooting/networking-and-proxies.md). Skips: a local **hub daemon**
(port 25463, WebSocket clients) starts "automatically when needed" and may already be running with
another environment; `CLINE_SESSION_BACKEND_MODE=local` keeps the session in-process (cli-reference;
source reads it). PostHog telemetry strings are in the binary (source); it is not a model call.
(4) Subagents yes: `spawn_agent` tool, enabled unless settings set `enableSpawn` to false (source;
defaults to on). Also agent teams. Process model unverified.
(5) `~/.cline/data/sessions/<id>/` per session plus SQLite index `~/.cline/data/db/sessions.db`
(cli-reference). `CLINE_DATA_DIR` moves it.
(6) `cline update`; the binary sets `CLINE_NO_AUTO_UPDATE=1` when it starts its own children (source), so
that variable is the switch (meaning inferred, unverified).
(7) Proxy-ready **likely**, DeepSeek-ready **yes**, subagents **yes**. Recipe: pin 3.0.74;
`CLINE_DIR=<pad>/cline`, `CLINE_DATA_DIR=<pad>/cline/data`, run `cline auth` as above; env `HTTPS_PROXY`,
`NODE_EXTRA_CA_CERTS=<run CA>`, `CLINE_SESSION_BACKEND_MODE=local`, `CLINE_NO_AUTO_UPDATE=1`. Prompt:
"Spawn one sub-agent to count the lines in every Python file here; then write the total to COUNT.txt."
Check: no hub process started (`cline hub status`); wire calls match session files; the key in
`CLINE_DIR` never reaches the upload.

**4. DeepSeek Harness (`dsh`).**
(1) `@deepseek-ai/dsh` 0.2.1-rc.5, 29 Oct 2026; 12 versions since 29 Sep, all release candidates or
alphas; "developer preview" (https://api-docs.deepseek.com/). DeepSeek recommends 0.2.1-rc.1 or later for
its refreshed model (news261017). Install `npm i -g @deepseek-ai/dsh@0.2.1-rc.5`. Headless:
`dsh --profile headless "<job>"`: "Run one fresh persisted session, print the final answer, and exit"
(package README).
(2) BYOK yes, DeepSeek native. Key setup is documented for the Web UI only
(https://deepseek-harness.github.io/deepseek-harness/en/guide/quickstart); `$DSH_HOME/.env` and
`$DSH_HOME/.credentials.yaml` exist (source); the exact env var name is unverified.
(3) Node. Routes global `fetch` through proxy variables from the environment, **then from
`$DSH_HOME/.env`**, so a `.env` can override ours; loopback always bypassed; "No custom certificate
authority": use `NODE_EXTRA_CA_CERTS`; OTLP telemetry goes direct to `dsh-otel-collector.deepseeksvc.com`
(`docs/plans/2026-10-30-audit/E-providers.md`, section E3.2 "DeepSeek's own harness").
(4) Subagents yes: `subagent` and `subagent_fork` "create managed children"; out-of-process backends also
exist, plus agent teams and a Ralph loop (`@deepseek-ai/dsh-base` README:52; bundled packages list).
In-process by default (unverified).
(5) `$DSH_HOME`, SQLite or JSON storage backends (packages list); layout unverified.
(6) Updates: unverified.
(7) Proxy-ready **likely**, DeepSeek-ready **yes**, subagents **yes**. Recipe: pin the rc; `DSH_HOME=<pad>/dsh`
with no proxy lines in its `.env`; env `HTTPS_PROXY`, `NODE_EXTRA_CA_CERTS`. Prompt: "Use the subagent
tool for one child that counts lines in all Python files; then write the total to COUNT.txt." Check: the
telemetry host shows up as an unrecorded tunnel or direct connection (expected, not a model call); child
calls recorded. Risk: rc churn breaks the recipe between today and Monday.

**5. Letta Code.**
(1) `@letta-ai/letta-code` 0.34.31, 29 Oct 2026; 35 versions since 29 Sep. Also `uv tool install letta`.
Headless: `letta -p "<prompt>"` (package README). **Letta Cloud is the default backend**; `--backend local`
or `letta backend local` keeps agent state local (README).
(2) BYOK yes. In local mode the provider catalog has native `deepseek` at `https://api.deepseek.com` with
`DEEPSEEK_API_KEY` (source, `letta.js`). In cloud mode, model calls may run on Letta's servers
(unverified): do not test that mode.
(3) Node script (`#!/usr/bin/env node`). No global dispatcher found (source), so model `fetch` follows
`HTTPS_PROXY` only with `NODE_USE_ENV_PROXY=1` on Node 22.21+ or 24+ (unverified for this harness). CA:
`NODE_EXTRA_CA_CERTS`.
(4) Subagents yes (general-purpose, forked, recall). They are **child processes** spawned with
`...workerEnv` plus `LETTA_CODE_AGENT_ROLE=subagent`, so they inherit our env (source).
(5) Under `~/.letta` (`LETTA_CONFIG_DIR`); transcripts as `messages.jsonl` plus `manifest.json` per
conversation (source); subagent processes skip session persistence (`shouldPersistSessionState`, source).
(6) Self-updates through npm; `DISABLE_AUTOUPDATER=1` turns it off (source).
(7) Proxy-ready **likely** (with `NODE_USE_ENV_PROXY=1`), DeepSeek-ready **yes**, subagents **yes**.
Recipe: pin; env `DEEPSEEK_API_KEY`, `HTTPS_PROXY`, `NODE_USE_ENV_PROXY=1`, `NODE_EXTRA_CA_CERTS`,
`DISABLE_AUTOUPDATER=1`, `LETTA_CONFIG_DIR=<pad>/letta`; run `letta --backend local -p "<prompt>"`.
Check: subagent calls recorded even though subagents write no session file of their own, so the wire is
the only record of them.

**6. OpenHands CLI.** 1.16.0 on PyPI, 8 May 2026, nothing since (tag 1.16.0 is the newest in
`OpenHands/OpenHands-CLI`); the docs index no longer lists CLI pages (https://docs.openhands.dev/llms.txt).
`uv tool install openhands --python 3.12`; headless `openhands --headless -t "<task>"` (`--json` for
JSONL). LLM settings in `~/.openhands/settings.json`; `LLM_MODEL`, `LLM_API_KEY`, `LLM_BASE_URL` are ignored
unless `--override-with-envs` (README at 1.16.0, line 68). DeepSeek through LiteLLM (`deepseek/<id>`).
Python, LiteLLM reads proxy env on both transports; CA from `SSL_CERT_FILE` (see `docs/plans/2026-10-30-audit/E-providers.md`,
part E-deepseek-local, SDK table). Sub-agents exist in the SDK ("Task Tool Set", https://docs.openhands.dev/sdk/guides/task-tool-set.md);
whether the CLI enables them is unverified. Conversations in `~/.openhands/conversations`. Proxy-ready
likely, DeepSeek-ready yes, subagents unverified. Low priority: stale.

**7. Pi.** `@earendil-works/pi-coding-agent` 1.4.0, 21 Oct 2026 (the older `@mariozechner/...` name stopped
at 0.73.1). Node 22.19+. Headless `pi --print "<prompt>"` or `--mode json`. DeepSeek native,
`DEEPSEEK_API_KEY` (package `docs/providers.md:36`). Proxy: `HTTP_PROXY`, `HTTPS_PROXY` honoured, and an
`httpProxy` setting that only agent-directory settings can set (`docs/environment-variables.md:99`,
`docs/settings.md:125`); its undici dispatcher is HTTP/1.1 only (CHANGELOG). CA: `NODE_EXTRA_CA_CERTS`
(Node standard, unverified). A `transport` setting can pick WebSocket for providers that offer it; DeepSeek
uses SSE. Sessions: JSONL tree, `~/.pi/agent/sessions/--<path>--/<timestamp>_<id>.jsonl`
(`docs/session-format.md`); `auth.json` holds keys. Updates: `PI_SKIP_VERSION_CHECK`, `PI_OFFLINE`,
`PI_TELEMETRY=0`. **Subagents: no.** README: "skips features like sub-agents". The example extension runs
each subagent as a separate `pi --mode json -p --no-session` process (`examples/extensions/subagent/index.ts:298-344`),
so children write no session file; installing it is forcing subagents by config. Use Pi only if the owner
accepts a harness without subagents.

**8 to 14, short.**
- CodeBuddy Code (Tencent) 2.178.1, 29 Oct: Claude Code style; DeepSeek through `~/.codebuddy/models.json`
  with `url` and `apiKey: "${DEEPSEEK_API_KEY}"`, but you must "Install and sign in" first
  (https://api-docs.deepseek.com/quick_start/agent_integrations/workbuddy). Reads `NODE_EXTRA_CA_CERTS`,
  `DISABLE_AUTOUPDATER` (source). Starts "prewarm" helper processes over Unix sockets (source `bin/`): a
  daemon risk. Needs own account.
- Qoder CLI 1.1.81, 29 Oct: sign-in required; DeepSeek only through the interactive `/model` Custom
  wizard, "Do not configure them manually in settings.json"
  (https://api-docs.deepseek.com/quick_start/agent_integrations/qoder). Not pad-friendly.
- Reasonix 2.54.0, 28 Oct: a Go 1.26.10 binary (source), DeepSeek only "on purpose"; `reasonix run "<task>"`;
  subagents via skills with `runAs: subagent`; config `~/.reasonix/config.json`
  (https://github.com/esengine/DeepSeek-Reasonix README). Go reads `HTTPS_PROXY`; CA by `SSL_CERT_FILE`,
  which replaces the system roots on Linux, so pass a bundle of system roots plus the run CA (unverified).
- Plandex cli/v2.2.1, 16 Jul (https://github.com/plandex-ai/plandex/releases): the CLI talks to a Plandex
  server, which makes the model calls. A local proxy on the CLI records nothing useful. No subagents.
- Warp agent CLI: needs `WARP_API_KEY`; BYOK only for three named vendors, no DeepSeek; a 2026 issue says
  BYOK keys could not be set on headless machines (https://docs.warp.dev/support-and-community/plans-and-billing/bring-your-own-api-key,
  https://upd.dev/warpdotdev/warp/issues/8937; unverified whether fixed).
- Continue CLI (`cn`) 1.5.47, 18 Jun 2026; no npm release since; one site says the repository went
  read-only after a final June release (https://openapps.pro/apps/continue, unverified).
- Roo Code: "Roo Code Extension was shut down on May 15th"; CLI repo archived 15 May 2026
  (https://roocode.com/blog/sunsetting-roo-code-extension-cloud-and-router;
  https://github.com/RooCodeInc/Roo-Code/releases/tag/cli-v0.1.17). Dead.

### O.6.3 Which backup for which failure

| If this one of the 15 fails | Swap in | Why |
|---|---|---|
| A Node harness with a JSONL session (OpenCode, Kilo, Qwen Code) | Kimi Code | Same runtime family, JSONL per subagent |
| A Python harness (Aider, Hermes) | Mistral Vibe | Python, `SSL_CERT_FILE` adds roots, subagents |
| A harness with a daemon or SQLite store (Goose, Crush, OpenCode 2) | Cline CLI with local backend | Tests the "daemon off" path |
| Any DeepSeek-native slot | DeepSeek Harness | Made for this provider; but preview |
| A child-process subagent model (Claude Code style) | Letta Code local | Children inherit env; wire is the only child record |
| An account-only harness (Amp, Cursor, Gemini CLI) | none of these | The swap does not test the same thing; keep it "not verified" |

## O.7 Per-harness details

Each section keeps the full facts, links and versions from the research parts. Sub-sections:
.1 version, install, headless; .2 keys and DeepSeek; .3 proxy and CA; .4 subagents; .5 session
files; .6 auto-update; .7 verdict and test-pad recipe.

### H1. Claude Code (test key: a Claude API key)

#### H1.1 Version, install, headless

| Item | Fact | Source |
|---|---|---|
| Latest | 2.1.312, 27 Oct 2026 18:36 UTC (dist-tags `latest`, `next`). Stable channel 2.1.308 (22 Oct). | npm registry above |
| Cadence | 26 releases in the 30 days to 27 Oct. About one a weekday. Expect about 6 more before 7 Nov. | npm `time` field |
| Install on Linux | `curl -fsSL https://claude.ai/install.sh \| bash`, or pin: `... \| bash -s 2.1.312`. npm `@anthropic-ai/claude-code` also works and pulls the same native binary. | https://code.claude.com/docs/en/setup |
| Headless | `claude -p "<prompt>"` (print mode), with `--permission-mode bypassPermissions` or `--dangerously-skip-permissions` for an unattended run. As root it refuses bypass mode unless `IS_SANDBOX=1` (`docs/harnesses/claude-code-cli.md:149`). Interactive chat is how people use it (point 7). | https://code.claude.com/docs/en/headless |

#### H1.2 Keys

- Bring your own key: yes. `ANTHROPIC_API_KEY` in the environment. In interactive mode Claude Code
  asks once whether to use that key; `-p` uses it at once. `apiKeyHelper` in settings is another way.
  Source: https://code.claude.com/docs/en/authentication (precedence list).
- DeepSeek: works through DeepSeek's Anthropic-format endpoint, `ANTHROPIC_BASE_URL=https://api.deepseek.com/anthropic`
  plus the DeepSeek key as `ANTHROPIC_API_KEY` (or `ANTHROPIC_AUTH_TOKEN`). DeepSeek ignores some
  fields (tool `cache_control`, `is_error` on tool results, MCP blocks). Source:
  https://api-docs.deepseek.com/guides/anthropic_api (fetched 29 Oct 2026). Not needed for the test plan.
- Test key: the owner's Claude API key, in `ANTHROPIC_API_KEY`. No base URL. Model host is
  `api.anthropic.com`.

#### H1.3 Proxy and CA

| Question | Answer | Source |
|---|---|---|
| Runtime | One native executable built on Bun ("bundled Bun runtime 1.4", changelog 2.1.181). Not Node, so `NODE_USE_ENV_PROXY` does nothing. | https://raw.githubusercontent.com/anthropics/claude-code/main/CHANGELOG.md |
| `HTTPS_PROXY` | Honoured for model calls and its own other requests. Reads the first of `https_proxy`, `HTTPS_PROXY`, `http_proxy`, `HTTP_PROXY`. `NO_PROXY` honoured (since 2.1.292 also for its own non-model requests). No SOCKS. An `https://` proxy URL is reported broken (third-party mirror, unverified); use `http://127.0.0.1:<port>`. | https://code.claude.com/docs/en/network-config ; https://claudeissues.com/issue/71796-bug-support-https-proxy-https-scheme-for-https-proxy-environment-variable |
| Custom CA | `NODE_EXTRA_CA_CERTS=<run CA>` adds to the trust set. Default trust is `bundled,system` (`CLAUDE_CODE_CERT_STORE`). `SSL_CERT_FILE` is not documented for its own requests. | network-config page; changelog 2.1.101 |
| HTTP/2 | Model calls measured as HTTP/1.1 SSE (`docs/harnesses/claude-code-cli.md:72`). Some background paths can speak h2 (changelog 2.1.208); the proxy picks ALPN, so it can offer only http/1.1. | changelog 2.1.208 |
| WebSockets | Only Claude in Chrome bridge and loopback; loopback WebSockets never use the proxy. | network-config page |
| Pinning | None documented, except the Claude apps gateway fingerprint on first sign-in. Changelog 2.1.307 says the CLI "filters its trusted certificates" by an undocumented rule; a name-constrained run CA still worked on 2.1.312 (measured, `docs/research/results/2026-10-vercel-gateway.md:53`). | changelog 2.1.307 |
| Paths that skip the proxy | (a) A settings `env` block (`~/.claude/settings.json` or project `.claude/settings.json`) can set `HTTPS_PROXY`, `NO_PROXY` or `CLAUDE_CODE_CERT_STORE`; whether it beats the launch env is unverified. (b) Bedrock, Vertex, Foundry, Mantle and the Claude apps gateway; the gateway refuses `HTTPS_PROXY`. (c) The agent-view supervisor (`claude agents`, `--bg`, `/background`) "inherits the environment of whichever shell starts it first" and outlives the shell. (d) Split-pane agent teams start new instances under tmux. Telemetry goes to `api.anthropic.com` and Datadog through the same proxy (off with `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`). | network-config, authentication, agent-view (https://code.claude.com/docs/en/agent-view), agent-teams pages |
| Tracker | Past regressions: native build ignored `HTTP_PROXY` for streaming (#14165, closed Feb 2026); `NO_PROXY` ignored on 2.1.83 Linux (#39862, closed Apr 2026); intermittent bypass on Windows (#15212, closed Feb 2026). No open October 2026 proxy issue found by web search (search only, unverified). | https://claudeissues.com/issue/14165-bug-native-binary-installation-ignores-http-proxy-for-api-streaming-requests-bun ; https://claudeissues.com/issue/39862-bug-no-proxy-completely-ignored-claude-code-v2-1-83-routes-all-traffic-through-f ; https://github.com/anthropics/claude-code/issues/15212 |

#### H1.4 Subagents

- Yes. The Agent tool starts a subagent in the same process (it "starts in the background" by
  default). 20 at once per session (`CLAUDE_CODE_MAX_CONCURRENT_SUBAGENTS`), nesting 3 deep, "no limit
  on the total number". Same process, so it shares the env. Source: https://code.claude.com/docs/en/sub-agents
- Workflows (`ultracode`, `/deep-research`): up to 1,000 agents per run, 16 at once by default. Docs
  speak of "its agents' processes"; whether they are separate OS processes and inherit `HTTPS_PROXY`
  is unverified. Source: https://code.claude.com/docs/en/workflows
- Prompt: "Use one subagent (the Agent tool) to list every file in this repo and report the count;
  then summarise its answer." Plain words like "use a subagent" are enough; do not force it by config.

#### H1.5 Session files

- Main transcript: `~/.claude/projects/<cwd with non-alphanumerics as ->/<session-id>.jsonl` (or under
  `CLAUDE_CONFIG_DIR`). JSONL, internal format, changes per release. Since 2.1.300 each request's system
  prompt, tools and parameters are written too. Source: https://code.claude.com/docs/en/sessions ; changelog 2.1.300.
- Subagents: `~/.claude/projects/<p>/<session-id>/subagents/agent-<id>.jsonl` plus `.meta.json`
  (docs now say so); workflow agents under `.../subagents/workflows/<id>/` (measured,
  `connectors/claude-code-cli/harness.toml:104-105`). Spilled tool output in `tool-results/`.
- Keys: `~/.claude/.credentials.json` (mode 0600) holds OAuth tokens on Linux. An `ANTHROPIC_API_KEY`
  from env is not written to a transcript by design, but `~/.claude.json` keeps an approval record of
  it (form unverified).

#### H1.6 Auto-update

Native installs check at start and while running, and install for the next start. `DISABLE_AUTOUPDATER=1`
stops the background check; `DISABLE_UPDATES=1` blocks every path, including `claude update` run by the
agent. `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1` also stops it. npm installs do not self-update the
same way. Source: https://code.claude.com/docs/en/setup ; https://code.claude.com/docs/en/env-vars

#### H1.7 Verdict and test-pad recipe

Proxy-ready: **yes** (measured on 2.1.312 through a gateway; direct `api.anthropic.com` with an API
key through the new proxy is unverified). DeepSeek-ready: **yes** (Anthropic-format endpoint), not used.
Subagents: **yes**.

Recipe. Install as a non-root user: `curl -fsSL https://claude.ai/install.sh | bash -s 2.1.312`. Write
`~/.claude/settings.json` with nothing but `{"permissions":{"defaultMode":"bypassPermissions"}}` (no
`env` block). goalcup passes: `ANTHROPIC_API_KEY` (from the pad's secret store, never logged),
`HTTPS_PROXY=http://127.0.0.1:<port>`, `HTTP_PROXY` the same, `NO_PROXY=127.0.0.1,localhost`,
`NODE_EXTRA_CA_CERTS=<run CA pem>`, `DISABLE_AUTOUPDATER=1`, `DISABLE_UPDATES=1`,
`CLAUDE_CODE_DISABLE_AGENT_VIEW=1`. Run `claude` in a scratch git repo and type: "Use one subagent to
count the files in this repo, then tell me its answer." Check: (1) every `POST /v1/messages` to
`api.anthropic.com` is in the proxy record with request and SSE body; (2) at least one request carries
the subagent's prompt; (3) `projects/<p>/<session>/subagents/agent-*.jsonl` exists and its turn count
matches the wire; (4) no CONNECT to a model host was left as an opaque tunnel; (5) the key appears
nowhere in the record (the `x-api-key` header masked). Unverified: whether the key prompt needs a
pre-seeded approval in `~/.claude.json` for interactive mode (try `-p` first, then interactive).

### H2. Codex CLI (test key: an OpenAI API key)

#### H2.1 Version, install, headless

| Item | Fact | Source |
|---|---|---|
| Latest | 0.165.0, tag 27 Oct 2026 (npm `latest`, binaries published 27 Oct 20:16 UTC). Alpha 0.166.0-alpha.6 on 28 Oct. | npm registry above ; https://github.com/openai/codex/releases/tag/rust-v0.165.0 |
| Cadence | About one minor release a week plus hotfixes; 11 stable tags in the 30 days to 27 Oct. 0.166.0 expected around 3 Nov (estimate). | appendix D2.1 |
| Install on Linux | `npm i -g @openai/codex@0.165.0` (Node wrapper that spawns the native Rust binary), or the standalone script `curl -fsSL https://chatgpt.com/codex/install.sh \| sh`, or the release binary. | https://github.com/openai/codex/blob/rust-v0.165.0/README.md |
| Headless | `codex exec "<prompt>"`. Read-only sandbox by default; `--sandbox workspace-write` or `--sandbox danger-full-access` (container only); `--full-auto` is gone; `--skip-git-repo-check`; `--json` for JSONL events. | https://learn.chatgpt.com/docs/non-interactive-mode (fetched 29 Oct 2026) |

#### H2.2 Keys

- Bring your own key: yes. Interactive: `printenv OPENAI_API_KEY | codex login --with-api-key` stores
  it in `$CODEX_HOME/auth.json` (file store by default). Headless: `CODEX_API_KEY=<key> codex exec ...`
  for one run, no file written. Source: https://learn.chatgpt.com/docs/auth.md ; non-interactive page
  above; `codex-rs/login/src/auth/manager.rs` (`CODEX_API_KEY_ENV_VAR`, read only where
  `enable_codex_api_key_env` is true, as in `exec`), https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/login/src/auth/manager.rs#L988-L1000
- DeepSeek: **no, not directly.** Codex 0.165 speaks only the Responses API; `wire_api = "chat"` now
  stops Codex with "no longer supported"
  (https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/model-provider-info/src/lib.rs#L105).
  DeepSeek lists only Chat Completions and Anthropic formats (appendix E3.1,
  https://api-docs.deepseek.com/). A translating gateway would be needed (unverified that DeepSeek has
  no Responses endpoint).
- **Correction (part O6, section O.8.4):** DeepSeek now has a Responses API (https://api-docs.deepseek.com/guides/responses_api)
  and a Codex guide (https://api-docs.deepseek.com/quick_start/agent_integrations/codex , fetched 29 Oct 2026),
  so Codex can run on DeepSeek. The test still uses the OpenAI key.
- Test key: the owner's OpenAI API key. Model host `api.openai.com`, path `/v1/responses`.

#### H2.3 Proxy and CA

| Question | Answer | Source |
|---|---|---|
| Runtime | Rust, rustls, reqwest and tokio-tungstenite. The npm `codex` is a Node wrapper with no network code. | https://github.com/openai/codex/blob/rust-v0.165.0/codex-cli/bin/codex.js#L231-L243 |
| `HTTPS_PROXY` | Honoured by reqwest clients (HTTPS fallback, auth, model list) and by the Responses WebSocket dialer (reads `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`, `NO_PROXY`). | https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/websocket-client/src/dialer.rs#L51-L60 |
| Custom CA | `CODEX_CA_CERTIFICATE=<pem>`, else `SSL_CERT_FILE`. Added on top of the native roots, for HTTPS and WebSocket alike. | https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/http-client/src/custom_ca.rs#L14 , `#L63-L64`, `#L261-L283` |
| WebSockets | Yes, and this is the main risk. With an API key the built-in `openai` provider still has `supports_websockets: true`: Codex first opens `wss://api.openai.com/v1/responses` (beta header `responses_websockets=2026-02-06`), then falls back to HTTPS POST plus SSE for the rest of the session if that fails. The proxy must record WebSocket frames (repo has `crates/goalcup-cli/src/proxy/websocket.rs`; WebSocket relay measured on 0.153.0, `docs/harnesses/codex-cli.md:155`; dialer code changed since, appendix D2.1). | https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/model-provider-info/src/lib.rs#L531-L566 ; `core/src/client.rs#L176`, `#L262-L263` |
| HTTP/2 | reqwest offers h2 and http/1.1; falls back to http/1.1 when the proxy offers only that. Only a local gRPC socket uses h2 prior knowledge. | appendix D2.2 |
| Pinning | None found (no `danger_accept_invalid`, no disabled roots). | appendix D2.2 |
| Paths that skip the proxy | (a) `$CODEX_HOME/.env` is loaded at start and overwrites every non-`CODEX_` key, so `HTTPS_PROXY`, `NO_PROXY` or `SSL_CERT_FILE` there beat goalcup's env (`CODEX_CA_CERTIFICATE` cannot be overridden). (b) A shared app-server daemon under `CODEX_HOME` makes calls with its own env; `--no-daemon` forces the embedded server. (c) `system_proxy_fallback` retries only login and cloud bootstrap GETs via the system proxy, not model calls. (d) `codex tcp-tunnel` (QUIC) and realtime WebRTC voice are opt-in. Telemetry and `ab.chatgpt.com` go through the proxy as normal HTTPS. | https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/arg0/src/lib.rs#L270-L296 ; `tui/src/daemon_startup.rs#L60-L125` |
| Tracker | Open WebSocket issues: #22156 "websocket closed by server before response.completed" (mirror, state unverified); #13041 WebSocket problem without proxy vars. No Codex CA or proxy regression found for October 2026 (search only). | https://upd.dev/openai/codex/issues/22156 ; https://upd.dev/openai/codex/issues/13041 |

#### H2.4 Subagents

- Yes. Tools `spawn_agent`, `send_input`, `wait_agent`, `close_agent` and others; `multi_agent` is
  stable and on. 6 at once per session (`agents.max_concurrent_threads_per_session`), depth 1, no total
  cap. Children are threads inside the same Codex process (`ThreadManager`; no `Command::new` in
  `codex-rs/core/src/agent/`), so they share the env. Since 0.164.0 a forked child copies the parent
  history. Source: https://learn.chatgpt.com/docs/agent-configuration/subagents.md ;
  https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/core/src/config/mod.rs#L258
- Prompt: "Spawn one sub-agent to count the files in this repo, wait for it, then report its answer."

#### H2.5 Session files

- `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<local time>-<thread id>.jsonl`, one per thread, one per
  child. JSONL `{timestamp, type, payload}`. Side files: `session_index.jsonl`, `history.jsonl`, a state
  SQLite (parent and child edges). Source: https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/rollout/src/recorder.rs#L1770-L1792
- Keys: `$CODEX_HOME/auth.json` holds the API key in plain text after `login --with-api-key` (or
  tokens after a ChatGPT sign-in). `$CODEX_HOME/.env` may hold keys. `codex exec` with `CODEX_API_KEY`
  writes no key file.

#### H2.6 Auto-update

Codex does not install updates by itself. The TUI checks at start and offers an upgrade at a prompt;
`check_for_update_on_startup = false` in `config.toml` (or `-c check_for_update_on_startup=false`)
turns the check off. Homebrew and npm update only when asked. Source:
https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/tui/src/updates.rs#L27-L30 ;
`codex-rs/core/src/config/mod.rs:4271` (default true). Whether `codex exec` runs the check: unverified.

#### H2.7 Verdict and test-pad recipe

Proxy-ready: **likely** (HTTPS path and CA are clear in source; the WebSocket path through the new
proxy on 0.165 is unverified). DeepSeek-ready: **yes since DeepSeek added a Responses API** (H2.2 correction; not used). Subagents: **yes**.

Recipe. `npm i -g @openai/codex@0.165.0` in a container. `CODEX_HOME=<pad dir>` with a `config.toml`
holding only `check_for_update_on_startup = false` and no `.env` file. Log in once:
`printenv OPENAI_API_KEY | codex login --with-api-key` (or use `CODEX_API_KEY` for `exec`). goalcup
passes: `HTTPS_PROXY=http://127.0.0.1:<port>`, `HTTP_PROXY` the same, `NO_PROXY=127.0.0.1,localhost`,
`CODEX_CA_CERTIFICATE=<run CA pem>`, `CODEX_HOME`. Launch `codex --no-daemon -C <scratch repo>` and
type: "Spawn one sub-agent to count the files in this repo, wait for it, then report its answer." Check:
(1) the record holds a `wss://api.openai.com/v1/responses` upgrade with `response.create` frames, or,
if the upgrade failed, `POST /v1/responses` SSE calls (both count; note which); (2) frames for the
child thread are present; (3) two rollout files exist under `sessions/` and the child's turn matches the
wire; (4) no tunnel to `api.openai.com` stayed opaque; (5) the key is masked in headers and absent
from `auth.json` copies in the upload. Repeat once with `codex exec --json` to see the headless shape.

### H3. Hermes Agent (test key: DeepSeek)

#### H3.1 Version, install, headless

| Item | Fact | Source |
|---|---|---|
| Latest | v0.22.5 (tag 22 Oct, release 22/23 Oct 2026). PyPI stops at 0.19.0 (20 Jul 2026). Daily canary, latest `v0.22.5+canary.20261029T064840Z`. v0.23.0 announced, no date. | https://github.com/NousResearch/hermes-agent/releases/tag/v0.22.5 ; https://pypi.org/pypi/hermes-agent/json |
| Cadence | 8 stable releases 8 to 23 Oct; none since 22 Oct. Main moves hundreds of commits a day. | appendix D3.2 |
| Install on Linux | `curl -fsSL https://hermes-agent.nousresearch.com/install.sh \| bash -s -- --skip-setup --commit <sha>` (default `--channel stable`; `--skip-setup` = non-interactive; with a TTY and no skip it runs `hermes setup` and offers a background gateway service). Python 3.14 via uv. | https://github.com/NousResearch/hermes-agent/blob/v0.22.5/scripts/install.sh#L48-L94 |
| Headless | `hermes chat --oneshot -q "<prompt>"` (or `-Q`, or non-TTY stdio); `--yolo` skips approvals; `--format stream-json` for JSONL. One-shot runs cap delegation at 2 children (`delegation.oneshot_max_children`). | https://github.com/NousResearch/hermes-agent/blob/v0.22.5/website/docs/reference/cli-commands.md (lines 126-140) |

#### H3.2 Keys

- Bring your own key: yes, about 40 built-in providers.
- DeepSeek: **native provider** `deepseek`. `DEEPSEEK_API_KEY` in `$HERMES_HOME/.env`, default base
  `https://api.deepseek.com/v1` (override `DEEPSEEK_BASE_URL`); select with `hermes model` or
  `config.yaml` `model: {provider: deepseek, default: <DeepSeek model id>}`. DeepSeek's own guide uses
  `hermes setup`, Quick Setup, provider DeepSeek, base `https://api.deepseek.com`. Sources:
  https://github.com/NousResearch/hermes-agent/blob/v0.22.5/hermes_cli/auth.py#L222 ;
  `website/docs/integrations/providers.md:48` at v0.22.5; https://api-docs.deepseek.com/quick_start/agent_integrations/hermes (fetched 29 Oct 2026).
- Test key: DeepSeek. Model host `api.deepseek.com`, OpenAI Chat Completions format.

#### H3.3 Proxy and CA

| Question | Answer | Source |
|---|---|---|
| Runtime | Python (3.11 to 3.14; installer uses 3.14). httpx and the OpenAI SDK for model calls. The Ink TUI and desktop app are clients of the Python gateway. | `pyproject.toml` at v0.22.5 |
| Process model | `hermes chat` is a thin client; every model call runs in a gateway process, one per `HERMES_HOME`. A gateway started by the chat copies the chat's whole env, so `HTTPS_PROXY` reaches it. A gateway that was already running keeps its own env. The chat-to-gateway WebSocket is local and opened with `proxy=None`. | https://github.com/NousResearch/hermes-agent/blob/v0.22.5/hermes_cli/gateway_runtime_start.py#L35-L57 ; `hermes_cli/gateway_client.py#L298-L305` |
| `HTTPS_PROXY` | Honoured for model calls (httpx env proxies; one NO_PROXY matcher). A malformed proxy URL stops the turn with a clear error. Measured on 0.19.0 and 0.22.5. | https://github.com/NousResearch/hermes-agent/blob/v0.22.5/agent/proxy_bypass.py#L1-L26 ; `agent/auxiliary_client.py#L2865-L2881` |
| Custom CA | Since 0.21.6 trust is the OS store through `truststore` 0.10.4, patched into `ssl.SSLContext` process-wide. `HERMES_CA_BUNDLE` no longer touches model calls. Without a provider `ssl_ca_cert` (goalcup no longer writes one), the model client gets a platform context. On Linux, truststore calls OpenSSL `set_default_verify_paths()`, which reads `SSL_CERT_FILE` and `SSL_CERT_DIR`. So on Linux, `SSL_CERT_FILE` **replaces** the default CA file for model calls. Hermes's own `platform_ssl_context()` adds it on top, but that path serves only the Nous sign-in. Rule for goalcup: pass `SSL_CERT_FILE` pointing at a bundle of the system roots plus the run CA, never the run CA alone. Installing the run CA in the OS store (`update-ca-certificates`) also works. Both unverified by a run on 0.22.5. macOS: `SSL_CERT_FILE` is not used by the Security framework; needs the keychain (out of scope, Linux first). | https://github.com/NousResearch/hermes-agent/blob/v0.22.5/agent/ssl_verify.py#L1-L20 , `#L117-L128`, `#L300-L343` ; https://raw.githubusercontent.com/sethmlarson/truststore/v0.10.4/src/truststore/_openssl.py (`_configure_context`) |
| HTTP/2, WebSockets, gRPC | No `http2` in agent code; model calls are HTTP/1.1 (measured). No model call over WebSocket. No gRPC. No pinning. | appendix D3.3.3 |
| Paths that skip the proxy | (a) A gateway already running for the same `HERMES_HOME` (started earlier, by cron, kanban, or a `hermes gateway install` service) makes calls with its own env: direct, not even tunnelled. (b) `copilot-acp` runs GitHub's Copilot CLI as a child with its own stack. (c) Anthropic, Gemini-native, Bedrock and Vertex adapters do not read `ssl_ca_cert` (not relevant with `SSL_CERT_FILE`, which is process-wide). (d) Optional iron-proxy and Bitwarden helpers, off by default. Non-model hosts seen: `models.dev`, `api.github.com`, `github.com`, `portal.nousresearch.com`. | appendix D3.3 ; `hermes_cli/auth.py:187` |
| Tracker | Search found a proxy-format PR (`socks://` to `socks5://`, mirror) and no open CA or proxy bug for October 2026 (search only, unverified). | https://github.yezi957.workers.dev/NousResearch/hermes-agent/pull/13465 (mirror) |

#### H3.4 Subagents

- Yes: `delegate_task`. Children run **inside the gateway process** as in-process agents, each with its
  own session row in `state.db` (`parent_session_id`). In an interactive chat, top-level delegations
  run in the background and report back between turns. Defaults: 10 at once, depth 1, 250 iterations,
  no wall-clock cap; 2 children max in one-shot mode. Same process, same env.
  Source: https://github.com/NousResearch/hermes-agent/blob/v0.22.5/hermes_cli/config_defaults.py#L1342-L1404 ;
  `tools/delegate_tool.py#L696-L713` ; https://hermes-agent.nousresearch.com/docs/user-guide/features/delegation
- Delegation on 0.22.5 has not been measured. 0.19.0 failed twice on 29 Oct ("internal executor
  error", `docs/plans/2026-10-29-record-everything-progress.md:74`) but worked on a Mac on 22 Oct.
- Prompt: "Delegate one subtask to a subagent: count the files in this repo. Wait for its result, then
  report it."

#### H3.5 Session files

- `$HERMES_HOME/state.db` (SQLite, WAL, schema 32 at v0.22.5): sessions, messages, usage,
  `async_delegations`. The gateway is the only writer. Shared by every session ever run in that home.
  Source: https://github.com/NousResearch/hermes-agent/blob/v0.22.5/website/docs/developer-guide/session-storage.md
- Subagent live logs: `$HERMES_HOME/cache/delegation/live/<id>/task-<n>.log` plus `manifest.json`,
  pruned after 7 days, redacted by Hermes before writing (`tools/delegation_live_log.py:25`, `:71-80`).
- Keys: `$HERMES_HOME/.env` (`DEEPSEEK_API_KEY` in plain text), `auth.json`, `.anthropic_oauth.json`,
  `mcp-tokens/`. Hermes can also borrow `~/.codex/auth.json` and `~/.claude/.credentials.json` for
  those providers. Whether `state.db` message rows are redacted: unverified.
  Source: `website/docs/user-guide/security.md:373`, `:680` at v0.22.5.

#### H3.6 Auto-update

Core does not update itself. A daily background check runs at chat start; `updates.check: false` in
`config.yaml` turns it off. Plugins: check every 24 h (`plugins.auto_update_check_hours: 0` turns it
off), apply off by default. Pin with the installer's `--commit <sha>` and never run `hermes update`.
Source: https://github.com/NousResearch/hermes-agent/blob/v0.22.5/hermes_cli/config_defaults.py#L2359-L2361 , `#L1738-L1744`.

#### H3.7 Verdict and test-pad recipe

Proxy-ready: **likely** (proxy honoured, measured; CA trust without the `recorder` entry depends on
`SSL_CERT_FILE` with system roots plus run CA, unverified). DeepSeek-ready: **yes** (native provider).
Subagents: **yes** (unmeasured on 0.22.5).

Recipe. Install as a non-root user: `curl -fsSL https://hermes-agent.nousresearch.com/install.sh | bash -s -- --skip-setup --commit bf0203a794`
(v0.22.5). Use a fresh `HERMES_HOME=<pad dir>`. Write `$HERMES_HOME/.env` with `DEEPSEEK_API_KEY=...`
and `config.yaml` with `model: {provider: deepseek, default: <DeepSeek model id>}`,
`approvals: {mode: off}`, `updates: {check: false}`. Run `hermes gateway stop -y` first so no old
gateway serves this home. Build `run-bundle.pem` = system bundle (`/etc/ssl/certs/ca-certificates.crt`)
plus the run CA. goalcup passes: `HERMES_HOME`, `HTTPS_PROXY=http://127.0.0.1:<port>`, `HTTP_PROXY`
the same, `NO_PROXY=127.0.0.1,localhost`, `SSL_CERT_FILE=<run-bundle.pem>`. Launch `hermes chat` in a
scratch repo and type: "Delegate one subtask to a subagent: count the files in this repo. Wait for its
result, then report it." Check: (1) every `POST /v1/chat/completions` to `api.deepseek.com` is in the
record (no TLS error in `hermes logs`); (2) a request with the child's prompt exists; (3) `state.db`
has a child session with `parent_session_id` set; (4) `ps` shows the gateway's env has `HTTPS_PROXY`
(`/proc/<pid>/environ`, check the key name only); (5) the DeepSeek key is masked; (6) after exit,
`hermes gateway stop -y` and confirm no Hermes process is left. Second test: `SSL_CERT_FILE` with the
run CA alone, to learn whether non-model hosts (`models.dev`, `github.com`) then fail.

### H4. OpenCode (1 and 2)

Two programs share the `opencode` command. OpenCode 1 (`opencode-ai`) is what `curl -fsSL
https://opencode.ai/install | bash` installs and has by far the most downloads (D-harnesses.md:1212). OpenCode 2
(`@opencode/cli`) is installed with `curl -fsSL https://opencode.ai/v2/install | bash` or
`npm install -g @opencode/cli` (https://opencode.ai/v2/docs, fetched 29 Oct 2026). Test 1 first; run 2 once.

"Facts sheet" below means `docs/harnesses/opencode-cli.md` on branch `origin/opencode-openclaw` (measured
28 Oct 2026; read with `git show origin/opencode-openclaw:docs/harnesses/opencode-cli.md`).

#### H4.1 Version, install, headless
- OpenCode 1: `1.19.5` (28 Oct); releases 30 Sep, 6 Oct, 11 Oct, 15 Oct, 16 Oct (two), 22 Oct, 28 Oct
  (https://registry.npmjs.org/opencode-ai, fetched 29 Oct 2026). About one a week.
- OpenCode 2: `2.2.2` (29 Oct 18:40 UTC); 22 stable releases since 29 Sep, often two a day
  (https://registry.npmjs.org/@opencode%2fcli, fetched 29 Oct 2026). Expect several during a run.
- Chat: `opencode`. Headless: `opencode run "<prompt>"` (facts sheet section 1).

#### H4.2 Keys
- BYOK: yes. DeepSeek is a native provider (`/connect`, search DeepSeek, paste key) (vendor docs,
  `packages/web/src/content/docs/providers.mdx:736-753` at `dev`). Keys go to
  `~/.local/share/opencode/auth.json` as `{"deepseek": {"type": "api", "key": "..."}}`, or come from the env
  (`DEEPSEEK_API_KEY`) or `provider.deepseek.options.apiKey` in config (facts sheet section 5).
- Wire: `POST https://api.deepseek.com/chat/completions` (measured in a run 28 Oct, facts sheet section 6).
- Config: `~/.config/opencode/opencode.json` with `"model"`, `"small_model"` (set it, or titles use a paid
  gateway model) and `"autoupdate": false`.
- Trap: an unknown model id silently fell back to a paid gateway model (measured in a run 28 Oct). Use an id that
  `opencode models deepseek` lists.
- Test key: DeepSeek.

#### H4.3 Proxy
- Runtime: Bun-compiled binary in both versions (OpenCode 1 measured; OpenCode 2 source:
  `packages/cli/script/build.ts:160-178` uses `Bun.build` with `compile`; `bin/opencode.cjs` is a Node launcher
  that spawns the native binary and passes the env).
- `HTTPS_PROXY`: honoured on its own (measured 28 Oct, OpenCode 1). Vendor docs: "OpenCode respects standard
  proxy environment variables" and **`NO_PROXY=localhost,127.0.0.1` is "required"** because "The TUI
  communicates with a local HTTP server" (https://opencode.ai/docs/network, fetched 29 Oct 2026). goalcup must
  pass it, or the TUI's own server traffic loops through the recorder.
- CA: `NODE_EXTRA_CA_CERTS` (adds to roots; "works for both proxy connections and direct API access", same page).
  `SSL_CERT_FILE` alone also worked (measured 28 Oct).
- Other hosts: `models.opencode.ai` (catalog), `registry.npmjs.org` (provider packages at first use), off with
  `OPENCODE_DISABLE_MODELS_FETCH` (facts sheet section 6). No gRPC or WebSocket for DeepSeek. HTTP/1.1 through
  the proxy (measured).
- **OpenCode 2 bypass risk (source):** by default the CLI resolves a shared background service
  (`packages/cli/src/services/server-connection.ts:47-56`), registered at `$XDG_STATE_HOME/opencode/service.json`
  (`service-config.ts:41-44`, `:120-131`) on port `0xC0DE` (`:46-56`). A service started earlier, outside the
  run, keeps its own env, so model calls skip the run's proxy. A service started by the run may outlive it and
  serve the next run with a dead proxy address. `--standalone` "Run[s] with a private server instead of the
  background service" (`packages/cli/src/commands/commands.ts:13-17`), spawned as a child with `extendEnv: true`
  (`packages/cli/src/services/standalone.ts:17-31`), so it inherits `HTTPS_PROXY`. Unverified by run.
- Known tracker issues: none found that name `HTTPS_PROXY` (search 29 Oct 2026). A third-party page points to
  connection issues #5344, #9788, #12083 (https://www.developersdigest.tech/errors/opencode/unable-to-connect-url),
  unverified.

#### H4.4 Subagents
- Built-in subagents General and Explore; the model calls them through the Task tool, or the user writes
  `@general ...` (https://opencode.ai/docs/agents, fetched 29 Oct 2026). Subagents use the parent's model unless set.
- In-process: OpenCode 1 `task.ts` creates a child session with `parentID` and runs `ops.prompt(...)` in the same
  process (`packages/opencode/src/tool/task.ts:139-233` at `v1.19.5`). It also has `background=true`
  ("launches the subagent asynchronously", `:25-38`). Same process, so same env and same proxy. Tool shells are
  child processes and inherit the env (facts sheet section 7).
- Prompt: "Use the general subagent (the Task tool) once to write the tests, then run them."
- Subagents through goalcup: **unverified** (facts sheet section 7 says unmeasured).

#### H4.5 Session files
- One SQLite file for all sessions: `~/.local/share/opencode/opencode.db` (tables `session`, `message`, `part`,
  `event`, `credential` and more); child sessions in the same file (measured, facts sheet section 4).
  `opencode export <id>` prints JSON.
- Keys: `auth.json` (plain text) and the `credential` table. Logs: `~/.local/share/opencode/log/opencode.log`.
- OpenCode 2: same database path per a third-party write-up
  (https://capitalandcompute.net/blog/opencode-2-0-what-changed/, 14 Oct 2026), unverified.

#### H4.6 Auto-update
- OpenCode 1: never installs by itself since 1.19.0; it only notifies. Off: `"autoupdate": false` or
  `OPENCODE_DISABLE_AUTOUPDATE=1` (`packages/opencode/src/cli/upgrade.ts:7-24` at `v1.19.5`). The notice
  suggests OpenCode 2; `/update` or `opencode upgrade --major` moves a user to 2.
- OpenCode 2: default policy `"notify"` (`packages/cli/src/services/updater.ts:164-171`); `"update": "auto"`
  installs. Off: `OPENCODE_DISABLE_AUTOUPDATE=1`, `"update": "disable"` or `"autoupdate": false`
  (`updater.ts:122-137`, `:533-545`).

#### H4.7 Verdict and recipe
- Proxy-ready: **yes** (OpenCode 1); **likely, only with `--standalone`** (OpenCode 2).
- DeepSeek-ready: **yes**. Subagents: **yes**.
- Recipe (OpenCode 1): `curl -fsSL https://opencode.ai/install | bash -s -- --version 1.19.5`. Write
  `~/.config/opencode/opencode.json` with `model` and `small_model` set to a DeepSeek id from
  `opencode models deepseek`, `"autoupdate": false`, `"share": "disabled"`, `"permission": "allow"`; put the
  DeepSeek key in `~/.local/share/opencode/auth.json` (or `DEEPSEEK_API_KEY`). Env from goalcup: `HTTPS_PROXY`,
  `NODE_EXTRA_CA_CERTS=<run CA>`, `NO_PROXY=localhost,127.0.0.1`, `OPENCODE_DISABLE_AUTOUPDATE=1`. Start
  `opencode`, type: "Write chess.py with a legal-move generator. Use the general subagent once to write the
  tests, then run them." Check: every `POST api.deepseek.com/chat/completions` is recorded, including the
  subagent's (its own system prompt and task text); no other model host; `opencode.db` has a child session with
  a `parentID`; no Gateway call (title model). OpenCode 2: same, install `@opencode/cli@2.2.2`, start
  `opencode --standalone`, and before the run confirm no `service.json` exists under the run's state dir.
  What could go wrong: forgetting `NO_PROXY` loops the TUI's local traffic; a bad model id goes to a paid model;
  a participant on OpenCode 2 without `--standalone` may record nothing.

### H5. OpenClaw

"Facts sheet" below means `docs/harnesses/openclaw-cli.md` on branch `origin/opencode-openclaw` (measured
28 Oct 2026). Source read at `openclaw/openclaw` `cfb5923c` (30 Oct 2026 UTC), one commit set ahead of
`v2026.10.9`.

#### H5.1 Version, install, headless
- Latest `2026.10.9` (29 Oct, tag `v2026.10.9`); `extended-stable` `2026.9.35` (27 Oct); beta
  `2026.10.10-beta.4`. 17 releases since 29 Sep, about 2 to 3 a week
  (https://registry.npmjs.org/openclaw and https://github.com/openclaw/openclaw/releases, fetched 29 Oct 2026).
  2026.10.9 changes updates and Gateway recovery only; no proxy change listed.
- Install: `curl -fsSL https://openclaw.ai/install.sh | bash -s -- --no-onboard --no-prompt` (npm global
  package; needs Node `>=24.22.1 <25 || >=26.12.1`, npm `engines`). Measured 28 Oct.
- Chat: `openclaw chat` (= `openclaw tui --local`, embedded runtime, no Gateway). Plain `openclaw tui` attaches to
  a Gateway daemon. Headless: `openclaw agent --local --agent main --message "<prompt>"` (facts sheet section 1;
  `docs/cli/tui.md:40`, `:90-96`). Local mode refuses to start while a Gateway owns the state dir
  (`docs/cli/tui.md:92-96`).

#### H5.2 Keys
- BYOK: yes. DeepSeek plugin `@openclaw/deepseek-provider`, provider `deepseek`, "OpenAI-compatible", base
  `https://api.deepseek.com`, auth `DEEPSEEK_API_KEY` (`docs/providers/deepseek.md:9-24`). Setup:
  `openclaw plugins install @openclaw/deepseek-provider`, then `openclaw onboard --auth-choice deepseek-api-key`
  (interactive) or put `DEEPSEEK_API_KEY=` in `~/.openclaw/.env` (measured 28 Oct: the file works with no env).
  Avoid the non-interactive onboard form: it takes the key in argv.
- Model: `openclaw models set deepseek/<id>`; refresh with `openclaw models list --refresh --provider deepseek`.
  An unknown id was refused and the first session went to the gateway default (measured 28 Oct).
- Wire: `POST https://api.deepseek.com/chat/completions` (measured in a run 28 Oct). Headers include
  `HTTP-Referer: https://openclaw.ai`, `X-Title: OpenClaw`.
- Test key: DeepSeek.

#### H5.3 Proxy
- Runtime: Node (`#!/usr/bin/env node`, `openclaw.mjs`).
- `HTTPS_PROXY`: honoured (measured 28 Oct). Env-only routing honours `NO_PROXY` (`docs/security/network-proxy.md:112`).
  The connector keeps `NODE_USE_ENV_PROXY=1` (facts sheet section 6); D-harnesses.md:1300 says it is needed.
  Pass it anyway.
- CA: `NODE_EXTRA_CA_CERTS` (extends; `SSL_CERT_FILE` ignored, measured). The CLI re-execs itself to add a CA
  under nvm, but keeps one that is already set (`src/bootstrap/node-startup-env.ts:37-44`,
  test `node-extra-ca-certs.test.ts:34`).
- **Its own proxy wins:** `proxy.proxyUrl` (config) beats `OPENCLAW_PROXY_URL` (env or `~/.openclaw/.env`),
  and both beat `HTTPS_PROXY` (`docs/security/network-proxy.md:15-27`; measured 28 Oct). A participant with either
  set sends model traffic to another proxy, unrecorded.
- Other bypasses (vendor docs, `network-proxy.md:79`, `:230-239`): loopback is direct by default; raw `net`,
  `tls`, `http2` sockets, native addons and non-OpenClaw child processes "may bypass"; the managed browser.
  Update check to `telemetry.openclaw.ai`; ClawHub install telemetry (`CLAWHUB_DISABLE_TELEMETRY`,
  `docs/help/environment.md:199-209`).
- Gateway daemon: keeps its own environment (systemd or launchd); the doc tells operators to put proxy settings in
  `~/.openclaw/.env` and reinstall the service (`network-proxy.md:45-47`).
- Known issues (third-party pages citing the tracker, unverified): Windows proxy regression in 2026.2.24 (#26455),
  `NO_PROXY` and browser control (#31219), a direct `api.anthropic.com` usage call with a Bedrock provider (#30672)
  (https://openclawvps.io/blog/openclaw-proxy-configuration, fetched 29 Oct 2026).

#### H5.4 Subagents
- Tools `sessions_spawn`, `sessions_yield`, `subagents`; children run "in its own session", keyed
  `agent:<agentId>:subagent:<uuid>`, and announce back (https://docs.openclaw.ai/tools/subagents, fetched
  29 Oct 2026). Each spawning session has an "in-process queue"; defaults `maxConcurrent` 8,
  `maxChildrenPerAgent` 5, `maxSpawnDepth` 5, `runTimeoutSeconds` 900 (`docs/tools/subagents/operations.md:13-15`,
  `nesting.md:21-27`). These are OpenClaw's own caps, not ours.
- **Local mode (source, unverified by run):** spawn dispatches in-process only inside a Gateway; otherwise it
  calls the Gateway over WebSocket (`src/agents/subagents/spawn/subagent-spawn-gateway.ts:114-127`, `:226-247`;
  `docs/tools/subagents/operations.md:143-147`). The local embedded stub answers only `sessions.list`,
  `sessions.resolve`, `sessions.search`, `chat.history` and throws "requires a running gateway" for anything else
  (`src/agents/tools/embedded-gateway-stub.ts:256-277`). So in `openclaw chat` a subagent likely fails, or runs
  inside a Gateway daemon whose env goalcup does not set. Either way the subagent's model calls may not be
  recorded.
- Prompt: "Use sessions_spawn to start one subagent that writes the tests, wait for it, then run them."

#### H5.5 Session files
- `~/.openclaw/agents/<agent>/agent/openclaw-agent.sqlite` (+ `-wal`, `-shm`), table `transcript_events`; no JSONL
  (measured 28 Oct). `~/.openclaw/state/openclaw.sqlite`. Logs `/tmp/openclaw/openclaw-YYYY-MM-DD.log` (outside
  HOME). `OPENCLAW_HOME` moves all OpenClaw paths (`docs/help/environment.md:56-62`).
- Keys: `~/.openclaw/.env`, `auth_profile_store` in the agent SQLite, legacy `auth-profiles.json`.

#### H5.6 Auto-update
- Gateway auto-update is off by default (`update.auto.enabled`); headless nodes update by default; a daily check
  runs on start. Off for all: `OPENCLAW_NO_AUTO_UPDATE=1` or `update.checkOnStart: false`
  (`docs/install/updating/automatic-updates.md:60-75`, `:92-106`; `docs/help/environment.md:158`).

#### H5.7 Verdict and recipe
- Proxy-ready: **likely** for the main agent in chat mode; **no** if `proxy.proxyUrl` or `OPENCLAW_PROXY_URL` is
  set; **unknown** for subagents.
- DeepSeek-ready: **yes**. Subagents: **yes, but they need a Gateway** (source).
- Recipe: Node 24.22.1+, `npm install -g openclaw@2026.10.9`,
  `openclaw plugins install @openclaw/deepseek-provider`, `DEEPSEEK_API_KEY=` in `~/.openclaw/.env`,
  `openclaw models set deepseek/<id from the refreshed list>`, `openclaw config set logging.file
  ~/.openclaw/logs/openclaw.log`. Check that `openclaw.json` and `.env` hold no `proxy.proxyUrl` or
  `OPENCLAW_PROXY_URL`. Env from goalcup: `HTTPS_PROXY`, `NODE_EXTRA_CA_CERTS=<run CA>`,
  `NODE_USE_ENV_PROXY=1`, `NO_PROXY=localhost,127.0.0.1`, `OPENCLAW_NO_AUTO_UPDATE=1`. Run 1: `openclaw chat`
  with the subagent prompt. Check: main-agent `POST api.deepseek.com/chat/completions` recorded; what the
  spawn does (error text, or a call to `ws://127.0.0.1:18789`). Run 2 (only if run 1 fails to spawn): start
  `openclaw gateway run` in the foreground as the recorded process with the same env, then attach `openclaw tui`
  from another terminal, and check the child's calls are recorded from the Gateway process. What could go wrong:
  the Gateway path changes how goalcup must launch OpenClaw (two processes), which is new work for Friday.

### H6. Amp (Sourcegraph)

#### H6.1 Version, install, headless
- Latest npm `@ampcode/cli` `0.0.1793275344-g17bf52`, published 2026-10-29 12:24 UTC; 219 versions between 29 Sep
  and 29 Oct, about 7 a day (https://registry.npmjs.org/@ampcode%2fcli, fetched 29 Oct 2026). Expect about 50
  builds during a 168-hour run.
- Install on Linux: `curl -fsSL https://ampcode.com/install.sh | bash` (https://ampcode.com/docs/cli, fetched
  29 Oct 2026), or `npm install -g @ampcode/cli` (npm README; postinstall picks `@ampcode/cli-linux-x64`).
- Chat mode: `amp` opens the TUI. Headless: `amp -x "<prompt>"` (execute mode; `--stream-json` exists).
  `amp -ox` runs the prompt in an orb on Amp's servers (https://ampcode.com/docs/cli/execute-mode, fetched
  29 Oct 2026). Headless needs `AMP_API_KEY` set to an access token from Settings that "starts with `sgamp_`"; the
  short-lived `amp login` token is rejected in that mode (same page).

#### H6.2 Keys
- Bring-your-own key: yes, but only through Amp. Model Routing supports API keys, subscriptions, clouds, routers
  and "any OpenAI- or Anthropic-compatible endpoint through a Custom URL connection" with formats
  `chat-completions` (default, `<base URL>/chat/completions`), `responses`, `anthropic-messages`
  (`<base URL>/v1/messages`) and `google-genai` (https://ampcode.com/docs/customize/model-routing, undated,
  fetched 29 Oct 2026). The Free Agent post says BYOK providers including "Custom endpoint URLs" are "now
  available to all Amp users on every tier" (https://ampcode.com/news/free-agent, 13 Sep 2026, updated 30 Sep 2026).
- **Where inference runs:** "Amp sends model requests from its own servers. If your provider or gateway accepts
  requests only from known IP addresses, allow [Amp's server IP addresses]" (model-routing page). So a DeepSeek
  key configured in Amp is used by Amp's servers. The recording proxy never sees the DeepSeek call.
- DeepSeek: not named in any Amp page. The route would be a Custom URL connection, format `chat-completions`,
  base `https://api.deepseek.com`, set in the web UI (`/settings/model-routing`) or with
  `amp config model-providers ...` (`list`, `test`, `check-access --provider-model`, `edit-key`, model mappings).
  A model shows in the Mode Dial "only when it knows the model's token limits"; otherwise add a Model Metadata
  Override (same page). Whether the main agent and `Task` subagents can run on a DeepSeek model end to end:
  **unverified**.
- Still needs an Amp account in every case: `amp login` (device flow) or `AMP_API_KEY`.
- Test key: an Amp access token, plus either the owner's DeepSeek key stored in Amp Model Routing (cheap, but
  stored on Amp's side, encrypted per the docs) or Amp credits. The DeepSeek key never goes on the local wire.
- Side note: strings in the 29 Oct binary include `auth.openai.com/oauth/...` and
  `api.anthropic.com/api/oauth/claude_cli/create_api_key` (D-harnesses.md:1336-1342). Amp also has a mode that
  drives a local Claude Code binary (`docs/harnesses/amp-cli.md:118`). Those subscription paths may make local
  calls; unverified.

#### H6.3 Proxy
- Runtime: native Bun single-file executable (Bun 1.4.4 per strings of the 29 Oct build, D-harnesses.md:147).
- `HTTPS_PROXY`: honoured for login, REST and the thread WebSocket (measured live 15 Oct and chat mode 16 to 17 Oct,
  macOS arm64, `docs/harnesses/amp-cli.md:129-130`). Vendor docs: "set `HTTP_PROXY` and `HTTPS_PROXY`" and
  `NODE_EXTRA_CA_CERTS` for certificates (https://ampcode.com/docs/cli/settings, fetched 29 Oct 2026).
- CA: `NODE_EXTRA_CA_CERTS`, adds to Bun's roots (mode `extend`, `connectors/amp-cli/harness.toml:62-66`).
- Wire: one WebSocket of JSON-RPC text frames to `ampcode.com/actors/gateway/threadActor/`, HTTP/1.1 upgrade
  through the proxy (`connectors/amp-cli/harness.toml:111-119`). No gRPC or HTTP/2-only path seen. Pinning: none
  seen. The string `AMP_USE_NATIVE_WEBSOCKET` exists; what it changes is unverified.
- What skips the recording: (a) all provider calls (server-side); (b) orbs (`-ox`) run on Amp's servers;
  (c) runners and the macOS app run Amp elsewhere; (d) the ripgrep download at start if `rg` is missing
  (`docs/harnesses/amp-cli.md:131`, host unseen).
- Known proxy issues in a public tracker: none found (Amp has no public issue tracker; search on 29 Oct 2026 found
  only third-party pages).
- Linux, logged in, through the proxy: **unverified** (all live probes were macOS; Linux probes were no-login).

#### H6.4 Subagents
- `Task` (delegated work) and "Read Thread"; Oracle, Librarian, Finder are plugins that add tools. "You can also
  ask Amp to use a subagent or to split independent work across several subagents." Subagents run the same model
  as the thread (https://ampcode.com/docs/models-and-subagents, fetched 29 Oct 2026).
- Where they run: the agent loop is on Amp's servers; tool calls run on the local client. `Task` subagent events
  arrive on the same WebSocket with `parentToolUseId` (measured 22 Oct, `docs/harnesses/amp-cli.md:176`). No
  local child process for the model loop, so env inheritance does not apply. Oracle and Librarian: unverified.
- Prompt: "Use one Task subagent to <task>, then report its answer."

#### H6.5 Session files
- No local transcript. Thread content lives on `ampcode.com`. Local: `~/.cache/amp/logs/threads/T-<uuid>.log`,
  JSON lines, metadata only (`connectors/amp-cli/harness.toml:84-90`).
- Key-holding files: `~/.local/share/amp/accounts.json`, `secrets.json` (mode 0600), and
  `.legacy-state-owner.json` (a value derived from the key) (`docs/harnesses/amp-cli.md:93`, `:109`).

#### H6.6 Auto-update
- Yes. "Amp automatically checks and installs new versions in the background" (https://ampcode.com/docs/cli).
  Setting `amp.updates.mode` (default `"auto"`; `"warn"`, `"disabled"`). `AMP_SKIP_UPDATE_CHECK=1` "disables all
  update checks" and overrides the setting. Runners: `amp.runner.autoUpdate.enabled`
  (https://ampcode.com/docs/cli/settings, fetched 29 Oct 2026).

#### H6.7 Verdict and recipe
- Proxy-ready: **yes for Amp's own WebSocket; no for provider calls** (they never leave Amp's servers).
- DeepSeek-ready: **needs own key** (Amp account), DeepSeek only via server-side Custom URL (unverified).
- Subagents: **yes**.
- Recipe: install `npm install -g @ampcode/cli@0.0.1793275344-g17bf52` (pin the version). In the Amp web UI, add a
  Custom URL connection, format `chat-completions`, base `https://api.deepseek.com`, with the DeepSeek key, plus a
  metadata override if the model does not appear in the Mode Dial; run `amp config model-providers test <id>`.
  Pass `AMP_API_KEY=<sgamp_ token>` or sign in during prep. Env from goalcup: `HTTPS_PROXY`,
  `NODE_EXTRA_CA_CERTS=<run CA>`, `AMP_SKIP_UPDATE_CHECK=1`. Prompt: "Write chess.py with a legal-move
  generator. Use one Task subagent to write the tests, then run them." Check: the WebSocket to
  `ampcode.com/actors/gateway/threadActor/` is recorded; subagent frames carry `parentToolUseId`; no direct
  connection to `api.deepseek.com` appears (expected); thread log `T-*.log` exists; the DeepSeek dashboard shows
  calls from Amp's IPs. What could go wrong: the free tier or the Mode Dial may refuse a DeepSeek model;
  Amp may still bill credits; a recorded Amp run is never a recording of provider calls, so it cannot satisfy
  point 1 the way the others do. Say so to the owner before counting Amp as "recorded".

### H7. Cursor CLI (`agent`, `cursor-agent`)

#### H7.1 Version, install, headless

| Item | Fact | Source |
|---|---|---|
| Latest | `2026.10.22-27dcf5f`. The install script still names this build on 29 Oct. | https://cursor.com/install (fetched 29 Oct 2026, lines 78-131) |
| Cadence | About weekly: 2026.09.15, 09.22, 09.28, 10.08, 10.15, 10.17, 10.22. | https://cursor.com/docs/cli/changelog (fetched 29 Oct 2026) |
| Install on Linux | `curl https://cursor.com/install -fsS \| bash`. It unpacks into `~/.local/share/cursor-agent/versions/<build>/` and links `~/.local/bin/agent` and `~/.local/bin/cursor-agent`. Not on npm. | install script above |
| Headless | `agent -p --force "<prompt>"`, with `--output-format text\|json\|stream-json`. Without `--force` it only proposes edits. | https://cursor.com/docs/cli/headless (fetched 29 Oct 2026) |

#### H7.2 Keys

- Bring your own key: **not in the public CLI.** Auth is a Cursor account: browser `agent login`, or a
  Cursor dashboard key in `CURSOR_API_KEY` or `--api-key`
  (https://cursor.com/docs/cli/reference/authentication, fetched 29 Oct 2026). The config reference lists no
  provider key or base URL setting (https://cursor.com/docs/cli/reference/configuration).
- Cursor's "BYOK" is a team model allowlist on Cursor's server (`byok_enabled`, `byok_disabled`, `use_byok_allowlist`
  in the bundle). Even then inference runs on Cursor's backend, not from the client (source; appendix D,
  `docs/plans/2026-10-30-audit/D-harnesses.md:1384-1386`).
- Hidden code path, new finding: the 2026.10.22 bundle has hidden flags `--base-url` (env
  `CURSOR_LOCAL_AGENT_BASE_URL` or `ANTHROPIC_BASE_URL`, "OpenAI-compatible or Anthropic Messages"),
  `--local-agent-api-key` (env `CURSOR_LOCAL_AGENT_API_KEY` or `ANTHROPIC_AUTH_TOKEN`) and `--authless`
  ("Run agent-cli-local without Cursor auth or backend services"). The same code exits with "--base-url can
  only be used with agent-cli-local" unless a local agent runtime is present, and the public build appears
  to start without one (source: module `./src/local-agent-provider-config.ts` in `index.js`; caller in
  `7000.index.js`). So DeepSeek through this path is **unverified and probably unavailable** in the public
  build. Do not plan on it.
- DeepSeek: **no.** Needs its own Cursor account and key.
- Test key: a Cursor account key in `CURSOR_API_KEY` (owner must buy a Cursor plan). Not DeepSeek.

#### H7.3 Proxy and CA

| Question | Answer | Source |
|---|---|---|
| Runtime | Bash launcher plus a bundled `node`. The launcher adds `--use-system-ca` unless `AGENT_CLI_CREDENTIAL_STORE=file`. | `cursor-agent` launcher in the package (source) |
| `HTTPS_PROXY` | Partly. Docs: set `HTTPS_PROXY` and `NODE_USE_ENV_PROXY=1`. The bundle installs `global-agent` with an empty namespace at start, so `node:http(s)` reads `HTTP_PROXY`/`HTTPS_PROXY`/`NO_PROXY`; native `fetch` needs `NODE_USE_ENV_PROXY=1`. | configuration docs above; `./src/utils/proxy-agent.ts` in `index.js` (source) |
| Custom CA | `NODE_EXTRA_CA_CERTS`, plus the system store through `--use-system-ca`. 2026.09.15 also added system CAs to the login fetch through `HTTPS_PROXY`. | configuration docs; changelog 2026.09.15 |
| HTTP/2 | **The blocker.** The agent stream is Connect RPC, HTTP/2 bidirectional by default. `network.useHttp1ForAgent: true` asks for HTTP/1.1 plus SSE, but server config can override it (`server_force_all_enabled` gives `useHttp1: false`). Our proxy refuses `h2`-only clients, so the agent cannot reach its backend when the server forces h2. | `index.js` function with `server_force_*` reasons (source); `D-harnesses.md:1388-1398` |
| Model calls | Not visible to us at all. The CLI talks only to `api2.cursor.sh` and agent hosts from server config (`agentn...api5.cursor.sh` seen by users). Cursor's servers build the prompt and call the provider. A working proxy would record Cursor's own protobuf stream, not the provider request, so point 1 ("every token the harness requests") is only partly met. | `./src/utils/service-urls.ts` (source); https://forum.cursor.com/t/agent-ignores-http-proxy-and-https-proxy-when-http-2-is-enabled-bypasses-corporate-proxy/173619 |
| Paths that skip the proxy | IDE (not CLI): with HTTP/2 on, the agent connects straight to `agentn.global.api5.cursor.sh:443`, ignoring proxy settings; staff confirmed on 29 Oct 2026. The reporter quotes a staff note of 16 Jun 2026 that the CLI's `http2.connect()` now honours `HTTPS_PROXY` (not seen by us, unverified). Metrics go to `api3.cursor.sh`. Cloud agents run on Cursor's machines. | forum thread 173619 above; `service-urls.ts` (source) |
| Tracker | Mixed reports: `useHttp1ForAgent: true` gives "ConnectError: [internal] Parse Error" for some users; others say the CLI "does not fully support" proxy env vars (Jan 2026). | https://forum.cursor.com/t/cursor-agent-cli-does-not-accept-http-1-1/135105 ; https://forum.cursor.com/t/cursor-cli-agent-fails-behind-corporate-proxy-on-rhel/148953 ; https://forum.cursor.com/t/cursor-agent-cli-behind-corporate-proxy/132972 |

#### H7.4 Subagents

- Yes. Built in: Explore and Browser. Custom ones in `.cursor/agents/` or `~/.cursor/agents/` (also reads
  `.claude/agents/` and `.codex/agents/`). Foreground or background, several at once
  (https://cursor.com/docs/agent/subagents, fetched 29 Oct 2026).
- Ask in the prompt: "Use a subagent to list every file in this repo and report the count, then summarise its
  answer." Naming one also works ("Have the explore subagent ...").
- Process model: inference is on Cursor's backend; local tool calls for subagents appear to run in the same
  CLI process (unverified). Cloud agents run elsewhere and are out of reach.

#### H7.5 Session files

- Chat store: `~/.cursor/chats/<md5 of workspace path>/<agent id>/store.db` (SQLite, content-addressed blobs,
  some protobuf) (source: `1623.index.js`, `getDbPath`; spike `spikes/cursor-cli/fixtures/README.md:1-8`).
- Transcripts: `~/.cursor/projects/<slug>/agent-transcripts/`; since 2026.10.17 subagent transcripts live in
  `agent-transcripts/<parent>/subagents/` (changelog 2026.10.17). JSONL, lossy (`D-harnesses.md:1398-1401`).
- Keys: login token in the OS keyring, or a file when `AGENT_CLI_CREDENTIAL_STORE=file` (launcher, source).
  Config `~/.cursor/cli-config.json`; `CURSOR_CONFIG_DIR`, `CURSOR_DATA_DIR` move them.

#### H7.6 Auto-update

- Yes, a background check after start (changelog 2026.10.22 mentions staged updates).
- Off: hidden flag `--disable-auto-update` (source, `index.js`), or `"channel": "static"` in
  `cli-config.json`, which the code skips (source, `7000.index.js`; unverified by a run). Docs list only
  `channel`.

#### H7.7 Verdict

- Proxy-ready: **no** (HTTP/2 bidi that the server can force; model calls happen on Cursor's servers anyway).
- DeepSeek-ready: **needs own key** (Cursor account).
- Subagents: **yes**.
- Test-pad recipe (only if the owner wants proof that it fails cleanly): install with the script above;
  `export CURSOR_API_KEY=<cursor key>`; write `~/.cursor/cli-config.json` with
  `{"network":{"useHttp1ForAgent":true}}`; goalcup passes `HTTPS_PROXY=http://127.0.0.1:<port>`,
  `NODE_USE_ENV_PROXY=1`, `NODE_EXTRA_CA_CERTS=<run CA>`; run
  `agent -p --force --output-format stream-json --disable-auto-update "Use a subagent to list every file in this repo and report the count, then summarise its answer."`.
  Check: does the proxy see `h2` refusals or an HTTP/1.1 SSE stream to a `*.cursor.sh` host; does the run
  stall (point 3); does `agent-transcripts/<parent>/subagents/` get a file. Everything here is unverified.
  Recommendation: keep Cursor out of Edition 01 and say so to participants.

### H8. Gemini CLI (Google)

#### H8.1 Version, install, headless

| Item | Fact | Source |
|---|---|---|
| Latest | 0.66.0 stable (27 Oct 2026 20:30 UTC). Preview 0.67.0-preview.0; nightly daily. | https://registry.npmjs.org/@google%2Fgemini-cli ; https://github.com/google-gemini/gemini-cli/releases/tag/v0.66.0 |
| Cadence | Weekly stable on Tuesdays: 0.62.0 (29 Sep), 0.63.0 (6 Oct), 0.64.0 (13 Oct), 0.65.0 (20 Oct), 0.66.0 (27 Oct). Expect 0.67.0 on 3 Nov. | npm `time` field |
| Status | Since 18 Jun 2026 it no longer serves free, Google AI Pro and Ultra users. It "will remain accessible via paid Gemini and Gemini Enterprise Agent Platform API keys" and for Code Assist Standard or Enterprise licences. Free-tier API keys are not mentioned. | https://developers.googleblog.com/an-important-update-transitioning-gemini-cli-to-antigravity-cli/ (19 May 2026) |
| Install on Linux | `npm install -g @google/gemini-cli@0.66.0` (Node 20 or newer; `engines.node >=20`). | npm metadata |
| Headless | `gemini -p "<prompt>" --approval-mode yolo --output-format stream-json --skip-trust`. `--skip-trust` sets `GEMINI_CLI_TRUST_WORKSPACE=true`; without trust a headless run stops. | `bundle/gemini-22EOGL6O.js` (options near lines 8055-8258, message at 11174) (source) |

#### H8.2 Keys

- Bring your own key: yes, but only Google keys. Auth types in the source: `oauth-personal`,
  `gemini-api-key`, `vertex-ai`, `compute-default-credentials`, `cloud-shell`, and `gateway`
  (`bundle/chunk-HJ5KJXNZ.js:314861-314886`, source).
- `gateway` mode: set `GOOGLE_GEMINI_BASE_URL` and the CLI talks to that URL, still in the Gemini API format,
  key from `GEMINI_API_KEY` (`chunk-HJ5KJXNZ.js:314877-314934`, source).
- DeepSeek: **no native route.** DeepSeek offers OpenAI-format and Anthropic-format endpoints only
  (https://api-docs.deepseek.com/, fetched 29 Oct 2026). Gemini CLI has no OpenAI-compatible provider (no
  `OPENAI` string anywhere in the 0.66.0 bundle). The only way is a translating gateway behind
  `GOOGLE_GEMINI_BASE_URL` (for example the community "Jamini" setup,
  https://awesome.ecosyste.ms/projects/github.com%2Filldynamics%2Fjamini, 0 stars, unvetted). That is not how
  participants run it, and it adds a moving part. Do not use it for the testbed.
- Forks: Qwen Code (`@qwen-code/qwen-code` 0.26.4, 29 Oct 2026, 85,288 downloads 21-27 Oct) is a Gemini CLI
  fork with an OpenAI-compatible provider (`modelProviders.openai`, `~/.qwen/settings.json`)
  (https://qwenlm.github.io/qwen-code-docs/en/users/configuration/model-providers/). It could take DeepSeek,
  but whether it sends DeepSeek's `reasoning_content` back (see H9.2) is unverified. Covered in H13.
- Test key: a **paid Gemini API key** in `GEMINI_API_KEY` (the owner must fund a Google AI Studio or Agent
  Platform key). If the owner does not want a Google key, drop Gemini CLI from the testbed.

#### H8.3 Proxy and CA

| Question | Answer | Source |
|---|---|---|
| Runtime | Node. `gemini.js` relaunches itself as a child `node` process (for heap size) with `env: {...process.env, GEMINI_CLI_NO_RELAUNCH: "true"}`, so the child inherits our env. 0.66.0 also forwards termination signals to the child. | `bundle/gemini.js` (`run()`), `chunk-NORTZ2HW.js:1906-1931` (source); release notes v0.66.0 |
| `HTTPS_PROXY` | Yes, for model calls and the rest. The config takes `proxy` only from `HTTPS_PROXY`, `https_proxy`, `HTTP_PROXY`, `http_proxy` (no settings key), then sets a global undici `EnvHttpProxyAgent` for all `fetch` (honours `NO_PROXY`). No `NODE_USE_ENV_PROXY` needed. Usage statistics use `HttpsProxyAgent` with the same proxy. | `gemini-22EOGL6O.js:8604`; `chunk-HJ5KJXNZ.js:329714-329726` and `:388124-388128`; `:285773-285780` (source) |
| `.env` files | `.env` in the project and `~/.gemini/.env` never override a variable that is already set, so they cannot replace our `HTTPS_PROXY`. They can add `GOOGLE_GEMINI_BASE_URL`; the proxy still sees that host. | `chunk-G5X4CDQU.js` `loadEnvironment`, the `Object.hasOwn(process.env, key)` check near line 16671 (source) |
| Custom CA | `NODE_EXTRA_CA_CERTS` in the launch env (a Node start-up setting, so it must be in the real env, not in `.gemini/.env`; a regression about the `.env` case was closed 5 Aug 2026). | https://github.com/google-gemini/gemini-cli/issues/25987 |
| HTTP/2, WebSockets, gRPC | Model calls are HTTP/1.1 through undici (the only `allowH2: true` is undici's unused `h2c` client). One `wss://generativelanguage.googleapis.com/...BidiGenerateContent` URL exists for the live API, not used by normal chat (unverified). OpenTelemetry export is off by default; if a participant turns it on with a GCP or gRPC exporter it may not use the proxy (unverified). No pinning found. | `chunk-HJ5KJXNZ.js:219082` (source) |
| Paths that skip the proxy | Sandbox mode (`--sandbox`, Docker or Podman) runs tools in a container and sets its own `HTTPS_PROXY` (default `http://localhost:8877` if none); auto-update is off in sandbox mode. Remote A2A subagents call other servers. Usage statistics go to `play.googleapis.com/log` through the proxy (off with `privacy.usageStatisticsEnabled: false`). | `gemini-22EOGL6O.js:10260-10261`, `:10511-10514`; `chunk-K52VRT6X.js:1642-1647`; `chunk-HJ5KJXNZ.js:284182` (source) |
| Tracker | Old proxy issues are login-related (Google sign-in behind a proxy, closed): #2300, #19374, #26171. No open October 2026 proxy issue found. | https://github.com/google-gemini/gemini-cli/issues?q=is%3Aissue%20proxy%20certificate (fetched 29 Oct 2026) |

#### H8.4 Subagents

- Yes, on by default (`experimental.enableAgents`, default `true`). Built in: `codebase_investigator`,
  `cli_help`, `generalist`, and `browser_agent` (off by default). Custom ones in `.gemini/agents/*.md` and
  `~/.gemini/agents/*.md`. Remote subagents over A2A are possible
  (https://geminicli.com/docs/core/subagents/, updated 8 Jun 2026; `chunk-G5X4CDQU.js:14419-14427`, source).
- Ask in plain words: "Use a subagent to list every file in this repo and report the count, then summarise
  its answer." Starting the prompt with `@generalist` forces one; prefer plain words (point 7).
- Process model: local subagents run in the same process with their own chat loop and their own transcript
  file (source: `kind === "subagent"` in the chat recorder). They share the process env.

#### H8.5 Session files

- Main: `~/.gemini/tmp/<project id>/chats/session-<YYYY-MM-DDTHH-MM>-<first 8 of id>.jsonl` (JSONL in 0.66.0;
  older `.json` files are still read).
- Subagents: `~/.gemini/tmp/<project id>/chats/<parent session id>/<subagent session id>.jsonl`.
- Source: `chunk-HJ5KJXNZ.js:287527-287560`. `<project id>` comes from a project registry, not a raw hash
  (source `getProjectIdentifier`; exact name rule unverified). `GEMINI_CLI_HOME` moves `~/.gemini`
  (`chunk-HJ5KJXNZ.js:252013-252019`).
- Retention: `general.sessionRetention` on by default, `maxAge` 30 days (`chunk-G5X4CDQU.js:12673`). A week
  run is safe, but deletion removes subagent folders with the parent.
- Keys: `GEMINI_API_KEY` may sit in `~/.gemini/.env` or a project `.env` (plain text). Google sign-in in
  `~/.gemini/oauth_creds.json` or the keyring (service `gemini-cli-oauth`); `gemini-credentials.json`;
  `mcp-oauth-tokens.json` (file names from source).

#### H8.6 Auto-update

- Yes. When installed with npm it runs the update command (`npm i -g @google/gemini-cli@<latest>`) in the
  background during an interactive session. Skipped for npx, pnpx, bunx, binary installs and sandbox mode.
- Off: `~/.gemini/settings.json` `{"general":{"enableAutoUpdate":false,"enableAutoUpdateNotification":false}}`.
  Source: `chunk-K52VRT6X.js:1640-1700`; schema `chunk-G5X4CDQU.js:12546-12562`. Whether `-p` runs check
  at all: unverified.

#### H8.7 Verdict

- Proxy-ready: **likely** (source is clear; no run through our proxy yet).
- DeepSeek-ready: **needs own key** (paid Gemini API key). No DeepSeek route.
- Subagents: **yes**.
- Test-pad recipe: `npm install -g @google/gemini-cli@0.66.0`; write `~/.gemini/settings.json` with
  `enableAutoUpdate: false`, `enableAutoUpdateNotification: false`; key in `GEMINI_API_KEY` (paid key, not in
  a file). goalcup passes `HTTPS_PROXY=http://127.0.0.1:<port>`, `NODE_EXTRA_CA_CERTS=<run CA>`. Run
  `gemini -p "Use a subagent to list every file in this repo and report the count, then summarise its answer." --approval-mode yolo --skip-trust --output-format stream-json`.
  Check: every request to `generativelanguage.googleapis.com` (`:streamGenerateContent`) is in the capture,
  including those from the subagent; a file appears under `chats/<parent id>/`; usage-statistics posts to
  `play.googleapis.com` show up as recorded non-model traffic; the relaunch child (second `node` process)
  used the proxy. All unverified until run. Low priority: usage is falling (364,690 npm downloads 21-27 Oct
  2026, https://api.npmjs.org/downloads/point/2026-10-21:2026-10-27/@google/gemini-cli) and individuals
  were moved to Antigravity CLI.

### H9. GitHub Copilot CLI (`copilot`)

#### H9.1 Version, install, headless

| Item | Fact | Source |
|---|---|---|
| Latest | 1.0.105 (npm 28 Oct 2026 00:44 UTC; changelog dated 27 Oct). Prerelease 1.0.105-1. | https://registry.npmjs.org/@github%2Fcopilot ; https://github.com/github/copilot-cli/blob/main/changelog.md |
| Cadence | 15 stable versions from 30 Sep to 28 Oct 2026 (1.0.90 to 1.0.105). Almost daily. Expect 5 to 8 more before 7 Nov. | npm `time` field |
| Runtime | One 198 MB ELF per platform (`@github/copilot-linux-x64`), a Node single-executable app with embedded Node v24.22.1. On start it unpacks `index.js`, `app.js`, `cli-main.js` and a native addon `prebuilds/linux-x64/runtime.node` into `~/.cache/copilot/pkg/linux-x64/<version>/` (`COPILOT_CACHE_HOME`, `COPILOT_PKG_CACHE_HOME` move it). The addon is Rust (a log line names `copilot_runtime::auth::gh`; changelog speaks of a Tokio runtime). | `strings` on the binary (source); https://github.com/github/copilot-cli/issues/5256 ; changelog 1.0.62 |
| Install on Linux | `npm install -g @github/copilot@1.0.105` (Node 22+), or `curl -fsSL https://gh.io/copilot-install \| VERSION="v1.0.105" bash`. | https://docs.github.com/en/copilot/how-tos/copilot-cli/set-up-copilot-cli/install-copilot-cli |
| Headless | `copilot -p "<prompt>" --allow-all-tools` (or `COPILOT_ALLOW_ALL=1`), `--output-format json` for JSONL, `--trust-folder` for one-off runs (1.0.100). `-p` waits for background tasks up to `COPILOT_TASK_WAIT_TIMEOUT_SECONDS`. | changelog 0.0.422, 1.0.100, 1.0.104, 1.0.71 |

#### H9.2 Keys

- Bring your own key: **yes, and no GitHub account needed.** "If you use your own LLM provider API keys
  (BYOK), GitHub authentication is not required." `COPILOT_OFFLINE=true` stops all calls to GitHub and turns
  telemetry fully off
  (https://docs.github.com/en/copilot/how-tos/copilot-cli/set-up-copilot-cli/authenticate-copilot-cli;
  https://github.blog/changelog/2026-04-07-copilot-cli-now-supports-byok-and-local-models/, 7 Apr 2026).
  Note: the install page still lists "an active GitHub Copilot subscription" as a prerequisite; the auth
  page wins for BYOK (unverified by a run).
- Config: `~/.copilot/providers.json` (or `COPILOT_PROVIDERS_CONFIG`) with `providers` (`name`, `type`
  `openai`/`azure`/`anthropic`, `baseUrl`, `apiKey` or `bearerToken`) and `models`. Keys there are plain
  text; `$VAR` is not expanded. The older env vars `COPILOT_PROVIDER_*` still work when `providers.json`
  declares nothing
  (https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/use-byok-models ;
  https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-config-dir-reference).
- DeepSeek: **yes, through the Anthropic-format endpoint.** DeepSeek's own guide:
  `COPILOT_PROVIDER_TYPE=anthropic`, `COPILOT_PROVIDER_BASE_URL=https://api.deepseek.com/anthropic`,
  `COPILOT_PROVIDER_API_KEY=<deepseek key>`, `COPILOT_MODEL=<deepseek model id>`; optional
  `COPILOT_PROVIDER_MAX_PROMPT_TOKENS`, `COPILOT_PROVIDER_MAX_OUTPUT_TOKENS`. The `openai` type fails with
  HTTP 400 "The reasoning_content in the thinking mode must be passed back to the API", because DeepSeek's
  thinking mode is on by default and requires that field back on every tool turn
  (https://api-docs.deepseek.com/quick_start/agent_integrations/copilot_cli ;
  https://api-docs.deepseek.com/guides/thinking_mode, both fetched 29 Oct 2026).
- Test key: DeepSeek, Anthropic format, as above. Using env vars keeps the key out of `providers.json`.

#### H9.3 Proxy and CA

| Question | Answer | Source |
|---|---|---|
| `HTTPS_PROXY` | Yes. Env vars documented in `--help` since 1.0.3; "proxy support via HTTPS_PROXY/HTTP_PROXY ... regardless of Node version" since 0.0.336. A settings key `proxyUrl` exists but is "Overridden by the `HTTP_PROXY` or `HTTPS_PROXY` environment variables (any casing)". Scheme-less URLs accepted (0.0.384). An `https://` proxy URL is not supported; use `http://127.0.0.1:<port>`. | changelog 1.0.3, 0.0.336, 1.0.64, 0.0.384; config dir reference (settings part); https://docs.github.com/copilot/concepts/network-settings |
| Custom CA | `NODE_EXTRA_CA_CERTS` plus the OS store ("Load CA certificates from system and environment variables", 0.0.370; loaded asynchronously since 1.0.40). The binary holds the strings `NODE_EXTRA_CA_CERTS`, `SSL_CERT_FILE`, `SSL_CERT_DIR`. Which layer opens model connections (embedded Node or the Rust addon) is **unverified**; if it is the Rust addon, only the system store or `SSL_CERT_FILE` may count. Pass both. | changelog; network-settings page; `strings` (source) |
| HTTP/2 | HTTP/1.1 by default since 1.0.57 (1 Jun 2026). `COPILOT_ENABLE_HTTP2=1` opts in; do not set it. | changelog 1.0.57 |
| WebSockets | Responses over WebSocket for GitHub-hosted OpenAI-family models and for BYOK `openai` providers with `wireApi: "responses"` (1.0.64). Not used by the Anthropic-type DeepSeek setup (unverified). Our proxy relays WebSocket upgrades. | changelog 1.0.64, 1.0.62 |
| Paths that skip the proxy | `/delegate` hands work to GitHub's cloud agent (needs GitHub sign-in; off without it). Session sync to GitHub (`remoteExport`, default `true`, only when signed in). Voice/STT runtime ignores proxy settings since 1.0.101 (#5245, open). The shell sandbox has its own proxy and CA (`copilot sandbox ca`); with no sandbox proxy configured it chains to `HTTPS_PROXY` (1.0.100). MCP servers inherit `HTTPS_PROXY` but may not trust `NODE_EXTRA_CA_CERTS` if not Node. | changelog 1.0.100, 1.0.91, 1.0.83; https://github.com/github/copilot-cli/issues/5245 ; https://coder.com/docs/ai-coder/ai-gateway/clients/copilot |
| Tracker | #333 "fetch failed ... SSL inspection" (open since Oct 2025): a Windows user on 1.0.54 said `NODE_EXTRA_CA_CERTS` was ignored by the embedded Node; maintainer on 26 Oct 2026 says fixes shipped up to 1.0.83 and asks for a retest. #5256 (open, 1.0.101, Windows): BYOK with no GitHub login and no `gh` on PATH spins a CPU core retrying `gh auth token`; signing in or setting `COPILOT_GITHUB_TOKEN` stops it. Whether `COPILOT_OFFLINE=true` avoids it: unverified. | https://github.com/github/copilot-cli/issues/333 ; https://github.com/github/copilot-cli/issues/5256 |

#### H9.4 Subagents

- Yes. Built in: explore, task, general-purpose, code-review (docs), plus rubber-duck (changelog). "Built-in
  sub-agents ... automatically inherit your provider configuration" (BYOK post, 7 Apr 2026). Custom agents
  in `~/.copilot/agents/` and the repo. `/fleet` runs many in parallel. Default nesting depth 4 to 6 and a
  plan-based concurrency cap; only usage-based billing users can raise them (`subagents.maxDepth` up to
  256, `subagents.maxConcurrency` up to 32). Those caps are Copilot's, not ours.
  (https://docs.github.com/en/copilot/how-tos/copilot-cli/use-copilot-cli-agents/invoke-custom-agents ;
  changelog 1.0.71, 1.0.66; config dir reference.)
- Ask: "Use a subagent to list every file in this repo and report the count, then summarise its answer."
- Process model: likely in the same process (a 1.0.6 fix names an HTTP/2 connection pool shared with active
  subagents), but docs say "subsidiary subagent process". Unverified. "Teleported" subagents (1.0.79) may
  run remotely; unverified.

#### H9.5 Session files

- `~/.copilot/session-state/<session id>/` holds `events.jsonl` (append-only JSONL), plans, checkpoints and
  workspace files. `~/.copilot/session-store.db` is a SQLite index ("can be rebuilt"). `~/.copilot/logs/` has
  per-session process logs. `COPILOT_HOME` moves the whole tree
  (https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-config-dir-reference).
- Subagents: each has its own subagent session (changelog 1.0.81 "recorded on that subagent's session and
  re-emitted on its parent"). Whether that is a separate folder or lines with an `agentId` inside the
  parent's `events.jsonl` is **unverified**.
- Keys: `providers.json` (plain text keys, if used); `config.json` holds the GitHub OAuth token in plain text
  when no keyring exists (headless Linux without libsecret); `mcp-secrets/` and `mcp-oauth-config/` as
  keyring fallbacks. Token env vars `COPILOT_GITHUB_TOKEN`, `GH_TOKEN`, `GITHUB_TOKEN` (auth page above).

#### H9.6 Auto-update

- Yes. `autoUpdate` (default `true`) downloads updates at session start into `~/.cache/copilot/pkg/`, and the
  loader runs the newest unpacked version; off by default in CI environments (0.0.407).
- Off: `--no-auto-update` or `COPILOT_AUTO_UPDATE=false` (both read by the loader: `process.argv.includes("--no-auto-update")`
  and `process.env.COPILOT_AUTO_UPDATE?.toLowerCase()==="false"`, source), or `"autoUpdate": false` in
  `~/.copilot/settings.json`. `--prefer-version` also skips it.

#### H9.7 Verdict

- Proxy-ready: **likely** (documented env proxy that beats the settings key; HTTP/1.1 default; CA path not
  yet measured with our run CA; which layer dials is unverified).
- DeepSeek-ready: **yes** (Anthropic type at `https://api.deepseek.com/anthropic`; not the `openai` type).
- Subagents: **yes**.
- Test-pad recipe: `npm install -g @github/copilot@1.0.105`. Pad env (participant setup):
  `COPILOT_PROVIDER_TYPE=anthropic`, `COPILOT_PROVIDER_BASE_URL=https://api.deepseek.com/anthropic`,
  `COPILOT_PROVIDER_API_KEY=<deepseek key>`, `COPILOT_MODEL=<deepseek model id>`, `COPILOT_OFFLINE=true`,
  `COPILOT_AUTO_UPDATE=false`, and a fresh `COPILOT_HOME`. goalcup passes
  `HTTPS_PROXY=http://127.0.0.1:<port>`, `NODE_EXTRA_CA_CERTS=<run CA>`, and `SSL_CERT_FILE=<system bundle plus run CA>`
  (a full bundle, since `SSL_CERT_FILE` may replace the default roots). Run
  `copilot -p "Use a subagent to list every file in this repo and report the count, then summarise its answer." --allow-all-tools --no-auto-update --output-format json`.
  Check: every `POST /anthropic/v1/messages` to `api.deepseek.com` is in the capture, including the
  subagent's; no 400 `reasoning_content` error; no direct connection to `api.deepseek.com` outside the proxy
  (watch with `ss -tnp` during the run); `events.jsonl` shows the subagent; CPU stays normal (issue #5256). A
  second run without `COPILOT_OFFLINE` shows what GitHub traffic (telemetry, `gh auth token` probing) does.
  All unverified until run. Best candidate of this lane: 1,407,715 npm downloads 21-27 Oct 2026
  (https://api.npmjs.org/downloads/point/2026-10-21:2026-10-27/@github/copilot).

### H10. Goose (aaif-goose, formerly block/goose)

#### H10.1 Version, install, headless

| Item | Fact | Source |
|---|---|---|
| Latest | v1.57.0, tag commit 27 Oct 2026. Earlier: v1.53.0 (30 Sep), v1.54.0, v1.55.0 (16 Oct), v1.56.0 (22 Oct). About weekly. v2 release-candidate tags also exist (`v2-rc.1`, `v2.0.0-rc-04-27-0`; dates unverified). | `git ls-remote --tags https://github.com/aaif-goose/goose` (29 Oct 2026); https://github.com/aaif-goose/goose/releases |
| Runtime | Rust single binary. | https://github.com/aaif-goose/goose/blob/v1.57.0/Cargo.toml |
| Install on Linux | `curl -fsSL https://github.com/aaif-goose/goose/releases/download/stable/download_cli.sh \| GOOSE_VERSION=v1.57.0 CONFIGURE=false bash`. Installs to `$HOME/.local/bin` (`GOOSE_BIN_DIR` moves it). | https://github.com/aaif-goose/goose/blob/v1.57.0/download_cli.sh#L7-L24 |
| Headless | `goose run -t "<prompt>"` (one shot; `--output-format json` or `stream-json`; `-q` quiet; `--max-turns`). Chat mode: `goose session`. `--no-session` turns off session storage: a participant may use it, and then no native session exists. | https://github.com/aaif-goose/goose/blob/v1.57.0/documentation/docs/guides/goose-cli-commands.md (lines 428-456) |

#### H10.2 Keys and DeepSeek

- Bring your own key: yes. Many native providers; keys come from env vars first, then config
  (`Config::get_param` reads the upper-case env var before the file:
  https://github.com/aaif-goose/goose/blob/v1.57.0/crates/goose/src/config/base.rs#L773-L778).
- DeepSeek: no native DeepSeek provider. Two documented ways:
  1. A custom provider file, `~/.config/goose/custom_providers/custom_deepseek.json`, with
     `"engine": "openai"`, `"base_url": "https://api.deepseek.com/chat/completions"`,
     `"api_key_env": "DEEPSEEK_API_KEY"`, a `models` list, `"supports_streaming": true`,
     `"requires_auth": true`; then `GOOSE_PROVIDER=custom_deepseek` (docs:
     https://github.com/aaif-goose/goose/blob/v1.57.0/documentation/docs/getting-started/providers.md
     lines 405-545). The name `custom_deepseek` is already special-cased in the OpenAI provider
     (it gets `max_tokens` instead of `max_completion_tokens`:
     https://github.com/aaif-goose/goose/blob/v1.57.0/crates/goose-providers/src/openai.rs#L470-L480),
     so this is the name to use. An `"engine": "anthropic"` file pointing at
     `https://api.deepseek.com/anthropic` should also work (unverified).
  2. The OpenAI provider with `OPENAI_HOST=https://api.deepseek.com` and `OPENAI_API_KEY` set to the
     DeepSeek key (`OPENAI_HOST`, `OPENAI_BASE_PATH` config keys:
     https://github.com/aaif-goose/goose/blob/v1.57.0/crates/goose-providers/src/openai.rs#L708-L728).
     Less clean: the variable names say OpenAI.
- Own account needed: no.
- Test key: DeepSeek.
- Key storage: system keyring by default. On a headless Linux box with no keyring Goose falls back
  to `~/.config/goose/secrets.yaml` (plain file). `GOOSE_DISABLE_KEYRING=1` forces the file
  (docs: `environment-variables.md` lines 313 and 343-347). An env var key avoids both.

#### H10.3 Proxy and CA

| Question | Answer | Source |
|---|---|---|
| `HTTPS_PROXY` for model calls | Yes. Documented (`HTTP_PROXY`, `HTTPS_PROXY`, `NO_PROXY`, plus OS proxy settings). reqwest is built with `system-proxy`. Goose turns the proxy off only for loopback HTTP targets (local OAuth callback, local Ollama-style endpoints). | https://github.com/aaif-goose/goose/blob/v1.57.0/documentation/docs/guides/environment-variables.md (lines 372-394); https://github.com/aaif-goose/goose/blob/v1.57.0/crates/goose/Cargo.toml#L124 ; https://github.com/aaif-goose/goose/blob/v1.57.0/crates/goose-providers/src/api_client.rs#L357-L380 |
| `HTTPS_PROXY` for everything else | Likely yes: other clients are plain `reqwest::Client::new()` or builders without `no_proxy()` (telemetry, model metadata). Unverified by a run. | https://github.com/aaif-goose/goose/blob/v1.57.0/crates/goose/src/posthog.rs#L80 |
| Trust a custom CA | Three ways. (a) `GOOSE_CA_CERT_PATH=<run CA PEM>`: added with `add_root_certificate` to every provider client built by the provider registry (extend mode). (b) The Linux system store: the default verifier loads it. (c) `SSL_CERT_FILE`: read by `rustls-native-certs`, but in **replace** mode ("certificates are only loaded from the locations specified via environment variables"), so the file must hold the system roots plus the run CA. Release binaries use rustls (the `rustls-tls` default feature); a `native-tls` build would use OpenSSL's store instead. | https://github.com/aaif-goose/goose/blob/v1.57.0/crates/goose/src/config/tls.rs#L32-L35 ; https://github.com/aaif-goose/goose/blob/v1.57.0/crates/goose/src/providers/init.rs#L57-L61 ; https://github.com/aaif-goose/goose/blob/v1.57.0/crates/goose-providers/src/api_client.rs#L396-L414 ; https://docs.rs/rustls-native-certs/0.8.4/rustls_native_certs/fn.load_native_certs.html (29 Oct 2026); https://github.com/aaif-goose/goose/blob/v1.57.0/crates/goose-cli/Cargo.toml#L146-L153 |
| HTTP/2, WebSockets, gRPC, pinning | reqwest `http2` is on, chosen by ALPN; our HTTP/1.1 proxy offers only HTTP/1.1, so the call falls back (unverified). WebSockets only for the optional live-voice feature, not text model calls. No gRPC. No certificate pinning found. | `crates/goose/Cargo.toml#L124`; `crates/goose-providers/Cargo.toml` lines 25 and 58 |
| Paths that skip the proxy | None found for model calls. Telemetry (PostHog `us.i.posthog.com`) is off by default (`GOOSE_TELEMETRY_ENABLED`, default false) and uses the proxy anyway. The desktop app talks to a local `goosed` server; the CLI does not need it. | https://github.com/aaif-goose/goose/blob/v1.57.0/crates/goose/src/posthog.rs#L17-L25 |
| Tracker | No open proxy bug found. #5541 "Inspecting https certificates" closed 14 May 2026 ("configurable TLS, native-tls support, and self-signed certificate handling" shipped). #6072 GUI proxy support PR closed 6 Jan 2026. | https://github.com/aaif-goose/goose/issues/5541 ; https://github.com/aaif-goose/goose/issues?q=proxy%20certificate (29 Oct 2026) |

#### H10.4 Subagents

- Yes. The `delegate` tool comes from the built-in "Summon" platform extension, on by default
  (https://github.com/aaif-goose/goose/blob/v1.57.0/crates/goose/src/agents/platform_extensions/mod.rs#L128-L139).
- How to ask: plain words, for example "Use 1 subagent to ...". Parallel when the prompt says
  "in parallel". Only in `GOOSE_MODE=auto` (the default); off in approve, smart approve and chat
  modes (docs, `subagents.mdx`).
- In-process: each subagent is a tokio task in the same process
  (https://github.com/aaif-goose/goose/blob/v1.57.0/crates/goose/src/agents/subagent_handler.rs#L184),
  so it uses the same HTTP clients, proxy and CA. Shell tools are child processes and inherit the env.
- Limits that matter for long runs: at most 5 run at once per parent (`GOOSE_MAX_CONCURRENT_SUBAGENTS`;
  extra tasks wait), and a **5-minute default timeout** per subagent ("Request longer timeout in
  your prompt"). Background subagents were removed in 1.55.0 (appendix D). These are Goose's
  limits, not ours; record what happens.
- External subagents: Goose can run another CLI (for example Codex) as an MCP server. That child is
  a different harness; it inherits the env, but its own proxy and CA rules apply.

#### H10.5 Session files

- One SQLite database: `~/.local/share/goose/sessions/sessions.db` (since 1.10.0). Subagents are
  rows of the same tables with `session_type = 'sub_agent'`
  (https://github.com/aaif-goose/goose/blob/v1.57.0/crates/goose/src/session/session_manager.rs#L33-L34 ,
  L60-L69). Holds messages, tool calls, token counts.
- Logs: `~/.local/state/goose/logs/cli/<date>/` (deleted after two weeks). History:
  `~/.config/goose/history.txt`. Config: `~/.config/goose/config.yaml`.
- Holds keys: `~/.config/goose/secrets.yaml` when the keyring is off or missing; the custom
  provider JSON can hold headers with secrets. (Docs: `guides/logs.md`.)

#### H10.6 Auto-update

The CLI does not update itself: `goose update` is manual (`--canary`, `--reconfigure`). No
startup version check found in `goose-cli`. A build feature `disable-update` removes the command.
The desktop app auto-updates. (https://github.com/aaif-goose/goose/blob/v1.57.0/documentation/docs/guides/updating-goose.md ,
https://github.com/aaif-goose/goose/blob/v1.57.0/crates/goose-cli/Cargo.toml)

#### H10.7 Verdict

Proxy-ready: **likely**. DeepSeek-ready: **yes** (custom provider). Subagents: **yes** (in-process).

Test-pad recipe (unverified): install v1.57.0 with the script above, `CONFIGURE=false`. Write
`~/.config/goose/custom_providers/custom_deepseek.json` as in H10.2 with the `<cheap model id>`
from api-docs.deepseek.com, and set `GOOSE_PROVIDER=custom_deepseek`, `GOOSE_MODEL=$DS_MODEL`,
`DEEPSEEK_API_KEY`, `GOOSE_DISABLE_KEYRING=1`. Goalcup passes `HTTPS_PROXY`, `HTTP_PROXY`,
`NO_PROXY=localhost,127.0.0.1`, and trust as **both** `GOOSE_CA_CERT_PATH=<run CA>` and
`SSL_CERT_FILE=<system roots + run CA>` (the second covers non-provider clients; it must be a full
bundle because it replaces the roots). Prompt: "Use 1 subagent to list the files in this folder and
write the list to FILES.md, then tell me what it found." Run `goose run -t "<prompt>"` and also
`goose session` in tmux. Check: the proxy saw POSTs to `api.deepseek.com/chat/completions` from both
the main agent and the subagent (two different system prompts); `sessions.db` has one `user` row and
one `sub_agent` row; nothing failed TLS; nothing went direct. Blind spots: a participant who sets
`GOOSE_CA_CERT_PATH` themselves, uses `--no-session`, or uses an MCP extension that opens its own
connections; the 5-minute subagent timeout in a week-long run.

### H11. Aider

#### H11.1 Version, install, headless

| Item | Fact | Source |
|---|---|---|
| Latest | 0.86.2, uploaded to PyPI 12 Feb 2026. Before that 0.86.1 (13 Aug 2025). `main` last moved 22 May 2026 (a merge of a model-list update). **Stale**: no release in 8.5 months. Still 65,827 PyPI downloads a week (appendix D). Python `>=3.10,<3.13`. | https://pypi.org/pypi/aider-chat/json ; `git ls-remote https://github.com/Aider-AI/aider` (29 Oct 2026) |
| Runtime | Python. Model calls go through LiteLLM 1.81.10 (pinned), which uses httpx; Aider's own side calls use `requests`. | https://github.com/Aider-AI/aider/blob/v0.86.2/requirements.txt (lines 136, 184, 338) |
| Install on Linux | `python -m pip install aider-install && aider-install`, or `uv tool install --python 3.12 aider-chat==0.86.2`. | https://aider.chat/docs/install.html (not fetched; general knowledge, unverified) |
| Headless | `aider --message "<prompt>" --yes-always` runs one turn and exits (`--message-file` too). Chat mode is the default. Aider is **not** an autonomous loop: it does one edit turn per user message. | https://github.com/Aider-AI/aider/blob/v0.86.2/aider/args.py#L639-L652 , #L760 |

#### H11.2 Keys and DeepSeek

- Bring your own key: yes, any LiteLLM provider. Keys from env vars, `--api-key deepseek=<key>`,
  `.aider.conf.yml`, or a `.env` file in the git root (loaded by default:
  https://github.com/Aider-AI/aider/blob/v0.86.2/aider/args.py#L801-L805).
- DeepSeek: native, through LiteLLM. `DEEPSEEK_API_KEY` plus `--model deepseek/<model id>` (docs:
  https://aider.chat/docs/llms/deepseek.html , 29 Oct 2026). With only `DEEPSEEK_API_KEY` set,
  Aider picks DeepSeek by itself (`aider/onboarding.py` line 65). Its `deepseek` alias points at an
  older model id (`aider/models.py` line 100); whether that id still answers is unverified, so pass
  the current `<cheap model id>` from api-docs.deepseek.com explicitly.
- Wire detail: LiteLLM 1.81.10 sends DeepSeek calls to `https://api.deepseek.com/beta` unless
  `DEEPSEEK_API_BASE` is set (`litellm/llms/deepseek/chat/transformation.py` lines 101-102 and 120).
  Same host, `/beta/chat/completions` path. The proxy must not filter on path.
- Own account needed: no. Test key: DeepSeek.

#### H11.3 Proxy and CA

| Question | Answer | Source |
|---|---|---|
| `HTTPS_PROXY` for model calls | Likely yes. Aider calls `litellm.completion` (sync). LiteLLM's sync handler builds `httpx.Client` with no custom transport, so httpx's default `trust_env` reads `HTTPS_PROXY`. The async aiohttp path also trusts env unless `DISABLE_AIOHTTP_TRUST_ENV`. | LiteLLM 1.81.10 `http_handler.py` lines 900-930 and 1172-1182; `aiohttp_transport.py` line 313 |
| `HTTPS_PROXY` for everything else | Yes: `requests` reads it. Side calls: `pypi.org` (version check), `raw.githubusercontent.com` (LiteLLM model prices JSON), `openrouter.ai` (only for OpenRouter models), PostHog `us.i.posthog.com` (analytics, if opted in). | https://github.com/Aider-AI/aider/blob/v0.86.2/aider/versioncheck.py#L77 ; https://github.com/Aider-AI/aider/blob/v0.86.2/aider/models.py#L150-L153 ; https://github.com/Aider-AI/aider/blob/v0.86.2/aider/analytics.py#L55-L57 |
| Trust a custom CA | Two variables, both **replace** mode. `SSL_CERT_FILE` for model calls: LiteLLM uses it instead of certifi when set. `REQUESTS_CA_BUNDLE` for Aider's `requests` side calls (requests ignores `SSL_CERT_FILE`; general knowledge). Point both at a bundle of system roots plus the run CA. The system store alone is **not** enough (certifi is the default). | LiteLLM 1.81.10 `http_handler.py` lines 169-271 |
| HTTP/2, WebSockets, gRPC, pinning | None. httpx without the `h2` package speaks HTTP/1.1 only (`h2` is not in `requirements.txt`). No pinning. `--no-verify-ssl` exists and turns verification off (a participant could use it; it does not skip the proxy). | `requirements.txt`; https://github.com/Aider-AI/aider/blob/v0.86.2/aider/main.py#L519-L527 |
| Paths that skip the proxy | None found. | |
| Tracker | #664 "Add an option to skip SSL verification for users behind firewalls" (opened 10 Jun 2024, 32 comments). #5572 "Uncaught SSLError" (15 Aug 2026, Windows certificate store, not proxy). | https://github.com/Aider-AI/aider/issues/664 ; https://github.com/Aider-AI/aider/issues/5572 (29 Oct 2026) |

#### H11.4 Subagents

None. Aider has no subagent or task tool. It does make extra model calls a reader could mistake
for agents: the "weak model" (commit messages, chat summaries) and, in `--architect` mode, an
architect model plus an editor model. All run in the one Python process and go through the same
proxy. Asking for a subagent in the prompt will just get a normal single-agent answer.

#### H11.5 Session files

- `.aider.chat.history.md` (Markdown) and `.aider.input.history` **in the git root of the project**,
  plus `.aider.tags.cache.v4/` (repo map cache). `--llm-history-file` (off by default) logs raw LLM
  messages. Risk: these sit in the folder the agent may commit and push.
  (https://github.com/Aider-AI/aider/blob/v0.86.2/aider/args.py#L278-L300 ; `aider/repomap.py` line 43)
- `~/.aider/`: `caches/versioncheck`, `analytics.json`, and `oauth-keys.env` (holds an OpenRouter key
  if the user logged in that way: https://github.com/Aider-AI/aider/blob/v0.86.2/aider/main.py#L370).
- Holds keys: `.env` in the git root, `.aider.conf.yml`, `~/.aider/oauth-keys.env`.

#### H11.6 Auto-update

`--check-update` is on by default: once a day it asks PyPI, and if a newer version exists it offers
`pip install aider-chat`. **With `--yes-always` it answers yes by itself**, installs and exits
("Re-run aider to use new version"). Path: `check_version` calls `install_upgrade`
(https://github.com/Aider-AI/aider/blob/v0.86.2/aider/versioncheck.py#L109-L113), which asks
`confirm_ask` (`aider/utils.py` line 314), which returns yes when `self.yes is True`
(https://github.com/Aider-AI/aider/blob/v0.86.2/aider/io.py#L865-L866). Today nothing newer than
0.86.2 exists, so this does not fire, but a release during Goal Cup would. Off switch:
`--no-check-update` or `AIDER_CHECK_UPDATE=false`. Analytics: `--no-analytics` (or
`--analytics-disable`); otherwise 10% of users are asked to opt in.

#### H11.7 Verdict

Proxy-ready: **likely**. DeepSeek-ready: **yes** (native). Subagents: **no**.

Test-pad recipe (unverified): `uv tool install --python 3.12 aider-chat==0.86.2`. Make an empty git
repo. Set `DEEPSEEK_API_KEY` in the env (not in a `.env` in the repo). Goalcup passes `HTTPS_PROXY`,
`HTTP_PROXY`, `NO_PROXY=localhost,127.0.0.1`, `SSL_CERT_FILE=<system roots + run CA>` and
`REQUESTS_CA_BUNDLE=<same bundle>`. Run `aider --model deepseek/$DS_MODEL --no-check-update
--no-analytics --yes-always --message "Use a subagent to write hello.py that prints hello, then run
it."`, and once in chat mode in tmux. Check: the proxy saw POSTs to
`api.deepseek.com/beta/chat/completions` for the main edit and for the weak-model commit message;
no TLS errors; no request to `pypi.org`. Expect no subagent. Blind spots: Aider is a chat tool,
not a long autonomous agent, so a Goal Cup run is many user turns; the history files land in the
repo folder; a participant with `--yes-always` and update checks on can be upgraded mid-run.
Aider is stale, so low priority.

### H12. Kilo CLI (`@kilocode/cli`, binary `kilo`)

#### H12.1 Version, install, headless

| Item | Fact | Source |
|---|---|---|
| Latest | GitHub tag v7.8.25 (28 Oct 2026, 20:43 UTC). npm dist-tags on 29 Oct: `latest` = 7.8.18 (26 Oct), `rc` = 7.8.25, plus a separate `kilo-v2` line (0.0.0-v2-13, 29 Oct). Cadence: 14 builds from 15 to 28 Oct, four on 28 Oct alone. 46,073 npm downloads in the week 21 to 27 Oct. | https://registry.npmjs.org/@kilocode%2Fcli ; https://api.npmjs.org/downloads/point/2026-10-21:2026-10-27/@kilocode/cli (29 Oct 2026) |
| Runtime | Bun-compiled native binary per platform (`@kilocode/cli-linux-x64` and others, including `-musl` and `-baseline`). The npm `bin/kilo` is a small Node launcher that spawns the native binary with the same env. An OpenCode fork. | https://github.com/Kilo-Org/kilocode/blob/v7.8.25/packages/opencode/bin/kilo#L25-L27 |
| Install on Linux | `npm install -g @kilocode/cli@7.8.25` (pin; `latest` lags). Or a release tarball, for example `kilo-linux-x64-baseline.tar.gz`. | `packages/kilo-docs` install partial; https://github.com/Kilo-Org/kilocode/releases |
| Headless | `kilo run --auto "<prompt>"`. Without `--auto` a non-interactive run auto-rejects permission requests and exits 1. Chat mode: `kilo` opens the TUI. | https://github.com/Kilo-Org/kilocode/blob/v7.8.25/packages/kilo-docs/pages/code-with-ai/platforms/cli.md (lines 575-621) |

#### H12.2 Keys and DeepSeek

- Bring your own key: yes. Kilo's own gateway (`api.kilo.ai`, `ai-gateway.kilo.ai`) is one provider
  among many and needs a Kilo account; BYOK providers do not.
- DeepSeek: native provider from the models.dev catalog. `DEEPSEEK_API_KEY` in the env, or
  `kilo auth login`, or config. Config file `~/.config/kilo/kilo.json` (or `./kilo.json`):
  `"model": "deepseek/<model id>"`. (https://github.com/Kilo-Org/kilocode/blob/v7.8.25/packages/kilo-docs/pages/ai-providers/deepseek.md)
  The doc names older model ids; list current ones with `kilo models deepseek` and pick the cheap model id.
- Set `small_model` to the same DeepSeek model. The OpenCode sheet measured that an unknown model id
  silently fell back to a paid gateway model and that the default title model was paid; Kilo shares
  that code shape (unverified for Kilo).
- Own account needed: no. Test key: DeepSeek.

#### H12.3 Proxy and CA

| Question | Answer | Source |
|---|---|---|
| `HTTPS_PROXY` for model calls | Likely yes: Bun's `fetch` honours it on its own (measured for OpenCode 1.19.4 on 28 Oct; not measured for Kilo). | OpenCode facts sheet, section 6 (`git show origin/opencode-openclaw:docs/harnesses/opencode-cli.md`) |
| `HTTPS_PROXY` for everything else | Likely yes, same `fetch`. Other hosts: `models.dev` (catalog; `KILO_DISABLE_MODELS_FETCH`), npm registry (auto-update and provider packages), PostHog `us.i.posthog.com` (telemetry, **on by default**), `api.kilo.ai` (only when logged in to Kilo). | https://github.com/Kilo-Org/kilocode/blob/v7.8.25/packages/core/src/models-dev.ts#L171 ; https://github.com/Kilo-Org/kilocode/blob/v7.8.25/packages/kilo-telemetry/src/client.ts#L8-L9 |
| Trust a custom CA | The binary is compiled with `--use-system-ca`, so it trusts the OS store. `NODE_EXTRA_CA_CERTS` should add to it (Bun supports it; measured for OpenCode, extend mode). `SSL_CERT_FILE` worked for OpenCode; with `--use-system-ca` its effect on Kilo is unverified. Best: put the run CA in the system store **or** set `NODE_EXTRA_CA_CERTS`. | https://github.com/Kilo-Org/kilocode/blob/v7.8.25/packages/opencode/script/build.ts#L348 |
| HTTP/2, WebSockets, gRPC, pinning | Model calls are plain HTTPS `fetch` (HTTP/1.1 through a proxy, as measured for OpenCode). Remote sessions use a WebSocket (`kilo-sessions/remote-ws.ts`), only with a Kilo login. No pinning. PR #15365 (merged 22 Oct) blocks a set of compromised public CA chains by upgrading the bundled Bun; it does not touch custom CAs, per the PR text. | https://github.com/Kilo-Org/kilocode/pull/15365 (29 Oct 2026) |
| Paths that skip the proxy | None found. The TUI talks to its own local HTTP server, so `NO_PROXY=localhost,127.0.0.1` is needed (as OpenCode). Session upload to Kilo cloud runs only with a `kilo` credential. | https://github.com/Kilo-Org/kilocode/blob/v7.8.25/packages/opencode/src/kilo-sessions/kilo-sessions.ts#L202-L207 |
| Tracker | No proxy or custom-CA issue found on the first page of a "proxy cli" search (172 results). | https://github.com/Kilo-Org/kilocode/issues?q=proxy%20cli (29 Oct 2026) |

#### H12.4 Subagents

- Yes. The `task` tool starts a child session with `parentID` set
  (https://github.com/Kilo-Org/kilocode/blob/v7.8.25/packages/opencode/src/tool/task.ts#L225-L233).
  The code, plan and debug agents "support subagents natively"; built-in subagents include
  `general` and `explore`. The Orchestrator mode is deprecated.
- How to ask: "Use the task tool to start one general subagent that ...".
- In-process: child sessions run inside the same Bun process, same `fetch`, same proxy and CA.
  Shell tools are child processes and inherit the env. Parallel subagents: `KILO_ENABLE_PARALLEL`
  (experimental).

#### H12.5 Session files

- One SQLite database: `~/.local/share/kilo/kilo.db` for `latest`/`rc` builds (other channels use
  `kilo-<channel>.db`); `KILO_DB` moves it
  (https://github.com/Kilo-Org/kilocode/blob/v7.8.25/packages/core/src/database/database.ts#L49-L60).
  Subagents are child `session` rows in the same database (OpenCode shape; table names unverified
  for Kilo).
- Logs `~/.local/share/kilo/log/`, cache `~/.cache/kilo/`, state `~/.local/state/kilo/`, temp
  `/tmp/kilo` (https://github.com/Kilo-Org/kilocode/blob/v7.8.25/packages/core/src/global.ts#L12-L36).
- Holds keys: `~/.local/share/kilo/auth.json`, and `kilo.json` if a key is written there.

#### H12.6 Auto-update

On by default for npm, yarn, pnpm and bun installs. At startup it asks the npm registry for the
dist-tag of its build channel; a newer **patch** is installed silently, a minor or major only
shows a notice. Off: `KILO_DISABLE_AUTOUPDATE=1` or `"autoupdate": false` in `kilo.json`
(`"notify"` only notifies).
(https://github.com/Kilo-Org/kilocode/blob/v7.8.25/packages/opencode/src/cli/upgrade.ts#L8-L46 ;
https://github.com/Kilo-Org/kilocode/blob/v7.8.25/packages/opencode/src/installation/index.ts#L274)
Unverified risk: with `latest` behind `rc`, a run on 7.8.25 might be moved to 7.8.18. Pin and turn
updates off.

#### H12.7 Verdict

Proxy-ready: **likely**. DeepSeek-ready: **yes** (native). Subagents: **yes** (in-process).

Test-pad recipe (unverified): `npm install -g @kilocode/cli@7.8.25`. Write `~/.config/kilo/kilo.json`
with `"model": "deepseek/<cheap model id>"`, `"small_model"` the same, `"autoupdate": false`,
`"share": "disabled"`, `"permission": "allow"`; set `DEEPSEEK_API_KEY`. Goalcup passes
`HTTPS_PROXY`, `HTTP_PROXY`, `NO_PROXY=localhost,127.0.0.1` and `NODE_EXTRA_CA_CERTS=<run CA>`
(plus the run CA in the system store if goalcup already does that). The pad also sets
`KILO_DISABLE_AUTOUPDATE=1` and, to cut noise, `KILO_TELEMETRY_LEVEL=off`
(https://github.com/Kilo-Org/kilocode/blob/v7.8.25/packages/kilo-telemetry/src/telemetry.ts#L125-L127).
Prompt: "Use the task tool to start one general subagent that lists the files here and writes
them to FILES.md. Then summarise its answer." Run `kilo run --auto "<prompt>"` and the TUI in tmux.
Check: POSTs to `api.deepseek.com/chat/completions` from parent and child; `kilo.db` has a session
row with a parent id; no PostHog or npm calls if those are off; no TLS errors. Blind spots: Kilo
ships several builds a day, so facts drift fast; the 2.x line (`kilo-v2`) may change the session
store; a participant logged in to Kilo sends model calls to the Kilo gateway (still through the
proxy, recorded, but a different wire).

### H13. Qwen Code (Alibaba, npm `@qwen-code/qwen-code`, binary `qwen`)

#### H13.1 Version, install, headless

| Item | Fact | Source |
|---|---|---|
| Latest | 0.26.4, 29 Oct 2026 14:41 UTC (dist-tag `latest`). Also `preview` and a daily `nightly`. | https://registry.npmjs.org/@qwen-code%2Fqwen-code (29 Oct 2026) |
| Cadence | 10 stable releases from 29 Sep to 29 Oct. Five in the last 9 days (0.26.0 on 21 Oct to 0.26.4 on 29 Oct). Expect 3 to 6 more before 7 Nov. | npm `time` field |
| Install on Linux | `npm install -g @qwen-code/qwen-code@0.26.4` (Node 22 or newer, `engines.node >=22.0.0`). Or the standalone script, which bundles its own Node: `curl -fsSL https://qwen-code-assets.oss-cn-hangzhou.aliyuncs.com/installation/install-qwen-standalone.sh \| bash`. | https://github.com/QwenLM/qwen-code/blob/v0.26.4/README.md#L39-L62 |
| Headless | `qwen -p "<prompt>"` (or piped stdin). `--yolo` or `--approval-mode yolo` for an unattended run. `--output-format json` and `stream-json` exist. Interactive TUI is how people use it (point 7). | https://github.com/QwenLM/qwen-code/blob/v0.26.4/docs/users/features/headless.md#L19-L59 , `#L291-L326` |

#### H13.2 Keys and DeepSeek

- Bring your own key: yes. Qwen's own free sign-in (Qwen OAuth) was discontinued on 15 Apr 2026, so
  everyone brings a key now (`docs/users/configuration/auth.md#L10-L12` at v0.26.4).
- DeepSeek: **native preset**. `/auth` then "Third-party Providers" then "DeepSeek API Key". The preset is
  OpenAI-format, base URL `https://api.deepseek.com`, key env `DEEPSEEK_API_KEY`
  (https://github.com/QwenLM/qwen-code/blob/v0.26.4/packages/core/src/providers/presets/deepseek.ts#L10-L49).
  Qwen also knows DeepSeek's reasoning-effort fields and sends `thinking: {type: disabled}` only to a real
  `api.deepseek.com` host (`docs/users/configuration/model-providers.md#L882`, `#L894`).
- Scripted setup (no wizard): `~/.qwen/settings.json` with a `modelProviders.openai[]` entry
  (`baseUrl: https://api.deepseek.com/v1`, `envKey: DEEPSEEK_API_KEY`), `security.auth.selectedType: "openai"`
  and `model.name`. Shape: `docs/users/configuration/auth.md#L54-L82`; a DeepSeek entry is at
  `docs/users/configuration/model-providers.md#L856-L873`. Env-only also works:
  `OPENAI_API_KEY` + `OPENAI_BASE_URL` + `OPENAI_MODEL` (`auth.md#L226`).
- Test key: **DeepSeek**.

#### H13.3 Proxy and CA

| Question | Answer | Source |
|---|---|---|
| Runtime | Node (default ink TUI). Bun is bundled only for the opt-in OpenTUI renderer (`QWEN_TUI_RENDERER=opentui`). On Linux the entry script runs in-process or relaunches itself as a Node child with the same env. | `packages/cli/src/llm.tsx#L1468-L1485` ; `scripts/cli-entry.js#L1-L30`, `#L469-L505` (v0.26.4) |
| `HTTPS_PROXY` | Honoured. Order: `--proxy` flag, then `settings.proxy`, then `HTTPS_PROXY`, `https_proxy`, `HTTP_PROXY`, `http_proxy`. Qwen installs a process-wide undici `EnvHttpProxyAgent`, so `NO_PROXY` is honoured. HTTP and HTTPS proxy URLs only, no SOCKS. | https://github.com/QwenLM/qwen-code/blob/v0.26.4/packages/cli/src/config/config.ts#L2717-L2723 ; `packages/core/src/config/config.ts#L3889-L3916` ; `packages/core/src/utils/proxyUtils.ts#L24-L65` |
| Everything else | Model calls (OpenAI and Anthropic SDKs) use a shared proxy dispatcher (`runtimeFetchOptions.ts#L229-L270`). MCP HTTP uses the same proxy (`#L290-L330`). Usage stats go to `gb4w8c3ygj-default-sea.rum.aliyuncs.com` through the same proxy (`packages/core/src/telemetry/qwen-logger/qwen-logger.ts#L83-L90`, `#L1270-L1283`). The model catalog reads `https://models.dev` (`packages/core/src/models/model-catalog.ts#L40`). Update checks spawn `npm`, which reads its own proxy config (unverified). | as listed |
| Custom CA | `NODE_EXTRA_CA_CERTS=<run CA>` (Node adds it to the bundled roots). Vendor troubleshooting says the same. A project `.env` may not set it (blocked), but the launch env may. | https://github.com/QwenLM/qwen-code/blob/v0.26.4/docs/users/support/troubleshooting.md#L18-L27 ; `packages/cli/src/config/shared-env-keys.ts#L107-L131` |
| HTTP/2, WebSockets, gRPC | undici agents default to HTTP/1.1. No WebSocket or gRPC on the model path found. | source, unverified by a run |
| Pinning | None found. `QWEN_TLS_INSECURE` / `--insecure` turns verification off (not needed). | `packages/core/src/utils/runtimeFetchOptions.ts#L80-L86` |
| Paths that skip the proxy | (a) A `proxy` key in `~/.qwen/settings.json` or `.qwen/settings.json`, or `--proxy`, **beats** `HTTPS_PROXY`. A participant's own proxy setting would send calls around goalcup. (b) `NO_PROXY` listing the provider. (c) Built-in `claude-code` and `codex` subagents launch those harnesses as child processes with their own keys and wire shapes. (d) `--sandbox` runs Qwen in a container; proxy handoff into it is unverified. (e) `qwen serve` daemon mode is opt-in. | `config.ts#L2717-L2723` ; `docs/users/features/sub-agents.md#L15-L40` |
| Tracker | No Qwen issue found about proxy plus custom CA. Docs cover corporate TLS inspection. | https://qwenlm.github.io/qwen-code-docs/en/users/support/troubleshooting/ (29 Oct 2026) |

#### H13.4 Subagents

- Yes. Tool `agent` (old name `task`); default type `general-purpose`
  (`packages/core/src/tools/tool-names.ts#L33`, `#L194`; `packages/core/src/subagents/builtin-agents.ts#L17`).
  Also "fork" subagents (inherit the parent history, always background), agent teams, Arena and
  `/coordinate` (`docs/users/features/sub-agents.md#L44`, `#L101-L129`).
- Process: **in-process** for Qwen's own subagents, teams and Arena
  (`docs/users/features/arena.md#L52`, `docs/users/features/multi-agent-coordination.md#L30`). An optional
  `executionBackend: container` exists (`packages/core/src/subagents/execution-backend.ts`). The built-in
  `claude-code` and `codex` executors are child processes (env inheritance unverified).
- Named subagents run in the background by default. Headless `-p` waits for running background agents
  before it prints the final result (`packages/cli/src/nonInteractiveCli.ts#L3729-L3760`).
- Fork children cannot spawn more subagents (`sub-agents.md#L125`).
- How to ask: "Use the agent tool to start one general-purpose subagent that ...".

#### H13.5 Session files

- Main chat: JSONL at `~/.qwen/projects/<sanitized-cwd>/chats/<sessionId>.jsonl`
  (`docs/users/features/headless.md#L56-L59`). `QWEN_HOME` and `QWEN_RUNTIME_DIR` move it
  (`docs/users/configuration/settings.md#L858-L859`).
- Subagents: `<projectDir>/subagents/<sessionId>/agent-<id>.jsonl` plus `agent-<id>.meta.json` and a
  transient `.jsonl.stream` (`packages/core/src/agents/agent-transcript.ts#L8-L21`, `#L68-L75`).
- Keys: `~/.qwen/settings.json` may hold keys in its `env` block in plain text (the docs warn about it,
  `auth.md#L380`); also `~/.qwen/.env`. Chat files hold prompts and tool output, not keys (unverified).

#### H13.6 Auto-update

- Yes. On startup Qwen checks npm and can install the update, unless `general.enableAutoUpdate: false`
  in `settings.json` (default `true`). Old keys `disableAutoUpdate` and `disableUpdateNag` migrate to it.
  (`packages/cli/src/startup/startup-prefetch.ts#L157-L160`; `docs/users/configuration/settings.md#L58-L89`).
- No user env switch found. `QWEN_CODE_SKIP_UPDATE_CHECK_ONCE` is internal and one-shot
  (`packages/cli/src/utils/processUtils.ts#L18`). Whether the check runs in `-p` mode is unverified.

#### H13.7 Verdict

- Proxy-ready: **likely** (source path is clean; no run). DeepSeek-ready: **yes** (native preset).
  Subagents: **yes**, in-process.
- Test-pad recipe (unverified): install `npm install -g @qwen-code/qwen-code@0.26.4` on Node 22. Write
  `~/.qwen/settings.json` with the DeepSeek provider entry above, `security.auth.selectedType: "openai"`,
  `model.name: "<DeepSeek model id>"` and `general.enableAutoUpdate: false`, with no `proxy` key (or let
  the tester use `/auth`, then "Third-party Providers", then DeepSeek, as a participant would). goalcup
  passes `HTTPS_PROXY=http://127.0.0.1:<port>`, `NODE_EXTRA_CA_CERTS=<run CA PEM>`, and the pad passes
  `DEEPSEEK_API_KEY`. Run interactive `qwen` and also `qwen -p "<prompt>" --yolo`. Prompt: "Use the agent
  tool to start one general-purpose subagent that counts the files in this folder and reports the number.
  Wait for it, then tell me the number." Check: the proxy shows chat-completion POSTs to `api.deepseek.com`
  from the parent and a second conversation with the subagent's own system prompt; no TLS errors; the
  RUM telemetry host and `models.dev` show up too; `chats/<id>.jsonl` and `subagents/<id>/agent-*.jsonl`
  exist.
- What could go wrong: a `proxy` key in the user's own settings silently wins over goalcup. Fast release
  pace (5 releases in 9 days) means the run may use a newer build than the one tested.

### H14. Crush (Charm, npm `@charmland/crush`, binary `crush`)

#### H14.1 Version, install, headless

| Item | Fact | Source |
|---|---|---|
| Latest | 0.99.0, 29 Oct 2026 16:33 UTC. Notes: Catwalk v3 provider catalog, hook events, server features. Nothing about proxy, TLS or subagents. | https://registry.npmjs.org/@charmland%2Fcrush ; https://github.com/charmbracelet/crush/releases/tag/v0.99.0 (29 Oct 2026) |
| Cadence | 6 releases from 29 Sep to 29 Oct (0.97.0 to 0.99.0). About one a week. | npm `time` field |
| Install on Linux | `npm install -g @charmland/crush@0.99.0` (postinstall downloads the matching GitHub release tarball, pinned to that version). Or the `crush_0.99.0_Linux_x86_64.tar.gz` release asset, Homebrew, Nix, AUR. | https://github.com/charmbracelet/crush/blob/v0.99.0/README.md#L24-L45 ; npm tarball `package.json`, `lib.js` |
| Headless | `crush run "<prompt>"` (also stdin). `-m provider/model`, `--small-model`, `-q`. `crush run` auto-approves every permission for its session. Interactive TUI (`crush`, `--yolo` to skip prompts) is the common use. | `internal/cmd/run.go#L33-L76`, `#L168-L174` ; `internal/app/app.go#L392-L394` ; `internal/cmd/root.go#L61` (v0.99.0) |

#### H14.2 Keys and DeepSeek

- Bring your own key: yes, for many providers via env vars or pasting a key in the model picker
  (`README.md#L196-L232`). Charm's own "Hyper" service is optional.
- DeepSeek: **native provider** in Catwalk v3: id `deepseek`, `api_key: "$DEEPSEEK_API_KEY"`,
  endpoint `https://api.deepseek.com/v1`, type `completions` (OpenAI chat format)
  (https://github.com/charmbracelet/catwalk/blob/v3.0.1/internal/providers/configs/deepseek.json#L1-L8).
  Setting `DEEPSEEK_API_KEY` is enough; pick the model with `ctrl+l` or `crush run -m deepseek/<DeepSeek model id>`.
  The README also shows a custom-provider form in the new `crushrc` (Bash) config (`README.md#L756-L773`).
- Config files: `./.crushrc`, `./crushrc`, `~/.config/crush/crushrc`; legacy `crush.json` still read;
  state in `~/.local/share/crush/crush.json` (`README.md#L286-L320`). Both config files can run shell code
  at load (`README.md#L322-L327`).
- Test key: **DeepSeek**.

#### H14.3 Proxy and CA

| Question | Answer | Source |
|---|---|---|
| Runtime | Go 1.27 static binary. | https://github.com/charmbracelet/crush/blob/v0.99.0/go.mod#L3 |
| `HTTPS_PROXY` | Honoured through Go's default transport (`ProxyFromEnvironment`). Provider clients get no custom client unless debug mode or a special provider, so the SDK default is used; the debug logger wraps `http.DefaultTransport`. Tools (`fetch`, `web_search`, `download`, `sourcegraph`) clone the default transport. Crush 0.38.0 also added OS proxy settings. | `internal/agent/coordinator.go#L1178-L1219` ; `internal/log/http.go#L15-L20` ; `internal/agent/tools/fetch.go#L47` ; https://newreleases.io/project/github/charmbracelet/crush/release/v0.38.0 |
| Everything else | Startup fetch of the provider catalog from `https://catwalk.charm.land` (`internal/config/load.go#L37`; off with `CRUSH_DISABLE_PROVIDER_AUTO_UPDATE=1`, `#L787`). Usage metrics to `https://data.charm.land` (PostHog; off with `CRUSH_DISABLE_METRICS=1` or `DO_NOT_TRACK=1`) (`internal/event/event.go#L17`; `internal/cmd/root.go#L925-L929`). Update notice from `api.github.com` (`internal/update/update.go#L14-L15`). All through the same transport. | as listed |
| Custom CA | Go on Linux: `SSL_CERT_FILE` **replaces** the default CA file and `SSL_CERT_DIR` replaces the default dirs. Pass a bundle of system roots plus the run CA, never the run CA alone. Installing the run CA in `/etc/ssl/certs` (`update-ca-certificates`) also works. No Crush-specific CA setting found. | Go `crypto/x509` rules (https://pkg.go.dev/crypto/x509#SystemCertPool); `rg RootCAs` finds nothing in `internal/` |
| HTTP/2, WebSockets, gRPC | Default transport tries HTTP/2 by ALPN; the proxy can offer only http/1.1. No WebSocket or gRPC on the model path found. | source, unverified by a run |
| Pinning | None found. | source |
| Paths that skip the proxy | (a) `CRUSH_CLIENT_SERVER=1` makes the TUI talk to a separate `crush server` process; an already-running server started without goalcup's env would not proxy (opt-in, off by default). (b) `NO_PROXY`. (c) Debug HTTP logging is opt-in. | `internal/cmd/root.go#L252-L282` |
| Tracker | No Crush issue found about proxy plus custom CA. | web search 29 Oct 2026 |

#### H14.4 Subagents

- Yes, but narrow. The `agent` tool launches a sub-agent with **read-only search tools only** (glob,
  grep, ls, view). Several can run in parallel (`fantasy.NewParallelAgentTool`). A second sub-agent sits
  behind `agentic_fetch` (web fetch summarised by a model).
  (`internal/agent/agent_tool.go#L23-L40`; `internal/agent/templates/agent_tool.md#L1`;
  `internal/agent/agentic_fetch_tool.go#L19`).
- Process: **in-process** goroutines in the same binary, same env (source).
- Each sub-agent gets a child session row with `parent_session_id`
  (`internal/session/session.go#L116-L133`; `internal/agent/coordinator.go#L1697`).
- How to ask: "Use the agent tool to start one sub-agent that searches for ... and reports back." A task
  that needs edits will not use it.

#### H14.5 Session files

- SQLite at `<project>/.crush/crush.db` (data dir defaults to `.crush`, found by walking up from the cwd).
  Tables `sessions`, `messages`, `files`. Logs at `.crush/logs/crush.log`.
  (`internal/config/config.go#L26`, `#L446-L450`; `internal/db/connect.go#L93`;
  `internal/db/migrations/20250424200609_initial.sql#L4-L59`; `internal/cmd/logs.go#L58`).
- Keys: a key pasted in the UI is written as `providers.<id>.api_key` to the global data file
  `~/.local/share/crush/crush.json`, as are OAuth tokens (`internal/config/store.go#L579-L590`;
  `internal/ui/dialog/api_key_input.go#L310`). With env-only keys nothing is written (unverified).

#### H14.6 Auto-update

- The binary does not update itself. It only shows an "update available" notice from the GitHub API
  (`internal/app/app.go#L960-L975`). The npm wrapper is pinned to one version. The provider catalog does
  update itself from Catwalk at start; `CRUSH_DISABLE_PROVIDER_AUTO_UPDATE=1` turns that off
  (`README.md#L935-L971`).

#### H14.7 Verdict

- Proxy-ready: **likely** (plain Go transport; CA rule is the one known trap). DeepSeek-ready: **yes**
  (native). Subagents: **yes**, read-only search sub-agents, in-process.
- Test-pad recipe (unverified): install `npm install -g @charmland/crush@0.99.0` before the run. No config
  file needed: the pad passes `DEEPSEEK_API_KEY`. goalcup passes `HTTPS_PROXY=http://127.0.0.1:<port>` and
  `SSL_CERT_FILE=<system roots + run CA bundle>`. Run interactive `crush` (choose DeepSeek with `ctrl+l`)
  and also `crush run -m deepseek/<DeepSeek model id> "<prompt>"`. Prompt: "Use the agent tool to start one
  sub-agent that finds every Markdown file in this folder and reports how many there are. Then tell me
  the number." Check: chat-completion POSTs to `api.deepseek.com/v1` from the parent and from a
  sub-agent with the task prompt; the title call on the small model; `catwalk.charm.land`,
  `data.charm.land` and `api.github.com` also pass through; `.crush/crush.db` has a session row with a
  `parent_session_id`.
- What could go wrong: passing only the run CA in `SSL_CERT_FILE` breaks every other host (catalog,
  metrics, MCP). A prompt that does not need search may never trigger the sub-agent, so the subagent test
  must ask for a search.

### H15. Factory Droid (npm `droid` / `@factory/cli`, binary `droid`)

#### H15.1 Version, install, headless

| Item | Fact | Source |
|---|---|---|
| Latest | 0.251.0, 27 Oct 2026 23:28 UTC. | https://registry.npmjs.org/droid (29 Oct 2026) |
| Cadence | 28 releases from 29 Sep to 29 Oct. About one a day. Expect 6 to 8 more before 7 Nov. | npm `time` field |
| Install on Linux | `npm install -g droid@0.251.0` (per-platform binary package; auto-update off at build time). Or `curl -fsSL https://app.factory.ai/cli \| sh` (auto-update on). Docs ask Linux users to install `xdg-utils` (browser sign-in). | https://docs.factory.com/droid-cli/quickstart.md ; https://docs.factory.com/droid-cli/cli-reference.md (Installing a specific version) |
| Headless | `droid exec "<prompt>"`. Read-only by default; `--auto low\|medium\|high` or `--skip-permissions-unsafe`. `-o json` / `stream-json`. `--model custom:<Display-Name>-<index>` picks a BYOK model. | https://docs.factory.com/droid-exec/overview.md (Autonomy, Custom models) |

#### H15.2 Keys and DeepSeek

- Bring your own key: yes. `customModels[]` in `~/.factory/settings.json`, fields `model`, `displayName`,
  `baseUrl`, `apiKey` (accepts `${VAR}`), `provider` (`anthropic`, `openai`,
  `generic-chat-completion-api`, `bedrock-converse`). Docs say keys stay local and are not uploaded.
  https://docs.factory.com/model-independence/byok.md (29 Oct 2026).
- DeepSeek: **OpenAI-compatible base URL**, `provider: "generic-chat-completion-api"`,
  `baseUrl: "https://api.deepseek.com/v1"`, `apiKey: "${DEEPSEEK_API_KEY}"`. No DeepSeek example in the
  docs; the Anthropic-format endpoint with `provider: "anthropic"` may also work (unverified).
- **Also needs a Factory account.** Interactive `droid` signs in through the browser; `droid exec` needs
  `FACTORY_API_KEY=fk-...` from the Factory settings page (droid-exec docs, Installation). Third-party
  guides say a free plan covers BYOK only (unverified). A login-free mode exists only in the airgap
  build, sold to enterprises (https://docs.factory.com/enterprise/airgapped-deployment.md). The public
  binary does read `FACTORY_AIRGAP_ENABLED` (binary), but using it is not how participants run Droid and
  is untested.
- Test key: **DeepSeek** for inference, plus a free Factory account and its `FACTORY_API_KEY` (a second
  secret goalcup must mask; it is sent to `api.factory.ai`).

#### H15.3 Proxy and CA

| Question | Answer | Source |
|---|---|---|
| Runtime | Bun 1.4.4 single-file executable (324 MB, ELF x86-64). | binary strings `Bun/1.4.4`, `/$bunfs/root/...` |
| `HTTPS_PROXY` | Honoured. Docs: "Droid respects standard proxy environment variables" `HTTPS_PROXY`, `HTTP_PROXY`, `NO_PROXY`. The binary's own help text says Droid ignores OS proxy and PAC settings and uses only those env vars. | https://docs.factory.com/enterprise/network-and-deployment.md (Proxies, custom CAs, and mTLS) ; binary |
| Everything else | Same Bun fetch for Factory hosts: `api.factory.ai` (auth, org settings, telemetry, cloud session sync, default on: `cloudSessionSync: true`), `app.factory.ai`, `downloads.factory.ai` (standalone updates). Docs say BYOK calls go to `baseUrl` and keys stay local; that inference does not pass through Factory is likely but unverified. | https://docs.factory.com/droid-cli/settings.md (Cloud session sync) ; binary config `apiBaseUrl: "https://api.factory.ai"` |
| Custom CA | `NODE_EXTRA_CA_CERTS` or `FACTORY_EXTRA_CA_CERTS` (both path lists). At start Droid merges bundled, system (`.crt`/`.pem` files and dirs) and extra CAs, then patches `tls.createSecureContext`, the https global agent and `globalThis.fetch` to use that list. It also writes a CA bundle for child processes. Cap of 4096 entries. Debug with `FACTORY_DEBUG_CERTS=1`. | docs (as above, "via `NODE_EXTRA_CA_CERTS` or OS-level trust stores") ; binary |
| HTTP/2, WebSockets, gRPC | Bun fetch; HTTP/1.1 expected (unverified). WebSockets appear for the daemon relay and Remote Control, not for BYOK model calls (unverified). | binary strings `daemon.relay_connect` |
| Pinning | None found. | binary |
| Paths that skip the proxy | (a) The interactive TUI runs sessions through a background daemon. When the TUI starts it as a child ("IPC transport: parent process") it inherits the env; a separately started `droid daemon` or the Factory desktop app would not carry goalcup's env (unverified). (b) Cloud "Droid Computers" and Missions run on Factory's side and are out of reach. (c) `NO_PROXY`. (d) Droid's own sandbox has a network policy with `httpProxyPort`; whether it touches model calls is unverified. | binary strings `ensureDaemonSession`, `~/.factory/logs/daemon-stderr.log` ; https://docs.factory.com/droid-cli/cli-reference.md (`droid daemon`) |
| Tracker | No proxy-plus-CA issue found. Relevant: issue 691, "subagents fail on every BYOK model", Free plan, `model: inherit`; subagent returns nothing or 402 and the parent does the work itself. Status unknown; GitHub returned 404 to a plain fetch, read via a mirror. | https://gh.r-mj.com/Factory-AI/factory/issues/691 (mirror, 29 Oct 2026; unverified) |

#### H15.4 Subagents

- Yes. The `Task` tool runs built-in droids `worker` and `explorer`, or custom droids from
  `.factory/droids/*.md` and `~/.factory/droids/*.md`. Foreground by default; `run_in_background: true`
  for parallel work. Subagents cannot spawn subagents.
  https://docs.factory.com/harness/subagents.md (29 Oct 2026).
- A subagent's model is `inherit` by default, but `/settings` can route Light, Medium and Heavy tiers to
  other models, which may be Factory-managed models. With BYOK only, those would fail or be refused.
- Process: unverified. Live progress streaming points to in-process; `DROID_PARENT_SESSION_ID` is read
  from the env for telemetry only (binary).
- How to ask: "Use the Task tool to run the explorer subagent to ...".

#### H15.5 Session files

- JSONL at `~/.factory/sessions/<project-slug>/<uuid>.jsonl` with a sibling `<uuid>.settings.json`; side
  chats under `sessions/<slug>/btw/` (binary). Subagent file layout unverified.
- Also `~/.factory/logs/`, `~/.factory/certs` (CA cache), `~/.factory/updates`.
- Keys: `apiKey` in `settings.json` if written literally (use `${DEEPSEEK_API_KEY}`). The Factory login
  token goes to the OS keyring, with a file fallback when `FACTORY_DISABLE_KEYRING` is set (binary,
  unverified). Cloud session sync uploads transcripts to Factory unless `cloudSessionSync: false`.

#### H15.6 Auto-update

- npm builds: off at build time. Standalone installs: on; switch off with
  `FACTORY_DROID_AUTO_UPDATE_ENABLED=false` in the env of every Droid process, daemon included.
  Enterprise admins have `disableAutoUpdate`.
  https://docs.factory.com/droid-cli/cli-reference.md (Auto-updates); binary `FACTORY_DROID_AUTO_UPDATE_ENABLED` check.

#### H15.7 Verdict

- Proxy-ready: **likely** (documented env proxy and CA; no run). DeepSeek-ready: **yes, but needs a
  Factory account** (free) as well. Subagents: **yes**, but with BYOK they may fail (issue 691).
- Test-pad recipe (unverified): install `npm install -g droid@0.251.0`. Create a free Factory account and
  a Factory API key. Write `~/.factory/settings.json` with one custom model:
  `{"customModels":[{"model":"<DeepSeek model id>","displayName":"DeepSeek","baseUrl":"https://api.deepseek.com/v1","apiKey":"${DEEPSEEK_API_KEY}","provider":"generic-chat-completion-api","maxOutputTokens":8192}]}`.
  The pad passes `DEEPSEEK_API_KEY` and `FACTORY_API_KEY`; goalcup passes `HTTPS_PROXY=http://127.0.0.1:<port>`
  and `NODE_EXTRA_CA_CERTS=<run CA PEM>` (add `FACTORY_EXTRA_CA_CERTS` with the same path if the first
  try fails). Run interactive `droid` (select the custom model with `/model`) and
  `droid exec --model "custom:DeepSeek-0" --auto high "<prompt>"`. Prompt: "Use the Task tool to run one
  explorer subagent that counts the Markdown files in this folder. Wait for it and report its number."
  Check: chat-completion POSTs to `api.deepseek.com` from the parent and the subagent; whether the
  subagent call ends in a 402 or never appears (issue 691); calls to `api.factory.ai` also recorded;
  `~/.factory/sessions/<slug>/<uuid>.jsonl` exists.
- What could go wrong: the Factory account is a second gate a participant may not have; the subagent bug
  can leave point 2 untestable on BYOK; daily releases; the daemon may outlive the run.

## O.8 Corrections to earlier appendices

### O.8.1 Claude Code, Codex, Hermes (refresh of appendix D, D1 to D3)

| Harness | Morning (appendix D) | Now (19:16 UTC) | Source |
|---|---|---|---|
| Claude Code | 2.1.312 latest (27 Oct), 2.1.308 stable | Same. No new release. | https://registry.npmjs.org/@anthropic-ai%2Fclaude-code (fetched 29 Oct 2026) |
| Codex CLI | 0.165.0 stable (27 Oct), 0.166.0-alpha.6 | Same. No 0.166.0 stable tag yet. | https://registry.npmjs.org/@openai%2Fcodex ; `git ls-remote --tags https://github.com/openai/codex` (29 Oct 2026) |
| Hermes Agent | v0.22.5 stable (22 Oct), main at `a938f00f0f` | Same release. Main moved 23 commits to `67d47601bc` (29 Oct 14:37 -0400). None touch TLS trust, proxy or delegation config (`agent/ssl_verify.py` unchanged; `tools/delegate_tool.py` +50/-8 lines; no delegation default changed in `config_defaults.py`). Latest canary still `v0.22.5+canary.20261029T064840Z`. | `git ls-remote https://github.com/NousResearch/hermes-agent` ; `git diff v0.22.5 origin/main` on a scratch clone |

The bigger change is the design, not the harnesses. Appendix D assumed goalcup writes Hermes's
`providers.recorder` entry with `base_url` and `ssl_ca_cert` (`connectors/hermes-cli/harness.toml:145-149`).
That entry sets a model base URL, so under the AGENTS.md rule it goes. Hermes must then trust the run
CA some other way (section H3.3). The good news: if the proxy intercepts every host, appendix D's
"silent unrecorded Hermes route" (D3.3.3) mostly closes, because one process-wide trust setting now
covers every host Hermes calls.

### O.8.2 Goose, Aider, Kilo (appendix D4)

| Appendix D said | Now | Evidence |
|---|---|---|
| Goose CA handling "unknown"; "rustls with bundled roots would refuse our CA" (D4.4) | Goose uses reqwest 0.13 with the `rustls` feature. reqwest 0.13 uses the platform verifier by default, which on Linux reads the system CA bundle. Goose also has its own `GOOSE_CA_CERT_PATH` setting that adds a CA to every model provider client. So Goose can trust the run CA. | reqwest changelog 0.13.0, https://github.com/seanmonstar/reqwest/blob/master/CHANGELOG.md ; rustls-platform-verifier docs, https://docs.rs/rustls-platform-verifier/0.7.0/rustls_platform_verifier/ ; https://github.com/aaif-goose/goose/blob/v1.57.0/crates/goose/src/config/tls.rs#L32-L35 (all 29 Oct 2026) |
| Aider: "LiteLLM uses its own bundle (unverified)" | True by default (certifi), but LiteLLM switches to the file in `SSL_CERT_FILE` when it is set. Aider's own side calls use `requests`, which reads `REQUESTS_CA_BUNDLE`, not `SSL_CERT_FILE`. | LiteLLM 1.81.10 wheel, `litellm/llms/custom_httpx/http_handler.py` lines 169-271 (read 29 Oct 2026) |
| Kilo: "`KILO_DATA_HOME` moves data (third-party)" | No such variable in the source. Data follows `XDG_DATA_HOME`; `KILO_DB` moves the database only. | https://github.com/Kilo-Org/kilocode/blob/v7.8.25/packages/core/src/global.ts#L12-L22 ; https://github.com/Kilo-Org/kilocode/blob/v7.8.25/packages/core/src/database/database.ts#L49-L60 |
| Kilo auto-update "unverified" | On by default for npm installs: it installs patch releases on its own. Off switch exists. | https://github.com/Kilo-Org/kilocode/blob/v7.8.25/packages/opencode/src/cli/upgrade.ts#L8-L46 |

### O.8.3 Qwen Code, Crush, Factory Droid (appendix D4)

| Appendix D said (`docs/plans/2026-10-30-audit/D-harnesses.md:115`, `:1220-1225`, `:1523-1525`) | Now |
|---|---|
| Qwen 0.26.3 latest (28 Oct) | 0.26.4, 29 Oct 2026 14:41 UTC |
| Crush 0.98.2 latest (23 Oct) | 0.99.0, 29 Oct 2026 16:33 UTC |
| Droid "native binary", runtime unverified | A Bun 1.4.4 single-file executable (binary: strings `Bun/1.4.4`, `bun-v1.4.4`) |
| Droid "has `disableAutoUpdate`" | That setting is an enterprise org control. The user switch is the env var `FACTORY_DROID_AUTO_UPDATE_ENABLED=false`. npm builds ship with auto-update off. |
| Qwen, Crush, Droid session formats unverified | Qwen: JSONL. Crush: SQLite. Droid: JSONL (details below). |

### O.8.4 Codex and DeepSeek

Part O1 said Codex is not DeepSeek-ready because it speaks only the Responses API. DeepSeek now offers a
Responses API and publishes a Codex guide (https://api-docs.deepseek.com/guides/responses_api ;
https://api-docs.deepseek.com/quick_start/agent_integrations/codex , fetched 29 Oct 2026). So Codex is
DeepSeek-ready. The plan does not change: Codex is tested with the OpenAI key. Appendix E also said the
DeepSeek status page showed no incidents; it shows uptime figures and the 15 Oct outage (O.5.3).

## O.9 What could go wrong, and what is still unverified

### O.9.1 Claude Code, Codex, Hermes

- Claude Code: `DISABLE_UPDATES=1` and `CLAUDE_CODE_DISABLE_AGENT_VIEW=1` change how some people use it
  (point 7); for the test pad that is fine, for participants it must be said plainly (point 6).
- Codex: `--no-daemon` is unknown before 0.156.1. If the proxy cannot relay a WebSocket upgrade, Codex
  quietly falls back to HTTPS for the session; the record then looks fine but the WebSocket path, which
  ChatGPT sign-in users get, stays untested.
- Hermes: a bundle with system roots plus the run CA trusts the run CA for every Python stack in the
  process, including tools the agent runs; that is the intent, but it must be per-run and removed after.
  If OpenSSL in the uv-built Python looks in a different default directory, the run CA alone might
  still work by accident through `SSL_CERT_DIR` defaults; test both ways.
- All three: the facts are from source and docs, not from runs through the new proxy. The 2-hour run
  report (`docs/research/results/2026-10-30-two-hour-run.md`) does not exist on any branch yet.

#### Unverified

- Claude Code: interactive API-key approval prompt in a fresh home; whether workflow agents are child
  processes; whether a settings `env` proxy value beats the launch env.
- Codex: WebSocket relay and recording through the new proxy on 0.165; whether `codex exec` runs the
  update check; that DeepSeek has no Responses endpoint.
- Hermes: CA trust through `SSL_CERT_FILE` on 0.22.5 in the uv-built Python; delegation on 0.22.5;
  whether `state.db` rows are redacted.

### O.9.2 Amp, OpenCode, OpenClaw

- Nothing here was run on Linux through goalcup with a logged-in Amp or with subagents in OpenCode or OpenClaw.
- Amp's docs are undated and change fast; Model Routing for a DeepSeek Custom URL may need a paid tier in practice.
- OpenCode 2 source is the `dev` branch head, which may be ahead of the 2.2.2 package.
- OpenClaw source is a 30 Oct UTC head, one commit set ahead of `v2026.10.9`.

### O.9.3 Cursor, Gemini CLI, Copilot CLI

- Copilot: a release between now and 7 Nov changes the transport or CA layer (15 releases in 30 days). Pin
  1.0.105 for the testbed, and re-check the changelog on 5 Nov.
- Copilot: if the Rust addon makes model calls and reads only its own roots, `NODE_EXTRA_CA_CERTS` will not
  help and the harness fails TLS. That fails loudly, not silently; the `SSL_CERT_FILE` bundle is the fallback.
- Copilot with DeepSeek `openai` type: participants who copy a generic guide get a 400 on the second turn.
  The guide should say "use `anthropic` type for DeepSeek".
- Gemini CLI: the owner must fund a paid Google key; without it Gemini cannot be tested at all.
- Cursor: even with HTTP/1.1 forced, the record is Cursor's protocol, not the provider call. If the owner
  lets Cursor in, the report must say the prompts Cursor's servers build are not recorded.
- Any harness: a participant who sets `NO_PROXY` broadly, or a harness setting we did not read, could send
  model calls around the proxy. A post-run check of direct connections to known model hosts would catch it.

### O.9.4 Goose, Aider, Kilo

- Every row above is from source and docs. None was run through our proxy. The first pad run is
  the real test.
- Replace-mode CA variables (`SSL_CERT_FILE` for Goose and Aider, `REQUESTS_CA_BUNDLE`) break every
  other host if goalcup passes only the run CA. Goalcup must pass a full bundle.
- If goalcup changes `HOME` or `XDG_*`, Goose and Kilo lose the participant's config and keys;
  if it does not, the pad must use a private `HOME`.
- Aider adds little to a Goal Cup test (no subagents, not autonomous, stale). Recommend Goose and
  Kilo first.

### O.9.5 Qwen Code, Crush, Factory Droid

- None of the three needs a model base URL from goalcup. All three honour `HTTPS_PROXY`; all are
  untested under the run CA. Each needs one probe run before anyone promises them.
- CA env differs by runtime: Node and Bun take `NODE_EXTRA_CA_CERTS` (adds to roots); Go takes
  `SSL_CERT_FILE` (replaces roots). goalcup can safely pass both: a full bundle in `SSL_CERT_FILE` and
  the run CA in `NODE_EXTRA_CA_CERTS`.
- All three send non-model traffic (telemetry, catalogs, Factory backend) through the same proxy. The
  recorder will see it; that is fine under "record everything", but the secret masking must cover
  `FACTORY_API_KEY` (Droid).
- Qwen's built-in `claude-code` and `codex` subagents launch other harnesses as child processes. If a
  participant asks for them, the run records a second harness inside the first (unverified).

### O.9.6 DeepSeek and the backups

- **Prices or peak hours change.** The page says DeepSeek "reserves the right to adjust them". Read it
  again before funding. The owner's 0.12/0.48 figure is already out of date.
- **Search billing from 31 Oct 16:00 UTC.** Any harness that uses DeepSeek's server-side web search adds
  0.75 dollars per 1,000 searches. Small, but those searches are invisible to the proxy except as blocks
  in the response.
- **Outages look like proxy bugs.** See O.5.3. Keep provider 4xx and 5xx in the record and in the pad table.
- **Usage gaps in stats.** Streams without `include_usage` have no usage. Do not report zero.
- **Backups are unverified.** Every proxy and CA line in Part B comes from docs or package source. The
  pad must run each one before it is called "verified".
- **Config files that hold keys or proxy settings.** Cline's `CLINE_DIR`, dsh's `$DSH_HOME/.env`, Pi's
  `auth.json`, Vibe's `$VIBE_HOME/.env`, Kimi's `config.toml` if a participant pastes the key. A `.env`
  that sets proxy variables can silently route around us (dsh documents this). The pad should give each
  harness a fresh home folder and check the record has the expected host.
- **Version drift by Monday.** Kimi Code has daily canaries, Cline nightlies, dsh release candidates,
  Letta and CodeBuddy several releases a week. Pin versions in the pad and turn auto-update off.
- **Model ids were left out of this file on purpose.** The pad needs them; copy them from the pricing
  page on the day.

## O.10 Sources for the DeepSeek facts and the backups (all fetched 29 Oct 2026 unless a date is given)

- DeepSeek: https://api-docs.deepseek.com/ ; https://api-docs.deepseek.com/quick_start/pricing ;
  https://api-docs.deepseek.com/quick_start/rate_limit ; https://api-docs.deepseek.com/quick_start/error_codes ;
  https://api-docs.deepseek.com/quick_start/token_usage ; https://api-docs.deepseek.com/guides/kv_cache ;
  https://api-docs.deepseek.com/guides/thinking_mode ; https://api-docs.deepseek.com/guides/tool_calls ;
  https://api-docs.deepseek.com/guides/responses_api ; https://api-docs.deepseek.com/guides/anthropic_api ;
  https://api-docs.deepseek.com/api/create-chat-completion ; https://api-docs.deepseek.com/news/news261017 (17 Oct 2026) ;
  agent guides under https://api-docs.deepseek.com/quick_start/agent_integrations/ (claude_code, codex,
  reasonix, workbuddy, qoder) ; https://status.deepseek.com/
- Outage: https://www.yicaiglobal.com/news/deepseek-suffers-nearly-five-hour-ai-service-outage-one-of-its-worst-this-year (15 Oct 2026) ;
  https://scmp.com/tech/big-tech/article/3371047/deepseek-offers-user-refunds-after-suffering-another-major-outage
- Registries: https://registry.npmjs.org/<pkg> (cline, @letta-ai/letta-code, @earendil-works/pi-coding-agent,
  @deepseek-ai/dsh, reasonix, @qoder-ai/qodercli, @tencent-ai/codebuddy-code, @moonshot-ai/kimi-code,
  @continuedev/cli, @augmentcode/auggie, @iflow-ai/iflow-cli) ; https://pypi.org/pypi/<pkg>/json (mistral-vibe,
  kimi-cli, openhands) ; download counts as stated in Part B
- Kimi Code: https://moonshotai.github.io/kimi-code/en/configuration/providers ;
  https://moonshotai.github.io/kimi-code/en/configuration/data-locations ;
  https://moonshotai.github.io/kimi-code/en/reference/kimi-command ; https://moonshotai.github.io/kimi-code/en/guides/migration
- Mistral Vibe: https://raw.githubusercontent.com/mistralai/mistral-vibe/v3.0.3/README.md ;
  https://docs.mistral.ai/mistral-vibe/terminal/configuration
- Cline: https://docs.cline.bot/cli/overview ; https://docs.cline.bot/cli/cli-reference ;
  https://docs.cline.bot/troubleshooting/networking-and-proxies.md
- DeepSeek Harness: https://deepseek-harness.github.io/deepseek-harness/en/guide/quickstart
- OpenHands: https://docs.openhands.dev/openhands/usage/cli/installation ;
  https://raw.githubusercontent.com/OpenHands/OpenHands-CLI/1.16.0/README.md
- Reasonix: https://github.com/esengine/DeepSeek-Reasonix (README on `main` and `main-v2`)
- Plandex: https://github.com/plandex-ai/plandex/releases ; Roo Code: https://roocode.com/blog/sunsetting-roo-code-extension-cloud-and-router ;
  Warp: https://docs.warp.dev/support-and-community/plans-and-billing/bring-your-own-api-key
- Repo: `AGENTS.md:81`; `crates/goalcup-cli/src/proxy/tls.rs:26-30`; `crates/goalcup-cli/src/proxy/http.rs:1-12`;
  `crates/goalcup-verifier/src/capture_analysis/chat.rs:134-142,625`; `crates/goalcup-verifier/src/capture_analysis/stats.rs:420`;
  sections H1 to H15 above; `docs/plans/2026-10-30-audit/E-providers.md`.
