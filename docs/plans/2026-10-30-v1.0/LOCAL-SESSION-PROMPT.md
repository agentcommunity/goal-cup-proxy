# Prompt for the local session on the owner's Mac

Paste the block below into a new Claude Code session started in the Goal Cup checkout on the Mac.
It is the hand-over from the cloud conductor (2 November). The cloud session stops using Vercel
Sandbox. All further tests run on the Mac.

---

You are the local test conductor for Goal Cup 1.0 on the owner's Mac. Read AGENTS.md, VISION.md,
docs/plans/2026-10-30-v1.0/OWNER-DECISIONS.md, MORNING.md, ACCEPTANCE.md and MAC-TESTS.md from
branch v1.0/integration first.

Where things stand: the code is ready for the freeze. Claude Code and Codex are proven in the cloud,
and seven more harnesses pass with every subagent linked to its parent. No one-hour run is a full
PASS yet. The best (A2 pass 4) met the subagent target but had a quiet kill, and the pad now waits
for work before a kill. Hermes has no PASS, pending owner ask 4.

The Mac (checked 2 November): macOS 26.6.2, M4 Pro, 12 cores, 24 GB, 353 GB free.
- Checkout: /Users/team/dev/PROJECTS/AgentCommunity/goal-cup-tracer on main, with origin/v1.0/integration fetched. Work in a new worktree at origin/v1.0/integration and leave main alone.
- Installed: Rust 1.98, tmux, Python 3.14, Node, npm, Docker. No UTM or Tart.
- Harnesses: Hermes 0.22.5 matches the pin. Claude Code and Codex are older than the pins, and Amp is newer. Nine are missing.
- No ~/.pad-keys yet. The owner says the test keys are in .env.local in the main checkout.

Rules:
- Never print, copy or log a key value. Write keys to ~/.pad-keys/<name> (mode 600) with a script that prints only names.
- Do not change the owner's everyday tools or their ~/.claude, ~/.codex or ~/.hermes. Install pinned harness versions into a separate npm prefix, on PATH only for pad runs.
- Staging only. Never touch main, tags, production or the website database. Push only v1.0/* branches.
- Ask the owner before any step that costs more than about 1 USD, and before any sign-in.

Steps, in order, stopping to report after each:
1. Build the worktree (cargo build --release --locked). Run `nice sh scripts/gate.sh full` in the background.
2. Write ~/.pad-keys from .env.local. Run a Hermes micro check: micro.toml with harness = "hermes-cli", key = "deepseek", where = "here". It costs cents.
3. Core rows with where = "here": core-claude-code, core-codex and core-hermes. Each needs credit on its key. Ask first.
4. The one-hour run: a2.toml with the Paynest prompt, where = "here". About 5 USD. Ask first.
5. Login runs by hand with the owner: Amp, Gemini CLI, and Claude Code and Codex subscriptions (docs/research/LAPTOP-LOGIN-RUNS.md).

Record each result in ACCEPTANCE.md with "Mac" in the build column, and push to v1.0/integration as
docs-only commits.

---
