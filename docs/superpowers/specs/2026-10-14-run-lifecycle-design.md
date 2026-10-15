# Goal Cup run lifecycle: from sign-up to the last upload

Status: design, written 2026-10-14 from a planning session with the owner (Balazs). It replaces nothing
yet; it adds to [`docs/SERVER-DESIGN.md`](../../SERVER-DESIGN.md) and
[`docs/HARNESS-SCALING.md`](../../HARNESS-SCALING.md), and where they disagree this document wins.
Calls marked *(author's call)* were made by the author and are open to correction.

> **Overridden in part.** The owner decisions in the [plan](../plans/2026-10-14-run-lifecycle.md)
> (its header) came after this document and win where they differ: there is no terminal host, no
> `goalcup attach` and no `tmux` requirement (§4: the harness runs in the participant's own
> terminal; over SSH the guide says to start inside `tmux`); the tracer seals and syncs for at most
> one hour after the stop, not until the deadline plus 24 hours (§5; the server closes the run 24
> hours after its end); idle emails go at 3 hours and again at 6 hours (§2, §6); the reservation is
> a floor, not a cap (§5); and a run not sealed 24 hours after its end is `incomplete`. The
> participant's path as built is [PARTICIPANT-GUIDE](../../PARTICIPANT-GUIDE.md).

## 1. Why

On 6 November 2026, 500 to 1,500 participants each start one coding agent and leave it for 168 hours.
The overnight test on 13–14 October ran 8 hours and showed what breaks: the tracer reserved 1 GB of
disk for each of 246 restarts and never released it (274 GB held for 3 GB of data), the last seal hit
its 10-second limit, and a cheap model stopped work after one turn each time. This document says how
one run goes from sign-up to the last upload, so that it survives a week on a participant's own
machine, with the least work for the participant and for us.

**Success:** a participant who has never used the tracer follows the prep steps (or tells their own
agent to follow them), starts one run, and leaves. 168 hours later the run is complete on our server,
with nothing lost, and nobody had to touch it.

**Anti-goals:**
- a run that depends on the participant watching it;
- a disk that fills, or a trace that is lost, because of one failed network call;
- code that is different for each harness or each platform where it does not have to be;
- limits on the participant's harness, model, network or settings.

## 2. Decided by the owner

- **Prompt:** each participant writes their own prompt. A `/goal-cup` skill (sponsor information and
  basic context for the agent) is added to it.
- **Accounts:** one main run for each AgentCommunity account. Starting a new run makes it the main run
  and shuts down any earlier run of that account; only the main run counts. *(author's call: a new start
  is accepted only inside the 24-hour start window; after the window, the account can only `continue`
  its main run.)* Restarts after a crash use `goalcup continue` and stay inside the same run. Practice
  runs have no limit and never replace the main run.
- **Login for each run:** every competition run is approved in the browser with the participant's
  AgentCommunity ID. The tracer is public, so a participant may install it before they sign up; the
  tracer must then send them through login and sign-up first.
- **Harness freedom:** any supported harness, any version, any model, network on, no limits. The
  tracer records the harness build and the models used; it pins and refuses nothing (D52, D53).
- **Platforms:** macOS (Apple silicon and Intel) and Linux (x86_64 and arm64). Windows means WSL2 for
  6 November; native Windows may come later. Code is shared across platforms and harnesses as much as
  possible.
- **The tracer records; it does not manage the agent.** It never sends the agent a message, never
  says "keep going", and never decides for the agent. The participant types the prompt as the first
  message in the harness's own terminal window. After that, the only human message that is permitted
  is "keep going" (typed by the participant). Any other human message removes the run from the
  qualifying runs at review. The tracer records every human message so the review can see it.
- **The harness runs as the participant set it up:** their settings, their permissions mode, their
  model, with the GitHub repository already in place.
- **Keep the machine up:** the tracer keeps the machine awake itself, and the run survives a closed
  terminal (section 4).
- **Disk:** one reservation for each run (3 GB), reused all week as uploads are confirmed, released after
  the last upload.
- **Timing:** more time and more retries for seal and sync; one failed attempt never ends a run.
- **Emails:** 20 minutes with no heartbeat → "your run stopped"; 6 hours with no model calls → "is your
  agent finished, or is something wrong?"
- **Deadline:** 168 hours on the server's clock, a hard stop (SERVER-DESIGN §3 step 6).
- **Data:** stored traces live with us. Participants accept the privacy policy; we store and analyse
  traces, and remove sensitive information from what we analyse or publish. Stored raw traces are
  write-once (the R2 bucket lock stays); there is no deletion on request.
- **Rules:** an agent that is idle until the deadline is permitted; its idle time is in the record.
- **Later, not now:** a "finish run" tool for the agent (MCP), and how the final commit reaches GitHub
  (force push or not). The owner and the judges decide these later; this design does not build them.

## 3. The participant's path

**Prep (any time before their start):**
1. Install the tracer (`brew install`, or the Linux tarball; on Windows inside WSL2).
2. Run `goalcup prep`. *(author's call: one command that walks the steps below and checks each.)*
   1. Log in: opens the AgentCommunity login in the browser. A participant without an account or
      without a Goal Cup sign-up is sent to sign-up first, then back.
   2. Pick a harness from the supported list; the tracer checks it is installed and shows its version.
   3. Log the harness in, inside the tracer's own home for that harness (never the participant's
      `~/.codex` or `~/.claude`). The participant adds their own settings, MCP servers and skills there;
      the tracer records what is in that home at the start of the run.
   4. GitHub (required): the participant creates an empty public repository and a token that can push
      only to it. The tracer stores both, checks the repository is public and empty, and clones it as the
      run's work folder.
   5. The machine: power, free disk (at least 3 GB plus margin), a network check to the server and the
      model hosts, and on WSL2 a check that the work folder is not on `/mnt/c`.
   6. A practice run of a few minutes against the practice event, which uploads and seals for real.
3. An **explainer and a skill for agents** cover the same steps, so a participant can tell their own
   agent "set me up for Goal Cup" and the agent can do it, stopping only for the human steps (browser
   logins, approving the token).

**Start (inside the 24-hour start window):**
1. `goalcup start`. The tracer asks the server to open a run. The server answers
   with a code; the tracer opens the approval page; the participant approves this run in the browser.
   The tracer receives a credential that is good for this one run only, until the deadline plus 24
   hours (the wrap-up upload). *(Device flow, as `website-alignment.md` recommends: the secret never
   goes in a URL.)*
2. The tracer starts the harness in its normal interactive terminal window, in the work folder (the
   clone of the participant's repository), with the participant's own settings and the `/goal-cup`
   skill available. The participant types their prompt as the first message, then leaves.

**During the week** (sections 4–6). The agent commits and pushes to the repository when it wants; the
tracer gives the push token to git only (a credential helper), and redaction removes it from the
record. If the agent stops, the participant may type "keep going"; nothing else. **At the deadline:**
the server says stop; the tracer stops the agent and recording at once, seals, records the work
folder's final commit in the final heartbeat, finishes the last upload, and releases its disk
reservation. The server records the repository's commit at the deadline. *(How the final commit is
made to match GitHub is left for the owner and the judges; see section 2.)*

## 4. Keeping the run alive

- **A terminal that survives:** the harness runs interactively, so its terminal must outlive the
  participant's window. *(author's call)* The tracer hosts the harness's terminal itself (a pseudo-
  terminal it owns, like tmux), so the participant can close the window or lose SSH and later
  `goalcup attach` to see the agent and type. If this is not ready for 6 November, prep tells the
  participant to start inside `tmux` or `screen` and keep it open. `goalcup status` shows the run;
  `goalcup continue` restarts it after a crash or reboot (a new segment, SERVER-DESIGN §3.5).
- **Awake:** the tracer holds a sleep lock for the run (`caffeinate` on macOS, `systemd-inhibit` on
  Linux where present). On WSL2 the Windows host can still sleep; prep tells the participant to set
  Windows power settings, and the 20-minute email catches it.
- **When the agent stops early:** the tracer does nothing to the agent. The record shows the idle time;
  the 6-hour email tells the participant; the participant may type "keep going".
- **When the harness process exits** (a crash, not a stop): the tracer records it, keeps the run open,
  and tells the participant in `goalcup status` and by email. The participant restarts the harness with
  `goalcup continue`.

## 5. Disk and timing

- **One reservation for each run:** 3 GB, taken at the start, held for the whole run. Uploaded data that
  the server has acknowledged (a signed ack whose head the tracer has checked) is removed locally after
  2 hours, and its space is reused. The reservation is released after the last acknowledged upload.
  No reservation for each segment. A trace pruned locally verifies against the server copy
  (SERVER-DESIGN §10).
- **Low disk:** a warning below 5 GB free beyond the reservation; a clean stop (seal and upload) when
  the reservation itself is nearly full and uploads are failing. *(author's call on numbers.)*
- **Timing:** every limit that ends an operation becomes a budget with retries: the seal gets minutes,
  not 10 seconds, and is retried with growing waits; sync retries every minute at first, then up to
  every 15 minutes, without end until the deadline plus 24 hours. A daemon that is slow to answer is
  waited for, not treated as dead. *(author's call: concrete numbers are left to the builder within
  "minutes, not seconds".)*
- **Lighter seal:** the seal records the final git commit and a digest of uncommitted changes, not a
  hash of every file in the workspace. *(author's call)* This is a D53-style change to v5 (older readers
  refuse new seals). The verifier's attribution check uses the git tree of that commit instead of the
  per-file list. Every run has git, because the work folder is a clone of the participant's repository.

## 6. Loud failures and emails

- If recording stops (a full queue, the proxy failing, the session file lost), the tracer says so in
  `goalcup status` at once, and the next heartbeat tells the server, which flags the run.
- **Email 1:** no heartbeat for 20 minutes → "your run stopped; run `goalcup continue`". Once for each
  silence.
- **Email 2:** heartbeats arrive, but no model calls for 6 hours → "your agent made no model calls for 6
  hours: is it finished, out of tokens, or offline?" Once for each idle period.
- Neither email is sent after the deadline.

## 7. Server and website

- **One kind of credential** (owner): every run, practice or competition, starts with the same browser
  approval and gets one credential bound to that run, valid until its deadline plus 24 hours. Stored by
  the tracer with mode 0600, never in the harness environment. *(author's call)* Automated tests use the
  staging server, which has a test-only approval switch; production never has it.

- **Accounts and runs:** one main run for each account (section 2). A new start inside the start window
  replaces the account's earlier run (the server stops it on its next heartbeat); the Worker's unique
  index on open runs and its 409 change to this "replace" rule. Practice runs are a separate event mode
  without the limit.
- **Login:** a device flow. The Worker issues the code and the run credential; the website builds the
  approval page and the sign-up. The long-lived token from `goalcup login` (#59) is replaced by the run
  credential; `goalcup login` becomes the AgentCommunity login for prep.
- **Ownership:** the website repository owns the database schema, its naming and migrations, the
  sign-up, the terms and privacy acceptance, and the approval page. This repository owns the tracer and
  the upload Worker. The Worker uses the website's table names. *(author's call: the website owns the
  database, so its convention wins.)*
- **Storage:** R2 on the same Cloudflare account as the website, write-once with the bucket lock.
- **Emails** go through the website's existing mail setup.

- **Starting point:** the upload stack (#80, #86, #87, #88: open run, uploader, Worker, deadline) is
  merged first; this work amends it. The Worker's SQL is a proposal to the website team, who adopt or
  rename it in their own migrations.

## 8. Tests

- **10-minute matrix:** for each supported harness on macOS arm64, macOS x86_64, Linux x86_64, Linux
  arm64 and WSL2: a real 10-minute run on a cheap model that records, uploads to the test server, seals
  and verifies. Run before each release; the result goes in the harness's facts sheet.
- **Edge cases,** each a scripted test where it can be: 30 minutes without network; laptop sleep; the
  agent process killed; disk nearly full; reboot then `goalcup continue`; tokens run out; the terminal
  window closed; a human message other than "keep going" (the review must see it); the server's stop at
  the deadline; a second `start` on one account replacing the first.
- **Long runs:** an 8-hour run against the test server on macOS and on Linux before the 28 October dry
  run.

## 9. Harness requirements

A supported harness runs in a terminal that the tracer starts, with the participant's own settings, and
writes what the tracer records (its session file and its model traffic, as today). Human messages typed
into it must appear in its session file, so the review can see them. The facts sheet says how the
harness is started, where its settings live, and how a human message appears in the record.

## 10. Not before 6 November

Native Windows; desktop and IDE apps; harnesses that keep no local session file, use SQLite stores or
HTTP/2 (HARNESS-SCALING row 0); a recorder for local models; more than one competition run for each
account; server-side decoding; a finish tool for the agent; a forced final push.

## 11. Order of work

| Dates | Who | What |
|---|---|---|
| 14–17 Oct | tracer | One reservation for each run, budgets and retries, lighter seal, loud failures |
| 14–18 Oct | website | Schema, sign-up, terms and privacy, the approval page |
| 15–20 Oct | tracer + server | Device-flow login, run credential, `goalcup prep`, interactive launch in a terminal the tracer hosts, `goalcup attach`, sleep lock |
| 18–21 Oct | server | Worker on staging with R2, emails, deadline stop |
| 20–24 Oct | all | 10-minute matrix and edge cases; 8-hour runs against staging |
| 24–26 Oct | all | Explainer, agent skill, `/goal-cup` skill; release freeze |
| 28 Oct – 4 Nov | all | Dry run; fixes only |
| 6 Nov | all | Start window opens |

## 12. Left to the builder

Exact budgets, retry constants and thresholds within the rules above; how the detached process is
managed on each platform; the MCP server's shape; the prep command's wording; how pruning is
implemented within section 5; test tooling.
