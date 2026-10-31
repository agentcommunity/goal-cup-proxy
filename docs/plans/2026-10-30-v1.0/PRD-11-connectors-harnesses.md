# PRD-11: Connectors and the 15 harnesses

Id: PRD-11. Version: goalcup 1.0. Date: 30 October 2026. Base: main at 4b226de2. Revision 2 (after
the three reviews of 30 Oct; answers at the end). Revision 3, 30 October 2026: alignment fixes against
the ledger (section "Alignment answers" at the end).
Index: [PRD-00](PRD-00-index.md). Format: [PRD-01](PRD-01-record-format.md).
Facts as of 29 Oct 2026 unless a line says otherwise.

## Goal

One small data file per harness, embedded in the binary, plus one shared list of model hosts. Every
harness goes through the same proxy, the same CA rule and the same session-file reader; only data
differs.

## Points served

7 (lead), 1 (the host list decides what is decrypted), 2 (session-file rows find every subagent),
3 (nothing here can stop a running harness), 4 (sign-in hosts never decrypted, key files never
snapshotted; key file values fed to masking only with the owner's yes to PRD-00 question 3).

## Owner decisions it carries out

- One system for every harness, no patchwork. One proxy, one shared host list (providers, gateways,
  cloud model services, vendor backends), plus hosts the participant registers, plus hosts the
  harness config names as a hint (BRIEF3, 29 Oct night).
- Per harness only data: CA variables, the harness's own proxy setting where it beats `HTTPS_PROXY`,
  daemons to stop, session-file patterns, the auto-update switch.
- Simplified connector schema: about 10 to 25 lines, `**` patterns, defaults in code, embedded in the
  binary, pins gone.
- PRD-00 revision 2 rulings this PRD follows: seam S3 (final shape), S16 (`--connector` and `--hosts`
  only, refused in competition mode), flat files, tiers live in `connectors/tiers.toml` (PRD-12),
  host hints from config files and named variables only, the watchdog runs `before` and `after` (S8),
  PRD-05 owns the gate on secret values.
- Owner questions with defaults taken (PRD-00): owner question 2 (old 8): the auto-update switch
  (`update_off`) is always written into goalcup's own isolated home, with no question (ledger H9), and
  the no-prompt values after one question; owner question 2 (old 13): the harness gets the participant's
  whole environment; owner question 4 (old 14): Qwen Code's harness-starting subagents are named in
  RULES; owner question 4 (old 15): file connectors refused in competition mode; owner question 3
  (old 12): exact-value masking is not built unless the owner says yes, so `secret_files` is not read
  in the default build (ledger M1).
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
| Dodger | A harness that ignores `HTTPS_PROXY`, refuses our CA, or a setting that routes around us | A tunnel record when its bytes pass us, its session files, a named gap, the words "not recorded" on the start line; the tier is "not supported" only when the harness's `direct` case looks like this (PRD-12 section 9) | dodger |

Three things look like gateways but are not, and the list handles them as data:

- **Vendor backends** (Amp, Factory's `api.factory.ai` (auth, telemetry and session sync; BYOK model
  calls go to `baseUrl`), Copilot's GitHub-hosted models, Gemini CLI on
  Google sign-in through `cloudcode-pa.googleapis.com`, Cursor). The harness talks to its own vendor,
  and the vendor's servers call the model. We decrypt and record the harness-to-vendor bytes in full.
  The vendor-to-provider call never touches the participant's machine, so it is out of reach for any
  recorder. That is a fact about where inference runs, not a proxy gap.
- **A gateway we do not know.** A self-hosted LiteLLM or a new gateway host. If the harness config
  or one of the connector's named base-URL variables names it, the hint adds it. If not, the
  participant registers it once (`goalcup setup --record-host <host>`, PRD-10). Until then it is a
  tunnel, and the per-host flag (PRD-03) and the live wire-against-native line (PRD-10) make it
  visible, so it is never silent.
- **A model server on this computer** (Ollama, LM Studio, vLLM on loopback). `NO_PROXY` keeps loopback
  off the proxy, so it is not recorded. When a hint names a loopback or private address, the plan
  carries a `local_model` note (PRD-01 section 11) and PRD-10 says it in one line. We name it; we do
  not build around it (that would need `HTTP_PROXY`, which PRD-03 refuses on purpose).

So the strong pads are `HTTPS_PROXY` plus gateway (full wire record), and dodgers ride the same light
pipeline with an honest label. No second system. The owner's later words, "dodgers get to use current
in a way", are exactly this: the same proxy, session files and a named gap, no extra code per dodger.

## Today

| What | Where | Lines | Fact |
|---|---|---:|---|
| Four connectors | `connectors/{amp,claude-code,codex,hermes}-cli/harness.toml` | 556 data | 115 to 159 lines each, about 60% comments |
| Schema, checker, loader, parser | `crates/goalcup-protocol/src/harness/{schema,check,load,parse}.rs`, `harness.rs` | 3,370 src | `deny_unknown_fields` structs (`schema.rs:214-1066`); `MAX_NATIVE_ROWS = 8` and no `**` (`check.rs:26`, `:424-462`) |
| Pins | `crates/goalcup-protocol/src/pins.rs`, `pins/connector-dirs.txt`, `scripts/pin-manifests.sh` | 750 + 7 + 274 | Every connector edit regenerates pins or every start refuses (`AGENTS.md:90-91`; `pins.rs:32`) |
| Frozen provider table | `crates/goalcup-protocol/providers.toml` | 25 | Two rows, frozen by D74 (`providers.toml:10`) |
| Config address reader | `crates/goalcup-cli/src/model_host.rs` | 1,540 src | One key per connector; no address means only the connector's hosts are decrypted (`model_host.rs:1-8`); refuses plain http, loopback, IP literals (`:16-24`); a small YAML lookup (`:682`) |
| CA bundle | `crates/goalcup-cli/src/ca_bundle.rs` | 207 src | `SSL_CERT_FILE` and `REQUESTS_CA_BUNDLE` get OS roots plus the run CA; a harness's own variable gets the CA alone (`ca_bundle.rs:17-19`; `capture_policy.rs:68`) |
| Launch env | `crates/goalcup-cli/src/launch.rs:320-333` | part of 2,090 | An allowlist: only `PATH`, `TERM`, `COLORTERM`, `LANG`, `SHELL`, `USER`, `LOGNAME`, `TMPDIR` and every `LC_*` pass. A `DEEPSEEK_API_KEY` exported in the shell never reaches the harness |
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
connectors/<id>.toml               one connector per harness, about 10 to 25 lines
connectors/tiers.toml              not this PRD: written by PRD-12's pad table, read by PRD-10
docs/harnesses/<id>.md             one facts sheet per harness (facts and limits only; the table in section 7 is the seed)
crates/goalcup/src/connector/      schema, loader, host list and host check, hints, launch plan (about 800 src lines)
```

All `connectors/<id>.toml` files and `model-hosts.toml` are compiled in with `include_str!` through one
list: `connector/builtin.rs` includes a list generated by `build.rs` from `connectors/*.toml` (about 15
lines), a `const` array of `(id, bytes)`, so no Rust file names a harness and a contributor adds only
the TOML file. The binary needs no connectors folder.
There are no pins. Instead, the `start` event (PRD-01 section 11) carries `connector`,
`connector_source`, `connector_sha256` (SHA-256 of the exact TOML bytes used), `harness_version`,
`hosts` (the three lists in force, each entry with its `kind` and `pinned` mark) and `hosts_sha256`
(SHA-256 of the host list file as read). Anything that differs from the release is visible in the
record. That is the whole integrity story.

A contributor or the pad loads files from disk with `goalcup start --connector <path.toml>` and
`--hosts <path.toml>` (seam S16; PRD-10 owns the flags, this PRD the loading). Same parser, same
checks; the start event then says `connector_source: "file"`. Both flags are refused in competition
mode (owner question 4, old 15). There is no `GOALCUP_CONNECTORS_DIR`, no dev mode and no origin classes.

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
| `host_hints` | list of `{file, key}` | none | Keys in the harness's config files whose URL values add hosts to the `config` list |
| `hint_env` | string list | none | Named participant variables whose URL values add hosts to the `config` list (for example `ANTHROPIC_BASE_URL`). Only the names listed; there is no scan of every `*_HOST` or `*_ENDPOINT` variable |
| `secret_files` | string list | none | The harness's own key files. Default build: not read for masking (owner question 3, old 12, default no; ledger M1); the connector test uses them to refuse a native row that would snapshot one. Only with the owner's yes (ledger M1a): their string values go to PRD-05 as exact-value candidates, with their key paths |
| `before`, `after` | list of argv | none | Run with `program` by the watchdog (PRD-02, S8) before launch and after exit: stop background daemons. 10 s each, failure only logged |
| `native` | list of `{name, root, pattern, form, warn_missing}` | `form = "append"`, `warn_missing = true` | Session-file rows (PRD-04). Root templates `{home}`, `{workspace}`; `**` allowed. `warn_missing = false` silences PRD-10's "no session file yet" line for a row that may legitimately stay empty |
| `mask` | list of `MaskRule` | none | Connector mask rules (PRD-05 grammar: `json_field` with `string` or `scalar`, or `header_prefix`) |
| `first_screen` | string | none | One line PRD-10 prints just before the harness opens (a first-run question to expect) |
| `no_prompts` | list of `{file, key, value}` | none | Settings `goalcup setup` writes into the isolated home after its one question, so the harness never stops to ask approval (owner question 2, old 8; PRD-10 section 7, S15) |
| `update_off` | list of `{file, key, value}` | none | The auto-update switch where it is only a setting (Hermes, Gemini CLI, Qwen Code, Codex). Always written into the isolated home, with no question (ledger H9; owner question 2, old 8): by `goalcup setup` with or without the no-prompt answer and with `--yes` (S15), and again by `plan` at every `goalcup start` and `continue`, before launch, so a participant who skipped setup or a harness that rewrote the file is still covered. It is not part of the `no_prompts` question. Where an env switch exists it goes in `env` instead |
| `skills_dir` | template | none | Where PRD-10 installs the goal-cup skill (`{home}/skills`). Only a path under `{home}`; the loader refuses `{workspace}` here (nothing is added to the participant's folder). Absent: no skill is installed |

Rules the loader checks, and nothing else: types; ids unique; native row names unique within a
connector (PRD-04: rows may overlap, and a file is read once, by the first row in file order that
matches its absolute path); templates known; patterns relative with no `..`, no absolute path, `**`
only as a whole component (PRD-04 section 2); `file` paths start with `{home}` or `{workspace}`, and
`skills_dir` with `{home}`; no `HTTPS_PROXY`, `NO_PROXY`, CA or `HOME`-style variable in `env` that the
code sets itself (those go in `home` or `ca_env`). No row caps, no deny lists, no harness-name checks.
The tier is not connector data: it comes from `connectors/tiers.toml` (PRD-12).

**No model address in connector data** (hard rule: never set a model base URL). `load` refuses a
connector whose `env` names match, case-insensitive, `BASE_URL`, `API_BASE`, `ENDPOINT`, `_HOST` or
`PROXY`, or whose `env`, `no_prompts` or `update_off` values contain `://`. Error: the connector sets a
model address, which goalcup never does. (`NODE_USE_ENV_PROXY` is a code default in section 3 step 4,
not connector `env`, so it is not hit.)

File keys use dots; `*` matches any one key or array index; a segment may contain `*` as a glob
(`*_BASE_URL`). Formats by extension: `.json`, `.toml`, `.yaml` or `.yml` (the block lookup copied
from `model_host.rs:682`), anything else is read as dotenv (`KEY=value` lines). A `file` may hold a
`*` in its last component (`custom_providers/*.json`). A file is opened without following a link
(`O_NOFOLLOW`) and read as UTF-8 with a BOM stripped (PRD-17 N7). `model_providers.*.base_url`
replaces the old `via` indirection: every provider table's host is hinted, selected or not. Files over
1 MiB, linked, unreadable and unparsable files are skipped with a log line and one start line (PRD-10):
"goalcup could not read <file>; a model service named there may not be recorded." (a memory guard,
never a refusal).

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

goalcup never sets `SSL_CERT_DIR`, `CURL_CA_BUNDLE` or `NODE_TLS_REJECT_UNAUTHORIZED` (PRD-17 T6, D43);
a value the participant exported passes through untouched. The participant's key variables pass
through as well, which is what Codex with `env_key` behind a gateway needs (PRD-17 P9).

`program --version` is probed once with that environment minus the proxy variables, 5 s limit, first
line kept (copied from `crates/goalcup-cli/src/harness_version.rs:93-153, 191`). Its output goes in the
`start` event and is checked against `version` when set.

**The isolated home.** `{home}` is `$GOALCUP_HOME/homes/<id>/`, one per harness, kept across runs so a
login made in `goalcup setup` lasts (D39). It is locked with `flock` while a run uses it; a second
start on the same harness refuses (Codex refresh tokens rotate, so two runs on one home race,
PRD-17 HF4). The lock file is `$GOALCUP_HOME/homes/<id>.goalcup.lock`, a name no harness uses
(`<connector>.lock` was Claude Code's own token-refresh lock, PRD-17 P12). Where `home` sets `HOME`
itself (Amp, Droid), the plan writes `{home}/.profile` that puts the handed `PATH` back, written beside
and renamed over, never through a link, so a login shell the agent runs still finds `goalcup`
(PRD-17 P11, copied from `crates/goalcup-cli/src/home.rs:591-620`). goalcup never writes into the
participant's real harness folders (`AGENTS.md`, "Never write into `~/.claude`..."), and the native
reader never sees the participant's older private sessions in a shared SQLite file. Four shapes cover all 15:

| Shape | Harnesses | `home` |
|---|---|---|
| The harness's own home variable | Claude Code, Codex, Hermes, OpenClaw, Copilot, Gemini, Qwen, Kimi, Cline | one variable to `{home}` |
| XDG folders | OpenCode, Kilo, Goose, Crush | `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_STATE_HOME`, `XDG_CACHE_HOME` under `{home}` |
| `HOME` itself | Amp, Factory Droid | `HOME = "{home}"` (no known narrower variable) |
| Workspace | Crush's database | `{workspace}/.crush`, read as a native root |

**Host hints.** Two sources only (PRD-00 S3): the connector's `host_hints` keys in config files, and
the participant variables the connector names in `hint_env` (for example `ANTHROPIC_BASE_URL`,
`OPENAI_BASE_URL`, `GOOGLE_GEMINI_BASE_URL`, `CODE_ASSIST_ENDPOINT`, `COPILOT_PROVIDER_BASE_URL`,
`KIMI_MODEL_BASE_URL`). There is no scan of every `*_HOST` or `*_ENDPOINT` variable: a database host
would be decrypted and break the tool that uses it (point 3). Each value goes through the one host
check (section 4). A hint is never a refusal:

- an `https` URL whose host passes the check adds that host to the `config` list;
- a URL whose host is loopback, an unspecified or mapped loopback address, a private address, or a
  name that resolves only to this computer (PRD-17 N2, N3, N6) adds one `local_model` note
  (`{"what":"local_model","host":"<host:port>"}`) to `LaunchPlan.notes`; it is not recorded, and
  PRD-10 says so in one line;
- anything else (plain `http` to a public host, an IP literal, a backslash or control byte in the
  authority, a `never` host, goalcup's own server) is skipped with one log line naming the key and the
  reason, never the value.

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
  "cloudcode-pa.googleapis.com", "api2.cursor.sh", "*.api5.cursor.sh",
]
never = ["auth.openai.com", "platform.claude.com", "authapi.ampcode.com", "oauth2.googleapis.com"]
pinned = []
```

- This file is the one list (PRD-03 section 3 keeps only the matching rules and points here). Rows
  beyond PRD-03's first list: Qwen's and Mistral's own APIs, Copilot's GitHub-hosted model hosts,
  Gemini's Code Assist host, Cline's and OpenCode's own gateways. Checked on 30 Oct 2026:
  `api.cline.bot` (Cline's OpenAI-compatible API at `https://api.cline.bot/api/v1`,
  https://docs.cline.bot/api/sdk-examples.md); `opencode.ai` (OpenCode Zen at
  `https://opencode.ai/zen/v1`, https://opencode.ai/docs/zen); `*.githubcopilot.com` (GitHub's Copilot
  allowlist reference lists `*.githubcopilot.com` and per-plan subdomains such as
  `*.individual.githubcopilot.com`,
  https://docs.github.com/en/enterprise-cloud@latest/copilot/reference/copilot-allowlist-reference);
  `cloudcode-pa.googleapis.com` (Gemini CLI's default `CODE_ASSIST_ENDPOINT` for Google sign-in,
  `packages/core/src/code_assist/server.ts` in the gemini-cli repository, read 30 Oct 2026). Every
  other added row stays "unverified" until a vendor doc or a pad run confirms it; the builder checks
  each against the vendor's current docs (ledger T7).
- `kind` for PRD-09 is the array name. Hosts from the participant or a config hint get kind `other`.
- `never` wins over every pattern and every source. A registration or hint naming a `never` host is
  dropped with a log line. goalcup's own server (`agentcommunity.org` and its subdomains, and any name
  holding `goalcup`, PRD-17 N5) is added to `never` in code.
- `pinned` is a list of names or patterns, empty at ship: no harness of the 15 is known to pin. A name
  in `pinned` is what PRD-03 and PRD-12 call "marked `pinned = true`": `decide` answers
  `Listed { pinned: true }` with the kind of the list that also holds it, or `other`. The pad's dodger
  case writes its own copy of this file with the harness's model host added to `pinned` and passes it
  with `--hosts` (S16).
- Left off on purpose: `api.cloudflare.com` (it also carries the agent's own deploys), `models.dev`,
  npm, GitHub, telemetry hosts, sign-in hosts. They are tunnels, counted with bytes and times.

The `HostList` (seam S3) merges three sources, fixed for an entry: `shared` (this file, or the
`--hosts` file), `participant` (read from `$GOALCUP_HOME/record-hosts.txt`, which PRD-10 writes),
`config` (hints). `decide(name)` answers `Listed { source, kind, pinned } | Never | NotListed`
(PRD-00 S3 naming). `ca_subtrees()` gives the wildcard-free suffix of each entry for the run CA's name
constraints (PRD-03 section 4). `lists()` gives the three lists with `kind` and `pinned` per entry, and
`sha256()` the hash of the shared file as read, for the `start` event.

**One host check for every source** (PRD-17 section 2.7, PRD-00 S3):
`connector::hosts::check(name: &str, source: Source) -> Result<Host, Skipped>`. The shared file, each
line of `record-hosts.txt`, `goalcup setup --record-host` (PRD-10 refuses on `Err`) and every hint call
it, so `lessons_hosts.rs` (PRD-17) tests one entry point. It copies today's rules
(`crates/goalcup-cli/src/model_host.rs:836-893`, `crates/goalcup-protocol/src/capture_policy.rs:1001-1049`
for `is_capture_host` and `local_host_problem`, `crates/goalcup-cli/src/proxy/preflight.rs:169-192`;
about 160 lines, PRD-17 ledger):
lowercase, labels of 1 to 63 of letters, digits and `-`, at most 253, no trailing dot, no IPv4 or IPv6
literal, no single-label name, no `localhost` forms or numeric spellings of an address, no backslash,
space or control byte in the authority (userinfo dropped before anything can log it), not a `never`
host, not goalcup's server. Patterns are allowed for `Shared` only; `Participant` takes an exact name
or one `*.` in front of at least two labels; `Config` takes exact names. `Skipped` says whether the
name points at this computer, so the hint path can write `local_model` instead of a plain skip.

### 5. The launch plan (seam S3)

PRD-00's S3 final shape, filled in. Where PRD-00's sketch abridges, this block is the detail; the
names and the signatures are PRD-00's.

```rust
pub fn load(id_or_path: &str, hosts_file: Option<&Path>) -> Result<Connector, ConnectorError>; // built-in id, or S16 files
pub fn plan(connector: &Connector, env: &Env, home: &Path) -> LaunchPlan;   // home = $GOALCUP_HOME/homes/<id>

pub struct Env {                          // what the plan reads besides the connector, gathered by PRD-10
    pub vars: BTreeMap<String, String>,   // the participant's whole environment (owner question 2, old 13)
    pub workspace: PathBuf,
    pub resume: bool,                     // goalcup continue
    pub run_start: SystemTime,            // `since` for native rows
    pub goalcup_home: PathBuf,            // record-hosts.txt lives here
    pub server_host: String,              // goalcup's own server, always `never`
}
pub struct LaunchPlan {
    pub connector: ConnectorRef,          // id, name, sha256, source ("builtin" or "file")
    pub harness_version: Option<String>,
    pub program: PathBuf,                 // resolved on PATH
    pub argv: Vec<String>,                // args or args + resume, templates filled
    pub env: BTreeMap<String, String>,    // complete harness env, minus port and CA paths
    pub home: PathBuf,                    // the isolated home, created
    pub lock: HomeLock,                   // the flock; held as long as the plan lives
    pub before: Vec<Vec<String>>,         // run by the watchdog before spawn (S8)
    pub after: Vec<Vec<String>>,          // run by the watchdog after harness exit, before native finish
    pub hosts: HostList,                  // shared + participant + config (PRD-00's `host_hints` are its config source)
    pub native: NativeSources,            // PRD-04 shape: NativeSource { name, root, pattern, form, since, warn_missing }
    pub mask_rules: Vec<MaskRule>,
    pub secret_values: Vec<(String, Vec<u8>)>, // empty in the default build; with the owner's yes (M1a), candidates with key paths for PRD-05's gate
    pub notes: Vec<serde_json::Value>,    // `local_model` notes, written once per entry after `start`
    pub refusal: Option<Refusal>,         // a refuse_if_set hit or a held lock: file and key, never the value
}
impl LaunchPlan {
    pub fn finish_env(&mut self, port: u16, run_ca: &Path, bundle: &Path);
}
pub fn write_settings(home: &Path, rows: &[SettingRow]) -> Vec<Skipped>; // `no_prompts` or `update_off` rows into the isolated home
```

`HostList`, `NativeSources` and `MaskRule` derive `Serialize` and `Deserialize`: the watchdog sends them
to the recorder in the start facts (PRD-00 S17, owned by PRD-02), with `notes`.

`load` refuses only what cannot run at all: an unknown id, a file that does not parse or breaks a
loader rule. `plan` never fails: anything a participant can fix is a `refusal`, which PRD-10 shows
before any process starts. `plan` reads files, the environment and `record-hosts.txt` once and resolves
hint names once (PRD-17 N6); it opens no connection. Before it returns, `plan` writes the connector's
`update_off` rows into the isolated home with `write_settings` (ledger H9); a row it cannot write is a
log line, never a refusal. PRD-10's `goalcup setup` calls the same `write_settings` for `no_prompts`
(after its question) and `update_off` (always). PRD-10 reads the participant-facing fields (`name`,
`first_screen`, `no_prompts`, `update_off`, `skills_dir`) from `Connector` directly. Whether a harness
runs inference on its vendor's servers is not a connector key: it is the host's `kind = "vendor"` in
`model-hosts.toml`, and PRD-10 words it from `conn` meta in the record (PRD-03 section 12).

The watchdog (PRD-02, S8) mints the CA from `hosts` with PRD-03's `RunCa::mint`, writes `run-ca.pem`
and `bundle.pem` into `$GOALCUP_HOME/runs/<run>/<entry>/` with PRD-03's `write_files`, binds the port,
calls `finish_env` with those two paths, runs `before`, spawns `program argv`, and after the harness
exits runs `after` before `native.finish`. The start facts it sends each boot carry `notes`; the
recorder writes them with `Recorder::event` after `start` on boot 0 only.

`secret_values`: **default build, empty.** Exact-value masking beyond `GH_TOKEN` is not in the ledger
(M1 is the masking list); PRD-00 owner question 3 (old 12) asks the owner, default no. Until the ledger
records the owner's yes (as M1a), `plan` reads no `secret_files` for masking and passes no
`secret_values`; the `secret_files` data rows stay (the connector test uses them, section Acceptance).

Only with the owner's yes: from each `secret_files` file, every string scalar (JSON, TOML, YAML; dotenv
values), each with its key path (`tokens.access_token`, `providers.deepseek.apiKey`,
`OPENAI_API_KEY`). No length floor here: PRD-05 section 9 owns the gate (credential-named key path, 20
to 256 bytes, not a URL or path) and the order and the 32-value guard, and counts what it skips. This
covers tokens a harness copies from a key file into a database we snapshot (OpenCode's `credential`
table, H4.5). A file that changes during the run is not re-read (PRD-05 owns any later watch).

Either way, OpenClaw keeps auth profiles in the agent SQLite itself (H5.5), and OpenCode may copy a key
into its `credential` table (H4.5). A token there that has no known shape, and that no exact value
covers, is masked by shape rules only. Acceptance 7 runs PRD-08 `secrets` on OpenClaw's agent SQLite;
OpenClaw stays "not verified" until it passes.

### 6. The 16 connector files

One per harness; OpenCode has a second file for its major version 2 (16 files; the pad table counts
OpenCode once, with both versions in its row). "Unverified" marks a value the builder confirms against
the vendor's current docs or `--help` before shipping it. `no_prompts` values are copied from today's
connectors where they exist (`connectors/{claude-code,codex,hermes}-cli/harness.toml`, the
`[[participant.no_prompts]]` tables); for the other 12 the builder adds them from the vendor's docs
when writing the file (third slice), or leaves the key out. Build order: the Claude Code, Codex,
Hermes and Kilo files in phase 0 (PRD-00 phase 0 step 7); the Claude Code and Codex launch plans first
in the core slice (phase 1a); Hermes's helpers in the second slice; the other 12 files in the third
slice. A line marked "unverified" in a file below is checked by the builder (builder notes); if the
command or key does not exist, the line is removed and the facts sheet names the gap. A `before` or
`after` command never stops a service outside the isolated home.

```toml
# connectors/claude-code-cli.toml  (Claude Code 2.1.312)
id = "claude-code-cli"
name = "Claude Code"
program = "claude"
resume = ["--continue"]
home = { CLAUDE_CONFIG_DIR = "{home}" }
env = { DISABLE_AUTOUPDATER = "1", DISABLE_UPDATES = "1", CLAUDE_CODE_DISABLE_AGENT_VIEW = "1", CLAUDE_CODE_RETRY_WATCHDOG = "1" }
unset = ["CLAUDE_CODE_CERT_STORE"]
refuse_if_set = [
  { file = "{home}/settings.json", keys = ["env.HTTPS_PROXY", "env.https_proxy", "env.NO_PROXY", "env.no_proxy", "env.CLAUDE_CODE_CERT_STORE", "env.NODE_EXTRA_CA_CERTS"] },
  { file = "{workspace}/.claude/settings.json", keys = ["env.HTTPS_PROXY", "env.https_proxy", "env.NO_PROXY", "env.no_proxy", "env.CLAUDE_CODE_CERT_STORE", "env.NODE_EXTRA_CA_CERTS"] },
  { file = "{workspace}/.claude/settings.local.json", keys = ["env.HTTPS_PROXY", "env.https_proxy", "env.NO_PROXY", "env.no_proxy", "env.CLAUDE_CODE_CERT_STORE", "env.NODE_EXTRA_CA_CERTS"] },
]
host_hints = [{ file = "{home}/settings.json", key = "env.ANTHROPIC_BASE_URL" }]
hint_env = ["ANTHROPIC_BASE_URL"]
secret_files = ["{home}/.credentials.json"]
native = [
  { name = "tool-results", root = "{home}/projects", pattern = "**/tool-results/*", form = "snapshot", warn_missing = false },
  { name = "meta", root = "{home}/projects", pattern = "**/*.meta.json", form = "snapshot", warn_missing = false },
  { name = "transcripts", root = "{home}/projects", pattern = "**/*.jsonl" },
]
mask = [{ kind = "anthropic_billing_header", header_prefix = "x-anthropic-billing-" }]
first_screen = "Claude Code may ask whether you trust this folder. Choose Yes. Background agents (agent view) are off during the run so every agent goes through the recorder."
no_prompts = [
  { file = "{home}/settings.json", key = "permissions.defaultMode", value = "bypassPermissions" },
  { file = "{home}/settings.json", key = "skipDangerousModePermissionPrompt", value = "true" },
]
skills_dir = "{home}/skills"
```

```toml
# connectors/codex-cli.toml  (Codex 0.165.0)
id = "codex-cli"
name = "Codex"
program = "codex"
args = ["--no-daemon", "-C", "{workspace}"]   # no `-c` override: it forces embedded mode with a startup warning (PRD-17 HF5)
resume = ["resume", "--last"]
home = { CODEX_HOME = "{home}" }
ca_env = ["CODEX_CA_CERTIFICATE"]
refuse_if_set = [{ file = "{home}/.env", keys = ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy", "NO_PROXY", "no_proxy", "SSL_CERT_FILE", "NODE_EXTRA_CA_CERTS"] }]
host_hints = [{ file = "{home}/config.toml", key = "model_providers.*.base_url" }, { file = "{home}/config.toml", key = "openai_base_url" }]
hint_env = ["OPENAI_BASE_URL"]
secret_files = ["{home}/auth.json", "{home}/.env"]
native = [
  { name = "sessions", root = "{home}/sessions", pattern = "**/rollout-*.jsonl" },
  { name = "state", root = "{home}", pattern = "*.sqlite*", form = "final" },
]
no_prompts = [
  { file = "{home}/config.toml", key = "approval_policy", value = "never" },
  { file = "{home}/config.toml", key = "sandbox_mode", value = "danger-full-access" },
]
update_off = [{ file = "{home}/config.toml", key = "check_for_update_on_startup", value = "false" }]
first_screen = "Codex may ask whether you trust this folder. Choose Yes."
skills_dir = "{home}/skills"
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
  { name = "delegation-manifest", root = "{home}/cache/delegation/live", pattern = "**/manifest.json", form = "snapshot", warn_missing = false },
  { name = "delegation", root = "{home}/cache/delegation/live", pattern = "**/task-*.log", warn_missing = false },
]
first_screen = "If Hermes asks to run setup, do it now, before you type your prompt: answer Y and pick your provider and model. Hermes then closes; continue the run, then type your prompt."
no_prompts = [{ file = "{home}/config.yaml", key = "approvals.mode", value = "off" }]
update_off = [{ file = "{home}/config.yaml", key = "updates.check", value = "false" }]
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
(`docs/help/environment.md:56-62` in the OpenClaw repo); the builder fixes the roots to match. OpenClaw
subagents need its Gateway daemon (appendix O H5.4); `openclaw chat` runs embedded, with no Gateway, and
1.0 does not start one (owner list OW-10). Whether a Gateway started outside goalcup can be stopped for
`OPENCLAW_HOME` only is checked by the builder (builder notes step 4); if yes, `before` and `after` are
added as for Hermes.

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
first_screen = "If Amp asks you to sign in, do it now, before you type your prompt."
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
hint_env = ["COPILOT_PROVIDER_BASE_URL"]
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
host_hints = [{ file = "{home}/.gemini/.env", key = "GOOGLE_GEMINI_BASE_URL" }, { file = "{home}/.gemini/.env", key = "CODE_ASSIST_ENDPOINT" }]
hint_env = ["GOOGLE_GEMINI_BASE_URL", "CODE_ASSIST_ENDPOINT"]
secret_files = ["{home}/.gemini/.env", "{home}/.gemini/oauth_creds.json"]
native = [{ name = "chats", root = "{home}/.gemini/tmp", pattern = "**/chats/**/*.jsonl" }]
update_off = [{ file = "{home}/.gemini/settings.json", key = "general.enableAutoUpdate", value = "false" }]
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
refuse_if_set = [{ file = "{home}/settings.json", keys = ["proxy"] }, { file = "{workspace}/.qwen/settings.json", keys = ["proxy"] }]
host_hints = [{ file = "{home}/settings.json", key = "modelProviders.*.*.baseUrl" }]
secret_files = ["{home}/settings.json", "{home}/.env"]
update_off = [{ file = "{home}/settings.json", key = "general.enableAutoUpdate", value = "false" }]
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
before = [["daemon", "stop"]]   # unverified: the builder confirms the command (step 4) or removes it and names the droid daemon as a dodge in the facts sheet
after = [["daemon", "stop"]]    # unverified, same check
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
hint_env = ["KIMI_MODEL_BASE_URL"]
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
before = [["hub", "stop"]]   # unverified: the builder confirms it stops only the hub for this `CLINE_DIR` (step 4), or removes it and names the hub as a dodge
after = [["hub", "stop"]]    # unverified, same check
host_hints = [{ file = "{home}/data/settings/providers.json", key = "*.baseUrl" }]
secret_files = ["{home}/data/settings/providers.json"]
native = [
  { name = "sessions", root = "{home}/data/sessions", pattern = "**/*" },
  { name = "index", root = "{home}/data/db", pattern = "sessions.db*", form = "final" },
]
```

These rows are the one source of native rows (PRD-04 section 2 keeps only the form rule). Two
choices made from PRD-04's points: Codex's state database row is `*.sqlite*` final in `CODEX_HOME`, so
the thread edges are kept whatever the file is called (a large log database is copied too; no cap);
Hermes's `manifest.json` is its own `snapshot` row before the `append` row for `task-*.log`, because a
rewritten manifest under `append` would end `source_changed` (both unverified until the Hermes pad
run). A row may match no file for a long time (Claude Code tool results, Hermes delegation when no
subagent runs): those carry `warn_missing = false` so PRD-10's line stays honest.

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
| 3 | Hermes v0.22.5 (Python) | Chat Completions from a gateway process per `HERMES_HOME`, about 40 providers, `model.base_url` for gateways | `HTTPS_PROXY`, `NO_PROXY` (httpx); trust through `truststore`, which on Linux reads `SSL_CERT_FILE` as a replacement, so the full bundle (H3.3) | `state.db` (all sessions, children with `parent_session_id`); `cache/delegation/live/<id>/task-<n>.log` (H3.5) | Check only; `updates.check: false` in `config.yaml`, no env switch (H3.6) | Keys in `$HERMES_HOME/.env`, Nous Portal sign-in, borrowed Codex or Claude credentials | A gateway already running for the home keeps its env and calls direct, not even tunnelled; `copilot-acp` child is wire only (section 8) (H3.3) | **Likely verified**, risk: gateway env and `SSL_CERT_FILE` untested on 0.22.5; core set |
| 4 | OpenCode 1.19.5; OpenCode 2.2.2 (Bun) | HTTPS to the chosen provider (native DeepSeek), or OpenCode's own gateway | `HTTPS_PROXY` (measured), `NO_PROXY=localhost,127.0.0.1` required; `NODE_EXTRA_CA_CERTS` adds, `SSL_CERT_FILE` also worked (facts sheet section 6) | One `opencode.db` for all sessions; subagents are child sessions in it (H4.5) | `OPENCODE_DISABLE_AUTOUPDATE=1` (H4.6) | Key in `auth.json` or env, OpenCode gateway account | OpenCode 2 attaches to a background service with its own env unless `--standalone` (H4.3) | **Likely verified** for 1 (measured 28 Oct); **risky** for 2; needs a PRD-09 reader written from its phase 5 sample |
| 5 | OpenClaw 2026.10.9 (Node 24+) | Chat Completions through provider plugins | `HTTPS_PROXY` with `NODE_USE_ENV_PROXY=1`; `NODE_EXTRA_CA_CERTS` adds, `SSL_CERT_FILE` ignored (measured); its own `proxy.proxyUrl` and `OPENCLAW_PROXY_URL` beat env (H5.3) | `agents/<agent>/agent/openclaw-agent.sqlite`, `state/openclaw.sqlite`; logs in `/tmp/openclaw` (H5.5) | `OPENCLAW_NO_AUTO_UPDATE=1` (H5.6) | Keys in `~/.openclaw/.env`, auth profiles in the agent SQLite | Own proxy setting; subagents need a Gateway daemon with its own env; raw sockets "may bypass" (H5.3, H5.4) | **Risky.** Main agent likely; subagents unknown; needs a PRD-09 reader written from its phase 5 sample. Subagents need a Gateway daemon (H5.4); 1.0 does not start one, so PRD-12 check C2 is expected INCONCLUSIVE "no subagent" and the tier "not verified: subagents need the Gateway" (owner list OW-10) |
| 6 | Amp 0.0.1793275344 (Bun) | One JSON-RPC WebSocket to `ampcode.com`; Amp's servers call the model | `HTTPS_PROXY`; `NODE_EXTRA_CA_CERTS` adds (H6.3) | No transcript; `~/.cache/amp/logs/threads/T-*.log` metadata (H6.5) | `AMP_SKIP_UPDATE_CHECK=1` (H6.6) | Amp account always (`amp login` or `sgamp_` token); keys stored in Amp | Orbs (`-ox`) run remotely; provider calls always server side (H6.3) | **Vendor backend, not verified this cycle** (no Amp account, 30 Oct); needs a PRD-09 reader written from its phase 5 sample |
| 7 | GitHub Copilot CLI 1.0.105 (Node SEA plus Rust addon) | HTTPS to GitHub-hosted models, or BYOK (`anthropic` type for DeepSeek) | `HTTPS_PROXY` beats its `proxyUrl` setting; `NODE_EXTRA_CA_CERTS` plus OS store; `SSL_CERT_FILE` string present; which layer dials is unverified (H9.3) | `session-state/<id>/events.jsonl`; `session-store.db`; subagent layout unverified (H9.5) | `COPILOT_AUTO_UPDATE=false` (H9.6) | GitHub sign-in or BYOK with no GitHub login | `/delegate` cloud agent; voice ignores proxy (issue 5245) (H9.3) | **Likely verified** if the CA layer holds; try the DeepSeek key; needs a PRD-09 reader written from its phase 5 sample |
| 8 | Gemini CLI 0.66.0 (Node) | HTTPS to `generativelanguage.googleapis.com` with a key, or `GOOGLE_GEMINI_BASE_URL`; on Google sign-in, Code Assist at `cloudcode-pa.googleapis.com` (`CODE_ASSIST_ENDPOINT`, listed as a vendor backend) | `HTTPS_PROXY` (global undici agent), `NO_PROXY`; `NODE_EXTRA_CA_CERTS` in the launch env only (H8.3) | `.gemini/tmp/<project>/chats/session-*.jsonl`; subagents `chats/<parent>/<id>.jsonl` (H8.5) | Settings only: `general.enableAutoUpdate: false`; no env switch (H8.6) | Paid Gemini API key, Agent Platform, Google sign-in (no longer for free users) | `--sandbox` sets its own proxy for tools; remote A2A subagents (H8.3) | **Likely verified** with the funded Google key; needs a PRD-09 reader written from its phase 5 sample |
| 9 | Goose v1.57.0 (Rust, rustls) | HTTPS to the provider; DeepSeek through a custom OpenAI-format provider | `HTTPS_PROXY`, `NO_PROXY`; `GOOSE_CA_CERT_PATH` adds; `SSL_CERT_FILE` replaces, so the full bundle (H10.3) | `~/.local/share/goose/sessions/sessions.db`; subagents are rows with `session_type = 'sub_agent'` (H10.5) | Does not self-update (H10.6) | Keys from env, keyring, or `secrets.yaml` | None found for model calls (H10.3) | **Likely verified**; needs a PRD-09 reader written from its phase 5 sample |
| 10 | Kilo CLI 7.8.25 (Bun, OpenCode fork) | HTTPS to the provider (native DeepSeek), Kilo's gateway, or Vercel AI Gateway | `HTTPS_PROXY`; OS store (`--use-system-ca`) plus `NODE_EXTRA_CA_CERTS` (H12.3) | `~/.local/share/kilo/kilo.db`; subagents are child sessions (H12.5) | `KILO_DISABLE_AUTOUPDATE=1`; silently installs patches otherwise (H12.6) | Key in env or `auth.json`, Kilo account for its gateway | None found (H12.3) | **Likely verified**; needs a PRD-09 reader written from its phase 5 sample |
| 11 | Qwen Code 0.26.4 (Node 22+) | Chat Completions to the provider (native DeepSeek preset) | `--proxy`, then settings `proxy`, then `HTTPS_PROXY`; `NODE_EXTRA_CA_CERTS` (H13.3) | `projects/<cwd>/chats/<id>.jsonl`; subagents `subagents/<id>/agent-<id>.jsonl` plus `.meta.json` (H13.5) | Settings only: `general.enableAutoUpdate: false` (H13.6) | Keys in env or `settings.json`; Qwen sign-in ended 15 Apr 2026 | Settings `proxy` and `--proxy` beat env; built-in `claude-code` and `codex` subagents start other harnesses: wire only (section 8), named in RULES (owner question 4, old 14) (H13.3) | **Likely verified**; needs a PRD-09 reader written from its phase 5 sample |
| 12 | Crush 0.99.0 (Go) | Chat Completions to the provider (native DeepSeek) | `HTTPS_PROXY` (Go default transport); `SSL_CERT_FILE` replaces, so the full bundle (H14.3) | `<workspace>/.crush/crush.db`; subagents are child session rows (H14.5) | Binary does not self-update (H14.6) | Key in env or pasted (stored in `crush.json`), Charm Hyper | `CRUSH_CLIENT_SERVER=1` with an older server (H14.3) | **Likely verified.** Its subagents are read-only search agents; needs a PRD-09 reader written from its phase 5 sample |
| 13 | Factory Droid 0.251.0 (Bun) | BYOK to `baseUrl`; Factory backend `api.factory.ai` for auth, telemetry and cloud session sync | `HTTPS_PROXY`, `NO_PROXY` only (no OS proxy); `NODE_EXTRA_CA_CERTS` or `FACTORY_EXTRA_CA_CERTS` (H15.3) | `~/.factory/sessions/<slug>/<uuid>.jsonl` plus `.settings.json`; subagent layout unverified (H15.5) | `FACTORY_DROID_AUTO_UPDATE_ENABLED=false` (H15.6) | Factory account always (browser or `FACTORY_API_KEY`), plus BYOK | A separately started `droid daemon` or desktop app; cloud Droid Computers (H15.3) | **Risky, not verified this cycle** (no account); BYOK subagents reported broken (issue 691); needs a PRD-09 reader written from its phase 5 sample |
| 14 | Kimi Code 2.4.0 (Node 22.19+) | HTTPS to `api.kimi.com` after sign-in, or a provider in `config.toml` | `HTTPS_PROXY`, `ALL_PROXY`, `NO_PROXY`; loopback always bypassed ([env vars](https://moonshotai.github.io/kimi-code/en/configuration/env-vars)); CA through `NODE_EXTRA_CA_CERTS` (Node standard, not documented, unverified) | `sessions/<workspace>/<session>/agents/main/wire.jsonl`; one `agents/agent-N/wire.jsonl` per subagent; `session_index.jsonl` ([data locations](https://moonshotai.github.io/kimi-code/en/configuration/data-locations)) | `KIMI_CODE_NO_AUTO_UPDATE=1` (env vars page) | Kimi OAuth, or provider keys through `api_key_env` (shell keys are not read otherwise) | None found | **Likely verified**; needs a PRD-09 reader written from its phase 5 sample |
| 15 | Cline CLI 3.0.74 (Bun) | HTTPS to the provider (native DeepSeek), or Cline's own provider | `https_proxy`, `http_proxy`, `no_proxy`; `NODE_EXTRA_CA_CERTS` only ([networking](https://docs.cline.bot/troubleshooting/networking-and-proxies)) | `data/sessions/<id>/`, index `data/db/sessions.db` ([CLI reference](https://docs.cline.bot/cli/cli-reference)) | `CLINE_NO_AUTO_UPDATE=1` inferred from the binary, unverified; release notes say updates install on exit (cli-v3.0.55) | Provider keys via `cline auth` (stored in `data/settings/providers.json`), Cline account | A hub daemon on port 25463 may already run with another env; `CLINE_SESSION_BACKEND_MODE=local` should keep sessions in process (unverified) | **Risky** (hub daemon); needs a PRD-09 reader written from its phase 5 sample |
| | Cursor CLI (out of the 15) | Connect RPC over HTTP/2 to `*.cursor.sh`; inference on Cursor's servers | Listed as a vendor backend; HTTP/2 is now supported (PRD-03) | `~/.cursor/chats/...` | `--disable-auto-update` | Cursor account | IDE ignores proxy with HTTP/2 on (Cursor forum, 29 Oct 2026) | **Documented dodger case.** No connector |

Versions: npm `time` fields and tags fetched 29 Oct 2026 (appendix O), and for Cline and Kimi Code
the npm registry on 30 Oct 2026: `cline` `latest` 3.0.74 (26 Oct), `@moonshot-ai/kimi-code`
`latest` 2.4.0 (28 Oct).

What "auto-update off" can and cannot do (ledger H9): where a harness has an env switch, `env` sets it
at every start. Where it has only a settings key (Hermes, Gemini CLI, Qwen Code, Codex), `update_off`
names it, and `plan` writes it into goalcup's own isolated home at every start and `continue`, never the
participant's real one; `goalcup setup` writes it too, with no question (owner question 2, old 8; S15).
A harness that rewrites the file mid-run may still update. A running process keeps its binary; an
update only matters at the next `goalcup continue`, and the `start` event records the version at every
start (appendix N, Q7).

### 8. Cross-harness subagents: wire only

Some harnesses start another harness as a subagent: Qwen Code's built-in `claude-code` and `codex`
subagents, Hermes's `copilot-acp` child (appendix O H13.3, H3.3). The child inherits the launch
environment, so its model calls go through the same proxy, are decrypted when the host is listed and
are recorded like any other call (points 1 and 2 on the wire). Its session files go to the child's own
home in the participant's real home folder (`~/.codex`, `~/.claude`), which no native row reads and
goalcup never touches (AGENTS.md hard rule). 1.0 does not redirect the child's home: that would hide
the child's login and is per-harness code. The case is named "wire only" in the facts sheets and the
tier table, and RULES names those subagents as not allowed (owner question 4, old 14).

## Files it owns

Create:

- `connectors/model-hosts.toml`; `connectors/<id>.toml` for the 16 files in section 6.
- `crates/goalcup/src/connector/mod.rs` (types, `load`, `plan`), `schema.rs` (serde structs, checks,
  about 170 lines), `builtin.rs` (the `include_str!` list), `hosts.rs` (`HostList`, matching,
  `hosts::check` copied per PRD-17 section 3, about 200), `hints.rs` (file key lookup in JSON, TOML,
  YAML, dotenv; `hint_env`; `local_model` notes, about 200, YAML copied from `model_host.rs:682`
  onward), `launch.rs` (env build, homes, lock, `.profile`, version probe, secret candidates, about
  230). About 800 src lines in all (PRD-00 row: 800 src, 400 tests).
- `crates/goalcup/tests/connector_*.rs` (section Acceptance).
- `docs/harnesses/<id>.md` for each harness: one page from section 7's row, facts and limits only,
  and "wire only" where section 8 applies. The pad recipe lives only in
  `scripts/pad/harnesses/<id>.toml` (PRD-12). The OpenCode and
  OpenClaw sheets are copied from branch `origin/opencode-openclaw` and updated; the old sheets for
  Amp, Claude Code, Codex and Hermes are rewritten to the new shape.

Change: `docs/harnesses/README.md`, the one harness table (harnesses and facts; its tier columns are
generated from `connectors/tiers.toml`, which PRD-12 owns). Other docs link to it, never copy it.

Delete (with PRD-13, which removes the old crates in the phase 3 merge): `connectors/*/harness.toml`
(four folders), `crates/goalcup-protocol/providers.toml`, `crates/goalcup-protocol/pins/`,
`scripts/pin-manifests.sh`. The old files are not edited before that: the old crates are excluded from
the workspace from phase 0 (PRD-00), so a row removed from an old file changes nothing that runs.

Must not touch: `src/proxy/` (PRD-03, which owns the `decide` call and the CA), `src/native/` (PRD-04),
`src/mask/` (PRD-05), `src/pipeline/` and `src/watchdog/` (PRD-02), `src/cli/` and `src/main.rs`
(PRD-10), `scripts/pad/**` and `connectors/tiers.toml` (PRD-12), `docs/*.md` (PRD-15, which also
rewrites `docs/CONNECTOR-SCHEMA.md` from section 2).

## Interfaces with other PRDs

| PRD | Uses or gives | Seam |
|---|---|---|
| PRD-00 | S3 final shape (section 5 fills it in), S16 (`--connector`, `--hosts`), the rulings on hints, tiers and secret values | S3, S16 |
| PRD-01 | `start` keys `connector`, `connector_source`, `connector_sha256`, `harness_version`, `hosts` (each entry with `kind` and `pinned`), `hosts_sha256`; note `local_model` (`{"what":"local_model","host":...}`) | data |
| PRD-02 | The watchdog spawns from `LaunchPlan` after `finish_env(port, run_ca, bundle)`; runs `before` before spawn and `after` after harness exit and before native finish (S8); carries `HostList`, `NativeSources`, mask rules and `plan.notes` in the start facts (S17; the S3 types derive `Serialize` and `Deserialize`) and the recorder writes `notes` after `start` on boot 0. The plan holds the home lock as long as it lives | S3, S8, S17 |
| PRD-03 | `HostList::decide(name) -> Listed { source, kind, pinned } \| Never \| NotListed`, `ca_subtrees()`. Its asks are met: `HTTPS_PROXY` without credential, `NO_PROXY` with `::1`, `NODE_USE_ENV_PROXY=1`, extend variables to `run-ca.pem`, `SSL_CERT_FILE` and `REQUESTS_CA_BUNDLE` to `bundle.pem`. PRD-03 owns the code that mints the CA and writes both PEM files; PRD-02's watchdog calls it. `pinned` is a list of names in the host file (section 4) | S3 |
| PRD-04 | `NativeSources` of `NativeSource { name, root, pattern, form, since, warn_missing }`, roots absolute, `{home}` and `{workspace}` resolved. Section 6 is the one source of rows | S3 |
| PRD-05 | `mask_rules: Vec<MaskRule>` (PRD-05 grammar, no `object`). Only with the owner's yes to question 3 (M1a): `env` and `secret_values: Vec<(String, Vec<u8>)>` as candidates with key paths, and PRD-05's gate, order and 32-value guard decide; no floor here. Default build: `secret_values` empty, `secret_files` not read. The new Codex file has no rate-limit, plan or credit rows | S3 |
| PRD-07 | `connector_sha256` in the start facts | data |
| PRD-08 | Connector id and the three host lists in the `start` event decide the harness reader and the "listed hosts decrypted" check | data |
| PRD-09 | `kind` per host (`provider`, `gateway`, `vendor`, and `other` for participant and config hosts), read from the record (`start` event, `conn` meta), never from an embedded copy of the list; connector ids and native row names for its hint table | data |
| PRD-10 | Calls `connector::load` and `connector::plan(&connector, &env, home)`; reads `name`, `first_screen`, `no_prompts`, `update_off`, `skills_dir` from `Connector`; `goalcup setup` calls `connector::write_settings` for `update_off` always and for `no_prompts` after its question; words the vendor backend line from `conn` meta `kind: "vendor"` in the record; prints the start line for a hint file it could not read; writes `record-hosts.txt`, which `plan` reads; calls `hosts::check(name, Source::Participant)` for `setup --record-host` and refuses on `Err`; shows `refusal`; words `local_model`; owns the `--connector` and `--hosts` flags and refuses them in competition mode. Helper stops are the watchdog's, not PRD-10's start step 7. `goalcup setup <id>` opens the harness in `{home}` so the participant signs in or configures a gateway there (for example the Vercel CLI writes `~/.config/kilo/kilo.json` in the real home; the participant must make that change inside goalcup's home) | S3, S15, S16 |
| PRD-12 | A connector plus a key is the pad's unit; flat `connectors/<id>.toml`, 16 files, one table row for OpenCode; the dodger case passes `--hosts` with a copy of `model-hosts.toml` whose `pinned` list holds the harness's model host. PRD-12 writes `connectors/tiers.toml`. The pad recipe per harness is appendix O section .7, with the paths of section 6, kept only in `scripts/pad/harnesses/<id>.toml`. The pad's `--pre <script>` sets up acceptance 3 to 5 (ask 6). A pad failure caused by connector data comes back to this PRD | S16, data |
| PRD-15 | Writes `docs/CONNECTOR-SCHEMA.md` (a page, not 1,002 lines) from section 2, and the participant guide lines on harness limits from sections 7 and 8 | docs |
| PRD-17 | Lessons N1 to N8 (host check, config reader, `NO_PROXY`), P9 to P12 (key pass-through, `NODE_USE_ENV_PROXY`, `.profile`, lock name), T6 (CA variables never set), HF1 to HF8 (harness facts; HF5 guarded by `connector_launch.rs::codex_gets_no_dash_c_override`) are acceptance for this PRD; `lessons_hosts.rs` calls `hosts::check` | tests |

## What is cut

| Path | Lines | Note |
|---|---:|---|
| `connectors/*/harness.toml` | 556 data | Replaced by about 380 lines in 16 files plus 30 in `model-hosts.toml` |
| `crates/goalcup-protocol/src/harness/{schema,check,load,parse}.rs`, `harness.rs` | 3,370 src | Replaced by about 370 lines (`schema.rs`, `builtin.rs`, `hosts.rs`) |
| `crates/goalcup-protocol/src/pins.rs`, `pins/connector-dirs.txt`, `scripts/pin-manifests.sh` | 1,031 | Pins gone; the start event's SHA-256 replaces them |
| `crates/goalcup-protocol/providers.toml` | 25 | Into `model-hosts.toml` |
| `crates/goalcup-cli/src/model_host.rs` | 1,540 src | About 200 in `hints.rs` and the host check in `hosts.rs`. The refusals at start go: a hint never refuses |
| `crates/goalcup-cli/src/connector.rs`, `home.rs`, `per_run_home.rs`, `harness_version.rs` | 1,854 src | About 230 in `launch.rs` (lock name and `.profile` copied) |
| Fields with no successor | | `time_left_tool` (with `goalcup mcp`), `setup_section`, `capture.identity`, `termination`, `argv_guard`, `platform`, `reserved_env_prefixes`, `configure`, `launch_prep`, `auth_hosts` (now `never`), `capture.hosts` (now the shared list), `model_host_from.via`/`absent`, `protocol_hint`, `since_run_start`, CA `mode`, `seed`, `real_dirs`. Kept in a flatter form: `display_name` (`name`), `first_screen`, `no_prompts` (one `value`), `warn_missing` (per row), `skills_dir` (a template) |
| Hermes workarounds | data | The `providers.recorder` entry with `base_url` (it set a model base URL), `HERMES_CA_BUNDLE`, the two `configure` commands, the `session_*.json` row (`connectors/hermes-cli/harness.toml:54-57, 88-92, 103-109, 145-152`) |
| Tests | 10,971 | Protocol `harness_schema` 4,451, `harness_loader` 650, `harness_conformance` 379, `connector_pins` 303, `connector_template` 120, `pins_build_fails` 86; CLI `connector_contract` 994, `harness_contract_golden` 975, `harness_contract_kit` 805, `harness_launch_plan` 782, `model_host_from` 793, `per_run_home` 438, `harness_version_probe` 195. PRD-13 deletes them with the crates |

Net: about 8,400 src and data lines and about 11,000 test lines go; about 800 src and 410 data lines
come back.

## Acceptance

Real-run proofs first (run on the pad, PRD-12; a watcher checks them). A row that needs a key or an
account this cycle does not have (OpenRouter, Amp, Factory; ledger T2) is marked "not run: no key"
and is not pending.

1. **Direct key.** Claude Code (Claude key) and Codex (OpenAI key) in the core slice, then Hermes
   (DeepSeek), each run the pad prompt from their built-in connector. Pass: every model request the
   harness's own session files record has a matching decrypted `http_req` or `ws_up` message in the
   record (for Hermes, whose `state.db` has per-session totals only, the per-session comparison after
   the seal counts, PRD-00 owner question 7, old 19); PRD-12 checks C1 and C2 pass; zero `conn` from the
   harness to its model host is a tunnel; a tunnel to another listed host carries a named reason (PRD-08
   `named`); the start event shows `connector_source: "builtin"` and the SHA-256 of the built-in file.
2. **Gateway.** Kilo, and the core set's gateway rows (PRD-12 P4), with `ai-gateway.vercel.sh` set in
   their config inside `{home}` (the pad sets it as a participant would; goalcup never does). Pass:
   same checks as 1; every `conn` to the gateway is `mode: "decrypt"`, `listed: "shared"`,
   `kind: "gateway"`; no connector has a gateway-specific line.
3. **Unknown host by hint, and a local model** (pad `--pre hint-local.sh`, PRD-12). Codex with a `--hosts` copy of `model-hosts.toml`
   without `api.openai.com`, and in `{home}/config.toml` a `[model_providers.x]` table with
   `base_url = "https://api.openai.com/v1"`, `wire_api = "responses"`, `env_key = "OPENAI_API_KEY"`,
   selected by `model_provider = "x"`; plus a second, unselected table
   `[model_providers.local]` with `base_url = "http://127.0.0.1:11434/v1"`. Pass: `api.openai.com` is
   `listed: "config"`, `kind: "other"`, decrypted, with the calls matched as in 1; one `local_model`
   note names `127.0.0.1:11434` and PRD-10 prints its line; the start event's `hosts_sha256` is the
   copy's hash. (A real host, not a pad-controlled one: a pad host would need a public name with a real
   certificate.)
4. **Dodger** (pad `--pre proxy-setting.sh`, PRD-12). Qwen Code with `"proxy": "http://127.0.0.1:9"`
   in `{home}/settings.json`. Pass: start refuses before launch, names the file and the key, and never
   prints the value; the same refusal for the workspace file `{workspace}/.qwen/settings.json`. Then Claude Code
   with the pad's `--hosts` copy whose `pinned` list holds `api.anthropic.com` (PRD-03): the harness
   still works, every connection to it is a tunnel with GAP `pinned`, zero decrypted requests to it.
5. **Daemons** (pad `--pre stale-gateway.sh`, PRD-12). Hermes with a gateway left running from a
   previous start (started without our env). Pass: `before` stops it; the run's model calls are
   decrypted; after exit no `hermes` process is left. Then Cline with a hub left running from a previous
   start, and OpenCode 2 with a background service left running: the run's model calls are decrypted and
   no old process made a model call. If no stop command exists, the facts sheet says so and the row
   stays "not verified".
6. **The other 12** (third slice). Each runs once with the pad prompt. Pass or fail goes into the
   PRD-12 table and `connectors/tiers.toml` with the harness version; `docs/harnesses/README.md` shows
   the tier from that file: "verified on <version>", "not verified" with its reason, or "not
   supported". Amp and Factory Droid: tier "not verified", reason "no key"; their pad row says "not run:
   no key". OpenClaw: C2 is expected INCONCLUSIVE "no subagent" (section 7 row 5).
7. **No secret.** For each run, the DeepSeek, Claude, OpenAI, Google and gateway keys appear nowhere
   in the record or the server copy. Only with the owner's yes to question 3 (M1a): also every value in
   the connector's `secret_files` that PRD-05 admitted (not counted in `mask_values.skipped`). PRD-08
   `secrets` also runs on OpenClaw's agent SQLite (section 5); OpenClaw stays "not verified" until it
   passes.

Automated tests that guard them (`crates/goalcup/tests/`):

- `connector_builtin.rs`: every file in `connectors/` parses, is in the built-in list, and its
  `native` patterns pass PRD-04's pattern check; `model-hosts.toml` parses and every entry passes
  `hosts::check(_, Shared)`; no native row pattern matches any of the connector's own `secret_files`
  (point 4: a key file is never snapshotted); duplicate native row names and a `skills_dir` under
  `{workspace}` are refused at load. One test, all files.
- `connector_no_model_address.rs`: each refused shape of section 2 (an `env` name with `BASE_URL`,
  `API_BASE`, `ENDPOINT`, `_HOST` or `PROXY`; a `://` value in `env`, `no_prompts` or `update_off`)
  fails `load`; all 16 shipped connectors load.
- `connector_hosts.rs`: exact, `*.suffix` and in-label `*` matching; `never` beats every source;
  goalcup's server is `never`; a `pinned` name answers `Listed { pinned: true }` with its kind;
  `ca_subtrees` for each pattern shape; `Participant` refuses a pattern wider than one label. The
  lesson cases N1 to N6 are PRD-17's `lessons_hosts.rs`, through the same `hosts::check`.
- `connector_hints.rs`: one key from each of JSON, TOML, YAML and dotenv, with `*` over keys and array
  indexes; a `hint_env` variable adds its host and an unnamed `DATABASE_HOST` does not; a loopback
  value gives one `local_model` note; plain `http` to a public host and IP values are skipped; a 2 MiB
  file skipped; a linked file is skipped; a BOM is stripped; an unselected `model_providers` table is
  hinted too.
- `connector_launch.rs`: the participant's `OPENAI_API_KEY` passes through; their `HTTPS_PROXY`,
  `HTTP_PROXY`, `ALL_PROXY`, `NO_PROXY` and CA variables are replaced; `unset` removes; the exact
  default set from section 3 is present after `finish_env`; `SSL_CERT_DIR`, `CURL_CA_BUNDLE` and
  `NODE_TLS_REJECT_UNAUTHORIZED` are never set by goalcup; `refuse_if_set` names file and key and never
  the value; a second plan on a locked home gives a refusal; the lock file ends `.goalcup.lock`; with
  `HOME` swapped, `sh -lc 'command -v goalcup'` finds it; `plan` writes every `update_off` row into the
  isolated home; `codex_gets_no_dash_c_override` (the Codex plan's argv holds no `-c`, PRD-17 HF5); a
  Claude Code proxy key in `{workspace}/.claude/settings.json` gives a refusal; `secret_values` is
  empty in the default build (with the owner's yes: carries key paths).

## Risks and blind spots

| Risk | What could go wrong | What we do |
|---|---|---|
| Whole-environment passthrough (owner question 2, old 13, default yes) | A participant's own `ANTHROPIC_BASE_URL` or cloud switch sends calls to a host we did not expect; a variable the participant forgot changes the harness | That is how they run it without goalcup (point 7); a variable the connector names in `hint_env` lists the host; any other unknown host is a tunnel the per-host flag shows. If the owner answers no, `Env.vars` becomes today's allowlist plus the connector's `hint_env` and the key variables of appendix O, and Crush, Kilo, Goose and Copilot then need their key in their home |
| Named hint variables only | A gateway set through a variable no connector names is a tunnel | The per-host flag (PRD-03) and the wire-against-native line (PRD-10) show it; `goalcup setup --record-host` fixes it; a missing name is one line in a connector |
| XDG homes | The agent's own tools that read `XDG_CONFIG_HOME` (for example `gh`) lose their login during OpenCode, Kilo, Goose and Crush runs | `GH_TOKEN` passes through; the guide says so (PRD-10). A narrower variable per harness, where one exists, is a later cleanup |
| `HOME` swap for Amp and Droid | git and ssh config of the participant are not visible to the agent | Said plainly at start (PRD-10); unverified whether Droid has a home variable (the builder checks `strings` for one in a build box) |
| `SSL_CERT_FILE` for every harness | A tool that trusted a corporate CA only through the participant's own `SSL_CERT_FILE` now fails on tunnelled hosts | The bundle holds the OS store; a corporate proxy chain is not supported in 1.0 (PRD-03 scope). Named in the guide |
| `refuse_if_set` | A participant cannot start until they remove a setting | It is before launch and names the fix; a run that records nothing is worth nothing (appendix N, Q8) |
| Host list rows not yet checked | Qwen, Mistral and the other added provider hosts may be wrong or incomplete; GitHub may route a plan to a host outside `*.githubcopilot.com` | Four rows were checked against vendor docs on 30 Oct (section 4); the rest stay unverified until the builder checks them; a wrong row only costs a tunnel, never a broken harness (PRD-03 refusal learning) |
| Gemini CLI on Google sign-in | Code Assist sign-in for individual accounts may be retired (one third-party report, unverified); the request shape differs from the Gemini API | `cloudcode-pa.googleapis.com` is listed as a vendor host, so calls are recorded either way; the pad uses the funded key, so the sign-in path is "not verified" |
| Decrypting `opencode.ai` | It also serves the install script and docs | Clients that read the bundle verify fine; a client that refuses our CA falls back to a tunnel after one failure (PRD-03) |
| Release pace | Amp about 7 builds a day; Copilot and Droid about daily; Kilo several a day | Versions recorded at every start; auto-update off where possible; the pad table names the version that passed |
| Secret files change during a run | A refreshed OAuth token is not in `secret_values` (in the default build nothing is) | Shape rules still apply (PRD-05); exact values and any later watch are PRD-05's, only with the owner's yes to question 3; the pad's no-secret check covers the keys it set |
| Tokens inside a snapshotted database | OpenClaw keeps auth profiles in its agent SQLite (H5.5); OpenCode may copy a key into its `credential` table (H4.5). A token there with no known shape is recorded raw | Shape rules only (section 5). Acceptance 7 runs PRD-08 `secrets` on OpenClaw's agent SQLite; OpenClaw stays "not verified" until it passes |
| OpenClaw Gateway | Subagents run in a daemon with its own env; a Gateway started outside goalcup calls around us | As builder notes step 4; acceptance 6 checks PRD-12 C2 for OpenClaw; tier "not verified: subagents need the Gateway" (owner list OW-10) |
| Hint file could not be read | A file over 1 MiB, linked, unreadable or unparsable names a model host we then do not decrypt | Skipped with a log line and one start line (section 2); the host is a tunnel the per-host flag shows |
| Cross-harness subagents | Qwen Code's and Hermes's children write session files into the participant's real home; the subagent check of the pad cannot see them | Wire only, named (section 8); RULES names them not allowed (owner question 4, old 14) |
| `update_off` in a settings file | A harness that rewrites its settings mid-run may still update | `plan` writes it again at every start and `continue`; the version is recorded at every start; an update lands only at the next `continue` |
| Hermes in-place compaction | Today's connector turned `compression.in_place` off so the JSON snapshot kept growing (D61); 1.0 reads `state.db`, not the snapshot, and no longer sets it | The wire holds every call; whether compaction rewrites `state.db` rows of a session is unverified; the Hermes pad run shows it, and a `no_prompts`-style setting can be added as data |
| Local model servers | Calls to Ollama or LM Studio on loopback are never recorded | Named by the `local_model` note and PRD-10's line; recording plain HTTP on loopback is out of 1.0 |
| OpenCode 2 under the 1.x connector | `version` refuses and points to `opencode2-cli` | One extra key in the schema; the alternative was a silent zero-call run |
| Hermes gateway with an old env | If `before` fails (old Hermes has no `-y`), the old gateway calls direct, not even tunnelled | Logged; the pad's check 5; tier stays "not verified" until it passes |
| macOS | Goose uses Apple folders, not XDG; `SSL_CERT_FILE` is ignored by the Security framework | Linux first (ledger T9); Goose and Hermes on macOS stay "not verified on macOS" until the owner runs them |

## Overridable defaults

| Default | Value | Who may change it |
|---|---|---|
| Environment | Participant's whole env, minus proxy and CA variables | Owner (question 2, old 13) |
| Hint variables | Only the names in each connector's `hint_env` | PRD-11 |
| `before`/`after` timeout | 10 s each | PRD-11 |
| Version probe | `--version`, 5 s | PRD-11 |
| Secret file values | Not read in the default build; with the owner's yes to question 3 (M1a), every string scalar with its key path, PRD-05's gate decides | Owner (question 3, old 12) |
| Hint file size guard | 1 MiB | PRD-11 |
| `CLAUDE_CODE_DISABLE_AGENT_VIEW=1` | on, said on Claude Code's first screen | Owner: PRD-00 question 2 (see owner list; it turns a feature off) |
| `CLAUDE_CODE_RETRY_WATCHDOG=1` | on | Owner (owner list OW-06; it changes how Claude Code retries; PRD-02 9.1 relies on it to ride over a recorder restart) |
| `pinned` list | empty | PRD-12 for the pad only |
| `update_off` | always written, with no question, by setup and by `plan` at every start, into goalcup's isolated home only (ledger H9) | Not overridable (ledger H9, locked) |
| `no_prompts` | written by setup after the one question, into goalcup's isolated home only | Owner (question 2, old 8) |

## Builder notes

Order of work (PRD-00 phase 0 step 7 and the three slices):

1. Phase 0: by H+1, replace the scaffold's placeholder `mod.rs` (PRD-00 phase 0 step 5) with the real
   S3 stub: the S3 types (`load`, `plan`, `Env`, `LaunchPlan`, `HostList`, `hosts::check`) as compiling
   stubs, with `plan` built from the Claude Code connector file (data), not from code. By H+1.5:
   publish `model-hosts.toml` and `schema.rs`, and write the Claude Code, Codex, Hermes and Kilo files.
   Before each file, open the vendor's current docs, or `--help` of the pinned version in a build box
   (ledger B1), and cite them in the commit (ledger T7). Check now: `codex --no-daemon` with
   `resume --last`, that Codex honours `check_for_update_on_startup` from `config.toml` and that the
   embedded-mode warning is gone without `-c`, Codex's `openai_base_url` key and state database name,
   `hermes gateway stop -y` on 0.22.5, `KILO_DISABLE_AUTOUPDATE` and the `kilo*.db*` name,
   `CLAUDE_CODE_DISABLE_AGENT_VIEW` and `DISABLE_UPDATES`. Others build against these.
2. Core slice: `hosts.rs` with the host check (copy first, PRD-17 ledger) and its test, then
   `launch.rs` (one generic planner), proven first on the Claude Code and Codex connector files
   (phase 1a needs those two plans), then `hints.rs`.
3. Second slice: Hermes's `before` and `after` against the watchdog, the gateway and dodger rows with
   PRD-12, `local_model` notes.
4. Third slice: the other 12 files. Before each, open the vendor's current docs or `--help` of the
   pinned version in a build box (ledger B1), cite them in the commit (ledger T7), and check every value
   marked unverified here: `OPENCLAW_HOME` layout, `GEMINI_CLI_HOME` layout (does it hold `.gemini/`?)
   and its settings file, `QWEN_HOME` layout, Copilot and Qwen `--continue`, Goose `session --resume`,
   Cline's hub with `CLINE_SESSION_BACKEND_MODE=local`, `CLINE_NO_AUTO_UPDATE` and `cline hub stop`
   (it must stop only the hub for this `CLINE_DIR`), Droid's home variable and `droid daemon stop`,
   OpenCode 2's background service, each harness's `no_prompts` values. OpenClaw: check whether its
   Gateway can be stopped for `OPENCLAW_HOME` only (`openclaw gateway stop` or the documented command).
   If yes, add `before` and `after` as for Hermes. If not, its facts sheet and tier reason say
   "subagents not recorded on the wire when they run in a Gateway started outside goalcup", and it
   ships "not verified". Never stop a system-wide service. For each listed vendor or gateway host, find
   the harness's sign-in or token host. A dedicated one goes in `never`. Where sign-in shares a listed
   host, the facts sheet says so and the pad's no-secret check covers it.
5. `docs/harnesses/*.md` last, from what the pad shows.

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
for `connector_sha256` and the three host lists; list `conn` streams to the harness's model host with
`mode: "tunnel"` (must be zero outside the dodger case), and check that a tunnel to any other listed
host carries a named reason (PRD-08 `named`); compare the harness's own session files (native
streams) with the wire requests, per subagent.

## Asks of other PRDs

The five asks of PRD-00 and the two of PRD-01 in revision 1 are closed: PRD-00 revision 2 took flat
files, the S3 shape, no credential, 16 files with one pad row, and the whole environment as owner
question 13 (now 2); PRD-01 has `connector_source`, `hosts` with `kind` and `pinned`, and `hosts_sha256`.
Asks 1 to 5 below were accepted by PRD-00 (R3-11); revision 3 changes ask 2 and adds asks 6 and 7. Left:

1. **PRD-02.** Carry `plan.notes` in the start facts and write them with `Recorder::event` after
   `start` on boot 0; keep the `LaunchPlan` alive (it holds the home lock) until the seal.
2. **PRD-10.** Drop start step 7 (helper stops are the watchdog's, S8); call `hosts::check` for
   `setup --record-host`; read `update_off` and `skills_dir` from `Connector` beside `no_prompts`; word
   `local_model` and "wire only". Call `connector::load(id_or_path, hosts_file)` then
   `connector::plan(&connector, &env, home)`; there is no `PlanCtx`. Setup writes `update_off` with no
   question, with `connector::write_settings`, and `no_prompts` after its one question (G3-X16); the
   auto-update switch is a separate key, not part of `no_prompts`, and `plan` applies it again at every
   start (drop "and auto-update" from setup step 3's question). Key the vendor backend line off `conn`
   meta `kind: "vendor"` in the record; the connector has no `vendor` key. Print the start line for a
   hint file `plan` could not read (section 2).
3. **PRD-12.** The dodger copy adds the harness's model host to the `pinned` list of a copy of
   `model-hosts.toml` (there is no per-entry `pinned = true` key); connectors are flat files, not
   folders; the override is `--connector` and `--hosts` (S16), not `GOALCUP_CONNECTORS_DIR`.
4. **PRD-03.** Section 3 says "`pinned = true`"; read it as "named in the `pinned` list" (section 4).
5. **PRD-15.** The guide and the tier table name "wire only" (section 8) and `local_model`. The one
   harness table is `docs/harnesses/README.md`; other docs link to it.
6. **PRD-12.** Add `--pre <script>` (run in the box after `goalcup setup --harness <id> --yes`, before
   `goalcup start`, output kept in `exec/<script>.txt`) and these scripts: `hint-local.sh` (writes the
   acceptance 3 tables into `{home}/config.toml` and the `--hosts` copy without `api.openai.com`),
   `proxy-setting.sh` (writes `"proxy": "http://127.0.0.1:9"` into Qwen's `{home}/settings.json`; the
   row passes when start refuses and names the file and key), `stale-gateway.sh` (starts
   `hermes gateway` with `HERMES_HOME={home}` and no proxy or CA variables, and leaves it running), and
   the same for Cline's hub and OpenCode 2's background service (acceptance 5).
7. **PRD-00.** S3 already says `LaunchPlan` is the block in section 5 and `host_hints` is
   `hosts: HostList` (PRD-00 G2-07, done). Left: drop `vendor = true` from the S3 list of connector
   fields kept for PRD-10 (alignment L3-11; the vendor kind lives in `model-hosts.toml`).

## Review answers

History of revision 2. Where an answer here disagrees with the body, the ledger or the "Alignment
answers" section (the `vendor` key, `update_off` written only by setup, `secret_values` by default,
the owner question numbers), the body, the ledger and "Alignment answers" win. It moves to
`REVIEW-HISTORY.md` in one later step (alignment X2-11).

Each item from the three reviews of 30 Oct that concerns this PRD. CC is the consistency check, A is
review A, B is review B.

| Item | Answer |
|---|---|
| CC 5 S3 sketch | Accepted. Section 5 now uses PRD-00's S3 names and signatures (`plan(&Connector, &Env, &Path)`, `Never`, `NativeSource` with `warn_missing`, `hosts::check`) and fills in the detail |
| CC 11, B C11 Flat files | Accepted (already flat); the pad table counts OpenCode once |
| CC 12 `src/watchdog/` | Accepted. "Must not touch" updated |
| CC 37 `start` keys | Accepted. `connector_source`, `hosts` with `kind` and `pinned`, `hosts_sha256` produced by `HostList::lists()` and `sha256()` |
| CC 41, B C4 Credential | Accepted. None in the launch env (unchanged) |
| CC 47 Helpers have no owner | Accepted. The watchdog runs `before` and `after` (S8); interface rows say so |
| CC 54 `decide()` return | Accepted. `Listed { source, kind, pinned } \| Never \| NotListed` |
| CC 55 Host list duplication | Accepted. Section 4 is the one list; PRD-03 keeps only matching rules |
| CC 56, B C5 PEM files | Accepted. PRD-03's code mints and writes, the watchdog calls it, folder `$GOALCUP_HOME/runs/<run>/<entry>/` |
| CC 59, 60 Root template, rows table | Accepted. Section 6 is the one source; PRD-04's two points taken (Hermes manifest row, Codex `*.sqlite*`) |
| CC 61, 73, B C14 Fields PRD-10 needs | Accepted. `first_screen`, `no_prompts`, `warn_missing` (per row), `vendor` added; also `skills_dir` (PRD-10 installs the skill) and `update_off` (owner question 8). Tier stays out (PRD-12's file) |
| CC 63, B C18 Secret value floor | Accepted. Candidates with key paths, no floor; PRD-05's gate decides (also PRD-05 ask 1) |
| CC 74, 75, B C12 One override | Accepted. `--connector` and `--hosts` only (S16), refused in competition mode |
| CC 97 16 files | Accepted. One pad row for OpenCode |
| CC 98, B M8 Tier file | Accepted. `connectors/tiers.toml` is PRD-12's; README's tier column is generated from it |
| CC 100 Contributor folder | Routed to PRD-12 (ask 3) |
| CC 109 `hosts::check` | Accepted. Section 4, one entry point for all sources, with the copied rules |
| CC 110 Host kinds | Accepted. Kinds read from the record; `other` for participant and config |
| A 6 Unlisted model host | Partly. Four host rows verified against vendor docs and Gemini's Code Assist host added; the per-host flag and the live line are PRD-03's and PRD-10's |
| A 7 Local model server | Accepted. `local_model` note from hints (section 3), acceptance 3; not recorded, named |
| A 16 Auto-update and cross-harness subagents | Partly. `update_off` written by setup into the isolated home (owner question 8). Cross-harness children: wire only, named (section 8); their homes are not redirected (per-harness code, hides their login) |
| B A4 Phase 1a slice | Accepted. Claude Code and Codex plans first (builder notes) |
| B A11 Dodger pass line | Accepted. Acceptance 4 states the observable pass, no exit code |
| B B4 16 files tonight | Accepted. Four in phase 0, the other 12 in the third slice |
| B N15 Env suffix scan | Accepted. Scan removed; named `hint_env` variables only. Whole environment kept as owner question 13 (default yes, PRD-00); the "no" fallback is in the risk table |
| B W3 No Factory key | Accepted. "not run: no key" for Amp and Factory; Factory key dropped from acceptance 7 |
| B W4 Pad-controlled gateway | Partly. A real host, as asked, but `api.openai.com` through a Codex `responses` provider instead of DeepSeek: Codex's support for a chat-completions provider is unverified, the responses route is documented |
| PRD-05 ask 1 | Accepted (CC 63). Its request to edit the old `codex-cli/harness.toml:110-113` is rejected: the old crates are excluded from phase 0 and the file is deleted at phase 3, and the new Codex file has no such rows |
| PRD-10 interface (delete `time_left_tool`) | Closed: the old files are deleted whole; the new schema has no such key |
| PRD-17 N1 to N8, P9 to P12, T6, X4 | Accepted. Host check, lock name, `.profile`, CA variables never set, key pass-through, one run per home, in sections 3 and 4 and the tests |
| PRD-17 F8 (Hermes compaction off) | Rejected for 1.0: the setting served the JSON snapshot that 1.0 no longer reads. Named in risks; data can add it if the Hermes pad run shows a loss |
| Owner's relayed question (gateway is an HTTPS proxy with another address) | Answered in the section above: yes, one pipe; vendor backends and local model servers named; dodgers ride the same pipeline with a label |

## Alignment answers

Revision 3, 30 October 2026. Fixes from the alignment fix lists (groups 1 to 3), checked against
OWNER-DECISIONS.md, PRD-00 revision 4 and the code. Only the PRD-11 part of each shared fix is made
here; the other files carry their own part.

### Group 1

| Fix | What was done |
|---|---|
| L4-1 | Applied (PRD-11 part). Default build: `secret_files` is not read for masking and `secret_values` is empty (section 2 row, section 5, `LaunchPlan` comment, PRD-05 interface row, acceptance 7, risks, defaults). The data rows stay. With the owner's yes as M1a, the old text is the plan |
| F3 | Applied (PRD-11 part). HF5 is guarded by `connector_launch.rs::codex_gets_no_dash_c_override`: the Codex `args` lose both `-c` overrides; `check_for_update_on_startup` moves to `update_off` in `config.toml`, and the trust question is said on `first_screen`. Builder step 1 checks both (unverified) |
| L3-1 | Partly, merged with G3-X16. `update_off` is not part of the `no_prompts` question; `plan` writes it at every `start` and `continue` before launch (section 5, `write_settings`), and `goalcup setup` also writes it with no question, as PRD-00 S15 and owner question 2 already say. One writer function serves both. Section 7 paragraph, risks and defaults rows updated. The PRD-10 lines are asked in ask 2 |
| L5-04 | Applied (PRD-11 part). Outlook rows 4 to 15 say a PRD-09 reader is written from the phase 5 sample. PRD-16 and PRD-12 parts are theirs |
| L5-05 | Applied. Claude Code refuses the same keys in `{workspace}/.claude/settings.json` and `settings.local.json`; Qwen Code in `{workspace}/.qwen/settings.json`; acceptance 4 and a launch test say so |
| L5-06 | Applied. Droid `before`/`after` `daemon stop` and Cline `before`/`after` `hub stop`, marked unverified; acceptance 5 adds Cline's hub and OpenCode 2's service; a missing stop command keeps the row "not verified" |
| L7-03 | Applied. Section 2 "No model address in connector data" load rule; test `connector_no_model_address.rs`. The 16 files were checked by eye: none is hit |
| F4 | Applied (PRD-11 part). Citations read HF4 and HF1 to HF8 |
| L3-2 | Applied. `CLAUDE_CODE_RETRY_WATCHDOG=1` kept; overridable-defaults row names the owner (OW-06) |
| L3-11 | Applied. `vendor` key removed from the schema, the Amp file and the field lists; PRD-10 words the line from `conn` meta `kind: "vendor"` (ask 2). PRD-00's S3 list still names `vendor = true`; ask 7 asks the replan to drop it |
| L5-08 | Applied, merged with G3-11a (builder step 1) |
| L5-09 | Applied. Dodger row of the shape table |
| L5-10 | Applied. macOS risk row: Goose and Hermes stay "not verified on macOS"; no new work for the owner's Mac run |
| L5-11 | Applied. Claude Code `unset = ["CLAUDE_CODE_CERT_STORE"]` and `env.no_proxy`, `env.NODE_EXTRA_CA_CERTS` refused; Codex `.env` adds `all_proxy` and `NODE_EXTRA_CA_CERTS` |
| L5-13 | Applied (PRD-11 part), merged with G3-11b. Row 5 outlook, OpenClaw note in section 6, acceptance 6. The `harness_subagents_unavailable` cause is PRD-12's |
| L7-06 | Applied. Builder steps 1 and 2: `plan` built from the connector file (data); one generic planner |

### Group 2

| Fix | What was done |
|---|---|
| G2-06 | Applied. Builder step 1: replace the scaffold's placeholder with the real S3 stub by H+1 |
| G2-08 | Applied (PRD-11 part). `HostList`, `NativeSources`, `MaskRule` derive `Serialize` and `Deserialize` (section 5); PRD-02 interface row names S17 and `notes` |
| P04-15 | Applied. `connector_builtin.rs` refuses a native row whose pattern matches one of the connector's own `secret_files` (no check in `src/native/`) |
| P04-16 | Applied. Native row names unique within a connector, refused at load (section 2), with PRD-04's first-row rule restated |
| X2-09 | Applied (PRD-11 part). Fact pages hold facts and limits only; the pad recipe lives only in `scripts/pad/harnesses/<id>.toml`; `docs/harnesses/README.md` is the one harness table. PRD-15 lines are PRD-15's |
| X2-11 | Partly. This pass edits only this file, so the move to `REVIEW-HISTORY.md` is left to the one later step that owns that file. "Review answers" opens with a line saying it is history and that the body, the ledger and these answers win; the asks say which were ruled (R3-11). Every ruling still binding is in the body |

### Group 3

| Fix | What was done |
|---|---|
| G3-X16 | Applied, merged with L3-1: `update_off` always written, no question, also with `--yes` (row, section 7, defaults: not overridable, H9 locked; ask 2) |
| G3-X35 | Applied. Ask 6 to PRD-12 (`--pre` and the scripts, plus Cline's hub and OpenCode 2's service for acceptance 5); acceptance 3, 4 and 5 name the script |
| G3-11a | Applied. Builder step 1 opens the docs or `--help` in a build box, cites them, and lists the checks; the Codex items left step 4 |
| G3-11b | Applied. Builder step 4 OpenClaw Gateway check, never a system-wide stop; risks row; section 6 note |
| G3-11c | Applied, in the default-build form of L4-1: tokens in OpenClaw's agent SQLite are masked by shape rules only; acceptance 7 runs PRD-08 `secrets` on it; risks row |
| G3-X57 | Applied. Claude Code `first_screen` says agent view is off; defaults row names PRD-00 question 2 and the owner list (G3-O3) |
| G3-11d | Applied. Owner questions written as "owner question N (old M)" in the body; the history section keeps the old numbers |
| G3-11e | Applied. Files it owns: "PRD-00 row: 800 src, 400 tests" |
| G3-11f | Applied. Section 6 build order |
| G3-X41 | Applied. Ask 2 names `load` then `plan`, no `PlanCtx`. Ask 7's S3 part is already in PRD-00 (G2-07), so ask 7 only names the `vendor` line |
| G3-11g | Applied. Acceptance 6 tier wording; Amp and Droid "not verified", reason "no key" |
| G3-11h | Applied. Acceptance 1 and the watcher note: zero tunnels to the harness's model host; other listed-host tunnels carry a named reason |
| G3-11i | Applied. Shape section Factory text; builder step 4 sign-in host check |
| G3-11j | Applied. "In a build box (ledger B1)" in step 4 and the `HOME` swap risk row |
| G3-11k | Applied. Section 2 file rules (no link, BOM stripped, every provider table hinted); `connector_hints.rs` cases |
| G3-11l | Applied. Section 2 skip line and its start line; risks row; ask 2 |
| G3-11m | Applied with L5-06: the Droid lines are real `before`/`after` entries marked unverified, removed if the command does not exist, rather than a comment line, so the builder has one shape to confirm |
| G3-11n | Applied, conditional on the owner's yes (L4-1) |
| G3-11o | Applied. `builtin.rs` includes a list generated by `build.rs` (section 1) |
| G3-11p | Applied. Today table: `schema.rs:214-1066`, `pins.rs:32`, `LC_*` (checked in the code) |
| G3-X38 | Applied. `skills_dir` only under `{home}`; loader refuses `{workspace}`; tested in `connector_builtin.rs` |

## Amendments

### 2026-10-30, replan after PRD-15 slice 3 (merge 4e0ca793)

- **Harness-page evidence links must survive the after-merge purge** (PRD-15 review A7). The TODO
  purge deletes `docs/research/` except run 1's report, `2026-10-vercel-gateway.md`,
  `VERCEL-COMPAT.md` and `m2-report-format.md`. On integration at 4e0ca793 the facts sheets link
  purged files: `docs/harnesses/amp-cli.md:228-229`, `claude-code-cli.md:95` and `:158-159`,
  `codex-cli.md:239-240`, `hermes-cli.md:214-215` (22 Oct local test day, fix plan v4, 26 Oct
  results). When a page is rewritten for 1.0, its evidence cites run 1's report or the pad rows
  (PRD-12), or the fact stands without a link; no new link goes into a purged path. Pages this build
  does not rewrite keep the links, and the purge's own `check-links.py` run fixes what is left.
- The three TODO anchor links PRD-15 asked about are handled on
  `v1.0/prd-11-connectors-harness-page-todo-links` (harnesses log 12:22Z) and land with the lane's
  next merge.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-04 core slice part 1 (merge 403d9165)

- **PRD-04's pattern check is on integration** as `crate::native::pattern::parse` (merge 403d9165).
  `connector_builtin.rs` calls it for every `native` row. It refuses one shape more than PRD-04
  section 2 lists: a trailing `**` (`**` alone, or a pattern ending in `/**`), since `**` matches
  folders only. No row in section 6 or in `connectors/*.toml` ends in `**`; a row that names every
  file below a folder writes `<folder>/**/*`.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-11 slice 2 (merge b9fa50f8)

- **S3 is on integration as a skeleton.** Real: `load`, `plan`, `Env`, `LaunchPlan`, `finish_env`,
  `HostList::from_file`, `lists` and `sha256`, `NativeSources`, `MaskRule`, `ConnectorError`, the
  loader checks and the env build. Stubs until the next slice: `HostList::decide` (every name
  `NotListed`, so every host tunnels), `ca_subtrees` (empty), `hosts::check` (lowercase and label
  shape only), `HomeLock::take` (holds nothing), the version probe (`None`), `write_settings` (writes
  nothing), `hints::collect` and `hints::refusal` (find nothing), and the `record-hosts.txt` read. A run
  on this build is not evidence.
- **S3 shape additions** (PRD-00 note): `LaunchPlan.ca_env: Vec<String>` (the connector's CA
  variables; `finish_env` sets them); `before` and `after` entries hold the arguments without the
  program, and the watchdog runs `program` followed by them. `Refusal` is `SettingSet { file, key }`,
  `HomeLocked { home }` or `VersionMismatch { program, want }`. `LaunchPlan` is not `Clone` (it owns
  the lock).
- **`builtin.rs` is a hand-written `(id, text)` list**, not a list generated by `build.rs` (section 1,
  and G3-11o, are superseded). PRD-00's ownership row ("builtin (include_str! list)") and PRD-14's
  `rule_code` (its one allowed file) already describe it this way, and `crates/goalcup/build.rs` has no
  owner. `connector_builtin.rs` fails when a `connectors/*.toml` file is missing from the list. At 16
  files rustfmt makes it about 70 lines; the third slice may use a small `macro_rules!` around
  `include_str!(concat!(...))` to keep one line per connector.
- **Loader rule narrowed (review F1b).** `no_prompts` and `update_off` rows take a `file` under
  `{home}` only, as `skills_dir` does. `{workspace}` stays allowed for `refuse_if_set`, `host_hints`
  and `secret_files`, which are only read. Section 2 says both lists are written into the isolated home,
  and `write_settings(home, rows)` has no workspace: a `{workspace}` row would be skipped forever or
  write into the participant's project. No shipped row uses `{workspace}` there.
- **One pattern check (review B7).** At b9fa50f8 `schema.rs` still has its own `pattern_ok`, which
  accepts a trailing `**` that PRD-04's `native::pattern::parse` refuses, and `connector_builtin.rs`
  has its own glob matcher (about 26 lines). The PRD-04 replan note of 403d9165 says the connector test
  calls `parse`; it does not yet. The next slice makes the loader call `crate::native::pattern::parse`
  for every `native` row, drops `pattern_ok`, and has the key-file test use `Pattern::matches`, as the
  Acceptance list already says ("pass PRD-04's pattern check").
- **Formats for the next slice** (lane log 10:57Z and 11:00Z). A hint or setting file whose name ends
  `.jsonc` is read as JSON with comments (Kilo writes `kilo.jsonc`), not as dotenv. `SettingRow.value`
  stays a string in the connector; `write_settings` writes `"true"` and `"false"` as the target
  format's boolean (Hermes tests `updates.check` with `is False`, so a string leaves the check on) and
  every other value as a string.
- **Open nits** go with the launch task of the next slice, since no other task there edits these files:
  F5b (`safe_message` cuts at the first `", expected"`, which can sit inside a quoted value; keep only
  the prefix word, or use `rfind`), F1b (above, with one refused case in `connector_builtin.rs`), B12
  (drop the first loop of `every_connector_file_is_built_in_and_loads_by_its_id`; the second covers
  it), B13 (`LaunchPlan::home` doc: created unless `refusal` is a held lock). F3's lock test lands with
  `HomeLock`. Acceptance lists any still open at the end as known.
- **Size (ledger L2; PRD-00 row "Connector code (PRD-11)", 800 src and 400 tests).** `wc -l` at
  b9fa50f8: src 1,149 (`mod.rs` 290, `schema.rs` 446, `hosts.rs` 229, `launch.rs` 136, `builtin.rs`
  27, `hints.rs` 21), 144% of the row with three files still stubs; tests 384 (`connector_builtin.rs`
  195, `connector_launch.rs` 147, `connector_no_model_address.rs` 42), 96% of the row with two test
  files to come. The 800 is the sum of the estimates in "Files it owns", which leave out `mod.rs` (the
  S3 types every user builds against) and put `schema.rs` at 170 (it holds the serde types, the PRD-05
  mask grammar check and section 2's rules, each error naming file and key, never a value).
  **Decision:** the area's ceiling is 1,510 src and 620 tests by `wc -l` at the end of the third slice.
  The extra lines serve point 4 (load errors never name a value; every path stays in its root; no
  native row reads a key file) and point 7 (four config formats read for hints and three written for
  settings, so the harness runs as the participant configured it). Per-file ceilings: `mod.rs` 285,
  `schema.rs` 425, `builtin.rs` 40, `hosts.rs` 290, `hints.rs` 220, `launch.rs` 250; tests
  `connector_builtin.rs` 165, `connector_no_model_address.rs` 45, `connector_launch.rs` 210,
  `connector_hosts.rs` 90 (the N1 to N6 cases stay in PRD-17's `lessons_hosts.rs`, not repeated),
  `connector_hints.rs` 110. Cut first: `pattern_ok` and the test glob (B7), B12, `safe_message` down
  to the prefix word. Reviewer B checks each merge against these numbers; past them needs another
  decision line.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-14 slice 1 (merge 22fe5926)

- **`rule_code.rs` reads every flat connector file at test time.** For each `connectors/*.toml` other
  than `model-hosts.toml` and `tiers.toml` it bans the file stem and the `program` value as whole
  tokens, case-insensitively, in string literals under `crates/goalcup/src` and
  `crates/goalcup-format/src`. It finds `program` as a line that starts exactly `program = "` and
  panics when none is there, so a connector writes `program = "name"` unindented, double-quoted, on
  one line. A new non-connector `.toml` placed flat in `connectors/` must be added to its leave-out
  list in the same merge (a PRD-14 edit). The `opencode`, `openclaw` and `amp` connectors become
  banned words when they land; `amp` is also an ordinary text (`&amp;`), so a merge that adds
  `connectors/amp-cli.toml` runs `rule_code` first and, on a hit, adds the literal to its `ALLOWED`
  list with a reason. The program name stays as the vendor ships it.
- `src/connector/builtin.rs` stays the one allowed file (PRD-14 note of b9fa50f8).

### 2026-10-30, replan after PRD-15 slice 5 (merge ea0a0fec)

- `docs/CONNECTOR-SCHEMA.md` on integration now describes the merged `connector/` code and marks eight
  places **(next slice)** with what the stubs do today: the version probe, `refuse_if_set`,
  `host_hints`, `hint_env`, a pattern ending in `**`, the settings files (`.jsonc`, `no_prompts` and
  `update_off` under `{home}` only, booleans), host decisions (today every host is a tunnel) and the
  three host sources with the full host check. The merge note of PRD-11's next slice names the rules it makes
  real, so PRD-15 can drop those marks and check each against the merged `hosts.rs`, `hints.rs`, `launch.rs`
  and `mod.rs`. A rule the slice rules differently from the page is a PRD-15 fix, not a PRD-11 one.
- The launch slice's `.profile` (PRD-17 P11) is not on the page; PRD-15's ARCHITECTURE rewrite states
  it from merged `launch.rs`.

### 2026-10-30, replan after PRD-11 slice 3 (merge c14d63b8)

- **S3 is real on integration.** `hosts::check` (the full PRD-17 2.7 rules), `HostList::decide`,
  `ca_subtrees`, the `record-hosts.txt` read, `hints` (JSON read as JSONC for every `.json` and
  `.jsonc` file, TOML, block YAML, dotenv), `refuse_if_set`, `HomeLock` (`try_lock` on
  `<home>.goalcup.lock`), the version probe (5 s, never logged) and `write_settings` (typed booleans,
  JSON, TOML and YAML only; dotenv and other formats come back skipped as "format not writable"). A run
  on this build tunnels only what the lists say. Facts the merge settled that the sections above leave
  open:
  - A `file` template's last component is a PRD-04 pattern (`pattern::parse`): `host_hints` and
    `refuse_if_set` read every regular file that fits; a folder that fits is skipped.
  - `plan` writes `update_off` at every start while the home lock is free, and logs a row not
    written ("update_off not written: ..."); `no_prompts` is written only by `goalcup setup`.
  - A JSON or TOML settings file that needs a change is rewritten without its comments (it is in the
    isolated home); a file already holding the value is not rewritten.
  - `Skipped.local` is true for loopback, unspecified, mapped loopback, private and link-local
    addresses and the `localhost` forms, so a hint at a LAN model server gives a `local_model` note.
    A numeric spelling such as `127.1` or `0x7f.1` is refused with `local` false: the hint is a
    plain skip, not a `local_model` note. Known; nothing is decrypted either way.
  - N6 asks the system resolver once per hint name with `to_socket_addrs` during `plan`, with no
    wait of our own: a dead resolver delays the start by the system's timeout per hint name. Known
    (point 3); the third slice bounds it only if it fits the ceiling below.
- **S3 additions** `LaunchPlan.unread: Vec<PathBuf>` and `LaunchPlan.log: Vec<String>` (PRD-00
  note). Consumer: PRD-10's slice that wires `connector::plan` into `start`.
- **Open nits closed:** B7 (the loader calls `native::pattern::parse`; `pattern_ok` and the test glob
  are gone), F5b, F1b with its refused case, B12, B13, and F3's lock test. Review rounds A and B on
  slice 3 closed with no open finding; B-N-SIZE went to this replan.
- **Size (ledger L2), `wc -l` at c14d63b8.** src 1,646 against the 1,510 ceiling (109%): `schema.rs`
  423/425, `mod.rs` 301/285, `hosts.rs` 301/290, `hints.rs` 303/220, `launch.rs` 291/250,
  `builtin.rs` 27/40. Tests 721 against 620 (116%, past the 10%): `connector_builtin.rs` 155/165,
  `connector_hints.rs` 196/110, `connector_hosts.rs` 105/90, `connector_launch.rs` 223/210,
  `connector_no_model_address.rs` 42/45. **Decision:** the area's final ceiling is 1,700 src and 760
  tests by `wc -l` at the end of the third slice; per-file ceilings are today's counts, except
  `launch.rs` and `mod.rs`, which share the 54 src lines left for the `.profile` writer (P11) and
  its call in `plan`, and `connector_launch.rs`, which gets the 39 test lines for its one case. The
  extra lines serve point 1 and point 7 (YAML lists and block scalars, JSONC for every JSON file, the
  `never` hosts in the list before any hint, typed booleans in three formats, so the harness runs as
  configured and every model host it names is recorded), point 4 (no value in any skip, log line or
  refusal) and point 6 (`unread` and `log`, so the participant is told which file was not read).
  Each `connector_hints.rs` case is an acceptance case (one per format, link and size guards,
  `local_model`, refusal, `false` and empty). The third slice cuts before it adds: first any
  repeated per-format setup in `connector_hints.rs`; nothing in the third slice's connector files
  counts here. Past 1,700 or 760 needs another decision line.
- **Third slice, unchanged in scope:** the `.profile` writer (P11) for Amp and Droid, and the
  remaining connector files of section 6 (four of 16 are on integration: `claude-code-cli`,
  `codex-cli`, `hermes-cli`, `kilo-cli`). Each new file is checked by `rule_code` (PRD-14 note).
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-12 slice 4 (merge 69f355c6)

- The pad now reads the flat `connectors/<id>.toml`: top-level `home` (it fills `{home}` only, for
  the harness login in the box) and `native` rows `name`, `root` and `pattern`. A `home` value that
  uses `{workspace}`, or a rename of those keys, is a seam line; the pad follows.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-10 slice 4 (merge a5d7ddc3)

- **`setup` copies one call of `write_settings`** (review B9): to report and save the file a row
  went to, `cli/setup.rs` fills each row's file with the same `fill(&row.file, home, Path::new(""))`
  as `connector::launch::write_settings` and calls `write_settings` once per row. The cleaner cut is
  PRD-11's, at its next touch of `launch.rs`: `write_settings` returns, for each row, the filled
  file and whether it was written (or the row index with each `Skipped`). PRD-10 then drops its own
  `fill` call in the same merge. Until then, a change to how `write_settings` fills a row's file is
  a seam line, and PRD-10 follows in the same merge, or setup prints and saves a path that was never
  written.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-11 slice 4, part 1 (merge a96d9f71)

- **On integration now:** ten connector files (`claude-code-cli`, `codex-cli`, `copilot-cli`,
  `gemini-cli`, `hermes-cli`, `kilo-cli`, `openclaw-cli`, `opencode-cli`, `opencode2-cli`,
  `qwen-cli`), `builtin.rs` in the one-line `entry!` form (21 lines at 10 ids; 27 at 16), the
  `.profile` writer (PRD-17 P11) and the 15-row harness table in `docs/harnesses/README.md`, which
  is the one harness table: other pages link to it and never copy it. Facts the merge settled:
  - **Version check:** a word of the first non-empty line of `program --version`, a leading `v`
    stripped, must start with `version` (`"2."` takes `v2.1.0`, never `1.12`). A version wrapped in
    punctuation, such as `(2.1.0)`, is not a word start and refuses; OpenCode's two connectors are
    told apart only by this rule, so the pad's first row of each is its proof on a real binary.
  - **`.profile`:** written by `plan` at every start while the home lock is free, only where the
    connector's `home` sets `HOME` and the harness environment has `PATH`, into the filled `HOME`
    folder. Only a `.profile` that starts with goalcup's own header line is replaced (beside and
    renamed over, never through a link); any other file is kept and `log` gets "kept a .profile
    goalcup did not write"; another error gives "could not write .profile: <kind>". No connector on
    integration sets `HOME`, so the first real use is Amp and Droid in slice 5.
  - OpenClaw's `openclaw.json` is JSON5 when hand-written; one that does not parse as JSON is named
    in `unread`, so its proxy keys are not checked and the participant is told (known, README row 5).
- **Not done in this part, moved:** the new-shape facts sheets for `claude-code-cli`, `codex-cli`,
  `hermes-cli` and `kilo-cli`. The README now says the sheets stay in the old shape and are rewritten
  from what the pad shows. All 15 sheets (and the group-A sheets planned for slice 5) move to the
  last slice, after the pad's rows, with the README tier refresh. Slice 5 keeps amp, droid, goose,
  crush, kimi-code and cline with their six `entry!` lines.
- **Still open from the PRD-10 note (review B9):** `write_settings` still returns only the skips, so
  `cli/setup.rs` keeps its own `fill` copy and the seam line rule of that note holds.
- **Size (ledger L2), `wc -l` at a96d9f71.** src 1,689 of the 1,700 ceiling: `schema.rs` 423,
  `mod.rs` 317, `launch.rs` 324, `hints.rs` 303, `hosts.rs` 301, `builtin.rs` 21. Tests 759 of 760:
  `connector_launch.rs` 261, `connector_hints.rs` 196, `connector_builtin.rs` 155,
  `connector_hosts.rs` 105, `connector_no_model_address.rs` 42. Slice 5's six `entry!` lines bring
  src to 1,695. **Decision:** the 1,700 and 760 ceilings stand. The B9 change is the one exception:
  it may pass 1,700 by at most the lines PRD-10 drops from `cli/setup.rs` in the same merge, so the
  crate total does not grow (point 6: setup reports the file that was really written). It adds no
  test file case; its proof is PRD-10's existing setup test. Anything else past 1,700 or 760 needs
  another decision line, and a new connector adds only data and its `entry!` line.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-12 slice 5 (merge 9e71bde0): pad data and watcher scripts for your acceptance 3, 4 and 5

- **Watcher scripts exist** under `scripts/pad/exec/`: `hint-local.sh` (acceptance 3, Codex: a
  `[model_providers.x]` for `api.openai.com` and an unselected local one, and a hosts copy without
  `api.openai.com`), `proxy-setting.sh` (acceptance 4: `proxy` in the harness's `settings.json`;
  `goalcup start` must refuse and name file and key, and not print the value), `stale-gateway.sh`
  (acceptance 5: the harness file's `daemon` command started under `env -i` with the connector's home
  names, left running for `start`; the pad then looks twice that no such process lives).
- **Pad data in the harness files, not connector data:** `model_host` (the dodger's pinned host) and
  `daemon`. The facts sheet rewrite of PRD-11's last slice reads these from `scripts/pad/harnesses/`
  and the rows' `row.json` (`exec`, `notes`); a connector with a daemon (Cline's hub, OpenCode 2's
  service) needs its `daemon` value for acceptance 5 to be a real run.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-31, replan after PRD-11 slice 5 (merge 7a647ba4)

- **On integration now:** all 16 connector files (`amp-cli`, `cline-cli`, `crush-cli`, `droid-cli`,
  `goose-cli`, `kimi-code-cli` added), 16 `entry!` lines, the sign-in and host rows, and README rows 6, 9
  and 12 to 15. `gate fast: PASS in 131s` on 7a647ba4 (integrator 00:12Z). Box runs before the merge:
  fmt, `clippy -D warnings`, 27 of 27 connector and `rule_code` tests. Not run: any real harness on these
  six (that is the pad's job), `gate full`, macOS.
- **Facts the slice settled, for the facts sheets:**
  - Amp is the only connector that sets `HOME` (plus the four XDG folders). Droid uses
    `FACTORY_HOME_OVERRIDE`, Goose `GOOSE_PATH_ROOT`, Kimi `KIMI_CODE_HOME`, Cline `CLINE_DIR` and
    `CLINE_DATA_DIR`, Crush the four XDG folders. So the first real use of the `.profile` writer is Amp
    alone, not "Amp and Droid" (the slice 4 note and the PRD-10 note said both). Amp has no key this cycle,
    so no pad row proves P11 on a real binary; the unit test stays the proof and the facts sheet says so.
  - Cline 3.0.74 has no continue form, so its connector has an empty `resume` and `goalcup continue`
    starts a new Cline session (README row 15). Droid has no `daemon stop`, so it has no `before` or
    `after` and the facts sheet names the daemon as a dodge. Cline's `hub stop` is scoped by
    `CLINE_DATA_DIR`.
  - Crush's database sits in the workspace (`{workspace}/.crush/crush.db`). Crush uses the closest
    existing `.crush` between the working folder and the git root, so a run below a repository that
    already has one writes there and the native root misses it (README row 12); the workspace file also
    holds the participant's earlier Crush sessions for that folder.
  - Amp's thread log folder (`.cache/amp/logs/threads`) is kept from a live run on an older version and
    is not confirmable from the package; the pad's first Amp row is its proof.
- **Last slice, data fix in `connectors/goose-cli.toml` (review nits A2-N1, A2-N2, no code):**
  1. Add `GONDOLA_HOST` and `ATOMIC_CHAT_HOST` to `hint_env` and change the comment above it to "the
     names those pages list on <date>"; it must not say "every".
  2. Known item for the Goose sheet: a host variable without a scheme (`OLLAMA_HOST` defaults to
     `localhost:11434`, `OPENAI_HOST` to `api.openai.com`) is skipped by `authority()` with
     "not a URL, skipped" naming the variable, so a schemeless `OLLAMA_HOST` gives no `local_model` line
     and a schemeless custom `OPENAI_HOST` stays a tunnel. Nothing is lost silently. Accepting a schemeless
     hint is a `hints.rs` change and is not made (1,695 of 1,700).
  3. **Decision:** `DATABRICKS_HOST` and `SNOWFLAKE_HOST` stay in `hint_env`. They are the providers' own
     names, a Goose user on those providers needs them, secrets stay masked, and goalcup sets
     `SSL_CERT_FILE` and `REQUESTS_CA_BUNDLE`. The Goose sheet says in the limits section that a
     Databricks or Snowflake host exported in the shell is decrypted and recorded for the run whoever
     uses it, including the agent's shell tools and their query traffic. Section 3's rule (hints are
     named, never scanned) is unchanged.
- **Size (ledger L2), `wc -l` at 7a647ba4.** src 1,695 of the 1,700 ceiling (`schema.rs` 423,
  `launch.rs` 324, `mod.rs` 317, `hints.rs` 303, `hosts.rs` 301, `builtin.rs` 27). Tests 759 of 760
  (`connector_launch.rs` 261, `connector_hints.rs` 196, `connector_builtin.rs` 155,
  `connector_hosts.rs` 105, `connector_no_model_address.rs` 42). The B9 exception is still open and
  unused. The last slice adds only data, sheets and the README tier refresh, no src line.
- **Left for the last slice, in order:** the goose data fix above; the 15 or 16 new-shape facts sheets
  and the tier refresh after the pad's rows (unchanged from the slice 4 note); drop the B9 `fill` copy
  with PRD-10 if still open.
- No locked ledger item is touched and no owner question comes from this slice.
