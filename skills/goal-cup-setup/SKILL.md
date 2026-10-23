---
name: goal-cup-setup
description: Use when a person asks you to set them up for the /goal Cup, get their machine ready for a Goal Cup run, or run `goalcup prep` for them. Walks the six prep steps, does the ones a command can do, and stops at each step that needs the person.
---

# Setting a person up for the /goal Cup

You are helping a person get their machine ready for a /goal Cup run with Goal Cup Proxy
(`goalcup`). The person's guide is [docs/PARTICIPANT-GUIDE.md](../../docs/PARTICIPANT-GUIDE.md);
the rules are [docs/RULES.md](../../docs/RULES.md). This skill gets them as far as
`goalcup prep: ready.` and no further.

## Never

- **Never start a competition run.** Do not run `goalcup start` without `--practice`, and never
  `goalcup start --test`. The person starts their own run, at the start window, themselves.
- **Never start a practice run either.** It opens the harness in the terminal for the person to
  type into; it is a human step (below).
- **Never type into the harness.** Not a prompt, not "keep going", nothing. Every message typed
  into a recorded harness is reviewed.
- Never ask for, read, print or store the GitHub token yourself. The person pastes it into
  `goalcup prep`, which reads it without showing it.
- Never write to `~/.claude`, `~/.codex` or any harness's own config, and never set a model base
  URL or a proxy variable. Goal Cup Proxy keeps its own home for each harness.

## Tell the person this early

Say these plainly, once, before the first human step:

- **Two approvals are normal.** `goalcup login` approves this computer once. It signs the person
  in and gives Goal Cup Proxy no credential for any run. Each `goalcup start` then approves that one
  run, so no run starts on their account without their approval. It is not a mistake when the
  browser asks again.
- **After each browser approval, return to the terminal now.** The next step happens there.
- **A firewall such as Little Snitch may ask about `goalcup`.** Allow it. Each Goal Cup Proxy update is a
  new program file, so the firewall may ask again after every update. Allow it again.
- **Any harness may show setup screens before the first prompt** (Claude Code's folder-trust
  question, a sign-in, a model choice). The person answers them themselves before they type their
  prompt. That is allowed for every harness. Do not answer them for the person.
- **Set the harness so it does not stop to ask for approvals.** During the run nobody may approve
  anything, so choose the harness's own setting for this before the run.
- **The person types one prompt, and after that only "keep going".**

## The loop

1. Check Goal Cup Proxy is installed: `goalcup --version`. If it is not, tell the person how to
   install it and stop. The guide's section 1 has the one command, for macOS, Linux and (inside
   WSL2) Windows: `curl -fsSL https://github.com/agentcommunity/goal-cup-tracer/releases/latest/download/install.sh | sh`.
   If no release is published yet, follow the README's build-from-source steps.
2. Run `goalcup prep --json`. It asks nothing and opens nothing. It prints one JSON array of six
   objects, in order, each `{"step", "state", "human", "say"}`:
   - `step` is `login`, `harness`, `harness_home`, `github`, `machine` or `practice`;
   - `state` is `ok`, `todo` or `fail`. An unavailable sleep lock never makes the `machine` step
     not-ok: its `say` then tells the person to keep the machine awake themselves (under WSL2,
     with the Windows power setting);
   - `human` is `true` when the step needs the person;
   - `say` is what was found, what to do, or why it failed.

   A flag prep refuses (a `--repo` that is not a GitHub repository address, a `--work-root` that
   is not a full path) gives no array: prep prints one plain sentence instead and exits non-zero.
   Show that sentence to the person verbatim.
3. If every `state` is `ok`, tell the person they are ready, and that they start their run with
   `goalcup start --harness <id>` inside the start window. The window opens on 7 November 2026 at
   00:00 UTC and lasts 24 hours. `goalcup start` asks for its own browser approval; that is the
   second approval, and it is normal. If the `machine` step's `say` asks them
   to keep the machine awake themselves, pass that on too. Stop.
4. Otherwise take the **first** step whose `state` is not `ok`:
   - **`"human": false`**: do it yourself where a command can. Run the command its `say` names,
     always with `--json` added when it is `goalcup prep`, so prep never opens the harness or asks
     for anything inside your session. For the harness choice that is
     `goalcup prep --json --harness <id>`, after asking the person which harness they want when
     `say` lists more than one; it saves the choice. Install a missing harness or `git` only with
     the person's agreement. Install Hermes only as `uv tool install hermes-agent==0.19.0`, never
     with Hermes's own installer, which installs the newest release (it writes no session file;
     see the Hermes section of [HARNESS-SETUP](../../docs/HARNESS-SETUP.md#hermes)). When `say` names something only the person can do (plug the machine
     in, free disk space, allow `goalcup` in a firewall, set Windows power settings), show them
     `say` and wait.
   - **`"human": true`**: stop. Show the person the `say` text exactly as printed, word for word,
     and wait until they say it is done. Do not try to do it for them. These are the browser
     login (the approval page shows a code; tell them to check it matches the one in the terminal,
     approve, and return to the terminal now), opening the harness to sign in and add their
     settings, creating the repository and
     pasting its token, and the practice run. Several of them need `goalcup prep` run in the
     person's own terminal, because `--json` never asks or opens anything.
5. Go back to step 2. Repeat until every step is `ok`.

`goalcup prep` is safe to run again at any time: it does not redo what it has settled. A `fail`
whose `say` you cannot act on goes to the person verbatim.

## When you are stuck

Show the person the exact text prep printed. For help they can write to goalcup@agentcommunity.org.
