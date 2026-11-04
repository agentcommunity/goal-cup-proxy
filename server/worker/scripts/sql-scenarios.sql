-- The Worker's scenarios on the real SQL (check-sql.sh runs this after the smoke calls). Each one
-- mirrors a LocalDb test, so db-local.ts and the website's functions answer alike. Production caller
-- (no request.jwt.claims) unless a scenario says staging. Any failed ASSERT stops the script.
DO $$
DECLARE
  u   constant text := '55555555-5555-5555-5555-555555555555';
  z   constant text := repeat('0', 64);
  h1  constant text := repeat('1', 64);
  h3  constant text := repeat('3', 64);
  h4  constant text := repeat('4', 64);
  h9  constant text := repeat('9', 64);
  t0  constant timestamptz := now();
  -- A chunk is b || its entry, offset, seqs, prev and head.
  b   constant jsonb := jsonb_build_object('length', 10, 'sha256', z, 'r2_key', 'k', 'received_at', t0, 'ack', '{}'::jsonb);
  n   int;
  r   jsonb;
  r0  jsonb;
  k   text;
BEGIN
  INSERT INTO goal_cup_tracer.consents (member_id, terms_version) VALUES (u::uuid, goal_cup_tracer.current_terms_version());
  -- One practice run per scenario: run and entry n, credential n. Run 56 ended 3 days ago.
  FOREACH n IN ARRAY ARRAY[51, 53, 54, 55, 56, 71] LOOP
    INSERT INTO goal_cup_tracer.run_credentials (credential_hash, member_id, mode, expires_at, staging)
    VALUES (lpad(n::text, 64, '0'), u::uuid, 'practice', t0 + interval '1 hour', false);
    r := public.goal_cup_upload_open_run(u, 'goalcup-practice', 'practice', 'r' || lpad(n::text, 16, '0'),
           'e' || lpad(n::text, 16, '0'), (t0 - CASE n WHEN 56 THEN interval '10 days' ELSE interval '0' END)::text,
           (t0 + CASE n WHEN 56 THEN interval '-3 days' ELSE interval '7 days' END)::text, 'c', z, z, 'w', 's' || n,
           '1', '1', 'https://github.com/o/r' || n, 'null'::jsonb, NULL, NULL, lpad(n::text, 64, '0'),
           (t0 + interval '8 days')::text);
    ASSERT r ->> 'result' = 'created', 'open run ' || n || ': ' || r::text;
  END LOOP;

  -- 1. commit advances offset, head, next_seq, stored_bytes, chunk_count; the same chunk again is a
  --    conflict and adds nothing; the staging Worker does not see a production run.
  k := public.goal_cup_upload_commit_chunk(u, 'r0000000000000051', b || jsonb_build_object('entry_id', 'e0000000000000051',
         'offset', 0, 'first_seq', 0, 'last_seq', 4, 'prev', z, 'head', h4), 1000::bigint);
  ASSERT k = 'committed', 'scenario 1: commit: ' || k;
  r0 := goal_cup_tracer.run_json('r0000000000000051');
  ASSERT r0 -> 'run' ->> 'stored_bytes' = '1000' AND r0 -> 'run' ->> 'chunk_count' = '1'
     AND r0 -> 'entry' ->> 'offset' = '10' AND r0 -> 'entry' ->> 'next_seq' = '5' AND r0 -> 'entry' ->> 'head' = h4,
     'scenario 1: after commit: ' || r0::text;
  k := public.goal_cup_upload_commit_chunk(u, 'r0000000000000051', b || jsonb_build_object('entry_id', 'e0000000000000051',
         'offset', 0, 'first_seq', 0, 'last_seq', 4, 'prev', z, 'head', h4), 1000::bigint);
  ASSERT k = 'conflict', 'scenario 1: same chunk again: ' || k;
  ASSERT goal_cup_tracer.run_json('r0000000000000051') = r0, 'scenario 1: the same chunk again changed the run';
  PERFORM set_config('request.jwt.claims', '{"role": "goal_cup_ingest_staging"}', true);
  k := public.goal_cup_upload_commit_chunk(u, 'r0000000000000051', b || jsonb_build_object('entry_id', 'e0000000000000051',
         'offset', 10, 'first_seq', 5, 'last_seq', 5, 'prev', h4, 'head', h1), 10::bigint);
  ASSERT k = 'not_found', 'scenario 1: staging saw a production run: ' || k;
  PERFORM set_config('request.jwt.claims', '', true);

  -- 2. A wrong offset, a wrong prev, a wrong first_seq: each a conflict, and nothing changes.
  k := public.goal_cup_upload_commit_chunk(u, 'r0000000000000051', b || jsonb_build_object('entry_id', 'e0000000000000051',
         'offset', 11, 'first_seq', 5, 'last_seq', 5, 'prev', h4, 'head', h1), 10::bigint);
  ASSERT k = 'conflict', 'scenario 2: wrong offset: ' || k;
  k := public.goal_cup_upload_commit_chunk(u, 'r0000000000000051', b || jsonb_build_object('entry_id', 'e0000000000000051',
         'offset', 10, 'first_seq', 5, 'last_seq', 5, 'prev', h3, 'head', h1), 10::bigint);
  ASSERT k = 'conflict', 'scenario 2: wrong prev: ' || k;
  k := public.goal_cup_upload_commit_chunk(u, 'r0000000000000051', b || jsonb_build_object('entry_id', 'e0000000000000051',
         'offset', 10, 'first_seq', 6, 'last_seq', 6, 'prev', h4, 'head', h1), 10::bigint);
  ASSERT k = 'conflict', 'scenario 2: wrong first_seq: ' || k;
  ASSERT goal_cup_tracer.run_json('r0000000000000051') = r0, 'scenario 2: a refused chunk changed the run';

  -- 3. After the final heartbeat commits seq 9: a chunk ending at 10 is beyond_final, one ending at 9
  --    with another head too; one ending at 9 with the final head commits.
  k := public.goal_cup_upload_heartbeat(u, 'r0000000000000053', 'e0000000000000053', 9, h9, 10, true, NULL, 'ok', 1,
         NULL, NULL, t0::text);
  ASSERT k = 'final_committed', 'scenario 3: final: ' || k;
  k := public.goal_cup_upload_commit_chunk(u, 'r0000000000000053', b || jsonb_build_object('entry_id', 'e0000000000000053',
         'offset', 0, 'first_seq', 0, 'last_seq', 10, 'prev', z, 'head', h1), 10::bigint);
  ASSERT k = 'beyond_final', 'scenario 3: ends at 10: ' || k;
  k := public.goal_cup_upload_commit_chunk(u, 'r0000000000000053', b || jsonb_build_object('entry_id', 'e0000000000000053',
         'offset', 0, 'first_seq', 0, 'last_seq', 9, 'prev', z, 'head', h1), 10::bigint);
  ASSERT k = 'beyond_final', 'scenario 3: ends at 9, other head: ' || k;
  k := public.goal_cup_upload_commit_chunk(u, 'r0000000000000053', b || jsonb_build_object('entry_id', 'e0000000000000053',
         'offset', 0, 'first_seq', 0, 'last_seq', 9, 'prev', z, 'head', h9), 10::bigint);
  ASSERT k = 'committed', 'scenario 3: ends at 9, final head: ' || k;

  -- 4. Seal: no_final without a final, chain_behind before the chain reaches it; both orders seal;
  --    the same seal again is already_sealed, another digest a conflict.
  --    Run 54: the chunk, then the final heartbeat.
  r := public.goal_cup_upload_seal_run(u, 'r0000000000000054', 'e0000000000000054', h1, h3, 'sk', 'bk', t0::text);
  ASSERT r ->> 'result' = 'no_final', 'scenario 4: 54 before anything: ' || r::text;
  k := public.goal_cup_upload_commit_chunk(u, 'r0000000000000054', b || jsonb_build_object('entry_id', 'e0000000000000054',
         'offset', 0, 'first_seq', 0, 'last_seq', 4, 'prev', z, 'head', h4), 10::bigint);
  ASSERT k = 'committed', 'scenario 4: 54 chunk: ' || k;
  r := public.goal_cup_upload_seal_run(u, 'r0000000000000054', 'e0000000000000054', h1, h3, 'sk', 'bk', t0::text);
  ASSERT r ->> 'result' = 'no_final', 'scenario 4: 54 no final: ' || r::text;
  k := public.goal_cup_upload_heartbeat(u, 'r0000000000000054', 'e0000000000000054', 4, h4, 10, true, NULL, 'ok', 1,
         NULL, NULL, t0::text);
  ASSERT k = 'final_committed', 'scenario 4: 54 final: ' || k;
  r := public.goal_cup_upload_seal_run(u, 'r0000000000000054', 'e0000000000000054', h1, h3, 'sk', 'bk', t0::text);
  ASSERT r ->> 'result' = 'sealed' AND r ->> 'run_status' = 'open', 'scenario 4: 54 seal: ' || r::text;
  --    Run 55: the final heartbeat, then the chunks.
  k := public.goal_cup_upload_heartbeat(u, 'r0000000000000055', 'e0000000000000055', 4, h4, 20, true, NULL, 'ok', 1,
         NULL, NULL, t0::text);
  ASSERT k = 'final_committed', 'scenario 4: 55 final: ' || k;
  r := public.goal_cup_upload_seal_run(u, 'r0000000000000055', 'e0000000000000055', h1, h3, 'sk', 'bk', t0::text);
  ASSERT r ->> 'result' = 'chain_behind', 'scenario 4: 55 nothing stored: ' || r::text;
  k := public.goal_cup_upload_commit_chunk(u, 'r0000000000000055', b || jsonb_build_object('entry_id', 'e0000000000000055',
         'offset', 0, 'first_seq', 0, 'last_seq', 3, 'prev', z, 'head', h3), 10::bigint);
  ASSERT k = 'committed', 'scenario 4: 55 chunk 0-3: ' || k;
  r := public.goal_cup_upload_seal_run(u, 'r0000000000000055', 'e0000000000000055', h1, h3, 'sk', 'bk', t0::text);
  ASSERT r ->> 'result' = 'chain_behind', 'scenario 4: 55 up to 3 of 4: ' || r::text;
  k := public.goal_cup_upload_commit_chunk(u, 'r0000000000000055', b || jsonb_build_object('entry_id', 'e0000000000000055',
         'offset', 10, 'first_seq', 4, 'last_seq', 4, 'prev', h3, 'head', h4), 10::bigint);
  ASSERT k = 'committed', 'scenario 4: 55 chunk 4: ' || k;
  r := public.goal_cup_upload_seal_run(u, 'r0000000000000055', 'e0000000000000055', h1, h3, 'sk', 'bk', t0::text);
  ASSERT r ->> 'result' = 'sealed' AND r ->> 'run_status' = 'open', 'scenario 4: 55 seal: ' || r::text;
  r := public.goal_cup_upload_seal_run(u, 'r0000000000000055', 'e0000000000000055', h1, h3, 'sk', 'bk', t0::text);
  ASSERT r ->> 'result' = 'already_sealed' AND r ->> 'run_status' = 'open', 'scenario 4: 55 again: ' || r::text;
  r := public.goal_cup_upload_seal_run(u, 'r0000000000000055', 'e0000000000000055', h9, h3, 'sk', 'bk', t0::text);
  ASSERT r ->> 'result' = 'conflict', 'scenario 4: 55 other digest: ' || r::text;

  -- 5. A run closed incomplete: commit, seal and heartbeat answer closed.
  k := public.goal_cup_upload_close_run(u, 'r0000000000000056', t0::text);
  ASSERT k = 'incomplete', 'scenario 5: close: ' || coalesce(k, 'null');
  k := public.goal_cup_upload_commit_chunk(u, 'r0000000000000056', b || jsonb_build_object('entry_id', 'e0000000000000056',
         'offset', 0, 'first_seq', 0, 'last_seq', 0, 'prev', z, 'head', h1), 10::bigint);
  ASSERT k = 'closed', 'scenario 5: commit: ' || k;
  r := public.goal_cup_upload_seal_run(u, 'r0000000000000056', 'e0000000000000056', h1, h3, 'sk', 'bk', t0::text);
  ASSERT r ->> 'result' = 'closed', 'scenario 5: seal: ' || r::text;
  k := public.goal_cup_upload_heartbeat(u, 'r0000000000000056', 'e0000000000000056', 0, h1, 10, false, NULL, 'ok', 1,
         NULL, NULL, t0::text);
  ASSERT k = 'closed', 'scenario 5: heartbeat: ' || k;

  -- 6. poll_device twice after approval: a credential once, then expired; one credential row.
  --    (Device 7, member 7, is approved for the parallel pickup in check-sql.sh.)
  INSERT INTO goal_cup_tracer.device_requests (device_hash, user_hash, purpose, mode, connector_id, tracer_version,
                                               created_at, expires_at, staging, status, member_id, decided_at)
  VALUES (repeat('6', 64), repeat('6', 64), 'run', 'practice', 'c', '1', t0, t0 + interval '15 minutes', false,
          'approved', '66666666-6666-6666-6666-666666666666', t0),
         (repeat('7', 64), repeat('7', 64), 'run', 'practice', 'c', '1', t0, t0 + interval '15 minutes', false,
          'approved', '77777777-7777-7777-7777-777777777777', t0);
  r := public.goal_cup_upload_poll_device(repeat('6', 64), t0::text);
  ASSERT r ->> 'result' = 'approved' AND r ->> 'credential' ~ '^gcr_[0-9a-f]{64}$', 'scenario 6: first poll: ' || (r ->> 'result');
  r := public.goal_cup_upload_poll_device(repeat('6', 64), t0::text);
  ASSERT r ->> 'result' = 'expired', 'scenario 6: second poll: ' || r::text;
  ASSERT (SELECT count(*) FROM goal_cup_tracer.run_credentials
           WHERE member_id = '66666666-6666-6666-6666-666666666666') = 1, 'scenario 6: not one credential row';

  -- 7. Purge (20261105100000): only this deployment's requests expired more than a day ago go.
  INSERT INTO goal_cup_tracer.device_requests (device_hash, user_hash, purpose, mode, connector_id, tracer_version,
                                               created_at, expires_at, staging)
  VALUES (repeat('d', 64), repeat('d', 64), 'run', 'practice', 'c', '1', t0 - interval '2 days',
          t0 - interval '2 days', false),
         (repeat('e', 64), repeat('e', 64), 'run', 'practice', 'c', '1', t0 - interval '1 hour',
          t0 - interval '1 hour', false),
         (repeat('f', 64), repeat('f', 64), 'run', 'practice', 'c', '1', t0 - interval '2 days',
          t0 - interval '2 days', true);
  n := public.goal_cup_upload_purge_device_requests(t0::text);
  ASSERT n = 1, 'scenario 7: purged ' || n;
  ASSERT NOT EXISTS (SELECT 1 FROM goal_cup_tracer.device_requests WHERE device_hash = repeat('d', 64)), 'scenario 7: old row kept';
  ASSERT (SELECT count(*) FROM goal_cup_tracer.device_requests WHERE device_hash IN (repeat('e', 64), repeat('f', 64))) = 2,
    'scenario 7: a young or a staging row went';
  ASSERT has_function_privilege('goal_cup_ingest_production', 'public.goal_cup_upload_purge_device_requests(text)', 'EXECUTE')
     AND has_function_privilege('goal_cup_ingest_staging', 'public.goal_cup_upload_purge_device_requests(text)', 'EXECUTE')
     AND has_function_privilege('service_role', 'public.goal_cup_upload_purge_device_requests(text)', 'EXECUTE'),
    'scenario 7: a Worker role cannot purge';
  ASSERT NOT has_function_privilege('anon', 'public.goal_cup_upload_purge_device_requests(text)', 'EXECUTE')
     AND NOT has_function_privilege('authenticated', 'public.goal_cup_upload_purge_device_requests(text)', 'EXECUTE')
     AND NOT has_function_privilege('public', 'public.goal_cup_upload_purge_device_requests(text)', 'EXECUTE'),
    'scenario 7: anon, authenticated or PUBLIC can purge';
END $$;
