# Record everything: build progress

29 Oct 2026. Short tracker for the [implementation plan](2026-10-29-record-everything-plan.md).
Scope for now: build and test Tasks 1 to 12 so we know they work. The 24-hour and 168-hour runs
on Tenki come later, after the build is clear and good.

## Guiding points

- Spec rules 1 to 17 are the target. Protocol v5 stays frozen; new data is new streams only.
- Keys: `CLAUDE_API_KEY` is the Claude key (used as `ANTHROPIC_API_KEY` inside the sandbox only).
  Keys go into 0600 files in the sandbox and into each harness's own settings there. Never in argv
  or output.
- Test models:
  - Claude Code: Haiku, lowest thinking setting.
  - Codex: OpenAI key, GPT-6.1 Luma, low thinking.
  - Other harnesses (Hermes and the rest): DeepSeek, latest flash release (`-1017`).
- Branches: one branch per task from `main`, one PR each, merged in plan order after the gate is green.
- When unsure, check the web or the code before acting. Keep it simple; no over-engineering.

## Task status

| Task | What | Status |
|---|---|---|
| 1 | Record every request on an opened host | not started |
| 2 | Clean model address; never Goal Cup's own host | not started |
| 3 | Upload reads only its own tunnel list | not started |
| 4 | Probe only what the run needs | not started |
| 5 | Up to 8 session-file rows; Claude subagents captured | not started |
| 6 | Copy a harness database once, after exit | not started |
| 7 | Header stream per exchange, credentials masked | not started |
| 8 | Evidence label counts model requests only | not started |
| 9 | Codex records an API-key login | not started |
| 10 | Proxy restarts itself on the same port | not started |
| 11 | Participant sees "recording works" and the right file | not started |
| 12 | Seams, operations and docs | not started |
| 13 | Measured runs (build and test first; long runs later) | not started |
| 14 | Owner steps (for you) | not started |

## Log

- 29 Oct: Phase 0 read-only map started (`CONTEXT.md` in the session scratchpad).
