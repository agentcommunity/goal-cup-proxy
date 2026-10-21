# Pending website migrations

These migrations were written in this repository first, because the website repository
(`agentCommunity_PAGE`), which owns the migrations, was not at hand. Each one is also the last
section of the mirror `../goal-cup-tracer-upload.sql`, byte for byte, so the Worker's SQL parity
test (`test/supabase.test.ts`) already checks the Worker against it.

Copy each file into the website repo's `supabase/migrations/`, unchanged and with the same name,
and merge it there. Apply it to staging first, then to production: the lead agent applies, the
owner approves. All are additive: functions, columns and one widened CHECK constraint, no row
changes. Apply them, in name order, before deploying the Worker that needs them (it calls the new
`goal_cup_upload_commit_chunk`, `goal_cup_upload_heartbeat` and `goal_cup_upload_record_final_commit`
overloads); the Worker that is live keeps working on them. Then regenerate the mirror from the
website repo (command in its header) and delete this folder.

- `20261021100000_goal_cup_tracer_test_runs_parallel.sql`: only competition replaces; test runs of
  one operator run side by side (owner, 21 Oct 2026).
- `20261021110000_goal_cup_tracer_run_size_limit.sql`: `stored_bytes` and `chunk_count` per entry,
  run totals in every run answer, and `goal_cup_upload_commit_chunk(…, p_stored_bytes)` for the
  per-run size limit (WP-D, `RUN_MAX_COMPRESSED_BYTES`).
- `20261021120000_goal_cup_tracer_vitals_disk_notice.sql`: the heartbeat's optional `vitals`
  (`heartbeats.vitals`, `runs.last_vitals`), the run's disk pause (`runs.disk_paused_at`,
  `runs.disk_notice_at`), the notice kind `disk_full` with its email, `disk_notice_at` in
  `goal_cup_upload_runs_for_tick`, and `goal_cup_upload_heartbeat(…, p_vitals)`.
- `20261021130000_goal_cup_tracer_final_push.sql`: the judged commit is the last push to the default
  branch that GitHub timestamped at or before `ends_at` (owner, 21 Oct 2026): `runs.final_pushed_at`,
  `final_branch`, `final_late_push`, `final_force_push`, `final_head_fallback`, and
  `goal_cup_upload_record_final_commit(…, p_pushed_at, p_branch, p_late_push, p_force_push, p_head_fallback)`.
