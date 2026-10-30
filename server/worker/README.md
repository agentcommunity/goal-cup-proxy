# Goal Cup upload Worker

The server end of `goalcup`'s upload: a Cloudflare Worker (TypeScript, no framework). It checks a
participant's run credential, streams the record's compressed chunks into R2, records progress in
Supabase (Postgres), owns the run's clock, and seals an entry once it holds the whole chain. It keeps
bytes; it does not judge them.

- **Wire contract (the authority):** [docs/UPLOAD-API.md](../../docs/UPLOAD-API.md). Paths, headers,
  status codes, error bodies and idempotency rules are implemented as written there. This page does
  not repeat them.
- **Deploying, test runs, bad days:** [docs/OPERATING.md](../../docs/OPERATING.md). The short steps for
  operators are in the [Operator runbook](#operator-runbook) below.

## How it is built

The server must keep every byte a participant uploads (a bad hour for their network or for the server
loses nothing), own the clock (it stamps the start, sets the deadline and says stop; the
participant's clock cannot extend a run), and tell the participant when a run goes quiet, goes idle,
has a recording problem or has ended. Recording never waits on the network.

| Part | What it does |
|---|---|
| Worker `goalcup-ingest` | Every call under `/v1`. Holds no state. Streams chunks into R2. Runs a cron every 5 minutes. |
| R2 | The chunks, one immutable object each. Private. |
| Supabase (Postgres) | Runs, entries, chunk index, heartbeats, notices, device requests, credentials (hashes only). The Worker calls one SQL function per step. |
| `DeviceLimiter` (Durable Object) | Counts device-flow calls per minute. |
| Resend | Sends the run emails. |
| GitHub API | Checks `repo_url` at open; reads the judged commit after the deadline. |
| Website (another repo) | Owns the database schema and migrations, sign-up, terms, the approval page and the member's address. |

**A byte store.** The Worker never decompresses, never parses a chunk and never holds a chunk body in
memory: it checks the headers, pipes the body into R2 and lets R2 check the SHA-256. A chunk is a byte
range of a compressed part file and continues its zstd frame, so it is not decodable alone, and a
128 MB isolate could not hold the decoder's window anyway. Rule for every later change: no
server-side reading runs in a Worker. Whether the bytes are a valid record is checked on the fetched
copy, on an organizer machine (`scripts/fetch-run.sh`, then `goalcup-verify check --server`).

**Storage.**

- Key: `traces/{event}/{run_id}/{entry_id}/{offset:016}-{sha256}.zst`, offset being the raw stream
  offset of the chunk's first byte. `Content-Type: application/zstd`.
- Every put is create-only and R2 checks the SHA-256, so a key holds only those bytes. A chunk is at
  most 32 MiB compressed; any JSON body at most 64 KiB.
- There is no per-run size limit. The database counts `stored_bytes` and `chunk_count` per entry in the
  step that commits a chunk, and the log line `large-run <run_id> stored=<bytes>` marks each 10 GiB a
  run crosses.
- The bucket lock (180 days) covers the competition prefix only (`traces/goalcup-cup-2026/`). Dry run,
  practice and test prefixes are not locked. No lifecycle rule deletes objects.

**The run.** Sign-in is the device flow; the server stores only SHA-256 hashes of the codes and the
credential. A run credential is unbound until `POST /v1/runs` binds it to the run it opens, then
reaches that run only; it lives one hour while unbound and until `deadline_at` plus 24 hours once
bound. The open stamps `official_start` from the server's clock and sets `deadline_at`. One account
has one main run: a new competition start replaces the old one. Practice and test runs replace
nothing and are never replaced; test runs are left out of every public number. `goalcup continue` adds a segment (a new entry with its own chain,
same deadline). How an entry is sealed: [The end of a run](#the-end-of-a-run-as-implemented).
24 hours after the run's end the server closes it: `sealed` or `incomplete`, then `410`
(`CLOSE_AFTER_SECONDS` changes this only in tests).

**One database for staging and production.** `sql/goal-cup-tracer-upload.sql` is a copy of the
website's migration; the website repo is the authority. Production and staging each have their own
Supabase secret key and role, which may run only the `goal_cup_upload_*` functions; row-level
security is on for every table. Both Workers write the same tables. Runs, run credentials and device
requests carry a `staging` column, set by the database from the caller's role (a Worker cannot claim
it), so **every organizer query filters `staging`**.

**What the server stores about a person.** Hashes of the codes and credential, never the plain
values. The machine facts sent at open (operating system, architecture, CPUs, memory, disk, WSL2,
container, mains power; no host name, user name or serial number). The IP address and country of the
open call and the latest pair from each `continue` and heartbeat. Each heartbeat's counts of what the
client wrote and lost. None of this is in an answer, a key, a log or a public page. The Worker does
not redact: goalcup masks secrets on the participant's machine before any byte reaches disk. Raw
records are never public.

**Read-only access for organizers.** The read-only database credential and an R2 token with Object
Read only (R2 tokens scope to buckets, not prefixes) live on the organizer's machine, never in a
build box, a log or a command line. Every organizer script opens `BEGIN READ ONLY` and ends
`ROLLBACK`, as `scripts/fetch-run.sh` does. `scripts/delete-test-runs.sh` writes, so only the owner
runs it.

**Website database: nothing to migrate in 1.0.** The 1.0 Worker fits the tables as they are:

| 1.0 | How it fits |
|---|---|
| No signed chunk answer | `{}` in `chunks.ack` (NOT NULL) and `entries.last_ack` |
| No seal documents | the final head in both seal digest columns, null keys |
| Heartbeat counts | `heartbeats.vitals` and `runs.last_vitals` hold `{"counts": ...}`; the column keeps its name. A `counts` that is not a flat object within 2 KiB is stored as `{"counts_dropped": <bytes>}` with the log line `counts-dropped <run_id>`, never refused |
| No disk pause | `runs.disk_paused_at` and `disk_notice_at` stay null |
| One connector digest | `connector_sha256` written into both digest columns |
| 24-hour close, credential life | unchanged |

Old SQL functions the 1.0 Worker no longer calls stay in the mirror, unused. Only the owner applies a
migration to the shared database.

**Usage stats (third slice, not built yet).** An organizer job writes `usage/{event}/{run}/{entry}.json`
to a bucket of its own, `goalcup-usage` (`goalcup-usage-staging` during the build), never the traces
bucket: the job needs write access and R2 tokens scope only to whole buckets. The Worker neither binds
nor serves it. The owner creates `goalcup-usage` with the production deploy. Test runs' usage objects go
with [Clean up test runs](#clean-up-test-runs).

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
  -d '{"purpose":"run","mode":"practice","connector_id":"codex-cli","proxy_version":"x.y.z"}'
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
| `src/runs.ts` | Open, continue, position, heartbeat, and the seal (`trySeal`) |
| `src/chunks.ts` | The chunk upload: early refusals, retry rule, streaming to R2, atomic commit, the seal on the completing chunk |
| `src/notices.ts` | The 5-minute cron: close runs, record the judged GitHub commit, queue the run emails |
| `src/github.ts` | The GitHub API: the judged commit (last push before the deadline) and the headers every GitHub call carries |
| `src/mail.ts` | Sends each queued notice once through Resend |
| `src/db.ts` | The `Db` interface: one method per step, and the run-state rules (`endsAt`, `statusAt`) |
| `src/db-supabase.ts` | Production: one Postgres function per step, through PostgREST RPC |
| `src/db-local.ts` | `wrangler dev` and tests only: JSON state in the R2 bucket under `state/`, atomic by etag |
| `src/context.ts`, `env.ts`, `errors.ts`, `util.ts` | Configuration, bindings, error types, helpers |
| `sql/goal-cup-tracer-upload.sql` | Copy of the website's migration that `db-supabase.ts` calls. The website repo is the authority. |
| `sql/pending/` | A migration written here first, until it is applied (with a README that says how to land it). Empty since 27 Oct 2026; `git show 32808ff8:server/worker/sql/pending/README.md` is the last such README |
| `scripts/check-sql.sh` | Applies the mirror and the pending migration to a throwaway local Postgres and smoke-calls the new functions (needs a file in `sql/pending/`: exits 2 while it is empty) |
| `scripts/delete-test-runs.sh` | Deletes all organizer test runs, rows and R2 objects in the traces and usage buckets ([OPERATING](../../docs/OPERATING.md#test-runs)) |
| `test/` | `config`, `github`, `supabase` (RPC names, stubbed fetch), `local-db`, `ratelimit`, `mail`, `device-unit`; `integration`, `device`, `segments` (HTTP against `wrangler dev`); `lifecycle`, `notices`, `test-mode` (the Worker in this process on a clock the test moves) |

## Configuration

Variables (in `wrangler.toml` `[vars]`, or `--var` / `.dev.vars` locally):

| Name | Default | Meaning |
|---|---|---|
| `EVENT_ID` | none (required) | Event that new competition runs belong to. Lowercase letters, digits, dashes. It becomes part of every R2 key. |
| `PRACTICE_EVENT_ID` | none (required) | Event that practice runs belong to. |
| `TEST_EVENT_ID` | none | Event that organizer test runs (mode `test`) belong to. Unset: the server opens no test run (`403`). |
| `RUN_HOURS` | `168` | A competition run's `deadline_at` = `official_start` + this many hours. |
| `PRACTICE_RUN_MINUTES` | `10` | A practice run's length. |
| `MIN_COMPETITION_VERSION` | empty (no minimum) | The oldest goalcup (`x.y.z`; the first three numbers of what it sends) that may sign in for, or open, a run in any mode. Older, or a version with no `x.y.z` while a minimum is set, is `403` with the update line. A value with no `x.y.z` in it is malformed (parsing is lenient: the first `x.y.z` counts) and makes those two calls `500 misconfigured`; `continue`, heartbeats and everything else are untouched. `wrangler.toml` sets `1.0.0` in both blocks ([steps to raise it](#set-the-minimum-competition-version)). |
| `DB_BACKEND` | `supabase` | `supabase` (production) or `local` (`wrangler dev` and tests only). Never set `local` in the deployed `wrangler.toml`. |
| `APPROVAL_URL` | in `wrangler.toml` | The website page where a member approves a device request, without a query string. `verification_uri_complete` adds `?code=<user code>`. |
| `STAGING` | off | `on` marks a staging deployment: the only place besides `DB_BACKEND=local` where `TEST_APPROVAL_KEY` may be set. Set only in `[env.staging.vars]`. |
| `GITHUB_CHECK` | off | `on` makes `POST /v1/runs` ask GitHub that `repo_url` is a public repository (`wrangler.toml` sets `on`; `npm run dev` and the tests turn it off). A missing or private repository is `400`; so is a repository that already has a commit (it must be empty when the run opens: `GET /repos/<owner>/<repo>/commits?per_page=1` answers `409` for an empty one); GitHub unreachable, rate-limiting or answering anything else is `502`, which goalcup retries. A practice open never asks: a practice run judges nothing. |
| `MAINTENANCE`, `MAINTENANCE_RETRY_AFTER` | off, `300` | `on` makes every call answer `503 maintenance` with that `Retry-After`, before authentication and before any body is read. The cron does nothing while it is on. |
| `DEVICE_START_PER_MINUTE`, `DEVICE_POLL_PER_MINUTE`, `DEVICE_TOKEN_PER_MINUTE` | `120`, `30`, `1200` | Device-flow limits: starts per client address, polls per device code, polls per client address (abuse cap). |
| `NOTICE_FROM` | none | Sender of the run emails. Needed once `RESEND_API_KEY` is set. |
| `NOTICE_RECIPIENT_OVERRIDE` | none | Send every run email to this one address instead of the member. A staging Worker refuses to send without it. |
| `RUN_SECONDS`, `CLOSE_AFTER_SECONDS`, `START_WINDOW_OPENS`, `START_WINDOW_CLOSES` | none | **Tests only, honoured only with `DB_BACKEND=local`** (anywhere else the Worker answers `500 misconfigured`). Run length in seconds, seconds until a finished run closes (production: 24 hours, fixed), and the competition start window (RFC 3339; an unset edge is open). In production the window is the event's row in the database. |

Secrets (`wrangler secret put <NAME>`; locally in `.dev.vars`):

| Name | Meaning |
|---|---|
| `SUPABASE_URL`, `SUPABASE_KEY` | The project URL, and a Supabase secret API key whose role may only run `public.goal_cup_upload_*`. Not the community-wide service key. |
| `RATE_LIMIT_KEY` | Random, at least 32 characters: the HMAC key of every rate-limit key. A deployment answers every device call `500 misconfigured` without it. |
| `GITHUB_TOKEN` | Read-only token for public repositories. Required in production and staging: the cron reads each run's judged commit (repository, activity and possibly `commits/HEAD`), and the unauthenticated rate limit is shared by every Worker on Cloudflare's egress addresses. Without it the cron logs `WARNING: GITHUB_TOKEN is not set` and carries on. |
| `RESEND_API_KEY` | Switches the run emails on. Send-only. |
| `TEST_APPROVAL_KEY` | **Tests and staging only.** Switches on `POST /v1/device/approve`, which approves a device request as any member. Set anywhere else, every call answers `500 misconfigured`. Unset, the route is `404`. Never set it in production. |

`test/config.test.ts` reads `wrangler.toml` and fails if the top level names `TEST_APPROVAL_KEY` or
sets `STAGING = "on"`, or if `[env.staging]` loses `routes = []`. Wrangler copies `routes` into an
environment but not `vars`, `r2_buckets` or Durable Object bindings, so staging repeats all of them.
Without `routes = []`, a staging deploy would move `ingest.agentcommunity.org` to the staging Worker.

## What is checked where

- **Before a chunk body is read:** the credential; the run exists and is the credential's own; the
  entry is one of the run's. A chunk the server already holds (same offset, `Goalcup-Length` and
  `Goalcup-Head`) is answered `200` from the stored row. Then: the run is not closed, the entry is
  not sealed, the end rules allow it (after `ends_at`: `503` until the entry's final
  heartbeat, then nothing past the committed end), `Content-Length` is present and at most
  33,554,432, `Content-Type` and all seven `Goalcup-*` headers are well formed, and the maintenance
  switch is off.
- **Then:** offset, `prev` and first sequence must continue exactly where the entry stands (`409`).
- **Storage:** the body is piped (never buffered) into R2 with a create-only put, and R2 checks the
  SHA-256 (`422 digest_mismatch`, nothing stored). If the name already exists, an earlier attempt stored
  it but its database step failed, and the upload continues from there.
- **Then one atomic database step** advances offset, sequence and head, records the chunk, and adds
  its compressed size and one chunk to the entry's `stored_bytes` and `chunk_count`, only if they
  still match what was checked. A request that loses a race is answered from the stored chunk if it
  was a retry, otherwise `409`. Then the seal is tried ([The end of a run](#the-end-of-a-run-as-implemented)).
- **Not checked here, by design:** whether the bytes are a valid chain.

## Operator runbook

Short steps for the people who run the event. Run the wrangler commands in `server/worker`, in a shell
where `CLOUDFLARE_ACCOUNT_ID` is set to the Agent Community account id (`export CLOUDFLARE_ACCOUNT_ID=<id>`).
Sign in with `npx wrangler login`, or set `CLOUDFLARE_API_TOKEN`. Production is the Worker `goalcup-ingest`
at `https://ingest.agentcommunity.org`. Staging is `goalcup-ingest-staging` at
`https://goalcup-ingest-staging.b-9a5.workers.dev`: add `--env staging` to each wrangler command, and use
the staging address in each `curl`. Do not put a secret in this file, in a commit or in a chat message.

Run `npm ci` in `server/worker` before any `npx wrangler` step, and again after you switch to another
commit. It installs the pinned wrangler. Check that `npx wrangler --version` prints `4.149.0`. Without
`npm ci`, `npx` can download a different version, and the steps here are checked for 4.149.0 only.

[Deploy checks](#deploy-checks) · [Maintenance](#maintenance-on-and-off) · [Rollback](#rollback) ·
[Logs](#logs) · [Change a start window](#change-a-start-window) · [Add a test operator](#add-a-test-operator) ·
[Clean up test runs](#clean-up-test-runs) · [Minimum competition version](#set-the-minimum-competition-version) ·
[EVENT_ID switch](#event_id-switch-6-nov-2026) · [Deploying 1.0](#deploying-10)

### Deploy checks

These rules apply to every `wrangler deploy` and every `wrangler rollback`, on production and on staging.
Each deploy step below repeats them in short form. The details are here.

1. **Deploy from `main` only.** Run `git switch main && git pull --ff-only`. `git status` must show no
   changes. A deploy from a branch can ship a `wrangler.toml` that main does not have. The commit you
   deploy must be on `main`. Add `--tag "$SHA" --message "$SHA"` to every deploy, with
   `SHA=$(git rev-parse --short HEAD)`, so that the live deployment names its commit.
2. **Take `npx wrangler deployments list` before and after.** Save both outputs. After the deploy there is
   exactly one new deployment, and `npx wrangler deployments status` names it as the live one.
3. **Check that `ingest.agentcommunity.org` still routes to `goalcup-ingest`.** Wrangler once moved the
   domain to the staging Worker (15 Oct). Check it two ways:
   - In the Cloudflare dashboard, open Workers & Pages, then `goalcup-ingest`, then Settings, then
     Domains & Routes. It lists `ingest.agentcommunity.org`. `goalcup-ingest-staging` lists no custom domain.
   - Run `curl -s -o /dev/null -w '%{http_code}\n' -X POST https://ingest.agentcommunity.org/v1/device/approve`.
     The answer is `404`: production has no `TEST_APPROVAL_KEY`. Any other answer means stop (`401` means
     the staging Worker answered). Call the owner. To move the domain back, open the dashboard page above
     for `goalcup-ingest` and add the custom domain again. (While maintenance is on, every such call
     answers `503`, so use the dashboard check only.)
4. **Variables come from `wrangler.toml` only.** A deploy replaces every `[vars]` value with the
   file's, so a value edited in the dashboard is gone after the next deploy. Change a variable
   (`EVENT_ID`, `MIN_COMPETITION_VERSION`, any other) through a PR to main. Once the cup's minimum is
   set ([Minimum competition version](#set-the-minimum-competition-version)), check after every
   production deploy that `npx wrangler versions view <version-id>` still shows
   `MIN_COMPETITION_VERSION` = the cup's minimum.
5. **The Worker before the client, and never back to 0.7.** The 1.0 Worker goes live before any 1.0
   client uploads to it, in the order of [Deploying 1.0](#deploying-10). Once 1.0 clients run, no
   deploy or rollback may bring back a 0.7 Worker: it refuses their heartbeats and never seals their
   entries. A bad 1.0 Worker is held with maintenance on and fixed forward.

At the top level, `wrangler deploy` prints "Multiple environments are defined". That is expected for
production. Go on.

After a staging deploy, rule 3 is the check that matters, and `npx wrangler deployments list` for
production (no `--env`) must show no new deployment.

### Maintenance on and off

Maintenance makes every call (except `GET /v1/health`) answer `503 maintenance` with `Retry-After`, before
authentication. goalcup clients back off and keep recording. The cron does nothing while it is on. Run deadlines do
not move.

There is one way to switch it on: `npx wrangler deploy --var MAINTENANCE:on`, from a clean checkout of
`main` at the commit that is live now. Do not edit `wrangler.toml`, and never commit `MAINTENANCE = "on"`.
The flag ships exactly the code that is live, so no new code reaches production during an incident.
This path does not run `npm test`, because the code is already live.

**While maintenance is on, add `--var MAINTENANCE:on` to every deploy** (a hotfix, the EVENT_ID switch, any
other). A deploy without it ends maintenance silently. A rollback to a version that was deployed without
maintenance ends it too. After any deploy, run the state check below.

State check. Run `curl -s -o /dev/null -w '%{http_code}\n' -X POST https://ingest.agentcommunity.org/v1/device`.
It answers `503` when maintenance is on. Any other answer (usually `400`) means it is off. Run this check
first, before you switch it on or off.

To switch it on:

1. Run the state check. If it answers `503`, maintenance is already on: stop.
2. Run `git fetch origin && git switch main && git pull --ff-only`. `git status` must show no changes.
3. Run `npx wrangler deployments list` (before). Find the live deployment. Its message names the commit
   (`SHA`). If it names none, stop and ask the owner. Run `git merge-base --is-ancestor <SHA> origin/main`:
   it must print nothing and exit 0. Then run `git switch --detach <SHA>`, then `npm ci`.
4. Run `SHA=$(git rev-parse --short HEAD)`, then
   `npx wrangler deploy --var MAINTENANCE:on --tag "$SHA" --message "$SHA maintenance on"`. Deploy from
   `main` only: this is a commit of `main`. To change the wait, add `--var MAINTENANCE_RETRY_AFTER:600`
   (the default is `300` seconds).
5. Run `npx wrangler deployments list` (after). Check that `ingest.agentcommunity.org` still routes to
   `goalcup-ingest` (dashboard check of [Deploy checks](#deploy-checks), rule 3).
6. Run the state check. It must answer `503`.
7. Run `git switch main`.

To switch it off:

1. Run the state check. If it does not answer `503`, maintenance is already off: stop.
2. Run `git fetch origin && git switch main && git pull --ff-only`. `git status` must show no changes. If
   the owner wants only the code that is live, use `git switch --detach <SHA>` as in step 3 above.
   Then run `npm ci`.
3. Run `npx wrangler deployments list` (before).
4. Run `SHA=$(git rev-parse --short HEAD)`, then
   `npx wrangler deploy --tag "$SHA" --message "$SHA maintenance off"`, with no `--var`. The value in
   `wrangler.toml` is `off`, so this ends maintenance. Deploy from `main` only.
5. Run `npx wrangler deployments list` (after). Check that `ingest.agentcommunity.org` still routes to
   `goalcup-ingest` ([Deploy checks](#deploy-checks), rule 3).
6. Run the state check. It must not answer `503`.

### Rollback

Use rollback when a deploy is bad and you know the last good version.

1. Run `git switch main && git pull --ff-only` and `npm ci`. Deploy from `main` only.
2. Run `npx wrangler deployments list` (before). Save the output. Each row shows its version id. Pick the
   version that was live before the bad deploy.
3. Run `npx wrangler rollback <version-id> --message "<reason>"`. Answer the prompt.
4. Run `npx wrangler deployments list` again (after). Check that `ingest.agentcommunity.org` still routes to
   `goalcup-ingest` ([Deploy checks](#deploy-checks), rule 3).
5. Confirm: `curl -s https://ingest.agentcommunity.org/v1/health` answers `{"status":"ok"}`. Then run the
   maintenance state check: a rollback to a version deployed without maintenance ends maintenance.

A rollback restores the code and the variables of that version. It does not undo a database migration or
change R2. Never roll back to a version deployed before 1.0 ([Deploy checks](#deploy-checks), rule 5).
To see which `EVENT_ID` a version carries, run `npx wrangler versions view <version-id>`. If you
cannot find a good version, switch maintenance on and ask the owner.

### Logs

- Workers Logs are on in `wrangler.toml` for production and for staging. They keep the Worker's own
  `console.error` lines: `misconfigured: ...`, `supabase <fn>: ...`, `tick <step> <run_id>: ...`,
  `unhandled: ...` (a `500 internal`), `mail notice ...`, `r2-put-unrecognised-error` and `seal-failed <run_id>: ...` (the client repeats its
  final heartbeat, which tries the seal again), and the
  warnings `large-run <run_id> stored=<bytes>` (each 10 GiB a run crosses) and `counts-dropped <run_id>`
  (a heartbeat's `counts` had the wrong shape and was not kept). Read them in the Cloudflare dashboard:
  Workers & Pages, the Worker, Observability (Logs). Search for the text you need.
- Invocation logs are off on purpose (`invocation_logs = false`). An invocation log keeps request details,
  such as the `Authorization` header and the client address. The data policy does not allow that. Because of
  this, the logs show no status code per request: only the Worker's own lines. `test/config.test.ts` fails
  if someone turns invocation logs on.
- For a live view, run `npx wrangler tail --search "<text>"`. It keeps nothing. Do not save or share its
  output.

### Change a start window

A start window is a row of `goal_cup_tracer.events` in the website's database. The Worker reads it on each
open. A window change needs no Worker deploy. (A change of `EVENT_ID` does: see the
[EVENT_ID switch](#event_id-switch-6-nov-2026).) Steps 1 to 6 run in the website
checkout (`agentCommunity_PAGE`), not in `server/worker`.

1. Add a migration in `supabase/migrations/` with `goal_cup_tracer` in its file name. It holds an
   `UPDATE goal_cup_tracer.events SET start_window_opens = '<UTC time>', start_window_closes = '<UTC time>'
   WHERE id = '<event id>' AND mode = '<mode>' AND staging = false;` for each row to change. The mode is
   `competition` or `dry_run`. The migration `20261016120000_goal_cup_tracer_event_windows.sql` is the model.
   Keep it add-only: change rows, not tables.
2. Merge it to the website `main`, and pull `main`.
3. Read the database URL into your shell without printing it:
   `export DATABASE_URL_REMOTE="$(sed -n 's/^DATABASE_URL_REMOTE=//p' .env.local)"`. Do not echo it.
4. Apply the migration in one transaction:
   `psql "$DATABASE_URL_REMOTE" -X -1 -v ON_ERROR_STOP=1 -f supabase/migrations/<file>.sql`.
5. Run `pnpm db:check` (it uses the exported `DATABASE_URL_REMOTE`). It must pass.
6. Look at the rows: `psql "$DATABASE_URL_REMOTE" -X -c "select id, mode, start_window_opens,
   start_window_closes from goal_cup_tracer.events where staging = false order by id"`.
7. Mirror the migration into this repository. Set `F` to the full path of
   `server/worker/sql/goal-cup-tracer-upload.sql` in a goal-cup-proxy worktree, and run the regeneration command in
   the header of that file (the lines above `REGEN-END`) from the root of the website checkout at its
   `main`. Read `git diff` of the mirror: the only new part must be your migration. Commit it and open a pull
   request here.

### Add a test operator

Only a member listed in `goal_cup_tracer.test_operators` can approve or open a test run. Anyone else gets
`403`. The owner adds the rows. The list is one for production and staging, because they share the database.

1. Get the member's id: the `auth.users` id (Supabase dashboard, Authentication, Users).
2. In the website checkout, export `DATABASE_URL_REMOTE` as in step 3 of "Change a start window".
3. Run `psql "$DATABASE_URL_REMOTE" -X -v ON_ERROR_STOP=1 -c "insert into goal_cup_tracer.test_operators
   (member_id, note) values ('<member-uuid>', '<name>')"`.
4. Check: `psql "$DATABASE_URL_REMOTE" -X -c "select member_id, note from goal_cup_tracer.test_operators"`.

To remove an operator, delete the row with the same `member_id`.

### Clean up test runs

`scripts/delete-test-runs.sh` deletes all organizer test runs, in the database and in R2
(`goalcup-traces/traces/test/`, `goalcup-traces-staging/traces/test-staging/`, `goalcup-usage/usage/test/`
and `goalcup-usage-staging/usage/test-staging/` only). It lists R2 first, so a failed listing changes
nothing. The token needs R2 read and write on both traces buckets and on each usage bucket that exists.
A usage bucket that is missing or not open to the token (`404` or `403`) is skipped with one line; its
test objects stay until a run with a token that reaches it. It needs `psql`,
`curl` and `python3`, and `DATABASE_URL`, `CLOUDFLARE_ACCOUNT_ID` and `CLOUDFLARE_API_TOKEN` in the
environment. `DATABASE_URL` is the website database URL: set it from `DATABASE_URL_REMOTE`
(`export DATABASE_URL="$DATABASE_URL_REMOTE"`). Run the script in `server/worker`.

1. Run `sh scripts/delete-test-runs.sh`. This is a dry run. It prints what would go and changes nothing.
2. Read the list. If it is right, run `sh scripts/delete-test-runs.sh --yes`.
3. To keep recent runs, add `--older-than '7 days'`. If the script reports objects not deleted, run it
   again without `--older-than`. That run deletes every test run. (With `--older-than`, a second run no longer
   knows the runs, because the rows are gone.) A test run still uploading while the script runs can leave
   objects behind; a run without `--older-than` removes them too.

More on test runs: [OPERATING](../../docs/OPERATING.md#test-runs).

### Set the minimum competition version

`MIN_COMPETITION_VERSION` refuses a device request and an open from an older goalcup, in every mode,
with `403` and an install line (D70; `continue` is never refused). `wrangler.toml` sets `1.0.0` in
both blocks: a 0.7 goalcup cannot work with the 1.0 Worker. Raise it when a later release must be the
least any participant runs, for example the cup's minimum. It must be live before 7 Nov 00:00 UTC
if the cup needs it.

**Only after that release is published and GitHub's latest release serves it.** The refusal's install
line installs `releases/latest`. Set the minimum before that and it installs an older goalcup, which
is refused again. Below, `<minimum>` is that release's number and `<older>` is the release before it.

1. Check the release. Every line of
   `curl -fsSL https://github.com/agentcommunity/goal-cup-proxy/releases/latest/download/SHA256SUMS`
   names `goalcup-<minimum>-`. If not, stop and wait.
2. On a branch, set `MIN_COMPETITION_VERSION = "<minimum>"` in both `[vars]` and `[env.staging.vars]`
   of `wrangler.toml` (`test/config.test.ts` fails if they differ). Open a PR and merge it to main.
   Never set it in the dashboard: the next deploy wipes it ([Deploy checks](#deploy-checks), rule 4).
3. On main at that merge commit: `npm ci && npm test`.
4. Deploy from main, with every [deploy check](#deploy-checks): `npx wrangler deployments list`
   (before), `SHA=$(git rev-parse --short HEAD)`, `npx wrangler deploy --tag "$SHA" --message "$SHA"`
   (add `--var MAINTENANCE:on` if maintenance is on), `npx wrangler deployments list` (after), and
   the domain check.
5. `npx wrangler versions view <version-id>` shows `MIN_COMPETITION_VERSION` = `<minimum>`.
6. Ask as an old and as a new client:
   ```sh
   for v in <older> <minimum>; do curl -s -o /dev/null -w "$v %{http_code}\n" -XPOST https://ingest.agentcommunity.org/v1/device \
     -H 'content-type: application/json' \
     -d "{\"purpose\":\"run\",\"mode\":\"competition\",\"connector_id\":\"codex-cli\",\"proxy_version\":\"$v\"}"; done
   ```
   `<older>` answers `403`, `<minimum>` answers `200` (a device request nobody approves; it expires by
   itself).

The EVENT_ID switch may carry the same change in its PR, if step 1 already passes on 6 Nov.

## EVENT_ID switch: 6 Nov 2026

Do the switch between 6 Nov 00:00 and 7 Nov 00:00 UTC. It does not wait for a release: releases
continue during the cup (D70). Production `EVENT_ID` stays `goalcup-dryrun-2026` until then. The start windows are database rows
(website migration `20261016120000_goal_cup_tracer_event_windows.sql`): the dry run closes
6 Nov 00:00 UTC and the cup opens 7 Nov 00:00 UTC, so any time on 6 Nov is safe. If the switch is
missed, every cup start on 7 Nov is refused ("after the start window ..."): competition starts read
the dry-run row, which closed 6 Nov 00:00 UTC.

1. On a branch, set `EVENT_ID = "goalcup-cup-2026"` in `wrangler.toml` `[vars]` (not
   `[env.staging.vars]`). If `MIN_COMPETITION_VERSION` is not the cup's minimum yet and that release is
   the latest release, set it in the same PR ([Minimum competition version](#set-the-minimum-competition-version),
   step 1 first). If that release is not out yet, leave it and do that section on its own once it is,
   before 7 Nov 00:00 UTC. Open a PR and merge it to main.
2. On main at that merge commit: `npm test` (in `server/worker`).
3. From main only, at that merge commit: run `npm ci`, then the maintenance state check. Run
   `npx wrangler deployments list` (before). Run `SHA=$(git rev-parse --short HEAD)`, then
   `npx wrangler deploy --tag "$SHA" --message "$SHA"` (production: top level, no `--env`). If maintenance
   is on, add `--var MAINTENANCE:on`, or the deploy ends it. Run `npx wrangler deployments list` (after).
   Check that `ingest.agentcommunity.org` still routes to `goalcup-ingest`
   ([Deploy checks](#deploy-checks), rule 3). Every later production deploy is from main, so it keeps the cup
   (a 7 Nov hotfix too).
4. Check the live Worker: `npx wrangler deployments status` names the live version, and
   `npx wrangler versions view <version-id>` shows `EVENT_ID` = `goalcup-cup-2026` (or the dashboard:
   goalcup-ingest, Settings, Variables), and `MIN_COMPETITION_VERSION` = the cup's minimum once it is set.
5. Open one practice run and one organizer test run against production; both start. Then
   `sh scripts/delete-test-runs.sh --yes` ([Clean up test runs](#clean-up-test-runs)).
6. A competition start now answers "The start window has not opened yet." (before the switch it
   answered "after the start window ..."). After 7 Nov 00:00 UTC, `select distinct event_id from
   goal_cup_tracer.runs where mode = 'competition' and official_start >= '2026-11-07'` shows only
   `goalcup-cup-2026`, and the R2 prefix `traces/goalcup-cup-2026/` holds its chunks.

## Deploying 1.0

Once, in this order. The 1.0 Worker and a 0.7 goalcup do not work together, so the order matters.

1. Until staging carries 1.0, builds test against a local Worker (`wrangler dev`, `DB_BACKEND=local`).
2. Staging: `npx wrangler deploy --env staging` from the integration head, with `1.0.0` as the floor
   in both blocks. Every later staging deploy is from the integration head too.
3. Before production, the owner checks that no real run is open (inside `BEGIN READ ONLY`:
   `SELECT count(*) FROM goal_cup_tracer.runs WHERE status='open' AND NOT staging AND mode <> 'test'`).
   A 0.7 goalcup mid-run stops uploading after the deploy; it keeps its data. With 1.0 the unchanged
   cron sends the `recording` email when a heartbeat says `failed`, and the idle emails count written
   requests: the owner confirms that is wanted.
4. Release `v1.0.0`, so `releases/latest` serves 1.0 and the floor's install line works.
5. Production (owner): `npx wrangler deploy` from the same commit, with every
   [deploy check](#deploy-checks). Only then delete the old signing secret:
   `npx wrangler secret delete RECEIPT_SIGNING_KEY` (the 0.7 Worker refuses every call without it, so
   never before). Create the `goalcup-usage` bucket.
6. Smoke (owner): one practice run with the 1.0 goalcup on production, sealed, fetched with
   `scripts/fetch-run.sh` and checked.
7. The [EVENT_ID switch](#event_id-switch-6-nov-2026) stays a manual deploy on 6 Nov; it can be the
   same deploy as step 5 if the timing fits.

There is no rollback to 0.7 after step 5. A bad 1.0 Worker is held with
[maintenance](#maintenance-on-and-off) on (clients back off and keep everything) and fixed forward.

## Checked on real R2 (staging, 26 Oct 2026)

- **Checksum:** a chunk whose body did not match its `Goalcup-Sha256` got `422 digest_mismatch`, and
  nothing was stored under its key. The Worker maps a failed `sha256` put by matching R2's error text
  (`10037`, "checksum", "did not match"; logged as `r2-put-digest-mismatch`). If R2 changes that
  wording, uploads come back `502` and the log has `r2-put-unrecognised-error` with the new wording:
  add it to the pattern in `src/chunks.ts`.
- **Create-only:** a decoy object put at the key of the next valid chunk stayed unchanged when that
  chunk was uploaded (the Worker answered `200`: an existing key is taken as an earlier attempt). Only
  production can prove this under the bucket lock, on `traces/goalcup-cup-2026/` after the EVENT_ID
  switch; that case is still open.
- A request with Python's default User-Agent is refused by Cloudflare (`error code: 1010`) before it
  reaches the Worker; send a `goalcup/` User-Agent when testing by hand.

## Decisions where the contract is silent

- `GET /v1/runs/{id}` always answers, whatever the status.
- A retried chunk's `200` body repeats what the original answered: the offset just after it,
  `last_seq + 1` and its head (not the run's current position), with `sealed` as the entry stands now.
- `Content-Length` missing, malformed or zero is `400 malformed`. `Content-Type` other than
  `application/zstd` is `400`.
- A wrong HTTP method on a known path is `405 method_not_allowed`. A malformed run id in a path is
  `404`; an unparseable chunk offset is `400`.
- A request lives 900 seconds from its creation in every state, so a device approval that is not
  picked up in time is lost.
- Database or storage failures are `502 upstream_error`, and misconfiguration is `500 misconfigured`
  (one generic sentence for every cause; the detail, which may hold configuration, goes to the Worker's
  log only). goalcup retries any 5xx.
- A run that is over is closed the first time any call names it. The cron does the same for runs
  nobody looks at.

## The cron

`scheduled` in `src/index.ts` runs `runTick` (`src/notices.ts`) every 5 minutes. Four steps, in this
order, each in its own try/catch: `closeRuns`, `recordFinalCommits`, `queueNotices`, `sendNotices`
(`src/mail.ts`). All take the time as a parameter. They read `Db.runsForTick(now)` (open runs, and runs
that ended in the last 48 hours). Every notice is decided here, never in a request handler, and a
staging Worker sends every email to its test inbox. The constants (20 minutes of silence, idle at 3
and 6 hours, an hour to wait for the commit, 5 minutes before GitHub is asked, an hour of grace for a
`404` or a failing activity API, 50 runs asked of GitHub per tick, 20 emails per tick) are in
`notices.ts`, `github.ts` and `mail.ts` and have no variable. The rules are in
[UPLOAD-API](../../docs/UPLOAD-API.md#what-the-server-does-on-its-own).

Cost: each step reads the run list with one call. `queueNotices` makes one call for the whole batch
however many runs there are. `closeRuns` makes one call per run due to close. `recordFinalCommits` asks
GitHub about at most 50 runs (taking turns when more are due), each with 2 to 4 fetches (repository,
activity, and the commit list and `commits/HEAD` only as a fallback; up to 8 for a run with more than
100 activities), and makes 50 writes. That is a few hundred subrequests in a busy tick, so the cron
needs the paid Workers plan (the free plan allows 50 per invocation). A failed step is logged
(`tick <step> <run_id>: <message>` or `tick queueNotices: <message>`) and the next tick retries; a notice
is stored once per run, kind and key, so a retry is safe.

Locally, `wrangler dev --test-scheduled` and `curl localhost:8787/__scheduled` run one tick. The outbox
rows are the objects under `state/notices/`. The tests call `worker.scheduled` on a moved clock
(`test/helpers/inproc.ts`, `tick`).

## The end of a run, as implemented

The final heartbeat, `stop`, `503 awaiting_final` and the `409` and `410` rules are as
[UPLOAD-API, "When a run ends"](../../docs/UPLOAD-API.md#when-a-run-ends) says; every rule is per entry.

- There is no seal call. `trySeal` (`src/runs.ts`) seals an entry once its final heartbeat has
  committed the end and the chain is held up to it, whichever comes second: the final heartbeat tries
  with the head it committed, and the chunk path tries when the stored chunk ends at the committed
  `(seq, head)`. The SQL seal step checks both facts again under a row lock, so two racing calls are
  safe. Not yet held is `sealed: false`, never an error.
- The end rules run again inside the atomic chunk commit.
- To test a deadline in seconds, use the local-only `RUN_SECONDS` and `CLOSE_AFTER_SECONDS`.
