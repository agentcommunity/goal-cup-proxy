# Your own agent loop

Connector [`homemade-cli`](../../connectors/homemade-cli.toml), harness id `homemade`. Status:
shipped connector. Tier: always "not verified", because the test pad has no build of your loop to
install.

This connector is for an agent loop you wrote yourself. A product someone else publishes, open
source or not, needs its own connector ([HARNESS-ONBOARDING](../HARNESS-ONBOARDING.md)), and that
connector counts for Edition 01 only if it is merged before the cup release tag (see
[RULES](../RULES.md#timeline)). The rules for a run with your loop, including its public source,
are in [RULES](../RULES.md#harness-support).

## What your loop must do

1. **Be on your `PATH` as `homemade-agent`, as its own executable.** Name its entry point
   `homemade-agent` (`[project.scripts]` in `pyproject.toml`, `bin` in `package.json`) and install it
   (`uv tool install -e .`, `npm install -g .`), or put this script on your `PATH` as `homemade-agent`:

   ```sh
   #!/bin/sh
   [ "$1" = --version ] && { echo 'alice-agent 0.3.1+abc1234 github.com/alice/agent'; exit 0; }
   exec <your loop> "$@"
   ```

   Never give an interpreter or launcher that name (`python3`, `node`, `npx`, `uvx`). goalcup starts
   it in your project folder, with no arguments at `goalcup start`.
2. **Be an interactive terminal program** that reads your first message from the keyboard. You type
   the message yourself; `--prompt` only puts it on the clipboard
   ([RULES](../RULES.md#one-prompt-then-only-keep-going)).
3. **Take a pasted message as one message,** several lines and blank lines included. Your first
   message is the push block, a blank line, then your prompt, pasted at once. Read a paste whole,
   with bracketed paste (prompt_toolkit has it) or by joining lines that arrive within about 200 ms
   of each other. A loop that reads one line per turn, such as a plain `input()` loop, splits it into
   many turns, and the record shows many messages where you sent one.
4. **Pick up the last conversation on `--continue`.** A `goalcup continue` after a part that recorded
   model requests or traces starts your loop with `--continue`. Its screen says the last session
   goes on, and from then on you may type only "keep going", also into a new conversation. So
   reload the last conversation then, for example from your traces in `GOALCUP_TRACE_DIR`. This is
   required for your run to survive a stop: a loop that starts fresh gets "keep going" with no goal
   and no push block.
5. **Answer `--version` at once.** Print one line on standard output, at most 64 characters, that
   names the loop, its version and the commit it runs, and links its public source, for example
   `alice-agent 0.3.1+abc1234 github.com/alice/agent`. Then exit 0 within 5 seconds. goalcup runs this
   with no proxy, in your project folder, at start, at continue and after the loop exits, so it must
   make no model call and write nothing. The record keeps that line for each part, and it is how your
   run names its harness: a run whose record lacks it is reviewed by hand
   ([RULES](../RULES.md#harness-support)). Do not change your loop during the run: a line that
   differs from the last part's is shown on the continue screen and kept in the record.
6. **Send every model call over `https` through `HTTPS_PROXY`,** and trust goalcup's certificate
   from `SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE` or `NODE_EXTRA_CA_CERTS`
   ([what goalcup sets](../HARNESS-SETUP.md#what-goalcup-sets-at-launch)). goalcup never sets
   `HTTP_PROXY`. To record a model host goalcup does not list, add it with
   `goalcup setup --record-host <host>`.
7. **Your loop may be built on any SDK,** including the Claude Agent SDK and the Codex SDK, which
   run Claude Code or Codex underneath: it is still your loop and one harness
   ([RULES](../RULES.md#harness-support)). Let the SDK's process inherit your environment, so it
   sees `HTTPS_PROXY` and the certificate. Both SDKs pass your environment on by default; in
   TypeScript their `env` option replaces it, so pass `{ ...process.env, ... }` there. Claude Code
   reads `NODE_EXTRA_CA_CERTS`, Codex `SSL_CERT_FILE`; unset `CLAUDE_CODE_CERT_STORE` and
   `CODEX_CA_CERTIFICATE` if you export them. A wrapper that only starts a product (`exec claude`)
   is that product, not your loop. Your loop's own subagents are allowed; starting a second harness
   beside your loop is not.

goalcup installs no skill for your loop. Put what the agent must know in your prompt, such as
`GOALCUP.md` and committing and pushing often.

## Traces

Your loop writes its turns and subagents as JSONL under `GOALCUP_TRACE_DIR`, a folder in the loop's
goalcup home: one JSON line per model call and per tool call, and each subagent's lines in a file of
their own or marked with the subagent's id and its parent's, for example:

```json
{"ts": "2026-12-05T10:04:11Z", "agent": "sub-2", "parent": "main", "type": "tool_call", "tool": "bash"}
```

The field names and the rest of the format are yours. Built on an SDK? Write each message or event
it streams as one JSON line: the Claude Agent SDK's assistant messages carry the session's id
(`session_id`) and, in a subagent, the id of the tool call that started it (`parent_tool_use_id`).
Every `*.jsonl` file below that folder is recorded as it grows, masked like any session file,
alongside the model calls; other file types there are not. A loop that writes none gets a
`goalcup status` line saying no traces file was found, and its review has only the model calls.
The folder is kept between parts, so `--continue` can reload from it. Never write a key or an
`Authorization` header into a trace: goalcup masks your exported key variables, the `.env` files in
your project folder and known key shapes, not a key your loop reads from its own config.

## Runtimes that miss the proxy or the certificate

Read 26 Nov 2026. A call that ignores `HTTPS_PROXY` is not recorded, and no gap marks it.
`goalcup status` warns only when no model request at all was written.

- **Node:** `fetch` uses `HTTPS_PROXY` (goalcup sets `NODE_USE_ENV_PROXY=1`) from Node 22.21 or
  24.0, and `node:http` and `node:https` from 22.21 or 24.5, unless you or your SDK pass your own
  agent or dispatcher (openai-node 4.x does: use 5 or later)
  ([Node docs](https://nodejs.org/en/learn/http/enterprise-network-configuration)). axios before
  1.16.1 gets `405` on every call, because goalcup's proxy speaks only `CONNECT`
  ([axios 1.16.1](https://github.com/axios/axios/releases/tag/v1.16.1)). Upgrade axios or use `fetch`.
- **Python:** `requests` and `httpx` work as they are. `aiohttp` needs `trust_env=True`.
- **Deno:** reads `NODE_EXTRA_CA_CERTS` from 2.8
  ([Deno docs](https://docs.deno.com/runtime/reference/env_variables/)). On older versions, set
  `DENO_CERT="$SSL_CERT_FILE"` in your script.
- **Go:** on macOS, a binary reads `SSL_CERT_FILE` only when it was built with Go 1.27 or later and
  its `go.mod` says 1.27 or later.
- **Rust:** a client with compiled-in roots (`webpki-roots`) or the macOS platform verifier ignores
  these variables. Add the certificates from `SSL_CERT_FILE` to the client yourself.
- **Java:** the JVM ignores `HTTPS_PROXY`, so Java loops are not supported.

## Try it

Run `goalcup setup --harness homemade --yes`, then `goalcup start --practice` in an empty folder.
`--yes` skips the sign-in step, which would open your loop once with no recording; there is nothing
to sign in to. Type a message. After five minutes of work, run `goalcup status` in another
terminal: it counts model requests and does not say that no traces file was found. To check a
paste, put two short paragraphs in a file outside the folder, run
`goalcup start --practice --prompt <that file>` and paste: your loop must answer once.
