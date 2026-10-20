# Event readiness plan (20 Oct to 6 Nov)

**Goal:** every one of about 1,500 runs holds the invariants below for 168 hours, on machines we do
not control, and we can show it with evidence that does not depend on trusting the authors.

**Dates (UTC):** dry runs from 23 Oct. Full-length 168 h rehearsal starts by **27 Oct** so its
findings land before the **4 Nov** feature stop. Last release 6 Nov 12:00. Start window 7 Nov.

**Method:** invariants first, pre-mortem second, measure from raw evidence third, change last.
Every change names the invariant it protects, its pass/fail line (written before the test), the
fault-injection tool that tests it, and an independent oracle. A change that protects no invariant
is cut. Two weeks before an event, every change is also a risk: rank by risk removed per risk added.

**Owner decisions (20 Oct):** server errors never stop a run; smart but simple upload and storage;
1 GiB always reserved for the recorder; a server per-run size limit; a long-test sampler; H-29
cheap path (detect, warn, clean after the event); the product is **Goal Cup Proxy** and the inner
component is the **recording proxy**.

Rules from `AGENTS.md` hold: record raw (D3), no harness names in core, one test per behaviour,
promise tests P1 to P7 never deleted, `gate.sh fast` before every commit (as a normal user: in a
root container 7 permission tests fail because `chmod 000` does not stop root).

---

## 1. Invariants

| ID | Invariant | Today |
|---|---|---|
| I1 | A run stops only at its deadline (server `stop` answer or local countdown) or by the participant | **Broken:** a bare 401/410 heartbeat stops the run (`crates/goalcup-cli/src/upload/uploader.rs:1131-1143`) |
| I2 | Every recorded byte reaches the server exactly once, or the gap is recorded and visible | Holds in tests; unmeasured past minutes on a real harness |
| I3 | No gap is ever silent, including on a full disk | **Broken (inferred):** ENOSPC latches the writer and discards later records, gaps included (`crates/goalcup-daemon/src/runtime/capture/writer.rs:152-155`) |
| I4 | Local disk use stays bounded: reserve plus about 2 GiB in steady state, and never pushes the machine into ENOSPC by itself | **Gap:** pruning ignores free disk (`crates/goalcup-cli/src/run/watch.rs:273`) |
| I5 | No secret value reaches research data | **Broken:** split keys in reply streams are recorded raw (H-29, all key shapes, breaks promise P1) |
| I6 | Server storage per run is bounded and visible | **Missing:** no quota (`docs/SERVER-DESIGN.md:127`); 180-day lock makes uploads undeletable |
| I7 | Memory, open files and CPU stay flat over 168 h | Unmeasured |

## 2. Pre-mortem: "it is 15 Nov and the event failed"

| # | Story | Invariant | Covered by |
|---|---|---|---|
| 1 | A Worker deploy returns 404 for one route; every run stops within 5 min | I1 | WP-A |
| 2 | A participant fills the disk on purpose; the run goes unrecorded for hours and nobody can tell | I3 | WP-C, RULES gap policy |
| 3 | A laptop with 20 GB free runs a busy agent through a 24 h outage and hits ENOSPC | I3, I4 | WP-B, WP-C |
| 4 | A model echoes a login token; researchers receive it | I5 | WP-E |
| 5 | One run uploads 500 GB of junk into the locked bucket | I6 | WP-D |
| 6 | The daemon leaks memory and dies on day 4 | I7 | WP-F soak |
| 7 | Hermes snapshots grow quadratically and saturate a slow uplink | I2, I4 | WP-0 measure, WP-B |
| 8 | 1,500 runs at the new upload pace exhaust Supabase connections | I2 | WP-F fleet load |
| 9 | The mass rename lands late and breaks the release | all | WP-H timing |
| 10 | A fix lands untested on real harnesses and regresses capture | all | canary release, ladder |

## 3. WP-0: Measure from raw evidence first (before changing behaviour)

Each measurement drives a decision. None needs a paid model run.

| Raw material | Measure | Decides |
|---|---|---|
| Private chain parts of the 18 and 19 Oct Codex runs | Bytes by record kind and stream; envelope vs payload; gaps between records (quiet periods) | WP-B idle threshold; whether framing dominates |
| Same chains, `zstd -3` and `--long=27` | Real compression ratio | WP-B long window; WP-D cap |
| Staging R2 object sizes vs `chunks` index lengths | Fleet compression ratio, for free | WP-D cap |
| Staging Supabase `heartbeats`, `chunks`, Worker logs | ACK lag, error codes actually seen, chunk sizes | WP-A, WP-B |
| Codex and Hermes source (open source) | When login files and session snapshots are rewritten | WP-B Hermes; WP-E guidance |
| Existing real traces, decoded text, gitleaks run | Any secret already present in real captures | WP-E urgency |
| Cloudflare R2/Workers and Supabase limits pages | Requests, rows, connections at 1,500 runs | WP-B pace, WP-D |

Tools: `jq` or Python over chain JSONL, `zstd`, `psql` read-only, `gitleaks detect --no-git`.

## 4. Work packages

Each WP lists: invariant, change, pass line (written now), fault tool, independent oracle.

### WP-A: Server errors never stop a run (I1, S, first)

- **Change:** only a heartbeat `stop` answer or the local deadline ends a run. Every error, any
  status, means keep recording and retry with backoff. After 3 refusals of the same kind, say it
  once in the terminal and `status`. Replacement still works: a replaced run gets a 200 `stop`
  with `reason: "replaced"` (`server/worker/src/runs.ts:102-107`), not an error.
- **Replace tests** `upload_chunks.rs:945` and `:1323` (they encode the old behaviour) with:
  a server answering 401/404/410/500 for an hour does not stop the run; afterwards every record
  arrives once. Add: server unreachable from minute 1, the run still stops at its deadline.
- **Pass:** 0 stops caused by errors across a scripted sequence of every status code; deadline
  accuracy within 60 s with the server unreachable.
- **Fault tool:** stand-in server returning scripted codes; staging `MAINTENANCE=on`; `toxiproxy`.
- **Oracle:** `cargo-mutants` on the stop logic (no surviving mutant that stops on an error);
  rebuilt R2 bytes equal the local chain after the outage.

### WP-B: Smart upload and storage (I2, I4, S-M)

- **Change:** upload when the chain has been quiet for N seconds with at least 1 MiB waiting
  (N from WP-0 gap data, start at 15 s); size trigger 64 MiB; longest gap 10 min; poll 5 s.
  Delete acknowledged parts at once when free space beyond the reserve is under 5 GiB; otherwise
  keep them up to 2 h as a safety copy. zstd long window if WP-0 shows a gain. One `host.log` line
  per chunk with raw and compressed bytes.
- **Not doing:** on-disk compression, snapshot dedup, a new part grammar.
- **Pass:** steady-state local chain under 2 GiB at 1 GB/h; ACK lag median under 2 min while
  active; with 6 GiB free and a 2 h outage, no ENOSPC; PUT rate under 20 per hour per run.
- **Fault tool:** `tc netem` or `toxiproxy` (slow uplink 1 Mbit/s, 10 min cuts); a loop-device
  ext4 image of fixed size.
- **Oracle:** the `zstd` CLI decompresses every chunk; `fetch-run.sh` rebuild equals local;
  `k6` fleet load of 1,500 simulated runs at the new pace against staging stays within Worker
  and Supabase limits.

### WP-C: Disk full never stops the agent and is never silent (I3, M)

- **Order:** first T25: fill a fixed-size ext4 image mid-run and record what happens today.
- **Change:** hold 1 GiB for the recorder for the run's life. On a write failing for space:
  release it, write a `StorageFull` gap, warn in the terminal and `status`, keep the agent
  running, resume recording when space frees, rebuild the hold when possible. Seal always writable.
- **Rule work:** RULES says how judges treat a `StorageFull` gap (owner decision), so filling the
  disk on purpose is visible and has a consequence.
- **Pass:** 100% of disk-full drills produce exactly one gap per episode with start and end,
  recording resumes within 60 s of space returning, seal and verifier succeed.
- **Fault tool:** loop-device ext4 image plus `fallocate` from a second process.
- **Oracle:** the verifier reports the gap with its byte span; rebuilt R2 bytes equal local.

### WP-D: Server per-run size limit (I6, M)

- **Change:** `stored_bytes` (compressed) and `chunk_count` per entry in `commit_chunk`; check
  after `requireRun` (`server/worker/src/chunks.ts:38`) against `RUN_MAX_COMPRESSED_BYTES`
  (start 64 GiB; set from WP-0 ratio at 4x the measured worst case). Over: `413 quota_exceeded`;
  the client keeps data and retries (WP-A). Operator notice at 50% and 100%.
- **Where:** website repo migration, SQL mirror, `db-local.ts`, `db-supabase.ts`, `chunks.ts`.
  Staging first; production by the lead agent with owner approval; additive only.
- **Pass:** a run past the cap is refused with no data loss locally; raising the var resumes it.
- **Oracle:** `npm --prefix server/worker test`; Supabase row counts match R2 object counts.

### WP-E: H-29 cheap path (I5, S-M)

- **Change:** verifier scan of decoded, joined assistant text and tool-call arguments for secret
  shapes (shared rules plus the gitleaks list); report kind, count and reference, never the value.
  The post-event export refuses or cleans flagged traces. Docs: H-29 covers all split keys; P1 has
  a known gap until the stream-aware redactor lands after the event. Participant guide: use a
  login you can revoke, rotate after the event.
- **Blind spot:** server cleanup makes cleaned copies; raw R2 objects stay locked for 180 days.
  Rotation is what closes the exposure.
- **Pass:** detects 100% of a corpus of keys split at every boundary across Anthropic, Responses,
  Chat Completions and WebSocket shapes, including real tokenizer splits from captured streams;
  0 findings on existing clean traces; no value ever printed.
- **Oracle:** gitleaks and trufflehog on the same joined text find the same set.

### WP-F: Soak, fleet load and test ladder (I2, I7)

- **Sampler** (`scripts/`, test-only, every 10 min, CSV): `df`, parts and bytes per trace dir,
  ACK lag, `state.json`, RSS and open files of daemon, CLI and recording proxy.
- **Ladder on staging** (Codex macOS first, Linux in parallel, WSL2 if available):

| When | Run | Drills |
|---|---|---|
| 23 to 24 Oct | 2 h | 10 min network cut |
| 24 to 25 Oct | 8 h | sleep and wake; `MAINTENANCE=on` 30 min |
| 25 to 26 Oct | 24 h | reboot plus `goalcup continue`; disk near full |
| from 27 Oct | 168 h | none; sampler only |

- **Canary:** the point release with WP-A and WP-B goes to 3 to 5 dry-run participants for 48 h
  before the 168 h run uses it. If not merged by 26 Oct, run v0.6.0 and repeat 24 h later.
- **Pass (written now):** no stop except the deadline; 0 unexplained gaps; RSS slope under
  1 MiB per hour after the first hour; open files flat; every record on the server once.
- **Oracle:** `fetch-run.sh` rebuild verifies with identical head, count and bytes; an
  adversarial reviewer with no context tries for 2 h to make a run go unrecorded or leak a key.

### WP-G: F-16 matcher (I2 evidence, S)

Port the old E02 cross-check (`agentcommunity/goal-cup`, branch `m2/v05-timezoner-ledger-final`,
`spikes/m2-first-release/e02/early-crosscheck.py`) next to `early-load.py`. Give every load body
unique content; match start-anchored ("the Open must fall within a short time after the item's
start"). Add the "identical body lost outside the gap window" mutant as a test.
**Pass:** all 6 checks pass on a fresh 30 min load and the mutant fails exactly 2.

### WP-H: Rename to Goal Cup Proxy (L, mechanical)

Product = Goal Cup Proxy; component = recording proxy; never a bare "the proxy" for the component
in user text. Terms ID `goal-cup-tracer-2026-10-15` stays (opaque; nobody re-accepts).

| Step | What | Risk | When |
|---|---|---|---|
| 1 | User-facing text | low | right after WP-A to WP-E merge |
| 2 | Code and comments; `tracer_version` to `proxy_version`, server accepts both | low | same PR |
| 3 | Connector comments; regenerate pins | low | same PR |
| 4 | Repo rename `goal-cup-proxy`; test old install and release URLs | medium | next release |
| 5 | DB schema rename with a Worker deploy in one window | medium | with the 26 Oct point release |

Risk view: steps 2 to 5 protect no invariant; the owner chose to do them before the event.

## 5. Event operations

- **Dashboards** (Supabase SQL plus Workers Logs): runs heartbeating in the last 10 min; ACK lag
  distribution (`heartbeats.offset_local - entries.upload_offset`); error codes per minute; runs
  near quota; `StorageFull` gaps reported.
- **Alerts:** heartbeating runs drop more than 5% in 10 min; any 4xx rate spike after a deploy.
- **Kill switch:** `MAINTENANCE=on` (safe with WP-A: runs keep recording).
- **Rollback:** previous Worker version via `wrangler rollback`; client point release ready.
- **On call:** a named person per 8 h shift, 7 to 14 Nov. Deploy freeze except hotfixes, staging
  first.
- **24 h close:** notify participants with a backlog to run `goalcup upload` before close.

## 6. Order

1. 20 to 22 Oct: WP-0 measurements; WP-A; WP-E docs and guidance; WP-F sampler; T25.
2. 22 to 26 Oct: WP-B, WP-C, WP-D on staging, WP-E scan; ladder 2 h, 8 h, 24 h; fleet load.
3. 24 to 26 Oct: canary point release to dry-run participants.
4. 27 Oct: 168 h run starts. WP-G; WP-H steps 1 to 3.
5. 28 Oct to 3 Nov: fix findings; WP-D to production; adversarial review; dashboards and alerts.
6. 4 Nov feature stop; 6 Nov last release and `EVENT_ID` switch.

## Owner decisions (20 Oct, round 2)

- A `StorageFull` gap is a red flag: something was off. Judges decide case by case.
- A trace with a detected secret passes through, flagged. It is cleaned before anyone else gets it.
- WP-H: all five steps before the event. Step 5 (schema) goes in a short window with the 26 Oct
  point release, before the 168 h run; with WP-A merged, runs keep recording through it.

## Where each part runs

| Needs the owner's machine or credentials | Can be built and tested in the cloud |
|---|---|
| WP-0 measurements on private chains | WP-A, WP-B, WP-C code with stand-in tests |
| Staging R2, Supabase and Worker log reads | WP-D Worker and SQL with `npm test` |
| Real harness runs: ladder, drills, canary, 168 h | WP-E verifier scan and split-key corpus |
| macOS sleep and wake; WSL2 | WP-F sampler and `k6` script (runs need staging access) |
| `k6` against staging; production migration | WP-G matcher; WP-H steps 1 to 3 |
| Clean `gate.sh fast` as a normal user | T25 only if the container allows a loop device |
