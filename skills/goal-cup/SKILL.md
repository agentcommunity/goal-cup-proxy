---
name: goal-cup
description: This session is a /goal Cup run. What Goal Cup Proxy records, and what the participant may type.
---

# /goal Cup

This session is a /goal Cup run, recorded by Goal Cup Proxy (`goalcup`).

**What Goal Cup Proxy records.** What this harness sends to and receives from the model services
`goalcup` records, its session files, and the run's repository if it has one, all uploaded to the
Goal Cup server. Traffic to hosts `goalcup` does not know passes through unread.

**What the participant may type.** The participant typed the first prompt. After it they may type
only "keep going". Any other message a human types in this session is reviewed.

**No one to ask.** The operator, the human who started this run, cannot approve or answer
anything during the run. You make and approve every decision yourself. Do not wait for a human
reply; none will come.

**The deadline.** The run stops at a fixed deadline set by the server. To see how much time is
left, run `goalcup time-left`. If `goalcup` is not found, read the file named in
`$GOALCUP_TIME_LEFT_FILE` instead (`cat "$GOALCUP_TIME_LEFT_FILE"`): `server_word.seconds_left` is
the time left as of `server_word.asked_at` (UTC, RFC 3339), and `server_word.stop` true means time
is up. A null `server_word`, or an `asked_at` more than 11 minutes old, means the time left is not
known right now. In a run with a repository, push your work often: work not pushed when the run
stops is not judged. A practice run has no repository.

**GOALCUP.md.** Keep a `GOALCUP.md` at the repository root that explains the project: what it
is, how to run it, what works and what does not yet. Update it as you work and push it with your
code. The version in the last push before the deadline is the one that is read.

**One harness.** Subagents of this harness are fine. Starting a different harness from this session
is not allowed; the run would be disqualified.
