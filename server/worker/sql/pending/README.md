# Pending website migration

**Needs the owner's OK before anyone applies it.** Nothing here is applied yet.

This migration was written in this repository first, because the website repository
(`agentCommunity_PAGE`), which owns the migrations, was not at hand. It is also the last section
of the mirror `../goal-cup-tracer-upload.sql`, byte for byte, so the Worker's SQL parity test
(`test/supabase.test.ts`) already checks the Worker against it.

`20261027100000_goal_cup_tracer_competition_repo_owner.sql` (its header has the detail): a
competition open is refused (`forbidden`, which the Worker answers as `403` with the message) when
its `repo_url` is already the repository of another member's competition run in the same event.
URLs compare in lower case with a trailing `.git` removed (`goal_cup_tracer.repo_key`). The same
member may name the repository again, and a lost-reply retry is still answered with its own run.
Practice and test runs are not checked. One new private function and a new body for
`goal_cup_upload_open_run` with the same signature; no table or row changes.

**It builds on the sign-up gate.** The new `open_run` body is `20261021120200_goal_cup_signup_gate.sql`'s
(applied 23 Oct), unchanged but for the repository block after the gate. A guard at the top fails
the apply, and rolls it back, unless `20261021120200` is in place. Any later change to `open_run`
in the website repo means this body must be rebuilt from that one.

Staging and production share ONE database and one `goal_cup_upload_open_run`, so applying the file
changes both Workers at once: there is no "staging first" for SQL. The check is scoped by event and
by the run's `staging` flag, so a staging run never blocks a production one. Every deployed Worker
already turns `forbidden` into `403` with the message, so no Worker deploy is needed for it to work.

The order:

1. Worker-side smoke on a throwaway Postgres loaded with the mirror:
   `sh server/worker/scripts/check-sql.sh` (needs a local Postgres: `initdb`, `pg_ctl`, `psql`).
2. Copy the file into the website repo's `supabase/migrations/`, unchanged and with the same name.
   In the same website PR:
   - append it to `GOAL_CUP_MIGRATIONS` in `supabase/migrations/__tests__/support/goal-cup-cluster.ts`
     (an explicit list: a file not in it is never applied by the website's tests);
   - add it to the `sqlFiles` of the `goal-cup-tracer-upload` and `goal-cup-signups` suites in
     `scripts/ci/postgres-suite-catalog.mjs` (the suites that list `20261021120200`);
   - add a website test: a signed-up member opens; a member with no sign-up is still refused with
     the sign-up message; another member's open of the same repository is refused.
   The website's goal-cup PostgreSQL suites are the authoritative check. Merge there.
3. Read-only, before the apply: repositories that more than one member's competition runs in the
   dry-run event already use. After the apply those members cannot reuse them in that event (the shared
   practice repository `goalcup-matrix-empty` may show up here if a dry run named it). Read the
   list and tell the owner before applying:
   ```sh
   set -a; source .env.local; set +a
   psql "$DATABASE_URL_REMOTE" -X -v ON_ERROR_STOP=1 <<'SQL'
   SELECT r.event_id, r.staging, regexp_replace(lower(r.repo_url), '\.git$', '') AS repo,
          count(DISTINCT r.member_id) AS members, count(*) AS runs
     FROM goal_cup_tracer.runs r
    WHERE r.mode = 'competition' AND r.event_id IN ('goalcup-dryrun-2026', 'goalcup-dryrun-staging')
    GROUP BY 1, 2, 3
   HAVING count(DISTINCT r.member_id) > 1
    ORDER BY 1, 4 DESC;
   SQL
   ```
   Drop the `event_id` condition to see every competition event.
4. With the owner's OK, apply it once from the website checkout (the lead agent applies, the owner
   approves), in one transaction after `SET lock_timeout = '5s'`, as the website's
   `docs/admin/GOAL-CUP-SIGNUP-RUNBOOK.md` does. `open_run` is hot during the window: a lock it
   cannot get fails in 5 seconds and applies nothing; wait and re-run.
   ```sh
   set -a; source .env.local; set +a
   psql "$DATABASE_URL_REMOTE" -X -v ON_ERROR_STOP=1 <<'SQL'
   SET lock_timeout = '5s';
   BEGIN;
   \i supabase/migrations/20261027100000_goal_cup_tracer_competition_repo_owner.sql
   COMMIT;
   SQL
   ```
   Expect the last line `COMMIT`. Then, read-only: `select position('requires_signup' in
   pg_get_functiondef('public.goal_cup_upload_open_run'::regproc)) > 0 and position('repo_key' in
   pg_get_functiondef('public.goal_cup_upload_open_run'::regproc)) > 0;` answers `t` (both the gate
   and the repository check are in). Then `pnpm db:check`.
5. Regenerate the mirror from the website repo (command in its header), check it is unchanged
   below `REGEN-END`, delete this file and the migration here, and change the mirror's header to
   say when it was applied.

The previous pending migration, `20261021100000_goal_cup_tracer_event_readiness.sql`, is in the
website repo's main and was applied to the shared database on 22 Oct 2026 (owner approved,
`pnpm db:check` passed). It is now only a section of the mirror.
