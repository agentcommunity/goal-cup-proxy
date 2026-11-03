# Alignment review X2: scope creep and over-engineering across the PRD set

Scope: every file in docs/plans/2026-10-30-v1.0/ (PRD-00 to PRD-17, BUILD-PROMPT.md) read against the
ledger, VISION.md (less code, no caps of our own) and AGENTS.md, and sized against the fresh designs in
docs/plans/2026-10-30-audit/B-fresh-design.md (B1.11, B2.12, B.6.2). Paths are relative to
docs/plans/2026-10-30-v1.0/ unless they start with `docs/`. Findings already raised by other alignment
parts are listed at the end and not repeated.

Counts: 0 blocker, 3 major, 8 minor.

## Planned size against the fresh designs

Plan numbers: PRD-00 "Size budget" (lines 753-777) and each PRD's "Files it owns". Fresh numbers: B1.11 and
B2.12 (Go). The fresh designers give Rust at 1.3 times (B2) to 2 times (B1) the Go lines; the last
columns use that range.

| Area | 1.0 plan src | B1 (Go) | B2 (Go) | Fresh in Rust (x1.3 to x2) | Plan against fresh |
|---|---:|---:|---:|---:|---:|
| Proxy, HTTP/1.1, HTTP/2, WebSocket, decoding (PRD-03) | 4,400 | 440 | 580 | 570 to 1,160 | 3.8x to 7.7x |
| Masking (PRD-05) | 3,530 | 250 | 250 | 325 to 500 | 7x to 11x |
| CLI, approval, status, text (PRD-10) | 3,040 | 560 | 450 | 585 to 1,120 | 2.7x to 5.2x |
| Format, pipeline, watchdog (PRD-01, PRD-02) | 2,350 | 270 | 260 plus supervisor | 350 to 540 | 4.4x to 6.7x |
| Native (PRD-04) | 780 | 150 | 250 | 195 to 500 | 1.6x to 4x |
| Upload (PRD-06) | 940 | 120 | 280 | 156 to 560 | 1.7x to 6x |
| Connector code (PRD-11) | 800 | 100 | 0 (TOML) | 0 to 200 | config hints are extra (B.6.2 row 4 asks for a small one) |
| Verifier and usage (PRD-08, PRD-09) | 2,870 | 150 (rebuild) | 250 (rebuild) | 195 to 500 | not comparable: S2 and T4 add stats and checks |
| **Client src without verifier and usage** | **15,840** | **1,890** | **2,070** | **2,450 to 4,140** | **3.8x to 6.5x** |
| **Rust tests** | **18,000** | **1,320** | **1,980** | **1,700 to 3,960** | **4.5x to 10.5x** |

What explains part of the gap, and is asked for: HTTP/2 (H6, about 450), the hash chain kept so the
Worker and database need no migration (S3), the restart path and null mode (R3), the usage reader (S2),
the pad checks in the verifier (T4), 15 connectors and config hints (T1, H2), the 20,000-stream and
2,000-connection guards (R5), and the proven parts copied on purpose (L3: CA and TLS about 450, HTTP
framing about 600, WebSocket about 500, decoding about 500, masking about 2,600). What is not asked for
by any ledger item, point or rule is in the findings below. Even after those cuts the plan stays about 4
times the fresh shape; most of the rest is L3 copies, which the owner chose knowingly.

## Findings

| id | File and section (lines) | What is wrong | Ledger item, rule or point | Severity | Exact fix |
|---|---|---|---|---|---|
| X2-01 | PRD-00 "Size budget" (753-777); PRD-13 "Target size after 1.0" (454-492); BUILD-PROMPT section 4 step 3 (reviewer B) | The plan sets no size target, only an overrun rule: "A builder who needs more than 30% over its row says why in the night log" (777). So 18,700 src may become about 24,000 with a log line. The budget already grew from revision 1's 12,000 to 16,000 to 18,700 (775). Tests (18,000) equal src, although PRD-14 section 1 says an automated test is written "only where a real run cannot show the failure every time". Nobody compares a PRD with the fresh design, so reviewer B has no yardstick for "less code" | L2 (locked: less code, cut before you add); VISION "Less code" | major | PRD-00 line 775-777: replace "The 18,700 is above ... says why in the night log." with "Each row is a ceiling, not an estimate. A builder who would pass its row first cuts what serves no point; past the row by more than 10% needs a replan `decision` line that names the point the extra lines serve. Reviewer B checks each merge request against its row and against the fresh design's row (audit B, B1.11 and B2.12). Targets after the cuts of X2-02 to X2-08: Rust src about 18,100, Rust tests about 15,000." Change the Rust row to "about 18,100 / about 15,000". PRD-13 table (472): same two numbers. BUILD-PROMPT section 4 step 3, reviewer B: add "and the PRD's size row (PRD-00 Size budget)". PRD-16 replan role (row "Replan", section 1): add "and posts `wc -l` of the merged PRD against its row" |
| X2-02 | PRD-05 "Files it owns" (479), "What is cut" (517-526); PRD-14 section 3 (114, 122) | Masking tests are budgeted at 5,000 lines, the largest test share, against 400 (B1) and 350 (B2) in the fresh designs. About 4,300 of them are old test code copied from four suites (`redaction.rs` 1,000, `capture_redaction.rs` 2,200, `capture_redaction_extra.rs` 600, run secrets 300, helpers 200). L3 asks to copy "masking rules and their test corpus": the corpus is the 86 fixture files (data, no line cost), not every old case. Most copied cases were written for the frame protocol PRD-05 itself cuts, and are then rewritten | L3 (what to copy); L2; VISION "Less code" | major | PRD-05 line 479: "about 5,000" becomes "about 2,000". Add after line 482: "Tests copy the corpus as data, the split-at-every-byte property test run over the whole corpus, and only the old cases that a PRD-17 lesson names or that pin a rule's edge (PEM pieces, escapes, the left guard, `url_password`, fail-closed per message). Other old cases are not ported; the property test over the corpus covers them." Line 526: "Tests about 10,600 to about 5,000" becomes "to about 2,000". PRD-14 line 114: "5,000 (PRD-05)" becomes "2,000 (PRD-05)"; line 122 total "about 18,000" becomes "about 15,000"; drop "Masking is the largest share on purpose" (124-125) or keep it with the new number |
| X2-03 | PRD-10 section 3 step 7 (156), section 7 step 4 (413-415), section 8 (455-461), section 9 (480-494), files table (593, 595, 597-598) | The CLI is 3,040 src lines (2.7 to 5.2 times the fresh shape) and carries parts no ledger item, point or rule asks for: (a) the sleep lock, 150 lines on three systems (B.6.2 row 7: "Not a point ... owner call"); a sleeping machine pauses the harness too, so nothing goes unrecorded; (b) `--prompt` composition plus clipboard support on four systems (`pbcopy`, `wl-copy`, `xclip`, `clip.exe`), a new feature from a planner default (owner question 2), with its own cap: a prompt over 64 KiB is refused (455), a limit of our own on the participant's one prompt; (c) setup warnings for battery and a TCP probe of the connector's model hosts (413-415), the same probe this PRD cuts from `start` because "the no-requests line catches an offline start" (171); (d) `text.rs` at 520 lines for every message | L2; VISION "Less code" and "No caps of our own"; point 6 is already met by status and the end screen | major | Section 9 (480-494): replace with "No sleep lock in 1.0. The launch block and the guide say once: keep the computer awake and on mains power. A suspend is written as a `suspend` event (PRD-01)". Delete step 7 (156), `src/cli/awake.rs` (595) and the sleep warning in 413. Section 8: keep printing the push block as today; `--prompt`, `first-message.txt` and the clipboard only if the owner answers yes to question 2's push part; if kept, drop the 64 KiB refusal ("reads the file as UTF-8; no size limit"). Line 415: delete "the model hosts of the chosen connector reachable (TCP, a warning only: F3);" and "on battery (H11);". Files table: `push.rs` 110 becomes 40, `text.rs` 520 becomes about 420, total "about 3,040" becomes "about 2,600". PRD-00 Size budget CLI row 3,040 becomes 2,600 |
| X2-04 | PRD-03 section 10 (418-430), section 4 point 4 (249-252), section 7 decoder guard (356-362), files table (457-475); PRD-02 section 2 rule 7; PRD-00 S1 `reserve` | Surface built with no caller, or built twice: `ProxyHandle::stop_accepting()` and `open_conns()` exist for a graceful hand-over that PRD-02 section 9.1 says was "weighed and not built", and PRD-02 takes `after_stop` hosts from its writer table; `Alpn::Mirror` is built and tested from the start though it is switched on only after late HTTP/2 rows; decoder memory has a second process-wide counter (`decoder_window`) beside `Recorder::reserve`, which then has no user. Consolidates P03-12, P03-13, P03-1 and P02-17 for the size budget | L2; R5 (one stated guard) | minor | PRD-03 line 418-430: keep only `idle()`; delete `stop_accepting` and `open_conns` from the text, line 498, line 616 and builder note 3 (663). Line 249-252: "A switch `Alpn::Mirror` ... exists from the start" becomes "Mirroring is added only after acceptance 3 passes with a night-log line". Section 7: the decoder window is taken with `Recorder::reserve(n)`; delete the separate 256 MiB counter. Files table total "about 4,400" becomes "about 4,250"; PRD-00 Proxy row the same |
| X2-05 | PRD-02 section 11 step 3 (536) and `pipeline_seal.rs` (730-732) | A time cap that drops recorded bytes: at every seal, queued bytes still unwritten after 60 s become `cut_at_seal` gaps, also on a plain harness exit, where nothing forces a deadline. With the 512 MiB guard and a slow disk this can cut real data at the end of a week. Same class as the 180 s native cut (L3-4, P04-2), not raised for the drain | R5 (no caps of our own; guards never stop recording); R4 | minor | Line 536: "**Drain:** wait until `held` is 0, at most 60 s (5 s for `signal`)." becomes "**Drain:** wait until `held` is 0 while the writer makes progress; stop early only when the writer wrote nothing for 60 s (5 s for `signal`). Bytes still queued then become `cut_at_seal` gaps." Line 730-732: "past the drain budget" becomes "after the writer stalled for 60 s". Overridable defaults row "Seal drain" the same |
| X2-06 | PRD-14 section 8 gate table (294), G1 (452); PRD-05 interfaces row PRD-14 | `gate full` builds and runs a release-mode test step whose only test, `release_mask_bench`, "has no floor and fails only if it crashes". It proves no point and costs a full release-mode test build (with `test-hooks`) on every `gate full`, before the real release build | L2; ledger section 7 test rule (a test stays if it proves a point) | minor | Delete the row at 294. Rename the bench `by_hand_mask_bench` (`#[ignore]`, never in a tier), run once by hand for PRD-02 R8 and PRD-05 A5, number in the night log. G1 (452): drop "the `release_mask_bench` report". PRD-05 interfaces row PRD-14: "`mask_bench` becomes `by_hand_mask_bench`, run by hand, report only" |
| X2-07 | PRD-12 section 3.1 (182-212), files table `exec/` (573) | The pad has seven fault kinds and four exec scripts on top of T4's "one tool, three cases". Several repeat a guard that already exists: `fd-limit` repeats PRD-02 `watchdog_rlimit.rs`; `kill-watchdog` and `kill-both` repeat `watchdog_restart.rs` and `pipeline_restart.rs` at the real-run layer; `route-drop` needs `sudo ip route del default` and may cut the box's own control channel (risk at 681, unverified) | T4 (locked shape of the pad); L2 | minor | Keep `kill-recorder` (done item 2) and `cut-worker` (U1, upload). Run R4, R5 and R7 once each through `--exec` scripts (as the other one-off proofs already do), not as fault kinds with their own expectations in `check.py`. Drop `route-drop` and `freeze` unless the owner wants them; PRD-02 R4 stays proven by `watchdog_rlimit.rs`. Files table: `pad-run` 300 and `check.py` 320 shrink by the fault branches (about 80) |
| X2-08 | PRD-14 section 9 `install.sh` (348-350); PRD-15 rule 7 (87) and guide appendix C (614-617, 632); PRD-10 section 11 (529-531, the 0.7 MCP line) | Code and docs for 0.7 leftovers: `install.sh` deletes a 0.7 `goalcup-daemon` and the 0.7 `share/goalcup/connectors/` folder on upgrade, and the guide gets an appendix for 0.7 testers. Every run so far is test data; there are no 0.7 participants to migrate | L7 (no backward compatibility) | minor | PRD-14 line 348-350: delete "on upgrade it deletes a `goalcup-daemon` left by 0.7 in the prefix and the 0.7 `share/goalcup/connectors/` folder;". PRD-15: delete guide appendix C and its exceptions (87, 614-617); risk row 632 says "0.7 is refused once the floor is set". PRD-10 section 11: delete the 0.7 `goalcup mcp` paragraph; one line in the release notes covers testers |
| X2-09 | PRD-15 section 2 rule 2 (75-76) against: PRD-11 section 7 table (654-692) and "Files it owns" `docs/harnesses/<id>.md` with "the pad recipe" (715-719) and `docs/harnesses/README.md` table (720); PRD-15 guide "the harness table with tiers" (97, 390-391); PRD-12 `scripts/pad/harnesses/<id>.toml` (574) | The same harness facts are planned in four places (PRD-11's table, 15 fact pages, the harnesses README table, the guide's harness table), and each harness's pad recipe in two (the fact page and the pad harness file). PRD-15's own rule is "One place per fact". Four copies drift and cost doc lines every lane must keep true | L2; PRD-15 rule 2; point 6 (one true answer) | minor | One harness table: `docs/harnesses/README.md` (tier columns generated from `connectors/tiers.toml`). The guide links to it instead of holding a second table (PRD-15 line 97: "the harness table with tiers" becomes "a link to `docs/harnesses/README.md`"; delete 390-391's table text). Fact pages hold facts and limits only; the pad recipe lives only in `scripts/pad/harnesses/<id>.toml` (PRD-11 line 715: delete "plus the pad recipe (appendix O section .7 of each harness)") |
| X2-10 | PRD-00 "Morning report template" (789-852) and PRD-16 section 13 (379-406) and BUILD-PROMPT section 8 | The end-of-build report is specified three times with different item lists (PRD-00's template has the codebase line and the gateway answer in its own order; PRD-16 has eleven items; BUILD-PROMPT a third list). The scribe must reconcile them | L2; one place per fact | minor | PRD-16 section 13: replace the numbered list with "The scribe fills PRD-00 'Morning report template'; that template is the one list." BUILD-PROMPT section 8: replace the list after "on the integration branch" with "following PRD-00 'Morning report template'" |
| X2-11 | Every PRD: "Review answers" sections (PRD-00 874-1042 and 17 others, about 900 lines) and the settled "Asks" sections | Planning history sits inside build specs. Builders read it with the spec; it repeats ruled items with old question numbers, which is the source of many stale cross-references other reviews found (P00-14, P05-9, P11-7, P12-17). It serves no point and no builder task | L2; point 6 (plain, true text) | minor | Move every "Review answers" section and every fully ruled "Asks" section into one file `REVIEW-HISTORY.md` in this folder, verbatim, with one heading per PRD. Each PRD keeps one line at its end: "Review history: REVIEW-HISTORY.md, section PRD-NN." The replan keeps new asks in the "Amendments" note only |

## Caps and checks of our own still in the plan

| Limit or check | Where | Kind | Verdict |
|---|---|---|---|
| Soft memory guard (512 MiB or RAM/8), gap and go on | PRD-02 section 3 | stated memory guard | aligned (R5) |
| Hard guard, exit 75 and restart | PRD-02 section 3 | stops recording for a restart | raised as L2-07 |
| Masker hold 16 KiB and 64 candidates, then `fail_closed`, counted | PRD-05 section 6 | memory guard; bytes starred and counted | aligned (named, never silent) |
| `client_hello`, `http_head`, `upgrade_copy`, `hpack_table`, `decoder_window` | PRD-03 section 11 | memory guards; parsing stops for the rest of a connection | raised as L2-15, P03-1; X2-04 |
| Seal drain 60 s | PRD-02 section 11 | time cap that drops queued bytes | X2-05 |
| Native finish 180 s | PRD-04 section 8 | time cap | raised as L3-4, P04-2 |
| Periodic database copy interval | PRD-04 section 6 | cost guard | raised as L2-18, P04-24 |
| 32 exact values, 20 to 256 bytes | PRD-05 section 9 | start-up guard; values past 32 unmasked | raised as P00-23, P05-15 (and L4-1 for the feature) |
| Config files over 1 MiB skipped | PRD-11 section 2 | memory guard; a gateway hint is lost | raised as P11-16 |
| `--prompt` file over 64 KiB refused | PRD-10 section 8 | cap on the participant's prompt | X2-03 |
| 25 s upstream retry on a listed host | PRD-03 section 2 | changes harness timing | raised as L2-14, P03-14 |
| Null mode retry 5 to 60 minutes | PRD-02 section 8 | off time | raised as F6, P00-2, P02-8 |
| Build stop lines 400 USD and 150 USD, 2 USD per smoke run | PRD-00, PRD-12, PRD-16 | spend caps beyond T6 | raised as P00-1, L6-09, L5-02 |
| Chunk 32 MiB, JSON body 64 KiB, heartbeat `counts` 2 KiB, control line 64 KiB | PRD-07, PRD-00 S8 | request and line size guards, nothing lost | aligned (the 64 KiB control line must grow if X2 or L4-1 change the 32-value guard) |
| Upload wrap-up 60 minutes, 24-hour tail | PRD-06 section 9 | nothing lost; the rest waits on disk | aligned |
| Start refusals: folder empty, `.env` in `.gitignore`, not in a git tree, `GH_TOKEN` line, WSL1, repo public and empty, `home_busy`, file connectors in competition, connector `version` prefix, `refuse_if_set` | PRD-10 section 3, PRD-11 section 2 | checks before launch | aligned: each maps to RULES or a point (repo and folder: RULES; `home_busy`: lesson X4; `refuse_if_set`: H7 and point 1; `version`: OpenCode 1 against 2) |
| Verifier checks beyond T4: `conn_bytes`, `mask_failclosed`, `refusal_after_trust`, `recorder_alive` | PRD-08 section 3 | silent-loss detectors | aligned (R4); report-only except `conn_bytes` at 50% |

## Also found by other alignment parts (not repeated, but scope creep)

- Exact-value masking from four sources and the Stripe pair built by default: L4-1, P00-6, P05-4.
- Whole-build spend stop lines and the 2 USD smoke cap: P00-1, L6-09, L5-02, P12-14.
- The `test-hooks` feature and `GOALCUP_TEST_*` hooks against L6 "legacy and test seams": F8, P00-7, P02-13.
- The watchdog's many jobs against "a tiny watchdog": F5, P00-3, P02-9, L2-19, P11-20.
- Three new crates where L3 says one: F10.
- The seal's `dirty` digest read by no PRD: P01-10. The usage `--text` summary: P09-14. The `vendor` bool that repeats the host list: L3-11.
- `CLAUDE_CODE_RETRY_WATCHDOG` and `CLAUDE_CODE_DISABLE_AGENT_VIEW` set on the harness: L3-2, P11-6.
- Rate-limit header exemption and kept query strings in masking: L4-4, L4-5.

## Checked and found aligned

- L6 cut list: PRD-13 groups A to G remove the daemon and its IPC, every listed cap, the research decoders
  (PRD-09 keeps only model, usage, error and thread parts), research and disk-full scripts, pins, deny lists
  (the old `capture_policy` tables), termination proof, signing, legacy seams and retired versions. Checked
  against PRD-00 "Not in this build" and PRD-13 sections 5 and 6.
- Cut discipline is real: no live spend pricing, no judge view, no run 1 converter, no silent-drop hook,
  no `stopped-runs.sql`, no mask worker pool or first-byte dispatch until measured, no HTTP/2 push or RFC
  8441, no native hot and quiet scheduler or tail witness, no DECISIONS rewrite or history purge
  (PRD-00 581-586; PRD-02 section 4; PRD-04 "What is cut"; PRD-05 section 11).
- No harness names in core code: PRD-14 `rule_code.rs` rule 1; connectors are flat data files (PRD-11).
- No per-subagent code: subagents come from the wire and the session-file walk, as both fresh designs
  found (PRD-04 sections 1 to 3; B finding 3).
- Uploader: no second compression, no spool, no 2-hour prune timer, no disk branch (PRD-06 sections 1, 7).
- Worker: shrinks to about 3,440 src; routes cut (receipt, abandon, seal), no new route, no migration,
  out-of-scope files untouched (PRD-07 sections 2, 11).
- One end-to-end test of about 150 lines in place of two suites of 2,825 lines; no speed number fails a
  gate (PRD-14 sections 7, 8).
- `refuse_if_set` is not a deny list: it serves H7 (a harness setting that beats `HTTPS_PROXY`) and is
  named before launch (PRD-11 section 2).
- Refusal learning with a trusted set and expiry is needed for H5 (a pinned client works through a
  tunnel after one refusal) and is kept to about 120 lines (PRD-03 section 4).
- PRD-17: one guard per lesson, no lesson tested twice at the same layer (section 1).
- PRD-16: the breaker is a third reviewer on core parts only; B2 says "at least two", so it is allowed.
  A4 is the phase 5 Kilo gateway row, not an extra run.
- BUILD-PROMPT section 4 step 3 already gives reviewer B "less code ... anything that serves no point";
  X2-01 adds the yardstick it lacks.
