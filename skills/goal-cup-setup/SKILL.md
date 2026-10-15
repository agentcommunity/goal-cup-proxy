---
name: goal-cup-setup
description: Use when a person asks you to set them up for the /goal Cup, get their machine ready for a Goal Cup run, or run `goalcup prep` for them. Walks the six prep steps, does the ones a command can do, and stops at each step that needs the person.
---

# Setting a person up for the /goal Cup

You are helping a person get their machine ready for a /goal Cup run with the Goal Cup tracer
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
  URL or a proxy variable. The tracer keeps its own home for each harness.

## The loop

1. Check the tracer is installed: `goalcup --version`. If it is not, tell the person how to
   install it (the guide's section 1) and stop.
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
   `goalcup start --harness <id>` inside the start window. If the `machine` step's `say` asks them
   to keep the machine awake themselves, pass that on too. Stop.
4. Otherwise take the **first** step whose `state` is not `ok`:
   - **`"human": false`**: do it yourself where a command can. Run the command its `say` names,
     always with `--json` added when it is `goalcup prep`, so prep never opens the harness or asks
     for anything inside your session. For the harness choice that is
     `goalcup prep --json --harness <id>`, after asking the person which harness they want when
     `say` lists more than one; it saves the choice. Install a missing harness or `git` only with
     the person's agreement. When `say` names something only the person can do (plug the machine
     in, free disk space, allow `goalcup` in a firewall, set Windows power settings), show them
     `say` and wait.
   - **`"human": true`**: stop. Show the person the `say` text exactly as printed, word for word,
     and wait until they say it is done. Do not try to do it for them. These are the browser
     login, opening the harness to sign in and add their settings, creating the repository and
     pasting its token, and the practice run. Several of them need `goalcup prep` run in the
     person's own terminal, because `--json` never asks or opens anything.
5. Go back to step 2. Repeat until every step is `ok`.

`goalcup prep` is safe to run again at any time: it does not redo what it has settled. A `fail`
whose `say` you cannot act on goes to the person verbatim.
