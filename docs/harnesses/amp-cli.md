# Amp harness facts

Connector id: `amp-cli` · Facts for version: `0.0.1793275344-g17bf52` · Written by: Goal Cup team · Last checked: 2026-10-31
Public disclosure: yes. Facts come from Amp's public docs, strings and help text of the shipped npm package, and live runs on macOS arm64 and Linux in October 2026. No keys, tokens, account names or personal paths are included.
Status: shipped connector ([`connectors/amp-cli.toml`](../../connectors/amp-cli.toml))

Source words: "docs" is [ampcode.com/docs](https://ampcode.com/docs/cli) as read on 2026-10-31; "help" is `amp --help` output; "live" is a logged-in run on `0.0.1791676897-g77c125` or `0.0.1792209635-g24a9fe`, older than the version above; "unknown" is not measured or read.

## 1. Run one prompt, in chat mode first

- `amp` with no argument opens the TUI ([docs](https://ampcode.com/docs/cli)). Two Ctrl-C presses quit with exit 0 (live).
- Reopen the last thread: `amp last` (help; the docs page does not list it). The connector's `resume` is `["last"]`.
- With no stored sign-in, `amp` asks "Would you like to log in to Amp?" before the TUI. The connector's `first_screen` line tells the participant to sign in then.
- Execute mode (`amp -x "<prompt>"`) also starts when stdout is not a terminal; it is not chat mode and is not used.

## 2. Install and record a version

- `curl -fsSL https://ampcode.com/install.sh | bash`, or `npm install -g @ampcode/cli@<version>`; the binary is a Bun single-file executable ([docs](https://ampcode.com/docs/cli); package metadata).
- `amp --version` prints the version first, then the release date. The connector sets no `version` word, so any build runs and the `start` event records the one that ran.
- About 7 builds a day on npm (registry, 29 Oct 2026). Amp installs updates in the background unless told not to (section 9).

## 3. Where it keeps things, per OS

| | macOS | Linux | Windows |
|---|---|---|---|
| Config dir | `$HOME/.config/amp/settings.json` or `.jsonc`; workspace `.amp/settings.json` ([docs](https://ampcode.com/docs/cli/settings)) | same | `%USERPROFILE%\.config\amp\settings.json`; `%ProgramData%\ampcode\managed-settings.json` (docs) |
| Session store | none locally; threads live on ampcode.com | none locally | none locally |
| Data | `$HOME/.local/share/amp/` (`accounts.json`, `secrets.json`, `device-id.json`, `session.json`, `last-threads/`) (live) | same | unknown |
| Logs | `$HOME/.cache/amp/logs/` (`cli.log`, `threads/T-<uuid>.log`) (live) | same | unknown |
| Env vars that move these | `HOME`; `XDG_*` honoured when set (measured) | same | unknown |

The connector is the only one that sets `HOME`, plus the four XDG folders, all under the isolated home. Because `HOME` moves, goalcup writes `$HOME/.profile` to put the handed `PATH` back after a login shell resets it. That writer is proven only by its unit test (section 9).

## 4. Session format and identity

- There is no local transcript. The only local record is a per-thread log, `$HOME/.cache/amp/logs/threads/T-<uuid>.log`: JSON lines with timestamp, level, message, thread id and pid. It holds transport state, frame lengths and tool executions, almost no conversation text (live).
- That folder is kept from a live run on an older version and cannot be confirmed from the package strings. The connector reads it as one `native` row, `thread-metadata`, pattern `T-*.log`. If the folder differs on `0.0.1793275344`, the row finds nothing and goalcup says so; the wire record is unaffected.
- The thread id is `T-<uuid>`. It is created by `POST /api/thread-actors` before the first message, so it is known before the first model turn (live).

## 5. Auth

- Amp account always: `amp login` (device flow) stores `accounts.json` and `secrets.json` under the data folder, mode 0600 (live). `AMP_API_KEY` takes precedence over saved accounts ([docs](https://ampcode.com/docs/cli)).
- Does the harness have a variable that moves its config and login together? No single one; `HOME` plus the XDG folders move them, so the connector sets all five and the participant signs in once, in `goalcup setup`.
- The thread WebSocket carries a short-lived JWT, `wsToken`, in its `Sec-WebSocket-Protocol` header (live). The connector's `mask` rule `amp_ws_token` covers the field `wsToken`.

## 6. Network

- Proxy and CA: `HTTPS_PROXY` and `NODE_EXTRA_CA_CERTS` ([docs](https://ampcode.com/docs/cli/settings)); live runs sent login, REST and the WebSocket through the proxy and trusted the run CA. The CA variable adds to the roots.
- Hosts: `ampcode.com` (listed as a vendor host, decrypted and recorded) and `authapi.ampcode.com` (sign-in, never decrypted). The thread is one WebSocket of JSON-RPC text frames at `/actors/gateway/threadActor/`, HTTP/1.1 upgrade (live).
- Provider calls are made on Amp's servers, not on the participant's machine ([docs](https://ampcode.com/docs/customize/model-routing)). The proxy records the thread WebSocket, never the call to the model provider. This is the main limit for point 1: a recorded Amp run is a recording of Amp's thread, not of provider requests.
- Ripgrep: with no `rg` on `PATH`, Amp downloads one at start from a host that passes as a tunnel and is not recorded. Install `rg` first.

## 7. Process model

- The local CLI is an executor: Amp's server sends tool leases, the CLI runs the tool and returns the result (live). Subagents (`Task`) arrive on the same WebSocket with a parent tool id (live, 22 Oct).
- Orbs (`-ox`) and runners run on Amp's machines; no local trace exists. Amp's Claude Code mode starts a second harness. [RULES](../RULES.md) does not allow either.

## 8. Things Goal Cup cannot isolate

- Server-side state: the thread, workspace managed settings, plugins and official skills live on ampcode.com.
- Machine-wide managed settings, `/etc/ampcode/managed-settings.json` on Linux, outside any home.
- Skills read from the workspace's ancestors and from Claude Code folders.
- `$TMPDIR`: Bun writes a native addon there, shared with the participant's other Amp runs.

## 9. Quirks and open questions

- `AMP_SKIP_UPDATE_CHECK=1` (set by the connector) "overrides this setting and disables all update checks"; `amp.updates.mode` defaults to `"auto"` ([docs](https://ampcode.com/docs/cli/settings)).
- Unknown: whether `amp last` resumes the same thread after a home switch, log rotation on a long run, and the Windows paths.
- No Amp account was available this cycle, so no run exists and nothing here was run on `0.0.1793275344`.

## Run without permission prompts

None needed: Amp runs tools without asking by default (strings in the binary; a Mac run made 19 commits with no prompt). Do not add `amp.permissions` rules that ask. The connector has no `no_prompts` row. See [HARNESS-SETUP](../HARNESS-SETUP.md#amp).

## Pad result

The tier, the tested version and the runs are in [`connectors/tiers.toml`](../../connectors/tiers.toml) and `PAD-RESULTS.md`; this sheet does not copy them.
