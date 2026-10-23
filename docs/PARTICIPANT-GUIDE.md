# Taking part in the /goal Cup: the participant's guide

You give one coding agent one prompt, then leave it alone for 168 hours. Goal Cup Proxy
(`goalcup`) records what the agent does and uploads the record. This page takes you from
installing Goal Cup Proxy to the end of your run. The rules themselves are in [RULES.md](RULES.md).

You can also hand this job to your own agent: tell it "set me up for Goal Cup" and point it at the
[goal-cup-setup skill](../skills/goal-cup-setup/SKILL.md). It does the steps a command can do and
stops at each step that needs you.

## What you need

- A Mac (Apple silicon or Intel) or a Linux machine (x86_64 or arm64). On Windows, use WSL2
  (Ubuntu inside Windows) and do everything below inside it.
- `git`.
- A shipped chat-mode harness with the account or model service it uses. The current source
  connectors are Claude Code, Codex CLI, Hermes and Amp; `goalcup doctor --harness all` checks
  every shipped connector and reports install status. It can exit 1 when an optional harness is
  not installed. See the [facts sheets](harnesses/README.md) for measured support
  and capture-only limits. Goal Cup Proxy records which harness build ran.
- A GitHub account.
- An Agent Community account, signed up for the Goal Cup on the website,
  [agentcommunity.org/goalcup](https://agentcommunity.org/goalcup). You also hand in your demo
  video and questionnaire there after the run.

## 1. Install

Open a terminal (on Windows, the Ubuntu one inside WSL2) and run:

```sh
curl -fsSL https://github.com/agentcommunity/goal-cup-proxy/releases/latest/download/install.sh | sh
```

This installs the latest release. It picks the build for your machine, checks it against the
release's SHA-256 list, and puts `goalcup` and its helper programs in `~/.local/bin`. It needs no
`sudo` and changes no setting. If it says `~/.local/bin` is not on your `PATH`, run the `export`
line it prints, and add that line to `~/.zshrc` or `~/.bashrc` to keep it. Then check:

```sh
goalcup --version
```

To upgrade, run the same command again. Do not upgrade goalcup during your run. If no release is
published yet, build from source as the [README](../README.md#build) says. Install inside WSL2,
not in Windows itself; native Windows comes later.

## 2. Log in: `goalcup login`

```sh
goalcup login
```

It opens your browser and prints:

```
Connect this computer to your Goal Cup account.
You do this once per computer. Each run asks for its own approval later; that is normal.
Your browser opened the approval page. Check that the page shows the code <code>.
If you do not see it, open this address: <address>
Waiting for you to approve… (the code expires in 15 min; press Ctrl-C to cancel)
```

Check that the page shows the same code, and approve. Then **return to the terminal now**. It
prints `Approved. This computer is connected to your Goal Cup account (<your name>).` and
`Next: goalcup prep`. Nothing secret is typed or stored: Goal Cup Proxy keeps the server's address and
your account's name. The server is `https://ingest.agentcommunity.org`; you do not need to name it.

**Two approvals are normal.** You approve once here, and once at each `goalcup start`. They do
different jobs:

- The login approval signs this computer in to your account. It gives Goal Cup Proxy no credential for
  any run, and it starts nothing. You do it once per computer.
- The approval at `goalcup start` approves that one run. Each run gets its own, so nothing records
  or opens a run on your account unless you approved that run, at that moment. For a competition
  run it is also the moment you accept that the new run replaces an earlier one.

`goalcup continue` asks for no approval: it restarts a run you already approved.

If you have no account yet, or have not signed up for the Goal Cup, the website sends you through
that first. A remote server or cloud sandbox you control is fine (RULES, "Where the run happens"); run
everything there over SSH. On a machine with no browser, set `GOALCUP_NO_BROWSER=1`.
`goalcup` then prints the address and the code and opens nothing: open the address on any device.

## 3. Get ready: `goalcup prep`

```sh
goalcup prep
```

Prep checks six steps, one line each: `[ok]`, `[todo]` and what to do, or `[fail]` and why. Run it
again as often as you like. What prep has settled (your harness, its sign-in, your repository
and token) is not asked again; the rest is checked each time. Prep never starts a run. When
every step is ok it says `goalcup prep: ready.` and prints the start command. Otherwise its last
line names the first step that needs you.

1. **login.** You have run `goalcup login`.
2. **harness.** A shipped harness on your `PATH`, with its version. Prep discovers the installed
   connectors. Choose one, for example `goalcup prep --harness codex-cli`; the current source
   also has `claude-code-cli`, `hermes-cli` and `amp-cli`. Prep remembers your choice. The line
   ends with a link to your harness's section of
   [Set up your harness for a 168-hour run](HARNESS-SETUP.md). Read it before a long run.
3. **harness_home.** Prep opens the harness once, in the home Goal Cup Proxy keeps for it. This is
   never your own `~/.claude` or `~/.codex`. Sign in there, with a login or account you can
   revoke (section 11 says why), and add the settings, MCP servers and skills you want your agent
   to have. Then exit the harness normally. Your runs use this home.
   With Hermes, choose a model service that uses an API key, not a browser sign-in (OAuth): a
   browser sign-in may fail to renew itself during a run, and the agent then stops
   ([sign-in lifetime](HARNESS-SETUP.md#4-sign-in-lifetime)).
   Nothing is recorded while prep has it open. Prep first lists these steps and says `Press Enter
   to open <harness>, or Ctrl-C to stop.` It opens the harness only after you press Enter.
4. **github.** Create a new repository on GitHub: public and empty (create it without a README,
   licence or `.gitignore`, so it has no commits). Then create a fine-grained token: give it
   access to this repository only, and set Contents to read and write. Set its expiry to at
   least 10 days after the day you plan to start. A token that expires stops your agent's pushes,
   and work that is not pushed is not judged. Run
   `goalcup prep --repo https://github.com/<owner>/<repo>` and paste the token when asked; it is not shown as you type.
   Your agent pushes its work to this repository, and Goal Cup Proxy hands the token to `git` when it
   pushes; it is never in the agent's environment.
5. **machine.** Mains power, enough free disk (a run reserves 3 GiB and warns when less than 5 GiB
   is free beyond that), the server and your model provider reachable, and `git` installed. It also
   says whether Goal Cup Proxy can keep the machine awake. When it cannot (always under WSL2, and on
   Linux without `systemd-inhibit`), the step still passes and tells you to keep the machine awake
   yourself; under WSL2, set sleep to Never in Windows. Under WSL2 it also checks that you are on
   WSL2 (not WSL1) and that Goal Cup Proxy's folders are not on a Windows drive such as `/mnt/c`. The
   folder each run's clone is made in is `goalcup-work` in your home folder; choose another with
   `--work-root <full path>`.
6. **practice.** A practice run of this harness, sealed and uploaded (the next section).

If a firewall such as Little Snitch asks about `goalcup`, allow it: the machine step fails when
the server or the provider cannot be reached. Each Goal Cup Proxy update is a new program file, so the
firewall may ask again after every update. Allow it again each time, and check it before you start
a run you care about.

**Set your harness so it does not stop to ask you.** During the run nobody may approve or answer
anything for the agent. For Codex and Claude Code, Goal Cup Proxy puts a `goal-cup` skill in the
harness home that tells the agent so: it makes and approves every decision itself and does not
wait for a reply. That tells the model only. Many harnesses also stop and ask you before they run
a command, edit a file or push, and that is a harness setting. You may not answer those prompts
during the run, so a harness that asks can wait for days. Set it before the run, in the harness
home that prep opens. You choose how. For example, Claude Code has a permission mode that does
not ask (`bypassPermissions`), and Codex can be set to never ask for approval
(`approval_policy = "never"`). Try your setting in a practice run.
[Set up your harness for a 168-hour run](HARNESS-SETUP.md) lists every setting per harness and
where it goes.

**Hermes and Amp.** The `goal-cup` skill does not reach these two harnesses yet, so their agent is
not told that it can check the clock. If you want it to know how much time is left, say in your
prompt that it may run `goalcup time-left`, or call the `time_left` tool if you
[added it](HARNESS-SETUP.md#add-the-goal-cup-time-left-tool).

## 4. A practice run: `goalcup start --practice`

```sh
goalcup start --harness <id> --practice
```

A practice run goes through the whole path: approve it in the browser, the harness opens in your
terminal in a fresh clone of your repository, you type a prompt, and the run records and uploads.
It lasts about ten minutes; then the server stops it, and Goal Cup Proxy seals and uploads it.

This is the second approval. You approved the login in section 2. This one approves this one
practice run, and it says so:

```
Approve this practice run of Claude Code in your browser.
Every run needs its own approval, so this is normal: login connected this computer, and this approval starts this one run.
```

Approve it in the browser, then **return to the terminal now**: the run starts there. Section 5
walks through what you see.

A practice run never pushes. Its clone has no way to push, so your repository stays empty until
your competition run. Practice runs do not count, never replace your competition run, send no
emails, and you can do as many as you like. Run `goalcup prep` afterwards: the practice step
should now be ok.

## 5. Your run: `goalcup start`

Start inside the 24-hour start window. It opens on 7 November 2026 at 00:00 UTC and closes at 00:00
UTC on 8 November.

```sh
goalcup start --harness <id>
```

Run it from your home folder or another folder of your own: `goalcup start` makes a folder named
`goalcup-` and a random name in the folder you run it from, for the harness's home during the
run. Do not run it from any folder inside your work folder; Goal Cup Proxy refuses that. Under WSL2,
do not run it from a Windows drive.

1. **Approve.** `goalcup` prints `Approve your competition run of Claude Code in your browser.`
   and the same line as above about the second approval, then opens your browser:
   `Your browser opened the approval page. Check that the page shows the code <code>.` Check the
   code, approve this run, and **return to the terminal now**. If you already have a competition
   run open, it first says so (`You already have a competition run open (started …). Approving
   replaces it: that run stops, and only the new run counts.`). Each competition run is approved
   this way. `goalcup` prints `Approved.` when the approval arrives.
2. **Clone.** It prints `Cloning your repository…` and clones your repository into your work
   folder (`goalcup-` followed by a random name). That clone is the agent's workspace.
3. **Open the run.** It opens the run on the server. Your 168 hours start here, on the server's
   clock.
4. **The harness opens in this terminal.** Before it does, goalcup prints a short block. It says
   `Your competition run is open.` and `Claude Code opens now in this terminal.`, and then what to
   do:

   ```
   Claude Code may ask whether you trust this folder. Choose Yes.
   Type your one prompt and press Enter. After that you may only type "keep going", when the agent has stopped.
   Your run ends at <day, time and zone, with UTC in brackets>, <N> h <M> min left. You do not need to stop it.
   goalcup prints nothing here until Claude Code exits. To see the run, run goalcup status in another terminal.
   Recording in chat mode: Claude Code opens now.
   ```

   Type your prompt as the first message, and press Enter. After that, type nothing but "keep
   going" (section 10). Tip: write your prompt in a file before the window opens. Then paste it
   whole and press Enter once, so it goes as one message.

**The harness's own setup screens.** Before you type your prompt, your harness may show its own
screens: a sign-in, a browser login, a folder-trust question, a model choice or a setup wizard.
This holds for every harness. Answer them as you choose; that is allowed, and it is not a message
to the agent. Once you have typed your prompt, the "keep going" rule holds (section 10).

For example, each run is a fresh clone, so Claude Code asks at the start of every run whether you
trust the folder. The cursor is on "No, exit". Choose "Yes, I trust this folder", then type your
prompt. If you choose "No, exit", Claude Code quits and Goal Cup Proxy ends this part of the run.
Run `goalcup status` and do what its `Next:` line says. When that is `goalcup continue`, Claude
Code opens again with a new session and asks again.

While the harness is open, `goalcup` prints nothing in this terminal. Use `goalcup status` from
another terminal to see the run.

When the harness exits, `goalcup` takes the terminal back:

1. Under the heading `Details for support` it prints technical lines. They are for support. You
   can skip them.
2. Below them it says what happened. If the agent exited before the deadline, it says
   `Claude Code has exited (<how>).` and `Your run is still open until <time>.`
3. Then it says what to do:
   `To go on, run: goalcup continue <run id> (you can do it in another terminal now).`
4. Last, it prints
   `Uploading the rest of the run… keep this terminal open until it says the upload is done.`
   Keep this terminal open until `Upload done.` appears.

**One run counts.** Your account has one main run. If you start a second competition run inside
the window, it replaces the first: the server stops the first run, and only the new one counts.
The new run needs a new, empty repository: `goalcup start` refuses a repository that already has
commits, so make one and point prep at it with `goalcup prep --repo <new URL>`.
After the window closes you cannot start a new competition run; you can only `goalcup continue`
the one you have. To replace a run on the same machine with the same harness, exit the first
run's harness before you start again: a second `goalcup start` of that harness is refused while
the first is still recording.

## 6. Leaving the machine

The run lasts a week. For all of it:

- **Keep the machine on mains power.** On battery a Mac can still sleep, through its lid or its
  power settings.
- **Keep a laptop's lid open.** A Mac sleeps when its lid closes, whatever Goal Cup Proxy does.
- **Keep it online.** Your agent needs the network for every model call. Uploads and the
  server's heartbeat need it too. When the network drops, Goal Cup Proxy keeps recording and
  retries the uploads for as long as the run lasts.
- **WSL2:** Goal Cup Proxy cannot keep Windows awake. In Windows Settings, System, Power, set sleep to
  Never for the week of your run.
- **Keep the terminal window open.** Closing it stops the agent. Goal Cup Proxy still seals that part
  of the run and uploads it, and the run stays open: restart the agent with `goalcup continue`
  (section 8).
- **Over SSH, start the run inside `tmux`** (or `screen`), so a dropped connection does not close
  the terminal.
- **Keep disk space free.** If the disk fills, the record shows a gap marked `StorageFull`. That
  is a red flag: something was off. The judges decide each such gap case by case. If you must
  free space, free it outside your run's folders (the clone in your work folder and the
  `goalcup-` folder that `goalcup start` made). Never delete anything the agent made.

The server emails you about your run: when it starts; when it has heard nothing from Goal Cup Proxy
for 20 minutes; when your agent has made no model calls for 3 hours, and again at 6 hours; when
the recording has a problem; when Goal Cup Proxy pauses your agent because the disk is almost full
(free space outside your run's folders, and the agent resumes by itself); and when the run has
ended and its commit is recorded.

## 7. Checking on the run: `goalcup status`

```sh
goalcup status
```

It shows the newest run started on this machine. A run that records, with nothing wrong, looks
like this:

```
Your competition run with Claude Code: recording.
  Ends: Sat 14 Nov, 10:12 CET (09:12 UTC), 95 h 10 min left.
  Claude Code: running.
  Upload: working, sent every 30 minutes (last sent 12 min ago; 2140 records wait for the next send).
  Problems: none.
Next: nothing to do. The run ends by itself.
```

The header says which run it is, its state, when it ends and how much time is left, whether the
agent runs, how the upload stands, and how many problems there are. Goal Cup Proxy sends the
record every 30 minutes, so records waiting between sends are normal; the line says `up to date`
only when the server has acknowledged every record written. If there are problems, one
`PROBLEM:` line each follows the header, and each says what to do. The last line is always
`Next:`, and it says what to do now. A block headed `Details:` follows, for support. On a machine
that has not started a run, `goalcup status` says `no run has been started from this machine yet`.

| The line says | What it means | What to do |
|---|---|---|
| `goalcup has not updated this run for …. The computer may have slept or restarted.` | The `goalcup start` process is not running: the machine rebooted or slept, or the process was killed. | The line ends with the fix. Most often it is `To go on, run: goalcup continue <run id>`. |
| `the recording has gaps` or `the recording failed` | Part of the record is missing. The server is told and the run is flagged for review. | Leave the run going. If it says to write to support, name the run id it gives. |
| `nothing was uploaded for … Nothing is lost; it waits on this computer. Check the network: …` | Uploads have been failing for half an hour or more. Nothing recorded is lost: it waits on the disk. | Check the network, and that no firewall such as Little Snitch is holding goalcup. After the run, `goalcup upload` sends what is left of your newest run. `goalcup upload <part-id>` sends one part. |
| `the server has not heard from this run for about … min` | The server has not heard from the run for about a quarter of an hour. | Check the network. |
| `only … of disk is free. Free some space outside your run's folders, …` | Less than 5 GiB is free beyond the run's reservation. | Free space outside your run's folders. Never delete anything the agent made. Under 1 GiB Goal Cup Proxy pauses your agent (the line below). |
| `The disk is almost full, so your agent is paused. Free some disk space outside your run's folders, and do not delete anything the agent made; it resumes by itself. If your agent seems stuck afterwards, type keep going.` | Less than 1 GiB is free beyond the run's reservation, so Goal Cup Proxy froze the agent: nothing happens unrecorded. The run never stops for the disk. | Free space outside your run's folders. Never delete anything the agent made. Above 3 GiB free the agent resumes by itself. If it then seems stuck, type `keep going`. |
| `Goal Cup has recorded … requests to the model service but no Claude Code session file yet. …` | Goal Cup Proxy recorded requests to the model service for ten minutes (counted from the first one, at least ten of them), but the harness has written no session file it could record. Its setting may be off, it may be another version, or its session folder may have moved. The run is weaker evidence until it does. It is also written once to the run's `host.log`. | Keep the agent running: nothing is stopped. Check that the harness is the supported version and set up as its section of [HARNESS-SETUP](HARNESS-SETUP.md) says (the line links to it). The line goes away once a session file is recorded. |
| `this computer may go to sleep (…)` | Goal Cup Proxy could not keep the machine awake. Under WSL2 this line is always there, because Windows decides. | Keep it on mains power with the lid open, or turn sleep off yourself (WSL2: the Windows power setting above). |
| `Claude Code has exited (…). Your run is still open until … To go on, run: goalcup continue <run id>` | The agent ended before the deadline: it crashed, you closed the window, or it quit. | Run the command it names. |
| `Claude Code has exited (…) and the run is over. Nothing to continue.` | The agent left on its own, and the run's time has passed since. | Nothing. |
| `Claude Code has exited (…) and this run cannot be continued.` | The agent left, and this run cannot be restarted: it is not on the server, or this machine has no credential for it. | Nothing to restart. |

(Goal Cup Proxy writes the harness's own name where this table says Claude Code.)

When Goal Cup Proxy stops the agent itself, at the deadline or because a newer run replaced this one,
`status` shows no `PROBLEM:` line about it. An agent that exited normally gets a plain line, not a
`PROBLEM:`: `Claude Code finished, and the run is over.` or `Claude Code finished, and this run
cannot be continued.`

## 8. Restarting the agent: `goalcup continue`

```sh
goalcup continue
```

When the agent stops before the deadline (a crash, a reboot, a closed window), this starts it
again in the same run: same deadline, same clone. The harness picks up its last session. If the
run has no session yet (the harness closed before your first message), the harness starts a new
one, and `goalcup` says so in one line. No approval is needed, and it says so: `Your run goes on.
No approval is needed: this computer kept the approval for this run.` While it opens, it prints
`The earlier part of this run uploads in the background.`

The harness may show its setup screens again (section 5), such as Claude Code's folder-trust
question. You may answer them. If it asks you to sign in again, contact support before you answer
(see Help). If the agent has stopped, type "keep going". If you never sent your prompt, send it
now: it is still your one prompt. Run `goalcup continue` from the same kind of folder as
`goalcup start` (section 5), in a terminal you will leave open.

Choose your model and model service in prep, before the run. Typing a harness command such as
`/model` after your prompt is typing like any other message: Goal Cup Proxy records it and it is
reviewed. Nothing is blocked.

A run can be continued only on the machine that started it, with the run's folder (its clone in
your work folder) where it was. A clone you moved to another disk may be linked back at its old
path. If that folder is gone, or is no longer a clone (its `.git` folder removed), `goalcup
continue` says so and changes nothing. If the clone is lost, end the run's last part so it still
reaches the server: `goalcup seal <part> --yes`, then `goalcup upload` (both work without the
clone). Goal Cup Proxy never restarts the agent on its own.

**Stopping the agent yourself: `goalcup seal`.** Run it from another terminal, never inside the
agent's session. It asks first: type `yes`. (`goalcup seal --yes` skips the question; a script
must use it.) Then it stops the agent and ends the current part. It does not end the run: the run
stays open until its deadline, and `goalcup continue` goes on with it. When the agent's work is
done, you need no extra step. Leave it; the run ends by itself at the deadline.

## 9. The deadline, and after it

Your run ends 168 hours after its official start, on the server's clock. The stop is hard: the
agent is stopped in whatever state it is in. There is no grace period.

- At the deadline the server tells Goal Cup Proxy to stop. It stops the agent and the
  recording, seals the record, tells the server which commit your clone had, and uploads what is
  left. It prints, in this order:
  1. `Time is up. Claude Code was stopped.`
  2. `Uploading the rest of the run… keep this terminal open until it says the upload is done.`
  3. When everything is sent: `Your run is sealed and uploaded. Nothing more to do.`

  It keeps trying for up to one hour. If that is not enough, it says
  `Your run is sealed on this computer.` and
  `Part of the run is not uploaded yet. Next: when you are online, run: goalcup upload`.
  What is left goes up by itself the next time you run any `goalcup` command while you are
  online. To send it yourself, run `goalcup upload`. It sends every part of your newest run that
  has not gone up yet.
- The server records the last push to your repository's default branch that GitHub received before
  the deadline (GitHub's timestamp, not the commit date). That commit is what is judged, so your agent
  must push its work before then; later pushes are not judged and are flagged, and so is a
  force-push. Goal Cup Proxy never pushes for it.
- The MIT License must be in your repository at that commit. Your repository starts empty, and
  you may not edit it during the run, so ask for it in your prompt.
- A `GOALCUP.md` at the root of your repository must explain the project: what it is, how to run
  it, what works and what does not yet. Judges read the version at that commit. Ask for it in your
  prompt too; not every harness gets the goal-cup skill that asks for it.
- After the run you hand in a demo video and answer a questionnaire. They are due 24 hours
  after your run's 168 hours end. You hand them in on the website,
  [agentcommunity.org/goalcup](https://agentcommunity.org/goalcup), not through Goal Cup Proxy.
- Without a network at the deadline, Goal Cup Proxy stops the run once 168 hours have passed on the
  machine's own clock, which keeps counting while the machine sleeps.
- 24 hours after the end, the server closes the run. A run whose last upload has not arrived by
  then is closed as incomplete, and it is judged on what did arrive.

## 10. The "keep going" rule

You type your prompt as the first message. After it, the only message you may type to the agent
is "keep going". Use it when the agent has stopped and you want it to carry on.

Every message you type into the harness is recorded. Any other message (an answer, a hint, an
approval, a correction) is seen at review, and it may remove the run at review.

One thing is not a message to the agent, and it is allowed: the harness's own setup screens
before you type your prompt (section 5), such as a sign-in or Claude Code's folder-trust question.
You may answer them again when `goalcup continue` opens the harness. Do not answer a question the
agent asks or an approval the agent needs. If the harness asks you to sign in again during the
run, contact support before you answer.

So set your harness up in prep to run without permission prompts: `goalcup prep` says which
setting Goal Cup recommends when it is missing, and
[Set up your harness for a 168-hour run](HARNESS-SETUP.md#1-run-without-permission-prompts) shows
where it goes. Goal Cup never changes it for you.

Goal Cup Proxy itself never types to the agent, never says "keep going" for you, and never restarts
it by itself. Your agent may ask how much time is left with `goalcup time-left`, or with the
[time-left tool](HARNESS-SETUP.md#add-the-goal-cup-time-left-tool) if you added it. You can run it
in your own terminal too. `goalcup seal` works only from a terminal of your own, never inside the
agent's session. It asks before it stops the agent, and your run stays open (section 8).

**No side channels.** During the run, neither you nor anyone helping you may change anything the
agent can read: files, tasks, GitHub issues, messages, documents, email, databases, websites,
forms, dashboards, environment values and the like. A message that reaches the agent by another
way (a thread tool, a remote control, a chat bridge) counts as a human message. The agent may keep
its own notes, issues and other state. Leave them alone until the run ends.

An agent that stops early and stays idle until the deadline is allowed. The idle time is in the
record.

Your agent may start subagents of its own harness. It must not start a different harness (a Codex
agent starting Claude Code, say), and it must not hand work to an AI agent outside its harness, on
this machine or another. Goal Cup Proxy cannot record those; if the review finds that such an
outside agent did work that ended up in your code, the run is banned.

Your prompt becomes public at the latest at the end of the event. Until then it stays private.

## 11. What is recorded

Goal Cup Proxy records your run, and only your run:

- everything the harness sends to its model provider and everything it gets back;
- the harness's own session files written during the run, including every message you type;
- the commit in your clone at the end, and a digest of any changes not committed;
- the harness build that ran;
- facts about the machine: the operating system and its version, the architecture, the number of
  CPUs, the memory, the disk size and free space, whether it is WSL2 or a container, and whether
  it is on mains power.

It records no host name, no user name and no serial number. It does not look at your other
programs, your browser, other terminals or your files outside the run. Its heartbeats report the
free disk space and the memory, CPU time and open files of Goal Cup Proxy's own processes, and
nothing about any other process. Passwords, keys and tokens
are removed on your machine before anything is written, with one known gap (below). The server
also keeps the IP address and country of each connection from Goal Cup Proxy; they are stored on the
server only, never in the record, and never made public.

**Your logins.** Your agent runs as you, so it can read the harness's login files and your own
`~/.codex`, `~/.claude` and Keychain. Goal Cup Proxy cannot stop that without stopping the harness,
which must read its login all run. The harness's login header is never stored. But if the model
repeats a key in its reply, the reply is recorded as it streams, in small pieces, and Goal Cup Proxy
cannot remove the key from those pieces yet. Such a trace is flagged and cleaned before anyone
else gets it, but the raw upload cannot be changed. So:

- sign the harness in with a login or account you can revoke;
- keep no secrets that have nothing to do with the run in the workspace or your home folder;
- after the event, log out of the harness or rotate its login.

Raw records stay private and are never published.

To see your own numbers, run `goalcup-verify --stats <trace> --output my-run.json`, with the folder
of the `trace:` line Goal Cup Proxy printed when it sealed the run (name each part's folder, in
order, if you ran `goalcup continue`). It writes tokens, models, tool calls and errors, subagents and how often you
typed "keep going": counts only, no text.

## 12. Test runs (organizers only)

`goalcup start --test` opens an organizer test run. It runs for the full length, like a
competition run, but only organizer accounts can open one, and all test runs are deleted from
the server together afterwards. It is not for participants: use `--practice` to try things out.

## Help

Write to goalcup@agentcommunity.org. Say which command you ran, and paste the `goalcup status`
output and the run id it shows. Never paste your GitHub token.
