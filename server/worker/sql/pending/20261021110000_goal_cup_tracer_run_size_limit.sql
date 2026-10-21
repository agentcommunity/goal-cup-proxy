-- Goal Cup tracer: a per-run server size limit (owner approved, 2026-10-21; event-readiness plan WP-D).
--
-- The upload Worker refuses a chunk that would take a run past RUN_MAX_COMPRESSED_BYTES (a Worker
-- variable, 64 GiB) with 413 quota_exceeded, before it stores anything in R2. To know what a run
-- holds, the database now counts it:
--   * entries.stored_bytes: the compressed bytes of the entry's committed chunks (each chunk's
--     request Content-Length, which is what R2 stores); entries.chunk_count: how many chunks.
--     Both start at 0. Chunks committed before this migration are not counted (no compressed size
--     was kept for them); the counters cover every chunk from here on.
--   * chunks.stored_bytes: the compressed bytes of that chunk; null for chunks from before.
--   * goal_cup_tracer.run_json adds "stored_bytes" and "chunk_count" to the run object: the sums
--     over the run's entries. Every function that answers a run (position, open, continue,
--     runs_for_tick) carries them; a Worker that does not read them ignores them.
--   * public.goal_cup_upload_commit_chunk(p_member_id, p_run_id, p_chunk, p_stored_bytes): a new
--     overload. In the same atomic step as before it records the chunk and adds p_stored_bytes and
--     one chunk to the entry. The Worker of this change calls it.
--   * The old three-argument goal_cup_upload_commit_chunk stays for the Worker that is live while
--     this is applied; it now calls the new one with no size, so it still counts the chunk (and adds
--     0 bytes). A later migration can drop it once every Worker sends p_stored_bytes.
--
-- Order: apply this before deploying the Worker that sends p_stored_bytes (staging, then
-- production). The live Worker keeps working on it unchanged.
--
-- Additive: two new columns on entries and one on chunks (constant defaults or null, no table
-- rewrite), one function replaced, one new function with its grants. No row is changed.
--
-- Operator notices at 50% and 100% of the limit are Worker log lines (quota-half, quota-full,
-- quota-exceeded), not rows in goal_cup_tracer.notices: that outbox mails the run's member, and a
-- new kind would mean widening its CHECK constraint and a new recipient rule.
--
-- Apply: staging first, then production (lead agent applies, owner approves):
--   psql "$DATABASE_URL_REMOTE" -X -1 -v ON_ERROR_STOP=1 -f <this file>; then pnpm db:check.

ALTER TABLE goal_cup_tracer.entries ADD COLUMN stored_bytes bigint NOT NULL DEFAULT 0 CHECK (stored_bytes >= 0);
ALTER TABLE goal_cup_tracer.entries ADD COLUMN chunk_count  bigint NOT NULL DEFAULT 0 CHECK (chunk_count >= 0);
ALTER TABLE goal_cup_tracer.chunks  ADD COLUMN stored_bytes bigint CHECK (stored_bytes >= 0);

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

-- Supabase grants EXECUTE on new functions to PUBLIC, anon and authenticated; revoke all three
-- (docs/SUPABASE.md, "SECURITY DEFINER grants"). Then the same Worker roles as the old form.
REVOKE ALL ON FUNCTION public.goal_cup_upload_commit_chunk(text, text, jsonb, bigint) FROM PUBLIC, anon, authenticated;
GRANT EXECUTE ON FUNCTION public.goal_cup_upload_commit_chunk(text, text, jsonb, bigint)
  TO goal_cup_ingest_staging, goal_cup_ingest_production, service_role;
