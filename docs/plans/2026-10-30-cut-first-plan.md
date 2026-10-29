# Cut-first plan for Goal Cup Proxy, Edition 01

Date: Thursday 29 October 2026. Base: main at 4b226de2.
Status: proposed, needs owner review. Nothing in this plan has been built or run.
Revised the same day after a fact check and two adversarial reviews. Every point and answer is in
[R-reviews.md](2026-10-30-audit/R-reviews.md); section 13 sums up what changed.

**The answer.** Keep the recording core: the proxy that the harness reaches through `HTTPS_PROXY`,
the native session-file reader, the masker, the chain writer, the chunk uploader and the Worker's
chunk store. Cut about 19,000 source lines and 28,000 test lines before Friday 6 November, all of
them deletions of parts that serve none of the seven points: the verifier's research decoders,
research scripts, disk-full handling, the start-time provider probe, byte goldens of connectors and
dead leftovers. Change the parts that block or lose data: raise our own bounds on streams and
connections to values that state their memory cost, delete the native file caps and the restart
caps, make the native poll cheap, and make a full disk end the chain cleanly. Add eight small things,
each tied to a point, such as the URL password rule and one "Recorded:" line for the participant.
Leave the big structural cuts for after Edition 01: the daemon merge, termination proof, signing,
pins, deny lists, the platform rewrite and the Rust copy of the Worker. They save about 25,000 more
source lines, but each one touches working code that ends or proves a 168-hour run.
Every code cut lands by Tuesday 3 November, before the 4 November feature stop (`TODO.md:42`).
Wednesday adds tests only (N4), tags the release candidate and holds the 2-hour run. Thursday is the
buffer, with pure deletes of history docs. Friday is the release. The Friday Worker deploy is
configuration only. All line figures are estimates from line ranges, plus or minus 30 percent.

## 2. The target shape

**The core we keep.** `goalcup start` launches the harness with `HTTPS_PROXY` pointing at a loopback
recording proxy. The proxy decrypts declared model hosts and passes every other host through
unread, naming it. Each exchange goes to the recorder (still a separate daemon for Friday). The
recorder masks secrets, then writes a hash-linked chain. It also polls the harness's session folder
for main and subagent files. The uploader sends 32 MiB chunks to the Worker, which stores them in R2
with a sequence number and `received_at`. `scripts/fetch-run.sh` and `goalcup-verify
--capture-integrity` rebuild a run by ID and time. The participant sees one "Recorded:" line and
`goalcup status`.

```mermaid
flowchart LR
  H[Harness and its subagents] -- HTTPS_PROXY --> P[Recording proxy in goalcup start]
  P -- decrypt declared model hosts --> M[Model provider]
  P -- pass through unread, name host --> O[Other hosts]
  P -- exchanges --> D[Recorder: mask, then chain]
  S[Session files, main and subagents] -- poll: open, read, close --> D
  D --> C[Local chain parts]
  C --> U[Uploader: 32 MiB chunks] --> W[Worker] --> R[R2 and run index]
  R --> F[fetch-run and verify: rebuild by ID and time]
  D -. counts .-> T[Participant: Recorded line and status]
```

**What the two fresh designs agree on** ([B.1](2026-10-30-audit/B-fresh-design.md#b1-both-designs-at-a-glance),
[top findings](2026-10-30-audit/B-fresh-design.md#top-findings)): an environment-set proxy with a run CA,
decrypt model hosts only, a session-root poller that opens, reads and closes each file, one masker
before disk, idempotent `PUT` upload, plain status, and a connector of 15 to 20 lines. No subagent code
is needed: subagents share the parent's network path and write more files. Neither design has a hash
chain, signed acks, termination proof, a daemon control API, disk handling or research decoders. Both
land at 2,500 to 3,800 lines in all (`B-fresh-design.md:15-16`), against about 102,000 production
lines today (85,800 of them in the client crates).

**Where this plan differs, and why.**

| Fresh designs | This plan for Friday | Why |
|---|---|---|
| One process, or a tiny supervisor | Keep the separate daemon | The merge is about 12,000 source lines and touches 22 CLI files ([C 2.2](2026-10-30-audit/C-module-audits.md#22-should-the-daemon-be-a-separate-process-at-all)). After Edition 01. |
| No hash chain or signing | Keep both | The Worker, verifier, uploader and prune all read acks today. Cutting them changes five copies of the contract in one week ([I](2026-10-30-audit/I-server-upload.md#9-hidden-couplings)). After Edition 01. |
| `killpg`, grace, `killpg` | Keep termination proof | It works and was measured. A 168-hour stop is not the place to try new code. After Edition 01. |
| Rewrite in Go | Cut the Rust toward the shape | B1 says so ([B top findings](2026-10-30-audit/B-fresh-design.md#top-findings), item 10); a rewrite does not fit in a week. |
| B1 forces `Accept-Encoding: gzip` | Never change a forwarded byte (B2 and today) | Needs a per-harness test. The proxy already decodes one gzip, deflate, brotli or zstd coding plus permessage-deflate; any other coding is an `UnsupportedEncoding` gap (`docs/ARCHITECTURE.md:211-213`). |
| Wildcard decrypt of cloud hosts | Keep the declared host list and name every passed-through host | Lane E rejects wildcards: they break boto3, gRPC and `cloudflared` and decrypt the agent's own cloud work ([E1 cuts](2026-10-30-audit/E-providers.md#e9-keep-cut-change-provider-facing-parts-only)). |
| Joined-view masking of split secrets (about 80 lines) | Not for Friday | In today's streaming masker it is not small ([G.8](2026-10-30-audit/G-secrets.md#g8-the-model-echo-gap-h-29-and-the-promised-scan-that-does-not-exist-points-4-6-g-surface-g6)). Disclose the gap; build an export-time scan later. |
| SQLite snapshots every 10 to 15 minutes | Keep Hermes's one final snapshot, lift its 64 MiB cap | New code; Hermes subagents reach the trace at seal only. A masked span changes length (`crates/goalcup-daemon/src/redact.rs:1199-1207`, the marker function), so the copy may not open as SQLite, and it holds earlier sessions of the shared home. Say so to Hermes users (AD8, Q4). |

## 3. Numbers

Counted with `wc -l`. "src" is production code; inline `#[cfg(test)]` modules count as tests. Today's
figures are from [A.1](2026-10-30-audit/A-inventory.md#a1-totals-by-area). The "after" columns are
estimates from the line ranges in sections 4 to 6, plus or minus 30 percent. Fixture and spike data
lines are not in these figures (section 4 gives them).

| Area | src today | tests today | src after Friday cuts | tests after Friday cuts | src after later cuts | tests after later cuts |
|---|---:|---:|---:|---:|---:|---:|
| `crates/goalcup-cli` | 38,636 | 69,357 | 35,760 | 64,840 | 30,870 | 53,320 |
| `crates/goalcup-daemon` | 18,959 | 34,559 | 18,800 | 33,850 | 9,100 | 19,550 |
| `crates/goalcup-platform` | 5,039 | 2,540 | 4,360 | 2,120 | 1,960 | 620 |
| `crates/goalcup-protocol` | 11,013 | 14,841 | 11,010 | 14,725 | 6,350 | 5,690 |
| `crates/goalcup-verifier` | 12,153 | 22,059 | 2,410 | 4,620 | 1,850 | 3,040 |
| `server/worker` (src plus ops scripts) | 4,395 | 5,749 | 4,390 | 5,700 | 3,640 | 4,860 |
| `scripts/` | 11,176 | 10,393 | 5,720 | 5,570 | 3,780 | 3,490 |
| Other (`packaging/install.sh`, `harnesses/install.sh`, `spikes/redact.py`) | 805 | 0 | 740 | 0 | 390 | 0 |
| **Total** | **102,176** | **159,498** | **83,200** | **131,400** | **57,900** | **90,600** |
| Plus the adds (section 6) | | | +400 | +800 | | |
| **Total with adds** | | | **about 83,600** | **about 132,200** | **about 58,300** | **about 91,400** |

So the Friday build is about 19 percent smaller in source and 18 percent smaller in tests. After the
later cuts it is about 43 percent smaller in source. The test figure fell from the first draft
because the Rust Worker stand-in (K3), `m2_contract.rs` and four script tests now stay until after
Edition 01. Lane J's test rewrite (keep about 37,000 lines, rewrite about 61,000 down to 30,000) would
take tests further ([J A7](2026-10-30-audit/J-tests-gate.md#a7-recommendations)).

## 4. Cut list

Points checked: which point could suffer, and why it does not. "Before Friday" means merged by Tuesday
3 November. Savings are counted once; where two rows touch the same lines, the row says so.

### 4a. Before Friday (biggest safe cuts first)

| Id | What | Path | src | tests | Risk | Tests that go | Points checked | When |
|---|---|---|---:|---:|---|---|---|---|
| K1 | Verifier research decoders, reports and stats, and the `doctor` decoder check | `crates/goalcup-verifier/src/capture_analysis/{api,registry,session,operations,wire,tracker,anthropic,openai,chat,thread,report,stats}.rs`, `native/*`; `crates/goalcup-cli/src/commands/doctor.rs:246-316` | 9,745 + 71 | 16,720 + 55 (+1,270 fixture lines) | Medium. `scripts/matrix.sh:2051-2064` counts calls with `--research-json`, so AD4 lands in the same PR. `reader.rs:36` imports `wire::SegmentView`: move that one type. Run labels (`wire_and_native` to `none`) and the bypass hint `native_activity_without_wire_streams` live only in `report.rs:375-404`, and `docs/RULES.md:306-321` and `docs/CHEAT-CHECKLIST.md:46-48, 94-97, 133-136, 144` cite them: AD4 prints a label from stream metadata, and PR3 rewords those lines to "labels are computed after the run". After PR8b, branch `decoders-ed01` from main with PR3 reverted, so the judges' decoders read Friday traces with the new bounds. README P6 names decoders (`README.md:287`), so Q1 first. | `m2_*decoder*`, `m2_native_observations`, `m2_research_report` (2,456), `m2_run_stats` (873), `m2_hermes_capture_only` (395), `connector_samples`, verifier `harness_contract_golden` (487), `common/decode.rs`, `common/wire.rs`. Edit, not delete: `every_hinted_path_is_read_by_its_decoder` (`crates/goalcup-cli/src/model_host.rs:1218-1270`), `tests/it/harness_contract_kit.rs:20-22, 511-513`, `tests/it/connector_contract.rs:36-37, 937-944`, `tests/full/connector_contract.rs:381-406`. | 1: the count proof moves to AD4. 5: the integrity reader (`reader.rs`, `lineage.rs`) stays and `fetch-run.sh` uses it. 6: the participant's call count comes from `state.json` (`crates/goalcup-cli/src/run/mod.rs:266`), not the verifier. | Before Friday (PR3) |
| K2 | Scripts that serve no point, and the version check | `scripts/campaign.py` 1,888; `linux-matrix.sh` 876 + `kit_settle.py` 302; `research-measure.py` 541, `-replay.py` 342, `-sample.py` 424, `-staging.py` 604, `-watch.py` 95 (keep `research-staging.sql`); `ci-scope.py` 90; `harnesses/install.sh` 62; `check-version.py` 206 (moved here from K25). Data: `campaign.example.json` 64. Keep `linux-harness-smoke.sh`. | 5,430 | 4,821 | Low. `campaign.py` is the only automated network-cut drill: do it by hand in the 2-hour run. Edit `.github/workflows/ci.yml` (scope step and these tests), `scripts/gate.sh:61` and `scripts/release.sh:101-104` (version check). `check-version.py` reads `docs/CONTRACTS.md` and `docs/DECISIONS.md` (`:46-47`), so it would also break on a later doc merge. | `test_campaign.py` 1,335, `test_linux_matrix.py` 1,219, five `test_research_*.py` 1,467, `test_early_crosscheck.py` 538, `test_ci_scope.py` 134, `test_check_version.py` 128. Keep `test_gate.py`, `test_release.py`, `test_linux_check.py`, `test_codeowners.py` (lane J): they test kept scripts. | 7: live runs keep `matrix.sh`, and `linux-harness-smoke.sh` stays for `scripts/linux-check.sh:99-104` and `.github/workflows/linux-x86_64.yml:63`. 3: the network cut moves to the 2-hour run (section 9). | Before Friday (PR2) |
| K4 | Connector byte goldens | `crates/goalcup-cli/tests/it/harness_contract_golden.rs` 975; `crates/goalcup-daemon/tests/it/harness_contract_golden.rs` 463; `crates/goalcup-protocol/tests/it/connector_template.rs` 120; fixtures `harness-golden` 2,110. Keep `m2_contract.rs` (record format), `frozen-connectors` and `schema-1-connectors` (kept tests read them). | 0 | 1,558 (+2,110 fixture lines) | Low. Keep `chain_golden.rs` (102) and `no_family_literals.rs` (378), which guards the hard rule at `AGENTS.md:87`. `m2_contract.rs` stays as the record format frozen for the cup, so a mid-cup fix release cannot change record bytes by accident. | the files listed | 5: the owner dropped the byte-for-byte rule for subagent files. 7: `shipped_connectors.rs` still loads every shipped connector. | Before Friday (PR4) |
| K5 | Disk-full handling (owner: no special handling), client only | `crates/goalcup-cli/src/run/reserve.rs` (533 src / 423 inline), `run/pause.rs` (211 / 149), the disk half of `vitals.rs`, disk parts of `run/watch.rs`, `run/mod.rs`, `commands/start/phases.rs`, `commands/start/forked.rs` (about 330, est.); `crates/goalcup-platform/src/disk.rs:58-239`; `scripts/t25-disk-full.sh` 93; `tests/full/t25_disk_full.rs` 1,173. Keep process vitals (`open_files`, CPU, memory). The Worker disk code stays until after Edition 01. | about 1,410 | about 1,830 | Medium. A real `ENOSPC` poisons the writer and leaves a torn tail (`crates/goalcup-platform/src/tracecap.rs:673-689`, `crates/goalcup-daemon/src/runtime/capture/writer.rs:206-222`): CH9 fixes this. The 30 s tick (`run/reserve.rs:41`) moves to `run/watch.rs`. The Worker already accepts a heartbeat with null disk fields (`server/worker/src/runs.ts:507`). | reserve and pause inline tests, disk cases of `vitals.rs`; `t25_disk_full.rs`; `platform/tests/it/disk.rs` (137); disk cases in `prep.rs`, `status_views.rs` | 3: the `SIGSTOP` pause itself freezes the harness, so the cut helps; process vitals stay so a handle leak is still seen. 5: the `StorageFull` gap stays (`writer.rs:294`). 6: one plain line says a full disk ends recording (AD8). | Before Friday (PR6) |
| K6 | Spike prototypes and unused spike data | `spikes/cursor-cli` (1,681), `spikes/openclaw` (7,887), prototype plugin code elsewhere in `spikes/` (hermes 374, opencode 269). Keep `spikes/redact.py`, `spikes/opencode` fixtures, chain golden files | 643 prototype (not counted as src) | 0 (+9,568 data lines) | Low. The redaction corpus test reads spike files (`crates/goalcup-daemon/tests/it/redaction.rs:608-650`); keep the OpenCode catch at `:625`. 73 `.jsonl` corpus files remain, above the test's floor of 50. | corpus assertion narrows | 4: the corpus still covers the four shipped harnesses. | Before Friday (PR2) |
| K7 | Start-path leftovers: the start-time provider probe, probe test seams and DNS loopback check, shared-home double recheck, legacy leftovers. Keep session sniffing on `continue`. | `proxy/preflight.rs` (250 src / 170 inline) and its call (`commands/start/refusals.rs:9`, `commands/start/phases.rs:107-142`), `participant/refusals.rs:150-215`, `commands/start/phases.rs:234-245, 282-290`, `model_host.rs:507-540`, `launch.rs:912-920, 1306-1349`, `home.rs:30-90` (keep the `flock` at `:79-95`), `commands/prep.rs:436-445`, `preflight/probe.rs:7-29` (all under `crates/goalcup-cli/src/`) | about 620 | about 910 | Low to medium. Mixed-file estimates. Reserve calls in these files are counted in K5. An offline start shows its problem at the "no model calls" line (`participant/status.rs:467-509`), not at once. `phases.rs:296-361` and `launch.rs:664-746` stay: they hold `started_at_unix`, `model_calls_before` (AD2 needs it) and `resume_failed`. | probe rows in `preflight_doctor.rs`, `start_launch.rs:1026-1110, 1217-1261`, `m2_cli_lifecycle.rs:679-713`, removed-flag tests | 7: `continue` still checks for a session before it resumes. 3: fewer reasons to refuse a start. | Before Friday (PR9) |
| K8 | `goalcup mcp` setup lines, and the legacy `git-credential` and old GitHub token flow. The `goalcup mcp` command itself stays through Edition 01. | `commands/prep.rs:730-760`, four `[participant.time_left_tool]` tables; `account/github.rs` (about 280 of 466), `main.rs:229-240, 327-336` | about 310 | about 150 | Low. Dry-run participants pasted the `goalcup mcp` lines into four harness configs; the command keeps answering, so no harness shows a failed server. 0.6.1 test clones cannot push (test data). | setup-line cases in `prep.rs` tests; git-credential cases in `tests/it/account.rs` | 6: `goalcup time-left`, the deadline file and the MCP tool stay. 3: no failed MCP server at harness start. | Before Friday (PR9), Q6 |
| K9 | Platform dead code and no-point facts | `crates/goalcup-platform/src/tracecap.rs:186-210, 344-352, 546-563, 735-775`, `tracecap/unix.rs:607-739`; `machine.rs:415-466, 541-561` (AC power, container); `handle_budget.rs` (79, goes with CH2, and edits `sourcecap.rs:3, 139, 152`, `sourcecap/unix.rs:2, 131`, `native_capture.rs:576`) | about 500 | about 280 | Low. `docs/RULES.md:366` promises the AC power and container facts: edit it in the same PR. | `descriptor_*`, `sweep_*`, AC power and container cases, budget tests | 5: chain writes still `fsync`. 4: no-follow session reads in `sourcecap` stay. | Before Friday (PR8b and PR9) |
| K10 | Uploader leftovers: `repo_head`, dual `proxy_version` and `tracer_version`, older-server deadline fallbacks. Keep the owed-ack path. | `crates/goalcup-cli/src/upload/uploader/local.rs:132-246`, `session.rs:33-45`, `upload/api.rs:14-69, 115-139, 247-262`, `uploader/heartbeat.rs:204-209, 231-280` | about 230 | about 380 | Low. The Worker must send `seconds_left` on open, continue and heartbeat; keep the local deadline path (`uploader/lifecycle.rs:129-133`). The Worker accepts a null `repo_head` (`server/worker/src/runs.ts:452-453, 472`). The owed-ack path (`session.rs:228-253, 327-347, 483-496`) stays: it lets the upload go on when `acks.jsonl` cannot be written, for example on a full disk. | `repo_head` cases, `api.rs` and heartbeat inline cases | 5: the chunk path is untouched. 6: the deadline still shows. | Before Friday (PR10) |
| K11 | Proxy outage windows and extra counters | `crates/goalcup-cli/src/proxy/outage.rs` (144 / 172), `proxy/mod.rs:647-920, 1176-1224, 1241-1270`, `proxy/connect.rs:402-426` | about 520 (sum of named ranges; C counts about 200 for the outage part alone, so some of `mod.rs:647-920` may stay) | about 290 | Low. The finish loses "provider unreachable from 10:02 to 10:07"; a kept counter gives "could not reach api.example.com 14 times". | outage inline tests; the open flake `an_upstream_outage_is_one_window_closed_by_the_next_reach` (`tests/it/m2_proxy_integration.rs:2853-2916`); `m2_tls_connect.rs:273-303` | 6: the count line stays. 3: nothing here touches forwarding. | Before Friday (PR7) |
| K12 | Daemon writer quota half and V1 row refusal | `crates/goalcup-daemon/src/runtime/capture/writer.rs` (quota part), `runtime/daemon.rs:143, 514` | about 150 | about 250 | Low. The 1 TiB quota is never reached. | storage-quota cases in `runtime/capture/tests.rs`; `capture_lifecycle.rs:2270` block | 5: the `StorageFull` gap is separate and stays. | Before Friday (PR8b) |
| K26 | v5 corpus test (moved here from K14) | `crates/goalcup-verifier/tests/it/v5_corpus.rs` 718; fixtures `tests/fixtures/v5-corpus` | 0 | 718 (+912 fixture lines) | Low. Test-only. The owner ended back-compat before Edition 01; older test traces are no longer re-read. PR8a then has no frozen corpus to regenerate. | `v5_corpus.rs` | 5: `m2_contract.rs` and `chain_golden.rs` still pin the current format. | Before Friday (PR4) |
| | **Total before Friday** | | **about 19,000** | **about 28,000** | | | | |

### 4b. After Edition 01

| Id | What | Path | src | tests | Risk | Tests that go | Points checked | When |
|---|---|---|---:|---:|---|---|---|---|
| K3 | Rust stand-in of the Worker (moved here from Friday) | `crates/goalcup-cli/tests/common/upload_server/` (2,514); `tests/it/upload_server.rs` 1,419, `upload_server_device.rs` 529, `upload_server_segments.rs` 961 | 0 | 5,423 | Medium. Ten other test files import the stand-in, `full/upload_e2e.rs` (2,518) among them; each needs a port to `wrangler dev` or a trimmed fake (C: trim it by about 500 lines). | the 65 stand-in tests | 5: AD7 (N4) tests the real Worker from Wednesday on. | After |
| K13 | Merge the daemon into `goalcup start` (control API, ingest IPC, registry, lifecycle, termination proof, CLI daemon client, spawn and retry, `proxy/ingest.rs`, protocol `control.rs`) | `crates/goalcup-daemon/src/{control,ingest.rs,registry.rs,terminate,runtime/termination.rs,...}`, `crates/goalcup-cli/src/daemon/`, `subreaper.rs`, `released_child.rs`, `launch.rs:1657-1994`, `proxy/ingest.rs`, `crates/goalcup-protocol/src/control.rs` | about 12,000 | about 15,000 | High. Ctrl-C and `SIGHUP` reach the recorder; offline seal is untested; `setsid` children escape `killpg`; README P5 and P7. | `control_api.rs`, `capture_ingest.rs`, `terminate_sim.rs` (2,714), `process_kill.rs` (1,285), `capture_gate.rs` (519), `daemon_client.rs` (1,248), `run_scoped_daemon.rs` | 3: one process can kill its own child group with no pid-reuse risk. 1: a dead `goalcup start` must still leave a gap. | After |
| K14 | Protocol and verifier back-compat, retired versions, seed support, dead V1 status fields, `export.rs` quota branches | `crates/goalcup-protocol/src/export.rs`, `control.rs:283-318`, `capture.rs:309-316`, `envelope.rs:24-28, 119-123`; verifier legacy branches | about 1,350 | about 3,000 | Medium. A control API bump in a week when the protocol already changes. No point gains. | legacy cases in `capture_terminals.rs`, `export_roundtrip.rs`, `roundtrip.rs:521-585` | 5: the reader still reads every current record. | After |
| K15 | Platform rewrite of `tracecap` and `sourcecap` on `std` | `crates/goalcup-platform/src/` | about 2,400 | about 1,500 | High, wide. | most platform tests | 4: keep no-follow reads, or a symlink to `~/.ssh` uploads. | After |
| K16 | Harness deny checks, trust tables and ceremony fields | `crates/goalcup-protocol/src/harness/check.rs:671-1048, 1113-1230, 1365-1412`, `capture_policy.rs:151-1000`, `schema.rs:686-748, 846-905` | about 1,780 | about 3,420 | Medium. Review becomes the only connector guard (D51). Touches the loader of every connector. | about 90 of 114 rows in `harness_schema.rs` | 7: connectors still load. 1: keep "no connector routes around the proxy". | After |
| K17 | Connector pins, folder digest, `pin-manifests.sh`, `check-connector.sh` | `crates/goalcup-protocol/src/pins.rs`, `connector_dir.rs`, `scripts/pin-manifests.sh` 274, `scripts/check-connector.sh` 314 | about 1,240 | about 1,210 | Medium. Packaging, the kit and `GOALCUP_CONNECTORS_DIR` (`crates/goalcup-cli/src/config.rs:132-147`) assume a folder; the Worker wants a 64-hex `connector_dir_sha256`. | `connector_pins.rs`, `pins_build_fails.rs`, `test_check_connector.py` | 7: embed the four `harness.toml` files instead. | After |
| K18 | Receipt, ack and status signing | `crates/goalcup-protocol/src/receipt.rs`, `ack.rs`, `control.rs:353-430`; `server/worker/src/sign.ts`; client ack checks; verifier receipt paths | about 1,000 | about 2,500 | High. Verifier, protocol, client, Worker and corpus change at once; prune must move to server position. Overlaps K13 (status signing). | `signing_vectors.rs`, `sign.test.ts`, receipt cases | 5: append-only offsets, R2 sha256 and `received_at` stay the anti-tamper facts. | After |
| K19 | CLI extras: `login`, SSH key warnings, clipboard push lines, `doctor` shrink, status technical block, `goalcup mcp` | `commands/login.rs`, `ssh_keys.rs`, `push_lines.rs`, `commands/doctor.rs`, `commands/status.rs`, `commands/mcp.rs` (273 / 201) | about 1,770 | about 2,000 | Medium. `packaging/install.sh:100-160` parses `status --json`; `matrix.sh` parses `login` and `doctor`. | `login.rs`, `ssh_keys.rs`, `status_views.rs` parts, `mcp.rs` tests | 6: status keeps the Recorded line; `goalcup time-left` stays. | After |
| K20 | Judged commit to an organizer script. `db-local.ts` stays: N4 runs `wrangler dev` with `DB_BACKEND=local` (`crates/goalcup-cli/tests/full/upload_worker.rs:50`). | `server/worker/src/github.ts` 223 | about 220 | about 180 | Medium. The judged commit with late and force flags is a cheat control (`github.ts:169-222`); someone must run the script. | `github.test.ts` | 5: storage path unchanged. | After |
| K21 | Seal-time workspace walk and Git digest | `crates/goalcup-daemon/src/workspace.rs` 485, `workspace_git.rs` 549 | about 800 | about 600 | Medium. `seal.json` Git fields and verifier `seal_git_*` change. | most of `workspace_layout.rs` (643) | 5: the repository on GitHub holds the work. | After |
| K22 | Finish stage machine to one timeout | `crates/goalcup-daemon/src/runtime/capture/finish.rs` 763 | about 400 | about 900 | Medium. Seal timing bugs. | most of `capture_finish_budget.rs` (1,267) | 3: a single deadline still bounds the end. | After |
| K23 | Rate limiter Durable Object to one zone WAF rule | `server/worker/src/ratelimit.ts` 139 | about 170 | 184 | Medium. Zone rule slot unverified; needs a `deleted_classes` migration. | `ratelimit.test.ts` | none of the seven. | After |
| K25 | `check-links.py`, `spikes/redact.py` oracle | `scripts/check-links.py` 1,349, `spikes/redact.py` 349 | about 1,700 | about 2,470 | Low. `check-links.py` guards this week's doc cuts; the oracle guards this week's masking change. | `test_check_links.py`, `spikes_redact_py_*` | 4: the Rust redaction suites stay. | After |
| K27 | Leftovers moved out of Friday by the reviews: Worker disk vitals and `disk_full` notice (`runs.ts:499-533`, `notices.ts`, about 90); the owed-ack path (about 60); client run give-back and `pending-open.json` (`upload/mod.rs:297-495`, about 160); `m2_contract.rs` (1,715 tests) once the cup ends | as listed | about 310 | about 2,140 | Low. Give-back cut may leave stray `started` or `silence` emails. | `notices.test.ts` disk cases, owed-ack cases, give-back cases, `m2_contract.rs` | 5: no record path changes. | After |
| | **Total after Edition 01** | | **about 25,100** | **about 40,500** | | | | |

K24 (crash and failure campaign tests, about 2,880 test lines) is not counted: it waits for N5,
a new end-to-end test (about 400 lines, [J A7.5](2026-10-30-audit/J-tests-gate.md#a75-the-minimum-test-set-that-proves-the-seven-points-end-to-end))
that no PR schedules. Until N5 exists, `capture_crash_campaign.rs` is the only test of `kill -9`
and a torn tail across real processes.

Overlaps: K13 holds the termination proof and part of K18 (status signing) and the CLI half of the
spawn gate; K15 starts from the platform left after K9; K19's `doctor` shrink excludes the decoder
check counted in K1.

## 5. Change list

| Id | What | Why (point) | Size | Risk |
|---|---|---|---|---|
| CH1 | Raise our own bounds to values that state their cost, in `crates/goalcup-protocol/src/capture.rs`: `MAX_OPEN_STREAMS` 128 to 16,384 (`:58`; each open stream holds at most 16 KiB of undecided masking bytes, `crates/goalcup-daemon/src/redact/stream.rs:122-125`, so 256 MiB worst case, near zero for a quiet JSONL file); `MAX_OPEN_CONNECTIONS` 128 to 1,024 (`:60`); `MAX_CAPTURE_QUEUE_BYTES` 8 MiB to 64 MiB (`:51`), with the proxy `CONTROL_RESERVE_BYTES` 512 KiB to 4 MiB (`crates/goalcup-cli/src/proxy/ingest.rs:113-116`, 4 KiB per connection as today, still under the `/8` assert at `:128`). `MAX_LIVE_PRODUCERS` stays 128: the compile check at `crates/goalcup-daemon/src/runtime/capture/sink.rs:18` allows at most 256, and producers are per proxy epoch, not per subagent. The verifier's `MAX_GOAL_STREAMS` (`crates/goalcup-verifier/src/capture_analysis/reader.rs:78`) follows `MAX_PRODUCERS`, as its comment says it should. Every user moves in the same PR: proxy `api.rs:451, 599`, `mod.rs:1463`; daemon `sink.rs:386, 425`, `ingest.rs:1051, 1335`, `runtime/capture/writer.rs:252`, `redact/stream.rs:2769`; verifier `reader.rs:1384, 1566, 1694` (`session.rs` is gone with K1). | 2, 3 | about 30 src, about 500 test lines rewritten | Medium. If the reader does not move with it, large runs read as invalid (exit 2). Worst-case memory is about 256 MiB masking plus 64 MiB queue on each side; the realistic figure is measured in N1. An append-only stream keeps its slot for the whole part (`runtime/capture/tests.rs:2236-2239`), so a part with more than 16,384 subagent files still waits; closing quiet streams is after Edition 01, and the 2-hour run projects the 168-hour peak. |
| CH2 | Delete the native share and file caps: `max_open_streams: MAX_OPEN_STREAMS / 4` and `max_files: 1024` (`crates/goalcup-daemon/src/native_capture.rs:279-280`), and `MAX_NATIVE_FILES` 20,000 (`:187`). A native stream holds no file handle between polls (`:76-81, 1415-1460`). The 32-handle budget (`handle_budget.rs`, from K9) goes with it. | 2 | about -40 src, about 250 test lines rewritten | Medium. CPU grows with files: CH3 ships close behind. |
| CH3 | Cheap native poll: idle poll every 1 s, not 100 ms (`runtime/capture/owner.rs:16`); open each parent folder once per pass (`native_capture.rs:1376-1430`). A poll with deferred work repeats at once (`owner.rs:186`). | 3 | about 40 src | Low. Some tests wait longer for bytes. A fast writer may lag at 256 KiB per poll (`native_plan.rs:11`); measure. |
| CH4 | A stream or connection past the bound is refused alone with the existing one-reason loss. Never latch `Disabled(LimitExceeded)` for the segment (`crates/goalcup-cli/src/proxy/api.rs:451-453, 599-601`). Naming the host in the loss is a record change and waits for after Edition 01. | 1, 3 | about 4 src, 1 test | Low. |
| CH5 | Raise the open-file soft limit to the hard limit (macOS clamped to 10,240, Linux to 65,536) at the top of `main`, before the daemon starts; drop `OPEN_FILES_WANTED` (`commands/start/proxy.rs:27, 33-63`; today raised late at `commands/start/phases.rs:516`). | 2, 3 | about 10 src | Low. A harness tool using `select(2)` above 1,024 handles may break (rare, unverified). |
| CH6 | The proxy never refuses: delete `MAX_RECORDER_RESTARTS` (`proxy/mod.rs:1022`, checked `:1079-1098`) and restart every time after 1 s, logging once a minute; delete the 503 past `MAX_CONNECTION_TASKS` (`:112`), keep the 10 ms `EMFILE` retry; delete `MAX_CONNECTION_FAILURES` (`proxy/ingest.rs:104`) and retry the daemon every 10 s for ever. A restart reuses the producer state (`proxy/mod.rs:1049-1060`), so it spends no producers. | 3 | about -60 src net, about 220 test lines rewritten | Low. A loop that dies on every accept restarts once a second (noise, logged). |
| CH8 | Masking option b, after AD1: drop the `email` rule (`crates/goalcup-daemon/src/redact.rs:284-288`, its recognizer in `redact/stream.rs`, `spikes/redact.py:69`); drop the four Codex rate-limit rows (`connectors/codex-cli/harness.toml:110-113`); keep the three id rows; fix the form test (`redact/stream.rs:3538-3546`); regenerate pins. | 4 (and 1: the email rule masked bytes in subagent files) | about -100 src, about -90 tests | Low. Emails, plan type and credit balance now enter the private trace: AD8 says so. |
| CH9 | On `StorageFull`, cut the chain back to the last whole record, then latch (`writer.rs:154, 216, 221`; `tracecap.rs:673-689`). The trace is then "incomplete", not "invalid". | 3, 5 | about 10 src, 1 test | Low. Must use the writer's known record length. |
| CH10 | Prune: delete a closed part only after the server acked it, never the last part; drop the 2-hour timer and the disk-pressure branch (`crates/goalcup-cli/src/upload/prune.rs:47-82`). | 5 (a week of chain must fit on a laptop) | about -60 src, about -100 tests | Low. No local copy after the ack (Q9), so the 2-hour run compares against its own copies (section 9). |
| CH11 | Passed-through hosts: keep one row per connection (host, port, bytes, times), which cheat check C5 needs ([L](2026-10-30-audit/L-red-team.md), C5). Past the 10,000-row bound (`proxy/tunnels.rs:20`), keep one row per new host and port with a count, so every host is named. The bound stays, and says why: it keeps the list under the 8 MiB the Worker accepts (`server/worker/src/runs.ts:27`, client `upload/uploader/finish.rs:257`). The format line `goalcup-tunnels-v1` does not change, so the Worker, which reads only that line (`runs.ts:73-86`), needs no change. Finish line: "Passed through unread: host:port, n connections." | 1, 5, 6 | about 30 src | Low. A crash before finish still loses the list, as today; a chained record waits for after Edition 01. |
| CH12 | Hermes final snapshot: remove the 64 MiB cap `NATIVE_FINISH_BYTES` (`owner.rs:13`); keep the 180 s finish budget (`crates/goalcup-protocol/src/control.rs:708`). Best effort: the masked copy may not open as SQLite (AD8). | 2, 5 | 1 line | Low. A long seal on a huge `state.db`. |
| CH13 | `goalcup setup` warns instead of failing on an unsupported filesystem (`commands/prep.rs:1104-1111`); `start` and `continue` already only warn (`folder_gate.rs:10-12`). Edit `docs/RULES.md:385` in the same PR. (The provider probe is deleted in K7.) | 3, 7 | small saving | Low. A run on btrfs (Fedora's default) is untested. |
| CH14 | Worker for Friday, configuration only: `MIN_COMPETITION_VERSION` to the Friday tag (`server/worker/wrangler.toml:55, 150`); `EVENT_ID` switch (`:42`); a comment on `RUN_MAX_COMPRESSED_BYTES` (`:59`) naming the storage cost. No Worker code changes before 7 November: it already accepts null vitals and a null `repo_head` (`runs.ts:452-453, 507`). Keep `model_requests` in the heartbeat (`runs.ts:458`): it is the server's only call signal. | 5 | config | Medium. Deploy order; the owner deploys from a local device. No website SQL change. |
| CH15 | Connector data: Claude Code keeps `DISABLE_AUTOUPDATER` and adds `DISABLE_UPDATES = "1"` and `CLAUDE_CODE_DISABLE_AGENT_VIEW = "1"` (`connectors/claude-code-cli/harness.toml:60`, [D](2026-10-30-audit/D-harnesses.md)); Codex unchanged, plus one test that its `-c` launch arguments stay, since they already force the embedded app server (`connectors/codex-cli/harness.toml:45-49`); Hermes rows for 0.22.5 only, dropping the 0.19.0 rows (`connectors/hermes-cli/harness.toml:54-57, 88-92, 103-109`) if Q4 is yes. | 1, 2, 7 | about 6 data lines, 1 test, pins | Medium. Participants lose agent view (AD8 says so). |
| CH16 | Gate flakes: fix a flake only when it fails a PR this week. The outage-window flake goes with K11. Sweep the port races and sub-second bounds ([J B4](2026-10-30-audit/J-tests-gate.md#b4-known-flaky-tests-and-root-causes)) after Edition 01. | trust in the gate (no point) | small | Low. |

CH7 (one proxy worker thread per core) is withdrawn for Friday: no measurement shows proxy latency.
The 2-hour run records per-call latency; change it after Edition 01 if needed.

## 6. Add list

Only what the seven points need, in the smallest version.

| Id | Add | Point | Smallest version |
|---|---|---|---|
| AD1 | URL password rule, before CH8 drops the email rule. Today only the email rule masks `https://user:TOKEN@host` (`redact.rs:280-283`). | 4 | Whole-match form from [G.1](2026-10-30-audit/G-secrets.md#g1-add-the-url-password-rule-then-drop-the-email-rule-points-4-1-3-5-g-code-g-1-g-2-g-surface-g1), in `redact.rs` SHARED, `stream.rs` SHAPES and `spikes/redact.py`. Test N3: percent-encoded form, a `git remote -v` line, split at every byte, a plain email left clear. About 61 src, 40 tests. |
| AD2 | One "Recorded:" line at the end of each part and in `goalcup status`: model calls with hosts, session files read (main agent and subagents), files not read yet, gaps, hosts passed through unread, upload state. Say "so far" while recording. | 6, 2 | Read existing sources: `model_calls` (`run/mod.rs:266`), gaps, CH11 rows. Count files with an opened stream, not `tracked_files()` (`native_capture.rs:740-742`, which includes waiting files); print waiting files as "not read yet: n". Replace "Recording works". About 30 src, 30 tests. |
| AD3 | "Recording has stopped" when the daemon does not answer for 3 watch ticks, or the proxy sink is disabled. Written once to `host.log` and `state.json`; `status` shows it: "exit the agent and run `goalcup continue`". Also one `PROBLEM:` line when `open_files` passes half the soft limit (from the kept process vitals). | 6, 1, 2, 3 | About 35 src, 40 tests ([F12](2026-10-30-audit/F-week-long-risks.md#f12-the-daemon-crashes-mid-run-change-highest-risk)). The full fix (restart the daemon) waits for K13. |
| AD4 | `live-check` after seal, called by `matrix.sh`: wire call count against the harness's own count; subagent files on disk against native streams by path; a secret scan of the local chain copy and the fetched server copy (raw, base64, percent-encoded); the Recorded line against these counts; a run label from stream metadata only (model requests on recorded paths, native activity), with the bypass hint when there is native activity and no wire stream. | 1, 2, 4, 6 (proof); event rules | About 220 src, 110 tests. Replaces `--research-json`. |
| AD5 | Routes we cannot see, for Codex: the host reader also reads `openai_base_url` and `[model_providers.openai] base_url`; refuse Codex built-ins with no table (`ollama`, `lmstudio`, `amazon-bedrock`, `gcp`), since they record nothing. Claude Code cloud switches are named in AD8 only. | 1, 6 | One PR in `crates/goalcup-cli/src/model_host.rs:334-405, 546-553`. About 40 src, 40 tests. |
| AD6 | Tests N1 and N2. N1: 2,000 session files, 400 growing at once, under `ulimit -n 256`; every byte recorded or a named gap per file; memory and CPU printed. N2: the recorder dies ten times; the port is served again each time and calls succeed. | 2, 3 | About 210 test lines, N1 in a serial group. N1 ships with CH1 and CH2 (PR8a), N2 with CH6 (PR7). |
| AD7 | Server rebuild proof N4: real client, real Worker under `wrangler dev`, Worker stopped and started mid-run, then `fetch-run.sh`; the rebuilt chain equals a copy of the local chain taken before prune; every call and file path present in ID and time order. Remove `#[ignore]` from `upload_worker.rs` (`scripts/gate.sh:90-93` already runs it by name). `fetch-run.sh` writes what arrived and names a missing range (`scripts/fetch-run.sh:218-219`): that source change lands in PR10, before the feature stop. | 5 | About 10 src, 300 tests. |
| AD8 | Participant text (docs and start screen): a full disk ends recording; emails, plan type and credit fields stay in the private trace; local models, Claude Code cloud switches (`CLAUDE_CODE_USE_BEDROCK`, `_VERTEX`, `_FOUNDRY`, `_MANTLE`) and unlisted providers are not recorded; Claude Code agent view is off during a run; Hermes 0.19 is not supported, Hermes subagents are read at the end of a part, and the `state.db` copy holds earlier sessions of the shared home and may not open as SQLite; run in `tmux`; upload within 24 hours of the deadline; run labels are computed after the run; `goalcup mcp` lines in harness configs may stay, the tool keeps working. | 6 | Text only, in the PR that changes the behaviour. |

Decided not to add for Friday: closing quiet append-only streams (after Edition 01, unless the
2-hour run projects more than 8,000 open streams in 168 hours), joined-view masking, periodic SQLite
snapshots, the system trust store, a daemon restart with a fresh producer, chained records for
outages and tunnels, a host name in loss records, a Worker flag for "native activity but zero wire
calls", OpenCode or OpenClaw connectors, a longer start retry.

## 7. PRs in order

Each PR runs `sh scripts/gate.sh fast`. "Alone" means it can merge to main by itself. Every client change
ships in the one Friday release. The Friday Worker deploy is configuration only.

| PR | Scope | Rough size | Tests that prove it | Merge or deploy |
|---|---|---|---|---|
| PR1 | Docs first: README promise list R1 to R7 (the seven points) replaces P1 to P7 and the never-delete rule (`README.md:276-288`, `AGENTS.md:101`); AGENTS hard rule on D65 (`AGENTS.md:84-86`); a "Decided" list in VISION; one line atop the phase 2 notes: "the decisions in the progress file win". Fix false lines: `README.md:150-151` and `docs/ARCHITECTURE.md:214` (headers), `docs/ARCHITECTURE.md:253-256` (proxy restart "not built"), `docs/HARNESS-SETUP.md:194-195` (Hermes 0.21.1), `docs/RULES.md:83-84` ("the verifier reports it": no such check), `docs/CHEAT-CHECKLIST.md:26-28` (hourly verify job), `docs/RULES.md:294-296` | about 160 doc lines | link check | Alone. Needs Q1. |
| PR2 | K2, K6; CI drops the scope step and script tests; `gate.sh:61` and `release.sh:101-104` drop the version check | -5,430 src, -4,821 tests, -10,211 data | remaining script tests; redaction corpus test | Alone |
| PR3 | K1, AD4; edits to the four kept tests named in K1; RULES and CHEAT-CHECKLIST label lines reworded | -9,816 src, -16,775 tests; +220 / +110 | `live-check` on an existing recorded trace gives the same call count and label as `--research-json` before the delete | Alone |
| PR4 | K4, K26 | -2,276 tests | gate fast | Alone |
| PR8a | CH1, CH2 without the handle budget, N1 | about +30 src net, about 750 test lines rewritten | N1 on Linux and on a Mac with `ulimit -n 256`; capture integrity tests; memory at 5,000 and 16,000 synthetic open streams | Protocol, daemon, proxy and verifier together; sets the release floor for `MIN_COMPETITION_VERSION`. Compiles alone, so it is the fallback if PR8b slips. |
| PR5 | AD1, CH8, AD8 masking lines | about -40 src, net -50 tests | redaction suites, N3, `recognizers_hold_every_prefix_of_a_match`, oracle agreement | Alone; one commit (see section 8) |
| PR6 | K5 (client only, process vitals kept), CH9, CH10, docs retiring D65 (disk text in 19 places, `docs/RULES.md` disk lines) | about -1,500 src, -1,900 tests | writer `StorageFull` test, prune tests, vitals tests | Client only. The Worker already accepts null disk fields. |
| PR7 | CH5, CH6, K11, N2 | about -500 src, -300 tests | proxy integration, N2 | Alone |
| PR8b | CH3, CH4, CH12, K12, handle budget (K9 part); `CONTROL_API_VERSION` bump if status shapes change | about -150 src, -300 tests | capture integrity tests; CPU at 5,000 and 16,000 synthetic files; N1 again | Daemon and proxy. If it is not green by Monday night, ship without it: PR8a already holds the raised bounds. |
| PR9 | K7 (probe included, sniffing kept), CH13, K8, rest of K9, AD2, AD3, `docs/RULES.md:366, 385` | about -950 src, -1,000 tests | `start_launch`, `m2_cli_lifecycle`, `matrix.sh` wording (`READY_MARK` at `participant.rs:76` unchanged) | Alone |
| PR10 | CH11, K10, `fetch-run.sh` partial rebuild (AD7 source part) | about -200 src, -380 tests | tunnel inline tests, `test_fetch_run.py` | Client only; format line unchanged |
| PR11 | AD5, CH15, pins regenerated, HARNESS-SETUP and RULES lines | about +40 src, +41 tests | `model_host_from.rs`, `shipped_connectors.rs`, kit cases, Codex `-c` test | Alone; connector and pins in one commit |
| PR12 | AD7 tests (N4), `#[ignore]` removed | +300 tests | N4 in the full tier | Tests only, allowed after the feature stop |
| PR13 | Pure deletes of history docs (section 11) | about -5,000 doc lines | link check | Docs only |

**Day by day.**

- Thu 29 Oct: owner answers section 14. PR1.
- Fri 30 Oct: PR2, PR3. If a 2-hour run happens on today's main, its open-file, stream and bytes-per-hour
  numbers check the CH1 values. Its report (`docs/research/results/2026-10-30-two-hour-run.md`) does not
  exist yet; nothing in this plan comes from it.
- Sat 31 Oct: PR4, PR8a, PR5.
- Sun 1 Nov: PR6, PR7.
- Mon 2 Nov: PR8b, PR9.
- Tue 3 Nov: PR10, PR11. Feature stop at 4 Nov 00:00 UTC. After PR8b merges, branch `decoders-ed01`
  from main with PR3 reverted; CI builds it once.
- Wed 4 Nov: PR12. Tag the release candidate. Full gate on the Linux sandbox and on a Mac. Owner
  deploys the Worker configuration to staging. The 2-hour run on the candidate (section 9).
- Thu 5 Nov: buffer. Fix only what the run found. PR13. Re-run the kit on the newest harness builds,
  with `linux-harness-smoke.sh`. A 10-minute Amp chat run with one subagent, or the docs say "Amp
  unverified on build X".
- Fri 6 Nov: release at 12:00 UTC. Owner deploys the production Worker configuration with
  `MIN_COMPETITION_VERSION` and the `EVENT_ID` switch before 7 Nov 00:00 UTC. No client release after
  this until the last run ends, except owner-signed fixes; a connector update for harness drift counts
  as such a fix.

Risky work stays after Edition 01: K3, K13 to K27. The evidence does not justify an earlier daemon merge:
no measured run covers a merged build ([C 2.2](2026-10-30-audit/C-module-audits.md#22-should-the-daemon-be-a-separate-process-at-all)).

## 8. What must deploy together

| Change | Parts that move together | Rule |
|---|---|---|
| Bounds (PR8a) | protocol `capture.rs`; proxy `ingest.rs:113-116`, `api.rs:451, 599`, `mod.rs:1463`; daemon `sink.rs`, `ingest.rs:1335`, `writer.rs:252`, `redact/stream.rs:122-125, 2769`, `native_capture.rs:187, 279-280`; verifier `reader.rs:78, 1566, 1694`; their tests | One release. `MIN_COMPETITION_VERSION` set to it, or an old verifier calls new traces invalid. |
| `CONTROL_API_VERSION` | daemon, CLI, CLI mock (`crates/goalcup-cli/src/daemon/mock.rs:157-168`) | Bump once, in the Friday release. The installer ties CLI and daemon versions (`packaging/install.sh:104-195`). |
| Disk cut (PR6) | client heartbeat, docs | Client only. The Worker accepts null disk fields (`runs.ts:507`). |
| Tunnel rows (PR10) | client `tunnels.rs`, `fetch-run.sh` | Client only, while the format line stays `goalcup-tunnels-v1`. |
| Masking (PR5) | `redact.rs`, `redact/stream.rs`, `spikes/redact.py`, Codex `harness.toml`, pins, RULES | One commit: if SHARED and SHAPES differ, `policy_ready` refuses and every producer is lost (`redact/stream.rs:754-772`). |
| Connector edits (PR11, PR5) | `harness.toml`, `sh scripts/pin-manifests.sh` | Same commit, or every start refuses. |
| Judging decoders | branch `decoders-ed01` = main after PR8b, with PR3 reverted | Built once. Used by organizers after the event, never shipped to participants. |
| Friday release | client tag, production Worker configuration, `MIN_COMPETITION_VERSION`, `EVENT_ID` (`wrangler.toml:42, 55`) | Same day, before 7 Nov 00:00 UTC. No Worker code change. |
| Website SQL | none for Friday | Later changes to seal order or the close window need a website migration. |

## 9. What to test in the next 2-hour run

The report of the planned 30 October run (`docs/research/results/2026-10-30-two-hour-run.md`) does not
exist on any branch. This run is on the release candidate, Wednesday 4 November. Save its report as
`docs/research/results/2026-11-04-two-hour-run.md`, and save the nextest JUnit output on any failure.

**How to run it, the way people use harnesses.** Vercel Sandbox on a plan whose session lasts past 2
hours (Hobby stops at 45 minutes, `docs/research/VERCEL-COMPAT.md:41`), Linux, a normal user (uid 1000,
not root: Claude Code refuses bypass mode as root, and the gate refuses root). Credential brokering off
for model hosts, or every model call fails (`docs/research/VERCEL-COMPAT.md:46`). A real terminal in
`tmux`. Real logins. The newest stable builds of Claude Code, Codex (ChatGPT login, so the WebSocket path)
and Hermes 0.22.5; Amp if time allows. Chat mode, not one-shot. The prompt asks for subagents, for
example "Build the app below. Split the work across as many parallel subagents as help, and keep going
until it is done." Never force subagents by config. Record `ulimit -n` (soft and hard), `uname -r`, the
filesystem type of `$HOME`, and whether a TTY exists. Every 10 minutes, `cp -a` the trace folder to a
folder outside the spool: prune and `clean_finished` delete acked parts
(`crates/goalcup-cli/src/upload/prune.rs:20-22`), so the copies are what the rebuild is compared with.

| Point | Check | Pass |
|---|---|---|
| 1 | `live-check` wire call count against the harness's own count; passed-through host list; Codex WebSocket reconnects | Counts equal, or each difference named. No model-looking host in the passed-through list. Each reconnect is a clean new exchange. |
| 2 | Subagent files on disk against chain paths; open native streams per hour; files "not read yet"; gaps | Every subagent file is in the chain by path. Claude Code passes 32 open native streams at least once. No `LimitExceeded` gap. The open-stream count per hour, projected to 168 hours, stays under half of 16,384, or closing quiet streams moves up. |
| 3 | 503s, refused model connections, `QueueFull` gaps, recorder kills, daemon kill, CPU, memory, open files, per-call latency through the proxy | No 503 and no refused model connection. No `QueueFull` gap. Kill the recorder twice: the harness carries on, a gap is recorded, the port serves again. Kill the daemon once: "Recording has stopped" shows within 2 minutes. Daemon CPU stays under a tenth of a core when idle (target, not yet measured). Open files stay well under the soft limit. Latency is recorded for the CH7 decision. A sleep-lock `PROBLEM:` line in the sandbox is expected and harmless (`docs/research/VERCEL-COMPAT.md:44`); note it. |
| 4 | A fake `.env` the agent prints, a URL with a password, a fake key the model echoes | All masked in the copied local chain and in the fetched server copy, in raw, base64 and percent-encoded forms. The echo split across deltas is a known gap: record the result, do not fail the run on it. |
| 5 | `fetch-run.sh` rebuild; `goalcup-verify --capture-integrity` on the server copy; bytes per hour | Rebuild equals the copied chain. Verify exits 0. Every segment is in order by ID and time. Compressed bytes per hour times 168 is under 64 GiB. |
| 6 | The Recorded line and passed-through lines on screen | They match `live-check`. No internal words. |
| 7 | One manual network cut of 2 minutes; harness auto-update off | Recording continues; upload catches up; no harness update during the run. |

## 10. Tests and gate after the cuts

**Stays.** Redaction suites and the Python oracle (point 4). Capture core: `capture_native.rs`,
`capture_ingest.rs`, `capture_lifecycle.rs`, with cap rows rewritten (points 1 to 3). Proxy relay tests
for TLS, HTTP and WebSocket. `m2_synthetic_vertical.rs`, `capture_crash_campaign.rs` and
`capture_failure_campaign.rs` (until N5). The Rust Worker stand-in and `upload_e2e.rs` (until K3).
`upload_worker.rs`, now without `#[ignore]`. Worker tests, always in the full tier. `chain_golden.rs`,
`m2_contract.rs`, `no_family_literals.rs`, `shipped_connectors.rs`, termination tests (until K13),
installer tests, `test_gate.py`, `test_release.py`, `test_linux_check.py`, `test_codeowners.py`.

**Goes before Friday.** The test lists in K1, K2, K4 to K12 and K26, about 28,000 lines, plus N1 and
N2 rewrites of the cap and restart rows.

**Target gate time.** Fast gate under 4 minutes warm on 4 vCPU; today it ranges from 54 s quiet to
306 s loaded ([J B3](2026-10-30-audit/J-tests-gate.md#b3-time-per-step-and-total)). Full gate under
12 minutes; it has never been timed, so time it once on Wednesday.

**Flakes.** Fix one only when it fails a PR this week (CH16). The open outage-window flake goes with
K11. Keep the `connector-runs` serial group. Put N1 in a serial group too: it opens hundreds of files.

**Known limits.** CI still excludes the daemon and CLI suites (`.github/workflows/ci.yml:15-16`), so the
hand-run `scripts/gate.sh full` is the gate. The gate refuses root (`scripts/gate.sh:32-39`).

## 11. Docs and decisions

From [K](2026-10-30-audit/K-docs-decisions.md#8-order-of-work): 82 Markdown files and 17,472 lines today;
target about 31 files and 7,000 to 7,500 lines, reached after Edition 01.

- **Before Friday, in PR1:** the promise list, the "Decided" list in VISION, AGENTS fixes, the false
  statements listed in PR1, the line atop the phase 2 notes.
- **Before Friday, inside each code PR:** the doc lines for that change (disk text in 19 places with PR6;
  cap text with PR8a; masking text with PR5; label text with PR3; RULES machine and disk lines with PR9).
- **Thu 5 Nov, PR13, pure deletes only:** `docs/archive/` (958 lines); research history (AMP-FOLLOWER,
  HOW-A-RUN-ENDS, LOCAL-CAMPAIGN, LOCAL-NEXT-STEPS, LOCAL-TEST-DAY, PARTICIPANT-REVIEW,
  RELEASE-IS-THE-LIST, SIMPLIFICATION, t25-disk-full with one line kept in ARCHITECTURE); ten fix plans
  and old results; done plans (18, 19, 20, 23, 28 and its spec and ledger); `RESEARCH-TARGET.md`,
  `LOCAL-RESEARCH-HANDOFF.md`. Keep `VERCEL-COMPAT.md`: the 2-hour run uses it.
- **After Edition 01:** merge CONTRACTS and TERMINATION into ARCHITECTURE, CHEAT-CHECKLIST into
  SECURITY, SERVER-DESIGN and UPLOAD-API into the Worker README, NOTIFICATIONS and RESTARTS-DESIGN into
  PARTICIPANT-GUIDE, CONNECTOR-SCHEMA into HARNESS-ONBOARDING. Retire D7, D12, D23, D40 reserve clause,
  D44, D48, D49, D50, D54 freeze, D58 (replace with "no caps of our own; any bound states its cost"),
  D65 (its text is fixed in PR6), D68, D69, D73 0.7.0 branches, and re-cite the about 90 code-comment
  cites. Delete the phase 2 notes, spec, plan and status once their binding lines have moved.
- **Never rename:** `docs/PARTICIPANT-GUIDE.md` (`crates/goalcup-cli/src/main.rs:59`) and the
  `### <Harness>` anchors in `docs/HARNESS-SETUP.md` (`crates/goalcup-cli/src/commands/prep.rs:896`).

## 12. Risks and blind spots of this plan

- **No long-run data.** The 2-hour run report does not exist. Peak streams, files, open handles, chain
  size per hour, memory and CPU are all estimates. The CH1 values and the CH3 poll interval are guesses
  until Wednesday.
- **Append-only streams pile up.** A stream for a subagent transcript keeps its slot for the whole part.
  At 16,384 this is far above "thousands of subagents", but it is still a wall for a very busy part. The
  2-hour run projects the peak; past it, new files wait and the Recorded line says "not read yet".
- **macOS is not tested in the sandbox.** The sandbox is Linux. The 256-handle default only shows on a Mac:
  N1 must run on a Mac with `ulimit -n 256`.
- **Estimates.** Mixed-file line counts are plus or minus 30 percent; 30 to 40 percent of source lines
  are doc comments.
- **Removing caps moves the limit to memory and handles.** Worst case about 256 MiB of held masking
  bytes and two 64 MiB queues. Process vitals stay so `status` and the `PROBLEM:` line show a leak.
- **PR8a and PR8b.** The bounds land Saturday so the Wednesday run tests them. If PR8b is late, the
  release ships with PR8a alone; poll CPU then stays at today's 100 ms interval.
- **Decoder cut.** Judges lose decoded reports in the shipped verifier. They use branch `decoders-ed01`
  after the event. Lane L says no event-time cheat control needs them ([L top findings](2026-10-30-audit/L-red-team.md#top-findings)).
- **Secrets still leak in two known ways:** a key the model echoes across deltas, and `cat .env` lines the
  rules miss (`ENCRYPTION_KEY=`, short passwords). Both are in the private raw trace under a 180-day lock.
- **Silent bypass.** A harness or tool that ignores `HTTPS_PROXY`, or a settings `env` that overrides it,
  gives a sealed run with no gap. AD4's bypass hint, AD5 and AD8 shrink this; they do not close it.
- **A dead `goalcup start` still blocks the harness** until `goalcup continue` (L point B). Fixing it needs K13.
- **Worker deploy.** The owner deploys from a local device. Friday is configuration only; stage it on
  Wednesday.
- **Emails unverified.** Whether `RESEND_API_KEY` is set in production is unverified
  (`server/worker/wrangler.toml:71, 84`); if not, no participant email is sent.
- **Old promises.** Reviewers may block a cut using README P1 to P7 or the phase 2 notes. PR1 must land first.
- **This plan cuts working code days before an event.** Every Friday cut is a deletion with a clear owner
  decision or no caller; the riskier cuts wait.

## 13. Challenges and answers

A fact check found 23 errors, and two reviews challenged the plan from both sides: one said cuts
break a point, the other said the plan still does too much. Full text and an answer to every point:
[R-reviews.md](2026-10-30-audit/R-reviews.md). What changed:

- **CH1 did not compile.** `MAX_LIVE_PRODUCERS` stays 128 (`sink.rs:18`). Streams go to 16,384,
  connections to 1,024, the queue to 64 MiB with the control reserve scaled, and the verifier's goal
  stream bound follows. The bounds now land Saturday as PR8a, apart from the riskier poll work (PR8b).
- **Kept, not cut, for Friday:** process vitals (point 3 needs them once caps go), session sniffing on
  `continue`, the `goalcup mcp` command, the owed-ack path, `linux-harness-smoke.sh` and four script
  tests, `m2_contract.rs`, the Rust Worker stand-in (K3, now after Edition 01), `db-local.ts` (N4 needs it).
- **Cut more:** the start-time provider probe, `check-version.py` and the v5 corpus test.
- **Smaller changes:** no Codex `--no-daemon` flag, no Worker code before the event, CH4 without a new
  record shape, CH11 keeps per-connection rows for cheat check C5, CH7 withdrawn, flakes fixed only
  when they fail, doc merges and decision retirements moved after Edition 01, no Claude Code warning
  in AD5.
- **Added:** a metadata run label in AD4 and a `decoders-ed01` branch for judges, a "not read yet" count
  in AD2, an open-files `PROBLEM:` line in AD3, copies of the trace in the 2-hour run, Vercel session and
  brokering setup lines, more participant notices in AD8, connector fixes allowed during the freeze.
- **Rejected:** cutting the sleep lock, the synced-folder refusal and `no_family_literals.rs`; moving
  status signing into PR8; raising the head bound (past it the proxy relays bytes unparsed,
  `proxy/http.rs:287, 489`); a longer start retry; the daemon-side per-stream change (the proxy and
  the native reader refuse first).

## 14. Owner questions

| # | Question | Recommendation |
|---|---|---|
| Q1 | Replace README P1 to P7 and the never-delete rule with the seven points as R1 to R7, so cut PRs may delete the tests they make moot, and let run labels be computed after the run (decoders kept on branch `decoders-ed01`)? | Yes |
| Q2 | Set open streams to 16,384 and connections to 1,024 per entry, the capture queue to 64 MiB, each stated as a memory guard, keep 128 live producers, delete the native 32-stream, 1,024-file and 20,000-file caps, and leave closing quiet streams for after Edition 01? | Yes |
| Q3 | Keep `RUN_MAX_COMPRESSED_BYTES` at 64 GiB, with one line saying it guards storage cost (about $1 a month per run at the cap)? | Yes |
| Q4 | Drop Hermes 0.19.0 support and pin Hermes 0.22.5, and keep the final `state.db` copy as best effort (it may not open as SQLite and holds earlier sessions of the shared home)? | Yes |
| Q5 | Leave the daemon merge, termination proof, signing, connector pins, deny lists, the Rust Worker stand-in and the event-rule commands (`login`, SSH key warnings, push lines, client repo check, run give-back) until after Edition 01? | Yes |
| Q6 | Stop printing the `goalcup mcp` setup lines now, keep the command working through Edition 01, and delete it after? | Yes |
| Q7 | Add `DISABLE_UPDATES = "1"` and `CLAUDE_CODE_DISABLE_AGENT_VIEW = "1"` to the Claude Code connector, keeping `DISABLE_AUTOUPDATER`? | Yes |
| Q8 | Refuse at start Codex built-in providers with no table (`ollama`, `lmstudio`, `amazon-bedrock`, `gcp`), and name Claude Code cloud switches in the docs only? | Yes |
| Q9 | Keep a simple local prune: delete a part only after the server acked it, never the last one, with no timer and no disk trigger? | Yes |
| Q10 | Freeze client releases from the Friday release until the last run ends, except owner-signed fixes, counting connector updates for harness drift as such fixes, with `MIN_COMPETITION_VERSION` set to that release? | Yes |

## 15. Appendix index

- [A-inventory.md](2026-10-30-audit/A-inventory.md): every line counted by area and by point; caps and disk code listed.
- [B-fresh-design.md](2026-10-30-audit/B-fresh-design.md): two fresh designs from the seven points, and how the code compares.
- [C-module-audits.md](2026-10-30-audit/C-module-audits.md): keep, cut or change for each of eleven areas, with couplings.
- [D-harnesses.md](2026-10-30-audit/D-harnesses.md): Claude Code, Codex, Hermes and others as people run them today.
- [E-providers.md](2026-10-30-audit/E-providers.md): providers, gateways and clouds, and the routes the proxy cannot see.
- [F-week-long-risks.md](2026-10-30-audit/F-week-long-risks.md): what breaks over 168 hours on a real machine.
- [G-secrets.md](2026-10-30-audit/G-secrets.md): every masking rule under option b, and the real leak paths.
- [H-participant.md](2026-10-30-audit/H-participant.md): the participant's steps and every message they see.
- [I-server-upload.md](2026-10-30-audit/I-server-upload.md): the Worker and upload path, and the smallest server for point 5.
- [J-tests-gate.md](2026-10-30-audit/J-tests-gate.md): the test map, the gate, gate times and flakes.
- [K-docs-decisions.md](2026-10-30-audit/K-docs-decisions.md): every doc and decision, with what to delete, merge or retire.
- [L-red-team.md](2026-10-30-audit/L-red-team.md): unrecordable runs, day-3 breakage and cheats.
- [R-reviews.md](2026-10-30-audit/R-reviews.md): the fact check, two adversarial reviews, and the answer to every point.
