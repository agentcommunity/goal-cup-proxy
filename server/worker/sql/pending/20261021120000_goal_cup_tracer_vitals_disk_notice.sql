-- Goal Cup tracer: heartbeat vitals and the disk_full email (2026-10-21).
--
-- The tracer's heartbeat gains an optional "vitals" object (docs/UPLOAD-API.md, heartbeat):
--   {"disk_free_bytes": u64, "disk_short": bool, "agent_paused": "disk" | null,
--    "rss_bytes": u64, "cpu_ms": u64, "open_files": u64}
-- about the tracer's own processes and the machine's free disk, nothing else. The Worker checks it
-- and passes only these keys, nulls dropped. The tracer pauses the agent itself when the disk is
-- almost full ("agent_paused": "disk") and resumes it once there is space again.
--   * heartbeats.vitals: the vitals that heartbeat sent; null when it sent none.
--   * runs.last_vitals: the latest vitals any heartbeat of the run sent (a heartbeat without
--     vitals keeps the last), like last_client_ip.
--   * runs.disk_paused_at: when the current disk pause began; null while the agent is not paused.
--     A heartbeat whose vitals say agent_paused "disk" starts a pause (if none is running); one
--     whose vitals do not say it (agent_paused null or absent) ends it. A heartbeat without vitals
--     changes neither: an older tracer, or one that could not measure, says nothing about a pause.
--   * runs.disk_notice_at: the start of the latest pause that is owed a disk_full email. A pause
--     that begins sets it, unless the one it holds is less than 6 hours old (at most one disk_full
--     email per run per 6 hours, so a disk that fills and frees in a loop does not flood the member).
--   * notices.kind may now be 'disk_full'. The Worker's cron queues one per disk_notice_at
--     (dedupe key) for a competition or test run before its end, never for practice: the same
--     rules as the other run emails, and a test run's email goes to its operator with "[TEST] ".
--     The cron, not the heartbeat, queues it, like every other notice.
--   * goal_cup_tracer.notice_text: the disk_full email.
--   * public.goal_cup_upload_runs_for_tick: each run also carries "disk_notice_at".
--   * public.goal_cup_upload_heartbeat(…, p_vitals jsonb): a new overload that stores the vitals and
--     moves the run's pause state in the same step. The Worker of this change calls it when a
--     heartbeat carries vitals, and the old 13-argument form otherwise.
--   * The old 13-argument goal_cup_upload_heartbeat (and goal_cup_tracer.note_heartbeat) stay, for
--     the Worker that is live while this is applied; each now calls the new form with no vitals.
--     p_vitals has no DEFAULT on purpose: with one, PostgREST could not choose between the two
--     overloads for a call without it.
--
-- Order: apply this before deploying the Worker that sends p_vitals (staging, then production). The
-- live Worker keeps working on it unchanged. A new Worker on a database without this migration
-- still records every heartbeat that carries no vitals; one that carries vitals is 502 (retried).
--
-- Additive: three new nullable columns on runs and one on heartbeats (no table rewrite), one CHECK
-- constraint widened, three functions replaced with the same signatures (grants kept), two new
-- function overloads with their grants. No row is changed.
--
-- Apply: staging first, then production (lead agent applies, owner approves):
--   psql "$DATABASE_URL_REMOTE" -X -1 -v ON_ERROR_STOP=1 -f <this file>; then pnpm db:check.

ALTER TABLE goal_cup_tracer.heartbeats ADD COLUMN vitals jsonb CHECK (vitals IS NULL OR jsonb_typeof(vitals) = 'object');
ALTER TABLE goal_cup_tracer.runs ADD COLUMN last_vitals jsonb CHECK (last_vitals IS NULL OR jsonb_typeof(last_vitals) = 'object');
ALTER TABLE goal_cup_tracer.runs ADD COLUMN disk_paused_at timestamptz;
ALTER TABLE goal_cup_tracer.runs ADD COLUMN disk_notice_at timestamptz;

ALTER TABLE goal_cup_tracer.notices DROP CONSTRAINT notices_kind_check;
ALTER TABLE goal_cup_tracer.notices ADD CONSTRAINT notices_kind_check
  CHECK (kind IN ('started', 'silence', 'idle_3h', 'idle_6h', 'recording', 'ended', 'disk_full'));

-- The run-level facts a heartbeat adds, now with its vitals and the disk pause they report.
CREATE FUNCTION goal_cup_tracer.note_heartbeat(
  p_run_id text, p_entry_id text, p_at timestamptz, p_recording text, p_model_requests bigint,
  p_client_ip text, p_client_country text, p_vitals jsonb
) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_grew   boolean := false;
  v_paused boolean := p_vitals IS NOT NULL AND p_vitals ->> 'agent_paused' = 'disk';
BEGIN
  IF p_model_requests IS NOT NULL THEN
    UPDATE entries SET model_requests = p_model_requests
     WHERE id = p_entry_id AND (model_requests IS NULL OR p_model_requests > model_requests);
    v_grew := FOUND;
  END IF;
  -- Every right-hand side reads the row as it was before this UPDATE.
  UPDATE runs SET
    last_heartbeat_at     = p_at,
    last_client_ip        = coalesce(p_client_ip, last_client_ip),
    last_client_country   = coalesce(p_client_country, last_client_country),
    flagged_at            = CASE WHEN flagged_at IS NULL AND p_recording IN ('gaps', 'failed') THEN p_at ELSE flagged_at END,
    flag_reason           = CASE WHEN flagged_at IS NULL AND p_recording IN ('gaps', 'failed') THEN p_recording ELSE flag_reason END,
    last_model_request_at = CASE WHEN v_grew THEN p_at ELSE last_model_request_at END,
    last_vitals           = coalesce(p_vitals, last_vitals),
    disk_notice_at        = CASE WHEN v_paused AND disk_paused_at IS NULL
                                      AND (disk_notice_at IS NULL OR disk_notice_at <= p_at - interval '6 hours')
                                 THEN p_at ELSE disk_notice_at END,
    disk_paused_at        = CASE WHEN p_vitals IS NULL THEN disk_paused_at
                                 WHEN v_paused THEN coalesce(disk_paused_at, p_at)
                                 ELSE NULL END
  WHERE id = p_run_id;
END $$;

-- The old form, for anything that still calls it: no vitals.
CREATE OR REPLACE FUNCTION goal_cup_tracer.note_heartbeat(
  p_run_id text, p_entry_id text, p_at timestamptz, p_recording text, p_model_requests bigint,
  p_client_ip text, p_client_country text
) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
BEGIN
  PERFORM note_heartbeat(p_run_id, p_entry_id, p_at, p_recording, p_model_requests, p_client_ip, p_client_country, NULL::jsonb);
END $$;

-- The Worker that is live while this migration is applied still calls the 13-argument form, and
-- the new Worker calls it for a heartbeat without vitals. It goes through the new function with
-- none. Defined before the new function on purpose: the mirror's parity test reads the last
-- definition of each name.
CREATE OR REPLACE FUNCTION public.goal_cup_upload_heartbeat(
  p_member_id text, p_run_id text, p_entry_id text,
  p_seq bigint, p_head text, p_offset_local bigint, p_final boolean, p_repo_head text,
  p_recording text, p_model_requests bigint, p_client_ip text, p_client_country text, p_at text
) RETURNS text
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
BEGIN
  RETURN public.goal_cup_upload_heartbeat(p_member_id, p_run_id, p_entry_id, p_seq, p_head, p_offset_local, p_final,
                                          p_repo_head, p_recording, p_model_requests, p_client_ip, p_client_country,
                                          p_at, NULL::jsonb);
END $$;

CREATE FUNCTION public.goal_cup_upload_heartbeat(
  p_member_id text, p_run_id text, p_entry_id text,
  p_seq bigint, p_head text, p_offset_local bigint, p_final boolean, p_repo_head text,
  p_recording text, p_model_requests bigint, p_client_ip text, p_client_country text, p_at text,
  p_vitals jsonb
) RETURNS text
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_run    runs;
  v_entry  entries;
  v_at     timestamptz := p_at::timestamptz;
  v_stored bigint;
  v_vitals jsonb := CASE WHEN jsonb_typeof(p_vitals) = 'object' THEN p_vitals END;
BEGIN
  SELECT * INTO v_run FROM runs WHERE id = p_run_id AND member_id = p_member_id::uuid AND staging = caller_is_staging() FOR UPDATE;
  IF NOT FOUND THEN RETURN 'not_found'; END IF;
  IF v_run.status IN ('abandoned', 'incomplete') THEN RETURN 'closed'; END IF;
  SELECT * INTO v_entry FROM entries WHERE id = p_entry_id AND run_id = p_run_id FOR UPDATE;
  IF NOT FOUND THEN RETURN 'not_found'; END IF;

  IF NOT p_final THEN
    IF v_entry.sealed_at IS NULL THEN
      INSERT INTO heartbeats (run_id, entry_id, received_at, seq, head, offset_local, final,
                              recording, model_requests, client_ip, client_country, vitals)
      VALUES (p_run_id, p_entry_id, v_at, p_seq, p_head, p_offset_local, false, p_recording, p_model_requests,
              p_client_ip, p_client_country, v_vitals);
      PERFORM note_heartbeat(p_run_id, p_entry_id, v_at, p_recording, p_model_requests, p_client_ip, p_client_country, v_vitals);
    END IF;
    RETURN 'recorded';
  END IF;

  IF v_entry.final_seq IS NOT NULL THEN
    IF v_entry.final_seq = p_seq AND v_entry.final_head = p_head THEN RETURN 'final_existing'; END IF;
    RETURN 'final_conflict';
  END IF;

  v_stored := v_entry.next_seq - 1;
  IF v_stored > p_seq OR (v_stored = p_seq AND v_entry.head <> p_head) THEN
    RETURN 'final_conflict';
  END IF;

  UPDATE entries
     SET final_seq = p_seq, final_head = p_head, final_repo_head = p_repo_head, final_at = v_at
   WHERE id = p_entry_id;
  INSERT INTO heartbeats (run_id, entry_id, received_at, seq, head, offset_local, final, repo_head,
                          recording, model_requests, client_ip, client_country, vitals)
  VALUES (p_run_id, p_entry_id, v_at, p_seq, p_head, p_offset_local, true, p_repo_head, p_recording, p_model_requests,
          p_client_ip, p_client_country, v_vitals);
  PERFORM note_heartbeat(p_run_id, p_entry_id, v_at, p_recording, p_model_requests, p_client_ip, p_client_country, v_vitals);
  RETURN 'final_committed';
END $$;

-- Same signature as before (grants kept); each run also carries disk_notice_at.
CREATE OR REPLACE FUNCTION public.goal_cup_upload_runs_for_tick(p_at text) RETURNS jsonb
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
  SELECT coalesce(jsonb_agg(
           (run_json(r.id) -> 'run') || jsonb_build_object(
             'last_heartbeat_at',     fmt_time(r.last_heartbeat_at),
             'last_model_request_at', fmt_time(r.last_model_request_at),
             'flagged_at',            fmt_time(r.flagged_at),
             'flag_reason',           r.flag_reason,
             'final_commit',          r.final_commit,
             'disk_notice_at',        fmt_time(r.disk_notice_at))
           ORDER BY r.official_start, r.id), '[]'::jsonb)
    FROM runs r
   WHERE r.status <> 'abandoned' AND r.staging = caller_is_staging()
     AND (r.status = 'open'
          OR least(r.deadline_at, coalesce(r.replaced_at, r.deadline_at)) >= p_at::timestamptz - interval '48 hours')
$$;

-- The email for one notice: {subject, text}. The website owns this copy. Same as before, plus disk_full.
CREATE OR REPLACE FUNCTION goal_cup_tracer.notice_text(p_kind text, p_payload jsonb, p_run_id text) RETURNS jsonb
LANGUAGE sql IMMUTABLE SET search_path = pg_catalog, pg_temp
AS $$
  SELECT jsonb_build_object('subject', s.subject, 'text', s.body || E'\n\nRun ' || p_run_id ||
    E'.\nYou get this email because you started a Goal Cup run with the goalcup tracer.\n-- Agent Community, https://agentcommunity.org/goalcup\n')
  FROM (SELECT
    CASE p_kind
      WHEN 'started'   THEN 'Goal Cup: your agent started'
      WHEN 'silence'   THEN 'Goal Cup: your run stopped'
      WHEN 'idle_3h'   THEN 'Goal Cup: no model calls for 3 hours'
      WHEN 'idle_6h'   THEN 'Goal Cup: no model calls for 6 hours'
      WHEN 'recording' THEN 'Goal Cup: the tracer reports a recording problem'
      WHEN 'ended'     THEN 'Goal Cup: your agent finished'
      WHEN 'disk_full' THEN 'Goal Cup: your agent is paused, the disk is almost full'
    END AS subject,
    CASE p_kind
      WHEN 'started' THEN
        'Your agent started its Goal Cup run at ' || coalesce(p_payload->>'official_start', '?') ||
        ' (UTC). The run ends at ' || coalesce(p_payload->>'deadline_at', '?') || E' (UTC).\n\n' ||
        'Repository: ' || coalesce(p_payload->>'repo_url', '?') || E'\n\n' ||
        'To check on it, run `goalcup status` on the machine that started it.'
      WHEN 'silence' THEN
        'We have not heard from your run since ' || coalesce(p_payload->>'last_heartbeat_at', 'it started') ||
        E' (UTC).\n\nYour run stopped. Run `goalcup continue` on the machine that started it.'
      WHEN 'idle_3h' THEN
        'Your agent made no model calls for 3 hours (since ' || coalesce(p_payload->>'since', '?') ||
        E' UTC): is it finished, out of tokens, or offline?'
      WHEN 'idle_6h' THEN
        'Your agent made no model calls for 6 hours (since ' || coalesce(p_payload->>'since', '?') ||
        E' UTC): is it finished, out of tokens, or offline?'
      WHEN 'recording' THEN
        'The tracer reports a recording problem (' || coalesce(p_payload->>'recording', 'unknown') ||
        E'). Run `goalcup status` on the machine that started the run.'
      WHEN 'ended' THEN
        'Your agent finished at ' || coalesce(p_payload->>'ends_at', '?') || E' (UTC).\n\n' ||
        CASE WHEN p_payload->>'commit' IS NULL THEN 'No commit was recorded for ' || coalesce(p_payload->>'repo_url', 'your repository') || '.'
             ELSE 'The commit we recorded is ' || (p_payload->>'commit') || ' in ' || coalesce(p_payload->>'repo_url', 'your repository') || '.' END
      WHEN 'disk_full' THEN
        'The tracer paused your agent at ' || coalesce(p_payload->>'since', '?') ||
        E' (UTC) because the disk of the machine running it is almost full.\n\n' ||
        'Free some disk space on that machine: the agent resumes by itself. ' ||
        'If it still seems stuck after that, type "keep going" in the agent.'
    END AS body) s
$$;

-- Supabase grants EXECUTE on new functions to PUBLIC, anon and authenticated; revoke all three
-- (docs/SUPABASE.md, "SECURITY DEFINER grants"). Then the same Worker roles as the old form.
REVOKE ALL ON FUNCTION goal_cup_tracer.note_heartbeat(text, text, timestamptz, text, bigint, text, text, jsonb) FROM PUBLIC, anon, authenticated;
REVOKE ALL ON FUNCTION public.goal_cup_upload_heartbeat(text, text, text, bigint, text, bigint, boolean, text, text, bigint, text, text, text, jsonb) FROM PUBLIC, anon, authenticated;
GRANT EXECUTE ON FUNCTION public.goal_cup_upload_heartbeat(text, text, text, bigint, text, bigint, boolean, text, text, bigint, text, text, text, jsonb)
  TO goal_cup_ingest_staging, goal_cup_ingest_production, service_role;
