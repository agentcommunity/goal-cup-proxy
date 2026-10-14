-- =====================================================================================
-- PROPOSAL ONLY. NEVER RUN AGAINST SUPABASE.
--
-- It was loaded into a throwaway local PostgreSQL 17 (with stub roles anon, authenticated,
-- authenticator) and its functions were exercised by hand: open, replace, the start window,
-- practice, the credential binding in the same step, continue and its repeat, receipt, commit
-- (stale offset refused), abandon, seal (needs the end and the chain), the close, the device flow
-- (create, poll, approve, deny, expire, pickup once, credential minted, expired), the heartbeat signals, the cron's
-- reads and the notice outbox, and the role's
-- lack of table privileges. That
-- proves the SQL parses and behaves as the Worker expects. It does NOT prove anything about
-- Supabase itself: PostgREST exposure of the schema, the JWT role, RLS defaults.
--
-- The website repo owns the `goalcup` schema, its migrations, the sign-up, the terms checkbox
-- and the approval page for device requests. This file is what the upload Worker (server/worker/) expects
-- to find, written so the website's owner can adopt, change or reject it. Expect to adjust names
-- and types on first contact with the real project.
--
-- Matches docs/SERVER-DESIGN.md section 7 and the TypeScript in src/db-supabase.ts, which
-- calls each function through PostgREST: POST {SUPABASE_URL}/rest/v1/rpc/<name> with
-- `Content-Profile: goalcup`. (Hence the schema must be in the project's "Exposed schemas"
-- setting; the grants below, not the exposure, are what keep it private.)
--
-- Conventions
--   * Times are timestamptz in the database; the Worker passes and receives RFC 3339 UTC
--     strings to the second, and the functions format them with to_char below.
--   * Every function is SECURITY DEFINER with a fixed search_path and is executable only by the
--     role `goalcup_worker`. PostgreSQL grants EXECUTE on a new function to PUBLIC by default, so
--     the `revoke ... on all functions` below runs AFTER the last `create function` (a revoke that
--     runs before the functions exist revokes nothing); it is what keeps `anon` and `authenticated`
--     out. `alter default privileges` near the top covers functions added later by this role.
--     `goalcup_worker` has NO table privileges, so a leaked Worker key can call these functions
--     and nothing else.
--   * Row-level security is enabled on every table in this same file; no policies are created
--     here, so no role other than the table owner (which the definer functions run as) can read
--     or write. Members read their own runs through a separate read-only function the website
--     owns (not part of the Worker's needs).
--   * Every function that takes a run takes the member as well and returns "not found" for a
--     run of another member, so a credential cannot probe for other members' runs.
-- =====================================================================================

create schema if not exists goalcup;

-- Belt and braces for functions added to this schema later. The load-bearing step is the
-- explicit revoke after the last `create function` below: a revoke on "all functions" only
-- touches functions that already exist.
alter default privileges in schema goalcup revoke execute on functions from public;
revoke usage on schema goalcup from anon, authenticated;

-- ---- tables ---------------------------------------------------------------------------

create table goalcup.events (
  id                   text primary key check (id ~ '^[a-z0-9][a-z0-9-]{0,63}$'),
  mode                 text not null check (mode in ('competition', 'dry_run', 'practice')),
  start_window_opens   timestamptz not null,
  start_window_closes  timestamptz not null,
  run_hours            integer not null default 168   -- not used yet (the Worker has RUN_HOURS); no grace period: at the end the run is over
);

-- Which version of the terms a member accepted. The website's consent flow writes this;
-- the Worker only asks "has this member accepted the current terms?" (inside upload_open_run).
create table goalcup.consents (
  member_id     uuid not null,
  terms_version text not null,
  accepted_at   timestamptz not null default now(),
  primary key (member_id, terms_version)
);

-- The device flow: the tracer asks (POST /v1/device), the member approves on the website, the
-- tracer picks up a run credential once. Only the sha256 of the codes is stored.
create table goalcup.device_requests (
  device_hash    text primary key check (device_hash ~ '^[0-9a-f]{64}$'),
  user_hash      text not null unique check (user_hash ~ '^[0-9a-f]{64}$'),   -- of the 8 letters, upper case, no dash
  purpose        text not null check (purpose in ('login', 'run')),
  mode           text not null check (mode in ('competition', 'practice')),
  connector_id   text not null,
  tracer_version text not null,
  created_at     timestamptz not null,
  expires_at     timestamptz not null,                                       -- created_at + 900 s, for every state
  status         text not null default 'pending' check (status in ('pending', 'approved', 'denied')),
  member_id      uuid,
  decided_at     timestamptz,
  picked_up_at   timestamptz                                                 -- set by the one pickup
);

-- A run: what an account started. Its chain, receipt and seal live on its entries (segments).
create table goalcup.runs (
  id                   text primary key check (id ~ '^r[0-9a-f]{16}$'),
  event_id             text not null references goalcup.events (id),
  member_id            uuid not null,
  mode                 text not null check (mode in ('competition', 'practice')),   -- from the credential
  official_start       timestamptz not null,
  deadline_at          timestamptz not null,
  -- Set when the account's later competition start replaced this run: the run ends then, if that
  -- is before deadline_at (ends_at = least(deadline_at, replaced_at)).
  replaced_at          timestamptz,
  -- 'sealed' once over with its current entry sealed; 'incomplete' 24 hours after it ended with that entry
  -- unsealed (the last piece is missing); 'abandoned' only by the participant's own call, before any chunk.
  status               text not null default 'open' check (status in ('open', 'sealed', 'incomplete', 'abandoned')),
  repo_url             text not null,
  machine              jsonb,                 -- non-identifying facts the tracer sent (OS, CPUs, memory...); never public
  client_ip            text,                  -- from the open call's request, kept on the server only
  client_country       text,
  last_client_ip       text,                  -- the latest pair, from a heartbeat or a continue (a missing one keeps the last)
  last_client_country  text,
  abandon_reason       text,
  sealed_at            timestamptz,
  last_heartbeat_at    timestamptz,           -- the latest heartbeat of any of its entries
  -- Facts from the heartbeats (docs/UPLOAD-API.md, "What the server does on its own").
  last_model_request_at timestamptz,          -- when an entry's model_requests was first reported or last grew; null until a heartbeat reports it
  flagged_at           timestamptz,           -- the first heartbeat that said the recording had gaps or failed
  flag_reason          text check (flag_reason in ('gaps', 'failed')),
  -- The commit GitHub had at the deadline: 40 hex, or 'none' when GitHub has no such repository. Null until asked.
  final_commit         text check (final_commit ~ '^([0-9a-f]{40}|none)$'),
  final_commit_at      timestamptz,
  created_at           timestamptz not null default now()
);

-- One main competition run per member, enforced by the database, so two simultaneous starts cannot
-- both win: a later start sets replaced_at on the earlier one in the same step that inserts.
create unique index runs_one_main_per_member on goalcup.runs (member_id)
  where status = 'open' and mode = 'competition' and replaced_at is null;

-- A run credential replaces the pasted member token. Minted at pickup (upload_poll_device), only its
-- sha256 is stored. It stands for one member; until it opens a run it is unbound and lasts one hour
-- from the approval, and once bound to a run it lasts until 24 hours after that run's deadline.
create table goalcup.run_credentials (
  credential_hash text primary key check (credential_hash ~ '^[0-9a-f]{64}$'),
  member_id       uuid not null,
  mode            text not null check (mode in ('competition', 'practice')),
  run_id          text references goalcup.runs (id),
  expires_at      timestamptz not null,
  created_at      timestamptz not null default now()
);

-- One segment of a run. Segment 0 is made by POST /v1/runs, the next ones by POST .../continue.
create table goalcup.entries (
  id            text primary key check (id ~ '^e[0-9a-f]{16}$'),   -- the entry_id
  run_id        text not null references goalcup.runs (id),
  segment       integer not null default 0,
  opened_at     timestamptz not null default now(),
  -- What the open (or continue) call sent; the receipt signs these.
  connector_id         text not null,
  manifest_sha256      text not null,
  connector_dir_sha256 text not null,
  workspace            text not null,
  session_id           text not null,
  harness_version      text not null,
  tracer_version       text not null,
  receipt       jsonb,                                              -- the signed StartReceipt, once issued
  upload_offset bigint not null default 0,
  next_seq      bigint not null default 0,                          -- the first record (the baseline) has seq 0
  head          text not null default repeat('0', 64),              -- genesis before any record
  last_ack      jsonb,
  last_chunk_at timestamptz,
  model_requests bigint,                                              -- the highest count this entry's heartbeats sent (each process counts from 0); null until one does
  -- The end of the entry, committed once by the final heartbeat: the last record the server will accept.
  final_seq       bigint,
  final_head      text,
  final_repo_head text,                                                -- 40 hex, or null
  final_at        timestamptz,
  seal_sha256     text,
  baseline_sha256 text,
  seal_key        text,                                                -- R2 keys of the stored seal.json / baseline.json
  baseline_key    text,
  sealed_at       timestamptz,
  check ((final_seq is null) = (final_head is null)),
  unique (run_id, segment)
);

-- One row per heartbeat (every 5 minutes per run: a few million rows over an event).
create table goalcup.heartbeats (
  run_id       text not null references goalcup.runs (id),
  entry_id     text not null references goalcup.entries (id),
  received_at  timestamptz not null,
  seq          bigint not null,
  head         text not null,
  offset_local bigint not null,
  final        boolean not null,
  repo_head    text,
  recording    text check (recording in ('ok', 'gaps', 'failed', 'unknown')),   -- as the tracer saw it; null if not sent
  client_ip      text,                   -- from the request, kept on the server only
  client_country text
);
create index heartbeats_run_received on goalcup.heartbeats (run_id, received_at);

-- The outbox of run emails. The Worker's cron queues a row; the website's mailer sends the email and
-- sets sent_at. The Worker never sends mail and never reads this table back. A notice is queued once
-- per (run, kind, dedupe_key): the key is what makes it the same notice again (the time of the last
-- heartbeat for 'silence', the time of the last model request for 'idle_*', empty for the others).
create table goalcup.notices (
  id          bigint generated always as identity primary key,
  run_id      text not null references goalcup.runs (id),
  member_id   uuid not null,
  kind        text not null check (kind in ('started', 'silence', 'idle_3h', 'idle_6h', 'recording', 'ended')),
  dedupe_key  text not null,
  payload     jsonb not null default '{}',   -- what the email needs: times, hours, the commit, the repo
  created_at  timestamptz not null,
  sent_at     timestamptz,
  unique (run_id, kind, dedupe_key)
);
create index notices_unsent on goalcup.notices (created_at) where sent_at is null;

create table goalcup.chunks (
  entry_id     text not null references goalcup.entries (id),
  "offset"     bigint not null,
  length       bigint not null check (length > 0),
  sha256       text not null,
  first_seq    bigint not null,
  last_seq     bigint not null,
  prev         text not null,
  head         text not null,
  r2_key       text not null,
  received_at  timestamptz not null,
  ack          jsonb not null,
  primary key (entry_id, "offset")
);

-- Other tables from SERVER-DESIGN section 7 (outages, verify_results) belong to later slices.

alter table goalcup.events   enable row level security;
alter table goalcup.consents enable row level security;
alter table goalcup.device_requests enable row level security;
alter table goalcup.run_credentials enable row level security;
alter table goalcup.runs     enable row level security;
alter table goalcup.entries  enable row level security;
alter table goalcup.chunks   enable row level security;
alter table goalcup.heartbeats enable row level security;
alter table goalcup.notices  enable row level security;

revoke all on all tables in schema goalcup from public, anon, authenticated;

-- ---- the Worker's role ------------------------------------------------------------------
-- The Worker's SUPABASE_KEY is a JWT whose `role` claim is goalcup_worker, signed with the
-- project's JWT secret (the owner mints it; it is not the community-wide service key).
do $$
begin
  if not exists (select 1 from pg_roles where rolname = 'goalcup_worker') then
    create role goalcup_worker nologin noinherit;
  end if;
end $$;
grant goalcup_worker to authenticator;           -- lets PostgREST switch to it
grant usage on schema goalcup to goalcup_worker; -- schema usage only; no table grants

-- ---- helpers -----------------------------------------------------------------------------

create function goalcup.fmt_time(t timestamptz) returns text
language sql immutable
as $$ select to_char(t at time zone 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"') $$;

-- The run and one of its entries as the Worker's {run: RunRow, entry: EntryRow} (src/db.ts): the entry
-- named by p_entry_id (null if it is not this run's), or the current one (the highest segment) if none.
create function goalcup.run_json(p_run_id text, p_entry_id text default null) returns jsonb
language sql stable security definer set search_path = goalcup, pg_temp
as $$
  select jsonb_build_object(
    'run', jsonb_build_object(
      'run_id',            r.id,
      'event_id',          r.event_id,
      'member_id',         r.member_id::text,
      'mode',              r.mode,
      'status',            r.status,
      'official_start',    goalcup.fmt_time(r.official_start),
      'deadline_at',       goalcup.fmt_time(r.deadline_at),
      'replaced_at',       goalcup.fmt_time(r.replaced_at),
      'repo_url',          r.repo_url,
      'first_entry_id',    first.id,
      'current_entry_id',  cur.id,
      'current_sealed_at', goalcup.fmt_time(cur.sealed_at)),
    'entry', (
      select jsonb_build_object(
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
        'final',                case when e.final_seq is null then null
                                     else jsonb_build_object('seq', e.final_seq, 'head', e.final_head) end,
        'receipt',              e.receipt,
        'seal_sha256',          e.seal_sha256,
        'baseline_sha256',      e.baseline_sha256,
        'sealed_at',            goalcup.fmt_time(e.sealed_at))
      from goalcup.entries e
      where e.run_id = r.id and e.id = coalesce(p_entry_id, cur.id))
  )
  from goalcup.runs r
  cross join lateral (select e.id, e.sealed_at from goalcup.entries e where e.run_id = r.id order by e.segment desc limit 1) cur
  cross join lateral (select e.id from goalcup.entries e where e.run_id = r.id and e.segment = 0) first
  where r.id = p_run_id
$$;

-- What the run's status is at p_at: 'sealed' once it is over with its current entry sealed, 'incomplete'
-- once it has been over for 24 hours with that entry unsealed. A status already written (abandoned,
-- sealed, incomplete) stays. The same rule as statusAt in src/db.ts.
create function goalcup.run_status_at(p_run_id text, p_at timestamptz) returns text
language sql stable security definer set search_path = goalcup, pg_temp
as $$
  select case
    when r.status <> 'open' or p_at < least(r.deadline_at, coalesce(r.replaced_at, r.deadline_at)) then r.status
    when cur.sealed_at is not null then 'sealed'
    when p_at > least(r.deadline_at, coalesce(r.replaced_at, r.deadline_at)) + interval '24 hours' then 'incomplete'
    else 'open'
  end
  from goalcup.runs r
  cross join lateral (select e.sealed_at from goalcup.entries e where e.run_id = r.id order by e.segment desc limit 1) cur
  where r.id = p_run_id
$$;

-- What a recorded heartbeat adds to its run (the same rules as withSignals in src/db-local.ts): the time, the
-- latest connection (a missing one keeps the last), the first flag, and an entry's first or higher
-- model_requests (each entry counts from zero, so a continued process is not held to the old maximum), which
-- moves the run's last model request.
create function goalcup.note_heartbeat(
  p_run_id text, p_entry_id text, p_at timestamptz, p_recording text, p_model_requests bigint, p_client_ip text, p_client_country text
) returns void
language plpgsql security definer set search_path = goalcup, pg_temp
as $$
declare
  v_grew boolean := false;
begin
  if p_model_requests is not null then
    update goalcup.entries set model_requests = p_model_requests
     where id = p_entry_id and (model_requests is null or p_model_requests > model_requests);
    v_grew := found;
  end if;
  update goalcup.runs set
    last_heartbeat_at     = p_at,
    last_client_ip        = coalesce(p_client_ip, last_client_ip),
    last_client_country   = coalesce(p_client_country, last_client_country),
    flagged_at            = case when flagged_at is null and p_recording in ('gaps', 'failed') then p_at else flagged_at end,
    flag_reason           = case when flagged_at is null and p_recording in ('gaps', 'failed') then p_recording else flag_reason end,
    last_model_request_at = case when v_grew then p_at else last_model_request_at end
  where id = p_run_id;
end $$;

-- ---- the device flow and the run credential ----------------------------------------------

-- Credential (sha256 hex) -> {"member_id", "run_id" (null until it opens a run), "mode", "expires_at"},
-- or null if unknown or past its expiry. Every authenticated call starts here.
create function goalcup.upload_credential(p_credential_hash text, p_now text) returns jsonb
language sql stable security definer set search_path = goalcup, pg_temp
as $$
  select jsonb_build_object('member_id', c.member_id::text, 'run_id', c.run_id, 'mode', c.mode,
                            'expires_at', goalcup.fmt_time(c.expires_at))
    from goalcup.run_credentials c
   where c.credential_hash = p_credential_hash and p_now::timestamptz < c.expires_at
$$;

-- POST /v1/device. A repeated hash is a unique violation (the Worker answers 502 and the tracer retries).
create function goalcup.upload_create_device(
  p_device_hash text, p_user_hash text, p_purpose text, p_mode text,
  p_connector_id text, p_tracer_version text, p_created_at text, p_expires_at text
) returns text
language plpgsql security definer set search_path = goalcup, pg_temp
as $$
begin
  insert into goalcup.device_requests (device_hash, user_hash, purpose, mode, connector_id, tracer_version, created_at, expires_at)
  values (p_device_hash, p_user_hash, p_purpose, p_mode, p_connector_id, p_tracer_version, p_created_at::timestamptz, p_expires_at::timestamptz);
  return 'created';
end $$;

-- PROPOSAL: the account handle a login request hands back. The website replaces this body with its
-- member/profile handle column.
create function goalcup.member_handle(p_member uuid) returns text
language sql stable security definer set search_path = goalcup, pg_temp
as $$ select p_member::text $$;

-- POST /v1/device/token. {"result": "pending"|"approved"|"denied"|"expired", ...}. "approved" comes back
-- once: a run request mints its credential here ("credential" is the only plain copy, "expires_at" is
-- the approval plus one hour), a login request returns the member's "handle". Every later poll, and any
-- poll at or after the request's expiry, is "expired".
create function goalcup.upload_poll_device(p_device_hash text, p_now text) returns jsonb
language plpgsql security definer set search_path = goalcup, pg_temp
as $$
declare
  v_now     timestamptz := p_now::timestamptz;
  v_dev     goalcup.device_requests;
  v_plain   text;
  v_expires timestamptz;
begin
  select * into v_dev from goalcup.device_requests where device_hash = p_device_hash for update;
  if not found or v_now >= v_dev.expires_at or v_dev.picked_up_at is not null then
    return jsonb_build_object('result', 'expired');
  end if;
  if v_dev.status = 'pending' then return jsonb_build_object('result', 'pending'); end if;
  if v_dev.status = 'denied' then return jsonb_build_object('result', 'denied'); end if;

  update goalcup.device_requests set picked_up_at = v_now where device_hash = p_device_hash;
  if v_dev.purpose = 'login' then
    return jsonb_build_object('result', 'approved', 'purpose', 'login', 'handle', goalcup.member_handle(v_dev.member_id));
  end if;

  -- "gcr_" and 64 lowercase hex digits (two UUIDs without dashes; gen_random_uuid() needs no extension).
  v_plain := 'gcr_' || replace(gen_random_uuid()::text || gen_random_uuid()::text, '-', '');
  v_expires := v_dev.decided_at + interval '1 hour';
  insert into goalcup.run_credentials (credential_hash, member_id, mode, expires_at)
  values (encode(sha256(convert_to(v_plain, 'UTF8')), 'hex'), v_dev.member_id, v_dev.mode, v_expires);
  return jsonb_build_object('result', 'approved', 'purpose', 'run', 'credential', v_plain,
                            'expires_at', goalcup.fmt_time(v_expires));
end $$;

-- Decide a pending request by its user code (the Worker passes 8 upper-case letters; the dash and case are
-- also accepted here). "ok" | "not_found" | "expired" | "already_decided".
create function goalcup.upload_approve_device(p_user_code text, p_member_id text, p_approve boolean, p_now text) returns text
language plpgsql security definer set search_path = goalcup, pg_temp
as $$
declare
  v_now timestamptz := p_now::timestamptz;
  v_dev goalcup.device_requests;
begin
  select * into v_dev from goalcup.device_requests
   where user_hash = encode(sha256(convert_to(upper(replace(p_user_code, '-', '')), 'UTF8')), 'hex')
     for update;
  if not found then return 'not_found'; end if;
  if v_now >= v_dev.expires_at then return 'expired'; end if;
  if v_dev.status <> 'pending' then return 'already_decided'; end if;
  update goalcup.device_requests
     set status = case when p_approve then 'approved' else 'denied' end,
         member_id = p_member_id::uuid, decided_at = v_now
   where device_hash = v_dev.device_hash;
  return 'ok';
end $$;

-- PROPOSAL, for the website's approval page. The page has signed the member in and passes the member's
-- id from its own session. Not granted to goalcup_worker (the Worker's own route is the test-only
-- upload_approve_device above, reachable only with STAGING=on or DB_BACKEND=local); the website grants
-- it to whichever role its server uses.
create function goalcup.goalcup_approve_device(p_user_code text, p_member_id uuid, p_approve boolean) returns text
language sql security definer set search_path = goalcup, pg_temp
as $$
  select goalcup.upload_approve_device(p_user_code, p_member_id::text, p_approve, goalcup.fmt_time(now()))
$$;

-- ---- the run calls -------------------------------------------------------------------------

-- POST /v1/runs. Returns {"result": "created"|"existing"|"conflict"|"forbidden", "run": {...}, "entry": {...}, "message": "..."}.
-- "existing": the credential is bound to a run whose segment 0 has this session (a retry that raced the original).
-- One transaction: a competition start replaces the member's other open competition run, the new run and its
-- segment 0 are inserted, and the credential is bound to the run. A credential that is unknown or already
-- bound is "conflict" and nothing is written.
create function goalcup.upload_open_run(
  p_member_id text, p_event_id text, p_mode text, p_run_id text, p_entry_id text,
  p_official_start text, p_deadline_at text,
  p_connector_id text, p_manifest_sha256 text, p_connector_dir_sha256 text,
  p_workspace text, p_session_id text, p_harness_version text, p_tracer_version text, p_repo_url text,
  p_machine jsonb, p_client_ip text, p_client_country text,
  p_credential_hash text, p_credential_expires_at text
) returns jsonb
language plpgsql security definer set search_path = goalcup, pg_temp
as $$
declare
  v_member uuid := p_member_id::uuid;
  v_event  goalcup.events;
  v_start  timestamptz := p_official_start::timestamptz;
  v_bound  text;
  v_try    integer;
begin
  select * into v_event from goalcup.events where id = p_event_id;
  if not found then
    return jsonb_build_object('result', 'forbidden', 'message', 'This event is not open for runs.');
  end if;

  -- Eligibility. (A lost reply never reaches here: the Worker answers a repeat from the bound credential.)
  if not exists (select 1 from goalcup.consents where member_id = v_member) then
    return jsonb_build_object('result', 'forbidden', 'message', 'Accept the Goal Cup terms on the website first.');
  end if;
  -- Practice has no start window.
  if p_mode = 'competition' then
    if v_start < v_event.start_window_opens then
      return jsonb_build_object('result', 'forbidden', 'message', 'The start window has not opened yet.');
    end if;
    if v_start > v_event.start_window_closes then
      return jsonb_build_object('result', 'forbidden',
                                'message', 'after the start window you can only `goalcup continue` your run');
    end if;
  end if;

  -- The credential binds in this same transaction: take its row, and refuse one that is unknown or bound.
  select run_id into v_bound from goalcup.run_credentials where credential_hash = p_credential_hash for update;
  if not found then
    return jsonb_build_object('result', 'conflict');
  end if;
  if v_bound is not null then
    -- A retry that raced the original open finds the credential bound to the run it made.
    if exists (select 1 from goalcup.runs r join goalcup.entries e on e.run_id = r.id and e.segment = 0
                where r.id = v_bound and r.member_id = v_member and e.session_id = p_session_id) then
      return jsonb_build_object('result', 'existing')
             || goalcup.run_json(v_bound, (select e.id from goalcup.entries e where e.run_id = v_bound and e.segment = 0));
    end if;
    return jsonb_build_object('result', 'conflict');
  end if;

  -- A later competition start replaces the member's other open competition run (it ends now). Two starts
  -- racing: the unique index lets one insert through; the other goes round once more and replaces it.
  for v_try in 1..2 loop
    if p_mode = 'competition' then
      update goalcup.runs set replaced_at = greatest(v_start, official_start)   -- never before it began
       where member_id = v_member and mode = 'competition' and status = 'open' and replaced_at is null;
    end if;
    begin
      insert into goalcup.runs (id, event_id, member_id, mode, official_start, deadline_at, repo_url,
                                machine, client_ip, client_country)
      values (p_run_id, p_event_id, v_member, p_mode, v_start, p_deadline_at::timestamptz, p_repo_url,
              nullif(p_machine, 'null'::jsonb), p_client_ip, p_client_country);   -- a JSON null is no facts
      exit;
    exception when unique_violation then
      if v_try = 2 then raise; end if;
    end;
  end loop;

  insert into goalcup.entries (id, run_id, segment, connector_id, manifest_sha256, connector_dir_sha256,
                               workspace, session_id, harness_version, tracer_version)
  values (p_entry_id, p_run_id, 0, p_connector_id, p_manifest_sha256, p_connector_dir_sha256,
          p_workspace, p_session_id, p_harness_version, p_tracer_version);
  update goalcup.run_credentials
     set run_id = p_run_id, expires_at = p_credential_expires_at::timestamptz
   where credential_hash = p_credential_hash;
  return jsonb_build_object('result', 'created') || goalcup.run_json(p_run_id);
end $$;

-- POST /v1/runs/{id}/continue. Returns {"result": "created"|"existing"|"conflict"|"closed"|"not_found",
-- "run": {...}, "entry": {...}} (the new, or the existing, current entry). "existing" is the same call again
-- (the current entry came from this session, after p_previous_entry_id); "conflict" is a previous entry that
-- is not the latest; "closed" is a run that is over (at or after ends_at), abandoned or closed.
create function goalcup.upload_continue_run(
  p_member_id text, p_run_id text, p_previous_entry_id text, p_entry_id text,
  p_connector_id text, p_manifest_sha256 text, p_connector_dir_sha256 text, p_workspace text,
  p_session_id text, p_harness_version text, p_tracer_version text,
  p_client_ip text, p_client_country text, p_at text
) returns jsonb
language plpgsql security definer set search_path = goalcup, pg_temp
as $$
declare
  v_run goalcup.runs;
  v_cur goalcup.entries;
  v_at  timestamptz := p_at::timestamptz;
begin
  select * into v_run from goalcup.runs where id = p_run_id and member_id = p_member_id::uuid for update;
  if not found then return jsonb_build_object('result', 'not_found'); end if;
  select * into v_cur from goalcup.entries where run_id = p_run_id order by segment desc limit 1;

  if v_cur.segment > 0 and v_cur.session_id = p_session_id
     and exists (select 1 from goalcup.entries
                  where run_id = p_run_id and segment = v_cur.segment - 1 and id = p_previous_entry_id) then
    return jsonb_build_object('result', 'existing') || goalcup.run_json(p_run_id);
  end if;
  if v_run.status <> 'open' or v_at >= least(v_run.deadline_at, coalesce(v_run.replaced_at, v_run.deadline_at)) then
    return jsonb_build_object('result', 'closed');
  end if;
  if v_cur.id <> p_previous_entry_id then
    return jsonb_build_object('result', 'conflict');
  end if;

  insert into goalcup.entries (id, run_id, segment, connector_id, manifest_sha256, connector_dir_sha256,
                               workspace, session_id, harness_version, tracer_version)
  values (p_entry_id, p_run_id, v_cur.segment + 1, p_connector_id, p_manifest_sha256, p_connector_dir_sha256,
          p_workspace, p_session_id, p_harness_version, p_tracer_version);
  update goalcup.runs set last_client_ip = coalesce(p_client_ip, last_client_ip),
                          last_client_country = coalesce(p_client_country, last_client_country) where id = p_run_id;
  return jsonb_build_object('result', 'created') || goalcup.run_json(p_run_id);
end $$;

-- GET /v1/runs/{id} (and the lookup every other call starts with): {"run": {...}, "entry": {...}|null}, the
-- entry named, or the current one when p_entry_id is null. Null if the run is not this member's.
create function goalcup.upload_run_position(p_member_id text, p_run_id text, p_entry_id text default null) returns jsonb
language sql stable security definer set search_path = goalcup, pg_temp
as $$
  select goalcup.run_json(r.id, p_entry_id) from goalcup.runs r where r.id = p_run_id and r.member_id = p_member_id::uuid
$$;

-- The stored chunk of the entry that starts at p_offset, in the Worker's ChunkRecord shape, or null.
create function goalcup.upload_get_chunk(p_member_id text, p_run_id text, p_entry_id text, p_offset bigint) returns jsonb
language sql stable security definer set search_path = goalcup, pg_temp
as $$
  select jsonb_build_object(
    'entry_id', c.entry_id, 'offset', c."offset", 'length', c.length, 'sha256', c.sha256,
    'first_seq', c.first_seq, 'last_seq', c.last_seq, 'prev', c.prev, 'head', c.head,
    'r2_key', c.r2_key, 'received_at', goalcup.fmt_time(c.received_at), 'ack', c.ack)
  from goalcup.chunks c
  join goalcup.entries e on e.id = c.entry_id
  join goalcup.runs r on r.id = e.run_id
  where r.id = p_run_id and r.member_id = p_member_id::uuid and c.entry_id = p_entry_id and c."offset" = p_offset
$$;

-- POST /v1/runs/{id}/receipt. The Worker signs; this stores, on the named entry.
-- {"result": "stored"|"existing"|"conflict"|"closed"|"not_found", "receipt": {...}}
create function goalcup.upload_issue_receipt(p_member_id text, p_run_id text, p_entry_id text, p_chain_head text, p_receipt jsonb) returns jsonb
language plpgsql security definer set search_path = goalcup, pg_temp
as $$
declare
  v_run   goalcup.runs;
  v_entry goalcup.entries;
begin
  select * into v_run from goalcup.runs where id = p_run_id and member_id = p_member_id::uuid for update;
  if not found then return jsonb_build_object('result', 'not_found'); end if;
  select * into v_entry from goalcup.entries where id = p_entry_id and run_id = p_run_id for update;
  if not found then return jsonb_build_object('result', 'not_found'); end if;
  if v_run.status in ('abandoned', 'incomplete') or v_entry.sealed_at is not null then
    return jsonb_build_object('result', 'closed');
  end if;
  if v_entry.receipt is not null then
    if v_entry.receipt -> 'claims' ->> 'chain_head' = p_chain_head then
      return jsonb_build_object('result', 'existing', 'receipt', v_entry.receipt);
    end if;
    return jsonb_build_object('result', 'conflict');
  end if;
  update goalcup.entries set receipt = p_receipt where id = p_entry_id;
  return jsonb_build_object('result', 'stored', 'receipt', p_receipt);
end $$;

-- POST /v1/runs/{id}/abandon. Returns 'abandoned' | 'has_chunks' | 'closed' | 'not_found' (a JSON string).
create function goalcup.upload_abandon_run(p_member_id text, p_run_id text, p_reason text, p_at text) returns text
language plpgsql security definer set search_path = goalcup, pg_temp
as $$
declare
  v_run goalcup.runs;
begin
  select * into v_run from goalcup.runs where id = p_run_id and member_id = p_member_id::uuid for update;
  if not found then return 'not_found'; end if;
  if v_run.status <> 'open' then return 'closed'; end if;
  if exists (select 1 from goalcup.chunks c join goalcup.entries e on e.id = c.entry_id where e.run_id = p_run_id) then
    return 'has_chunks';
  end if;
  update goalcup.runs set status = 'abandoned', abandon_reason = p_reason, sealed_at = p_at::timestamptz where id = p_run_id;
  return 'abandoned';
end $$;

-- PUT /v1/runs/{id}/chunks/{offset}, after the bytes are in R2. THE atomic step: advances the
-- entry only if offset, head (prev) and next_seq (first_seq) still are what the Worker checked.
-- p_chunk is the Worker's ChunkRecord. Returns 'committed' | 'conflict' | 'beyond_final' | 'closed' | 'not_found'.
create function goalcup.upload_commit_chunk(p_member_id text, p_run_id text, p_chunk jsonb) returns text
language plpgsql security definer set search_path = goalcup, pg_temp
as $$
declare
  v_run   goalcup.runs;
  v_entry goalcup.entries;
  v_off   bigint := (p_chunk ->> 'offset')::bigint;
  v_len   bigint := (p_chunk ->> 'length')::bigint;
begin
  -- Lock order everywhere (commit, abandon, seal, close): the run first, then its entry. So a commit
  -- racing an abandon or a seal queues behind it and then sees the new status.
  select * into v_run from goalcup.runs where id = p_run_id and member_id = p_member_id::uuid for update;
  if not found then return 'not_found'; end if;

  -- Concurrent commits for the same run queue on the run lock above; the loser re-checks below.
  select * into v_entry from goalcup.entries
    where id = p_chunk ->> 'entry_id' and run_id = p_run_id for update;
  if not found then return 'not_found'; end if;
  -- A run that is 'sealed' still takes the wrap-up of an older segment; the Worker enforces the 24 hours.
  if v_run.status in ('abandoned', 'incomplete') or v_entry.sealed_at is not null then return 'closed'; end if;

  -- Nothing past the end the final heartbeat committed, whatever the Worker saw a moment ago.
  if v_entry.final_seq is not null
     and ((p_chunk ->> 'last_seq')::bigint > v_entry.final_seq
          or ((p_chunk ->> 'last_seq')::bigint = v_entry.final_seq and p_chunk ->> 'head' <> v_entry.final_head)) then
    return 'beyond_final';
  end if;

  if v_entry.upload_offset <> v_off
     or v_entry.head <> p_chunk ->> 'prev'
     or v_entry.next_seq <> (p_chunk ->> 'first_seq')::bigint then
    return 'conflict';
  end if;

  insert into goalcup.chunks (entry_id, "offset", length, sha256, first_seq, last_seq, prev, head, r2_key, received_at, ack)
  values (v_entry.id, v_off, v_len, p_chunk ->> 'sha256', (p_chunk ->> 'first_seq')::bigint,
          (p_chunk ->> 'last_seq')::bigint, p_chunk ->> 'prev', p_chunk ->> 'head', p_chunk ->> 'r2_key',
          (p_chunk ->> 'received_at')::timestamptz, p_chunk -> 'ack');

  update goalcup.entries
     set upload_offset = v_off + v_len,
         next_seq      = (p_chunk ->> 'last_seq')::bigint + 1,
         head          = p_chunk ->> 'head',
         last_ack      = p_chunk -> 'ack',
         last_chunk_at = (p_chunk ->> 'received_at')::timestamptz
   where id = v_entry.id;
  return 'committed';
end $$;

-- POST /v1/runs/{id}/seal, after seal.json / baseline.json are in R2 under p_seal_key / p_baseline_key.
-- Seals the named entry. {"result": "sealed"|"already_sealed" (same hashes)|"conflict" (other hashes)|
-- "no_final" (the entry has no committed end)|"chain_behind" (the server does not hold the chain up to it)|
-- "closed"|"not_found", "run_status": ...}. The two checks run here, under the entry lock, so a chunk or a
-- final heartbeat cannot slip between the Worker's look and the seal. p_sealed_at is the Worker's "now": the
-- run becomes 'sealed' in this step when this is its current entry and the run is over.
create function goalcup.upload_seal_run(
  p_member_id text, p_run_id text, p_entry_id text,
  p_seal_sha256 text, p_baseline_sha256 text, p_seal_key text, p_baseline_key text, p_sealed_at text
) returns jsonb
language plpgsql security definer set search_path = goalcup, pg_temp
as $$
declare
  v_run    goalcup.runs;
  v_entry  goalcup.entries;
  v_at     timestamptz := p_sealed_at::timestamptz;
  v_status text;
begin
  select * into v_run from goalcup.runs where id = p_run_id and member_id = p_member_id::uuid for update;
  if not found then return jsonb_build_object('result', 'not_found'); end if;
  select * into v_entry from goalcup.entries where id = p_entry_id and run_id = p_run_id for update;
  if not found then return jsonb_build_object('result', 'not_found'); end if;

  if v_entry.sealed_at is not null then
    if v_entry.seal_sha256 = p_seal_sha256 and v_entry.baseline_sha256 = p_baseline_sha256 then
      return jsonb_build_object('result', 'already_sealed', 'run_status', goalcup.run_status_at(p_run_id, v_at));
    end if;
    return jsonb_build_object('result', 'conflict');
  end if;
  if v_run.status in ('abandoned', 'incomplete') then return jsonb_build_object('result', 'closed'); end if;
  if v_entry.final_seq is null then return jsonb_build_object('result', 'no_final'); end if;
  if v_entry.next_seq - 1 <> v_entry.final_seq or v_entry.head <> v_entry.final_head then
    return jsonb_build_object('result', 'chain_behind');
  end if;

  update goalcup.entries
     set seal_sha256 = p_seal_sha256, baseline_sha256 = p_baseline_sha256,
         seal_key = p_seal_key, baseline_key = p_baseline_key, sealed_at = v_at
   where id = p_entry_id;
  v_status := goalcup.run_status_at(p_run_id, v_at);
  if v_status <> v_run.status then
    update goalcup.runs set status = v_status, sealed_at = case when v_status = 'sealed' then v_at else sealed_at end
     where id = p_run_id;
  end if;
  return jsonb_build_object('result', 'sealed', 'run_status', v_status);
end $$;

-- POST /v1/runs/{id}/heartbeat. Records the heartbeat (and its signals, see note_heartbeat); a final one
-- commits the end of the entry (once).
-- Returns 'recorded' | 'final_committed' | 'final_existing' | 'final_conflict' | 'closed' | 'not_found'.
-- Lock order as everywhere: the run, then its entry.
create function goalcup.upload_heartbeat(
  p_member_id text, p_run_id text, p_entry_id text,
  p_seq bigint, p_head text, p_offset_local bigint, p_final boolean, p_repo_head text,
  p_recording text, p_model_requests bigint, p_client_ip text, p_client_country text, p_at text
) returns text
language plpgsql security definer set search_path = goalcup, pg_temp
as $$
declare
  v_run    goalcup.runs;
  v_entry  goalcup.entries;
  v_at     timestamptz := p_at::timestamptz;
  v_stored bigint;
begin
  select * into v_run from goalcup.runs where id = p_run_id and member_id = p_member_id::uuid for update;
  if not found then return 'not_found'; end if;
  if v_run.status in ('abandoned', 'incomplete') then return 'closed'; end if;

  select * into v_entry from goalcup.entries where id = p_entry_id and run_id = p_run_id for update;
  if not found then return 'not_found'; end if;

  if not p_final then
    if v_entry.sealed_at is null then   -- a sealed entry has nothing left to record
      insert into goalcup.heartbeats (run_id, entry_id, received_at, seq, head, offset_local, final,
                                      recording, client_ip, client_country)
      values (p_run_id, p_entry_id, v_at, p_seq, p_head, p_offset_local, false, p_recording, p_client_ip, p_client_country);
      perform goalcup.note_heartbeat(p_run_id, p_entry_id, v_at, p_recording, p_model_requests, p_client_ip, p_client_country);
    end if;
    return 'recorded';
  end if;

  if v_entry.final_seq is not null then
    if v_entry.final_seq = p_seq and v_entry.final_head = p_head then return 'final_existing'; end if;
    return 'final_conflict';
  end if;

  -- The end cannot contradict the chain the server already holds (next_seq - 1 is -1 before any record).
  v_stored := v_entry.next_seq - 1;
  if v_stored > p_seq or (v_stored = p_seq and v_entry.head <> p_head) then
    return 'final_conflict';
  end if;

  update goalcup.entries
     set final_seq = p_seq, final_head = p_head, final_repo_head = p_repo_head, final_at = v_at
   where id = p_entry_id;
  insert into goalcup.heartbeats (run_id, entry_id, received_at, seq, head, offset_local, final, repo_head,
                                  recording, client_ip, client_country)
  values (p_run_id, p_entry_id, v_at, p_seq, p_head, p_offset_local, true, p_repo_head, p_recording, p_client_ip, p_client_country);
  perform goalcup.note_heartbeat(p_run_id, p_entry_id, v_at, p_recording, p_model_requests, p_client_ip, p_client_country);
  return 'final_committed';
end $$;

-- The close of a run that is over (the Worker calls this the first time it sees one, and its cron does):
-- 'sealed' if its current entry is sealed, 'incomplete' if it has been over for 24 hours without that.
-- Returns the status now ('open' when nothing changed), or null if the run is not this member's.
create function goalcup.upload_close_run(p_member_id text, p_run_id text, p_at text) returns text
language plpgsql security definer set search_path = goalcup, pg_temp
as $$
declare
  v_run    goalcup.runs;
  v_at     timestamptz := p_at::timestamptz;
  v_status text;
begin
  select * into v_run from goalcup.runs where id = p_run_id and member_id = p_member_id::uuid for update;
  if not found then return null; end if;
  v_status := goalcup.run_status_at(p_run_id, v_at);
  if v_status <> v_run.status then
    update goalcup.runs
       set status = v_status,
           sealed_at = case when v_status = 'sealed'
                            then (select e.sealed_at from goalcup.entries e where e.run_id = p_run_id order by e.segment desc limit 1)
                            else sealed_at end
     where id = p_run_id;
  end if;
  return v_status;
end $$;

-- ---- the cron (src/notices.ts) --------------------------------------------------------------

-- The runs the cron looks at: every open run, and every run that ended within 48 hours before p_at (the same
-- as TICK_LOOKBACK_SECONDS in src/db.ts), except abandoned ones; oldest first. Each is the run as run_json
-- gives it, plus what the heartbeats and the deadline added (the TickRun of src/db.ts).
create function goalcup.upload_runs_for_tick(p_at text) returns jsonb
language sql stable security definer set search_path = goalcup, pg_temp
as $$
  select coalesce(jsonb_agg(
           (goalcup.run_json(r.id) -> 'run') || jsonb_build_object(
             'last_heartbeat_at',     goalcup.fmt_time(r.last_heartbeat_at),
             'last_model_request_at', goalcup.fmt_time(r.last_model_request_at),
             'flagged_at',            goalcup.fmt_time(r.flagged_at),
             'flag_reason',           r.flag_reason,
             'final_commit',          r.final_commit)
           order by r.official_start, r.id), '[]'::jsonb)
    from goalcup.runs r
   where r.status <> 'abandoned'
     and (r.status = 'open'
          or least(r.deadline_at, coalesce(r.replaced_at, r.deadline_at)) >= p_at::timestamptz - interval '48 hours')
$$;

-- Queue a notice in the outbox. True when this call queued it, false when one with the same run, kind and
-- dedupe key was there already (or the run is not this member's).
create function goalcup.upload_queue_notice(
  p_run_id text, p_member_id text, p_kind text, p_dedupe_key text, p_payload jsonb, p_at text
) returns boolean
language plpgsql security definer set search_path = goalcup, pg_temp
as $$
declare
  v_rows integer;
begin
  insert into goalcup.notices (run_id, member_id, kind, dedupe_key, payload, created_at)
  select r.id, r.member_id, p_kind, p_dedupe_key, coalesce(p_payload, '{}'::jsonb), p_at::timestamptz
    from goalcup.runs r
   where r.id = p_run_id and r.member_id = p_member_id::uuid
  on conflict (run_id, kind, dedupe_key) do nothing;
  get diagnostics v_rows = row_count;
  return v_rows = 1;
end $$;

-- Keep the commit GitHub had at the deadline ('none' when it has no such repository). The first one stays.
-- True when this call stored it.
create function goalcup.upload_record_final_commit(p_member_id text, p_run_id text, p_commit text, p_at text) returns boolean
language plpgsql security definer set search_path = goalcup, pg_temp
as $$
declare
  v_rows integer;
begin
  update goalcup.runs set final_commit = p_commit, final_commit_at = p_at::timestamptz
   where id = p_run_id and member_id = p_member_id::uuid and final_commit is null;
  get diagnostics v_rows = row_count;
  return v_rows = 1;
end $$;

-- ---- privileges: now that every function exists -----------------------------------------------
-- PUBLIC has EXECUTE on each function just created (fmt_time, run_json, run_status_at and the upload_* ones).
-- Take it away from everybody, then give the Worker's role exactly the upload_* functions.
-- (the helpers stay unexecutable by it: the definer functions call them as the owner.)
revoke all on all functions in schema goalcup from public, anon, authenticated;
revoke all on all tables    in schema goalcup from public, anon, authenticated;

-- ---- grants: the Worker may execute exactly these, nothing else -----------------------------

grant execute on function
  goalcup.upload_credential(text, text),
  goalcup.upload_create_device(text, text, text, text, text, text, text, text),
  goalcup.upload_poll_device(text, text),
  goalcup.upload_approve_device(text, text, boolean, text),  -- STAGING ONLY: approves a request as any member.
                                                              -- Owner decision: grant it only to a staging Worker role.
  goalcup.upload_open_run(text, text, text, text, text, text, text, text, text, text, text, text, text, text, text,
                          jsonb, text, text, text, text),
  goalcup.upload_continue_run(text, text, text, text, text, text, text, text, text, text, text, text, text, text),
  goalcup.upload_run_position(text, text, text),
  goalcup.upload_get_chunk(text, text, text, bigint),
  goalcup.upload_issue_receipt(text, text, text, text, jsonb),
  goalcup.upload_abandon_run(text, text, text, text),
  goalcup.upload_commit_chunk(text, text, jsonb),
  goalcup.upload_seal_run(text, text, text, text, text, text, text, text),
  goalcup.upload_heartbeat(text, text, text, bigint, text, bigint, boolean, text, text, bigint, text, text, text),
  goalcup.upload_close_run(text, text, text),
  goalcup.upload_runs_for_tick(text),
  goalcup.upload_queue_notice(text, text, text, text, jsonb, text),
  goalcup.upload_record_final_commit(text, text, text, text)
to goalcup_worker;

-- ---- what the website's owner still has to decide --------------------------------------------
--  * Where "accepted the current terms" really lives (this file assumes goalcup.consents).
--  * Seeding goalcup.events: one row for the competition (its start window decides when a competition run
--    may be started) and one for practice (the window columns are ignored for it). The Worker takes
--    EVENT_ID, PRACTICE_EVENT_ID, RUN_HOURS and PRACTICE_RUN_MINUTES from its own variables; upload_open_run
--    only checks that the event exists and, for a competition run, the window.
--  * The mailer: it reads goalcup.notices where sent_at is null, writes the email for each kind (started, silence,
--    idle_3h, idle_6h, recording, ended; the payload has what it needs), sends it and sets sent_at. Nothing here
--    sends mail, and goalcup_worker can write the outbox only through upload_queue_notice.
--  * The approval page: it calls goalcup.goalcup_approve_device(user_code, member_id, approve) for the
--    signed-in member, and replaces goalcup.member_handle with the real handle column.
--  * Whether PostgREST should expose schema goalcup at all (required for RPC), or whether thin
--    wrappers in `public` should be called instead (then drop the Content-Profile header in db-supabase.ts).
