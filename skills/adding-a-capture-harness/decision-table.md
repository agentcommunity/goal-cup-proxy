# Do I need a core PR?

Can this harness be a data-only connector, or does it need a reviewed core change first? For a
contributor or agent at step 3 of [SKILL.md](SKILL.md). Updated: 2026-10-14.

Each row is a closed rule in
[`capture_policy.rs`](../../crates/goalcup-protocol/src/capture_policy.rs) or
[`harness.rs`](../../crates/goalcup-protocol/src/harness.rs) (`check_cross_field`). Field limits
are in [CONNECTOR-SCHEMA](../../docs/CONNECTOR-SCHEMA.md).

**Row 0 comes first**: the [check-first list](../../docs/HARNESS-ONBOARDING.md#row-0) names the
harnesses no row below can fit (no local session file, a database store, HTTP/2-only or gRPC
traffic, a local model, a desktop app). Stop there before this table; its known gaps do not
stop you.

**Core PR** means: STOP. Open an issue that names the row and links your facts sheet
(`docs/harnesses/<id>.md`, step 2). Write no connector until a maintainer lands the change (with
a [DECISIONS](../../docs/DECISIONS.md) row when policy moves). Never spell around a rule. The
facts sheet, with `Status: facts only`, is still a complete PR.

## The table

| # | Rule | Data-only fits when | Otherwise |
|---|---|---|---|
| 1 | Credential name | no key variable, `DEEPSEEK_API_KEY`, or `AMP_API_KEY` (D49) | core PR |
| 2 | Credential with identity | per-run home; `stderr_line`, or `stdout_json` (D50) | core PR |
| 3 | Credential in the kit | none, or the case names it in `credential_env` | core PR |
| 4 | Identity carrier | `none`, `preassigned`, `stdout_json` or `stderr_line` (plain-text stdout is not one) | core PR |
| 5 | Home variable | `home.env` is `<WORD>_HOME` or `<WORD>_CONFIG_DIR`, or is left out | core PR |
| 6 | XDG | the harness falls back to `$HOME` and needs no XDG seed | core PR |
| 7 | CA variable | the harness honours `CODEX_CA_CERTIFICATE`, `NODE_EXTRA_CA_CERTS` or `HERMES_CA_BUNDLE` | core PR |
| 8 | Native form | the own-session file is appended JSONL or rewritten whole | core decision |
| 9 | One native row | exactly one local own-session store | core decision |
| 10 | Executable | one entry-point file (a binary, or a script with a shebang), not an interpreter | fits (recorded, not pinned) |
| 11 | Endpoint names | you set no endpoint variable at all | issue, not blocking |
| 12 | Other refusals | no refused env name, argv path or custom regex is needed | core PR |
| 13 | Argv and env | always, when no generic rule refuses them (D51): no Rust row, a model change included; the reviewer reads every value | refused: row 12 |

Rows 1, 7 and 12 (the redaction regex) are closed on purpose; they are security boundaries, not
missing features. A credential name decides which secret the tracer copies from the
participant's shell into your process. A CA variable is a trust statement about the
participant's machine. A redaction regex runs inside the signed daemon. Each new one is a core
PR with a DECISIONS entry.

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
4. Exactly four ways to tell the tracer the session id are supported, each with its binding:
   - `none` (no binding): no id; the native root is private to the run.
   - `preassigned` + `at_launch`: the harness accepts an id flag; the tracer mints the id and
     passes it in `args` as `{session_id}`.
   - `stdout_json` + `at_launch`: the harness prints a JSON line on stdout, before any model
     text, with a field that marks it (`match_field`/`match_value`) and the id (`id_field`).
   - `stderr_line` + `deferred`: the harness prints a stderr line that is exactly
     `<prefix><id>`, where the id is a `token` (`[A-Za-z0-9_-]{1,128}`) or `timestamp-hex`.

   A session id printed as plain text on stdout (for example `session: abc`) is **not**
   supported. Map it onto one of the four if the harness can: an id flag (`preassigned`), a
   JSON event mode (`stdout_json`), the same line on stderr (`stderr_line`), or, when its
   session files land under the per-run home anyway, no id at all (`none`). If none fits, it
   is a core PR, which adds a new carrier to the loader and a kit case for it.
5. The regex is `^[A-Z][A-Z0-9]*_(HOME|CONFIG_DIR)$`, and other tools' variables (`CARGO_HOME`,
   `NPM_*` and so on) are refused. A harness with no such variable, or one with another shape,
   leaves `home.env` out: it then keeps its state under the private run `HOME`, its native
   `root` is `{run_home}/...`, `config_home` is `per_run`, and it takes no seeds or `dirs`
   (nothing would read them). A login kept in that home lasts one run, and the run has no
   terminal for an interactive sign-in, so such a harness needs a credential (row 1) or keeps
   its login in the macOS Keychain (`run_home = "keychain_link"`).
6. Every `XDG_*` name is refused in `capture.env` and as `home.env`. The launch env starts
   empty, so XDG is unset and the harness uses `$HOME`, the private run home. Seeds land only in
   the config home that `home.env` names.
7. Registry: `CODEX_CA_CERTIFICATE` (extend), `NODE_EXTRA_CA_CERTS` (extend),
   `HERMES_CA_BUNDLE` (replace). `SSL_CERT_FILE`, `SSL_CERT_DIR`, `CURL_CA_BUNDLE` and
   `REQUESTS_CA_BUNDLE` are refused because they change trust for every child tool. A Python
   harness that honours only `SSL_CERT_FILE` or `REQUESTS_CA_BUNDLE` therefore needs a core
   PR for its own CA variable; say so early, in the facts sheet's section 6.
8. `append_only` or `snapshot` only. SQLite, LevelDB or any database store is refused.
9. Schema 1 refuses zero native rows (no wire-only connector) and more than one. Sessions kept
   only on a vendor server have no schema-1 fit.
10. No `sha256` (D53). The CLI resolves `program` on `PATH`, follows symlinks, and hashes the
    regular file only to record it and to refuse a swap mid-launch; any build runs. A pinned
    `program` cannot be an interpreter (`python3`, `node`, `sh`, `env`). A script with a shebang
    is fine: the script is what is recorded. Per-arch builds need nothing per platform; name the
    version command in `version_args` if there is one.
11. Names ending `_URL` (including `_BASE_URL` and `_API_URL`), `_BASE`, `_HOST`,
    `_HOSTNAME`, `_ADDR`, `_ADDRESS`, `_SERVER`, `_ORIGIN` or `_REGISTRY`, or containing
    `ENDPOINT`, are refused, and so is any value with `://`. A harness endpoint variable with
    another name is not refused by name: never set it, record it in the facts sheet and open an
    issue to refuse it.
12. For example `NO_PAGER` (the `PAGER` suffix rule), a runtime variable that loads code
    (`PYTHONBREAKPOINT`, `NODE_REPL_EXTERNAL_MODULE`), an absolute path in `args`, an argument
    that turns certificate verification off or names a command, hook or server, a `goal_flag`
    with the word `exec`, `file`, `run` or `config` (`--execute` is fine), a host that is
    `localhost`, `*.localhost`, single-label or numeric (row 0: a local model), or a redaction
    regex that is not one of the four core recognizers. How much the harness may do (model,
    sandbox, permissions) is never refused: that is the participant's, and a connector does not
    limit it.
13. There is no list of reviewed arguments or variables. A refused value is refused with the
    rule's reason at step 9. What no rule can see (a dropped safety flag, a value that turns
    self-update back on) is the reviewer's job, so write in the facts sheet why each flag and
    variable is there.

## Worked example: Amp

Amp is a coding agent CLI, installed from npm, that talks to Amp's own server. The rows below
come from Goal Cup's own key-free, network-less probe of Amp `0.0.1791676897-g77c125` on Linux
in October 2026, not from the Amp team; anything here that Amp's facts sheet
([amp.md](../../docs/harnesses/amp.md), a draft for the Amp team to confirm) corrects wins. The rows it hits, and why:

- **1, credential name**: Amp reads `AMP_API_KEY`. Admitted by D49. Fits.
- **2, credential with identity**: the likely carrier is a stdout JSON init event. Admitted by
  D50 with a per-run home. Fits once a live run confirms the carrier.
- **3, kit**: a keyed case names `credential_env`, as `synthetic-keyed-stdout-json`
  already does. Fits.
- **9, one native row**: no local own-session file was found; sessions appear to live on Amp's
  server. Core decision, possibly no schema-1 fit.
- **11, endpoint names**: `AMP_URL` redirects the server and is refused by name since the
  `*_URL` rule. Fits.
- **6, XDG**: settings follow `XDG_CONFIG_HOME`, which is refused. With XDG unset all state
  lands in the run home, so it fits unless a settings seed is required.
- **5, home variable**: `AMP_HOME` passes the shape but does not move settings, so leave
  `home.env` out; settings then land in the private run home. Fits.
- **7, CA variable**: `NODE_EXTRA_CA_CERTS` is a registry row. Fits on paper; live UNKNOWN.
- **10, executable**: npm symlinks lead to one native binary, so `program = "amp"` resolves to
  the regular file, which is recorded. Fits.

Amp most likely stops at row 0, before this table: no local own-session file was found, and
row 9 is the same fact seen from the schema. The rows above assume a facts sheet that finds one.
Until a local own-session file turns up, or a core decision admits server-side sessions, Amp's
facts sheet, with `Status: facts only`, is the whole contribution, and it is exactly what that
decision needs. Its goal flag, `--execute`, passes the D51 goal-flag rule (whole words only).
