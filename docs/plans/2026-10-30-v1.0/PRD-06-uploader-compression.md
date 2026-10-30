# PRD-06: Uploader and compression

Id: PRD-06. Version: goalcup 1.0. Date: 30 October 2026. Base: main at 4b226de2.
Index: [PRD-00](PRD-00-index.md). Format: [PRD-01](PRD-01-record-format.md) (binding).

## Goal

Send every compressed part file to the server in small chunks, soon after it is written, through any
network trouble, without ever slowing the harness. Tell the server and the participant only written
counts, off time and a plain recording state, and free local disk only once the server holds the bytes.

## Points served

5 (lead: upload everything, so the server can rebuild the story), 3 (the upload never waits on the
harness and the harness never waits on the upload), 6 (the participant sees plainly what reached the
server and what waits), 1 and 2 (no cap of ours on bytes, chunks or entries).

## Owner decisions it carries out

- zstd with a long window (`--long=27`): run 1's server copy was 4 times smaller (100.8 MB against
  408.9 MB, run report "Size and compression").
- Prune a part once the server saved it (owner answer 9, BRIEF2).
- No caps of our own; the per-run size limit goes (owner answers 2 and 3). Only stated memory guards.
- Upload in smaller chunks, more often, with a capped backoff (run report "Fix first").
- The participant, status, heartbeat and server see only written counts and off time (BRIEF3).
- Email work is out of scope: the heartbeat says `failed`; the server can mail later (BRIEF3).
- One process: the uploader lives in the recorder process (option B, BRIEF3).
- Keep everything on organizer test runs (this PRD's scope, from the computed task).
- Chunk cadence of 5 minutes or by size (this PRD's scope; see Ask A1 on PRD-00 question 7).

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
                                                     ServerWord (stop, seconds_left) out
upload thread --> upload.json (state for PRD-10 and the pad)
              --> prune: delete closed parts the server holds
```

- **The uploader never compresses and never decodes.** The writer already compresses (PRD-01
  section 12). A chunk is a byte range of a part file between two index lines. The old second pass,
  its spool file and its line parser go.
- **Two plain threads**, `upload` and `beat`, started by `run_recorder` after boot recovery (PRD-02
  section 9.2 step 4), at the same low priority as other recording work (nice 10, macOS
  `QOS_CLASS_UTILITY`). They use the blocking `ureq` client copied from `http.rs`. They never touch
  the relay runtime, so a hung server cannot reach a relay task. (PRD-02 says "the uploader on the
  same tokio runtime"; two threads are simpler with a blocking client and need no async HTTP crate.
  If PRD-02 insists, the same loops run under `spawn_blocking`.)
- **One queue of entries.** The upload thread serves every entry on this computer whose
  `upload.json` is not `done`, whose run's credential is still there and that is not being sent by
  another uploader (section 9): oldest entry first, the live entry last. That covers a dead entry
  closed by `close_dead_entry` (PRD-02 section 9.4) and an earlier run whose wrap-up ran out.
- **In null mode** (PRD-02 section 8 step 7) both threads still run: what is on disk is sent, and the
  heartbeat says `failed`.
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
reader needs the same 128 MiB and no code decides by RAM. PRD-01 allows window 26; we do not use it
until a measured machine needs it.

Memory, measured with Python `zstandard` 0.25 streaming 300 MiB (appendix M 4.5,
`M-two-hour-run.md:424-425`), unverified in Rust until built:

| Item | Client memory | Note |
|---|---|---|
| Compressor, level 3, no long window (today's uploader) | about 7 MiB | Only while a chunk was built |
| Compressor, `window_log` 26 with long matching | about 73 MiB | Not used in 1.0 |
| **Compressor, `window_log` 27 with long matching (1.0)** | **about 141 MiB** | For the whole run, in the writer. The window fills as data flows: an idle run grows toward it slowly |
| Compressor at level 9, `window_log` 27 | about 159 MiB | Not used |
| Uploader | under 1 MiB | No spool, no body in memory: it hashes the range in one read pass and streams it in a second, with a 64 KiB buffer each |
| Decompression on the client | 0 | The client never decodes. 128 MiB is paid by readers only: `fetch-run.sh` (PRD-07), `goalcup-verify` (PRD-08), the usage reader (PRD-09) |

What this adds up to: run 1's two processes used about 80 MB together (run report measure 7). The 1.0
recorder adds about 141 MiB of compressor, so about 170 to 200 MiB at rest. Under load PRD-02's
memory guard holds up to its limit on top (PRD-02 Risks: about 750 MiB peak). The compressor is part
of the steady cost, not of the guard.

- **Rotation must not double it.** Closing a part and opening the next must drop the old encoder
  before the new one allocates, or the writer briefly holds about 282 MiB (Ask B2 of PRD-01).
- **Sandbox fit.** A Vercel Sandbox gives 2 GB of memory per vCPU
  ([Vercel Sandbox pricing and limits](https://vercel.com/docs/vercel-sandbox/pricing)). A 1-vCPU
  sandbox with Codex at 320 to 385 MB (run 1) and a recorder at 200 to 750 MiB fits; the pad uses at
  least 2 vCPU.
- **Throughput.** Long matching makes compression slower than plain level 3; the rate on the target
  machines is unmeasured (PRD-02 acceptance R8, at least 50 MB/s). If R8 fails, the first change is
  level 1 with the same window and long matching, because the long window is where the gain is. Any
  change needs a measured reason in the night log (PRD-01 overridable defaults).

### 3. Chunks from parts

Inputs: the part files and their `.idx` lines (PRD-01 section 1), the **durable point** from PRD-02's
`watch` channel (seam S4: part path, `c_end`, `raw_end`, link `seq`, hash), and the server's position
(`offset`, `next_seq`, `head`) from `GET /v1/runs/{run}?entry_id=`.

**Where a chunk starts.** The server's `offset` S is a raw offset (PRD-01 section 12). Find it:

1. S equals a part's name (its first raw offset): start at that part, compressed byte 0.
2. Else S equals the `raw_end` of an `L` line: start in that part at that line's `c_end`.
3. Else S equals the `raw_end` of an `E` or `R` line: start at byte 0 of the next part.
4. Else S is nowhere in the local index: the copies disagree (section 5, `stuck`).

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

So the waits run 1, 2, 4, 8 ... up to 2 minutes, never more, and a flaky network that still makes
progress never escalates. That fixes the run 1 bug (`lifecycle.rs:513-533`). The clock is the
recorder's `now()` (boot time, counts sleep), checked on a 1 s tick, so a wait is never stretched by
a suspend.

| Answer | Class | Action |
|---|---|---|
| No answer, reset, timeout (connect 15 s, `http.rs:15`; no byte moved for 60 s; whole PUT 600 s) | `network` | Backoff |
| 429, 5xx, 503 maintenance | `server_busy` | Backoff |
| 503 `awaiting_final` | `server_busy` | Backoff; the beat thread is sending the final heartbeat (section 6) |
| 409 | none | Read the position again at once and go on from there (kept from `session.rs:349-353`) |
| 400, 413, 422, 401, 404, 410 | `refused` | Backoff, for ever, while the process lives: a bad deploy is fixed on the server (D64 holds). Said once after 3 in a row, kept in `upload.json` until the server answers normally |
| The two copies disagree (section 3, step 4 or a hash mismatch) | `stuck` | Read the position again every 2 minutes; never send across the disagreement; say so plainly |

Only `goalcup upload` by hand gives up, on 401, 404, 410 and `stuck`, and says why (kept from
`policy.rs:248-265`). A 422 (SHA-256 refused) re-hashes the range before the retry.

### 6. Heartbeat

The beat thread sends `POST /v1/runs/{run}/heartbeat` for the live entry every 5 minutes, and **at
once** when the recording state changes. A failed beat is retried with section 5's backoff, its own
counter. The body (S5 with PRD-07):

```json
{"entry_id":"e0123456789abcdef","seq":3601,"head":"<64 hex>","offset_local":726000000,
 "final":false,"recording":"gaps","model_requests":905,
 "written":{"http_post":243,"ws_up_msgs":662,"conns":120,"tunnels":118,"native_files":43,
            "native_bytes":48200000,"gaps":3,"lost_bytes":1500000,"lost_unknown":0},
 "off":{"spans":1,"ms":2300,"since":null},
 "upload":{"waiting_bytes":1048576}}
```

- `seq`, `head`, `offset_local`: the durable point's link and `raw_end`. Before the first durable
  point there is nothing to say and no beat is sent.
- `model_requests`: model requests written, `http_post + ws_up_msgs` summed over boots (PRD-01
  section 9, PRD-02 section 6, from S4 `Counts`). Never a selected or offered count.
- `written`: the same COUNT keys, entry totals. `off`: the sum of `recorder_down` spans, plus the open
  span while in null mode (`since` is its start, else null).
- `recording`, decided from S4 and `status.json` `mode`:

| State | When |
|---|---|
| `ok` | No GAP of any reason in this entry, no off time, recording now |
| `gaps` | Recording now, and at least one GAP was written or a stream is losing now (`mode: losing`), or off time is above 0 |
| `failed` | Recording is off now (`mode: null`), or the entry ended while off |

  `unknown` is never sent by 1.0. `goalcup upload` by hand sends no `recording`, `model_requests`,
  `written` or `off`.
- The Worker stores `written`, `off` and `upload` in the existing `heartbeats.vitals` jsonb column
  (`server/worker/sql/goal-cup-tracer-upload.sql:2552`), so no website migration (PRD-00 question 1).
  Disk and process vitals (`api.rs:214-235`, `src/vitals.rs`) are gone.
- **The answer** (`stop`, `reason`, `deadline_at`, `seconds_left`, `api.rs:237-252`) is published
  as `ServerWord {stop, reason, seconds_left, asked_at}` on a `watch` channel, with `asked_at` taken
  before the call (kept from `heartbeat.rs:119-121`). PRD-02 seals on `stop` and resets its deadline
  timer from `seconds_left`; PRD-10's `time-left` reads it. The old-server fallbacks
  (`LocalDeadline::of`, `until`, `heartbeat.rs:249-290`) go: no backward compatibility.
- **Final heartbeat.** Once the seal is durable (PRD-02 section 11 step 4), the beat thread sends
  `final: true` with the seal LINK's `seq`, hash and `raw_end`, and repeats it with backoff until
  it is answered 200 or the wrap-up ends. It goes before the rest of the chunks, because past the
  deadline the Worker refuses chunks until the end is committed (`chunks.ts:52-60`). For a dead entry
  closed by `close_dead_entry`, the final heartbeat names its `recovered` seal the same way.
- **No seal call.** `seal.json`, `baseline.json` and the tunnel list are records now (PRD-01 section
  11). The entry is complete when the final heartbeat is committed and the server's offset reaches it.
  If PRD-07 keeps a `POST .../seal` route to mark the entry, the uploader sends `{"entry_id"}` after
  the last chunk; nothing else.

### 7. Prune, and keep on test runs

A part file and its `.idx` are deleted when all of these hold:

1. The part is closed: its last index line is `E` or `R`.
2. The confirmed server offset is at or past that line's `raw_end`. Confirmed means one of: a 200
   answer to our own PUT whose `offset` and `head` are the chunk's end; or a position read whose
   `offset`, `next_seq - 1` and `head` match a local index line or `upload.json`'s `confirmed`.
   A number the server merely reports is never enough (kept from `session.rs:196-220`).
3. Every older part of the entry is already deleted (prefix only, so the rest stays contiguous).
4. `keep` is false.

There is no 2-hour timer and no disk-pressure branch (`prune.rs:31`, `:47-81`): owner answer 9 and
"disk full: no special handling". `upload.json` records the confirmed position **before** a delete,
so a later boot can still check the server's `head` at a pruned boundary. The open part is never
deleted. After a sealed entry is fully stored, its folder keeps only `upload.json`, `status.json`,
`counts.jsonl` and `cursors.json`.

**`keep` is true** (nothing is ever pruned) when:

- `goalcup start` or `continue` ran with `GOALCUP_KEEP_PARTS=1` in its environment. The pad (PRD-12)
  and the 2-hour run (PRD-16) set it. PRD-10 passes it in the start facts and may also offer a flag.
- The run has no server (local only): nothing can be confirmed.

`keep` is written into `upload.json`, so the verifier and the pad know why parts are there. A
participant who sets it only keeps more on their own disk; that is harmless.

### 8. Resume after a crash, a sleep or a reboot

| Event | What the uploader does |
|---|---|
| Recorder killed mid-PUT | The next boot starts the uploader after recovery (PRD-02 9.2 step 4). It reads the position. If the lost PUT was committed, the offset moved and it goes on; if not, the chunk is cut again from the same lines and resent. The Worker answers a repeat with the stored result (`chunks.ts:44-47`). No spool to clean (today's `session.rs:138-140`) |
| Crash between index line and upload | Nothing special: lines at or before the durable point survive recovery by construction (PRD-01 section 1) |
| Null mode | Keeps sending what is on disk with the `fdatasync` rule of section 3; heartbeat `failed` |
| Laptop sleep | Sockets die; the next call fails and backs off at most 2 minutes. Timers run on boot time, so on waking the 5-minute round and the heartbeat are already due and go at once |
| Machine reboot, or both processes killed | Nothing runs until the participant acts. `goalcup continue` closes the dead entry (PRD-02 9.4) and its recorder's uploader sends the old entry first, then the new one. `goalcup upload` does the same by hand |
| `upload.json` lost or corrupt | It is a cache. The server's position is the truth; it is checked against the local index as in section 3 |
| Server lost bytes the client pruned | `stuck`: said plainly, never "fixed" by sending around the hole. Organizers decide (Risks) |

### 9. The finish and `goalcup upload`

- **After the seal** the recorder process stays alive for the wrap-up: final heartbeat, then rounds
  until `done`, for at most 60 minutes counted on boot time (kept from `policy.rs:86`). PRD-10 shows
  progress from `upload.json` once a second. Ctrl-C at any moment is safe: what is left waits on disk.
- **`goalcup upload [entry]`** (PRD-10 dispatches, this PRD implements `upload::manual::run`):
  1. Take `upload.lock` (an `flock` in the entry folder, never `rec/lock`, so a restarting recorder
     is never shut out: PRD-02 9.2 step 1 exits on a held `rec/lock`).
  2. If the entry has no seal and no live recorder, call `pipeline::close_dead_entry` (PRD-02 9.4).
     If it answers `StillRecording`, say "this part is still recording; it sends itself" and stop.
  3. Final heartbeat, then chunks, then prune, with section 5's rules, giving up only on 401, 404,
     410 or `stuck`.
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
| Finish, not done | `Upload: 120 MB still waits on this computer. Run goalcup upload when you are online. Nothing is lost.` |
| Keep on | `Upload: done. Every part is kept on this computer (GOALCUP_KEEP_PARTS).` |

Never a count of calls here: upload is about bytes. Counts are recording's (PRD-10).

### 11. `upload.json` (seam S9, owned here)

One file per entry folder, written by atomic rename at most once a second when something changed,
mode 0600. Read by PRD-10 (status, finish), PRD-12 (pad), PRD-16 (watchers).

```json
{"v":1,"run":"r0123456789abcdef","entry":"e0123456789abcdef","server":"https://...",
 "keep":false,"state":"waiting",
 "confirmed":{"raw":726000000,"seq":3601,"hash":"<64 hex>","at":"2026-11-07T10:00:01Z"},
 "durable_raw":727000000,"waiting_bytes":1048576,
 "sent":{"chunks":120,"bytes":98000000},
 "last_ok":"2026-11-07T10:00:01Z",
 "last_error":{"at":"2026-11-07T10:02:30Z","class":"network","status":null,"text":"connection reset"},
 "failures":2,"next_try":"2026-11-07T10:02:34Z",
 "final":"none","pruned":{"parts":3,"bytes":402653184},
 "beat":{"last_ok":"2026-11-07T10:00:00Z","failures":0,"recording":"gaps"}}
```

`state` is `local`, `sending`, `caught_up`, `waiting` (network or server busy), `refused`, `stuck`
or `done`. `final` is `none`, `sent` or `committed`. `waiting_bytes` is compressed bytes between the
server's offset and the durable point. `text` never holds a header, a token or a body.

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
| `uploader/heartbeat.rs` (308 / 109) | **Changes**: new body (written counts, off, state), state-change beats, `ServerWord`; old-server deadline fallbacks go | `upload/beat.rs`, about 140 |
| `uploader/finish.rs` (354 / 121) | **Changes**: final heartbeat then the rest stay; seal documents, tunnel list reading (`:236-348`) go | in `upload/send.rs` |
| `uploader/local.rs` (246 / 81) | **Goes** mostly: `lock_entry` (now `upload.lock`), `repo_head` (the seal records `workspace_head`), `sent` marker (now `upload.json` `done`), chain end and link lookups (now index lookups) | `upload/state.rs`, about 70 |
| `prune.rs` (173 / 248) | **Changes**: section 7's four conditions; the 2-hour timer, `AckAges`, disk pressure, `clean_finished` and `clean_finished_runs` go | `upload/prune.rs`, about 50 |
| `api.rs` (291 / 51) | **Changes**: `RunPosition` without `last_ack`; `ChunkAnswer` without `ack`; new `HeartbeatRequest`; `HeartbeatAnswer` kept. Open, continue and device types move to PRD-10; the dual `tracer_version` name (`api.rs:310-341`) goes; seal, receipt and abandon types go | in `upload/client.rs` and PRD-10's files |
| `mod.rs` (523 / 0) | **Splits**: `open_run`, `continue_run`, `is_run_id` (about 200) move to PRD-10; receipt, `keep_run`, abandon, give-back and `pending-open.json` (about 250) go; `UploadRecord` becomes the entry's `server.json`, written by PRD-10 | `upload/mod.rs`, about 60 |
| `uploader.rs` (57 / 25) | Doc and re-exports, rewritten | `upload/mod.rs` |
| Outside the module: `commands/upload.rs` (426), `participant/upload.rs` (107), `src/vitals.rs` (209), leftovers in `main.rs:349-383` | Manual upload logic becomes `upload/manual.rs` (about 90); words move to PRD-10; vitals and leftovers go | |

New code: about 940 src lines (client 220, chunk 150, send 180, beat 140, policy 80, state 70,
prune 50, manual 90, mod 60, minus overlap), against 3,931 src lines in the old module plus 742
outside it. See Ask A3.

## Files it owns

Create (all new, in the fresh crate):

- `crates/goalcup/src/upload/mod.rs` (`spawn(ctx) -> UploadHandle`, the entry queue, `finish`)
- `crates/goalcup/src/upload/client.rs` (copied HTTP client, request and answer types for upload)
- `crates/goalcup/src/upload/chunk.rs` (index lookup, chunk cut, headers, the hash pass)
- `crates/goalcup/src/upload/send.rs` (rounds, retries, position, conflict, final order)
- `crates/goalcup/src/upload/beat.rs` (heartbeat, state, final heartbeat, `ServerWord`)
- `crates/goalcup/src/upload/policy.rs` (cadence, backoff, wrap-up)
- `crates/goalcup/src/upload/prune.rs`, `crates/goalcup/src/upload/state.rs` (`upload.json`),
  `crates/goalcup/src/upload/manual.rs` (`goalcup upload`)
- `crates/goalcup/tests/upload_*.rs` (acceptance below)
- Local files in the entry folder: `upload.json`, `upload.lock`

Must not touch: `crates/goalcup-format/` (PRD-01; the compressor settings live in `part.rs`),
`src/pipeline/` and `src/watchdog.rs` (PRD-02), `src/cli/` and `src/main.rs` (PRD-10),
`server/worker/**`, `scripts/fetch-run.sh` and `docs/UPLOAD-API.md` (PRD-07), `tests/common/`
(PRD-14), the old crates (PRD-13).

## Interfaces with other PRDs

| PRD | What crosses | Direction |
|---|---|---|
| PRD-01 (F1) | `IndexLine {kind L/E/R, seq, raw_end, c_end, hash}`, `read_index(path)`, part naming by first raw offset, the 16 MiB flush bound, `hash(-1)`; compressor constants in `part.rs` | Used here; Asks B1 to B3 |
| PRD-02 (S4, S8) | `Recorder::durable()` watch and `counts()`; `status.json` `mode`; `Recorder::now()` (boot time); `upload::spawn` called after recovery and in null mode; the seal hands over with `UploadHandle::finish(wrap_up)`; `pipeline::close_dead_entry` used by `goalcup upload` | Both |
| PRD-02 and PRD-10 | `ServerWord {stop, reason, seconds_left, asked_at}` on a `watch` channel: PRD-02 seals on `stop` (reason `deadline` or `replaced`) and resets its deadline timer from `seconds_left`; PRD-10's `time-left` reads it | Given here; Ask A2 |
| PRD-07 (S5) | `GET` position without `last_ack`; `PUT` chunk with today's headers, answer without `ack` (column gets `{}`); heartbeat accepts `written`, `off`, `upload` and stores them in `heartbeats.vitals`; `recording` `failed` is real; no run size limit (413 no longer sent); the 32 MiB cap stays; `GENESIS` stays 64 zeros; `Retry-After` on 429 and 503; seal route dropped or reduced to `{entry_id}`; `fetch-run.sh` concatenates then decodes once with `zstd -dc --long=27` | Used here |
| PRD-08 | Server copy check: concatenated fetched chunks equal concatenated local parts (only with `keep`) | Used there |
| PRD-09 | Nothing from the client; reads the fetched copy | none |
| PRD-10 | Writes `server.json` (server URL, run, entry, deadline) and the run credential before `start` launches; passes `keep` in the start facts; reads `upload.json` (S9); renders section 10's lines; dispatches `goalcup upload` to `upload::manual::run`; uses `upload::client::ServerClient` for device flow, open and continue | Both |
| PRD-12 | Sets `GOALCUP_KEEP_PARTS=1`; checks `upload.json` and the database as in Acceptance | Used there |
| PRD-13 | Deletes `crates/goalcup-cli/src/upload/**`, `commands/upload.rs`, `vitals.rs` and the tests listed below | Done there |
| PRD-14 | The fake Worker in `tests/common/` (position, chunk, heartbeat routes with injectable failures); the `test-hooks` feature for policy overrides (`GOALCUP_TEST_UPLOAD_POLICY` = round, trigger and backoff in ms) | Used here |
| PRD-16 | The 2-hour run sets `keep`; watcher checks U2 below every 10 minutes | Used there |
| PRD-17 | Lessons that touch upload: lag bug, prune at seal, lost answer after commit, `awaiting_final` order | Used here |

## What is cut

| Path | Src lines | Tests that go |
|---|---|---|
| `crates/goalcup-cli/src/upload/**` | 3,931 (rewritten to about 940) | 812 inline test lines in `api.rs`, `policy.rs`, `prune.rs`, `uploader.rs`, `finish.rs`, `heartbeat.rs`, `lifecycle.rs`, `local.rs` |
| `crates/goalcup-cli/src/commands/upload.rs` | 426 (to about 90) | in `tests/it/upload_*.rs` |
| `crates/goalcup-cli/src/participant/upload.rs` | 107 (words move to PRD-10) | none |
| `crates/goalcup-cli/src/vitals.rs` | 209 | its unit tests; vitals cases in `tests/it/upload_server.rs` |
| Leftovers step, `crates/goalcup-cli/src/main.rs:349-383` | 35 | in `tests/it/upload_*.rs` |
| Signed acks and receipts on the client (`acks.jsonl`, `receipt.json`, `session.rs:449-496`, `mod.rs:243-295`) | inside the above | ack and receipt cases in `tests/it/upload_chunks.rs`, `upload_open.rs` |
| Upload integration tests | none | `tests/it/upload_chunks.rs` (1,736), `upload_open.rs` (1,548), `upload_server.rs` (1,419), `upload_server_device.rs` (529), `upload_server_segments.rs` (961), `tests/full/upload_worker.rs` (456), the Rust stand-in server `tests/common/upload_server/` (2,514): 9,163 test lines. Device and open cases PRD-10 still needs are rewritten there |

## Acceptance

### Real-run proofs (pad, PRD-12, Linux sandbox, local Worker overnight)

| # | Proof | Pass |
|---|---|---|
| U1 | **Server copy equals local.** Any pad run with `keep`. After the finish, `fetch-run.sh` the entry | Concatenated fetched chunks have the same SHA-256 as `cat rec-*.gcr.zst`; `zstd -dc --long=27` exits 0 on both; every chunk is at most 32 MiB and has raw length above 0; no chunk crosses a part (each chunk's range matches index lines) |
| U2 | **The upload keeps up.** A Codex run whose prompt asks for 20 subagents, 30 minutes, `keep` on. A watcher reads `upload.json` and `entries.offset` every minute | At every minute the server is at most 6 minutes or 64 MiB (raw) behind the durable point. No wait over 120 s in `upload.json` history. After the seal, `done` within 2 minutes |
| U3 | **Network cut.** During a Claude Code run with one subagent, block the Worker's address for 10 minutes (sandbox firewall rule), then lift it | The harness turn completes; per-call time to first byte during the cut is within 20% of before (from recorded OPEN times). `upload.json` shows `waiting` with class `network` within 1 minute; waits are 2 s, 4 s ... capped at 120 s; the server catches up within 3 minutes of the lift; U1 holds |
| U4 | **Crash mid-PUT.** The kill test (PRD-01 acceptance 3, PRD-02 R1) with a 32 MiB burst in flight | U1 holds; the Worker log shows at most one repeat of the in-flight chunk; no `stuck` |
| U5 | **Prune.** A Claude Code run without `keep`, forced to rotate (a `test-hooks` rotation at 64 MiB), with the Worker blocked for the first 10 minutes | No part is deleted while the Worker is blocked. After it returns, each closed part is deleted only after its chunks are stored (compare file mtimes of deletion in the host log with `chunks.received_at`). After the finish, only the four state files remain, and the server copy still decodes and passes `goalcup-verify check --server` |
| U6 | **Heartbeat says the truth.** The null-mode run (PRD-02 R6) | Database `heartbeats` rows: `recording` goes `ok` or `gaps`, then `failed` within 10 s of null mode, then `gaps` after recovery; `model_requests` never above the verifier's recount at that row's `seq`; the final row's `vitals.off.ms` equals the sum of `recorder_down` gaps |
| U7 | **Memory.** U2's run, with `ps` every 10 s | Recorder RSS minus `held_bytes` (from `status.json`) stays under 260 MiB; no step of about 140 MiB at rotation |
| U8 | **Manual upload of a dead entry.** `kill -9` watchdog and recorder together mid-run (PRD-02 R7), then `goalcup upload` | The entry is closed `recovered`, the final heartbeat committed, U1 holds, the participant line says what was sent |

macOS (owner's run): close the lid for 10 minutes mid-run. Pass: the first heartbeat after waking is
sent within 10 s; the server catches up within 3 minutes.

### Automated tests that guard them

- `upload_chunks.rs`: from real `PartWriter` output (PRD-01), chunks start at each of section 3's four
  cases, end on index lines, stay within 32 MiB, never cross a part, never pass the durable point,
  never have raw length 0; the `R` recovery chunk is sent whole; the first `Goalcup-Prev` is 64 zeros;
  concatenating every chunk equals the part files.
- `upload_backoff.rs`: the wait sequence and the 120 s cap; `Retry-After: 900` waits 120 s; a fake
  Worker that fails every other PUT never makes a wait above 4 s (reset on progress).
- `upload_resume.rs`: the fake Worker commits a chunk and drops the answer; the next attempt reads
  the position and goes on with no duplicate. A server offset inside a pruned range gives `stuck`, no
  send, no panic. A part whose last `L` line lies past the file length is never sent.
- `upload_prune.rs`: never the open part, never unconfirmed, never with `keep`, always a prefix;
  `upload.json` is written before the delete.
- `upload_beat.rs`: the body maps S4 counts exactly; a state change beats at once; the final
  heartbeat goes before chunks after the seal; `503 awaiting_final` then 200 completes the entry;
  `ServerWord` carries `stop` and `seconds_left`.
- `upload_manual.rs`: gives up on 401, 404, 410 and `stuck` with the reason; refuses while a recorder
  lives; never takes `rec/lock`.
- `upload_null.rs`: with no live writer, the uploader calls `fdatasync` before sending (a test hook
  counts calls) and sends up to the last whole index line.

## Risks and blind spots

- **A lost server chunk now costs the rest of its part.** A chunk continues its part's zstd frame
  (PRD-01 section 12), and pruned parts cannot be resent. The Worker stores create-only with the
  SHA-256 in the key, so this needs a server bug. `keep` on test runs is the evidence while 1.0 is new.
- **Pruning removes the participant's own full copy.** Accepted by the owner (answer 9). Local
  `goalcup-verify check` then covers only what is left; PRD-08's server check is the full one.
- **Database rows.** Up to about 3,000 `chunks` rows per busy entry-week in the shared website
  database, against about 336 today; with 1,500 busy entries that is about 4.5 million rows a week
  (estimate). Idle time sends nothing. A 10-minute round halves it (overridable default).
- **Credential and close window.** The run credential and the server's intake end 24 hours after the
  run (`server/worker/src/db.ts:17`, appendix I section 6). A laptop offline longer than that cannot
  upload its tail. The uploader keeps trying and says `refused`; the fix is on the server (Ask A4).
- **The errors of run 1 are unexplained** (appendix M 4.7: os errors 11, 104, 32). Smaller chunks
  and a 2-minute cap shrink their cost; they do not remove the cause.
- **No proxy support for the upload.** `try_proxy_from_env(false)` stays (`http.rs:167`), so the run
  token never goes through a proxy. A participant whose network allows only a proxy cannot upload.
  Not handled in 1.0; status says `network`.
- **141 MiB for the whole run** on the participant's machine, about 5 times run 1's two processes.
  Unmeasured in Rust; U7 measures it.
- **Write-time compression speed** is unmeasured (PRD-02 R8). The uploader cannot help if the writer
  is slow; the guard writes gaps instead.
- **Two uploaders on one entry** (a manual one racing a recorder that just started) waste bytes but
  stay correct through 409 and re-read.
- **`kern.bootsessionuuid` and boot-time clocks on macOS** are PRD-02's; the sleep proof is the
  owner's Mac run.

## Overridable defaults

| Default | Value | Who may change it |
|---|---|---|
| Round cadence | 5 minutes, first interval random | Owner (PRD-00 question 7) |
| Size trigger | 16 MiB compressed waiting | PRD-06 after U2 |
| Chunk size | as many index lines as fit in 32 MiB | PRD-07's cap bounds it |
| Backoff | 2 s doubling, cap 120 s, jitter 0.5 to 1.0, reset on progress | PRD-06 |
| Timeouts | connect 15 s, no progress 60 s, whole PUT 600 s, JSON 30 s | PRD-06 |
| Heartbeat | every 5 minutes, and on state change | PRD-06 |
| Wrap-up | 60 minutes on boot time | PRD-10 |
| zstd | level 3, window 27, long matching, all machines | PRD-06 with a measured reason; level 1 first if R8 fails |
| Prune | at once after confirmation; never with `keep` | Owner |
| `keep` | `GOALCUP_KEEP_PARTS=1`, or no server | PRD-10 may add a flag |

## Builder notes

Order of work:

1. `client.rs` first (copy `http.rs`, drop what section 12 cuts, add the no-progress timeouts) and
   `chunk.rs` against PRD-01's `PartWriter` and `read_index` as soon as F1 compiles. Pin the four
   start cases and the 32 MiB end rule with tests before any network code.
2. `policy.rs` and `send.rs` against PRD-14's fake Worker: rounds, backoff with reset, 409 re-read,
   resume. Then `state.rs` (`upload.json`).
3. `beat.rs` with S4 stubs (`Counts`, `Durable`), then `ServerWord`, then the final heartbeat order.
4. `prune.rs`, then `manual.rs` once PRD-02's `close_dead_entry` exists.
5. Phase 1a: the first real Claude Code run with `keep` against a local Worker; check U1 before
   anything else merges.

Traps from the audit and the old code:

- Reset `failures` on progress, not only on caught-up (`lifecycle.rs:513-533`). This was run 1's
  18-minute wait.
- Never send past the durable point, and `fdatasync` yourself when no writer published one.
- Never treat a reported server offset as confirmed; match it to a local link first
  (`session.rs:204-220`).
- Write `upload.json` before deleting a part, never after.
- Never take `rec/lock` from the uploader (PRD-02 9.2 step 1 exits on it).
- The final heartbeat goes before the tail of chunks past the deadline (`chunks.ts:52-60`).
- No `Authorization` value, body byte or header value in `upload.json` or any log line.
- No 0-length chunk: if the cut gives raw length 0, the index is wrong; stop and say so.
- Read the part file through a second descriptor; never seek the writer's.

How a watcher proves it in a real run: start the pad with `GOALCUP_KEEP_PARTS=1`; every minute log
`upload.json` (`confirmed.raw`, `durable_raw`, `state`, `next_try`) and the database `entries.offset`;
after the finish run `fetch-run.sh`, compare SHA-256 of the fetched concatenation with
`cat rec-*.gcr.zst`, and read the `heartbeats` rows for U6. For U3, add and remove one firewall rule
for the Worker's address and read the same log.

## Asks of PRD-00 and PRD-01

| # | Of | Ask | Why |
|---|---|---|---|
| A1 | PRD-00 | Change the default of owner question 7 from "a chunk every minute" to **every 5 minutes, or at once when 16 MiB waits**, and its row estimate from about 10,000 to about 3,000 `chunks` rows per busy entry-week | This PRD's scope sets 5 minutes; the two must agree |
| A2 | PRD-00 | Add seam **S9** (`upload.json`, owner PRD-06, used by 10, 12, 16, stable by H+3) and seam **S10** (`ServerWord` watch channel, owner PRD-06, used by 02 and 10, stable by H+2) to the seam table. Today nothing carries the heartbeat's `stop` and `seconds_left` to the seal or to `time-left` | The deadline and "replaced" stop must reach PRD-02's seal |
| A3 | PRD-00 | Raise the upload size estimate from 700 to about 940 src lines | The copied HTTP client with its own rustls roots is about 220 of them; PRD-10 shares it for device flow, open and continue |
| A4 | PRD-00 | Add an owner question: **how long after a run ends does the server take its tail?** Today 24 hours, and the credential expires with it (`server/worker/src/db.ts:17, 57`). Default: unchanged for 1.0 | Point 5 says upload everything; a laptop offline for a day loses its tail (appendix I section 6) |
| A5 | PRD-00 | Name `GOALCUP_KEEP_PARTS=1` in the pad and 2-hour run defaults, so "server copy equal to local" in the definition of done can be checked | Without `keep`, closed parts are pruned before the check |
| B1 | PRD-01 | At rotation, `fdatasync` the closed part and then its index (with the `E` line) before the durable point moves into the next part. Section 1 names "every 5 s and at the seal" only | Otherwise a power loss can drop an `E` line the uploader already sent past |
| B2 | PRD-01 | At rotation, finish and drop the old encoder before the new one allocates | Avoids a brief 282 MiB peak (two 141 MiB compressors) |
| B3 | PRD-01 | State in section 1 that an index line is written only after the compressed bytes up to its `c_end` are written to the part file, and that a reader takes only lines ending in a newline | The uploader in null mode and `goalcup upload` rely on it |
