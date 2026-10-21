-- Goal Cup tracer: test runs no longer replace each other (owner, 2026-10-21: "several organiser
-- test runs of one test operator can run in parallel").
--
-- 20261015170000 made a new test run end the operator's open test run ("the same mode replaces the
-- same mode"). Now only competition replaces:
--   * a new competition run ends the member's open competition run (unchanged);
--   * a test run replaces nothing and is never replaced, so one operator may have several open
--     test runs at once; it still never touches a competition run;
--   * practice still never replaces.
--   * goal_cup_device_request answers replaces_open_run = true and replaces_run_started_at only for a
--     competition run request. The website's approval page shows "This replaces your current run"
--     from those keys, so it stops saying it for test runs without a website change (check the
--     page's own wording for test runs in the website repo).
--
-- Additive: two functions replaced (CREATE OR REPLACE keeps their grants). No table, column, row or
-- grant changes. Test runs that 20261015170000 already marked replaced stay replaced.
--
-- Apply: staging first, then production (lead agent applies, owner approves):
--   psql "$DATABASE_URL_REMOTE" -X -1 -v ON_ERROR_STOP=1 -f <this file>; then pnpm db:check.

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
