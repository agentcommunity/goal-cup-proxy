# OpenCode and OpenClaw under Goal Cup, 28 Oct 2026 UTC (results)

Can Goal Cup Proxy record OpenCode and OpenClaw by the repository's own connector template? Measured
in a Claude Code cloud container (Linux x86_64, kernel 6.18.44, ext4 `HOME` and `/tmp`, user
namespaces available). Every probe used a private `HOME`. No key, token or trace is in this file.

**Verdict: both are `facts only`, and both record under Goal Cup.** Both harnesses keep their
sessions only in SQLite, which the guide lists under "Cannot be captured" (stop at step 0). No
connector was added and no pin changed. The light proof passed for both: in a Vercel Sandbox,
with the unpinned proof connectors under `--dev`, Goal Cup intercepted every model call of
OpenCode and OpenClaw, through the Gateway and through DeepSeek, and every trace verified valid
(see "Runs"). Facts sheets: [opencode-cli](../../harnesses/opencode-cli.md),
[openclaw-cli](../../harnesses/openclaw-cli.md). The first session's notes are in the
[handover](2026-10-opencode-openclaw-handover.md).

## Versions and machine

| | Value |
|---|---|
| goalcup | built from `main` at `a732e00` (`cargo build --release --offline`, Rust 1.99.0) |
| OpenCode | 1.19.4, `curl -fsSL https://opencode.ai/install \| bash -s -- --no-modify-path`, native ELF |
| OpenClaw | 2026.10.8 (dc2523e), `openclaw.ai/install.sh --no-onboard --no-prompt`, npm package on Node 26.12.1 |
| Network | all egress through the container's TLS re-signing proxy; no direct DNS (`curl --noproxy '*'` fails) |
| Reachable | `ai-gateway.vercel.sh` (308 on `/`), `api.deepseek.com` (401 unauthenticated) |

## How the transport was tested

Two throwaway local proxies, outside the repository: a logging CONNECT relay to the container's
proxy, and a TLS-intercepting proxy with its own private test CA that logs the request line and
whether an auth header came (its length, never the value) and answers 401. Keys were invented
strings in each harness's own key file. Each case ran the harness's one-shot command
(`opencode run`, `openclaw agent --local`) from an empty environment (`env -i`).

| Case | OpenCode 1.19.4 | OpenClaw 2026.10.8 |
|---|---|---|
| `HTTPS_PROXY` + `NODE_EXTRA_CA_CERTS` = test CA | TLS ok, `POST /v3/ai/language-model`, `Authorization` sent | TLS ok, `POST /v1/messages`, `x-api-key` sent |
| `HTTPS_PROXY`, no CA variable | fails: `unable to verify the first certificate` | fails: `UNABLE_TO_VERIFY_LEAF_SIGNATURE` |
| `SSL_CERT_FILE` = test CA only | TLS ok | ignored, fails |
| key only in own home file, no env var | used (`auth.json`) | used (`.openclaw/.env`) |
| key file removed | provider not loaded (`Model not found`) | not tested |
| `OPENCLAW_PROXY_URL` (env or `.env`) / `proxy.proxyUrl` | n/a | used, and **wins over `HTTPS_PROXY`** |
| hosts from a fresh home | `ai-gateway.vercel.sh`, `models.opencode.ai`, `registry.npmjs.org` | `ai-gateway.vercel.sh`, npm for plugins |

## Docs vs reality

| Claim | Docs said | Measured |
|---|---|---|
| OpenCode session store | `opencode.db` SQLite (troubleshooting page) | same; no JSON files. The repo's old spike and D44 already expected this |
| OpenCode proxy and CA | `HTTPS_PROXY`, `NODE_EXTRA_CA_CERTS` | both work; `SSL_CERT_FILE` works too (Bun), not documented |
| OpenCode `--version` | n/a | creates empty config and data dirs in `HOME` |
| OpenCode small model | n/a | defaults to a paid Gateway model for titles; set `small_model` |
| OpenCode Gateway wire | "Vercel AI Gateway" provider | AI SDK gateway protocol `/v3/ai/language-model`, not OpenAI or Anthropic shape |
| OpenClaw install | Node 24.22.1+ or 26.12.1+ | Node 22 refused; installer asks for `nvm install 26` |
| OpenClaw transcripts | SQLite `openclaw-agent.sqlite` (JSONL retired) | same, table `transcript_events` |
| OpenClaw key file | `~/.openclaw/.env` recommended | works with no env var |
| OpenClaw proxy | `proxy.proxyUrl` beats `OPENCLAW_PROXY_URL`; env proxy honoured | both override `HTTPS_PROXY`; a recorder bypass Goal Cup does not check |
| OpenClaw CA | `NODE_EXTRA_CA_CERTS` for TLS inspection | same; `SSL_CERT_FILE` ignored |
| OpenClaw logs | `/tmp/openclaw/`, `logging.file` moves it | same |
| Vercel Gateway coding-agent pages | not fetched (vercel.com and docs.openclaw.ai were outside the container's DNS allowlist); read via the harnesses' own docs sources | |

## What blocks each harness

| Harness | Blocker | Guide row |
|---|---|---|
| OpenCode | Sessions only in SQLite | Cannot be captured: "A database session store"; needs a read-only snapshot native form (D44) |
| OpenCode | No model URL in its config by default (catalog-provided), so `model_host_from` has nothing to read | row 15 |
| OpenCode | Auth and database follow `XDG_DATA_HOME`; `OPENCODE_CONFIG_DIR` moves config only | rows 5, 6 |
| OpenClaw | Transcripts only in SQLite | Cannot be captured |
| OpenClaw | `openclaw` is a Node script | row 10 (interpreter); `--dev` only |
| OpenClaw | `proxy.proxyUrl` / `OPENCLAW_PROXY_URL` override the recorder's `HTTPS_PROXY` | isolation gap, section 8 of the sheet |
| OpenClaw | No model URL in its config by default | row 15 |
| OpenClaw | `--local` is on the RULES list; chat mode is `tui --local` | open question |

## Runs

Second session, 28 Oct 2026 UTC, in one Vercel Sandbox (`gc-connectors`, 4 vCPUs, Ubuntu
26.04.1, x86_64, XFS, Node 24.22.1, direct egress). goalcup 0.7.0 from the release `install.sh`
(the release binary accepted the proof connectors; no build needed). OpenCode 1.19.4 (the image
already had the npm `opencode-ai` package; the installer kept it). OpenClaw 2026.10.8 (dc2523e)
from its installer, on the image's Node 24.22.1. Each run: local, no account, `goalcup start
--harness <id> --dev` in tmux, prompt "Build a chess game in Python.", about 2 to 3 minutes,
exit, then the printed `verify:` command. Pass: integrity valid, intercepted above 0, model host
not tunnelled.

| Run | Entry | `proxy admission:` | Model calls recorded | Native row | Result |
|---|---|---|---|---|---|
| OpenCode, Gateway `nvidia/nemotron-3.5-lightning-free` | `e2a683d83e83a43e9` | `accepted=19 intercepted=1 tunneled_not_approved=18` | 27 `POST ai-gateway.vercel.sh/v3/ai/language-model`: 15 × 200, 12 × 429 | `opencode.log` | **PASS** |
| OpenClaw, Gateway `nvidia/nemotron-3.5-lightning-free` | `e2479e7457d0d0166` | `accepted=19 intercepted=19` | 17 `POST ai-gateway.vercel.sh/v1/messages`: 12 × 200, 5 × 429 | `openclaw.log` | **PASS** (stop not confirmed, below) |
| OpenCode, DeepSeek `deepseek/deepseek-v4-flash` | `ea5775bead446dc93` | `accepted=2 intercepted=1 tunneled_not_approved=1` | 2 `POST ai-gateway.vercel.sh/v3/ai/language-model`, both 403: OpenCode fell back to a paid Gateway model (below) | `opencode.log` | not a DeepSeek proof |
| OpenCode, DeepSeek `deepseek/deepseek-flash` | `ee0eae0f61b275c0e` | `accepted=1 intercepted=1` | 4 `POST api.deepseek.com/chat/completions`: 4 × 200 | `opencode.log` | **PASS** |
| OpenClaw, DeepSeek `deepseek/deepseek-flash` | `e55f4dd6ac0ff37fb` | `accepted=4 intercepted=4` | 2 `POST api.deepseek.com/chat/completions`: 2 × 200 | `openclaw.log` | **PASS** |

Every run's verifier line: `goalcup-verify: capture integrity valid (anchor local_only, exit 0,
findings: none)`. In no run was the model host tunnelled. `intercepted` counts connections, not
requests: OpenCode kept one connection open for all 27 Gateway calls. goalcup does not log the
names of tunnelled hosts; for OpenCode they are expected to be `models.opencode.ai` and
`registry.npmjs.org` (first session), not checked here.

What the runs showed:

- **Gateway free tier.** `poolside/laguna-s-2.1-free` answered 429 at once: 5 requests per minute
  per team, shared with the other session. The runs used the fallback model; both harnesses hit
  429s and retried. The 429 exchanges are recorded like any other.
- **OpenCode falls back to a paid model without saying so.** `deepseek/deepseek-v4-flash` is not
  in OpenCode's catalog, so the TUI opened on `google/gemini-3-pro-image` ("Nano Banana Pro")
  through the Gateway. The Gateway answered 403 to both calls; the run was stopped at once.
  DeepSeek's own `/models` now lists `deepseek-flash` and `deepseek-v4-pro` (`deepseek-v4-flash`
  still answers); both harnesses' catalogs match that. Check the model line in the TUI before
  typing.
- **OpenClaw rejects an unknown model.** `config set ... deepseek/deepseek-v4-flash` answered
  "Unknown model". After `openclaw models list --refresh --provider deepseek`,
  `deepseek/deepseek-flash` was accepted and called (not the Gateway).
- **Exiting.** OpenCode: Escape, then Ctrl-C twice, seals at once (`AlreadyStopped`). OpenClaw:
  while it waited on 429 retries ("finishing context"), Ctrl-C and `/exit` were refused ("wait for
  it to finish before aborting"); `goalcup seal --yes` from another window stopped it, but goalcup
  logged `harness termination: Unresolved ... (reason: ProcessAuthorityUnavailable)`. No OpenClaw
  process was left afterwards. In the DeepSeek run, Escape then `/exit` exited cleanly.
- **Setup without an account.** `goalcup setup` in a terminal starts the device login first and
  waits up to 15 minutes. To reach the harness-home step locally, a `server.json` naming a closed
  loopback port made the login fail at once; it was removed before `goalcup start`. `goalcup
  start` itself needs no login. A shared run home made by hand must be mode 0700, or start
  refuses it.
- **Open questions settled.** `openclaw chat` (`tui --local`) records under chat mode; OpenCode's
  TUI writes `opencode.log` during a run, so the native row finds it; both DeepSeek providers call
  `/chat/completions`.

Not run: `scripts/check-connector.sh` (no connector) and `sh scripts/gate.sh fast` (docs-only
change). Keys were deleted inside the sandbox before it was stopped and removed. `sandbox stop`
saved an automatic snapshot (30-day expiry) holding the run folders and traces but no key file;
deleting it is left to the owner.

## What blocked the first session

Its environment was read at start: `AI_GATEWAY_API_KEY` held one character, and the DeepSeek and
Vercel variables were absent. It also could not host a Goal Cup run (all egress through a
re-signing proxy, no IPv6 for the gate). The second session had all five variables and used a
Vercel Sandbox.

## Proof connectors (not shipped)

Two unpinned connectors for a light `--dev` proof were written and load under `Origin::Dev` with
the repository's own schema code. They record the wire and use each harness's own append-only log
as the native row (a metadata leg, as amp-cli does), not a transcript. They are in the
[handover](2026-10-opencode-openclaw-handover.md#proof-connectors), not under `connectors/`,
because the stop rule forbids a connector here.
