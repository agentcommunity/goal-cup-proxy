# Laptop runs with subscription logins (owner test guide)

28 Oct 2026. The [one capture system](../plans/2026-10-28-one-capture-system.md) was measured in a
Vercel Sandbox with a gateway key only ([ledger](../plans/2026-10-28-one-capture-system-ledger.md)
L27). A subscription login was not tried there: on a datacenter address it puts the account at
risk. These runs finish the job on the owner's own computer, on its normal network, no VPN, one
run per account.

Each run takes about 10 minutes. Record the result rows at the end in the results file
([2026-10-vercel-gateway.md](results/2026-10-vercel-gateway.md)) and ledger.

## 0. Build and set up once

The code is on `main` but not in a tagged release yet, so build it:

```bash
cd goal-cup-proxy && git pull && cargo build --release --locked
```

```bash
export PATH="$PWD/target/release:$PATH" && goalcup --version
```

`<home>` below is `~/Library/Application Support/goalcup` on macOS and `~/.config/goalcup` on
Linux.

```bash
goalcup login
```

After each run, set `TRACE` to the path on the finish output's `trace:` line (it is under
`<home>-artifacts/artifacts/<entry id>/trace`), then read its label:

```bash
goalcup-verify --capture-integrity "$TRACE"
```

```bash
goalcup-verify --research-json "$TRACE" --output /tmp/report.ndjson && tail -n 1 /tmp/report.ndjson | jq '{evidence: .analysis.evidence, cross_leg: .analysis.cross_leg}'
```

The finish output also has a `sealed hosts:` line; the full list is the `tunnels-<entry id>.jsonl`
file under `<home>/runs/`.

## 1. Claude Code with a Claude subscription, no gateway

Make sure `<home>/capture-homes/claude-code-cli/settings.json` has no `env.ANTHROPIC_BASE_URL`
(delete the key if a gateway test left it there).

```bash
goalcup setup --harness claude-code-cli
```

Sign in with the subscription when Claude Code asks, then exit.

```bash
d=$(mktemp -d ~/goalcup-practice-XXXX) && cd "$d" && goalcup start --harness claude-code-cli --practice
```

Type "Build a chess game in Python.", wait about 3 minutes, exit Claude Code.

Pass:
- the start is not refused;
- the run's `host.log` has a `model service:` line saying `env.ANTHROPIC_BASE_URL` is not set;
- integrity `valid`, `evidence` is `wire_and_native`, `wire_model_requests` above 0;
- `api.anthropic.com` is not in the `sealed hosts:` line (it was opened and recorded);
- `platform.claude.com`, if listed, has reason `sign_in` in the tunnels file.

## 2. Codex with a ChatGPT login, no gateway

Make sure `<home>/capture-homes/codex-cli/config.toml` has no `model_provider` line, or one that
names a built-in provider (`openai`).

```bash
goalcup setup --harness codex-cli
```

Sign in with ChatGPT, then exit.

```bash
d=$(mktemp -d ~/goalcup-practice-XXXX) && cd "$d" && goalcup start --harness codex-cli --practice
```

Same prompt, about 3 minutes, exit. Pass: the same as run 1, with `chatgpt.com` not in the
`sealed hosts:` line, and `auth.openai.com`, if listed, as `sign_in`.

## 3. Codex with an OpenAI API key (decides a future host row)

Sign Codex in with an API key instead, then run as in 2:

```bash
printenv OPENAI_API_KEY | CODEX_HOME="<home>/capture-homes/codex-cli" codex login --with-api-key
```

Record, do not judge: is `api.openai.com` in the `sealed hosts:` line? If it is, the API-key login
goes to a host the connector does not open, so the run labels `native_only`. That decides whether
the connector gains an `api.openai.com` row (spec "Out of scope").

## 4. Optional: the gateway recipes on the laptop

The sandbox measured these already (Codex `wire_and_native`, Claude Code after the decoder fix).
Repeat only if a sandbox result looks off. Recipes:
[HARNESS-SETUP](../HARNESS-SETUP.md) "Through a gateway". Use a gateway key, never a
subscription, through a gateway.

## What to write down

| Run | Harness version | Start refused? | evidence | wire_model_requests | sealed hosts line | Notes |
|---|---|---|---|---|---|---|
| 1 Claude subscription | | | | | | |
| 2 Codex ChatGPT | | | | | | |
| 3 Codex API key | | | | | | is `api.openai.com` listed? |

If a run fails, keep the trace (do not commit it) and the finish output, and note the exact
error line.
