# Claude Code and Codex through Vercel AI Gateway: design

22 Oct 2026. A design for review, not merged without the owner. No code has changed. Vercel and
vendor facts were read today from the pages linked on each line; Goal Cup facts name their file.
**Unmeasured** means nobody has run it. Background: [VERCEL-COMPAT](VERCEL-COMPAT.md), section 2.

## 1. Problem

A participant may point Claude Code or Codex at Vercel AI Gateway (`ai-gateway.vercel.sh`). Today
that model traffic is not recorded. The Claude Code connector intercepts only `api.anthropic.com`
(`providers = ["anthropic"]`), and the Codex connector only `chatgpt.com`. Every other host is an
opaque tunnel (D1, D62). So the gateway's traffic tunnels, counted in `proxy admission:` as
`tunneled_not_approved`, with no wire record. The native session file still records. Nothing
refuses the start.

The hard rule stays: **Goal Cup never sets a model base URL** (D1, AGENTS.md). The participant
sets the gateway in the harness's own config, as D67 asks for permission settings: Goal Cup
recommends, the participant configures. Goal Cup then reads the configured address as data, as
D62 does for Hermes (`model_host_from`), and records that host.

## 2. Facts that shape the design

| Fact | Source |
|---|---|
| One host, `ai-gateway.vercel.sh`. Anthropic Messages at `/v1/messages`, OpenAI Chat Completions at `/v1/chat/completions`, Responses at `/v1/responses`. | [AI Gateway](https://vercel.com/docs/ai-gateway), [Anthropic Messages](https://vercel.com/docs/ai-gateway/sdks-and-apis/anthropic-messages-api) |
| Claude Code base `https://ai-gateway.vercel.sh/claude-code`, with no `/v1`: the SDK adds `/v1/messages`. Key in `ANTHROPIC_AUTH_TOKEN`, with `ANTHROPIC_API_KEY=""`. A Claude subscription works too, with `ANTHROPIC_CUSTOM_HEADERS="x-ai-gateway-api-key: Bearer <key>"`. | [Claude Code page](https://vercel.com/docs/ai-gateway/coding-agents/claude-code) |
| `vercel ai-gateway setup --agent claude-code` writes `ANTHROPIC_BASE_URL` into `~/.claude/settings.json` and puts the key in the shell startup file (from the Keychain on macOS). | same page |
| Claude Code's `env` settings key sets variables "for every session and its subprocesses". Precedence: managed, `--settings`, project local, shared project, user. The `env` block follows those levels. Project `env` values wait for folder trust. Claude Code reloads settings files when they change. | [Settings](https://code.claude.com/docs/en/settings), [Settings reference](https://code.claude.com/docs/en/settings-reference) |
| Codex: `model_provider = "vercel"` and `[model_providers.vercel]` with `base_url = "https://ai-gateway.vercel.sh/codex/v1"`, `env_key = "AI_GATEWAY_API_KEY"`, `wire_api = "responses"`. The setup command exports the key from the shell startup file. WebSocket mode is `supports_websockets = true`, OpenAI models only. Since 0.134.0 Codex reads no `profile` selector from `config.toml`; profiles are separate files picked by `--profile`. | [Codex page](https://vercel.com/docs/ai-gateway/coding-agents/openai-codex) |
| Codex provider keys: `base_url`, `env_key`, `experimental_bearer_token` ("discouraged"), an `[auth]` command that prints the token, `http_headers`, `env_http_headers`. Built-in ids `openai`, `ollama`, `lmstudio` cannot be redefined. Top-level `openai_base_url` moves the built-in `openai` provider. A trusted project's `.codex/config.toml` cannot set `model_provider`, `model_providers` or `openai_base_url`. | [Codex config reference](https://learn.chatgpt.com/docs/config-file/config-reference) |
| Gateway keys are sent as `Authorization: Bearer` or `x-api-key`. Keys are seen with a `vck_` prefix (Vercel's own example), but no format is promised. Per-key budgets exist. | [API keys](https://vercel.com/docs/ai-gateway/authentication-and-byok/api-keys) |
| The harness's environment is built from nothing, so a key or base URL exported in the shell never reaches it. | D61, `crates/goalcup-cli/tests/it/prep.rs` |
| `model_host_from` refuses a key part named `env` (or `auth`, `key`, `token`, ...) and refuses a missing value. | D62, `crates/goalcup-protocol/src/harness/check.rs`, `crates/goalcup-cli/src/model_host.rs` |
| The recording proxy never records a header value; query strings are dropped. | `crates/goalcup-cli/src/proxy/metadata.rs`, CONNECTOR-SCHEMA `[redaction]` |

Two consequences matter most:

1. **The key must live in the harness's config, not the shell.** For Claude Code that is the
   `env` block of `settings.json` in its Goal Cup home. For Codex, `env_key` cannot work under
   goalcup: the key goes in `experimental_bearer_token` or an `[auth]` command. The Vercel setup
   command does neither, and it writes `~/.claude` and `~/.codex`, which goalcup never uses.
2. **The address sits where `model_host_from` cannot reach it today.** Claude Code's is
   `env.ANTHROPIC_BASE_URL` in `settings.json` (refused: `env`). Codex's is
   `model_providers.<id>.base_url`, where `<id>` is the value of `model_provider` (a two-key
   lookup the reader cannot express).

## 3. Options

| | What | For | Against |
|---|---|---|---|
| **A. Fixed rows** | Add `ai-gateway.vercel.sh` to `providers.toml` (paths `/claude-code/v1/messages`, `/v1/messages`, hint `anthropic-messages`; `/codex/v1/responses`, `/v1/responses`, hint `openai-responses`) and name it in both connectors. | No schema change. Small. | Names one vendor in core, against D62 ("no provider list"). Every Claude and Codex start then probes the gateway and refuses if it does not answer, even for participants who never use it. OpenRouter, LiteLLM or a company proxy still tunnel silently. Needs a measured run per row. |
| **B. Read the address (recommended)** | Extend D62: an `optional` form, a narrow `env` carve-out for `*_BASE_URL`, and a `{key}` selector for Codex. Both connectors read where the participant set the address. | Any https gateway the participant picks is recorded, as for Hermes. A set address is recorded or the start refuses; it never tunnels silently. Participants with no base URL see no change. No vendor in core. | A DECISIONS amendment and a schema addition (schema 1, additive). Reads `settings.json` for one more key. |
| **C. Refuse only** | Read the address as in B, but refuse any start that names one. | Closes the silent tunnel. | Turns away a legitimate setup the owner may want. Still needs B's reader. |

## 4. Recommendation: option B

Propose a new decision (D68, number fixed at merge): **"A set model base URL is recorded or
refused, never tunnelled."** Three additive schema 1 changes to `capture.model_host_from`, then
one line in each connector.

### 4.1 Schema delta

```toml
# connectors/claude-code-cli/harness.toml
model_host_from = { file = "settings.json", key = "env.ANTHROPIC_BASE_URL", optional = true }

# connectors/codex-cli/harness.toml
model_host_from = [
  { file = "config.toml", key = "model_providers.{model_provider}.base_url", optional = true },
  { file = "config.toml", key = "openai_base_url", optional = true },
]
```

1. **`optional = true`.** A missing file, key or selector means "no base URL set": the run records
   the connector's `hosts` rows only, as today, and `host.log` says `model service: none set
   (<key> in <file>)`. A value that **is** set follows D62 unchanged: recorded if it is an https
   DNS name that is not local, Goal Cup's or an auth host, else refused at start and not ready at
   prep. `optional` needs at least one `hosts` row, so an optional connector always records
   something. Without `optional`, D62 is unchanged (Hermes keeps refusing a missing value).
2. **The `env` carve-out.** A key may be exactly two parts, `env.<NAME>`, when `<NAME>` is upper
   case and ends in `_BASE_URL`. Every other use of `env`, and every other denied word, stays
   refused. So the reader can be pointed at `env.ANTHROPIC_BASE_URL` and never at
   `env.ANTHROPIC_AUTH_TOKEN`. This reads no file goalcup does not already parse:
   `participant.no_prompts` already reads `settings.json` (D67), and the reader keeps one string
   and logs only `host:port`.
3. **The `{key}` selector.** One part of the key may be `{<top-level key>}`. The reader takes the
   string at that top-level key in the same file and puts it in that part. The value must be
   1 to 64 bytes of `[A-Za-z0-9_-]`; anything else is "not set". This is the two-key lookup Codex
   needs, and nothing more general.
4. **A list of at most 2 rows.** `model_host_from` may be one table or a list of up to 2. The first
   row whose value is set wins. For Codex this matches its own rule: a custom `model_provider`
   uses its `base_url`; with none, or with `openai` (whose table cannot exist), the built-in
   provider uses `openai_base_url` if set, else its default host (`chatgpt.com`, already a row).

The rest of D62 is kept as is: the file is read at each start and continue, never during a run;
the host joins the intercepted hosts for that segment; a host the connector lists keeps its row.

### 4.2 What is recorded, and what tunnels

| Harness | Base the participant sets | Recorded, with hint | Recorded, no hint | Relayed, not recorded |
|---|---|---|---|---|
| Claude Code | `https://ai-gateway.vercel.sh/claude-code` | `POST /claude-code/v1/messages`, `anthropic-messages` | the rest under `/claude-code/`: `/v1/messages/count_tokens`, `/v1/models` (model picker) | any other path on the host |
| Codex | `https://ai-gateway.vercel.sh/codex/v1` | `POST /codex/v1/responses`, `openai-responses` | the rest under `/codex/v1/`: `/models` | any other path on the host |

This is what `ModelHost::approved()` already builds from a base path
(`crates/goalcup-cli/src/model_host.rs`): `<base>/messages`, `<base>/responses` and
`<base>/chat/completions`, also under `<base>/v1`. No new wire hint and no decoder change is
expected; the gateway's model ids (`anthropic/claude-...`) and any extra response fields are
**unmeasured** against the existing decoders.

The `api.anthropic.com` and `chatgpt.com` rows stay. A gateway run intercepts both its own row
and the gateway. If the harness still calls its home host for anything (Claude Code with a
subscription login may), that is recorded as before. Sign-in hosts (`platform.claude.com`,
`auth.openai.com`) still tunnel. Everything else (git, registries, MCP) still tunnels.

Codex's WebSocket mode (`supports_websockets = true`) is relayed by the same proxy that records
Codex's `chatgpt.com` WebSocket, but is **unmeasured** on the gateway. Recommend it off.

### 4.3 The key

- **Headers.** The gateway key travels as `Authorization`, `x-api-key` or
  `x-ai-gateway-api-key`. No header value is ever recorded (`proxy/metadata.rs`). Nothing changes.
- **Goal Cup never reads a key** (D61). The reader reads one `*_BASE_URL` string. The key sits in
  the same file (`settings.json` `env`, or Codex `config.toml`), which the baseline lists by name,
  size and SHA-256 only (D56), as it does today.
- **The agent can see the key.** Claude Code passes its `env` block to every subprocess, and Codex
  keeps the key in a file the agent can read. That is true of any API-key setup today. If the
  agent prints it, the shared redaction catches `Bearer <key>`, `ANTHROPIC_AUTH_TOKEN=<key>`,
  `"ANTHROPIC_AUTH_TOKEN": "<key>"`, `AI_GATEWAY_API_KEY=<key>` and
  `experimental_bearer_token = "<key>"` (the `bearer` and `generic_secret_assignment` shapes in
  `crates/goalcup-daemon/src/redact/stream.rs`). **A bare `vck_...` on its own is not matched.**
  Follow-up (core, daemon, separate PR): a shared shape `vercel_ai_gateway_key`, once the key
  alphabet and length are measured.

### 4.4 CA and the start probe

- **CA.** Both connectors already hand the recording proxy CA in extend mode
  (`NODE_EXTRA_CA_CERTS`, `CODEX_CA_CERTIFICATE`). An intercepted gateway is trusted through it.
  Extend mode fails open: a gateway host that is *not* intercepted still verifies against the
  real roots and works, unrecorded. That is why a set address must be read, not guessed. Whether
  Codex's custom-provider client honours `CODEX_CA_CERTIFICATE` and `HTTPS_PROXY` like its
  ChatGPT client is **unmeasured**.
- **Upstream.** The recording proxy checks the gateway's certificate against `webpki-roots`. On a
  Vercel Sandbox with firewall credential brokering for this host, that check fails (VERCEL-COMPAT
  section 1). Brokering stays off for the model host.
- **Probe.** With an address set, `goalcup start` and `continue` probe `ai-gateway.vercel.sh:443`
  beside the connector's rows (`proxy::preflight::endpoints_reachable`), and prep's reachability
  line includes it. Without one, nothing new is probed, unlike option A.

### 4.5 What the participant configures (D67 style)

Goal Cup writes none of this. HARNESS-SETUP gains one short section per harness, "Use Vercel AI
Gateway". The Vercel setup command does not do it: it writes `~/.claude` or `~/.codex` and the
shell startup file, which no run reads.

Claude Code, in `capture-homes/claude-code-cli/settings.json` (keep the other keys):

```json
{
  "env": {
    "ANTHROPIC_BASE_URL": "https://ai-gateway.vercel.sh/claude-code",
    "ANTHROPIC_AUTH_TOKEN": "<your AI Gateway key>",
    "ANTHROPIC_API_KEY": "",
    "CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY": "1"
  }
}
```

With a Claude subscription instead of a key: `ANTHROPIC_BASE_URL` plus
`"ANTHROPIC_CUSTOM_HEADERS": "x-ai-gateway-api-key: Bearer <key>"`, then sign in through prep.

Codex, in `capture-homes/codex-cli/config.toml`:

```toml
model_provider = "vercel"
model = "<a gateway model id>"

[model_providers.vercel]
name = "Vercel AI Gateway"
base_url = "https://ai-gateway.vercel.sh/codex/v1"
wire_api = "responses"
experimental_bearer_token = "<your AI Gateway key>"
```

`env_key` does not work under goalcup (no shell variable reaches Codex). An `[auth]` command is
the alternative; whether one that reads the macOS Keychain works with Codex's empty run `HOME` is
**unmeasured**.

### 4.6 What prep checks and says

- Address set and recordable: the harness home step adds one line, `Claude Code uses
  ai-gateway.vercel.sh:443 (env.ANTHROPIC_BASE_URL in settings.json); Goal Cup records it.`
- Address set and not recordable (plain http, localhost, an IP): the step is not ready, with the
  D62 sentence and the way to fix it. Same at start, before anything is created.
- No address: nothing new.
- Prep never checks the key and never says whether it is set (D61). A missing or wrong key shows
  as a 401 in the practice run, which HARNESS-SETUP tells the participant to do first.

## 5. Failure modes

| Case | What happens | Who sees it |
|---|---|---|
| Base URL set where goalcup does not read: a project `.claude/settings.json` or `.claude/settings.local.json` in the workspace (higher precedence, applies once the folder is trusted, and Claude Code reloads it), a managed settings file, or a Codex `--profile` file | The gateway tunnels unrecorded. `proxy admission:` counts `tunneled_not_approved`. The native leg records; the verifier reports native activity without wire streams. | Organizer, at review. A competition clone starts empty, so this needs the agent (or the participant) to write the file mid-run. Codex is safe here: a project config cannot set providers. |
| Address changed mid-run | Not read until the next segment, as D62 says. Claude Code reloads settings; whether it switches base URL mid-session is **unmeasured**. | Same as above. |
| Gateway unreachable at start | Start refuses with the probe's sentence. Mid-run, the harness's own retries apply (`CLAUDE_CODE_RETRY_WATCHDOG`). | Participant. |
| Key in the shell, not the config | The harness gets no key; the first call fails 401. | Participant, in the practice run. |
| Key budget spent | The gateway refuses; Claude Code stops at once on a spend-limit 429. | Participant. A rules question (section 7). |
| An older goalcup with an address set | Tunnels silently, as today. | Nobody at start. Release note: gateway use needs the release with this change. |
| Firewall brokering on the gateway host | Every model call fails the upstream TLS check. | Participant, at the first call. |

## 6. Test plan

Unit, one per behaviour (`crates/goalcup-protocol/src/harness/check.rs`, `model_host.rs`):

1. `env.ANTHROPIC_BASE_URL` loads; `env.ANTHROPIC_AUTH_TOKEN`, `env.FOO`, `env.a.X_BASE_URL` and
   `auth.x_BASE_URL` are refused.
2. `optional` with no `hosts` row is refused.
3. Optional and absent (no file, no key, no selector value) gives no model host and no refusal;
   optional and set to `http://localhost:4000` refuses as D62 does.
4. The selector: `model_provider = "vercel"` reads `model_providers.vercel.base_url`; a selector
   value with `.` or over 64 bytes is "not set"; a missing selector falls to the next row.
5. Two rows: the first set row wins; neither set gives none.

Integration (`crates/goalcup-cli/tests/it`, dev origin with a `*.localhost` TLS stand-in):

6. **The promise:** a base URL set in the harness's own config is recorded or the start refuses;
   it never tunnels. A Claude-shaped `settings.json` naming the stand-in records
   `/claude-code/v1/messages` with `anthropic-messages`; a Codex-shaped `config.toml` records
   `/codex/v1/responses` with `openai-responses`.
7. With no address set, the recorded hosts are exactly today's (no change for subscription users).
8. Prep: a recordable address shows the model service line; an unrecordable one keeps the home
   not ready.

Connector edits regenerate the pins (`sh scripts/pin-manifests.sh`). Live, with a gateway key
(owner or maintainer): one practice run per harness; `proxy admission:` shows `intercepted=` above
0 and no `ai-gateway.vercel.sh` tunnel; `goalcup-verify --research-json` decodes the turns; a
search of the trace for the key finds nothing.

## 7. Open questions for the owner

1. **D62's reach.** Is a gateway "the model service the harness's own config names" for Claude
   Code and Codex too, so D62 covers them? If yes, approve the D68 amendment above.
2. **Model freedom (D52).** Through the gateway, Claude Code can run OpenAI models and Codex can
   run Claude. D52 says "model free". Is that allowed, and is a run still entered as its harness?
3. **Gateway routing.** Virtual models and fallbacks can serve a different model than the one
   requested, and the record shows the request. Allowed, or must the participant turn fallbacks
   off?
4. **Fairness of credits.** If Vercel sponsors gateway credits, do all participants get the same
   budget, and is a run that ends on a spent budget treated like any other spend limit?
5. **Unrecorded model traffic.** If the gateway tunnels anyway (a project settings file, an old
   goalcup), does the run count, or is it "routing work around Goal Cup Proxy" (RULES)?
6. **Option A instead?** If you prefer no schema change before 6 Nov, option A works for Vercel
   only, at the cost of a gateway probe on every Claude and Codex start.
7. **Codex API-key logins.** Unrelated to the gateway: a Codex participant who logs in with an
   OpenAI API key most likely talks to `api.openai.com` (**unmeasured**), which the connector does
   not list today, so that traffic would tunnel too. Measure it and add an `openai` row?
