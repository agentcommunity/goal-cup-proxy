-- COPY, for reference. The authority is the website repo (agentCommunity_PAGE), supabase/migrations/.
-- This file is every *goal_cup_tracer*.sql migration there, in name order, one after the other, each
-- under a "-- ===== <file name> =====" line. Do not edit it by hand. After a website migration merges,
-- run this from the root of the website checkout (at its main), with F set to this file, to rewrite it
-- (the header above the REGEN-END line is kept, the rest is regenerated):
--   F=<tracer>/server/worker/sql/goal-cup-tracer-upload.sql; { sed '/^-- REGEN-END/q' "$F"; for m in supabase/migrations/*goal_cup_tracer*.sql; do printf '\n-- ======================== %s ========================\n' "${m##*/}"; cat "$m"; done; } > "$F.new" && mv "$F.new" "$F"
-- Change a migration there first, then regenerate here. Applied to the Agent Community Supabase
-- project on 2026-10-15 (20261016120000: applied before this copy of it merged).
-- REGEN-END

-- ======================== 20261015120000_goal_cup_tracer_upload.sql ========================
-- Goal Cup tracer upload: the database half of the upload Worker (goal-cup-tracer, server/worker).
--
-- Adapted from an earlier proposal in the goal-cup-tracer repo (since removed) to this repo's
-- conventions. Additive only: a new private schema, new tables, new functions, one new role.
-- Nothing existing is altered, and no existing row is read for writing.
--
--   * Tables live in the new schema goal_cup_tracer. It is NOT exposed to PostgREST (the project
--     exposes public and graphql_public only), so no API key can address a table directly.
--     RLS is enabled on every table with no policies, and every privilege is revoked from
--     PUBLIC, anon and authenticated.
--   * The Worker calls public.goal_cup_upload_* through PostgREST RPC. Each is SECURITY DEFINER
--     with a fixed search_path and is executable only by the dedicated role
--     goal_cup_ingest_staging (the staging Worker's API key carries this role) and service_role.
--     goal_cup_upload_approve_device (approve as any member) is granted to the staging role only.
--   * The website's approval page calls goal_cup_device_request, goal_cup_accept_terms and
--     goal_cup_approve_device with the service-role client and the signed-in member as
--     p_actor_id (service_role only, like every other server RPC here).
--   * member_id is the auth.users id. There is deliberately no foreign key to auth.users: a key
--     would change what deleting an account does. Erasure/export hooks are a follow-up.
--   * Times cross the API as RFC 3339 UTC strings to the second (goal_cup_tracer.fmt_time).
--   * Mail: the Worker's cron queues rows in goal_cup_tracer.notices and sends each once through
--     Resend. The copy, the recipient (auth.users.email, read only) and the suppression check
--     (public.email_suppression_list, read only) are decided here, in goal_cup_upload_claim_notices.
--
-- Apply: psql "$DATABASE_URL_REMOTE" -X -1 -v ON_ERROR_STOP=1 -f <this file>; then pnpm db:check.

CREATE SCHEMA goal_cup_tracer;
REVOKE ALL ON SCHEMA goal_cup_tracer FROM PUBLIC, anon, authenticated;

-- ---- tables ---------------------------------------------------------------------------------

CREATE TABLE goal_cup_tracer.events (
  id                   text PRIMARY KEY CHECK (id ~ '^[a-z0-9][a-z0-9-]{0,63}$'),
  mode                 text NOT NULL CHECK (mode IN ('competition', 'dry_run', 'practice')),
  start_window_opens   timestamptz NOT NULL,
  start_window_closes  timestamptz NOT NULL,
  run_hours            integer NOT NULL DEFAULT 168,
  staging              boolean NOT NULL DEFAULT false,
  created_at           timestamptz NOT NULL DEFAULT now()
);

-- Goal Cup sign-up and terms: one row per member and accepted terms version.
CREATE TABLE goal_cup_tracer.consents (
  member_id     uuid NOT NULL,
  terms_version text NOT NULL CHECK (length(terms_version) BETWEEN 1 AND 64),
  accepted_at   timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (member_id, terms_version)
);

-- The device flow. Only the sha256 of the device code and of the user code (8 letters, upper
-- case, no dash) is stored.
CREATE TABLE goal_cup_tracer.device_requests (
  device_hash    text PRIMARY KEY CHECK (device_hash ~ '^[0-9a-f]{64}$'),
  user_hash      text NOT NULL UNIQUE CHECK (user_hash ~ '^[0-9a-f]{64}$'),
  purpose        text NOT NULL CHECK (purpose IN ('login', 'run')),
  mode           text NOT NULL CHECK (mode IN ('competition', 'practice')),
  connector_id   text NOT NULL,
  tracer_version text NOT NULL,
  created_at     timestamptz NOT NULL,
  expires_at     timestamptz NOT NULL,
  status         text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'approved', 'denied')),
  member_id      uuid,
  decided_at     timestamptz,
  picked_up_at   timestamptz
);

CREATE TABLE goal_cup_tracer.runs (
  id                    text PRIMARY KEY CHECK (id ~ '^r[0-9a-f]{16}$'),
  event_id              text NOT NULL REFERENCES goal_cup_tracer.events (id),
  member_id             uuid NOT NULL,
  mode                  text NOT NULL CHECK (mode IN ('competition', 'practice')),
  official_start        timestamptz NOT NULL,
  deadline_at           timestamptz NOT NULL,
  replaced_at           timestamptz,
  status                text NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'sealed', 'incomplete', 'abandoned')),
  repo_url              text NOT NULL,
  machine               jsonb,
  client_ip             text,
  client_country        text,
  last_client_ip        text,
  last_client_country   text,
  abandon_reason        text,
  sealed_at             timestamptz,
  last_heartbeat_at     timestamptz,
  last_model_request_at timestamptz,
  flagged_at            timestamptz,
  flag_reason           text CHECK (flag_reason IN ('gaps', 'failed')),
  final_commit          text CHECK (final_commit ~ '^([0-9a-f]{40}|none)$'),
  final_commit_at       timestamptz,
  created_at            timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX goal_cup_tracer_runs_one_main_per_member ON goal_cup_tracer.runs (member_id)
  WHERE status = 'open' AND mode = 'competition' AND replaced_at IS NULL;

-- A run credential. Minted at pickup, only its sha256 is stored. Unbound it lasts one hour from
-- the approval; bound to a run, until 24 hours after that run's deadline.
CREATE TABLE goal_cup_tracer.run_credentials (
  credential_hash text PRIMARY KEY CHECK (credential_hash ~ '^[0-9a-f]{64}$'),
  member_id       uuid NOT NULL,
  mode            text NOT NULL CHECK (mode IN ('competition', 'practice')),
  run_id          text REFERENCES goal_cup_tracer.runs (id),
  expires_at      timestamptz NOT NULL,
  created_at      timestamptz NOT NULL DEFAULT now()
);

-- One segment of a run.
CREATE TABLE goal_cup_tracer.entries (
  id                   text PRIMARY KEY CHECK (id ~ '^e[0-9a-f]{16}$'),
  run_id               text NOT NULL REFERENCES goal_cup_tracer.runs (id),
  segment              integer NOT NULL DEFAULT 0,
  opened_at            timestamptz NOT NULL DEFAULT now(),
  connector_id         text NOT NULL,
  manifest_sha256      text NOT NULL,
  connector_dir_sha256 text NOT NULL,
  workspace            text NOT NULL,
  session_id           text NOT NULL,
  harness_version      text NOT NULL,
  tracer_version       text NOT NULL,
  receipt              jsonb,
  upload_offset        bigint NOT NULL DEFAULT 0,
  next_seq             bigint NOT NULL DEFAULT 0,
  head                 text NOT NULL DEFAULT repeat('0', 64),
  last_ack             jsonb,
  last_chunk_at        timestamptz,
  model_requests       bigint,
  final_seq            bigint,
  final_head           text,
  final_repo_head      text,
  final_at             timestamptz,
  seal_sha256          text,
  baseline_sha256      text,
  seal_key             text,
  baseline_key         text,
  sealed_at            timestamptz,
  CHECK ((final_seq IS NULL) = (final_head IS NULL)),
  UNIQUE (run_id, segment)
);

CREATE TABLE goal_cup_tracer.heartbeats (
  id             bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  run_id         text NOT NULL REFERENCES goal_cup_tracer.runs (id),
  entry_id       text NOT NULL REFERENCES goal_cup_tracer.entries (id),
  received_at    timestamptz NOT NULL,
  seq            bigint NOT NULL,
  head           text NOT NULL,
  offset_local   bigint NOT NULL,
  final          boolean NOT NULL,
  repo_head      text,
  recording      text CHECK (recording IN ('ok', 'gaps', 'failed', 'unknown')),
  model_requests bigint,
  client_ip      text,
  client_country text
);
CREATE INDEX goal_cup_tracer_heartbeats_run_received ON goal_cup_tracer.heartbeats (run_id, received_at);

-- The outbox of run emails: queued by the Worker's cron, sent once (sent_at), or skipped.
CREATE TABLE goal_cup_tracer.notices (
  id              bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  run_id          text NOT NULL REFERENCES goal_cup_tracer.runs (id),
  member_id       uuid NOT NULL,
  kind            text NOT NULL CHECK (kind IN ('started', 'silence', 'idle_3h', 'idle_6h', 'recording', 'ended')),
  dedupe_key      text NOT NULL,
  payload         jsonb NOT NULL DEFAULT '{}',
  created_at      timestamptz NOT NULL,
  claimed_at      timestamptz,
  attempts        integer NOT NULL DEFAULT 0,
  sent_at         timestamptz,
  sent_to         text,
  provider_id     text,
  skipped_at      timestamptz,
  skipped_reason  text,
  last_error      text,
  UNIQUE (run_id, kind, dedupe_key)
);
CREATE INDEX goal_cup_tracer_notices_unsent ON goal_cup_tracer.notices (created_at)
  WHERE sent_at IS NULL AND skipped_at IS NULL;

CREATE TABLE goal_cup_tracer.chunks (
  entry_id     text NOT NULL REFERENCES goal_cup_tracer.entries (id),
  "offset"     bigint NOT NULL,
  length       bigint NOT NULL CHECK (length > 0),
  sha256       text NOT NULL,
  first_seq    bigint NOT NULL,
  last_seq     bigint NOT NULL,
  prev         text NOT NULL,
  head         text NOT NULL,
  r2_key       text NOT NULL,
  received_at  timestamptz NOT NULL,
  ack          jsonb NOT NULL,
  PRIMARY KEY (entry_id, "offset")
);

ALTER TABLE goal_cup_tracer.events          ENABLE ROW LEVEL SECURITY;
ALTER TABLE goal_cup_tracer.consents        ENABLE ROW LEVEL SECURITY;
ALTER TABLE goal_cup_tracer.device_requests ENABLE ROW LEVEL SECURITY;
ALTER TABLE goal_cup_tracer.runs            ENABLE ROW LEVEL SECURITY;
ALTER TABLE goal_cup_tracer.run_credentials ENABLE ROW LEVEL SECURITY;
ALTER TABLE goal_cup_tracer.entries         ENABLE ROW LEVEL SECURITY;
ALTER TABLE goal_cup_tracer.heartbeats      ENABLE ROW LEVEL SECURITY;
ALTER TABLE goal_cup_tracer.notices         ENABLE ROW LEVEL SECURITY;
ALTER TABLE goal_cup_tracer.chunks          ENABLE ROW LEVEL SECURITY;
REVOKE ALL ON ALL TABLES    IN SCHEMA goal_cup_tracer FROM PUBLIC, anon, authenticated;
REVOKE ALL ON ALL SEQUENCES IN SCHEMA goal_cup_tracer FROM PUBLIC, anon, authenticated;

-- ---- the staging Worker's role -----------------------------------------------------------
-- The staging Worker's Supabase secret API key carries this role (its JWT template sets
-- role=goal_cup_ingest_staging), so PostgREST switches to it. It has no table privileges and no
-- usage on goal_cup_tracer: it can execute the functions granted below and nothing else.
DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'goal_cup_ingest_staging') THEN
    CREATE ROLE goal_cup_ingest_staging NOLOGIN NOINHERIT;
  END IF;
END $$;
ALTER ROLE goal_cup_ingest_staging SET statement_timeout = '8s';
GRANT goal_cup_ingest_staging TO authenticator;

-- ---- helpers (schema goal_cup_tracer; never granted to anyone) --------------------------------

CREATE FUNCTION goal_cup_tracer.fmt_time(t timestamptz) RETURNS text
LANGUAGE sql IMMUTABLE SET search_path = pg_catalog, pg_temp
AS $$ SELECT to_char(t AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"') $$;

-- The terms version a member must have accepted. A new version is a new migration.
CREATE FUNCTION goal_cup_tracer.current_terms_version() RETURNS text
LANGUAGE sql IMMUTABLE SET search_path = pg_catalog, pg_temp
AS $$ SELECT 'goal-cup-tracer-2026-10-15'::text $$;

CREATE FUNCTION goal_cup_tracer.has_current_terms(p_member uuid) RETURNS boolean
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
  SELECT EXISTS (SELECT 1 FROM goal_cup_tracer.consents
                  WHERE member_id = p_member AND terms_version = goal_cup_tracer.current_terms_version())
$$;

-- The handle a login request hands back: the member's directory slug, else 'member'.
CREATE FUNCTION goal_cup_tracer.member_handle(p_member uuid) RETURNS text
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
  SELECT coalesce((SELECT mp.slug FROM public.member_profiles mp
                    WHERE mp.user_id = p_member AND mp.slug IS NOT NULL
                    ORDER BY mp.is_listed DESC NULLS LAST LIMIT 1), 'member')
$$;

-- The run and one of its entries as the Worker's {run, entry}.
CREATE FUNCTION goal_cup_tracer.run_json(p_run_id text, p_entry_id text DEFAULT NULL) RETURNS jsonb
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
      'current_sealed_at', fmt_time(cur.sealed_at)),
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
  WHERE r.id = p_run_id
$$;

CREATE FUNCTION goal_cup_tracer.run_status_at(p_run_id text, p_at timestamptz) RETURNS text
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
  SELECT CASE
    WHEN r.status <> 'open' OR p_at < least(r.deadline_at, coalesce(r.replaced_at, r.deadline_at)) THEN r.status
    WHEN cur.sealed_at IS NOT NULL THEN 'sealed'
    WHEN p_at > least(r.deadline_at, coalesce(r.replaced_at, r.deadline_at)) + interval '24 hours' THEN 'incomplete'
    ELSE 'open'
  END
  FROM runs r
  CROSS JOIN LATERAL (SELECT e.sealed_at FROM entries e WHERE e.run_id = r.id ORDER BY e.segment DESC LIMIT 1) cur
  WHERE r.id = p_run_id
$$;

CREATE FUNCTION goal_cup_tracer.note_heartbeat(
  p_run_id text, p_entry_id text, p_at timestamptz, p_recording text, p_model_requests bigint,
  p_client_ip text, p_client_country text
) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_grew boolean := false;
BEGIN
  IF p_model_requests IS NOT NULL THEN
    UPDATE entries SET model_requests = p_model_requests
     WHERE id = p_entry_id AND (model_requests IS NULL OR p_model_requests > model_requests);
    v_grew := FOUND;
  END IF;
  UPDATE runs SET
    last_heartbeat_at     = p_at,
    last_client_ip        = coalesce(p_client_ip, last_client_ip),
    last_client_country   = coalesce(p_client_country, last_client_country),
    flagged_at            = CASE WHEN flagged_at IS NULL AND p_recording IN ('gaps', 'failed') THEN p_at ELSE flagged_at END,
    flag_reason           = CASE WHEN flagged_at IS NULL AND p_recording IN ('gaps', 'failed') THEN p_recording ELSE flag_reason END,
    last_model_request_at = CASE WHEN v_grew THEN p_at ELSE last_model_request_at END
  WHERE id = p_run_id;
END $$;

-- Decide a pending request by its user code. 'ok' | 'not_found' | 'expired' | 'already_decided'.
CREATE FUNCTION goal_cup_tracer.decide_device(p_user_code text, p_member uuid, p_approve boolean, p_now timestamptz) RETURNS text
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_dev device_requests;
BEGIN
  SELECT * INTO v_dev FROM device_requests
   WHERE user_hash = encode(sha256(convert_to(upper(replace(p_user_code, '-', '')), 'UTF8')), 'hex')
     FOR UPDATE;
  IF NOT FOUND THEN RETURN 'not_found'; END IF;
  IF p_now >= v_dev.expires_at THEN RETURN 'expired'; END IF;
  IF v_dev.status <> 'pending' THEN RETURN 'already_decided'; END IF;
  UPDATE device_requests
     SET status = CASE WHEN p_approve THEN 'approved' ELSE 'denied' END,
         member_id = p_member, decided_at = p_now
   WHERE device_hash = v_dev.device_hash;
  RETURN 'ok';
END $$;

-- The email for one notice: {subject, text}. The website owns this copy.
CREATE FUNCTION goal_cup_tracer.notice_text(p_kind text, p_payload jsonb, p_run_id text) RETURNS jsonb
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
    END AS body) s
$$;

-- ---- the Worker's functions (public, granted to the Worker role and service_role) --------------

CREATE FUNCTION public.goal_cup_upload_credential(p_credential_hash text, p_now text) RETURNS jsonb
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
  SELECT jsonb_build_object('member_id', c.member_id::text, 'run_id', c.run_id, 'mode', c.mode,
                            'expires_at', fmt_time(c.expires_at))
    FROM run_credentials c
   WHERE c.credential_hash = p_credential_hash AND p_now::timestamptz < c.expires_at
$$;

CREATE FUNCTION public.goal_cup_upload_create_device(
  p_device_hash text, p_user_hash text, p_purpose text, p_mode text,
  p_connector_id text, p_tracer_version text, p_created_at text, p_expires_at text
) RETURNS text
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
BEGIN
  INSERT INTO device_requests (device_hash, user_hash, purpose, mode, connector_id, tracer_version, created_at, expires_at)
  VALUES (p_device_hash, p_user_hash, p_purpose, p_mode, p_connector_id, p_tracer_version,
          p_created_at::timestamptz, p_expires_at::timestamptz);
  RETURN 'created';
END $$;

CREATE FUNCTION public.goal_cup_upload_poll_device(p_device_hash text, p_now text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_now     timestamptz := p_now::timestamptz;
  v_dev     device_requests;
  v_plain   text;
  v_expires timestamptz;
BEGIN
  SELECT * INTO v_dev FROM device_requests WHERE device_hash = p_device_hash FOR UPDATE;
  IF NOT FOUND OR v_now >= v_dev.expires_at OR v_dev.picked_up_at IS NOT NULL THEN
    RETURN jsonb_build_object('result', 'expired');
  END IF;
  IF v_dev.status = 'pending' THEN RETURN jsonb_build_object('result', 'pending'); END IF;
  IF v_dev.status = 'denied' THEN RETURN jsonb_build_object('result', 'denied'); END IF;

  UPDATE device_requests SET picked_up_at = v_now WHERE device_hash = p_device_hash;
  IF v_dev.purpose = 'login' THEN
    RETURN jsonb_build_object('result', 'approved', 'purpose', 'login', 'handle', member_handle(v_dev.member_id));
  END IF;

  v_plain := 'gcr_' || replace(gen_random_uuid()::text || gen_random_uuid()::text, '-', '');
  v_expires := v_dev.decided_at + interval '1 hour';
  INSERT INTO run_credentials (credential_hash, member_id, mode, expires_at)
  VALUES (encode(sha256(convert_to(v_plain, 'UTF8')), 'hex'), v_dev.member_id, v_dev.mode, v_expires);
  RETURN jsonb_build_object('result', 'approved', 'purpose', 'run', 'credential', v_plain,
                            'expires_at', fmt_time(v_expires));
END $$;

-- STAGING ONLY: approve as any member (POST /v1/device/approve, behind TEST_APPROVAL_KEY).
-- Granted to goal_cup_ingest_staging only, never to service_role or a production role.
CREATE FUNCTION public.goal_cup_upload_approve_device(p_user_code text, p_member_id text, p_approve boolean, p_now text) RETURNS text
LANGUAGE sql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$ SELECT decide_device(p_user_code, p_member_id::uuid, p_approve, p_now::timestamptz) $$;

CREATE FUNCTION public.goal_cup_upload_open_run(
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
  v_member uuid := p_member_id::uuid;
  v_event  events;
  v_start  timestamptz := p_official_start::timestamptz;
  v_bound  text;
  v_try    integer;
BEGIN
  SELECT * INTO v_event FROM events WHERE id = p_event_id;
  IF NOT FOUND THEN
    RETURN jsonb_build_object('result', 'forbidden', 'message', 'This event is not open for runs.');
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

  SELECT run_id INTO v_bound FROM run_credentials WHERE credential_hash = p_credential_hash FOR UPDATE;
  IF NOT FOUND THEN
    RETURN jsonb_build_object('result', 'conflict');
  END IF;
  IF v_bound IS NOT NULL THEN
    IF EXISTS (SELECT 1 FROM runs r JOIN entries e ON e.run_id = r.id AND e.segment = 0
                WHERE r.id = v_bound AND r.member_id = v_member AND e.session_id = p_session_id) THEN
      RETURN jsonb_build_object('result', 'existing')
             || run_json(v_bound, (SELECT e.id FROM entries e WHERE e.run_id = v_bound AND e.segment = 0));
    END IF;
    RETURN jsonb_build_object('result', 'conflict');
  END IF;

  FOR v_try IN 1..2 LOOP
    IF p_mode = 'competition' THEN
      UPDATE runs SET replaced_at = greatest(v_start, official_start)
       WHERE member_id = v_member AND mode = 'competition' AND status = 'open' AND replaced_at IS NULL;
    END IF;
    BEGIN
      INSERT INTO runs (id, event_id, member_id, mode, official_start, deadline_at, repo_url,
                        machine, client_ip, client_country)
      VALUES (p_run_id, p_event_id, v_member, p_mode, v_start, p_deadline_at::timestamptz, p_repo_url,
              nullif(p_machine, 'null'::jsonb), p_client_ip, p_client_country);
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
     SET run_id = p_run_id, expires_at = p_credential_expires_at::timestamptz
   WHERE credential_hash = p_credential_hash;
  RETURN jsonb_build_object('result', 'created') || run_json(p_run_id);
END $$;

CREATE FUNCTION public.goal_cup_upload_continue_run(
  p_member_id text, p_run_id text, p_previous_entry_id text, p_entry_id text,
  p_connector_id text, p_manifest_sha256 text, p_connector_dir_sha256 text, p_workspace text,
  p_session_id text, p_harness_version text, p_tracer_version text,
  p_client_ip text, p_client_country text, p_at text
) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_run runs;
  v_cur entries;
  v_at  timestamptz := p_at::timestamptz;
BEGIN
  SELECT * INTO v_run FROM runs WHERE id = p_run_id AND member_id = p_member_id::uuid FOR UPDATE;
  IF NOT FOUND THEN RETURN jsonb_build_object('result', 'not_found'); END IF;
  SELECT * INTO v_cur FROM entries WHERE run_id = p_run_id ORDER BY segment DESC LIMIT 1;

  IF v_cur.segment > 0 AND v_cur.session_id = p_session_id
     AND EXISTS (SELECT 1 FROM entries
                  WHERE run_id = p_run_id AND segment = v_cur.segment - 1 AND id = p_previous_entry_id) THEN
    RETURN jsonb_build_object('result', 'existing') || run_json(p_run_id);
  END IF;
  IF v_run.status <> 'open' OR v_at >= least(v_run.deadline_at, coalesce(v_run.replaced_at, v_run.deadline_at)) THEN
    RETURN jsonb_build_object('result', 'closed');
  END IF;
  IF v_cur.id <> p_previous_entry_id THEN
    RETURN jsonb_build_object('result', 'conflict');
  END IF;

  INSERT INTO entries (id, run_id, segment, connector_id, manifest_sha256, connector_dir_sha256,
                       workspace, session_id, harness_version, tracer_version)
  VALUES (p_entry_id, p_run_id, v_cur.segment + 1, p_connector_id, p_manifest_sha256, p_connector_dir_sha256,
          p_workspace, p_session_id, p_harness_version, p_tracer_version);
  UPDATE runs SET last_client_ip = coalesce(p_client_ip, last_client_ip),
                  last_client_country = coalesce(p_client_country, last_client_country)
   WHERE id = p_run_id;
  RETURN jsonb_build_object('result', 'created') || run_json(p_run_id);
END $$;

CREATE FUNCTION public.goal_cup_upload_run_position(p_member_id text, p_run_id text, p_entry_id text DEFAULT NULL) RETURNS jsonb
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
  SELECT run_json(r.id, p_entry_id) FROM runs r WHERE r.id = p_run_id AND r.member_id = p_member_id::uuid
$$;

CREATE FUNCTION public.goal_cup_upload_get_chunk(p_member_id text, p_run_id text, p_entry_id text, p_offset bigint) RETURNS jsonb
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
  SELECT jsonb_build_object(
    'entry_id', c.entry_id, 'offset', c."offset", 'length', c.length, 'sha256', c.sha256,
    'first_seq', c.first_seq, 'last_seq', c.last_seq, 'prev', c.prev, 'head', c.head,
    'r2_key', c.r2_key, 'received_at', fmt_time(c.received_at), 'ack', c.ack)
  FROM chunks c
  JOIN entries e ON e.id = c.entry_id
  JOIN runs r ON r.id = e.run_id
  WHERE r.id = p_run_id AND r.member_id = p_member_id::uuid AND c.entry_id = p_entry_id AND c."offset" = p_offset
$$;

CREATE FUNCTION public.goal_cup_upload_issue_receipt(p_member_id text, p_run_id text, p_entry_id text, p_chain_head text, p_receipt jsonb) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_run   runs;
  v_entry entries;
BEGIN
  SELECT * INTO v_run FROM runs WHERE id = p_run_id AND member_id = p_member_id::uuid FOR UPDATE;
  IF NOT FOUND THEN RETURN jsonb_build_object('result', 'not_found'); END IF;
  SELECT * INTO v_entry FROM entries WHERE id = p_entry_id AND run_id = p_run_id FOR UPDATE;
  IF NOT FOUND THEN RETURN jsonb_build_object('result', 'not_found'); END IF;
  IF v_run.status IN ('abandoned', 'incomplete') OR v_entry.sealed_at IS NOT NULL THEN
    RETURN jsonb_build_object('result', 'closed');
  END IF;
  IF v_entry.receipt IS NOT NULL THEN
    IF v_entry.receipt -> 'claims' ->> 'chain_head' = p_chain_head THEN
      RETURN jsonb_build_object('result', 'existing', 'receipt', v_entry.receipt);
    END IF;
    RETURN jsonb_build_object('result', 'conflict');
  END IF;
  UPDATE entries SET receipt = p_receipt WHERE id = p_entry_id;
  RETURN jsonb_build_object('result', 'stored', 'receipt', p_receipt);
END $$;

CREATE FUNCTION public.goal_cup_upload_abandon_run(p_member_id text, p_run_id text, p_reason text, p_at text) RETURNS text
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_run runs;
BEGIN
  SELECT * INTO v_run FROM runs WHERE id = p_run_id AND member_id = p_member_id::uuid FOR UPDATE;
  IF NOT FOUND THEN RETURN 'not_found'; END IF;
  IF v_run.status <> 'open' THEN RETURN 'closed'; END IF;
  IF EXISTS (SELECT 1 FROM chunks c JOIN entries e ON e.id = c.entry_id WHERE e.run_id = p_run_id) THEN
    RETURN 'has_chunks';
  END IF;
  UPDATE runs SET status = 'abandoned', abandon_reason = p_reason, sealed_at = p_at::timestamptz WHERE id = p_run_id;
  RETURN 'abandoned';
END $$;

CREATE FUNCTION public.goal_cup_upload_commit_chunk(p_member_id text, p_run_id text, p_chunk jsonb) RETURNS text
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_run   runs;
  v_entry entries;
  v_off   bigint := (p_chunk ->> 'offset')::bigint;
  v_len   bigint := (p_chunk ->> 'length')::bigint;
BEGIN
  SELECT * INTO v_run FROM runs WHERE id = p_run_id AND member_id = p_member_id::uuid FOR UPDATE;
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

  INSERT INTO chunks (entry_id, "offset", length, sha256, first_seq, last_seq, prev, head, r2_key, received_at, ack)
  VALUES (v_entry.id, v_off, v_len, p_chunk ->> 'sha256', (p_chunk ->> 'first_seq')::bigint,
          (p_chunk ->> 'last_seq')::bigint, p_chunk ->> 'prev', p_chunk ->> 'head', p_chunk ->> 'r2_key',
          (p_chunk ->> 'received_at')::timestamptz, p_chunk -> 'ack');

  UPDATE entries
     SET upload_offset = v_off + v_len,
         next_seq      = (p_chunk ->> 'last_seq')::bigint + 1,
         head          = p_chunk ->> 'head',
         last_ack      = p_chunk -> 'ack',
         last_chunk_at = (p_chunk ->> 'received_at')::timestamptz
   WHERE id = v_entry.id;
  RETURN 'committed';
END $$;

CREATE FUNCTION public.goal_cup_upload_seal_run(
  p_member_id text, p_run_id text, p_entry_id text,
  p_seal_sha256 text, p_baseline_sha256 text, p_seal_key text, p_baseline_key text, p_sealed_at text
) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_run    runs;
  v_entry  entries;
  v_at     timestamptz := p_sealed_at::timestamptz;
  v_status text;
BEGIN
  SELECT * INTO v_run FROM runs WHERE id = p_run_id AND member_id = p_member_id::uuid FOR UPDATE;
  IF NOT FOUND THEN RETURN jsonb_build_object('result', 'not_found'); END IF;
  SELECT * INTO v_entry FROM entries WHERE id = p_entry_id AND run_id = p_run_id FOR UPDATE;
  IF NOT FOUND THEN RETURN jsonb_build_object('result', 'not_found'); END IF;

  IF v_entry.sealed_at IS NOT NULL THEN
    IF v_entry.seal_sha256 = p_seal_sha256 AND v_entry.baseline_sha256 = p_baseline_sha256 THEN
      RETURN jsonb_build_object('result', 'already_sealed', 'run_status', run_status_at(p_run_id, v_at));
    END IF;
    RETURN jsonb_build_object('result', 'conflict');
  END IF;
  IF v_run.status IN ('abandoned', 'incomplete') THEN RETURN jsonb_build_object('result', 'closed'); END IF;
  IF v_entry.final_seq IS NULL THEN RETURN jsonb_build_object('result', 'no_final'); END IF;
  IF v_entry.next_seq - 1 <> v_entry.final_seq OR v_entry.head <> v_entry.final_head THEN
    RETURN jsonb_build_object('result', 'chain_behind');
  END IF;

  UPDATE entries
     SET seal_sha256 = p_seal_sha256, baseline_sha256 = p_baseline_sha256,
         seal_key = p_seal_key, baseline_key = p_baseline_key, sealed_at = v_at
   WHERE id = p_entry_id;
  v_status := run_status_at(p_run_id, v_at);
  IF v_status <> v_run.status THEN
    UPDATE runs SET status = v_status, sealed_at = CASE WHEN v_status = 'sealed' THEN v_at ELSE sealed_at END
     WHERE id = p_run_id;
  END IF;
  RETURN jsonb_build_object('result', 'sealed', 'run_status', v_status);
END $$;

CREATE FUNCTION public.goal_cup_upload_heartbeat(
  p_member_id text, p_run_id text, p_entry_id text,
  p_seq bigint, p_head text, p_offset_local bigint, p_final boolean, p_repo_head text,
  p_recording text, p_model_requests bigint, p_client_ip text, p_client_country text, p_at text
) RETURNS text
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_run    runs;
  v_entry  entries;
  v_at     timestamptz := p_at::timestamptz;
  v_stored bigint;
BEGIN
  SELECT * INTO v_run FROM runs WHERE id = p_run_id AND member_id = p_member_id::uuid FOR UPDATE;
  IF NOT FOUND THEN RETURN 'not_found'; END IF;
  IF v_run.status IN ('abandoned', 'incomplete') THEN RETURN 'closed'; END IF;
  SELECT * INTO v_entry FROM entries WHERE id = p_entry_id AND run_id = p_run_id FOR UPDATE;
  IF NOT FOUND THEN RETURN 'not_found'; END IF;

  IF NOT p_final THEN
    IF v_entry.sealed_at IS NULL THEN
      INSERT INTO heartbeats (run_id, entry_id, received_at, seq, head, offset_local, final,
                              recording, model_requests, client_ip, client_country)
      VALUES (p_run_id, p_entry_id, v_at, p_seq, p_head, p_offset_local, false, p_recording, p_model_requests,
              p_client_ip, p_client_country);
      PERFORM note_heartbeat(p_run_id, p_entry_id, v_at, p_recording, p_model_requests, p_client_ip, p_client_country);
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
                          recording, model_requests, client_ip, client_country)
  VALUES (p_run_id, p_entry_id, v_at, p_seq, p_head, p_offset_local, true, p_repo_head, p_recording, p_model_requests,
          p_client_ip, p_client_country);
  PERFORM note_heartbeat(p_run_id, p_entry_id, v_at, p_recording, p_model_requests, p_client_ip, p_client_country);
  RETURN 'final_committed';
END $$;

CREATE FUNCTION public.goal_cup_upload_close_run(p_member_id text, p_run_id text, p_at text) RETURNS text
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_run    runs;
  v_at     timestamptz := p_at::timestamptz;
  v_status text;
BEGIN
  SELECT * INTO v_run FROM runs WHERE id = p_run_id AND member_id = p_member_id::uuid FOR UPDATE;
  IF NOT FOUND THEN RETURN NULL; END IF;
  v_status := run_status_at(p_run_id, v_at);
  IF v_status <> v_run.status THEN
    UPDATE runs
       SET status = v_status,
           sealed_at = CASE WHEN v_status = 'sealed'
                            THEN (SELECT e.sealed_at FROM entries e WHERE e.run_id = p_run_id ORDER BY e.segment DESC LIMIT 1)
                            ELSE sealed_at END
     WHERE id = p_run_id;
  END IF;
  RETURN v_status;
END $$;

CREATE FUNCTION public.goal_cup_upload_runs_for_tick(p_at text) RETURNS jsonb
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
  SELECT coalesce(jsonb_agg(
           (run_json(r.id) -> 'run') || jsonb_build_object(
             'last_heartbeat_at',     fmt_time(r.last_heartbeat_at),
             'last_model_request_at', fmt_time(r.last_model_request_at),
             'flagged_at',            fmt_time(r.flagged_at),
             'flag_reason',           r.flag_reason,
             'final_commit',          r.final_commit)
           ORDER BY r.official_start, r.id), '[]'::jsonb)
    FROM runs r
   WHERE r.status <> 'abandoned'
     AND (r.status = 'open'
          OR least(r.deadline_at, coalesce(r.replaced_at, r.deadline_at)) >= p_at::timestamptz - interval '48 hours')
$$;

CREATE FUNCTION public.goal_cup_upload_queue_notices(p_notices jsonb) RETURNS integer
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_rows integer;
BEGIN
  INSERT INTO notices (run_id, member_id, kind, dedupe_key, payload, created_at)
  SELECT r.id, r.member_id, n.kind, n.dedupe_key, coalesce(n.payload, '{}'::jsonb), n.at::timestamptz
    FROM jsonb_to_recordset(p_notices) AS n(run_id text, member_id text, kind text, dedupe_key text, payload jsonb, at text)
    JOIN runs r ON r.id = n.run_id AND r.member_id = n.member_id::uuid
  ON CONFLICT (run_id, kind, dedupe_key) DO NOTHING;
  GET DIAGNOSTICS v_rows = ROW_COUNT;
  RETURN v_rows;
END $$;

CREATE FUNCTION public.goal_cup_upload_record_final_commit(p_member_id text, p_run_id text, p_commit text, p_at text) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_rows integer;
BEGIN
  UPDATE runs SET final_commit = p_commit, final_commit_at = p_at::timestamptz
   WHERE id = p_run_id AND member_id = p_member_id::uuid AND final_commit IS NULL;
  GET DIAGNOSTICS v_rows = ROW_COUNT;
  RETURN v_rows = 1;
END $$;

-- The mailer, step 1: claim up to p_limit unsent notices (a claim lasts 15 minutes, so a send that
-- died is retried; the Worker sends with Resend idempotency key goal-cup-notice/<id>, so a retry
-- inside Resend's 24 hours cannot send twice). A notice whose member has no address, or whose
-- address is suppressed, is marked skipped here and not returned. Returns
-- [{id, kind, run_id, to, subject, text}]; "to" is null only when the member has no auth.users
-- row (synthetic staging members), and the Worker then sends it to its staging test address or
-- skips it.
CREATE FUNCTION public.goal_cup_upload_claim_notices(p_limit integer, p_now text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_now timestamptz := p_now::timestamptz;
  v_out jsonb := '[]'::jsonb;
  v_n   record;
  v_to  text;
  v_has_user boolean;
BEGIN
  FOR v_n IN
    SELECT * FROM notices
     WHERE sent_at IS NULL AND skipped_at IS NULL
       AND (claimed_at IS NULL OR claimed_at < v_now - interval '15 minutes')
       AND attempts < 5
     ORDER BY created_at, id
     LIMIT greatest(least(coalesce(p_limit, 20), 50), 0)
     FOR UPDATE SKIP LOCKED
  LOOP
    SELECT u.email, true INTO v_to, v_has_user FROM auth.users u WHERE u.id = v_n.member_id;
    IF v_has_user IS NULL THEN
      v_to := NULL;
    ELSIF v_to IS NULL OR btrim(v_to) = '' THEN
      UPDATE notices SET skipped_at = v_now, skipped_reason = 'no_address' WHERE id = v_n.id;
      CONTINUE;
    ELSIF EXISTS (SELECT 1 FROM public.email_suppression_list s
                   WHERE lower(s.email) = lower(v_to) AND (s.expires_at IS NULL OR s.expires_at > v_now)) THEN
      UPDATE notices SET skipped_at = v_now, skipped_reason = 'suppressed' WHERE id = v_n.id;
      CONTINUE;
    END IF;
    UPDATE notices SET claimed_at = v_now, attempts = attempts + 1 WHERE id = v_n.id;
    v_out := v_out || jsonb_build_array(
      jsonb_build_object('id', v_n.id, 'kind', v_n.kind, 'run_id', v_n.run_id, 'to', v_to)
      || notice_text(v_n.kind, v_n.payload, v_n.run_id));
    v_has_user := NULL;
  END LOOP;
  RETURN v_out;
END $$;

-- The mailer, step 2: record the outcome of one claimed notice. p_outcome: 'sent' (p_detail is
-- the provider message id, p_to the address used), 'skipped' (p_detail is the reason) or
-- 'failed' (p_detail is a short error; the notice is claimed again after 15 minutes, up to 5
-- attempts). Returns 'ok' or 'not_found' (no such unsent notice).
CREATE FUNCTION public.goal_cup_upload_finish_notice(p_id bigint, p_outcome text, p_to text, p_detail text, p_now text) RETURNS text
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_now timestamptz := p_now::timestamptz;
BEGIN
  IF p_outcome = 'sent' THEN
    UPDATE notices SET sent_at = v_now, sent_to = left(p_to, 320), provider_id = left(p_detail, 200), last_error = NULL
     WHERE id = p_id AND sent_at IS NULL AND skipped_at IS NULL;
  ELSIF p_outcome = 'skipped' THEN
    UPDATE notices SET skipped_at = v_now, skipped_reason = left(p_detail, 200)
     WHERE id = p_id AND sent_at IS NULL AND skipped_at IS NULL;
  ELSIF p_outcome = 'failed' THEN
    UPDATE notices SET last_error = left(p_detail, 500)
     WHERE id = p_id AND sent_at IS NULL AND skipped_at IS NULL;
  ELSE
    RAISE EXCEPTION 'goal_cup_upload_finish_notice: unknown outcome %', p_outcome USING ERRCODE = '22023';
  END IF;
  IF NOT FOUND THEN RETURN 'not_found'; END IF;
  RETURN 'ok';
END $$;

-- ---- the website's functions (service_role only; the server passes the signed-in member) -------

-- What the approval page shows for a user code. {"result": "not_found"} or
-- {"result": "ok", "status": "pending"|"approved"|"denied"|"expired", "purpose", "mode",
--  "connector_id", "tracer_version", "expires_at", "replaces_open_run", "terms_ok",
--  "terms_version", "decided_by_you"}.
CREATE FUNCTION public.goal_cup_device_request(p_actor_id uuid, p_user_code text) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_dev device_requests;
BEGIN
  SELECT * INTO v_dev FROM device_requests
   WHERE user_hash = encode(sha256(convert_to(upper(replace(coalesce(p_user_code, ''), '-', '')), 'UTF8')), 'hex');
  IF NOT FOUND THEN RETURN jsonb_build_object('result', 'not_found'); END IF;
  RETURN jsonb_build_object(
    'result', 'ok',
    'status', CASE WHEN v_dev.status = 'pending' AND now() >= v_dev.expires_at THEN 'expired' ELSE v_dev.status END,
    'purpose', v_dev.purpose,
    'mode', v_dev.mode,
    'connector_id', v_dev.connector_id,
    'tracer_version', v_dev.tracer_version,
    'expires_at', fmt_time(v_dev.expires_at),
    'replaces_open_run', v_dev.purpose = 'run' AND v_dev.mode = 'competition' AND EXISTS (
      SELECT 1 FROM runs r WHERE r.member_id = p_actor_id AND r.mode = 'competition' AND r.status = 'open'
         AND r.replaced_at IS NULL AND now() < r.deadline_at),
    'terms_ok', has_current_terms(p_actor_id),
    'terms_version', current_terms_version(),
    'decided_by_you', v_dev.member_id IS NOT DISTINCT FROM p_actor_id AND v_dev.status <> 'pending');
END $$;

-- Goal Cup sign-up: the signed-in member accepts the current terms. 'ok' | 'stale_version' |
-- 'not_a_member'. Idempotent.
CREATE FUNCTION public.goal_cup_accept_terms(p_actor_id uuid, p_terms_version text) RETURNS text
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
BEGIN
  IF p_terms_version IS DISTINCT FROM current_terms_version() THEN RETURN 'stale_version'; END IF;
  IF NOT EXISTS (SELECT 1 FROM auth.users u WHERE u.id = p_actor_id AND u.email_confirmed_at IS NOT NULL) THEN
    RETURN 'not_a_member';
  END IF;
  INSERT INTO consents (member_id, terms_version) VALUES (p_actor_id, p_terms_version)
  ON CONFLICT (member_id, terms_version) DO NOTHING;
  RETURN 'ok';
END $$;

-- Approve or deny a device request as the signed-in member. The only production path that lets a
-- credential be minted. 'ok' | 'not_found' | 'expired' | 'already_decided' | 'terms_required' |
-- 'not_a_member'. A denial needs neither terms nor a verified email.
CREATE FUNCTION public.goal_cup_approve_device(p_actor_id uuid, p_user_code text, p_approve boolean) RETURNS text
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
BEGIN
  IF p_approve THEN
    IF NOT EXISTS (SELECT 1 FROM auth.users u WHERE u.id = p_actor_id AND u.email_confirmed_at IS NOT NULL) THEN
      RETURN 'not_a_member';
    END IF;
    IF NOT has_current_terms(p_actor_id) THEN RETURN 'terms_required'; END IF;
  END IF;
  RETURN decide_device(p_user_code, p_actor_id, coalesce(p_approve, false), now());
END $$;

-- ---- privileges, now that every function exists ----------------------------------------------
-- Supabase grants EXECUTE on new functions to PUBLIC, anon and authenticated; revoke all three
-- explicitly (docs/SUPABASE.md, "SECURITY DEFINER grants").
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA goal_cup_tracer FROM PUBLIC, anon, authenticated;
REVOKE ALL ON FUNCTION
  public.goal_cup_upload_credential(text, text),
  public.goal_cup_upload_create_device(text, text, text, text, text, text, text, text),
  public.goal_cup_upload_poll_device(text, text),
  public.goal_cup_upload_approve_device(text, text, boolean, text),
  public.goal_cup_upload_open_run(text, text, text, text, text, text, text, text, text, text, text, text, text, text, text, jsonb, text, text, text, text),
  public.goal_cup_upload_continue_run(text, text, text, text, text, text, text, text, text, text, text, text, text, text),
  public.goal_cup_upload_run_position(text, text, text),
  public.goal_cup_upload_get_chunk(text, text, text, bigint),
  public.goal_cup_upload_issue_receipt(text, text, text, text, jsonb),
  public.goal_cup_upload_abandon_run(text, text, text, text),
  public.goal_cup_upload_commit_chunk(text, text, jsonb),
  public.goal_cup_upload_seal_run(text, text, text, text, text, text, text, text),
  public.goal_cup_upload_heartbeat(text, text, text, bigint, text, bigint, boolean, text, text, bigint, text, text, text),
  public.goal_cup_upload_close_run(text, text, text),
  public.goal_cup_upload_runs_for_tick(text),
  public.goal_cup_upload_queue_notices(jsonb),
  public.goal_cup_upload_record_final_commit(text, text, text, text),
  public.goal_cup_upload_claim_notices(integer, text),
  public.goal_cup_upload_finish_notice(bigint, text, text, text, text),
  public.goal_cup_device_request(uuid, text),
  public.goal_cup_accept_terms(uuid, text),
  public.goal_cup_approve_device(uuid, text, boolean)
FROM PUBLIC, anon, authenticated;

-- The Worker: the staging role and service_role (a later production Worker role gets the same
-- list without goal_cup_upload_approve_device).
GRANT EXECUTE ON FUNCTION
  public.goal_cup_upload_credential(text, text),
  public.goal_cup_upload_create_device(text, text, text, text, text, text, text, text),
  public.goal_cup_upload_poll_device(text, text),
  public.goal_cup_upload_open_run(text, text, text, text, text, text, text, text, text, text, text, text, text, text, text, jsonb, text, text, text, text),
  public.goal_cup_upload_continue_run(text, text, text, text, text, text, text, text, text, text, text, text, text, text),
  public.goal_cup_upload_run_position(text, text, text),
  public.goal_cup_upload_get_chunk(text, text, text, bigint),
  public.goal_cup_upload_issue_receipt(text, text, text, text, jsonb),
  public.goal_cup_upload_abandon_run(text, text, text, text),
  public.goal_cup_upload_commit_chunk(text, text, jsonb),
  public.goal_cup_upload_seal_run(text, text, text, text, text, text, text, text),
  public.goal_cup_upload_heartbeat(text, text, text, bigint, text, bigint, boolean, text, text, bigint, text, text, text),
  public.goal_cup_upload_close_run(text, text, text),
  public.goal_cup_upload_runs_for_tick(text),
  public.goal_cup_upload_queue_notices(jsonb),
  public.goal_cup_upload_record_final_commit(text, text, text, text),
  public.goal_cup_upload_claim_notices(integer, text),
  public.goal_cup_upload_finish_notice(bigint, text, text, text, text)
TO goal_cup_ingest_staging, service_role;

-- STAGING ONLY: approve as any member.
GRANT EXECUTE ON FUNCTION public.goal_cup_upload_approve_device(text, text, boolean, text) TO goal_cup_ingest_staging;

-- The website's approval page (server, service-role client).
GRANT EXECUTE ON FUNCTION
  public.goal_cup_device_request(uuid, text),
  public.goal_cup_accept_terms(uuid, text),
  public.goal_cup_approve_device(uuid, text, boolean)
TO service_role;

-- ======================== 20261015120100_goal_cup_tracer_staging_events.sql ========================
-- Goal Cup tracer staging events (staging = true). Inserts two new rows into the new table
-- goal_cup_tracer.events (20261015120000_goal_cup_tracer_upload.sql); touches nothing else.
-- The staging upload Worker (goalcup-ingest-staging) is deployed with
--   EVENT_ID          = goalcup-dryrun-staging    (its "competition" runs: 168 h, start window below)
--   PRACTICE_EVENT_ID = goalcup-practice-staging  (10-minute practice runs; no start window)
INSERT INTO goal_cup_tracer.events (id, mode, start_window_opens, start_window_closes, run_hours, staging)
VALUES
  ('goalcup-practice-staging', 'practice', '2026-10-15T00:00:00Z', '2027-01-31T00:00:00Z', 168, true),
  ('goalcup-dryrun-staging',   'dry_run',  '2026-10-15T00:00:00Z', '2026-11-05T00:00:00Z', 168, true)
ON CONFLICT (id) DO NOTHING;

-- ======================== 20261015130000_goal_cup_tracer_deployment_fence.sql ========================
-- Goal Cup tracer: fence staging from production (security review, 2026-10-15).
--
-- The staging upload Worker and the website share one database. Before this migration the staging
-- Worker's test approval (goal_cup_upload_approve_device, behind TEST_APPROVAL_KEY) could approve
-- a device request as ANY member, and a staging run could replace a member's production run.
--
--   * The deployment of a caller is decided by the database, from the PostgREST JWT role: the
--     staging Worker's key carries role goal_cup_ingest_staging (goal_cup_tracer.caller_is_staging()).
--     The Worker cannot claim it; any other caller (service_role, a later production role, psql) is
--     production.
--   * Device requests and run credentials are tagged with the deployment that created them; a
--     credential is only ever honoured by a Worker of the same deployment, and a device request is
--     only picked up there.
--   * Runs carry `staging` (copied from their event). A Worker can open runs only in events of its
--     own deployment, can only see or change runs of its own deployment, and a competition start
--     replaces only runs of its own deployment. One open main run per member per deployment.
--   * The test approval refuses unless the caller is the staging Worker, the request is a staging
--     request, and the member is in goal_cup_tracer.test_members (synthetic ids, not accounts).
--     It is granted to goal_cup_ingest_staging only, never to service_role or a production role.
--
-- Additive for everything that existed before today. It alters only the goal_cup_tracer tables
-- created by 20261015120000 the same day, whose rows were all staging smoke-test rows (backfilled
-- below as staging).

-- ---- the caller's deployment -------------------------------------------------------------------
CREATE FUNCTION goal_cup_tracer.caller_is_staging() RETURNS boolean
LANGUAGE sql STABLE SET search_path = pg_catalog, pg_temp
AS $$
  SELECT coalesce(nullif(current_setting('request.jwt.claims', true), '')::jsonb ->> 'role', '') = 'goal_cup_ingest_staging'
$$;

-- ---- tags ----------------------------------------------------------------------------------------
-- Every row present when this runs was made by the staging Worker's smoke test: fill as staging,
-- then default to production.
ALTER TABLE goal_cup_tracer.device_requests ADD COLUMN staging boolean NOT NULL DEFAULT true;
ALTER TABLE goal_cup_tracer.device_requests ALTER COLUMN staging DROP DEFAULT;
ALTER TABLE goal_cup_tracer.run_credentials ADD COLUMN staging boolean NOT NULL DEFAULT true;
ALTER TABLE goal_cup_tracer.run_credentials ALTER COLUMN staging DROP DEFAULT;
ALTER TABLE goal_cup_tracer.runs ADD COLUMN staging boolean NOT NULL DEFAULT true;
ALTER TABLE goal_cup_tracer.runs ALTER COLUMN staging DROP DEFAULT;

DROP INDEX goal_cup_tracer.goal_cup_tracer_runs_one_main_per_member;
CREATE UNIQUE INDEX goal_cup_tracer_runs_one_main_per_member_deployment ON goal_cup_tracer.runs (member_id, staging)
  WHERE status = 'open' AND mode = 'competition' AND replaced_at IS NULL;

-- The only members the staging test approval may approve as. Synthetic ids: no auth.users row.
CREATE TABLE goal_cup_tracer.test_members (
  member_id  uuid PRIMARY KEY,
  note       text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);
ALTER TABLE goal_cup_tracer.test_members ENABLE ROW LEVEL SECURITY;
REVOKE ALL ON goal_cup_tracer.test_members FROM PUBLIC, anon, authenticated;
INSERT INTO goal_cup_tracer.test_members (member_id, note) VALUES
  ('00000000-0000-4000-8000-0000000c0de1', 'staging smoke test member 1 (synthetic, no account)'),
  ('00000000-0000-4000-8000-0000000c0de2', 'staging smoke test member 2 (synthetic, no account)');

-- ---- the device flow and credentials -----------------------------------------------------------
CREATE OR REPLACE FUNCTION public.goal_cup_upload_create_device(
  p_device_hash text, p_user_hash text, p_purpose text, p_mode text,
  p_connector_id text, p_tracer_version text, p_created_at text, p_expires_at text
) RETURNS text
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
BEGIN
  INSERT INTO device_requests (device_hash, user_hash, purpose, mode, connector_id, tracer_version, created_at, expires_at, staging)
  VALUES (p_device_hash, p_user_hash, p_purpose, p_mode, p_connector_id, p_tracer_version,
          p_created_at::timestamptz, p_expires_at::timestamptz, caller_is_staging());
  RETURN 'created';
END $$;

CREATE OR REPLACE FUNCTION public.goal_cup_upload_poll_device(p_device_hash text, p_now text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_now     timestamptz := p_now::timestamptz;
  v_dev     device_requests;
  v_plain   text;
  v_expires timestamptz;
BEGIN
  SELECT * INTO v_dev FROM device_requests
   WHERE device_hash = p_device_hash AND staging = caller_is_staging() FOR UPDATE;
  IF NOT FOUND OR v_now >= v_dev.expires_at OR v_dev.picked_up_at IS NOT NULL THEN
    RETURN jsonb_build_object('result', 'expired');
  END IF;
  IF v_dev.status = 'pending' THEN RETURN jsonb_build_object('result', 'pending'); END IF;
  IF v_dev.status = 'denied' THEN RETURN jsonb_build_object('result', 'denied'); END IF;

  UPDATE device_requests SET picked_up_at = v_now WHERE device_hash = p_device_hash;
  IF v_dev.purpose = 'login' THEN
    RETURN jsonb_build_object('result', 'approved', 'purpose', 'login', 'handle', member_handle(v_dev.member_id));
  END IF;

  v_plain := 'gcr_' || replace(gen_random_uuid()::text || gen_random_uuid()::text, '-', '');
  v_expires := v_dev.decided_at + interval '1 hour';
  INSERT INTO run_credentials (credential_hash, member_id, mode, expires_at, staging)
  VALUES (encode(sha256(convert_to(v_plain, 'UTF8')), 'hex'), v_dev.member_id, v_dev.mode, v_expires, v_dev.staging);
  RETURN jsonb_build_object('result', 'approved', 'purpose', 'run', 'credential', v_plain,
                            'expires_at', fmt_time(v_expires));
END $$;

CREATE OR REPLACE FUNCTION public.goal_cup_upload_credential(p_credential_hash text, p_now text) RETURNS jsonb
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
  SELECT jsonb_build_object('member_id', c.member_id::text, 'run_id', c.run_id, 'mode', c.mode,
                            'expires_at', fmt_time(c.expires_at))
    FROM run_credentials c
   WHERE c.credential_hash = p_credential_hash AND p_now::timestamptz < c.expires_at
     AND c.staging = caller_is_staging()
$$;

-- STAGING ONLY. 'forbidden' unless the caller is the staging Worker, the request was made on
-- staging, and the member is a synthetic test member.
CREATE OR REPLACE FUNCTION public.goal_cup_upload_approve_device(p_user_code text, p_member_id text, p_approve boolean, p_now text) RETURNS text
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
BEGIN
  IF NOT caller_is_staging() THEN RETURN 'forbidden'; END IF;
  IF p_member_id IS NULL OR p_member_id !~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
     OR NOT EXISTS (SELECT 1 FROM test_members WHERE member_id = p_member_id::uuid) THEN
    RETURN 'forbidden';
  END IF;
  IF NOT EXISTS (SELECT 1 FROM device_requests
                  WHERE user_hash = encode(sha256(convert_to(upper(replace(p_user_code, '-', '')), 'UTF8')), 'hex')
                    AND staging) THEN
    RETURN 'not_found';
  END IF;
  RETURN decide_device(p_user_code, p_member_id::uuid, p_approve, p_now::timestamptz);
END $$;

-- ---- runs ------------------------------------------------------------------------------------------
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
  v_bound   text;
  v_try     integer;
BEGIN
  SELECT * INTO v_event FROM events WHERE id = p_event_id AND staging = v_staging;
  IF NOT FOUND THEN
    RETURN jsonb_build_object('result', 'forbidden', 'message', 'This event is not open for runs.');
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

  SELECT run_id INTO v_bound FROM run_credentials
   WHERE credential_hash = p_credential_hash AND staging = v_staging FOR UPDATE;
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
    IF p_mode = 'competition' THEN
      UPDATE runs SET replaced_at = greatest(v_start, official_start)
       WHERE member_id = v_member AND mode = 'competition' AND status = 'open' AND replaced_at IS NULL
         AND staging = v_staging;
    END IF;
    BEGIN
      INSERT INTO runs (id, event_id, member_id, mode, official_start, deadline_at, repo_url,
                        machine, client_ip, client_country, staging)
      VALUES (p_run_id, p_event_id, v_member, p_mode, v_start, p_deadline_at::timestamptz, p_repo_url,
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
     SET run_id = p_run_id, expires_at = p_credential_expires_at::timestamptz
   WHERE credential_hash = p_credential_hash;
  RETURN jsonb_build_object('result', 'created') || run_json(p_run_id);
END $$;

-- ---- the website: a staging request never says it replaces a production run, and vice versa ------
CREATE OR REPLACE FUNCTION public.goal_cup_device_request(p_actor_id uuid, p_user_code text) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_dev device_requests;
BEGIN
  SELECT * INTO v_dev FROM device_requests
   WHERE user_hash = encode(sha256(convert_to(upper(replace(coalesce(p_user_code, ''), '-', '')), 'UTF8')), 'hex');
  IF NOT FOUND THEN RETURN jsonb_build_object('result', 'not_found'); END IF;
  RETURN jsonb_build_object(
    'result', 'ok',
    'status', CASE WHEN v_dev.status = 'pending' AND now() >= v_dev.expires_at THEN 'expired' ELSE v_dev.status END,
    'purpose', v_dev.purpose,
    'mode', v_dev.mode,
    'connector_id', v_dev.connector_id,
    'tracer_version', v_dev.tracer_version,
    'expires_at', fmt_time(v_dev.expires_at),
    'replaces_open_run', v_dev.purpose = 'run' AND v_dev.mode = 'competition' AND EXISTS (
      SELECT 1 FROM runs r WHERE r.member_id = p_actor_id AND r.mode = 'competition' AND r.status = 'open'
         AND r.replaced_at IS NULL AND r.staging = v_dev.staging AND now() < r.deadline_at),
    'terms_ok', has_current_terms(p_actor_id),
    'terms_version', current_terms_version(),
    'decided_by_you', v_dev.member_id IS NOT DISTINCT FROM p_actor_id AND v_dev.status <> 'pending');
END $$;

-- ---- every other Worker function: only runs of the caller's own deployment ----------------------
CREATE OR REPLACE FUNCTION public.goal_cup_upload_continue_run(
  p_member_id text, p_run_id text, p_previous_entry_id text, p_entry_id text,
  p_connector_id text, p_manifest_sha256 text, p_connector_dir_sha256 text, p_workspace text,
  p_session_id text, p_harness_version text, p_tracer_version text,
  p_client_ip text, p_client_country text, p_at text
) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_run runs;
  v_cur entries;
  v_at  timestamptz := p_at::timestamptz;
BEGIN
  SELECT * INTO v_run FROM runs WHERE id = p_run_id AND member_id = p_member_id::uuid AND staging = caller_is_staging() FOR UPDATE;
  IF NOT FOUND THEN RETURN jsonb_build_object('result', 'not_found'); END IF;
  SELECT * INTO v_cur FROM entries WHERE run_id = p_run_id ORDER BY segment DESC LIMIT 1;

  IF v_cur.segment > 0 AND v_cur.session_id = p_session_id
     AND EXISTS (SELECT 1 FROM entries
                  WHERE run_id = p_run_id AND segment = v_cur.segment - 1 AND id = p_previous_entry_id) THEN
    RETURN jsonb_build_object('result', 'existing') || run_json(p_run_id);
  END IF;
  IF v_run.status <> 'open' OR v_at >= least(v_run.deadline_at, coalesce(v_run.replaced_at, v_run.deadline_at)) THEN
    RETURN jsonb_build_object('result', 'closed');
  END IF;
  IF v_cur.id <> p_previous_entry_id THEN
    RETURN jsonb_build_object('result', 'conflict');
  END IF;

  INSERT INTO entries (id, run_id, segment, connector_id, manifest_sha256, connector_dir_sha256,
                       workspace, session_id, harness_version, tracer_version)
  VALUES (p_entry_id, p_run_id, v_cur.segment + 1, p_connector_id, p_manifest_sha256, p_connector_dir_sha256,
          p_workspace, p_session_id, p_harness_version, p_tracer_version);
  UPDATE runs SET last_client_ip = coalesce(p_client_ip, last_client_ip),
                  last_client_country = coalesce(p_client_country, last_client_country)
   WHERE id = p_run_id;
  RETURN jsonb_build_object('result', 'created') || run_json(p_run_id);
END $$;

CREATE OR REPLACE FUNCTION public.goal_cup_upload_issue_receipt(p_member_id text, p_run_id text, p_entry_id text, p_chain_head text, p_receipt jsonb) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_run   runs;
  v_entry entries;
BEGIN
  SELECT * INTO v_run FROM runs WHERE id = p_run_id AND member_id = p_member_id::uuid AND staging = caller_is_staging() FOR UPDATE;
  IF NOT FOUND THEN RETURN jsonb_build_object('result', 'not_found'); END IF;
  SELECT * INTO v_entry FROM entries WHERE id = p_entry_id AND run_id = p_run_id FOR UPDATE;
  IF NOT FOUND THEN RETURN jsonb_build_object('result', 'not_found'); END IF;
  IF v_run.status IN ('abandoned', 'incomplete') OR v_entry.sealed_at IS NOT NULL THEN
    RETURN jsonb_build_object('result', 'closed');
  END IF;
  IF v_entry.receipt IS NOT NULL THEN
    IF v_entry.receipt -> 'claims' ->> 'chain_head' = p_chain_head THEN
      RETURN jsonb_build_object('result', 'existing', 'receipt', v_entry.receipt);
    END IF;
    RETURN jsonb_build_object('result', 'conflict');
  END IF;
  UPDATE entries SET receipt = p_receipt WHERE id = p_entry_id;
  RETURN jsonb_build_object('result', 'stored', 'receipt', p_receipt);
END $$;

CREATE OR REPLACE FUNCTION public.goal_cup_upload_abandon_run(p_member_id text, p_run_id text, p_reason text, p_at text) RETURNS text
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_run runs;
BEGIN
  SELECT * INTO v_run FROM runs WHERE id = p_run_id AND member_id = p_member_id::uuid AND staging = caller_is_staging() FOR UPDATE;
  IF NOT FOUND THEN RETURN 'not_found'; END IF;
  IF v_run.status <> 'open' THEN RETURN 'closed'; END IF;
  IF EXISTS (SELECT 1 FROM chunks c JOIN entries e ON e.id = c.entry_id WHERE e.run_id = p_run_id) THEN
    RETURN 'has_chunks';
  END IF;
  UPDATE runs SET status = 'abandoned', abandon_reason = p_reason, sealed_at = p_at::timestamptz WHERE id = p_run_id;
  RETURN 'abandoned';
END $$;

CREATE OR REPLACE FUNCTION public.goal_cup_upload_commit_chunk(p_member_id text, p_run_id text, p_chunk jsonb) RETURNS text
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_run   runs;
  v_entry entries;
  v_off   bigint := (p_chunk ->> 'offset')::bigint;
  v_len   bigint := (p_chunk ->> 'length')::bigint;
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

  INSERT INTO chunks (entry_id, "offset", length, sha256, first_seq, last_seq, prev, head, r2_key, received_at, ack)
  VALUES (v_entry.id, v_off, v_len, p_chunk ->> 'sha256', (p_chunk ->> 'first_seq')::bigint,
          (p_chunk ->> 'last_seq')::bigint, p_chunk ->> 'prev', p_chunk ->> 'head', p_chunk ->> 'r2_key',
          (p_chunk ->> 'received_at')::timestamptz, p_chunk -> 'ack');

  UPDATE entries
     SET upload_offset = v_off + v_len,
         next_seq      = (p_chunk ->> 'last_seq')::bigint + 1,
         head          = p_chunk ->> 'head',
         last_ack      = p_chunk -> 'ack',
         last_chunk_at = (p_chunk ->> 'received_at')::timestamptz
   WHERE id = v_entry.id;
  RETURN 'committed';
END $$;

CREATE OR REPLACE FUNCTION public.goal_cup_upload_seal_run(
  p_member_id text, p_run_id text, p_entry_id text,
  p_seal_sha256 text, p_baseline_sha256 text, p_seal_key text, p_baseline_key text, p_sealed_at text
) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_run    runs;
  v_entry  entries;
  v_at     timestamptz := p_sealed_at::timestamptz;
  v_status text;
BEGIN
  SELECT * INTO v_run FROM runs WHERE id = p_run_id AND member_id = p_member_id::uuid AND staging = caller_is_staging() FOR UPDATE;
  IF NOT FOUND THEN RETURN jsonb_build_object('result', 'not_found'); END IF;
  SELECT * INTO v_entry FROM entries WHERE id = p_entry_id AND run_id = p_run_id FOR UPDATE;
  IF NOT FOUND THEN RETURN jsonb_build_object('result', 'not_found'); END IF;

  IF v_entry.sealed_at IS NOT NULL THEN
    IF v_entry.seal_sha256 = p_seal_sha256 AND v_entry.baseline_sha256 = p_baseline_sha256 THEN
      RETURN jsonb_build_object('result', 'already_sealed', 'run_status', run_status_at(p_run_id, v_at));
    END IF;
    RETURN jsonb_build_object('result', 'conflict');
  END IF;
  IF v_run.status IN ('abandoned', 'incomplete') THEN RETURN jsonb_build_object('result', 'closed'); END IF;
  IF v_entry.final_seq IS NULL THEN RETURN jsonb_build_object('result', 'no_final'); END IF;
  IF v_entry.next_seq - 1 <> v_entry.final_seq OR v_entry.head <> v_entry.final_head THEN
    RETURN jsonb_build_object('result', 'chain_behind');
  END IF;

  UPDATE entries
     SET seal_sha256 = p_seal_sha256, baseline_sha256 = p_baseline_sha256,
         seal_key = p_seal_key, baseline_key = p_baseline_key, sealed_at = v_at
   WHERE id = p_entry_id;
  v_status := run_status_at(p_run_id, v_at);
  IF v_status <> v_run.status THEN
    UPDATE runs SET status = v_status, sealed_at = CASE WHEN v_status = 'sealed' THEN v_at ELSE sealed_at END
     WHERE id = p_run_id;
  END IF;
  RETURN jsonb_build_object('result', 'sealed', 'run_status', v_status);
END $$;

CREATE OR REPLACE FUNCTION public.goal_cup_upload_heartbeat(
  p_member_id text, p_run_id text, p_entry_id text,
  p_seq bigint, p_head text, p_offset_local bigint, p_final boolean, p_repo_head text,
  p_recording text, p_model_requests bigint, p_client_ip text, p_client_country text, p_at text
) RETURNS text
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_run    runs;
  v_entry  entries;
  v_at     timestamptz := p_at::timestamptz;
  v_stored bigint;
BEGIN
  SELECT * INTO v_run FROM runs WHERE id = p_run_id AND member_id = p_member_id::uuid AND staging = caller_is_staging() FOR UPDATE;
  IF NOT FOUND THEN RETURN 'not_found'; END IF;
  IF v_run.status IN ('abandoned', 'incomplete') THEN RETURN 'closed'; END IF;
  SELECT * INTO v_entry FROM entries WHERE id = p_entry_id AND run_id = p_run_id FOR UPDATE;
  IF NOT FOUND THEN RETURN 'not_found'; END IF;

  IF NOT p_final THEN
    IF v_entry.sealed_at IS NULL THEN
      INSERT INTO heartbeats (run_id, entry_id, received_at, seq, head, offset_local, final,
                              recording, model_requests, client_ip, client_country)
      VALUES (p_run_id, p_entry_id, v_at, p_seq, p_head, p_offset_local, false, p_recording, p_model_requests,
              p_client_ip, p_client_country);
      PERFORM note_heartbeat(p_run_id, p_entry_id, v_at, p_recording, p_model_requests, p_client_ip, p_client_country);
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
                          recording, model_requests, client_ip, client_country)
  VALUES (p_run_id, p_entry_id, v_at, p_seq, p_head, p_offset_local, true, p_repo_head, p_recording, p_model_requests,
          p_client_ip, p_client_country);
  PERFORM note_heartbeat(p_run_id, p_entry_id, v_at, p_recording, p_model_requests, p_client_ip, p_client_country);
  RETURN 'final_committed';
END $$;

CREATE OR REPLACE FUNCTION public.goal_cup_upload_close_run(p_member_id text, p_run_id text, p_at text) RETURNS text
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_run    runs;
  v_at     timestamptz := p_at::timestamptz;
  v_status text;
BEGIN
  SELECT * INTO v_run FROM runs WHERE id = p_run_id AND member_id = p_member_id::uuid AND staging = caller_is_staging() FOR UPDATE;
  IF NOT FOUND THEN RETURN NULL; END IF;
  v_status := run_status_at(p_run_id, v_at);
  IF v_status <> v_run.status THEN
    UPDATE runs
       SET status = v_status,
           sealed_at = CASE WHEN v_status = 'sealed'
                            THEN (SELECT e.sealed_at FROM entries e WHERE e.run_id = p_run_id ORDER BY e.segment DESC LIMIT 1)
                            ELSE sealed_at END
     WHERE id = p_run_id;
  END IF;
  RETURN v_status;
END $$;

CREATE OR REPLACE FUNCTION public.goal_cup_upload_run_position(p_member_id text, p_run_id text, p_entry_id text DEFAULT NULL) RETURNS jsonb
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
  SELECT run_json(r.id, p_entry_id) FROM runs r WHERE r.id = p_run_id AND r.member_id = p_member_id::uuid AND r.staging = caller_is_staging()
$$;

CREATE OR REPLACE FUNCTION public.goal_cup_upload_get_chunk(p_member_id text, p_run_id text, p_entry_id text, p_offset bigint) RETURNS jsonb
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
  SELECT jsonb_build_object(
    'entry_id', c.entry_id, 'offset', c."offset", 'length', c.length, 'sha256', c.sha256,
    'first_seq', c.first_seq, 'last_seq', c.last_seq, 'prev', c.prev, 'head', c.head,
    'r2_key', c.r2_key, 'received_at', fmt_time(c.received_at), 'ack', c.ack)
  FROM chunks c
  JOIN entries e ON e.id = c.entry_id
  JOIN runs r ON r.id = e.run_id
  WHERE r.id = p_run_id AND r.member_id = p_member_id::uuid AND r.staging = caller_is_staging() AND c.entry_id = p_entry_id AND c."offset" = p_offset
$$;

CREATE OR REPLACE FUNCTION public.goal_cup_upload_record_final_commit(p_member_id text, p_run_id text, p_commit text, p_at text) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_rows integer;
BEGIN
  UPDATE runs SET final_commit = p_commit, final_commit_at = p_at::timestamptz
   WHERE id = p_run_id AND member_id = p_member_id::uuid AND staging = caller_is_staging() AND final_commit IS NULL;
  GET DIAGNOSTICS v_rows = ROW_COUNT;
  RETURN v_rows = 1;
END $$;

CREATE OR REPLACE FUNCTION public.goal_cup_upload_runs_for_tick(p_at text) RETURNS jsonb
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
  SELECT coalesce(jsonb_agg(
           (run_json(r.id) -> 'run') || jsonb_build_object(
             'last_heartbeat_at',     fmt_time(r.last_heartbeat_at),
             'last_model_request_at', fmt_time(r.last_model_request_at),
             'flagged_at',            fmt_time(r.flagged_at),
             'flag_reason',           r.flag_reason,
             'final_commit',          r.final_commit)
           ORDER BY r.official_start, r.id), '[]'::jsonb)
    FROM runs r
   WHERE r.status <> 'abandoned' AND r.staging = caller_is_staging()
     AND (r.status = 'open'
          OR least(r.deadline_at, coalesce(r.replaced_at, r.deadline_at)) >= p_at::timestamptz - interval '48 hours')
$$;

CREATE OR REPLACE FUNCTION public.goal_cup_upload_queue_notices(p_notices jsonb) RETURNS integer
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_rows integer;
BEGIN
  INSERT INTO notices (run_id, member_id, kind, dedupe_key, payload, created_at)
  SELECT r.id, r.member_id, n.kind, n.dedupe_key, coalesce(n.payload, '{}'::jsonb), n.at::timestamptz
    FROM jsonb_to_recordset(p_notices) AS n(run_id text, member_id text, kind text, dedupe_key text, payload jsonb, at text)
    JOIN runs r ON r.id = n.run_id AND r.member_id = n.member_id::uuid AND r.staging = caller_is_staging()
  ON CONFLICT (run_id, kind, dedupe_key) DO NOTHING;
  GET DIAGNOSTICS v_rows = ROW_COUNT;
  RETURN v_rows;
END $$;

CREATE OR REPLACE FUNCTION public.goal_cup_upload_claim_notices(p_limit integer, p_now text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_now timestamptz := p_now::timestamptz;
  v_out jsonb := '[]'::jsonb;
  v_n   record;
  v_to  text;
  v_has_user boolean;
BEGIN
  FOR v_n IN
    SELECT n.* FROM notices n JOIN runs r ON r.id = n.run_id AND r.staging = caller_is_staging()
     WHERE n.sent_at IS NULL AND n.skipped_at IS NULL
       AND (n.claimed_at IS NULL OR n.claimed_at < v_now - interval '15 minutes')
       AND n.attempts < 5
     ORDER BY n.created_at, n.id
     LIMIT greatest(least(coalesce(p_limit, 20), 50), 0)
     FOR UPDATE OF n SKIP LOCKED
  LOOP
    SELECT u.email, true INTO v_to, v_has_user FROM auth.users u WHERE u.id = v_n.member_id;
    IF v_has_user IS NULL THEN
      v_to := NULL;
    ELSIF v_to IS NULL OR btrim(v_to) = '' THEN
      UPDATE notices SET skipped_at = v_now, skipped_reason = 'no_address' WHERE id = v_n.id;
      CONTINUE;
    ELSIF EXISTS (SELECT 1 FROM public.email_suppression_list s
                   WHERE lower(s.email) = lower(v_to) AND (s.expires_at IS NULL OR s.expires_at > v_now)) THEN
      UPDATE notices SET skipped_at = v_now, skipped_reason = 'suppressed' WHERE id = v_n.id;
      CONTINUE;
    END IF;
    UPDATE notices SET claimed_at = v_now, attempts = attempts + 1 WHERE id = v_n.id;
    v_out := v_out || jsonb_build_array(
      jsonb_build_object('id', v_n.id, 'kind', v_n.kind, 'run_id', v_n.run_id, 'to', v_to)
      || notice_text(v_n.kind, v_n.payload, v_n.run_id));
    v_has_user := NULL;
  END LOOP;
  RETURN v_out;
END $$;

CREATE OR REPLACE FUNCTION public.goal_cup_upload_finish_notice(p_id bigint, p_outcome text, p_to text, p_detail text, p_now text) RETURNS text
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_now timestamptz := p_now::timestamptz;
BEGIN
  IF p_outcome = 'sent' THEN
    UPDATE notices SET sent_at = v_now, sent_to = left(p_to, 320), provider_id = left(p_detail, 200), last_error = NULL
     WHERE id = p_id AND sent_at IS NULL AND skipped_at IS NULL
       AND run_id IN (SELECT r.id FROM runs r WHERE r.staging = caller_is_staging());
  ELSIF p_outcome = 'skipped' THEN
    UPDATE notices SET skipped_at = v_now, skipped_reason = left(p_detail, 200)
     WHERE id = p_id AND sent_at IS NULL AND skipped_at IS NULL
       AND run_id IN (SELECT r.id FROM runs r WHERE r.staging = caller_is_staging());
  ELSIF p_outcome = 'failed' THEN
    UPDATE notices SET last_error = left(p_detail, 500)
     WHERE id = p_id AND sent_at IS NULL AND skipped_at IS NULL
       AND run_id IN (SELECT r.id FROM runs r WHERE r.staging = caller_is_staging());
  ELSE
    RAISE EXCEPTION 'goal_cup_upload_finish_notice: unknown outcome %', p_outcome USING ERRCODE = '22023';
  END IF;
  IF NOT FOUND THEN RETURN 'not_found'; END IF;
  RETURN 'ok';
END $$;

-- ---- privileges ------------------------------------------------------------------------------------
-- CREATE OR REPLACE keeps each function's ACL; the new helper gets the usual revoke, and the test
-- approval is granted (again: it was revoked by hand at 05:15Z while this fix was written) to the
-- staging role only.
REVOKE ALL ON FUNCTION goal_cup_tracer.caller_is_staging() FROM PUBLIC, anon, authenticated;
REVOKE ALL ON FUNCTION public.goal_cup_upload_approve_device(text, text, boolean, text) FROM PUBLIC, anon, authenticated, service_role;
GRANT EXECUTE ON FUNCTION public.goal_cup_upload_approve_device(text, text, boolean, text) TO goal_cup_ingest_staging;

-- ======================== 20261015140000_goal_cup_tracer_member_binding.sql ========================
-- Goal Cup tracer: bind a run credential to its member; staging never sees an address
-- (review of goal-cup-tracer PR #127, 2026-10-15).
--
--   * goal_cup_upload_open_run: the credential must belong to p_member_id (and to p_mode). Before,
--     a holder of the staging Worker's key could test-approve a synthetic member and then open a
--     staging run under any real member id.
--   * goal_cup_upload_claim_notices: for a staging caller "to" is always null and auth.users is not
--     read, so the database itself never hands the staging Worker a member's email address.
--
-- Replaces two functions created by 20261015130000 (same signatures, so their grants are kept).
-- No table or row changes.

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
  v_bound   text;
  v_try     integer;
BEGIN
  SELECT * INTO v_event FROM events WHERE id = p_event_id AND staging = v_staging;
  IF NOT FOUND THEN
    RETURN jsonb_build_object('result', 'forbidden', 'message', 'This event is not open for runs.');
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
    IF p_mode = 'competition' THEN
      UPDATE runs SET replaced_at = greatest(v_start, official_start)
       WHERE member_id = v_member AND mode = 'competition' AND status = 'open' AND replaced_at IS NULL
         AND staging = v_staging;
    END IF;
    BEGIN
      INSERT INTO runs (id, event_id, member_id, mode, official_start, deadline_at, repo_url,
                        machine, client_ip, client_country, staging)
      VALUES (p_run_id, p_event_id, v_member, p_mode, v_start, p_deadline_at::timestamptz, p_repo_url,
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
     SET run_id = p_run_id, expires_at = p_credential_expires_at::timestamptz
   WHERE credential_hash = p_credential_hash;
  RETURN jsonb_build_object('result', 'created') || run_json(p_run_id);
END $$;

CREATE OR REPLACE FUNCTION public.goal_cup_upload_claim_notices(p_limit integer, p_now text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_now timestamptz := p_now::timestamptz;
  v_out jsonb := '[]'::jsonb;
  v_n   record;
  v_to  text;
  v_has_user boolean;
BEGIN
  FOR v_n IN
    SELECT n.* FROM notices n JOIN runs r ON r.id = n.run_id AND r.staging = caller_is_staging()
     WHERE n.sent_at IS NULL AND n.skipped_at IS NULL
       AND (n.claimed_at IS NULL OR n.claimed_at < v_now - interval '15 minutes')
       AND n.attempts < 5
     ORDER BY n.created_at, n.id
     LIMIT greatest(least(coalesce(p_limit, 20), 50), 0)
     FOR UPDATE OF n SKIP LOCKED
  LOOP
    -- A staging caller never learns a member's address: "to" is always null there, and the
    -- staging Worker sends to its test address (or skips).
    IF caller_is_staging() THEN
      v_to := NULL; v_has_user := NULL;
    ELSE
      SELECT u.email, true INTO v_to, v_has_user FROM auth.users u WHERE u.id = v_n.member_id;
    END IF;
    IF v_has_user IS NULL THEN
      v_to := NULL;
    ELSIF v_to IS NULL OR btrim(v_to) = '' THEN
      UPDATE notices SET skipped_at = v_now, skipped_reason = 'no_address' WHERE id = v_n.id;
      CONTINUE;
    ELSIF EXISTS (SELECT 1 FROM public.email_suppression_list s
                   WHERE lower(s.email) = lower(v_to) AND (s.expires_at IS NULL OR s.expires_at > v_now)) THEN
      UPDATE notices SET skipped_at = v_now, skipped_reason = 'suppressed' WHERE id = v_n.id;
      CONTINUE;
    END IF;
    UPDATE notices SET claimed_at = v_now, attempts = attempts + 1 WHERE id = v_n.id;
    v_out := v_out || jsonb_build_array(
      jsonb_build_object('id', v_n.id, 'kind', v_n.kind, 'run_id', v_n.run_id, 'to', v_to)
      || notice_text(v_n.kind, v_n.payload, v_n.run_id));
    v_has_user := NULL;
  END LOOP;
  RETURN v_out;
END $$;

-- ======================== 20261015150000_goal_cup_tracer_production.sql ========================
-- Goal Cup tracer: production upload Worker role, production events, account erasure.
--
-- Additive. A new role, new grants, three new event rows, one new function. Nothing existing is
-- altered; no existing row is changed.
--
--   * Role goal_cup_ingest_production: the production Worker's Supabase secret API key
--     (goalcup_ingest_production) carries it. goal_cup_tracer.caller_is_staging() is true only for
--     goal_cup_ingest_staging, so this role is production behind the deployment fence. It may
--     execute the Worker's goal_cup_upload_* functions and nothing else in the schema; it is never
--     granted goal_cup_upload_approve_device (the staging test approval).
--   * Production events (staging = false). The times are placeholders until the owner confirms them:
--       goalcup-practice     practice, always open (10-minute runs; Worker PRACTICE_RUN_MINUTES)
--       goalcup-dryrun-2026  dry run, start window 2026-10-28 16:00 UTC + 24 h   [CONFIRM]
--       goalcup-cup-2026     competition, start window 2026-11-06 16:00 UTC + 24 h, 168-hour runs [CONFIRM]
--     Changing a time later is an UPDATE of these rows only (a new migration).
--   * goal_cup_erase_member(p_actor_id): called by the account deletion (DELETE /api/my-data). Deletes
--     the member's contact data and keeps run evidence until the raw traces are deleted after appeals:
--     terms acceptances deleted, unsent run emails deleted, sent ones lose the address, client IP and
--     country removed from runs and heartbeats, run credentials expired. Runs, entries, chunks,
--     heartbeats (seq, head, times) and the R2 objects stay. Idempotent; service_role only.

DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'goal_cup_ingest_production') THEN
    CREATE ROLE goal_cup_ingest_production NOLOGIN NOINHERIT;
  END IF;
END $$;
ALTER ROLE goal_cup_ingest_production SET statement_timeout = '8s';
GRANT goal_cup_ingest_production TO authenticator;

GRANT EXECUTE ON FUNCTION
  public.goal_cup_upload_credential(text, text),
  public.goal_cup_upload_create_device(text, text, text, text, text, text, text, text),
  public.goal_cup_upload_poll_device(text, text),
  public.goal_cup_upload_open_run(text, text, text, text, text, text, text, text, text, text, text, text, text, text, text, jsonb, text, text, text, text),
  public.goal_cup_upload_continue_run(text, text, text, text, text, text, text, text, text, text, text, text, text, text),
  public.goal_cup_upload_run_position(text, text, text),
  public.goal_cup_upload_get_chunk(text, text, text, bigint),
  public.goal_cup_upload_issue_receipt(text, text, text, text, jsonb),
  public.goal_cup_upload_abandon_run(text, text, text, text),
  public.goal_cup_upload_commit_chunk(text, text, jsonb),
  public.goal_cup_upload_seal_run(text, text, text, text, text, text, text, text),
  public.goal_cup_upload_heartbeat(text, text, text, bigint, text, bigint, boolean, text, text, bigint, text, text, text),
  public.goal_cup_upload_close_run(text, text, text),
  public.goal_cup_upload_runs_for_tick(text),
  public.goal_cup_upload_queue_notices(jsonb),
  public.goal_cup_upload_record_final_commit(text, text, text, text),
  public.goal_cup_upload_claim_notices(integer, text),
  public.goal_cup_upload_finish_notice(bigint, text, text, text, text)
TO goal_cup_ingest_production;

INSERT INTO goal_cup_tracer.events (id, mode, start_window_opens, start_window_closes, run_hours, staging)
VALUES
  ('goalcup-practice',    'practice',    '2026-10-15T00:00:00Z', '2027-06-30T00:00:00Z', 168, false),
  ('goalcup-dryrun-2026', 'dry_run',     '2026-10-28T16:00:00Z', '2026-10-29T16:00:00Z', 168, false),  -- [CONFIRM]
  ('goalcup-cup-2026',    'competition', '2026-11-06T16:00:00Z', '2026-11-07T16:00:00Z', 168, false)   -- [CONFIRM]
ON CONFLICT (id) DO NOTHING;

CREATE FUNCTION public.goal_cup_erase_member(p_actor_id uuid) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_consents   integer;
  v_unsent     integer;
  v_sent       integer;
  v_runs       integer;
  v_heartbeats integer;
  v_creds      integer;
BEGIN
  IF p_actor_id IS NULL THEN RAISE EXCEPTION 'goal_cup_erase_member: no member' USING ERRCODE = '22023'; END IF;
  DELETE FROM consents WHERE member_id = p_actor_id;
  GET DIAGNOSTICS v_consents = ROW_COUNT;
  DELETE FROM notices WHERE member_id = p_actor_id AND sent_at IS NULL;
  GET DIAGNOSTICS v_unsent = ROW_COUNT;
  UPDATE notices SET sent_to = NULL WHERE member_id = p_actor_id AND sent_to IS NOT NULL;
  GET DIAGNOSTICS v_sent = ROW_COUNT;
  UPDATE runs SET client_ip = NULL, client_country = NULL, last_client_ip = NULL, last_client_country = NULL
   WHERE member_id = p_actor_id
     AND (client_ip IS NOT NULL OR client_country IS NOT NULL OR last_client_ip IS NOT NULL OR last_client_country IS NOT NULL);
  GET DIAGNOSTICS v_runs = ROW_COUNT;
  UPDATE heartbeats h SET client_ip = NULL, client_country = NULL
    FROM runs r
   WHERE h.run_id = r.id AND r.member_id = p_actor_id AND (h.client_ip IS NOT NULL OR h.client_country IS NOT NULL);
  GET DIAGNOSTICS v_heartbeats = ROW_COUNT;
  UPDATE run_credentials SET expires_at = now() WHERE member_id = p_actor_id AND expires_at > now();
  GET DIAGNOSTICS v_creds = ROW_COUNT;
  RETURN jsonb_build_object('consents', v_consents, 'unsent_notices', v_unsent, 'sent_notices', v_sent,
                            'runs', v_runs, 'heartbeats', v_heartbeats, 'credentials', v_creds);
END $$;
REVOKE ALL ON FUNCTION public.goal_cup_erase_member(uuid) FROM PUBLIC, anon, authenticated;
GRANT EXECUTE ON FUNCTION public.goal_cup_erase_member(uuid) TO service_role;

-- ======================== 20261015160000_goal_cup_tracer_test_mode.sql ========================
-- Goal Cup tracer: the TEST run mode (owner request, 2026-10-15).
--
-- Organizers' own test runs, easy to tell apart and to delete in one go.
--   * mode 'test' for events, device requests, credentials and runs (wire value: "mode":"test" in
--     POST /v1/device). Events 'test' (production, R2 prefix traces/test/, no bucket lock) and
--     'test-staging'; both always open, run_hours 168 (a test run lasts its event's run_hours).
--   * Only members in goal_cup_tracer.test_operators may approve or open a test run (enforced here:
--     goal_cup_approve_device answers 'not_test_operator', the staging test approval 'forbidden',
--     open_run 'forbidden'). The table starts empty; the owner adds organizers.
--   * Test runs never replace and are never replaced (only competition runs replace), and their
--     emails go to the operator's own address with "[TEST] " before the subject.
--   * goal_cup_tracer.delete_test_runs(older_than) deletes every test-mode row (runs, entries, chunks,
--     heartbeats, notices, credentials, device requests) and returns the R2 keys to delete;
--     service_role only. goal-cup-tracer server/worker/scripts/delete-test-runs.sh wraps it.
--   * Aggregates (live numbers, research, reports) must leave out runs with mode = 'test'.
--
-- Widens four mode CHECK constraints on goal_cup_tracer tables created today (20261015120000);
-- nothing outside goal_cup_tracer changes and no row is changed.

ALTER TABLE goal_cup_tracer.events DROP CONSTRAINT events_mode_check;
ALTER TABLE goal_cup_tracer.events ADD CONSTRAINT events_mode_check CHECK (mode IN ('competition', 'dry_run', 'practice', 'test'));
ALTER TABLE goal_cup_tracer.device_requests DROP CONSTRAINT device_requests_mode_check;
ALTER TABLE goal_cup_tracer.device_requests ADD CONSTRAINT device_requests_mode_check CHECK (mode IN ('competition', 'practice', 'test'));
ALTER TABLE goal_cup_tracer.run_credentials DROP CONSTRAINT run_credentials_mode_check;
ALTER TABLE goal_cup_tracer.run_credentials ADD CONSTRAINT run_credentials_mode_check CHECK (mode IN ('competition', 'practice', 'test'));
ALTER TABLE goal_cup_tracer.runs DROP CONSTRAINT runs_mode_check;
ALTER TABLE goal_cup_tracer.runs ADD CONSTRAINT runs_mode_check CHECK (mode IN ('competition', 'practice', 'test'));

CREATE TABLE goal_cup_tracer.test_operators (
  member_id  uuid PRIMARY KEY,
  note       text NOT NULL DEFAULT '',
  created_at timestamptz NOT NULL DEFAULT now()
);
ALTER TABLE goal_cup_tracer.test_operators ENABLE ROW LEVEL SECURITY;
REVOKE ALL ON goal_cup_tracer.test_operators FROM PUBLIC, anon, authenticated;

INSERT INTO goal_cup_tracer.events (id, mode, start_window_opens, start_window_closes, run_hours, staging)
VALUES
  ('test',         'test', '2026-10-15T00:00:00Z', '2027-06-30T00:00:00Z', 168, false),
  ('test-staging', 'test', '2026-10-15T00:00:00Z', '2027-06-30T00:00:00Z', 168, true)
ON CONFLICT (id) DO NOTHING;

CREATE FUNCTION goal_cup_tracer.is_test_operator(p_member uuid) RETURNS boolean
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$ SELECT EXISTS (SELECT 1 FROM goal_cup_tracer.test_operators WHERE member_id = p_member) $$;

-- {subject, text} with "[TEST] " before the subject when the run is a test run.
CREATE FUNCTION goal_cup_tracer.test_marked(p_email jsonb, p_run_id text) RETURNS jsonb
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
  SELECT CASE WHEN EXISTS (SELECT 1 FROM goal_cup_tracer.runs WHERE id = p_run_id AND mode = 'test')
              THEN jsonb_set(p_email, '{subject}', to_jsonb('[TEST] ' || (p_email ->> 'subject')))
              ELSE p_email END
$$;

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
    IF p_mode = 'competition' THEN
      UPDATE runs SET replaced_at = greatest(v_start, official_start)
       WHERE member_id = v_member AND mode = 'competition' AND status = 'open' AND replaced_at IS NULL
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

CREATE OR REPLACE FUNCTION public.goal_cup_upload_claim_notices(p_limit integer, p_now text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_now timestamptz := p_now::timestamptz;
  v_out jsonb := '[]'::jsonb;
  v_n   record;
  v_to  text;
  v_has_user boolean;
BEGIN
  FOR v_n IN
    SELECT n.* FROM notices n JOIN runs r ON r.id = n.run_id AND r.staging = caller_is_staging()
     WHERE n.sent_at IS NULL AND n.skipped_at IS NULL
       AND (n.claimed_at IS NULL OR n.claimed_at < v_now - interval '15 minutes')
       AND n.attempts < 5
     ORDER BY n.created_at, n.id
     LIMIT greatest(least(coalesce(p_limit, 20), 50), 0)
     FOR UPDATE OF n SKIP LOCKED
  LOOP
    -- A staging caller never learns a member's address: "to" is always null there, and the
    -- staging Worker sends to its test address (or skips).
    IF caller_is_staging() THEN
      v_to := NULL; v_has_user := NULL;
    ELSE
      SELECT u.email, true INTO v_to, v_has_user FROM auth.users u WHERE u.id = v_n.member_id;
    END IF;
    IF v_has_user IS NULL THEN
      v_to := NULL;
    ELSIF v_to IS NULL OR btrim(v_to) = '' THEN
      UPDATE notices SET skipped_at = v_now, skipped_reason = 'no_address' WHERE id = v_n.id;
      CONTINUE;
    ELSIF EXISTS (SELECT 1 FROM public.email_suppression_list s
                   WHERE lower(s.email) = lower(v_to) AND (s.expires_at IS NULL OR s.expires_at > v_now)) THEN
      UPDATE notices SET skipped_at = v_now, skipped_reason = 'suppressed' WHERE id = v_n.id;
      CONTINUE;
    END IF;
    UPDATE notices SET claimed_at = v_now, attempts = attempts + 1 WHERE id = v_n.id;
    v_out := v_out || jsonb_build_array(
      jsonb_build_object('id', v_n.id, 'kind', v_n.kind, 'run_id', v_n.run_id, 'to', v_to)
      || test_marked(notice_text(v_n.kind, v_n.payload, v_n.run_id), v_n.run_id));
    v_has_user := NULL;
  END LOOP;
  RETURN v_out;
END $$;

CREATE OR REPLACE FUNCTION public.goal_cup_approve_device(p_actor_id uuid, p_user_code text, p_approve boolean) RETURNS text
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
BEGIN
  IF p_approve THEN
    IF NOT EXISTS (SELECT 1 FROM auth.users u WHERE u.id = p_actor_id AND u.email_confirmed_at IS NOT NULL) THEN
      RETURN 'not_a_member';
    END IF;
    IF NOT has_current_terms(p_actor_id) THEN RETURN 'terms_required'; END IF;
    IF EXISTS (SELECT 1 FROM device_requests
                WHERE user_hash = encode(sha256(convert_to(upper(replace(coalesce(p_user_code, ''), '-', '')), 'UTF8')), 'hex')
                  AND mode = 'test')
       AND NOT is_test_operator(p_actor_id) THEN
      RETURN 'not_test_operator';
    END IF;
  END IF;
  RETURN decide_device(p_user_code, p_actor_id, coalesce(p_approve, false), now());
END $$;

CREATE OR REPLACE FUNCTION public.goal_cup_upload_approve_device(p_user_code text, p_member_id text, p_approve boolean, p_now text) RETURNS text
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
BEGIN
  IF NOT caller_is_staging() THEN RETURN 'forbidden'; END IF;
  IF p_member_id IS NULL OR p_member_id !~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
     OR NOT EXISTS (SELECT 1 FROM test_members WHERE member_id = p_member_id::uuid) THEN
    RETURN 'forbidden';
  END IF;
  IF NOT EXISTS (SELECT 1 FROM device_requests
                  WHERE user_hash = encode(sha256(convert_to(upper(replace(p_user_code, '-', '')), 'UTF8')), 'hex')
                    AND staging) THEN
    RETURN 'not_found';
  END IF;
  IF EXISTS (SELECT 1 FROM device_requests
              WHERE user_hash = encode(sha256(convert_to(upper(replace(p_user_code, '-', '')), 'UTF8')), 'hex')
                AND mode = 'test')
     AND NOT is_test_operator(p_member_id::uuid) THEN
    RETURN 'forbidden';
  END IF;
  RETURN decide_device(p_user_code, p_member_id::uuid, p_approve, p_now::timestamptz);
END $$;

-- One-step cleanup of every test-mode row older than older_than (default: all of them). Returns
-- {"runs": [...], "r2_keys": [...], "counts": {...}}: the R2 objects of the deleted runs, all under
-- traces/<test event>/, for the caller to delete. A dry run is the same call inside a transaction
-- that is rolled back. service_role (and the owner) only.
CREATE FUNCTION goal_cup_tracer.delete_test_runs(older_than interval DEFAULT '0') RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_cut   timestamptz := now() - coalesce(older_than, interval '0');
  v_runs  text[];
  v_keys  jsonb;
  v_n     jsonb := '{}'::jsonb;
  v_rows  integer;
BEGIN
  SELECT coalesce(array_agg(id ORDER BY id), '{}') INTO v_runs FROM runs WHERE mode = 'test' AND created_at <= v_cut;
  SELECT coalesce(jsonb_agg(k ORDER BY k), '[]'::jsonb) INTO v_keys FROM (
    SELECT c.r2_key AS k FROM chunks c JOIN entries e ON e.id = c.entry_id WHERE e.run_id = ANY (v_runs)
    UNION SELECT e.seal_key FROM entries e WHERE e.run_id = ANY (v_runs) AND e.seal_key IS NOT NULL
    UNION SELECT e.baseline_key FROM entries e WHERE e.run_id = ANY (v_runs) AND e.baseline_key IS NOT NULL) keys;
  DELETE FROM chunks c USING entries e WHERE e.id = c.entry_id AND e.run_id = ANY (v_runs);
  GET DIAGNOSTICS v_rows = ROW_COUNT; v_n := v_n || jsonb_build_object('chunks', v_rows);
  DELETE FROM heartbeats WHERE run_id = ANY (v_runs);
  GET DIAGNOSTICS v_rows = ROW_COUNT; v_n := v_n || jsonb_build_object('heartbeats', v_rows);
  DELETE FROM notices WHERE run_id = ANY (v_runs);
  GET DIAGNOSTICS v_rows = ROW_COUNT; v_n := v_n || jsonb_build_object('notices', v_rows);
  DELETE FROM entries WHERE run_id = ANY (v_runs);
  GET DIAGNOSTICS v_rows = ROW_COUNT; v_n := v_n || jsonb_build_object('entries', v_rows);
  DELETE FROM run_credentials WHERE mode = 'test' AND (run_id = ANY (v_runs) OR (run_id IS NULL AND created_at <= v_cut));
  GET DIAGNOSTICS v_rows = ROW_COUNT; v_n := v_n || jsonb_build_object('credentials', v_rows);
  DELETE FROM runs WHERE id = ANY (v_runs);
  GET DIAGNOSTICS v_rows = ROW_COUNT; v_n := v_n || jsonb_build_object('runs', v_rows);
  DELETE FROM device_requests WHERE mode = 'test' AND created_at <= v_cut;
  GET DIAGNOSTICS v_rows = ROW_COUNT; v_n := v_n || jsonb_build_object('device_requests', v_rows);
  RETURN jsonb_build_object('runs', to_jsonb(v_runs), 'r2_keys', v_keys, 'counts', v_n);
END $$;

REVOKE ALL ON FUNCTION goal_cup_tracer.is_test_operator(uuid) FROM PUBLIC, anon, authenticated;
REVOKE ALL ON FUNCTION goal_cup_tracer.test_marked(jsonb, text) FROM PUBLIC, anon, authenticated;
REVOKE ALL ON FUNCTION goal_cup_tracer.delete_test_runs(interval) FROM PUBLIC, anon, authenticated;
GRANT USAGE ON SCHEMA goal_cup_tracer TO service_role;
GRANT EXECUTE ON FUNCTION goal_cup_tracer.delete_test_runs(interval) TO service_role;

-- ======================== 20261015170000_goal_cup_tracer_test_replaces_test.sql ========================
-- Goal Cup tracer: a test run replaces the operator's previous open test run (owner, 2026-10-15:
-- "a test run never replaces a competition run; the same mode replaces the same mode").
--
-- 20261015160000 made test runs never replace anything. Now the rule is one rule for both modes
-- that replace: a new competition run ends the member's open competition run (unchanged), a new
-- test run ends the member's open test run. A test run still never touches a competition run, and
-- practice still never replaces. The approval page's "this replaces your current run" follows.
-- Replaces two functions (CREATE OR REPLACE keeps their grants); no table, row or grant changes.

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
    -- The same mode replaces the same mode: a competition run its competition run, a test run its
    -- test run. A test run never touches a competition run, and practice never replaces.
    IF p_mode IN ('competition', 'test') THEN
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
  v_dev device_requests;
BEGIN
  SELECT * INTO v_dev FROM device_requests
   WHERE user_hash = encode(sha256(convert_to(upper(replace(coalesce(p_user_code, ''), '-', '')), 'UTF8')), 'hex');
  IF NOT FOUND THEN RETURN jsonb_build_object('result', 'not_found'); END IF;
  RETURN jsonb_build_object(
    'result', 'ok',
    'status', CASE WHEN v_dev.status = 'pending' AND now() >= v_dev.expires_at THEN 'expired' ELSE v_dev.status END,
    'purpose', v_dev.purpose,
    'mode', v_dev.mode,
    'connector_id', v_dev.connector_id,
    'tracer_version', v_dev.tracer_version,
    'expires_at', fmt_time(v_dev.expires_at),
    'replaces_open_run', v_dev.purpose = 'run' AND v_dev.mode IN ('competition', 'test') AND EXISTS (
      SELECT 1 FROM runs r WHERE r.member_id = p_actor_id AND r.mode = v_dev.mode AND r.status = 'open'
         AND r.replaced_at IS NULL AND r.staging = v_dev.staging AND now() < r.deadline_at),
    'terms_ok', has_current_terms(p_actor_id),
    'terms_version', current_terms_version(),
    'decided_by_you', v_dev.member_id IS NOT DISTINCT FROM p_actor_id AND v_dev.status <> 'pending');
END $$;

-- ======================== 20261016120000_goal_cup_tracer_event_windows.sql ========================
-- Goal Cup tracer: the confirmed start windows of the dry run and the cup (owner, 2026-10-16).
--
-- 20261015150000_goal_cup_tracer_production.sql inserted both rows with placeholder times
-- ([CONFIRM]): the cup opened 2026-11-06 16:00 UTC and closed 2026-11-07 16:00 UTC, 16 hours early,
-- so goal_cup_upload_open_run would have refused every real start after 2026-11-07 16:00 UTC.
--
--   * goalcup-cup-2026     competition, start window 2026-11-07 00:00 UTC to 2026-11-08 00:00 UTC (24 h)
--   * goalcup-dryrun-2026  dry run, open from the first release (2026-10-23 00:00 UTC) to
--                          2026-11-06 00:00 UTC, so a participant can start a dry run any day before
--                          the Worker's EVENT_ID moves to the cup with the last release (6 Nov)
--
-- Additive: two UPDATEs of the production rows only. No staging row, no other column, no other row.

UPDATE goal_cup_tracer.events
   SET start_window_opens  = '2026-11-07T00:00:00Z',
       start_window_closes = '2026-11-08T00:00:00Z'
 WHERE id = 'goalcup-cup-2026' AND mode = 'competition' AND staging = false;

UPDATE goal_cup_tracer.events
   SET start_window_opens  = '2026-10-23T00:00:00Z',
       start_window_closes = '2026-11-06T00:00:00Z'
 WHERE id = 'goalcup-dryrun-2026' AND mode = 'dry_run' AND staging = false;
