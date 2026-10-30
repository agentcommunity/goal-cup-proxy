# PRD-07: Worker, server and database

Id: PRD-07. Version: goalcup 1.0. Date: 30 October 2026. Base: main at 4b226de2.
Index: [PRD-00](PRD-00-index.md). Format: [PRD-01](PRD-01-record-format.md). Owns seam S5 (the upload
HTTP contract, `docs/UPLOAD-API.md`).

## Goal

The server stores every GCR1 chunk the client sends, in order, with its own clock, and never reads
inside it. It tells organizers which runs stopped recording, and it needs no website database change.

## Points served

- **5** (lead): chunks stored create-only by offset and hash; the entry sealed by its final heartbeat;
  `fetch-run` writes whatever arrived and names what is missing; no per-run size limit.
- **6**: the heartbeat carries written counts, off time and a real `failed`; organizers can find a run
  whose recording stopped.
- **3**: no server answer stops a run except `stop` (D64, kept); the maintenance switch stays.
- **1, 2**: nothing on the server counts or limits calls, streams, subagents or bytes.

## Owner decisions it carries out

- Verifier and Worker move to the new format in the same build and deploy together with the client.
  No backward compatibility (BRIEF3, 29 Oct evening).
- Signing is cut (BRIEF3, "All planned cuts are in"; PRD-00 table row PRD-07).
- Remove the per-run size limit, "as much as needed" (owner answer 2 and 3, BRIEF2).
- The heartbeat says `failed`; email work is out of scope, the server can mail later (BRIEF3).
- Counts: the participant, status, heartbeat and server see only written counts and off time.
- The website database is a hard stop for overnight agents (PRD-00 defaults): so this PRD changes no
  table, column or SQL function.
- The usage reader runs as an organizer job and stores JSON in R2 (PRD-00 default, seam S7).

Not in this PRD (the task names them out of scope): the device flow and its approval page, the
notices and mail in the cron, the GitHub judged commit. They stay exactly as they are. Signing and the
receipt are removed as a pure deletion, because PRD-00 binds it; nothing replaces them.

## Today

| What | Where | Fact |
|---|---|---|
| Worker size | `server/worker/src/*.ts` | 4,001 src lines (`wc -l`), 17 files; tests 5,255 lines in 15 files plus 494 helper lines (appendix C section 9) |
| Routes | `server/worker/src/index.ts:24-60` | device (3), open, position, receipt, abandon, seal, heartbeat, continue, chunks; health at `:65`; maintenance switch `:72-79`; cron `:105-107` |
| Chunk path | `server/worker/src/chunks.ts:34-115` | Header checks, retry by offset `:46-47`, end rules `:55-70`, size limit `:74`, stream to R2 `:202-228`, sign ack `:82`, one atomic commit `:96` |
| Per-run size limit | `chunks.ts:123-144`, `context.ts:12, 62-67, 126`, `wrangler.toml:56-59, 151` | 64 GiB compressed per run, 413 `quota_exceeded` |
| Signing | `src/sign.ts` (116), `context.ts:133-145, 168-170`, `chunks.ts:82`, `runs.ts:352-389` | Ed25519 receipt and per-chunk ack; secret `RECEIPT_SIGNING_KEY` (`wrangler.toml:81`) |
| Abandon | `runs.ts:393-414` | Gives back a run with no chunks |
| Seal | `runs.ts:540-637`, constants `:25-29`, helpers `:57-85` | Takes `seal.json`, `baseline.json`, `tunnels.jsonl` in one JSON body up to 40 MiB, parsed in a 128 MB isolate |
| Heartbeat | `runs.ts:444-491`, vitals validator `:499-533` | `recording` already accepts `failed` (`:455-457`); run 1 never sent it (run report "What the server shows") |
| Version floor | `runs.ts:154-164, 228`, `device.ts:54-55`, `wrangler.toml:55` | Competition only; empty today |
| Version alias | `util.ts:79-91` | Accepts `proxy_version` or `tracer_version` |
| Close time | `db.ts:17`, SQL `run_status_at` `sql/goal-cup-tracer-upload.sql:317-329` (24 hours at `:323`) | No upload 24 h after the end |
| Tables | `sql/goal-cup-tracer-upload.sql:92-117` (runs), `:132-162` (entries), `:164-178` (heartbeats), `:203-216` (chunks; `ack jsonb NOT NULL` at `:214`, `length > 0` at `:206`), `:2549-2560` (stored bytes, vitals) | The website repo owns them; this file is a mirror (`:1-24`). Website migrations end at 20261027100000, same as the mirror |
| Heartbeat SQL | `sql:2811-2842` (`note_heartbeat`), `:2871-2922` (14-argument `goal_cup_upload_heartbeat` with `p_vitals jsonb`) | Stores `vitals` on the row and as `runs.last_vitals`; `flag_reason` is first-wins (`:2831-2832`) |
| Seal SQL | `sql:1409-1448` | Needs final committed and the chain held to it; stores any two strings as the two digests; keys may be null |
| Rebuild | `scripts/fetch-run.sh` (349) | Decodes each chunk alone (`:327`); refuses to write an entry whose chunks stop short (`:214-219`); writes acks, receipt, seal docs |
| Run 1 on the server | run report, "What the server shows" | `recording` never `failed`; `model_requests` 1,766 for 842 recorded; nine heartbeats in a row with the same `seq`; no screen shows the loss |

## Design

### 1. The server stays a byte store

The Worker never decompresses, never parses a chunk, and never holds a chunk body in memory. That is
already true (`chunks.ts:202-228` pipes the body into R2 through a `FixedLengthStream`) and it is
what makes GCR1 work on the server unchanged:

- A GCR1 chunk is a byte range of a compressed part file (PRD-01 section 12). It continues its part's
  zstd frame, so it is **not** decodable alone. The Worker does not care: it checks headers, streams,
  lets R2 check SHA-256, and commits.
- The 128 MiB decompression window (PRD-01 sections 12 and 15) is therefore never met inside a
  Worker isolate, which has 128 MB ([Workers limits](https://developers.cloudflare.com/workers/platform/limits/)).
  Rule for every later change: **no server-side reading runs in a Worker.** Decoding happens in the
  verifier and the usage reader on an organizer machine (PRD-08, PRD-09).
- The chunk headers keep their names and checks. Their meaning moves to GCR1, as PRD-01 section 12
  says: path offset and `Goalcup-Length` are raw GCR1 stream offsets and lengths; `Goalcup-First-Seq`,
  `Goalcup-Last-Seq`, `Goalcup-Prev`, `Goalcup-Head` are LINK sequence numbers and link hashes. The
  genesis stays 64 zero hex digits (`db.ts:67`, PRD-01 `hash(-1)`), the first seq stays 0 (`db.ts:65`).
- R2 key unchanged: `traces/{event}/{run}/{entry}/{offset:016}-{sha256}.zst` (`chunks.ts:156-158`).
  `Content-Type: application/zstd` stays. The 32 MiB chunk cap stays (`chunks.ts:22`): it guards one
  request's size and retry cost, and PRD-01 keeps a flush interval at 16 MiB raw so a chunk always fits.
- With the seal body gone (section 3), the largest JSON body the Worker parses is 64 KiB
  (`runs.ts:25`). The untested 40 MiB seal body in a 128 MB isolate (appendix I section 4) is gone.

### 2. Routes kept and cut

| Route | 1.0 | Change |
|---|---|---|
| `GET /v1/health` | keep | none |
| `POST /v1/device`, `POST /v1/device/token` | keep | none (out of scope). Version floor in all modes (section 6) |
| `POST /v1/device/approve` | keep | none; staging and local only, the pad needs it |
| `POST /v1/runs` (open) | keep | body: `connector_sha256` replaces `manifest_sha256` and `connector_dir_sha256`; `tracer_version` alias gone; version floor in all modes |
| `POST /v1/runs/{run}/continue` | keep | same body change |
| `GET /v1/runs/{run}[?entry_id=]` | keep | answer drops `last_ack`, adds `sealed` (bool) |
| `PUT /v1/runs/{run}/chunks/{offset}` | keep | no signing; no size limit; answer `{offset, next_seq, head, received_at}` |
| `POST /v1/runs/{run}/heartbeat` | keep | new body (section 4); a final heartbeat seals the entry (section 3) |
| `POST /v1/runs/{run}/receipt` | **cut** | signing cut |
| `POST /v1/runs/{run}/abandon` | **cut** | serves no point (appendix I section 4); a failed competition start is replaced by the next start, a practice or test run is harmless test data |
| `POST /v1/runs/{run}/seal` | **cut** | the seal, the start facts and the tunnels are records now (PRD-01 section 11) |
| cron every 5 minutes | keep as is | out of scope; see "Risks" for what it now reads |

A cut path answers `404 not_found` from the existing router (`index.ts:67`). A 0.7 client that calls it
fails loudly before it launches a harness.

### 3. The seal happens on the final heartbeat

There is no seal call. The entry is sealed by the server when two facts hold: the final heartbeat has
committed the seal LINK (`seq`, `head`), and the server holds the chain up to that link. The heartbeat
handler checks this after `recordHeartbeat` answers `final_committed` or `final_existing`:

```ts
// after a final heartbeat was recorded or repeated
if (entry.sealed_at === null && entry.next_seq - 1 === seq && entry.head === head) {
  await ctx.db.sealRun({ member_id, run_id, entry_id, seal_sha256: head, baseline_sha256: head,
                         seal_key: null, baseline_key: null, sealed_at: now });
}
// answer: { stop, deadline_at, seconds_left, reason, sealed }
```

- It calls the existing SQL `goal_cup_upload_seal_run` (`sql:1409-1448`) unchanged. That function
  rechecks final and chain in its own locked step, so two racing calls are safe. The two digest
  columns get the seal link's hash; the two key columns get null (`delete_test_runs` already skips
  null keys, `sql:2142-2143`). **No migration.**
- **Client rule (PRD-06):** send the final heartbeat; upload the rest; send the final heartbeat again
  until its answer says `sealed: true`. A repeat is `final_existing` and costs nothing. This also
  covers the order the server already enforces after `ends_at`: chunk `503 awaiting_final` until the
  final heartbeat (`chunks.ts:55-59`).
- `GET` position also answers `sealed`, so `goalcup upload` can tell a finished entry.
- Run status rules are unchanged: `sealed` once the run is over and its current entry is sealed,
  `incomplete` 24 hours later without it (`db.ts:48-52`).

### 4. Heartbeat: written counts, off time and four states

Body (S5, sent by PRD-06):

```json
{"entry_id":"e0123456789abcdef","seq":3601,"head":"<64 hex>","offset_local":726000000,"final":false,
 "repo_head":null,"recording":"gaps","model_requests":905,
 "counts":{"v":1,"boot":2,"mode":"recording","off_ms":2300,"off_spans":1,"off_since":null,
           "gaps":3,"lost_bytes":1500000,"lost_unknown":0,"http_post":243,"ws_up_msgs":662,
           "conns":120,"tunnels":118,"wire_bytes":674000000,"native_files":43,"native_bytes":48200000}}
```

- `seq`, `head`, `offset_local` are the **durable point** (PRD-02 seam S4): the last LINK on disk and the
  raw offset just past it. The final heartbeat names the seal LINK.
- `model_requests` is PRD-01 section 9's "model requests written" (`http_post + ws_up_msgs`), summed
  over boots, **as of the durable point**, so the server's number never exceeds what a server copy
  can show. It goes to `entries.model_requests` (kept as the highest per entry, `sql:2817-2821`).
- `counts` is a flat JSON object of whole numbers, short strings, booleans and nulls, at most 2 KiB
  serialized. The Worker checks only that shape (it reuses the `machine` check, `runs.ts:192-202`,
  with a 2 KiB bound), drops a key named `agent_paused`, and passes it as `p_vitals` to the existing
  14-argument SQL function. It lands in `heartbeats.vitals` and `runs.last_vitals`. The server
  interprets nothing; the organizer query reads `off_ms`, `mode`, `native_bytes`. Column names stay
  `vitals` because renaming is a migration; `docs/UPLOAD-API.md` says so in one line.
- `counts` is required in 1.0, so every heartbeat uses the 14-argument form and none calls the old
  13-argument overload. The old `vitals` validator (`runs.ts:499-533`) and the `Vitals` type use in the
  handler go.
- `recording`, required in 1.0, maps from PRD-02's `status.json`:

| `recording` | Means | Client sends it when (PRD-02 section 6) |
|---|---|---|
| `unknown` | nothing written yet | before the first COUNT of the entry |
| `ok` | recording, nothing lost | `mode` `recording`, `gaps` 0, `lost_bytes` 0, `off_ms` 0 |
| `gaps` | recording now, something was lost | `mode` `recording`, `losing` or `writer_slow`, and any of `gaps`, `lost_bytes`, `off_ms`, `streams_in_gap` above 0 |
| `failed` | **recording is off now** | `mode` `null` (the watchdog's null mode), or the recorder is not running while the heartbeat is sent |

`failed` is a state, not a latch: the next heartbeat after a good boot says `gaps`. The server keeps
every heartbeat row, so the off spans are visible in time order. `runs.flag_reason` stays first-wins
(`sql:2831-2832`): a run flagged `gaps` first never shows `failed` there, which is why the organizer
query reads the heartbeat rows, not the flag.

The answer gains `sealed`. `stop`, `reason`, `deadline_at`, `seconds_left` are unchanged (D64, D66).

### 5. No per-run size limit, and what storage costs

Delete the 413 `quota_exceeded` check (`chunks.ts:74, 123-135`), its config (`context.ts:12, 26-27,
62-67, 126`) and its two `wrangler.toml` lines. Keep the stored-bytes counter (the 4-argument
`goal_cup_upload_commit_chunk`, `sql:2758-2804`, already adds each chunk's compressed size) and keep
one log line at every 10 GiB a run crosses (replacing `noteQuota`, `chunks.ts:138-144`), so the owner
sees a large run in Workers Logs.

Storage cost, stated (R2 at $0.015 per GB-month, Class A $4.50 per million, egress free:
[R2 pricing](https://developers.cloudflare.com/r2/pricing/)):

| Case | Compressed size (estimate) | Cost |
|---|---|---|
| Run 1 rate, 2 hours, whole | 75 to 150 MB (PRD-01 section 14) | under $0.01 a month |
| A busy week at run 1's rate | 10 to 20 GB (PRD-01 section 14) | $0.15 to $0.30 a month; the competition prefix is locked 180 days (`wrangler.toml:31`), so about $0.90 to $1.80 per run in all |
| 1,500 busy weeks | 15 to 30 TB | $225 to $450 a month, about $1,350 to $2,700 over the lock |
| A stuck or hostile client at 100 Mbit/s for 8 days | about 8.6 TB | about $130 a month, about $780 over the lock, for that one run |
| Class A writes, one chunk per minute while data flows | up to 10,080 per entry-week; 1,500 runs about 15 million | about $63 a month above the free million |

The limit guarded only the last two rows. Without it, the guard is a person: the 10 GiB log lines and
the organizer query in section 8 (it lists stored bytes per run). The manual lever, should a run
misbehave, is to delete that run's row in `goal_cup_tracer.run_credentials` (the client then gets
`401`, keeps its data and stops uploading); that is a hand write to the shared database by the owner,
not code. Bytes already under the lock cannot be deleted for 180 days.

### 6. Open, continue and the version floor

- `connector_sha256` (64 hex, the SHA-256 of the connector file the client ran, PRD-11) replaces the two
  digests. The Worker writes it into both NOT NULL columns `manifest_sha256` and
  `connector_dir_sha256` (`sql:137-138`). No migration.
- The `tracer_version` alias goes (`util.ts:79-91`): only `proxy_version` is read. The column name stays.
- **`MIN_COMPETITION_VERSION` applies to every mode**, not only competition (`runs.ts:228`,
  `device.ts:55`): a 0.7 client in practice or test mode would otherwise open a run and then fail at the
  cut receipt call with a bare 404. It is set to `1.0.0` in `wrangler.toml` in the same commit that
  ships 1.0 (`wrangler.toml:51-55` warns that a dashboard edit is wiped by the next deploy). `continue`
  stays unchecked (D70).
- Everything else in open and continue stays: server clock (D16), one main run, the start window,
  GitHub public-and-empty check, `machine`, the IP and country columns (owner may cut them later).

### 7. `fetch-run`: the server copy, whole or partial

`scripts/fetch-run.sh` is rewritten to fetch bytes and decode nothing:

```
sh scripts/fetch-run.sh [--bucket B] [--index FILE] [--store DIR] [--local PERSIST_DIR] <run|entry> <outdir>
<outdir>/<entry>/rec.gcr.zst   every chunk of the entry, SHA-256 checked, concatenated in offset order
<outdir>/<entry>/server.json   the entry's server facts (below)
```

- **Index:** one `BEGIN READ ONLY` query as today (`fetch-run.sh:121-137`), now selecting `upload_offset`,
  `next_seq`, `head`, `final_seq`, `final_head`, `sealed_at`, `stored_bytes`, `chunk_count`, and per chunk
  `offset`, `length`, `sha256`, `first_seq`, `last_seq`, `prev`, `head`, `r2_key`, `received_at`. The
  `ack`, `receipt`, `seal_key` and `baseline_key` columns are no longer read.
- **Checks, per entry:** offsets run on from 0; `first_seq` runs on from 0; each chunk's `prev` is the
  previous chunk's `head` (genesis for the first; today's script never compares them); each object
  matches its SHA-256; the last chunk ends at `upload_offset`.
- **Partial is normal.** An entry whose chunks stop before the committed end, or that has no final
  heartbeat, is written as far as it goes, and `server.json` says what is missing. Today the script
  writes nothing then (`fetch-run.sh:214-219`). A hole in the middle cannot happen (the Worker accepts
  only the next offset, `chunks.ts:68-70`); if the index shows one, the script stops with no output for
  that entry, as today.
- `server.json`:

```json
{"v":1,"run_id":"r…","entry_id":"e…","segment":0,"upload_offset":726000000,"next_seq":3602,
 "head":"<hex>","final":{"seq":3601,"head":"<hex>"},"sealed_at":"2026-11-14T00:00:09Z",
 "complete":true,"missing":null,"stored_bytes":96000000,"chunk_count":121,
 "chunks":[{"offset":0,"length":16777216,"sha256":"<hex>","first_seq":0,"last_seq":18,
            "prev":"<hex>","head":"<hex>","received_at":"…"}]}
```

  `complete` is true when `final` is set and `next_seq - 1 = final.seq` with the same head. Otherwise
  `missing` is `{"from_offset": <upload_offset>, "from_seq": <next_seq>, "to_seq": <final.seq or null>}`.
- **Decoding is the reader's job.** `goalcup-verify check --server <dir>` (PRD-08) and the usage reader
  (PRD-09) decode `rec.gcr.zst` with window 2^27, which is also the default limit of `zstd -d`
  (appendix M section 4.4). A person can run `zstd -dc --long=27 rec.gcr.zst`; an entry that is not
  sealed ends inside an open frame, so `zstd` reports a truncated input after writing every whole
  block (expected, PRD-01 section 12). fetch-run no longer needs `zstd`.
- **Byte equality.** `rec.gcr.zst` equals the concatenated local part files, byte for byte, up to the
  server's offset (PRD-01 acceptance 2). That is the one comparison a watcher runs.
- **`--local PERSIST_DIR`** reads a local Worker (`wrangler dev` with `DB_BACKEND=local`) for the pad:
  the index from `state/runs/<run>.json` (`db-local.ts:5-12`) and each object with
  `npx wrangler r2 object get <bucket>/<key> --local --persist-to <dir> --file <dest>`. Whether those
  two flags work on `r2 object get` in wrangler 4.149.0 is **unverified**; the builder checks it first.
  Fallback if they do not: the pad runs the local Worker with `--persist-to` and a test-only Worker
  route is **not** added; instead the pad keeps the client's local parts until the check (prune off)
  and compares against an R2 mirror made by `wrangler r2 object get --local` per key.

### 8. Organizers: finding runs whose recording stopped

New file `server/worker/scripts/stopped-runs.sql`, run read-only:

```
psql "$DATABASE_URL" -X -v ON_ERROR_STOP=1 -v event=goalcup-cup-2026 -f server/worker/scripts/stopped-runs.sql
```

```sql
BEGIN READ ONLY;
WITH cur AS (            -- the current entry of each open production run in the event
  SELECT DISTINCT ON (e.run_id) e.run_id, e.id AS entry_id, e.final_seq
    FROM goal_cup_tracer.entries e
    JOIN goal_cup_tracer.runs r ON r.id = e.run_id
   WHERE r.event_id = :'event' AND r.status = 'open' AND NOT r.staging
     AND now() < least(r.deadline_at, coalesce(r.replaced_at, r.deadline_at))
   ORDER BY e.run_id, e.segment DESC),
last AS (                -- its latest heartbeat
  SELECT DISTINCT ON (h.entry_id) h.entry_id, h.received_at, h.seq, h.recording, h.model_requests, h.vitals
    FROM goal_cup_tracer.heartbeats h JOIN cur USING (entry_id)
   ORDER BY h.entry_id, h.received_at DESC),
ago AS (                 -- its heartbeat from at least 30 minutes before that
  SELECT DISTINCT ON (h.entry_id) h.entry_id, h.seq, h.model_requests, h.vitals
    FROM goal_cup_tracer.heartbeats h JOIN last l USING (entry_id)
   WHERE h.received_at <= l.received_at - interval '30 minutes'
   ORDER BY h.entry_id, h.received_at DESC)
SELECT c.run_id, c.entry_id, l.received_at AS last_heartbeat, l.recording,
       (l.vitals ->> 'off_ms')::bigint AS off_ms, (l.vitals ->> 'lost_bytes')::bigint AS lost_bytes,
       CASE
         WHEN l.entry_id IS NULL                          THEN 'never_heard'
         WHEN l.recording = 'failed'                      THEN 'off_now'
         WHEN l.received_at < now() - interval '20 minutes' THEN 'silent'
         WHEN a.entry_id IS NOT NULL AND l.seq = a.seq    THEN 'writer_stuck'
         WHEN a.entry_id IS NOT NULL AND l.model_requests = a.model_requests
              AND (l.vitals ->> 'native_bytes')::bigint > (a.vitals ->> 'native_bytes')::bigint + 1048576
                                                          THEN 'wire_quiet'
       END AS why
  FROM cur c LEFT JOIN last l USING (entry_id) LEFT JOIN ago a USING (entry_id)
 WHERE c.final_seq IS NULL
 ORDER BY why NULLS LAST, l.received_at;
ROLLBACK;
```

A second short query in the same file lists the 20 runs with the most stored bytes (`sum(entries.stored_bytes)`
per run), for section 5.

| `why` | Meaning | Why it is a stop |
|---|---|---|
| `off_now` | the client says recording is off (null mode) | PRD-02 section 8 step 7 |
| `silent` | no heartbeat for 20 minutes (four missed at a 5-minute cadence; same window as the cron's `SILENCE_AFTER`, `notices.ts:20`) | the machine sleeps, is offline, or goalcup died |
| `writer_stuck` | heartbeats arrive, the durable link has not moved for 30 minutes | a live recorder writes a LINK at least once a minute (PRD-01 section 6, the COUNT forces one); run 1's tell was nine heartbeats with one `seq` |
| `wire_quiet` | session files grew by over 1 MiB in 30 minutes while no model request was written | the agent works but its calls do not pass the proxy: a dodger, a lost `HTTPS_PROXY`, or a proxy fault. A hint, not proof: a long tool run also writes session files |
| `never_heard` | an open current entry with no heartbeat at all | start failed after open, or the client never reached the server |

A healthy run has `why` null and is listed last. `staging` rows are excluded (staging and production
share one database, `sql:1092`).

### 9. Read-only access for organizers

| Need | 1.0 (no database change) | Later, owner's choice |
|---|---|---|
| Database | Every organizer script opens `BEGIN READ ONLY` and ends `ROLLBACK` (fetch-run, stopped-runs). That protects against mistakes, not misuse: the connection is the owner's. Overnight agents get **no** production database URL: the pad runs on a local Worker (PRD-00 default "Pad server") | A login role with `SELECT` on `goal_cup_tracer.runs, entries, chunks, heartbeats, events`, with column grants that leave out `client_ip`, `client_country`, `last_client_ip`, `last_client_country`, `machine`. That is a website migration through the pending flow (`server/worker/README.md:64`), applied by the owner. The SQL text goes in the PR description, not in `sql/` |
| R2, traces | A Cloudflare R2 token with **Object Read only**, scoped to `goalcup-traces` (and `goalcup-traces-staging`). R2 tokens can be scoped to buckets but not prefixes ([R2 tokens](https://developers.cloudflare.com/r2/api/tokens/)). fetch-run already takes it from the environment only and never prints it (`fetch-run.sh:34-40`) | none |
| R2, usage | Read: an Object Read token on the usage bucket. Write: only the usage job (PRD-09) | none |

### 10. Where the usage stats live

- **An R2 object, not a table.** Seam S7 (PRD-09): `usage/{event}/{run}/{entry}.json`, written by the
  organizer job. No database migration.
- **A separate bucket**, `goalcup-usage` (staging `goalcup-usage-staging`), not the traces bucket.
  Reason: the job needs write access, R2 tokens scope only to whole buckets, and a write token on
  `goalcup-traces` could delete unlocked trace chunks (test, dry run, practice). Usage JSON is derived
  and can be rebuilt, so it sits outside the bucket lock and may be overwritten when the reader
  improves. The Worker does not bind or serve it.
- `server/worker/scripts/delete-test-runs.sh` also empties `usage/test/` and `usage/test-staging/` in
  the usage buckets, so a test-run cleanup leaves no orphan stats.

### 11. Website database: nothing to migrate in 1.0

| 1.0 needs | How it fits the tables as they are | A later migration could |
|---|---|---|
| No signed ack | write `{}` into `chunks.ack` (NOT NULL, `sql:214`) and `entries.last_ack` | drop the column |
| No seal documents | seal link hash into both digest columns, null keys (section 3) | rename or drop |
| Counts and off time | `heartbeats.vitals`, `runs.last_vitals` (jsonb, `sql:2552-2553`) | rename to `counts` |
| `failed` after `gaps` | read heartbeat rows; `flag_reason` stays first-wins | make the flag the latest state |
| One connector digest | write it into both digest columns | merge them |
| Usage stats | R2 JSON (section 10) | add tables if organizers want SQL over them |
| Version floor in all modes | Worker only | none |
| Receipt and abandon | the SQL functions stay, unused | drop them and the 13-argument overloads |
| 24-hour close and credential life | unchanged (`sql:323`, `:2603`) | lengthen both together for point 5 (owner question below) |

Only the owner applies a migration to the shared production database (website CLAUDE.md); none is needed.

### 12. Deploy order with the client

1. **Overnight:** nothing is deployed. The pad runs the 1.0 Worker locally (`wrangler dev`,
   `DB_BACKEND=local`) against the 1.0 client from the same commit (PRD-00 default, question 2).
2. **Morning, before deploy:** the owner checks that no real run is open
   (`SELECT count(*) FROM goal_cup_tracer.runs WHERE status='open' AND NOT staging AND mode <> 'test'`
   inside `BEGIN READ ONLY`). Every run so far is test data, but a 0.7 client mid-run would stop
   uploading after the deploy (its heartbeat lacks `counts`, its seal route is gone); it keeps its data.
3. **Staging:** `wrangler deploy --env staging` with `MIN_COMPETITION_VERSION = "1.0.0"`. Core-set pad
   runs (Claude Code, Codex, Hermes, direct key) against staging; `fetch-run` and `stopped-runs.sql`
   against the staging rows (filter `staging` in place of `NOT staging`).
4. **Release:** tag `v1.0.0` (PRD-14), so `releases/latest` serves 1.0.
5. **Production:** `wrangler deploy` from the same commit, then delete the `RECEIPT_SIGNING_KEY` secret
   (only after the deploy: the 0.7 Worker refuses every call without it, `context.ts:136`).
6. **Smoke:** one practice run with the 1.0 client on production, sealed, fetched, checked.
7. The `EVENT_ID` switch stays the owner's manual deploy between 6 Nov 00:00 and 7 Nov 00:00 UTC
   (`wrangler.toml:37-42`); it can be the same deploy as step 5 if the timing fits.

**No rollback to the 0.7 Worker** once 1.0 clients run: it would refuse their heartbeats and never seal
them. A bad 1.0 Worker is held with `MAINTENANCE=on` (clients back off and keep everything,
`index.ts:72-79`) and fixed forward.

## Files it owns

Change:
- `server/worker/src/index.ts` (routes), `chunks.ts` (no signing, no quota, 10 GiB log line),
  `runs.ts` (heartbeat, seal on final, open and continue fields, version floor, cut receipt, abandon,
  seal), `context.ts` (no signer, no byte limit), `db.ts`, `db-local.ts`, `db-supabase.ts` (remove
  `storeReceipt`, `abandonRun`; `SealArgs` keys nullable; ack type `{}`), `device.ts` (version floor
  in all modes), `util.ts` (alias cut), `env.ts` (drop `RECEIPT_SIGNING_KEY`, `RUN_MAX_COMPRESSED_BYTES`).
- `server/worker/test/*.test.ts`, `test/helpers/*.ts` (section "What is cut"; new cases below).
- `server/worker/wrangler.toml` (vars, secret list, comments), `server/worker/README.md`.
- `server/worker/scripts/delete-test-runs.sh` (usage buckets), `server/worker/scripts/check-sql.sh` (a
  last step that seeds heartbeat rows and runs `stopped-runs.sql`).
- `scripts/fetch-run.sh`, `scripts/test_fetch_run.py`, `docs/UPLOAD-API.md` (seam S5).

Create: `server/worker/scripts/stopped-runs.sql`.

Delete: `server/worker/src/sign.ts`, `test/sign.test.ts`, `test/vectors/signing.json`, `test/quota.test.ts`.

Must not touch: `server/worker/sql/**` (the website's mirror; regenerated only from the website repo),
`server/worker/src/notices.ts`, `mail.ts`, `github.ts`, `ratelimit.ts` (out of scope), anything under
`crates/` (PRD-01 to PRD-11), `docs/*.md` other than `docs/UPLOAD-API.md` (PRD-15).

## Interfaces with other PRDs

| PRD | What | Direction |
|---|---|---|
| PRD-01 | Chunk headers, key, raw offsets, link seq and hash (section 12); seal LINK (section 11) | used here, unchanged |
| PRD-02 | S4 durable point and `status.json` (`mode`, `written`, `off`); null mode | the heartbeat's source |
| PRD-06 | **S5**: open and continue bodies, chunk headers and answers, heartbeat body and states, the seal rule "repeat the final heartbeat until `sealed`", error table, backoff (D64 unchanged). Asks: heartbeat every 5 minutes; counts as of the durable point; a chunk at most every 5 minutes or 32 MiB (keeps `chunks` rows near 2,000 per busy entry-week, against about 10,000 at one a minute, PRD-00 question 7) | owned here, published by H+2 |
| PRD-08 | `<entry>/rec.gcr.zst` and `server.json` from fetch-run; `check --server` decodes and compares | owned here (layout) |
| PRD-09 | S7 key; the usage bucket name; reads fetch-run output | bucket proposed here |
| PRD-10 | `sealed` in heartbeat and position answers; the plain words for `failed` | used there |
| PRD-11 | `connector_sha256` value | used here |
| PRD-12 | `fetch-run --local` for pad server checks; `stopped-runs.sql` on staging in the morning | used there |
| PRD-13 | Deletes the Rust stand-in server `crates/goalcup-cli/tests/common/upload_server/` (2,514 test lines) with the old crate | one server, not two |
| PRD-14 | Gate runs `npm test` in `server/worker`, `scripts/test_fetch_run.py`, and `check-sql.sh` where PostgreSQL is present; PRD-06's tests run against `wrangler dev`, not a second fake Worker | used there |
| PRD-15 | Fix stale docs that name signing, receipts, acks, the seal call, the verify job (`docs/OPERATING.md`, `docs/SERVER-DESIGN.md`, `docs/CHEAT-CHECKLIST.md`) | done there |
| PRD-16 | Deploy order (section 12); owner deploys | used there |

## What is cut

| Path | Src lines | Tests that go |
|---|---|---|
| `src/sign.ts` and signer plumbing (`context.ts:133-145, 168-170`, `chunks.ts:82`) | about 135 | `test/sign.test.ts` (68), `test/vectors/signing.json` (26), ack checks in `test/helpers/client.ts` |
| Receipt route and DB methods (`runs.ts:352-389`, `db-*.ts`) | about 75 | receipt cases in `integration.test.ts`, `lifecycle.test.ts`, `supabase.test.ts`, `device.test.ts` (about 150, est.) |
| Abandon route and DB methods (`runs.ts:393-414`, `db-*.ts`) | about 60 | abandon cases in `integration`, `local-db`, `supabase`, `config` (about 120, est.) |
| Seal route, documents, tunnel list (`runs.ts:25-29, 57-85, 535-637`) | about 140, replaced by about 15 | seal-document and tunnel cases in `integration.test.ts` (14 tunnel references), `lifecycle`, `segments` (about 250, est.) |
| Per-run size limit (`chunks.ts:74, 123-144`, `context.ts:11-12, 26-27, 61-67, 126`) | about 50 | `test/quota.test.ts` (114), `config.test.ts:237` |
| Vitals validator (`runs.ts:499-533`) | 35, replaced by about 10 | vitals cases in `notices.test.ts` (31 references, about 70 lines) |
| `tracer_version` alias (`util.ts:79-91`) | about 10 | `lifecycle.test.ts:481-505`, `device-unit.test.ts` alias cases |
| fetch-run acks, receipt, seal docs, per-chunk decode (`fetch-run.sh:12-14, 174-184, 222-226, 282-318, 326-332`) | about 80 removed, about 40 added | matching cases in `scripts/test_fetch_run.py` |

Net: Worker about 4,000 to about 3,450 src lines (est.). PRD-00's 3,000 target assumed notices, mail
and the GitHub commit moved out; they are out of scope here (about 530 more lines, appendix I section 12).
Tests about 5,750 to about 4,950 lines (est.). Secrets the owner holds drop from six to five
(`RECEIPT_SIGNING_KEY` goes).

## Acceptance

Real-run proofs first (pad, PRD-12, Linux sandbox, local Worker unless said):

1. **Whole server copy, sealed.** Claude Code, direct key, one subagent. Pass: the final heartbeat
   answers `sealed: true`; `fetch-run --local` writes `rec.gcr.zst` whose SHA-256 equals that of the
   concatenated local parts; `server.json` has `complete: true`; `goalcup-verify check --server` passes.
2. **Kill test on the server side.** Codex pad run, recorder killed twice (PRD-01 acceptance 3). Pass:
   the server copy equals local; heartbeat rows show `ok` before the first kill and `gaps` after; no
   chunk was refused except `409` re-reads; the entry seals.
3. **`failed` reaches the server.** A Codex pad run with PRD-02's test hook that forces null mode for
   5 minutes. Pass: at least one heartbeat row with `recording = failed` and `counts.mode = "null"`,
   then `gaps` after the good boot; `counts.off_ms` covers the null span within 10 s.
4. **Partial upload.** Block the Worker port for the last 10 minutes of a Codex run, let the run seal
   locally, stop the client. Pass: fetch-run writes `rec.gcr.zst` with `complete: false` and `missing`
   naming `from_seq`; `goalcup-verify check --server` reports an open end, not an error; after
   `goalcup upload`, a second fetch is `complete: true` and equals local.
5. **No size limit.** A Codex run with 20 subagents for 30 minutes and the local `stored_bytes` set past
   64 GiB before it starts. Pass: no `413`; report compressed bytes per hour for the section 5 table.
6. **Morning, staging** (after the owner deploys): three short runs: one healthy, one forced into null
   mode, one whose sandbox is stopped mid-run. Pass: `stopped-runs.sql` (with `staging`) lists exactly
   `off_now` and `silent` for the last two, and nothing for the healthy one.

Automated tests that guard them:

- `integration.test.ts`: chunks then a final heartbeat seal the entry; a final heartbeat first, then
  chunks, then a repeat seals it; position says `sealed`; `counts` lands on the heartbeat and the run;
  a `counts` over 2 KiB or not flat is `400`; receipt, abandon and seal paths are `404`.
- `config.test.ts`: `MIN_COMPETITION_VERSION` refuses a 0.7 practice and test open and device request;
  `continue` is not refused.
- `test_fetch_run.py`: a partial entry is written with `missing`; a chunk whose `prev` is not the
  previous `head` stops that entry; output equals the concatenated chunk bodies.
- `check-sql.sh` last step: seeded heartbeat rows give each `why` value once.

## Risks and blind spots

| Risk | What could go wrong | What we do |
|---|---|---|
| No size limit | A stuck client fills R2 with locked bytes for 180 days | 10 GiB log lines, the top-runs query, the credential lever (section 5) |
| One lost chunk | A chunk is not decodable alone, so a missing middle chunk loses the rest of its part (PRD-01 risks) | The Worker accepts only the next offset; fetch-run checks prev and head continuity |
| `vitals` named wrongly | A reader of the website database thinks `vitals` holds disk facts | One line in `docs/UPLOAD-API.md`; rename in a later migration |
| `flag_reason` first-wins | The cron's `recording` email and the run flag say `gaps` even when recording later failed | The query reads heartbeat rows; email work is out of scope |
| Cron reads the new numbers | `idle_3h` and `idle_6h` (`notices.ts:163-171`) now see written requests, so a run whose proxy dies gets an idle email it did not get before | That is the honest signal; out of scope to change |
| `wire_quiet` false alarms | A long tool run writes session files with no model call | Listed as a hint; an organizer reads the run before acting |
| 24-hour close | A laptop off for a day after the deadline loses its tail (point 5), `sql:323` | Owner question; needs a migration of close and credential together |
| Old client mid-run at deploy | Its heartbeats are `400` and it never seals | Deploy when no real run is open (section 12 step 2) |
| `wrangler r2 object get --local` | The flags may not exist on that command | Builder checks first; fallback in section 7 |
| Organizer access | `BEGIN READ ONLY` on the owner's connection is not access control | The reader role is a later owner choice |
| Zone body limit | The zone's plan is unverified (appendix I section 7); 100 MB on Free and Pro | 32 MiB chunks fit any plan |
| R2 bucket lock and create-only | Create-only under the lock is unproven (`server/worker/README.md:405-408`) | Owner step on the first competition chunk, unchanged |

## Overridable defaults

| Default | Value | Who may change it |
|---|---|---|
| Version floor | `1.0.0`, every mode | owner |
| Usage storage | separate bucket `goalcup-usage` | owner, PRD-09 |
| Heartbeat cadence the query assumes | 5 minutes; `silent` after 20, `writer_stuck` and `wire_quiet` over 30 | PRD-06 with this PRD |
| Large-run log line | every 10 GiB per run | owner |
| `counts` bound | 2 KiB, flat | this PRD |
| IP and country columns | kept as today | owner (appendix I suggests cutting) |
| Close and credential life | 24 hours after the end | owner, by website migration |
| Reader role | not created; SQL proposed | owner |

## Builder notes

Order of work:

1. H+0 to H+2: rewrite `docs/UPLOAD-API.md` sections open, continue, position, chunks, heartbeat,
   "When a run ends", error table; delete receipt, abandon, seal sections. Publish S5 in the night log.
   PRD-06 builds against it.
2. Deletions: `sign.ts`, signer in `context.ts`, receipt, abandon, seal, quota, vitals validator, alias.
   `npm test` green after removing the matching cases.
3. Heartbeat `counts`, `recording` required, seal on final, `sealed` in answers; `connector_sha256`;
   version floor in all modes. New integration cases.
4. `fetch-run.sh` rewrite and `test_fetch_run.py`; check `--local` against a `wrangler dev` run first.
5. `stopped-runs.sql`, the `check-sql.sh` step, `delete-test-runs.sh`, `wrangler.toml`, `README.md`.

Traps from the audit:

- `chunks.ack` is NOT NULL (`sql:214`): write `{}`, never null.
- Call only the 14-argument heartbeat: an object in `p_vitals` picks it (`db-supabase.ts:357-359`).
  The parity test in `supabase.test.ts` checks one overload per call.
- `db-local.ts` must keep matching the SQL (appendix C coupling 1): seal with null keys, `vitals`
  stored as given. Its disk-pause twin stays and stays dormant.
- `length > 0` (`sql:206`): the client never sends an empty chunk (PRD-01 section 1 guarantees it).
- Staging and production share one database: every organizer query filters `staging`.
- Do not touch `ratelimit.ts`: removing a Durable Object class needs a `deleted_classes` migration
  (appendix I section 4).
- Delete the signing secret only after the new Worker is live (section 12 step 5).
- A credential lives until deadline plus 24 hours (`db.ts:57`), so after that a late upload is `401`,
  not `410`.

How a watcher proves it in a real run: after each pad run, run
`sh scripts/fetch-run.sh --local <persist dir> <run> <out>`, then compare
`sha256sum <out>/<entry>/rec.gcr.zst` with `cat rec-*.gcr.zst | sha256sum` from the local entry
(before pruning), read `complete` in `server.json`, and run `goalcup-verify check --server <out>/<entry>`.
For the heartbeat, read the local state object `state/runs/<run>.json` and check the `recording` and
`vitals` of every heartbeat against `status.json` snapshots taken at the same minutes.

## Asks of PRD-00 and PRD-01

1. **PRD-00, scope.** The task for this PRD says device approval, signing, receipts, notices and mail
   are out of scope; PRD-00 says PRD-07 cuts signing. This PRD follows PRD-00: signing and receipts are
   deleted, nothing else is done to them; device flow, notices and mail are untouched. Please confirm,
   or move the signing deletion elsewhere.
2. **PRD-00, overnight server access.** The default "read-only database and R2 access" conflicts with
   "pad server: local Worker" and with the website database being a hard stop. Please change it to:
   no production credentials overnight; read-only access in the morning after the owner's answer to
   question 1. A real read-only role is a website migration.
3. **PRD-00, question 1 text.** Add what 1.0 maps onto existing columns: seal link hash into the two
   seal digest columns with null keys; `counts` into `heartbeats.vitals`; `connector_sha256` into both
   digest columns. Add the 24-hour close as a separate later question (point 5).
4. **PRD-00, seam S7.** Name the bucket: `goalcup-usage`, separate from traces (section 10).
5. **PRD-00, question 7.** Chunk cadence drives `chunks` rows in the shared database. This PRD asks
   PRD-06 for at most one chunk per 5 minutes or 32 MiB.
6. **PRD-00, question 8.** The version floor applies to every mode, not only competition (section 6).
7. **PRD-00, size budget.** The Worker lands near 3,450 src lines, not 3,000, because notices, mail and
   the GitHub commit are out of scope.
8. **PRD-01 section 11.** Say that the server seals the entry when the final heartbeat names the seal
   LINK and the server holds the chain to it; there is no seal call.
9. **PRD-01 section 12, "What fetch needs".** fetch-run concatenates and checks, and does not decode;
   the verifier and the usage reader decode with window 2^27. The `zstd -dc --long=27` line stays as
   the manual way.
10. **PRD-01 section 9.** State that the heartbeat's `model_requests` and `counts` are taken at the
    durable point, so the server never shows a count that a crash could take back.
