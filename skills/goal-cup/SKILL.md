---
name: goal-cup
description: This session is a /goal Cup run. What Goal Cup Proxy records, and what the participant may type.
---

# /goal Cup

This session is a /goal Cup run, recorded by Goal Cup Proxy (`goalcup`).

**What Goal Cup Proxy records.** Everything this harness sends to its model provider and everything
it receives, the harness's own session files, and the commits in the run's repository. The record
is sealed when the run ends and uploaded to the Goal Cup server.

**What the participant may type.** The participant typed the first prompt. After it they may type
only "keep going". Any other message a human types in this session is reviewed.

**No one to ask.** The operator, the human who started this run, cannot approve or answer
anything during the run. You make and approve every decision yourself. Do not wait for a human
reply; none will come.

**The deadline.** The run stops at a fixed deadline set by the server, 168 hours after the
official start. To see how much time is left, call the `time_left` tool of the `goalcup` MCP
server if this session has it. Otherwise run `goalcup time-left`. If `goalcup` is not found,
read the file named in `$GOALCUP_TIME_LEFT_FILE` instead (`cat "$GOALCUP_TIME_LEFT_FILE"`):
`seconds_left` is the time left, `deadline_at` the deadline in UTC, and `as_of` when it was
written; a value more than five minutes old is not current. Push your work often: work not pushed
when the run stops is not judged.

**GOALCUP.md.** Keep a `GOALCUP.md` at the repository root that explains the project: what it
is, how to run it, what works and what does not yet. Update it as you work and push it with your
code. The version in the last push before the deadline is the one that is read.

**One harness.** Subagents of this harness are fine. Starting a different harness from this session
is not allowed; the run would be disqualified.
