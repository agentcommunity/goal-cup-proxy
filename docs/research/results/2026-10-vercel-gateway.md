# Vercel AI Gateway and Vercel Sandbox, 28 Oct 2026 UTC (results)

Two rounds on the same day. Round 1 tried a Claude Code cloud container and was blocked. Round 2
ran in a Vercel Sandbox (`gc-gateway`, 4 vCPU, 40-minute timeout, region `iad1`) with goalcup
0.7.0 from the release `install.sh`. Desk research: [VERCEL-COMPAT](../VERCEL-COMPAT.md). All runs
were local runs with a fresh `GOALCUP_CONFIG_DIR` and no account, so nothing was uploaded. The
traces, config homes and sandbox are deleted. No key or token is in this file. The AI Gateway key
reached the sandbox as a copied file (mode 0600), never in argv or in output.

**Verdict.**

1. **Vercel Sandbox can host a short goalcup run.** XFS, user namespaces, IPv6 on loopback and
   direct egress all pass, with no TLS re-termination. goalcup's upstream TLS check passed against
   `ai-gateway.vercel.sh`.
2. **Hermes through the gateway is recorded.** With Hermes's built-in `ai-gateway` provider it
   PASSES: 13 wire model requests, decoded as `openai-chat-completions`. With a bare
   `provider: custom` endpoint the calls are recorded but fail with HTTP 401, because Hermes sends
   no key.
3. **Claude Code through the gateway is not recorded on the wire**, as expected. The native session
   is recorded, but the verifier reports `native_activity_without_wire_streams` with 0 wire model
   requests. The fix is D74 ([Decisions](#decisions-owner-28-oct)), not a `vercel` row; measured
   after D74 it is recorded ([Round 3](#round-3-28-oct-one-capture-system-sandbox)).
4. **`intercepted=` above 0 is not proof of a model record.** The Claude run shows `intercepted=20`
   and still has no wire stream. Judge with the research JSON's `cross_leg.wire_model_requests`.

## Decisions (owner, 28 Oct)

The `vercel` row below was not taken. The owner chose one capture rule set for every harness,
recorded as [D74](../../DECISIONS.md#d74-the-model-host-is-the-connectors-default-or-the-one-address-the-harnesss-own-config-names): the model hosts are the connector's defaults and the one address
the harness's own config names. Claude Code reads `env.ANTHROPIC_BASE_URL` in `settings.json` and
Codex reads `model_providers.*.base_url` via `model_provider`, both falling back to their default
hosts; `providers.toml` is frozen; every sealed connection is listed and uploaded with the seal;
the verifier labels each run's evidence. The work is the
[spec](../../plans/2026-10-28-one-capture-system-spec.md) and the
[plan](../../plans/2026-10-28-one-capture-system.md).

## Round 3: 28 Oct, one capture system (sandbox)

Measured on the integrated build of the one capture system (Tasks 1 to 7), in a Vercel Sandbox
(8 vCPU, `iad1`, non-persistent), as user `ubuntu`. Local runs: no `goalcup login`, so each run
was `goalcup start --harness <id>` with anchor `local only`, and nothing was uploaded. Prompt
"Build a chess game in Python.", about 3 minutes, then exit. Harnesses: Claude Code 2.1.312 (npm),
Codex 0.165.0 (release binary), Hermes v0.22.5. Gateway key only, copied as a 0600 file; key files
and settings were deleted and the sandboxes removed after the runs.

**Model.** Every run used the free `nvidia/nemotron-3.5-lightning-free` for every model alias. The
team has no paid gateway credits, so `deepseek/deepseek-v4.1-flash` answers 403 ("Free tier users
do not have access to this model"). 429s were seen in all three runs, and the harnesses retried.

| Run | `analysis.evidence` | `wire_model_requests` | `cross_leg` state | Integrity |
|---|---|---|---|---|
| Claude Code, `env.ANTHROPIC_BASE_URL` in `settings.json`, first run | `wire_undecoded` | 0 (19 calls recorded) | `unchecked` | valid, exit 0 |
| Claude Code, rerun with the decoder fix (#286) | `wire_and_native` | 15 | `consistent` | valid, exit 0 |
| Codex, provider `vercel`, key in `$CODEX_HOME/.env` | `wire_and_native` | 17 | `consistent` | valid, exit 0 |
| Hermes, built-in `ai-gateway` provider | `wire_only` | 9 | `unchecked` (no session file) | valid, exit 0 |

The first Claude Code run recorded 19 `POST ai-gateway.vercel.sh /claude-code/v1/messages`, but
the Anthropic decoder accepted only the exact path `/v1/messages`, so it labelled them
`wire_undecoded`. That is the bug the label was built to show. The fix accepts a hinted path
ending in `/messages` (ledger L29). Codex posted to `/codex/v1/responses` over plain HTTPS, no
WebSocket.

**Sealed hosts.** The gateway is in no sealed-hosts line, so it was opened and recorded:

- Claude Code (rerun): `sealed hosts: downloads.claude.ai:443 x2 7991 KiB, raw.githubusercontent.com:443 x1 369 KiB`
- Codex: `sealed hosts: github.com:443 x2 24992 KiB, ab.chatgpt.com:443 x1 1031 KiB, raw.githubusercontent.com:443 x1 9 KiB`
- Hermes: `sealed hosts: models.dev:443 x1 489 KiB, hermes-agent.nousresearch.com:443 x2 18 KiB, portal.nousresearch.com:443 x1 9 KiB, api.github.com:443 x2 0 KiB, github.com:443 x1 0 KiB (+2 more, list in tunnels-<entry_id>.jsonl)`

Each tunnel file was mode 0600, tagged `goalcup-tunnels-v1`, with `dropped 0`; the Claude Code
and Codex rows were all `not_model_host`. Hermes's 7 cut tunnels have null bytes, shown as "0 KiB" in this build (later
"? KiB"). Tunnel times are whole seconds.

**Refusal.** Claude Code with `ANTHROPIC_BASE_URL=http://localhost:4000` was refused at start,
exit 1, nothing created: "goalcup: Claude Code is set to use http://localhost:4000, which Goal Cup
cannot record: it is plain http, which goes around the recorder (set by `env.ANTHROPIC_BASE_URL`
in settings.json). Goal Cup records only an https model service that is not on this computer. Run
goalcup setup --harness claude-code-cli, choose such a service there, then run the command again."

**Also seen.** A capture home made by hand with umask 022 is refused: it must be mode 0700. The
finish output's `trace:` line gives the trace path; the tunnel list is at
`<GOALCUP_CONFIG_DIR>/runs/<entry_id>/tunnels-<entry_id>.jsonl`.

**Left for the laptop.** Subscription logins were not tried here: on a datacenter address they put
the account at risk. Claude Code with a subscription and no base URL, Codex with a ChatGPT login,
and Codex with an OpenAI API key: [the laptop guide](../LAPTOP-LOGIN-RUNS.md).

## Round 2: Vercel Sandbox facts

| Check | Result |
|---|---|
| `uname -a` | `Linux 7eab51a4-523 6.18.49 #1 SMP Thu Oct 1 14:49:36 UTC 2026 x86_64 GNU/Linux` |
| OS | Ubuntu 26.04.1 LTS; user `ubuntu` (uid 1000, in `sudo`); `HOME=/vercel`, not `/home/ubuntu` |
| `nproc` / memory | 4 / 8 GiB |
| `df -T "$HOME" /tmp` | both `/dev/vdb xfs`, 64 GB, 63 GB free |
| `findmnt -no FSTYPE -T "$HOME"` | `xfs` (goalcup accepts it) |
| `unshare -Ur true` | `userns-ok` |
| IPv6 on loopback | yes: `::1` on `lo`, bind works |
| DNS and egress | `ai-gateway.vercel.sh` resolves directly; no proxy |
| Tools present | `tmux`, `git`, `python3`, `curl`, `node`, `npm`, `sudo`; no `systemd-inhibit` |
| goalcup | 0.7.0, from `releases/latest/download/install.sh`, into `~/.local/bin` |
| Hermes | v0.22.5 (2026.9.24), Hermes's `install.sh --non-interactive --skip-browser --skip-computer-use` |
| Claude Code | 2.1.312, native `claude.ai/install.sh` |
| `goalcup doctor --harness hermes-cli` | PASS: manifest pin, executable, decoders |

Sandbox findings for VERCEL-COMPAT:

- **Filesystem:** XFS, so blocker 1 is cleared. Drives were not tested.
- **A stop takes a snapshot by default.** `sandbox stop` wrote a 1.9 GiB snapshot that expires in
  30 days. A key left on disk would survive the sandbox in that snapshot. Delete keys before a
  stop, or create the sandbox with `--non-persistent`. I deleted the snapshot.
- **`sandbox exec` is not a terminal.** `goalcup start` refuses a pipe ("goalcup must hold the
  foreground of a terminal"). Run goalcup inside `tmux` in the sandbox; this worked.
- Session stop during a run (blocker 2) and the 24-hour limit were not tested: this was a light
  proof, as asked.

## Round 2: runs

Prompt each time: `Build a chess game in Python.`, then about 3 minutes, then exit the harness.
Model `poolside/laguna-s-2.1-free`. Its free tier allows 5 requests a minute per team, so both
harnesses hit HTTP 429 and retried by themselves. Hermes also saw "Provider temporarily
unavailable" (503). No run needed the fallback model.

| Run | Verifier | `proxy admission:` | Wire / native | Result |
|---|---|---|---|---|
| 4a-1 Hermes, `provider: custom`, `base_url` `https://ai-gateway.vercel.sh/v1`, key `OPENAI_API_KEY` in `.env` | `capture integrity valid (anchor local_only, exit 0, findings: none)` | `accepted=15 intercepted=2 tunneled_not_approved=12 auth_refused=1` | 2 intercepted calls, both answered HTTP 401 | **FAIL**: recorded, but the agent never got a model reply |
| 4a-2 Hermes, `provider: ai-gateway`, same `base_url`, key `AI_GATEWAY_API_KEY` in `.env` | `capture integrity valid (anchor local_only, exit 0, findings: none)` | `accepted=20 intercepted=14 tunneled_not_approved=4 failed=2 client_tls=2` | `wire_model_requests=13`, `wire_streams=28`, 53 `openai-chat-completions` observations; 2 tool calls | **PASS** |
| 4b Claude Code, `settings.json` `env` `ANTHROPIC_BASE_URL=https://ai-gateway.vercel.sh/claude-code`, `ANTHROPIC_AUTH_TOKEN` | `capture integrity valid (anchor local_only, exit 0, findings: none)` | `accepted=26 intercepted=20 tunneled_not_approved=6` | `cross_leg.state=native_activity_without_wire_streams`, `wire_model_requests=0`, `native_model_activity=7`, 49 `claude-code-native` observations | **FAIL, as expected**: gateway traffic tunnelled, not recorded |

Notes:

- **4a-1, why the key is missing.** The key is good: the same key, sent by `curl` from the sandbox,
  got 503 (busy) on `/v1/chat/completions` and 200 on `/claude-code/v1/messages`, never 401. The gateway's 401 says no key arrived. The likely cause is
  D72's known limit: goalcup's `providers.recorder` trust entry for the same URL has no key, and
  with a bare `provider: custom` Hermes takes that entry as the URL's owner. That limit was marked
  "unverified"; it now fails in a measured run. The exact Hermes code path is **unverified**.
- **4a-2, the two `client_tls` failures** match Hermes's model-list and local-server probes
  measured before (`docs/harnesses/hermes-cli.md`, model host). They are not model calls
  (inference: the same counts, and every model call was recorded).
- **4b, what `intercepted=20` is.** The Claude connector approves one host, `api.anthropic.com`.
  The trace has 20 connection opens and closes and no `wire_request` record. So Claude Code still
  talks to `api.anthropic.com` for non-model calls, and goalcup intercepts those connections, but
  no path matches `/v1/messages`. The model calls went to the gateway (its configured base URL),
  which is not approved, so they are among `tunneled_not_approved=6`.
- **Tunnelled host names are not printed.** `proxy admission:` gives counts only, and
  `host.log` has nothing more. "The model host was not tunnelled" is shown by the wire records,
  not by that line.
- Claude Code warned `Model "poolside/laguna-s-2.1-free" isn't recognized · context assumed to be
  200k tokens`. Harmless here.

## Round 1: Claude Code cloud container (blocked)

Build `a732e00` from this checkout, `cargo build --release --offline`: PASS in 1 min 48 s. Facts:
Linux 6.18.44 x86_64, 4 CPUs, 15 GiB, ext4, `userns-ok`, **no IPv6**. Blocked because:

1. All outbound traffic goes through the session's egress proxy, which re-signs TLS with its own
   CA. `ai-gateway.vercel.sh` does not resolve without it. Going around it is not allowed.
2. goalcup's recording proxy resolves and connects to upstream hosts itself
   (`crates/goalcup-cli/src/proxy/connect.rs:950`, `:988`), with no upstream proxy setting. It
   trusts only `webpki-roots` (`crates/goalcup-cli/src/proxy/tls.rs:232`). Any machine whose egress
   re-signs TLS cannot host a run, by design. That includes a Vercel Sandbox with firewall
   credential brokering on the model host.
3. The gate needs IPv6 on loopback; that kernel has none.

## The change, as designed (not taken; see Decisions)

Run 4b is the measured "before" run the row needs. One problem with the row as first asked: a
`providers.toml` row is a `HostSpec` with **one `protocol_hint` for all its paths**
(`crates/goalcup-protocol/src/harness/schema.rs`; `start/proxy.rs::approved_hosts` copies it to
every path), and a connector may list a host once (`harness/check.rs:1169`). So one `vercel` row
cannot give `/v1/chat/completions`, `/v1/messages` and `/v1/responses` each their correct hint.

Proposed, for the Claude Code connector only:

```toml
# Vercel AI Gateway, the Anthropic Messages surface (measured with Claude Code,
# docs/research/results/2026-10-vercel-gateway.md). One hint per row: the gateway's OpenAI paths
# need their own row or per-path hints.
[vercel]
host = "ai-gateway.vercel.sh"
evidence_paths = ["/claude-code/v1/messages", "/v1/messages", "/coding-agent/v1/messages"]
protocol_hint = "anthropic-messages"
```

Then `providers = ["anthropic", "vercel"]` in `connectors/claude-code-cli/harness.toml`, `sh
scripts/pin-manifests.sh`, `sh scripts/gate.sh fast`, and a rerun of 4b that should show
`wire_model_requests` above 0. No DECISIONS entry is required for a new row (CONNECTOR-SCHEMA asks
only for a reviewed row from a measured run). But D62 says `providers.toml` is for a harness that
reaches one fixed service, and Claude Code through a gateway reaches two, so the owner may want a
line on that.

**Start-probe cost, and the choice.** With the row, every Claude Code start also makes one TCP
connect to `ai-gateway.vercel.sh:443` and refuses the start if it gets no answer within 10 s. That
includes participants on a plain Claude subscription. I would take that cost: it is one connect,
and the refusal names the host. The risk is a network that blocks Vercel. The alternative,
`model_host_from` on `settings.json` `env.ANTHROPIC_BASE_URL`, probes only what the participant
names, but D62 forbids a key that names `env` and has no optional form. That needs a DECISIONS
change, not a row.

**Owner questions:** should the gateway's OpenAI paths be covered at all? Hermes already records
the gateway through `model_host_from`, and Codex cannot use a row until its key problem is solved.
If yes, the clean fix is per-path hints in `HostSpec` (a schema change), not a second row.

## The Codex gap (not attempted)

1. **The key never arrives.** `env_key = "AI_GATEWAY_API_KEY"` makes Codex read the key from its
   environment. goalcup builds that environment from nothing (D36, D61), so Codex fails before any
   model call. A fix keeps the key in `config.toml` in Goal Cup's home (`experimental_bearer_token`,
   or a `[model_providers.vercel.auth]` command). Both are unmeasured with goalcup.
2. **The wrong host is recorded.** The connector intercepts only `chatgpt.com`
   (`/backend-api/codex/responses`). `ai-gateway.vercel.sh` tunnels.
3. **Recording would need** a row for `ai-gateway.vercel.sh` with `/codex/v1/responses`,
   `/v1/responses` and `/codex/v1/responses/compact` under `openai-responses`, or
   `model_host_from = { file = "config.toml", key = "model_providers.vercel.base_url" }`, which
   refuses every ChatGPT-login participant unless D62 gains an optional form. `/codex/v1/models`
   needs no capture. Leave `supports_websockets` off until measured.

## The server test for later (not run)

On a laptop or sandbox with direct egress, a Goal Cup account, Hermes and Claude Code installed as
native binaries first on `PATH`, and an AI Gateway key. `<home>` is
`~/Library/Application Support/goalcup` on macOS and `~/.config/goalcup` on Linux. Keep the key
only in the files under `<home>/capture-homes/` named below.

```sh
goalcup login
```

**4a, Hermes.** Use the built-in provider; a bare custom endpoint fails (4a-1). Run setup once,
exit Hermes, then edit `<home>/capture-homes/hermes-cli/config.yaml` so `model` reads
`provider: ai-gateway`, `base_url: https://ai-gateway.vercel.sh/v1`,
`default: poolside/laguna-s-2.1-free`, and put `AI_GATEWAY_API_KEY=<key>` in `.env` there (mode
0600). Repeat setup until every step says `[ok]`.

```sh
goalcup setup --harness hermes-cli
```

```sh
d=$(mktemp -d ~/goalcup-practice-XXXX) && cd "$d" && goalcup start --harness hermes-cli --practice
```

**4b, Claude Code.** Run setup once, exit, then write
`<home>/capture-homes/claude-code-cli/settings.json` (gateway key only, never a Claude
subscription). Pin every model alias to the free model so background calls do not use a paid one.

```sh
goalcup setup --harness claude-code-cli
```

```json
{
  "env": {
    "ANTHROPIC_BASE_URL": "https://ai-gateway.vercel.sh/claude-code",
    "ANTHROPIC_AUTH_TOKEN": "<your AI Gateway key>",
    "ANTHROPIC_API_KEY": "",
    "CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY": "1",
    "ANTHROPIC_MODEL": "poolside/laguna-s-2.1-free",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL": "poolside/laguna-s-2.1-free",
    "ANTHROPIC_DEFAULT_SONNET_MODEL": "poolside/laguna-s-2.1-free",
    "ANTHROPIC_DEFAULT_OPUS_MODEL": "poolside/laguna-s-2.1-free",
    "CLAUDE_CODE_SUBAGENT_MODEL": "poolside/laguna-s-2.1-free"
  },
  "permissions": { "defaultMode": "bypassPermissions" },
  "skipDangerousModePermissionPrompt": true
}
```

```sh
d=$(mktemp -d ~/goalcup-practice-XXXX) && cd "$d" && goalcup start --harness claude-code-cli --practice
```

For each: type `Build a chess game in Python.`, wait about 3 minutes, exit the harness, then run
the printed `verify:` command and the research JSON:

```sh
goalcup-verify --research-json <trace-dir> --output "$HOME/research.ndjson"
```

Pass: `capture integrity valid`, and the last line's `analysis.cross_leg.wire_model_requests` is
above 0. Today 4a passes and 4b should not, until D74 lands.

## Follow-ups

- Done in Round 3: under D74 (no `vercel` row), Claude Code shows `wire_model_requests` 15.
- Hermes: a bare `provider: custom` endpoint gets HTTP 401 under goalcup (4a-1). Say so in
  HARNESS-SETUP's Hermes section, and point Vercel users to the built-in `ai-gateway` provider.
- Done by D74: the `sealed hosts:` finish line and the uploaded tunnel list name each tunnelled
  host.
- Owner: decided by D74. Neither per-path hints in `HostSpec` nor a `vercel` row.
