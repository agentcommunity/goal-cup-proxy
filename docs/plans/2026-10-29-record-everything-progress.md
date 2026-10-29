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
| 4 | Probe only what the run needs | merged in #294 (a3987d6); gate PASS, live PASS; the container restart lost the pipeline once, resumed by hand |
| 5 | Up to 8 session-file rows; Claude subagents captured | merged in #296 (eb9ad87); gate PASS, live PASS, subagent transcript in the chain; budget is an owner decision (L33) |
| 6 | Copy a harness database once, after exit | not started |
| 7 | Header stream per exchange, credentials masked | merged in #293 (1919549); gate PASS, live PASS (44 header streams, [masked] present, no Bearer) |
| 8 | Evidence label counts model requests only | merged in #289 (5ce4b2a); gate PASS on rerun, live PASS |
| 9 | Codex records an API-key login | merged in #295 (2bf49ac, squashed; a model-named trailer stayed off main); gate PASS, live PASS; subagent proof still needed in Task 13 |
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
- 28 Oct: Task 7 merged in #293 (1919549). The sandbox gate failed one kit test on the first two heads: the kit matcher counted the new header stream as a second request. The test now skips artifact-hint streams (fce86a4). Owner notes: known-prefix credentials in non-credential headers rely on the daemon redactor; query masking is wider than the spec wording.
- 28 Oct: Task 4 merged in #294 (a3987d6). One rule for hard and warn hosts (model_host::provider_hosts), used by start and setup. Test-only seam GOALCUP_TEST_PROBE_ANSWERS needs owner awareness. D74 needs a line for the default-host change. The container restarted during the Task 4 fix round; the uncommitted refactor was compiled, tested, committed and reviewed by hand.
- 28 Oct: Task 9 merged in #295 (squash, 2bf49ac). Review found that a ChatGPT-login run could be refused for the API host; fixed by probing only the first host without a model service. The fix commit carried a model-named Co-Authored-By line; the squash keeps it off main.
- 29 Oct: Task 5 merged in #296 (eb9ad87). The live run put the subagent transcript in the chain. The kit's own subagent counter reads 0, which is a bug in that counter, not the chain. Owner decision before any long run: the native budget (32 open append-only streams, 1024 files) can lose transcripts past about 32 subagents (ledger L33).
- 29 Oct: The container restarted twice more; both interrupted pipelines were resumed by hand from the branch state.
