# Lane J: Worker hardening and server notes
**Wave:** 1   **Depends on:** none   **Branch:** fix/server
**Covers findings:** cr-upload-server-1, -4, -5, -6, -7, -8, -10, -11, -12; docs-6; MULTI-VERSION-PLAN task 9 (server notes); `abandoned` status (arch, REVIEW-REPORT Plan 5). Not mine: -2 (refuted), -3 (lane B), -9 (client PUT bound).
**Owns files:** `server/worker/**` (except the window-refusal text in `db-local.ts`, lane A), `docs/UPLOAD-API.md`, `server/worker/README.md`, `server/worker/scripts/*`, new `crates/goalcup/src/cli/notes.rs`.
**Shared files (only these parts):**
- `crates/goalcup/src/cli/approve.rs` (lane A): struct `Open` (line 198-204) gains one field; `begin()` gets one call right after `let open: Open = ...` (line 143). Nothing else.
- `crates/goalcup/src/cli/continue_run.rs`: struct `Next` (line 335-339) gains one field; `prepare()` gets one call after the `is_id` check (line 233-238); the JSON body at line 207 gains `"os"`.
- `crates/goalcup/src/cli/mod.rs`: one line `pub mod notes;` in the module list (line 3-17).
- `scripts/gate.sh`: one conditional step after line 82 (`npm --prefix server/worker test`), Task J.8 only.
- `docs/OPERATING.md`: lines 130-132 only (Task J.9).
**Produces (for other lanes):** wire field `notes` (array of at most 5 strings) in the 200/201 answers of `POST /v1/runs` and `POST /v1/runs/{id}/continue`; optional request field `os` (string) on continue; file `server/worker/src/advisories.json`; `crates/goalcup/src/cli/notes.rs` with `pub struct Notes` and `Notes::print(&self, out: &mut dyn Write)`. Lane F/I: the Worker matches versions with the exact and numeric-prefix grammar only, the same as `version.rs` `matches` for "0.22.6" and "0.22".
**Consumes:** none (no Rust function from lane F: the Worker parses the version itself).
## Dropped
- cr-upload-server-1, rate-limit half: a credential belongs to one approved member and one run; the unbounded part is bytes, which J.5 (practice ceiling) and J.6 (alert) cover. A per-credential limiter would add a Durable Object call to every chunk, on the path of point 3.
- cr-upload-server-7, real fix: the credential is minted at pickup and only its hash is stored, so it cannot be sent twice. Reword only (J.4).
- `abandoned` status: stays readable (`db.ts:19`, `db-supabase.ts:45`, `runs.ts:69`). Cutting it makes a position call on any pre-1.0 `abandoned` row a 502. Owner query first (question 3); if it returns 0 on production and staging, delete those three spots and `local-db.test.ts:108`.
- `build` / `server_time` always-null fields: not in `server/worker/src` or `UPLOAD-API.md` (grep found none); nothing to cut here.
- `harness_version_word` and `notes_sent` stored on the entry (MULTI-VERSION-PLAN, Changes): both need a website migration and serve no rule here. The raw `harness_version` is already stored (`runs.ts:144`).
## Review focus
1. A note full of escape codes, bidi marks, or one 5 MB string: stripped and capped on the client (J.11), shape-checked and capped in the file test (J.10).
2. `notes` of the wrong type in an answer must not fail an open or a continue (point 3): J.11 `wrong_shapes_are_ignored`.
3. Two sessions committing the same chunk at once on real Postgres: exactly one commits (J.8).
4. A Worker deployed before the pending migration lands: the purge step fails, the other cron steps still run (J.7).
5. The practice ceiling must not touch a competition run, and a 410 on a practice chunk must not stop recording (client backs off and retries on 410, `UPLOAD-API.md:40`) (J.5).

Order: J.1, J.2, J.4, J.8, J.9, J.10, J.11 are PARALLEL-OK. J.3 then J.5 (both `chunks.ts`). J.6 then J.7 (both `notices.ts`, `env.ts`). J.8 before J.7 (J.7 adds a scenario). J.12 last (all doc lines).

### Task J.1: Cut the dead `dev` field   (size S, PARALLEL-OK)
**Files:** Modify `server/worker/src/runs.ts` (comment at 96, `DEV_COMPETES` 99, `requireDevAllowed` 125-128, `const dev` 192, line 204, the block 255-260; drop the `boolField` import only if line 7-22 shows no other use); Modify `server/worker/test/lifecycle.test.ts` (describe at 522: delete the five `dev` tests and "the version answer comes before the dev answers"; keep the version-floor tests).
**Interfaces:** Consumes none. Produces none; a request that still carries `dev` is ignored.
- [ ] Step 1: In `lifecycle.test.ts` add inside the version describe:
  ```ts
  lifecycle("a body that still says dev is ignored", async (r) => {
    const alice = await r.cred("alice");
    assert.equal((await r.open(alice, "s1", { dev: true })).status, 201);
    const b = await r.start(await r.cred("bob"), "s2");
    assert.equal((await r.cont(await r.cred("bob"), b.run_id, b.entry_id, "s3", { dev: "yes" })).status, 201);
  }, WINDOW);
  ```
  (Use the third argument the neighbouring tests use for their vars; WINDOW if they pass none.)
- [ ] Step 2: `cd server/worker && node --test --test-name-pattern="still says dev" test/lifecycle.test.ts`. Expect FAIL: 403 "A --dev run never enters the competition" on the first assert.
- [ ] Step 3: Delete the code listed under Files in `runs.ts`; delete the five old dev tests.
- [ ] Step 4: `cd server/worker && npm test`. Expect PASS (typecheck catches any leftover reference).
- [ ] Step 5: `git add server/worker/src/runs.ts server/worker/test/lifecycle.test.ts && git commit -m "worker: cut the dead dev field (D84)"`

### Task J.2: Cut the `disk_full` producer; shape-check the credential row   (size S, PARALLEL-OK)
**Files:** Modify `server/worker/src/notices.ts` (line 151-153, the comment and the push), `server/worker/src/db.ts` (TickRun field, 344-348), `server/worker/src/db-supabase.ts` (TICK_SHAPE line 88; `credential()` line 208-211; add `CRED_SHAPE` next to `TICK_SHAPE`), `server/worker/src/db-local.ts` (line 642 only, not the window text), `server/worker/test/supabase.test.ts` (296-310 and line 320 `disk_notice_at: null`; keep test 312: `disk_full` stays in `NOTICE_KINDS`, because the website SQL still defines its text and an old queued row must still be mailed).
**Interfaces:** `CRED_SHAPE: Record<string, Check>` = `member_id: text, run_id: nullable(RUN_ID test), mode: oneOf("competition","practice","test"), expires_at: isTime`. `credential()` returns `this.#row("upload_credential", v, "credential", CRED_SHAPE)` for a non-null `v`.
- [ ] Step 1: In `supabase.test.ts` replace the "disk_notice_at comes through" test with two tests: `a tick run needs no disk_notice_at` (stub `[{ ...tickRun }]` built without the key, expect `runsForTick` to return it) and `credential row with a wrong mode is a 502` (stub `upload_credential` returning `{ member_id: "m", run_id: null, mode: "banana", expires_at: "2026-11-06T10:00:00Z" }`; `assert.rejects(db().credential(H("a"), T), (e) => e instanceof ApiError && e.status === 502)`).
- [ ] Step 2: `cd server/worker && node --test --test-name-pattern="needs no disk_notice_at|wrong mode" test/supabase.test.ts`. Expect FAIL: first rejects with 502 (field required), second resolves with the bad row.
- [ ] Step 3: Make the edits under Files.
- [ ] Step 4: `cd server/worker && npm test`. Expect PASS.
- [ ] Step 5: `git add server/worker && git commit -m "worker: drop the disk_full producer, check the credential row"`

### Task J.3: A stored-chunk retry must match sha256, prev and first_seq   (size S, PARALLEL-OK with J.1/J.2, before J.5)
**Files:** Modify `server/worker/src/chunks.ts` line 48 (the `held` test) and line 100-104 (the `conflict` branch: the same test; make one local function `sameChunk(c: ChunkRecord, h: ChunkHeaders): boolean` used by both); Modify `server/worker/test/integration.test.ts` test "the same offset with a different length or head is 409" (line 261).
**Interfaces:** `sameChunk` compares `length`, `head`, `sha256`, `prev`, `first_seq`, `last_seq`.
- [ ] Step 1: In that test add three asserts, each `assertError(await put({ body: c1.body, headers: { ...c1.headers, <h>: <v> } }, 0), 409, "conflict")` with `["goalcup-sha256", "e".repeat(64)]`, `["goalcup-prev", "9".repeat(64)]`, `["goalcup-first-seq", "1"]` (the last with `goalcup-last-seq` set to `String(c1.lastSeq + 1)`).
- [ ] Step 2: `cd server/worker && node --test --test-name-pattern="different length or head" test/integration.test.ts`. Expect FAIL: status 200 on the sha256 case (stored answer returned).
- [ ] Step 3: Implement `sameChunk`; keep the existing test "a retry of the same chunk is 200 ... body unread" green (it sends other bytes under the same headers, so the header sha256 still matches).
- [ ] Step 4: `cd server/worker && node --test test/integration.test.ts`. Expect PASS.
- [ ] Step 5: `git add server/worker/src/chunks.ts server/worker/test/integration.test.ts && git commit -m "worker: a stored-chunk retry must match sha256, prev and seqs"`

### Task J.4: Say what a lost poll reply costs   (size S, PARALLEL-OK)
**Files:** Modify `server/worker/src/device.ts` line 103-104 (the `expired` message); Modify `server/worker/test/device.test.ts` (the test that reaches `expired`; find it with `grep -n "expired\|picked" server/worker/test/device.test.ts`); doc line in J.12.
**Interfaces:** message becomes `This sign-in request has expired, or its answer was already collected. If goalcup did not get it, run the command again to start a new sign-in.`
- [ ] Step 1: In the device test that polls twice after approval, assert `second.body.message === "This sign-in request has expired, or its answer was already collected. If goalcup did not get it, run the command again to start a new sign-in."`.
- [ ] Step 2: `cd server/worker && node --test --test-name-pattern="picked up|again" test/device.test.ts`. Expect FAIL: old text.
- [ ] Step 3: Change the string.
- [ ] Step 4: `cd server/worker && node --test test/device.test.ts`. Expect PASS.
- [ ] Step 5: `git add server/worker/src/device.ts server/worker/test/device.test.ts && git commit -m "worker: say what a collected sign-in means"`

### Task J.5: A ceiling on practice runs   (size S, after J.3)
**Files:** Modify `server/worker/src/context.ts` (Config gets `practiceMaxBytes: number`, read near line 108 with `positiveNumber("PRACTICE_MAX_BYTES", env.PRACTICE_MAX_BYTES, 1073741824, "bytes", 2 ** 40)`); Modify `server/worker/src/env.ts` (`PRACTICE_MAX_BYTES?: string`); Modify `server/worker/src/chunks.ts` (after `requireOpen`, line 53: one check); Modify `server/worker/wrangler.toml` (`[vars]` and `[env.staging.vars]`: `PRACTICE_MAX_BYTES = "1073741824"`, with a comment: compressed bytes a practice run may store; competition and test runs have none); Modify `server/worker/test/lifecycle.test.ts` (next to "no per-run size limit").
**Interfaces:** `if (run.mode === "practice" && run.stored_bytes + contentLength > ctx.cfg.practiceMaxBytes) throw closed("This practice run has reached its upload limit.")`. Test runs get no ceiling (organizers run real agents for hours); J.6 alerts on them.
- [ ] Step 1: Add `lifecycle("a practice run stops taking chunks at its ceiling; a competition run does not", ...)` with vars `{ ...WINDOW, PRACTICE_MAX_BYTES: "1000" }`. For `practice` and `competition` runs: `commitChunk` a held chunk (as in the neighbouring test) with `stored_bytes: 1000`, then PUT `makeChunk(entry, 5, 3, head, "x")` at offset 10. Assert practice: `410`, `error "closed"`; competition: `200`.
- [ ] Step 2: `cd server/worker && node --test --test-name-pattern="practice run stops" test/lifecycle.test.ts`. Expect FAIL: practice answers 200.
- [ ] Step 3: Implement.
- [ ] Step 4: `cd server/worker && npm test`. Expect PASS.
- [ ] Step 5: `git add server/worker && git commit -m "worker: a storage ceiling on practice runs"`

### Task J.6: A storage alert from the cron   (size M, before J.7)
**Files:** Modify `server/worker/src/notices.ts` (new exported `alertStorage(ctx, now)`; add `["alertStorage", alertStorage]` to the step list in `runTick`, line 205); Modify `server/worker/src/mail.ts` (new exported `sendAlert(ctx, subject, text)`: the Resend call body of `sendNotices` is the model; to `ALERT_EMAIL`; staging sends to `NOTICE_RECIPIENT_OVERRIDE`); Modify `server/worker/src/env.ts` (`ALERT_EMAIL?`, `STORAGE_ALERT_BYTES?`); Modify `server/worker/wrangler.toml` (`STORAGE_ALERT_BYTES = "0"` = off, a comment saying the owner sets it, and that `ALERT_EMAIL` is a `wrangler secret`); Test `server/worker/test/notices.test.ts`, `server/worker/test/mail.test.ts`.
**Interfaces:** `alertStorage(ctx: Ctx, now: Date): Promise<number>` returns 1 when it sent. It sums `stored_bytes` of `ctx.db.runsForTick(...)` (open runs and runs that ended within the lookback) and mails once per 6 hours when the sum is at least `STORAGE_ALERT_BYTES` (0, unset, or no `ALERT_EMAIL`: nothing). The marker is the R2 object `cron/storage-alert.json` `{ "at": "<time>" }`, outside `traces/`, like `HEALTH_KEY`. Subject `Goal Cup server: stored bytes over the alert level`; the text holds the sum, the level, and the top 3 run ids by `stored_bytes` (ids only, no member data). A failed send is logged and does not write the marker.
- [ ] Step 1: In `mail.test.ts` style (stub Resend, `FakeBucket`) write three tests: `storage alert: below the level sends nothing` (sum 99, level 100); `storage alert: over the level sends one mail, naming the top 3 runs, and a second tick within 6 hours sends none` (two ticks 5 minutes apart: `sent.length === 1`; a third at +6h01m: `sent.length === 2`); `storage alert: level 0 or no ALERT_EMAIL sends nothing`. The ctx's `db.runsForTick` returns rows with `stored_bytes` 60 and 50 (sum 110).
- [ ] Step 2: `cd server/worker && node --test --test-name-pattern="storage alert" test/mail.test.ts`. Expect FAIL: `alertStorage` is not exported.
- [ ] Step 3: Implement.
- [ ] Step 4: `cd server/worker && npm test`. Expect PASS.
- [ ] Step 5: `git add server/worker && git commit -m "worker: mail the owner when stored bytes pass a level"`

### Task J.7: Purge expired device requests   (size M, after J.6 and J.8)
**Files:** Create `server/worker/sql/pending/20261105100000_goal_cup_purge_device_requests.sql`; Modify `server/worker/sql/goal-cup-tracer-upload.sql` (append `\n-- ======================== 20261105100000_goal_cup_purge_device_requests.sql ========================\n` and the file, the regeneration format `check-sql.sh` checks byte for byte); Modify `server/worker/src/db.ts` (interface `Db`: `purgeDevices(now: string): Promise<number>`), `db-supabase.ts` (RPC `upload_purge_device_requests`, `p_now`; result checked as a non-negative integer), `db-local.ts` (returns 0: local state is thrown away); Modify `server/worker/src/notices.ts` (`purgeDevices` step in `runTick`); Test `supabase.test.ts`, `notices.test.ts`, and a scenario in `server/worker/scripts/sql-scenarios.sql` (from J.8).
**Interfaces:** SQL `public.goal_cup_upload_purge_device_requests(p_now text) RETURNS integer`, SECURITY DEFINER, `SET search_path = goal_cup_tracer, pg_temp`, deletes `device_requests` where `staging = caller_is_staging()` and `expires_at < p_now::timestamptz - interval '1 day'`; REVOKE from PUBLIC, anon, authenticated; GRANT EXECUTE to `goal_cup_ingest_production`, `goal_cup_ingest_staging`, `service_role`. Only the owner applies it (`server/worker/README.md` "Deploying"); the website repo gets the same file.
- [ ] Step 1: Tests. `supabase.test.ts`: `purgeDevices -> upload_purge_device_requests(p_now)` returns 3 for a stub of `3`, and a stub of `"x"` is a 502. `notices.test.ts`: `a failing purge step does not stop the other cron steps` (db stub whose `purgeDevices` throws; `closeRuns` still called). Scenario (append to `sql-scenarios.sql`): insert three `device_requests` rows with `expires_at` = now minus 2 days, now minus 1 hour, and a staging one minus 2 days; call purge as production; assert 1 returned, the other two rows remain.
- [ ] Step 2: `cd server/worker && node --test --test-name-pattern="purgeDevices|purge step" test/supabase.test.ts test/notices.test.ts`. Expect FAIL: `purgeDevices` is not a function. Then `sh server/worker/scripts/check-sql.sh` (J.8). Expect FAIL on the new scenario: function does not exist.
- [ ] Step 3: Write the migration and mirror section, the three `Db` methods and the cron step.
- [ ] Step 4: `cd server/worker && npm test && sh scripts/check-sql.sh`. Expect PASS; the mirror equals the pending file byte for byte.
- [ ] Step 5: `git add server/worker && git commit -m "worker: purge expired device requests (pending migration)"`

### Task J.8: Run the real SQL under the Worker's scenarios   (size L, PARALLEL-OK; before J.7)
**Files:** Modify `server/worker/scripts/check-sql.sh` (lines 61-68: with no file in `sql/pending/` apply the whole mirror and go on, instead of exit 2; at the end run `psql -f scripts/sql-scenarios.sql` and then the parallel checks); Create `server/worker/scripts/sql-scenarios.sql`; Modify `scripts/gate.sh` (one step after line 82); Modify `server/worker/README.md` layout row for `check-sql.sh` (line 151).
**Interfaces:** `sql-scenarios.sql` is one `DO $$ ... $$` block of `ASSERT`s in the style of the existing smoke block (lines 117-285), run after it, over the functions the Worker calls: `goal_cup_upload_open_run`, `..._commit_chunk(text,text,jsonb,bigint)`, `..._heartbeat`, `..._seal_run`, `..._close_run`, `..._poll_device`, `..._get_chunk`. gate.sh step: `if command -v initdb >/dev/null 2>&1; then step sh server/worker/scripts/check-sql.sh; else echo "gate.sh: SQL scenarios skipped (no initdb)"; fi` (owner question 4).
Scenarios (names are the comment headings in the file; each mirrors a LocalDb test):
1. `commit` advances offset, head, next_seq, `stored_bytes`, `chunk_count`; the same chunk again is `conflict` and adds nothing (local-db.test.ts:381).
2. A wrong offset, a wrong prev, a wrong first_seq each answer `conflict` and change nothing (integration.test.ts:270).
3. After the final heartbeat commits seq 9: a chunk ending at 10 is `beyond_final`; one ending at 9 with head `final_head` commits (local-db.test.ts:270).
4. Seal: before the chain reaches the end `chain_behind`; without a final `no_final`; both orders (chunk then final heartbeat, final heartbeat then chunk) reach `sealed`; a second seal with the same digests is `already_sealed`, another digest `conflict` (local-db.test.ts:300).
5. A run closed `incomplete`: commit, seal and heartbeat answer `closed` (local-db.test.ts:278).
6. `poll_device` twice after approval: the first returns a credential, the second `expired`, and `run_credentials` holds exactly one new row (local-db.test.ts:445).
7. Parallel (shell part of `check-sql.sh`, after the DO block): 8 background `psql` calls commit the same chunk at the same offset; the output holds exactly one `committed` and seven `conflict`; `chunk_count` is 1. The same with 8 `poll_device` calls on one approved request: one `approved`, seven `expired`, one credential row.
- [ ] Step 1: Write `sql-scenarios.sql` scenarios 1-6 and the parallel part as failing assertions: temporarily call the script with the scenario for `chunk_count` expecting 2 (wrong on purpose) to prove the harness fails.
- [ ] Step 2: `sh server/worker/scripts/check-sql.sh`. Expect exit 1 with `ASSERT` text naming scenario 1 (and today, before the `exit 2` change, exit 2 "no file in sql/pending/"). Needs PostgreSQL `initdb` on PATH (the owner's Mac or Linux; `PG_BIN=<dir>` otherwise).
- [ ] Step 3: Fix the expectation to the true value; make the no-pending path apply the whole mirror.
- [ ] Step 4: `sh server/worker/scripts/check-sql.sh`. Expect exit 0 and `check-sql: smoke calls passed` plus `check-sql: scenarios passed`. Then run it twice more to catch a flaky parallel part.
- [ ] Step 5: `git add server/worker/scripts scripts/gate.sh server/worker/README.md && git commit -m "worker: run the real SQL under the Worker's scenarios"`

### Task J.9: A failed R2 delete must be retryable without emptying everything   (size M, PARALLEL-OK)
**Files:** Modify `server/worker/scripts/delete-test-runs.sh` (header comment lines 2-16 say R2 is listed before the database changes: keep that, and state that the objects are deleted after the commit; new option `--objects FILE`; lines 108-121); Create `server/worker/test/delete-script.test.ts`; Modify `server/worker/README.md` "Clean up test runs" step 3 (line 400-404); Modify `docs/OPERATING.md` lines 130-132.
**Interfaces:** On a failed object delete the script writes `delete-test-runs.failed` in the current directory (lines `<bucket> <key>`, only the ones that failed), exits 1, and says `Retry only these: sh scripts/delete-test-runs.sh --objects delete-test-runs.failed --yes`. `--objects FILE` skips the database, reads the plan from FILE, refuses any line whose key is outside the four fixed bucket:prefix pairs, and (without `--yes`) is a dry run that prints the count. The old advice "run again without `--older-than`" goes.
- [ ] Step 1: `delete-script.test.ts` (node:test, `spawnSync("sh", [script, "--older-than", "7 days", "--yes"], { env: { PATH: stubDir + ":" + process.env.PATH, DATABASE_URL: "x", CLOUDFLARE_ACCOUNT_ID: "a", CLOUDFLARE_API_TOKEN: "t" }, cwd: tmp })`). Stubs written to `stubDir`: `psql` prints `{"runs":["r0000000000000001"],"counts":{}}`; `curl` answers the list call with one key `traces/test/r0000000000000001/e1/0.zst` for `goalcup-traces` (empty for the others, HTTP 200) and fails (`exit 22`) on `-X DELETE`. Assert: exit 1; `tmp/delete-test-runs.failed` equals `goalcup-traces traces/test/r0000000000000001/e1/0.zst\n`; stderr contains `--objects delete-test-runs.failed` and does not contain `without --older-than`. Second test: `--objects` with a line `goalcup-traces other/key` exits 1 and the stub curl log shows no DELETE.
- [ ] Step 2: `cd server/worker && node --test test/delete-script.test.ts`. Expect FAIL: no `.failed` file; old advice in stderr.
- [ ] Step 3: Implement.
- [ ] Step 4: `cd server/worker && npm test && sh -n scripts/delete-test-runs.sh`. Expect PASS.
- [ ] Step 5: `git add server/worker docs/OPERATING.md && git commit -m "worker: delete-test-runs retries only the objects that failed"`

### Task J.10: Server notes, Worker side   (size M, PARALLEL-OK)
**Files:** Create `server/worker/src/advisories.json` (`[]` to start); Create `server/worker/src/notes.ts`; Modify `server/worker/src/runs.ts` (`runView` and the `continueRun` answer gain `...notesField`; open reads `machine?.os`, continue reads `b.os`); Modify `server/worker/tsconfig.json` (`"resolveJsonModule": true`); Test `server/worker/test/notes.test.ts`, `server/worker/test/lifecycle.test.ts`.
**Interfaces:**
```ts
export interface Advisory { connector: string; version: string; os: string; note: string } // "*" matches any
export const NOTES_MAX = 5; export const NOTE_MAX_CHARS = 200;
export function pickNotes(rows: readonly Advisory[], facts: { connector: string; harnessVersion: string; os: string }): string[]
export function validAdvisories(rows: unknown): string | null  // null = valid, else the first problem
```
`pickNotes`: file order; at most 5; `version` "*" matches all, otherwise exact on the numeric parts of the first `\d+(\.\d+)*` token in `harnessVersion` ("0.22.6" matches "Hermes v0.22.6" and not "0.22.60"), or a numeric prefix ("0.22" matches "0.22.6" and "0.22", not "0.220"); `connector` and `os` exact (os is `std::env::consts::OS`, "macos" or "linux") or "*". `validAdvisories` requires an array of at most 50 objects with exactly the four string keys, `note` 1-200 chars with no control character (`/\p{Cc}/u`) and no `‪-‮⁦-⁩`, and `version` matching `^(\*|\d+(\.\d+)*)$`. The answers carry `notes` only when non-empty, so existing answers stay byte-identical.
- [ ] Step 1: `notes.test.ts`:
  ```ts
  const rows = [
    { connector: "hermes-cli", version: "0.22.6", os: "*", note: "A" },
    { connector: "hermes-cli", version: "0.22", os: "linux", note: "B" },
    { connector: "*", version: "*", os: "*", note: "C" },
  ];
  test("exact, prefix and any", () => {
    assert.deepEqual(pickNotes(rows, { connector: "hermes-cli", harnessVersion: "Hermes Agent v0.22.6", os: "macos" }), ["A", "C"]);
    assert.deepEqual(pickNotes(rows, { connector: "hermes-cli", harnessVersion: "0.22.60", os: "linux" }), ["B", "C"]);
    assert.deepEqual(pickNotes(rows, { connector: "hermes-cli", harnessVersion: "0.220.1", os: "linux" }), ["C"]);
    assert.deepEqual(pickNotes(rows, { connector: "codex-cli", harnessVersion: "unknown", os: "" }), ["C"]);
  });
  test("at most five", () => assert.equal(pickNotes(Array(9).fill(rows[2]), { connector: "x", harnessVersion: "1", os: "" }).length, 5));
  test("the committed file is valid", () => assert.equal(validAdvisories(advisories), null));
  test("validAdvisories refuses ESC, bidi, 201 chars, a sixth key, version 'v1'", () => { /* one row each; each returns a non-null string */ });
  ```
  `lifecycle.test.ts`: `open and continue carry notes only when a row matches`, injecting rows through `pickNotes`'s module-level `ADVISORIES` that the test replaces (export `let ADVISORIES` with a `setAdvisoriesForTest`-free approach: `runs.ts` imports the JSON and passes it; the test starts the rig with a vars key `ADVISORIES_JSON` honoured only when `DB_BACKEND=local`, via the existing `testOnly` guard in `context.ts:60`). Assert: with a row `{connector:"other-cli", version:"9", os:"*", note:"Use the newer build."}`, the continue answer (`harness_version: "9"`, `connector_id: "other-cli"` per `rig.ts:46`) has `notes: ["Use the newer build."]`; with no matching row, `"notes" in body === false`; the open answer of the same run on replay (`200`) carries the same notes.
- [ ] Step 2: `cd server/worker && node --test test/notes.test.ts`. Expect FAIL: cannot find `../src/notes.ts`.
- [ ] Step 3: Implement `notes.ts` and the wiring.
- [ ] Step 4: `cd server/worker && npm test`. Expect PASS.
- [ ] Step 5: `git add server/worker && git commit -m "worker: notes in the open and continue answers, from advisories.json"`

### Task J.11: Print the notes, client side   (size M, PARALLEL-OK; wire contract from J.10)
**Files:** Create `crates/goalcup/src/cli/notes.rs`; Modify `crates/goalcup/src/cli/mod.rs` (`pub mod notes;`), `approve.rs` (insertion points above), `continue_run.rs` (insertion points above).
**Interfaces:**
```rust
#[derive(Debug, Default, PartialEq)]
pub struct Notes(Vec<String>);          // Deserialize: lenient (a Value in; anything but an array of strings gives the strings it has, else empty)
impl Notes { pub fn print(&self, out: &mut dyn std::io::Write) }   // "Note from the server: <line>\n" each; never errors
```
Rules: first 5 strings; each stripped of `char::is_control()` characters and of U+200B-200F, U+202A-202E, U+2066-2069; cut to 200 chars (chars, not bytes); trimmed; empty ones skipped. Fields: `#[serde(default)] notes: Notes` in `Open` and `Next`. Calls: `open.notes.print(out);` and `got.notes.print(out);`. Continue body gains `"os": std::env::consts::OS`.
- [ ] Step 1: Unit tests in `notes.rs` (`#[cfg(test)]`):
  - `keeps_five_lines_of_200_chars`: 7 strings in, 5 out; a 300-char string out is 200 chars; a 150-emoji string is 150 chars.
  - `strips_controls_and_escapes`: `"\u{1b}[31mred\u{1b}[0m\u{7}\n\u{9b}x\u{202e}y"` gives `"[31mred[0mxy"`.
  - `skips_lines_that_end_up_empty`: `["\u{1b}", "  ", "ok"]` gives `["ok"]`.
  - `wrong_shapes_are_ignored`: `serde_json::from_value::<Wrap>(json!({"notes": 5}))` is empty; `{"notes": [1, "a", null]}` is `["a"]`; no key is empty. (`Wrap` is a test struct with `#[serde(default)] notes: Notes`.)
  - `print_writes_prefixed_lines`: output equals `"Note from the server: ok\n"`.
  - A test in `tests/cli_continue.rs` is not added: the fake Worker (`tests/common/worker.rs`) is not changed.
- [ ] Step 2: `cargo test -p goalcup cli::notes`. Expect FAIL: unresolved module `notes`.
- [ ] Step 3: Implement `notes.rs`, add the two fields and the calls, the `os` key and the `mod` line.
- [ ] Step 4: `cargo test -p goalcup cli::notes && cargo nextest run -p goalcup --test cli_approve --test cli_continue`. Expect PASS (an answer without `notes` is unchanged).
- [ ] Step 5: `git add crates/goalcup/src/cli && git commit -m "goalcup: print the server's notes (capped, control characters stripped)"`

### Task J.12: Docs   (size S, last; verification by grep)
**Files:** Modify `docs/UPLOAD-API.md`, `server/worker/README.md`.
**Edits:**
1. README line 67-73 ("What the server stores about a person"): add the workspace path (the absolute folder goalcup ran in, `cli/approve.rs:131`, stored per entry, may hold the OS user name; RULES already tells the participant).
2. UPLOAD-API line 11 and README line 81-92: "1.0 needs none" and "nothing to migrate" are wrong: `stored_bytes` and `chunk_count` need website migration `20261021100000` (`db-supabase.ts:80`); add the J.7 migration as pending.
3. UPLOAD-API table row 429 (line 38): "Too many calls" becomes "Too many device-flow calls (the sign-in calls only; authenticated calls are not limited)".
4. UPLOAD-API chunk section (line 276, 292): `Content-Length` is required; without it `400 malformed`; no chunked transfer.
5. UPLOAD-API: remove the `dev` paragraph (line 174-177) and "a `dev` competition run" from the 403 row (line 37); line 546 (`disk_full`) says the cron no longer queues it; line 119 and the 410 row gain the J.4 wording.
6. UPLOAD-API open and continue sections: the `notes` field (answer), `os` (continue request), the advisory file and its caps.
7. README: `PRACTICE_MAX_BYTES`, `STORAGE_ALERT_BYTES`, `ALERT_EMAIL` in Configuration (line 155 on); "Deploying": the purge migration is applied before the Worker that calls it (a missing function is one logged line per tick, no harm).
- [ ] Step 1: Make the edits.
- [ ] Step 2: `grep -n "dev\b" docs/UPLOAD-API.md | grep -iv "development\|device"` returns no `--dev` line; `grep -c "workspace" server/worker/README.md` is at least 1; `grep -n "1.0 needs none\|nothing to migrate" docs/UPLOAD-API.md server/worker/README.md` returns nothing; `grep -n "PRACTICE_MAX_BYTES\|STORAGE_ALERT_BYTES\|ALERT_EMAIL" server/worker/README.md` returns three lines.
- [ ] Step 3: `git add docs/UPLOAD-API.md server/worker/README.md && git commit -m "docs: the Worker's contract and README match the code"`

## Open owner questions
1. `ALERT_EMAIL` address and levels: suggested `PRACTICE_MAX_BYTES` 1 GiB and `STORAGE_ALERT_BYTES` 200 GiB (J.5, J.6).
2. Who applies the J.7 migration in the website repo (`/Users/team/dev/PROJECTS/AgentCommunity/agentCommunity_PAGE`), and when? Production deploy of the Worker stays the owner's (OPERATING.md); this lane tests on staging only.
3. `select count(*) from goal_cup_tracer.runs where status = 'abandoned'` on production and staging: 0 allows cutting that status.
4. Add the SQL scenarios to `gate.sh` (needs PostgreSQL on the Mac)? Planned as skipped when `initdb` is absent.
5. Advisory text is reviewed in git; should a note ever be allowed without a release of the Worker (a dashboard edit)? Planned: no.
