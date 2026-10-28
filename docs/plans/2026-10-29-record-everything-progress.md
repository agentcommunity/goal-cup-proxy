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
| 1 | Record every request on an opened host | merged in #292 (bf8ce8b); gate PASS, live PASS |
| 2 | Clean model address; never Goal Cup's own host | merged in #291 (86b9dae); gate PASS, live PASS |
| 3 | Upload reads only its own tunnel list | merged in #290 (444aa15); gate PASS, live PASS |
| 4 | Probe only what the run needs | not started |
| 5 | Up to 8 session-file rows; Claude subagents captured | not started |
| 6 | Copy a harness database once, after exit | not started |
| 7 | Header stream per exchange, credentials masked | not started |
| 8 | Evidence label counts model requests only | merged in #289 (5ce4b2a); gate PASS on rerun, live PASS |
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
- 28 Oct: Commit messages. Four commits carry a model name in a Co-Authored-By line: 4b7b8fa, 35abe88, fddcf87 and 3f6f055. The system rule says no model id in commits. Not rewritten; owner to decide. From now on they are left out.
- 28 Oct: Task 8 merged in #289 (5ce4b2a). First gate run on the merged head failed one proxy test (an_upstream_outage_is_one_window_closed_by_the_next_reach). It passed 5 of 5 locally on main and on the rerun of the gate, so it is treated as a timing flake. Open item: the test reserves a port, drops it, then reconnects; a parallel test can take the port. Not fixed yet.
- 28 Oct: Task 2 pipeline started (build, review, fit, gate, live).
- 28 Oct: Task 2 merged in #291 (86b9dae). Review found two medium security gaps (a backslash in the URL authority; 0.0.0.0 and mapped loopback names). Both fixed before merge. Gate PASS, live PASS.
- 28 Oct: Task 1 merged in #292 (bf8ce8b). Review found a high gap: form-encoded refresh tokens were not masked. Fixed in the same PR; redaction changes sit outside the Task 1 file list, flagged to the owner. Push protection refused two fake test fixtures; rewritten in unpushed commits. Gate PASS, live PASS (10 model calls, 46 wire streams).
- Open for the owner: the redaction change in Task 1 (keep in Task 1 or split); the per-reason gap dedupe; the 16-byte floor; the Claude Co-Authored-By line rule (the attribution reminder asks for it, the system rule forbids a model id in commits). Earlier commits carrying a model name are listed above and not rewritten.
- Open item: the excluded-upgrade test was stale and is now upgrade_on_an_opened_host_is_captured_whatever_its_path.
