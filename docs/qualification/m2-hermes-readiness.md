# Hermes M2 readiness contract

> **Current (2026-10-02).** A source-only contract. [Reference row 1](m2-hermes-reference-row.md) exercised its runtime gates, and the seams listed as missing under "Native snapshot activation and collection" are built (`shipped_source` in `runtime/capture/native_scope.rs`). Index: [README](README.md).

- Date: 2026-10-01; provider section updated 2026-10-02 for D40 and D41
- Installed candidate: `hermes-agent==0.19.0`
- Reference row 1 (2026-10-02): **PASS**, sealed complete capture; see [m2-hermes-reference-row.md](m2-hermes-reference-row.md).
- Previous result: **READY_FOR_SEAMS (source-only)**. Every launch input is
  determined: provider `deepseek` and `DEEPSEEK_API_KEY` (D40); model
  `deepseek-v4-flash` with thinking off, CA variable `HERMES_CA_BUNDLE`, and the
  key-fingerprint and key-fragment rules (D41). Nothing here is a runtime claim, and
  every runtime gate below is still open.

This document freezes the source-backed launch, identity, isolation, native-output,
and transport contract for the installed Hermes candidate. It does not claim a
working authenticated run. No credential file or credential value was read, no
Hermes command was invoked, and no model or network request was made. The
2026-10-02 update is source reading only.

For source references below:

```text
$HERMES_ENV = <repo>/harnesses/uv-tools/hermes-agent
$SITE       = $HERMES_ENV/lib/python3.13/site-packages
```

## Frozen installed identity

| Item | Observed identity | Result |
|---|---|---|
| Public executable | `<repo>/harnesses/uv-bin/hermes` -> `$HERMES_ENV/bin/hermes` | Source inspected |
| Console script | 372 bytes; SHA-256 `0444572f0bce8a805473fbcaa431beed399818528758b454fbf24825a51be325`; entry point `hermes_cli.main:main` | Pinned |
| Distribution | `hermes-agent==0.19.0`; `Requires-Python: >=3.11,<3.14` | Pinned |
| Interpreter | CPython `3.13.13`; arm64 Mach-O target `~/.local/share/uv/python/cpython-3.13-macos-aarch64-none/bin/python3.13`; 49,968 bytes; SHA-256 `3237648c5222017bba78737370570e4c9d5a01e552cdf2fa11f107c8d00fc06e` | Pinned |
| Installer metadata | uv `0.11.7`; system site packages disabled | Pinned |
| Distribution record | `$SITE/hermes_agent-0.19.0.dist-info/RECORD`; SHA-256 `fc484f3dc2445fac3ed45e661dc3e438cc4418b8285eef9b20b7603516a62c50` | Pinned |
| Safe installed-file manifest | 5,207 files; SHA-256 `ff5a977d82b2da550f732f7de1a22f6d26292f88a4887264cba6dc740e967c75` | Pinned |
| Hermes Python-source manifest | 572 files; SHA-256 `fc6e3e8436bc1b653f043706351c1f2c7deb52ba8ce153df6b12a74b401dec7e` | Pinned |
| Installed dependency identities | 70 `Name==Version` rows; SHA-256 `5c7d00df0886a08514839d541899bd57d505362f57dce9408b6bd7b14000691e` | Pinned |

The file inventory was produced from `rg --files --hidden --no-ignore -0`
over `$SITE`. The safe manifest excludes bytecode and cache directories, `.env`,
`auth.json`, `state.db`, session/log/history directories, and `.hermes_history`.
Each manifest row is `<file-sha256><two spaces><site-relative-path>`, sorted by
relative path, terminated by `\n`; the table records SHA-256 of the concatenated
rows. The source subset is top-level Python plus `agent/`, `hermes_cli/`,
`tools/`, `providers/`, and `gateway/`, excluding tests. The dependency manifest
contains only `Name==Version` extracted from installed `*.dist-info/METADATA`,
case-insensitively sorted. No home, auth, dotenv, token, session, or history data
participated.

Selected installed dependency identities are `openai==2.24.0`, `httpx==0.28.1`,
`certifi==2026.5.20`, `requests==2.33.0`, `websockets==15.0.1`,
`PyYAML==6.0.3`, and `pydantic==2.13.4`.

## Exact launch contract

The source-backed candidate argv is:

```text
[
  "<repo>/harnesses/uv-bin/hermes",
  "chat",
  "-Q",
  "--provider", "deepseek",
  "--model", "deepseek-v4-flash",
  "--query=" + PROMPT
]
```

The goal must be one argv element, `--query=<goal>`. Goal Cup's capture template
convention (`args` ending in `--`, goal appended after it) does not work here. The
pinned CPython 3.13.13 `argparse` gives an option that takes one value the pattern
`([A])`, with no `--` allowed before the value
(`~/.local/share/uv/python/cpython-3.13-macos-aarch64-none/lib/python3.13/argparse.py:2451`),
so `-q -- <goal>` is refused as "expected one argument". `chat` has no positional
query (`$SITE/hermes_cli/_parser.py:274-276`). The `--query=<goal>` spelling also
stops a goal that starts with `-` from being read as an option. It matters for a
second reason: before argparse runs, Hermes scans raw `sys.argv` element by element.
A `-p`/`--profile`/`--profile=` token anywhere switches `HERMES_HOME`, and `--tui`,
`--cli` and `-h` are matched the same way (`$SITE/hermes_cli/main.py:284-286,529-560,13161`).
With `-q <goal>`, a goal reading `--profile=x` or `--tui` would be acted on. Because
`argparse` splits an option only at the first `=`, `--query=<goal>` keeps any goal as
one value.

`chat`, `-q`, `--model`, `--provider`, and `-Q` are defined at
`$SITE/hermes_cli/_parser.py:269-331`; dispatch into `cli.main` is at
`$SITE/hermes_cli/main.py:2593-2619`. `-Q` is required: the quiet path suppresses
the banner, spinner, and streaming display, prints the final response once to
stdout, and emits session information to stderr. The `-z` one-shot path is not
usable because it does not emit this session carrier and has different approval
semantics.

The provider is the literal `deepseek` (D40). `auto` is rejected:
`$SITE/cli.py:3913-3918` takes the provider from the CLI argument first, then config
`model.provider`, then `HERMES_INFERENCE_PROVIDER`, then `auto`, and the `auto` path
(`$SITE/hermes_cli/auth.py:1733-1808`) selects heuristically from the environment,
which cannot prove which provider, host, or key was used. The model is
`deepseek-v4-flash` (D41, owner-confirmed; see "Model ID"). The argv is complete.
It still needs the capture-profile seams before Goal Cup can launch it.

The adapter must launch with a constructed allowlist environment, not
`os.environ` inheritance:

| Variable | Contract |
|---|---|
| `HOME` | Fresh private run directory, `<run>/home` |
| `HERMES_HOME` | Fresh private run directory, `<run>/hermes` |
| `TERMINAL_HOME_MODE` | Literal `profile`; create `<run>/hermes/home` before launch |
| `DEEPSEEK_API_KEY` | The owner's key (D40), set on the Hermes child only; never in a manifest, argv, `LaunchPlan.env_set`, log or evidence |
| `HTTPS_PROXY` | Existing process-scoped loopback forward proxy URL |
| `HERMES_CA_BUNDLE` | The per-launch proxy CA path (replaces `SSL_CERT_FILE`, see below) |
| `NO_PROXY` | The tracker's `127.0.0.1,localhost`; it does not cover `api.deepseek.com` |
| `PATH`, locale, temp | Fixed tracker-safe values needed by the harness; no general inheritance |

All provider base-URL variables remain absent, especially `DEEPSEEK_BASE_URL`
(read at `$SITE/hermes_cli/auth.py:6545-6548` and
`$SITE/hermes_cli/runtime_provider.py:1474-1484`) and `OPENAI_BASE_URL`.

Two rows changed on 2026-10-02:

- **CA variable.** `SSL_CERT_FILE` is refused by Goal Cup's capture profile
  (`crates/goalcup-protocol/src/manifest.rs:972-989`; it replaces a process's roots
  everywhere, including tool children). Hermes' provider clients check
  `HERMES_CA_BUNDLE` first, before `SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE` and
  `CURL_CA_BUNDLE` (`$SITE/agent/ssl_verify.py:30-58`). They build
  `ssl.create_default_context(cafile=...)`, so the named file *replaces* the roots
  for those clients only. Goal Cup's measured CA variables are documented as
  extending roots (`manifest.rs:895-898`). D41 admits `HERMES_CA_BUNDLE` for the
  `hermes` family only, as a root-replacing variable. `SSL_CERT_FILE`,
  `REQUESTS_CA_BUNDLE` and `CURL_CA_BUNDLE` stay refused.

  The effect is wider than the provider client. `model_metadata`'s `requests`
  calls also take `HERMES_CA_BUNDLE` first (`$SITE/agent/model_metadata.py:27-45`;
  call sites `:924,1003,1050,1081,1877,1948`), so they fail TLS on any host the
  proxy does not intercept. That fails closed, but a context-length or probe
  fallback may behave differently from an unproxied run. Only the bare
  `requests.get` of models.dev keeps certifi and tunnels
  (`$SITE/agent/models_dev.py:290-293`).

  Start-up also fails closed. `verify_ca_bundle` raises if a named bundle is
  missing or loads no certificates (`$SITE/agent/ssl_guard.py:61-84`), and agent
  init calls it before building the client (`$SITE/agent/agent_init.py:1155-1159`);
  `ssl_verify.py:56-62` alone would silently fall back to certifi.
  `HERMES_SKIP_SSL_GUARD` must stay absent, and the capture env deny list refuses
  every `HERMES_` name (seam design S1).
- **Redaction switch.** `HERMES_REDACT_SECRETS` defaults to on when unset
  (`$SITE/agent/redact.py:69`), and the capture env deny list refuses any name
  containing `SECRET` (`manifest.rs:1113-1135`). It is left unset. The private config
  must not set `security.redact_secrets: false` (`$SITE/hermes_cli/main.py:656-690`).
No app-wide config or trust store is changed. `--safe-mode` and
`--ignore-user-config` must not be added: both bypass the private config file
that enables the supported JSON snapshot.

`--ignore-rules` is intentionally omitted. The parser describes it as skipping
auto-injection of `AGENTS.md`, `SOUL.md`, `.cursorrules`, memory, and preloaded
skills (`$SITE/hermes_cli/_parser.py:220-239`). The CLI implements it by setting
`self.ignore_rules` (`$SITE/cli.py:3973-3977`); agent setup passes that value as
both `skip_context_files` and `skip_memory`
(`$SITE/hermes_cli/cli_agent_setup_mixin.py:353-395`). The system-prompt builder
then skips SOUL and project context injection, while agent initialization skips
built-in and external memory loading (`$SITE/agent/system_prompt.py:188-194,462-478`;
`$SITE/agent/agent_init.py:1430-1459`). It is not a filesystem or
process-isolation control, and it does not itself change the tool permission
mode; its risk is removing instruction files that may state participant rules
or permissions. In a fresh private external task workspace and fresh
private `HERMES_HOME`, isolation comes from the paths and environment above, so
the flag is unnecessary and could suppress legitimate participant or project
instructions. A measurement profile that intentionally excludes those inputs
would need to declare and review that semantic change; the default adapter must
not apply it silently.

`HERMES_HOME` is resolved directly from the process environment
(`$SITE/hermes_constants.py:107-132`). `TERMINAL_HOME_MODE=profile` selects
`$HERMES_HOME/home` for tool children (`$SITE/hermes_constants.py:789-832`).
Hermes still creates private state and logs; that is contained inside the run
directory.

## Session identity carrier

This candidate has a supported process-tied carrier.

1. A fresh non-resumed CLI run generates
   `YYYYMMDD_HHMMSS_<six-lowercase-hex>` at `$SITE/cli.py:4095-4102`.
2. The CLI passes that exact ID into `AIAgent` at
   `$SITE/hermes_cli/cli_agent_setup_mixin.py:353-383`; the agent retains a
   supplied ID and uses `$HERMES_HOME/sessions` at
   `$SITE/agent/agent_init.py:1272-1307`.
3. The quiet path runs the turn, synchronizes a compression-created continuation
   ID if needed, prints the final response to stdout, then prints the CLI-owned
   line `session_id: <id>` to stderr at `$SITE/cli.py:15978-16025`.
4. Normal turn finalization calls `_persist_session` before returning
   (`$SITE/agent/turn_finalizer.py:313-325`). The stderr marker therefore follows
   the task's final persistence attempt. It precedes process cleanup and is a
   final-turn identity carrier, not an early session-start event.

The marker is process output, not model-generated final text. The adapter must
capture stdout and stderr separately, require exit success, require exactly one
distinct marker matching the fresh-ID grammar, and bind only that ID. It must
never select the newest file, infer identity from cwd, or scan unrelated
sessions. Missing, malformed, or conflicting markers are an identity gap.

## Native snapshot activation and collection

The only private config required for native output is:

```yaml
sessions:
  write_json_snapshots: true
```

It lives at `$HERMES_HOME/config.yaml`. The setting defaults to false at
`$SITE/hermes_cli/config.py:3200-3208`; `AIAgent` reads it during setup at
`$SITE/agent/agent_init.py:1294-1307`.

The DeepSeek route adds private-config keys and a private auth-store seed. None of
them names a provider, host, key or base URL. The full `config.yaml` is:

```yaml
sessions:
  write_json_snapshots: true
agent:
  reasoning_effort: "none"   # D41; quoted so YAML never reads it as a boolean
auxiliary:
  title_generation:
    enabled: false           # default true (config.py:1688-1697)
compression:
  enabled: false             # default true (config.py:1464-1465); keeps one session ID per run
```

Compression would rotate the session ID mid-run, and the stderr marker would then
name the continuation (`$SITE/cli.py:15990-15995`). Any auxiliary call that still
happens uses the profile's `default_aux_model`, `deepseek-chat`
(`$SITE/plugins/model-providers/deepseek/__init__.py:97`). It is recorded as a
second model value, not hidden.

**`auth.json` seed (D41, with a correction).** Without a seed, Hermes' credential
pool copies the environment key into a pool entry and writes
`$HERMES_HOME/auth.json` (`$SITE/agent/credential_pool.py:2301-2363,2400-2420,2576-2600`).
The disk boundary strips the raw value but keeps `secret_fingerprint`, the first
16 hex digits of the key's SHA-256 (`$SITE/agent/credential_persistence.py:123-170`).

`_load_auth_store` keeps a file only if it holds a `providers` or `credential_pool`
dict, and otherwise substitutes `{"version":1,"providers":{}}`
(`$SITE/hermes_cli/auth.py:1109-1150`). So the minimal store D41 names behaves
exactly like no file at all: the env source is still seeded and the fingerprint is
still written. The seed must also carry the suppression that the pool's env loop
checks (`credential_pool.py:2408`; `auth.py:1512-1519`):

```json
{"version": 1, "providers": {}, "suppressed_sources": {"deepseek": ["env:DEEPSEEK_API_KEY"]}}
```

The file is written with mode 0600. Key resolution still reads the environment
variable directly (`auth.py:586-591`). The D41 proof is unchanged: after an offline
synthetic-key run, the file is byte-identical to the seed and has no
`credential_pool` key. That test fails with the literal two-key seed, which is how
the correction is enforced.

**Key-fragment residue (D41).** On any non-retryable 4xx, Hermes writes
`$HERMES_HOME/sessions/request_dump_<sid>_<ts>.json`
(`$SITE/agent/conversation_loop.py:3936-3939`; `$SITE/agent/agent_runtime_helpers.py:1595-1670`).
Its header is `Bearer key[:8]...key[-4:]` (`$SITE/run_agent.py:2336-2345`), and
Hermes' redactor does not mask the JSON form. A 401 body may also echo a masked key.
Before any run artifact is retained, the tracker scans with the in-memory key for
its forms. A hit fails the row, and the run directory stays local and uncommitted.
The exact pattern `session_{id}.json` never selects a `request_dump_*` file.

For the emitted session ID, the exact native candidate is:

```text
$HERMES_HOME/sessions/session_<session-id>.json
```

`$SITE/run_agent.py:2694-2781` derives that path from the current session ID and
rewrites the full snapshot at every persistence point. The object contains
`session_id`, `model`, `base_url`, `platform`, start/update times, normalized
system prompt, tools, message count, and messages. Writes use temp-file,
`fsync`, and replace through `atomic_json_write` (`$SITE/utils.py:139-200`).

This is a supported native JSON output **candidate**, not a qualified full-native
claim. It is a normalized and secret-redacted final-state snapshot. It is not a
realtime raw event stream or a lossless history of every rewrite, and it omits
SQLite-only detail such as per-message timestamps and token counts. `state.db`
remains canonical and its installed schema constant is 22
(`$SITE/hermes_state.py:153-155`). The late stderr identity means this source can
prove final normalized snapshot evidence only unless Hermes gains an earlier
process-owned carrier.

Goal Cup already has the bounded collector this source needs. Reuse
`NativeCapture` with a `NativeSourceSpec` rooted at the private
`$HERMES_HOME/sessions`, pattern `session_{id}.json`, form
`NativeStoreForm::Snapshot`, no parser hint until verifier qualification, and
the exact validated stderr marker as `scope_id`
(`crates/goalcup-daemon/src/native_capture.rs:209-302`). Snapshot mode reads the
whole observed generation as raw bytes, ends it complete only at its observed
length, and interrupts it with `SourceChanged` when a rewrite changes the source
during collection (`native_capture.rs:1420-1521`). Its existing directory,
stream, queue, and finish limits apply; the runtime uses a 256 KiB poll budget,
100 ms polling, and a bounded finish of at most 64 MiB within the remaining
capture finish budget (`NATIVE_POLL_BYTES` in `runtime/capture/native_scope.rs`,
`NATIVE_FINISH_BYTES` in `runtime/capture/owner.rs`, the drain share in
`runtime/capture/finish.rs`). There is
no basis for a separate post-exit reader or invented 16 MiB/two-second limits.

The genuine missing seams are integration points, not a new collector:

- `scoped_native_source` currently maps only `codex` and `claude-code`, both as
  append-only sources (`runtime/capture/native_scope.rs`, `scoped_native_source`
  and `shipped_source`); Hermes needs the snapshot
  source mapping above;
- launch scope is currently supplied before `record_capture_launch`; Codex gets
  an early stdout `thread.started` ID and Claude uses its preassigned ID, while
  other families get no scope (`crates/goalcup-cli/src/commands/start.rs`, `run_harness`, which
  reads the `CaptureStreams` choice in `launch.rs`);
- Hermes emits its own ID on stderr only after the turn's final persistence, so
  it needs a bounded stderr tee/parser plus an explicitly reviewed late-scope
  activation or delayed launch-binding seam. Current daemon code creates the
  required native capture only when `CaptureLaunchRequest.native_scope` is
  present, otherwise recording a source gap (`runtime/capture/mod.rs`,
  `record_capture_launch`);
- verifier registration for the normalized JSON format remains a separate
  qualification gate. Raw-byte capture can use `hint: None` until then.

Because a source-supported own-session JSON exists, an unsupported SQLite
adapter is not currently justified. The adapter must never open `state.db` as a
fallback or guess another session file. If runtime proves the JSON missing,
stale, or insufficient, the candidate becomes **BLOCKED unsupported SQLite**
until the separate bounded schema-22/WAL snapshot architecture gate is reviewed.

## Provider, credential, child-process, and transport mapping

The provider is decided by D40 (2026-10-02): Hermes' own `deepseek` provider, with
the owner's DeepSeek API key in `DEEPSEEK_API_KEY`, passed to the child only. No
base-URL variable or config `base_url` is set, and the `openai-codex` OAuth login is
not used. The earlier `openai-api` row is withdrawn.

| Item | Source-backed value | Evidence |
|---|---|---|
| Provider ID | `deepseek` (registry key and `id`); alias `deepseek-chat` via the plugin profile | `$SITE/hermes_cli/auth.py:344-351`; `$SITE/plugins/model-providers/deepseek/__init__.py:85-98` |
| Key variable | `DEEPSEEK_API_KEY`, the only entry in `api_key_env_vars` | `auth.py:349`; plugin `env_vars` at `__init__.py:88` |
| Host / path | `https://api.deepseek.com/v1` hardcoded as `inference_base_url` and profile `base_url`; requests go to `POST /v1/chat/completions` | `auth.py:348`; `__init__.py:96`; `auth.py:6545-6584` falls back to it when `DEEPSEEK_BASE_URL` is absent |
| API mode | `chat_completions` (OpenAI-compatible Chat Completions, not Responses); transport overlay `openai_chat` | `$SITE/hermes_cli/runtime_provider.py:2068-2098` (default `chat_completions`; `_detect_api_mode_for_url` returns none for this host, `:101-139`); `$SITE/hermes_cli/providers.py:133-136` |
| Streaming | `stream: true` with `stream_options.include_usage`; SSE response | `$SITE/agent/conversation_loop.py:1407-1452`; `$SITE/agent/chat_completion_helpers.py:2639-2667` |
| HTTP version | HTTP/1.1 (`httpx` HTTP/2 off by default) | `$SITE/httpx/_client.py:639-650`; `$SITE/agent/process_bootstrap.py:145-199` |
| Known model IDs | `deepseek-v4-pro`, `deepseek-v4-flash`, `deepseek-chat`, `deepseek-reasoner` (catalog); profile fallbacks `deepseek-chat`, `deepseek-reasoner`; auxiliary default `deepseek-chat`; `/model` default identity `deepseek-chat` | `$SITE/hermes_cli/models.py:387-392`; `__init__.py:92-97`; `$SITE/hermes_cli/model_switch.py:296` |

**Key precedence when `--provider deepseek` is explicit.** `resolve_provider`
returns a registry ID unchanged and never runs the `auto` heuristics
(`auth.py:1801-1808`). `_resolve_explicit_runtime` applies only to CLI-supplied
explicit key or base URL values, which this launch never passes
(`runtime_provider.py:1367-1378`). Resolution then tries Hermes' credential pool,
which is seeded from the same variable (`runtime_provider.py:1702-1789`), then the
API-key path (`runtime_provider.py:2041-2092`). For `DEEPSEEK_API_KEY` that path
reads first `$HERMES_HOME/.env`, then the process environment, then the pool
(`auth.py:568-605`; `$SITE/hermes_cli/config.py:8192-8215`). In a fresh private
`HERMES_HOME` with no `.env` and no `auth.json` credential rows, only the child's
`DEEPSEEK_API_KEY` can be selected. With a fresh `HERMES_HOME` outside `~/.hermes`,
there is no global-root auth fallback (`auth.py:916-944`;
`$SITE/hermes_constants.py:154-184`). A missing key raises `missing_api_key` at
resolution (`runtime_provider.py:2047-2058`). It does not fall back to another
provider unless config defines `fallback_providers`, which defaults to empty
(`$SITE/hermes_cli/cli_agent_setup_mixin.py:41-80`; `config.py:1001`). The private
config must not define a fallback chain, `model.base_url`, `model.api_mode`, `providers:` blocks or
`custom_providers`.

### Model ID

**Ruling (D41): `deepseek-v4-flash`, owner-confirmed, with `agent.reasoning_effort:
"none"`.** It supersedes the earlier `deepseek-flash` pin. An earlier version of this
record said the owner had asked not to use the alias. That is reversed: the owner
explicitly confirmed `deepseek-v4-flash`.

The row records three `model` values verbatim as evidence of what served it: the
request body's `model`, the response chunks' `model` (DeepSeek returns the dated
served ID; `$SITE/hermes_cli/model_normalize.py:141-144`), and the snapshot's
`model`, which is Hermes' own value. The row claims only what the response field
says. Reconsider when DeepSeek retires the alias or a newer Hermes knows
`deepseek-flash`.

Why not `deepseek-flash` (DeepSeek V4.1 Flash, per the root's reading of DeepSeek's
2026-09-10 release note, not verified here): **Hermes 0.19.0 cannot send that string
through its `deepseek` provider.** Every DeepSeek model name goes
through `_normalize_for_deepseek`
(`$SITE/hermes_cli/model_normalize.py:446-451,148-180`). It passes through only the
four canonical IDs or `^deepseek-v\d+([-.].+)?$`. Any other name containing
`reasoner`, `r1`, `think`, `reasoning` or `cot` becomes `deepseek-reasoner`.
Everything else becomes `deepseek-chat`. `deepseek-flash` matches neither rule, so
it is rewritten to `deepseek-chat` twice: by the CLI (`$SITE/cli.py:5336-5357`) and
again at agent construction (`$SITE/agent/agent_init.py:497-506`). There is no
validation error; the CLI prints a warning only. The only supported request-override
path belongs to named custom providers, which need a `base_url`
(`runtime_provider.py:917-1052`), so it is excluded.

What each launchable choice sends, by source:

| `--model` value | Sent to DeepSeek | Thinking / `reasoning_effort` sent by Hermes |
|---|---|---|
| `deepseek-flash` | `deepseek-chat` (silent rewrite) | none: `deepseek-chat` is treated as non-thinking (`__init__.py:28-58`) |
| `deepseek-chat` | `deepseek-chat` | none. Hermes' own note says DeepSeek maps it server-side to `deepseek-v4-flash` non-thinking (`$SITE/agent/model_metadata.py:262-272`); what it maps to after 2026-09-10 is not known here |
| `deepseek-v4-flash` | `deepseek-v4-flash` | thinking-capable by name (`__init__.py:38-41`): `extra_body.thinking={"type":"enabled"}` by default. Top-level `reasoning_effort` is sent only if `agent.reasoning_effort` is set: `low`/`medium`/`high` pass through, `xhigh`/`max`/`ultra` become `max` (`__init__.py:60-82`). `agent.reasoning_effort: none` sends `thinking={"type":"disabled"}` and no effort (`$SITE/hermes_constants.py:840-864,992-1022`). Note `medium` passes through although the unverified DeepSeek claim lists only low/high/max |

With the chosen row (`deepseek-v4-flash`, `reasoning_effort: "none"`), Hermes sends
`extra_body.thinking={"type":"disabled"}` and no `reasoning_effort`. That also avoids
the reasoning-content echo 400 described at `__init__.py:1-17`. The other options
were rejected under D41:
- `deepseek-chat`: a legacy alias whose server-side mapping is unverified.
- Waiting for a newer Hermes: it would mean re-pinning every identity here.
- A slash-prefix trick: it relies on undocumented prefix tolerance and bypasses the
  thinking parameter.

The `--model` argument alone is not proof of what served the row.

### Egress expected beside the primary call

- `GET https://models.dev/api.json` through bare `requests` when the private home
  has no fresh cache (`$SITE/agent/models_dev.py:240-320`). It is reached from the
  context-length lookup at `$SITE/agent/model_metadata.py:2511-2526`. models.dev is
  not an intercepted host, so the request tunnels opaquely and keeps certifi roots.
  Other `model_metadata` `requests` calls honour `HERMES_CA_BUNDLE` and fail closed
  on non-intercepted hosts (see the environment notes).
- Session title generation and compression are auxiliary calls to the same
  provider, and the private config disables both (`config.py:1464-1465,1688-1697`;
  `$SITE/agent/title_generator.py:57-71`).
- `api.deepseek.com` is a known provider host (`model_metadata.py:481`), so no
  `/v1/models` probe is expected (`model_metadata.py:2349-2358`).

### Owner key handling (run time only)

The owner keeps the key as `DEEPSEEK_API_KEY` in
a private env file outside this repository (mode 0600, no
DeepSeek base URL there, per the owner). That file was not opened for this record.
At run time only that one variable is loaded into the launching `goalcup` process,
without echo, and handed to the Hermes child only. It is never written into the
goal-cup tree, argv, TOML or evidence. The loader and the child-only handoff are
specified in
`.superpowers/sdd/2026-10-01-m2-phase-2-hermes-reference/seams-design-v2.md`,
which supersedes `seams-design.md`. The key-fragment scan uses that in-memory value
and stores nothing derived from it.

**Isolation of every invocation (D41).** Every Hermes invocation runs with a private
`HOME` and `HERMES_HOME`, version probes included. A V1 version probe would
otherwise run Hermes against the real home: `probe_env` keeps the real `HOME`
(`crates/goalcup-cli/src/preflight/probe.rs:168-173,352-359`). At import, Hermes
reads `~/.hermes/active_profile` and `.env` and starts file logging under
`~/.hermes/logs` (`$SITE/hermes_cli/main.py:600-615,649-705`). The `hermes-cli`
connector is therefore capture-only: every V1 path refuses it before spawning
anything, and the capture path never probes (its version is the pinned profile
value).

Hermes derives the terminal/execute-code provider blocklist from the provider
registry: every row's `api_key_env_vars` and `base_url_env_var`, so
`DEEPSEEK_API_KEY` and `DEEPSEEK_BASE_URL` are included (`$SITE/tools/environments/local.py:224-236`).
It also adds `OPENAI_*`/`ANTHROPIC_*` names explicitly (`local.py:250-332`). The default terminal child path
sanitizes those variables and removes active virtualenv markers at
`$SITE/tools/environments/local.py:451-493`. The general non-terminal helper
strips provider/tool credentials unless a caller explicitly asks for credential
inheritance (`$SITE/tools/environments/local.py:539-605`); no literal
`inherit_credentials=True` call site was found in the installed Python tree.
The private config must not opt provider keys into passthrough. Runtime validation
must still prove a synthetic marker value is absent from a child tool environment
before a real key is used.

Auxiliary model clients can inherit the live main runtime in process, including
the main provider/model/base URL/key, through the scoped runtime mechanisms at
`$SITE/agent/auxiliary_client.py:2201-2315` and `:2325-2444`. Their OpenAI client
factory uses the same process proxy helper and CA resolver at
`$SITE/agent/auxiliary_client.py:134-214`. The private config must contain no
auxiliary-provider override, so an auxiliary task cannot name a second host or
credential. An authenticated qualification must inspect all observed proxy
requests, not assume the primary call is the only egress.

The process transport seam is source-backed:

- `$SITE/agent/process_bootstrap.py:112-142` selects `HTTPS_PROXY`, then
  `HTTP_PROXY`, `ALL_PROXY`, and lowercase variants, and applies `NO_PROXY`;
- `$SITE/agent/process_bootstrap.py:145-199` constructs the explicit `httpx`
  client with the selected proxy and supplied TLS verification context;
- `$SITE/agent/ssl_verify.py:22-63` takes a process CA file from
  `HERMES_CA_BUNDLE` first, then `SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE`,
  `CURL_CA_BUNDLE`, without changing global trust. The file replaces the roots for
  that client (see the environment table);
- installed `httpx` defaults to HTTP/1.1 enabled and HTTP/2 disabled at
  `$SITE/httpx/_client.py:639-650`.

So by source the `deepseek` route is HTTP/1.1 SSE Chat Completions to
`api.deepseek.com`, and it fits the existing loopback proxy with `HERMES_CA_BUNDLE`
(admitted for the `hermes` family by D41). It is not
transport-qualified until a local synthetic fixture observes this client through
the existing proxy/CA. That fixture exercises the client seam separately and must
not override Hermes' model base URL.

On the Goal Cup side, the proxy intercepts only the profile's `[[capture.hosts]]`
and tunnels every other host opaquely
(`crates/goalcup-cli/src/proxy/connect.rs:5-15,432-456`). The profile would
approve `api.deepseek.com` with exact path `/v1/chat/completions`. The verifier's
only OpenAI decoder is Responses-only and selects streams by the exact hint
`openai-responses` (`crates/goalcup-verifier/src/capture_analysis/openai.rs:1-14,96-100,155`).
Chat Completions SSE (`choices[].delta`) is a different format. A DeepSeek stream
therefore needs its own hint (proposed `openai-chat-completions`). It stays
capturable, and is reported as unsupported semantics until a decoder is
registered (`crates/goalcup-protocol/src/manifest.rs:965-968`).

*Update 2026-10-05: the `openai-chat-completions` decoder is registered
(`capture_analysis/chat.rs`). The paragraph above is the record of why it was needed.*

Request headers are never recorded. The relay keeps bodies plus allowlisted
metadata, and no header value or credential is retained
(`crates/goalcup-cli/src/proxy/http.rs:40`; `proxy/metadata.rs:22-23`). The real
wire exposure is therefore a response body that echoes a masked key, such as a
401. The shared `openai_key` and `bearer` rules need a long contiguous token
(`crates/goalcup-daemon/src/redact.rs:102-106`), so they miss a masked fragment.
D41 adds masked-key forms to wire redaction (seam design S9), and the tracker's
key-fragment scan is the final check.

## Credential-free import readiness

One bounded check was run under an explicit lightweight inspection lease. It
used the pinned interpreter with `-I` and `env -i`, private empty
`HOME`/`HERMES_HOME`/cwd, `PYTHONNOUSERSITE=1`, bytecode disabled, no inherited
credential variables, and socket `connect`, `connect_ex`, and
`create_connection` blocked before importing Hermes modules. The installed
package-level `.env` paths were confirmed absent without reading any dotenv
content. A 12-second alarm bounded the import.

Observed result in 0.68 seconds:

```text
IMPORT_OK hermes-agent=0.19.0 python=3.13.13
IMPORT_SOURCE hermes_cli.main=$SITE/hermes_cli/main.py
IMPORT_SOURCE cli=$SITE/cli.py
IMPORT_SOURCE run_agent=$SITE/run_agent.py
```

Import created only private `state.db`, `logs/errors.log`, `logs/agent.log`, and
`SOUL.md`; the temporary sandbox was then removed. This proves dependency/import
availability for those three modules from the pinned tree. It does not prove CLI
startup, provider resolution, model access, native snapshot output, tool-child
sanitization, or transport.

## Qualification gates

| Gate | Evidence required | State |
|---|---|---|
| Exact owner metadata | Provider ID, model ID, and credential environment variable name only; never the value | **Resolved**: `deepseek` and `DEEPSEEK_API_KEY` (D40); `deepseek-v4-flash` with `reasoning_effort: "none"` (D41, owner-confirmed) |
| Capture-profile policy | `HERMES_CA_BUNDLE` for the `hermes` family only; `--query=<goal>` goal placement; per-run `HERMES_HOME`; capture-only connector; `HERMES_*` env names refused | CA ruled by D41; the schema is pending S1 and the capture-only slice review |
| Credential API review | Typed child-only handoff; no debug or serialization surface; `auth.json` byte-identical to the corrected seed after an offline synthetic-key run; key-fragment scan clean | Pending S4/S8 review and the synthetic tests |
| CLI/session runtime | Exact argv; separate stdout/stderr; successful exit; one process-tied session marker | Pending sole runtime lease |
| Native JSON runtime | Private config activation; exact own-session file; final session ID matches JSON; stable after child exit; within bounds | Pending sole runtime lease |
| Child secret isolation | Synthetic marker absent from every exercised child environment | Pending sole runtime lease |
| Transport | Local synthetic provider client observed through existing proxy/CA; HTTP/1.1; no base-URL override | Pending H-09/root serialized test lease |
| Authenticated reference | One bounded real request; the request, response and snapshot `model` values recorded; key-fragment scan clean before anything is retained; independent raw/native evidence review | Pending the seams and later authorization |

Overall readiness: **READY_FOR_SEAMS (source-only)**. By source, every launch input
is now concrete: executable, interpreter, import tree, argv, provider, model, key
variable, host, wire format, CA variable, private config and seed, the process-tied
identity carrier, and snapshot activation. One discrepancy is open: D41 names the
`auth.json` seed as `version` plus empty `providers`, and that seed is ineffective
without `suppressed_sources` (see "Native snapshot activation"). The byte-identical
test settles it. The seams are specified in `seams-design-v2.md`. Hermes support must
not be marked complete until every gate above passes on these identical pinned
inputs. Nothing in this record is a runtime claim.
