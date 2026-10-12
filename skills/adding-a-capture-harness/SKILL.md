---
name: adding-a-capture-harness
description: Use when adding or supporting a CLI coding-agent harness in Goal Cup capture, writing a harness facts sheet under docs/harnesses/, writing connector data under connectors/, or deciding whether retained traffic needs a separate verifier decoder.
---

# Adding a capture harness

What do I do, in order, to add a harness to the Goal Cup tracer? The checklist for a harness
team's coding agent. Every step has a command, its expected output, and a STOP rule. Read the
linked files when a step points to them; do not guess past a STOP. The human summary is
[HARNESS-ONBOARDING](../../docs/HARNESS-ONBOARDING.md). Updated: 2026-10-12.

A contribution is data only: a facts sheet `docs/harnesses/<id>.md`, a connector
`connectors/<id>/` (`harness.toml` plus reviewed seed files), one pin line, regenerated goldens
and one kit case. No `.rs` file. Policy rows and decoders are separate core PRs. A harness that
does not fit yet contributes the facts sheet alone.

## STOP rules

- **Needs a core PR** (step 3): stop, open an issue naming the decision-table row and linking
  the facts sheet. Write no connector. The facts sheet, with `Status: facts only`, is the PR.
- **Unknown fact** in transport, session storage, isolation or auth, or session identity: write
  "unknown" in the facts sheet, set `Status: facts only`, and stop.
- **A key or token anywhere it should not be**: a key never goes in a file, argv, log, facts
  sheet, PR or chat. Never open or print a key file. The kit uses an invented key.
- **Real HOME touched**: stop and report it. Every probe uses a private `HOME`.

## Checklist

Placeholders: `<id>` is the connector id (`<name>-cli`, matching `^[a-z][a-z0-9-]{1,47}$`),
`<program>` is the harness executable. Run from the repo root.

1. **Set up.** Fork and branch as in
   [CONTRIBUTING](../../CONTRIBUTING.md#from-fork-to-pull-request) and read the rules in
   [AGENTS.md](../../AGENTS.md) once. Check `which cargo` is `~/.cargo/bin/cargo` (rustup), not
   a Homebrew cargo that shadows it. Then, online, once per machine and once per checkout:
   ```
   cargo nextest --version || cargo install cargo-nextest --locked
   cargo fetch --locked
   cargo build
   ```
   Expect: `Finished` and exit 0. After this the gate runs cargo `--offline`. Warm timings:
   `check-connector.sh` about 1 to 1.5 minutes, `gate.sh fast` about 2.5; a cold first build
   takes longer, and `target/` grows to about 3 GB. Keep one cargo target directory per
   checkout (the default `target/`).
2. **Fill the facts sheet.** `docs/harnesses/<id>.md` from
   [templates/harness-facts.md](templates/harness-facts.md):
   ```
   mkdir -p docs/harnesses
   cp skills/adding-a-capture-harness/templates/harness-facts.md docs/harnesses/<id>.md
   ```
   Keep every heading and its order; replace each one-line prompt with facts. Every fact has a
   source (docs link, "measured on version X") or says "unknown"; never guess. No secrets,
   tokens or personal paths (`$HOME`, `%APPDATA%`). Measure with a private `HOME` and harness
   home under a temp directory, against the exact installed version. Worked example:
   [codex-cli.md](../../docs/harnesses/codex-cli.md). Rules and the meaning of each `Status:`
   value: [docs/harnesses/README.md](../../docs/harnesses/README.md). The sections map onto the
   connector like this:

   | Facts sheet section | Lands in `harness.toml` |
   |---|---|
   | 1. Run one prompt, headless | `[capture].args`, `goal_flag` or a trailing `--`, `continuation` |
   | 2. Install and pin a version | `[capture.executable]` (step 4) |
   | 3. Where it keeps things (macOS column) | `[capture.home]`, `[[capture.native]].root` |
   | 4. Session format and identity | `[[capture.native]]` `pattern`/`form`, `[capture.identity]` |
   | 5. Auth | `[capture.credential]`, `auth_hosts`, `[capture.home]` `run_home` |
   | 6. Network | `[[capture.hosts]]`, `[capture.ca]` |
   | 7. Process model | `[capture.termination]`, `[capture.argv_guard]` |
   | 8. Things Goal Cup cannot isolate | nothing; it stays in the sheet (a blocker if it changes the run) |

   Section 8 lives only in the sheet; the PR description links the sheet rather than repeating
   it. STOP on an unknown in sections 4, 5 or 6, or in the macOS column of section 3.
3. **Decide: do I need a core PR?** Walk every row of [decision-table.md](decision-table.md).
   STOP on any "core PR" or "core decision" row.
4. **Pin the executable.** Hash the regular file the CLI will run, not a symlink:
   ```
   python3 -c 'import os,shutil,sys; print(os.path.realpath(shutil.which(sys.argv[1])))' <program>
   shasum -a 256 <that path>
   <program> --version
   ```
   Expect: one absolute path; one line `<64 hex>  <path>`; the version. These go into
   `[capture.executable]` and section 2 of the facts sheet; `version` is the number only
   (`1.2.3`, not `toy 1.2.3`). A harness that is a script with a shebang passes the pin: the
   hash is of the script file, and the interpreter it runs under is not pinned.
   No check verifies this hash against your binary: `check-connector.sh` and the kit never run
   the real harness (the kit runs `/usr/bin/python3`), and the binary need not be on `PATH`
   for them. A wrong hash shows up only as a changed `"sha256"` in your `<id>.json` golden
   (step 7), and as a refusal when `goalcup start` launches the real binary. A maintainer
   re-derives the hash from an official artifact before merging, so record where yours came
   from in section 2.
5. **Write the connector.**
   ```
   mkdir -p connectors/<id>
   cp skills/adding-a-capture-harness/templates/connector-harness.toml connectors/<id>/harness.toml
   ```
   Replace every `<PLACEHOLDER>` and delete unused options; the template's comments may go too.
   The closest complete example is `connectors/hermes-cli/`. Seeds go in `connectors/<id>/files/`
   and must be named in `seed`; any other file in the folder is refused. Field rules:
   [CONNECTOR-SCHEMA](../../docs/CONNECTOR-SCHEMA.md). Check that no placeholder is left
   outside comments: `grep -nE '^[^#]*<[^>]+>' connectors/<id>/harness.toml`. Expect: no output.
6. **Regenerate the pins.** `sh scripts/pin-manifests.sh`. Expect: no output, exit 0. Then
   `git diff --stat -- crates/*/pins`. Expect: `crates/goalcup-protocol/pins/connector-dirs.txt`
   with one line added for `<id>`, and nothing else. Re-run after every edit under
   `connectors/<id>/`; never edit a pin by hand.
7. **Regenerate the goldens.**
   ```
   GOALCUP_REGEN_HARNESS_GOLDEN=1 cargo test -p goalcup-cli --test full harness_contract_golden::
   GOALCUP_REGEN_HARNESS_GOLDEN=1 cargo test -p goalcup-daemon --test full harness_contract_golden::
   git status --short crates/goalcup-cli/tests/fixtures/harness-golden
   ```
   Expect: both `test result: ok.`; a new `<id>.json` and `<id>.daemon.json`, and changed
   `enumeration.json`, `redaction-extra-patterns.json` and `native-stream-labels.json`.
   `git diff` must show no change to another connector's values. A changed golden that does not
   mention `<id>` did not change because of your connector: `git checkout -- <that file>` and
   name it in the PR. Set the variable per command as above, never `export` it.
8. **Add the kit case.**
   ```
   k=crates/goalcup-cli/tests/fixtures/harness-kit/<id>
   mkdir -p $k && cp skills/adding-a-capture-harness/templates/kit-case/* $k/
   ```
   Follow the comments in the three files, then delete them if you like (replacing a file
   wholesale is fine): the case mirrors the connector's home, identity and
   native shape with a Python stand-in, uses id `kit-<id>`, and copies any seeds into
   `$k/files/`. Read the closest baseline case beside it: `controlled-workload`,
   `synthetic-preassigned`, `synthetic-stderr-token`, `synthetic-keyed-stderr`,
   `synthetic-stdout-json` or `synthetic-keyed-stdout-json`. A keyed case names
   `credential_env` in `expect.toml` and copies the connector's `[capture.credential]`; the kit
   hands the stand-in an invented key and checks that no fragment of it leaks.
   `native_path` is relative to the native root (the `pattern`, without the root's
   directories); `native_decoy_path` is a suffix under the run directory and does include the
   root's last directory. The comments in `expect.toml` have a worked pair.
   Check: `grep -nE '^[^#]*<[^>]+>' $k/harness.toml $k/expect.toml`. Expect: no output. In
   `stand-in.py` only the EDIT block must be free of `<`: the stand-in stops on a leftover there
   and step 9 checks it; the docstring's `<HARNESS_ID>` is safe to edit or keep.
   Check that `BODY` in `stand-in.py` equals `wire_request` in `expect.toml`:
   ```
   grep -hE '^(BODY|wire_request) = b?"' $k/stand-in.py $k/expect.toml | cut -d\" -f2 | uniq -c
   ```
   Expect: one line, `2` and then the body.
9. **Run the connector check.** `sh scripts/check-connector.sh <id>`. It runs files, pins,
   schema, both goldens and the kit. Expect, last line:
   `check-connector: <id>: 6 passed, 0 failed, 0 skipped`. The files, pins and goldens steps
   print what to do; a golden failure names the differing files. A kit failure prints only
   `FAIL kit` in the summary; scroll up to the Rust assertion, which shows the expected and
   actual values raw, for example
   `left: "session:session-x_42.jsonl#g1"` / `right: "session:sessions/session-x_42.jsonl#g1"`
   (`left` is what was captured, `right` what `expect.toml` predicts; here `native_path`
   wrongly repeated the root's `sessions/`). Fix the `expect.toml` field the `right` side
   came from. When the only failure is a golden that does not
   mention `<id>` (step 7), say so in the PR. Off macOS the kit cannot run: STOP and ask for a
   Mac rather than claim a pass.
10. **Run the gate.** `sh scripts/gate.sh fast`. Expect, last line: `gate fast: PASS in <N>s`.
    It runs build, format, lint and the fast tests; the goldens and kit are covered by step 9.
11. **Review your own diff** against the
    [reviewer checklist](../../docs/CONNECTOR-CONTRIBUTING.md#reviewing-a-connector-pr): every
    field and seed justified, no base URL or endpoint variable, no secrets, one pin line.
12. **Open the PR** holding exactly: `connectors/<id>/`, the one new line in
    `connector-dirs.txt`, the golden JSON from step 7, the kit case from step 8, and
    `docs/harnesses/<id>.md`. Check with `git status --short`. In the body, paste the last line
    of steps 9 and 10, name anything restored or not run, say which parts an agent wrote, and
    have a person confirm they read the whole diff. Claim only what passed: the kit shows the
    tracer works with a stand-in shaped like the harness, not that the real harness was run.

Fast inner loop while fixing schema errors (step 9 runs it too):
`cargo test -p goalcup-protocol --test it -- harness_conformance:: connector_pins::`.

## When something fails

| Symptom | Where to look |
|---|---|
| Field or value refused | [CONNECTOR-SCHEMA](../../docs/CONNECTOR-SCHEMA.md), then step 3 |
| `declares id ..., which is not the folder name` | `id` must equal the folder name |
| Pin test fails after an edit | re-run step 6 and commit what it writes |
| Golden mismatch | step 7, then read the JSON diff |
| A kit case refused for a pinned id | the kit id must be `kit-<id>`, not `<id>` |
| `stand-in.py: replace the placeholder in <NAME>` | the EDIT block in `stand-in.py` |

## Reference

- [templates/](templates/): the facts sheet (step 2), connector (step 5) and kit case (step 8).
- [decision-table.md](decision-table.md): what needs a core PR, with Amp as the worked example.
- [docs/harnesses/README.md](../../docs/harnesses/README.md): why facts sheets exist, how to update.
- [CONNECTOR-CONTRIBUTING](../../docs/CONNECTOR-CONTRIBUTING.md): the reviewer checklist.
- [writing-a-decoder](writing-a-decoder.md): a separate verifier PR; a connector needs none.
- Never set a model base URL, write into a real harness home, or commit `harnesses/`, `target/`
  or `spikes/*/out/` ([AGENTS.md](../../AGENTS.md)).
