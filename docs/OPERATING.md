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
  [Running the verifier](#running-the-verifier), [Pinning connectors](#pinning-connectors),
  [Finding codes](#finding-codes)

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
| 23–24 Oct | First release (`scripts/release.sh prepare`, merge, `scripts/release.sh tag`) |
| 23 Oct to 6 Nov 00:00 | Dry-run window (production, `goalcup-dryrun-2026`). Dry runs can start on any day. |
| 4 Nov 00:00 | Feature stop: fixes only |
| 6 Nov | `EVENT_ID` switch and last release (12:00). See [EVENT_ID switch](../server/worker/README.md#event_id-switch-6-nov-2026-with-the-last-release). |
| 7 Nov 00:00 | Start window opens. It lasts 24 hours. |
| 8 Nov 00:00 | Start window closes |
| 168 hours after each start | That run's deadline. The latest run ends 192 hours after the window opens. |

Before the window opens:

1. The `goalcup-cup-2026` row in the website's `events` table has `start_window_opens` =
   `2026-11-07T00:00:00Z` and `start_window_closes` = `2026-11-08T00:00:00Z`. This is applied in
   production. The dry-run row closes on 6 Nov, 00:00.
2. Do the `EVENT_ID` switch on 6 Nov. The steps are in
   [server/worker/README.md](../server/worker/README.md#event_id-switch-6-nov-2026-with-the-last-release).
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

A `v*` tag that equals the workspace version builds four tarballs, `SHA256SUMS` and `install.sh`
(participants install with it). Steps and layout are in [packaging/README.md](../packaging/README.md).
The binaries are unsigned and not notarized. Run `sh scripts/gate.sh full` before every tag. A release
tarball carries every folder under `connectors/`; a merged connector ships by being merged, but the
pins are compiled in, so a connector merged after a release needs a point release.

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
`goalcup start --practice` in them, waits for the run to end, and deletes the temporary directory. It reads
the connector's `harness.toml` to tell the two kinds apart. A chat-mode connector (`interactive = true`) runs
under a pseudo-terminal into which the script types one prompt; the tracer stops it at the practice deadline
(10 minutes). A headless connector (like `hermes-cli`) runs with `--goal-file` holding `MATRIX_PROMPT`, with
nothing typed; it ends when the harness does, and the script then waits for the server's deadline before it
asks for the server's copy. `--timeout` is one budget for both waits together. A connector id that is not one of
the two fakes needs its `harness.toml` to be found (see Connectors below), or the row FAILs and says so.
It prints the log path and one row:

```
<connector> <os>-<arch> <harness_version> PASS|FAIL <date>
```

`<os>` is `macos`, `linux` or `wsl2`; `<arch>` is `arm64` or `x86_64`. A FAIL names its first reason on
stderr and in the log. PASS means:

1. both approvals (the login, then the run) were given, the run opened, and the harness was up;
2. the run ended: a chat-mode run at its deadline (the tracer's `deadline:` line, which it writes to the
   run's `host.log`, not the terminal, so the script reads both), a headless run at its deadline or with
   `harness exited: exit code 0.`; and `uploaded: … and the seal` in both;
3. `goalcup-verify --capture-integrity <trace>` exited 0 with `integrity: valid` and `anchor: signed`;
4. the server's copy agrees with the local chain.

**The server copy.** The upload API has no call that returns the stored chain, so the script does not
download it. It asks `GET /v1/runs/{id}?entry_id=…` with the run's own credential and requires the answer
to equal the local chain the verifier just checked: `offset` equal to the bytes of `chain.jsonl`,
`next_seq` to the record count, `head` and the last acknowledgement's `hash` to the verifier's
`validated_head`, `final` to the last record, `status: sealed` and `stop: true` (for a headless run, once the server's deadline
has passed: the script polls until then, within what is left of `--timeout`). The server stores a chunk
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
- `goalcup` is started with `GOALCUP_NO_BROWSER=1`, so it opens no browser page per approval. (That variable
  is a switch of the shipped `goalcup`: set it and `goalcup login` and `goalcup start` print the address and
  code and open nothing.)
- A provider key comes from a file too: `MATRIX_KEY_ENV_FILE=<path>`, a mode 0600 file with one
  `DEEPSEEK_API_KEY=` line. When the connector's `harness.toml` declares that credential (`hermes-cli`), the
  start goes through `scripts/with-deepseek-key.sh`, which reads the key itself. The key is never in this
  script's environment, arguments, log or files. The path is not printed or written to a file: it is in the
  start wrapper's environment and in the loader's argument list (a same-user `ps` can see the path, never the
  key). **Today a keyed connector is refused up front:** a harness given a provider key is never uploaded (RULES),
  so the real `goalcup start --practice` refuses it, and `matrix.sh` says "keyed harnesses are not uploaded today
  (RULES); matrix cannot test them" before it starts anything. The plumbing stays behind `MATRIX_KEYED_UPLOADED=1`
  for when that rule changes. The key file is ignored for a connector with no such credential.
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
For a chat-mode connector the script types `MATRIX_PROMPT` once, `MATRIX_PROMPT_DELAY` seconds (30) after the
harness is up, and sends the Enter `MATRIX_SUBMIT_DELAY` seconds (2) later in a write of its own: Codex reads
text and Enter in one write as a paste and leaves the text in its composer. A first-run dialog that would
swallow the prompt can be answered first with `MATRIX_PRE_PROMPT_KEYS` (a `printf` format sent
`MATRIX_PRE_PROMPT_DELAY` seconds, default 5, after the harness is up). Claude Code asks "Is this a project you
created or one you trust?" in every new workspace and selects "No, exit" first, so
`MATRIX_PRE_PROMPT_KEYS='\033[B\r'` (Down, Enter) answers "Yes, I trust this folder". In real-harness mode a
PASS does not prove the prompt was submitted; check the session in the kept directory (`--keep`) or the
trace. `m2-fake-tui` (chat) and `m2-fake-headless` (headless, a fake that exits at once with code 0) are the
two that need no harness.

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
| A bad deploy, or anything you cannot explain | Switch maintenance on with `npx wrangler deploy --var MAINTENANCE:on` from a clean checkout of the commit that is live now ([steps](../server/worker/README.md#maintenance-on-and-off)); do not edit `wrangler.toml`. Every call answers `503 maintenance` with `Retry-After` (default 300 s), tracers back off, and the cron pauses. Any deploy during maintenance must repeat `--var MAINTENANCE:on`, or maintenance ends. Then fix the cause or `npx wrangler rollback <version-id>`, and switch it off by a deploy without the flag. It blocks new starts too: there is no separate pause. |
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
`LINUX_CHECK_HARNESSES=1`, which ends with `scripts/linux-harness-smoke.sh`. That script drives each
connector the way its manifest says: a headless connector gets `--goal-file`; a chat-mode connector
(`[capture] interactive = true`, as `codex-cli` and `claude-code-cli` are) gets a terminal of its own,
one typed prompt, then a hangup, and the run still seals. A green `ci` run proves
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

`$GOALCUP_NO_BROWSER` set to any non-empty value makes `goalcup login` and `goalcup start` open no browser. The
approval address and code are printed as always, with a line saying a browser did not open. It is for a headless
or remote machine and for scripts that approve another way (`scripts/matrix.sh` sets it).

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

The daemon refuses a key it does not know, so a `daemon.toml` that still sets a retired V1 key
(`upload_url`, `server_public_key`, `receipt_binding`, a cadence such as `poll_interval_ms`, or an
`[event]` section) does not load: delete it. The round's dates are the upload server's. A `config_dir` that differs from the CLI's makes the
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

This build pins the production ingest key. A production trace verifies without any option, and the report says
`key_source: "pinned"`. A receipt or acknowledgement signed by a key this build does not pin (staging, a test
signer) gives `receipt_key_not_accepted` or `ack_signature_unverified`. `--dev-key <hex>` (repeatable, 64 hex characters) accepts signatures under that
key for one run, and the report records `key_source: "supplied"` so a development run is never mistaken for a
production one.

### Judging a run from the server's copy

A participant's folder may have been pruned (`chain_pruned_locally`), or you may hold only the server's
copy. `scripts/fetch-run.sh` rebuilds a trace directory from the database's chunk index and the R2 bucket:

```sh
sh scripts/fetch-run.sh <run-id | entry-id> <outdir>      # prints <outdir>/<entry-id> for each entry
goalcup-verify --capture-integrity <outdir>/<entry-id>
```

A run id builds one directory per entry (a continued run has several). The chunks come from the database's
chunk index, never from an R2 listing, because a lost race can leave an orphan object. The script checks
that every chunk matches the index's SHA-256 and length and that the offsets run on from 0 with no gap, and it
writes nothing into `<outdir>` unless the entry is whole. It never overwrites a directory. It writes
`chain.jsonl`, `acks.jsonl`, `receipt.json` and, for a sealed entry, `seal.json` and `baseline.json`.

It needs `zstd`, `python3`, `psql` and `curl`, and in the environment `DATABASE_URL`, `CLOUDFLARE_ACCOUNT_ID` and
`CLOUDFLARE_API_TOKEN`. Use read-only credentials: a Postgres role that can only `SELECT` from
`goal_cup_tracer.entries` and `goal_cup_tracer.chunks` (and has `USAGE` on the schema, and `BYPASSRLS` or a
policy), and an R2 token with object read access only. The script runs its one query inside `BEGIN READ ONLY`
and only sends `GET` requests to R2. It prints no credential (not under `sh -x` either) and puts none on a
command line: the database URL goes to psql in a mode-600 service file, the token to curl on its stdin. Files
it writes are private to you (`umask 077`). It also checks the index reaches the end the server holds
(`upload_offset`, and the final heartbeat's seq and head when one was sent). `--bucket
goalcup-traces-staging` reads the staging bucket, and a staging or test trace also needs `--dev-key`.
`--index FILE` and `--store DIR` replace the database and R2 with a saved index and a mirror of the bucket.
`scripts/test_fetch_run.py` tests it against such a fake.

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

## Finding codes

`goalcup-verify --capture-integrity` lists the codes it found in `findings`, in the order it found them, and
the stderr summary repeats them as `findings: a,b,c` (`none` when there are none). The chain scan stops at
the first invalid line, so the first code it adds is the cause. (`receipt_*` codes are added before the scan
starts.) Codes after it only say what the validated prefix lacks (for example `launch_missing` after a
dropped line). `validated_records` and `validated_head` say where the good prefix ends.

**Who and where.** The report (schema 3) names the run and the first problem, so a batch can join it to the
server's rows and a judge need not infer where the scan stopped. `entry_id` is the entry the validated
prefix belongs to. `profile_id` is the connector id of its first `Launch` (`null` if the prefix holds none).
`first_failure` is `{"seq", "code"}`: for an `invalid` report the first violation, for an `incomplete` one the
first missing or unproven piece (`chain_unsealed`, `seal_manifest_missing` and the like), and `null` for a
`valid` one or an incomplete one that is only about coverage (`gap_recorded`, `producer_recovered`,
`producer_unclosed`). `seq` is the position the bad record should have had, which is also
`validated_records` when the chain starts at 0. It is `null` for a problem in a document (the receipt,
`seal.json`, `baseline.json`), in the chain as a whole, or on the first record of a chain pruned locally (its
position is only its own claim). "First" is the order the verifier looks, not the lowest seq: the receipt, then
the records in seq order, then the documents. A bad receipt is therefore named even when a record also breaks;
`findings` lists both. A batch is then
`for d in runs/*; do goalcup-verify --capture-integrity "$d" | jq -c '{entry_id, profile_id, integrity, first_failure}'; done`.

**Deadline facts.** The report (schema 2 and later) also has `records_after_deadline` and
`last_seq_at_or_before_deadline`. They compare each record's `recorded_at` with the receipt's `deadline_at`.
`recorded_at` is the **participant's machine clock**: the chain hash covers it, but nothing makes it true, and a
participant can set the clock back. Treat the two fields as a hint, not proof. They never change the exit.
A run finished early has `records_after_deadline` 0. A run cut at its deadline usually has its cutoff and
finalization after it, but a record in the deadline's own second counts as before it. Both fields are empty
(`null`) without a verified receipt. The **trusted time is the server's**: `last_ack_at` (the same value as
`verified_ack.acked_at`, signed with the server key) compared with `receipt_clock.deadline_at`, and the
server's own chunk acknowledgements. The research summary (`--research-json`, since schema 3) repeats the three
fields under `delivery`, unchanged, with the same caveat.

The **Exit** column is the exit a code gives alone: 2 invalid evidence, 1 incomplete (not proven, not
forged), `-` a note that changes nothing by itself. Exit 2 beats exit 1. When a trace is invalid, compare it
with the server copy; do not repair it. When it is incomplete, check whether the run simply did not finish.
More than 128 distinct codes end with `findings_omitted:<n>`.

**The chain and its links**

| Code | Exit | Meaning |
|---|---|---|
| `torn_tail` | 2 | The chain ends in a line with no newline. The prefix before it is reported. |
| `envelope_too_large` | 2 | One line is longer than the line bound. |
| `record_malformed` | 2 | A line is not a readable record. |
| `record_not_canonical` | 2 | A line decodes, but is not the exact compact form of its record (extra key, spaces, CR). The hash covers none of those bytes. |
| `protocol_version_unsupported` | 2 | A record names a protocol version this build does not know. |
| `capture_envelope_invalid` | 2 | A line breaks the v5 envelope rule: a retired pre-v5 version, redaction on the envelope, or a first record that is not the baseline. |
| `link_seq_mismatch` | 2 | A line's `seq` is not the one after the line before. A line was dropped, repeated or moved. |
| `link_prev_mismatch` | 2 | A line's `prev` is not the hash of the line before. A line came from another chain, or one was removed. |
| `link_hash_mismatch` | 2 | A line's hash is not the hash of its content. The line was edited. |
| `chain_phase_invalid` | 2 | A record is out of order: after the terminal, a second baseline, a signed terminal with no cutoff, or a chain that ends in the V1 seal (no v5 writer produces one). |
| `entry_id_mismatch` | 2 | A line names another entry than the first line. |
| `chain_part_missing` | 2 | A part file is missing between two others, or part names do not join up. |
| `chain_pruned_locally` | 1 | The first parts of the chain were removed on this machine. Verify the server copy. |
| `chain_unsealed` | 1 | The chain has no terminal record. The run did not finish, or the machine died first. |

**Receipt, cutoff and acknowledgements**

| Code | Exit | Meaning |
|---|---|---|
| `receipt_too_large`, `receipt_malformed` | 2 | `receipt.json` is too big, or does not parse (this includes a bad run key). |
| `receipt_key_not_accepted` | 2 | The receipt names a server key this build does not pin. Use `--dev-key` only for a test server. |
| `receipt_signature_invalid` | 2 | The signature does not verify under the key. |
| `receipt_clock_invalid` | 2 | `started_at` or `deadline_at` is not a time, or the start is after the deadline. |
| `receipt_entry_mismatch` | 2 | The receipt is for another entry. |
| `receipt_chain_head_unbound` | 2 | The receipt's `chain_head` is not a link of this chain. (Not raised on a chain pruned locally.) |
| `signed_terminal_receipt_missing` | 1 | The chain ends with a signed finalization, but no `receipt.json` was kept. |
| `cutoff_receipt_mismatch` | 2 | The cutoff record names another receipt than the one kept. |
| `cutoff_deadline_mismatch` | 2 | The cutoff record names another `deadline_at` than the receipt. This binds the stop to the 168-hour deadline. |
| `ack_too_large`, `ack_torn_tail`, `ack_malformed` | 2 | A line of `acks.jsonl` is too long, has no newline, or does not parse. |
| `ack_time_invalid` | 2 | An `acked_at` is not a time. |
| `ack_not_monotonic` | 2 | Acknowledgements go back in `seq` or in time. |
| `ack_entry_mismatch` | 2 | An acknowledgement is for another entry. |
| `ack_hash_mismatch` | 2 | The acknowledged hash is not the chain's hash at that `seq`. The chain changed after the server saw it. |
| `ack_signature_invalid` | 2 | The signature does not verify under the key that anchored the trace. |
| `ack_signature_unverified` | 2 | No accepted key verifies the signature. |
| `ack_beyond_chain` | 2 | An acknowledgement is for a `seq` the chain does not hold. The chain was cut short. |
| `server_key_unpinned` | - | A supplied `--dev-key` verified the trace. `key_source` is `supplied`. |

**Baseline, seal and launch**

| Code | Exit | Meaning |
|---|---|---|
| `baseline_manifest_missing` | 1 | `baseline.json` is absent or empty. |
| `baseline_manifest_malformed`, `baseline_manifest_too_large` | 2 | `baseline.json` does not parse, or is too big. |
| `baseline_manifest_mismatch` | 2 | `baseline.json` does not match the digest, file count or entry in the baseline record. |
| `seal_manifest_missing` | 1 | `seal.json` is absent or empty (a crash between the terminal record and the file). |
| `seal_manifest_malformed`, `seal_manifest_too_large` | 2 | `seal.json` does not parse, or is too big. |
| `seal_manifest_mismatch` | 2 | `seal.json` does not match the terminal record. |
| `baseline_git_unreadable`, `seal_git_unreadable` | 1 | Git could not be read in the workspace at that point. The report's `git_unreadable` says why. |
| `seal_git_missing` | 1 | The baseline recorded Git state, but the seal did not. |
| `launch_missing` | 1 | The chain has no `Launch` record. |
| `launch_goal_unbound` | 1 | A `Launch` names a goal stream that did not end `Complete`. |

**Coverage notes** (each one makes the exit 1 through `coverage_incomplete`, but is not damage)

| Code | Exit | Meaning |
|---|---|---|
| `gap_recorded` | 1 | The tracer recorded a gap (lost bytes or an unavailable source). Gaps stay recorded. |
| `producer_recovered` | 1 | A producer was replaced after a loss. |
| `producer_unclosed` | 1 | A producer never closed. Its tail is unknown. |
| `recovery_names_unrecorded_producer` | - | A recovery named a producer that left no record. The recovery already gives exit 1. |

**Record rules** (all exit 2: a record breaks a rule of its own)

| Code | Meaning |
|---|---|
| `record_field_invalid` | A field fails its format or bound: an id, a time, a text, a `Launch` field. |
| `producer_limit_exceeded`, `launch_limit_exceeded`, `goal_stream_limit_exceeded`, `open_stream_limit_exceeded`, `connection_limit_exceeded` | A per-entry bound is passed. |
| `record_after_producer_closed`, `record_from_lost_producer` | A record comes from a producer that closed, or that a recovery replaced. |
| `producer_closed_twice`, `producer_closed_with_open_streams`, `producer_closed_final_seq_inconsistent` | A close is repeated, leaves streams open, or claims a final `seq` below the records kept. |
| `recovery_producer_not_fresh`, `recovery_names_non_wire_producer`, `recovery_after_producer_closed`, `producer_recovered_twice` | A recovery breaks the rule that it replaces one unclosed wire producer, once. |
| `connection_reobserved_outside_recovery`, `connection_already_open`, `connection_not_open` | A connection opens twice, is seen again outside a recovery, or is used or closed when not open. |
| `stream_id_not_increasing`, `stream_not_open`, `gap_on_unknown_stream` | A stream id goes back, a record names a stream not open, or a gap names a stream never opened. |
| `segment_time_reversed`, `segment_part_out_of_order`, `segment_range_invalid` | A segment's times, part number or byte range cannot be right. |
| `segment_source_overlap`, `segment_source_unexplained_hole` | A segment overlaps the last one, or skips bytes with no gap recorded. |
| `segment_data_too_large`, `segment_data_not_base64`, `segment_audit_out_of_range`, `segment_length_mismatch` | A segment's data is too big, is not base64, has a redaction span outside its range, or has the wrong length. |
