# Local test campaign

How a maintainer's test machines run the [event readiness plan](../plans/2026-10-20-event-readiness.md)
measurements (WP-0) and long runs (WP-F): a Mac, Docker on that Mac for Linux, and a WSL2 machine.
One supervising agent runs it; one subagent per cell watches a run and reports back. Read
[AGENTS.md](../../AGENTS.md) and [OPERATING](../OPERATING.md) ("Test runs", "Edge cases by hand") first.

What a campaign proves: that a real harness on a real machine records, uploads and survives the
drills for the length of the cell, measured against pass lines written before the run. What it does
not prove: other platforms, other harness versions, model behaviour, or participant behaviour.

## Rules for every agent in the campaign

- Staging only. Production is refused by the scripts; never work around that.
- Goal Cup Proxy records and never manages the agent. Type the cell's prompt once. Never type "keep
  going", never restart a harness to fill time, never edit a workspace during a run.
- Secrets are files, never arguments, environment values, logs or chat. Never print or copy a
  harness login (`auth.json`, `.credentials.json`, `.env`, Keychain). Never write `~/.codex`,
  `~/.claude`, `~/.hermes` or app-wide config.
- Cheapest settings for every test: Codex `gpt-5.6-luna` at effort `low` (or `minimal` if the
  installed Codex accepts it), never `gpt-6-astra`; Claude Code `haiku` with thinking off. The
  scripts set these in the Goal Cup Proxy harness home before each run and fail the run if a turn used another
  model. A cell that needs a stronger model says so in its config and is approved by the owner.
- No permission prompts in a test cell (nobody may answer one): the scripts also set Codex
  `approval_policy = "never"` and `sandbox_mode = "danger-full-access"`, and Claude Code
  `permissions.defaultMode = "bypassPermissions"` with `skipDangerousModePermissionPrompt`, in the
  same home before each run. This is a test-kit setting, like the cheapest model; the container or
  the owner's test machine is the sandbox. What participants set is their own choice, on Goal
  Cup's recommendation ([results](results/2026-10-22-local-test-day.md),
  [plan v4](results/2026-10-22-fix-plan-v4.md)).
- Raw traces, logs and result packets stay outside the repository. Only aggregates go back to the
  cloud agent or into Git.
- A failed cell is evidence. Keep its log and trace; do not rerun it to get a pass.

## One-time setup

1. **Build tools.** `cargo fetch --locked`, Docker running, `python3`, `curl`, `zstd`, `psql`.
2. **Staging test key.** The staging `TEST_APPROVAL_KEY` in a file only you can read
   (`chmod 600`). Its path goes in `GOALCUP_TEST_APPROVAL_KEY_FILE`. The key is sent to the
   campaign's `server`, so the kit accepts only the staging Worker
   (`https://goalcup-ingest-staging.b-9a5.workers.dev`) and a server on this machine
   (`http://localhost[:port]`, `http://127.0.0.1[:port]`); another `https://` server must be named
   exactly in `MATRIX_ALLOW_SERVER`, and production is never accepted. A server that is neither
   staging nor production needs `MATRIX_FETCH_BUCKET` for a long run's server copy.
   **GitHub login and the project folder.** Every cell is a test run, and a test run asks for its own
   new, empty, public repository at its start (D69). The kit makes one per run (`gh repo create
   <user>/goalcup-kit-<cell>-<random> --public`) with the GitHub login already there, then makes the
   project folder a player makes (a `.gitignore` holding `.env*` and a `.env.local` holding
   `GH_TOKEN=<gh auth token>`, mode 0600, never printed) and runs `goalcup start --test --repo` from
   it. The agent pushes with that token (D71); goalcup passes it no git login. Host cells: this
   machine's login (`gh auth login`, HTTPS, Yes to authenticate Git). Docker cells: the container's,
   made once per cell by `campaign.py login` (it runs `linux-matrix.sh github`, which opens `gh auth
   login` in the container; it stays on the cell's volume). The kit never copies or mounts a login
   or a token. **The agent in a Docker cell can use the GitHub login stored on the cell's volume,
   and the token in its project's `.env.local`, with all of that login's scopes** (it runs as the
   same user, with no sandbox, and `gh` reads the login from the volume's home). That login is the
   owner's own (accepted for this kit); use a login whose scopes you are willing to let an agent
   use. Nothing else secret is in a Docker cell's container:
   the staging key never enters it (the host approves the device codes the container writes down),
   and a Docker cell takes no database URL or R2 token ([OPERATING](../OPERATING.md), the Docker
   cell and "A long run").
   **Before each campaign:** start it on new cell volumes (`docker volume rm
   goalcup-linux-matrix-<connector>-<cell>`, then log in again), because the agent of an earlier
   run could have written into a reused volume's home (its shell startup files, its git and `gh`
   configuration) and the next run would execute that. A fresh volume for every run is the
   strongest setting. The container half runs its own Python isolated (`python3 -I`: no user site,
   no `PYTHON*` variables), so a planted `usercustomize.py` or `.pth` cannot forge a device code;
   a volume reused within one campaign still accepts the remaining route, a planted git or `gh`
   configuration (and shell startup file) that the next run's tools read. And **rotate the staging approval key**
   (`TEST_APPROVAL_KEY`) after any campaign run with an older kit: older kits mounted the key, and
   a database URL and R2 token, into the cell's container, where the agent could read them.
3. **Test members.** Until website migration `20261021100000` is on staging, a test run replaces
   the earlier test run of the same operator, so parallel cells need different members. Staging
   ships two synthetic members (`...0c0de1`, `...0c0de2`). Once the migration is applied, test runs
   no longer replace each other: set `"test_runs_parallel": true` in the campaign `defaults` and
   cells may share one member. To add synthetic members anyway (the database is shared with
   production; synthetic members have no account and production cannot approve them):

   ```sql
   insert into goal_cup_tracer.test_members (member_id, note) values
     ('00000000-0000-4000-8000-0000000c0de3', 'campaign cell 3 (synthetic)'),
     ('00000000-0000-4000-8000-0000000c0de4', 'campaign cell 4 (synthetic)')
   on conflict do nothing;
   insert into goal_cup_tracer.test_operators (member_id, note)
   select member_id, note from goal_cup_tracer.test_members
   where member_id::text like '00000000-0000-4000-8000-0000000c0de%'
   on conflict do nothing;
   ```

4. **No practice repository is needed.** A practice run starts in an empty project folder and
   pushes nothing (D69); the server is told `agentcommunity/goalcup-matrix-empty` for it.
5. **Build once,** before the logins (a build needs neither a host cell's `config_dir`, which its
   login makes, nor a GitHub login). `python3 scripts/campaign.py build CONFIG` builds the Linux image
   and binaries one time. Runs never build, so cells can run side by side. On the Mac, `cargo build --offline
   --workspace` and put `target/debug` first on `PATH`; allow the rebuilt `goalcup` in Little Snitch.

## Cells

A cell is one harness, on one machine kind, for one duration, with one test member and its own
harness sign-in. Copy `scripts/campaign.example.json` and edit it: the key file path, host
`config_dir`s and members. Each run gets a new repository the kit makes; a cell's `repo`
(`<owner>/<name>`) names one instead. The kit never deletes a repository: each `result.json` and the
summary list the ones made (`repo_made`), and the run prints them at the end; delete them with
`gh repo delete <name> --yes`, after `gh auth refresh -s delete_repo` (the default `gh auth
login` lacks that scope, and the delete fails without it); afterwards `gh auth refresh -r delete_repo`
takes the scope off again, because host cells' agents are given that login's token. Nothing writes a token into a cell's configuration; a `github.json`
an older kit left there is no longer read for new runs and can be deleted. The default prompt is [campaign-workload.txt](campaign-workload.txt): a cheap, many-step
coding task that stays in the workspace and stops on its own. An agent that finishes early and idles
is still a valid infrastructure run; note the idle time in the report. Cells of an hour or more use
[campaign-long-workload.txt](campaign-long-workload.txt) instead: the same steps, then items from
its own TODO.md until the deadline, and `goalcup time-left` every hour into TIME.md (the short
workload ends in about 5 minutes with a cheap model, so a 2-hour cell would idle for 115 and a drill
on an idle chain proves little). A long Hermes cell also needs `agent.max_turns` raised in Hermes's
own `config.yaml` (default 90; Hermes stopped there after about 7 minutes on 22 Oct), see
[hermes-cli.md](../harnesses/hermes-cli.md#9-quirks-and-open-questions). Codex effort `minimal` is
refused unless the installed Codex lists it for Luna; use `low` until that is checked. `python3 scripts/campaign.py check CONFIG`
refuses duplicate members among parallel cells, shared config directories, a `repo` two cells name
or the practice repository, production, and an unreadable key file.

Suggested first campaign:

| Cell | Where | Harness | Length | Drills |
|---|---|---|---|---|
| `mac-codex` | Mac host | codex-cli | ladder: 2 h, 8 h, 24 h | Mac sleep, network off (by hand, prompted) |
| `lx-codex` | Docker | codex-cli | same | network cut, pause |
| `lx-claude` | Docker | claude-code-cli | 2 h first | network cut |
| `mac-claude` | Mac host | claude-code-cli | 2 h first | none |
| `wsl-codex` | WSL2 host | codex-cli | 8 h | network off (by hand) |
| `lx-codex-168` | Docker | codex-cli | 168 h from 27 Oct | none; sampler only |

Claude cells start at 2 h: the subscription hit its weekly quota before. Add Hermes only with an
API-key provider the owner approves.

The default workload never pushes, so its repository stays empty and the cell can run again. A
**full-flow cell** tests pushes and the judged commit: use
[campaign-build-workload.txt](campaign-build-workload.txt); each run gets a new repository anyway. The
[local test day](LOCAL-TEST-DAY.md) plans a day of such cells across every surface.

### Every scenario, and how to run it

| Scenario | Command | Needs |
|---|---|---|
| Practice: an empty project folder, no repository | `sh scripts/matrix.sh <connector> <server>` (Docker: `linux-matrix.sh run <id>`) | a signed-in harness (`MATRIX_CONFIG_DIR`), or `m2-fake-tui` |
| Test run with a repository | `python3 scripts/campaign.py run CONFIG`, or `MATRIX_CONFIG_DIR=<dir> sh scripts/matrix.sh <id> <server> --test --duration <s>` | gh signed in (Docker: `linux-matrix.sh github <id>` once per cell) |
| Refusals: folder not empty, `.git` present, `.env.local` without `.gitignore`, repository with commits (also a README-only one), private, bare name | `MATRIX_BIN_DIR=target/kit-test/debug sh scripts/matrix.sh m2-fake-tui <server> --refusals` | a test build (`cargo build --offline -p goalcup-cli --features test-support --target-dir target/kit-test`), no harness |

[OPERATING, The matrix](../OPERATING.md#the-matrix) says what each one checks.

## Logins: one per cell

Codex rotates its refresh token and writes `auth.json` back (measured); a copied login breaks the
original (`refresh_token_reused`). So every cell signs in once in its own home and never shares or
copies it. A Docker cell's login also signs in to GitHub in the container (once, on its volume):

```sh
python3 scripts/campaign.py login CONFIG lx-codex     # Docker: the harness, then gh auth login, in the cell's volume
python3 scripts/campaign.py login CONFIG mac-codex    # host: goalcup login + goalcup setup
```

A Docker cell's login lives on its volume `goalcup-linux-matrix-<connector>-<cell>` and survives
every later run. `docker volume rm` forgets it. Claude Code on Linux stores its login in a place not
yet measured; the first login logs the file names it finds (never contents). Record them in
[claude-code-cli.md](../harnesses/claude-code-cli.md).

## Running

```sh
python3 scripts/campaign.py run CONFIG --parallel 4 --out "$RESULTS/campaign-$(date -u +%Y%m%dT%H%MZ)"
python3 scripts/campaign.py status "$OUT"      # poll; what the supervisor reads
python3 scripts/campaign.py evaluate "$OUT"    # recompute pass lines from the samples
```

A cell of 30 minutes or more is a long run: its matrix row is judged on the server's copy, which the
matrix fetches only when the campaign's environment holds the read-only fetch settings
([OPERATING, A long run](../OPERATING.md#the-matrix)); otherwise the row is `NOT-VERIFIED`.
Each cell writes `run.log`, `samples.csv` and `result.json` under `$OUT/<cell>/`; the campaign writes
`summary.json` and `summary.md`. Parallel limit: CPU (each Docker cell takes up to 6), subscription
quota, and one browser sign-in per new cell.

### Drills

| Drill | Docker cell (automatic) | Host cell (prompted, done by the supervisor or owner) |
|---|---|---|
| Network cut | `docker network disconnect`, reconnect after N s | macOS `networksetup -setairportpower en0 off`; WSL2 from Windows |
| Sleep | `docker pause` / `unpause` (processes frozen, clock runs) | macOS `pmset sleepnow` |
| Server outage | staging `MAINTENANCE=on` (organiser, by hand) | same |
| Reboot, then continue | by hand: `docker kill`, start again, `goalcup continue <run>` | restart the machine, `goalcup continue <run>` |
| Disk full | by hand, privileged loop image only | macOS `hdiutil create -size 6g -fs APFS -volname gcsmall /tmp/gcsmall.dmg`, config on it |

A drill is always undone, even when the campaign is stopped (Ctrl-C reconnects and unpauses).
Host drills print `DRILL-NOW cell=... kind=... do: ...`; the supervisor does it and notes the time.
For the disk-full drill, OPERATING case 4 is the pass line: below 1 GiB free the agent is paused,
above 3 GiB it resumes by itself, and the run never stops for the disk (D65, WP-C). The run before
WP-C stopped silently instead ([T25](t25-disk-full.md)).

## WP-0 measurements (no paid run)

```sh
python3 scripts/research-measure.py "$TRACE_DIR" --json > "$RESULTS/measure-<run>.json"
# test runs; for dry runs use --event goalcup-dryrun-staging
python3 scripts/research-staging.py --event test-staging --json \
  --database-url-file "$SECRETS/staging-db-url" --cloudflare-token-file "$SECRETS/r2-read-token" \
  > "$RESULTS/staging.json"
```

`research-measure.py` reads private chain files and prints only sizes, counts, quiet gaps and
compression ratios. `research-staging.py` reads staging Supabase and R2 read-only
(the database URL and an R2 read token from files you name, `chmod 600`; `CLOUDFLARE_ACCOUNT_ID`
from the environment) and prints
compression ratio, chunk sizes, ACK lag and heartbeat health. Both outputs are safe to paste back.
Also run `gitleaks detect --no-git --source <decoded text dir>` over decoded reports of existing
traces (WP-E urgency), and report counts only.

## Agent roles

**Supervisor (main agent).** Owns the config, the build, the logins (with the owner at the
browser), starting `campaign.py run`, host drills, and the final report. Spawns one cell agent per
running cell. Never opens a harness session itself.

**Cell agent (one per cell).** Reads only its cell's files: `run.log`, `samples.csv`,
`result.json`, and `goalcup status` for its config directory. Every 10 minutes it checks: run still
up, last sample age, ACK lag, disk, RSS trend, any `PROBLEM:` line. It acts only by telling the
supervisor (a host drill is due, a pass line is failing, the run stopped). At the end it writes the
cell report below. It never types into the harness, never seals early, never reruns.

**Cell report** (`$OUT/<cell>/report.md`, also returned to the supervisor):

| Field | Content |
|---|---|
| Cell | id, where, connector, harness version, model and effort seen |
| Outcome | PASS or FAIL, matrix row, how it ended |
| Pass lines | each line: value, threshold, pass, fail or insufficient data |
| Drills | each drill: planned and actual time, effect seen, recovery time |
| Problems | each `PROBLEM:` line and gap, with time |
| Evidence | paths to log, samples, trace (local only) |
| Not covered | what this cell did not test |

The supervisor merges cell reports into one table and sends the cloud agent only aggregates and
pass-line results.

## Pass lines (from the plan, written before the runs)

- No stop before the cell's duration except the planned seal.
- Zero unexplained gaps; every drill gap explained by its drill.
- After a network cut or a pause ends, an acknowledgement covering the last record written before
  its end arrives within 35 minutes (uploads run on a 30-minute timer, plus a margin; samples every
  5 minutes, `sample_every_s: 300`, are enough to judge it).
- RSS slope under 1 MiB per hour after the first hour; open files flat.
- Local chain on disk under 2 GiB beyond the reserve in steady state.
- Server copy equals the local chain (`--check-copy`, and `fetch-run.sh` for one cell per stage). A
  Docker cell takes no database URL or R2 token, so its long runs say `NOT-VERIFIED`; check them on
  the host after the run with `matrix.sh --check-copy` (host cells may be given the read-only
  credentials).

## Cleanup

`sh server/worker/scripts/delete-test-runs.sh` (dry run first) removes staging test runs and their
R2 objects. `docker volume rm goalcup-linux-matrix-<connector>-<cell>` forgets a cell's login. Host
cells: delete the cell's config directory and its `-artifacts` and `-runs` siblings. Delete the test
repositories the run listed (`gh auth refresh -s delete_repo`, then `gh repo delete <name> --yes`,
then `gh auth refresh -r delete_repo` to remove the scope again; the kit never deletes one).
