# Appendix D: harness research (Lane D)

Date: 29 Oct 2026. Repo: `/home/user/goal-cup-proxy` at 4b226de2 (same commit as origin/main). Read-only:
no harness was run, nothing was built, no test was run, no key or secret was read. This appendix merges
four lane parts: Claude Code (D1), Codex CLI (D2), Hermes Agent (D3) and the other harnesses (D4).
Section numbers inside a part refer to that part: "section 7" in D2 means D2.7.

## Lane summary

Goal Cup promises four harnesses: Claude Code, Codex CLI, Hermes and Amp (site FAQ,
`/home/user/agentcommunity_page/features/goal-cup-campaign/content.ts:292`). All four ship fast: Claude
Code made 26 releases in the 30 days to 27 Oct, Codex 11 stable tags, Hermes 8 stable releases between
8 and 23 Oct plus a daily canary, and Amp 218 npm builds. Most of our facts and connectors were measured
on older builds (Claude Code 2.1.283, Codex 0.153.0 and 0.156.1, Hermes 0.19.0, an Amp build from
mid-October). The capture recipes are mostly still right about where files live and which host to open,
but three things break the seven points on a 168-hour run. First, scale: every harness can start far
more subagents than our native capture holds (32 open streams, 1,024 files, a 64 MiB `state.db` copy).
Second, silent model routes: each harness has at least one way to send model calls to a host we do not
open (a provider switch, a base URL key our reader ignores, or, on Hermes 0.21.6 and later, any host
other than `model.base_url`), and those calls pass as opaque tunnels with no record. Third, things that
run outside our process tree or override our environment: a Claude Code background supervisor, a Codex
shared app-server daemon or `$CODEX_HOME/.env`, a Hermes gateway, and similar services in OpenCode 2,
OpenClaw, Amp runners and Cline. Several connector lines are dead weight and can go
(`NODE_USE_ENV_PROXY` for Claude Code and Codex, seven Codex redaction rows, `HERMES_CA_BUNDLE` and two
Hermes `configure` commands). Of the other harnesses, none should be added before Friday: Cursor cannot be
recorded by an HTTP/1.1 proxy, OpenCode 2 and OpenClaw have unmeasured bypasses, and Copilot CLI is the
best candidate after Edition 01. The 2-hour run report (`docs/research/results/2026-10-30-two-hour-run.md`)
does not exist on any of the remote branches yet, so nothing in this appendix depends on it.

## Top findings

1. **Scale breaks point 2 first.** Native capture holds 32 open streams and 1,024 files for the whole
   run, with a hard ceiling of 20,000 (`crates/goalcup-daemon/src/native_capture.rs:187`, `:272-283`;
   `crates/goalcup-protocol/src/capture.rs:58`). The Hermes `state.db` copy at seal is capped at 64 MiB
   (`NATIVE_FINISH_BYTES = 64 << 20`, `crates/goalcup-daemon/src/runtime/capture/owner.rs:13`). Claude
   Code runs 20 subagents at once per session, up to 1,000 agents per workflow, with no total cap. Codex
   runs 6 at once with no total cap, and since 0.164.0 each forked child copies the parent history.
   Hermes runs 10 at once. Remove the caps (owner decision already made), and do not hold one file
   descriptor per append-only file: the macOS default is 256 open files per process. (D1.8.4, D2.8,
   D3.9)
2. **Every shipped harness has a silent unrecorded model route.** Claude Code: Bedrock, Vertex, Foundry,
   Mantle, the Claude apps gateway, or a base URL in project or local settings (D1.3.3). Codex:
   `openai_base_url`, an `[model_providers.openai] base_url` override without `model_provider`, and the
   built-ins `amazon-bedrock`, `gcp` (new in 0.165.0), `ollama`, `lmstudio` (D2.2). Hermes 0.21.6 and
   later: any host other than `model.base_url`, because Hermes now trusts the OS store and the call
   succeeds through the opaque tunnel (D3.3.3). Amp: bring-your-own-key and subscription routing,
   unmeasured (D4.3.3). The cheapest answer is a doc line plus, at most, a start-time warning; the
   verifier already labels a run with native activity and no wire calls
   (`docs/research/results/2026-10-vercel-gateway.md:127`).
3. **Config files can override our launch environment.** Codex loads `$CODEX_HOME/.env` and overwrites
   every key that does not start with `CODEX_`, so a `HTTPS_PROXY`, `NO_PROXY` or `SSL_CERT_FILE` line
   there skips the recorder (D2.2). Claude Code reads proxy and CA variables from any settings `env`
   block, re-applied live when the file is saved; whether they override our launch env is unverified
   (D1.3.4, D1.10 risk 5). OpenClaw's `OPENCLAW_PROXY_URL` and `proxy.proxyUrl` win over `HTTPS_PROXY`
   (measured, D4.3.2).
4. **Background services run outside our process tree.** Claude Code's per-user supervisor for agent
   view outlives the shell and inherits the first shell's environment (D1.4). Codex's TUI reuses a shared
   app-server daemon if one listens under `CODEX_HOME`; our `-c` args avoid it today by accident (D2.2).
   Hermes model calls all run in a gateway process, which our `gateway stop` handles (D3.3.1). OpenCode 2,
   OpenClaw's TUI mode, Amp runners and the Cline hub daemon have the same shape (D4). Proposed: add
   `CLAUDE_CODE_DISABLE_AGENT_VIEW = "1"` (owner decision) and Codex `--no-daemon`.
5. **Version drift is certain.** About 6 Claude Code releases before 7 Nov and 6 more during the run;
   Codex 0.166.0 expected around 3 Nov; Hermes v0.23.0 announced but not dated; Amp about 50 builds
   during the run. The Codex ChatGPT WebSocket path was last measured on 0.156.1 and its dialer code has
   changed since (9 files, 602 insertions, 222 deletions). Participants get Hermes 0.22.5 from the
   installer, but our capture runs used 0.19.0. Re-run the kit on the newest builds on 5 or 6 Nov, and
   tell participants how to pin a version.
6. **Hermes `HERMES_CA_BUNDLE` stopped steering model calls in 0.21.6 (8 Oct 2026), on every OS.** It is
   now read only by the Nous Portal sign-in and token refresh, where it replaces trust, so our setting can
   only break a refresh. The `recorder` provider entry with `ssl_ca_cert` is what makes recording work.
   Cut `HERMES_CA_BUNDLE`, both `configure` commands and the `session_*.json` row unless the owner keeps
   0.19.0 support. Our advice to set `agent.max_turns: 2000` now adds a cap (unlimited since 0.21.0).
7. **Dead connector lines.** `NODE_USE_ENV_PROXY` does nothing for Claude Code (a Bun binary) or Codex (a
   Node wrapper with no network code). Cut it in both connectors and keep the schema support for Node
   harnesses such as OpenClaw and Cursor. The seven Codex `extra_patterns` rows are not secrets: cut all
   of them, and keep the `json_field` mechanism for Amp's `wsToken`.
8. **Facts sheets are stale.** Claude Code's sheet is for 2.1.283 with at least eight wrong or stale
   lines; Codex's mixes 0.153, 0.156.1 and 0.165 and could shrink to about 80 lines; Hermes docs carry
   the wrong `max_turns`, `tirith`, schema 22, 60 s timeout, "macOS only" and "up to 0.21.0" lines; the
   Amp sheet names Bun 1.4.1 (the binary says 1.4.4) and misses bring-your-own-key and runners.
9. **Subagents on the shipped Hermes version are unmeasured.** The repo says `delegate_task` failed twice
   on 0.19.0 on 29 Oct, but the same 0.19.0 ran two children on a Mac on 22 Oct, and the error text was
   not kept. Measure on 0.22.5 with "ask for subagents in the prompt" before 6 Nov.
10. **Close the "bring another harness" door now.** The FAQ still allows a new harness after a 1-hour
    proof run before 6 Nov. Cursor CLI cannot be recorded (HTTP/2 the server can force); OpenCode 2 and
    OpenClaw have unmeasured bypasses; Copilot CLI is the best next connector after Edition 01. Cut the
    dead `harnesses/install.sh` (62 lines) and the RULES lines for unsupported harnesses.
11. **Sign-in and usage limits over 168 hours.** Claude Code and Codex plans have 5-hour and weekly
    limits that can stall a run for days. Claude Code's macOS Keychain entry is keyed per config dir
    (vendor docs), which contradicts our "shared" note. Codex token refresh breaks a copied `auth.json`
    (`refresh_token_reused`). Our own `HERMES_CA_BUNDLE` breaks the Nous Portal refresh.

## Summary table

Versions are the newest release on 29 Oct 2026 unless noted. "Proxy honoured" means whether model calls
follow `HTTPS_PROXY` so that the recorder can see them. Detail and sources are in D1 to D4.

| Harness | Version as of 29 Oct | Model path | Proxy honoured | Subagent file location | Auto-update default | Top risk for 7 Nov |
|---|---|---|---|---|---|---|
| Claude Code (shipped) | 2.1.312 (27 Oct; stable channel 2.1.308) | `POST /v1/messages` to `api.anthropic.com` or `env.ANTHROPIC_BASE_URL`, HTTP/1.1, SSE; also Bedrock, Vertex, Foundry, Mantle, apps gateway | Yes (`HTTPS_PROXY`, `NO_PROXY`; regressed before: #14165, #39862) | `~/.claude/projects/{project}/{sessionId}/subagents/agent-{agentId}.jsonl`; workflows `<session>/subagents/workflows/<id>/agent-*.jsonl` | On for native installs (check at start and while running; applies at next start); off for Homebrew, WinGet, apt, dnf, apk. Connector sets `DISABLE_AUTOUPDATER=1` | Subagent and workflow fan-out passes 32 streams or 1,024 files |
| Codex CLI (shipped) | 0.165.0 (27 Oct); 0.166.0-alpha.6 (28 Oct) | `/responses` on `chatgpt.com/backend-api/codex` (ChatGPT sign-in) or `api.openai.com/v1` (API key); WebSocket first, HTTPS + SSE fallback, zstd bodies | Yes, HTTPS and WebSocket; partly, because `$CODEX_HOME/.env` can override it | Own rollout per child: `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<local time>-<thread id>.jsonl` | Startup check with a prompt; updates only if the user picks it. Connector sets `check_for_update_on_startup=false`. Homebrew and npm update outside Codex | Unseen model hosts (`openai_base_url`, `gcp`, `amazon-bedrock`) and `.env` override; ChatGPT WebSocket last measured on 0.156.1 |
| Hermes Agent (shipped) | v0.22.5 (22 Oct) from the installer; PyPI stops at 0.19.0 (20 Jul) | Gateway process calls `model.base_url` (one `recorder` provider), HTTP/1.1; about 40 provider plugins | Partly: proxy yes, but the proxy CA is trusted only for the `recorder` host; other hosts tunnel unrecorded since 0.21.6 | Rows in `<HERMES_HOME>/state.db` (`parent_session_id`, `async_delegations`); live logs `<HERMES_HOME>/cache/delegation/live/<id>/task-<n>.log`, pruned after 7 days | No self-update; passive daily check (`updates.check`); plugin check every 24 h, apply off | Silent unrecorded calls to other hosts; `delegate_task` unmeasured on 0.22.5; `state.db` over 64 MiB |
| Amp (shipped) | `0.0.1793271832-g7615fe` (npm, 29 Oct); `cli-version.txt` already `0.0.1793274046-g0b6b2b` | One WebSocket of JSON-RPC to `ampcode.com/actors/gateway/threadActor/`; inference on Amp's server | Yes (measured 15 to 17 Oct) | No local file; subagent snapshots on the same WebSocket with `parent_tool_use_id` | Update check on; connector sets `AMP_SKIP_UPDATE_CHECK=1`; runners restart themselves for updates | Bring-your-own-key routing may call providers locally, unrecorded |
| OpenCode 1 (not supported) | 1.19.5 (28 Oct) | Provider APIs, for example `POST https://ai-gateway.vercel.sh/v3/ai/language-model` | Yes (measured, Bun) | Same SQLite file as the parent (subagents unmeasured) | Off since 1.19.0 | `/update` or `opencode upgrade --major` moves a user to v2 |
| OpenCode 2 (not supported) | `@opencode/cli` 2.2.1 (29 Oct) | Unverified | Unverified: clients attach to a shared background service | `~/.local/share/opencode/opencode.db`, background subagents (third-party, unverified) | Unverified | Shared service started earlier with no proxy |
| OpenClaw (not supported) | 2026.10.8 (27 Oct); extended-stable 2026.9.35 | `POST https://ai-gateway.vercel.sh/v1/messages`; DeepSeek `/chat/completions` | Partly: needs `NODE_USE_ENV_PROXY=1`; `OPENCLAW_PROXY_URL` and `proxy.proxyUrl` win (measured) | Same SQLite file, `openclaw-agent.sqlite`, table `transcript_events` | Daily update check on; off with `OPENCLAW_NO_AUTO_UPDATE=1` | Own proxy setting bypasses the recorder |
| Cursor CLI (not supported) | 2026.10.22 | `api2.cursor.sh` backend; agent URL from server config; inference on Cursor's backend | Partly: `HTTPS_PROXY` with `NODE_USE_ENV_PROXY=1`, but HTTP/2 bidi by default and the server can force it | `agent-transcripts/<parent>/subagents/` (since 2026.10.17) plus SQLite `store.db` | Background update check on | Not recordable: our proxy refuses `h2` |
| GitHub Copilot CLI (not supported) | 1.0.105 (28 Oct) | `*.githubcopilot.com`, `*.individual.githubcopilot.com`; BYOK in `~/.copilot/providers.json` | Yes (changelog); HTTP/1.1 default | Same `~/.copilot/session-state/<session-id>/events.jsonl`, `agentId` field (unverified) | `autoUpdate` true, downloads at each session start | Updates at every start; sessions synced to GitHub by default |
| Gemini CLI (not supported) | 0.66.0 (27 Oct) | Google endpoints; `GOOGLE_GEMINI_BASE_URL`, `GOOGLE_VERTEX_BASE_URL` | Expected yes (Node), unverified for 0.66.0 | `~/.gemini/tmp/<project_hash>/chats/` (subagent location unverified) | `general.enableAutoUpdate` true | Retired for individual users since 18 Jun 2026 |
| Antigravity CLI `agy` (not supported) | 1.2.4 (tested by a third party, 8 Oct); current unverified | Google, `GOOGLE_GEMINI_BASE_URL` | Partly: user reports say since mid-2026; CA rules depend on Go version | Unknown | Self-updates in the background, no documented off switch | Closed Go binary that self-updates |
| Goose (not supported) | v1.57.0 (27 Oct) | Provider APIs; declarative AI gateway provider | Docs say yes | SQLite `~/.local/share/goose/sessions/sessions.db` | CLI manual (`goose update`); desktop auto-updates | CA handling unknown |
| Aider (not supported) | 0.86.2 (12 Feb 2026) | LiteLLM to providers | httpx reads it (unverified) | No subagents | `--check-update` on | Stale; history file in the repo folder |
| Cline CLI (not supported) | 3.0.74 (26 Oct) | Unverified | Not documented | `~/.cline/data/sessions/<id>/` (subagents unverified) | Not documented | Hub daemon over WebSocket |
| Kilo CLI (not supported) | 7.8.25 (28 Oct; `latest` tag says 7.8.18) | As OpenCode | Likely (OpenCode fork, Bun) | SQLite, as OpenCode | Unverified | Same shape as OpenCode 1 |
| Pi, Factory Droid, Qwen Code, Crush, Kimi CLI (not supported) | Pi 1.4.0 (21 Oct); Droid 0.251.0 (27 Oct); Qwen 0.26.3 (28 Oct); Crush 0.98.2 (23 Oct); Kimi 1.52.0 (22 Sep) | Unverified | Unverified (Node, native, Node, Go, Python) | Unverified | Droid has `disableAutoUpdate`; others unverified | Low use at Goal Cup |

## Where the parts overlap or disagree

Shared findings, stated once here and kept in each part's own words below:

- **Native caps.** D1, D2 and D3 each say the 32-stream and 1,024-file limits
  (`crates/goalcup-daemon/src/native_capture.rs:272-283`) and the 64 MiB finish cap
  (`owner.rs:13`) are too low for a week of subagents.
- **`NODE_USE_ENV_PROXY`.** D1 and D2 both find it a no-op for their harness; D4 shows OpenClaw and
  Cursor need it, so the schema support stays.
- **Server storage limit.** D1 (risk 7) and D2 (risk 5) both note that bigger native files count against
  `RUN_MAX_COMPRESSED_BYTES` (64 GiB). D2 estimates 1 to 2 GiB of copied history per 1,000 Codex
  children (unverified).
- **HTTP/1.1-only proxy.** D1, D2 and D4 all rely on it. D1 cites `docs/ARCHITECTURE.md:211`; D2 cites
  `crates/goalcup-cli/src/proxy/tls.rs:26-31`; D4 cites `tls.rs:26-30`. The ALPN text sits at lines
  26-30 (line 31 is a blank doc-comment line), so both ranges point at the same fact.
- **Weekly plan limits.** D1 (risk 9) and D2 (risk 7) both warn a 168-hour run can stall for days.

Where the parts, or a part and the repo, disagree (both sides kept):

| Topic | One side | Other side |
|---|---|---|
| Rust harnesses and our CA | D4 runtime table: "rustls with bundled roots would refuse our CA (unverified)" for Goose "and Codex" | D2: Codex adds `CODEX_CA_CERTIFICATE` to native roots for HTTPS and WebSocket ([`custom_ca.rs#L259-L283`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/http-client/src/custom_ca.rs#L259)), so the doubt applies to Goose only |
| Hermes 0.19.0 delegation | `docs/plans/2026-10-29-record-everything-progress.md:74`: failed twice on 29 Oct with "an internal executor error" | `docs/research/results/2026-10-22-local-test-day.md:40`, `:83`: two `delegate_task` children on the wire on a Mac on 22 Oct |
| Which Hermes version we measured | D3 point 1: "Our measured runs of 29 Oct used 0.19.0"; capture recipe on 0.19.0 (`connectors/hermes-cli/harness.toml:2`) | Launch prep and per-home gateways measured on 0.22.5 (`docs/harnesses/hermes-cli.md:116-129`); hosts seen in a 0.22.5 sandbox run (`docs/plans/2026-10-29-record-everything-phase2-notes.md:27-33`) |
| Hermes session auto-prune | v0.22.0 release notes: on by default | v0.22.5 code: `sessions.auto_prune` false (the code is what runs) |
| Hermes `HERMES_CA_BUNDLE` | Repo: ignored "on macOS" in 0.22 (`docs/harnesses/hermes-cli.md:130`; `docs/DECISIONS.md:979`) | D3 source read: ignored for model calls on every OS since 0.21.6 |
| Claude Code macOS Keychain | Repo: "shared across config directories" (`docs/harnesses/claude-code-cli.md:66`), "shared with your own Claude Code (measured)" (`docs/HARNESS-SETUP.md:409`) | Vendor docs: the Keychain entry is keyed to `CLAUDE_CONFIG_DIR`. Needs a re-measure |
| Codex rollout compression | `docs/harnesses/codex-cli.md:127`: compressed after 7 days | 0.165.0 source: `local_thread_store_compression` is UnderDevelopment and off |
| Codex release dates | Tag dates: 0.161.0 on 6 Oct, 0.164.0 on 22 Oct | Changelog: 7 Oct and 23 Oct (next UTC day) |
| Amp newest version | npm: `0.0.1793271832-g7615fe` | `static.ampcode.com/cli/cli-version.txt`: `0.0.1793274046-g0b6b2b` |
| Amp Bun runtime | `docs/harnesses/amp-cli.md:79`: Bun 1.4.1 | Newest binary strings: `Bun v1.4.4` |
| SQLite session stores | `docs/HARNESS-ONBOARDING.md:188`: a live SQLite store "Cannot be captured", stop | `README.md:137-138`: the chat file is a backup of the wire record; OpenCode and OpenClaw recorded wire-only on 28 Oct |

## D1. Claude Code (the Anthropic CLI)

Research date: 29 Oct 2026. Latest release checked: 2.1.312 (published 27 Oct 2026). Repo read at
`/home/user/goal-cup-proxy` (origin/main 4b226de2). Read-only: nothing in the repo was changed.

Source words: "vendor docs" = a page on code.claude.com fetched today (29 Oct 2026; the pages carry no
date of their own, so the fetch date is the date). "changelog" = the CHANGELOG.md in the
anthropics/claude-code repo, fetched today; release dates come from the npm registry. "repo" = this
repository. "unverified" = I could not confirm it from a vendor source or a measurement.

### D1.1 Short answer

1. Our connector is mostly right about where Claude Code writes its files. The vendor docs now
   confirm the subagent path we only had from one kit run.
2. Claude Code ships almost daily: 26 releases in the 30 days to 27 Oct. Expect about 6 more
   before 7 Nov and about 6 more during the run.
3. The biggest gap for a 168-hour run is scale, not format. One Claude Code session may run 20
   subagents at once (more with ultracode), a workflow may run 1,000 agents, and there is no total
   cap. Our native capture holds 32 open streams and 1,024 files for the whole run. That breaks
   point 2 on the first big fan-out.
4. Three ways Claude Code can run outside our recording: background sessions under a per-user
   supervisor ("agent view"), split-pane agent teams started by tmux, and a model provider other
   than `env.ANTHROPIC_BASE_URL` (Bedrock, Vertex, Foundry, Mantle, a Claude apps gateway).
   None is handled in the connector.
5. The facts sheet `docs/harnesses/claude-code-cli.md` is for 2.1.283 and has at least eight
   stale or wrong lines (section 7 below). The connector itself needs two small changes and one
   comment fix.

### D1.2 Release facts and cadence

| Fact | Value | Source |
|---|---|---|
| Latest | 2.1.312, 27 Oct 2026 18:36 UTC | npm registry `@anthropic-ai/claude-code`, https://registry.npmjs.org/@anthropic-ai%2Fclaude-code (dist-tags `latest` and `next`) |
| Stable channel | 2.1.308, published 22 Oct 2026 | same npm record (dist-tag `stable`); also https://downloads.claude.ai/claude-code-releases/stable returned `2.1.308` today |
| Releases in 30 days | 26 (2.1.285 on 29 Sep to 2.1.312 on 27 Oct); 52 in 60 days | npm `time` field, same URL |
| Gaps | 2.1.297, 2.1.298 never published; most weekdays get one release, some days two | npm `time` field |
| Stable channel lag | "typically about one week old, skipping releases with major regressions" | https://code.claude.com/docs/en/setup (fetched 29 Oct 2026) |
| Runtime | One native executable built on Bun ("Upgraded the bundled Bun runtime to 1.4", 2.1.181). The npm package only pulls the native binary per platform; "The installed `claude` binary does not itself invoke Node." | changelog 2.1.181, https://raw.githubusercontent.com/anthropics/claude-code/main/CHANGELOG.md ; https://code.claude.com/docs/en/setup |
| Platforms on npm | darwin-arm64, darwin-x64, linux-x64, linux-arm64, linux-x64-musl, linux-arm64-musl, win32-x64, win32-arm64 | npm `optionalDependencies` of 2.1.312 |

### D1.3 How Claude Code reaches models

#### D1.3.1 Hosts

From https://code.claude.com/docs/en/network-config (fetched 29 Oct 2026), "Network access requirements":

| Host | Use |
|---|---|
| `api.anthropic.com` | Model calls, WebFetch domain safety check, feature flags, telemetry event logging |
| `claude.ai`, `claude.com`, `sso.claude.com` | Browser sign-in |
| `platform.claude.com` | OAuth token exchange, refresh and revoke, for Console and claude.ai sign-ins |
| `mcp-proxy.anthropic.com` | claude.ai connectors (on by default for claude.ai users) |
| `downloads.claude.ai` | Native installer, auto-updater, update version checks, plugin downloads; since 2.1.311 also a signed model catalog on Bedrock, Vertex, Foundry, Mantle (changelog 2.1.311, `CLAUDE_CODE_MODEL_CATALOG=off`) |
| `storage.googleapis.com`, `registry.npmjs.org`, `github.com`, `raw.githubusercontent.com` | Plugins, npm installs, marketplaces, changelog feed at startup |
| `bridge.claudeusercontent.com` | Claude in Chrome WebSocket bridge |
| `*.frame.claudeusercontent.com` | Artifact reads |
| `http-intake.logs.us5.datadoghq.com`, `browser-intake-us5-datadoghq.com` | Optional telemetry and error reports |
| `code.claude.com` | Documentation lookups |

Our own measurement agrees for the model host: `POST /v1/messages`, HTTP/1.1, `text/event-stream`
(`docs/harnesses/claude-code-cli.md:72`), and the 2.1.312 sandbox run saw
`downloads.claude.ai` and `raw.githubusercontent.com` as sealed tunnels
(`docs/research/results/2026-10-vercel-gateway.md:65`).

#### D1.3.2 Credentials and where they live

From https://code.claude.com/docs/en/authentication (fetched 29 Oct 2026):

- Precedence: (1) `CLAUDE_CODE_USE_BEDROCK` / `_VERTEX` / `_FOUNDRY` cloud credentials,
  (2) `ANTHROPIC_AUTH_TOKEN`, (3) `ANTHROPIC_API_KEY` (interactive mode asks once),
  (4) `apiKeyHelper`, (5) `CLAUDE_CODE_OAUTH_TOKEN` (one-year token from `claude setup-token`),
  (6) Anthropic profile or federation credentials, (7) subscription OAuth from `/login`.
  A signed-in Claude apps gateway outranks all of them.
- macOS: Keychain. "If you've set the `CLAUDE_CONFIG_DIR` environment variable, Claude Code keeps
  the `.credentials.json` file under that directory instead ... and keys the macOS Keychain entry
  to that directory too, so a session with a different `CLAUDE_CONFIG_DIR` reads a different entry."
  If the Keychain refuses the write (locked, SSH), it falls back to `.credentials.json`, mode 0600.
- Linux: `~/.claude/.credentials.json` mode 0600, or under `CLAUDE_CONFIG_DIR`.
- Anthropic profiles live in `~/.config/anthropic` (macOS, Linux). Our run `HOME` hides them.
- A login near expiry warns 3 days ahead; an expired login that cannot refresh fails each model
  request with "Login expired". Background and Remote Control sessions "stop making progress once
  the credential expires".

#### D1.3.3 Base URLs, providers and gateways

| Route | How it is set | Where model calls go | Source |
|---|---|---|---|
| Subscription or API key, default | nothing | `api.anthropic.com` | network-config page |
| Custom endpoint (LLM gateway) | `ANTHROPIC_BASE_URL` in env or settings `env` | the gateway | https://code.claude.com/docs/en/env-vars (fetched 29 Oct 2026) |
| Bedrock / Mantle | `CLAUDE_CODE_USE_BEDROCK` or `CLAUDE_CODE_USE_MANTLE`, optional `ANTHROPIC_BEDROCK_BASE_URL` | `bedrock-runtime.<region>.amazonaws.com` or the base URL | env-vars page; changelog 2.1.309 |
| Vertex | `CLAUDE_CODE_USE_VERTEX`, optional `ANTHROPIC_VERTEX_BASE_URL` | Google endpoint | env-vars page |
| Foundry | `CLAUDE_CODE_USE_FOUNDRY`, `ANTHROPIC_FOUNDRY_RESOURCE` | Azure endpoint | env-vars page; changelog 2.1.284 |
| Claude apps gateway (new, self-hosted, since 2.1.195) | `/login` then "Cloud gateway", or `forceLoginMethod: "gateway"` and `forceLoginGatewayUrl` in managed or (since 2.1.295) user settings | the gateway; `/login` requires it to resolve to a private address, shows a TLS fingerprint on first connect, and refuses when `HTTPS_PROXY` applies to it | https://code.claude.com/docs/en/claude-apps-gateway (fetched 29 Oct 2026); changelog 2.1.295 |

Note: the WebFetch tool's domain check and the fast-mode check still go to `api.anthropic.com`
even behind a gateway (network-config page).

#### D1.3.4 Proxy and custom CA

| Question | Answer | Source |
|---|---|---|
| Honours `HTTPS_PROXY`? | Yes. Reads the first set of `https_proxy`, `HTTPS_PROXY`, `http_proxy`, `HTTP_PROXY`. `NO_PROXY` honoured (space or comma list, `*` for all). No SOCKS. A proxy URL it cannot parse stops launch. | network-config page |
| WebSockets and loopback | "Claude Code never sends its WebSocket connections to `localhost`, `::1`, or `127.0.0.0/8` through the proxy." | network-config page |
| Custom CA | `NODE_EXTRA_CA_CERTS` appends to the trust set. Default trust is `bundled,system` via `CLAUDE_CODE_CERT_STORE` (OS store default since 2.1.101). `SSL_CERT_FILE` is not documented for the CLI's own requests (only named as a git/tool variable in changelog 2.1.300). | network-config page; changelog 2.1.101, 2.1.300 |
| Settings can set them | "All environment variables shown on this page can also be configured in `settings.json`." This includes project and local settings for a terminal session. Settings `env` values are re-applied to a running session when the file is saved (agent teams page). | network-config page; https://code.claude.com/docs/en/agent-teams (fetched 29 Oct 2026) |
| `NODE_USE_ENV_PROXY` | Not needed. The binary is Bun, not Node, and Claude Code reads the proxy variables itself. Not documented anywhere on the env-vars page. | changelog 2.1.181; env-vars page |
| HTTP/2 | No documented HTTP/2 for model calls; we measured HTTP/1.1. Changelog 2.1.208 mentions "a server closed an HTTP/2 connection with a GOAWAY" for supervised and background sessions, so some client paths can speak HTTP/2. Our recorder terminates TLS and picks ALPN itself, so a client that offers h2 and http/1.1 gets http/1.1 (unverified for every path). | changelog 2.1.208; `docs/ARCHITECTURE.md:211` |
| Pinned certs | Only the Claude apps gateway sign-in shows a fingerprint on first connect. Nothing else documented. | claude-apps-gateway page |
| CA filtering | 2.1.307 says the VS Code extension and self-hosted runner now "filter their trusted certificates as the CLI does". The CLI filters its trust list by some rule that is not documented (unverified what it drops). Our name-constrained CA still worked on 2.1.312 (wire calls recorded through the gateway host, `docs/research/results/2026-10-vercel-gateway.md:53`). | changelog 2.1.307 |
| Proxy bugs in the past | Native (Bun) build ignored `HTTP_PROXY` for streaming API calls (issue #14165, closed Feb 2026); `NO_PROXY` ignored on Linux native 2.1.83 (issue #39862, closed Apr 2026); `NO_PROXY` ignored for Claude Code's own non-model requests until 2.1.292. So proxy handling regresses from time to time. | https://claudeissues.com/issue/14165-bug-native-binary-installation-ignores-http-proxy-for-api-streaming-requests-bun ; https://claudeissues.com/issue/39862-bug-no-proxy-completely-ignored-claude-code-v2-1-83-routes-all-traffic-through-f ; changelog 2.1.292 |

### D1.4 Subagents, workflows, teams and background sessions

| Kind | How it runs | Where its files are | Limits | Source |
|---|---|---|---|---|
| Subagent (Agent tool) | In the session; "a subagent that Claude spawns from your main conversation starts in the background" by default | `~/.claude/projects/{project}/{sessionId}/subagents/agent-{agentId}.jsonl` (vendor docs now say so) | 20 running at once per session (`CLAUDE_CODE_MAX_CONCURRENT_SUBAGENTS`, cannot be disabled); nesting 3 layers (`CLAUDE_CODE_MAX_SUBAGENT_SPAWN_DEPTH`); "There's no limit on the total number of subagents" | https://code.claude.com/docs/en/sub-agents ; env-vars page |
| Workflow agents (dynamic workflows, `ultracode`, `/deep-research`) | A JavaScript script run by a runtime in the background. The docs speak of "its agents' processes", so agents may be separate processes (unverified). | The script and saved results are "under your session's directory in `~/.claude/projects/`"; our kit saw `<session>/subagents/workflows/<id>/agent-*.jsonl` on disk (`connectors/claude-code-cli/harness.toml:104-105`) | 16 at once by default (`CLAUDE_CODE_WORKFLOW_MAX_CONCURRENT_AGENTS`, 1 to 256); 1,000 agents per run; 4,096 items per `parallel()`; ultracode lifts the 20-subagent cap; stalled agents restart up to 5 times | https://code.claude.com/docs/en/workflows (fetched 29 Oct 2026) |
| Agent teams | Experimental, off unless `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1`. In-process by default; split panes start "separate Claude Code instances" in tmux or iTerm2 | Team config `~/.claude/teams/{team}/config.json` (removed at end), mailboxes `~/.claude/teams/{team}/inboxes/{agent}.json`, tasks `~/.claude/tasks/{team}/`. Teammate transcript path not documented (unverified) | "no hard limit" on teammates | agent-teams page |
| Background sessions (agent view, `claude agents`, `--bg`, `/background`, the left-arrow key) | "A per-user supervisor process starts on demand, outlives your shell, and hosts every `claude agents`, `--bg`, and `/background` session." It "inherits the environment of whichever shell starts it first." With `CLAUDE_CONFIG_DIR` set it "runs as a separate instance". It restarts itself onto a new version after an auto-update. | `daemon.log`, `daemon/roster.json`, `jobs/<id>/state.json` under the config dir; transcripts still under `projects/` (picker shows them) | Off with `CLAUDE_CODE_DISABLE_AGENT_VIEW=1` or `disableAgentView` | https://code.claude.com/docs/en/agent-view (fetched 29 Oct 2026); network-config page, "Apply network settings to background agents"; env-vars page |

### D1.5 Session files and format

From https://code.claude.com/docs/en/sessions (fetched 29 Oct 2026):

- Path: `<config>/projects/<project>/<session-id>.jsonl`; `<project>` is the working directory with
  non-alphanumerics replaced by `-`, cut to 200 characters plus a hash when longer.
  `CLAUDE_CODE_PROJECT_DIR_NAME` (with `CLAUDE_CONFIG_DIR`, env only, 2.1.234+) fixes the name.
- Each line is "a message, a tool use or result, a record of a request Claude Code sent the model,
  or a metadata entry. The entry format is internal ... and changes between versions."
- Since 2.1.300 transcripts "also record each API request's system prompt, tools and parameters"
  (changelog 2.1.300, published 14 Oct 2026). This makes transcripts larger and means our
  facts-sheet claim that the transcript omits the system prompt and tool schemas is now false.
- `/cd` moves a session to the new directory's project storage. `/branch` and `--fork-session`
  copy to a new file. Two terminals on one session interleave into one file.
- `CLAUDE_CODE_TRANSCRIPT_LOCAL_GC=1` rewrites (trims) a transcript after compaction, but only for
  `-p` and SDK sessions and only when set in the launch environment (env-vars page, 2.1.287+).
  Chat mode is not affected.
- `CLAUDE_CODE_SKIP_PROMPT_HISTORY=1` stops transcript writing entirely.
- Retention: `cleanupPeriodDays`, 30 by default, also for subagent transcripts (sub-agents page).
- Large tool output spills to the session's `tool-results` folder (changelog 2.1.304 names it).

### D1.6 Auto-update and how to stop it

From https://code.claude.com/docs/en/setup (fetched 29 Oct 2026):

- Native installs check "on startup and periodically while running"; updates install in the
  background and "take effect the next time you start Claude Code". Homebrew, WinGet, apt, dnf
  and apk do not auto-update.
- `DISABLE_AUTOUPDATER=1` stops only the background check; `claude update` and `claude install`
  still work. `DISABLE_UPDATES=1` blocks every update path.
- `autoUpdatesChannel`: `latest` (default) or `stable`. `minimumVersion` sets a floor. Managed
  `requiredMinimumVersion` and `requiredMaximumVersion` refuse to start outside a range.
- Exact install: `curl -fsSL https://claude.ai/install.sh | bash -s 2.1.308` (any version number).
- Launcher: `~/.local/bin/claude` symlinks into `~/.local/share/claude/versions/`. The installer
  keeps "the version the launcher points to and any version a session on the same machine is
  running", plus the two newest, and deletes the rest. This answers our open question at
  `docs/harnesses/claude-code-cli.md:103`: another install's updater does not prune a version a run
  is using on the same machine.
- `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC` also disables auto-updates, telemetry, release notes
  and feature flags (env-vars page). Feature flags off disables Remote Control and some features.
- The background supervisor restarts itself onto a new version after an update (agent-view page).

### D1.7 Changes in the last 30 days that matter to us

All from the changelog, https://raw.githubusercontent.com/anthropics/claude-code/main/CHANGELOG.md ;
dates from npm.

| Version (date) | Change | Why it matters |
|---|---|---|
| 2.1.285 (29 Sep) | Sessions behind a custom `ANTHROPIC_BASE_URL` use 1M context | Larger request bodies through the recorder |
| 2.1.287 (1 Oct) | `CLAUDE_CODE_TRANSCRIPT_LOCAL_GC` added | Rewrites files, `-p` only; chat mode safe |
| 2.1.288 (2 Oct) | `CLAUDE_CODE_RETRY_WATCHDOG` gives up after three stream timeouts; npm updater placeholder fix | Our retry setting behaves differently on long streams |
| 2.1.292 (6 Oct) | `NO_PROXY` now honoured for Claude Code's own API requests (sign-in, policy, feedback, artifacts) when `HTTPS_PROXY` is set | Our `NO_PROXY=127.0.0.1,localhost` is safe; a participant `NO_PROXY` in settings now has more effect |
| 2.1.295 (8 Oct) | `CLAUDE_CODE_RETRY_WATCHDOG_MAX_WAIT_MS`; `forceLoginGatewayUrl` allowed in user settings | New gateway route our D74 rule does not read |
| 2.1.299 (14 Oct) | Streams through a custom base URL wait 300 s, not 180 s | Long idle streams through the recorder |
| 2.1.300 (14 Oct) | Transcripts record each API request's system prompt, tools and parameters; first entry after resume no longer lost after a crash cut the last line | Larger native files; facts sheet line 60 now wrong |
| 2.1.301 (15 Oct) | `CLAUDE_CODE_RETRY_WATCHDOG_MAX_USAGE_LIMIT_WAIT_MS` | Usage-limit waits can be bounded |
| 2.1.304 (19 Oct) | Tool search on by default behind a custom base URL; tool-results folder copy fix | |
| 2.1.305 (20 Oct) | `CLAUDE_CODE_RETRY_WATCHDOG_USAGE_LIMIT_RECHECK_MS` | |
| 2.1.307 (21 Oct) | Subagent transcripts had placeholder token usage and empty stop reason for streamed responses (fixed); "filter trusted certificates as the CLI does" | Our native counts from older subagent files may read zero usage; CA filtering rule undocumented |
| 2.1.309 (22 Oct) | Long tool output previews no longer cut an emoji, which had left transcripts `jq` could not parse | Older versions can write invalid JSON lines |
| 2.1.310 (23 Oct) | Background cleanup deleted recent files when `cleanupPeriodDays` was very large (fixed) | |
| 2.1.311 (26 Oct) | `claude -p` and SDK transcripts saved out of order (fixed); `SubagentStop` hooks ran twice a minute per subagent (fixed); retry watchdog no longer retries forever on requests marked not retryable; model catalog from `downloads.claude.ai` on 3P providers | |
| 2.1.312 (27 Oct) | Subagent `autoCompactWindow` can lower the window | |

### D1.8 What our repo assumes, and what is stale or wrong

#### D1.8.1 Connector `connectors/claude-code-cli/harness.toml` (159 lines, data)

| Line(s) | Assumes | Status | Evidence |
|---|---|---|---|
| 1 | "Claude Code 2.1.283" | Stale. Kit used 2.1.310 and 2.1.312. | npm; `docs/research/results/2026-10-vercel-gateway.md:102` |
| 34 | Auth host `platform.claude.com` | Right, but incomplete for first sign-in (`claude.ai`, `claude.com`, `sso.claude.com`). They tunnel anyway, so no change. | network-config page |
| 36-40 | Model host is `api.anthropic.com` or `env.ANTHROPIC_BASE_URL` in user `settings.json` | Incomplete. Misses `CLAUDE_CODE_USE_BEDROCK/VERTEX/FOUNDRY/MANTLE`, the provider base URLs, the Claude apps gateway, and `ANTHROPIC_BASE_URL` in project or local settings. Each tunnels and is not recorded on the wire. The repo already lists this as open (`docs/plans/2026-10-29-record-everything-phase2-notes.md:369`, ledger L24). | authentication page; claude-apps-gateway page |
| 50 | `--continue` appends to the same file | Right; but `--continue` refuses when the latest session is still running in the background ("Your most recent conversation is running in the background") | sessions page |
| 58-59 | "Node's built-in HTTP client honours HTTPS_PROXY ... only with this (D59)" | Wrong for Claude Code. The binary is Bun and reads the proxy variables itself. The line is a harmless no-op. | changelog 2.1.181; setup page; network-config page; Amp row says the same for Bun at `connectors/amp-cli/harness.toml:43-44` |
| 60 | `DISABLE_AUTOUPDATER = "1"` | Right for the background check. Does not stop `claude update` run by the agent. Does not stop a background supervisor from restarting onto a new version. | setup page; agent-view page |
| 61-65 | `CLAUDE_CODE_RETRY_WATCHDOG` comment: 300 retries, 429/529 indefinitely, spend limit fails at once | Mostly right. Since 2.1.288 long streams give up after three timeouts; since 2.1.311 non-retryable errors are not retried. | env-vars page; changelog |
| 82-84 | CA via `NODE_EXTRA_CA_CERTS`, mode extend | Right. Default trust also includes the OS store. | network-config page |
| 92-148 | Six native rows: main, subagents, subagent meta, workflow subagents, workflow meta, tool-results | Paths right; subagent path now confirmed by vendor docs. Not covered: teammate files, team config and inbox JSON (outside `projects/`), workflow scripts and saved results (path unverified), memory files under `projects/<project>/memory/`. | sub-agents page; workflows page; sessions page |
| 100-107 comment | Subagent pattern is "community write-ups only, not vendor docs" | Stale. Vendor docs now state it. | sub-agents page |
| 150-152 | Terminate by process group, 10 s | Does not reach a background supervisor, which outlives the shell. | agent-view page |
| 156-159 | Billing header prefix redaction | Still needed: since 2.1.300 the transcript records the system prompt, where the billing text sits (`docs/harnesses/claude-code-cli.md:60` says the transcript omitted it on 2.1.258). | changelog 2.1.300 |

#### D1.8.2 Facts sheet `docs/harnesses/claude-code-cli.md` (170 lines, doc)

| Line | Says | Status |
|---|---|---|
| 3 | Facts for 2.1.283, last checked 13 Oct | Stale. Current is 2.1.312. |
| 32 | "Install an exact version: unknown" | Wrong now: `install.sh | bash -s <version>` (setup page). |
| 33 | Stop self-update with `DISABLE_AUTOUPDATER=1` | Incomplete: `DISABLE_UPDATES=1` blocks manual updates too. |
| 45 | Linux config dir unknown | Answered: same `~/.claude` or `CLAUDE_CONFIG_DIR` (sessions, authentication pages). |
| 46 | Subagent pattern "community write-ups only" | Stale: vendor docs confirm. |
| 60 | "The transcript omits the system-reminder block, tool schemas and the billing header" | Wrong since 2.1.300 for system prompt and tools (changelog 2.1.300; sessions page "a record of a request"). |
| 66 | macOS Keychain "shared across config directories" | Contradicted by vendor docs: the Keychain entry is keyed to `CLAUDE_CONFIG_DIR`. Linux credentials: `.credentials.json` in the config dir (no longer unknown). |
| 74 | Connector hosts unknown | Answered: `mcp-proxy.anthropic.com`; `ENABLE_CLAUDEAI_MCP_SERVERS=false` alone stops the fetch. |
| 76 | Update host unknown | Answered: `downloads.claude.ai` (Homebrew: `formulae.brew.sh`). |
| 79 | Base URL overrides `ANTHROPIC_BASE_URL`, `ANTHROPIC_API_URL` | Incomplete: provider switches, provider base URLs, apps gateway. `ANTHROPIC_API_URL` is not on the env-vars page (unverified that it still works). |
| 83 | "subagents run in-process" | Partly stale: workflow agents may be processes; split-pane teammates are separate instances; background sessions run under a supervisor. |
| 103 | Whether another updater could prune a running version | Answered: the installer keeps any version a session on the same machine runs. |
| 163-170 | 32 open streams, 1,024 files | Accurate for the code, but far below what Claude Code can produce (section 4). |

#### D1.8.3 Setup guide `docs/HARNESS-SETUP.md`

| Line | Says | Status |
|---|---|---|
| 145-168 | Gateway via `env.ANTHROPIC_BASE_URL` plus `ANTHROPIC_AUTH_TOKEN` in `settings.json`; a base URL in workspace `.claude/` settings is not read | Right, and measured on 2.1.312. Should also say: no Bedrock, Vertex, Foundry, Mantle or apps gateway, and no `HTTPS_PROXY`, `NO_PROXY` or `CLAUDE_CODE_CERT_STORE` in any settings `env` block. |
| 394 | Claude Code turn limit not measured | Still true; the 2-hour run report (`docs/research/results/2026-10-30-two-hour-run.md`) does not exist on any branch yet. |
| 409 | macOS: "your login Keychain, shared with your own Claude Code (measured)" | Contradicted by vendor docs (entry keyed per config dir). Probably the participant must sign in inside the Goal Cup home once. Needs a re-measure on the current version. |
| 423 | "Subagents run inside the same process" | Partly stale, as 8.2 line 83. |

#### D1.8.4 Code limits (`crates/goalcup-daemon/src/native_capture.rs:272-283`, `crates/goalcup-protocol/src/capture.rs:58`)

`max_open_streams = MAX_OPEN_STREAMS / 4` = 32, `max_files = 1024`, hard ceiling
`MAX_NATIVE_FILES = 20_000` (`native_capture.rs:187`). The facts sheet says append-only streams stay
open until the run ends (`docs/harnesses/claude-code-cli.md:167`). With Claude Code's defaults (20
concurrent subagents, no total cap, 1,000 agents per workflow, each agent writing a `.jsonl` and a
`.meta.json`, plus every spilled tool result), a single workflow passes 32 streams and a busy day
passes 1,024 files.

### D1.9 Verdicts

| Part (path, src lines, test lines) | What it does | Points | Verdict | Reason | Lines saved (src / tests) | Risk | Tests that go |
|---|---|---|---|---|---|---|---|
| Main transcript row (`connectors/claude-code-cli/harness.toml:92-98`, data) | Records `projects/*/*.jsonl` since run start | 1, 5, 6 | Keep | Vendor-documented path; the only record of typed input and the agent's turns | 0 / 0 | Format is internal and changes per release; decoder already counts unknown types as unrecognized (`crates/goalcup-verifier/src/capture_analysis/native/claude_code.rs:31-36`) | none |
| Subagent rows (`harness.toml:108-140`, data) | Subagent and workflow agent transcripts and meta files | 2, 5 | Keep, and collapse to one transcript row and one meta row if the pattern matcher accepts a two-depth alternative (unverified) | Path now vendor-documented; workflow depth observed | 0 to 14 / 0 | One wider glob could pick up files we do not want; nested subagent location not documented (unverified, likely flat under the session) | none |
| Tool-results row (`harness.toml:142-148`, data) | Spilled tool output files | 5 | Keep only once the file cap is removed; otherwise cut first | The wire already records what the model read; these files eat the 1,024 cap fastest | 0 to 7 / 0 | Without it the server lacks the full output text the agent saw by path | none |
| Native caps (`native_capture.rs:272-283`) | 32 open streams, 1,024 files | none (they guard no stated cost) | Change: remove the file cap; do not hold one descriptor per append-only file, reopen on growth instead | Claude Code fan-out exceeds both on a normal workflow | small, in daemon (other lanes measure) | macOS default 256 open files per process; reopen-on-growth must not miss a rewrite or a move (`/cd`) | the limit tests in `goalcup-daemon` (other lane) |
| `NODE_USE_ENV_PROXY` (`harness.toml:58-59`) | Node proxy switch | none for Claude Code | Cut for this connector | Bun binary; no-op; comment is wrong | 2 / 0 | None found; npm install also runs the native binary | none |
| `DISABLE_AUTOUPDATER` (`harness.toml:60`) | Stops the in-run update check | 3, 7 | Keep; consider adding `DISABLE_UPDATES = "1"` | Keeps the hashed build the one that runs | 0 / 0 | An agent's `claude update` changes the binary for `goalcup continue` (still recorded by D53) | none |
| `CLAUDE_CODE_RETRY_WATCHDOG` (`harness.toml:65`) | Long retry for unattended runs | 3 | Keep, fix the comment | A week-long run will meet outages and usage limits | 0 / 0 | A weekly usage limit can stall days; that is the harness, not us | none |
| Add `CLAUDE_CODE_DISABLE_AGENT_VIEW = "1"` (new, 1 line) | Stops the background supervisor | 1, 2, 3 | Add (owner decision) | A supervisor outlives the run, keeps the run's loopback proxy env, and is not in the process tree we stop | -1 / 0 | Participants who use agent view lose it; tell them in the participant notice (point 6). If not added, measure what happens when a participant presses the left-arrow key mid-run. | none |
| Model host rule D74 for Claude (`harness.toml:40`; `crates/goalcup-cli/src/model_host.rs`, 1,540 lines src incl. tests, shared) | Opens `env.ANTHROPIC_BASE_URL` beside the default | 1, 7 | Keep as is; add a doc line and, at most, one `goalcup prep` warning for `CLAUDE_CODE_USE_*`, `forceLoginGatewayUrl`, and proxy or CA keys in any settings `env` | Least code; the verifier already labels a run with native activity and no wire calls (`docs/research/results/2026-10-vercel-gateway.md:127`) | 0 / 0 | A Bedrock or gateway participant gets a run with no wire record; detected after, not prevented | none |
| Facts sheet (`docs/harnesses/claude-code-cli.md`, 170 lines doc) | Harness facts | 6, 7 | Change: refresh to 2.1.312 with the lines in 8.2; drop the headless recipe section (lines 13-26), since D57 removed headless | Stale facts mislead the next agent | about 15 doc lines | Removing headless facts loses measurements a later headless connector would need (they stay in git history) | none |
| Claude native decoder (`crates/goalcup-verifier/src/capture_analysis/native/claude_code.rs`, 219 lines src) | Counts turns and usage from transcripts | 5, 6 | Keep | Small; tolerant of new record types | 0 / 0 | Since 2.1.300 new request records may be counted as unrecognized noise; check the report wording | none |

### D1.10 Risks for a run that starts 7 Nov 2026 and lasts 168 hours

| # | Risk | Likelihood | Effect | What to do |
|---|---|---|---|---|
| 1 | About 6 Claude Code releases between 29 Oct and 7 Nov and about 6 more during the run. Native installs update between segments (`goalcup continue` starts the new build). | Certain | A format or proxy regression lands mid-event. Proxy handling has regressed before (#14165, #39862). | Re-run the kit on the newest build on 5 or 6 Nov. Tell participants to use `autoUpdatesChannel: "stable"` or install a fixed version with `bash -s <version>` (a recommendation, like D67). Record the build at each segment (D53 already does). |
| 2 | Subagent and workflow fan-out passes 32 streams or 1,024 files | High for any prompt that asks for parallel work; certain with `ultracode` | Point 2 fails: later subagents have only a gap | Remove the caps (owner decision already made); test with a prompt that asks for a 50-agent workflow |
| 3 | Background session or supervisor (left-arrow key, `/background`, `--bg`) | Medium: the key is one press away and the UI offers it | Sessions outlive the run; after the run their calls fail at a dead loopback proxy; our stop does not reach them | `CLAUDE_CODE_DISABLE_AGENT_VIEW=1`, or measure and document |
| 4 | Provider or gateway outside `env.ANTHROPIC_BASE_URL` (Bedrock, Vertex, Foundry, Mantle, apps gateway, project-settings base URL) | Low to medium (company accounts) | Model calls tunnel, not recorded; run labelled native only | Say so plainly in the setup guide; optional prep warning |
| 5 | Participant or agent writes `HTTPS_PROXY`, `NO_PROXY="*"` or `CLAUDE_CODE_CERT_STORE` into a settings `env` block (project `.claude/settings.json` is writable by the agent and re-applied live) | Low | Calls bypass the recorder or fail TLS; whether settings override our launch env is unverified | Verifier label catches the bypass; add to the cheat checklist |
| 6 | Agent teams in split-pane mode (tmux, iTerm2) | Low: experimental and off by default | Teammates are new processes started by tmux; their env comes from the tmux server, may lack our proxy (unverified) | Document: teams only in the default in-process mode |
| 7 | Transcript size grows since 2.1.300 (each request's system prompt, tools and parameters) | Certain on 2.1.300+; size per request unmeasured | Bigger uploads; counts against `RUN_MAX_COMPRESSED_BYTES` (64 GiB) | Measure bytes per hour in the 2-hour run (report not written yet) |
| 8 | OAuth login expiry or refresh over 7 days; Keychain locked after sleep on macOS (changelog 2.1.281, 2.1.307) | Medium on laptops | Harness stops making calls until `/login`; rules say contact support first (`docs/HARNESS-SETUP.md:400-409`) | Suggest `claude setup-token` (one year) as an option, or a fresh `/login` right before the start; re-measure the Keychain per-config-dir behaviour |
| 9 | Usage limits on a subscription (5-hour, weekly) | High over 168 hours | Watchdog waits; long idle stretches. Not a recording issue. | Tell participants it is normal (point 6) |
| 10 | Undocumented CA filtering (2.1.307 wording) drops a name-constrained root | Low (2.1.312 worked) | Every call fails TLS: blocks the harness (point 3) | Kit check on the release used on 7 Nov |
| 11 | Running as root in the Vercel Sandbox | Medium for the owner's 2-hour run | Claude Code refuses bypass mode as root unless `IS_SANDBOX=1` (`docs/harnesses/claude-code-cli.md:149`) | Run as uid 1000, as the ledger did (`docs/plans/2026-10-28-one-capture-system-ledger.md`, L18) |
| 12 | `/cd`, `/branch`, `--fork-session` move or copy a session file mid-run | Low | A new path appears; old stream ends. With `since_run_start` the new file is picked up; the old one must not be flagged as tampered | Include in the reconcile tests (other lane) |
| 13 | Remote Control (phone or browser types into the session) | Low | Input that is not the participant's terminal; a rules question, not a recording one | Transcript `origin.kind` shows it; mention in RULES |

### D1.11 What could go wrong with these recommendations

- Removing the caps without closing idle descriptors can hit the macOS 256 open-file default and
  crash or stall the daemon. Measure open descriptors with 200 subagent files before shipping.
- Disabling agent view changes how some people use Claude Code (point 7). If the owner keeps it,
  the run must at least say plainly that background sessions were not recorded (point 6).
- Cutting `NODE_USE_ENV_PROXY` for Claude Code is safe for the Bun binary, but keep the schema
  support for Node harnesses (Codex).
- A `goalcup prep` warning for provider switches is new code. The doc-only path is cheaper and the
  verifier label already shows a run with no wire calls.
- Telling participants to use the stable channel means they run a build about a week old, which
  may lack a fix they need; a pinned version must be one we tested.

### D1.12 Unverified

- Whether workflow agents are separate OS processes, and whether they inherit `HTTPS_PROXY`.
- Where nested subagents (a subagent's subagent) and in-process teammates write their transcripts.
- Whether a settings `env` value overrides an `HTTPS_PROXY` or `NO_PROXY` given in the launch
  environment.
- The rule behind "filter trusted certificates as the CLI does".
- Whether `ANTHROPIC_API_URL` is still read.
- Bytes added per request by the 2.1.300 transcript change.
- The macOS Keychain behaviour with our `keychain_link` home on 2.1.312 (vendor docs say per
  config dir; our sheet says shared).
- Any 2-hour run result: `docs/research/results/2026-10-30-two-hour-run.md` does not exist yet.

### Sources (D1)

- npm registry, @anthropic-ai/claude-code (release times, dist-tags), fetched 29 Oct 2026: https://registry.npmjs.org/@anthropic-ai%2Fclaude-code
- Release channel pointers, fetched 29 Oct 2026: https://downloads.claude.ai/claude-code-releases/stable , https://downloads.claude.ai/claude-code-releases/latest
- Claude Code changelog (2.1.312 at top), fetched 29 Oct 2026: https://raw.githubusercontent.com/anthropics/claude-code/main/CHANGELOG.md
- Enterprise network configuration: https://code.claude.com/docs/en/network-config
- Environment variables: https://code.claude.com/docs/en/env-vars
- Advanced setup (install, updates, channels, pruning): https://code.claude.com/docs/en/setup
- Authentication: https://code.claude.com/docs/en/authentication
- Manage sessions (transcript paths and format): https://code.claude.com/docs/en/sessions
- Subagents: https://code.claude.com/docs/en/sub-agents
- Dynamic workflows: https://code.claude.com/docs/en/workflows
- Agent teams: https://code.claude.com/docs/en/agent-teams
- Agent view and background sessions: https://code.claude.com/docs/en/agent-view
- Claude apps gateway: https://code.claude.com/docs/en/claude-apps-gateway
- Issue #14165, native binary ignored HTTP_PROXY for streaming (closed Feb 2026): https://claudeissues.com/issue/14165-bug-native-binary-installation-ignores-http-proxy-for-api-streaming-requests-bun
- Issue #39862, NO_PROXY ignored on 2.1.83 Linux native (closed Apr 2026): https://claudeissues.com/issue/39862-bug-no-proxy-completely-ignored-claude-code-v2-1-83-routes-all-traffic-through-f
- Issue #15212, intermittent proxy bypass on Windows: https://claudeissues.com/issue/15212-bug-claude-code-intermittently-ignores-http-proxy-on-startup-windows

## D2. Codex CLI (OpenAI)

Lane D. Scope: how Codex CLI reaches models, how its subagents and session files work, updates,
release pace, changes in the last 30 days, and what that means for Goal Cup Proxy's connector
(`connectors/codex-cli/harness.toml`), its facts sheet (`docs/harnesses/codex-cli.md`) and the
code that serves them. Run window: start 7 Nov 2026, up to 168 hours.

### How this was checked (D2)

- Source: a shallow clone of `openai/codex` at tag `rust-v0.165.0` (the newest stable tag on
  29 Oct 2026), read only, plus a blobless fetch of tags since 15 Sep 2026 for dates and diffs.
  Source links below point to
  `https://github.com/openai/codex/blob/rust-v0.165.0/<path>#L<line>`. I read the files, I did
  not run Codex.
- Web: the official docs at `learn.chatgpt.com` (the `developers.openai.com/codex/*` pages now
  308-redirect there), the GitHub release page, and web search. Dates are given per fact.
- Repo: `/home/user/goal-cup-proxy` at 4b226de2 (29 Oct 2026). Line counts are `wc -l`.
- Nothing was built, no test was run, no harness was run, no secret was read.

### D2.1 Release pace and the last 30 days

Stable tags and their tag dates (from `git for-each-ref` on the fetched tags; tag pages at
`https://github.com/openai/codex/releases/tag/rust-v<version>`):

| Version | Tag date (local tz of tagger) | Notes |
|---|---|---|
| 0.153.0 | 2 Sep 2026 | The version our connector header and facts sheet still name |
| 0.156.1 | 22 Sep 2026 | Last version measured with ChatGPT sign-in and a native child (18-19 Oct probes) |
| 0.159.1 to 0.159.3 | 29 to 30 Sep 2026 | |
| 0.160.0 / 0.160.1 | 1 Oct / 5 Oct 2026 | |
| 0.161.0 | 6 Oct 2026 (changelog: 7 Oct) | |
| 0.162.0 / 0.162.1 | 8 Oct / 9 Oct 2026 | |
| 0.163.0 | 15 Oct 2026 | |
| 0.164.0 | 22 Oct 2026, 20:15 -0700 (changelog: 23 Oct) | |
| 0.165.0 | 27 Oct 2026 | Newest stable. Used in the 28 Oct gateway sandbox run |
| 0.166.0-alpha.6 | 28 Oct 2026 | Alphas ship almost daily |

- Pace: 30 stable tags and 16 minor versions between 26 Aug (0.150.0) and 27 Oct (0.165.0), so
  about one minor release a week plus hotfixes. In the last 30 days: 11 stable tags (0.159.1 to
  0.165.0). Expect 0.166.0 around 3 Nov and one or two more stable releases during a 168-hour run
  (estimate from the pace, unverified).
- Sources: [changelog](https://learn.chatgpt.com/docs/changelog) (fetched 29 Oct 2026; lists
  0.165.0 on 2026-10-27, 0.164.0 on 2026-10-23, 0.163.0 on 2026-10-15, 0.162.0 on 2026-10-08),
  [0.165.0 release](https://github.com/openai/codex/releases/tag/rust-v0.165.0) (27 Oct).

Changes in the last 30 days that touch us (changelog, fetched 29 Oct 2026):

| Release | Change | Why it matters |
|---|---|---|
| 0.161.0 | WebSocket-to-HTTP fallback honours server retry guidance; API-key model discovery on by default | Fallback path is real; an API-key run lists models from `api.openai.com` |
| 0.162.0 | Sub-agent activity records resolved model and effort; partial-history subagent forks removed; unknown TUI keys rejected in strict config validation; custom providers gain capability overrides | Rollout content changes; strict validation is off by default (`config/src/state.rs#L35`) |
| 0.163.0 | Spawned children stay provisional until the spawn result returns; multi-agent v2 spawning consolidated | Child rollout timing may differ from 0.156.1 measurement |
| 0.164.0 | Full agent forks preserve parent history by default; resume finds compressed histories; app-server WebSocket zstd (flag-gated); Windows proxy Negotiate/NTLM | Every forked child copies the parent history: more bytes per subagent |
| 0.165.0 | Built-in Google Cloud Vertex provider (`model_provider = "gcp"`); managed config loaders for custom config servers; cached decrypted auth | A new built-in provider our `model_host_from` cannot see |

Between 0.156.1 (last ChatGPT WebSocket measurement) and 0.165.0 the WebSocket dialer and
proxy-route code changed: 9 files, 602 insertions, 222 deletions in `websocket-client/src`,
`http-client/src/outbound_proxy.rs`, `custom_ca.rs` and `responses_websocket.rs` (`git diff
--stat rust-v0.156.1 rust-v0.165.0`). So the "ChatGPT WebSocket through our proxy" fact is
measured on older code.

### D2.2 How Codex reaches models

#### Hosts and paths (D2)

| Sign-in | Default base URL | Model path | Transport | Source |
|---|---|---|---|---|
| ChatGPT sign-in (also access token, PAT, agent identity) | `https://chatgpt.com/backend-api/codex` | `/responses` | WebSocket first (`wss://`, permessage-deflate), HTTPS POST + SSE fallback | [`model-provider-info/src/lib.rs#L82`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/model-provider-info/src/lib.rs#L82), [`#L431-L446`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/model-provider-info/src/lib.rs#L431-L446) |
| API key | `https://api.openai.com/v1` | `/responses` | Same: the built-in `openai` provider has `supports_websockets: true` whatever the sign-in | [`#L531-L566`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/model-provider-info/src/lib.rs#L566) |
| Custom `[model_providers.X]` | its `base_url` | `/responses` (`wire_api = "chat"` was removed) | HTTPS + SSE unless the table sets `supports_websockets = true` | [`#L105`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/model-provider-info/src/lib.rs#L105), [`#L204`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/model-provider-info/src/lib.rs#L204) |
| Built-ins with no table: `amazon-bedrock`, `gcp` (new in 0.165), `ollama`, `lmstudio` | provider-specific | | | [`#L695-L760`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/model-provider-info/src/lib.rs#L695) |

- WebSocket beta header `responses_websockets=2026-02-06`
  ([`core/src/client.rs#L176`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/core/src/client.rs#L176)).
  The `responses_websockets` feature flag is now "Removed": WebSockets are always on for a
  provider that supports them ([`features/src/lib.rs#L2141`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/features/src/lib.rs#L2141)).
- Fallback is per session: one failed WebSocket turn (for example HTTP 426) switches that session
  to HTTPS for the rest of its life ([`core/src/client.rs#L262-L263`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/core/src/client.rs#L262), [`#L2087-L2091`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/core/src/client.rs#L2087)).
- HTTPS request bodies are zstd-compressed when signed in with ChatGPT to the OpenAI provider
  (`enable_request_compression`, Stable, on by default:
  [`features/src/lib.rs#L1483`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/features/src/lib.rs#L1483),
  [`core/src/client.rs#L1770-L1778`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/core/src/client.rs#L1770)).
  Our HTTP relay decodes one zstd coding (`crates/goalcup-cli/src/proxy/http.rs:21-23`), so this
  is covered for recording; the server-side decoder must also expect it.
- Other hosts seen in source and runs: `auth.openai.com` (token refresh), other
  `chatgpt.com/backend-api/*` paths (models, plugins, apps, `wham` usage), `ab.chatgpt.com`
  (seen tunnelled, about 1 MiB, in the 28 Oct gateway run:
  `docs/research/results/2026-10-vercel-gateway.md:66`; purpose unverified),
  `api.github.com` and `formulae.brew.sh` for the update check
  ([`tui/src/updates.rs#L61-L62`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/tui/src/updates.rs#L61)),
  and a `gov.chatgpt.com` variant for government workspaces
  ([`codex-mcp/src/mcp/mod.rs#L383`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/codex-mcp/src/mcp/mod.rs#L383)).

#### Config keys and env vars that move the model host (D2)

- `model_provider` picks a provider; `[model_providers.<id>] base_url` sets its address. A table
  named `openai` can override the built-in's `base_url` too
  ([`model-provider-info/src/lib.rs#L736-L780`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/model-provider-info/src/lib.rs#L736)).
- **`openai_base_url`** (top-level key): "Base URL override for the built-in `openai` model
  provider" ([`config/src/config_toml.rs#L434-L435`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/config/src/config_toml.rs#L434),
  used at [`core/src/config/mod.rs#L3978-L3985`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/core/src/config/mod.rs#L3978)).
  **Our reader does not look at it** (see section 7).
- `chatgpt_base_url` moves the ChatGPT backend for non-model calls. Project-local
  `.codex/config.toml` cannot set either key (deny list:
  [`config/src/loader/mod.rs#L89-L100`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/config/src/loader/mod.rs#L89)).
- Config layers: `$CODEX_HOME/config.toml`, system `/etc/codex/config.toml`
  ([`#L80`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/config/src/loader/mod.rs#L80)),
  managed requirements (MDM, `requirements.toml`, and since 0.165 "custom config servers"),
  cloud config layers for business workspaces, and `-c` on the command line. A managed layer can
  force `model_provider` (`required_model_provider`,
  [`core/src/config/mod.rs#L4005-L4008`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/core/src/config/mod.rs#L4005)).
- `OPENAI_BASE_URL` and `CODEX_BASE_URL` env vars: Codex 0.165 does not read either for the model
  provider. `OPENAI_BASE_URL` is read only by the opt-in network-proxy credential broker
  ([`network-proxy/src/credential_broker/providers/openai.rs#L14`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/network-proxy/src/credential_broker/providers/openai.rs#L14));
  `CODEX_BASE_URL` does not appear in the source. Our launcher removing them is harmless but no
  longer needed for Codex.
- Legacy `profile = "x"` in `config.toml` now stops Codex with an error; profiles are
  `--profile x` with `x.config.toml` ([`core/src/config/mod.rs#L3493-L3500`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/core/src/config/mod.rs#L3493)).
  Already true in 0.153.0 (checked on the tag).

#### `$CODEX_HOME/.env` overrides the launch environment (D2)

At startup, before any thread, Codex loads `$CODEX_HOME/.env` and calls `set_var` for every key
that does not start with `CODEX_`
([`arg0/src/lib.rs#L270-L296`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/arg0/src/lib.rs#L270),
called at [`#L134`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/arg0/src/lib.rs#L134)).
It overwrites what the parent set. So a line `HTTPS_PROXY=...`, `NO_PROXY=*` or `SSL_CERT_FILE=...`
in that file replaces Goal Cup's values inside Codex, and model calls then skip the recorder.
`CODEX_CA_CERTIFICATE` itself cannot be overridden this way (it has the `CODEX_` prefix), and it
wins over `SSL_CERT_FILE`. Our launcher builds the env from nothing (`crates/goalcup-cli/src/launch.rs:13-41`)
but nothing checks this file, and `docs/HARNESS-SETUP.md:92-93` tells participants to put a
gateway key there. Accidental breakage is unlikely; deliberate bypass is easy. Unverified by a run.

#### ChatGPT sign-in versus API key (D2)

- Two paths: ChatGPT sign-in (plan credits) or an API key (API billing). Credentials are stored by
  `cli_auth_credentials_store`: `file` (default, `$CODEX_HOME/auth.json`), `keyring`, `auto`,
  `ephemeral`. Admins can force `forced_login_method` and a workspace. Source:
  [Authentication docs](https://learn.chatgpt.com/docs/auth.md) (no date on page; fetched
  29 Oct 2026); default `File` at
  [`config/src/types.rs#L136-L146`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/config/src/types.rs#L136).
- API key: `printenv OPENAI_API_KEY | codex login --with-api-key` stores it in the home
  ([`cli/src/main.rs#L527-L546`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/cli/src/main.rs#L527);
  `--api-key <value>` is deprecated). `OPENAI_API_KEY` in the environment is only a prefill for
  the onboarding screen ([`tui/src/onboarding/auth.rs#L852`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/tui/src/onboarding/auth.rs#L852));
  `CODEX_API_KEY` is read from the environment ([`login/src/auth/manager.rs#L1540`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/login/src/auth/manager.rs#L1540))
  but our launch never passes it and `.env` cannot set it.
- Device-code login (`codex login --device-auth`) exists for headless machines (auth docs above).
- Token refresh happens automatically against `auth.openai.com` (auth docs). Copying `auth.json`
  to a second home and refreshing there invalidates the first (our measurement,
  `docs/harnesses/codex-cli.md:144`).
- Plan limits: a ChatGPT plan has 5-hour and weekly allowances (feature text "five-hour and
  weekly allowance history" in [`features/src/lib.rs`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/features/src/lib.rs)).
  A 168-hour run can hit the weekly limit and stall. That is the participant's cost, not ours, but
  the participant must know.

#### Proxy, custom CA, and what can bypass them (D2)

| Path | Honours `HTTPS_PROXY` | Custom CA | Source |
|---|---|---|---|
| reqwest HTTPS clients (model HTTPS fallback, auth, models, plugins) | Yes, reqwest default policy (`ReqwestDefault`) unless `respect_system_proxy` | `CODEX_CA_CERTIFICATE`, else `SSL_CERT_FILE`; **adds** to native roots and forces rustls | [`http-client/src/custom_ca.rs#L14`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/http-client/src/custom_ca.rs#L14), [`#L259-L283`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/http-client/src/custom_ca.rs#L259), [`#L297-L334`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/http-client/src/custom_ca.rs#L297) |
| Responses WebSocket | Yes: tokio-tungstenite `proxy` feature reads `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`, `NO_PROXY`; explicit CONNECT path for proxy auth | Same CA policy via `maybe_build_rustls_client_config_with_custom_ca` | [`websocket-client/src/dialer.rs#L51-L60`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/websocket-client/src/dialer.rs#L51), [`custom_ca.rs#L187-L201`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/http-client/src/custom_ca.rs#L187) |
| `respect_system_proxy` (UnderDevelopment, off) | Tries macOS/Windows system proxy and PAC first, env second | | [`features/src/lib.rs#L1517`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/features/src/lib.rs#L1517), [`outbound_proxy.rs#L1-L5`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/http-client/src/outbound_proxy.rs#L1) |
| `system_proxy_fallback` (Stable, **on**) | After normal routing fails, retries via the system proxy, but only for cloud bootstrap GETs and the login token exchange, not model calls | | [`features/src/lib.rs#L1523`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/features/src/lib.rs#L1523), [`backend-client/src/client.rs#L366-L399`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/backend-client/src/client.rs#L366), [`login/src/server.rs#L700-L720`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/login/src/server.rs#L700) |

- HTTP/2: reqwest offers `h2` and `http/1.1` by ALPN; our MITM offers only `http/1.1`
  (`crates/goalcup-cli/src/proxy/tls.rs:26-31`), so it falls back. The only
  `http2_prior_knowledge` use is a local code-mode gRPC socket
  ([`code-mode/src/grpc_session/transport.rs#L100`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/code-mode/src/grpc_session/transport.rs#L100)).
- No certificate pinning or verification bypass found (`rg` for `danger_accept_invalid`,
  `tls_built_in_root_certs(false)`: none).
- Native runtime: Rust, rustls; the npm `codex` is a Node wrapper that only spawns the native
  binary and makes no network calls ([`codex-cli/bin/codex.js#L231-L243`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-cli/bin/codex.js#L231)).
  `NODE_USE_ENV_PROXY` therefore does nothing for Codex itself.
- Things that do not use the HTTP proxy: the `codex tcp-tunnel` HTTP/3 (QUIC, UDP) subcommand
  ([`tcp-tunnel/src/lib.rs#L1`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/tcp-tunnel/src/lib.rs#L1))
  and realtime WebRTC voice (`realtime-webrtc`). Both are opt-in and not used in an unattended
  chat run (unverified for voice in the TUI).
- Shared app-server daemon: the TUI reuses a background app-server if one listens on a socket
  under `CODEX_HOME`. Then the daemon, with its own environment, makes the model calls.
  Any `-c` override other than `features.*` or `tui.fullscreen_transcript`, or `--no-daemon`,
  forces the embedded server ([`tui/src/daemon_startup.rs#L60-L125`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/tui/src/daemon_startup.rs#L60),
  [`tui/src/lib.rs#L1056-L1093`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/tui/src/lib.rs#L1056)).
  Our connector passes `-c check_for_update_on_startup=false` and `-c projects=...`
  (`connectors/codex-cli/harness.toml:45-49`), so today it always runs embedded, by accident.
  `--no-daemon` exists from 0.156.1 on (not in 0.153.0, checked on the tags).

### D2.3 Subagents

- Tools: `spawn_agent`, `send_input`, `resume_agent`, `wait_agent`, `close_agent`,
  `send_message`, `followup_task`, `list_agents`
  ([`analytics/src/reducer.rs#L3660-L3668`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/analytics/src/reducer.rs#L3660)).
- `multi_agent` is Stable and on; `multi_agent_v2` Stable and off
  ([`features/src/lib.rs#L1529-L1536`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/features/src/lib.rs#L1529)).
- Limits: `agents.max_concurrent_threads_per_session` (alias `max_threads`) defaults to 6
  **concurrent** threads; `agents.max_depth` defaults to 1
  ([`core/src/config/mod.rs#L258`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/core/src/config/mod.rs#L258),
  [`#L268`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/core/src/config/mod.rs#L268)).
  The slot is released when a child closes ([`core/src/agent/registry.rs#L97-L120`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/core/src/agent/registry.rs#L97)),
  so the total over a week is not capped. Docs: subagents run when asked; they inherit model,
  effort, sandbox and approval unless configured; custom agents in `~/.codex/agents/` or
  `.codex/agents/` ([Subagents docs](https://learn.chatgpt.com/docs/agent-configuration/subagents.md),
  no date; fetched 29 Oct 2026).
- Files: each child is its own thread with its own rollout under
  `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<local time>-<thread id>.jsonl`
  ([`rollout/src/recorder.rs#L1770-L1792`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/rollout/src/recorder.rs#L1770)).
  A child is ephemeral only if the parent is. Parent and child edges also go to the
  agent graph store (SQLite) ([`core/src/agent/control.rs#L693-L715`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/core/src/agent/control.rs#L693)).
  Since 0.164.0, a full fork copies the parent's history into the child (changelog), so each
  child file starts with a copy of the parent context.
- `resume --last` only picks CLI or VS Code sessions, never a subagent
  ([`tui/src/lib.rs#L804-L813`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/tui/src/lib.rs#L804)),
  and filters by working directory unless `--all`
  ([`cli/src/main.rs#L359-L384`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/cli/src/main.rs#L359)).

### D2.4 Session files and format

- JSONL rollouts, one per thread, `{timestamp, ordinal?, type, payload}`; `thread/revert` writes
  a new `rollout-<ts>-<thread>_<rollout_id>.jsonl` ([`rollout/src/recorder.rs#L99-L103`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/rollout/src/recorder.rs#L99)).
  Side files: `session_index.jsonl`, `history.jsonl`, state SQLite.
- Compression of cold rollouts to `.jsonl.zst`: feature `local_thread_store_compression` is
  UnderDevelopment and off; it runs only at startup when on
  ([`features/src/lib.rs#L1351`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/features/src/lib.rs#L1351),
  [`core/src/thread_manager.rs#L488-L545`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/core/src/thread_manager.rs#L488)).
  If a participant turns it on, a later `goalcup continue` could see a rollout replaced by a
  `.zst` file. Low risk.
- Date folders use local time; a 168-hour run crosses 7 or 8 folders. The glob `*/*/*/rollout-*.jsonl`
  covers that.

### D2.5 Updates and install

- `check_for_update_on_startup = false` still disables the check and the prompt in 0.165
  ([`tui/src/updates.rs#L27-L30`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/tui/src/updates.rs#L27)).
  An update runs only if the user picks it at the prompt; managed requirements can also set the
  key ([`tui/src/debug_config.rs#L183`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/tui/src/debug_config.rs#L183)).
- Install methods now detected: npm, bun, pnpm, vite-plus, Homebrew cask, **standalone**
  (`curl -fsSL https://chatgpt.com/codex/install.sh | sh`, kept under
  `~/.codex/packages/standalone/...`), and a managed daemon
  ([`tui/src/update_action.rs#L8-L68`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/tui/src/update_action.rs#L8),
  [`install-context/src/lib.rs#L20-L65`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/install-context/src/lib.rs#L20)).
  The README now lists the standalone script first, npm and Homebrew after
  ([README](https://github.com/openai/codex/blob/rust-v0.165.0/README.md), 27 Oct 2026).
  0.164.0 prunes unused standalone releases after an install (changelog).
- The standalone binary lives under the participant's real `~/.codex`, not under Goal Cup's
  `CODEX_HOME`. Our `real_dirs` check refuses only a config home there
  (`docs/CONNECTOR-SCHEMA.md:518`), so it should not refuse the executable (unverified by a run).
- Outside updates still change the binary between `start` and `continue`: `brew upgrade`, an npm
  global update, or a standalone reinstall. The running process keeps its old inode.

### D2.6 What the repo assumes today

| Claim in the repo | Where | Status on 0.165.0 |
|---|---|---|
| "Codex CLI 0.153.0" capture profile | `connectors/codex-cli/harness.toml:1` | Stale header: 0.165.0 was used on 28 Oct (`docs/HARNESS-SETUP.md:113`) |
| Facts for 0.153.0, last checked 13 Oct | `docs/harnesses/codex-cli.md:3` | Stale. The sheet mixes 0.153, 0.156.1 and a 0.165 gateway run |
| `codex` on PATH is the npm wrapper | `docs/harnesses/codex-cli.md:109`, `harness.toml:95` | Partly stale: standalone and Homebrew installs give the native binary directly. `native_pid` copes with a direct native (`crates/goalcup-daemon/src/terminate/darwin.rs:13-27`), unverified on Linux |
| `CODEX_CA_CERTIFICATE`, `mode = "extend"`, handed the proxy CA alone | `harness.toml:78-80`, `crates/goalcup-cli/src/ca_bundle.rs:17-19,37-43` | Correct. Codex adds the file to native roots for HTTPS and WebSocket |
| `NODE_USE_ENV_PROXY = "1"` (D59) | `harness.toml:60-61` | Does nothing for Codex: the wrapper has no network code |
| Model host from `model_providers.*.base_url` via `model_provider`, else defaults | `harness.toml:33-37`, `crates/goalcup-cli/src/model_host.rs:334-405` | Misses `openai_base_url`, an `[model_providers.openai] base_url` override with no `model_provider` line, built-ins `amazon-bedrock`, `gcp`, `ollama`, `lmstudio`, and system or managed layers. Each sends model calls to a host that stays an opaque tunnel (`model_host.rs:5-7`): unrecorded |
| Hosts `chatgpt.com` `/backend-api/codex/responses`, `api.openai.com` `/v1/responses` | `harness.toml:122-130` | Correct for both sign-ins. Every path on an opened host is recorded (Task 1, `docs/plans/2026-10-29-record-everything-progress.md:24`) |
| API key "not yet verified", how Codex stores it unknown | `docs/harnesses/codex-cli.md:142,150`, `docs/HARNESS-SETUP.md:115-122` | Stale: Task 9 live PASS (`progress.md:32,73`); method is `codex login --with-api-key` into `auth.json` |
| `OPENAI_BASE_URL`, `CODEX_BASE_URL` removed and refused | `docs/harnesses/codex-cli.md:157` | Harmless; Codex reads neither for models. The real gap is `$CODEX_HOME/.env` (section 2) |
| Subagents "not yet checked in a live run" | `docs/harnesses/codex-cli.md:165` | Stale: the 19 Oct probe (`codex-cli.md:62-83`) and the 29 Oct run (`progress.md:73`) recorded a child rollout |
| Rollouts compressed after 7 days | `docs/harnesses/codex-cli.md:127` | Off by default in 0.165 (UnderDevelopment) |
| Seven `extra_patterns` for plan, credits, rate limits, ids, turn state | `harness.toml:101-117` | None is a secret. Owner decision: drop the rate-limit rules. The two ids and the turn token are not secrets either |
| `[tui]` hide keys exist | `docs/HARNESS-SETUP.md:465` | Still exist ([`config/src/types.rs#L1058-L1062`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/config/src/types.rs#L1058)) |
| `-c` args, `-C`, `resume --last` | `harness.toml:45-52` | All still valid in 0.165 ([`cli/src/main.rs#L351`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/cli/src/main.rs#L351), [`#L367`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/cli/src/main.rs#L367); `projects.*.trust_level` at [`config/src/config_toml.rs#L477`](https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/config/src/config_toml.rs#L477)) |
| Native budget 1024 files, 32 open streams | `crates/goalcup-daemon/src/native_capture.rs:272-283` | Too low for a week of subagents (owner already decided to raise or remove) |

### D2.7 Keep, cut, change (Codex lane)

Verdict format from the brief. Lines are from `wc -l`; "data" means TOML or Markdown, not Rust.

| Part (path, src lines, test lines) | What it does | Points | Verdict | Reason | Lines saved (src / tests) | Risk | Tests that go |
|---|---|---|---|---|---|---|---|
| Connector hosts rows (`connectors/codex-cli/harness.toml:119-130`, data 12) | Opens `chatgpt.com` and `api.openai.com` for MITM | 1, 7 | Keep | Both sign-ins land here, WebSocket or HTTPS | 0 / 0 | `gov.chatgpt.com` and data-residency hosts not covered (rare) | none |
| `capture.ca` `CODEX_CA_CERTIFICATE` extend (`harness.toml:78-80`, data 3) | Lets Codex trust the proxy CA | 1, 3 | Keep | Codex adds it to native roots for HTTPS and WebSocket | 0 / 0 | None found | none |
| `NODE_USE_ENV_PROXY` row (`harness.toml:60-61`, data 2) | Node proxy switch | none for Codex | Cut | The wrapper makes no network calls | 2 data / 0 | Agent-run Node tools no longer get it; they still see `HTTPS_PROXY` and their traffic is not model traffic | Golden fixture lines in `crates/goalcup-cli/tests/fixtures/harness-golden/codex-cli*.json` (regenerate) |
| `[redaction] extra_patterns` 7 rows (`harness.toml:101-117`, data 17) | Strips plan, credits, rate limits, installation id, safety id, turn state | 4 claimed, none actually | Cut all seven (owner decision covers four; the other three are ids, not secrets) | Not secrets; they break byte equality and add work | 17 data / about 19 test refs in `crates/goalcup-daemon/tests/it/redaction.rs`, `capture_redaction_extra.rs`, `capture_harness_load.rs` | Participant privacy: plan type and an installation id upload. Say so in point 6 text. Keep the `json_field` mechanism: Amp's `wsToken` is a secret (`connectors/amp-cli/harness.toml:102`) | Codex rows in those three files |
| `model_host_from` reader (`crates/goalcup-cli/src/model_host.rs`, 1540 lines with inline tests) | Reads the gateway address from Codex's config | 1, 7 | Change | Add `openai_base_url` and `[model_providers.openai] base_url`; refuse plainly a built-in non-OpenAI provider with no table (bedrock, gcp, ollama, lmstudio) instead of silently tunnelling it | +30 to 60 est. / +tests | Over-refusing a working setup; system or managed layers still unseen (say so) | none |
| `-c` launch args (`harness.toml:45-49`) | Turns off update prompt, trusts workspace | 3, 7 | Keep, and add `--no-daemon` | `-c` already forces the embedded app server; `--no-daemon` makes that explicit so a shared daemon in the home cannot make model calls with another environment | +1 data / 0 | `--no-daemon` is unknown to 0.153.0; require 0.156.1 or later | none |
| `resume_args = ["resume","--last"]` (`harness.toml:52`) | `goalcup continue` | 2, 7 | Keep, measure | Skips subagents; filters by cwd, which matches `-C {workspace}` | 0 / 0 | Never measured logged in (`codex-cli.md:209`) | none |
| Native row (`harness.toml:87-93`) | Reads all rollouts since run start | 2, 5 | Keep | Children are separate rollouts in the same tree | 0 / 0 | Needs the raised file and stream budgets | none |
| Codex native decoder (`crates/goalcup-verifier/src/capture_analysis/native/codex.rs`, 443 src) | Interprets rollout JSONL | 6 | Keep, re-measure on 0.165/0.166 | Decoder notes say measured on 0.153.0 and 0.156.1 only (`codex.rs:3-5`) | 0 / 0 | Records added since 0.162 (resolved model per child, history copies) show as unknown | none |
| Facts sheet (`docs/harnesses/codex-cli.md`, 242 data) | Codex facts | 6, 7 | Change: rewrite to about 80 lines for the current release | Mixed versions, stale "unknown" rows, long history notes | about 160 data / 0 | Losing measurement history (it stays in git and `docs/research/results/`) | none |
| `HARNESS-SETUP.md` Codex parts (lines 77-122, 263-280, 408, 422, 442, 464-465) | Participant setup | 6, 7 | Change | API-key path is now known; add a line forbidding proxy or CA keys in `codex-cli/.env`; mention weekly plan limits | small | none | none |

### D2.8 Risks for a run that starts 7 Nov 2026 and lasts 168 hours

1. **Version drift.** Participants will run 0.166 or 0.167 (about one minor release a week).
   The ChatGPT WebSocket path through our proxy was last measured on 0.156.1, and the dialer code
   changed since (section 1). Action: one ChatGPT-login and one API-key run on the newest stable
   release before 6 Nov, with "ask for subagents" in the prompt. Tell participants to pin the
   version they test with and to turn off Homebrew auto-update for the week.
2. **Unseen model hosts.** `openai_base_url`, an `openai` table override without
   `model_provider`, or a built-in like `gcp` or `amazon-bedrock` send model calls through an
   opaque tunnel. The run then shows few or no model calls. Action: section 7 change to the reader.
3. **`$CODEX_HOME/.env` can override `HTTPS_PROXY`, `NO_PROXY`, `SSL_CERT_FILE`.** Model calls skip
   the recorder. Action: `goalcup start` refuses or warns when that file names a `*PROXY*` or
   `*CERT*` key (a few lines), or the doc forbids it and the cheat checklist names it.
4. **Shared app-server daemon.** If someone starts `codex app-server daemon` with Goal Cup's
   `CODEX_HOME`, a TUI without exclusions would let the daemon make the calls. Our `-c` args
   prevent it today; add `--no-daemon` so it does not rest on an accident.
5. **Subagent volume.** Six concurrent children, no total cap, and full-history forks by default
   since 0.164. Over a week this can be hundreds or thousands of rollout files, each starting with
   a copy of the parent history. Action: the owner's decision to raise or remove the 1024-file and
   32-stream limits applies here first. The 64 GiB server limit could be reached by a busy run
   (estimate, unverified: 1 to 2 MiB of copied history per child times 1000 children is 1 to
   2 GiB native, plus wire).
6. **Session-scoped HTTP fallback.** After one failed WebSocket turn (for example during a proxy
   restart), the session uses HTTPS with zstd bodies for the rest of the week. Recording still
   works; the server must decode zstd request bodies.
7. **Plan limits and sign-in.** A ChatGPT plan's weekly limit can stall a 168-hour run. Token
   refresh runs against `auth.openai.com` on its own; a participant who logs in elsewhere with a
   copied `auth.json` breaks the run's token (`refresh_token_reused`).
8. **Install method.** Standalone and Homebrew installs run the native binary directly (no npm
   wrapper). Termination by `native_pid` was measured with the wrapper. Action: one Linux and one
   macOS run with the standalone install.
9. **Managed settings.** Business or enterprise workspaces can push config layers (since 0.165,
   from custom config servers) that change provider, features or update checks. Goal Cup cannot
   see them. Say so in the participant text.

### D2.9 Blind spots

- I read source, I did not run Codex. Every "works through the proxy" claim for 0.165 is from
  source reading plus the 28 Oct gateway run, which used plain HTTPS to a gateway, not the
  ChatGPT WebSocket.
- `ab.chatgpt.com` purpose and whether it can turn features on remotely: unverified.
- The official docs pages carry no dates; I give the fetch date.
- Release dates are tag dates; the changelog sometimes shows the next UTC day.

## D3. Hermes Agent (Nous Research)

Date: 29 Oct 2026. Lane D. Read-only. No harness was run.

Sources used:
- The public repository, cloned blobless on 29 Oct 2026 into the scratchpad. Source facts are read at
  tag `v0.22.5` (commit `bf0203a794`, tag date 22 Oct 2026, GitHub release 22/23 Oct). `main` was at
  `a938f00f0f` (29 Oct 2026). Source links below point at the `v0.22.5` tag; line numbers are from
  that checkout.
- Vendor docs and release pages (links given per fact).
- This repo at `/home/user/goal-cup-proxy` (commit 4b226de2).

Words used: "source" = read in Hermes's code at v0.22.5, not run. "vendor docs" = Nous Research's
docs or release notes. "measured" = Goal Cup measured it (taken from this repo's docs, not re-run).
"unverified" = nobody has checked it.

### D3.1 Short answer

1. **The version participants get is 0.22.5, not 0.19.0.** Hermes's own installer installs the latest
   stable GitHub release (v0.22.5, 22 Oct 2026). PyPI still stops at 0.19.0 (20 Jul 2026). Our
   measured runs of 29 Oct used 0.19.0, so they do not show what a participant will run.
2. **`HERMES_CA_BUNDLE` stopped steering model calls in 0.21.6 (8 Oct 2026), on every OS, not only
   macOS.** Since then it is read only by the Nous Portal sign-in and token refresh, where it
   *replaces* trust. Our connector still sets it to the proxy CA alone, so on 0.21.6+ it can only
   break a Nous Portal sign-in refresh and never helps a model call. Cut it.
3. **The `recorder` provider entry with `ssl_ca_cert` is what makes recording work on 0.22.** Keep
   it. It covers only the one base URL in `model.base_url`. Since 0.21.6 Hermes trusts the OS store
   (`truststore`), so a model call to any *other* host (subagent route on another provider,
   auxiliary fallback to OpenRouter, mixture-of-agents, fallback providers) now **succeeds through
   the opaque tunnel and is not recorded**. On 0.19.0 such calls failed loudly. This is a silent gap
   for point 1.
4. **`delegate_task` on 0.22.5: not measured.** The repo says 0.19.0 failed twice on 29 Oct with an
   "internal executor error", but the same 0.19.0 ran two children on the Mac on 22 Oct. The exact
   error text is not kept anywhere in the repo, so I cannot match it to an upstream fix. Many
   delegation fixes landed after 0.19.0 (listed in section 5). Measure delegation on 0.22.5 before
   6 Nov.
5. **Two `configure` commands and the session-file row are dead on 0.21.1+.** On 0.22.5 the
   `compression.in_place false` command forces what Hermes calls its "legacy rotating-compaction
   path", for no gain. Cut all three if we stop supporting 0.19.0.
6. **Our long-run advice is now wrong.** `agent.max_turns` has defaulted to unlimited since 0.21.0.
   HARNESS-SETUP tells participants to set 2000, which *adds* a cap.
7. **The `state.db` copy at seal is our only source of subagent links on 0.21.1+**, but it is capped
   at 64 MiB by our own constant, and the database is shared by every session ever run in the Goal
   Cup Hermes home. A week-long run may pass that cap and record a gap instead.
8. **Release pace is very high.** Eight stable releases from 8 to 23 Oct, a canary tag every day,
   and about 13,600 commits on `main` in the last 30 days. v0.23.0 is announced but not dated.
   A new stable release before or during the run window is likely (unverified).

### D3.2 Versions, releases and cadence

| Fact | Value | Source |
|---|---|---|
| PyPI newest | 0.19.0, uploaded 20 Jul 2026 | https://pypi.org/pypi/hermes-agent/json (read 29 Oct 2026) |
| Tag scheme | Date tags (`v2026.7.20` = 0.19.0 ... `v2026.9.24` = 0.21.5), then SemVer tags from `v0.21.6` | `git ls-remote --tags https://github.com/NousResearch/hermes-agent` (29 Oct 2026); `pyproject.toml` at each tag |
| Stable releases, last 30 days | v0.21.6 (8 Oct), v0.21.7 (11/12 Oct), v0.22.0 (13/14 Oct), v0.22.1 (14 Oct), v0.22.2 (15 Oct), v0.22.3 (21 Oct), v0.22.4 (21 Oct), v0.22.5 (22/23 Oct) | tag dates from the clone; https://github.com/NousResearch/hermes-agent/releases |
| Canary | `v0.22.5+canary.<UTC stamp>` every day around 07:00 UTC, latest 29 Oct | same `ls-remote` |
| Commits on `main`, 29 Sep to 29 Oct | 13,587 (about 450 a day). 7,051 between v0.21.6 and v0.22.5. 41,201 between 0.19.0 and 0.22.5 | `git rev-list --count` on the clone |
| Next release | v0.22.5 notes say "full curated notes will ship with v0.23.0". No date given | https://github.com/NousResearch/hermes-agent/releases/tag/v0.22.5 (22 Oct 2026) |
| Version string in `pyproject.toml` | `0.0.0` from v0.21.6 on (version comes from the release tag) | `pyproject.toml` at `v0.21.6`, `v0.22.5` |
| Python | `>=3.11,<3.15`; managed installs use Python 3.14 from v0.22.0 | `pyproject.toml:15` at v0.22.5; https://github.com/NousResearch/hermes-agent/releases/tag/v0.22.0 (13 Oct 2026) |

**What the installer installs.** `curl -fsSL https://hermes-agent.nousresearch.com/install.sh | bash`
defaults to `--channel stable`, the latest published release, found through
`api.github.com/.../releases/latest`, then the `github.com/.../releases/latest` redirect, then
`git ls-remote` ([install.sh at v0.22.5, lines 26-94 and 654-672](https://github.com/NousResearch/hermes-agent/blob/v0.22.5/scripts/install.sh#L654-L672);
the live script fetched on 29 Oct 2026 has the same logic). `--commit SHA` pins an exact commit. With
a terminal, the installer also runs `hermes setup` and then `hermes gateway install --if-missing`,
which offers a background gateway service ([install.sh lines 1153-1170](https://github.com/NousResearch/hermes-agent/blob/v0.22.5/scripts/install.sh#L1153-L1170)).
That service serves the participant's own `~/.hermes`, not Goal Cup's home (see section 3.3).

### D3.3 How Hermes 0.22.5 reaches models

#### D3.3.1 Process model

- `hermes chat` is a thin client. Every model call runs in a **gateway** process, one per
  `HERMES_HOME`. The chat talks to it over a local WebSocket, opened with `proxy=None`, so the local
  link never goes through `HTTPS_PROXY`
  ([hermes_cli/gateway_client.py:298-305](https://github.com/NousResearch/hermes-agent/blob/v0.22.5/hermes_cli/gateway_client.py#L298-L305)).
- A gateway started by a chat gets the chat's whole environment (`dict(os.environ)`), so
  `HTTPS_PROXY` reaches it ([hermes_cli/gateway_runtime_start.py:35-57](https://github.com/NousResearch/hermes-agent/blob/v0.22.5/hermes_cli/gateway_runtime_start.py#L35-L57)).
  It strips only Hermes's own per-launch switches (`DAEMON_STRIPPED_ENV`, lines 18-31).
- The gateway finds its home through `<HERMES_HOME>/gateway.sock` or a pointer file, so a separate
  `HERMES_HOME` gets its own gateway
  ([hermes_cli/gateway_runtime_discovery.py:143-173](https://github.com/NousResearch/hermes-agent/blob/v0.22.5/hermes_cli/gateway_runtime_discovery.py#L143-L173)).
- A chat-started gateway exits by itself after 600 s with no client and no work. Background
  delegations, cron jobs and in-flight runs keep it up
  ([gateway/run_idle_exit.py:1-19, 32, 150-170](https://github.com/NousResearch/hermes-agent/blob/v0.22.5/gateway/run_idle_exit.py#L1-L19)).
- v0.22.0 made one gateway per machine serve all *named profiles* of the default home
  ([v0.22.0 notes](https://github.com/NousResearch/hermes-agent/releases/tag/v0.22.0), 13 Oct 2026;
  `gateway.multiplex` comment, [config_defaults.py:2159-2166](https://github.com/NousResearch/hermes-agent/blob/v0.22.5/hermes_cli/config_defaults.py#L2159-L2166)).
  Goal Cup's home is a separate root, not a named profile, and the team measured fresh per-home
  gateways on 0.22.5 (docs/harnesses/hermes-cli.md:116-129). How a multiplexed default gateway
  treats a second root: unverified.

#### D3.3.2 Endpoints, keys and sign-in

- About 40 built-in provider plugins (`plugins/model-providers/*` at v0.22.5). API-key rows carry a
  default base URL, key variables and a `<PROVIDER>_BASE_URL` override variable, for example DeepSeek
  `https://api.deepseek.com/v1`, `DEEPSEEK_API_KEY`, `DEEPSEEK_BASE_URL`; Vercel AI Gateway
  `https://ai-gateway.vercel.sh/v1`, `AI_GATEWAY_API_KEY`, `AI_GATEWAY_BASE_URL`; Anthropic
  `https://api.anthropic.com`, `ANTHROPIC_BASE_URL`
  ([hermes_cli/auth.py:172-250](https://github.com/NousResearch/hermes-agent/blob/v0.22.5/hermes_cli/auth.py#L172-L250)).
- Sign-in providers: Nous Portal (device code; portal `https://portal.nousresearch.com`, inference
  `https://inference-api.nousresearch.com/v1`), OpenAI Codex (`https://chatgpt.com/backend-api/codex`),
  xAI OAuth, Qwen OAuth, MiniMax OAuth
  ([hermes_cli/auth_constants.py:55-90](https://github.com/NousResearch/hermes-agent/blob/v0.22.5/hermes_cli/auth_constants.py#L55-L90)).
  v0.22.5 reworked sign-in into a plain device login
  ([v0.22.5 notes](https://github.com/NousResearch/hermes-agent/releases/tag/v0.22.5), 22 Oct 2026).
- External-process provider: `copilot-acp` runs GitHub's Copilot CLI as a child process, whose
  network stack is its own (auth.py:187). Not covered by any Hermes trust setting (source,
  unverified in practice).
- Native SDK paths that do not read a provider's `ssl_ca_cert`: Anthropic, Gemini native, Bedrock,
  Vertex, Codex runtime (no `verify`/`ssl_ca_cert` handling in `agent/anthropic_adapter.py`,
  `agent/gemini_native_adapter.py`, `agent/bedrock_adapter.py`, `agent/codex_runtime.py`; source).
- Other model-call routes that can leave `model.base_url`:
  - `delegation.model/provider/base_url/fallback_providers` and the new pluggable delegation
    backends (`delegation.backend`, `delegation.backends`)
    ([config_defaults.py:1342-1390](https://github.com/NousResearch/hermes-agent/blob/v0.22.5/hermes_cli/config_defaults.py#L1342-L1390);
    [delegation docs](https://hermes-agent.nousresearch.com/docs/user-guide/features/delegation), undated, read 29 Oct 2026);
  - `auxiliary.*` tasks; provider `auto` inherits the main model, but the auto chain may fall back to
    OpenRouter when `OPENROUTER_API_KEY` is set
    ([config_defaults.py:752-781](https://github.com/NousResearch/hermes-agent/blob/v0.22.5/hermes_cli/config_defaults.py#L752-L781));
  - the `moa` (mixture of agents) virtual provider (`agent/agent_runtime_helpers.py:2005-2013`).

#### D3.3.3 Proxy and certificate trust (the part that matters most)

| Mechanism | Hermes 0.19.0 to 0.21.5 | Hermes 0.21.6 and later (0.22.5 read) | Source |
|---|---|---|---|
| `HTTPS_PROXY` | honoured (measured) | honoured; one NO_PROXY matcher for the LLM transport | [agent/proxy_bypass.py:1-26](https://github.com/NousResearch/hermes-agent/blob/v0.22.5/agent/proxy_bypass.py#L1-L26) |
| Default trust | certifi and `HERMES_CA_BUNDLE` ladder | OS store through `truststore` (macOS Security framework, Windows CryptoAPI, Linux OpenSSL paths), patched into `ssl.SSLContext` process-wide | [agent/ssl_verify.py:1-20](https://github.com/NousResearch/hermes-agent/blob/v0.22.5/agent/ssl_verify.py#L1-L20); `truststore` first appears in `ssl_verify.py` at `v0.21.6` |
| `HERMES_CA_BUNDLE` | chat and auxiliary clients; replaces roots | **Nous Portal sign-in and token refresh only**, still *replaces* roots (`ssl.create_default_context(cafile=...)`) | [hermes_cli/auth_device_flow.py:235-257](https://github.com/NousResearch/hermes-agent/blob/v0.22.5/hermes_cli/auth_device_flow.py#L235-L257); called from `_resolve_nous_runtime_credentials` (`hermes_cli/auth_nous.py:1097-1121`) and `refresh_nous_oauth_from_state` (`auth_nous.py:816-836`); vendor docs: [security.md, "TLS certificate trust"](https://github.com/NousResearch/hermes-agent/blob/v0.22.5/website/docs/user-guide/security.md) ("Hermes' provider resolver no longer selects trust through `HERMES_CA_BUNDLE`") |
| Provider `ssl_ca_cert` | not used by the connector | replaces trust for the endpoint whose base URL matches a `providers:` or `custom_providers:` entry; covers main chat, auxiliary client, model metadata and catalog probes; a missing file falls back to the OS store with a warning | [ssl_verify.py:300-343](https://github.com/NousResearch/hermes-agent/blob/v0.22.5/agent/ssl_verify.py#L300-L343); [config_providers.py:446-472](https://github.com/NousResearch/hermes-agent/blob/v0.22.5/hermes_cli/config_providers.py#L446-L472); [agent_runtime_helpers.py:2015-2019](https://github.com/NousResearch/hermes-agent/blob/v0.22.5/agent/agent_runtime_helpers.py#L2015-L2019) |
| `SSL_CERT_FILE` | not used by us | added *on top of* the OS store for `platform_ssl_context()`. But `resolve_httpx_verify` returns a plain platform context when `SSL_CERT_FILE` is set, so for the httpx model client it is not loaded explicitly; whether it then works depends on the OS verifier (macOS: no; Linux OpenSSL: maybe). Unverified | [ssl_verify.py:119-131, 341-343](https://github.com/NousResearch/hermes-agent/blob/v0.22.5/agent/ssl_verify.py#L119-L131) |
| `REQUESTS_CA_BUNDLE`, `CURL_CA_BUNDLE` | | read by plain `requests` calls in some tools, not by provider clients | `ssl_verify.py:231-262`; security.md |
| `NODE_EXTRA_CA_CERTS` | | not read by Hermes core (Python). Used only for the optional iron-proxy egress sandbox | `website/docs/reference/environment-variables.md:311` at v0.22.5 |

Bypass checks (source, v0.22.5):
- **HTTP/2:** no `http2` in `agent/`, `hermes_cli/`, `tools/`, `gateway/`. Model calls are HTTP/1.1
  (measured on 0.19.0 and 0.22.5 traffic in this repo).
- **WebSockets:** only the local chat-to-gateway link (with `proxy=None`), browser tools and
  streaming TTS. No model call over WebSocket found in `agent/`.
- **Pinned certificates:** none found in `agent/ssl_verify.py`.
- **Other runtimes:** core is Python. The Ink TUI and the desktop app are Node and Electron, but on
  0.22 they are clients of the Python gateway, which makes the model calls. `uv` (Rust) is used for
  install only. Optional Go or Rust binaries (`iron-proxy`, Bitwarden `bws`) download only when
  their features are turned on (`config_defaults.py:2488-2614`). The external Copilot CLI is the one
  model path in another runtime.
- **Consequence on 0.21.6+:** any host other than the `recorder` URL is trusted through the OS
  store. Through Goal Cup's opaque tunnel it gets its real certificate, so the call **works and is
  not recorded**. On 0.19.0 the replace-mode `HERMES_CA_BUNDLE` made such calls fail. The
  "fails closed" reasoning in D41 and in `connectors/hermes-cli/harness.toml:88-92` no longer holds.

#### D3.3.4 Non-model hosts seen

`models.dev` (metadata), `api.github.com` and `github.com` (update check and release lookup),
`hermes-agent.nousresearch.com`, `portal.nousresearch.com` (measured in this repo's 0.22.5 sandbox
run, per docs/plans/2026-10-29-record-everything-phase2-notes.md:27-33). The `tirith` scanner and
its 32 MB GitHub download were **removed in v0.22.0**
([v0.22.0 notes](https://github.com/NousResearch/hermes-agent/releases/tag/v0.22.0), 13 Oct 2026).

### D3.4 Sessions, `state.db` and files

- **Session snapshot file:** removed in 0.21.1 (`v2026.9.7`), as our docs already say
  (docs/harnesses/hermes-cli.md:56). Not present in v0.22.5 config defaults.
- **`state.db`:** `<HERMES_HOME>/state.db`, SQLite, WAL mode, with `-wal` and `-shm`.
  **Schema version 32 at v0.22.5** (`hermes_state_common.py:289`), up from 22 on 0.19.0
  (docs/harnesses/hermes-cli.md:47). Tables include `sessions`, `messages`, `session_model_usage`,
  `async_delegations`, `gateway_routing`, `worker_executions`, `conversation_generations` and others
  (`CREATE TABLE` statements in `hermes_state*.py`).
- **Downgrade is refused.** A state.db upgraded by a newer Hermes makes an older Hermes abort some
  writes ("Downgrading is unsupported")
  ([hermes_state_common.py:785](https://github.com/NousResearch/hermes-agent/blob/v0.22.5/hermes_state_common.py#L785)).
  This matters because HARNESS-SETUP tells some participants to install 0.19.0 into the same
  Goal Cup home (docs/HARNESS-SETUP.md:202-207).
- **Other `state.db*` names Hermes may write:** `state.db.pre-update-emergency-*` backups (source).
  Our pattern `state.db*` (connectors/hermes-cli/harness.toml:118) would pick one up if an update ran
  during the run.
- **Compression:** `compression.in_place` defaults to `true`; `false` is "the legacy
  rotating-compaction path", which the default was made to avoid ("the session-rotation bug
  cluster") ([config_defaults.py:695-700](https://github.com/NousResearch/hermes-agent/blob/v0.22.5/hermes_cli/config_defaults.py#L695-L700)).
- **Session pruning:** `sessions.auto_prune` is `false` at v0.22.5, but while the key is missing,
  "every surface asks once the disk holding state.db runs low"
  ([config_defaults.py:2278-2288](https://github.com/NousResearch/hermes-agent/blob/v0.22.5/hermes_cli/config_defaults.py#L2278-L2288)).
  That is a question on screen when the disk is low. The v0.22.0 notes say auto-prune is on by
  default; the v0.22.5 code says off. The code is what runs.
- **Hermes's own redaction:** live delegation logs are redacted before they are written
  (`tools/delegation_live_log.py:71-80`). Whether `state.db` message rows are redacted: unverified
  (no redaction call found on the message write path in a quick read).

### D3.5 Subagents (`delegate_task`)

How it works on 0.22.5 (source and [vendor docs](https://hermes-agent.nousresearch.com/docs/user-guide/features/delegation), read 29 Oct 2026, undated):
- Children run **in the gateway process** as in-process agents, each with its own session row in
  `state.db` (`parent_session_id` is set; `tools/delegate_tool.py:303`, `:355`). Since 0.20.2 each
  child has its own `SessionDB` connection to the same file (commit `65e005d0e7`, 14 Aug 2026).
- In an interactive chat, **top-level calls run in the background**: the call returns at once, and
  results come back as a new message between turns. "/stop, /new, or process exit halts running
  subagents (whole tree)" ([tools/delegate_tool.py:696-713](https://github.com/NousResearch/hermes-agent/blob/v0.22.5/tools/delegate_tool.py#L696-L713)).
- Defaults: `max_concurrent_children: 10`, `max_spawn_depth: 1`, `max_iterations: 250`,
  `child_timeout_seconds: 0` (no wall-clock cap), `oneshot_max_children: 2` (only for `-q` runs),
  `subagent_auto_approve: false` (a dangerous command in a child is auto-denied unless approvals are
  off) ([config_defaults.py:1342-1404](https://github.com/NousResearch/hermes-agent/blob/v0.22.5/hermes_cli/config_defaults.py#L1342-L1404)).
- Files: live transcripts `<HERMES_HOME>/cache/delegation/live/<id>/task-<n>.log` plus
  `manifest.json`, pruned after 7 days (`LIVE_RETENTION_DAYS = 7`,
  `tools/delegation_live_log.py:25`); zero-call timeout logs
  `<HERMES_HOME>/logs/subagent-timeout-<session>-<ts>.log`; optional worktrees
  `<repo>/.worktrees/subagent-<id>` (vendor docs).
- v0.22.5 added pluggable delegation backends (route children by model map or profile), 22 Oct 2026
  ([v0.22.5 notes](https://github.com/NousResearch/hermes-agent/releases/tag/v0.22.5); about 40
  `fix(delegation)` commits on 22 Oct alone).

The 0.19.0 failure:
- docs/plans/2026-10-29-record-everything-progress.md:74 says `delegate_task` failed twice on
  0.19.0 "with an internal executor error". The exact message is not in the repo (searched all
  branches with `git log --all -G 'internal executor'`; only that table and the status doc
  mention it). So I cannot tie it to one upstream fix. Unverified.
- Against it: on 22 Oct the Mac cell ran 0.19.0 with **two `delegate_task` children on the wire**
  (docs/research/results/2026-10-22-local-test-day.md:40, :83). So 0.19.0 delegation is not broken
  everywhere. The 29 Oct failure may come from the sandbox, the model or the prompt.
- Upstream fixes after 0.19.0 that touch child execution (commit, first release): async batches
  isolated from parent interrupts (`3d30232eba`, 0.19.1, 28 Jul); dedicated child `SessionDB`
  after a closed-handle crash, "'NoneType' object has no attribute 'execute'" (`65e005d0e7`, 0.20.2,
  14 Aug); sequential tool executor made interruptible (`c257e9196b`, 0.20.3, 16 Aug); failed
  children reported as failed with a clean error (`5a134383fe`, 0.21.0, 30 Aug); iteration cap 50 to
  250 and concurrency 3 to 10 (14 and 15 Aug). Whether any of these is the 29 Oct error: unverified.
- Upstream issue found by search: [#12440](https://github.com/NousResearch/hermes-agent/issues/12440)
  (opened 19 Apr 2026, closed "not planned" 10 Jun 2026): child model config ignored, giving a
  context-window error. Different error; shown only so nobody mistakes it for ours.

**Conclusion:** no evidence that a newer release fixes the 29 Oct failure, and no evidence it is
still there. It has to be measured on 0.22.5 the way users run it (interactive chat, ask for
subagents in the prompt).

### D3.6 Auto-update

- **Hermes core does not update itself from the CLI.** `hermes update` is explicit. A passive check
  runs in a background thread at chat start (`hermes_cli/banner.py:274-286`) and is cached for a day;
  `updates.check: false` turns it off
  ([source_check.py:535](https://github.com/NousResearch/hermes-agent/blob/v0.22.5/hermes_cli/source_check.py#L535);
  [config_defaults.py:2359-2361](https://github.com/NousResearch/hermes-agent/blob/v0.22.5/hermes_cli/config_defaults.py#L2359-L2361)).
  v0.22.0: "Update checks run once a day" and source installs follow stable SemVer releases by
  default ([v0.22.0 notes](https://github.com/NousResearch/hermes-agent/releases/tag/v0.22.0)).
- **Plugins:** the gateway checks for plugin updates every 24 h (`plugins.auto_update_check_hours`,
  0 disables); applying is off by default (`plugins.auto_apply: false`)
  ([config_defaults.py:1738-1744](https://github.com/NousResearch/hermes-agent/blob/v0.22.5/hermes_cli/config_defaults.py#L1738-L1744)).
- **Desktop app and Docker `:latest`** update by their own channels (v0.22.5 notes;
  `hermes_cli/dashboard_procs.py:78` mentions desktop auto-update). Not measured.
- **To freeze a version for the run:** install with `--commit <sha>` (or keep the installed
  checkout and do not run `hermes update`), and set `updates.check: false`.

### D3.7 Other 0.22 behaviour that touches a 168-hour run

| Setting | 0.22.5 default | Effect on a long run | Source |
|---|---|---|---|
| `agent.max_turns` | `None` = unlimited (since 0.21.0; 90 on 0.19.0) | No step cap unless the participant sets one | [config_defaults.py:75-78](https://github.com/NousResearch/hermes-agent/blob/v0.22.5/hermes_cli/config_defaults.py#L75-L78); `hermes_cli/config.py` at `v2026.7.20` had 90 |
| `agent.auto_recovery_cycles` | 5 (waits 15/30/60/60/60 s) | A provider or proxy outage longer than about 4 minutes ends the turn; Hermes then waits for "keep going" | [config_defaults.py:135-139](https://github.com/NousResearch/hermes-agent/blob/v0.22.5/hermes_cli/config_defaults.py#L135-L139) |
| `approvals.mode` | `smart`, timeout 300 s (was 60 s); `off` still works | Our advice ("off") still applies | [config_defaults.py:1679-1697](https://github.com/NousResearch/hermes-agent/blob/v0.22.5/hermes_cli/config_defaults.py#L1679-L1697); `tools/approval.py:510` |
| `gateway.unmanaged_idle_exit_seconds` | 600 | Background work keeps the gateway up | `config_defaults.py:2115` |
| Tool names | `todo` to `todo_list`, `cronjob` to `cronjob_manage`, `process` to `process_manage`; old names still work | none for us | v0.22.0 notes |

### D3.8 Compare with this repo

#### D3.8.1 What our code, connector and docs assume

| Claim in repo | Where | Status at v0.22.5 |
|---|---|---|
| Capture recipe measured on 0.19.0; launch prep on 0.22.5 | connectors/hermes-cli/harness.toml:2; docs/harnesses/hermes-cli.md:3 ("Facts for version 0.19.0", last checked 16 Oct) | True but stale: participants get 0.22.5 (docs/HARNESS-SETUP.md:185) |
| `HERMES_CA_BUNDLE` "replaces the roots of Hermes's provider clients ... fails closed" | harness.toml:88-92 (`[capture.ca] env = ["HERMES_CA_BUNDLE"], mode = "replace"`) | **Wrong from 0.21.6 on.** Read only by Nous sign-in/refresh, where it breaks the refresh (section 3.3) |
| "Hermes 0.22 on macOS ignores `HERMES_CA_BUNDLE`" | docs/harnesses/hermes-cli.md:130; docs/DECISIONS.md:979 | **Too narrow.** Ignored for model calls on every OS since 0.21.6 |
| "Up to 0.21.0, `HERMES_CA_BUNDLE`" | docs/harnesses/hermes-cli.md:79 | Should read "up to 0.21.5" |
| No provider list because a listed unused host is only probed and an unlisted used host "would fail" | harness.toml:61-68 | **Wrong from 0.21.6 on.** An unlisted host now works and is not recorded |
| `configure`: `sessions.write_json_snapshots true`, `compression.in_place false` | harness.toml:54-57; docs/DECISIONS.md:559-573 | First is ignored on 0.21.1+. Second forces Hermes's legacy compaction path on 0.22.5 for no gain |
| Native row `session_*.json` | harness.toml:103-109 | Finds nothing on 0.21.1+ |
| Native row `state.db*`, `final_snapshot` | harness.toml:115-121; budget `NATIVE_FINISH_BYTES = 64 << 20` in crates/goalcup-daemon/src/runtime/capture/owner.rs:13 | Right idea; the 64 MiB cap is a cap of our own and the file is shared by every session in the home |
| `[launch_prep]` writes `providers.recorder` with `ssl_ca_cert`, stops the gateway before launch and after exit | harness.toml:145-152; crates/goalcup-cli/src/launch.rs:1076-1270 (195 src lines) | **Correct and needed** on 0.21.6+ (source agrees: `config_providers.py:446-472`) |
| `model_host_from = model.base_url`, one host | harness.toml:68 | Correct for the main route; misses delegation, auxiliary, MoA and fallback routes |
| `agent.max_turns` default 90; set 2000 | docs/HARNESS-SETUP.md:222-235, :392; docs/harnesses/hermes-cli.md:102 | **Stale.** Unlimited by default since 0.21.0; 2000 adds a cap |
| `tirith` downloaded from GitHub at start | docs/harnesses/hermes-cli.md:76, :103; docs/DECISIONS.md:573 | **Stale.** Removed in 0.22.0 |
| State store "schema version 22" | docs/harnesses/hermes-cli.md:47, :53 | Now 32 |
| Approval prompt timeout 60 s | docs/harnesses/hermes-cli.md:193 | Now 300 s |
| OAuth refresh fails under `HERMES_CA_BUNDLE` (known gap) | docs/harnesses/hermes-cli.md:106; TODO.md:338 | Still true on 0.22.5 for Nous Portal, *because we set it*. Codex and xAI OAuth no longer read it (only the Nous Portal sign-in, guest and token paths call `_resolve_verify`, which reads it) |
| Install 0.19.0 for Anthropic and other unsupported setups | docs/HARNESS-SETUP.md:202-207; README.md:44 | Downgrade into a home with a schema-32 `state.db` is unsupported by Hermes (section 4) |
| Delegation fails in 0.19.0 | progress.md:74, :78 | Contradicted by the 22 Oct Mac cell; error text not kept |
| PyPI newest is 0.19.0 | docs/harnesses/hermes-cli.md:36 | Still true on 29 Oct 2026 |
| `argv_guard`, `reserved_env_prefixes = ["HERMES_"]` | harness.toml:60, :124-128 | Measured on 0.19.0 only; not re-read here |

#### D3.8.2 Keep, cut, change

| Part | Points served | Verdict | Reason | What could go wrong |
|---|---|---|---|---|
| `[launch_prep]` `providers.recorder.base_url` and `ssl_ca_cert`, plus `recorder-ca.pem` (harness.toml:145-149) | 1, 7 | **Keep** | The only way 0.21.6+ trusts the proxy for the model host | A participant's own entry for the same URL wins (docs already say so) |
| `gateway stop -y` before launch and after exit (harness.toml:150-152) | 1, 3 | **Keep** | A stale gateway sends calls unrecorded; a live one holds the proxy open | After-exit stop also kills running background subagents. Correct at the end of a part, but a participant who exits Hermes to "pause" loses those children |
| `[capture.ca] HERMES_CA_BUNDLE` replace (harness.toml:88-92) | none on 0.21.6+ | **Cut** (if we drop 0.19.0 support) | Does not reach model clients; breaks Nous Portal token refresh | On 0.19.0 it is the only trust path; cutting it breaks 0.19.0 recording |
| `configure` both commands (harness.toml:54-57) | none on 0.21.1+ | **Cut** (if we drop 0.19.0) | First ignored, second changes how Hermes compacts (point 7) | 0.19.0 loses its session file |
| Native row `session_*.json` (harness.toml:103-109) | none on 0.21.1+ | **Cut** (if we drop 0.19.0) | Matches nothing | none on 0.22 |
| Native row `state.db*` final snapshot (harness.toml:115-121) | 2, 5 | **Keep, change** | Only record of child sessions, `parent_session_id` and `async_delegations` | Remove the 64 MiB cap for this row or raise it with a stated cost reason; consider an SQLite backup copy instead of three raw files; check that the redactor handles SQLite pages (a secret split across pages could pass, unverified); the copy includes every earlier session in the shared home (privacy) |
| `model_host_from` one key (harness.toml:68) | 1 | **Change** | On 0.21.6+ other model hosts succeed unrecorded | At minimum warn at start when `delegation.*`, `auxiliary.*.base_url`, `fallback_providers` or provider `moa` name another host. Full fix: one `recorder<N>` entry per host |
| Support for 0.19.0 (PyPI) | 7 | **Owner call; lean cut** | Participants get 0.22.5; keeping 0.19.0 keeps three dead-on-0.22 parts and the downgrade risk | Some setups (Anthropic native, Azure query URL) then have no recorded path at all |
| Docs: max_turns 2000, tirith, schema 22, 60 s, "macOS only", "up to 0.21.0" | 6 | **Change** | Wrong for 0.22.5 | Telling users to remove max_turns: a model that loops never stops, but nobody is typing anyway |
| Measured-run evidence on 0.19.0 | 7 | **Change** | Measure on 0.22.5 (and pin it) | |

### D3.9 Risks for a run starting 7 Nov 2026, lasting 168 hours

1. **Version drift (high).** Eight stable releases in 15 days and a daily canary. A participant who
   installs on 6 or 7 Nov may get 0.22.6 or 0.23.0, which we never ran. v0.23.0 is announced
   (v0.22.5 notes) but not dated. Mitigation: name and test one exact version, tell participants to
   install with `--commit <sha>`, set `updates.check: false`, and have `goalcup` print the Hermes
   version at start (it already reads `--version`).
2. **Silent unrecorded model calls (high, point 1).** Since 0.21.6 a model call to any host other
   than `model.base_url` works through the opaque tunnel and is not recorded. Triggers: a
   `delegation.provider` or a v0.22.5 delegation backend on another provider, auxiliary fallback to
   OpenRouter, `fallback_providers`, MoA, Copilot ACP, or a `<PROVIDER>_BASE_URL` in `.env`. On
   0.19.0 these failed loudly; now they are silent.
3. **Subagents not measured on the shipped version (high, point 2).** Background children run in
   the gateway and stop when the chat exits or the gateway is stopped. Their only record of
   parentage is `state.db`, copied once at seal.
4. **`state.db` over 64 MiB (medium, points 2 and 5).** A week of a busy agent with subagents, plus
   every earlier session in the shared home, may pass `NATIVE_FINISH_BYTES` (owner.rs:13), and the
   seal then writes a gap instead of the database. Size over 168 h: not measured.
5. **Nous Portal sign-in breaks mid-run (medium).** Our `HERMES_CA_BUNDLE` makes the token refresh
   to `portal.nousresearch.com` fail through the opaque tunnel once the first token expires.
6. **Outage longer than about 4 minutes ends the turn (medium, point 3).** `auto_recovery_cycles: 5`.
   An upstream outage, a long network cut or a slow proxy rebind leaves Hermes idle until someone
   types "keep going". Not a recording failure, but a week-long run will meet one.
7. **Wrong long-run advice (medium).** HARNESS-SETUP's `max_turns: 2000` caps an otherwise
   unlimited agent. Low-disk prune question (`sessions.auto_prune` unset) can appear on screen.
8. **Downgrade in the shared home (medium).** Following our "install 0.19.0 instead" advice after
   using 0.22.5 in the same Goal Cup home meets a schema-32 database that Hermes says it cannot
   downgrade.
9. **Background gateway service (low).** The installer offers `hermes gateway install`; it serves
   the participant's own home, not Goal Cup's (separate socket). A kanban or cron worker in
   Goal Cup's home could still start a gateway between our stop and the launch (already in
   docs/harnesses/hermes-cli.md:185-187).
10. **No 2-hour run report.** `docs/research/results/2026-10-30-two-hour-run.md` does not exist on
    any branch, so there is no long measurement of 0.22.5 with the current main.

### D3.10 Smallest set of actions before 6 Nov

1. Pick one Hermes release (today: v0.22.5, commit `bf0203a794`) and run one interactive chat on
   it that asks for subagents in the prompt. Record: calls recorded, children on the wire,
   `state.db` size at seal.
2. Cut `HERMES_CA_BUNDLE`, both `configure` commands and the `session_*.json` row, unless the owner
   keeps 0.19.0 support.
3. Lift or justify the 64 MiB finish cap for the `state.db` row.
4. Add a start-time warning (no refusal, point 3) when `config.yaml` names a model route on a host
   other than `model.base_url`.
5. Fix the docs listed in section 8.1 (max_turns, tirith, schema, CA wording, 0.19.0 advice).

## D4. The other harnesses, for awareness

Lane D. Scope: OpenCode (1 and 2), OpenClaw, Amp, Cursor CLI, Gemini CLI and its replacement
Antigravity CLI, GitHub Copilot CLI, Goose, Aider, Cline CLI, Kilo CLI, and a short look at Pi,
Factory Droid, Qwen Code, Crush and Kimi CLI. For each: how it reaches models, proxy and CA,
subagents, session files, updates, release pace, the last 30 days, and risk for a run that starts
7 Nov 2026 and lasts 168 hours. Then: what this repo assumes, what is stale, keep, cut, change.

### How this was checked (D4)

- Release dates: the npm registry (`registry.npmjs.org/<pkg>`, `time` field), PyPI JSON, and
  `git ls-remote` plus a one-commit fetch of tags for Goose. Fetched 29 Oct 2026. Weekly download
  counts: `api.npmjs.org/downloads/point/2026-10-21:2026-10-27/<pkg>` and pypistats.org.
- Source read without running: the Cursor CLI package `2026.10.22-27dcf5f` (linux x64 tarball from
  `downloads.cursor.com`, URL taken from `https://cursor.com/install`, last-modified 22 Oct 2026),
  and `strings` of the Amp linux x64 binary `0.0.1793271832-g7615fe` (npm, 29 Oct 2026). Both in
  scratch folders, never executed.
- Web: vendor docs, release pages and changelogs, each linked with its date. The `gh` API is
  blocked for these repos in this session, so GitHub release pages were read with a web fetch.
- Repo: `/home/user/goal-cup-proxy` at 4b226de2, plus `origin/opencode-openclaw` (bf15fe85, 28 Oct,
  not merged into main) and the site repo `/home/user/agentcommunity_page` for the public FAQ.
- No harness was run. No key or secret was read. (The missing 2-hour run report is noted once in
  the lane summary.)

### D4.1 Short answer

- **Only four harnesses are promised to participants.** The public FAQ says "The ones the proxy
  supports: Claude Code, Codex CLI, Hermes and Amp ... Bringing your own or another harness? Its
  proxy support must be added before 6 November, and you must complete a 1-hour run with it that
  proves recording works" (`/home/user/agentcommunity_page/features/goal-cup-campaign/content.ts:292`).
  The site's harness name list pins the same four
  (`/home/user/agentcommunity_page/features/goal-cup-proxy/GOAL-CUP-PROXY.md:56-57`). So the
  harnesses in this lane matter for Friday only if someone asks to bring one.
- **Amp is the only one here we ship.** Its connector is 12 days and about 218 builds behind
  (section 3.3). The update-check switch it relies on is still in the newest binary. A new
  unmeasured risk: Amp now lets users bring their own keys and subscriptions (30 Sep), and the
  binary carries OAuth URLs for two model vendors. If any of that inference leaves the local CLI,
  our connector does not open those hosts.
- **OpenCode and OpenClaw record wire-only today** (proof runs on 28 Oct, branch
  `origin/opencode-openclaw`). Both are big in the wild. But OpenCode 2 (`@opencode/cli` 2.2.1,
  29 Oct) runs a shared background service that clients attach to, and OpenClaw lets its own config
  override `HTTPS_PROXY`. Neither is safe to add before Friday without a fresh probe.
- **Cursor CLI cannot be recorded by this proxy.** It talks HTTP/2 bidirectional streams by default,
  and the server can force HTTP/2 even when the user sets the HTTP/1.1 option (read from the
  bundle). Our proxy is HTTP/1.1 only (`crates/goalcup-cli/src/proxy/tls.rs:26-30`).
- **Copilot CLI is the best next harness after Edition 01.** One append-only `events.jsonl` per
  session, with subagent events inside it, HTTP/1.1 by default, honours `HTTPS_PROXY`.
- **Gemini CLI is retired for individuals** since 18 Jun 2026. Its replacement, Antigravity CLI
  (`agy`), is a closed Go binary that self-updates with no documented off switch.
- **Aider is stale** (last release 12 Feb 2026). Goose, Cline, Kilo, Crush keep sessions in SQLite.
- **Cut and change in this repo:** cut the dead `harnesses/install.sh` (62 lines), cut harness-specific
  rules for harnesses we do not support (`docs/RULES.md:167-168`, `:216-225`), drop the "database
  session store cannot be captured" stop rule (`docs/HARNESS-ONBOARDING.md:188`) since the wire leg
  is the record, refresh the Amp facts, and decide now whether "bring another harness" stays open.

### D4.2 Ranking

Two different rankings. "Use at Goal Cup" is driven by the public FAQ (only four supported). "Use in
the wild" uses weekly npm or PyPI downloads for 21 to 27 Oct 2026 as a rough hint. Downloads are not
users: CI, auto-update and library use inflate them, and Amp mostly installs from its own script.

| Harness | Weekly downloads (21-27 Oct) | Likely use at Goal Cup | Hard to record (1 easy, 5 not possible today) | Why |
|---|---|---|---|---|
| Amp | 36,558 (`@ampcode/cli`) + 24,987 (`@sourcegraph/amp`) | Supported, medium | 3 | Wire is one WebSocket to `ampcode.com`; no local transcript; agent loop runs on Amp's server; BYOK routing unmeasured |
| OpenCode 1 / 2 | 3,501,455 (`opencode-ai`) + 591,938 (`@opencode/cli`) | Not supported; most likely "bring my own" ask | 3 (v1), 4 (v2) | v1 recorded wire-only on 28 Oct; v2 background service unmeasured |
| OpenClaw | 3,114,044 | Not supported; possible ask | 4 | Recorded wire-only on 28 Oct, but its own proxy setting wins over `HTTPS_PROXY`; Node script |
| GitHub Copilot CLI | 1,407,715 | Not supported | 2 | Append-only JSONL with subagents inside; HTTP/1.1 default; Node plus a Rust addon |
| Cursor CLI | not on npm | Not supported | 5 | HTTP/2 bidi, server can force it; SQLite `store.db` |
| Pi (`@earendil-works/pi-coding-agent`) | 4,929,894 | Not supported | 2 to 3 (unverified) | Node; JSONL sessions (unverified); count likely inflated by library use |
| Gemini CLI | 364,690 | Very low (retired for individuals) | 2 (unverified) | Node; JSON chat files; only paid API keys and enterprise still served |
| Antigravity CLI (`agy`) | not on npm | Low | 4 | Closed Go binary, self-updates, proxy support added mid-2026, CA rules depend on Go version |
| Cline CLI | 86,058 | Very low | 3 | Native per-platform binary; hub daemon mode over WebSocket; JSON files plus SQLite index |
| Qwen Code | 85,288 | Very low | 2 to 3 (unverified) | Gemini CLI fork; Node |
| Aider | 65,827 (PyPI) | Very low | 2 | Python; no subagents; but stale and writes history into the repo folder |
| Kilo CLI | 46,073 | Very low | 3 | OpenCode fork, SQLite |
| Goose | not on npm | Very low | 3 | Rust; SQLite `sessions.db`; CA handling unknown |
| Crush | 13,870 | Very low | 3 | Go; SQLite |
| Factory Droid | 8,408 | Very low | 3 (unverified) | Native binary; session format unverified |
| Kimi CLI | 19,213 (PyPI) | Very low | unverified | Python |

The October 2026 survey in commit 366d1797 (`docs/HARNESS-SCALING.md`, no longer on main) ranked
Claude Code, Copilot, Codex, Cursor, Antigravity, OpenCode, Hermes, Kilo, and others. That matches
the picture above, except that Cursor and Antigravity are close to impossible for this proxy.

### D4.3 Per harness

#### D4.3.1 OpenCode 1 and OpenCode 2

**Release pace and last 30 days.** OpenCode 1 (`opencode-ai`): 1.19.0 on 11 Oct, 1.19.1 on 15 Oct,
1.19.2 and 1.19.3 on 16 Oct, 1.19.4 on 22 Oct, 1.19.5 on 28 Oct (npm). OpenCode 2 (`@opencode/cli`):
2.0.0 on 11 Sep, 21 stable versions since 29 Sep, 2.2.0 on 27 Oct, 2.2.1 on 29 Oct (npm).
Changes that matter, from the [release page](https://github.com/anomalyco/opencode/releases)
(fetched 29 Oct):

| Release | Change | Why it matters |
|---|---|---|
| 1.19.0 (11 Oct) | Updates are manual now; "updates are no longer installed automatically"; `opencode upgrade --major` moves to 2.x | v1 no longer changes under a run by itself |
| 1.19.1 (15 Oct) | "Session sharing has been discontinued" | `share` is on our RULES ban list (`docs/RULES.md:219`); now moot |
| 1.19.2 (16 Oct) | v1 install script refuses 2.x | Matches the facts sheet |
| 1.19.4 (22 Oct) | Bundled Bun 1.4.4, root certificates NSS 3.131.1 | CA trust is Bun's own store plus `NODE_EXTRA_CA_CERTS` |
| 2.2.0 (27 Oct) | Session history loads from the latest compaction; experimental blob storage for session media | v2 storage still moving |
| 2.2.1 (29 Oct) | Responses API for more providers; plugin event stream follows each plugin server's endpoint | More wire shapes |

**OpenCode 2 is a different program.** It is installed with
`curl -fsSL https://opencode.ai/v2/install | bash` or `npm install -g @opencode/cli`
([v2 docs](https://opencode.ai/v2/docs), fetched 29 Oct). A third-party write-up (14 Oct 2026)
says: terminal, desktop and web clients "attach to one shared background service"; the service
registers at `~/.local/state/opencode/service.json`; sessions live in
`~/.local/share/opencode/opencode.db`; subagents can run with `background: true`; and the v2 binary
replaces v1 under the same `opencode` command
([capitalandcompute.net](https://capitalandcompute.net/blog/opencode-2-0-what-changed/)). The
service model is unverified by us. If true, a TUI started by `goalcup start` may attach to a service
started earlier with another environment, so model calls would not pass through the run's
`HTTPS_PROXY`. That is the Hermes gateway problem again (D72).

**Model hosts, proxy, CA** (facts sheet, v1.19.4, measured 28 Oct,
`git show origin/opencode-openclaw:docs/harnesses/opencode-cli.md`, section 6): Bun binary, honours
`HTTPS_PROXY`; `NODE_EXTRA_CA_CERTS` adds to roots (mode `extend`); `SSL_CERT_FILE` alone also
works. Gateway wire `POST https://ai-gateway.vercel.sh/v3/ai/language-model` (not a standard
format); DeepSeek `POST https://api.deepseek.com/chat/completions`. Also calls
`models.opencode.ai` and `registry.npmjs.org`. Keys in `$HOME/.local/share/opencode/auth.json`.

**Subagents.** v1: the task tool starts child sessions in the same SQLite file (facts sheet section 7
says subagents unmeasured). v2: background subagents (third-party, unverified).

**Session files.** One SQLite file for all sessions (measured, v1). No JSON. So no native leg
without a SQLite reader; the wire is the record.

**Auto-update.** v1: off by default since 1.19.0. Other switches: `OPENCODE_DISABLE_AUTOUPDATE`,
`"autoupdate": false` (facts sheet). v2: unverified.

**Risk for 7 Nov.** A participant on v1 is stable. A participant who typed `/update` or
`opencode upgrade --major` is on v2, which is unmeasured. Old beta docs named the binary `opencode2`;
the npm name `opencode2` is now a "security holding package" (npm, 13 Oct 2026), so someone may
have installed a bad package under that name. Not our bug, but a reason not to tell people to install
by name without checking.

#### D4.3.2 OpenClaw

**Release pace.** 16 stable npm versions since 29 Sep. Two lines: the main line (2026.10.4 on
15 Oct, 2026.10.5 on 18 Oct, 2026.10.6 on 22 Oct, 2026.10.7 on 24 Oct, 2026.10.8 on 27 Oct) and an
"extended-stable" line (2026.9.33 on 15 Oct, 2026.9.34 on 24 Oct, 2026.9.35 on 27 Oct) described as
"our current equivalent to LTS" ([releases](https://github.com/openclaw/openclaw/releases), fetched
29 Oct). Beta 2026.10.10-beta.4 on 27 Oct. Expect 2 to 3 releases during a 168-hour run.

**Last 30 days.** 2026.10.8: SQLite lock contention at start retried; session archiving through
symlinked state paths; the secret egress proxy rejects a distrusted CA chain. 2026.9.35: full-text
index repair after a SQLite tokenizer change. No proxy setting change is listed (same page).

**Model hosts, proxy, CA** (facts sheet v2026.10.8, measured 28 Oct,
`origin/opencode-openclaw:docs/harnesses/openclaw-cli.md`, section 6): `openclaw` is a
`#!/usr/bin/env node` script (npm `bin: openclaw.mjs`, engines `node >=24.22.1 <25 || >=26.12.1`).
Honours `HTTPS_PROXY` with `NODE_USE_ENV_PROXY=1`; `NODE_EXTRA_CA_CERTS` extends roots;
`SSL_CERT_FILE` ignored. **`OPENCLAW_PROXY_URL` and `proxy.proxyUrl` win over `HTTPS_PROXY`**
(measured). Gateway wire `POST https://ai-gateway.vercel.sh/v1/messages`; DeepSeek
`/chat/completions`. Daily update check to `telemetry.openclaw.ai`, off with
`OPENCLAW_NO_AUTO_UPDATE=1` or `update.checkOnStart: false`.

**Subagents.** `sessions_spawn` and `sessions_send`; 2026.10.8 fixes child results after session
reuse ([releases](https://github.com/openclaw/openclaw/releases)). Storage: same SQLite file.

**Session files.** `$HOME/.openclaw/agents/<agent>/agent/openclaw-agent.sqlite`, table
`transcript_events` (measured). Logs default to `/tmp/openclaw/`, shared by every `HOME`.

**Risk for 7 Nov.** The proxy override is a silent bypass. A connector would have to refuse a run
when either setting is present in `openclaw.json` or `$HOME/.openclaw/.env`. The `openclaw tui`
default mode connects to a Gateway daemon with its own environment (unmeasured).

#### D4.3.3 Amp (shipped connector)

**Release pace.** `@ampcode/cli`: 218 npm builds between 29 Sep and 29 Oct, about 7 a day; newest
`0.0.1793271832-g7615fe` on 29 Oct (npm). `https://static.ampcode.com/cli/cli-version.txt` already
returned `0.0.1793274046-g0b6b2b` on 29 Oct. Expect about 50 builds during a 168-hour run.

**Last 30 days** ([Amp news](https://ampcode.com/news), fetched 29 Oct): self-hosted orbs (22 Oct,
28 Oct), macOS Satellites (29 Oct), runners: "The Mac App Is Your Runner" and "Shared Runners"
(24 Sep), "One Runner, Many Worktrees" (22 Sep). The
[Free Agent post](https://ampcode.com/news/free-agent) (13 Sep, updated 30 Sep): Amp is "free to use
when you bring your own compute and model subscriptions/keys"; it says users can use their existing model-vendor
subscriptions; BYOK providers include OpenRouter, Bedrock, Vercel AI Gateway, Cloudflare AI Gateway and
"custom endpoint URLs", set under Model Routing. The post does not say where that inference runs.

**What the newest binary shows** (strings of `0.0.1793271832-g7615fe`, linux x64, 161,605,808
bytes):

- `AMP_SKIP_UPDATE_CHECK` is still present (3 hits). `static.ampcode.com/cli/cli-version.txt` is
  still the update URL. So the connector's switch (`connectors/amp-cli/harness.toml:45`) still has a
  target. Whether it still works: unmeasured on this build.
- Runtime string `Bun v1.4.4`. The facts sheet says Bun 1.4.1 (`docs/harnesses/amp-cli.md:79`).
- New: `https://auth.openai.com/oauth/authorize`, `https://auth.openai.com/oauth/token`,
  `https://api.anthropic.com/api/oauth/claude_cli/create_api_key`, `AMP_EXTERNAL_AGENT_RUNTIME`,
  `AMP_USE_NATIVE_WEBSOCKET`, `model_provider_chatgpt`, `/settings/model-routing`. These suggest the
  local CLI can sign in to model vendors. Whether it then calls them directly: unverified.

**Model hosts, proxy, CA** (facts sheet, measured 15 to 17 Oct): one WebSocket of JSON-RPC to
`ampcode.com/actors/gateway/threadActor/`, HTTP/1.1, through `HTTPS_PROXY`, CA via
`NODE_EXTRA_CA_CERTS` (mode `extend`) (`docs/harnesses/amp-cli.md` section 6). Inference runs on
Amp's server, so the wire shows thread messages, not provider requests.

**Subagents.** `Task`, `oracle`, `librarian`, `finder` exist (`docs/harnesses/amp-cli.md:144`). The
19 Oct decoder sees a subagent's snapshots with the parent `Task` call as `parent_tool_use_id`
(`docs/harnesses/amp-cli.md`, "Stored-byte replay"). So subagents are recorded as server snapshots
on the same WebSocket, not as separate local files.

**Session files.** None with content. Per-thread metadata log `$HOME/.cache/amp/logs/threads/T-*.log`
(`connectors/amp-cli/harness.toml:84-90`).

**Auto-update.** `AMP_SKIP_UPDATE_CHECK=1` (connector). `amp update` and the brew tap exist. Runners
have `AMP_RUNNER_EXIT_FOR_UPDATE` and `AMP_RUNNER_RELAUNCH_FROM_VERSION` (strings), so runner mode
restarts itself for updates.

**Risk for 7 Nov.** (a) BYOK or subscription routing may move inference to the local CLI and to
hosts the connector does not open (`connectors/amp-cli/harness.toml:111-115` opens only
`ampcode.com`). Those calls would pass as sealed tunnels and be missed by point 1. (b) The thread
protocol lives on Amp's server and can change during the week with no client update; the decoder
(version 3) may then read less, but raw bytes are still captured. (c) Runners, the Mac app runner
and shared runners can execute a thread outside `goalcup`'s process tree; RULES names "Amp orbs and
Amp's remote executors" (`docs/RULES.md:387`) but not runners by name.

#### D4.3.4 Cursor CLI (`cursor-agent`, `agent`)

**Release pace.** Builds named by date: 2026.10.22, 2026.10.17, 2026.10.15, 2026.10.08,
2026.09.28, 2026.09.22, 2026.09.15 ([CLI changelog](https://cursor.com/docs/cli/changelog), fetched
29 Oct). About one a week.

**Last 30 days** (same changelog): 2026.10.22 custom subagents fall back to the parent model;
2026.10.17 post-tool hooks fire for subagents, and "subagent transcripts" move to
`agent-transcripts/<parent>/subagents/`; 2026.10.15 Grep and Glob can search outside the workspace;
2026.10.08 new Bedrock models.

**Install and runtime** (read from `https://cursor.com/install`, 29 Oct): downloads
`https://downloads.cursor.com/lab/2026.10.22-27dcf5f/<os>/<arch>/agent-cli-package.tar.gz` into
`~/.local/share/cursor-agent/versions/<build>/` and links `~/.local/bin/agent` and
`~/.local/bin/cursor-agent`. The entry point `cursor-agent` is a bash script that runs a bundled
`node` with `--use-system-ca`, unless `AGENT_CLI_CREDENTIAL_STORE=file` (read from the package).

**Model hosts.** `api2.cursor.sh` is the backend; the agent URL itself comes from server config
(`agentUrl`, `agentnUrl`), so it is not fixed (read from the bundle). Inference runs on Cursor's
backend; the CLI never calls a provider directly in normal use.

**Proxy, CA, HTTP/2.** Docs: set `HTTPS_PROXY`, `NODE_USE_ENV_PROXY=1`, `NODE_EXTRA_CA_CERTS`; set
`"network": {"useHttp1ForAgent": true}` because some proxies "don't support HTTP/2 bidirectional
streaming" ([CLI configuration](https://cursor.com/docs/cli/reference/configuration), fetched 29 Oct,
no page date). The bundle shows the transport is decided by server config first:
`server_force_all_enabled` gives `useHttp1: false` even when the local setting is `true`; only the
default case uses the local setting (function `lt`, read from the package). So Cursor can switch to
HTTP/2 under us mid-run. Our proxy refuses `h2`-only clients (`crates/goalcup-cli/src/proxy/tls.rs:26-30`,
`crates/goalcup-cli/src/proxy/connect.rs:624-637`).

**Subagents and sessions.** Chat store `store.db` (SQLite, content-addressed blobs, some protobuf)
and a lossy `agent-transcripts` JSONL; the subagent `meta` row carries
`subagentInfo.{parentAgentId, rootParentAgentId, toolCallId, typeName}`
(`spikes/cursor-cli/fixtures/README.md:15-25`, spike of 3 Sep 2026). Config
`~/.cursor/cli-config.json`, `CURSOR_CONFIG_DIR`, `CURSOR_DATA_DIR`.

**Auto-update.** A background update check (changelog 2026.10.22); a hidden `--disable-auto-update`
flag and `AGENT_CLI_UPDATE_CHECK_URL` exist (bundle). The docs list only `channel`.

**Verdict.** Not recordable by an HTTP/1.1 proxy with any confidence. Keep it out.

#### D4.3.5 Gemini CLI and Antigravity CLI

**Gemini CLI.** Google's post of 19 May 2026: on 18 Jun 2026 Gemini CLI stopped serving Google AI
Pro, Ultra and free individual users; it remains available through paid API keys and for
enterprise licences
([Google Developers Blog](https://developers.googleblog.com/an-important-update-transitioning-gemini-cli-to-antigravity-cli/)).
It still ships weekly: 0.63.0 (6 Oct), 0.64.0 (13 Oct), 0.65.0 (20 Oct), 0.66.0 (27 Oct) (npm).
0.66.0 verifies the active binary after an update
([releases](https://github.com/google-gemini/gemini-cli/releases), fetched 29 Oct). Node package
(`bundle/gemini.js`). Sessions in `~/.gemini/tmp/<project_hash>/chats/`, deleted after 30 days by
default ([session docs](https://geminicli.com/docs/cli/session-management/), updated 18 Jun 2026).
Subagents: `experimental.enableAgents`, default `true`. Auto-update: `general.enableAutoUpdate`,
default `true`. Base URL overrides `GOOGLE_GEMINI_BASE_URL`, `GOOGLE_VERTEX_BASE_URL`
([configuration](https://geminicli.com/docs/reference/configuration/), no date). Proxy and CA:
Node, so `HTTPS_PROXY` and `NODE_EXTRA_CA_CERTS` are expected to work (unverified for 0.66.0). Our
own TODO already notes Hermes with a Gemini provider is untested (`TODO.md:214`).

**Antigravity CLI (`agy`).** Built in Go, shares a harness with the Antigravity desktop app, keeps
skills, hooks and subagents (Google post above). Install
`curl -fsSL https://antigravity.google/cli/install.sh | bash`; settings
`~/.gemini/antigravity-cli/settings.json`; Google sign-in through the OS keyring, or
`GEMINI_API_KEY` with `"modelProvider": "gemini"`; `GOOGLE_GEMINI_BASE_URL`; "The binary
self-updates in the background" with no documented off switch
([computingforgeeks](https://computingforgeeks.com/antigravity-cli-cheat-sheet/), updated 8 Oct
2026, tested on 1.2.4). Proxy: a user reported on 20 May that `HTTPS_PROXY` did nothing, then on
26 Jun that "the official agy now supports proxying through environment variables"
([V2EX](https://www.v2ex.com/t/1214168)). CA: Go reads `SSL_CERT_FILE` on Linux; on macOS and
Windows, setting it stops the system verifier from being used only from Go 1.27
([crypto/x509](https://pkg.go.dev/crypto/x509#SystemCertPool)). Which Go version `agy` is built with:
unknown. Session storage: unknown.

#### D4.3.6 GitHub Copilot CLI

**Release pace.** 15 npm versions since 29 Sep; 1.0.100 (19 Oct) to 1.0.105 (28 Oct) (npm;
[changelog](https://github.com/github/copilot-cli/blob/main/changelog.md), raw file fetched 29 Oct).
Almost daily.

**Runtime.** The npm package now ships per-platform native packages (`@github/copilot-linux-x64`
and others) with a `npm-loader.js` entry (npm). Changelog: command-line parsing "moved from
Commander to a Rust grammar" (1.0.83, 4 Sep), "Native binary crash ... now falls through to the
JavaScript fallback" (1.0.55, 28 May), WebSocket transport "outside the Tokio runtime" (1.0.62,
13 Jun), and V8 heap notes in Oct. So it is Node with a native Rust part. Which layer opens the model
connection: unverified.

**Model hosts.** `*.githubcopilot.com` and `*.individual.githubcopilot.com`
([allowlist reference](https://docs.github.com/en/enterprise-cloud@latest/copilot/reference/copilot-allowlist-reference)).
Sign-in through GitHub. BYOK providers in `~/.copilot/providers.json` ("Contains plain-text
credentials") ([config dir reference](https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-config-dir-reference),
no date). WebSocket Responses for BYOK OpenAI-compatible providers since 1.0.64 (23 Jun).

**Proxy, CA, HTTP/2.** `HTTPS_PROXY` honoured "regardless of Node version" (0.0.x line); "Default
networking transport is now HTTP/1.1 ... Opt into HTTP/2 with COPILOT_ENABLE_HTTP2=1" (1.0.57,
1 Jun); "Load CA certificates from system and environment variables" (0.0.370, 18 Dec 2025)
(changelog). Coder's guide uses `HTTPS_PROXY` plus `NODE_EXTRA_CA_CERTS`
([Coder docs](https://coder.com/docs/ai-coder/ai-gateway/clients/copilot)).

**Subagents and sessions.** `~/.copilot/session-state/<session-id>/events.jsonl` plus plans,
checkpoints and tracked files; `session-store.db` is an index; `COPILOT_HOME` moves the whole tree
(config dir reference above). Subagent events sit in the same log with an `agentId` field
(community readers, unverified against an official schema). "By default, locally-run sessions ...
are synced to your GitHub account"
([session data](https://docs.github.com/en/copilot/concepts/agents/copilot-cli/chronicle)).

**Auto-update.** `autoUpdate` (default `true`) "Downloads CLI updates ... at the start of each
session"; `--no-auto-update` "runs the version bundled in the binary" (1.0.85); cache in
`~/.cache/copilot`, moved by `COPILOT_CACHE_HOME`.

**Verdict.** The best fit of this lane for a later connector: an append-only JSONL native leg with
subagents inside, a home variable, HTTP/1.1 by default. Not for Friday.

#### D4.3.7 Goose

Repository moved to `aaif-goose/goose` (`git ls-remote` on the old `block/goose` URL returns the
same tags). Tags: v1.53.0 (30 Sep), v1.55.0 (16 Oct), v1.56.0 (22 Oct), v1.57.0 (27 Oct). About
weekly. Last 30 days ([releases](https://github.com/aaif-goose/goose/releases), fetched 29 Oct):
background subagents and the orchestrator removed (1.55.0); concurrent subagents capped by
`GOOSE_MAX_CONCURRENT_SUBAGENTS` (1.56.0); a declarative AI gateway provider (1.55.0); callers can
supply their own session ids (1.57.0). Sessions: SQLite
`~/.local/share/goose/sessions/sessions.db` since 1.10.0
([session docs](https://goose-docs.pages.dev/docs/guides/sessions/session-management), no date).
Rust binary. Proxy: "standard environment variables and system proxy settings" (docs diff in
[PR 7311](https://github.meowingcats01.workers.dev/aaif-goose/goose/pull/7311/files)); CA: unknown. CLI
update is `goose update` (manual); desktop auto-updates.

#### D4.3.8 Aider

Last releases 0.86.0 (9 Aug 2025), 0.86.1 (13 Aug 2025), 0.86.2 (12 Feb 2026) (PyPI). Still
65,827 downloads in the last week (pypistats). Python, LiteLLM. No subagents. History files in the
working folder: `.aider.chat.history.md`, `.aider.input.history`, and `--llm-history-file`
(off by default); `--check-update` on by default; `--verify-ssl` on by default
([options](https://aider.chat/docs/config/options.html), no date). Risk: the history file sits in
the repo folder the agent pushes. Low use; skip.

#### D4.3.9 Cline CLI and Kilo CLI

**Cline CLI** (`cline`): 3.0.67 (30 Sep) to 3.0.74 (26 Oct), 9 versions in 30 days (npm). The npm
package now pulls native per-platform packages (`@cline/cli-linux-x64` and others). Sessions under
`~/.cline/data/sessions/<id>/`, not the VS Code `tasks/` layout
([codeburn issue 874](https://github.com/getagentseal/codeburn/issues/874)); the SDK stores
"session manifests and messages as files" with a manifest JSON and a messages JSON
([SDK sessions](https://docs.cline.bot/sdk/sessions)); a hub daemon with clients attached over
WebSocket ([fast.io guide](https://www.fast.io/resources/cline-architecture-guide.md)). `--data-dir`
gives isolated state ([CLI overview](https://docs.cline.bot/cli/overview)). Proxy, CA, auto-update:
not documented on those pages.

**Kilo CLI** (`@kilocode/cli`, binary `kilo`): an OpenCode fork, "supports the same configuration
options" ([Kilo CLI docs](https://kilo.ai/docs/code-with-ai/platforms/cli), no date). 7.8.16 (22 Oct)
to 7.8.25 (28 Oct); four builds on 28 Oct alone, while the `latest` tag still says 7.8.18 (npm).
SQLite like OpenCode; `KILO_DATA_HOME` moves data (third-party). Same recording shape as OpenCode 1.

#### D4.3.10 Others, one line each

| Harness | Facts (source, date) | Fit |
|---|---|---|
| Pi (`@earendil-works/pi-coding-agent`, `pi`) | 1.0.1 (3 Oct) to 1.4.0 (21 Oct), Node `>=22.19.0` (npm). The old `@mariozechner/pi-coding-agent` stopped at 0.73.1 (7 May) | Likely JSONL sessions (unverified) |
| Factory Droid (`droid`) | 0.244.0 (17 Oct) to 0.251.0 (27 Oct), native per-platform packages (npm); `disableAutoUpdate` setting ([CLI reference](https://docs.factory.ai/droid-cli/cli-reference)) | Session format unverified |
| Qwen Code | 0.26.0 (21 Oct) to 0.26.3 (28 Oct) (npm); Gemini CLI fork; optional Bun packages | Node; unverified |
| Crush (`@charmland/crush`) | 0.98.0 (8 Oct), 0.98.2 (23 Oct) (npm); Go | Go CA rules as for `agy` |
| Kimi CLI | 1.52.0 (22 Sep), nothing since (PyPI) | Quiet |
| Augment `auggie` | 0.36.0 (21 Aug), nothing since (npm) | Quiet |

### D4.4 Proxy and CA, across all of them

| Runtime | Harnesses | Honours `HTTPS_PROXY` | Custom CA | Bypass risk |
|---|---|---|---|---|
| Bun single binary | OpenCode, Amp, Kilo (likely) | Yes, on its own (measured for OpenCode and Amp) | `NODE_EXTRA_CA_CERTS` extends; `SSL_CERT_FILE` also works for OpenCode | Extend mode: a direct connection would still verify (`connectors/amp-cli/harness.toml:65-70`) |
| Node | OpenClaw, Gemini CLI, Qwen, Pi, Cursor (bundled node) | With `NODE_USE_ENV_PROXY=1` or own agent code | `NODE_EXTRA_CA_CERTS`; Cursor adds `--use-system-ca` | OpenClaw's own proxy setting wins; Cursor HTTP/2 |
| Node plus Rust addon | Copilot CLI | Yes (changelog) | System plus env (changelog) | Which layer dials: unverified |
| Go | Antigravity, Crush | Go's default transport reads it; `agy` only since mid-2026 | `SSL_CERT_FILE` on Linux; on macOS only from Go 1.27 | Version-dependent |
| Rust | Goose (and Codex, see D2) | Docs say yes for Goose | Unknown | rustls with bundled roots would refuse our CA (unverified) |
| Python | Aider, Kimi | httpx reads it | `SSL_CERT_FILE` or `REQUESTS_CA_BUNDLE` (general knowledge, unverified here) | LiteLLM uses its own bundle (unverified) |

Our proxy speaks HTTP/1.1 and WebSocket upgrades only
(`docs/HARNESS-ONBOARDING.md:189`, `crates/goalcup-cli/src/proxy/tls.rs:26-30`). Cursor is the only
harness in this lane that needs HTTP/2; Copilot made HTTP/2 opt-in.

### D4.5 What the repo assumes, and what is stale or wrong

| Path | What it assumes | Status |
|---|---|---|
| `README.md:4` | Four harnesses: Codex CLI, Claude Code, Hermes or Amp | Matches the FAQ. Keep |
| `README.md:137-138` | "The chat file ... is a backup of the wire record; a harness may have none" | Correct, and it contradicts the next row |
| `docs/HARNESS-ONBOARDING.md:188` | A live SQLite store means "Cannot be captured", stop at step 0 | Stale under "record everything": OpenCode and OpenClaw recorded wire-only on 28 Oct (`origin/opencode-openclaw:docs/research/results/2026-10-opencode-openclaw.md`). The SQLite store blocks only the native leg |
| `docs/HARNESS-ONBOARDING.md:189` | HTTP/2-only or gRPC traffic cannot be recorded | Correct; Cursor is the case |
| `docs/RULES.md:167-168` | "OpenCode and Cursor read the skills in your home directory" | Rules text about harnesses we do not support |
| `docs/RULES.md:216-225` | Banned flags for OpenCode, OpenClaw and Cursor | Same. OpenCode `share` is gone since 1.19.1 (15 Oct). Cursor `AGENT_CLI_CREDENTIAL_STORE=file` still exists in the 22 Oct launcher |
| `docs/RULES.md:387` | Cloud modes not eligible: "Amp orbs and Amp's remote executors" | Missing Amp runners, the Mac app runner and self-hosted orbs (Sep and Oct news) |
| `connectors/amp-cli/harness.toml:1` | Version `0.0.1792209635-g24a9fe` | About 218 builds old. Builds are recorded, not pinned, so only the comment is stale |
| `connectors/amp-cli/harness.toml:42-45` | `AMP_SKIP_UPDATE_CHECK=1` stops the version check | String still in the 29 Oct binary. Keep; re-measure once |
| `connectors/amp-cli/harness.toml:111-115` | `ampcode.com` is Amp's only model host | Unverified since BYOK and subscription routing (30 Sep). Probe needed |
| `docs/harnesses/amp-cli.md:3`, `:79` | Facts for Oct 11 and 17 builds; Bun 1.4.1 | Bun is 1.4.4 in the newest build; last checked 19 Oct |
| `docs/HARNESS-SETUP.md:413-426` | Amp `Task` subagents use a larger model than the mode | Measured 15 Oct; Amp's Dial (10 Sep) lets users pick models per mode, so it may differ now (unverified) |
| `harnesses/install.sh:15-57` | Installs `opencode-ai`, `@deepseek-ai/dsh`, `openclaw`; sets `XDG_DATA_HOME`; cites `docs/harness-research/*.md` | Dead: that folder does not exist; capture refuses `XDG_*`; OpenClaw needs Node 24.22.1+ |
| `docs/plans/2026-10-28-one-capture-system.md:43` | "Do not touch `connectors/opencode*` or `connectors/openclaw*` (another session owns them)" | No such connectors on main; branch not merged |
| `spikes/cursor-cli`, `spikes/openclaw`, `spikes/opencode` (1,681, 7,887 and 1,938 lines of fixture data) | Early-September fixtures for harnesses we do not ship | Only used by one over-redaction test that walks every spike fixture (`crates/goalcup-daemon/tests/it/redaction.rs:608-640`) |
| `/home/user/agentcommunity_page/features/goal-cup-campaign/content.ts:292` | Other harnesses may be added before 6 Nov after a 1-hour proof run | An open door with one week left |

### D4.6 Keep, cut, change

| Part (path, lines) | What it does | Points | Verdict | Reason | Lines saved (src / tests) | Risk |
|---|---|---|---|---|---|---|
| Scope of four harnesses (`README.md:4`, site FAQ) | Promise to participants | 7 | Keep, and close the "bring another harness" door now | Nothing else is measured end to end; a 1-hour proof in the last week adds a connector with no long-run data | 0 / 0 | A participant who planned on OpenCode or OpenClaw is turned away. Say it now, not on 6 Nov |
| `connectors/amp-cli/harness.toml` (115, data) | Amp capture | 1, 2, 3, 7 | Keep; change: one probe of BYOK routing on the newest build, and add the hosts it calls if any | Point 1 may miss local inference | 0 / 0 | If BYOK calls go to many hosts, the host list grows; a model-host-from rule may be needed |
| `docs/harnesses/amp-cli.md` (231, doc) | Amp facts | 6, 7 | Change: refresh version, Bun version, BYOK, runners | Stale since 19 Oct | 0 / 0 | None |
| `docs/RULES.md:216-225` and `:167-168` (about 12 lines) | Flag bans for unsupported harnesses | none for Friday | Cut the OpenCode, OpenClaw and Cursor parts; keep one line: only listed harnesses | Rules for harnesses no one may use add noise | about 8 doc lines | If a harness is added later, its bans must come back with its connector |
| `docs/RULES.md:387` | Cloud modes banned | 3, 7 | Change: add "Amp runners on another machine" by name | New Amp features since 17 Sep | +1 line | Over-banning a runner on the same machine; word it as "another machine" |
| `docs/HARNESS-ONBOARDING.md:188` | SQLite store is a stop rule | 7 | Change: say the wire is the record and the session file is optional | Matches README:137-138 and owner decisions | 0 / 0 | People may read it as "SQLite harnesses supported now"; they are not supported until a connector ships |
| `harnesses/install.sh` (62, script) | Installs old spike harnesses | none | Cut | Dead references, refused env vars | 62 / 0 | None found; no code or test references it |
| `spikes/cursor-cli`, `spikes/openclaw` (fixtures) | Spike data | 4 (via one test) | Cut, or leave to the lane that owns spikes | Only feeds `redaction.rs:608-640`; cutting means narrowing that test's input | 9,568 data lines / test input shrinks | The over-redaction test loses coverage of those shapes; keep `spikes/opencode` if its known catch is kept (`redaction.rs:625`) |
| HTTP/1.1-only proxy (`crates/goalcup-cli/src/proxy/tls.rs:26-30`) | Refuses `h2` | 1, 3 | Keep; do not build HTTP/2 | Only Cursor needs it, and Cursor's server can force it anyway | 0 / 0 | Cursor users cannot join; stated in the FAQ already |
| OpenCode and OpenClaw connectors (branch only) | Wire-only capture | 1, 7 | Do not add before Friday | v2 background service and OpenClaw proxy override are unmeasured bypasses | 0 / 0 | Lost entrants who use them |

### D4.7 Risks for a run that starts 7 Nov 2026 and lasts 168 hours

1. **Amp ships about 7 builds a day.** About 50 builds land during the run. The connector stops the
   CLI's own check, but the agent can run `npm install` or `amp update` itself. Builds are recorded,
   not refused, so this is a flag, not a failure (`docs/RULES.md` "Do not update the harness").
2. **Amp's server-side protocol can change mid-run.** The client stays the same, the thread
   messages change. Raw bytes are kept (point 5), the decoder may read less (point 6).
3. **Amp BYOK or subscription routing may call providers locally.** Unmeasured. If so, point 1 fails
   silently for those calls: they show up as sealed tunnels with bytes and times only.
4. **Amp runners.** A participant with the Mac app open may have a runner that picks up the thread
   on the same Mac but outside `goalcup`'s process tree and environment. Unmeasured.
5. **If OpenCode or OpenClaw is let in late:** OpenCode v2's shared service may have started before
   the run with no proxy; OpenClaw's `proxy.proxyUrl` sends calls past the recorder; OpenClaw logs
   to `/tmp/openclaw/` shared by every `HOME`; both keep sessions only in SQLite, so the wire is the
   only record and a missed call has no backup.
6. **If Cursor is tried:** the server may force HTTP/2; the proxy refuses it and the agent cannot
   reach its model, or retries for a week. That blocks the harness, which breaks point 3 in spirit.
7. **Copilot CLI** (if someone uses it anyway, unrecorded): it downloads updates at every session
   start by default, and syncs sessions to GitHub by default. Not our record.
8. **Gemini CLI users** on a personal Google sign-in were cut off on 18 Jun 2026. Anyone who plans
   on it with a free account will find it does not work.
9. **Release churn in general.** In the 30 days before 29 Oct: Amp 218 builds, Copilot 15,
   OpenClaw 16, Kilo 18, OpenCode v2 21, Droid 28. Any facts sheet older than two weeks is likely
   stale on some detail.

### D4.8 What could go wrong with these recommendations

- Closing the door may turn away a strong team that built its setup on OpenCode. Mitigation: say so
  this week on the site and in the participant email, with the reason.
- The Amp BYOK probe may find many provider hosts. Adding them all widens what the proxy opens and
  records; a `model_host_from` reading of Amp's settings may be needed, which is new code days
  before Friday. Fallback: tell Amp users to keep Amp's own inference for the run and say so in RULES.
- Cutting the RULES flag bans removes text that a reviewer used to judge a run. If a later edition
  adds a harness, the bans must be re-measured anyway, because flags change (OpenCode `share` is
  already gone).
- Cutting spike fixtures changes what the over-redaction test sees. The test still runs on the
  remaining fixtures, but a regression on those shapes would no longer be caught.
- Download counts are a weak proxy for use at a hackathon. Amp installs mostly from its own script,
  so its npm count understates it. Pi's count likely includes library use.

### D4.9 Unverified

- OpenCode 2 background service, its session path and its subagents (one third-party source).
- Whether Amp's newest build still honours `AMP_SKIP_UPDATE_CHECK` (string present, behaviour not run).
- Whether Amp BYOK or vendor-subscription routing calls providers from the local CLI.
- Cursor's agent host names (server-provided); whether a Cursor HTTP/2 refusal falls back to HTTP/1.1.
- Copilot CLI: which layer opens model connections; whether `NODE_EXTRA_CA_CERTS` reaches it; the
  `events.jsonl` schema and `agentId` field (community sources).
- Gemini CLI 0.66 proxy behaviour; Antigravity's Go version, session storage and update switch.
- Goose CA handling; Cline proxy, CA and update behaviour; Kilo session path; Droid, Pi, Qwen,
  Crush and Kimi session formats.
- Python CA variables for Aider and LiteLLM (general knowledge, not checked for these versions).

### Sources (D4)

Web (fetched 29 Oct 2026 unless noted; doc or release dates in the text):

- [OpenCode releases](https://github.com/anomalyco/opencode/releases)
- [OpenCode 2 docs](https://opencode.ai/v2/docs)
- [OpenCode 2.0 write-up, 14 Oct 2026](https://capitalandcompute.net/blog/opencode-2-0-what-changed/)
- [OpenClaw releases](https://github.com/openclaw/openclaw/releases)
- [Amp news](https://ampcode.com/news)
- [Amp: Free Agent, 13 Sep 2026, updated 30 Sep 2026](https://ampcode.com/news/free-agent)
- [Cursor CLI changelog](https://cursor.com/docs/cli/changelog)
- [Cursor CLI configuration](https://cursor.com/docs/cli/reference/configuration)
- [Cursor install script](https://cursor.com/install)
- [Google: transitioning Gemini CLI to Antigravity CLI, 19 May 2026](https://developers.googleblog.com/an-important-update-transitioning-gemini-cli-to-antigravity-cli/)
- [Gemini CLI releases](https://github.com/google-gemini/gemini-cli/releases)
- [Gemini CLI session management, updated 18 Jun 2026](https://geminicli.com/docs/cli/session-management/)
- [Gemini CLI configuration](https://geminicli.com/docs/reference/configuration/)
- [Antigravity CLI cheat sheet, updated 8 Oct 2026](https://computingforgeeks.com/antigravity-cli-cheat-sheet/)
- [V2EX thread on agy and proxies, May to Jul 2026](https://www.v2ex.com/t/1214168)
- [Go crypto/x509 SystemCertPool](https://pkg.go.dev/crypto/x509#SystemCertPool)
- [Copilot CLI changelog (raw)](https://raw.githubusercontent.com/github/copilot-cli/main/changelog.md)
- [Copilot CLI config directory reference](https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-config-dir-reference)
- [Copilot CLI session data](https://docs.github.com/en/copilot/concepts/agents/copilot-cli/chronicle)
- [Copilot allowlist reference](https://docs.github.com/en/enterprise-cloud@latest/copilot/reference/copilot-allowlist-reference)
- [Coder: Copilot behind a proxy](https://coder.com/docs/ai-coder/ai-gateway/clients/copilot)
- [Goose releases](https://github.com/aaif-goose/goose/releases)
- [Goose session management](https://goose-docs.pages.dev/docs/guides/sessions/session-management)
- [Goose proxy docs diff, PR 7311](https://github.meowingcats01.workers.dev/aaif-goose/goose/pull/7311/files)
- [Aider options](https://aider.chat/docs/config/options.html)
- [Cline CLI overview](https://docs.cline.bot/cli/overview)
- [Cline SDK sessions](https://docs.cline.bot/sdk/sessions)
- [codeburn issue 874 (Cline CLI paths)](https://github.com/getagentseal/codeburn/issues/874)
- [Cline architecture guide](https://www.fast.io/resources/cline-architecture-guide.md)
- [Kilo CLI docs](https://kilo.ai/docs/code-with-ai/platforms/cli)
- [Factory Droid CLI reference](https://docs.factory.ai/droid-cli/cli-reference)
- npm registry `https://registry.npmjs.org/<package>` and `https://api.npmjs.org/downloads/point/2026-10-21:2026-10-27/<package>`; PyPI `https://pypi.org/pypi/<package>/json`; pypistats `https://pypistats.org/api/packages/<package>/recent`

Repo (4b226de2 unless noted): `README.md:4`, `README.md:137-138`, `docs/HARNESS-ONBOARDING.md:188-189`,
`docs/RULES.md:167-168`, `docs/RULES.md:216-225`, `docs/RULES.md:387`,
`connectors/amp-cli/harness.toml:1-115`, `docs/harnesses/amp-cli.md:3,79,132,144`,
`docs/HARNESS-SETUP.md:171-181,413-426`, `harnesses/install.sh:1-62`,
`docs/plans/2026-10-28-one-capture-system.md:43`, `crates/goalcup-cli/src/proxy/tls.rs:26-30`,
`crates/goalcup-cli/src/proxy/connect.rs:624-637`, `crates/goalcup-daemon/tests/it/redaction.rs:608-640`,
`spikes/cursor-cli/fixtures/README.md:1-25`, `TODO.md:214`;
`origin/opencode-openclaw:docs/harnesses/opencode-cli.md`, `origin/opencode-openclaw:docs/harnesses/openclaw-cli.md`,
`origin/opencode-openclaw:docs/research/results/2026-10-opencode-openclaw.md`;
`/home/user/agentcommunity_page/features/goal-cup-campaign/content.ts:292`,
`/home/user/agentcommunity_page/features/goal-cup-proxy/GOAL-CUP-PROXY.md:56-57`.
