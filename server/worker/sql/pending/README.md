# Pending website migration

This migration was written in this repository first, because the website repository
(`agentCommunity_PAGE`), which owns the migrations, was not at hand. It is also the last section
of the mirror `../goal-cup-tracer-upload.sql`, byte for byte, so the Worker's SQL parity test
(`test/supabase.test.ts`) already checks the Worker against it.

`20261021100000_goal_cup_tracer_event_readiness.sql` holds five changes in one file, one
transaction (its header has the detail):

1. Only competition replaces; test runs of one operator run side by side (owner, 21 Oct 2026).
2. `stored_bytes` and `chunk_count` per entry, run totals in every run answer, and
   `goal_cup_upload_commit_chunk(…, p_stored_bytes)` for the per-run size limit (WP-D,
   `RUN_MAX_COMPRESSED_BYTES`).
3. The heartbeat's optional `vitals` (`heartbeats.vitals`, `runs.last_vitals`), the run's disk pause
   (`runs.disk_paused_at`, `runs.disk_notice_at`), the notice kind `disk_full`, `disk_notice_at` in
   `goal_cup_upload_runs_for_tick`, and `goal_cup_upload_heartbeat(…, p_vitals)`.
4. The judged commit is the last push to the default branch that GitHub timestamped at or before
   `ends_at` (owner, 21 Oct 2026): `runs.final_pushed_at`, `final_branch`, `final_late_push`,
   `final_force_push`, `final_head_fallback`, and
   `goal_cup_upload_record_final_commit(…, p_pushed_at, p_branch, p_late_push, p_force_push, p_head_fallback)`.
5. New words for the `silence`, `ended` and `disk_full` emails (docs/research/NOTIFICATIONS.md),
   in `notice_text`.

All additive: functions, columns and one widened CHECK constraint, no row changes. The
old-signature wrappers keep the live production Worker working.

Staging and production share ONE database, so applying the file applies it to both at once: there
is no "staging first" for SQL. The order:

1. Test it on a throwaway Postgres loaded with the mirror: `sh server/worker/scripts/check-sql.sh`
   (needs a local Postgres: `initdb`, `pg_ctl`, `psql`).
2. Copy the file into the website repo's `supabase/migrations/`, unchanged and with the same name,
   and merge it there.
3. Apply it once (the lead agent applies, the owner approves):
   `psql "$DATABASE_URL_REMOTE" -X -1 -v ON_ERROR_STOP=1 -f <file>`; then `pnpm db:check`.
4. Deploy and test the staging Worker, then deploy production. The Worker that is live keeps
   working on the migrated database; the new Worker calls the new
   `goal_cup_upload_commit_chunk`, `goal_cup_upload_heartbeat` and
   `goal_cup_upload_record_final_commit` overloads, so it needs the migration first.
5. Regenerate the mirror from the website repo (command in its header) and delete this folder.
