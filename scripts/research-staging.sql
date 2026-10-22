-- Read-only measurement of one event's uploads: chunk sizes, ACK lag, heartbeat health.
--
-- Used by scripts/research-staging.py (which adds the R2 object sizes and does the arithmetic), and
-- usable on its own as an event dashboard, on staging or production, with a read-only role:
--
--   psql -X -q -A -t -v ON_ERROR_STOP=1 -v event=test-staging -v since=-infinity \
--     -f scripts/research-staging.sql
--
-- (the connection comes from the environment or a service file, never from the command line).
--   :'event'  the event id (goal_cup_tracer.events.id), e.g. test-staging or goalcup-dryrun-staging
--   :'since'  a timestamptz; only runs created at or after it ('-infinity' for all)
--
-- Every row is one JSON object with a "section" key. Ids are full here; the script truncates them.
-- Nothing here reads member ids, emails, IP addresses, repo URLs, receipts, acks or the free text
-- of an abandon reason (it is only tested for presence).
--
-- The whole file is one transaction, READ ONLY and REPEATABLE READ, so every section sees the same
-- snapshot and no statement can write. It ends in ROLLBACK. Nothing is SET for the session, so a
-- pooled connection (Supabase's pooler) is left as it was found.
--
-- Columns used, all in server/worker/sql/goal-cup-tracer-upload.sql:
--   events (43-51): id, mode, staging, run_hours
--   runs (78-101, staging 1078): id, event_id, mode, status, official_start, deadline_at,
--     replaced_at, abandon_reason, flag_reason, last_heartbeat_at, created_at, staging
--   entries (118-145): id, run_id, segment, opened_at, upload_offset, last_chunk_at, model_requests,
--     final_at, sealed_at
--   heartbeats (150-164): id, run_id, entry_id, received_at, offset_local, final, recording
--   chunks (189-202): entry_id, "offset", length, r2_key, received_at
--   notices (168-185): run_id, kind, sent_at, skipped_at, last_error, attempts

BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET TRANSACTION READ ONLY;
SET LOCAL statement_timeout = '120s';

-- @section event: the event row (none: the event does not exist).
SELECT jsonb_build_object(
  'section', 'event', 'id', ev.id, 'mode', ev.mode, 'staging', ev.staging, 'run_hours', ev.run_hours,
  'now', extract(epoch FROM now()))::text
FROM goal_cup_tracer.events ev
WHERE ev.id = :'event';

-- @section run: one row per run, with its heartbeat gaps (seconds between consecutive heartbeats
-- of the run, over all its entries).
WITH sel AS (
  SELECT r.* FROM goal_cup_tracer.runs r
  WHERE r.event_id = :'event' AND r.created_at >= :'since'::timestamptz
), beats AS (
  SELECT h.run_id, h.received_at,
         extract(epoch FROM h.received_at - lag(h.received_at) OVER (PARTITION BY h.run_id ORDER BY h.received_at, h.id)) AS gap
  FROM goal_cup_tracer.heartbeats h JOIN sel ON sel.id = h.run_id
), per_run AS (
  SELECT run_id, count(*) AS n, min(received_at) AS first_at, max(received_at) AS last_at,
         max(gap) AS max_gap,
         percentile_cont(0.5) WITHIN GROUP (ORDER BY gap) AS p50_gap,
         percentile_cont(0.9) WITHIN GROUP (ORDER BY gap) AS p90_gap
  FROM beats GROUP BY run_id
)
SELECT jsonb_build_object(
  'section', 'run', 'run', sel.id, 'mode', sel.mode, 'status', sel.status, 'staging', sel.staging,
  'official_start', extract(epoch FROM sel.official_start),
  'deadline_at', extract(epoch FROM sel.deadline_at),
  'replaced', sel.replaced_at IS NOT NULL,
  'abandon_reason_given', sel.abandon_reason IS NOT NULL,
  'flag_reason', sel.flag_reason,
  'last_heartbeat_at', extract(epoch FROM sel.last_heartbeat_at),
  'heartbeats', coalesce(b.n, 0),
  'first_beat_at', extract(epoch FROM b.first_at),
  'last_beat_at', extract(epoch FROM b.last_at),
  'max_gap_s', b.max_gap, 'p50_gap_s', b.p50_gap, 'p90_gap_s', b.p90_gap)::text
FROM sel LEFT JOIN per_run b ON b.run_id = sel.id
ORDER BY sel.created_at, sel.id;

-- @section entry: one row per entry, with its latest heartbeat. ACK lag is offset_local (the
-- client's chain bytes at that heartbeat) minus upload_offset (the bytes the server holds now), and
-- also minus acked_at_beat (the bytes the server held when that heartbeat arrived).
WITH sel AS (
  SELECT r.id FROM goal_cup_tracer.runs r
  WHERE r.event_id = :'event' AND r.created_at >= :'since'::timestamptz
), latest AS (
  SELECT DISTINCT ON (h.entry_id) h.entry_id, h.received_at, h.offset_local, h.final, h.recording
  FROM goal_cup_tracer.heartbeats h JOIN sel ON sel.id = h.run_id
  ORDER BY h.entry_id, h.received_at DESC, h.id DESC
)
SELECT jsonb_build_object(
  'section', 'entry', 'run', e.run_id, 'entry', e.id, 'segment', e.segment,
  'upload_offset', e.upload_offset,
  'opened_at', extract(epoch FROM e.opened_at),
  'last_chunk_at', extract(epoch FROM e.last_chunk_at),
  'final_at', extract(epoch FROM e.final_at),
  'sealed', e.sealed_at IS NOT NULL,
  'model_requests', e.model_requests,
  'beat_at', extract(epoch FROM l.received_at),
  'offset_local', l.offset_local,
  'beat_final', l.final,
  'recording', l.recording,
  'acked_at_beat', CASE WHEN l.entry_id IS NULL THEN NULL ELSE
    (SELECT coalesce(max(c."offset" + c.length), 0) FROM goal_cup_tracer.chunks c
      WHERE c.entry_id = e.id AND c.received_at <= l.received_at) END)::text
FROM goal_cup_tracer.entries e
JOIN sel ON sel.id = e.run_id
LEFT JOIN latest l ON l.entry_id = e.id
ORDER BY e.run_id, e.segment;

-- @section chunk: one row per chunk (the script joins r2_key to the R2 listing for the compressed
-- size; the database stores only the logical, uncompressed length).
SELECT jsonb_build_object(
  'section', 'chunk', 'run', e.run_id, 'entry', c.entry_id, 'offset', c."offset", 'length', c.length,
  'key', c.r2_key, 'at', extract(epoch FROM c.received_at))::text
FROM goal_cup_tracer.chunks c
JOIN goal_cup_tracer.entries e ON e.id = c.entry_id
JOIN goal_cup_tracer.runs r ON r.id = e.run_id
WHERE r.event_id = :'event' AND r.created_at >= :'since'::timestamptz
ORDER BY c.entry_id, c."offset";

-- @section notice: the run emails by kind and state ('silence' and 'recording' are health signals;
-- 'failed' means a send error is recorded and the notice is not yet sent or skipped).
SELECT jsonb_build_object('section', 'notice', 'kind', s.kind, 'state', s.state,
                          'count', count(*), 'attempts', sum(s.attempts))::text
FROM (
  SELECT n.kind, n.attempts,
         CASE WHEN n.sent_at IS NOT NULL THEN 'sent'
              WHEN n.skipped_at IS NOT NULL THEN 'skipped'
              WHEN n.last_error IS NOT NULL THEN 'failed'
              ELSE 'pending' END AS state
  FROM goal_cup_tracer.notices n
  JOIN goal_cup_tracer.runs r ON r.id = n.run_id
  WHERE r.event_id = :'event' AND r.created_at >= :'since'::timestamptz
) s
GROUP BY s.kind, s.state
ORDER BY s.kind, s.state;

-- @section summary: the dashboard in one row, from the database alone (no R2 sizes).
WITH sel AS (
  SELECT r.* FROM goal_cup_tracer.runs r
  WHERE r.event_id = :'event' AND r.created_at >= :'since'::timestamptz
), last_beat AS (
  SELECT h.run_id, max(h.received_at) AS at FROM goal_cup_tracer.heartbeats h JOIN sel ON sel.id = h.run_id
  GROUP BY h.run_id
), latest AS (
  SELECT DISTINCT ON (h.entry_id) h.entry_id, h.offset_local
  FROM goal_cup_tracer.heartbeats h JOIN sel ON sel.id = h.run_id
  ORDER BY h.entry_id, h.received_at DESC, h.id DESC
), lag AS (
  SELECT l.offset_local - e.upload_offset AS bytes FROM latest l JOIN goal_cup_tracer.entries e ON e.id = l.entry_id
), ch AS (
  SELECT c.length FROM goal_cup_tracer.chunks c
  JOIN goal_cup_tracer.entries e ON e.id = c.entry_id JOIN sel ON sel.id = e.run_id
)
SELECT jsonb_build_object(
  'section', 'summary',
  'runs', (SELECT count(*) FROM sel),
  'runs_open', (SELECT count(*) FROM sel WHERE status = 'open'),
  'runs_beating_10m', (SELECT count(*) FROM last_beat WHERE at >= now() - interval '10 minutes'),
  'runs_open_silent_10m', (SELECT count(*) FROM sel LEFT JOIN last_beat b ON b.run_id = sel.id
                             WHERE sel.status = 'open' AND now() < sel.deadline_at
                               AND (b.at IS NULL OR b.at < now() - interval '10 minutes')),
  'chunks', (SELECT count(*) FROM ch),
  'chunk_bytes', (SELECT coalesce(sum(length), 0) FROM ch),
  'chunk_p50', (SELECT percentile_disc(0.5) WITHIN GROUP (ORDER BY length) FROM ch),
  'chunk_p90', (SELECT percentile_disc(0.9) WITHIN GROUP (ORDER BY length) FROM ch),
  'chunk_max', (SELECT max(length) FROM ch),
  'ack_lag_p50', (SELECT percentile_disc(0.5) WITHIN GROUP (ORDER BY bytes) FROM lag),
  'ack_lag_p90', (SELECT percentile_disc(0.9) WITHIN GROUP (ORDER BY bytes) FROM lag),
  'ack_lag_max', (SELECT max(bytes) FROM lag))::text;

ROLLBACK;
