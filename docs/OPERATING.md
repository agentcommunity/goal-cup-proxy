# Operating Goal Cup

Part 1 is the runbook for organizers: where things run, how to deploy, test runs, the matrix, and what
to do on a bad day. Part 2 is the reference for running the tracer binaries (build, daemon, CLI exit
codes, verifier, pins). The short path for a developer (build, gate, install, one capture) is in the
[README](../README.md). The server's design is in [SERVER-DESIGN](SERVER-DESIGN.md) and its wire
contract in [UPLOAD-API](UPLOAD-API.md).

Never write a secret, a key value or a key file path into a doc, an issue, a log or a chat. This page
names secrets only.

## Contents

- Part 1, organizers: [Where things run](#where-things-run), [Dates and checklist](#dates-and-checklist),
  [Deploy](#deploy), [Test runs](#test-runs), [Releasing the tracer](#releasing-the-tracer),
  [The matrix](#the-matrix), [Edge cases by hand](#edge-cases-by-hand), [Bad days](#bad-days)
- Part 2, the tracer: [Building and verifying](#building-and-verifying),
  [Installing the participant binaries](#installing-the-participant-binaries),
  [Running the daemon](#running-the-daemon), [Running a capture with the CLI](#running-a-capture-with-the-cli),
  [Running the verifier](#running-the-verifier), [Pinning connectors](#pinning-connectors)

# Part 1: organizers

## Where things run

| | Production | Staging | Local |
|---|---|---|---|
| Worker | `goalcup-ingest` at `https://ingest.agentcommunity.org` (custom domain, no workers.dev) | `goalcup-ingest-staging` at `https://goalcup-ingest-staging.b-9a5.workers.dev` | `npm run dev` at `http://localhost:8787` |
| R2 bucket | `goalcup-traces` (lock on `traces/goalcup-cup-2026/` only) | `goalcup-traces-staging` (no lock) | simulated on disk |
| Competition event | `EVENT_ID` in `wrangler.toml`: now `goalcup-dryrun-2026`, then `goalcup-cup-2026` | `goalcup-dryrun-staging` | any |
| Practice event | `goalcup-practice` (always open, 10-minute runs) | `goalcup-practice-staging` | any |
| Test event | `test` (prefix `traces/test/`, never locked) | `test-staging` | any |
| Database | the website's Supabase project, schema `goal_cup_tracer` | the same project | JSON state in the simulated bucket |
| Supabase key / role | `goalcup_ingest_production` / `goal_cup_ingest_production` | `goalcup_ingest_staging` / `goal_cup_ingest_staging` | none |
| Resend key | `goalcup-ingest-production` (send-only) | `goalcup-ingest-staging` (every mail goes to `delivered@resend.dev`) | none |
| Test approval route | never | on (`STAGING=on` and the `TEST_APPROVAL_KEY` secret) | on (`.dev.vars`) |
| Approval page | `https://agentcommunity.org/goalcup/approve` (the website) | the same | the test route |

The website repo owns the schema, migrations, sign-up, terms and the approval page. A schema change
is a website migration; copy it to `server/worker/sql/goal-cup-tracer-upload.sql` in the same change.
During the event week, schema changes are add-only.

## Dates and checklist

All times UTC.

| Date | What |
|---|---|
| 23–24 Oct | First release and Homebrew tap |
| 26 Oct | Freeze |
| about 28 Oct | Dry run (production, `goalcup-dryrun-2026`) |
| 7 Nov 00:00 | Start window opens. It lasts 24 hours. |
| 8 Nov 00:00 | Start window closes |
| 168 hours after each start | That run's deadline. The latest run ends 192 hours after the window opens. |

Before the window opens:

1. The `goalcup-cup-2026` row in the website's `events` table has `start_window_opens` =
   `2026-11-07T00:00:00Z` and `start_window_closes` = `2026-11-08T00:00:00Z`. The old placeholder is 6 Nov
   16:00. Change it with a website migration.
2. Set `EVENT_ID = "goalcup-cup-2026"` in `server/worker/wrangler.toml` (top-level `[vars]`) and deploy.
   Staging keeps its own event ids.
3. Run the matrix on staging for each shipped connector ([The matrix](#the-matrix)).
4. Confirm `GET https://ingest.agentcommunity.org/v1/health` answers `{"status":"ok"}`, then run one
   practice run against production by hand.
5. Delete the test runs ([Test runs](#test-runs)) and confirm the bucket lock rule still covers
   `traces/goalcup-cup-2026/`.
6. Check that `MAINTENANCE` is `off`.

Emails go out at most 20 each 5-minute tick (240 an hour). A burst of 1,500 `started` mails at the
window opening drains over about six hours. The limit is `NOTICES_PER_TICK` in `server/worker/src/mail.ts`.

## Deploy

Deploys are manual. From `server/worker`:

```sh
npm ci && npm test                                  # type-check, unit and end-to-end tests
CLOUDFLARE_ACCOUNT_ID=<id> npx wrangler deploy               # production: top level, no --env
CLOUDFLARE_ACCOUNT_ID=<id> npx wrangler deploy --env staging # staging
```

- Deploy staging first and run the matrix against it. Deploy production only from a tree whose
  `npm test` passes.
- Never set `DB_BACKEND=local` or `TEST_APPROVAL_KEY` in the production block. `test/config.test.ts`
  fails if you do.
- Roll back with `npx wrangler rollback` (the same account variable). Wrangler keeps recent versions.
- Secrets are set with `wrangler secret put <NAME>` (add `--env staging` for staging):
  `RECEIPT_SIGNING_KEY`, `SUPABASE_URL`, `SUPABASE_KEY`, `RATE_LIMIT_KEY`, `GITHUB_TOKEN`,
  `RESEND_API_KEY`, and for staging only `TEST_APPROVAL_KEY`. The signing seed is made offline and kept in
  the owner's password manager, never in a repo. Meanings: [server/worker/README](../server/worker/README.md#configuration).
- Watch a deploy with `npx wrangler tail`. Useful log lines: `tick <step> <run_id>: …` (a cron step failed),
  `r2-put-unrecognised-error` (R2 refused a put for a reason the Worker does not know), `misconfigured: …`.
- Before the first real upload, prove the two R2 behaviours listed in the worker README ("Before launch").

## Test runs

An organizer test run is a full-length run in the test event. Use it for the long edge cases.

- Only members on the database's `test_operators` list can approve or open one. Anyone else gets `403`.
- Open one with `goalcup start --harness <id> --test` (add `--dev` for an unpinned connector), logged in to
  the server you mean to use. Use staging for experiments:
  `goalcup login --server https://goalcup-ingest-staging.b-9a5.workers.dev`, in a configuration directory of
  its own (`GOALCUP_CONFIG_DIR`).
- A test run replaces only an earlier test run of the same operator. It never touches a competition run.
  It is left out of public numbers. Its emails go to the operator, marked `[TEST]`.
- Delete every test run, rows and R2 objects, in one go:

```sh
cd server/worker
sh scripts/delete-test-runs.sh                 # dry run: prints what would go
sh scripts/delete-test-runs.sh --yes           # deletes
sh scripts/delete-test-runs.sh --older-than '7 days' --yes   # only runs older than that
```

  It needs `psql`, `curl`, `python3` and these in the environment: `DATABASE_URL` (the website's Postgres),
  `CLOUDFLARE_ACCOUNT_ID` and `CLOUDFLARE_API_TOKEN` (R2 read and write). It touches only
  `goalcup-traces/traces/test/` and `goalcup-traces-staging/traces/test-staging/`, and stops if it finds a
  key outside them.

## Releasing the tracer

A `v*` tag that equals the workspace version builds four tarballs and the Homebrew formula. Steps and
layout are in [packaging/README.md](../packaging/README.md). The binaries are unsigned and not notarized.
Run `sh scripts/gate.sh full` before every tag. A release tarball carries `codex-cli` and
`claude-code-cli`; `hermes-cli` stays out because it needs a provider key.

## The matrix

Does a run of this connector, on this machine, against this server, record, stop on the server's
deadline, seal and upload a copy the server agrees with? `scripts/matrix.sh` answers that for one
connector in one command and prints one row. Run it before each release, for each shipped connector, on
each platform (macOS arm64 and x86_64, Linux x86_64 and arm64, WSL2).

```
scripts/matrix.sh <connector> <server-url> [--keep] [--log FILE] [--timeout SECONDS]
scripts/matrix.sh --check-copy <config-dir> [run-id]
```

It makes a temporary configuration directory, work folder and `HOME`, runs `goalcup login` and
`goalcup start --practice` in them (the harness under a pseudo-terminal into which it types one prompt),
waits for the tracer's own stop at the practice deadline (10 minutes), and deletes the temporary directory.
It prints the log path and one row:

```
<connector> <os>-<arch> <harness_version> PASS|FAIL <date>
```

`<os>` is `macos`, `linux` or `wsl2`; `<arch>` is `arm64` or `x86_64`. A FAIL names its first reason on
stderr and in the log. PASS means:

1. both approvals (the login, then the run) were given, the run opened, and the harness was up;
2. the run ended at its deadline: the tracer printed its `deadline:` line and `uploaded: … and the seal`;
3. `goalcup-verify --capture-integrity <trace>` exited 0 with `integrity: valid` and `anchor: signed`;
4. the server's copy agrees with the local chain.

**The server copy.** The upload API has no call that returns the stored chain, so the script does not
download it. It asks `GET /v1/runs/{id}?entry_id=…` with the run's own credential and requires the answer
to equal the local chain the verifier just checked: `offset` equal to the bytes of `chain.jsonl`,
`next_seq` to the record count, `head` and the last acknowledgement's `hash` to the verifier's
`validated_head`, `final` to the last record, `status: sealed` and `stop: true`. The server stores a chunk
only if its SHA-256 matches, at the offset, previous hash and sequence it holds, and signs an
acknowledgement the verifier checks. Equal positions therefore mean the server holds the chain the tracer
sent, in order. It does not prove the stored bytes; only the verify job (not built) would.

A server whose signing key this build does not pin (a local Worker, staging) fails the first verification
with `receipt_key_not_accepted`. The script then verifies again with `--dev-key` set to
`MATRIX_SERVER_KEY`, else the public key in the run's own receipt, and logs that the trust came from this
invocation (`key_source: "supplied"`).

**Rules the script keeps.**
- The approval key comes from a file: `GOALCUP_TEST_APPROVAL_KEY_FILE=<path>`. The script reads it only
  at the moment it approves, piping it into `curl`'s standard input, and removes the variable from its
  environment so no child learns the path. A key in an environment variable is refused. Never `cat` the
  file or put its value in an argument, a log or a command line.
- The member comes from `GOALCUP_TEST_MEMBER_ID`.
- With no key file, it prints the approval address and code and waits while you approve in a browser.
- It approves at most one device code per phase and stops looking for codes once the harness is up.
- It never reads or writes the real goalcup configuration directory, `~/.codex`, `~/.claude` or `~/.hermes`.
- It refuses `https://ingest.agentcommunity.org` unless given `--allow-production`. Never pass it. Plain
  `http` is accepted only to `localhost`, `127.0.0.1` or `[::1]`.

**Targets.**
- **A local Worker:** `cd server/worker && npm ci && cp .dev.vars.example .dev.vars`, then start the Worker
  with a short practice run: `node node_modules/wrangler/bin/wrangler.js dev --env-file .dev.vars --var
  DB_BACKEND:local --var PRACTICE_RUN_MINUTES:2 --port 8787`. In another terminal make a key file from
  the dev values (`sed -n 's/^TEST_APPROVAL_KEY=//p' server/worker/.dev.vars > "$KF" && chmod 600 "$KF"`,
  with `KF` in a fresh `mktemp -d` directory), run `GOALCUP_TEST_APPROVAL_KEY_FILE="$KF" sh scripts/matrix.sh
  m2-fake-tui http://localhost:8787`, then remove the key file and stop the Worker.
- **Staging:** needs a file holding staging's test approval key and a member id it may approve as:
  `GOALCUP_TEST_APPROVAL_KEY_FILE=<key file> GOALCUP_TEST_MEMBER_ID=<member id> sh scripts/matrix.sh
  <connector> https://goalcup-ingest-staging.b-9a5.workers.dev`. In Docker (the way to run Linux on a Mac),
  mount the file read-only (`-v <key file>:/run/key:ro`, `GOALCUP_TEST_APPROVAL_KEY_FILE=/run/key`); never
  pass the key with `-e`. A run takes about 12 minutes. With a per-app firewall such as Little Snitch, the
  new `goalcup` build is held until you allow its exact path; the row then FAILs with `goalcup login
  exited 1: … connection timed out`. Allow the binary and run again.
- **Production:** never.

**Connectors.** `m2-fake-tui` is a connector the script writes around the test suite's fake terminal UI
(`crates/goalcup-cli/tests/fixtures/m2-fake-tui.sh`), admitted with `--dev`: no harness, no model, no
login. A PASS proves the tracker, daemon, pseudo-terminal handling, server and verifier, and nothing about a
real harness. A shipped connector id (`codex-cli`, `claude-code-cli`) runs that harness for real. It needs the
harness installed and logged in inside a prepared configuration directory of its own:
`GOALCUP_CONFIG_DIR=<dir> goalcup prep`, then `MATRIX_CONFIG_DIR=<dir> sh scripts/matrix.sh codex-cli <url>`.
The script types `MATRIX_PROMPT` once, `MATRIX_PROMPT_DELAY` seconds (30) after the harness is up. In
real-harness mode a PASS does not prove the prompt was typed; check the session in the kept directory
(`--keep`) or the research report. Only the fake has been run through the script so far.

**Other environment.** `MATRIX_REPO_URL` (default: the public empty repository
`https://github.com/agentcommunity/goalcup-matrix-empty`), `MATRIX_BIN_DIR`, `--keep`, `--timeout`
(1800 s). It needs `curl` 7.55 or later, `python3` and `script(1)`.

**`--check-copy`.** For a run you made by hand, verifies every entry of the run locally and compares each
with the server's position, using the run's credential from the configuration directory. It starts and
changes nothing, and works on a run that is still recording. Entries still uploading may differ until they
catch up: run it again. A segment the tracker closed from the tail of its chain (after a reboot or a kill)
verifies with exit 1, incomplete coverage, and is reported on its line.

**Recorded rows.** A row from the fake connector proves the tracker, server and verifier on that platform
and says nothing about a harness.

| Row | Server | Notes |
|---|---|---|
| `m2-fake-tui linux-arm64 matrix-fake-tui_1.0.0 PASS 2026-10-15` | staging | Fake harness, in Docker on an arm64 Mac. Server copy matched: 12 records, `final` = record 11. |
| `m2-fake-tui macos-arm64 matrix-fake-tui_1.0.0 PASS 2026-10-15` | local Worker, 2-minute practice run | Fake harness. Same checks, same shape. Not yet run on staging from macOS (the firewall case above). |

## Edge cases by hand

Each case is a run you start yourself and break on purpose, to find what goes wrong in a week.

- **Run.** Practice runs last 10 minutes, enough for the quick cases. The long ones need an organizer test
  run on staging, in a configuration directory of its own, after `goalcup prep`. Never on production.
- **Watch** from a second terminal with the same `GOALCUP_CONFIG_DIR`: `goalcup status` (a `PROBLEM:` line
  for anything the participant must act on) and `goalcup entries`.
- **Check the server copy** when the case ends and uploads have caught up:
  `sh scripts/matrix.sh --check-copy "$GOALCUP_CONFIG_DIR"`. It prints `check-copy <os>-<arch> <run id> PASS
  <date>` and exits 0 when every entry matches.
- *Fake ok* means the fake harness (`m2-fake-tui`, through a `--dev` start on a prepared directory with
  `GOALCUP_CONNECTORS_DIR` set) is enough. *Real harness* means it is not, because the fake makes no model
  calls and writes no native session format the verifier decodes.
- A case passes when the tracer did what "Expect" says and `--check-copy` exits 0. File any failure with the
  exact `goalcup status` text, the log and the run id.

| # | Case | Do | Expect | Fail if |
|---|---|---|---|---|
| 1 | 30 minutes offline (fake ok; real for provider-outage lines) | Test run. Network off for 35 minutes (macOS `networksetup -setairportpower en0 off`; Linux `nmcli networking off` or `ip link set <if> down`), then on. | Recording continues. After 15 minutes `PROBLEM: the last N heartbeats to the server failed`. After 30 minutes without an acknowledged upload, `PROBLEM: last upload N min ago`. Back online the uploader retries (60 s, doubling to 15 min) and catches up; both lines go away. | The run stops, recording reports gaps the outage does not explain, or the server is still behind 25 minutes after the network returned. |
| 2 | Laptop sleep (fake ok) | Test run. Sleep 30 minutes (`pmset sleepnow`, or lid; Linux `systemctl suspend`). | The tracker holds a sleep lock (`PROBLEM: the machine may sleep` if it cannot). After wake: heartbeats resume within 5 minutes, the uploader catches up, and the deadline counts through the sleep (a deadline that passed while asleep stops the run at wake). | The tracer exits or hangs, `status` calls the recording healthy while the verifier reports a gap, or the server stays behind. |
| 3 | Agent killed, then `continue` (fake ok; a practice run works inside its 10 minutes) | Kill the harness process, not `goalcup`. | `goalcup start` prints `harness exited: ended by signal 9.` and that the run stays open until the deadline, uploads until caught up, exits 0. `status` shows `PROBLEM: the agent's process exited (…): run goalcup continue`. `goalcup continue` asks no approval, reopens the last session, adds a second entry, and leaves the deadline where it was. `--check-copy` shows two entries, the first with `final`. | `continue` asks for approval, the deadline moves, or an entry's server position differs. |
| 4 | Disk nearly full (fake ok; needs a small volume and case 1's outage) | A volume of about 5 GiB for the configuration directory and its `-artifacts` and `-runs` siblings (macOS `hdiutil create -size 5g -fs APFS -volname gcsmall /tmp/gcsmall.dmg`; Linux a loop-mounted ext4 or XFS image). Test run, network off, fill the volume to under 256 MB free. | First `PROBLEM: disk: N free beyond the reservation`. After 30 minutes offline the run stops itself ("the disk is nearly full and uploads are failing"), the harness ends, the segment seals locally, the run stays open and `status` says `run goalcup continue`. Free space, restore the network, `goalcup continue`. | The chain is torn (`goalcup-verify` exit 2), or recording fails instead of stopping. |
| 5 | Reboot, then `continue` (fake ok) | Restart the machine during a test run (also a hard power-off, on a separate run). | Nothing runs after reboot. `goalcup continue` closes the segment the dead daemon left (one `IngestLost` gap, no repair of a torn tail), sends what it can, asks for the next segment and reopens the harness. The first segment verifies with exit 1 (a torn tail is exit 2 and must be named); the second is clean. | `continue` is refused with the run open and the deadline ahead, or the first entry's server position is not the closed chain's. |
| 6 | Tokens run out (real harness) | Start with an account whose model quota is spent, and type the prompt. | The harness reports its own error and idles. The tracker keeps recording without a `PROBLEM` line. `model_requests` stops growing, so a competition run gets `idle_3h` and `idle_6h` mails (never a practice or test run). Typing "keep going" later is allowed. | The tracker stops, treats the idle agent as a recording failure, or sends the harness anything. |
| 7 | Window closed, then `continue` (fake ok; a practice run works) | Close the terminal window with the harness running. | The harness and `goalcup start` end; `pgrep -f goalcup` shows no `goalcup start`. In a new window `status` names the exited agent; `goalcup continue` opens the next segment as in case 3. | A process of the old run survives, or the first segment cannot be sealed or sent. |
| 8 | A human message other than "keep going" (real harness) | Practice or test run. After the prompt type `please also add tests`, then `keep going`. | The tracker prints and blocks nothing. `goalcup-verify --research-json <trace> --output /tmp/report.ndjson` gives a summary with `assistance.evidence: observed` and `positive_native_markers` of at least 1. | The message is missing from the trace, the report says `none_observed`, or the tracker interfered. |
| 9 | The deadline stop (fake ok; this is the matrix script) | `sh scripts/matrix.sh m2-fake-tui <server>`. | The tracer stops the harness at the deadline (`deadline: the run's time is up…`), seals, sends the final heartbeat, uploads the rest and the seal, and prints `harness termination: Confirmed`. The server shows `status: sealed`, `stop: true`, `final` = the last record. | The row says FAIL. |
| 10 | A second start replaces the first (fake ok) | Two configuration directories `a` and `b`, same member, a server with an open competition event (staging's dry-run event; runs are 168 hours, so end the second with `goalcup seal`). Start in `a`, then in `b`. | Within one heartbeat (5 minutes) `a` prints `a newer run on your account replaced this one`, stops its agent, seals and sends its final heartbeat. `GET` for the first run has `ends_at` = the second run's `official_start`, and a chunk past the first run's committed end is `410`. Two practice starts replace nothing. | The first run records on past 5 minutes after the second opened, or both stay open. |

**The long runs.** Two 8-hour runs, one on macOS and one on Linux, on staging with a real harness and an
organizer test run: start, leave, and at the end `goalcup seal`, then `--check-copy`.

## Bad days

Recording on the participant's machine never waits on the server, so a server problem costs upload time,
not data. The deadline stays on the server's clock; no switch moves it, and tracers also stop on their own
clocks.

| Symptom | Do |
|---|---|
| A bad deploy, or anything you cannot explain | Set `MAINTENANCE = "on"` in `wrangler.toml` and deploy. Every call answers `503 maintenance` with `Retry-After` (default 300 s), tracers back off, and the cron pauses. Then `npx wrangler rollback`, and set the switch back to `off`. It blocks new starts too: there is no separate pause. |
| Supabase is down or read-only | The Worker answers `502 upstream_error`; tracers retry for as long as the run lasts. Nothing is lost. Optionally switch maintenance on for a clean `503`. Check again when Supabase is back; the cron catches up. |
| R2 is down | Chunk puts answer `502`; same as above. A retried chunk is safe. |
| GitHub is down or rate-limits | Opens answer `502` (the tracer retries). The cron retries the deadline commit each tick for 48 hours; the `ended` mail waits an hour for the commit, then goes without it. Check that `GITHUB_TOKEN` is valid. |
| Many `429` on the device flow (a venue sharing one address) | Raise `DEVICE_START_PER_MINUTE`, `DEVICE_POLL_PER_MINUTE` or `DEVICE_TOKEN_PER_MINUTE` in `wrangler.toml` and deploy. Every device call answers `500 misconfigured` if the `DEVICE_LIMITER` binding or `RATE_LIMIT_KEY` is missing. |
| `500 misconfigured` on any call | Read `npx wrangler tail`; the detail is in the log, never in the answer. Usual causes: a missing secret, `APPROVAL_URL`, a test-only variable set in production. |
| A participant cannot approve a run | The website needs the current terms accepted (`403` or a message on the approval page). A start outside the window is `403` too. |
| A participant's run shows `incomplete` | Its last piece did not arrive within 24 hours of the end. It is judged on what arrived. Nothing to repair. |
| A participant lost their machine mid-run | `goalcup continue` works only on the machine that started the run. The run ends at its deadline with what was uploaded. |
| Wrong run is the main run | An account has one main run; a later competition start replaced it. `GET /v1/runs/{id}` on the first shows `ends_at` = the second's start. This is by design. |
| Emails are late | At most 20 go out per 5-minute tick. The queue drains by itself; raise `NOTICES_PER_TICK` in `mail.ts` to speed it. Check `RESEND_API_KEY` and `NOTICE_FROM` are set. |
| The signing key may be compromised | Add the new public key to `SERVER_PUBLIC_KEYS` in `crates/goalcup-verifier/src/lib.rs` (keep the old one so existing acknowledgements still verify), release the verifier, then `wrangler secret put RECEIPT_SIGNING_KEY` with a seed made offline. Avoid rotating mid-event unless the key leaked. |
| A secret leaked (Supabase, Resend, GitHub, Cloudflare) | Revoke it at its provider, create a new one, `wrangler secret put` it, deploy. Treat the old value as public. |

# Part 2: the tracer

## Building and verifying

- **Rust 1.99 or later.** `rust-toolchain.toml` pins `1.99.0`; rustup installs it on first use. The release
  workflow builds with the same pin.
- **macOS or Linux.** `goalcup-platform` refuses to compile on anything else
  (`crates/goalcup-platform/src/lib.rs`). Windows runs in WSL2.
- **A C compiler**, because `ring` compiles C and assembly.
- **`shasum` or `sha256sum`**, because a test runs `scripts/pin-manifests.sh --print`, and **`python3`**
  if you change `connectors/`.

`cargo test --workspace` is the whole suite. "Green" means `sh scripts/gate.sh fast` passes (and `full`
before a release); [README, The gate](../README.md#the-gate) has the tiers. Narrow runs:

```
cargo test -p goalcup-daemon                          # one crate
cargo test -p goalcup-daemon --test it control_api::  # one module of one crate
```

On the maintainer's machine every cargo command carries `--offline`, so run `cargo fetch --locked` once
first. The CLI tests drive the shipped `goalcup-daemon` and `goalcup-verify` binaries, and only
`--workspace` builds them; they say so by name rather than skip.

The gate skips every `#[ignore]` test (child halves, fixture regenerators, real-process and live-harness
campaigns, vertical suites). List them with `rg '^\s*#\[ignore' crates` and run one suite at a time with its
documented setup. The default gate is not proof that ignored coverage ran. The synthetic vertical suite
runs by hand after building the workspace binaries:

```sh
cargo test -p goalcup-cli --offline --features test-support --test full m2_synthetic_vertical:: -- --ignored --test-threads=1
```

Nothing in the suite reaches the network except the ignored Worker suite
([UPLOAD-API](UPLOAD-API.md)). The daemon's control API is the one listener it serves, and
`DaemonConfig::validate` refuses a non-loopback address.

### CI

`.github/workflows/ci.yml` is a Linux smoke check, **not the gate**. Its one job, `check`, runs on
`ubuntu-24.04` for every pull request into and push to `main`, and on manual starts. The `scope` step runs
`scripts/ci-scope.py`: it gives `rust=false` only when every changed path is Markdown under `docs/`, at
the root, or the PR template. Every run runs `test_ci_scope.py`, `test_check_links.py` and
`scripts/check-links.py --root .`. With `rust=true` it also runs build, fmt, clippy `-D warnings` and
`cargo test --workspace --exclude goalcup-daemon --exclude goalcup-cli`.

Keep it small. Do not add branches, jobs, a build matrix, or the daemon and CLI suites: they spawn process
trees that hang or kill a 2-core runner. Those suites run on Linux in Docker with `scripts/linux-check.sh`.
`.github/workflows/linux-x86_64.yml` is manual only; it runs that script on an x86_64 runner with
`LINUX_CHECK_HARNESSES=1`, which ends with `scripts/linux-harness-smoke.sh`. A green `ci` run proves
nothing about macOS.

## Installing the participant binaries

A clean checkout has none of `goalcup`, `goalcup-daemon` and `goalcup-verify` on `PATH`. `cargo build
--release` makes them. Either put `target/release` in front of `PATH`, or copy the three binaries and set
`GOALCUP_CONNECTORS_DIR` to this checkout's `connectors/`.

The connector directories (`harness.toml` and seeds) are the rulebook: how a harness is launched and
captured, with their digests pinned in `goalcup-protocol`. They live beside the binary, never under a
directory the participant writes. `goalcup` looks in `$GOALCUP_CONNECTORS_DIR`, then
`<exe dir>/../share/goalcup/connectors` (an installed layout), then `<exe dir>/../../connectors` (which
makes `target/release/goalcup` find this repository's `connectors/` with nothing exported). Each is tried
beside the path `goalcup` was started by and then beside its resolved path, skipping a candidate that holds a
link. A binary copied to `~/bin` matches none and refuses, naming the variable. Discovery considers
immediate child directories with a regular `harness.toml`, validates every schema-1 profile, then selects by
declared id.

`$GOALCUP_CONFIG_DIR` overrides the configuration directory for the CLI and the daemon it starts, which is
how a second, isolated tracker runs on one machine (a test, a development build beside an installed one).
The default is `~/Library/Application Support/goalcup` on macOS and `$XDG_CONFIG_HOME/goalcup` (else
`~/.config/goalcup`) elsewhere.

## Running the daemon

You normally never start it. `goalcup start`, `seal`, `status` and `entries` start `goalcup-daemon --config
$GOALCUP_CONFIG_DIR/daemon.toml --exit-when-idle` when nothing answers on `control.json`, and `goalcup
start` writes a default `daemon.toml` the first time. The README's [daemon](../README.md#the-daemon)
note says how it behaves. The daemon runs in the foreground under whatever started it, has no pidfile and
no logger, and counts what it would log on the signed status document. With `--exit-when-idle` it exits 0
once no capture is unfinished and no authenticated control request has arrived for 30 seconds.

```
goalcup-daemon --config <FILE> [--foreground] [--exit-when-idle]
goalcup-daemon --config <FILE> --print-addresses
goalcup-daemon --version
```

The configuration keys, all absolute paths except the address:

```toml
artifact_root = "/Users/you/goalcup-artifacts"   # the only tree the daemon writes outside a workspace; BESIDE config_dir
connectors_dir = "/Users/you/src/goal-cup/connectors"
config_dir = "/Users/you/Library/Application Support/goalcup"   # holds control.json; the directory the CLI reads
workspace_parent = "/Users/you/goalcup-runs"     # never nested with artifact_root (an agent lists its workspace's parent)
control_addr = "127.0.0.1:7787"                  # loopback only, or the daemon refuses to start
```

`upload_url`, `server_public_key` and `receipt_binding` are still accepted and validated so old
configurations load; nothing reads them. An optional `[event]` section (`t0`, `start_window_closes_at`,
`rules_version`, all three together) tells the daemon the round's dates; without it the signed status
document reports `event_status_source: "fallback"`. A `config_dir` that differs from the CLI's makes the
started daemon write `control.json` where the CLI is not looking, and `goalcup start` refuses, naming
`daemon.log`.

`--print-addresses` prints the control address and the `control.json` path without starting anything, so it
works while a daemon holds the lock. It never prints the bearer.

| Exit | Meaning |
|---|---|
| 0 | Stopped on SIGTERM or SIGINT, or idle under `--exit-when-idle`, or `--print-addresses` / `--version` succeeded. Neither signal seals an entry. After a restart an unfinished capture has status and seal authority only. |
| 1 | Refused, in one sentence on stderr prefixed `goalcup-daemon:`. A configuration that would not read or validate, a startup failure, or `--print-addresses` with no `control.json`. |
| 2 | clap refused the arguments. |

## Running a capture with the CLI

```
goalcup start --harness <id> [--practice | --test]   # chat mode: opens the harness in this terminal
goalcup start --harness <id> --goal-file <PATH>      # a headless harness: launches, waits, seals, prints `verify:`
goalcup status | entries | seal | continue | upload <entry>
goalcup-verify --capture-integrity <trace-dir>       # the line `start` printed
```

Every flag is in `goalcup <command> --help`. Run the `verify:` line `start` printed, not one typed from
memory. `goalcup doctor` checks, per connector, that the files are the pinned ones and finds the harness
executable on `PATH` (hashed, never started). Any build passes. `goalcup doctor --harness <id>` scopes the
report and the exit code to one connector; without it, any shipped connector that fails makes the exit 1.

Before anything is created, `goalcup start` makes one TCP connect to port 443 of each provider host the
connector intercepts, all within 10 s. A host that does not answer refuses the run and names the host and
this binary's path (a per-app firewall holds a new binary's connections until its path is allowed). During a
run, provider-host connects are retried with a 250 ms to 2 s backoff until 25 s of the 30 s admission
deadline, and are never a replayed request. Provider-host connects that still fail coalesce into outage
windows, reported at the finish as `upstream_outages=N` on the `proxy admission:` line and one `proxy
upstream outage:` line per window. The frozen v5 chain has no record kind for an outage, so the windows are
not in the trace.

`goalcup`'s exit codes:

| Exit | Meaning |
|---|---|
| 0 | Done. |
| 1 | Refused, and you can fix it: no daemon to ask or none could be started, a tracker still busy, an entry already sealed, a mistyped command, a failed preflight check. On Linux also a sealed run that left a process running. |
| 2 | Never this binary's. It is "invalid evidence", and only `goalcup-verify` leaves under it. |
| 3 | The CLI could not do its job: a file it could not read, an answer it could not parse. |

`--dev` on `start` and `prep` admits a connector directory this build does not pin; the run says so and does
not count as an entry. `goalcup-verify --dev-key <hex>` accepts a server key this build does not pin, for one
run. Neither makes a failing entry pass.

## Running the verifier

```
goalcup-verify --capture-integrity <trace> [--dev-key <hex>]
goalcup-verify --research-json <trace> --output <report.ndjson> [--dev-key <hex>]
```

`<trace>` is a protocol-v5 trace directory (`chain.jsonl` and the files beside it). `--capture-integrity`
prints the integrity report on stdout and nothing else. `--research-json` writes the private research report
to a new `*.ndjson` file outside the trace and leaves stdout empty. Notices, errors and the summary line are
stderr.

| Exit | Meaning |
|---|---|
| 0 | Complete, valid evidence. The anchor is `signed` only when a pinned server key verified an acknowledgement; otherwise it stays `local_only`. |
| 1 | Incomplete or unsealed coverage (for the research report: incomplete or partial). |
| 2 | Invalid evidence, a torn tail included. |
| 3 | No usable report: the trace could not be read, its chain is not v5, or the invocation was refused. |

A receipt or acknowledgement signed by a key this build does not pin gives `receipt_key_not_accepted` or
`ack_signature_unverified`. `--dev-key <hex>` (repeatable, 64 hex characters) accepts signatures under that
key for one run, and the report records `key_source: "supplied"` so a development run is never mistaken for a
production one.

## Pinning connectors

```
sh scripts/pin-manifests.sh
```

It recomputes the whole-directory pin table, `CONNECTOR_DIR_SHA256S`, from
`crates/goalcup-protocol/pins/connector-dirs.txt` (read by `crates/goalcup-protocol/src/pins.rs`; a malformed
file does not compile): one digest per connector directory, seeds and templates under `files/` included. The
loader hashes each connector directory and refuses a pinned connector whose bytes moved, and a v5 entry binds
the digest at create and in `Launch.connector_dir_sha256`. `goalcup doctor` runs the same comparison over the
installation.

Run it in the same commit as any change under `connectors/`. `sh scripts/pin-manifests.sh && git diff
--exit-code` proves the pins match. `--print` writes nothing and prints the directory rows.
