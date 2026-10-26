# Plan: clean, simple `goalcup setup`

> **Archived 25 Oct 2026. Partly overtaken by D71.** The four-step `goalcup setup` with no GitHub
> stands. Everything below about the player's own git login (finding the GitHub user, the push
> test, the clone, the five login variables, the practice git isolation, the work folder and its
> keys file) was cut for 0.7: the project folder is the workspace and the agent pushes with the
> player's one-repo token. Read [D71](../DECISIONS.md#d71-the-project-folder-is-the-workspace-the-agent-pushes-with-the-participants-one-repo-token).

Goal: a player goes from zero to a started run in about 5 minutes of their own time.
Principle: goalcup observes. It stores no key and passes no key. The agent pushes with
the player's own git login, the same way the player's laptop already does.

## What the player sees (revised with the owner, 23 Oct)

Setup has 4 steps and no GitHub. GitHub comes only when a real run starts.

```
$ goalcup setup
Goal Cup setup. 4 steps. Ctrl-C stops at any time; run it again to continue.

[1/4] This computer   connected to Goal Cup as you@example.com                 ✓
[2/4] Harness         Installed:      claude, codex
                      Also supported: hermes, amp (how to install: <link>)
                      Your own harness? <link to "Add your harness">
                      Which one? [claude]
[3/4] Sign in         Press Enter to open Claude Code in its Goal Cup home.
                      Sign in, or paste your API key when it asks. Goal Cup
                      never sees it. Set "don't ask for permission", then exit.  ✓
[4/4] This machine    disk, power, network                                      ✓

Keys for your agent (optional): /Users/you/goalcup-work/.env.local
  Open it now? [y/N]   (made empty with a short comment; Goal Cup never reads it)

Ready.
  Try it (10 min, no GitHub, never counts): goalcup start --harness claude --practice
  The real run:                             goalcup start --harness claude
```

The real run (dry run or cup), and only then:

```
$ goalcup start --harness claude
Your run needs a new, empty, public GitHub repo. Each real run gets its own.
  Name it: my-goal-cup
  Opening github.com/new with that name. Click "Create repository", then Enter.
  agentcommunity/my-goal-cup: public, empty, you can push.                              ✓
Approve this run in your browser: <link>
```

- **Practice needs no GitHub.** The agent works in a local empty git folder, and nothing is
  pushed.
- **Each real run asks for its own repo at start.** A dry run and then the cup run means two
  repos. If the last repo was already used, start suggests a new name (`my-goal-cup-2`).
- **Step 2 lists every supported harness**, not only installed ones, plus an "add your own
  harness" link (the contributor page from #243).
- **Step 3 covers API keys too.** The player types the key into the harness's own sign-in
  (Claude Code, Codex, Amp), or into its Goal Cup home (Hermes's `.env`). It is the
  harness's own file. Goal Cup never reads it.
- **The keys file:** setup creates `~/goalcup-work/.env.local` (empty, with a 3-line
  comment), prints its full path, and offers to open it in the system's default app.
  Goal Cup never reads it again.
- Step 1 runs `goalcup login` inline if this computer is not connected yet.
- GitHub user: found from the player's own login (`ssh -T git@github.com` says
  "Hi <user>!", or `gh api user`). If both fail, it never asks for a user name: it prints a
  short guide (make an account at github.com/signup, install `gh`, run `gh auth login` and let
  it set up git), waits for Enter and looks again. It never makes an account for the player.
- If the push check fails, it prints one fix and one link (GitHub's "add an SSH key" page,
  or `gh auth login`), then stops before the run is approved.

## Changes in goalcup (one PR)

1. **`goalcup setup`** is the new name. `goalcup prep` keeps working as a hidden alias, so old
   docs and players are not broken. The steps go from 6 to 4: GitHub moves to `start`, and the
   practice step is dropped.
1b. **Practice without GitHub.** `start --practice` clones nothing. The agent works in a new
   local git folder with no remote.
1c. **The repo is asked at `start`, for real runs only.** It is saved for that run only. The
   next real run asks again, and suggests a new name when the last repo is no longer empty.
2. **No token.**
   - `start` no longer asks for a GitHub token, and no `github.json` is written.
   - It checks "public and empty" with GitHub's open API (no login needed).
   - It checks "you can push" with the player's own git: `git push --dry-run` from a temp
     repo.
3. **The clone uses the player's own git.** `goalcup start` clones with the player's normal
   git and login, over SSH if that works, otherwise HTTPS. No credential helper is written
   into the clone.
4. **The agent's git finds the player's login.** The env is still built from nothing, and
   only these are carried across, as paths or sockets, never key values:
   - `SSH_AUTH_SOCK`, so a key with a passphrase or one in the Mac keychain works. A key
     without a passphrase already works, because SSH finds `~/.ssh` without `HOME`.
   - `GIT_CONFIG_GLOBAL` = the player's real `~/.gitconfig`, so a saved-password helper
     (macOS keychain, Git Credential Manager) works.
   - `GH_CONFIG_DIR` = the player's real gh config folder, if `gh` is the helper.
5. **`.env*` is never pushed by mistake.** At clone time goalcup adds `.env*` to the clone's
   `.git/info/exclude`, which is git's private ignore list. It is never committed, and
   goalcup never reads a `.env` file.
6. **Old runs keep working.** The hidden `goalcup git-credential` helper stays for clones made
   by older versions (dry runs started this week). New clones do not use it. After the event
   it can be removed.

## Docs and rules

- PARTICIPANT-GUIDE and README: the 4-step setup above replaces the 6 prep steps. Short.
- RULES, a new "Keys" section, 3 lines:
  - Never put keys in your repo.
  - You may give your agent keys in `~/goalcup-work/.env.local`. Set them up before the
    start; changing them during the run is outside help.
  - Your agent can use any service and make its own accounts. What it does is in the record
    and is what counts.
- DECISIONS: a new D69, "Goal Cup Proxy holds no keys; the agent pushes with the player's own
  git login." It narrows D1/D38 (env built from nothing) by exactly the three path variables
  above.
- Website copy: `goalcup prep` becomes `goalcup setup`, and the token step is removed.

## Tests

- Setup: each step's pass and fail lines; the alias `prep` still works.
- Repo check: public, empty and pushable, using a local bare repo plus a fake API.
- Clone: no credential helper in `.git/config`; `.env*` in `.git/info/exclude`.
- Env: exactly the three new variables pass through, and only when set; no key value ever
  does.
- Old clones with the `goalcup git-credential` helper still push.
- `sh scripts/gate.sh fast` passes (the IPv6 test aside, a container limit).

## Real-life test (owner, on a Mac, from zero)

1. Fresh macOS user account. Install, then `goalcup setup`. Time it.
2. Try both GitHub logins: SSH key, and HTTPS with the keychain.
3. Practice run, then a dry run. Check that the agent's `git push` lands on GitHub.
4. Write down every stop. Each one becomes a fix the same day.

## Blind spots

- The agent can push to any of the player's repos (accepted by the owner).
- Pushing over SSH skips the recording proxy. That is fine: the judged commit is read from
  GitHub's API (`server/worker/src/github.ts`), not from recorded traffic.
- Some networks block SSH port 22. Setup suggests `ssh.github.com` port 443 or HTTPS.
- To verify while building:
  - GitHub's `github.com/new?name=...&visibility=public` prefill.
  - Whether `git push --dry-run` proves write access on an empty repo.
- Needs a release before 6 Nov 12:00 UTC. Dry-run players set up this week keep the old flow
  until they upgrade.

## Changes while building (23 Oct)

- `goalcup start --repo <owner>/<name>` names the repo without the question, for scripts and
  tests. Without it, a real run asks.
- The server asks every run for a public `repo_url`. A practice run has none, so it sends the
  organisers' public, empty practice repository
  (`https://github.com/agentcommunity/goalcup-matrix-empty`). Nothing is cloned from it or pushed
  to it. A server change to accept a practice run with no repo is left for later.
- Setup asks "Open it now?" only when it has just made the keys file.
- Verified: GitHub's docs list `name`, `owner` and `visibility` as query parameters of
  `github.com/new` (invalid ones are ignored). Not verified against GitHub: that a dry-run push
  fails without write access (this container's git proxy answers for github.com). Git's push
  connection is GitHub's receive-pack, which needs a login with write access, so a pass means
  git got that far; the participant's first real push is the final proof.
