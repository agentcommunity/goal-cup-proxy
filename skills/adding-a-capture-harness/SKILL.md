---
name: adding-a-capture-harness
description: Use when adding or supporting a CLI coding-agent harness in Goal Cup capture, writing schema-1 connector data under connectors/, onboarding a harness with no connector, or deciding whether retained traffic needs a separate verifier decoder.
---

# Adding a capture harness

What do I do, in order, to add a harness connector? The single entry point for a contributor's
coding agent. Every step has a command, its expected output, and a STOP rule. Read the linked
docs when a step points to them; do not guess past a STOP. Updated: 2026-10-12.

A normal contribution is data only: `connectors/<id>/harness.toml`, reviewed seeds, one pin
line, regenerated goldens, one kit case and a readiness record. No `.rs` file. Policy rows and
decoders are separate core PRs. The best claim a contributor can earn is **synthetic tested**.

## STOP rules

- **Needs a core PR**: stop, open an issue naming the rule, and write no connector.
- **UNKNOWN fact**: in transport, native storage or scope, isolation or auth, or identity:
  record BLOCKED in the readiness record and stop.
- **Needs a live key or subscription**: maintainer only. Never open, paste or log a key.
- **Real HOME touched**: stop and report it. Every probe uses a private `HOME`.

## Checklist

Placeholders: `<id>` is the connector id (`<name>-cli`, matching `^[a-z][a-z0-9-]{1,47}$`),
`<name>` is the id without `-cli`, `<program>` is the harness executable. Run from the repo root.

1. **Set up.** Fork and branch as in
   [CONTRIBUTING](../../CONTRIBUTING.md#from-fork-to-pull-request). Read the rules in
   [AGENTS.md](../../AGENTS.md) once. Then run `cargo build`.
   Expect: `Finished` and exit 0. Keep one cargo target directory per checkout (the default
   `target/`). Never share a `CARGO_TARGET_DIR` between two checkouts or worktrees: cargo then
   runs the other checkout's test binaries, which read and write the other checkout's files.
2. **Probe and fill the worksheet.** Use the
   [capability worksheet](../../docs/HARNESS-ONBOARDING.md#1-capability-worksheet) with a
   private `HOME` and harness home under a temp directory, and the exact installed candidate.
   Write `UNKNOWN`, never a guess. STOP on an UNKNOWN in the four Phase 1 facts above.
3. **Decide: do I need a core PR?** Walk every row of [decision-table.md](decision-table.md).
   STOP on any "core PR" or "core decision" row.
4. **Pin the executable.** Hash the regular file the CLI will run, not a symlink:
   ```
   python3 -c 'import os,shutil,sys; print(os.path.realpath(shutil.which(sys.argv[1])))' <program>
   shasum -a 256 <that path>        # or: sha256sum <that path>
   <program> --version
   ```
   Expect: one absolute path; one line `<64 hex>  <path>`; the version. These go into
   `[capture.executable]`. Write each in the readiness record with where it came from.
5. **Write the readiness record.** Create `docs/qualification/m2-<name>-readiness.md` from the
   [handoff record](../../docs/HARNESS-ONBOARDING.md#7-handoff-record) shaped like
   `docs/qualification/m2-amp-readiness.md`.
6. **Write the connector.**
   ```
   mkdir -p connectors/<id>
   cp skills/adding-a-capture-harness/templates/connector-harness.toml connectors/<id>/harness.toml
   ```
   Replace every `<PLACEHOLDER>` and delete unused options. Seeds go in `connectors/<id>/files/`
   and must be named in `seed`; any other file in the folder is refused. Field rules:
   [CONNECTOR-SCHEMA](../../docs/CONNECTOR-SCHEMA.md). Check that no placeholder is left
   outside comments: `grep -nE '^[^#]*<[^>]+>' connectors/<id>/harness.toml`. Expect: no output.
7. **Regenerate the pins.** `sh scripts/pin-manifests.sh`. Expect: no output, exit 0. Then
   `git diff --stat -- crates/*/pins`. Expect: `connector-dirs.txt` with one line added for
   `<id>`. Re-run after every edit under `connectors/<id>/`; never edit a pin by hand.
8. **Run the schema and pin tests.**
   `cargo test -p goalcup-protocol --test it -- harness_conformance:: connector_pins::`.
   Expect: two `test result: ok.` lines. A failure names the field; fix it and go back to 7.
9. **Regenerate the goldens.**
   ```
   GOALCUP_REGEN_HARNESS_GOLDEN=1 cargo test -p goalcup-cli --test full harness_contract_golden::
   GOALCUP_REGEN_HARNESS_GOLDEN=1 cargo test -p goalcup-daemon --test full harness_contract_golden::
   git status --short crates/goalcup-cli/tests/fixtures/harness-golden
   ```
   Expect: both `test result: ok.`; a new `<id>.json`, and changed `enumeration.json` and
   `redaction-extra-patterns.json`. On macOS also a new `<id>.daemon.json` and a changed
   `native-stream-labels.json`. `git diff` must show no change to another connector's values.
   A changed golden that does not mention `<id>` did not change because of your connector:
   `git checkout -- <that file>` and name it in the PR. Set the variable per command as above,
   never `export` it (`check-connector.sh` unsets it, so its golden step always compares).
10. **Add the kit case.**
    ```
    k=crates/goalcup-cli/tests/fixtures/harness-kit/<id>
    mkdir -p $k && cp skills/adding-a-capture-harness/templates/kit-case/* $k/
    ```
    Follow the comments in the three files: the case mirrors the connector's home, identity and
    native shape with a Python stand-in, uses id `kit-<id>`, and copies any seeds into
    `$k/files/`. The baseline cases beside it are `controlled-workload`,
    `synthetic-preassigned`, `synthetic-stderr-token`, `synthetic-keyed-stderr`,
    `synthetic-stdout-json` and `synthetic-keyed-stdout-json`; read the closest. Yours is
    one more case of the same `harness_contract_kit` test. A keyed case names `credential_env`
    in `expect.toml` and copies the connector's `[capture.credential]`: the kit hands the
    stand-in an invented key, never a real one, and checks that no fragment of it leaks.
    `AMP_API_KEY` (D49) and a credential with `stdout_json` identity (D50) are admitted.
    Check: `grep -nE '^[^#]*<[^>]+>' $k/harness.toml $k/expect.toml`. Expect: no output.
    The stand-in itself stops with `replace the placeholder in <NAME>` while one is left.
    Check that `BODY` in `stand-in.py` equals `wire_request` in `expect.toml`:
    ```
    grep -hE '^(BODY|wire_request) = b?"' $k/stand-in.py $k/expect.toml | cut -d\" -f2 | uniq -c
    ```
    Expect: one line, `2` and then the body.
11. **Run the one-command check.** `sh scripts/check-connector.sh <id>`.
    Expect on macOS: last line `check-connector: <id>: 6 passed, 0 failed, 0 skipped`.
    Elsewhere the kit cannot run: add `--allow-no-kit`, expect
    `SKIPPED: harness kit is macOS-only; a maintainer runs it` and
    `5 passed, 0 failed, 1 skipped`, and write in the PR that the kit was not run. A failed
    step prints what to do; a golden failure names the differing files. When the only failure
    is a golden that does not mention `<id>` (step 9), the check is otherwise clean: say so.
12. **Run the gate.** The four commands in [CONTRIBUTING](../../CONTRIBUTING.md#the-gate), in
    order, with Linux's documented exclusions if you are not on macOS.
13. **Review your own PR** against the
    [reviewer checklist](../../docs/CONNECTOR-CONTRIBUTING.md#reviewing-a-connector-pr): every
    field and seed, no base URL, no secrets, one intended pin line.
14. **Open the PR** (one commit is fine; no sign-off needed, D47) holding exactly:
    `connectors/<id>/`, the one new line in `connector-dirs.txt`, the golden JSON from step 9,
    the kit case from step 10, the readiness record and its index row. Check with
    `git status --short`. In the PR body
    ([CONTRIBUTING](../../CONTRIBUTING.md#adding-a-harness)) paste the step 11 summary and name
    anything not run: the kit off macOS, restored platform goldens. Claim "synthetic tested" at
    most. STOP here: a live row is maintainer only
    ([tiny real row](../../docs/CONNECTOR-CONTRIBUTING.md#tiny-real-row)).

## When something fails

| Symptom | Where to look |
|---|---|
| Field or value refused | [CONNECTOR-SCHEMA](../../docs/CONNECTOR-SCHEMA.md), then step 3 |
| `declares id ..., which is not the folder name` | `id` must equal the folder name |
| Pin test fails after an edit | re-run step 7 and commit what it writes |
| Golden mismatch | step 9, then read the JSON diff |
| A kit case refused for a pinned id | the kit id must be `kit-<id>`, not `<id>` |

## Reference

- [templates/](templates/): the connector and kit-case templates used in steps 6 and 10.
- [decision-table.md](decision-table.md): what needs a core PR, with Amp as the worked example.
- [HARNESS-ONBOARDING](../../docs/HARNESS-ONBOARDING.md): worksheet, claims and handoff record.
- [CONNECTOR-CONTRIBUTING](../../docs/CONNECTOR-CONTRIBUTING.md): reviewer checklist, live row.
- [writing-a-decoder](writing-a-decoder.md): a separate verifier PR; capture-only needs none.
- Never set a model base URL, write into a real harness home, or commit `harnesses/`, `target/`
  or `spikes/*/out/` ([AGENTS.md](../../AGENTS.md)).
