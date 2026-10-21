-- Goal Cup tracer: new words for the silence, ended and disk_full emails (2026-10-21).
--
-- The run emails are one system, run by the server (owner, 21 Oct 2026). This changes only their
-- words (docs/research/NOTIFICATIONS.md has the reasons and the full table):
--   * silence: no longer says "Your run stopped". The server does not know that: a wifi drop, a
--     sleeping computer or our own outage look the same from here. It now says when we last heard
--     from the run, that recording goes on while the computer is offline or asleep, to run
--     `goalcup status` and do what its Next line says, and never to run `goalcup start` (a new
--     start replaces the run). The subject changes to match. The Worker of this change also mails
--     it only when the server itself was well for the whole silence (docs/UPLOAD-API.md, "The
--     server's health"); that rule is in the Worker, not here.
--   * ended: when the payload says part of the run has not arrived ("uploaded": false), the email
--     says to run `goalcup upload` before the close ("closes_at", the end + 24 hours). When it says
--     the run is in ("uploaded": true), it says nothing more is needed. A payload without the key
--     (queued by an older Worker) gets neither line, as before.
--   * disk_full: says where to free space: outside the run's folder, and never anything the agent
--     made.
-- The other kinds keep their words. The kinds and the CHECK constraint do not change.
--
-- Order: either way round with the Worker of this change. Before it, the ended payload has no
-- "uploaded" key, and the ended email reads as before.
--
-- Additive: one function replaced with the same signature (grants kept). No row is changed.
--
-- Apply: staging first, then production (lead agent applies, owner approves):
--   psql "$DATABASE_URL_REMOTE" -X -1 -v ON_ERROR_STOP=1 -f <this file>; then pnpm db:check.

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
