-- =====================================================================================
-- PROPOSAL ONLY. NEVER RUN AGAINST SUPABASE.
--
-- It was loaded once into a throwaway local PostgreSQL 17 (with stub roles anon, authenticated,
-- authenticator) and its functions were exercised by hand: open / repeat / conflict, receipt,
-- commit (stale offset refused), abandon, seal, and the role's lack of table privileges. That
-- proves the SQL parses and behaves as the Worker expects. It does NOT prove anything about
-- Supabase itself: PostgREST exposure of the schema, the JWT role, RLS defaults.
--
-- The website repo owns the `goalcup` schema, its migrations, token minting, the terms
-- checkbox and the connect page. This file is what the upload Worker (server/worker/) expects
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
--     run of another member, so a token cannot probe for other members' runs.
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
  run_hours            integer not null default 168,
  grace_hours          integer not null default 48
);

-- Which version of the terms a member accepted. The website's consent flow writes this;
-- the Worker only asks "has this member accepted the current terms?" (inside upload_open_run).
create table goalcup.consents (
  member_id     uuid not null,
  terms_version text not null,
  accepted_at   timestamptz not null default now(),
  primary key (member_id, terms_version)
);

-- Only the sha256 of a token is stored. Minted by a server-side function in the website
-- repo, never by inserting rows from a client.
create table goalcup.tokens (
  token_hash  text primary key check (token_hash ~ '^[0-9a-f]{64}$'),
  member_id   uuid not null,
  created_at  timestamptz not null default now(),
  revoked_at  timestamptz
);

create table goalcup.runs (
  id                   text primary key check (id ~ '^r[0-9a-f]{16}$'),
  event_id             text not null references goalcup.events (id),
  member_id            uuid not null,
  official_start       timestamptz not null,
  deadline_at          timestamptz not null,
  status               text not null default 'open' check (status in ('open', 'sealed', 'abandoned', 'flagged')),
  repo_url             text not null,
  session_id           text not null,
  connector_id         text not null,
  manifest_sha256      text not null,
  connector_dir_sha256 text not null,
  workspace            text not null,
  harness_version      text not null,
  tracer_version       text not null,
  receipt              jsonb,                 -- the signed StartReceipt, once issued
  abandon_reason       text,
  seal_sha256          text,
  baseline_sha256      text,
  seal_key             text,                  -- R2 keys of the stored seal.json / baseline.json
  baseline_key         text,
  sealed_at            timestamptz,
  final_commit         text,
  final_commit_at      timestamptz,
  last_heartbeat_at    timestamptz,
  created_at           timestamptz not null default now()
);

-- One open run per member, enforced by the database, so two simultaneous starts cannot both win.
create unique index runs_one_open_per_member on goalcup.runs (member_id) where status = 'open';

create table goalcup.entries (
  id            text primary key check (id ~ '^e[0-9a-f]{16}$'),   -- the entry_id
  run_id        text not null references goalcup.runs (id),
  segment       integer not null default 0,
  opened_at     timestamptz not null default now(),
  upload_offset bigint not null default 0,
  next_seq      bigint not null default 0,                          -- the first record (the baseline) has seq 0
  head          text not null default repeat('0', 64),              -- genesis before any record
  last_ack      jsonb,
  last_chunk_at timestamptz,
  sealed_at     timestamptz,
  unique (run_id, segment)
);

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

-- Other tables from SERVER-DESIGN section 7 (heartbeats, outages, verify_results) belong to
-- later slices and are not needed by these six calls.

alter table goalcup.events   enable row level security;
alter table goalcup.consents enable row level security;
alter table goalcup.tokens   enable row level security;
alter table goalcup.runs     enable row level security;
alter table goalcup.entries  enable row level security;
alter table goalcup.chunks   enable row level security;

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

-- The run as the Worker's RunRow (src/db.ts). Joins the run with its (first) entry.
create function goalcup.run_json(p_run_id text) returns jsonb
language sql stable security definer set search_path = goalcup, pg_temp
as $$
  select jsonb_build_object(
    'run_id',               r.id,
    'entry_id',             e.id,
    'event_id',             r.event_id,
    'member_id',            r.member_id::text,
    'status',               r.status,
    'official_start',       goalcup.fmt_time(r.official_start),
    'deadline_at',          goalcup.fmt_time(r.deadline_at),
    'connector_id',         r.connector_id,
    'manifest_sha256',      r.manifest_sha256,
    'connector_dir_sha256', r.connector_dir_sha256,
    'workspace',            r.workspace,
    'session_id',           r.session_id,
    'repo_url',             r.repo_url,
    'offset',               e.upload_offset,
    'next_seq',             e.next_seq,
    'head',                 e.head,
    'last_ack',             e.last_ack,
    'receipt',              r.receipt,
    'seal_sha256',          r.seal_sha256,
    'baseline_sha256',      r.baseline_sha256
  )
  from goalcup.runs r
  join goalcup.entries e on e.run_id = r.id and e.segment = 0
  where r.id = p_run_id
$$;

-- ---- the six calls (+ token lookup) ------------------------------------------------------

-- Token (sha256 hex) -> member id as text, or null if unknown or revoked.
create function goalcup.upload_lookup_token(p_token_hash text) returns text
language sql stable security definer set search_path = goalcup, pg_temp
as $$
  select member_id::text from goalcup.tokens where token_hash = p_token_hash and revoked_at is null
$$;

-- POST /v1/runs. Returns {"result": "created"|"existing"|"conflict"|"forbidden", "run": {...}, "message": "..."}.
create function goalcup.upload_open_run(
  p_member_id text, p_event_id text, p_run_id text, p_entry_id text,
  p_official_start text, p_deadline_at text,
  p_connector_id text, p_manifest_sha256 text, p_connector_dir_sha256 text,
  p_workspace text, p_session_id text, p_harness_version text, p_tracer_version text, p_repo_url text
) returns jsonb
language plpgsql security definer set search_path = goalcup, pg_temp
as $$
declare
  v_member uuid := p_member_id::uuid;
  v_event  goalcup.events;
  v_open   goalcup.runs;
  v_start  timestamptz := p_official_start::timestamptz;
begin
  select * into v_event from goalcup.events where id = p_event_id;
  if not found then
    return jsonb_build_object('result', 'forbidden', 'message', 'This event is not open for runs.');
  end if;

  -- A run that was never sealed 24 hours after its deadline is abandoned, which frees the
  -- member's one open slot (the unique index is on open runs only).
  update goalcup.runs
     set status = 'abandoned', abandon_reason = 'expired: not sealed 24 hours after the deadline', sealed_at = v_start
   where member_id = v_member and status = 'open' and seal_sha256 is null
     and v_start > deadline_at + interval '24 hours';

  -- Same member, same session, still open, no receipt yet: the first answer was lost in transit.
  select * into v_open from goalcup.runs where member_id = v_member and status = 'open';
  if found then
    if v_open.session_id = p_session_id and v_open.receipt is null then
      return jsonb_build_object('result', 'existing', 'run', goalcup.run_json(v_open.id));
    end if;
    return jsonb_build_object('result', 'conflict');
  end if;

  -- Eligibility (checked after the idempotent repeat so a lost reply never turns into a refusal).
  if not exists (select 1 from goalcup.consents where member_id = v_member) then
    return jsonb_build_object('result', 'forbidden', 'message', 'Accept the Goal Cup terms on the website first.');
  end if;
  if v_start < v_event.start_window_opens or v_start > v_event.start_window_closes then
    return jsonb_build_object('result', 'forbidden', 'message', 'Runs cannot be started outside the start window.');
  end if;

  begin
    insert into goalcup.runs (id, event_id, member_id, official_start, deadline_at, repo_url, session_id,
                              connector_id, manifest_sha256, connector_dir_sha256, workspace,
                              harness_version, tracer_version)
    values (p_run_id, p_event_id, v_member, v_start, p_deadline_at::timestamptz, p_repo_url, p_session_id,
            p_connector_id, p_manifest_sha256, p_connector_dir_sha256, p_workspace,
            p_harness_version, p_tracer_version);
    insert into goalcup.entries (id, run_id, segment) values (p_entry_id, p_run_id, 0);
  exception when unique_violation then
    -- Two starts raced; the unique index let one through. That one has committed by now, so look
    -- at it again: if it is this same session (a lost-reply retry racing the original) the answer
    -- is the same run; otherwise this account has another open run.
    select * into v_open from goalcup.runs where member_id = v_member and status = 'open';
    if found and v_open.session_id = p_session_id and v_open.receipt is null then
      return jsonb_build_object('result', 'existing', 'run', goalcup.run_json(v_open.id));
    end if;
    return jsonb_build_object('result', 'conflict');
  end;
  return jsonb_build_object('result', 'created', 'run', goalcup.run_json(p_run_id));
end $$;

-- GET /v1/runs/{id} (and the lookup every other call starts with). Null if not this member's.
create function goalcup.upload_run_position(p_member_id text, p_run_id text) returns jsonb
language sql stable security definer set search_path = goalcup, pg_temp
as $$
  select goalcup.run_json(r.id) from goalcup.runs r where r.id = p_run_id and r.member_id = p_member_id::uuid
$$;

-- The stored chunk that starts at p_offset, in the Worker's ChunkRecord shape, or null.
create function goalcup.upload_get_chunk(p_member_id text, p_run_id text, p_offset bigint) returns jsonb
language sql stable security definer set search_path = goalcup, pg_temp
as $$
  select jsonb_build_object(
    'entry_id', c.entry_id, 'offset', c."offset", 'length', c.length, 'sha256', c.sha256,
    'first_seq', c.first_seq, 'last_seq', c.last_seq, 'prev', c.prev, 'head', c.head,
    'r2_key', c.r2_key, 'received_at', goalcup.fmt_time(c.received_at), 'ack', c.ack)
  from goalcup.chunks c
  join goalcup.entries e on e.id = c.entry_id
  join goalcup.runs r on r.id = e.run_id
  where r.id = p_run_id and r.member_id = p_member_id::uuid and c."offset" = p_offset
$$;

-- POST /v1/runs/{id}/receipt. The Worker signs; this stores. {"result": "stored"|"existing"|"conflict"|"closed", "receipt": {...}}
create function goalcup.upload_issue_receipt(p_member_id text, p_run_id text, p_chain_head text, p_receipt jsonb) returns jsonb
language plpgsql security definer set search_path = goalcup, pg_temp
as $$
declare
  v_run goalcup.runs;
begin
  select * into v_run from goalcup.runs where id = p_run_id and member_id = p_member_id::uuid for update;
  if not found or v_run.status <> 'open' then
    return jsonb_build_object('result', 'closed');
  end if;
  if v_run.receipt is not null then
    if v_run.receipt -> 'claims' ->> 'chain_head' = p_chain_head then
      return jsonb_build_object('result', 'existing', 'receipt', v_run.receipt);
    end if;
    return jsonb_build_object('result', 'conflict');
  end if;
  update goalcup.runs set receipt = p_receipt where id = p_run_id;
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
-- p_chunk is the Worker's ChunkRecord. Returns 'committed' | 'conflict' | 'closed' | 'not_found'.
create function goalcup.upload_commit_chunk(p_member_id text, p_run_id text, p_chunk jsonb) returns text
language plpgsql security definer set search_path = goalcup, pg_temp
as $$
declare
  v_run   goalcup.runs;
  v_entry goalcup.entries;
  v_off   bigint := (p_chunk ->> 'offset')::bigint;
  v_len   bigint := (p_chunk ->> 'length')::bigint;
begin
  -- Lock order everywhere (commit, abandon, seal): the run first, then its entry. So a commit
  -- racing an abandon or a seal queues behind it and then sees the new status.
  select * into v_run from goalcup.runs where id = p_run_id and member_id = p_member_id::uuid for update;
  if not found then return 'not_found'; end if;
  if v_run.status <> 'open' then return 'closed'; end if;

  -- Concurrent commits for the same run queue on the run lock above; the loser re-checks below.
  select * into v_entry from goalcup.entries
    where id = p_chunk ->> 'entry_id' and run_id = p_run_id for update;
  if not found then return 'not_found'; end if;

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
-- Returns 'sealed' | 'already_sealed' (same hashes) | 'conflict' (other hashes) | 'closed' | 'not_found'.
create function goalcup.upload_seal_run(
  p_member_id text, p_run_id text, p_entry_id text,
  p_seal_sha256 text, p_baseline_sha256 text, p_seal_key text, p_baseline_key text, p_sealed_at text
) returns text
language plpgsql security definer set search_path = goalcup, pg_temp
as $$
declare
  v_run goalcup.runs;
begin
  select * into v_run from goalcup.runs where id = p_run_id and member_id = p_member_id::uuid for update;
  if not found then return 'not_found'; end if;
  if v_run.status = 'sealed' then
    if v_run.seal_sha256 = p_seal_sha256 and v_run.baseline_sha256 = p_baseline_sha256 then
      return 'already_sealed';
    end if;
    return 'conflict';
  end if;
  if v_run.status <> 'open' then return 'closed'; end if;
  update goalcup.runs
     set status = 'sealed', seal_sha256 = p_seal_sha256, baseline_sha256 = p_baseline_sha256,
         seal_key = p_seal_key, baseline_key = p_baseline_key, sealed_at = p_sealed_at::timestamptz
   where id = p_run_id;
  update goalcup.entries set sealed_at = p_sealed_at::timestamptz where id = p_entry_id and run_id = p_run_id;
  return 'sealed';
end $$;

-- ---- privileges: now that every function exists -----------------------------------------------
-- PUBLIC has EXECUTE on each function just created (fmt_time, run_json and the upload_* ones).
-- Take it away from everybody, then give the Worker's role exactly the upload_* functions.
-- (run_json and fmt_time stay unexecutable by it: the definer functions call them as the owner.)
revoke all on all functions in schema goalcup from public, anon, authenticated;
revoke all on all tables    in schema goalcup from public, anon, authenticated;

-- ---- grants: the Worker may execute exactly these, nothing else -----------------------------

grant execute on function
  goalcup.upload_lookup_token(text),
  goalcup.upload_open_run(text, text, text, text, text, text, text, text, text, text, text, text, text, text),
  goalcup.upload_run_position(text, text),
  goalcup.upload_get_chunk(text, text, bigint),
  goalcup.upload_issue_receipt(text, text, text, jsonb),
  goalcup.upload_abandon_run(text, text, text, text),
  goalcup.upload_commit_chunk(text, text, jsonb),
  goalcup.upload_seal_run(text, text, text, text, text, text, text, text)
to goalcup_worker;

-- ---- what the website's owner still has to decide --------------------------------------------
--  * Where "accepted the current terms" really lives (this file assumes goalcup.consents).
--  * Seeding goalcup.events: one row per event (practice, dry_run, competition) with its window,
--    run_hours and grace_hours. The Worker currently takes EVENT_ID / RUN_HOURS / GRACE_HOURS from
--    its own variables and only checks in upload_open_run that the event exists and is open for starts.
--  * Minting tokens (insert sha256(token) into goalcup.tokens from a server-side function).
--  * Whether PostgREST should expose schema goalcup at all (required for RPC), or whether thin
--    wrappers in `public` should be called instead (then drop the Content-Profile header in db-supabase.ts).
