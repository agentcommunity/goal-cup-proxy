# OpenCode harness facts

Connector id: `opencode-cli` · Facts for version: `1.19.4` · Written by: Goal Cup team (agent-written, measured in a Linux cloud container) · Last checked: 2026-10-28
Public disclosure: yes. No keys, tokens or personal paths. Every probe used a private `HOME` and an invented key.
Status: facts only. OpenCode keeps its sessions only in a SQLite database, which schema 1 cannot
capture ([Cannot be captured](../HARNESS-ONBOARDING.md#row-0), "A database session store"; D44).

Source words: "measured" means run on Linux x86_64 (kernel 6.18), OpenCode 1.19.4, 2026-10-28,
with a private `HOME`, through a local test proxy; "vendor docs" means opencode.ai/docs as fetched
on 2026-10-28; "unknown" means not measured or read. "Measured in a run" means a local Goal Cup
run (goalcup 0.7.0, `--dev`, the unpinned proof connector) in a Vercel Sandbox, Linux x86_64,
2026-10-28, with real Gateway and DeepSeek replies (see the [results file](../research/results/2026-10-opencode-openclaw.md)).

## 1. Run one prompt, in chat mode first

- Chat mode: `opencode` with no argument opens the TUI in the current folder (help text:
  `opencode [project]  start opencode tui [default]`). Driven under `goalcup start` in tmux: it
  records (measured in a run). Exit: Escape, then Ctrl-C twice; the run seals at once.
- Reopen the last session: `opencode --continue` (`-c`), or `--session <id>` (help text).
- One-shot mode (not chat mode, used only for the probes below): `opencode run "<prompt>"`.
- Modes that switch capture off are listed in [RULES](../RULES.md): `--pure`, `OPENCODE_PURE=1`,
  `OPENCODE_DISABLE_PROJECT_CONFIG=1`, `--attach`, `share`, `import`, the dispose endpoints. A
  connector would guard them with `argv_guard` and `reserved_env_prefixes = ["OPENCODE_"]`.

## 2. Install and record a version

- Official installer: `curl -fsSL https://opencode.ai/install | bash` (`--version <v>`,
  `--no-modify-path`). Binary: `$HOME/.opencode/bin/opencode`, a native ELF executable on Linux,
  not an npm wrapper (measured). The installer refuses 2.x; 2.x has its own beta installer
  (vendor docs).
- Version: `opencode --version` prints `1.19.4` (measured). It also creates empty
  `$HOME/.config/opencode` and `$HOME/.local/share/opencode/log` in whatever `HOME` it runs with
  (measured), so a version probe must use a private `HOME` too.

## 3. Where it keeps things, per OS

| | macOS | Linux | Windows |
|---|---|---|---|
| Config dir | unknown | `$HOME/.config/opencode/opencode.json` (measured, `opencode debug paths`) | unknown |
| Session / transcript store | unknown | `$HOME/.local/share/opencode/opencode.db`, SQLite (measured) | unknown |
| Logs / cache | unknown | `$HOME/.local/share/opencode/log/opencode.log`, `$HOME/.cache/opencode/` (`models.json`, `bin/`), `$HOME/.local/state/opencode/locks`, `/tmp/opencode` (measured) | unknown |
| Env vars that move these | | `OPENCODE_CONFIG_DIR` (config only), `OPENCODE_CONFIG` (file), `OPENCODE_DB` (database), `XDG_*` (vendor docs) | |

Provider packages are installed at first use into `$HOME/.config/opencode/node_modules` from
`registry.npmjs.org` (measured).

## 4. Session format and identity

- One shared SQLite file for every session: `opencode.db`, tables `session`, `message`, `part`,
  `event`, `credential` and 15 more (measured). One failed one-shot turn wrote 1 `session`,
  2 `message` and 1 `part` row (measured). No JSON or JSONL session file is written; the old
  `storage/` JSON tree is gone (measured, vendor docs).
- Session ids look like `ses_<26 chars>` and appear in the log line `message=created id=ses_...`
  (measured). `opencode export <id>` prints a session as JSON on demand (vendor docs).
- This is the stop rule. A connector needs a read-only snapshot native form for SQLite (D44).

## 5. Auth

- API keys: `opencode auth login` (alias `opencode providers`) or `/connect` stores them in
  `$HOME/.local/share/opencode/auth.json` as `{"<provider>": {"type": "api", "key": "..."}}`
  (vendor docs; format measured by `opencode auth list` reading a file written in that shape).
- A key in `auth.json` works with no environment variable: an invented key in `auth.json` was sent
  as `Authorization: Bearer <key>` to the Gateway with no `AI_GATEWAY_API_KEY` set (measured).
  Without `auth.json` the `vercel` provider does not load at all (`Model not found`, measured).
- Keys can also sit in config (`provider.<id>.options.apiKey`, or `{env:VAR}`), or come from the
  environment or a project `.env` (vendor docs).
- A variable that moves config and login together: no. `OPENCODE_CONFIG_DIR` moves config only;
  `auth.json` and the database follow `XDG_DATA_HOME`, which a connector may not set (row 6). So a
  connector would keep one shared private `HOME` (`run_home = "shared"`, `config_home = "per_run"`).

## 6. Network

- CA: `NODE_EXTRA_CA_CERTS` adds to the roots (mode `extend`). With a private test CA: set, TLS to
  the test proxy succeeded; unset, OpenCode failed with `unable to verify the first certificate`
  (measured). `SSL_CERT_FILE` alone also worked (measured; Bun reads it).
- `HTTPS_PROXY`: honoured on its own (measured, every request went through the test proxy).
  `NO_PROXY=localhost,127.0.0.1` is needed so the TUI's local server is not proxied (vendor docs).
  Node: no, a Bun-compiled binary, so `NODE_USE_ENV_PROXY` is not needed.
- Model address: OpenCode does not write it to a file. The Vercel provider (`vercel`) and DeepSeek
  provider (`deepseek`) take their URL from the models.dev catalog; only an optional
  `provider.<id>.options.baseURL` in `opencode.json` names one (vendor docs; measured: our config
  had none). So `model_host_from` has nothing to read by default.
- Wire, measured with an invented key: Gateway `POST https://ai-gateway.vercel.sh/v3/ai/language-model`
  (the AI SDK gateway protocol, not `/chat/completions`). Same with a real key (measured in a
  run: 27 calls on one connection). DeepSeek `deepseek/deepseek-flash`:
  `POST https://api.deepseek.com/chat/completions` (measured in a run).
- **Unknown model id falls back to a paid model.** `deepseek/deepseek-v4-flash` is not in the
  catalog; the TUI then opened on `google/gemini-3-pro-image` through the Gateway with no warning
  (measured in a run; the Gateway answered 403). The catalog (`opencode models deepseek`) lists
  `deepseek/deepseek-flash` and `deepseek/deepseek-v4-pro`.
- Other hosts, measured from a fresh home: `models.opencode.ai` (`GET /api.json`, the model
  catalog), `registry.npmjs.org` (provider packages). Off switches: `OPENCODE_DISABLE_MODELS_FETCH`,
  `OPENCODE_DISABLE_AUTOUPDATE` or `"autoupdate": false`, `"share": "disabled"` (vendor docs).
- The default "small model" for titles is a paid Gateway model (`google/gemini-nano-banana-2.1`,
  measured); set `small_model` to keep a free-model test free.
- No base URL variable is set by Goal Cup (D1).

## 7. Process model

One binary; the TUI talks to its own local HTTP server. Tools run as child shells (`/usr/bin/bash`
here, measured). `opencode run` exits 1 on a provider error (measured). Ctrl-C twice ends the
TUI (measured in a run). SIGTERM and subagents: unmeasured.

## 8. Things Goal Cup cannot isolate

- Managed config in `/etc/opencode/` (Linux) and `/Library/Application Support/opencode/` (macOS),
  and a remote `.well-known/opencode` config, both merged above the user's (vendor docs).
- `/tmp/opencode` is shared by every `HOME` (measured, `opencode debug paths`).
- OpenCode reads `~/.claude/CLAUDE.md` and `.claude/skills` unless `OPENCODE_DISABLE_CLAUDE_CODE`
  is set (vendor docs). In a private run `HOME` that finds nothing.

## 9. Quirks and open questions

- Two `opencode run` probes hung at `init` for 90 s and then later ones worked with the same
  settings (measured, cause unknown).
- Could the Gateway's `/v3/ai/language-model` wire be decoded? No decoder exists today.
- The TUI writes `$HOME/.local/share/opencode/log/opencode.log` during a run, so a native log row
  finds a file changed since the run started (measured in a run).

## Run without permission prompts

`opencode.json`: `"permission": "allow"` or `"permission": {"edit": "allow", "bash": "allow"}`
(vendor docs). Most permissions already default to `allow`; `external_directory`, `doom_loop` and
reads of `*.env` default to `ask` (vendor docs). `opencode run --auto` approves for one run. The
setting was written; with it, the TUI wrote and edited `chess.py`, and no permission prompt
showed in the screens captured (measured in a run).
