# Kilo CLI harness facts

Connector id: `kilo-cli` · Facts for version: `7.8.25` · Written by: Goal Cup team · Last checked: 2026-10-30
Public disclosure: yes. Everything here comes from Kilo's public docs, the public `Kilo-Org/kilocode` source at 7.8.25 and one install of `@kilocode/cli@7.8.25` in a build box; no keys, tokens or personal paths are included.
Status: core ready, connector pending (the 1.0 connector file is `connectors/kilo-cli.toml`; the launch plan code is not built yet).

Source words: "vendor docs" means [kilo.ai/docs/cli](https://kilo.ai/docs/cli); "source" means `Kilo-Org/kilocode` at 7.8.25 (`packages/opencode/src/config/config.ts`, `packages/opencode/src/config/paths.ts`); "measured" means the build-box install; "unknown" means nobody on our side has read or measured it.

## 1. Run one prompt

- `kilo` opens the TUI in the current folder; `kilo --continue` (`-c`) resumes the last session (vendor docs; measured `--help`).
- `kilo run "<prompt>"` is the non-interactive form. Without `--auto` it cannot ask and rejects every permission request, exiting 1 (vendor docs). The connector runs the TUI.

## 2. Install and record a version

- npm package `@kilocode/cli`; `kilo --version` prints `7.8.25` (measured).
- Kilo installs patches by itself unless `KILO_DISABLE_AUTOUPDATE=1` (source, `packages/core/src/flag/flag.ts`, `packages/opencode/src/cli/upgrade.ts`). The connector sets it.

## 3. Where it keeps things

- Config: `$XDG_CONFIG_HOME/kilo/` (vendor docs; measured with `kilo debug paths`). Data: `$XDG_DATA_HOME/kilo/`. The connector points all four XDG variables into goalcup's home.
- Session store: `$XDG_DATA_HOME/kilo/kilo.db`, or `kilo-<channel>.db` off the latest channel (source, `storage/db.ts`; measured `kilo-rc.db`). The connector copies `kilo*.db*` at the end.
- Keys: `$XDG_DATA_HOME/kilo/auth.json` (source, `auth/index.ts`), or an `apiKey` in a config file.

## 4. Config files and which one wins

Global files, all read, merged in this order, a later file winning on the same key (source, `config.ts`):
`config.json`, `kilo.json`, `kilo.jsonc`, `opencode.json`, `opencode.jsonc`, all in `$XDG_CONFIG_HOME/kilo/`.

When Kilo writes its global config (first run, `/auto-approve`, "always run"), it writes into the first file that exists of `kilo.jsonc`, `kilo.json`, `opencode.jsonc`, `opencode.json`, `config.json` (source). In an empty home the first run writes `kilo.jsonc` (measured).

Project files (source, `paths.ts`): `kilo.jsonc`, `kilo.json`, `opencode.jsonc`, `opencode.json` from the launch folder up to the worktree root, then the same names in `.kilocode/` and `.kilo/`. Project settings win over global ones (vendor docs). After all files come `KILO_CONFIG`, `KILO_CONFIG_CONTENT` and managed settings, each later one winning (source).

Every file is parsed as JSON with comments (source, `ConfigParse.jsonc`), whatever its extension.

What the connector does with this:

- `host_hints` reads `provider.*.options.baseURL` from every global and project file above.
- `no_prompts` writes `permission = "allow"` into `opencode.json`. It is merged after `kilo.json` and `kilo.jsonc`, so a `kilo.jsonc` Kilo writes on its first run does not undo it. If `opencode.json` exists first, Kilo writes into it, keeping our key. Only `opencode.jsonc`, or a project file, can override it, and those are the participant's own choice. `permission` accepts one action for every tool: `"allow"`, `"ask"` or `"deny"` (vendor docs).
- `secret_files` names `auth.json` and the five global config names.

## 5. Seams for the hint reader (`connector/hints.rs`)

These connector rows rely on two reader rules that PRD-11 section 2 does not spell out yet:

1. **`.jsonc` is read as JSON with comments** (`//` and `/* */` comments and trailing commas allowed), not as dotenv. Section 2 reads any extension other than `.json`, `.toml`, `.yaml` and `.yml` as dotenv; read that way, Kilo's own default file `kilo.jsonc` hints nothing, with no error.
2. **A `file` holds at most one `*`, in its last component.** `kilo.json*` matches `kilo.json` and `kilo.jsonc`; each match is read by its own extension.

The hints slice ships a test that loads `connectors/kilo-cli.toml`, writes a `{home}/.config/kilo/kilo.jsonc` with a comment and a `provider.x.options.baseURL`, and checks that the host lands in the `config` list.

## 6. Network

- Model host: the provider's own (for example `api.deepseek.com`), Kilo's gateway (`api.kilo.ai`, `ai-gateway.kilo.ai`) or Vercel AI Gateway (`ai-gateway.vercel.sh`) set as an OpenAI-compatible provider's `options.baseURL` ([Vercel's Kilo guide](https://vercel.com/docs/ai-gateway/coding-agents/kilo-code)).
- Proxy and CA: `HTTPS_PROXY`; the OS store plus `NODE_EXTRA_CA_CERTS`. Kilo's TUI talks to its own local server, so it needs `NO_PROXY` for loopback (PRD-11 section 3).
- On an unknown model id Kilo may fall back to a paid gateway model (PRD-11 builder notes).

## 7. Process model

- One Bun process; subagents are child sessions in the same `kilo.db` (PRD-11 section 7). Exit codes of the TUI: unknown.

## 8. Things Goal Cup cannot isolate

- `KILO_CONFIG`, `KILO_CONFIG_DIR` and `KILO_CONFIG_CONTENT` in the participant's environment can name a provider `baseURL` no hint reads. Its host is then a tunnel the per-host flag shows.
- Kilo also reads `.kilo/` and `.kilocode/` in the real home directory (source, `paths.ts` searches from `Global.Path.home`). goalcup does not move `HOME`, so a `baseURL` there is not hinted either.
- Managed and organisation settings: unknown.

## 9. Open questions

- Every "unknown" above. Whether `kilo --continue` picks the right session after a home switch is not measured.
