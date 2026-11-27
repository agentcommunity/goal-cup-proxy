---
name: adding-a-capture-harness
description: Use when adding or supporting a CLI coding-agent harness in Goal Cup capture, writing a harness facts sheet under docs/harnesses/, writing connector data under connectors/, or writing the test pad data file for a harness.
---

# Adding a capture harness

Any harness can be added, not only the ones that ship. The test pad is the one way to add it:
you write two small data files, run one command, read one table. No test, no decoder, and no Rust
unless you add a `[usage]` table (step 3).

Rules that never bend:

- A contribution is data. Add no `.rs` file, and write no test.
- Never set a model base URL in a connector. The pad file may hold the provider and model
  settings a participant would write, because it is test data and goalcup never reads it.
- Never write into a real harness home (`~/.claude`, `~/.codex`, `~/.hermes`, ...). The pad
  runs the harness in an isolated home.
- Never put a key or token in the repository, argv, a log, a facts sheet, a PR or chat, and never
  open or print a key file. The pad takes a key name. With `where = "here"` it copies the bare key
  from `~/.pad-keys/<key name>` (mode 0600, outside the checkout), a file the human writes.
- Never commit `pad-out/` or `target/`.

## The five steps

1. **Read the harness's current docs first**, and cite each page with its date. Write what you
   learn in a facts sheet, [templates/harness-facts.md](templates/harness-facts.md) (rules in
   [docs/harnesses/README.md](../../docs/harnesses/README.md)). Then write `connectors/<id>.toml`:
   one flat file, data only. Every key and the rule that refuses it:
   [docs/CONNECTOR-SCHEMA.md](../../docs/CONNECTOR-SCHEMA.md). Copy a neighbour such as
   `connectors/hermes-cli.toml`. A harness PR is that TOML file plus the facts page: `build.rs` globs `connectors/*.toml`, so no `.rs` file or list names it.
   Core code never names a harness, so CI's `check` fails (`rule_code`) when your id or `program`
   is a word core text already uses (`agent`, `node`, `sh`), and (`native_safety`) when your id's
   first word is a word in `crates/goalcup/src/native/*.rs` (`continue-cli` fails on `continue`).
   This must print nothing:
   `grep -riE '(^|[^a-z0-9])<first word of the id>([^a-z0-9]|$)' crates/goalcup/src/native/`.
   Use the harness's own longer alias (Cursor: `cursor-agent`), or say in the PR that it has none.
   Where the harness names its threads in request headers, or writes a session format already read, a
   `[usage]` table says so ([the usage table](../../docs/CONNECTOR-SCHEMA.md#the-usage-table)).
   A version that needs other values gets a `[[versions]]` row (`match`, `say`, and the keys it
   changes; the first match wins): said at start, recorded, never a refusal
   ([Harness versions](../../docs/CONNECTOR-SCHEMA.md#harness-versions)).
2. **Write `scripts/pad/harnesses/<id>.toml`**, from
   [templates/pad-harness.toml](templates/pad-harness.toml): the pinned version and how to
   install exactly that version (the pad records any installed version, so another build runs
   with `here_path` in the run file, a list of folders put first on `PATH`, for a build installed on
   your Mac; `release_versions = ["<v>", ...]` lists the versions a release needs verified), how the key reaches the harness (`login`, `file` or `env`, as
   its docs say), the provider and model config it should start with, what to type to exit, and
   the host of its model calls (`model_host`). Look at `claude-code-cli.toml`,
   `codex-cli.toml` and `hermes-cli.toml` beside it. Launch facts (proxy and CA variables,
   permission prompts, auto-update switches) belong in the connector, not here.
3. **Run it.** You need Python 3.11 or later, `tmux`, `git`, the harness, and the latest `goalcup`
   and `goalcup-verify` installed as a participant installs them
   ([guide](../../docs/PARTICIPANT-GUIDE.md#2-install)). From the checkout's root, copy
   `scripts/pad/runs/micro.toml` to `pad-out/<id>.toml` (`mkdir -p pad-out`; never committed) and set
   `harness = "<id>"`, `key = "<key name>"` (a name in `scripts/pad/pad.toml` `[keys]`),
   `connector = "connectors/<id>.toml"`, `where = "here"`, `prompt = "smoke"` and `minutes = 10`
   (it keeps `server = "off"`; with micro's 3 minutes a slower harness has not committed when the
   run is sealed, and every check but C3 reads INCONCLUSIVE "the harness did no work"). `connector`
   loads your new file into the installed `goalcup`, for setup and start, so nothing is rebuilt.
   Three cases add a line:
   - A host you added to `connectors/model-hosts.toml`: `hosts = "connectors/model-hosts.toml"`.
     Without it the host is tunnelled, not decrypted, and C1 fails.
   - A harness that only signs in (no API key): sign it in once, in a terminal,
     `GOALCUP_HOME=~/gc-signin/<id> goalcup setup --harness connectors/<id>.toml`, which opens it;
     sign in and quit. Then set `key = "none"` and `goalcup_home = "~/gc-signin/<id>"` in the run
     file, and `key_kind = "signin"` in the pad file (`key_kind = "none"` runs nothing).
   - A `[usage]` table: the verifier reads it only from a built-in connector. Commit, build with
     Rust (`cargo build --release --locked -p goalcup -p goalcup-verify`), copy
     `target/release/goalcup` and `target/release/goalcup-verify` into `pad-out/build`, write
     `git rev-parse HEAD` into `pad-out/build/commit`, and set `build = "pad-out/build"`.

   Then run `scripts/pad/pad pad-out/<id>.toml`, and read the table and `report.txt` in the run's
   folder under `pad-out/<id>/`. When goalcup refuses (a setup FAIL, or a start that is
   INCONCLUSIVE "start-refused"), the step's line ends with goalcup's own last line: fix what it
   names. The bar, with the server off: C1, C3 and C6 PASS; C4 says "no local check failed"; C2
   PASS, or INCONCLUSIVE "no thread with a parent" when the harness has no subagents or the
   connector cannot link them (never FAIL); C5 PASS (a FAIL on `coverage.unattributed` means a
   `[usage]` table with `join = "thread"` whose thread is missing from some requests). The row's
   result stays INCONCLUSIVE, because C4 needs the server; that is expected. The record is the
   proof; no log reader is needed.
4. **Optional: a native sample.** If the harness keeps session files and the row says
   `local_log: none`, say so in the pull request. A here run with the server off keeps no record,
   so the sample comes from the maintainer's staging rerun. A reader for the format may later be
   written from it. It adds the local log check and is not needed for a tier.
5. **Hand it to your human, who opens the pull request** and states in it that they read the
   whole diff ([CONTRIBUTING](../../CONTRIBUTING.md#license-and-who-answers-for-a-pr)). Add the
   harness's row to the table in [docs/harnesses/README.md](../../docs/harnesses/README.md), the
   list participants pick from, and give your human `row.json` from the run and the facts sheet.
   A maintainer runs the pad on staging with `mode = "full"` and `case = "all"`. A PASS gives the
   tier. For Edition 01, a connector counts only if it is merged before the cup release tag (see
   [RULES](../../docs/RULES.md#timeline)).

A facts sheet with `Status: facts only` is a complete PR when a key fact is unknown. Stop and say
so rather than guess.
