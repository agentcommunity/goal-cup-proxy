# The Goal Cup server, as built

The upload server takes a run's trace while the run goes on, keeps it safe, and decides when the run
is over. This page says how it is built and why. The wire contract is [UPLOAD-API](UPLOAD-API.md).
Code, tests, configuration and local use are in [server/worker](../server/worker/README.md).
Organizer steps (deploy, test runs, bad days) are in [OPERATING](OPERATING.md).

## What it must do

1. **Keep every trace.** A bad hour for the participant's network or for the server loses nothing.
2. **Own the clock.** The server stamps the official start, sets the deadline 168 hours later, and
   tells Goal Cup Proxy to stop. The participant's clock cannot extend a run.
3. **Tell the participant** when a run goes quiet, goes idle, has a recording problem, is paused
   because the disk is almost full, or has ended.

Recording on the participant's machine never waits on the network. The daemon has no network code.

## The parts

| Part | What it does |
|---|---|
| Worker `goalcup-ingest` (Cloudflare Workers, TypeScript) | All calls under `/v1`. Holds no state. Streams chunks into R2, signs receipts and acknowledgements, runs a cron every 5 minutes. |
| R2 | Raw trace chunks, one immutable object each. Private. |
| Supabase (Postgres) | Runs, entries, chunk index, heartbeats, notices, device requests, credentials (hashes only). The Worker calls one SQL function for each step. |
| `DeviceLimiter` (Durable Object) | Counts device-flow calls per minute. |
| Resend | Sends the run emails. |
| GitHub API | Checks `repo_url` at open. Reads the last push to the default branch at or before the deadline (repository activity API). |
| Website (another repo) | Owns the database schema and migrations, sign-up, terms, the approval page, and the member's address for mail. |

This repository owns Goal Cup Proxy and the Worker. The Worker's SQL
([sql/goal-cup-tracer-upload.sql](../server/worker/sql/goal-cup-tracer-upload.sql)) is a copy of the
website's migration; the website repo is the authority. The Worker connects with a Supabase secret
key whose role can run the `goal_cup_upload_*` functions and nothing else. Row-level security is on
for every table. Production and staging each have their own key and role.

## The run, step by step

1. **Sign in.** `goalcup login` and every `goalcup start` use the device flow: the Worker issues a
   device code and a short user code, the member approves on the website, and Goal Cup Proxy picks up
   the result. Approval needs the current terms accepted. The server stores only the SHA-256 of the
   codes and the credential.
2. **Run credential.** A competition or practice approval yields a credential (`gcr_` and 64 hex).
   It is unbound until `POST /v1/runs` binds it to the run it opens. After that it can reach that run
   only; any other run id is `404`. It expires one hour after approval while unbound, and at
   `deadline_at` plus 24 hours once bound.
3. **Open.** The server checks the start window (competition only), that `repo_url` is a public
   GitHub repository, and picks the event from the credential's mode. It stamps `official_start` from
   its own clock and sets `deadline_at` (`RUN_HOURS`, or `PRACTICE_RUN_MINUTES` for practice). It
   mints `run_id` and `entry_id`. One account has one main run: a new competition start replaces the
   old one, which then ends at the new start. Only competition replaces: practice and organizer test
   runs replace nothing and are never replaced, so an operator can run several test runs at once.
4. **Receipt.** Once the daemon has the entry and its baseline, Goal Cup Proxy asks for a signed start
   receipt over the entry's facts and chain head.
5. **Record.** Every 5 minutes Goal Cup Proxy sends a heartbeat `(seq, head)`. The server stamps its
   receive time, which commits Goal Cup Proxy to that chain position before the bytes arrive. Chunks go
   every 30 minutes, or sooner at 256 MiB of chain; each is acknowledged with a signature.
6. **Segments.** If the agent dies, `goalcup continue` adds a segment to the same run: new
   `entry_id`, own chain, own receipt, same deadline. Only the machine that started the run can do
   it, with the saved credential.
7. **End.** At `ends_at` (the deadline, or the start of a replacing run) the heartbeat answer says
   `stop`. Goal Cup Proxy stops the agent, seals, sends a final heartbeat with the chain end and the
   workspace commit, and uploads up to that end for at most one hour. The server accepts nothing past
   the committed end. Without a network Goal Cup Proxy stops on its own clock.
8. **Close.** 24 hours after `ends_at` the server closes the run: `sealed` if the last entry is
   sealed, otherwise `incomplete`. It then answers `410`. There is no grace period and no late window.
9. **Commit at the deadline.** Once the run has reached its deadline the cron reads, from GitHub's
   repository activity API, the last push to the default branch that GitHub timestamped at or before
   `ends_at`, and keeps it with flags for later pushes and force-pushes
   ([UPLOAD-API](UPLOAD-API.md#what-the-server-does-on-its-own)). That commit is what judges review.
   When the activity API cannot say (unavailable, no activity, or a listing cut off by the page
   limit), the cron falls back to the commit list by commit date, then `commits/HEAD`, and sets
   `final_head_fallback`; a GitHub error status that lasts the grace hour is kept as `none` with the
   same flag. A row with `final_head_fallback` is not the rule's answer (commit dates can be set by
   the pusher, and the head may be a later push), so an organizer reviews it by hand.

## Storage

- Chunk key: `traces/{event}/{run_id}/{entry_id}/{offset:016}-{sha256}.zst`. Seal documents sit
  beside them as `seal-{sha256}.json` and `baseline-{sha256}.json`.
- Every put is create-only and R2 checks the SHA-256, so a name holds only those bytes. The body is
  piped, never held in memory.
- The bucket lock (180 days) covers the competition prefix only. The dry run, practice and test
  prefixes are not locked. No lifecycle rule deletes objects yet.
- The Worker checks headers only (offset, `prev`, first sequence, length, SHA-256). It does not check
  that the bytes form a valid chain. That is the verifier's job, over the reassembled copy.

## Signing

Receipts and acknowledgements use the verifier's existing formats
(`crates/goalcup-protocol/src/receipt.rs`, `ack.rs`) and are signed with an Ed25519 key that exists
only as a Worker secret. The verifier pins the public key (`SERVER_PUBLIC_KEYS` in
`crates/goalcup-verifier/src/lib.rs`). Changing the key means releasing a new verifier.
`goalcup-verify --dev-key <hex>` accepts another key for one run and records that it did.

## What the server stores about a person

- Hashes of the codes and credential. Never the plain values.
- The machine facts Goal Cup Proxy sends at open (operating system, architecture, CPUs, memory, disk,
  WSL2, container, mains power). No host name, user name or serial number.
- The IP address and country of the open call and the latest pair from each `continue` and heartbeat.
- The heartbeat vitals: free disk space on the machine, and the memory, CPU time and open files of
  Goal Cup Proxy's own processes. Nothing about other processes.
- None of this is in an answer, a key, a log or a public page.
- The Worker does not redact traces. Goal Cup Proxy removes secrets on the participant's machine before
  anything is written or uploaded. Raw traces are never public.

## Limits and switches

- **Rate limits** (device flow only; the other calls carry a credential): 120 starts a minute per
  client address (an IPv6 client by its /64), 30 polls a minute per device code, 1200 polls a minute
  per address as an abuse cap. Counters are Durable Objects keyed by an HMAC of the address or code
  under `RATE_LIMIT_KEY`. Over a limit is `429` with `Retry-After`. A deployment without the limiter
  answers every device call `500 misconfigured`.
- **Maintenance switch:** `MAINTENANCE=on` answers every call `503 maintenance` with `Retry-After`
  and stops the cron. Clients back off and lose nothing.
- **Sizes:** a chunk is at most 32 MiB compressed; each seal document at most 16 MiB; other bodies at
  most 64 KiB.
- **Per-run size limit:** a run may hold at most `RUN_MAX_COMPRESSED_BYTES` (64 GiB) of compressed
  chunks in R2, over all its entries. The database counts `stored_bytes` (each committed chunk's
  `Content-Length`) and `chunk_count` per entry in the same atomic step that commits the chunk, and
  every run answer carries the run's totals. The chunk upload checks the total plus the new chunk
  before it reads or stores the body: over the limit is `413 quota_exceeded`, nothing stored.
  Goal Cup Proxy keeps its data and retries, so raising the variable (a deploy) resumes the upload. The check
  reads the total of a moment ago, so concurrent uploads to several entries of one run can pass it by
  one chunk each. Follow-up (not done): make the check atomic by passing the limit to
  `goal_cup_upload_commit_chunk` and refusing in the same step that counts the bytes; that needs a
  new database signature and the R2 object is already stored by then, so it is a larger change. The
  operators see Worker log lines `quota-half` and `quota-full` when a committed
  chunk takes a run to 50% or 100% of the limit, and `quota-exceeded` on each refusal (repeated at
  Goal Cup Proxy's retry pace). These are log lines, not emails: the notice outbox mails the run's member,
  and an operator email would need a new notice kind and recipient rule in the website's database.

## Notices

The cron closes runs that are over, records deadline commits, queues emails in an outbox, and sends
them. The seven kinds (`started`, `silence`, `idle_3h`, `idle_6h`, `recording`, `ended`,
`disk_full`) and their rules are in [UPLOAD-API](UPLOAD-API.md#what-the-server-does-on-its-own). Each
is queued once for its run, kind and key, so a repeated tick sends nothing twice. Staging sends every
email to a test inbox.

Every notice is decided by the cron, never in a request handler. For `disk_full` the heartbeat only
records state: its optional `vitals` move the run's disk pause (`disk_paused_at`) and, at most once
per 6 hours, the start of a pause that is owed an email (`disk_notice_at`, the dedupe key). The cron
queues one `disk_full` per value, under the same rules as the others: competition and test runs
before their end, never practice, and a test run's email goes to its operator with `[TEST]`.

## Test runs

An organizer test run (`mode: test`) uses the test event, runs the full length, and mails the
operator only. Only accounts on the database's test-operator list can approve or open one. A test
run replaces nothing and is never replaced, so an operator may run several at once. Test runs
are left out of every public number. `server/worker/scripts/delete-test-runs.sh` deletes all of them.
See [OPERATING](OPERATING.md#test-runs).

## Live totals

Built, not yet live: it needs website migration 20261022100000 applied, then a Worker deploy
([OPERATING](OPERATING.md#deploy)). `GET /v1/live.json` is the one public, unauthenticated read: the
community totals of the current event, from the database the server already fills (agents started,
running and quiet, harness mix, agent hours, model calls, restarts, data stored, notices). The cron's
last step calls one SQL function (`goal_cup_upload_live_snapshot`) and writes the answer to R2 as
`live/{event_id}.json`; the GET serves that copy (each data center keeps it for 60 seconds) and
never calls the database, so traffic costs nothing there and a bad database minute only makes the
copy older. The SQL reads per-run columns and the last hour of heartbeats only, never a whole week
of them: about 20 ms at event scale. The Worker rebuilds the document
key by key before it writes it, so only the published fields can exist. Test and practice runs are
left out in the SQL. A number fed by fewer than 5 runs is `null`, a harness with fewer than 5 runs
folds into `other`, and call and byte totals are rounded down so two snapshots cannot be compared
to read one run. Nothing per run or per member is in it. Contract:
[UPLOAD-API](UPLOAD-API.md#get-v1livejson--public-community-totals).

The `github` block (website migration 20261023100000, `GITHUB_STATS=on`) adds commits, pull
requests and lines from the entries' public repositories. The GitHub reader is one more cron step,
after the snapshot and on its own: at most 20 runs a tick, three cheap calls each (the commit count
from the `Link` header, a search count of the owner's pull requests, `code_frequency`), about
hourly per run, and one last reading after the end, counted up to the judged commit, after which the
run is never read again. It stops for the tick on a rate limit, on an unreachable GitHub, or when
fewer than 1,000 calls are left in the hour, so the judged commits and the start check keep their
share of the token. The database stores numbers only, one row per run (`run_github`), and the
snapshot sums them per repository with the same group of 5; lines are floored to 100. Details:
[UPLOAD-API](UPLOAD-API.md#the-github-numbers).

## Not built

- The verify job that reassembles stored chunks and runs `goalcup-verify` over them. Until it exists,
  nothing downloads the stored chain; `scripts/matrix.sh --check-copy` compares positions instead.
- The rest of the live stats design: working hours (it needs a stored counter), survival and the
  hourly strip, tokens and tool calls from the traces (the verify job), uptime and outage windows,
  the daily copies. Their fields in `live.json` are `null`.
- A separate organizer pause on new starts (`MAINTENANCE` stops everything).
- A second pinned signing key for rotation.
- Teams, and more than one main run per account.
