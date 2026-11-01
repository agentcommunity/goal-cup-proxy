# First message for the new conductor session

Paste everything below the line into the new session.

---

You are the conductor of the goalcup 1.0 finish in agentcommunity/goal-cup-proxy. I opt in to large multi-agent orchestration: use workflows and subagents, run independent lanes in parallel, and pick the model per task (most capable for hard core code, review and the breaker; sonnet for well-specified implementation, rig data, docs and pad rows; haiku for mechanical work).

Steps:
1. `git fetch origin v1.0/integration v1.0/conductor-handover` and work from v1.0/integration with the handover merged (if v1.0/conductor-handover is not yet on integration, merge it first with a merge commit).
2. Read, in order: CLAUDE.md, AGENTS.md, VISION.md, docs/plans/2026-10-30-v1.0/OWNER-DECISIONS.md, docs/plans/2026-10-30-v1.0/conductor/owner-decisions-draft.md (owner answers not yet copied into OWNER-DECISIONS.md; they bind you), conductor/STATE.md, conductor/TOOLS.md, conductor/SECRETS.md, docs/superpowers/plans/README.md and the four plans it lists.
3. Check the environment variables in SECRETS.md, printing only "set" or "missing" for each, never a value. Write the result as the first lines of a new section in NIGHT-LOG.md.
4. Install the conductor tools as TOOLS.md says (npm install, the patched sandbox client, the proxy preload, the /root/gc-night and /root/gc-tools folders), and change the commit trailer session URL in the tools to this session.
5. Execute the plans subagent-driven: a fresh subagent implements each task and a fresh reviewer checks it before the next one starts, then a whole-branch review at the end. Follow each plan's Execution section for lanes and models. Order: the handover plan's portability tasks first, then the code wave (single-pass matcher, rig fixes) as one merge train with one gate, one shared build, the composite 10-minute harness wave (all harnesses at once), one fix round, the one-hour composite runs, then release (owner steps), then big tests.

Rules that win over everything: OWNER-DECISIONS.md, the owner-decisions draft, VISION.md and AGENTS.md. Never print, log or put a key in a command line. No compile or test on the conductor machine: builds and runs happen only in Vercel Sandboxes. Use boxes deliberately: no box per fix round, merge trains with one gate, one build per wave, composite rows. Claude Code tests use Haiku only. Staging only; push only v1.0/* branches; merge commits only; never force-push; never touch production, main, tags or the website database (read-only fetches through the short-lived index box are allowed). No model names or em dashes in docs or commits. Any chat message from me while a workflow runs is for you, not for the workflow agents: say so in every agent brief.

Call me only for decisions that are mine (the owner asks in STATE.md) and for the W1 to W7 cases in BUILD-PROMPT.md. When done, write MORNING.md and ACCEPTANCE.md on v1.0/integration.
