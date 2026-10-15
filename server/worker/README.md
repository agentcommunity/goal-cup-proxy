# Goal Cup upload Worker

The server end of `goalcup`'s trace upload: a Cloudflare Worker (TypeScript, no framework) that
checks a participant's run credential, streams zstd chunks of `chain.jsonl` into R2, records progress in
Supabase (Postgres), and returns Ed25519-signed receipts and acknowledgements in the formats the
Rust verifier already checks.

- **Wire contract (the authority):** `docs/UPLOAD-API.md`. Paths, headers, status codes, error
  bodies and idempotency rules are implemented exactly as written there.
- **Design:** `docs/SERVER-DESIGN.md`, sections 2 to 7, 11, 14 and 16.

Production is not deployed. Staging is (2026-10-15): see "Staging, as deployed" below. Local runs and
tests use `wrangler dev`, which simulates R2 on disk and touch no Cloudflare or Supabase account.

## Run it locally

Needs Node 22.18 or newer (the tests run TypeScript directly) and network access for
`npm install` only.

```sh
cd server/worker
npm install
cp .dev.vars.example .dev.vars     # test values only; .dev.vars is git-ignored
npm run dev                        # http://localhost:8787, DB_BACKEND=local, R2 simulated
npm test                           # type-check, then unit and end-to-end tests
```

`npm test` starts its own `wrangler dev` processes (on free ports, with temporary state, and with
wrangler's own config directory pointed at a temporary folder), so it needs no setup and leaves
nothing behind. It takes about 15 seconds.

Try it by hand against `npm run dev`. A credential comes from the device flow; the test approval
route (`TEST_APPROVAL_KEY` in `.dev.vars`) stands in for the website's approval page:

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
| `src/index.ts` | Routing, maintenance switch, authentication, error mapping |
| `src/device.ts` | The device flow: start, poll, and the test-only approval |
| `src/auth.ts` | Bearer run credential -> member, and the one run it may touch |
| `src/runs.ts` | Open, continue, receipt, abandon, position, heartbeat, seal |
| `src/notices.ts` | The 5-minute cron: close runs, record the GitHub commit at the deadline, queue the run emails |
| `src/chunks.ts` | The chunk upload (early refusals, retry rule, streaming to R2, atomic commit) |
| `src/sign.ts` | Ed25519 receipts and acks (byte-exact with the Rust formats) |
| `src/db.ts` | The `Db` interface: one method per step |
| `src/db-supabase.ts` | Production: one Postgres function per step, via PostgREST RPC |
| `src/db-local.ts` | `wrangler dev` and tests only: JSON state in the R2 bucket under `state/`, atomic by etag |
| `src/mail.ts` | The run emails: each queued notice sent once through Resend (cron step `sendNotices`) |
| `sql/goal-cup-tracer-upload.sql` | Copy of the website's migration that SupabaseDb calls (the website repo is the authority) |
| `test/` | `sign.test.ts` (Rust vectors), `config.test.ts`, `github.test.ts`, `supabase.test.ts` (RPC names and arguments, stubbed fetch), `local-db.test.ts`, `integration.test.ts`, `device.test.ts` and `segments.test.ts` (HTTP against `wrangler dev`), `lifecycle.test.ts` (the Worker in this process on a clock the test moves: replace, window, continue, close), `notices.test.ts` (the same rig, running the cron), `device-unit.test.ts` |

## Configuration

Variables (in `wrangler.toml` `[vars]`, or `--var` / `.dev.vars` locally):

| Name | Default | Meaning |
|---|---|---|
| `EVENT_ID` | none (required) | Event that new competition runs belong to. Lowercase letters, digits, dashes; it becomes part of every R2 key. |
| `PRACTICE_EVENT_ID` | none (required) | Event that practice runs belong to (same rules). One deployment serves one competition and one practice event for now. |
| `RUN_HOURS` | `168` | A competition run's `deadline_at` = `official_start` + this many hours. |
| `PRACTICE_RUN_MINUTES` | `10` | A practice run's `deadline_at` = `official_start` + this many minutes. |
| `DB_BACKEND` | `supabase` | `supabase` (production) or `local`. `local` is for `wrangler dev` and tests. Never set it in the deployed `wrangler.toml`. |
| `APPROVAL_URL` | placeholder in `wrangler.toml` | The website's page where a member approves a device request, without a query string. The Worker answers `verification_uri` with it and `verification_uri_complete` with `?code=<user code>` added. |
| `STAGING` | off | `on` marks a staging deployment: the only place besides `DB_BACKEND=local` where `TEST_APPROVAL_KEY` may be set. Set only in `[env.staging.vars]` of `wrangler.toml`. |
| `GITHUB_CHECK` | off | `on` makes `POST /v1/runs` ask the GitHub API that `repo_url` is a public repository (`wrangler.toml` sets `on`; `npm run dev` and the tests turn it off so they need no network). A missing or private repository is `400`; GitHub being unreachable or rate-limiting is `502`, which the tracer retries. |
| `MAINTENANCE` | off | `on` makes every call answer `503 maintenance` with `Retry-After`, before authentication and before any body is read. |
| `MAINTENANCE_RETRY_AFTER` | `300` | Seconds for that header. |
| `NOTICE_FROM` | none | Sender of the run emails, e.g. `Agent Community <updates@member.agentcommunity.org>`. Needed once `RESEND_API_KEY` is set. |
| `NOTICE_RECIPIENT_OVERRIDE` | none | Send every run email to this one address instead of the member. A staging Worker refuses to send without it (staging: `delivered@resend.dev`, Resend's test inbox). |
| `RUN_SECONDS` | none | **Tests only, honoured only with `DB_BACKEND=local`** (anywhere else the Worker answers `500 misconfigured`): competition run length in seconds, replacing `RUN_HOURS`, so a test can run a whole deadline in seconds. |
| `CLOSE_AFTER_SECONDS` | `86400` | **Tests only, same rule as `RUN_SECONDS`:** how long after a run ends it is closed (`sealed`, or `incomplete`); production: 24 hours, fixed. |
| `START_WINDOW_OPENS`, `START_WINDOW_CLOSES` | none | **Tests only, same rule as `RUN_SECONDS`:** the competition start window, RFC 3339 like `2026-11-06T09:00:00Z`; an unset edge is open, both unset means always open. In production the window is the event's row in the database. Practice has none. |

Secrets (`wrangler secret put <NAME>`; locally in `.dev.vars`):

| Name | Meaning |
|---|---|
| `RECEIPT_SIGNING_KEY` | The Ed25519 seed, 32 bytes as 64 hex characters, generated offline. Its public key is what the tracer pins. |
| `SUPABASE_URL` | `https://<project>.supabase.co` (supabase backend). |
| `SUPABASE_KEY` | A Supabase secret API key whose JWT role may only execute `public.goal_cup_upload_*` (staging: key `goalcup_ingest_staging`, role `goal_cup_ingest_staging`). Not the community-wide service key. |
| `RESEND_API_KEY` | Optional: switches the run emails on (the website's Resend account). See `NOTICE_FROM` and `NOTICE_RECIPIENT_OVERRIDE`. |
| `GITHUB_TOKEN` | Read-only token for public repositories. **Required in production and staging:** the cron asks GitHub for each run's commit at its deadline, and the unauthenticated rate limit is shared by every Worker on Cloudflare's egress addresses, so without a token those calls fail. Optional locally. |
| `TEST_APPROVAL_KEY` | **Tests and staging only.** Switches on `POST /v1/device/approve`, which approves a device request as any member (`Authorization: Bearer <key>`). Honoured only with `DB_BACKEND=local` or `STAGING=on`; set anywhere else, every call answers `500 misconfigured`. Unset, the route does not exist (`404`). Never set it in production. |

## What is checked where

- **Before a chunk body is read:** credential, run exists and is the credential's own, the entry is one of
  the run's, a chunk the server already holds (same offset, `Goalcup-Length` and `Goalcup-Head`) is
  answered `200` with the stored ack, the run is not abandoned or closed and the entry not sealed, the
  end rules below allow it (after the run's `ends_at`: `503` until the entry's final heartbeat, then
  nothing past the committed end), `Content-Length` present and at most 33,554,432, `Content-Type`
  and all seven `Goalcup-*` headers well formed, maintenance switch.
- **Then:** offset, `prev` and first sequence must continue exactly where the entry stands (`409`).
- **Storage:** the body is piped (never buffered) into R2 with a create-only put and R2 checks
  the SHA-256 (`422 digest_mismatch`, nothing stored). Key:
  `traces/{event}/{run_id}/{entry_id}/{offset:016}-{sha256}.zst`. The hash is part of the name, so
  a name can only ever hold those bytes; if the name already exists (an earlier attempt stored it
  but its database step failed) the upload continues from there.
- **Then one atomic database step** advances offset, sequence and head and records the chunk and
  its ack, only if they still match what was checked. A request that loses a race is answered
  from the stored chunk if it was a retry, otherwise `409`.
- **Not checked here, by design:** whether the bytes really are a valid chain. The verify job
  does that over the reassembled copy.

## Before launch: things only real services can prove

- **R2 checksum failure.** The Worker maps a failed `sha256` put to `422 digest_mismatch` by
  recognising the error text (`10037`, "checksum", "did not match"). That wording is proven only
  in the local simulator. On a deployed preview, upload a chunk with a wrong `Goalcup-Sha256`
  against real R2 and confirm `422` and nothing stored. If it comes back `502`, the Worker log has
  a line starting `r2-put-unrecognised-error` with the real wording: add it to the pattern in
  `src/chunks.ts`. (A recognised failure logs `r2-put-digest-mismatch`.)
- **Create-only puts** (`onlyIf: { etagDoesNotMatch: "*" }`) must refuse to overwrite on real R2,
  including under the 180-day bucket lock.
- **The proposed SQL** has only been run on a local PostgreSQL, never on Supabase/PostgREST.

## What the owner must create before deploying

Nothing below exists yet. Nothing was created, logged into, or deployed while writing this code.

1. **Cloudflare R2.** A private bucket for traces (name it in `wrangler.toml`, replacing
   `replace-me-goalcup-traces`). Turn on a **bucket lock of 180 days** and **Data Access Logs**.
   Check that a locked bucket still allows the create-only puts this Worker makes (it never
   overwrites or deletes). The Worker does not need a second bucket in production; the
   `state/` objects exist only for the local backend.
2. **The Worker and its address.** On the existing paid Cloudflare account: deploy this Worker
   and attach a custom domain such as `ingest.agentcommunity.org` (uncomment and edit `routes` in
   `wrangler.toml`). `workers_dev` is off, so there is no public `*.workers.dev` address.
3. **Secrets** (`wrangler secret put`):
   - `RECEIPT_SIGNING_KEY`, made offline (for example 32 random bytes as hex on a machine that is
     not the build machine). Keep the seed in the owner's password manager, never in a repo. The public key is then
     pinned in the tracer and verifier.
   - `SUPABASE_URL` and `SUPABASE_KEY`: the key of a dedicated Postgres role (`goalcup_worker` in
     the proposed SQL) that can execute the `goalcup.upload_*` functions and nothing else.
   - `GITHUB_TOKEN` (read access to public repositories), for the production Worker and for staging.
4. **Supabase, in the website repo** (the website repo owns the schema): done for staging on
   2026-10-15 as migration `20261015120000_goal_cup_tracer_upload.sql` (private schema
   `goal_cup_tracer`, the `public.goal_cup_upload_*` functions; copy in `sql/goal-cup-tracer-upload.sql`).
   Production still needs its own Worker role and key (the staging role may approve as any member).
   The owner also has to decide or provide:
   - the `events` rows (start window, `run_hours`) and the `EVENT_ID` the Worker
     is deployed with; the proposal checks that the event exists and that now is inside its start
     window, while `RUN_HOURS` comes from the Worker's variables;
   - where "accepted the current terms" lives (the proposal assumes `goalcup.consents`);
   - the mailer: done in this Worker (`src/mail.ts`) with the email text, address and suppression
     from the database; production needs its own send-only Resend key (staging has
     `goalcup-ingest-staging`, send-only, domain member.agentcommunity.org);
   - the approval page for device requests (it calls `goalcup.goalcup_approve_device`; only the
     `sha256` of codes and credentials is stored), and the member's handle for `member_handle`;
   - exposing schema `goalcup` to PostgREST (RPC needs it) while revoking everything from `anon`
     and `authenticated`, as the proposal does.
5. **Not built yet** (SERVER-DESIGN sections 4, 11): per-credential rate limiting (`429`; do it with a
   Cloudflare rate-limiting rule on the route), the per-run storage quota, live totals in the
   heartbeat, outages, the verify job, `live.json`, monitoring. The organizer pause on new starts is not a
   separate switch; `MAINTENANCE=on` stops everything.

## Decisions where the contract is silent

- The first record's `seq` is **0** (the baseline link of a v5 chain; this follows the contract), so
  a fresh run reports `next_seq: 0` and `head` = 64 zeros.
- A credential opens one run. `POST /v1/runs` repeated with the same `session_id` on a credential
  that already opened a run returns that run (`200`), also after the receipt; another `session_id`
  is `409`. A competition start by the account replaces its other open competition run (that run
  ends then; see `replaced_at`); a practice start replaces nothing.
- A credential sees only the run it opened: another run id, or any run id before it has opened
  one, is `404`. Unknown or expired is `401`. It lasts an hour from the approval until it opens a
  run, then until 24 hours after that run's `deadline_at`.
- The device flow's errors reuse the table's codes: a denied request is `403 forbidden`; an
  expired, unknown or already picked-up one is `410 closed`. A request lives 900 seconds from its
  creation in every state, so an approval that is not picked up in time is lost too.
- A repeated `abandon` of an already abandoned run is `200`. Abandoning a sealed run is `410`.
- A retried chunk's `200` body repeats what the original answered: that chunk's ack, the offset
  just after it, `last_seq + 1`, and its head (not the run's current position).
- `Content-Length` missing, malformed or zero is `400 malformed` (the contract's table has no 411).
  `Content-Type` other than `application/zstd` is `400`.
- A `repo_url` that GitHub says is private or missing is `400 malformed`.
- `GET /v1/runs/{id}` always answers, whatever the status. Receipts are still issued after
  `deadline_at` (nothing in the contract forbids it and nothing depends on it).
- Malformed run ids in a path (not `r` + 16 hex) are `404`; an unparseable chunk offset is `400`.
- A wrong HTTP method on a known path is `405 method_not_allowed` (not in the contract's table).
- The seal documents are stored in R2 as `seal-{sha256}.json` and `baseline-{sha256}.json` beside
  the chunks (create-only, named by hash), so a repeat is cheap and different documents can never
  overwrite them.
- Database or storage failures are `502 upstream_error` and misconfiguration is
  `500 misconfigured` (the same generic sentence for every cause: the detail, which may hold
  configuration, goes to the Worker's log only); the tracer retries any 5xx.
- A run that is over is closed the first time anyone looks at it (any call naming the run) once it
  is: `sealed` if its current entry is sealed, `incomplete` more than 24 hours after its end
  without that; it then answers `410`. The cron (below) does the same for runs nobody looks at.
  (`abandoned` is only the participant's own `abandon`.)

## The cron, as implemented

`scheduled` in `src/index.ts` runs `runTick` (`src/notices.ts`) every 5 minutes (`[triggers]` in
`wrangler.toml`). It builds the same context a request does and runs three steps, each with its own
try/catch: `closeRuns`, `recordFinalCommits`, `queueNotices`. All take the time as a parameter. They
read `Db.runsForTick(now)` (open runs, and runs that ended in the last 48 hours; never abandoned
ones) and write with `Db.closeRun`, `Db.recordFinalCommit` and `Db.queueNotices` (one call for the whole batch). The constants
(20 minutes of silence, idle at 3 and 6 hours, an hour to wait for the commit, 50 GitHub calls per
tick) are in `notices.ts` and have no variable. The rules are in `docs/UPLOAD-API.md`, "What the
server does on its own". Locally, `wrangler dev --test-scheduled` and `curl
localhost:8787/__scheduled` run one tick; the outbox rows are the objects under `state/notices/`.
Cost: each step reads the run list with one call (three in all), `queueNotices` makes one call for
the whole batch of notices however many runs there are (every open run owes `started` each tick; the
database skips what is there already), `closeRuns` makes one call per run that is due to close, and
`recordFinalCommits` makes at most 50 GitHub fetches and 50 writes. That is about a hundred
subrequests in a busy tick, not one per open run. It still assumes the paid Workers plan (the free
plan allows 50 subrequests per invocation). A run that fails in `closeRuns` or `recordFinalCommits`
is logged (`tick <step> <run_id>: <message>`) and the rest go on; a failed notice batch is logged
once (`tick queueNotices: <message>`) and the next tick sends the same batch again, which is safe
because a notice is stored once per run, kind and key.
The Worker's tests call `worker.scheduled` on a moved clock (`test/helpers/inproc.ts`, `tick`).

## Production, as deployed (2026-10-15)

- Worker `goalcup-ingest` on `https://ingest.agentcommunity.org` (custom domain; no workers.dev),
  cron every 5 minutes, no test switch (no `STAGING`, no `TEST_APPROVAL_KEY`, no recipient
  override: members get their run emails).
- R2 bucket `goalcup-traces`. Bucket lock rule `goalcup-cup-2026-180d`: objects under
  `traces/goalcup-cup-2026/` cannot be deleted or overwritten for 180 days after upload. The dry
  run (`traces/goalcup-dryrun-2026/`), practice (`traces/goalcup-practice/`) and test
  (`traces/test/`) prefixes are not locked. No lifecycle (deletion) rule yet: deleting the raw
  traces after appeals is a later owner step.
- Events (website migration `20261015150000`): `goalcup-practice` (always open),
  `goalcup-dryrun-2026` and `goalcup-cup-2026` (placeholder start windows 28 Oct and 6 Nov
  16:00 UTC + 24 h, to be confirmed), `test`. `EVENT_ID` is the dry run now: switch it to
  `goalcup-cup-2026` and redeploy after the dry run.
- Keys: Supabase secret key `goalcup_ingest_production` (role `goal_cup_ingest_production`:
  the upload functions only, never the test approval); Resend key `goalcup-ingest-production`
  (sending access, member.agentcommunity.org); `RECEIPT_SIGNING_KEY` from a seed made offline,
  whose public key is pinned in `goalcup-verifier` (`SERVER_PUBLIC_KEYS`).
- Rate limits `RL_DEVICE_START` (20/min) and `RL_DEVICE_TOKEN` (120/min) per hashed client address
  (namespaces 4101-4102; the website owns 4001-4019).

Deploy again: `CLOUDFLARE_ACCOUNT_ID=<id> npx wrangler deploy` (top level, no `--env`).

## Staging

`wrangler deploy --env staging` makes a second Worker, `goalcup-ingest-staging`, with its own
bucket and its own event ids, `STAGING = "on"` and the same cron. It is the one place besides local
tests where `TEST_APPROVAL_KEY` may exist, as a secret (`wrangler secret put TEST_APPROVAL_KEY --env
staging`). `wrangler.toml` repeats every `[vars]` entry for staging, because wrangler does not copy
them into an environment. `test/config.test.ts` reads the file and fails if the top level names
`TEST_APPROVAL_KEY` or sets `STAGING = "on"`. Unlike `vars`, wrangler *does* copy `routes` into an
environment, so `[env.staging]` sets `routes = []`: without it a staging deploy moves
`ingest.agentcommunity.org` to the staging Worker (this happened on 2026-10-15, 10:54-11:03 UTC, and
was undone by pointing the custom domain back at `goalcup-ingest`). The config test checks it.
Staging has its own rate-limit namespaces (4103-4104) with low limits.

### Staging, as deployed (2026-10-15)

- Worker `goalcup-ingest-staging` at `https://goalcup-ingest-staging.b-9a5.workers.dev` (workers.dev
  on staging only), on the Agent Community Cloudflare account; cron every 5 minutes.
- R2 bucket `goalcup-traces-staging` (no bucket lock).
- Database: the website's Supabase project, schema `goal_cup_tracer`, events
  `goalcup-practice-staging` (10-minute practice runs) and `goalcup-dryrun-staging` (168-hour runs,
  start window 15 Oct to 5 Nov 2026). The Worker's key is the secret API key `goalcup_ingest_staging`,
  whose role `goal_cup_ingest_staging` can execute the `goal_cup_upload_*` functions and nothing
  else in the schema; it is the only role granted `goal_cup_upload_approve_device` (the test
  approval).
- Secrets: `RECEIPT_SIGNING_KEY` (a fresh staging seed, generated on deploy and not kept anywhere
  else; its public key is in every receipt), `SUPABASE_URL`, `SUPABASE_KEY`, `TEST_APPROVAL_KEY`,
  `RESEND_API_KEY` (its own Resend key `goalcup-ingest-staging`: sending access only, domain
  member.agentcommunity.org), `GITHUB_TOKEN` (the website's existing read token).
- Approval page: `https://agentcommunity.org/goalcup/approve?code=XXXX-XXXX` (website).
- Emails: every notice goes to `delivered@resend.dev`, never to a member.

Deploy again from this folder with the account named: `CLOUDFLARE_ACCOUNT_ID=<id> npx wrangler deploy --env staging`.

## The end of a run, as implemented

(`docs/UPLOAD-API.md`, "When a run ends" and the heartbeat call.) A run ends at
`ends_at = min(deadline_at, replaced_at)`; every rule below is against that, per entry.

- `POST /v1/runs/{id}/heartbeat` stores the last heartbeat and answers
  `{"stop": <now >= ends_at>, "deadline_at": …, "reason": "deadline"|"replaced"|null}`. `GET`
  reports the same `stop`, `ends_at` and the entry's `final`.
- The final heartbeat commits `(seq, head, repo_head)` once per entry. The same `(seq, head)` again
  is `200` (a different `repo_head` is ignored); a different one is `409`. It is also `409` if the
  server already holds chain past that `seq`, or holds that `seq` with another head: such a final
  names a chain that is not the one stored. It is accepted until the run is abandoned or closed.
- A chunk after `ends_at` while the entry has no final committed is `503` (error code
  `awaiting_final`) with `Retry-After: 60`. Once a final is committed, before or after the end, a
  chunk ending after the committed `seq`, or ending at it with another head, is `410`. The same rule
  runs again inside the atomic commit, so a final committed during an upload wins.
- A seal needs the entry's committed end and the chain held up to it, both checked in the atomic
  seal step: no final is `409` (`503` with `Retry-After: 60` once the run is over); a chain not yet
  at the committed `(seq, head)` is `409` (the tracer reads the position and uploads the rest); `200`
  otherwise.
- There is no grace period and no late window.
- Testing a deadline takes seconds with the local-only `RUN_SECONDS` and `CLOSE_AFTER_SECONDS`.
