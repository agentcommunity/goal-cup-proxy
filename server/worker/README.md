# Goal Cup upload Worker

The server end of `goalcup`'s trace upload: a Cloudflare Worker (TypeScript, no framework). It checks
a participant's run credential, streams zstd chunks of `chain.jsonl` into R2, records progress in
Supabase (Postgres), and returns Ed25519-signed receipts and acknowledgements in the formats the Rust
verifier checks.

- **Wire contract (the authority):** [docs/UPLOAD-API.md](../../docs/UPLOAD-API.md). Paths, headers,
  status codes, error bodies and idempotency rules are implemented as written there.
- **Design:** [docs/SERVER-DESIGN.md](../../docs/SERVER-DESIGN.md).
- **Deploying, test runs, bad days:** [docs/OPERATING.md](../../docs/OPERATING.md). The short steps for
  operators are in the [Operator runbook](#operator-runbook) below.

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
| `src/runs.ts` | Open, continue, receipt, abandon, position, heartbeat, seal |
| `src/chunks.ts` | The chunk upload: early refusals, retry rule, streaming to R2, atomic commit |
| `src/notices.ts` | The 5-minute cron: close runs, record the judged GitHub commit, queue the run emails |
| `src/github.ts` | The GitHub API: the judged commit (last push before the deadline) and the headers every GitHub call carries |
| `src/mail.ts` | Sends each queued notice once through Resend |
| `src/sign.ts` | Ed25519 receipts and acknowledgements (byte-exact with the Rust formats) |
| `src/db.ts` | The `Db` interface: one method per step, and the run-state rules (`endsAt`, `statusAt`) |
| `src/db-supabase.ts` | Production: one Postgres function per step, through PostgREST RPC |
| `src/db-local.ts` | `wrangler dev` and tests only: JSON state in the R2 bucket under `state/`, atomic by etag |
| `src/context.ts`, `env.ts`, `errors.ts`, `util.ts` | Configuration, bindings, error types, helpers |
| `sql/goal-cup-tracer-upload.sql` | Copy of the website's migration that `db-supabase.ts` calls. The website repo is the authority. |
| `sql/pending/` | The migration written here first, not yet in the website repo nor applied (its README says how to land it) |
| `scripts/check-sql.sh` | Applies the mirror and the pending migration to a throwaway local Postgres and smoke-calls the new functions |
| `scripts/delete-test-runs.sh` | Deletes all organizer test runs, rows and R2 objects ([OPERATING](../../docs/OPERATING.md#test-runs)) |
| `test/` | `sign.test.ts` (the signing vectors shared with the Rust crate), `config`, `github`, `supabase` (RPC names, stubbed fetch), `local-db`, `ratelimit`, `mail`, `device-unit`; `integration`, `device`, `segments` (HTTP against `wrangler dev`); `lifecycle`, `notices`, `test-mode` (the Worker in this process on a clock the test moves) |

## Configuration

Variables (in `wrangler.toml` `[vars]`, or `--var` / `.dev.vars` locally):

| Name | Default | Meaning |
|---|---|---|
| `EVENT_ID` | none (required) | Event that new competition runs belong to. Lowercase letters, digits, dashes. It becomes part of every R2 key. |
| `PRACTICE_EVENT_ID` | none (required) | Event that practice runs belong to. |
| `TEST_EVENT_ID` | none | Event that organizer test runs (mode `test`) belong to. Unset: the server opens no test run (`403`). |
| `RUN_HOURS` | `168` | A competition run's `deadline_at` = `official_start` + this many hours. |
| `PRACTICE_RUN_MINUTES` | `10` | A practice run's length. |
| `MIN_COMPETITION_VERSION` | empty (no minimum) | The oldest goalcup (`x.y.z`; the first three numbers of what it sends) that may sign in for, or start, a competition run. Older, or a version with no `x.y.z` while a minimum is set, is `403` with the update line. A value with no `x.y.z` in it is malformed (parsing is lenient: the first `x.y.z` counts) and makes those two calls `500 misconfigured`; heartbeats and everything else are untouched. |
| `LOCAL_ADMIN_MEMBERS` | none | `DB_BACKEND=local` only: comma-separated member ids the local database calls admins, for `--dev` runs. In production the database decides (`goal_cup_upload_member_is_admin`). |
| `RUN_MAX_COMPRESSED_BYTES` | `68719476736` (64 GiB) | The most compressed chunk bytes one run may hold in R2, over all its entries. A chunk that would pass it is `413 quota_exceeded`, nothing stored; goalcup retries, so raising it (a deploy) resumes the upload. Log lines `quota-half`, `quota-full`, `quota-exceeded <run_id>`. |
| `DB_BACKEND` | `supabase` | `supabase` (production) or `local` (`wrangler dev` and tests only). Never set `local` in the deployed `wrangler.toml`. |
| `APPROVAL_URL` | in `wrangler.toml` | The website page where a member approves a device request, without a query string. `verification_uri_complete` adds `?code=<user code>`. |
| `STAGING` | off | `on` marks a staging deployment: the only place besides `DB_BACKEND=local` where `TEST_APPROVAL_KEY` may be set. Set only in `[env.staging.vars]`. |
| `GITHUB_CHECK` | off | `on` makes `POST /v1/runs` ask GitHub that `repo_url` is a public repository (`wrangler.toml` sets `on`; `npm run dev` and the tests turn it off). A missing or private repository is `400`; GitHub unreachable or rate-limiting is `502`, which goalcup retries. |
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
  `Goalcup-Head`) is answered `200` with the stored ack. Then: the run is not abandoned or closed, the
  entry is not sealed, the end rules allow it (after `ends_at`: `503` until the entry's final
  heartbeat, then nothing past the committed end), `Content-Length` is present and at most
  33,554,432, `Content-Type` and all seven `Goalcup-*` headers are well formed, and the maintenance
  switch is off.
- **Then:** offset, `prev` and first sequence must continue exactly where the entry stands (`409`).
- **Then the run's size limit:** the run's `stored_bytes` plus this chunk's `Content-Length` must stay
  within `RUN_MAX_COMPRESSED_BYTES` (`413 quota_exceeded`, body unread, nothing stored).
- **Storage:** the body is piped (never buffered) into R2 with a create-only put, and R2 checks the
  SHA-256 (`422 digest_mismatch`, nothing stored). If the name already exists, an earlier attempt stored
  it but its database step failed, and the upload continues from there.
- **Then one atomic database step** advances offset, sequence and head, records the chunk and its
  ack, and adds the chunk's compressed size and one chunk to the entry's `stored_bytes` and
  `chunk_count`, only if they still match what was checked. A request that loses a race is answered from the
  stored chunk if it was a retry, otherwise `409`.
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
[Clean up test runs](#clean-up-test-runs) · [EVENT_ID switch](#event_id-switch-6-nov-2026-with-the-last-release)

### Deploy checks

Three rules apply to every `wrangler deploy` and every `wrangler rollback`, on production and on staging.
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
change R2. To see which `EVENT_ID` a version carries, run `npx wrangler versions view <version-id>`. If you
cannot find a good version, switch maintenance on and ask the owner.

### Logs

- Workers Logs are on in `wrangler.toml` for production and for staging. They keep the Worker's own
  `console.error` lines: `misconfigured: ...`, `supabase <fn>: ...`, `tick <step> <run_id>: ...`,
  `unhandled: ...` (a `500 internal`), `mail notice ...` and `r2-put-unrecognised-error`. Read them in the Cloudflare dashboard: Workers & Pages, the Worker, Observability (Logs).
  Search for the text you need.
- Invocation logs are off on purpose (`invocation_logs = false`). An invocation log keeps request details,
  such as the `Authorization` header and the client address. The data policy does not allow that. Because of
  this, the logs show no status code per request: only the Worker's own lines. `test/config.test.ts` fails
  if someone turns invocation logs on.
- For a live view, run `npx wrangler tail --search "<text>"`. It keeps nothing. Do not save or share its
  output.

### Change a start window

A start window is a row of `goal_cup_tracer.events` in the website's database. The Worker reads it on each
open. A window change needs no Worker deploy. (A change of `EVENT_ID` does: see the
[EVENT_ID switch](#event_id-switch-6-nov-2026-with-the-last-release).) Steps 1 to 6 run in the website
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
(`goalcup-traces/traces/test/` and `goalcup-traces-staging/traces/test-staging/` only). It needs `psql`,
`curl` and `python3`, and `DATABASE_URL`, `CLOUDFLARE_ACCOUNT_ID` and `CLOUDFLARE_API_TOKEN` in the
environment. `DATABASE_URL` is the website database URL: set it from `DATABASE_URL_REMOTE`
(`export DATABASE_URL="$DATABASE_URL_REMOTE"`). Run the script in `server/worker`.

1. Run `sh scripts/delete-test-runs.sh`. This is a dry run. It prints what would go and changes nothing.
2. Read the list. If it is right, run `sh scripts/delete-test-runs.sh --yes`.
3. To keep recent runs, add `--older-than '7 days'`. If the script reports objects not deleted, run it
   again without `--older-than`. That run deletes every test run. (With `--older-than`, a second run no longer
   knows the keys, because the rows are gone.)

More on test runs: [OPERATING](../../docs/OPERATING.md#test-runs).

## EVENT_ID switch: 6 Nov 2026, with the last release

Production `EVENT_ID` stays `goalcup-dryrun-2026` until then. The start windows are database rows
(website migration `20261016120000_goal_cup_tracer_event_windows.sql`): the dry run closes
6 Nov 00:00 UTC and the cup opens 7 Nov 00:00 UTC, so any time on 6 Nov is safe. If the switch is
missed, every cup start on 7 Nov is refused ("after the start window ..."): competition starts read
the dry-run row, which closed 6 Nov 00:00 UTC.

1. On a branch, set `EVENT_ID = "goalcup-cup-2026"` in `wrangler.toml` `[vars]` (not
   `[env.staging.vars]`). Open a PR and merge it to main.
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
   goalcup-ingest, Settings, Variables).
5. Open one practice run and one organizer test run against production; both start. Then
   `sh scripts/delete-test-runs.sh --yes` ([Clean up test runs](#clean-up-test-runs)).
6. A competition start now answers "The start window has not opened yet." (before the switch it
   answered "after the start window ..."). After 7 Nov 00:00 UTC, `select distinct event_id from
   goal_cup_tracer.runs where mode = 'competition' and official_start >= '2026-11-07'` shows only
   `goalcup-cup-2026`, and the R2 prefix `traces/goalcup-cup-2026/` holds its chunks.

## Before launch: what only real services can prove

Status on 16 Oct 2026: **neither check has a record.** The repository shows no run of either one on staging
or on production. Treat both as open. When you run one, write the date and the result here, in the same pull request.

- **R2 checksum failure.** The Worker maps a failed `sha256` put to `422 digest_mismatch` by matching
  the error text (`10037`, "checksum", "did not match"). That wording is proven only in the local
  simulator. On a deployed Worker, upload a chunk with a wrong `Goalcup-Sha256` against real R2 and
  confirm `422` and nothing stored. If it comes back `502`, the Worker log has a line starting
  `r2-put-unrecognised-error` with the real wording: add it to the pattern in `src/chunks.ts`. (A
  recognised failure logs `r2-put-digest-mismatch`.) Staging has a real R2 bucket, so staging can prove it.
- **Create-only puts** (`onlyIf: { etagDoesNotMatch: "*" }`) must refuse to overwrite on real R2,
  including under the bucket lock. Staging can prove the refusal. Staging has no bucket lock, so only
  production can prove the lock case, and only on the locked prefix `traces/goalcup-cup-2026/`, which holds
  data only after the EVENT_ID switch. Test runs use `traces/test/`, which is never locked.

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
  log only). goalcup retries any 5xx.
- A run that is over is closed the first time any call names it. The cron does the same for runs
  nobody looks at.

## The cron

`scheduled` in `src/index.ts` runs `runTick` (`src/notices.ts`) every 5 minutes. Four steps, in this
order, each in its own try/catch: `closeRuns`, `recordFinalCommits`, `queueNotices`, `sendNotices`
(`src/mail.ts`). All take the time as a parameter. They read `Db.runsForTick(now)` (open runs, and runs
that ended in the last 48 hours; never abandoned ones). The constants (20 minutes of silence, idle at 3
and 6 hours, an hour to wait for the commit, 5 minutes before GitHub is asked, an hour of grace for a
`404` or a failing activity API, 50 runs asked of GitHub per tick, 20 emails per tick) are in
`notices.ts`, `github.ts` and `mail.ts` and have no variable. The rules are in
[UPLOAD-API](../../docs/UPLOAD-API.md#what-the-server-does-on-its-own).

Cost: each step reads the run list with one call. `queueNotices` makes one call for the whole batch
however many runs there are. `closeRuns` makes one call per run due to close. `recordFinalCommits` asks
GitHub about at most 50 runs (taking turns when more are due), each with 2 to 4 fetches (repository,
activity, and the commit list and `commits/HEAD` only as a fallback; up to 8 for a run with more than
100 activities), and makes 50 writes. That is a few
hundred subrequests in a busy tick, so the cron
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
