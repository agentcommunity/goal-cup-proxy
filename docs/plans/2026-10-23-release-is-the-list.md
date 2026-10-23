# The release is the list; `--dev` for admins (D70) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Players run the latest release (nudged, and the server refuses old versions in the competition), nobody updates over a recording run, `--dev` works only for admins and never competes, and setup warns about harness versions known not to record.

**Architecture:** No server list of connectors: the release's compiled pins stay the list. The Worker gains a `dev` flag, an admin check through one new SQL function, and a minimum competition version. The CLI gains the `--dev` rules, a latest-release check, a known-bad-version warning, a practice reminder and shorter practice thresholds. `install.sh` gains a guard.

**Tech Stack:** PostgreSQL (Supabase, plpgsql), Cloudflare Worker (TypeScript, `node --test`), Rust CLI (`goalcup-cli`, `goalcup-protocol`), POSIX `sh` (installer).

**Spec:** [`docs/research/RELEASE-IS-THE-LIST.md`](../research/RELEASE-IS-THE-LIST.md)

## Global Constraints

- The D70 release ships before **6 Nov 2026 12:00 UTC**; releases may follow during the cup. From the D70 release to 15 Nov 2026, `CONTROL_API_VERSION` does not change (Task 4 pins it).
- Tasks 4, 5 and 6 (CLI) start only after #243 and #257 are merged to `main`, in that order (4, then 5, then 6). Tasks 1 to 3 do not wait. If #257 has not merged by 30 Oct, ship Tasks 1 to 3, set `MIN_COMPETITION_VERSION` only once a release with Task 4 exists, and tell the owner which of 5 and 6 are cut.
- Staging and production share ONE database. The migration only adds one function. Write it in `server/worker/sql/pending/` first, append it byte for byte to the mirror `server/worker/sql/goal-cup-tracer-upload.sql`, then copy it unchanged to the website repo (`agentCommunity_PAGE/supabase/migrations/`), following `server/worker/sql/pending/README.md`. File name: `20261024100000_goal_cup_tracer_member_is_admin.sql`.
- New SQL function grants: `REVOKE ALL ... FROM PUBLIC, anon, authenticated` (revoking `PUBLIC` alone is a no-op on Supabase), then grant `EXECUTE` exactly as the other `goal_cup_upload_*` functions are granted. `pnpm db:check` in the website repo stays green.
- Admin = the member's `auth.users` row has `email_confirmed_at IS NOT NULL` and `lower(btrim(email))` equals `lower(btrim(public.admins.email))` for some row.
- Version rule: the first `<digits>.<digits>.<digits>` in a string; anything after it (a `-dev` suffix, a build note) is ignored. A string with no such match is "unknown". The CLI sends its version as `proxy_version` and `tracer_version` (`CARGO_PKG_VERSION`); the Worker reads it with `proxyVersionField`.
- Install line (one place in code, `crates/goalcup-cli/src/participant/update.rs`, `INSTALL_LINE`): `curl -fsSL https://github.com/agentcommunity/goal-cup-proxy/releases/latest/download/install.sh | sh`
- Exact copy (tests assert it):
  - `TOO_OLD`: `This goalcup ({version}) is too old for the cup. Update it when no run is open: {INSTALL_LINE}`
  - `DEV_COMPETES`: `A --dev run never enters the competition. Start without --dev.`
  - `DEV_NOT_ADMIN`: `--dev is for Goal Cup admins only. Start without --dev.`
  - `DEV_WARNING` (the whole pin-gate warning for an unpinned connector with `--dev`): `` `{id}` is not pinned by this build ({digest}). This is a --dev run: it is for Goal Cup admins only, and it never enters the competition. ``
  - `UPDATE_NUDGE`: `goalcup {latest} is out (you have {current}). Update when no run is open: {INSTALL_LINE}`
  - `BAD_VERSION`: `{harness} {version} {why}. Fix: {fix}`
  - `PRACTICE_REMINDER`: `You have not finished a practice run with {harness} on this computer. Try one first: goalcup start --practice --harness {id}`
  - `INSTALL_BLOCKED`: `A Goal Cup run is recording on this computer. Update after it ends, or run the installer again with: curl -fsSL https://github.com/agentcommunity/goal-cup-proxy/releases/latest/download/install.sh | sh -s -- --force`
  - `INSTALL_UNSURE`: `Could not tell whether a Goal Cup run is recording. If one is, stop this update and run it after the run ends.`
- "A run is open on this computer": a local run record whose deadline is in the future and that is not sealed.
- Player-facing text: plain short English, no em dashes. No secrets in logs or argv. Never set a model base URL.
- Every commit is signed off (`git commit -s`, DCO check). Rust: `PATH="$HOME/.cargo/bin:$PATH"` (1.99). Worker: `cd server/worker && npm test`. SQL: `sh server/worker/scripts/check-sql.sh`. Connector edits: `sh scripts/pin-manifests.sh` in the same commit. Do not edit the frozen schema-1 fixtures (D68).

## Review Focus

1. **The latest-release check is slow, offline, or in a test:** setup and start wait at most 3 s (DNS included), say nothing on any failure or under `--json`, and a test-support build makes no request unless `GOALCUP_TEST_RELEASES_URL` is set (Task 5 tests).
2. **An update over a recording run:** the installer stops; "no run on record" goes on quietly; a goalcup without `status --json`, or one that errors, gives `INSTALL_UNSURE` and goes on (Task 3 tests).
3. **`--dev` in tests and tools:** 13 CLI test files and the D68 frozen contract tests use `--dev` without a login; the local refusal applies only to a start that opens a server run, so they keep passing untouched (Task 4 test).
4. **A harness that prints its version in an odd way** (`Hermes Agent v0.22.4 (2026.10.21)`, `0.19.0`, `unknown`): the warning matches the right versions and stays quiet on unknown (Task 6 tests).
5. **`MIN_COMPETITION_VERSION` empty or malformed:** empty means no limit; malformed refuses competition opens and device requests only (500 `misconfigured`), never heartbeats or uploads (Task 2 test).

---

### Task 1: Database: the admin check

**Files:**
- Create: `server/worker/sql/pending/20261024100000_goal_cup_tracer_member_is_admin.sql`
- Modify: `server/worker/sql/goal-cup-tracer-upload.sql` (append as the last section), `server/worker/sql/pending/README.md`
- Modify: `server/worker/scripts/check-sql.sh` (stubs and checks below)

**Interfaces:**
- Produces: `public.goal_cup_upload_member_is_admin(p_member_id text) RETURNS boolean` (STABLE, SECURITY DEFINER, `search_path = goal_cup_tracer, pg_temp`; schema-qualify `auth.users` and `public.admins`).

- [ ] **Step 1: Add checks to `check-sql.sh`** (after the pending files are applied; a wrong answer fails the script):
  - stub `CREATE TABLE public.admins (email text NOT NULL)` with the other Supabase stubs (add `email_confirmed_at` to the `auth.users` stub if missing);
  - a confirmed user `admin@example.org` with `' Admin@Example.org '` in `public.admins`: `true`;
  - an unconfirmed user whose email is in `public.admins`: `false`;
  - a confirmed user not in `public.admins`: `false`; an unknown uuid: `false`;
  - `has_function_privilege('anon', 'public.goal_cup_upload_member_is_admin(text)', 'EXECUTE')` and the same for `authenticated`: `false`.
- [ ] **Step 2:** `sh server/worker/scripts/check-sql.sh`. Expected: FAIL (no function).
- [ ] **Step 3: Write the migration** (header in the style of `20261021100000`: what, why, D70, additive only, apply command), append it to the mirror, update the pending README.
- [ ] **Step 4:** `sh server/worker/scripts/check-sql.sh` and `cd server/worker && npm test` (mirror parity). Expected: PASS.
- [ ] **Step 5: Commit** `git commit -s -m "Database: an admin check for --dev runs (D70)"`.

### Task 2: Worker: `dev`, the admin check, and the minimum competition version

**Files:**
- Modify: `server/worker/src/runs.ts` (`openRun`, `continueRun`), `server/worker/src/device.ts` (competition device requests)
- Modify: `server/worker/src/util.ts` (`boolField(o, name, fallback: boolean): boolean`, non-boolean is `malformed`; `versionTriple(text: string): [number, number, number] | null` per the version rule)
- Modify: `server/worker/src/db.ts`, `db-supabase.ts`, `db-local.ts` (`memberIsAdmin(member_id: string): Promise<boolean>`; `LocalDb.create` takes an admin set, also read from a `LOCAL_ADMIN_MEMBERS` var, comma-separated, for `wrangler dev`)
- Modify: `server/worker/src/env.ts` and `server/worker/src/context.ts` (`minCompetitionVersion: { kind: "none" } | { kind: "set"; v: [number, number, number] } | { kind: "malformed"; raw: string }` from `MIN_COMPETITION_VERSION`; empty or unset is `none`; reading config never throws for it)
- Modify: `server/worker/wrangler.toml` (`MIN_COMPETITION_VERSION = ""` in `[vars]` of every environment)
- Test: `server/worker/test/lifecycle.test.ts`, `server/worker/test/config.test.ts`, `server/worker/test/supabase.test.ts`, `server/worker/test/device.test.ts`; the admin-call counter goes on the `LocalDb` test double in `server/worker/test/helpers/`

**Interfaces:**
- Consumes: Task 1's function.
- Produces: `POST /v1/runs` and `POST /v1/runs/{id}/continue` accept optional boolean `dev` (missing is `false`). In `openRun` the checks run after the lost-reply return and before the GitHub check; in `continueRun` before the database call. Order and answers:
  1. `TOO_OLD`, 403 `forbidden` (competition device request and competition open only; the sent version is below the minimum, or unknown while a minimum is set). A malformed minimum gives 500 `misconfigured` at the same two points only.
  2. `DEV_COMPETES`, 403 (`dev` and the run's mode is competition).
  3. `DEV_NOT_ADMIN`, 403 (`dev` and `memberIsAdmin` is false; called only when `dev` is true).

- [ ] **Step 1: Write the failing tests:**
  - lifecycle: `a dev competition open is refused` (403, `DEV_COMPETES`, also for an admin); `a dev practice open by a non-admin is refused` (403, `DEV_NOT_ADMIN`); `a dev practice open by an admin opens` (201); `a dev continue of a competition run is refused` (403, `DEV_COMPETES`); `an open without dev never asks whether the member is an admin` (counter stays 0); `dev must be a boolean` (`"yes"` gives 400); `an old goalcup cannot open a competition run once a minimum is set` (a minimum set, a request from an older release: 403 `TOO_OLD` naming that older release); `the minimum does not touch continue or practice` (both work from the older release); `a version with a suffix compares by its numbers` (`0.7.0-dev` passes `0.7.0`); `a retried open of an existing run is answered even below the minimum`; `a malformed minimum refuses competition opens and competition device requests and leaves heartbeats, chunk uploads, get and continue alone` (500 `misconfigured` on open; heartbeat 200); `a dev that is not a boolean, null included, is refused as malformed` (400).
  - device: `a competition device request below the minimum is refused with TOO_OLD`.
  - config: `an empty minimum is no limit`; `a malformed minimum is reported, not thrown` (`"0.7"`).
  - supabase: `memberIsAdmin calls goal_cup_upload_member_is_admin with p_member_id`.
- [ ] **Step 2:** `cd server/worker && npm test`. Expected: the new tests FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** `npm test`. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -s -m "Worker: dev runs for admins only, never competing; a minimum competition version (D70)"`.

### Task 3: Installer: never update over a recording run

**Files:**
- Modify: `packaging/install.sh` (also `repo=` and the usage line: `goal-cup-tracer` becomes `goal-cup-proxy`)
- Test: `scripts/test_install.py` (runs in `sh scripts/gate.sh full`, not `fast`)

**Interfaces:**
- Produces: before it replaces anything, `install.sh` finds the goalcup it would replace (the target prefix's `bin/goalcup` first, then the one on `PATH`). It runs `goalcup status --json`, stopped after 10 s by a background process and `kill` (macOS has no `timeout`). Reading the output (the current release and `main` print `{payload, public_key, signature, cached_at, local}`):
  - `payload.health == "online"` and the capture's run state is `starting`, `running` or `exited`: recording. Print `INSTALL_BLOCKED`, exit 1, replace nothing. A `sealed` run does not block.
  - the "no run on record" error: not recording, go on quietly.
  - the daemon cannot be reached (it exits about 30 s after the last seal, so nothing is recording): go on quietly.
  - anything else (no such flag, other errors, unparsable output, the time limit): print `INSTALL_UNSURE` and go on.
  - `--force` (and `GOALCUP_FORCE=1`) skips the guard.
  Note in the script that `status` may spend up to 3 s sending leftover uploads, and that it looks only at the newest run on record.

- [ ] **Step 1: Write the failing tests** with a fake `goalcup` on `PATH` and in the prefix: `a recording run blocks the install` (exit 1, `INSTALL_BLOCKED`, nothing replaced); `an exited, unsealed run blocks`; `a sealed run does not block`; `an unreachable daemon goes on quietly`; `no run on record goes on quietly`; `a goalcup without status --json gives the unsure line and goes on`; `a hanging status is stopped after 10 s and gives the unsure line`; `the prefix copy is asked before the one on PATH`; `force skips the guard`.
- [ ] **Step 2:** `python3 -I scripts/test_install.py`. Expected: FAIL.
- [ ] **Step 3: Implement** in POSIX `sh` (no bash); document `--force` in the usage header.
- [ ] **Step 4:** the tests, then `sh scripts/gate.sh full`. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -s -m "install.sh: never update over a recording run (D70)"`.

### Task 4: CLI: the `--dev` rules and the frozen control API

Start after #243 and #257 are on `main`.

**Files:**
- Modify: `crates/goalcup-cli/src/upload/api.rs` (`#[serde(default, skip_serializing_if = "std::ops::Not::not")] pub dev: bool` on `OpenRunRequest` and `ContinueRequest`)
- Modify: `crates/goalcup-cli/src/commands/start/server.rs`, `crates/goalcup-cli/src/commands/continue_run.rs` (fill `dev`; a continued run reads it from its stored record)
- Modify: `crates/goalcup-cli/src/participant/refusals.rs` (`DEV_COMPETES`), called from `crates/goalcup-cli/src/commands/start/refusals.rs` only when the start opens a server run (an upload server is configured) and the run is competition
- Modify: `crates/goalcup-cli/src/preflight/gates.rs` (`dev_or_fail`'s warning becomes `DEV_WARNING`; the rest of the pin gate is unchanged)
- Modify: `crates/goalcup-cli/src/commands/status.rs` (a `dev` run says `(--dev run)` in its header line)
- Modify: `crates/goalcup-protocol/src/control.rs` tests (pin the version)
- Test: `crates/goalcup-cli/tests/it/` (the start and continue files that cover `--dev`)

**Interfaces:**
- Consumes: Task 2's wire field and refusals.

- [ ] **Step 1: Write the failing tests:** `dev with a competition run is refused before approval when logged in` (no device request reaches the fake server; output has `DEV_COMPETES`); `dev without a login still starts a local run` (the existing `--dev` tests keep passing unchanged); `dev sends dev true on open and continue`; `an open without dev sends no dev key`; `a server refusal of a dev run is shown in full` (fake server 403 `DEV_NOT_ADMIN`; no clone, no harness started); `status shows a dev run`; in `goalcup-protocol`: `assert_eq!(CONTROL_API_VERSION, <value on main after #243>, "D70: frozen until 15 Nov 2026")`.
- [ ] **Step 2:** `PATH="$HOME/.cargo/bin:$PATH" cargo test -p goalcup-cli --test it <names>` and `cargo test -p goalcup-protocol`. Expected: FAIL.
- [ ] **Step 3: Implement.** For `scripts/matrix.sh`'s fake cell (a practice `--dev` run against the local Worker), set `LOCAL_ADMIN_MEMBERS` to its member. A real `--dev` run on staging needs a real admin account: say so in `docs/OPERATING.md` (Task 7).
- [ ] **Step 4:** the same tests, then `sh scripts/gate.sh fast`. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -s -m "CLI: --dev never competes and is sent to the server; control API frozen (D70)"`.

### Task 5: CLI: update nudge, practice reminder, practice thresholds

**Files:**
- Create: `crates/goalcup-cli/src/participant/update.rs` (`INSTALL_LINE`; `latest_release(timeout: Duration) -> Option<String>`; `update_nudge(current: &str, latest: &str) -> Option<String>`)
- Create: `crates/goalcup-cli/src/participant/practice.rs` (`has_finished_practice(cfg: &Path, connector_id: &str) -> bool`: the predicate `prep.rs` `practice_step` used before #257 (a practice run, same `connector_id`, with a `server_url`, every entry uploaded), restored as a shared function; a `dev` run does not count)
- Modify: `crates/goalcup-cli/src/commands/prep.rs` (setup prints the nudge at its end when no run is open and not under `--json`)
- Modify: `crates/goalcup-cli/src/commands/start/` (the nudge only when no run is open; `PRACTICE_REMINDER` before approval for a test or competition start when `has_finished_practice` is false)
- Modify: `crates/goalcup-cli/src/run/mod.rs` and `crates/goalcup-cli/src/main.rs` (`NativeSilence::practice()` = 5 requests, 3 minutes; a practice run and a `continue` of a practice run use it)
- Test: unit tests in `update.rs`, `practice.rs` and `run/mod.rs`; `crates/goalcup-cli/tests/it/` for the start and setup output

**Interfaces:**
- `latest_release` asks `https://github.com/agentcommunity/goal-cup-proxy/releases/latest` with `redirects(0)` (as `upload/http.rs` already does) and reads the version from the `Location` header (`…/tag/v<version>`). It runs on a thread and the caller stops waiting after the timeout (`recv_timeout`), because ureq does not time out DNS lookups. Any error, a non-redirect answer, a bad `Location` or the timeout returns `None`. In a test-support build it asks `GOALCUP_TEST_RELEASES_URL`, and makes no request at all when that is unset.
- `update_nudge` returns `UPDATE_NUDGE` only when `latest` is newer than `current` by the version rule.

- [ ] **Step 1: Write the failing tests:** `update_nudge` (an older release against a newer one: the line; equal: `None`; `0.7.0-dev` vs `0.7.0`: `None`; unknown: `None`); `latest_release` against a fake server (302 with a `Location` ending in a newer release's tag gives that number; 200, 500, a bad `Location`, and a hang past the timeout give `None`, the hang within 3 s); `no release request without the test address`; `setup prints the nudge when a newer release exists`; `setup with --json prints no nudge`; `start prints no nudge while a run is open`; `start reminds about practice when none is on record`; `start does not remind after a finished practice run with that harness`; `a dev practice run does not count`; `NativeSilence::practice holds at 5 requests and 3 minutes`; `a continued practice run uses the practice thresholds`.
- [ ] **Step 2:** run them. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** run them, then `sh scripts/gate.sh fast`. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -s -m "CLI: tell players about new releases, remind them to practise (D70)"`.

### Task 6: Known-bad harness versions

**Files:**
- Modify: `crates/goalcup-protocol/src/harness/schema.rs` (`ParticipantSpec` gains optional `bad_versions: Vec<BadVersion>`, `BadVersion { from: String, before: Option<String>, why: String, fix: String }`; `from` inclusive, `before` exclusive; `why` and `fix` single line, at most 200 bytes)
- Modify: `crates/goalcup-protocol/src/harness/check.rs` (validate: versions parse, `from < before` when both exist)
- Modify: `crates/goalcup-cli/src/commands/prep.rs` (setup probes with the connector's `version_args` and prints `BAD_VERSION` on a match)
- Modify: `crates/goalcup-cli/src/commands/start/` (a probe before approval, with the same 10 s limit as the existing probe, printing `BAD_VERSION`; the existing post-approval probe in `phases.rs` is unchanged; never refuse)
- Modify: `connectors/hermes-cli/harness.toml` (`bad_versions = [{ from = "0.21.1", why = "writes no session file, so Goal Cup records only part of the run", fix = "uv tool install --force hermes-agent==0.19.0, then check hermes --version" }]`), then `sh scripts/pin-manifests.sh`. This changes the Hermes pin: players see the warning only after updating to the release that carries it.
- Modify: `docs/CONNECTOR-SCHEMA.md` (the new optional field, in "The connector contract")
- Test: protocol unit tests; `crates/goalcup-cli/tests/it/` setup and start output. New tests only: do not edit the frozen schema-1 fixtures (D68).

**Interfaces:**
- Produces: `pub fn bad_version_match<'a>(printed: &str, rules: &'a [BadVersion]) -> Option<&'a BadVersion>` in `goalcup-protocol`, using the version rule.

- [ ] **Step 1: Write the failing tests:** `bad_version_match` (`Hermes Agent v0.22.4 (2026.10.21)` matches `from 0.21.1`; `0.19.0` does not; `0.21.1` matches; `unknown` does not; a `before` bound excludes its own version); `a malformed bad_versions entry is refused at load`; `setup warns about a bad Hermes version and goes on` (fake `hermes` printing `0.22.4`); `start warns before approval and goes on`.
- [ ] **Step 2:** run them. Expected: FAIL.
- [ ] **Step 3: Implement**, edit the Hermes connector, regenerate pins.
- [ ] **Step 4:** run them, the contract and pin tests, then `sh scripts/gate.sh fast`. Expected: PASS.
- [ ] **Step 5: Commit** `git commit -s -m "Connectors may list known-bad harness versions; setup warns (D70)"`.

### Task 7: Docs and the decision

**Files:** `docs/DECISIONS.md` (D70 from the design's section 7; D68's open bullet becomes "Settled by D70"; D43 ruling 4 marked withdrawn by D70), `docs/RULES.md` (`--dev` is for admins and never competes; keep goalcup up to date, never during a run), `docs/PARTICIPANT-GUIDE.md` (updating and the `--force` line; practise first), `docs/HARNESS-ONBOARDING.md` (a new harness during the cup: merge and release; players update), `docs/OPERATING.md` (setting `MIN_COMPETITION_VERSION` in `wrangler.toml`; releasing during the cup without changing `CONTROL_API_VERSION`; staging `--dev` needs a real admin account), `docs/UPLOAD-API.md` (`dev`, the refusals, the device-request refusal), `docs/CHEAT-CHECKLIST.md` (a modified goalcup can report any digest, version or `dev`; the trace review is the control), the privacy section of `docs/RULES.md` or `docs/PARTICIPANT-GUIDE.md` (setup and start ask github.com for the latest release).

- [ ] **Step 1:** `git grep -n -i -E -- '--dev|6 Nov 2026 12:00|last release|goal-cup-tracer/releases' docs` and list every place that says `--dev` is open to all, that 6 Nov is the last release for a harness, or that links the old repository name.
- [ ] **Step 2:** update each place. Plain short English, no em dashes.
- [ ] **Step 3:** `sh scripts/gate.sh fast`. Expected: PASS.
- [ ] **Step 4: Commit** `git commit -s -m "Docs: D70"`.

### Task 8: Rollout (operator steps, owner approves each)

1. Copy the Task 1 migration to the website repo unchanged; merge it there. Apply once: `psql "$DATABASE_URL_REMOTE" -X -1 -v ON_ERROR_STOP=1 -f <file>`; then `pnpm db:check`.
2. Deploy the staging Worker (`MIN_COMPETITION_VERSION` empty). Check: a `--dev` practice open from a synthetic test member gives `DEV_NOT_ADMIN`; from the owner's admin account it opens; a `--dev` competition open gives `DEV_COMPETES`.
3. Deploy the production Worker with `MIN_COMPETITION_VERSION` empty.
4. Release the CLI (Tasks 4 to 6) before 6 Nov 2026 12:00 UTC.
5. Straight after: set `MIN_COMPETITION_VERSION = "<that release's x.y.z>"` in `wrangler.toml` `[vars]` for production (and staging), merge, deploy, before 6 Nov 2026 12:00 UTC. Older goalcups never send `dev`, so this is what stops their `--dev` runs from competing. An older goalcup shows the refusal inside its generic server-error wording; check it reads acceptably. Check: a competition device request from an older release on staging gives `TOO_OLD`.
6. Later, raise it only when an old version must stop competing.
