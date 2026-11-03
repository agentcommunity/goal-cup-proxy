# N. Architecture choice and owner answers

Lane N, round 2. Thursday 29 October 2026. Review only: nothing was built, run or tested.
Plan: [2026-10-30-cut-first-plan.md](../2026-10-30-cut-first-plan.md). Inputs: lanes N1 (architecture)
and N2 (owner answers).

Base: the working tree at `/home/user/goal-cup-proxy` (main 4b226de2 plus the plan and appendices, commit
b19ebd44). The 2-hour run used build a31094c, an ancestor of this tree with no change under `crates/`
since (`git diff --stat a31094c HEAD -- crates` is empty). So every code line cited here is the code
that ran.

Run evidence: `git show origin/smart/tender-clarke-qupne3:docs/research/results/2026-10-30-two-hour-run.md`
and the folder beside it on that branch. Cited as "run report" and "run: <file>" (for example
`final-reconcile.txt`, `host.log`, `verify-local.txt`). The report exists on that branch only, not on main.

"src" is the part of a file above its last `#[cfg(test)]`, counted with `wc -l`. New-code and day
figures are estimates, plus or minus 50 percent. "Unverified" means nobody measured it.

## Summary

1. The owner's idea (register the AI endpoints, record that traffic through a local proxy, send it to
   the server, check against local logs) is the shape the code already has. The run proves that core
   works: before 15:43 the wire matched Codex's own record in every 10-minute bucket (run:
   `final-reconcile.txt`).
2. The run broke in the scaffolding between the proxy and the disk: the IPC hop from proxy to daemon,
   its shared 8 MiB queue, the rule that drops a whole producer, and a failure budget built for a dead
   daemon. Then the count said "1736 model calls" while about 470 calls had no record.
3. Three options. **A**: keep the shape and patch it (about 400 changed src lines, about 3 days).
   **B**: the proxy writes its own wire record, no IPC (about 1,300 to 1,600 new src, 7 to 10 days).
   **C**: a fresh minimal rewrite (15 to 25 days).
4. **Recommend A**, with the run fixes first and the cuts second. It is the only option that fits the
   5 working days before the 4 November feature stop and leaves time for a measured 2-hour rerun.
   After Edition 01, move to the full merge (B-full) through a seam that already exists.
5. **Fallback** if the fixes are not green by Monday night: a four-constant build plus honest wording,
   so the run's failure becomes a series of visible gaps instead of a silent stop.
6. Owner answers in short: 16,384 counts open byte flows, not agents (Q2); gateways already work when
   the harness config names them, and Hermes should be the version the installer gives, 0.22.5 (Q4);
   defer the big structural cuts and also the cuts that fix nothing the run found (Q5); stop printing
   the `goalcup mcp` lines (Q6); Codex and Amp already have their update switch, Claude Code and
   Hermes need one line each (Q7); a "table" is a section of Codex's config file, not an API (Q8);
   prune after the server ack, and stop printing a local check that cannot pass (Q9); nothing is due in
   24 hours, there are 5 working days, and the must-haves are listed by day (Q10).

---

## 1. The owner's idea and how it maps to today's code

> "Can't people just register the AI endpoint(s) the harness will use, and we record that traffic and
> send it to the server, like a local proxy, and in some cases boost that with a check against local
> logs? Isn't that all we should be doing, with a little scaffolding?"

### 1.1 Part by part

| Part of the idea | What exists today | Lines (src) | Verdict |
|---|---|---|---|
| Register the endpoints | Each connector lists its default model hosts (`connectors/codex-cli/harness.toml:119-128`: `chatgpt.com`, `api.openai.com`; `connectors/amp-cli/harness.toml:111-112`; Claude Code from `crates/goalcup-protocol/providers.toml`). `model_host_from` reads the one base URL the harness's own config names (`connectors/claude-code-cli/harness.toml:40`, `codex-cli/harness.toml:37`, `hermes-cli/harness.toml:68`), read by `crates/goalcup-cli/src/model_host.rs:1-25`, merged at `commands/start/proxy.rs:153-188` | `model_host.rs` 893 | This is registration, done for the participant. Keep. Add one explicit input (1.3). |
| A local proxy that records | The loopback CONNECT proxy in `goalcup start`: `proxy/connect.rs` 1,282, `tls.rs` 295, `http.rs` 2,053, `websocket.rs` 1,020, `headers.rs` 268, `metadata.rs` 255 | about 5,170 | Keep. This part worked in the run. |
| Send it to the server | The daemon masks and chains (`goalcup-daemon`), the uploader sends zstd chunks (`crates/goalcup-cli/src/upload/`, 4,743 lines with inline tests), the Worker stores them in R2 | see section 2 | Keep the uploader and Worker. The hop between proxy and chain writer is what failed. |
| A check against local logs | The native leg reads the harness's session files and chains them in full (`native_capture.rs` 1,632) | 1,632 | Keep reading them (rollouts were 47 MB of 1.3 GB, cheap). Add the live cross-check (1.4). |
| "A little bit of scaffolding" | The scaffolding is large: proxy IPC sender `proxy/ingest.rs` 1,137 src; daemon receiver `goalcup-daemon/src/ingest.rs` 1,356; epoch and recovery logic in `proxy/api.rs:700-824`; daemon control API 1,714; CLI daemon client 2,080; termination proof 2,498; registry 1,009 | about 9,900 | The run broke in this layer. It is what to shrink, after Friday. |

So the owner is right about the shape. The over-build is in the parts between the proxy and the disk,
and the run failure came from exactly there.

### 1.2 What "registering an endpoint" means in practice

A registered endpoint is a host (and optionally a base path) that the proxy will decrypt. Four things
must hold for a call to it to be recorded:

1. The harness sends it through the proxy (it honours `HTTPS_PROXY`; Node needs `NODE_USE_ENV_PROXY=1`, D59).
2. The harness trusts the run CA for that name (CA variable per connector). The run CA is
   name-constrained to the hosts known at start (`crates/goalcup-cli/src/proxy/tls.rs:131-137`, per
   `docs/plans/2026-10-30-audit/E-providers.md` top finding 11). So a host added mid-run takes effect at
   the next `goalcup continue`.
3. The host is not a sign-in host (`auth_hosts`, never decrypted; point 4).
4. It speaks HTTP/1.1 over TLS (all providers and gateways in scope do, E-providers lane summary).

Today that list is built automatically: connector defaults plus the address the harness config names
(D62, D74; `docs/DECISIONS.md:592`, `:1105`). The participant does not type hosts.

### 1.3 Add one explicit "record this endpoint too" input

For routes the config reader cannot see (Q4 lists them), the cheapest honest add in the spirit of the
owner's idea:

- `goalcup setup` (or `start`) accepts `--record-host https://host/base` (or `--also-record`) and keeps
  it in the run's config. `approved_hosts` (`commands/start/proxy.rs:153-188`) merges it like the
  config-named host, and the CA's allowed names take it at start (`proxy/tls.rs:131-137`). Same refusals
  as `model_host.rs` (no `http`, no loopback, no auth host, no goalcup host). About 40 to 80 src and
  50 test lines.
- **Not** a reverse proxy that rewrites the harness's base URL to localhost. `docs/RULES.md:14` forbids
  changing a base URL, and it breaks OAuth logins (`B-fresh-design.md:178`).

What could go wrong: a participant registers a host that the agent's tools also call (for example a
cloud API used by tests); a tool that does not trust the run CA then fails (point 3), so say so at
setup. Hermes trusts our CA only for its one `recorder` provider entry (`D-harnesses.md:828-830`), so
an extra Hermes host fails TLS loudly unless goalcup also writes a provider entry for it. Cloud hosts
(Bedrock, Vertex) may break AWS and Google clients that use their own trust store
(`E-providers.md:383`, `:767-775`). Timing: after the must-fix items (Q10), or Edition 02.

### 1.4 "Boost with local logs": make it a live alarm

The native leg is already the "local logs" half, and it is cheap. What it lacks is use as a live
check. In the run, rollout files kept growing after 15:43 while the chain got no model call, and
nothing said so, because the CLI's "no model calls" alarm reads the selection-time counter, which
kept rising.

Harness-agnostic version, no decoder needed (D36.4 keeps the recorder format-blind,
`docs/DECISIONS.md:175-177`): in the 30-second watch tick, if native bytes grew in the last 5 minutes
and the count of model calls **written to the chain** (fix F5) did not, print one `PROBLEM:` line and
write it to `host.log` and `state.json`. About 40 src and 40 test lines. It would have fired about
5 minutes after 15:43.

A per-harness count (Codex `token_count` events against wire calls, as the run's `final.py` did) is a
decoder. Keep it in `live-check` after the run (plan AD4), not in the client.

---

## 2. What the run proves, and where each cause lives

### 2.1 Facts from the run

| Fact | Evidence |
|---|---|
| Before 15:40 the wire matched Codex's own count per 10-minute bucket (131/134, 193/192, 105/104, 113/111; 15:10 shows 181/236 because of retries) | run: `final-reconcile.txt`, bucket table |
| From 15:43:15 the chain holds no model call; Codex made 159, 140, 135 and 104 in its last buckets | run: `final-reconcile.txt`; run report, measure 1 |
| Concurrency was low. Most of the 42 subagents ran one to four at a time (for example three started at 15:13 and three at 15:35) | run: `final-reconcile.txt`, per-thread first and last times |
| So the overflow was driven by body size and bursts (1.5 MB average request body, 95 rate-limit retries that moved Codex to HTTP POST with the full context), not by 40 parallel subagents | run report, measures 6 and 9 |
| 22 `queue_full` gaps and 17 `recovery` records from 15:15 to 15:43; then nothing | run report, measure 4 |
| The relay never slowed the harness: first call under 1 s, Codex never blocked | run report, measure 9 |
| Daemon 50 MB RSS and 9 to 17 percent of a core averaged over its life; peak unknown | run report, measure 7 |
| Some upload PUTs of about 30 MB compressed failed with os error 11, 104 and 32; backoff grew to 497 s and later 1,083 s | run: `host.log` (upload lines) |

### 2.2 The cause chain, mapped to code

| # | Cause | Code | Exists only because of the process split? |
|---|---|---|---|
| C1 | One shared queue of 8 MiB, controls included | `crates/goalcup-protocol/src/capture.rs:50-51` | No. Any single-writer design has a queue. The size and the overflow rule are the problem. |
| C2 | A frame that does not fit loses the whole producer, not one stream | `crates/goalcup-daemon/src/runtime/capture/sink.rs:270-286` (`lose(&mut state, &id, reason)`) and `crates/goalcup-daemon/src/ingest.rs:1111-1119` (`LossCause::SinkRefused`) | Partly. In one process the same rule could exist, but nobody would read it as a dead peer. |
| C3 | The proxy sees the daemon close the socket and counts it as a failure | `crates/goalcup-cli/src/proxy/ingest.rs:1104-1133` (`lost`, then `count_failure`) | Yes. "Remote close" has no reason on a socket, so load looks like death (`ingest.rs:16`). |
| C4 | An epoch under 60 s does not reset the count; ten in a row latch the sink off for good | `ingest.rs:98-108` (`MAX_CONNECTION_FAILURES = 10`, `STABLE_EPOCH = 60 s`), `ingest.rs:455-471` (`latch_terminal`, `count_failure`) | Yes. The budget exists to give up on an unreachable daemon. |
| C5 | After the latch the proxy drops frames locally; the daemon never sees a loss, so no gap has an end | `ingest.rs:455-459` (`discard_queue`, `Phase::Disabled`) | Yes. Only the daemon writes gaps, and it was not told. |
| C6 | The finish line counts at selection, not at write, and counts every WebSocket client message | `crates/goalcup-cli/src/proxy/http.rs:305-320`, `proxy/websocket.rs:504`, `proxy/api.rs:397-403`; printed at `commands/start/finish.rs:502-505` | No. A counting choice. |
| C7 | 32 native stream slots for 43 rollout files | `crates/goalcup-daemon/src/native_capture.rs:279-280` (`MAX_OPEN_STREAMS / 4`, `max_files: 1024`) | No. A cap of our own. |
| C8 | One stream per WebSocket message (152,728 streams, three records each) and full-context bodies: about 1.3 GB per hour | `proxy/websocket.rs:950-970` (`open` per message) | No. A record-shape choice. |
| C9 | Upload: 30-minute interval, 256 MiB eager mark, 32 MiB chunks, backoff up to 15 minutes | `crates/goalcup-cli/src/upload/policy.rs:80-86`, `upload/uploader/chunk.rs:13` | No. Constants. |

Not verified: why the daemon fell behind. Two candidates in the code: the owner thread drains the
queue, runs every redactor and polls the native files on the same thread
(`crates/goalcup-daemon/src/runtime/capture/owner.rs:160-176`, native poll every 100 ms at `:16`), and
it syncs the chain every 800 ms (`owner.rs:10`). The run captured only lifetime CPU averages, so
whether the owner thread was saturated in bursts is unknown.

Three of the nine causes (C3, C4, C5) exist only because the recorder is a second process. Two more
(C1, C2) are made worse by it: there are two 8 MiB queues in series (`proxy/ingest.rs:116-125` and
`sink.rs:6-10`), and the daemon's refusal reaches the proxy as a closed socket.

### 2.3 Do the reasons for the daemon split still hold?

| Decision | What it says | Holds? | Evidence |
|---|---|---|---|
| D3, "record raw, interpret later" | The daemon parses and judges nothing; the verifier judges | Yes | `docs/DECISIONS.md:68-73` |
| D3, "keeps the signed, hard-to-change daemon small" | The reason for "the CLI never links the daemon" | **No.** The binaries are unsigned and not notarized | `packaging/README.md:22-23`; `docs/ARCHITECTURE.md:50` (the ban); `C-module-audits.md:295` already says so |
| D4, one run-scoped detached daemon | `setsid`, so Ctrl-C or a closed terminal does not reach the chain writer; crash recovery is `goalcup seal` reopening the entry | **Weakly.** `goalcup start` already shares the terminal with a chat TUI (`crates/goalcup-platform/src/terminal.rs`, 292 src) and can ignore SIGINT and SIGHUP itself. The split's one real gain: if `goalcup start` dies, the daemon still seals the chain. But the harness is then stuck anyway, because its proxy died with `goalcup start` (`C-module-audits.md:632-634`) | `docs/DECISIONS.md:75-86` |
| D14, redaction at capture | Mask before bytes are hashed or reach disk | **Yes**, it is point 4. It binds every option: no option may spool raw bodies to disk and mask later | `docs/DECISIONS.md:121-124`; `C-module-audits.md:558, 574` |
| D58, producer IDs | 65,536 IDs, one spent per recovered epoch; "`MAX_CONNECTION_FAILURES` still ends the wire leg" | The ID budget holds only if recoveries stay rare (one per 9.2 s for 168 h). The give-up rule is what failed | `docs/DECISIONS.md:488-496` |

No decision forces the split for Friday, but D14 forces whoever writes the record to mask first, in
memory. And the split is not the cheapest thing to remove in a week.

---

## 3. Options A, B and C

Working days to the feature stop (4 Nov 00:00 UTC, `TODO.md:42`): Fri 30 Oct, Sat 31, Sun 1, Mon 2,
Tue 3. That is 5 days, shared with the plan's cut PRs and one 2-hour rerun. Wed 4 is tests and the
release candidate, Thu 5 is buffer.

### 3.1 Option A: keep today's shape and patch it

Shape unchanged: proxy in `goalcup start`, IPC to a separate daemon with one chain writer and the
native reader, uploader, Worker. The run fixes F1 to F9 (F2, F3 and F6 are already in the plan as CH6,
CH1, CH2 and CH3):

| Id | Fix | Where | New or changed src / tests | Cause fixed |
|---|---|---|---|---|
| F1 | Load never kills a producer. The sink answers "busy" instead of losing the producer; the daemon's receiver stops reading that socket and retries the same frame after 10 to 50 ms (TCP backpressure). Pressure moves to the proxy's own queue, whose overflow is a `QueueFull` loss that is already **not** a failure (`proxy/ingest.rs:57`) and is followed by a recovery. The native adapter retries a busy answer on its next poll. Needs one new `SubmitResult` variant. A busy state that lasts 30 s falls back to the old loss. | `goalcup-protocol/src/capture.rs:370-373`; `goalcup-daemon/src/runtime/capture/sink.rs:270-286`; `goalcup-daemon/src/ingest.rs:1111-1119`; native submit path | about 80 / 250 | C2 |
| F2 | Delete the failure budget: no `MAX_CONNECTION_FAILURES`, no `STABLE_EPOCH`, no `count_failure`; while `goalcup start` lives, reconnect on the 1, 2, 5, 10 s schedule for ever (`ingest.rs:90-97`) | `proxy/ingest.rs:98-108, 455-471, 1104-1133` | about -40 / 200 rewritten | C3, C4 |
| F3 | Queue 8 MiB to 64 MiB on both sides, control reserves scaled (CH1). A real memory cost, stated: up to 128 MiB only while the daemon is behind | `capture.rs:51`; `proxy/ingest.rs:116`; `sink.rs:7` | about 10 / tests in CH1 | C1 |
| F4 | Loss is never silent. (a) Any `Disabled` answer the proxy reaches writes "wire recording off since T" to `host.log` and `state.json` at once (plan AD3). (b) At finish, if the last wire producer was lost and never recovered, the daemon writes one stream-less gap stamped with the loss time; the seal is its end. (c) A loss gap's end is the next `Recovery`; `goalcup status` and the finish line print the total minutes off | proxy `mod.rs` health; daemon `runtime/capture/finish.rs`, `writer.rs`; CLI `run/watch.rs`, `commands/start/finish.rs` | about 80 / 120 | C5 |
| F5 | Count a model call only when its request stream is written. The daemon counts request streams it wrote whose metadata passes the model-call rule (move `ExchangeMeta::counts_as_model_call`, `proxy/metadata.rs:199-210`, into the protocol crate so both sides use one rule) and reports the count in its status. The finish line and status read that count; the proxy's selection counter stays only for the "no calls" alarm's first minutes. WebSocket: say "requests", since a client message can be an interrupt (`counts_as_model_call` cannot tell, D36.4) | protocol, daemon status (`CONTROL_API_VERSION` bump, `crates/goalcup-cli/src/daemon/mock.rs:157-168`), `commands/start/finish.rs:502-505`, `run/mod.rs:266` | about 120 / 150 | C6 |
| F6 | Native caps: delete the 32-stream share, the 1,024-file and 20,000-file caps; idle poll 1 s; one directory open per pass (CH2, CH3) | `native_capture.rs:187, 279-280`; `owner.rs:16` | about 0 net / 250 rewritten | C7 |
| F7 | Live cross-check (1.4): native grew, no written call for 5 minutes, one `PROBLEM:` line | `run/watch.rs` | about 40 / 40 | silent loss in general |
| F8 | Upload: interval 30 to 5 minutes; backoff cap 15 to 2 minutes; client chunk cap 32 to 8 MiB compressed (the Worker still accepts up to 32 MiB, `server/worker/src/chunks.ts:22`), so a failed PUT costs less | `upload/policy.rs:80-86`; `upload/uploader/chunk.rs:13-16` | about 5 / 20 | C9 |
| F9 | Trace size: accept it for Friday. One stream per WebSocket direction is a record-shape change that also breaks the judges' decoder branch; do it after Edition 01. Optional and cheap: zstd long-distance matching on the upload chunk (`chunk.rs:17-18` says "no long window" today) plus `zstd -d --long=27` in `scripts/fetch-run.sh:327`; the Worker never decompresses (`chunks.ts:178, 212`). Measure the ratio on the rerun before turning it on | `chunk.rs:144`, `fetch-run.sh:327` | about 6 / 20 | C8 (cost only) |
| | **Total run fixes** | | **about 400 changed src, about 900 test lines** | |

How A handles the run's load (40 subagents, 1.5 MB bodies, rate-limit bursts): the daemon no longer
drops a producer under load (F1). If the writer falls behind, TCP backpressure fills the proxy's 64 MiB
queue (about 40 full-context bodies). Only if that also overflows is the epoch lost: a `QueueFull` gap,
a recovery, recording goes on, and the minutes off are shown (F4). No terminal latch is left except
shutdown and the 65,536-ID budget. Residual weakness: a loss is still per epoch, so every stream in
flight at that moment is lost, not one stream. And the single owner thread is still the throughput
limit; its peak rate is unmeasured.

An alternative to F1 from lane N2 (M2 in Q10): a full queue loses only the frames of the stream that
overflowed, with a gap on that stream. Both meet the handover's "backpressure or a bigger buffer per
stream, not dropping a whole producer". F1 is preferred because it moves no bytes into a gap while the
proxy queue still has room.

### 3.2 Option B: the proxy writes the record itself

Shape: each intercepted connection masks its own bytes in process (D14) and appends records to its own
file in the part folder. The file path is the ID. No IPC, no producers, no epochs, no shared queue, no
failure budget. A per-connection bounded buffer feeds a blocking writer; if it overflows, that one
connection gets a gap with a start and an end.

- *B-full* (daemon gone): native reader, seal, status, termination and `goalcup seal` from another
  terminal all move into `goalcup start`. That is the plan's K13 merge, about 12,000 src and 15,000 test
  lines, touching 22 CLI files (`C-module-audits.md:316`). Not in a week.
- *B-wire* (daemon keeps native, seal and status): only the wire record moves. This is the B in the
  table below.

Reused (B-wire): proxy relay and decoders (`connect.rs` 1,282, `tls.rs` 295, `http.rs` 2,053,
`websocket.rs` 1,020, `headers.rs` 268, `metadata.rs` 255, about 5,170 src); the emitter core
(`proxy/api.rs:296-650`, about 350); the redactor (`goalcup-daemon/src/redact.rs` 1,209 and
`redact/stream.rs` 2,946 src, about 10,000 test lines, linked into the CLI); uploader chunk logic,
Worker, native capture unchanged.

Goes: `proxy/ingest.rs` 1,137 src; daemon `ingest.rs` 1,356 src; recovery and re-observation in
`proxy/api.rs:700-824` (about 125); the wire half of `sink.rs` (about 150); ingest handshake types in
the protocol (about 100). About 2,850 src. Tests: `capture_ingest.rs` 3,095, `m2_capture_sender.rs`
1,991, `m2_ipc_compose.rs`, parts of `m2_emitter.rs` (about 5,500). Also the dependency ban tests
(`the_structural_bans_hold_at_the_dependency_level`, `the_shipped_binary_links_no_daemon`,
`docs/ARCHITECTURE.md:50-53`).

New code (estimates): per-connection writer with buffer, masking on a blocking pool and file rotation,
about 350. Record format for wire files and a hash per file, about 150. Uploader that sends many files
(by path) beside the chain, about 250. Worker key and index per file, about 150 TypeScript plus 200
tests, possibly an SQL change. `fetch-run.sh` and a rebuild reader for wire files, about 250. Status
counts at write, finish and seal changes, about 150. About 1,300 to 1,600 src and 1,500 to 2,000 test
lines; net about -1,300 src.

Today's proxy runtime has one worker thread (`commands/start/proxy.rs:96-100`). Masking 1.5 MB bodies
on it would slow every relay (point 3), so B must move masking to a blocking pool and raise the thread
count, which the plan withdrew (CH7) for lack of measurement.

### 3.3 Option C: fresh minimal rewrite to the owner's idea

Shape: B1 or B2 from `docs/plans/2026-10-30-audit/B-fresh-design.md`: one binary, environment proxy
with a run CA, decrypt the registered hosts, mask in memory, segments uploaded by idempotent `PUT`,
session folder poller, plain status. If the Rust relay and redactor are reused too, this is option B,
not a rewrite.

New code: B1 about 2,040 client src in Go; B2 about 2,070 (`B-fresh-design.md:536`, `:1052`). In Rust,
1.3 to 2 times (`:518`, `:1039`): about 2,700 to 4,100. Plus the parts both designs missed but need:
terminal sharing for chat TUIs, the config reader for model hosts, no-follow session reads, Hermes
launch prep (`B-fresh-design.md:1230-1249`), about 1,000 to 1,500 more. Plus about 2,000 to 2,400 test
lines. Both designers said a rewrite in eight days is the largest risk and advised cutting the existing
code toward the shape (`B-fresh-design.md:62-64`, `:547-550`).

### 3.4 Side by side

| | A: patch today's shape | B-wire: proxy writes its own wire record | C: fresh rewrite |
|---|---|---|---|
| Reused | Everything | Relay and decoders (about 5,170 src), emitter core (about 350), redactor (about 4,150), uploader core, Worker, native leg | Connector data (four `harness.toml` files), maybe the Worker, the redaction corpus as a bar |
| Goes before Friday | Failure budget, native caps (plus the plan's cuts) | About 2,850 src, about 5,500 tests of IPC and epochs | About 85,800 client src (plan section 2) |
| New lines | About 400 changed src, about 900 tests | About 1,300 to 1,600 src, 1,500 to 2,000 tests | About 3,700 to 5,600 src in Rust, 2,000 to 2,400 tests |
| Working days | About 3 for the fixes (5 available) | 7 to 10, plus a Worker deploy and full gate | 15 to 25, plus re-measuring every harness |
| Run failure handling | Fixed: no load-driven drop (F1), no give-up (F2), never silent (F4), count at write (F5). Loss stays per epoch if both queues overflow | Fixed, loss per connection with start and end | Fixed on paper; the relay that matched Codex before 15:43 is replaced by untested code |
| Point 1, every model call | Yes, within the host list; F7 catches a bypass or a silent wire | Yes, no shared queue to overflow | On paper |
| Point 2, every subagent | Yes, F6 removes the native caps; subagent wire calls cross the same proxy | Yes | On paper |
| Point 3, never block | Yes. Backpressure stays inside goalcup; the relay forwards first (`proxy/http.rs`, `C-module-audits.md:499`) | Yes only if masking is off the relay thread; at risk otherwise | On paper |
| Point 4, no secrets | Unchanged D14 path; masking option b (plan PR5) | Holds (in-memory masking), but masker state becomes per connection, which changes how a secret split across messages is held | Must be rebuilt and re-proved against the corpus |
| Point 5, upload everything | Yes, lag fixed by F8. About 400 MB an hour compressed at the run's worst rate, about 67 GB for 168 hours; R2 about $1 a month per such run (plan Q3, unverified price) | Needs a new uploader and Worker path, untested end to end | New path |
| Point 6, tell plainly | Yes: F4 minutes off, F5 count at write, F7 alarm | Counts at write come free | On paper |
| Point 7, harnesses as people run them | Unchanged, no connector or launch change | Unchanged | Weak: every measured behaviour (for example `connectors/hermes-cli/harness.toml:130-155`) must be found again |
| Record format, Worker, verifier change | Control API bump only | All three; the plan section 8 "must deploy together" list grows by four rows | All |
| Biggest risk | Owner-thread throughput unmeasured; a busy state that never clears | New record format a week before the event; the judges' decoders and `goalcup-verify` read one chain per entry today; many open files per part; masking on the relay thread | Not ready on 6 November; the event starts 7 November 00:00 UTC with no working recorder |
| Test before Friday | Yes: L1 load test Sunday, 2-hour rerun Monday, RC run Wednesday | Barely: needs all of A's tests plus a server rebuild of wire files under `wrangler dev` (`crates/goalcup-cli/tests/full/upload_worker.rs`, today `#[ignore]`), a per-connection masking test across two WebSocket messages, and a run per harness. No time for a 2-hour rerun | Not achievable |

### 3.5 Tests for option A

1. Unit and integration: a sink whose drain is paused for 5 s while 40 streams push 1.5 MB bodies: no
   producer lost, or one `QueueFull` gap followed by a recovery; never `Disabled`. Rewrite
   `an_epoch_that_takes_one_item_and_closes_at_once_still_runs_out_at_ten`
   (`crates/goalcup-cli/tests/it/m2_capture_sender.rs:1123-1157`) to assert the opposite.
2. Load test L1 with real binaries: the test model server, 40 parallel clients sending 1.5 MB POSTs,
   one WebSocket at 50 messages a second, 20 minutes; `SIGSTOP` the daemon for 30 s twice. Pass: every
   sent request is in the chain or inside a gap with start and end; the finish count equals the chain.
   Report the owner thread's sustained MB per second.
3. The same Codex prompt, 2 hours, Vercel Sandbox, Monday 2 Nov, with the run's `final.py` reconcile.
   Pass: every 10-minute bucket matches, or the difference sits inside a named gap that status showed.

---

## 4. Recommendation and fallback

**Recommend A, run fixes first, cuts second.** It keeps the core that worked, removes every cause the
run found, and leaves a measured 2-hour rerun before the feature stop. The owner asked for "something
that works versus not working" (owner answer 5); A is the only option with time to prove it works.

The plan already had two of the fixes (CH6 deletes the failure budget, CH1 raises the queue) but would
have kept the overstated call count: AD2 reads the selection-time count at `run/mod.rs:266`. F5 fixes
that. The plan also rejected the daemon-side per-stream change (plan section 13); the run evidence
overturns that.

The day plan is in Q10 below.

**After Edition 01:** go to B-full through the seam that already exists. The proxy's emitter takes any
`Arc<dyn CaptureSink>` (`proxy/api.rs:262, 300`), and the protocol calls `CaptureSink` "the one
submission seam shared by the IPC sender and the direct component sink"
(`goalcup-protocol/src/capture.rs:388-394`). The daemon's `OwnerSink` already implements it. Running
the capture runtime inside `goalcup start` and handing the proxy the `OwnerSink` directly deletes both
ingest halves without a new record format, uploader or Worker change. Then add per-stream loss and one
stream per WebSocket direction. Seal, status from another terminal and termination still need the K13
work.

**Fallback if the fixes slip** (F1, F4 or F5 not green by Monday night): ship the four-constant build.

1. `MAX_CONNECTION_FAILURES` effectively unbounded (or the `count_failure` call removed): the sink never
   latches off while `goalcup start` lives (`proxy/ingest.rs:104`).
2. `MAX_CAPTURE_QUEUE_BYTES` 8 to 64 MiB with the reserves scaled (`capture.rs:51`).
3. Native share and file caps removed (`native_capture.rs:187, 279-280`).
4. The finish line stops saying "Recording works: N model calls recorded"; it says "N requests reached
   the recording proxy; run `goalcup status` to see gaps" (`commands/start/finish.rs:502-505`), and the
   participant guide says the count is not a proof of recording.

With 1 and 2 the run's exact failure becomes a series of `queue_full` gaps with recoveries, each one
visible, instead of a silent stop. It is still coarse (per-epoch loss) and still overstates the count,
so say so in the release notes.

**What could go wrong with the recommendation**

- **The writer is slower than we think.** If the owner thread cannot sustain a Codex HTTP fallback
  burst, F1 turns drops into backpressure and the proxy's 64 MiB queue still overflows. Recording
  continues with gaps, but a heavy run could spend a lot of time in gaps. If L1 shows a low rate, move
  the native poll off the owner thread (a small change) before Monday.
- **A busy answer that never clears.** F1 needs its timeout back to the old loss, or a wedged writer
  stalls one producer's socket for ever. A stall is worse than a gap.
- **Producer IDs run out.** With frequent overflow-and-recover cycles, 65,536 IDs last about 73 hours at
  one recovery every 4 s. F1 makes recoveries rare; L1 and the rerun should count them per hour.
- **The control API bump (F5)** must ship in the one release with the installer tying CLI and daemon
  versions (`packaging/install.sh:104-195`, per plan section 8).
- **Counting at write** depends on stream metadata. A stream whose `End` is lost in a gap is not
  counted, which is the point; the participant must read "N written, M minutes off" together.
- **Trace size.** Accepting 1.3 GB an hour raw means a fast disk and upload. Prune after ack (Q9) keeps
  the local copy small only if upload keeps up; F8 helps, but a slow home connection may still lag.
- **Moving cuts out.** Reviewers may want the K1 delete before the decoder branch is cut; the plan's
  `decoders-ed01` branch still works if K1 simply does not happen this week.
- **One 2-hour run with 20 subagents does not prove 1,000.** The week-long projection rests on memory
  figures that were not measured at peak.

---

## 5. Owner questions

Q1 (replace README P1 to P7 with the seven points) is answered yes and needs no more facts. Q3 (the run
size limit) is folded into Q2, as the owner did.

### Q2. What does "16,384 open streams" count? Is it agents? And the run size limit

**In plain words.** The plan wanted to raise `MAX_OPEN_STREAMS` from 128 to 16,384. The owner asked
whether that means 16,384 agent threads, and said: no caps, remove the per-run size limit.

**Facts.** It is not agents. It counts recorded byte flows that are open at the same moment. A
"stream" is one recorded flow of bytes with an open, some bytes, and an end
(`crates/goalcup-protocol/src/capture.rs:96-103` lists the kinds):

| Kind | One stream is | How long it stays open | Source |
|---|---|---|---|
| HTTP request body (`WireRequest`) | the body of one model request | until the request is sent | `crates/goalcup-cli/src/proxy/http.rs:17-19` |
| HTTP response body (`WireResponse`) | the body of one answer | until the answer ends (a streamed answer: seconds to minutes) | same |
| Header streams (`http-head-v1`) | the masked head of one request or answer | a moment | `crates/goalcup-cli/src/proxy/http.rs:45-48` |
| WebSocket message (`WsClient`, `WsServer`) | one message in one direction | a moment: one event of a streamed answer | `crates/goalcup-cli/src/proxy/websocket.rs:6-14` |
| Native (`Native`) | one session file (main agent or one subagent transcript) | the whole entry (one `goalcup start` or `continue`) | `crates/goalcup-daemon/src/runtime/capture/tests.rs:2235-2239` |

The run made 152,728 WebSocket server-message streams in 49 minutes (about 52 a second), but each one
lived for a moment. A WebSocket connection has at most one message open per direction at a time.

Today's bounds (`crates/goalcup-protocol/src/capture.rs`):

| Constant | Value | Line | What it counts |
|---|---|---|---|
| `MAX_OPEN_STREAMS` | 128 | `:58` | streams open at once, per entry, wire and native together |
| `MAX_OPEN_CONNECTIONS` | 128 | `:60` | captured network connections open at once |
| `MAX_CAPTURE_QUEUE_BYTES` | 8 MiB | `:51` | bytes waiting for the chain writer, all producers together |
| `MAX_LIVE_PRODUCERS` | 128 | `:66` | live proxy or native readers (a proxy reconnect is a new producer) |
| `MAX_PRODUCERS` | 65,536 | `:63` | producers over the whole entry, reconnects included |
| Native share | 32 (`MAX_OPEN_STREAMS / 4`) | `crates/goalcup-daemon/src/native_capture.rs:279` | session files read at once |
| Native files | 1,024 tracked (20,000 hard ceiling) | `native_capture.rs:280`, `:187` | session files followed per entry |

Harnesses cap subagents running at once: Claude Code 20 per session
(`CLAUDE_CODE_MAX_CONCURRENT_SUBAGENTS`, `docs/plans/2026-10-30-audit/D-harnesses.md:265`), Codex 6 by
default (`agents.max_threads`, `D-harnesses.md:663-666`), Hermes 10 (`max_concurrent_children`,
`D-harnesses.md:999-1003`). Only session files pile up, because each keeps its slot for the entry.

| Run | Native streams | Wire streams | Total open at once | Today (128 / native 32) |
|---|---:|---:|---:|---|
| The 2-hour Codex run: 42 subagents, about 20 at once | 43 | about 40 | about 85 | Native share full: 32 slots for 43 files, 11 transcripts probably lost (`owner.rs:143-144` wrote the stream-less `limit_exceeded` gap at the seal) |
| 40 subagents in one entry | about 41 | 40 to 80 | about 80 to 125 | At the edge of 128; native share full |
| 1,000 subagents in one entry (over a week, 20 at once) | about 1,001 | about 40 | about 1,050 | Far past both |
| 10,000 subagents in one entry | about 10,001 | about 40 | about 10,050 | Fits under 16,384 |

Estimates from the code and the run; peak open streams were not measured (unverified).

Past a bound today: on the proxy side one stream too many turns the whole producer off
(`crates/goalcup-cli/src/proxy/api.rs:451-453`: `Epoch::Disabled(GapReason::LimitExceeded)`); on the
native side the 33rd file waits for the whole entry and becomes one stream-less gap at the finish.

Memory per open stream: at most 16 KiB of not-yet-masked bytes (`MAX_HELD_BYTES`,
`crates/goalcup-daemon/src/redact/stream.rs:122-125`), plus up to 8 KiB of metadata (`capture.rs:70`).
A native stream holds no open file between polls (`native_capture.rs:76-81`), so streams do not use
file handles (the owner's worry about the macOS 256 handle default does not apply to this count).

| Open streams | Worst-case masking memory | Typical |
|---:|---:|---|
| 128 (today) | 2 MiB | near zero |
| 1,050 (1,000 subagents) | about 16 MiB | near zero |
| 16,384 (plan CH1) | 256 MiB | near zero |
| 100,000 | about 1.5 GiB | near zero |

The queue is different: every queued byte is RAM. With 1.5 MB request bodies, 8 MiB holds about five
bodies, which is why it overflowed; 64 MiB holds about 40. The daemon used 50 to 52 MB RSS (run report,
measure 7).

Run size limit: `RUN_MAX_COMPRESSED_BYTES` is 64 GiB (`server/worker/wrangler.toml:59` and `:151`;
default `server/worker/src/context.ts:12`; checked in `server/worker/src/chunks.ts:118-130`, answers 413
and the client retries). The run's chain grew 1.3 GB an hour, compressed 3.2 to 1 (run report, measure
6): about 0.41 GB an hour, about 68 GB for 168 hours. That is right at the 64 GiB limit (68.7 GB) with only
20 subagents. It would bite. To remove it, no code is needed: set the variable to `9007199254740991`
(the largest value `parseSafeUint` takes, `server/worker/src/util.ts:119-123`).

Cloudflare limits that remain (none binds today):

| Limit | Value | Our use | Source |
|---|---|---|---|
| Worker request body | 100 MB on Free and Pro, 200 MB Business | a chunk is at most 32 MiB compressed (`server/worker/src/chunks.ts:22`, client `crates/goalcup-cli/src/upload/uploader/chunk.rs:13`) | https://developers.cloudflare.com/workers/platform/limits/ |
| Worker CPU per request | 30 s default, up to 5 min | one chunk store | same |
| R2 object | 5 GiB single upload, 4.995 TiB multipart | one object per chunk | https://developers.cloudflare.com/r2/platform/limits/ |
| R2 bucket size | unlimited | | same |
| R2 writes to one key | 1 a second | each chunk has its own key | same |
| R2 price | $0.015 per GB-month, egress free | about $1 a month for a 68 GB run | https://developers.cloudflare.com/r2/pricing/ |

Our own server rule that still matters: a run closes 24 hours after it ends and takes no more upload
(`CLOSE_AFTER_SECONDS`, `server/worker/src/db.ts:17`). In the 2-hour run the upload lagged badly
(7 percent sent at t+61 min, backoff up to 1,083 s). A week-long backlog must finish inside that day.

**Recommended answer.**

- 16,384 is "byte flows open at the same moment", not agents. A subagent costs about one open stream
  for its session file, plus a few short ones while it calls the model.
- Delete the native share (32) and the native file caps (1,024 and 20,000). They are caps of our own
  with no cost behind them, and they lost 11 transcripts in the run.
- Keep one entry-wide open-stream bound at 16,384 and say why next to it: "memory guard, at most 256 MiB
  of held masking bytes". That fits about 16,000 subagents in one entry. Past it, refuse only the one
  new stream with a gap (plan CH4), never the whole producer.
- Raise the queue to 64 MiB **and** stop dropping a whole producer when it is full (F1). Size alone is
  not enough for 1,000 subagents.
- Set `RUN_MAX_COMPRESSED_BYTES` to the largest value. Add a Workers Logs alert on stored bytes per run
  instead of a refusal.

What could go wrong: a run with tens of thousands of session files in one entry still meets the bound;
the fix after Edition 01 is to close quiet append-only streams and reopen them at their offset. Memory
grows with what is truly in flight; a slow disk fills the queue to its new size. Without a server limit,
a bug that loops uploads costs money until someone looks.

### Q4. Gateways: "with and without gateway"? And which Hermes?

**In plain words.** The plan asked whether to drop Hermes 0.19.0 and pin 0.22.5. The owner read it as
"do we support gateways?" and asked: ideally both, with and without; was that not the plan? Both are
answered. (In the plan, "gateway" meant Hermes's own gateway process; the connector handles both
Hermes versions there, `connectors/hermes-cli/harness.toml:130-155`, D72: on 0.19 the `gateway stop -y`
command exits 2 and is only logged.)

**Facts: AI gateways.** Yes, both already work, by one rule (D74). goalcup reads the one address the
harness's own config names and records that host. With no address, it records the harness's default
host. There is no provider list (`crates/goalcup-cli/src/model_host.rs:1-8`). Plain `http`, loopback
and IP addresses are refused (`model_host.rs:16-24`).

| Harness | Where the participant writes the gateway address | Key goalcup reads | Measured? |
|---|---|---|---|
| Claude Code | `env.ANTHROPIC_BASE_URL` in `claude-code-cli/settings.json` (Goal Cup's home), e.g. `https://ai-gateway.vercel.sh/claude-code` or `https://openrouter.ai/api` | `connectors/claude-code-cli/harness.toml:40` | Vercel: 15 calls recorded, 28 Oct (`docs/research/results/2026-10-vercel-gateway.md:53`). OpenRouter: expected, unmeasured |
| Codex | `model_provider = "vercel"` plus a `[model_providers.vercel]` table with `base_url` | `connectors/codex-cli/harness.toml:37` (`via = "model_provider"`) | Vercel: 17 calls, HTTP only, no WebSocket (same doc `:54`, `:60-61`) |
| Hermes | `model.base_url` in `config.yaml` | `connectors/hermes-cli/harness.toml:68` | Vercel built-in `ai-gateway`: 9 calls (same doc `:55`) |
| Amp | no choice: Amp always talks to its own server | connector default host | yes, earlier |

Setup steps are in `docs/HARNESS-SETUP.md:92-113` (Codex) and `:144-169` (Claude Code). The 28 Oct
measurement used a free model on Vercel only. Also measured per `E-providers.md:497-526` (E8.2):
Cloudflare `gateway.ai.cloudflare.com/...`, remote LiteLLM and Azure base URLs take the same path.

What is missing (each records nothing and today says nothing):

| Route | Why it is not recorded | Source |
|---|---|---|
| Claude Code `ANTHROPIC_BASE_URL` in a shell export, a project `.claude/` file or managed settings | goalcup reads only Goal Cup's `settings.json` | `docs/HARNESS-SETUP.md:164-168` |
| Claude Code cloud switches `CLAUDE_CODE_USE_BEDROCK`, `_VERTEX`, `_FOUNDRY`, `_MANTLE` | no base URL in the key we read; tunnelled | `docs/plans/2026-10-30-audit/E-providers.md:497-520` (E8.2) |
| Codex top-level `openai_base_url` | the reader does not look at it | `D-harnesses.md:566-571` |
| Codex built-ins with no table: `ollama`, `lmstudio`, `amazon-bedrock`, `gcp` | see Q8 | E8.2 |
| Codex `--profile x` (Cloudflare's own Codex recipe) | goalcup builds Codex's argv and does not read profiles | `docs/HARNESS-SETUP.md:105-106` |
| Hermes 0.21.6+ calls to any host other than `model.base_url` (delegation provider, auxiliary fallback, fallback providers, mixture of agents) | Hermes now trusts the OS store, so these succeed through the opaque tunnel | `D-harnesses.md:828-834`, `:1112-1116` |
| A base URL or proxy setting in a `.env` the harness reads: `$CODEX_HOME/.env` with `HTTPS_PROXY=` or `NO_PROXY=*`, Hermes `<PROVIDER>_BASE_URL` | Codex overwrites our environment from that file; Hermes reads it | `D-harnesses.md:589-600`; `connectors/hermes-cli/harness.toml:62-64`; E top finding 5 |
| Codex WebSocket mode through Vercel | opt-in in the provider table; unmeasured | `E-providers.md:312` |

**Facts: Hermes version.**

| Fact | Source |
|---|---|
| Hermes's installer gives the latest stable release, today v0.22.5 (22 Oct). PyPI stops at 0.19.0 (20 Jul) | `D-harnesses.md:818-821`, `:850-852`; https://github.com/NousResearch/hermes-agent/releases (fetched 29 Oct 2026: newest is still v0.22.5) |
| 0.19.0 `delegate_task` failed twice on 29 Oct with "an internal executor error"; the exact text was not kept | `docs/plans/2026-10-29-record-everything-progress.md:74` |
| The same 0.19.0 ran two `delegate_task` children on a Mac on 22 Oct | `D-harnesses.md:1014-1016` |
| Delegation fixes after 0.19.0: 0.19.1, 0.20.2, 0.20.3, 0.21.0; about 40 delegation commits in 0.22.5 | `D-harnesses.md:1017-1024` |
| Delegation on 0.22.5 has never been measured by us. No branch has a run since (searched 29 Oct) | `D-harnesses.md:835-839` |
| Our connector still carries three parts that only 0.19.0 needs (`HERMES_CA_BUNDLE`, two `configure` commands, the `session_*.json` row), and the first breaks Nous Portal token refresh on 0.21.6+ | `connectors/hermes-cli/harness.toml:54-57`, `:88-92`, `:103-109`; `D-harnesses.md:1088-1093` |
| A shared home that ran 0.22.5 cannot go back to 0.19.0 (schema 32 database) | `D-harnesses.md:1129-1131` |

**Recommended answer.**

- Gateways: yes, "with and without" already works for Claude Code, Codex and Hermes, and it was the
  plan. For Friday: (1) warn at start, never refuse mid-run, when the config uses a route we cannot see
  (cloud switches, Codex `openai_base_url` until read, Hermes routes on other hosts); (2) read Codex
  `openai_base_url` (small); (3) one OpenRouter run with Claude Code or Codex if time allows; (4) the
  "register extra endpoints" input (section 1.3) after the must-fix items, or for Edition 02.
- Hermes: support the version the installer gives (0.22.5 today) and say so; drop 0.19.0 and its three
  connector parts. Whether delegation works is unknown until one 0.22.5 run with "use subagents" in the
  prompt. Until then the docs say "Hermes subagents: not yet measured". Tell participants to install
  with `--commit <sha>` and set `updates.check: false` (Q7).

What could go wrong: a 0.22.6 or 0.23.0 lands before 7 Nov (8 stable releases in 15 days,
`D-harnesses.md:853-856`) and changes the provider entry we write. Warnings at start can be ignored;
the finish line must then show near-zero model calls plainly.

### Q5. Defer the big structural cuts? ("I would like something that works versus not working")

**In plain words.** The plan asked whether to leave the daemon merge, termination proof, signing,
connector pins, deny lists, the Rust Worker stand-in and the event-rule commands (`login`, SSH key
warnings, push lines, client repo check, run give-back) until after Edition 01. The owner was unsure
and asked for something that works.

**Facts.**

- Those cuts (K13 to K22, plan section 4b) save about 25,000 more src lines, but each touches working
  code that ends or proves a 168-hour run. K13 alone (the daemon merge) is about 12,000 src and 15,000
  test lines, rated high risk (Ctrl-C and `SIGHUP` reach the recorder; offline seal is untested).
- The run broke in part of the K13 area (the IPC layer, section 2.2), but option A fixes it with about
  400 changed lines, while the merge needs 7 to 10 days even in its smaller B-wire form (section 3).
- The run also showed the plan's day budget is now short by about 2 days, because the run fixes F1, F4,
  F5, F7 and F8 are new work.
- Some "before Friday" cuts fix nothing the run found: K1 (verifier decoders, 9,800 src and 16,800 test
  lines of churn), K4, K26, K10, K11 and K12.

**Recommended answer.** Yes, defer the structural cuts. "Works" first means the run fixes first. Also
move K1, K10, K11 and K12 after Edition 01 to make room; this needs the owner's yes. Keep the cheap pure
deletes in separate files on Tuesday (K2 scripts, K6 spikes). K4 and K26 (test-only deletes) may land
any day they do not take review time from the run fixes; lane N1 would move them after, lane N2 would
keep them if spare: the owner can choose, the risk is low either way. K1 is only worth doing before
Friday if AD4 replaces what `scripts/matrix.sh` reads.

What could go wrong: the code stays large through Edition 01, and the IPC layer that failed stays in
place, patched. The B-full merge after Edition 01 is still large work. Reviewers may want K1 before the
`decoders-ed01` branch is cut; that branch still works if K1 does not happen this week.

### Q6. Stop printing the `goalcup mcp` setup lines

**In plain words.** The owner said yes: no need to print the lines that add Goal Cup's time-left tool
to the harness's MCP settings.

**Facts.** `goalcup prep` prints them from `time_left_tool_advice`
(`crates/goalcup-cli/src/commands/prep.rs:729-760`), driven by a `[participant.time_left_tool]` table
in each of four connectors (`connectors/claude-code-cli/harness.toml:15`, `codex-cli/harness.toml:13`,
`amp-cli/harness.toml:17`, `hermes-cli/harness.toml:21`). Goal Cup never writes those lines into the
harness's files (D67). `commands/mcp.rs` is 474 lines (src and inline tests). Plan K8 removes the setup
lines and the legacy `git-credential` flow, about 310 src and 150 test lines, and keeps the command.

**Recommended answer.** Yes, as K8 in PR9. Keep the `goalcup mcp` command working through Edition 01
(dry-run participants pasted the lines into four harness configs; if the command vanished, their
harness would show a failed MCP server at start, against point 3). Delete it after Edition 01 (K19).
`goalcup time-left` and the deadline file stay, so the agent can still learn the time left. One line in
the participant text (AD8): "`goalcup mcp` lines in harness configs may stay, the tool keeps working."

What could go wrong: new participants no longer learn of the tool, so an agent that relied on it asks
the shell instead; that works as long as `goalcup` is on its `PATH`.

### Q7. Auto-update switches: why only Claude Code? What about Codex and the others?

**In plain words.** The plan proposed two Claude Code switches. The owner asked why only Claude Code.
The others already have their switch in the connector, or do not update by themselves.

**Facts.**

| Harness | Does it update itself? | While running? | Our connector today | Switch to use | Source |
|---|---|---|---|---|---|
| Claude Code, native install | Yes, downloads and installs in the background; takes effect at the next start | Checks "on startup and periodically while running" | `DISABLE_AUTOUPDATER = "1"` (`connectors/claude-code-cli/harness.toml:60`) | Add `DISABLE_UPDATES = "1"`: blocks every path, including a `claude update` the agent runs. `DISABLE_AUTOUPDATER` alone stops only the background check | https://code.claude.com/docs/en/setup (fetched 29 Oct 2026) |
| Claude Code, npm | Yes, if the npm folder is writable | same | same | same | same page |
| Claude Code, Homebrew, WinGet, apt, dnf, apk | No, unless `CLAUDE_CODE_PACKAGE_MANAGER_AUTO_UPDATE=1` | | | nothing; `DISABLE_UPDATES` covers it (unverified for that variable) | same page |
| Claude Code background sessions | The supervisor restarts itself onto a new version after an update | | | covered by `DISABLE_UPDATES`; plan also turns agent view off (`CLAUDE_CODE_DISABLE_AGENT_VIEW = "1"`) | `D-harnesses.md:268`, `:310` |
| Codex, npm | No. npm never updates a global package by itself | | `-c check_for_update_on_startup=false` (`connectors/codex-cli/harness.toml:46`) | nothing more | |
| Codex, Homebrew cask | No. Homebrew upgrades only when someone runs `brew upgrade` (or installs the third-party autoupdate tap) | | same | tell participants not to run `brew upgrade` during the week | `D-harnesses.md:762` |
| Codex, standalone script | No automatic install. Codex checks at start only (cached 20 hours) and shows a prompt; it installs only if the user picks it | No: no timer in `updates.rs` | same `-c` switch turns the check and prompt off | nothing more | https://github.com/openai/codex/blob/rust-v0.165.0/codex-rs/tui/src/updates.rs (fetched 29 Oct 2026); `D-harnesses.md:700-703` |
| Hermes | No. `hermes update` is manual; a passive check runs once a day | Check only, at chat start | nothing | `updates.check: false` (could be one more `configure` line, `harness.toml:54-57`) | `D-harnesses.md:1035-1048`; https://github.com/NousResearch/hermes-agent/blob/v0.22.5/hermes_cli/config_defaults.py#L2359-L2361 |
| Amp | Update check on by default; runner mode restarts itself for updates | | `AMP_SKIP_UPDATE_CHECK = "1"` (`connectors/amp-cli/harness.toml:45`) | nothing more | `D-harnesses.md:1355-1357` |
| OpenCode (not supported) | No since 1.19.0: notify only; `/update` or `opencode upgrade` installs | | no connector | `"autoupdate": false` in config | https://opencode.ai/docs/config/ (fetched 29 Oct 2026); `D-harnesses.md:1276-1277` |
| OpenClaw (not supported) | Auto-apply off by default; start-up hint on | | no connector | `update.checkOnStart: false`; `OPENCLAW_NO_AUTO_UPDATE=1` stops applies | https://docs.openclaw.ai/install/updating |

A running process keeps its old binary in every case. An update matters only at the next
`goalcup continue`, which starts the new build (`D-harnesses.md:715-716`).

**Recommended answer.** Add `DISABLE_UPDATES = "1"` for Claude Code (one data line and pins). Add
`updates.check false` for Hermes if Hermes stays (one data line). Codex and Amp are already covered.
Nothing for OpenCode and OpenClaw until they have connectors. Tell participants in one line: "Do not run
`brew upgrade`, `npm update -g` or `hermes update` during your run."

What could go wrong: a participant's own setting or a managed setting can override ours; an agent may
still run `npm install -g` on its own. Both only change the build at the next `continue`, and the run
records the version at each start.

### Q8. What is a "provider table" in Codex? Is it an API endpoint?

**In plain words.** No, not an API endpoint. In Codex's config file (`config.toml`), a "table" is a
section that starts with a line in square brackets. A provider table looks like this:

```toml
model_provider = "vercel"

[model_providers.vercel]
base_url = "https://ai-gateway.vercel.sh/codex/v1"
env_key = "AI_GATEWAY_API_KEY"
```

goalcup reads `model_provider`, then the `base_url` in the table with that name
(`connectors/codex-cli/harness.toml:37`), and records that host. Codex also has built-in providers that
need no table. For those there is no address in the file, so goalcup cannot see where the calls go.

**Facts.**

| Built-in provider | Where calls go | Recorded today? | Why not |
|---|---|---|---|
| `openai` (default) | `chatgpt.com` (ChatGPT sign-in) or `api.openai.com` (API key) | Yes: these are the connector's default hosts | |
| `ollama` | `http://localhost:11434/v1` | No, and nothing is said | Plain `http` to this computer. The harness sends it directly: `NO_PROXY` lists `localhost` (`crates/goalcup-cli/src/launch.rs:319-320`), and our proxy only handles HTTPS. goalcup refuses such an address when it is written in the config (`model_host.rs:16-24`), but a built-in has none to refuse |
| `lmstudio` | `http://localhost:1234/v1` | No | same |
| `amazon-bedrock` | `https://bedrock-mantle.<region>.api.aws/openai/v1` (derived from the region) | No, tunnelled unread | The host is not in the file and not on our list, so it passes through closed. Opening it would also need the AWS client to trust our CA, which is unverified |
| `gcp` (new in 0.165) | `https://aiplatform.googleapis.com/v1/projects/<p>/locations/<l>/publishers/openai/v1` | No, tunnelled | same |

Sources: `E-providers.md:194-195` (built-in local and cloud providers, Codex 0.165.0 source
`model-provider-info/src/lib.rs#L690-L823`), `D-harnesses.md:539`, E8.2 rows at `E-providers.md:516-518`.

A correction to the earlier brief: signed requests (AWS SigV4) are not what stops recording. Our proxy
never changes a forwarded byte, so a signature still checks. What stops it is that the address is not
in the config, so it is not on our list; and, if we opened it, a client with its own trust store would
reject our certificate (`E-providers.md:383` for boto3, `:767-775`). Client certificates (Google mTLS
endpoints) would truly break, but no coding harness uses them by default.

**Recommended answer.** The plain choice: "If your Codex config picks a built-in provider that has no
address line (`ollama`, `lmstudio`, `amazon-bedrock`, `gcp`), Goal Cup cannot record your model calls.
Should `goalcup start` (a) stop before the run with a fix, (b) warn and go on, or (c) say nothing?"
Recommend (a) for these four, before the harness starts: a run that records zero model calls is worth
nothing to the participant, and the fix is one table with an `https` address. Stopping before launch does
not block a running harness (point 3). For `openai_base_url`, read it instead of refusing.

What could go wrong: Codex adds a new built-in (it added `gcp` in 0.165), which slips through until we
add it; so also warn on any `model_provider` name that has no table and is not `openai`. A participant
with a local model has no recorded path at all; the guide must say local models are not recorded.

### Q9. Prune a part once the server saved it

**In plain words.** The owner said yes: delete a local chain part once the server has saved it. The
2-hour run found that after pruning, the local check of the trace fails, and the `verify:` line printed
at the end points at a folder that cannot be checked whole.

**Facts.**

- Today a part is deleted 2 hours after the server acked it, or at once under disk pressure; never the
  last part (`crates/goalcup-cli/src/upload/prune.rs:1-31`). After the seal and a finished upload,
  `clean_finished` deletes every part but the last at once (`prune.rs:21-22`, `:120`).
- In the run, all 20 chunks (1.30 GB) were acked 3 s after the seal, and the local trace was pruned at
  once (run report, measure 5).
- Local verify then said `"integrity": "incomplete"`, `"findings": ["chain_pruned_locally"]`, exit 1
  (run: `verify-local.txt`). It did check the links from the first surviving record and the server's
  signed ack (`verified_ack` matches the terminal record). The verifier prints "verify the server copy"
  (`crates/goalcup-verifier/src/bin/goalcup-verify.rs:144-149`; rule at
  `crates/goalcup-verifier/src/capture_analysis/reader.rs:286-290`).
- The finish line still prints `verify: goalcup-verify --capture-integrity <local folder>`
  (`crates/goalcup-cli/src/commands/seal.rs:452`).
- Only organizers can fetch the server copy: `scripts/fetch-run.sh` needs `DATABASE_URL` and R2
  credentials. That is why the server copy was "NOT VERIFIED" and the full trace was never scanned for
  secrets (run report, measures 5 and 8).

So the two do not truly clash. Pruning after the ack is right for a week-long run (about 200 GB of raw
chain at the run's rate). The gaps are: the participant gets a check command that cannot pass, and the
organizer test run had no way to check the server copy.

**Recommended answer.** Yes, prune a part once the server acked it, never the last part, no timer, no
disk trigger (plan CH10). Add three small things:

1. The finish line stops printing a local `verify:` command once parts are pruned. It says: "Your full
   recording is on the server (acknowledged at <time>). This computer keeps the last part only."
2. Organizer test runs keep their parts: either copy the folder every 10 minutes (no code, plan section
   9), or one environment switch that turns pruning off (a few lines).
3. Give the 2-hour run observer a read-only way to fetch the server copy (read-only R2 token and database
   URL in the sandbox), so `fetch-run.sh`, `goalcup-verify` and the secret scan run on the server copy.
   This is the only real proof of point 5.

What could go wrong: once pruned, a server-side loss cannot be repaired from the laptop (R2 is durable,
but a Worker bug that acks before storing would lose data for good). The ack is signed and checked
against the chain head, which limits this. A participant can no longer check their own trace locally.

### Q10. Timeline: can it be done, and the release freeze

**In plain words.** The plan asked to freeze client releases from the Friday release until the last run
ends, except owner-signed fixes (connector updates for harness drift count as such), with
`MIN_COMPETITION_VERSION` set to that release. The owner hopes so, but fears it will not be fully done in
the next 24 hours.

**Facts: the dates.**

| When (UTC) | What | Source |
|---|---|---|
| Thu 29 Oct | today | |
| Tue 3 Nov, end | last day before the feature stop (our own rule) | `TODO.md:42` (4 Nov 00:00) |
| Wed 4 Nov | tag release candidate; 2-hour run on it | plan section 7 |
| Fri 6 Nov 12:00 | planned release; "not a cutoff" | `TODO.md:43`, D70 |
| 7 Nov 00:00 to 8 Nov 00:00 | start window; each run then lasts 168 hours | `TODO.md:44` |

Nothing has to be done in 24 hours. There are five working days before the feature stop. The release
does not work today for a busy run: recording stopped for good after 49 minutes with about 20
subagents, silently (run report, verdict 1).

**Must land by Tuesday 3 Nov** (sizes are rough estimates):

| # | Change | Fixes | Size | Day |
|---|---|---|---|---|
| M1 | Never switch recording off while the daemon answers: delete the failure budget (`MAX_CONNECTION_FAILURES`, `STABLE_EPOCH`, `count_failure`, `crates/goalcup-cli/src/proxy/ingest.rs:101-108`, `:463-471`, `:1106-1131`) and the restart budget; reconnect forever with a short capped backoff (F2, plan CH6) | run cause C3, C4 | about -60 src net, 200 test lines rewritten | Fri 30 |
| M2 | The daemon never drops a whole producer for a full queue (F1, or per-stream loss as the alternative); queue to 64 MiB (F3; `capture.rs:51`, `crates/goalcup-daemon/src/runtime/capture/sink.rs:10`, `:274`) | run cause C1, C2 | about 60 to 120 src, about 300 test lines | Fri 30 to Sat 31 |
| M3 | Every loss leaves a gap with a start and an end (F4); a model call counts only when its request stream is written (F5: move the count from `record_generation_request`, `crates/goalcup-cli/src/proxy/api.rs:397-403`, called at `proxy/websocket.rs:504`, to what the daemon wrote); status and the finish line show "recording off: N minutes" | run cause C5, C6 | about 200 src, 270 tests | Sat 31 |
| M4 | Native caps deleted (F6), one open-stream bound 16,384 with its cost stated, an overflow refuses one stream only (plan CH1, CH2, CH4) | run cause C7, Q2 | about 30 src, 750 test lines rewritten | Fri 30 to Sat 31 |
| M5 | URL password rule, then masking option b (plan AD1, CH8) | point 4 | about 60 src | Fri 30 |
| M6 | Live alarm (F7) and upload constants (F8); measure the zstd long-window ratio (F9) on L1 | silent loss, run cause C9 | about 45 src, 60 tests | Sun 1 |
| M7 | Honest finish line and status: one "Recorded:" line, "not read yet", "recording has stopped" (plan AD2 reading the written count, AD3) | point 6 | about 65 src | Mon 2 |
| M8 | Prune after ack, no local `verify:` after prune (Q9) | point 5 | about 30 src | Mon 2 |
| M9 | Connector data: `DISABLE_UPDATES`, Hermes `updates.check`, Hermes 0.22.5 rows, Codex refusal for built-ins and `openai_base_url` read (Q4, Q7, Q8, plan CH15, AD5) | points 1, 7 | about 40 src plus data and pins | Tue 3 |
| M10 | Worker configuration only: `RUN_MAX_COMPRESSED_BYTES` to the largest value, `MIN_COMPETITION_VERSION`, `EVENT_ID` | point 5 | config | staged Wed 4, production Fri 6 |
| M11 | Re-run the same 2-hour Codex test on the candidate, plus one Claude Code run that asks for subagents, with read-only server access for the observer. Pass: no silent loss, wire calls match the harness's own record per 10-minute bucket or sit inside a shown gap, every subagent file in the chain | proof | no code | Mon 2 (fix build), Wed 4 (candidate) |

**Day by day** (replaces the plan's day plan where they clash):

| Day | Work |
|---|---|
| Fri 30 Oct | M1 (F2), M2 start (F3 and F1), M4 native caps (F6), M5 masking. PR1 docs. |
| Sat 31 Oct | M2 finish, M3 (F4, F5; one control API bump), M4 bound and per-stream refusal |
| Sun 1 Nov | M6 (F7, F8); load test L1; F9 ratio measured on L1's chain |
| Mon 2 Nov | The same Codex prompt for 2 hours in the Vercel Sandbox, plus a 1-hour Claude Code run that asks for subagents; reconcile per 10-minute bucket. M7, M8 text |
| Tue 3 Nov | Fix what Monday found. M9 connector data. Then only cheap pure deletes (K2 scripts, K6 spikes). Feature stop at midnight |
| Wed 4 Nov | Release candidate; the plan's 2-hour run (section 9) with the new pass rule; stage the Worker config (M10) |
| Thu 5 Nov | Buffer |
| Fri 6 Nov | Release; Worker config to production |

**Can wait until after Edition 01, or be dropped if time runs short:** one stream per WebSocket
direction and deduping repeated context (a record format change across proxy, daemon, verifier and the
judges' decoders; accept about 68 GB compressed per week-long run, about $1 a month in R2); the big
deletions (Q5); the "register extra endpoints" add (section 1.3), periodic Hermes snapshots, a
host-list-free proxy.

**If Tuesday slips:** the feature stop is our own rule. Moving it to Wednesday 4 Nov 12:00 still leaves
the Wednesday run and a Thursday buffer. M1, M3 and M4 are the must-haves: without them a busy run loses
recording silently, as on 29 Oct. M2 can fall back to "a much bigger queue only" if F1 is not green by
Monday night, since M1 then turns each drop into a gap and a reconnect instead of a silent stop. That is
the fallback build in section 4.

**Recommended answer.** Yes to the freeze from the Friday release, with owner-signed fixes allowed and
connector updates for harness drift counted as such fixes, and `MIN_COMPETITION_VERSION` set to that
release. The work fits in five days only if the run fixes come first and K1, K10, K11 and K12 move after
Edition 01 (Q5).

What could go wrong: M2 and M3 touch the capture core days before the event; the gate must run the
capture suites, and M11 is the only end-to-end proof. One 2-hour run with 20 subagents does not prove
1,000; the run report says the daemon's peak CPU is unverified. A Worker deploy from the owner's device
on Friday is a single point of failure; stage it on Wednesday. A fix needed mid-cup must still pass the
gate and tie CLI and daemon versions in one release (`packaging/install.sh:104-195`).

---

## 6. Not verified

- Why the daemon fell behind (owner-thread saturation, the native poll on the same thread, or `fsync`).
  Only lifetime CPU averages were captured.
- That 64 MiB is enough for a heavier run. It holds about 40 full-context bodies.
- Peak open streams in any run.
- The zstd long-window ratio on a real chain.
- The server copy of the run (not fetched: `fetch-run.sh` needs `DATABASE_URL` and R2 credentials). The
  run's full 1.3 GB trace was not scanned for secrets (pruned before the scan).
- Whether the large-PUT network errors were the sandbox's egress or a general limit; smaller chunks are
  a cheap hedge either way.
- Hermes 0.22.5 delegation; OpenRouter through Claude Code or Codex; Codex WebSocket through Vercel.
- macOS behaviour of any of this; the run was Linux.
- All new-code line and day figures are estimates.
