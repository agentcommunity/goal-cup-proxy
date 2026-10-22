-- Goal Cup Proxy: live community totals for GET /v1/live.json (2026-10-22). Step (a) of the live
-- stats design (goal-cup-proxy PR #241, docs/research/LIVE-STATS-DESIGN.md).
--
-- One new function, public.goal_cup_upload_live_snapshot(p_event_id, p_at). The Worker's 5-minute
-- cron calls it once per tick, writes the answer to R2 as live/{event_id}.json, and the public
-- GET /v1/live.json serves that copy. No request reads the database. It reads data already stored:
-- events, runs, entries, heartbeats (the last hour only), notices. No table, column, index or row
-- changes.
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
-- at least min_group (5) runs feed it: a count of runs is null below 5 (0 too); a sum is null unless
-- 5 runs added to it. A harness with fewer than 5 started runs folds into "other", and "other" is
-- left out while it has fewer than 5; a connector id that is not lowercase letters, digits and
-- dashes also folds into "other". Owner question (PR #241): is 5 right?
-- Limits against differencing (comparing numbers to learn about one run): model_calls is floored to
-- a multiple of 100 and data_bytes to a whole MiB, so one run's growth between two snapshots does
-- not show to the call or byte. When "other" is left out, its started count still follows from the
-- total minus the named harnesses (accepted: a count, nothing more); per-harness "running" is then
-- null for every harness, so who is running in the hidden group does not follow. Other differences
-- between successive snapshots (one run starts, ends, goes quiet) stay visible as a change of 1 in a
-- count of 5 or more: that is what a live count is.
--
-- The answer, schema 1 (the Worker checks it key by key before it writes anything):
--   {"schema_version": 1,
--    "event": {"id", "kind", "start_window_opens", "start_window_closes", "run_hours"},
--    "generated_at": p_at, "min_group": 5,
--    "entries": {"signed_up": null, "started", "running", "working", "quiet", "finished", "sealed", "incomplete"},
--    "harnesses": [{"id", "started", "running"}] or null,
--    "work": {"agent_hours", "working_hours": null, "model_calls", "model_calls_last_hour", "restarts", "data_bytes"},
--    "notices": {"silence", "idle_3h", "idle_6h", "disk_full"},
--    "survival": null, "hourly": null, "trace": null, "github": null}
-- or NULL when p_event_id is not an event of the caller's deployment.
--   * A run is over at its end (deadline, or the start of a run that replaced it), or earlier once
--     its current entry has its final heartbeat or seal (the agent stopped). finished: over;
--     sealed and incomplete: over, as the close wrote them. running: not over, last heartbeat under
--     20 minutes old (the silence threshold); working: running, and last model request under 30
--     minutes old; quiet: not over, no heartbeat (or start) for 20 minutes or more. signed_up stays
--     null: the sign-up table is not in this schema yet.
--   * agent_hours: per run, the time from its start to its last heartbeat (at most its end), summed,
--     to one decimal; fed by the runs that have beaten. A quiet gap inside a run counts too: an exact
--     count of beating time would read every heartbeat of the event each tick, which this must not.
--     working_hours is null for the same reason (it needs a stored counter or the verify job).
--   * model_calls: the sum of entries.model_requests (each entry counts from its own start, so a
--     continue adds and never double counts), floored to 100; fed by the runs that reported.
--   * model_calls_last_hour: per entry with a heartbeat in the last hour, the highest count reported
--     in that hour minus the count of its last heartbeat before the hour (0 for a new entry), summed;
--     fed by the runs whose count grew in that hour.
--   * restarts: entries with segment > 0; fed by the runs that restarted. data_bytes: the sum of
--     entries.stored_bytes, floored to a MiB; fed by the runs that stored a chunk.
--   * notices: rows per kind; each fed by the runs that have one.
--   * survival and hourly (later steps of the design) are null for now; the Worker adds "server": null.
--
-- Cost (measured at event scale, 400 runs x 2016 heartbeats, by the scale case in check-sql.sh): the
-- totals come from per-run and per-entry columns (runs, entries: one row each); heartbeats are read
-- only for the last hour, through the existing index (run_id, received_at): one range per run plus
-- one backward probe per entry that beat in the hour. No sort or hash over all heartbeats: about
-- 20 ms and no temp files with work_mem 4 MB (EXPLAIN ANALYZE, BUFFERS on Postgres 16). STABLE; it
-- writes nothing.
--
-- Apply ONCE: staging and production share one database. First run it on a throwaway Postgres
-- loaded with the mirror (server/worker/scripts/check-sql.sh in goal-cup-proxy). Then copy it into
-- the website repo's supabase/migrations/ unchanged, and the lead agent applies it, the owner
-- approves:
--   psql "$DATABASE_URL_REMOTE" -X -1 -v ON_ERROR_STOP=1 -f <this file>; then pnpm db:check.
-- Then deploy the Worker that calls it. A Worker without it never calls it; a Worker with it and a
-- database without it logs live-snapshot-failed each tick and changes nothing else.

-- SET jit = off: the planner's cost estimate for the per-entry probe crosses jit_above_cost, and at
-- event scale JIT compilation took about 1.9 s of a 1.9 s call; without it the call takes about 20 ms.
CREATE FUNCTION public.goal_cup_upload_live_snapshot(p_event_id text, p_at text) RETURNS jsonb
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp SET jit = off
AS $$
  WITH ev AS (
    SELECT e.* FROM events e WHERE e.id = p_event_id AND e.staging = caller_is_staging()
  ),
  -- Every competition run of the event that has started, with its end and whether it stopped early.
  c AS MATERIALIZED (
    SELECT r.id, r.status, r.official_start, r.last_heartbeat_at, r.last_model_request_at,
           least(r.deadline_at, coalesce(r.replaced_at, r.deadline_at)) AS ends_at,
           (r.replaced_at IS NOT NULL AND r.replaced_at < r.deadline_at) AS replaced,
           coalesce((SELECT en.final_seq IS NOT NULL OR en.sealed_at IS NOT NULL
                       FROM entries en WHERE en.run_id = r.id ORDER BY en.segment DESC LIMIT 1), false) AS stopped
      FROM runs r JOIN ev ON ev.id = r.event_id
     WHERE r.mode = 'competition' AND r.staging = caller_is_staging() AND r.status <> 'abandoned'
       AND r.official_start <= p_at::timestamptz
  ),
  -- The main runs (the entries), what each is now, and its harness (segment 0's connector).
  m AS (
    SELECT s.status, s.over,
           NOT s.over AND s.last_heartbeat_at >= s.t - interval '20 minutes' AS running,
           NOT s.over AND s.last_heartbeat_at >= s.t - interval '20 minutes'
             AND s.last_model_request_at >= s.t - interval '30 minutes' AS working,
           NOT s.over AND coalesce(s.last_heartbeat_at, s.official_start) < s.t - interval '20 minutes' AS quiet,
           s.harness
      FROM (SELECT c.*, p_at::timestamptz AS t, (p_at::timestamptz >= c.ends_at OR c.stopped) AS over,
                   (SELECT CASE WHEN en.connector_id ~ '^[a-z0-9][a-z0-9-]{0,63}$' THEN en.connector_id ELSE 'other' END
                      FROM entries en WHERE en.run_id = c.id AND en.segment = 0) AS harness
              FROM c WHERE NOT c.replaced) s
  ),
  counts AS (
    SELECT count(*) AS started,
           count(*) FILTER (WHERE running) AS running,
           count(*) FILTER (WHERE working) AS working,
           count(*) FILTER (WHERE quiet) AS quiet,
           count(*) FILTER (WHERE over) AS finished,
           count(*) FILTER (WHERE over AND status = 'sealed') AS sealed,
           count(*) FILTER (WHERE over AND status = 'incomplete') AS incomplete
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
  -- A hidden "other" hides every harness's running count (see the differencing note above).
  hidden AS (SELECT EXISTS (SELECT 1 FROM harness WHERE id = 'other' AND started < 5) AS other),
  ent AS (
    SELECT sum(en.model_requests) AS model_calls,
           count(DISTINCT en.run_id) FILTER (WHERE en.model_requests IS NOT NULL) AS calling_runs,
           count(*) FILTER (WHERE en.segment > 0) AS restarts,
           count(DISTINCT en.run_id) FILTER (WHERE en.segment > 0) AS restarted_runs,
           sum(en.stored_bytes) AS data_bytes,
           count(DISTINCT en.run_id) FILTER (WHERE en.stored_bytes > 0) AS storing_runs
      FROM entries en JOIN c ON c.id = en.run_id
  ),
  span AS (
    SELECT sum(extract(epoch FROM greatest(least(c.last_heartbeat_at, c.ends_at, p_at::timestamptz) - c.official_start,
                                          interval '0'))) / 3600.0 AS hours,
           count(*) FILTER (WHERE c.last_heartbeat_at IS NOT NULL) AS beaten_runs
      FROM c
  ),
  -- The last hour: each entry's highest count in it (an index range per run) ...
  recent AS (
    SELECT h.run_id, h.entry_id, max(h.model_requests) AS top
      FROM c JOIN heartbeats h ON h.run_id = c.id
     WHERE h.received_at > p_at::timestamptz - interval '1 hour' AND h.received_at <= p_at::timestamptz
       AND h.model_requests IS NOT NULL
     GROUP BY h.run_id, h.entry_id
  ),
  -- ... minus its count just before the hour (one backward index probe per entry).
  grown AS (
    SELECT r.run_id,
           greatest(r.top - coalesce((SELECT h.model_requests FROM heartbeats h
                                       WHERE h.run_id = r.run_id AND h.entry_id = r.entry_id
                                         AND h.received_at <= p_at::timestamptz - interval '1 hour'
                                         AND h.model_requests IS NOT NULL
                                       ORDER BY h.received_at DESC LIMIT 1), 0), 0) AS calls
      FROM recent r
  ),
  hour AS (
    SELECT coalesce(sum(calls), 0) AS calls, count(DISTINCT run_id) FILTER (WHERE calls > 0) AS runs FROM grown
  ),
  nt AS (
    SELECT nn.kind, count(*) AS n, count(DISTINCT nn.run_id) AS runs
      FROM notices nn JOIN c ON c.id = nn.run_id
     WHERE nn.kind IN ('silence', 'idle_3h', 'idle_6h', 'disk_full') AND nn.created_at <= p_at::timestamptz
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
    'generated_at', fmt_time(p_at::timestamptz),
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
               'id', hh.id, 'started', hh.started,
               'running', CASE WHEN NOT hd.other AND hh.running >= 5 THEN hh.running END)
             ORDER BY hh.id = 'other', hh.started DESC, hh.id), '[]'::jsonb)
        FROM harness hh WHERE hh.started >= 5) END,
    'work', jsonb_build_object(
      'agent_hours',           CASE WHEN sp.beaten_runs >= 5 THEN round(sp.hours, 1) END,
      'working_hours',         NULL,
      'model_calls',           CASE WHEN e.calling_runs >= 5 THEN floor(e.model_calls / 100) * 100 END,
      'model_calls_last_hour', CASE WHEN hr.runs >= 5 THEN hr.calls END,
      'restarts',              CASE WHEN e.restarted_runs >= 5 THEN e.restarts END,
      'data_bytes',            CASE WHEN e.storing_runs >= 5 THEN floor(e.data_bytes / 1048576) * 1048576 END),
    'notices', (
      SELECT jsonb_object_agg(kinds.kind, (SELECT CASE WHEN nt.runs >= 5 THEN nt.n END FROM nt WHERE nt.kind = kinds.kind))
        FROM unnest(ARRAY['silence', 'idle_3h', 'idle_6h', 'disk_full']) AS kinds(kind)),
    'survival', NULL,
    'hourly', NULL,
    'trace', NULL,
    'github', NULL)
  FROM ev, counts k, hidden hd, ent e, span sp, hour hr
$$;

-- ---- grants -------------------------------------------------------------------------------------

-- Supabase grants EXECUTE on new functions to PUBLIC, anon and authenticated; revoke all three
-- (docs/SUPABASE.md, "SECURITY DEFINER grants"). Then the Worker roles, like every goal_cup_upload_*.
REVOKE ALL ON FUNCTION public.goal_cup_upload_live_snapshot(text, text) FROM PUBLIC, anon, authenticated;
GRANT EXECUTE ON FUNCTION public.goal_cup_upload_live_snapshot(text, text)
  TO goal_cup_ingest_staging, goal_cup_ingest_production, service_role;
