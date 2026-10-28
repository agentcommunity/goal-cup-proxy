# Vercel AI Gateway, cloud container, 28 Oct 2026 UTC (results)

A Claude Code cloud container, build `a732e00` (goalcup 0.7.0, built from this checkout). Desk
research for this round is [VERCEL-COMPAT](../VERCEL-COMPAT.md). No run was recorded, so no trace
exists. No key or token is in this file; the AI Gateway key was present in the environment and
was never printed or written.

**Verdict: blocked at step 2. This machine cannot host a goalcup run, so no harness was run and
no code was changed.** All outbound traffic here goes through the session's own egress proxy,
which re-signs TLS with a private CA. goalcup's recording proxy resolves and connects to upstream
hosts itself and trusts only the Mozilla roots, so it cannot reach `ai-gateway.vercel.sh` from
here. The `vercel` row is not added: `providers.toml` allows a row "only from a measured run",
and the run could not happen. The proposed row, the Codex gap and the laptop commands are below.

## Facts

| Check | Result |
|---|---|
| `uname -a` | `Linux vm 6.18.44-fc-v180 #1 SMP PREEMPT_DYNAMIC x86_64 GNU/Linux` |
| `nproc` | 4 |
| `free -g` | 15 GiB total, 15 free, no swap |
| `df -T "$HOME" /tmp` | both `/dev/vda ext4`, 31 GB free |
| `findmnt -no FSTYPE -T "$HOME"` | `ext4` |
| `unshare -Ur true` | `userns-ok` |
| `curl -sI https://ai-gateway.vercel.sh` | `HTTP/2 308` to `vercel.com/ai-gateway`, but only through the session proxy (`HTTPS_PROXY`) |
| `GET /v1/models` | both `poolside/laguna-s-2.1-free` and `nvidia/nemotron-3.5-lightning-free` listed, price 0, tag `tool-use` |
| Direct DNS (`getent hosts ai-gateway.vercel.sh`) | **unresolved** (`github.com` and `api.anthropic.com` resolve) |
| IPv6 | **none**: no `/proc/net/if_inet6`; `socket(AF_INET6)` fails with errno 97 |
| User | root (the gate and Claude Code both want a normal user) |
| Build | `cargo fetch --locked && cargo build --release --offline`: PASS, 1 m 48 s |

## Why the machine fails

1. **No direct path to the gateway.** The session's egress goes through `HTTPS_PROXY`, a local
   relay to a policy proxy that "re-terminates TLS" with a session CA (`/root/.ccr/README.md`).
   `ai-gateway.vercel.sh` does not resolve without it. Going around that proxy is not allowed in
   this session, and I did not try.
2. **goalcup cannot use that proxy.** The recording proxy calls `lookup_host` and
   `TcpStream::connect` itself (`crates/goalcup-cli/src/proxy/connect.rs:950`, `:988`). It has no
   upstream proxy setting. It checks upstream TLS only against `webpki-roots`
   (`crates/goalcup-cli/src/proxy/tls.rs:232`), so it would refuse the session CA anyway. That is
   the same trap VERCEL-COMPAT section 1 names for Sandbox firewall brokering.
3. **Effect on each run.** For 4a, the start preflight (`proxy/preflight.rs`) probes the
   `model.base_url` host on port 443 and refuses the start. For 4b and 4c, the harness's CONNECT to
   the gateway would be tunnelled by goalcup, then fail at DNS. No result here would say anything
   about Vercel.
4. **The gate would not pass here either.** It needs IPv6 on loopback (README, "The gate"), and
   this kernel has no IPv6.

## Results

| Run | Result | Verifier | `proxy admission:` | Harness version |
|---|---|---|---|---|
| 4a Hermes, custom endpoint `/v1` | NOT RUN (blocked) | none | none | not installed |
| 4b Claude Code, `ANTHROPIC_BASE_URL=.../claude-code` | NOT RUN (blocked) | none | none | not installed |
| 4c Codex, Vercel's `config.toml` | NOT RUN (blocked) | none | none | not installed |
| 5 change + `gate.sh fast` + rerun 4b | NOT DONE | none | none | none |

Expected results, from the code and docs, to confirm on a laptop:

- **4a:** PASS. Hermes's connector reads `model.base_url` (D62), so `ai-gateway.vercel.sh` is
  intercepted with `/v1/chat/completions` under the `openai-chat-completions` hint.
- **4b:** native session recorded, `ai-gateway.vercel.sh` tunnelled (`tunneled_not_approved` above
  0, `intercepted=0` for model calls), no wire record.
- **4c:** the model call fails or is not recorded. See "The Codex gap".

## The change, as designed (not applied)

**One hint per row is a blocker for the row as asked.** A `providers.toml` row is a `HostSpec`
with one `protocol_hint` for all its paths (`crates/goalcup-protocol/src/harness/schema.rs`;
`start/proxy.rs::approved_hosts` copies the row's hint to every path). A connector may list a host
once (`harness/check.rs:1169`). So one `vercel` row cannot give `/v1/chat/completions`,
`/v1/messages` and `/v1/responses` each their correct hint. A mixed row would decode two of the
three formats with the wrong decoder. Only `model_host_from` has per-path hints today
(`crates/goalcup-cli/src/model_host.rs`, `WIRE_PATHS`).

What I would add, once a laptop run measures it:

```toml
# Vercel AI Gateway, the Anthropic Messages surface (measured with Claude Code, <results file>).
# One hint per row: the gateway's OpenAI paths (/v1/chat/completions, /v1/responses,
# /codex/v1/responses) need their own row or per-path hints.
[vercel]
host = "ai-gateway.vercel.sh"
evidence_paths = ["/claude-code/v1/messages", "/v1/messages", "/coding-agent/v1/messages"]
protocol_hint = "anthropic-messages"
```

and in `connectors/claude-code-cli/harness.toml`: `providers = ["anthropic", "vercel"]`, then
`sh scripts/pin-manifests.sh`. No DECISIONS entry is required for a new row: CONNECTOR-SCHEMA asks
only for "a reviewed row… added from a measured run", and asks for a DECISIONS entry only to retire
or tighten. Note D62 says `providers.toml` "stays for a harness that can reach one fixed service";
Claude Code with a gateway reaches two, so the owner may want a line on that.

**Start-probe cost, and the choice.** With the row, every Claude Code start also probes
`ai-gateway.vercel.sh:443` and refuses the start if it does not answer within 10 s, even for a
participant who uses only a Claude subscription. Measured cost of a reachable probe: none yet. I
would take that cost: it is one TCP connect, and the refusal message names the host. The risk is a
participant's network that blocks Vercel; measure that on the laptop run too. The other option is `model_host_from` on
`settings.json` `env.ANTHROPIC_BASE_URL`, which probes only what the participant names, but D62
forbids a key naming `env` and has no optional form. That is a DECISIONS change, not a row.

**Owner question first.** Should the OpenAI paths go in at all? Hermes already records the
gateway through `model_host_from`, and Codex cannot use a row until its key problem is solved. If
yes, the clean fix is per-path hints in `HostSpec` (a schema change), not a second `vercel-openai`
row, because a connector that needed both would list the host twice.

## The Codex gap (not attempted)

Vercel's Codex setup cannot work under goalcup as written:

1. **The key never arrives.** `env_key = "AI_GATEWAY_API_KEY"` makes Codex read the key from its
   environment. goalcup builds that environment from nothing (D36, D61), so the variable is absent
   and Codex fails before any model call. A fix must keep the key in `config.toml` in Goal Cup's
   home, for example `experimental_bearer_token` or a `[model_providers.vercel.auth]` command.
   Both are unmeasured with goalcup.
2. **The wrong host is recorded.** The connector intercepts only `chatgpt.com`
   (`/backend-api/codex/responses`). `ai-gateway.vercel.sh` tunnels, so even a working call leaves
   no wire record.
3. **What recording would need:** either a row for `ai-gateway.vercel.sh` with
   `/codex/v1/responses`, `/v1/responses` and `/codex/v1/responses/compact` under `openai-responses`
   (same probe cost; conflicts with a `vercel` Anthropic row in any connector that lists both), or
   `model_host_from = { file = "config.toml", key = "model_providers.vercel.base_url" }`, which
   refuses every ChatGPT-login participant unless D62 gains an optional form. `/codex/v1/models`
   returns a proprietary shape and needs no capture. Leave `supports_websockets` off until measured.

## Laptop commands for later (step 6, not run)

On a Mac or Linux laptop with direct internet, a Goal Cup account, Hermes and Claude Code installed
as native binaries first on `PATH`, and an AI Gateway key. `<home>` is
`~/Library/Application Support/goalcup` on macOS and `~/.config/goalcup` on Linux. Never paste the
key into a terminal history you share or into a file outside `<home>/capture-homes/`.

```sh
goalcup login
```

**4a, Hermes, practice.** In setup, choose a custom OpenAI-compatible endpoint, base URL
`https://ai-gateway.vercel.sh/v1`, your gateway key, model `poolside/laguna-s-2.1-free` (fall back
to `nvidia/nemotron-3.5-lightning-free` if it cannot call tools). Repeat until all steps say `[ok]`.

```sh
goalcup setup --harness hermes-cli
```

```sh
d=$(mktemp -d ~/goalcup-practice-XXXX) && cd "$d" && goalcup start --harness hermes-cli --practice
```

Type `Write a tested fizzbuzz in Python and commit it.`, let it finish, exit Hermes, then run the
printed `verify:` command. Pass: `integrity: valid`, `intercepted=` above 0, and
`ai-gateway.vercel.sh` not tunnelled.

**4b, Claude Code, practice.** Run setup once, then exit Claude Code and edit
`<home>/capture-homes/claude-code-cli/settings.json` (gateway key only, never a Claude
subscription). The `ANTHROPIC_MODEL` line is unverified with Claude Code's gateway model names; if
it is refused, pick the model with `/model`.

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
    "ANTHROPIC_MODEL": "poolside/laguna-s-2.1-free"
  },
  "permissions": { "defaultMode": "bypassPermissions" },
  "skipDangerousModePermissionPrompt": true
}
```

```sh
d=$(mktemp -d ~/goalcup-practice-XXXX) && cd "$d" && goalcup start --harness claude-code-cli --practice
```

Same prompt, then the printed `verify:` command. Before the row: native session recorded,
`tunneled_not_approved` above 0, no wire stream. Note `claude --version`, `hermes --version`,
the verifier output and the `proxy admission:` line in a new results file. That run is the
measurement the `vercel` row needs; then add the row, run `sh scripts/gate.sh fast`, and repeat
4b to see `intercepted=` count the gateway calls.

## Follow-ups

- Run step 6 on a laptop, then land the `vercel` row from that measured run.
- Owner: per-path hints in `HostSpec`, or Anthropic-only `vercel` row (section "The change").
- A goalcup run inside a TLS-re-terminating egress proxy (this container, Vercel Sandbox with
  brokering) is impossible by design. Say so in the participant guide's machine section.
