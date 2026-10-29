# Reviews of the cut-first plan: fact check, two reviews, answers

Date: Thursday 29 October 2026. Plan: [2026-10-30-cut-first-plan.md](../2026-10-30-cut-first-plan.md).
Base: main at 4b226de2 (the audit branch at a2c9ac20 adds only docs).

This file holds three inputs and the answer to each point in them:

1. A fact check of the first draft of the plan (23 corrections).
2. Review 1, which argued that some cuts break a point, an event rule or the Friday date.
3. Review 2, which argued that the plan is still over-engineered.

Each input is copied in full, with its headings moved down two levels. After each input comes a
table with one row per point: **accept** (the plan changed), **partly accept** (the plan changed in a
smaller or different way), or **reject** (no change), with the reason and the evidence. Every answer
was checked in the code or the appendices; anything not checked says "unverified". Nothing was built
or run. The 2-hour run report (`docs/research/results/2026-10-30-two-hour-run.md`) does not exist on
any branch, so no answer rests on it.

The plan's section 13, "Challenges and answers", sums up what changed.

## 1. The fact check, in full

### Fact check of docs/plans/2026-10-30-cut-first-plan.md

Checked against the code at HEAD a2c9ac20 (main 4b226de2 plus the audit docs) and appendices A to L.
Format: claim | what is true | evidence. Most serious first.

#### Corrections

1. **CH1 / Q2: set `MAX_LIVE_PRODUCERS` to 16,384, "256 MiB worst case".**
   True: this will not compile, and the worst-case memory is far above 256 MiB.
   - `sink.rs:18` has a compile-time check: `MAX_LIVE_PRODUCERS * CONTROL_ITEM_COST <= CONTROL_RESERVE_BYTES`, which is 512 B times n against 128 KiB. So n can be at most 256.
   - The ingest worst case is `MAX_LIVE_PRODUCERS x MAX_INGEST_FRAME_BYTES` (512 KiB). At 16,384 that is 8 GiB.
   - The proxy's `CONTROL_RESERVE_BYTES` (512 KiB) is sized for a `Recovery` that re-observes all 128 connections. At 16,384 connections it does not fit in the 8 MiB `MAX_CAPTURE_QUEUE_BYTES`.
   - C says producers are per proxy epoch, not per subagent, so keep the 128 live-producer bound.
   Fix: drop `MAX_LIVE_PRODUCERS` from CH1 and Q2. Resize both control reserves for the new connection bound, and state that memory.
   Evidence: `crates/goalcup-daemon/src/runtime/capture/sink.rs:7,16,18`; `crates/goalcup-daemon/src/ingest.rs:90-96`; `crates/goalcup-protocol/src/capture.rs:49,51`; `crates/goalcup-cli/src/proxy/ingest.rs:113-116,122`; `C-module-audits.md:155,541`.

2. **CH1 and section 8 file lists ("move the daemon `sink.rs:183, 313, 322` ... verifier `reader.rs:1567, 1695`, `session.rs:644, 761`").**
   True:
   - `session.rs` is deleted by K1 in PR3, before PR8.
   - `sink.rs:183,313` check `MAX_PRODUCERS` (65,536), which CH1 does not change.
   - The list misses other users of the bounds: `redact/stream.rs:2769` (the masker refuses a stream past `MAX_OPEN_STREAMS`), `runtime/capture/writer.rs:252`, `sink.rs:386,425`, `ingest.rs:1051`, `proxy/mod.rs:1463` and `proxy/api.rs:599`.
   - The reader lines are 1566 and 1694.
   Evidence: `rg MAX_OPEN_STREAMS|MAX_OPEN_CONNECTIONS|MAX_LIVE_PRODUCERS crates/*/src`.

3. **K3: delete `tests/common/upload_server/` (2,514). Tests that go: "all 65 stand-in tests".**
   True:
   - Ten other test files import the stand-in: `full/upload_e2e.rs` (2,518), `it/start_launch.rs`, `it/restarts.rs`, `it/model_host_from.rs`, `it/account.rs`, `it/hermes_offline_key.rs`, `it/upload_chunks.rs`, `it/prep.rs`, `it/upload_open.rs`, and `full/t25_disk_full.rs` (that last one is cut in K5).
   - C says keep the stand-in as a fake and trim it by about 500 lines.
   - So either K3 is 2,909 test lines (only the 3 `upload_server*.rs` files), or porting those files to `wrangler dev` is extra work with no estimate.
   Evidence: `grep -rn common::upload_server crates/goalcup-cli/tests`; `C-module-audits.md:1331-1333`.

4. **K1 "Tests that go" is incomplete.**
   True: the decoder registry is also used by:
   - the inline test `every_hinted_path_is_read_by_its_decoder` (`crates/goalcup-cli/src/model_host.rs:1218-1270`; C names it);
   - kept tests `tests/it/harness_contract_kit.rs:20-22,511-513`, `tests/it/connector_contract.rs:36-37,937-944` and `tests/full/connector_contract.rs:381-406`.
   PR3 must edit these.
   Evidence: `C-module-audits.md:1729,1767`.

5. **K4: delete the `frozen-connectors` and `schema-1-connectors` fixtures. Tests that go: "the files listed".**
   True: kept tests read these fixtures:
   - `frozen-connectors`: `goalcup-protocol/tests/it/harness_loader.rs:483`, `harness_conformance.rs:220`, `goalcup-daemon/src/runtime/capture/tests.rs:1694`, `goalcup-daemon/tests/common/support/control.rs:481`.
   - `schema-1-connectors`: `goalcup-cli/tests/common/support/mod.rs:92`, used 11 times in `it/connector_contract.rs` and 4 times in `full/connector_contract.rs`.
   - The fixture count also leaves out `frozen-connectors` (237 lines): the true total is 2,701, not 2,464.

6. **K2: "scripts that serve no point". The only follow-up edit named is the ci.yml scope step.**
   True:
   - `linux-harness-smoke.sh` is run by the kept `scripts/linux-check.sh:100-104` and by `.github/workflows/linux-x86_64.yml:63`.
   - Four of the tests that go test kept code: `test_gate.py` (gate.sh), `test_release.py` (release.sh), `test_linux_check.py` (linux-check.sh) and `test_codeowners.py`. Lane J says keep `test_gate`, `test_release` and `test_codeowners` (`J-tests-gate.md:706,709,714`); C says cut them. The plan does not say it chose C.
   - `campaign.example.json` (64) is counted as src, but A.1 counts it as data.

7. **K6 / PR2: "about 1,044 prototype" lines "elsewhere in spikes/", and PR2 "-10,600 data".**
   True:
   - The 1,044 includes cursor (198) and openclaw (203), which sit inside the folders K6 already deletes.
   - The prototype code outside those folders is 643 (hermes 374, opencode 269).
   - So PR2 removes about 10,211 data lines; 10,600 counts about 400 twice.
   - Note: after the cut, 73 `.jsonl` corpus files remain, above the test's floor of 50, so the corpus test holds.
   Evidence: `A-inventory.md:1344-1352`.

8. **K7 and CH13 claim the same lines.**
   True:
   - K7 cuts `commands/start/phases.rs:107-142` and counts it as savings. CH13 keeps that same probe block (`:107-140`) as a warning. PR7 and PR9 both edit it.
   - CH13's "filesystem check (`commands/prep.rs:413, 416`)" points at the save-before-machine-step lines. The check itself is `prep.rs:1104-1111` (`check_filesystems`, then `State::Fail`).

9. **Timing and count claims in the opening summary.**
   True:
   - "Every cut lands by Tuesday 3 November" is false: K3 lands in PR12 on Wednesday 4 November, and the PR13 doc cuts land Thursday.
   - "Add only seven small things" is false: the add list has eight (AD1 to AD8).

10. **PR1 "fix false lines".**
    True: K lists three false statements:
    - "no header is recorded" (`README.md:150-151`, `docs/ARCHITECTURE.md:214`);
    - "automatic recording proxy restart: not built" (`docs/ARCHITECTURE.md:253-256`);
    - "Hermes 0.21.1 and later: model calls only" (`docs/HARNESS-SETUP.md:194-195`).
    PR1 leaves out the last two. ARCHITECTURE line 213 is "named in host.log"; the header line is 214.
    Evidence: `K-docs-decisions.md:38-41,525-527`.

11. **Section 2 table: "existing relays already decode all encodings (`README.md:150`)".**
    True: `README.md:150` says bytes are relayed unchanged. Decoding is described in `docs/ARCHITECTURE.md:211-213`, and it covers only one gzip, deflate, brotli or zstd coding plus permessage-deflate. Any other coding is an `UnsupportedEncoding` gap.

12. **K1 risk: "`scripts/matrix.sh:2057-2064, 2247-2269` counts calls with `--research-json`".**
    True: lines 2247-2269 are `verify_trace` with `--capture-integrity`. Only `:2051-2064` (and the comment at `:32`) use `--research-json`. The wrong range is copied from `A-inventory.md:154,923`.

13. **K10 risk: "the Worker accepts a null `repo_head` (`runs.ts:461-462`)".**
    True: it is `server/worker/src/runs.ts:452-453` (and `:472`). Lines 461-462 are `requireAccepting`.

14. **CH11: "no 10,000-row cap", and the Worker "stops parsing" the list.**
    True:
    - Two 8 MiB caps on the tunnel list remain: client `MAX_TUNNELS_BYTES` (`upload/uploader/finish.rs:257`) and Worker `TUNNELS_MAX` (`runs.ts:27`). The plan does not mention them.
    - The Worker reads only the first line (format and `entry_id`, `runs.ts:73-86`). If the `goalcup-tunnels-v1` format line stays, no Worker change and no deploy order are needed. "Worker first" applies only if the format changes.
    - Unstated risk: L's cheat check C5 uses per-connection host, port, bytes and times (`L-red-team.md:437`). One row per host with first and last seen loses that.

15. **CH8, CH15 and section 8 (masking and connector rows): "regenerate pins and goldens".**
    True: K4 (PR4) deletes the goldens before PR5 and PR11. Only the pins remain to regenerate.

16. **K9: `handle_budget.rs` "goes with CH1".**
    True: it is the native 32-handle budget. It is used by `goalcup-platform/src/sourcecap.rs:3,139,152`, `sourcecap/unix.rs:2,131` and `goalcup-daemon/src/native_capture.rs:576`. So it pairs with CH2, and removing it edits `sourcecap`, which K9 says stays.

17. **K11: "about 370" src.**
    True: the named ranges add up to about 522: `outage.rs` 144, `mod.rs` 274 + 49 + 30, `connect.rs` 25. Either say which parts of `mod.rs:647-920` stay, or raise the figure. C gives about 200 for the outage part (`C-module-audits.md:545`).

18. **K20: about 1,030 src and about 1,100 tests.**
    True: `db-local.ts` 701 + `github.ts` 223 = 924 src; `local-db.test.ts` 590 + `github.test.ts` 184 = 774 tests. The plan names nothing else.

19. **AD7 / K3: `upload_worker.rs` "leaves `#[ignore]` and runs in the full tier".**
    True: `gate full` already runs it by name (`scripts/gate.sh:90-93`). The only change is removing `#[ignore]`.

20. **Section 2: "Both land near 2,000 client source lines, against about 102,000 today".**
    True: 102,176 is all production source, including the Worker (4,395) and scripts (11,176). B compares its 2,500 to 3,800 total lines with 102,000 (`B-fresh-design.md:15-16`); the client crates are 85,800.
    Also: "Both designers say so" (the Go row) is wrong: B item 10 quotes only B1 (`B-fresh-design.md:33-35`).

21. **AD1: "60 tests".**
    True: G says about +40 tests for 61 src, or +80 tests for the larger version (`G-secrets.md:247`).

22. **CH15: `DISABLE_UPDATES` "in place of" `DISABLE_AUTOUPDATER`.**
    True: D says keep `DISABLE_AUTOUPDATER` and add `DISABLE_UPDATES` (`D-harnesses.md:399`). D:299 says `DISABLE_UPDATES` blocks every update path, so replacing it is plausible, but it differs from the appendix.

23. **Bounds the owner asked to raise but the plan leaves out.**
    True: the verifier's `MAX_GOAL_STREAMS = 128` (`crates/goalcup-verifier/src/capture_analysis/reader.rs:78,1384`) is flagged in `K-docs-decisions.md:242` and is not in CH1 or section 8. It may not matter for subagents (unverified), but say so.

#### Checked and right

- **A.1 totals:** 102,176 src and 159,498 tests. Per-area "after" sums: 82,840 / 124,200 / 57,260 / 86,790. Cut sums: K1-K12 = 19,336 src and 35,183 tests (plan says about 35,300); K13-K25 = 25,570 src and 37,394 tests.
- **Percents:** 19, 22 and 44 percent are right. The adds are about 381 src and 790 tests.
- **Line counts with `wc -l`:**
  - K2 scripts 5,530 and tests 5,262 (1,107 = 226 + 180 + 107 + 56 + 538).
  - K4 tests 3,273.
  - K3 stand-in 2,514 + 1,419 + 529 + 961, and 65 tests (34 + 12 + 19).
  - K1 src 9,745 (10,858 raw minus 1,113 inline); `doctor.rs:246-316` is 71 lines.
  - K5 sizes: `reserve.rs` 533/423, `pause.rs` 211/149, `vitals.rs` 158/51, `disk.rs:58-239`, `t25` 93 and 1,173, platform `disk.rs` test 137.
  - `mcp.rs` 273/201, `outage.rs` 144/172, later-cut test files (`terminate_sim.rs` 2,714, `process_kill.rs` 1,285 and the others), `ratelimit` 139/184.
- **Constants and cites:** `capture.rs:58,60,66`; `native_capture.rs:187,279-280`; `owner.rs:13,16`; `native_plan.rs:11`; `proxy/mod.rs:112,1022`; `proxy/ingest.rs:104`; `start/proxy.rs:27,98`; `phases.rs:516`; `tunnels.rs:20`; `redact/stream.rs:122-125,754-772,3538-3546`; Codex `harness.toml:110-113`; `redact.py:69`; `control.rs:349,708`; `run/mod.rs:266`; `wrangler.toml:42,55,59,150`; `runs.ts:458`; `ci.yml:15-16`; `gate.sh:32-39`; `TODO.md:42`; `README.md:276-288`; `AGENTS.md:84-86,101`.
- **Deploy pairing:**
  - PR8: the bounds live in protocol, daemon, proxy and verifier, and one release covers them. The Worker does not hold them.
  - Signing: K18 correctly names Worker, client, verifier, protocol and corpus.
  - PR6: Worker first is safe.
- **Style:** no em dash, no model id (the only regex hit was "boto3"). All 14 appendix anchors resolve.

### 1a. Answers to the fact check

| # | Claim | Answer | What the plan does now | Evidence checked |
|---|---|---|---|---|
| 1 | CH1 sets `MAX_LIVE_PRODUCERS` to 16,384; "256 MiB worst case" | Accept | `MAX_LIVE_PRODUCERS` stays 128. Streams go to 16,384, connections to 1,024, `MAX_CAPTURE_QUEUE_BYTES` to 64 MiB and the proxy `CONTROL_RESERVE_BYTES` to 4 MiB (4 KiB per connection, as today). The memory line now names the masking bytes and both queues. | `crates/goalcup-daemon/src/runtime/capture/sink.rs:7, 16, 18`; `crates/goalcup-cli/src/proxy/ingest.rs:113-116, 128`; `crates/goalcup-protocol/src/capture.rs:51, 58, 60, 66` |
| 2 | CH1 and section 8 file lists are wrong or short | Accept | The lists now name `api.rs:451, 599`, `mod.rs:1463`, `sink.rs:386, 425`, `ingest.rs:1051, 1335`, `writer.rs:252`, `redact/stream.rs:2769`, `reader.rs:78, 1384, 1566, 1694`, and drop `session.rs` (gone with K1) and `sink.rs:183, 313` (they check `MAX_PRODUCERS`). | `rg MAX_OPEN_STREAMS\|MAX_OPEN_CONNECTIONS\|MAX_LIVE_PRODUCERS crates/*/src` |
| 3 | K3 breaks ten more test files | Accept | K3 moves to after Edition 01 (both reviews asked for this too). | `grep -rn common::upload_server crates/goalcup-cli/tests` |
| 4 | K1 test list is incomplete | Accept | K1 names the four kept tests PR3 must edit. | `crates/goalcup-cli/src/model_host.rs:1218-1270`; `C-module-audits.md:1729, 1767` |
| 5 | K4 deletes fixtures that kept tests read | Accept | K4 keeps `frozen-connectors` and `schema-1-connectors`. It deletes only the connector goldens and the `harness-golden` fixtures (2,110 lines). | the kept readers listed in item 5 |
| 6 | K2 deletes a script and tests that guard kept code | Accept | `linux-harness-smoke.sh`, `test_gate.py`, `test_release.py`, `test_linux_check.py` and `test_codeowners.py` stay. `campaign.example.json` is counted as data. | `scripts/linux-check.sh:99-104`; `.github/workflows/linux-x86_64.yml:63`; `J-tests-gate.md:706, 709, 714` |
| 7 | K6 prototype lines and PR2 data lines are double-counted | Accept | K6 says 643 prototype lines; PR2 says 10,211 data lines. | `A-inventory.md:1344-1352` |
| 8 | K7 and CH13 claim the same probe lines; CH13 cites the wrong filesystem lines | Accept | The probe is deleted in K7 (see review 2, A1). CH13 now cites `prep.rs:1104-1111`, the real `check_filesystems` call. | `crates/goalcup-cli/src/commands/prep.rs:1104-1111` |
| 9 | "Every cut lands by Tuesday" and "seven adds" are false | Accept | The summary says every code cut lands by Tuesday; Wednesday adds tests only; Thursday has doc deletes; the adds are eight. | plan sections 6 and 7 |
| 10 | PR1 misses two false statements; wrong ARCHITECTURE line | Accept | PR1 lists `ARCHITECTURE.md:214`, `:253-256` and `HARNESS-SETUP.md:194-195`, plus `RULES.md:83-84` from review 1. | `K-docs-decisions.md:38-41, 525-527` |
| 11 | "Existing relays already decode all encodings (`README.md:150`)" | Accept | The row now cites `docs/ARCHITECTURE.md:211-213`: one coding plus permessage-deflate; others give an `UnsupportedEncoding` gap. | `docs/ARCHITECTURE.md:211-213` |
| 12 | Wrong `matrix.sh` range | Accept | K1 cites `scripts/matrix.sh:2051-2064`. | `scripts/matrix.sh:2051-2064, 2247-2269` |
| 13 | Wrong `runs.ts` line for null `repo_head` | Accept | K10 cites `runs.ts:452-453, 472`. | `server/worker/src/runs.ts:452-453` |
| 14 | CH11 misses two 8 MiB caps; the Worker reads only the first line; one row per host loses cheat check C5 | Accept | CH11 keeps per-connection rows, keeps the format line (so no Worker change), and states why the 10,000-row and 8 MiB bounds stay. Past 10,000 rows, new hosts still get a row with a count. | `crates/goalcup-cli/src/proxy/tunnels.rs:17-20`; `upload/uploader/finish.rs:257`; `server/worker/src/runs.ts:27, 73-86`; `L-red-team.md:287` |
| 15 | Goldens are gone before PR5 and PR11, so only pins need regenerating | Accept | CH8, CH15 and section 8 say "pins" only. | plan section 7 order |
| 16 | `handle_budget.rs` pairs with CH2, not CH1, and edits `sourcecap` | Accept | K9 and CH2 say so; it lands in PR8b. | `crates/goalcup-platform/src/sourcecap.rs:3, 139, 152`; `crates/goalcup-daemon/src/native_capture.rs:576` |
| 17 | K11 is about 522 src, not 370 | Accept | K11 says about 520, and notes that C counts about 200 for the outage part alone. | `C-module-audits.md:545` |
| 18 | K20 sizes are 924 / 774 | Accept, then superseded | K20 now cuts only `github.ts` (223 / 184): `db-local.ts` stays because N4 needs it (review 2). | `crates/goalcup-cli/tests/full/upload_worker.rs:50` |
| 19 | `upload_worker.rs` already runs in the full tier | Accept | AD7 says the only gate change is removing `#[ignore]`. | `scripts/gate.sh:90-93` |
| 20 | "2,000 client lines against 102,000"; "both designers say" Go | Accept | Section 2 says 2,500 to 3,800 lines in all against about 102,000 (85,800 in the client crates); the Go row cites B1 only. | `B-fresh-design.md:15-16, 33-35` |
| 21 | AD1 is about 40 test lines | Accept | AD1 says 40. | `G-secrets.md:247` |
| 22 | CH15 replaces `DISABLE_AUTOUPDATER`; D says keep it | Accept | CH15 keeps it and adds `DISABLE_UPDATES`. One extra line costs nothing and follows the appendix. | `D-harnesses.md:299, 399` |
| 23 | The verifier's `MAX_GOAL_STREAMS` (128) is left out | Accept | CH1 sets it to follow `MAX_PRODUCERS`. Its own comment says it was meant to track `MAX_PRODUCERS` (`reader.rs:76-77`). Whether each recovered epoch writes a goal stream is unverified; the change is one line either way. | `crates/goalcup-verifier/src/capture_analysis/reader.rs:76-78, 1384` |

The "checked and right" list in the fact check needs no answer. Its K1 to K12 sums no longer apply,
because the cut list changed; section 3 of the plan has the new figures.

## 2. Review 1 (cuts break a point), in full

### Review 1 (adversarial): where the cut-first plan breaks a point, an event rule or the Friday date

Plan: `docs/plans/2026-10-30-cut-first-plan.md` at HEAD a2c9ac20. Read with `parts/check.md` and digests A to L.
Read only. Nothing built or run. I do not repeat the 23 corrections in `check.md`; I build on them where they
matter. Every claim below was checked in the code or docs unless marked "unverified".
The 2-hour run report (`docs/research/results/2026-10-30-two-hour-run.md`) does not exist, so no finding
below rests on it.

#### Summary

Most deletions are safe. The serious problems are in five places:

1. **K1 deletes the run labels and the bypass hint that the event rules and the cheat checklist rely on.**
   Also, "tag the last commit with decoders" does not work, because that tag still has the 128 caps.
2. **CH1 and CH2 leave subagent transcript streams open for the whole part.** So any fixed bound is reached
   on a busy week-long run, and from then on every new subagent file waits unrecorded. PR8's fallback
   does not hold together.
3. **K5 deletes the only telemetry for open files, CPU and memory (`vitals.rs`)** in the same week the plan
   moves the limits to memory and file handles. It also does this before the Wednesday run that has to
   measure them.
4. **Three changes land too late or with no gain, and add risk:** K3 on release-candidate day, the Codex
   `--no-daemon` flag (CH15), and the session sniffing cut in K7 on the `continue` recovery path.
5. **The point 5 check in section 9 and AD7 cannot pass as written.** Prune and `clean_finished` delete the
   local parts they compare against.

#### 1. Challenged cuts and changes (most serious first)

| Id | Point or rule broken | Failure on a real run | Evidence | What the plan should do instead |
|---|---|---|---|---|
| K1 | Event rules (RULES "What a label is worth"; CHEAT-CHECKLIST bounds); points 1 and 5 at judging | A Claude Code run on macOS. The participant puts a gateway in the workspace `.claude/settings.json`. Every call tunnels sealed. The checklist says the bound is "the run's label reads `native_only`, and `analysis.cross_leg` notes `native_activity_without_wire_streams`". After K1, no shipped tool computes either. A `none` run "does not count" (RULES), but nothing says which runs are `none`. The fallback is "tag the last commit with decoders". That tag predates PR8, so its reader returns `open_stream_limit_exceeded` / `connection_limit_exceeded` (exit 2) on any Friday trace with more than 128 open streams. That is the very subagent-heavy runs the cuts are for. | Labels and the bypass hint are only in `crates/goalcup-verifier/src/capture_analysis/report.rs:375-404`, which K1 deletes. `docs/RULES.md:306-321`. `docs/CHEAT-CHECKLIST.md:46-48, 94-97, 133-136, 144` cite them as the bound. Reader caps: `reader.rs:1566, 1694` (and `MAX_GOAL_STREAMS` `:78`). PR3 (K1) is Fri 30 Oct, PR8 is Mon 2 Nov (plan section 7). | Keep the cut, but add two conditions: (a) after PR8 merges, create branch `decoders-ed01` from main and revert PR3 on it, so the judging decoders read Friday traces with the new bounds; CI builds that branch once. (b) PR1 or PR3 rewords `RULES.md:306-321` and the four checklist bounds: "labels are computed after the run by the organizer tool". If the owner will not keep a branch, defer K1 until after Edition 01. It is verifier-only code, so it saves the participant nothing on Friday. |
| CH1, CH2 (with PR8 fallback) | Points 2 and 3 | Claude Code, Linux sandbox, day 4. The prompt asked for parallel subagents. Each subagent transcript is append-only, and **an append-only file keeps its stream open across polls for the rest of the part**. Open native streams only grow. Raising 128 to 16,384 (which does not compile, `check.md` item 1) only delays the wall. Once it is reached, each new subagent file waits (`waiting_files += 1`) and records nothing for the rest of the part. Tool-result snapshot files end, so they do not pile up; subagent transcripts do. Also, PR8's fallback, "keep the old bounds raised only in the protocol and verifier", is not possible. The daemon's native share (`MAX_OPEN_STREAMS / 4`) and the masker (`redact/stream.rs:2769`) read the same protocol constant. And CH5 and CH6 are in PR7, not PR8. | `crates/goalcup-daemon/src/native_capture.rs:44-47` (append-only stream kept open); `:1555-1556` (waits when full); `:279` (`/4`); `crates/goalcup-protocol/src/capture.rs:58, 60, 66`; `sink.rs:18` compile check. | (a) Split PR8. PR8a (Sat 31 Oct): a tested constant raise that compiles today. `MAX_OPEN_STREAMS` and `MAX_OPEN_CONNECTIONS` to 1,024 (memory about 16 MiB of held masking bytes), live producers stay 128, native share removed, `max_files` to the 20,000 ceiling. That shape goes into the Wednesday run. PR8b: CH4 unlatch, poll change, handle budget. (b) Add one small change: end a caught-up append-only stream after about 10 quiet minutes, and open a new generation when the file grows. The generation code already exists for replaced files (`native_capture.rs:56-60`). About 40 src. Or at least measure the open-stream growth rate in the Wednesday run and print the projected 168-hour peak against the bound. |
| K5 (the `vitals.rs` part) | Point 3 (seeing blocking before it happens) | Codex, macOS, day 2. Caps are gone (CH1, CH2, CH6), so the next limit is file handles and memory. Today each heartbeat carries `open_files`, CPU and memory for `goalcup` and the daemon, and `goalcup status` shows them. K5 deletes all of it. A handle leak climbing toward 10,240 is then invisible until accepts fail with `EMFILE`. It also lands in PR6 (31 Oct / 1 Nov), before the Wednesday run whose section 9 row 3 needs "CPU, open files". | `crates/goalcup-cli/src/vitals.rs:1-6, 150`; `upload/api.rs:234`. Plan section 9 row 3 and section 12 ("removing caps moves the limit to memory and CPU"). | Smaller cut: delete only `disk_free` and the disk notice. Keep process vitals (`open_files`, CPU, memory), about 100 lines. Add one `PROBLEM:` line when `open_files` passes half the soft limit (lane C asked for it). |
| K3 | Friday date (gate on RC day); point 5 coverage | Wed 4 Nov is the release-candidate day and the day of the 2-hour run. Deleting the stand-in that day breaks ten more test files, `full/upload_e2e.rs` (2,518) among them (`check.md` item 3). It also makes the full gate depend on Node and `wrangler dev` the same morning. A red gate on RC day pushes the tag into Thursday's buffer. K3 removes no runtime code. | `check.md` item 3; `scripts/gate.sh:54-58, 90-93`; feature stop "fixes only" `TODO.md:196-200`. | Defer K3 until after Edition 01. Keep AD7: remove `#[ignore]` from `upload_worker.rs` and add N4 next to the stand-in. |
| K2 (`linux-harness-smoke.sh`, `test_gate.py`, `test_release.py`, `test_codeowners.py`) | Point 7; release process | Thu 5 Nov the plan re-runs on the newest harness builds. `linux-harness-smoke.sh` is the only check that needs no login: it starts the newest Codex and Claude Code builds through `goalcup` on Linux and runs `goalcup-verify` on the trace. `.github/workflows/linux-x86_64.yml:63` runs it, so that workflow breaks. Separately, every release must pass `gate.sh full` and `linux-check.sh` (`TODO.md:199-200`). Deleting the tests of `gate.sh` and `release.sh` a week before the release removes the guard on the two scripts that cut it. | `scripts/linux-check.sh:99-104`; `.github/workflows/linux-x86_64.yml:63`; `TODO.md:199-200`; lane J keep list (`check.md` item 6). | Keep `linux-harness-smoke.sh` (242) and the workflow for Friday. Keep `test_gate.py`, `test_release.py` and `test_codeowners.py` (lane J). The other K2 cuts are safe. |
| CH15 (Codex `--no-daemon`) | Points 3 and 7 | A participant has Codex 0.153 from Homebrew. Codex rejects the unknown flag and exits, so `goalcup start` launches nothing. Today's `-c` launch arguments already force the embedded app server, so the flag adds no recording. It is also unverified whether `codex resume --last` accepts the flag on `continue`. | `parts/D-codex.md:183-188, 283` (`--no-daemon` from 0.156.1; `-c` already forces embedded); `connectors/codex-cli/harness.toml:45-52`. | Do not add `--no-daemon`. Add one test asserting that the `-c` arguments stay. Cut before you add. |
| K7 (session sniffing on `continue`) | Points 3 and 7 (the recovery path for a week-long run) | Claude Code, day 3. The terminal closed before the first message of the new part. `goalcup continue` now always asks for `--continue` or `resume --last`. The harness finds no session and exits at once. Only on a second `goalcup continue` does the `resume_failed` path open a new session. The cited range `phases.rs:296-361` also contains `started_at_unix`, `model_calls_before` (needed by AD2) and `resume_failed`, which must stay. | `crates/goalcup-cli/src/commands/start/phases.rs:296-361`; `crates/goalcup-cli/src/launch.rs:664-699`. | Keep session sniffing for Friday (about 130 src). Cut only the probe test seams and the legacy leftovers in K7. |
| K8 (`goalcup mcp`) | Point 3 (noise), point 6 | Dry runs have been open since 23 Oct. Participants pasted `"<path>/goalcup" mcp` into `.claude.json`, Codex `config.toml`, Hermes `config.yaml` and Amp settings, as `goalcup setup` told them to. After K8 every harness start launches a server that exits with an unknown-subcommand error. Claude Code shows a failed MCP server; Codex and Hermes behaviour on a failed server is unverified. | `crates/goalcup-cli/src/commands/prep.rs:733-760`; `connectors/*/harness.toml` `[participant.time_left_tool]`. | Keep a stub `goalcup mcp` (about 20 lines). It answers `initialize`, lists no tools and logs "use `goalcup time-left`". Or have `setup` and `start` print "remove these lines". AD8 must say it. |
| K4 (`m2_contract.rs`) | Point 5 across a mid-cup fix release | K4 cites "the owner dropped the byte-for-byte rule". That decision is about subagent files against disk (`docs/plans/2026-10-29-record-everything-progress.md:93`), not the record format. During the cup, owner-signed fix releases are allowed. A fix that changes record bytes by accident makes Friday traces unreadable by the new verifier, or the reverse. These goldens are the guard for that. | `crates/goalcup-protocol/tests/it/m2_contract.rs:1-4`; plan section 7 Friday and Q10. | Delete the connector JSON goldens and the frozen fixtures, after fixing their readers (`check.md` item 5). Keep `m2_contract.rs` and regenerate it once at the Friday tag, as the frozen-for-the-cup format. |
| Section 9 row 5 and AD7 | Point 5 proof | The run checks "Rebuild equals the local chain". But chain parts close every 30 minutes, prune deletes acked parts (with CH10, at once), and after a finished upload `clean_finished` deletes every part but the last. So the local chain is gone when the comparison runs. | `crates/goalcup-protocol/src/export.rs:62-64`; `crates/goalcup-cli/src/upload/prune.rs:20-22, 120`; `commands/start/finish.rs:290`. | In the run script, copy the trace folder every 10 minutes with `cp -a` to a folder outside the spool, and compare against that copy. In N4, use a test-support switch that keeps the parts. |
| AD2 | Point 6 | The "Recorded:" line takes session files from `native_files_admitted`, which is `tracked_files()`: every file in the map, including files waiting for a stream and files that could not be read. With a full stream table it says "37 session files" when 5 were never read. A wrong count is worse than none (lane H). | `native_capture.rs:740-742` (`self.files.len()`); `:1555-1556`. | Count files with an opened stream, and print waiting files as "not read yet: n". |
| CH4 | Points 1 and 3 (size and scope) | The plan says "a chained loss naming the connection and host, about 10 src". But `Loss` carries only a reason, and losses are deduplicated per reason per emitter. Naming the connection and host is a protocol record change, with verifier and corpus edits, inside PR8. | `crates/goalcup-cli/src/proxy/api.rs:451-453, 672, 675`. | Smallest version for Friday: stop latching `Disabled` and keep the existing one-reason loss. Naming the host waits for after Edition 01. |
| CH11 | Cheat C5 (second harness, RULES:80-84) | Codex run. The agent starts Claude Code, whose calls tunnel to `api.anthropic.com`. With one row per host and only first and last seen, a judge can no longer line up each connection's time with the tool call that started the second harness. | `docs/plans/2026-10-30-audit/L-red-team.md:437` (C5 needs "host, port, bytes, times"); `docs/RULES.md:80-84`. | Keep per-connection rows for hosts that look like model hosts (lane E's suffix list), and aggregate the rest. This still removes the 10,000-row cap. |
| K10 (owed-ack path) | Point 5 when the disk is full | With K5, a full disk is "no special handling". The owed-ack path is what lets the uploader keep sending older chunks when `acks.jsonl` cannot be written. If the cut makes a local ack failure stop the upload or loop, then on a full disk the already-recorded tail never reaches the server. | `crates/goalcup-cli/src/upload/uploader/session.rs:228-253, 327-347, 483-496`. | State the rule in the PR: an ack that cannot be written is dropped (the code says dropping is always safe) and the upload goes on. Add one test on a read-only `acks.jsonl`. The `repo_head` part is safe for Friday, because `seal.json` keeps its Git fields until K21. |
| Q10 (client freeze) | Point 7; D70 | Claude Code ships about 6 builds a week and Amp about 50. If a new build moves subagent files or adds a host on 10 Nov, the fix is a connector change, and connectors are compiled into the client. A freeze with only "owner-signed fixes" blocks or delays it. | `docs/RULES.md:411` ("a merged connector ships in the next one"); `TODO.md:198-199`; `parts/digest-D.md:70`. | Name connector updates for harness drift as an allowed fix in Q10 and section 7. |
| CH13, K9 | Event rules text | CH13 makes the filesystem check a warning, but `RULES.md:385` says a run "needs a supported disk type". K9 cuts the AC power and container facts that `RULES.md:366` promises to record. Today `start` and `continue` already only warn (`folder_gate.rs:10-12`); only `setup` fails, for example on Fedora's default btrfs. | `docs/RULES.md:366, 385`; `crates/goalcup-cli/src/folder_gate.rs:10-12`; `crates/goalcup-platform/src/machine.rs:358-372`. | Keep both changes. Edit the two RULES lines in the same PRs. |

#### 2. Cuts and changes I checked and found safe

| Id | Why safe |
|---|---|
| K2 (rest) | `campaign.py`, the research scripts, `ci-scope.py`, `linux-matrix.sh` and `harnesses/install.sh` serve no point. The network-cut drill moves to the manual run. |
| K5 (reserve, pause, `ReserveFile`, Worker `disk_full`, t25) | The owner ruled on it. The `SIGSTOP` pause itself blocks the harness, so removing it helps point 3. The `StorageFull` gap stays, and CH9 makes the tail clean. Exception: keep `vitals.rs` (above). |
| K6 | Spike data only. 73 `.jsonl` files remain, above the corpus test's floor of 50 (`check.md` item 7). |
| K7 (probe seams, DNS loopback check, shared-home recheck, legacy leftovers) | The `flock` that serialises a shared home stays (`home.rs:79-95`). The recheck is a symlink-swap hardening with a named residual (D39). The local-host refusal stays in `capture_policy.rs`. |
| K8 (`git-credential` and the old token flow) | Only 0.6.1 test clones use it. |
| K9 | Dead code. It pairs with CH2, not CH1 (`check.md` item 16). |
| K11 | Forwarding is untouched. A count line replaces the windows. |
| K12 | The 1 TiB quota is never reached. |
| CH3 | A poll with deferred work repeats on the next pass (`owner.rs:186`), so the 1 s idle interval does not cap throughput when there is a backlog. |
| CH5, CH6 | They remove refusals that block the harness. The `EMFILE` retry stays. |
| CH7 | The runtime is already multi-thread with one worker (`commands/start/proxy.rs:96-100`), so the code is `Send` already; only the thread count changes. Measure added latency in the run. |
| CH8 plus AD1 | One commit with the oracle and `policy_ready`. The URL rule comes before the email rule is dropped. |
| CH9, CH10 | Small. Signed acks stay, so prune stays safe. |
| CH12 | It only removes a byte cap. The 180 s finish budget still ends the finish. Note that the shared Hermes home's `state.db` holds earlier practice sessions too (point 4). Say so in AD8. |
| CH14, CH16, AD1, AD3, AD5, AD6 | Small, tied to a point, and inside the feature stop. AD5's refusal matches the existing local-host refusal. |

#### 3. Additions the plan misses

| Add | Point | Smallest version |
|---|---|---|
| End quiet append-only native streams, and reopen them on growth | 2 | About 40 src, plus one N1 row. Without it, any stream bound is reached on a busy week. |
| Keep process vitals plus an `open_files` `PROBLEM:` line | 3, 6 | Keep about 100 lines of `vitals.rs` and add one line. |
| A `decoders-ed01` branch that tracks the PR8 reader | Event rules (labels, C1, C6) | One `git revert` on a branch, built once. |
| Participant notices missing from AD8: Claude Code agent view is off during a run; Hermes 0.19 is not supported; remove the `goalcup mcp` lines; labels come after the run | 6 | Text only. |
| PR1 false lines: `RULES.md:83-84` ("the verifier reports it": no such check exists, `L-red-team.md:287`) and the checklist "hourly verify job" (`CHEAT-CHECKLIST.md:26-28`, already in PR1) | Event rules | Text only. |
| An Amp check before Friday. The site promises Amp (`parts/digest-D.md:4-5`), yet the plan runs it only "if time allows" | 7 | A 10-minute Amp chat run with one subagent on Thursday's newest build, or "unverified on build X" in the docs. |
| Section 9 setup lines: Vercel credential brokering off for model hosts, and a copy loop for the trace folder | 5, 7 | Two lines in the run script (`docs/research/VERCEL-COMPAT.md:46`). |

#### 4. Timing

- PR8 is the riskiest PR, and it lands Monday with PR9 (K7, K8, K9, AD2, AD3). That is the heaviest day,
  one day before the feature stop. Splitting off PR8a (above) moves the part the Wednesday run needs to Saturday.
- AD7 changes `fetch-run.sh` (source) in PR12, after the feature stop. That counts as a fix only if it is
  framed as one. Better: move the `fetch-run.sh` part into PR10 (Tuesday).
- With K3 deferred, PR12 is N4 and CH16 only, and Wednesday is free for the release candidate and the run.

### 2a. Answers to review 1

**Section 1: challenged cuts and changes**

| # | Point | Answer | What the plan does now | Evidence checked |
|---|---|---|---|---|
| 1 | K1 deletes the run labels and the bypass hint; the old tag cannot read Friday traces | Partly accept | K1 stays a Friday cut, with three conditions. (a) AD4 prints a label from stream metadata and the bypass hint when there is native activity and no wire stream. (b) PR3 rewords `RULES.md:306-321` and the four checklist bounds: labels are computed after the run. (c) After PR8b, branch `decoders-ed01` from main with PR3 reverted, built once, so the judges' decoders read the new bounds. Q1 now asks this. Not deferred: the decoders are 9,816 src and 16,775 test lines that every PR this week would otherwise have to keep green, PR8a among them. | `crates/goalcup-verifier/src/capture_analysis/report.rs:375-404`; `docs/RULES.md:306-321`; `docs/CHEAT-CHECKLIST.md:46-48, 95-97, 135, 144` |
| 2 | CH1 and CH2: append-only streams stay open for the whole part, so any bound is reached; PR8's fallback does not hold | Partly accept | Confirmed: an append-only stream keeps its slot until the finish (`crates/goalcup-daemon/src/native_capture.rs:44-47`; `runtime/capture/tests.rs:2236-2239`, which also says ending idle streams is an owner decision). PR8 is split: PR8a (Saturday) holds only the constants, which compile, so it is the fallback; PR8b (Monday) holds the poll, CH4, CH12 and the handle budget. The stream bound is 16,384, not the 1,024 the review suggests: 1,024 is within reach of "thousands of subagents" in one part, and 16,384 costs at most 256 MiB of held masking bytes (`redact/stream.rs:122-125`), near zero for a quiet JSONL file. Closing quiet streams is not built for Friday: it is new logic in the generation code, the verifier's handling of a second generation that starts mid-file is unverified, and it would land in release week. The 2-hour run projects the 168-hour peak; if it passes half the bound, the change moves up. AD2 shows waiting files as "not read yet". | `native_capture.rs:44-47, 1555-1556`; `runtime/capture/tests.rs:2236-2239`; `redact/stream.rs:122-125, 2769` |
| 3 | K5 deletes process vitals just as limits move to handles and memory | Accept | K5 cuts only the disk half of `vitals.rs`. Process vitals stay, and AD3 adds a `PROBLEM:` line when `open_files` passes half the soft limit. | `crates/goalcup-cli/src/vitals.rs:1-6, 120-147` |
| 4 | K3 on release-candidate day breaks ten files and adds a Node dependency | Accept | K3 moves to after Edition 01. PR12 is N4 only. | fact check item 3; `scripts/gate.sh:90-93` |
| 5 | K2 deletes `linux-harness-smoke.sh` and tests of the release scripts | Accept | They stay. | `scripts/linux-check.sh:99-104`; `.github/workflows/linux-x86_64.yml:63` |
| 6 | Codex `--no-daemon` breaks older Codex and adds nothing | Accept | No flag. One test asserts that the `-c` launch arguments stay, since they already force the embedded app server. | `D-harnesses.md:649-654`; `connectors/codex-cli/harness.toml:45-49` |
| 7 | K7's sniffing cut breaks the `continue` recovery path | Accept | Session sniffing stays. K7 keeps `phases.rs:296-361` and `launch.rs:664-746`. | `crates/goalcup-cli/src/commands/start/phases.rs:296-361`; `launch.rs:664-699` |
| 8 | K8 leaves pasted `goalcup mcp` lines pointing at a missing command | Partly accept | No stub (new code). The `goalcup mcp` command stays as it is through Edition 01; only the setup lines and the four connector tables go, so nobody adds new lines. The command moves to K19. Q6 asks this. | `crates/goalcup-cli/src/commands/prep.rs:730-760` |
| 9 | K4's `m2_contract.rs` guards the record format across mid-cup fix releases | Accept | `m2_contract.rs` stays through the cup (K27 after). The owner's byte-for-byte decision is about subagent files against disk, not the record format. | `docs/plans/2026-10-29-record-everything-progress.md:93`; `crates/goalcup-protocol/tests/it/m2_contract.rs:1-4` |
| 10 | The rebuild check compares against local parts that prune deleted | Accept | The 2-hour run copies the trace folder every 10 minutes and compares against the copies. N4 compares against a copy the test takes before prune; no test switch is added. | `crates/goalcup-cli/src/upload/prune.rs:15-22`; `crates/goalcup-protocol/src/export.rs:62-64`; `commands/start/finish.rs:286-290` |
| 11 | AD2 counts waiting files as recorded | Accept | AD2 counts files with an opened stream and prints "not read yet: n". | `native_capture.rs:740-742` |
| 12 | CH4 naming the host is a record change | Accept | CH4 only stops the latch and keeps the one-reason loss. Naming the host waits. Lane L also sized the fix at about 1 line. | `crates/goalcup-cli/src/proxy/api.rs:451-453, 599-601`; `L-red-team.md:30-33` |
| 13 | CH11 one row per host loses cheat check C5 | Accept | Per-connection rows stay. Past 10,000 rows, a new host still gets one row with a count. | `L-red-team.md:287` |
| 14 | K10's owed-ack cut could stop upload on a full disk | Accept | The owed-ack path stays for Friday (K27 after). Keeping working code is cheaper than a new rule and a new test. | `crates/goalcup-cli/src/upload/uploader/session.rs:228-253, 327-347, 483-496` |
| 15 | Q10's freeze blocks connector fixes for harness drift | Accept | Q10 and section 7 count a connector update for harness drift as an allowed fix. | `docs/RULES.md:411` |
| 16 | CH13 and K9 contradict `RULES.md:366, 385` | Accept | Both lines are edited in PR9. | `docs/RULES.md:366, 385`; `crates/goalcup-cli/src/folder_gate.rs:10-12` |

**Section 2: cuts found safe.** No answer needed. The plan keeps them, with the `vitals.rs`
exception from row 3.

**Section 3: additions the plan misses**

| Add | Answer | Reason |
|---|---|---|
| End quiet append-only streams, reopen on growth | Partly accept | Not for Friday (row 2). The 2-hour run measures the growth rate; the plan names the threshold that moves it up. |
| Keep process vitals plus an `open_files` `PROBLEM:` line | Accept | In K5 and AD3. |
| `decoders-ed01` branch | Accept | In section 7 and section 8. |
| More participant notices | Accept | AD8 now names agent view off, Hermes 0.19 unsupported, the `goalcup mcp` lines, labels after the run and the Hermes `state.db` copy. |
| PR1 false lines `RULES.md:83-84` and the checklist hourly job | Accept | In PR1. `L-red-team.md:287` confirms there is no second-harness check. |
| Amp check before Friday | Partly accept | A 10-minute Amp chat run on Thursday if a login is at hand; otherwise the docs say "Amp unverified on build X". It is not a release blocker. |
| Section 9 setup lines | Accept | Brokering off and the copy loop are in section 9, plus the Vercel session length (Hobby stops at 45 minutes, `docs/research/VERCEL-COMPAT.md:41`). |

**Section 4: timing**

| Point | Answer | Reason |
|---|---|---|
| Split PR8 so the bounds land Saturday | Accept | PR8a on Saturday, PR8b on Monday. |
| Move the `fetch-run.sh` change into PR10 | Accept | PR10 holds it, before the feature stop. |
| With K3 deferred, PR12 is N4 and CH16 only | Accept | PR12 is N4 only; CH16 is now "fix on failure". |

## 3. Review 2 (still over-engineered), in full

### Review 2 (adversarial): is the cut-first plan still over-engineered?

Plan reviewed: `docs/plans/2026-10-30-cut-first-plan.md` (355 lines, HEAD of the audit branch).
Also read: `parts/check.md`, digests A to L, `docs/plans/2026-10-30-audit/B-fresh-design.md` (top findings,
B.6), and the code lines cited below. Read only. Nothing was built or run. The 2-hour run report
(`docs/research/results/2026-10-30-two-hour-run.md`) does not exist, so no finding here uses it.

#### Short verdict

The plan cuts the right big things (decoders, research scripts, disk handling, goldens) and its
deferrals of the daemon merge, signing and termination are fair for a release in eight days. It is
still over-engineered in four ways:

1. **It keeps checks and refusals that serve no point and were never put to the owner.** The
   start-time provider probe is turned into a warning instead of deleted. The sleep lock, the
   synced-folder refusal, the client run give-back, `check-version.py` and the v5 corpus test stay
   with no question in section 13.
2. **It does release-week work that no point needs before Friday.** Worker code edits (K5, CH11)
   that the Worker does not need, K3's stand-in delete on release-candidate day, CH7 with no
   measured need, a gate-flake sweep (CH16 names "trust in the gate", not a point), and a
   DECISIONS retirement that re-cites about 600 code-comment lines on the buffer day.
3. **Several caps of our own are still standing and are not listed.** The daemon sink drops a
   whole producer on one over-limit stream or a full 8 MiB queue. The 64 KiB head cap, the
   verifier's 128 goal streams, the 3 tries in 9 s at start and the 24 h close window are also
   still there.
4. **Some later cuts are pure test or text deletions that could land now and make PR8 safer.**
   These are the v5 corpus, `check-version.py` and the probe.

Taking the low-risk items below saves about **460 src and 1,000 test lines** more before Friday,
plus 912 fixture lines. The owner-call items add about **850 src and 940 tests**. It also takes the
Friday Worker deploy down to **config only**, and removes about 6,000 lines of release-week churn
(K3 port and doc re-cites) from Wednesday and Thursday.

#### 1. Kept parts, checks and refusals with no point behind them

| # | What | Path:lines | Point it fails | Cut or change instead | Lines saved (src / tests) | Risk |
|---|---|---|---|---|---|---|
| A1 | Provider-host TCP probe, kept as a warning (CH13) | `crates/goalcup-cli/src/proxy/preflight.rs:1-250` (src), `:251-420` (inline tests); called from `commands/start/refusals.rs:9`, `commands/start/phases.rs:107-140` | 3 and 6 are already met by the "no model calls" problem line the plan itself names (`participant/status.rs:467-509`). As a warning, the probe repeats that line 10 minutes earlier. It checks TCP only, not CA trust (`B-fresh-design.md` B.6.3 row 13), so it does not catch the failures B2 wanted a start check for. | Delete the probe and its call. This also settles the K7/CH13 clash on `phases.rs:107-142` (check.md item 8). Keep `resolves_only_to_loopback` only if anything still calls it after K7 cuts `model_host.rs:507-540` (today its one caller is `model_host.rs:523`). | about 250 / about 170 inline, plus probe rows in `preflight_doctor.rs` and `start_launch.rs:1026-1110` (already in K7) | Low. An offline start shows its problem after the first quiet interval, not at once. |
| A2 | Sleep lock and its `PROBLEM:` line | `crates/goalcup-platform/src/awake.rs` (269 src; 256 test lines in `tests/it/awake.rs`); `participant/status.rs:511-512` | None. A sleeping laptop pauses the harness and the recorder together, so nothing goes unrecorded (`B-fresh-design.md` B.6.2 item 7). F5 says the line likely shows in containers with no `systemd-inhibit`, which may include the Vercel Sandbox (unverified). That would be a false problem line, which hurts 6. | Owner question, missing from section 13. Smallest step: make the line a one-time start note (1 line). Full cut after Edition 01. | 269 / 256 (full cut) | Low for the record. The participant's score may suffer if a laptop sleeps. |
| A3 | `scripts/check-version.py` in the fast gate (`gate.sh:61`) and in `release.sh:101-104` | `scripts/check-version.py` (206), `scripts/test_check_version.py` (128) | None. It also reads `docs/CONTRACTS.md` and `docs/DECISIONS.md` (`check-version.py:27-28`), and PR13 merges CONTRACTS into ARCHITECTURE. That coupling is not named in the plan, so PR13 breaks the fast gate on Thursday. | Cut in PR2 (C, `C-module-audits` row 11). Edit `gate.sh:61` and `release.sh:101-104`. | 206 / 128 | Low. A second version string could slip in for one release. |
| A4 | Client run give-back and `pending-open.json` | `crates/goalcup-cli/src/upload/mod.rs:297-495` minus `keep_run` | None. Lane I calls it Fri and harmless: an empty open run stays until its deadline (`I-server-upload`, digest I row "Abandon and give-back"). | Owner question (C says it may leave stray `started` or `silence` emails). The client stops calling abandon, and the Worker route stays. | about 160 / about 300 (I's estimate) | Low to medium: stray emails. |
| A5 | Synced-folder refusal and `.gitignore` matcher | `crates/goalcup-cli/src/per_run_home.rs:318-489`; `commands/start/project.rs:225-330` | These are refusals that stop a start (they work against 3 and 7). Lane C lists them for Friday (C row 4 "Synced-folder refusal; gitignore matcher"). H's journey part keeps them for point 4. | Owner question, not in section 13. If kept, make each one a warning (H F4). | about 270 / about 130 (C) | Medium. A token in a synced folder reaches the participant's own sync service. |
| A6 | Event-rule commands with no owner question | `commands/run_repo.rs` (197, a client copy of the Worker's repo check `runs.ts:314-351`), `push_lines.rs` (152 src), `ssh_keys.rs` (192 src) | None (`B-fresh-design.md` B.6.2 item 9: about 2,280 src in all). A Q1 and B Q7 both ask this question. The plan asks only about `mcp` (Q6). | Add one owner question. Before Friday, the only safe step is making the client repo check a hint (about 45 src, lane I). The rest stays as K19, after Edition 01. | about 45 now | Low. The Worker's check stays and is the cheat control (L). |
| A7 | Hermes `state.db*` final copy through the redactor, with its cap lifted (CH12) | `connectors/hermes-cli/harness.toml:111-121`; `runtime/capture/owner.rs:13` | Point 2 is served by the wire: Hermes subagents' model calls cross the proxy. G says a masked span changes length, so the copy no longer opens as SQLite (`redact.rs:1201-1207`). The copy also holds every earlier session in the shared home, which works against 4. | Owner question (G Q4, L-M). Either drop the row (8 data lines, and CH12 is then not needed) or keep it and say it is best effort. | 8 data lines, plus Hermes goldens (unverified) | Medium. The Hermes native subagent tree is lost. |
| A8 | `no_family_literals.rs` kept "as a cheap guard of hard rules" | `crates/goalcup-protocol/tests/it/no_family_literals.rs` (378 tests) | None of the seven. C groups it with the D68 contributor guards, and the plan retires D68 (section 11). It imports `pins::shipped_connector_ids`, so it also ties K17 to the protocol. | Keep for Friday if the owner wants it, but say it guards a contributor rule. Or cut it with K4. | 0 / 378 | Low. A harness name creeps into core code. |
| A9 | Status signing kept until K13 or K18 | `crates/goalcup-protocol/src/control.rs:353-430`; CLI `entries.rs` key pinning (about 60) | None. A H7 calls it low value (the agent can reach the daemon anyway). PR8 already bumps `CONTROL_API_VERSION` (section 8). | Fold into PR8 as an option, since the version bump is paid there anyway. Only status signing. Receipts and acks stay. | about 150 / about 250 | Medium. It adds to the riskiest PR. Skip it if PR8 is late. |

#### 2. Release-week work no point needs before Friday

| # | Work | Where | Why it serves no point now | Instead | Effect |
|---|---|---|---|---|---|
| W1 | Worker code edits in PR6 (K5 Worker part) and PR10 (CH11 "Worker stops parsing") | `server/worker/src/runs.ts:499-533`, `notices.ts`; `runs.ts:73-86` | The Worker already accepts a heartbeat with no `vitals` and no `repo_head` (`runs.ts:452-458`, `vitalsField` returns null at `:507`). With no client sending `agent_paused`, the `disk_full` notice never fires. The Worker reads only the first line of the tunnel list (check.md item 14), so CH11 needs no Worker change while the format line stays. | Make the Friday Worker deploy config only: `MIN_COMPETITION_VERSION` and `EVENT_ID` (CH14). Delete the dead Worker disk code after Edition 01. | No Worker code deploy before 7 Nov. The "Worker first" rows for PR6 and PR10 in section 8 go. About 100 Worker src lines move later. |
| W2 | K3: delete the Rust stand-in on Wednesday (PR12) | `crates/goalcup-cli/tests/common/upload_server/` and 10 importing test files (check.md item 3) | Tests only. Point 5 is proved by AD7/N4 whether or not the stand-in exists. Porting 10 files to `wrangler dev` has no estimate, and it lands on release-candidate day. | Keep AD7 (N4, about 300 tests). Removing `#[ignore]` is the whole gate change (check.md item 19). Move K3 to after Edition 01. | Saves an unestimated port on the busiest day. The 5,423 test deletions move later. |
| W3 | CH7: one worker thread per core | `commands/start/proxy.rs:98` | No measurement shows proxy latency (L: "latency not recorded"). The plan names its own risk: "a hidden ordering assumption". | Hold it. Record per-call latency in the 2-hour run and change it after Edition 01 if needed. | 1 line not changed. One unknown removed. |
| W4 | CH16: sweep 12 port-race sites and about 30 time bounds | J-R8, J-R9 sites | The plan's "Why" column says "trust in the gate", which is not one of the seven points. Several sites sit in tests that later cuts delete (`control_api.rs:896-898`, `:1319-1321`, `daemon_client.rs:984`). | Fix a flake only when it fails a PR this week. Sweep after Edition 01. | Fewer test edits during the freeze. |
| W5 | PR13 retires 15 decisions and merges 7 docs on Thursday | Section 11; K says about 600 decision-cite lines and about 90 code-comment cites in `crates/`, `server/`, `connectors/` | Re-citing code comments is a code edit on the buffer day. Internal doc merges serve no point for participants. `check-version.py` breaks on the CONTRACTS merge (A3). | Before Friday, do only participant-facing truth (PR1 plus the doc lines inside each code PR). Pure deletes of history files can go any day. Move the DECISIONS retirements and the CONTRACTS, TERMINATION and CHEAT-CHECKLIST merges to after Edition 01. Put one line atop the phase 2 notes: "the decisions in the progress file win" (A H13). | About 600 cite lines no longer touched in release week. Thursday stays a real buffer. |
| W6 | AD5 warning for Claude Code cloud switches | `model_host.rs` | AD8 already tells participants that cloud switches are not recorded. D says doc-only is cheaper. A warning is new start-path code. | Keep AD5's `openai_base_url` reading and the Codex built-in refusal (both serve point 1). Drop the Claude warning. | about 20 / 20 not added (estimate) |

#### 3. Caps of our own still standing (not in CH1 to CH16)

| Cap | Path:lines | What it does today | Point hurt | Smallest fix | Lines |
|---|---|---|---|---|---|
| Daemon per-stream limit drops the whole producer | `crates/goalcup-daemon/src/runtime/capture/sink.rs:270-280` (`Refusal::Limit` gives `LimitExceeded`, then `lose(...)`) | CH4 fixes only the proxy side (`proxy/api.rs`). On the daemon side, one stream past the bound still loses the whole producer, so wire capture for that proxy epoch ends. Whether the proxy then starts a fresh epoch is unverified. | 1, 3 | Use the CH4 rule here too: refuse the one stream with a gap, and keep the producer. Do it in PR8, which already touches `sink.rs`. | about 10 src, 1 test |
| Payload queue full drops the whole producer | `sink.rs:273-275` (`PAYLOAD_QUEUE_BYTES`, within the 8 MiB `MAX_CAPTURE_QUEUE_BYTES`, `crates/goalcup-protocol/src/capture.rs:51`) | A burst larger than the queue loses the producer (G names this the CPU risk; C suggests 64 MiB). CH7 and more streams make bursts more likely. | 1 | Raise it and state the memory it guards, or lose one frame with a gap. Measure `QueueFull` in the run. | 1 to 10 src |
| Head size | `crates/goalcup-cli/src/proxy/http.rs:86` (`MAX_HEAD_BYTES` 64 KiB) | L calls it a bound that "guards nothing real". What happens past it is unverified (an error at `:721`). | 1 | Raise to 1 MiB (L-F). | 1 |
| Verifier goal streams | `crates/goalcup-verifier/src/capture_analysis/reader.rs:78, 1384` (`MAX_GOAL_STREAMS` 128) | Left out of CH1 (check.md item 23). | 5 (an invalid read) | Move it with CH1 or say why it stays. | 1 |
| Start open tries | `crates/goalcup-cli/src/upload/mod.rs:44-45` (3 tries, 3 s apart) | A Worker blip of 9 s in the 24 h start window stops the start. | 3, 7 | Retry with backoff for a few minutes, with one line a minute (F14 gives the shape). | about 15 src |
| Close window and credential | `server/worker/src/db.ts:17, 57` (24 h) | Upload must finish within 24 h of the deadline. AD8 only tells people. | 5 | Owner question (missing). The value needs a website migration, so tell people for Friday and change it later. | 0 now |
| Tunnel list 8 MiB, client and Worker | `upload/uploader/finish.rs:257`, `runs.ts:27` | Not in CH11 (check.md item 14). With one row per host it will not bite. | none, if rows aggregate | One line that says why each stays. | 0 |
| Producers 65,536 per entry | `capture.rs:63` | States its reason (one per 9.2 s for 168 h). I checked: a recorder restart reuses `shared` (`proxy/mod.rs:1049-1060`), so CH6's restart loop does not burn producers. | none | Keep. | 0 |

#### 4. Later cuts that could land before Friday safely

| Id | Cut | Why it is safe now | Lines | Risk |
|---|---|---|---|---|
| L1 | v5 corpus test and fixtures (part of K14) | Test-only deletion. The owner ended back-compat, and D54 is retired in section 11. Deleting it means PR8 does not have to regenerate a frozen corpus after it changes reader bounds. Keep about 60 lines of binary smoke (C). | 0 / 718 (+912 fixture lines) | Low. Older test traces are no longer re-read. |
| L2 | `check-version.py` (A3, from K25) | No point. It breaks on the PR13 doc merge. | 206 / 128 | Low |
| L3 | Provider probe (A1) | Deletion. The problem line that covers it is already in place. | about 250 / about 170+ | Low |
| L4 | Writer quota: also cut the `export.rs` quota and terminal-allowance branches with K12 | `CHAIN_QUOTA_BYTES` 1 TiB is never reached (`capture.rs:53-56`). C lists the `export.rs` parts as a Friday cut. | about 60 / unknown (estimate) | Low to medium. It touches the chain writer. Skip if PR8 is late. |

**Checked and not safe before Friday** (the plan is right): K13 daemon merge, K15 platform rewrite,
K16 deny lists (they touch every connector loader), K17 pins (the pin gate at
`preflight/gates.rs:111-140` is the only check that a shipped connector was not edited), K18 signing
(five copies of the contract), K21 seal walk (its caps do not block: "the seal goes on",
`goalcup-daemon/src/workspace.rs:121-123`), K22 finish machine, and K24 campaigns
(`capture_crash_campaign.rs` is the only test of `kill -9` and a torn tail across real processes,
which CH9 changes).

**Rows that overstate later savings.** K20 (`db-local.ts`) is not a cut while AD7 stands: N4 runs
`wrangler dev` with `DB_BACKEND=local` (`crates/goalcup-cli/tests/full/upload_worker.rs:2, 50`;
`server/worker/src/context.ts:4`). K24 waits for "N5", but N5 is in no AD row or PR, so K24 never
becomes due. Either schedule N5 or drop K24 from the later total.

#### 5. Against the two fresh designs (B)

| Fresh designs have none of this | Plan keeps it for Friday | Fair? |
|---|---|---|
| Separate daemon, control API, IPC (5,214 + 1,908) | Kept | Yes. 12,000 lines across 22 CLI files, and no measured merged build. |
| Hash chain, signed acks, receipt | Kept | Yes for acks and receipts (prune and upload read them). Status signing can go in PR8 (A9). |
| Termination proof (about 3,910) | Kept | Yes. It ends a 168-hour run, and the replacement is untried. |
| Pins and deny lists (about 4,300) | Kept | Yes for Friday (connector anti-tamper), cut after. |
| Start-time probe | B2 keeps a CA-trust check; today's probe is TCP only | No. Delete it (A1). |
| Sleep lock | Neither has one; both record a sleep gap | No owner question. Add one (A2). |
| Event-rule commands (about 2,280) | Kept silently, except `mcp` | No owner question. Add one (A6). |
| Hermes SQLite | B wants periodic snapshots; the plan keeps one final copy with the cap lifted | Unclear. The copy may not open as SQLite after masking. Owner question (A7). |
| No caps | Plan raises the proxy caps but leaves the daemon-side producer loss | Partly. See section 3. |
| Rebuild tool with timeline and files | `fetch-run.sh` plus the verifier | Yes for Friday (AD7 adds a partial rebuild). |

What B lists that the plan rightly keeps and that both designers missed: `terminal.rs` and
`host_log.rs` (points 3 and 7), the `model_host.rs` config reader (point 1), no-follow session reads
in `sourcecap` (point 4), the installer guard (point 3) and `spikes/redact.py` as an oracle (point 4).
The plan is right to keep each of these.

#### 6. Checked and found justified

- **Cuts K1, K2, K4 to K12:** each serves no point, or its point is covered elsewhere.
- **Adds:**
  - AD1 (point 4, must land before CH8).
  - AD2 and AD3 (point 6, about 25 src each).
  - AD4 (200 src replaces about 9,745 src of decoders and gives the count proof for points 1, 2, 4, 6).
  - AD6 (N1 and N2 prove 2 and 3).
  - AD7/N4 (point 5).
  - AD8 (text).
- **Changes:**
  - CH2 and CH3 (no handle is held between polls, `native_capture.rs:76-81`).
  - CH4.
  - CH5.
  - CH6 (restart forever reuses the same producer state).
  - CH8 to CH10 (CH10 is owner Q9).
  - CH12 (if A7 keeps the row).
  - CH14 config.
  - CH15.
- **Bounds that state their cost and stay:**
  - `max_dir_entries` 8,192 per poll is a work bound that resumes on the next poll
    (`native_capture.rs:69-75, 419`), so nothing is lost.
  - `MAX_FRESH_IDS` 128 stops a sink loop and states why (`native_capture.rs:190-194`).
  - The 180 s finish budget (`control.rs:708`).
  - `RUN_MAX_COMPRESSED_BYTES` (Q3).
  - The rate limiter (guards a real cost, as B.6.1 says).
- **Q10, the release freeze with `MIN_COMPETITION_VERSION`:** needed, because the version is
  checked at start only (F15).
- **check.md correction 1 (CH1 cannot set `MAX_LIVE_PRODUCERS` to 16,384):** I agree. Keeping 128
  live producers is right, because producers are per proxy epoch, not per subagent.

#### 7. Not verified

- Whether the proxy starts a fresh epoch after the daemon loses its producer (section 3, row 1).
- Whether the sleep-lock problem line shows in the Vercel Sandbox (A2).
- Whether the battery `todo` blocks a start (`commands/prep.rs:974-979`, H11). It is not checked here.
- What happens past `MAX_HEAD_BYTES`.
- Whether `RESEND_API_KEY` is set in production. Emails are off while it is unset
  (`server/worker/wrangler.toml:71, 84`). If it is unset, the email code is dead in production, and
  that is worth one owner line.
- Every line figure marked "about" is an appendix estimate, plus or minus 30 percent.

### 3a. Answers to review 2

**Section 1: kept parts with no point behind them**

| # | Point | Answer | What the plan does now | Evidence checked |
|---|---|---|---|---|
| A1 | Delete the provider probe instead of making it a warning | Accept | K7 deletes `proxy/preflight.rs` and its call. This also ends the K7 and CH13 clash. The "no model calls" problem line covers an offline start. | `crates/goalcup-cli/src/proxy/preflight.rs:1-420` (inline tests from `:251`); `participant/status.rs:465-509` |
| A2 | Sleep lock serves no point; owner question | Reject | It keeps a laptop awake, so a week-long run on a laptop keeps working; that helps the participant and costs no recording. It refuses nothing and blocks nothing. The sandbox `PROBLEM:` line is already expected and harmless (`docs/research/VERCEL-COMPAT.md:44`); the 2-hour run notes it. No owner question: the ten slots go to decisions that change Friday. | `crates/goalcup-platform/src/awake.rs`; `F-week-long-risks.md` F5 |
| A3 | Cut `check-version.py` | Accept | Cut in PR2 (K2), with `gate.sh:61` and `release.sh:101-104`. It reads `docs/CONTRACTS.md` and `docs/DECISIONS.md` (`check-version.py:46-47`). | `scripts/gate.sh:61`; `scripts/release.sh:101` |
| A4 | Cut the client run give-back | Reject for Friday | Harmless today (lane I), and it sits on the upload start path. Moved to K27 after Edition 01; Q5 names it. | `crates/goalcup-cli/src/upload/mod.rs:297-495` |
| A5 | Synced-folder refusal and `.gitignore` matcher | Reject | The refusal keeps a run folder, which holds `request_dump_*.json`, off sync services (`crates/goalcup-cli/src/per_run_home.rs:310-313`): that is point 4. It refuses before the harness starts, so it never blocks a running harness (point 3). The `.gitignore` matcher finds a project token (`commands/start/project.rs:333`); it stays with it. | `per_run_home.rs:310-340`; `commands/start/project.rs:231-333` |
| A6 | Event-rule commands with no owner question | Partly accept | No client change before Friday. Q5 now names `login`, SSH key warnings, push lines, the client repo check and the give-back, all after Edition 01. Making the repo check a hint now is not done: no evidence shows it refusing a valid run. | `B-fresh-design.md` B.6.2 item 9 |
| A7 | Hermes `state.db` copy may not open as SQLite and holds old sessions | Partly accept | Confirmed: a masked span becomes `<redacted:KIND>`, which changes length (`crates/goalcup-daemon/src/redact.rs:1199-1207`). The row stays as best effort (it still gives the Hermes subagent tree when nothing is masked); CH12 stays; AD8 and Q4 say so. | `connectors/hermes-cli/harness.toml:111-121` |
| A8 | `no_family_literals.rs` guards a retired rule | Reject | It guards a live hard rule: "No harness names in core code" (`AGENTS.md:87`). D68 is about the connector schema lock, not this rule. 378 test lines, no runtime cost. | `AGENTS.md:87`; `docs/DECISIONS.md:784` |
| A9 | Fold status signing into PR8 | Reject | It adds work to the riskiest PR for no point. It stays in K18 after Edition 01. | `crates/goalcup-protocol/src/control.rs:353-430` |

**Section 2: release-week work no point needs**

| # | Point | Answer | What the plan does now | Evidence checked |
|---|---|---|---|---|
| W1 | No Worker code edits before the event | Accept | Friday's Worker deploy is configuration only. The Worker disk code moves to K27. CH11 keeps the format line, so the Worker needs no change. | `server/worker/src/runs.ts:452-458, 507`; `runs.ts:73-86` |
| W2 | K3 on release-candidate day | Accept | After Edition 01. | fact check item 3 |
| W3 | CH7 has no measured need | Accept | Withdrawn for Friday. The 2-hour run records per-call latency. Review 1 found it safe; safe is not the same as needed. | `crates/goalcup-cli/src/commands/start/proxy.rs:96-100` |
| W4 | The flake sweep serves no point | Accept | CH16 is now "fix a flake only when it fails a PR this week". | `J-tests-gate.md` B4 |
| W5 | PR13 retirements and merges on the buffer day | Accept | PR13 is pure deletes of history docs. Merges, retirements and the code-comment re-cites move after Edition 01. PR1 adds the line atop the phase 2 notes. | `K-docs-decisions.md` |
| W6 | Drop the Claude Code cloud-switch warning from AD5 | Accept | AD5 is Codex only; AD8 names the Claude Code switches. | plan AD5, AD8 |

**Section 3: caps of our own still standing**

| Cap | Answer | Reason and evidence |
|---|---|---|
| Daemon per-stream limit drops the whole producer | Reject | The proxy refuses an over-limit stream or connection before it sends it (`crates/goalcup-cli/src/proxy/api.rs:451, 599`), and the native reader waits before it opens one (`native_capture.rs:1555-1556`), both against the same protocol constant. The daemon check at `sink.rs:270-280` is a backstop that a correct producer never reaches. Whether the proxy opens a fresh epoch after such a loss stays unverified. |
| Payload queue full drops the whole producer | Accept | CH1 raises `MAX_CAPTURE_QUEUE_BYTES` to 64 MiB (it must rise anyway to hold the larger connection reserve) and states the memory. The 2-hour run counts `QueueFull` gaps. |
| Head size 64 KiB | Reject | Past the bound, the proxy relays the bytes unparsed (`crates/goalcup-cli/src/proxy/http.rs:287, 489`), so the harness is not blocked. Whether those bytes are kept as an opaque record is unverified. No point gains. |
| Verifier goal streams 128 | Accept | CH1 sets it to follow `MAX_PRODUCERS`. |
| Start open tries (3 tries, 3 s apart) | Reject for Friday | The start window is 24 hours; a failed start is plain, can be retried, and nothing runs unrecorded (`crates/goalcup-cli/src/upload/mod.rs:44-45`). The F14 retry shape waits for after Edition 01. |
| Close window 24 h | Reject a change | It needs a website migration. AD8 tells participants to upload within 24 hours of the deadline. |
| Tunnel list 8 MiB, client and Worker | Accept | CH11 says why both stay: they bound the seal body the Worker accepts. |
| Producers 65,536 | Agree | Kept. The review's check that a restart reuses the producer state (`proxy/mod.rs:1049-1060`) is cited in CH6. |

**Section 4: later cuts that could land before Friday**

| Id | Answer | Reason |
|---|---|---|
| L1 v5 corpus test | Accept | K26, PR4. Test-only, back-compat ended. |
| L2 `check-version.py` | Accept | K2, PR2. |
| L3 provider probe | Accept | K7, PR9. |
| L4 `export.rs` quota branches | Reject for Friday | It touches the chain writer in release week; K14 after Edition 01. |

**Rows that overstate later savings**

| Point | Answer | Reason |
|---|---|---|
| K20 `db-local.ts` is needed by N4 | Accept | Confirmed: `upload_worker.rs:50` sets `DB_BACKEND=local`, and `server/worker/src/context.ts:149` reads it. K20 now cuts only `github.ts`. |
| K24 waits for an N5 nobody schedules | Accept | K24 leaves the later total, with a note that it waits for N5. |

**Section 5 (against the fresh designs).** Every "No" row is answered above: the probe (A1, accept),
the sleep lock (A2, reject), event-rule commands (A6, partly accept), the Hermes copy (A7, partly
accept), daemon-side caps (section 3). The rows marked fair need no change.

**Section 6 (checked and justified).** No answer needed. CH7, which the review did not list as
justified, is withdrawn (W3).

**Section 7 (not verified).** The epoch question and the head-bound question stay unverified, as said
above. The sleep-lock line in the sandbox is expected per `VERCEL-COMPAT.md:44`. The battery `todo`
was not checked here either (unverified). `RESEND_API_KEY` in production is unverified; the plan's
risk list now names it.

## 4. Net effect on the plan

| Measure | First draft | Now |
|---|---:|---:|
| Friday cuts, src | about 19,300 | about 19,000 |
| Friday cuts, tests | about 35,300 | about 28,000 |
| Later cuts, src | about 25,600 | about 25,100 |
| Later cuts, tests | about 37,400 | about 40,500 |
| Adds, src / tests | about 400 / 800 | about 400 / 800 |
| Worker code changes before the event | 2 PRs | none |
| PRs | 13 | 14 (PR8 split) |
| Owner questions | 10 | 10 |

The test cut is smaller because the reviews showed some test deletions were either risky on release
day (K3) or guards the cup still needs (`m2_contract.rs`, the release-script tests). The source cut is
about the same: the probe, `check-version.py` and a larger K11 replace what stays (process vitals,
session sniffing, the `goalcup mcp` command, the owed-ack path).
