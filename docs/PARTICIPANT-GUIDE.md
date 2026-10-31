# Taking part in the /goal Cup: the participant's guide

You give one coding agent one prompt, then leave it alone for 168 hours. Goal Cup Proxy
(`goalcup`) sits between your agent and the internet, records what the agent does and uploads the
record. This page takes you from install to the end of your run. The rules are in
[RULES.md](RULES.md), and what the record holds is in [RECORD-FORMAT.md](RECORD-FORMAT.md).

> **Build state.** `seal`, `upload`, `status` and `time-left` are built. `continue` and `recover`
> print "not built yet". `start` runs its checks, then prints "start is not built yet"; no approval
> is asked and no run is opened. Sections 5 to 8 describe the 1.0 release and cannot be followed end
> to end yet. This note goes in the final pass.

## What goalcup does for you

- It writes down your agent's model requests and session files for the whole run, subagents too,
  and uploads them as it goes, so a dead computer loses almost nothing.
- It never slows your agent. If something cannot be recorded, it notes a gap and goes on.
- It hides secrets before anything is saved, and says what it could not catch
  ([section 9](#9-what-is-recorded-and-what-is-not)).
- It tells you what it wrote and what it lost, on screen, during the run and at the end.

It has eight commands: `setup`, `start`, `continue`, `recover`, `status`, `seal`, `upload` and
`time-left`. `goalcup --help` lists them.

## 1. What you need

- A Mac or a Linux machine. On Windows use WSL2 (Ubuntu inside Windows) and do everything inside it.
  WSL1 is refused, and so is a project folder on a Windows drive.
- `git`, and a GitHub account for the real run.
- A harness from [the harness list](harnesses/README.md), and the account or API key it uses.
- An Agent Community account signed up at [agentcommunity.org/goalcup](https://agentcommunity.org/goalcup),
  where you hand in your demo video and questionnaire after the run.

## 2. Install

```sh
curl -fsSL https://github.com/agentcommunity/goal-cup-proxy/releases/latest/download/install.sh | sh
goalcup --version
```

It installs the latest release into `~/.local/bin`, with no `sudo`. If it says that folder is not on
your `PATH`, run the line it prints. Update later by running the same command, with no run open:
the installer refuses while a run is recording.

If a firewall such as Little Snitch asks about `goalcup`, allow it. It asks once for each program,
and again after each update.

`goalcup` keeps its files in `~/Library/Application Support/goalcup` on a Mac, `~/.config/goalcup`
on Linux and WSL2, or the folder `GOALCUP_HOME` names: runs in `runs/`, each harness's home in
`homes/`. Every path it prints is in quotes, because a Mac path holds a space.

## 3. Set up

```sh
goalcup setup
```

It starts no run and needs no GitHub. Run it again whenever it names something to fix. It ends with
`Ready.` and the two commands to use next.

1. **Harness.** Pick the harness you will use, or pass `--harness <id>`; the ids are in
   [the harness list](harnesses/README.md). Use the id wherever this page says `<id>`. It prints the
   harness name, version and path. A harness that is not installed gets "Install it, then run
   goalcup setup again."
2. **Sign in.** Press Enter and the harness opens in a home that Goal Cup keeps for it, never in your
   own `~/.claude`, `~/.codex` or the like. Sign in there, or paste your API key when it asks, then
   quit the harness. The sign-in is the harness's own file in that home; the project folder's
   `.env.local` is not for it. To use a gateway, set it up inside that home as you would without
   Goal Cup: nothing else is needed, and the gateway's host is recorded like any model service.
3. **Settings.** Setup always switches the harness's auto-update off in its home where the harness
   has a setting for it (for example Codex, Hermes, Gemini CLI and Qwen Code). Then it asks one
   question: whether to set the harness to never stop and ask for approval. Say yes: nobody may
   answer a prompt during the run, and a harness that waits for you waits until the deadline. A line
   then says what it set and where. With `--yes` the question is skipped and the answer is yes, and
   setup asks and opens nothing else. If you say no, set it yourself
   ([section 4](#4-your-harness-the-list-and-the-tiers)).
4. **This computer.** One warning line each, none of which stops setup: the computer may sleep
   (keep it on mains power, lid open; under WSL2 set Windows sleep to Never), the server cannot be
   reached, `git` is missing, or the WSL problem above. Then the paragraph of
   [section 9](#9-what-is-recorded-and-what-is-not).

A model host `goalcup` does not know, such as a gateway of your own:

```sh
goalcup setup --record-host llm.example.com
```

It adds the host to `record-hosts.txt` in the `goalcup` folder (the line it prints gives the path; edit
the file to remove one). From the next part on, that host is decrypted and recorded; during a run,
`goalcup seal` then `goalcup continue` starts that part now. It refuses, changing nothing: an IP
address; a name of this computer; `goalcup`'s own server; a name that is not a host name; a pattern
wider than one leading `*.`; and a sign-in host, which `goalcup` never decrypts.

## 4. Your harness: the list and the tiers

[The harness list](harnesses/README.md) has one row for each harness: how it signs in, how it
reaches its models, where it keeps its sessions, and the one thing to know before you start. Its
tier column is written from the test pad's results. A tier does not decide whether you may take
part. The three tiers:

- **Verified on a version.** The test pad ran this harness at that version with a direct key, and
  through a gateway where it has one and in the "cannot read" case, and every check passed on `goalcup`'s own record: no unexplained
  gap, no missed model request, every subagent recorded, no secret found, the server's copy whole,
  usage readable, and the counts on screen true.
- **Not verified.** `goalcup` records what reaches it, the same way, but the pad has not passed on
  this harness yet. The list says why (for example, no key). `goalcup status` tells you if model
  requests seem to be missing.
- **Not supported.** The harness ignores the proxy or refuses its certificate. Its session files are
  recorded and, if it refused the certificate, the connection as a tunnel; its model requests are not.

The start screen names your harness, its version and its tier. The notes below hold for every
harness; a fact about one harness is in its row or sheet in `docs/harnesses/`.

**Run without permission prompts.** A prompt waits for an answer, answering is typing, and you may
type only "keep going". On the test day a Codex run spent 8.7 of its 10 minutes at an approval
prompt. Setup writes the no-prompt values into the harness's home when you say yes
([section 3](#3-set-up)); check in the practice run that the agent commits without asking. The agent
then changes anything your user can, so use a machine or container you let it change. Run the harness
as a normal user: Claude Code refuses to skip prompts as root.

**Let the agent commit and push.** What is judged is the last push to your repository's default
branch before the deadline, and `goalcup` never pushes for the agent. Say in your prompt that it
commits and pushes often, and let it write `.git` (a sandbox can forbid that; the values setup
writes allow it). The push block ([section 6](#6-your-run)) gives it the repository and the token.

**Turn and step limits.** Some harnesses stop after a number of steps and wait at their prompt
(Hermes after 90 by default, `agent.max_turns`). Raise the limit in the harness's home if it has
one. An agent that stops early and stays idle is allowed; your prompt decides how long it has work.

**Sign-in lifetime.** Use a login or key you can revoke, and do not sign in again during the run; if
the harness asks you to, write to support first ([RULES](RULES.md#one-prompt-then-only-keep-going)).
An API key avoids a refresh that goes to a host `goalcup` does not record. Never copy a harness's
login file into another home: a refresh in the copy can invalidate the original.

**Subagents and their models.** Your agent may start subagents of its own harness, never of a
different one ([RULES](RULES.md#one-prompt-then-only-keep-going)). A subagent can use another model
than the main agent, and your account pays for both. Try them in a practice run, then read your
provider's usage page.

**Tools on PATH.** The harness gets the `PATH` of the terminal you run `goalcup start` in, and its
own `HOME`, so your `~/.zshrc` and `~/.profile` are not read. Install what the agent needs (for
example `rg`) first. In a practice run, ask the agent to run `goalcup time-left` to see it works.

**First-run screens.** Before your prompt a harness may show its own screens (a sign-in, a
trust-this-folder question, a wizard). Answering them is allowed; it is not a message to the agent.
Some come back at each start. Wait for the chat input before you paste your first message: an open
screen can take the paste, and it is lost.

**Auto-update.** `goalcup` turns the harness's auto-update off by an environment switch where the
harness has one; where it has a setting instead, setup writes it into the harness's home. An update
matters only at the next `goalcup continue`, and the record names the harness version at every start.

## 5. A practice run

Make an empty folder and start there. `Ready.` printed the command; it looks like this:

```sh
d=$(mktemp -d) && cd "$d" && goalcup start --practice
```

Approve the run in the browser and return to the terminal at once. The harness opens in that
folder. Type any prompt. After about ten minutes the server stops the run, and `goalcup` seals and
uploads it. A practice run needs no repository and no token, so the agent has nowhere to push. It
never counts. Do as many as you like.

Check that the agent works without asking you anything, that `goalcup status` shows "Written so
far" model requests, and that the end screen ([section 8](#8-the-end)) reads right. If `status` says
no model request was written, fix the setup now.

## 6. Your run

Start inside the 24-hour start window (7 Nov 2026, 00:00 to 8 Nov, 00:00 UTC). Steps 1 to 4 can be
done before it opens. A real run needs its own new repository and token. Do the steps in this order:
a token for one repository needs the repository to exist first.

1. **Create the repository.** On [github.com/new](https://github.com/new): public, and empty. Add no
   README, no `.gitignore`, no licence. A repository with any commit is refused, and the server
   checks again when the run opens.
2. **Make a token for it.** On GitHub, Settings, Developer settings, Fine-grained personal access
   tokens, Generate new token:
   - Repository access: **only select repositories**, and pick that one repository.
   - Repository permissions, **Contents: Read and write**. Read-only fails the push with a 403.
   - Expiration: **30 days**.

   Copy it (it starts `github_pat_`). The agent pushes with this token, and it reaches that one
   repository only. The agent still runs as you and can read what your programs can read, so keep no
   other secrets in the project folder.

   **Your SSH key is not covered.** The agent is you, and `ssh` reads `~/.ssh` whatever `HOME` says;
   in a test the agent used the tester's key within two minutes. Give your key a passphrase, use an
   SSH agent that asks before each use, or run in a container
   ([results](research/results/2026-10-26-mac-retest.md)).
3. **Make the project folder.** A new folder that holds only a `.gitignore` and a `.env.local` (a
   macOS `.DS_Store` is allowed). First the folder and the `.gitignore`:

   ```sh
   mkdir -p ~/code/my-cup-run && cd ~/code/my-cup-run
   printf '.env*\n' > .gitignore
   ```

   Copy the token on GitHub and copy nothing else until the next two commands are done. The first
   writes `.env.local` private and keeps the token out of your shell history. The second must
   print `1`:

   ```sh
   (umask 077; printf 'GH_TOKEN=%s\n' "$(pbpaste)" > .env.local)
   grep -c '^GH_TOKEN=github_pat_[A-Za-z0-9_]*$' .env.local
   ```

   `pbpaste` is the Mac clipboard; on Linux use `xclip -o -selection clipboard` (or `wl-paste`). Under
   WSL2 the Windows clipboard adds a carriage return, which is refused, so use
   `"$(powershell.exe Get-Clipboard | tr -d '\r')"` in its place. The line must be exactly
   `GH_TOKEN=<token>`: no `export`, no quotes, no spaces. Have it in the file before you start: the
   recorder hides that exact value everywhere, and a token added later is covered by its shape only.
   Keep the folder out of any Git repository, off a Windows drive under WSL2, and out of Desktop,
   Documents and any synced folder (iCloud Drive, Dropbox, OneDrive, Google Drive): the service would
   upload `.env.local`.
4. **Write your prompt** in a file outside the project folder, for example `~/goal-prompt.txt`. Ask
   in it for the MIT License and a `GOALCUP.md` (what the project is, how to run it, what works).
5. **Start.** In the project folder:

   ```sh
   goalcup start --repo <user>/<name> --prompt ~/goal-prompt.txt
   ```

   It checks the folder and that the repository is public and empty. A wrong folder costs you
   nothing: it stops, lists every problem with its fix and ends with "Nothing was created."
   ([Appendix A](#appendix-a-start-refusals)). Then it opens the approval page. Check that the page
   shows the code printed in the terminal, approve, and **return to the terminal now**. If you
   already have a competition run open, approving replaces it. Your 168 hours start when the server
   opens the run.

The harness opens in the project folder. `goalcup` builds your **first message**: the push block,
a blank line, then your prompt. It is on your clipboard when a tool for that exists (`pbcopy` on a
Mac, `wl-copy` or `xclip` on Linux, `clip.exe` under WSL2) and always in `first-message.txt` in the
run's folder; the screen gives the path. Answer the harness's own first screens, then **paste it
once as your first message and press Enter**. Without `--prompt` the screen prints the push block and
says your first message must start with it. The block looks like this; yours differs, and the
`credential` line is one line:

```
  Push to https://github.com/<user>/<name>
  The token is GH_TOKEN in .env.local in this folder. Never print it, and never write it into a file, a git setting or a remote address.
  After git init, run:
    git config user.name "<user>"
    git config user.email "<id>+<user>@users.noreply.github.com"
    git config credential.https://github.com.helper '!f() { echo username=x-access-token; echo "password=$(sed -n s/^GH_TOKEN=//p "/full/path/to/my-cup-run/.env.local")"; }; f'
    git remote add origin https://github.com/<user>/<name>.git
```

The block is how the agent learns the repository and how to push; `goalcup` writes nothing into
your project folder, and the agent runs `git init` and those lines itself. Never paste the token into
your prompt. After the first message you may type only "keep going", when the agent has stopped
([RULES](RULES.md#one-prompt-then-only-keep-going)).

## 7. During the run

Leave the terminal open. In another one:

```sh
goalcup status
```

It reads the run's files and never asks the recorder, so it works while a run records, after it
ends and after a crash. A running block looks like this:

```
Your competition run with Claude Code: recording.
  Ends: Sat 14 Nov, 10:12 CET (09:12 UTC), 95 h 10 min left.
  Claude Code: running.
  Recording: on.
  Written so far: 905 model requests (api.anthropic.com 812, openrouter.ai 93); 896 whole, 9 with missing parts.
  Session files: 43, 48 MB.
  Off: 1 time, 2 min 30 s in total.
  Upload: the server holds everything up to 14:02 (1 min ago).
  Parts: 2. Part 1 sealed. Part 2 recording.
  Problems: none.
```

- **Ends** is the server's deadline in your time zone and in UTC, with the time left.
- **Written so far** counts model requests written to the record, by host; "whole" and "with missing
  parts" appear when some requests have a hole.
- **Session files** is how many of the harness's files are in the record, and their size.
- **Off** is the time recording was off, as a count and a total. Each is a gap in the record.
- **Upload** says what the server holds ([section 8](#8-the-end)). Uploads go every 5 minutes, and at
  once when 16 MiB wait. A failed send waits 2 seconds, doubling up to 2 minutes, and then tries again.
- **Parts**: one `start` or `continue` is one part. **Problems** has one line for each cause.

Before any run exists, `status` says "There is no run on this computer yet. To start one, run:
goalcup start". `goalcup status --json` prints the same facts as JSON.

**The recording line** is one of these.

| It says | What it means and what to do |
|---|---|
| `Recording: on.` | Nothing to do. |
| `Recording is restarting (since 15:43:15).` | The recorder stopped and `goalcup` is starting it again, which takes about a second. Nothing to do. |
| `Recording is off since ... Your agent keeps working; this time will show as a gap.` | The restart took over 30 seconds. Nothing to do. |
| `Recording is not answering since ...`, `Recording is behind: nothing written for N s.` | The recorder is slow. Your agent is not slowed. Wait. |
| `Recording is losing data now: ...` | Some connections or files have a gap. Your agent is not slowed. The end screen says how much. |
| `Recording: finished at 16:54. Nothing is recorded now.` | The part is sealed. |
| `PROBLEM: Recording is off since ...: goalcup stopped, so your agent cannot reach the model service. Your agent is still running. Run: goalcup recover` | Both `goalcup` and its recorder stopped. In another terminal run `goalcup recover`. It brings the recorder back, never touches your agent, and the time between is recorded as a gap. |
| `PROBLEM: Recording is off since ...: goalcup is not running. To go on, run: goalcup continue <run>` | The agent has stopped too. Run the command. |
| `PROBLEM: Recording is off since ...: it stopped right after each restart, so goalcup lets your agent's traffic pass unrecorded for now.` | The recorder keeps failing, most often because a write to disk failed. Your agent keeps working and `goalcup` tries again, at most 5 minutes apart. Free space on the disk. The server is told. |

`goalcup recover` prints that recording is on again, for how long it was off, and that the agent was
not restarted. It refuses with one line, and changes nothing, if the part is sealed, the run is over,
`goalcup` still runs for the part (it restarts recording itself), or the agent is no longer running.
Run it in another terminal, not inside the agent.

**Problem lines.** A `PROBLEM:` line appears only where you must act or report; each cause is one
line.

| It says | Do this |
|---|---|
| `No model request was written yet. If your agent is working, it may not use the recording proxy.` (5 minutes after the launch) | Type your prompt if you have not. If the agent is answering, write to support. |
| `No model request was written for 10 minutes, but <harness>'s session files grew. ...` | The agent may not be using the proxy, or uses a model service `goalcup` does not know. Its session files are still recorded. See `goalcup setup --record-host`. |
| `<host> streamed data for N minutes and was not recorded. If it is a model service, run: goalcup setup --record-host <host>. ...` (also "received N MB from your agent") | Run the command, then `goalcup seal` and `goalcup continue`; the host is recorded from that part on. If it is not a model service, ignore the line. |
| `Not recorded: <host> refused the recorder (it checks its own certificates). Its traffic passed through unread.` (or: its certificate is not one `goalcup` trusts, a company proxy) | Nothing to fix. The traffic passes unread and the end screen names it. |
| `goalcup has not updated this run since ... The computer may have slept or restarted. To go on, run: goalcup continue <run>` | The computer slept or restarted. Run the command. |
| `<harness> has exited (exit code 0). The run is still open until ... To go on, run: goalcup continue <run>` | The agent ended before the deadline. Run the command. |
| `goalcup hid N MB it could not check for secrets in time. Those bytes are stars in the record.` | Nothing to do. Hiding too much is the safe side. |

Lines without `PROBLEM:` (no session file found yet, a session folder on another disk, recording of
one connection stopped, a model on this computer, many connections open) need nothing from you.

**If the agent stops before the deadline** (a crash, a reboot, a closed window), run `goalcup continue`
in a terminal you will leave open. It needs no new approval, keeps the same deadline and the same
folder, and is a new part of the same run. If the earlier parts wrote model requests or session
files, it resumes the harness's last conversation; if that fails at once it opens a new one. Either
way type only "keep going". Do not paste the first message again: it is shown again only if no model
request was recorded, which means your prompt may never have been sent. A new conversation can still
push if the agent's `git config` and `git remote add` lines are saved in the repository's
`.git/config`. Do not delete or move the project folder. `goalcup seal` stops the agent and ends the
current part (it asks first; `--yes` skips that). The run stays open and `continue` goes on.

**Time left.** `goalcup time-left` prints when the run stops, from the server's last word; the agent
may run it too. After 11 minutes without word it says the time is unknown right now.

**Leaving the machine.** Keep it on mains power, awake, online and the lid open; over SSH, start
inside `tmux`. Sleep suspends the agent too and loses nothing, but the clock runs on.

**Change nothing the agent can read:** files, issues, messages, documents, websites. See
[RULES, During the run](RULES.md#during-the-run).

## 8. The end

At 168 hours, on the server's clock, `goalcup` stops the agent, seals the record and uploads the rest,
with no grace period. What is judged is the last push to the repository's default branch that GitHub
received before the deadline. Later pushes are flagged, and so is a force-push. The end screen reads:

```
Codex has exited (exit code 0).

What this part recorded (on this computer):
  905 model requests written: api.openai.com 812, chatgpt.com 93.
  896 of them written whole, 9 with missing parts. 1.5 MB was not recorded.
  43 session files, 48 MB.
  Recording was off 2 times, 2 min 30 s in total.
  Not recorded, passed through unread: registry.npmjs.org (105 connections, 80 MB), ab.chatgpt.com (11, 24 MB), and 2 more hosts.
Upload: done. The server holds your whole record (95 MB). Removed here: 3 record files the server holds.
Details for support are in "/home/you/.config/goalcup/runs/<run>/<part>/goalcup.log"
```

- The first lines say the agent has exited and, if the run is still open, how to go on with
  `goalcup continue`. A part that ended without `goalcup` (the computer stopped) says so.
- **Model requests written**, by host. With none: "No model request was written."
- **Nothing was lost.** is the second line only when there was no gap, no off time, no tunnel to a
  model host, no byte hidden for lack of time, no cut file and no byte after the stop. Otherwise each
  loss has its line with numbers: requests with missing parts, bytes not recorded, gaps of unknown
  size, recording off, files still growing at the end, bytes that programs the agent started sent
  after the part ended, and programs still running at the end (which `goalcup` did not stop).
- **Passed through unread** lists the hosts the agent connected to that `goalcup` does not decrypt,
  by size, five at most and the rest as "and N more hosts". A host on the model list that was passed
  through is always named on its own line with the reason in words
  ([Appendix B](#appendix-b-gap-reasons-in-plain-words)).
- **Upload** is one line. "The server holds your whole record" is the end. Other lines: the server
  holds everything up to a time; it is not reaching the server (nothing is lost, the rest goes up by
  itself); the server refuses the run or the copies differ (write to support).

If bytes are still waiting, the line reads "Upload: ... still waits on this computer. Run goalcup
upload before <time>, when you are online. Nothing is lost yet." Run `goalcup upload`; it is safe
to run again. The server takes uploads until 24 hours after the run ends, because the run's
credential ends then. After that time the line says the server closed the run, and what is still on
your computer cannot be sent; the run is judged on what arrived. In the first hour after the seal
`goalcup` keeps sending by itself.

Record files the server already holds are removed from your computer, so the folder is small. Set
`GOALCUP_KEEP_PARTS` to keep them. `goalcup.log` stays; it is not part of the record and is never
uploaded.

Then:

- **Delete the token** on GitHub (Settings, Developer settings, Fine-grained personal access tokens,
  Delete), and delete `.env.local`. Log the harness out, or rotate its key.
- Hand in your demo video and questionnaire on
  [agentcommunity.org/goalcup](https://agentcommunity.org/goalcup) within 24 hours.

## 9. What is recorded, and what is not

Goal Cup Proxy hides known secret shapes in memory, before anything is saved: API keys with a known
prefix, login headers, cookies, passwords in URLs, private keys, login tokens, values named like
`api_key`, `secret`, `token` or `password`, and your project's GitHub token. A hidden value is
replaced by stars of the same length. It cannot catch every secret. These can still be recorded: a
key the model repeats back in its reply, a key written right after a letter, a digit or `_`, a short
password, and a value with no known shape (for example a line of `cat .env` whose name is not one of
those). For a harness that keeps a database, the database is read whole and can hold the same. Your
email address and your plan and credit balance are recorded. Raw records stay private and are
cleaned before research use. Use keys you can revoke, and rotate every key after the run.

Also not recorded, or recorded less:

- **Tunnels.** A host that is not on the model list is passed through unread: only its name, bytes
  and times are written.
- **Vendor backends.** Where the harness sends its traffic to its vendor and the vendor calls the
  model, the traffic to the vendor is recorded, not the vendor's own model calls.
- **Harnesses that ignore the proxy** are "not supported": session files and connections only.
- **Wire only.** If a harness starts another harness as a subagent, that one's model requests are
  recorded and its session files are not.
- **A model server on this computer** (Ollama, LM Studio) is reached past the proxy and not
  recorded. The status block names it.
- **HTTP/3, QUIC and ECH** never reach a proxy.
- **A network that re-signs TLS** and needs its own proxy is not supported.
- **A token used as the user name of a URL with no password** may be recorded.
- **SQLite harnesses** keep their data in a database; it is copied once, at the seal.
- **On Linux**, memory `goalcup` holds can reach swap that is not encrypted. It is the one place raw
  bytes can leave memory.

Core dumps are switched off for `goalcup`.

## 10. Help

Write to goalcup@agentcommunity.org. Say which command you ran, and paste the `goalcup status`
output with the run id it shows. The end screen names the `goalcup.log` of the part; it holds only
fixed words, numbers and host names, never a header, body, token or key. Never paste your GitHub token.

## Appendix A: start refusals

Every folder problem is listed in one refusal, then "Nothing was created." Fix each one and run
`start` again.

| It says | Do this |
|---|---|
| `It holds "<names>". Start in a new folder that holds only .gitignore, .DS_Store and .env files.` | Make a new folder with only those. |
| `.gitignore does not cover "<name>", so a push could publish it. Add the line .env* to .gitignore.` | Add the line `.env*`. |
| `".env.local" needs exactly one plain GH_TOKEN=<your token> line (no quotes, no export, no spaces, no Windows CRLF line ending).` | Fix the line. A real run needs it; a practice run needs no token. |
| `It is inside the Git work tree "<path>". Start in a folder outside any Git repository.` | Use another folder. |
| `It and goalcup's folder "<path>" hold each other or sit side by side.` | Use a folder elsewhere, or set `GOALCUP_HOME` to a folder away from it. |
| `The prompt file "<path>" is inside the project folder. Move it out of the folder.` | Move the file out. |
| `This is WSL1, which cannot run goalcup.` or `... is on a Windows drive (...)` | Convert to WSL2 (`wsl --set-version <distro> 2`), and use a folder in the Linux home. |
| `--connector and --hosts are for practice and test runs. A competition run uses the connectors built into goalcup.` | Remove the flags. |
| `goalcup cannot tell where to keep your runs: HOME is not set to a full path.` | Set `HOME`, or `GOALCUP_HOME`, to a full path outside your project. |
| `"<file>" sets <key>, which routes around the recorder. Remove it, then run this again.` | Remove that setting from the file named. The line names the file and key, never the value. |
| `<harness> --version does not show <version>, the version goalcup works with.` | Install that version. |
| `a run uses "<home>" now. Run this again once it has ended.` | One run at a time can use a harness home. |

These two are warnings, not refusals: the folder looks synced (a sync client may upload `.env.local`
and your token), and a folder is on NFS or SMB, where file locks may not hold. A synced folder is
found by name (Dropbox, OneDrive, Google Drive, iCloud), so keep out of Desktop and Documents anyway.

The server checks, when it opens the run, that the repository is public and empty and that the run
is inside the start window. Its refusal text is shown after "goalcup: ". An approval can fail
(denied, expired or interrupted); run the command again.

## Appendix B: gap reasons in plain words

Every loss in the record is one gap with a reason. The screens use these words.

| Reason | In plain words |
|---|---|
| `memory_guard` | under heavy load |
| `recorder_down` | recording was off |
| `unsupported_encoding`, `decode_failed` | a reply in a form goalcup cannot read |
| `source_changed` | a session file was replaced or cut short |
| `tls_refused`, `pinned` | the program refused the recorder |
| `cut_at_seal` | still arriving when the part ended |
| `h2_unsupported` | a connection goalcup could not follow |
| `mask_failed` | goalcup could not check it for secrets, so it dropped it |
| `upstream_untrusted` | the model service's certificate was not one goalcup trusts |
| `protocol_other` | a connection in a form goalcup does not read |
| `after_stop` | sent after the agent ended |

A reason this release has no words for shows as "other (n)". A full disk is not a gap reason: the
recorder fails, recording goes off and the time is counted in "Off".
