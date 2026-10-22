-- Goal Cup Proxy: live community totals for GET /v1/live.json (2026-10-22). Step (a) of the live
-- stats design (goal-cup-proxy PR #241, docs/research/LIVE-STATS-DESIGN.md).
--
-- One new function, public.goal_cup_upload_live_snapshot(p_event_id, p_at). The Worker's 5-minute
-- cron calls it once per tick, writes the answer to R2 as live/{event_id}.json, and the public
-- GET /v1/live.json serves that copy. No request reads the database. It reads data already stored:
-- events, runs, entries, heartbeats, notices. No table, column or row changes.
--
-- What it counts:
--   * Only competition runs (runs.mode = 'competition') of p_event_id, of the caller's own
--     deployment (runs.staging = caller_is_staging(), and the event too), not abandoned, started at
--     or before p_at. Test runs (mode 'test') and practice runs (mode 'practice') are never counted,
--     here in the SQL, so no caller can forget.
--   * "Entry" counts (started, running, working, quiet, finished, sealed, incomplete, harnesses) count
--     main runs only: a run a later start replaced is not an entry.
--   * "Work" totals (agent hours, model calls, restarts, data) count every competition run of the
--     event, replaced ones too: that work happened.
--
-- Privacy: community totals only. No member id, email, repository, IP address, country, machine
-- fact, vitals, run id, entry id or session id is read into the answer. Every number is null unless
-- at least min_group (5) runs feed it: a count of runs is null below 5; a sum is null unless 5 runs
-- contributed to it. A harness with fewer than 5 started runs folds into "other", and "other" is left
-- out while it has fewer than 5. A connector id that is not lowercase letters, digits and dashes
-- also folds into "other". Owner question (PR #241): is 5 right?
--
-- The answer, schema 1 (the Worker checks it key by key before it writes anything):
--   {"schema_version": 1,
--    "event": {"id", "kind", "start_window_opens", "start_window_closes", "run_hours"},
--    "generated_at": p_at, "min_group": 5,
--    "entries": {"signed_up": null, "started", "running", "working", "quiet", "finished", "sealed", "incomplete"},
--    "harnesses": [{"id", "started", "running"}] or null,
--    "work": {"agent_hours", "working_hours", "model_calls", "model_calls_last_hour", "restarts", "data_bytes"},
--    "notices": {"silence", "idle_3h", "idle_6h", "disk_full"},
--    "survival": null, "hourly": null, "trace": null, "github": null}
-- or NULL when p_event_id is not an event of the caller's deployment.
--   * running: before its end, last heartbeat under 20 minutes old (the silence threshold);
--     working: running, and last model request under 30 minutes old; quiet: before its end, no
--     heartbeat (or start) for 20 minutes or more; finished: past its end (sealed and incomplete as
--     the close wrote them). signed_up stays null: the sign-up table is not in this schema yet.
--   * agent_hours: per run, the distinct 5-minute slots with a heartbeat, times 5 minutes, summed,
--     to one decimal. working_hours: the same, counting only slots with a heartbeat whose
--     model_requests is above every earlier one of its entry (the run's model calls grew).
--   * model_calls: the sum of entries.model_requests (each entry counts from its own start, so a
--     continue adds and never double counts). model_calls_last_hour: per entry, the highest count a
--     heartbeat reported up to p_at minus the highest up to an hour before (0 if none), summed; fed
--     by the runs with a heartbeat in that hour.
--   * restarts: entries with segment > 0. data_bytes: the sum of entries.stored_bytes.
--   * notices: rows per kind; each fed by the runs that have one.
--   * survival and hourly (later steps of the design) are null for now; the Worker adds "server": null.
--
-- Cost: one pass over the event's heartbeats (index (run_id, received_at)), small counts otherwise.
-- STABLE; it writes nothing.
--
-- Apply ONCE: staging and production share one database. First run it on a throwaway Postgres
-- loaded with the mirror (server/worker/scripts/check-sql.sh in goal-cup-proxy). Then copy it into
-- the website repo's supabase/migrations/ unchanged, and the lead agent applies it, the owner
-- approves:
--   psql "$DATABASE_URL_REMOTE" -X -1 -v ON_ERROR_STOP=1 -f <this file>; then pnpm db:check.
-- Then deploy the Worker that calls it. A Worker without it never calls it; a Worker with it and a
-- database without it logs live-snapshot-failed each tick and changes nothing else.

CREATE FUNCTION public.goal_cup_upload_live_snapshot(p_event_id text, p_at text) RETURNS jsonb
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
  WITH ev AS (
    SELECT e.* FROM events e WHERE e.id = p_event_id AND e.staging = caller_is_staging()
  ),
  now_ AS (SELECT p_at::timestamptz AS t),
  -- Every competition run of the event that has started.
  c AS (
    SELECT r.id, r.status, r.official_start, r.last_heartbeat_at, r.last_model_request_at,
           least(r.deadline_at, coalesce(r.replaced_at, r.deadline_at)) AS ends_at,
           (r.replaced_at IS NOT NULL AND r.replaced_at < r.deadline_at) AS replaced
      FROM runs r JOIN ev ON ev.id = r.event_id
     WHERE r.mode = 'competition' AND r.staging = caller_is_staging() AND r.status <> 'abandoned'
       AND r.official_start <= (SELECT t FROM now_)
  ),
  -- The main runs (the entries), what each is now, and its harness (segment 0's connector).
  m AS (
    SELECT c.status,
           n.t < c.ends_at AND c.last_heartbeat_at >= n.t - interval '20 minutes' AS running,
           n.t < c.ends_at AND c.last_heartbeat_at >= n.t - interval '20 minutes'
             AND c.last_model_request_at >= n.t - interval '30 minutes' AS working,
           n.t < c.ends_at AND coalesce(c.last_heartbeat_at, c.official_start) < n.t - interval '20 minutes' AS quiet,
           n.t >= c.ends_at AS finished,
           (SELECT CASE WHEN en.connector_id ~ '^[a-z0-9][a-z0-9-]{0,63}$' THEN en.connector_id ELSE 'other' END
              FROM entries en WHERE en.run_id = c.id AND en.segment = 0) AS harness
      FROM c CROSS JOIN now_ n
     WHERE NOT c.replaced
  ),
  counts AS (
    SELECT count(*) AS started,
           count(*) FILTER (WHERE running) AS running,
           count(*) FILTER (WHERE working) AS working,
           count(*) FILTER (WHERE quiet) AS quiet,
           count(*) FILTER (WHERE finished) AS finished,
           count(*) FILTER (WHERE finished AND status = 'sealed') AS sealed,
           count(*) FILTER (WHERE finished AND status = 'incomplete') AS incomplete
      FROM m
  ),
  by_harness AS (
    SELECT coalesce(harness, 'other') AS id, count(*) AS started, count(*) FILTER (WHERE running) AS running
      FROM m GROUP BY 1
  ),
  harness AS (
    SELECT CASE WHEN started >= 5 THEN id ELSE 'other' END AS id, sum(started) AS started, sum(running) AS running
      FROM by_harness GROUP BY 1
  ),
  ent AS (
    SELECT sum(en.model_requests) AS model_calls,
           count(DISTINCT en.run_id) FILTER (WHERE en.model_requests IS NOT NULL) AS calling_runs,
           count(*) FILTER (WHERE en.segment > 0) AS restarts,
           count(DISTINCT en.run_id) FILTER (WHERE en.segment > 0) AS restarted_runs,
           sum(en.stored_bytes) AS data_bytes,
           count(DISTINCT en.run_id) FILTER (WHERE en.stored_bytes > 0) AS storing_runs
      FROM entries en JOIN c ON c.id = en.run_id
  ),
  hb AS (
    SELECT h.id, h.run_id, h.entry_id, h.received_at, h.model_requests
      FROM heartbeats h JOIN c ON c.id = h.run_id
     WHERE h.received_at <= (SELECT t FROM now_)
  ),
  beats AS (
    SELECT count(DISTINCT h.run_id || ':' || floor(extract(epoch FROM h.received_at) / 300)::bigint) AS slots,
           count(DISTINCT h.run_id) AS beating_runs,
           count(DISTINCT h.run_id) FILTER (WHERE h.received_at > n.t - interval '1 hour') AS hour_runs
      FROM hb h CROSS JOIN now_ n
  ),
  -- Heartbeats at which the entry's model calls grew (above every earlier report of that entry).
  grew AS (
    SELECT h.run_id, h.received_at
      FROM (SELECT h.run_id, h.received_at, h.model_requests,
                   max(h.model_requests) OVER (PARTITION BY h.entry_id ORDER BY h.received_at, h.id
                                               ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING) AS before
              FROM hb h WHERE h.model_requests IS NOT NULL) h
     WHERE h.model_requests > coalesce(h.before, 0)
  ),
  working_slots AS (
    SELECT count(DISTINCT g.run_id || ':' || floor(extract(epoch FROM g.received_at) / 300)::bigint) AS slots FROM grew g
  ),
  hour_calls AS (
    SELECT sum(greatest(coalesce(upto_now, 0) - coalesce(upto_hour_ago, 0), 0)) AS calls
      FROM (SELECT max(h.model_requests) AS upto_now,
                   max(h.model_requests) FILTER (WHERE h.received_at <= n.t - interval '1 hour') AS upto_hour_ago
              FROM hb h CROSS JOIN now_ n GROUP BY h.entry_id) per_entry
  ),
  nt AS (
    SELECT nn.kind, count(*) AS n, count(DISTINCT nn.run_id) AS runs
      FROM notices nn JOIN c ON c.id = nn.run_id
     WHERE nn.kind IN ('silence', 'idle_3h', 'idle_6h', 'disk_full') AND nn.created_at <= (SELECT t FROM now_)
     GROUP BY nn.kind
  )
  SELECT jsonb_build_object(
    'schema_version', 1,
    'event', jsonb_build_object(
      'id', ev.id,
      'kind', ev.mode,
      'start_window_opens', fmt_time(ev.start_window_opens),
      'start_window_closes', fmt_time(ev.start_window_closes),
      'run_hours', ev.run_hours),
    'generated_at', fmt_time((SELECT t FROM now_)),
    'min_group', 5,
    'entries', jsonb_build_object(
      'signed_up', NULL,
      'started',    CASE WHEN k.started >= 5 THEN k.started END,
      'running',    CASE WHEN k.running >= 5 THEN k.running END,
      'working',    CASE WHEN k.working >= 5 THEN k.working END,
      'quiet',      CASE WHEN k.quiet >= 5 THEN k.quiet END,
      'finished',   CASE WHEN k.finished >= 5 THEN k.finished END,
      'sealed',     CASE WHEN k.sealed >= 5 THEN k.sealed END,
      'incomplete', CASE WHEN k.incomplete >= 5 THEN k.incomplete END),
    'harnesses', CASE WHEN k.started >= 5 THEN (
      SELECT coalesce(jsonb_agg(jsonb_build_object(
               'id', hh.id, 'started', hh.started, 'running', CASE WHEN hh.running >= 5 THEN hh.running END)
             ORDER BY hh.id = 'other', hh.started DESC, hh.id), '[]'::jsonb)
        FROM harness hh WHERE hh.started >= 5) END,
    'work', jsonb_build_object(
      'agent_hours',           CASE WHEN b.beating_runs >= 5 THEN round(b.slots * 5 / 60.0, 1) END,
      'working_hours',         CASE WHEN b.beating_runs >= 5 THEN round(w.slots * 5 / 60.0, 1) END,
      'model_calls',           CASE WHEN e.calling_runs >= 5 THEN e.model_calls END,
      'model_calls_last_hour', CASE WHEN b.hour_runs >= 5 THEN coalesce(hc.calls, 0) END,
      'restarts',              CASE WHEN e.restarted_runs >= 5 THEN e.restarts END,
      'data_bytes',            CASE WHEN e.storing_runs >= 5 THEN e.data_bytes END),
    'notices', (
      SELECT jsonb_object_agg(kinds.kind, (SELECT CASE WHEN nt.runs >= 5 THEN nt.n END FROM nt WHERE nt.kind = kinds.kind))
        FROM unnest(ARRAY['silence', 'idle_3h', 'idle_6h', 'disk_full']) AS kinds(kind)),
    'survival', NULL,
    'hourly', NULL,
    'trace', NULL,
    'github', NULL)
  FROM ev, counts k, ent e, beats b, working_slots w, hour_calls hc
$$;

-- ---- grants -------------------------------------------------------------------------------------

-- Supabase grants EXECUTE on new functions to PUBLIC, anon and authenticated; revoke all three
-- (docs/SUPABASE.md, "SECURITY DEFINER grants"). Then the Worker roles, like every goal_cup_upload_*.
REVOKE ALL ON FUNCTION public.goal_cup_upload_live_snapshot(text, text) FROM PUBLIC, anon, authenticated;
GRANT EXECUTE ON FUNCTION public.goal_cup_upload_live_snapshot(text, text)
  TO goal_cup_ingest_staging, goal_cup_ingest_production, service_role;
