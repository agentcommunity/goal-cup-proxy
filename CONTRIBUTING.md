# Contributing

How do I send a change to Goal Cup, and what will a maintainer ask for? For outside contributors
and their agents.

Harness connectors are data contributions. Bug fixes, docs and tests are welcome too. Open an issue
first for anything large. Read [AGENTS.md](AGENTS.md) once: its rules bind people and agents alike.

## From fork to pull request

The upstream repository is `https://github.com/agentcommunity/goal-cup-proxy.git`. Every pull
request targets its `main` branch. Fork it under your own account, then:

```
git clone https://github.com/<your-account>/goal-cup-proxy.git
cd goal-cup-proxy
git remote add upstream https://github.com/agentcommunity/goal-cup-proxy.git
git fetch upstream
git switch -c <topic> upstream/main
# edit, then run the gate below
git commit -m "<area>: <what changed>"
git push -u origin <topic>
```

Open a pull request from `<your-account>:<topic>` into `main` and fill in the template. Keep a PR
to one concern. Never paste a credential, token or real transcript into a PR, issue or comment; if
you did, say so privately ([SECURITY.md](SECURITY.md)) and rotate it.

## License and who answers for a PR

Contributions are accepted under the [Apache License 2.0](LICENSE) (section 5). No DCO sign-off is
required (D47). Coding agents are welcome, but a human opens the PR, states in it that they read
the whole diff, and answers review questions. A `Co-Authored-By:` trailer says what an agent wrote.

## The gate

One script, two tiers ([README, The gate](README.md#the-gate)). Every change passes the fast tier;
a release passes the full one:

```
sh scripts/gate.sh fast
sh scripts/gate.sh full   # before a release
```

Fast runs `cargo fmt --check`, clippy with warnings denied, every test under `cargo nextest`, the
doctests, the Worker tests and the Python script and pad tests. Full adds the `#[ignore]` end to
end tests, the release build, and a check that no `GOALCUP_TEST_` text is in the release binary.
It needs `cargo-nextest`, `zstd`, Python 3.11, `git`, `npm` and `npm ci --prefix server/worker`,
names whatever is missing in one line, and refuses to run as root. The last line is
`gate <tier>: PASS in <N>s` or `gate <tier>: FAIL`: report it, the platform and the Rust version
in the PR.

CI runs `sh scripts/gate.sh fast` with no secret. If you cannot run the gate, say so in the PR.

## What `main` requires

- A change reaches `main` only through a pull request. No force pushes, no branch deletion.
- One required status check: `check`, the `ci` workflow's one job, which runs the fast gate on every
  pull request, docs-only ones too. On a first-time fork PR it may show pending until a maintainer
  approves the workflow run (GitHub's default).
- Checks are strict: update your branch by merging upstream `main` into it, and push.
- One approving review. Paths in [`.github/CODEOWNERS`](.github/CODEOWNERS) (connectors, masking,
  the record format, the pad and its skill, packaging, the server, `.github/`, `SECURITY.md`,
  `docs/DECISIONS.md`) need a code owner's approval. A new push dismisses earlier approvals. All
  review conversations are resolved before merging.
- While `@nembal` is the only maintainer, the repository admin role may merge its own PR without a
  second approval (a ruleset bypass), by policy only with `check` passing. That is not independent
  review, and such a PR says so. Outside PRs get no exception.

## Adding a harness

The test pad is the one way to add a harness, and a harness is data: no `.rs` file, no test, no
decoder. [The skill](skills/adding-a-capture-harness/SKILL.md) walks you or your coding agent
through it. In short:

1. Write the facts sheet `docs/harnesses/<id>.md` from
   [`harness-facts.md`](skills/adding-a-capture-harness/templates/harness-facts.md), citing the
   vendor's current docs. If a key fact is unknown, stop there: facts only is a complete PR.
2. Write `connectors/<id>.toml`, one flat file of data
   ([CONNECTOR-SCHEMA](docs/CONNECTOR-SCHEMA.md)).
3. Write `scripts/pad/harnesses/<id>.toml` from
   [`pad-harness.toml`](skills/adding-a-capture-harness/templates/pad-harness.toml).
4. Run `scripts/pad/pad` on a copy of `scripts/pad/runs/micro.toml` until its table passes.
5. Open the PR with the pad's `row.json`. A maintainer reruns the pad on staging; a pass gives the
   tier. A maintainer acknowledges a connector PR within 7 days, which is not a promise to merge.

A new decrypted host is one line in `connectors/model-hosts.toml`. A new rule that connectors
cannot express (D42, D84) is a separate core PR with a `docs/DECISIONS.md` entry. Pad results are
synthetic: they say "recorded in a test", not "works for every user". Never commit a real trace,
a key, `pad-out/` or `target/`, and read any fixture by eye first.

### Reviewing a connector pull request

A connector is trusted data compiled into every build, so a code owner reads each field. Check:

- the connector sets no base URL and no endpoint setting;
- no proxy or CA variable appears outside `ca_env`;
- `env` holds no key, token or user path, and no variable that names a command, hook or host;
- every host added to `connectors/model-hosts.toml` has a vendor doc link, and is a model host,
  not a registry or an unrelated service;
- the pad row is attached, from a run of the pinned version, and its table passes;
- the PR holds no `.rs` file and no real trace.

## Maintainer-owned files

`TODO.md` and `docs/DECISIONS.md` are maintainer-owned. They conflict on every merge, so do not
edit them in an outside PR. Put proposed text in the PR description and a maintainer lands it.

Report vulnerabilities privately ([SECURITY.md](SECURITY.md)). Everyone follows the
[Code of Conduct](CODE_OF_CONDUCT.md).
