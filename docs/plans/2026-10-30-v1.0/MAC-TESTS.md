# Testing on the Mac (from 2 November)

Vercel Sandbox stops here: it cost a lot, and it cannot test subscription logins (a login from a
datacenter address puts the account at risk). The rest of 1.0 is tested on the owner's Mac, in
Linux VMs on that Mac, or later in the partner's sandbox (Tenki).

## What the Mac adds

- **The macOS build itself.** Nothing has run on macOS since 22 October.
- **Login harnesses.** Amp, Gemini CLI with a Google sign-in, and Claude Code and Codex on a
  subscription. None of these could run in Vercel.
- **No box bill.** Runs cost only the model calls. Each row keeps its `cap_usd`, and the models
  are the pinned cheap ones.

## Set up once (about 30 minutes)

1. Get the code and build it:

   ```bash
   git clone https://github.com/agentcommunity/goal-cup-proxy && cd goal-cup-proxy && git checkout v1.0/integration
   ```

   ```bash
   cargo build --release --locked && export PATH="$PWD/target/release:$PATH" && goalcup --version
   ```

2. The pad needs `tmux`, Python 3.11 or later, and Node with npm:

   ```bash
   brew install tmux python@3.12 node
   ```

3. Install each harness at the version the pad pins. The `install` line in
   `scripts/pad/harnesses/<id>.toml` gives it. For example, Claude Code 2.1.312, Codex 0.165.0,
   Hermes 0.22.5, Gemini CLI 0.66.0 and Amp `0.0.1792209635-g24a9fe`.

4. API keys for pad rows go in `~/.pad-keys/<name>`, mode 600, one file each. The names are
   `claude`, `openai`, `deepseek`, `gemini` and `gateway`. Never put a key on a command line or in
   chat.

5. Run the gate once as your normal user (not root). On the Mac this replaces the gate boxes:

   ```bash
   nice sh scripts/gate.sh full
   ```

## Runs, cheapest first

Every pad row is an existing settings file with one line added, `where = "here"`, so it runs on the
Mac in a fresh temporary folder with its own goalcup home. Keep `server` as it is (staging).

| # | What | Settings | Cost | Proves |
|---|---|---|---|---|
| 1 | Smoke | `scripts/pad/runs/micro.toml` | cents | the Mac build records a call |
| 2 | Core rows | `core-claude-code.toml`, `core-codex.toml`, `core-hermes.toml` | about 1 to 2 USD each | the release blockers on macOS |
| 3 | One-hour run | `a2.toml` with the Paynest prompt | about 5 USD | the first one-hour PASS: kills, recover, 15 subagents |
| 4 | Login rows | by hand, as in `docs/research/LAPTOP-LOGIN-RUNS.md` | your subscriptions | Amp, Gemini CLI login, Claude Code and Codex logins |

Start a pad row like this (one row at a time on the Mac):

```bash
scripts/pad/pad scripts/pad/runs/micro.toml
```

Row 3 needs credit on the Claude key. Codex rows need credit on the OpenAI key.

## Linux VMs on the Mac

The Linux rows (the platform most participants use) move from Vercel to one VM on the Mac (UTM or
Tart, Ubuntu, 4 cores, 8 GB). Install the toolchain and the pinned harnesses once, then snapshot it
clean. Reset to the snapshot before each row: that gives the same fresh start a Vercel box gave, and
the VM stays warm between rows.

To drive the VM from the pad, one small change is needed. All box work goes through
`scripts/pad/sbx.mjs` (133 lines, commands `create`, `put`, `put-env`, `push-dir`, `exec`, `get`,
`rm`, `list`, `snapshot`). A second adapter with the same commands over `ssh` and the VM's
snapshot reset lets every settings file run unchanged. That is the next code task, and it goes
through the merge queue with a gate run on the Mac.

## Tenki

The same adapter shape fits Tenki: one file with the same nine commands against Tenki's API. To
build it we need Tenki's API documentation and a staging token, stored like the other keys, never
in chat. Until then, the Mac and its VM cover every row.

## What changes for the conductor

- No more Vercel boxes for rows, builds or gates from 2 November. The merge-queue gate runs on the
  Mac, where `gate.sh` is the same command.
- Results go in `ACCEPTANCE.md` as before, marked "Mac" or "Mac VM" in the build column.
