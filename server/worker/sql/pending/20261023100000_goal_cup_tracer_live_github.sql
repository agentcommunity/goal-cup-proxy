-- Goal Cup Proxy: GitHub totals for GET /v1/live.json (2026-10-23). Step (c) of the live stats
-- design (goal-cup-proxy PR #241, docs/research/LIVE-STATS-DESIGN.md): how many commits, pull
-- requests and lines the entries' public repositories got during the week, as community totals.
-- Apply after 20261022100000_goal_cup_tracer_live_stats.sql, which it builds on.
--
-- One new table and two new functions, and the live snapshot again with its `github` block filled:
--   * goal_cup_tracer.run_github: one row per run, numbers only (commits, PRs opened, lines added
--     and removed), with when they were read, when the run is due again, and whether the last
--     reading was taken. No repository URL, owner, commit message, author name or sha is stored:
--     the URL stays where it is (runs.repo_url).
--   * public.goal_cup_upload_github_due(p_event_id, p_at, p_limit): the cron's work list. The
--     competition runs of the event, of the caller's deployment, main runs only (a replaced run's
--     repository is usually its successor's), not abandoned, started, not final, due at p_at;
--     oldest due first (a run never read is due from its start), at most p_limit (1 to 100).
--     [{"run_id", "repo_url", "official_start", "ends_at", "final_commit"}].
--   * public.goal_cup_upload_record_github(p_run_id, p_github, p_at, p_next_at): store one reading.
--     p_github: {"commits", "prs_opened", "lines_added", "lines_removed": a whole number >= 0, or
--     null for "not read this time" (the stored number stays), "final": true | false}; any other
--     key or value raises. p_next_at: when to read again (null when final). Returns false, and
--     changes nothing, for a run that is not a competition run of the caller's deployment, or whose
--     last reading was already taken: after it the row never changes again. read_at moves only when
--     a number was read.
--   * public.goal_cup_upload_live_snapshot: as in 20261022100000, plus the `github` block. The rest
--     of the function is unchanged, word for word. CREATE OR REPLACE is allowed here because the
--     mirror does not hold the function yet (scripts/check-sql.sh refuses it for a function the
--     mirror has).
--
-- The `github` block: null while fewer than 5 repositories have a reading; else
--   {"as_of": the newest reading, "repos": repositories with a reading, "commits", "prs_opened",
--    "lines_added", "lines_removed"}
-- each total null unless at least 5 repositories feed it; lines floored to a multiple of 100.
-- Counted per repository, not per run: main runs of the snapshot's runs (competition, this event,
-- this deployment, not abandoned), and two runs that name the same repository count it once (the
-- newest reading). Test and practice runs are never read nor counted.
--
-- Privacy: community totals only. The repositories are public by rule, but the snapshot still holds
-- no URL, owner, repository name or per-run number. The cron reads GitHub (src/live-github.ts); this
-- file only stores and sums.
--
-- Cost: the reader writes one row per run about once an hour (at most 20 runs a tick). The snapshot
-- adds one join over the event's runs and run_github rows (one row each); no heartbeat is read.
--
-- Apply ONCE: staging and production share one database. First run it on a throwaway Postgres
-- loaded with the mirror (server/worker/scripts/check-sql.sh in goal-cup-proxy). Then copy it into
-- the website repo's supabase/migrations/ unchanged, and the lead agent applies it, the owner
-- approves:
--   psql "$DATABASE_URL_REMOTE" -X -1 -v ON_ERROR_STOP=1 -f <this file>; then pnpm db:check.
-- Then deploy the Worker that calls it, with GITHUB_STATS = "on". A Worker without it never calls
-- it; a Worker with it and a database without it logs github-stats-failed each tick and changes
-- nothing else.

-- ---- the table ----------------------------------------------------------------------------------

CREATE TABLE goal_cup_tracer.run_github (
  run_id        text PRIMARY KEY REFERENCES goal_cup_tracer.runs (id) ON DELETE CASCADE,
  commits       bigint CHECK (commits >= 0),
  prs_opened    bigint CHECK (prs_opened >= 0),
  lines_added   bigint CHECK (lines_added >= 0),
  lines_removed bigint CHECK (lines_removed >= 0),
  read_at       timestamptz,
  due_at        timestamptz,
  final         boolean NOT NULL DEFAULT false,
  CHECK (final OR due_at IS NOT NULL)
);
ALTER TABLE goal_cup_tracer.run_github ENABLE ROW LEVEL SECURITY;
REVOKE ALL ON goal_cup_tracer.run_github FROM PUBLIC, anon, authenticated;

-- ---- the work list ------------------------------------------------------------------------------

CREATE FUNCTION public.goal_cup_upload_github_due(p_event_id text, p_at text, p_limit integer) RETURNS jsonb
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
  SELECT coalesce(jsonb_agg(jsonb_build_object(
           'run_id', d.id, 'repo_url', d.repo_url, 'official_start', fmt_time(d.official_start),
           'ends_at', fmt_time(d.deadline_at), 'final_commit', d.final_commit) ORDER BY d.due, d.id), '[]'::jsonb)
    FROM (SELECT r.id, r.repo_url, r.official_start, r.deadline_at, r.final_commit,
                 coalesce(g.due_at, r.official_start) AS due
            FROM runs r
            JOIN events e ON e.id = r.event_id AND e.staging = caller_is_staging()
            LEFT JOIN run_github g ON g.run_id = r.id
           WHERE r.event_id = p_event_id AND r.mode = 'competition' AND r.staging = caller_is_staging()
             AND r.status <> 'abandoned' AND (r.replaced_at IS NULL OR r.replaced_at >= r.deadline_at)
             AND r.official_start <= p_at::timestamptz
             AND NOT coalesce(g.final, false)
             AND coalesce(g.due_at, r.official_start) <= p_at::timestamptz
           ORDER BY due, r.id
           LIMIT greatest(1, least(coalesce(p_limit, 20), 100))) d
$$;

-- ---- one reading --------------------------------------------------------------------------------

CREATE FUNCTION public.goal_cup_upload_record_github(p_run_id text, p_github jsonb, p_at text, p_next_at text) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  k text;
  v jsonb;
  n numeric;
  v_final boolean;
  v_read boolean := false;
BEGIN
  IF jsonb_typeof(p_github) IS DISTINCT FROM 'object' THEN
    RAISE EXCEPTION 'record_github: the reading is not an object';
  END IF;
  FOR k IN SELECT jsonb_object_keys(p_github) LOOP
    IF k NOT IN ('commits', 'prs_opened', 'lines_added', 'lines_removed', 'final') THEN
      RAISE EXCEPTION 'record_github: unknown key %', k;
    END IF;
  END LOOP;
  IF jsonb_typeof(p_github -> 'final') IS DISTINCT FROM 'boolean' THEN
    RAISE EXCEPTION 'record_github: final is not true or false';
  END IF;
  v_final := (p_github ->> 'final')::boolean;
  FOREACH k IN ARRAY ARRAY['commits', 'prs_opened', 'lines_added', 'lines_removed'] LOOP
    v := coalesce(p_github -> k, 'null'::jsonb);
    CONTINUE WHEN v = 'null'::jsonb;
    IF jsonb_typeof(v) <> 'number' THEN
      RAISE EXCEPTION 'record_github: % is not a number', k;
    END IF;
    n := v::text::numeric;
    IF n < 0 OR n <> trunc(n) OR n > 9007199254740991 THEN
      RAISE EXCEPTION 'record_github: % is not a whole number of at least 0', k;
    END IF;
    v_read := true;
  END LOOP;
  IF NOT v_final AND p_next_at IS NULL THEN
    RAISE EXCEPTION 'record_github: a reading that is not the last needs p_next_at';
  END IF;
  IF NOT EXISTS (SELECT 1 FROM runs r WHERE r.id = p_run_id AND r.mode = 'competition' AND r.staging = caller_is_staging()) THEN
    RETURN false;
  END IF;
  INSERT INTO run_github AS g (run_id, commits, prs_opened, lines_added, lines_removed, read_at, due_at, final)
  VALUES (p_run_id, (p_github ->> 'commits')::bigint, (p_github ->> 'prs_opened')::bigint,
          (p_github ->> 'lines_added')::bigint, (p_github ->> 'lines_removed')::bigint,
          CASE WHEN v_read THEN p_at::timestamptz END, CASE WHEN NOT v_final THEN p_next_at::timestamptz END, v_final)
  ON CONFLICT (run_id) DO UPDATE
     SET commits       = coalesce(EXCLUDED.commits, g.commits),
         prs_opened    = coalesce(EXCLUDED.prs_opened, g.prs_opened),
         lines_added   = coalesce(EXCLUDED.lines_added, g.lines_added),
         lines_removed = coalesce(EXCLUDED.lines_removed, g.lines_removed),
         read_at       = coalesce(EXCLUDED.read_at, g.read_at),
         due_at        = EXCLUDED.due_at,
         final         = EXCLUDED.final
   WHERE NOT g.final;
  RETURN FOUND;
END
$$;

-- ---- the live snapshot, with its github block ---------------------------------------------------

CREATE OR REPLACE FUNCTION public.goal_cup_upload_live_snapshot(p_event_id text, p_at text) RETURNS jsonb
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
  ),
  -- GitHub: the newest reading of each repository of a main run (two runs naming one repository
  -- count it once), read at or before p_at.
  gh AS (
    SELECT DISTINCT ON (lower(r.repo_url)) g.commits, g.prs_opened, g.lines_added, g.lines_removed, g.read_at
      FROM c JOIN runs r ON r.id = c.id JOIN run_github g ON g.run_id = c.id
     WHERE NOT c.replaced AND g.read_at <= p_at::timestamptz
     ORDER BY lower(r.repo_url), g.read_at DESC
  ),
  ght AS (
    SELECT max(read_at) AS as_of, count(*) AS repos,
           sum(commits) AS commits, count(commits) AS commits_n,
           sum(prs_opened) AS prs, count(prs_opened) AS prs_n,
           sum(lines_added) AS added, count(lines_added) AS added_n,
           sum(lines_removed) AS removed, count(lines_removed) AS removed_n
      FROM gh
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
    'github', CASE WHEN gt.repos >= 5 THEN jsonb_build_object(
      'as_of',         fmt_time(gt.as_of),
      'repos',         gt.repos,
      'commits',       CASE WHEN gt.commits_n >= 5 THEN gt.commits END,
      'prs_opened',    CASE WHEN gt.prs_n >= 5 THEN gt.prs END,
      'lines_added',   CASE WHEN gt.added_n >= 5 THEN floor(gt.added / 100) * 100 END,
      'lines_removed', CASE WHEN gt.removed_n >= 5 THEN floor(gt.removed / 100) * 100 END) END)
  FROM ev, counts k, hidden hd, ent e, span sp, hour hr, ght gt
$$;

-- ---- grants -------------------------------------------------------------------------------------

-- Supabase grants EXECUTE on new functions to PUBLIC, anon and authenticated; revoke all three
-- (docs/SUPABASE.md, "SECURITY DEFINER grants"). Then the Worker roles, like every goal_cup_upload_*.
-- CREATE OR REPLACE keeps the snapshot's grants from 20261022100000; they are stated again here, so
-- the file is safe on its own.
REVOKE ALL ON FUNCTION public.goal_cup_upload_github_due(text, text, integer) FROM PUBLIC, anon, authenticated;
REVOKE ALL ON FUNCTION public.goal_cup_upload_record_github(text, jsonb, text, text) FROM PUBLIC, anon, authenticated;
REVOKE ALL ON FUNCTION public.goal_cup_upload_live_snapshot(text, text) FROM PUBLIC, anon, authenticated;
GRANT EXECUTE ON FUNCTION public.goal_cup_upload_github_due(text, text, integer)
  TO goal_cup_ingest_staging, goal_cup_ingest_production, service_role;
GRANT EXECUTE ON FUNCTION public.goal_cup_upload_record_github(text, jsonb, text, text)
  TO goal_cup_ingest_staging, goal_cup_ingest_production, service_role;
GRANT EXECUTE ON FUNCTION public.goal_cup_upload_live_snapshot(text, text)
  TO goal_cup_ingest_staging, goal_cup_ingest_production, service_role;
