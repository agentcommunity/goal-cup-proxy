# Gemini CLI capture research: documentation/source preflight (v0.58.0)

Status: documentation and source reading only, 2026-09-10. Gemini CLI was not
installed or run, no model was called, no authentication was accessed, and no
configuration was written. Every A1-A11 result remains unmeasured. This is an
M2/V1.1 expansion candidate, outside the narrow V1 foundation in `docs/DECISIONS.md` D29.

Evidence labels in this file are deliberate:

- **Documented** means the tagged vendor documentation says it.
- **Source-derived** means the tagged implementation supports the statement.
- **Unmeasured** means the future macOS spike still has to observe it.

## Release inspected

The exact stable release inspected is **Gemini CLI v0.58.0**, published
2026-09-01. The release points to commit
`ac9431c9e2290d68af31a77614ff2fddb2391ca3` ([release], [commit],
[package version]). Gemini's release policy calls the weekly `latest` channel
the stable release ([release policy]). Source references below are pinned to
that tag; none is a claim about a later build.

[release]: https://github.com/google-gemini/gemini-cli/releases/tag/v0.58.0
[commit]: https://github.com/google-gemini/gemini-cli/commit/ac9431c9e2290d68af31a77614ff2fddb2391ca3
[package version]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/package.json#L1-L4
[release policy]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/docs/releases.md#L217-L258

## Evidence surfaces

### Session store: useful, mutable operation log

**Documented.** Gemini says it automatically saves prompts, model responses,
tool inputs and outputs, token statistics, and thought summaries under
`~/.gemini/tmp/<project_hash>/chats/`; sessions are project-scoped and retained
for 30 days by default ([session management L7-L22], [retention L146-L186]). A
session can be resumed by full UUID with `--resume <uuid>` ([resume L24-L53]).

**Source-derived.** Under a redirected home this path becomes
`$GEMINI_CLI_HOME/.gemini/tmp/<project_hash>/chats/`. A main session file is
`session-<minute>-<session-id-prefix>.jsonl`; subagent files are nested under the
complete parent session ID ([recording initialization L418-L543]). Records
include `id`, `timestamp`, content and optional display content; Gemini records
also include tool calls, thoughts, tokens, and model, but user records have no
origin field ([recording types L42-L104]).

The JSONL must not be treated as one immutable final event per line. Repeated
message IDs replace the folded in-memory value, `$rewindTo` removes a suffix,
and `$set` replaces metadata or messages. The loader ignores individually
malformed lines ([loader L168-L345]); resume may migrate a legacy JSON file or
atomically reconstruct an unreadable JSONL while preserving the old bytes under
an `.unreadable-*` name ([recording initialization L418-L475],
[rewrite L575-L631]). Normal writes append one JSON line ([append L559-L565]). A
future tailer must capture raw append bytes and detect inode
replacement/truncation; the verifier must fold the operation log separately.
Gemini's permissive loader does not relax Goal Cup's partial-tail refusal.

The vendor phrase “complete conversation history” does not establish a complete
model request. The persisted message type has no system-instruction or tool-
definition field, and history masking/compression can later append a `$set` with
changed messages or tool results ([history sync L953-L1029]). Full request
coverage remains unmeasured.

[session management L7-L22]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/docs/cli/session-management.md#L7-L22
[retention L146-L186]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/docs/cli/session-management.md#L146-L186
[resume L24-L53]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/docs/cli/session-management.md#L24-L53
[recording initialization L418-L543]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/packages/core/src/services/chatRecordingService.ts#L418-L543
[recording types L42-L104]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/packages/core/src/services/chatRecordingTypes.ts#L42-L104
[loader L168-L345]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/packages/core/src/services/chatRecordingService.ts#L168-L345
[rewrite L575-L631]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/packages/core/src/services/chatRecordingService.ts#L575-L631
[append L559-L565]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/packages/core/src/services/chatRecordingService.ts#L559-L565
[history sync L953-L1029]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/packages/core/src/services/chatRecordingService.ts#L953-L1029

### Project hooks: a candidate independent leg, with coverage gaps

**Documented.** Command hooks receive JSON on stdin and return JSON on stdout;
exit 2 blocks the applicable action ([hook mechanics L6-L17]). Common input
contains `session_id`, `transcript_path`, `cwd`, event name, and timestamp
([hook base L46-L58]). `BeforeAgent` carries the original submitted prompt and
can deny it before it reaches history ([BeforeAgent L143-L161]). Tool hooks
expose call arguments and results; `BeforeModel`/`AfterModel` expose a stable
request/response form; `AfterModel` fires once per streamed chunk
([tool hooks L77-L139], [model hooks L185-L239]). `SessionStart.source`
distinguishes `startup`, `resume`, and `clear`; `SessionEnd` is best effort and
is not awaited ([lifecycle hooks L243-L270]).

**Source-derived.** Project hooks are admitted only for a trusted workspace. On
the first trusted-workspace load, the registry warns that unrecognized project
hooks will execute, immediately adds their keys and attempts to persist them to
`$GEMINI_CLI_HOME/.gemini/trusted_hooks.json`, then registers the configuration;
hook commands execute later when their events are dispatched
([hook trust L139-L188], [trusted hook storage L99-L120]). This is a warning and
automatic-recording path, not a consent gate. `hooksConfig.enabled` can remove
the whole hook system, and the configured disabled-name list controls
individual entries ([hook setup L1088-L1115], [disabled hooks L256-L282]). A
hard arming signal and continuous hash watch are therefore prerequisites, not
assumptions.

The model hook is not a raw request tap. Its translator intentionally drops
images, function calls, function responses and other non-text parts, and exposes
only a small generation-config subset ([hook translator L153-L220]). A
successful `BeforeModel` post cannot by itself prove all model-visible bytes.

[hook mechanics L6-L17]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/docs/hooks/reference.md#L6-L17
[hook base L46-L58]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/docs/hooks/reference.md#L46-L58
[BeforeAgent L143-L161]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/docs/hooks/reference.md#L143-L161
[tool hooks L77-L139]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/docs/hooks/reference.md#L77-L139
[model hooks L185-L239]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/docs/hooks/reference.md#L185-L239
[lifecycle hooks L243-L270]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/docs/hooks/reference.md#L243-L270
[hook trust L139-L188]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/packages/core/src/hooks/hookRegistry.ts#L139-L188
[trusted hook storage L99-L120]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/packages/core/src/hooks/trustedHooks.ts#L99-L120
[hook setup L1088-L1115]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/packages/cli/src/config/config.ts#L1088-L1115
[disabled hooks L256-L282]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/packages/core/src/hooks/hookRegistry.ts#L256-L282
[hook translator L153-L220]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/packages/core/src/hooks/hookTranslator.ts#L153-L220

### Local telemetry: broader request view, bounded retained prefix

**Documented.** Local telemetry can be enabled with detailed traces and prompt
logging and written to `GEMINI_TELEMETRY_OUTFILE`, which overrides the OTLP
endpoint ([telemetry configuration L33-L49]).

**Source-derived.** When an outfile is set, the same append-mode file receives
span, log, and metric JSON ([file exporters L21-L105],
[SDK selection L281-L345]). Each LLM span starts from the request contents and
separately adds the system instruction and tool definitions
([logging generator L355-L396], [streaming path L447-L524]). For every
stringified input, output, and detailed attribute, the truncation path retains
at most **10,000 UTF-16 code units**, preserving whole surrogate-plus-combining
segments, and then appends the truncation marker outside that limit
([trace truncation L48-L77], [truncate helper L106-L144],
[trace finalization L158-L177]). The resulting attribute can therefore exceed
10,000 UTF-16 code units and is not a byte-bounded record.

This closes no completeness claim. The transcript omits system/tool definitions,
the hook drops non-text parts, and telemetry truncates large attributes. A
future measurement must compare all three surfaces against controlled sentinels;
until then there is no source-proven lossless model-input leg and no justified
`full` tier.

[telemetry configuration L33-L49]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/docs/cli/telemetry.md#L33-L49
[file exporters L21-L105]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/packages/core/src/telemetry/file-exporters.ts#L21-L105
[SDK selection L281-L345]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/packages/core/src/telemetry/sdk.ts#L281-L345
[logging generator L355-L396]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/packages/core/src/core/loggingContentGenerator.ts#L355-L396
[streaming path L447-L524]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/packages/core/src/core/loggingContentGenerator.ts#L447-L524
[trace truncation L48-L77]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/packages/core/src/telemetry/trace.ts#L48-L77
[truncate helper L106-L144]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/packages/core/src/utils/textUtils.ts#L106-L144
[trace finalization L158-L177]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/packages/core/src/telemetry/trace.ts#L158-L177

## Classification and lifecycle questions

**Human versus synthetic.** `MessageRecord` distinguishes `user` from `gemini`
but has no human-origin field. Resume reconstruction also synthesizes model
history `role: "user"` function responses, while the recorder can create new
synthetic turns during history sync ([session reconstruction L110-L227],
[history sync L979-L1017]). The future human rule must pair `BeforeAgent.prompt`
with leg-1 candidates and measure compaction, hook context, tool responses,
automatic retries and subagent notices before defining exclusions.

**Resume.** Source resolution reuses the stored session ID and file for an exact
`--resume <uuid>` ([resume resolution L203-L319]). A bare/latest resume with no
sessions warns and creates a new session, so the spike must resume by exact UUID
and test latest/no-session as a failure control. `SessionStart` reports `resume`;
a hard kill cannot be expected to produce the best-effort `SessionEnd`.

**Local agents.** Built-in agents run a separate `GeminiChat` with a fresh agent
ID while preserving the root parent session ID ([local executor L134-L145],
[subagent chat L1054-L1087]). The parent tool record receives the child's
`agentId`, and the child JSONL path is nested below the root session ID
([parent tool record L1682-L1715], [recording initialization L484-L518]). This
is a source-derived candidate parent link, still unmeasured end to end. Use one
built-in local agent in the spike; do not add a remote-agent dependency.

**Model identity.** Gemini messages can store `model`; `/model` applies to
subsequent interactions but does not override subagents
([model selection L1-L38]). Routing may silently use a fallback model for
utility calls without changing the configured model ([model routing L3-L27]).
A6 must compare the transcript, hook `llm_request.model`, telemetry request model
and the observed mid-run switch; a configured-model label alone is insufficient.

**Large outputs.** The default legacy shell/MCP display threshold is 40,000
characters ([configuration threshold]). With context management enabled, most
tools are distilled above the configured token threshold: the raw value is
written under the project temp directory, and the model receives a truncated
value that names that file ([distillation L39-L80], [distillation L116-L177],
[spill writer L729-L754]). Completed transcript tool calls record the response
parts after this truncation ([tool result L371-L399], [parent tool record
L1682-L1715]). A3 must preserve the spill bytes as leg 1, prove the transcript
contains exactly what the model saw, and cover masking/compression rewrites.

**TUI versus headless.** `-p` or a non-TTY enters headless mode, whose streaming
JSON is a separate event surface ([headless L1-L35]). `SessionStart` additional
context is inserted differently in interactive and non-interactive modes
([lifecycle hooks L245-L258]). The primary spike must drive the real TUI; a
separate headless run may compare behavior but cannot substitute for it.

[session reconstruction L110-L227]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/packages/core/src/utils/sessionUtils.ts#L110-L227
[resume resolution L203-L319]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/packages/cli/src/gemini.tsx#L203-L319
[local executor L134-L145]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/packages/core/src/agents/local-executor.ts#L134-L145
[subagent chat L1054-L1087]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/packages/core/src/agents/local-executor.ts#L1054-L1087
[parent tool record L1682-L1715]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/packages/core/src/core/geminiChat.ts#L1682-L1715
[model selection L1-L38]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/docs/cli/model.md#L1-L38
[model routing L3-L27]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/docs/cli/model-routing.md#L3-L27
[configuration threshold]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/docs/reference/configuration.md#L1828-L1833
[distillation L39-L80]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/packages/core/src/context/toolDistillationService.ts#L39-L80
[distillation L116-L177]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/packages/core/src/context/toolDistillationService.ts#L116-L177
[spill writer L729-L754]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/packages/core/src/utils/fileUtils.ts#L729-L754
[tool result L371-L399]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/packages/core/src/scheduler/tool-executor.ts#L371-L399
[headless L1-L35]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/docs/cli/headless.md#L1-L35

## Isolation and authentication

**Documented.** Configuration precedence is defaults, system defaults, user,
project, system overrides, environment, then CLI arguments. Project settings
live at `.gemini/settings.json`; both macOS system settings paths can be
overridden ([configuration layers L7-L61]). `GEMINI_CLI_HOME` relocates user
configuration and storage, and `GEMINI_CLI_TRUSTED_FOLDERS_PATH` relocates the
trust file ([configuration environment L2613-L2633]). The CLI searches project
ancestors and then `~/.env`, so the future run must begin from a controlled
directory tree and isolated home ([environment search L2587-L2600]).

Google AI Pro/Ultra subscription use is supported through “Sign in with Google”
and caches credentials locally ([Google sign-in L42-L62]). No proxy or model
base URL is needed or permitted for the spike.

**Source-derived.** OAuth uses a hybrid store with service
`gemini-cli-oauth`. Without an override, keychain initialization performs a
set/get/delete functional probe against the native store
([OAuth storage L16-L50], [keychain L99-L119], [keychain probe L175-L201]). The
undocumented source switch `GEMINI_FORCE_FILE_STORAGE=true` skips native
keychain loading and uses an encrypted mode-0600 file at
`$GEMINI_CLI_HOME/.gemini/gemini-credentials.json` ([file keychain L14-L26],
[file keychain L80-L116]).

The smallest acceptable future auth setup is therefore an interactive Google
sign-in performed before the measured session with both `GEMINI_CLI_HOME` and
`GEMINI_FORCE_FILE_STORAGE=true` already pinned to the isolated harness home.
The credential-file baseline records metadata only: existence, file type, mode,
size and modification time as available. It never reads, copies or hashes
credential bytes. Content hashes are reserved for non-secret hook and control
files. Native keychain behavior is a negative failure control, not an authorized
app-wide exception. Existing V1-surface exceptions do not extend to Gemini.

This is not yet proof of A10. Some tagged code still calls `os.homedir()`
directly ([direct home reads]), and the macOS system-policy directory is
separate from the overridable settings files ([system paths L133-L151]). A
future run must also pin `HOME` and the XDG roots, redirect the two system
settings files, source-audit remaining paths, and compare an explicit real-home
or system baseline before and after. Any unexplained real-home or native
keychain write requires an isolation decision; it cannot be waived from this
preflight.

[configuration layers L7-L61]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/docs/reference/configuration.md#L7-L61
[configuration environment L2613-L2633]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/docs/reference/configuration.md#L2613-L2633
[environment search L2587-L2600]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/docs/reference/configuration.md#L2587-L2600
[Google sign-in L42-L62]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/docs/get-started/authentication.mdx#L42-L62
[OAuth storage L16-L50]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/packages/core/src/code_assist/oauth-credential-storage.ts#L16-L50
[keychain L99-L119]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/packages/core/src/services/keychainService.ts#L99-L119
[keychain probe L175-L201]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/packages/core/src/services/keychainService.ts#L175-L201
[file keychain L14-L26]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/packages/core/src/services/fileKeychain.ts#L14-L26
[file keychain L80-L116]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/packages/core/src/services/fileKeychain.ts#L80-L116
[direct home reads]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/packages/core/src/policy/sandboxPolicyManager.ts#L96-L108
[system paths L133-L151]: https://github.com/google-gemini/gemini-cli/blob/v0.58.0/packages/core/src/config/storage.ts#L133-L151

## Smallest future spike

Use the existing `spikes/stub-receiver.py`, `spikes/stub-tailer.py`, snapshot and
checker patterns. A later implementation may add only Gemini-specific folder
settings, stdin hook forwarding, store folding/tailing and checks; this research
adds none of them.

The candidate process environment is exact apart from run-local absolute paths:

```text
GEMINI_CLI_HOME=$H
HOME=$H
XDG_CONFIG_HOME=$H/.config
XDG_DATA_HOME=$H/.local/share
XDG_STATE_HOME=$H/.local/state
XDG_CACHE_HOME=$H/.cache
GEMINI_FORCE_FILE_STORAGE=true
GEMINI_CLI_TRUST_WORKSPACE=true
GEMINI_CLI_TRUSTED_FOLDERS_PATH=$H/.gemini/trustedFolders.json
GEMINI_CLI_SYSTEM_SETTINGS_PATH=$CONTROL/system-settings.json
GEMINI_CLI_SYSTEM_DEFAULTS_PATH=$CONTROL/system-defaults.json
GEMINI_TELEMETRY_ENABLED=true
GEMINI_TELEMETRY_TARGET=local
GEMINI_TELEMETRY_TRACES_ENABLED=true
GEMINI_TELEMETRY_LOG_PROMPTS=true
GEMINI_TELEMETRY_OUTFILE=$RAW/telemetry.json
GEMINI_TELEMETRY_USE_COLLECTOR=false
GOALCUP_GEMINI_HOOK_SHIM=$CONTROL/gemini-hook-shim
```

Seed both system settings files as `{}`. The only project settings are
`hooksConfig.enabled=true`, `hooksConfig.disabled=[]`,
`general.sessionRetention.enabled=false`, and command-hook entries for these
exact events: `SessionStart`, `SessionEnd`, `BeforeAgent`, `AfterAgent`,
`BeforeTool`, `AfterTool`, `BeforeModel`, `BeforeToolSelection`, `AfterModel`,
and `PreCompress`. Each event has one sequential hook named
`goalcup-capture`, command `${GOALCUP_GEMINI_HOOK_SHIM}`, and a 5,000 ms
timeout. The future shim must forward unmodified stdin to the authenticated
local receiver and return event-correct JSON; it does not exist in this
preflight. The file contains no auth token, provider base URL, model setting, or
global path. Start with `--model gemini-2.5-pro` and switch to
`gemini-2.5-flash` only if both appear in the authenticated model list; otherwise
abort and record availability rather than silently substitute.

1. Install exactly v0.58.0 under ignored `harnesses/gemini-cli/`, verify the
   executable version and package digest, and create four disjoint paths: source,
   isolated home, tracked workspace, and ignored raw output. The output and home
   must not be inside or adjacent to the tracked workspace.
2. Before sign-in or launch, apply the exact environment above, including all
   XDG home roots, `GEMINI_FORCE_FILE_STORAGE=true`,
   `GEMINI_CLI_SYSTEM_SETTINGS_PATH`,
   `GEMINI_CLI_SYSTEM_DEFAULTS_PATH`, and
   `GEMINI_CLI_TRUSTED_FOLDERS_PATH` to those isolated paths. Set telemetry to
   local file output with enabled/traces/log-prompts true. Do not set a provider
   base URL or proxy.
3. Pre-authenticate “Sign in with Google” inside that isolated home. Record only
   credential-file metadata (existence, file type, mode, size and modification
   time as available); never read or hash credential bytes. Put the hook commands
   solely in the tracked workspace's `.gemini/settings.json`, trust only that
   workspace, hash the non-secret hook and control files, then require observed
   authenticated `SessionStart(startup)` and the expected transcript path before
   accepting the goal.
4. Drive the real TUI through the canonical goal, two `keep going` turns, one
   controlled write tool, one large-output tool, one built-in local subagent, a
   model switch, an over-length denial and a disallowed message. Run a parallel
   Gemini TUI from the same isolated home in a different workspace for A9.
5. Interrupt during a turn, kill the native process tree, preserve the partial
   tail, and resume the exact UUID. Require `SessionStart(resume)`, the same
   session ID/store relationship, a fresh completed turn after resume, and a
   separate latest/no-session failure control.
6. Capture raw transcript operations, spill files, hook posts, telemetry file,
   process output and before/after isolation snapshots only under ignored raw
   output. Partition by exact workspace and bound session before persistence;
   foreign records are discarded before hashing. Keep secrets in private raw
   evidence, redact only copies proposed as fixtures, and never rewrite
   receipt-bound bytes.
7. Stop cleanly, fold the final operation log without changing raw bytes, compare
   it with the tail capture and all auxiliary surfaces, then score A1-A11. Run a
   separate headless comparison only after the TUI result.

The tier remains **pending measurement**. `full` is available only if the real
spike proves two independent, partitionable legs and reconstructs every required
model-visible/tool-result byte despite telemetry retaining at most 10,000 UTF-16
code units before appending its truncation marker.
Incomplete but usable evidence may justify `elevated_review` with named findings.
Unpartitionable capture, app-wide credential/config mutation, missing arming, or
no defensible human/model-visible reconstruction requires refusal.
