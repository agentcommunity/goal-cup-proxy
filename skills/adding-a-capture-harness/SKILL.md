---
name: adding-a-capture-harness
description: Use when adding or supporting a CLI coding-agent harness in Goal Cup capture, writing a harness facts sheet under docs/harnesses/, writing connector data under connectors/, or writing the test pad data file for a harness.
---

# Adding a capture harness

Any harness can be added, not only the ones that ship. The test pad is the one way to add it:
you write two small data files, run one command, read one table. No Rust, no test, no decoder.

Rules that never bend:

- A contribution is data. Add no `.rs` file, and write no test.
- Never set a model base URL in a connector. The pad file may hold the provider and model
  settings a participant would write, because it is test data and goalcup never reads it.
- Never write into a real harness home (`~/.claude`, `~/.codex`, `~/.hermes`, ...). The pad
  runs the harness in an isolated home.
- Never put a key or token in a file, argv, log, facts sheet, PR or chat, and never open a key
  file. Give the pad a key name; it reads the value from the environment.
- Never commit `pad-out/`, `target/` or `spikes/*/out/`.

## The five steps

1. **Read the harness's current docs first**, and cite each page with its date. Write what you
   learn in a facts sheet, [templates/harness-facts.md](templates/harness-facts.md) (rules in
   [docs/harnesses/README.md](../../docs/harnesses/README.md)). Then write `connectors/<id>.toml`:
   one flat file, data only. Every key and the rule that refuses it:
   [docs/CONNECTOR-SCHEMA.md](../../docs/CONNECTOR-SCHEMA.md). Copy a neighbour such as
   `connectors/hermes-cli.toml`. A maintainer adds the one line in
   `crates/goalcup/src/connector/builtin.rs` when the PR merges.
2. **Write `scripts/pad/harnesses/<id>.toml`**, from
   [templates/pad-harness.toml](templates/pad-harness.toml): the pinned version and how to
   install exactly that version, how the key reaches the harness (`login`, `file` or `env`, as
   its docs say), the provider and model config it should start with, what to type to exit, and
   the host of its model calls (`model_host`). Look at `claude-code-cli.toml`,
   `codex-cli.toml` and `hermes-cli.toml` beside it. Launch facts (proxy and CA variables,
   permission prompts, auto-update switches) belong in the connector, not here.
3. **Run it.** Copy `scripts/pad/runs/micro.toml` and set `harness = "<id>"`,
   `key = "<your key name>"`, `connector = "connectors/<id>.toml"`, `where = "here"` and
   `prompt = "smoke"` (it keeps `server = "off"`). `connector` loads your new file into the
   shipped binary, so nothing is rebuilt. Then run `scripts/pad/pad <that file>`. Read the table
   and `report.txt` under `pad-out/`. Fix the connector until checks C1 to C3 and C6 pass and
   C4's local half passes. The record is the proof; no log reader is needed.
4. **Optional: attach a native sample.** If the harness keeps session files and the row says
   `local_log: none`, attach the masked native streams of the run to the pull request:
   `goalcup-verify dump <entry> --kind open` lists the streams (`skind` says `native`), and
   `goalcup-verify dump <entry> --sid <N> --text` prints one. A reader for the format may later
   be written from that sample. It adds the local log check and is not needed for a tier.
5. **Open the pull request** with `row.json` from the run and the facts sheet. A maintainer runs
   the pad on staging with `mode = "full"` and `case = "all"`. A PASS gives the tier.

A facts sheet with `Status: facts only` is a complete PR when a key fact is unknown. Stop and say
so rather than guess.
