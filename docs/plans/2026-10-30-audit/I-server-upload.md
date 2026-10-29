# Lane I: server and upload

Audit of the whole upload path, from `goalcup start` on the participant's machine to the Worker,
R2 and the database. Read-only. Line counts are `wc -l`; "src" is shipped code, "tests" is test
code (Rust `#[cfg(test)]` blocks at the end of a src file count as tests). Numbers marked "est." are
estimates, not counts. Web facts carry links. Anything not checked is marked "unverified".

The 2-hour run report (`docs/research/results/2026-10-30-two-hour-run.md`) does not exist on any
branch yet, so nothing below relies on it. There is no measured run with many subagents uploading.

## 1. Short answer

- **The server already has what it needs to rebuild the story by ID and time.** It is all inside
  the chain the client uploads: one hash-linked `chain.jsonl` per segment, where every record has
  a `seq` (total order), a stream ID (`producer` + `stream`), capture times (`wall`, `mono_ns`,
  `clock_id`) and, for a subagent file, its path as the label. The server adds its own receive
  times (heartbeats every 5 minutes, chunks every 30 minutes). No new field is needed for point 5.
- **Most of the Worker is not upload.** Of 4,001 src lines, about 1,100 to 1,250 serve emails,
  GitHub, device rate limits, signing, abandon and vitals. None of those serves one of the seven
  points. The chunk path itself (`chunks.ts`, 228 lines) is small and sound.
- **The contract has five copies:** the client, the Rust stand-in server for tests (2,514 test
  lines), the Worker, its local database (`db-local.ts`, 701 lines) and the SQL in the website repo
  (4,468 lines mirrored here). Every rule change touches all five.
- **Real gaps for point 5:** the rebuild script writes nothing for a segment whose tail never
  arrived (`scripts/fetch-run.sh:218-219`); the server refuses the seal documents until the whole
  chain is up (`server/worker/src/runs.ts:575-576`); and the server stops taking bytes 24 hours
  after the run ends (`server/worker/src/db.ts:17`), a cap of our own.
- **Cloudflare limits do not bite** for a week-long upload with one R2 object per chunk, as built.
  R2 multipart would be the wrong tool: unfinished multipart uploads are cancelled after 7 days,
  which is exactly the run length.

## 2. What the server needs to rebuild the story

| Need (point 5) | Where it lives today | Verdict |
|---|---|---|
| Run identity: who, which event, start, deadline, end | `runs` row: `member_id`, `event_id`, `official_start`, `deadline_at`, `replaced_at` (`server/worker/sql/goal-cup-tracer-upload.sql:92-117`) | keep |
| Segment order | `entries.segment`, `UNIQUE (run_id, segment)` (`sql:132-162`) | keep |
| The bytes, in order | `chunks` rows `(entry_id, offset)` with `first_seq`, `last_seq`, `prev`, `head`, `r2_key` (`sql:203-216`); R2 key `traces/{event}/{run}/{entry}/{offset:016}-{sha256}.zst` (`server/worker/src/chunks.ts:155-158`) | keep |
| Order inside a segment | Chain `seq`, `prev`, `hash` on every line (`crates/goalcup-protocol/src/envelope.rs:9-26`) | keep, already there |
| Stream identity | `StreamKey { producer, stream }` on `Open`, `Segment`, `End`, `Gap` (`crates/goalcup-protocol/src/capture.rs:79-82`, `256-345`) | keep |
| Subagent identity | A native stream's label is source name + relative path + generation (`crates/goalcup-daemon/src/native_capture.rs:1560-1574`). The path is the ID, as the owner decided. Wire calls carry a connection ID and exchange number (`capture.rs:108-112`); which subagent made a wire call is a decoder question, not a server one | keep |
| Client times | `CaptureTime { wall, mono_ns, clock_id }` per record, `first`/`last` per `Segment`, `recorded_at` per line (`capture.rs:88-92`, `envelope.rs:19`) | keep |
| Server times (what the participant's clock cannot move) | `official_start` at open; `heartbeats.received_at` with `(seq, head)` every 5 min (`sql:164-178`); `chunks.received_at` | keep |
| Recording gaps | `Gap` and `Loss` records in the chain (`capture.rs:191-205`, `226-243`) | keep |
| Upload gaps (bytes that never arrived) | Server offset vs the final heartbeat's committed `(seq, head)` and `offset_local` | keep; fix the rebuild script (section 9) |
| The end of a segment | Final heartbeat `(seq, head)`, then `seal.json`, `baseline.json` | keep; loosen the order (section 10) |
| Hosts passed through unread | `tunnels.jsonl` beside the seal (D74) | keep the file, stop parsing it on the server |
| Who may run (sign-up gate) | Device flow plus the website's approval page and SQL gate | keep for Friday (section 4) |

Nothing in the list needs signatures, emails, GitHub reads, vitals, IP addresses or rate limits.

## 3. The upload path today

Client (`crates/goalcup-cli`):

1. `goalcup start` asks the participant to approve the run in the browser (device flow,
   `src/account/device.rs`), gets a run credential, and opens the run (`POST /v1/runs`,
   `src/upload/mod.rs:141`).
2. The daemon creates the segment under the server's `entry_id`. The client asks for a signed
   start receipt over the baseline hash (`POST .../receipt`, `src/upload/mod.rs:243-295`).
3. While the harness runs, a background uploader reads the chain the daemon writes and never
   touches it. It sends one zstd chunk at launch, then every 30 minutes, or sooner once 256 MiB
   waits (`src/upload/policy.rs:83-96`). A heartbeat goes every 5 minutes. Each chunk's signed
   ack is checked and kept in `acks.jsonl`. Parts the server acked are deleted locally after
   2 hours or under disk pressure (`src/upload/prune.rs:31`).
4. At the end: stop, seal locally, final heartbeat with `(seq, head)`, upload the rest, then
   `POST .../seal` with `seal.json`, `baseline.json`, `tunnels.jsonl`, for up to one hour
   (`policy.rs:93`). What is left goes with `goalcup upload` by hand.

No step waits on the network while the harness records (`src/upload/uploader.rs:8-12`), and no
server answer stops a run except `stop: true` (D64). This holds point 3 and must stay.

Worker routes today (`server/worker/src/index.ts:23-58`):

| Route | Purpose | Point served |
|---|---|---|
| `GET /v1/health` | liveness | none (ops, 1 line) |
| `POST /v1/device`, `POST /v1/device/token` | sign-in, run credential | gate for 5 (who the run belongs to) |
| `POST /v1/device/approve` | test-only approval, staging and local | tests only |
| `POST /v1/runs` | open, server clock, ids | 5 |
| `POST /v1/runs/{id}/continue` | next segment | 2, 5, 7 |
| `GET /v1/runs/{id}` | server position | 5 |
| `PUT /v1/runs/{id}/chunks/{offset}` | store chain bytes | 5 |
| `POST /v1/runs/{id}/heartbeat` | time anchor, stop signal, end commit | 5 (and the deadline rule) |
| `POST /v1/runs/{id}/seal` | seal documents, tunnel list | 5, 6 |
| `POST /v1/runs/{id}/receipt` | signed start receipt | none |
| `POST /v1/runs/{id}/abandon` | give back a run that never launched | none |
| cron every 5 min | close runs, GitHub judged commit, queue and send 7 kinds of email | close: 5; the rest: none |

## 4. Verdicts, Worker and database

Format: Part (path, src lines, test lines) | What it does | Points | Verdict | Reason | Lines saved (src / tests) | Risk | Tests that go.

| Part | What it does | Points | Verdict | Reason | Saved src / tests | Risk | Tests that go |
|---|---|---|---|---|---|---|---|
| Chunk upload (`server/worker/src/chunks.ts`, 228 / in `integration`, `segments`, `quota` tests) | Header checks, retry rule, streams body to R2 create-only with R2 checking SHA-256, one atomic DB commit | 5 | keep | Small, streams without holding the body, idempotent. The one-object-per-chunk layout is right for a 7-day upload (section 7) | 0 / 0 | none new | none |
| Per-run size limit `RUN_MAX_COMPRESSED_BYTES` (`chunks.ts:117-142`, ~26; `quota.test.ts` 114) | 413 when a run's compressed bytes pass 64 GiB | none directly; guards a real cost | keep, owner sets the value | It guards R2 storage cost against a stuck or hostile client and says so (`wrangler.toml` comment). At $0.015 per GB-month ([R2 pricing](https://developers.cloudflare.com/r2/pricing/)) 64 GiB is about $1 a month per run; 1,500 runs at the cap would be about $1,450 a month. Measured busy Hermes: about 600 MB/h raw, 5 to 7 GB compressed per 168 h (`docs/research/results/2026-10-22-local-test-day.md:133`), far below the cap | 0 / 0 | A run with thousands of subagents is unmeasured; if it passes the cap the client keeps data and retries, so nothing is lost until the run closes | none |
| Run open, continue, position (`runs.ts:211-312`, `418-438`) | Server clock, ids, one main run, start window, segment add | 5, 2, 7 | keep | The server is the clock (D16). Continue keeps a run whole across harness restarts | 0 / 0 | none | none |
| GitHub checks at open (`runs.ts:314-351`, 38) | Repo must exist, be public and empty | none (a rule in RULES.md, D71) | change: keep one copy | The client already checks the same thing before approval (`crates/goalcup-cli/src/commands/run_repo.rs:128`, `account/github.rs:294`). Two copies of one rule. Keep the Worker's (it cannot be skipped); make the client's a hint or drop it | 0 / 0 here (client side below) | GitHub outage at open already answers 502 and the client retries, fine | none |
| Heartbeat core (`runs.ts:444-491`) | `(seq, head, offset_local, final)` stamped with server time; answers `stop`, `seconds_left` | 5 | keep | The 5-minute server-time anchors are the strongest time evidence the server has. `stop` and `seconds_left` drive the deadline (D64) | 0 / 0 | none | none |
| Heartbeat extras: `vitals`, `model_requests`, `recording`, `repo_head`, IP and country (`runs.ts:452-480`, `499-535`; `db-local.ts:597-658`) | Disk pause state, idle and flag emails, workspace commit, client address | none | cut | Vitals and the disk pause go with the owner's "disk full: no special handling". `model_requests` and `recording` only feed emails. `repo_head` repeats what `seal.json` already holds (D56). IP and country serve no point and are personal data | about 110 / about 300 (est.) | Losing IP removes one signal for spotting shared accounts (owner decision). The SQL functions still take these arguments: pass `null`, no migration needed | vitals and disk-pause cases in `notices.test.ts`, `lifecycle.test.ts`, `local-db.test.ts` |
| Seal (`runs.ts:540-615`, 76 plus helpers) | Stores `seal.json`, `baseline.json`, `tunnels.jsonl`; needs final heartbeat and full chain first | 5, 6 | change | Keep storing the documents. Stop parsing the tunnel list (`runs.ts:73-86`): the server stores bytes. Later (needs a website migration): accept the documents once the end is committed, even if the chain tail is not up yet. The JSON body of up to 40 MiB is parsed whole in a 128 MB isolate (`runs.ts:29`, `35-47`); peak memory is unmeasured (unverified risk) | 14 now; about 60 later / about 80 | Changing seal order needs the SQL `goal_cup_upload_seal_run` changed in the website repo | tunnel format-line tests in `integration.test.ts` |
| Receipt route and signing (`runs.ts:352-392`, 41; `sign.ts` 116; `RECEIPT_SIGNING_KEY` secret; `sign.test.ts` 68, `test/vectors/signing.json` 26) | Ed25519 start receipt and per-chunk ack in the verifier's format | none | cut (cross-crate, see section 5) | The server's own copy is what is judged (`scripts/fetch-run.sh`). The server's DB already records what it got and when. Signatures only let someone verify a participant's local copy without trusting the organizers, which Goal Cup does not need | about 160 here; about 1,000 total with client, protocol, verifier (est.) / about 2,500 total (est.) | Biggest risk in this lane: touches verifier, protocol, client, Worker and corpus at once. `chunks.ack` is `NOT NULL` (`sql:214`): keep writing `{}` to avoid a migration. Loses the local-copy anchor described in `docs/CHEAT-CHECKLIST.md:14-22` | `sign.test.ts`, signing vectors, receipt cases in `integration`, `lifecycle`, `segments` |
| Abandon (`runs.ts:393-417`, 25; DB functions) | Gives back a run that opened but never launched | none | cut | A failed competition start is replaced by the next start anyway (one main run per account). An empty practice or test run is harmless test data | 25 + about 40 in db layers / about 120 (est.) | An empty open run stays until its deadline; nobody is hurt | abandon cases in `integration.test.ts`, `local-db.test.ts`, `supabase.test.ts` |
| Device flow (`device.ts` 146; `auth.ts` 29; tests 393) | Browser approval on the website, run credential, sign-up gate | gate for 5 | keep for Friday | It ties a run to a signed-up member, and the approval page and sign-up gate live in the website repo and the shared database. Replacing it costs a website change in 7 days. The `login` purpose (approve with no run) could go later | 0 / 0 | none new | none |
| Test approval route (`device.ts:120-146`, 27) | Approves as any member, staging and local only | tests | keep | It lets tests and the Vercel sandbox run without a browser. Production answers 404 | 0 / 0 | Must never get a key in production (already guarded) | none |
| Rate limiter Durable Object (`ratelimit.ts` 139; `RATE_LIMIT_KEY`; DO migration; `ratelimit.test.ts` 184; 3 vars) | Per-minute limits on device start and poll | none | cut, replace with one WAF rule | The device code is 256 random bits, so poll limits guard nothing. The start limit guards only junk rows in `device_requests`. A zone rate-limiting rule does that with no code: Free plan has 1 rule, by IP, 10 s period; Pro has 2, up to 60 s ([WAF rate limiting rules](https://developers.cloudflare.com/waf/rate-limiting-rules/)) | about 170 / 184 | Whether the `agentcommunity.org` zone's rule slot is free is unverified. Removing a DO class needs a `deleted_classes` migration in `wrangler.toml` | `ratelimit.test.ts`, limit cases in `device.test.ts`, `config.test.ts` |
| Cron: close runs (`notices.ts:56-63`) | Writes `sealed` or `incomplete` once a run is over | 5 | keep | Small. Status is also computed on read (`db.ts:48`) | 0 / 0 | none | none |
| Cron: judged GitHub commit (`github.ts` 223; `notices.ts:68-100`; `GITHUB_TOKEN`; `github.test.ts` 184) | Reads the last push before the deadline from GitHub's activity API | none (a judging rule) | cut from the Worker; owner decides | Judging, not recording. An organizer script run after the event can read the same API. How long GitHub keeps repository activity is unverified, so the script must run within days | about 250 / about 400 (est., incl. notice cases) | If no one runs the script, the judged commit is not fixed; a deleted or private repo after the event loses it. This is the one cut with judging risk | `github.test.ts`, final-commit cases in `notices.test.ts`, `local-db.test.ts` |
| Cron: emails (`notices.ts` rest, `mail.ts` 70; outbox, `notice_text` in the website DB; `RESEND_API_KEY`; `notices.test.ts` 1,064, `mail.test.ts` 108) | `started`, `silence`, `idle_3h`, `idle_6h`, `recording`, `ended`, `disk_full` | none (point 6 is the terminal) | cut all but `ended`, or all | `disk_full` goes with the disk decision. `silence` and idle are run coaching. `ended` is the only one that helps point 5: it tells a participant whose upload did not finish to run `goalcup upload`. Also: at most 20 sends per tick (`mail.ts:18`), so 1,500 `started` mails would take about 6 hours | about 250 / about 900 (est.) keeping `ended` | Without `silence` a participant with a sleeping laptop is not told; the run still uploads later. A known gap already exists: a lone run never gets `silence` (`docs/UPLOAD-API.md:574-579`) | most of `notices.test.ts` |
| Health marker in R2 (`notices.ts:104-131`) | Suppresses `silence` during our own outages | none | cut with `silence` | Only exists for `silence` | about 30 / in notices tests | none | as above |
| `db-supabase.ts` 433, `db-local.ts` 701, `db.ts` 485 | One DB call per step; local backend for dev and tests | 5 | change | Remove methods and types for notices, tick, final commit, receipt, abandon, vitals | about 350 (est.) / in the tests above | `db-local.ts` must keep mirroring the SQL for what stays | as above |
| SQL mirror (`server/worker/sql/goal-cup-tracer-upload.sql`, 4,468) | Copy of the website's migrations | 5 | keep as a copy; do not change for Friday | The website repo owns it. Unused functions do no harm. Every Friday cut above works with the functions as they are | 0 / 0 | A migration is owner-approved work on the shared production database (website `CLAUDE.md`) | none |
| Maintenance switch (`index.ts:68-75`) | 503 for every call | 3 (the client backs off, loses nothing) | keep | 8 lines, lets the owner fix a bad deploy without failing runs | 0 / 0 | none | none |
| `MIN_COMPETITION_VERSION` (`runs.ts:144-165`) | Refuses old clients in the competition | 5 (with no backward compatibility, old clients must be refused) | keep, and set it | D70. Needed more now that the protocol may change | 0 / 0 | Must be set before the start window | none |

Worker total if all Friday cuts are taken: about 1,100 to 1,250 of 4,001 src lines, about 1,900 to
2,200 of 5,800 test lines (est.).

## 5. Verdicts, client, protocol, verifier, scripts

| Part | What it does | Points | Verdict | Reason | Saved src / tests | Risk | Tests that go |
|---|---|---|---|---|---|---|---|
| Uploader core: chunk, session, lifecycle, finish, local (`crates/goalcup-cli/src/upload/uploader/*.rs`, 1,903 / 395) | Builds chunks from the server's position, sends, retries, wrap-up | 5, 3 | keep | Never waits on the network, never stops the run (D64) | 0 / 0 | none | none |
| Heartbeat thread (`uploader/heartbeat.rs`, 308 / 109) | Beats, stop, own deadline count | 5 | change | Drop vitals, `model_requests`, `recording`, `repo_head` fields | about 30 / about 30 | none | vitals cases |
| `src/vitals.rs` (158 / 51) | Disk and process facts for heartbeats | none | cut | Goes with the disk decision and the Worker cut | 158 / 51 | none | its unit tests, vitals checks in `tests/it/upload_server.rs` |
| Policy and backoff (`upload/policy.rs`, 292 / 93) | 30-min cadence, 256 MiB eager, 60 s to 15 min backoff, 1-hour wrap-up | 3, 5 | keep | These are timings, not caps. The 1-hour wrap-up is fine because `goalcup upload` finishes the rest | 0 / 0 | none | none |
| Receipt and ack handling (`upload/mod.rs:243-295`, ack checks in `uploader/session.rs`, `uploader/local.rs`; `goalcup-protocol/src/receipt.rs` 186, `ack.rs` 95; verifier `reader.rs:406-443`, `487-494`, `620-675`, `908-983`, `1238-1299`, plus D63 launch check) | Fetch, check and keep signed receipt and acks; verifier checks them under a pinned key | none | cut (same change as the Worker row) | See section 4. Pruning then uses the server's position instead of a signed ack | about 850 (est.) / about 2,000 (est., incl. signed corpus fixtures and upload tests) | Large change across five crates and the v5 corpus; do it only after the deletion-only cuts, with the verifier reporting the anchor as "server copy" | signed-receipt corpus cases, ack cases in `tests/it/upload_*.rs` |
| Abandon and give-back (`upload/mod.rs:297-495` minus `keep_run`, about 160; `pending-open.json`) | Gives back a run when start fails before launch, and after a killed start | none | cut | Same reason as the Worker row | about 160 / about 300 (est.) | none | give-back cases in `tests/it/upload_open.rs` |
| Old GitHub token flow (`account/github.rs`, about 250 of 466: `GithubSetup`, token checks, `credential_helper`; hidden `git-credential` command, `src/main.rs:233`, `329`) | Answers git for clones made before 0.7 | none | cut | No backward compatibility before Edition 01. The file says itself it can go after the dry runs (`account/github.rs:9-14`) | about 280 / about 150 (est.) | A 0.6.1 clone still on a machine stops pushing; that is test data | git-credential cases in `tests/it/account.rs` |
| Client GitHub public-and-empty check (`account/github.rs:294-337`; `commands/run_repo.rs:128`) | Same rule as the Worker | none | change: keep only as early advice, or cut | Duplicate of the Worker check | about 45 / small | Without it the participant learns of a non-empty repo one step later (after approval) | some in `tests/it/account.rs` |
| Prune (`upload/prune.rs`, 173 / 248) | Deletes acked parts after 2 h, or at once under disk pressure | none directly (disk) | change | Keep only "delete parts the server holds, 2 hours later", read from the server's position, not a signed ack. Drop the disk-pressure trigger with the disk decision | about 60 (est.) / about 100 (est.) | Without any prune, a busy run (100 GB raw a week) can fill a laptop; with no prune at all the owner's "user cleans up" applies | disk-pressure cases |
| `goalcup upload` (`commands/upload.rs`, 343 / 83) | Finishes an upload by hand | 5 | keep | The only way to send what the 1-hour wrap-up left | 0 / 0 | none | none |
| Rust stand-in server (`crates/goalcup-cli/tests/common/upload_server/`, 0 / 2,514) | A second server implementation the Rust tests run against | none | cut, run Rust upload tests against `wrangler dev` with the local DB | `tests/full/upload_worker.rs` already does this. One server, not two | 0 / 2,514, plus about half of 6,193 lines in `tests/it/upload_*.rs` (est.) | Rust tests then need Node and `npm ci`; slower; port flakiness. Lane J may decide where tests run | `upload_server*.rs`, parts of `upload_chunks.rs`, `upload_open.rs` |
| `scripts/fetch-run.sh` (349) | Rebuilds a run from DB index and R2, for the verifier | 5 | change | It refuses to write a segment whose chunks stop before the committed end (`fetch-run.sh:218-219`). For point 5 it must write what arrived and say what is missing | about -10 (adds) / tests in `scripts/test_fetch_run.py` change | none | none |

## 6. Limits: real cost, or a cap of our own

| Limit | Where | Guards | Verdict |
|---|---|---|---|
| Chunk at most 32 MiB compressed | `chunks.ts:24`, `uploader/chunk.rs:13` | Worker request body (100 MB on Free and Pro zones) and memory | Real (below the platform limit). Keep |
| Seal documents 16 MiB each, tunnel list 8 MiB, 40 MiB body | `runs.ts:26-29` | 128 MB isolate memory while parsing JSON | Real, but better solved by sending each file as its own raw PUT that streams (section 10) |
| Other JSON bodies 64 KiB | `runs.ts:25` | memory, abuse | Real, cheap. Keep |
| `RUN_MAX_COMPRESSED_BYTES` 64 GiB | `wrangler.toml` | R2 storage cost | Real cost, says so. Owner sets value |
| Close 24 hours after the run ends; credential expires deadline + 24 h | `db.ts:17`, `db.ts:57` | Nothing that costs money: a judging timeline | Cap of our own. Point 5 says upload everything. Proposal: keep accepting chunks up to the committed final end until judging closes (owner picks the date) |
| Wrap-up 1 hour in `goalcup start` | `policy.rs:93` | The participant's terminal not hanging | Not a cap on data (`goalcup upload` continues). Keep |
| Device rate limits 120 / 30 / 1200 a minute | `wrangler.toml` vars, `ratelimit.ts` | junk rows from an unauthenticated route | Real but tiny; a zone WAF rule does it with no code |
| 20 emails per tick | `mail.ts:18` | Resend rate | Real, moot if emails go |
| 50 GitHub runs per tick, 5 pages, 8 s per call | `notices.ts:36`, `github.ts:28-32` | GitHub rate limits, cron time | Real, moot if moved to a script |
| Per-run size check reads a total "of a moment ago" | `chunks.ts:111-128` | (a known race, one chunk per entry) | Acceptable; do not fix for Friday |

Not on the server, but met on the way: the native leg's 32 open streams and 1,024 files, and 128
live producers. Those are daemon limits (other lanes). The server sets no per-stream or
per-subagent limit, and should not.

## 7. Cloudflare and database limits for a week-long upload

| Limit | Value | Source | Effect here |
|---|---|---|---|
| Worker request body | 100 MB (Free, Pro), 200 MB (Business), Enterprise more; set by the zone's plan, not the Workers plan | [Workers limits](https://developers.cloudflare.com/workers/platform/limits/) | 32 MiB chunks and a 40 MiB seal body fit on any plan. The zone's plan is unverified |
| Worker CPU per HTTP request | 30 s default, up to 5 min (paid) | same | Streaming a chunk to R2 uses little CPU; R2 checks the SHA-256. No `limits.cpu_ms` is set and none is needed |
| Worker wall time per HTTP request | no hard limit while the client stays connected | same | A slow home upload of 32 MiB is fine; the client gives a PUT 600 s (`policy.rs:14`) |
| Worker memory | 128 MB per isolate | same | Matters only for the JSON seal body (section 4) |
| Cron CPU and wall | 30 s CPU for intervals under 1 hour; 15 min wall | same | The tick is light if emails and GitHub go |
| Subrequests | 10,000 per invocation (paid) | same | Not near |
| R2 single PUT | 5 GiB | [R2 limits](https://developers.cloudflare.com/r2/platform/limits/) | 32 MiB per object, far below |
| R2 object size | about 5 TiB | same | not reached |
| R2 writes to the same key | 1 per second | same | Keys are unique (offset + hash), create-only |
| R2 key length | 1,024 bytes | same | Keys are about 120 bytes |
| R2 multipart | parts at least 5 MiB and all equal except the last, at most 10,000 parts, unfinished uploads cancelled after 7 days by default | [R2 multipart](https://developers.cloudflare.com/r2/objects/multipart-objects/) | Do not assemble a run into one multipart object: a 168-hour run plus wrap-up outlives the 7-day default, and chunk sizes vary. One object per chunk, as built, is right |
| R2 price | $0.015 per GB-month; Class A $4.50 per million; egress free | [R2 pricing](https://developers.cloudflare.com/r2/pricing/) | At least 336 chunks per segment per week (parts rotate every 30 min, D55); 1,500 runs is about 0.5 million Class A operations, inside the 1 million free each month |
| D1 (if anyone proposes it) | 10 GB per database, single-threaded, 2 MB per row, 30 s per query | [D1 limits](https://developers.cloudflare.com/d1/platform/limits/) | Not needed. Moving off Supabase would cut the website coupling but lose the sign-up gate and members, which live there |
| Supabase API statement timeout | 8 s for API roles by default; the Worker's roles set 8 s (`sql:240`, `sql:1823`) | [Supabase timeouts](https://supabase.io/docs/guides/database/timeouts) | Each step is one small function; fine |
| Supabase disk | Pro includes 8 GB and auto-grows unless the spend cap blocks it | [Database size](https://supabase.com/docs/guides/platform/database-size.md) | Heartbeats: 2,016 rows per run per week; 1,500 runs is about 3 million rows (est. a few hundred MB). Chunks: about 0.5 million rows with a JSON ack each. Fine, but it is the website's shared production database |

## 8. Decisions behind this area

| Decision | What it says | Still holds under the seven points and owner decisions? |
|---|---|---|
| D16 | The server fixes the clock; the signed receipt gives start and deadline | The clock part holds (open answer, `seconds_left`). The signed receipt part does not serve a point; amend D16 if signing goes |
| D55 | Chain rotates into parts at 256 MiB or 30 min; acked parts may be deleted locally | Holds. If acks are no longer signed, "acked" becomes "the server's position says it holds them" |
| D63 | A signed receipt must match the Launch it anchors | Goes with the receipt cut |
| D64 | No server error stops a run or ends its upload | Holds; it is point 3. Keep |
| D65 | On a nearly full disk, pause the agent | Overruled by the owner ("disk full: no special handling"). Vitals, `disk_full` email and the pause go together |
| D70 | `MIN_COMPETITION_VERSION`; `--dev` never competes | Holds, and matters more now that the protocol may change |
| D71 | Project folder is the workspace; repo must be public and empty | Holds as a rule (RULES.md, not a point). Keep one check, not two |
| D69 | Goal Cup holds no keys | Holds; the old token flow it kept for 0.6.1 clones can go (no backward compatibility) |
| D74 | Tunnel list is a side file sent with the seal | Holds for points 3 and 6 (what passed unread). The server should store it, not parse it |
| (none) | Emails, rate limits, quota, judged commit | No D number. They came from event-readiness work packages (WP-A, WP-C, WP-D). Under "how we build" each must justify itself: only the quota guards a real cost |

## 9. Hidden couplings

1. **The website owns the database.** The SQL here is a copy (`sql:1-24`). Any change to a function
   signature or a `NOT NULL` column is a website migration, applied by hand to the shared
   production database. So for Friday, prefer cuts that only stop calling functions, and pass
   `null` or `{}` where a column needs a value (`chunks.ack` is `NOT NULL`, `sql:214`).
2. **Five copies of the contract.** Client, Rust stand-in, Worker, `db-local.ts`, SQL. Cutting the
   stand-in removes one; the local DB must keep mirroring the SQL for what stays.
3. **Acks drive local deletion.** `prune.rs` deletes only parts a signed ack proved
   (`uploader/session.rs:197-200`). Cutting signing must move prune onto the server's position in the
   same change, or prune deletes nothing (safe) or too much (unsafe).
4. **Final heartbeat, seal and chunks are ordered.** After `ends_at` a chunk is `503
   awaiting_final` until the final heartbeat; the seal is `409` until the chain reaches the
   committed end (`chunks.ts:61-69`, `runs.ts:567-576`). A client change in this order breaks the
   end of every run. Change all three together, or not at all.
5. **The `ended` email depends on the judged commit** (`docs/UPLOAD-API.md:553`). Moving GitHub off
   the cron changes when `ended` is queued.
6. **Credential expiry equals close time.** A run past `deadline_at + 24 h` is `401`, not `410`
   (`docs/UPLOAD-API.md:458-460`). Lengthening the close window must lengthen the credential too.
7. **Deploy order.** The Worker must be deployed before a client that sends a new field
   (progress log, 28 Oct). With no backward compatibility, ship Worker and client the same day and
   set `MIN_COMPETITION_VERSION`.
8. **The rebuild script is the only reader.** `scripts/fetch-run.sh` reads the chunk index from the
   DB (an R2 listing can hold an orphan from a lost race, `fetch-run.sh:6-7`). The "verify job"
   that `docs/CHEAT-CHECKLIST.md:26-27` says runs every hour does not exist (`docs/SERVER-DESIGN.md`,
   "Not built"). The docs disagree.
9. **Secrets the owner must hold to deploy from a laptop:** `RECEIPT_SIGNING_KEY`, `SUPABASE_URL`,
   `SUPABASE_KEY`, `RESEND_API_KEY`, `GITHUB_TOKEN`, `RATE_LIMIT_KEY`. The cuts leave two
   (`SUPABASE_URL`, `SUPABASE_KEY`).

## 10. The smallest upload protocol and server that meets point 5

Principles: the chain is the story; the server stores bytes in order, stamps its own time, and
answers "where am I" and "stop". It interprets nothing.

**Routes (8, from 12 plus a cron with 4 steps):**

| # | Route | Body | Answer | Note |
|---|---|---|---|---|
| 1 | `GET /v1/health` | none | `{status: ok}` | as now |
| 2 | `POST /v1/device` | purpose, mode, connector, version | device code, user code, URL | as now (website approval page unchanged) |
| 3 | `POST /v1/device/token` | device code | credential once | as now |
| 4 | `POST /v1/runs` | connector id, digests, harness and proxy version, `session_id`, `repo_url`, `dev` | `run_id`, `entry_id`, `official_start`, `deadline_at`, `seconds_left` | drop `machine`, IP and country |
| 5 | `POST /v1/runs/{run}/continue` | previous entry, same fields | new `entry_id`, segment, `seconds_left` | as now |
| 6 | `GET /v1/runs/{run}?entry_id=` | none | `offset`, `next_seq`, `head`, `final`, `stop`, `seconds_left` | drop `last_ack` |
| 7 | `PUT /v1/runs/{run}/chunks/{offset}` | zstd frame, headers as now | `offset`, `next_seq`, `head`, `received_at` | no signature |
| 8 | `POST /v1/runs/{run}/heartbeat` | `entry_id`, `seq`, `head`, `offset_local`, `final` | `stop`, `reason`, `deadline_at`, `seconds_left` | drop vitals, counts, flags, `repo_head` |
| (9) | `POST /v1/runs/{run}/seal` for Friday; later `PUT /v1/runs/{run}/entries/{entry}/files/{seal.json, baseline.json, tunnels.jsonl}` | raw file, streamed, create-only by hash | `200` | Later form: accepted once the end is committed, without waiting for the chain tail; no JSON wrapping, no parsing |

`POST /v1/device/approve` stays for staging and local tests only. Cut: `receipt`, `abandon`.

**Cron:** one step, close runs that are over. Optionally the `ended` email (owner decision).
Judged commit: an organizer script after each deadline.

**Storage:** unchanged. R2 `traces/{event}/{run}/{entry}/{offset:016}-{sha256}.zst` plus the seal
files; DB tables `runs`, `entries`, `chunks`, `heartbeats`, `run_credentials`, `device_requests`
(website-owned). No new table.

**Rebuild:** `scripts/fetch-run.sh` joins chunks in offset order per segment, segments in order,
writes what arrived even when the tail is missing, and prints the missing range as an upload gap.
The verifier reads the result.

**What this keeps for each point:** 5 (all of it), 3 (D64 retries, maintenance switch, no caps that
stop recording), 6 (the tunnel list and the terminal), 2 and 7 (continue, no per-stream limits on
the server).

## 11. What could go wrong (blind spots)

- **Signing cut:** if anyone later needs to prove to an outsider what the server held at a time,
  the DB rows are the only witness, and the organizers control them. Goal Cup's judges trust the
  organizers, so this is acceptable, but say so in RULES.
- **Judged commit moved to a script:** a person must run it within days of each deadline; GitHub
  activity retention is unverified.
- **Emails cut:** participants with a sleeping laptop or an unfinished upload hear nothing by
  email. Keep `ended` if in doubt.
- **WAF rule instead of the limiter:** zone plan and free rule slot unverified; if the slot is used,
  keep the Durable Object for Friday.
- **Rust stand-in cut:** Rust tests then depend on Node and `wrangler dev`; slower and possibly
  flaky.
- **Throughput with thousands of subagents:** unmeasured. The design is one sequential uploader per
  segment. At 10x compression and a 10 Mbit/s uplink, one 32 MiB chunk takes about 27 s, so
  about 4 GB of raw chain an hour can keep up (est.). Beyond that a backlog grows on disk and
  uploads later; nothing is lost while the disk lasts.
- **Seal body memory:** a 40 MiB JSON seal body in a 128 MB isolate is untested at the top size.
- **Website migration needed for the later steps** (seal order, close window, credential expiry):
  owner approval and `db:check` on the shared production database. Not for Friday.

## 12. Short lists

**Top cuts (lines saved, src / tests; est. where marked):**

1. Emails, health marker and GitHub judged commit from the Worker (keep `ended` optional):
   about 530 / about 1,300 (est.). Drops `RESEND_API_KEY` (if `ended` goes) and `GITHUB_TOKEN`.
2. Receipt and ack signing across Worker, client, protocol, verifier: about 1,000 / about 2,500
   (est.). Largest change; do it after the deletion-only cuts.
3. Rust stand-in upload server: 0 / 2,514, plus about 3,000 of upload integration tests (est.).
4. Vitals, disk pause signal, `model_requests`, `recording`, `repo_head`, IP and country:
   about 300 / about 380 (est.).
5. Device rate limiter Durable Object (replace with a WAF rule): about 170 / 184.
6. Abandon and give-back, Worker and client: about 225 / about 420 (est.).
7. Old GitHub token flow and `git-credential`: about 280 / about 150 (est.).

**Top changes:**

1. `scripts/fetch-run.sh:218-219`: write what arrived and name the missing range, instead of
   nothing.
2. Server stores the tunnel list without parsing it (`runs.ts:73-86`).
3. Prune by the server's position, not a signed ack; drop the disk-pressure trigger.
4. One copy of the GitHub public-and-empty check (the Worker's).
5. Later, with a website migration: seal files as raw PUTs accepted once the end is committed;
   close window and credential expiry long enough to take a late tail up to the committed end.
6. Set `MIN_COMPETITION_VERSION` and deploy Worker and client together.
7. Fix the docs that disagree on the verify job (`docs/CHEAT-CHECKLIST.md:26-27` vs
   `docs/SERVER-DESIGN.md` "Not built").

**Must add for the seven points:**

1. Point 5: the rebuild must handle a partial upload (change 1 above). Nothing else is missing on
   the server: IDs, order, times and gaps are already in the chain.
2. Point 5: an owner decision on how long after the end the server keeps taking bytes up to the
   committed end (today 24 hours, a cap of our own).
3. A measured upload with many subagents (the 2-hour sandbox run, report not yet on any branch) to
   check that one sequential uploader keeps up.
