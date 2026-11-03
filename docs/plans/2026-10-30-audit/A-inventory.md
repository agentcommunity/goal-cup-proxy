# Appendix A: Inventory of Goal Cup Proxy (Lane A)

Repo: /home/user/goal-cup-proxy, commit 4b226de2 (same as origin/main). Read-only audit of 29 Oct 2026.
Nothing was built, run or tested. This appendix merges five part inventories: A1 (goalcup-cli), A2
(goalcup-daemon and goalcup-platform), A3 (goalcup-protocol and goalcup-verifier), A4 (everything outside
crates/ and docs/) and A5 (docs/ and root Markdown).

**Lane summary.** Goal Cup Proxy has about 102,000 lines of code that ships or runs (85,800 of them are
production Rust in five crates) and about 159,500 lines of tests, so tests are 1.6 times the code. About
66,000 code lines (65 percent) serve at least one of the seven points. About 26,000 lines (26 percent) serve
none of them. The biggest blocks with no point are the verifier's research decoders (9,745), the CLI's event
rules, termination proof and disk code (5,461), the research and hygiene scripts (4,556) and the daemon's
process-tree kill and seal-time workspace walk (3,532). Another 9,400 lines are plumbing, and 5,214 of those
exist only because the recorder runs as a separate daemon process. The caps the owner wants raised are spread
over four crates that must change together, or long runs turn into "invalid evidence". The disk handling the
owner dropped is still in code in three crates, in the Worker, in a script and in 19 doc places. The docs
hold 17,486 lines; about 5,200 are history that can go, and many current docs still state the v5 freeze and
the disk pause as rules.

The 2-hour run report (`docs/research/results/2026-10-30-two-hour-run.md`) does not exist on main or on any
of the 89 remote branches. Nothing in this appendix relies on it, and no number here comes from a long real
run.

## Top findings

1. **A quarter of the code serves no point.** 26,153 of 102,406 src lines (section A.2). The single largest
   cut is the verifier's research decoders, reports and stats: 9,745 src, 16,720 test lines and 1,270 fixture
   lines, with no participant or server use (`crates/goalcup-cli/src/commands/doctor.rs:246-316` is the only
   shipped use, a Warn). But `scripts/matrix.sh:2057-2064` uses `--research-json` to count calls, so the test
   kit needs a small replacement (section A.3, item 5).
2. **The separate daemon process costs about 5,200 src and 6,800 test lines** (control API 1,714, ingest IPC
   1,356, registry 1,009, lifecycle 640, config 236, binary 259). Merging it into the CLI is the largest
   change on the table and the riskiest before 6 November (part A2, observation 2).
3. **Caps of our own sit in four crates that must move together** (section A.4): protocol constants
   (`crates/goalcup-protocol/src/capture.rs:58-66`), daemon native capture and sink
   (`crates/goalcup-daemon/src/native_capture.rs:187,194,273-283`, `sink.rs:183,313,322`), platform handle budget
   (`crates/goalcup-platform/src/handle_budget.rs:13`) and the verifier reader, which marks a trace invalid
   (exit 2) past 128 open streams or connections (`crates/goalcup-verifier/src/capture_analysis/reader.rs:1567,1695`).
   Two CLI caps also go against point 3: a 503 past 1,024 connections (`crates/goalcup-cli/src/proxy/mod.rs:110-121`)
   and an unserved port after 5 recorder restarts (`proxy/mod.rs:1022`, checked at `:1079-1098`).
4. **Disk handling the owner dropped is still everywhere** (section A.5): reserve file, SIGSTOP pause and
   vitals in the CLI (about 1,200 production lines plus 1,173 lines of `full/t25_disk_full.rs`), `ReserveFile` in
   platform (182), disk vitals and the `disk_full` email in the Worker and the website SQL, `scripts/t25-disk-full.sh`,
   and 19 doc places including the hard rule at `AGENTS.md:84-86`.
5. **Process-tree termination is the biggest no-point block in the daemon** (2,428 src, about 4,950 test lines).
   The parts disagree on how much of it can go (section A.3, item 1).
6. **Test code that tests test code.** 5,423 CLI test lines are a Rust stand-in of the TypeScript Worker and
   tests of that stand-in (`crates/goalcup-cli/tests/common/upload_server`, `tests/it/upload_server*.rs`),
   while `full/upload_worker.rs` (456) already runs the real Worker under `wrangler dev`.
7. **CI runs neither the daemon, the CLI nor the Worker suites** (`.github/workflows/ci.yml:73-92` excludes
   goalcup-daemon and goalcup-cli and has no npm step). Only the hand-run `scripts/gate.sh full` covers them.
8. **Point 5 has a server-side gap.** The verify job that rebuilds a run from stored chunks is not built
   (`docs/SERVER-DESIGN.md:157-159`, `docs/OPERATING.md:310`), but `docs/CHEAT-CHECKLIST.md:27-28` says it re-runs
   every hour. Today the rebuild is the organiser's `scripts/fetch-run.sh` plus `goalcup-verify --capture-integrity`.
9. **Back-compat code and freeze text remain.** 29 legacy or compatibility markers in `goalcup-protocol/src`,
   and the v5 freeze is stated as a rule in `docs/CONTRACTS.md:510-560`, `docs/ARCHITECTURE.md:191`, D42, D54 and
   several plans (part A5, group C). The owner decided no backward compatibility before Edition 01.
10. **Masking option b touches known lines.** The email rule is at `crates/goalcup-daemon/src/redact.rs:279-287`
    and `redact/stream.rs:541-543` (recognizer near `:1539-1700`); the Codex rows are at
    `connectors/codex-cli/harness.toml:110-111`. The URL password rule does not exist yet.
11. **Point 1 and 7 risks outside the four shipped harnesses.** OpenClaw's own proxy setting can route model
    traffic past the recorder with no gap; OpenCode and OpenClaw keep sessions in SQLite. Neither can ship as a
    connector by Friday without a wire-only connector (part A4, section 3).
12. **Docs: 17,486 lines, about 5,200 are cut candidates** (history, done plans, fix plans). Two participant-facing
    lines are wrong today: "no header is recorded" (`README.md:150-151`, `docs/ARCHITECTURE.md:214`).

## A.0 How the lines were counted (all parts)

- `wc -l` per file. "src" means production code: lines in `src/` minus each file's trailing `#[cfg(test)] mod`
  block (found by brace matching). Inline test modules, `crates/goalcup-cli/src/daemon/mock.rs`,
  `crates/goalcup-cli/src/participant/tests.rs` and `crates/goalcup-daemon/src/runtime/capture/tests.rs` count as
  tests. Small test-only items in the middle of files (for example 118 lines in `export.rs`, 38 `test-hooks`
  sites in 8 daemon files) stay counted as src, so src slightly overstates shipped code.
- The brief's "about 103k lines of src and 126k lines of tests" counts whole `src/` folders (102,831 lines) and
  `tests/` folders (126,325 lines including 213 shell fixture lines). This appendix splits the inline tests out.
- Points are the seven points of `VISION.md:28-34`. Each part used its own bucket names; this appendix maps them
  to four buckets: **serves a point**, **plumbing** (shared code, including A2's separate-process bucket P),
  **event rules** (only A3 split these out) and **none**. A1 and A2 count event rules and competition needs
  (GitHub repo check, project folder, termination proof, seal-time workspace walk) as "none".
- Mixed-file estimates in A1 come from function boundaries and are plus or minus 30 percent.

## A.1 Totals by area

| Area | src (production) | test code inline in src | tests/ files | all test lines | fixtures and data |
|---|---:|---:|---:|---:|---|
| crates/goalcup-cli | 38,636 | 9,384 | 59,973 | 69,357 | 3,824 test fixtures |
| crates/goalcup-daemon | 18,959 | 4,086 | 30,473 (30,260 `.rs` + 213 shell fixtures) | 34,559 | Cargo.toml 88 |
| crates/goalcup-platform | 5,039 | 856 | 1,684 | 2,540 | Cargo.toml 20 |
| crates/goalcup-protocol | 11,013 | 1,370 | 13,471 | 14,841 | 720 test fixtures; providers.toml 25, pins 7, Cargo.toml 41 |
| crates/goalcup-verifier | 12,153 | 1,335 | 20,724 | 22,059 | 2,182 test fixtures (466 lines of Python); Cargo.toml 47 |
| **crates total** | **85,800** | **17,031** | **126,325** | **143,356** | **6,726 test fixtures** |
| server/worker | 4,001 src + 394 scripts | 0 | 5,749 | 5,749 | sql mirror 4,468; README, wrangler.toml, package files 2,324; vectors and tsconfig 51 |
| scripts/ | 11,176 | 0 | 10,393 (`test_*.py`) | 10,393 | README 79, campaign.example.json 64, research-staging.sql 166 |
| packaging/ | 394 (install.sh) | | tested by scripts/test_install.py (614, counted above) | | README 101 |
| harnesses/install.sh | 62 | | | | |
| spikes/ | 349 (redact.py) | | | | 23,364 (incl. about 1,044 lines of dead prototype code) |
| connectors/ | | | | | 556 |
| skills/ | | | | | 739 |
| .github/ | | | | | 533 |
| docs/ and root Markdown | | | | | 17,486 (79 files) |
| root config and legal, .config, .claude | | | | | about 485 |
| **Total** | **102,176** | **17,031** | | **159,498** | |

## A.2 Lines by point served

Two views. The first puts every src line in one bucket, so it adds up. The second counts a file toward every
point it lists, so rows overlap and do not add up. Both cover the 102,406 src lines of the crates, the Worker
src and scripts, the scripts' 230 data lines, the Worker scripts, packaging, harnesses/install.sh and
spikes/redact.py. Script: `merge/points.py` in the lane scratchpad, which reads the part tables.

**View 1: one bucket per line (src)**

| Area | src | serves a point | plumbing | event rules | none | test lines tied to the "none" src (from the parts) |
|---|---:|---:|---:|---:|---:|---|
| goalcup-cli | 38,636 | 29,666 | 3,509 | (in none) | 5,461 | about 4,637 |
| goalcup-daemon | 18,959 | 9,760 | 5,667 (5,214 separate-process + 453 generic) | (in none) | 3,532 | 5,706 |
| goalcup-platform | 5,039 | 4,337 | 124 | 0 | 578 | 359 |
| goalcup-protocol | 11,013 | 9,360 | 68 | 823 (process, receipt, seal) | 762 (pins, connector_dir, constant_time) | 997 (pins 619, lint 378) |
| goalcup-verifier | 12,153 | 2,369 | 39 | 0 | 9,745 | 16,720 (+1,270 fixtures) |
| server/worker/src | 4,001 | 2,938 | 0 | 0 | 1,063 (db-local 701, github 223, ratelimit 139) | 774 (local-db.test 590, github.test 184) |
| server/worker/scripts | 394 | 0 | 0 | 0 | 394 (ops) | 0 |
| scripts/ (code + data) | 11,406 | 6,850 | 0 | 0 | 4,556 | 3,721 |
| packaging/install.sh | 394 | 394 (7, 3) | 0 | 0 | 0 | 0 |
| harnesses/install.sh | 62 | 0 | 0 | 0 | 62 | 0 |
| spikes/redact.py | 349 | 349 (4) | 0 | 0 | 0 | 0 |
| **Total** | **102,406** | **66,023 (64.5%)** | **9,407 (9.2%)** | **823 (0.8%)** | **26,153 (25.5%)** | **about 32,900** |

Notes: "none" in the CLI and daemon includes event rules and competition needs (see A.0). Some "none" code is
still kept by its part for a reason: `ratelimit.ts` guards a real cost; `db-local.ts` carries every local
Worker test; `kit_settle.py` guards test cost; `third-party-licenses.py` is legal. "Serves a point" does not
mean lean: redaction (4,153 src) and the finish budget (763) serve a point and are still flagged as over-built.

**View 2: src lines in parts that list each point (overlapping)**

| Point | src lines | Largest parts |
|---|---:|---|
| 1. Record every model call | 21,270 | CLI proxy/ (9,212), launch.rs, model_host.rs; daemon native_capture.rs, owner.rs, capture/mod.rs; protocol capture.rs, capture_policy.rs |
| 2. Record every subagent | 12,502 | daemon native_capture.rs (1,632), native_plan.rs; platform sourcecap (1,285); scripts/campaign.py (1,888) |
| 3. Never block or slow | 17,961 | CLI proxy/, retry.rs, upload/policy.rs, prune.rs; daemon sink.rs, writer.rs, owner.rs; packaging/install.sh |
| 4. Never record secrets | 9,952 | daemon redact.rs + redact/stream.rs (4,153); CLI proxy/headers.rs, metadata.rs, model_host.rs, per_run_home.rs; protocol run_secret.rs; spikes/redact.py |
| 5. Upload everything | 27,983 | CLI upload/ (3,956), commands/start/server.rs, seal.rs; platform tracecap (1,833); protocol export.rs (1,872); Worker (2,938); fetch-run.sh; matrix.sh |
| 6. Tell the participant | 7,620 | CLI participant/ (1,788), commands/status.rs, run/mod.rs; Worker mail.ts, notices.ts; daemon status.rs |
| 7. Real harnesses as people run them | 21,511 | CLI launch.rs (1,994), home.rs, prep.rs; protocol harness/* (3,351) and capture_policy.rs (1,077); scripts matrix.sh, linux-matrix.sh, campaign.py |
| none | 26,153 | see view 1 |

## A.3 Where the parts disagree

| # | Topic | One view | Other view | Note |
|---|---|---|---|---|
| 1 | Termination and process code | A2: `terminate/*` and `runtime/termination.rs` (2,428 src) serve no point; a plain `killpg(SIGTERM)`, grace, `killpg(SIGKILL)` would do. A1: subreaper.rs, released_child.rs and the D40 spawn gate are no-point. | A3: keep protocol `process.rs` (497), because the 168-hour stop needs process discovery and the PID reuse guard (a rule, not a point). | Both agree it is not one of the seven points. They differ on whether the event rules still need it. |
| 2 | The D40 spawn gate | A1: the park-before-exec gate in `crates/goalcup-cli/src/launch.rs:1657-1994` (about 340 lines) serves no point. | A2: the daemon side, `register_process_blocking` (`crates/goalcup-daemon/src/control/handlers.rs:465-628`), serves point 1, because recording starts before the harness execs. | A1 also notes the daemon uses the registered process for `harness_build` and seal (`crates/goalcup-daemon/src/runtime/capture/mod.rs:756-776`). Unverified whether the first model call can race the registration without the gate. |
| 3 | Connector pins | A3: cut `pins.rs` (520 src, 619 test lines); four in-repo connectors are reviewed code and the Worker stores the digest but never compares it. | A4: keep `scripts/pin-manifests.sh`, "pins are how a connector is trusted"; A1 counts `preflight/gates.rs` as point 7. | If pins go, pin-manifests.sh, the doctor pin check and `preflight/gates.rs:114-117` go with them. |
| 4 | Vitals | A1: `vitals.rs` (158 src) serves no point (ops telemetry, disk dropped). | A2: platform `disk.rs` `free_bytes` serves point 6 and `machine.rs` process usage weakly serves point 6. | Small either way. |
| 5 | Research decoders | A3: cut all 9,745 src from the Friday build. | A4: keep `scripts/matrix.sh`, which calls `--research-json` (`scripts/matrix.sh:2057-2064`, `2247-2269`); keep `skills/.../writing-a-decoder.md`. | A cut needs a 100 to 200 line call and subagent count to replace what matrix.sh reads. |
| 6 | Seal-time workspace state | A2: `workspace.rs` and `workspace_git.rs` (1,015 src) serve no point (competition need). | A3: keep `seal.rs` and `receipt.rs` in protocol as rules evidence. | Same question as row 1: are event rules in scope for Friday? |
| 7 | Redaction size | A2: `redact/stream.rs` (2,945 src) is over-built; a simpler chunk-overlap approach may do. | A4: keep `spikes/redact.py` as the independent cross-check; A2's own risk note says cutting tests here is riskier than anywhere. | Any rewrite must keep "a secret split across two frames is still caught". |
| 8 | Prune of acked parts under disk pressure | A1: keep `upload/prune.rs` (173 src) as points 3 and 5; a week run may not fit on disk without it. | Owner: "Disk full: no special handling". | Prune deletes data already acked by the server; it is not a pause. A1's view looks compatible with the owner decision, but it does act on disk pressure. |
| 9 | Location of `MAX_RECORDER_RESTARTS` | A1: `crates/goalcup-cli/src/proxy/mod.rs:1019-1022`. | A5: `proxy/mod.rs:162`. | Checked: the constant is at `:1022`; `:162` is a doc comment that names it. Fixed in A5's text below. |
| 10 | What happens after 5 restarts | A5 (from `docs/plans/2026-10-29-record-everything-status.md:53-54`): "the recorder refuses calls". | A1 (from code): the sixth accept-loop death leaves the port unserved, so the harness cannot reach its provider. | Same effect for point 3: the harness is blocked. |

## A.4 Caps of our own (all parts)

| Cap | Value | Where | Kind | Note |
|---|---|---|---|---|
| `MAX_OPEN_STREAMS` | 128 open payload streams per entry | `crates/goalcup-protocol/src/capture.rs:58` (`:25-72,183-202`); `docs/CONTRACTS.md:28` | owner: raise or remove | Verifier session refuses past it (`crates/goalcup-verifier/src/capture_analysis/session.rs:644,761`) |
| `MAX_OPEN_CONNECTIONS` | 128 | `capture.rs:25-72` | owner: raise or remove | Reader marks the trace invalid: `connection_limit_exceeded` (`reader.rs:1695`) |
| `MAX_LIVE_PRODUCERS` | 128 | `capture.rs`; `crates/goalcup-daemon/src/runtime/capture/sink.rs:322`; `docs/ARCHITECTURE.md:189` | owner: raise or remove | |
| `MAX_PRODUCERS` | 65,536 | `capture.rs`; `sink.rs:183,313` | review | |
| Native `max_files` | 1,024 default | `crates/goalcup-daemon/src/native_capture.rs:273-283`; `docs/HARNESS-ONBOARDING.md:198-202` | owner: raise or remove | Past it a file is a `LimitExceeded` gap (`docs/DECISIONS.md:483-485`) |
| Native `max_open_streams` | `MAX_OPEN_STREAMS/4` = 32 | `native_capture.rs:273-283` | owner: raise or remove | One fd per open append-only stream |
| `MAX_NATIVE_FILES` | 20,000 | `native_capture.rs:187` | review | |
| `MAX_FRESH_IDS` | 128 | `native_capture.rs:194` | review | |
| Native handle budget | 32 | `native_capture.rs:80`; `crates/goalcup-platform/src/handle_budget.rs:13` (`MAX_GIT_LIVE_HANDLES_V1`) | review | Guards the macOS 256 fd default, but 32 is far below it |
| `open_stream_limit_exceeded` | trace invalid, exit 2 | `crates/goalcup-verifier/src/capture_analysis/reader.rs:1567`; test `crates/goalcup-verifier/tests/it/m2_capture_integrity.rs:1973-1984` | must move with the daemon | Otherwise long runs read as invalid evidence |
| Verifier stats | `MAX_TRACES` 128, `MAX_IDS` 1,000,000 | `capture_analysis/stats.rs:39-55` | goes with the research cut | |
| `MAX_NATIVE_ROWS` | 8 | `crates/goalcup-protocol/src/harness/check.rs:26` | review | |
| `MAX_SEED_BYTES` | 64 KiB | `crates/goalcup-protocol/src/harness.rs` | review | |
| `MAX_CONNECTION_TASKS` | 1,024, then a `503` to the harness | `crates/goalcup-cli/src/proxy/mod.rs:110-121`; fd limit raised to 2 x 1,024 + 256 at `crates/goalcup-cli/src/commands/start/proxy.rs:24-60` | against point 3 | Can refuse harness traffic |
| `MAX_RECORDER_RESTARTS` | 5 | `proxy/mod.rs:1022`, checked at `:1079-1098`; `docs/plans/2026-10-29-record-everything-status.md:53-54` ("a board decision, not from the spec") | against point 3 | Port left unserved after the sixth death |
| `MAX_CONNECTION_FAILURES` | 10 in a row | `crates/goalcup-cli/src/proxy/ingest.rs:100-104`; `docs/DECISIONS.md:495` (D58) | point 1 risk | Recording stops for the rest of that launch; the harness keeps running. Unverified whether Task 10 changed it |
| Ingest handshakes | 16 pending, 5 s setup; docs say 128 unsettled | `crates/goalcup-daemon/src/ingest.rs:145-158`; `docs/DECISIONS.md:491-496` | review | |
| `MAX_TUNNEL_NOTES` | 10,000 rows plus a dropped count | `crates/goalcup-cli/src/proxy/tunnels.rs:20`; `docs/DECISIONS.md:1145`, `docs/CONTRACTS.md:543-546`, `docs/CHEAT-CHECKLIST.md:124-125` | memory guard, says why | Low risk |
| `MAX_KEPT_OUTAGE_WINDOWS` | 32 | `crates/goalcup-cli/src/proxy/outage.rs:22` | memory guard, says why | Low risk |
| Daemon unlaunched limit, permits | 15 min, 64 in-flight | `crates/goalcup-daemon/src/runtime/daemon.rs:29,103` | review | |
| Finish budget | 180 s, 7 stages | `crates/goalcup-daemon/src/runtime/capture/finish.rs:11-763` | keep a simple timeout | Point 3 needs some deadline at the end |
| Hash limits | workspace 64 MiB, harness build 512 MiB | `crates/goalcup-daemon/src/workspace.rs:24,209`; `runtime/capture/harness_build.rs:28` | go with those files | |
| `MAX_CAPTURE_QUEUE_BYTES` | 8 MiB | `capture.rs` | keep: memory is a real cost | Say so in the code |
| `CHAIN_QUOTA_BYTES` | 1 TiB | `capture.rs`; `docs/CONTRACTS.md:26` | accounting only | Never allocated |
| Worker chunk | 32 MiB | `server/worker/src/chunks.ts:22` | keep: Worker memory | |
| `RUN_MAX_COMPRESSED_BYTES` | 64 GiB | `server/worker/src/context.ts:12`, `server/worker/wrangler.toml:56-60`; `docs/SERVER-DESIGN.md:120`, `docs/UPLOAD-API.md:302` | owner to decide | Storage cost; over it a chunk gets 413 and the client retries |
| Worker JSON, seal document, tunnels list, machine facts | 64 KiB, 16 MiB, 8 MiB, 4 KiB | `server/worker/src/runs.ts:25-29,174`; client 16 MiB at `crates/goalcup-cli/src/upload/uploader/finish.rs:254` | review for week-long runs | Whether seal.json or tunnels.jsonl grow with subagent count is unverified |
| Device flow rate limits | 120 / 30 / 1,200 per minute | `server/worker/src/ratelimit.ts`, `wrangler.toml` | keep: abuse guard | |
| Test design cap | 3 subagents alive at once | `docs/plans/2026-10-29-record-everything-phase2-notes.md:758-759` | conflicts with "no caps of our own" | |

Risk for all raised caps: `max_open_streams` holds one fd per open stream. Thousands of streams hit the macOS
256 default fd limit (unverified on a real run). Either raise the soft limit with `setrlimit` at start, or
open, read and close each file per poll. Memory per open stream in the daemon and reader also grows with no
bound once the caps go.

## A.5 Disk handling the owner dropped (all parts)

| Where | Lines | Part |
|---|---|---|
| `crates/goalcup-cli/src/run/reserve.rs` | 533 src, 423 inline tests | A1 |
| `crates/goalcup-cli/src/run/pause.rs` (D65 SIGSTOP/SIGCONT, also against point 3) | 211 src, 149 inline tests | A1 |
| `crates/goalcup-cli/src/vitals.rs` | 158 src, 51 inline tests | A1 |
| Disk parts of `run/watch.rs`, `run/mod.rs`, `commands/start/phases.rs`, `commands/start/forked.rs` | about 200 + 100 + part of 250 + 30 (est.) | A1 |
| `crates/goalcup-cli/tests/full/t25_disk_full.rs` | 1,173 tests | A1 |
| `crates/goalcup-platform/src/disk.rs` `ReserveFile` (`:58-239`), `tests/it/disk.rs` | 182 src, 137 tests | A2 |
| Worker disk vitals and `disk_full` notice: `server/worker/src/runs.ts:500-530`, `db.ts:252,397`, `notices.ts`; website SQL | about 60 to 120 src | A4 |
| `scripts/t25-disk-full.sh` | 93 | A4 |
| Docs: 19 places, including `AGENTS.md:84-86`, `README.md:79-80`, `docs/RULES.md:324-325`, D65 | see part A5 group A | A5 |

Risk: `docs/research/t25-disk-full.md:8-9` measured that before WP-C "a full disk silently ends the recording".
With no pause, the docs must say plainly that a full disk ends recording (point 6), and the `StorageFull` gap
rule (`docs/RULES.md:322-323`) must stay true.

## A.6 Line-count spot checks

Checked with `wc -l` while merging:

| File or set | Part says | wc -l | Result |
|---|---:|---:|---|
| `crates/goalcup-cli/src/run/reserve.rs` | 956 | 956 | correct |
| `crates/goalcup-daemon/src/redact/stream.rs` | 2,945 src + 1,063 inline | 4,008 | correct |
| `crates/goalcup-verifier/src/capture_analysis/anthropic.rs` | 1,585 + 53 | 1,638 | correct |
| `crates/goalcup-protocol/src/export.rs` | 1,872 + 145 | 2,017 | correct |
| `server/worker/src/runs.ts` | 637 | 637 | correct |
| `crates/goalcup-cli/src/launch.rs`, `crates/goalcup-daemon/src/native_capture.rs`, `scripts/check-links.py`, `scripts/matrix.sh`, `docs/DECISIONS.md` | 2,090, 1,632, 1,349, 2,522, 1,162 | same | correct |
| goalcup-cli `src/` and `tests/*.rs` | 48,020 and 59,973 | same | correct |
| server/worker src and test | 4,001 and 5,749 | same | correct |
| docs + root Markdown | 17,486 | 17,486 | correct |
| spikes/ | 349 + about 23,364 | 23,713 | correct |
| **scripts/ non-test** | **11,327** | **11,485 incl. README (11,406 without; 11,176 code + 230 data)** | **wrong, fixed in A4 below** |
| **daemon `tests/`** | **30,473** | **30,260 in `.rs` + 213 shell fixtures** | **total right, made explicit in A2 below** |
| **docs file counts** | **plans 12, research 18, all 76** | **plans 13, research 20, all 79 (71 under docs/)** | **wrong, fixed in A5 below** |

---

## A.7 goalcup-cli (part A1)

### How this was counted

- Line counts are `wc -l`-equivalent per file. "src" means files under `crates/goalcup-cli/src`.
- "test code in src" is the `#[cfg(test)] mod ... { }` blocks inside each src file, found by brace matching
  (script: `/tmp/claude-0/-home-user/c3485515-d491-51f1-8b7f-6278b4ef8e37/scratchpad/inv.py`). Two whole src files are
  test code too: `src/daemon/mock.rs` (636, behind feature `test-support`) and `src/participant/tests.rs` (702).
- "production" = src total minus test code in src. Comment and doc lines stay in "production".
- "Points" = which of the seven points (VISION.md) the file serves. "plumbing" = shared code every point needs
  (command tree, config, daemon client). "none" = serves none of the seven: event rules (GitHub repo, project folder,
  deadline, time left for the agent), termination proof (D22, D40, subreaper), disk handling the owner dropped
  ("Disk full: no special handling", docs/plans/2026-10-29-record-everything-progress.md:88), back-compat code,
  and ops telemetry.
- "No-point lines" are production lines only. For a whole "none" file it is exact. For a mixed file it is an
  estimate from function boundaries (marked est. below). Tests that go with them are listed separately.

### Headline numbers

| Measure | Lines |
|---|---:|
| src total (92 files) | 48,020 |
| test code inside src (inline `cfg(test)` modules 8,046, plus `daemon/mock.rs` 636, `participant/tests.rs` 702) | 9,384 |
| production src | 38,636 |
| of which comment or doc lines (`//`, `//!`, `///`) | about 9,183 of the 48,020 (19%), blank 2,804 |
| tests/ .rs files (63 files) | 59,973 |
| tests/ fixtures (json, toml, sh, py, hex, http, br) | 3,824 |
| **production src lines that serve no point** | **5,461 (14% of production)**: 4,031 exact in 16 whole files, about 1,430 est. inside 14 mixed files |
| test lines that go with the no-point code | about 4,637: 1,494 inline in the 16 whole files, 3,143 in 6 integration files (it/account.rs 1,195, full/t25_disk_full.rs 1,173, it/capture_gate.rs 519, it/process_handoff.rs 147, full/launcher_sweep.rs 93, it/dependency_bans.rs 16) |

### Totals per folder

| Folder | Files | src total | production | test code in src | No-point production lines | Integration test lines that mainly target it |
|---|---:|---:|---:|---:|---:|---:|
| account | 4 | 1,046 | 1,028 | 18 | 466 | 1,195 (it/account.rs) |
| commands (incl. start/) | 20 | 10,294 | 9,082 | 1,212 | 1,415 | 10,897 |
| daemon | 3 | 2,080 | 1,434 | 646 (mock.rs) | 0 | 2,145 |
| participant (participant.rs + participant/) | 10 | 2,490 | 1,788 | 702 (tests.rs) | 60 | 0 (inline only) |
| preflight | 3 | 277 | 277 | 0 | 30 | 411 |
| proxy | 12 | 10,561 | 9,212 | 1,349 | 0 | 17,764 (7 it files + 6 support modules) |
| run | 5 | 4,978 | 2,859 | 2,119 | 1,304 | 1,173 (t25_disk_full) plus shares of status_views |
| upload | 12 | 4,743 | 3,956 | 787 | 110 | 11,681 (5,423 of it is the Worker stand-in and tests of that stand-in) |
| top-level files | 23 | 11,551 | 9,000 | 2,551 | 2,076 | 8,015 |
| whole-CLI campaigns and vertical (no single folder) | | | | | | 5,610 |
| shared test plumbing (support/mod.rs, mains, secret) | | | | | | 1,082 |
| **Total** | **92** | **48,020** | **38,636** | **9,384** | **5,461** | **59,973** |

Folder split of no-point lines: run is the worst by share (1,304 of 2,859 production, 46%), then top-level files
(2,076 of 9,000, 23%), account (466 of 1,028, 45%), commands (1,415 of 9,082, 16%). Proxy and daemon have none.

### The no-point code, in one list

Whole files (exact production lines; inline tests in brackets):

| File | Lines | Why no point |
|---|---:|---|
| `run/reserve.rs` | 533 [423] | 3 GiB reserve file. Owner: no disk-full handling. |
| `account/github.rs` | 466 [0] | Public-repo check (event rule) plus the pre-D69 credential helper its own header says "can go" (src/account/github.rs:10-15). |
| `commands/start/project.rs` | 363 [0] | Project-folder rules: empty, no git, .env gitignored (event rule and token safety). |
| `subreaper.rs` | 354 [263] | Linux orphan sweep for "Confirmed" termination. |
| `released_child.rs` | 291 [0] | Ends a child released onto a sealed entry (D40 race). |
| `commands/mcp.rs` | 273 [201] | MCP time-left tool for the agent (D66). |
| `run/time_left.rs` | 260 [170] | Local deadline count and the time-left file. |
| `run/pause.rs` | 211 [149] | SIGSTOP/SIGCONT on low disk (D65). Owner dropped disk handling. |
| `harness_version.rs` | 203 [0] | Runs `<harness> --version` and records it. Research metadata. |
| `commands/run_repo.rs` | 197 [0] | Asks and checks the run's GitHub repo. |
| `ssh_keys.rs` | 192 [49] | Passphrase-less SSH key warning. |
| `arming.rs` | 175 [0] | Ctrl-C abandon window of start. |
| `vitals.rs` | 158 [51] | Disk and own-process vitals for heartbeats and status. |
| `push_lines.rs` | 152 [61] | Push lines file and clipboard. |
| `commands/time_left.rs` | 117 [51] | `goalcup time-left`. |
| `skill.rs` | 86 [76] | Writes SKILL.md into the harness's skills folder before each launch. |
| **Total** | **4,031 [1,494]** | |

Parts of mixed files (estimates, from function ranges):

| File | Est. lines | What part |
|---|---:|---|
| `launch.rs` | 340 | D40 spawn gate: park the child on a socketpair until the daemon registered it (src/launch.rs:1657-1994). |
| `commands/start/phases.rs` | 250 | push lines, skill install, project token, reserve and pause wiring (src/commands/start/phases.rs:464-630 and around). |
| `run/watch.rs` | 200 | disk look/follow, reserve resize, pause, time-left file (src/run/watch.rs:222-360, 471-530). |
| `commands/prep.rs` | 120 | SSH warning, time-left tool advice, no-prompts advice (src/commands/prep.rs:420, 739-762, 911-950). |
| `upload/uploader/heartbeat.rs` | 110 | local deadline count `watch_deadline` / `LocalDeadline` (src/upload/uploader/heartbeat.rs:153-308). |
| `run/mod.rs` | 100 | disk state, deadline and "is over" helpers. |
| `entries.rs` | 60 | run-public-key pinning to defend `status` from a forged signed status (anti-forgery, not a point). |
| `participant/times.rs` | 60 | time-left wording. |
| `home.rs` | 40 | old home lock path, back-compat (src/home.rs:118-156). |
| `commands/doctor.rs` | 40 | SSH-key section (src/commands/doctor.rs:433). |
| `commands/start/forked.rs` | 30 | reserve end and release (src/commands/start/forked.rs:255-274). |
| `preflight/probe.rs` | 30 | sweep of probe files an older doctor left (src/preflight/probe.rs:18-46). |
| `commands/status.rs` | 25 | push-lines section (src/commands/status.rs:472-491). |
| `main.rs` | 25 | hidden `git-credential` command (src/main.rs:233-240, 327-335). |
| **Total** | **about 1,430** | |

### Other findings that matter for the cut

1. **Caps of our own inside this crate**: listed with every citation in section A.4 (`MAX_CONNECTION_TASKS`,
   `MAX_RECORDER_RESTARTS`, `MAX_CONNECTION_FAILURES`, `MAX_TUNNEL_NOTES`, `MAX_KEPT_OUTAGE_WINDOWS`).
2. **Test code in src**: 9,384 lines (20% of src). Plus about 80 `feature = "test-support"` lines and 17
   `GOALCUP_TEST_*` environment seams (`grep -rn GOALCUP_TEST_ src`), each a branch a shipped build compiles out
   but a reader must still skip.
3. **The Worker stand-in**: tests/common/upload_server (2,514 lines) is a Rust copy of the TypeScript Worker, and
   it/upload_server*.rs (2,909 lines) test that copy. 5,423 test lines exist to emulate a server that
   full/upload_worker.rs (456) already runs for real under `wrangler dev`.
4. **Comment weight**: about 19% of src lines are comments. Many module headers explain removed V1 parts
   (D48, OS-29) instead of current code, for example src/arming.rs:1-20 and src/preflight/mod.rs:1-8.
5. **Stale text**: src/account/mod.rs:8-11 still says github "clones it ... and answers git's credential protocol",
   which D69 removed. Cargo.toml comments still say "nine-command tree" (crates/goalcup-cli/Cargo.toml:44) while main.rs ships eleven.
6. **Biggest single parts serving points**: proxy/ (9,212 production, points 1 to 4) and its 17,764 test lines;
   launch.rs (2,090, point 7); commands/start/ (4,822 total, the run itself); upload/ (3,956 production, point 5).

### Blind spots and risks of calling these "no point"

- **Event rules are not in the seven points but are in docs/RULES.md.** GitHub repo checks, project-folder checks,
  time left and push lines may still be required by the rules. Cutting them moves the check to the server, the
  verifier or the participant. Check RULES.md before any cut.
- **Termination proof** (subreaper, released_child, spawn gate, process handoff): the daemon uses the registered
  process for `harness_build` and for termination at seal (crates/goalcup-daemon/src/runtime/capture/mod.rs:756-776).
  Registration itself must stay if the daemon keeps those. Only the park-before-exec gate and the sweeps are the
  extra part. Unverified whether the verifier's "Confirmed" verdict depends on them.
- **Disk code** (reserve, pause, vitals, parts of watch): the owner said no special handling. But `upload/prune.rs`
  (deleting acked chain parts) is kept here as points 3 and 5, since a week run with thousands of subagents may not
  fit on disk without it. If prune also goes, long runs can fill the disk faster.
- **harness_version.rs** might be judged point 6 or 7 by someone else (it tells readers which build ran). Cost is
  small (203 lines).
- **Mixed-file estimates** are from function boundaries, not a line-by-line read. Treat them as plus or minus 30%.

### Per-file inventory of src

Columns: src lines (total) = whole file; production = total minus inline test module; test code in src = inline
`cfg(test)` lines (or the whole file for test-only files); no-point lines = production lines serving none of the
seven points (exact for whole files, est. for mixed).

#### account

| File | src lines (total) | production | test code in src | What it does | Points | No-point lines (est.) | Matching tests (crates/goalcup-cli/tests unless inline) |
|---|---:|---:|---:|---|---|---:|---|
| `account/credential.rs` | 176 | 176 | 0 | Keeps the run-bound upload credential (gcr_...) at mode 0600 in the run folder; never printed. | 5 (and 4) | 0 | it/upload_open.rs (part), full/upload_e2e.rs (part) |
| `account/device.rs` | 389 | 371 | 18 | Browser device-code approval for a run or a login against the upload server; polls until approved. | 5 | 0 | it/upload_server_device.rs (tests the stand-in), it/upload_open.rs (part), full/upload_worker.rs (part) |
| `account/github.rs` | 466 | 466 | 0 | Checks a real run's GitHub repo is public and empty; plus the pre-D69 git credential helper kept 'for old clones' (its own header says it can go). | none (event rule: public repo; back-compat helper) | 466 | it/account.rs 1195 |
| `account/mod.rs` | 15 | 15 | 0 | Module list for device, credential, github. Its header still says github 'clones it' and answers credentials; out of date after D69. | plumbing | 0 | none |
| **account total** | **1046** | **1028** | **18** | | | **466** | |

#### commands

| File | src lines (total) | production | test code in src | What it does | Points | No-point lines (est.) | Matching tests (crates/goalcup-cli/tests unless inline) |
|---|---:|---:|---:|---|---|---:|---|
| `commands/continue_run.rs` | 494 | 417 | 77 | goalcup continue: start the next segment of a run after the agent stopped, with resume_args. | 1, 2 (whole 168 h run) | 0 | it/restarts.rs 497 (part), full/upload_e2e.rs (part) |
| `commands/doctor.rs` | 556 | 471 | 85 | goalcup doctor: connector pin, harness on PATH, decoders, daemon reachability, SSH-key warning. | 7 (SSH warning: none) | 40 | it/preflight_doctor.rs 478, it/capture_only_refusal.rs 212 (part) |
| `commands/entries.rs` | 82 | 82 | 0 | goalcup entries: list runs in the daemon registry. | 6 | 0 | it/status_views.rs (part) |
| `commands/login.rs` | 84 | 84 | 0 | goalcup login: device approval, store server URL and account handle. | 5 | 0 | it/login.rs 159 |
| `commands/mcp.rs` | 474 | 273 | 201 | goalcup mcp: a tiny MCP server whose one tool returns time left to the agent (D66). | none (agent convenience) | 273 | inline 201 |
| `commands/mod.rs` | 29 | 29 | 0 | Module list of the eleven sub-commands. | plumbing | 0 | contract_gate.rs (command tree) |
| `commands/prep.rs` | 1364 | 1301 | 63 | goalcup setup: login, pick harness, set up its home, machine checks (power, disk, network, filesystem), printed as ok/warn/todo/fail. | 7, 5 (SSH, time-left tool and no-prompts advice: none) | 120 | it/prep.rs 1600 |
| `commands/run_repo.rs` | 197 | 197 | 0 | Asks a real run's GitHub repo name, opens GitHub's new-repo page, checks public and empty, remembers the name. | none (event rule) | 197 | it/account.rs (part) |
| `commands/seal.rs` | 784 | 586 | 198 | goalcup seal: ask the daemon to seal an entry early and report how the finish went. | 5 | 0 | it/seal_scan.rs 206, m2_cli_lifecycle.rs (part) |
| `commands/start/finish.rs` | 609 | 545 | 64 | Phase 5: stop proxy, seal within the finish budget, say what was recorded ('recording works' line). | 5, 6 | 0 | m2_cli_lifecycle.rs (part), full/m2_synthetic_vertical.rs (part) |
| `commands/start/forked.rs` | 334 | 316 | 18 | Typed states of an entry after fork (gate, released, exited), upload start, reserve release. | 1, 5 (reserve release: none) | 30 | m2_cli_lifecycle.rs (part) |
| `commands/start/mod.rs` | 591 | 591 | 0 | goalcup start orchestration: options, capture() order, continue_segment, test pause/panic seams. | 1, 2, 5, 7 | 0 | it/start_launch.rs 1481, m2_cli_lifecycle.rs 1944 |
| `commands/start/phases.rs` | 1713 | 1511 | 202 | The phases: prepare, claim shared home, create and gate (abandon window), register and release, run harness; also push lines, skill install, project token, reserve and pause wiring. | 1, 2, 3, 7 (push lines, skill, project token, reserve, pause: none) | 250 | it/start_launch.rs, m2_cli_lifecycle.rs, it/hermes_offline_key.rs 626, it/model_host_from.rs (part) |
| `commands/start/project.rs` | 363 | 363 | 0 | Checks the project folder before approval: empty but for .gitignore/.env, outside git and sync folders, .env gitignored. | none (event rule and token safety) | 363 | it/upload_open.rs (part), it/start_launch.rs (part) |
| `commands/start/proxy.rs` | 437 | 317 | 120 | Starts the in-process recording proxy, raises the open-file limit, admission line, checks the signed status for 'sealed'. | 1, 2, 3 | 0 | m2_cli_lifecycle.rs (part) |
| `commands/start/refusals.rs` | 229 | 229 | 0 | Refusal sentences for an interrupted start, run-folder checks, provider reachability probe and its test seams. | 3, 7 | 0 | it/start_launch.rs (part), it/model_host_from.rs (part) |
| `commands/start/server.rs` | 546 | 546 | 0 | Server side of start: prep checks, approval, open run or next segment, give back an unused credential. | 5 | 0 | it/upload_open.rs 1548, full/upload_e2e.rs (part) |
| `commands/status.rs` | 814 | 764 | 50 | goalcup status: verify signed status against the pinned key, show run state, problems, push lines. | 6 (push section: none) | 25 | it/status_views.rs 1869, it/native_silence.rs 152 |
| `commands/time_left.rs` | 168 | 117 | 51 | goalcup time-left: print what is left of the run (file inside the harness, run state outside). | none (agent convenience, D66) | 117 | inline 51 |
| `commands/upload.rs` | 426 | 343 | 83 | goalcup upload: finish uploading a run whose start could not (server away, laptop slept, killed). | 5 | 0 | it/upload_open.rs (part), full/upload_e2e.rs (part), full/upload_worker.rs (part) |
| **commands total** | **10294** | **9082** | **1212** | | | **1415** | |

#### daemon

| File | src lines (total) | production | test code in src | What it does | Points | No-point lines (est.) | Matching tests (crates/goalcup-cli/tests unless inline) |
|---|---:|---:|---:|---|---|---:|---|
| `daemon/mock.rs` | 636 | 0 | 636 | MockDaemonClient for tests, behind feature test-support. Test code living in src. | test code | 0 | used by it/daemon_client.rs and most command tests |
| `daemon/mod.rs` | 724 | 724 | 0 | The control-API client: one loopback transport, bearer auth, no retry; capture create/register/seal/status calls. | plumbing for 1, 2, 5 | 0 | it/daemon_client.rs 1248 |
| `daemon/spawn.rs` | 720 | 710 | 10 | Starts goalcup-daemon on demand with a clean environment and waits for it (run-scoped daemon). | plumbing for 1, 2 (and 4: no env leak) | 0 | full/run_scoped_daemon.rs 897, contract_gate.rs (part) |
| **daemon total** | **2080** | **1434** | **646** | | | **0** | |

#### participant

| File | src lines (total) | production | test code in src | What it does | Points | No-point lines (est.) | Matching tests (crates/goalcup-cli/tests unless inline) |
|---|---:|---:|---:|---|---|---:|---|
| `participant.rs` | 113 | 113 | 0 | Root of the participant wording module: rules for plain text and shared helpers. | 6 | 0 | participant/tests.rs |
| `participant/approval.rs` | 126 | 126 | 0 | Wording around the browser approval. | 6 (5) | 0 | participant/tests.rs (part) |
| `participant/ending.rs` | 143 | 143 | 0 | Wording at the end of a part of a run. | 6 | 0 | participant/tests.rs (part) |
| `participant/launch.rs` | 186 | 186 | 0 | Block printed just before launch (includes the prompt/push block). | 6 | 0 | participant/tests.rs (part) |
| `participant/refusals.rs` | 313 | 313 | 0 | Refusal wording for start and continue. | 6 | 0 | participant/tests.rs (part) |
| `participant/seal.rs` | 79 | 79 | 0 | goalcup seal wording. | 6 | 0 | participant/tests.rs (part) |
| `participant/status.rs` | 552 | 552 | 0 | goalcup status wording: where the run is, problems, next step. | 6 | 0 | participant/tests.rs (part), it/status_views.rs (part) |
| `participant/tests.rs` | 702 | 0 | 702 | Unit tests of the participant wording (a test file under src). | test code | 0 | (is itself a test file) |
| `participant/times.rs` | 169 | 169 | 0 | Times and time-left wording. | 6 (time-left parts: none) | 60 | participant/tests.rs (part) |
| `participant/upload.rs` | 107 | 107 | 0 | Wording for goalcup upload and entries. | 6 | 0 | participant/tests.rs (part) |
| **participant total** | **2490** | **1788** | **702** | | | **60** | |

#### preflight

| File | src lines (total) | production | test code in src | What it does | Points | No-point lines (est.) | Matching tests (crates/goalcup-cli/tests unless inline) |
|---|---:|---:|---:|---|---|---:|---|
| `preflight/gates.rs` | 171 | 171 | 0 | Check/report shape for doctor and the connector directory pin gate. | 7 | 0 | it/preflight_doctor.rs (part), shipped_connectors.rs (part) |
| `preflight/mod.rs` | 36 | 36 | 0 | Module root and re-exports for pre-run checks. | 7 | 0 | it/preflight_doctor.rs (part) |
| `preflight/probe.rs` | 70 | 70 | 0 | PATH walk to resolve a harness binary; sweep of probe files an older doctor left. | 7 (sweep: none) | 30 | it/preflight_doctor.rs (part) |
| **preflight total** | **277** | **277** | **0** | | | **30** | |

#### proxy

| File | src lines (total) | production | test code in src | What it does | Points | No-point lines (est.) | Matching tests (crates/goalcup-cli/tests unless inline) |
|---|---:|---:|---:|---|---|---:|---|
| `proxy/api.rs` | 824 | 824 | 0 | Relay I/O types, capture policy, CaptureEmitter that orders frames without blocking relays. | 1, 2, 3 | 0 | it/m2_emitter.rs 1517 |
| `proxy/connect.rs` | 1565 | 1281 | 284 | CONNECT admission, SNI peek, upstream TLS validation, intercept vs tunnel decision, upstream retry. | 1, 3 | 0 | it/m2_tls_connect.rs 1275, inline 284 |
| `proxy/headers.rs` | 403 | 267 | 136 | Header streams per exchange with credential values masked before they reach the chain. | 1, 4 | 0 | it/m2_http_relay.rs (part), inline 136 |
| `proxy/http.rs` | 2257 | 2052 | 205 | HTTP/1.1 relay: forward bytes unchanged, decode bodies (gzip, br, zstd, deflate), hand to emitter, upgrade to WebSocket. | 1, 3 | 0 | it/m2_http_relay.rs 2249, inline 205 |
| `proxy/ingest.rs` | 1177 | 1136 | 41 | Bounded IPC sink to the daemon's ingest port, reconnect and fresh-epoch recovery (gives up after 10 failures in a row). | 1, 3, 5 | 0 | it/m2_capture_sender.rs 1991, it/m2_ipc_compose.rs 1606 |
| `proxy/metadata.rs` | 407 | 254 | 153 | Allowlisted, sanitized exchange metadata (method, host, path without query, status, content type) and capture clock. | 1, 4 | 0 | inline 153, it/m2_websocket_relay.rs (part) |
| `proxy/mod.rs` | 1532 | 1498 | 34 | Proxy lifecycle: listener, per-launch CA, accept loop with 1024-connection cap (503 over it) and up to 5 restarts, drain on stop. | 1, 2, 3 | 0 | it/m2_proxy_integration.rs 3025 |
| `proxy/outage.rs` | 316 | 144 | 172 | Coalesces upstream connect failures into outage windows (keeps 32). | 3 | 0 | inline 172 |
| `proxy/preflight.rs` | 420 | 250 | 170 | Before start: one bounded TCP connect to each intercepted provider host; refuses or warns if unreachable. | 1, 7 | 0 | inline 170, it/model_host_from.rs (part) |
| `proxy/tls.rs` | 295 | 295 | 0 | Per-launch CA and leaf certificates, upstream rustls config with webpki roots. | 1 | 0 | it/m2_tls_connect.rs (part) |
| `proxy/tunnels.rs` | 285 | 192 | 93 | List of tunnels relayed unread (host, port, reason, bytes), written at finish; capped at 10,000 rows. | 6, 3 | 0 | inline 93, it/upload_server.rs (part), it/restarts.rs (part) |
| `proxy/websocket.rs` | 1080 | 1019 | 61 | WebSocket relay: forward at once, then unmask/inflate a copy and emit one stream per message. | 1, 3 | 0 | it/m2_websocket_relay.rs 2074, inline 61 |
| **proxy total** | **10561** | **9212** | **1349** | | | **0** | |

#### run

| File | src lines (total) | production | test code in src | What it does | Points | No-point lines (est.) | Matching tests (crates/goalcup-cli/tests unless inline) |
|---|---:|---:|---:|---|---|---:|---|
| `run/mod.rs` | 1747 | 1137 | 610 | Run record and run state (state.json), advice for status, native-silence and no-model-calls notes, run folders. | 6, 5 (disk and deadline fields: none) | 100 | inline 610, it/status_views.rs (part), it/native_silence.rs |
| `run/pause.rs` | 360 | 211 | 149 | SIGSTOP/SIGCONT the agent when the disk is nearly full (D65). | none (owner: no disk-full handling; also manages the agent) | 211 | inline 149, full/t25_disk_full.rs (part) |
| `run/reserve.rs` | 956 | 533 | 423 | A 3 GiB physically allocated reserve file and resize rules. | none (owner: no disk-full handling) | 533 | inline 423, full/t25_disk_full.rs 1173 |
| `run/time_left.rs` | 430 | 260 | 170 | Counts the run down from the server's last answer on a continuous clock; time-left file for the agent. | none (deadline and agent convenience) | 260 | inline 170 |
| `run/watch.rs` | 1485 | 718 | 767 | 30 s tick: prune acked parts, measure disk, resize reserve, pause/resume, write run state for status. | 5, 6 (disk, reserve, pause, time-left parts: none) | 200 | inline 767, it/status_views.rs (part) |
| **run total** | **4978** | **2859** | **2119** | | | **1304** | |

#### upload

| File | src lines (total) | production | test code in src | What it does | Points | No-point lines (est.) | Matching tests (crates/goalcup-cli/tests unless inline) |
|---|---:|---:|---:|---|---|---:|---|
| `upload/api.rs` | 342 | 291 | 51 | Upload API documents (request and answer types). | 5 | 0 | inline 51, it/upload_chunks.rs (part) |
| `upload/http.rs` | 372 | 372 | 0 | The one HTTPS client to the upload server (rustls + ureq), bounded answers, no retry. | 5 | 0 | it/upload_open.rs (part) |
| `upload/mod.rs` | 523 | 523 | 0 | open_run, continue_run, receipts, pending opens and give-back, abandon, wait for stopping. | 5 | 0 | it/upload_open.rs (part), full/upload_e2e.rs (part) |
| `upload/policy.rs` | 385 | 292 | 93 | Chunk timing, backoff, which answers end a manual upload, wrap-up length. | 5, 3 | 0 | inline 93, it/upload_chunks.rs (part) |
| `upload/prune.rs` | 421 | 173 | 248 | Which acked chain parts may be deleted, after a delay or at once under disk pressure. | 5, 3 (lets a week run fit on disk) | 0 | inline 248, it/restarts.rs (part) |
| `upload/uploader.rs` | 82 | 82 | 0 | Uploader root: logical offsets across chain parts, module list. | 5 | 0 | it/upload_chunks.rs (part) |
| `upload/uploader/chunk.rs` | 227 | 227 | 0 | Builds a chunk of whole chain lines as one zstd frame in a spool file. | 5 | 0 | it/upload_chunks.rs 1736 (part) |
| `upload/uploader/finish.rs` | 475 | 354 | 121 | Upload wrap-up: final heartbeat, rest of chain, seal, tunnels list, report. | 5 | 0 | inline 121, full/upload_e2e.rs (part) |
| `upload/uploader/heartbeat.rs` | 417 | 308 | 109 | Heartbeats every few minutes, stop answers, and a local deadline count when the server is away. | 5 (local deadline count: none) | 110 | inline 109 |
| `upload/uploader/lifecycle.rs` | 661 | 577 | 84 | Threads beside a run (heartbeat, upload), finish, drain of an earlier segment. | 5 | 0 | inline 84, full/upload_e2e.rs (part) |
| `upload/uploader/local.rs` | 327 | 246 | 81 | Upload lock, local chain reads at offsets, workspace commit, spool notes. | 5 | 0 | inline 81 |
| `upload/uploader/session.rs` | 511 | 511 | 0 | One entry's uploader: server offset, send chunk by chunk, check and keep acks. | 5 | 0 | it/upload_chunks.rs (part), full/upload_e2e.rs 2518, full/upload_worker.rs 456 |
| **upload total** | **4743** | **3956** | **787** | | | **110** | |

#### top-level

| File | src lines (total) | production | test code in src | What it does | Points | No-point lines (est.) | Matching tests (crates/goalcup-cli/tests unless inline) |
|---|---:|---:|---:|---|---|---:|---|
| `arming.rs` | 175 | 175 | 0 | Ctrl-C handler and the 'abandon window' flag of goalcup start; the V1 arming gate is gone, the name stayed. | none (housekeeping on interrupt) | 175 | inline only via start tests |
| `ca_bundle.rs` | 207 | 207 | 0 | Builds SSL_CERT_FILE / REQUESTS_CA_BUNDLE: proxy CA plus OS roots, so tool children keep working through the proxy. | 1, 3, 7 | 0 | it/ca_bundle.rs 88 |
| `config.rs` | 616 | 426 | 190 | Goal Cup's own config dir ($GOALCUP_CONFIG_DIR), control.json, daemon.toml, secret-file writes. | plumbing (4: never borrows a harness home) | 0 | contract_gate.rs 782 (part), env_resolution.rs (part) |
| `connector.rs` | 167 | 133 | 34 | Finds a shipped connector by the id its harness.toml declares; pin allowlist. | 7 | 0 | shipped_connectors.rs 411 |
| `entries.rs` | 208 | 208 | 0 | Per-entry cache in <cfg>/entries: the run public key pinned at creation so status can verify the signed status. | 6 (anti-forgery part: none) | 60 | it/status_views.rs (part) |
| `folder_gate.rs` | 224 | 156 | 68 | WSL1/WSL2 and filesystem-type checks for the folders a run writes to (APFS, ext4, XFS). | 7 | 0 | inline 68 |
| `harness_version.rs` | 203 | 203 | 0 | Runs '<harness> --version' once and records the first line (reported, not pinned, D53). | none (research metadata) | 203 | it/harness_version_probe.rs 195 (part) |
| `home.rs` | 995 | 700 | 295 | Shared harness homes (D39, D59): judge, lock one run at a time, repair loose modes, the run's own folder, login profile. | 7 (old lock path: none) | 40 | inline 295, it/start_launch.rs (part) |
| `host_log.rs` | 253 | 253 | 0 | Routes every host message to host.log while a chat-mode harness owns the terminal. | 3, 6 | 0 | it/native_silence.rs, it/model_host_from.rs (part) |
| `launch.rs` | 2090 | 1994 | 96 | Builds argv and env from nothing, resolves the harness binary, CA files, prep launch, after-exit hooks, and the D40 spawn gate (park child until daemon registered it). | 7, 1, 4 (spawn gate about 340 lines: none) | 340 | it/harness_launch_plan.rs 782, it/capture_gate.rs 519, it/process_handoff.rs 147, env_resolution.rs 217, it/harness_contract_golden.rs 975, it/harness_contract_kit.rs 805, it/connector_contract.rs 994 |
| `lib.rs` | 212 | 203 | 9 | Crate root: module list, CliError and the exit-code ladder, finish budget and chat test seams. | plumbing | 0 | contract_gate.rs (part) |
| `main.rs` | 974 | 953 | 21 | clap command tree, exit codes, help text, hidden git-credential command, subreaper arm. | plumbing (git-credential: none) | 25 | contract_gate.rs (part) |
| `model_host.rs` | 1540 | 893 | 647 | Reads the model service address from the harness's own config (D62) so that host is intercepted; masks userinfo. | 1, 7, 4 | 0 | it/model_host_from.rs 793, inline 647 |
| `per_run_home.rs` | 489 | 489 | 0 | Makes a fresh private harness home inside the run folder from seed files (Amp uses it). | 7, 4 | 0 | it/per_run_home.rs 438, full/per_run_lifecycle.rs 366 |
| `prep_file.rs` | 88 | 88 | 0 | <cfg>/prep.json: harness picked, home ready, last repo name; read by start. | 7 | 0 | it/prep.rs (part), it/account.rs (part) |
| `push_lines.rs` | 213 | 152 | 61 | Keeps the 'push lines' for the prompt in push-lines.txt, copies to clipboard, shows them again. | none (event rule: push to repo) | 152 | inline 61 |
| `released_child.rs` | 291 | 291 | 0 | Ends a just-released harness child if the entry was found sealed in between (D40 race). | none (termination proof) | 291 | it/capture_gate.rs (part) |
| `retry.rs` | 1165 | 474 | 691 | The one retry of a busy daemon: generic, entry/process schedules, seal retry against the finish budget. | plumbing for 3, 5 | 0 | inline 691, it/daemon_client.rs (part), it/seal_scan.rs (part) |
| `server.rs` | 212 | 212 | 0 | <cfg>/server.json: upload server URL and account handle. | 5 | 0 | it/login.rs (part) |
| `skill.rs` | 162 | 86 | 76 | Installs the goal-cup SKILL.md into the harness's skills folder before each launch. | none (tells the agent rules; writes into the harness home) | 86 | inline 76 |
| `ssh_keys.rs` | 241 | 192 | 49 | Warns if a passphrase-less SSH key or loaded ssh-agent could let the agent push anywhere. | none (participant safety) | 192 | inline 49 |
| `subreaper.rs` | 617 | 354 | 263 | Linux: become child subreaper and sweep processes the harness left behind before the seal. | none (termination proof) | 354 | inline 263, full/launcher_sweep.rs 93 |
| `vitals.rs` | 209 | 158 | 51 | Disk and goalcup-process vitals in heartbeats and status. | none (disk handling dropped; ops telemetry) | 158 | inline 51, it/upload_chunks.rs (part) |
| **top-level total** | **11551** | **9000** | **2551** | | | **2076** | |


### Test files (crates/goalcup-cli/tests)

Fixtures (non-.rs) add 3,824 lines: harness-golden JSON 2,110, connector and kit TOML, stand-in scripts, TLS and HTTP samples.

##### tests/  (3354 lines)

| Test file | Lines | Tests what | Covers | Points |
|---|---:|---|---|---|
| `contract_gate.rs` | 782 | exit ladder, eleven-command tree, config dir, control.json | lib.rs, main.rs, config.rs, daemon/spawn.rs | plumbing |
| `env_resolution.rs` | 217 | tests that mutate process env | launch.rs, config.rs | 4, 7 |
| `m2_cli_lifecycle.rs` | 1944 | start and seal on a real in-process daemon with a fake harness | commands/start/*, commands/seal.rs, run/reserve.rs | 1, 5 (reserve rows: none) |
| `shipped_connectors.rs` | 411 | shipped connectors found by id, pins agree | connector.rs, preflight, config.rs | 7 |

##### tests/common/  (12069 lines)

| Test file | Lines | Tests what | Covers | Points |
|---|---:|---|---|---|
| `common/capture_campaign_support/mod.rs` | 337 | shared campaign support | whole CLI | 3 |
| `common/capture_crash_support/mod.rs` | 172 | crash campaign support | whole CLI | 3 |
| `common/capture_failure_support/mod.rs` | 152 | failure campaign support | whole CLI | 3 |
| `common/m2_cli_support/mod.rs` | 1673 | in-process daemon rig for m2_cli_lifecycle | commands/start | 1, 5 |
| `common/m2_common/mod.rs` | 240 | loopback PKI and helpers | proxy | 1 |
| `common/m2_http_support/mod.rs` | 486 | in-memory peers, recording sink | proxy/http.rs | 1 |
| `common/m2_profile_support/mod.rs` | 171 | fake harness that records its launch | launch.rs | 7 |
| `common/m2_proxy_support/mod.rs` | 1300 | direct sink, upstream fixtures | proxy/mod.rs | 1 |
| `common/m2_sender_support/mod.rs` | 693 | ingest endpoint emulator | proxy/ingest.rs | 1, 3 |
| `common/m2_tls_support/mod.rs` | 525 | TLS fixtures | proxy/connect.rs, proxy/tls.rs | 1 |
| `common/m2_vertical_support/endpoint.rs` | 692 | controlled TLS upstream | whole CLI | 1 |
| `common/m2_vertical_support/mod.rs` | 800 | real binaries rig for the vertical | whole CLI | 1, 2, 5 |
| `common/m2_vertical_support/wire.rs` | 356 | HTTP/WS framing for test peers | whole CLI | 1 |
| `common/m2_ws_support/mod.rs` | 783 | RFC 6455 frame encoder | proxy/websocket.rs | 1 |
| `common/mod.rs` | 35 | module glue | shared | test plumbing |
| `common/real_home_guard.rs` | 171 | snapshots real harness homes to prove no write | launch.rs, per_run_home.rs | 7, 4 |
| `common/secret_support/mod.rs` | 9 | one fake provider key | shared | 4 |
| `common/support/mod.rs` | 960 | shared helpers (mock daemon setups, connectors, entries) | shared | test plumbing |
| `common/upload_server/handlers.rs` | 1506 | stand-in routes | upload | 5 |
| `common/upload_server/mod.rs` | 603 | local stand-in of the Worker upload server | upload | 5 |
| `common/upload_server/state.rs` | 405 | stand-in state on disk | upload | 5 |

##### tests/it/  (35457 lines)

| Test file | Lines | Tests what | Covers | Points |
|---|---:|---|---|---|
| `it/account.rs` | 1195 | GitHub repo check, repo question, old credential helper | account/github.rs, commands/run_repo.rs | none |
| `it/ca_bundle.rs` | 88 | CA bundle content and mode | ca_bundle.rs | 1, 7 |
| `it/capture_gate.rs` | 519 | D40 spawn gate and ending a released child | launch.rs spawn gate, released_child.rs | none (termination) |
| `it/capture_only_refusal.rs` | 212 | schema-1 capture behaviour, doctor without spawning | commands/doctor.rs, commands/start | 7 |
| `it/connector_contract.rs` | 994 | static rules every shipped connector is held to | launch.rs, connector.rs, per_run_home.rs | 7 |
| `it/daemon_client.rs` | 1248 | control client wire rules and the mock | daemon/mod.rs, daemon/mock.rs, retry.rs | plumbing |
| `it/dependency_bans.rs` | 16 | D3 dependency ban | crate deps | none (architecture rule) |
| `it/harness_contract_golden.rs` | 975 | per-connector goldens (fixtures/harness-golden) | launch.rs, connector.rs, per_run_home.rs | 7 |
| `it/harness_contract_kit.rs` | 805 | offline conformance cases (fixtures/harness-kit) | launch.rs, commands/start | 7 |
| `it/harness_launch_plan.rs` | 782 | pure launch decisions on fixture connectors | launch.rs, model_host.rs | 7, 1 |
| `it/harness_version_probe.rs` | 195 | harness binary resolution and version line | launch.rs, harness_version.rs | 7 (version line: none) |
| `it/hermes_offline_key.rs` | 626 | no fragment of the provider key in chain or upload (P1) | commands/start, launch.rs | 4 |
| `it/login.rs` | 159 | server.json at 0600, URL rules | commands/login.rs, server.rs | 5 |
| `it/m2_capture_sender.rs` | 1991 | IPC sink order, loss, close, recovery | proxy/ingest.rs | 1, 3 |
| `it/m2_emitter.rs` | 1517 | CaptureEmitter contract | proxy/api.rs | 1, 3 |
| `it/m2_http_relay.rs` | 2249 | HTTP relay bytes and capture | proxy/http.rs, proxy/headers.rs | 1, 3, 4 |
| `it/m2_ipc_compose.rs` | 1606 | composed proxy to daemon chain | proxy + daemon ingest | 1, 3 |
| `it/m2_proxy_integration.rs` | 3025 | proxy root over real TLS | proxy/mod.rs | 1, 2, 3 |
| `it/m2_tls_connect.rs` | 1275 | CONNECT admission, SNI, CA | proxy/connect.rs, proxy/tls.rs | 1 |
| `it/m2_websocket_relay.rs` | 2074 | WebSocket relay bytes and capture | proxy/websocket.rs | 1, 3 |
| `it/main.rs` | 52 | fast-tier module list | shared | test plumbing |
| `it/model_host_from.rs` | 793 | model service from harness config; refusals | model_host.rs, commands/start | 1, 7 |
| `it/native_silence.rs` | 152 | silent native leg noted, never stops | run/watch.rs, commands/status.rs | 6, 3 |
| `it/per_run_home.rs` | 438 | per-run home made from seeds | per_run_home.rs | 7, 4 |
| `it/preflight_doctor.rs` | 478 | doctor over shipped connectors, --dev | commands/doctor.rs, preflight | 7 |
| `it/prep.rs` | 1600 | setup four steps | commands/prep.rs, prep_file.rs | 7, 5 |
| `it/process_handoff.rs` | 147 | D22 process handle handed to daemon | launch.rs | none (termination) |
| `it/restarts.rs` | 497 | a run across platform stops | commands/continue_run.rs, upload/prune.rs | 1, 2, 5 |
| `it/seal_scan.rs` | 206 | seal with busy and torn entry | commands/seal.rs | 5 |
| `it/start_launch.rs` | 1481 | argv per connector, refusals before spawn | launch.rs, commands/start | 7 |
| `it/status_views.rs` | 1869 | status and entries surfaces | commands/status.rs, commands/entries.rs, run | 6 |
| `it/upload_chunks.rs` | 1736 | chunk cutting, backoff | upload/uploader/* | 5 |
| `it/upload_open.rs` | 1548 | what start sends to and takes from the server | commands/start/server.rs, upload/mod.rs, account/credential.rs | 5 |
| `it/upload_server.rs` | 1419 | tests the test stand-in against UPLOAD-API.md | tests/common/upload_server | 5 (tests test code) |
| `it/upload_server_device.rs` | 529 | stand-in device flow | tests/common/upload_server | 5 (tests test code) |
| `it/upload_server_segments.rs` | 961 | stand-in run lifecycle | tests/common/upload_server | 5 (tests test code) |

##### tests/full/  (9093 lines)

| Test file | Lines | Tests what | Covers | Points |
|---|---:|---|---|---|
| `full/capture_crash_campaign.rs` | 1465 | daemon killed under traffic, torn tail, recovery | whole CLI + daemon | 3 |
| `full/capture_failure_campaign.rs` | 755 | quota exhausted, refused append | whole CLI + daemon | 3 |
| `full/connector_contract.rs` | 463 | one participant-shaped run per shipped connector | whole CLI | 7 |
| `full/launcher_sweep.rs` | 93 | Linux orphan sweep | subreaper.rs | none (termination) |
| `full/m2_synthetic_vertical.rs` | 881 | local production vertical, synthetic workload | whole CLI | 1, 2, 5 |
| `full/main.rs` | 26 | slow-tier module list | shared | test plumbing |
| `full/per_run_lifecycle.rs` | 366 | per-run connector start | per_run_home.rs, commands/start | 7 |
| `full/run_scoped_daemon.rs` | 897 | daemon started on demand, exits when idle | daemon/spawn.rs | plumbing |
| `full/t25_disk_full.rs` | 1173 | literal ENOSPC rows, measured not fixed | run/reserve.rs, run/pause.rs | none (owner: no disk-full handling) |
| `full/upload_e2e.rs` | 2518 | start against the local upload stand-in | upload, commands/start, commands/upload | 5 |
| `full/upload_worker.rs` | 456 | same runs against the real Worker under wrangler dev | upload | 5 |



---

## A.8 goalcup-daemon and goalcup-platform (part A2)

### How the lines were counted

- `wc -l` on every file. Cargo.toml files are left out of the code totals (daemon 88, platform 20).
- "src" means production lines in `src/`. A `#[cfg(test)] mod tests { ... }` block at the end of a
  src file is counted as **tests (inline)**, not src. The split was done by a small script that
  finds each top-level `#[cfg(test)]` followed by `mod name {` and counts to its closing `}`.
  `src/runtime/capture/tests.rs` (2,340 lines) is a test module included by `mod tests;` at
  `src/runtime/capture/mod.rs:1345-1346`, so it is counted as tests too.
- Small `#[cfg(test)]` or `feature = "test-hooks"` items inside production code (fault seams such
  as `goalcup-platform/src/tracecap.rs:805-830`, `goalcup-platform/src/sourcecap/unix.rs:655-669`,
  and 38 `test-hooks` sites across 8 daemon files) stay counted as src. So the src figures slightly
  overstate shipped code.
- Totals check: daemon `src/` 23,045 = 18,959 src + 4,086 inline tests. Platform `src/` 5,895 =
  5,039 src + 856 inline tests.

### Buckets used for "which point"

The seven points are in VISION.md and the brief. Each part gets the points it serves, and one bucket:

| Bucket | Meaning |
|---|---|
| D | Serves one or more of the seven points directly (it may still be over-built; see notes) |
| P | Plumbing that exists only because the recorder is a **separate daemon process** from the CLI (control API, IPC, registry, daemon lifecycle). It carries points 1, 2, 5 but would mostly vanish if the recorder ran inside the CLI process |
| G | Generic plumbing (errors, clocks, atomic write, config dir) |
| N | Serves none of the seven points. Often a competition or integrity rule, noted as such |

"Lines that serve no point" below means bucket N only. Bucket P is reported separately because it
is a design choice, not dead weight, but it is the largest cut candidate after N.

---

### Part 1: crates/goalcup-daemon

#### 1a. src files

| File | What it does (one line) | src | inline tests | Points | Bucket | Notes |
|---|---|---|---|---|---|---|
| `src/bin/goalcup-daemon.rs` | Foreground daemon binary: args, start, wait, stop on SIGTERM/SIGINT without sealing (`:1-30`) | 259 | 0 | 1, 5 (carrier) | P | Only needed because the recorder is its own process |
| `src/lib.rs` | Module list, `DaemonError`, clock helpers `now_unix`/`now_rfc3339` | 120 | 0 | none | G | |
| `src/config.rs` | Daemon config file: workspace parent, artifact root, control address; refuses nested roots (`:61-236`) | 236 | 0 | none (5 carrier) | P | Nesting rule is D5 "agent never sees its evidence" |
| `src/fsutil.rs` | Private atomic write (temp, fsync, rename) and temp sweep | 234 | 83 | none | G | |
| `src/control/mod.rs` | Control API state, bearer auth rule, re-exports | 417 | 0 | 1, 2, 5 (carrier) | P | |
| `src/control/file.rs` | `control.json`: where the CLI finds the daemon, its port and bearer (`:7-150`) | 152 | 0 | carrier | P | |
| `src/control/router.rs` | axum router, Host and version checks, bearer check, 1 MiB body cap (`:40-465`) | 465 | 0 | carrier | P | |
| `src/control/handlers.rs` | Handlers: health, status, list, seal, create capture, goal, launch, register process, abandon (`:22-680`) | 680 | 0 | 1, 5, 6 (carrier) | P | `register_process_blocking` is the spawn gate (`:465-628`), which serves point 1 (recording starts before the harness execs) |
| `src/ingest.rs` | Loopback TCP receiver for proxy frames: per-entry secret, framing checks, handshakes, producer close (`:145-1356`) | 1,356 | 0 | 1, 3 (carrier) | P | Exists because the proxy (in the CLI) and the daemon are two processes. Limits: 16 pending handshakes, 5 s setup (`:145-158`) |
| `src/native_capture.rs` | Reads the harness's own session files (main and subagent transcripts) as byte frames; append-only, snapshot and final-snapshot forms (`:1-110`) | 1,632 | 0 | **1, 2, 3, 7** | D | Holds the caps the owner wants raised: `max_files: 1024` default and `max_open_streams = MAX_OPEN_STREAMS/4` = 32 (`:273-283`), `MAX_NATIVE_FILES` 20,000 (`:187`), `MAX_FRESH_IDS` 128 (`:194`), 32-handle budget (`:80`) |
| `src/redact.rs` | Shared secret pattern set and whole-record redactor (also the test oracle) | 1,208 | 71 | **4** | D | Includes the `email` rule the owner said to drop (`:279-287`) and the JSON value-span logic used for `rate_limits` (`:325-345`) |
| `src/redact/stream.rs` | Streaming redaction: hand-written bounded recognizers per pattern, hold-back, audit spans (`:1-2945`) | 2,945 | 1,063 | **4** | D | Largest single src file in the two crates. Email recognizer at `:541-543`, `:1539-1700`. Over-built for point 4: a simpler line or chunk-overlap approach may do, see risk below |
| `src/registry.rs` | redb database of entries: state, run keys (ed25519 seed), process hand-off phases (`:31-1009`) | 1,009 | 0 | 5 (restart), 6 (status keys) | P | Second redb store so a damaged run does not lose the daemon's list. Needed mostly because the daemon outlives runs and restarts |
| `src/status.rs` | Builds and signs the status document with the run key (`:47-149`) | 149 | 0 | **6** | D | The signature (ed25519) is beyond point 6 |
| `src/workspace.rs` | Checks where the artifact root sits; walks and hashes the workspace at seal (64 MiB hash limit, `:24`, `:209`) | 485 | 0 | none | N | Competition need (final state of the work), not one of the seven points |
| `src/workspace_git.rs` | At seal, reads the Git commit and a digest of uncommitted changes with a locked-down `git` (`:1-30`) | 530 | 19 | none | N | Competition need (D56) |
| `src/runtime/mod.rs` | Glue: declares `Daemon` and the capture runtime | 99 | 18 | none | G | |
| `src/runtime/daemon.rs` | The `Daemon` object: start, roots, idle stop, `UNLAUNCHED_LIMIT` 15 min, 64 in-flight permits (`:29`, `:103`, `:128-640`) | 640 | 0 | carrier | P | |
| `src/runtime/termination.rs` | Runs the tree kill at finish and records what it may claim; monotone durable clock (`:1-226`) | 226 | 59 | none | N | |
| `src/runtime/capture/mod.rs` | Capture entries: create (baseline), open, reopen after restart, abandon, goal, launch, summaries (`:142-1346`) | 1,346 | 0 | **1, 2, 5** | D | Includes reopen-after-restart paths (`:795-880`) and `abandon_stale_unlaunched` |
| `src/runtime/capture/harness.rs` | Loads the connector's schema-1 `harness.toml` for a capture | 71 | 47 | **7** | D | |
| `src/runtime/capture/harness_build.rs` | Hashes the harness program and running image for `Launch` (D53), up to 512 MiB (`:28`) | 89 | 90 | none | N | Nice metadata, not a point |
| `src/runtime/capture/native_plan.rs` | Turns `[[capture.native]]` rows into native sources and roots, `since_run_start` (D57) | 311 | 0 | **2, 7** | D | |
| `src/runtime/capture/owner.rs` | Owner thread: one redactor per producer, drains sink into chain, polls native files, flush 200 ms, sync 800 ms (`:7-28`) | 388 | 0 | **1, 2, 3, 4** | D | |
| `src/runtime/capture/sink.rs` | Bounded in-memory queue: epoch admission, per-producer sequence checks, control reserve (`:7-18`, `:245-556`) | 556 | 0 | **3** | D | Enforces `MAX_LIVE_PRODUCERS` 128 (`:322`) and `MAX_PRODUCERS` 65,536 (`:183`, `:313`), the open-stream caps the owner wants raised |
| `src/runtime/capture/writer.rs` | Appends records to the hash-linked chain, writes gaps for loss | 391 | 0 | **5, 3** | D | Orphan-rule enforcement lives here too (`:227-275`) |
| `src/runtime/capture/finish.rs` | Local finish: 180 s budget, 7 stages, refusals, expired-finish latch, termination binding (`:11-763`) | 763 | 0 | 5 (partly) | D | Most lines are budget, stage tokens and latch logic. Plus 10 `test-hooks` seams (`:592-645`) |
| `src/runtime/capture/tests.rs` | Unit tests for sink, owner, writer, finish, orphan rule | 0 | 2,340 | 1, 3, 5 | D | Whole file is tests |
| `src/terminate/mod.rs` | Kills the harness process tree at seal: walk, SIGTERM, grace, SIGKILL, sweep, anchors (D22, D30) | 848 | 0 | none | N | |
| `src/terminate/authority.rs` | Owned-set ordering (deepest first), identity and argv guards before signalling | 622 | 107 | none | N | |
| `src/terminate/darwin.rs` | macOS audit-token signalling for a group-less single process | 422 | 189 | none | N | |
| `src/terminate/os.rs` | `ProcessOs` seam over `/bin/ps` and libc so tests can simulate a process table | 310 | 0 | none | N | |

Daemon src total: **18,959 src, 4,086 inline tests.**

#### 1b. tests/ files (all test lines)

| File | What it tests | Lines | Points | Bucket |
|---|---|---|---|---|
| `tests/common/mod.rs` | Declares shared support | 16 | none | G |
| `tests/common/capture_lifecycle_support.rs` | Real daemon, v5 control calls, chain readers | 579 | 1, 5 | D |
| `tests/common/support/mod.rs` | Registry and config fixtures | 142 | carrier | P |
| `tests/common/support/control.rs` | Hand-written control API client | 499 | carrier | P |
| `tests/common/support/daemon.rs` | Daemon config and fake harness handle | 56 | carrier | P |
| `tests/common/support/redaction.rs` | Streaming-redaction driver, seeded generator, oracle check | 321 | 4 | D |
| `tests/common/support/status.rs` | One status input and entry | 64 | 6 | D |
| `tests/common/support/table_lock.rs` | Serialises process-table tests | 90 | none | N |
| `tests/common/support/tree.rs` | Builds process trees for kill tests | 282 | none | N |
| `tests/fixtures/wrapper-*.sh` (12 files, shell, not .rs) | Shell process shapes for kill tests | 213 | none | N |
| `tests/dir_modes.rs` | Unix modes of the two configured roots, umask | 338 | carrier | P |
| `tests/health_status.rs` | Signed status document | 151 | 6 | D |
| `tests/it/main.rs` | Fast test binary (note duplicated `#[cfg]` at `:12-13`) | 33 | none | G |
| `tests/it/binary_surface.rs` | `--version` before anything opens | 32 | carrier | P |
| `tests/it/capture_harness_load.rs` | Connector loading and digest refusals at create | 449 | 7 | D |
| `tests/it/capture_ingest.rs` | Ingest receiver over real loopback TCP (49 tests) | 3,095 | 1, 3 | P |
| `tests/it/capture_lifecycle.rs` | Create, goal, launch, status through the real daemon (73 tests) | 3,529 | 1, 5, 6 | D |
| `tests/it/capture_native.rs` | Native file capture: scope, symlinks, generations, limits (45 tests) | 2,664 | 2 | D |
| `tests/it/capture_redaction.rs` | Streaming equals whole-record redaction at every chunking (43 tests) | 4,745 | 4 | D |
| `tests/it/capture_redaction_extra.rs` | Connector `extra_patterns` on the streaming redactor | 1,113 | 4 | D |
| `tests/it/capture_redaction_run_secrets.rs` | Run proxy credential removed in every encoded form (D60) | 379 | 4 | D |
| `tests/it/config_load.rs` | Config defaults and refusals | 160 | carrier | P |
| `tests/it/control_api.rs` | Routes, bearer rule, `control.json`, handlers (31 tests) | 1,735 | carrier | P |
| `tests/it/harness_contract_golden.rs` | Per-connector daemon goldens (macOS only) | 463 | 7 | D |
| `tests/it/redaction.rs` | Pattern set, connector patterns, audit (53 tests) | 2,629 | 4 | D |
| `tests/it/registry_roundtrip.rs` | Registry survives restart | 652 | carrier | P |
| `tests/it/start_race.rs` | Two daemons racing for the registry lock | 108 | carrier | P |
| `tests/it/table_lock_group.rs` | Lock refuses outside nextest serial group | 15 | none | N |
| `tests/it/terminate_sim.rs` | Tree kill against a simulated process table (34 tests) | 2,714 | none | N |
| `tests/it/workspace_layout.rs` | Artifact root placement and seal walk | 643 | none | N |
| `tests/full/main.rs` | Slow test binary | 12 | none | G |
| `tests/full/capture_finish_budget.rs` | The 180 s finish budget and stage refusals (18 tests) | 1,267 | 5 (partly) | D |
| `tests/full/process_kill.rs` | Tree kill against real `/bin/sh` trees (28 tests) | 1,285 | none | N |

Daemon `tests/` total: **30,473 lines** (30,260 in `.rs` files plus the 213 shell fixture lines; checked with wc -l). With inline tests: **34,559 test lines** for 18,959 src lines (1.8 test lines per src line).

#### 1c. Daemon totals by bucket

| Bucket | src | inline tests | tests/ | All tests |
|---|---|---|---|---|
| D (serves a point) | 9,760 | 3,521 | 18,353 | 21,874 |
| P (separate-process plumbing) | 5,214 | 0 | 6,817 | 6,817 |
| G (generic plumbing) | 453 | 101 | 61 | 162 |
| N (no point) | **3,532** | 464 | 5,242 | **5,706** |
| **Total** | **18,959** | **4,086** | **30,473** | **34,559** |

Bucket N in the daemon, by theme:

| Theme | src | tests |
|---|---|---|
| Process-tree termination (`terminate/*`, `runtime/termination.rs`) | 2,428 | 355 inline + 2,714 sim + 1,285 kill + 282 tree + 213 fixtures + 90 lock + 15 group = 4,954 |
| Seal-time workspace walk and Git state (`workspace.rs`, `workspace_git.rs`) | 1,015 | 19 + 643 = 662 |
| Harness build digest (`harness_build.rs`) | 89 | 90 |

---

### Part 2: crates/goalcup-platform

Who uses it (grep of `goalcup_platform::<module>` outside the crate): `awake` and `terminal` by the
CLI; `machine` and `disk` by the CLI; `sourcecap` and `handle_budget` by `goalcup-daemon/src/native_capture.rs`;
`tracecap` by the daemon, the protocol crate, the verifier and the CLI upload prune;
`process_image` by `goalcup-daemon/src/runtime/capture/harness_build.rs`; `clone` by the CLI and
`goalcup-daemon/src/workspace.rs`; `errno` only inside the crate.

#### 2a. src files

| File | What it does (one line) | src | inline tests | Points | Bucket | Notes |
|---|---|---|---|---|---|---|
| `src/lib.rs` | Module list, Unix-only compile error, config-dir resolver (`:1-82`) | 82 | 152 | none | G | |
| `src/awake.rs` | Holds a sleep lock (`caffeinate` / `systemd-inhibit`) while a run is live | 269 | 0 | 1 (indirect) | D | A week-long run on a laptop would sleep otherwise. Owner call |
| `src/clone.rs` | Rule: is a folder the root of its own Git clone (D4, D56) | 68 | 96 | none | N | Competition rule for the project folder |
| `src/disk.rs` `free_bytes` (`:1-57`) | Free space on a volume, shown in status vitals | 57 | 0 | 6 | D | |
| `src/disk.rs` `ReserveFile` (`:58-239`) | Pre-allocated reserve file held against a full disk (used by `goalcup-cli/src/run/reserve.rs`) | 182 | 0 | none | N | Owner: "Disk full: no special handling" |
| `src/errno.rs` | Thread `errno` for the `readdir` end idiom | 42 | 0 | none | G | |
| `src/handle_budget.rs` | 32-handle limit for native capture (`MAX_GIT_LIVE_HANDLES_V1`, `:13`) | 79 | 0 | 3 | D | A cap of our own. Guards the macOS 256 fd default, but at 32 it is far below it |
| `src/machine.rs` WSL and filesystem checks (`:89-414`) | Detect WSL2, Windows-host and unsupported filesystems; refuse unsafe folders | 326 | part of 98 | 7 | D | |
| `src/machine.rs` URL opener (`:467-540`) | Open `https` URLs in a browser for login | 74 | part of 98 | 5 | D | Login is needed to upload |
| `src/machine.rs` process usage (`:677-798`) | CPU and memory of the harness for vitals | 122 | part of 98 | 6 | D | Weak link to point 6 |
| `src/machine.rs` facts, account home, AC power, container, sysctl (`:1-88`, `:415-466`, `:541-676`, `:799-810`) | Machine facts uploaded with the run, `~/.ssh` home, AC power warning, container marker | 288 | part of 98 | none | N | |
| `src/process_image.rs` | Path of a running process's executable (Linux `/proc/<pid>/exe`, macOS `proc_pidpath`) | 40 | 0 | none | N | Only used by `harness_build.rs` |
| `src/sourcecap.rs` | Handle-rooted, no-follow reads of session files: identity, names, refusals | 616 | 0 | 1, 2 | D | Safe reading is needed; the depth of no-follow, mount and identity checks is integrity hardening beyond the points |
| `src/sourcecap/unix.rs` | Unix `openat`/`fstatat`/`readdir` implementation | 669 | 0 | 1, 2 | D | |
| `src/terminal.rs` | Shares the terminal with a chat-mode harness: signal shield, foreground group, echo off (D57) | 292 | 190 | 7 | D | Needed for interactive TUIs |
| `src/tracecap.rs` | Handle-rooted writes of the chain part files, fixed documents, artifact root | 843 | 177 | 5 | D | Same hardening note as sourcecap |
| `src/tracecap/unix.rs` | Unix implementation | 990 | 143 | 5 | D | |

Platform src total: **5,039 src, 856 inline tests.**

#### 2b. tests/ files

| File | What it tests | Lines | Points | Bucket |
|---|---|---|---|---|
| `tests/it/main.rs` | Test binary | 17 | none | G |
| `tests/it/artifact_root_unix.rs` | Artifact root and trace dir refuse path swaps and symlinks | 224 | 5 | D |
| `tests/it/awake.rs` | Sleep lock with a fake `systemd-inhibit` | 256 | 1 (indirect) | D |
| `tests/it/disk.rs` | Reserve file hold, shrink, refuse | 137 | none | N |
| `tests/it/machine.rs` | WSL and filesystem parsing (about 250 lines, point 7); facts, AC power, container (about 126 lines, none) | 376 | 7 / none | D / N |
| `tests/it/unix_snapshot_primitives.rs` | Bounded child listing and source identity | 88 | 2 | D |
| `tests/it/unix_sourcecap.rs` | No-follow source reads | 261 | 2 | D |
| `tests/it/unix_tracecap.rs` | Trace dir writes | 325 | 5 | D |

Platform `tests/` total: **1,684 lines.** With inline: **2,540 test lines** for 5,039 src lines.

#### 2c. Platform totals by bucket

| Bucket | src | inline tests | tests/ | All tests |
|---|---|---|---|---|
| D | 4,337 | 608 | 1,404 | 2,012 |
| G | 124 | 152 | 17 | 169 |
| N | **578** | 96 | 263 | **359** |
| **Total** | **5,039** | **856** | **1,684** | **2,540** |

---

### Part 3: Grand totals

| Folder | src | tests (inline + tests/) | src with no point (N) | tests with no point (N) | src separate-process plumbing (P) | tests P |
|---|---|---|---|---|---|---|
| goalcup-daemon | 18,959 | 34,559 | 3,532 | 5,706 | 5,214 | 6,817 |
| goalcup-platform | 5,039 | 2,540 | 578 | 359 | 0 | 0 |
| **Both** | **23,998** | **37,099** | **4,110** | **6,065** | **5,214** | **6,817** |

- **Lines that serve no point: 4,110 src and 6,065 test lines, 10,175 in all** (17 percent of src,
  16 percent of tests).
- If the recorder ran inside the CLI process, bucket P (5,214 src, 6,817 tests) would mostly go too.
  That is an estimate, not a measured cut: some of it (the spawn gate, a small run file) would be
  rewritten in a smaller form.

---

### Part 4: Observations for the verdict (with risks)

1. **Termination is the biggest no-point block** (2,428 src, about 4,950 test lines). It proves
   what a kill "may claim" with `/bin/ps` guards, Darwin audit tokens and a simulated process
   table. None of the seven points needs that proof. A plain `killpg(SIGTERM)`, grace,
   `killpg(SIGKILL)` would end the run.
   *Risk:* agents that start background processes outside the group (setsid, double fork) may keep
   running after seal and make model calls nobody records. That is a run-integrity concern, not a
   recording one. Also check how `goalcup seal` mid-run behaves after the cut.
2. **The daemon is a separate process.** That choice costs the control API (1,714 src), ingest IPC
   (1,356), registry (1,009), daemon lifecycle (640), config (236) and the binary (259), plus 6,817
   test lines. The proxy already runs in the CLI (`docs/ARCHITECTURE.md` section 3).
   *Risk:* the separation is a stated rule (D3, `goalcup-cli/tests/it/dependency_bans.rs`); it
   lets a run survive a CLI crash and lets `goalcup status` and `goalcup seal` talk to a live run
   from another terminal. Merging means redesigning those commands and rewriting large tests in the
   week before 6 November. This is a high-effort change; flag it as a "change" only if the owner
   accepts the risk.
3. **Caps the owner wants raised or removed live here.** The full list, the citations and the fd risk are in
   section A.4.
4. **Redaction is 4,153 src lines** (`redact.rs` 1,208 + `stream.rs` 2,945) and 9,187 test lines in
   `tests/` plus 1,134 inline. The email rule to drop is at `redact.rs:279-287` and
   `stream.rs:541-543` with its recognizer near `stream.rs:1539-1700`.
   *Risk:* the streaming engine guarantees a secret split across two frames is still caught.
   A simpler design must keep that, or secrets leak (point 4). Cutting tests here is riskier than
   anywhere else.
5. **Seal-time workspace state** (`workspace.rs`, `workspace_git.rs`, 1,015 src) serves the
   competition (what the agent built), not any of the seven points. Keep only if judging needs it.
   *Risk:* without it the server has no final state of the work; the participant's Git push may be
   enough, unverified.
6. **Finish budget** (`finish.rs`, 763 src; `tests/full/capture_finish_budget.rs`, 1,267) has 7
   stages, a 180 s budget and an expired-finish latch. Point 5 needs only "drain, write the last
   records, seal". *Risk:* without a deadline, a stuck drain could hang `goalcup` at the end; a
   simple timeout keeps point 3.
7. **Disk reserve file** (`disk.rs:58-239`, 182 src, 137 tests, plus `goalcup-cli/src/run/reserve.rs`
   outside this scope) contradicts the owner decision "Disk full: no special handling".
   *Risk:* low; a full disk then shows up as a write error and a gap.
8. **Machine facts, AC power, container marker, harness build digest, process image** (about 417
   src) are metadata no point needs. *Risk:* the server or a report may read these fields; check
   the Worker and verifier before cutting (not checked here).
9. **Stale doc:** `docs/ARCHITECTURE.md:339` cites `goalcup-daemon/tests/it/capture_deferred_scope.rs`,
   which does not exist.
10. **Platform coverage gap in tests:** `tests/it/harness_contract_golden.rs` is macOS only, and
    `capture_finish_budget`, `capture_lifecycle`, `capture_harness_load` run only on macOS and Linux
    (`tests/it/main.rs:12-17`, `tests/full/main.rs:9`). Not a cut, but tests that do not run on the
    Vercel Sandbox OS prove nothing there (which OS that sandbox uses is unverified here).


---

## A.9 goalcup-protocol and goalcup-verifier (part A3)

### How the lines were counted

- `wc -l` on every file. "src" means the lines in `src/` that are not inside a file's trailing
  `#[cfg(test)] mod ...` block. "inline tests" means the lines of that block (they live in `src/`
  but are test code). "tests" means `tests/**/*.rs`. "fixtures" means non-Rust files under `tests/`.
- A few items are compiled only under `test` or the `test-hooks` feature but sit in the middle of a
  file (118 lines in `export.rs`, 20 in `pins.rs`, a few in `lineage.rs` and `wire.rs`). They are
  counted as src below and called out where it matters.

### Totals per folder

| Folder | Files | Lines (wc -l) | Kind |
|---|---|---|---|
| goalcup-protocol/src (non-test) | 28 .rs | 11,013 | src |
| goalcup-protocol/src inline test modules | 13 modules | 1,370 | tests (54 `#[test]`) |
| goalcup-protocol/tests/*.rs | 19 .rs | 13,471 | tests (315 `#[test]`) |
| goalcup-protocol/tests fixtures | 23 files | 720 | test data |
| goalcup-protocol data (`providers.toml`, `pins/connector-dirs.txt`, `Cargo.toml`) | 3 | 25 + 7 + 41 | build data |
| goalcup-verifier/src (non-test) | 20 .rs | 12,153 | src |
| goalcup-verifier/src inline test modules | 8 modules | 1,335 | tests (28 `#[test]`) |
| goalcup-verifier/tests/*.rs | 20 .rs | 20,724 | tests (350 `#[test]`) |
| goalcup-verifier/tests fixtures | 124 files | 2,182 (incl. 466 lines of Python in two `derive.py`) | test data |
| goalcup-verifier `Cargo.toml` | 1 | 47 | build data |
| **Both crates, src** | | **23,166** | |
| **Both crates, all test code (inline + tests/)** | | **36,900** (+2,902 fixture lines) | |

Test code is 1.6 times the src.

### Who uses these crates (evidence)

- The daemon never links the verifier; the CLI does (`docs/ARCHITECTURE.md:46-51`).
- The CLI uses the verifier in one shipped place: `goalcup doctor`'s `decoder_check`
  (`crates/goalcup-cli/src/commands/doctor.rs:246-316`). It builds `DecoderRegistry::builtin()` and
  only prints a Warn when a declared protocol hint has no decoder. One more use is a unit test in
  `crates/goalcup-cli/src/model_host.rs:1224-1260`. The CLI full tests use `scan_capture_trace`
  (`crates/goalcup-cli/tests/full/upload_e2e.rs:7`, `upload_worker.rs:11`).
- The server (Cloudflare Worker, TypeScript) links neither crate. Its "verify job" that would run
  `goalcup-verify` over stored chunks is not built (`docs/SERVER-DESIGN.md:159`,
  `docs/OPERATING.md:310`, `crates/goalcup-verifier/src/lib.rs:1-2`). `docs/CHEAT-CHECKLIST.md:27-28`
  says the job re-runs `goalcup-verify` every hour; that is not true today.
- The Worker stores and signs `connector_dir_sha256` (`server/worker/src/runs.ts:183`,
  `server/worker/src/sign.ts:26,61`). A grep found no allowlist comparison in `server/worker/src`.
- The organiser uses `goalcup-verify --capture-integrity` after `scripts/fetch-run.sh`
  (`scripts/fetch-run.sh:21,349`).
- The test kit uses `--capture-integrity` and `--research-json` (`scripts/matrix.sh:2057-2064`,
  `2247-2269`; `scripts/linux-harness-smoke.sh:231`; `scripts/research-replay.py:106`).
  No script calls `--stats` (grep over `scripts/`); only docs mention it.
- The participant is told what was recorded by the CLI from daemon status and proxy counters
  (`control.rs` `StatusPayload`, `ProxyCounters`, `capture_status.rs`), not by the decoders.

### goalcup-protocol: every file

Points: 1 calls, 2 subagents, 3 never block / gaps, 4 secrets, 5 upload and rebuild by ID and time,
6 tell the participant, 7 real harnesses. "Rules" means it enforces a Goal Cup rule (start window,
168 hours, fresh repo) that is not one of the seven points.

| File | What it does | src | inline tests | Main test files (lines) | Used by (src) | Points |
|---|---|---|---|---|---|---|
| lib.rs | Module list and `ProtocolError` | 68 | 0 | none | all | infra |
| capture.rs | v5 capture record types (`Open`, `Segment`, `End`, `Gap`, `Connection`, `Recovery`, `Launch`, `Assistance`, `LocalFinish`), gap reasons, size and count bounds (`MAX_OPEN_STREAMS` 128, `MAX_OPEN_CONNECTIONS` 128, `MAX_LIVE_PRODUCERS` 128, `MAX_PRODUCERS` 65,536, `MAX_CAPTURE_QUEUE_BYTES` 8 MiB, `CHAIN_QUOTA_BYTES` 1 TiB) (`capture.rs:25-72,183-202`) | 744 | 103 | it/m2_contract.rs (1,715) | cli, daemon, verifier | 1, 2, 3, 5 |
| capture_status.rs | Status projection: run, capture, delivery, analysis, anchor states (`capture_status.rs:15-87`) | 87 | 0 | it/control_shapes.rs, it/m2_contract.rs | cli, verifier | 6 |
| chain.rs | `link_hash`, `GENESIS`, the per-line hash chain (`chain.rs:21-45`) | 45 | 0 | it/chain_golden.rs (102) | cli, daemon, verifier | 5 |
| clock.rs | Sleep-counting monotonic clock for recorded times (`clock.rs:1-60`) | 60 | 63 | none | cli, daemon | 5 |
| envelope.rs | The one chained record shape; `EventBody` = Baseline, Seal, Capture; keeps a pre-v5 redaction field (`envelope.rs:24,95-129`) | 129 | 0 | it/roundtrip.rs | daemon, verifier | 5 |
| events.rs | `HealthState` (online, sealed, sealing_recovery_needed) | 26 | 0 | it/roundtrip.rs, it/control_shapes.rs | cli, daemon | 6 |
| export.rs | The one trace-directory writer: chain parts (256 MiB / 30 min rotation), acks, baseline, local finish, resume; also a legacy-seal terminal kind and test-only writers `write_receipt`, `write_seal_for_protocol`, resume seams (118 lines under `cfg(any(test, feature="test-hooks"))`, `export.rs:38,1164-1293,1424,1496-1555,1684-1724`) | 1,872 | 145 | export_roundtrip.rs (1,477), it/capture_storage.rs (829), it/capture_terminals.rs (884), it/roundtrip.rs (1,143, shared) | cli, daemon, verifier | 5 (and 3 via parts) |
| control.rs | Local CLI to daemon HTTP API: routes, `StatusPayload`, signed status (`SignedStatus`, `STATUS_DOMAIN`), `ProxyCounters`, finish budgets, errors, `DaemonClient` trait; `CONTROL_API_VERSION` 12 with a long version history (`control.rs:27-66`) | 1,010 | 30 | it/control_shapes.rs (689), it/control_consumer.rs (121) | cli, daemon | 3, 5, 6 |
| ack.rs | Server-signed acknowledgement of a chain prefix (`ack.rs:1-95`) | 95 | 0 | it/roundtrip.rs, it/signing_vectors.rs (54) | cli uploader, verifier reader | 5 (upload progress); tamper evidence |
| receipt.rs | Server-signed start receipt and its digest (`receipt.rs:15-186`) | 186 | 0 | it/roundtrip.rs, it/signing_vectors.rs | cli upload, verifier reader | Rules (start window anchor) |
| seal.rs | Workspace manifest (files, git state, walk cap, exclusions) for baseline and finish (`seal.rs:6-140`) | 140 | 0 | it/roundtrip.rs | cli, daemon, verifier | Rules (fresh product) |
| provenance.rs | Digest and UTC timestamp newtypes, run key spelling, termination outcome (`provenance.rs:13-155`) | 275 | 0 | it/provenance_wire.rs (46) | cli, daemon, verifier | 5 (partly rules) |
| process.rs | `ps`-based process discovery, PID reuse guard, `ProcessHandle`, liveness (`process.rs:27-494`) | 497 | 251 | it/ps_env.rs (24) | cli, daemon | Rules (168 h stop); not a point |
| run_secret.rs | The run's own secret values (proxy credential, GitHub token, max 2) and every encoded form the daemon must mask (`run_secret.rs:1-376`) | 376 | 154 | none in tests/ | cli, daemon | 4 |
| redaction.rs | `RedactionAudit` span shape carried beside redacted bytes | 23 | 0 | it/roundtrip.rs | daemon, verifier | 4 |
| hints.rs | Grammar of protocol hints (`valid_hint`) | 27 | 21 | none | daemon (native_capture) | 1 (decoder selection) |
| manifest.rs | `KillStrategy`, `LaunchMode`, `Kill` vocabulary | 49 | 0 | it/control_shapes.rs | cli, daemon | 7 |
| template.rs | `{name}` substitution that fails on a leftover placeholder; launch argv and env | 114 | 123 | it/connector_template.rs (indirect) | cli | 7 |
| constant_time.rs | `constant_time_eq` for control bearer, ingest token, proxy credential | 27 | 14 | none | cli, daemon | none (local hygiene) |
| connector_dir.rs | SHA-256 digest of a whole connector folder, refuses symlinks and devices (`connector_dir.rs:61`) | 215 | 188 | it/connector_pins.rs | via harness loader and pins only | none (tamper check); digest is recorded in `Launch` |
| pins.rs | Build-time `const fn` parser of `pins/connector-dirs.txt`, pin lookups, run-time twin parser (`pins.rs:1-520`) | 520 | 230 | it/connector_pins.rs (303), it/pins_build_fails.rs (86) | cli preflight and doctor, verifier report | none (tamper check of connectors) |
| capture_policy.rs | Trust tables a connector is checked against: CA env names, injection env, provider prefixes, interpreter list, startup files, seed content spelling check, host and path rules (`capture_policy.rs:45-1071`) | 1,077 | 29 | it/harness_schema.rs, it/harness_conformance.rs | cli (launch, model_host, tls) | partly 1 (no route around the proxy), partly 4 (no key in a seed) |
| harness.rs | Harness module root, schema version, `MAX_SEED_BYTES` 64 KiB | 72 | 0 | see below | cli, daemon | 7 |
| harness/check.rs | `HarnessManifest::validate`, every refusal rule in fixed order; `MAX_NATIVE_ROWS` 8 (`check.rs:26`) | 1,580 | 0 | it/harness_schema.rs (4,451) | via harness | 7 |
| harness/load.rs | One loader: read, hash, parse one connector dir; `Dev` origin guard | 325 | 19 | it/harness_loader.rs (650) | via harness | 7 |
| harness/parse.rs | Schema dispatch, retired-key and headless refusals, strict deserialization | 228 | 0 | it/harness_schema.rs | via harness | 7 |
| harness/schema.rs | Typed `harness.toml` structs and limits (`schema.rs:74-900`), `provider_hosts` from `providers.toml` | 1,146 | 0 | it/harness_schema.rs, it/harness_conformance.rs (379), it/connector_template.rs (120) | via harness | 7 |

Tests that check no single module: `it/no_family_literals.rs` (378, a repo lint: no harness name in
core code, scans protocol, daemon, verifier, CLI), `it/main.rs` (20).

#### Protocol src by point group

| Group | Files | src lines |
|---|---|---|
| Recording core (1, 2, 3, 5, 6) | capture, capture_status, chain, clock, envelope, events, export, control, ack, provenance, hints, lib | 4,438 |
| Secrets (4) | run_secret, redaction | 399 |
| Harness contract (7) | harness.rs + check, load, parse, schema; template; manifest; capture_policy | 4,591 |
| No point (rules or tamper checks only) | pins 520, process 497, connector_dir 215, receipt 186, seal 140, constant_time 27 | 1,585 |
| **Total** | | **11,013** |

Inside the "point" groups there is more that serves no point: the 118 test-only lines in
`export.rs`, the legacy-seal terminal kind (`export.rs:311-315,787,1157,1520,1550`), the pre-v5
descriptor sweep (`export.rs:58`), the pre-v5 envelope redaction field (`envelope.rs:24`), the
"older traces stay valid" launch fields (`capture.rs:311-327`), the `#[doc(hidden)]` re-export shim
(`capture.rs:18`, `capture_status.rs:7-8`), the status signature (`control.rs:27-31,353-360`), and
the control API version history comment (`control.rs:33-66`). There are 29 legacy or compatibility
markers in `goalcup-protocol/src` (grep for legacy, older trace, valid forever, pre-v5, written
before, shim, doc(hidden)). With no backward compatibility before Edition 01, all of these can go.

#### Protocol tests by subject

| Subject | Files | Lines |
|---|---|---|
| Harness schema and loader | it/harness_schema.rs 4,451; it/harness_loader.rs 650; it/harness_conformance.rs 379; it/connector_template.rs 120 | 5,600 |
| Trace writer | export_roundtrip.rs 1,477; it/capture_storage.rs 829; it/capture_terminals.rs 884; it/roundtrip.rs 1,143 | 4,333 |
| Capture wire goldens | it/m2_contract.rs | 1,715 |
| Control API | it/control_shapes.rs 689; it/control_consumer.rs 121 | 810 |
| Connector pins | it/connector_pins.rs 303; it/pins_build_fails.rs 86 | 389 |
| Lint | it/no_family_literals.rs | 378 |
| Small | chain_golden 102; signing_vectors 54; provenance_wire 46; ps_env 24; main 20 | 246 |
| **Total** | | **13,471** |

Fixtures (720 lines): frozen-connectors 237, harness fixtures 339, m2-ingest 124, m2-capture 11,
m2-status 9. The harness and m2 fixtures are also read by CLI and daemon tests
(`goalcup-cli/tests/it/harness_contract_kit.rs`, `goalcup-daemon/tests/it/capture_ingest.rs`).

### goalcup-verifier: every file

#### Integrity path (needed by the organiser now, and by the server verify job if it is built)

| File | What it does | src | inline tests | Test files (lines) | Points |
|---|---|---|---|---|---|
| lib.rs | Re-exports, pinned server key `SERVER_PUBLIC_KEYS` (one key), `VerifyOptions` (`lib.rs:1-53`). `allow_descriptor_key` "has no effect" (`lib.rs:40-43`) | 53 | 0 | | 5 |
| capture_analysis/mod.rs | Module list | 39 | 0 | | infra |
| capture_analysis/reader.rs | Bounded streaming integrity reader: chain hashes, v5 only, baseline, receipt, acks, seal, terminal, gaps, open-stream and producer bounds; exits 0/1/2/3 (`reader.rs:1-75,1384-1403,1560-1567,1690-1695`) | 1,889 | 181 | it/m2_capture_integrity.rs (3,737), it/v5_corpus.rs (718), common/capture_records.rs (523), common/records.rs (31), common/mod.rs (81), it/main.rs (27); fixtures v5-corpus (912 lines, 98 files) | 5, and 3 (reports gaps) |
| capture_analysis/lineage.rs | Producer identity and recovery supersession for reader and session | 101 | 41 | (inline) | 5 |
| bin/goalcup-verify.rs | CLI with three modes `--capture-integrity`, `--research-json`, `--stats`, plus `--dev-key` | 326 | 0 | it/m2_capture_integrity.rs, it/v5_corpus.rs run the binary | 5 |
| **Integrity subtotal** | | **2,408** | **222** | **5,117** (+912 fixtures) | |

The reader also imports `wire::SegmentView` (`reader.rs:36`), a small piece of `wire.rs`.

#### Research decoders and reports (needed by neither server nor participant today)

| File | What it does | src | inline tests | Test files (lines) | Points |
|---|---|---|---|---|---|
| capture_analysis/api.rs | `DecoderV1` trait, `StreamOpen`, observation types | 313 | 0 | it/m2_decoder_v1.rs (1,126) | none (research) |
| capture_analysis/registry.rs | `DecoderRegistry::builtin()`, six decoders | 213 | 0 | it/m2_decoder_v1.rs, it/harness_contract_golden.rs (487) | none; doctor Warn only |
| capture_analysis/session.rs | Runs decoders over records, isolation, panic hook, `ObservationCounts` incl. `wire_model_requests` (`session.rs:180-212`); also refuses past `MAX_OPEN_STREAMS` (`session.rs:644,761`) | 878 | 62 | it/m2_decoder_v1.rs, m2_decoder_v1_quiet_panic.rs (127), common/decode.rs (58) | none (research measure of 1, 2) |
| capture_analysis/operations.rs | Bounded record counts, first `Launch`, references, goal excerpt | 265 | 83 | it/m2_research_report.rs | none |
| capture_analysis/wire.rs | Segment assembler, SSE framer, sink counters, field helpers | 991 | 348 | it/m2_openai_decoder.rs etc., common/wire.rs (153) | none (one type used by reader) |
| capture_analysis/tracker.rs | Stream tracker shared by the two OpenAI wire decoders | 600 | 0 | it/m2_openai_decoder.rs, it/m2_chat_decoder.rs | none |
| capture_analysis/anthropic.rs | Anthropic Messages decoder | 1,585 | 53 | it/m2_anthropic_decoder.rs (2,658); fixtures m2-anthropic (331) | none (measures 1) |
| capture_analysis/openai.rs | OpenAI Responses decoder (HTTP and WebSocket) | 664 | 0 | it/m2_openai_decoder.rs (2,149); fixtures m2-openai (741) | none (measures 1) |
| capture_analysis/chat.rs | OpenAI Chat Completions decoder (Hermes path) | 1,080 | 0 | it/m2_chat_decoder.rs (2,524), it/m2_hermes_capture_only.rs (395) | none (measures 1) |
| capture_analysis/thread.rs | Amp thread WebSocket decoder, links subagent snapshots by `parentToolUseId` | 383 | 0 | it/m2_thread_decoder.rs (625) | none (measures 2) |
| capture_analysis/native/mod.rs | Generic native JSONL decoder | 617 | 0 | it/m2_native_observations.rs (1,737); fixtures m2-native (31) | none |
| capture_analysis/native/claude_code.rs | Claude Code project JSONL format, authorship markers | 219 | 0 | same | none (measures 2, rules: keep going) |
| capture_analysis/native/codex.rs | Codex rollout JSONL, `spawn_agent` child attribution | 443 | 0 | same | none (measures 2) |
| capture_analysis/report.rs | Private NDJSON research report (`--research-json`) | 463 | 483 | it/m2_research_report.rs (2,456); fixtures m2-report-bytes (53) | none (research; used by test kit) |
| capture_analysis/stats.rs | Numbers-only run summary (`--stats`), caps `MAX_TRACES` 128, `MAX_IDS` 1,000,000 (`stats.rs:39-55`) | 1,031 | 84 | it/m2_run_stats.rs (873) | none (no script calls it) |
| **Research subtotal** | | **9,745** | **1,113** | **15,607** (+1,270 fixtures) | |

Cross-cutting research tests: it/connector_samples.rs (239, fixtures/connectors 114 lines) and
it/harness_contract_golden.rs (487) pin decoder selection.

#### Verifier src by use

| Use | src | inline tests | tests/ | fixtures |
|---|---|---|---|---|
| Integrity (organiser, future server job) | 2,408 | 222 | 5,117 | 912 |
| Research decoders and reports | 9,745 | 1,113 | 15,607 | 1,270 |
| **Total** | **12,153** | **1,335** | **20,724** | **2,182** |

80 percent of the verifier src (9,745 of 12,153) and 75 percent of its tests serve no point
directly. They are the research instrument and the test kit's way of counting calls and
subagents. The only shipped participant use is a doctor Warn (`doctor.rs:246-316`).

### Lines that serve no point (both crates, src only)

| Part | src | Why no point |
|---|---|---|
| Verifier research decoders, session, report, stats | 9,745 | Research only; no participant or server use |
| protocol pins.rs + connector_dir.rs | 735 | Tamper check of 4 reviewed, code-owned connector folders |
| protocol process.rs | 497 | 168-hour stop and PID reuse guard: a rule, not a point |
| protocol receipt.rs + seal.rs | 326 | Start-window and fresh-repo evidence: rules |
| protocol constant_time.rs | 27 | Local auth hygiene |
| protocol legacy and test-only code inside point modules | about 250 (118 test-only in export.rs, plus legacy seal, pre-v5 fields, shims, status signing) | Backward compatibility or tests |
| **Total** | **about 11,580 of 23,166** | |

Rules code (process, receipt, seal, about 1,000 lines) is not a point but Goal Cup still needs it.
The honest "pure waste for Friday" figure is the research decoders plus pins plus legacy code:
about 10,700 src lines.

### Verdicts (brief format)

| Part (path, src, test lines) | What it does | Points | Verdict | Reason | Lines saved (src / tests) | Risk | Tests that go |
|---|---|---|---|---|---|---|---|
| verifier research: `capture_analysis/{api,registry,session,operations,wire,tracker,anthropic,openai,chat,thread,native/*,report,stats}.rs`, 9,745 src, 1,113 inline + 15,607 tests | Decode recorded bodies into model calls, tool calls, usage, subagent links; research report; stats | none (measures 1, 2) | Cut from the Friday build; move to a research branch or crate outside the workspace gate | Nothing a participant or the server needs. The doctor check is a Warn. `--stats` has no caller | 9,745 / 16,720 (+1,270 fixture lines) | The test kit loses its "agent worked" and call-count checks (`scripts/matrix.sh:2057-2064`); the kit's subagent count already reads 0 (progress doc line 60). Replace with a 100 to 200 line count of `Open` records by path from the reader, or a Python script over chain JSON. `doctor.rs:246-316` and the `model_host.rs` test must go too. Judges lose ready-made decoded reports | m2_anthropic_decoder, m2_chat_decoder, m2_openai_decoder, m2_native_observations, m2_thread_decoder, m2_decoder_v1, m2_decoder_v1_quiet_panic, m2_research_report, m2_run_stats, m2_hermes_capture_only, connector_samples, harness_contract_golden, common/decode.rs, common/wire.rs |
| verifier integrity: `reader.rs`, `lineage.rs`, `lib.rs`, `mod.rs`, bin, 2,408 src, 5,339 tests | Checks chain, acks, receipt, baseline, terminal; reports gaps | 5, 3 | Keep, change | The organiser needs it after `fetch-run.sh`. Must change together with the daemon: `open_stream_limit_exceeded` and `connection_limit_exceeded` make a trace invalid, exit 2 (`reader.rs:1567,1695`; test `m2_capture_integrity.rs:1973-1984`). Drop legacy-seal and pre-v5 branches and `allow_descriptor_key`. Bin keeps one mode | about 300 to 500 / about 1,000 | If the daemon caps are lifted and the reader is not, every run with more than 128 open streams or connections reads as invalid evidence. The v5 corpus (912 fixture lines) must be regenerated | parts of v5_corpus.rs and m2_capture_integrity.rs for legacy and limit cases |
| `capture.rs`, 744 src, 103 + 1,715 tests | Record types and bounds | 1, 2, 3, 5 | Keep, change | Remove or raise `MAX_OPEN_STREAMS`, `MAX_OPEN_CONNECTIONS`, `MAX_LIVE_PRODUCERS` per owner decision (no caps of our own); keep `MAX_CAPTURE_QUEUE_BYTES` (memory, a real cost) and say so. Drop legacy launch fields and the `Assistance` tool-allowance pair no daemon writes (`capture.rs:311-316`) | about 50 / some of m2_contract.rs | Memory per open stream in daemon and reader grows with no bound; macOS 256 open files per process (owner note) | m2_contract.rs goldens rewritten |
| `export.rs`, 1,872 src, 145 + 3,190 tests | Writes the trace | 5 | Keep, change | Drop legacy seal kind, pre-v5 descriptor sweep, test-only receipt and seal writers (118 lines), resume seams if not needed | about 300 / about 800 | Resume after crash is the subtle part; cutting seams loses crash-point tests | capture_terminals.rs seal cases, parts of export_roundtrip.rs |
| `control.rs`, 1,010 src, 30 + 810 tests | CLI to daemon API | 3, 5, 6 | Keep, change | Drop status signing (`SignedStatus`, `STATUS_DOMAIN`), the version history comment, legacy-payload notes | about 150 / about 200 | The status route is unauthenticated loopback; without a signature a local process could fake status. Low value since the agent can already reach the daemon (CHEAT-CHECKLIST D52) | parts of control_shapes.rs |
| `chain.rs`, `envelope.rs`, `clock.rs`, `events.rs`, `capture_status.rs`, `hints.rs`, 374 src | Chain, record, time, health, status | 5, 6, 1 | Keep | Small and central. Drop the pre-v5 envelope field | about 10 / 0 | none known | none |
| `ack.rs` 95, `provenance.rs` 275 | Server ack, digest and time types | 5 | Keep | Ack drives upload progress | 0 | none | none |
| `receipt.rs` 186, `seal.rs` 140 | Start receipt, workspace manifest | Rules | Keep (owner call) | Start window and fresh-repo rules need evidence | 0 | Cutting makes start time and planted files unprovable | roundtrip.rs receipt cases |
| `process.rs` 497 src, 251 + 24 tests | Process discovery, PID guard | Rules | Keep | The 168-hour stop needs it | 0 | none | none |
| `run_secret.rs` 376, `redaction.rs` 23 | Secret forms and audit shape | 4 | Keep | Point 4 | 0 | none | none |
| `pins.rs` 520 src, 230 + 389 tests; `connector_dir.rs` 215 src, 188 tests | Build-time connector pins and folder digest | none | Cut pins.rs; keep the digest function only if `Launch` still records it | Four in-repo connectors are reviewed code; the Worker stores the digest but does not compare it | 520 / 619 (keep connector_dir) | A locally edited connector is no longer refused at preflight; the recorded digest still shows it. `goalcup doctor` pin check and CLI preflight gate (`preflight/gates.rs:114-117`) must change | connector_pins.rs, pins_build_fails.rs, pins inline tests |
| `harness/*` 3,351 src + `capture_policy.rs` 1,077 + `template.rs` 114 + `manifest.rs` 49, tests 5,600 + 48 inline | Connector schema, refusal tables, trust tables | 7 (partly 1, 4) | Change: shrink | 114 schema tests and a 1,077-line trust table guard four reviewed files. Keep the rules that stop traffic bypassing the proxy (1) and keys in seeds (4); drop limits on display names, configure args, no-prompt values and similar | about 2,000 / about 3,500 (estimate, unverified until done) | Phase 2 notes want schema changes (multi-provider hosts, `schema.rs:783-900`); shrinking at the same time adds churn. A weaker check lets a connector edit route around the proxy | most of harness_schema.rs |
| `constant_time.rs` 27 | Secret compare | none | Keep | 27 lines; removing it is a security bug | 0 | none | none |
| `it/no_family_literals.rs` 378 | Lint: no harness name in core | none | Cut | A style rule, not a point | 0 / 378 | Harness names may creep into core | itself |

### Blind spots

- The reader and session enforce the same 128 caps as the daemon (section A.4). Raising them in one place only
  turns long runs with many subagents into "invalid evidence" (exit 2). This is the main coupling.
- The decoders are the only code that turns a trace into "N model calls, M subagents". Cutting them
  without a small replacement leaves no way to show points 1 and 2 held in a test run. The phase 2
  notes also lean on `wire_model_requests` (`session.rs:180-212`) as the "decoded model calls" number.
- `docs/CHEAT-CHECKLIST.md:27-28` claims a server verify job that does not exist. Any trust argument
  that leans on it is unverified.
- Inline `#[cfg(test)]` detection used the first top-level `#[cfg(test)] mod`. Small test-only items
  in the middle of files (118 lines in export.rs, 20 in pins.rs, a few in lineage.rs and wire.rs) are
  counted as src.


---

## A.10 Everything outside crates/ and docs/ (part A4)

"src" means shipped or run code; "tests" means test code; "data" means config, fixtures or docs.
Points are the seven points of VISION.md (1 calls, 2 subagents, 3 never block, 4 no secrets,
5 upload everything, 6 tell the participant, 7 real harnesses as people run them).

### 0. Totals

| Area | src | tests | data / other |
|---|---|---|---|
| server/worker/src (17 files) | 4,001 | | |
| server/worker/test (19 files incl. helpers, vectors, tsconfig) | | 5,749 (.ts only) | 51 (vectors json + tsconfig) |
| server/worker/scripts (2 sh) | 394 | | |
| server/worker/sql (mirror of website migrations) | | | 4,468 |
| server/worker other (README 471, wrangler.toml 164, package.json 21, package-lock 1,636, tsconfig 24, .dev.vars.example 5, .gitignore 3) | | | 2,324 |
| connectors/ (4 harness.toml) | | | 556 |
| packaging/ (install.sh 394, README 101) | 394 | | 101 |
| harnesses/install.sh | 62 | | |
| skills/ (3 skills, 6 files) | | | 739 |
| spikes/ (9 folders + redact.py; 183 files, 75 of them `.redactions.jsonl`) | 349 (redact.py) + prototype code (see 6) | | about 23,364 |
| scripts/ non-test (26 files incl. README, example json, sql) | 11,176 | | 309 (README 79, campaign.example.json 64, research-staging.sql 166) |
| scripts/test_*.py (20 files) | | 10,393 | |
| .github/ (3 workflows 276, CODEOWNERS 57, templates 183, dependabot 17) | | | 533 |
| Root files (README 318, TODO 350, VISION 44, AGENTS 110, CLAUDE 16, SECURITY 127, CONTRIBUTING 126, CODE_OF_CONDUCT 132, LICENSE 202, NOTICE 15, Cargo.toml 128, rust-toolchain 7, .gitignore 38, .gitattributes) | | | about 1,650 |
| .config/nextest.toml 61, .claude/agents/campaign-cell.md 31 | | | 92 |

The brief's "about 17k lines" for the Worker matches: 4,001 src + 5,749 tests + 394 scripts + 4,468 sql
+ 2,324 other is about 16,900.

---

### 1. server/worker (the upload Worker)

Cloudflare Worker, TypeScript, no framework. It holds the participant's run credential check, streams
zstd chunks into R2, records progress in Supabase through PostgREST RPC, signs receipts and acks, and
runs a 5-minute cron. Header: server/worker/src/index.ts:1-2.

#### 1.1 Every route (server/worker/src/index.ts:24-60, 64)

| # | Method | Path | Handler | Auth | Points |
|---|---|---|---|---|---|
| 0 | GET | /v1/health | inline json ok (index.ts:64) | none | none (liveness) |
| 1 | POST | /v1/device | createDevice (device.ts:48) | none, rate limited per address | 5 (ties upload to a member) |
| 2 | POST | /v1/device/token | pollDevice (device.ts:98) | none, rate limited per code and address | 5 |
| 3 | POST | /v1/device/approve | approveDevice (device.ts:120) | only when TEST_APPROVAL_KEY is set (staging and local) | none (test-only) |
| 4 | POST | /v1/runs | openRun (runs.ts:211) | credential | 5 |
| 5 | GET | /v1/runs/{run_id} | getPosition (runs.ts:418) | credential bound to run | 5 (resume upload) |
| 6 | POST | /v1/runs/{run_id}/receipt | issueReceipt (runs.ts:352) | bound | 5 (signed start) |
| 7 | POST | /v1/runs/{run_id}/abandon | abandonRun (runs.ts:393) | bound | none (event lifecycle) |
| 8 | POST | /v1/runs/{run_id}/seal | sealRun (runs.ts:540) | bound | 5 |
| 9 | POST | /v1/runs/{run_id}/heartbeat | heartbeat (runs.ts:444) | bound | 5, 6 (vitals feed the emails) |
| 10 | POST | /v1/runs/{run_id}/continue | continueRun (runs.ts:274) | bound | 5, 7 (keep going after a stop) |
| 11 | PUT | /v1/runs/{run_id}/chunks/{offset} | putChunk (chunks.ts:34) | bound | 5 |
| cron | scheduled `*/5 * * * *` | runTick (notices.ts:192) | | | 6 (emails), none (judged commit) |

A wrong method on a known path is 405 (index.ts:66-68). A maintenance switch answers 503 to every
route but health before any database call (index.ts:71-78). R2 key per chunk:
`traces/{event}/{run}/{entry}/{offset16}-{sha256}.zst` (chunks.ts:157). This matches the owner's
"the file path is the ID; the server orders by ID and time".

#### 1.2 Every src file

| File | Lines (src) | What it does | Points | Verdict | Reason | Risk |
|---|---|---|---|---|---|---|
| src/index.ts | 108 | Router, maintenance switch, auth gate, error mapping, cron export | 5 | keep | Small and central. | none |
| src/auth.ts | 29 | Bearer `gcr_` credential check, sha256 only stored (auth.ts:1-3) | 4, 5 | keep | | none |
| src/chunks.ts | 228 | PUT chunk: idempotent retry, chain continuity, run size limit, R2 create-only put with sha256 check, signed ack (chunks.ts:1-15) | 5 | keep | The core of point 5. 32 MiB per chunk cap (chunks.ts:22) guards Worker memory and request size, a real cost. | none |
| src/context.ts | 171 | Parses config vars, picks the db backend, builds the signer; default run limit 64 GiB (context.ts:12) | 5 | keep, trim | MIN_COMPETITION_VERSION parsing and maintenance config are not one of the seven points. | Low |
| src/db.ts | 485 | The Db interface and shared rules: run status, credential lifetimes, notice kinds, vitals types (db.ts:17-65, 234-252, 397) | 5 | keep, change | Drop `disk_full` notice kind and DISK_NOTICE_GAP_SECONDS (db.ts:252, 397) per "Disk full: no special handling". | Changing a notice kind needs a matching website migration (SQL is the authority). |
| src/db-supabase.ts | 433 | Production Db: one PostgREST RPC per method, 10 s timeout (db-supabase.ts:1-12, 136) | 5 | keep | | none |
| src/db-local.ts | 701 | Test and `wrangler dev` Db, state as JSON in R2 with etag CAS (db-local.ts:1-13) | none directly (tests) | keep for Friday, cut later | 701 lines plus 590 test lines exist only to test without Supabase. Every Worker integration test and the Rust `upload_worker` gate tests run against it. | Cutting it now removes the only local end-to-end test of upload. |
| src/device.ts | 146 | Device flow: start, poll, test approve (device.ts:1-8) | 5 | keep | Needed to tie a run to a member. The approve route is test-only. | none |
| src/env.ts | 96 | Bindings, vars and secrets type | 5 | keep | | none |
| src/errors.ts | 51 | Error codes and JSON helpers | 5 | keep | | none |
| src/github.ts | 223 | Cron reads GitHub to find the judged commit at each deadline; also headers for the public-repo check (github.ts:1-3, 173) | none | change (low priority) | Event judging, not recording. Could become a one-off organizer script after the deadline. | Unverified whether GitHub's activity data is still readable days later; doing it in the cron removes that doubt. Keep for Friday if unsure. |
| src/mail.ts | 70 | Sends queued run emails through Resend, idempotent (mail.ts:1-12) | 6 | keep | Emails are the only server-side way to tell a participant something. | none |
| src/notices.ts | 220 | Cron: close over runs, record judged commit, queue emails (started, silence, idle_3h, idle_6h, recording, ended, disk_full) | 6, none | keep, change | Cut the `disk_full` notice (owner decision). idle_3h and idle_6h are not one of the seven points; optional cut. | Notice text lives in the website SQL; a cut needs both repos. |
| src/ratelimit.ts | 139 | Durable Object counters on the device flow (ratelimit.ts:1-15) | none | keep | Guards a real cost and abuse on the only unauthenticated calls, and says so. Fits "a limit must guard a real cost". | none |
| src/runs.ts | 637 | Open, continue, receipt, abandon, position, heartbeat (with vitals), seal (with tunnels list) | 5, 6 | keep, change | Drop the disk vitals (`disk_free_bytes`, `disk_short`, `agent_paused: "disk"`, runs.ts:500-530) with the disk-full cut. Review caps: JSON 64 KiB, each seal document 16 MiB, tunnels list 8 MiB (runs.ts:25-29), machine facts 4 KiB (runs.ts:174). | A week-long run with thousands of subagents may grow seal.json or the tunnels list past 16 MiB / 8 MiB (unverified; client uses the same 16 MiB, crates/goalcup-cli/src/upload/uploader/finish.rs:254). If it can, raise both sides together. |
| src/sign.ts | 116 | Ed25519 receipts and acks, byte layout fixed by the Rust verifier (sign.ts:1-13) | 5 | keep | The acks prove what the server holds. | none |
| src/util.ts | 148 | Hex, time, ids, field checks, REPO_URL (util.ts:1-13) | 5 | keep | | none |
| **Total src** | **4,001** | | | | Realistic cut for Friday: about 60 to 120 lines (disk vitals, disk notice). | |

#### 1.3 Tests (server/worker/test), 5,749 lines

| File | Lines | What it tests | Kind |
|---|---|---|---|
| config.test.ts | 298 | Config guards, SupabaseDb request shape (stub fetch) | unit |
| device-unit.test.ts | 87 | Codes, constant-time compare, credential lifetimes | unit |
| device.test.ts | 306 | Device flow end to end over `wrangler dev` | e2e |
| github.test.ts | 184 | Public-repo check, stub fetch | unit |
| integration.test.ts | 814 | Whole contract over HTTP against `wrangler dev` (46 `test(` calls) | e2e |
| lifecycle.test.ts | 645 | Replace, practice, start window, continue, close at +24 h, on a moved clock | in-process |
| local-db.test.ts | 590 | LocalDb rules on a fake bucket | unit, goes if db-local.ts goes |
| mail.test.ts | 108 | Emails against fake db and stub Resend | unit |
| notices.test.ts | 1,064 | The cron: close, judged commit, email queue | in-process |
| quota.test.ts | 114 | RUN_MAX_COMPRESSED_BYTES 413 | in-process |
| ratelimit.test.ts | 184 | Durable Object limits | unit |
| segments.test.ts | 150 | Lifecycle over real HTTP with chunk bodies | e2e |
| sign.test.ts | 68 | Signatures match Rust vectors (test/vectors/signing.json, 26 lines) | unit |
| supabase.test.ts | 567 | Every RPC name and argument | unit |
| test-mode.test.ts | 76 | Organizer test runs | unit |
| helpers/client.ts 158, fake-r2.ts 57, inproc.ts 80, rig.ts 86, worker.ts 113 | 494 | Test harness | helpers |

Who runs them: `npm --prefix server/worker test` (package.json:11-12) runs only in `gate.sh full`
(scripts/gate.sh:107-108). CI does not run them (.github/workflows/ci.yml:73-92 has no npm step).
The Rust `upload_worker` tests also start the real Worker in `gate.sh full` (scripts/gate.sh:89-91).

Tests that go with the suggested Worker cuts: parts of notices.test.ts (disk_full cases) and the
vitals cases in integration.test.ts and lifecycle.test.ts (exact count unverified, they were not
run). If github.ts moves out: github.test.ts (184) and the judged-commit cases in notices.test.ts.

#### 1.4 SQL: server/worker/sql/goal-cup-tracer-upload.sql (4,468 lines, data)

A read-only COPY of every `*goal_cup*` migration in the website repo, in name order
(sql/goal-cup-tracer-upload.sql:1-24). It is not applied from here. It creates schema
`goal_cup_tracer` with tables events, consents, device_requests, runs, run_credentials, entries,
heartbeats, notices, chunks (lines 52-245), later test_members, test_operators, signups, teammates
(1100, 1917, 3094, 3119), and about 30 `public.goal_cup_upload_*` RPC functions that the Worker calls,
many redefined several times by later migrations (for example open_run at 474, 1185, 1671, 1944,
2177, 2568, 3889). It also holds the website's sign-up, teammate, terms and erase functions
(3214-3808, 1854, 4093), which the Worker never calls.

| Verdict | Reason | Risk |
|---|---|---|
| keep, change | It is a reference mirror, not code. Only used by `scripts/check-sql.sh` and as documentation. For a fresh build one could keep only the final body of each function the Worker calls. | The authority is the website repo. Editing here does nothing; any server change (for example the disk_full notice) must land there first. |

#### 1.5 Worker scripts (394 lines)

| File | Lines | What it does | Run by | Points | Verdict |
|---|---|---|---|---|---|
| scripts/check-sql.sh | 299 | Starts a throwaway Postgres, stubs Supabase, applies the SQL mirror and any `sql/pending/` file, smoke-calls functions (check-sql.sh:1-18) | by hand | none | keep only while website migrations change; it guards the shared production database. |
| scripts/delete-test-runs.sh | 95 | Dry-run-by-default delete of organizer test runs in the database and R2 (delete-test-runs.sh:1-13) | by hand | none (ops) | keep; small and needed to clean test data before Edition 01. |

#### 1.6 wrangler.toml (164 lines) and config

Production Worker `goalcup-ingest` on `ingest.agentcommunity.org` (wrangler.toml:6-16), R2 bucket
`goalcup-traces` with a 180-day lock on the cup prefix (32-35), Durable Object `DeviceLimiter`
(21-28), cron every 5 minutes (100-101), observability with invocation logs off (109-113), and a
full `[env.staging]` copy (121-164). Vars of note:

| Var | Value | Comment |
|---|---|---|
| EVENT_ID | `goalcup-dryrun-2026` | Must switch to `goalcup-cup-2026` between 6 Nov 00:00 and 7 Nov 00:00 UTC (wrangler.toml:39-44). A Friday deploy item. |
| RUN_HOURS | 168 | Matches the event. |
| RUN_MAX_COMPRESSED_BYTES | 64 GiB | Owner to decide (brief). Over it a chunk is 413 and the client keeps data and retries (wrangler.toml:56-60). |
| MIN_COMPETITION_VERSION | empty | Not one of the seven points. |
| MAINTENANCE | off | Ops switch. |
| DEVICE_*_PER_MINUTE | 120 / 30 / 1200 | Abuse guards. |

Verdict: keep. The owner deploys from a local device (brief), which this file supports (no
account_id, wrangler.toml:5). Risk: the staging block repeats every var by hand
(wrangler.toml:115-120); a var added only at the top is missing on staging.

package.json (21) pins wrangler 4.149.0 and TypeScript; Node >= 22.18. package-lock.json 1,636 lines.
README.md (471) is the operator runbook (deploy, maintenance, rollback, EVENT_ID switch, cron).

---

### 2. connectors/ (556 lines of data)

Each connector is one `harness.toml`. There are no seed files in connectors/: no `files/` folders
exist (find shows only the four harness.toml files). Codex and Hermes say their home is "never
seeded" (codex-cli/harness.toml:69, hermes-cli/harness.toml:80). The directory pins live in
crates/goalcup-protocol/pins/ (out of scope here) and are rewritten by scripts/pin-manifests.sh.

| Connector | Lines | Wire capture | Native capture (session files) | Subagents | Redaction rows | Points |
|---|---|---|---|---|---|---|
| claude-code-cli | 159 | Provider table `anthropic`, or the gateway in `settings.json` `env.ANTHROPIC_BASE_URL` (lines 37-43) | `projects/*/*.jsonl` append-only (99-105) | Four extra rows: `subagents/agent-*.jsonl`, their meta, `subagents/workflows/*/agent-*.jsonl`, their meta; plus `tool-results/*` (115-152). Observed in one real run (comment lines 107-114). | `anthropic_billing_header` (157-159) | 1, 2, 4, 7 |
| codex-cli | 130 | `chatgpt.com/backend-api/codex/responses`, `api.openai.com/v1/responses`, or the configured provider base_url | `sessions/*/*/*/rollout-*.jsonl` | Covered by the same rollout pattern: a subagent writes its own rollout file (spike fixture spikes/codex-cli/fixtures/rollout-subagent-01a06615.jsonl). | 7 rows: rate_limits, plan_type, credits_remaining, credits, installation_id, safety_identifier, turn_state | 1, 2, 4, 7 |
| hermes-cli | 152 | Provider from `config.yaml` `model.base_url`; launch_prep points provider `recorder` at the proxy | `sessions/session_*.json` snapshot, `state.db*` final_snapshot | Delegations live in `state.db` (spike fixture spikes/hermes/fixtures/db-async_delegations.jsonl); `state.db` is captured only as a final snapshot, so a week-long run sees it once, at the end. | none | 1, 2 (partial), 7 |
| amp-cli | 115 | `ampcode.com` thread JSON-RPC paths | `.cache/amp/logs/threads/T-*.log`, metadata only, no transcript | Not stated in the file (unverified) | `amp_ws_token` (secret) | 1, 7 |

Verdicts:
- All four: keep. They are data, not code, and they are what point 7 needs.
- codex-cli `[redaction] extra_patterns`: change per owner masking decision b ("drop the Codex
  rate-limit rules"). The likely rows are `codex_rate_limits`, `codex_plan_type`,
  `codex_credits_remaining`, `codex_credits`. Whether `installation_id`, `safety_identifier` and
  `turn_state` count as "rate-limit rules" is unverified; they are identifiers, not secrets. Risk:
  any connector edit changes its pin, so pin-manifests.sh must run in the same commit, and goldens
  may change.
- hermes-cli: risk for point 2. Subagent records in `state.db` reach the trace only at the end
  (`final_snapshot`). If the run dies hard, they may be missing. The wire leg still records every
  subagent model call, which is what point 1 needs.
- Each connector sets `warn_missing = false` on the subagent rows, so a run with no subagents does not
  report a gap (claude-code-cli/harness.toml:121-152). Good for point 3.

### 3. OpenCode and OpenClaw facts sheets (branch origin/opencode-openclaw only)

Read with `git show origin/opencode-openclaw:<path>`. The branch adds 5 files, 696 lines
(docs/harnesses/README.md +2, openclaw-cli.md 121, opencode-cli.md 126, two results files 145 and 302).
Not on main. Not edited.

| Sheet | Lines | Version | Status | Key facts for the seven points |
|---|---|---|---|---|
| docs/harnesses/opencode-cli.md | 126 | 1.19.4, measured 2026-10-28 on Linux x86_64 | Facts only. Sessions only in SQLite `opencode.db`, which schema 1 cannot capture (section 4). | Point 1: wire works through `HTTPS_PROXY`, Gateway `POST /v3/ai/language-model` (no decoder exists) and DeepSeek `/chat/completions`; "Measured in a run: 27 calls on one connection". Point 2: subagents unmeasured (section 7). Point 7: chat mode `opencode` TUI records in a real goalcup run (section 1). Hazard: an unknown model id silently falls back to a paid Gateway model (section 6). |
| docs/harnesses/openclaw-cli.md | 121 | 2026.10.8, measured 2026-10-28 | Facts only. Transcripts only in SQLite; executable is a Node script so a pinned connector is refused (row 10). | Point 1: its own `proxy.proxyUrl` or `OPENCLAW_PROXY_URL` overrides `HTTPS_PROXY` and sends model traffic past the recorder (section 6, 8). Goal Cup reads neither today. Point 2: subagents unmeasured (section 7). Point 3: Ctrl-C refused during retries; `goalcup seal --yes` stopped it with termination `Unresolved` (section 7). Rules question open: is `openclaw chat` (`tui --local`) an accepted chat mode (section 9). |

Verdict for Friday: neither harness can ship as a connector without a SQLite native form or a
wire-only mode. A wire-only connector (no native rows) would satisfy point 1 for both, since the wire
records every model call; point 2 is then covered only through the wire. OpenClaw needs a guard
against its own proxy setting, or its traffic is silently unrecorded (a point 1 failure without a
gap). Risk: both sheets say subagents are unmeasured.

### 4. packaging/ and harnesses/install.sh

| File | Lines | What it does | Run by | Points | Verdict | Reason / risk |
|---|---|---|---|---|---|---|
| packaging/install.sh | 394 (src) | The participant's one-command installer: picks target, checks SHA256SUMS, test-runs new binaries before replacing, installs into ~/.local, refuses to replace goalcup while a run records (install.sh:1-36; functions at 41-243) | participants; release CI (.github/workflows/release.yml:80-89); tested by scripts/test_install.py (614 lines) in gate full | 7, 3 (never replaces a recording goalcup) | keep, trim | The recording-run guard (`guard_recording_run`, install.sh:110-198, about 90 lines) protects point 3. Could shrink but is cheap insurance. |
| packaging/README.md | 101 (data) | Release layout and publishing steps | readers | 7 | keep | |
| harnesses/install.sh | 62 (src) | Research script: installs Codex, OpenCode, a DeepSeek harness, OpenClaw from npm and Hermes from uv into an isolated folder and writes env.sh (harnesses/install.sh:1-62) | by hand only; mentioned in docs/harnesses/codex-cli.md:107 and hermes-cli.md:36 | none | cut | Not used by any test, gate or CI. Installs harnesses unpinned. Its env.sh sets `XDG_DATA_HOME` and `OPENCLAW_STATE_DIR` that a connector may not set. Risk: two facts sheets point to it; fix those two lines. |

### 5. skills/ (739 lines of data)

| File | Lines | What it does | Used by | Points | Verdict |
|---|---|---|---|---|---|
| skills/goal-cup/SKILL.md | 34 | Text put into the harness home before each launch: what is recorded, only "keep going", the deadline and `time_left`, GOALCUP.md, "Subagents of this harness are fine" | Embedded in the binary with `include_str!` (crates/goalcup-cli/src/skill.rs:18) | 6, 2 | keep. It tells the agent plainly what is recorded and that subagents are fine, which fits "ask for subagents in the prompt, never force them". |
| skills/goal-cup-setup/SKILL.md | 106 | A skill a participant hands to their own agent to walk `goalcup setup` | linked from README.md:19 | 7, 4 (never handle tokens) | keep. |
| skills/adding-a-capture-harness/SKILL.md | 39 | Points to docs/HARNESS-ONBOARDING.md | contributors; CODEOWNERS | 7 | keep. |
| .../templates/connector-harness.toml | 209 | Connector template | contributors | 7 | keep, check it matches the four live connectors (not checked). |
| .../templates/harness-facts.md | 74 | Facts sheet template | contributors | 7 | keep. |
| .../writing-a-decoder.md | 277 | How to write a verifier decoder | contributors | 1 | keep, or move under docs/. After Friday. |

### 6. spikes/ (about 23,700 lines, 183 files, about 8.5 MB)

Raw-but-redacted artifacts from the September spikes (2026-09-03 runs), plus two scripts.
Nothing here ships. But several crate tests read these files at test time, so a cut is not free.

| Folder / file | Files | Lines | kB | What it holds | Read by tests? | Points | Verdict |
|---|---|---|---|---|---|---|---|
| redact.py | 1 | 349 (src) | | Conservative secret redaction for fixtures (redact.py:1-30) | Yes: daemon redaction tests run it and compare with the daemon redactor (crates/goalcup-daemon/tests/it/redaction.rs:1307, 1431, 1944; capture_redaction.rs:3710) | 4 | keep. It is the second, independent implementation the secret rules are checked against, and the PR template requires it (.github/PULL_REQUEST_TEMPLATE.md:12). Risk if cut: the cross-check of the masking rules goes. |
| claude-code-cli/ | 23 | 2,959 | 492 | Session, subagent, hooks, raw request bodies, Claude Code 2.1.259 | Yes: chain golden (crates/goalcup-protocol/tests/it/chain_golden.rs:62-76) | 2, 4 | keep the 3 files the golden reads; the rest only feeds the corpus scan below. |
| claude-desktop/ | 17 | 4,391 | 516 | Claude Desktop attach mode | Only the corpus scan (redaction.rs:615-650 reads every `spikes/*/fixtures/*.jsonl`, asserting more than 50 files) | none | cut candidate, but see risk. |
| codex-app/ | 16 | 532 | 244 | ChatGPT app attach mode | Yes: capture_redaction.rs:3432 reads rollout-main-01a06657.jsonl | 4 | keep that one file. |
| codex-cli/ | 23 | 904 | 540 | Codex 0.153.0 rollouts incl. subagent rollout | corpus scan; m2_native_observations.rs names them in comments only | 2 | keep as evidence for subagent rollouts. |
| cursor-cli/ | 22 | 1,681 | 996 | Cursor agent, plus a prototype hook script goalcup-hook.py (198 lines) | corpus scan | none | cut candidate (no connector). |
| hermes/ | 27 | 1,508 | 2,400 | Hermes 0.19.0, db dumps incl. delegations, a prototype plugin goalcup-tracker/__init__.py (374 lines) | corpus scan | 2 | keep the delegation dump as evidence; prototype plugin is dead code. |
| m2-first-release/ | 7 | 1,564 | 128 | e02 early-load.py (955) and early-crosscheck.py (517), plus excerpts | Yes: scripts/test_early_crosscheck.py (538) and the CLI's m2 controlled workload fixture imports early-load.py (crates/goalcup-cli/tests/fixtures/m2-controlled-workload.py:61, tests/common/m2_vertical_support/mod.rs:546) | none directly | keep while those CLI tests exist; goes if the M2 vertical tests go. |
| openclaw/ | 31 | 7,887 | 1,036 | OpenClaw 2026.8.2 Gateway-plugin era, incl. hook-llm-input-full.json (6,921 lines) and a prototype capture plugin (203 lines) | corpus scan | none | cut candidate; outdated against the 2026.10.8 facts sheet (SQLite era). |
| opencode/ | 17 | 1,938 | 2,180 | OpenCode 1.18.27 era, prototype plugin goalcup-tracker.ts (269 lines) | Yes: redaction.rs:624-625 expects a hit in opencode/fixtures/hooks.jsonl | 4 | keep hooks.jsonl; the rest is outdated against 1.19.4 (SQLite only). |

Risk for any spikes cut: the corpus test fails if fewer than 50 `.jsonl` fixtures remain
(redaction.rs:641-645), and the opencode hit is required (redaction.rs:624-625). A cut needs that test
changed in the same commit. The prototype hook and plugin code (about 1,044 lines: cursor 198,
hermes 374, openclaw 203, opencode 269) is dead: the product moved to the proxy and native files.
Lines saved if the attach-mode and outdated spikes go (claude-desktop, cursor-cli, openclaw, most of
opencode and hermes): about 16,000 lines of data and about 4 MB, no src lines.

---

### 7. scripts/ (11,176 src lines plus 230 data lines, 10,393 test lines)

"Gate" is `sh scripts/gate.sh fast|full`; "CI" is .github/workflows/ci.yml; see scripts/README.md:1-79.
Gate full runs every `test_*.py` (scripts/gate.sh:109). CI runs four of them (ci.yml:73-77).

#### 7.1 Checks and the gate

| Script | Lines (src) | Test file (lines) | What it does | Run by | Points | Verdict | Reason / risk |
|---|---|---|---|---|---|---|---|
| gate.sh | 113 | test_gate.py (226) | The gate: version check, build, fmt, clippy, nextest; full adds ignored e2e, upload_worker, Worker npm test, script tests, link check (gate.sh:1-19) | by hand | none (quality) | keep | It is the "done" bar in AGENTS.md. |
| check-version.py | 206 | test_check_version.py (128) | Fails if the version is written outside Cargo.toml | gate fast and full (gate.sh:62) | none | change: shrink | 334 lines to guard one string. A 10-line grep would do. Risk: a stale version in a doc. |
| check-links.py | 1,349 | test_check_links.py (1,672) | Checks every relative Markdown link and anchor | CI (ci.yml:78); gate full | none | change: shrink or cut | 3,021 lines for doc links. Not one of the seven points. Risk: broken links in participant docs (point 6), which is a real but small risk. |
| ci-scope.py | 90 | test_ci_scope.py (134) | Decides whether CI runs the Rust steps (docs-only PRs skip) | CI (ci.yml:58-70) | none | keep or cut | Saves CI minutes; small. |
| pin-manifests.sh | 274 | (tested by crates connector_pins) | Rewrites connector directory pins | by hand; check-connector.sh | 7 | keep | Pins are how a connector is trusted. |
| check-connector.sh | 314 | test_check_connector.py (411) | Every local check for one connector (files, pins, schema, goldens, kit) | by hand; tests in CI | 7 | keep | Contributor path for new harnesses. |
| linux-check.sh | 106 | test_linux_check.py (107) | Linux half of the gate in Docker with ext4 and XFS | by hand; Linux CI (linux-x86_64.yml:44) | 7 | keep | Proves the Linux build people run. |
| linux-harness-smoke.sh | 242 | none | No-login smoke of the measured Linux Codex and Claude Code builds | Linux CI (linux-x86_64.yml:63) | 7 | keep | Cheap proof a real harness launches. |

#### 7.2 Real runs and measurements

| Script | Lines (src) | Test file (lines) | What it does | Run by | Points | Verdict | Reason / risk |
|---|---|---|---|---|---|---|---|
| matrix.sh | 2,522 | test_matrix.py (1,517) | One real practice or test run of one connector against one server, checked end to end, one PASS/FAIL row (matrix.sh:1-40) | by hand | 7, 5 | keep, shrink after the event | It is how runs are tested the way people run them. scripts/README.md:79 already plans a split. |
| linux-matrix.sh | 876 | test_linux_matrix.py (1,219) | matrix.sh for a real harness in Docker against staging | by hand | 7 | keep | Used for the Linux and Vercel Sandbox style tests. |
| campaign.py | 1,888 | test_campaign.py (1,335) | Runs many long test cells in parallel with drills, writes reports | by hand | 7, 2 | keep for the 2-hour and long runs; shrink later | No subagent forcing found in campaign.py or campaign.example.json (grep), which fits "never force them by config". |
| campaign.example.json | 64 (data) | | Example campaign config | campaign.py | 7 | keep | |
| kit_settle.py | 302 | none | Sets the cheapest-model and no-prompt settings in a test cell's harness home, then checks the run used that model (kit_settle.py:1-30) | linux-matrix.sh, campaign.py | none (test cost) | keep | It guards real test cost. It writes harness settings only in the test home, never the participant's. |
| t25-disk-full.sh | 93 | none | Measures a run on a literally full disk (needs sudo and a loop image) | by hand | none | cut | Owner decision: "Disk full: no special handling". |
| research-measure.py | 541 | test_research_measure.py (365) | Sizes, gaps and compression of a trace's chain; aggregates | by hand | none | cut (or park) | Research, not recording. |
| research-sample.py | 424 | test_research_sample.py (247) | One CSV row of soak measurements per interval | by hand; campaign.py | none | keep if campaign.py needs it, else cut | campaign.py calls it (scripts/README.md:31). |
| research-staging.py | 604 | test_research_staging.py (426) | Read-only aggregates of one event's uploads | by hand | none (ops) | park | Useful to watch the cup's uploads, but not needed to record. |
| research-staging.sql | 166 (data) | | The query behind research-staging.py | research-staging.py | none | goes with it | |
| research-watch.py | 95 | test_research_watch.py (104) | Seals one chosen entry after a time budget | by hand | none | cut | Research only. |
| research-replay.py | 342 | test_research_replay.py (325) | Compares two traces with one verifier | by hand | none | cut | Research only. |
| fetch-run.sh | 349 | test_fetch_run.py (695) | Rebuilds a run's trace from the server's R2 copy and database index so goalcup-verify can judge it (fetch-run.sh:1-40) | by hand (organizer) | 5 | keep | This is the "server can rebuild the full story" half of point 5. Read only, never prints credentials. |

#### 7.3 Releases

| Script | Lines (src) | Test file (lines) | What it does | Run by | Points | Verdict |
|---|---|---|---|---|---|---|
| release.sh | 115 | test_release.py (180) | `prepare` opens the version PR, `tag` tags the merge | by hand | 7 | keep |
| package-release.sh | 45 | none | Packs one tarball for one target | release CI (release.yml:73) | 7 | keep |
| third-party-licenses.py | 231 | test_third_party_licenses.py (94) | Writes the NOTICE of third-party licences in a tarball | package-release.sh | none (legal) | keep |
| smoke-release.sh | 55 | none | Smoke-tests one tarball | release CI (release.yml:76) | 7 | keep |

#### 7.4 Script tests not tied to a script above

| Test | Lines | Tests | Run by | Verdict |
|---|---|---|---|---|
| test_codeowners.py | 56 | Every CODEOWNERS path exists | CI; gate full | keep (small) |
| test_early_crosscheck.py | 538 | spikes/m2-first-release/e02 cross-check | gate full | cut with the M2 spike if those tests go |
| test_install.py | 614 | packaging/install.sh | gate full | keep |

#### 7.5 Script totals if the suggested cuts land

| Cut | src | tests |
|---|---|---|
| t25-disk-full.sh | 93 | 0 |
| research-measure, -watch, -replay | 978 | 794 |
| research-staging .py + .sql (park, optional) | 770 | 426 |
| check-version.py shrunk to a grep (about 10 lines) | about 196 | 128 |
| check-links.py (optional) | 1,349 | 1,672 |
| **Firm cuts only (first three rows)** | **1,071** | **794** |
| **With optional rows** | **3,386** | **3,020** |

Risk: research scripts are cheap to keep out of the build path, since nothing ships them. Their cost
is gate time (gate full runs every script test) and reading load, not runtime.

---

### 8. .github/

| File | Lines | What it does | Points | Verdict |
|---|---|---|---|---|
| workflows/ci.yml | 93 | On PR and push to main: scope check, 4 script tests, link check, then (unless docs-only) cargo build, fmt, clippy, `cargo test --workspace --exclude goalcup-daemon --exclude goalcup-cli` (ci.yml:73-92). Does not run Worker tests, daemon tests or CLI tests. | none (quality) | keep. Note: CI never tests the daemon, CLI or Worker; only the hand-run gate does. |
| workflows/linux-x86_64.yml | 63 | Manual only: linux-check.sh in Docker, and the no-login harness smoke on a runner | 7 | keep |
| workflows/release.yml | 120 | On `v*` tag: build 3 binaries on 4 targets (macOS arm64 and Intel, Linux x86_64 and arm64 on Ubuntu 22.04 for glibc 2.35), package, smoke, test install.sh, publish with SHA256SUMS | 7 | keep. Needed to ship by Friday. |
| dependabot.yml | 17 | Weekly updates for actions, cargo, npm (server/worker) | none | change: pause until after the event, so no dependency PR lands in the freeze. Risk: a security fix waits a week. |
| CODEOWNERS | 57 | Owner review on connectors, protocol, proxy, redaction, server, packaging | 4, 7 | keep |
| PULL_REQUEST_TEMPLATE.md | 46 | Checklist: gate passed, no credentials, fixtures through redact.py | 4 | keep |
| ISSUE_TEMPLATE/bug_report.yml 72, harness_request.yml 57, config.yml 8 | 137 | Bug and harness request forms; private security link | 7 | keep |

### 9. Root files

| File | Lines | What it is | Points | Verdict / notes |
|---|---|---|---|---|
| Cargo.toml | 128 | Workspace of 5 crates, version 0.7.0, rust 1.99, dev profile tuning, 27 workspace deps | all (build) | keep. Dependency map (grep of crates/*/Cargo.toml): proxy-only deps used by goalcup-cli only: rustls, tokio-rustls, socket2, rcgen, webpki-roots, sha1, flate2, brotli-decompressor, zstd, ureq, x509-parser (tests). Daemon only: redb, regex. Both: axum, tokio. The comment on axum says "The hook receiver and the control endpoint" (Cargo.toml:86-88), but no hook receiver is left in crates/*/src (grep finds none); axum serves the daemon's control API (crates/goalcup-daemon/src/control/). Stale comment, fix. |
| rust-toolchain.toml | 7 | Pins 1.99.0 | build | keep |
| .config/nextest.toml | 61 | nextest profile for the gate, serial groups, priorities | none | keep |
| README.md | 318 | Product intro, participant quick start, commands, how it works, platforms, add a harness, developer section with promises P1 to P7 (README.md:282-288) | 6, 7 | keep, change: P1 to P7 are test promises, not the seven VISION points; align the names to avoid two lists called "seven". |
| TODO.md | 350 | Task list: research validation, dates, work to feature stop 4 Nov, WP0, WP10 release (6 Nov 12:00 UTC planned), parked, clean-ups, open questions, known issues | none | keep, prune after the plan is set. |
| VISION.md | 44 | The seven points | all | keep |
| AGENTS.md | 110 | Map, hard rules, the gate, "done" | none | keep |
| CLAUDE.md | 16 | Points to AGENTS and VISION, lists the seven points | none | keep |
| SECURITY.md 127, CONTRIBUTING.md 126, CODE_OF_CONDUCT.md 132, LICENSE 202, NOTICE 15 | 602 | Policy and legal | 4 (SECURITY) | keep |
| .gitignore | 38 | Ignores harnesses/* except install.sh and README.md (README.md does not exist there), dev homes, spikes/*/out, target, dist | 4 | keep; drop the `!harnesses/install.sh` line if that script is cut. |
| .gitattributes | 3 rules | LF line endings; `*.jsonl -text` | none | keep |
| .claude/agents/campaign-cell.md | 31 | A read-only watcher agent for one campaign cell | none (testing) | keep while campaign.py is used |

---

### 10. Summary verdict table (brief format)

| Part (path, src, tests) | What it does | Points | Verdict | Reason | Lines saved (src / tests) | Risk | Tests that go |
|---|---|---|---|---|---|---|---|
| server/worker/src core (index, auth, chunks, context, db, db-supabase, device, env, errors, sign, util, runs), 2,859 src | Upload API | 5, 4 | keep | Point 5 | 0 / 0 | none | none |
| server/worker disk vitals and disk_full notice (runs.ts, db.ts, notices.ts, SQL in website) | Disk email and vitals | none after owner decision | cut | "Disk full: no special handling" | about 60 to 120 / unverified | Needs a website migration too; the heartbeat must still accept old fields until clients update (no backward compatibility needed before Edition 01, so the client can change at once). | disk_full cases in notices.test.ts, vitals cases in integration and lifecycle tests |
| server/worker/src/github.ts, 223 | Judged commit in cron | none | change (after Friday) | Judging, not recording | 223 / about 184+ | GitHub data may not be readable later (unverified) | github.test.ts, judged-commit cases |
| server/worker/src/db-local.ts, 701 / local-db.test.ts 590 | Test backend | none | keep for Friday | All local e2e tests use it | 0 now | Losing the local upload tests | none now |
| server/worker/src/ratelimit.ts, 139 / 184 | Device-flow limits | none | keep | Guards a real cost, says so | 0 | none | none |
| server/worker/sql, 4,468 data | Mirror of website SQL | 5 (reference) | keep, trim later | Authority is the website repo | 0 src | Drift from the website | none |
| connectors/*, 556 data | Four harnesses | 1, 2, 4, 7 | keep; change codex redaction rows | Owner masking decision b | about 4 to 7 data lines | Pin and golden updates in the same commit | golden files change, none deleted |
| OpenCode and OpenClaw sheets (branch only), 247 data | Facts | 7 | keep as facts; no connector by Friday | SQLite stores; OpenClaw proxy override | 0 | OpenClaw traffic can bypass the recorder silently | none |
| packaging/install.sh, 394 / 614 | Installer | 7, 3 | keep | Participants need it | 0 | none | none |
| harnesses/install.sh, 62 | Research installer | none | cut | Unused, unpinned | 62 / 0 | Two doc lines point to it | none |
| skills/*, 739 data | Agent skill text, setup skill, contributor skill | 6, 7, 2 | keep | goal-cup skill is embedded in the binary | 0 | none | none |
| spikes/redact.py, 349 | Second redactor | 4 | keep | Cross-checks secret masking | 0 | none | none |
| spikes/* fixtures and prototypes, about 23,400 data | September spike artifacts | 2, 4 (a few files) | cut most, keep the files tests read | Outdated harness versions, dead prototype plugins | about 1,044 prototype code + about 15,000 data / 0 | Corpus test needs more than 50 jsonl files and the opencode hit (redaction.rs:615-650); change that test in the same commit | the corpus assertion changes |
| scripts research-* (measure, watch, replay), t25 | Research and disk-full measurement | none | cut | Not one of the seven points; disk-full decision | 1,071 / 794 | Lose research tooling for after the cup | test_research_measure, test_research_watch, test_research_replay |
| scripts/check-links.py, check-version.py | Doc and version hygiene | none | change: shrink | 3,021 and 334 lines for hygiene | up to about 1,545 / 1,800 | Broken participant doc links | test_check_links.py, test_check_version.py |
| scripts matrix.sh, linux-matrix.sh, campaign.py, kit_settle.py | Real-run tests | 7, 2, 5 | keep, split after event | Tests the way people run it | 0 now | They are large (5,588 src, 4,071 tests) | none now |
| scripts/fetch-run.sh, 349 / 695 | Rebuild a run from the server | 5 | keep | Proves point 5 end to end | 0 | none | none |
| scripts release, package, smoke, licences | Release | 7 | keep | Ship by Friday | 0 | none | none |
| .github workflows | CI, Linux, release | 7 | keep; pause dependabot | Freeze | 0 | Delayed security update | none |
| Cargo.toml | Workspace | all | keep; fix stale axum comment | Accuracy | 0 | none | none |

### 11. Blind spots and open items

- I did not run any test, so the counts of test cases that go with each cut are estimates.
- CI never runs the daemon, CLI or Worker suites (ci.yml:92 excludes daemon and cli; no npm step).
  Only the hand-run `gate.sh full` covers the Worker. A Worker change for Friday must be tested with
  `npm --prefix server/worker test` by hand.
- Seal document and tunnel list caps (16 MiB and 8 MiB, runs.ts:26-27; client 16 MiB,
  finish.rs:254) may matter for week-long runs with thousands of subagents. Whether seal.json or
  tunnels.jsonl grow with subagent or connection count is unverified here; the crate audit should check.
- Hermes subagent data in state.db is captured only at the end (`final_snapshot`), a point 2 risk
  for runs that die hard.
- OpenClaw's own proxy setting can route model traffic around the recorder with no gap recorded; if
  OpenClaw is ever allowed, this breaks point 1 silently.
- The EVENT_ID switch (wrangler.toml:39-44) is a manual deploy step in the 6 Nov to 7 Nov window.


---

## A.11 docs/ and root Markdown (part A5)

### How this was done

- Every file under `docs/` (71 files, including 4 `.txt` workloads) and every root `.md` (8 files)
  was listed with `find` and counted with `wc -l`. All counts below are doc lines, not code.
- Each file's head was read. Large files were checked with targeted greps for the owner decisions
  (disk handling, caps, v5 freeze, backward compatibility, byte match, masking rules, headers,
  subagents by config) and then read around each hit.
- "Points" means the seven points in `VISION.md:28-34`. "Owner decisions" means
  `docs/plans/2026-10-29-record-everything-progress.md:80-100`.
- Status words: **current** (true today and in line with the points), **stale** (facts that were
  true once and are now wrong or dated), **superseded** (replaced by a newer doc or decision;
  history), **conflicts** (says something the seven points or the owner decisions now reject).
  A file can be current overall and still hold a conflicting line; the line is quoted.
- The two OpenCode and OpenClaw facts sheets exist only on `origin/opencode-openclaw` (bf15fe85,
  28 Oct) and were read with `git show`.
- `docs/research/results/2026-10-30-two-hour-run.md` does not exist on main (see the note at the top). It is named only as
  a future output in `docs/plans/2026-10-29-next-prompts.md:44`. Nothing below relies on it.

### Totals

| Area | Files | Lines |
|---|---|---|
| Root `.md` | 8 | 1,223 |
| `docs/*.md` (top level) | 16 | 7,728 |
| `docs/plans/` | 13 | 2,794 |
| `docs/research/` (top level, incl. 4 `.txt`) | 20 | 2,578 |
| `docs/research/results/` | 14 | 1,294 |
| `docs/harnesses/` | 5 | 911 |
| `docs/archive/` | 3 | 958 |
| **All docs on main** | **79** | **17,486** |
| Branch-only sheets (`origin/opencode-openclaw`) | 2 | 247 |

Rough split by status (main only): current about 6,900 lines; current but holding at least one
conflicting line about 6,200; stale or superseded history about 4,400.

### The conflicts that matter most (quoted, path:line)

These are the lines that contradict the seven points or the owner decisions. They are grouped by
the decision they break. Fixing them is mostly deletion of a sentence or a row.

#### A. "Disk full: no special handling. The harness stops and the user cleans up."

The D65 disk pause (SIGSTOP below 1 GiB, SIGCONT above 3 GiB, a status PROBLEM line, a heartbeat
vital and a `disk_full` email) and the 3 GiB per-run reserve file are described as current rules
in 19 places.

| Where | Quote |
|---|---|
| `AGENTS.md:84-86` | "Two exceptions: D65, on a nearly full disk Goal Cup Proxy freezes the agent and resumes it by itself" (a hard rule, so every agent reads it first) |
| `README.md:79-80` | "If the disk is nearly full (under 1 GiB free beyond the run's reservation), the agent is paused until space returns, then resumes by itself." |
| `docs/RULES.md:324-325` | "A nearly full disk pauses the agent. Under 1 GiB free beyond the run's reservation Goal Cup Proxy freezes the agent" |
| `docs/PARTICIPANT-GUIDE.md:243-244`, `:325` | "under 1 GiB free beyond the run's reservation, `goalcup` pauses the agent until you free space" |
| `docs/DECISIONS.md:692-725` (D65), quote `:699` | "Freeze, not prompt. The watch tick (every 30 s) sends `SIGSTOP` to the harness" |
| `docs/DECISIONS.md:229` (D40) | "Disk reservation. One 3 GiB reserve file for each run (`RUN_RESERVE_BYTES`)" |
| `docs/DECISIONS.md:186` (D37.3) | "Real recorder disk capacity is required before launch, not a free-space check" |
| `docs/DECISIONS.md:43` (status table) | "D65, D66, D67 / current" |
| `docs/UPLOAD-API.md:380-407`, `:554`, `:586-590` | "A heartbeat whose vitals say `"agent_paused": "disk"` starts a disk pause" and the `disk_full` email rules |
| `docs/SERVER-DESIGN.md:14`, `:139-145` | the server's "must do" list and the `disk_full` notice |
| `docs/OPERATING.md:457` | edge case 4, "Disk nearly full", the pause drill |
| `docs/ARCHITECTURE.md:146-147` | "The run host holds one disk reservation per run (D40 as amended)" |
| `TODO.md:126` | "pause the agent below 1 GiB free, resume above 3 GiB (D65)" |
| `docs/plans/2026-10-20-event-readiness.md:15`, `:97-107` (WP-C) | "1 GiB always reserved for the recorder" |
| `docs/research/NOTIFICATIONS.md:28`, `:55`, `:102`, `:126` | the disk pause email rows |
| `docs/research/LOCAL-CAMPAIGN.md:206-207` | "below 1 GiB free the agent is paused, above 3 GiB it resumes by itself" |
| `docs/research/LOCAL-NEXT-STEPS.md:144-148` | D65 drill checklist |
| `docs/research/coverage-matrix.md:63` | "D65 PASS: paused below 1 GiB within 35 s" |

Blind spot: removing the pause also changes point 3 behaviour. Today a pause *slows* the harness
on purpose (it is a SIGSTOP), which itself conflicts with point 3. With no pause, a full disk means
the harness and the recorder both hit ENOSPC. `docs/research/t25-disk-full.md:8-9` measured that
before WP-C "a full disk silently ends the recording". If the pause goes, the doc set must still
say plainly (point 6) that a full disk ends recording, and the `StorageFull` gap rule
(`docs/RULES.md:322-323`) must stay true.

#### B. "Raise or remove the open-stream limits (128, 32 native) and the 1024-file cap" and "no caps of our own"

| Where | Quote |
|---|---|
| `docs/CONTRACTS.md:28` | "`MAX_OPEN_STREAMS` / 128 / Open payload streams per entry" (also `:29`, `:31`, `:207`) |
| `docs/HARNESS-ONBOARDING.md:198-202` | "A per-run native ceiling. All native rows of one run share 32 open streams and 1024 tracked files (the defaults)." |
| `docs/harnesses/claude-code-cli.md:163-170` (section 11) | "Open streams: 32 at once across all native rows" and "Tracked files: 1024 across all rows" |
| `docs/DECISIONS.md:483-485` (D57) | "A file kept out for good (past `max_files`, or refused) is a `LimitExceeded` gap." |
| `docs/DECISIONS.md:491-496` (D58) | "the receiver refuses a handshake while 128 are unsettled" and "`MAX_CONNECTION_FAILURES` (10 in a row) still ends the wire leg" |
| `docs/ARCHITECTURE.md:189` | "`MAX_LIVE_PRODUCERS` 128 (D58)" |
| `docs/DECISIONS.md:1145`, `docs/CONTRACTS.md:543-546`, `docs/CHEAT-CHECKLIST.md:124-125` | tunnel list "at most 10,000 rows plus a dropped count" and 8 MiB |
| `docs/plans/2026-10-29-record-everything-status.md:53-54` | "After five restarts in one run, the recorder refuses calls for the rest of the run. Five is a board decision, not from the spec." (a cap of our own that also blocks the harness, point 3; the code has `MAX_RECORDER_RESTARTS` at `crates/goalcup-cli/src/proxy/mod.rs:1022`, checked at `:1079-1098`; the "refuses calls" behaviour is the doc's claim, not checked here) |
| `docs/plans/2026-10-29-record-everything-phase2-notes.md:758-759` | "Concurrency cap: at most three subagents alive at once" (a cap written into the 2-hour test design) |
| `docs/plans/2026-10-29-record-everything-phase2-notes.md:270`, `:279` | "Raise the default to 4096" / "File cap, 1024 or 4096" (superseded by "raise or remove") |

Not a conflict: `RUN_MAX_COMPRESSED_BYTES` (64 GiB) in `docs/SERVER-DESIGN.md:120` and
`docs/UPLOAD-API.md:302` guards storage cost and says so; the owner has it as an open decision.
`CHAIN_QUOTA_BYTES` 1 TiB (`docs/CONTRACTS.md:26`) is accounting only and never allocated.

#### C. "No backward compatibility before Edition 01" and "Protocol v5 is frozen no longer applies"

| Where | Quote |
|---|---|
| `docs/CONTRACTS.md:510-560`, quote `:529` | "What is frozen. `M2_PROTOCOL_VERSION = 5` is the only switch." and `:518-519` "An additive v5 field needs a DECISIONS entry that accepts older verifiers failing newer traces." |
| `docs/ARCHITECTURE.md:191` | "The v5 freeze. Records freeze: any new field on a v5 record is v6" |
| `docs/DECISIONS.md:404` (D54) | "v5 bytes and every v5 trace's verdict are frozen." |
| `docs/DECISIONS.md:262` (D42) | "The protocol freezes at v5 for v0.x." |
| `docs/DECISIONS.md:792-797` (D68) | "Compatibility. While schema 1 is read, a connector that loads today keeps loading" (D68 is still "proposed, owner to approve", `:44`) |
| `docs/CONNECTOR-SCHEMA.md:94-120`, quote `:98` | "A connector that loads today keeps loading, under the same rules" plus a 14-day deprecation rule |
| `docs/DECISIONS.md:883-884` (D70) | "From 0.7.0 to 15 Nov 2026 a release keeps `CONTROL_API_VERSION` (12), pinned by tests" |
| `docs/DECISIONS.md:1156`, `docs/CONNECTOR-SCHEMA.md:110`, `docs/plans/2026-10-28-one-capture-system-spec.md:57` | "`providers.toml` is frozen." |
| `docs/plans/2026-10-29-record-everything-spec.md:41-44` (rule 8) | "protocol v5 for the event. New data (headers) goes in as new streams that older verifiers skip." |
| `docs/plans/2026-10-29-record-everything-plan.md:12`, `:25` | "Protocol v5 is frozen (D54): no new record type, field or `CaptureSource`." |
| `docs/plans/2026-10-28-one-capture-system.md:12`, `:27` | "No v5 record changes." |
| `docs/plans/2026-10-29-record-everything-progress.md:9` | "Protocol v5 stays frozen; new data is new streams only." (the same file overrides it at `:84-85`) |
| `docs/plans/2026-10-29-record-everything-status.md:52`, `:106` | "the format is frozen" given as the reason no gap reason was added |

Blind spot: the freeze text also protects the reader corpus and the `P2` promise ("the chain
catches a stray edit", `README.md:283`). Lifting the freeze is fine for compatibility, but the
chain and verifier still need one agreed version for Edition 01. Say that instead of deleting
the whole section.

#### D. "Byte-for-byte match of subagent files against disk is no longer required"

| Where | Quote |
|---|---|
| `docs/plans/2026-10-29-record-everything-plan.md:307` | pass line "every subagent transcript on disk is in the chain byte for byte" |
| `docs/plans/2026-10-29-record-everything-status.md:43-45` | open item 3, now decided |
| `docs/plans/2026-10-29-record-everything-progress.md:72-73`, `:78` | the measured-run "Fails byte-for-byte" verdicts |

`docs/OPERATING.md:315` and `docs/UPLOAD-API.md:5` also say "byte for byte", but about the local
chain against the server copy and about client/server agreement. Those are point 5 checks, not the
dropped requirement. Keep them.

#### E. Masking option b (add the URL password rule, drop the email rule and the Codex rate-limit rules)

| Where | Quote |
|---|---|
| `docs/plans/2026-10-29-record-everything-phase2-notes.md:485` | "Emails and rate-limit messages. Keep the masking as it is." |
| `docs/harnesses/codex-cli.md:135` | "`rate_limits` object carries account plan type, credit balance and quota (measured; Goal Cup redacts it)" (true for the code today, `connectors/codex-cli/harness.toml:110-111`; wrong once option b lands) |

`docs/RULES.md:369-372` already warns that Codex stamps plan type, credit balance and quota into
token-count events, so the participant wording stays right after option b.

#### F. Point 3 "Never block or slow the harness"

| Where | Quote |
|---|---|
| D65 pause (group A above) | a deliberate SIGSTOP of the harness |
| `docs/DECISIONS.md:614-626` (D62, narrowed by D74) | "Refused at start. A missing value, a non-URL, plain `http` ..." (narrowed: with `absent = "defaults"` only a present bad value refuses; a refusal before launch does not slow a running harness, so this is a tension, not a break) |
| `docs/plans/2026-10-29-record-everything-status.md:53` | five-restart cap, then calls refused (group B) |
| `docs/DECISIONS.md:495` | "`MAX_CONNECTION_FAILURES` (10 in a row) still ends the wire leg" (ends recording, not the harness; a point 1 risk over a week of flaky network; unverified whether Task 10's restart supervisor changed this) |

#### G. Point 1 "to any provider" against "declared hosts only"

These are design statements that are narrower than point 1. They are accurate today, so they are
not stale, but a fresh build from the seven points would have to answer them.

| Where | Quote |
|---|---|
| `docs/DECISIONS.md:59` (D1) | "It decrypts only the provider hosts the connector declares and tunnels every other host." |
| `docs/ARCHITECTURE.md:25-26` | "1. Bounded capture. The harness's declared native artifacts and model traffic for the run." |
| `AGENTS.md:4` | "It records declared model traffic and native session artifacts" |
| `docs/HARNESS-ONBOARDING.md:190` | "A local or loopback model ... A connector that names a loopback or single-label host is refused" |
| `docs/CHEAT-CHECKLIST.md:130-144` | a Codex profile provider, Bedrock or a local `ollama` provider "tunnels, sealed and unrecorded" or "is not seen at all" |
| `docs/plans/2026-10-29-record-everything-phase2-notes.md:482-484` | "Several providers. Not covered today. ... This is the largest real gap" |

#### H. Point 7 "never force subagents by config"

| Where | Quote |
|---|---|
| `docs/LOCAL-RESEARCH-HANDOFF.md:112-113` | "`[features]` / `multi_agent = true`" written into the Codex home for the 18 Oct run (enables the tool by config; the workload then asks for subagents in the prompt). History doc; borderline. |

The current test prompts ask for subagents in the prompt, which is what VISION wants:
`docs/research/campaign-build-workload.txt` ("Use your harness's subagent or task tool at least
twice") and `docs/plans/2026-10-29-next-prompts.md` (prompt 1).

#### I. "Simplify first" against older schedule text

| Where | Quote |
|---|---|
| `docs/research/SIMPLIFICATION-2026-10-21.md:4-5` | "anything that moves lots of code waits until after the event" |
| `TODO.md:42`, `:195-200`; `docs/OPERATING.md:52` | "4 Nov 00:00 / Feature stop: after this, fixes only" (a cut landing 4 to 6 Nov breaks this rule as written) |
| `TODO.md:228-248` | "Larger clean-ups ... Anything not done by 4 Nov moves to after the event" |

### Stale facts (wrong today, not a value conflict)

| Where | What is wrong |
|---|---|
| `README.md:150-151` | "Bytes are relayed unchanged; no header is recorded." Task 7 (#293) added a masked header stream per exchange; `docs/RULES.md:336-345` and `docs/CONTRACTS.md:69-70` already say so. |
| `docs/ARCHITECTURE.md:214` | "No header is recorded." Same as above. |
| `docs/CHEAT-CHECKLIST.md:203-205` | "The harness's own login header is never stored: headers are skipped unread (`.../http.rs:726` ...)". The login header is masked inside a recorded header stream now; the line number is likely stale too (unverified). |
| `SECURITY.md:68` | "Only declared provider hosts and paths are decrypted and retained." Task 1 (#292) records every path on an opened host. |
| `AGENTS.md:24-27` | "Read first" points at the 20 Oct event readiness plan and `LOCAL-NEXT-STEPS` (22 Oct, branch `smart/zen-gates-d9tmbh`). Neither is the current plan. |
| `docs/README.md` (map) | Omits `VISION.md` and the four newest plan files (progress, spec, status, phase2 notes, next prompts). `:30` lists the event readiness plan as current organizer reading. |
| `TODO.md:93` | "Wire-only core ... is deferred: not before 4 Nov." Done by D74 (zero native rows). |
| `TODO.md:213`, `:333` | "Read Hermes's `state.db` ... not in 0.7.0" / "Open: a reader for Hermes's `state.db`". Task 6 (#297) copies it once at finish. |
| `TODO.md` overall | Status blocks are dated 21 and 22 Oct. Nothing on Tasks 1 to 12 (merged #289 to #300) or the 29 Oct owner decisions. AGENTS calls it "The single task list". |
| `docs/RESEARCH-TARGET.md:43` | "Known native selectors omit some subagent artifacts, notably Claude's deeper transcripts." Task 5 (#296) added the subagent rows. |
| `docs/RESEARCH-TARGET.md:32-35` | "storage/processing are bounded ... Measure actual effects before widening scope." Overtaken by "record everything". |
| `docs/research/HOW-A-RUN-ENDS.md:44-49` | "Ideas we dropped ... An MCP server." `goalcup mcp` with a `time_left` tool now ships (D66, `README.md:102`). |
| `docs/research/RESTARTS-DESIGN.md:35-37` | Paths name `~/goalcup-work/<clone>`; the clone flow was removed by D71. |
| `docs/harnesses/claude-code-cli.md:3` | "Facts for version: `2.1.283` ... Last checked: 2026-10-13"; later runs used 2.1.299 (`TODO.md:145`). |
| `docs/harnesses/codex-cli.md:9-11` | "17 Oct update" banner over "Earlier pending rows"; sheet reads as a log, not a current sheet. |
| `docs/plans/2026-10-29-record-everything-status.md:40-45` | Open items 2 and 3 are now decided by the owner. |
| Branch sheets `opencode-cli.md:5-6`, `openclaw-cli.md:5-6` | "Status: facts only. ... SQLite, which schema 1 cannot capture". Written 28 Oct before Task 6 added `final_snapshot` (a database copied once after exit). `docs/HARNESS-ONBOARDING.md:188` still refuses a database "the harness keeps writing during the run" but allows one copied after exit, as Hermes's `state.db` is. Whether that fits OpenCode or OpenClaw is unverified. |

### Inventory, file by file

Audience key: P participant, D developer or agent, B harness builder, O organizer, R researcher,
Own owner. Points are 1 to 7 from VISION; "none" means the file serves no point directly.

#### Root Markdown

| File | Lines | Audience | Points | Status | Notes |
|---|---|---|---|---|---|
| `VISION.md` | 44 | everyone | defines 1 to 7 | current | The source of truth. Not linked from `README.md` or `docs/README.md`; only `AGENTS.md:12` and `CLAUDE.md:4` link it. |
| `CLAUDE.md` | 16 | D | all | current | Points to AGENTS and VISION, repeats the seven points. |
| `AGENTS.md` | 110 | D | all (map, rules) | conflicts | `:84-86` D65 exception (A). `:24-27` stale read-first list. `:4` narrower than point 1 (G). |
| `README.md` | 318 | P, D | 6, 7 | conflicts | `:79-80` disk pause (A). `:150-151` "no header is recorded" stale. Its "promises P1 to P7" (`:276-288`) are a different list from the seven points; same labels, easy to confuse. |
| `TODO.md` | 350 | D, Own | none directly | stale | See stale table. `:126` D65 (A). `:42`, `:228-248` schedule (I). Most content is 21 to 22 Oct status. |
| `CONTRIBUTING.md` | 126 | outside contributors | 7 | current | Fork, PR, licence, connector data contributions. |
| `SECURITY.md` | 127 | reporters, reviewers | 4 | current, one stale line | `:68` stale (headers and paths). |
| `CODE_OF_CONDUCT.md` | 132 | community | none | current | Standard Contributor Covenant text. Keep as is. |

#### docs/ top level

| File | Lines | Audience | Points | Status | Notes |
|---|---|---|---|---|---|
| `docs/README.md` | 83 | everyone | none (navigation) | stale | Missing VISION and the 29 Oct plan files; lists history as current. |
| `docs/ARCHITECTURE.md` | 350 | D | 1, 3, 4, 5 | conflicts | `:191` v5 freeze (C), `:189` 128 producers (B), `:214` header line stale, `:25-26` "Bounded capture" (G), `:146` reservation (A). Glossary (section 12) is useful. |
| `docs/CHEAT-CHECKLIST.md` | 305 | D, R, reviewers | 4 (and anti-cheat, which is not one of the seven) | current, one stale line | `:203-205` headers "skipped unread". Long; its value is the list of accepted blind spots. |
| `docs/CONNECTOR-SCHEMA.md` | 1,002 | B | 7 | conflicts | `:94-120` compatibility promise and deprecation rule (C), `:110` providers frozen. The longest doc. Field reference is needed while connectors exist. |
| `docs/CONTRACTS.md` | 654 | D | 1, 3, 4, 5 | conflicts | `:28` stream limit (B), `:510-560` v5 freeze (C). Read by a test: `crates/goalcup-protocol/tests/it/m2_contract.rs:1481`. |
| `docs/DECISIONS.md` | 1,162 | D | all | conflicts | Status table (`:13-48`) dated 28 Oct and does not reflect the 29 Oct decisions. Conflicting entries: D37.3 `:186`, D40 `:229`, D42 `:262`, D54 `:404`, D57 `:483`, D58 `:491-496`, D65 `:692-725`, D68 `:792`, D70 `:883`, D74 `:1145`, `:1156`. Read by `scripts/check-version.py:47`. |
| `docs/HARNESS-ONBOARDING.md` | 579 | B | 7 | conflicts | `:198-202` native ceiling (B). Otherwise updated for Task 6 (`:188`). |
| `docs/HARNESS-SETUP.md` | 468 | P | 6, 7 | current | Per-harness settings for a 168 h run. Read by `crates/goalcup-protocol/tests/it/harness_conformance.rs:127-143`. |
| `docs/LOCAL-RESEARCH-HANDOFF.md` | 306 | R (history) | none | superseded | Self-labelled "history (18 Oct 2026)" at `:3`. `:112-113` enables subagents by config (H). |
| `docs/OPERATING.md` | 863 | O, D | 5 (and operations) | conflicts | `:457` disk drill (A), `:52` feature stop (I). Deploy section matches "the owner deploys the Worker from a local device" (`:83-97`, deploys are manual). The matrix section alone is about 280 lines (`:154-434`). |
| `docs/PARTICIPANT-GUIDE.md` | 330 | P | 6 | conflicts | `:243-244`, `:325` disk pause (A). Otherwise the plain participant path, point 6. |
| `docs/RESEARCH-TARGET.md` | 115 | R | 1, 2 | stale, partly superseded by VISION | `:43`, `:32-35` stale; dated 18 Oct; still the "read before planning capture work" doc in `AGENTS.md:27`. |
| `docs/RULES.md` | 489 | P | 6 (event contract) | conflicts | `:324-325` disk pause (A). The "What a trace holds" text (`:336-348`) is current after Task 7 and is the best point 6 text in the repo. |
| `docs/SERVER-DESIGN.md` | 164 | O, D | 5 | conflicts | `:14`, `:139-145` disk notice (A). `:157-159` "Not built: The verify job that reassembles stored chunks" is a real point 5 gap: the server stores but nothing rebuilds the story server side. |
| `docs/TERMINATION.md` | 262 | D | none directly (ends the harness at the deadline; supports 5 via the seal) | current | Precise contract; long for what the seven points need. |
| `docs/UPLOAD-API.md` | 596 | D, O | 5 | conflicts | `:380-407`, `:554`, `:586-590` disk pause vitals and email (A). Read by three Worker tests and `scripts/gate.sh:91`. |

#### docs/plans/

| File | Lines | Audience | Points | Status | Notes |
|---|---|---|---|---|---|
| `2026-10-18-shared-manual-seal.md` | 46 | D | none | superseded | Done plan; docs map lists it under History. |
| `2026-10-19-research-readiness.md` | 110 | D, R | 1, 2 | superseded | Done plan; History. |
| `2026-10-20-event-readiness.md` | 221 | O, D | 3, 5 | superseded, conflicts | `:15` "1 GiB always reserved for the recorder", WP-C `:97-107` (A). Still pointed at as current by `AGENTS.md:25`, `docs/README.md:30`, `TODO.md:97`, and named in `scripts/campaign.py`. |
| `2026-10-23-release-is-the-list.md` | 198 | D | none | superseded | Self-labelled "Partly cancelled on 25 Oct" (`:3`); D70 is the rule. |
| `2026-10-28-one-capture-system-spec.md` | 73 | D, Own | 1, 7 | superseded | Built and recorded as D74. `:57` providers frozen (C). |
| `2026-10-28-one-capture-system.md` | 368 | D | 1, 7 | superseded | Done (all steps ticked). `:12`, `:27` v5 frozen (C). |
| `2026-10-28-one-capture-system-ledger.md` | 41 | D, Own | 1, 2, 7 | superseded | L33 (`:41`) is the 32/1024 budget question, now answered by the owner. |
| `2026-10-29-record-everything-spec.md` | 81 | Own, D | all | current, one conflict | Rules 1 to 17 are close to the seven points. Rule 8 (`:41-44`) conflicts (C). Rule 7 (`:38-40`) "number of subagents equals the number of distinct conversations" is a measurable point 2 check worth keeping. |
| `2026-10-29-record-everything-plan.md` | 328 | D | all | superseded (Tasks 1 to 12 merged) | `:12`, `:25` v5 frozen (C); `:307` byte-for-byte pass line (D). Task 13 (measured runs) still open. |
| `2026-10-29-record-everything-progress.md` | 100 | Own, D | all | current | Holds the newest owner decisions (`:80-100`). Internal conflict: `:9` "Protocol v5 stays frozen" against `:84-85`. |
| `2026-10-29-record-everything-status.md` | 213 | Own | all | current snapshot, partly overtaken | Open items 2 and 3 decided since. `:52`, `:106` treat the freeze as binding (C). `:53` five-restart cap (B, F). |
| `2026-10-29-record-everything-phase2-notes.md` | 936 | Own, D | 1, 2, 3, 4 | input only; conflicts | Its own header says "They are input for the next build, not decisions" (`:3-4`). `:485` keep masking (E), `:758` three-subagent cap (B), `:270`, `:279` file cap options (B). Part E questions 3, 4 and 8 are partly answered. The progress file says the decisions win where the notes propose more code. |
| `2026-10-29-next-prompts.md` | 79 | agents | 1, 2, 7 | current | The 2-hour run prompt and this audit's prompt. Names `2026-10-30-two-hour-run.md` (`:44`) and `2026-10-30-cut-first-plan.md` (`:76`) as outputs; neither exists on main. Not linked from any doc. |

#### docs/research/ (top level)

| File | Lines | Audience | Points | Status | Notes |
|---|---|---|---|---|---|
| `AMP-FOLLOWER.md` | 86 | D | 2, 7 | superseded | 21 to 22 Oct note; answer "not for the event". |
| `HOW-A-RUN-ENDS.md` | 76 | O | 6 (end of run) | stale | `:44-49` MCP server "dropped" but it ships. The rest (server clock, judged push) is in RULES. |
| `LAPTOP-LOGIN-RUNS.md` | 115 | Own | 7 | current, pending | Subscription-login runs for the owner's laptop. No result rows found on main (`docs/research/results/2026-10-vercel-gateway.md:83-85` still says "left for the laptop"); whether they ran is unverified. |
| `LOCAL-CAMPAIGN.md` | 275 | maintainers | 3, 5, 7 (testing) | stale, conflicts | `:206-207` D65 drill (A). Referenced by `.claude/agents/campaign-cell.md` and `scripts/README.md`. |
| `LOCAL-NEXT-STEPS.md` | 235 | maintainers | testing | stale, conflicts | Dated 22 Oct, branch `smart/zen-gates-d9tmbh`. `:144-148` D65. Still "read first" in `AGENTS.md:26`, `TODO.md:10`, `docs/ARCHITECTURE.md:15`, `docs/RESEARCH-TARGET.md:7`. |
| `LOCAL-TEST-DAY.md` | 170 | maintainers | testing | superseded | 22 Oct one-day plan; results are in `results/2026-10-22-local-test-day.md`. `:124` D65 drill. |
| `NOTIFICATIONS.md` | 150 | O | 6 | conflicts | Every message a participant gets. Disk pause rows (A). |
| `PARTICIPANT-REVIEW-2026-10-21.md` | 62 | D | 6 | superseded | Section A done (`TODO.md:158`); section B waits for the owner. |
| `RELEASE-IS-THE-LIST.md` | 148 | D | none | superseded | Self-labelled "Cut down on 25 Oct" (`:3-8`). D70 is the rule. |
| `RESTARTS-DESIGN.md` | 150 | O, D | 3, 5 | stale | Pre-D71 paths (`:35-37`). The "continue after a sandbox stop" idea still matters for a Vercel Sandbox run. |
| `SIMPLIFICATION-2026-10-21.md` | 63 | D | none (less code) | superseded, conflicts | `:4-5` "waits until after the event" (I). Its "after the event" list A1 to A13 is a ready-made cut list; A1 and A2 are already done (`:9-11`). |
| `VERCEL-COMPAT.md` | 180 | O | 7 | partly superseded | Self-labelled superseded by D74 for Claude Code and Codex (`:3-6`). Still a read-first for the 2-hour run in a Vercel Sandbox. |
| `coverage-matrix.md` | 164 | R | 7 | stale | Status 22 Oct. `:63` D65 pass row (A). |
| `m2-report-format.md` | 376 | R, D | 1, 2, 6 | current | The research report schema 5 (decoded calls, subagents). Referenced by verifier tests and the decoder skill. |
| `replay-comparison.md` | 48 | R | none | current | Tool doc for `scripts/research-replay.py`. |
| `t25-disk-full.md` | 247 | D | 3 | superseded | 21 Oct measurement behind D65. Its finding (a full disk silently ended the recording before WP-C) is the main risk of "no special handling". |
| `campaign-workload.txt` | 1 | test prompt | 7 | superseded | Older campaign prompt. |
| `campaign-build-workload.txt` | 1 | test prompt | 2, 7 | current | Asks for subagents in the prompt (in line with VISION `:44`). |
| `campaign-long-workload.txt` | 1 | test prompt | 7 | current | Long-run prompt. |
| `codex-luna-30m-workload.txt` | 30 | test prompt | 2, 7 | superseded | 18 Oct 30-minute research prompt. |

#### docs/research/results/

| File | Lines | Audience | Points | Status | Notes |
|---|---|---|---|---|---|
| `2026-10-22-fix-plan.md` | 79 | D | testing | superseded | v1, merged on 22 Oct. |
| `2026-10-22-fix-plan-v2.md` | 102 | D | testing | superseded | Merged. Brotli and zstd decoding (point 1) came from here. |
| `2026-10-22-fix-plan-v3.md` | 83 | D | testing | superseded | Merged; mentions disk-pause drills. |
| `2026-10-22-fix-plan-v4.md` | 60 | D | testing | superseded | Merged; no-prompt settings, cited by D67. |
| `2026-10-22-fix-plan-v5.md` | 43 | D | testing | superseded | PATH in login shells. |
| `2026-10-22-fix-plan-v6.md` | 33 | D | testing | superseded | Home lock collision. |
| `2026-10-22-fix-plan-v7.md` | 20 | D | testing | superseded | Start-up dialog. |
| `2026-10-22-fix-plan-v8.md` | 37 | D | 6 | superseded | Led to HARNESS-SETUP.md. |
| `2026-10-22-fix-plan-v9.md` | 39 | D | none | superseded | Led to `goalcup mcp` (cited by D66 `:750`). Says "Draft for owner review" (`:7`), now built. |
| `2026-10-22-fix-plan-v10.md` | 27 | D | 1 | superseded | TLS greeting loss fix. |
| `2026-10-22-local-test-day.md` | 234 | D, Own | testing | superseded | Evidence; cited by D67 and TODO. |
| `2026-10-24-real-machine-round.md` | 149 | D, Own | testing | superseded | Refers to `LOCAL-REVIEW-BRIEF.md` on another branch (`:3`). |
| `2026-10-26-mac-retest.md` | 99 | D, Own | 4, 7 | superseded but cited | Evidence for the SSH key finding cited by `docs/CHEAT-CHECKLIST.md:168`. |
| `2026-10-vercel-gateway.md` | 289 | O, D | 1, 7 | current evidence | The D74 measurements and the Vercel Sandbox path; read-first for the 2-hour run. |
| `2026-10-30-two-hour-run.md` | (missing) | | | does not exist | Not on main and, per the brief, on none of the 89 remote branches. |

#### docs/archive/

| File | Lines | Audience | Points | Status | Notes |
|---|---|---|---|---|---|
| `2026-10-23-simple-setup.md` | 157 | D | none | superseded | Self-labelled "Archived 25 Oct 2026. Partly overtaken by D71." |
| `PARTICIPANT-GUIDE-0.6.md` | 549 | P (old) | none | superseded | Self-labelled "It is not current." |
| `TODO-2026-10.md` | 252 | D | none | superseded | Finished work; keeps old ticket IDs resolvable for code comments (`:3-6`). |

#### docs/harnesses/ (main) and branch-only sheets

| File | Lines | Audience | Points | Status | Notes |
|---|---|---|---|---|---|
| `README.md` | 51 | B | 7 | current | Rules for a facts sheet; lists four sheets. |
| `amp-cli.md` | 231 | B, D | 7 | current (last checked 19 Oct) | Amp's native leg is a metadata log; subagent ids ride the wire (see `AMP-FOLLOWER.md`). |
| `claude-code-cli.md` | 170 | B, D | 2, 7 | conflicts | Section 11 `:163-170` caps (B). Version header stale. |
| `codex-cli.md` | 242 | B, D | 7 | stale | `:135` "Goal Cup redacts it" (E); dated banners. |
| `hermes-cli.md` | 217 | B, D | 7 | current | Updated for `state.db` `final_snapshot` (`:55`) and Hermes 0.22. |
| `opencode-cli.md` (branch only) | 126 | B | 7 | stale | Facts only; "cannot capture" verdict predates Task 6. OpenCode is not shipped, so point 7 has a gap here. |
| `openclaw-cli.md` (branch only) | 121 | B | 7 | stale | Same as above; also its own proxy settings override `HTTPS_PROXY` (`:7`), a real blocker to check. |

### Docs that serve no point

`CODE_OF_CONDUCT.md`, `docs/README.md` (navigation), `docs/TERMINATION.md` (indirect),
`docs/research/replay-comparison.md`, the three archive files, the two done 18 to 19 Oct plans,
`RELEASE-IS-THE-LIST.md` (both copies), `LOCAL-RESEARCH-HANDOFF.md`, and the ten fix plans. About
2,900 lines.

### Points with weak doc coverage

| Point | Gap in the docs |
|---|---|
| 1 | No doc says plainly that a second provider, a profile provider or a local model is unrecorded, in participant words. The facts are in `CHEAT-CHECKLIST.md:130-144` and the phase 2 notes, not in RULES or the guide. |
| 2 | No participant doc says how many subagents can be recorded. Today the only numbers (32 and 1024) are in builder docs (B), and they are about to change. |
| 3 | No doc lists every place Goal Cup Proxy can stop or slow the harness (pause, restart cap, refusals at start). |
| 5 | `docs/SERVER-DESIGN.md:157-159`: the server-side rebuild ("verify job that reassembles stored chunks") is not built. `scripts/fetch-run.sh` is mentioned in `docs/CONTRACTS.md:549` and `TODO.md:245` as the rebuild path. No doc says how the server orders many subagent files "by ID and time". |
| 6 | Good: `docs/RULES.md:336-348` and `docs/PARTICIPANT-GUIDE.md:314`. These two must be edited with any masking or disk change. |
| 7 | Four shipped harnesses. OpenCode and OpenClaw sheets sit on an unmerged branch. No doc for Cursor and other common CLIs beyond the banned-flags list in `docs/RULES.md:216-224`. |

### Recommendation: keep, cut, change

Verdict per group. "Lines saved" counts doc lines only. Every cut doc stays in git history.

| Part | Lines | Verdict | Reason | Lines saved | Risk if done | What must change with it |
|---|---|---|---|---|---|---|
| `docs/archive/` (3 files) | 958 | cut | Self-labelled not current. Git keeps it. | 958 | Old ticket IDs in code comments (F-xx, H-xx) stop resolving to a file (`docs/archive/TODO-2026-10.md:3-6`). | Links in `docs/README.md:81`, `docs/PARTICIPANT-GUIDE.md:5`, `TODO.md`, `docs/CHEAT-CHECKLIST.md:7`, DECISIONS D69 `:821`. |
| Done plans: 18 Oct, 19 Oct, 23 Oct, the three one-capture-system files, the record-everything plan, the event readiness plan | 1,385 | cut | All done or replaced; several hold v5 and disk text that misleads agents. | 1,385 | `scripts/campaign.py` names the event readiness plan; D70 and D74 link the plan files. | Repoint `AGENTS.md:24-27`, `docs/README.md:30-32`, `TODO.md:97`, D74 `:1107-1110`. |
| Fix plans v1 to v10, local test day results, real-machine round | 906 | cut | History of 22 to 24 Oct. | 906 | D66 cites v9 (`docs/DECISIONS.md:750`), D67 cites v4 and the test day results (`:766-767`). | Drop or repoint those citations. |
| Research history: AMP-FOLLOWER, LOCAL-CAMPAIGN, LOCAL-NEXT-STEPS, LOCAL-TEST-DAY, PARTICIPANT-REVIEW, RELEASE-IS-THE-LIST, RESTARTS-DESIGN, SIMPLIFICATION, t25, coverage-matrix, two old workloads | 1,631 | cut (keep SIMPLIFICATION's A-list and t25's one finding as two lines in the cut-first plan) | Dated 21 to 22 Oct; several conflict (A, I). | about 1,630 | `.claude/agents/campaign-cell.md` and `scripts/README.md` name LOCAL-CAMPAIGN; losing t25 loses the only measurement of a real ENOSPC. | Update those two files; keep the t25 finding somewhere a builder will see it. |
| `docs/LOCAL-RESEARCH-HANDOFF.md` | 306 | cut | Self-labelled history. | 306 | None found in code. | `docs/README.md:74`, `docs/ARCHITECTURE.md:16`, `docs/RESEARCH-TARGET.md:8`, `docs/research/coverage-matrix.md:7`. |
| `docs/RESEARCH-TARGET.md` | 115 | change (fold the still-true lines into VISION or ARCHITECTURE section 1, then cut) | VISION now holds the target. | about 100 | Research scope words ("context-visible tool calls are sufficient") are cited by AGENTS and TODO. | Move that one sentence into ARCHITECTURE. |
| `TODO.md` | 350 | change (rewrite to open items only) | Status blocks are 7 days old; misses the 29 Oct state. | about 250 | Known issues H-29, H-28, F-07 are real; keep them. | Keep "Known issues"; drop dated status. |
| `docs/research/HOW-A-RUN-ENDS.md` | 76 | cut | Its rules live in RULES; one stale claim. | 76 | Small. | `docs/README.md:28`. |
| DECISIONS, CONTRACTS, ARCHITECTURE, CONNECTOR-SCHEMA | 3,168 | change | Mark D65, D54, D42's freeze sentence, D68 and the v5 freeze section as superseded by the 29 Oct decisions; update the limits table when the code changes. | small now; more once code is cut | A test reads CONTRACTS (`m2_contract.rs:1481`) and `check-version.py` reads DECISIONS and CONTRACTS. Editing wording there can fail the gate. | Change with the code, in the same PR. |
| README, RULES, PARTICIPANT-GUIDE, UPLOAD-API, SERVER-DESIGN, OPERATING, NOTIFICATIONS, AGENTS | about 3,800 | change | Remove the disk pause and reserve text only after the code is cut. Fix the two "no header is recorded" lines now. | under 100 | Participant docs must not promise a pause that the shipped binary no longer does, and must not drop it before the binary ships. | Edit in the same PR as the code change. |
| `docs/plans/2026-10-29-*` (progress, spec, status, phase2 notes, next prompts) | 1,409 | keep until the cut-first plan lands, then archive by cutting | They hold the newest owner decisions. | later | Phase 2 notes propose more code than the decisions allow; an agent reading them first may build it. | Add one line at the top of the phase 2 notes and the status file: "decisions in the progress file win". |
| Harness sheets on main | 911 | keep, change two | Point 7 evidence. | 0 | Claude section 11 and Codex `:135` will be wrong after the cap and masking changes. | Edit with the code. |
| `docs/README.md` | 83 | change | Add VISION; drop cut files. | about 20 | Link check (`scripts/check-links.py`, gate full) fails on any dead link. | Run the link check. |

Estimated doc lines cut: about 5,200 of 17,486 (about 30 percent), plus about 350 from rewrites.

### Blind spots and what could go wrong

- **Docs follow code, not the other way round.** Most conflicts describe code that is still on
  main (for example `MAX_OPEN_STREAMS = 128` at `crates/goalcup-protocol/src/capture.rs:58`, the
  email rule at `crates/goalcup-daemon/src/redact.rs:285`, the Codex rules at
  `connectors/codex-cli/harness.toml:110-111`). Editing the docs before the code ships would make
  them wrong in the other direction. The participant docs (README, RULES, guide) must change in
  the same release as the binary.
- **The gate reads docs.** `scripts/gate.sh:91`, `scripts/check-version.py:27-72`,
  `crates/goalcup-protocol/tests/it/m2_contract.rs:1481` and
  `crates/goalcup-protocol/tests/it/harness_conformance.rs:127-143` read doc files, and the full
  gate runs a link check. Cutting files without fixing links fails the gate.
- **History has evidence value.** The fix plans and results are the only record of why some
  rules exist (for example why D67 tells participants to turn off permission prompts). Git keeps
  them, but agents rarely read git history. Keep a one-line pointer (commit hash) in DECISIONS
  where an entry cites a cut file.
- **Two "P1 to P7" lists.** README's promises P1 to P7 (`README.md:276-288`, "never deleted")
  are not the seven points. AGENTS says "Never delete a test in the promises list". A cut-first
  plan that confuses the two could delete a guarded test or keep a dead one.
- **Not verified here:** the line number `http.rs:726` in `docs/CHEAT-CHECKLIST.md:204`; whether
  the five-restart cap really refuses calls (taken from the status doc); whether Task 10 changed
  the `MAX_CONNECTION_FAILURES` rule in D58; whether the laptop login runs happened; whether
  `final_snapshot` fits OpenCode or OpenClaw; the two-hour run results (the file does not exist).
- **Inbound link counts** used for the cut list are from a filename grep and may miss links
  written without the file name.
