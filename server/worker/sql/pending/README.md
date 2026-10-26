# Pending website migration

**Needs the owner's OK before anyone applies it.** Nothing here is applied yet.

This migration was written in this repository first, because the website repository
(`agentCommunity_PAGE`), which owns the migrations, was not at hand. It is also the last section
of the mirror `../goal-cup-tracer-upload.sql`, byte for byte, so the Worker's SQL parity test
(`test/supabase.test.ts`) already checks the Worker against it.

`20261027100000_goal_cup_tracer_competition_repo_owner.sql` (its header has the detail): a
competition open is refused (`forbidden`, which the Worker answers as `403` with the message) when
its `repo_url` is already the repository of another member's competition run in the same event.
URLs compare in lower case with a trailing `.git` and `/` removed (`goal_cup_tracer.repo_key`). The
same member may name the repository again, and a lost-reply retry is still answered with its own
run. Practice and test runs are not checked. One new private function and a new body for
`goal_cup_upload_open_run` with the same signature; no table or row changes.

Staging and production share ONE database and one `goal_cup_upload_open_run`, so applying the file
changes both Workers at once: there is no "staging first" for SQL. The check is scoped by event and
by the run's `staging` flag, so a staging run never blocks a production one. Every deployed Worker
already turns `forbidden` into `403` with the message, so no Worker deploy is needed for it to work.

The order:

1. Test it on a throwaway Postgres loaded with the mirror: `sh server/worker/scripts/check-sql.sh`
   (needs a local Postgres: `initdb`, `pg_ctl`, `psql`).
2. Copy the file into the website repo's `supabase/migrations/`, unchanged and with the same name,
   and merge it there.
3. With the owner's OK, apply it once (the lead agent applies, the owner approves):
   `psql "$DATABASE_URL_REMOTE" -X -1 -v ON_ERROR_STOP=1 -f <file>`; then `pnpm db:check`.
4. Regenerate the mirror from the website repo (command in its header), check it is unchanged
   below `REGEN-END`, delete this file and the migration here, and change the mirror's header to
   say when it was applied.

The previous pending migration, `20261021100000_goal_cup_tracer_event_readiness.sql`, is in the
website repo's main and was applied to the shared database on 22 Oct 2026 (owner approved,
`pnpm db:check` passed). It is now only a section of the mirror.
