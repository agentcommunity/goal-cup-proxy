-- Goal Cup Proxy: purge expired device requests (2026-11-05).
--
-- A device request lives 15 minutes, but its row stayed for good: every `goalcup login` or start
-- left one behind. public.goal_cup_upload_purge_device_requests deletes the caller's deployment's
-- requests that expired more than a day before p_now, and returns how many it deleted. The Worker's
-- 5-minute cron calls it as its last step; a Worker deployed before this migration logs the missing
-- function and runs its other steps as before.
--
-- New function only; no table changes.

CREATE FUNCTION public.goal_cup_upload_purge_device_requests(p_now text) RETURNS integer
LANGUAGE sql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
  WITH gone AS (
    DELETE FROM device_requests
     WHERE staging = caller_is_staging() AND expires_at < p_now::timestamptz - interval '1 day'
    RETURNING 1
  )
  SELECT count(*)::integer FROM gone
$$;

-- Supabase grants EXECUTE on new functions to PUBLIC, anon and authenticated; revoke all three
-- (docs/SUPABASE.md, "SECURITY DEFINER grants"). Then the Worker roles.
REVOKE ALL ON FUNCTION public.goal_cup_upload_purge_device_requests(text) FROM PUBLIC, anon, authenticated;
GRANT EXECUTE ON FUNCTION public.goal_cup_upload_purge_device_requests(text)
  TO goal_cup_ingest_staging, goal_cup_ingest_production, service_role;
