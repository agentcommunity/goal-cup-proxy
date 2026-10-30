# PRD-06: Uploader and compression

Id: PRD-06. Version: goalcup 1.0. Revision 3, 30 October 2026 (the alignment pass against the ledger;
revision 2 and revision 1 earlier the same day). Base: main at 4b226de2. Index: [PRD-00](PRD-00-index.md)
revision 4. Ledger: [OWNER-DECISIONS.md](OWNER-DECISIONS.md) wins over this file. Format:
[PRD-01](PRD-01-record-format.md) revision 2 (binding). Upload contract: [PRD-07](PRD-07-worker-server.md)
seam S5 (binding). Revision 2 took PRD-00's rulings on this PRD's asks and answered the three reviews
(section "Review answers"). Revision 3 applies the alignment fix lists (section "Alignment answers" at
the end).

## Goal

Send every compressed part file to the server in small chunks, soon after it is written, through any
network trouble, without ever slowing the harness. Tell the server and the participant only written
counts, off time and a plain recording state, and free local disk only once the server holds the bytes.

## Points served

5 (lead: upload everything, so the server can rebuild the story), 3 (the upload never waits on the
harness and the harness never waits on the upload), 6 (the participant sees plainly what reached the
server and what waits), 1 and 2 (no cap of ours on bytes, chunks or entries).

## Owner decisions it carries out

- F2 (locked): zstd with a long window (`--long=27`): run 1's server copy was 4 times smaller
  (100.8 MB against 408.9 MB, run report "Size and compression").
- U1 (locked): prune a part once the server confirmed it stored it.
- R5 (locked): no caps of our own; the per-run size limit goes. Only stated memory guards.
- U1 (locked): upload often, in small chunks, with a backoff that is capped short and resets when the
  server's offset moves.
- R6 and R7 (locked): the participant, status, heartbeat and server see only written counts and off
  time; the heartbeat says `failed` when recording is off. Email work is out of scope; the server can
  mail later.
- L5 (locked): one process. The uploader lives in the recorder process, on two plain threads (PRD-00
  "Smaller interface rulings"), and never ends that process.
- Keep every part on test runs: `GOALCUP_KEEP_PARTS=1` on every pad and acceptance run (PRD-00
  defaults "Kept parts", done item 1).
- Chunk cadence: every 5 minutes, or at once when 16 MiB compressed waits (PRD-00 defaults, PRD-00
  owner question 1, old 6).
- B4 (locked): every build and acceptance run uses the **staging** Worker with the test approval
  route; nothing here touches production.
- The tail window stays 24 hours in 1.0, and the participant is told the time once (PRD-00 owner
  question 1, old 2).

## Today

| What | Where | Fact |
|---|---|---|
| The upload module | `crates/goalcup-cli/src/upload/` | 4,743 lines: 3,931 src and 812 inline test lines (counts below) |
| Second compression pass | `uploader/chunk.rs:102-227` | Re-reads chain lines, checks each link, compresses into a spool file at level 3 with no long window (`chunk.rs:17-19`, `:144`) |
| Chunk cap | `uploader/chunk.rs:12-16`; `server/worker/src/chunks.ts:22` | 32 MiB compressed; chunks held 2.8 MB to 264 MB of chain each (appendix M, C6) |
| Cadence | `upload/policy.rs:77-91` | A chunk every 30 min, or once 256 MiB of chain waits; first interval random (`policy.rs:183-197`) |
| Backoff | `upload/policy.rs:283-291` | 60 s doubling to 15 min, plus 0 to 25% jitter, never sooner than `Retry-After` |
| The lag bug | `uploader/lifecycle.rs:513-533` | `failures` resets only on `CaughtUp`, so while the agent outpaces the upload the wait keeps doubling across stored chunks: run 1 waited 61, 130, 269, 497, 987, 975, 1,083 and 947 s (appendix M 4.7, `M-two-hour-run.md:462-486`). At t+61 min only 7% had reached the server |
| Signed acks | `uploader/session.rs:449-496`, `acks.jsonl` | Every ack is checked under the receipt key and appended; prune trusts only signed acks (`session.rs:196-220`) |
| Heartbeat | `uploader/heartbeat.rs:36-70`; `api.rs:187-235` | Every 300 s, 30 s timeout (`heartbeat.rs:74`); `recording` and `model_requests` counted at selection, plus disk and process vitals |
| Prune | `upload/prune.rs:31`, `:47-81` | A part 2 h after a signed ack, or at once under disk pressure; at the end every part but the last of a sealed, sent entry (`commands/start/finish.rs:288-290`). Run 1 lost its local evidence this way (appendix M 4.9) |
| HTTP client | `upload/http.rs:96-237` | `ureq` 2, own rustls roots, no proxy from env (`http.rs:167`), no redirects, one connection per call (`:155-158`), PUT timeout 600 s (`policy.rs:14`) |
| Leftovers | `src/main.rs:349-383`, `commands/upload.rs:22, 52` | Every participant command first sends other runs' unsent parts for 3 s |

Run 1 facts (run report measure 5 and 7): 20 chunks, 1,304,968,017 bytes of chain, 11 network
errors (os errors 11, 104, 32, one EOF), all stored by 3 s after the seal. `goalcup start` (proxy and
uploader) used 26 to 32 MB RSS; the daemon 50 to 52 MB.

## Design

### 1. The shape

```
writer thread (PRD-02) --PartWriter--> rec-*.gcr.zst + rec-*.idx      (compressed at write, PRD-01)
            |                                  ^
            +-- Durable on a watch (S4) -------+---- upload thread: read idx, cut chunk, PUT
                                               |
                                               +---- beat thread: heartbeat, final heartbeat,
                                                     ServerWord (stop, reason, seconds_left) out
                                                     on a watch (S10) -> recorder -> watchdog
upload thread --> upload.json (S9: state, server_word, for PRD-10 and the pad)
              --> prune: delete closed parts the server holds
```

- **The uploader never compresses and never decodes.** The writer already compresses (PRD-01
  section 12). A chunk is a byte range of a part file between two index lines. The old second pass,
  its spool file and its line parser go. It links `goalcup-format` only for `IndexLine` and
  `read_index`.
- **Two plain threads**, `upload` and `beat`, started by `run_recorder` after boot recovery (PRD-02
  section 9.2 step 5) and lowered with `pipeline::low_priority()` (PRD-02; no macOS QoS class in
  1.0). They use the blocking `ureq` client copied from `http.rs`. They never touch the relay
  runtime, so a hung server cannot reach a relay task. They copy `Durable` out with `*rx.borrow()`
  (or `borrow().clone()`) on their 1 s tick (PRD-02 S4) and drop the borrow at once; no `Ref` is held
  across a read, a hash or a call, so the writer never waits on them. Settled by PRD-00; no
  `spawn_blocking` variant.
- **The uploader never ends the process.** Each upload round and each beat runs inside
  `catch_unwind`. A panic there writes one fixed-text line through `cli::log::line` (S11), sets
  `upload.json` `state` `waiting` with class `internal`, backs off by section 5 and never ends the
  process. Recording never depends on the uploader (PRD-02 9.2 step 1 lists no uploader exit).
- **One queue of entries.** The upload thread serves every entry on this computer whose
  `upload.json` is not `done` or `closed`, whose run's credential is still there and that is not
  being sent by another uploader (section 9). Each round serves the live entry first, then one round
  for each older entry, oldest first. An entry that is `refused`, `stuck` or waiting on backoff never
  holds the queue: the thread moves on and comes back when its `next_try` is due. An entry whose
  `upload_by` has passed is dropped from the queue and its `upload.json` says `closed`. The upload
  thread (not the beat thread) sends the final heartbeat of every non-live entry. The queue covers a
  dead entry closed by `close_dead_entry` (PRD-02 section 9.4) and an earlier run whose wrap-up ran
  out.
- **In null mode** (PRD-02 section 8 step 9) the null recorder still runs both threads: what is on
  disk is sent (with the `fdatasync` rule of section 3), and the heartbeat says `failed`.
- **No harness-specific code.** Direct key, gateway or dodger: the record is the same format, so the
  upload is the same.

### 2. Compression and its memory cost on the client

Settings (fixed by PRD-01 section 12, confirmed here): zstd level 3, `window_log` 27, long distance
matching on, content checksum off, one frame per part, flushed at every LINK. The `zstd` 0.13.3 crate
(the locked version, `Cargo.lock:2041-2042`) exposes `Encoder::window_log`,
`Encoder::long_distance_matching` and `Encoder::include_checksum` with no crate feature
([docs.rs](https://docs.rs/zstd/0.13.3/zstd/stream/write/struct.Encoder.html)); only `multithread`
needs the `zstdmt` feature, and we do not use it.

**Decision: window 27 on every machine, no small-machine fallback in 1.0.** One setting means every
reader needs the same 128 MiB and no code decides by RAM. Window 27 with long matching is locked
(ledger F2); a smaller window needs the owner.

Memory, measured with Python `zstandard` 0.25 streaming 300 MiB (appendix M 4.5,
`M-two-hour-run.md:424-425`), unverified in Rust until built:

| Item | Client memory | Note |
|---|---|---|
| Compressor, level 3, no long window (today's uploader) | about 7 MiB | Only while a chunk was built |
| **Compressor, `window_log` 27 with long matching (1.0)** | **about 141 MiB** | For the whole run, in the writer. The window fills as data flows: an idle run grows toward it slowly |
| Compressor at level 9, `window_log` 27 | about 159 MiB | Not used |
| Uploader | under 1 MiB | No spool, no body in memory: it hashes the range in one read pass and streams it in a second, with a 64 KiB buffer each |
| Decompression on the client | 0 | The client never decodes. 128 MiB is paid by readers only: `fetch-run.sh` (PRD-07), `goalcup-verify` (PRD-08), the usage reader (PRD-09) |

What this adds up to: run 1's two processes used about 80 MB together (run report measure 7). The 1.0
recorder adds about 141 MiB of compressor, so about 170 to 200 MiB at rest. Under load PRD-02's
memory guard holds up to its limit on top (PRD-02 Risks: about 750 MiB peak). The compressor is part
of the steady cost, not of the guard.

- **Rotation must not double it.** Closing a part and opening the next must drop the old encoder
  before the new one allocates, or the writer briefly holds about 282 MiB. PRD-01 revision 2
  section 1 fixes that order.
- **Sandbox fit.** A Vercel Sandbox gives 2 GB of memory per vCPU
  ([Vercel Sandbox pricing and limits](https://vercel.com/docs/vercel-sandbox/pricing)). A 1-vCPU
  sandbox with Codex at 320 to 385 MB (run 1) and a recorder at 200 to 750 MiB fits; the pad uses at
  least 2 vCPU.
- **Throughput.** Long matching makes compression slower than plain level 3; the rate on the target
  machines is unmeasured. PRD-02 R8 measures it and reports it; it is never a pass line (PRD-00:
  no speed number fails a gate). If R8 reports under 50 MB/s and names the compressor as the slow
  stage, the first change is level 1 with the same window and long matching, because the long
  window is where the gain is. A level change needs that measured reason in the night log (PRD-01
  overridable defaults); the window and long matching never change without the owner (ledger F2).

### 3. Chunks from parts

Inputs: the part files and their `.idx` lines (PRD-01 section 1), the **durable point** from PRD-02's
`watch` channel (seam S4: part path, `c_end`, `raw_end`, link `seq`, hash), and the server's position
(`offset`, `next_seq`, `head`) from `GET /v1/runs/{run}?entry_id=`.

**Where a chunk starts.** The server's `offset` S is a raw offset (PRD-01 section 12). Find it:

1. S equals a part's name (its first raw offset): start at that part, compressed byte 0.
2. Else S equals the `raw_end` of an `L` line: start in that part at that line's `c_end`.
3. Else S equals the `raw_end` of an `E` or `R` line: start at byte 0 of the next part.
4. Else S is nowhere in the local index: the copies disagree (section 5, `stuck`).

Case 1 is checked first. It also covers PRD-01's recovery fallback (section 12, "Fallback if step 2
fails"), where a crashed part file ends at an `L` line and the next part file starts at that line's
`raw_end`.

**Where it ends.** Take the following index lines of the same part, in order, while the range stays
at most 32 MiB (the Worker's cap, `chunks.ts:22`) and the line is at or before the durable point. The
chunk ends at the last line taken. It never crosses a part. A flush interval is at most 16 MiB raw
(PRD-01 section 10), so the first line always fits; if it does not, that is a writer bug: the
uploader sets `stuck` with the line and says so, never sends a chunk the Worker must refuse.

**Headers** (meaning unchanged from today, `session.rs:427-447`):

| Header | Value |
|---|---|
| path offset | S |
| `Content-Length` | `c_end(last) - c_start` |
| `Goalcup-Length` | `raw_end(last) - S`, always above 0 (PRD-01 never writes an `L` line for the last link of a closed part) |
| `Goalcup-First-Seq` | the server's `next_seq` |
| `Goalcup-Last-Seq`, `Goalcup-Head` | the last line's link `seq` and hash (hex) |
| `Goalcup-Prev` | the server's `head`; 64 zeros at S = 0, which is the Worker's `GENESIS` (`server/worker/src/db.ts:67`) and PRD-01's `hash(-1)` |
| `Goalcup-Sha256` | SHA-256 of the body, from the first read pass |
| `Content-Type` | `application/zstd` |

Before sending, the uploader checks that the first line's `seq` equals `next_seq` and that the link
hash at S (from the index, or from `upload.json` when S is a pruned boundary) equals the server's
`head`. A mismatch is `stuck`, not an upload.

**Durable only.** In the live recorder the upper bound is the durable point. When no live writer
publishes one (null mode, `goalcup upload`, a dead entry), the bound is the last whole index line
whose `c_end` is within the file, and the uploader itself calls `fdatasync` on the part and then on
the index before it sends that range. Without that, a power loss could leave the server holding
bytes that recovery later truncates away (PRD-01 section 12 step 1), and the two copies would split
for good.

### 4. Cadence

| Trigger | What happens |
|---|---|
| First durable point of an entry (about 5 s after start) | Send at once, so the server sees the run begin and the pad can check early |
| 5 minutes since the last round began | A round. The first interval after the opening chunk is a random share of 5 minutes, so runs started together do not send together (kept from `policy.rs:183-197`) |
| 16 MiB compressed waits past the server's offset | A round at once |
| The seal is durable | Rounds back to back until done (section 9) |

A **round** sends chunks one after another until the server reaches the durable point the round began
with. Rounds never overlap. Nothing waits on the harness and the harness waits on nothing.

What this costs and buys:

- **Lag.** The server is at most about 5 minutes plus one round behind while the network works.
  Run 1's compressed rate was about 83 MB an hour at 10 to 1 (PRD-01 section 14), about 7 MB per
  5 minutes, so a round is usually one chunk.
- **Rows.** A run that writes all week sends at most 2,016 time-driven chunks per entry-week, plus
  one per 16 MiB in bursts (10 to 20 GB compressed a busy week is about 600 to 1,200 more). That is
  up to about 3,000 `chunks` rows and R2 objects per busy entry-week. An idle hour sends nothing.
  R2 Class A is $4.50 per million after 1 million free a month
  ([R2 pricing](https://developers.cloudflare.com/r2/pricing/)).
- **Bytes resent after an error:** at most one chunk (32 MiB). The Worker answers a resent stored
  chunk without reading its body (`chunks.ts:44-47`).

### 5. Retries and backoff

One rule for every failed call of a background upload, chunk or position read:

```
wait = min(120 s, 2 s x 2^(failures - 1)) x random(0.5 .. 1.0)
wait = max(wait, min(Retry-After, 120 s))
failures = 0 whenever the server's offset moved forward (a stored chunk, or a position read past
             the last known offset), and whenever the entry is caught up
```

So the nominal waits are 2, 4, 8 ... 120 s, each times a random 0.5 to 1.0, and never more than
120 s, and a flaky network that still makes progress never escalates. That fixes the run 1 bug (`lifecycle.rs:513-533`). The clock is the
recorder's `now()` (boot time, counts sleep), checked on a 1 s tick, so a wait is never stretched by
a suspend.

| Answer | Class | Action |
|---|---|---|
| No answer, reset, timeout (connect 15 s, `http.rs:15`; no byte moved for 60 s; whole PUT 600 s) | `network` | Backoff |
| 429, 5xx, 503 maintenance | `server_busy` | Backoff |
| 503 `awaiting_final` | `server_busy` | Wait until `upload.json` `final` is `sent` (the beat thread had the final heartbeat committed, section 6), then retry at once. The Worker's `Retry-After: 60` (`FINAL_RETRY_AFTER`, `server/worker/src/runs.ts:440`) is not waited out once the final is committed |
| 409 | none | Read the position again at once and go on from there (kept from `session.rs:349-353`). A 409 whose re-read shows no forward move counts as a failure for the backoff |
| 400, 413, 422, 401, 404, 410 | `refused` | Backoff, for ever, while the process lives: a bad deploy is fixed on the server (D64 holds). Said once after 3 in a row, kept in `upload.json` until the server answers normally |
| The two copies disagree (section 3, step 4 or a hash mismatch) | `stuck` | Read the position again every 2 minutes; never send across the disagreement; say so plainly |

Only `goalcup upload` by hand gives up, on 401, 404, 410 and `stuck`, and says why (kept from
`policy.rs:248-265`). A 422 (SHA-256 refused) re-hashes the range before the retry. A panic inside a
round or a beat (section 1) is class `internal` and backs off like `network`.

Every failed call writes one `goalcup.log` line through `cli::log::line` (S11):
`upload: <class> <status or -> wait <s>s` (no URL, header or body). The watcher reads the waits from
these lines (U3, U4).

### 6. Heartbeat

The beat thread sends `POST /v1/runs/{run}/heartbeat` for the live entry every 5 minutes, and **at
once** when the recording state changes. A failed beat is retried with section 5's backoff, its own
counter. The body is PRD-07's seam S5 (PRD-07 section 4 owns it; this PRD fills it):

```json
{"entry_id":"e0123456789abcdef","seq":3601,"head":"<64 hex>","offset_local":726000000,"final":false,
 "repo_head":null,"recording":"gaps","model_requests":905,
 "counts":{"v":1,"boot":2,"mode":"recording","off_ms":2300,"off_spans":1,"off_since":null,
           "gaps":3,"lost_bytes":1500000,"lost_unknown":0,"http_post":243,"ws_up_msgs":662,
           "conns":120,"tunnels":118,"wire_bytes":674000000,"native_files":43,"native_bytes":48200000,
           "waiting_bytes":1048576}}
```

- `seq`, `head`, `offset_local`: the durable point's link and `raw_end`. Before the first durable
  point there is nothing to say and no beat is sent, so `recording: "unknown"` is never sent.
- `model_requests`: model requests written, `http_post + ws_up_msgs`, entry totals over boots (PRD-01
  section 9, PRD-02 section 6). Never a selected or offered count.
- `counts`: one **flat** object, at most 2 KiB. PRD-07 drops anything else with a `counts-dropped`
  log line and records the heartbeat without it (`200`, never `400`, PRD-07 section 4), so the loss
  is silent; `upload_beat.rs` guards the size. The keys, named here (PRD-07 fixes no key list;
  `docs/UPLOAD-API.md` copies these): every whole-number COUNT counter of PRD-01 section 9 as entry
  totals at the durable LINK (maps such as `hosts` are left out, so it stays flat), plus `v`, `boot`,
  `mode`, `off_ms` and `off_spans` (the sum of `recorder_down` spans, plus the open span while in
  null mode), `off_since` (the null span's start, else null) and `waiting_bytes` (section 11).
- **`mode`.** A live recorder computes it from the `Durable` snapshot's age and PRD-02's live atomics
  (`written`, `losing`, `held_bytes`, `writer_idle_ms`), not only from `status.json`. A null
  recorder, `goalcup upload` and a dead entry read it from `status.json`. The final heartbeat sends
  `mode: sealed`.
- **Where the numbers come from.** A live recorder: the `Durable` snapshot (PRD-02 S4: counters at
  the durable LINK, entry totals). A null recorder, `goalcup upload` and a dead entry: PRD-02's
  entry totals read from `counts.jsonl` up to the last link on disk (`pipeline::entry_totals`, ask
  P2) and `mode` and `off.since` from `status.json`. For a sealed or recovered entry these equal the
  seal's totals by PRD-02's construction ("sealed `written` equals seal totals"), so the uploader
  sends the seal's totals without decoding a part file. `goalcup upload` therefore sends `counts` and
  `recording` like any heartbeat (PRD-00 ruling: `counts` stays required).
- `repo_head`: null on every heartbeat but the final one, which carries the seal's
  `workspace_head` (40 hex, or null; `server/worker/src/runs.ts:452-472` keeps it only when `final`).
  The live recorder hands it over with `UploadHandle::finish`; `goalcup upload` reads it from
  `status.json` (ask P1).
- `recording`, decided from `mode` and the counts:

| State | When |
|---|---|
| `ok` | `mode` `recording` (or `sealed`), and `gaps`, `lost_bytes`, `lost_unknown`, `off_ms` and `streams_in_gap` are all 0 |
| `gaps` | Recording now (or sealed), and any of those is above 0, or `mode` is `losing` or `writer_slow` (records wait unwritten for 10 to 60 s). A recovered entry is `gaps`: its `abandoned` span is off time |
| `failed` | Recording is off: `mode` null, or `writer_idle_ms` over 60 s while records wait (PRD-02's stall exit follows and the next boot writes a `recorder_down` gap with cause `writer_stalled`). A state, not a latch: the next beat after a good boot says `gaps` |

- The Worker stores `counts` in the existing `heartbeats.vitals` jsonb column
  (`server/worker/sql/goal-cup-tracer-upload.sql:2552`), so no website migration (ledger S3; PRD-00
  owner question 1).
  Disk and process vitals (`api.rs:214-235`, `src/vitals.rs`) are gone.
- **The answer** (`stop`, `reason`, `deadline_at`, `seconds_left`, `sealed`; PRD-07 section 3) is
  published as `ServerWord {stop, reason, seconds_left, asked_at}` on a `watch` channel (seam S10),
  with `asked_at` taken before the call (kept from `heartbeat.rs:119-121`), and written into
  `upload.json` as `server_word` (seam S9) for `goalcup time-left`. The recorder forwards it to the
  watchdog as the `server` control line (PRD-02 section 8); the watchdog resets its deadline timer
  from `seconds_left`, and on `stop` ends the harness and seals with reason `server_stop`, the
  server's reason in `server_reason` (PRD-02 sections 8 and 11). The old-server fallbacks
  (`LocalDeadline::of`, `until`, `heartbeat.rs:249-290`) go: no backward compatibility.
- **Final heartbeat.** Once the seal is durable (PRD-02 section 11 step 5), the beat thread sends
  `final: true` with the seal LINK's `seq`, hash and `raw_end` (the last `E` or `R` index line) and
  `repo_head`. It goes before the rest of the chunks, because past the deadline the Worker refuses
  chunks until the end is committed (`chunks.ts:52-60`). After the last chunk is stored, if its
  answer did not say `sealed: true`, it sends the final heartbeat again, and repeats it with backoff
  until the answer says `sealed: true` (PRD-07
  section 3; a repeat is `final_existing` and costs nothing) or the wrap-up ends. Once a final
  heartbeat was sent, no ordinary heartbeat follows for that entry (PRD-17 S4). For a dead entry
  closed by `close_dead_entry`, the final heartbeat names its `recovered` seal the same way.
- **No seal call.** PRD-07 cuts `POST .../seal`. `seal.json`, `baseline.json` and the tunnel list are
  records now (PRD-01 section 11). The entry is complete when a heartbeat, chunk or position answer
  says `sealed: true`.
- **A replaced run.** When a heartbeat answer says `stop` with reason `replaced`, `upload_by`
  (section 9) becomes the answer's time plus 24 hours, because the Worker closes a replaced run
  24 hours after `replaced_at` (`server/worker/src/db.ts:23-26, 38-40`).

### 7. Prune, and keep on test runs

A part file and its `.idx` are deleted when all of these hold:

1. The part is closed: its last index line is `E` or `R`, or a later part file of the same entry
   exists (a crashed part under PRD-01's recovery fallback ends at an `L` line).
2. The confirmed server offset is at or past that line's `raw_end`. Confirmed means one of: a 200
   answer to our own PUT whose `offset` and `head` are the chunk's end; or a position read whose
   `offset`, `next_seq - 1` and `head` match a local index line or `upload.json`'s `confirmed`.
   A number the server merely reports is never enough (kept from `session.rs:196-220`).
3. Every older part of the entry is already deleted (prefix only, so the rest stays contiguous).
4. `keep` is false.

There is no 2-hour timer and no disk-pressure branch (`prune.rs:31`, `:47-81`): owner answer 9 and
"disk full: no special handling". `upload.json` records the confirmed position **before** a delete,
so a later boot can still check the server's `head` at a pruned boundary. The open part is never
deleted. Prune deletes part files and their `.idx` files and nothing else. After a sealed entry is
fully stored, its folder still holds every other local file of PRD-00's list: `upload.json`,
`upload.lock`, `status.json`, `counts.jsonl`, `cursors.json` (`continue` needs it), `goalcup.log`,
`run-ca.pem` (certificate only), `bundle.pem`, `rec/lock` and `rec/watchdog.pid`. The run's
`server.json` lives one folder up (`$GOALCUP_HOME/runs/<run>/`, PRD-10) and is never pruned.

**`keep` is true** (nothing is ever pruned) when:

- `goalcup start` or `continue` ran with `GOALCUP_KEEP_PARTS=1` in its environment. The pad (PRD-12)
  and the 2-hour run (PRD-16) set it. PRD-10 passes it in the start facts (no flag).
- The run has no server (local only): nothing can be confirmed.

`keep` is written into `upload.json`, so the verifier and the pad know why parts are there. A
participant who sets it only keeps more on their own disk; that is harmless.

### 8. Resume after a crash, a sleep or a reboot

| Event | What the uploader does |
|---|---|
| Recorder killed mid-PUT | The next boot starts the uploader after recovery (PRD-02 9.2 step 5). It reads the position. If the lost PUT was committed, the offset moved and it goes on; if not, the chunk is cut again from the same lines and resent. The Worker answers a repeat with the stored result (`chunks.ts:44-47`). No spool to clean (today's `session.rs:138-140`) |
| Crash between index line and upload | Nothing special: lines at or before the durable point survive recovery by construction (PRD-01 section 1) |
| Null mode | Keeps sending what is on disk with the `fdatasync` rule of section 3; heartbeat `failed` |
| Laptop sleep | Sockets die; the next call fails and backs off at most 2 minutes. Timers run on boot time, so on waking the 5-minute round and the heartbeat are already due and go at once |
| Machine reboot, or both processes killed | Nothing runs until the participant acts. If the harness still runs, `goalcup recover` restarts only the recorder on the same port with the same CA, and its uploader goes on from the server's position (PRD-02 8.1, ledger R3b). Otherwise `goalcup continue` closes the dead entry (PRD-02 9.4) and its recorder's uploader sends the old entry first, then the new one. `goalcup upload` does the same by hand |
| `upload.json` lost or corrupt | It is a cache. The server's position is the truth; it is checked against the local index as in section 3 |
| Server lost bytes the client pruned | `stuck`: said plainly, never "fixed" by sending around the hole. Organizers decide (Risks) |

### 9. The finish and `goalcup upload`

- **After the seal** the recorder process stays alive for the wrap-up: final heartbeat, then rounds,
  then the final heartbeat again until `sealed`, for at most 60 minutes counted on boot time (kept
  from `policy.rs:86`). PRD-10 shows progress from `upload.json` once a second. Ctrl-C ends the
  watchdog's wait only; the recorder finishes the wrap-up alone (PRD-02 section 8 step 11). Whatever
  is left after 60 minutes waits on disk. The wrap-up never holds the harness: PRD-02 bounds the seal
  itself (PRD-00 S8), and the wrap-up starts after it.
- **The tail window.** The run credential and the server's intake end 24 hours after the run's
  deadline (`server/worker/src/db.ts:17`, `:57`). `upload.json` carries that time as `upload_by`
  (deadline from the run's `server.json` plus 24 hours; for a replaced run, section 6). When the
  wrap-up ends with bytes left, the finish line names it once: "Run goalcup upload before <time>."
  (review A finding 15, PRD-00 owner question 1, old 2).
- **`goalcup upload [entry]`** (PRD-10 dispatches, this PRD implements `upload::manual::run`):
  1. Take `upload.lock` (an `flock` in the entry folder, never `rec/lock`, so a restarting recorder
     is never shut out: PRD-02 9.2 step 2 exits 73 on a `rec/lock` held past 10 s).
  2. If the entry has no seal and no live recorder, call `pipeline::close_dead_entry` (PRD-02 9.4).
     If it answers `StillRecording`, say "this part is still recording; it sends itself" and stop.
  3. Read the position. If it says `sealed: true` and the offset is the seal LINK's `raw_end`,
     prune and say "already on the server". Otherwise: final heartbeat (with the counts of
     section 6), then chunks, then the final heartbeat until `sealed`, then prune, with section 5's
     rules, giving up only on 401, 404, 410 or `stuck`. A 401 after `upload_by` is said as "the
     server closed this run on <time>; the rest stays on this computer".
- Every uploader, background or manual, holds `upload.lock` for the entry it sends. A second one
  waits on the 1 s tick. Two uploaders on one entry would still be safe (409 then re-read), only
  wasteful.
- The "send leftovers before every command" step (`src/main.rs:349-383`) goes. Leftovers go with the
  next recorder's queue (section 1) or `goalcup upload`, and `goalcup status` names them.

### 10. What the participant sees

The words are PRD-10's. The facts come from `upload.json`. Drafts, plain and true:

| Moment | Line |
|---|---|
| Start | `Upload: to <server>, every 5 minutes.` (local only: `Upload: none. This run stays on this computer.`) |
| Status, caught up | `Upload: the server holds everything up to 14:02 (1 min ago).` |
| Status, waiting | `Upload: not reaching the server since 13:40 (no network). 48 MB waits here and goes up by itself. Nothing is lost.` |
| Status, refused | `Upload: the server refuses this run (401). It keeps trying. If this lasts, write to <support>.` |
| Status, stuck | `Upload: stopped. The server's copy and this computer's copy differ at byte <n>. Nothing more is sent. Write to <support>.` |
| Finish, done | `Upload: done. The server holds your whole record (N MB). Removed here: M parts the server holds.` |
| Finish, not done | `Upload: 120 MB still waits on this computer. Run goalcup upload before Sun 15 Nov, 00:00 UTC, when you are online. Nothing is lost yet.` (the time is `upload_by`) |
| Upload, server closed | `Upload: the server closed this run on Sun 15 Nov, 00:00 UTC. 120 MB stays on this computer and cannot be sent.` |
| Keep on | `Upload: done. Every part is kept on this computer (GOALCUP_KEEP_PARTS).` |

Never a count of calls here: upload is about bytes. Counts are recording's (PRD-10).

### 11. `upload.json` (seam S9, owned here)

One file per entry folder, written by atomic rename at most once a second when something changed,
mode 0600. Read by PRD-10 (status, finish), PRD-12 (pad), PRD-16 (watchers).

```json
{"v":1,"run":"r0123456789abcdef","entry":"e0123456789abcdef","server_url":"https://...",
 "keep":false,"state":"waiting","upload_by":"2026-11-15T00:00:00Z",
 "confirmed":{"raw":726000000,"seq":3601,"hash":"<64 hex>","at":"2026-11-07T10:00:01Z"},
 "durable_raw":727000000,"waiting_bytes":1048576,
 "sent":{"chunks":120,"bytes":98000000},
 "last_ok":"2026-11-07T10:00:01Z",
 "last_error":{"at":"2026-11-07T10:02:30Z","class":"network","status":null,"text":"connection reset"},
 "failures":2,"next_try":"2026-11-07T10:02:34Z",
 "final":"none","pruned":{"parts":3,"bytes":402653184},
 "beat":{"last_ok":"2026-11-07T10:00:00Z","failures":0,"recording":"gaps"},
 "server_word":{"stop":false,"reason":null,"seconds_left":561540,"asked_at":"2026-11-07T10:00:00Z"}}
```

`state` is `local`, `sending`, `caught_up`, `waiting` (network, server busy or `internal`), `refused`,
`stuck`, `done` or `closed` (`upload_by` passed with bytes left; section 1). `final` is `none`, `sent`
(the server committed the final heartbeat) or `sealed` (a heartbeat, chunk or position answer said
`sealed: true`). `waiting_bytes` is compressed bytes between the server's offset and the
durable point. `server_url` is the server's address (renamed from `server`, which PRD-10 first asked
for the answer; PRD-00 named that key `server_word`). `server_word` is the last heartbeat answer,
null before the first one; `goalcup time-left` computes the time left as `seconds_left` minus the
time since `asked_at` and treats a value older than 11 minutes as stale (PRD-10). `upload_by` is
section 9's tail window. `text` never holds a header, a token or a body.

### 12. What of `crates/goalcup-cli/src/upload` stays, changes and goes

Nothing is linked from the old crate (PRD-13 deletes it). "Copied" means rewritten into
`crates/goalcup/src/upload/` with the old tests' cases as acceptance where they apply.

| Old file (src / inline tests) | Fate | Where in 1.0 |
|---|---|---|
| `http.rs` (372 / 0) | **Stays, copied and trimmed**: `ServerClient` with own rustls roots, no proxy from env, no redirects, `json_timed`, `put_stream`, bounded reads. Adds a 60 s no-progress read and write timeout on the agent | `upload/client.rs`, about 220. PRD-10 uses it for device flow, open and continue |
| `policy.rs` (292 / 93) | **Changes**: 5-minute cadence, 16 MiB trigger, 2 s to 120 s backoff with reset on progress, `Retry-After` capped. `WrapUp` and the random first interval kept; `ends_manual_upload` kept; `LOCAL_WRAP_UP`, `WrapUpSpec` and the `from_test_spec` string go (test overrides come through PRD-14's `test-hooks`) | `upload/policy.rs`, about 80 |
| `uploader/chunk.rs` (227 / 0) | **Goes.** No second compression, no spool, no line parser, no 30 MiB soft cut | Replaced by `upload/chunk.rs` (index ranges), about 150 |
| `uploader/session.rs` (511 / 0) | **Changes**: position read, conflict re-read, confirmed-offset rule and the attempt loop stay in idea; acks, `keep_ack`, `owed_ack`, `acks.jsonl`, receipt key go (about 200 lines) | `upload/send.rs`, about 180 |
| `uploader/lifecycle.rs` (577 / 84) | **Changes**: one thread and one loop over the entry queue; the `failures` bug (`:513-533`) fixed by the reset rule | in `upload/send.rs` and `upload/mod.rs` |
| `uploader/heartbeat.rs` (308 / 109) | **Changes**: PRD-07's flat `counts` body, state-change beats, final repeated until `sealed`, `ServerWord`; old-server deadline fallbacks go | `upload/beat.rs`, about 140 |
| `uploader/finish.rs` (354 / 121) | **Changes**: final heartbeat then the rest stay; seal documents, tunnel list reading (`:236-348`) go | in `upload/send.rs` |
| `uploader/local.rs` (246 / 81) | **Goes** mostly: `lock_entry` (now `upload.lock`), `repo_head` (the seal records `workspace_head`), `sent` marker (now `upload.json` `done`), chain end and link lookups (now index lookups) | `upload/state.rs`, about 70 |
| `prune.rs` (173 / 248) | **Changes**: section 7's four conditions; the 2-hour timer, `AckAges`, disk pressure, `clean_finished` and `clean_finished_runs` go | `upload/prune.rs`, about 50 |
| `api.rs` (291 / 51) | **Changes**: `RunPosition` without `last_ack`; `ChunkAnswer` without `ack`; new `HeartbeatRequest`; `HeartbeatAnswer` kept. Open, continue and device types move to PRD-10; the dual `tracer_version` name (`api.rs:18-61`, tests at `:310-341`) goes; seal, receipt and abandon types go | in `upload/client.rs` and PRD-10's files |
| `mod.rs` (523 / 0) | **Splits**: `open_run`, `continue_run`, `is_run_id` (about 200) move to PRD-10; receipt, `keep_run`, abandon, give-back and `pending-open.json` (about 250) go; `UploadRecord` becomes the run's `$GOALCUP_HOME/runs/<run>/server.json`, written by PRD-10 | `upload/mod.rs`, about 60 |
| `uploader.rs` (57 / 25) | Doc and re-exports, rewritten | `upload/mod.rs` |
| Outside the module: `commands/upload.rs` (426), `participant/upload.rs` (107), `src/vitals.rs` (209), leftovers in `main.rs:349-383` | Manual upload logic becomes `upload/manual.rs` (about 90); words move to PRD-10; vitals and leftovers go | |

New code: about 940 src lines (client 220, chunk 150, send 180, beat 140, policy 80, state 70,
prune 50, manual 90, mod 60, minus overlap), against 3,931 src lines in the old module plus 742
outside it. PRD-00's size budget carries the 940.

## Files it owns

Create (all new, in the fresh crate):

- `crates/goalcup/src/upload/mod.rs` (seam S18: `spawn(ctx) -> UploadHandle`,
  `UploadHandle::finish(wrap_up, seal_facts { reason, workspace_head, totals })`; the entry queue)
- `crates/goalcup/src/upload/client.rs` (copied HTTP client, request and answer types for upload)
- `crates/goalcup/src/upload/chunk.rs` (index lookup, chunk cut, headers, the hash pass)
- `crates/goalcup/src/upload/send.rs` (rounds, retries, position, conflict, final order)
- `crates/goalcup/src/upload/beat.rs` (heartbeat, state, final heartbeat, `ServerWord`)
- `crates/goalcup/src/upload/policy.rs` (cadence, backoff, wrap-up)
- `crates/goalcup/src/upload/prune.rs`, `crates/goalcup/src/upload/state.rs` (`upload.json`),
  `crates/goalcup/src/upload/manual.rs` (`goalcup upload`)
- `crates/goalcup/tests/upload_*.rs` (acceptance below), including `upload_worker.rs` (full tier)
- Local files in the entry folder: `upload.json`, `upload.lock`

Must not touch: `crates/goalcup-format/` (PRD-01; the compressor settings live in `part.rs`),
`src/pipeline/` and `src/watchdog.rs` (PRD-02), `src/cli/` and `src/main.rs` (PRD-10),
`server/worker/**`, `scripts/fetch-run.sh` and `docs/UPLOAD-API.md` (PRD-07), `tests/common/`
(PRD-14), the old crates (PRD-13).

## Interfaces with other PRDs

| PRD | What crosses | Direction |
|---|---|---|
| PRD-01 (F1) | `IndexLine {kind L/E/R, seq, raw_end, c_end, hash}`, `read_index(path)`, part naming by first raw offset, the 16 MiB flush bound, `hash(-1)`, write order (index line after its bytes; only newline-ended lines count), rotation order (sync part, sync index, durable point at `E`, drop the old encoder, then open the next); compressor constants in `part.rs` | Used here; old asks B1 to B3 taken by PRD-01 revision 2 |
| PRD-02 (S4, S8) | `Recorder::durable()` watch (`Durable` with entry-total counters at the durable LINK, copied out of `borrow()` at once); the live atomics `written`, `losing`, `held_bytes`, `writer_idle_ms` for `mode`; `status.json` `mode` and `off.since`; `Recorder::now()` (boot time); `pipeline::low_priority()`; `upload::spawn` after recovery (9.2 step 5) and in null mode; the seal hands over with `UploadHandle::finish(wrap_up, seal_facts { reason, workspace_head, totals })` (seam S18); `pipeline::close_dead_entry` and `pipeline::entry_totals` used by `goalcup upload` | Both; asks P1 to P3 |
| PRD-02 and PRD-10 (S10, S9) | `ServerWord {stop, reason, seconds_left, asked_at}` on a `watch` channel: the recorder forwards it to the watchdog as the `server` line; the watchdog resets its deadline timer from `seconds_left` and on `stop` ends the harness and seals with reason `server_stop`. The same answer is `upload.json` `server_word`, which `goalcup time-left` reads | Given here |
| PRD-07 (S5, S13) | `GET` position without `last_ack`, with `sealed`; `PUT` chunk with today's headers, answer `{offset, next_seq, head, received_at, sealed}`; heartbeat with one flat `counts` object (at most 2 KiB, plus `waiting_bytes`; a bad one is dropped with a `counts-dropped` log line, the heartbeat kept, never `400`; keys named in section 6), `repo_head` on the final, `recording` required (`ok`, `gaps`, `failed`; no `unknown` from the client); answer adds `sealed`; the final heartbeat repeated until `sealed: true`; no seal route; no run size limit (413 no longer sent); the 32 MiB cap stays; `GENESIS` stays 64 zeros; `Retry-After` on 429 and 503. `fetch-run.sh` writes `rec.gcr.zst` (chunks joined, not decoded) and `fetch.json` (S13), whose `heartbeats` list U6 reads | Used here |
| PRD-08 | Server copy check: `rec.gcr.zst` equals concatenated local parts (only with `keep`) | Used there |
| PRD-09 | Nothing from the client; reads the fetched copy | none |
| PRD-10 | Writes `$GOALCUP_HOME/runs/<run>/server.json` (server URL, run, deadline) and the run credential before `start` launches; passes `keep` in the start facts; reads `upload.json` (S9, incl. `server_word` and `upload_by`); renders section 10's lines; dispatches `goalcup upload` to `upload::manual::run`; uses `upload::client::ServerClient` for device flow, open and continue | Both |
| PRD-12 | Sets `GOALCUP_KEEP_PARTS=1`; reads `upload.json` (`keep`, `final: sealed`) for its upload wait; runs the cut of U3 and U5 (`cut-worker@1+10`, one `/etc/hosts` line; its stand-in never terminates TLS) | Used there |
| PRD-10 (S11) | `cli::log::line`: the one fixed-text `upload:` line per failed call and the panic line (sections 1 and 5) | Used here |
| PRD-13 | Deletes `crates/goalcup-cli/src/upload/**`, `commands/upload.rs`, `vitals.rs` and the tests listed below | Done there |
| PRD-14 | The fake Worker in `tests/common/` (position, chunk, heartbeat routes with injectable failures; until it lands this PRD keeps a private one, removed at phase 3); the `test-hooks` hooks `GOALCUP_TEST_UPLOAD_POLICY` (round, trigger and backoff in ms) and `GOALCUP_TEST_FSYNC_COUNT` (the feature is pending the owner, PRD-00 owner question 12; default kept, never in a release build). `GOALCUP_TEST_ROTATE_MB` is PRD-02's and no proof here needs it. The clock case of `upload_backoff.rs` needs no hook: `policy.rs` takes `now` as an argument. `upload_worker.rs` (full tier: the real client against `wrangler dev`, `fetch-run.sh --local`, equal bytes) is this PRD's | Used here |
| PRD-16 | First staging deploy after this PRD and PRD-07's heartbeat step merge; the 2-hour run sets `keep`; watcher checks U2 (X5) and U7 (X9) | Used there |
| PRD-17 | Lessons S1 to S9 (S7: a clock set back does not lengthen a segment): lag bug, prune at seal, lost answer after commit, `409` re-read, `awaiting_final` order, no ordinary beat after the final, heartbeat after a cut | Used here |

## What is cut

| Path | Src lines | Tests that go |
|---|---|---|
| `crates/goalcup-cli/src/upload/**` | 3,931 (rewritten to about 940) | 812 inline test lines in `api.rs`, `policy.rs`, `prune.rs`, `uploader.rs`, `finish.rs`, `heartbeat.rs`, `lifecycle.rs`, `local.rs` |
| `crates/goalcup-cli/src/commands/upload.rs` | 426 (to about 90) | in `tests/it/upload_*.rs` |
| `crates/goalcup-cli/src/participant/upload.rs` | 107 (words move to PRD-10) | none |
| `crates/goalcup-cli/src/vitals.rs` | 209 | its unit tests; vitals cases in `tests/it/upload_server.rs` |
| Leftovers step, `crates/goalcup-cli/src/main.rs:349-383` | 35 | in `tests/it/upload_*.rs` |
| Signed acks and receipts on the client (`acks.jsonl`, `receipt.json`, `session.rs:449-496`, `mod.rs:243-295`) | inside the above | ack and receipt cases in `tests/it/upload_chunks.rs`, `upload_open.rs` |
| Upload integration tests | none | `tests/it/upload_chunks.rs` (1,736), `upload_open.rs` (1,548), `upload_server.rs` (1,419), `upload_server_device.rs` (529), `upload_server_segments.rs` (961), the Rust stand-in server `tests/common/upload_server/` (2,514): 8,707 test lines. `tests/full/upload_worker.rs` (456) is rewritten as this PRD's `upload_worker.rs`. Device and open cases PRD-10 still needs are rewritten there |

## Acceptance

### Real-run proofs (pad, PRD-12, Linux sandbox, staging Worker)

Every run here uses the staging Worker with the test approval route (PRD-00 defaults "Server"). Runs
before staging carries 1.0 use `GOALCUP_SERVER=off` and prove nothing for this PRD; its first real
proof is U1 right after the first staging deploy (PRD-16). Database reads happen on the conductor,
which alone holds the read-only credentials. No proof here needs a `test-hooks` build (review B T2).

**The cut.** U3 and U5 block the Worker with one line, `127.0.0.1 <worker host>`, appended to
`/etc/hosts` with `sudo` and removed after 10 minutes. Pad boxes run as `ubuntu` with passwordless
`sudo` (`docs/research/VERCEL-COMPAT.md:47`). The uploader opens one connection per call
(`http.rs:155-158`, kept), so each call resolves the name again and is refused at once, class
`network`. That glibc reads `/etc/hosts` on every lookup in the box (no name cache running) is
unverified; the watcher checks it first with `getent hosts <worker host>` before and after.

| # | Proof | Pass |
|---|---|---|
| U1 | **Server copy equals local.** Any pad run with `keep`. After the finish, `fetch-run.sh` the entry | `rec.gcr.zst` has the same SHA-256 as `cat rec-*.gcr.zst`; `zstd -dc --long=27` exits 0 on both (if PRD-01's recovery fallback is in force, decode part by part instead, using `fetch.json` `bytes` to split `rec.gcr.zst`; each part decodes, and only parts closed by a crash may end inside a frame at an index line); in `fetch.json` every chunk is at most 32 MiB compressed and has raw length above 0, and each chunk's start and end are a part name or an index line's `raw_end` of the same part (no chunk crosses a part); `upload.json` `final` is `sealed` |
| U2 | **The upload keeps up.** A Codex `full` pad run (the payments prompt with many subagents, `keep` on), and again during A1 (PRD-16 X5). A watcher reads `upload.json` every minute; the conductor reads `entries.upload_offset` | At every minute t of the watcher's log, `entries.upload_offset` is at or past the `durable_raw` the log shows at t minus 6 minutes, except while `upload.json` shows a network or server error. No wait over 120 s in the `upload:` lines of `goalcup.log`. After the seal, `done` within 2 minutes |
| U3 | **Network cut.** During a Claude Code `smoke` pad run with one subagent, the cut for 10 minutes, then lifted | The harness turn completes, and its own log shows no error during the cut; in every minute of the cut in which the harness log shows a call, `status.json` written model requests rise. `upload.json` shows `waiting` with class `network` within 1 minute of the first round or heartbeat due after the cut began (at most 6 minutes after it); in the `upload:` lines of `goalcup.log` (section 5), each wait is at most 2^n s for the n-th failure in a row, and none above 120 s; the first heartbeat after the lift goes within 2 minutes; the server catches up within 3 minutes of the lift; U1 holds |
| U4 | **Crash mid-PUT.** The kill test (PRD-02 R1). The watcher times one of the two kills for a moment when `upload.json` says `sending`, when it can | U1 holds: `fetch.json` checks pass with no hole; the `upload:` lines of `goalcup.log` show at most one resend of the chunk that was in flight at the kill; no `stuck`. If no kill met a PUT, the row says so and `upload_resume.rs` stands for it |
| U5 | **Prune.** A Claude Code `smoke` pad run **without** `keep`, with the cut from minute 1 for 10 minutes (PRD-12 `cut-worker@1+10`). The watcher lists the entry folder every 30 s | No part file is deleted while the Worker is blocked. After the finish, every part file and `.idx` is gone and every other local file of section 7 is still there; each part was deleted after the `received_at` of the chunk that ended it (`fetch.json` against the watcher's listings); the server copy decodes and passes `goalcup-verify check --server`. Pruning of parts closed during a run (rotation) is proven by `upload_prune.rs`, because a 20-minute run does not rotate and rotation by hook is not a real-run proof |
| U6 | **Heartbeat says the truth.** The kill test run (PRD-02 R1, two plain `kill -9`) | The heartbeat rows in `fetch.json` `heartbeats` (PRD-07 section 7; no hand-written database query): `recording` is `ok` before the first kill and `gaps` after it, never `failed` (no null mode in that run); `model_requests` never above the verifier's recount at that row's `seq`; the final row's `counts` `off_ms` (`vitals -> 'counts' ->> 'off_ms'`) equals the sum of the two `recorder_down` gaps; the final row's `repo_head` equals the seal's `workspace_head`. The `failed` state is proven by `upload_beat.rs` against PRD-02's `watchdog_null.rs` output and end to end by `upload_worker.rs` |
| U7a | **Memory.** U2's run, with `ps` every 10 s | Recorder RSS minus `held_bytes` (from `status.json`) stays under 260 MiB |
| U7b | **Memory at rotation.** A1 (PRD-16 X9), with `ps` every 10 s | No step of about 140 MiB at the 60-minute rotation |
| U8 | **Manual upload of a dead entry.** `kill -9` watchdog and recorder together mid-run (PRD-02 R7), then `goalcup upload` | The entry is closed `recovered`; the final heartbeat is committed with `recording: "gaps"` and the counts of `counts.jsonl`; `final` is `sealed`; U1 holds; the participant line says what was sent |

Sleep. macOS (owner's run): close the lid for 10 minutes mid-run. Linux stand-in (PRD-12 and
PRD-16 run it): `SIGSTOP` the whole process tree, recorder included, for 10 minutes during a Codex
run, then `SIGCONT`. Pass for both: the first heartbeat after waking is sent within 10 s; the server
catches up within 3 minutes. The stand-in proves catch-up after a long silence, not the clock
choice (boot time runs on under `SIGSTOP`); the clock is proven by `upload_backoff.rs`.

### Automated tests that guard them

- `upload_chunks.rs`: from real `PartWriter` output (PRD-01), chunks start at each of section 3's four
  cases (and at a fallback part that ends at an `L` line), end on index lines, stay within 32 MiB,
  never cross a part, never pass the durable point, never have raw length 0; the `R` recovery chunk
  is sent whole; the first `Goalcup-Prev` is 64 zeros; concatenating every chunk equals the part
  files.
- `upload_backoff.rs`: the wait sequence and the 120 s cap; `Retry-After: 900` waits 120 s; a fake
  Worker that fails every other PUT never makes a wait above 4 s (reset on progress); with `now`
  moved 10 minutes ahead in one step, the round and the heartbeat are due on the next tick. `policy.rs`
  takes `now` as an argument; the clock case is a unit test of that pure function, with no new
  `GOALCUP_TEST_*` hook.
- `upload_resume.rs`: the fake Worker commits a chunk and drops the answer; the next attempt reads
  the position and goes on with no duplicate. A `409` re-reads and moves only to a position a local
  index line confirms. A server offset inside a pruned range gives `stuck`, no send, no panic. A part
  whose last `L` line lies past the file length is never sent. A `409` whose re-read shows no forward
  move counts as a failure and backs off. A scripted answer that panics the parser leaves the recorder
  running, writes one `goalcup.log` line and sets class `internal`.
- `upload_prune.rs`: with the test's own `PartWriter` at a small `rotate_at`: closed parts are pruned
  during the run once confirmed; never the open part, never unconfirmed, never with `keep`, always a
  prefix; only part and `.idx` files are deleted; `upload.json` is written before the delete.
- `upload_beat.rs`: `counts` is flat, at most 2 KiB, and maps the `Durable` counters exactly; a
  `status.json` with `mode: null` gives `failed`, a recovered entry gives `gaps`; a state change
  beats at once; the final heartbeat goes before chunks after the seal and repeats until
  `sealed: true`; no ordinary beat after a final; `503 awaiting_final` is retried at once after the
  final is committed; `ServerWord` carries `stop` and `seconds_left` and lands in `upload.json`
  `server_word`; `writer_idle_ms` over 60 s while records wait gives `failed`, `writer_slow` gives
  `gaps`; a `stop` with reason `replaced` sets `upload_by` to the answer's time plus 24 hours;
  `clock_set_back_does_not_lengthen_a_segment`: the wall clock set back a day between two beats does
  not move the deadline from `ServerWord` later (it counts on `Recorder::now()`, boot time; PRD-17 S7).
- `upload_manual.rs`: gives up on 401, 404, 410 and `stuck` with the reason; refuses while a recorder
  lives; never takes `rec/lock`; sends `counts` from `counts.jsonl`; an entry the position already
  calls `sealed` is not sent again.
- `upload_null.rs`: with no live writer, the uploader calls `fdatasync` before sending (a test hook
  counts calls) and sends up to the last whole index line.
- `upload_worker.rs` (full tier, `#[ignore]` in fast, in a build box): the real client against the
  real Worker under `wrangler dev` with its local database (`DB_BACKEND=local`). (1) A sealed entry
  with two parts; `scripts/fetch-run.sh --local`; the fetched `rec.gcr.zst` equals `cat rec-*.gcr.zst`
  byte for byte; the entry is `sealed`. (2) `failed` end to end: the `test-hooks` binary with forced
  null mode (PRD-02 `watchdog_null.rs` setup); a stored heartbeat has `recording` `failed` and
  `vitals.counts.mode` JSON null (PRD-07 section 4), then `gaps` after the next good boot.

## Risks and blind spots

- **A lost server chunk now costs the rest of its part.** A chunk continues its part's zstd frame
  (PRD-01 section 12), and pruned parts cannot be resent. The Worker stores create-only with the
  SHA-256 in the key, so this needs a server bug. `keep` on test runs is the evidence while 1.0 is new.
- **Pruning removes the participant's own full copy.** Accepted by the owner (answer 9). Local
  `goalcup-verify check` then covers only what is left; PRD-08's server check is the full one.
- **Database rows.** Up to about 3,000 `chunks` rows per busy entry-week in the shared website
  database, against about 336 today; with 1,500 busy entries that is about 4.5 million rows a week
  (estimate). Idle time sends nothing. A 10-minute round halves it (overridable default, PRD-00 owner question 1).
- **Credential and close window.** The run credential and the server's intake end 24 hours after the
  run's deadline (`server/worker/src/db.ts:17`, `:57`, appendix I section 6). A laptop offline longer
  than that, or a Worker outage near the end, loses its tail for good. The participant is told the
  time once (section 9, `upload_by`); a longer window is the owner's (PRD-00 owner question 1, old 2).
- **The errors of run 1 are unexplained** (appendix M 4.7: os errors 11, 104, 32). Smaller chunks
  and a 2-minute cap shrink their cost; they do not remove the cause.
- **No proxy support for the upload.** `try_proxy_from_env(false)` stays (`http.rs:167`), so the run
  token never goes through a proxy. A participant whose network allows only a proxy cannot upload.
  Not handled in 1.0; status says `network`.
- **141 MiB for the whole run** on the participant's machine, about 5 times run 1's two processes.
  Unmeasured in Rust; U7 measures it.
- **Write-time compression speed** is unmeasured (PRD-02 R8 reports it). The uploader cannot help if
  the writer is slow; the guard writes gaps instead.
- **The cut in U3 and U5 relies on `/etc/hosts`** being read on each lookup. If `getent` shows the
  cut did not take, PRD-12's stand-in is used (a local listener that refuses or holds the connection;
  it never terminates TLS); else the row says "not run: cut did not take"; the automated tests still
  cover the behaviour.
- **Counts for a dead entry come from `counts.jsonl`**, not from decoding the record. If that file
  is lost, the final heartbeat sends zeros (never above the recount) and the verifier's server
  check is the truth.
- **Two uploaders on one entry** (a manual one racing a recorder that just started) waste bytes but
  stay correct through 409 and re-read.
- **`kern.bootsessionuuid` and boot-time clocks on macOS** are PRD-02's; the sleep proof is the
  owner's Mac run.

## Overridable defaults

| Default | Value | Who may change it |
|---|---|---|
| Round cadence | 5 minutes, first interval random | Owner (PRD-00 owner question 1, old 6) |
| Size trigger | 16 MiB compressed waiting | PRD-06 after U2 |
| Chunk size | as many index lines as fit in 32 MiB | PRD-07's cap bounds it |
| Backoff | 2 s doubling, cap 120 s, jitter 0.5 to 1.0, reset on progress | PRD-06 |
| Timeouts | connect 15 s, no progress 60 s, whole PUT 600 s, JSON 30 s | PRD-06 |
| Heartbeat | every 5 minutes, and on state change | PRD-06 |
| Wrap-up | 60 minutes on boot time | PRD-10 |
| zstd level | 3, all machines | PRD-06 with a measured reason; level 1 first if R8 names the compressor as slow |
| zstd window 27 and long matching | locked (ledger F2) | Owner only |
| Prune | at once after confirmation; never with `keep` | Owner |
| `keep` | `GOALCUP_KEEP_PARTS=1`, or no server (`GOALCUP_SERVER=off`); no flag | Owner |
| Tail window and its warning | 24 hours after the deadline (after `replaced_at` for a replaced run); one line with the time | Owner (PRD-00 owner question 1, old 2) |
| How U3 and U5 cut the Worker | one `/etc/hosts` line with `sudo` | PRD-12's watcher, if the box needs another way |

## Builder notes

Order of work:

The uploader is in PRD-00's **second slice**: the core slice and phase 1a run with
`GOALCUP_SERVER=off` and do not wait on this PRD. Steps 1 and 2 may start at H+1.5 because they need
only F1 and a fake Worker.

1. `client.rs` first (copy `http.rs`, drop what section 12 cuts, add the no-progress timeouts) and
   `chunk.rs` against PRD-01's `PartWriter` and `read_index` as soon as F1 compiles. Pin the four
   start cases and the 32 MiB end rule with tests before any network code.
2. `policy.rs` and `send.rs` against a fake Worker (PRD-14's `tests/common/` once it lands; a private
   one until then, removed at phase 3): rounds, backoff with reset, 409 re-read, resume. Then
   `state.rs` (`upload.json`, S9 published by H+3).
3. `beat.rs` against PRD-07's S5 body and PRD-02's S4 stubs (`Counts`, `Durable`), then `ServerWord`
   (S10 by H+2), then the final heartbeat order and the `sealed` repeat.
4. `prune.rs`, then `manual.rs` once PRD-02's `close_dead_entry` and `entry_totals` exist.
5. Merge with PRD-07's heartbeat step; PRD-16 then deploys staging. The first real proof is one
   Claude Code `smoke` run with `keep` on staging and U1 on it, before U2 to U8.

Traps from the audit and the old code:

- Reset `failures` on progress, not only on caught-up (`lifecycle.rs:513-533`). This was run 1's
  18-minute wait.
- Never send past the durable point, and `fdatasync` yourself when no writer published one.
- Never treat a reported server offset as confirmed; match it to a local link first
  (`session.rs:204-220`).
- Write `upload.json` before deleting a part, never after.
- Never take `rec/lock` from the uploader (PRD-02 9.2 step 2 exits 73 on it).
- The final heartbeat goes before the tail of chunks past the deadline (`chunks.ts:52-60`), and is
  repeated after the last chunk until `sealed: true`. No ordinary beat after a final.
- `counts` must stay flat and under 2 KiB, or the server drops every `counts` silently. Never put the `hosts` map
  in it.
- No `Authorization` value, body byte or header value in `upload.json` or any log line.
- No 0-length chunk: if the cut gives raw length 0, the index is wrong; stop and say so.
- Read the part file through a second descriptor; never seek the writer's.
- Copy `Durable` out of the watch at once; never hold the `borrow()` across a read, a hash or a call.
- Catch every panic in a round or a beat; the uploader never ends the recorder process.

How a watcher proves it in a real run: start the pad with `GOALCUP_KEEP_PARTS=1` on staging; every
minute log `upload.json` (`confirmed.raw`, `durable_raw`, `state`, `next_try`, `final`) and ask the
conductor for `entries.upload_offset`; after the finish run `fetch-run.sh`, compare the SHA-256 of
`rec.gcr.zst` with `cat rec-*.gcr.zst`, and read `fetch.json` `heartbeats` for U6. For
U3 and U5, add and remove the one `/etc/hosts` line of "The cut" and read the same log.

## Asks of other PRDs

Revision 1's asks, as ruled:

| # | Of | Ask | Ruling |
|---|---|---|---|
| A1 | PRD-00 | Cadence every 5 minutes or 16 MiB, about 3,000 `chunks` rows per busy entry-week | Taken: PRD-00 defaults and owner question 1 (old 6) |
| A2 | PRD-00 | Seams S9 `upload.json` and S10 `ServerWord` | Taken: S9 (with `server_word`) and S10 in PRD-00's seam table |
| A3 | PRD-00 | Upload size about 940 src lines | Taken: PRD-00 size budget |
| A4 | PRD-00 | Owner question on the 24-hour tail window | Taken: owner question 1 (old 2), with the one-line warning (section 9) |
| A5 | PRD-00 | `GOALCUP_KEEP_PARTS=1` on the pad and acceptance runs | Taken: PRD-00 defaults and done item 1 |
| B1 to B3 | PRD-01 | Rotation sync order, drop the encoder first, index line only after its bytes | Taken: PRD-01 revision 2 section 1 |

New in revision 2 (all ruled by PRD-00 R3-2, R3-6 and R3-11; the body above carries the rulings):

| # | Of | Ask | Why |
|---|---|---|---|
| P1 | PRD-02 | `status.json` in `mode: sealed` carries `seal {reason, workspace_head}`. `close_dead_entry` also writes `status.json` with `mode: sealed` and the totals it wrote | `goalcup upload` sends the final heartbeat's `repo_head`, `mode` and counts without decoding a part file |
| P2 | PRD-02 | Expose `pipeline::entry_totals(entry_dir) -> Counts`: `counts.jsonl` summed by PRD-01 section 9 up to the last link on disk, the same rule boot recovery and `close_dead_entry` use | A null recorder and `goalcup upload` need the entry totals; one function, not two copies |
| P3 | PRD-02 | State that `Durable`'s counters are entry totals over boots, and that `UploadHandle::finish` takes the seal's `workspace_head` | The heartbeat sends entry totals and the final `repo_head` |
| P4 | PRD-07 | Apply PRD-00's rulings in the PRD-07 text: drop `unknown` from the client table; the cost row "one chunk per minute, 10,080" becomes about 3,000 a busy entry-week; its interfaces row says 16 MiB, not 32 MiB; `counts` accepts `waiting_bytes` | Builders of PRD-06 and PRD-07 read the same S5 |
| P5 | PRD-10 | `time-left` reads `upload.json` `server_word` (not `server`); the finish line uses `upload_by`; add section 10's "server closed" line | Key names and words of S9 |

## Review answers

This section is history. The body, the ledger and the "Alignment answers" below win over it.

Sources: CC = the consistency check item; A = review A finding; B = review B item.

| Item | Verdict and what changed |
|---|---|
| CC 1 (asks not absorbed) | Accepted. PRD-00 and PRD-01 revision 2 took A1 to A5 and B1 to B3; "Asks of other PRDs" lists the rulings and five new asks |
| CC 21, B C23 (cadence numbers) | Accepted, unchanged: 5 minutes or 16 MiB, about 3,000 rows. Question number updated to PRD-00 owner question 6 (now 1). PRD-07's table is ask P4 |
| CC 23 (A4 as owner question) | Accepted by PRD-00 (owner question 2, now 1). Section 9 adds `upload_by` and the one-line warning |
| CC 31 (seal reason for the server's `stop`) | Accepted. Section 6: the watchdog seals with `server_stop` and `server_reason`; "reason `deadline` or `replaced`" removed |
| CC 40 (B1 to B3) | Accepted by PRD-01 revision 2. Interfaces row PRD-01 cites the write and rotation order |
| CC 46, B C7 (uploader runtime) | Accepted. Two plain threads; the `spawn_blocking` hedge and the macOS QoS class removed (`pipeline::low_priority()`, PRD-02 has no QoS in 1.0) |
| CC 48 (`ServerWord` consumer) | Accepted. The recorder forwards it to the watchdog as the `server` line; the watchdog owns the deadline timer and the `server_stop` seal (section 6, interfaces) |
| CC 49 (seal time budgets) | Accepted. Section 9 says the wrap-up starts after PRD-02's bounded seal and never holds the harness |
| CC 67, B C6 (heartbeat body) | Accepted. Section 6 sends PRD-07's flat `counts` (at most 2 KiB) with `waiting_bytes` and `repo_head`; nested `written`, `off`, `upload` removed |
| CC 68 (manual upload heartbeat) | Accepted, PRD-00's second option: `goalcup upload` sends `counts` and `recording` like any heartbeat. Partly different in how: the totals come from `counts.jsonl` through PRD-02's `entry_totals` (ask P2), which equal the seal's totals, instead of decoding the record, so the uploader still never decodes |
| CC 69 (`unknown`) | Accepted. Never sent: no beat before the first durable point. PRD-07's table is ask P4 |
| CC 70 (final repeat rule, seal hedge) | Accepted. Final heartbeat, chunks, then the final again until `sealed: true`; the seal-route hedge removed; `final` in `upload.json` gains `sealed`. PRD-17 S4 (no ordinary beat after a final) added |
| CC 71, B C8 (`server` key collides) | Accepted. `server` renamed `server_url`; `server_word {stop, reason, seconds_left, asked_at}` added; both the watch channel (S10) and the file key (S9) stay, as review B asked |
| CC 72 (`server.json` scope, files that stay) | Accepted. `server.json` is per run (PRD-10); section 7 lists every file that stays and says prune deletes only part and `.idx` files; fetch output is `fetch.json` (S13) |
| CC 94 (`GOALCUP_TEST_ROTATE_MB` owner) | Accepted. It is PRD-02's; no proof here uses it. U5 runs without a hook and `upload_prune.rs` sets `rotate_at` on its own `PartWriter` |
| CC 105 (`failed` rule) | Accepted. `failed` only while `mode: null`; "or the entry ended while off" removed; a recovered entry reports `gaps` |
| A 14 (which clock) | Accepted; already on boot time. `upload_backoff.rs` now moves a fake `now()` 10 minutes and checks the round and beat fire on the next tick |
| A 15 (Worker outage near the end) | Accepted. `upload_by` in `upload.json`, the finish line names the time, a "server closed" line, a clear manual-upload message on 401 after it. A longer window stays the owner's |
| A coverage: Linux sleep | Partly. PRD-12 and PRD-16 run the `SIGSTOP` stand-in; this PRD states its pass line and says it proves catch-up, not the clock |
| A coverage: network change on the harness leg | Rejected here, routed: the harness leg is PRD-03's and PRD-12's. U3 cuts only the Worker, which is this PRD's leg |
| B C10 (local Worker or staging) | Accepted. Acceptance runs on staging; `GOALCUP_SERVER=off` runs prove nothing here |
| B C25, M7 (seam numbers) | Accepted as renumbered by PRD-00: S9 `upload.json`, S10 `ServerWord` |
| B M3 (`tests/common/` late) | Accepted. A private fake Worker until PRD-14's lands, removed at phase 3 (builder notes) |
| B O7 (first staging deploy) | Accepted. Builder notes step 5: merge with PRD-07's heartbeat step, then staging, then U1 on one smoke run |
| B section 10 (slices) | Accepted. The uploader is second slice; steps 1 and 2 may start early because they need only F1 |
| B T2 (hook-based real runs) | Accepted. U5 runs without a rotation hook; U6 moves to the plain kill-test run; the `failed` state and rotation pruning are automated tests. U4 no longer asks for a 32 MiB burst it cannot cause |
| B W5 (firewall needs root) | Partly. Root is there (passwordless `sudo`, `docs/research/VERCEL-COMPAT.md:47`). A forwarder that terminates TLS breaks name checks against the Worker's host. The cut is one `/etc/hosts` line; the watcher checks with `getent` that it takes. Revision 3: "so no forwarder" removed; PRD-12's stand-in never terminates TLS |
| B W8 (time to first byte within 20%) | Accepted. Dropped from U3; the pass is now "the harness log shows no error and written requests keep rising during the cut", which a flaky provider cannot fail |
| B W10 (server equals local needs kept parts) | Accepted; already so. PRD-00 done item 1 says it |
| A run 1 table (lag bug, prune at seal) | No change needed: sections 5 and 7 already answer them |

## Alignment answers

Revision 3. Fix lists g1, g2 and g3, checked against the ledger, PRD-00 revision 4, PRD-02, PRD-07,
PRD-12 and PRD-17.

| Fix id | What was done |
|---|---|
| F3 | Applied the PRD-06 part: `upload_beat.rs` gains `clock_set_back_does_not_lengthen_a_segment` (PRD-17 S7); the PRD-17 interfaces row names S7. Moving S7's owner to PRD-06 in PRD-17 is that file's part |
| L2-03 | Applied the PRD-06 part (merged with G2-31): section 6 computes `mode` from `Durable` age and the live atomics, and sends `failed` once `writer_idle_ms` passes 60 s while records wait. The status thread and stall exit are PRD-02's part |
| L4-2 | Applied (merged with G2-20): section 2 says window 27 is locked (ledger F2); the 73 MiB window 26 row is dropped; the defaults row is split into level (PRD-06 with a measured reason) and window with long matching (owner only) |
| G2-20 | Applied, as L4-2. The throughput paragraph allows only a level change without the owner |
| 06-1 | Applied: live entry first, then one round per older entry; `refused`, `stuck` or backing-off entries never hold the queue; past `upload_by` the entry is `closed`; the upload thread sends non-live final heartbeats; `closed` added to section 11 |
| 06-3 | Applied: U3's `waiting` timing counts from the first round or beat due after the cut; section 5 adds one S11 `goalcup.log` line per failed call; U3 reads the waits from those lines; S11 row added to the interfaces table |
| 06-4 | Applied: U2's pass line compares `entries.upload_offset` with `durable_raw` six minutes earlier, except while a network or server error shows; no byte-size line. PRD-16 X5 is that file's part |
| 06-5 | Applied in section 1 and in the builder traps: `Durable` is copied out and the borrow dropped at once |
| 06-6 | Applied (merged with G3-X29): `upload_worker.rs` added to the tests and to the files it owns; dropped from "What is cut" (test line total now 8,707) |
| G2-19 | Applied the PRD-06 part: `policy.rs` takes `now`; the clock case is a unit test with no new hook. The PRD-14 row cites PRD-00 owner question 12 (the `test-hooks` feature, called O-3 in the fix list) |
| G2-31 | Applied the PRD-06 part: `mode` from `Durable` age and live atomics; `writer_slow` removed from `ok`; `failed` row covers null mode and `writer_idle_ms` over 60 s; the final heartbeat sends `mode: sealed`. Partly: the fix list did not say where `writer_slow` goes, so it maps to `gaps` (records wait unwritten), never `ok` |
| G2-32 | Applied the PRD-06 part: section 1 puts every round and beat inside `catch_unwind`, class `internal`, never ending the process; `upload_resume.rs` gains the parser-panic case |
| 06-11 | Applied: chunk answer carries `sealed`; the final heartbeat repeats only if the last chunk's answer did not seal; section 11 `final` names all three answers |
| 06-13 | Applied (merged with G3-X26): U6 reads `counts` `off_ms`, path `vitals -> 'counts' ->> 'off_ms'` |
| 06-14 | Applied: U1 decodes part by part under PRD-01's recovery fallback |
| 06-15 | Applied: section 6 sets `upload_by` from a `replaced` stop; section 9 and defaults cite it; `upload_beat.rs` case added |
| 06-16 | Applied: U4 uses `fetch.json` checks and the `goalcup.log` lines, not a Worker log |
| 06-17 | Applied the PRD-06 part: U5 uses `cut-worker@1+10`; Risks name PRD-12's stand-in that never terminates TLS; "so no forwarder" removed from B W5 |
| 06-18 | Applied: Ctrl-C ends the watchdog's wait only; the recorder finishes the wrap-up (PRD-02 section 8 step 11) |
| 06-19 | Applied: no `keep` flag; the defaults row's owner is the owner |
| 06-20 | Applied: U7 split into U7a (U2's run) and U7b (A1, PRD-16 X9) |
| 06-22 | Applied: `api.rs:18-61` (tests at `:310-341`) |
| 06-7 | Applied: section 5 states the nominal waits with jitter and the 120 s ceiling; U3 as 06-3 |
| 06-8 | Applied: a 409 with no forward move counts as a failure; `upload_resume.rs` case added |
| 06-9 | Applied (merged with G3-X22): a bad `counts` is dropped silently by PRD-07, never `400`; trap line changed |
| G2-12 | Applied: PRD-00 is now revision 4 with twelve questions, so old 6 and old 2 are both question 1 and are written "owner question 1, old N"; "Owner decisions it carries out" tags each line with its ledger id (F2, U1, R5, R6, R7, L5, B4) |
| G2-15 | Applied: Files and Interfaces give seam S18's signature `UploadHandle::finish(wrap_up, seal_facts { reason, workspace_head, totals })` |
| X2-11 | Partly. Moving the review sections into a new `REVIEW-HISTORY.md` is not done here: this pass edits only this file, and many writers of one new file would race. "Review answers" now says it is history and that the body, the ledger and these answers win; the revision 2 asks are marked ruled. The move may run once, after every alignment pass, by one agent |
| G3-X22 | Applied: section 6 says a bad `counts` is dropped with `counts-dropped` and the heartbeat kept; section 6 names the keys and `docs/UPLOAD-API.md` copies them; the chunk answer has `sealed` |
| G3-X23 | Applied as case (2) of `upload_worker.rs`. Partly: the fix list wrote `mode` as the string `"null"`; PRD-07 section 4 sends JSON null, so the test asserts JSON null |
| G3-X26 | Applied: U6 and the watcher read `fetch.json` `heartbeats` (PRD-07 section 7 adds the list), not a database query |
| G3-X29 | Applied: `upload_worker.rs` (full tier, `#[ignore]` in fast) with `fetch-run.sh --local` and equal bytes; listed in the PRD-14 interfaces row |

## Amendments

### 2026-10-30, replan after PRD-07 slice 1 (merge 4dcc1e7b)

- **S5 is on integration** (`docs/UPLOAD-API.md`, 4dcc1e7b) and the real Worker follows it for the
  heartbeat `counts`, the three `recording` states, `sealed` in heartbeat, chunk and position answers,
  the seal on the final heartbeat and on the completing chunk, no seal route and no 413 path.
- **Until PRD-07 step 3 merges, the real Worker still sends `ack` in the chunk answer and `last_ack` in
  the position answer.** The client's types must ignore unknown fields (no strict deny), so the same
  client works before and after step 3.
- **`upload_worker.rs` under `fetch-run.sh --local` sees one heartbeat row per entry** (the local
  store keeps `last_vitals` only). U6's check of every heartbeat row reads a staging fetch.

### 2026-10-30, replan after PRD-10 slice 1 (merge d55030ee)

- **S11 is on integration** (`goalcup::cli::log::{line, Fact, Host}`). Upload errors go in as
  `Fact::Io(kind)` for I/O, `Fact::N` for an HTTP status or a byte count (the after-seal count
  included), and `Fact::Word` for a server word from S10's fixed set; never a response body, a server
  message text, the address with a query, or the credential. The server host, when logged, is
  `Fact::Host(Host::checked(..))` of the parsed `GOALCUP_SERVER` authority. `line` never creates a
  folder, so log against an entry folder that exists.
- `cli::server()` returns `Result<Server, &'static str>` with `Server::Url` (no trailing `/`) or
  `Server::Off`; it refuses a user part in the address and `http://` off loopback. The uploader takes
  the base address from it and adds no check of its own.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-10 slice 2 (merge 56495dd6)

- **`server.json` is on integration** as `cli::ServerJson` (PRD-10 note). On `v1.0/prd-06-upload`,
  `upload::server_url` parses it as a loose `serde_json::Value`. Read it instead as
  `cli::read_json::<cli::ServerJson>(&cli::server_json(run_dir))` and take `.server`
  (`None` for off). One shape and less code. The shape is compatible today, so this is not a blocker.
- To write a time, use `goalcup_format::dump::wall`. To parse one, use `cli::unix_of`. Add no
  third time helper.
- `status --json` already prints `upload.json` whole. When S9 merges, PRD-10 adds the upload lines
  to status and the end screen, and `time-left`. S9's `upload.json` keys are then a seam with
  `cli/status.rs` and `cli/finish.rs`.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-14 slice 2 (merge 04886295)

- **The fake Worker is on integration** (`tests/common/worker.rs` at 04886295): S5 routes with its own
  position, chunk answers `{offset, next_seq, head, received_at, sealed}`, a bad `counts` dropped with
  `200`, and a script of injected answers (status and body, `Retry-After`, delay, drop the answer
  after commit). The open acceptance case "a Worker failing every other PUT never makes a wait above
  4 s" can land with `send.rs` on it now; `upload_*` tests drop any private stand-in they kept.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-06 slice 1 (merge e6bd8a50)

- **On integration:** S18 (`upload::spawn`, `UploadCtx`, `UploadHandle::finish`, `SealFacts`), S10
  (`ServerWord` on a `watch` channel), S9 (`state.rs`), `policy.rs`, `client.rs` and
  `tests/upload_backoff.rs`. `chunk.rs`, `send.rs`, `beat.rs`, `prune.rs` and `manual.rs` are later
  slices. Until they merge, `spawn` starts no thread and writes `upload.json` once (`local` with
  `keep` when the server is off, else `waiting` with `last_error` class `internal`), no `ServerWord`
  is ever published, and `finish` returns at once.
- **BR2-r1 (nit, open) goes to the next slice that touches `client.rs`.** `Bounded` checks its deadline
  before each read, but the read then blocks for the socket read timeout that ureq 2.12.1 sets once at
  connect (`src/stream.rs:433-436`). A peer that sends one byte just before the deadline and then goes
  quiet holds a heartbeat or position call for about 60 s and a PUT handshake for about 90 s, not 30 s.
  Take the reviewer's two lines: in `Bounded::read`, before the inner read, set the socket's read
  timeout to the smaller of the time left and `STALL_TIMEOUT`. Check it with a throwaway box probe
  whose peer sends one byte at 29 s and then nothing (the 13:03Z probe trickled every 5 s, so it did
  not reach this case); it is not a committed test (ledger B10). If the lines are not taken, the
  `json()` doc says "about 30 s" and acceptance lists the 60 s and 90 s worst cases. The watchdog is
  not affected: the beat has its own thread and `ServerWord.asked_at` is taken before the call.
- **`server_url` still parses `server.json` as a loose `serde_json::Value`.** The PRD-10 slice 2 note
  stands: the next slice that touches `mod.rs` reads `cli::read_json::<cli::ServerJson>` and takes
  `.server`. It also saves lines.
- **Acceptance case open:** "a worker failing every other PUT never makes a wait above 4 s" was cut from
  `upload_backoff.rs` (13:12Z decision) and lands with `send.rs`. PRD-14's fake Worker
  (`tests/common/worker.rs`, on integration) already serves it: queue alternating
  `Inject { answer: Some((503, ..)), .. }` and `Inject::default()` on `Route::Chunk`. Each inject is
  used once, in order. PRD-14 needs no change for it.
- **Size** (`wc -l` at e6bd8a50, row "Upload (PRD-06)", 940 src and 1,000 tests). Src files total 1,359
  (`client.rs` 692, `state.rs` 289, `policy.rs` 264, `mod.rs` 114). Of those, 232 are inline
  `#[cfg(test)]` modules (client 166, state 66). Without them the src is 1,127, which is 120% of the row
  with 4 of 9 files written. The same four files' own estimates (section 12) add up to 430 (client 220,
  policy 80, state 70, mod 60), so they run at 262%. With the other five at their estimates (chunk 150,
  send 180, beat 140, prune 50, manual 90, together 610), the module projects to about 1,737
  (185%), or 1,969 with the inline tests. The 13:12Z decision line (points 5 and 6: bounded timeouts,
  own TLS, the refusal read after a broken PUT, the `upload.json` fields) covers this merge. It was
  written at 1,106, and the BR1 and BR2 fixes added about 21 lines to `client.rs` after it. From now
  on, later slices build at their own estimates. Each merge needs its own decision line, because the
  module is already past 1,034 (row plus 10%). Before a slice adds anything, it cuts: the loose
  `server.json` parse (above); inline test cases that guard none of the seven points, the format,
  masking, a never-stop rule or an acceptance case (ledger B10); and `policy.rs`, which runs at 330%
  of its estimate, is reviewer B's first place to look. Tests: `tests/upload_backoff.rs` 95 and the
  232 inline lines, 327 of 1,000.
- No locked ledger item is touched and no owner question comes from this slice. The overshoot goes into
  the end-of-build size report against the Rust total.

### 2026-10-30, replan after PRD-14 slice 3 (merge 00fac28d)

- **`Policy::from_env` must be called from the path the `goalcup` binary runs.** PRD-14's `gate full`
  and `release.yml` now require `GOALCUP_TEST_` text in the hooked `target/debug/goalcup` before they
  trust the clean grep of the release binary (PRD-05 amendment). `GOALCUP_TEST_UPLOAD_POLICY` in
  `upload/policy.rs` is the only such read in `src`, and nothing calls `Policy::from_env`, on
  integration or on `v1.0/prd-06-upload` at this replan, so the linker drops it and both checks fail.
  PRD-06's next slice has the uploader take its policy from `Policy::from_env()`; until then
  `gate full` is red and no release can be cut. A plain release build still reads no hook.
- No locked ledger item is touched and no owner question comes from this slice.
