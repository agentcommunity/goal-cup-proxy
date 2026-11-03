# Alignment review: PRD-14 tests and gate (plus BUILD-PROMPT on B1, B2, B2a)

Reviewed: `docs/plans/2026-10-30-v1.0/PRD-14-tests-gate.md` (625 lines), end to end, at branch head 638bcfd6.
Read against: OWNER-DECISIONS.md (ledger), VISION.md, AGENTS.md, PRD-00 (Today, Architecture, ownership,
Where the build runs, Build order, merge rule, done, AGENTS changes, size budget, R3 rulings), PRD-01
(sections 6, 12, 16, interfaces), PRD-02 (tests, hooks, scaffold dependencies), PRD-04 and PRD-06
(acceptance tests), PRD-05 (bench), PRD-08 (tests, flags), PRD-12 (C1 to C6, section 9), PRD-13
(sections 5 and 6), PRD-15 (PRD-14 rows), PRD-16 (sections 1 to 5, 14, files, interfaces, acceptance,
builder notes, asks), PRD-17 (PRD-14 rows), BUILD-PROMPT.md.

Counts: 1 blocker, 10 major, 13 minor.

## Findings

| id | file and section (lines) | what is wrong | breaks | severity | exact fix |
|---|---|---|---|---|---|
| T14-1 | PRD-14 section 8, build-lock paragraph (281-283) | Still names "the fallback of PRD-16 S8 (builds one at a time on the conductor as a non-root user)". PRD-16 S8 (PRD-16:339) now says builds wait for Vercel and never fall back to the conductor; PRD-16 ask 3 (PRD-16:602-603) asked PRD-14 to drop this line and PRD-00 R3-11 accepted it ("builds never fall back to the conductor"). A builder reading PRD-14 would keep a conductor build path | Ledger B1 (locked) | blocker | Replace "It matters only where two cargo jobs share one machine: a build box used by the gate and PRD-12's rebuild step, or the fallback of PRD-16 S8 (builds one at a time on the conductor as a non-root user)." with "It matters only where two cargo jobs share one build box: the gate and PRD-12's rebuild step. Nothing builds on the conductor, even while sandboxes are down (ledger B1, PRD-16 S8)." |
| T14-2 | Builder notes 1 and 2 (513-518); section 8 script step (292); `.config/nextest.toml` bullet (306-311) | The H+0 gate change cannot pass its own gate. (a) The named list runs `discover -s scripts/pad`, but `scripts/pad/` does not exist until PRD-12 lands (checked: no such folder today), and "no skipped step" forbids skipping it. (b) `test_gate.py` tests today's gate (14 old-crate references) and must be rewritten in the same commit. (c) Today's `.config/nextest.toml` filters name `package(goalcup-cli)` and `package(goalcup-daemon)` (`nextest.toml:16-17`); once phase 0 step 3 moves those crates to `exclude`, nextest rejects them, yet note 2 puts the new `nextest.toml` in a later lane step. (d) Between step 2 and step 3 the new `--workspace` gate runs the old suites without their old serial groups | PRD-00 phase 0 (every merge needs a green `gate fast`); PRD-14's own "no skipped step" | major | Builder note 1 becomes: "H+0, one merge request: `gate.sh`, the new `.config/nextest.toml`, the rewritten `test_gate.py`, and the deletes of `check-version.py`, `ci-scope.py` and their tests. The named script list holds only tests that exist on integration; PRD-14 adds `discover -s scripts/pad` in the edit that follows PRD-12's first `test_pad.py` merge (PRD-12 writes a night-log note). The new gate is run on the result of phase 0 step 3 (the workspace edit), and the two merge back to back." Drop `.config/nextest.toml` from note 2 |
| T14-3 | Section 8, `heavy` group (306-311); trap (531-532); risk row (491) | The group is built from exact `binary(name)` filters for binaries that do not exist at H+0, while the PRD's own trap says nextest refuses a filter naming a binary that does not exist (`gate.sh:73-74`). If it does, `gate fast` is red from phase 0 until the last listed binary lands. The group also omits `lessons_launch`, which PRD-00 (`PRD-00:313`) and PRD-17 (`PRD-17:382, 408`) put in `heavy` | PRD-00 merge rule (green gate); PRD-17 interface | major | Replace the filter with one regex and a fallback: "group `heavy` (one at a time): `binary(/^(watchdog_.*\|e2e_.*\|lessons_launch\|pipeline_restart\|pipeline_stall\|pipeline_many_streams\|proxy_connect\|proxy_restart\|native_walk)$/)`. In phase 0 the builder runs `cargo nextest list` on the scaffold, where none of these exists; if nextest refuses a regex that matches nothing, the group starts empty and PRD-14 adds each name in the merge that brings its binary (the owning PRD's merge request names it)." |
| T14-4 | Acceptance G6 (457); builder notes "How a watcher proves it" (541-544); also PRD-16 N5 (PRD-16:501) | The in-build proof runs `scripts/release.sh tag` "on a scratch commit". `release.sh tag` takes no commit: it tags `origin/main` and pushes the tag (`release.sh:54-67`). During the build `origin/main` is 0.7.0 and nothing merges to main, so the command refuses on the version line, not on the release check: the proof is not provable. The pass text "on the verified commit it tags" invites a tag during the build, which PRD-16 forbids ("Nothing is tagged during the build") | Ledger B5, B6 (tags and main are the owner's); provability | major | G6 row: "**The tag is what the pad verified.** In the build, a watcher runs `scripts/pad/table --release-check <N1 commit>` (exit 0) and `--release-check <a scratch commit that changes one line under crates/>` (exit 1, the differing paths named). At the owner's release, `release.sh tag 1.0.0` runs the same check first. `test_release.py` proves `tag` stops on a failed check." Same words in the watcher line and in PRD-16 N5 (an Amendments note) |
| T14-5 | Section 2, point 6 row (97); G2 (455) | G2 says the sabotage "COUNT counts a record at offer instead of at write" fails on `check_invariants` inv 7, and point 6's guard is `check_invariants.rs` invariant 7. But `check_invariants.rs` builds its own entries with `PartWriter` and mutates them (PRD-08:346-348); it never runs the recorder's `pipeline/counts.rs`, so it cannot catch a recorder counting change. The fast guard that runs the real pipeline and asserts "COUNT equals a recount" is `pipeline_guard.rs` (PRD-02) | Ledger R6 (locked, written counts); point 6; G2 provability | major | G2 guards: "`pipeline_guard` (the GAP sabotage and the COUNT sabotage), `mask_rules`, `proxy_connect`". Point 6 row: "`pipeline_guard.rs` (COUNT equals a recount of what was written), `check_invariants.rs` invariant 7 (the verifier's recount), `cli_finish.rs` (...)" |
| T14-6 | Section 2, point 2 row (93) | Names `native_capture.rs`, which no PRD builds. PRD-04's tests are `native_pattern`, `native_walk`, `native_tail`, `native_room`, `native_snapshot`, `native_safety`, `native_resume`, `native_finish` (PRD-04 Acceptance). PRD-15 copies this list into README as the promise list "with every file name" (PRD-15:307) | Point 2; AGENTS change 1 (the protected list must name real tests) | major | Replace "`native_capture.rs`" with "`native_tail.rs` (live EOF, truncation, a vanished file gives a GAP), `native_finish.rs` (finish reads everything, no byte cap)" |
| T14-7 | Sections 2 (96), 3 (115), 5 (189-191); interfaces PRD-06 (416) | `upload_worker.rs` (full tier: real client, real Worker under `wrangler dev`, `fetch-run.sh --local`, equal bytes) is assigned to PRD-06 ("owns"), but PRD-06 does not list it in its Acceptance tests or its PRD-14 interface row (PRD-06:482); it appears there only as a deleted old file (PRD-06:496). PRD-06's planner works from PRD-06 and will not build it, so point 5's only real-Worker guard and a `gate full` step select nothing | Point 5; ledger S1 (client and Worker change together) | major | Add an Amendments note to PRD-06: "Automated tests: `upload_worker.rs`, `#[ignore]` (full tier): the real client against `wrangler dev` with its local database, then `scripts/fetch-run.sh --local`; the fetched `rec.gcr.zst` equals the local parts byte for byte." PRD-14 interface row PRD-06 then reads "PRD-06 Amendments" as its source |
| T14-8 | Section 6, rules 3 and 4 (231-239) | PRD-13 section 6 (PRD-13:217-222) has a Worker half: scan `server/worker/src/` and `server/worker/wrangler.toml` for `receipt`, `agent_paused`, `RECEIPT_SIGNING_KEY`, `RUN_MAX_COMPRESSED_BYTES`, and `src/sign.ts` must not exist. PRD-14 keeps only "rule 3's patterns under `server/worker/src/`", only after the cut, and not `wrangler.toml`. `RUN_MAX_COMPRESSED_BYTES` is today's 64 GiB per-run cap (PRD-07:49; present in `chunks.ts`, `context.ts`, `env.ts`, `db.ts`, `errors.ts`, `wrangler.toml`); with PRD-14's text nothing stops it coming back | Ledger R5 (locked: no per-run size limit on the server); L6 (signing cut) | major | Add to rule 3: "Worker half (`server/worker/src/**` and `server/worker/wrangler.toml`; not `sql/`, `test/`, `README.md`, `scripts/check-sql.sh`): `receipt`, `agent_paused`, `RECEIPT_SIGNING_KEY`, `RUN_MAX_COMPRESSED_BYTES` match nothing, and `server/worker/src/sign.ts` does not exist. It joins the gate in the PRD-14 edit right after PRD-07's cut merges (PRD-07 night-log note)." Rule 4 keeps only the reference guard |
| T14-9 | Builder note 1 (513-516); Owner decisions (31-32) | The H+0 gate change is landed "by the integrator ... from this section's text", with no planner or reviewers named, and merge rule 3 ("`gate fast` as it stands on integration") cannot pass because the old gate is red. A conductor may read this as an exemption from the review pipeline | Ledger B2 (locked: at least two independent reviewers read each diff) | major | Add to note 1: "It goes through the merge rule like any merge request: reviewers A and B read the diff (ledger B2). Rule 3 is met by the new gate run on the merged result, because the old gate is red on `docs/plans/` (PRD-00 Today)." |
| T14-10 | Acceptance G1 (452) | Pass is a time line ("Fast under 5 minutes warm, full under 20"), while section 8 (313-316) says the gate never fails on time and PRD-00 says no speed number fails a gate | PRD-00 rule of the shape; clarity | minor | G1 Pass: "Step times, the `release_mask_bench` report and the three slowest tests (JUnit) in the night log. Past an alarm (5 or 20 minutes) the slowest step and test are named and PRD-16 decides. No pass or fail on time." |
| T14-11 | Section 3 budget (110-121) | PRD-02 row says 12 test files; PRD-02 lists 13 (pipeline guard, stall, many_streams, native_room, mask_panic, restart, seal, secure, no_latch; watchdog restart, null, end, rlimit). PRD-01 row omits `zstd_close` (PRD-00:101). PRD-17 row is 1,500 against PRD-17's own "about 1,300" (PRD-17:379). Rust total 142,447 here, 142,413 in PRD-00 | Clarity (the "one source" table) | minor | PRD-02 row "13 `pipeline_*` and `watchdog_*`"; PRD-01 row "`zstd_close`, `golden`, `roundtrip`, `recovery`"; PRD-17 row "1,300 (PRD-17)" and total "about 17,750"; one "today" figure shared with PRD-00 |
| T14-12 | Section 7 (255) | `goalcup-verify check --json --known` has no file; PRD-08 defines `--known <file>` (PRD-08:88) | Builder misread | minor | "`goalcup-verify check --json --known <file with the planted value>`" |
| T14-13 | Section 8 table (294); interfaces PRD-05 (415) | The release step selects test names `^release_`, but PRD-05 names the file `mask_bench.rs` (PRD-05:479, 505). A builder may rename the file, not the function, and the step then selects nothing | Builder misread | minor | "The test function in `mask_bench.rs` is named `release_mask_bench`; a release step that selects no test fails" |
| T14-14 | Section 9 `release.sh` (356-360) | The release check compares `crates/`, `server/worker/src/`, `connectors/`, `Cargo.lock`, not the workspace `Cargo.toml` (profiles, features) or `rust-toolchain.toml`, so a profile or toolchain change after the pad run would not refuse | T8 (core set verified on the tagged binary) | minor | Add "`Cargo.toml` and `rust-toolchain.toml`" to the compared paths here and in PRD-12 section 9 (Amendments note) |
| T14-15 | Section 9 `install.sh` (352-353) | "one PATH line that both exports and appends to the right profile file (H8)": "H8" reads as ledger H8 (session files), and the line can be read as install.sh editing the shell profile, which today it does not (`install.sh:377-389` prints) | Clarity; AGENTS "never write any app-wide config" | minor | "prints one line that, when the participant runs it, exports PATH and appends itself to the right profile file (audit H, item 8). install.sh writes no profile itself." |
| T14-16 | Section 4 rules (139-164); G5 (456) | `release.yml` runs the Rust guards on macOS, but some guards are Linux-only (`pipeline_secure.rs` reads `/proc/<pid>/limits`). Without a rule, G5 fails at the owner's release on a test, not on a bug | T9 (macOS later); G5 provability | minor | Add a rule: "A test that reads `/proc` or calls a Linux-only API is `#[cfg(target_os = \"linux\")]`; a macOS-only case is gated the same way." |
| T14-17 | Section 6 rule 3 (231-234) | Scans only `crates/goalcup*/src/`; PRD-13's guard scans the whole new crates, including `Cargo.toml` (a `test-hooks` feature in any other crate). PRD-14 also says `goalcup-verify` declares no hooks while PRD-13 allows one there | PRD-13 interface | minor | "Rule 3 scans `crates/goalcup*/` (src, tests, Cargo.toml) with PRD-13's patterns, including `test-hooks` declared in any `Cargo.toml` other than `crates/goalcup/Cargo.toml`" (PRD-13 gets the narrower crate list by Amendments note) |
| T14-18 | Section 2 guard list (90-100) | Two locked behaviours have no named guard, so under the new AGENTS rule their tests read as deletable: heartbeat `failed` while recording is off (`upload_beat.rs`, PRD-06) and usage stats (`usage_*`, PRD-09) | Ledger R7, S2 (locked) | minor | Point 6 row adds "`upload_beat.rs` (`failed` while recording is off)"; new row "Usage stats (ledger S2) \| PRD-12 C5 \| `usage_run1.rs`, `usage_join.rs` \| PRD-09" |
| T14-19 | Asks (546-563) | Ask 1 to PRD-16 is already done in PRD-16 (G2 and G5 after merge, the version comment, build boxes only); asks 1 to 3 were ruled in PRD-00 R3-11; PRD-16's ask 3 to PRD-14 was accepted there but not applied (T14-1) | Clarity | minor | Mark asks 1 to 3 "ruled, PRD-00 R3-11, applied there"; add "PRD-16 ask 3: applied (section 8)" once T14-1 is made |
| T14-20 | PRD-16 section 1 hosts (PRD-16:82) and phase 0 step 7 (PRD-16:157) | The build snapshot still holds PostgreSQL; PRD-14 removed `check-sql.sh` and PostgreSQL from the gate (section 8, 300-301; review answer 623) | Less code; clarity | minor | PRD-16 Amendments: drop "PostgreSQL" from both lines |
| BP-1 | BUILD-PROMPT section 1 item 5 (24-26) and section 4 lane table (59-73) | Item 5 says follow PRD-16 "except where this prompt ... says otherwise", so the prompt's lane table wins over PRD-16 section 1, the reverse of PRD-00 R3-10 and the ownership row on BUILD-PROMPT. The tables differ: PRD-05 sits in "Format and core", PRD-14 sits in "Cut and docs" with "the cut only after the lessons tests pass", there is no "Test support and gate" lane, Judge waits "after PRD-01" and Participant "after PRD-02" (PRD-16: H+0), and there is no "Real runs" lane. A conductor following the prompt starts `tests/common/` late (review B M3) and may hold PRD-14's gate change in a lane gated on the cut | Ledger B2 (one workflow per lane; settled by R3-10) | major | Item 5: "Follow it. On lanes, hours and the merge rule PRD-16 wins over this prompt (PRD-00 R3-10); the ledger wins over both." Replace the section 4 table with PRD-16 section 1's 12-row lane table, word for word |
| BP-2 | BUILD-PROMPT section 4 step 5 (90-93) | The replan "updates the remaining PRDs and this plan in a commit". PRD-00 says BUILD-PROMPT is not edited during the build and a replan only appends an "Amendments" note, never rewriting a section a builder works from (PRD-00 ownership rows on BUILD-PROMPT and each PRD; PRD-16 Replan role) | Ledger B2a (locked); three texts disagree on what "the build plan" is | major | Use the B2a text below in all three files |
| BP-3 | BUILD-PROMPT section 2 (36-40) | Says builds happen in sandboxes, but not that they never fall back to this session when sandboxes fail (PRD-16 S8) nor that the gate runs as `ubuntu`, never with `GOALCUP_GATE_ALLOW_ROOT=1` (PRD-16 traps) | Ledger B1 | minor | Use the B1 text below |
| BP-4 | BUILD-PROMPT section 4 step 3, breaker (86-88) | "tries to break it: a run 1 replay (about 40 subagents, ...)" reads as a real replay per diff; PRD-16's breaker uses `pipeline_guard.rs` and the latest pad smoke row and "never runs a real 2-hour replay per diff" | Clarity; less cost | minor | "Core parts get a third reviewer, the breaker, who tries to break it against run 1's shape with `pipeline_guard.rs` and the latest pad smoke row, once per merge request; never a real 2-hour replay per diff." |

## Text for B1, B2 and B2a (BUILD-PROMPT, PRD-16 section 1, PRD-00 "Where the build runs" and "Lanes")

All three files carry the same words, so the conductor, the integrator and every planner read one rule.

**B1.** "Every build, compile, test, gate run and real run happens in Vercel Sandboxes: build boxes (user
`ubuntu`, from the build snapshot, no key or token) and pad boxes (one harness key file, 0600, for one
run). The conductor session writes code, reviews and orchestrates only; it never runs cargo, npm tests or
the gate. If sandboxes fail, builds wait and retry and never fall back to the conductor (PRD-16 S8). The
gate runs as `ubuntu`, never with `GOALCUP_GATE_ALLOW_ROOT=1`. GitHub CI on `v1.0/integration` is an
independent record, not a build host."

**B2.** "One workflow per lane, all lanes running in parallel in this one session, at most 2 agents each.
The lane table is PRD-16 section 1. Per PRD: a planner writes the task list from the PRD, the ledger and
the code; builders work in parallel git worktrees inside the PRD's owned paths and test in build boxes;
at least two independent reviewers read every diff (A: correctness, the seven points, the ledger,
masking before disk; B: code quality and less code), plus a breaker for core parts (PRD-01, 02, 03, 05,
06); fix; merge by PRD-16 section 4. Separate cloud sessions are a fallback only, as PRD-16 section 1
says."

**B2a.** "After each PRD merges, a replan agent reads the review findings, test results and real-run
results, and appends an 'Amendments' note to each remaining PRD it affects and to PRD-16 (the build plan),
with a night-log `decision` line. It never rewrites a section a builder is working from, never edits
BUILD-PROMPT.md, and never changes a locked ledger item: such a conflict goes to the wake list."

## Code facts spot-checked (all in the code at 638bcfd6)

| Fact in PRD-14 | Result |
|---|---|
| `gate.sh` 113 lines; root refusal `:32-39`; version step `:61`; build `:63-65`; probe and fallback `:69-83`; full e2e and `upload_worker` `:85-93`; doctests `:96`; `cargo test` fallback `:97-105`; full-only steps `:107-111`, link check `:110` | Right |
| `.config/nextest.toml` 61 lines, slow period `:7`, priorities `:14-19` | Right |
| `ci.yml` 93 lines; "NOT the gate" `:1-2`; lean rule `:5`; daemon and CLI excluded `:15-16`, `:93`; no secret `:20-21`; scope `:60-75`; link check `:80` | Right |
| `release.yml` 120 lines; tag versus `Cargo.toml` `:55-62`; three-crate build `:69-71`; publish from `:93` | Right |
| `crates/goalcup-daemon/Cargo.toml:12-21, 76-79` (`test-hooks`, self dev-dependency) and the trap text at `:17-19` | Right |
| `Cargo.toml:14-17` version comment, `:40-54` regex at opt-level 3, `:61` daemon profile | Right |
| `third-party-licenses.py:24` `SHIPPED`; `test_install.py:28` protocol fixtures; `install.sh:9`, `:104`, `:377-389`; `smoke-release.sh:39-54` doctor and pin checks | Right |
| Flake sites `m2_proxy_integration.rs:2865-2868` and `upload_worker.rs:191-197` bind port 0 and drop it | Right |
| `check-version.py` fails, all findings in `docs/plans/` | Right (exit 1; 94 findings at 638bcfd6, 95 at 7f9d88a8 as the PRD says; the count moves as plans are edited) |
| `AGENTS.md:76-77` (`cargo fetch` then `--offline`), `AGENTS.md:68-77, 105` | Right |
| `J-tests-gate.md:446-458` (gate times), `:469-476` (flakes, port race), `:420-423` (Worker test time not recorded) | Right |

## Checked and aligned

- Ledger L2, L6, L7: old-crate tests go with the crates; no reader for 0.7 status in `install.sh`; cut
  scripts deleted; no backward compatibility.
- L3, L4: `tests/common/` copies proven helpers on purpose; old crates leave only after PRD-17's lessons
  run in the fast tier.
- R2, R3, R4, R5: guards assert "past N, recorded or a gap, relay goes on"; no test may pin a cap, a
  latch or a failure budget; `watchdog_null`, `pipeline_no_latch`, `pipeline_stall` named (gap in the
  Worker half, T14-8).
- R8 and the AGENTS "record raw" rule: hooks off in the shipped binary, grep in `gate full` and the
  release smoke; `goalcup-format` has no hooks.
- H1, H4: `rule_code.rs` rule 1 fails on a harness id or a model or gateway host in recorder source;
  `e2e_run` asserts the gateway and the provider have the same `conn` shape.
- F2: tests use `window_log` 20, golden and recovery keep 27, matching PRD-01 section 12.
- M1: `mask_rules.rs` guards `url_password` and the left guard; G2 sabotages `url_password`.
- S1: Worker tests in both tiers; the real Worker in the full tier (owner gap T14-7).
- S3: `check-sql.sh` is not a step; no schema change.
- B3: the gate never runs a harness or spends a key; real runs stay in PRD-12 and PRD-16.
- B4: tests use the fake Worker or a local `wrangler dev`; nothing touches staging or production.
- B5, B6: tag after the owner's merge; release order puts the production floor last (G6 wording aside,
  T14-4).
- T8, T9: release check on the core rows plus the Codex kill row; macOS runs at the owner's release (G5).
- Hard rules: private `HOME` and the real-home guard; no base URL set (the upstream hook is test-only);
  nothing under `target/` committed; P1 to P7 replaced by the guard list (AGENTS change 1 quoted exactly as
  PRD-00 writes it).
- PRD-00 agreement: gate change first in phase 0, old crates via `exclude`, no `--exclude` line, PRD-14
  alone edits `gate.sh`, G2 and G5 after merge, test budget total equals PRD-00's 18,000 and 3,350 row.
- PRD-01 agreement: `PartOptions { window_log, rotate_at }`, the 998-byte worked example and its dump as
  files, monotonic record times with wall time from LINK.
- PRD-02, PRD-03, PRD-05, PRD-12, PRD-15, PRD-17 interface rows match those PRDs, except the items above.
- Builds nothing not agreed: the release check, CI on integration pushes and Rust guards on four targets
  serve T8 and point 7 and are listed as owner-overridable defaults.
- Style: no em dash, no model names or ids in PRD-14.
