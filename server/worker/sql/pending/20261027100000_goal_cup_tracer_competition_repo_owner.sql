-- Goal Cup Proxy: one competition repository, one member (2026-10-26). The name is dated 27 Oct so it
-- sorts after the website's latest migration.
--
-- A member could open a competition run naming another member's new, empty, public repository, and
-- the deadline job would then judge that member's pushed commit as theirs (0.7.0 security review,
-- I2). Now goal_cup_upload_open_run refuses a competition open whose repo_url is already the
-- repository of a competition run of another member in the same event:
--   * the URLs compare by goal_cup_tracer.repo_key: lower case, with a trailing ".git" removed. The
--     Worker accepts only https://github.com/<owner>/<repo> with no trailing "/", so nothing else
--     can differ;
--   * every run of the other member counts, whatever its state (open, replaced, sealed, incomplete,
--     abandoned);
--   * the same member may name the repository again (a new start after a replaced run), and a
--     retried open (a lost reply) is answered "existing" before the check, as before;
--   * practice and test runs are not checked and do not count;
--   * the answer is "forbidden" with a message, which every deployed Worker already turns into 403
--     with that message. No Worker change is needed for it to work.
-- Two opens of one repository by two members take turns on a transaction-scoped advisory lock keyed
-- by the event and the repo_key. Lock order: credential (FOR UPDATE), sign-up (FOR SHARE), then this
-- lock; erasure and sign-up never take it, so the order stays acyclic.
--
-- Builds on 20261021120200_goal_cup_signup_gate.sql (applied to production on 2026-10-23): the body
-- below is that one, unchanged but for the new block after the sign-up gate. The guard first makes
-- the apply fail, and its transaction roll back, unless 20261021120200 is in place, so this can
-- never silently remove the sign-up gate.
--
-- Staging and production share this database and this function. The check is scoped by event and by
-- the runs' staging flag, so a staging run never blocks a production run, or the reverse.
--
-- Additive: one new private function and a new body for one function with the same signature
-- (grants stay). No table or row changes.
--
-- Apply: as server/worker/sql/pending/README.md in goal-cup-proxy says (the read-only check first,
-- then one transaction after SET lock_timeout = '5s'); then pnpm db:check.

-- ---- guard: the sign-up gate first ----------------------------------------------------------------
-- The body below starts from 20261021120200. On a database without it, it would bring back an
-- open_run with no sign-up gate. seed_staging_signup is created only by 20261021120200, and the live
-- open_run must already check requires_signup.
DO $$
BEGIN
  IF to_regprocedure('goal_cup_tracer.seed_staging_signup(uuid,text)') IS NULL
     OR position('requires_signup' in pg_get_functiondef('public.goal_cup_upload_open_run'::regproc)) = 0 THEN
    RAISE EXCEPTION '20261027100000 needs 20261021120200_goal_cup_signup_gate.sql: apply 20261021120200 first';
  END IF;
END $$;

-- The form of a repository URL that two runs are compared by.
CREATE FUNCTION goal_cup_tracer.repo_key(p_repo_url text) RETURNS text
LANGUAGE sql IMMUTABLE SET search_path = pg_catalog, pg_temp
AS $$ SELECT regexp_replace(lower(p_repo_url), '\.git$', '') $$;

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

  -- The sign-up gate (20261021120200): an official start in an event that requires a sign-up needs the operator's
  -- active sign-up. FOR SHARE holds the row until this transaction ends, so a withdrawal (FOR UPDATE) cannot commit
  -- in between; a start that waited on a withdrawal re-reads the row, finds it withdrawn and is refused. Nothing has
  -- been written yet, so the refusal leaves no trace.
  IF p_mode = 'competition' AND v_event.mode = 'competition' AND v_event.requires_signup THEN
    PERFORM 1 FROM signups
     WHERE event_id = p_event_id AND member_id = v_member AND withdrawn_at IS NULL
       FOR SHARE;
    IF NOT FOUND THEN
      RETURN jsonb_build_object('result', 'forbidden',
                                'message', 'Sign up for Goal Cup at agentcommunity.org/goalcup before your official start.');
    END IF;
  END IF;

  -- One repository, one member (20261027100000): a competition run may not name a repository that a
  -- competition run of another member in this event already names, in any state. The same member
  -- may name it again. After the sign-up gate, so a member who has not signed up gets that message
  -- and only an eligible open takes the lock. The lock makes two members' opens of one repository
  -- take turns, so both cannot pass the check at once; it is held to the end of this transaction.
  IF p_mode = 'competition' THEN
    PERFORM pg_advisory_xact_lock(hashtextextended('goal_cup_tracer.repo ' || p_event_id || ' ' || repo_key(p_repo_url), 0));
    IF EXISTS (SELECT 1 FROM runs r
                WHERE r.event_id = p_event_id AND r.mode = 'competition' AND r.staging = v_staging
                  AND r.member_id <> v_member AND repo_key(r.repo_url) = repo_key(p_repo_url)) THEN
      RETURN jsonb_build_object('result', 'forbidden', 'message',
        'Another member''s run in this event already uses this repository. Create a new empty public repository and start again.');
    END IF;
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

-- ---- grants -------------------------------------------------------------------------------------

-- Supabase grants EXECUTE on new functions to PUBLIC, anon and authenticated; revoke all three
-- (docs/SUPABASE.md, "SECURITY DEFINER grants"). Only goal_cup_upload_open_run calls it.
REVOKE ALL ON FUNCTION goal_cup_tracer.repo_key(text) FROM PUBLIC, anon, authenticated;
