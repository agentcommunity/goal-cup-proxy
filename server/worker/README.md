# Goal Cup upload Worker

The server end of `goalcup`'s trace upload: a Cloudflare Worker (TypeScript, no framework). It checks
a participant's run credential, streams zstd chunks of `chain.jsonl` into R2, records progress in
Supabase (Postgres), and returns Ed25519-signed receipts and acknowledgements in the formats the Rust
verifier checks.

- **Wire contract (the authority):** [docs/UPLOAD-API.md](../../docs/UPLOAD-API.md). Paths, headers,
  status codes, error bodies and idempotency rules are implemented as written there.
- **Design:** [docs/SERVER-DESIGN.md](../../docs/SERVER-DESIGN.md).
- **Deploying, test runs, bad days:** [docs/OPERATING.md](../../docs/OPERATING.md).

## Run it locally

Needs Node 22.18 or newer (the tests run TypeScript directly) and network access for `npm install`
only. `wrangler dev` simulates R2 on disk and touches no Cloudflare or Supabase account.

```sh
cd server/worker
npm install
cp .dev.vars.example .dev.vars     # test values only; .dev.vars is git-ignored
npm run dev                        # http://localhost:8787, DB_BACKEND=local, R2 simulated
npm test                           # type-check, then unit and end-to-end tests
```

`npm test` starts its own `wrangler dev` processes (free ports, temporary state, wrangler's config
directory pointed at a temporary folder), so it needs no setup and leaves nothing behind. It takes
about 15 seconds.

Try it by hand against `npm run dev`. A credential comes from the device flow; the test approval route
(`TEST_APPROVAL_KEY` in `.dev.vars`) stands in for the website's approval page:

```sh
curl -s -XPOST localhost:8787/v1/device -H 'content-type: application/json' \
  -d '{"purpose":"run","mode":"practice","connector_id":"codex-cli","tracer_version":"0.1.0"}'
# -> device_code, user_code. Approve the user code as a member, then pick up the credential:
curl -s -XPOST localhost:8787/v1/device/approve -H 'authorization: Bearer dev-approval-key' \
  -H 'content-type: application/json' -d '{"user_code":"<user_code>","member_id":"member1","approve":true}'
curl -s -XPOST localhost:8787/v1/device/token -H 'content-type: application/json' \
  -d '{"device_code":"<device_code>"}'
# -> credential (gcr_…): send it as 'authorization: Bearer <credential>' to POST /v1/runs.
```

## Layout

| Path | What |
|---|---|
| `src/index.ts` | Routing, maintenance switch, authentication, error mapping, the cron entry |
| `src/device.ts` | The device flow: start, poll, and the test-only approval |
| `src/ratelimit.ts` | The `DeviceLimiter` Durable Object and the device-flow limits |
| `src/auth.ts` | Bearer run credential to member, and the one run it may touch |
| `src/runs.ts` | Open, continue, receipt, abandon, position, heartbeat, seal |
| `src/chunks.ts` | The chunk upload: early refusals, retry rule, streaming to R2, atomic commit |
| `src/notices.ts` | The 5-minute cron: close runs, record the GitHub commit at the deadline, queue the run emails |
| `src/mail.ts` | Sends each queued notice once through Resend |
| `src/sign.ts` | Ed25519 receipts and acknowledgements (byte-exact with the Rust formats) |
| `src/db.ts` | The `Db` interface: one method per step, and the run-state rules (`endsAt`, `statusAt`) |
| `src/db-supabase.ts` | Production: one Postgres function per step, through PostgREST RPC |
| `src/db-local.ts` | `wrangler dev` and tests only: JSON state in the R2 bucket under `state/`, atomic by etag |
| `src/context.ts`, `env.ts`, `errors.ts`, `util.ts` | Configuration, bindings, error types, helpers |
| `sql/goal-cup-tracer-upload.sql` | Copy of the website's migration that `db-supabase.ts` calls. The website repo is the authority. |
| `scripts/delete-test-runs.sh` | Deletes all organizer test runs, rows and R2 objects ([OPERATING](../../docs/OPERATING.md#test-runs)) |
| `test/` | `sign.test.ts` (Rust vectors), `config`, `github`, `supabase` (RPC names, stubbed fetch), `local-db`, `ratelimit`, `mail`, `device-unit`; `integration`, `device`, `segments` (HTTP against `wrangler dev`); `lifecycle`, `notices`, `test-mode` (the Worker in this process on a clock the test moves) |

## Configuration

Variables (in `wrangler.toml` `[vars]`, or `--var` / `.dev.vars` locally):

| Name | Default | Meaning |
|---|---|---|
| `EVENT_ID` | none (required) | Event that new competition runs belong to. Lowercase letters, digits, dashes. It becomes part of every R2 key. |
| `PRACTICE_EVENT_ID` | none (required) | Event that practice runs belong to. |
| `TEST_EVENT_ID` | none | Event that organizer test runs (mode `test`) belong to. Unset: the server opens no test run (`403`). |
| `RUN_HOURS` | `168` | A competition run's `deadline_at` = `official_start` + this many hours. |
| `PRACTICE_RUN_MINUTES` | `10` | A practice run's length. |
| `DB_BACKEND` | `supabase` | `supabase` (production) or `local` (`wrangler dev` and tests only). Never set `local` in the deployed `wrangler.toml`. |
| `APPROVAL_URL` | in `wrangler.toml` | The website page where a member approves a device request, without a query string. `verification_uri_complete` adds `?code=<user code>`. |
| `STAGING` | off | `on` marks a staging deployment: the only place besides `DB_BACKEND=local` where `TEST_APPROVAL_KEY` may be set. Set only in `[env.staging.vars]`. |
| `GITHUB_CHECK` | off | `on` makes `POST /v1/runs` ask GitHub that `repo_url` is a public repository (`wrangler.toml` sets `on`; `npm run dev` and the tests turn it off). A missing or private repository is `400`; GitHub unreachable or rate-limiting is `502`, which the tracer retries. |
| `MAINTENANCE`, `MAINTENANCE_RETRY_AFTER` | off, `300` | `on` makes every call answer `503 maintenance` with that `Retry-After`, before authentication and before any body is read. The cron does nothing while it is on. |
| `DEVICE_START_PER_MINUTE`, `DEVICE_POLL_PER_MINUTE`, `DEVICE_TOKEN_PER_MINUTE` | `120`, `30`, `1200` | Device-flow limits: starts per client address, polls per device code, polls per client address (abuse cap). |
| `NOTICE_FROM` | none | Sender of the run emails. Needed once `RESEND_API_KEY` is set. |
| `NOTICE_RECIPIENT_OVERRIDE` | none | Send every run email to this one address instead of the member. A staging Worker refuses to send without it. |
| `RUN_SECONDS`, `CLOSE_AFTER_SECONDS`, `START_WINDOW_OPENS`, `START_WINDOW_CLOSES` | none | **Tests only, honoured only with `DB_BACKEND=local`** (anywhere else the Worker answers `500 misconfigured`). Run length in seconds, seconds until a finished run closes (production: 24 hours, fixed), and the competition start window (RFC 3339; an unset edge is open). In production the window is the event's row in the database. |

Secrets (`wrangler secret put <NAME>`; locally in `.dev.vars`):

| Name | Meaning |
|---|---|
| `RECEIPT_SIGNING_KEY` | The Ed25519 seed, 32 bytes as 64 hex characters, generated offline. Its public key is pinned in the verifier. |
| `SUPABASE_URL`, `SUPABASE_KEY` | The project URL, and a Supabase secret API key whose role may only run `public.goal_cup_upload_*`. Not the community-wide service key. |
| `RATE_LIMIT_KEY` | Random, at least 32 characters: the HMAC key of every rate-limit key. A deployment answers every device call `500 misconfigured` without it. |
| `GITHUB_TOKEN` | Read-only token for public repositories. Required in production and staging: the cron reads each run's commit at its deadline, and the unauthenticated rate limit is shared by every Worker on Cloudflare's egress addresses. |
| `RESEND_API_KEY` | Switches the run emails on. Send-only. |
| `TEST_APPROVAL_KEY` | **Tests and staging only.** Switches on `POST /v1/device/approve`, which approves a device request as any member. Set anywhere else, every call answers `500 misconfigured`. Unset, the route is `404`. Never set it in production. |

`test/config.test.ts` reads `wrangler.toml` and fails if the top level names `TEST_APPROVAL_KEY` or
sets `STAGING = "on"`, or if `[env.staging]` loses `routes = []`. Wrangler copies `routes` into an
environment but not `vars`, `r2_buckets` or Durable Object bindings, so staging repeats all of them.
Without `routes = []`, a staging deploy would move `ingest.agentcommunity.org` to the staging Worker.

## What is checked where

- **Before a chunk body is read:** the credential; the run exists and is the credential's own; the
  entry is one of the run's. A chunk the server already holds (same offset, `Goalcup-Length` and
  `Goalcup-Head`) is answered `200` with the stored ack. Then: the run is not abandoned or closed, the
  entry is not sealed, the end rules allow it (after `ends_at`: `503` until the entry's final
  heartbeat, then nothing past the committed end), `Content-Length` is present and at most
  33,554,432, `Content-Type` and all seven `Goalcup-*` headers are well formed, and the maintenance
  switch is off.
- **Then:** offset, `prev` and first sequence must continue exactly where the entry stands (`409`).
- **Storage:** the body is piped (never buffered) into R2 with a create-only put, and R2 checks the
  SHA-256 (`422 digest_mismatch`, nothing stored). If the name already exists, an earlier attempt stored
  it but its database step failed, and the upload continues from there.
- **Then one atomic database step** advances offset, sequence and head and records the chunk and its
  ack, only if they still match what was checked. A request that loses a race is answered from the
  stored chunk if it was a retry, otherwise `409`.
- **Not checked here, by design:** whether the bytes are a valid chain.

## Before launch: what only real services can prove

- **R2 checksum failure.** The Worker maps a failed `sha256` put to `422 digest_mismatch` by matching
  the error text (`10037`, "checksum", "did not match"). That wording is proven only in the local
  simulator. On a deployed Worker, upload a chunk with a wrong `Goalcup-Sha256` against real R2 and
  confirm `422` and nothing stored. If it comes back `502`, the Worker log has a line starting
  `r2-put-unrecognised-error` with the real wording: add it to the pattern in `src/chunks.ts`. (A
  recognised failure logs `r2-put-digest-mismatch`.)
- **Create-only puts** (`onlyIf: { etagDoesNotMatch: "*" }`) must refuse to overwrite on real R2,
  including under the bucket lock.

## Decisions where the contract is silent

- The first record's `seq` is **0** (the baseline link of a v5 chain), so a fresh run reports
  `next_seq: 0` and `head` = 64 zeros.
- `GET /v1/runs/{id}` always answers, whatever the status. Receipts are still issued after
  `deadline_at`.
- A retried chunk's `200` body repeats what the original answered: that chunk's ack, the offset just
  after it, `last_seq + 1`, and its head (not the run's current position).
- `Content-Length` missing, malformed or zero is `400 malformed`. `Content-Type` other than
  `application/zstd` is `400`.
- A wrong HTTP method on a known path is `405 method_not_allowed`. A malformed run id in a path is
  `404`; an unparseable chunk offset is `400`.
- A repeated `abandon` of an abandoned run is `200`. Abandoning a sealed run is `410`.
- A request lives 900 seconds from its creation in every state, so a device approval that is not
  picked up in time is lost.
- Database or storage failures are `502 upstream_error`, and misconfiguration is `500 misconfigured`
  (one generic sentence for every cause; the detail, which may hold configuration, goes to the Worker's
  log only). The tracer retries any 5xx.
- A run that is over is closed the first time any call names it. The cron does the same for runs
  nobody looks at.

## The cron

`scheduled` in `src/index.ts` runs `runTick` (`src/notices.ts`) every 5 minutes. Four steps, in this
order, each in its own try/catch: `closeRuns`, `recordFinalCommits`, `queueNotices`, `sendNotices`
(`src/mail.ts`). All take the time as a parameter. They read `Db.runsForTick(now)` (open runs, and runs
that ended in the last 48 hours; never abandoned ones). The constants (20 minutes of silence, idle at 3
and 6 hours, an hour to wait for the commit, 50 GitHub calls per tick, 20 emails per tick) are in
`notices.ts` and `mail.ts` and have no variable. The rules are in
[UPLOAD-API](../../docs/UPLOAD-API.md#what-the-server-does-on-its-own).

Cost: each step reads the run list with one call. `queueNotices` makes one call for the whole batch
however many runs there are. `closeRuns` makes one call per run due to close. `recordFinalCommits` makes
at most 50 GitHub fetches and 50 writes. That is about a hundred subrequests in a busy tick, so the cron
needs the paid Workers plan (the free plan allows 50 per invocation). A failed step is logged
(`tick <step> <run_id>: <message>` or `tick queueNotices: <message>`) and the next tick retries; a notice
is stored once per run, kind and key, so a retry is safe.

Locally, `wrangler dev --test-scheduled` and `curl localhost:8787/__scheduled` run one tick. The outbox
rows are the objects under `state/notices/`. The tests call `worker.scheduled` on a moved clock
(`test/helpers/inproc.ts`, `tick`).

## The end of a run, as implemented

A run ends at `ends_at = min(deadline_at, replaced_at)`; every rule is per entry.
([UPLOAD-API, "When a run ends"](../../docs/UPLOAD-API.md#when-a-run-ends).)

- `POST /v1/runs/{id}/heartbeat` stores the heartbeat and answers `stop` once `now >= ends_at`, with the
  `reason`. `GET` reports the same `stop`, `ends_at` and the entry's `final`.
- The final heartbeat commits `(seq, head, repo_head)` once per entry. The same `(seq, head)` again is
  `200`. A different one is `409`. It is also `409` if the server already holds chain past that `seq`, or
  holds that `seq` with another head. It is accepted until the run is abandoned or closed.
- A chunk after `ends_at` while the entry has no final committed is `503 awaiting_final` with
  `Retry-After: 60`. Once a final is committed, a chunk ending after the committed `seq`, or ending at it
  with another head, is `410`. The same rule runs again inside the atomic commit.
- A seal needs the committed end and the chain held up to it, both checked in the atomic seal step: no
  final is `409` (`503` once the run is over), a chain not yet at the committed end is `409`.
- To test a deadline in seconds, use the local-only `RUN_SECONDS` and `CLOSE_AFTER_SECONDS`.
