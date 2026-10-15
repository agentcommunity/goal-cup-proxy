---
name: adding-a-capture-harness
description: Use when adding or supporting a CLI coding-agent harness in Goal Cup capture, writing a harness facts sheet under docs/harnesses/, writing connector data under connectors/, or deciding whether retained traffic needs a separate verifier decoder.
---

# Adding a capture harness

This skill holds no procedure of its own. The procedure is one document. Read it and follow it
in order.

1. Read [docs/HARNESS-ONBOARDING.md](../../docs/HARNESS-ONBOARDING.md) from top to bottom. It has
   the steps, the commands, the expected output of each command, the stop rules, the table that
   says whether you need a core change, and the reviewer checklist.
2. Look up every `harness.toml` field in [docs/CONNECTOR-SCHEMA.md](../../docs/CONNECTOR-SCHEMA.md).
3. Write the facts sheet from [templates/harness-facts.md](templates/harness-facts.md). The rules
   are in [docs/harnesses/README.md](../../docs/harnesses/README.md).
4. Copy the other two templates:
   - [templates/connector-harness.toml](templates/connector-harness.toml) to
     `connectors/<id>/harness.toml`;
   - [templates/kit-case/](templates/kit-case/) to
     `crates/goalcup-cli/tests/fixtures/harness-kit/<id>/`.
5. Run `sh scripts/check-connector.sh <id>` and `sh scripts/gate.sh fast`.

A semantic decoder for the harness's traffic is a separate verifier change, not part of a
connector PR: [writing-a-decoder](writing-a-decoder.md).

Rules that never bend:

- A contribution is data only. Add no `.rs` file.
- Never set a model base URL. Never write into a real harness home or `~/.claude`, `~/.codex`,
  `~/.hermes`. Every probe uses a private `HOME`.
- Never put a key or token in a file, argv, log, facts sheet, PR or chat. Never open a key file.
- Never commit `harnesses/`, `target/` or `spikes/*/out/`.
- On a stop rule in the guide, stop. A facts sheet with `Status: facts only` is a complete PR.
