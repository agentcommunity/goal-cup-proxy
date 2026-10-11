# Amp CLI M2 readiness record

Can the exact Amp CLI candidate be captured through the existing schema-1 seam, and what blocks it?
For the maintainer deciding Tasks 2 and 3 of the v0.6.0 plan, and for whoever runs the live probe.
Updated: 2026-10-11.

- Status: **BLOCKED**. Transport, native storage/scope, auth lifecycle and identity binding are
  UNKNOWN or measured unsupported (see [Readiness verdict](#readiness-verdict)). No
  `connectors/amp-cli` exists and none may be added on this record.
- Candidate: npm `@sourcegraph/amp@0.0.1791676897-g77c125`, an alias that installs
  `@ampcode/cli` and its native per-platform package at the same version.
- Probe host: Linux x86_64, glibc, key-free. Goal Cup source `6befa1f` (public
  `goal-cup-tracer`). No credential existed, was requested, read or printed. No authenticated
  request was made; every Amp process ran in a network namespace with no route out.
- Raw output: `/tmp/claude-0/amp-probe/` (local, not in git): `raw/`, `tarballs/`, `x/`
  (extracted packages), `pkg/` (npm prefix), `static/strings.txt`, `scripts/`, one directory per
  probe run.

Every fact carries one evidence tag:

- **measured-here**: observed on this Linux host by a key-free, network-less probe.
- **static**: read from npm metadata, shipped files, or string literals in the binary (not run).
- **UNKNOWN**: not established; the owner's live probe below must settle it.

The probe method follows [HARNESS-ONBOARDING](../HARNESS-ONBOARDING.md#1-capability-worksheet).
Each Amp run used `unshare -n`, `env -i`, a fresh private `HOME` (and, in the first runs, private
`XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_CACHE_HOME`, `XDG_STATE_HOME`), a private `TMPDIR`, an
empty workspace with no `AGENTS.md` or `.amp`, stdin `/dev/null`, a timeout, and `strace` for
file and socket calls. The real `HOME` holds no `.config/amp`, `.local/share/amp`, `.cache/amp`
or `.amp` before or after (metadata check only).

## Frozen candidate identity

| Item | Value | Evidence |
|---|---|---|
| Requested package | `@sourcegraph/amp@0.0.1791676897-g77c125`, dist-tag `latest` on 2026-10-11; description "Renamed to @ampcode/cli"; sole dependency `@ampcode/cli` at the same version; no install script | static (`npm view`) |
| Alias tarball | `https://registry.npmjs.org/@sourcegraph/amp/-/amp-0.0.1791676897-g77c125.tgz`; integrity `sha512-bVFf8PIPalrPdMYrEQzJr7O/VBIRilIq7O7g4gGxXb2DcEV1VM6kPStVjIA5kMuMghbOBnmV0JXhB3O/6iSYtw==` | static |
| Real package | `@ampcode/cli@0.0.1791676897-g77c125`; tarball `https://registry.npmjs.org/@ampcode/cli/-/cli-0.0.1791676897-g77c125.tgz`; integrity `sha512-d+8BRL9pbiJM57wd4fQcWsjDHNGFfFeOMYnVN9qEEi0Kun4Pjz0unnTKdr6kXERAqwf8567BXF/uwhiXVpmtLg==` | static |
| Publisher | both published by "GitHub Actions" through npm trusted publishing (OIDC); `gitHead` `77c125710201165716ff6b47c9ec9441c1abc649`; registry signatures present (not verified here) | static |
| Install script | `@ampcode/cli` `postinstall: node install.cjs` (3,933 bytes, read in full). It resolves the platform package, hard-links (or copies) its `amp` to `bin/amp`, makes `bin/amp.exe` a symlink to `amp`, and makes no network call. Without it, `bin/amp.exe` is a 141-byte shell stub that prints an error and exits 1. Fallback `cli-wrapper.cjs` spawns the native binary under `node` | static; postinstall run by npm here |
| Platform packages | optional dependencies `@ampcode/cli-{darwin-arm64,darwin-x64,linux-arm64,linux-x64,win32-x64}` at the same version; each ships one `amp` executable | static |
| Executable form | native single-file executable built with Bun 1.4.1 (embedded `Bun v1.4.1 (4661e494f)`); ELF x86-64 on Linux, Mach-O arm64 and x86_64 on macOS. Not a JS entry and no `node` interpreter at run time | measured-here (`file`); static (strings) |
| Single-exe dist-tag | `next` is `0.0.1778732631-gc5875c-singleexe`, published 2026-05-14, older than `latest`; `latest` itself already ships native per-platform binaries | static |
| `amp --version` | `0.0.1791676897-g77c125 (released 2026-10-11T00:01:37.000Z, ...)`; `amp version` adds `npm package: @ampcode/cli` | measured-here |
| Installed link chain | `<prefix>/node_modules/.bin/amp` -> `../@ampcode/cli/bin/amp.exe` -> `amp`; `bin/amp` is a regular file with link count 2, the same inode as `@ampcode/cli-linux-x64/amp` | measured-here |
| Linux x64 payload | 134,157,792 bytes; SHA-256 `6c1b791d8e2c4661460b96276860c289d50ff0cacadaf1061ccb401295792036`; identical in the tarball, `@ampcode/cli-linux-x64/amp` and `@ampcode/cli/bin/amp`; unchanged after all probes | measured-here |
| darwin-arm64 payload | 109,666,576 bytes; SHA-256 `107c8b2c312e97447bd20d9f7d678d477195bd330753de81749b547bc7477f44` (tarball `sha512-gVjbZZXyRbmkPN19FXGtxF+MbGlMDaTwduHC3x7tO0PiXdLGl57oTFxlKIyHz3UdtwfdexyK5is6ofxe0O2vAQ==`) | static (extracted on Linux); installed macOS bytes UNKNOWN |
| darwin-x64 payload | 87,444,976 bytes; SHA-256 `7e99fec7bfd68613cd73fe08f65abc4a9893ca5fff96e3a0d0d07905124dcc06` (tarball `sha512-9FjTcgx7KEIuhqjX2cZtlWh2ozpz8vDsxqVOKKqzKf5YmglYJ0zOKI7H8TDOx5bOs0ZsIK+kpQxFT8Gcz3X9ow==`) | static; installed macOS bytes UNKNOWN |

The pin target is the regular file, not either symlink. Goal Cup's `capture_executable` resolves
`program` on the launcher's `PATH` and canonicalises it before `pin_harness_executable`, so
`program = "amp"` with the npm `.bin` on `PATH` pins `@ampcode/cli/bin/amp` and the symlink refusal
in `launch.rs` is not reached on that path (source reading of
[start.rs](../../crates/goalcup-cli/src/commands/start.rs), `capture_executable`). The Bun runtime
needs no interpreter, so the interpreter refusal does not apply.

## Capability worksheet

| Capability | Fact | Evidence |
|---|---|---|
| Headless argv | `amp --stream-json --execute=<goal>`: the whole goal binds as the execute message, including goals `--help`, `-v` and `threads` (logged `executeFlag` equals the goal). `-x <goal>` as two elements also works; words after a bare `-x` are parsed as a command. Help says execute mode also turns on when stdout is redirected | measured-here (offline; the run then fails to reach the server); static (help) |
| Goal placement | fits `goal_flag = "--execute"`; no argv pre-scan seen for `--help` or `-v` inside the goal, so no `argv_guard` row is indicated | measured-here |
| Local executor | `--executor local|orb|runner:<id>` exists; what "local" keeps on this machine is UNKNOWN | static (help); UNKNOWN |
| Provider and hosts | the CLI talks to `AMP_URL` (default `https://ampcode.com/`). Seen offline: `GET https://ampcode.com/api/internal?loadPlugins`. Strings: `/api/thread-actors`, `/api/threads/`, `/api/telemetry`, update check `https://static.ampcode.com/cli/cli-version.txt`, ripgrep download `https://storage.googleapis.com/amp-public-assets-prod-0/ripgrep`. No provider inference host is present; `api.anthropic.com` appears only in a Claude Code OAuth helper path | measured-here (log); static |
| Model relay | inference appears to run behind `ampcode.com` (no provider host in the binary; "thread actor" and rivetkit strings). Wire evidence would then be Amp's own protocol to its server, not a provider API | static; UNKNOWN |
| Transport, version, body | strings show a rivetkit actor client (`Sec-WebSocket-Protocol`, `rivet_encoding`, `rivet_conn_params`, `rivet_token`), a bundled `cbor-extract` native addon and `text/event-stream`. WebSocket, SSE, HTTP version and body encoding (JSON or CBOR) are all UNKNOWN | static; UNKNOWN |
| Forward proxy | embedded Bun runtime code reads `HTTPS_PROXY`/`https_proxy`/`NO_PROXY`; a setting `amp.proxy` also exists. Whether the actor WebSocket honours the env proxy is UNKNOWN | static; UNKNOWN |
| CA variable | embedded Bun runtime reads `NODE_EXTRA_CA_CERTS` (extend) and offers `--use-system-ca`/`NODE_USE_SYSTEM_CA`; `NODE_EXTRA_CA_CERTS` is an admitted extend row. Live trust behaviour is UNKNOWN | static; UNKNOWN |
| Config paths | with only `HOME` set: settings `$HOME/.config/amp/settings.json` (and `.jsonc`), log `$HOME/.cache/amp/logs/cli.log`, data `$HOME/.local/share/amp/`. With XDG set, settings and log follow `XDG_CONFIG_HOME` and `XDG_CACHE_HOME`. `AMP_HOME` does not move settings or log | measured-here |
| Data written key-free | `device-id.json` (an installation ID, mode 0600), `.legacy-state-owner.json` (see auth row); read-attempted `accounts.json`, `secrets.json`, `workspace-managed-settings-<hash>.json`; "file-based secrets storage" on Linux | measured-here |
| Other reads | `/etc/ampcode/managed-settings.json` (machine-wide, outside any private home); `/etc/gitconfig`, `$HOME/.gitconfig`, `$HOME/.config/git/config`. macOS managed path appears as `Application Support/ampcode` (full path UNKNOWN). Server-side workspace managed settings and global plugins are fetched from `ampcode.com` | measured-here (Linux); static (macOS) |
| Workspace settings | `<workspace root>/.amp/settings.json`; the root is the nearest ancestor holding `.git`, else cwd. A non-git ancestor `.amp/settings.json` was not read | measured-here |
| Ancestor inheritance | skills are scanned from cwd and every ancestor up to `/` (logged `searchRoots`). `AGENTS.md`/guidance file search is UNKNOWN (`AMP_IGNORE_GUIDANCE_FILES` exists) | measured-here (skills); static |
| Run-time temp code | Bun extracts a 3,114,112-byte native addon to `$TMPDIR/.bun-0-<hash>.node` (mode 0600) and `dlopen`s it. A second run with the same `TMPDIR` reused the existing file after only a `stat`; no content check was seen in its syscalls | measured-here |
| Updater | `amp update [--target-version]`; setting `amp.updates.mode` = `warn`/`disabled`/`auto`; env `AMP_SKIP_UPDATE_CHECK=1`; runner `amp.runner.autoUpdate.enabled`; `AMP_BINARY_PATCH_UPDATES`; a `brew upgrade ampcode/tap/ampcode` path. What an npm-installed binary does on update, and whether the check runs in execute mode, are UNKNOWN; no update URL appeared in the offline debug log | static; UNKNOWN |
| Credential | `AMP_API_KEY` is an access token and "takes precedence over stored credentials"; `amp login` stores credentials in the data dir. Token grammar and length are UNKNOWN | static |
| No-auth path | no key, execute mode: prints `No API key found. Starting login flow...` and a login URL to **stdout**, then waits on stdin for a pasted code; with stdin `/dev/null` it did not exit in 40 s (killed, exit 124) | measured-here (network-less) |
| Unreachable-server path | synthetic invented token, no route out: exit 1, stdout empty, stderr `Couldn't connect to the Amp server at https://ampcode.com/.`. The synthetic token was absent from every file under the private home, workspace and `TMPDIR`, from stdout, stderr and the debug log | measured-here |
| Key-derived file | `$HOME/.local/share/amp/.legacy-state-owner.json` holds `identityKey`, 16 hex digits. It was identical for the same synthetic token across three fresh homes and different for another token: a deterministic function of the key, i.e. a key fingerprint written to disk. Its derivation is UNKNOWN (not plain SHA-256/SHA-1/MD5/SHA-512 of the token) | measured-here |
| Native session store | UNKNOWN. No thread file appeared (no thread was created offline). Strings point to server-side "thread actors" (`/api/thread-actors`, Rivet), and `threads export`/`threads raw` fetch from the server. A local append-only or snapshot own-session file is not evidenced | static; UNKNOWN |
| Identity carrier | documentation names a stdout `system`/`init` event with `session_id`; thread IDs look like `T-<uuid>` (fits the `token` grammar). Neither failure path emitted an init event, so it follows server contact. Timing, field paths and uniqueness are UNKNOWN | static; UNKNOWN |
| Semantics | stream JSON claims Claude Code compatibility; no Amp decoder exists. Do not infer a provider wire format from it | static |
| Privacy | `amp orb redact-output` and `AMP_DISABLE_SECRET_REDACTION` exist; error and login text goes to stdout and stderr; key echo on a real 401 is UNKNOWN | static; UNKNOWN |

## Mapping to Goal Cup policy

Source: [capture_policy.rs](../../crates/goalcup-protocol/src/capture_policy.rs),
[harness.rs](../../crates/goalcup-protocol/src/harness.rs) `check_cross_field`,
[launch.rs](../../crates/goalcup-cli/src/launch.rs) `build_capture_launch` and
`child_secret_for_harness`, and the generic kit
[harness_contract_kit.rs](../../crates/goalcup-cli/tests/harness_contract_kit.rs). Field contracts:
[CONNECTOR-SCHEMA](../CONNECTOR-SCHEMA.md).

| Fact | Goal Cup today | Result |
|---|---|---|
| Native Bun executable behind npm symlinks | `program = "amp"`, canonicalised and pinned to the regular file | fits |
| Exact version and hash | executable pin refuses changed bytes before and after registration; never silently re-pinned | fits (macOS hash to re-measure) |
| `--execute=<goal>` | `goal_flag = "--execute"`, `args` not ending in `--` | fits |
| State under `HOME` when XDG is unset | capture env is built from nothing (`env_clear`), so no `XDG_*` reaches Amp and `HOME` is the per-run run home: all measured Amp state lands in the run directory | fits for isolation |
| Seeding Amp settings | seeds land only under the per-run config home (`home.env`); Amp reads `$HOME/.config/amp`. `XDG_CONFIG_HOME` is refused as `home.env` (XDG prefix and shape); `AMP_SETTINGS_FILE` cannot take an absolute value; no `{config_home}` template token | blocked only if a settings seed is required |
| Update suppression | `AMP_SKIP_UPDATE_CHECK = "1"` passes the `capture.env` name and value rules | fits by static reading; effect UNKNOWN |
| `home.env` for a per-run home | `AMP_HOME` passes the one-word `_HOME` shape; measured not to move settings or log. Its remaining effect (tool bin dir, installer) is UNKNOWN | candidate fit, owner to confirm |
| `AMP_URL` endpoint override | absent from the built env; a `://` value is refused by `env_value_denied`; the name itself passes `env_name_denied` (no `_BASE_URL`/`_API_URL`/`ENDPOINT`) | gap: name not refused |
| `AMP_API_KEY` | `CREDENTIAL_ENV` admits only `DEEPSEEK_API_KEY` | blocked |
| Credential with `stdout_json` identity | `check_cross_field` refuses it: one tee per stream, so key scan and identity cannot share stdout | blocked |
| Credential needs `per_run` | `config_home = "per_run"` required; a `home.env` must still be named | fits if `AMP_HOME` is acceptable |
| Synthetic kit for a credential profile | kit `Expect` has no credential field and the runner supplies no synthetic key, so a credential connector case cannot run | blocked for Task 3 |
| `NODE_EXTRA_CA_CERTS` | admitted extend row | fits by static reading; live UNKNOWN |
| Proxy-only HTTP/1.1 with WebSocket relay | the proxy offers ALPN `http/1.1` only and relays an upgraded WebSocket | UNKNOWN until live |
| Own-session native store | schema 1 needs one `append_only` or `snapshot` source; stdout JSON is not native capture | blocked (UNKNOWN, likely absent) |
| Key fingerprint on disk | D41 forbids persisting credential values or fingerprints; the key-fragment scan matches fragments, not a derived hash | blocked pending a decision |
| Machine-wide managed settings and server-pushed settings/plugins | no Goal Cup control over `/etc/ampcode` or `ampcode.com` account state | residual; must be disclosed or ruled out |
| Bun addon reused from `TMPDIR` | capture launch passes the launcher's `TMPDIR` through `CAPTURE_IDENTITY_ENV` | residual (same-UID substitution) |
| Interactive login on no key | Goal Cup refuses a launch with no key before spawning, so the login flow is not reached; on a revoked key the stdout login text would precede any JSON | fits; revoked-key path UNKNOWN |

## Core changes Amp would need

Smallest first. Each is a separate reviewed core PR with tests (and a DECISIONS row where policy
changes); none belongs in a connector-data PR. Items 6 and 7 may be unfixable on Goal Cup's side.

1. **Refuse `AMP_URL` by name.** Add it to `env_name_denied` in `capture_policy.rs` (an exact
   name, or a `_URL` suffix rule if review prefers), with a `harness_schema.rs` refusal case.
   Defence in depth: today only the value rule stops it.
2. **Admit `AMP_API_KEY`.** Add it to `CREDENTIAL_ENV` with a DECISIONS row (plan Task 2), only
   after the owner confirms the token fits `child_secret_named`'s grammar: 20 to 256 bytes of
   `[A-Za-z0-9_-]`. If it does not, the grammar change is part of this PR.
3. **Teach the generic kit a credential.** Add an optional credential field to the kit `Expect`
   and have the runner inject an invented key into the launcher env, so a credential profile is
   synthetic-testable without a real key.
4. **Private `TMPDIR` per capture launch.** Point `TMPDIR` at a directory inside the run
   directory instead of passing the launcher's, so Bun's extracted addon cannot be pre-planted.
   Optional hardening; it touches every harness, so it needs its own review.
5. **Credential with `stdout_json` identity.** One stdout tee that both scans for key fragments
   and extracts the identity event, then lift the refusal in `check_cross_field`, with a kit case.
   Needed only if live probing confirms the stdout init event is the identity carrier.
6. **Settings seed under the run home.** Only if a reviewed settings file is required (for
   example `amp.updates.mode = "disabled"` if `AMP_SKIP_UPDATE_CHECK` proves ineffective): either
   seeds below the run home or a schema-1 home mode where `HOME` is the per-run config home. A
   DECISIONS row, because it changes the home contract.
7. **Key fingerprint and native store.** These are owner decisions, not small patches:
   - The `identityKey` file needs an Amp-side suppression, a seed that prevents it, or an
     explicit D41 amendment that confines it to the uncommitted run directory.
   - If Amp keeps no local own-session store, schema 1 has no fit. A wrapper that writes a
     native file from stdout or an export is forbidden by the plan. Support would need a new core
     design for a server-relay harness, and the wire leg would need an Amp-protocol decoder.

## Owner live-probe steps (macOS)

The owner alone supplies the key, through the approved local route (a private mode-0600 env file
loaded into the launching process only). Never paste the key into a command line, a file in this
repository, a log or this record; record names and outcomes only. Run every step with a fresh
private `HOME` (and no inherited `XDG_*`), a private `TMPDIR`, an empty workspace outside any Git
work tree, no `AGENTS.md` or `.amp`, the forward proxy and its CA, and a watchdog.

1. **Identity of the install.** On each claimed Mac architecture run
   `npm install --prefix <private> @sourcegraph/amp@0.0.1791676897-g77c125`. Record `npm view` integrity, the canonical path of
   `node_modules/.bin/amp`, `amp --version`, and SHA-256 of `@ampcode/cli/bin/amp`; compare with
   the darwin payload hashes above. Re-hash after every later step.
2. **Machine state.** Record whether `/Library/Application Support/ampcode/` (and any
   `managed-settings.json` or `managed-settings.d`) exists; metadata only.
3. **No-auth path.** No key, `amp --stream-json --execute=<goal>` with stdin `/dev/null`: confirm
   it waits on the login prompt, and that nothing reaches the network beyond what is logged.
4. **Auth-error path.** An invalid but well-formed invented token: record exit, stdout, stderr
   and log text (scan them for the invented value), and every file written.
5. **Decoy session.** In one private home, run a first short goal, then the measured goal.
   Establish whether any local file holds thread content, its path, form (append-only JSONL,
   rewritten snapshot, SQLite or none) and whether it is scoped by thread ID. Confirm the decoy
   thread is distinguishable.
6. **Identity.** Capture stdout JSON lines: the first event's `type`/`subtype`, the
   `session_id` path, whether it precedes the first model request, and whether it matches a
   native file name or field.
7. **Transport.** Through the proxy with `NODE_EXTRA_CA_CERTS`: hosts, paths, ALPN, HTTP
   version, WebSocket upgrade or SSE, body encoding (JSON or CBOR), and which hosts tunnel. Repeat
   once without the CA to confirm Amp fails closed rather than falling back to system trust.
8. **Credential residue.** After a real row, scan the private home, `TMPDIR`, stdout, stderr and
   the log for key fragments with the existing scanner; record whether `.legacy-state-owner.json`,
   `secrets.json` or `accounts.json` was written (existence and size only), and whether the macOS
   Keychain was touched.
9. **Workspace-ancestor settings.** Place a decoy `.amp/settings.json`, `AGENTS.md` and
   `.agents/skills` in an ancestor of the workspace (with and without a `.git` ancestor) and
   confirm which are read during the measured goal.
10. **Updater.** With `AMP_SKIP_UPDATE_CHECK=1`, confirm no request to `static.ampcode.com` and
    no change to the executable bytes; repeat with `amp.updates.mode` unset to see default
    behaviour. Also note any ripgrep download from `storage.googleapis.com` and where it lands.

Only these steps can establish: the macOS installed hash, the native store and its scope, the
identity carrier and its binding time, the transport and body encoding through the proxy, CA
behaviour, the token grammar, auth-error echo, Keychain use, ancestor guidance inheritance and
updater effect in execute mode.

## Readiness verdict

| Phase 1 field | State |
|---|---|
| Executable identity | known on Linux; macOS installed bytes UNKNOWN |
| Headless argv and goal placement | known (`--stream-json --execute=<goal>`) |
| Transport, hosts, version, body | UNKNOWN |
| Isolation | per-run `HOME` contains measured state; machine-wide and server-pushed settings, ancestor skills and `TMPDIR` reuse are residuals |
| Auth | `AMP_API_KEY` not admitted; a key fingerprint is written to disk; grammar UNKNOWN |
| Native storage/scope | UNKNOWN, likely server-side only |
| Identity source/binding | UNKNOWN; candidate `stdout_json` refused with a credential |
| Semantics | UNKNOWN; no decoder |

**BLOCKED.** Downstream Tasks 2 and 3 stay closed until the live probe replaces every UNKNOWN in
transport, native storage/scope, auth and identity, and the owner rules on the key-fingerprint
file. If the live probe finds no local own-session store, Amp is BLOCKED unsupported under schema
1, and only a separately scoped core design could change that.

## Handoff record

```text
Claim sought / actual outcome: readiness / BLOCKED (key-free probe only)
Source revision; dirty inputs: goal-cup-tracer 6befa1f; none
Candidate executable path/version/SHA-256; package provenance:
  <prefix>/node_modules/@ampcode/cli/bin/amp; 0.0.1791676897-g77c125;
  linux-x64 6c1b791d8e2c4661460b96276860c289d50ff0cacadaf1061ccb401295792036;
  npm trusted publishing, gitHead 77c125710201165716ff6b47c9ec9441c1abc649
Connector directory pin; harness-contract schema: none; schema 1 target
Transport hosts/paths/version/body; native storage/form/scope: UNKNOWN; UNKNOWN
Isolation roots (metadata only); auth variable names and lifecycle:
  per-run HOME (.config/amp, .local/share/amp, .cache/amp); AMP_API_KEY (not admitted)
Identity source/binding and crosscheck; semantic decoder: UNKNOWN; none
Generic kit command, build/test exits and case results: not run (no connector, no cargo)
Real-row ENTRY/RUN_DIR/TRACE and exits: none
Independent result bytes and hash: none
Redaction, gaps, unknowns, private artifact paths and deferred coverage:
  /tmp/claude-0/amp-probe (local, not in git); UNKNOWNs listed above
Reviewer, permitted claim and qualification evidence link: unreviewed; no claim; none
```
