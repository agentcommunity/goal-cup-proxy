# Gemini CLI harness facts

Connector id: `gemini-cli` · Facts for version: `0.66.0` · Written by: Goal Cup team · Last checked: 2026-10-31
Public disclosure: yes. Facts come from Google's public Gemini CLI docs and from strings and source of the 0.66.0 npm package. No keys, tokens, account names or personal paths are included.
Status: shipped connector ([`connectors/gemini-cli.toml`](../../connectors/gemini-cli.toml))

Source words: "docs" is [geminicli.com/docs](https://geminicli.com/docs/cli/session-management/) as read on 2026-10-31; "package" is the 0.66.0 npm bundle as read for the connector and the appendix notes of 29 Oct 2026; "unknown" is not measured or read. Nothing was run on a real Gemini CLI this cycle, so no fact here is "measured". The docs pages carry a notice that Gemini CLI was replaced by Antigravity CLI on 18 Jun 2026 for unpaid and Google One users; paid API keys and Code Assist licences still work.

## 1. Run one prompt, in chat mode first

- `gemini` with no argument opens the interactive session. `gemini --resume` (or `-r`) loads the most recent session; `/resume` opens the session browser ([docs](https://geminicli.com/docs/cli/session-management/)).
- The connector's `resume` is `["--resume", "latest"]`. The docs page read shows `--resume` alone, an index or a session id; `latest` is not on it. The word comes from the 0.66.0 package check named in the connector header.
- `gemini -p "<prompt>"` is the one-shot form; it is not chat mode and is not used. How to quit: unknown.

## 2. Install and record a version

- `npm install -g @google/gemini-cli@0.66.0`, Node 20 or later (package metadata). A stable release comes out weekly, on Tuesdays (npm, 29 Oct).
- `gemini --version` prints the version (unverified on this page set). The connector sets no `version` key, so any build runs and the `start` event records the one that ran.

## 3. Where it keeps things, per OS

| | macOS | Linux | Windows |
|---|---|---|---|
| Config dir | `~/.gemini/`, with `settings.json`; project `.gemini/settings.json` ([docs](https://geminicli.com/docs/reference/configuration/)) | same | unknown |
| Session store | `~/.gemini/tmp/<project>/chats/` ([docs](https://geminicli.com/docs/cli/session-management/)) | same | unknown |
| Logs / cache | unknown | unknown | unknown |
| Env vars that move these | `GEMINI_CLI_HOME`: "the root directory for user-level config and storage", and the CLI creates `.gemini` inside it (docs) | same | same |

The connector sets `GEMINI_CLI_HOME` to the isolated home, so every `~/.gemini` path above sits at `{home}/.gemini`. The `.env` file is searched from the working folder upward, stopping at the folder with `.git` or the home folder, then `~/.env`; the first one found is used and they are not merged (docs). The connector reads `{home}/.gemini/.env`, `{workspace}/.gemini/.env` and `{workspace}/.env` for host hints; a `.env` in a parent folder above the workspace, or the real `~/.env`, is not read.

## 4. Session format and identity

- JSONL, one file per session: `chats/session-<timestamp>-<first 8 of the id>.jsonl`. A subagent has its own file at `chats/<parent session id>/<subagent session id>.jsonl` (package). Older `.json` session files are still read by Gemini; the connector reads only `.jsonl`.
- `<project>` comes from a project registry, not a plain hash of the path; the docs call it `<project_hash>`. The exact rule is unknown, so the connector matches `**/chats/**/*.jsonl` under `{home}/.gemini/tmp`.
- Retention: `general.sessionRetention` is on by default with `maxAge` `30d`; deleting a session also deletes its logs and tool outputs (docs). A week-long run is inside that.

## 5. Auth

- Three ways in: sign in with Google, `GEMINI_API_KEY` from Google AI Studio, or Vertex AI (Application Default Credentials, a service account JSON or `GOOGLE_API_KEY`) ([docs](https://geminicli.com/docs/get-started/authentication/)). Variables can be kept in `.gemini/.env`.
- Sign-in is "cached locally"; the docs do not name the file. The package uses `oauth_creds.json` in the config folder or the system keyring. The connector lists `{home}/.gemini/.env` and `{home}/.gemini/oauth_creds.json` as secret files.
- Does the harness have a variable that moves its config and login? Yes, `GEMINI_CLI_HOME`; the connector sets it. A login kept in the system keyring is outside the isolated home (section 8).

## 6. Network

- Proxy: `HTTPS_PROXY`, `https_proxy`, `HTTP_PROXY`, `http_proxy` are read, and a global undici proxy agent honours `NO_PROXY` (package). There is no settings key for it, so nothing to refuse. Node: yes; `NODE_USE_ENV_PROXY` is not needed, goalcup sets it anyway.
- CA: `NODE_EXTRA_CA_CERTS` adds to the roots ([docs](https://geminicli.com/docs/resources/troubleshooting/)). It must be in the real launch environment, not in a `.env` file (package; issue 25987).
- Hosts: `generativelanguage.googleapis.com` is a provider host. `cloudcode-pa.googleapis.com` is a vendor host: it carries both the Code Assist model calls and the sign-in calls of Google sign-in, so goalcup cannot decrypt one and not the other. `oauth2.googleapis.com` and `accounts.google.com` are never decrypted. Usage statistics go to `play.googleapis.com/log` (package) and pass as a tunnel.
- Model address: `GOOGLE_GEMINI_BASE_URL` points the API-key route at another URL, which must be HTTPS unless it is local ([docs](https://geminicli.com/docs/reference/configuration/)); `CODE_ASSIST_ENDPOINT` does the same for Code Assist (package). The connector hints both, in the three `.env` files above and in the participant's environment. goalcup never sets either.
- Calls are HTTP/1.1 through undici; no WebSocket on the chat path (package, unverified by a run).

## 7. Process model

- `gemini` relaunches itself as a child `node` process with the same environment, and forwards termination signals to it (package, release notes 0.66.0). Exit codes: unknown.
- Local subagents run in the main process, each in its own context loop ([docs](https://geminicli.com/docs/core/subagents/)): `codebase_investigator`, `cli_help` and `generalist` are on by default, `browser_agent` is off. Each writes its own session file (section 4).
- Remote subagents use the Agent2Agent protocol (docs).

## 8. Things Goal Cup cannot isolate

- Remote A2A subagents: the model work happens on the server the participant names, so the wire shows only the request Gemini sends there.
- `cloudcode-pa.googleapis.com` mixes model calls with sign-in calls (section 6).
- `--sandbox` runs tools in a container with its own proxy setting (default `http://localhost:8877`) (package), and the browser agent starts Chrome with a profile under `~/.gemini/cli-browser-profile/` (docs).
- The system keyring, if sign-in is kept there.

## 9. Quirks and open questions

- `general.enableAutoUpdate` is `true` by default and has no environment switch (docs). The connector's `update_off` row writes `false` into `{home}/.gemini/settings.json` at every start and continue.
- Unknown: whether `-p` runs check for updates, the Windows paths, how to quit, exit codes, the project registry rule.

## Run without permission prompts

The connector has no `no_prompts` row. The docs pages read name no settings key for it; the package has the flag `--approval-mode yolo`, which goalcup does not pass. The participant answers the prompts Gemini shows. Whether a settings key for it exists: unknown.

## Pad result

The tier, the tested version and the runs are in [`connectors/tiers.toml`](../../connectors/tiers.toml) and [`PAD-RESULTS.md`](../plans/2026-10-30-v1.0/PAD-RESULTS.md); this sheet does not copy them.
