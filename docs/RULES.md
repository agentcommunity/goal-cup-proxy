# The /goal Cup: rules and event plan

This is the participant contract: what a run may and may not do, and what Goal Cup Proxy touches on your
machine. The step-by-step path is the [participant's guide](PARTICIPANT-GUIDE.md). Open work is in
[TODO](../TODO.md).

**How Goal Cup Proxy runs.** `goalcup start` opens your harness in its normal chat mode, in your own
terminal, in the project folder you start it from. That folder is the workspace; your agent pushes
its work to your new, empty, public GitHub repository. You approve each run in the
browser with your Agent Community account. Two approvals are normal: `goalcup login` signs this
computer in once and starts nothing, and each `goalcup start` approves that one run. You type the
prompt as the first message; after it, the only message you may type is "keep going". Goal Cup Proxy
records the harness's model traffic through a recording proxy on your machine, set for the harness process only, and the harness's session files.
It uploads the record to the Goal Cup server while the run goes on. It never changes a model base URL
or any app-wide trust or config, never sends the agent a message, and never restarts it on its own.
Secrets are removed on your machine before anything is written or uploaded, with one known gap
([your logins](#goal-cup-proxy-auth-and-privacy)). Raw records stay private.

## The idea

**One Prompt. One Agent. 168 Hours.**

One person gives one coding agent one prompt, then lets it work alone for one week. The agent may
plan, create skills, use tools and services, spawn subagents of its own harness, build
infrastructure, and surprise us.

The goal is to show what agents can do today without human product work, and to collect useful
evidence about the current agent ecosystem.

## Timeline

All times are UTC.

- **Release availability:** install the latest published Goal Cup Proxy release on macOS, Linux or WSL2
  with `curl -fsSL https://github.com/agentcommunity/goal-cup-proxy/releases/latest/download/install.sh | sh`
  ([guide](PARTICIPANT-GUIDE.md#1-install)).
- **23 October to 6 November, 00:00:** the dry-run window is open; you can start dry runs on any
  day. A release is planned for 6 November, 12:00; it is not a cutoff, and releases may follow (D70). A dry run is a 168-hour rehearsal
  that does not count ([guide](PARTICIPANT-GUIDE.md#the-dry-run-optional)).
- **Before the window:** you prepare with `goalcup setup` and practice runs. Practice is not limited
  to the window.
- **A dry run started on 0.6.1:** seal it, then update. The installer that shipped with 0.6.1 does not
  check for a recording run; the current one does, but seal first anyway ([guide](PARTICIPANT-GUIDE.md#the-dry-run-optional)).
- **7 November 2026, 00:00:** the 24-hour start window opens. It closes at 00:00 on 8 November.
- **During the window:** you finalize your prompt and start your run.
- **The run's start:** the server opens the run, stamps its official start on its own clock, and
  sets its deadline 168 hours later.
- **At 168 hours, on the server's clock:** a hard stop. Goal Cup Proxy stops the agent in whatever state
  it is in, seals and uploads the record, and the server records the last push to your GitHub
  repository's default branch before the deadline ([Submission](#submission)). The clock never pauses. There is no grace period for a build.
- **24 hours after the end:** the server closes the run. A run whose final upload has not arrived by
  then is closed as *incomplete* and judged on what arrived.
- **After that:** compliance review, preliminary review if needed, judging, public results, and
  research learnings. Judging and result dates are not yet set.

The latest possible run ends 192 hours after the window opens.

## Accounts and runs

- A verified email on an Agent Community account is required.
- Agent Community `pending` status only means the optional endorsement letter has not been signed.
  Pending and endorsed members may both take part.
- You accept the competition terms and the short rule summary on the website
  ([agentcommunity.org/goalcup](https://agentcommunity.org/goalcup)) before a run can be approved.
- **One main run per account.** You approve every competition run in the browser. A new competition
  start inside the start window becomes your main run and replaces any earlier one: the server stops
  the earlier run, and only the main run counts. A replacement run needs a new, empty repository
  and a token for it, because `goalcup start` refuses a repository that already has commits. After the window closes,
  an account can only `goalcup continue` its main run.
- There is no separate registration. Every approved competition start creates a run.
- A run is made of parts. A part is one unbroken stretch of recording. When the agent stops before
  the deadline (a crash, a reboot, a closed window), `goalcup continue` starts the next part of the
  same run, with the same deadline, on the same machine.
- **Practice runs** (`goalcup start --practice`) are short, unlimited, separate from the competition,
  and never replace your main run. A practice run needs no GitHub repository and no token: the agent
  works in the empty folder you start from, and nothing is pushed.
- **Organizer test runs** (`goalcup start --test`) are opened only by organizer accounts. They do not
  count, and they are deleted from the server afterwards.
- Your run needs its own prompt, root agent and empty repository.
- **One harness per run.** Your agent may spawn subagents of its own harness. It must not start a
  different harness (a Codex agent starting Claude Code, under any name), because the cup compares
  harnesses and models, not assemblies of them, and this build cannot record the second harness.
  Goal Cup Proxy does not stop it. It records that traffic too, the verifier reports it, and the run is
  disqualified.
- Work created during one run (a replaced run included) may not be used to seed another run.

## One prompt, then only "keep going"

- Each run begins with one text prompt, which you type as the first message in the harness's own
  terminal. It is recorded exactly, in the harness's session files and its model traffic.
- **Each prompt becomes public at the latest at the end of the event.** Until then it stays private, so
  nobody can copy it during the start window.
- The prompt should be no more than **4,000 characters**. A longer one is not refused; the review sees
  its length. Attachments are allowed.
- Any instruction file your harness reads from the workspace at the start (`AGENTS.md`, `CLAUDE.md` and
  the like) is part of the prompt: it is recorded with it and published with it. Prepared context is
  allowed. Hidden context is not, and there is nowhere to hide it.
- **After the first prompt, the only human message permitted is "keep going".** Goal Cup Proxy records
  every message a human sends to the agent, by any channel, so the review can see it. Any other human
  message is reviewed, and it may remove the run at review. This holds after `goalcup continue`
  too, also when the harness opens a new conversation: do not send the prompt or the push lines
  again. Harness commands typed after the prompt
  (for example `/model` to switch model) count as typing: they are recorded and reviewed, never
  blocked. Choose your model in setup.
- Answering a permission prompt is typing too, so set your harness up in setup to run without them.
  Goal Cup recommends how, per harness, and never changes your harness's settings (D67;
  [Set up your harness for a 168-hour run](HARNESS-SETUP.md)).
- The review works out who said each message from what the model saw and how the harness structures its
  messages. That is a detection, not a proof. An organizer reviews a message it cannot explain.
- Goal Cup Proxy never sends anything to the agent, and never relays, edits or blocks your input: no
  reminders, no automatic "keep going", no automatic restart. `goalcup continue` is yours to run.
- The agent may run `goalcup time-left` to ask how much time the run has left (D66). It is an answer
  the agent asks for; Goal Cup Proxy never sends it anything.
- `goalcup seal` refuses to run inside the agent's session, so the agent cannot stop itself. You may
  run `goalcup seal` yourself from another terminal. It asks first, then stops the agent and ends the
  current part.
  It does not end the run: the run stays open until its deadline, and `goalcup continue` goes on
  with it. When the agent's work is done, you need no extra step.
- **Harness setup before the first prompt is yours.** This holds for every harness. Anything the
  harness shows before you type your prompt is allowed: a sign-in, a browser login, a folder-trust
  question, a model choice, a setup wizard. You choose how to answer it. It is not a message to
  the agent. When `goalcup continue` opens the harness again, you may answer the same start-up
  screens again (Claude Code's folder-trust question, for example).
- After the first prompt, the rule above holds again: the only message you may type is "keep
  going". If a harness asks you to sign in again during the run, the rules do not cover that yet:
  contact support (goalcup@agentcommunity.org) before you answer it.
- An agent that stops early and stays idle until the deadline is permitted. Its idle time is in the
  record.

## Prepared agent setups are allowed

The 4,000-character limit applies to the direct prompt. It is not a limit on the agent's setup.

- Models, skills, rules files, memory, MCP servers, custom tools, services, accounts, compute and
  external websites may be prepared before the run. You keep full freedom of model, effort and
  settings, and the network is on.
- **A GitHub repository is required for each real run.** You create a new, empty, public
  repository (no README, `.gitignore` or licence) and give it to `goalcup start` as
  `--repo <user>/<name>`. Your repository must be empty when the run opens: `goalcup start` checks
  it before the approval, and the server checks again when it opens the run, by asking GitHub
  whether the repository has any commit. A repository with one is refused.
- **The workspace is the folder you start from.** It starts with no README, source, configuration,
  hidden starter files or existing product. It may hold only a `.gitignore`, a macOS `.DS_Store` and `.env` or
  `.env.*` files (your token and your agent's keys; `.envrc` is not one), must be outside any Git repository and not beside
  goalcup's own folders, and every `.env` file must be covered by the `.gitignore`;
  `goalcup start` checks all of this before the approval. The record lists each such key file by
  name and size, never its content or a hash of it, in the folder and in the harness's home; git's
  `.git-credentials` is treated the same. Goal Cup Proxy adds nothing to the folder and makes no
  commit or push in it (at the stop it only reads the repository, to seal it): its own
  `goal-cup` skill goes in the harness's home, not the workspace. Your agent makes the repository
  itself (`git init`) and pushes.
- The agent pushes its work to the repository with the token you made for it (see Keys). Goal Cup
  Proxy passes the agent no git login, SSH agent or `gh` configuration. It does not claim the agent
  cannot reach one: the agent runs as you, and `ssh` reads `~/.ssh` from your account whatever
  `HOME` says. In a test on 26 Oct 2026 the agent used the tester's own SSH key unprompted, so
  with an SSH key it can push to any repository that key reaches. Give the key a passphrase or a
  per-use confirmation (for example the 1Password SSH agent with approval), or run in a Docker
  container ([results](research/results/2026-10-26-mac-retest.md)). The commit the server records at the deadline is what is judged.
- Prepared capabilities may help the agent work. A prebuilt product, substantial product code or an
  existing application may not be smuggled in as "context". The judged result must be a fresh build.
- Participant-controlled instructions and task context available to the agent at the start are frozen
  for that run. A human cannot change them during the run.
- The original prompt and the participant-controlled context the agent actually uses must be disclosed
  when the run is published. Public skills or pages can be linked by exact version. Private or changeable
  material needs a frozen publishable copy.
- An empty workspace is not an empty context, and some harnesses fill it in for you. The Codex desktop
  app injects your saved memory summary and your global skill listing into every thread. OpenCode and
  Cursor read the skills in your home directory into the system prompt. That is allowed, and it is
  disclosed with your run like any other prepared context. If you would rather it were not there,
  turn it off before you start. You cannot change it during the run.
- If Goal Cup Proxy cannot resolve a referenced skill, website, MCP resource, memory or similar source
  automatically, organizers may ask for access and you must provide the exact frozen material.
- Credentials, tokens and unrelated private account data are never published.
- **You will see that it is working.** `goalcup status` shows the recording's state and health, and a
  `PROBLEM:` line for anything wrong. The server emails you when your run starts, when it has heard
  nothing from Goal Cup Proxy for 20 minutes, when your agent has made no model calls for 3 hours and
  again at 6 hours, when the recording has a problem, when Goal Cup Proxy pauses your agent because
  the disk is almost full, and when the run has ended and its commit is recorded. There is no token
  limit.
- Ordinary public internet pages and normal third-party service responses do not need to be
  republished in full. Their source should be traceable when the harness exposes it.
- Skills, plans, task lists, external state and infrastructure the agent creates after the start are
  part of its work and are allowed.

The direct prompt is the only live human instruction. Prepared resources remain subject to the
disclosure and no-side-channel rules.

## Keys

- Never put keys in your repository. Your project folder's `.gitignore` must cover every `.env` and `.env.*`
  file in it (`.env*` does), and `goalcup start` refuses to start otherwise. Residual: the agent can still force
  a file in (`git add -f`). Goal Cup Proxy cannot stop that, and a token published that way is
  yours to revoke.
- Keys go in the project folder's `.env.local`. Set them up before the start; changing them during
  the run is outside help. The GitHub token must be one line, exactly `GH_TOKEN=<token>`. The
  recorder redacts the token's value from everything it records (every common encoding) if it was
  in the folder at the start; a token added later is covered by its `github_pat_` pattern only. So is a token after a
  recorder restart: a reopened entry has no run secrets, and the pattern covers fine-grained
  tokens. A real run needs the token line (a practice run does not). Model keys there are for your agent's own product. The harness's own
  sign-in happened in setup and does not go there.
- **One token kind: a GitHub fine-grained personal access token for the one cup repository, with
  Contents read and write and an expiry of 30 days,** saved as `GH_TOKEN=...` in `.env.local`. You
  make it after the repository exists. A broader token (a classic token, or one for all your
  repositories) lets the agent reach more than you meant to give.
- **Delete the token after your run,** on GitHub, and delete `.env.local`.
- Your agent can use any service and make its own accounts. What it does is in the record and is
  what counts.

## During the run

- One measured root agent owns the run. It may create trace-linked subagents inside the same
  harness. It may not run another harness; one run uses one harness.
- Do not update the harness during the run. Goal Cup Proxy records the harness build that runs and
  refuses no build. Some apps update themselves whatever you do; Goal Cup Proxy samples the version,
  records a change as an event, and an organizer looks at it. That is a flag, not a disqualification.
- **Do not use flags or modes that switch capture off.** Each of these was tested and silently
  produces a session with no evidence: Hermes `--safe-mode`, `HERMES_SAFE_MODE=1` and
  `--ignore-user-config`; OpenCode `--pure`, `OPENCODE_PURE=1`, `OPENCODE_DISABLE_PROJECT_CONFIG=1`,
  `--attach`, `share`, `import`, and the `instance/dispose` and `global/dispose` endpoints; OpenClaw
  incognito threads, `agent exec`, `--profile`, `--dev` and `proxy`; Codex `--ephemeral`,
  `--remote ws://`, `queue`, `fork` and `remote-control`; Cursor `-p` and `--print`, `--api-key` and
  `CURSOR_API_KEY`, `-e`/`--endpoint`, the `&` cloud handoff, `--worktree` and
  `AGENT_CLI_CREDENTIAL_STORE=file`; Claude `--no-session-persistence` and
  `CLAUDE_CODE_SKIP_PROMPT_HISTORY`. All cloud and remote execution modes stay banned for the same
  reason: they leave no local trace.
- Side channels that reach the agent count as human input wherever they come from. The known ones: the
  Codex app's thread tools (`send_message_to_thread`, `create_thread`, `fork_thread`,
  `handoff_thread`) and its queued follow-up mode; OpenCode's `POST /session/:id/shell`, which runs a
  real command and is invisible to hook-based recording; Hermes gateway, serve and cron delivery;
  Claude Remote Control and Channels. A message sent through them is a human message like any other.
  Goal Cup Proxy records it where it can see it, and the review treats one it cannot see as unexplained.
- The human may not write code, answer questions, make product decisions, approve actions or edit the
  product (except the harness setup before the first prompt, above).
- No side-channel prompting. You or a collaborator may not change anything the agent can read: files,
  Linear tasks, GitHub issues, messages, documents, email, databases, websites, forms, dashboards,
  webhooks, environment values or similar state.
- The agent may create and keep its own Linear project, notes, issues, skills or other external state.
  Humans must leave that state alone until the run ends.
- Prepare accounts, keys, permissions, payment methods and spending limits before the start. That
  includes the harness's own approval setting, so it does not stop to ask you before it acts.
  Missing preparation is not itself a violation, but the human may not fix it after the first prompt
  by typing a key, completing a login, clicking an approval or changing the environment. The one
  exception is the harness's own setup before your first prompt
  ([One prompt, then only "keep going"](#one-prompt-then-only-keep-going)).
- The agent may authenticate, purchase, create, deploy or operate resources on its own, using
  authority it already has.
- Power or internet may be restored and the agent restarted in the same run with `goalcup continue`.
  The clock does not pause and no task information may be supplied: the only message you may type is
  "keep going".

## Goal Cup Proxy, auth and privacy

- The official open-source Goal Cup Proxy and Agent Community Auth are required for the full run.
- `goalcup login` records nothing. Recording begins only when you start a run with `goalcup start` and
  approve it in the browser.
- `goalcup start` is a terminal command. It checks the folder you run it in and your repository,
  opens the run on the server after you approve it, starts the recording proxy, and opens the
  harness in chat mode in that folder. It makes no commit or push there; it only reads git, to seal.
- **GitHub calls.** `goalcup setup` makes none. `goalcup start` makes a few public, unauthenticated
  calls to `api.github.com` for the repository check (public, and no commits). If GitHub cannot
  be asked, it says so and goes on, and the server checks when it opens the run. The Worker calls
  GitHub when it opens the run, and again at judging to read the last push. Nothing of your git
  login is passed to the agent: not your SSH agent, your `gh` configuration or your credential
  helper. Your agent runs as you, so it can still read what any program of yours can read (next
  bullets), including an SSH key in `~/.ssh` (see Keys).
- **Platforms.** macOS (Apple silicon and Intel) and Linux (x86_64 and arm64). On Windows, use WSL2
  for the event. Native Windows comes later.
- **Desktop apps are not supported.** The Codex desktop app and Claude Desktop cannot be launched with
  a private environment, so the only way to record one would be a system-wide proxy that captures
  every application on the machine. We do not do that. Use the CLI.
- **Your 168 hours start when the server opens the run**, just before the harness launches, on the
  server's clock. `goalcup status` shows a live run's recording state, health, storage and gaps, and
  each heartbeat tells the server how the recording is doing.
- Each harness writes its sessions into Goal Cup Proxy's own home for that harness, never into your own
  store. Goal Cup Proxy records every session file new or written there since the run started. Runs that
  share a harness home are recorded one at a time.
- **No connector in this build changes app-wide configuration.** Goal Cup Proxy reads nothing from and
  writes nothing to your `~/.claude`, `~/.codex` or `~/.hermes`, and the daemon refuses a config home
  inside any of them. A future connector that must change one would say so in its manifest, and your
  run would disclose it.
- **Your agent is not limited the same way.** It runs with full access as your user, so it can read
  and write anything you can, those folders and your Keychain included. Codex and Claude Code each use
  one config home in Goal Cup Proxy's own config directory, shared by that connector's runs. Goal Cup Proxy
  never copies your own settings into it: `goalcup setup` opens the harness there so you sign in and add
  the settings, MCP servers and skills you want. Claude Code's run `HOME` on macOS holds a link to your
  `~/Library/Keychains`, for your subscription login. Hermes uses one such home too, set up by its
  own wizard when `goalcup setup` opens it. Each harness keeps its own login in its home. The
  harness's access token passes through the recording proxy's memory on its way to the model
  service; it is forwarded unread and never written
  ([`crates/goalcup-cli/src/proxy/http.rs`](../crates/goalcup-cli/src/proxy/http.rs)).
- **Your agent can read your logins, and one recording gap remains.** The harness must read and
  refresh its own login all run, and its tools run as your user. So Goal Cup Proxy cannot stop your
  agent reading the harness's login files in Goal Cup Proxy's home (Codex `auth.json`, Hermes `.env`
  and `auth.json`, Amp `secrets.json`, Claude Code's credential store), or your own `~/.codex`,
  `~/.claude` and Keychain. The harness's login header is never stored. But a key the model
  repeats in its reply is recorded as it streams, in small pieces that redaction does not match
  yet. A trace in which the verifier finds secret-shaped text is not rejected: it is flagged, and
  it is cleaned before anyone else gets it. An uploaded raw record cannot be rewritten. So use a
  login or account you can revoke, keep no unrelated secrets in the workspace or your home folder,
  and log out of the harness or rotate its login after the event.
- **What the evidence covers.** Every request your agent sent to the model service Goal Cup opened is
  captured; a call that bypasses `HTTPS_PROXY` is not. Nor is every byte a tool produced: several
  harnesses truncate large tool output before writing it, and on some the untruncated bytes exist
  nowhere on disk. A few internal model calls, such as title generators and follow-up suggestion
  helpers, appear in no transcript on any harness we measured. Neither affects judging.
  [CHEAT-CHECKLIST](CHEAT-CHECKLIST.md) lists what a verifier `pass` does not cover.
- <a id="what-a-label-is-worth"></a>**What a label is worth.** The verifier labels what each run
  holds (D74): `wire_and_native` (the model requests and the harness's chat file), `wire_only`
  (the model requests alone), `wire_undecoded` (model traffic recorded in a form no decoder reads
  yet) and `native_only` (the chat file alone). Each of these counts. `none` (no model traffic
  and no chat file) does not. "Native" means model work in the chat file: a chat file with no
  model work in it labels `none`.
- Goal Cup Proxy downtime while the agent kept working is reviewed, not automatically penalized. Downtime
  while nothing happened is an outage.
- **A full disk is a red flag.** When the disk fills, the record shows a `StorageFull` gap. It means
  something was off. Judges decide each such gap case by case.
- **A nearly full disk pauses the agent.** Under 1 GiB free beyond the run's reservation Goal Cup Proxy
  freezes the agent, so nothing happens unrecorded, and resumes it by itself above 3 GiB (D65).
- The server is authoritative for the start window, the official start, the deadline and the stop.
  Without a network at the deadline, Goal Cup Proxy stops the run once the run's length has passed on the
  machine's own clock.
- Goal Cup Proxy captures the exact prompt, starting workspace state, observable prepared context, later
  human messages, agent and subagent lifecycle, tool calls and results, external sites and services
  used, harness and model identity when available, run continuity, and final state.
- The record uploads to the server during the run, every 30 minutes or sooner by size. When the server
  cannot be reached, the record waits on your disk and the upload retries; the run goes on. After the
  stop, Goal Cup Proxy seals and uploads for at most one hour. What is still unsent goes up
  by itself the next time you run any `goalcup` command, and `goalcup upload` sends it by hand.
- **What Goal Cup Proxy records, and what it never does.** It observes one process: the harness it
  launched for your run, and any program that harness starts. It records that agent's conversation
  with the model provider, every request and every reply, by pointing that one process at a recording
  proxy on your machine through its own environment; nothing else on the machine goes through it. It
  records what you type into that agent, because the review must see every human message. Traffic from
  the agent to any other address, and traffic on the provider's address that is not a model call, is
  noted as the address, the port, the bytes each way, the times and the reason, nothing more, and
  is never decrypted; that list is uploaded with the seal. This holds only for programs that use
  the `HTTPS_PROXY` setting: a tool that ignores it (`ssh`, a raw socket, a command that
  unsets `HTTPS_PROXY`) connects directly and is not seen at all. Sign-in and token refresh are never
  decrypted either. Your ChatGPT, Claude or other credentials are never stored: the recording proxy removes them
  before anything is written, and keeps neither their values nor any fingerprint of them. Goal Cup Proxy
  does not watch other terminals, your browser, other processes or audio. It also records the harness
  build and non-identifying facts about the machine (operating system and version, architecture, CPU
  count, memory, disk size and free space, WSL2, container, mains power): no host name, user name or
  serial number. The server keeps the IP address and country of each connection from Goal Cup Proxy, on
  the server only, never in the trace and never public.
- **Raw traces and terminal output stay private.** They are used only for compliance review and the
  announced research project. They hold more about you than you might expect: Codex stamps your plan
  type, credit balance and quota percentage into every token-count event, and harnesses read
  credentials from outside their own home directories. That is another reason raw traces are never
  published. They are kept until judging and appeals end, then deleted. The privacy notice on the
  website has the full terms.
- A published run includes the original prompt, the disclosed starting context, harness and model
  information when known, the verification status (the verifier's `pass`, `flag` or `fail` verdict
  and its findings), and the final result. A verdict is not proof of anything.
- Modifying, disabling, bypassing, or deliberately routing work around Goal Cup Proxy is grounds for
  disqualification.

## Harness support

**Where the run happens.** Goal Cup Proxy and the harness run on the same machine, and that machine
may be a laptop, a desktop, or a remote server or cloud sandbox you control (or one the organizers
provide), reached over SSH. It needs a supported disk type (APFS, ext4 or XFS) and a supported system.
Harness modes where the agent itself runs in a vendor's cloud (Claude Code on the web, Codex cloud,
Cursor cloud agents, Amp orbs and Amp's remote executors) and CI runners are not eligible, because
they leave no local trace. Amp's Claude Code mode starts a second harness and is not allowed either.

**Remote computers as tools.** Your agent may use other computers as tools: deploy to a server, run a
database, call an API. Only its own commands and their results are recorded, not what happens on the
far side.

**Other agents.** Your agent may not hand its work to another AI agent outside its own harness: an
agent running on another machine, a hosted coding agent, or a different harness it starts (a sub
harness rather than a subagent). Goal Cup Proxy cannot block this. If the review finds that such an
outside agent did work that ended up in your code, the run is banned.

Every connector shipped with the participant build is available to participants. Run
`goalcup doctor --harness all` for that build's roster; the source [facts index](harnesses/README.md)
records platform measurements and capture/decoder limits. Current chat-mode connectors are:

| Harness | Id | Notes |
|---|---|---|
| Claude Code CLI | `claude-code-cli` | The build that ran is recorded. |
| Codex CLI | `codex-cli` | Launch the native binary, not the npm wrapper. |
| Hermes Agent | `hermes-cli` | Its own setup keeps provider/login state; native snapshots are capture-only. |
| Amp CLI | `amp-cli` | Conversation snapshots are decoded from the recording proxy; local metadata is not a transcript. |

Other harnesses can join through the [connector process](HARNESS-ONBOARDING.md#getting-a-harness-into-the-event).
Releases continue during the cup, and a merged connector ships in the next one. A harness
without a shipped, tested connector has no supported recording path and its runs do not count.
Source inclusion, event qualification and actual platform measurements remain distinct.

## Submission

- The submission is your run's GitHub repository. It must be empty when the run opens; the server
  checks. At the deadline the server stops the run. The judged
  version is the last push to the repository's default branch that GitHub received before your deadline,
  by GitHub's own timestamp; commit dates are ignored. Later pushes are not judged and are flagged, and a
  force-push during the run is flagged. Your agent pushes its own work; Goal Cup Proxy never pushes, makes
  no commit of its own and never force-pushes. Work not pushed by the deadline is not in the submission.
- At the stop Goal Cup Proxy also records the git commit your project folder had (if the agent made
  a repository there), and a digest of any uncommitted changes, in the sealed record.
- The MIT License must exist in the submitted repository. The result must be a fresh, usable or
  executable product.
- **`GOALCUP.md` is required.** The submitted repository has a `GOALCUP.md` at its root that
  explains the project: what it is, how to run it, what works and what does not yet. The agent keeps
  it up to date as it works; judges read the version in the judged push, and it can be your written
  material. Ask for it in your prompt: the goal-cup skill tells Codex and Claude Code, but not every
  harness gets that skill.
- **Demo video and questionnaire.** You hand in a demo video and answer a questionnaire after the run.
  They are due 24 hours after your own run's 168 hours end, so a later start gets a later due time.
  You hand them in on the website, not through Goal Cup Proxy. Nothing is added to your repository for
  them.
- A local product may run for demonstration, and ordinary runtime data and caches are allowed. Source,
  history and submitted artifacts must stay unchanged through judging. Online services may not be
  rebuilt or redeployed with changed code.
- An incomplete or unverifiable result is not automatically misconduct. Organizer-caused capture gaps
  get a review and an explicit statement of the evidence's limits.

## Compliance and judging

- Goal Cup Proxy and the private trace review establish whether the run followed the rules. They do not
  score the quality of the work.
- Organizers may use automated checks and an internal review agent to find anomalies, but a human
  makes disqualification decisions.
- Judges receive the original prompt, disclosed starting context, and the repository at the commit
  recorded at the deadline. They do not receive private raw traces.
- The main Cup goes to the strongest final product. Final judging criteria will be published before
  the window opens.
- If there are many runs, a preliminary panel may create a shortlist using the same published
  product criteria.
- The first announced judges are Jesse Vincent, creator of Superpowers, and Scott Motte, creator of
  dotenv.

## Public race and sharing

- The public board may show aggregate run states such as joined, started, running, sealed,
  published and verified.
- Participants remain anonymous by default and may opt into public recognition.
- Agent Community may share aggregate race updates throughout the week.
- A public X post made before the window opens is worth one bonus point. The participant submits its URL
  on the website, signed in; engagement does not affect the point.
- Each prompt becomes public at the latest at the end of the event (see above). Raw traces never become
  public.

## Support and partners

- **Participant support:** goalcup@agentcommunity.org.
- **Offer free access:** give participants product access, credits or API quota. Financial sponsorship
  is not required for this basic support.
- **Create an award:** pair event sponsorship with a named prize inside the main Cup. The sponsor
  supplies a clear criterion, a prize and a product expert to help judge it. It does not create a
  separate competition.
- **Back the event:** contribute general funding, compute, infrastructure, tooling, marketing,
  distribution or media support without a product-specific award.
- **Community partner:** no product or funding is required. The partner makes a real promotional
  commitment, such as an event listing, newsletter, social post or member outreach, and is publicly
  listed as an official partner.
- Sponsors and partners must be Agent Community members.
- Awards, their criteria and judges freeze when the window opens.
- Judge, partner and sponsor enquiries: goalcup@agentcommunity.org.

## Still to decide

- Judging and result dates.
- Signed binary release policy and event capacity limits.
- Final judging rubric, shortlist threshold and appeal process.
