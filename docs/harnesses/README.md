# Harness facts sheets

One public page per harness, `docs/harnesses/<connector-id>.md`. It says where that harness keeps
its sessions, config and logins on each OS, how it authenticates, and which hosts it calls.

A connector describes only what Goal Cup Proxy does today. The sheet records the rest: what to
check when Goal Cup Proxy gains a platform or a new way to capture, without asking each team again.
A sheet is also a complete contribution on its own. A harness that does not fit Goal Cup Proxy yet
sends its sheet with status `facts only`, and the sheet shows what to build.

## The 15 harnesses

This is the one harness table; other pages link here and never copy it. Each harness has one
connector file, `connectors/<id>.toml` (OpenCode has two, one per major version). Whether a harness
is verified, and on which version, is written by the test pad to `connectors/tiers.toml`, not here.
"Wire only" means the model calls are recorded through the proxy, but the session files are not.

| # | Harness (version checked) | Connector | Reaches models | Sessions and subagents | What goalcup cannot record or stop |
|---|---|---|---|---|---|
| 1 | Claude Code 2.1.312 | `claude-code-cli` | HTTPS to `api.anthropic.com` or a gateway | `projects/<p>/<session>.jsonl`; subagents under `<session>/subagents/` | A settings `env` block that sets a proxy or CA value is refused at start |
| 2 | Codex 0.165.0 | `codex-cli` | WebSocket and HTTPS to `api.openai.com`; `chatgpt.com` on ChatGPT sign-in | `sessions/YYYY/MM/DD/rollout-*.jsonl`, child threads too | `$CODEX_HOME/.env` overwrites the proxy and CA, so a proxy or CA value there is refused at start; started with `--no-daemon` |
| 3 | Hermes 0.22.5 | `hermes-cli` | Chat Completions from its gateway process | `state.db`; delegation logs under `cache/delegation/live/` | Its `copilot-acp` child is wire only; goalcup stops the gateway for its own home before and after the run |
| 4 | OpenCode 1.19.5 and 2.2.2 | `opencode-cli`, `opencode2-cli` | HTTPS to the chosen provider, or OpenCode's gateway on `opencode.ai` | One `opencode.db`; subagents are child sessions in it | `opencode.ai` serves both the gateway and the console page where a participant copies a key; OpenCode 2 is started `--standalone`, so it does not attach to a shared background server |
| 5 | OpenClaw 2026.10.9 | `openclaw-cli` | Chat Completions through provider plugins | `agents/<agent>/agent/openclaw-agent.sqlite`, `state/openclaw.sqlite` | Subagents not recorded on the wire when they run in a Gateway started outside goalcup: goalcup starts no Gateway and stops none, because `openclaw gateway stop` stops the account's own service. Its own proxy setting is refused at start; a hand-written JSON5 `openclaw.json` does not parse as JSON, so its proxy setting is not checked |
| 6 | Amp 0.0.1793275344 | `amp-cli` | One WebSocket to `ampcode.com`; Amp's servers call the model | No transcript; a metadata log per thread | Provider calls are made on Amp's servers |
| 7 | GitHub Copilot CLI 1.0.105 | `copilot-cli` | HTTPS to `*.githubcopilot.com`, or a provider key | `session-state/<id>/events.jsonl`, `session-store.db` | Sign-in and the token exchange use `github.com` and `api.github.com`, which carry far more than sign-in; goalcup passes them through as tunnels and does not read them. `/delegate` runs a cloud agent |
| 8 | Gemini CLI 0.66.0 | `gemini-cli` | HTTPS to `generativelanguage.googleapis.com`; Code Assist on Google sign-in | `.gemini/tmp/<project>/chats/session-*.jsonl` | `cloudcode-pa.googleapis.com` carries both Code Assist model calls and sign-in calls; remote A2A subagents run elsewhere |
| 9 | Goose 1.57.0 | `goose-cli` | HTTPS to the provider | `data/sessions/sessions.db`; subagents are rows in it | Asks about usage data and a provider on a first start in a new home |
| 10 | Kilo CLI 7.8.25 | `kilo-cli` | HTTPS to the provider, Kilo's gateway or Vercel AI Gateway | `kilo.db`; subagents are child sessions | None found |
| 11 | Qwen Code 0.26.4 | `qwen-cli` | Chat Completions to the provider | `projects/<p>/chats/<id>.jsonl`; subagents under `projects/<p>/subagents/` | Its built-in `claude-code` and `codex` subagents are wire only; a settings `proxy` is refused at start |
| 12 | Crush 0.99.0 | `crush-cli` | Chat Completions to the provider | `<workspace>/.crush/crush.db`; subagents are child rows | None found |
| 13 | Factory Droid 0.251.0 | `droid-cli` | A provider key to its base URL; Factory's backend for sign-in and sync | `~/.factory/sessions/<slug>/<uuid>.jsonl` | A separately started `droid daemon` or desktop app keeps its own environment, and `droid daemon` has no stop command, so goalcup cannot stop it |
| 14 | Kimi Code 2.4.1 | `kimi-code-cli` | HTTPS to `api.kimi.com`, or a provider in `config.toml` | `sessions/<workspace>/<session>/agents/*/wire.jsonl`; index `session_index.jsonl` | None found |
| 15 | Cline CLI 3.0.74 | `cline-cli` | HTTPS to the provider, or Cline's own provider | `data/sessions/<id>/`, index `data/db/sessions.db` | A hub daemon already running may hold another environment. No continue form: `goalcup continue` starts a new Cline session |

The per-harness sheets below are still in the old shape; they are rewritten from what the test pad shows, after it runs.

The sheets so far: [codex-cli](codex-cli.md) (the worked example), [claude-code-cli](claude-code-cli.md),
[hermes-cli](hermes-cli.md), [kilo-cli](kilo-cli.md) and [amp-cli](amp-cli.md).

Current research validation evidence and its limits are tracked in the
[coverage matrix](../research/coverage-matrix.md), separately from these capability facts.

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
connector change when the sheet's facts still hold: builds are recorded, not pinned.
