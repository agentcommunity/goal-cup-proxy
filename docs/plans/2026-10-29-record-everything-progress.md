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
| 3 | Upload reads only its own tunnel list | merged to main in #290 (444aa15); gate PASS, live PASS |
| 4 | Probe only what the run needs | not started |
| 5 | Up to 8 session-file rows; Claude subagents captured | not started |
| 6 | Copy a harness database once, after exit | not started |
| 7 | Header stream per exchange, credentials masked | not started |
| 8 | Evidence label counts model requests only | PR #289 open; main merged into it (1eda6db); gate re-running; live PASS before the merge |
| 9 | Codex records an API-key login | not started |
| 10 | Proxy restarts itself on the same port | not started |
| 11 | Participant sees "recording works" and the right file | not started |
| 12 | Seams, operations and docs | not started |
| 13 | Measured runs (build and test first; long runs later) | not started |
| 14 | Owner steps (for you) | not started |

## Log

- 28 Oct: Phase 0 map done. Nine readers checked the plan against main. 99 claims true, 7 moved,
  44 wrong, 4 conflicts. The wrong ones changed design for Tasks 2, 3, 4, 6, 7, 8, 10, 11, 12 and 13.
- 28 Oct: Build decisions written (one page, for every builder). The spec wins over older decisions
  where it narrows them; each such change goes in the ledger with the reason.
- 28 Oct: crates.io reachable; local Rust 1.99.0 builds; `origin/main` is b6b220b.
- Model names are not yet confirmed. Web search found no official "GPT-6.1 Luma" id, and the repo
  uses gpt-5.6-luna. The live check settles it with a real call and the name that worked is recorded.
- 28 Oct: Live kit proven on main for claude-code-cli, codex-cli (gpt-5.6-luna, since gpt-6.1-luma returned 404) and hermes-cli (deepseek-flash). Sandbox gate on main PASS in 306s.
- 28 Oct: Task 8 review: one medium finding (a test that would not catch a revert) fixed; one low finding fixed (WebSocket client messages). Owner calls: schema version bump, and the Hermes and Amp wording.
- 28 Oct: Task 3 merged in #290 (444aa15). Gate PASS in 146s with Worker tests. Live Claude run PASS, but 7 wire calls against 10 BEFORE; the kit does not print the tunnel-list line, so acceptance is inferred. Deploy the Worker before any client from main seals (owner step).
- 28 Oct: Commit messages. Two earlier ledger commits and one Task 3 commit (3f6f055 is clean; 4b7b8fa, 35abe88, fddcf87 and fcf13ee carry a model name in a Co-Authored-By line). Not rewritten: owner to decide. The system rule says no model id in commits.
