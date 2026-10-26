-- COPY, for reference. The authority is the website repo (agentCommunity_PAGE), supabase/migrations/.
-- This file is every *goal_cup*.sql migration there, in name order, one after the other, each
-- under a "-- ===== <file name> =====" line. Do not edit it by hand. After a website migration merges,
-- run this from the root of the website checkout (at its main), with F set to this file, to rewrite it
-- (the header above the REGEN-END line is kept, the rest is regenerated):
--   F=<tracer>/server/worker/sql/goal-cup-tracer-upload.sql; { sed '/^-- REGEN-END/q' "$F"; for m in supabase/migrations/*goal_cup*.sql; do printf '\n-- ======================== %s ========================\n' "${m##*/}"; cat "$m"; done; } > "$F.new" && mv "$F.new" "$F"
-- Change a migration there first, then regenerate here.
-- Scope (widened 2026-10-27 from *goal_cup_tracer*.sql): the sign-up migrations (20261021120000 to
-- 20261021120400) and 20261017150200 replace goal_cup_upload_open_run and goal_cup_upload_claim_notices,
-- so a mirror without them held bodies production no longer has. One website migration outside the
-- glob also replaces a mirrored function: 20261017110000_email_delivery_rules.sql replaces
-- goal_cup_upload_claim_notices, and 20261017150200 replaces it again. Check that no other one does,
-- from the website root:
--   grep -lE 'FUNCTION +(public|goal_cup_tracer)\.' supabase/migrations/*.sql | xargs grep -lE 'FUNCTION +(public\.goal_cup|goal_cup_tracer\.)' | grep -v goal_cup
-- It must print only 20261017110000_email_delivery_rules.sql. A new name there means a mirrored
-- function may have a newer body outside this file: widen the glob or stop.
-- Applied to the shared database: everything up to 20261021100000 (the last on 2026-10-22), and
-- 20261021120000 to 20261021120200 (2026-10-23). NOT applied as of 2026-10-27:
-- 20261021120400_goal_cup_terms_2026_10_23.sql (website GOAL-CUP-SIGNUP-RUNBOOK, step 4).
-- PENDING (2026-10-27): the last section, 20261027100000_goal_cup_tracer_competition_repo_owner.sql,
-- was written here first and is not yet in the website repo nor applied anywhere: see sql/pending/README.md.
-- Once it merges there, regenerate as above; the output must equal this file below REGEN-END.
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

-- ======================== 20261016130000_goal_cup_tracer_device_request_replaces.sql ========================
-- Goal Cup tracer: the approval page says when the run it replaces started (owner, 2026-10-16:
-- "on replace, 'This replaces your current run, started <time>'").
--
-- goal_cup_device_request answered replaces_open_run as a boolean only. It now also answers
-- replaces_run_started_at: the official start (fmt_time) of the open run that the approval would
-- replace, or null when it replaces nothing. The rule for "replaces" is unchanged (20261015170000):
-- a run request in competition or test mode, and an open, unreplaced run of the same member, mode
-- and deployment whose deadline has not passed. replaces_open_run keeps its meaning; it is true
-- exactly when replaces_run_started_at is not null.
--
-- Additive: one function replaced (CREATE OR REPLACE keeps its grants), one new key in its answer.
-- No table, column, row or grant changes. A website build without the new key ignores it.

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
  IF v_dev.purpose = 'run' AND v_dev.mode IN ('competition', 'test') THEN
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

-- ======================== 20261017150200_goal_cup_notices_via_email_queue.sql ========================
-- Goal Cup notices now hand off to the email service atomically under the notice
-- row lock. sent_at means durable email-service handoff, never provider delivery.
-- Returning [] prevents an older upload Worker from submitting the mail itself.
-- queue_id deliberately has no FK: queue retention is independent of run evidence.
ALTER TABLE goal_cup_tracer.notices ADD COLUMN queue_id bigint;

DO $migration$
DECLARE old_expression text;
BEGIN
  SELECT pg_get_expr(conbin, conrelid) INTO old_expression FROM pg_constraint
   WHERE conrelid = 'public.email_tracking'::regclass AND conname = 'email_tracking_email_type_check';
  IF old_expression IS NULL THEN RAISE EXCEPTION 'email tracking constraint missing' USING ERRCODE = '55000'; END IF;
  ALTER TABLE public.email_tracking DROP CONSTRAINT email_tracking_email_type_check;
  EXECUTE 'ALTER TABLE public.email_tracking ADD CONSTRAINT email_tracking_email_type_check CHECK (('
    || old_expression || ') OR email_type = ''goalcup_run_notice'')';
END $migration$;

CREATE OR REPLACE FUNCTION public.goal_cup_upload_claim_notices(p_limit integer, p_now text) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_now timestamptz := p_now::timestamptz;
  v_staging boolean := caller_is_staging();
  v_n record;
  v_to text;
  v_data jsonb;
  v_queue_id bigint;
BEGIN
  FOR v_n IN
    SELECT n.* FROM notices n JOIN runs r ON r.id = n.run_id AND r.staging = v_staging
     WHERE n.sent_at IS NULL AND n.queue_id IS NULL AND n.skipped_at IS NULL
       AND (n.claimed_at IS NULL OR n.claimed_at < v_now - interval '15 minutes')
       AND n.attempts < 5
     ORDER BY n.created_at, n.id
     LIMIT greatest(least(coalesce(p_limit, 20), 200), 0)
     FOR UPDATE OF n SKIP LOCKED
  LOOP
    IF v_staging THEN
      v_to := 'delivered@resend.dev';
    ELSE
      SELECT u.email INTO v_to FROM auth.users u WHERE u.id = v_n.member_id;
    END IF;
    IF v_to IS NULL OR btrim(v_to) = '' THEN
      UPDATE notices SET skipped_at = v_now, skipped_reason = 'no_address' WHERE id = v_n.id;
      CONTINUE;
    END IF;
    v_to := lower(btrim(v_to));
    v_data := test_marked(notice_text(v_n.kind, v_n.payload, v_n.run_id), v_n.run_id)
      || jsonb_build_object('run_id', v_n.run_id, 'kind', v_n.kind);
    IF v_staging THEN
      v_data := jsonb_set(v_data, '{subject}', to_jsonb('[staging] ' || (v_data ->> 'subject')));
    END IF;

    -- Delivery policy and unsubscribe links belong to the gateway. This named
    -- site template has no stored content row and follows site-announcement retries.
    INSERT INTO public.email_queue(template, recipient_email, data, source, user_id,
                                   metadata, writer, kind, list_key, content_id)
    VALUES ('goalcup_run_notice', v_to, v_data, 'cron',
            CASE WHEN v_staging THEN NULL ELSE v_n.member_id END,
            jsonb_build_object('goal_cup_notice_id', v_n.id, 'run_id', v_n.run_id, 'kind', v_n.kind),
            'goalcup', 'announcement', 'goalcup', NULL)
    RETURNING id INTO v_queue_id;

    UPDATE notices SET sent_at = v_now, sent_to = v_to, queue_id = v_queue_id,
                       claimed_at = v_now, attempts = attempts + 1
     WHERE id = v_n.id;
  END LOOP;
  RETURN '[]'::jsonb;
END $$;

-- CREATE OR REPLACE preserves the existing service/ingest-role grants and signature.
-- finish_notice stays unchanged for an older Worker with an outstanding claim.
REVOKE ALL ON FUNCTION public.goal_cup_upload_claim_notices(integer, text) FROM PUBLIC, anon, authenticated;

-- ======================== 20261021100000_goal_cup_tracer_event_readiness.sql ========================
-- Goal Cup Proxy: event readiness (2026-10-21). One migration for five changes, applied once.
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
--        A negative p_stored_bytes raises; NULL (no size known) comes only from the old wrapper.
--    Operator notices at 50% and 100% of the limit are Worker log lines (quota-half, quota-full,
--    quota-exceeded), not rows in goal_cup_tracer.notices: that outbox mails the run's member.
--
-- 3. Heartbeat vitals and the disk_full email. The heartbeat gains an optional "vitals" object
--    (docs/UPLOAD-API.md, heartbeat):
--      {"disk_free_bytes": u64, "disk_short": bool, "agent_paused": "disk" | null,
--       "rss_bytes": u64, "cpu_ms": u64, "open_files": u64}
--    about Goal Cup Proxy's own processes and the machine's free disk, nothing else. The Worker checks
--    it and passes only these keys, nulls dropped. Goal Cup Proxy pauses the agent itself when the disk
--    is almost full ("agent_paused": "disk") and resumes it once there is space again.
--      * heartbeats.vitals: the vitals that heartbeat sent; null when it sent none.
--      * runs.last_vitals: the latest vitals any heartbeat of the run sent (a heartbeat without
--        vitals keeps the last), like last_client_ip.
--      * runs.disk_paused_at: when the current disk pause began; null while the agent is not
--        paused. A heartbeat whose vitals say agent_paused "disk" starts a pause (if none is
--        running); one whose vitals do not say it ends it. A heartbeat without vitals changes
--        neither: an older Goal Cup Proxy, or one that could not measure, says nothing about a pause.
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
-- The mirror's parity test (test/supabase.test.ts) checks that each Worker call matches exactly one
-- overload.
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
-- It now goes through the new function (no size known: NULL, counted as 0 bytes, one chunk).
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
  v_bytes bigint := coalesce(p_stored_bytes, 0); -- NULL only from the old wrapper above
BEGIN
  IF p_stored_bytes < 0 THEN
    RAISE EXCEPTION 'goal_cup_upload_commit_chunk: p_stored_bytes is negative (%)', p_stored_bytes;
  END IF;
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

-- The run-level facts a heartbeat adds, now with its vitals and the disk pause they report. The
-- disk pause rule has a twin in the Worker's local store, diskPause in server/worker/src/db-local.ts
-- (goal-cup-tracer): change both together.
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
-- none.
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

-- The old form, for the Worker that is live while this is applied: no detail.
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

-- ======================== 20261021120000_goal_cup_signups.sql ========================
-- Goal Cup Edition 01 sign-up: who enters the cup, and with which team.
--
-- Spec: docs/specs/2026-10-20-goal-cup-data-flows-design.md (A, F); plan:
-- docs/plans/2026-10-21-goal-cup-signup-data-flows.md (Task 2).
--
-- Locking: goal_cup_signup takes pg_advisory_xact_lock(hashtext('goal_cup_signup:' || event_id)) before
-- anything else. Every write that enforces a rule across signups and teammates takes the same lock first, so
-- the checks below see a stable picture and a collision returns 'email_taken' / 'already_signed_up'. The
-- unique indexes are a backstop only; there is deliberately no unique_violation handler. events is never
-- locked FOR UPDATE (it would block the FOR KEY SHARE lock open_run takes).
--
-- Additive: two new columns with a default, one UPDATE and one INSERT of events rows, new tables, new
-- functions. No existing function is replaced.
--
-- Apply: as docs/admin/GOAL-CUP-SIGNUP-RUNBOOK.md says (one transaction after SET lock_timeout = '5s'); then pnpm db:check.

-- ---- events ---------------------------------------------------------------------------------------

-- Two switches, set independently. requires_signup gates the official start (open_run and device_request, 20261021120200);
-- false is the start-gate kill switch and leaves sign-ups open. signup_open takes sign-ups (goal_cup_signup, until the
-- start window closes; not bounded below by start_window_opens); false closes sign-ups and leaves the gate on.
ALTER TABLE goal_cup_tracer.events ADD COLUMN requires_signup boolean NOT NULL DEFAULT false;
ALTER TABLE goal_cup_tracer.events ADD COLUMN signup_open boolean NOT NULL DEFAULT false;

-- The cup's start gate is on from now, but its sign-ups stay closed (signup_open keeps its default false) until the
-- terms migration 20261021120400 opens them.
UPDATE goal_cup_tracer.events SET requires_signup = true
 WHERE id = 'goalcup-cup-2026' AND mode = 'competition' AND staging = false;

-- The staging rehearsal of the cup. Organizers move its window with an UPDATE when they rehearse. The seeded window
-- is closed and in the past, so the event gates nothing until a rehearsal opens it: while a goalcup-cup-staging window
-- is open, staging dry-run approvals answer signup_required (see the runbook).
INSERT INTO goal_cup_tracer.events (id, mode, start_window_opens, start_window_closes, run_hours, staging, requires_signup, signup_open)
VALUES ('goalcup-cup-staging', 'competition', '2026-10-01T00:00:00Z', '2026-10-02T00:00:00Z', 168, true, true, true)
ON CONFLICT (id) DO NOTHING;

-- ---- tables ---------------------------------------------------------------------------------------

-- One active sign-up per member and event (the operator), numbered per event.
-- member_id is the operator's auth.users id, with no foreign key (the same convention as runs).
-- org_verified means "came from the operator's own ORGANIZATION registration", not a verified identity.
-- operator_erased_at: the operator's account was erased (goal_cup_erase_member). An entry with a cup run then stays
-- active as a submission (owner decision 2026-10-21) and operator_name() answers NULL, so no person is named on it.
CREATE TABLE goal_cup_tracer.signups (
  id                  uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  event_id            text NOT NULL REFERENCES goal_cup_tracer.events (id),
  member_id           uuid NOT NULL,
  entry_no            integer NOT NULL CHECK (entry_no > 0),
  name                text NOT NULL CHECK (name = btrim(name) AND char_length(name) BETWEEN 1 AND 80),
  org_registration_id uuid,
  org_name            text CHECK (org_name = btrim(org_name) AND char_length(org_name) BETWEEN 1 AND 200),
  org_verified        boolean NOT NULL DEFAULT false,
  anonymous           boolean NOT NULL DEFAULT false,
  research_opt_in     boolean NOT NULL DEFAULT false,
  created_at          timestamptz NOT NULL DEFAULT now(),
  updated_at          timestamptz NOT NULL DEFAULT now(),
  withdrawn_at        timestamptz,
  operator_erased_at  timestamptz,
  CHECK (org_verified = (org_registration_id IS NOT NULL)),
  UNIQUE (event_id, entry_no),
  UNIQUE (id, event_id)  -- target of the teammates (signup_id, event_id) key
);
CREATE UNIQUE INDEX goal_cup_tracer_signups_one_active ON goal_cup_tracer.signups (event_id, member_id)
  WHERE withdrawn_at IS NULL;
CREATE INDEX goal_cup_tracer_signups_member ON goal_cup_tracer.signups (member_id);

-- The people an operator lists, by lower-cased and trimmed email, at most once per event. Plus-aliases are not merged:
-- matching is exact. event_id is copied from the sign-up (the composite key keeps it equal) for that rule.
CREATE TABLE goal_cup_tracer.teammates (
  id               uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  signup_id        uuid NOT NULL,
  event_id         text NOT NULL,
  name             text NOT NULL CHECK (name = btrim(name) AND char_length(name) BETWEEN 1 AND 80),
  email_normalized text NOT NULL CHECK (email_normalized = lower(btrim(email_normalized)))
                                 CHECK (char_length(email_normalized) BETWEEN 3 AND 254),
  role_label       text CHECK (role_label = btrim(role_label) AND char_length(role_label) BETWEEN 1 AND 40),
  show_name        boolean NOT NULL DEFAULT false,
  notice_sent_at   timestamptz,
  created_at       timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (signup_id, event_id) REFERENCES goal_cup_tracer.signups (id, event_id) ON DELETE CASCADE,
  UNIQUE (event_id, email_normalized)
);
CREATE INDEX goal_cup_tracer_teammates_signup ON goal_cup_tracer.teammates (signup_id);

ALTER TABLE goal_cup_tracer.signups   ENABLE ROW LEVEL SECURITY;
ALTER TABLE goal_cup_tracer.teammates ENABLE ROW LEVEL SECURITY;
REVOKE ALL ON goal_cup_tracer.signups, goal_cup_tracer.teammates FROM PUBLIC, anon, authenticated;

-- ---- helpers (schema goal_cup_tracer; never granted to anyone) --------------------------------------

-- Frozen: the operator has a competition run in the sign-up's event. official_start is NOT NULL by
-- schema, so any such run counts, a replaced one included. Once frozen, the team list is locked.
CREATE FUNCTION goal_cup_tracer.is_frozen(p_signup_id uuid) RETURNS boolean
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
  SELECT EXISTS (
    SELECT 1 FROM signups s
      JOIN runs r ON r.member_id = s.member_id AND r.event_id = s.event_id
     WHERE s.id = p_signup_id AND r.mode = 'competition' AND r.official_start IS NOT NULL)
$$;

-- Where an email is already in use in an event: 'teammate' (listed on an entry), 'operator' (the current
-- auth.users email of the operator of an active sign-up, or of a withdrawn one whose operator has a cup run in the
-- event: that withdrawal is final, and the former operator must not come back as anyone's teammate) or NULL.
-- p_except_signup leaves one sign-up out (its own list when that list is replaced). The email is compared lower-cased
-- and trimmed.
CREATE FUNCTION goal_cup_tracer.email_in_use(p_event_id text, p_email text, p_except_signup uuid) RETURNS text
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
  SELECT CASE
    WHEN EXISTS (SELECT 1 FROM teammates t
                  WHERE t.event_id = p_event_id AND t.email_normalized = lower(btrim(p_email))
                    AND t.signup_id IS DISTINCT FROM p_except_signup) THEN 'teammate'
    WHEN EXISTS (SELECT 1 FROM signups s JOIN auth.users u ON u.id = s.member_id
                  WHERE s.event_id = p_event_id AND (s.withdrawn_at IS NULL OR is_frozen(s.id))
                    AND s.id IS DISTINCT FROM p_except_signup
                    AND lower(btrim(u.email)) = lower(btrim(p_email))) THEN 'operator'
  END
$$;

-- A team list from a form, normalised: [{"name","email","role"}] in input order, or NULL when it is malformed.
-- NULL means an empty list. Otherwise it must be an array of at most 10 objects with string "name" and "email" and a
-- string, null or absent "role". Names 1-80 characters after trimming, role trimmed (blank means none) up to 40,
-- email lower-cased and trimmed, up to 254, of the form x@y.z with no spaces, unique within the list; no control
-- characters. goal_cup_signup and goal_cup_teammates_set share it, so both apply the same limits.
CREATE FUNCTION goal_cup_tracer.normalized_team(p_list jsonb) RETURNS jsonb
LANGUAGE plpgsql IMMUTABLE SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_list         jsonb := coalesce(p_list, '[]'::jsonb);
  v_team         jsonb := '[]'::jsonb;
  v_item         jsonb;
  v_member_name  text;
  v_member_email text;
  v_role         text;
BEGIN
  IF jsonb_typeof(v_list) <> 'array' THEN RETURN NULL; END IF;
  IF jsonb_array_length(v_list) > 10 THEN RETURN NULL; END IF;
  FOR v_item IN SELECT value FROM jsonb_array_elements(v_list) WITH ORDINALITY ORDER BY ordinality LOOP
    IF jsonb_typeof(v_item) IS DISTINCT FROM 'object'
       OR jsonb_typeof(v_item -> 'name') IS DISTINCT FROM 'string'
       OR jsonb_typeof(v_item -> 'email') IS DISTINCT FROM 'string'
       OR coalesce(jsonb_typeof(v_item -> 'role'), 'null') NOT IN ('string', 'null') THEN
      RETURN NULL;
    END IF;
    v_member_name  := btrim(v_item ->> 'name');
    v_member_email := lower(btrim(v_item ->> 'email'));
    v_role         := nullif(btrim(v_item ->> 'role'), '');
    IF char_length(v_member_name) NOT BETWEEN 1 AND 80 OR v_member_name ~ '[[:cntrl:]]'
       OR char_length(v_member_email) > 254
       OR v_member_email !~ '^[^@[:space:][:cntrl:]]+@[^@[:space:][:cntrl:]]+\.[^@[:space:][:cntrl:]]+$'
       OR char_length(v_role) > 40 OR v_role ~ '[[:cntrl:]]'
       OR v_team @> jsonb_build_array(jsonb_build_object('email', v_member_email)) THEN
      RETURN NULL;
    END IF;
    v_team := v_team || jsonb_build_array(jsonb_build_object('name', v_member_name, 'email', v_member_email, 'role', v_role));
  END LOOP;
  RETURN v_team;
END $$;

-- ---- the website's functions (service_role only; the server passes the signed-in member) -----------

-- The terms version the sign-up form must send back.
CREATE FUNCTION public.goal_cup_current_terms() RETURNS text
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$ SELECT current_terms_version() $$;

-- Whether the sign-up page takes sign-ups for an event now, so it can say so before anyone fills in the form:
-- {"open": signup_open and the start window not yet closed (goal_cup_signup's checks 3 and 4), "closes_at": the
-- window close as an RFC 3339 UTC string}. An unknown or NULL event is {"open": false, "closes_at": null}. Reads only;
-- goal_cup_signup still decides at write time.
CREATE FUNCTION public.goal_cup_signup_status(p_event_id text) RETURNS jsonb
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
  SELECT coalesce(
    (SELECT jsonb_build_object('open', e.signup_open AND now() < e.start_window_closes,
                               'closes_at', fmt_time(e.start_window_closes))
       FROM events e WHERE e.id = p_event_id),
    jsonb_build_object('open', false, 'closes_at', NULL))
$$;

-- Sign the signed-in member up for an event as an operator, with an optional team.
--
-- p_teammates: [{"name": text, "email": text, "role": text|null}], at most 10 (NULL = none).
-- p_team_confirmed: the operator confirms everyone listed knows and is 18 or older; required when anyone is
-- listed. p_org_registration_id: one of the actor's own ORGANIZATION registrations (org_name is then taken
-- from it and org_verified is true); without it, p_org_name is optional free text (org_verified false).
--
-- Returns {"result":"ok","signup_id","entry_no","teammates":[{"id","name","email"}]} or {"result": R}.
-- The checks run in this fixed order, and the first that fails is the result:
--    1. not_a_member         no auth.users row with a confirmed email for p_actor_id
--    2. stale_version        p_terms_version is not the current terms version
--    3. not_signup_event     no such event, or signup_open is false (requires_signup is not read: it gates the
--                            start only)
--    4. closed               now() is at or after the event's start_window_closes
--    5. already_signed_up    the actor has an active sign-up in the event
--    6. withdrawn_after_run  the actor has a withdrawn sign-up in the event and a competition run in it
--    7. is_teammate          the actor's email is listed on an entry in the event (a list kept by a withdrawal
--                            after a cup run counts: such a list is never freed in the edition)
--    8. invalid              malformed name, org_name or teammates (shape, lengths, email form, duplicates)
--    9. invalid_org          p_org_registration_id is not one of the actor's ORGANIZATION registrations
--   10. team_not_confirmed   someone is listed and p_team_confirmed is not true
--   11. operator_is_listed   the actor's own email is in the list
--   12. email_taken          a listed email is in use in the event (the first one, in list order), plus "email"
-- On 'ok' the consent for p_terms_version is recorded (kept if already there).
--
-- Limits: name and teammate names 1-80 characters after trimming, free-text org_name up to 120 (blank means
-- none; ignored when p_org_registration_id is given), role up to 40 (blank means none), email up to 254 and of
-- the form x@y.z with no spaces; no control characters in any text. A registration's organization_name is
-- snapshotted trimmed and cut to 200 characters.
CREATE FUNCTION public.goal_cup_signup(
  p_actor_id uuid, p_event_id text, p_terms_version text, p_name text, p_org_registration_id uuid,
  p_org_name text, p_research_opt_in boolean, p_team_confirmed boolean, p_teammates jsonb
) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_event        events;
  v_email        text;
  v_name         text := btrim(p_name);
  v_org_name     text := nullif(btrim(p_org_name), '');
  v_org_verified boolean := false;
  v_team         jsonb := normalized_team(p_teammates);  -- [{"name","email","role"}], NULL when malformed
  v_item         jsonb;
  v_taken        text;
  v_signup_id    uuid;
  v_entry_no     integer;
BEGIN
  PERFORM pg_advisory_xact_lock(hashtext('goal_cup_signup:' || p_event_id));

  -- 1. not_a_member
  SELECT lower(btrim(u.email)) INTO v_email FROM auth.users u
   WHERE u.id = p_actor_id AND u.email_confirmed_at IS NOT NULL;
  IF NOT FOUND THEN RETURN jsonb_build_object('result', 'not_a_member'); END IF;

  -- 2. stale_version
  IF p_terms_version IS DISTINCT FROM current_terms_version() THEN
    RETURN jsonb_build_object('result', 'stale_version');
  END IF;

  -- 3. not_signup_event, 4. closed
  SELECT * INTO v_event FROM events WHERE id = p_event_id;
  IF NOT FOUND OR NOT v_event.signup_open THEN RETURN jsonb_build_object('result', 'not_signup_event'); END IF;
  IF now() >= v_event.start_window_closes THEN RETURN jsonb_build_object('result', 'closed'); END IF;

  -- 5. already_signed_up
  IF EXISTS (SELECT 1 FROM signups WHERE event_id = p_event_id AND member_id = p_actor_id AND withdrawn_at IS NULL) THEN
    RETURN jsonb_build_object('result', 'already_signed_up');
  END IF;

  -- 6. withdrawn_after_run: withdrawal after a cup run is final for the edition.
  IF EXISTS (SELECT 1 FROM signups WHERE event_id = p_event_id AND member_id = p_actor_id AND withdrawn_at IS NOT NULL)
     AND EXISTS (SELECT 1 FROM runs WHERE event_id = p_event_id AND member_id = p_actor_id AND mode = 'competition') THEN
    RETURN jsonb_build_object('result', 'withdrawn_after_run');
  END IF;

  -- 7. is_teammate (the actor's email is confirmed, checked in 1)
  IF EXISTS (SELECT 1 FROM teammates WHERE event_id = p_event_id AND email_normalized = v_email) THEN
    RETURN jsonb_build_object('result', 'is_teammate');
  END IF;

  -- 8. invalid
  IF v_name IS NULL OR char_length(v_name) NOT BETWEEN 1 AND 80 OR v_name ~ '[[:cntrl:]]'
     OR (p_org_registration_id IS NULL AND (char_length(v_org_name) > 120 OR v_org_name ~ '[[:cntrl:]]'))
     OR v_team IS NULL THEN
    RETURN jsonb_build_object('result', 'invalid');
  END IF;

  -- 9. invalid_org
  IF p_org_registration_id IS NOT NULL THEN
    SELECT nullif(btrim(left(btrim(r.organization_name), 200)), '') INTO v_org_name FROM public.registrations r
     WHERE r.id = p_org_registration_id AND r.user_id = p_actor_id AND r.registration_type = 'ORGANIZATION';
    IF NOT FOUND THEN RETURN jsonb_build_object('result', 'invalid_org'); END IF;
    v_org_verified := true;
  END IF;

  -- 10. team_not_confirmed
  IF jsonb_array_length(v_team) > 0 AND p_team_confirmed IS NOT TRUE THEN
    RETURN jsonb_build_object('result', 'team_not_confirmed');
  END IF;

  -- 11. operator_is_listed
  IF v_team @> jsonb_build_array(jsonb_build_object('email', v_email)) THEN
    RETURN jsonb_build_object('result', 'operator_is_listed');
  END IF;

  -- 12. email_taken
  FOR v_item IN SELECT value FROM jsonb_array_elements(v_team) WITH ORDINALITY ORDER BY ordinality LOOP
    v_taken := email_in_use(p_event_id, v_item ->> 'email', NULL);
    IF v_taken IS NOT NULL THEN
      RETURN jsonb_build_object('result', 'email_taken', 'email', v_item ->> 'email');
    END IF;
  END LOOP;

  -- Insert: entry_no is max + 1 under the lock (gaps after a withdrawal are expected).
  SELECT coalesce(max(entry_no), 0) + 1 INTO v_entry_no FROM signups WHERE event_id = p_event_id;
  INSERT INTO signups (event_id, member_id, entry_no, name, org_registration_id, org_name, org_verified, research_opt_in)
  VALUES (p_event_id, p_actor_id, v_entry_no, v_name, p_org_registration_id, v_org_name, v_org_verified,
          coalesce(p_research_opt_in, false))
  RETURNING id INTO v_signup_id;

  INSERT INTO consents (member_id, terms_version) VALUES (p_actor_id, p_terms_version)
  ON CONFLICT (member_id, terms_version) DO NOTHING;

  WITH listed AS (
    SELECT value ->> 'name' AS name, value ->> 'email' AS email, value ->> 'role' AS role, ordinality
      FROM jsonb_array_elements(v_team) WITH ORDINALITY
  ), inserted AS (
    INSERT INTO teammates (signup_id, event_id, name, email_normalized, role_label)
    SELECT v_signup_id, p_event_id, name, email, role FROM listed ORDER BY ordinality
    RETURNING id, name, email_normalized
  )
  SELECT coalesce(jsonb_agg(jsonb_build_object('id', i.id, 'name', i.name, 'email', i.email_normalized) ORDER BY l.ordinality), '[]'::jsonb)
    INTO v_team
    FROM inserted i JOIN listed l ON l.email = i.email_normalized;

  RETURN jsonb_build_object('result', 'ok', 'signup_id', v_signup_id, 'entry_no', v_entry_no, 'teammates', v_team);
END $$;

-- ---- privileges ---------------------------------------------------------------------------------------
-- Supabase grants EXECUTE on new functions to PUBLIC, anon and authenticated; revoke all three explicitly
-- (docs/SUPABASE.md, "SECURITY DEFINER grants").
REVOKE ALL ON FUNCTION
  goal_cup_tracer.is_frozen(uuid),
  goal_cup_tracer.normalized_team(jsonb),
  goal_cup_tracer.email_in_use(text, text, uuid),
  public.goal_cup_current_terms(),
  public.goal_cup_signup_status(text),
  public.goal_cup_signup(uuid, text, text, text, uuid, text, boolean, boolean, jsonb)
FROM PUBLIC, anon, authenticated;

GRANT EXECUTE ON FUNCTION
  public.goal_cup_current_terms(),
  public.goal_cup_signup_status(text),
  public.goal_cup_signup(uuid, text, text, text, uuid, text, boolean, boolean, jsonb)
TO service_role;

-- An apply against drifted production rows must fail loudly, not leave the gate off, staging sign-ups closed, or
-- sign-ups open on any other event (the cup's own sign-ups must still be closed here).
DO $$
DECLARE
  v_gated text;
  v_open  text;
BEGIN
  SELECT coalesce(string_agg(id, ',' ORDER BY id), '') INTO v_gated
    FROM goal_cup_tracer.events WHERE requires_signup;
  IF v_gated IS DISTINCT FROM 'goalcup-cup-2026,goalcup-cup-staging' THEN
    RAISE EXCEPTION 'goal cup sign-up gate: requires_signup is set on [%], expected [goalcup-cup-2026,goalcup-cup-staging]', v_gated;
  END IF;
  SELECT coalesce(string_agg(id, ',' ORDER BY id), '') INTO v_open
    FROM goal_cup_tracer.events WHERE signup_open;
  IF v_open IS DISTINCT FROM 'goalcup-cup-staging' THEN
    RAISE EXCEPTION 'goal cup sign-up gate: signup_open is set on [%], expected [goalcup-cup-staging]', v_open;
  END IF;
  IF NOT EXISTS (SELECT 1 FROM goal_cup_tracer.events
                  WHERE id = 'goalcup-cup-staging' AND mode = 'competition' AND staging) THEN
    RAISE EXCEPTION 'goal cup sign-up gate: goalcup-cup-staging is not a competition event with staging = true';
  END IF;
END $$;

-- ======================== 20261021120100_goal_cup_signup_team.sql ========================
-- Goal Cup Edition 01 sign-up: the entry view, the operator's edits, withdrawal, the teammates' own choices, teammate
-- removal, and the state of one sign-up for the email follow-up.
--
-- Spec: docs/specs/2026-10-20-goal-cup-data-flows-design.md (B, C); plan:
-- docs/plans/2026-10-21-goal-cup-signup-data-flows.md (Tasks 3 and 5).
--
-- Every id from a form is checked against p_actor_id here. A sign-up the actor does not operate, or one that does
-- not exist, is 'not_operator' either way, so existence does not leak.
--
-- Locking (spec database rules 1 and 2): teammates_set, signup_withdraw, teammate_show_name, teammate_leave and
-- teammate_remove take pg_advisory_xact_lock(hashtext('goal_cup_signup:' || event_id)) first (when only an id is given,
-- its event id is read unlocked first), then the signups row FOR UPDATE, then the teammates rows. All five check is_frozen
-- in a later statement than the row lock, so a cup run committed while they waited is seen (open_run holds the
-- sign-up FOR SHARE until it commits, but only while requires_signup is on; with the start gate off, a withdrawal
-- or erasure racing a first start can still delete the team: accepted). teammates_set, teammate_show_name,
-- teammate_leave and teammate_remove refuse a frozen list.
-- signup_withdraw is allowed after a run too; it checks is_frozen only to decide whether the teammate rows are
-- deleted: before any cup run they are (their emails are freed), after one they are kept (their emails stay used for
-- the edition, so a listed person cannot start the entry again as an operator). events is never locked FOR UPDATE.
-- There is no unique_violation handler: the advisory lock makes a collision 'email_taken'.
--
-- Additive: new functions only.
--
-- Apply: as docs/admin/GOAL-CUP-SIGNUP-RUNBOOK.md says (one transaction after SET lock_timeout = '5s'); then pnpm db:check.

-- ---- helpers (schema goal_cup_tracer; never granted to anyone) --------------------------------------

-- The operator's current cup run in an event: the latest non-replaced competition run by official start, else the
-- latest replaced one. NULL when there is none.
CREATE FUNCTION goal_cup_tracer.current_cup_run(p_member_id uuid, p_event_id text) RETURNS goal_cup_tracer.runs
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
  SELECT * FROM runs
   WHERE member_id = p_member_id AND event_id = p_event_id AND mode = 'competition'
   ORDER BY (replaced_at IS NULL) DESC, official_start DESC, created_at DESC, id DESC
   LIMIT 1
$$;

-- The operator's name as teammates see it: the full_name of the operator's own registration (the one the entry
-- is linked to first, then an INDIVIDUAL one, then the most recent), cut to 120 characters. Without one, the
-- entry name stands in. Never the operator's email. NULL once the operator's account was erased
-- (signups.operator_erased_at): nothing may name that person, not a registration that outlived the account.
CREATE FUNCTION goal_cup_tracer.operator_name(p_signup_id uuid) RETURNS text
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
  SELECT CASE WHEN s.operator_erased_at IS NOT NULL THEN NULL ELSE coalesce(
    (SELECT btrim(left(btrim(r.full_name), 120)) FROM public.registrations r
      WHERE r.user_id = s.member_id AND nullif(btrim(r.full_name), '') IS NOT NULL
      ORDER BY (r.id = s.org_registration_id) IS TRUE DESC, (r.registration_type = 'INDIVIDUAL') DESC,
               r.created_at DESC NULLS LAST, r.id
      LIMIT 1),
    s.name) END
  FROM signups s WHERE s.id = p_signup_id
$$;

-- ---- the website's functions (service_role only; the server passes the signed-in member) -----------

-- The entry the signed-in member is on in an event.
--
-- Which entry: the actor's active sign-up (role operator); else the actor's latest withdrawn sign-up whose withdrawal
-- is final (the actor has a cup run in the event, is_frozen; role operator), which no listing can hide; else the entry
-- that lists the actor's verified email (role teammate; the account's email_confirmed_at must be set, matched
-- lower-cased and trimmed); else the actor's latest withdrawn sign-up (role operator). Otherwise
-- {"result":"none","email": the account email as stored, or null when there is no account}.
--
-- {"result":"ok","role","frozen","signup":{"id","entry_no","name","org_name","org_verified","anonymous",
--  "research_opt_in","withdrawn_at"},"operator_name","teammates":[…],"run":{"status","official_start",
--  "deadline_at","final_commit"}|null,"show_name"}
--   * teammates, ordered by when they were listed, then name: for an operator {"id","name","email","role",
--     "show_name"}; for a teammate {"name","role","show_name"} only (no ids, no emails).
--   * show_name: the signed-in teammate's own choice; null for an operator.
--   * run: the operator's current cup run (current_cup_run), null when there is none.
--   * Times are RFC 3339 UTC strings to the second.
CREATE FUNCTION public.goal_cup_my_entry(p_actor_id uuid, p_event_id text) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_email      text;
  v_confirmed  boolean := false;
  v_signup     signups;
  v_self       teammates;
  v_role       text;
  v_run        runs;
BEGIN
  SELECT u.email, u.email_confirmed_at IS NOT NULL INTO v_email, v_confirmed FROM auth.users u WHERE u.id = p_actor_id;

  -- The actor's own sign-up: the active one, else the latest withdrawn one. All of them share one operator and event,
  -- so one cup run (current_cup_run) decides whether a withdrawal is final (is_frozen).
  SELECT * INTO v_signup FROM signups
   WHERE event_id = p_event_id AND member_id = p_actor_id
   ORDER BY withdrawn_at DESC NULLS FIRST, created_at DESC, id DESC
   LIMIT 1;
  IF FOUND THEN
    v_run := current_cup_run(p_actor_id, p_event_id);
    IF v_signup.withdrawn_at IS NULL OR v_run.id IS NOT NULL THEN v_role := 'operator'; END IF;
  END IF;

  IF v_role IS NULL AND v_confirmed THEN
    SELECT * INTO v_self FROM teammates
     WHERE event_id = p_event_id AND email_normalized = lower(btrim(v_email));
    IF FOUND THEN
      SELECT * INTO v_signup FROM signups WHERE id = v_self.signup_id;
      v_run := current_cup_run(v_signup.member_id, p_event_id);
      v_role := 'teammate';
    END IF;
  END IF;

  IF v_role IS NULL THEN
    IF v_signup.id IS NULL THEN RETURN jsonb_build_object('result', 'none', 'email', v_email); END IF;
    v_role := 'operator';
  END IF;

  RETURN jsonb_build_object(
    'result', 'ok',
    'role', v_role,
    -- Relies on current_cup_run matching is_frozen (runs.official_start NOT NULL; replaced runs counted): change both.
    'frozen', v_run.id IS NOT NULL,
    'signup', jsonb_build_object(
      'id', v_signup.id, 'entry_no', v_signup.entry_no, 'name', v_signup.name, 'org_name', v_signup.org_name,
      'org_verified', v_signup.org_verified, 'anonymous', v_signup.anonymous,
      'research_opt_in', v_signup.research_opt_in, 'withdrawn_at', fmt_time(v_signup.withdrawn_at)),
    'operator_name', operator_name(v_signup.id),
    'teammates', (
      SELECT coalesce(jsonb_agg(
               jsonb_build_object('name', t.name, 'role', t.role_label, 'show_name', t.show_name)
               || CASE WHEN v_role = 'operator' THEN jsonb_build_object('id', t.id, 'email', t.email_normalized)
                       ELSE '{}'::jsonb END
               ORDER BY t.created_at, t.name, t.id), '[]'::jsonb)
        FROM teammates t WHERE t.signup_id = v_signup.id),
    'run', CASE WHEN v_run.id IS NULL THEN NULL ELSE jsonb_build_object(
      'status', v_run.status, 'official_start', fmt_time(v_run.official_start),
      'deadline_at', fmt_time(v_run.deadline_at), 'final_commit', v_run.final_commit) END,
    'show_name', CASE WHEN v_role = 'teammate' THEN v_self.show_name END);
END $$;

-- Replace the operator's team list. It diffs by email: an email that stays keeps its row (id, notice_sent_at,
-- show_name), with the new name and role; a new email gets a new row; a dropped email's row is deleted.
--
-- p_teammates and p_team_confirmed follow goal_cup_signup: the same list shape and limits (normalized_team), NULL
-- means an empty list, and the confirmation is required whenever anyone is listed.
--
-- Returns {"result":"ok","added":[{"id","name","email"}]} (the new rows, in list order) or {"result": R}. The
-- checks run in this fixed order, and the first that fails is the result:
--   1. not_operator         p_signup_id is not a sign-up of p_actor_id (or does not exist)
--   2. withdrawn            the sign-up is withdrawn
--   3. frozen               the operator has a cup run in the event (is_frozen)
--   4. invalid              malformed list
--   5. team_not_confirmed   someone is listed and p_team_confirmed is not true
--   6. operator_is_listed   the operator's own current email is in the list
--   7. email_taken          an email being added is in use in the event (the first, in list order), plus "email".
--                           Emails already on this list are not re-checked.
CREATE FUNCTION public.goal_cup_teammates_set(p_actor_id uuid, p_signup_id uuid, p_teammates jsonb, p_team_confirmed boolean)
RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_event_id text;
  v_signup   signups;
  v_team     jsonb;
  v_email    text;
  v_item     jsonb;
  v_added    jsonb;
BEGIN
  -- 1. not_operator (read unlocked for the event id, then again under the locks)
  SELECT event_id INTO v_event_id FROM signups WHERE id = p_signup_id AND member_id = p_actor_id;
  IF NOT FOUND THEN RETURN jsonb_build_object('result', 'not_operator'); END IF;

  PERFORM pg_advisory_xact_lock(hashtext('goal_cup_signup:' || v_event_id));
  SELECT * INTO v_signup FROM signups WHERE id = p_signup_id AND member_id = p_actor_id FOR UPDATE;
  IF NOT FOUND THEN RETURN jsonb_build_object('result', 'not_operator'); END IF;

  -- 2. withdrawn
  IF v_signup.withdrawn_at IS NOT NULL THEN RETURN jsonb_build_object('result', 'withdrawn'); END IF;

  -- 3. frozen (a statement after the row lock, so a run committed while this call waited is seen)
  IF is_frozen(p_signup_id) THEN RETURN jsonb_build_object('result', 'frozen'); END IF;

  -- 4. invalid
  v_team := normalized_team(p_teammates);
  IF v_team IS NULL THEN RETURN jsonb_build_object('result', 'invalid'); END IF;

  -- 5. team_not_confirmed
  IF jsonb_array_length(v_team) > 0 AND p_team_confirmed IS NOT TRUE THEN
    RETURN jsonb_build_object('result', 'team_not_confirmed');
  END IF;

  -- 6. operator_is_listed
  SELECT lower(btrim(u.email)) INTO v_email FROM auth.users u WHERE u.id = p_actor_id;
  IF v_email IS NOT NULL AND v_team @> jsonb_build_array(jsonb_build_object('email', v_email)) THEN
    RETURN jsonb_build_object('result', 'operator_is_listed');
  END IF;

  -- 7. email_taken
  FOR v_item IN SELECT value FROM jsonb_array_elements(v_team) WITH ORDINALITY ORDER BY ordinality LOOP
    CONTINUE WHEN EXISTS (SELECT 1 FROM teammates WHERE signup_id = p_signup_id AND email_normalized = v_item ->> 'email');
    IF email_in_use(v_event_id, v_item ->> 'email', p_signup_id) IS NOT NULL THEN
      RETURN jsonb_build_object('result', 'email_taken', 'email', v_item ->> 'email');
    END IF;
  END LOOP;

  -- Write: the sign-up row is locked above; the teammates rows are changed after it.
  DELETE FROM teammates t
   WHERE t.signup_id = p_signup_id
     AND NOT v_team @> jsonb_build_array(jsonb_build_object('email', t.email_normalized));

  UPDATE teammates t SET name = l.value ->> 'name', role_label = l.value ->> 'role'
    FROM jsonb_array_elements(v_team) l
   WHERE t.signup_id = p_signup_id AND t.email_normalized = l.value ->> 'email'
     AND (t.name, t.role_label) IS DISTINCT FROM (l.value ->> 'name', l.value ->> 'role');

  WITH listed AS (
    SELECT value ->> 'name' AS name, value ->> 'email' AS email, value ->> 'role' AS role, ordinality
      FROM jsonb_array_elements(v_team) WITH ORDINALITY
  ), inserted AS (
    INSERT INTO teammates (signup_id, event_id, name, email_normalized, role_label)
    SELECT p_signup_id, v_event_id, l.name, l.email, l.role FROM listed l
     WHERE NOT EXISTS (SELECT 1 FROM teammates t WHERE t.signup_id = p_signup_id AND t.email_normalized = l.email)
     ORDER BY l.ordinality
    RETURNING id, name, email_normalized
  )
  SELECT coalesce(jsonb_agg(jsonb_build_object('id', i.id, 'name', i.name, 'email', i.email_normalized) ORDER BY l.ordinality), '[]'::jsonb)
    INTO v_added
    FROM inserted i JOIN listed l ON l.email = i.email_normalized;

  UPDATE signups SET updated_at = now() WHERE id = p_signup_id;

  RETURN jsonb_build_object('result', 'ok', 'added', v_added);
END $$;

-- Change the entry's name, anonymity and research opt-in. All three are required (the form sends them all).
-- Returns, in this order: not_operator, withdrawn, invalid (name 1-80 characters after trimming with no control
-- characters; a NULL argument), run_ended (the research opt-in changes while the operator's current cup run,
-- current_cup_run, is no longer 'open'; the name and anonymity can still change), or ok.
-- A single-row write: it locks the sign-up FOR UPDATE and takes no advisory lock.
CREATE FUNCTION public.goal_cup_signup_update(p_actor_id uuid, p_signup_id uuid, p_name text, p_anonymous boolean, p_research_opt_in boolean)
RETURNS text
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_signup signups;
  v_name   text := btrim(p_name);
  v_run    runs;
BEGIN
  SELECT * INTO v_signup FROM signups WHERE id = p_signup_id AND member_id = p_actor_id FOR UPDATE;
  IF NOT FOUND THEN RETURN 'not_operator'; END IF;
  IF v_signup.withdrawn_at IS NOT NULL THEN RETURN 'withdrawn'; END IF;
  IF v_name IS NULL OR char_length(v_name) NOT BETWEEN 1 AND 80 OR v_name ~ '[[:cntrl:]]'
     OR p_anonymous IS NULL OR p_research_opt_in IS NULL THEN
    RETURN 'invalid';
  END IF;
  IF p_research_opt_in IS DISTINCT FROM v_signup.research_opt_in THEN
    v_run := current_cup_run(v_signup.member_id, v_signup.event_id);
    IF v_run.id IS NOT NULL AND v_run.status <> 'open' THEN RETURN 'run_ended'; END IF;
  END IF;
  UPDATE signups SET name = v_name, anonymous = p_anonymous, research_opt_in = p_research_opt_in, updated_at = now()
   WHERE id = p_signup_id;
  RETURN 'ok';
END $$;

-- Withdraw the operator's sign-up: set withdrawn_at. Before any cup run (not is_frozen) it also deletes the
-- teammate rows, which frees their emails. After a cup run it is final for the edition (goal_cup_signup answers
-- withdrawn_after_run) and keeps the teammate rows: the list stays frozen, its emails stay in use (goal_cup_signup
-- answers is_teammate to a listed account, email_taken to anyone listing one), and closing that path is the point:
-- a listed person must not start the withdrawn entry again under a new sign-up. A listed person can still leave
-- through erasure (goal_cup_erase_member). Returns not_operator, already_withdrawn or ok.
CREATE FUNCTION public.goal_cup_signup_withdraw(p_actor_id uuid, p_signup_id uuid) RETURNS text
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_event_id text;
  v_signup   signups;
  v_frozen   boolean;
BEGIN
  SELECT event_id INTO v_event_id FROM signups WHERE id = p_signup_id AND member_id = p_actor_id;
  IF NOT FOUND THEN RETURN 'not_operator'; END IF;

  PERFORM pg_advisory_xact_lock(hashtext('goal_cup_signup:' || v_event_id));
  SELECT * INTO v_signup FROM signups WHERE id = p_signup_id AND member_id = p_actor_id FOR UPDATE;
  IF NOT FOUND THEN RETURN 'not_operator'; END IF;
  IF v_signup.withdrawn_at IS NOT NULL THEN RETURN 'already_withdrawn'; END IF;

  -- A statement after the row lock, so a cup run committed while this call waited on open_run's FOR SHARE is seen.
  v_frozen := is_frozen(p_signup_id);

  UPDATE signups SET withdrawn_at = now(), updated_at = now() WHERE id = p_signup_id;
  IF NOT v_frozen THEN
    DELETE FROM teammates WHERE signup_id = p_signup_id;
  END IF;
  RETURN 'ok';
END $$;

-- A linked teammate shows or hides their own name (p_show NULL hides it). The actor must have a confirmed email
-- that is listed in the event. Returns not_listed, frozen or ok.
CREATE FUNCTION public.goal_cup_teammate_show_name(p_actor_id uuid, p_event_id text, p_show boolean) RETURNS text
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_email     text;
  v_teammate  uuid;
  v_signup_id uuid;
BEGIN
  SELECT lower(btrim(u.email)) INTO v_email FROM auth.users u WHERE u.id = p_actor_id AND u.email_confirmed_at IS NOT NULL;
  IF v_email IS NULL THEN RETURN 'not_listed'; END IF;

  -- Under the event lock the listing cannot change (every teammates write takes it), so one read serves; the final
  -- statement locks the row.
  PERFORM pg_advisory_xact_lock(hashtext('goal_cup_signup:' || p_event_id));
  SELECT id, signup_id INTO v_teammate, v_signup_id FROM teammates WHERE event_id = p_event_id AND email_normalized = v_email;
  IF NOT FOUND THEN RETURN 'not_listed'; END IF;
  PERFORM 1 FROM signups WHERE id = v_signup_id FOR UPDATE;

  IF is_frozen(v_signup_id) THEN RETURN 'frozen'; END IF;

  UPDATE teammates SET show_name = (p_show IS TRUE) WHERE id = v_teammate;
  RETURN 'ok';
END $$;

-- A linked teammate leaves the team they are listed on in the event: their own row is deleted, unless the list is
-- frozen (the official start locks it for everyone, owner decision 2026-10-21). The actor must have a confirmed email
-- that is listed in the event, as goal_cup_teammate_show_name. Same locks as goal_cup_teammate_remove: the event lock,
-- the sign-up FOR UPDATE, the row, then is_frozen in a later statement, so a start that committed while this waited is
-- seen. Returns not_listed, frozen or ok.
CREATE FUNCTION public.goal_cup_teammate_leave(p_actor_id uuid, p_event_id text) RETURNS text
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_email     text;
  v_teammate  uuid;
  v_signup_id uuid;
BEGIN
  SELECT lower(btrim(u.email)) INTO v_email FROM auth.users u WHERE u.id = p_actor_id AND u.email_confirmed_at IS NOT NULL;
  IF v_email IS NULL THEN RETURN 'not_listed'; END IF;

  -- Under the event lock the listing cannot change (every teammates write takes it), so one read serves; the final
  -- statement locks the row.
  PERFORM pg_advisory_xact_lock(hashtext('goal_cup_signup:' || p_event_id));
  SELECT id, signup_id INTO v_teammate, v_signup_id FROM teammates WHERE event_id = p_event_id AND email_normalized = v_email;
  IF NOT FOUND THEN RETURN 'not_listed'; END IF;
  PERFORM 1 FROM signups WHERE id = v_signup_id FOR UPDATE;

  IF is_frozen(v_signup_id) THEN RETURN 'frozen'; END IF;

  DELETE FROM teammates WHERE id = v_teammate;
  RETURN 'ok';
END $$;

-- The "Remove me" page: goal_cup_teammate_preview and goal_cup_teammate_remove take no actor; the website verifies the
-- HMAC-signed removal token first.

-- What the "Remove me" page shows. {"result":"ok","name","entry_name","operator_name","frozen"} or
-- {"result":"not_found"}. No emails.
CREATE FUNCTION public.goal_cup_teammate_preview(p_teammate_id uuid) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_name      text;
  v_entry     text;
  v_signup_id uuid;
BEGIN
  SELECT t.name, s.name, s.id INTO v_name, v_entry, v_signup_id
    FROM teammates t JOIN signups s ON s.id = t.signup_id WHERE t.id = p_teammate_id;
  IF NOT FOUND THEN RETURN jsonb_build_object('result', 'not_found'); END IF;
  RETURN jsonb_build_object('result', 'ok', 'name', v_name, 'entry_name', v_entry,
                            'operator_name', operator_name(v_signup_id), 'frozen', is_frozen(v_signup_id));
END $$;

-- Remove a teammate from an entry (the "Remove me" link): delete the row unless the list is frozen.
-- Returns not_found, frozen or ok.
CREATE FUNCTION public.goal_cup_teammate_remove(p_teammate_id uuid) RETURNS text
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_event_id  text;
  v_signup_id uuid;
BEGIN
  SELECT event_id INTO v_event_id FROM teammates WHERE id = p_teammate_id;
  IF NOT FOUND THEN RETURN 'not_found'; END IF;

  -- Read again under the event lock: the row may have gone while this call waited. Every teammates write takes that
  -- lock, so the row stays until the final statement, which locks it.
  PERFORM pg_advisory_xact_lock(hashtext('goal_cup_signup:' || v_event_id));
  SELECT signup_id INTO v_signup_id FROM teammates WHERE id = p_teammate_id;
  IF NOT FOUND THEN RETURN 'not_found'; END IF;
  PERFORM 1 FROM signups WHERE id = v_signup_id FOR UPDATE;

  IF is_frozen(v_signup_id) THEN RETURN 'frozen'; END IF;

  DELETE FROM teammates WHERE id = p_teammate_id;
  RETURN 'ok';
END $$;

-- The state of one sign-up, for the email follow-up, which calls it at send time to re-check that a mail is still
-- wanted: 'withdrawn' (withdrawn_at is set; wins over 'started', as a withdrawal after the start is final), else
-- 'started' (is_frozen: the operator has a competition run in the event, a replaced one included), else 'active'.
-- NULL for no such sign-up or a NULL id; the follow-up treats NULL as "do not send". It only reads and takes no lock,
-- so a state read never waits on a start or a withdrawal.
CREATE FUNCTION public.goal_cup_signup_state(p_signup_id uuid) RETURNS text
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
  SELECT CASE
           WHEN s.withdrawn_at IS NOT NULL THEN 'withdrawn'
           WHEN is_frozen(s.id) THEN 'started'
           ELSE 'active'
         END
    FROM signups s
   WHERE s.id = p_signup_id
$$;

-- ---- privileges ---------------------------------------------------------------------------------------
-- Supabase grants EXECUTE on new functions to PUBLIC, anon and authenticated; revoke all three explicitly
-- (docs/SUPABASE.md, "SECURITY DEFINER grants").
REVOKE ALL ON FUNCTION
  goal_cup_tracer.current_cup_run(uuid, text),
  goal_cup_tracer.operator_name(uuid),
  public.goal_cup_my_entry(uuid, text),
  public.goal_cup_teammates_set(uuid, uuid, jsonb, boolean),
  public.goal_cup_signup_update(uuid, uuid, text, boolean, boolean),
  public.goal_cup_signup_withdraw(uuid, uuid),
  public.goal_cup_teammate_show_name(uuid, text, boolean),
  public.goal_cup_teammate_leave(uuid, text),
  public.goal_cup_teammate_preview(uuid),
  public.goal_cup_teammate_remove(uuid),
  public.goal_cup_signup_state(uuid)
FROM PUBLIC, anon, authenticated;

GRANT EXECUTE ON FUNCTION
  public.goal_cup_my_entry(uuid, text),
  public.goal_cup_teammates_set(uuid, uuid, jsonb, boolean),
  public.goal_cup_signup_update(uuid, uuid, text, boolean, boolean),
  public.goal_cup_signup_withdraw(uuid, uuid),
  public.goal_cup_teammate_show_name(uuid, text, boolean),
  public.goal_cup_teammate_leave(uuid, text),
  public.goal_cup_teammate_preview(uuid),
  public.goal_cup_teammate_remove(uuid),
  public.goal_cup_signup_state(uuid)
TO service_role;

-- ======================== 20261021120200_goal_cup_signup_gate.sql ========================
-- Goal Cup Edition 01 sign-up: the gate on the official start, the approval page, account erasure and the staging seed.
--
-- Spec: docs/specs/2026-10-20-goal-cup-data-flows-design.md (database rules 1-5 and 9, H); plan:
-- docs/plans/2026-10-21-goal-cup-signup-data-flows.md (Task 4).
--
-- Replaces public.goal_cup_upload_open_run, goal_cup_device_request and goal_cup_erase_member from their latest
-- bodies, with the same argument lists (a new argument would add a PUBLIC-executable overload), and adds
-- goal_cup_tracer.seed_staging_signup. Each section below says what changed. The latest bodies of open_run and
-- device_request are the ones in 20261021100000_goal_cup_tracer_event_readiness.sql (applied to production on
-- 2026-10-22: only a competition run replaces; test runs run side by side). This migration builds on it and must be
-- applied after it; it keeps that rule and adds only the sign-up parts. erase_member's latest body is 20261015150000.
--
-- Locking (spec database rule 1): erasure takes the event advisory lock of every event that takes sign-ups or gates
-- starts, or holds any sign-up, in one fixed order (by id) before any row, so it waits for a sign-up in flight and two
-- erasures never lock in opposite orders; then run credentials, sign-ups and teammates, in the order open_run locks
-- them (credential, then sign-up), so an erasure racing a start waits rather than deadlocks. events is never locked
-- FOR UPDATE.
--
-- CREATE OR REPLACE keeps each function's ACL; the grants are asserted again below anyway. No table, column or row
-- changes.
--
-- Apply: as docs/admin/GOAL-CUP-SIGNUP-RUNBOOK.md says (one transaction after SET lock_timeout = '5s'); then pnpm db:check.

-- ---- guard: event readiness first -------------------------------------------------------------------------
-- The bodies below start from 20261021100000. On a database without it they would silently bring back the older
-- rule (a test run replaces the operator's test run), so the apply fails here and its transaction rolls back. The
-- check is an overload only 20261021100000 creates.
DO $$
BEGIN
  IF to_regprocedure('public.goal_cup_upload_commit_chunk(text,text,jsonb,bigint)') IS NULL THEN
    RAISE EXCEPTION '20261021120200 needs 20261021100000_goal_cup_tracer_event_readiness.sql: apply 20261021100000 first';
  END IF;
END $$;

-- ---- open_run: the sign-up gate ---------------------------------------------------------------------------
-- Unchanged from 20261021100000 except the gate, placed after the credential checks (so a retried start of a run
-- that already exists still answers 'existing') and before the first write. Only a competition run in an event with
-- requires_signup is gated (practice, dry run and test are not); the Worker passes the forbidden message through as
-- its 403.
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

-- ---- device_request: what the approval page needs to know about the sign-up --------------------------------
-- Unchanged from 20261021100000 except four new keys, always present (replaces_open_run stays competition only):
--   * signup_required: the request is a run in competition mode, and its own deployment (device_requests.staging)
--     has an event with requires_signup, mode competition and the start window open now. If several match, the one
--     whose window opened last (then the lowest id). That event is "the sign-up event" below.
--   * signed_up: the actor has an active sign-up in the sign-up event.
--   * operator_name: the actor's verified email (email_confirmed_at set; matched lower-cased and trimmed) is listed
--     on another operator's active entry in the sign-up event: that operator's name (operator_name()), or '' when the
--     entry names no operator any more (an erased operator's kept entry). Non-null means "listed as a teammate".
--   * signup_event_id: the sign-up event's id, so the approval page signs the actor up for the event this request
--     would start in (a staging request reads the staging event).
-- Otherwise false, false, null and null.
CREATE OR REPLACE FUNCTION public.goal_cup_device_request(p_actor_id uuid, p_user_code text) RETURNS jsonb
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_dev           device_requests;
  v_replaces      timestamptz;
  v_signup_event  text;
  v_signed_up     boolean := false;
  v_operator_name text;
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
  IF v_dev.purpose = 'run' AND v_dev.mode = 'competition' THEN
    SELECT e.id INTO v_signup_event FROM events e
     WHERE e.staging = v_dev.staging AND e.requires_signup AND e.mode = 'competition'
       AND e.start_window_opens <= now() AND now() <= e.start_window_closes
     ORDER BY e.start_window_opens DESC, e.id
     LIMIT 1;
    IF v_signup_event IS NOT NULL THEN
      v_signed_up := EXISTS (SELECT 1 FROM signups s
                              WHERE s.event_id = v_signup_event AND s.member_id = p_actor_id AND s.withdrawn_at IS NULL);
      SELECT coalesce(operator_name(s.id), '') INTO v_operator_name
        FROM auth.users u
        JOIN teammates t ON t.event_id = v_signup_event AND t.email_normalized = lower(btrim(u.email))
        JOIN signups s ON s.id = t.signup_id AND s.withdrawn_at IS NULL AND s.member_id <> p_actor_id
       WHERE u.id = p_actor_id AND u.email_confirmed_at IS NOT NULL;
    END IF;
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
    'decided_by_you', v_dev.member_id IS NOT DISTINCT FROM p_actor_id AND v_dev.status <> 'pending',
    'signup_required', v_signup_event IS NOT NULL,
    'signed_up', v_signed_up,
    'operator_name', v_operator_name,
    'signup_event_id', v_signup_event);
END $$;

-- ---- erase_member: the sign-up and the team lists too --------------------------------------------------------
-- From 20261015150000, with (spec H as changed by the owner on 2026-10-21: the hackathon submission is not the
-- membership):
--   * every sign-up of the member gets operator_erased_at, so operator_name() answers NULL for it: no entry resolves
--     or shows the person's name any more, not a registration that outlived the account, and never an email.
--     member_id stays, like runs.member_id.
--   * signups_kept: the member's active sign-ups whose operator has a cup run in the event (is_frozen) stay active as
--     submissions, made anonymous (anonymous = true) so public pages show only "Entry NNN"; the repo, prompt, videos
--     and research_opt_in are unchanged. The rows keep name, org_name and the frozen team list, which the anonymous
--     flag keeps off public pages.
--   * signups_withdrawn: the member's other active sign-ups (no cup run) are withdrawn, as before. The rows stay: an
--     entry number is never reused.
--   * entry_teammates: the teammate rows deleted from the member's own sign-ups without a cup run, which frees those
--     emails. A sign-up with a cup run keeps them (frozen; other people's listings). Both decisions read is_frozen in a
--     statement after the sign-ups are locked FOR UPDATE, which waits on open_run's FOR SHARE, so a run committed
--     meanwhile is seen. That holds only while requires_signup is on: with the start gate off, open_run takes no
--     FOR SHARE, so an erasure racing a first start can still withdraw the entry and delete the team (accepted).
--   * teammate_listings: every teammate row listing the member's email is deleted, in any event, on a frozen list too
--     (privacy wins over the frozen list). The email is the account's current auth.users email, lower-cased and
--     trimmed, and only when it is verified (spec database rule 7: an account matches a teammate only with
--     email_confirmed_at set; an unverified account never removes a listing of an address it has not proven).
--     DELETE /api/my-data calls this before it deletes the auth user.
-- Order: the event advisory locks of every event with signup_open, requires_signup or any sign-up, sorted by id, before
-- any row; then any further event the member's rows touch, re-read until none is new (a safety net: new sign-ups only
-- happen in a locked event); then the run credentials (before the sign-ups, as open_run locks them), the sign-ups FOR
-- UPDATE, the teammates; then, as before, consents, notices, runs and heartbeats. Idempotent: a second call changes
-- nothing and answers zeros.
CREATE OR REPLACE FUNCTION public.goal_cup_erase_member(p_actor_id uuid) RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_consents   integer;
  v_unsent     integer;
  v_sent       integer;
  v_runs       integer;
  v_heartbeats integer;
  v_creds      integer;
  v_email      text;
  v_locked     text[] := '{}';
  v_new        text[];
  v_event      text;
  v_withdrawn  integer;
  v_kept       integer;
  v_entry_team integer;
  v_listings   integer;
BEGIN
  IF p_actor_id IS NULL THEN RAISE EXCEPTION 'goal_cup_erase_member: no member' USING ERRCODE = '22023'; END IF;

  SELECT lower(btrim(u.email)) INTO v_email FROM auth.users u
   WHERE u.id = p_actor_id AND u.email_confirmed_at IS NOT NULL;

  -- Every event a sign-up can appear in, in one fixed order, before any row: a sign-up in flight is waited for, and two
  -- erasures cannot lock in opposite orders.
  FOR v_event IN
    SELECT e.id FROM events e
     WHERE e.signup_open OR e.requires_signup OR EXISTS (SELECT 1 FROM signups s WHERE s.event_id = e.id)
     ORDER BY e.id COLLATE "C"
  LOOP
    PERFORM pg_advisory_xact_lock(hashtext('goal_cup_signup:' || v_event));
    v_locked := v_locked || v_event;
  END LOOP;

  -- Any other event the member's rows touch (none is expected). Re-read until none is new.
  LOOP
    SELECT array_agg(x.event_id ORDER BY x.event_id COLLATE "C") INTO v_new FROM (
      SELECT s.event_id FROM signups s WHERE s.member_id = p_actor_id
      UNION
      SELECT t.event_id FROM teammates t JOIN signups s ON s.id = t.signup_id WHERE s.member_id = p_actor_id
      UNION
      SELECT t.event_id FROM teammates t WHERE t.email_normalized = v_email
    ) x
    WHERE x.event_id <> ALL (v_locked);
    EXIT WHEN v_new IS NULL;
    FOREACH v_event IN ARRAY v_new LOOP
      PERFORM pg_advisory_xact_lock(hashtext('goal_cup_signup:' || v_event));
    END LOOP;
    v_locked := v_locked || v_new;
  END LOOP;

  UPDATE run_credentials SET expires_at = now() WHERE member_id = p_actor_id AND expires_at > now();
  GET DIAGNOSTICS v_creds = ROW_COUNT;

  -- The sign-up rows first (this waits on open_run's FOR SHARE), so the is_frozen reads below see a committed start.
  PERFORM 1 FROM signups WHERE member_id = p_actor_id AND event_id = ANY (v_locked) FOR UPDATE;

  UPDATE signups s SET operator_erased_at = now(), anonymous = true, updated_at = now()
   WHERE s.member_id = p_actor_id AND s.event_id = ANY (v_locked) AND s.operator_erased_at IS NULL
     AND s.withdrawn_at IS NULL AND is_frozen(s.id);
  GET DIAGNOSTICS v_kept = ROW_COUNT;
  UPDATE signups s SET withdrawn_at = now(), operator_erased_at = coalesce(s.operator_erased_at, now()), updated_at = now()
   WHERE s.member_id = p_actor_id AND s.event_id = ANY (v_locked) AND s.withdrawn_at IS NULL AND NOT is_frozen(s.id);
  GET DIAGNOSTICS v_withdrawn = ROW_COUNT;
  -- Already withdrawn sign-ups name nobody either.
  UPDATE signups s SET operator_erased_at = now()
   WHERE s.member_id = p_actor_id AND s.event_id = ANY (v_locked) AND s.operator_erased_at IS NULL;

  DELETE FROM teammates t USING signups s
   WHERE s.id = t.signup_id AND s.member_id = p_actor_id AND t.event_id = ANY (v_locked)
     AND NOT is_frozen(s.id);
  GET DIAGNOSTICS v_entry_team = ROW_COUNT;
  DELETE FROM teammates t WHERE t.email_normalized = v_email AND t.event_id = ANY (v_locked);
  GET DIAGNOSTICS v_listings = ROW_COUNT;

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
  RETURN jsonb_build_object('consents', v_consents, 'unsent_notices', v_unsent, 'sent_notices', v_sent,
                            'runs', v_runs, 'heartbeats', v_heartbeats, 'credentials', v_creds,
                            'signups_withdrawn', v_withdrawn, 'signups_kept', v_kept, 'entry_teammates', v_entry_team,
                            'teammate_listings', v_listings);
END $$;

-- ---- the staging seed ----------------------------------------------------------------------------------------
-- An active sign-up on goalcup-cup-staging for a staging test member (goal_cup_tracer.test_members), for the
-- rehearsal: synthetic members have no account and cannot use the real sign-up. Refuses (raises) anyone else and a
-- name that is not 1-80 characters after trimming without control characters. Takes the event's sign-up lock, then
-- numbers the entry max + 1 like goal_cup_signup. Idempotent: a member who already has an active sign-up there gets
-- its id back, unchanged. No team, no organization, no consent (the rehearsal records terms separately).
CREATE FUNCTION goal_cup_tracer.seed_staging_signup(p_member uuid, p_name text) RETURNS uuid
LANGUAGE plpgsql SECURITY DEFINER SET search_path = goal_cup_tracer, pg_temp
AS $$
DECLARE
  v_event    constant text := 'goalcup-cup-staging';
  v_name     text := btrim(p_name);
  v_id       uuid;
  v_entry_no integer;
BEGIN
  PERFORM pg_advisory_xact_lock(hashtext('goal_cup_signup:' || v_event));
  IF NOT EXISTS (SELECT 1 FROM test_members WHERE member_id = p_member) THEN
    RAISE EXCEPTION 'seed_staging_signup: % is not a staging test member', p_member USING ERRCODE = '42501';
  END IF;
  IF v_name IS NULL OR char_length(v_name) NOT BETWEEN 1 AND 80 OR v_name ~ '[[:cntrl:]]' THEN
    RAISE EXCEPTION 'seed_staging_signup: the name must be 1 to 80 characters' USING ERRCODE = '22023';
  END IF;
  SELECT id INTO v_id FROM signups
   WHERE event_id = v_event AND member_id = p_member AND withdrawn_at IS NULL FOR UPDATE;
  IF FOUND THEN RETURN v_id; END IF;
  SELECT coalesce(max(entry_no), 0) + 1 INTO v_entry_no FROM signups WHERE event_id = v_event;
  INSERT INTO signups (event_id, member_id, entry_no, name) VALUES (v_event, p_member, v_entry_no, v_name)
  RETURNING id INTO v_id;
  RETURN v_id;
END $$;

-- ---- privileges ---------------------------------------------------------------------------------------------
-- Asserted again exactly as 20261015120000 and 20261015150000 grant them (CREATE OR REPLACE keeps an ACL, but this
-- states it): open_run for both Worker roles and service_role; device_request and erase_member for service_role.
-- Supabase grants EXECUTE on new functions to PUBLIC, anon and authenticated; revoke all three explicitly
-- (docs/SUPABASE.md, "SECURITY DEFINER grants").
REVOKE ALL ON FUNCTION
  public.goal_cup_upload_open_run(text, text, text, text, text, text, text, text, text, text, text, text, text, text, text, jsonb, text, text, text, text),
  public.goal_cup_device_request(uuid, text),
  public.goal_cup_erase_member(uuid)
FROM PUBLIC, anon, authenticated;
GRANT EXECUTE ON FUNCTION
  public.goal_cup_upload_open_run(text, text, text, text, text, text, text, text, text, text, text, text, text, text, text, jsonb, text, text, text, text)
TO goal_cup_ingest_staging, goal_cup_ingest_production, service_role;
GRANT EXECUTE ON FUNCTION
  public.goal_cup_device_request(uuid, text),
  public.goal_cup_erase_member(uuid)
TO service_role;

REVOKE ALL ON FUNCTION goal_cup_tracer.seed_staging_signup(uuid, text)
FROM PUBLIC, anon, authenticated, goal_cup_ingest_staging, goal_cup_ingest_production;
GRANT EXECUTE ON FUNCTION goal_cup_tracer.seed_staging_signup(uuid, text) TO service_role;

-- ======================== 20261021120400_goal_cup_terms_2026_10_23.sql ========================
-- Goal Cup Edition 01: the terms version for the 23 Oct rules and privacy texts.
--
-- Spec: docs/specs/2026-10-20-goal-cup-data-flows-design.md; plan: docs/plans/2026-10-21-goal-cup-signup-data-flows.md
-- (Task 5).
--
-- APPLY ONLY AFTER the website with the 23 Oct texts is live. The sign-up form sends back the version it was
-- rendered with (public.goal_cup_current_terms()); once this migration is applied, a member who accepted the old
-- version 'goal-cup-tracer-2026-10-15' no longer has current terms (has_current_terms) and must accept again, so
-- the texts members read must already be the 23 Oct ones.
--
-- Rules/privacy text hash: TBD at apply time (owner)
--
-- Opens production sign-ups: sign-up opens only with the new terms version (spec), so this migration also sets
-- events.signup_open = true for goalcup-cup-2026, which 20261021120000 left false. Every sign-up therefore records the
-- 23 Oct terms. goalcup-cup-staging has been open since 120000. To close sign-ups again later, set signup_open = false.
--
-- Replaces goal_cup_tracer.current_terms_version() from 20261015120000_goal_cup_tracer_upload.sql (~line 225) with the
-- same attributes (LANGUAGE sql IMMUTABLE, SET search_path = pg_catalog, pg_temp); only the returned value changes.
-- That migration grants the function to nobody (REVOKE ALL ON ALL FUNCTIONS IN SCHEMA goal_cup_tracer ...) and
-- CREATE OR REPLACE keeps the ACL; the revoke is asserted again below. No table or column changes; one row update
-- (the signup_open flag on the cup).
--
-- Apply: as docs/admin/GOAL-CUP-SIGNUP-RUNBOOK.md says (one transaction after SET lock_timeout = '5s'); then pnpm db:check.

CREATE OR REPLACE FUNCTION goal_cup_tracer.current_terms_version() RETURNS text
LANGUAGE sql IMMUTABLE SET search_path = pg_catalog, pg_temp
AS $$ SELECT 'goal-cup-2026-10-23'::text $$;

REVOKE ALL ON FUNCTION goal_cup_tracer.current_terms_version() FROM PUBLIC, anon, authenticated;

-- Production sign-ups open together with the new terms version, in the same transaction. Exactly one row must
-- change: zero means the cup row is missing or renamed, so the apply fails instead of leaving sign-ups closed.
DO $open$
DECLARE v_changed integer;
BEGIN
  UPDATE goal_cup_tracer.events SET signup_open = true WHERE id = 'goalcup-cup-2026';
  GET DIAGNOSTICS v_changed = ROW_COUNT;
  IF v_changed <> 1 THEN
    RAISE EXCEPTION 'goal cup terms: opening sign-ups changed % rows of goalcup-cup-2026, expected 1', v_changed;
  END IF;
END $open$;

-- ======================== 20261027100000_goal_cup_tracer_competition_repo_owner.sql ========================
-- Goal Cup Proxy: one competition repository, one member (2026-10-27).
--
-- A member could open a competition run naming another member's new, empty, public repository, and
-- the deadline job would then judge that member's pushed commit as theirs (0.7.0 security review,
-- I2). Now goal_cup_upload_open_run refuses a competition open whose repo_url is already the
-- repository of a competition run of another member in the same event:
--   * the URLs compare by goal_cup_tracer.repo_key: lower case, with a trailing ".git" and trailing
--     "/" removed;
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
AS $$ SELECT regexp_replace(lower(p_repo_url), '(\.git)?/*$', '') $$;

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
