# Do I need a core PR?

Can this harness be a data-only connector, or does it need a reviewed core change first? For a
contributor or agent at step 3 of [SKILL.md](SKILL.md). Updated: 2026-10-11.

Each row is a closed rule in
[`capture_policy.rs`](../../crates/goalcup-protocol/src/capture_policy.rs) or
[`harness.rs`](../../crates/goalcup-protocol/src/harness.rs) (`check_cross_field`). Field limits
are in [CONNECTOR-SCHEMA](../../docs/CONNECTOR-SCHEMA.md).

**Core PR** means: STOP. Open an issue that names the row and links your readiness record. Write
no connector until a maintainer lands the change (with a [DECISIONS](../../docs/DECISIONS.md) row
when policy moves). Never spell around a rule.

## The table

| # | Rule | Data-only fits when | Otherwise |
|---|---|---|---|
| 1 | Credential name | no key variable, `DEEPSEEK_API_KEY`, or `AMP_API_KEY` (D49) | core PR |
| 2 | Credential with identity | per-run home; `stderr_line`, or `stdout_json` (D50) | core PR |
| 3 | Credential in the kit | none, or the case names it in `credential_env` | core PR |
| 4 | Identity pair | one of the four admitted pairs | core PR |
| 5 | Home variable | `home.env` is `<WORD>_HOME` or `<WORD>_CONFIG_DIR` | core PR |
| 6 | XDG | the harness falls back to `$HOME` and needs no XDG seed | core PR |
| 7 | CA variable | the harness honours a CA registry variable | core PR |
| 8 | Native form | the own-session file is appended JSONL or rewritten whole | core decision |
| 9 | One native row | exactly one local own-session store | core decision |
| 10 | Executable pin | one entry-point file, not an interpreter | record per-arch |
| 11 | Endpoint names | you set no endpoint variable at all | issue, not blocking |
| 12 | Other refusals | no refused env name, argv path or custom regex is needed | core PR |

## How to tell, row by row

1. `CREDENTIAL_ENV` lists `DEEPSEEK_API_KEY` and `AMP_API_KEY`. `AMP_API_KEY` is admitted by
   [D49](../../docs/DECISIONS.md#d49-m2-admit-amp_api_key-as-a-child-credential).
   Subscription or login auth needs no `[capture.credential]` table.
2. A credential always needs `config_home = "per_run"`. With `stderr_line` + `deferred` it is
   in force. With `stdout_json` + `at_launch` it rests on
   [D50](../../docs/DECISIONS.md#d50-m2-amend-d43-ruling-3-for-stdout_json-identity):
   one stdout reader feeds both the key-fragment scan and the identity
   matcher. A credential with `preassigned` or `none` identity is refused.
3. A kit case may name the credential variable in `expect.toml` as `credential_env`. The runner
   then hands the stand-in an invented key and checks that no fragment of it leaks. Copy the
   connector's `[capture.credential]` into the case, and write the marker as
   `synthetic-keyed-stderr/stand-in.py` does. The kit is macOS-only: off macOS, say in the
   PR that it was not run.
4. Admitted pairs: `none` (no binding); `preassigned` + `at_launch` (id passed in `args` as
   `{session_id}`); `stdout_json` + `at_launch`; `stderr_line` + `deferred` with grammar
   `token` (`[A-Za-z0-9_-]{1,128}`) or `timestamp-hex`. Any other carrier needs a kit case too.
5. The regex is `^[A-Z][A-Z0-9]*_(HOME|CONFIG_DIR)$`, and other tools' variables (`CARGO_HOME`,
   `NPM_*` and so on) are refused.
6. Every `XDG_*` name is refused in `capture.env` and as `home.env`. The launch env starts
   empty, so XDG is unset and the harness uses `$HOME`, the private run home. Seeds land only in
   the config home that `home.env` names.
7. Registry: `CODEX_CA_CERTIFICATE` (extend), `NODE_EXTRA_CA_CERTS` (extend),
   `HERMES_CA_BUNDLE` (replace). `SSL_CERT_FILE`, `SSL_CERT_DIR`, `CURL_CA_BUNDLE` and
   `REQUESTS_CA_BUNDLE` are refused because they change trust for every child tool.
8. `append_only` or `snapshot` only. SQLite, LevelDB or any database store is refused.
9. Schema 1 refuses zero native rows (no wire-only connector) and more than one. Sessions kept
   only on a vendor server have no schema-1 fit.
10. One `sha256`. The CLI resolves `program` on `PATH`, follows symlinks, and hashes the regular
    file. A pinned `program` cannot be an interpreter (`python3`, `node`, `sh`, `env`). Per-arch
    binaries hash differently: pin the platform you measured and list the others in readiness.
11. Names ending `_URL` (including `_BASE_URL` and `_API_URL`) or containing `ENDPOINT` are
    refused, and so is any value with `://`. A harness endpoint variable with another name (no
    `_URL` suffix) is not refused by name: never set it, record it in readiness and open an
    issue to refuse it.
12. For example `NO_PAGER` (the `PAGER` suffix rule), an absolute path in `args`, or a
    redaction regex that is not one of the four core recognizers.

## Worked example: Amp

Amp is a coding agent CLI, installed from npm, that talks to Amp's own server. It is BLOCKED at
step 3. Its readiness record is
[docs/qualification/m2-amp-readiness.md](../../docs/qualification/m2-amp-readiness.md). The rows
it hits, and why:

- **1, credential name**: Amp reads `AMP_API_KEY`. Admitted by D49. Fits.
- **2, credential with identity**: the likely carrier is a stdout JSON init event. Admitted by
  D50 with a per-run home. Fits once the live probe confirms the carrier.
- **3, kit**: a keyed case names `credential_env`, as `synthetic-keyed-stdout-json`
  already does. Fits.
- **9, one native row**: no local own-session file was found; sessions appear to live on Amp's
  server. Core decision, possibly no schema-1 fit.
- **11, endpoint names**: `AMP_URL` redirects the server and is refused by name since the
  `*_URL` rule. Fits.
- **6, XDG**: settings follow `XDG_CONFIG_HOME`, which is refused. With XDG unset all state
  lands in the run home, so it fits unless a settings seed is required.
- **5, home variable**: `AMP_HOME` passes the shape but does not move settings. Maintainer call.
- **7, CA variable**: `NODE_EXTRA_CA_CERTS` is a registry row. Fits on paper; live UNKNOWN.
- **10, executable pin**: npm symlinks lead to one native binary, so `program = "amp"` pins the
  regular file. Darwin hashes differ per architecture and need a Mac.

One core row is enough to stop. Amp stops at step 3, and its readiness record is the whole
contribution.
