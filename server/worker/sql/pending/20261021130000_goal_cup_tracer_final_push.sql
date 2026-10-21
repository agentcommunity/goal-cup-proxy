-- Goal Cup tracer: the judged commit is the last push before the deadline (owner, 2026-10-21).
--
-- The judged version of a run is the last push to the repository's default branch that GitHub
-- itself timestamped at or before the run's ends_at. Pushes after that do not count. The Worker's
-- cron reads it from GitHub's repository activity API (GET /repos/{owner}/{repo}/activity, the
-- `timestamp` GitHub gave each push) instead of the head GitHub has when asked; commit dates are
-- ignored. runs.final_commit keeps its meaning (that commit, or 'none'); these columns say how it
-- was found:
--   * runs.final_pushed_at: when GitHub received the judged push; null when not known.
--   * runs.final_branch: the default branch that was read.
--   * runs.final_late_push: GitHub had a push to that branch after ends_at when it was asked.
--   * runs.final_force_push: GitHub had a force push to that branch between official_start and ends_at.
--   * runs.final_head_fallback: the commit is the head GitHub had when asked (commits/HEAD), because
--     the activity API was unavailable or listed no update of the branch.
--   All null for rows recorded before this migration (and for 'none' from a missing repository).
--   * public.goal_cup_upload_record_final_commit(…, p_pushed_at, p_branch, p_late_push,
--     p_force_push, p_head_fallback): a new overload that stores the commit and these, once (the
--     first stays, as before). The Worker of this change calls it.
--   * The old 4-argument form stays for the Worker that is live while this is applied; it now calls
--     the new one with no detail. The new parameters have no DEFAULT on purpose: with one, PostgREST
--     could not choose between the two overloads for a call without them.
--
-- Order: apply this before deploying the Worker that sends p_pushed_at (staging, then production).
-- The live Worker keeps working on it unchanged. A new Worker on a database without this migration
-- cannot record a final commit (502, logged; the next tick retries).
--
-- Additive: five new nullable columns on runs (no table rewrite), one function replaced with the
-- same signature (grants kept), one new function overload with its grants. No row is changed.
--
-- Apply: staging first, then production (lead agent applies, owner approves):
--   psql "$DATABASE_URL_REMOTE" -X -1 -v ON_ERROR_STOP=1 -f <this file>; then pnpm db:check.

ALTER TABLE goal_cup_tracer.runs ADD COLUMN final_pushed_at     timestamptz;
ALTER TABLE goal_cup_tracer.runs ADD COLUMN final_branch        text CHECK (final_branch IS NULL OR length(final_branch) BETWEEN 1 AND 255);
ALTER TABLE goal_cup_tracer.runs ADD COLUMN final_late_push     boolean;
ALTER TABLE goal_cup_tracer.runs ADD COLUMN final_force_push    boolean;
ALTER TABLE goal_cup_tracer.runs ADD COLUMN final_head_fallback boolean;

-- The old form, for the Worker that is live while this is applied: no detail. Defined before the new
-- function on purpose: the mirror's parity test reads the last definition of each name.
CREATE OR REPLACE FUNCTION public.goal_cup_upload_record_final_commit(p_member_id text, p_run_id text, p_commit text, p_at text) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
BEGIN
  RETURN public.goal_cup_upload_record_final_commit(p_member_id, p_run_id, p_commit, p_at,
                                                    NULL::text, NULL::text, NULL::boolean, NULL::boolean, NULL::boolean);
END $$;

CREATE FUNCTION public.goal_cup_upload_record_final_commit(
  p_member_id text, p_run_id text, p_commit text, p_at text,
  p_pushed_at text, p_branch text, p_late_push boolean, p_force_push boolean, p_head_fallback boolean
) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_rows integer;
BEGIN
  UPDATE runs SET final_commit        = p_commit,
                  final_commit_at     = p_at::timestamptz,
                  final_pushed_at     = p_pushed_at::timestamptz,
                  final_branch        = p_branch,
                  final_late_push     = p_late_push,
                  final_force_push    = p_force_push,
                  final_head_fallback = p_head_fallback
   WHERE id = p_run_id AND member_id = p_member_id::uuid AND staging = caller_is_staging() AND final_commit IS NULL;
  GET DIAGNOSTICS v_rows = ROW_COUNT;
  RETURN v_rows = 1;
END $$;

-- Supabase grants EXECUTE on new functions to PUBLIC, anon and authenticated; revoke all three
-- (docs/SUPABASE.md, "SECURITY DEFINER grants"). Then the same Worker roles as the old form.
REVOKE ALL ON FUNCTION public.goal_cup_upload_record_final_commit(text, text, text, text, text, text, boolean, boolean, boolean) FROM PUBLIC, anon, authenticated;
GRANT EXECUTE ON FUNCTION public.goal_cup_upload_record_final_commit(text, text, text, text, text, text, boolean, boolean, boolean)
  TO goal_cup_ingest_staging, goal_cup_ingest_production, service_role;
