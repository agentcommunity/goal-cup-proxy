# Server plan: upload, storage, auth and the admin page

What the server side of Goal Cup should be, and what the tracer must add to use it. This is a plan for the
owner to decide on (TODO.md steps 2 and 3). Nothing here is built yet. Updated: 2026-10-13.

Decisions are written as options with a recommendation. Section 9 lists the open questions in order of
importance.

## 0. What exists today (the ground this stands on)

- **One file carries the whole run.** `goalcup start` runs the capture proxy, and a run-scoped
  `goalcup-daemon` writes the entry's evidence into a trace directory. The important file is
  `chain.jsonl`: append-only, one hash-linked JSON envelope per line, with both legs (the harness's own
  session file as redacted `Segment` records, and its model traffic). Each line carries `seq`, `prev`
  and `hash`, and `hash = sha256(prev ++ CHAIN_DOMAIN ++ canonical content)` (`Envelope::chain_bytes`
  in `crates/goalcup-protocol/src/envelope.rs`). The chain starts at `seq` 0 with 64 zeros as `prev`.
- **The other files are small.** `baseline.json` is bound by the `seq` 0 claim, and `seal.json` by the
  final `LocalFinish` record.
- **Delivery is local only.** There is no server, receipt or acknowledgement today. The v5 record format
  is frozen, so the server plan must not add chain records.
- **Outage windows already exist.** `crates/goalcup-cli/src/proxy/outage.rs` coalesces failed upstream
  connects into windows (`first_failure`, `last_failure`, `failures`, `recovered`). Today they are only
  printed at finish, because the chain has no record kind for them.
- **Measured growth.** A real Codex run wrote about 90 KB/s (15.2 MB in 170 s), roughly 0.3 GB/hour or
  54 GB/week. Claude Code wrote about 17 KB/s (about 3 GB/week). A 30-minute synthetic load wrote
  180 MB. Tonight's 8-hour run measures the real curve; treat these as estimates until then.
- **The fixed reservation will run out.** Each run reserves 10 GiB of disk up front
  (`DEFAULT_CAPTURE_RESERVE_BYTES`). A week of Codex does not fit, which is why pruning (section 7)
  matters.
- **Identity already lives on Supabase.** The website's identity and community records are on
  Supabase. `docs/registration-and-auth-design.md` plans event records there, and says large traces
  belong in private object storage, not database rows.

## 1. Hosting options

Sizing basis: about 50 participants, up to about 50 GB raw per participant per week in the worst case
(all Codex), so about 2.5 TB raw for the event week. JSON compresses well, but the factor is unmeasured;
the costs below assume no compression. Kept for about 3 months for judging and research.
Prices are public list prices as known up to mid-2026. **Check them before deciding.**

| | (a) Cloudflare Worker + R2 | (b) Supabase (Storage + Postgres + Auth) | (c) Rust service on a VPS + R2/S3 |
|---|---|---|---|
| Storage, 2.5 TB × 3 months | R2 ~$0.015/GB-month ≈ $38/month | ~$0.021/GB-month over the Pro plan's 100 GB ≈ $50/month, plus the $25 plan | R2 as in (a), or S3 ~$0.023/GB-month ≈ $58/month |
| Researchers downloading everything once | Free: R2 has no egress fee | ~$0.09/GB past the included 250 GB ≈ $200 per full pull | Free with R2; ~$0.09/GB with S3 |
| Compute | Workers paid plan ~$5/month | Edge Functions included in Pro | A ~€5–10/month VPS |
| Ops burden | Low: no servers; deploy with `wrangler` | Lowest for auth and DB; storage is managed | Highest: patching, TLS, monitoring, backups are ours |
| Auth fit | Must verify Supabase-issued tokens or our own upload tokens | Native: participants are already Supabase users, and RLS matches the spec | Our code verifies tokens; can call Supabase for identity |
| Large uploads | Presigned PUT to R2 (up to 5 GB per object, multipart beyond) | Resumable (TUS) uploads; large file limits on paid plans | Streams through our service, or presigned R2 URLs |
| Verifying chunks | A Worker can stream and hash a chunk; per-request CPU limits make a queue consumer safer for big chunks | An Edge Function reading big objects is awkward; better as a separate job | Easy: the service is Rust and links `goalcup-protocol` directly |
| Lock-in | R2 speaks the S3 API, so data is portable; Worker code is Cloudflare-specific but small | Postgres is portable; Storage and RLS rules less so | Least: portable binary, S3 API |

**Recommendation: a hybrid of (b) for people and (a) for bytes.**
- **Supabase** keeps what it is already good at: identity, event records, participants, upload tokens,
  entry rows, and the admin page's data, with RLS.
- **R2** holds the evidence bytes: cheapest at this size, and free for researchers to read.
- **A small control service** sits between them. It checks the token, records the entry state, hands out
  presigned R2 URLs, and verifies each chunk. Write it in Rust so it reuses `goalcup-protocol`'s chain
  check, and run it on Cloudflare (Workers, wasm) or a tiny VPS. The protocol below does not care which.
  If the owner wants zero servers, a Supabase Edge Function can mint the URLs, and a scheduled job does
  the verification.

## 2. Upload protocol (provider-neutral)

All requests use HTTPS and `Authorization: Bearer <upload token>`. JSON in, JSON out. The token is read
from a 0600 file in the tracker's config directory: never from the harness's environment, never logged,
never written into the trace.

| Call | Purpose |
|---|---|
| `POST /v1/entries` `{entry_id, connector, harness_version, tracer_version, created_at}` | Register an entry (idempotent). Returns its upload state. |
| `GET /v1/entries/{id}` | Upload state `{offset, seq, head, sealed}`: the server's last verified position. How a crashed or restarted CLI resumes. |
| `POST /v1/entries/{id}/chunks` `{offset, length, sha256, first_seq, prev, last_seq, head}` + body | Append bytes `[offset, offset+length)` of `chain.jsonl`, complete lines only, zstd-compressed in transit. The server answers with an **ack** `{offset, seq, head, received_at}`, or `409` plus its current state if the offset is not the next one. |
| `POST /v1/entries/{id}/heartbeat` `{offset_local, records, gaps, outage_open, at}` | Tiny, every 5 minutes. "Last contact" and live health for the admin page. |
| `POST /v1/entries/{id}/outages` `[{first_failure, last_failure, failures, recovered}]` | Outage windows, sent with each hourly chunk and at seal. The chain stays unchanged. |
| `POST /v1/entries/{id}/seal` `{seal.json, baseline.json}` | After the last chunk. The server checks the seal binds the final head and marks the entry sealed. |

**Cadence.**
- A chunk every 60 minutes, or sooner once 256 MB is waiting.
- A heartbeat every 5 minutes.
- At finish, flush everything and send the seal.

**Offline.** Recording never waits on the network. The uploader retries every 60 s with a little jitter;
when the network is back, it catches up in 256 MB chunks, oldest first.

**After a CLI crash.** `goalcup upload <entry>`, or the next `goalcup start` or `seal`, calls
`GET /v1/entries/{id}` and continues from the server's offset. The server is the source of truth for
what it has, so no local state can disagree with it. The client also keeps a small `uploads.jsonl` of
acks in the config directory, for display and for pruning (section 7).

**Mapping onto signed-URL uploads (options a and b).** The chunk call splits in two:
1. `POST /v1/entries/{id}/chunks/intent` with the same JSON. The service checks the offset and returns a
   short-lived presigned `PUT` URL for `staging/{entry}/{offset}`.
2. The client `PUT`s the bytes straight to R2.
3. `POST /v1/entries/{id}/chunks/commit`. The service streams the staged object, verifies it
   (section 3), moves it to its final key, and returns the ack.

Same semantics, and the bytes never pass through our compute.

**Where the uploader lives.** In the CLI (`goalcup start`), which already runs the proxy and owns the
network. The daemon stays network-free and small, as AGENTS.md asks.

## 3. What the server verifies, and when

| When | Check | On failure |
|---|---|---|
| Every chunk (live, cheap) | The offset is the next one, the bytes end on a newline, and the length and sha256 match. Every line is a canonical v5 envelope for this `entry_id`. `seq` continues, `prev` equals the stored head, and the recomputed hash equals `hash`. This is the same line check as the verifier's reader (`crates/goalcup-verifier/src/capture_analysis/reader.rs`). | Reject the chunk (`422`) and flag the entry. The client keeps its local copy and reports it. |
| Seal | The final head is the `LocalFinish` record, `seal.json`'s digest matches what `LocalFinish` binds, and `baseline.json` matches the `seq` 0 claim. | Mark the entry "sealed with errors". |
| After seal (batch) | Run `goalcup-verify --capture-integrity` on the reassembled trace, and store the exit and summary. | That is the integrity result; judging stays human. |

Content judgements (rules, human input, scoring) stay offline and later: record raw, interpret later.
A server accepting an upload is not a compliance verdict.

## 4. Auth

- **Now, for the practice beta: hand-issued upload tokens.**
  - An organizer creates a token per participant in Supabase and gives it to them out of band. Only a
    hash of it is stored.
  - The participant runs `goalcup login --token-file <path>`, a new subcommand that copies the token into
    the config directory at mode 0600.
  - A token can create entries and append to its own entries only: append-only, no reads of other
    entries, no deletes.
  - If an agent on the same machine read the token (same user; a known limit), the worst it could do is
    append chunks, which fail the continuity check.
- **Later: the device flow in `docs/registration-and-auth-design.md` section 6.**
  - `goalcup auth` opens the browser, the person approves, and the CLI stores an installation grant.
  - The control service derives entry-scoped upload authority from that grant. The upload calls stay the
    same; only the token's source changes.
  - Token lifetimes must cover a seven-day run without an interactive login (the spec's own
    requirement). Use a refresh-token style grant, refreshed by the uploader.
- **Not in v1:** server-signed acks. The server's copy is the tamper anchor (a participant cannot rewrite
  what was uploaded an hour ago). Signed acks become worth it if pruning is chosen (section 7).

## 5. Storage, retention and access

**Layout in R2 (private bucket):**

```
{event}/{participant}/{entry_id}/chain/{offset:016}-{end:016}.jsonl.zst
{event}/{participant}/{entry_id}/baseline.json
{event}/{participant}/{entry_id}/seal.json
{event}/{participant}/{entry_id}/outages.jsonl
```

**Rows in Supabase Postgres:**
- `entries`: participant, connector, created, last contact, verified offset, `seq`, head, sealed, verdict.
- `chunks`: offset, length, sha256, `seq` range, head, received time, object key.
- `outages`: one row per window.

**Retention (proposal):**
- Practice entries: 30 days.
- Competition entries: until results and appeals close, plus a research period the owner sets (for
  example 12 months), then deleted.
- A participant can ask for deletion; the organizers handle it by hand at first.

**Encryption at rest:** R2 and Supabase encrypt at rest by default. Client-side encryption is not
proposed: researchers need to read the bytes, and it would add key custody for little gain.

**Who reads what:**

| Who | Access |
|---|---|
| Participants | Write-only, to their own entries. They see their own status on the dashboard. |
| Organizers | Read everything through the admin page. |
| Researchers | Read-only R2 API tokens scoped to the bucket, or time-limited download links. No write or delete. |
| The public | Nothing. Traces are never published. Prompts follow the publication policy, after the event. |

## 6. The admin page (TODO step 3)

One page, inside the website, behind Supabase auth with an organizer role. Per entry:
- participant;
- harness and version;
- start time and **first prompt**. The goal is captured as its own stream (`CaptureSource::Goal`) right after
  the entry is created, so the server extracts it from the first chunk. Only the first prompt matters for a one-prompt event, though the server can flag
  later human input;
- last contact (heartbeat), and uploaded-up-to (time, records, bytes);
- gaps;
- outage windows;
- sealed or not, and the integrity result after seal.

Data comes from the Postgres rows the control service and the post-seal job write. The page reads rows,
never raw traces. Raw evidence stays in R2, behind a download link for organizers.

## 7. Local pruning after ack (needs an owner decision)

The problem: a week of Codex is about 54 GB, and the run reserves 10 GiB up front. Without pruning, a
long Codex run fills its reservation (capture then stops with `StorageFull` gaps, and the agent keeps
running) or needs a huge reservation.

| Option | How | What it costs |
|---|---|---|
| A. No pruning | Raise the reservation to fit the run, or accept a capture stop | Simplest, but a 50+ GB reservation on a participant's laptop, or lost capture late in the run |
| **B. Punch holes in the acked prefix (recommended)** | The file stays one `chain.jsonl` with unchanged offsets. Once the server has acked offset N, and the data before N is older than 2 hours, the CLI frees those blocks (`F_PUNCHHOLE` on APFS, `fallocate(PUNCH_HOLE)` on Linux), then **re-reserves** the same amount ahead of the writer, so the reservation rolls forward instead of running out. | Local `goalcup-verify` can no longer check the whole chain from local disk. It checks the tail from a checkpoint `{offset, seq, head}`, and the server copy is checked in full. The checkpoint must be trustworthy, so B is where signed server acks earn their place. Needs a short measured spike: punching behind an active writer on APFS, and re-reserving. |
| C. Segment files | Rotate `chain.jsonl` into hourly files, and delete acked ones | A trace layout change, more verifier work, and it touches the frozen-format boundary. More than B for the same result. |

Recommendation: B, after tonight's run gives the real growth rate. It needs a decision entry, because it
changes what "verify locally" means.

## 8. Tracer-side work, in small PRs

The same in every hosting option. Sizes are rough (S ≈ under 300 lines, M ≈ 300–1,000, L ≈ more).

| # | PR | Size | Notes |
|---|---|---|---|
| 1 | Reference ingest server (Rust, filesystem store) implementing section 2 and the section 3 checks | M | Used by every client test. In option (c) it is the start of production. |
| 2 | Uploader in `goalcup start`: `upload.toml` (url, token file), complete-line chunker, ack state, hourly plus 60 s retry, catch-up | M | Off unless configured. Loopback end-to-end tests: normal, outage, restart, wrong token, tampered chunk. |
| 3 | `goalcup upload <entry>` and `goalcup login --token-file` | S | Resume from `GET` state. Pushes crashed or finished entries. |
| 4 | Heartbeat and outage windows | S | Uses `proxy/outage.rs` as is. |
| 5 | Seal flush and seal call; plain-language status lines | S | `goalcup status` shows uploaded-up-to. |
| 6 | Signed-URL transport (intent / PUT / commit) | S | Only if (a) or (b) is chosen. |
| 7 | Pruning: punch holes, rolling reservation, verifier checkpoint mode, signed acks | M–L | After the section 7 decision and a spike. |
| 8 | `goalcup auth` device flow | M | After the registration work in the website. |

**Server-side work, for the hybrid:**

| Work | Size |
|---|---|
| Control service (from PR 1, plus R2 and Supabase adapters) | M |
| Supabase tables and RLS | S |
| Post-seal verify job | S |
| Admin page | M |

## 9. Open questions for the owner (most important first)

1. **Hosting:** approve the hybrid (Supabase for identity, rows and the admin page; R2 for evidence; a
   small Rust control service)? Or pick (a), (b) or (c) outright.
2. **Pruning:** option B (punch holes plus a rolling reservation; local verification becomes "tail from
   a checkpoint"), A, or C?
3. **Access and retention:** organizers and read-only researchers only; practice kept 30 days;
   competition kept until appeals close plus how long for research? Who handles deletion requests?
4. **Is upload mandatory for a competition entry** (no upload, not eligible)? Or is local-only still
   acceptable in some cases?
5. **Prompt visibility on the admin page:** organizers only until the event ends?
6. **Tokens:** hand-issued per participant for the practice beta. When must the full `goalcup auth`
   flow be ready?
7. **Signed acks:** only together with pruning (recommended), or from day one?
8. **Data location:** any EU residency need? R2 supports EU-jurisdiction buckets.
9. **Uplink budget:** is about 0.3 GB/hour per Codex participant acceptable, with compression on by
   default?
