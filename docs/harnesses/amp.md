# Amp harness facts

Connector id: `amp-cli` (proposed; no connector exists) · Facts for version: `0.0.1791676897-g77c125` · Written by: Goal Cup team, draft for the Amp team · Last checked: 2026-10-15
Public disclosure: yes. The facts come from Amp's public documentation, string literals and help text in the shipped npm package, a key-free, network-less probe on one Linux host (2026-10-12), and one live, logged-in probe on macOS arm64 (2026-10-15). The live probe used the Goal Cup maintainer's own Amp account, a private `HOME` and `TMPDIR`, a workspace outside any Git tree, and a capturing loopback proxy. No Amp source code was read. Raw probe output is not published, because it holds tokens.
Status: facts only (draft for the Amp team to confirm)

How to read this sheet. "(live)" marks a fact measured in the logged-in macOS probe: two execute-mode runs, a one-word decoy goal on the default mode and a file-plus-shell goal on `--mode low`, both through the proxy with `AMP_SKIP_UPDATE_CHECK=1`. A fact with no marker was measured by the key-free Linux probe (x86_64, glibc, no route out). Every other fact is marked "(please confirm)" and is one of: Amp public docs fetched 2026-10-11, a string or help text read from the package without running that path, or "unknown". We would rather be told we are wrong than guess.

## Summary of the live probe

- **Where the agent runs.** The local CLI is an *executor*: it runs tools on the machine. The agent loop and inference run on Amp's server, behind a per-thread "thread actor" (live).
- **Local record.** There is no local transcript. Amp writes a per-thread metadata log and a pointer to the last thread id (live). See section 4.
- **Wire.** All traffic went to `ampcode.com` and `authapi.ampcode.com` over HTTP/1.1, through the proxy, trusting the proxy CA. The thread travels over one WebSocket as plain-text JSON-RPC: the user message goes up; text, thinking, tool calls and deltas come down; tool results go up (live). See section 6.
- **System prompt.** Amp's base system prompt did not appear on the wire. The client sends its tool descriptions up. Per-model mode prompts are downloaded to the client cache (live). See section 6.
- **Fit for Goal Cup.** A wire-only capture of the local CLI is possible with Goal Cup's existing proxy design plus an Amp protocol decoder. A native leg in the Claude Code sense does not exist.
- **Orbs.** A CLI on the participant's machine that starts an orb thread joins it as an observer and receives the same thread stream, so an orb thread can be recorded the same way (live, one run). See "Orbs" in section 7.

## 1. Run one prompt, in chat mode first

Chat mode: not measured. The command that opens Amp's chat, how the participant types the prompt, how to quit and the flag that reopens the last thread are unknown (please confirm). All measurements below are of the execute form.

Headless recipe (execute mode):

```
amp --stream-json --execute=<goal>
```

- The whole goal binds as the execute message, including goals that look like flags (`--help`, `-v`, `threads`). `-x <goal>` as two argv elements also works. Words after a bare `-x` are parsed as a command. Execute mode also turns on when stdout is not a terminal: `amp <command> --help` with stdout redirected fails with "User message must be provided" (live).
- `--stream-json` prints Claude Code-shaped stream JSON, one object per line (live): `system`/`init` (`cwd`, `session_id`, `tools` (29 names), `mcp_servers`, `agent_mode`), then `user`, `assistant` (text or `tool_use`), `user` (`tool_result`), and a final `result` (`subtype` `success`, `duration_ms`, `is_error`, `num_turns`, `result`, `session_id`). Every line carries `session_id`.
- Exit code after a successful run: 0 (live). Wall time 13.8 s for the one-word decoy and 17.7 s for the three-turn goal.
- `-m, --mode <value>` accepts `low` in execute mode (live). The init line reports `agent_mode` `medium` when no mode is given and `low` with `--mode low`. Whether `--mode` accepts the external agent key in execute mode: unknown (please confirm).
- Offline outcomes: with no key, stdout prints `No API key found. Starting login flow...` and a login URL, then waits for a pasted code; with stdin `/dev/null` it did not exit in 40 s. With an invented `AMP_API_KEY` and no route out, exit 1, empty stdout, stderr `Couldn't connect to the Amp server at https://ampcode.com/.`
- After an execute run, the CLI calls `/api/internal?archiveThread` (live).

## 2. Install and record a version

- `npm install @sourcegraph/amp@0.0.1791676897-g77c125`. This is an alias published under the dist-tag `latest` on 2026-10-11 with the description "Renamed to @ampcode/cli"; its only dependency is `@ampcode/cli` at the same version (npm metadata; please confirm that the alias will keep resolving).
- `@ampcode/cli` has `postinstall: node install.cjs`, which hard-links (or copies) the platform package's `amp` to `bin/amp`, makes `bin/amp.exe` a symlink to it, and makes no network call. Optional dependencies: `@ampcode/cli-{darwin-arm64,darwin-x64,linux-arm64,linux-x64,win32-x64}` at the same version (npm metadata; please confirm).
- The executable is a native single-file Bun 1.4.1 executable: ELF on Linux, Mach-O on macOS. No `node` at run time. Without the postinstall, `cli-wrapper.cjs` runs the native binary under `node` (package contents; please confirm).
- Where it lands (measured, Linux): `<prefix>/node_modules/@ampcode/cli/bin/amp`, a regular file with link count 2 (same inode as the platform package's `amp`); `<prefix>/node_modules/.bin/amp` links to `bin/amp.exe`, which links to `amp`. The daemon records the digest of the regular file.
- Print the version: `amp --version` prints `0.0.1791676897-g77c125 (released 2026-10-11T00:01:37.000Z, ...)`; `amp version` adds `npm package: @ampcode/cli`.
- Linux x64 payload: 134,157,792 bytes, SHA-256 `6c1b791d8e2c4661460b96276860c289d50ff0cacadaf1061ccb401295792036`, identical in the tarball, the platform package and `bin/amp` (measured).
- macOS arm64: the installed `bin/amp` is a Mach-O arm64 executable with SHA-256 `107c8b2c312e97447bd20d9f7d678d477195bd330753de81749b547bc7477f44`, the same as the bytes extracted from the tarball, and it was unchanged after both runs (live). macOS x64 and Windows installed bytes: unknown (please confirm).
- Other install paths seen in help or strings: `amp update [--target-version]` and `brew upgrade ampcode/tap/ampcode` (please confirm which applies to an npm install).

## 3. Where it keeps things, per OS

| | macOS | Linux | Windows |
|---|---|---|---|
| Config dir | No settings file was written during the live probe. Help names the default `$HOME/.config/amp/settings.json` (live). Managed settings `/Library/Application Support/ampcode/managed-settings.json` (Amp docs; please confirm). Workspace `.amp/settings.json` | `$HOME/.config/amp/settings.json` (and `.jsonc`); follows `XDG_CONFIG_HOME`. Machine-wide `/etc/ampcode/managed-settings.json` (read attempt measured). Workspace `<root>/.amp/settings.json`, where root is the nearest ancestor holding `.git`, else the cwd | Managed settings `%ProgramData%\ampcode\managed-settings.json` (Amp docs; please confirm). User settings path: unknown (please confirm) |
| Session / transcript store | None. Thread content lives on Amp's server (live). Local per-thread files are in section 4 | No thread file appeared offline; strings point to server-side thread actors (`/api/thread-actors`, `/api/threads/`) | unknown (please confirm) |
| Data | `$HOME/.local/share/amp/` (live): `accounts.json`, `secrets.json`, `device-id.json` (all mode 0600, written by `amp login`), `session.json` (last mode and UI state), `.legacy-state-owner.json`, `last-threads/*.json` | `$HOME/.local/share/amp/` holding `device-id.json` (mode 0600) and `.legacy-state-owner.json`; reads were attempted for `accounts.json`, `secrets.json` and `workspace-managed-settings-<hash>.json` | unknown (please confirm) |
| Logs / cache | `$HOME/.cache/amp/` (live): `logs/cli.log`, `logs/plugin-runtime.log`, `logs/threads/T-<uuid>.log`, `global-plugins/ampcode.com/...` (mode definitions), `global-skills/ampcode.com/official/...` (skills). Bun writes `$HOME/Library/Caches/bun/` and extracts `$TMPDIR/.bun-501-<hash>.node` | Log `$HOME/.cache/amp/logs/cli.log` (follows `XDG_CACHE_HOME`). Bun extracts a native addon to `$TMPDIR/.bun-0-<hash>.node` (mode 0600) and reuses it on later runs | unknown (please confirm) |
| Env vars that move these | `HOME` (live: every file above stayed under the private `HOME` and `TMPDIR`). `XDG_*` presumably as on Linux (please confirm). `AMP_SETTINGS_FILE` and `--settings-file` name a settings file | `HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME`, `XDG_DATA_HOME` (settings and log measured to follow XDG when set); `AMP_HOME` did not move settings or log (its other effect: unknown, please confirm); `TMPDIR` for the Bun addon | unknown (please confirm which profile variables apply) |

On macOS, Amp uses the same XDG-style paths under `$HOME` as on Linux, not `~/Library/Application Support` (live).

Skills are scanned from the cwd and every ancestor up to `/`; Amp also loads skills from Claude Code directories unless `amp.skills.disableClaudeCodeSkills` is set (measured for the scan; docs for the setting). Official skills and mode definitions are fetched from `ampcode.com` (`/api/internal?loadSkills`, `?loadPlugins`) and cached under `$HOME/.cache/amp/` (live). Whether `AGENTS.md` guidance files are searched in ancestors: unknown (`AMP_IGNORE_GUIDANCE_FILES` exists; please confirm).

## 4. Session format and identity

- **No local transcript** (live). After two runs, no file under the private `HOME` or `TMPDIR` held the conversation.
- **Per-thread log** (live): `$HOME/.cache/amp/logs/threads/T-<uuid>.log`, one per thread, JSON lines with `@timestamp`, `level`, `message`, `threadId` and `pid`. It records transport state changes, the executor handshake, JSON-RPC requests sent and completed, server message types with frame lengths, and each tool execution (`executing tool: create_file`, `executing tool: async_shell`). It holds almost no conversation text: the string `goal-cup probe` from the measured goal appeared once. It is a metadata log, not a transcript, but its tool events and frame counts can cross-check a wire capture.
- **Last-thread pointer** (live): `$HOME/.local/share/amp/last-threads/<hash>-<hash>-execute-session.json` holds only the thread id as a JSON string.
- **Session id** (live): `T-<uuid>`, for example `T-01a13ecd-...`. It is the `session_id` of the first stdout line (`system`/`init`) and of every later line, the name of the per-thread log, the value in the last-thread pointer, and the `tid` claim of the WebSocket token. The CLI creates the thread with `POST /api/thread-actors` before the user message is sent, so the id is bound before the first model turn. The two runs got different ids.
- A file derived from the key: `$HOME/.local/share/amp/.legacy-state-owner.json` holds `identityKey`, 16 hex digits, identical for the same invented token across three fresh homes and different for another token (measured, Linux). It was also written in the live macOS probe. Its derivation and purpose: unknown (please confirm).

## 5. Auth

- `amp login` uses a device-code flow (live): it prints `https://ampcode.com/auth/cli/device?user_code=XXXX-XXXX` and a matching code, then polls `authapi.ampcode.com/user_management/authenticate` (HTTP 400 until the browser confirms, then 200) and prints `Logged in as <email>.` Exit 0.
- Credential storage on macOS (live): `$HOME/.local/share/amp/accounts.json` and `secrets.json`, mode 0600. The macOS Keychain was not changed: a listing of Keychain item names before the login and after both runs was identical.
- `AMP_API_KEY` is an access token that "takes precedence over stored credentials" (help text; please confirm). The live probe used `amp login`, not a key. Token grammar and length: unknown (please confirm). Goal Cup currently assumes 20 to 256 bytes of `[A-Za-z0-9_-]` and fails closed on anything else.
- **The WebSocket carries a credential in a header** (live): each thread WebSocket sends a short-lived JWT (`wsToken`, about 5 minutes, claims include `tid` and `sub`) inside the `Sec-WebSocket-Protocol` header (`rivet_conn_params`). Any capture must strip this header before hashing, storing or uploading.
- Claude Code mode (docs, please confirm): a Claude subscription is used through the local `claude` binary and its login; Amp removes `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN` and the Bedrock, Vertex and Foundry settings from Claude's environment, and needs Claude Code 2.1.276 or later. Not exercised in the live probe.
- What a rejected or revoked key prints (does it echo the key?): unknown (please confirm).

## 6. Network

- **Hosts** (live): `ampcode.com` and `authapi.ampcode.com` only. No model provider host was contacted from the machine. All requests used HTTP/1.1.
- **REST** (live): JSON over `POST /api/internal?<method>` (`getUserInfo`, `recordCLISignIn`, `loadPlugins`, `loadSkills`, `getThreadTail`, `getThreadProject`, `getSecretFingerprints`, `archiveThread`), `POST /api/thread-actors` (201), and `GET /api/auth/oauth-config`.
- **Thread transport** (live): a WebSocket upgrade on `ampcode.com/actors/gateway/threadActor/websocket/` with subprotocols `rivet`, `rivet_encoding.bare`, `rivet_skip_ready_wait` and `rivet_conn_params.<json>`, where the JSON names `"transport":"json-rpc"`. Despite `bare`, every frame seen was text: JSON objects or arrays, plus `ping`/`pong`. No CBOR was seen. Each run opened two thread WebSockets.
- **What the WebSocket carries** (live). Up: `executor_connect`, `executor_environment_snapshot`, `executor_skill_snapshot`, `executor_guidance_snapshot`, `executor_tools_register` (the client's tool names and descriptions), `client_append_user_msg`, `executor_tool_lease_ack`, `tool_progress`, `executor_tool_result`. Down: `message_added`, `delta`, `text`, `thinking`, `tool_use`, `tool_lease`, `agent_state`, `inference_tools`, `thread_settings`, `thread_title`, `complete`, among others. The whole conversation, including tool calls and results, is on this connection.
- **Model identity on the wire** (live): the default mode showed `claude-opus-5-5`; `--mode low` showed `accounts/fireworks/models/glm-5p3-flash`.
- **System prompt** (live): Amp's base system prompt did not appear in any captured request or frame. The tool descriptions the client registers do appear. Per-model mode prompts ("You are an autonomous coding agent ...") are part of the mode definitions that `loadPlugins` downloads to every logged-in client and caches under `$HOME/.cache/amp/global-plugins/` (13 of the 15 mode files contain such text).
- **Proxy and CA** (live): with process-scoped `HTTPS_PROXY` and `NODE_EXTRA_CA_CERTS` naming the proxy's CA, every request, including the login, REST and WebSocket traffic, went through the proxy and trusted that CA. Whether Amp fails closed without the CA: not tested.
- Update check: `https://static.ampcode.com/cli/cli-version.txt` (string). With `AMP_SKIP_UPDATE_CHECK=1`, no request to `static.ampcode.com` was seen (live). Behaviour without that variable: not tested.
- Telemetry: no request to `/api/telemetry` was seen in the live runs. Its contents and opt-out: unknown (please confirm).
- Ripgrep download: `https://storage.googleapis.com/amp-public-assets-prod-0/ripgrep` (string); not seen in the live runs.
- Base-URL override variables (Goal Cup never sets these): `AMP_URL`. Goal Cup refuses any `*_URL` name since 2026-10-12.

## 7. Process model

- **The local CLI is an executor** (live). The server sends a `tool_lease`; the CLI runs the tool locally (`create_file`, `async_shell` in the measured goal), and sends `executor_tool_result`. Inference and the agent loop run on Amp's server.
- Subprocesses seen offline: `git rev-parse --show-toplevel` only. The live probe did not trace subprocesses.
- Claude Code mode (docs, please confirm): Amp spawns the local `claude` through the Claude Agent SDK; the runner finds `claude` on `PATH`, in `~/.local/bin` or in `~/bin`.
- `--executor local|orb|runner:<id>` exists (help). With the live result above, `local` means the tools run on this machine and the thread runs on Amp's server.
- Behaviour on SIGTERM and Ctrl-C, and the exit codes other than 0 and 1: unknown (please confirm).
- Subagents (`Task`, `oracle`, `librarian`, `finder` and others appear in the init tool list): where they run and whether their traffic uses the same WebSocket: unknown (please confirm).

### Orbs (live, one run)

- `amp --stream-json --execute=<goal> --orb-execute --mode low` (the `-ox` shorthand) from a directory with no Git remote ran the thread in an orb without a repository and printed a warning on stderr that no Amp project matched. The orb was a Debian 12 x86_64 VM (`uname` host `e2b.local`). Exit 0, wall time 22.5 s.
- The CLI sent the goal in `POST /api/internal?createRemoteExecutorThread` (fields `agentMode`, `content`, `features`, `userState`, `projectID`, `threadMeta`), then joined the thread WebSocket as an **observer**: the per-thread log records `"role":"observer"` and no executor callbacks. Its only upward message was `client_resume`.
- Down the observer's WebSocket came the whole thread: `message_added`, `delta`, `text`, `tool_use` (the orb's `shell_command`), `tool_progress`, the tool output (the `uname` text), `snapshot`, `agent_state`, `executor_status` and `complete`. stdout printed the same Claude Code-shaped stream as a local run, with the orb's tool call and result.
- So a client on the participant's machine can record an orb thread through the same proxy, with nothing executed locally. Not yet tested: whether an observer that connects late or reconnects receives the full history; whether threads that an orb spawns (`create_thread`) must be followed separately; and a run long enough for the orb to sleep and wake.

## 8. Things Goal Cup cannot isolate

- Machine-wide managed settings: `/etc/ampcode/managed-settings.json` on Linux was read during the probe, outside any private home (measured). The macOS and Windows locations are from docs (please confirm).
- Server-side state: the thread itself, workspace managed settings, global plugins and official skills are held on `ampcode.com` and fetched at run time (live).
- Skills from ancestors of the workspace and from Claude Code directories.
- The Bun addon reused from `$TMPDIR`: a file planted there before the run would be loaded (measured, Linux: reused after only a `stat`).
- The unpinned `claude` child in Claude Code mode.

## 9. Quirks and open questions

- Execute mode opens an interactive login on a missing key, with the login text on stdout before any JSON.
- Execute mode also starts when stdout is not a terminal, so `amp <command> --help` piped to a file fails instead of printing help (live).
- The `latest` dist-tag is the native-binary line; an older dist-tag, `next`, was published 2026-05-14 (npm metadata).

### Open questions for Amp

1. Would Amp write a local, append-only record of each thread (or offer a supported local export), so Goal Cup can check the wire capture against a second record?
2. Is the JSON-RPC message set on the thread WebSocket stable enough to decode, and is there a schema or version field we should key on?
3. Does Amp fail closed when `NODE_EXTRA_CA_CERTS` names a missing CA, rather than fall back to system trust?
4. What are the token grammar and length for `AMP_API_KEY`, and does a rejected or revoked key get echoed in output, logs or files?
5. What is `.legacy-state-owner.json` `identityKey` derived from, and is there a setting that stops it being written?
6. What are the user config, data and log locations on Windows, and the full macOS managed-settings path?
7. Do subagents (`Task`, `oracle`, `librarian`) run inside the same thread actor and stream over the same WebSocket?
8. Does the update check run in execute mode without `AMP_SKIP_UPDATE_CHECK=1`? What does an npm-installed binary do on `amp update`?
9. Does `--mode` accept the Claude Code external agent in execute mode, with what key?
10. What else does `AMP_HOME` change?
11. Are `AGENTS.md` guidance files searched in ancestors of the workspace?
12. Are the macOS x64 and Windows installed executable bytes the same as the extracted tarball bytes?
13. Orbs: does an observer that connects late, or reconnects after a drop, receive the full thread history? Is there a supported read-only way to follow a thread (and the threads it spawns) for many hours?
