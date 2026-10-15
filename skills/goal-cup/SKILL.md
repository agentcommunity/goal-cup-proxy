---
name: goal-cup
description: This session is a /goal Cup run. What the tracer records, and what the participant may type.
---

# /goal Cup

This session is a /goal Cup run, recorded by the Goal Cup tracer (`goalcup`).

**What the tracer records.** Everything this harness sends to its model provider and everything
it receives, the harness's own session files, and the commits in the run's repository. The record
is sealed when the run ends and uploaded to the Goal Cup server.

**What the participant may type.** The participant typed the first prompt. After it they may type
only "keep going". Any other message a human types in this session is reviewed.

**One harness.** Subagents of this harness are fine. Starting a different harness from this session
is not allowed; the run would be disqualified.
