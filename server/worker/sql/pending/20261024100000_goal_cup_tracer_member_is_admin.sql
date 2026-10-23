-- Goal Cup Proxy: an admin check for --dev runs (D70, 2026-10-24).
--
-- What: one function, public.goal_cup_upload_member_is_admin(p_member_id text) RETURNS boolean.
-- Why: under D70 the server decides who may start a `--dev` run. A --dev run is a test run for Goal
-- Cup admins only; the Worker asks this function before it opens one, so a player who passes --dev
-- is refused. The website already keeps its admins in public.admins (email only), so this reads that
-- table and does not add a second list.
--
-- Rule: the member is an admin when their auth.users row has a confirmed email
-- (email_confirmed_at IS NOT NULL) and lower(btrim(email)) equals lower(btrim(public.admins.email))
-- for some row. An unconfirmed address never counts, whatever is in public.admins.
--
-- p_member_id is the auth.users id as text, the same as in the other goal_cup_upload_* functions.
-- A value that is not a uuid (or is NULL) answers false; it does not raise.
--
-- Additive only: one new function and its grants. No table, column or row changes, and nothing that
-- is live today calls it, so applying it changes nothing until the new Worker is deployed.
--
-- Apply ONCE: staging and production share one database, so there is no "staging first" for SQL.
-- First run it on a throwaway Postgres loaded with the mirror (server/worker/scripts/check-sql.sh in
-- goal-cup-tracer). Then the lead agent applies it, the owner approves:
--   psql "$DATABASE_URL_REMOTE" -X -1 -v ON_ERROR_STOP=1 -f <this file>; then pnpm db:check.
-- Then deploy and test the staging Worker, then deploy production.

CREATE FUNCTION public.goal_cup_upload_member_is_admin(p_member_id text) RETURNS boolean
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_member uuid;
BEGIN
  BEGIN
    v_member := p_member_id::uuid;
  EXCEPTION WHEN invalid_text_representation THEN
    RETURN false;
  END;
  RETURN coalesce((
    SELECT true
      FROM auth.users u
     WHERE u.id = v_member
       AND u.email_confirmed_at IS NOT NULL
       AND u.email IS NOT NULL
       AND EXISTS (SELECT 1 FROM public.admins a WHERE lower(btrim(a.email)) = lower(btrim(u.email)))
  ), false);
END
$$;

-- Supabase grants EXECUTE on new functions to PUBLIC, anon and authenticated; revoke all three
-- (docs/SUPABASE.md, "SECURITY DEFINER grants"). Then the same Worker roles as the other functions.
REVOKE ALL ON FUNCTION public.goal_cup_upload_member_is_admin(text) FROM PUBLIC, anon, authenticated;
GRANT EXECUTE ON FUNCTION public.goal_cup_upload_member_is_admin(text)
  TO goal_cup_ingest_staging, goal_cup_ingest_production, service_role;
