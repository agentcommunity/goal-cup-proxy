# Handover: OpenCode and OpenClaw light proof, for a session with the keys

For the next agent. Read this file, then the [results](2026-10-opencode-openclaw.md) and the two
facts sheets ([opencode-cli](../../harnesses/opencode-cli.md),
[openclaw-cli](../../harnesses/openclaw-cli.md)). Branch: `opencode-openclaw` (draft PR open).
Written 2026-10-28.

**Status: done 2026-10-28** by a second session; the light proof passed for both harnesses. See the
[results](2026-10-opencode-openclaw.md#runs). Two notes for anyone reusing this file: the free
Gateway model below is rate limited to 5 requests per minute per team, and `deepseek-v4-flash` is
not in either harness's catalog; use `deepseek/deepseek-flash`.

## Where things stand

- Both harnesses were installed and probed with a private `HOME` and invented keys. Both honour
  `HTTPS_PROXY` and `NODE_EXTRA_CA_CERTS`, and both read a key from their own home file.
- Both keep sessions only in SQLite. By the guide that is "Cannot be captured": both sheets are
  `Status: facts only`. Do not add `connectors/opencode-cli` or `connectors/openclaw-cli`, and do
  not change pins, unless the owner decides otherwise.
- Not done, because the first session had no keys: one real model reply per harness, and the light
  proof that Goal Cup records each one (step 4 below).

## What the owner asked for (the current plan)

Light proof only: show each harness's traffic records under Goal Cup. The owner tests in depth
locally later.

1. Install the Vercel Sandbox CLI. Create one sandbox named `gc-connectors` (4 vCPUs, 40 min
   timeout). **Another session uses `gc-gateway`: never touch it.** Everything happens in this one
   sandbox; it stops at 45 minutes.
2. Pass `AI_GATEWAY_API_KEY` and `DEEPSEEK_API_KEY` in without printing them. Never put a key in
   logged argv.
3. In the sandbox: install goalcup with the release `install.sh`, then OpenCode and OpenClaw with
   their official installers. Copy the proof connectors (below) in, point `GOALCUP_CONNECTORS_DIR`
   at them, and use `--dev`. If the release binary refuses them, build goalcup from this branch in
   the sandbox.
4. Per harness, one local run with no account (README "Try a local run without an account"),
   through the Gateway with `poolside/laguna-s-2.1-free` (fallback
   `nvidia/nemotron-3.5-lightning-free`). Time left: one more through DeepSeek. Prompt:
   "Build a chess game in Python." Let it work about 3 minutes, exit the harness so the run seals,
   run the printed `verify:` command.
   **Pass:** integrity valid, intercepted above 0, the model host not tunnelled. Record the
   verifier line and the `proxy admission:` line.
5. If a harness does not record, do not dig deep: write what you saw, keep it "facts only", move
   on. Update the results file and the draft PR, stop and delete the sandbox, push.

## Rules that still hold

- Never print, log or commit a key. Keys go only where the harness keeps them: OpenCode
  `$HOME/.local/share/opencode/auth.json` (`{"vercel": {"type": "api", "key": "..."}}`, mode
  0600), OpenClaw `$HOME/.openclaw/.env` (`AI_GATEWAY_API_KEY=...`). Write them from the
  environment with a script, never with the key in argv. Check a key by its HTTP status, never by
  echoing it.
- Gateway: free models only. Check `https://ai-gateway.vercel.sh/v1/models` first. A 429 is a rate
  limit: wait and retry. DeepSeek: the cheapest chat model; `deepseek-chat` is retired, use
  `deepseek-v4-flash` (OpenCode `deepseek/deepseek-v4-flash`, OpenClaw `deepseek/deepseek-v4-flash`).
- **OpenCode's title model is paid by default** (`google/gemini-nano-banana-2.1`). Set
  `"small_model": "vercel/poolside/laguna-s-2.1-free"` in `opencode.json` before any Gateway run.
- Do not edit `crates/goalcup-protocol/providers.toml` (owned by the Vercel Gateway session).
- Never write into a real `~/.claude`, `~/.codex`, `~/.hermes` or app-wide config. Even
  `opencode --version` creates dirs in `HOME`.
- Never commit traces, `spikes/*/out/`, `harnesses/` or anything with a key. Delete temp homes and
  run folders at the end.
- Branch `opencode-openclaw`; push with `git push -u origin opencode-openclaw`. The PR stays draft.

## Environment checks to do first

```bash
for v in VERCEL_TOKEN VERCEL_TEAM_ID VERCEL_PROJECT_ID AI_GATEWAY_API_KEY DEEPSEEK_API_KEY; do printenv "$v" >/dev/null && echo "$v set, $(printenv "$v" | wc -c) bytes" || echo "$v missing"; done
```

```bash
curl -s -o /dev/null -w '%{http_code}\n' https://ai-gateway.vercel.sh/v1/chat/completions -H "Authorization: Bearer $AI_GATEWAY_API_KEY" -H 'content-type: application/json' -d '{"model":"poolside/laguna-s-2.1-free","messages":[{"role":"user","content":"Reply PONG"}],"max_tokens":10}'
```

```bash
curl -s -o /dev/null -w '%{http_code}\n' https://api.deepseek.com/chat/completions -H "Authorization: Bearer $DEEPSEEK_API_KEY" -H 'content-type: application/json' -d '{"model":"deepseek-v4-flash","messages":[{"role":"user","content":"Reply PONG"}],"max_tokens":10}'
```

Expect 200 for both. The first session saw a 1-byte Gateway key and no DeepSeek or Vercel
variables: the environment is read when a session starts, so keys added later need a new session.

## Sandbox mechanics (Sandbox CLI 4.10.0, measured from `--help` only)

- Install: `npm i -g sandbox@latest`. The CLI reads its token from `VERCEL_AUTH_TOKEN` or
  `--token`; it does not read `VERCEL_TOKEN`. Prefer `export VERCEL_AUTH_TOKEN="$VERCEL_TOKEN"`
  over `--token` in argv. Team and project: `--scope "$VERCEL_TEAM_ID" --project "$VERCEL_PROJECT_ID"`
  (ids, not secrets).
- Create: `sandbox create --name gc-connectors --vcpus 4 --timeout 40m --scope ... --project ...`.
- `--env KEY=VALUE` on `create` and `exec` puts the value in argv. Safer: write a 0600 file from
  the environment locally (a script, never echo), `sandbox cp` it in, have the in-sandbox setup
  script read it into each harness's key file, then delete both copies.
- Run commands: `sandbox exec gc-connectors -- <cmd>` (`--workdir`, `--sudo`, `--timeout`). Copy:
  `sandbox cp <src> gc-connectors:<dst>`. Interactive: `sandbox ssh gc-connectors` or
  `sandbox exec -i`.
- End: `sandbox stop gc-connectors`, then `sandbox rm gc-connectors`.
- Check before creating that the image has `curl`, `git`, a C compiler (only if you must build),
  Node 26 for OpenClaw (`nvm install 26` or the installer's own Node), and an ext4 or XFS `HOME`
  (`df -T "$HOME"`).

**Blind spot: chat mode needs a terminal.** `goalcup start` opens the harness's TUI, and the
participant types. An agent cannot type into `sandbox exec`. Plan: install `tmux` in the sandbox,
start `goalcup start --harness <id> --dev` inside a detached tmux session, use
`tmux send-keys` to type the prompt and Enter, wait about 3 minutes, then exit the TUI (OpenCode:
`/exit` or Ctrl-C twice; OpenClaw: `/exit` or Ctrl-C, unmeasured), and read the output with
`tmux capture-pane`. If the TUI cannot be driven, `goalcup seal` from another tmux window ends the
run.

## Per-harness setup inside the sandbox

goalcup starts the harness with an empty environment and its own `HOME`. With
`run_home = "shared"` that home is `<GOALCUP_CONFIG_DIR>/capture-homes/<id>.run-home/`. Run
`goalcup setup --harness <id> --dev` first: it creates the home and opens the harness once.
Put the key and settings into that home (`HOME=<that path>` for the harness's own commands), as a
participant would.

OpenCode (`opencode-cli`):

- Config `<home>/.config/opencode/opencode.json`:
  `{"model": "vercel/poolside/laguna-s-2.1-free", "small_model": "vercel/poolside/laguna-s-2.1-free", "permission": "allow", "autoupdate": false, "share": "disabled"}`.
- Key `<home>/.local/share/opencode/auth.json` as above. For DeepSeek add
  `"deepseek": {"type": "api", "key": "..."}` and use model `deepseek/deepseek-v4-flash`.
- First use downloads the provider package from `registry.npmjs.org` and the catalog from
  `models.opencode.ai`; both are tunnels in the proof connector, which is expected.

OpenClaw (`openclaw-cli`):

- `HOME=<home> openclaw plugins install @openclaw/vercel-ai-gateway-provider` (and
  `@openclaw/deepseek-provider`).
- `HOME=<home> openclaw config set agents.defaults.model.primary vercel-ai-gateway/poolside/laguna-s-2.1-free`.
- `HOME=<home> openclaw config set logging.file "<home>/.openclaw/logs/openclaw.log"`. Without it
  the log goes to `/tmp/openclaw/` and the native row finds nothing.
- Key `<home>/.openclaw/.env`: `AI_GATEWAY_API_KEY=...` (and `DEEPSEEK_API_KEY=...`), mode 0600.
- Do **not** set `proxy.proxyUrl` or `OPENCLAW_PROXY_URL`: they override the recorder.
- In the first session, switching the model to `deepseek/deepseek-v4-flash` still called the
  Gateway. Check `openclaw models list` or `openclaw config get agents.defaults.model` if it happens.

## What to record per run

- Harness version (`opencode --version`, `openclaw --version`) and goalcup version.
- The `proxy admission:` line, the `model service:` or host lines, the list of tunnelled hosts.
- The `verify:` command's output: `goalcup-verify --capture-integrity <trace>` exit code and its
  summary line; intercepted count above 0; the model host (`ai-gateway.vercel.sh` or
  `api.deepseek.com`) not among the tunnels.
- PASS or FAIL against the pass rule. Write it into the results file's "Runs" table. Never paste a
  transcript.

## Proof connectors

Not shipped and not pinned: `--dev` only. Both load under `Origin::Dev` with the repository's
schema code (checked with a throwaway loader calling
`goalcup_protocol::harness::load_discovered_dir`, because `goalcup setup` hides load errors).
They use `[[capture.hosts]]` rows, not `model_host_from`, because neither harness writes its
model URL to a file (row 15). The native row is each harness's own log, a metadata leg without a
transcript. Create them in a folder of their own, for example `$HOME/my-connector/<id>/harness.toml`.

`opencode-cli/harness.toml`:

```toml
# OpenCode 1.19.4, PROOF ONLY (--dev, not shipped). Sessions live in SQLite (opencode.db), which
# schema 1 cannot capture; the native row is OpenCode's own append-only log, a metadata leg like
# amp-cli's. Facts: docs/harnesses/opencode-cli.md.
schema_version = 1
id = "opencode-cli"

[participant]
display_name = "OpenCode"

[capture]
interactive = true
continuation = "chat: the participant types the prompt and may type keep going"
args = []
resume_args = ["--continue"]

[capture.executable]
program = "opencode"
version_args = ["--version"]

[capture.env]
SHELL = "/bin/zsh"
OPENCODE_DISABLE_AUTOUPDATE = "1"

[capture.platform.linux-x86_64.env]
SHELL = "/bin/bash"

[capture.platform.linux-aarch64.env]
SHELL = "/bin/bash"

[capture.home]
run_home = "shared"
config_home = "per_run"
real_dirs = [
  "{user_home}/.config/opencode",
  "{user_home}/.local/share/opencode",
  "{user_home}/.cache/opencode",
]

[capture.ca]
env = ["NODE_EXTRA_CA_CERTS"]
mode = "extend"

[capture.identity]
source = "none"

[[capture.native]]
name = "log-no-transcript"
root = "{run_home}/.local/share/opencode/log"
pattern = "*.log"
form = "append_only"
since_run_start = true

[capture.termination]
strategy = "process_group"
grace_seconds = 10

[[capture.hosts]]
host = "ai-gateway.vercel.sh"
prefix_paths = ["/v3/ai/"]

[[capture.hosts]]
host = "api.deepseek.com"
evidence_paths = ["/chat/completions", "/v1/chat/completions"]
```

`openclaw-cli/harness.toml`:

```toml
# OpenClaw 2026.10.8, PROOF ONLY (--dev, not shipped). Transcripts live in SQLite
# (agents/<id>/agent/openclaw-agent.sqlite), which schema 1 cannot capture. The native row is
# OpenClaw's own log, which the participant first moves into the home once:
#   openclaw config set logging.file "$HOME/.openclaw/logs/openclaw.log"
# `openclaw` is a Node script, so only --dev accepts it. Facts: docs/harnesses/openclaw-cli.md.
schema_version = 1
id = "openclaw-cli"

[participant]
display_name = "OpenClaw"

[capture]
interactive = true
continuation = "chat: the participant types the prompt and may type keep going"
args = ["chat"]

[capture.executable]
program = "openclaw"
version_args = ["--version"]

[capture.env]
SHELL = "/bin/zsh"
NODE_USE_ENV_PROXY = "1"
OPENCLAW_NO_AUTO_UPDATE = "1"

[capture.platform.linux-x86_64.env]
SHELL = "/bin/bash"

[capture.platform.linux-aarch64.env]
SHELL = "/bin/bash"

[capture.home]
run_home = "shared"
config_home = "per_run"
real_dirs = ["{user_home}/.openclaw"]

[capture.ca]
env = ["NODE_EXTRA_CA_CERTS"]
mode = "extend"

[capture.identity]
source = "none"

[[capture.native]]
name = "log-no-transcript"
root = "{run_home}/.openclaw/logs"
pattern = "*.log"
form = "append_only"
since_run_start = true

[capture.termination]
strategy = "process_group"
grace_seconds = 10

[[capture.hosts]]
host = "ai-gateway.vercel.sh"
evidence_paths = ["/v1/messages"]

[[capture.hosts]]
host = "api.deepseek.com"
evidence_paths = ["/chat/completions", "/v1/chat/completions", "/anthropic/v1/messages"]
```

Open questions to settle by measurement: does a goalcup chat-mode launch of `openclaw chat` (that
is `tui --local`) record; does OpenCode's TUI write `opencode.log` lines during a run (the native
row needs a file changed since the run started); does the DeepSeek plugin of each harness call the
paths listed.

## Done

- Results file updated with each run's PASS or FAIL, verifier line and `proxy admission:` line.
- Facts sheets: replace "unmeasured" where a run measured the fact; keep `Status: facts only`
  (the SQLite store still blocks a shipped connector).
- Sandbox `gc-connectors` stopped and removed; temp homes deleted.
- Committed, pushed to `opencode-openclaw`, draft PR body updated (five lines).
