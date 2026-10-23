# Server accept list and admin-only `--dev` (D70) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A competition run needs a connector digest the server has accepted. `--dev` works only for website admins and never competes. A new harness then needs one reviewed database row, not a release.

**Architecture:** The rule lives in the database functions that open and continue a run, so every Worker (old and new) applies it. The Worker carries a new `dev` flag. The CLI stops refusing unlisted connectors, loads them with the strict rules, and refuses `--dev` in the competition before approval.

**Tech Stack:** PostgreSQL (Supabase, plpgsql), Cloudflare Worker (TypeScript, `node --test`), Rust CLI (`goalcup-cli`, `goalcup-protocol`).

**Spec:** [`docs/research/ACCEPT-LIST-DESIGN.md`](../research/ACCEPT-LIST-DESIGN.md)

## Global Constraints

- Release cut: before **6 Nov 2026 12:00 UTC**. Tasks 4 and 5 (CLI) must be on `main` before it.
- Tasks 4 and 5 start only after #243 and #257 are merged to `main` (they change the same CLI files). Tasks 1 to 3 do not wait.
- Staging and production share ONE database. The migration is additive only: new table, new columns with defaults, new function overloads; the old signatures stay as wrappers that pass `dev = false`.
- Write the migration in `server/worker/sql/pending/` first, add it to the mirror `server/worker/sql/goal-cup-tracer-upload.sql` byte for byte as its last section, then copy it unchanged to the website repo (`agentCommunity_PAGE/supabase/migrations/`). Follow `server/worker/sql/pending/README.md`. Build on the mirror's latest version of each function: `goal_cup_upload_open_run` from the pending `20261021100000` file, `goal_cup_upload_continue_run` from `20261015130000`.
- **`p_dev boolean` has NO DEFAULT on both new overloads.** A default would make the live Worker's 20-argument open and 14-argument continue match two overloads, and PostgREST would refuse every production open (see the `20261021100000` header, which avoids defaults for this reason).
- Every new function: `REVOKE ALL ... FROM PUBLIC, anon, authenticated` (revoking from `PUBLIC` alone is a no-op on Supabase), then grant only what the existing upload functions grant. `pnpm db:check` in the website repo must stay green.
- Migration file name: `20261024100000_goal_cup_tracer_connector_accept.sql`.
- Seed digests (v0.6.1 and `main`, from `crates/goalcup-protocol/pins/connector-dirs.txt`):
  - `amp-cli` `9b9fe10809eb8e3e1833788909b8e915f40d7adf2cfbaaa7b40db774ce5b304c`
  - `claude-code-cli` `04c8322a66ea5e7b376feb2cf52a21ab8aaafb0430fc550396e65d4bbb1a5dbc`
  - `codex-cli` `239325791a34ac7c78021c49e9bf7bfd6d265f9a8a336a5b3da8a71ca04aafba`
  - `hermes-cli` `6d7e123435703f6e1b6e4f91a500a6bf301333ba7f4b3d26924d0412e9e27fba`
  - `accepted_by` for seed rows: `'seed: v0.6.1 pins'`.
- Refusal copy, exact, used by SQL and checked by tests:
  - `NOT_ACCEPTED`: `This connector is not accepted for the cup yet. You can practise or test with it, and ask for it to be accepted (see HARNESS-ONBOARDING).`
  - `DEV_COMPETES`: `A --dev run never enters the competition. Start without --dev, or use a practice or test run.`
  - `DEV_NOT_ADMIN`: `--dev is for Goal Cup admins only. Start without --dev: your connector then loads with the strict rules.`
- Admin = the member's `auth.users` row has `email_confirmed_at IS NOT NULL` and `lower(btrim(email))` equals `lower(btrim(public.admins.email))` for some row.
- The check order in `connector_refusal`: `DEV_COMPETES` (`p_dev` and mode `competition`), then `DEV_NOT_ADMIN` (`p_dev` and not admin), then `NOT_ACCEPTED` (mode `competition` and the `(digest, connector_id)` pair not in the table).
- No secrets in logs or argv. Never set a model base URL. Player-facing text: plain short English, no em dashes.
- Every commit is signed off (`git commit -s`, DCO check). Rust: `PATH="$HOME/.cargo/bin:$PATH"` (needs 1.99). Worker: `cd server/worker && npm test`. SQL: `sh server/worker/scripts/check-sql.sh` (needs `initdb`, `pg_ctl`, `psql`).

## Review Focus

1. **A digest players run is not seeded, or the new overload is ambiguous:** every competition start is refused at once, on production too. Task 1 adds a test that every pinned digest is seeded, and a check that the old-signature calls each match exactly one function.
2. **`continue` with a different connector:** a competition run opened with an accepted digest continues with an unlisted one. It must be refused (Task 1 and Task 2 tests).
3. **An old Worker or old CLI that sends no `dev`:** it must be read as `false`, and the competition check must still apply through the old SQL signature (Task 1 wrapper test, Task 2 parse test).
4. **Admin email that differs in case or spaces** from the sign-in email: the member is still an admin (Task 1 test).
5. **A player's own connectors folder** (`GOALCUP_CONNECTORS_DIR` with the shipped four plus one new folder): the shipped four load as pinned, the new one loads with the strict rules, and the daemon accepts the same bytes (Task 4 test).

---

### Task 1: Database: accept list, admin check, `dev` on open and continue

**Files:**
- Create: `server/worker/sql/pending/20261024100000_goal_cup_tracer_connector_accept.sql`
- Modify: `server/worker/sql/goal-cup-tracer-upload.sql` (append the same file as the last section)
- Modify: `server/worker/sql/pending/README.md` (list the new file and its changes)
- Modify: `server/worker/scripts/check-sql.sh` (stub `public.admins`; smoke and assertion calls below)
- Create: `server/worker/test/accept-seed.test.ts`

**Interfaces:**
- Produces, in SQL:
  - table `goal_cup_tracer.accepted_connectors (connector_dir_sha256 text PRIMARY KEY CHECK (connector_dir_sha256 ~ '^[0-9a-f]{64}$'), connector_id text NOT NULL, accepted_at timestamptz NOT NULL DEFAULT now(), accepted_by text NOT NULL, note text)`, RLS on, all privileges revoked from `PUBLIC, anon, authenticated`, seeded with the four rows above.
  - columns `goal_cup_tracer.entries.dev boolean NOT NULL DEFAULT false` and `goal_cup_tracer.entries.connector_accepted boolean` (set at insert: is the `(digest, connector_id)` pair in the table).
  - `goal_cup_tracer.member_is_admin(p_member uuid) RETURNS boolean` (STABLE, SECURITY DEFINER, `search_path = goal_cup_tracer, pg_temp`, schema-qualify `auth.users` and `public.admins`; the rule in Global Constraints).
  - `goal_cup_tracer.connector_refusal(p_member uuid, p_mode text, p_connector_id text, p_digest text, p_dev boolean) RETURNS text`: `NULL` when allowed, else the exact copy, in the order in Global Constraints.
  - `public.goal_cup_upload_open_run(<all current params>, p_dev boolean)` (no default): calls `connector_refusal` after the event, terms and window checks and before anything is written (before the competition replace), and returns `{"result":"forbidden","message":<text>}` when it is not `NULL`. The current signature stays as a wrapper passing `false`.
  - `public.goal_cup_upload_continue_run(<all current params>, p_dev boolean)` (no default): the same, with the mode read from the run row, checked after the `existing`, `closed` and `conflict` answers and before the insert. The current signature stays as a wrapper passing `false`.
  - `public.goal_cup_upload_accepted_connectors() RETURNS jsonb`: `[{"connector_id":…,"connector_dir_sha256":…}]` ordered by `connector_id`, granted like the other upload functions.

- [ ] **Step 1: Write the failing seed test** `server/worker/test/accept-seed.test.ts`, test `every pinned connector digest is seeded`: read `crates/goalcup-protocol/pins/connector-dirs.txt` (skip `#` lines) and the mirror `server/worker/sql/goal-cup-tracer-upload.sql` (not the pending file, which is deleted once it reaches the website repo); gather the values of every `INSERT INTO goal_cup_tracer.accepted_connectors`; assert each pinned `(id, digest)` pair is among them.
- [ ] **Step 2:** `cd server/worker && npm test`. Expected: the new test FAILS (file missing).
- [ ] **Step 3: Add the checks to `check-sql.sh`** after the pending files are applied, each `psql` call failing the script on a wrong answer:
  - stub `CREATE TABLE public.admins (email text NOT NULL)` with the other Supabase stubs (and `auth.users.email_confirmed_at` if the stub lacks it); insert `' Admin@Example.org '` for a confirmed stub `auth.users` row with email `admin@example.org`, and a second, unconfirmed user whose email is also in `public.admins` (must NOT count as admin);
  - the competition cases need a consent row, a competition credential and a `p_official_start` inside the event's start window: insert a test event with an open window rather than relying on today's date;
  - overloads: `SELECT count(*) FROM pg_proc WHERE proname = 'goal_cup_upload_open_run'` is 2 and the new one has no parameter default (`pronargdefaults = 0`), the same for `goal_cup_upload_continue_run`; a call with the old 20 named open arguments and the old 14 named continue arguments resolves to exactly one function (call each by name, as PostgREST does);
  - `has_function_privilege('anon', …, 'EXECUTE')` and `'authenticated'` are false for every new function;
  - open competition, accepted digest, `p_dev => false`: `created`;
  - open competition, unlisted digest: `forbidden` with `NOT_ACCEPTED`;
  - open competition, accepted digest, `p_dev => true`, admin: `forbidden` with `DEV_COMPETES`;
  - open competition, accepted digest, `p_dev => true`, non-admin: `forbidden` with `DEV_COMPETES` (not `DEV_NOT_ADMIN`);
  - open competition, an accepted digest under a different `connector_id`: `forbidden` with `NOT_ACCEPTED`;
  - open practice, unlisted digest, `p_dev => true`, non-admin: `forbidden` with `DEV_NOT_ADMIN`;
  - open practice, unlisted digest, `p_dev => true`, admin (the case/space-different email): `created`, and the entry has `dev = true`, `connector_accepted = false`;
  - open practice, unlisted digest, `p_dev => false`, non-admin: `created`;
  - old open signature, competition, unlisted digest: `forbidden` with `NOT_ACCEPTED`;
  - continue the accepted competition run with an unlisted digest (new and old signature): `forbidden` with `NOT_ACCEPTED`, and no new entry row;
  - continue a closed run with an unlisted digest: `closed` (the closed answer comes first);
  - `goal_cup_upload_accepted_connectors()` returns the four seeded rows.
- [ ] **Step 4:** `sh server/worker/scripts/check-sql.sh`. Expected: FAILS (the migration does not exist).
- [ ] **Step 5: Write the migration** with the Interfaces above, a header in the style of `20261021100000` (what, why, D70, "additive only", apply command), and the same grants as the existing upload functions. Append it byte for byte to the mirror. Update `sql/pending/README.md`.
- [ ] **Step 6:** `sh server/worker/scripts/check-sql.sh` and `cd server/worker && npm test`. Expected: both PASS (the seed test and the mirror parity test included).
- [ ] **Step 7: Commit** `git commit -s -m "Server accept list for connectors, admin-only dev runs (D70): migration"`.

### Task 2: Worker: carry `dev`, map the new refusals

**Files:**
- Modify: `server/worker/src/runs.ts` (`openRun`, `continueRun`, `connectorFields` or a new `devField`)
- Modify: `server/worker/src/db.ts` (`OpenRunArgs.dev`, `ContinueRunArgs.dev`, `ContinueRunResult` gains `{ kind: "forbidden"; message: string }`)
- Modify: `server/worker/src/db-supabase.ts` (pass `p_dev`; map `forbidden` on continue)
- Modify: `server/worker/src/db-local.ts` (the same rules, in memory: an accepted set seeded with the four `(id, digest)` pairs, an admin set of member ids; both configurable through the `LocalDb` constructor, and the admin set also from a `LOCAL_ADMIN_MEMBERS` var for `wrangler dev`; keep it in step with the SQL)
- Modify: `server/worker/src/util.ts` (add `boolField(o, name, fallback: boolean): boolean`, refusing non-boolean values with `malformed`)
- Modify: `server/worker/test/helpers/client.ts` (its competition opens use digest `"c".repeat(64)`, which is not accepted: register that pair as accepted in the test `LocalDb`, so existing lifecycle tests keep passing for the right reason)
- Test: `server/worker/test/lifecycle.test.ts`, `server/worker/test/local-db.test.ts`, `server/worker/test/supabase.test.ts`

**Interfaces:**
- Consumes: Task 1's SQL signatures and copy.
- Produces: `POST /v1/runs` and `POST /v1/runs/{id}/continue` accept an optional boolean `dev` (missing means `false`; a non-boolean is `400 malformed`). A refusal is `403 forbidden` with the SQL message on both routes.

- [ ] **Step 1: Write the failing tests** in `lifecycle.test.ts`:
  - `a competition open with an unlisted connector is refused` → status 403, body message `NOT_ACCEPTED`;
  - `a dev open by a non-admin is refused` → 403, `DEV_NOT_ADMIN`;
  - `a dev competition open is refused even for an admin` → 403, `DEV_COMPETES`;
  - `a practice open with an unlisted connector and no dev opens` → 201;
  - `an open without dev is read as dev false` → 201 with a seeded digest in competition;
  - `dev must be a boolean` → `dev: "yes"` gives 400;
  - `a continue with an unlisted connector in a competition run is refused` → 403, `NOT_ACCEPTED`.
  In `supabase.test.ts`, assert the RPC payload carries `p_dev` (true and false), and that a continue answer `{"result":"forbidden","message":…}` maps to `forbidden`, not to the `502` that unknown answers get today (`db-supabase.ts` continue mapping).
- [ ] **Step 2:** `cd server/worker && npm test`. Expected: the new tests FAIL.
- [ ] **Step 3: Implement** the fields, the `db-local` rules and the 403 mapping in `continueRun` (`case "forbidden": throw new ApiError(403, "forbidden", result.message)`).
- [ ] **Step 4:** `cd server/worker && npm test`. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -s -m "Worker carries dev and refuses unaccepted connectors (D70)"`.

### Task 3: Worker: public list of accepted connectors

**Files:**
- Modify: `server/worker/src/index.ts` (route `GET /v1/connectors/accepted`, no auth, placed after the maintenance switch so maintenance costs no database call)
- Create: `server/worker/src/connectors.ts` (`serveAccepted(ctx: Ctx, request: Request): Promise<Response>`)
- Modify: `server/worker/src/db.ts`, `db-supabase.ts`, `db-local.ts` (`acceptedConnectors(): Promise<Array<{ connector_id: string; connector_dir_sha256: string }>>`)
- Test: `server/worker/test/lifecycle.test.ts`

**Interfaces:**
- Produces: `GET /v1/connectors/accepted` → `200 {"accepted":[{"connector_id":…,"connector_dir_sha256":…}]}`, `Cache-Control: public, max-age=60`, cached at the edge under the path alone (a query string must not bypass the cache). Any other method: 405.

- [ ] **Step 1: Write the failing tests:** `the accepted list is public and lists the seeded four`; `a query string is served from the same cache key` (two requests with different `?x=` values reach the database once, using the test double's call counter).
- [ ] **Step 2:** `npm test`. Expected: FAIL.
- [ ] **Step 3: Implement** `serveAccepted`, keying the cache with `new Request(new URL(request.url).origin + "/v1/connectors/accepted")`.
- [ ] **Step 4:** `npm test`. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -s -m "Worker: public list of accepted connectors (D70)"`.

### Task 4: CLI: strict load for unlisted connectors, `dev` on the wire, no `--dev` in the competition

Start after #243 and #257 are on `main`.

**Files:**
- Modify: `crates/goalcup-cli/src/preflight/gates.rs` (`harness_pin_gate`, `dev_or_fail`)
- Modify: `crates/goalcup-cli/src/upload/api.rs` (`OpenRunRequest.dev`, `ContinueRequest.dev`)
- Modify: `crates/goalcup-cli/src/commands/start/server.rs` and `crates/goalcup-cli/src/commands/continue_run.rs` (fill `dev` from the run's `--dev`; a continued run reads it from its stored record)
- Modify: `crates/goalcup-cli/src/participant/refusals.rs` (local refusal of `--dev` with a competition run, `DEV_COMPETES` copy), called from `commands/start/refusals.rs`
- Modify: `crates/goalcup-cli/src/commands/prep.rs` (`goalcup setup --dev` runs a practice upload: send `dev`)
- Modify: `crates/goalcup-cli/src/commands/doctor.rs` (it calls the pin gate with `dev = false`: an unlisted connector is now reported with the non-failing text, not a fail)
- Modify: `crates/goalcup-cli/src/commands/status.rs` (a `dev` run says so in its header line)
- Modify: `crates/goalcup-verifier/src/capture_analysis/report.rs` (the `Dev/unpinned` label text becomes `Not in this build's list (check the server's accepted list)`; the machine-readable value stays the same)
- Modify: `crates/goalcup-cli/tests/common/m2_cli_support/mod.rs` (its `start --harness <id> --dev` runs are competition mode today; add `--practice`, or `--test` where the test needs a test run, so the new local refusal does not break them)
- Modify: `scripts/matrix.sh` fake cell and the #258 kit, if merged (they start `--dev` runs against staging: use practice mode, and note that a real `--dev` run on staging needs an admin member)
- Test: `crates/goalcup-cli/tests/it/` (the start, continue and connector-contract files that already cover the pin gate; add `own_connectors_dir.rs` if no file fits)

**Interfaces:**
- Consumes: Task 2's wire field and 403 copy.
- Produces: `#[serde(default, skip_serializing_if = "std::ops::Not::not")] pub dev: bool` on both request types (sent only when true). The pin gate's three outcomes:
  - pinned id, pinned digest: pass, unchanged;
  - pinned id, other digest: fail, unchanged (a changed copy needs its own id);
  - unpinned, no `--dev`: does not stop the start; text: `` `{id}` is not in this build's list. It loads with the strict rules, and the server decides whether a competition run with it counts. ``
  - unpinned, `--dev`: does not stop the start; text: `This is a --dev run. It is for Goal Cup admins only, and it never enters the competition.`

- [ ] **Step 1: Write the failing tests:**
  - `an unlisted connector without dev starts a practice run` (fake server answers 201; the open request has no `dev` key);
  - `an unlisted connector without dev loads with the strict rules` (a loopback model host in its `hosts` is refused at load);
  - `dev sends dev true on open and continue`;
  - `dev with a competition run is refused before approval` (no device request reaches the fake server; output has `DEV_COMPETES`);
  - `a server refusal of the connector is shown in full` (fake server answers 403 `NOT_ACCEPTED`; output has the message; no clone, no harness started);
  - `a connectors dir with the shipped four and one new folder` (`GOALCUP_CONNECTORS_DIR`): the shipped four pass the pin gate, the new one loads strict, and the daemon starts the run with the same digest;
  - `doctor reports an unlisted connector without failing`;
  - `setup with dev sends dev true on its practice open`.
- [ ] **Step 2:** `PATH="$HOME/.cargo/bin:$PATH" cargo test -p goalcup-cli --test it <names>`. Expected: FAIL.
- [ ] **Step 3: Implement.** The CLI's loader already validates an unlisted directory as `Origin::Pinned` (the strict rules) when `--dev` was not typed (`connector.rs`), so the change is the gate's levels and text, the wire field, and the local competition refusal. The daemon does not change: it loads unlisted folders as `Origin::Dev` (`goalcup-daemon/src/runtime/capture/harness.rs`), which only chooses its validation, and D46 makes it run the same bytes the CLI validated.
- [ ] **Step 4:** run the same tests, then `sh scripts/gate.sh fast`. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -s -m "CLI: unlisted connectors load strict, dev is sent and never competes (D70)"`.

### Task 5: CLI: tell the player before approval

**Files:**
- Modify: `crates/goalcup-cli/src/upload/api.rs` (`accepted_connectors()` client for Task 3's route)
- Modify: `crates/goalcup-cli/src/commands/start/server.rs` (in the pre-approval checks of a competition run)
- Test: `crates/goalcup-cli/tests/it/` (same file as Task 4)

**Interfaces:**
- Consumes: `GET /v1/connectors/accepted` (Task 3).
- Produces: a competition start whose digest is not in the list is refused before approval with `NOT_ACCEPTED`. If the list cannot be read (any error, 404 from an older Worker, timeout of 5 s), the start goes on and the server decides at open.

- [ ] **Step 1: Write the failing tests:** `an unaccepted connector is refused before approval when the list says so`; `an unreadable list does not stop the start` (fake server answers 500 and then accepts the open).
- [ ] **Step 2:** run them. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** run them and `sh scripts/gate.sh fast`. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -s -m "CLI: check the accepted list before approval (D70)"`.

### Task 6: Docs and the decision

**Files:**
- Modify: `docs/DECISIONS.md` (add D70 from the design's section 8; replace D68's "Open: the pin and the cutoff" bullet with "Settled by D70")
- Modify: `docs/CONNECTOR-SCHEMA.md` ("Pinned, or `--dev` only" becomes "Accepted on the server, or `--dev` for admins")
- Modify: `docs/HARNESS-ONBOARDING.md` ("Getting a harness into the event": a reviewed row, no release; practise and test first; the folder copy and `GOALCUP_CONNECTORS_DIR`)
- Modify: `docs/RULES.md`, `docs/PARTICIPANT-GUIDE.md` (own harness; `--dev` is for admins)
- Modify: `docs/UPLOAD-API.md` (`dev` field, the three 403 messages, `GET /v1/connectors/accepted`)
- Modify: `docs/OPERATING.md` (how an operator accepts a connector: compute its digest with `goalcup doctor` on the reviewed folder, confirm it loads with the strict rules (no `--dev`), then the exact `INSERT` with `connector_id`, `accepted_by` and `note`, run against the shared database; how to remove one; that a removal stops new opens and continues only)
- Modify: `docs/CHEAT-CHECKLIST.md` (a modified goalcup can report an accepted digest while running other bytes, as with the pins; the trace review is the control)
- Modify: `HARNESS-ONBOARDING.md` and the design's section 6 facts: a stray file refuses a folder, one dev-only folder blocks strict starts from that directory, a running daemon keeps its directory until 30 s after its last seal, the variable must stay set for the whole run
- Modify: `crates/goalcup-cli/src/preflight/gates.rs` doc comments and `docs/harnesses/*.md` where they say a release is needed

- [ ] **Step 1:** `git grep -n -i -E 'pinned|--dev|6 Nov 2026 12:00' docs crates/goalcup-cli/src` and list every place that says a new harness needs a release or that `--dev` is open to everyone.
- [ ] **Step 2:** update each place. Keep player text plain and short, no em dashes.
- [ ] **Step 3:** `sh scripts/gate.sh fast` (link and docs checks). Expected: PASS.
- [ ] **Step 4: Commit** `git commit -s -m "Docs: D70, accepted connectors and admin-only dev"`.

### Task 7: Rollout (operator steps, owner approves each)

Not code. Do these in order and record each result in the PR:

0. Read-only, before applying: count open competition runs (all events, staging and production) whose latest entry's `(connector_dir_sha256, connector_id)` is not one of the four seeded pairs. Those runs cannot `continue` after the migration. Tell their players first, or wait until they end.
1. Copy the migration to `agentCommunity_PAGE/supabase/migrations/` unchanged; merge it there.
2. Apply once: `psql "$DATABASE_URL_REMOTE" -X -1 -v ON_ERROR_STOP=1 -f <file>`; then `pnpm db:check` in the website repo. The pending `20261021100000` file must be applied first if it is not yet.
3. Straight after: deploy the staging Worker and the production Worker (the old Worker turns the new continue refusal into a `502` that goalcup retries forever).
4. Check: `SELECT count(*) FROM goal_cup_tracer.accepted_connectors` is 4. Then a competition start on staging (event `goalcup-dryrun-staging`, window open to 5 Nov) with v0.6.1 and a shipped connector opens. Do not smoke-test with a production competition start: it writes a real competition row. A test-mode run does not exercise the check.
5. Run the Task 2 cases against staging with a test member and with an admin member. Staging test members are synthetic (no `auth.users` row), so they are never admins: an admin case needs a real admin account.
6. Ship Tasks 4 and 5 in the release before 6 Nov 2026 12:00 UTC.
