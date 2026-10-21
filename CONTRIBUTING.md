# Contributing

How do I send a change to Goal Cup, and what will a maintainer ask for? For outside contributors
and their agents.

Harness connectors are data contributions. Bug fixes, docs and tests are welcome too. Open an issue
first for anything large. Read [AGENTS.md](AGENTS.md) once: its rules bind people and agents alike.

## From fork to pull request

The upstream repository is `https://github.com/agentcommunity/goal-cup-tracer.git`. Every pull
request targets its `main` branch. Fork it under your own account, then:

```
git clone https://github.com/<your-account>/goal-cup-tracer.git
cd goal-cup-tracer
git remote add upstream https://github.com/agentcommunity/goal-cup-tracer.git
git fetch upstream
git switch -c <topic> upstream/main
# edit, then run the gate below
git commit -m "<area>: <what changed>"
git push -u origin <topic>
```

Open a pull request from `<your-account>:<topic>` into `agentcommunity/goal-cup-tracer:main` and
fill in the template. Keep a PR to one concern. Do not paste a credential, token or real
transcript anywhere in a PR, issue or comment. If you did by accident, say so privately
([SECURITY.md](SECURITY.md)) and rotate it.

## License and who answers for a PR

Contributions are accepted under the [Apache License 2.0](LICENSE) (section 5). No DCO sign-off is
required (D47). Coding agents are welcome, but a human opens the PR, states in it that they read
the whole diff, and answers review questions. Say which parts an agent wrote. A `Co-Authored-By:`
trailer on the commit is enough.

## The gate

Build prerequisites are in [README, Build](README.md#build). Every change passes the fast gate. A
release passes the full one ([setup and what each tier runs](README.md#the-gate)):

```
sh scripts/gate.sh fast
sh scripts/gate.sh full   # before a release
```

Run `cargo fetch --locked` and install `cargo-nextest --locked` once as setup; project build and
gate commands then run with `--offline`. Report the gate's last line, the platform and the Rust
version in the PR. If you cannot run the gate, run CI's form
`cargo test --offline --workspace --exclude goalcup-daemon --exclude goalcup-cli`,
say so in the PR, and a maintainer runs the gate.

## What `main` requires

- A change reaches `main` only through a pull request. No force pushes, no branch deletion.
- One required status check: `check`, the `ci` workflow's one Linux job. It is a smoke test, not
  the gate: it skips the `goalcup-daemon` and `goalcup-cli` suites, which spawn real process trees
  and hang small runners. A docs-only PR skips the Rust steps. On a first-time fork PR, `check`
  may show pending until a maintainer approves the workflow run (GitHub's default).
- Checks are strict: update your branch by merging upstream `main` into it, or by rebasing it,
  and push.
- One approving review. Paths in [`.github/CODEOWNERS`](.github/CODEOWNERS) (connectors, pins, the
  harness contract, launch, proxy, redaction, packaging, `.github/`, `SECURITY.md`,
  `docs/DECISIONS.md`) need a code owner's approval. A new push dismisses earlier approvals. All
  review conversations are resolved before merging.
- While `@nembal` is the only maintainer, the repository admin role may merge its own PR without a
  second approval (a ruleset bypass), by policy only with `check` passing. That is not independent
  review, and such a PR says so. Direct pushes to `main` stay blocked for everyone. Outside PRs get
  no exception.

## Adding a harness

Start with [Add and test a connector](docs/HARNESS-ONBOARDING.md#quick-path), then give your coding agent the
step-by-step [skill](skills/adding-a-capture-harness/SKILL.md). The reviewer checklist is
[HARNESS-ONBOARDING](docs/HARNESS-ONBOARDING.md#reviewing-a-connector-pr). The current source roster
is the [connector directory](connectors), with [facts sheets](docs/harnesses/README.md). Check first that the harness can be captured at
all ([HARNESS-ONBOARDING, check first](docs/HARNESS-ONBOARDING.md#row-0)).

A data-only harness PR contains:

- `connectors/<id>/harness.toml` and its reviewed `files/**` seeds;
- the pin diff from `sh scripts/pin-manifests.sh`: exactly one line of
  `crates/goalcup-protocol/pins/connector-dirs.txt` per connector you add or edit. Commit what the
  script writes. Never hand-edit or hand-merge it; re-run it on the merged tree;
- the regenerated golden JSON in `crates/goalcup-cli/tests/fixtures/harness-golden/`: the new
  `<id>.json`, and on macOS also `<id>.daemon.json`. No shared golden changes for a new
  connector. The daemon's live golden runs on macOS only;
- the facts sheet `docs/harnesses/<id>.md`, from
  `skills/adding-a-capture-harness/templates/harness-facts.md` ([docs/harnesses](docs/harnesses/README.md));
- no harness-kit case: every connector is chat mode, and the kit runs every pinned connector
  from its `harness.toml`;
- the last lines of `sh scripts/check-connector.sh <id>` and `sh scripts/gate.sh fast`.

Your PR contains no `.rs` file. A connector's arguments, model name and environment variables are
data, held to generic rules (D51). So are a CA variable's name and a `providers` name. A new
policy row (CA mode or name shape, provider host, redaction form, identity carrier,
native-store form or interpreter) is a separate core PR with a
`docs/DECISIONS.md` entry. A semantic decoder is a separate verifier PR.

The kit uses stand-ins and a test-owned proxy. A pass means "synthetic tested", not "works with
the real harness". Record an exact version or hash only when the source or an official artifact
proves it. If transport, native storage, auth or session identity is unknown, stop and record the
blocker instead of guessing. A maintainer acknowledges a connector PR within 7 days. That is not
a promise to merge it.

Connectors are trusted code. A template, environment variable, seed or host entry can run or leak
things on every machine that uses the build. Pins are regenerated in the same PR, so they do not
stop a malicious merge. Every connector PR needs code-owner review against the
[reviewer checklist](docs/HARNESS-ONBOARDING.md#reviewing-a-connector-pr). Never commit real
traces, keys, `target/`, `harnesses/` or `spikes/*/out/`. Redact and review any committed fixture
with `spikes/redact.py`.

## Maintainer-owned files

`TODO.md` and `docs/DECISIONS.md` are maintainer-owned. They conflict on every merge, so do not
edit them in an outside PR. Put proposed text in the PR description and a maintainer lands it.

## Security and conduct

Report vulnerabilities privately, not in an issue: [SECURITY.md](SECURITY.md). Everyone follows
the [Code of Conduct](CODE_OF_CONDUCT.md).
