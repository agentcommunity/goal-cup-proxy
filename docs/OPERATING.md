# Operating Goal Cup

The runbook for organizers: where things run, the dates, how to deploy, test runs, how to fetch and
check a run, what a run whose recording is off looks like, what storage costs, what to do on a bad day,
and how to release. A developer's path (build, gate, one capture) is in the [README](../README.md). The
server's design and operator steps are in the [Worker README](../server/worker/README.md); its wire
contract is [UPLOAD-API](UPLOAD-API.md).

Never write a secret, a key value or a key file path into a doc, an issue, a log or a chat. This page
names secrets only.

## Where things run

| | Production | Staging | Local |
|---|---|---|---|
| Worker | `goalcup-ingest` at `https://ingest.agentcommunity.org` (custom domain, no workers.dev) | `goalcup-ingest-staging` at `https://goalcup-ingest-staging.b-9a5.workers.dev` | `npm run dev` at `http://localhost:8787` |
| Trace bucket | `goalcup-traces` (lock on `traces/goalcup-cup-2026/` only) | `goalcup-traces-staging` (no lock) | simulated on disk |
| Usage bucket | `goalcup-usage` (the owner creates it with the production deploy) | `goalcup-usage-staging` | none |
| Competition event | `EVENT_ID` in `wrangler.toml`: now `goalcup-dryrun-2026`, then `goalcup-cup-2026` | `goalcup-dryrun-staging` | any |
| Practice event | `goalcup-practice` (always open, 10-minute runs) | `goalcup-practice-staging` | any |
| Test event | `test` | `test-staging` | any |
| Database | the website's Supabase project, schema `goal_cup_tracer` | the same project | JSON state in the simulated bucket |
| Supabase key / role | `goalcup_ingest_production` / `goal_cup_ingest_production` | `goalcup_ingest_staging` / `goal_cup_ingest_staging` | none |
| Resend key | `goalcup-ingest-production` (send-only) | `goalcup-ingest-staging` (every mail goes to `delivered@resend.dev`) | none |
| Test approval route | never | on (`STAGING=on` and the `TEST_APPROVAL_KEY` secret) | on (`.dev.vars`) |
| Approval page | `https://agentcommunity.org/goalcup/approve` (the website) | the same | the test route |

Test runs live under four bucket and prefix pairs, and only these:

| Bucket | Prefix |
|---|---|
| `goalcup-traces` | `traces/test/` |
| `goalcup-traces-staging` | `traces/test-staging/` |
| `goalcup-usage` | `usage/test/` |
| `goalcup-usage-staging` | `usage/test-staging/` |

Test prefixes are never locked. The website repo owns the schema, migrations, sign-up, terms and the
approval page. A schema change is a website migration; copy it to `server/worker/sql/goal-cup-tracer-upload.sql`
in the same change. During the event week, schema changes are add-only. The 1.0 Worker needs no
migration ([Worker README](../server/worker/README.md#how-it-is-built)).

## Dates and checklist

All times UTC.

| Date | What |
|---|---|
| 23 Oct to 6 Nov 00:00 | Dry-run window (production, `goalcup-dryrun-2026`). Dry runs can start on any day. |
| 6 Nov | Release 1.0.0 ([Releasing](#releasing)), then the owner deploys production ([Deploy](#deploy)), then the `EVENT_ID` switch, any time from 00:00 on 6 Nov to 00:00 on 7 Nov ([EVENT_ID switch](../server/worker/README.md#event_id-switch-6-nov-2026)). Releases may continue during the cup. |
| 7 Nov 00:00 | Start window opens. It lasts 24 hours. |
| 8 Nov 00:00 | Start window closes |
| 168 hours after each start | That run's deadline. The latest run ends 192 hours after the window opens. |

Before the window opens:

1. The `goalcup-cup-2026` row in the website's `events` table has `start_window_opens` =
   `2026-11-07T00:00:00Z` and `start_window_closes` = `2026-11-08T00:00:00Z`.
2. Production carries the 1.0 Worker ([Deploy](#deploy)) with `MIN_COMPETITION_VERSION` at `1.0.0`.
3. The `EVENT_ID` switch is done. Staging keeps its own event ids.
4. The pad's release rows hold ([Releasing](#releasing)).
5. `GET https://ingest.agentcommunity.org/v1/health` answers `{"status":"ok"}`, and one practice run
   against production started, sealed and fetched cleanly ([Fetch and check a run](#fetch-and-check-a-run)).
6. Test runs are deleted ([Test runs and the pad](#test-runs-and-the-pad)) and the bucket lock rule still
   covers `traces/goalcup-cup-2026/`.
7. `MAINTENANCE` is `off`.

Emails go out at most 20 each 5-minute tick (240 an hour). A burst of 1,500 `started` mails at the window
opening drains over about six hours. The limit is `NOTICES_PER_TICK` in `server/worker/src/mail.ts`.

## Deploy

Every deploy is a `wrangler` command run in `server/worker`, from `main` (staging: from the integration
head), under the rules of [Deploy checks](../server/worker/README.md#deploy-checks): exactly one new
deployment, the domain still routes to `goalcup-ingest`, variables come from `wrangler.toml` only.

```sh
npm ci && npm test                                            # type-check, unit and end-to-end tests
CLOUDFLARE_ACCOUNT_ID=<id> npx wrangler deploy --env staging  # staging
CLOUDFLARE_ACCOUNT_ID=<id> npx wrangler deploy                # production: top level, no --env
```

**Staging (agents may).** Deploy with `--env staging` from the integration head. It carries
`MIN_COMPETITION_VERSION = "1.0.0"` from its first deploy, and every pad run uploads to it.

**Production (the owner, after the merge to `main`).** Once, in the order of
[Deploying 1.0](../server/worker/README.md#deploying-10): check that no real run is open, publish the
1.0.0 release, deploy with every deploy check (`wrangler.toml` holds the `1.0.0` floor in both blocks;
`npx wrangler versions view <version-id>` must show it), only then delete the old `RECEIPT_SIGNING_KEY`
secret, create the `goalcup-usage` bucket, and smoke with one practice run, sealed, fetched and checked.
There is no rollback to the 0.7 Worker. A bad 1.0 Worker is held with `MAINTENANCE=on` and fixed forward
([Maintenance](../server/worker/README.md#maintenance-on-and-off); [Rollback](../server/worker/README.md#rollback),
only to a 1.0 version).

Secrets are set with `npx wrangler secret put <NAME>` (add `--env staging` for staging): `SUPABASE_URL`,
`SUPABASE_KEY`, `RATE_LIMIT_KEY`, `GITHUB_TOKEN`, `RESEND_API_KEY`, and for staging only `TEST_APPROVAL_KEY`
([meanings](../server/worker/README.md#configuration)). Never set `DB_BACKEND=local` or `TEST_APPROVAL_KEY`
in the production block; `test/config.test.ts` fails if you do. Watch a deploy with `npx wrangler tail`; the
log lines are listed under [Logs](../server/worker/README.md#logs).

## Test runs and the pad

<a id="test-runs"></a>

An organizer test run is a run in the test event. Only members on `goal_cup_tracer.test_operators` can open
one ([Add a test operator](../server/worker/README.md#add-a-test-operator)). It replaces nothing, is never
replaced, is left out of public numbers, and its emails go to the operator marked `[TEST]`.

The pad answers whether a connector, on this build, against staging, records and uploads a copy the
server holds whole. `python3.11 scripts/pad/pad scripts/pad/runs/<mode>.toml` runs it in a build sandbox
and prints one table; the modes are `micro`, `cycle` and `full`. Exit 0 means every row passed, 1 a
failure, 3 inconclusive, 2 a usage error. Model keys are named in `scripts/pad/pad.toml` and read by name
from the conductor's environment; they and the staging approval and database credentials never enter the
sandbox. `scripts/pad/table` writes the rows and `PAD-RESULTS.md`, and
`scripts/pad/table --release-check <commit>` is the release gate ([Releasing](#releasing)). Design and
settings: [PRD-12](plans/2026-10-30-v1.0/PRD-12-test-pad.md).

Delete every test run, database rows and R2 objects, in one go (from `server/worker`):

```sh
sh scripts/delete-test-runs.sh                              # dry run: prints what would go
sh scripts/delete-test-runs.sh --yes                        # deletes
sh scripts/delete-test-runs.sh --older-than '7 days' --yes  # only runs older than that
```

It needs `psql`, `curl` and `python3`, and `DATABASE_URL` (the website database), `CLOUDFLARE_ACCOUNT_ID`
and `CLOUDFLARE_API_TOKEN` in the environment. The token needs R2 read and write on both traces buckets
and on each usage bucket that exists. It touches only the four bucket and prefix pairs above and lists R2
before the database changes, so a failed listing changes nothing. A usage bucket that is missing or not open
to the token (`404` or `403`) is skipped with one line; a usage bucket skipped under `--older-than` keeps its
test objects until a run without `--older-than` with a token that reaches it. If the script reports objects
not deleted, run it again without `--older-than`. Details:
[Clean up test runs](../server/worker/README.md#clean-up-test-runs).

## Fetch and check a run

The Worker keeps bytes and judges nothing; the check runs on a fetched copy, on an organizer machine. The
read-only database URL and a read-only R2 token live there, in the environment, never in a build box, a
log or a command line.

```sh
sh scripts/fetch-run.sh [--bucket B] <run id | entry id> <outdir>   # run id r..., entry id e...
goalcup-verify check --server <outdir>/<entry>                       # exit 0, 1 or 2; 3 means no report
goalcup-verify secrets --known <file> <outdir>/<entry>               # known values and secret shapes
goalcup-verify usage <outdir>/<entry> --out <usage dir>              # model calls and tokens per entry
```

`fetch-run.sh` reads the index in `BEGIN READ ONLY`, downloads every chunk, checks each against its SHA-256
and writes `<outdir>/<entry>/rec.gcr.zst` (the chunks joined in offset order, never decompressed) and
`fetch.json` (the entry row, `complete`, `missing`, every heartbeat row and every chunk). The default bucket
is `goalcup-traces`; for staging pass `--bucket goalcup-traces-staging`. A partial entry is normal and exits
0; `fetch.json` says what is missing. It exits 1 when an object is missing or bad, the chain breaks or the
index has a hole, and it never overwrites. Against a local `wrangler dev` use `--local <persist dir>`.

`check --server` reads the fetched folder; `--json` prints the report, `--local <entry dir>` compares the
server copy with the participant's own entry, and `--known <file>` (one secret value per line) adds the
secrets check. `dump` prints one JSON line per record. `usage` is the next section. The report and its exit codes are in
[RECORD-FORMAT](RECORD-FORMAT.md#the-goalcup-verify-check---json-report).

## The usage command

`goalcup-verify usage` lists the model requests of a fetched run with the model, thread and tokens of
each. It reads the folder offline: no credential, no network, nothing written to the folder it reads.

```sh
goalcup-verify usage <outdir>/<entry> --out <usage dir>   # one entry; the summary and the calls file
goalcup-verify usage <outdir>/<run folder> --out <usage dir>  # every entry in it
goalcup-verify usage <outdir>/<entry>                     # the summary only, one JSON line on stdout
```

The path is an entry folder, a run folder or one part file. `--out <dir>` creates the folder and writes,
per entry, `<name>.json` (the summary) and `<name>.calls.jsonl` (one line per request). `<name>` is the
entry id when it is letters and digits, else the entry's folder name. `--json` prints the summary on
standard output as well. Without `--out` the summary goes to standard output.

Exit 0 when any entry gave output; 3 when none did, and for a wrong command line. In a run folder an
entry that cannot be read gets one line on standard error naming its folder, and the rest go on. If
decoding stops part way through an entry, its summary still comes out with `read_error` true and a line
on standard error: the counts are of what was read.

**`null` means not said, never zero.** A token key the provider did not report is `null`, and a sum
is `null` when no row said it. Every key, the merged count and the coverage line are in
[RECORD-FORMAT](RECORD-FORMAT.md#the-goalcup-verify-usage-files). Neither file holds body text or a
session file path. The step that puts these files in R2 comes with the usage job script (PRD-09); until
it merges, run the commands above by hand.

## Runs whose recording is off

Each heartbeat carries `recording`: `ok`, `gaps` (something was lost) or `failed` (recording is off now:
the recorder is in null mode and records nothing, or its writer has stalled for over a minute while records wait). `failed` is a
state, not a latch (the heartbeat section of [UPLOAD-API](UPLOAD-API.md)). The first heartbeat that says
`gaps` or `failed` flags the run, and the cron sends the participant one `recording` email. The cron also
mails silence (no heartbeat for 20 minutes, only when the server itself was well) and idle (no written model
request for 3 and 6 hours).

To find a run whose recording stopped, read the `heartbeats` list in its `fetch.json`: `recording`,
`received_at`, `seq`, `model_requests` and `counts` per heartbeat. A `failed` row names when it began.

## Storage cost and the one manual lever

R2 costs $0.015 per GB-month, Class A writes $4.50 per million after 1 million free a month, and egress is
free ([R2 pricing](https://developers.cloudflare.com/r2/pricing/)). There is no per-run size limit.

| Case | Compressed size (estimate) | Cost |
|---|---|---|
| A 2-hour run at run 1's rate | 75 to 150 MB | under $0.01 a month |
| A busy week at run 1's rate | 10 to 20 GB | $0.15 to $0.30 a month; the competition prefix is locked 180 days, so about $0.90 to $1.80 per run in all |
| 200 agents at once, a week | about 400 GB | about $6 a month |
| 1,500 busy weeks | 15 to 30 TB | $225 to $450 a month |
| A stuck or hostile client at 100 Mbit/s for 8 days | about 8.6 TB | about $130 a month, about $780 over the lock, for that one run |

The guard is a person: the Worker logs `large-run <run_id> stored=<bytes>` at every 10 GiB a run crosses, and
`stored_bytes` per entry is in every `fetch.json`. If a run misbehaves, the one lever is to delete that run's
row in `goal_cup_tracer.run_credentials`: its client then gets `401` and keeps its data, and the server stores nothing more for that run.
That is a hand write to the shared database by the owner, not code. Bytes already under the lock cannot be
deleted for 180 days.

## Bad days

Recording on the participant's machine never waits on the server, so a server problem costs upload time, not
data. The deadline stays on the server's clock; no switch moves it, and each `goalcup` client also stops on
its own clock. No error answer stops a run or ends its upload: only a `200` heartbeat answer with `stop` does,
or the client's own count to the deadline. Fix the server and the runs catch up.

| Symptom | Do |
|---|---|
| A bad deploy, or anything you cannot explain | Switch maintenance on with `npx wrangler deploy --var MAINTENANCE:on` from a clean checkout of the commit that is live now ([steps](../server/worker/README.md#maintenance-on-and-off)); do not edit `wrangler.toml`. Every call answers `503 maintenance` with `Retry-After` (default 300 s), clients back off, and the cron pauses. Any deploy during maintenance must repeat `--var MAINTENANCE:on`, or maintenance ends. Then fix forward, and switch it off by a deploy without the flag. It blocks new starts too: there is no separate pause. |
| A deploy answers runs `401`, `404` (a lost route), `410`, `413` or anything else it should not | Runs keep recording and ask again with backoff; none stops. After repeated refusals the participant's terminal and `goalcup status` say the server refuses this run and that goalcup keeps trying. Hotfix, or roll back to a 1.0 version ([Rollback](../server/worker/README.md#rollback)); the uploads catch up by themselves. |
| Supabase is down or read-only | The Worker answers `502 upstream_error`; `goalcup` retries for as long as the run lasts. Nothing is lost. Optionally switch maintenance on for a clean `503`. The cron catches up when Supabase is back. |
| R2 is down | Chunk puts answer `502`; same as above. A retried chunk is safe. |
| GitHub is down or rate-limits | Opens answer `502` (the client retries). The cron retries the judged commit (the last push to the default branch at or before `ends_at`) each tick for 48 hours; the `ended` mail waits an hour for it, then goes without it. A repository `404` is retried for an hour before it is kept as `none`; an activity API that fails for an hour falls back to the branch's last commit dated before `ends_at`, then to `commits/HEAD`. Those runs are flagged `final_head_fallback`: check them by hand (commit dates can be set by the agent, and the head may include late pushes). Check that `GITHUB_TOKEN` is set and valid: the cron logs `WARNING: GITHUB_TOKEN is not set` without it. |
| Many `429` on the device flow (a venue sharing one address) | Raise `DEVICE_START_PER_MINUTE`, `DEVICE_POLL_PER_MINUTE` or `DEVICE_TOKEN_PER_MINUTE` in `wrangler.toml` and deploy. Every device call answers `500 misconfigured` if the `DEVICE_LIMITER` binding or `RATE_LIMIT_KEY` is missing. |
| `500 misconfigured` on any call | Read `npx wrangler tail`; the detail is in the log, never in the answer. Usual causes: a missing secret, `APPROVAL_URL`, a test-only variable set in production. |
| A participant cannot approve a run | The website needs the current terms accepted (`403` or a message on the approval page). A start outside the window is `403` too, and so is a client older than `MIN_COMPETITION_VERSION` (the answer carries an install line). |
| A participant's run shows `incomplete` | Its last piece did not arrive within 24 hours of the end. It is judged on what arrived. Nothing to repair. |
| A participant lost their machine mid-run | `goalcup continue` works only on the machine that started the run. The run ends at its deadline with what was uploaded. |
| Wrong run is the main run | An account has one main run; a later competition start replaced it. `GET /v1/runs/{id}` on the first shows `ends_at` = the second's start. This is by design. |
| Emails are late | At most 20 go out per 5-minute tick. The queue drains by itself; raise `NOTICES_PER_TICK` in `mail.ts` to speed it. Check `RESEND_API_KEY` and `NOTICE_FROM` are set. |
| A secret leaked (Supabase, Resend, GitHub, Cloudflare) | Revoke it at its provider, create a new one, `npx wrangler secret put` it, deploy. Treat the old value as public. |

## Releasing

The version is written once, in the workspace `Cargo.toml`. In order:

1. If `origin/main` lacks the version, `sh scripts/release.sh prepare <x.y.z>` opens the version pull
   request; merge it under the `main` ruleset.
2. In a build box, run `sh scripts/gate.sh full` on the merged commit ([README, The gate](../README.md#the-gate)).
   CI (`.github/workflows/ci.yml`) is one job, `check`, that runs `sh scripts/gate.sh fast` on pull requests
   into `main` and pushes to `main` and `v1.0/integration`; there is no scope step and no separate Linux check.
3. `sh scripts/release.sh tag <x.y.z>` fetches, checks that `origin/main` has the version, runs
   `scripts/pad/table --release-check` on that commit, refuses the tag if it fails, then tags that commit
   and pushes only the tag.
4. The tag starts `.github/workflows/release.yml`, which builds the four targets and publishes the release.
   Check the release and the installer on a clean machine; layout and options are in
   [packaging/README.md](../packaging/README.md).
5. Then the owner deploys the Worker ([Deploy](#deploy)). Raise `MIN_COMPETITION_VERSION` only after the
   release is published and `releases/latest` serves it.

If you tested 0.7, install 1.0 fresh; 0.7 runs cannot be continued. (This line goes in the 1.0.0 release
notes.) The binaries are unsigned and not notarized. Participants install with one command, given in
[packaging/README.md](../packaging/README.md).
