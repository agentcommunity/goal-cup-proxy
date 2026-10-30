# PRD-11: Connectors and the 15 harnesses

Id: PRD-11. Version: goalcup 1.0. Date: 30 October 2026. Base: main at 4b226de2.
Index: [PRD-00](PRD-00-index.md). Format: [PRD-01](PRD-01-record-format.md).
Facts as of 29 Oct 2026 unless a line says otherwise.

## Goal

One small data file per harness, embedded in the binary, plus one shared list of model hosts. Every
harness goes through the same proxy, the same CA rule and the same session-file reader; only data
differs.

## Points served

7 (lead), 1 (the host list decides what is decrypted), 2 (session-file rows find every subagent),
3 (nothing here can stop a running harness), 4 (sign-in hosts never decrypted, secret files fed to
masking).

## Owner decisions it carries out

- One system for every harness, no patchwork. One proxy, one shared host list (providers, gateways,
  cloud model services, vendor backends), plus hosts the participant registers, plus hosts the
  harness config names as a hint (BRIEF3, 29 Oct night).
- Per harness only data: CA variables, the harness's own proxy setting where it beats `HTTPS_PROXY`,
  daemons to stop, session-file patterns, the auto-update switch.
- Simplified connector schema: 10 to 20 lines, `**` patterns, defaults in code, embedded in the
  binary, pins gone.
- A harness that ignores `HTTPS_PROXY` or our CA is "not supported", never hacked around.
- Connectors turn auto-updates off where a switch exists.
- Every vendor fact is checked against the vendor's current docs before it is built (30 Oct early).
- The 15 harnesses: Claude Code, Codex, Hermes, OpenCode, OpenClaw, Amp, GitHub Copilot CLI,
  Gemini CLI, Goose, Kilo CLI, Qwen Code, Crush, Factory Droid, Kimi Code, Cline CLI. Cursor and Aider
  are out; Cursor stays the documented dodger case.

## The owner's question: is a gateway just another HTTPS proxy endpoint?

Yes. A gateway such as Vercel AI Gateway, OpenRouter or Cloudflare AI Gateway is an HTTPS server
with a different host name. The harness still opens a `CONNECT` through our `HTTPS_PROXY`, still
checks our CA, and the proxy decrypts it because the host is on the shared list. Kilo under Vercel AI
Gateway is the same case as Kilo with a DeepSeek key: Kilo's config names
`ai-gateway.vercel.sh` as an OpenAI-compatible provider
([Vercel's Kilo guide](https://vercel.com/docs/ai-gateway/coding-agents/kilo-code),
[Kilo's page](https://kilo.ai/docs/ai-providers/vercel-ai-gateway)), Kilo's Bun `fetch` honours
`HTTPS_PROXY`, and that host is on the list. Nothing in the code knows the word "gateway" except the
`kind` label the usage reader prints (PRD-09).

You read it right. There are only three shapes, and two of them are the same pipe:

| Shape | Example | What we record | Pad case (PRD-12) |
|---|---|---|---|
| Direct key | Codex to `api.openai.com` | Every byte, decrypted | direct key |
| Gateway | Claude Code to `ai-gateway.vercel.sh`, Kilo to the same | Every byte, decrypted. Identical code path | gateway |
| Dodger | A harness that ignores `HTTPS_PROXY`, refuses our CA, or a setting that routes around us | A tunnel record when its bytes pass us, its session files, a named gap, the label "not supported" | dodger |

Two things look like gateways but are not, and the list handles both as data:

- **Vendor backends** (Amp, Factory's `api.factory.ai`, Cursor). The harness talks to its own vendor,
  and the vendor's servers call the model. We decrypt and record the harness-to-vendor bytes in full.
  The vendor-to-provider call never touches the participant's machine, so it is out of reach for any
  recorder. That is a fact about where inference runs, not a proxy gap.
- **A gateway we do not know.** A self-hosted LiteLLM or a new gateway host. If the harness config
  names it, the config hint adds it. If not, the participant registers it once
  (`goalcup setup --record-host <host>`, PRD-10). Until then it is a tunnel, and a long streaming
  tunnel is flagged (PRD-03), so it is never silent.

So the strong pads are `HTTPS_PROXY` plus gateway (full wire record), and dodgers ride the same light
pipeline with an honest label. No second system.

## Today

| What | Where | Lines | Fact |
|---|---|---:|---|
| Four connectors | `connectors/{amp,claude-code,codex,hermes}-cli/harness.toml` | 556 data | 115 to 159 lines each, about 60% comments |
| Schema, checker, loader, parser | `crates/goalcup-protocol/src/harness/{schema,check,load,parse}.rs`, `harness.rs` | 3,370 src | `deny_unknown_fields` structs (`schema.rs:216-958`); `MAX_NATIVE_ROWS = 8` and no `**` (`check.rs:26`, `:424-462`) |
| Pins | `crates/goalcup-protocol/src/pins.rs`, `pins/connector-dirs.txt`, `scripts/pin-manifests.sh` | 750 + 7 + 274 | Every connector edit regenerates pins or every start refuses (`AGENTS.md:90-91`; `pins.rs:33`) |
| Frozen provider table | `crates/goalcup-protocol/providers.toml` | 25 | Two rows, frozen by D74 (`providers.toml:10`) |
| Config address reader | `crates/goalcup-cli/src/model_host.rs` | 1,540 src | One key per connector; no address means only the connector's hosts are decrypted (`model_host.rs:1-8`); refuses plain http, loopback, IP literals (`:16-24`); a small YAML lookup (`:682`) |
| CA bundle | `crates/goalcup-cli/src/ca_bundle.rs` | 207 src | `SSL_CERT_FILE` and `REQUESTS_CA_BUNDLE` get OS roots plus the run CA; a harness's own variable gets the CA alone (`ca_bundle.rs:17-19`; `capture_policy.rs:68`) |
| Launch env | `crates/goalcup-cli/src/launch.rs:320-333` | part of 2,090 | An allowlist: only `PATH`, `TERM`, `COLORTERM`, `LANG`, `SHELL`, `USER`, `LOGNAME`, `TMPDIR` pass. A `DEEPSEEK_API_KEY` exported in the shell never reaches the harness |
| Homes | `crates/goalcup-cli/src/home.rs`, `per_run_home.rs`, `daemon/.../native_plan.rs:137-311` | 995 + 489 + 175 | One shared isolated home per harness (D39), locked, `real_dirs` refusal |
| Hermes workarounds | `connectors/hermes-cli/harness.toml:54-57, 60, 88-92, 145-152` | data | Writes a `providers.recorder` entry with `base_url` (a model base URL, against `AGENTS.md:81`) and `HERMES_CA_BUNDLE` that breaks Nous token refresh on 0.21.6+ (appendix N, Q4) |
| Tests | protocol `tests/it/{harness_schema,harness_loader,harness_conformance,connector_pins,connector_template,pins_build_fails}.rs`; cli `tests/it/{connector_contract,harness_contract_golden,harness_contract_kit,harness_launch_plan,per_run_home,model_host_from,harness_version_probe}.rs` | 5,989 + 4,982 tests | Mostly schema shape and pins |

Run 1 did not exercise any of this beyond Codex; its report is on branch
`origin/smart/tender-clarke-qupne3` (`docs/research/results/2026-10-30-two-hour-run.md`). Per-harness
facts below come from appendix O (`docs/plans/2026-10-30-audit/O-harness-access.md`), appendix D, the
OpenCode and OpenClaw facts sheets on `origin/opencode-openclaw`, and web checks for Kimi Code and
Cline CLI on 30 Oct 2026 (links inline).

## Design

### 1. Files

```
connectors/model-hosts.toml        the one shared host list
connectors/<id>.toml               one connector per harness, 10 to 20 lines
docs/harnesses/<id>.md             one facts sheet per harness (short; the table in section 7 is the seed)
crates/goalcup/src/connector/      schema, loader, host list, hints, launch plan (about 750 src lines)
```

All `connectors/*.toml` are compiled in with `include_str!` through one generated list
(`connector/builtin.rs`, a `const` array of `(id, bytes)`). The binary needs no connectors folder.
There are no pins. Instead, the `start` event (PRD-01 section 11) carries `connector`,
`connector_sha256` (SHA-256 of the exact TOML bytes used), `harness_version`, and the three host lists
in force. Anything that differs from the release is visible in the record. That is the whole
integrity story.

A contributor or the pad loads a file from disk with `goalcup start --connector <path.toml>` and
`--hosts <path.toml>` (PRD-10 owns the flags). Same parser, same checks; the start event then says
`connector_source: "file"`. No dev mode, no origin classes.

### 2. The connector schema

Required: `id`, `name`, `program`. Everything else is optional. Unknown keys are refused at load, so
a typo fails the build test, not a run.

| Key | Type | Default in code | What it does |
|---|---|---|---|
| `id` | string `[a-z0-9-]{1,32}` | | Connector id; `goalcup start <id>` |
| `name` | string | | Shown to the participant |
| `program` | string | | Executable found on the participant's `PATH` |
| `version` | string | none | Prefix the `--version` output must contain, else start refuses with "use connector X" (only OpenCode 1 versus 2 needs it) |
| `args` | string list | `[]` | Arguments for `goalcup start`. Templates `{workspace}` |
| `resume` | string list | `[]` | Appended for `goalcup continue` |
| `home` | map var to template | none | Variables pointed into goalcup's isolated home for this harness. Templates `{home}` |
| `env` | map | none | Extra fixed variables (auto-update switches, daemon modes) |
| `ca_env` | string list | none | The harness's own CA variables. Each gets `run-ca.pem` (the CA alone). Only extend-mode variables belong here |
| `unset` | string list | none | Participant variables removed because they beat `HTTPS_PROXY` (for example `OPENCLAW_PROXY_URL`) |
| `refuse_if_set` | list of `{file, keys}` | none | Before launch: a non-empty value at any key stops the start with the file and key named. For settings that route around us |
| `host_hints` | list of `{file, key}` | none | Keys whose `https` URL values add hosts to the `config` list |
| `secret_files` | string list | none | Files whose values PRD-05 masks as exact values (keys the harness stores itself) |
| `before`, `after` | list of argv | none | Run with `program` before launch and after exit: stop background daemons. 10 s each, failure only logged |
| `native` | list of `{name, root, pattern, form}` | `form = "append"` | Session-file rows (PRD-04). Root templates `{home}`, `{workspace}`; `**` allowed |
| `mask` | list of `MaskRule` | none | Connector mask rules (PRD-05 grammar: `json_field` with `string` or `scalar`, or `header_prefix`) |

Rules the loader checks, and nothing else: types; ids and row names unique; templates known;
patterns relative with no `..`, no absolute path, `**` only as a whole component (PRD-04 section 2);
`file` paths start with `{home}` or `{workspace}`; no `HTTPS_PROXY`, `NO_PROXY`, CA or `HOME`-style
variable in `env` that the code sets itself (those go in `home` or `ca_env`). No row caps, no deny
lists, no harness-name checks.

File keys use dots; `*` matches any one key or array index; a segment may contain `*` as a glob
(`*_BASE_URL`). Formats by extension: `.json`, `.toml`, `.yaml` or `.yml` (the block lookup copied
from `model_host.rs:682`), anything else is read as dotenv (`KEY=value` lines). A `file` may hold a
`*` in its last component (`custom_providers/*.json`). Files over 1 MiB, unreadable files and
unparsable files are skipped with a log line (a memory guard, never a refusal).

### 3. Defaults in code (the same for every harness)

The launch plan starts from the **participant's whole environment** (point 7: run the way they run
it; a key exported in their shell reaches the harness as it would without us). Then, in order:

1. Remove `HTTPS_PROXY`, `https_proxy`, `HTTP_PROXY`, `http_proxy`, `ALL_PROXY`, `all_proxy`,
   `NO_PROXY`, `no_proxy`, `NODE_EXTRA_CA_CERTS`, `SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE`, plus the
   connector's `unset`.
2. Set `HTTPS_PROXY` and `https_proxy` to `http://127.0.0.1:<port>` (no credential, PRD-03 design
   point 5). Never `HTTP_PROXY` (PRD-03 answers non-`CONNECT` with 405).
3. Set `NO_PROXY` and `no_proxy` to `127.0.0.1,localhost,::1`. OpenCode and Kilo need it for their own
   local server (https://opencode.ai/docs/network).
4. Set `NODE_USE_ENV_PROXY=1` (Node 22.21+ and 24+ built-in `fetch` then honours the proxy; Bun and
   non-Node runtimes ignore it).
5. Set `NODE_EXTRA_CA_CERTS` to `run-ca.pem`; set `SSL_CERT_FILE` and `REQUESTS_CA_BUNDLE` to
   `bundle.pem` (OS roots plus the run CA, run CA first). Those two replace a client's roots in
   Python, Go, OpenSSL and `rustls-native-certs`, so they must hold the full bundle; tunnelled hosts
   then still verify (appendix O.1, last paragraph). Each `ca_env` variable gets `run-ca.pem`.
6. Apply `home` (after creating the folders, mode 0700), then `env`.

`program --version` is probed once with that environment minus the proxy variables, 5 s limit, first
line kept (copied from `crates/goalcup-cli/src/harness_version.rs:93-153, 191`). Its output goes in the
`start` event and is checked against `version` when set.

**The isolated home.** `{home}` is `$GOALCUP_HOME/homes/<id>/`, one per harness, kept across runs so a
login made in `goalcup setup` lasts (D39). It is locked with `flock` while a run uses it; a second
start on the same harness refuses. goalcup never writes into the participant's real harness folders
(`AGENTS.md`, "Never write into `~/.claude`..."), and the native reader never sees the participant's
older private sessions in a shared SQLite file. Four shapes cover all 15:

| Shape | Harnesses | `home` |
|---|---|---|
| The harness's own home variable | Claude Code, Codex, Hermes, OpenClaw, Copilot, Gemini, Qwen, Kimi, Cline | one variable to `{home}` |
| XDG folders | OpenCode, Kilo, Goose, Crush | `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_STATE_HOME`, `XDG_CACHE_HOME` under `{home}` |
| `HOME` itself | Amp, Factory Droid | `HOME = "{home}"` (no known narrower variable) |
| Workspace | Crush's database | `{workspace}/.crush`, read as a native root |

**Host hints from the environment.** Besides `host_hints`, every participant variable whose name ends
in `_BASE_URL`, `_API_BASE`, `_ENDPOINT` or `_HOST` and whose value is an `https` URL adds its host to
the `config` list (for example `ANTHROPIC_BASE_URL`, `OPENAI_BASE_URL`, `GOOGLE_GEMINI_BASE_URL`,
`COPILOT_PROVIDER_BASE_URL`, `KIMI_MODEL_BASE_URL`, `AZURE_OPENAI_ENDPOINT`). A hint value is skipped
(with a log line) when it is not `https`, names loopback or an IP literal, or names a `never` host or
goalcup's own server. A hint is never a refusal.

### 4. The shared host list: `connectors/model-hosts.toml`

```toml
# Hosts decrypted and recorded byte for byte for every harness. Everything else is a tunnel.
# Patterns: exact name; "*.a.b" (one or more labels before a.b); one "*" inside one label.
provider = [
  "api.anthropic.com", "api.openai.com", "chatgpt.com", "api.deepseek.com",
  "generativelanguage.googleapis.com", "api.moonshot.ai", "api.moonshot.cn", "api.kimi.com",
  "dashscope.aliyuncs.com", "dashscope-intl.aliyuncs.com", "api.mistral.ai", "api.x.ai",
  "api.z.ai", "open.bigmodel.cn", "api.groq.com", "api.together.ai", "api.together.xyz",
  "api.fireworks.ai", "api.cerebras.ai",
  "bedrock-runtime.*.amazonaws.com", "bedrock-mantle.*.api.aws",
  "aiplatform.googleapis.com", "*-aiplatform.googleapis.com",
  "aiplatform.us.rep.googleapis.com", "aiplatform.eu.rep.googleapis.com",
  "*.openai.azure.com", "*.services.ai.azure.com",
]
gateway = [
  "ai-gateway.vercel.sh", "openrouter.ai", "gateway.ai.cloudflare.com",
  "api.kilo.ai", "ai-gateway.kilo.ai", "api.cline.bot", "opencode.ai",
]
vendor = [
  "ampcode.com", "api.factory.ai", "api.githubcopilot.com", "*.githubcopilot.com",
  "api2.cursor.sh", "*.api5.cursor.sh",
]
never = ["auth.openai.com", "platform.claude.com", "authapi.ampcode.com"]
pinned = []
```

- Hosts are the starting list from PRD-03 section 3, plus Qwen's and Mistral's own APIs, Copilot's
  GitHub-hosted model hosts, Cline's and OpenCode's own gateways. Each added row is "unverified" until
  a pad run or vendor doc confirms it; the builder checks each against the vendor's current docs.
  Rows for `api.cline.bot`, `opencode.ai` and the two `githubcopilot.com` entries are from memory
  and **unverified**.
- `kind` for PRD-09 is the array name. Hosts from the participant or a config hint get kind `other`.
- `never` wins over every pattern and every source. A registration or hint naming a `never` host is
  dropped with a log line. goalcup's own server host is added to `never` in code.
- `pinned` is empty at ship: no harness of the 15 is known to pin. The pad's dodger case writes its
  own copy with a host marked pinned (PRD-03 interfaces, PRD-12 row).
- Left off on purpose: `api.cloudflare.com` (it also carries the agent's own deploys), `models.dev`,
  npm, GitHub, telemetry hosts. They are tunnels, counted with bytes and times.

The `HostList` (seam S3) merges three sources, fixed for an entry: `shared` (this file),
`participant` (a list PRD-10 passes in), `config` (hints). `decide(sni)` answers
`Listed { source, kind, pinned } | SignIn | NotListed`. `ca_subtrees()` gives the wildcard-free suffix
of each entry for the run CA's name constraints (PRD-03 section 4). `lists()` gives the three lists
for the `start` event.

### 5. The launch plan (seam S3)

```rust
pub struct LaunchPlan {
    pub connector: ConnectorRef,          // id, name, sha256, source ("builtin" or "file")
    pub harness_version: Option<String>,
    pub program: PathBuf,                 // resolved on PATH
    pub argv: Vec<String>,                // args or args + resume, templates filled
    pub env: BTreeMap<String, String>,    // complete harness env, minus port and CA paths
    pub home: PathBuf,                    // the isolated home, created and locked
    pub before: Vec<Vec<String>>,         // run by the watchdog before spawn
    pub after: Vec<Vec<String>>,          // run after harness exit, before native finish
    pub hosts: HostList,
    pub native: NativeSources,            // PRD-04 shape: rows with absolute roots, one `since`
    pub mask_rules: Vec<MaskRule>,
    pub secret_values: Vec<Vec<u8>>,      // from secret_files, read at plan time
    pub refusal: Option<Refusal>,         // a refuse_if_set hit: file and key, never the value
}
impl LaunchPlan {
    pub fn finish_env(&mut self, port: u16, run_ca: &Path, bundle: &Path);
}
pub fn plan(id_or_path: &str, ctx: &PlanCtx) -> Result<LaunchPlan, ConnectorError>;
```

`PlanCtx` holds the workspace, the run start time, `resume: bool`, the participant hosts and the
participant environment. `plan` reads files and the environment once; it opens no socket. The
watchdog (PRD-02) mints the CA from `hosts`, binds the port, calls `finish_env`, runs `before`, spawns
`program argv`. PRD-10 shows `refusal` and stops before any of that.

`secret_values`: from each `secret_files` file, every string value of 16 bytes or more (JSON, TOML,
YAML scalars; dotenv values). They go to PRD-05 as exact values beside the launch environment's
credential-named values (PRD-05 section 9). This covers tokens with no known shape that a harness keeps
in its home and may copy into a database we snapshot (OpenCode's `credential` table, OpenClaw's
`auth_profile_store`, appendix O H4.5, H5.5).

### 6. The 16 connector files

One per harness; OpenCode has a second file for its major version 2. "Unverified" marks a value the
builder confirms against the vendor's current docs or `--help` before shipping it.

```toml
# connectors/claude-code-cli.toml  (Claude Code 2.1.312)
id = "claude-code-cli"
name = "Claude Code"
program = "claude"
resume = ["--continue"]
home = { CLAUDE_CONFIG_DIR = "{home}" }
env = { DISABLE_AUTOUPDATER = "1", DISABLE_UPDATES = "1", CLAUDE_CODE_DISABLE_AGENT_VIEW = "1", CLAUDE_CODE_RETRY_WATCHDOG = "1" }
refuse_if_set = [{ file = "{home}/settings.json", keys = ["env.HTTPS_PROXY", "env.https_proxy", "env.NO_PROXY", "env.CLAUDE_CODE_CERT_STORE"] }]
host_hints = [{ file = "{home}/settings.json", key = "env.ANTHROPIC_BASE_URL" }]
secret_files = ["{home}/.credentials.json"]
native = [
  { name = "tool-results", root = "{home}/projects", pattern = "**/tool-results/*", form = "snapshot" },
  { name = "meta", root = "{home}/projects", pattern = "**/*.meta.json", form = "snapshot" },
  { name = "transcripts", root = "{home}/projects", pattern = "**/*.jsonl" },
]
mask = [{ kind = "anthropic_billing_header", header_prefix = "x-anthropic-billing-" }]
```

```toml
# connectors/codex-cli.toml  (Codex 0.165.0)
id = "codex-cli"
name = "Codex"
program = "codex"
args = ["--no-daemon", "-c", "check_for_update_on_startup=false", "-c", 'projects={{ "{workspace}" = {{ trust_level = "trusted" }} }}', "-C", "{workspace}"]
resume = ["resume", "--last"]
home = { CODEX_HOME = "{home}" }
ca_env = ["CODEX_CA_CERTIFICATE"]
refuse_if_set = [{ file = "{home}/.env", keys = ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "NO_PROXY", "no_proxy", "SSL_CERT_FILE"] }]
host_hints = [{ file = "{home}/config.toml", key = "model_providers.*.base_url" }, { file = "{home}/config.toml", key = "openai_base_url" }]
secret_files = ["{home}/auth.json", "{home}/.env"]
native = [
  { name = "sessions", root = "{home}/sessions", pattern = "**/rollout-*.jsonl" },
  { name = "state", root = "{home}", pattern = "*.sqlite*", form = "final" },
]
mask = [
  { kind = "codex_installation_id", json_field = "x-codex-installation-id", value = "scalar" },
  { kind = "codex_safety_identifier", json_field = "safety_identifier", value = "scalar" },
  { kind = "codex_turn_state", json_field = "x-codex-turn-state", value = "scalar" },
]
```

```toml
# connectors/hermes-cli.toml  (Hermes Agent v0.22.5)
id = "hermes-cli"
name = "Hermes"
program = "hermes"
args = ["chat"]
resume = ["--continue"]
home = { HERMES_HOME = "{home}" }
before = [["gateway", "stop", "-y"]]
after = [["gateway", "stop", "-y"]]
host_hints = [{ file = "{home}/config.yaml", key = "model.base_url" }, { file = "{home}/.env", key = "*_BASE_URL" }]
secret_files = ["{home}/.env", "{home}/auth.json", "{home}/.anthropic_oauth.json"]
native = [
  { name = "state", root = "{home}", pattern = "state.db*", form = "final" },
  { name = "delegation", root = "{home}/cache/delegation/live", pattern = "**/*" },
]
```

```toml
# connectors/opencode-cli.toml  (OpenCode 1.19.5)
id = "opencode-cli"
name = "OpenCode"
program = "opencode"
version = "1."
resume = ["--continue"]
home = { XDG_CONFIG_HOME = "{home}/.config", XDG_DATA_HOME = "{home}/.local/share", XDG_STATE_HOME = "{home}/.local/state", XDG_CACHE_HOME = "{home}/.cache" }
env = { OPENCODE_DISABLE_AUTOUPDATE = "1" }
host_hints = [{ file = "{home}/.config/opencode/opencode.json", key = "provider.*.options.baseURL" }]
secret_files = ["{home}/.local/share/opencode/auth.json"]
native = [{ name = "db", root = "{home}/.local/share/opencode", pattern = "opencode.db*", form = "final" }]
```

`connectors/opencode2-cli.toml` is the same with `id = "opencode2-cli"`, `version = "2."` and
`args = ["--standalone"]` (OpenCode 2 otherwise attaches to a shared background service that keeps its
own environment; appendix O H4.3). If OpenCode 2 has an environment switch for standalone mode, the
builder uses it and merges the two files into one (unverified).

```toml
# connectors/openclaw-cli.toml  (OpenClaw 2026.10.9)
id = "openclaw-cli"
name = "OpenClaw"
program = "openclaw"
args = ["chat"]
home = { OPENCLAW_HOME = "{home}" }
env = { OPENCLAW_NO_AUTO_UPDATE = "1" }
unset = ["OPENCLAW_PROXY_URL"]
refuse_if_set = [
  { file = "{home}/.openclaw/openclaw.json", keys = ["proxy.proxyUrl"] },
  { file = "{home}/.openclaw/.env", keys = ["OPENCLAW_PROXY_URL", "HTTPS_PROXY", "NO_PROXY"] },
]
secret_files = ["{home}/.openclaw/.env", "{home}/.openclaw/auth-profiles.json"]
native = [
  { name = "agents", root = "{home}/.openclaw/agents", pattern = "*/agent/openclaw-agent.sqlite*", form = "final" },
  { name = "state", root = "{home}/.openclaw/state", pattern = "openclaw.sqlite*", form = "final" },
]
```

Whether `OPENCLAW_HOME` holds `.openclaw/` or is that folder is unverified
(`docs/help/environment.md:56-62` in the OpenClaw repo); the builder fixes the roots to match.

```toml
# connectors/amp-cli.toml  (Amp 0.0.1793275344-g17bf52)
id = "amp-cli"
name = "Amp"
program = "amp"
resume = ["last"]
home = { HOME = "{home}" }
env = { AMP_SKIP_UPDATE_CHECK = "1" }
secret_files = ["{home}/.local/share/amp/secrets.json", "{home}/.local/share/amp/accounts.json"]
native = [{ name = "thread-metadata", root = "{home}/.cache/amp/logs/threads", pattern = "T-*.log" }]
mask = [{ kind = "amp_ws_token", json_field = "wsToken", value = "string" }]
```

```toml
# connectors/copilot-cli.toml  (GitHub Copilot CLI 1.0.105)
id = "copilot-cli"
name = "GitHub Copilot CLI"
program = "copilot"
resume = ["--continue"]
home = { COPILOT_HOME = "{home}" }
env = { COPILOT_AUTO_UPDATE = "false" }
host_hints = [{ file = "{home}/providers.json", key = "providers.*.baseUrl" }]
secret_files = ["{home}/providers.json", "{home}/config.json"]
native = [
  { name = "sessions", root = "{home}/session-state", pattern = "**/events.jsonl" },
  { name = "store", root = "{home}", pattern = "session-store.db*", form = "final" },
]
```

```toml
# connectors/gemini-cli.toml  (Gemini CLI 0.66.0)
id = "gemini-cli"
name = "Gemini CLI"
program = "gemini"
home = { GEMINI_CLI_HOME = "{home}" }
host_hints = [{ file = "{home}/.gemini/.env", key = "GOOGLE_GEMINI_BASE_URL" }]
secret_files = ["{home}/.gemini/.env", "{home}/.gemini/oauth_creds.json"]
native = [{ name = "chats", root = "{home}/.gemini/tmp", pattern = "**/chats/**/*.jsonl" }]
```

```toml
# connectors/goose-cli.toml  (Goose v1.57.0)
id = "goose-cli"
name = "Goose"
program = "goose"
args = ["session"]
resume = ["--resume"]
home = { XDG_CONFIG_HOME = "{home}/.config", XDG_DATA_HOME = "{home}/.local/share", XDG_STATE_HOME = "{home}/.local/state", XDG_CACHE_HOME = "{home}/.cache" }
ca_env = ["GOOSE_CA_CERT_PATH"]
host_hints = [{ file = "{home}/.config/goose/custom_providers/*.json", key = "base_url" }, { file = "{home}/.config/goose/config.yaml", key = "OPENAI_HOST" }]
secret_files = ["{home}/.config/goose/secrets.yaml"]
native = [{ name = "sessions", root = "{home}/.local/share/goose/sessions", pattern = "sessions.db*", form = "final" }]
```

```toml
# connectors/kilo-cli.toml  (Kilo CLI 7.8.25)
id = "kilo-cli"
name = "Kilo CLI"
program = "kilo"
resume = ["--continue"]
home = { XDG_CONFIG_HOME = "{home}/.config", XDG_DATA_HOME = "{home}/.local/share", XDG_STATE_HOME = "{home}/.local/state", XDG_CACHE_HOME = "{home}/.cache" }
env = { KILO_DISABLE_AUTOUPDATE = "1" }
host_hints = [{ file = "{home}/.config/kilo/kilo.json", key = "provider.*.options.baseURL" }]
secret_files = ["{home}/.local/share/kilo/auth.json"]
native = [{ name = "db", root = "{home}/.local/share/kilo", pattern = "kilo*.db*", form = "final" }]
```

```toml
# connectors/qwen-cli.toml  (Qwen Code 0.26.4)
id = "qwen-cli"
name = "Qwen Code"
program = "qwen"
resume = ["--continue"]
home = { QWEN_HOME = "{home}" }
refuse_if_set = [{ file = "{home}/settings.json", keys = ["proxy"] }]
host_hints = [{ file = "{home}/settings.json", key = "modelProviders.*.*.baseUrl" }]
secret_files = ["{home}/settings.json", "{home}/.env"]
native = [
  { name = "meta", root = "{home}/projects", pattern = "**/*.meta.json", form = "snapshot" },
  { name = "chats", root = "{home}/projects", pattern = "**/*.jsonl" },
]
```

```toml
# connectors/crush-cli.toml  (Crush 0.99.0)
id = "crush-cli"
name = "Crush"
program = "crush"
home = { XDG_CONFIG_HOME = "{home}/.config", XDG_DATA_HOME = "{home}/.local/share", XDG_STATE_HOME = "{home}/.local/state", XDG_CACHE_HOME = "{home}/.cache" }
unset = ["CRUSH_CLIENT_SERVER"]
host_hints = [{ file = "{home}/.config/crush/crush.json", key = "providers.*.base_url" }]
secret_files = ["{home}/.local/share/crush/crush.json"]
native = [{ name = "db", root = "{workspace}/.crush", pattern = "crush.db*", form = "final" }]
```

```toml
# connectors/droid-cli.toml  (Factory Droid 0.251.0)
id = "droid-cli"
name = "Factory Droid"
program = "droid"
home = { HOME = "{home}" }
env = { FACTORY_DROID_AUTO_UPDATE_ENABLED = "false" }
ca_env = ["FACTORY_EXTRA_CA_CERTS"]
host_hints = [{ file = "{home}/.factory/settings.json", key = "customModels.*.baseUrl" }]
secret_files = ["{home}/.factory/settings.json"]
native = [
  { name = "settings", root = "{home}/.factory/sessions", pattern = "**/*.settings.json", form = "snapshot" },
  { name = "sessions", root = "{home}/.factory/sessions", pattern = "**/*.jsonl" },
]
```

```toml
# connectors/kimi-code-cli.toml  (Kimi Code 2.4.0)
id = "kimi-code-cli"
name = "Kimi Code"
program = "kimi"
home = { KIMI_CODE_HOME = "{home}" }
env = { KIMI_CODE_NO_AUTO_UPDATE = "1" }
host_hints = [{ file = "{home}/config.toml", key = "providers.*.base_url" }]
secret_files = ["{home}/config.toml", "{home}/credentials/*.json"]
native = [
  { name = "index", root = "{home}/sessions", pattern = "session_index.jsonl" },
  { name = "wire", root = "{home}/sessions", pattern = "**/wire.jsonl" },
]
```

```toml
# connectors/cline-cli.toml  (Cline CLI 3.0.74)
id = "cline-cli"
name = "Cline CLI"
program = "cline"
home = { CLINE_DIR = "{home}", CLINE_DATA_DIR = "{home}/data" }
env = { CLINE_SESSION_BACKEND_MODE = "local", CLINE_NO_AUTO_UPDATE = "1" }
host_hints = [{ file = "{home}/data/settings/providers.json", key = "*.baseUrl" }]
secret_files = ["{home}/data/settings/providers.json"]
native = [
  { name = "sessions", root = "{home}/data/sessions", pattern = "**/*" },
  { name = "index", root = "{home}/data/db", pattern = "sessions.db*", form = "final" },
]
```

Native rows refine PRD-04 section 2 (its table cites appendix O). Two changes: Codex's state
database row (`*.sqlite*` final in `CODEX_HOME`, so the thread edges are kept whatever the file is
called) and Hermes's `delegation` row uses `**/*` (both the logs and `manifest.json`).

### 7. The 15 harnesses

Columns: how the harness reaches models; proxy and CA variables it honours; session and subagent
files; the auto-update switch; login types; known dodges; the outlook before the pad. Tiers after
the pad are only "verified" (on a named version) or "not verified" (PRD-12); a harness the pad shows
ignoring `HTTPS_PROXY` or our CA becomes "not supported". Sources are appendix O section numbers
unless a link is given.

| # | Harness (version, 29 Oct) | Reaches models | Proxy and CA it honours | Sessions and subagents | Auto-update switch | Login types | Known dodges | Outlook |
|---|---|---|---|---|---|---|---|---|
| 1 | Claude Code 2.1.312 (Bun binary) | HTTPS SSE to `api.anthropic.com`, or a gateway in `ANTHROPIC_BASE_URL`; Bedrock, Vertex, Foundry hosts when switched | `HTTPS_PROXY` (first of `https_proxy`, `HTTPS_PROXY`, `http_proxy`, `HTTP_PROXY`), `NO_PROXY`; `NODE_EXTRA_CA_CERTS` adds; `CLAUDE_CODE_CERT_STORE` (H1.3) | `projects/<p>/<session>.jsonl`; subagents `<session>/subagents/agent-<id>.jsonl` plus `.meta.json`; workflow agents one level deeper (H1.5) | `DISABLE_AUTOUPDATER=1`, `DISABLE_UPDATES=1` (H1.6) | API key env, subscription sign-in (OAuth), `apiKeyHelper`, cloud credentials | Settings `env` block may set proxy values; agent-view supervisor keeps an old env; Claude apps gateway (H1.3) | **Likely verified.** Measured through a gateway on 2.1.312; core set |
| 2 | Codex 0.165.0 (Rust) | WebSocket `wss://api.openai.com/v1/responses` first, HTTPS SSE fallback; `chatgpt.com` with ChatGPT sign-in; provider tables | `HTTPS_PROXY`, `ALL_PROXY`, `NO_PROXY` for HTTPS and WebSocket; `CODEX_CA_CERTIFICATE`, else `SSL_CERT_FILE`, both add (H2.3) | `sessions/YYYY/MM/DD/rollout-*.jsonl`, one per thread, child threads too; state SQLite holds parent edges (H2.5) | No self-install; `check_for_update_on_startup=false` (H2.6) | API key (`CODEX_API_KEY` or `login --with-api-key`), ChatGPT sign-in | `$CODEX_HOME/.env` overwrites env proxy and CA; shared app-server daemon (`--no-daemon` avoids it) (H2.3) | **Likely verified.** Run 1 recorded it; WebSocket per direction is the new part; core set |
| 3 | Hermes v0.22.5 (Python) | Chat Completions from a gateway process per `HERMES_HOME`, about 40 providers, `model.base_url` for gateways | `HTTPS_PROXY`, `NO_PROXY` (httpx); trust through `truststore`, which on Linux reads `SSL_CERT_FILE` as a replacement, so the full bundle (H3.3) | `state.db` (all sessions, children with `parent_session_id`); `cache/delegation/live/<id>/task-<n>.log` (H3.5) | Check only; `updates.check: false` in `config.yaml`, no env switch (H3.6) | Keys in `$HERMES_HOME/.env`, Nous Portal sign-in, borrowed Codex or Claude credentials | A gateway already running for the home keeps its env and calls direct, not even tunnelled; `copilot-acp` child (H3.3) | **Likely verified**, risk: gateway env and `SSL_CERT_FILE` untested on 0.22.5; core set |
| 4 | OpenCode 1.19.5; OpenCode 2.2.2 (Bun) | HTTPS to the chosen provider (native DeepSeek), or OpenCode's own gateway | `HTTPS_PROXY` (measured), `NO_PROXY=localhost,127.0.0.1` required; `NODE_EXTRA_CA_CERTS` adds, `SSL_CERT_FILE` also worked (facts sheet section 6) | One `opencode.db` for all sessions; subagents are child sessions in it (H4.5) | `OPENCODE_DISABLE_AUTOUPDATE=1` (H4.6) | Key in `auth.json` or env, OpenCode gateway account | OpenCode 2 attaches to a background service with its own env unless `--standalone` (H4.3) | **Likely verified** for 1 (measured 28 Oct); **risky** for 2 |
| 5 | OpenClaw 2026.10.9 (Node 24+) | Chat Completions through provider plugins | `HTTPS_PROXY` with `NODE_USE_ENV_PROXY=1`; `NODE_EXTRA_CA_CERTS` adds, `SSL_CERT_FILE` ignored (measured); its own `proxy.proxyUrl` and `OPENCLAW_PROXY_URL` beat env (H5.3) | `agents/<agent>/agent/openclaw-agent.sqlite`, `state/openclaw.sqlite`; logs in `/tmp/openclaw` (H5.5) | `OPENCLAW_NO_AUTO_UPDATE=1` (H5.6) | Keys in `~/.openclaw/.env`, auth profiles in the agent SQLite | Own proxy setting; subagents need a Gateway daemon with its own env; raw sockets "may bypass" (H5.3, H5.4) | **Risky.** Main agent likely; subagents unknown |
| 6 | Amp 0.0.1793275344 (Bun) | One JSON-RPC WebSocket to `ampcode.com`; Amp's servers call the model | `HTTPS_PROXY`; `NODE_EXTRA_CA_CERTS` adds (H6.3) | No transcript; `~/.cache/amp/logs/threads/T-*.log` metadata (H6.5) | `AMP_SKIP_UPDATE_CHECK=1` (H6.6) | Amp account always (`amp login` or `sgamp_` token); keys stored in Amp | Orbs (`-ox`) run remotely; provider calls always server side (H6.3) | **Vendor backend, not verified this cycle** (no Amp account, 30 Oct) |
| 7 | GitHub Copilot CLI 1.0.105 (Node SEA plus Rust addon) | HTTPS to GitHub-hosted models, or BYOK (`anthropic` type for DeepSeek) | `HTTPS_PROXY` beats its `proxyUrl` setting; `NODE_EXTRA_CA_CERTS` plus OS store; `SSL_CERT_FILE` string present; which layer dials is unverified (H9.3) | `session-state/<id>/events.jsonl`; `session-store.db`; subagent layout unverified (H9.5) | `COPILOT_AUTO_UPDATE=false` (H9.6) | GitHub sign-in or BYOK with no GitHub login | `/delegate` cloud agent; voice ignores proxy (issue 5245) (H9.3) | **Likely verified** if the CA layer holds; try the DeepSeek key |
| 8 | Gemini CLI 0.66.0 (Node) | HTTPS to `generativelanguage.googleapis.com`, or `GOOGLE_GEMINI_BASE_URL` | `HTTPS_PROXY` (global undici agent), `NO_PROXY`; `NODE_EXTRA_CA_CERTS` in the launch env only (H8.3) | `.gemini/tmp/<project>/chats/session-*.jsonl`; subagents `chats/<parent>/<id>.jsonl` (H8.5) | Settings only: `general.enableAutoUpdate: false`; no env switch (H8.6) | Paid Gemini API key, Agent Platform, Google sign-in (no longer for free users) | `--sandbox` sets its own proxy for tools; remote A2A subagents (H8.3) | **Likely verified** with the funded Google key |
| 9 | Goose v1.57.0 (Rust, rustls) | HTTPS to the provider; DeepSeek through a custom OpenAI-format provider | `HTTPS_PROXY`, `NO_PROXY`; `GOOSE_CA_CERT_PATH` adds; `SSL_CERT_FILE` replaces, so the full bundle (H10.3) | `~/.local/share/goose/sessions/sessions.db`; subagents are rows with `session_type = 'sub_agent'` (H10.5) | Does not self-update (H10.6) | Keys from env, keyring, or `secrets.yaml` | None found for model calls (H10.3) | **Likely verified** |
| 10 | Kilo CLI 7.8.25 (Bun, OpenCode fork) | HTTPS to the provider (native DeepSeek), Kilo's gateway, or Vercel AI Gateway | `HTTPS_PROXY`; OS store (`--use-system-ca`) plus `NODE_EXTRA_CA_CERTS` (H12.3) | `~/.local/share/kilo/kilo.db`; subagents are child sessions (H12.5) | `KILO_DISABLE_AUTOUPDATE=1`; silently installs patches otherwise (H12.6) | Key in env or `auth.json`, Kilo account for its gateway | None found (H12.3) | **Likely verified** |
| 11 | Qwen Code 0.26.4 (Node 22+) | Chat Completions to the provider (native DeepSeek preset) | `--proxy`, then settings `proxy`, then `HTTPS_PROXY`; `NODE_EXTRA_CA_CERTS` (H13.3) | `projects/<cwd>/chats/<id>.jsonl`; subagents `subagents/<id>/agent-<id>.jsonl` plus `.meta.json` (H13.5) | Settings only: `general.enableAutoUpdate: false` (H13.6) | Keys in env or `settings.json`; Qwen sign-in ended 15 Apr 2026 | Settings `proxy` and `--proxy` beat env; built-in `claude-code` and `codex` subagents start other harnesses (H13.3) | **Likely verified** |
| 12 | Crush 0.99.0 (Go) | Chat Completions to the provider (native DeepSeek) | `HTTPS_PROXY` (Go default transport); `SSL_CERT_FILE` replaces, so the full bundle (H14.3) | `<workspace>/.crush/crush.db`; subagents are child session rows (H14.5) | Binary does not self-update (H14.6) | Key in env or pasted (stored in `crush.json`), Charm Hyper | `CRUSH_CLIENT_SERVER=1` with an older server (H14.3) | **Likely verified.** Its subagents are read-only search agents |
| 13 | Factory Droid 0.251.0 (Bun) | BYOK to `baseUrl`; Factory backend `api.factory.ai` for auth, telemetry and cloud session sync | `HTTPS_PROXY`, `NO_PROXY` only (no OS proxy); `NODE_EXTRA_CA_CERTS` or `FACTORY_EXTRA_CA_CERTS` (H15.3) | `~/.factory/sessions/<slug>/<uuid>.jsonl` plus `.settings.json`; subagent layout unverified (H15.5) | `FACTORY_DROID_AUTO_UPDATE_ENABLED=false` (H15.6) | Factory account always (browser or `FACTORY_API_KEY`), plus BYOK | A separately started `droid daemon` or desktop app; cloud Droid Computers (H15.3) | **Risky, not verified this cycle** (no account); BYOK subagents reported broken (issue 691) |
| 14 | Kimi Code 2.4.0 (Node 22.19+) | HTTPS to `api.kimi.com` after sign-in, or a provider in `config.toml` | `HTTPS_PROXY`, `ALL_PROXY`, `NO_PROXY`; loopback always bypassed ([env vars](https://moonshotai.github.io/kimi-code/en/configuration/env-vars)); CA through `NODE_EXTRA_CA_CERTS` (Node standard, not documented, unverified) | `sessions/<workspace>/<session>/agents/main/wire.jsonl`; one `agents/agent-N/wire.jsonl` per subagent; `session_index.jsonl` ([data locations](https://moonshotai.github.io/kimi-code/en/configuration/data-locations)) | `KIMI_CODE_NO_AUTO_UPDATE=1` (env vars page) | Kimi OAuth, or provider keys through `api_key_env` (shell keys are not read otherwise) | None found | **Likely verified** |
| 15 | Cline CLI 3.0.74 (Bun) | HTTPS to the provider (native DeepSeek), or Cline's own provider | `https_proxy`, `http_proxy`, `no_proxy`; `NODE_EXTRA_CA_CERTS` only ([networking](https://docs.cline.bot/troubleshooting/networking-and-proxies)) | `data/sessions/<id>/`, index `data/db/sessions.db` ([CLI reference](https://docs.cline.bot/cli/cli-reference)) | `CLINE_NO_AUTO_UPDATE=1` inferred from the binary, unverified; release notes say updates install on exit (cli-v3.0.55) | Provider keys via `cline auth` (stored in `data/settings/providers.json`), Cline account | A hub daemon on port 25463 may already run with another env; `CLINE_SESSION_BACKEND_MODE=local` should keep sessions in process (unverified) | **Risky** (hub daemon) |
| | Cursor CLI (out of the 15) | Connect RPC over HTTP/2 to `*.cursor.sh`; inference on Cursor's servers | Listed as a vendor backend; HTTP/2 is now supported (PRD-03) | `~/.cursor/chats/...` | `--disable-auto-update` | Cursor account | IDE ignores proxy with HTTP/2 on (Cursor forum, 29 Oct 2026) | **Documented dodger case.** No connector |

Versions: npm `time` fields and tags fetched 29 Oct 2026 (appendix O), and for Cline and Kimi Code
the npm registry on 30 Oct 2026: `cline` `latest` 3.0.74 (26 Oct), `@moonshot-ai/kimi-code`
`latest` 2.4.0 (28 Oct).

What "auto-update off" can and cannot do: where a harness has only a settings key (Hermes, Gemini CLI,
Qwen Code), goalcup does not write into the harness's settings. A running process keeps its binary;
an update only matters at the next `goalcup continue`, and the `start` event records the version at
every start (appendix N, Q7). PRD-10 tells the participant in one line.

## Files it owns

Create:

- `connectors/model-hosts.toml`; `connectors/<id>.toml` for the 16 files in section 6.
- `crates/goalcup/src/connector/mod.rs` (types, `plan`), `schema.rs` (serde structs, checks, about 150
  lines), `builtin.rs` (the `include_str!` list), `hosts.rs` (`HostList`, matching, about 150),
  `hints.rs` (file key lookup in JSON, TOML, YAML, dotenv; env suffix scan, about 220, YAML copied from
  `model_host.rs:682` onward), `launch.rs` (env build, homes, lock, version probe, secret files, about
  200). About 750 src lines in all.
- `crates/goalcup/tests/connector_*.rs` (section Acceptance).
- `docs/harnesses/<id>.md` for each harness: one page from section 7's row plus the pad recipe
  (appendix O section .7 of each harness). The OpenCode and OpenClaw sheets are copied from branch
  `origin/opencode-openclaw` and updated; the old sheets for Amp, Claude Code, Codex and Hermes are
  rewritten to the new shape.

Change: `docs/harnesses/README.md` (the table).

Delete (with PRD-13, which removes the old crates): `connectors/*/harness.toml` (four folders),
`crates/goalcup-protocol/providers.toml`, `crates/goalcup-protocol/pins/`, `scripts/pin-manifests.sh`.

Must not touch: `src/proxy/` (PRD-03, which owns the `decide` call and the CA), `src/native/` (PRD-04),
`src/mask/` (PRD-05), `src/pipeline/` and `src/watchdog.rs` (PRD-02), `src/cli/` and `src/main.rs`
(PRD-10), `scripts/pad/**` (PRD-12), `docs/*.md` (PRD-15, which also rewrites
`docs/CONNECTOR-SCHEMA.md` from section 2).

## Interfaces with other PRDs

| PRD | Uses or gives | Seam |
|---|---|---|
| PRD-02 | Spawns from `LaunchPlan` after `finish_env(port, run_ca, bundle)`; runs `before` before spawn and `after` after harness exit and before native finish (PRD-04 ask 3) | S3 |
| PRD-03 | `HostList::decide(sni) -> Listed { source, kind, pinned } \| SignIn \| NotListed`, `ca_subtrees()`. Its asks are met: `HTTPS_PROXY` without credential, `NO_PROXY` with `::1`, `NODE_USE_ENV_PROXY=1`, extend variables to `run-ca.pem`, `SSL_CERT_FILE` and `REQUESTS_CA_BUNDLE` to `bundle.pem`. PRD-03 writes both PEM files (it owns `ca_bundle.rs` logic) | S3 |
| PRD-04 | `NativeSources { rows: Vec<Row { name, root: PathBuf, pattern, form }>, since }`, roots absolute, `{home}` and `{workspace}` resolved. Rows in section 6 | S3 |
| PRD-05 | `mask_rules: Vec<MaskRule>` (PRD-05 grammar, no `object`); `env` (credential-named values) and `secret_values` (from `secret_files`) as exact values. Codex rate-limit, plan and credit rows are gone (`connectors/codex-cli/harness.toml:110-113`) | S3 |
| PRD-07 | `connector_sha256` in the start facts | data |
| PRD-08 | Connector id and the three host lists in the `start` event decide the harness reader and the "listed hosts decrypted" check | data |
| PRD-09 | `kind` per host (`provider`, `gateway`, `vendor`, and `other` for participant and config hosts); connector ids and native row names for its hint table | data |
| PRD-10 | Calls `connector::plan(id_or_path, ctx)`; passes participant hosts from `goalcup setup --record-host`; shows `refusal`; `goalcup setup <id>` opens the harness in `{home}` so the participant signs in or configures a gateway there (for example the Vercel CLI writes `~/.config/kilo/kilo.json` in the real home; the participant must make that change inside goalcup's home). Flags `--connector` and `--hosts` | S3 |
| PRD-12 | A connector plus a key is the pad's unit; the dodger case uses `--hosts` with a `pinned` copy. The pad recipe per harness is appendix O section .7, with the paths of section 6 | data |
| PRD-15 | Writes `docs/CONNECTOR-SCHEMA.md` (a page, not 1,002 lines) from section 2, and the participant guide lines on harness limits from section 7 | docs |

## What is cut

| Path | Lines | Note |
|---|---:|---|
| `connectors/*/harness.toml` | 556 data | Replaced by about 330 lines in 16 files plus 30 in `model-hosts.toml` |
| `crates/goalcup-protocol/src/harness/{schema,check,load,parse}.rs`, `harness.rs` | 3,370 src | Replaced by about 370 lines (`schema.rs`, `builtin.rs`, `hosts.rs`) |
| `crates/goalcup-protocol/src/pins.rs`, `pins/connector-dirs.txt`, `scripts/pin-manifests.sh` | 1,031 | Pins gone; the start event's SHA-256 replaces them |
| `crates/goalcup-protocol/providers.toml` | 25 | Into `model-hosts.toml` |
| `crates/goalcup-cli/src/model_host.rs` | 1,540 src | About 220 in `hints.rs`. The refusals at start go: a hint never refuses |
| `crates/goalcup-cli/src/connector.rs`, `home.rs`, `per_run_home.rs`, `harness_version.rs` | 1,854 src | About 200 in `launch.rs` |
| Fields with no successor | | `participant.*` except the name, `time_left_tool`, `no_prompts`, `first_screen`, `setup_section`, `capture.identity`, `termination`, `argv_guard`, `platform`, `reserved_env_prefixes`, `configure`, `launch_prep`, `auth_hosts` (now `never`), `capture.hosts` (now the shared list), `model_host_from.via`/`absent`, `protocol_hint`, `since_run_start`, `warn_missing`, CA `mode`, `seed`, `real_dirs`, `skills_dir` |
| Hermes workarounds | data | The `providers.recorder` entry with `base_url` (it set a model base URL), `HERMES_CA_BUNDLE`, the two `configure` commands, the `session_*.json` row (`connectors/hermes-cli/harness.toml:54-57, 88-92, 103-109, 145-152`) |
| Tests | 10,971 | Protocol `harness_schema` 4,451, `harness_loader` 650, `harness_conformance` 379, `connector_pins` 303, `connector_template` 120, `pins_build_fails` 86; CLI `connector_contract` 994, `harness_contract_golden` 975, `harness_contract_kit` 805, `harness_launch_plan` 782, `model_host_from` 793, `per_run_home` 438, `harness_version_probe` 195. PRD-13 deletes them with the crates |

Net: about 8,400 src and data lines and about 11,000 test lines go; about 750 src and 360 data lines
come back.

## Acceptance

Real-run proofs first (run on the pad, PRD-12; a watcher checks them):

1. **Direct key.** Claude Code (Claude key), Codex (OpenAI key) and Hermes (DeepSeek) each run the
   pad prompt from their connector. Pass: every model request the harness's own session file records
   has a matching decrypted `http_req` or `ws_up` message in the record; zero connections to a listed
   host are tunnels; the start event shows the connector SHA-256 of the built-in file.
2. **Gateway.** Claude Code and Kilo with `ai-gateway.vercel.sh` set in their config inside `{home}`
   (the pad sets it as a participant would; goalcup never does). Pass: same checks as 1; the gateway
   host is `listed: "shared"`; the call bytes are byte-for-byte the same shape as a direct call.
3. **Unknown gateway by hint.** Codex with a `[model_providers.x]` table whose `base_url` host is not on
   the shared list (for example a test gateway the pad controls). Pass: that host is `listed: "config"`
   and decrypted.
4. **Dodger.** Qwen Code with `"proxy": "http://127.0.0.1:9"` in `{home}/settings.json`. Pass: start
   refuses before launch, names the file and the key, and never prints the value. Then Claude Code
   with the pad's `--hosts` copy marking `api.anthropic.com` pinned (PRD-03): the harness still works,
   every connection to it is a tunnel with GAP `pinned`.
5. **Daemons.** Hermes with a gateway left running from a previous start (started without our env).
   Pass: `before` stops it; the run's model calls are decrypted; after exit no `hermes` process is left.
6. **The other 12.** Each runs once with the pad prompt. Pass or fail goes into the PRD-12 table with
   the harness version, and the row in `docs/harnesses/README.md` says "verified on <version>" or "not
   verified".
7. **No secret.** For each run, the DeepSeek, Claude, OpenAI, Google and Factory keys, and every value
   in the connector's `secret_files`, appear nowhere in the record or the server copy.

Automated tests that guard them (`crates/goalcup/tests/`):

- `connector_builtin.rs`: every file in `connectors/` parses, is in the built-in list, has at most 20
  lines of keys, and its `native` patterns pass PRD-04's pattern check. One test, all files.
- `connector_hosts.rs`: exact, `*.suffix` and in-label `*` matching; `never` beats every source; a
  participant `never` host is dropped; `ca_subtrees` for each pattern shape.
- `connector_hints.rs`: one key from each of JSON, TOML, YAML and dotenv, with `*` over keys and array
  indexes; `http`, loopback and IP values skipped; the env suffix scan; a 2 MiB file skipped.
- `connector_launch.rs`: the participant's `OPENAI_API_KEY` passes through; their `HTTPS_PROXY`,
  `HTTP_PROXY`, `ALL_PROXY`, `NO_PROXY` and CA variables are replaced; `unset` removes; the exact
  default set from section 3 is present after `finish_env`; `refuse_if_set` names file and key and
  never the value; a second plan on a locked home fails.

## Risks and blind spots

| Risk | What could go wrong | What we do |
|---|---|---|
| Whole-environment passthrough | A participant's own `ANTHROPIC_BASE_URL` or cloud switch sends calls to a host we did not expect | That is how they run it (point 7); the env hint scan lists it; an unknown one is a flagged tunnel |
| XDG homes | The agent's own tools that read `XDG_CONFIG_HOME` (for example `gh`) lose their login during OpenCode, Kilo, Goose and Crush runs | `GH_TOKEN` passes through; the guide says so (PRD-10). A narrower variable per harness, where one exists, is a later cleanup |
| `HOME` swap for Amp and Droid | git and ssh config of the participant are not visible to the agent | Said plainly at start (PRD-10); unverified whether Droid has a home variable (the builder checks `strings` for one) |
| `SSL_CERT_FILE` for every harness | A tool that trusted a corporate CA only through the participant's own `SSL_CERT_FILE` now fails on tunnelled hosts | The bundle holds the OS store; a corporate proxy chain is not supported in 1.0 (PRD-03 scope). Named in the guide |
| `refuse_if_set` | A participant cannot start until they remove a setting | It is before launch and names the fix; a run that records nothing is worth nothing (appendix N, Q8) |
| Host list rows from memory | `api.cline.bot`, `opencode.ai`, the `githubcopilot.com` pair, Qwen and Mistral hosts may be wrong or incomplete | Marked unverified; checked against vendor docs before the build; a wrong row only costs a tunnel, never a broken harness (PRD-03 refusal learning) |
| Decrypting `opencode.ai` | It also serves the install script and docs | Clients that read the bundle verify fine; a client that refuses our CA falls back to a tunnel after one failure (PRD-03) |
| Release pace | Amp about 7 builds a day; Copilot and Droid about daily; Kilo several a day | Versions recorded at every start; auto-update off where possible; the pad table names the version that passed |
| Secret files change during a run | A refreshed OAuth token is not in `secret_values` | Shape rules still apply (PRD-05); the pad's no-secret check covers the keys it set |
| OpenCode 2 under the 1.x connector | `version` refuses and points to `opencode2-cli` | One extra key in the schema; the alternative was a silent zero-call run |
| Hermes gateway with an old env | If `before` fails (old Hermes has no `-y`), the old gateway calls direct, not even tunnelled | Logged; the pad's check 5; tier stays "not verified" until it passes |
| macOS | Goose uses Apple folders, not XDG; `SSL_CERT_FILE` is ignored by the Security framework | Linux first; the owner's Mac run checks Goose and Hermes |

## Overridable defaults

| Default | Value | Who may change it |
|---|---|---|
| Environment | Participant's whole env, minus proxy and CA variables | Owner |
| Env hint suffixes | `_BASE_URL`, `_API_BASE`, `_ENDPOINT`, `_HOST`, `https` only | PRD-11 |
| `before`/`after` timeout | 10 s each | PRD-11 |
| Version probe | `--version`, 5 s | PRD-11 |
| Secret file value length | 16 bytes or more | PRD-05 |
| Hint file size guard | 1 MiB | PRD-11 |
| `CLAUDE_CODE_DISABLE_AGENT_VIEW=1` | on | Owner (it turns a feature off) |
| `pinned` list | empty | PRD-12 for the pad only |

## Builder notes

Order of work:

1. H+0 to H+1.5: publish `model-hosts.toml`, `schema.rs` and the S3 types with `plan` as a stub that
   returns the Claude Code plan. Others build against it.
2. `hosts.rs` and its test, then `hints.rs`, then `launch.rs`.
3. Write the 16 files. Before each, open the vendor's current docs or `--help` of the pinned version
   and check every value marked unverified here: `codex --no-daemon` with `resume`, `OPENCLAW_HOME`
   layout, `GEMINI_CLI_HOME` layout (does it hold `.gemini/`?), `QWEN_HOME` layout, Copilot and Qwen
   `--continue`, Goose `session --resume`, Cline's hub with `CLINE_SESSION_BACKEND_MODE=local`, Droid's
   home variable and daemon stop command, Codex's state database file name.
4. `docs/harnesses/*.md` last, from what the pad shows.

Traps from the audit:

- Never pass the run CA alone in `SSL_CERT_FILE`: Python, Go and rustls stacks then fail every tunnel
  (appendix O.1; `ca_bundle.rs:17-19`).
- Codex reads `$CODEX_HOME/.env` after our environment and overwrites it (H2.3): `refuse_if_set` is the
  only guard.
- OpenCode and Kilo need `NO_PROXY=localhost,127.0.0.1` or the TUI's own server loops through the proxy.
- OpenCode and Kilo fall back to a paid gateway model on an unknown model id (facts sheet section 6):
  the pad sets a valid id and `small_model`.
- Copilot with DeepSeek must use the `anthropic` provider type; `openai` gives a 400 on the second turn
  (H9.2).
- Crush's subagent runs only for search work; Goose's only in auto mode, 5 at a time, 5-minute
  timeout; Hermes one-shot mode caps children at 2. These are the harnesses' caps, not ours.
- Never set a model base URL (`AGENTS.md:81`). The Hermes `providers.recorder` entry is the example of
  what not to rebuild.

How a watcher proves it in a real run: run `goalcup-verify check` on the entry; read the `start` event
for `connector_sha256` and the three host lists; list `conn` streams to any `shared` host with `mode:
"tunnel"` (must be zero outside the dodger case); compare the harness's own session files (native
streams) with the wire requests, per subagent.

## Asks of PRD-00 and PRD-01

PRD-00:

1. **One file per harness, not one folder.** The ownership table says "one connector folder per
   harness". Flat `connectors/<id>.toml` is less; there are no seed files any more.
2. **S3 shape.** Write S3 as `connector::{plan, LaunchPlan, HostList, NativeSources, MaskRule}`, with
   `LaunchPlan::finish_env(port, run_ca, bundle)`, and `HostList::decide` returning `kind` as well as
   `source` and `pinned`.
3. **PRD-02 section 8 step 5** still says `HTTPS_PROXY` "with the fixed port and credential" and step 3
   sends "the proxy credential". PRD-03 design point 5 removes the credential. The launch env here has
   none; PRD-02 should drop it.
4. **The 16th file.** OpenCode has two connectors (major versions 1 and 2). The pad table should count
   OpenCode once, with both versions in its row.
5. **Owner default to confirm:** the harness gets the participant's whole environment (minus proxy and
   CA variables), not today's allowlist (`crates/goalcup-cli/src/launch.rs:320-333`). Without it, a
   key exported in the shell does not reach Crush, Kilo, Goose or Copilot.

PRD-01:

1. **`start` event key `connector_source`** (`builtin` or `file`). With pins gone, it is how a reader
   tells a release connector from a contributor's file without comparing hashes by hand.
2. **Host kinds in the `start` event.** `meta.listed` stays `shared`, `participant` or `config`. The
   `start` event's `hosts` should also carry each shared entry's `kind` (`provider`, `gateway`,
   `vendor`), so PRD-09 reads kinds from the record instead of embedding the host list at build time,
   and a run made with a `--hosts` file is read with its own kinds.
