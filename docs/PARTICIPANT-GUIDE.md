# Taking part in the /goal Cup: the participant's guide

You give one coding agent one prompt, then leave it alone for 168 hours. Goal Cup Proxy
(`goalcup`) records what the agent does and uploads the record. This page takes you from install
to the end of your run. The rules are in [RULES.md](RULES.md). The long version of this guide, from
before 0.7, is in [the archive](archive/PARTICIPANT-GUIDE-0.6.md).

You can hand the setup to your own agent: tell it "set me up for Goal Cup" and point it at the
[goal-cup-setup skill](../skills/goal-cup-setup/SKILL.md). It does what a command can do and stops
at each step that needs you.

## What you need

- A Mac or a Linux machine. On Windows, use WSL2 (Ubuntu inside Windows) and do everything inside it.
- `git`, and a GitHub account.
- A chat-mode harness (Claude Code, Codex CLI, Hermes or Amp) and the account or API key it uses.
  Per-harness notes: [HARNESS-SETUP](HARNESS-SETUP.md). For Hermes, the latest release from
  Hermes's own installer is fine with an API-key service such as DeepSeek or OpenRouter
  ([limits](HARNESS-SETUP.md#hermes)).
- An Agent Community account signed up for the Goal Cup at
  [agentcommunity.org/goalcup](https://agentcommunity.org/goalcup). You hand in your demo video
  and questionnaire there after the run.

## 1. Install

```sh
curl -fsSL https://github.com/agentcommunity/goal-cup-proxy/releases/latest/download/install.sh | sh
goalcup --version
```

It installs the latest release into `~/.local/bin`, with no `sudo`. If it says that folder is not
on your `PATH`, run the `export` line it prints. Update later by running the same line, but only
with no run open: the installer refuses while a run is recording. If a start is refused right
after an update, wait a minute (the old recorder exits when idle) and start again.

## 2. Set up: `goalcup setup`

```sh
goalcup setup
```

Four steps, one line each. Ctrl-C stops it; run it again to go on. It starts no run and needs no
GitHub.

1. **This computer.** Your browser opens an approval page. Check that it shows the same code as the
   terminal, approve, and **return to the terminal now**. You do this once per computer.
2. **Harness.** Pick the harness you will use, or pass `--harness <id>`. The id is the connector's
   name: `claude-code-cli`, `codex-cli`, `hermes-cli` or `amp-cli`. Use it wherever this page
   says `<id>`.
3. **Sign in.** Press Enter and the harness opens in a home Goal Cup keeps for it (never your own
   `~/.claude` or `~/.codex`). Sign in, or paste your API key when it asks. That is the harness's
   own file; its access token is forwarded unread and never written
   ([README](../README.md#the-access-token)). Then set the harness to **never stop and ask you for
   permission**, because nobody may answer during the run
   ([how, per harness](HARNESS-SETUP.md#1-run-without-permission-prompts)). Exit it normally.
4. **This machine.** Mains power, free disk, network. Keep the machine awake yourself where it
   says so (always under WSL2). If an SSH key in your `~/.ssh` has no passphrase, a `[warn]` line
   names it: during a run your agent could use it to push to any of your GitHub repositories.
   Add a passphrase (`ssh-keygen -p -f <key>`), use an SSH agent that asks before each use, or
   run in Docker. A softer `[note]` says when your SSH agent holds keys. Neither stops setup, and
   `goalcup doctor` says the same. The check stays on this computer: it connects nowhere and
   asks your SSH agent for its list only, never to use a key.

It ends with `Ready.`, the practice command, and a pointer to the folder step for your real run. If a firewall such as Little Snitch asks about
`goalcup`, allow it, and again after each update.

## 3. A practice run

Make an empty folder and start there. `Ready.` printed this line:

```sh
d=$(mktemp -d ~/goalcup-practice-XXXX) && cd "$d" && goalcup start --harness <id> --practice
```

Each paste makes a new folder in your home, so it is safe to paste twice.

Approve the run in the browser and return to the terminal. The harness opens in that folder. Type
any prompt. After about ten minutes the server stops the run, and `goalcup` seals and uploads it.
A practice run needs no repository and no token, so the agent has nowhere to push. It never counts.
Do as many as you like. It is not the dry run: a practice run lasts ten minutes and needs no
repository; a [dry run](#the-dry-run-optional) is a full 168-hour rehearsal that needs both.
If the harness makes five model requests and writes no session file, `goalcup status` says so
after three minutes. Fix it now.

<a id="the-dry-run-optional"></a>

## 4. A dry run (optional)

From 23 October to 6 November you may start a full rehearsal, with the steps of section 5 in the
same order. It lasts 168 hours and does not count. It needs its own new repository and token; your cup run then
needs another pair. A second dry run replaces the first.

**Started a dry run on 0.6.1 before 0.7 came out?** Seal it, then update. Run `goalcup seal` from
another terminal and wait for `Upload done.`, then run the installer.

The installer that shipped with 0.6.1 does not check for a recording run. The one the command in
section 1 downloads now does. Still seal first.

A dry run that is still recording when the cup start window opens must be sealed the same way
before you start your cup run.

## 5. Your run

Start inside the 24-hour start window (7 Nov 2026, 00:00 to 8 Nov, 00:00 UTC). Steps 1 to 3 can
be done before it opens. For a [dry run](#the-dry-run-optional) the window is open now, so use
this section as it is. Do the four steps in this order: a token for one repository needs the
repository to exist first.

1. **Create the repository.** On [github.com/new](https://github.com/new): public, and empty. Add
   no README, no `.gitignore`, no licence. A repository with any commit is refused, and the server
   checks again when the run opens. Each real run needs its own new repository.
2. **Make a token for it.** On GitHub, Settings, Developer settings, Fine-grained personal access
   tokens, Generate new token:
   - Repository access: **only select repositories**, and pick that one repository.
   - Permissions, Repository permissions, **Contents: Read and write**. Read-only is not enough:
     the push fails with a 403 "Permission denied".
   - Expiration: **30 days**.

   Copy it (it starts `github_pat_`). Your agent pushes with this token. Goal Cup passes the
   agent none of your git login, SSH agent or `gh` settings, so the token is the one push login
   Goal Cup gives it. The token itself reaches this repository only. A real run needs it: `start` refuses
   a folder with no `GH_TOKEN` line. The agent still runs as you and
   can read what your programs can read, so keep no other secrets in the project folder.

   **Your SSH key is not covered.** The token limits itself to one repository, but the agent is
   you, and `ssh` reads your keys from `~/.ssh` whatever `HOME` says. In a test the agent used the
   tester's own SSH key unprompted, within about two minutes; a push to any repository that key
   reaches would have worked (nothing was pushed). If you have an SSH key, give it a passphrase
   or make it ask before each use (for example the 1Password SSH agent with approval), or run in
   a Docker container (`goalcup setup` and `goalcup doctor` warn of a key with no passphrase), as the organizers' Docker cells do ([LOCAL-CAMPAIGN](research/LOCAL-CAMPAIGN.md)).
   See the [26 Oct results](research/results/2026-10-26-mac-retest.md).
3. **Make the project folder.** This is where the agent works. Make a new folder that will hold
   only a `.gitignore` and a `.env.local` (a macOS `.DS_Store` is allowed). First the folder and
   the `.gitignore`:

   ```sh
   mkdir -p ~/code/my-cup-run && cd ~/code/my-cup-run
   printf '.env*\n' > .gitignore
   ```

   Now copy the token on GitHub. It stays on your clipboard: copy nothing else until the next two
   commands are done. The first writes `.env.local` private from the start and keeps the token out
   of your shell history. The second must print `1`:

   ```sh
   (umask 077; printf 'GH_TOKEN=%s\n' "$(pbpaste)" > .env.local)
   grep -c '^GH_TOKEN=github_pat_[A-Za-z0-9_]*$' .env.local
   ```

   If it prints `0`, copy the token again and repeat. `pbpaste` is the Mac clipboard; on Linux use
   `xclip -o -selection clipboard` (or `wl-paste` on Wayland) in its place. Under WSL2 the Windows
   clipboard adds a hidden carriage return, which `goalcup start` refuses, so use this first line
   instead (or write the file in an editor):

   ```sh
   (umask 077; printf 'GH_TOKEN=%s\n' "$(powershell.exe Get-Clipboard | tr -d '\r')" > .env.local)
   ```

   The check also prints `0` for anything after the token, such as a space or a carriage return.
   The line must be exactly `GH_TOKEN=<token>`: no `export`, no quotes, no spaces. Have it in the
   file before you start: the recorder redacts that value everywhere, and a token added later is
   covered by its `github_pat_` pattern only.

   Keys your agent's own product needs may go in `.env.local` too, one `NAME=value` per line, but
   not the harness's sign-in. `goalcup` refuses an `.env` file that `.gitignore` does not cover, so
   the agent's first `git add .` cannot publish the token. Keep the folder out of any git
   repository, and off a Windows drive under WSL2. Do not use Desktop, Documents or any synced
   folder (iCloud Drive, Dropbox, OneDrive, Google Drive): the service would upload `.env.local`.
4. **Start.** In that folder:

   ```sh
   goalcup start --harness <id> --repo <user>/<name>
   ```

   `goalcup` checks the folder and that the repository is public and empty. A wrong folder or
   repository costs you nothing: it stops before the approval and says what to fix
   ([appendix](#appendix-if-goalcup-start-stops)). Then it opens the approval page. Check the
   code, approve, and **return to the terminal now**. If you already have a competition run open,
   approving replaces it. Your 168 hours start when the server opens the run.

The harness then opens in the project folder. It may first show its own screens (a sign-in, a
trust question: answer Yes); answering them is allowed. `goalcup` then prints a block of lines
that starts with "Push to". **Paste that block at the very start of your first prompt**, before
your own words. It is how the agent learns which repository to push to and how; without it the
agent does not know the repository. Do not skip it. `goalcup` prints
the block indented by two spaces; paste it as it is. An example (yours differs; the long
`credential` line is one line, so copy it whole):

```
  Push to https://github.com/<user>/<name>
  The token is GH_TOKEN in .env.local in this folder. Never print it, and never write it into a file, a git setting or a remote address.
  After git init, run:
    git config user.name "<user>"
    git config user.email "<id>+<user>@users.noreply.github.com"
    git config credential.https://github.com.helper '!f() { echo username=x-access-token; echo "password=$(sed -n s/^GH_TOKEN=//p "/full/path/to/my-cup-run/.env.local")"; }; f'
    git remote add origin https://github.com/<user>/<name>.git
```

**The block is never lost.** It is also on your clipboard when `goalcup` could copy it there (the
screen then says "The push block is on your clipboard. Paste it (Cmd-V, or Ctrl-Shift-V on Linux)
at the very start of your first prompt."; it uses `pbcopy` on a Mac, `wl-copy` or `xclip` on
Linux, `clip.exe` under WSL2; otherwise it says to copy the block from the screen), in the file `push-lines.txt` in the run's folder (the screen
gives its path), and in `goalcup status` while the run is open (for a run started before the cup's minimum release,
only on the start screen). Pasting it is the only way the
agent learns them: Goal Cup writes nothing into your project folder.

The agent runs `git init` and those lines, then pushes with the token. Goal Cup makes no commit and
no push for it. Never paste the token into your prompt, and do not ask the agent to force-add `.env.local`.

Your prompt should also ask for the MIT License and a `GOALCUP.md` (what the project is, how to run
it, what works). Write it in a file first, then paste it and press Enter once. After it, you may
type only "keep going", when the agent has stopped. See [RULES](RULES.md#one-prompt-then-only-keep-going).

## 6. Check on it, and restart it

Leave the terminal open. In another one:

```sh
goalcup status
```

It shows the state, time left, upload and any `PROBLEM:` lines. Its last line, `Next:`, says what
to do. [The appendix](#appendix-what-goalcup-status-says) lists the messages.

If the agent stops before the deadline (a crash, a reboot, a closed window), run `goalcup continue`
in a terminal you will leave open. It needs no new approval and keeps the same deadline, in the
same folder. Once the run has recorded model calls, your prompt was sent, and from then on you
type only "keep going" when the agent has stopped: also when the harness opens a new conversation
instead of picking up its last one. Do not paste the push block or your prompt again. The
agent's git setup does not live in the conversation: if the agent ran the push block's `git
config` and `git remote add` lines in the repository, they are saved in its `.git/config`, and an
agent in a new conversation can still push. If it did not, the new conversation has no remote to
push to. Hermes picks
up its last conversation too, once the run has recorded model calls. If no model call was
recorded yet, your prompt may never have been sent: the screen then shows the push block again
and says to send your one prompt, block first. Do not delete or move the project folder. `goalcup seal` stops the agent and ends the
current part. The run stays open, and `continue` goes on with it.

**While it runs, change nothing the agent can read:** files, issues, messages, documents, websites.
See [RULES, During the run](RULES.md#during-the-run).

**Leaving the machine.** Keep it on mains power, awake, online and the lid open. Over SSH, start the
run inside `tmux`. Keep disk free: under 1 GiB free beyond the run's reservation, `goalcup` pauses
the agent until you free space outside the run's folders. Never delete anything the agent made.

## 7. The end

At 168 hours, on the server's clock, `goalcup` stops the agent, seals the record and uploads the
rest, with no grace period. Keep the terminal open until `Upload done.` What is judged is the last
push to the repository's default branch that GitHub received before the deadline. Later pushes are
flagged, and so is a force-push. If something is left unsent, run `goalcup upload`.

Then:

- **Delete the token** on GitHub (Settings, Developer settings, Fine-grained personal access
  tokens, Delete), and delete `.env.local`. Log the harness out, or rotate its key.
- Hand in your demo video and questionnaire on
  [agentcommunity.org/goalcup](https://agentcommunity.org/goalcup) within 24 hours.

## Help

Write to goalcup@agentcommunity.org. Say which command you ran, and paste the `goalcup status`
output with the run id it shows. Never paste your GitHub token.

## Appendix: if `goalcup start` stops

Every row but the outage row stops the start and says "Nothing was created". Fix it and run
`start` again. Most stop before the approval. The folder is checked once more right after you
approve: if it changed meanwhile (an editor added a folder), the start stops there, before the run
is opened, with the same message.

| It says | Do this |
|---|---|
| The folder also holds other files | Make a new folder with only `.gitignore`, `.env` and `.env.*` files (and a macOS `.DS_Store`). |
| No `GH_TOKEN` line (a real run only) | Put `GH_TOKEN=<token>` in `.env.local`. A practice run needs no token. If there is no `.gitignore` either, it also prints the two lines to put in one (`.env` and `.env.*`). |
| `.gitignore` does not cover an `.env` file | Add the line `.env*` (or `.env` and `.env.*`). |
| Write your token as one line, exactly `GH_TOKEN=<token>` | Remove `export`, quotes and spaces, and any second `GH_TOKEN` line in any `.env` file. |
| This folder belongs to run `<id>` | You started a run here and it is not over. `goalcup status` shows it; if the agent has exited, `goalcup continue`. A new run needs a new folder. If the run here is a competition run, the message also asks for a new repository: a new competition run replaces that run. |
| A service syncs this folder | Dropbox, OneDrive, Google Drive or iCloud (Desktop and Documents too, when iCloud syncs them) would upload `.env.local`. Use a folder outside it, for example under `~/code`. |
| The folder is inside a Git repository | Use a folder outside any repository. If it names `GOALCUP_CONFIG_DIR`, goalcup's own folder is inside a repository: set that variable to a folder outside it. |
| The folder is goalcup's own, or next to one | Use a folder somewhere else, for example in your home folder. |
| The repository is not empty (a README counts) | Make a new repository with no README, `.gitignore` or licence. |
| GitHub does not show the repository | It is not created yet, or it is private. Make it public. |
| GitHub could not be asked just now | Nothing to do. The start goes on, and the server checks when it opens the run. A repository with a commit is refused then, after your approval. |
| A `--dev` run never enters the competition | Add `--practice`. |

If the repository belongs to an organization, the agent commits under the organization's private
GitHub address, not yours.

## Appendix: what `goalcup status` says

A run that records, with nothing wrong:

```
Your competition run with Claude Code: recording.
  Ends: Sat 14 Nov, 10:12 CET (09:12 UTC), 95 h 10 min left.
  Claude Code: running.
  Upload: working, sent every 30 minutes (last sent 12 min ago; 2140 records wait for the next send).
  Problems: none.
  This computer: 212.4 GiB of disk free; goalcup uses 38.2 MiB of memory, 4 min of CPU time so far.
Next: nothing to do. The run ends by itself.
```

Records waiting between sends are normal; `up to date` appears only when the server has
acknowledged every record. A `PROBLEM:` line says what to do.

When the agent stops, `goalcup start` ends with what it recorded. `Recording works: N model call(s)
recorded from <host>` means your agent's model calls reached the recording, and names the model
service they went to. If that line is missing and the block says no model calls were recorded,
fix the setup before the next run. Every request to the model service is recorded, whatever its
path, so a model list or a token refresh there is in the trace too. The `sealed hosts (passed
through unread)` line names the other hosts your agent connected to, with the number of connections
and their size. Those hosts are not read, and the model service is never among them. Your
credential headers are masked in the trace as `[masked]`, and credentials in bodies are redacted by
pattern as `<redacted:KIND>`, so a key your model repeats in a reply may not be caught. The trace
also holds the files and paths on your machine that the agent read and wrote.

| The line says | What it means | What to do |
|---|---|---|
| `goalcup has not updated this run for …. The computer may have slept or restarted.` | The `goalcup start` process is not running. | The line ends with the fix, most often `goalcup continue <run id>`. |
| `the recording has gaps` or `the recording failed` | Part of the record is missing. The server is told and the run is flagged for review. | Leave the run going. If it says to write to support, name the run id. |
| `nothing was uploaded for … Nothing is lost; it waits on this computer.` | Uploads have failed for half an hour or more. | Check the network and that no firewall holds `goalcup`. Afterwards `goalcup upload` sends what is left. |
| `the server has not heard from this run for about … min` | The server has heard nothing for about a quarter of an hour. | Check the network. |
| `only … of disk is free. Free some space outside your run's folders, …` | Less than 5 GiB is free beyond the run's reservation. | Free space outside the run's folders. Never delete anything the agent made. |
| `The disk is almost full, so your agent is paused. …` | Under 1 GiB is free, so `goalcup` froze the agent. | Free space. Above 3 GiB free the agent resumes by itself. If it seems stuck, type `keep going`. |
| `Goal Cup has recorded … requests to the model service but no … session file yet. …` | The harness made model calls but wrote no session file `goalcup` could record. A practice run says so after 5 requests and 3 minutes, a real run after 10 and 10. Its setting may be off, or it may be another version. | Keep the agent running. Check the harness version and setup in [HARNESS-SETUP](HARNESS-SETUP.md). |
| `Goal Cup has recorded no model calls in … minutes. …` | Goal Cup has seen no model call from your agent: 3 minutes into a practice run, 10 into a real one. Either you have not typed your prompt yet, or your agent's calls are not going through Goal Cup (for example they come from a background process, or fail the recording certificate; the line says so when they fail). It goes away once a call is recorded. | If you have not typed your prompt, type it. If your agent is answering, exit it and run `goalcup continue`. If it happens again, run `goalcup doctor`. |
| `this computer may go to sleep (…)` | `goalcup` could not keep the machine awake (always so under WSL2). | Mains power, lid open, or turn sleep off yourself. |
| `… has exited (…). Your run is still open until … To go on, run: goalcup continue <run id>` | The agent ended before the deadline. | Run the command it names. |
| `… has exited (…) and the run is over.` or `… and this run cannot be continued.` | The deadline passed, or the run cannot be restarted. | Nothing. |
