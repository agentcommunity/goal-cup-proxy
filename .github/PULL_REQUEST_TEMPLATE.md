<!-- What does this change, and why? One concern per PR. -->

<!-- Required checks, review rules, the fork-to-PR steps and the gate: CONTRIBUTING.md. -->

## Every PR

- [ ] I, a human, read the whole diff and answer for it. Any AI assistance is disclosed here or
      in `Co-Authored-By:` trailers.
- [ ] I ran the gate, `sh scripts/gate.sh fast`, and it printed `gate fast: PASS`.
      Platform and Rust version used:
- [ ] No credential, token, real transcript or trace is in this PR, its commits, or its description.
- [ ] Any committed fixture went through `spikes/redact.py` and I reviewed it by eye. Nothing under
      `spikes/*/out/`, `harnesses/` or `target/` is committed.
- [ ] The change sets no model base URL and writes no app-wide config (`~/.claude`, `~/.codex`,
      `~/.hermes`); tests that launch a harness use a private HOME.
- [ ] Outside contributors: I did not edit `TODO.md` or `docs/DECISIONS.md`; any
      proposed text for them is in this description.

## Harness PR only (skip for other PRs)

Follow `skills/adding-a-capture-harness` and `docs/HARNESS-ONBOARDING.md`. The schema reference is
`docs/CONNECTOR-SCHEMA.md`; the surrounding M2 contract is
`docs/CONTRACTS.md#harness-contract-v1`. A code owner reviews against the numbered list (items 1
to 10) in `docs/HARNESS-ONBOARDING.md#reviewing-a-connector-pr`; each box below is the
author's half of that list, so say where the reviewer can check it.

- [ ] Facts sheet `docs/harnesses/<id>.md` filled from the skill's template ("unknown" where
      unknown), with the exact harness version, executable hash and the harness's argv pre-scan
      source (or "none") (item 2).
- [ ] I ran `sh scripts/pin-manifests.sh` in the same commit and the conformance tests as in
      `docs/HARNESS-ONBOARDING.md#steps`. The pin diff is one line of
      `crates/goalcup-protocol/pins/connector-dirs.txt` (item 10).
- [ ] If two harness PRs conflict on the pins, I re-ran the pin script on the merged tree instead
      of hand-merging the generated data.
- [ ] Exact version and executable SHA-256, and the official artifact they came from; `program` is
      the harness's own entry point (item 4).
- [ ] Each file under `files/`, `args`, `env` value, seed, host, native pattern, identity source and
      CA variable is justified in this description, with no model base URL or
      redirect (items 1 to 9).
- [ ] No kit case is added: the generic connector contract supplies the stand-in. Any committed
      fixture went through `spikes/redact.py` (items 7 and 10).
- [ ] The diff touches only `connectors/<id>/`, the pin line above, the goldens and
      `docs/harnesses/<id>.md`; no `.rs` file is changed and no policy row or decoder is added
      (item 10).
- [ ] Last lines of `sh scripts/check-connector.sh <id>` and `sh scripts/gate.sh fast` pasted.
      Claim only what passed: the kit runs a stand-in, not the real harness.
