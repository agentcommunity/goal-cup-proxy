# Appendix E: providers and gateways (Lane E)

Date: 29 Oct 2026. Repo: `/home/user/goal-cup-proxy` at 4b226de2 (same commit as origin/main). One part
read the repo at `947895f3`, which is three docs-only audit commits on top of `4b226de2`; no code file
differs. Read-only: nothing was built, no test was run, no harness was run, no key or secret was read.
Line counts are `wc -l`. This appendix merges three lane parts: Anthropic and OpenAI (E1, part
`E-anthropic-openai`), DeepSeek, other OpenAI-compatible providers and local servers (E2, part
`E-deepseek-local`), and gateways and clouds (E3, part `E-gateways-clouds`). Citations of
`parts/D-*.md` point to lane D's part files, now merged in `D-harnesses.md`; `parts/F-os.md` is merged
in `F-week-long-risks.md`. The 2-hour run report (`docs/research/results/2026-10-30-two-hour-run.md`)
does not exist on any of the remote branches yet. Nothing here depends on it.

## Lane summary

Every provider, gateway and cloud in scope speaks plain HTTPS on port 443 and accepts HTTP/1.1 for the
calls a coding agent makes. Streams are SSE everywhere except Bedrock's native AWS event-stream and
Vertex's `:streamRawPredict`; WebSockets appear only in OpenAI's Responses "WebSocket mode" (also on
Vercel, LiteLLM and the ChatGPT backend Codex uses), Realtime and Live APIs, and Cloudflare's universal
socket. No official SDK or doc pins certificates. So our HTTP/1.1 proxy, SSE framer, WebSocket relay,
zstd decoding and header masking already fit the remote services, and the "one address the config
names" rule (D74) records every gateway (measured on Vercel). The real gaps are not the providers. They
are routes the reader cannot see (Claude Code's Bedrock, Vertex and Foundry switches; Codex built-ins
`amazon-bedrock`, `gcp`, `ollama`, `lmstudio`; Codex profiles; `.env` overrides), clients that never
reach the proxy (old Node `fetch`, the `ws` package, old Python `websockets`, aiohttp without
`trust_env`, AWS SDK for JS v3, loopback and plain HTTP), and clients that would break if we decrypted
more (boto3, gRPC, `cloudflared`, Go and Java on macOS). Recording every call with no host list is not
safe for Friday 6 Nov. The Friday answer is: keep the small list, warn before the run on routes we cannot
see, name every sealed tunnel with no row cap, and fix the participant docs.

## Top findings

1. **Remote services fit the proxy.** All accept HTTP/1.1; none needs HTTP/2, HTTP/3, gRPC or a
   WebSocket for a coding agent (E1, E2, E3). Exceptions outside coding agents: gRPC clients for Vertex
   (HTTP/2 only) and Bedrock's bidirectional speech stream (HTTP/2). No pinning found anywhere.
2. **Gateways are easy, clouds are not.** Vercel, OpenRouter, Cloudflare, LiteLLM and Azure serve the
   three wire formats our decoders know under one base URL. Vercel is measured: Claude Code 15, Codex 17
   and Hermes 9 wire model requests (`docs/research/results/2026-10-vercel-gateway.md:50-55`). Claude
   Code's `CLAUDE_CODE_USE_BEDROCK` / `_VERTEX` / `_FOUNDRY` and Codex's built-in `amazon-bedrock` and
   `gcp` tunnel silently, and Bedrock and Vertex native streams cannot be decoded anyway.
3. **Every cloud also serves a base-URL route our rule already records**: Bedrock
   `https://bedrock-runtime.{region}.amazonaws.com/anthropic/v1/messages` and `/openai/v1/...`, Foundry
   `https://{resource}.services.ai.azure.com/anthropic/v1/messages` and `/openai/v1/`,
   Vertex `/endpoints/openapi/chat/completions` and Codex's `gcp` `.../publishers/openai/v1/responses`.
   The cheapest fix is docs, not decoders.
4. **Local model servers cannot be recorded, by design, and two paths are silent.** Plain `http://`,
   loopback, IP literals and local-only names are refused (`crates/goalcup-cli/src/model_host.rs:450-475`,
   `:522-524`). But Codex's built-in `ollama`/`lmstudio` providers (and `CODEX_OSS_BASE_URL`) and Ollama
   "cloud" models (harness to local daemon to `ollama.com`) are not refused and not recorded.
   `docs/RULES.md:134-136` promises "full freedom of model ... compute" and no participant doc says local
   models are not recorded.
5. **Bypass paths that matter**: Node global `fetch` before 22.21.0 (or 23.x) ignores
   `NODE_USE_ENV_PROXY`; `ws` without an agent dials directly; Python `websockets` 13 and 14 ignore proxy
   env; aiohttp ignores it without `trust_env`; AWS SDK for JS v3 never reads it; harness `.env` files
   (`$CODEX_HOME/.env`, `$DSH_HOME/.env`) and Claude Code's `settings.json` `env` override ours.
6. **Recording every call with no host list is not safe for Friday.** Blind interception breaks tools
   that do not trust our CA (git, curl, pip, boto3, gRPC, `cloudflared`, Go, Java). The only list-free
   design that works is intercept by peer process: about 200 to 400 src lines plus tests on two OSes
   (estimate). Edition 02.
7. **Header masking misses a few non-key identifiers**: `chatgpt-account-id`, `x-codex-turn-state`,
   `OpenAI-Organization`/`-Project`, `anthropic-workspace-id` and an ephemeral key in
   `Sec-WebSocket-Protocol`. Owner to decide whether account ids are secrets (two words in `headers.rs`).
8. **The tunnel list stops naming hosts after 10,000 rows** (`crates/goalcup-cli/src/proxy/tunnels.rs:19-20`).
   A week of `npm install` and `git` passes that. Aggregate per host and port instead.
9. **Cut the shared provider table** (`crates/goalcup-protocol/providers.toml`, about 55 src lines of
   loader). Two rows, one user; the `deepseek` row is unused and names `/v1/chat/completions`, while
   DeepSeek's docs now use `https://api.deepseek.com/chat/completions` with no `/v1`.
10. **Several docs are stale**: "no header is recorded" (`docs/ARCHITECTURE.md:213`,
    `connectors/codex-cli/harness.toml:101-107`) has been false since 28 Oct (commit `e0204ba3`); the
    Vercel research docs still describe pre-D74 behaviour.
11. **168-hour risks**: provider outages (about 10 Anthropic and about 15 OpenAI model incidents in
    October, at least 25 Fireworks incidents), subscription weekly limits and spend caps, a mid-run
    switch of model service that tunnels until `goalcup continue` because the CA is name-constrained
    (`crates/goalcup-cli/src/proxy/tls.rs:131-137`), Vercel Sandbox's 24-hour session maximum, Vercel
    OIDC tokens' 12-hour life.

## Summary table

"h2" means HTTP/2, "h3" HTTP/3. HTTP version cells from keyless probes may show the sandbox egress
proxy's choice, not the vendor's (see "Where the parts disagree").

| Provider or gateway | Hosts | Stream format | WebSockets | HTTP/2 | Pinning | Proxy-skip risk |
|---|---|---|---|---|---|---|
| Anthropic API | `api.anthropic.com` (model), `platform.claude.com` (sign-in, kept opaque) | SSE, named events | None for Messages | Not documented; Claude Code measured HTTP/1.1; probe inconclusive (answered by egress `envoy`) | None found | Low for Claude Code (Bun binary). `settings.json` `env` override. TS SDK on old Node `fetch` |
| OpenAI API | `api.openai.com`; residency `us.`, `eu.`, `ae.api.openai.com` | SSE | Responses WebSocket mode (60 min), Realtime `wss`, WebRTC calls | Probe: h2 and `alt-svc: h3`; HTTP/1.1 works | None found | `ws` without agent, Python `websockets` 13/14, old Node `fetch`, WebRTC over UDP |
| ChatGPT backend (Codex) | `chatgpt.com/backend-api/codex`, `auth.openai.com`, `gov.chatgpt.com`, `chat.openai.com` | SSE fallback, zstd request bodies | WebSocket first, permessage-deflate | Probe: h2; Codex used HTTP/1.1 through our proxy | None found | `$CODEX_HOME/.env`; `gov.chatgpt.com` not listed |
| DeepSeek | `api.deepseek.com`, `api.deepseek.com/anthropic` | SSE with `: keep-alive` comments | None | h2 offered, HTTP/1.1 accepted, no h3 | None found | DeepSeek's own harness `dsh`: loopback always bypassed, `$DSH_HOME/.env`, telemetry direct |
| Moonshot | `api.moonshot.ai`, `api.moonshot.cn` | SSE | None | h2 offered, no h3 | None found | Low (config-named base URL) |
| Z.ai | `api.z.ai`, `open.bigmodel.cn` | SSE | None | h2, `alt-svc: h3` | None found | Low. `/api/v1/responses` returns 200 with an error body |
| Groq | `api.groq.com` (`/openai/v1`) | SSE, usage in `x_groq.usage` (unverified) | None | h2, `alt-svc: h3` | None found | `groq-sdk` (Node) on old Node `fetch` |
| Together | `api.together.ai`, `us-api.together.ai`, `api.together.xyz` | SSE | None | h2, `alt-svc: h3` | None found | Low |
| Fireworks | `api.fireworks.ai` (`/inference`) | SSE | None | h2, no h3 | None found | `fireworks-ai` uses aiohttp; `trust_env` unverified |
| Ollama (local) | `127.0.0.1:11434` (`OLLAMA_HOST`) | NDJSON native; SSE on OpenAI and Anthropic routes | None | Plain HTTP | n/a | Always skips (plain HTTP, loopback). Refused when read; silent for Codex built-in |
| Ollama cloud models | local daemon, then `ollama.com` | as above | None | n/a | n/a | Always: the daemon's leg uses its own env |
| LM Studio | `localhost:1234` | SSE on HTTP routes | SDKs talk to the app over WebSocket | Plain HTTP | n/a | Always skips; silent for Codex built-in |
| llama.cpp `llama-server` | `127.0.0.1:9931` (README now; was 8080), UNIX socket | SSE | None | Plain HTTP (TLS optional) | n/a | Always skips |
| vLLM | port 8000, UNIX socket | SSE; `--grpc` replaces HTTP | None | Plain HTTP; gRPC needs h2 | n/a | Always skips |
| Vercel AI Gateway | `ai-gateway.vercel.sh` | SSE | Responses WebSocket, about 13 min max | Not documented; measured HTTP/1.1 through our proxy | None found | Low, measured. OIDC 12 h; Sandbox brokering terminates TLS |
| OpenRouter | `openrouter.ai` (`/api/v1`) | SSE with comments, double `finish_reason` | None documented | Not documented | None found | Ori launcher starts the harness outside `goalcup start` |
| Cloudflare AI Gateway | `gateway.ai.cloudflare.com`, `api.cloudflare.com`, custom domain | Provider's SSE | Realtime and universal `wss` | Not documented | None found | Codex recipe is a profile we do not read; `cloudflared` (Go) helper on macOS |
| LiteLLM proxy | self-hosted, default `http://0.0.0.0:4000` | SSE | Responses WebSocket, 60 min | Not documented | None found | Loopback plain HTTP refused; remote `https://` fine |
| Amazon Bedrock | `bedrock-runtime.{region}.amazonaws.com`, `bedrock-mantle.{region}.api.aws` | Native: binary AWS event-stream; compatible routes: SSE | Bidirectional stream (h2) only | "all our APIs support http2"; SDKs HTTP/1.1 by default (general knowledge) | None found | High: Claude Code switch and Codex built-in tunnel; boto3 trusts `certifi` only; AWS JS v3 ignores env |
| Google Vertex AI | `{LOCATION}-aiplatform.googleapis.com`, `aiplatform.googleapis.com`, `aiplatform.us.rep.googleapis.com`, `aiplatform.eu.rep.googleapis.com` | `:streamRawPredict`; `streamGenerateContent` (SSE with `alt=sse`); OpenAI-compatible SSE | Live API `wss` | gRPC transport needs h2 | None found; mTLS endpoints exist | High: Claude Code switch and Codex `gcp` tunnel; gRPC h2 only with own roots; `@google/genai` uses `ws` |
| Azure OpenAI / Microsoft Foundry | `{resource}.openai.azure.com`, `{resource}.services.ai.azure.com`, `example-resource.azure.anthropic.com` (SDK comment) | SSE | Realtime `wss` (unverified for v1) | Not documented | None found | `CLAUDE_CODE_USE_FOUNDRY` tunnels; Hermes with `?api-version=` refused |

## Where the parts disagree

| Topic | E1 / E2 say | E3 says | Reading |
|---|---|---|---|
| What the keyless probes show about HTTP versions | E1: `api.openai.com` negotiated h2 and showed `alt-svc: h3=":443"; ma=86400`; `chatgpt.com` negotiated h2. The Anthropic probe was answered by the egress gateway (`server: envoy`), so it says nothing. E2: its probes "did reach the providers" (provider `server`, `cf-ray` and error bodies), so h2 offers and `alt-svc` are the providers' | The sandbox egress proxy re-signs TLS, so the HTTP/2 the probe showed "is the egress proxy's choice, not the vendor's"; probes only show the hosts answer | Both stated. Response bodies and headers did come from providers, but ALPN on the client leg may be the egress proxy's. Treat h2 and h3 cells as unverified at the vendor; HTTP/1.1 acceptance is backed by measured runs and docs |
| The `model_host.rs:1-8` / `:5-7` comment "There is no provider list" | E2: right for every remote provider in scope; keep | False for the cloud switches, which name no base URL in the key we read; change the comment to "one address per connector" | Both stated. True for base URLs, false for switches |
| `model_host.rs` verdict | E1: keep (lane D adds Codex keys). E2: keep, small change: refuse Codex built-ins `ollama`/`lmstudio` with no table, reword two refusal strings (about +20 src) | Keep; add a separate cloud-switch warning or refusal (about +30 to +50 src est.) | Complementary. One change can cover both: refuse or warn on any Codex built-in with no `base_url` and on Claude Code cloud switches |
| Decoder line counts | E1: `anthropic.rs` 1638, `openai.rs` 664, `chat.rs` 1080, "total lines incl. inline tests". E2: `chat.rs` 1,080, "no inline tests"; tests in `crates/goalcup-verifier/tests/it/m2_chat_decoder.rs`, 2,524 | `anthropic.rs` 1,585 src + 53 inline tests; `openai.rs` 664; `chat.rs` 1,080, "total src, tests in separate files" | Consistent: 1,585 + 53 = 1,638 |
| Wildcard decrypt of cloud hosts (lane B) | not discussed | Reject `docs/plans/2026-10-30-audit/B-fresh-design.md:264-268`, `:668-674` | Disagreement with lane B; E3's reasons are in E9 |
| `providers.toml` | E1: cut, move each row into its connector. E2: cut the `deepseek` row with the table | Keep "frozen"; "the sibling's cut stands" | Agreement on cutting |
| Launch env lines | E1: `launch.rs:855-860`, `:320`. E2: `:319-320`, `:855-860` | `:319-320`, `:855-861` | Same code; ranges differ by one line |

## How this was checked

- Web: the providers', gateways' and clouds' own docs, fetched 29 Oct 2026, mostly as Markdown (`.md` or
  `llms.txt` versions); npm and PyPI metadata; Node.js docs; package source read from unpkg, PyPI wheels,
  npm tarballs (DeepSeek's `@deepseek-ai/dsh`) and GitHub raw (`botocore` develop branch, gRPC env doc).
  Page dates are the "last updated" or `ms.date` the page shows; "undated" or "no date" means the page
  shows none. GitHub API access from this sandbox is blocked for some repos, so GitHub release pages were
  read with the web fetcher.
- Codex source: the shallow clone of `openai/codex` at tag `rust-v0.165.0` (commit `360319d8`, 27 Oct
  2026) that lane D left in the scratchpad (`dl/c165`). Links point to
  `https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/<path>`.
- Keyless probes on 29 Oct 2026: one `POST` (empty body or `{}`, no key, no token) to each model path,
  over HTTP/1.1 and, for E2, also HTTP/2. Only status, content type, the first bytes of the error body
  and `server`/`alt-svc` headers were read. All went through this sandbox's egress proxy (see the
  disagreement above).
- Harness-side facts (Claude Code, Codex, Hermes proxy handling, config files, subagents) are in lane D.
  Cited, not repeated.
- Model names are left out on purpose; where a source names a model, this says "a hosted model".

## E1. Anthropic API

### E1.1 Facts

| Topic | Fact | Source (date) |
|---|---|---|
| Host | `https://api.anthropic.com`. Cloudflare fronts it ("On the first-party Claude API, Cloudflare returns this error before the request reaches the API servers", for 413) | https://platform.claude.com/docs/en/api/overview, https://platform.claude.com/docs/en/api/errors (fetched 29 Oct 2026) |
| Model path | `POST /v1/messages`. Also `POST /v1/messages/count_tokens`, `POST /v1/messages/batches`, `GET /v1/models`, `/v1/files`, `/v1/skills`; beta `/v1/agents`, `/v1/sessions` (with `GET /v1/sessions/{id}/events/stream`), `/v1/environments` | overview page (29 Oct 2026) |
| Token endpoint on the model host | `POST /v1/oauth/token` on `api.anthropic.com` issues short-lived access tokens for Workload Identity Federation | overview page, Authentication table (29 Oct 2026) |
| Auth headers | `Authorization: Bearer <key or token>` is now the primary form; `x-api-key` is "Legacy fallback ... still supported". `anthropic-version` required (doc example `2026-10-21`). `anthropic-workspace-id` for keys not scoped to a workspace. `anthropic-beta` for betas | overview page (29 Oct 2026) |
| Subscription sign-in (Claude Code) | Token exchange and refresh at `platform.claude.com`; model calls still to `api.anthropic.com` | lane D, https://code.claude.com/docs/en/network-config (29 Oct 2026) |
| API versions | `2026-10-21` is new. It removes `temperature`, `top_p`, `top_k` and rejects two block placements. No change to streaming events. Policy: Anthropic "may ... add new variants to enum-like output values (for example, streaming event types)" | https://platform.claude.com/docs/en/api/versioning (29 Oct 2026) |
| Streaming | SSE, named events: `message_start`, `content_block_start`, `content_block_delta`, `content_block_stop`, `message_delta`, `message_stop`, `ping`, `error`. Deltas: `text_delta`, `input_json_delta`, `thinking_delta`, `signature_delta`. New: a `fallback` content block at each model boundary during server-side fallback, and `input_transformations` on `message_start` / final `message_delta` under a beta header. Server tools add `server_tool_use` and `web_search_tool_result` blocks. "New event types may be added" | https://platform.claude.com/docs/en/build-with-claude/streaming (29 Oct 2026) |
| WebSocket | None for the Messages API | streaming page (29 Oct 2026) |
| HTTP version | Not documented. Not probed: in this sandbox the request to `api.anthropic.com` was answered by the sandbox's own egress gateway (`server: envoy`, no CONNECT line), so it says nothing about Anthropic. Claude Code measured HTTP/1.1 (`docs/harnesses/claude-code-cli.md:72`, per lane D) | probe 29 Oct 2026 |
| Errors | 400, 401, 402, 403, 404, 409, 413, 429 `rate_limit_error`, 500, 504 `timeout_error`, 529 `overloaded_error`. Mid-stream errors arrive as an SSE `error` event after a 200. Body: `{"type":"error","error":{"type":...,"message":...},"request_id":...}` | https://platform.claude.com/docs/en/api/errors (29 Oct 2026) |
| Rate limits | Token bucket per org. `retry-after` and `anthropic-ratelimit-{requests,tokens,input-tokens,output-tokens}-{limit,remaining,reset}`. A spend-cap 429 has no `retry-after` and fails until the next month (`error.details.error_code = enforced_spend_limit_reached`). "Acceleration limits" can 429 a sharp rise in use | https://platform.claude.com/docs/en/api/rate-limits (29 Oct 2026) |
| Long requests | Networks may drop idle connections; SDK sets TCP keep-alive and expects streaming for anything over 10 minutes | errors page, "Long requests" (29 Oct 2026) |
| Outages | Status feed lists 17 incidents in October 2026, about 10 of them "elevated errors" on model calls (5, 6, 21, 24, 25, 27 Oct) | https://status.claude.com/history.rss (fetched 29 Oct 2026) |
| Request size | Messages up to 128 MB | overview page (29 Oct 2026) |

### E1.2 Anthropic SDKs

| SDK | Latest | Transport and proxy | Source |
|---|---|---|---|
| TypeScript `@anthropic-ai/sdk` | 1.3.0, 23 Oct 2026 | Runtime dependencies only `standardwebhooks`, `json-schema-to-ts`, `@opentelemetry/api`, so it uses the runtime's global `fetch`. Under Node, global `fetch` uses `HTTPS_PROXY` only with `NODE_USE_ENV_PROXY=1` on Node 22.21+ or 24+ (see E7). Proxy option in the README: not checked (unverified) | https://registry.npmjs.org/@anthropic-ai%2Fsdk (29 Oct 2026) |
| Python `anthropic` | 2.3.0, 23 Oct 2026 | Now built on `httpx2` (Pydantic's fork of httpx). The client reads `get_environment_proxies()` and mounts a proxy transport per scheme, so `HTTPS_PROXY` and `NO_PROXY` are honoured. HTTP/1.1 unless the caller passes `http2`. TCP keep-alive 60 s. Wheel `anthropic/_base_client.py:1022-1038` | https://pypi.org/pypi/anthropic/json (29 Oct 2026) |
| `httpx2` trust | 2.16.0, 21 Oct 2026. Since 2.3.0 (1 Jun 2026) the default trust is the OS store (`truststore`) on macOS and Windows, and the compiled-in OpenSSL paths on Linux. `SSL_CERT_FILE` or `SSL_CERT_DIR`, when set, **replace** the trust entirely (`httpcore2/_ssl.py:20-40`) | https://github.com/pydantic/httpx2/blob/main/src/httpx2/CHANGELOG.md (2.3.0 entry), wheel `httpcore2-2.16.0` |
| `@anthropic-ai/bedrock-sdk`, `-vertex-sdk`, `-foundry-sdk` | 1.1.0, 23 Oct 2026 | Transport is the Anthropic client's `fetch` (details in E6.5 to E6.7) | unpkg, see E6 |
| Pinning | None found in either SDK or in the docs | as above |

## E2. OpenAI API and the ChatGPT backend Codex uses

### E2.1 OpenAI API

| Topic | Fact | Source (date) |
|---|---|---|
| Host | `https://api.openai.com/v1`. Cloudflare (`server: cloudflare`, `cf-ray`). Data residency hosts `us.api.openai.com`, `eu.api.openai.com`, `ae.api.openai.com` exist in the Python SDK (`openai/_data_residency.py:15-18`) | probe 29 Oct 2026; PyPI `openai` 3.41.0 wheel (28 Oct 2026) |
| Model paths | `POST /v1/responses` (SSE when `stream: true`), `POST /v1/chat/completions`, `/v1/responses/compact`, Realtime `wss://api.openai.com/v1/realtime` and `/v1/realtime/calls` (WebRTC) | https://developers.openai.com/api/docs/guides/websocket-mode (29 Oct 2026); Codex `codex-api/src/endpoint/realtime_call.rs#L442` |
| WebSocket mode | One persistent connection to `/v1/responses`. Client sends `response.create`; server sends the usual Responses events. "Connections last up to 60 minutes. Reconnect at the limit." Error code `websocket_connection_limit_reached`. Up to 16 in-flight responses per connection across "lanes" named by `stream_id` (at most 32 named lanes); events of different lanes interleave and carry `stream_id`. `generate: false` warm-up. A connection-local cache serves `previous_response_id`; it is lost on close | https://developers.openai.com/api/docs/guides/websocket-mode.md (no date on page, fetched 29 Oct 2026). Launch date: Codex sends beta header `responses_websockets=2026-02-06` (lane D); news reports point to spring 2026, exact OpenAI changelog date unverified |
| Auth | `Authorization: Bearer <key or access token>`. Optional `OpenAI-Organization`, `OpenAI-Project`. Response `x-request-id`; optional client `X-Client-Request-Id` | https://developers.openai.com/api/reference/overview (29 Oct 2026) |
| Browser Realtime auth | The Node SDK's browser Realtime client puts the key in the WebSocket subprotocol: `Sec-WebSocket-Protocol: realtime, openai-insecure-api-key.<key>` | `openai@7.45.1/realtime/websocket.js:153-156` via https://unpkg.com/openai@7.45.1/realtime/websocket.js (29 Oct 2026) |
| HTTP version | Probe negotiated HTTP/2 and saw `alt-svc: h3=":443"; ma=86400` on `api.openai.com`. `chatgpt.com` negotiated HTTP/2 (but see "Where the parts disagree"). HTTP/1.1 also works: Codex used it through our proxy (`docs/harnesses/codex-cli.md:149`) | probe 29 Oct 2026 |
| Rate limits | `x-ratelimit-{limit,remaining,reset}-{requests,tokens}` and `-project-tokens`. 429 with `slow_down` for a fast ramp; hard spend limit 429 "isn't resolved by retrying". `Retry-After` is a minimum | https://developers.openai.com/api/docs/guides/rate-limits (29 Oct 2026) |
| Outages | Status history lists about 15 incidents in October 2026 that name the API, Responses or Codex (5, 6, 7, 13, 14, 20, 21, 22 Oct), one of them sign-in errors for about an hour on 20 Oct | https://status.openai.com/history (fetched 29 Oct 2026) |

### E2.2 ChatGPT backend (Codex on a ChatGPT sign-in), from Codex 0.165.0 source

| Topic | Fact | Source |
|---|---|---|
| Base | `https://chatgpt.com/backend-api/codex` for every ChatGPT-style auth mode (ChatGPT, ChatGPT tokens, headers, agent identity, personal access token); `https://api.openai.com/v1` otherwise | `model-provider-info/src/lib.rs#L82`, `#L431-L450` |
| Model calls | `/responses` over WebSocket first (permessage-deflate on), HTTP POST + SSE fallback. Remote compaction v2 runs through the same Responses stream executor | `codex-api/src/endpoint/responses_websocket.rs#L464`, `#L578`; `core/src/compact_remote_v2.rs` |
| Other paths on the same host | `/models`, `alpha/search`, `memories/trace_summarize`, `images/generations`, `images/edits`, `realtime/calls`, `/backend-api/wham/...` (remote control) | `codex-api/src/endpoint/{models,search,memories,images}.rs`, `realtime_call.rs#L487`, `app-server-transport/src/transport/remote_control/clients.rs#L270` |
| Other hosts | `auth.openai.com` (token exchange and refresh), `gov.chatgpt.com` (government accounts), `chat.openai.com` (legacy) | `login/src/auth/manager.rs`, `codex-mcp/src/mcp/mod.rs#L383` |
| Request headers | `Authorization: Bearer <JWT>`, `chatgpt-account-id`, `originator`, session and thread ids, `x-openai-subagent` for subagent turns, `x-codex-turn-state` (a sticky-routing token the server sets and the client replays), `x-openai-internal-codex-residency` under managed residency | `codex-api/src/endpoint/responses.rs#L90-L92`, `core/src/client.rs#L164`, `#L291-L304`, `model-provider-info/src/lib.rs#L43` |
| Request body coding | HTTP fallback bodies are zstd level 3 when the feature `enable_request_compression` is on (Stable, default on) and the sign-in uses the Codex backend | `core/src/client.rs#L1770-L1778`, `http-client/src/request.rs#L209-L211`, `features/src/lib.rs#L1482-L1485` |
| Retries | Request retries 4, stream retries 5 (max 100 each), stream idle timeout 300 s, WebSocket connect timeout 15 s. Handles `websocket_connection_limit_reached` | `model-provider-info/src/lib.rs#L68-L77`, `codex-api/src/endpoint/responses_websocket.rs#L166` |
| DoH, QUIC, pinning | No DNS-over-HTTPS crate, no pinning found. QUIC only in the opt-in `codex tcp-tunnel` (lane D) | `rg` over the tree; lane D section 2 |
| Built-in local providers | `ollama` and `lmstudio` with base `http://localhost:{11434|1234}/v1`, wire `responses`, overridable by `CODEX_OSS_PORT` and `CODEX_OSS_BASE_URL` | https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/model-provider-info/src/lib.rs#L690-L823 (27 Oct 2026) |
| Built-in cloud providers | `amazon-bedrock` and `gcp` (see E6.5, E6.6) | `model-provider-info/src/lib.rs#L84-L103`, `#L573-L611`; `model-provider/src/gcp/mod.rs#L209` |

### E2.3 OpenAI SDKs

| SDK | Latest | Transport and proxy | Source |
|---|---|---|---|
| Node `openai` | 7.45.1, 27 Oct 2026, engines `node >=22` | Zero runtime dependencies: HTTP uses global `fetch`, so `NODE_USE_ENV_PROXY` decides (E7). The README tells users to pass an undici `ProxyAgent` dispatcher or Bun's `proxy` option. WebSockets use the `ws` package (peer dependency `ws ^8.21.0`) | https://registry.npmjs.org/openai, https://unpkg.com/openai@7.45.1/README.md "Configuring proxies" (29 Oct 2026) |
| `ws` 8.22.0 | | Sets `opts.createConnection` to its own `tls.connect` unless the caller gives one (`lib/websocket.js:766-767`). Node docs: `createConnection` is used "when the `agent` option is not used". So a `ws` socket without an explicit agent dials the host directly and ignores `HTTPS_PROXY`, even with `NODE_USE_ENV_PROXY=1` (read from source and docs, not measured) | https://unpkg.com/ws@8.22.0/lib/websocket.js; https://nodejs.org/api/http.json `http.request` options (29 Oct 2026) |
| Python `openai` | 3.41.0, 28 Oct 2026 | `httpx2` like Anthropic's (honours env proxies). Realtime and WebSocket need `websockets>=13,<16`. `websockets` uses env proxies only from 15.0 (16 Feb 2025), so 13.x or 14.x dials directly | https://pypi.org/pypi/openai/json; https://websockets.readthedocs.io/en/stable/project/changelog.html (latest 17.3, 19 Oct 2026) |
| Pinning | None found | as above |

## E3. DeepSeek

### E3.1 API

| Topic | Fact | Source (date) |
|---|---|---|
| Hosts | `https://api.deepseek.com` (OpenAI format) and `https://api.deepseek.com/anthropic` (Anthropic format). Fronted by CloudFront (`via: ... cloudfront.net`, `server: elb`) | https://api-docs.deepseek.com/ (undated, fetched 29 Oct 2026); probe 29 Oct 2026 |
| Model paths | `POST /chat/completions` in the docs; `/v1/chat/completions` also answers (401 without a key); Anthropic route `POST /anthropic/v1/messages` (401). `/v1/responses` answered 401 before routing, so whether it exists is unverified | docs above; probe 29 Oct 2026 |
| Auth | OpenAI route: `Authorization: Bearer <key>`. Anthropic route: `x-api-key` "Fully Supported"; Bearer not mentioned | https://api-docs.deepseek.com/, https://api-docs.deepseek.com/guides/anthropic_api (undated, fetched 29 Oct 2026) |
| Streaming | SSE with `stream: true`. While a request waits, the server sends `: keep-alive` SSE comments (streaming) or **empty lines** before the JSON body (non-streaming). "If the request has not started inference after 10 minutes, the server will close the connection" | https://api-docs.deepseek.com/quick_start/rate_limit (undated, fetched 29 Oct 2026) |
| HTTP version | HTTP/2 offered, HTTP/1.1 accepted (probe). No `alt-svc: h3` seen | probe 29 Oct 2026 |
| Errors | 400, 401, 402 "Insufficient Balance", 422, 429 "Rate Limit Reached", 500, 503 "Server Overloaded". A 401 body is plain text with no content type: `Authentication Fails (governor)` | https://api-docs.deepseek.com/quick_start/error_codes (undated); probe 29 Oct 2026 |
| Rate limits | Concurrency based: "A request counts as one concurrent connection from the time it is sent until the model response is complete"; over the limit gives 429. No published RPM | rate_limit page above |
| Model aliases | A news item (`news261017`) says a model alias is now served by a newer build and two older names are retired but still accepted. Recording is not affected | https://api-docs.deepseek.com/ (fetched 29 Oct 2026) |
| Outages | `status.deepseek.com` showed no incidents for Aug to Oct 2026 and "Everything is running smoothly". The page is script-rendered and its JSON feed is not the Statuspage format, so this is weak evidence (unverified) | https://status.deepseek.com/, https://status.deepseek.com/history (fetched 29 Oct 2026) |
| SDK | No official SDK: the docs use the OpenAI SDKs (E2.3) | docs above |

### E3.2 DeepSeek's own harness

DeepSeek publishes a coding harness, `@deepseek-ai/dsh` (repository
`github.com/deepseek-ai/deepseek-harness`). Latest tag `0.2.1-rc.2` on 17 Oct 2026, **540,139 downloads
in the week 21 to 27 Oct 2026** (https://registry.npmjs.org/@deepseek-ai%2Fdsh,
https://api.npmjs.org/downloads/point/2026-10-21:2026-10-27/@deepseek-ai/dsh). It has subagents, agent
teams and a Ralph loop (package list in its bundled skill
`dsh-agent-preset/skills/cordis-composition-reference/references/packages.md`). No lane D part covers it
beyond the dead `harnesses/install.sh` line (`parts/D-others.md:414`).

Its `@deepseek-ai/dsh-http-proxy` README (in the 0.2.1-rc.2 tarball, read 29 Oct 2026) says:

- It routes Node's global `fetch` through `http_proxy`/`https_proxy`/`all_proxy`, read from the
  environment and then from `$DSH_HOME/.env`. A project's `.env` may not set them.
- "Loopback is always bypassed": `localhost`, all of `127.0.0.0/8`, `::1`, `0.0.0.0`.
- "Telemetry is direct by design": the OTLP exporter uses `node:http` and goes direct to
  `dsh-otel-collector.deepseeksvc.com` (string in the tarball).
- "No custom certificate authority": an intercepting proxy needs `NODE_EXTRA_CA_CERTS`.
- Child Node processes follow the proxy only on Node 22.21+ or 24+.

So dsh could be recorded the same way as Claude Code (proxy env plus `NODE_EXTRA_CA_CERTS`), but its own
`$DSH_HOME/.env` can override our proxy variables, just like `$CODEX_HOME/.env`
(`parts/D-codex.md:123-149`). Not measured. Point 7 signal only: no connector exists.

## E4. Other OpenAI-compatible providers

All probes: 29 Oct 2026, no key, body `{}`.

| Provider | Base URLs a harness is told to use | Paths probed (HTTP/1.1 status) | HTTP/2, HTTP/3 | Auth | Source (date) |
|---|---|---|---|---|---|
| Moonshot | OpenAI: `https://api.moonshot.ai/v1`. Claude Code: `ANTHROPIC_BASE_URL=https://api.moonshot.ai/anthropic` with `ANTHROPIC_AUTH_TOKEN`. Codex: `[model_providers.kimi] base_url = "https://api.moonshot.ai/v1"`, `wire_api = "responses"`, `env_key = "KIMI_API_KEY"`. China host `api.moonshot.cn` also answers | `/v1/chat/completions` 401, `/anthropic/v1/messages` 401, `/v1/responses` 401 | h2 offered, behind Cloudflare; no h3 seen | Bearer | https://platform.kimi.ai/docs/guide/agent-support, https://platform.kimi.ai/docs/guide/claude-code-kimi.md, https://platform.kimi.ai/docs/guide/codex-kimi.md (all undated, fetched 29 Oct 2026). Docs moved from `platform.moonshot.ai` (301) |
| Z.ai | General: `https://api.z.ai/api/paas/v4`. Coding plan: Anthropic `https://api.z.ai/api/anthropic`, Chat Completions `https://api.z.ai/api/coding/paas/v4`, Responses `https://api.z.ai/api/v1`. China: `open.bigmodel.cn` | `/api/paas/v4/chat/completions` 401, `/api/coding/paas/v4/chat/completions` 401, `/api/anthropic/v1/messages` 401, `/api/v1/responses` **200 with an error body** `{"code":1001,...,"success":false}` | h2, and `alt-svc: h3=":443"` | Bearer. Python `zai-sdk` 0.2.3 (16 Jun 2026) depends on `pyjwt`: it signs a JWT from the key (unverified detail, from the dependency) | https://docs.z.ai/guides/overview/quick-start, https://docs.z.ai/devpack/quick-start.md, https://docs.z.ai/devpack/tool/claude (undated, fetched 29 Oct 2026); https://pypi.org/pypi/zai-sdk/json |
| Groq | `https://api.groq.com/openai/v1`; Chat Completions and Responses | `/openai/v1/chat/completions` 401, `/openai/v1/responses` 401 | h2, `alt-svc: h3` (Cloudflare and Google front) | Bearer | https://console.groq.com/docs/openai (undated, fetched 29 Oct 2026) |
| Together | Docs: `https://api.together.ai/v1` (US only: `https://us-api.together.ai/v1`). The older `api.together.xyz` still answers | `/v1/chat/completions` 401 on both hosts; `/v1/responses` 400 (validation, so the route exists though the docs do not list it) | h2, `alt-svc: h3` | Bearer | https://docs.together.ai/docs/openai-api-compatibility (undated, fetched 29 Oct 2026) |
| Fireworks | OpenAI: `https://api.fireworks.ai/inference/v1`. Claude Code: `ANTHROPIC_BASE_URL=https://api.fireworks.ai/inference` with `ANTHROPIC_AUTH_TOKEN` | `/inference/v1/chat/completions`, `/inference/v1/messages`, `/inference/v1/responses`: all **400 before auth** (it validates the body first) | h2; no h3 seen | Bearer | https://docs.fireworks.ai/tools-sdks/anthropic-compatibility (undated, fetched 29 Oct 2026) |

Streaming for all five: SSE, as in the OpenAI or Anthropic wire. Provider-specific fields we know of:
Groq sends end-of-stream usage under `x_groq.usage` in the final chunk (third-party docs only:
https://apis.io/asyncapis/groq/groq-asyncapi/, https://hexdocs.pm/req_llm/groq.html; unverified against
Groq's reference). No provider in scope offers a WebSocket or gRPC model API to these harnesses (none
found in the docs above).

### E4.1 Rate limits and outages

| Provider | Limits | Outages in October 2026 | Source |
|---|---|---|---|
| Moonshot | Tiered table not readable on the page; 429 with `X-RateLimit-*` documented only for the search tools | Status feed unreachable from this sandbox (502 at the egress proxy) | https://platform.kimi.ai/docs/pricing/limits (fetched 29 Oct 2026) |
| Z.ai coding plan | Credits per 5 hours and per week: Lite 2,000 / 10,000, Pro 12,000 / 60,000, Max 28,000 / 140,000. Peak hours Mon to Fri 14:00 to 18:00 UTC+8 cost double the off-peak rate. What happens at the limit is not stated | `status.z.ai` refused by the sandbox egress (unverified) | https://docs.z.ai/devpack/overview (fetched 29 Oct 2026) |
| Groq | `x-ratelimit-{limit,remaining,reset}-{requests,tokens}` (requests per day, tokens per minute); 429 with `retry-after` only on 429 | 3 incidents in October (10 and 12 Oct), one "major" | https://console.groq.com/docs/rate-limits; https://groqstatus.com/api/v2/incidents.json (fetched 29 Oct 2026) |
| Together | Not read (unverified) | Status page is not Statuspage JSON (unverified) | https://status.together.ai/ |
| Fireworks | Not read (unverified) | At least 25 incidents in October, all "minor" "Service Degradation" for specific hosted models, 20 to 28 Oct. The feed holds the latest 50 only, so the count is a floor | https://status.fireworks.ai/api/v2/incidents.json (fetched 29 Oct 2026) |

### E4.2 Official SDKs (only matter if a harness uses them)

| SDK | Latest | HTTP stack | Proxy env | Source |
|---|---|---|---|---|
| `groq` (Python) | 1.7.0, 26 Aug 2026 | `httpx` | honoured by httpx by default | https://pypi.org/pypi/groq/json |
| `groq-sdk` (Node) | 1.6.0, 26 Aug 2026 | no dependencies, so global `fetch` | only with `NODE_USE_ENV_PROXY=1` on Node 22.21+ or 24+ (E7) | https://registry.npmjs.org/groq-sdk |
| `together` (Python) | 3.0.0, 22 Oct 2026 | `httpx` | honoured | https://pypi.org/pypi/together/json |
| `fireworks-ai` (Python) | 1.2.30, 21 Oct 2026 | `httpx` plus `httpx-aiohttp` and `aiohttp`. aiohttp ignores proxy env unless `trust_env=True`; whether this SDK sets it: unverified | possible bypass | https://pypi.org/pypi/fireworks-ai/json; https://github-redirect.dependabot.com/aio-libs/aiohttp/discussions/5855 |
| `zai-sdk` (Python) | 0.2.3, 16 Jun 2026 | `httpx`, `pyjwt` | honoured | https://pypi.org/pypi/zai-sdk/json |
| `litellm` (used by several Python harnesses) | 1.106.6, 28 Oct 2026 | aiohttp transport by default, `httpx[http2]` otherwise | "reads `HTTP_PROXY`, `HTTPS_PROXY` and `NO_PROXY` ... on both transports"; `DISABLE_AIOHTTP_TRUST_ENV=True` turns that off. CA from `SSL_CERT_FILE` or `SSL_VERIFY` | https://docs.litellm.ai/docs/guides/security_settings.md (last_updated 28 Oct 2026); https://pypi.org/pypi/litellm/json |

Pinning: none found in any of these (dependency lists show no pinning library; source not read line by
line, so "none found", not "none").

## E5. Local model servers

| Server | Latest | Default address | Model paths | Streaming | Auth | Source (date) |
|---|---|---|---|---|---|---|
| Ollama | v0.40.13, 28 Oct 2026 (v0.40.9 to v0.40.13 in one week) | `127.0.0.1:11434`; `OLLAMA_HOST` changes it, which is how it is put on the LAN | Native `/api/chat` (newline-delimited JSON); OpenAI `/v1/chat/completions`, `/v1/responses` (since v0.13.3), `/v1/completions`; Anthropic `/v1/messages`. Claude Code: `ANTHROPIC_BASE_URL=http://localhost:11434`, `ANTHROPIC_AUTH_TOKEN=ollama`. `ollama launch` starts coding tools (Codex, Claude Code, OpenCode) wired to it | SSE on the OpenAI and Anthropic routes | None locally ("required but ignored"). Hosted `https://ollama.com/v1` and `/api` need `Authorization: Bearer` (probe: 401) | https://github.com/ollama/ollama/releases, https://docs.ollama.com/api/openai-compatibility, https://docs.ollama.com/api/anthropic-compatibility, https://docs.ollama.com/faq (fetched 29 Oct 2026) |
| Ollama "cloud" models | same | The harness talks to the **local** daemon; "the Ollama Public Key ... lets your local Ollama instance talk to ollama.com". The daemon uses its own `HTTPS_PROXY`, not the harness's | same as above | same | local none; daemon signs to ollama.com | https://docs.ollama.com/faq (fetched 29 Oct 2026) |
| LM Studio | app version unverified; `@lmstudio/sdk` 2.1.0, 14 Oct 2026; Python `lmstudio` 1.5.0 (22 Aug 2025) | `localhost:1234` | OpenAI `/v1/...`, Anthropic `POST /v1/messages`. Claude Code: `ANTHROPIC_BASE_URL=http://localhost:1234`. The SDKs talk to the app over a WebSocket (`httpx-ws`) | SSE on HTTP routes | Optional; when on, `x-api-key` or Bearer | https://lmstudio.ai/docs/developer/anthropic-compat; https://registry.npmjs.org/@lmstudio%2Fsdk; https://pypi.org/pypi/lmstudio/json (fetched 29 Oct 2026) |
| llama.cpp `llama-server` | master README, fetched 29 Oct 2026 (no tag read) | `--host` default `127.0.0.1` (also UNIX sockets ending `.sock`); `--port` default **9931** in the current README (it was 8080 for years; check before writing it in a guide) | `/v1/chat/completions`, `/v1/completions`, `/v1/responses`, `POST /v1/messages` (Anthropic), native `/completion` | SSE | `--api-key`, `--api-key-file`, `LLAMA_API_KEY`; TLS only with `--ssl-key-file`/`--ssl-cert-file` on an OpenSSL build | https://raw.githubusercontent.com/ggml-org/llama.cpp/master/tools/server/README.md (fetched 29 Oct 2026) |
| vLLM | 0.32.0, 18 Oct 2026 | `--port` default 8000; UNIX socket option | `/v1/chat/completions`, `/v1/responses`, `/v1/messages`, `/v1/messages/count_tokens`. `--grpc` starts a gRPC server **instead of** HTTP (needs `vllm[grpc]`) | SSE on HTTP | `--api-key` (Bearer) | https://pypi.org/pypi/vllm/json; https://docs.vllm.ai/en/latest/serving/online_serving/; https://docs.vllm.ai/en/latest/cli/serve/ (fetched 29 Oct 2026) |
| LiteLLM proxy (self-hosted) | see E6.4 | `http://0.0.0.0:4000` | see E6.4 | SSE | see E6.4 | see E6.4 |

Codex's built-in `ollama` and `lmstudio` providers are in E2.2.

## E6. Gateways and clouds

Every row has its source and the page or release date. "Unverified" means no source was found.

### E6.1 Vercel AI Gateway

| Topic | Fact | Source (date) |
|---|---|---|
| Host | One host, `ai-gateway.vercel.sh` | https://vercel.com/docs/ai-gateway (14 Sep 2026) |
| Paths | `/v1/chat/completions`, `/v1/responses`, `/v1/responses/compact`, `/v1/messages`, `/v1/models`. Per-agent bases that pass straight to `/v1`: Claude Code `/claude-code`, Codex `/codex/v1`, Cursor `/cursor/v1`, others `/coding-agent/v1` | https://vercel.com/docs/ai-gateway/coding-agents (13 Sep 2026); https://vercel.com/docs/ai-gateway/sdks-and-apis/responses (8 Sep 2026) |
| Streaming | SSE on all three wire formats. WebSocket mode for Responses at `wss://ai-gateway.vercel.sh/v1/responses`, same `response.*` events; idle timeout 5 minutes, "Maximum connection duration: Approximately 13 minutes, then close 1001". WebSocket support live since 27 Jul 2026 | https://vercel.com/docs/ai-gateway/sdks-and-apis/responses/websockets (8 Sep 2026); https://vercel.com/changelog/websocket-support-for-openai-responses-api-live-on-ai-gateway (27 Jul 2026) |
| WebSocket in Codex | Opt-in: `responses_websockets_v2 = true` and `supports_websockets = true` in the provider table; works only for one vendor's models, others answer "is not available over WebSocket" | https://vercel.com/docs/ai-gateway/coding-agents/openai-codex (13 Sep 2026) |
| HTTP version | Not documented. Measured: Codex 0.165.0 and Claude Code 2.1.312 talked to it through our HTTP/1.1-only proxy, every call recorded | `docs/research/results/2026-10-vercel-gateway.md:50-61` |
| Auth | `Authorization: Bearer <key>` or `x-api-key`; OIDC token as Bearer (key wins if both). Claude Code with a subscription sends the gateway key in `ANTHROPIC_CUSTOM_HEADERS="x-ai-gateway-api-key: Bearer ..."` | https://vercel.com/docs/ai-gateway/sdks-and-apis/anthropic-messages-api (18 Sep 2026); claude-code page (13 Sep 2026) |
| OIDC lifetime | "Vercel OIDC tokens are only valid for 12 hours" | https://vercel.com/docs/ai-gateway/authentication-and-byok/oidc (8 Sep 2026) |
| Setup tool | `vercel ai-gateway setup` writes `~/.claude/settings.json` and, on macOS, exports `ANTHROPIC_AUTH_TOKEN` from the shell startup file via the Keychain. For Codex it can copy rollout files to a new session id with `model_provider = "vercel"` | claude-code page and openai-codex page (13 Sep 2026) |
| Rate limits | Paid tier: no gateway limit, provider limits only. Free tier: lower per-model limits. 429 body `{"error":{"message":"Rate limit exceeded","type":"rate_limit_exceeded"}}`; "Some 429 responses include a `retry-after` header" | https://vercel.com/docs/ai-gateway/rate-limits (8 Sep 2026) |
| Failover | Gateway routes across providers and fallback models; budgets are soft caps; a failed BYOK call "can fall back to system credentials" | https://vercel.com/docs/ai-gateway (14 Sep 2026) |
| Outages | Vercel status feed (last 25 items, 1 Sep to 23 Oct 2026): no AI Gateway incident; "Elevated Sandbox error rates in Paris (cdg1)" 6 Oct; "Elevated latency in VCR, Blob, and Sandbox API" 4 Sep | https://www.vercel-status.com/history.atom (fetched 29 Oct 2026) |
| SDKs | `ai` 7.0.156 and `@ai-sdk/gateway` 4.0.129 (both 28 Oct 2026); runtime `fetch`, so the Node proxy rules in E7 apply | https://registry.npmjs.org/ai, https://registry.npmjs.org/@ai-sdk%2Fgateway |
| Pinning, DoH | None found | as above |

Sandbox facts that touch the gateway: the firewall "terminates TLS" for any domain with a `transform`
rule (credential brokering); ordinary allowed domains are matched by SNI only and not decrypted
(https://vercel.com/docs/sandbox/concepts/firewall, 16 Sep 2026). Max session 24 hours on Pro
(https://vercel.com/docs/sandbox/pricing, 10 Sep 2026).

### E6.2 OpenRouter

| Topic | Fact | Source (date) |
|---|---|---|
| Host and base | `https://openrouter.ai/api/v1` | https://openrouter.ai/docs/api-reference/overview (no date) |
| Paths | `/api/v1/chat/completions`, `/api/v1/responses` ("OpenAI-compatible Responses API"), `/api/v1/messages` (Anthropic Messages format), `/api/v1/embeddings`, `/api/v1/key`, `/api/v1/generation?id=`. Batches accept the same three model endpoints | https://openrouter.ai/docs/llms.txt (fetched 29 Oct 2026) |
| Claude Code | `ANTHROPIC_BASE_URL="https://openrouter.ai/api"`, key in `ANTHROPIC_AUTH_TOKEN` (sent as Bearer), `ANTHROPIC_API_KEY=""`. Also suggests the project file `.claude/settings.local.json` | https://openrouter.ai/docs/cookbook/coding-agents/claude-code-integration.md (no date) |
| Launcher | "Ori Harness": `ori claude`, `ori codex`, `ori hermes` and others start the real CLI "with OpenRouter credentials ... applied to that session"; `ori harness setup <agent>` writes the agent's own config instead. Ori 0.10.1, 24 Aug 2026 | https://openrouter.ai/docs/guides/ori/harness.md, https://openrouter.ai/docs/guides/ori/changelog.md |
| Streaming | SSE. "OpenRouter occasionally sends comments to prevent connection timeouts", e.g. `: OPENROUTER PROCESSING`. Chat streams end with an extra usage chunk before `[DONE]`, so `finish_reason` appears twice. Mid-stream errors arrive as `data:` events with an `error` object and `finish_reason: "error"`; errors before streaming are plain JSON | https://openrouter.ai/docs/api_reference/streaming.md, https://openrouter.ai/docs/api_reference/errors-and-debugging.md (no date) |
| WebSocket | None documented | as above |
| Auth | `Authorization: Bearer <key>`; optional `HTTP-Referer`, `X-OpenRouter-Title` (`X-Title` accepted) | api-reference overview |
| Rate limits | Global per account, "Making additional accounts or API keys will not affect your rate limits"; `:free` variants have per-minute and per-day caps; `X-RateLimit-*` headers on the error response; 402 for credits and an "in-flight spending budget"; 408, 429, 502, 503 | https://openrouter.ai/docs/api_reference/limits.md, errors page |
| Outages | Status feed not reachable from here (`status.openrouter.ai` answered "Status page not found" / AccessDenied). Unverified | fetched 29 Oct 2026 |
| SDKs | `@openrouter/sdk` 1.5.21 (29 Oct 2026, only dependency `zod`, so runtime `fetch`); PyPI `openrouter` 1.3.94 (29 Oct 2026) | npm, PyPI |
| HTTP version, pinning | Not documented; none found | |

### E6.3 Cloudflare AI Gateway

| Topic | Fact | Source (date) |
|---|---|---|
| Hosts | `gateway.ai.cloudflare.com` (provider-native and `compat`), `api.cloudflare.com` (newer REST API), or a custom domain (with Cloudflare Access) | https://developers.cloudflare.com/ai-gateway/configuration/authentication/ (17 Jun 2026) |
| Paths | Account and gateway are in the path: `/v1/{account_id}/{gateway_id}/anthropic/v1/messages`, `/v1/{account_id}/{gateway_id}/openai/responses`, `.../openai/chat/completions`, `.../compat/chat/completions` (now "deprecated" for single-model use), `.../aws-bedrock/bedrock-runtime/{region}/...`, `.../google-vertex-ai/v1/...`. REST: `api.cloudflare.com/client/v4/accounts/{id}/ai/v1/chat/completions` and `/ai/run` | https://developers.cloudflare.com/ai-gateway/usage/providers/anthropic/ (27 Oct 2026); .../providers/openai/ (20 Apr 2026); .../usage/chat-completion/ (2 Oct 2026); .../usage/rest-api/ (9 Oct 2026) |
| Claude Code | `ANTHROPIC_BASE_URL=https://gateway.ai.cloudflare.com/v1/<ACCOUNT_ID>/<GATEWAY_ID>/anthropic` plus `ANTHROPIC_CUSTOM_HEADERS="cf-aig-authorization: Bearer <token>"`. Bedrock and Vertex variants use `ANTHROPIC_BEDROCK_BASE_URL` / `ANTHROPIC_VERTEX_BASE_URL`. With Access: `apiKeyHelper = "cloudflared access login ..."` | https://developers.cloudflare.com/ai-gateway/integrations/coding-agents/claude-code/ (17 Aug 2026) |
| Codex | A profile file `~/.codex/cloudflare-aig.config.toml`, run with `codex --profile cloudflare-aig`; `base_url = ".../openai"`, `wire_api = "responses"`; with Access, a `[model_providers.X.auth]` command running `cloudflared` | https://developers.cloudflare.com/ai-gateway/integrations/coding-agents/openai-codex/ (5 Aug 2026) |
| Streaming | Whatever the provider sends (SSE). WebSockets: realtime `wss://gateway.ai.cloudflare.com/v1/<acct>/<gw>/openai?model=...` and `.../google?api_key=...`; non-realtime `wss://gateway.ai.cloudflare.com/v1/{acct}/{gw}` with `universal.create` frames | .../websockets-api/ (12 Jun 2026), .../non-realtime-api/ (8 May 2026), .../realtime-api/ (20 Apr 2026) |
| Auth | Gateway: `cf-aig-authorization: Bearer <token>` (or `sec-websocket-protocol` from a browser). Provider key: `x-api-key` on the Anthropic route ("does not recognize `Authorization: Bearer` as provider credentials"), `Authorization` elsewhere. Realtime keys may sit in the query (`api_key=`) | anthropic page (27 Oct 2026), non-realtime page (8 May 2026), realtime page (20 Apr 2026) |
| Rate limits | Per gateway, fixed or sliding window, `429 Too Many Requests`. Retry, timeout and fallback headers `cf-aig-max-attempts`, `cf-aig-retry-delay`, `cf-aig-request-timeout`, `cf-aig-backoff`; cache headers `cf-aig-cache-*` | .../features/rate-limiting/ (30 Sep 2026), .../glossary/ (15 Sep 2026) |
| Recent changes | 8 Oct 2026: log and request ids are UUIDv7; 6 Oct 2026: rejected provider credentials now HTTP 401 code 2009 (Vertex no longer retried; Unified Billing gets 503) | https://developers.cloudflare.com/ai-gateway/changelog/ |
| Caching | A cached answer comes from Cloudflare, not the provider; to the client it is still one HTTP exchange | https://developers.cloudflare.com/ai-gateway/ (20 Apr 2026) |
| Outages | Cloudflare status feed, last 25 items: none name AI Gateway or Workers AI | https://www.cloudflarestatus.com/history.atom (fetched 29 Oct 2026) |
| HTTP version, pinning | Not documented; none found | |

### E6.4 LiteLLM proxy

| Topic | Fact | Source (date) |
|---|---|---|
| Where it runs | Self-hosted. Docs default: `http://0.0.0.0:4000` ("RUNNING on http://0.0.0.0:4000") | https://docs.litellm.ai/docs/tutorials/claude_responses_api (no date) |
| Paths | Unified `/v1/messages` (Claude Code base `http://0.0.0.0:4000`), pass-through `/anthropic`, `/v1/responses`, `/v1/chat/completions`, `/health` | same; https://docs.litellm.ai/docs/response_api (no date) |
| Streaming | SSE. WebSocket mode on `/v1/responses` "automatically available", e.g. `ws://localhost:4000/v1/responses?model=...`; sessions "default to 60 minutes", closed with code 1000 | response_api page |
| Auth | `Authorization: Bearer sk-...` (master or virtual key); configurable header (`litellm_key_header_name`, example `X-Litellm-Key`); Claude Code helper value sent "as `Authorization` and `X-Api-Key` headers" | https://docs.litellm.ai/docs/proxy/virtual_keys, tutorial (no date) |
| Rate limits | Per key `tpm_limit`, `rpm_limit`; response headers not documented on that page | virtual_keys page |
| Release | PyPI `litellm` 1.106.6, 28 Oct 2026 (client proxy env handling in E4.2) | https://pypi.org/pypi/litellm/json |
| HTTP version, pinning | Not documented. Unverified | |

### E6.5 Amazon Bedrock

| Topic | Fact | Source (date) |
|---|---|---|
| Hosts | `bedrock-runtime.{region}.amazonaws.com` ("recommended" for most new apps) and `bedrock-mantle.{region}.api.aws` (server-side tools, background runs, projects). "Both endpoints use the same underlying Mantle inference engine" | https://docs.aws.amazon.com/bedrock/latest/userguide/endpoints.html (no date, fetched 29 Oct 2026) |
| Native paths | `POST /model/{modelId}/invoke`, `/invoke-with-response-stream`, `/converse`, `/converse-stream`, `/invoke-with-bidirectional-stream` | https://docs.aws.amazon.com/bedrock/latest/APIReference/API_runtime_InvokeModelWithResponseStream.html |
| Compatible paths | `bedrock-runtime`: `/anthropic/v1/messages`, `/openai/v1/chat/completions`, `/openai/v1/responses`. `bedrock-mantle`: `/anthropic/v1/messages`, `/v1/chat/completions`, `/openai/v1` Responses | https://docs.aws.amazon.com/bedrock/latest/userguide/inference-messages-api.html, endpoints page |
| Streaming | Native: AWS event-stream, a binary framing (content type `application/vnd.amazon.eventstream`, general knowledge, not on the page read); errors such as `throttlingException`, `modelStreamErrorException`, `serviceUnavailableException` arrive inside the stream. Compatible paths: SSE, same as the vendor API | InvokeModelWithResponseStream reference; messages page |
| HTTP version | AWS: "all our APIs support http2" (said for the bidirectional stream, which needs it); boto3 and the Anthropic SDKs use HTTP/1.1 by default (general knowledge) | https://docs.aws.amazon.com/nova/latest/userguide/speech-bidirection.html (no date) |
| Auth | SigV4: `Authorization: AWS4-HMAC-SHA256 Credential=..., Signature=...`, `X-Amz-Date`, `X-Amz-Security-Token`. Or a Bedrock API key: `Authorization: Bearer $AWS_BEARER_TOKEN_BEDROCK`; on the Messages route also `x-api-key` | https://docs.aws.amazon.com/bedrock/latest/userguide/api-keys-use.html; messages page |
| Claude Code and SDK | `@anthropic-ai/bedrock-sdk` 1.1.0 (23 Oct 2026): base `ANTHROPIC_BEDROCK_BASE_URL` or `https://bedrock-runtime.${region}.amazonaws.com`, paths `/model/${model}/invoke-with-response-stream`, transport is the Anthropic client's `fetch` | https://unpkg.com/@anthropic-ai/bedrock-sdk@1.1.0/client.js lines 92, 224 |
| Codex | Built-in `amazon-bedrock` provider: no `base_url`; derives `https://bedrock-mantle.{region}.api.aws/openai/v1`, Responses wire, AWS auth, header `x-amzn-mantle-client-agent: codex`, WebSockets off | `model-provider-info/src/lib.rs#L84-L103`, `#L573-L611`; `model-provider/src/amazon_bedrock/mantle.rs#L56` (rust-v0.165.0, 27 Oct 2026) |
| Rate limits | `bedrock-runtime`: fixed per-model RPM and TPM quotas; `bedrock-mantle`: "work-queueing" with "higher initial throughput limits" | endpoints page |
| Proxy env | boto3/botocore: honours `HTTPS_PROXY` (`getproxies()`), but trusts only `certifi` or its own `cacert.pem` (or `AWS_CA_BUNDLE`), never `SSL_CERT_FILE` or `REQUESTS_CA_BUNDLE`. AWS SDK for JS v3: proxies only "through a third-party HTTP agent" passed in code | https://github.com/boto/botocore/blob/develop/botocore/httpsession.py#L95-L110, `botocore/utils.py` `get_environ_proxies` (develop, fetched 29 Oct 2026); boto3 1.43.121 (28 Oct 2026); https://docs.aws.amazon.com/sdk-for-javascript/v3/developer-guide/node-configuring-proxies.html (no date) |
| Pinning, DoH | None found | |
| Outages | Not checked (AWS health feed not read). Unverified | |

### E6.6 Google Vertex AI

Google's docs now title this section "Agent Platform"; the URLs still say `vertex-ai`.

| Topic | Fact | Source (date) |
|---|---|---|
| Hosts | `{LOCATION}-aiplatform.googleapis.com`, global `aiplatform.googleapis.com`, and multi-region `aiplatform.us.rep.googleapis.com`, `aiplatform.eu.rep.googleapis.com` | https://docs.cloud.google.com/vertex-ai/generative-ai/docs/partner-models/claude/use-claude (28 Oct 2026); https://unpkg.com/@anthropic-ai/vertex-sdk@1.1.0/client.js lines 85-103 (23 Oct 2026) |
| Anthropic models | `POST https://{LOCATION}-aiplatform.googleapis.com/v1/projects/{P}/locations/{L}/publishers/anthropic/models/{M}:streamRawPredict` (`:rawPredict` non-stream), body `"anthropic_version": "vertex-2023-10-16"`; token counting at `.../count-tokens:rawPredict`. No `/v1/messages` path | use-claude page; vertex-sdk client.js lines 227-243 |
| Google's own models | `POST https://{service-endpoint}/v1/{model}:streamGenerateContent`; SSE with `alt=sse`, otherwise a streamed JSON array (`alt=sse` behaviour from general knowledge, not on the page read) | https://docs.cloud.google.com/vertex-ai/generative-ai/docs/reference/rest/v1/projects.locations.endpoints/streamGenerateContent (22 Oct 2026) |
| OpenAI-compatible | `.../v1/projects/{P}/locations/{L}/endpoints/openapi/chat/completions`; Codex's `gcp` provider uses `https://aiplatform.googleapis.com/v1/projects/{project}/locations/{location}/publishers/openai/v1` with the Responses wire | https://docs.cloud.google.com/vertex-ai/generative-ai/docs/migrate/openai/overview (28 Oct 2026); `model-provider/src/gcp/mod.rs#L209` (rust-v0.165.0) |
| WebSocket | Live API: `wss://{location}-aiplatform.googleapis.com/ws/google.cloud.aiplatform.v1.LlmBidiService/BidiGenerateContent` | https://docs.cloud.google.com/vertex-ai/generative-ai/docs/live-api/get-started-websocket (no date read) |
| gRPC | Python `google-cloud-aiplatform` 2.7.0 (28 Oct 2026) takes `api_transport` "grpc" or "rest"; API keys only with "rest". gRPC needs HTTP/2. gRPC reads `grpc_proxy`, then `https_proxy`, then `http_proxy`, and roots from `GRPC_DEFAULT_SSL_ROOTS_FILE_PATH` | https://github.com/googleapis/python-aiplatform/blob/main/google/cloud/aiplatform/initializer.py lines 154-238; https://github.com/grpc/grpc/blob/master/doc/environment_variables.md (fetched 29 Oct 2026) |
| Auth | `Authorization: Bearer $(gcloud auth print-access-token)` (OAuth access token, about one hour); API key in "express mode" (`?key=` or `x-goog-api-key`, general knowledge) | use-claude page |
| SDKs | `@google/genai` 2.32.0 (29 Oct 2026) depends on `ws` (Live API); PyPI `google-genai` 2.34.0 (29 Oct 2026) | npm, PyPI |
| Rate limits | Per-model quotas, 429 "Resource exhausted"; a "Troubleshooting error code 429" page exists (not read) | use-claude page nav |
| Pinning | None found. mTLS endpoints (`*.mtls.googleapis.com`) with client certificates exist (general knowledge); a client certificate cannot pass a decrypting proxy | |

### E6.7 Azure OpenAI and Azure AI Foundry (Microsoft Foundry)

| Topic | Fact | Source (date) |
|---|---|---|
| Hosts | `{resource}.openai.azure.com` and `{resource}.services.ai.azure.com` (both accept `/openai/v1/`); the Foundry SDK comment also names `example-resource.azure.anthropic.com` | https://learn.microsoft.com/en-us/azure/foundry/openai/api-version-lifecycle (ms.date 13 May 2026, updated 5 Jun 2026); https://unpkg.com/@anthropic-ai/foundry-sdk@1.1.0/client.js line 65 |
| v1 API | Since August 2025: `https://{resource}.openai.azure.com/openai/v1/responses` and `/chat/completions`, "api-version is no longer a required parameter" | lifecycle page |
| Legacy API | `/openai/deployments/{deployment}/chat/completions?api-version=...` (general knowledge; still accepted) | |
| Anthropic models | Base `https://{resource}.services.ai.azure.com/anthropic`, target `.../anthropic/v1/messages`, header `anthropic-version: 2023-06-01`; Claude Code switch `CLAUDE_CODE_USE_FOUNDRY` with `ANTHROPIC_FOUNDRY_RESOURCE` or `ANTHROPIC_FOUNDRY_BASE_URL` | https://learn.microsoft.com/en-us/azure/foundry/foundry-models/how-to/use-foundry-models-claude (ms.date 21 Sep 2026); foundry-sdk client.js lines 74-89 |
| Auth | `api-key: <key>` or `Authorization: Bearer <Entra token>` (scope `https://ai.azure.com/.default`); on the Anthropic route `x-api-key`. Some models are "Entra ID authentication only" | lifecycle page; use-foundry-models page; foundry-sdk client.js line 145 |
| Streaming | SSE (OpenAI and Anthropic formats). Realtime over WebSocket (`wss://.../openai/v1/realtime`, general knowledge, unverified for v1) | |
| Rate limits | `x-ratelimit-limit-requests`, `-tokens`, `-remaining-*`, `-reset-*`; `retry-after-ms` on 429. Standard deployments may get a "temporary rate limit adjustment" that "typically resolves within a few hours" | https://learn.microsoft.com/en-us/azure/foundry-classic/openai/how-to/quota (ms.date 4 May 2026) |
| SDKs | `azure-identity` 1.27.0 (26 Oct 2026), `@azure/identity` 4.14.0 (13 Oct 2026), `@anthropic-ai/foundry-sdk` 1.1.0 (23 Oct 2026, transport `fetch`) | PyPI, npm |
| HTTP version, pinning | Not documented; none found | |

## E7. Everything that can skip `HTTPS_PROXY`

`goalcup start` sets `HTTPS_PROXY`, `https_proxy`, `NO_PROXY=127.0.0.1,localhost` and the profile's CA
variable, and nothing else (`crates/goalcup-cli/src/launch.rs:319-320`, `:855-861`). It sets no
`HTTP_PROXY` or `ALL_PROXY`, and the proxy answers anything but `CONNECT` with 405
(`crates/goalcup-cli/src/proxy/connect.rs:701`, `:796`). Plain-HTTP model hosts are refused anyway
(`crates/goalcup-cli/src/model_host.rs:16-19`).

| Path | Skips or breaks when | Who is affected, and is it seen in our record? | Source |
|---|---|---|---|
| Node global `fetch` | Node before 22.21.0 (20 Oct 2025), or Node 23.x, or 24.0 to 24.4 for the CLI flag: `NODE_USE_ENV_PROXY` is ignored. Also any SDK caller that passes its own dispatcher or agent | Not Claude Code (Bun binary) or Codex (Rust) per lane D. Yes for a Node harness on old Node (Amp connector sets `NODE_USE_ENV_PROXY`, unverified which Node it runs). Also `groq-sdk`, `@openrouter/sdk`, `ai`, Anthropic TS SDKs. Not recorded | https://nodejs.org/api/cli.json (`NODE_USE_ENV_PROXY` added v24.0.0, v22.21.0); https://nodejs.org/dist/index.json |
| `ws` WebSockets in Node | Always, unless the caller passes an agent | Any Node harness that uses the OpenAI Responses or Realtime WebSocket through `openai-node`; `@google/genai` Live API. Not recorded | E2.3 |
| Python `websockets` 13 or 14 | Always | A Python harness that uses OpenAI WebSocket mode with an old `websockets` | E2.3 |
| aiohttp-based clients without `trust_env` | aiohttp ignores proxy env by default | `fireworks-ai` (unverified); `litellm` with `DISABLE_AIOHTTP_TRUST_ENV=True`. Tunnel list does not see it (no CONNECT) | aiohttp discussion, E4.2 |
| AWS SDK for JS v3 | Never reads proxy env; needs an agent in code | A Node tool using the AWS SDK directly. Not the Anthropic Bedrock SDK, which uses `fetch` for model calls | AWS JS proxy page; bedrock-sdk client.js |
| boto3 | Goes through the proxy, but trusts only `certifi`. If its host were decrypted, every call fails TLS | Hermes's Bedrock path (lane D: `agent/bedrock_adapter.py` ignores `ssl_ca_cert`), any agent tool using boto3 | botocore `httpsession.py#L95-L110` |
| gRPC (Vertex Python default) | Goes through the proxy by CONNECT. If decrypted: offers only `h2`, so our ALPN check refuses it (`crates/goalcup-cli/src/proxy/connect.rs:624-636`), and it trusts its own roots | Any tool using `google-cloud-aiplatform` with the default transport | gRPC env doc; aiplatform initializer |
| Harness config overrides our env | Claude Code `settings.json` `env` block; `$CODEX_HOME/.env`; `$DSH_HOME/.env` | Yes for both shipped connectors. See lane D (`parts/D-claude-code.md` risk 5, `parts/D-codex.md` risk 3) and lane L | lane D; E3.2 |
| Ori launcher | `ori claude` starts the harness itself, outside `goalcup start` | An OpenRouter user who launches through Ori | Ori harness page |
| Any `http://` model URL (local, LAN, or a plain-http gateway) | No `HTTP_PROXY` is set, so clients connect directly | Refused at prep and start when the connector reads the URL (`model_host.rs:450-452`); silent when it does not | repo |
| Loopback (`localhost`, `127.0.0.1`) | Our `NO_PROXY`. Our launch overwrites any inherited value with loopback only. Go's `ProxyFromEnvironment` returns no proxy for `localhost` whatever the env says (https://pkg.go.dev/net/http#ProxyFromEnvironment, go1.27.3, fetched 29 Oct 2026). dsh always bypasses `127.0.0.0/8`, `::1`, `0.0.0.0` (E3.2) | Refused when read (`crates/goalcup-protocol/src/capture_policy.rs:1017-1030`); silent otherwise (`docs/CHEAT-CHECKLIST.md:136-143`) | as cited |
| LiteLLM on loopback | `http://0.0.0.0:4000` is plain HTTP and loopback: never proxied | A participant with a local LiteLLM. Refused at start (`model_host.rs:451-452`) | LiteLLM tutorial |
| Codex `model_provider = "ollama"` or `"lmstudio"`, or `CODEX_OSS_BASE_URL` in `$CODEX_HOME/.env` | The reader looks for a `[model_providers.<name>]` table, finds none, and under `absent = "defaults"` records the OpenAI hosts instead (`connectors/codex-cli/harness.toml:33-37`, test `model_host.rs:974-979`) | No refusal; run labelled `native_only` | `parts/D-codex.md:261`, `:282` |
| Ollama cloud models | Harness to local daemon is loopback; daemon to `ollama.com` is another process with its own env | Not recorded | Ollama FAQ |
| Hermes `<PROVIDER>_BASE_URL` in `$HERMES_HOME/.env` pointing at a local server | The reader reads `model.base_url` only | No refusal; calls go direct | `docs/harnesses/hermes-cli.md:105` |
| UNIX sockets (llama.cpp `.sock`, vLLM socket) | Not TCP | Not recorded | READMEs in E5 |
| Cloudflare Access `apiKeyHelper` / Codex `auth` command (`cloudflared`) | Go program that must reach the custom gateway domain; if that domain is the decrypted model host, `cloudflared` sees our certificate. Go on Linux reads `SSL_CERT_FILE`; on macOS it uses the system verifier (general knowledge, unverified for current Go) | Cloudflare Access users on macOS | Cloudflare claude-code and codex pages |
| WebRTC (Realtime calls) | Media goes over UDP, never through an HTTP proxy | Only Codex voice, opt-in (lane D) | `codex-api/src/endpoint/realtime_call.rs` |
| HTTP/3 (QUIC) | A client that follows `alt-svc: h3` and does not use the proxy. A client that uses a CONNECT proxy cannot use QUIC. `api.openai.com`, Z.ai, Groq and Together advertise `h3` | No shipped harness or SDK found using h3 for model calls | probes 29 Oct 2026 |
| DNS over HTTPS | Only matters for a client that already skips the proxy; with a CONNECT proxy the proxy resolves names | None found in Codex or any SDK here; Claude Code unverified | `rg` over Codex 0.165.0 |
| mTLS (`*.mtls.googleapis.com`) | A client certificate cannot be relayed by a decrypting proxy | Rare for coding agents | general knowledge |
| Instance metadata (`169.254.169.254`, `metadata.google.internal`) | Plain HTTP, goes direct | Only for credentials on cloud VMs; not model calls | general knowledge |
| LAN host by DNS name with a **private CA** (common for a home GPU box) | Not a skip: it is intercepted, then our upstream TLS check uses only the compiled-in Mozilla roots (`crates/goalcup-cli/src/proxy/tls.rs:21-25`, `:231-234`), so every call fails. The start preflight is a bare TCP connect (`proxy/preflight.rs:10-13`) and passes. The harness is blocked, not just unrecorded (point 3) | Failed handshakes counted. Corporate TLS inspection has the same effect (`parts/F-os.md:33`, `:276`) | repo |

## E8. Compare with this repo

### E8.1 What our code and data assume

Rows that more than one part checked are merged; the reality column joins their findings.

| Assumption (path:line) | Reality | Verdict |
|---|---|---|
| The model host is whatever one key in the harness's config names; "There is no provider list: whatever service the participant set up is the one recorded" (`crates/goalcup-cli/src/model_host.rs:1-8`, `:5-7`; Hermes `connectors/hermes-cli/harness.toml:62-68`; Claude Code `connectors/claude-code-cli/harness.toml:37-40`; Codex `connectors/codex-cli/harness.toml:33-37`) | E2: right for every provider in E4; each is set up by naming one base URL in exactly those keys (Claude Code `ANTHROPIC_BASE_URL`, Codex `[model_providers.*] base_url`, Hermes `model.base_url`). E3: true for a base URL, false for the cloud switches, which name no base URL in the key we read | Keep the rule. E3: change the comment to say "one address per connector" |
| Anthropic host row `api.anthropic.com`, evidence path `/v1/messages`, hint `anthropic-messages` (`crates/goalcup-protocol/providers.toml:16-19`) | Right. Other paths on the host (`count_tokens`, `/v1/models`, `/v1/oauth/token`) are still captured, with no hint, because capture is per host, not per path | Keep the row. Cut the shared table (see below) |
| `providers.toml` row `deepseek`: `api.deepseek.com`, evidence path `/v1/chat/completions` (`crates/goalcup-protocol/providers.toml:21-25`) | Used by no shipped connector (`connectors/*/harness.toml`: only `providers = ["anthropic"]` at `connectors/claude-code-cli/harness.toml:36`); Hermes dropped it (`docs/DECISIONS.md:630`). DeepSeek's docs now use `/chat/completions` without `/v1` | Cut (with the whole table) |
| `providers.toml` "Frozen (D74): no new rows" (`crates/goalcup-protocol/providers.toml:10-11`) | Fine as a rule. No gateway or cloud row is needed: base URLs cover them. But the table now serves two rows for two connectors, and adds a loader (`crates/goalcup-protocol/src/harness/schema.rs:846-900`) and a `providers` field for no gain | Cut (E1, E2); E3 "keep frozen; the sibling's cut stands" |
| Codex hosts `chatgpt.com` `/backend-api/codex/responses` and `api.openai.com` `/v1/responses` (`connectors/codex-cli/harness.toml:122-130`) | Right for both sign-ins on 0.165.0. Not covered: `gov.chatgpt.com`, data residency hosts, new built-in providers (lane D) | Keep. Add `gov.chatgpt.com` only if a participant needs it |
| `auth_hosts` `auth.openai.com`, `platform.claude.com` stay opaque (`connectors/codex-cli/harness.toml:32`, `connectors/claude-code-cli/harness.toml:34`, `crates/goalcup-protocol/src/harness/schema.rs:475-477`) | Right: token exchange and refresh live there | Keep (point 4) |
| Hint rows `/chat/completions`, `/messages`, `/responses` (`WIRE_PATHS`) under `<base>` and `<base>/v1`; the rest of the base captured with no hint (`crates/goalcup-cli/src/model_host.rs:42-48`, `:44-48`, `:83-114`, `:86-112`, `:87-112`, `:90-98`) | Right for both wire formats and for WebSocket mode on a gateway (`GET <base>/responses` upgrade). Matches DeepSeek `/chat/completions` and `/anthropic/v1/messages`; Moonshot `/v1/...` and `/anthropic/v1/messages`; Z.ai `/api/anthropic/v1/messages`, `/api/coding/paas/v4/chat/completions`, `/api/v1/responses`; Groq `/openai/v1/...`; Fireworks `/inference/v1/messages`; OpenRouter's `/api` + `/v1/messages`; Cloudflare's `/compat`; Azure's `/openai/v1` | Keep |
| Capture is per host, whatever the path (`crates/goalcup-cli/src/proxy/http.rs:11-16`, `proxy/metadata.rs:134-147`); a POST there counts as a model call (`metadata.rs:199-210`) | Good: provider extras (`/models`, `count_tokens`, the Ollama native `/api/chat` on `ollama.com`, Vercel `/codex/v1/responses/compact`) are still captured, just without a decoder | Keep |
| Every model host is a DNS name; refuse plain http, loopback, IP literals, single-label and numeric names (`model_host.rs:450-475`, `crates/goalcup-protocol/src/capture_policy.rs:1001-1047`, `:1016-1044`) | Correct: every remote service uses DNS names, and none of the refused forms can pass our proxy (E7). The capture_policy message still says "a local model is not supported by schema 1" (`capture_policy.rs:1027`); "schema 1" means nothing to a participant | Keep the rule; change the words in that one string |
| Refusal text "Goal Cup records only an https model service that is not on this computer" (`crates/goalcup-cli/src/participant/refusals.rs:98-108`) | Clear. But a LAN machine reached by IP is also refused with "an IP address" wording, and a LAN name with a private CA passes and then fails every call | Change: say "not on this computer or your local network, with a public certificate" (one string) |
| Path at most 512 bytes, no `%2e`, no odd bytes (`model_host.rs:845-852`, `capture_policy.rs:1071-1076`) | Longest real base seen: Cloudflare's `/v1/<32 hex>/<gateway>/aws-bedrock/bedrock-runtime/<region>/`, well under 512 | Keep |
| `NO_PROXY=127.0.0.1,localhost` (`launch.rs:319-320`) | Needed, so the harness reaches our own loopback proxy. Does not list `::1`; harmless, because nothing here is reached over IPv6 loopback with HTTPS | Keep |
| HTTP/1.1 only, h2-only refused, both legs (`crates/goalcup-cli/src/proxy/tls.rs:24-31`, `:50`, `:213`, `:275`, `:280-283`; `docs/ARCHITECTURE.md:211`) | Every service in scope accepts HTTP/1.1. Codex (reqwest) offers both and falls back; Python SDKs default to HTTP/1.1; Claude Code measured HTTP/1.1. Only gRPC needs h2, and only on a host we open. Local servers are plain HTTP and never reach us | Keep. Lane C's h2-only tunnel fallback is cheap insurance |
| No idle timeout in the relay (only connect and admission timeouts, `proxy/connect.rs:79-95`) | DeepSeek can hold a request up to 10 minutes with keep-alive lines before inference. No proxy timeout cuts it | Keep. Do not add a relay idle timeout |
| zstd window up to 8 MiB (`crates/goalcup-cli/src/proxy/http.rs:100-102`) | Codex encodes request bodies with zstd level 3, which uses a window well under 8 MiB (zstd level table, unverified for very large bodies) | Keep |
| WebSocket relay with permessage-deflate (`crates/goalcup-cli/src/proxy/websocket.rs`) | Required: Codex enables permessage-deflate (`responses_websocket.rs#L578`) | Keep |
| CA name-constrained to approved hosts known at start, valid 8 days (`crates/goalcup-cli/src/proxy/tls.rs:15-19`, `:55`, `:131-137`) | Fits 168 hours plus a day. Each `continue` mints a new CA. Correct for safety, but a new host cannot be opened mid-run without a new CA, so a mid-run change of service needs `goalcup continue` | Keep; document |
| Header masking by name parts `key`, `token`, `secret`, `password`, `auth`, plus `cookie`, plus any `Bearer`/`Basic` value (`crates/goalcup-cli/src/proxy/headers.rs:23-30`); query values in the start line masked (`headers.rs:315-322`) | Masks `Authorization`, `x-api-key`, `anthropic-ratelimit-*tokens*`, `x-ratelimit-*-tokens`, `x-ai-gateway-api-key`, `cf-aig-authorization`, `api-key`, `x-goog-api-key`, SigV4 `Authorization`, `X-Amz-Security-Token`, `?api_key=`, `?key=`, JWT-signed Bearer. Does **not** mask `chatgpt-account-id` (an account identifier), `x-codex-turn-state` (an opaque server token the connector strips from JSON bodies, `connectors/codex-cli/harness.toml:101-117`), `OpenAI-Organization`/`-Project`, `anthropic-workspace-id`, or `Sec-WebSocket-Protocol` (holds a key for browser Realtime; the daemon's `openai_key` rule catches an `sk-` key, but not an ephemeral key, `crates/goalcup-daemon/src/redact.rs:197-198`) | Change: owner to decide whether account ids count as secrets. If yes, add `account-id` and `turn-state` to the name list (2 words). Do not add more rules (owner masking decision b) |
| Token bodies from `/v1/oauth/token` on an intercepted host | Masked by the JSON key rule (`access_token`, `refresh_token`) and `jwt`, `sk-ant-` rules (`crates/goalcup-daemon/src/redact.rs:197-261`) | Keep |
| Body redaction vendor rules (`crates/goalcup-daemon/src/redact.rs:198`, `:223`, `:224-226`, `:288-295`, `:289`) | DeepSeek, Moonshot and OpenRouter keys start with `sk-` and are caught; `AKIA` and `AIza` are caught. Groq (`gsk_`), Fireworks (`fw_`), Together (`tgp_v1_`), Z.ai (`<hex>.<secret>`) key shapes (prefixes from common use, unverified), `ASIA` temporary key ids, Bedrock API keys, Google `ya29.` tokens and Azure keys have no vendor rule. They are still caught as `NAME=value` or JSON `"api_key": "..."` (`redact.rs:261`, `:277`), and as Bearer values. A bare key printed alone in a tool output is not | Keep (owner decision b: no new rules beyond the URL password). Name it as a known residual in the participant guide; flag to lane G |
| Anthropic decoder: request blocks `text`, `tool_result`; response blocks `text`, `thinking`, `tool_use`; four delta types; no WebSocket form; path ends `/messages`; SSE or JSON error (`crates/goalcup-verifier/src/capture_analysis/anthropic.rs:12-26`, `:16-26`, `:95-98`, `:538-567`) | Correct shape for every compatible route. New blocks (`server_tool_use`, `web_search_tool_result`, `fallback`, `redacted_thinking`) become counted `unknown` (`anthropic.rs:1271-1283`), which does not break a message. Bedrock native and Vertex `rawPredict` would be `unsupported_endpoint` or `unsupported_content_type` | Keep as is; never let unknown counts fail a run. Do not add event-stream or rawPredict decoders before Friday: the doc route is cheaper |
| Decoders pick a stream by exact hint only (`anthropic.rs:91-93`, `openai.rs:106-115`, `chat.rs:117-123`), need `application/json` requests and `text/event-stream` or JSON-error responses (`anthropic.rs:538-567`, `tracker.rs:151-164`) | Fits every gateway and compatible route | Keep |
| OpenAI decoder: `response.create`, a closed list of quiet events, terminal events, `output_item.done`, `error`; wire and WebSocket sources (`crates/goalcup-verifier/src/capture_analysis/openai.rs:106-115`, `:132-143`, `:310`, `:386-392`) | The real stream has more: `response.metadata` and `codex.rate_limits` in our own fixture; reasoning summary deltas, annotations, `response.queued`, web search events in the docs. Each is one `unknown` per event. `stream_id` is ignored, so with WebSocket lanes the decoder keeps facts true but cannot say which lane an item came from. Vercel and LiteLLM WebSocket mode use the same events; Vercel closes after about 13 minutes, LiteLLM after 60 | Keep. Change only if cheap: treat unknown `*.delta` events as quiet. Never gate on decoder completeness |
| Chat Completions decoder knows DeepSeek's `prompt_cache_hit_tokens`/`miss_tokens` and `reasoning_content` (`chat.rs:128-145`) | OpenAI also serves this wire; fine. Groq's `x_groq.usage` and any provider-specific usage location is not read, so usage stats can be blank for some providers. Recording is not affected | Keep (lane C-verifier owns the call). Never gate a run on decoder usage |
| Error bodies are kept as safe `code`/`type` tokens (`chat.rs:68-70`) | DeepSeek 401 is `text/plain`; Z.ai uses numeric string codes and on `/api/v1/responses` returns **HTTP 200** with an error body. Both become `unknown` or odd tokens in the report | Keep. Report detail only |
| SSE framer skips comment lines and handles CR, LF, CRLF (`crates/goalcup-verifier/src/capture_analysis/wire.rs:543-544`, `:599-615`) | DeepSeek's `: keep-alive` and OpenRouter's `: OPENROUTER PROCESSING` comments are handled. A non-stream JSON body after DeepSeek's empty lines parses, because JSON allows leading whitespace | Keep |
| Tunnel list: at most 10,000 rows, the rest only counted (`crates/goalcup-cli/src/proxy/tunnels.rs:19-20`) | A week of `npm install` and `git` easily passes 10,000 tunnels. A model host first used after that is not named | Change: keep one row per host and port with counts and bytes (bounded by distinct hosts, not by time) |
| Outage windows only for approved and sign-in hosts (`crates/goalcup-cli/src/proxy/outage.rs:1-11`) | A gateway is an approved host, so its outages are counted. Gateway-side failover hides provider outages from us; we see only the gateway's answer | Keep |
| Connector env deny list blocks `AWS_`, `GOOGLE_`, `AZURE_`, `VERTEX_`, `CLOUD_ML_` names (`crates/goalcup-protocol/src/capture_policy.rs:273-283`) and seed content `bedrock`, `vertex`, `foundry` (`:815-826`) | Right: these govern what a connector may set, not what the participant sets. It also means we can never hand `AWS_CA_BUNDLE` or similar to a tool | Keep |
| `ca_bundle.rs` hands shared variables a system-plus-proxy bundle (`crates/goalcup-cli/src/ca_bundle.rs:1-19`) | `httpx2` would trust only our CA if `SSL_CERT_FILE` held only it, and fail on tunnelled hosts | Keep |
| Participant docs | `docs/RULES.md:134-136`: "full freedom of model ... compute". `docs/HARNESS-SETUP.md:201-202` lists services that work, but no doc says a local model is not recorded. Only `docs/CHEAT-CHECKLIST.md:141` mentions `ollama`, for reviewers | Change: one sentence in RULES and the guide |
| `docs/harnesses/hermes-cli.md:75` measured `model.base_url: https://api.deepseek.com/v1` | Still works (probe: `/v1/chat/completions` 401), but the provider's docs now use the base with no `/v1` | Keep; note both forms work |
| Gateway example in the unclean-path refusal: `https://ai-gateway.vercel.sh/v1` (`model_host.rs:491-496`) | Fine | Keep |

### E8.2 What happens to each gateway and cloud setup today

The connectors read one address each: Claude Code `env.ANTHROPIC_BASE_URL` in `settings.json`
(`connectors/claude-code-cli/harness.toml:34-40`), Codex `model_providers.*.base_url` via
`model_provider` (`connectors/codex-cli/harness.toml:32-37`), Hermes `model.base_url`
(`connectors/hermes-cli/harness.toml:64-68`). That address's host is opened and its paths hinted.

| Setup (base URL the participant writes) | Opened? | Hint on the model path | Decoded? | Notes |
|---|---|---|---|---|
| Vercel, Claude Code `https://ai-gateway.vercel.sh/claude-code` | yes | `anthropic-messages` | yes, measured 15 calls | `docs/research/results/2026-10-vercel-gateway.md:53` |
| Vercel, Codex `https://ai-gateway.vercel.sh/codex/v1` | yes | `openai-responses` | yes, measured 17 calls | `/codex/v1/responses/compact` is recorded with no hint (capture-only). WebSocket mode unmeasured |
| Vercel, Hermes `https://ai-gateway.vercel.sh/v1` | yes | `openai-chat-completions` | yes, measured 9 calls | result `:55` |
| OpenRouter, Claude Code `https://openrouter.ai/api` | yes | `anthropic-messages` (`/api/v1/messages`) | expected; unmeasured | SSE comments are skipped by the framer (`wire.rs:599-615`) |
| OpenRouter, Codex or Hermes `https://openrouter.ai/api/v1` | yes | `openai-responses` / `openai-chat-completions` | expected; unmeasured | double `finish_reason` and `finish_reason: "error"` chunks not in our fixtures (unverified) |
| Cloudflare, Claude Code `.../v1/<acct>/<gw>/anthropic` | yes | `anthropic-messages` | expected; unmeasured | `cf-aig-authorization` masked (name contains `auth`, `headers.rs:23-30`) |
| Cloudflare, Codex via `codex --profile cloudflare-aig` | **no** | none | no | Goal Cup builds Codex's argv and does not read profiles (`docs/HARNESS-SETUP.md:105-106`); Cloudflare's own recipe is a profile. Silent tunnel under `absent = "defaults"` |
| Cloudflare non-realtime WebSocket `wss://.../v1/<acct>/<gw>` | if the host is the model host | none | no (`universal.create` frames) | capture-only |
| LiteLLM `http://0.0.0.0:4000` | refused at start | | | correct: plain http and loopback (`model_host.rs:451-452`, `docs/plans/2026-10-28-one-capture-system.md:51`) |
| LiteLLM on a remote `https://` host | yes | all three hints | expected | |
| Claude Code `CLAUDE_CODE_USE_BEDROCK` (default host or `ANTHROPIC_BEDROCK_BASE_URL`) | **no** | | | tunnels; documented gap (`docs/CHEAT-CHECKLIST.md:136-144`). Even if opened: event-stream on `/model/.../invoke-with-response-stream`, no decoder |
| Claude Code `ANTHROPIC_BASE_URL=https://bedrock-runtime.<r>.amazonaws.com/anthropic` + Bedrock API key | yes | `anthropic-messages` | expected | AWS documents `x-api-key` on this route; whether Claude Code works this way is unverified |
| Claude Code `CLAUDE_CODE_USE_VERTEX` | **no** | | | tunnels. Even if opened: `:streamRawPredict` does not end in `/messages`, so the Anthropic decoder rejects it as `unsupported_endpoint` |
| Claude Code `CLAUDE_CODE_USE_FOUNDRY` | **no** | | | tunnels |
| Claude Code `ANTHROPIC_BASE_URL=https://<res>.services.ai.azure.com/anthropic` + key | yes | `anthropic-messages` | expected | unmeasured |
| Codex `model_provider = "amazon-bedrock"` (no table) | **no** | | | built-in with no `base_url`, so `absent = "defaults"` opens only `chatgpt.com` and `api.openai.com` (`docs/HARNESS-SETUP.md:108-112`); calls go to `bedrock-mantle.<r>.api.aws` and tunnel |
| Codex `model_provider = "gcp"` | **no** | | | same; calls go to `aiplatform.googleapis.com/.../publishers/openai/v1/responses` |
| Codex `model_provider = "ollama"` / `"lmstudio"` | **no** | | | records the OpenAI hosts; local calls go direct; run labelled `native_only` (E7) |
| Codex custom table, Azure `https://<res>.openai.azure.com/openai/v1` | yes | `openai-responses` | expected | `api-key` header masked (`key`) |
| Hermes with Azure `?api-version=` | refused | | | `docs/HARNESS-SETUP.md:207` |

### E8.3 Stale or wrong docs

| Where | Says | Reality |
|---|---|---|
| `docs/ARCHITECTURE.md:213` | "No header is recorded." | Request and response heads are recorded as `http-head-v1` streams with credentials masked since 28 Oct 2026 (`crates/goalcup-cli/src/proxy/headers.rs:1-11`, `http.rs:1322`; commits `e0204ba3`, `0a4d848e`, `f715a089`) |
| `connectors/codex-cli/harness.toml:101-107` | "Each is a JSON field there, not an HTTP header: no header is ever recorded." | `x-codex-turn-state` is also an HTTP response and request header in 0.165.0 (`codex-api/src/sse/responses.rs#L33`, `core/src/client.rs#L291-L304`), and headers are now recorded, unmasked for this name |
| `crates/goalcup-protocol/providers.toml:10` | "Frozen (D74): no new rows" | Fine as a rule, but the table now serves two rows for two connectors. It adds a loader (`crates/goalcup-protocol/src/harness/schema.rs:846-900`) and a `providers` field for no gain |
| `docs/harnesses/codex-cli.md:150` | API key path "Measured with a key: not yet" | Still true; OpenAI docs confirm the same `/v1/responses` WebSocket and SSE forms, so the row is right in shape |
| `docs/ARCHITECTURE.md:211` | "HTTP/1.1 and WebSocket only (no HTTP/2)" | Still true of the proxy. Add one line: every provider accepts HTTP/1.1; an h2-only client is refused today |
| `docs/research/VERCEL-COMPAT.md:18-21` (Verdict 2) | "AI Gateway: only Hermes and Amp are recorded today ... Claude Code and Codex ... opaque tunnel" | Superseded by D74 and measured on 28 Oct (the banner at `:3-6` says so, but the verdict text was not changed) |
| `docs/research/VERCEL-COMPAT.md:73-75` | Response compression "**unverified** against the gateway" | Round 3 decoded every call, so whatever the gateway sent was read; say "measured, no problem" |
| `docs/research/VERCEL-COMPAT.md:146-147`, `:178-180` | "Hermes is the one harness the gateway is recorded for today"; "Today the expected result is no wire record" for Claude Code or Codex | Stale since 28 Oct |
| `docs/research/VERCEL-COMPAT.md:41` | Max session 24 hours on Pro | Still true (pricing page, 10 Sep 2026) |
| `docs/research/VERCEL-COMPAT.md:46` | Brokering terminates TLS | Still true (firewall page, 16 Sep 2026) |
| `docs/research/results/2026-10-vercel-gateway.md:144-146` | "Tunnelled host names are not printed" | Fixed by D74: the `sealed hosts` line (`:63-67` of the same file) |
| `docs/research/results/2026-10-vercel-gateway.md:203-215` | "The Codex gap (not attempted)": key never arrives, wrong host recorded | Solved in Round 3 (`:54`, key in `$CODEX_HOME/.env`) |
| `docs/research/results/2026-10-vercel-gateway.md:279-280` | "Today 4a passes and 4b should not, until D74 lands" | D74 landed; 4b passes |
| `docs/HARNESS-SETUP.md:146-169` | Gateway recipe names only Vercel | Fine, but add one line each for OpenRouter (`https://openrouter.ai/api`), Cloudflare, Bedrock `/anthropic` and Foundry `/anthropic`, and say Bedrock, Vertex and Foundry switches are not recorded |
| `docs/HARNESS-SETUP.md:108-112` | A built-in provider with no table opens the default hosts | True, but should say plainly that `amazon-bedrock`, `gcp`, `ollama` and `lmstudio` are then **not recorded** |
| `docs/RULES.md:134-136` | "full freedom of model ... compute" | Local models are not recorded (E5, E7) |
| `crates/goalcup-protocol/src/capture_policy.rs:1027` | "a local model is not supported by schema 1" | "schema 1" means nothing to a participant |

## E9. Keep, cut, change (provider-facing parts only)

| Part (path, src lines, test lines) | What it does | Points | Verdict | Reason | Lines saved (src / tests) | Risk | Tests that go |
|---|---|---|---|---|---|---|---|
| Connector host rows (`connectors/codex-cli/harness.toml:119-130`, data; `connectors/claude-code-cli/harness.toml:34-36`, data) | Name the default model hosts | 1, 7 | keep | Both sign-ins and both providers land on these hosts | 0 / 0 | A new provider or host in a harness update is a silent tunnel until the list changes | none |
| Shared provider table (`crates/goalcup-protocol/providers.toml`, 25 data; loader `schema.rs:846-900`, about 55 src; tests in `crates/goalcup-protocol/tests/it/harness_schema.rs`, not counted) | Lets a connector name `anthropic` or `deepseek` instead of writing the row | none directly | cut | Two rows, two users (only `anthropic` used). Move each row into its connector | about 55 / small | A connector edit loses the row by mistake; `shipped_connectors.rs` would catch it | the provider-table cases in `harness_schema.rs` |
| `providers.toml` `deepseek` row (5 data lines) | Shared DeepSeek host row | none | cut (with the table) | Unused, path stale | 5 / fixtures that name it (`crates/goalcup-protocol/tests/it/harness_schema.rs`, `harness_loader.rs`, `tests/fixtures/...`), not counted | A schema-1 fixture still naming `deepseek` fails to load; update the fixture in the same change | provider-table cases |
| `model_host.rs` reader and host rule (`crates/goalcup-cli/src/model_host.rs`, 894 src, 646 inline tests; plus `crates/goalcup-cli/tests/it/model_host_from.rs`, 793 tests) | Reads the one configured base URL from harness config, opens its host, hints three wire paths | 1, 7 | keep, small change | It is what makes "any provider, no list" true; gateways are how many participants run. Measured on Vercel. Lane D adds Codex keys. E2 change: refuse Codex built-ins `ollama`/`lmstudio` (and others with no table) instead of falling back (as `parts/D-codex.md:282`), and reword the two refusal strings | 0 / 0 (adds about 20 src, a few tests) | Misses providers the reader cannot see (lane D). Over-refusing: a participant who set `model_provider = "ollama"` but never used it is stopped; acceptable, the message tells them what to edit | none |
| Cloud-switch warning (new, in `goalcup prep` and `start`) | Warn or refuse when the harness config sets `CLAUDE_CODE_USE_BEDROCK`, `_VERTEX`, `_FOUNDRY`, `_MANTLE`, or Codex `model_provider` is `amazon-bedrock`, `amazon-bedrock-runtime` or `gcp` with no `base_url` | 1, 3, 6 | change (add) | Turns a silent, unrecorded run into a clear message before the run. Reuses `read_setting` (`model_host.rs:546-553`). Can share code with the Codex built-in refusal above | about +30 to +50 src est. / small | Over-refusing a working setup; a switch set in a place we do not read (shell, managed settings) is still missed | none |
| Local-host refusal (`crates/goalcup-protocol/src/capture_policy.rs:1016-1044`, about 29 src) | Refuses loopback, single-label, numeric names | 1, 3 | keep | Without it a run would look recorded and record nothing | 0 / 0 | none new | none |
| Header name masking (`crates/goalcup-cli/src/proxy/headers.rs`, 267 src, 136 inline tests) | Masks credential headers | 4 | keep, tiny change if owner wants | Covers every auth header every service here uses | +2 / +a few | Account ids recorded if left as is | none |
| Decoders (`anthropic.rs` 1,585 src + 53 inline tests = 1,638; `openai.rs` 664; `chat.rs` 1,080, no inline tests; `crates/goalcup-verifier/tests/it/m2_chat_decoder.rs`, 2,524 tests) | Turn raw bytes into observations for the three wire formats | 6 | keep, change only how unknowns count | One chat decoder serves DeepSeek, Moonshot, Z.ai, Groq, Together, Fireworks and gateways; every gateway and compatible cloud route uses these formats; providers add event types by policy, so a closed list will always see unknowns | 0 / 0 | None for recording; only for the report. Provider extras and OpenRouter's double `finish_reason` and error chunks may show as unknowns | none |
| Event-stream or `rawPredict` decoders | Would read Bedrock native and Vertex native streams | 6 | do not add | Not needed if participants use base-URL routes; new binary parser is risk before Friday | 0 / 0 | A Bedrock-native participant gets capture-only bytes, if ever opened | none |
| Preflight TCP probe (`crates/goalcup-cli/src/proxy/preflight.rs`, 250 src, 170 inline tests) | Fails a start whose model host cannot be reached | 3 | keep | Cheap. It does not catch a private-CA host; adding a TLS check is not worth it for Friday | 0 / 0 | LAN-with-private-CA blocked at first call, not at start | none |
| No `HTTP_PROXY`, no absolute-form handling (`launch.rs:855-860`, `proxy/connect.rs:701`) | Keeps plain HTTP and local traffic out of the proxy | 3, 4 | keep | Adding plain-HTTP capture is new code, records the agent's own local dev traffic, and still misses Go and dsh loopback (E13) | 0 / 0 | Local models stay unrecordable | none |
| Tunnel list cap (`crates/goalcup-cli/src/proxy/tunnels.rs`, 192 src, 93 inline tests) | Lists sealed tunnels, 10,000 rows max | 1, 5, 6 | change | Aggregate per host:port instead of per connection. Same or less code; no cap of our own; a late unrecorded gateway is still named | about 0 to -20 / a few rewritten | A host-level row loses per-connection times (keep first and last seen) | `tunnels.rs` row-cap tests |
| "Possible unrecorded model traffic" line at finish (proposed by all three parts) | Name tunnelled hosts that look like model streams (long-lived, many small server writes) or match a short suffix list: `openai.com`, `anthropic.com`, `chatgpt.com`, `openrouter.ai`, `deepseek.com`, `moonshot.ai`, `moonshot.cn`, `z.ai`, `bigmodel.cn`, `groq.com`, `together.ai`, `together.xyz`, `fireworks.ai`, `ollama.com`, `ai-gateway.vercel.sh`, `gateway.ai.cloudflare.com`, `amazonaws.com` with `bedrock`, `api.aws`, `aiplatform.googleapis.com`, `azure.com` | 3, 6 | change (optional, add) | Warns only; never opens anything; a few lines over data the tunnel record already has | about +15 src est. / small | It is still a list; a missing suffix is silent | none |
| Gateway recipes in `docs/HARNESS-SETUP.md` | Tell participants which base URL works | 6, 7 | change (docs) | Cheapest way to bring Bedrock, Foundry, OpenRouter and Cloudflare users into the recorded path | 0 / 0 | Bedrock `/anthropic` and Foundry `/anthropic` with Claude Code are unmeasured; mark them so | none |
| `goalcup doctor` Node version check (new) | Print the Node version when the connector sets `NODE_USE_ENV_PROXY`; refuse under 22.21 | 1, 7 | change (optional, add) | Old Node `fetch` sends every call direct | a few src | Only for Node harnesses outside the four shipped | none |
| Wildcard decrypt of cloud hosts (proposed in `docs/plans/2026-10-30-audit/B-fresh-design.md:264-268`, `:668-674`) | Open `bedrock-runtime.*`, `*-aiplatform.googleapis.com`, `*.openai.azure.com` for everyone | 1 | reject | Breaks boto3 (certifi only), gRPC (h2 only, own roots), `cloudflared`; misses `aiplatform.googleapis.com`, `*.rep.googleapis.com`, `bedrock-mantle.*.api.aws`, `*.services.ai.azure.com`; puts the agent's own cloud work (deploys, tests) through decryption | 0 / 0 | n/a | n/a |

## E10. Risks for a run that starts 7 Nov 2026 and lasts 168 hours

| Risk | Likelihood | Effect | What to do |
|---|---|---|---|
| Provider outages. October 2026 had about 10 Anthropic model-error incidents, about 15 OpenAI incidents naming API or Codex, 3 Groq incidents, at least 25 Fireworks degradation incidents | High (several in a week) | Harness retries or stalls, slow turns. Recording sees 429, 500, 529 and SSE `error` events; nothing is lost | Nothing in code. Tell participants the run continues after an outage (Claude Code `CLAUDE_CODE_RETRY_WATCHDOG=1` is set, `connectors/claude-code-cli/harness.toml:61-65`; Codex retries 4 and 5 times, then the participant types keep going) |
| Subscription weekly limits (ChatGPT plans, Claude plans, Z.ai coding plan 5-hour and weekly credits) and spend caps (API keys: a spend-cap 429 with no `retry-after` until the 1st of the month) | High for a full 168-hour run on one plan | The harness stops making calls, possibly days before the end. Not a recording fault | Say it in the participant guide: name 5-hour and weekly caps. No code |
| DeepSeek balance runs out (402) or concurrency 429 over a week | Medium for pay-as-you-go keys | Harness errors or stalls; every error is recorded | Guide: top up before the run |
| Gateway outage or soft budget hit. Vercel budgets are soft caps; OpenRouter in-flight budget gives 402; Azure "temporary rate limit adjustment" can last hours | Medium | Harness retries or stalls; recording sees each 429, 402, 503 | Nothing in code. Tell participants to keep credit headroom for a week |
| Free-tier gateway limits | High if a participant uses free credits | Many 429s (seen in all Vercel runs, `2026-10-vercel-gateway.md:48`, `:119-120`) | Guide: paid tier for a 168-hour run |
| DeepSeek 10-minute pre-inference waits under load | Medium at peak | The harness's own timeout may fire first; the proxy will not cut it | None in code. Z.ai and others tell Claude Code users to raise `API_TIMEOUT_MS` in `settings.json`, which works under our launch |
| WebSocket connection limits: OpenAI 60 minutes, Vercel about 13 minutes, LiteLLM 60 minutes | Certain for Codex on WebSocket; certain on Vercel or LiteLLM if WebSocket mode is on | About 170 or more reconnects per long session at 60 minutes; about 775 per week on Vercel. Each is a new CONNECT and upgrade | None: each message is its own stream and each connection closes, so open-stream limits are not hit. Check the 2-hour run (not yet written) shows a reconnect recorded cleanly |
| WebSocket lanes (`stream_id`) adopted by a Codex update mid-run | Low (not in 0.165.0 source) | Up to 16 parallel responses on one socket; decoder loses lane attribution only | Pin the harness version for the run; Codex has `check_for_update_on_startup=false` in our args |
| Harness auto-update during the run changes hosts or events | Medium (Claude Code ships almost daily, Codex weekly; lane D) | New hosts tunnel silently; new events become unknowns | `DISABLE_AUTOUPDATER=1` is set for Claude Code (`connectors/claude-code-cli/harness.toml:60`); Codex update check is off. Keep both |
| Gateway changes paths or adds events during the week | Low | Unknown events are counted, not fatal | Never gate a run on decoder completeness |
| Groq or another provider returns usage only in a vendor field | Certain for some | Usage blank in the report; recording intact | Never gate on decoder stats |
| Participant switches model service mid-run (rate limit, outage, budget) by editing the harness config | Medium to high over a week | New host tunnels, unrecorded, until `goalcup continue`; the CA cannot cover it (`tls.rs:131-137`) | Guide line; the "possible model traffic" warning; server flags sealed gateway hosts |
| Cloud switch or Codex built-in (`amazon-bedrock`, `gcp`) or a Codex profile (Cloudflare's own recipe) | Low to medium (company accounts) | Whole run's model calls tunnel; label `native_only` | Prep warning plus doc recipe (E9) |
| A participant uses a local model (Ollama, LM Studio) because the rules allow "full freedom of ... compute" | Low to medium | Hermes and Claude Code: refused at prep (good, but a surprise late). Codex with `--oss`/`model_provider = "ollama"`: **not refused**, nothing recorded, run labelled `native_only` | Change the rules sentence now; refuse Codex built-ins (about 20 lines) |
| Ollama cloud models through the local daemon | Low | Same as above, silent | Covered by the loopback refusal for Hermes and Claude Code; the Codex fix covers Codex. Say it in the guide |
| A Node or Python harness on an old runtime, or using `ws` for WebSockets | Medium for harnesses outside the four shipped | Every model call goes direct, unrecorded | `goalcup doctor` could print the Node version when the connector sets `NODE_USE_ENV_PROXY`; refuse under 22.21. A few lines |
| DeepSeek's own harness (540k weekly downloads) is asked for | Medium | No connector; cannot run under Goal Cup | Point 7 decision for the owner; first measurement would follow the Claude Code pattern (`NODE_EXTRA_CA_CERTS` plus proxy env), but `$DSH_HOME/.env` can override our proxy |
| `chatgpt-account-id` and `x-codex-turn-state` recorded in header streams | Certain on ChatGPT sign-in | An account identifier reaches the server | Owner decides; two words in `headers.rs` if yes |
| A Python harness sets `SSL_CERT_FILE` to our CA only | Low (the repo hands shared variables a system-plus-proxy bundle, `crates/goalcup-cli/src/ca_bundle.rs:1-19`) | `httpx2` would trust only our CA and fail on tunnelled hosts | Already handled. Keep `ca_bundle.rs` |
| A LAN model box with a private CA, or a corporate TLS-inspecting network | Low | Every model call fails at our upstream check; harness blocked (point 3); start passes because the probe is TCP only | Say in the refusal and the guide: a public certificate is required. Corporate case: `parts/F-os.md` (in `F-week-long-risks.md`) |
| Vercel OIDC token used as the gateway key | Low | Expires after 12 hours; calls fail with 401 | Guide: use an API key, not OIDC |
| Vercel Sandbox as the run machine | Certain if used | 24-hour session maximum: at least seven forced restarts; a stop snapshots disk including key files (`2026-10-vercel-gateway.md:108-110`) | Owner decision; the 2-hour run (report not written yet) will not test the 24-hour boundary |
| Sandbox credential brokering on the model host | Low | Our upstream TLS check refuses the firewall's certificate; every model call fails | Keep brokering off for the gateway host (`VERCEL-COMPAT.md:46`) |
| Tunnel list cap | High over a week | Late sealed hosts not named | Aggregate per host (E9) |
| Unmeasured gateways (OpenRouter, Cloudflare, Azure, Bedrock `/anthropic`) | Medium | A small decoder mismatch shows as `wire_undecoded` (as Claude Code on Vercel did before fix #286, `2026-10-vercel-gateway.md:57-60`) | One 10-minute practice run per route before 7 Nov, if any participant plans to use it |
| Provider or doc host moves (Moonshot docs moved to `platform.kimi.ai`; Together docs now name `api.together.ai` while `api.together.xyz` still answers) | Medium | None while the config names the host the harness actually calls | None |
| Ollama ships daily (five releases 22 to 28 Oct) | High | Only matters for local use, which is refused | None |
| HTTP/3 or WebRTC paths | Low | Unrecorded media or calls | None for Friday |

## E11. Blind spots

- No harness was measured through our proxy in this lane. The only measured routes are Vercel (Claude
  Code, Codex, Hermes, `docs/research/results/2026-10-vercel-gateway.md`) and DeepSeek through Hermes
  (`docs/harnesses/hermes-cli.md:75`). Every "expected" in E8.2 is reasoning from paths and formats.
  HTTP versions, header names and body codings come from source and docs, not from a capture.
- Keyless probes show routing and answers, not streaming shapes. They went through the sandbox egress
  proxy; whether their h2 and h3 results reflect the vendor is disputed between the parts (see "Where
  the parts disagree"). HTTP/1.1 claims rest on docs, SDK defaults (general knowledge) and measured runs.
- The `ws` and `websockets` bypass claims come from reading source and docs. A one-minute local test (a
  Node script with `ws` and `NODE_USE_ENV_PROXY=1` against a logging proxy) would settle it. Unverified.
- The Anthropic TypeScript SDK's proxy section was not read (the npm README was a stub). Unverified
  beyond "uses global fetch".
- Whether `fireworks-ai` sets `trust_env`, and whether Codex's reqwest skips loopback, were not read in
  source.
- Key prefixes for Groq, Fireworks, Together and Z.ai are from common use, not from their docs.
- Bedrock event-stream content type, Vertex `alt=sse`, Azure realtime path and Vertex API-key header
  are general knowledge, not read on a page today.
- Whether Claude Code works with Bedrock's `/anthropic` route or Foundry's `/anthropic` route via
  `ANTHROPIC_BASE_URL` is unverified. AWS and Microsoft document the routes; Anthropic's harness docs
  (lane D) document the switches instead.
- Status feeds: Anthropic RSS holds only the last 25 items; Vercel and Cloudflare read once (last 25
  items each); titles were classed by hand. DeepSeek's and Together's status pages could not be read as
  feeds; Moonshot's and Z.ai's were blocked by this sandbox's egress; OpenRouter, AWS, Google and Azure
  not read.
- gRPC support for userinfo in `https_proxy` (our proxy requires `Proxy-Authorization`,
  `crates/goalcup-cli/src/proxy/connect.rs:30-35`) is unverified. It matters only for a gRPC tool whose
  host is tunnelled.
- The llama.cpp default port (9931) contradicts long-standing usage (8080); the current README is
  quoted and when it changed was not checked.
- dsh facts come from its published READMEs, not from running it.
- The tunnel-or-tailnet workaround for local models (E13) is untested.
- Whether ChatGPT account ids count as "secrets" under point 4 is a policy call, not a fact.
- Line estimates for new code (option B, cloud-switch warning, suffix warning) are guesses; nothing was
  written.

## E12. Sources (fetched 29 Oct 2026 unless a date is given)

Anthropic:
- https://platform.claude.com/docs/en/api/overview
- https://platform.claude.com/docs/en/api/versioning
- https://platform.claude.com/docs/en/build-with-claude/streaming
- https://platform.claude.com/docs/en/api/errors
- https://platform.claude.com/docs/en/api/rate-limits
- https://status.claude.com/history.rss
- https://code.claude.com/docs/en/network-config

OpenAI and Codex:
- https://developers.openai.com/api/docs/guides/websocket-mode, https://developers.openai.com/api/docs/guides/websocket-mode.md
- https://developers.openai.com/api/docs/guides/rate-limits
- https://developers.openai.com/api/reference/overview
- https://status.openai.com/history
- https://github.com/openai/codex/tree/rust-v0.165.0/codex-rs (27 Oct 2026), including
  https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/model-provider-info/src/lib.rs (and `#L690-L823`),
  https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/model-provider/src/amazon_bedrock/mantle.rs,
  https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/model-provider/src/gcp/mod.rs

DeepSeek and other providers:
- https://api-docs.deepseek.com/
- https://api-docs.deepseek.com/quick_start/rate_limit
- https://api-docs.deepseek.com/quick_start/error_codes
- https://api-docs.deepseek.com/guides/anthropic_api
- https://status.deepseek.com/, https://status.deepseek.com/history
- https://registry.npmjs.org/@deepseek-ai%2Fdsh (0.2.1-rc.2, 17 Oct 2026); tarballs of `@deepseek-ai/dsh-http-proxy`, `dsh-launch-environment`, `dsh-agent-preset` 0.2.1-rc.2
- https://api.npmjs.org/downloads/point/2026-10-21:2026-10-27/@deepseek-ai/dsh
- https://platform.kimi.ai/docs/guide/agent-support, https://platform.kimi.ai/docs/guide/claude-code-kimi.md, https://platform.kimi.ai/docs/guide/codex-kimi.md, https://platform.kimi.ai/docs/pricing/limits
- https://docs.z.ai/guides/overview/quick-start, https://docs.z.ai/devpack/quick-start.md, https://docs.z.ai/devpack/tool/claude, https://docs.z.ai/devpack/overview
- https://console.groq.com/docs/openai, https://console.groq.com/docs/rate-limits, https://groqstatus.com/api/v2/incidents.json
- https://apis.io/asyncapis/groq/groq-asyncapi/, https://hexdocs.pm/req_llm/groq.html (third party)
- https://docs.together.ai/docs/openai-api-compatibility, https://status.together.ai/
- https://docs.fireworks.ai/tools-sdks/anthropic-compatibility, https://status.fireworks.ai/api/v2/incidents.json
- Keyless probes of `api.anthropic.com`, `api.openai.com`, `chatgpt.com`, `api.deepseek.com`, `api.moonshot.ai`, `api.moonshot.cn`, `api.z.ai`, `open.bigmodel.cn`, `api.groq.com`, `api.together.xyz`, `api.together.ai`, `api.fireworks.ai`, `ollama.com` and the gateway and cloud hosts (29 Oct 2026)

Local servers:
- https://github.com/ollama/ollama/releases (v0.40.13, 28 Oct 2026), https://docs.ollama.com/api/openai-compatibility, https://docs.ollama.com/api/anthropic-compatibility, https://docs.ollama.com/faq
- https://lmstudio.ai/docs/developer/anthropic-compat
- https://raw.githubusercontent.com/ggml-org/llama.cpp/master/tools/server/README.md
- https://docs.vllm.ai/en/latest/serving/online_serving/, https://docs.vllm.ai/en/latest/cli/serve/

Vercel:
- https://vercel.com/docs/ai-gateway, https://vercel.com/docs/ai-gateway/coding-agents,
  https://vercel.com/docs/ai-gateway/coding-agents/claude-code,
  https://vercel.com/docs/ai-gateway/coding-agents/openai-codex,
  https://vercel.com/docs/ai-gateway/rate-limits,
  https://vercel.com/docs/ai-gateway/sdks-and-apis/anthropic-messages-api,
  https://vercel.com/docs/ai-gateway/sdks-and-apis/responses,
  https://vercel.com/docs/ai-gateway/sdks-and-apis/responses/websockets,
  https://vercel.com/docs/ai-gateway/authentication-and-byok/oidc,
  https://vercel.com/docs/sandbox/concepts/firewall, https://vercel.com/docs/sandbox/pricing,
  https://www.vercel-status.com/history.atom
- https://vercel.com/changelog/websocket-support-for-openai-responses-api-live-on-ai-gateway (27 Jul 2026)

OpenRouter:
- https://openrouter.ai/docs/api-reference/overview, https://openrouter.ai/docs/llms.txt,
  https://openrouter.ai/docs/api_reference/streaming.md,
  https://openrouter.ai/docs/api_reference/limits.md,
  https://openrouter.ai/docs/api_reference/errors-and-debugging.md,
  https://openrouter.ai/docs/cookbook/coding-agents/claude-code-integration.md,
  https://openrouter.ai/docs/guides/ori/harness.md, https://openrouter.ai/docs/guides/ori/codex.md,
  https://openrouter.ai/docs/guides/ori/changelog.md

Cloudflare:
- https://developers.cloudflare.com/ai-gateway/llms.txt and the pages under
  https://developers.cloudflare.com/ai-gateway/ named in E6.3 (usage/providers/anthropic,
  usage/providers/openai, usage/chat-completion, usage/rest-api, usage/websockets-api and its two
  children, configuration/authentication, features/rate-limiting, glossary,
  reference/troubleshooting, integrations/coding-agents/claude-code,
  integrations/coding-agents/openai-codex, changelog), https://www.cloudflarestatus.com/history.atom

LiteLLM:
- https://docs.litellm.ai/docs/tutorials/claude_responses_api,
  https://docs.litellm.ai/docs/proxy/virtual_keys, https://docs.litellm.ai/docs/response_api,
  https://docs.litellm.ai/docs/guides/security_settings.md (last_updated 28 Oct 2026)

AWS:
- https://docs.aws.amazon.com/bedrock/latest/userguide/endpoints.html,
  https://docs.aws.amazon.com/bedrock/latest/userguide/inference-messages-api.html,
  https://docs.aws.amazon.com/bedrock/latest/userguide/api-keys-use.html,
  https://docs.aws.amazon.com/bedrock/latest/APIReference/API_runtime_InvokeModelWithResponseStream.html,
  https://docs.aws.amazon.com/nova/latest/userguide/speech-bidirection.html,
  https://docs.aws.amazon.com/sdk-for-javascript/v3/developer-guide/node-configuring-proxies.html,
  https://github.com/boto/botocore/blob/develop/botocore/httpsession.py,
  https://unpkg.com/@anthropic-ai/bedrock-sdk@1.1.0/client.js

Google:
- https://docs.cloud.google.com/vertex-ai/generative-ai/docs/partner-models/claude/use-claude,
  https://docs.cloud.google.com/vertex-ai/generative-ai/docs/reference/rest/v1/projects.locations.endpoints/streamGenerateContent,
  https://docs.cloud.google.com/vertex-ai/generative-ai/docs/migrate/openai/overview,
  https://docs.cloud.google.com/vertex-ai/generative-ai/docs/live-api/get-started-websocket,
  https://github.com/googleapis/python-aiplatform/blob/main/google/cloud/aiplatform/initializer.py,
  https://github.com/grpc/grpc/blob/master/doc/environment_variables.md,
  https://unpkg.com/@anthropic-ai/vertex-sdk@1.1.0/client.js

Microsoft:
- https://learn.microsoft.com/en-us/azure/foundry/openai/api-version-lifecycle,
  https://learn.microsoft.com/en-us/azure/foundry/foundry-models/how-to/use-foundry-models-claude,
  https://learn.microsoft.com/en-us/azure/foundry-classic/openai/how-to/quota,
  https://unpkg.com/@anthropic-ai/foundry-sdk@1.1.0/client.js

Packages, runtimes and libraries:
- npm: https://registry.npmjs.org/openai (7.45.1, 27 Oct 2026), https://registry.npmjs.org/@anthropic-ai%2Fsdk (1.3.0, 23 Oct 2026), https://registry.npmjs.org/groq-sdk (1.6.0, 26 Aug 2026), https://registry.npmjs.org/@lmstudio%2Fsdk (2.1.0, 14 Oct 2026), https://registry.npmjs.org/ollama (0.6.4, 29 Sep 2026), https://registry.npmjs.org/ai, https://registry.npmjs.org/@ai-sdk%2Fgateway, and `@aws-sdk/client-bedrock-runtime` 3.1159.0 (28 Oct 2026), `@anthropic-ai/bedrock-sdk`, `@anthropic-ai/vertex-sdk`, `@anthropic-ai/foundry-sdk`, `@google/genai`, `@openrouter/sdk`, `@azure/identity` at https://registry.npmjs.org/
- PyPI: https://pypi.org/pypi/openai/json (3.41.0, 28 Oct 2026), https://pypi.org/pypi/anthropic/json (2.3.0, 23 Oct 2026), https://pypi.org/pypi/httpx2/json (2.16.0, 21 Oct 2026), https://pypi.org/pypi/groq/json (1.7.0, 26 Aug 2026), https://pypi.org/pypi/together/json (3.0.0, 22 Oct 2026), https://pypi.org/pypi/zai-sdk/json (0.2.3, 16 Jun 2026), https://pypi.org/pypi/fireworks-ai/json (1.2.30, 21 Oct 2026), https://pypi.org/pypi/litellm/json (1.106.6, 28 Oct 2026), https://pypi.org/pypi/ollama/json (0.6.3, 29 Sep 2026), https://pypi.org/pypi/lmstudio/json (1.5.0, 22 Aug 2025), https://pypi.org/pypi/vllm/json (0.32.0, 18 Oct 2026), and `boto3`, `google-cloud-aiplatform`, `google-genai`, `azure-identity`, `openrouter`, `grpcio` 1.85.0 (26 Oct 2026) at https://pypi.org/pypi/
- https://github.com/pydantic/httpx2/blob/main/src/httpx2/CHANGELOG.md
- https://unpkg.com/openai@7.45.1/README.md, https://unpkg.com/openai@7.45.1/realtime/websocket.js
- https://unpkg.com/ws@8.22.0/lib/websocket.js
- https://websockets.readthedocs.io/en/stable/project/changelog.html
- https://nodejs.org/api/cli.json, https://nodejs.org/api/http.json, https://nodejs.org/dist/index.json
- https://pkg.go.dev/net/http#ProxyFromEnvironment (go1.27.3)
- https://github-redirect.dependabot.com/aio-libs/aiohttp/discussions/5855

## E13. What a host-list-free proxy would need

### What "every call" needs, in order

1. The call reaches the proxy. That needs `HTTPS_PROXY` honoured, and every path in E7 closed.
2. The proxy can read it. That needs TLS interception, so the client must trust our CA **for that host
   name**.
3. Interception never breaks a client that does not trust our CA (point 3).
4. Sign-in traffic is never opened, or is fully redacted (point 4).

A host list is how we meet 2 to 4 today: intercept only names the harness trusts us for, tunnel the
rest (`crates/goalcup-cli/src/proxy/connect.rs:10-17`, `:528-579`), and keep a deny list of sign-in
hosts.

### Options for remote HTTPS services

| Option | How | Works for | Breaks | Cost | Verdict |
|---|---|---|---|---|---|
| A. Intercept every SNI | Drop the name constraint, mint a leaf for any SNI | The harness process, which trusts our CA through its own variable | Every tool child that uses its own trust store for its own hosts: git, curl, pip, cargo, Go and Java programs, Docker, boto3 (`certifi` only), gRPC (own roots, h2 only, which our ALPN check refuses), `cloudflared`. `SSL_CERT_FILE` bundles help some, not all (Go on macOS, Java, Bun's bundled store). We cannot hand `AWS_CA_BUNDLE` or `GRPC_*` to anything, because the deny list forbids cloud names in `capture.env` (`capture_policy.rs:273-283`), and each new variable is another list. Cloud hosts carry the agent's own work (tests calling Bedrock, Vertex or Azure; deploys through `api.cloudflare.com`), so more unrelated traffic and credentials pass the recorder (point 4). Also widens what a leaked key could sign | Small code, large risk | No |
| B. Intercept by peer process | For each CONNECT, find the client socket's process (Linux `/proc/net/tcp` or sock_diag, macOS libproc), intercept only if it is the harness binary or its native child; tunnel everyone else | Every host the harness itself calls, including unknown gateways, providers and clouds; tool children untouched | Harness helper processes (Claude Code's background supervisor, Codex app-server daemon) must be recognised too. Containers and sandboxes may hide `/proc` details. Also needs a CA without name constraints, or one minted per new host (`tls.rs` cannot do that today), and a decoder fallback that stores unknown formats (Bedrock event-stream, Vertex `rawPredict`) as capture-only | About 200 to 400 src lines plus tests (estimate), two OSes, new failure modes | Not for Friday. Revisit for Edition 02 |
| C. Learn on failure | Intercept, and if the client rejects our certificate, tunnel that host next time | Eventually most hosts | The first connection to each host fails, and some tools do not retry (point 3) | Medium | No |
| D. Keep a small list, record every gap | Connector defaults plus the config-named host (as today, D74), and chain every unlisted tunnel as a record with SNI, port, bytes and time (lane C) | All known paths. Unknown paths become visible gaps the server and participant see (points 3, 5, 6) | Calls to an unlisted model host are not readable | Lane C already proposes the chained tunnel record | Yes, for Friday |

### Local and LAN servers (plain HTTP)

Plain HTTP needs no CA, so in principle the proxy could record every plain-HTTP request with no list at
all. It would need:

1. `HTTP_PROXY`/`http_proxy` set to our proxy, and `localhost` removed from `NO_PROXY` (keeping the
   proxy's own address reachable).
2. The proxy to accept absolute-form `GET http://host/...` requests (today: 405), relay them, and
   capture them with the same HTTP/1.1 relay.

It would still miss: Go programs (never proxy `localhost`), DeepSeek's harness (always bypasses
loopback), Ollama cloud models (the daemon's own leg), UNIX sockets, and Codex's built-in local
providers if reqwest bypasses loopback (unverified). It would also pull the agent's own local traffic
(dev servers, test servers, databases over HTTP) through the proxy, which can slow it (point 3) and
records data the agent's app handles, including its test credentials (point 4). Ollama's own FAQ warns
that `HTTP_PROXY` "can interrupt client connections to the server" (https://docs.ollama.com/faq).
**Not for Friday.**

A no-code path that already works in theory: serve the local model over HTTPS under a public DNS name
with a public certificate, for example a tunnel or a tailnet name, and put that `https://` address in
the harness config. The name resolves to a non-loopback address, so the reader accepts it and the proxy
intercepts it with the Mozilla roots. Not measured; Ollama does not serve TLS itself, so a front such as
`tailscale serve` is needed (unverified that its names resolve to a non-loopback address on the same
machine). Offer it in the guide only after one test.

### Summary: the full list-free proxy, and the Friday version

A proxy that records every call with no host list would need all of these:

| Need | Why | Today |
|---|---|---|
| Every client reaches the proxy | Old Node `fetch`, `ws`, Python `websockets` 13/14, aiohttp without `trust_env`, AWS JS v3, `.env` overrides, Ori launcher, WebRTC and loopback all skip it (E7) | Partly: `HTTPS_PROXY` plus connector env only |
| Peer-process interception on Linux and macOS, with helper-process recognition | The only way to decrypt unknown hosts without breaking tools that do not trust our CA | None |
| A CA without name constraints, or per-host minting at run time | Today's CA is fixed to hosts known at start (`tls.rs:131-137`) | Name-constrained, 8 days |
| A kept deny list of sign-in hosts | Point 4: `auth.openai.com`, `platform.claude.com` must stay opaque. This is still a list | Yes (`auth_hosts`) |
| HTTP/2 relay, or an h2-only tunnel fallback | gRPC clients offer only h2 | HTTP/1.1 only; lane C proposes the fallback |
| Capture-only storage for unknown formats | Bedrock event-stream, Vertex `rawPredict`, Cloudflare `universal.create` | Per-host capture already stores bytes without a hint |
| Plain-HTTP capture (`HTTP_PROXY`, absolute form) for local servers | Local models are plain HTTP on loopback | Refused by design |

For Friday 6 Nov, do option D plus four small things: (1) keep the connector defaults plus the one
config-named address; (2) tell the participant before the run when the config uses a route we cannot see
(cloud switches, Codex built-ins with no `base_url` including `ollama` and `lmstudio`, a Codex profile),
with the base-URL alternative; (3) name every sealed host at finish and in the upload, aggregated per
host with no row cap, and flag model-provider, gateway and cloud suffixes as "possible unrecorded model
traffic" (warn only); (4) say in the guide that local models are not recorded and that changing the model
service mid-run takes effect at the next `goalcup continue`.
