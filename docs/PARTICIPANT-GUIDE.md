# Taking part in the /goal Cup: the participant's guide

You give one coding agent one prompt, then leave it alone for 168 hours. The Goal Cup tracer
(`goalcup`) records what the agent does and uploads the record. This page takes you from
installing the tracer to the end of your run. The rules themselves are in [RULES.md](RULES.md).

You can also hand this job to your own agent: tell it "set me up for Goal Cup" and point it at the
[goal-cup-setup skill](../skills/goal-cup-setup/SKILL.md). It does the steps a command can do and
stops at each step that needs you.

## What you need

- A Mac (Apple silicon or Intel) or a Linux machine (x86_64 or arm64). On Windows, use WSL2
  (Ubuntu inside Windows) and do everything below inside it.
- `git`.
- A harness the tracer runs in chat mode: Claude Code (`claude-code-cli`) or Codex CLI
  (`codex-cli`), any build, with the subscription or account it signs in with. The tracer records
  which build ran; it does not refuse one.
- A GitHub account.
- An Agent Community account, signed up for the Goal Cup on the website.

## 1. Install

With Homebrew, from the first release (23–24 October 2026):

```sh
brew install agentcommunity/tap/goalcup
```

Before the release, build from source as the [README](../README.md#build) says. On Windows, install
inside WSL2, not in Windows itself. Native Windows comes later.

## 2. Log in: `goalcup login`

```sh
goalcup login
```

It prints an address and a short code and opens your browser. Check that the page shows the same
code, and approve. Nothing secret is typed or stored: the tracer keeps the server's address and
your account's name. The server is `https://ingest.agentcommunity.org`; you do not need to name it.

If you have no account yet, or have not signed up for the Goal Cup, the website sends you through
that first.

## 3. Get ready: `goalcup prep`

```sh
goalcup prep
```

Prep checks six steps, one line each: `[ok]`, `[todo]` and what to do, or `[fail]` and why. Run it
again as often as you like. What prep has settled (your harness, its sign-in, your repository
and token) is not asked again; the rest is checked each time. Prep never starts a run. When
every step is ok it says `goalcup prep: ready.` and prints the start command.

1. **login.** You have run `goalcup login`.
2. **harness.** The harness you will run, found on your `PATH`, with its version. This build
   offers two, so choose one: `goalcup prep --harness claude-code-cli` or
   `goalcup prep --harness codex-cli`. Prep remembers your choice.
3. **harness_home.** Prep opens the harness once, in the home the tracer keeps for it. This is
   never your own `~/.claude` or `~/.codex`. Sign in there, and add the settings, MCP servers and
   skills you want your agent to have. Then exit the harness normally. Your runs use this home.
   Nothing is recorded while prep has it open.
4. **github.** Create a new repository on GitHub: public and empty (create it without a README,
   licence or `.gitignore`, so it has no commits). Then create a fine-grained token for that one
   repository with Contents set to read and write. Run `goalcup prep --repo
   https://github.com/<owner>/<repo>` and paste the token when asked; it is not shown as you type.
   Your agent pushes its work to this repository, and the tracer hands the token to `git` when it
   pushes; it is never in the agent's environment.
5. **machine.** Mains power, enough free disk (a run reserves 3 GiB and warns when less than 5 GiB
   is free beyond that), the server and your model provider reachable, and `git` installed. It also
   says whether the tracer can keep the machine awake. When it cannot (always under WSL2, and on
   Linux without `systemd-inhibit`), the step still passes and tells you to keep the machine awake
   yourself; under WSL2, set sleep to Never in Windows. Under WSL2 it also checks that you are on
   WSL2 (not WSL1) and that the tracer's folders are not on a Windows drive such as `/mnt/c`. The
   folder each run's clone is made in is `goalcup-work` in your home folder; choose another with
   `--work-root <full path>`.
6. **practice.** A practice run of this harness, sealed and uploaded (the next section).

If a firewall such as Little Snitch asks about `goalcup`, allow it: the machine step fails when
the server or the provider cannot be reached.

## 4. A practice run: `goalcup start --practice`

```sh
goalcup start --harness <id> --practice
```

A practice run goes through the whole path: approve it in the browser, the harness opens in your
terminal in a fresh clone of your repository, you type a prompt, and the run records and uploads.
It lasts about ten minutes; then the server stops it, and the tracer seals and uploads it.

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
run. Do not run it from any folder inside your work folder; the tracer refuses that. Under WSL2,
do not run it from a Windows drive.

1. The tracer prints `Open <address> and check that it shows the code <code>.` and opens your
   browser. Approve this run. Each competition run is approved this way.
2. It clones your repository into your work folder (`goalcup-` followed by a random name). That
   clone is the agent's workspace.
3. It opens the run on the server and prints the official start and the deadline. Your 168 hours
   start here, on the server's clock.
4. The harness opens in this terminal. Type your prompt as the first message. After that, type
   nothing but "keep going" (below).

While the harness is open, `goalcup` prints nothing in this terminal. Use `goalcup status` from
another terminal to see the run.

**One run counts.** Your account has one main run. If you start a second competition run inside
the window, it replaces the first: the server stops the first run, and only the new one counts.
After the window closes you cannot start a new competition run; you can only `goalcup continue`
the one you have. To replace a run on the same machine with the same harness, exit the first
run's harness before you start again: a second `goalcup start` of that harness is refused while
the first is still recording.

## 6. Leaving the machine

The run lasts a week. For all of it:

- **Keep the machine on mains power.** On battery a Mac can still sleep, through its lid or its
  power settings.
- **Keep a laptop's lid open.** A Mac sleeps when its lid closes, whatever the tracer does.
- **Keep it online.** Recording does not need the network, but uploads and the server's
  heartbeat do. The tracer retries for as long as the run lasts.
- **WSL2:** the tracer cannot keep Windows awake. In Windows Settings, System, Power, set sleep to
  Never for the week of your run.
- **Keep the terminal window open.** Closing it stops the agent. The tracer still seals that part
  of the run and uploads it, and the run stays open: restart the agent with `goalcup continue`
  (section 8).
- **Over SSH, start the run inside `tmux`** (or `screen`), so a dropped connection does not close
  the terminal.

The server emails you about your run: when it starts; when it has heard nothing from the tracer
for 20 minutes; when your agent has made no model calls for 3 hours, and again at 6 hours; when
the recording has a problem; and when the run has ended and its commit is recorded.

## 7. Checking on the run: `goalcup status`

```sh
goalcup status
```

It shows the newest run started on this machine. It lists anything wrong first, one `PROBLEM:`
line each, then the recording's state, health and storage. A run with nothing wrong has no
`PROBLEM:` line.

| The line says | What it means | What to do |
|---|---|---|
| `run state not updated for …: the tracer may have stopped` | The `goalcup start` process is not running: the machine rebooted or slept, or the process was killed. | If the line ends `run goalcup continue`, do that. |
| `recording has gaps` or `recording failed` | Part of the record is missing. The server is told and the run is flagged for review. | Leave the run going. |
| `last upload … ago: …` | Uploads have been failing for half an hour or more. Nothing recorded is lost: it waits on the disk. | Check the network. After the run, `goalcup upload <entry>` sends what is left. |
| `the last … heartbeats to the server failed: …` | The server has not heard from the run for about a quarter of an hour. | Check the network. |
| `disk: … free beyond the reservation` | Less than 5 GiB is free beyond the run's reservation. | Free some disk. If the disk fills while uploads are failing, the tracer stops the agent and the run stays open: free space, then `goalcup continue`. |
| `the machine may sleep: …` | The tracer could not keep the machine awake. Under WSL2 this line is always there, because Windows decides. | Plug the machine in, or turn sleep off yourself (WSL2: the Windows power setting above). |
| `the agent's process exited (…): run goalcup continue` | The agent ended before the deadline: it crashed, you closed the window, or it quit. | Run `goalcup continue`. |
| `the agent's process exited (…); the run is over` | The agent left on its own, and the run's time has passed since. | Nothing. |
| `the agent's process exited (…); this run cannot be continued` | The agent left, and this run cannot be restarted: it is not on the server, its harness runs headless, or this machine has no credential for it. | Nothing to restart. |

When the tracer stops the agent itself, at the deadline or because a newer run replaced this one,
`status` shows no line about it. An agent that exited normally gets a plain line, not a
`PROBLEM:`: `the agent finished; the run is over` or `the agent finished; this run cannot be
continued`.

## 8. Restarting the agent: `goalcup continue`

```sh
goalcup continue
```

When the agent stops before the deadline (a crash, a reboot, a closed window), this starts it
again in the same run: same deadline, same clone, the harness resuming its last session. Nothing
is approved again. Run it from the same kind of folder as `goalcup start` (section 5), in a
terminal you will leave open.

A run can be continued only on the machine that started it. The tracer never restarts the agent
on its own.

## 9. The deadline, and after it

Your run ends 168 hours after its official start, on the server's clock. The stop is hard: the
agent is stopped in whatever state it is in. There is no grace period.

- At the deadline the server tells the tracer to stop. The tracer stops the agent and the
  recording, seals the record, tells the server which commit your clone had, and uploads what is
  left. It keeps trying for up to one hour. If that is not enough it says so, and
  `goalcup upload <entry>` finishes the job later. Run `goalcup entries` to see the entry ids.
- The server records the latest commit in your GitHub repository at the deadline. That commit is
  what is judged, so your agent must push its work before then. The tracer never pushes for it.
- After the run you hand in a demo video and answer a questionnaire. They are due either 24 hours
  after the run ends or on a fixed date around 15 November: **[owner to confirm]**.
- Without a network at the deadline, the tracer stops the run once 168 hours have passed on the
  machine's own clock, which keeps counting while the machine sleeps.
- 24 hours after the end, the server closes the run. A run whose last upload has not arrived by
  then is closed as incomplete, and it is judged on what did arrive.

## 10. The "keep going" rule

You type your prompt as the first message. After it, the only message you may type to the agent
is "keep going". Use it when the agent has stopped and you want it to carry on.

Every message you type into the harness is recorded. Any other message (an answer, a hint, an
approval, a correction) is seen at review, and it may remove the run at review.
The tracer itself never types to the agent, never says "keep going" for you, and never restarts
it by itself.

An agent that stops early and stays idle until the deadline is allowed. The idle time is in the
record.

Your agent may start subagents of its own harness. It must not start a different harness (a Codex
agent starting Claude Code, say). The tracer cannot record the second harness, and the run is
disqualified.

Your prompt becomes public at the latest at the end of the event. Until then it stays private.

## 11. What is recorded

The tracer records your run, and only your run:

- everything the harness sends to its model provider and everything it gets back;
- the harness's own session files written during the run, including every message you type;
- the commit in your clone at the end, and a digest of any changes not committed;
- the harness build that ran;
- facts about the machine: the operating system and its version, the architecture, the number of
  CPUs, the memory, the disk size and free space, whether it is WSL2 or a container, and whether
  it is on mains power.

It records no host name, no user name and no serial number. It does not look at your other
programs, your browser, other terminals or your files outside the run. Passwords, keys and tokens
are removed on your machine before anything is written. The server also keeps the IP address and
country of each connection from the tracer; they are stored on the server only, never in the
record, and never made public.

Raw records stay private and are never published.

## 12. Test runs (organizers only)

`goalcup start --test` opens an organizer test run. It runs for the full length, like a
competition run, but only organizer accounts can open one, and all test runs are deleted from
the server together afterwards. It is not for participants: use `--practice` to try things out.

## Help

Write to goalcup@agentcommunity.org. Say which command you ran, and paste the `goalcup status`
output and the entry id from `goalcup entries`. Never paste your GitHub token.
