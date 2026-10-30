<!-- What does this change, and why? One concern per PR. -->

<!-- Required checks, review rules, the fork-to-PR steps and the gate: CONTRIBUTING.md. -->

## Every PR

- [ ] I, a human, read the whole diff and answer for it. Any AI assistance is disclosed here or
      in `Co-Authored-By:` trailers.
- [ ] I ran `sh scripts/gate.sh fast` and pasted its last line, `gate fast: PASS in <N>s`, here.
      Platform and Rust version used:
- [ ] No credential, token, real transcript or trace is in this PR, its commits, or its description.
      Nothing under `target/` is committed.
- [ ] The change sets no model base URL and writes no app-wide config (`~/.claude`, `~/.codex`,
      `~/.hermes`); tests that launch a harness use a private HOME.

## Connector PR only (skip for other PRs)

Follow `skills/adding-a-capture-harness`.

- [ ] The connector is data only (`connectors/<id>.toml`); no `.rs` file is changed.
- [ ] The pad's `row.json` for this connector is attached or pasted, and its row is PASS.
      Claim only what passed.
