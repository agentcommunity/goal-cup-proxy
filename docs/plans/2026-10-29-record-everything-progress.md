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
  - Codex: OpenAI key, the GPT Luma model at low thinking (the name that the provider serves).
  - Other harnesses (Hermes and the rest): DeepSeek, latest flash release.
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
| 6 | Copy a harness database once, after exit | merged in #297 (cc44537); gate PASS, live PASS (state.db, -wal and -shm copied once at finish); owner: release order and trace size |
| 7 | Header stream per exchange, credentials masked | merged in #293 (1919549); gate PASS, live PASS (44 header streams, [masked] present, no Bearer) |
| 8 | Evidence label counts model requests only | merged in #289 (5ce4b2a); gate PASS on rerun, live PASS |
| 9 | Codex records an API-key login | merged in #295 (2bf49ac, squashed; a model-named trailer stayed off main); gate PASS, live PASS; subagent proof still needed in Task 13 |
| 10 | Proxy restarts itself on the same port | merged in #298 (04118da); gate PASS, live PASS; cap of 5 is an owner decision; rebind window has no gap |
| 11 | Participant sees "recording works" and the right file | merged in #299 (b102cdc); gate PASS, live PASS (kit parser fixed for the new sealed-hosts wording); chat-mode wording to confirm (L21) |
| 12 | Seams, operations and docs | merged in #300 (a31094c); gate PASS, live PASS |
| 13 | Measured runs (build and test first; long runs later) | partial: see the measured-run table below |
| 14 | Owner steps (for you) | not started |

## Log

- 28 Oct: Phase 0 map done. Nine readers checked the plan against main. 99 claims true, 7 moved,
  44 wrong, 4 conflicts. The wrong ones changed design for Tasks 2, 3, 4, 6, 7, 8, 10, 11, 12 and 13.
- 28 Oct: Build decisions written (one page, for every builder). The spec wins over older decisions
  where it narrows them; each such change goes in the ledger with the reason.
- 28 Oct: crates.io reachable; local Rust 1.99.0 builds; `origin/main` is b6b220b.
- Model names: the first Codex name returned an error from the provider, and a second name works. The live check records the name that worked.
- 28 Oct: Live kit proven on main for claude-code-cli, codex-cli (the second Codex name, since the first returned an error) and hermes-cli (the DeepSeek flash setting). Sandbox gate on main PASS in 306s.
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
- 29 Oct: Task 6 merged in #297 (cc44537). Review found that the copy could run while the harness might still write, and that the queue cap cut copies near 8 MiB. Both fixed before merge. Owner: release order (older binaries refuse final_snapshot), and trace size grows with the shared Hermes database.
- 29 Oct: Task 10 merged in #298 (04118da). The restart budget spans a run, including `continue`. Owner: keep the cap of five (spec rule 16 has no cap), and decide a gap reason for connections refused during the rebind window (v5 is frozen).
- 29 Oct: Task 11 merged in #299 (b102cdc). The finish line names configured hosts, not observed hosts (owner to decide). The kit parser for sealed hosts was updated to the new wording; the product output was correct.

## Measured runs (29 Oct, main a31094c, test keys)

| Run | Decoded model calls | Evidence | Subagents | Result |
|---|---|---|---|---|
| Claude Code, Haiku, thinking off | 9 | wire_and_native | 1 on disk, 1 in chain; 2 conversations (main and subagent); 3 auxiliary calls | Fails byte-for-byte only: the redactor masks 3 emails in the subagent transcript (15 bytes) |
| Codex 0.153.0, Luma low | 11 | wire_and_native | spawn_agent ran; child rollout in chain; 3 conversations (one auxiliary, purpose unknown) | Fails: the on-disk child copy is not checkable, 5 codex_rate_limits spans are redacted in the chain, and the kit summary reads 0 subagents |
| Hermes 0.19.0, DeepSeek flash | 5 | wire_only | none: delegate_task fails in 0.19.0 with an internal executor error, twice | Fails the subagent requirement (Hermes-side defect) |
| AI gateway run (free model) | not run | | | Not run: the kit has no gateway mode yet |
| Negative runs | covered by focused tests | | | Base path refusal, blocked-default-host start and mid-run accept restart are tested in the gate; no live run |

Owner decisions: (1) redaction of emails and codex rate-limit blocks in the chain versus spec rule 3 and the byte-for-byte check; (2) a Hermes version with working delegation, or accept no measured subagent run for Hermes; (3) whether the kit's subagent counter is fixed before the next run; (4) the value-masking gap in headers (spec rule 3) noted in PR #300.
