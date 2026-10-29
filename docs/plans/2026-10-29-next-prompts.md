# Next prompts: the 2-hour run and the cut-first audit

29 Oct 2026. Two prompts for Claude Code cloud agents. Both read [VISION.md](../../VISION.md) first.
Run them in parallel: the run measures what is deployed now; the audit decides what to cut.

## Prompt 1: the measured 2-hour run

```text
You are a Claude Code cloud agent running a measured 2-hour test of Goal Cup Proxy (agentcommunity/goal-cup-proxy) in a Vercel Sandbox. You act as the participant and the observer. You change no product code.

Read first, on main: VISION.md (the seven points), AGENTS.md, docs/OPERATING.md (the "Test runs" and matrix sections), docs/research/VERCEL-COMPAT.md, docs/research/results/2026-10-vercel-gateway.md, docs/plans/2026-10-29-record-everything-progress.md.

Setup:
- Keys in your environment: CLAUDE_API_KEY, VERCEL_TOKEN, VERCEL_TEAM_ID, VERCEL_PROJECT_ID. Print only "set" or "missing". Never print, log or commit a key, or put one in argv. Put a key in the sandbox only as a 0600 file and in the harness's own settings there. CLAUDE_API_KEY becomes ANTHROPIC_API_KEY only inside the sandbox.
- Create one Vercel Sandbox. Check its maximum run time and set it to at least 3 hours. If it cannot reach that, stop and tell the owner.
- Build goalcup from main at the commit production runs (a31094c2, or newer main if the owner says so). Install the latest Claude Code release.
- Production is current and staging is stale, so log in to production. Use an organizer test run: goalcup start --harness claude-code-cli --test. When goalcup shows a device code, ask the owner to approve it and wait.
- Set up the harness the way a participant would: goalcup setup, the Haiku model at the lowest thinking setting, never stopping for approval.
- If the run needs a GitHub repository or token, ask the owner. Never invent credentials.
- Before you start, ask the owner for the spend cap.

The run:
- Start the harness in tmux and type the prompt below like a person would. Use scripts/matrix.sh --test --duration 7200 instead only if it lets you send this exact prompt.
- After the prompt, you may type only "keep going", and only when the harness has stopped and waits for input. Never give hints, fixes or decisions.
- End and seal the run at 2 hours.

The prompt to send, as written:
Build a PayPal-like payments product from scratch in this repository: a user app (sign up, wallet, send and request money, history), an admin console (users, disputes, risk flags, refunds) and enterprise features (teams, roles, invoices, API keys, webhooks, audit log). Keep a task ledger in LEDGER.md and update it after every task. Work in stages. For each stage, use planning subagents to break it into detailed tasks, then builder subagents to write the code and tests in parallel. After each large task is merged, run a code-quality subagent to review and fix it. Commit and push after every task. Keep going until the whole product is done.

Stop early, and say why, if: the spend cap is reached, no model call is recorded for 15 minutes, goalcup prints a PROBLEM line, or the sandbox is about to expire.

Measure every 15 minutes (one line in your log) and at the end:
1. Model calls: recorded calls over time, reconciled with the harness's own record (the Claude session files). A call in the harness record that is missing from the trace is a finding.
2. Subagents: how many the harness started (agent-*.jsonl on disk) vs how many are in the trace. Does each link to its parent session and carry times?
3. Hosts: every host the harness reached. Any host listed as sealed (not recorded) is a finding.
4. Gaps: the count and reason of every gap, and every proxy restart.
5. Upload: segments acknowledged by the server, upload lag, the final seal and receipt, and goalcup-verify on the fetched trace (scripts/fetch-run.sh): pass or fail, with reasons.
6. Size: trace bytes, compressed bytes uploaded, growth per hour.
7. Load: CPU and memory of goalcup, the daemon and the proxy; open file handles; disk use.
8. Secrets: scan the trace for the key you used (compare hashes, never print it) and for Authorization headers. The scan must find none.
9. Harness experience: any refusal, warning, stall, slowdown or question that blocked it; time from prompt to first call.
10. Spend: tokens from the trace, and the estimated cost.

Report: write docs/research/results/2026-10-30-two-hour-run.md on your branch. Plain English, short sentences, no em dashes, no model ids. A table for the ten measures, each finding with the path to its evidence, and a short "fix first" list mapped to the seven points in VISION.md. Push your branch. Do not open a PR or touch main. Then post a 10-line summary.

Hard rules: no product code changes. Never deploy. Never delete runs, branches or data; the owner cleans up test runs.
```

## Prompt 2: the cut-first audit (read-only, a large workflow)

```text
You are a Claude Code cloud agent. Run a large, read-only, multi-agent workflow (use a workflow; 30 to 60 agents is fine) over agentcommunity/goal-cup-proxy. The question: if we built Goal Cup Proxy fresh from the seven points in VISION.md, what would we keep, cut and change, so the version for Friday 6 November works 100% with the least code?

We over-engineered. Cut first, then add only what the seven points need. No backward compatibility is needed: every run so far is test data, and the protocol, verifier, Worker and client may change together.

Read first, on main: VISION.md, AGENTS.md, CLAUDE.md, docs/plans/2026-10-29-record-everything-progress.md (owner decisions after phase 2), docs/plans/2026-10-29-record-everything-phase2-notes.md, docs/DECISIONS.md, docs/RULES.md, docs/ARCHITECTURE.md. If the 2-hour run report (docs/research/results/2026-10-30-two-hour-run.md) exists on any branch, read it too.

Lanes, run in parallel. Every agent cites file paths and line ranges, or links for web facts.
A. Inventory: every crate, module, connector, script, Worker route and doc. What it does, its size in lines, and which of the seven points it serves (or none).
B. Fresh design: two designers who do not read the code. From the seven points alone, each designs the smallest system that meets them. Then compare the two designs with the inventory.
C. Module audits, one agent per area: protocol; daemon; CLI proxy; CLI start, setup, model host and preflight; uploader; verifier; Worker; connectors; scripts and gate. For each part: keep, cut or change, the reason, lines saved, and the risk.
D. Harness research (web and source, latest releases): Claude Code, Codex, Hermes, plus a look at other popular harnesses for awareness. How each reaches models (endpoints, env vars, gateways, sign-in vs API key), how subagents work and where their files live, session files, release cadence, changes in the last 30 days, and risks for a run starting 7 November.
E. Provider and gateway research: Anthropic, OpenAI, DeepSeek, Vercel AI Gateway, OpenRouter, cloud-hosted model services and local models. Hosts, streaming formats, WebSockets, HTTP/2, certificate pinning, and anything that can skip HTTPS_PROXY.
F. Week-long run risks: sleep, network changes, a closed laptop, disk, memory growth, file handles (macOS allows 256 per process by default), clock changes, harness auto-updates mid-run, sign-in expiry, rate limits and provider outages.
G. Secrets: every place a secret can appear (headers, bodies, session files, env dumps, tool output), and the least masking that meets point 4.
H. Participant experience: from install to the end of a run, as a new user on macOS, Linux and WSL. Every message and refusal they see, and which can go.
I. Server and upload: Worker routes, limits and quotas, and what the server needs to rebuild the story by ID and time. Anything over-built.
J. Tests and gate: which tests protect the seven points, which protect parts that would be cut, gate time, and flaky tests.
K. Docs and decisions: entries in DECISIONS.md and the docs that conflict with the seven points, and docs to delete or merge.
L. Red team: what would make a run unrecordable or break a harness on day 3, and which cheats in docs/CHEAT-CHECKLIST.md the record must still show after the cuts.

Then:
- One synthesis agent writes the plan: a cut list (lines, risk, tests that go), a change list, an add list (only what the seven points need), PRs in order, what must deploy together, what to test in the next 2-hour run, and at most 10 owner questions.
- Two adversarial reviewers challenge it: one argues "this cut breaks a point", the other "this is still over-engineered". The synthesis agent answers both and finalises.

Output, on your branch: docs/plans/2026-10-30-cut-first-plan.md (the plan, plain English, short sentences, no em dashes, no model ids), and one appendix per lane under docs/plans/2026-10-30-audit/. Push your branch. Do not open a PR or touch main. Then post a 15-line summary with the top 10 cuts and the owner questions.

Hard rules: read-only on product code; change nothing outside the plan files. Web search is allowed and encouraged. No keys are needed: never print or use any. Read the opencode and openclaw connectors if useful, but never edit them.
```
