# Event readiness plan (20 Oct to 6 Nov)

**Goal:** a 168-hour run that never stops because of our server, never fills a disk, never goes
silently unrecorded, and keeps model-echoed secrets out of research data. Then rename the product
to **Goal Cup Proxy**.

**Dates (UTC):** dry runs from 23 Oct. Start the full-length 168 h rehearsal by **27 Oct** so its
findings land before the **4 Nov** feature stop. Last release 6 Nov 12:00. Start window 7 Nov.

**Owner decisions (20 Oct):**

1. Server errors never stop a run. Only the server's normal `stop` answer or the local deadline
   stops one. Keep recording locally, retry, hotfix the server.
2. Storage: smart and safe, not over-engineered. Upload more often, including between model calls.
3. Disk full: never stop the agent. Keep **1 GiB always reserved** for the recorder. Gap visible.
4. Server per-run size limit: yes.
5. Long-test sampler: yes, for tests.
6. H-29: cheap path now (detect, warn, clean on the server after the event). No stream rewrite
   before the event.
7. Name: **Goal Cup Proxy**. The inner component is always the **recording proxy**.

Research behind this plan: verified file:line evidence for each item below. Rules from
`AGENTS.md` hold throughout: record raw (D3), no harness names in core, one test per behaviour,
promise tests (P1 to P7) are never deleted, `gate.sh fast` before every commit.

Note: in a root container, 7 permission-based tests fail (`chmod 000` does not stop root). Run the
gate as a normal user.

---

## WP-A: Server errors never stop a run (S, first)

**Today:** a heartbeat answered 401 or 410 stops the run, agent included
(`crates/goalcup-cli/src/upload/uploader.rs:1131-1143`). A 401, 404 or 410 on any call ends the
upload for the rest of the run (`upload/policy.rs:248-261`). The Worker's JSON error code is never
checked, and a missing route also answers 404 (`server/worker/src/index.ts:66`). One bad deploy or
database restore could stop every run within about 5 minutes.

**Change:**
- Only two things stop a run: a heartbeat answer that says `stop`, and the local deadline
  (`LocalDeadline`, WP11). Every error, any status, is "keep recording, try again with backoff".
- The upload never ends itself during the run. After the run, the wrap-up and `goalcup upload`
  carry on as now.
- After 3 consecutive refusals of the same kind, say it once in the terminal and in `status`
  ("the server refuses this run's uploads; recording continues locally").

**Tests:** replace `upload_chunks.rs:945` `a_401_ends_the_upload_at_once` and `:1323`
`a_401_or_410_on_a_heartbeat_stops_the_run_and_names_it` with the new promise: a server answering
401/404/410/500 for a while does not stop the run; once it answers normally, every record arrives
exactly once. Keep `upload_e2e` outage tests.

**Blind spot:** a run that was really replaced or revoked keeps running locally. Harmless: it
cannot upload, and the participant is told.

**Ops:** deploy freeze 7 to 15 Nov except hotfixes; every hotfix goes to staging first;
`MAINTENANCE=on` stays the safe switch.

## WP-B: Smart upload and storage (S-M)

**Today:** a chunk goes every 30 min or at 256 MiB waiting (`policy.rs:79-82`). Acknowledged parts
are deleted 2 h later (`upload/prune.rs:26`), or at once only when the chain alone is past 3 GiB
(`run/watch.rs:273`). Free disk is never considered. The 2 h has no written reason; it is a safety
copy in case the server's position ever moves back (for example a database restore).

**Change:**
- **Upload between model calls.** Send what is waiting when the chain has been quiet for about
  15 s (a model call has finished), with at least 1 MiB waiting. Generic: it watches the chain's
  byte count, not any harness format. Poll every 5 s instead of 30 s.
- **Size trigger 64 MiB** instead of 256 MiB. **Longest gap 10 min** instead of 30 min.
- **Delete by disk, not only by clock.** Keep acknowledged parts as a safety copy while disk is
  comfortable (up to 2 h). When free space beyond the reserve drops under `LOW_DISK_BYTES`
  (5 GiB), delete acknowledged parts at once. The part holding the last acknowledged record is
  still never deleted.
- **zstd long window** (`uploader.rs:62`, window log 27). Lossless; plain `zstd -d` reads it.
  Synthetic Hermes snapshots: 163 MB raw, 2.1 MB today, 0.64 MB with the long window.
- **One `host.log` line per chunk:** raw bytes, compressed bytes. This finally measures the
  compression ratio.

**Not doing (over-engineering for now):** compressing parts on disk, snapshot dedup, a new part
grammar.

**Cost check:** about 5 to 15 PUTs per hour for a busy run instead of about 4. Fine for R2 and
the `chunks` table at 1,500 runs. The server has no chunk rate limit, so nothing to change there.

**Tests:** idle trigger fires after a quiet chain and not during a busy one; low free disk deletes
acknowledged parts before 2 h; the last acknowledged part survives; long-window chunks decompress
in the stand-in and in `scripts/fetch-run.sh`.

## WP-C: Disk full never stops the agent (M)

**Today:** the 3 GiB reserve is a floor given back to anyone, the agent included
(`run/reserve.rs:3-9,21`). On a real ENOSPC the daemon writer latches a failure and discards every
later record, gap records included (`crates/goalcup-daemon/src/runtime/capture/writer.rs:152-155`).
The harness likely keeps running unrecorded (inferred). An agent could fill the disk on purpose to
go unrecorded.

**Change:**
- **1 GiB is always held** for the recorder and never given back to the disk while the run lives.
- When a write fails for space: release the 1 GiB, write a `StorageFull` gap, warn in the terminal
  and `status`, and keep the agent running. Resume recording automatically once space frees.
  Rebuild the 1 GiB hold when it can.
- The seal must always be writable (it is outside the reservation today,
  `docs/CHEAT-CHECKLIST.md:185-187`).

**Order:** run **T25 first** (literal ENOSPC on a small disposable ext4 image) to see what actually
happens today: seal, `continue`, verifier. Then build the change.

**Tests:** fill the volume mid-run: one `StorageFull` gap, the agent still running, recording
resumes after space frees, seal and verifier succeed, the gap is reported.

**Blind spot:** start needs 1 GiB more free space than today on small disks. `doctor` says so.

## WP-D: Server per-run size limit (M)

**Today:** no per-run quota (`docs/SERVER-DESIGN.md:127`). The Worker stores the client-claimed
length only and never decompresses. Everything under `traces/goalcup-cup-2026/` is locked for
180 days, so anything uploaded cannot be deleted.

**Change:**
- Add `stored_bytes` (sum of compressed Content-Length) and `chunk_count` to the entry in
  `commit_chunk`. Check after `requireRun` (`server/worker/src/chunks.ts:38`) against a
  `RUN_MAX_COMPRESSED_BYTES` var, generous (64 GiB, about 4x the estimated worst case).
- Over the limit: `413 quota_exceeded`. The client keeps the data and retries (WP-A), so an
  organiser can raise the var and nothing is lost.
- Operator notice when a run passes 50% and 100%.

**Where:** website repo migration (agentCommunity_PAGE `supabase/migrations/`), the SQL mirror
`server/worker/sql/goal-cup-tracer-upload.sql`, `db-local.ts`, `db-supabase.ts`, `chunks.ts`.
`npm --prefix server/worker test`. Staging first, then production (lead agent applies, owner
approves). Additive only.

## WP-E: H-29 cheap path (S-M)

**Facts:** the provider login header is never stored (headers are skipped unread,
`crates/goalcup-cli/src/proxy/http.rs:726`; pinned by `m2_http_relay.rs:662`). H-29 is a secret the
model streams in its reply: the stream splits it into pieces with JSON between them, so no rule
sees it whole. It is wider than written: every key shape leaks this way, not only PEM (reproduced
with `spikes/redact.py` for `sk-ant-`, `AKIA`, `ghp_` and split PEM). The same text is redacted in
the native transcript and in the next request. This is a known gap in promise P1.

**Change:**
- **Verifier scan.** Over the decoded, joined assistant text and tool-call arguments, flag
  secret-shaped text. Report kind, count and source reference, never the value. Use the existing
  shared rules plus the gitleaks rule list (MIT) for wider shapes (Amp tokens have no rule today).
- **Export gate.** The post-event research export (clean-up 14) refuses or cleans a flagged trace.
- **Docs.** Widen H-29 in `TODO.md` and `docs/CHEAT-CHECKLIST.md` to all split keys. Participant
  guide and RULES: the agent can read the harness login; use a login you can revoke and rotate it
  after the event.
- **After the event:** the stream-aware redactor (join text inside each SSE/WebSocket event before
  matching; about 4 shared stream shapes; does not slow the agent).

**Blind spot:** server cleanup after the event can only produce cleaned **copies**. The raw objects
in R2 are locked for 180 days and stay readable by organisers. Participants rotating their login
is what actually closes the exposure.

**Tests:** the verifier flags a key split across Anthropic, Responses, Chat Completions and
WebSocket deltas; it never prints the value; a clean trace has zero findings.

## WP-F: Long-run sampler and test ladder (S, then calendar time)

**Sampler** (`scripts/`, test-only): every 10 min record `df`, parts and bytes per trace dir, ACK
lag (last `seq` in `acks.jsonl` against `status --json`), `state.json`, RSS and open files for
daemon, CLI and proxy (`/proc/<pid>/fd` on Linux, `lsof -p` on macOS). Plain CSV.

**Ladder on staging, Codex on macOS first, Linux in parallel, WSL2 if a machine exists:**

| When | Run | Drills during the run |
|---|---|---|
| 23 to 24 Oct | 2 h | wifi off 10 min |
| 24 to 25 Oct | 8 h | sleep and wake; staging `MAINTENANCE=on` for 30 min |
| 25 to 26 Oct | 24 h | reboot, then `goalcup continue`; disk near full on a test volume |
| from 27 Oct | 168 h | leave alone; sampler only |

Use a point release with WP-A and WP-B for the 168 h run if they are merged by 26 Oct; otherwise
run v0.6.0 and repeat a 24 h run after they land.

**Pass:** no stop except the deadline; zero unexplained gaps; local disk stays under reserve plus
about 2 GiB; RSS and open files flat; every record on the server once.

## WP-G: F-16 matcher (S, before the 8 h synthetic qualification)

The old matcher is a test script (old repo `agentcommunity/goal-cup`, branch
`m2/v05-timezoner-ledger-final`, `spikes/m2-first-release/e02/early-crosscheck.py`). It failed 2 of
its 6 checks because identical test bodies let each item take its neighbour's stream.

**Change:** port the script next to `spikes/m2-first-release/e02/early-load.py`; give every load
body unique content (simplest robust fix); match start-anchored ("the Open must fall within a
short time after the item's start"). Add the "identical body lost outside the gap window" mutant as
a regression test.

## WP-H: Rename to Goal Cup Proxy (L, mechanical)

723 uses of "tracer" in 102 files. "proxy" already appears about 1,900 times for the recording
proxy, so the rule is: product = **Goal Cup Proxy**, component = **recording proxy**, never a bare
"the proxy" for the component in user text.

| Step | What | Risk |
|---|---|---|
| 1 | User-facing text: README, guides, skills, help, refusal messages, `install.sh` text | low |
| 2 | Code and comments; wire field `tracer_version` becomes `proxy_version`, server accepts both | low |
| 3 | Connector comments; regenerate pins in the same commit | low |
| 4 | Repo rename to `goal-cup-proxy`; test old install and release URLs redirect | medium |
| 5 | DB schema `goal_cup_tracer` to `goal_cup_proxy` with a Worker deploy in one window | medium |

- The terms string `goal-cup-tracer-2026-10-15` is an opaque ID: keep it, so nobody re-accepts.
- Timing: steps 1 to 3 as one PR right after WP-A to WP-E merge (a mass rename conflicts with
  every open branch). Step 4 with the next release. Step 5 is invisible to users; do it before the
  dry-run traffic grows or after the event, not during the 168 h rehearsal.

---

## Order

1. 20 to 22 Oct: WP-A, WP-E docs and guidance, WP-F sampler, T25 measurement.
2. 22 to 26 Oct: WP-B, WP-C, WP-D (staging), WP-E verifier scan. Ladder runs 2 h, 8 h, 24 h.
3. 26 Oct: point release for the long run.
4. 27 Oct: start the 168 h run. WP-H steps 1 to 3, WP-G.
5. 28 Oct to 3 Nov: fix what the long runs find. WP-D to production. WP-H step 4.
6. 4 Nov: feature stop. 6 Nov: last release and `EVENT_ID` switch.

## Open owner questions

- WP-H step 5 (schema rename): before 23 Oct, or after the event?
- WP-E: should a flagged trace be blocked from research export, or cleaned and kept?
