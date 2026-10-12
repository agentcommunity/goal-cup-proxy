# Harness onboarding trial

Can a coding agent add a connector end to end from the docs alone? A record of three trial runs
of [the skill](../../skills/adding-a-capture-harness/SKILL.md), for maintainers who change the
onboarding docs. Updated: 2026-10-11.

## Setup

- Reader: the agent that wrote the skill, acting as a fresh reader. It followed only the skill
  and the files the skill links, and read no Rust beyond test failure messages. It was not a
  separate agent session, so the author's own blind spots may remain.
- Harness: a made-up `toy-cli`. Program `toy` is a five-line shell script, reached through a
  symlink on `PATH`, hashed as the pinned executable and never launched by any test. Identity
  `stderr_line`/`deferred`, per-run `TOY_HOME` with one seed, one append-only JSONL native
  file, `NODE_EXTRA_CA_CERTS`, no credential.
- Platform: Linux x86_64, cargo 1.97.0. Source: the onboarding docs before the fixes below.
  Each run used a throwaway worktree under a temp directory, removed afterwards.

## Final run (trial 3)

Step numbers are the skill's. Step 2 was skipped: the harness is made up.

- 1: `cargo build` finished, exit 0.
- 3: all 12 rows of the decision table fit; no core PR.
- 4: the `realpath` one-liner resolved the `toy-link` symlink to the script; `shasum -a 256`
  printed one digest line; `toy --version` printed `toy 1.2.3`, recorded as `1.2.3`.
- 5: readiness record written from the handoff template; one row added to the M2 index table.
- 6: template copied to `connectors/toy-cli/harness.toml`, placeholders replaced, one seed
  added. The placeholder grep printed nothing.
- 7: `sh scripts/pin-manifests.sh` exit 0; `git diff --stat -- crates/*/pins` showed one
  added line.
- 8: `harness_conformance` 5 passed and `connector_pins` 13 passed.
- 9: both regeneration commands passed. New `toy-cli.json`; `enumeration.json` and
  `redaction-extra-patterns.json` gained toy rows. `loader-script.json` changed without
  mentioning toy, so it was restored as step 9 says.
- 10: kit templates copied to `harness-kit/toy-cli/` and edited per their comments. The
  placeholder grep printed nothing.
- 11: `sh scripts/check-connector.sh --allow-no-kit toy-cli` ended `4 passed, 1 failed,
  1 skipped`; the one failure was `loader-script.json`. With the pending Linux loader-golden fix
  (BSD `stat` in `scripts/with-deepseek-key.sh`) applied, it ended `5 passed, 0 failed,
  1 skipped`, exit 0.
- 12: in trial 2, with that fix applied: `cargo build`, `cargo fmt --all --check`, clippy and
  `cargo test --workspace --exclude goalcup-daemon --exclude goalcup-cli` all exit 0.
- 14: `git status --short` showed exactly the files step 14 lists.

The stand-in was also run on Linux against a local CONNECT proxy and TLS echo built for the
trial (not the kit). It printed the identity, wrote the native and decoy files with the bytes
`expect.toml` declares, and completed the echo. The template stand-in passed the same smoke for
`none`, `preassigned`, and `stdout_json` with a snapshot.

## Stuck points found and fixed

Eleven, in order found. Each is fixed in the skill, its templates, `check-connector.sh` or a test.

1. The template said `version` is what `--version` prints; the schema wants the number only.
2. "Add one row to the qualification index" named no table or row shape.
3. The placeholder grep missed `<extend|replace>` and dotted placeholders, and matched comments.
4. A `CARGO_TARGET_DIR` shared with other worktrees ran their test binaries: goldens showed a
   credential name from another branch, and a regeneration wrote into the other worktree.
   Cargo leaves the checkout path out of a workspace crate's build hash, and tests use
   `env!("CARGO_MANIFEST_DIR")`. Fixed: step 1 says one target directory per checkout, and the
   script warns when `CARGO_TARGET_DIR` is outside the checkout.
5. Golden regeneration on Linux also rewrote `loader-script.json`, unrelated to the connector.
6. The decoy keys did not say how the template stand-in names the decoy file.
7. The check script's golden failure named no file and said "regenerate", which would commit
   the Linux loader golden. It now names the files and says what to do with each kind.
8. Step 14 relied on CONTRIBUTING's PR list, which omits the regenerated goldens.
9. The Linux gate failed: `checks_v13.rs` asserted exactly three connector folders and two V1
   manifests. Fixed: both counts are read from `connectors/`, with a floor of three.
10. The kit template had no `seed` or `dirs` lines, and step 10 had no placeholder check.
11. Step 11 did not say how to read a check whose only failure is a platform golden.

Trial 1 found 1 to 9, trial 2 found 10 and 11, and trial 3 found none.

## Still needs a maintainer or macOS

- The harness kit never ran on the toy case. `harness_contract_kit` is macOS-only; the Linux
  smoke above is not the kit. The macOS expected line `6 passed, 0 failed, 0 skipped` is
  unverified.
- The daemon's live goldens (`<id>.daemon.json`, `native-stream-labels.json`) are written only
  on macOS. A Linux contributor's PR lacks them, so a maintainer regenerates them.
- The macOS-only `goalcup-cli` and `goalcup-daemon` suites did not run with a fifth connector.
  Whether any of them assumes exactly three is UNKNOWN.
- Until the pending Linux loader-golden fix (BSD `stat` in `scripts/with-deepseek-key.sh`)
  lands, Linux shows the `loader-script.json` golden failure on every tree.
- The `*.native.json` files under `harness-golden/` are written and read by no test.
- A credential connector cannot have a complete kit case, because the kit injects no key
  (decision table, row 3).
