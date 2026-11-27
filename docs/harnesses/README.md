# Harness facts sheets

One public page per harness, `docs/harnesses/<connector-id>.md`. It says where that harness keeps
its sessions, config and logins on each OS, how it authenticates, and which hosts it calls.
Hosts that a harness or the agent's own tools open on purpose but that carry no model calls (Codex's `ab.chatgpt.com`, Claude Code's Datadog intake, GitHub and the package registries) are listed as expected tunnels in the `expected` array of `connectors/model-hosts.toml`.

A connector describes only what Goal Cup Proxy does today. The sheet records the rest: what to
check when Goal Cup Proxy gains a platform or a new way to capture, without asking each team again.
A sheet is also a complete contribution on its own. A harness that does not fit Goal Cup Proxy yet
sends its sheet with status `facts only`, and the sheet shows what to build.

## The harnesses

This is the one harness table; other pages link here and never copy it. Each Connector links to its
sheet. Each harness has one connector file, `connectors/<id>.toml` (OpenCode 1 and 2 share `opencode-cli`, which has a `1.x` version row; runs started as `opencode2-cli` continue under it).
"Wire only" means the model calls are recorded through the proxy, but the session files are not.

| # | Harness (version checked) | Connector | Reaches models | Sessions and subagents | Tier (from `connectors/tiers.toml`) | What goalcup cannot record or stop |
|---|---|---|---|---|---|---|
| 1 | Claude Code 2.1.312 | [`claude-code-cli`](claude-code-cli.md) | HTTPS to `api.anthropic.com` or a gateway | `projects/<p>/<session>.jsonl`; subagents under `<session>/subagents/` | verified (2.1.312, darwin-arm64) | A settings `env` block that sets a proxy or CA value is refused at start |
| 2 | Codex 0.165.0 | [`codex-cli`](codex-cli.md) | WebSocket and HTTPS to `api.openai.com`; `chatgpt.com` on ChatGPT sign-in | `sessions/YYYY/MM/DD/rollout-*.jsonl`, child threads too | verified (0.165.0, darwin-arm64) | `$CODEX_HOME/.env` overwrites the proxy and CA, so a proxy or CA value there is refused at start; started with `--no-daemon` |
| 3 | Hermes 0.22.5 and 0.22.6 | [`hermes-cli`](hermes-cli.md) | Chat Completions from its gateway process | delegation `manifest.json` and `task-*.log` files under `cache/delegation/live/`; `state.db` is not copied (its search index can hold text out of context) | verified (0.22.5 and 0.22.6, darwin-arm64) | Its `copilot-acp` child is wire only; goalcup stops the gateway for its own home before and after the run; a proxy or CA value in its `.env` is refused at start |
| 4 | OpenCode 1.19.5 and 2.2.2 | [`opencode-cli`](opencode-cli.md) | HTTPS to the chosen provider, or OpenCode's gateway on `opencode.ai` | `opencode.db`, not copied (it holds saved keys and OAuth tokens); subagents are child sessions, linked from the wire | not verified | `opencode.ai` serves both the gateway and the console page where a participant copies a key; OpenCode 2 is started `--standalone`, so it does not attach to a shared background server |
| 5 | OpenClaw 2026.10.9 | [`openclaw-cli`](openclaw-cli.md) | Chat Completions through provider plugins | `agents/<agent>/agent/openclaw-agent.sqlite`, `state/openclaw.sqlite`; neither is copied (they hold its sign-in), so wire only | not verified | Subagents are not recorded on the wire when they run in a Gateway started outside goalcup; its own proxy setting is refused at start (sheet section 8); an `agents.defaults.workspace` in its `openclaw.json` (OpenClaw's onboarding writes one) moves the agent out of the project (sheet section 1) |
| 6 | Amp 0.0.1793275344 | [`amp-cli`](amp-cli.md) | One WebSocket to `ampcode.com`; Amp's servers call the model | No transcript; a metadata log per thread | not verified | Provider calls are made on Amp's servers |
| 7 | GitHub Copilot CLI 1.0.105 | [`copilot-cli`](copilot-cli.md) | HTTPS to `*.githubcopilot.com`, or a provider key | `session-state/<id>/events.jsonl`; `session-store.db`, a search index, is not copied | not verified | Sign-in and token exchange to `github.com` and `api.github.com` pass through as tunnels and are not read; `/delegate` runs a cloud agent (sheet section 8) |
| 8 | Gemini CLI 0.66.0 | [`gemini-cli`](gemini-cli.md) | HTTPS to `generativelanguage.googleapis.com`; Code Assist on Google sign-in | `.gemini/tmp/<project>/chats/session-*.jsonl` | not verified | `cloudcode-pa.googleapis.com` carries both Code Assist model calls and sign-in calls; remote A2A subagents run elsewhere |
| 9 | Goose 1.57.0 | [`goose-cli`](goose-cli.md) | HTTPS to the provider | `data/sessions/sessions.db`; subagents are rows in it | not verified | A provider picked from the built-in catalog is recorded only if its host is on the shared list; otherwise it is a tunnel (sheet section 6) |
| 10 | Kilo CLI 7.8.25 | [`kilo-cli`](kilo-cli.md) | HTTPS to the provider, Kilo's gateway or Vercel AI Gateway | `kilo.db`; subagents are child sessions | not verified | A provider picked from the built-in catalog is recorded only if its host is on the shared list; otherwise it is a tunnel (sheet section 6) |
| 11 | Qwen Code 0.26.4 | [`qwen-cli`](qwen-cli.md) | Chat Completions to the provider | `projects/<p>/chats/<id>.jsonl`; subagents under `projects/<p>/subagents/` | not verified | Its built-in `claude-code` and `codex` subagents are wire only; a settings `proxy` is refused at start |
| 12 | Crush 0.99.0 | [`crush-cli`](crush-cli.md) | Chat Completions to the provider | `<workspace>/.crush/crush.db`; subagents are child rows | not verified | A run started below a repository root that has `.crush` writes `crush.db` there, not where goalcup reads; an unhinted provider stays a tunnel (sheet section 8) |
| 13 | Factory Droid 0.251.0 | [`droid-cli`](droid-cli.md) | A provider key to its base URL; Factory's backend for sign-in and sync | `.factory/sessions/<slug>/<uuid>.jsonl` under the home goalcup sets with `FACTORY_HOME_OVERRIDE` (undocumented; `HOME` stays the participant's) | not verified | A separately started `droid daemon` or desktop app keeps its own environment and cannot be stopped by goalcup (sheet section 8) |
| 14 | Kimi Code 2.4.1 | [`kimi-code-cli`](kimi-code-cli.md) | HTTPS to `api.kimi.com`, or a provider in `config.toml` | `sessions/<workspace>/<session>/agents/*/wire.jsonl`; index `session_index.jsonl` | not verified | None found |
| 15 | Cline CLI 3.0.74 | [`cline-cli`](cline-cli.md) | HTTPS to the provider, or Cline's own provider | `data/sessions/<id>/`, index `data/db/sessions.db` | not verified | A hub daemon already running may hold another environment. No continue form: `goalcup continue` starts a new Cline session |
| 16 | Your own loop (`homemade-agent`), no build checked | [`homemade-cli`](homemade-cli.md) | Whatever your loop calls, through `HTTPS_PROXY` | Any `*.jsonl` it writes below `GOALCUP_TRACE_DIR` | not verified (always) | Calls that ignore `HTTPS_PROXY` or the certificate variables, and anything the loop does not trace |

The Tier column is a snapshot of [`connectors/tiers.toml`](../../connectors/tiers.toml), which the test
pad writes; that file is the record, and the column is not regenerated. Until the pad has run a harness
its tier is "not verified" with the reason "not yet run on the pad". A verified or unsupported tier
appears only after the pad records it (`scripts/pad/rows/PAD-RESULTS.md`). Each product sheet
ends with a "Pad result" section that points to the file. The full list of limits is in section 8 of
each product sheet.

## Not on the list

- **Your own agent loop**, one you wrote yourself, needs no connector of its own. Put it on your
  `PATH` as `homemade-agent` and use `--harness homemade`: [what it must do](homemade-cli.md).
- **A product not in the table** (for example Cursor CLI or Aider) is not in this build, and a run
  with it does not count. It joins with its own connector
  ([HARNESS-ONBOARDING](../HARNESS-ONBOARDING.md)), or ask for it with a
  [harness request](https://github.com/agentcommunity/goal-cup-proxy/issues/new?template=harness_request.yml).
  A connector reaches participants only in a release, and counts for Edition 01 only if it is
  merged before the cup release tag (see [RULES](../RULES.md#timeline)).
- **Desktop and IDE agents** (Cursor, Windsurf, Zed, VS Code agent mode) are not supported: use a
  CLI. Cloud agents are not eligible ([RULES](../RULES.md#harness-support)).

## What is public

Everything in a sheet is public. A sheet holds facts from your documentation, or that anyone can
measure on an installed copy: paths, variable names, file formats, host names, exit codes. It
never holds a secret, token, key, account name or personal path. Write `$HOME` and `%APPDATA%`. To
keep a fact private, leave it out, say so on the "Public disclosure" line, and email it to
goalcup@agentcommunity.org.

## How to write one

Copy [the template](../../skills/adding-a-capture-harness/templates/harness-facts.md) to
`docs/harnesses/<connector-id>.md`. Keep its headings and order, so every sheet reads the same.
Replace each one-line prompt with facts. Every fact has a source: a link to your docs, "measured
on version X", or "unknown". Write "unknown" rather than guess. "Same as macOS" is fine when true.

The `Status:` line takes one of three values:

- **shipped connector**: `connectors/<id>.toml` is on main, or the same PR adds it. This is source
  inclusion, not a claim that participant binaries are published or event practice is complete.
- **core ready, connector pending**: no row of the core-change table in
  [HARNESS-ONBOARDING](../HARNESS-ONBOARDING.md#do-i-need-a-core-change) needs a core change, but no
  connector exists yet.
- **facts only**: Goal Cup Proxy needs a core change first, or a key fact is unknown.

Section 8, "Things Goal Cup cannot isolate", lives only in the sheet. A connector PR links the
sheet in its description and does not repeat it.

## How to update one

When a release changes something a sheet says (a path, a variable, the session format, an auth
flow, a host), open a PR that edits the sheet and updates "Facts for version" and "Last checked".
If the harness has a connector, change the connector in the same PR and follow
[HARNESS-ONBOARDING](../HARNESS-ONBOARDING.md) again. A new build of a known harness needs no
connector change when the sheet's facts still hold: builds are recorded, never refused
([Harness versions](../CONNECTOR-SCHEMA.md#harness-versions)).
