# The /goal Cup — Rules and Event Plan

> **On `m2/wire` (current, updated 2026-10-11).** A process-scoped loopback proxy records the
> harness's model traffic (the wire leg) next to its own session store (the native leg). It runs
> inside `goalcup start --goal-file`, a local capture run with no grant, arming or server; the
> run ends with a local seal. It never changes a model base URL or app-wide trust or config.
> It forwards credentials in memory and retains neither their values nor fingerprints; every
> body and native segment is sanitized before encoding, hashing, disk or upload, and source
> traces stay private. The tracker supplies no PTY, input relay or continuation
> ([D37](DECISIONS.md#d37-m2-autonomous-runs-research-evidence-and-open-contributions)): human
> assistance ends the unassisted interval, and later capture is assisted evidence. The count-first
> passages below are V1 wording that D37 supersedes; current M2 scope is
> [publicly documented](ARCHITECTURE.md#1-scope). Chain space
> is reserved before launch; a later capture failure is visible and does not block the agent.
> The proxy and recorder are built and measured on short rows; they are not qualified for long
> runs (see [TODO](../TODO.md)) and not in any public release. Passages marked **planned** describe
> what no shipped command does yet.

## V1 foundation revision (2026-09-13)

Current status is in [TODO](../TODO.md).

The event rules below remain the intended competition contract. Features needed to operate them publicly—including production services and keys, public Git publication, presentation collection, broader support, and the later 168-hour exercise—are V1.1 work. V1 tests the same start/interruption/recovery/finish semantics with signed short deadlines.

Status: main working rulebook. Target start: late September or early October 2026; exact T0, timezone and judging
dates are not yet set. This is the main source of truth for the competition; public pages and future tracker
requirements should match it.

## The idea

**One Prompt. One Agent. 168 Hours.**

One person gives one coding agent one prompt, then lets it work autonomously for one week. The agent may plan, create skills, use tools and services, spawn subagents, build infrastructure, and surprise us.

The goal is to show the world what agents can achieve today without human product work—and to collect useful evidence about the current agent ecosystem.

## Timeline

- **Around T−14 days:** publish the Luma event, public challenge, open-source tracker, contribution guide, first judges, partners, and sponsor opportunities.
- **T0:** a global 24-hour starting window opens.
- **During that 24-hour window:** participants finalize their prompt, prepare access, run preflight, and start one or more entries.
- **Each entry start:** the server records `started_at` and starts that entry's own 168-hour clock.
- **At 168 hours:** the entry seals automatically. The clock never pauses and there is no build grace period.
- **Next 24 hours:** presentation only. Submit the YouTube demo link and required Markdown questionnaire beside the immutable repository and commit; do not change the product.
- **After publication closes:** compliance review, preliminary review if needed, judging, public results, and research learnings.

The latest possible run ends 192 hours after T0. The latest publication window ends 216 hours after T0.

## Accounts and entries

- A verified email on an Agent Community account is required.
- Agent Community `pending` status only means the optional endorsement letter has not been signed. Pending and endorsed members may both participate.
- Competition terms and the short rule summary are accepted on the website before a device authorization code is shown.
- One person may run multiple entries, including several in parallel.
- There is no separate manual entry registration. Every successful tracked start creates an entry.
- Every entry needs its own prompt, root agent, empty workspace, tracker session, and 168-hour clock.
- **One harness per entry.** Your agent may spawn subagents of its own harness. If it launches a different harness (a Codex agent starting Claude Code, under any name), the tracker does not stop it; it records that traffic too, the verifier reports it, and the entry is disqualified, because the cup compares harnesses and models, not assemblies of them.
- Work created during one measured entry may not be used to seed another entry.

## One prompt, then a count

- Each entry begins with one text prompt, which is recorded exactly and becomes public with the entry. With `goalcup start --goal-file` (the `m2/wire` local capture run) the file is passed to your agent as its first message ([D7](DECISIONS.md#d7-start-flow-fresh-session-first-typed-message-is-the-goal)).
- The prompt should be no more than **4,000 characters**. A longer one is not refused: the tracker records the length and the verifier flags it. Attachments are allowed.
- Any instruction file your harness reads from the workspace at the start (`AGENTS.md`, `CLAUDE.md` and the like) is part of the prompt: it is recorded with it and published with it. Prepared context is allowed; hidden context is not, and there is nowhere to hide it.
- After the start, every message a human sends to the agent, by any channel, is recorded, published with the entry, and counted as an intervention. `keep going` after an outage or a crash counts too and is labelled as a resume. Nothing is banned; the count is the score.
- An entry with zero interventions carries a label that says so. That label is a detection, not a proof: the tracker records what the model saw and works out who said it from what was typed and how the harness structures its messages. A message it cannot explain is published as unexplained and reviewed by an organizer.
- The tracker never sends anything to the agent, and never relays, edits or blocks your input: no reminders, no automatic continuation.

## Prepared agent setups are allowed

The 4,000-character limit applies to the direct prompt. It is not a limit on the capability of the agent setup.

- Models, skills, rules files, memory, MCP servers, custom tools, services, accounts, compute, and external websites may be prepared before the run.
- The official product workspace must still be completely empty at the start: no README, source, configuration, Git metadata, hidden starter files, or existing product. The only exception is the tracker's own folder-scoped configuration files, which the tracker writes, watches, excludes from the sealed result, and discloses with the entry. Editing them is a flag.
- A remote repository is optional at start. If it already exists, it must also be empty. The agent may create the repository during the run.
- Prepared capabilities may help the agent work, but a prebuilt product, substantial product code, or existing application may not be smuggled in as “context.” The judged result must be a fresh build.
- Participant-controlled instructions and task-specific context available to the agent at the start are frozen for that entry. They cannot be changed by a human during the run.
- The original prompt and the participant-controlled context the agent actually uses must be disclosed with the public entry. Public skills or pages can be linked by exact version; private or changeable material needs a frozen publishable copy.
- An empty workspace is not an empty context, and some harnesses fill it in for you. The Codex desktop app injects your saved memory summary and your global skill listing into every thread; OpenCode and Cursor read the skills in your home directory into the system prompt. That is allowed, and it is disclosed with your entry like any other prepared context. If you would rather it were not there, turn it off before you start; you cannot change it during the run.
- If the tracker cannot resolve a referenced skill, website, MCP resource, memory, or similar source automatically, organizers may request access and the participant must provide the exact frozen material.
- Credentials, tokens, and unrelated private account data are never published.
- **You will see that it is working.** **Planned:** a small status page in your browser while the entry runs: armed or live, time left, last sync, and any step you still have to do. The tracker sends no notifications; `goalcup status` is how you check. There is no token limit. **Planned:** token usage read from your harness's own records and shown on your entry.
- Ordinary public internet pages and normal third-party service responses do not need to be republished in full. Their source should be traceable when the harness exposes it.
- Skills, plans, task lists, external state, and infrastructure created by the agent after the start are part of the agent's work and are allowed.

The direct prompt is the only live human instruction. Prepared resources remain subject to the disclosure and no-side-channel rules.

## During the run

- One measured root agent owns the entry. It may autonomously create trace-linked subagents inside the same harness. It may not run another harness as a subagent; one entry uses one harness.
- Do not update the harness during the run. The tracker records the version at start and refuses to start on untested versions. Some apps update themselves whatever you do; the tracker samples the version, records the change as an event, and an organizer looks at it. That is a flag, not a disqualification.
- **Do not use flags or modes that switch capture off.** These were tested and each one silently produces a session with no evidence: Hermes `--safe-mode`, `HERMES_SAFE_MODE=1` and `--ignore-user-config`; OpenCode `--pure`, `OPENCODE_PURE=1`, `OPENCODE_DISABLE_PROJECT_CONFIG=1`, `--attach`, `share`, `import`, and the `instance/dispose` and `global/dispose` endpoints; OpenClaw incognito threads, `agent exec`, `--local`, `--profile`, `--dev` and `proxy`; Codex `--ephemeral`, `--remote ws://`, `queue`, `fork` and `remote-control`; Cursor `-p` and `--print` for a tracked session, `--api-key` and `CURSOR_API_KEY`, `-e`/`--endpoint`, the `&` cloud handoff, `--worktree` and `AGENT_CLI_CREDENTIAL_STORE=file`; Claude `--no-session-persistence` and `CLAUDE_CODE_SKIP_PROMPT_HISTORY`. All cloud and remote execution modes stay banned for the same reason: they leave no local trace.
- Side channels that reach the agent count as human input wherever they come from, and the ones we know about are named: the Codex app's thread tools (`send_message_to_thread`, `create_thread`, `fork_thread`, `handoff_thread`) and its queued follow-up mode; OpenCode's `POST /session/:id/shell`, which runs a real command and is invisible to the hook leg; Hermes gateway, serve and cron delivery; Claude Remote Control and Channels. Messages sent through them are interventions like any other: recorded where the tracker can see them, counted, and published; one it cannot see arrives at the model anyway and is published as unexplained.
- The human may not write code, answer questions, make product decisions, approve actions, or edit the product.
- No side-channel prompting. The participant or a collaborator may not change anything the agent can read: files, Linear tasks, GitHub issues, messages, documents, email, databases, websites, forms, dashboards, webhooks, environment values, or similar state.
- The agent may create and maintain its own Linear project, notes, issues, skills, or other external state. Humans must leave that state untouched until the run seals.
- Accounts, keys, permissions, payment methods, and spending limits should be prepared before the start. Missing preparation is not itself a violation, but the human may not fix it after the start by typing a key, completing login, clicking an approval, or changing the environment.
- The agent may autonomously authenticate, purchase, create, deploy, or operate resources using authority already available to it.
- Power or internet may be restored and the same tracked session may be relaunched. The clock does not pause and no task information may be supplied.

## Tracker, auth, and privacy

- The official open-source tracker and Agent Community Auth are required for the full run.
- Authentication does not start recording. Tracking begins only when the participant explicitly starts an entry, and the active state must be visible.
- `goalcup start` is a terminal command. It creates the empty workspace and a fresh `HOME` for the harness. Desktop apps cannot be launched this way and are not supported. With `--goal-file` on `m2/wire` the command starts the recording proxy in-process and launches the harness headless with your prompt as the first message; that run has no server, so no clock. **Planned:** starting your clock at the moment that first message leaves your machine for the model provider.
- **The tracker will not start your clock until it can prove it is recording.** A `m2/wire` run has no clock and no arming gate. **Planned:** the proof is that the harness was launched in the tracked folder, its first transcript record has been chained, and its first request to the model provider has passed through the recording proxy. **Planned:** an entry that cannot show that proof within an arming window is refused with a reason before any clock starts. `goalcup status` shows a live run's capture state, health, storage, gaps and whether assistance was recorded. **Planned:** the last model request seen.
- **The Codex desktop app and Claude Desktop have no supported recording path.** A desktop app cannot be launched with a private environment, so the only way to record one would be a system-wide proxy that captures every application on the machine. We do not do that. Use the CLI.
- The tracker retains only sessions opened in the folder it created. Each harness writes its sessions into an isolated config home, never your own store, and the daemon reads only the session file named with the launched session's id.
- **Claude Desktop has no connector on `m2/wire`. If a Desktop connector ships (V1.1) and your entry uses Claude Desktop with raw body capture, run no other Claude Code session on that machine for the duration.** The switch that produces the strongest evidence is app-wide, so it captures every Claude process on the machine. The tracker discards any body that does not belong to the tracked session, but in a six-minute test that still meant sweeping up 23 request bodies that were none of our business, one of them from an unrelated folder. Keep the machine to one Claude session and the problem does not arise.
- **Four app-wide settings changes are permitted, and each one is disclosed on your public entry**: the raw-body switch in Claude Desktop's user settings, the twelve hook-trust entries Codex's own trust screen writes into your Codex config, one plugin allow-list key in the Hermes home config, and the state file, tracking database, skill re-sync and keychain token that the Cursor CLI writes into your real `~/.cursor` no matter how it is pinned. The first three are recorded before and after and reversed at seal. The fourth is the harness doing it to itself; the tracker records it and cannot undo it. Nothing else app-wide is touched. None of the four is performed on `m2/wire`: they belong to the desktop, gateway and Cursor entries in the support table below, none of which has a connector here, and no manifest under `connectors/` declares an app-wide exception. The Codex trust entries also belong to a hook leg that `m2/wire` no longer has.
- **What a run takes from your own home.** The tracker reads nothing from and writes nothing to your `~/.claude`, `~/.codex` or `~/.hermes`, and the daemon refuses a config home inside any of them. Codex and Claude Code each use one isolated config home in the tracker's own config directory, shared by that connector's runs, created empty and never seeded. Claude Code's run `HOME` holds only a link to your `~/Library/Keychains`, for your subscription login ([D38](DECISIONS.md#d38-m2-fresh-per-run-home-for-claude-code-cheapest-models-for-live-tests)). Hermes gets a fresh per-run home seeded only with the connector's own files, and its provider key comes from your environment.
- **What the evidence actually covers.** Every token your agent saw is captured. Every byte a tool produced is not: several harnesses truncate large tool output before writing it, and on some of them the untruncated bytes exist nowhere on disk. A few internal model calls, such as title generators and follow-up suggestion helpers, appear in no transcript on any harness we measured. Neither affects judging, and both are stated here so nobody discovers them later.
- Tracker downtime while the agent kept working is reviewed, not automatically penalized. Downtime while nothing happened is an outage.
- The server is authoritative for the 24-hour start window, `started_at`, `deadline_at`, and publication deadline.
- The tracker should capture the exact prompt, starting workspace state, observable prepared context, later human messages, agent/subagent lifecycle, tool calls and results, external sites/services used, harness and model identity when available, run continuity, and final state.
- **Planned:** trace data uploads to Agent Community during the run. An encrypted local buffer may bridge ordinary network outages.
- **What the tracker records, and what it never does.** On `m2/wire`, `goalcup start --goal-file` starts the recording proxy in-process. The tracker observes one process: the harness it launched for your entry, and any program that harness starts. It records that agent's conversation with the model provider, every request and every reply, by pointing that one process at a recording proxy on your own machine through its own environment; nothing else on the machine goes through it. It records what you type into that agent, because those messages are the interventions it publishes. Traffic from the agent to any other address, and traffic on the provider's address that is not a model call, is noted as the address and the number of bytes each way, nothing more; it is never decrypted. Sign-in and token refresh are never decrypted either. Your ChatGPT, Claude or other credentials are never stored, anywhere: the proxy removes them before anything is written, and keeps neither their values nor any fingerprint of them. It does not watch other terminals, your browser, other processes, or audio. Raw traces stay private, are redacted a second time on the server, and are never published.
- Raw traces and terminal output remain private. They are used only for compliance review and the announced research project. They also contain more about you than you might expect: Codex stamps your plan type, credit balance and quota percentage into every token-count event, and harnesses read credentials from outside their own home directories. That is another reason raw traces are never published.
- The public entry includes the original prompt, disclosed starting context, harness/model information when known, verification status (the verifier's `pass`, `flag` or `fail` verdict and its findings), and final result. A verdict is not proof of anything; `docs/CHEAT-CHECKLIST.md` lists what a `pass` does not cover.
- Modifying, disabling, bypassing, or deliberately routing work around the tracker is grounds for disqualification.

## Harness support

Cloud and remote execution modes (Claude Code on the web, Codex cloud, Cursor cloud agents, CI runners, remote servers) are not eligible because they leave no local trace.

We ran a capture test on every harness on 2026-09-03, one per harness and mode, and the results are public in `docs/CAPTURE-RESEARCH.md` (git history at `6494030`). This is the support list that came out of it. **Full** means the capture is complete enough to verify automatically. **Elevated review** means an organizer reads the trace before your entry is accepted; it is not a penalty and it does not affect judging.

**This table is the event's target set, not the V1 release matrix.** **On `m2/wire` only `codex-cli`, `claude-code-cli` and `hermes-cli` (capture-only) ship a connector; no other row has one** (the OpenCode connector exists only on `v1/trunk`). V1 defines only the exact Codex CLI/macOS row for final qualification. Claude Code CLI and OpenCode remain preserved candidates; they and the two Desktop surfaces are promoted independently in V1.1 after the same lifecycle suite passes. Anything below about an unqualified harness describes its intended event behavior, not a current V1 support claim.

| Harness | Mode | Support | What you need to know |
|---|---|---|---|
| Claude Code CLI | terminal | full | pin version 2.1.259 or later |
| Codex CLI | terminal | full | the tracker launches the real binary, not the npm wrapper |
| OpenCode | terminal | full | loading the tracker plugin installs a dependency tree into the workspace; it is excluded from your sealed result |
| Claude Desktop | desktop app | elevated review | no other Claude session on the machine if raw body capture is used |
| Codex desktop app | desktop app | elevated review | the one-time `/hooks` trust step above is mandatory, before the goal thread |
| Hermes | terminal | elevated review | one app-wide plugin key; no length gate, so an over-length goal is recorded as a violation rather than refused; on `m2/wire` it is capture-only ([D41](DECISIONS.md#d41-m2-hermes-reference-row-model-ca-trust-key-fingerprint-and-key-fragment-rules)) and writes no app-wide key |
| OpenClaw | gateway | elevated review | the runtime must be pinned per model; the tracker checks it at arming |
| Cursor CLI | terminal | elevated review | interactive CLI only; `-p` and `--print` are banned, because in print mode nothing witnesses what you typed |
| DeepSeek Harness (dsh) | any | **not supported for v1** | never successfully measured; it can join later through the same tested pull request as any other harness |

Each harness was checked using its current official interfaces for:

- prompt and later-message capture;
- tool calls and results;
- subagent lineage;
- model and harness identity;
- skills, rules, memory, MCP, and other starting-context discovery;
- session stop/resume behavior;
- CLI, desktop, cloud, and remote modes;
- connector failure and tamper visibility.

Only modes that meet the minimum capture contract and pass the public conformance suite will appear in the final supported release list. Other harnesses may join through an approved, tested pull request merged before T0.

## Automatic submission and presentation

- At the signed deadline the coordinator records the cutoff, stops the managed harness or dedicated attach app where safe, and snapshots eligible files after termination or a bounded timeout.
- A separate least-privilege publisher constructs the sealed Git tree and a Goal Cup seal commit when needed, then attempts a bounded, noninteractive normal push to the registered public GitHub remote. No force push or history rewrite is permitted.
- The MIT License must exist in the sealed result. The result must be a fresh, usable or executable product.
- A failed push leaves the local seal intact. Later repository publication is prominently flagged as late; the presentation window does not turn it into on-time publication.
- During the following 24 hours the human submits a required YouTube explanation/demo link, the required versioned Markdown questionnaire, the sealed repository and commit, run instructions, architecture, limitations and judging notes, plus a live URL for an online service.
- The service stores the packet separately. No presentation file or commit is added to the sealed repository.
- A local product may run for demonstration, and ordinary runtime data/caches are allowed. Source, history and submitted artifacts must remain unchanged through judging. Online services may not be rebuilt or redeployed with changed code.
- An incomplete or unverifiable result is not automatically misconduct. Organizer-caused capture gaps receive review and an explicit statement of evidentiary limits.

## Compliance and judging

- The tracker and private trace review establish whether the run followed the rules. They do not score the quality of the work.
- Organizers may use automated checks and an internal review agent to find anomalies, but a human makes disqualification decisions.
- Judges receive the original prompt, disclosed starting context, and exact sealed final product. They do not receive private raw traces.
- The main Cup goes to the strongest final product. Final judging criteria will be published before T0.
- If entry volume is large, a preliminary panel may create a shortlist using the same published product criteria.
- The first announced judges are Jesse Vincent, creator of Superpowers, and Scott Motte, creator of dotenv.

## Public race and sharing

- The public board may show aggregate entry states such as joined, started, running, sealed, published, and verified.
- Participants remain anonymous by default and may opt into public recognition.
- Agent Community may share aggregate race updates throughout the week.
- A public X post made before T0 is worth one bonus point. The participant submits its URL through the authenticated page or CLI; engagement does not affect the point.
- The accepted starting prompt is public at entry start. Raw traces never become public.

## Support and partners

- **Offer free access:** give participants product access, credits, or API quota. Financial sponsorship is not required for this entry-level support.
- **Create an award:** pair event sponsorship with a named prize inside the main Cup. The sponsor supplies a clear criterion, prize, and product expert to help judge it. It does not create a separate competition.
- **Back the event:** contribute general funding, compute, infrastructure, tooling, marketing, distribution, or media support without requiring a product-specific award.
- **Community partner:** no product or funding is required. The partner makes a real promotional commitment—such as an event listing, newsletter, social post, or member outreach—and is publicly listed as an official partner.
- Sponsors and partners must be Agent Community members.
- Awards, their criteria, and judges freeze at T0.
- Judge, partner, and sponsor enquiries: `goalhack@agentcommunity.org`.

## Still to decide

- Exact T0, timezone, and judging/result dates.
- Final tested version ranges per connector, re-verified shortly before T0.
- Trace retention, deletion, organizer access, and participant withdrawal terms.
- Event storage/capacity limits and signed binary release policy.
- Final judging rubric, shortlist threshold, and appeal process.
