# Scripts

What is here, who runs it, and whether the gate or CI runs it. "The gate" is
`sh scripts/gate.sh fast|full` ([README](../README.md#the-gate)). CI is `.github/workflows/ci.yml`:
one job, `check`, that runs `sh scripts/gate.sh fast` on every pull request into `main` and every
push to `main` and `v1.0/integration`. "Release CI" is `.github/workflows/release.yml`, which runs
on a tag. "By hand" means nothing runs it for you. The header of each script has its options.

| Script | What it does | Who | Run by |
|---|---|---|---|
| `gate.sh` | `fast`: format check, clippy, the Rust tests and doctests, the Worker tests, the script tests below. `full`: fast, then every `#[ignore]` test except `by_hand_*`, the release build and a check that no test hook is in it. | developer, build box | by hand; CI runs `fast` |
| `fetch-run.sh` | Rebuilds a run's trace from the server's copy (checks each chunk's SHA-256) so `goalcup-verify` can judge it. Needs the organizer's read-only credentials. | organizer | by hand; the pad's `fetch` step |
| `release.sh` | `prepare` opens the version pull request; `tag` runs the pad's release check, then tags what `main` holds. | maintainer | by hand |
| `package-release.sh` | Packs one release tarball (`goalcup`, `goalcup-verify`, licences) for one target. | release | release CI |
| `third-party-licenses.py` | Writes the third-party notices the tarball carries; stops on a licence not allowed. | release | `package-release.sh` |
| `smoke-release.sh` | Smoke-tests one tarball with an empty environment: versions, the shipped reader on the worked example, no test hook. | release | release CI |

## The test pad (`scripts/pad/`)

The pad runs a real harness in a sandbox box against the staging Worker, a local one or none, and
decides whether the record is right. It is also how a harness is added. Nobody but a maintainer
or a harness builder runs it; neither the gate nor CI nor release CI does, and it needs the
sandbox and model keys from the environment (names only in `pad.toml`).

- `pad <settings file>` (a file from `runs/`): walks the steps of one run, a limit on each, always cleans up, prints one table. Exit 0 every row PASS, 1 any FAIL, 3 any INCONCLUSIVE.
- `pad-run <run dir> <step>`: the half that runs inside the box, one step at a time. It prints no key.
- `check.py <run folder>`: reads the JSON a run left, maps it to checks C1 to C6, writes `row.json` and `report.txt`. It never runs `goalcup-verify`.
- `table`: gathers the `row.json` files into `connectors/tiers.toml`, `pad-rows.jsonl` and `PAD-RESULTS.md`. `table --release-check <commit>` exits 0 only when the release rows hold for that commit; `release.sh tag` runs it first.
- `sbx.mjs` is the one place that talks to the sandbox service; `pad.toml`, `harnesses/`, `prompts/`, `runs/` and `image` are its data.

## Tests of the scripts

Unit tests with stubs and no network. The gate runs the first three, so CI does too.

| Test | Tests |
|---|---|
| `test_install.py` | `packaging/install.sh` (the install never breaks a running recording) |
| `test_fetch_run.py` | `fetch-run.sh` |
| `pad/test_pad.py` | The pad: key files, no key in any argument or file, pane replacement, cleanup, `check.py` crash rows (Python 3.11 or later) |
