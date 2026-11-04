# cr-upload-server notes (crates/goalcup/src/upload, server/worker, docs/UPLOAD-API.md)

## Findings (top first)
- important: no ceiling/alert on stored bytes and no rate limit on any authenticated route; practice runs unlimited in number, final seq client-chosen => unbounded R2 writes by any member. server/worker/src/chunks.ts:112-119 (log line only), ratelimit.ts (device flow only).
- important: GitHub is a hard dependency of every competition open (runs.ts:209-211, 2 calls/open, 502 => client waits) and shares one token with the cron (notices.ts:36, up to 50 runs x 8 calls/tick). Start-window storm / GitHub secondary limit => nobody can open; retries amplify. Doc says "never refuses a player".
- important: goalcup's own server client ignores HTTPS_PROXY and system roots (upload/client.rs:213-229): proxy-only / TLS-inspecting networks cannot sign in, open or upload. (same theme as cr-proxy)
- important (test gap): production SQL path (commit_chunk/seal/heartbeat) never run by Worker tests; LocalDb vs SQL parity is by hand (agree arch-5). check-sql.sh smoke only.
- minor: server stores absolute `workspace` path (OS user name) while RULES.md:344-347 / worker README say no user name (cli/approve.rs:131, runs.ts:142).
- minor: expired device_requests never purged (SQL deletes test rows only); unauthenticated writes 120/min/IP; limiter DO per random code.
- minor: poll_device mints+marks picked_up in one step; lost reply => credential unrecoverable, redo browser approval (db-supabase.ts #rpc 10 s, SQL poll_device).
- minor: held-chunk retry matches offset+length+head only (chunks.ts:48); not sha256. Client then confirms+prunes on the echo (send.rs:180). Unreachable today, one-line hardening.
- minor: PUT whole-call 600 s with chunks up to 32 MiB, no shrink after timeout (client.rs:18, chunk.rs:16): uplink < ~55 KB/s never finishes a full chunk.
- minor: dead code: `dev` (client never sends; runs.ts:192,255 + UPLOAD-API.md:174), disk_full notice, `credential()` result unchecked (db-supabase.ts:206).
- minor: doc drift: 429 "Too many calls" is device flow only; "1.0 needs none" SQL (UPLOAD-API.md:11) vs stored_bytes migration; Content-Length required unstated.
- minor: delete-test-runs.sh failure advice re-runs without --older-than (empties all test prefixes); DB commits before R2 deletes.

## Strengths
- Idempotent chunk protocol: offset/prev/first_seq, stored-chunk retry before run state, create-only R2 key with hash in name, atomic SQL step under FOR UPDATE, seal by either of two orders + client repeat for the race.
- Client: one request per call, no redirects, no proxy for token, redacted Token, fixed-words logs, durable-point bound, flock, atomic 0600 upload.json, prune only after synced upload.json.
- Credential: 256-bit, only sha256 stored, bound to one run, 404 not 403 across members. invocation_logs off. Test key gated by STAGING/local.

## Re other lanes
- cr-proxy (proxy chaining, corp roots): agree, extra evidence: upload/client.rs:228 try_proxy_from_env(false) and webpki-only roots mean even goalcup's own device flow/open/PUT fail on those networks, before any harness runs.
- arch-5 agree (dual DB); I rate the missing SQL-path test important.
- docs-6 agree: Worker still has dev handling, client never sends it (cli/approve.rs:128-132).
- cr-watchdog respawn-forever: no upload-side impact; uploader threads are guarded and restart themselves.
## Inbox: none received.
- versions (server-fetched compat file): feasible on my side. The Worker already stores harness_version per entry (runs.ts:144, free string <=64, "unknown" allowed) and gates goalcup itself with MIN_COMPETITION_VERSION (runs.ts:113). A GET /v1/compat (static JSON from R2 or a var, unauthenticated, cacheable, never blocking: client falls back to built-in) fits the 7-route Worker; must not be able to refuse a start (point 3) and must be signed or at least served only from ingest host over TLS.
- cr-format-verify 2 (check without --known runs no secret scan): server stores whatever arrives (chunks.ts "server checks only these headers"), so the verifier is the only secret check; agree important.

## Answers
Inbox: no inbox-cr-upload-server.md existed; no questions to answer.

## Round 2 responses
- usability-6: agree, server side confirmed. device.ts:55 only runs requireMinimumVersion at request time; window/closed/abandoned checks (runs.ts:69-70) fire at open, after browser approval. Fix: check window/status at device request (DB only, no GitHub).
- versions-3/4 (compat data): a GET /v1/compat from R2 is feasible; Worker already stores harness_version free text (runs.ts:144). Must never refuse a start.
- arch-3: agree, same as my test-gap finding.
