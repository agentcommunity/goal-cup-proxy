# Docs map

Which page do I read? Find your role below and start at the top of its list. Each line says what
one page is for. The words we use (run, part, segment, chain file, seal, receipt, ack) are defined
once, in [ARCHITECTURE section 12](ARCHITECTURE.md#12-terms).

## Participant

You run one agent for 168 hours.

- [PARTICIPANT-GUIDE](PARTICIPANT-GUIDE.md): one page: install, setup, the repository, token and project folder, start, status, continue, the end. The long 0.6 guide is in `archive/`.
- [HARNESS-SETUP](HARNESS-SETUP.md): the harness settings for a 168-hour run (permission prompts, commits, step limits, sign-in, subagents, `PATH`, first-run screens), per harness.
- [RULES](RULES.md): the contract. What a run may and may not do, and what Goal Cup Proxy touches.
- [goal-cup-setup skill](../skills/goal-cup-setup/SKILL.md): hand the setup to your own agent.
- [SECURITY](../SECURITY.md): what the tool does on your machine, and how to report a problem.

## Organizer

You run the event, the server and the test runs.

- [OPERATING](OPERATING.md): the runbook. Deploy, test runs, the matrix, edge cases by hand, bad days, finding codes.
- [SERVER-DESIGN](SERVER-DESIGN.md): how the upload server is built, and why.
- [UPLOAD-API](UPLOAD-API.md): the exact wire contract between `goalcup` and the server.
- [server/worker README](../server/worker/README.md): the Worker's code, tests, local use and runbook.
- [packaging README](../packaging/README.md): release tarballs and `install.sh`.
- [scripts README](../scripts/README.md): one line per script, and which ones CI and the gate run.
- [Notifications](research/NOTIFICATIONS.md): every message a participant can get, and who sends it.
- [How a run ends](research/HOW-A-RUN-ENDS.md): the plain-English plan for the end of a run.
- [Restarts across session limits](research/RESTARTS-DESIGN.md): a run on a sandbox that stops every 24 hours, and `goalcup continue` after each stop.
- [Event readiness plan](plans/2026-10-20-event-readiness.md): invariants and work packages from 20 Oct to 6 Nov.
- [Local next steps](research/LOCAL-NEXT-STEPS.md): what to do on real machines next, in order.
- [Local test day](research/LOCAL-TEST-DAY.md): the 8-hour plan and handover for one day of local testing on every surface.
- [Local campaign](research/LOCAL-CAMPAIGN.md): how long test cells and drills run on a maintainer's test machines.
- [campaign-cell agent](../.claude/agents/campaign-cell.md): a public Claude Code subagent that watches one running campaign cell and reports back. It only reads.

## Harness builder

You add a coding-agent harness as a connector.

- [adding-a-capture-harness skill](../skills/adding-a-capture-harness/SKILL.md): where to start.
- [HARNESS-ONBOARDING](HARNESS-ONBOARDING.md): the one procedure, with commands, expected output and stop rules.
- [CONNECTOR-SCHEMA](CONNECTOR-SCHEMA.md): what a chat connector writes, then every `harness.toml` field.
- [Harness facts sheets](harnesses/README.md): one page per harness, and the rules for writing one.
- [writing-a-decoder](../skills/adding-a-capture-harness/writing-a-decoder.md): a separate verifier change, not part of a connector.
- [CONTRIBUTING](../CONTRIBUTING.md): how to send a change, and what a maintainer asks for.

## Developer

You change the Rust code, the Worker or the scripts.

- [AGENTS](../AGENTS.md): the repository map, hard rules, tests and "done".
- [README](../README.md): build, install, run one capture, the gate.
- [ARCHITECTURE](ARCHITECTURE.md): how the parts fit, the glossary, and where each guarantee is tested.
- [DECISIONS](DECISIONS.md): the rules code cites as `D<number>`, with a status table at the top.
- [CONTRACTS](CONTRACTS.md): byte and format rules for v5 capture, the local finish and the control API.
- [TERMINATION](TERMINATION.md): what `Confirmed` termination does and does not claim.
- [CHEAT-CHECKLIST](CHEAT-CHECKLIST.md): read before you trust a verifier Pass.
- [TODO](../TODO.md): open work and known issues.
- [Simplification plan](research/SIMPLIFICATION-2026-10-21.md): what to simplify before and after the event.
- [Participant review](research/PARTICIPANT-REVIEW-2026-10-21.md): what three reviewers found walking the participant path.

## Researcher

You read traces and judge what they show.

- [RESEARCH-TARGET](RESEARCH-TARGET.md): what the recorder should achieve, and what counts as coverage.
- [Coverage matrix](research/coverage-matrix.md): measured harness and platform cells, and their limits.
- [Report format](research/m2-report-format.md): the private research report (schema 5) written by `goalcup-verify --research-json`.
- [Replay comparison](research/replay-comparison.md): compare two traces you already have with one verifier.
- [T25 disk full](research/t25-disk-full.md): what a run did on a really full disk (measured 21 Oct).
- [LOCAL-RESEARCH-HANDOFF](LOCAL-RESEARCH-HANDOFF.md): the 18 Oct handoff for the first Codex Luna/high run (history).
- Workloads: [campaign-workload.txt](research/campaign-workload.txt) (the campaign prompt) and [codex-luna-30m-workload.txt](research/codex-luna-30m-workload.txt) (the 30-minute research prompt).

## History

Kept for references. Do not plan from these.

- [TODO archive, October 2026](archive/TODO-2026-10.md): finished work packages WP0 to WP12, the 18 to 19 Oct runs, Review 13.
- [Shared manual-seal plan](plans/2026-10-18-shared-manual-seal.md): the 18 Oct plan for seal-time termination.
- [Research readiness plan](plans/2026-10-19-research-readiness.md): the 19 Oct plan for research phases 1 and 2.
