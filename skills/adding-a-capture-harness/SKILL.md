---
name: adding-a-capture-harness
description: Use when adding or supporting a CLI coding-agent harness in Goal Cup capture, writing a harness facts sheet under docs/harnesses/, writing connector data under connectors/, or deciding whether retained traffic needs a separate verifier decoder.
---

# Adding a capture harness

Any harness can be added, not only the ones that ship. A connector is data: one folder, no Rust.
This skill holds no procedure of its own. The procedure is one page:
[docs/HARNESS-ONBOARDING.md](../../docs/HARNESS-ONBOARDING.md). Follow it in order, from its
[quick path](../../docs/HARNESS-ONBOARDING.md#quick-path). It has the steps, the commands, the
expected output of each, the stop rules, the "do I need a core change?" table and the reviewer
checklist.

The pages it uses:

- [The connector contract](../../docs/CONNECTOR-SCHEMA.md#the-connector-contract): what a
  connector may rely on, and what core changes will not break. Every field:
  [docs/CONNECTOR-SCHEMA.md](../../docs/CONNECTOR-SCHEMA.md).
- The templates: [templates/connector-harness.toml](templates/connector-harness.toml) (chat mode,
  D57, the only mode) and [templates/harness-facts.md](templates/harness-facts.md) (rules in
  [docs/harnesses/README.md](../../docs/harnesses/README.md)).
- [docs/HARNESS-SETUP.md](../../docs/HARNESS-SETUP.md): the participants' setup guide. A
  connector PR adds its harness's section there (`participant.setup_section`).
- A semantic decoder is a separate verifier change, never part of a connector PR:
  [writing-a-decoder](writing-a-decoder.md). A connector needs none.

Rules that never bend:

- A contribution is data only. Add no `.rs` file, and write no test or kit case: the connector
  contract runs every connector folder from its `harness.toml`.
- Never set a model base URL. Never write into a real harness home or `~/.claude`, `~/.codex`,
  `~/.hermes`. Every probe uses a private `HOME`.
- Never put a key or token in a file, argv, log, facts sheet, PR or chat. Never open a key file.
- Never commit `harnesses/`, `target/` or `spikes/*/out/`.
- On a stop rule in the guide, stop. A facts sheet with `Status: facts only` is a complete PR.
- A connector runs without `--dev` only once a release pins it. For Goal Cup 2026 the last
  release is 6 Nov 2026 12:00 UTC.
