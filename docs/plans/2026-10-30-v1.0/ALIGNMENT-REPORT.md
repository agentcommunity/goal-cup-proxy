# Alignment report: goalcup 1.0 plan against the ledger

Final check, 30 Oct 2026. Read: OWNER-DECISIONS.md (the ledger), VISION.md, AGENTS.md, BUILD-PROMPT.md, PRD-00
to PRD-17, the 27 review files in `alignment/`, and the three owner lists of the alignment pass. Review only:
no PRD, product file or ledger line was edited by this check. This file is its one output.

## Verdict

**Go.** Every ledger item is built somewhere in the PRD set, and every blocker the alignment reviews raised
(16 of them) is applied in the file that owns it. The R3-10 conflict is settled the way the ledger says:
BUILD-PROMPT, PRD-16 and PRD-00 now agree on parallel lane workflows in one session, at least two independent
reviewers per diff, a replan after each PRD merge, and builds, tests and real runs only in Vercel Sandboxes.
The folder has no em dash and no model name or id. Four ledger lines are built on a reading that waits for
the owner's yes (the watchdog's jobs, one test-only build feature, two Claude Code switches, and the
"never commit" wording). Each has a safe default, so none stops the build. Ten owner questions are below;
the two worth answering before the paste are 1 (exact-value masking, which changes what the second slice
builds) and 2 (the left guard).

Updated 30 Oct: ledger R3a, R3b and T4 added and carried into PRD-02, 08, 10, 12, 16.

## 1. Ledger coverage

Verdicts: **covered** (built as the ledger says), **covered, reading** (built on a reading that waits for the
owner, with a safe default), **partly**, **missing**. Section names are the PRD headings as they stand now.

| Item | Verdict | Where it is built |
|---|---|---|
| L1 goalcup 1.0, one big build | covered | PRD-00 Goal, "What 1.0 is"; PRD-10 s11 "Version 1.0"; PRD-16 s14; BUILD-PROMPT intro ("Do not stop at a small update") |
| L2 seven points, less code | covered | PRD-00 "The seven points, mapped to PRDs", "Size budget"; every PRD "Points served"; BUILD-PROMPT s4 reviewer B |
| L3 fresh core, copy-ins, old crates deleted in the same build | covered | PRD-00 "File ownership"; PRD-03 s4, s5, s7, s8 (CA, TLS, HTTP/1.1, brotli, WebSocket deflate copied); PRD-05 s2 (rules and corpus); PRD-11 s6; PRD-07 s1 (chunk store kept); PRD-13 s4 copy ledger, s5; PRD-16 s5 and S7 (late gate calls the owner under W6, cut lands before `REPORT_AT`) |
| L4 lessons list before any deletion | covered | PRD-17 s2.1 to s2.17 (now with format and recovery, verifier, start and status rows, and DECISIONS entry by entry), Acceptance (deletion gate); PRD-13 s5; PRD-00 done item 3 |
| L5 one process, no daemon hand-off | covered | PRD-00 "Architecture" (rules of the shape); PRD-02 s1, s7; PRD-13 group A; PRD-17 s2.11 |
| L6 all planned cuts | covered, reading | PRD-13 groups A to G, s6 no-return guard; PRD-00 "Not in this build". Reading: one `test-hooks` build feature in `crates/goalcup` only, never shipped (PRD-00 owner question 12; PRD-14 s5; question 4 below) |
| L7 no backward compatibility | covered | PRD-01 "What is cut" (no v5 reader); PRD-07 s6 (1.0.0 floor); PRD-08 s6; PRD-16 s14 |
| R1 record everything to model hosts | covered | PRD-03 s1 to s9; PRD-04 s2 to s7. Accepted gaps named, not recorded: loopback local model servers (`local_model` note, PRD-01 s11) and session files of a harness started as another harness's subagent (PRD-11 s8). Question 8 |
| R2 never block or slow | covered | PRD-02 s7; PRD-03 s1 (relay first); PRD-06 s1 (plain threads off the relay) |
| R3 recorder never stops for good | covered, reading | PRD-02 s8, s9 (same port, restart at once, `recorder_down` gap, no restart cap); PRD-00 "Architecture". Reading: the watchdog is the start process with launcher jobs (PRD-00 question 9; question 3 below) |
| R3a eager crash-loop retry, never ending | covered | PRD-02 s8 steps 8 and 9 (restart at once; null mode only after 3 crashes in 10 minutes, each within 60 s of its start; retries after 30 s, 1, 2, then every 5 minutes, never ending; count resets after 60 s up); PRD-00 "Architecture"; PRD-15 D75; PRD-16 header |
| R3b `goalcup recover` when the watchdog died | covered | PRD-02 s8.1, s9.1 and R9 (same port, same CA from `rec/run-ca.key`, 0600, removed at the seal; harness never restarts); PRD-10 `goalcup recover` and the both-dead status lines; PRD-12 `kill-watchdog-recover` fault; PRD-16 A2; PRD-00, PRD-01, PRD-03, PRD-15 and PRD-17 key-file text |
| R4 no silent loss | covered | PRD-01 s7, s13 (invariant 12); PRD-03 s9, s11; PRD-04 s4, s8; PRD-08 s3 |
| R5 no caps of our own | covered | PRD-02 s3 (memory guards write a gap); PRD-07 s5 (413 and per-run limit deleted); PRD-13 group B; no whole-build spend cap (PRD-16 s9) |
| R6 written counts | covered | PRD-01 s9; PRD-02 s6; PRD-06 s6; PRD-07 s4; PRD-10 s4, s5 |
| R7 heartbeat says `failed` | covered | PRD-06 s6; PRD-07 s4; PRD-00 done item 8 |
| R8 record raw, interpret later | covered | PRD-02 s1; PRD-09 s1 (usage after upload); PRD-14 s6 `rule_code.rs` |
| H1 one proxy | covered | PRD-03 Goal, s1; PRD-13 s3 |
| H2 one shared host list | covered | PRD-11 s4 (`connectors/model-hosts.toml`); PRD-03 s3 |
| H3 other hosts relayed as tunnels | covered | PRD-03 s9 (`long_tunnel` flag) |
| H4 gateways are another HTTPS host | covered | PRD-03 "Answer to the owner's gateway question"; PRD-11 owner's question section; PRD-12 s2 gateway case |
| H5 proxy dodgers | covered | PRD-03 s1, s4 (pinned and refused fall back to a tunnel with a named gap); PRD-04 s10; PRD-12 s2 dodger case |
| H6 HTTP/2 | covered | PRD-03 s6; PRD-00 done item 9 |
| H7 per-harness data only | covered, reading | PRD-11 s2, s3, s6. Reading: Claude Code keeps `CLAUDE_CODE_RETRY_WATCHDOG=1` and `CLAUDE_CODE_DISABLE_AGENT_VIEW=1` (PRD-11 "Overridable defaults"; question 5) |
| H8 session files, no caps, path is the ID | covered | PRD-04 s2 to s8 |
| H9 auto-updates off, version recorded | covered | PRD-11 s5, s6; PRD-10 s7; PRD-12 s8 (`harness_version`) |
| H10 vendor hosts open by default | covered | PRD-11 s4 `vendor` list |
| F1 new record format | covered | PRD-01 s1 to s9 |
| F2 zstd long window | covered | PRD-01 s12; PRD-06 s2 (window 2^27 and long matching locked; only the level may move) |
| M1 masking | covered | PRD-05 s2 (rules), s4 (left guard `[A-Za-z0-9_]`), s5 (URL password), s8 (email and the four Codex plan, quota and credit rows dropped), s1 (before disk). Exact values beyond `GH_TOKEN` not built without a yes (question 1) |
| U1 upload | covered | PRD-06 s4, s5 (backoff capped at 120 s, reset when the offset moves), s7 (prune) |
| S1 verifier and Worker move together | covered | PRD-07 s12; PRD-08; PRD-00 done item 7 |
| S2 usage and research stats | covered | PRD-09 s3 to s7 |
| S3 no schema migration | covered | PRD-07 s10, s11; PRD-09 s7 (`goalcup-usage-staging` in R2) |
| T1 15 harnesses | covered | PRD-11 s6, s7; PRD-12 s6 |
| T2 keys | covered | PRD-12 s7; PRD-11 s7; BUILD-PROMPT s3 |
| T3 tiers | covered | PRD-12 s9, s13 |
| T4 test pad, three cases, one table | covered | PRD-12 s1, s2, s8 (C1 to C6), s9, s11; the proxy record is the proof: no unexplained gap and the history check (PRD-08 s3.2), local session files only an optional extra, no outside API or provider usage lookup (PRD-00 done items 1 and 2, PRD-02, PRD-09, PRD-16, BUILD-PROMPT) |
| T5 20-minute payments workload | covered | PRD-12 s5; BUILD-PROMPT s5 |
| T6 10 USD per run | covered | PRD-12 s7 (checked every 5 minutes, seal at 10 USD); PRD-16 s9; PRD-00 "Not in this build" |
| T7 vendor docs first | covered | PRD-11 "Builder notes"; BUILD-PROMPT s4 step 2 |
| T8 core set blocks release | covered | PRD-00 done item 1; PRD-12 s9, s13 |
| T9 Linux now, macOS later | covered | PRD-12 Goal; PRD-00 "Defaults taken" |
| B1 builds only in Vercel Sandboxes | covered | BUILD-PROMPT s2; PRD-16 s1 (never falls back to the conductor), S8; PRD-00 "Where the build runs"; PRD-14 s8 |
| B2 parallel workflows, planner, builders, two reviewers | covered | BUILD-PROMPT s4; PRD-16 s1, s4; PRD-00 "Build order", "Branches and the merge rule" |
| B2a replan after each PRD | covered | BUILD-PROMPT s4 step 5; PRD-16 s1 Replan row; PRD-00 ownership row `BUILD-PROMPT.md` |
| B3 short tests early, mass testing at the end | covered | BUILD-PROMPT s5; PRD-16 s3, s6, s10 (S1 one cause, S2 reorders never drops, S3) |
| B4 staging only | covered | BUILD-PROMPT s2; PRD-16 "Owner decisions", s8 |
| B5 agents deploy staging only | covered | PRD-16 s3 first staging deploy, s14; PRD-07 s12 |
| B6 integration branch only | covered | PRD-00 "Branches and the merge rule"; BUILD-PROMPT s2 |
| B7 morning start | covered | BUILD-PROMPT intro; PRD-00 "Build order" |
| B8 autonomy | covered | PRD-16 s8, s11 (W1 to W7); BUILD-PROMPT s7 |
| s7 never set a model base URL | covered | PRD-00 "AGENTS.md: what stands"; PRD-11 (Hermes recorder base URL removed); PRD-12 gateway case set as a participant would |
| s7 records, never manages; D65 out, D66 stays | covered | PRD-00 "AGENTS.md"; PRD-13 (K5); PRD-10 s10; PRD-15 s4 |
| s7 no harness names in core code | covered | PRD-14 s6 `rule_code.rs` |
| s7 never write into real harness homes | covered | PRD-11 s5; PRD-15 s4 |
| s7 pins go, connectors embedded | covered | PRD-13 (K17); PRD-11 s1 |
| s7 never commit `spikes/*/out/`, `harnesses/`, `target/` | covered, reading | PRD-15 s4 rewrites the dead `spikes/` path to root `/pad-out/` (owner list G3-O4; question 10) |
| s7 record raw, reworded | covered | PRD-15 s4; PRD-00 "AGENTS.md" |
| s7 promise tests become the seven points | covered | PRD-00 "AGENTS.md" change 1; PRD-14 s2 |
| s8 out of scope | covered | No email code change (PRD-07 s12 note), no terminal capture, no agent production deploy (PRD-16 s14), no schema change, no OpenRouter, Amp, Factory keys ("not verified: no key"), Cursor and Aider out |
| O1 staging shares the database | covered | Read-only role on the conductor only (PRD-00 "Where the build runs"; PRD-07 s9) |
| O2 Copilot with the DeepSeek key | covered | PRD-11 s7; PRD-12 s7 (try the key, "not verified" if it fails) |

## 2. The R3-10 conflict

Settled per ledger B1, B2 and B2a. The three texts agree:

- **Parallel workflows in one session.** BUILD-PROMPT s4 (lines 84-88) starts PRD-16's twelve lane workflows in
  parallel; PRD-16 s1 (lines 91-96) says the same and that its table replaces BUILD-PROMPT's older one;
  PRD-00 "Lanes" (lines 532-553) copies the same twelve lanes. Separate cloud sessions are a fallback only
  in all three.
- **At least two independent reviewers.** BUILD-PROMPT s4 step 3 (lines 104-124); PRD-16 s1 Reviewers row
  and s4 item 4 (lines 228-243); PRD-00 "Branches and the merge rule" and "Lanes". Each reviewer is a fresh
  agent that writes before seeing the other's findings. Integrator code changes go through reviewers too
  (BUILD-PROMPT lines 90-93).
- **Replan after each PRD.** BUILD-PROMPT step 5 (lines 126-132), PRD-16 s1 Replan row, PRD-00 ownership row
  for `BUILD-PROMPT.md` (line 356): Amendments notes on the remaining PRDs and on PRD-16, which with PRD-00
  "Build order" is the build plan. BUILD-PROMPT is edited before H+0 only.
- **Vercel-only builds.** BUILD-PROMPT s2 (lines 45-56); PRD-16 s1 hosts table and S8; PRD-00 "Where the
  build runs". CI on GitHub's runners is named a record, not a build host (PRD-16 s4).
- PRD-00 R3-10 (line 968) now says the ledger wins over all three and that BUILD-PROMPT's per-PRD pipeline
  stands.

## 3. Em dash and model names

- Em dash character: **0** in all 47 files of the folder (20 plan files and 27 review files).
- Model names or ids: **0.** The matches are connector ids (`claude-code-cli`, `claude-desktop`), a host
  (`chatgpt.com`), the ChatGPT sign-in, key names ("Claude key", as the ledger writes it) and harness names.
  "The cheapest current OpenAI model" is the only model reference, and it names none.

## 4. Blockers left

None. Minor items a builder will not trip on, each with its fix, for the conductor's first replan or the
owner:

| id | Where | What | Fix |
|---|---|---|---|
| AR-1 | PRD-16:122, Builder row | Stray punctuation "merge request;, runs its tests" | Make it "merge request; runs its tests" |
| AR-2 | PRD-15:268 against owner-g3 G3-O4 | PRD-15 calls the new "never commit" wording "default yes"; the owner list gives the no-answer default as "keep the text and add `pad-out/` beside it" | Owner question 10 settles it; until then the build may use PRD-15's wording, since `spikes/` is deleted and the old path names nothing |
| AR-3 | PRD-12:904 (review history) | A history row still calls 400 USD a build stop line | None needed: the section opens with "the body wins", and the body (PRD-12 s7, PRD-16 s9) has no build cap |
| AR-4 | Owner lists g1 OW-02 and g2 O-1 | They recommend opposite left guard classes | Merged as question 2 below, one recommendation |
| AR-5 | 17 PRDs, X2-11 | Review history stays inside each PRD; no `REVIEW-HISTORY.md` | Leave it: each history section opens with "the body, the ledger and these answers win" |

## 5. What changed in this review

The alignment pass applied the fix lists g1, g2 and g3 to each file and logged each one in the file's
"Alignment answers" section. Counts from those sections:

| File | Fixes logged | Applied | Partly | Rejected or not changed |
|---|---|---|---|---|
| BUILD-PROMPT.md | 43 | 40 | 3 | 0 |
| PRD-00 index | 69 | 67 | 2 | 0 |
| PRD-01 record format | 36 | 34 | 2 | 0 |
| PRD-02 recorder core | 61 | 55 | 6 | 0 |
| PRD-03 proxy | 27 | 24 | 2 | 1 |
| PRD-04 native capture | 25 | 24 | 1 | 0 |
| PRD-05 masking | 30 | 28 | 2 | 0 |
| PRD-06 uploader | 32 | 31 | 1 | 0 |
| PRD-07 Worker | 25 | 21 | 4 | 0 |
| PRD-08 verifier | 20 | 19 | 1 | 0 |
| PRD-09 usage | 25 | 23 | 2 | 0 |
| PRD-10 CLI | 37 | 31 | 5 | 1 |
| PRD-11 connectors | 43 | 41 | 2 | 0 |
| PRD-12 test pad | 47 | 43 | 2 | 2 |
| PRD-13 cuts | 24 | 21 | 3 | 0 |
| PRD-14 tests and gate | 28 | 24 | 3 | 1 |
| PRD-15 docs | 30 | 27 | 2 | 1 |
| PRD-16 orchestration | 77 | 69 | 5 | 3 |
| PRD-17 lessons | 21 | 18 | 3 | 0 |
| **Total** | **700** | **640** | **51** | **9** |

OWNER-DECISIONS.md and VISION.md are unchanged. Seventeen of the 51 "partly" rows are the same item (X2-11,
above). Each rejection gives a reason tied to the ledger or a later ruling; none drops a locked item. All 16
blockers from the reviews (L2-01, L4-1, L5-01, L6-05, L6-06, L6-09, P00-1, P01-1, P04-1, P05-1, P05-2, P09-1,
P16-1, T14-1, X3-1, X3-2) are applied, some through their merged fix ids (G2-01, G2-20, G2-22, G3-X5,
G3-X9). This final check added only this file.

## 6. Owner questions

Merged from owner lists g1 (12 items), g2 (6) and g3 (5). PRD-00's own questions 1, 2 and 4 to 8 keep their
defaults and stay in PRD-00 "Owner questions"; question 6 there (run 1's prompt) is a morning step below.
A ledger change from a yes goes into OWNER-DECISIONS.md by your hand: agents never edit it.

| # | Question | Recommended answer | If no answer, the build |
|---|---|---|---|
| 1 | Mask more than M1 lists? Exact bytes of credential-named values from the launch environment, workspace `.env` files and the harness's key files, plus Stripe test keys and `whsec_` webhook secrets. (PRD-00 q3; OW-01, O-2) | Yes. Add it as M1a before the paste. It masks only known secret bytes, and `cat .env` is the biggest real leak path (point 4) | Builds M1 as written; only `GH_TOKEN` is masked by exact value; the pad plants no Stripe or canary values; `MORNING.md` says "not built" |
| 2 | Masking details: (a) left guard class `[A-Za-z0-9_]` or also `-`? (b) keep masking headers named `ratelimit`? (c) "drop the Codex rate-limit rules" means the four plan, quota and credit rows only? (PRD-00 q10, q11; OW-02 to OW-04, O-1) | (a) `[A-Za-z0-9_]`: a key after `-` is a real leak, a false mask inside a blob only stars a few bytes. (b) Yes, keep them masked. (c) Yes | Exactly these three answers |
| 3 | The "tiny watchdog" is the `goalcup start` process. Beside the restart loop it holds the port and run CA, runs helper stops and starts and ends the harness. It never touches a recorded byte. OK? (PRD-00 q9; OW-05, O-4) | Yes. These are the launcher's existing jobs; splitting them adds a third process | Builds it this way |
| 4 | Keep one test-only `test-hooks` build feature in `crates/goalcup`, off by default and never in a release build, though L6 cut the test seams? (PRD-00 q12; O-3) | Yes. Restart, guard and rotation cannot be proven in the gate's time without it; the gate checks the shipped binary has none | Builds it |
| 5 | Claude Code keeps `CLAUDE_CODE_RETRY_WATCHDOG=1` (rides over a recorder restart) and `CLAUDE_CODE_DISABLE_AGENT_VIEW=1` (background agents would call around the recorder), said on its first screen? (OW-06, G3-O3) | Yes to both | Both on, with the first-screen line |
| 6 | With 1.0 the unchanged email cron sends the `recording` email when a heartbeat says `failed`. Wanted? (OW-12) | Yes. It is what R7 means, with no code change. Confirm before the production deploy | No email code change; noted in `MORNING.md` |
| 7 | Proxy: answer `405` to a non-CONNECT request, and write one `connect_failed` note per host per minute when an upstream connect fails? (OW-09) | Yes to both: a few lines, and the record shows a host the harness could not reach | `405`; a failed connect is a log line only |
| 8 | Trim for 1.0: cut the sleep lock (about 150 lines; tell people to keep the machine awake), and accept as out of scope: loopback local model servers, session files of a harness started as another harness's subagent, OpenClaw's Gateway daemon, a heartbeat-age flag for silent entries. (O-5; OW-07, OW-08, OW-10, OW-11) | Yes to all. Add the four to ledger section 8 | Keeps the sleep lock (PRD-10 s9); the four stay unbuilt, named as notes or "not verified" |
| 9 | Two credentials: (a) split Cloudflare into a staging deploy token and an R2 read-only token? (b) create a throwaway GitHub owner and a fine-grained push token (Contents read and write) for pad repos? (G3-O1, G3-O2) | Yes to both. A Workers edit token may deploy production, and `GH_TOKEN` must never enter a pad box | One Cloudflare token, used only for staging deploys, bucket creation and R2 reads; pad rows run without a repo and say "push not tested" |
| 10 | Two wording rules: (a) the hard rule "never commit" names root `/pad-out/` instead of the deleted `spikes/*/out/`; (b) the commit attribution trailer is the one exception to "no model names in commits". (G3-O4, G3-O5) | Yes to both | (a) PRD-15's wording; (b) the trailer stays |

## 7. Morning steps

1. **Answer the questions above**, or let the defaults stand. Write any yes that changes the ledger (questions
   1, 2 and 8) into OWNER-DECISIONS.md and push it to `smart/intelligent-newton-0avrpi` before the paste.
2. **The two staging values.** Paste this into a local agent on your machine, in your goal-cup-proxy checkout,
   with wrangler signed in:

   > Set up staging test approval for the goalcup 1.0 build. Work in `server/worker`. Never touch the
   > production Worker: every wrangler command carries `--env staging`. Never print a secret.
   > 1. Make `~/goalcup-staging` with mode 0700. Generate a random key with `openssl rand -hex 32` straight
   >    into `~/goalcup-staging/approval-key` (mode 0600). Do not echo it.
   > 2. Set it on staging only: `wrangler secret put TEST_APPROVAL_KEY --env staging < ~/goalcup-staging/approval-key`.
   > 3. Find my member id: the uuid of my website account. Use a read-only query (`BEGIN READ ONLY` ...
   >    `ROLLBACK`). Write it to `~/goalcup-staging/member-id` (mode 0600).
   > 4. In the same read-only way, check that this member has accepted the current Goal Cup terms
   >    (`goal_cup_tracer.has_current_terms`). If not, tell me to accept them on the website.
   > 5. Prove the route: `POST <staging URL from wrangler.toml>/v1/device/approve` with header
   >    `Authorization: Bearer <the key, read from the file, never printed>` and body
   >    `{"user_code":"<a made-up code in the shape normalizeUserCode in src/device.ts accepts>","member_id":"<id>","approve":false}`.
   >    The answer must be "No such user code", not a wrong key or a missing route.
   > 6. Report only: the two file paths, the first 8 characters of the member id, the terms result and the
   >    route result.

   Then put the two files' contents into the cloud environment secrets as `GOALCUP_TEST_APPROVAL_KEY` and
   `GOALCUP_TEST_MEMBER_ID`.
3. **Keys** in the cloud environment secrets (BUILD-PROMPT s3): `CLAUDE_API_KEY`, `OPENAI_API_KEY`,
   `DEEPSEEK_API_KEY`, `GEMINI_API_KEY` (the funded one), `AI_GATEWAY_API_KEY`, `VERCEL_TOKEN` and the team id,
   `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID` (or the split pair, question 9), `DATABASE_URL` (read-only
   role), `GH_TOKEN`. Optional: the pad push token (question 9) and run 1's prompt in a file on the conductor
   before phase 6, so A1 repeats it word for word (PRD-00 question 6).
4. **Paste.** Open a new Claude Code cloud session on `agentcommunity/goal-cup-proxy` and paste
   BUILD-PROMPT.md from the first `---` line to the second. Do not paste "Alignment answers".
5. **Later, on your Mac** (T9): in the macOS run, ask the agent once for at least 100 subagents and read the
   `fd_limit` note and the status line (owner list O-6).
