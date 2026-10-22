# Set up your harness for a 168-hour run

During your run you may type only "keep going" ([RULES](RULES.md#one-prompt-then-only-keep-going)).
So your harness must work for 168 hours without you. This page tells you which harness settings
to set before the start, and where they go.

Goal Cup never changes your harness settings
([D67](DECISIONS.md#d67-goal-cup-recommends-running-without-permission-prompts-the-participant-sets-it)).
You set them yourself. `goalcup prep` tells you when a permission setting is missing, and links
to this page.

The facts here come from Goal Cup's own runs, most of them from the
[local test day of 22 October 2026](research/results/2026-10-22-local-test-day.md). "Not measured"
means that nobody at Goal Cup has tried it. The details are in each
[harness facts sheet](harnesses/README.md).

## Before your run

1. Run `goalcup prep --harness <id>`. Prep opens your harness once, in its Goal Cup home.
2. Sign in there. Answer the harness's first-run screens there ([section 7](#7-first-run-screens)).
3. Exit the harness.
4. Open the settings file of your harness ([where the settings live](#where-the-settings-live)).
5. Add the settings from your harness's section below.
6. Add the Goal Cup time-left tool ([how](#add-the-goal-cup-time-left-tool)).
7. Run `goalcup prep` again. It names each permission setting that is still missing.
8. Do a practice run: `goalcup start --harness <id> --practice`.
9. In the practice run, check that the agent commits without asking you anything, and ask it to
   call its `time_left` tool.

## Where the settings live

Your runs use the harness's Goal Cup home. They never use your own `~/.codex`, `~/.claude` or
`~/.hermes`. Change the settings in the Goal Cup home.

The Goal Cup homes are in the `capture-homes` folder of your Goal Cup configuration folder:

- macOS: `~/Library/Application Support/goalcup/capture-homes/`
- Linux and WSL2: `~/.config/goalcup/capture-homes/` (or `$XDG_CONFIG_HOME/goalcup/capture-homes/`)
- If you set `GOALCUP_CONFIG_DIR`: `$GOALCUP_CONFIG_DIR/capture-homes/`

| Harness | Goal Cup home, in `capture-homes/` | Settings file in that home |
|---|---|---|
| Codex | `codex-cli/` (Codex's `CODEX_HOME`) | `config.toml` |
| Claude Code | `claude-code-cli/` (Claude Code's `CLAUDE_CONFIG_DIR`) | `settings.json` |
| Amp | `amp-cli.run-home/` (Amp's `HOME`) | `.config/amp/settings.json` |
| Hermes | `hermes-cli/` (Hermes's `HERMES_HOME`) | `config.yaml` |

Keep the other keys in the file. Add only the keys below.

## Your harness

### Codex

Add these two lines to `codex-cli/config.toml`:

```toml
approval_policy = "never"
sandbox_mode = "danger-full-access"
```

1. These lines stop all approval prompts ([section 1](#1-run-without-permission-prompts)).
2. `danger-full-access` also lets the agent write `.git`, so it can commit
   ([section 2](#2-let-the-agent-commit-and-push)).
3. Without a sandbox, the agent's commands can change anything your user can. Run it on a machine
   or in a container that you are content to let it change.

### Claude Code

Add these keys to `claude-code-cli/settings.json`:

```json
{
  "permissions": { "defaultMode": "bypassPermissions" },
  "skipDangerousModePermissionPrompt": true
}
```

1. Set both keys. Without the second key, Claude Code starts in a mode that asks
   ([section 1](#1-run-without-permission-prompts)).
2. Run Claude Code as a normal user, not as root. Claude Code refuses bypass mode as root.
3. At each start, answer the screens before you type your prompt
   ([section 7](#7-first-run-screens)). Choose "Yes, I trust this folder". If Claude Code offers
   auto mode, choose "No, keep bypass permissions".
4. Bypass mode lets the agent's commands change anything your user can. Run it on a machine or in
   a container that you are content to let it change.

### Amp

Amp needs no setting to run without permission prompts.

1. Do not add `amp.permissions` rules that ask, in `amp-cli.run-home/.config/amp/settings.json`.
2. Sign in when `goalcup prep` opens Amp ([section 4](#4-sign-in-lifetime)).
3. Choose your mode in prep. Amp keeps the mode you chose last (`agentMode` in
   `.local/share/amp/session.json` of its home).
4. Read [section 5](#5-subagents-and-their-models) before you choose a cheap mode: Amp's
   subagents used a larger model.
5. Put `rg` (ripgrep) on your `PATH` before the start ([section 6](#6-your-tools-on-path)).

### Hermes

Add these keys to `hermes-cli/config.yaml`:

```yaml
approvals:
  mode: "off"
agent:
  max_turns: 2000
```

1. `approvals.mode: "off"` stops the approval prompts ([section 1](#1-run-without-permission-prompts)).
   Write `"off"` with the quotes.
2. `agent.max_turns` lets Hermes work much longer than its default of 90 steps
   ([section 3](#3-turn-and-step-limits)).
3. You can also set it with a command:
   `HERMES_HOME="<your capture-homes folder>/hermes-cli" hermes config set agent.max_turns 2000`.
4. Choose a model service with an API key, not an OAuth sign-in ([section 4](#4-sign-in-lifetime)).

## Add the Goal Cup time-left tool

Your agent may ask how much time is left. `goalcup time-left` works only when the agent's shell
finds `goalcup` on its `PATH` ([section 6](#6-your-tools-on-path)). The time-left tool works
whatever the `PATH`: your harness starts `goalcup mcp` by its full path, and the agent calls its
tool `time_left`.

- It answers the same line as `goalcup time-left`, from the same file.
- It only reads. It cannot stop, seal or continue anything, and it opens no network port.
- It talks to your harness over standard input and output, not over the network. So the recording
  proxy does not see the call. The harness's own session file shows the call and its answer, as
  for any tool (not measured per harness).
- If it is missing, the agent can still run `goalcup time-left`, or read the file named in
  `$GOALCUP_TIME_LEFT_FILE`.

Goal Cup does not add the tool for you. `goalcup prep` prints the lines for your harness, with the
full path of your `goalcup` filled in, and the file to put them in. Add them to that file in the
harness's Goal Cup home, next to the MCP servers you already have. Below, `/path/to/goalcup` stands
for the path prep prints (`command -v goalcup` shows it too), and `<capture-homes>` for the folder
in [where the settings live](#where-the-settings-live).

In a practice run, ask the agent to call `time_left`. That shows the tool works in your harness.

### Codex: the time-left tool

Add to `codex-cli/config.toml`:

```toml
[mcp_servers.goalcup]
command = "/path/to/goalcup"
args = ["mcp"]
env_vars = ["GOALCUP_TIME_LEFT_FILE"]
```

1. Codex hands an MCP server only a few variables of its own, plus the ones `env_vars` names. So
   keep the `env_vars` line.
2. You can also run
   `CODEX_HOME="<capture-homes>/codex-cli" codex mcp add goalcup -- /path/to/goalcup mcp`, then
   add the `env_vars` line.
3. Measured with Codex 0.156.1: `codex mcp add` writes the first three lines, and `codex mcp get`
   reads `env_vars`. Codex starting the tool in a session, and the agent calling it: not measured.

### Claude Code: the time-left tool

Run:

```sh
CLAUDE_CONFIG_DIR="<capture-homes>/claude-code-cli" claude mcp add --scope user goalcup -- /path/to/goalcup mcp
```

1. That adds the server to `mcpServers` in `claude-code-cli/.claude.json`. You can add it there by
   hand instead, while Claude Code is not running:
   `"mcpServers": { "goalcup": { "command": "/path/to/goalcup", "args": ["mcp"] } }`.
2. Measured with Claude Code 2.1.299: the command writes there, and `claude mcp list` starts the
   tool, which answers ("Connected") and gets `GOALCUP_TIME_LEFT_FILE` from Claude Code's
   environment. The agent calling it in a run: not measured.

### Amp: the time-left tool

Add to `amp-cli.run-home/.config/amp/settings.json`:

```json
"amp.mcpServers": {
  "goalcup": {
    "command": "/path/to/goalcup",
    "args": ["mcp"]
  }
}
```

1. `amp.mcpServers` is one key, with the dot in it.
2. You can also run
   `HOME="<capture-homes>/amp-cli.run-home" amp mcp add goalcup -- /path/to/goalcup mcp`.
3. Measured: `amp mcp add` writes this key in that file. Whether Amp hands the tool
   `GOALCUP_TIME_LEFT_FILE`, and the agent calling it: not measured.

### Hermes: the time-left tool

Add to `hermes-cli/config.yaml`:

```yaml
mcp_servers:
  goalcup:
    command: "/path/to/goalcup"
    args: ["mcp"]
    env:
      GOALCUP_TIME_LEFT_FILE: "${GOALCUP_TIME_LEFT_FILE}"
```

1. Hermes hands an MCP server only `PATH`, `HOME` and a few more, plus its `env` map. It fills
   `${GOALCUP_TIME_LEFT_FILE}` from its own environment. So keep the `env` lines (read from the
   source, `tools/mcp_tool.py`).
2. Hermes needs its MCP support installed. Without it, `hermes mcp test goalcup` says to install
   `hermes-agent[mcp]` (measured).
3. Hermes starting the tool, and the agent calling it: not measured.

## 1. Run without permission prompts

A permission prompt waits for an answer. Answering it is typing, and you may type only "keep
going". So a harness that asks waits until the deadline. On 22 October, a Codex run waited 8.7 of
its 10 minutes at an approval prompt. A Claude Code run in Docker waited at "This command requires
approval" until its deadline.

| Harness | Set | Without it |
|---|---|---|
| Codex | `approval_policy = "never"` and `sandbox_mode = "danger-full-access"` in `config.toml` | Codex asks whenever its sandbox blocks a command. In a Linux container the sandbox cannot start (`bwrap: No permissions to create a new namespace`), so every command asks (measured). With `never` alone, those commands fail, and the agent cannot commit. |
| Claude Code | `permissions.defaultMode = "bypassPermissions"` and `skipDangerousModePermissionPrompt = true` in `settings.json` | Without `skipDangerousModePermissionPrompt`, Claude Code 2.1.283 and 2.1.299 start in `default` mode, which asks (measured). |
| Amp | Nothing. Do not add `amp.permissions` rules that ask. | By default Amp runs tools and commands without asking (read from the binary). A Mac run made 19 commits with no prompt (measured). Files on Amp's guarded list can still need an approval (read from the binary, not measured). |
| Hermes | `approvals.mode: "off"` in `config.yaml` | By default (`smart`) a helper model approves low-risk commands, and the rest get a prompt with a 60-second timeout (read from the source, not measured). What happens after the timeout: not measured. The test-day runs made their commits with no stall (measured). |

`goalcup prep` reads the Codex, Claude Code and Hermes settings above. When one is missing, prep
names it in its `harness_home` step. Prep never changes it, and never fails because of it.

Claude Code's `claude --dangerously-skip-permissions` writes `skipDangerousModePermissionPrompt`
into the home it was started with, usually `~/.claude`. That is not the Goal Cup home. So set the
key in the Goal Cup home's `settings.json` yourself.

## 2. Let the agent commit and push

What is judged is the last push to your repository's default branch before the deadline. Goal
Cup never pushes for your agent.

1. Say in your prompt that the agent commits and pushes its work often.
2. Let the agent write `.git`. For Codex, this needs `sandbox_mode = "danger-full-access"`.
3. Goal Cup hands your GitHub token to `git` when the agent pushes. The token is never in the
   agent's environment.
4. A practice run cannot push. So a practice run shows commits, not pushes.

| Harness | Commits on 22 October | Push from a competition run |
|---|---|---|
| Codex | With the default sandbox: none. `git add` failed with `Operation not permitted` on `.git/index.lock`. With the settings above: 2 commits in Docker. | Not measured |
| Claude Code | With the settings above: 6 commits in Docker (2.1.283). In auto mode: 3 commits on a Mac (2.1.299). | Not measured |
| Amp | With no setting: 19 commits on a Mac. | Not measured |
| Hermes | 13 commits in Docker in one 120-minute run. | Not measured |

## 3. Turn and step limits

A harness can stop by itself after a number of steps. It then waits at its prompt. You may type
"keep going", but nobody types it for you while you sleep.

| Harness | Limit | What we saw |
|---|---|---|
| Hermes | `agent.max_turns`, default 90 | Hermes printed "Iteration budget reached (90/90)" and stopped after about 7 minutes. It stayed idle for the rest of a 120-minute run (measured). The test kit sets 1000. The value a 168-hour run needs: not measured. |
| Codex | Not measured | The 120-minute runs finished their task in minutes and then idled. They did not reach a limit. |
| Claude Code | Not measured | No 120-minute Claude Code run on 22 October. |
| Amp | Not measured | The 120-minute runs finished their task in minutes and then idled. They did not reach a limit. |

An agent that stops early and stays idle is allowed. Your prompt decides how long the agent has
work to do.

## 4. Sign-in lifetime

1. Sign in when `goalcup prep` opens the harness. Use a login or account that you can revoke.
2. Do not sign in again during the run. If the harness asks you to sign in again, contact
   support before you answer ([participant's guide](PARTICIPANT-GUIDE.md#10-the-keep-going-rule)).

| Harness | Where the sign-in is kept | What we know about a long run |
|---|---|---|
| Codex | `auth.json` in its Goal Cup home (macOS, measured) | Do not copy `auth.json` to another Codex home. A refresh in the copy made the Goal Cup home's token invalid (`refresh_token_reused`, measured). A token that expires during a run: not measured. |
| Claude Code | macOS: your login Keychain, shared with your own Claude Code (measured). Linux: inside its Goal Cup home (measured). | A sign-in about 8 hours old needed a refresh at the first request. Goal Cup's own lock file blocked that refresh until fix plan v6, so the agent stalled (measured). Goal Cup's lock now has another name. A start with an expired token since that fix: not measured. |
| Amp | `accounts.json` and `secrets.json` in its Goal Cup home (measured) | A sign-in that expires during a run: not measured. |
| Hermes | API key: `.env` in its Goal Cup home (measured). OAuth providers: `auth.json` there (read from the source). | API-key services worked (measured). An OAuth sign-in (Nous Portal and others) may fail to refresh during a run, because its refresh does not go to the recorded model host (read from the source, not measured). Use an API key. |

## 5. Subagents and their models

Your agent may start subagents of its own harness. It must not start a different harness
([participant's guide](PARTICIPANT-GUIDE.md#10-the-keep-going-rule)). A subagent can use a
different model from the main agent. Your account pays for both.

| Harness | What we saw |
|---|---|
| Amp | In mode `low` the main thread used `glm-5p3-flash`, but the `Task` subagents used `claude-opus-5-5` (measured). The mode does not choose the subagents' model. A setting that does: not measured. |
| Codex | A subagent used its parent's model and effort (measured, one subagent). For a subagent that forks the full history, Codex does not guarantee the requested model and effort (read from the Codex 0.156.1 source). A setting for it: not measured. |
| Claude Code | Subagents run inside the same process and write their own transcript files (measured). Their model: not measured. |
| Hermes | `delegate_task` started two subagents in one run (measured). Their model: not measured. |

Try subagents in a practice run. Then check your provider's usage page.

## 6. Your tools on PATH

Goal Cup hands the harness the `PATH` of the terminal where you run `goalcup start`. The run's
`HOME` is an empty folder that Goal Cup makes, so your `~/.zshrc` and `~/.profile` are not read
there.

1. Install the tools your agent needs before the start.
2. Check that they are on `PATH` in the terminal you start from.
3. In a practice run, ask the agent to run `goalcup time-left`. That shows `goalcup` is on its
   `PATH`. The [time-left tool](#add-the-goal-cup-time-left-tool) does not need `goalcup` on the
   agent's `PATH`.

| Harness | What we saw |
|---|---|
| Codex | Codex runs each command in a login shell (`/bin/sh -lc`). On Debian and Ubuntu a login shell resets `PATH`, so Codex in Docker could not find `goalcup` (measured). Since fix plan v5, Goal Cup writes a `.profile` into the run's `HOME` that puts your `PATH` back. No Codex setting changes. A Docker run since that fix: not measured. On macOS the login shell keeps `PATH` (measured). |
| Claude Code | Found `goalcup` in Docker and on a Mac (measured). |
| Amp | Amp uses `rg` from your `PATH`. Without it, Amp tried to download ripgrep at start (measured), and Goal Cup does not record that download. Install ripgrep. |
| Hermes | `goalcup time-left` worked inside Hermes on a Mac and in Docker (measured). |

## 7. First-run screens

Before your prompt, your harness may show its own screens: a sign-in, a folder-trust question, an
offer or a setup wizard. Answering them is allowed. It is not a message to the agent. After your
prompt, the "keep going" rule holds.

1. Answer what you can in `goalcup prep`, when prep opens the harness.
2. Some screens come back at each start. Answer them before you type your prompt.
3. Wait for the chat input before you paste your prompt. A screen that is open can take the
   prompt, and the prompt is lost (measured with Claude Code and Amp).

| Harness | Screen | When | Choose |
|---|---|---|---|
| Claude Code | "Quick safety check: Is this a project you created or one you trust?" | Every run, because each run is a fresh clone (measured) | "Yes, I trust this folder". The cursor starts on "No, exit". |
| Claude Code | "Yes, set auto mode as my default permission mode / No, keep bypass permissions" | With the bypass settings, version 2.1.299 (measured) | "No, keep bypass permissions" |
| Claude Code | "Try the new fullscreen renderer?" | From the second start on (measured) | "Not now", or as you like. Enter picks "Yes, try it", which restarts Claude Code's display. |
| Claude Code | "1 MCP server needs authentication" | When your account syncs MCP servers into the home (measured, 2.1.283) | Answer it before your prompt. |
| Codex | "Update available" | Goal Cup's launch turns the update check off, which stops this screen (measured) | Nothing to do |
| Codex | "Approaching rate limits - switch model?" and two model-migration offers | Codex 0.153.0 can show them in a session. Whether one stops an agent during a run: not measured. | The `[tui]` keys `hide_rate_limit_model_nudge`, `hide_gpt5_1_migration_prompt` and `"hide_gpt-5.1-codex-max_migration_prompt"` exist in the 0.153.0 binary (measured). Their effect in a run: not measured. |
| Amp | "Would you like to log in to Amp?" | A home with no sign-in (measured) | Sign in during `goalcup prep`. |
| Amp | Welcome screens | First start in a new home (measured in Docker) | Answer them during `goalcup prep`. Amp keeps the answer in its home. |
| Hermes | "Run setup now? [Y/n]" | A home with no model service (measured) | Answer Y during `goalcup prep`, and choose your service and model. If it comes in a run, do the setup; Hermes then closes. Run `goalcup continue`, then type your prompt. |
