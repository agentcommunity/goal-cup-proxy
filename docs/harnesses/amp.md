# Amp harness facts

Connector id: `amp-cli` (proposed; no connector exists) · Facts for version: `0.0.1791676897-g77c125` · Written by: Goal Cup team, draft for the Amp team · Last checked: 2026-10-12
Public disclosure: yes. The facts come from Amp's public documentation, string literals and help text in the shipped npm package, and a key-free, network-less probe on one Linux host. No Amp credential was used, no authenticated request was made, and no Amp source code was read. Raw probe output is not published.
Status: facts only, draft for the Amp team to confirm

How to read this sheet. A fact with no marker was measured by Goal Cup's key-free probe (Linux x86_64, glibc, every Amp process in a network namespace with no route out). Every other fact is marked "(please confirm)" and is one of: Amp public docs fetched 2026-10-11, a string or help text read from the package without running that path, or "unknown". We would rather be told we are wrong than guess. The full evidence is in the [Amp readiness record](../qualification/m2-amp-readiness.md); the Goal Cup changes made so far are in [amp-core-prep](../research/amp-core-prep.md).

## 1. Run one prompt, headless

```
amp --stream-json --execute=<goal>
```

- The whole goal binds as the execute message, including goals that look like flags (`--help`, `-v`, `threads`). `-x <goal>` as two argv elements also works. Words after a bare `-x` are parsed as a command. Help says execute mode also turns on when stdout is redirected.
- `--stream-json` prints Claude Code-compatible stream JSON in execute mode (help text; please confirm the event shapes).
- Measured offline outcomes: with no key, stdout prints `No API key found. Starting login flow...` and a login URL, then waits for a pasted code; with stdin `/dev/null` it did not exit in 40 s. With an invented `AMP_API_KEY` and no route out, exit 1, empty stdout, stderr `Couldn't connect to the Amp server at https://ampcode.com/.`
- Exit code and final output after a real successful run: unknown (please confirm).
- `-m, --mode <value>` selects `low`, `medium`, `high`, `ultra` or a plugin mode by key or label (help text). Whether `--mode` accepts the external agent key in execute mode: unknown (please confirm). An unknown mode and a built-in one failed identically before server contact.

## 2. Install and pin a version

- `npm install @sourcegraph/amp@0.0.1791676897-g77c125`. This is an alias published under the dist-tag `latest` on 2026-10-11 with the description "Renamed to @ampcode/cli"; its only dependency is `@ampcode/cli` at the same version (npm metadata; please confirm that the alias will keep resolving).
- `@ampcode/cli` has `postinstall: node install.cjs`, which hard-links (or copies) the platform package's `amp` to `bin/amp`, makes `bin/amp.exe` a symlink to it, and makes no network call. Optional dependencies: `@ampcode/cli-{darwin-arm64,darwin-x64,linux-arm64,linux-x64,win32-x64}` at the same version (npm metadata; please confirm).
- The executable is a native single-file Bun 1.4.1 executable: ELF on Linux, Mach-O on macOS. No `node` at run time. Without the postinstall, `cli-wrapper.cjs` runs the native binary under `node` (package contents; please confirm).
- Where it lands (measured, Linux): `<prefix>/node_modules/@ampcode/cli/bin/amp`, a regular file with link count 2 (same inode as the platform package's `amp`); `<prefix>/node_modules/.bin/amp` links to `bin/amp.exe`, which links to `amp`. Goal Cup pins the regular file.
- Print the version: `amp --version` prints `0.0.1791676897-g77c125 (released 2026-10-11T00:01:37.000Z, ...)`; `amp version` adds `npm package: @ampcode/cli`.
- Linux x64 payload: 134,157,792 bytes, SHA-256 `6c1b791d8e2c4661460b96276860c289d50ff0cacadaf1061ccb401295792036`, identical in the tarball, the platform package and `bin/amp` (measured). macOS arm64 and x64 payload hashes were read from extracted tarballs, not from a macOS install: please confirm the installed bytes. Windows payload: unknown (please confirm).
- Other install paths seen in help or strings: `amp update [--target-version]` and `brew upgrade ampcode/tap/ampcode` (please confirm which applies to an npm install).

## 3. Where it keeps things, per OS

| | macOS | Linux | Windows |
|---|---|---|---|
| Config dir | User settings `~/.config/amp/settings.json` (Amp docs; please confirm). Managed settings `/Library/Application Support/ampcode/managed-settings.json` (Amp docs; please confirm). Workspace `.amp/settings.json` | `$HOME/.config/amp/settings.json` (and `.jsonc`); follows `XDG_CONFIG_HOME`. Machine-wide `/etc/ampcode/managed-settings.json` (read attempt measured). Workspace `<root>/.amp/settings.json`, where root is the nearest ancestor holding `.git`, else the cwd | Managed settings `%ProgramData%\ampcode\managed-settings.json` (Amp docs; please confirm). User settings path: unknown (please confirm) |
| Session / transcript store | unknown (please confirm) | unknown. No thread file appeared offline; strings point to server-side "thread actors" (`/api/thread-actors`, `/api/threads/`), and `threads export` / `threads raw` fetch from the server (please confirm) | unknown (please confirm) |
| Logs / cache | unknown (please confirm) | Log `$HOME/.cache/amp/logs/cli.log` (follows `XDG_CACHE_HOME`); data `$HOME/.local/share/amp/` holding `device-id.json` (mode 0600) and `.legacy-state-owner.json`; reads were attempted for `accounts.json`, `secrets.json` and `workspace-managed-settings-<hash>.json`. Bun extracts a native addon to `$TMPDIR/.bun-0-<hash>.node` (mode 0600) and reuses it on later runs | unknown (please confirm) |
| Env vars that move these | `HOME`, `XDG_*` presumably as on Linux (please confirm). `AMP_SETTINGS_FILE` and `--settings-file` name a settings file (Amp docs; please confirm absolute values are accepted) | `HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME`, `XDG_DATA_HOME` (settings and log measured to follow XDG when set); `AMP_HOME` did not move settings or log (measured; its other effect: unknown, please confirm); `TMPDIR` for the Bun addon | unknown (please confirm which profile variables apply) |

Skills are scanned from the cwd and every ancestor up to `/`; Amp also loads skills from Claude Code directories unless `amp.skills.disableClaudeCodeSkills` is set (measured for the scan; docs for the setting). Whether `AGENTS.md` guidance files are searched in ancestors: unknown (`AMP_IGNORE_GUIDANCE_FILES` exists; please confirm).

## 4. Session format and identity

- File type, one file per session or shared: unknown (please confirm). We have not seen a local own-session file, and believe threads live on Amp's server; if so there is nothing for Goal Cup to read as a native store.
- Session id form: documentation names a stdout event with `type` `system` and subtype `init` carrying `session_id`, and thread ids look like `T-<uuid>` (Amp docs and help; please confirm).
- Where the id first appears: unknown. Neither offline failure path emitted an init event, so it presumably follows server contact (please confirm). Whether it precedes the first model request, whether it matches any file name or field, and its uniqueness: unknown (please confirm).
- A file derived from the key: `$HOME/.local/share/amp/.legacy-state-owner.json` holds `identityKey`, 16 hex digits, identical for the same invented token across three fresh homes and different for another token (measured). Its derivation and purpose: unknown (please confirm). It is not a plain SHA-256, SHA-1, MD5 or SHA-512 of the token.

## 5. Auth

- Auth the CLI accepts: an access token in `AMP_API_KEY`, which "takes precedence over stored credentials", and `amp login`, which stores credentials in the data directory (help text and strings; please confirm).
- Token grammar and length: unknown (please confirm). Goal Cup currently assumes 20 to 256 bytes of `[A-Za-z0-9_-]` and fails closed on anything else.
- Credential storage per OS: Linux: "file-based secrets storage" under the data dir (strings; please confirm the file names). macOS Keychain use: unknown (please confirm). Windows: unknown (please confirm).
- Claude Code mode (docs, please confirm): a Claude subscription is used through the local `claude` binary and its login; Amp removes `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN` and the Bedrock, Vertex and Foundry settings from Claude's environment, and needs Claude Code 2.1.276 or later.
- What a rejected or revoked key prints (does it echo the key?): unknown (please confirm).
- Files derived from the key: see `.legacy-state-owner.json` in section 4.

## 6. Network

- Server: `AMP_URL`, default `https://ampcode.com/`. Seen offline: `GET https://ampcode.com/api/internal?loadPlugins`. Strings: `/api/thread-actors`, `/api/threads/`, `/api/telemetry`.
- Update check: `https://static.ampcode.com/cli/cli-version.txt` (string; please confirm whether it runs in execute mode). Ripgrep download: `https://storage.googleapis.com/amp-public-assets-prod-0/ripgrep` (string; please confirm when it triggers and where it lands).
- Model/provider hosts for Amp's own agent: none found in the binary; inference appears to run behind `ampcode.com` (please confirm). Whether bring-your-own-key inference goes straight to the provider or through Amp: unknown (please confirm). `api.anthropic.com` appears as the default base URL of an embedded Anthropic SDK.
- Transport: strings show a rivetkit actor client (`Sec-WebSocket-Protocol`, `rivet_encoding`), a `cbor-extract` addon and `text/event-stream`. WebSocket versus SSE, HTTP version and body encoding (JSON or CBOR): unknown (please confirm).
- Telemetry: `/api/telemetry` exists; its contents and opt-out: unknown (please confirm).
- `HTTPS_PROXY`: the embedded Bun runtime reads `HTTPS_PROXY`, `https_proxy` and `NO_PROXY`, and a setting `amp.proxy` exists (please confirm). Whether the actor WebSocket honours it: unknown (please confirm).
- Custom CA: the embedded runtime reads `NODE_EXTRA_CA_CERTS` (extend) and offers `--use-system-ca` / `NODE_USE_SYSTEM_CA`. Live trust behaviour: unknown (please confirm).
- Base-URL override variables (Goal Cup never sets these): `AMP_URL`. Goal Cup refuses any `*_URL` name since 2026-10-12.

## 7. Process model

- Subprocesses seen: `git rev-parse --show-toplevel` only (offline probe). No `claude` child started in Claude Code mode offline (nothing reached the server).
- Claude Code mode (docs, please confirm): Amp spawns the local `claude` through the Claude Agent SDK; the runner finds `claude` on `PATH`, in `~/.local/bin` or in `~/bin`. Goal Cup's capture launch uses `PATH=/usr/bin:/bin:/usr/sbin:/sbin`, which would not find it (code); whether Amp has another way: unknown (please confirm).
- `--executor local|orb|runner:<id>` exists (help); what "local" keeps on the machine: unknown (please confirm).
- Behaviour on SIGTERM and Ctrl-C, and the exit codes other than 0 and 1: unknown (please confirm).
- Tools and subagents: unknown (please confirm).

## 8. Things Goal Cup cannot isolate

- Machine-wide managed settings: `/etc/ampcode/managed-settings.json` on Linux was read during the probe, outside any private home (measured). The macOS and Windows locations are from docs (please confirm).
- Server-side state: workspace managed settings and global plugins are fetched from `ampcode.com`; the external agent's post-install script and system prompt addition are account settings held there (help text; please confirm).
- Skills from ancestors of the workspace and from Claude Code directories.
- The Bun addon reused from `$TMPDIR`: a file planted there before the run would be loaded (measured: reused after only a `stat`).
- The unpinned `claude` child in Claude Code mode, and the thread itself, which lives on Amp's side.

## 9. Quirks and open questions

- Execute mode opens an interactive login on a missing key, with the login text on stdout before any JSON.
- The `latest` dist-tag is the native-binary line; an older dist-tag, `next`, was published 2026-05-14 (npm metadata).
- Nothing on this sheet was measured against a live Amp service.

### Open questions for Amp

1. Where does Amp keep a thread, and is there any local own-session file (path, form, scope per thread, append-only or rewritten)? If threads live only on your server, is there a supported local record?
2. What is the identity carrier? Does the stdout `system`/`init` event with `session_id` exist in execute mode, when is it written relative to the first model request, and is the id stable and unique?
3. Which transport carries a thread: WebSocket, SSE or both, which HTTP version, and is the body JSON or CBOR? Do the actor WebSocket and all other calls honour `HTTPS_PROXY`?
4. Does Amp trust `NODE_EXTRA_CA_CERTS` in a live run, and does it fail closed rather than fall back to system trust when the CA is missing?
5. What are the token grammar and length for `AMP_API_KEY`, and does a rejected or revoked key get echoed in output, logs or files?
6. What is `.legacy-state-owner.json` `identityKey` derived from, and is there a setting that stops it being written?
7. Does Amp use the macOS Keychain, or write `accounts.json` or `secrets.json`, and where are credentials stored on macOS and Windows?
8. What are the user config, data and log locations on macOS and Windows, and the full macOS managed-settings path?
9. Does `--mode` accept the Claude Code external agent in execute mode, with what key, and does `amp config external-agents` need the "External agents" experimental feature turned on?
10. In Claude Code mode, how does Amp find `claude`, does the child inherit `HTTPS_PROXY` and `NODE_EXTRA_CA_CERTS`, and does Amp use `persistSession: false` or a `sessionStore` that mirrors transcripts to your server?
11. Does the update check run in execute mode, and does `AMP_SKIP_UPDATE_CHECK=1` suppress it? What does an npm-installed binary do on `amp update`?
12. What does `--executor local` keep on this machine, and what else does `AMP_HOME` change?
13. Are `AGENTS.md` guidance files searched in ancestors of the workspace?
14. Does bring-your-own-key inference go straight to the provider or through `ampcode.com`?
15. Are the macOS arm64 and x64 and the Windows installed executable bytes the same as the extracted tarball bytes?
