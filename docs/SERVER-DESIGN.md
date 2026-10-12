# Goal Cup server: upload, storage and live numbers

Design for the server side of the tracer, for the event going live on **6 November 2026**. Written
2026-10-13 from the owner's decisions, then revised after four reviews (builder check, security and
privacy, scale and operations, Cloudflare product research). Replaces the draft `docs/SERVER-PLAN.md`
(PR #25). Calls marked *(my call)* were made by the author and are open to correction. The owner's answers to the
open questions are recorded in section 15. Platforms for 6 November: macOS and Linux, plus Windows
through WSL2; installation through Homebrew.

## 1. Why, and what success looks like

Goal Cup runs 500 to 1,500 coding agents at once, each for up to 168 hours, on participants' own Macs.
The server has three jobs, in order of importance:

1. **Keep every trace, safely and cheaply.** Nothing is lost when a participant's network or our server
   has a bad hour. If only this works, the event works.
2. **Live numbers during the week:** agents running now, tokens used, tool calls made, refreshed every
   few minutes. These feed the agentcommunity.org website (another repo), posts by our own agent, and
   press.
3. **Clean data later** (a separate project after the event): decode the raw traces into tidy categories
   for analysis and LLM review. The raw traces are kept untouched, so this can wait.

**Success:** on 6 November a participant connects their tracer, starts inside the 24-hour window, and for
168 hours their trace lands on our side every hour, with a signed receipt each time. The website shows
live totals. Nobody is paged at 3 a.m. The bill stays in the low hundreds of dollars.

**Anti-goals:**
- A server that needs babysitting.
- A bill in the thousands.
- Raw traces ever public.
- Anything that makes recording on the participant's machine wait on the network.

**Constraints:**
- The v5 chain format is frozen.
- The tracer's daemon stays small and network-free.
- "Safe and 100% working code, without magic" (owner).
- About 12 days to a working end-to-end path, because the 168-hour dry run starts around 28 October.

## 2. The shape

| Part | Holds | Why |
|---|---|---|
| **Upload Worker** (Cloudflare Workers, paid plan) | No state. Checks the token, checks each upload continues where the last one ended (by its headers), streams the bytes into R2, records them in Supabase in one atomic step, returns a signed receipt. | No servers to run, scales by itself, sits next to R2. Small enough to write in TypeScript *(my call; see section 5)*. |
| **R2** (one private bucket) | Raw trace chunks, compressed, one immutable object per chunk. | About $15 per TB per month; reading back out is free. A bucket lock makes stored traces undeletable for 180 days, and Data Access Logs record who read what. |
| **Supabase** (the existing agentcommunity project, **Pro plan**) | Members and auth (already there). In a separate `goalcup` schema: the events, tokens, runs, chunk index, heartbeats, outages and verify results. | Members and consent already live there. |
| **Verify job** | Runs our own `goalcup-verify` over each run's stored chunks: hourly while the run is open, and in full at seal. | The record hash is computed over a canonical form that only our Rust code reproduces exactly, so the real check must be the real verifier. |
| **Live numbers file** | `live.json`, rebuilt every minute by a scheduled Worker and served from cache. | The website never queries the database directly, so a press spike can't slow uploads or the community site. |

The tracer talks only to the Worker. The website reads `live.json`, plus a member's own run status
through Supabase with row-level security.

## 3. The participant's path, as the server sees it

1. **Joining, consent and the tracer download** happen on the website. Before a token can be issued, the
   site records that the member accepted the current Goal Cup terms. The Worker checks this flag when a
   run opens.
2. **Connect.** A signed-in member clicks "connect a tracer" and gets a token, shown once. They run
   `goalcup login` and paste it. The tracer stores it in its config directory (mode 0600), never in the
   harness's environment, never logged, never in a trace. Supabase stores only its hash.
   - The token stays valid until the member revokes it on the site.
   - Tokens are minted only by a server-side database function, never by inserting rows directly.

   *(My call: a pasted token for 6 November. The browser-approval flow in
   `docs/registration-and-auth-design.md` can replace it later without changing any upload call.)*
3. **Start.** `goalcup start` calls `POST /v1/runs` **before the agent launches**. The Worker:
   - checks the token and the terms flag;
   - picks the event (competition, dry run or practice; section 9);
   - checks the start window and that the account has no other open run (a unique index makes two
     simultaneous starts impossible);
   - creates the run.

   **The server issues the `entry_id`**, stamps `official_start` from its own clock, and sets
   `deadline_at` = `official_start` + 168 hours. It returns a **signed start receipt** in the verifier's
   existing format (`StartReceipt`, with `started_at` and `deadline_at`). The tracer writes that
   `entry_id` into every record, so the trace is bound to this server run and can't be a recording made
   earlier. If the Worker refuses or can't be reached, nothing starts and the message says why.
4. **Run, up to 168 hours.**
   - The first chunk uploads as soon as the agent launches.
   - Then one chunk every hour, or as soon as 32 MB (compressed) is waiting.
   - A heartbeat every 5 minutes.
5. **If the run stops** (laptop crash, reboot, the agent process dies), the participant brings it
   back:
   - they run `goalcup continue`;
   - the tracer reattaches to the **same server run**: same official start, same deadline, and the clock
     kept running;
   - the server issues a new `entry_id` for this next segment, and the tracer relaunches the harness
     on its own session (Codex and Claude Code both resume sessions) with the message "keep going";
   - the record shows the gap and that message, which the rules allow.

   **Notifications:**
   - if the server hears nothing from an open run for 20 minutes, it emails the participant: "your
     run stopped; run `goalcup continue`";
   - *(nice to have)* a small login item on the participant's machine notices an interrupted run after
     a reboot and shows a notification that leads to `goalcup continue`.
6. **The deadline.** At `deadline_at` the tracer stops the agent and seals, and the run is over.
   - **The submission is the participant's GitHub repo.** Its URL is given when the run opens, and at
     `deadline_at` the server records the latest commit on its default branch through the GitHub API.
     That commit is what judges review; later pushes don't count.
   - The tracer also notes the local repo's latest commit at the deadline, as a cross-check.
   - Uploads that arrive late are accepted for 48 hours, but only records time-stamped before
     `deadline_at`. That way a participant whose network dropped in the last hours doesn't lose the
     evidence of those hours, and no work after the deadline counts. *(The owner's rule is "when the
     timer is up it's over"; accepting late delivery of earlier recordings is the author's reading of it,
     flagged for confirmation.)*

## 4. The calls

All calls use HTTPS with `Authorization: Bearer <token>` and are versioned under `/v1`. Every call
checks that the run belongs to the token's member.

| Call | What it does |
|---|---|
| `POST /v1/runs` `{connector, harness_version, tracer_version, repo_url, chain_head_at_create}` | Opens a run (section 3). Returns `{run_id, entry_id, official_start, deadline_at, start_receipt}`. Refuses unsupported tracer or connector versions. |
| `POST /v1/runs/{id}/continue` `{previous_entry_id, chain_head_at_create}` | After a stop: issues the next segment's `entry_id` for the same run, before the agent relaunches. Refused after `deadline_at`. |
| `GET /v1/runs/{id}` | The server's position: `{offset, seq, head, status}`. Used to resume after a crash. |
| `POST /v1/runs/{id}/heartbeat` | Every 5 minutes: `{at, seq, head, offset_local, tokens_in, tokens_out, tool_calls, model_requests, outage_open}`. The server stamps its receive time on `(seq, head)`, which commits the participant to that chain position before the bytes arrive. Totals are cumulative. |
| `PUT /v1/runs/{id}/chunks/{offset}` + headers `{length, sha256, first_seq, last_seq, prev, head}`, body = zstd bytes | Stores bytes `[offset, offset+length)` of `chain.jsonl`, whole lines only. Returns a signed **acknowledgement** in the verifier's existing format (`AckCheckpoint` in `acks.jsonl`). |
| `POST /v1/runs/{id}/outages` | Outage windows the proxy already tracks, sent with chunks and at seal. Server-measured heartbeat gaps are the authority; these add detail. |
| `POST /v1/runs/{id}/seal` `{seal_json, baseline_json}` | Last call. Triggers the full verify job. |

**Acceptance of a chunk, step by step.**
1. Reject early, before reading the body, if:
   - the token is wrong;
   - the run isn't open, or is past its grace period;
   - `Content-Length` is over 32 MB;
   - the run is over its storage quota;
   - the per-token rate limit trips;
   - the maintenance switch is on.
2. Check the headers: `offset` is the run's next offset, and `prev` equals the stored head.
3. **Stream** the body into R2 with a create-only write under `traces/{event}/{run_id}/{entry_id}/{offset:016}-{sha256}.zst`.
   R2 itself checks the sha256 and refuses to overwrite. The body is never held in memory.
4. One Postgres function atomically advances the run's offset and head, and inserts the chunk row, only
   if they still match what step 2 saw.
5. Sign and return the acknowledgement.

A retry of a chunk the server already has returns the same stored acknowledgement with `200`, so a lost
reply never leaves a hole.

**When the full check later fails** (section 5), the chunk is kept, the run is flagged for a human, and
uploads continue. A bad chunk never strands a run for the rest of the week.

**Backoff.** The tracer retries with exponential backoff (from 60 s up to 15 min, with jitter) and obeys
`Retry-After`. The server can tell every tracer to back off (the maintenance switch) without losing data,
because recording never waits.

## 5. Checking, and the receipts

- **Every chunk, at upload:** cheap header checks only (offset, `prev`, length, the sha256 R2 verifies).
- **Every hour, and at seal:** the verify job reassembles the run from R2 and runs `goalcup-verify`. It
  checks the full hash chain, that every record carries the server-issued `entry_id`, that the heads
  committed in heartbeats appear in the chain at the right place, record times against `official_start`
  and `deadline_at`, and late arrivals. Results go to Supabase.
- **Where the verify job runs:** *(my call)* on an organizer's always-on machine or a small VPS, pulling
  from R2, for 6 November. Cloudflare Containers can host it later. It runs the same binary participants
  run.
- **Receipts:** the start receipt and acknowledgements reuse the verifier's existing signed formats, so
  verifier work shrinks to pinning keys.
  - They're signed with an Ed25519 key held only as a Worker secret.
  - **Two public keys** (active and standby) are pinned in the verifier release, so a key can rotate
    mid-event without re-releasing the tracer.
  - The key is made offline. A short runbook covers rotation and compromise.
  - The old published placeholder key is no longer trusted (tracer hardening, already in progress).
  - Each receipt names its event and mode, so practice and dry-run receipts can never pass as
    competition ones.

## 6. Live numbers

**Counted on the participant's machine.** Before each heartbeat, the tracer runs the verifier's existing
decoders (`usage` and `tool_call`) over the records written since the last count. Each model request and
tool call counts once, from the model traffic only.

**Sanity checks on the server:**
- totals may only grow;
- a cap per 5-minute interval;
- a cross-check against bytes actually uploaded.

A run counts as "running" only if a chunk was accepted in the last ~70 minutes, and flagged runs are left
out. These numbers are for the website and press, not judging; the raw trace stays the ground truth.

**Publishing.** A scheduled Worker rebuilds `live.json` every minute from one Postgres query. It holds
field totals, agents running now, runs started and runs sealed. It's served with caching, and browsers
never touch the database.
- Per-run figures are public (the owner's call); the terms say so. The site may show them every 5
  minutes or as daily updates.
- No client-supplied text ever appears in public data.

## 7. Data model (Supabase, schema `goalcup`, not exposed to the public API)

| Table | Key columns |
|---|---|
| `events` | `id`, `mode` (competition, dry_run, practice), `start_window_opens`, `start_window_closes`, `run_hours` (168), `grace_hours` |
| `tokens` | `member_id`, `token_hash`, `created_at`, `revoked_at` |
| `runs` | `id`, `event_id`, `member_id`, `official_start`, `deadline_at`, `status` (open, sealed, abandoned, flagged), `repo_url`, `final_commit`, `final_commit_at`, `last_heartbeat_at`, latest totals, versions |
| `entries` | `id` (the `entry_id`), `run_id`, `segment` (0, 1, 2 …), `opened_at`, `upload_offset`, `head`, `last_chunk_at`, `sealed_at` |
| `chunks` | `entry_id`, `offset` (unique per entry), `length`, `sha256`, `first_seq`, `last_seq`, `prev`, `head`, `r2_key`, `received_at`, `ack` |
| `heartbeats` | `run_id`, `received_at`, `seq`, `head`, totals |
| `outages` | `run_id`, `first_failure`, `last_failure`, `failures`, `recovered`, `source` (proxy or server) |
| `verify_results` | `run_id`, `checked_through_offset`, `exit`, `summary`, `checked_at` |

**Rules:**
- Row-level security is enabled in the same migration that creates each table, and default grants to the
  public keys are revoked.
- Members can only read their own runs, through one read-only function.
- The Worker calls one database function per API call, as a dedicated role limited to this schema
  *(my call; not the community-wide service key)*.
- A unique index on (`member_id`, open runs) enforces one open run per account.
- Tests use the public key and a second member to prove that neither can see or change anything they
  shouldn't.

## 8. Who can read what

- **Participants:** their own run's status and totals, and a download of the server copy of their own
  trace.
- **The public, our agent, the press:** `live.json` only.
- **Organizers:** everything, with their own accounts.
- **Researchers:** named people with expiring read-only credentials. R2 Data Access Logs record reads,
  on a best-effort basis. The redacted copy only, and no member IDs.
- **Raw traces are never public.** They're kept 12 months, then archived; deletion on request.

What the tracer redacts happens on the participant's machine before anything is written or uploaded.
There is no second redaction pass on the server; `RULES.md` is reworded to say so (section 15).

## 9. Practice and dry runs

`events` has three modes, and the Worker picks the event by the time window and the token's eligibility:
- **practice:** open any time, runs cut short, for participants to test their setup;
- **dry_run:** the 28 October run with friendly teams;
- **competition:** 6 November.

Only competition runs appear in `live.json`, and receipts carry the mode. (Confirmed by the owner.)

## 10. Local disk

The owner wants local data older than 2 hours deleted once the server holds a **verified, signed**
acknowledgement for it, so a 168-hour run never fills a laptop.
- The tracer checks the acknowledgement's signature and head before pruning.
- The v5 format doesn't change. `goalcup-verify` on a pruned local copy says plainly "pruned: verify
  against the server copy".
- The disk reservation becomes rolling: a few hours of headroom instead of 10 GiB up front.

## 11. Operations

- **Deploys:** Worker versions with gradual rollout and `wrangler rollback` to any recent version. Schema
  changes are add-only during the event week.
- **Monitoring with nobody on call:**
  - an hourly scheduled check posts to Slack or Discord when runs go quiet, chunks are flagged, uploads
    lag or errors spike;
  - a daily digest;
  - an external uptime check, Cloudflare and Supabase cost alerts, and a simple status page.
- **Switches:** a maintenance switch (back off, retry later) and an organizer pause on new starts.
- **Before 28 October:** a staging copy of everything, and a load test on deployed infrastructure, not
  local emulators. Test simultaneous uploads, the hour-mark burst, and catch-up after a simulated outage.
- **Runbooks:** receipt-key rotation and compromise, a bad deploy mid-event, Supabase read-only, R2
  outage, a data breach.

## 12. Cost at 1,500 agents for the event month

| Item | Estimate |
|---|---|
| Workers paid plan and requests (about 3.3 million) | about $6 |
| R2 storage (assume about 10 TB compressed; tonight's 8-hour run replaces this guess) | about $150 |
| R2 writes and reads | a few dollars |
| Supabase Pro | $25 plus usage |
| Verify-job machine | $0–20 |

**Total: about $185–200 for the event month.** After 12 months, traces move to archive storage (about
$1 per TB per month), possibly kept for good.

## 13. What the tracer needs (small PRs, in order)

1. `goalcup login`, and token storage that's never in the harness environment.
2. Open the run before launch: adopt the server's `entry_id`, `official_start` and `deadline_at`, and keep
   the start receipt. Depends on the sleep-proof clock fix (in progress).
3. The uploader in `goalcup start`: first chunk at launch, then hourly or at 32 MB, streaming, backoff
   with `Retry-After`, idempotent retries, acknowledgements kept in `acks.jsonl`, and `goalcup upload
   <entry>` after a crash.
4. Heartbeat with `(seq, head)` and live totals.
5. Outage windows sent with chunks and at seal.
5a. `goalcup continue`: reattach to the same run, a new segment `entry_id`, and resume the harness's own
    session with "keep going" (a connector field for each harness's resume command).
5b. The deadline stop: at `deadline_at`, stop the agent, note the local repo's latest commit, seal.
5c. *(Nice to have)* the login-item notification after a reboot.
6. Pruning after verified acknowledgements, and the rolling reservation.
7. Verifier: pin two keys, report "pruned", report lateness.

## 14. What the server needs

1. A spike first, one day:
   - stream a 32 MB body into R2 from a Worker without buffering;
   - 4–8 parallel uploads on a deployed preview;
   - confirm the bucket lock doesn't block a later move to a colder class.
2. Supabase migrations (section 7), with the row-level security tests.
3. The upload Worker: six calls, header checks, create-only R2 writes, the atomic commit function and
   signing.
4. The verify job and `live.json` builder.
4a. The quiet-run email (20 minutes without a heartbeat), sent through the website's existing mail setup.
4b. At each run's deadline, a scheduled job records the repo's latest default-branch commit through the
    GitHub API (a token with read access to public repos).
5. Monitoring, switches, staging, and the load test.
6. The receipt keys, made offline, with two public keys pinned in the verifier.

## 15. The owner's answers (2026-10-13)

1. **Crash or reboot:** the run continues with `goalcup continue` (section 3, step 5), with an email after
   20 minutes of silence and an optional notification after reboot. A run is never restarted from zero
   inside the window. One open run per account; more agents need more accounts.
2. **Retention:** keep raw traces for 12 months, then move them to cheap archive storage, possibly
   forever if it stays cheap. Deletion on request. This goes in the terms before consent.
3. **Second redaction on the server:** not built. `RULES.md` is reworded to "secrets are removed on your
   machine before anything is saved or sent".
4. **Practice and dry runs:** yes, as in section 9. Funding practice tokens is a separate question.
5. **Public per-run numbers:** yes, shown publicly (the site may choose to show daily updates). The
   infrastructure supports 5-minute updates. The terms say run statistics are public.
6. **Deadline:** at 168 hours it's over. The submission is the repo's latest commit on its default branch,
   recorded by the server at the deadline. Late delivery of records made before the deadline is accepted
   for 48 hours *(author's reading; confirm)*.
7. **Logistics:**
   - Cloudflare: the existing paid ($5) account. Domain: e.g. `ingest.agentcommunity.org`.
   - Supabase: the existing paid project (about 30,000 members).
   - The **website repo owns** auth, the `goalcup` schema and its migrations, token minting, the connect
     page, the terms checkbox and the live display; it's already connected to R2 and Cloudflare.
   - This repo owns the tracer and the upload Worker.

## 16. Not before 6 November

- Streaming the trace live.
- Server-side decoding or cleaning (job 3).
- A custom admin dashboard: organizers use Supabase's table view.
- The browser-approval connect flow.
- Teams.
- More than one open run per account.
- A native Windows build (Windows participants use WSL2).
- Durable Objects, Queues, D1, Analytics Engine, Hyperdrive, Pipelines/Basin and Containers.

## 17. Left to the builder

Exact chunk boundaries and compression settings (no long-window zstd); backoff constants; the pruning
mechanism (within section 10); code structure; migration tooling; where the verify job is triggered
from.
