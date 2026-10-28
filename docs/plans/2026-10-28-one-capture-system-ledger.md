# One capture system: decision ledger

Running log of choices made while building the [plan](2026-10-28-one-capture-system.md) against the
[spec](2026-10-28-one-capture-system-spec.md). The owner gave the builders room to change the plan
when the product ends up better; every such change is written here with its reason. Newest last.

| # | Date | Decision | Why |
|---|---|---|---|
| L1 | 28 Oct | One branch and one worktree per PR, all cut from `main`: `capture/t1-native-optional`, `capture/t5-evidence-label`, `capture/t67-tunnel-list`, `capture/t234-model-host-defaults`, `capture/t8-docs-d74`. | Parallel builders without shared state; each PR reviews on its own. |
| L2 | 28 Oct | The spec, plan and this ledger land on `main` first, through the Vercel Gateway results PR. | Every later PR links to them; the docs PR's link check passes. |
| L3 | 28 Oct | Builders run focused tests on the cloud container; `sh scripts/gate.sh fast` runs once per PR on a Vercel Sandbox gate machine forked from one snapshot. | The container has 4 cores; GitHub runners are slow; the sandbox has 8 and a non-root user. |
| L4 | 28 Oct | Subscription-login runs (Claude Code, Codex ChatGPT) wait for the owner's laptop; the agents prepare a test guide and run every gateway-key path in the sandbox. | Subscription logins on a datacenter IP risk the account (account-safety research, 28 Oct). |
