# Notifications: one system, run by the server (21 Oct 2026)

Owner, 21 Oct 2026: "the notification system should be one consistent system; it is handled by
the server, not the proxy."

This page lists every message a participant can get about a run. Email is the server's channel.
The terminal and `goalcup status` lines are shown for comparison: they are goalcup's own words on
the participant's computer, and they should say the same thing as the email. Sources:
`server/worker/src/notices.ts` (when an email is queued), `goal_cup_tracer.notice_text` in
`server/worker/sql/` (the words), `crates/goalcup-cli/src/participant.rs` (the terminal words),
and `docs/research/PARTICIPANT-REVIEW-2026-10-21.md` (A1, A3, A6, A7).

## Rules for every message

1. **The server decides.** Every email is decided in one place: the server's cron, every 5 minutes.
   It uses only facts the server holds. No request handler and no goalcup sends one. The words
   live in one place: `notice_text` in the database.
2. **One voice, one command.** A message names one command. The email and the terminal name the
   same one. When the server cannot know which command is right, it says: run `goalcup status`
   and do what its Next line says.
3. **Never `goalcup start` during a run.** A new start replaces the run. No message advises it
   while a run is open. The silence email says so plainly: do not run `goalcup start`.
4. **Never "your run stopped" unless the server knows.** The server knows a run is over only at
   its deadline, when a newer run replaced it, or when it closed. Silence is not one of these.
5. **No alarm during our own trouble.** If the server itself was not well, it sends no silence
   email. See "The silence rule" below.
6. **Nothing goes to the agent.** Messages go to the person. goalcup never types into the agent
   or writes to it about any of this (D66). The disk pause (D65) freezes the agent; it sends no
   words.
7. **Once per thing.** Each email goes at most once per run, kind and key. Each has a limit, shown
   in the table.
8. **Plain words.** "Run" and "part" only. Times in UTC. No jargon such as entry, segment, chain or
   anchor.

## Variants

- **Competition run:** every email below goes to the account's email address.
- **Test run** (organizers only): the same emails go to the operator, with `[TEST]` before the
  subject.
- **Practice run:** no email at all. The terminal and `goalcup status` only.
- **Staging server:** every email goes to its test address, with `[staging]` before the subject.

## Every event

"Today" is the code at this commit. "Proposed" is not built yet and needs a decision (see the
last sections). Times like `X` are filled in by the server.

| # | Event | Trigger (server facts only) | Email: timing and limit | Email words | Terminal and `goalcup status` today | The one action |
|---|---|---|---|---|---|---|
| 1 | Run started | A competition or test run is open and before its end. | `started`. First tick after the open (within 5 min). Once per run. | **Goal Cup: your agent started.** "Your agent started its Goal Cup run at X (UTC). The run ends at Y (UTC). Repository: R. To check on it, run `goalcup status` on the machine that started it." (unchanged) | "Your competition run is open." ... "Your run ends at Y. You do not need to stop it." | Nothing. To check: `goalcup status`. |
| 2 | Silence (no heartbeat) | The run's last heartbeat (or its start, if none came) is more than 20 min old, **and** the server has been well for more than 20 min (see "The silence rule"). | `silence`. First tick that meets both. Once per silence (key: the last heartbeat). A run that comes back and goes quiet again gets a new one. | **Goal Cup: we have not heard from your run.** "We have not heard from your run since X (UTC). If the computer is offline or asleep, recording goes on and catches up later. At the computer that runs it, run `goalcup status` and do what its Next line says. Do not run `goalcup start`: a new start replaces this run." (**changed now**) | `PROBLEM: the server has not heard from this run for about X (why). Check the network.` If goalcup itself stopped: `goalcup has not updated this run for X. The computer may have slept or restarted. To go on, run: goalcup continue <run>` | `goalcup status`, then its Next line. |
| 3 | Idle 3 h | The run is beating (heartbeat within 20 min), has reported model calls at least once, and its last new model call is more than 3 h old. | `idle_3h`. Once per idle period (key: the last model call). | Today: **Goal Cup: no model calls for 3 hours.** "Your agent made no model calls for 3 hours (since X UTC): is it finished, out of tokens, or offline?" Proposed: "Your agent has made no model calls for 3 hours (since X UTC). Your run is still open and recording. Look at the agent's window. If the agent has stopped, type keep going. That is all you may type." | None. | Look at the agent. If it stopped: type keep going. |
| 4 | Idle 6 h | The same, 6 h. | `idle_6h`. Once per idle period. | The same as row 3, with 6 hours. | None. | The same as row 3. |
| 5 | Recording problem | A heartbeat said the recording has `gaps` or `failed`. | `recording`. Once per run. | Today: "Goal Cup Proxy reports a recording problem (gaps). Run `goalcup status` on the machine that started the run." Proposed, the same as the terminal: gaps: "Part of what happened was not recorded. Keep the run going. If this keeps happening, write to goalcup@agentcommunity.org." failed: "The recording failed. Write to goalcup@agentcommunity.org and name run R." | gaps: `PROBLEM: the recording has gaps: part of what happened was not recorded. Keep the run going. If this keeps happening, write to goalcup@agentcommunity.org.` failed: `PROBLEM: the recording failed. Write to goalcup@agentcommunity.org and name run R.` | gaps: nothing. failed: write to support. |
| 6 | Disk paused | A heartbeat's vitals say goalcup paused the agent for the disk (`agent_paused: "disk"`), starting a new pause. | `disk_full`. Within one tick. At most one per run per 6 h. | **Goal Cup: your agent is paused, the disk is almost full.** "Goal Cup Proxy paused your agent at X (UTC) because the disk of the computer running it is almost full. Free space outside your run's folder. Do not delete anything the agent made. The agent resumes by itself. If it still seems stuck after that, type "keep going" in the agent." (**changed now**) | `The disk is almost full, so your agent is paused. Free some disk space; it resumes by itself. If your agent seems stuck afterwards, type keep going.` | Free space outside the run's folder. |
| 7 | Disk short (before a pause) | None the server uses. The vitals carry `disk_short`. | No email. | None. | `PROBLEM: only X of disk is free. Free some space.` | Free space outside the run's folder. |
| 8 | Upload behind | Proposed: the run beats, but the server has stored no new part of it for 2 h while goalcup reports more on disk. The server does not keep this fact for the cron yet. | Today: no email. Proposed: an organizer alert, not a participant email (the cause is often ours, D64). | None. | `PROBLEM: nothing was uploaded for X (why). Nothing is lost; it waits on this computer. Check the network.` | Nothing. goalcup retries by itself. |
| 9 | Server refuses uploads (D64) | The server's own fault. | No email. Organizers fix the server. | None. | `PROBLEM: the server refuses this run's uploads (HTTP N). Recording continues on this computer and nothing is lost; it is sent once the server takes it again. Nothing to do.` | Nothing. |
| 10 | Run ended | The run reached its deadline and was not replaced. The judged commit is recorded, or an hour has passed without it. | `ended`. 5 to 60 min after the end. Once per run. | **Goal Cup: your agent finished.** "Your agent finished at X (UTC). The commit we recorded is C in R." (or "No commit was recorded for R.") Then, if the server holds the run sealed: "Your run is sealed on the server. Nothing more to do." If not: "Part of your run has not arrived yet. On the computer that ran it, run `goalcup upload` before Z (UTC). It sends only what is missing. After that time the server takes no more uploads." Z is the end plus 24 h. (**changed now**) | `Time is up. <harness> was stopped.` Then `Your run is sealed and uploaded. Nothing more to do.` or `Your run is sealed on this computer.` / `Part of the run is not uploaded yet. Next: when you are online, run: goalcup upload` | Nothing, or `goalcup upload` before Z. |
| 11 | Upload incomplete before the 24 h close | Proposed: the run is over, not replaced, and still not sealed on the server 18 h after its end. | Proposed new kind `close_soon`. Once per run, 6 h before the close. | Proposed: **Goal Cup: part of your run has not arrived.** "Your run ended at X (UTC), but part of it has still not arrived. On the computer that ran it, run `goalcup upload` before Z (UTC). After that time the server takes no more uploads." | `Next: run goalcup upload`, or for a computer that died before the seal `Next: run goalcup seal, then goalcup upload` | `goalcup upload` before Z. |
| 12 | Replaced | A newer competition start on the account replaced the run (`replaced_at`). | Today: no email. Proposed new kind `replaced`. Once per run, at the next tick. | Proposed: **Goal Cup: your run was replaced.** "Your run R was replaced at X (UTC) by a newer run on your account. Only the newer run counts. If you did not start a new run, write to goalcup@agentcommunity.org now." | On the old run's computer: `This run was replaced by a newer run on your account. <harness> was stopped.` / `Only the newer run counts.` | Nothing, or write to support if you did not start it. |
| 13 | Our own trouble (missed ticks, database down, maintenance) | The server's health marker (see below). | No participant email. Proposed: one notice on the website, by the organizers. | None. | The terminal's network or server lines (rows 2, 8, 9). | Nothing. |

## The silence rule (built now)

The old silence email said "Your run stopped. Run `goalcup continue`." after 20 min without a
heartbeat. A wifi drop, a sleeping computer and our own outage all look like that. During a server
problem everyone got it at once, and a worried participant might run `goalcup start`, which
replaces the run.

The new rule is in `server/worker/src/notices.ts` (`noteTick`):

1. The cron keeps a small marker in the server's storage bucket: the time of its last tick, and
   since when the server has been well.
2. A tick is **well** when two things are true. The tick before it ran at most 11 min earlier (at
   most one 5-minute tick was missed). And some run's heartbeat, of any mode, reached the server in
   the last 10 min.
3. Anything else starts the well stretch again at that tick: two ticks in a row that were missed
   or that the database failed, the maintenance switch (it deletes the marker), no heartbeat from any run, or a marker
   that cannot be read.
4. A silence email goes out only when the run's last heartbeat **and** the start of the well
   stretch are both more than 20 min old.

So nobody is mailed during our trouble. After it, every run has 20 min to come back. goalcup's
retries are at most about 15 min apart (D64), so a healthy run is back in time. The email's key is still the
run's last heartbeat, so a silence that outlasts our trouble is mailed once, after it.

**The limits.** A run that is the only one beating cannot be told apart from a server nobody
reaches. So a lone run gets no silence email. This matters for a small dry run, and for an
organizer testing the silence email: they need a second run that beats. Trouble that hits only part
of the world (one network or region) still looks like silence and is still mailed.

Tests: `server/worker/test/notices.test.ts` (missed ticks, a fleet gone quiet, a lone run,
maintenance, an unreadable marker, and a healthy server with one silent run).

## What changes now (this change)

- **Silence email:** new subject and words (row 2), and the silence rule above.
- **Ended email:** the server adds two facts to its payload: whether the run is sealed on the
  server (`uploaded`), and when it closes (`closes_at`). The email then says either "Nothing more to
  do" or "run `goalcup upload` before Z" (row 10).
- **Disk paused email:** says where to free space, and not to delete the agent's work (row 6).
- The words are a new pending migration,
  `server/worker/sql/pending/20261021140000_goal_cup_tracer_notice_text.sql`. It replaces only
  `notice_text`. It works in either order with the Worker.
- `docs/UPLOAD-API.md`, "What the server does on its own", describes the rule and the payload.

## Proposed, not built

- **Idle words** (rows 3 and 4): drop "offline" (a beating run is online) and say the one thing to
  do. Waits for the owner's ruling on mid-run keystrokes and quota (review B3).
- **Recording words** (row 5): split `gaps` and `failed` and match the terminal.
- **New email `replaced`** (row 12): a new start replaces a run with no email today. It is also a
  security signal if someone else started it.
- **New email `close_soon`** (row 11): one reminder 6 h before the close, if the run is still not
  sealed on the server.
- **Upload behind** (row 8): an organizer alert, not a participant email. The cron would need the
  server's stored length and goalcup's reported length per run.
- **Terminal lines** to match the emails (crate changes, outside this change):
  - After the upload, `Part of the run is not uploaded yet; goalcup continue sends it.` should name
    `goalcup upload`, like everything else (A3).
  - A dead run past its deadline should say `goalcup seal, then goalcup upload`, not "run again
    once the run has ended" forever (A3). The ended email's `goalcup upload` relies on this fix.
  - The preflight refusal during `goalcup continue` should say `goalcup continue <run>. Your run is
    still open.`, not "start again" (A6).
  - The disk pause line should say "Free space outside your run's folder; do not delete anything the
    agent made." like the email and the guide (A7).
  - The heartbeat problem line could add "Your run is still open; nothing is lost." to "Check the
    network."

## Follow-ups

- `uploaded` means the run's last part is sealed on the server. An earlier part could still be
  missing. To say so, the cron needs per-part facts from the database.
- Times are shown as `2026-11-08T03:10:00Z (UTC)`. A friendlier format is the website's call.
- A lone run gets no silence email (see the limits). If that is not acceptable, the next step is an
  organizer alert, not a participant email.

## Needs the owner's or the website's decision

- **The website owns mail delivery** (Resend, the address, suppression) **and may own the words.**
  Please approve the new words for `silence`, `ended` and `disk_full`, then merge the pending
  migration in the website repo (`sql/pending/README.md`).
- Is it all right that a lone run gets no silence email?
- Add the `replaced` and `close_soon` emails? Each needs a new kind in the database's CHECK
  constraint, `notice_text` words and a Worker change.
- Idle words: what may a participant do when the agent's quota or plan runs out (B3)?
- Upload behind: organizer alert only, or a participant email too?
- Our own trouble: a notice on the website, one email from the organizers, or nothing?
- Practice runs: stay email-free?
