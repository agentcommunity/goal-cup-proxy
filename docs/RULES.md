# The /goal Cup — Rules and Event Plan

> **How the tracer runs (updated 2026-10-15).** `goalcup start` opens your harness in its normal
> chat mode, in your own terminal, in a clone of your new, empty, public GitHub repository. You
> approve each run in the browser with your Agent Community account. You type the prompt as the
> first message; after it, the only message you may type is "keep going". The tracer records the
> harness's model traffic through a recording proxy on your machine, set for the harness process
> only, and its session files; it uploads the record to the Goal Cup server while the run goes on.
> It never changes a model base URL or app-wide trust or config, never sends the agent a message,
> and never restarts it on its own. Credentials are removed on your machine before anything is
> written or uploaded, and raw records stay private. The step-by-step path is the
> [participant's guide](PARTICIPANT-GUIDE.md); the design behind it is the
> [run-lifecycle design](superpowers/specs/2026-10-14-run-lifecycle-design.md) and the owner
> decisions in its [plan](superpowers/plans/2026-10-14-run-lifecycle.md). Passages marked
> **planned** describe what no shipped command does yet.

Status: main working rulebook. The 24-hour start window opens on 6 November 2026; its exact hours
and timezone, and the judging dates, are not yet set. This is the main source of truth for the
competition; public pages and future tracker requirements should match it. Open work is in
[TODO](../TODO.md).

## The idea

**One Prompt. One Agent. 168 Hours.**

One person gives one coding agent one prompt, then lets it work autonomously for one week. The agent may plan, create skills, use tools and services, spawn subagents, build infrastructure, and surprise us.

The goal is to show the world what agents can achieve today without human product work—and to collect useful evidence about the current agent ecosystem.

## Timeline

- **Around T−14 days:** publish the Luma event, public challenge, open-source tracker, contribution guide, first judges, partners, and sponsor opportunities.
- **T0:** a global 24-hour starting window opens.
- **Before the window:** participants prepare with `goalcup prep` and practice runs, which are not limited to the window.
- **During that 24-hour window:** participants finalize their prompt and start their run.
- **The run's start:** the server opens the run, stamps its official start on its own clock, and sets its deadline 168 hours later.
- **At 168 hours, on the server's clock:** a hard stop. The tracer stops the agent in whatever state it is in, seals and uploads the record, and the server records the commit in the run's GitHub repository. The clock never pauses and there is no build grace period.
- **24 hours after the end:** the server closes the run. A run whose final upload has not arrived by then is closed as *incomplete* and judged on what arrived.
- **After that:** compliance review, preliminary review if needed, judging, public results, and research learnings.

The latest possible run ends 192 hours after T0.

## Accounts and entries

- A verified email on an Agent Community account is required.
- Agent Community `pending` status only means the optional endorsement letter has not been signed. Pending and endorsed members may both participate.
- Competition terms and the short rule summary are accepted on the website before a run can be approved.
- **One main run per account.** Every competition run is approved in the browser with your Agent Community account. A new competition start inside the start window becomes your main run and replaces any earlier one: the server stops the earlier run, and only the main run counts. After the window closes, an account can only `goalcup continue` its main run.
- There is no separate manual entry registration. Every approved competition start creates a run (your entry).
- A run is made of segments. When the agent stops before the deadline (a crash, a reboot, a closed window), `goalcup continue` starts the next segment of the same run, with the same deadline, on the same machine.
- **Practice runs** (`goalcup start --practice`) are short, unlimited, separate from the competition, and never replace your main run. A practice run never pushes, so your repository stays empty until the competition run.
- **Organizer test runs** (`goalcup start --test`) are opened only by organizer accounts. They are not entries, and they are deleted from the server afterwards.
- Your run needs its own prompt, root agent and empty repository.
- **One harness per run.** Your agent may spawn subagents of its own harness. If it launches a different harness (a Codex agent starting Claude Code, under any name), the tracker does not stop it; it records that traffic too, the verifier reports it, and the run is disqualified, because the cup compares harnesses and models, not assemblies of them.
- Work created during one run (a replaced run included) may not be used to seed another run.

## One prompt, then only "keep going"

- Each run begins with one text prompt, which you type as the first message in the harness's own terminal. It is recorded exactly, in the harness's session files and its model traffic, and becomes public with the entry after the run.
- The prompt should be no more than **4,000 characters**. A longer one is not refused; the review sees its length. Attachments are allowed.
- Any instruction file your harness reads from the workspace at the start (`AGENTS.md`, `CLAUDE.md` and the like) is part of the prompt: it is recorded with it and published with it. Prepared context is allowed; hidden context is not, and there is nowhere to hide it.
- **After the first prompt, the only human message permitted is "keep going".** Every message a human sends to the agent, by any channel, is recorded so the review can see it. Any other human message removes the run from the qualifying runs at review.
- The review works out who said each message from what the model saw and how the harness structures its messages. That is a detection, not a proof. A message it cannot explain is reviewed by an organizer.
- The tracker never sends anything to the agent, and never relays, edits or blocks your input: no reminders, no automatic "keep going", no automatic restart. `goalcup continue` is yours to run.
- An agent that stops early and stays idle until the deadline is permitted. Its idle time is in the record.

## Prepared agent setups are allowed

The 4,000-character limit applies to the direct prompt. It is not a limit on the capability of the agent setup.

- Models, skills, rules files, memory, MCP servers, custom tools, services, accounts, compute, and external websites may be prepared before the run.
- **A GitHub repository is required.** Before the run you create a new, empty, public repository and a token that can push only to it; `goalcup prep` checks both. The run's workspace is a clone of that repository, so it starts with no README, source, configuration, hidden starter files, or existing product. The tracker adds nothing to it: its own `goal-cup` skill goes in the harness's home, not the workspace.
- The agent pushes its work to that repository; the tracker gives the token to `git` only. The commit the server records at the deadline is what is judged.
- Prepared capabilities may help the agent work, but a prebuilt product, substantial product code, or existing application may not be smuggled in as “context.” The judged result must be a fresh build.
- Participant-controlled instructions and task-specific context available to the agent at the start are frozen for that entry. They cannot be changed by a human during the run.
- The original prompt and the participant-controlled context the agent actually uses must be disclosed with the public entry. Public skills or pages can be linked by exact version; private or changeable material needs a frozen publishable copy.
- An empty workspace is not an empty context, and some harnesses fill it in for you. The Codex desktop app injects your saved memory summary and your global skill listing into every thread; OpenCode and Cursor read the skills in your home directory into the system prompt. That is allowed, and it is disclosed with your entry like any other prepared context. If you would rather it were not there, turn it off before you start; you cannot change it during the run.
- If the tracker cannot resolve a referenced skill, website, MCP resource, memory, or similar source automatically, organizers may request access and the participant must provide the exact frozen material.
- Credentials, tokens, and unrelated private account data are never published.
- **You will see that it is working.** `goalcup status` shows the recording's state and health, and a `PROBLEM:` line for anything wrong. The server emails you when your run starts, when it has heard nothing from the tracker for 20 minutes, when your agent has made no model calls for 3 hours and again at 6 hours, when the recording has a problem, and when the run has ended and its commit is recorded. There is no token limit. **Planned:** token usage read from your harness's own records and shown on your entry.
- Ordinary public internet pages and normal third-party service responses do not need to be republished in full. Their source should be traceable when the harness exposes it.
- Skills, plans, task lists, external state, and infrastructure created by the agent after the start are part of the agent's work and are allowed.

The direct prompt is the only live human instruction. Prepared resources remain subject to the disclosure and no-side-channel rules.

## During the run

- One measured root agent owns the entry. It may autonomously create trace-linked subagents inside the same harness. It may not run another harness as a subagent; one entry uses one harness.
- Do not update the harness during the run. The tracker records the harness build that runs; it refuses no build. Some apps update themselves whatever you do; the tracker samples the version, records the change as an event, and an organizer looks at it. That is a flag, not a disqualification.
- **Do not use flags or modes that switch capture off.** These were tested and each one silently produces a session with no evidence: Hermes `--safe-mode`, `HERMES_SAFE_MODE=1` and `--ignore-user-config`; OpenCode `--pure`, `OPENCODE_PURE=1`, `OPENCODE_DISABLE_PROJECT_CONFIG=1`, `--attach`, `share`, `import`, and the `instance/dispose` and `global/dispose` endpoints; OpenClaw incognito threads, `agent exec`, `--local`, `--profile`, `--dev` and `proxy`; Codex `--ephemeral`, `--remote ws://`, `queue`, `fork` and `remote-control`; Cursor `-p` and `--print` for a tracked session, `--api-key` and `CURSOR_API_KEY`, `-e`/`--endpoint`, the `&` cloud handoff, `--worktree` and `AGENT_CLI_CREDENTIAL_STORE=file`; Claude `--no-session-persistence` and `CLAUDE_CODE_SKIP_PROMPT_HISTORY`. All cloud and remote execution modes stay banned for the same reason: they leave no local trace.
- Side channels that reach the agent count as human input wherever they come from, and the ones we know about are named: the Codex app's thread tools (`send_message_to_thread`, `create_thread`, `fork_thread`, `handoff_thread`) and its queued follow-up mode; OpenCode's `POST /session/:id/shell`, which runs a real command and is invisible to hook-based recording; Hermes gateway, serve and cron delivery; Claude Remote Control and Channels. Messages sent through them are human messages like any other: recorded where the tracker can see them and reviewed; one it cannot see arrives at the model anyway and is reviewed as unexplained.
- The human may not write code, answer questions, make product decisions, approve actions, or edit the product.
- No side-channel prompting. The participant or a collaborator may not change anything the agent can read: files, Linear tasks, GitHub issues, messages, documents, email, databases, websites, forms, dashboards, webhooks, environment values, or similar state.
- The agent may create and maintain its own Linear project, notes, issues, skills, or other external state. Humans must leave that state untouched until the run seals.
- Accounts, keys, permissions, payment methods, and spending limits should be prepared before the start. Missing preparation is not itself a violation, but the human may not fix it after the start by typing a key, completing login, clicking an approval, or changing the environment.
- The agent may autonomously authenticate, purchase, create, deploy, or operate resources using authority already available to it.
- Power or internet may be restored and the agent restarted in the same run with `goalcup continue`. The clock does not pause and no task information may be supplied: the only message you may type is "keep going".

## Tracker, auth, and privacy

- The official open-source tracker and Agent Community Auth are required for the full run.
- Logging in (`goalcup login`) records nothing. Recording begins only when you start a run with `goalcup start` and approve it in the browser.
- `goalcup start` is a terminal command. It clones your repository as the workspace, opens the run on the server, starts the recording proxy, and opens the harness in chat mode in your terminal. Desktop apps cannot be launched this way and are not supported.
- **Your 168 hours start when the server opens the run**, just before the harness launches, on the server's clock. `goalcup status` shows a live run's recording state, health, storage and gaps, and each heartbeat tells the server how the recording is doing.
- **The Codex desktop app and Claude Desktop have no supported recording path.** A desktop app cannot be launched with a private environment, so the only way to record one would be a system-wide proxy that captures every application on the machine. We do not do that. Use the CLI.
- Each harness writes its sessions into the tracker's own home for that harness, never your own store, and the tracker records every session file new or written there since the run started. Runs that share a harness home are recorded one at a time.
- **Claude Desktop has no connector in the current build. If a Desktop connector ships in a later release and your entry uses Claude Desktop with raw body capture, run no other Claude Code session on that machine for the duration.** The switch that produces the strongest evidence is app-wide, so it captures every Claude process on the machine. The tracker discards any body that does not belong to the tracked session, but in a six-minute test that still meant sweeping up 23 request bodies that were none of our business, one of them from an unrelated folder. Keep the machine to one Claude session and the problem does not arise.
- **Four app-wide settings changes are permitted, and each one is disclosed on your public entry**: the raw-body switch in Claude Desktop's user settings, the twelve hook-trust entries Codex's own trust screen writes into your Codex config, one plugin allow-list key in the Hermes home config, and the state file, tracking database, skill re-sync and keychain token that the Cursor CLI writes into your real `~/.cursor` no matter how it is pinned. The first three are recorded before and after and reversed at seal. The fourth is the harness doing it to itself; the tracker records it and cannot undo it. Nothing else app-wide is touched. None of the four is performed in the current build: they belong to the desktop, gateway and Cursor entries in the support table below, none of which has a connector here, and no manifest under `connectors/` declares an app-wide exception. The Codex trust entries also belong to a hook-based recording path that the current build no longer has.
- **What a run takes from your own home.** The tracker reads nothing from and writes nothing to your `~/.claude`, `~/.codex` or `~/.hermes`, and the daemon refuses a config home inside any of them. Your agent is not limited the same way: since D52 it runs with full access as your user, so it can read and write anything you can, those folders and your Keychain included. Codex and Claude Code each use one config home in the tracker's own config directory, shared by that connector's runs. The tracker never copies your own settings into it: `goalcup prep` opens the harness there so you sign in and add the settings, MCP servers and skills you want. Claude Code's run `HOME` on macOS holds a link to your `~/Library/Keychains`, for your subscription login. Hermes gets a fresh per-run home seeded only with the connector's own files, and its provider key comes from your environment.
- **What the evidence actually covers.** Every token your agent saw is captured. Every byte a tool produced is not: several harnesses truncate large tool output before writing it, and on some of them the untruncated bytes exist nowhere on disk. A few internal model calls, such as title generators and follow-up suggestion helpers, appear in no transcript on any harness we measured. Neither affects judging, and both are stated here so nobody discovers them later.
- Tracker downtime while the agent kept working is reviewed, not automatically penalized. Downtime while nothing happened is an outage.
- The server is authoritative for the 24-hour start window, the official start, the deadline and the stop. Without a network at the deadline, the tracker stops the run once the run's length has passed on the machine's own clock.
- The tracker should capture the exact prompt, starting workspace state, observable prepared context, later human messages, agent/subagent lifecycle, tool calls and results, external sites/services used, harness and model identity when available, run continuity, and final state.
- The record uploads to the Goal Cup server during the run, every 30 minutes or sooner by size. When the server cannot be reached, the record waits on your disk and the upload retries; the run goes on. After the stop, the tracker seals and uploads for at most one hour, and `goalcup upload` finishes the job later.
- **What the tracker records, and what it never does.** `goalcup start` starts the recording proxy in-process. The tracker observes one process: the harness it launched for your entry, and any program that harness starts. It records that agent's conversation with the model provider, every request and every reply, by pointing that one process at a recording proxy on your own machine through its own environment; nothing else on the machine goes through it. It records what you type into that agent, because the review must see every human message. Traffic from the agent to any other address, and traffic on the provider's address that is not a model call, is noted as the address and the number of bytes each way, nothing more; it is never decrypted. That holds only for programs that use the proxy setting: a tool that ignores it (`ssh`, a raw socket, a command that unsets `HTTPS_PROXY`) connects directly and is not seen at all. Sign-in and token refresh are never decrypted either. Your ChatGPT, Claude or other credentials are never stored, anywhere: the proxy removes them before anything is written, and keeps neither their values nor any fingerprint of them. It does not watch other terminals, your browser, other processes, or audio. It also records the harness build and non-identifying facts about the machine (operating system and version, architecture, CPU count, memory, disk size and free space, WSL2, container, mains power): no host name, user name or serial number. The server keeps the IP address and country of each connection from the tracker, on the server only, never in the trace and never public. Raw traces stay private and are never published.
- Raw traces and terminal output remain private. They are used only for compliance review and the announced research project. They also contain more about you than you might expect: Codex stamps your plan type, credit balance and quota percentage into every token-count event, and harnesses read credentials from outside their own home directories. That is another reason raw traces are never published.
- The public entry includes the original prompt, disclosed starting context, harness/model information when known, verification status (the verifier's `pass`, `flag` or `fail` verdict and its findings), and final result. A verdict is not proof of anything; `docs/CHEAT-CHECKLIST.md` lists what a `pass` does not cover.
- Modifying, disabling, bypassing, or deliberately routing work around the tracker is grounds for disqualification.

## Harness support

Cloud and remote execution modes (Claude Code on the web, Codex cloud, Cursor cloud agents, CI runners, remote servers) are not eligible because they leave no local trace.

We ran a capture test on every harness on 2026-09-03, one per harness and mode, and the results are public in `docs/CAPTURE-RESEARCH.md` (git history at `6494030`). This is the support list that came out of it. **Full** means the capture is complete enough to verify automatically. **Elevated review** means an organizer reads the trace before your entry is accepted; it is not a penalty and it does not affect judging.

**This table is the event's target set, not the list of what a release supports.** **For 6 November the supported harnesses are Codex CLI (`codex-cli`) and Claude Code CLI (`claude-code-cli`), in chat mode, on macOS, Linux and Windows through WSL2.** `hermes-cli` also ships, but it needs a provider key, and a run of a harness given a provider key is never uploaded, so it cannot make an entry. No other row has a connector. Anything below about another harness describes its intended event behavior, not a current support claim.

| Harness | Mode | Support | What you need to know |
|---|---|---|---|
| Claude Code CLI | terminal | full | any build; the build that ran is recorded |
| Codex CLI | terminal | full | the tracker launches the real binary, not the npm wrapper |
| OpenCode | terminal | full | loading the tracker plugin installs a dependency tree into the workspace; it is excluded from your sealed result |
| Claude Desktop | desktop app | elevated review | no other Claude session on the machine if raw body capture is used |
| Codex desktop app | desktop app | elevated review | the one-time `/hooks` trust step above is mandatory, before the goal thread |
| Hermes | terminal | elevated review | one app-wide plugin key; no length gate, so an over-length goal is recorded as a violation rather than refused; in the current build it is capture-only and writes no app-wide key |
| OpenClaw | gateway | elevated review | the runtime must be pinned per model; the tracker checks it before the run starts |
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

## Submission

- The submission is your run's GitHub repository. At the deadline the server stops the run and records the repository's latest commit. Your agent pushes its own work; the tracker never pushes, makes no commit of its own and never force-pushes. Work not pushed by the deadline is not in the submission.
- At the stop the tracker also records the commit your clone had, and a digest of any uncommitted changes, in the sealed record.
- The MIT License must exist in the submitted repository. The result must be a fresh, usable or executable product.
- Whether a presentation packet (a demo video, a questionnaire) is collected after the deadline, and how, is still to decide (below). Nothing is added to your repository for it.
- A local product may run for demonstration, and ordinary runtime data/caches are allowed. Source, history and submitted artifacts must remain unchanged through judging. Online services may not be rebuilt or redeployed with changed code.
- An incomplete or unverifiable result is not automatically misconduct. Organizer-caused capture gaps receive review and an explicit statement of evidentiary limits.

## Compliance and judging

- The tracker and private trace review establish whether the run followed the rules. They do not score the quality of the work.
- Organizers may use automated checks and an internal review agent to find anomalies, but a human makes disqualification decisions.
- Judges receive the original prompt, disclosed starting context, and the repository at the commit recorded at the deadline. They do not receive private raw traces.
- The main Cup goes to the strongest final product. Final judging criteria will be published before T0.
- If entry volume is large, a preliminary panel may create a shortlist using the same published product criteria.
- The first announced judges are Jesse Vincent, creator of Superpowers, and Scott Motte, creator of dotenv.

## Public race and sharing

- The public board may show aggregate entry states such as joined, started, running, sealed, published, and verified.
- Participants remain anonymous by default and may opt into public recognition.
- Agent Community may share aggregate race updates throughout the week.
- A public X post made before T0 is worth one bonus point. The participant submits its URL through the authenticated page or CLI; engagement does not affect the point.
- The starting prompt is published with the entry after the run, not at its start. Raw traces never become public.

## Support and partners

- **Offer free access:** give participants product access, credits, or API quota. Financial sponsorship is not required for this entry-level support.
- **Create an award:** pair event sponsorship with a named prize inside the main Cup. The sponsor supplies a clear criterion, prize, and product expert to help judge it. It does not create a separate competition.
- **Back the event:** contribute general funding, compute, infrastructure, tooling, marketing, distribution, or media support without requiring a product-specific award.
- **Community partner:** no product or funding is required. The partner makes a real promotional commitment—such as an event listing, newsletter, social post, or member outreach—and is publicly listed as an official partner.
- Sponsors and partners must be Agent Community members.
- Awards, their criteria, and judges freeze at T0.
- Judge, partner, and sponsor enquiries: `goalhack@agentcommunity.org`.

## Still to decide

- The exact hours and timezone of the start window, and the judging and result dates.
- Whether a presentation packet (demo video, questionnaire) is collected after the deadline, and how.
- Trace retention, deletion, organizer access, and participant withdrawal terms.
- Event storage/capacity limits and signed binary release policy.
- Final judging rubric, shortlist threshold, and appeal process.
