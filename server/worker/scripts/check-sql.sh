#!/bin/sh
# Try the pending website migration on a throwaway Postgres before anyone applies it to the shared
# database (sql/pending/README.md). It:
#   1. starts a private PostgreSQL in a temporary directory (unix socket only, trust auth);
#   2. stubs what Supabase provides (the anon, authenticated, service_role and authenticator roles,
#      auth.users, public.admins, public.email_suppression_list, public.member_profiles);
#   3. applies the mirror sql/goal-cup-tracer-upload.sql up to its first pending section, in one
#      transaction: the database as it is today;
#   4. checks that the mirror's pending sections are the files in sql/pending/, byte for byte;
#   5. applies each pending file with the production command (psql -X -1 -v ON_ERROR_STOP=1 -f);
#   6. smoke-calls the new and the old-signature functions, then stops and deletes everything.
#
#   sh server/worker/scripts/check-sql.sh
#
# Needs PostgreSQL's initdb, pg_ctl and psql (found on PATH or under /usr/lib/postgresql; PG_BIN=<dir>
# picks one). Run as root, the server runs as the postgres user. Exit 0 means every step passed.
set -eu

here=$(cd "$(dirname "$0")/.." && pwd)
sql=$here/sql
mirror=$sql/goal-cup-tracer-upload.sql
# The postmaster refuses to start on macOS when the locale is empty or unknown ("postmaster became
# multithreaded during startup ... Set the LC_ALL environment variable to a valid locale"); the
# database is --no-locale anyway.
LC_ALL=C
export LC_ALL
port=54329

if [ -z "${PG_BIN:-}" ]; then
  if command -v initdb >/dev/null 2>&1; then
    PG_BIN=$(dirname "$(command -v initdb)")
  else
    PG_BIN=$(ls -d /usr/lib/postgresql/*/bin 2>/dev/null | sort -V | tail -n 1)
  fi
fi
if [ ! -x "${PG_BIN:-}/initdb" ]; then
  echo "check-sql: no initdb found: install PostgreSQL or set PG_BIN" >&2
  exit 2
fi

tmp=$(mktemp -d)
as_pg=""
if [ "$(id -u)" = 0 ]; then
  chown postgres "$tmp"
  as_pg="runuser -u postgres --"
fi
cleanup() {
  $as_pg "$PG_BIN/pg_ctl" -D "$tmp/data" -m immediate stop >/dev/null 2>&1 || true
  rm -rf "$tmp"
}
trap cleanup EXIT
trap 'cleanup; trap - EXIT; exit 130' INT TERM
cd "$tmp"

$as_pg "$PG_BIN/initdb" -D "$tmp/data" -U postgres --auth=trust -E UTF8 --no-locale >/dev/null
$as_pg "$PG_BIN/pg_ctl" -D "$tmp/data" -l "$tmp/log" -w -o "-c listen_addresses='' -k $tmp -p $port" start >/dev/null
pg() { "$PG_BIN/psql" -X -q -v ON_ERROR_STOP=1 -h "$tmp" -p "$port" -U postgres -d postgres "$@"; }
echo "check-sql: Postgres $("$PG_BIN/postgres" --version | awk '{print $3}') in $tmp"

cat > "$tmp/stubs.sql" <<'SQL'
CREATE ROLE anon NOLOGIN;
CREATE ROLE authenticated NOLOGIN;
CREATE ROLE service_role NOLOGIN;
CREATE ROLE authenticator NOLOGIN;
CREATE SCHEMA auth;
CREATE TABLE auth.users (id uuid PRIMARY KEY, email text, email_confirmed_at timestamptz);
CREATE TABLE public.admins (email text NOT NULL);
CREATE TABLE public.email_suppression_list (email text NOT NULL, expires_at timestamptz);
CREATE TABLE public.member_profiles (user_id uuid NOT NULL, slug text, is_listed boolean);
SQL

# The mirror up to the line before its first pending section, and the check that the rest of it is
# the pending files (the regeneration format: a blank line, the "=====" line, the file).
# The pending files, in name order, as the positional parameters (a glob, so a name with a space
# stays one name).
set -- "$sql"/pending/*.sql
if [ ! -e "$1" ]; then
  echo "check-sql: no file in sql/pending/" >&2
  exit 2
fi
first=$(basename "$1")
marker="-- ======================== $first ========================"
awk -v m="$marker" '$0 == m { exit } NR > 1 { print prev } { prev = $0 }' "$mirror" > "$tmp/base.sql"
for f in "$@"; do
  printf '\n-- ======================== %s ========================\n' "${f##*/}"
  cat "$f"
done > "$tmp/sections.sql"
if ! cat "$tmp/base.sql" "$tmp/sections.sql" | cmp -s - "$mirror"; then
  echo "check-sql: the mirror's last sections are not the files in sql/pending/, byte for byte" >&2
  exit 1
fi

pg -1 -f "$tmp/stubs.sql" -f "$tmp/base.sql"
echo "check-sql: applied the mirror up to $first"
for f in "$@"; do
  pg -1 -f "$f"
  echo "check-sql: applied ${f##*/}"
done

# Smoke calls (production deployment: no request.jwt.claims). Any failed ASSERT stops the script.
pg -1 <<'SQL'
DO $$
DECLARE
  m   constant text := '11111111-1111-1111-1111-111111111111';
  z   constant text := repeat('0', 64);
  h1  constant text := repeat('1', 64);
  h2  constant text := repeat('2', 64);
  t0  constant timestamptz := now();
  r   jsonb;
  run goal_cup_tracer.runs;
  k   text;
BEGIN
  INSERT INTO goal_cup_tracer.test_operators (member_id) VALUES (m::uuid);
  INSERT INTO goal_cup_tracer.consents (member_id, terms_version) VALUES (m::uuid, goal_cup_tracer.current_terms_version());
  INSERT INTO goal_cup_tracer.run_credentials (credential_hash, member_id, mode, expires_at, staging)
  VALUES (repeat('a', 64), m::uuid, 'test', t0 + interval '1 hour', false),
         (repeat('b', 64), m::uuid, 'test', t0 + interval '1 hour', false);

  -- 1. Two test runs of one operator: the second replaces nothing.
  r := public.goal_cup_upload_open_run(m, 'test', 'test', 'r0000000000000001', 'e0000000000000001', t0::text,
         (t0 + interval '1 day')::text, 'c', z, z, 'w', 's1', '1', '1', 'https://github.com/o/a', 'null'::jsonb,
         NULL, NULL, repeat('a', 64), (t0 + interval '2 days')::text);
  ASSERT r ->> 'result' = 'created', 'open run 1: ' || r::text;
  ASSERT (r -> 'run' ->> 'stored_bytes') = '0' AND (r -> 'run' ->> 'chunk_count') = '0', 'run totals: ' || r::text;
  r := public.goal_cup_upload_open_run(m, 'test', 'test', 'r0000000000000002', 'e0000000000000002', t0::text,
         (t0 + interval '1 day')::text, 'c', z, z, 'w', 's2', '1', '1', 'https://github.com/o/b', 'null'::jsonb,
         NULL, NULL, repeat('b', 64), (t0 + interval '2 days')::text);
  ASSERT r ->> 'result' = 'created', 'open run 2: ' || r::text;
  ASSERT (SELECT count(*) FROM goal_cup_tracer.runs WHERE replaced_at IS NULL AND status = 'open') = 2, 'a test run replaced another';
  INSERT INTO goal_cup_tracer.device_requests (device_hash, user_hash, purpose, mode, connector_id, tracer_version,
                                               created_at, expires_at, staging)
  VALUES (repeat('c', 64), encode(sha256(convert_to('ABCDEFGH', 'UTF8')), 'hex'), 'run', 'test', 'c', '1',
          t0, t0 + interval '15 minutes', false);
  r := public.goal_cup_device_request(m::uuid, 'ABCD-EFGH');
  ASSERT r ->> 'result' = 'ok' AND (r ->> 'replaces_open_run')::boolean = false, 'device request: ' || r::text;

  -- 2. Chunks count their stored bytes, through the new and the old form.
  k := public.goal_cup_upload_commit_chunk(m, 'r0000000000000001', jsonb_build_object(
         'entry_id', 'e0000000000000001', 'offset', 0, 'length', 100, 'sha256', z, 'first_seq', 0, 'last_seq', 4,
         'prev', z, 'head', h1, 'r2_key', 'k1', 'received_at', t0, 'ack', '{}'::jsonb), 40::bigint);
  ASSERT k = 'committed', 'commit_chunk (4 args): ' || k;
  k := public.goal_cup_upload_commit_chunk(m, 'r0000000000000001', jsonb_build_object(
         'entry_id', 'e0000000000000001', 'offset', 100, 'length', 50, 'sha256', z, 'first_seq', 5, 'last_seq', 6,
         'prev', h1, 'head', h2, 'r2_key', 'k2', 'received_at', t0, 'ack', '{}'::jsonb));
  ASSERT k = 'committed', 'commit_chunk (3 args): ' || k;
  r := goal_cup_tracer.run_json('r0000000000000001') -> 'run';
  ASSERT r ->> 'stored_bytes' = '40' AND r ->> 'chunk_count' = '2', 'run totals after chunks: ' || r::text;
  k := NULL;
  BEGIN
    k := public.goal_cup_upload_commit_chunk(m, 'r0000000000000001', jsonb_build_object(
           'entry_id', 'e0000000000000001', 'offset', 150, 'length', 10, 'sha256', z, 'first_seq', 7, 'last_seq', 7,
           'prev', h2, 'head', h1, 'r2_key', 'k3', 'received_at', t0, 'ack', '{}'::jsonb), (-1)::bigint);
  EXCEPTION WHEN raise_exception THEN
    k := 'raised';
  END;
  ASSERT k = 'raised', 'commit_chunk took a negative size: ' || coalesce(k, 'null');

  -- 3. Vitals and the disk pause, through the new and the old heartbeat.
  k := public.goal_cup_upload_heartbeat(m, 'r0000000000000001', 'e0000000000000001', 6, h2, 150, false, NULL, 'ok', 1,
         NULL, NULL, t0::text, '{"agent_paused": "disk", "disk_free_bytes": 1}'::jsonb);
  ASSERT k = 'recorded', 'heartbeat (14 args): ' || k;
  SELECT * INTO run FROM goal_cup_tracer.runs WHERE id = 'r0000000000000001';
  ASSERT run.disk_paused_at = t0 AND run.disk_notice_at = t0 AND run.last_vitals ->> 'disk_free_bytes' = '1', 'pause not started';
  k := public.goal_cup_upload_heartbeat(m, 'r0000000000000001', 'e0000000000000001', 6, h2, 150, false, NULL, 'ok', 1,
         NULL, NULL, (t0 + interval '1 minute')::text);
  ASSERT k = 'recorded', 'heartbeat (13 args): ' || k;
  ASSERT (SELECT disk_paused_at FROM goal_cup_tracer.runs WHERE id = 'r0000000000000001') = t0, 'no vitals ended the pause';
  k := public.goal_cup_upload_heartbeat(m, 'r0000000000000001', 'e0000000000000001', 6, h2, 150, false, NULL, 'ok', 1,
         NULL, NULL, (t0 + interval '2 minutes')::text, '{"disk_free_bytes": 9}'::jsonb);
  ASSERT (SELECT disk_paused_at IS NULL FROM goal_cup_tracer.runs WHERE id = 'r0000000000000001'), 'pause not ended';
  -- A new pause within 6 hours of the last owed one owes nothing; paused to paused is the same pause.
  k := public.goal_cup_upload_heartbeat(m, 'r0000000000000001', 'e0000000000000001', 6, h2, 150, false, NULL, 'ok', 1,
         NULL, NULL, (t0 + interval '3 minutes')::text, '{"agent_paused": "disk"}'::jsonb);
  SELECT * INTO run FROM goal_cup_tracer.runs WHERE id = 'r0000000000000001';
  ASSERT run.disk_paused_at = t0 + interval '3 minutes' AND run.disk_notice_at = t0, 'second pause within 6 hours';
  k := public.goal_cup_upload_heartbeat(m, 'r0000000000000001', 'e0000000000000001', 6, h2, 150, false, NULL, 'ok', 1,
         NULL, NULL, (t0 + interval '4 minutes')::text, '{"agent_paused": "disk"}'::jsonb);
  SELECT * INTO run FROM goal_cup_tracer.runs WHERE id = 'r0000000000000001';
  ASSERT run.disk_paused_at = t0 + interval '3 minutes' AND run.disk_notice_at = t0, 'paused to paused moved the pause';
  k := public.goal_cup_upload_heartbeat(m, 'r0000000000000001', 'e0000000000000001', 6, h2, 150, false, NULL, 'ok', 1,
         NULL, NULL, (t0 + interval '5 minutes')::text, '{}'::jsonb);
  -- 6 hours after the last owed pause, a new one is owed again.
  k := public.goal_cup_upload_heartbeat(m, 'r0000000000000001', 'e0000000000000001', 6, h2, 150, false, NULL, 'ok', 1,
         NULL, NULL, (t0 + interval '6 hours')::text, '{"agent_paused": "disk"}'::jsonb);
  SELECT * INTO run FROM goal_cup_tracer.runs WHERE id = 'r0000000000000001';
  ASSERT run.disk_paused_at = t0 + interval '6 hours' AND run.disk_notice_at = t0 + interval '6 hours', 'pause after 6 hours owes a notice';
  r := public.goal_cup_upload_runs_for_tick(t0::text);
  ASSERT jsonb_array_length(r) = 2 AND r -> 0 ? 'disk_notice_at', 'runs_for_tick: ' || r::text;
  INSERT INTO goal_cup_tracer.notices (run_id, member_id, kind, dedupe_key, created_at)
  VALUES ('r0000000000000001', m::uuid, 'disk_full', t0::text, t0);

  -- 4. The judged push, through the new and the old form; the first stays.
  ASSERT public.goal_cup_upload_record_final_commit(m, 'r0000000000000001', repeat('f', 40), t0::text,
           t0::text, 'main', false, false, false), 'record_final_commit (9 args)';
  ASSERT NOT public.goal_cup_upload_record_final_commit(m, 'r0000000000000001', repeat('e', 40), t0::text,
           NULL, NULL, NULL, NULL, NULL), 'record_final_commit replaced the first';
  ASSERT public.goal_cup_upload_record_final_commit(m, 'r0000000000000002', 'none', t0::text), 'record_final_commit (4 args)';
  ASSERT (SELECT final_branch FROM goal_cup_tracer.runs WHERE id = 'r0000000000000001') = 'main', 'final_branch';

  -- 5. Every kind has an email; the new words are there.
  FOREACH k IN ARRAY ARRAY['started', 'silence', 'idle_3h', 'idle_6h', 'recording', 'ended', 'disk_full'] LOOP
    r := goal_cup_tracer.notice_text(k, '{}'::jsonb, 'r0000000000000001');
    ASSERT r ->> 'subject' IS NOT NULL AND r ->> 'text' IS NOT NULL, 'notice_text: ' || k;
  END LOOP;
  ASSERT goal_cup_tracer.notice_text('silence', '{}', 'r') ->> 'text' LIKE '%goalcup status%', 'silence words';
  ASSERT goal_cup_tracer.notice_text('ended', '{"uploaded": false}', 'r') ->> 'text' LIKE '%goalcup upload%', 'ended words';

  -- 6. Grants: the Worker roles can call the new overloads; anon, authenticated and PUBLIC cannot,
  --    nor call the internal 8-argument note_heartbeat.
  FOREACH k IN ARRAY ARRAY[
    'public.goal_cup_upload_commit_chunk(text, text, jsonb, bigint)',
    'public.goal_cup_upload_heartbeat(text, text, text, bigint, text, bigint, boolean, text, text, bigint, text, text, text, jsonb)',
    'public.goal_cup_upload_record_final_commit(text, text, text, text, text, text, boolean, boolean, boolean)'
  ] LOOP
    ASSERT has_function_privilege('goal_cup_ingest_staging', k, 'EXECUTE'), 'staging Worker cannot call ' || k;
    ASSERT has_function_privilege('goal_cup_ingest_production', k, 'EXECUTE'), 'production Worker cannot call ' || k;
    ASSERT has_function_privilege('service_role', k, 'EXECUTE'), 'service_role cannot call ' || k;
  END LOOP;
  FOREACH k IN ARRAY ARRAY[
    'public.goal_cup_upload_commit_chunk(text, text, jsonb, bigint)',
    'public.goal_cup_upload_heartbeat(text, text, text, bigint, text, bigint, boolean, text, text, bigint, text, text, text, jsonb)',
    'public.goal_cup_upload_record_final_commit(text, text, text, text, text, text, boolean, boolean, boolean)',
    'goal_cup_tracer.note_heartbeat(text, text, timestamptz, text, bigint, text, text, jsonb)'
  ] LOOP
    ASSERT NOT has_function_privilege('anon', k, 'EXECUTE'), 'anon can call ' || k;
    ASSERT NOT has_function_privilege('authenticated', k, 'EXECUTE'), 'authenticated can call ' || k;
    ASSERT NOT has_function_privilege('public', k, 'EXECUTE'), 'PUBLIC can call ' || k;
  END LOOP;

END $$;
SQL
echo "check-sql: smoke calls passed"
