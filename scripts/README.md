# Scripts

One line per file: what it does, who runs it, and whether CI or the gate runs it. "Gate" is
`sh scripts/gate.sh fast|full` ([README](../README.md#the-gate)). "CI" is `.github/workflows/ci.yml`
on every pull request; "release CI" and "Linux CI" are `release.yml` and the manual
`linux-x86_64.yml`. "By hand" means nothing runs it for you. The script's own header has the
details and the options.

## Checks and the gate

| Script | What it does | Who | Run by |
|---|---|---|---|
| `gate.sh` | The test gate: `fast` before every commit, `full` before a release. | developer | by hand |
| `linux-check.sh` | The Linux half of the gate, in Docker (ext4 and XFS volumes inside). | developer, release | by hand before a release; Linux CI |
| `check-version.py` | Fails when the version is written anywhere but the workspace `Cargo.toml`. | developer | gate (fast and full) |
| `check-links.py` | Checks every relative Markdown link and `#anchor`. | everyone who edits docs | CI; gate (full) |
| `ci-scope.py` | Decides whether a CI run needs the Rust smoke (prints `rust=true` or `rust=false`). | CI | CI |
| `pin-manifests.sh` | Rewrites the connector directory pins. Run it after any edit under `connectors/`. | harness builder, developer | by hand; `check-connector.sh` |
| `check-connector.sh` | Every local check for one connector: files, pins, schema, goldens, kit. | harness builder | by hand |

## Real runs and measurements

| Script | What it does | Who | Run by |
|---|---|---|---|
| `matrix.sh` | One practice run (or `--test --duration` test run) of one connector against one server, and one PASS/FAIL row. | organizer | by hand |
| `linux-matrix.sh` | `matrix.sh` for a real harness on Linux, in Docker, against staging. | organizer | by hand |
| `linux-harness-smoke.sh` | No-login smoke of the measured Linux Codex and Claude Code builds. | developer | Linux CI (`LINUX_CHECK_HARNESSES=1`) |
| `campaign.py` | Runs many long test cells in parallel, with drills, and writes reports ([local campaign](../docs/research/LOCAL-CAMPAIGN.md)). | organizer | by hand |
| `campaign.example.json` | An example campaign config for `campaign.py`. | organizer | `campaign.py` |
| `kit_settle.py` | Settles a test cell's `prep.json` and `github.json` (the kit's GitHub token), the cheapest-model rules and the run's model check. | organizer | `linux-matrix.sh`; `campaign.py` |
| `t25-disk-full.sh` | Measures what a run does on a really full disk (needs `sudo` and a loop image). | developer | by hand |
| `research-measure.py` | Sizes, counts, quiet gaps and compression of a trace's chain files; aggregates only. | researcher, organizer | by hand |
| `research-sample.py` | One CSV row of soak measurements per interval (disk, chain, ACK lag, processes). | organizer | by hand; `campaign.py` |
| `research-staging.py` | Read-only aggregates of one event's uploads: chunk sizes, ACK lag, heartbeat health. | organizer | by hand |
| `research-staging.sql` | The query behind `research-staging.py`; also usable alone in `psql`. | organizer | `research-staging.py` |
| `research-watch.py` | Seals one chosen entry after a time budget (a bounded research run). | researcher | by hand |
| `research-replay.py` | Compares two traces you already have with one verifier ([replay comparison](../docs/research/replay-comparison.md)). | researcher | by hand |
| `fetch-run.sh` | Rebuilds a run's trace from the server's copy so `goalcup-verify` can judge it. Organizer only. | organizer | by hand |

## Releases

| Script | What it does | Who | Run by |
|---|---|---|---|
| `release.sh` | `prepare` opens the version PR; `tag` tags what it merged. | maintainer | by hand |
| `package-release.sh` | Packages one release tarball for one target. | release | release CI |
| `third-party-licenses.py` | Writes the third-party notices a tarball carries. | release | `package-release.sh` |
| `smoke-release.sh` | Smoke-tests one release tarball. | release | release CI |

## Tests of the scripts

Every `test_*.py` file is a unit test of one script, with stubs and no network. The full gate runs
all of them (`python3 -m unittest discover -s scripts`). CI also runs four of them on every pull
request: `test_ci_scope.py`, `test_check_links.py`, `test_check_connector.py` and
`test_codeowners.py`.

| Test | Tests |
|---|---|
| `test_campaign.py` | `campaign.py` |
| `test_check_connector.py` | `check-connector.sh` (also CI) |
| `test_check_links.py` | `check-links.py` (also CI) |
| `test_check_version.py` | `check-version.py` |
| `test_ci_scope.py` | `ci-scope.py` (also CI) |
| `test_codeowners.py` | Every path in `.github/CODEOWNERS` exists (also CI) |
| `test_early_crosscheck.py` | The E02 cross-check in `spikes/m2-first-release/e02/` (F-16) |
| `test_fetch_run.py` | `fetch-run.sh` |
| `test_gate.py` | `gate.sh` |
| `test_install.py` | `packaging/install.sh` |
| `test_linux_check.py` | `linux-check.sh` |
| `test_linux_matrix.py` | `linux-matrix.sh` |
| `test_matrix.py` | `matrix.sh` |
| `test_release.py` | `release.sh` |
| `test_research_measure.py` | `research-measure.py` |
| `test_research_replay.py` | `research-replay.py` |
| `test_research_sample.py` | `research-sample.py` |
| `test_research_staging.py` | `research-staging.py` |
| `test_research_watch.py` | `research-watch.py` |
| `test_third_party_licenses.py` | `third-party-licenses.py` |

TODO (after the event): move the helpers `campaign.py`, `matrix.sh` and `linux-matrix.sh` repeat into a shared library, and split `campaign.py` and `matrix.sh` into smaller files.
