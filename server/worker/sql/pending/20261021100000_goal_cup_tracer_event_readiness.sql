-- Goal Cup tracer: event readiness (2026-10-21). One migration for five changes, applied once.
--
-- 1. Test runs no longer replace each other (owner, 21 Oct 2026: "several organiser test runs of one
--    test operator can run in parallel"). 20261015170000 made a new test run end the operator's
--    open test run. Now only competition replaces:
--      * a new competition run ends the member's open competition run (unchanged);
--      * a test run replaces nothing and is never replaced, so one operator may have several open
--        test runs at once; it still never touches a competition run;
--      * practice still never replaces;
--      * goal_cup_device_request answers replaces_open_run = true and replaces_run_started_at only
--        for a competition run request. The website's approval page shows "This replaces your
--        current run" from those keys, so it stops saying it for test runs without a website change
--        (check the page's own wording for test runs in the website repo).
--    Test runs that 20261015170000 already marked replaced stay replaced.
--
-- 2. A per-run server size limit (owner approved; event-readiness plan WP-D). The Worker refuses a
--    chunk that would take a run past RUN_MAX_COMPRESSED_BYTES (a Worker variable, 64 GiB) with 413
--    quota_exceeded, before it stores anything in R2. To know what a run holds, the database counts it:
--      * entries.stored_bytes: the compressed bytes of the entry's committed chunks (each chunk's
--        request Content-Length, which is what R2 stores); entries.chunk_count: how many chunks.
--        Both start at 0. Chunks committed before this migration are not counted (no compressed
--        size was kept for them); the counters cover every chunk from here on.
--      * chunks.stored_bytes: the compressed bytes of that chunk; null for chunks from before.
--      * goal_cup_tracer.run_json adds "stored_bytes" and "chunk_count" to the run object: the sums
--        over the run's entries. Every function that answers a run (position, open, continue,
--        runs_for_tick) carries them; a Worker that does not read them ignores them.
--      * public.goal_cup_upload_commit_chunk(…, p_stored_bytes): a new overload. In the same atomic
--        step as before it records the chunk and adds p_stored_bytes and one chunk to the entry.
--    Operator notices at 50% and 100% of the limit are Worker log lines (quota-half, quota-full,
--    quota-exceeded), not rows in goal_cup_tracer.notices: that outbox mails the run's member.
--
-- 3. Heartbeat vitals and the disk_full email. The heartbeat gains an optional "vitals" object
--    (docs/UPLOAD-API.md, heartbeat):
--      {"disk_free_bytes": u64, "disk_short": bool, "agent_paused": "disk" | null,
--       "rss_bytes": u64, "cpu_ms": u64, "open_files": u64}
--    about the tracer's own processes and the machine's free disk, nothing else. The Worker checks
--    it and passes only these keys, nulls dropped. The tracer pauses the agent itself when the disk
--    is almost full ("agent_paused": "disk") and resumes it once there is space again.
--      * heartbeats.vitals: the vitals that heartbeat sent; null when it sent none.
--      * runs.last_vitals: the latest vitals any heartbeat of the run sent (a heartbeat without
--        vitals keeps the last), like last_client_ip.
--      * runs.disk_paused_at: when the current disk pause began; null while the agent is not
--        paused. A heartbeat whose vitals say agent_paused "disk" starts a pause (if none is
--        running); one whose vitals do not say it ends it. A heartbeat without vitals changes
--        neither: an older tracer, or one that could not measure, says nothing about a pause.
--      * runs.disk_notice_at: the start of the latest pause that is owed a disk_full email. A pause
--        that begins sets it, unless the one it holds is less than 6 hours old (at most one
--        disk_full email per run per 6 hours).
--      * notices.kind may now be 'disk_full'. The Worker's cron queues one per disk_notice_at
--        (dedupe key) for a competition or test run before its end, never for practice: the same
--        rules as the other run emails, and a test run's email goes to its operator with "[TEST] ".
--      * public.goal_cup_upload_runs_for_tick: each run also carries "disk_notice_at".
--      * public.goal_cup_upload_heartbeat(…, p_vitals jsonb): a new overload that stores the vitals
--        and moves the run's pause state in the same step. The Worker calls it when a heartbeat
--        carries vitals, and the old 13-argument form otherwise.
--
-- 4. The judged commit is the last push before the deadline (owner, 21 Oct 2026): the last push to
--    the repository's default branch that GitHub itself timestamped at or before the run's ends_at.
--    The Worker's cron reads it from GET /repos/{owner}/{repo}/activity (the `timestamp` GitHub gave
--    each push); commit dates are ignored. runs.final_commit keeps its meaning (that commit, or
--    'none'); these columns say how it was found (all null for rows recorded before this):
--      * runs.final_pushed_at: when GitHub received the judged push; null when not known.
--      * runs.final_branch: the default branch that was read.
--      * runs.final_late_push: GitHub had a push to that branch after ends_at when it was asked.
--      * runs.final_force_push: GitHub had a force push to that branch between official_start and
--        ends_at.
--      * runs.final_head_fallback: the commit is the head GitHub had when asked (commits/HEAD),
--        because the activity API was unavailable or listed no update of the branch.
--      * public.goal_cup_upload_record_final_commit(…, p_pushed_at, p_branch, p_late_push,
--        p_force_push, p_head_fallback): a new overload that stores the commit and these, once
--        (the first stays, as before).
--
-- 5. New words for three run emails (docs/research/NOTIFICATIONS.md), in goal_cup_tracer.notice_text:
--      * silence: no longer says "Your run stopped" (a wifi drop, a sleeping computer or our own
--        outage look the same from here). It says when we last heard from the run, that recording
--        goes on while the computer is offline or asleep, to run `goalcup status` and do what its
--        Next line says, and never to run `goalcup start`. The subject changes to match. The Worker
--        mails it only when the server itself was well for the whole silence; that rule is in the
--        Worker.
--      * ended: when the payload says part of the run has not arrived ("uploaded": false), it says
--        to run `goalcup upload` before "closes_at" (the end + 24 hours); "uploaded": true says
--        nothing more is needed; a payload without the key (an older Worker) gets neither line.
--      * disk_full: free space outside the run's folder, and never anything the agent made.
--    The other kinds keep their words.
--
-- Old signatures stay. The Worker that is live while this is applied keeps calling the 3-argument
-- goal_cup_upload_commit_chunk (now: no size, 0 bytes and one chunk), the 13-argument
-- goal_cup_upload_heartbeat and goal_cup_tracer.note_heartbeat (now: no vitals) and the 4-argument
-- goal_cup_upload_record_final_commit (now: no detail); each calls the new form. A later migration
-- can drop them once every Worker sends the new arguments. The new parameters have no DEFAULT on
-- purpose: with one, PostgREST could not choose between the two overloads for a call without them.
-- Each old form is defined before its new one on purpose: the mirror's parity test reads the last
-- definition of each name.
--
-- Additive: new columns (constant defaults or null, no table rewrite), the notices kind CHECK
-- widened, functions replaced with the same signatures (grants kept), four new function overloads
-- with their grants. No row is changed.
--
-- Apply ONCE: staging and production share one database, so there is no "staging first" for SQL.
-- First run it on a throwaway Postgres loaded with the mirror (server/worker/scripts/check-sql.sh in
-- goal-cup-tracer). Then the lead agent applies it, the owner approves:
--   psql "$DATABASE_URL_REMOTE" -X -1 -v ON_ERROR_STOP=1 -f <this file>; then pnpm db:check.
-- Then deploy and test the staging Worker, then deploy production.

-- ---- columns and the notices kind CHECK ---------------------------------------------------------

ALTER TABLE goal_cup_tracer.entries ADD COLUMN stored_bytes bigint NOT NULL DEFAULT 0 CHECK (stored_bytes >= 0);
ALTER TABLE goal_cup_tracer.entries ADD COLUMN chunk_count  bigint NOT NULL DEFAULT 0 CHECK (chunk_count >= 0);
ALTER TABLE goal_cup_tracer.chunks  ADD COLUMN stored_bytes bigint CHECK (stored_bytes >= 0);
ALTER TABLE goal_cup_tracer.heartbeats ADD COLUMN vitals jsonb CHECK (vitals IS NULL OR jsonb_typeof(vitals) = 'object');
ALTER TABLE goal_cup_tracer.runs ADD COLUMN last_vitals jsonb CHECK (last_vitals IS NULL OR jsonb_typeof(last_vitals) = 'object');
ALTER TABLE goal_cup_tracer.runs ADD COLUMN disk_paused_at timestamptz;
ALTER TABLE goal_cup_tracer.runs ADD COLUMN disk_notice_at timestamptz;
ALTER TABLE goal_cup_tracer.runs ADD COLUMN final_pushed_at     timestamptz;
ALTER TABLE goal_cup_tracer.runs ADD COLUMN final_branch        text CHECK (final_branch IS NULL OR length(final_branch) BETWEEN 1 AND 255);
ALTER TABLE goal_cup_tracer.runs ADD COLUMN final_late_push     boolean;
ALTER TABLE goal_cup_tracer.runs ADD COLUMN final_force_push    boolean;
ALTER TABLE goal_cup_tracer.runs ADD COLUMN final_head_fallback boolean;

ALTER TABLE goal_cup_tracer.notices DROP CONSTRAINT notices_kind_check;
ALTER TABLE goal_cup_tracer.notices ADD CONSTRAINT notices_kind_check
  CHECK (kind IN ('started', 'silence', 'idle_3h', 'idle_6h', 'recording', 'ended', 'disk_full'));

-- ---- 1. only competition replaces ---------------------------------------------------------------

CREATE OR REPLACE FUNCTION public.goal_cup_upload_open_run(
  p_member_id text, p_event_id text, p_mode text, p_run_id text, p_entry_id text,
  p_official_start text, p_deadline_at text,
  p_connector_id text, p_manifest_sha256 text, p_connector_dir_sha256 text,
  p_workspace text, p_session_id text, p_harness_version text, p_tracer_version text, p_repo_url text,
  p_machine jsonb, p_client_ip text, p_client_country text,
  p_credential_hash text, p_credential_expires_at text
) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_member  uuid := p_member_id::uuid;
  v_staging boolean := caller_is_staging();
  v_event   events;
  v_start   timestamptz := p_official_start::timestamptz;
  v_deadline timestamptz := p_deadline_at::timestamptz;
  v_cred_expires timestamptz := p_credential_expires_at::timestamptz;
  v_bound   text;
  v_try     integer;
BEGIN
  SELECT * INTO v_event FROM events WHERE id = p_event_id AND staging = v_staging;
  -- The event must be of the run's kind: practice in a practice event, a test run in a test
  -- event, a competition run in the competition or the dry run.
  IF NOT FOUND
     OR (p_mode = 'practice' AND v_event.mode <> 'practice')
     OR (p_mode = 'test' AND v_event.mode <> 'test')
     OR (p_mode = 'competition' AND v_event.mode NOT IN ('competition', 'dry_run')) THEN
    RETURN jsonb_build_object('result', 'forbidden', 'message', 'This event is not open for runs.');
  END IF;
  -- Test runs: allowlisted organizer accounts only; their length is the event's run_hours.
  IF p_mode = 'test' THEN
    IF NOT is_test_operator(v_member) THEN
      RETURN jsonb_build_object('result', 'forbidden', 'message', 'Test runs are for Goal Cup organizers only.');
    END IF;
    v_deadline := v_start + make_interval(hours => v_event.run_hours);
    v_cred_expires := v_deadline + interval '24 hours';
  END IF;
  IF NOT has_current_terms(v_member) THEN
    RETURN jsonb_build_object('result', 'forbidden', 'message', 'Accept the Goal Cup terms on the website first.');
  END IF;
  IF p_mode = 'competition' THEN
    IF v_start < v_event.start_window_opens THEN
      RETURN jsonb_build_object('result', 'forbidden', 'message', 'The start window has not opened yet.');
    END IF;
    IF v_start > v_event.start_window_closes THEN
      RETURN jsonb_build_object('result', 'forbidden',
                                'message', 'after the start window you can only `goalcup continue` your run');
    END IF;
  END IF;

  -- The credential opens runs for its own member and mode only: a credential approved for one
  -- member can never open (or replace) a run of another.
  SELECT run_id INTO v_bound FROM run_credentials
   WHERE credential_hash = p_credential_hash AND staging = v_staging
     AND member_id = v_member AND mode = p_mode FOR UPDATE;
  IF NOT FOUND THEN
    RETURN jsonb_build_object('result', 'conflict');
  END IF;
  IF v_bound IS NOT NULL THEN
    IF EXISTS (SELECT 1 FROM runs r JOIN entries e ON e.run_id = r.id AND e.segment = 0
                WHERE r.id = v_bound AND r.member_id = v_member AND r.staging = v_staging AND e.session_id = p_session_id) THEN
      RETURN jsonb_build_object('result', 'existing')
             || run_json(v_bound, (SELECT e.id FROM entries e WHERE e.run_id = v_bound AND e.segment = 0));
    END IF;
    RETURN jsonb_build_object('result', 'conflict');
  END IF;

  FOR v_try IN 1..2 LOOP
    -- Only competition replaces: a competition run ends the member's open competition run. A test
    -- run never replaces (an operator's test runs run side by side) and never touches a competition
    -- run, and practice never replaces.
    IF p_mode = 'competition' THEN
      UPDATE runs SET replaced_at = greatest(v_start, official_start)
       WHERE member_id = v_member AND mode = p_mode AND status = 'open' AND replaced_at IS NULL
         AND staging = v_staging;
    END IF;
    BEGIN
      INSERT INTO runs (id, event_id, member_id, mode, official_start, deadline_at, repo_url,
                        machine, client_ip, client_country, staging)
      VALUES (p_run_id, p_event_id, v_member, p_mode, v_start, v_deadline, p_repo_url,
              nullif(p_machine, 'null'::jsonb), p_client_ip, p_client_country, v_staging);
      EXIT;
    EXCEPTION WHEN unique_violation THEN
      IF v_try = 2 THEN RAISE; END IF;
    END;
  END LOOP;

  INSERT INTO entries (id, run_id, segment, connector_id, manifest_sha256, connector_dir_sha256,
                       workspace, session_id, harness_version, tracer_version)
  VALUES (p_entry_id, p_run_id, 0, p_connector_id, p_manifest_sha256, p_connector_dir_sha256,
          p_workspace, p_session_id, p_harness_version, p_tracer_version);
  UPDATE run_credentials
     SET run_id = p_run_id, expires_at = v_cred_expires
   WHERE credential_hash = p_credential_hash;
  RETURN jsonb_build_object('result', 'created') || run_json(p_run_id);
END $$;

CREATE OR REPLACE FUNCTION public.goal_cup_device_request(p_actor_id uuid, p_user_code text) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_dev      device_requests;
  v_replaces timestamptz;
BEGIN
  SELECT * INTO v_dev FROM device_requests
   WHERE user_hash = encode(sha256(convert_to(upper(replace(coalesce(p_user_code, ''), '-', '')), 'UTF8')), 'hex');
  IF NOT FOUND THEN RETURN jsonb_build_object('result', 'not_found'); END IF;
  -- Only a competition run request replaces anything (20261021100000).
  IF v_dev.purpose = 'run' AND v_dev.mode = 'competition' THEN
    SELECT max(r.official_start) INTO v_replaces FROM runs r
     WHERE r.member_id = p_actor_id AND r.mode = v_dev.mode AND r.status = 'open'
       AND r.replaced_at IS NULL AND r.staging = v_dev.staging AND now() < r.deadline_at;
  END IF;
  RETURN jsonb_build_object(
    'result', 'ok',
    'status', CASE WHEN v_dev.status = 'pending' AND now() >= v_dev.expires_at THEN 'expired' ELSE v_dev.status END,
    'purpose', v_dev.purpose,
    'mode', v_dev.mode,
    'connector_id', v_dev.connector_id,
    'tracer_version', v_dev.tracer_version,
    'expires_at', fmt_time(v_dev.expires_at),
    'replaces_open_run', v_replaces IS NOT NULL,
    'replaces_run_started_at', fmt_time(v_replaces),
    'terms_ok', has_current_terms(p_actor_id),
    'terms_version', current_terms_version(),
    'decided_by_you', v_dev.member_id IS NOT DISTINCT FROM p_actor_id AND v_dev.status <> 'pending');
END $$;

-- ---- 2. the per-run size limit ------------------------------------------------------------------

CREATE OR REPLACE FUNCTION goal_cup_tracer.run_json(p_run_id text, p_entry_id text DEFAULT NULL) RETURNS jsonb
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
  SELECT jsonb_build_object(
    'run', jsonb_build_object(
      'run_id',            r.id,
      'event_id',          r.event_id,
      'member_id',         r.member_id::text,
      'mode',              r.mode,
      'status',            r.status,
      'official_start',    fmt_time(r.official_start),
      'deadline_at',       fmt_time(r.deadline_at),
      'replaced_at',       fmt_time(r.replaced_at),
      'repo_url',          r.repo_url,
      'first_entry_id',    first.id,
      'current_entry_id',  cur.id,
      'current_sealed_at', fmt_time(cur.sealed_at),
      'stored_bytes',      tot.stored_bytes,
      'chunk_count',       tot.chunk_count),
    'entry', (
      SELECT jsonb_build_object(
        'entry_id',             e.id,
        'run_id',               e.run_id,
        'segment',              e.segment,
        'connector_id',         e.connector_id,
        'manifest_sha256',      e.manifest_sha256,
        'connector_dir_sha256', e.connector_dir_sha256,
        'workspace',            e.workspace,
        'session_id',           e.session_id,
        'offset',               e.upload_offset,
        'next_seq',             e.next_seq,
        'head',                 e.head,
        'last_ack',             e.last_ack,
        'final',                CASE WHEN e.final_seq IS NULL THEN NULL
                                     ELSE jsonb_build_object('seq', e.final_seq, 'head', e.final_head) END,
        'receipt',              e.receipt,
        'seal_sha256',          e.seal_sha256,
        'baseline_sha256',      e.baseline_sha256,
        'sealed_at',            fmt_time(e.sealed_at))
      FROM entries e
      WHERE e.run_id = r.id AND e.id = coalesce(p_entry_id, cur.id))
  )
  FROM runs r
  CROSS JOIN LATERAL (SELECT e.id, e.sealed_at FROM entries e WHERE e.run_id = r.id ORDER BY e.segment DESC LIMIT 1) cur
  CROSS JOIN LATERAL (SELECT e.id FROM entries e WHERE e.run_id = r.id AND e.segment = 0) first
  CROSS JOIN LATERAL (SELECT coalesce(sum(e.stored_bytes), 0)::bigint AS stored_bytes,
                             coalesce(sum(e.chunk_count), 0)::bigint AS chunk_count
                        FROM entries e WHERE e.run_id = r.id) tot
  WHERE r.id = p_run_id
$$;

-- The Worker that is live while this migration is applied still calls the three-argument form.
-- It now goes through the new function (no size known: 0 bytes, one chunk). Defined before the
-- new function on purpose: the mirror's parity test reads the last definition of each name.
CREATE OR REPLACE FUNCTION public.goal_cup_upload_commit_chunk(p_member_id text, p_run_id text, p_chunk jsonb) RETURNS text
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
BEGIN
  RETURN public.goal_cup_upload_commit_chunk(p_member_id, p_run_id, p_chunk, NULL::bigint);
END $$;

CREATE FUNCTION public.goal_cup_upload_commit_chunk(p_member_id text, p_run_id text, p_chunk jsonb, p_stored_bytes bigint) RETURNS text
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_run   runs;
  v_entry entries;
  v_off   bigint := (p_chunk ->> 'offset')::bigint;
  v_len   bigint := (p_chunk ->> 'length')::bigint;
  v_bytes bigint := greatest(coalesce(p_stored_bytes, 0), 0);
BEGIN
  SELECT * INTO v_run FROM runs WHERE id = p_run_id AND member_id = p_member_id::uuid AND staging = caller_is_staging() FOR UPDATE;
  IF NOT FOUND THEN RETURN 'not_found'; END IF;
  SELECT * INTO v_entry FROM entries WHERE id = p_chunk ->> 'entry_id' AND run_id = p_run_id FOR UPDATE;
  IF NOT FOUND THEN RETURN 'not_found'; END IF;
  IF v_run.status IN ('abandoned', 'incomplete') OR v_entry.sealed_at IS NOT NULL THEN RETURN 'closed'; END IF;

  IF v_entry.final_seq IS NOT NULL
     AND ((p_chunk ->> 'last_seq')::bigint > v_entry.final_seq
          OR ((p_chunk ->> 'last_seq')::bigint = v_entry.final_seq AND p_chunk ->> 'head' <> v_entry.final_head)) THEN
    RETURN 'beyond_final';
  END IF;

  IF v_entry.upload_offset <> v_off
     OR v_entry.head <> p_chunk ->> 'prev'
     OR v_entry.next_seq <> (p_chunk ->> 'first_seq')::bigint THEN
    RETURN 'conflict';
  END IF;

  INSERT INTO chunks (entry_id, "offset", length, sha256, first_seq, last_seq, prev, head, r2_key, received_at, ack, stored_bytes)
  VALUES (v_entry.id, v_off, v_len, p_chunk ->> 'sha256', (p_chunk ->> 'first_seq')::bigint,
          (p_chunk ->> 'last_seq')::bigint, p_chunk ->> 'prev', p_chunk ->> 'head', p_chunk ->> 'r2_key',
          (p_chunk ->> 'received_at')::timestamptz, p_chunk -> 'ack', p_stored_bytes);

  UPDATE entries
     SET upload_offset = v_off + v_len,
         next_seq      = (p_chunk ->> 'last_seq')::bigint + 1,
         head          = p_chunk ->> 'head',
         last_ack      = p_chunk -> 'ack',
         last_chunk_at = (p_chunk ->> 'received_at')::timestamptz,
         stored_bytes  = stored_bytes + v_bytes,
         chunk_count   = chunk_count + 1
   WHERE id = v_entry.id;
  RETURN 'committed';
END $$;

-- ---- 3. heartbeat vitals and the disk pause -----------------------------------------------------

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

-- ---- 4. the judged commit is the last push before the deadline ----------------------------------

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

-- ---- 5. the run emails --------------------------------------------------------------------------

-- The email for one notice: {subject, text}. The website owns this copy.
-- The product is named Goal Cup Proxy (goalcup) in the footer and the recording and disk_full emails.
CREATE OR REPLACE FUNCTION goal_cup_tracer.notice_text(p_kind text, p_payload jsonb, p_run_id text) RETURNS jsonb
LANGUAGE sql IMMUTABLE SET search_path = pg_catalog, pg_temp
AS $$
  SELECT jsonb_build_object('subject', s.subject, 'text', s.body || E'\n\nRun ' || p_run_id ||
    E'.\nYou get this email because you started a Goal Cup run with Goal Cup Proxy (goalcup).\n-- Agent Community, https://agentcommunity.org/goalcup\n')
  FROM (SELECT
    CASE p_kind
      WHEN 'started'   THEN 'Goal Cup: your agent started'
      WHEN 'silence'   THEN 'Goal Cup: we have not heard from your run'
      WHEN 'idle_3h'   THEN 'Goal Cup: no model calls for 3 hours'
      WHEN 'idle_6h'   THEN 'Goal Cup: no model calls for 6 hours'
      WHEN 'recording' THEN 'Goal Cup: Goal Cup Proxy reports a recording problem'
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
        'We have not heard from your run since ' ||
        CASE WHEN p_payload->>'last_heartbeat_at' IS NULL THEN 'it started'
             ELSE (p_payload->>'last_heartbeat_at') || ' (UTC)' END || E'.\n\n' ||
        E'If the computer is offline or asleep, recording goes on and catches up later.\n\n' ||
        E'At the computer that runs it, run `goalcup status` and do what its Next line says.\n\n' ||
        'Do not run `goalcup start`: a new start replaces this run.'
      WHEN 'idle_3h' THEN
        'Your agent made no model calls for 3 hours (since ' || coalesce(p_payload->>'since', '?') ||
        E' UTC): is it finished, out of tokens, or offline?'
      WHEN 'idle_6h' THEN
        'Your agent made no model calls for 6 hours (since ' || coalesce(p_payload->>'since', '?') ||
        E' UTC): is it finished, out of tokens, or offline?'
      WHEN 'recording' THEN
        'Goal Cup Proxy reports a recording problem (' || coalesce(p_payload->>'recording', 'unknown') ||
        E'). Run `goalcup status` on the machine that started the run.'
      WHEN 'ended' THEN
        'Your agent finished at ' || coalesce(p_payload->>'ends_at', '?') || E' (UTC).\n\n' ||
        CASE WHEN p_payload->>'commit' IS NULL THEN 'No commit was recorded for ' || coalesce(p_payload->>'repo_url', 'your repository') || '.'
             ELSE 'The commit we recorded is ' || (p_payload->>'commit') || ' in ' || coalesce(p_payload->>'repo_url', 'your repository') || '.' END ||
        CASE p_payload->>'uploaded'
          WHEN 'false' THEN
            E'\n\nPart of your run has not arrived yet. On the computer that ran it, run `goalcup upload` before ' ||
            coalesce(p_payload->>'closes_at', '?') || ' (UTC). It sends only what is missing. After that time the server takes no more uploads.'
          WHEN 'true' THEN E'\n\nYour run is sealed on the server. Nothing more to do.'
          ELSE ''
        END
      WHEN 'disk_full' THEN
        'Goal Cup Proxy paused your agent at ' || coalesce(p_payload->>'since', '?') ||
        E' (UTC) because the disk of the computer running it is almost full.\n\n' ||
        'Free space outside your run''s folder. Do not delete anything the agent made. ' ||
        'The agent resumes by itself. If it still seems stuck after that, type "keep going" in the agent.'
    END AS body) s
$$;

-- ---- grants -------------------------------------------------------------------------------------

-- Supabase grants EXECUTE on new functions to PUBLIC, anon and authenticated; revoke all three
-- (docs/SUPABASE.md, "SECURITY DEFINER grants"). Then the same Worker roles as the old form.
REVOKE ALL ON FUNCTION public.goal_cup_upload_commit_chunk(text, text, jsonb, bigint) FROM PUBLIC, anon, authenticated;
GRANT EXECUTE ON FUNCTION public.goal_cup_upload_commit_chunk(text, text, jsonb, bigint)
  TO goal_cup_ingest_staging, goal_cup_ingest_production, service_role;
REVOKE ALL ON FUNCTION goal_cup_tracer.note_heartbeat(text, text, timestamptz, text, bigint, text, text, jsonb) FROM PUBLIC, anon, authenticated;
REVOKE ALL ON FUNCTION public.goal_cup_upload_heartbeat(text, text, text, bigint, text, bigint, boolean, text, text, bigint, text, text, text, jsonb) FROM PUBLIC, anon, authenticated;
GRANT EXECUTE ON FUNCTION public.goal_cup_upload_heartbeat(text, text, text, bigint, text, bigint, boolean, text, text, bigint, text, text, text, jsonb)
  TO goal_cup_ingest_staging, goal_cup_ingest_production, service_role;
REVOKE ALL ON FUNCTION public.goal_cup_upload_record_final_commit(text, text, text, text, text, text, boolean, boolean, boolean) FROM PUBLIC, anon, authenticated;
GRANT EXECUTE ON FUNCTION public.goal_cup_upload_record_final_commit(text, text, text, text, text, text, boolean, boolean, boolean)
  TO goal_cup_ingest_staging, goal_cup_ingest_production, service_role;
