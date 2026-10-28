# OpenCode and OpenClaw under Goal Cup, 28 Oct 2026 UTC (results)

Can Goal Cup Proxy record OpenCode and OpenClaw by the repository's own connector template? Measured
in a Claude Code cloud container (Linux x86_64, kernel 6.18.44, ext4 `HOME` and `/tmp`, user
namespaces available). Every probe used a private `HOME`. No key, token or trace is in this file.

**Verdict: both are `facts only`.** Both harnesses now keep their sessions only in SQLite, which
the guide lists under "Cannot be captured" (stop at step 0). No connector was added and no pin
changed. Transport looks good for both: each honours `HTTPS_PROXY` and `NODE_EXTRA_CA_CERTS`, and
each reads its key from its own home file with no environment variable. No real model reply and no
Goal Cup run were possible here (see "What blocked the runs"). Facts sheets:
[opencode-cli](../../harnesses/opencode-cli.md), [openclaw-cli](../../harnesses/openclaw-cli.md).
Next steps for a session that has the keys: [handover](2026-10-opencode-openclaw-handover.md).

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

| Run | Result |
|---|---|
| OpenCode, Gateway `poolside/laguna-s-2.1-free` | not run: no valid key (Gateway answered 401 to the container's key) |
| OpenCode, DeepSeek | not run: `DEEPSEEK_API_KEY` not set |
| OpenClaw, Gateway | not run: as above |
| OpenClaw, DeepSeek | not run: as above |
| Goal Cup local run (any) | not run: this container cannot host one (all egress goes through a re-signing proxy that goalcup's recorder does not use; no IPv6 for the gate) |
| Vercel Sandbox runs | not run: `VERCEL_TOKEN`, `VERCEL_TEAM_ID`, `VERCEL_PROJECT_ID` not set in this session |
| `scripts/check-connector.sh` | not applicable: no connector |
| `sh scripts/gate.sh fast` | not run (instructed: the container cannot host the gate); docs-only change |

No verifier line, `proxy admission:` line or tunnelled-host list exists for either harness yet.

## What blocked the runs

The session's environment was read at start. Keys added to the environment afterwards did not
reach it: `AI_GATEWAY_API_KEY` held a one-character value, and the DeepSeek and Vercel variables
were absent. A new session picks up the current values.

## Proof connectors (not shipped)

Two unpinned connectors for a light `--dev` proof were written and load under `Origin::Dev` with
the repository's own schema code. They record the wire and use each harness's own append-only log
as the native row (a metadata leg, as amp-cli does), not a transcript. They are in the
[handover](2026-10-opencode-openclaw-handover.md#proof-connectors), not under `connectors/`,
because the stop rule forbids a connector here.
