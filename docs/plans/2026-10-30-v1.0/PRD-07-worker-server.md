# PRD-07: Worker, server and database

Id: PRD-07. Version: goalcup 1.0. Revision 3, 30 October 2026 (the alignment pass against the ledger;
revision 2 and revision 1 earlier the same day). Base: main at 4b226de2. Index: [PRD-00](PRD-00-index.md)
revision 4 (binding). Format: [PRD-01](PRD-01-record-format.md). Ledger: [OWNER-DECISIONS](OWNER-DECISIONS.md)
wins over this file. Owns seam S5 (the upload HTTP contract, `docs/UPLOAD-API.md`) and seam S13 (the
fetched layout). Revision 3 applies the alignment fix lists (section "Alignment answers" at the end).

## Goal

The server stores every GCR1 chunk the client sends, in order, with its own clock, and never reads
inside it. It seals an entry as soon as it holds the whole chain, and it needs no website database change.

## Points served

- **5** (lead): chunks stored create-only by offset and hash; the entry sealed by the final heartbeat or
  by the chunk that completes it; `fetch-run` writes whatever arrived and names what is missing; no
  per-run size limit.
- **6**: the heartbeat carries written counts, off time and a real `failed`; every heartbeat row is kept,
  so the off spans can be read in time order.
- **3**: no server answer stops a run except `stop` (D64, kept). A malformed `counts` never costs a
  heartbeat. The maintenance switch stays.
- **1, 2**: nothing on the server counts or limits calls, streams, subagents or bytes.

## Owner decisions it carries out

- Verifier and Worker move to the new format in the same build and deploy together with the client.
  No backward compatibility (ledger L7, S1).
- Signing is cut (ledger L6). Signing and the receipt are removed as a pure deletion; nothing replaces them.
- No per-run size limit (ledger R5; owner answers 2 and 3, BRIEF2).
- The heartbeat says `failed`; email work is out of scope, the server can mail later (ledger R7).
- Counts: the participant, status, heartbeat and server see only written counts and off time (R6).
- No website database schema migration; the new Worker fits today's tables; anything new goes in R2 (S3).
- Staging only for build runs; agents deploy the Worker to staging; production deploy is the owner's
  (B4, B5). Staging carries the `1.0.0` floor from its first deploy (PRD-00 question 5).
- Read-only database and R2 credentials live on the conductor only, never in a box (PRD-00 defaults).
- The usage job writes JSON to R2 in its own bucket, in the third slice (PRD-00 S7, build order).

Not in this PRD: the device flow and its approval page, the notices and mail in the cron, the GitHub
judged-commit cron and start-time repository checks (PRD-00 question 4: untouched in 1.0). They stay
exactly as they are.

## Today

| What | Where | Fact |
|---|---|---|
| Worker size | `server/worker/src/*.ts` | 4,001 src lines (`wc -l`), 17 files; tests 5,255 lines in 15 files plus 494 helper lines (appendix C section 9) |
| Routes | `server/worker/src/index.ts:24-60` | device (3), open, position, receipt, abandon, seal, heartbeat, continue, chunks; health at `:65`; maintenance switch `:72-79`; cron `:105-107` |
| Chunk path | `server/worker/src/chunks.ts:34-115` | Header checks, retry by offset `:46-47`, end rules `:55-70`, size limit `:74`, stream to R2 `:202-228`, sign ack `:82`, one atomic commit `:96`, answer `:151-153` |
| Per-run size limit | `chunks.ts:123-144`, `context.ts:12, 62-67, 126`, `wrangler.toml:56-59, 151` | 64 GiB compressed per run, 413 `quota_exceeded` |
| Signing | `src/sign.ts` (116), `context.ts:133-145, 168-170`, `chunks.ts:82`, `runs.ts:352-389` | Ed25519 receipt and per-chunk ack; secret `RECEIPT_SIGNING_KEY` (`wrangler.toml:81`) |
| Abandon | `runs.ts:393-414` | Gives back a run with no chunks |
| Seal | `runs.ts:540-637`, constants `:25-29`, helpers `:57-85` | Takes `seal.json`, `baseline.json`, `tunnels.jsonl` in one JSON body up to 40 MiB, parsed in a 128 MB isolate |
| Heartbeat | `runs.ts:444-491`, vitals validator `:499-533` | `recording` already accepts `failed` (`:455-457`); run 1 never sent it. Optional facts are null when absent; bad vitals are dropped with a log line, because "vitals never cost a heartbeat" (`:502-507`, D64 at `docs/DECISIONS.md:660`) |
| Version floor | `runs.ts:154-164, 228`, `device.ts:54-55`, `wrangler.toml:55, 150` | Competition only; empty in both blocks today |
| Version alias | `util.ts:79-91` | Accepts `proxy_version` or `tracer_version` |
| Close time | `db.ts:17`, SQL `run_status_at` `sql/goal-cup-tracer-upload.sql:317-329` (24 hours at `:323`) | No upload 24 h after the end |
| Tables | `sql/goal-cup-tracer-upload.sql:131-161` (entries), `:163-177` (heartbeats), `:203-216` (chunks; `ack jsonb NOT NULL` at `:214`, `length > 0` at `:206`), `:2549-2553` (stored bytes, `vitals` must be an object) | The website repo owns them; this file is a mirror (`:1-24`) |
| Heartbeat SQL | `sql:2811-2842` (`note_heartbeat`; reads `p_vitals ->> 'agent_paused'` at `:2819`), `:2871-2922` (14-argument `goal_cup_upload_heartbeat`) | Stores `vitals` on the row and as `runs.last_vitals`; `flag_reason` is first-wins (`:2831-2832`); a repeated final heartbeat (`final_existing`) inserts no row |
| Seal SQL | `sql:1409-1448` | Needs final committed and the chain held to it, rechecked under a row lock; stores the two digest strings it is given; keys may be null |
| Disk-pause twin | `db-local.ts:612-623` (`diskPause`), `db.ts:242-249` (`Vitals`) | Mirrors the SQL disk pause for the local store |
| Rebuild | `scripts/fetch-run.sh` (349) | Decodes each chunk alone (`:327`); refuses to write an entry whose chunks stop short (`:214-219`); writes acks, receipt, seal docs; reads R2 by HTTPS with a read-only token (`:24-36`) |
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
  verifier and the usage reader on an organizer machine, or in a box during the build (PRD-08, PRD-09,
  ledger B1).
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
| `PUT /v1/runs/{run}/chunks/{offset}` | keep | no signing; no size limit; answer `{offset, next_seq, head, received_at, sealed}`; the chunk that completes a committed end seals the entry (section 3) |
| `POST /v1/runs/{run}/heartbeat` | keep | new body (section 4); a final heartbeat seals the entry when the chain is held (section 3) |
| `POST /v1/runs/{run}/receipt` | **cut** | signing cut |
| `POST /v1/runs/{run}/abandon` | **cut** | serves no point (appendix I section 4); a failed competition start is replaced by the next start, a practice or test run is harmless test data |
| `POST /v1/runs/{run}/seal` | **cut** | the seal, the start facts and the tunnels are records now (PRD-01 section 11) |
| cron every 5 minutes | keep as is | out of scope; see "Risks" for what it now reads |

A cut path answers `404 not_found` from the existing router (`index.ts:67`). A 0.7 client is refused
earlier by the version floor (section 6).

### 3. The seal happens when the server holds the whole chain

There is no seal call. The entry is sealed by the server when two facts hold: the final heartbeat has
committed the seal LINK (`seq`, `head`), and the server holds the chain up to that link. Whichever
arrives second seals it:

```ts
// runs.ts heartbeat, after recordHeartbeat answered final_committed or final_existing
let sealed = entry.sealed_at !== null;
if (!sealed) sealed = await trySeal(ctx, member, runId, entry.entry_id, head);   // head = the final head

// chunks.ts, after commitChunk answered committed: does this chunk end at the committed end?
let sealed = false;
if (entry.final !== null && h.lastSeq === entry.final.seq && h.head === entry.final.head)
  sealed = await trySeal(ctx, member, runId, entry.entry_id, h.head);

async function trySeal(ctx: Ctx, member: string, runId: string, entryId: string, head: string): Promise<boolean> {
  const r = await ctx.db.sealRun({ member_id: member, run_id: runId, entry_id: entryId,
    seal_sha256: head, baseline_sha256: head, seal_key: null, baseline_key: null,
    sealed_at: formatTime(ctx.now()) });
  return r.kind === "sealed" || r.kind === "already_sealed";   // no_final, chain_behind: not yet
}
```

- It calls the existing SQL `goal_cup_upload_seal_run` (`sql:1409-1448`) unchanged. That function
  rechecks final and chain under a row lock, so two racing calls are safe. `head` is always the final
  head when the seal succeeds: the heartbeat passes the head it just committed, and the chunk path
  only tries when its own head equals the committed final head. The two digest columns get that hash;
  the two key columns get null (`delete_test_runs` already skips null keys, `sql:2142-2143`).
  **No migration.**
- Why both paths: if the client stops right after its last chunk (Ctrl-C, laptop lid) and never
  repeats the final heartbeat, the chunk path still seals the entry. Without it, the run would turn
  `incomplete` 24 hours later although the server holds every byte (point 5).
- **Client rule (S5, PRD-06):** send the final heartbeat; upload the rest; repeat the final heartbeat
  until an answer says `sealed: true` (heartbeat, chunk or position answer). A repeat is
  `final_existing`: no row, no cost. The repeat covers the one race the chunk path cannot see (the
  final heartbeat commits while the last chunk is in flight). This also keeps the order the server
  already enforces after `ends_at`: chunk `503 awaiting_final` until the final heartbeat (`chunks.ts:55-59`).
- A resent chunk the server already holds is answered from the stored row (`chunks.ts:46-47`) with
  `sealed` from the entry as read.
- Run status rules are unchanged: `sealed` once the run is over and its current entry is sealed,
  `incomplete` 24 hours later without it (`db.ts:48-52`).

### 4. Heartbeat: written counts, off time and three states sent

Body (S5, sent by PRD-06):

```json
{"entry_id":"e0123456789abcdef","seq":3601,"head":"<64 hex>","offset_local":726000000,"final":false,
 "repo_head":null,"recording":"gaps","model_requests":905,
 "counts":{"v":1,"boot":2,"mode":"recording","off_ms":2300,"off_spans":1,"off_since":null,
           "gaps":3,"lost_bytes":1500000,"lost_unknown":0,"http_post":243,"ws_up_msgs":662,
           "conns":120,"tunnels":118,"wire_bytes":674000000,"native_files":43,"native_bytes":48200000,
           "waiting_bytes":1048576}}
```

- `seq`, `head`, `offset_local` are the **durable point** (PRD-02 seam S4): the last LINK on disk and the
  raw offset just past it. The final heartbeat names the seal LINK.
- `model_requests` is PRD-01 section 9's "model requests written" (`http_post + ws_up_msgs`), summed
  over boots, **as of the durable point**, so the server's number never exceeds what a server copy
  can show. It goes to `entries.model_requests` (kept as the highest per entry, `sql:2817-2821`).
- `counts` is a flat JSON object of whole numbers, short strings (at most 64 bytes), booleans and nulls,
  at most 2 KiB serialized. `mode` is `"recording"`, `"losing"`, `"writer_slow"`, JSON null (null
  mode) or `"sealed"` (PRD-02 section 6; the final heartbeat sends `mode: sealed`).
  `off_since` is the start of the open off span as an RFC 3339 string, or null. `waiting_bytes` is the
  compressed bytes on disk not yet confirmed by the server (PRD-06 section 11). The Worker fixes no key
  list inside `counts`; PRD-06 section 6 names the keys and `docs/UPLOAD-API.md` copies them.
- **The client always sends `recording`, `model_requests` and `counts`** (PRD-00 ruling): the beat
  thread from S4, and `goalcup upload` of a sealed or recovered entry from `pipeline::entry_totals`
  (R3-2), which equal the seal's totals. The Worker still treats all three as optional facts, as today
  (`runs.ts:455-459`): a missing one is stored as null. A `counts` that is not a flat object within
  2 KiB is **dropped**: the heartbeat is recorded with `p_vitals` `{"counts_dropped": <serialized byte
  length>}` and the log line `counts-dropped <run_id>` (no value), so the drop is visible on the row. It is
  never a `400`: a refused heartbeat is a heartbeat lost, and a refused final heartbeat would leave the
  entry unsealed and every later chunk `503`. That is D64's rule for vitals today (`runs.ts:502-507`).
- **Storage.** The Worker passes `{"counts": <counts>}` as `p_vitals` to the existing 14-argument SQL
  function. It lands in `heartbeats.vitals` and `runs.last_vitals`. Nesting it under one key means no
  client key can reach the old disk-pause rule, which reads `vitals ->> 'agent_paused'` at the top
  level (`sql:2819`), and the Worker never names that field. Readers use `vitals -> 'counts' ->> 'off_ms'`.
  A heartbeat without `counts` calls the 13-argument form, as today (`db-supabase.ts:357-359`). The
  server interprets nothing. Column names stay `vitals` because renaming is a migration;
  `docs/UPLOAD-API.md` says so in one line.
- The old `vitals` validator (`runs.ts:499-533`), the `Vitals` fields in `db.ts:242-249` and the
  local disk-pause twin (`db-local.ts:612-623`) go. `db-local.ts` stores `last_vitals` as given; its
  `disk_paused_at` and `disk_notice_at` stay null, as the SQL columns will.
- `recording`, decided by PRD-06 section 6 from `mode` and the counts (one rule for PRD-06, PRD-07,
  PRD-08 and PRD-16; PRD-06's text wins if they ever differ):

| `recording` | Means | Client sends it when |
|---|---|---|
| `ok` | recording, nothing lost | `mode` `recording` (or `sealed`), and `gaps`, `lost_bytes`, `lost_unknown`, `off_ms` and `streams_in_gap` are all 0 |
| `gaps` | recording now (or sealed), something was lost | any of those is above 0, or `mode` is `losing` or `writer_slow`. A recovered entry is `gaps` (PRD-06 section 6) |
| `failed` | **recording is off now** | null mode, or a writer stalled past 60 s (`writer_idle_ms` over 60 s while records wait); and only then |

1.0 never sends `unknown` (no heartbeat goes before the first durable point). The Worker and the SQL
CHECK (`sql:174`) keep accepting it; nothing changes there.

`failed` is a state, not a latch: the next heartbeat after a good boot says `gaps`. The server keeps
every heartbeat row, so the off spans are visible in time order. `runs.flag_reason` stays first-wins
(`sql:2831-2832`): a run flagged `gaps` first never shows `failed` there, which is why every reader
reads the `heartbeats` list in fetch-run's `fetch.json` (section 7), not the flag.

The answer gains `sealed`. `stop`, `reason`, `deadline_at`, `seconds_left` are unchanged (D64, D66).

### 5. No per-run size limit, and what storage costs

Delete the 413 `quota_exceeded` check (`chunks.ts:74, 123-135`), its config (`context.ts:12, 26-27,
62-67, 126`) and its two `wrangler.toml` lines. Keep the stored-bytes counter (the 4-argument
`goal_cup_upload_commit_chunk`, `sql:2758-2804`, already adds each chunk's compressed size) and keep
one log line at every 10 GiB a run crosses (replacing `noteQuota`, `chunks.ts:138-144`), so the owner
sees a large run in Workers Logs.

Storage cost, stated (R2 at $0.015 per GB-month, Class A $4.50 per million after 1 million free a month,
egress free: [R2 pricing](https://developers.cloudflare.com/r2/pricing/)):

| Case | Compressed size (estimate) | Cost |
|---|---|---|
| Run 1 rate, 2 hours, whole | 75 to 150 MB (PRD-01 section 14) | under $0.01 a month |
| A busy week at run 1's rate | 10 to 20 GB (PRD-01 section 14) | $0.15 to $0.30 a month; the competition prefix is locked 180 days (`wrangler.toml:31`), so about $0.90 to $1.80 per run in all |
| 200 agents at once, a week (PRD-01 section 14 estimate) | about 400 GB | about $6 a month |
| 1,500 busy weeks | 15 to 30 TB | $225 to $450 a month, about $1,350 to $2,700 over the lock |
| A stuck or hostile client at 100 Mbit/s for 8 days | about 8.6 TB | about $130 a month, about $780 over the lock, for that one run |
| Class A writes, a chunk every 5 minutes or at once when 16 MiB compressed waits (PRD-06 section 4) | up to about 3,000 per busy entry-week (2,016 time-driven plus bursts); 1,500 runs about 4.5 million | about $16 above the free million in the event month |

The limit guarded only the hostile row. Without it, the guard is a person: the 10 GiB log lines, and
`stored_bytes` per entry in every `fetch.json` (section 7). The manual lever, should a run misbehave,
is to delete that run's row in `goal_cup_tracer.run_credentials` (the client then gets `401`, keeps its
data and stops uploading); that is a hand write to the shared database by the owner, not code. Bytes
already under the lock cannot be deleted for 180 days.

### 6. Open, continue and the version floor

- `connector_sha256` (64 hex, the SHA-256 of the connector file the client ran, PRD-11) replaces the two
  digests. The Worker writes it into both NOT NULL columns `manifest_sha256` and
  `connector_dir_sha256` (`sql:137-138`). No migration.
- The `tracer_version` alias goes (`util.ts:79-91`): only `proxy_version` is read. The column name stays.
- **`MIN_COMPETITION_VERSION` applies to every mode**, not only competition (`runs.ts:228`,
  `device.ts:55`): a 0.7 client in practice or test mode would otherwise open a run and then fail at a
  cut route with a bare 404.
- **Where the floor is set.** `MIN_COMPETITION_VERSION = "1.0.0"` goes into **both** blocks of
  `wrangler.toml` (`:55` production, `:150` staging) in the commit that first deploys 1.0 to staging
  (PRD-00 question 5, PRD-16 section 3). Staging refuses 0.7 clients from that deploy; no 0.7 client
  runs on staging in this build, and it tests the refusal path. Production gets the floor only when the
  owner deploys production, after the 1.0.0 release is published (PRD-14), so nobody is refused while
  1.0 cannot be downloaded. A dashboard edit is wiped by the next deploy (`wrangler.toml:51-55`).
  `continue` stays unchecked (D70).
- **The `403` text becomes mode-neutral** (today it says "too old for the cup", `runs.ts:163`):
  `This goalcup (<sent>) is older than this server accepts (<min>). Update it: <INSTALL_LINE>`.
  PRD-10 shows it as is.
- Everything else in open and continue stays: server clock (D16), one main run, the start window,
  GitHub public-and-empty check, `machine`, the IP and country columns (owner may cut them later).

### 7. `fetch-run`: the server copy, whole or partial (seam S13)

`scripts/fetch-run.sh` is rewritten to fetch bytes and decode nothing:

```
sh scripts/fetch-run.sh [--bucket B] [--index FILE] [--store DIR] [--local PERSIST_DIR] <run|entry> <outdir>
<outdir>/<entry>/rec.gcr.zst   every chunk of the entry, SHA-256 checked, concatenated in offset order
<outdir>/<entry>/fetch.json    the entry's server facts and chunk list (below)
```

The file is `fetch.json`, not `server.json`, so it never clashes with the client's local `server.json`
(PRD-10). It is the one source for PRD-08 (`server_chunks_whole`, `server_counts_match`), PRD-12 (C4,
K6) and PRD-16 (X6).

- **Where it runs.** Against staging: on the conductor, with the read-only database URL and a read-only
  R2 token from the environment (never printed, `fetch-run.sh:24-36`), never in a box. Staging:
  `--bucket goalcup-traces-staging`. With `--local` (no credential): in the build or pad box where
  `wrangler dev` runs. The fetched folder is then copied into a box, and every `goalcup-verify`
  command runs there (ledger B1, PRD-00 "Where the build runs").
- **Index:** one `BEGIN READ ONLY` query as today (`fetch-run.sh:121-137`), now selecting per entry
  `run_id`, `segment`, `upload_offset`, `next_seq`, `head`, `model_requests`, `final_seq`, `final_head`,
  `final_at`, `sealed_at`, `stored_bytes`, `chunk_count`; every heartbeat row of the entry in time
  order (`received_at`, `seq`, `offset_local`, `final`, `recording`, `model_requests`,
  `vitals -> 'counts'`, `vitals -> 'counts_dropped'`); and per chunk `offset`, `length`,
  `stored_bytes`, `sha256`, `first_seq`, `last_seq`, `prev`, `head`, `r2_key`, `received_at`. The
  `ack`, `receipt`, `seal_key` and `baseline_key` columns are no longer read. At one heartbeat row per
  5 minutes plus state changes, a 168-hour entry has about 2,100 rows, under 5 MB even with every
  `counts` at its 2 KiB bound.
- **Checks, per entry:** offsets run on from 0; `first_seq` runs on from 0; each chunk's `prev` is the
  previous chunk's `head` (genesis for the first; today's script never compares them); each object
  matches its SHA-256; the last chunk ends at `upload_offset`. An object that is missing or fails its
  SHA-256 ends that entry there: `rec.gcr.zst` holds the chunks before it, `missing` gains
  `"why":"object_bad"` and the key, and the script exits 1.
- **Partial is normal.** An entry whose chunks stop before the committed end, or that has no final
  heartbeat, is written as far as it goes, and `fetch.json` says what is missing. Today the script
  writes nothing then (`fetch-run.sh:214-219`). A hole in the middle cannot happen (the Worker accepts
  only the next offset, `chunks.ts:68-70`); if the index shows one, the script stops with no output for
  that entry, as today.
- `fetch.json`:

```json
{"v":1,"run_id":"r…","entry_id":"e…","segment":0,"upload_offset":726000000,"next_seq":3602,
 "head":"<hex>","model_requests":905,"final":{"seq":3601,"head":"<hex>","at":"…"},
 "sealed_at":"2026-11-14T00:00:09Z","complete":true,"missing":null,
 "last_heartbeat":{"received_at":"…","seq":3601,"recording":"gaps","model_requests":905,
                   "counts":{"off_ms":2300,"…":"…"}},
 "heartbeats":[{"received_at":"…","seq":12,"offset_local":2400000,"final":false,"recording":"ok",
                "model_requests":4,"counts":{"…":"…"},"counts_dropped":null}],
 "stored_bytes":96000000,"chunk_count":121,
 "chunks":[{"offset":0,"length":16777216,"bytes":1810000,"sha256":"<hex>","first_seq":0,"last_seq":18,
            "prev":"<hex>","head":"<hex>","r2_key":"traces/…","received_at":"…"}]}
```

  `complete` is true when `final` is set and `next_seq - 1 = final.seq` with the same head. Otherwise
  `missing` is `{"from_offset": <upload_offset>, "from_seq": <next_seq>, "to_seq": <final.seq or null>}`.
  `heartbeats` holds every heartbeat row of the entry in `received_at` order (PRD-06 U6 and PRD-16 X6
  read it instead of their own query); `last_heartbeat` is its last element, kept for short readers.
  `bytes` gives each chunk's place in `rec.gcr.zst`. If PRD-01's recovery fallback is used, the reader
  finds part starts by trying a zstd frame header (`28 b5 2f fd`) at each chunk start.
- **Decoding is the reader's job.** `goalcup-verify check --server <dir>` (PRD-08) and the usage reader
  (PRD-09) decode `rec.gcr.zst` with window 2^27, which is also the default limit of `zstd -d`
  (appendix M section 4.4). A person can run `zstd -dc --long=27 rec.gcr.zst`; an entry that is not
  sealed ends inside an open frame, so `zstd` reports a truncated input after writing every whole
  block (expected, PRD-01 section 12). fetch-run no longer needs `zstd`.
- **Byte equality.** `rec.gcr.zst` equals the concatenated local part files, byte for byte, up to the
  server's offset (PRD-01 acceptance 2), when the run kept its parts (`GOALCUP_KEEP_PARTS=1`).
- **`--local PERSIST_DIR`** reads a local Worker (`wrangler dev` with `DB_BACKEND=local`), for builds
  before staging carries 1.0 and for contributors without staging access (PRD-14's `upload_worker.rs`
  and the full-tier Worker test use it, so it stays). The index is read from the R2 object
  `state/runs/<run>.json` in the local persist store (`db-local.ts:5-12`), not a file, and its shape is
  `db-local.ts`'s, not the SQL index. Each object is read with
  `npx wrangler r2 object get <bucket>/<key> --local --persist-to <dir> --file <dest>`. Whether those
  two flags work on `r2 object get` in wrangler 4.149.0 is **unverified**; the builder checks it first.
  Fallback if they do not: `--store DIR` over a mirror made per key, and no test-only Worker route.

### 8. After merge: finding runs whose recording stopped

Not in this build (PRD-00 "Not in this build"; review B N11). Until then, organizers read the
`heartbeats` list in `fetch.json` (section 7) per run. The design is kept here so the later work does
not start over:

- New file `server/worker/scripts/stopped-runs.sql`, run read-only (`BEGIN READ ONLY` ... `ROLLBACK`),
  filtering `staging`. For each open current entry with no final heartbeat it gives one `why`:
  `never_heard` (no heartbeat), `off_now` (latest `recording = failed`), `silent` (no heartbeat for 20
  minutes; same window as the cron's `SILENCE_AFTER`, `notices.ts:20`), `writer_stuck` (the durable
  `seq` has not moved for 30 minutes; run 1's tell was nine heartbeats with one `seq`), `wire_quiet`
  (`native_bytes` grew by over 1 MiB in 30 minutes while `model_requests` stayed the same: a hint of a
  dodger or a lost `HTTPS_PROXY`, not proof). A second query lists the 20 runs with the most stored bytes.
- Known limit (review A finding 6): `wire_quiet` needs zero written requests for 30 minutes, so a run
  whose main agent is recorded while some subagents are not shows nothing here. The live check on the
  client belongs to PRD-02 and PRD-10.

### 9. Read-only access for organizers

| Need | 1.0 (no database change) | Later, owner's choice |
|---|---|---|
| Database | The read-only database credential the owner provides lives on the conductor only (PRD-00). Every organizer script also opens `BEGIN READ ONLY` and ends `ROLLBACK` (fetch-run). Staging and production share one database (`sql:1092`), so every query filters `staging` | A login role with `SELECT` on `goal_cup_tracer.runs, entries, chunks, heartbeats, events`, with column grants that leave out `client_ip`, `client_country`, `last_client_ip`, `last_client_country`, `machine`. That is a website migration through the pending flow (`server/worker/README.md:64`), applied by the owner. The SQL text goes in the PR description, not in `sql/` |
| R2, traces | A Cloudflare R2 token with **Object Read only**, scoped to `goalcup-traces-staging` for the build (and `goalcup-traces` for the owner). R2 tokens can be scoped to buckets but not prefixes ([R2 tokens](https://developers.cloudflare.com/r2/api/tokens/)). On the conductor only | none |
| R2, usage | Read: an Object Read token on the usage bucket. Write: only the usage job (PRD-09) | none |

Test runs use the existing staging bucket `goalcup-traces-staging`; no separate test bucket is made.
`delete-test-runs.sh` already cleans test runs there. `delete-test-runs.sh` is the owner's tool (it
writes to the shared database and to R2). Agents never run it (ledger B8, S3).

### 10. Where the usage stats live (third slice)

- **An R2 object, not a table.** Seam S7 (PRD-09): `usage/{event}/{run}/{entry}.json` and its
  siblings, written by the organizer job. No database migration.
- **A separate bucket**, `goalcup-usage` (`goalcup-usage-staging` during the build), not the traces
  bucket. Reason: the job needs write access, R2 tokens scope only to whole buckets, and a write token on
  a traces bucket could delete unlocked trace chunks (test, dry run, practice). Usage JSON is derived
  and can be rebuilt, so it sits outside the bucket lock and may be overwritten when the reader
  improves. The Worker does not bind or serve it.
- Agents may create `goalcup-usage-staging` (ledger S3). `goalcup-usage` is the owner's to create with
  the production deploy.
- `server/worker/scripts/delete-test-runs.sh` also empties `usage/test/` and `usage/test-staging/` in
  the usage buckets, so a test-run cleanup leaves no orphan stats. This lands with the job, in the third
  slice; before that the usage buckets hold nothing to clean. Agents edit the script; only the owner
  runs it (section 9).

### 11. Website database: nothing to migrate in 1.0

| 1.0 needs | How it fits the tables as they are | A later migration could |
|---|---|---|
| No signed ack | write `{}` into `chunks.ack` (NOT NULL, `sql:214`) and `entries.last_ack` | drop the column |
| No seal documents | final head into both digest columns, null keys (section 3) | rename or drop |
| Counts and off time | `heartbeats.vitals`, `runs.last_vitals` as `{"counts": ...}` (jsonb object, `sql:2552-2553`) | rename to `counts` |
| No disk pause | `runs.disk_paused_at` and `disk_notice_at` stay null: no `agent_paused` key reaches `note_heartbeat` | drop them and the `disk_full` notice |
| `failed` after `gaps` | read `fetch.json` `heartbeats` (every row); `flag_reason` stays first-wins | make the flag the latest state |
| One connector digest | write it into both digest columns | merge them |
| Usage stats | R2 JSON (section 10) | add tables if organizers want SQL over them |
| Version floor in all modes | Worker only | none |
| Receipt and abandon | the SQL functions stay, unused | drop them and the 13-argument overloads |
| 24-hour close and credential life | unchanged (`sql:323`, `:2603`) | lengthen both together for point 5 (PRD-00 question 1) |

Only the owner applies a migration to the shared production database (website CLAUDE.md); none is needed.

### 12. Deploy order with the client

1. **Before staging carries 1.0:** builds test against a local Worker (`wrangler dev`,
   `DB_BACKEND=local`) or with `GOALCUP_SERVER=off` (PRD-00 defaults, PRD-12 `--server local|off`).
2. **First staging deploy (agents, ledger B5):** the integrator runs `wrangler deploy --env staging`
   from the integration head as soon as PRD-07 builder step 2 and PRD-06 are merged (PRD-16 section 3),
   with the `1.0.0` floor in both blocks (section 6). Every later staging deploy is from the integration
   head and writes a night-log line with the commit and the Worker version id. Every pad and acceptance
   run uploads to staging from then on.
3. **Before the owner's production deploy:** the owner checks that no real run is open
   (`SELECT count(*) FROM goal_cup_tracer.runs WHERE status='open' AND NOT staging AND mode <> 'test'`
   inside `BEGIN READ ONLY`). Every run so far is test data, but a 0.7 client mid-run would stop
   uploading after the deploy (its seal and receipt routes are gone); it keeps its data.
   Note for the owner: with 1.0 the unchanged cron sends the `recording` email when a heartbeat says
   `failed`, and the idle notices count written requests. No email code changed; confirm this is
   wanted before the production deploy (PRD-16 `MORNING.md` owner steps carry the same line).
4. **Release:** tag `v1.0.0` (PRD-14), so `releases/latest` serves 1.0.
5. **Production (owner, from his own device):** `wrangler deploy` from the same commit, then delete the
   `RECEIPT_SIGNING_KEY` secret (only after the deploy: the 0.7 Worker refuses every call without it,
   `context.ts:136`). Create `goalcup-usage` (the usage job is in this build, third slice).
6. **Smoke (owner):** one practice run with the 1.0 client on production, sealed, fetched, checked.
7. The `EVENT_ID` switch stays the owner's manual deploy between 6 Nov 00:00 and 7 Nov 00:00 UTC
   (`wrangler.toml:37-42`); it can be the same deploy as step 5 if the timing fits.

**No rollback to the 0.7 Worker** once 1.0 clients run: it would refuse their heartbeats and never seal
them. A bad 1.0 Worker is held with `MAINTENANCE=on` (clients back off and keep everything,
`index.ts:72-79`) and fixed forward.

## Files it owns

Change:
- `server/worker/src/index.ts` (routes), `chunks.ts` (no signing, no quota, 10 GiB log line, seal on
  the completing chunk, `sealed` in the answer), `runs.ts` (heartbeat, seal on final, open and continue
  fields, version floor, cut receipt, abandon, seal, `vitalsField`), `context.ts` (no signer, no byte
  limit), `db.ts` (`Vitals`, `receipt`, `last_ack`, `StartReceipt`, `Ack` go; `SealArgs` keys nullable),
  `db-local.ts` (receipt and abandon go, `diskPause` twin goes, vitals stored as given),
  `db-supabase.ts` (`storeReceipt`, `abandonRun` go; `receipt`, `last_ack`, `ack` leave the row
  shapes), `device.ts` (version floor in all modes), `util.ts` (alias cut), `env.ts` (drop
  `RECEIPT_SIGNING_KEY`, `RUN_MAX_COMPRESSED_BYTES`).
- `server/worker/test/*.test.ts`, `test/helpers/*.ts` (section "What is cut"; new cases below).
- `server/worker/wrangler.toml` (vars, floor in both blocks, secret list, comments), `server/worker/README.md`.
- `server/worker/scripts/delete-test-runs.sh` (usage buckets, third slice).
- `scripts/fetch-run.sh`, `scripts/test_fetch_run.py`, `docs/UPLOAD-API.md` (seam S5).

Create: nothing in this build. (`server/worker/scripts/stopped-runs.sql` and its `check-sql.sh` step
come after the merge, section 8.)

Delete: `server/worker/src/sign.ts`, `test/sign.test.ts`, `test/vectors/signing.json`, `test/quota.test.ts`.

Must not touch: `server/worker/sql/**` (the website's mirror; regenerated only from the website repo),
`server/worker/src/notices.ts`, `mail.ts`, `github.ts`, `ratelimit.ts` (out of scope), anything under
`crates/` (PRD-01 to PRD-11), `docs/*.md` other than `docs/UPLOAD-API.md` (PRD-15).

## Interfaces with other PRDs

| PRD | What | Direction |
|---|---|---|
| PRD-00 | S5 and S13 as ruled; staging floor; read-only credentials on the conductor; questions 1, 4, 5 | binding here |
| PRD-01 | Chunk headers, key, raw offsets, link seq and hash (section 12); seal LINK and "no seal call" (section 11); counts at the durable point (section 9); recovery fallback (section 12) | used here, unchanged |
| PRD-02 | S4 durable point and `status.json` (`mode`, written counts, off); null mode | the heartbeat's source |
| PRD-06 | **S5**: chunk headers and answers (with `sealed`), heartbeat body (flat `counts` with `waiting_bytes`, `repo_head`; PRD-06 section 6 names the `counts` keys) and the three states it sends (PRD-06 section 6's rule), the rule "repeat the final heartbeat until an answer says `sealed`", `goalcup upload` sends `pipeline::entry_totals` (R3-2), error table, backoff (D64 unchanged). Cadence: a chunk every 5 minutes or at once when 16 MiB compressed waits (PRD-00 question 1) | owned here, published by H+2 |
| PRD-08 | **S13**: `<entry>/rec.gcr.zst` and `<entry>/fetch.json` (entry row, last heartbeat, every heartbeat row, chunk list with keys, SHA-256 and compressed size); `check --server` decodes and compares | owned here |
| PRD-09 | S7 key; the usage bucket names; reads fetch-run output | bucket named here, third slice |
| PRD-10 | **S5**: open and continue bodies (`connector_sha256`, `proxy_version` only), the version floor `403` and its text (section 6); `sealed` in heartbeat, chunk and position answers; the plain words for `failed`; the local `server.json` stays PRD-10's | used there |
| PRD-11 | `connector_sha256` value | used here |
| PRD-12 | `fetch-run` from the conductor against staging, `--local` in the box where `wrangler dev` runs before staging; the fetched copy is copied into the pad box and every `goalcup-verify` check runs there; `fetch.json` for C4 and K6; test runs in `goalcup-traces-staging` | used there |
| PRD-13 | Deletes the Rust stand-in server `crates/goalcup-cli/tests/common/upload_server/` (2,514 test lines) with the old crate. Its no-return guard must skip `server/worker/sql/` (the mirror keeps `receipt` and `agent_paused`); after this PRD, Worker `src/` names neither | one server, not two |
| PRD-14 | Gate runs `npm test` in `server/worker` and `scripts/test_fetch_run.py`; PRD-06's tests run against `wrangler dev` or the fake Worker in `tests/common/`, which follows S5 | used there |
| PRD-15 | Fix stale docs that name signing, receipts, acks, the seal call, the verify job (`docs/OPERATING.md`, `docs/SERVER-DESIGN.md`, `docs/CHEAT-CHECKLIST.md`); `server/worker/README.md` absorbs SERVER-DESIGN's still-true facts (builder step 5), then PRD-15 moves that file | done there |
| PRD-16 | First staging deploy after builder step 2 and PRD-06; production deploy order (section 12); X6 reads `fetch.json` `heartbeats` (`failed` only while recording is off, section 4's rule) and runs its checks in the run's pad box | used there |

## What is cut

| Path | Src lines | Tests that go |
|---|---|---|
| `src/sign.ts` and signer plumbing (`context.ts:133-145, 168-170`, `chunks.ts:82`) | about 135 | `test/sign.test.ts` (68), `test/vectors/signing.json` (26), ack checks in `test/helpers/client.ts` |
| Receipt route and DB methods (`runs.ts:352-389`, `db-*.ts`) | about 75 | receipt cases in `integration.test.ts`, `lifecycle.test.ts`, `supabase.test.ts`, `device.test.ts` (about 150, est.) |
| Abandon route and DB methods (`runs.ts:393-414`, `db-*.ts`) | about 60 | abandon cases in `integration`, `local-db`, `supabase`, `config` (about 120, est.) |
| Seal route, documents, tunnel list (`runs.ts:25-29, 57-85, 535-637`) | about 140, replaced by about 20 (section 3, both paths) | seal-document and tunnel cases in `integration.test.ts` (14 tunnel references), `lifecycle`, `segments` (about 250, est.) |
| Per-run size limit (`chunks.ts:74, 123-144`, `context.ts:11-12, 26-27, 61-67, 126`) | about 50 | `test/quota.test.ts` (114), `config.test.ts:237` |
| Vitals validator (`runs.ts:499-533`) and `Vitals` fields (`db.ts:242-249`) | about 45, replaced by about 12 | vitals cases in `notices.test.ts` (31 references, about 70 lines) |
| Local disk-pause twin (`db-local.ts:612-623`) | about 12 | disk-pause cases in `local-db.test.ts` (est. 40) |
| `tracer_version` alias (`util.ts:79-91`) | about 10 | `lifecycle.test.ts:481-505`, `device-unit.test.ts` alias cases |
| fetch-run acks, receipt, seal docs, per-chunk decode (`fetch-run.sh:12-14, 174-184, 222-226, 282-318, 326-332`) | about 80 removed, about 45 added | matching cases in `scripts/test_fetch_run.py` |

Net: Worker about 4,000 to about 3,440 src lines (est.). PRD-00's size table (about 3,450) holds; the
first 3,000 target assumed notices, mail and the GitHub commit moved out, which is not in this build.
Tests about 5,750 to about 4,950 lines (est.). Secrets the owner holds drop from six to five
(`RECEIPT_SIGNING_KEY` goes).

## Acceptance

Real-run proofs first (pad, PRD-12, Linux sandbox, **staging Worker**; `fetch-run` on the conductor with
`--bucket goalcup-traces-staging`, the fetched copy then copied into the run's pad box, where every
`goalcup-verify` command runs (ledger B1); every run with `GOALCUP_KEEP_PARTS=1`). Before staging
carries 1.0, the same proofs may run once against `--local` in a box as an early signal; they count
only on staging.

1. **Whole server copy, sealed.** Claude Code, direct key, `smoke` prompt. Pass: an answer says
   `sealed: true`; `fetch-run` writes `rec.gcr.zst` whose SHA-256 equals that of the concatenated local
   parts; `fetch.json` has `complete: true` and `model_requests` equal to the seal's written total;
   `goalcup-verify check --server` passes.
2. **Kill test on the server side.** The Codex run of PRD-02 R1. Pass: PRD-06 U6 holds; the server
   copy equals local; the entry seals; `upload.json` and `goalcup.log` show no chunk answer other than
   `200` and `409`. The first kill comes after the first heartbeat row (PRD-02 R1 says so).
3. **Partial, then whole.** The run of PRD-06 U8 (watchdog and recorder killed together mid-run). Pass,
   before `goalcup upload`: `fetch-run` writes `rec.gcr.zst` with `complete: false` and `missing` naming
   `from_seq`, and `goalcup-verify check --server` reports an open end, not an error. After
   `goalcup upload`: a second fetch is `complete: true`, equals local, and the last heartbeat in
   `fetch.json` carries `counts` (the seal's totals).
4. **Size, measured.** From the A1 and A2 runs (PRD-16), report compressed bytes per hour from
   `fetch.json` `stored_bytes` into section 5's table. No `413` appears in any run (there is no 413 path).

`failed` end to end: one automated test in a build box runs the `test-hooks` binary with forced null
mode (PRD-02 `watchdog_null.rs` setup) against `wrangler dev` (`DB_BACKEND=local`) and asserts a stored
heartbeat with `recording` `failed` and `vitals.counts.mode` JSON null, then `gaps` after the next good
boot. No real-run spend. Owner of the test: PRD-06 (`upload_worker.rs` case 2), against this PRD's Worker.

Automated tests that guard them:

- `integration.test.ts`: chunks then a final heartbeat seal the entry; a final heartbeat first, then
  chunks: the last chunk's answer says `sealed: true` with no repeat; a repeat final heartbeat after
  that answers `sealed: true` and adds no row; position says `sealed`; a heartbeat with
  `recording: "failed"` and `counts` stores both, `vitals` as `{"counts": ...}` on the row and the run;
  a `counts` over 2 KiB or not flat gives `200`, a row whose `vitals` is `{"counts_dropped": n}` and one
  `counts-dropped` log line;
  a `counts` with an `agent_paused` key starts no pause; receipt, abandon and seal paths are `404`; no
  chunk is refused for size.
- `config.test.ts`: `MIN_COMPETITION_VERSION` refuses a 0.7 practice and test open and device request;
  `continue` is not refused.
- `test_fetch_run.py`: a partial entry is written with `missing`; a chunk whose `prev` is not the
  previous `head` stops that entry; a missing object or one that fails its SHA-256 ends that entry
  there with `"why":"object_bad"` and exit 1; output equals the concatenated chunk bodies; `fetch.json`
  holds the last heartbeat's `recording` and `counts`; `heartbeats` holds every row in `received_at`
  order.

## Risks and blind spots

| Risk | What could go wrong | What we do |
|---|---|---|
| No size limit | A stuck client fills R2 with locked bytes for 180 days | 10 GiB log lines, `stored_bytes` in `fetch.json`, the credential lever (section 5); the top-runs query after merge |
| One lost chunk | A chunk is not decodable alone, so a missing middle chunk loses the rest of its part (PRD-01 risks) | The Worker accepts only the next offset; fetch-run checks prev and head continuity |
| Seal on the chunk path | One more database call, only on the chunk that ends at a committed final | The SQL rechecks under its lock; a wrong head cannot seal because the path tries only with the committed final head |
| Final heartbeat races the last chunk | Neither path sees both facts, the entry stays unsealed | The client repeats the final heartbeat until `sealed` (S5) |
| `counts` dropped | A client bug sends a bad `counts` and the server keeps only `recording` and `model_requests` | The row's `vitals` is `{"counts_dropped": n}` (shown in `fetch.json` `heartbeats`) and the log line `counts-dropped`; PRD-06's `upload_beat.rs` test pins the body; a dropped `counts` never loses the heartbeat (D64) |
| `vitals` named wrongly | A reader of the website database thinks `vitals` holds disk facts | One line in `docs/UPLOAD-API.md`; readers use `vitals -> 'counts'`; rename in a later migration |
| `flag_reason` first-wins | The cron's `recording` email and the run flag say `gaps` even when recording later failed | Readers use `fetch.json` `heartbeats`; email work is out of scope |
| Cron reads the new numbers | `idle_3h` and `idle_6h` (`notices.ts:163-171`) now see written requests, so a run whose proxy dies gets an idle email it did not get before | That is the honest signal; out of scope to change |
| No organizer query in this build | During the build nobody is told a staging run stopped recording | Watchers read `fetch.json` `heartbeats` per run (PRD-16 X6); `stopped-runs.sql` after merge |
| 24-hour close | A laptop off for a day after the deadline loses its tail (point 5), `sql:323` | PRD-00 question 1: unchanged, with PRD-06's one-line "run `goalcup upload` before <time>" warning (review A finding 15) |
| Old client mid-run at deploy | Its routes are gone and it never seals | Owner checks no real run is open (section 12 step 3) |
| Recovery fallback in PRD-01 | A plain concatenation stops at the first crashed part | `fetch.json` `bytes` gives each chunk's place; the reader finds part starts by trying a zstd frame header (`28 b5 2f fd`) at each chunk start |
| `wrangler r2 object get --local` | The flags may not exist on that command | Builder checks first; fallback `--store` (section 7) |
| Organizer access | The read-only credential is the owner's choice of role; `BEGIN READ ONLY` guards only mistakes | Conductor only; the reader role is a later owner choice |
| Zone body limit | The zone's plan is unverified (appendix I section 7); 100 MB on Free and Pro | 32 MiB chunks fit any plan |
| R2 bucket lock and create-only | Create-only under the lock is unproven (`server/worker/README.md:405-408`) | Owner step on the first competition chunk, unchanged |

## Overridable defaults

| Default | Value | Who may change it |
|---|---|---|
| Version floor | `1.0.0`, every mode; staging from its first 1.0 deploy, production at the owner's deploy | owner |
| Bad `counts` | dropped with a log line and `{"counts_dropped": n}` on the row, heartbeat kept | this PRD |
| `counts` bound | 2 KiB, flat, strings at most 64 bytes | this PRD |
| Usage storage | separate bucket `goalcup-usage` (`-staging` in the build) | owner, PRD-09 |
| Test bucket | the existing `goalcup-traces-staging` | PRD-16 |
| Large-run log line | every 10 GiB per run | owner |
| IP and country columns | kept as today | owner (appendix I suggests cutting) |
| Close and credential life | 24 hours after the end | owner, by website migration (PRD-00 question 1) |
| Reader role | not created; SQL proposed | owner |

## Builder notes

Order of work (step 2 is the milestone that lets PRD-16 deploy staging, review B O7):

1. H+0 to H+2: rewrite `docs/UPLOAD-API.md` sections open, continue, position, chunks, heartbeat,
   "When a run ends", error table; delete receipt, abandon, seal sections. Publish S5 in the night log.
   PRD-06 builds against it.
2. Heartbeat `counts` (nested storage, drop on bad shape), `recording` table, seal on final heartbeat
   and on the completing chunk, `sealed` in all three answers; `connector_sha256`; version floor in all
   modes and `1.0.0` in both `wrangler.toml` blocks. New integration cases. Tell the integrator: staging
   can carry 1.0 once PRD-06 is in.
3. Deletions: `sign.ts`, signer in `context.ts`, receipt, abandon, seal, quota, vitals validator and
   `Vitals` fields, local disk-pause twin, alias. `npm test` green after removing the matching cases.
4. `fetch-run.sh` rewrite and `test_fetch_run.py` (S13, by H+3); check `--local` against a `wrangler dev`
   run in a build box first, then a staging fetch from the conductor (fetch only, no check there).
5. `wrangler.toml` comments, `README.md`. Fold the still-true facts of `docs/SERVER-DESIGN.md` into
   `server/worker/README.md` and write one night-log line "SERVER-DESIGN facts are in the Worker
   README" for PRD-15. Third slice: `delete-test-runs.sh` for the usage buckets (edited, never run by
   an agent, section 9).

`npm test`, `test_fetch_run.py` and every `wrangler dev` run happen in a build box (ledger B1). The
only Worker step on the conductor is `wrangler deploy --env staging`, because the staging token never
enters a box; it deploys, it does not test.

Traps from the audit:

- `chunks.ack` is NOT NULL (`sql:214`): write `{}`, never null.
- Only a heartbeat with `counts` calls the 14-argument form; an object in `p_vitals` picks it
  (`db-supabase.ts:357-359`). The parity test in `supabase.test.ts` checks one overload per call.
- `p_vitals` is `{"counts": ...}`, never the flat object: the SQL reads `agent_paused` at the top level.
- `db-local.ts` must keep matching the SQL (appendix C coupling 1): seal with null keys, `vitals`
  stored as given, a repeated final heartbeat adds no row.
- `length > 0` (`sql:206`): the client never sends an empty chunk (PRD-01 section 1 guarantees it).
- Staging and production share one database: every organizer query filters `staging`.
- Do not touch `ratelimit.ts`: removing a Durable Object class needs a `deleted_classes` migration
  (appendix I section 4).
- Delete the signing secret only after the new Worker is live (section 12 step 5).
- A credential lives until deadline plus 24 hours (`db.ts:57`), so after that a late upload is `401`,
  not `410`.
- Never put the read-only database URL or R2 token in a pad or build box, a log or a command line.

How a watcher proves it in a real run: after each pad run, on the conductor, run
`sh scripts/fetch-run.sh --bucket goalcup-traces-staging <run> <out>`, then copy `<out>/<entry>` into
the run's pad box and, there, compare `sha256sum rec.gcr.zst` with `cat rec-*.gcr.zst | sha256sum`
from the kept local entry, read `complete`, `model_requests` and `last_heartbeat` in `fetch.json`, and
run `goalcup-verify check --server <copied entry> --local <entry dir>` (ledger B1: no build binary runs
on the conductor). For the heartbeat, read the `heartbeats` list in `fetch.json` and check each row's
`recording` and `counts` against the `status.json` snapshots the pad took at the same minutes.

## Asks of PRD-00 and PRD-01

All ten asks of revision 1 are closed by PRD-00 revision 2 and PRD-01 revision 2 (PRD-00 review answers,
"PRD-07 asks 1 to 7"; PRD-01 review answers, "PRD-07 asks 8 to 10"). Ask 2 was ruled the other way
(read-only credentials on the conductor) and ask 5 changed to 16 MiB; this revision follows both. The
two notes revision 2 left for PRD-00 are applied by PRD-00 R3-6:

1. S5's text "the final heartbeat repeated until the answer says `sealed: true`" adds "or a chunk or
   position answer": the completing chunk now seals too (section 3). Applied by PRD-00 R3-6.
2. S13's `fetch.json` also carries the last heartbeat's `counts` and each chunk's compressed size.
   Applied by PRD-00 R3-6.

Open asks of revision 3:

3. **PRD-06 and PRD-14:** section 6 and its builder notes say a bad `counts` is dropped with a
   `counts-dropped` log line and the heartbeat is kept (`200`), never `400`; its interfaces row gives the
   chunk answer as `{offset, next_seq, head, received_at, sealed}`. PRD-14's fake Worker follows the
   same. (PRD-06 revision 3 already says so; PRD-14's fake Worker is the open part.)
4. **PRD-08:** when `fetch.json` has `final` null, `server_chunks_whole` checks continuity only and
   reports the open end as incomplete (exit 1, never `fail`); `server_counts_match` is `skip` "not
   sealed". Acceptance 3 depends on it.
5. **PRD-16:** X6 reads `fetch.json` `heartbeats` instead of its own query, uses section 4's `recording`
   rule (`failed` only for null mode or a writer stalled past 60 s), and runs its checks in the run's
   pad box.

## Review answers

This section is history. The body, the ledger and the "Alignment answers" below win over it.

Sources: `prd-consistency.md` (CC), `prd-review-A.md` (RA), `prd-review-B.md` (RB). Code checked where
an item was disputed.

| Item | What | Answer |
|---|---|---|
| CC 67, RB C6 | Heartbeat body differs from PRD-06 | **Accepted.** S5 is the flat `counts`; `waiting_bytes` added; `repo_head` kept. PRD-06 follows (PRD-00 ruling) |
| CC 68 | Manual `goalcup upload` heartbeat without counts | **Accepted, with a server change.** The client always sends the seal's totals (PRD-00 ruling). The Worker treats the three facts as optional as today and **drops** a bad `counts` instead of a `400`: checked in code, `runs.ts:502-507` and D64 say vitals never cost a heartbeat, and a refused final heartbeat would block the seal and every later chunk |
| CC 69, RB C6 | `unknown` in the client table | **Accepted.** Dropped from the table; the Worker and SQL CHECK still accept it, no code change |
| CC 70 | Final heartbeat repeat rule | **Accepted and extended.** Repeat until `sealed`; the chunk that completes a committed end now seals too, so a client that stops after its last chunk still seals (section 3). Checked against `sql:1409-1448` (lock and recheck) and `chunks.ts:46-47` |
| CC 21, CC 81, RB C23 | Chunk cadence and the cost table | **Accepted.** 5 minutes or 16 MiB compressed; Class A row recomputed (about 3,000 per entry-week, about $16 for 1,500 runs) |
| CC 79, RB C9, CC 72 | Fetched layout differs from PRD-08 and PRD-12 | **Accepted.** `fetch.json` (S13) with the entry row, `model_requests`, final, the last heartbeat (`recording`, `counts`) and the chunk list with keys, SHA-256 and compressed size. Not `server.json`, to avoid the clash with PRD-10's local file |
| CC 80, RB C10 | Overnight deploy story contradicts the ledger | **Accepted.** Section 12 rewritten: agents deploy staging after builder step 2 and PRD-06; acceptance on staging; `--local` only before staging carries 1.0 |
| CC 82, CC 86 | `agent_paused` and receipt names in Worker code hit PRD-13's guard | **Accepted, done here.** `counts` is stored nested, so the Worker never names `agent_paused`; the local disk-pause twin, `Vitals`, `receipt`, `last_ack` types go. Only the SQL mirror keeps the names; PRD-13 skips `server/worker/sql/` |
| CC 19, CC ask 2 | Server access overnight | **Accepted.** Read-only database and R2 credentials on the conductor only (section 9); ask 2 of revision 1 withdrawn |
| CC 20, RB C22 | Judged-commit logic | **Accepted.** Untouched in 1.0 (PRD-00 question 4) |
| CC 22, RB N14 | Staging floor | **Accepted as PRD-00 ruled** (N14 rejected there): `1.0.0` in both blocks from the first staging deploy; production refuses 0.7 only from the owner's deploy after the release |
| CC 8 | PRD-00 row and ask 1 (signing scope) | **Closed.** PRD-00 row says signing and receipts deleted, the rest untouched |
| CC 99 | Test bucket | **Accepted.** The existing `goalcup-traces-staging`; no new bucket |
| RB N3 | Usage job and bucket | **Partly.** As PRD-00 ruled: the bucket and the `delete-test-runs.sh` step in the third slice, not after merge; agents create only the staging usage bucket |
| RB N11 | `stopped-runs.sql` | **Accepted.** After merge; design kept in section 8; `check-sql.sh` step and file creation removed from this build |
| RB W6 | Acceptance 5 needs a database edit | **Accepted.** Dropped; size is measured from A1 and A2, and a test shows no 413 path |
| RB W7 | Acceptance 6 on staging in the morning | **Accepted.** Dropped with N11 |
| RB T2 | Real-run proof of `failed` needs a hooks build | **Partly.** PRD-07's acceptance 3 removed as a duplicate of PRD-06 U6; the server half is an integration test. The partial-upload proof now reuses PRD-06 U8 (plain kills, no hooks, no firewall rule, also answers RB W5 for this PRD) |
| RB O7 | Staging waits on PRD-07's order | **Accepted.** The heartbeat, seal and floor work moved to builder step 2, before the deletions |
| RA finding 6 | Unlisted model host tunnels go unflagged; `wire_quiet` misses partly recorded runs | **Partly.** Noted as a known limit in section 8; the live client-side check belongs to PRD-02 and PRD-10; the query itself is after merge |
| RA finding 15 | Worker outage near the end loses the tail after 24 h | **Partly.** PRD-00 question 1 (unchanged window) with PRD-06's warning line; this PRD lists it in risks and section 11. The longer window needs a website migration, the owner's |
| CC 1 (asks not absorbed) | PRD-07's ten asks | **Closed.** All ruled in PRD-00 and PRD-01 revision 2; the asks section now says so |
| Relayed user request (gateways, "one system") | Is a gateway just another HTTPS host? | **No change here.** The Worker stores bytes and never sees hosts; gateway handling is PRD-03 and PRD-11 (ledger H4) |

## Alignment answers

Revision 3. Fix lists g1, g2 and g3, checked against the ledger, PRD-00 revision 4, PRD-02, PRD-06
revision 3, PRD-08, PRD-12, PRD-15, PRD-16 and the Worker code (`runs.ts`, `db.ts`, `db-local.ts`).

| Fix id | What was done |
|---|---|
| L6-05 | Applied the PRD-07 part: "How a watcher proves it" copies `<out>/<entry>` into the pad box and runs `goalcup-verify check --server ... --local ...` there. The acceptance preamble says the same (ledger B1) |
| L2-08 | Partly, merged with G2-31 and G3-X8. "A recovered entry is `gaps`" applied. "And only then" kept, but after G2-31's writer stall: `failed` is null mode or a writer stalled past 60 s, as PRD-06 revision 3 section 6 now says |
| L4-6 | Applied: section 12 step 5 creates `goalcup-usage` (the usage job is in this build, third slice) |
| L7-09 | Partly: the owner note is in section 12 step 3. The id "OW-12" is not defined in any file yet, so the note points to PRD-16's `MORNING.md` owner steps instead; that line is PRD-16's part |
| G2-31 | Applied the PRD-07 part: the `failed` row reads "null mode, or a writer stalled past 60 s"; a recovered entry is `gaps`; `sealed` added to the `mode` list (the final heartbeat sends `mode: sealed`) |
| 06-23 | Applied: `goalcup upload` sends `pipeline::entry_totals` (R3-2) in section 4 and the PRD-06 interfaces row |
| X1-7 | Applied: cost table row "200 agents at once, a week: about 400 GB, about $6 a month" |
| X2-11 | Partly. Moving the review sections into a new `REVIEW-HISTORY.md` is not done here: this pass edits only this file, and many writers of one new file would race (as PRD-00 revision 4 answered). "Review answers" now says it is history and that the body, the ledger and these answers win. The two revision 2 notes are marked applied by R3-6. The move may run once, after every alignment pass, by one agent |
| G3-X8 | Partly. The table now follows PRD-06 section 6 (`ok` with `recording` or `sealed` and all loss counts 0; `gaps` for any loss, `losing`, `writer_slow` or a recovered entry). `mode` list gains `sealed`. CC 69's "entry ended while off" removed. Rejected two parts: "`failed` only for null mode" (G2-31 and PRD-06 revision 3 add the 60 s writer stall), and `mode` as the string `"null"` (PRD-06 and its test use JSON null) |
| G3-X22 | Applied: section 4 says the Worker fixes no key list inside `counts`; ask 3 added (PRD-06 already says so in revision 3; PRD-14's fake Worker is the open part) |
| G3-X23 | Applied, with JSON null instead of `"null"` (see G3-X8). The note names PRD-06 `upload_worker.rs` case 2 as the test |
| G3-X1 | Applied: "Where it runs" split (staging fetch on the conductor; `--local` in the box with `wrangler dev`; every `goalcup-verify` in a box); builder notes say `npm test`, `test_fetch_run.py` and `wrangler dev` run in a build box and only `wrangler deploy --env staging` runs on the conductor; step 4 and the PRD-12 row follow |
| G3-X26 | Applied: index query and `fetch.json` gain `heartbeats` (every row, time order); `test_fetch_run.py` checks it; sections 4, 8 and 11, risks, the watcher text and the PRD-16 row read it. Partly: the size line says "under 5 MB" (2,100 rows at the 2 KiB `counts` bound), not "under 3 MB". The PRD-06 U6 and PRD-16 X6 parts are theirs (ask 5) |
| G3-X24 | Applied: ask 4 to PRD-08; acceptance 3 unchanged |
| G3-X56 | Applied: acceptance 2 rewritten as given |
| G3-07a | Applied: a dropped `counts` stores `{"counts_dropped": n}` in `p_vitals` with the log line; integration test, risk row, default row and `fetch.json` `heartbeats` show it. No `agent_paused` key, so the old disk-pause rule is not reached |
| G3-07b | Applied: `--local` kept; the index is the R2 object `state/runs/<run>.json` in the local persist store with `db-local.ts`'s shape (checked at `db-local.ts:1-12`) |
| G3-07c | Applied: open and continue bodies and the `403` text moved to the PRD-10 row (PRD-10 opens the run, its section 1 step 5) |
| G3-07d | Applied: builder step 5 folds SERVER-DESIGN's still-true facts into `server/worker/README.md` with the night-log line; PRD-15 row says so |
| G3-07e | Applied: header revision 3 against PRD-00 revision 4 (the fix said revision 3; PRD-00 is now at 4). Question numbers mapped to revision 4: judged-commit 3 to 4, cadence 6 to 1, tail window 2 to 1, interfaces "2, 3, 5" to "1, 4, 5". The two notes say "Applied by PRD-00 R3-6" |
| G3-07f | Applied: section 6 gives the mode-neutral `403` text (today's is "too old for the cup", `runs.ts:163`) |
| G3-07g | Applied: a missing or bad object ends the entry with `"why":"object_bad"` and exit 1; `test_fetch_run.py` case added |
| G3-07h | Applied in section 7 and the recovery-fallback risk row |
| G3-07i | Applied in sections 9 and 10 and builder step 5: `delete-test-runs.sh` is the owner's tool; agents edit it, never run it |
| G3-07j | Applied: `Vitals` is `db.ts:242-249` (checked in code) in the Today table, section 4 and What is cut; `waiting_bytes` cites PRD-06 section 11 |

## Amendments

### 2026-10-30, replan after PRD-07 slice 1 (merge 4dcc1e7b)

Slice 1 merged builder steps 1, 2 and 4. What the merge changed for the slices still to come:

- **Step 3 is smaller.** The fixer already did part of it: the per-run size limit is gone (no 413 path,
  ledger R5), the `Vitals` type, the local disk-pause twin and the dead seal check are gone, and
  `test/quota.test.ts` is deleted. Still to delete: `src/sign.ts`, `test/sign.test.ts`,
  `test/vectors/signing.json`, the signer in `context.ts`, the `receipt` and `abandon` routes
  (`index.ts:43-46`, `runs.ts` `issueReceipt` and `abandonRun`) and their store calls, `ack` in the
  chunk answer, `last_ack` in the position answer, the `tracer_version` alias (`runs.ts:145`, `:195`),
  `RECEIPT_SIGNING_KEY` from `env.ts` and the `wrangler.toml` secret list (only after the new Worker
  is live, section 12 step 5), and the "sign the ack" line in the `chunks.ts` header. Step 3 adds the
  `404 not_found` acceptance cases for `/receipt` and `/abandon` (the `/seal` 404 and "no chunk
  refused for size" are already on integration).
- **Until step 3 lands, `docs/UPLOAD-API.md` is ahead of the code** (review A3): its line "Gone since
  0.7, and not coming back" is not yet true for `ack`, `last_ack`, receipt, abandon and the alias. A
  client built to S5 ignores the extra fields, so nothing breaks. Step 3 makes the line true; no doc
  edit is needed.
- **Review nits carried into step 3:** B1, in `db-local.ts`: the `StoredRun` comment (about line 97)
  and the `withSignals` docstring (about line 593) still describe the disk pause, and line 608 writes
  `disk_paused_at: null` for nothing; write `...(a.vitals === null ? {} : { last_vitals: a.vitals })`
  and fix both comments. B2: drop the integration case "counts with an agent_paused key start no pause"
  (`integration.test.ts` about line 807), or cut it to the `last_vitals` nesting; `local-db.test.ts`
  (about line 325) and `supabase.test.ts:305-315` stay as the one local and one production guard.
  `integration.test.ts:185` ("once a receipt exists, repeating the open call ...") needs a receipt
  and goes with the receipt route.
- **Review nit N1 for step 4 or 5:** the staging index query keeps to one side by the bucket name
  (`ev.staging = :'staging'`). A staging run fetched without `--bucket goalcup-traces-staging` stops
  with "the index holds no entry for <id>". Name the side in that message (and the `--bucket` flag to
  try). The `--local` check against `wrangler dev` was not re-run after the fixer's trim; rerun it in a
  box with that change.
- **`--local` keeps only each entry's last heartbeat**, since the local store holds `last_vitals` only.
  A full `heartbeats` list is a staging fetch fact. PRD-06's `upload_worker.rs` and PRD-14's full tier
  read one heartbeat row per entry under `--local`.
- **Answer to PRD-17's question (lessons log, 10:45Z)** on the Worker test for "a repeated open answers
  the same run" (S10): `test/device.test.ts:156` ("repeat open with the same session_id returns the
  same run; another session_id is 409") and `test/lifecycle.test.ts:222` ("a lost-reply repeat of the
  open is the same run, also after continue"). Both need no receipt and stay through step 3.
- **Size on integration** (wc -l at 4dcc1e7b, all of `server/worker/src`, the out-of-scope files
  included): src 3,848 against the row's about 3,440 (11.9% over), tests 5,568 against about 4,950
  (12.5% over). Step 3 removes `sign.ts` (116), the receipt, abandon and alias code, `sign.test.ts`
  (68) and `signing.json` (26); report again after it. The conductor's 11:20Z decision makes overage
  alone a nit; the replan reports the counts.

### 2026-10-30, replan after PRD-17 upkeep slice 2 (merge ae970618)

- **Five Worker cases guard lessons** (`LESSONS.md` L-S1, L-S2, L-S3, L-S10): the chunk retry, the
  `Goalcup-Sha256` mismatch and the past-deadline 503 in `integration.test.ts`, the repeat open in
  `device.test.ts` and in `lifecycle.test.ts`. A slice that renames one writes the new title in the
  lessons log, as the 11:47Z seam line did for the chunk retry; a slice that removes one names the
  case that now carries the lesson. No other step changes.
