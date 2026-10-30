# PRD-12: The test pad and the real-run loop

Id: PRD-12. Version: goalcup 1.0. Status: plan for the overnight build. Date: 30 October 2026.
Base: main at 4b226de2. Index: [PRD-00](PRD-00-index.md). Format: [PRD-01](PRD-01-record-format.md).

## Goal

One command takes a connector and a key, runs real work in a Vercel Sandbox (or on a Mac by hand), and
prints one pass or fail row per harness and case. It is also how a contributor adds a harness.

## Points served

1 (wire calls match the harness's own log), 2 (every subagent recorded), 3 (the harness finishes its task;
the dodger case is relayed untouched), 4 (no secret in the record, the server copy or the pad's own
output), 5 (server copy whole, equal to local, readable), 6 (the counts the participant saw equal the
verifier's recount), 7 (real harnesses, run the way people run them: a terminal, a typed first message,
subagents asked for in the prompt, never forced by config).

## Owner decisions it carries out

- 15 harnesses tested through one test pad: a connector plus a key, one real prompt, then the checks
  "wire calls match the harness's own log, every subagent recorded, no secret, server copy whole"; one
  pass or fail table; the same pad is the contributor path (BRIEF3, 29 Oct evening).
- Three cases: direct key, through a gateway, and a dodger. Same checks for each (BRIEF3, 29 Oct night).
  A harness that ignores `HTTPS_PROXY` or our CA is "not supported", never hacked around.
- Tiers: "verified" (passed on a named version) and "not verified".
- Core set that blocks release: Claude Code, Codex, Hermes. Others may ship "not verified".
- Usage stats are part of the build; the pad checks they are present (BRIEF3, "usage and research stats").
- Watcher agents read failures and fix them; three failed fixes on one cause end that loop with a note.
- Staging Worker for every overnight run, approved by the test route (`TEST_APPROVAL_KEY`); never an
  approval bypass on production; a separate R2 bucket for test runs is allowed (BRIEF3, 30 Oct early).
- 10 USD cap per run. Pad runs last about 20 minutes and do real work: build a payments platform under
  a made-up name, with an admin and a user model, an admin frontend and a user frontend, using many
  subagents in parallel. During the build, tests stay short (BRIEF3, 30 Oct early).
- Keys in 0600 files only; cost is not a constraint for test runs; OpenRouter, Amp and Factory Droid have
  no key this cycle and ship "not verified"; Copilot CLI tries the DeepSeek key.
- Linux sandboxes overnight; the owner runs the pad once on a real Mac after the build.
- The owner's gateway question (relayed request): a gateway is one more HTTPS endpoint on the one host
  list, so the gateway case must pass with the same checks and no harness-specific code (PRD-00, "One
  system for every harness"). The pad proves that per harness.

Where the task text and BRIEF3 differ, BRIEF3 wins: the task says "one real prompt asking for one
subagent"; the 30 Oct decision asks for 20 minutes of real work with many subagents. The pad keeps both
as two prompt profiles: `smoke` (one subagent, about 3 minutes) during the build, `full` for the release
table.

## Today

| What | Where | Lines | Fact |
|---|---|---:|---|
| Practice and test runs of one connector | `scripts/matrix.sh` | 2,522 | One row `<connector> <os>-<arch> <version> PASS\|FAIL\|NOT-VERIFIED <date>` (`:6-10`). Checks are v5: `--capture-integrity`, the server position, `--research-json` "agent worked" (`:23-41`, `:2054-2112`). No subagent check, no harness-log check, no secret scan of the record |
| Approval key handling | `scripts/matrix.sh:64-73`, `:839-887` | | The key is a file path; the file is piped into `curl` stdin as a header at approval time; with `MATRIX_CODES_FILE` the host approves and the key never enters the box the agent runs in |
| Server allowlist | `scripts/matrix.sh:253-258`, `:559-600` | | Only the staging Worker `https://goalcup-ingest-staging.b-9a5.workers.dev` and loopback `http://` are accepted; production needs a flag |
| Environment scrub | `scripts/matrix.sh:889-915` | | Strips SSH agent, git and gh login variables before goalcup runs |
| Typing into a TUI | `scripts/matrix.sh:104-129`, `:1848-1898` | | Prompt typed on one line; bracketed paste when the harness turned it on; Enter sent after `MATRIX_SUBMIT_DELAY` because Codex reads text plus Enter in one write as a paste (`:120-122`); pre-prompt keys for Claude Code's trust dialog (`:123-129`); 30 s ready delay (`:1763`) |
| Token leak scan of the pushed repo | `scripts/matrix.sh:1267-1320` | | Scans every git object for the GitHub token, raw, base64 and percent-encoded |
| Static connector checks | `scripts/check-connector.sh` | 314 | Steps: placeholders and `NODE_USE_ENV_PROXY` (`:8-16`), pins (`:17-19`), schema tests (`:20`), goldens (`:21-26`), kit (`:27-32`). Pins, goldens and the kit are cut in 1.0 |
| Docker and host cells | `scripts/linux-matrix.sh` 876, `scripts/linux-harness-smoke.sh` 242, `scripts/linux-check.sh` 106, `scripts/campaign.py` 1,888 plus `campaign.example.json` 64 | | Wrappers and a batch runner around `matrix.sh` |
| First-run settings | `scripts/kit_settle.py:1-30` | 302 | Writes no-prompt and first-run dialog answers into the harness's isolated home, and checks the model used afterwards |
| Run 1 observer | `final.py` 28, `measure.py` 225, `secrets.py` 23 on `origin/smart/tender-clarke-qupne3:docs/research/results/2026-10-30-two-hour-run/` | | `measure.py` tails the v5 chain (`:64-131`), samples processes (`:169-186`) and writes a 60 s summary (`:187-225`); `final.py` builds the per-thread table from rollouts (`:4-28`); `secrets.py` scans for the key and token and prints hash prefixes only (`:1-23`) |
| Tests of the runners | `scripts/test_matrix.py` 1,517, `test_linux_matrix.py` 1,219, `test_linux_check.py` 107, `test_campaign.py` 1,335, `test_check_connector.py` 411 | 4,589 | Tests of test runners (appendix J, `J-tests-gate.md:261`) |
| Contributor path | `skills/adding-a-capture-harness/SKILL.md` 39, `writing-a-decoder.md` 277, `templates/connector-harness.toml` 209, `templates/harness-facts.md` 74; `docs/HARNESS-ONBOARDING.md` 579 | | Data-only connector, schema tests, goldens, kit; a decoder is a separate verifier change |

What run 1 taught the pad (run report, scratchpad copy `run-report-74daaa03.md`):

| # | Run 1 bug | How it hid | Pad check that must catch it |
|---|---|---|---|
| B1 | Wire recording stopped at 15:43:15Z for good, no GAP; 473 harness calls after it (sections "Root cause" and 4) | The verifier's `cross_leg` said `consistent` | C1 wire against the harness's log |
| B2 | 23 gaps with one time each and `lost_bytes: null` (section 3) | Gaps only made the verdict "incomplete" | C4 record whole (`gap_has_end`) |
| B3 | Finish line said 1,736 recorded; the server stored 1,766; the record held 842 (Verdict 3, section 5) | Counted at selection, not at write | C6 counts the participant saw |
| B4 | 32 native slots for 43 rollout files; 11 subagents with neither leg (section 3) | One unnamed `limit_exceeded` gap | C2 every subagent recorded |
| B5 | The local trace was pruned at the seal; the full trace was never checked or scanned on the box (measures 5 and 8) | The `verify:` line pointed at a pruned folder | C4 needs kept parts; a pruned run is a FAIL, never a skip |
| B6 | The heartbeat never said `failed`; the server row said `gaps` only (section 5) | Nobody read the server's numbers | C4 `server_counts_match` |
| B7 | `provider_errors` read 0 against about 97 rate-limit errors (section 1) | Errors came from the token leg | Shown in every row, from PRD-08 |
| B8 | The observer scanned secrets per segment, on the files left after pruning (`secrets.py:13-22`) | A secret split across segments, or in the pruned trace, was never looked at | C3 joined scan on local parts and server copy |
| B9 | The observer crashed and restarted four times, re-reading from the start (run report, "Observer notes") | Lost tick lines | The pad's watch is append-only and restartable |

## Design

### 1. One command

```
scripts/pad/pad <connector> --key <file> [--case direct|gateway|dodger|all]
                [--prompt smoke|full] [--server staging|local|off] [--where sandbox|here]
                [--gateway-key <file>] [--version <harness version>] [--build <dir>]
```

- `<connector>` is a folder under `connectors/` (PRD-11). `--key` is the harness's own model key file
  (Claude, OpenAI, DeepSeek or Google), 0600, outside the repo. The pad refuses a key file that is not a
  regular file, is group or world readable, or sits inside the checkout (copied rule from
  `scripts/matrix.sh:92-104`, the GitHub token file rule).
- Defaults: `--case all`, `--prompt full`, `--server staging`, `--where sandbox`, the harness version
  pinned in the pad's harness file (section 6), `--build` = `target/release` of the current integration
  commit.
- `--case all` runs direct, gateway and dodger as three runs in three sandboxes, in parallel.
- Exit: 0 when every row is PASS, 1 when any row is FAIL, 3 when any row is INCONCLUSIVE and none FAIL,
  2 on a usage error.
- Output: one folder per run, `pad-out/<connector>/<case>/<UTC stamp>/` (git-ignored), and one line per
  row on stdout. `scripts/pad/table` turns every `row.json` into `PAD-RESULTS.md` (section 9).

Two halves, so that no organizer secret ever enters the box the agent runs in:

| Half | Runs where | Holds |
|---|---|---|
| `pad` (host) | The watcher's machine, or the owner's Mac | Approval key, read-only database and R2 credentials, the gateway key until copied, the known-values file, the spend ledger. Creates and removes sandboxes, approves, fetches, checks, writes the row |
| `pad-run` (box) | Inside the sandbox, or a temp folder on the Mac with `--where here` | The harness, goalcup, goalcup-verify, the harness key and gateway key (0600, deleted at the end), the project folder, the tmux session |

### 2. The three cases

The pad sets the case the way a participant would, inside goalcup's isolated harness home (PRD-11). It
never sets a base URL through goalcup (`AGENTS.md:81-95`, D1, D36); PRD-00's default "Gateway case in the
pad" applies.

| Case | What the pad sets | What must hold beyond the six checks |
|---|---|---|
| `direct` | The vendor key, through the harness's own login or config (section 6) | Every `conn` to the model host is `decrypt`, `listed: shared`. Verifier exit 0 |
| `gateway` | Vercel AI Gateway: the harness's own base URL setting and the gateway key, as Vercel documents it (Claude Code: `ANTHROPIC_BASE_URL` with the `/claude-code` path and `ANTHROPIC_AUTH_TOKEN`, https://vercel.com/docs/ai-gateway/coding-agents/claude-code ; Codex: a custom provider at `https://ai-gateway.vercel.sh/v1` with `wire_api = "responses"`, https://vercel.com/docs/ai-gateway/coding-agents/openai-codex ; Hermes: its built-in `ai-gateway` provider with `AI_GATEWAY_API_KEY`, measured PASS on 28 Oct in `origin/capture/plan-hardening:docs/research/results/2026-10-vercel-gateway.md`, run 4a-2) | `ai-gateway.vercel.sh` is `decrypt` and `listed: shared` with no participant or config entry needed; no `conn` to the vendor's own model host carries a model call; usage rows show `host_kind: gateway` (PRD-09). Verifier exit 0. A harness with no documented gateway route gets `n/a`, never a fake config |
| `dodger` | `direct`, plus a copy of `connectors/model-hosts.toml` with the harness's model host marked `pinned = true`, given to goalcup through the connectors override (ask 4) | The harness finishes; every `conn` to that host is `tunnel` with a GAP `pinned`; zero decrypted model requests to it; native streams are whole; the start line and end screen say "not supported" or "not recorded" (PRD-10 section 6), never "recording works"; usage says "not recorded on the wire". Verifier exit 1 with `wire_keeps_up: named`. Exit 0 or exit 2 is FAIL |

Why the dodger case is simulated: the real dodgers have no key tonight (Amp) or are out (Cursor,
`O-harness-access.md:107-142`). A pinned host is exactly what the proxy does for a real pinning client
(PRD-03 section 12, PRD-03 interfaces row PRD-12), so the same code path runs. Amp, when it has an
account, is run as `direct`: the pad then expects the vendor-backend rows of PRD-03 section 12.

### 3. One run, step by step (`--where sandbox`)

| Time | Host (`pad`) | Box (`pad-run`) |
|---|---|---|
| 0:00 | Check the spend ledger (section 7). `sandbox create --name pad-<id> --snapshot <pad image> --non-persistent --vcpus 4 --timeout 50m` (https://vercel.com/docs/sandbox/cli-reference). `sandbox copy` the two binaries, `scripts/pad/`, the harness key file and, for `gateway`, the gateway key file | |
| 0:01 | | Check the harness version equals the pinned one; refuse otherwise. Make the project folder and, with `--server staging`, a new empty public repo with the GitHub token (as `matrix.sh` `--test`, `:14-18`, `:162-187`), its token in `.env.local` 0600. Plant files (section 5). `goalcup setup --harness <id> --yes` (ask 3). Put the key where the harness's own login puts it, in goalcup's isolated home (section 6). Delete the copied key file |
| 0:02 | | `GOALCUP_SERVER=<staging>`, `GOALCUP_KEEP_PARTS=1`, `GOALCUP_NO_BROWSER=1`; in tmux session `pad`: `goalcup start --test --harness <id> --repo <repo> --prompt prompt.txt`. Write the device code goalcup prints (contract words "shows the code XXXX-XXXX.", PRD-10 line 651) to `code.txt` |
| 0:03 | Read `code.txt`; approve on the staging test route with the key piped from its file (copied from `matrix.sh:839-887`) | |
| 0:04 | | Wait for "Recording in chat mode", then the harness's ready text from the pad harness file (default 30 s). Send pre-prompt keys if the file names any. Paste `runs/<run>/first-message.txt` with `tmux load-buffer` and `tmux paste-buffer -p`, wait the submit delay (default 2 s), send Enter |
| 0:04 to 0:24 | Every 2 min: copy out `watch/*.jsonl`; update the spend ledger | Watch (section 4) |
| 0:24 | | `goalcup seal` at 20 minutes after the paste (`full`), 3 minutes (`smoke`), when the harness exits, or at the spend stop. Save the last screen. Wait until `upload.json` says the server holds the seal's offset (10 minutes at most) |
| 0:27 | `sandbox copy` out `rec/`, `status.json` series, `goalcup.log`, `upload.json`, panes, `harness-files.json`, `watch/`. `sandbox remove pad-<id>` | Wipe keys, `.env.local`, the harness home |
| 0:28 | `fetch-run` the entry from staging (PRD-07); run the checks (section 8); write `row.json` and `report.txt`; scan the pad's own output folder for known values; append to the ledger | |

`--where here` runs both halves on one machine, in a fresh temp folder with a fresh `GOALCUP_HOME`.
goalcup's isolated harness homes keep `~/.claude`, `~/.codex` and `~/.hermes` untouched (PRD-11). It needs
`tmux`, `sqlite3`, `zstd` and the harness installed. This is the Mac path and the contributor path.

`--server local` uses a `wrangler dev` Worker (PRD-07 section 7 `--local`) for phases before staging carries
1.0. `--server off` is local only: C4's server rows are `skip`, so the row can never be PASS, only
"local pass" (contributors without staging access).

### 4. The watch

Append-only, so a restart of the watch loses nothing (B9). Every line has a wall time.

| File | Every | What |
|---|---|---|
| `watch/status.jsonl` | 10 s | `goalcup status --json` (PRD-10 section 5): mode, written, off, losing |
| `watch/live.jsonl` | 5 min | `goalcup-verify check --live --json` result and failed check ids (PRD-08) |
| `watch/procs.jsonl` | 30 s | RSS, CPU, open files and threads of watchdog, recorder and harness processes (copied from `measure.py:169-186`, process names changed) |
| `watch/spend.jsonl` | 2 min | `goalcup-verify usage <entry dir> --live --json` totals priced with `prices.toml` (section 7) |
| `watch/pane-<n>.txt` | 1 min, and at the seal | `tmux capture-pane -p` of the last screen |

An early alarm is not a stop: a `fail` on `wire_keeps_up` or `recorder_alive` in `live.jsonl` is written
in the row as "first seen at T", and the run goes on, so the final checks see the whole shape.

### 5. Prompts and planted files

`scripts/pad/prompts/smoke.txt` (one subagent, about 3 minutes):

```
First run sh ./pad-setup-check.sh and show me its output. Then use one subagent to count the
files in this folder and write the number to count.txt. Wait for it, then commit count.txt.
```

`scripts/pad/prompts/full.txt` (about 20 minutes of real work; the run is sealed at 20 minutes whatever
the state):

```
First run sh ./pad-setup-check.sh and show me its output. Then build Paynest, a payments platform
in the style of PayPal: users with wallets, payments and refunds between users, and an admin role
that can freeze accounts and review disputes. Build the data model and API, an admin web frontend
and a user web frontend, with tests. Plan the work, then use many subagents in parallel: at least
one each for the data model, the API, the admin frontend, the user frontend and the tests. Commit
and push as you go. Keep going until all of it works.
```

The subagent request is plain words in the prompt (point 7: "ask for subagents in the prompt, never force
them by config"). The pad harness file may change the word ("subagent", "sub-agent", "delegate") to the
harness's own term; nothing else.

Planted files (PRD-05 interfaces row PRD-12, PRD-05 section 10), with fresh random values per run, made
on the host and added to the known-values file:

- `.env.local`: the GitHub token (real), plus one fake value per shape in PRD-05 section 10 (an OpenAI-
  shaped key, a GitHub-shaped token, a Google-shaped key, a JWT, a PEM private key) and one 40-byte
  random value with no shape (`PAD_SECRET_PLAIN`).
- `.git/config`: a second remote `https://pad:<random password>@example.invalid/pad.git`.
- `pad-setup-check.sh`: prints `.env.local`, runs `git remote -v` and `env | sort`. `env` in the
  harness's tool shell shows whatever key the harness exports (masked by exact value, PRD-05 section 9).
- `.gitignore` holding `.env*` (the starting kit, `matrix.sh:922-925`).

If the agent never ran the script (no `pad-setup-check` output in the wire or native record), C3 still
runs, and the row says "secret exercise not done". That is INCONCLUSIVE for C3, not PASS.

### 6. Per-harness pad data

One small file per harness, `scripts/pad/harnesses/<connector>.toml`, written by whoever adds the harness.
It is test data, not connector data: it holds what only the pad needs. goalcup never reads it.

```toml
version = "0.165.0"                       # pinned; the row names it
install = "npm i -g @openai/codex@0.165.0"
key_kind = "openai"                       # which --key the harness takes
login = "printenv OPENAI_API_KEY | codex login --with-api-key"   # run with HOME and CODEX_HOME of the isolated home; the key reaches it on stdin only
ready_text = ""                           # text on screen when the harness can take input; empty: wait 30 s
pre_keys = ""                             # printf format, for a first-run dialog (matrix.sh:123-129)
submit_delay = 2
subagent_word = "sub-agent"
[gateway]                                 # absent: the gateway case is n/a
files = { "config.toml" = "gateway/codex-config.toml" }          # copied into the isolated home
key_env_file = ".env"                     # the gateway key goes here as AI_GATEWAY_API_KEY=..., 0600
```

The facts come from `O-harness-access.md` (per-harness recipes, for example H1.7 `:599-616`, H2.7
`:687-702`, H3.7 `:770-789`). Launch facts (CA variables, proxy settings that beat `HTTPS_PROXY`, daemons
to stop, auto-update switches, session-file rows) are PRD-11's connector data, not here. If a pad run fails
because one of them is wrong, the fix goes to PRD-11.

Keys per harness tonight (PRD-00 default "The 15 harnesses", BRIEF3 30 Oct): Claude Code (Claude key),
Codex (OpenAI key), Gemini CLI (Google key), the rest DeepSeek, Copilot CLI DeepSeek through its
bring-your-own-key route (`O-harness-access.md:94`), the OpenAI cheap-model key where a harness takes an
OpenAI-style key and DeepSeek is rate limited. Amp, Factory Droid and any OpenRouter case have no key: the
pad prints `not verified: no key` and runs nothing.

### 7. Keys and spend

Keys:

- On the host, the owner puts each key in its own 0600 file under one folder outside the repo
  (`~/.pad-keys/`). The pad reads a key only to copy its file into a box or to pipe it into `curl`; never
  into a variable, argument, log, prompt or row.
- The approval key, database and R2 credentials never leave the host. The device code comes out of the
  box; the approval goes in from the host (the `MATRIX_CODES_FILE` rule, `matrix.sh:70-73`).
- In the box, the harness key and gateway key exist only as long as the harness's login needs them, then
  only where the harness keeps them (its isolated home). At the end the box is removed. Sandboxes are
  always `--non-persistent`: a stop otherwise snapshots the disk with the keys in it for 30 days
  (`2026-10-vercel-gateway.md`, Round 2 findings).
- The pad image (section 10) is built with no key, ever.
- The known-values file (real keys, the GitHub token, the approval key, every planted value) is made on the
  host, 0600, and given to `goalcup-verify` with `--known`; it is deleted when the row is written.

Spend:

| Guard | Value | What happens |
|---|---|---|
| Per run | 10 USD (owner, 30 Oct). `smoke` 2 USD | `pad-run` seals at 80% of the cap by the live estimate. The row says `stopped_at_cap` with the time; the checks run on what was recorded. Not a failure |
| Per night | `night_cap` in `scripts/pad/pad.toml`. Default: PRD-00's placeholder until PRD-00 changes it (ask 2) | A new run does not start past it. The pad prints one line and the watcher writes a night-log note (PRD-00 autonomy: stop for spend past a cap) |
| Per key at once | at most 2 sandboxes per provider key, 6 sandboxes in all | A queue on the host. Run 1 hit OpenAI's rate limit with about 20 subagents (run report measure 9) |
| Provider side | the owner sets a budget on each key where the vendor offers one | Unverified which vendors enforce a hard stop |

The live estimate prices usage rows from `goalcup-verify usage --live` (PRD-09) with `scripts/pad/prices.toml`
(per provider host and model, as the usage reader names them, copied by the builder from each vendor's
pricing page on the day; an unknown model takes the highest price listed for its host). The ledger
`pad-out/spend.jsonl` has one line per run: estimate, provider, case, time. It is an estimate; the
owner's provider dashboards are the truth.

### 8. The checks and the row

The host runs, for each run:

```
goalcup-verify check --json --known <known> --harness-files harness-files.json <local entry>
goalcup-verify check --json --server <fetched> --local <local entry> --known <known>
goalcup-verify secrets --known <known> <fetched>
goalcup-verify usage --json <fetched>
```

`harness-files.json` is made in the box before the seal's wait ends: every file under the connector's
native roots (PRD-11 S3 rows) changed since the start marker, as `[{"path": "<source>:<relative
path>", "size": n}]` (PRD-08 section 1). The verifier never walks a home folder itself.

The six checks. Each is PASS, FAIL or INCONCLUSIVE. Keys are PRD-08's S6 report and PRD-09's S7 summary.

| # | Check | PASS when | INCONCLUSIVE when | Catches |
|---|---|---|---|---|
| C1 | Wire calls match the harness's own log | `wire_keeps_up` and `threads_match` are `pass` (`named` in the dodger case); usage `readable: true` and `coverage.native_calls` at least 2; the case rows of section 2 hold | usage `readable: false` ("no log reader for this harness yet", the contributor path, section 11); the harness made fewer than 2 calls | B1 |
| C2 | Every subagent recorded | `subagents_present` `pass` with the manifest; usage `threads` has at least one thread with a parent; every thread has a native stream or wire calls | no thread with a parent in the harness's own log (the agent did not spawn one; rerun once, then a prompt note) | B4 |
| C3 | No secret | 0 known-value hits in local parts, the server copy, `status.json`, `goalcup.log`, `upload.json`, the pad's output folder and the pushed repo's git objects (leak scan copied from `matrix.sh:1267-1320`); 0 unexpected shape hits; every planted shaped value has at least one MASK | the setup-check script never ran (section 5) | B8 |
| C4 | Record whole, server copy whole and readable | local `check`: no `fail`; `server_chunks_whole`, `server_equals_local`, `server_counts_match` all `pass`; parts kept (`upload.json` `keep: true`). A `skip` on `server_equals_local` is FAIL, not skip | `--server off` (row is "local pass" at best) | B2, B5, B6 |
| C5 | Usage stats present | usage on the server copy: merged input plus output tokens above 0; every thread has a model and calls above 0; one row per subagent; `unattributed` 0. Dodger: native tokens present, wire none | same as C1 (no reader) | the usage reader silently reading nothing |
| C6 | Counts the participant saw | `status.json` in `mode: sealed`: `written` equals the verifier's `counts.model_requests` and the server row's `model_requests`; `off` equals `off_ms`; the number on the saved end screen is that same number; no `status.jsonl` sample has `written` above the final recount | none | B3 |

Before any check: the run must have done work. The workspace has at least one commit since the start, and
the record has at least one model request or native call (the idea of `matrix.sh:2054-2150`). Otherwise
the row is INCONCLUSIVE "the harness did no work" (a first-run dialog ate the prompt, a login failed).

INCONCLUSIVE causes that are never goalcup's failure, decided first from the record's own `http_resp`
statuses (`O-harness-access.md:280-281`):

| Cause | Sign |
|---|---|
| `provider_down` | Most model responses are 5xx, or the provider's status page shows an incident |
| `provider_refused` | 401 or 403 on the model host (a bad or unfunded key) |
| `sandbox_lost` | The box vanished or timed out before the seal |
| `harness_install` | The pinned version would not install |

Row, `row.json` (sorted keys, no secret, no path outside the run folder):

```json
{"connector":"codex-cli","harness_version":"0.165.0","case":"direct","prompt":"full",
 "where":"sandbox","os":"linux-x86_64","goalcup":"1.0.0+<commit>","server":"staging",
 "run":"r…","entry":"e…","started":"…","minutes":20.1,"result":"PASS",
 "checks":{"C1":"PASS","C2":"PASS","C3":"PASS","C4":"PASS","C5":"PASS","C6":"PASS"},
 "why":[], "facts":{"wire_requests":0,"harness_calls":0,"threads":0,"subagents":0,
  "native_files":0,"gaps":{},"off_ms":0,"provider_errors":{},"secret_hits":0,"masks":{},
  "tokens":{"input":0,"cached":0,"output":0,"reasoning":0},"models":[],
  "raw_bytes":0,"compressed_bytes":0,"recorder_rss_peak_mb":0,"spend_usd_est":0,
  "first_live_alarm":null,"stopped_at_cap":false},
 "pad_trusted_at":"<commit of the last passing selftest>"}
```

`why` holds one plain line per non-PASS check, copied from the verifier's `why`. The row result is PASS
when all six are PASS, FAIL when any is FAIL, otherwise INCONCLUSIVE.

### 9. The table and the tiers

`scripts/pad/table` reads every `row.json` under `pad-out/` and writes
`docs/plans/2026-10-30-v1.0/PAD-RESULTS.md`:

1. A summary: one line per harness: tier, version, the three case results, date.
2. One table per harness: one row per run (newest first), the six checks, the facts that matter (wire
   requests against harness calls, subagents, tokens, gaps, spend), and `why` for anything not PASS.

Tier rule:

- **verified on `<version>`**: the latest `full` run of each applicable case is PASS on that version,
  built from the release commit, with a trusted pad (section 12). `gateway` counts only where a documented
  route exists; `n/a` does not block.
- **not verified**: anything else, with the reason (`no key`, `no log reader`, a failing check, `loop
  stopped after three fixes`).
- **not supported**: the harness ignores `HTTPS_PROXY` or refuses the run CA on its model host in the
  `direct` case, so the direct run looks like a dodger (tunnels with `tls_refused`, or nothing reaching the
  proxy while session files grow). Never hacked around.

The tier lines also go to `pad-out/tiers.json` in the shape PRD-11 needs for the start line of PRD-10
section 6 (ask 5).

### 10. The pad image

`scripts/pad/image` builds one Vercel Sandbox snapshot per night: Ubuntu with `tmux`, `sqlite3`, `zstd`,
`jq`, `git`, `gh`, Node and Python, plus every harness at its pinned version as a non-root user (Claude
Code refuses bypass mode as root, `O-harness-access.md:542`). No key, no goalcup build. Each run starts
from it, so a 20-minute run does not spend 5 minutes installing. The snapshot id is written in every row.
Rebuild it when a pinned version changes. The sandbox's `HOME` is `/vercel` and its user is `ubuntu`
(`2026-10-vercel-gateway.md`, Round 2 facts); the pad uses `$HOME`, never a fixed path.

### 11. The contributor path

The pad is the one way to add a harness (owner decision). `skills/adding-a-capture-harness/SKILL.md` is
rewritten to these steps, about 60 lines:

1. Read the harness's current docs. Write `connectors/<id>/` from PRD-11's schema (data only, no Rust).
2. Write `scripts/pad/harnesses/<id>.toml` (section 6).
3. Run `scripts/pad/pad <id> --key <your key file> --case direct --prompt smoke --where here --server off`.
   Read `report.txt`. Fix the connector until C1 to C3 and C6 pass or are INCONCLUSIVE only for "no log
   reader".
4. If C1 says "no log reader": attach the masked native streams of the run (`goalcup-verify dump --kind
   native <entry>`) to the pull request. A reader is then written in PRD-09's module from that sample
   (PRD-09: "a reader for a session format is written only from a real pad sample"). A contributor may
   write it; it is a code change reviewed as such.
5. Open the pull request with `row.json`. A maintainer runs the full pad on staging, all three cases.
   PASS gives the tier.

`writing-a-decoder.md` (277 lines) goes: decoders are cut (PRD-08 section 6) and the usage reader rule
replaces it. `templates/connector-harness.toml` (209 lines) is replaced by a pointer to PRD-11's example
connector. `templates/harness-facts.md` stays, with a "pad result" section added. A new
`templates/pad-harness.toml` is the section 6 file. `docs/HARNESS-ONBOARDING.md` is PRD-15's to shorten to
a link (ask 7).

### 12. The pad must fail before it is trusted

The pad is trusted at a commit only when `scripts/pad/selftest` turns every known fault into FAIL on the
right check, and a clean run into PASS. It runs at the start of phase 4, again after any change under
`scripts/pad/`, `crates/goalcup-verify/`, or a check threshold, and the row field `pad_trusted_at` names the
commit. A row made by an untrusted pad says so and cannot give a tier.

| Control | Input | Must give |
|---|---|---|
| K1 run 1 itself | Run 1's server copy rewritten to GCR1 by PRD-01's test converter (PRD-01 acceptance 4, PRD-08 ask of PRD-01 7), with run 1's 43 rollout files as the manifest. Only if read-only access to run 1's copy exists; else the night log says so | C1 FAIL (`wire_keeps_up`, 15:43 to 16:54), C2 FAIL (11 children), C4 FAIL (`gap_has_end`, `server_counts_match` 1,766 against the written count), C6 FAIL. `provider_errors` 97 to 99 |
| K2 run 1 shape | PRD-08's run 1 shape builder (`check_run1_shape.rs`), written as a part folder | the same four FAILs |
| K3 silent wire drop, real | Codex `smoke` on a `test-hooks` build with the silent drop hook on the wire at minute 2 (ask 1) | C1 FAIL, first live alarm within 7 minutes; C6 PASS (written counts stay honest) |
| K4 silent native drop, real | Codex `smoke` with the same hook on native files after the first | C2 FAIL |
| K5 secret leak, real | Claude Code `smoke`, plus a value planted in a file outside the workspace that the setup script also prints. PRD-05 section 10: such a value is not masked | C3 FAIL with one known-value hit, in local parts and server copy |
| K6 server tamper | A copy of K7's fetched folder: one flipped byte; one middle chunk removed; `model_requests` in `entry.json` plus one | C4 FAIL for each |
| K7 clean | Claude Code `smoke`, direct, kept parts | PASS on all six |
| K8 pruned | K7 again without `GOALCUP_KEEP_PARTS` | C4 FAIL ("parts pruned"), never PASS or skip |
| K9 count tamper | K7's `status.json` with `written` plus one | C6 FAIL |
| K10 usage gone | K7 with the usage summary emptied | C5 FAIL |

K1, K2, K6, K9 and K10 cost nothing and run in seconds on the host. K3, K4, K5, K7 and K8 are short real
runs, about 2 USD in all.

### 13. The watcher loop

A watcher is an agent (PRD-16 schedules them; one per harness and case). It acts on a row that is FAIL.
INCONCLUSIVE rows are rerun once with no code change; a second INCONCLUSIVE goes in the table as is.

1. **Read.** `row.json`, `report.txt`, the verifier JSON, `watch/`, the panes, `goalcup.log`, then
   `goalcup-verify dump --sid <first failing sid>` on the fetched copy (PRD-08 keeps the first failure's
   offset and `sid`).
2. **Name the cause.** Signature: `<connector>|<case>|<check id>|<first finding code>`. Find the owning PRD:

   | Sign | Look first in |
   |---|---|
   | Wire has the calls, usage does not read them | PRD-09 |
   | Wire lacks the calls; GAPs or a `memory_guard` | PRD-02 |
   | Wire lacks the calls; `tunnel` or `tls_refused` on a listed host | PRD-11 (CA variables, proxy setting), then PRD-03 |
   | Nothing reached the proxy while session files grew | PRD-11 (daemon to stop, setting that beats `HTTPS_PROXY`); else the harness is "not supported" |
   | A rollout or transcript file has no native stream | PRD-04, or PRD-11's rows |
   | Known value or shape in the record | PRD-05 |
   | `server_*` | PRD-06, then PRD-07 |
   | `inv*`, `gap_has_end`, `streams_closed` | PRD-01, PRD-02 |
   | C6 | PRD-02 (counts), PRD-10 (text) |
   | The pad itself (a wrong manifest, a typed prompt split) | PRD-12 |

3. **Reproduce cheap.** Rerun with `--prompt smoke` and the same case. If it does not reproduce, rerun
   `full` once.
4. **Fix.** On the owning PRD's branch, as that PRD's builder, inside its owned paths (PRD-00 merge rule).
   Run that PRD's own tests.
5. **Rebuild.** `cargo build --release -p goalcup -p goalcup-verify` on the build host, one build at a
   time (a lock file in `pad-out/`; the machine has 4 CPUs shared by many agents). The run copies the new
   binaries; the box never builds.
6. **Rerun** the same command, same harness version. Same signature again: one failed fix. A different
   signature: the first cause counts as fixed and a new loop starts.
7. **Stop** after three failed fixes on one signature: a night-log entry (PRD-16) with the three attempts,
   the row stays FAIL, the tier is "not verified". For the core set that is a release blocker in the
   morning hand-off.

Rules that never bend:

- A watcher never makes a check pass by changing the check, a tolerance or the pad's expectation, unless it
  shows with record evidence that the check was wrong. That change goes to the check's owner (PRD-08 or
  PRD-12), is written in the night log as "check changed", and `selftest` must still turn K1 to K10 into
  FAIL before any row is trusted again.
- Never hack around a harness: no base URL from goalcup, no patched harness, no disabled pinning. A harness
  that will not use the proxy is "not supported".
- One cause, one watcher: a lock file per signature in `pad-out/loops/`.

## Files it owns

Create (about 1,350 lines in all, against 6,314 src and 4,589 test lines of the tools it replaces):

| Path | What | Lines (est.) |
|---|---|---:|
| `scripts/pad/pad` | Host half: arguments, key file rules, server allowlist (copied from `matrix.sh:253-258`, `:559-600`), sandbox lifecycle, approval (`:839-887`), fetch, checks, ledger, queue | 300 |
| `scripts/pad/pad-run` | Box half: version check, project and repo, plants, setup, login, tmux start, typing, watch, seal, upload wait, wipe | 250 |
| `scripts/pad/check.py` | Section 8: runs the verifier, maps S6 and S7 to C1 to C6, INCONCLUSIVE causes, `row.json`, `report.txt`, the leak scan of the pushed repo and the output folder | 300 |
| `scripts/pad/selftest` | Section 12 | 120 |
| `scripts/pad/table` | Section 9 | 120 |
| `scripts/pad/image` | Section 10 | 60 |
| `scripts/pad/pad.toml`, `prices.toml` | Caps, parallelism, staging address; prices | 40 |
| `scripts/pad/prompts/smoke.txt`, `full.txt`, `pad-setup-check.sh` | Section 5 | 20 |
| `scripts/pad/harnesses/<connector>.toml` and `gateway/` snippets | Section 6, one per harness (15) | 15 x 15 |
| `scripts/pad/test_pad.py` | Automated tests below | 250 |
| `docs/plans/2026-10-30-v1.0/PAD-RESULTS.md` | Generated by `table` | |
| `skills/adding-a-capture-harness/SKILL.md` (rewrite), `templates/pad-harness.toml` (new), `templates/harness-facts.md` (change) | Section 11 | 100 |

Delete: `skills/adding-a-capture-harness/writing-a-decoder.md`, `templates/connector-harness.toml`.

Must not touch: `crates/**` (PRD-01 to PRD-10), `server/**` and `scripts/fetch-run.sh` (PRD-07),
`connectors/**` including `model-hosts.toml` (PRD-11; the dodger copy is made in the run folder, never in
the checkout), `scripts/gate.sh` and CI (PRD-14), `docs/*.md` (PRD-15), `NIGHT-LOG.md` (PRD-16; watchers
write it in their PRD-16 role), the old scripts (PRD-13 deletes them).

## Interfaces with other PRDs

| PRD | What crosses | Direction |
|---|---|---|
| PRD-01 | `rec/` layout, `zstd -dc --long=27` on parts; the start event's `hosts`, `connector`, `harness_version` (section 11); the run 1 converter for K1 | Used here |
| PRD-02 | `status.json` (mode, written, off, losing); `GOALCUP_KEEP_PARTS`; `test-hooks` build; the silent drop hook (ask 1); proofs R1 (kill test), R2, R4, R6 run on the pad with the pad's harness setup | Used here; proofs run there |
| PRD-03 | Section 12 per-case expectations; proofs 1 to 10 use the pad; the dodger case is the `pinned = true` list copy | Used here |
| PRD-04 | The manifest rule for C2 (PRD-04 interfaces row PRD-12) | Used here |
| PRD-05 | The planted file shapes (section 10) and A1 | Used here |
| PRD-06 | `upload.json` (`keep`, server offset), "seal stored" for the upload wait | Used here |
| PRD-07 | `fetch-run.sh` against staging (read-only, from the host) and `--local`; the staging test approval route `POST /v1/device/approve` (PRD-07 line 89); `entry.json` with the database row | Used here |
| PRD-08 | S6 `check --json`, `--harness-files`, `--known`, `secrets`, `check --live`, `dump`; `check_run1_shape.rs` builder for K2 | Used here |
| PRD-09 | S7 summary and `usage --live`; `readable`, `coverage`, `threads`, `host_kind`; new readers only from pad samples | Used here |
| PRD-10 | `GOALCUP_SERVER`, `GOALCUP_HOME`, `GOALCUP_KEEP_PARTS`, `GOALCUP_NO_BROWSER`; `start --test --repo --prompt` and `first-message.txt`; `seal`; `status --json`; contract words; `setup --yes` (ask 3) | Used here |
| PRD-11 | Connector data and launch plan; native roots for the manifest; the connectors override for the dodger copy (ask 4); the isolated home path; tiers from `tiers.json` (ask 5) | Used here |
| PRD-13 | Deletes the replaced scripts and their tests (below) once `selftest` passes | Done there |
| PRD-14 | `scripts/pad/test_pad.py` in `gate fast`; the `test-hooks` feature | Used here |
| PRD-15 | `docs/HARNESS-ONBOARDING.md` shrinks to a link to the skill; `docs/OPERATING.md` "The matrix" section points to the pad | Written there |
| PRD-16 | Schedules pad phases and watchers; the night log; the morning hand-off reads `PAD-RESULTS.md`; the 2-hour run reuses `pad-run`'s watch | Used there |
| PRD-17 | Lessons that only a real harness shows (Codex WebSocket deflate, Anthropic brotli) are confirmed by the core-set rows | Used there |

## What is cut

Deleted by PRD-13 after `selftest` passes on the integration branch; what the pad needs is copied first
(the functions named in "Today").

| Path | Src lines | Tests that go |
|---|---:|---|
| `scripts/matrix.sh` | 2,522 | `scripts/test_matrix.py` 1,517 |
| `scripts/linux-matrix.sh` | 876 | `scripts/test_linux_matrix.py` 1,219 |
| `scripts/linux-harness-smoke.sh` | 242 | none |
| `scripts/linux-check.sh` | 106 | `scripts/test_linux_check.py` 107 |
| `scripts/check-connector.sh` | 314 | `scripts/test_check_connector.py` 411 |
| `scripts/campaign.py`, `campaign.example.json` | 1,952 | `scripts/test_campaign.py` 1,335 |
| `scripts/kit_settle.py` | 302 | none (its first-run answers move to PRD-11's `no_prompts` data) |
| `skills/adding-a-capture-harness/writing-a-decoder.md`, `templates/connector-harness.toml` | 486 | none |
| **Total** | **6,800** | **4,589** |

## Acceptance

### Real-run proofs (first)

| # | Proof | Pass |
|---|---|---|
| P1 | **The pad fails before it passes.** `scripts/pad/selftest` on the integration commit | K1 to K6, K8 to K10 each FAIL on exactly the named checks; K7 PASS on all six. If K1 cannot run (no read-only access), the night log says so and K2 stands in |
| P2 | **Phase 1a smoke.** As soon as PRD-01, 02, 03 and 05 record one request: Claude Code and Codex, `direct`, `smoke`, `--server off` | C1, C2, C3 and C6 PASS or INCONCLUSIVE only for "no log reader"; C4's local half PASS |
| P3 | **Core set, release.** Claude Code, Codex, Hermes, each `direct`, `gateway`, `dodger`, `full`, staging, trusted pad | Nine PASS rows on named versions (PRD-00 done item 1). Each `direct` row shows subagents above 1, tokens above 0, `provider_errors` listed |
| P4 | **One system for gateways.** The three `gateway` rows of P3 | `ai-gateway.vercel.sh` is `listed: shared` in every `conn`; no connector has a gateway-specific line; wire requests against harness calls within PRD-08's tolerance |
| P5 | **Parallel.** Six sandboxes at once (two harnesses x three cases) | No cross-talk: six distinct runs and entries, each row's known values found nowhere in another run's record; no 429 caused by the pad's own queue rule being ignored |
| P6 | **Provider refusal is not our failure.** Hermes `smoke` with a DeepSeek key file holding a wrong key | Row INCONCLUSIVE `provider_refused`; C4 and C6 still PASS (the 401s are recorded) |
| P7 | **The table.** After phase 5 | `PAD-RESULTS.md` has 15 harnesses, a tier and a reason for each, a named version for each "verified", and `pad_trusted_at` on every row |
| P8 | **Contributor dry run.** A builder who did not write the pad adds Kimi Code (a backup in `O-harness-access.md:314`) with only the skill and the pad, on `--where here --server off` | A row with C1 PASS or "no log reader", and under one hour of work, noted in the night log |
| P9 | **The Mac.** The owner runs `scripts/pad/pad claude-code-cli --key <file> --case direct --where here` on a Mac after the build | PASS, or a named failure the night log turns into a fix; Hermes on macOS may need the keychain for the run CA (`O-harness-access.md:733`) |

### Automated tests that guard them

`scripts/pad/test_pad.py` (Python `unittest`, no network, no harness):

- Mapping: S6 and S7 fixtures (clean, run 1 shape, dodger, pruned, no reader) give the expected C1 to C6
  and row result; a `skip` on `server_equals_local` is FAIL; `readable: false` is INCONCLUSIVE, never PASS.
- INCONCLUSIVE causes: 5xx-heavy, 401 and empty records classify as in section 8.
- Key rules: a key file that is 0644, a symlink, or inside the checkout is refused; no key value appears
  in any argument the pad builds (a fake key and a recorded `subprocess` call list).
- Server allowlist: production refused without the flag; staging and loopback accepted (cases copied
  from `scripts/test_matrix.py`).
- Table: the same rows give the same bytes; tier rules (n/a gateway does not block; untrusted pad gives no
  tier).

## Risks and blind spots

| Risk | What we do |
|---|---|
| A harness's first-run screen changes and eats the prompt | The "did work" gate makes it INCONCLUSIVE, not a false PASS; pane captures show the screen; `pre_keys` and `ready_text` are data |
| The agent does not spawn a subagent, or refuses to print `.env` | C2 or C3 INCONCLUSIVE, rerun once; the prompt word is per harness. A run never passes on a missing exercise |
| False alarms in C1 (a harness writes a call twice, a log format changes) | PRD-08's tolerance and "harness above wire only" rule; P3 must pass before any other harness is judged; a check change follows the rule in section 13 |
| The pad itself has a bug that hides a loss | `selftest` K1 to K10 before trust; `pad_trusted_at` on every row |
| A dodger simulated by a pinned list is not a real dodger | It runs the same proxy path as a real pinning client (PRD-03). A real one (Amp) is run when an account exists; Cursor stays the documented case |
| Typing into a TUI through tmux differs from a person typing | tmux is a real terminal; the paste uses bracketed paste when the harness asked for it. If a harness has it off, a multi-line first message can submit early: the pad then joins lines with spaces as `matrix.sh:109-118` does, and the row says so. Which tmux format shows a pane's bracketed paste state is unverified; the builder checks `tmux list-panes -F` first |
| Vercel Sandbox concurrency and egress limits | Unverified for 6 at once; the queue default is a value in `pad.toml`. The 28 Oct runs showed direct egress and no TLS re-termination (`2026-10-vercel-gateway.md`, Round 2) |
| Keys outlive the run | `--non-persistent`, `sandbox remove`, wipe step, no key in the image; C3 scans the pad's own output |
| Provider outage overnight looks like 15 failures | `provider_down` from recorded statuses first (`O-harness-access.md:266-281`) |
| Spend estimate is wrong (unknown model price, cache misses) | Highest-price fallback; per-run seal at 80%; provider budgets are the backstop; the owner's dashboards are the truth |
| A "verified" version is not the one people run on 7 Nov | The row names the version; connectors turn auto-update off (PRD-11); PRD-16 reruns the core set on 5 Nov |
| Staging shared by parallel runs | Each run has its own run and entry; checks read by entry id; a separate R2 bucket for test runs (owner, 30 Oct) |
| Checks run on the host, which also builds | Checks of a 20-minute run take under a minute (PRD-08 R6 estimate); builds hold a lock |
| Linux only overnight | P9 on the owner's Mac; macOS CA trust for Python harnesses differs |

## Overridable defaults

| Default | Value | Who may change it |
|---|---|---|
| Run length, `full` | 20 minutes after the paste | Owner |
| Run length, `smoke` | 3 minutes | PRD-16 |
| Per-run cap | 10 USD (`full`), 2 USD (`smoke`), seal at 80% | Owner |
| Night cap | PRD-00's value (ask 2) | Owner |
| Parallelism | 6 sandboxes, 2 per provider key | PRD-16 |
| Sandbox | 4 vCPU, 50-minute timeout, `--non-persistent` | PRD-16 |
| Ready delay, submit delay | 30 s, 2 s, per harness override | Harness file author |
| Upload wait after the seal | 10 minutes | PRD-06 after measuring |
| Rerun of INCONCLUSIVE | once | PRD-16 |
| Failed fixes per cause | 3 | Owner |
| Tier needs | every applicable case PASS on `full` with a trusted pad | Owner |

## Builder notes

Order of work:

1. `check.py` and `test_pad.py` against fixture S6 and S7 JSON (S6 stable by H+3, S7 by H+4). Then the
   offline controls K2, K6, K9, K10 in `selftest`. This is the part everything else trusts.
2. `pad-run` and `pad` with `--where here --server off` for Claude Code `smoke`: phase 1a needs it
   (P2). Copy the approval, allowlist and scrub functions from `matrix.sh` with their line references in a
   comment.
3. Sandbox lifecycle and `image`; then `--server local`, then staging, approval and fetch.
4. The three cases for the core set; the harness files for Claude Code, Codex and Hermes first.
5. The real controls K3, K4, K5, K7, K8; mark the pad trusted.
6. `table`, the ledger and the queue; then the other 12 harness files, each written after reading the
   vendor's current docs.
7. The skill and templates.

Traps from the audit:

- Codex reads text and Enter in one write as a paste: send Enter after a delay (`matrix.sh:120-122`).
  Codex's `$CODEX_HOME/.env` overwrites `HTTPS_PROXY`, `NO_PROXY` and `SSL_CERT_FILE` (`O-harness-access.md:127-128`, `:657`):
  the gateway key file may hold only the key line.
- Claude Code asks "do you trust this folder" in every new workspace (`matrix.sh:123-129`) and refuses
  bypass mode as root (`O-harness-access.md:542`). Run as `ubuntu`.
- Hermes: stop any gateway for the same home first (`O-harness-access.md:125-126`); one-shot mode caps
  delegation at 2 children (`O-harness-access.md:713`), so run the chat TUI as people do.
- `sandbox exec` is not a terminal; goalcup refuses a pipe. Run goalcup inside tmux
  (`2026-10-vercel-gateway.md`, Round 2 findings).
- Never compare raw wire attempts with harness calls: warm-ups, interrupts and refused retries are wire
  requests (run report section 4). C1 reads PRD-08's comparison, which already excludes them.
- Never scan or check a pruned folder (B5): `GOALCUP_KEEP_PARTS=1` always; C4 fails without it.
- Never let the watch re-read from the start after a restart (B9): append-only files with times.
- Run 1's observer had no live alarm; the pad records the first `check --live` failure time.

How a watcher proves it in a real run: run `scripts/pad/selftest` and confirm K1 to K10 in its output;
then `scripts/pad/pad codex-cli --key ~/.pad-keys/openai --case direct --prompt full`; during the run
`tail -f pad-out/codex-cli/direct/<stamp>/watch/status.jsonl`; after it, read `row.json` and confirm
`subagents` above 1, `wire_requests` within tolerance of `harness_calls`, `secret_hits` 0, C4 PASS, and
`pad_trusted_at` set; then `scripts/pad/table` and read the Codex table in `PAD-RESULTS.md`.

## Asks of PRD-00 and PRD-01

PRD-00:

1. **Seam S11: the silent drop test hook.** Under PRD-14's `test-hooks` feature, owned by PRD-02:
   `GOALCUP_TEST_SILENT_DROP=wire@<seconds>` stops offering wire copies to the writer with no GAP, and
   `native@<n>` stops reading native files after the first n. Same ask as PRD-08 ask of PRD-00 5, widened
   to native. Stable by H+4. Without it K3 and K4 cannot prove the real path catches run 1's shape.
2. **Update the defaults to the 30 Oct decisions.** "Pad server" becomes the staging Worker with the test
   approval route (local Worker only before staging carries 1.0); per-run cap 10 USD (not 5); the run
   length is 20 minutes of the payments-platform prompt with many subagents (smoke prompt during the
   build); "Server access overnight" names read-only database and R2 credentials on the host only, never
   in a sandbox. Set the night cap: 15 harnesses x 3 cases x up to 10 USD is up to 450 USD, so the 50 USD
   placeholder would stop phase 5 early.
3. **Seam S12 (PRD-10): `goalcup setup --harness <id> --yes`** writes the connector's no-prompt values into
   the isolated home with no terminal and no question, and prints that home's path. The pad needs the
   participant's own path, not a copy of `kit_settle.py`.
4. **Seam S13 (PRD-11): a connectors override.** `GOALCUP_CONNECTORS_DIR` (today's name,
   `matrix.sh:58`, `:906`) or a host-list file path, read by the 1.0 loader in release builds too, so the
   dodger case runs the shipped binary. PRD-03's row says "no code seam"; reading a different list is one.
5. **Ownership of tiers.** PRD-10 prints the tier from connector data; the pad produces it. Either PRD-12
   writes a generated `connectors/tiers.toml` (add it to PRD-12's paths), or PRD-11 copies
   `pad-out/tiers.json` in at merge. Default taken here: PRD-11 copies it.
6. **Phase 4 and 5 timing.** One `full` run takes about 30 minutes end to end; 45 runs serially is about
   22 hours. Phase 5 must run 6 sandboxes at once; put that in the schedule (PRD-16).
7. **PRD-15 note.** `docs/HARNESS-ONBOARDING.md` (579 lines) becomes a link to the skill; `docs/OPERATING.md`
   "The matrix" points to the pad.
8. **Done item 1 and PRD-08 R4 disagree on the dodger.** PRD-08 R4 accepts exit 1 or 2; this PRD requires
   exit 1 with `wire_keeps_up: named`, because exit 2 means some other check failed. Please state exit 1 in
   the definition of done.
9. **`scripts/pad/test_pad.py` in `gate fast`** (PRD-14).

PRD-01:

1. **The host list in force must show a pinned mark.** Section 11's `start` event lists `hosts` by source;
   add each entry's `pinned` flag and a `hosts_sha256` of the list file read. Then a dodger pad run, or a
   participant's edited list, is visible in the record itself, and a judge never mistakes a pinned host for
   a refusal.
2. **`keep` in the start event.** Record `GOALCUP_KEEP_PARTS` as `"keep": true|false`, so C4 can read from
   the record (and the server copy) whether local parts were meant to stay, not only from `upload.json`.
