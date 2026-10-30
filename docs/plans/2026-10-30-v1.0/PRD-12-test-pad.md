# PRD-12: The test pad and the real-run loop

Id: PRD-12. Version: goalcup 1.0. Revision 5, 30 October 2026 (ledger items B3, B9, B10 and B11;
revision 4: R3a, R3b and T4; revision 3: the alignment pass against the ledger; revision 2 and revision 1:
earlier the same day). Base: main at 4b226de2. Index: [PRD-00](PRD-00-index.md)
(revision 4). Format: [PRD-01](PRD-01-record-format.md).
Ledger: [OWNER-DECISIONS.md](OWNER-DECISIONS.md) wins where it differs.

Revision 3 applies the alignment fix lists; revision 4 carries in ledger items R3a, R3b and T4; revision 5
carries in B3, B9, B10 and B11 (the simple rig, its three modes, cleanup, fewer tests, more parallel runs).
"Alignment answers", the last section, says what was done with each. Revision 2's answers to the three earlier reviews are kept as history above it.

## Goal

A small test rig an agent can use without driving it (ledger B9). One command and one small settings file
(harness, key name, case, mode, prompt, steps). The rig walks every step by itself with built-in waits
(start, wait for calls, check the data, crash, wait for recovery, keep going, seal, fetch, check, clean
up), in a Vercel Sandbox (or on a Mac by hand), and prints one plain table. Three modes climb the test
ladder: `micro`, `cycle` and `full` (ledger B3). It is also how a contributor adds a harness.

## Points served

1 (the proxy record shows no missed call: no unexplained gap, and the history check; the harness's own
log is an extra check where it keeps one), 2 (every subagent recorded), 3 (the harness finishes its task;
the dodger case is relayed untouched; fault rows show a recorder crash, a frozen machine or a cut network
do not stop the harness, and `goalcup recover` brings recording back without restarting it), 4 (no secret in the record, the server copy or the pad's own output), 5 (server
copy whole, equal to local, readable), 6 (the counts the participant saw equal the verifier's recount),
7 (real harnesses, run the way people run them: a terminal, a typed first message, subagents asked for
in the prompt, never forced by config).

## Owner decisions it carries out

- **One small rig for agents** (ledger B9): one command, one settings file per run, built-in waits, every
  step walked by the rig, one plain table. An agent sets the settings and reads the table; it never
  drives a step by hand. Before it is trusted it must fail on run 1's known bugs (section 12, `selftest`),
  and a reviewer reads its first outputs by hand to check it tests the right thing (section 12).
- **The test ladder** (ledger B3) is the rig's three modes (section 1a): `micro` (one harness, one key, a
  few calls, local capture first, then one layer at a time), `cycle` (a few minutes per pad: setup, data
  flows, look at the data, crash, stand back up, keep going where it makes sense, seal, server copy,
  clean up) and `full` (20 minutes with the payments prompt, every pad, all in parallel). Every run
  cleans up its sandbox, test repo and local files (section 3.2).
- **Do not overbuild tests** (ledger B10): the rig's real runs are the main proof. `test_pad.py` guards
  only secrets in the pad's own hands (point 4) and the never-stop expectations of the crash rows; the
  check mapping is proven by `selftest`'s controls, not by a second set of unit tests. No test pins
  internal shapes or wording.
- **All the parallelism we can afford** (ledger B11): every rig run is its own sandbox; agents run many
  settings files at once. The only limit of our own is the per-key queue, since rate limits turn rows
  INCONCLUSIVE (section 7).
- 15 harnesses tested through one test pad: a connector plus a key, one real prompt, then the checks
  of ledger T4: no unexplained gap, every subagent recorded, no secret in the record, server copy whole,
  usage stats present, and the history check; one pass or fail table; the same pad is the contributor
  path (ledger T4).
- **The proxy record is the proof** (ledger T4). Every check reads the record, `status.json` and the
  staging server copy. The harness's own session files are an optional extra check, the local log
  check, where the harness keeps them and PRD-09 reads them. A harness with no local log (Hermes run
  through its gateway, Amp) reaches "verified" from the record alone (section 8).
- **No outside API is called to verify** (ledger T4): no provider usage or billing lookup, no provider
  status page, no clone of the pushed repo. The pushed-repo scan reads the project's own git store in
  the box (section 8).
- Three cases: direct key, through a gateway, and a dodger. Same checks for each (ledger T4). A harness
  that ignores `HTTPS_PROXY` or our CA is "not supported", never hacked around (ledger T3).
- Tiers: "verified" (passed on a named version) and "not verified" (ledger T3).
- Core set that blocks release: Claude Code, Codex, Hermes (ledger T8, "1 core OK").
- Definition of done item 1 (PRD-00): direct key, gateway and dodger, all `full`, kept parts, every
  check; Hermes's per-session comparison is now the local log check (ledger T4); the dodger passes by
  showing its loss honestly. Done item 9: the
  HTTP/2 and long-tunnel `--exec` rows (ledger H6, H3).
- Usage stats are part of the build; the pad checks they are present (ledger S2).
- A recorder crash is restarted at once; only a crash loop (3 crashes within 10 minutes, each within
  60 s of its start) goes to pass-through, with retries never more than 5 minutes apart (ledger R3a).
  The `kill-recorder` rows kill 2 minutes apart, so they expect an eager restart and no `null_mode` gap
  (section 3.1). The crash loop itself is PRD-02's proof (`watchdog_null.rs`), not a pad row.
- `goalcup recover` restarts only the recorder after the watchdog died, on the same port with the same
  run certificate, with a gap for the time it was down; the harness never restarts (ledger R3b). The
  pad runs it as the fault `kill-watchdog-recover` (section 3.1, P12). The run certificate key now sits
  in a 0600 file in the run folder during the run, so the pad adds it to the box's known values and C3
  proves it reaches no record, server copy or log.
- Watcher agents read failures and fix them; three failed fixes on one cause end that loop with a note
  (ledger B3).
- Staging Worker for every run, approved by the test route with the staging approval key and the owner's
  member id; never an approval bypass on production, and the pad never sends a run to production
  (ledger B4). Test runs go to the existing staging
  bucket `goalcup-traces-staging`; the owner allowed a separate bucket, PRD-07 does not make one.
- 10 USD cap per run, checked during the run (ledger T6); cost is otherwise not a constraint, so there is
  no whole-build cap. Pad runs last about 20 minutes and do real work: build a payments
  platform under a made-up name, with an admin and a user model, an admin frontend and a user frontend,
  using many subagents in parallel (ledger T5). During the build, runs climb the ladder of section 1a
  (`micro`, then `cycle`); 20-minute `full` runs come at the end (ledger B3).
- Keys in 0600 files only; cost is not a constraint for test runs; OpenRouter, Amp and Factory Droid
  have no key this cycle and ship "not verified: no key"; Copilot CLI tries the DeepSeek key (ledger T2).
- Builds, tests and real runs happen in Vercel Sandboxes; the conductor never compiles, tests or runs
  `goalcup` or `goalcup-verify` (ledger B1). It runs sandbox control, approval, `fetch-run` and the
  pad's own scripts only.
- Masking is ledger M1 as written. Exact-value masking of `.env` and key-file values and the Stripe
  test-key and `whsec_` rules wait for the owner (PRD-00 owner question 3, default no). Until the ledger
  records a yes, the pad plants no `sk_test_`, `whsec_` or canary, and its no-shape planted value is a
  stated loss, reported and not failed (PRD-05 A1).
- Implementers read the vendor's current docs before writing any harness file (ledger T7).
- Linux sandboxes in this build; the owner runs the pad once on a real Mac after it (ledger T9).
- The owner's gateway question (relayed request: "when its gateway isnt it still a httpsproxy just
  different api endpoint? ... ideally we have one system for everything"). Yes. A gateway is one more
  HTTPS host on the one shared list (ledger H4, PRD-00 "One system for every harness"). The `gateway`
  case must pass with the same checks and no harness-specific or gateway-specific code; P4 proves it on
  the core set, and the Kilo gateway row (the owner's own example, PRD-16 A4) proves it beyond them.
  Dodgers "use current in a way": the same proxy, a tunnel, a named gap and the session files.
- Cut by PRD-00 ("Not in this build"): the run 1 converter (so control K1 goes), the silent-drop test
  hook (so controls K3 and K4 go), pricing inside goalcup and `prices.toml` (the pad's own spend check
  stays, section 7), and the contributor dry run P8 (after the merge to main).

Where the task text and the ledger differ, the ledger wins: the task says "one real prompt asking for one
subagent"; ledger T5 asks for 20 minutes of real work with many subagents. The pad keeps three prompt
profiles: `micro` (no subagent, a few calls) for the first micro rung, `smoke` (one subagent, about 3
minutes) for the micro rungs from "subagents" on and for K5, K7 and K8; `full` (the payments prompt) for
`cycle` runs, sealed after a few minutes, and for every `full`, release and tier row.

## Today

| What | Where | Lines | Fact |
|---|---|---:|---|
| Practice and test runs of one connector | `scripts/matrix.sh` | 2,522 | One row `<connector> <os>-<arch> <version> PASS\|FAIL\|NOT-VERIFIED <date>` (`:6-10`). Checks are v5: `--capture-integrity`, the server position, `--research-json` "agent worked" (`:23-41`, `:2054-2112`). No subagent check, no harness-log check, no secret scan of the record |
| Approval key handling | `scripts/matrix.sh:64-73`, `:839-887` | | The key is a file path; the file is piped into `curl` stdin as a header at approval time; with `MATRIX_CODES_FILE` the conductor approves and the key never enters the box the agent runs in |
| Server allowlist | `scripts/matrix.sh:253-258`, `:559-600` | | Only the staging Worker `https://goalcup-ingest-staging.b-9a5.workers.dev` and loopback `http://` are accepted; production needs a flag (checked 30 Oct: `STAGING_SERVER` at `:258`). The pad copies the allowlist without the flag: production is always refused (ledger B4) |
| Key file rule | `scripts/matrix.sh:92-104` | | A token file must be a regular file, mode 0600, outside the checkout; never echoed, never an argument |
| Environment scrub | `scripts/matrix.sh:889-915` | | Strips SSH agent, git and gh login variables before goalcup runs |
| Typing into a TUI | `scripts/matrix.sh:104-129`, `:1848-1898` | | Prompt typed on one line; bracketed paste when the harness turned it on; Enter sent after `MATRIX_SUBMIT_DELAY` because Codex reads text plus Enter in one write as a paste (`:120-122`); pre-prompt keys for Claude Code's trust dialog (`:123-129`); 30 s ready delay (`:1763`) |
| Token leak scan of the pushed repo | `scripts/matrix.sh:1267-1320` | | Scans every git object for the GitHub token, raw, base64 and percent-encoded |
| Static connector checks | `scripts/check-connector.sh` | 314 | Steps: placeholders and `NODE_USE_ENV_PROXY` (`:8-16`), pins (`:17-19`), schema tests (`:20`), goldens (`:21-26`), kit (`:27-32`). Pins, goldens and the kit are cut in 1.0 |
| Docker and host cells | `scripts/linux-matrix.sh` 876, `scripts/linux-harness-smoke.sh` 242, `scripts/linux-check.sh` 106, `scripts/campaign.py` 1,888 plus `campaign.example.json` 64 | | Wrappers and a batch runner around `matrix.sh` |
| First-run settings | `scripts/kit_settle.py:1-30` | 302 | Writes no-prompt and first-run dialog answers into the harness's isolated home, and checks the model used afterwards |
| Run 1 observer | `final.py` 28, `measure.py` 225, `secrets.py` 23 on `origin/smart/tender-clarke-qupne3:docs/research/results/2026-10-30-two-hour-run/` | | `measure.py` tails the v5 chain (`:64-131`), samples processes (`:169-186`) and writes a 60 s summary (`:187-225`); `final.py` builds the per-thread table from rollouts (`:4-28`); `secrets.py` scans for the key and token and prints hash prefixes only (`:1-23`) |
| Tests of the runners | `scripts/test_matrix.py` 1,517, `test_linux_matrix.py` 1,219, `test_linux_check.py` 107, `test_campaign.py` 1,335, `test_check_connector.py` 411 | 4,589 | Tests of test runners (appendix J, `J-tests-gate.md:261`) |
| Contributor path | `skills/adding-a-capture-harness/SKILL.md` 39, `writing-a-decoder.md` 277, `templates/connector-harness.toml` 209, `templates/harness-facts.md` 74 (all under `skills/adding-a-capture-harness/`); `docs/HARNESS-ONBOARDING.md` 579 | | Data-only connector, schema tests, goldens, kit; a decoder is a separate verifier change |
| Gateway runs, 28 Oct | `origin/capture/plan-hardening:docs/research/results/2026-10-vercel-gateway.md` | | Claude Code, Codex (provider `vercel`, key in `$CODEX_HOME/.env`) and Hermes (built-in `ai-gateway`) each recorded through Vercel AI Gateway in a sandbox (`:50-55`). Sandbox facts: user `ubuntu`, `HOME=/vercel`, xfs (`:92-95`); a stop snapshots the disk with keys for 30 days unless `--non-persistent` (`:108-110`) |

What run 1 taught the pad (run report, scratchpad copy `run-report-74daaa03.md`):

| # | Run 1 bug | How it hid | Pad check that must catch it |
|---|---|---|---|
| B1 | Wire recording stopped at 15:43:15Z for good, no GAP; 473 harness calls after it (sections "Root cause" and 4) | The verifier's `cross_leg` said `consistent` | C1: no unexplained gap and the history check, from the record; the harness's log as the extra check where kept |
| B2 | 23 gaps with one time each and `lost_bytes: null` (section 3) | Gaps only made the verdict "incomplete" | C4 record whole (`gap_has_end`) |
| B3 | Finish line said 1,736 recorded; the server stored 1,766; the record held 842 (Verdict 3, section 5) | Counted at selection, not at write | C6 counts the participant saw |
| B4 | 32 native slots for 43 rollout files; 11 subagents with neither leg (section 3) | One unnamed `limit_exceeded` gap | C2 every subagent recorded |
| B5 | The local trace was pruned at the seal; the full trace was never checked or scanned on the box (measures 5 and 8) | The `verify:` line pointed at a pruned folder | C4 needs kept parts; a pruned run is a FAIL, never a skip |
| B6 | The heartbeat never said `failed`; the server row said `gaps` only (section 5) | Nobody read the server's numbers | C4 `server_counts_match` |
| B7 | `provider_errors` read 0 against 95 to 99 rate-limit failures (run report measure 9 and section 4: 95 failed calls, about 97 `error` events, 99 `response.failed`) | Errors came from the token leg | Shown in every row, from PRD-08 |
| B8 | The observer scanned secrets per segment, on the files left after pruning (`secrets.py:13-22`) | A secret split across segments, or in the pruned trace, was never looked at | C3 joined scan on local parts and server copy |
| B9 | The observer crashed and restarted four times, re-reading from the start (run report, "Observer notes") | Lost tick lines | The pad's watch is append-only and restartable |

## Design

### 1. One command, one settings file

```
scripts/pad/pad <settings file>
```

That is the whole interface for a run (ledger B9). An agent writes one small settings file, runs the
command, and reads the one plain table it prints. It never drives a step by hand: each step waits for its
own condition, with a time limit, and the rig goes on by itself (section 1a). Many agents run many
settings files at once, each in its own sandbox (ledger B11).

```toml
# pad-out/settings/codex-cycle.toml
harness = "codex-cli"   # connector id: connectors/<id>.toml and scripts/pad/harnesses/<id>.toml
key = "openai"          # key name: ~/.pad-keys/<name>, written from the environment secret (section 7)
case = "direct"         # direct | gateway | dodger | all
mode = "cycle"          # micro | cycle | full (section 1a); sets every default below
# Optional; a line left out takes the mode's default:
server = "staging"      # off | local | staging
prompt = "full"         # micro | smoke | full | a path to a prompt file
minutes = 8             # seal this many minutes after the paste; the box timeout is minutes plus 30
steps = ["exec h2-parallel.sh@5"]   # added to the mode's own steps, in time order (section 1a)
```

Rarely used keys, all optional: `where` (`sandbox`, the default; `here` for the Mac and the contributor
path), `gateway_key` (a key name), `version` (default: the version pinned in the harness file, section
6), `build` (default: the latest build box output, the binaries the integrator built from the current
integration commit, PRD-00 "Where the build runs"; with `where = "here"`, the `goalcup` and
`goalcup-verify` on `PATH`), `connector` (a connector file from disk, passed to
`goalcup start --connector`, seam S16, the contributor path), `hosts` (a host-list copy, passed to
`goalcup start --hosts`, S16; PRD-11's acceptance 3 passes one with a host removed; the dodger case makes
its own copy, section 2; both work in test and practice runs only, PRD-00 owner question 4), `keep`
(default true; false leaves `GOALCUP_KEEP_PARTS` unset, only for K8 and PRD-06's U5, and such a row can
never give a tier, C4) and `keep_going` (idle minutes before "keep going", default 1). An unknown key is a
usage error, so a typo never runs the wrong test quietly. There is no server address setting: the rig
knows only the staging Worker and loopback, so production is always refused (ledger B4; the allowlist
of `scripts/matrix.sh:253-258`, `:559-600`, without its production flag).

- The key file `~/.pad-keys/<key>` is 0600 and outside the repo; the pad refuses one that is not a
  regular file, is group or world readable, or sits inside the checkout (copied rule from
  `scripts/matrix.sh:92-104`, the GitHub token file rule).
- `case = "all"` runs direct, gateway and dodger as three runs in three sandboxes, in parallel.
- Other PRDs name the old flags. Each maps to one setting:

| Flag in other PRDs | Setting |
|---|---|
| `--case`, `--prompt`, `--minutes`, `--server`, `--where`, `--version`, `--build`, `--connector`, `--hosts`, `--gateway-key` | the key of the same name |
| `--prompt-file <path>` (PRD-16 A1 uses run 1's prompt) | `prompt = "<path>"` |
| `--keep-going <n>` | step `keep-going`, with `keep_going = n` |
| `--kill-recorder 8,10` | steps `kill-recorder@8`, `kill-recorder@10` |
| `--fault <kind>@<m>[+<n>]`, `--exec <m>:<script>`, `--pre <script>` | steps `<kind>@<m>[+<n>]`, `exec <script>@<m>`, `pre <script>` (section 3.1) |
| `--no-keep` | `keep = false` |

- Exit: 0 when every row is PASS, 1 when any row is FAIL, 3 when any row is INCONCLUSIVE and none FAIL,
  2 on a usage error.
- Output: the one plain table on stdout (below), and one folder per run,
  `pad-out/<connector>/<case>/<UTC stamp>/`, holding `table.txt` (the same table), `row.json` and
  `report.txt`. `pad-out/` is git-ignored: the pad's first commit (phase 0) adds `/pad-out/` to
  `.gitignore` before any run writes there. `scripts/pad/table` turns every `row.json` into
  `PAD-RESULTS.md`, `pad-rows.jsonl` and `connectors/tiers.toml` (section 9).
- The other two commands are not for running tests: `scripts/pad/selftest` (section 12) and
  `scripts/pad/table` (section 9).

The table, one line per step, then the result:

```
pad codex-cli direct cycle   run r7f..  entry e2c..  box pad-3a1   12m 40s
step              result  at     saw
setup             ok      0:00   codex <pinned>, goalcup 1.0.0+<commit>
start             ok      0:01   approved, prompt pasted
wait-calls 3      ok      0:02   3 model calls written
look              ok      0:02   calls 3, threads 1, gaps 0, known hits 0
kill-recorder@3   ok      0:03   recorder 4121 killed on an open stream
wait-recovery     ok      0:03   recorder 4188 up in 0.6 s, one recorder_down gap 0.6 s, calls 9 > 7
keep-going        skip    0:06   agent still working
seal              ok      0:08   sealed at 8 min, 41 calls written, off 0.6 s
wait-upload       ok      0:09   final: sealed
fetch             ok      0:10   server copy fetched
check             PASS    0:11   C1 PASS  C2 PASS  C3 PASS  C4 PASS  C5 PASS  C6 PASS
cleanup           ok      0:12   box removed, repo deleted, keys wiped, run folder trimmed
result            PASS           pad-out/codex-cli/direct/20261031T0910Z/
```

A non-PASS check adds one line under `check` with its `why` (section 8).

### 1a. Modes, steps and the ladder

Three modes, one per rung of the test ladder (ledger B3). The mode sets the defaults and the steps; the
settings file's `steps` add to them.

| Mode | For | Defaults | Steps, in order |
|---|---|---|---|
| `micro` | Proving each piece is live while building: one harness, one key, a few calls | `server = "off"`, `prompt = "micro"`, `minutes = 3` | setup, start, wait-calls 2, look, seal, check, cleanup |
| `cycle` | A few minutes per pad, not 20, for every harness and case as soon as its connector lands | `server = "staging"`, `prompt = "full"`, `minutes = 8` | setup, start, wait-calls 3, look, kill-recorder@3, wait-recovery, keep-going (only where the harness file says `keep_going = true`), seal, wait-upload, fetch, check, cleanup |
| `full` | The end: 20 minutes, the payments prompt, every pad, all in parallel, until they pass; the only rows that give a tier | `server = "staging"`, `prompt = "full"`, `minutes = 20` | setup, start, wait-calls 3, look every 5 minutes (the watch, section 4), the settings' steps, seal, wait-upload, fetch, check, cleanup |

`wait-upload` and `fetch` run whenever `server` is not `off`, in any mode. A `cycle` row carries its
crash in `variant`, so it is a fault row and never a tier row (section 9).

The steps and their built-in waits. A step that times out never stops the rig: it writes its line and
the rig goes on to seal, fetch, check and clean up, so every run ends with a table and no box left.

| Step | What the rig does | Waits for | Limit, then |
|---|---|---|---|
| `setup` | Box from the pad image, copies, version check, plants, `goalcup setup`, key route (section 3, 0:00 to 0:01) | | 5 min, then FAIL "setup", cleanup |
| `start` | `goalcup start` in tmux, approval when `server` is not `off`, paste, Enter (section 3, 0:02 to 0:04) | "Recording in chat mode", then the harness's ready text | 3 min, then INCONCLUSIVE "harness did not start" |
| `wait-calls n` | Reads `status.json` every 5 s | `written.model_requests` at least n | 4 min, then INCONCLUSIVE "the harness did no work" |
| `look` | `check --live --json` and `usage --json` on the open entry, under `nice -n 19`; prints calls, threads, gaps and known-value hits so far | | 1 min. A live `fail` or a known-value hit is written as "first seen at T" and the run goes on (section 4) |
| `kill-recorder@m` and the other faults | Section 3.1 | An open response stream, up to 60 s | As section 3.1 |
| `wait-recovery` | Reads `status.json` and the open entry | A new recorder pid, `mode: recording`, one closed `recorder_down` gap, and written model requests up by at least 1 | 3 min, then FAIL "did not stand back up"; the run still seals, fetches and checks |
| `keep-going` | Types "keep going" once, as a participant may (`AGENTS.md:83-84`); goalcup itself never does it (PRD-16 A1) | Pane and wire both idle `keep_going` minutes | Until the seal; "skip: agent still working" if never idle |
| `seal` | `goalcup seal --yes` | `minutes` after the paste, the spend cap (section 7), or the harness's exit | |
| `wait-upload` | Reads `upload.json` | `final: sealed` | 10 min, or 5 min after the last fault ends, whichever is later |
| `fetch` | Conductor: `fetch-run`, copy in (section 3, 0:27) | | 5 min, then C4 FAIL "server copy not fetched" |
| `check` | Section 8 | | |
| `cleanup` | Section 3.2 | | Always runs, also after a failure, a timeout or an interrupt |

The ladder. Climb one rung at a time; when a rung fails, fix it before the next. Each rung runs first on
one harness and one key (Claude Code or Codex), then on the others in parallel. Rungs 2 and 3 build on the
rung before; rungs 4 to 9 each make one change to rung 3's settings, so a failure points at one layer.
A `kill-recorder` step is always followed by `wait-recovery`; the rig adds it.

| Rung | Settings |
|---|---|
| 1 Local capture | `mode = "micro"`, `case = "direct"`, `server = "off"`, `prompt = "micro"` (nothing uploaded) |
| 2 Upload | rung 1 with `server = "staging"` |
| 3 Subagents | rung 2 with `prompt = "smoke"` |
| 4 Recorder crash | rung 3 with `steps = ["kill-recorder@1"]` |
| 5 Watchdog kill and `goalcup recover` | rung 3 with `steps = ["kill-watchdog-recover@1"]`, `minutes = 5` |
| 6 Keep going | rung 3 with `steps = ["keep-going"]` |
| 7 Gateway | rung 3 with `case = "gateway"` |
| 8 Dodger | rung 3 with `case = "dodger"` |
| 9 HTTP/2 | rung 3 with `steps = ["exec h2-parallel.sh@1"]` (its verdict is the watcher's, section 3.1) |
| Cycle | `mode = "cycle"` for every harness with a key and every case, many in parallel |
| Full | `mode = "full"` for every pad, all in parallel, until they pass (PRD-16) |

The repo keeps three example settings files, `scripts/pad/runs/micro.toml`, `cycle.toml` and
`full.toml`; an agent copies one and changes a line.

Two halves, so that no organizer secret ever enters the box the agent runs in:

| Half | Runs where | Holds |
|---|---|---|
| `pad` (conductor) | The conductor (this cloud session, PRD-00 "Where the build runs"), or the owner's Mac | Approval key, member id, `GH_TOKEN`, read-only database and R2 credentials, the key files until copied, the known-values files, the spend ledger. Creates the run's test repo with `GH_TOKEN`, which never enters a box, and deletes it at cleanup; creates and removes sandboxes; approves; fetches; copies the fetched entry and the per-run known-values file into the box; has `pad-run` run the checks there; writes the row from their JSON. It never compiles, tests or runs `goalcup` or `goalcup-verify` (ledger B1) |
| `pad-run` (box) | Inside the sandbox, or a temp folder on the Mac with `where = "here"` | The harness, goalcup, goalcup-verify, the harness key, gateway key and push token (0600, wiped after the seal), the project folder, the tmux session, the fault and exec schedule, the live spend check, and the checks of section 8 |

### 2. The three cases

The pad sets the case the way a participant would, inside goalcup's isolated harness home (PRD-11). It
never sets a base URL through goalcup (`AGENTS.md:81-95`, D1, D36); PRD-00's default "Gateway case in the
pad" applies.

| Case | What the pad sets | What must hold beyond the six checks |
|---|---|---|
| `direct` | The vendor key, through the harness's own login or config (section 6) | Every `conn` to the model host is `decrypt`, `listed: shared`, `kind: provider` (PRD-03 section 12). Verifier exit 0 |
| `gateway` | Vercel AI Gateway: the harness's own base URL setting and the gateway key, as Vercel documents it on the day (ledger T7). Claude Code: `ANTHROPIC_BASE_URL` with the `/claude-code` path and `ANTHROPIC_AUTH_TOKEN`, https://vercel.com/docs/ai-gateway/coding-agents/claude-code . Codex: a custom provider at `https://ai-gateway.vercel.sh/v1` with `wire_api = "responses"`, https://vercel.com/docs/ai-gateway/coding-agents/openai-codex , key in `$CODEX_HOME/.env` (measured 28 Oct, `2026-10-vercel-gateway.md:54`). Hermes: its built-in `ai-gateway` provider with `AI_GATEWAY_API_KEY` (measured 28 Oct, `:55`). Kilo: an OpenAI-compatible provider, https://vercel.com/docs/ai-gateway/coding-agents/kilo-code (PRD-16 A4) | `ai-gateway.vercel.sh` is `decrypt`, `listed: shared`, `kind: gateway`, with no participant or config entry needed; no `conn` to the vendor's own model host carries a model call; usage call rows show `host_kind: gateway` (PRD-09). Verifier exit 0. A harness with no documented gateway route gets `n/a`, never a made-up config |
| `dodger` | `direct`, plus a copy of `connectors/model-hosts.toml` in the run folder with the harness's model host added to `pinned` (PRD-11 section 4), passed with `goalcup start --hosts <copy>` (S16) | The harness finishes; every `conn` to that host is `tunnel` with a GAP `pinned`; zero decrypted model requests to it; native streams are whole; the start event's `hosts` shows the host `pinned` and `hosts_sha256` equals the copy's SHA-256 (PRD-01); the launch block shows `Host list from a file: <path> (not verified).` and the end screen names the host under "Not recorded" (PRD-10 acceptance 3), never "recording works"; usage says "not recorded on the wire". Verifier exit 1, from `listed_hosts_decrypted: named` (PRD-08 R4); `wire_keeps_up` is `named` where it runs and `skip` "no per-call times" for a totals harness (Hermes); the history check is `skip` (no decrypted call to that host). Exit 0 is FAIL (the copy was not used); exit 2 is FAIL |

Why the dodger case is simulated: the real dodgers have no key this cycle (Amp) or are out (Cursor,
`O-harness-access.md:107-142`). A list entry marked `pinned` takes the tunnel path a real pinning client
reaches after refusal learning, but skips the learning itself. That path, with its one refused call, is
proven by PRD-03 acceptance 7 and `proxy_tls.rs`, not by the dodger rows. Amp, when it has an account, is run as `direct`: the pad
then expects the vendor-backend row of PRD-03 section 12. A dodger with no session files on disk still
passes by showing its loss honestly: C1, C2 and C5 read the named `pinned` tunnels and claim no call,
thread or token the record did not see (section 8).

### 3. One run, step by step (`where = "sandbox"`)

The times are a `full` run's. `micro` and `cycle` run the same rows on their shorter clocks, with the
steps of section 1a between 0:04 and the seal.

| Time | Conductor (`pad`) | Box (`pad-run`) |
|---|---|---|
| 0:00 | Check the key queue (section 7). With `--server staging` and a push token: create the empty public repo `<pad owner>/gc-pad-<id>` with `GH_TOKEN` (as `matrix.sh` `--test`, `:14-18`, `:162-187`). `sandbox create --name pad-<id> --snapshot <pad image> --non-persistent --vcpus 4 --timeout <minutes + 30>m` (https://vercel.com/docs/sandbox/cli-reference). `sandbox copy` the two binaries from the build box output, `scripts/pad/`, the harness key file, for `gateway` the gateway key file, and the push token file | |
| 0:01 | | Check the harness version equals the pinned one; refuse otherwise. Make the project folder; with a push token, write it to `.env.local` as `GH_TOKEN=<push token>`, 0600 (PRD-10 start step 2 needs that line). Plant files (section 5). Apply a `fd-limit` fault if scheduled. `goalcup setup --harness <id> --yes` (S15). Run the `--pre` script if given. Give the key to the harness by its `key_route` (section 6): the harness's own login with the key file on stdin, a file in goalcup's isolated home, or the launch shell's environment. Delete each copied key file once its route has used it |
| 0:02 | | `GOALCUP_SERVER=<staging>`, `GOALCUP_KEEP_PARTS=1` (unless `keep = false`), `GOALCUP_NO_BROWSER=1`; in tmux session `pad`: `goalcup start --test --harness <id> [--repo <repo>] --prompt prompt.txt [--connector <file>] [--hosts <copy>]` (`--repo` only with a push token). Write the device code goalcup prints (contract words "shows the code XXXX-XXXX.", PRD-10 section 3 step 4) to `code.txt`. With `server = "off"`: `GOALCUP_SERVER=off`, no repo, no approval |
| 0:03 | Read `code.txt`; approve on the staging test route `POST /v1/device/approve` (PRD-07) with the approval key and the owner's member id, each piped from its own 0600 file (copied from `matrix.sh:839-887`) | |
| 0:04 | | Wait for "Recording in chat mode", then the harness's ready text from the pad harness file (default 30 s). Send pre-prompt keys if the file names any. Paste `runs/<run>/first-message.txt` if goalcup wrote it (PRD-10 section 8), else the prompt file, with `tmux load-buffer` and `tmux paste-buffer -p`; wait the submit delay (default 2 s); send Enter. Start the fault and exec schedule (section 3.1). Copy the run certificate key file (ledger R3b; its name and place are PRD-02's) into `pad-run`'s own 0600 file `known-box`, for C3 |
| 0:04 to 0:24 | Every 2 min: copy out `watch/*.jsonl` and the panes, with known values replaced (section 4). Every 5 min: the spend check (section 7) | Watch (section 4) |
| 0:24 | | `goalcup seal --yes` at `minutes` after the paste, at the spend cap, or when the harness exits. Save the last screen. Wait until `upload.json` says `final: sealed` (PRD-06, S9), up to 10 minutes or until 5 minutes after the last scheduled fault ends, whichever is later. Write `harness-files.json`. Wipe keys, `.env.local` and the harness home |
| 0:27 | `sandbox copy` out `rec/`, `status.json` series, `goalcup.log`, `upload.json`, panes, `harness-files.json`, `watch/`, `exec/` (panes with known values replaced). `scripts/fetch-run.sh --bucket goalcup-traces-staging <entry> <out>` (PRD-07, S13: `<out>/<entry>/rec.gcr.zst` and `fetch.json`). `sandbox copy` the fetched entry and the per-run known-values files in. `sandbox exec` the commands of section 8. Copy out their JSON. Then cleanup (section 3.2) | Append `known-box` to the known-values file. Run the section 8 commands; delete the known-values files and `known-box` after the scan |
| 0:28 | Write `row.json`, `report.txt` and `table.txt` from the JSON; scan the pad's own output folder for every known value, the approval key and the conductor's credentials included; append to the ledger; print the table | |

`where = "here"` runs both halves on one machine, in a fresh temp folder with a fresh `GOALCUP_HOME`.
goalcup's isolated harness homes keep `~/.claude`, `~/.codex` and `~/.hermes` untouched (PRD-11). It needs
`tmux`, `sqlite3`, `zstd` and the harness installed. This is the Mac path and the contributor path. It
never runs on the conductor (ledger B1); during the build it runs only inside a pad box (phase 1a,
builder notes step 2).

`server = "local"` uses a `wrangler dev` Worker (PRD-07 section 7 `--local`) for phases before staging carries
1.0. `server = "off"` is local only: C4's server rows are `skip`, so the row can never be PASS, only
"local pass" (phase 1a, and contributors without staging access).

#### 3.1 Faults and watcher commands

Real runs that prove another PRD's restart, upload or proxy behaviour run on the pad, so the setup,
watch and checks are the same. Each fault is a step in the settings file's `steps`, written
`<kind>@<minute>[+<minutes>]`; a watcher command is `exec <script>@<minute>` and a setup script is
`pre <script>` (the `--exec` and `--pre` of the text below, section 1). No fault needs a `test-hooks` build (PRD-02, PRD-06 and PRD-10 say none of
their real-run proofs does). `pad-run` schedules each fault at its minute after the paste, in a detached
process, and writes what it did with the wall time to `watch/faults.jsonl`.

| Kind | What `pad-run` does | Owner and pass line | What the checks expect |
|---|---|---|---|
| `kill-recorder@m` | Waits from minute m until `status.json` shows an open response stream (up to 60 s, else kills anyway and says so), then `kill -9` of the recorder pid in `status.json` | PRD-02 R1 (Codex, done item 2) and its kill case on Claude Code and Hermes; PRD-06 U4 and U6; PRD-10 acceptance 2 | Verifier exit 1. Exactly one `recorder_down` gap per kill, start before end, each under 2 s (PRD-02 defaults). No `null_mode` gap, and no `status.jsonl` line with `mode: null`: the kills are 2 minutes apart, so each restart is eager and no crash loop forms (ledger R3a). C1 and C2 may be `named` only by those gaps; any other named loss is FAIL |
| `kill-watchdog@m` | `kill -9` of the pid in `rec/watchdog.pid`. At minute m+3 `pad-run` sends the harness file's `exit_keys` and runs no `goalcup seal` | PRD-02 R5 | Seal reason `harness_exit` within 35 s of the harness's exit; checks as a clean run |
| `kill-watchdog-recover@m` | `kill -9` of the pid in `rec/watchdog.pid`. Then `goalcup recover` at once in a new tmux window `pad:recover`, whose shell has the run's `GOALCUP_*` values and none of the harness's proxy or CA variables (the recorder still lives). At m+1, `kill -9` of the recorder pid in `status.json`; nothing restarts it now. Plain `goalcup status` every 2 s for 10 s. At m+2, `goalcup recover` again in `pad:recover`. The harness is never touched. At `minutes`, send `exit_keys`, then SIGTERM to `harness.pid` after 30 s if it still lives; no `goalcup seal` | PRD-10 acceptance 11 (ledger R3b); PRD-02 section 9.1 row "The watchdog, then later the recorder". PRD-10 kills both together; the pad's one-minute split covers that case and the order the ledger names | The first `recover` exits 0 with `Recording is on; there is nothing to recover.` Status shows the line ending `Run: goalcup recover` within 10 s of the recorder kill. The second `recover` exits 0 with `Recording is on again`. `status.jsonl` shows the same harness pid and the same `port` before and after. Exactly one `recorder_down` gap, from the recorder kill to the recovered boot's first record, start before end. Every later `conn` to a listed host is `decrypt` (same run certificate). Written model requests after the recover above 0. Seal reason `harness_exit` (or `deadline`). C1 and C2 `named` only by that gap. If the harness lost its terminal when the watchdog died (PRD-02 section 9.1), status must show the `goalcup continue` line instead; the row is INCONCLUSIVE "harness lost its terminal" and the watcher follows PRD-10 acceptance 11 |
| `kill-both@m` | `kill -9` of watchdog and recorder together; send `exit_keys`, then SIGTERM to `harness.pid` from `status.json` after 30 s if it still lives; then `goalcup continue` in a new tmux window `pad:cont` (or `goalcup upload` for PRD-06 U8, named in the fault: `kill-both@m+upload`). Typing, panes and the watch follow that window | PRD-02 R7, PRD-06 U8 | Old entry sealed `recovered` with an `abandoned` gap; checks run on both entries; named loss only inside the crash window |
| `freeze@m+n` | `SIGSTOP` the whole process tree (watchdog, recorder, harness and its children) for n minutes (default 10), then `SIGCONT` | PRD-06 sleep stand-in; review A coverage gap. Not a clock proof (boot time runs on under `SIGSTOP`, PRD-02) | First heartbeat after `SIGCONT` within 10 s; server catches up within 3 minutes; checks as a clean run |
| `cut-worker@m+n` | Appends `127.0.0.1 <worker host>` to `/etc/hosts` with `sudo` for n minutes (default 10); `getent hosts <worker host>` before, during and after, logged. If the lookup does not change, a local listener that refuses or holds the connection stands in; it never terminates TLS (PRD-06 risks). If neither takes, the row says "not run: cut did not take" | PRD-06 U3, U5 | Checks as a clean run; `upload.json` lines of PRD-06 read from `watch/upload.jsonl` |
| `route-drop@m+n` | `sudo ip route del default` for n minutes (default 1), restored by a detached `sleep; ip route add` so it comes back even if the box's control channel drops | Review A coverage gap (network change on the harness leg); PRD-03 owns the proxy side | The harness recovers and finishes; every later `conn` to a listed host is `decrypt`; PRD-08's `refusal_after_trust` warn is absent; C1 may be `named` only for calls inside the drop |
| `fd-limit@0` | `ulimit -Sn 256` in the shell before `goalcup start` | PRD-02 R4 (with a prompt asking for 20 subagents) | Start event `fd_limit` raised; checks as a clean run |

`--exec <minute>:<script>` runs `scripts/pad/exec/<script>` in the box at that minute, in the
harness's proxy and CA environment: the values of `HTTPS_PROXY`, `NO_PROXY` and the CA variables the
launch plan set (PRD-11), read from `/proc/<harness pid>/environ` and filtered to those names only, never
a key variable. Output goes to `exec/<script>.txt`. The scripts are the commands other PRDs' proofs need
but an agent may not run (review B W1, W2): `h2-parallel.sh` (PRD-03 acceptance 3), `refusal-curl.sh`
(PRD-03 acceptance 7), `long-tunnel.sh` (PRD-03 acceptance 9), `time-left.sh` (PRD-10 acceptance 7).
The owning PRD's pass line decides; the pad only runs them and keeps the output. The watcher who reads
the output and the record against that pass line writes `exec/<script>.verdict` (`PASS` or `FAIL` and
one line why), and the row carries it in `exec` (section 8). `--release-check` reads the `h2-parallel.sh`
and `long-tunnel.sh` verdicts (PRD-00 done item 9).

`--pre <script>` runs `scripts/pad/exec/<script>` in the box once, after `goalcup setup --harness <id>
--yes` and before `goalcup start`, with output in `exec/<script>.txt`. It sets up the cases of PRD-11
acceptance 3 to 5: `hint-local.sh` (writes the acceptance 3 provider tables into `{home}/config.toml`
and makes the `--hosts` copy without `api.openai.com`), `proxy-setting.sh` (writes
`"proxy": "http://127.0.0.1:9"` into Qwen Code's `{home}/settings.json`; that row passes when start
refuses and names the file and key), and `stale-gateway.sh` (starts the daemon named by the pad
harness file's `daemon` value with the isolated home and no proxy or CA variables, and leaves it
running: Hermes's gateway, Cline's hub, OpenCode 2's background service). PRD-11 reads those rows.

A fault row is never a tier row (section 9). Its `row.json` names the faults in `variant`, and its
`result` is PASS only when the six checks hold with the expected named losses above and the owner's
machine-checkable lines hold (gap count and length, exit code, seal reason). Owner lines that need a
human read (for example "the harness log shows no failed call it did not retry") go in `report.txt` for
the watcher.

#### 3.2 Cleanup after every run

`cleanup` is the last step of every mode and always runs: after a PASS, a FAIL, a timed-out step, an
error in the rig, or an interrupt (a trap in `pad`). It does, and prints in its table line:

- Box: `sandbox remove pad-<id>`. Backstops: every box is `--non-persistent` with a timeout of `minutes`
  plus 30, so a rig that dies still leaves no box with keys for long.
- Keys: the harness key, gateway key, push token, `.env.local`, `known-box` and the known-values files
  are already wiped in the box (section 3, 0:24 and 0:27); cleanup checks they are gone before it
  removes the box.
- Test repo: the conductor deletes `<pad owner>/gc-pad-<id>` with `GH_TOKEN`, after the git-object scan.
  If the token may not delete repos (unverified), the line says "repo kept" and a night-log line lists
  it for the owner.
- Local files: with `where = "here"`, the tmux session is killed and the temp folder and its
  `GOALCUP_HOME` removed. On the conductor, a PASS run folder is trimmed to `table.txt`, `row.json`,
  `report.txt` and the verifier and usage JSON; the fetched copy, the copied parts, the panes and
  `watch/` are deleted. A FAIL or INCONCLUSIVE folder is kept whole for the watcher, who deletes it when
  its loop ends (section 13).
- The server copy stays in the staging test bucket `goalcup-traces-staging`: it is the evidence of C4.

A cleanup that fails changes no check result; the table line says what is left, and the night log
lists it.

### 4. The watch

Append-only, so a restart of the watch loses nothing (B9). Every line has a wall time.

| File | Every | What |
|---|---|---|
| `watch/status.jsonl` | 10 s | `goalcup status --json` (PRD-10, S4): mode, written, off, losing |
| `watch/live.jsonl` | 5 min | `goalcup-verify check --live --json` result and failed check ids (PRD-08), run under `nice -n 19` so it does not slow the harness |
| `watch/spend.jsonl` | 5 min | The spend check of section 7: tokens per model from `usage --json` on the open entry, the estimate in USD, and the seal if it reached 10 USD |
| `watch/upload.jsonl` | 1 min | `upload.json` (S9): state, waits, server offset, `keep`, `final` |
| `watch/procs.jsonl` | 30 s | RSS, CPU, open files and threads of watchdog, recorder and harness processes (copied from `measure.py:169-186`, process names changed) |
| `watch/faults.jsonl` | per event | What each fault and exec did, with times (section 3.1) |
| `watch/pane-<n>.txt` | 1 min, and at the seal | `tmux capture-pane -p` of the last screen |

The setup script prints `.env.local` and `env` on the harness's screen, so panes hold secrets. At each
copy-out the conductor replaces every known value, in every form PRD-08 scans (raw, base64, percent,
JSON-escaped), in `watch/pane-*.txt` and the saved last screen with `[known:<n>]` before writing them to
the run folder, and counts them in `facts.screen_known`. A value on screen is not a record leak. C3's
scan of the output folder runs after this and counts any other hit.

An early alarm is not a stop: a `fail` on `wire_keeps_up` or `recorder_alive` in `live.jsonl` is written
in the row as "first seen at T", and the run goes on, so the final checks see the whole shape.

### 5. Prompts and planted files

`scripts/pad/prompts/micro.txt` (no subagent, a few calls; rung 1 of the ladder):

```
First run sh ./pad-setup-check.sh and show me its output. Then count the files in this folder,
write the number to count.txt and commit it.
```

`scripts/pad/prompts/smoke.txt` (one subagent, about 3 minutes):

```
First run sh ./pad-setup-check.sh and show me its output. Then use one subagent to count the
files in this folder and write the number to count.txt. Wait for it, then commit count.txt.
```

`scripts/pad/prompts/full.txt` (about 20 minutes of real work; a `full` run is sealed at 20 minutes and a
`cycle` run at 8, whatever the state):

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

Planted files (PRD-05 section 10 and its interfaces row PRD-12), with fresh random values per run, made
on the conductor and added to the known-values file:

- `.env.local`: the run's push token (real) as `GH_TOKEN=`, when there is one (section 7), plus one fake
  value per shape in PRD-05 section 10 (an OpenAI-shaped key, a GitHub-shaped token, a Google-shaped
  key, a JWT, a PEM private key), and one 40-byte random value with no shape (`PAD_SECRET_PLAIN`).
  In the default build (PRD-00 owner question 3: no) `PAD_SECRET_PLAIN` is not masked: it is kept in a
  separate known-values file, its hits are reported as `facts.plain_hits` and in the night log, and
  they fail nothing (PRD-05 A1). With the owner's yes it joins the main file and a hit is a C3 FAIL.
- Only with the owner's yes to PRD-00 question 3: a `sk_test_` and a `whsec_` value in `.env.local`,
  and in `full` runs a canary: at minute 5 `pad-run` appends a fresh `STRIPE_SECRET_KEY=sk_test_<random>`
  line. Both go in the known-values file. The canary tests the shape rule only, since exact values are
  read once at start (PRD-05 section 9). If the agent never reads the file again, the row says "canary
  not exercised"; that changes no result. A hit is a C3 FAIL (review A finding 12). Without the yes
  none of these is planted.
- `.git/config`: a second remote `https://pad:<random password>@example.invalid/pad.git`.
- `pad-setup-check.sh`: prints `.env.local`, runs `git remote -v` and `env | sort`. `env` in the
  harness's tool shell shows whatever key the harness exports, masked by its shape rule; in the default
  build only the `GH_TOKEN` value is masked by exact bytes (PRD-05 section 9).
- `.gitignore` holding `.env*` (the starting kit, `matrix.sh:922-925`).

If the agent never ran the script (no `pad-setup-check` output in the wire or native record), C3 still
runs, and the row says "secret exercise not done". That is INCONCLUSIVE for C3, not PASS.

### 6. Per-harness pad data

One small file per connector, `scripts/pad/harnesses/<connector id>.toml`, written by whoever adds the
harness. 16 files: OpenCode has two connectors (major versions 1 and 2, PRD-11); the table shows OpenCode
as one harness with both versions in its row. The file is test data, not connector data: it holds what
only the pad needs. goalcup never reads it.

```toml
version = "<pinned version>"              # the row names it
install = "npm i -g @openai/codex@<pinned version>"
key_kind = "openai"                       # which --key the harness takes
key_route = "login"                       # how the key reaches the harness, as its docs say: "login", "file" or "env"
login = "codex login --with-api-key"      # "login": run with HOME and CODEX_HOME of the isolated home; the key file goes to stdin, never an argument
key_file = ""                             # "file": a path in the isolated home, e.g. "{home}/.gemini/.env"; written 0600
key_line = ""                             # "file": the line written there, e.g. "GEMINI_API_KEY={key}"
key_env = ""                              # "env": e.g. "ANTHROPIC_API_KEY"; the launch shell runs export NAME="$(cat <file>)" just before goalcup start, never echoed, never in argv, a log or a row
ready_text = ""                           # text on screen when the harness can take input; empty: wait 30 s
pre_keys = ""                             # printf format, for a first-run dialog (matrix.sh:123-129)
exit_keys = "/exit\r"                    # printf format that ends the harness (kill-watchdog, kill-both)
daemon = ""                               # command stale-gateway.sh starts, e.g. "hermes gateway"
submit_delay = 2
subagent_word = "sub-agent"
keep_going = true                         # cycle mode types "keep going" for this harness (a few, where it makes sense)
[direct]
files = { }                               # provider and model config copied into the isolated home, as a participant writes it
env = { }                                 # non-secret values exported in the launch shell before goalcup start, as a participant does
[gateway]                                 # absent: the gateway case is n/a
files = { "config.toml" = "gateway/codex-config.toml" }          # copied into the isolated home
env = { }                                 # non-secret values only
key_route = "file"                        # same three routes for the gateway key
key_file = "{home}/.env"
key_line = "AI_GATEWAY_API_KEY={key}"     # 0600, only this line
```

Provider, model and base URL settings are pad data here, never connector data: goalcup never sets a base
URL (AGENTS.md hard rule 1, ledger section 7). The facts come from `O-harness-access.md` (per-harness
recipes, for example H1.7 `:599-616`, H2.7 `:687-702`, H3.7 `:770-789`) and the vendor's current docs on
the day (ledger T7): the builder reads each harness's documented key route before writing its file.
Launch facts (CA variables, proxy settings that beat `HTTPS_PROXY`, daemons to stop, auto-update
switches, no-prompt values, session-file rows) are PRD-11's connector data, not here. If a pad run fails
on a launch fact, the fix goes to PRD-11.

Keys per harness (ledger T2): Claude Code (Claude key), Codex (OpenAI key), Gemini CLI (the funded
Google key), the rest DeepSeek, Copilot CLI DeepSeek through its bring-your-own-key route
(`O-harness-access.md:94`), the OpenAI cheap-model key where a harness takes an OpenAI-style key and
DeepSeek is rate limited. Amp, Factory Droid and any OpenRouter case have no key: the pad prints
`not verified: no key` and runs nothing.

### 7. Keys and spend

Keys:

- On the conductor, `pad` writes each key from the cloud environment secret named in BUILD-PROMPT
  section 3 into its own 0600 file under `~/.pad-keys/`, outside the repo, once, at first use, with
  `umask 077` and no echo. The pad reads a key only to copy its file into a box or to pipe it into
  `curl`; never into an argument, log, prompt or row. In the box, a key reaches an environment variable
  only in the launch shell, for `key_route = "env"`, when that is the harness's documented key route.
- The approval key, member id, `GH_TOKEN`, database and R2 credentials never leave the conductor. The
  device code comes out of the box; the approval goes in from the conductor (the `MATRIX_CODES_FILE`
  rule, `matrix.sh:70-73`).
- The push token: a fine-grained token of a throwaway pad owner, Contents read and write only, the one
  credential the box needs for `git push`. `pad` refuses it if it starts `gho_` or `ghp_` (the rule of
  `matrix.sh:92-104`). It is an owner input; until the owner provides it, rows run without `--repo` (no
  push block), C3 skips the pushed-repo scan and the row says "push not tested". `GH_TOKEN`, which
  creates the repos, never enters a box.
- In the box, the harness key and gateway key exist only as long as their route needs them, then only
  where the harness keeps them (its isolated home) or in the launch shell's environment for
  `key_route = "env"`. At the end the box is removed. Sandboxes are always `--non-persistent`: a stop
  otherwise snapshots the disk with the keys in it for 30 days (`2026-10-vercel-gateway.md:108-110`).
- The pad image (section 10) is built with no key, ever. No key ever enters a build box.
- The per-run known-values file holds only the values that entered this box: its harness key, gateway
  key and push token, every planted value and (with the owner's yes) the canary. It is made on the
  conductor, 0600, copied into the box for the checks, given to `goalcup-verify` with `--known`, and
  removed with the box. The no-shape value sits in its own file (section 5). The conductor's scan of
  the output folder also looks for the approval key and the conductor's credentials.

Spend (no pricing inside goalcup; PRD-00 "Not in this build"):

| Guard | Value | What happens |
|---|---|---|
| Per run | 10 USD (ledger T6). A `smoke` run is expected near 2 USD: an estimate, not a cap | Every 5 minutes during every run, `pad` runs `nice -n 19 goalcup-verify usage <open entry> --json` in the box with `sandbox exec` (an open entry reads to its last whole frame, PRD-09), prices the tokens per model with the rate lines below, and at 10 USD runs `goalcup seal --yes` in the box. The row gets `over_cap: true`, the watcher writes a night-log line (ledger B8), and the checks run on what was recorded. After the run the ledger line is written as below and marks `over_cap` too. Not a check failure |
| Build spend | No cap (ledger T6: cost is otherwise not a constraint) | The scribe writes a `spend` line and tells the owner at 200 and 400 USD of model spend by the ledger estimate and at 75 and 150 USD of sandboxes; the report shows it; runs go on (PRD-00 owner question 8) |
| Per key at once | At most 2 sandboxes per provider key, except DeepSeek: up to 6, back to 2 for the rest of the build once a row's 429 share passes 5% (PRD-00 R3-9). No total cap of our own: as many boxes as the Vercel account allows (ledger B11, which wins over R3-9's 6 in all); runs with no key (`selftest`'s offline controls) never queue | A queue on the conductor. Run 1 hit OpenAI's rate limit with about 20 subagents (run report measure 9). PRD-16 may hold a key for one run alone (A1) |
| Provider side | The owner sets a budget on each key where the vendor offers one | Unverified which vendors enforce a hard stop |

The ledger `pad-out/spend.jsonl` has one line per run, written after the checks: tokens per model from
`goalcup-verify usage` (input, cached, output, reasoning), an estimate in USD, box minutes, provider,
case, time. Prices come from one rate line per model the pad keys use in `scripts/pad/pad.toml` (input,
cached input and output per million tokens, copied from the vendor's pricing page on the day), applied
per model from `usage` `models[].tokens`. A model with no line is priced at the highest rate the vendor
lists for that key, and the row says `priced_high: <model>`. The pad reads no provider usage or billing
page (ledger T4); the owner's own dashboards stay the owner's check of real spend.

### 8. The checks and the row

`pad-run` runs these in the box, after the conductor copied in the fetched entry and the per-run
known-values files (ledger B1: the conductor runs `fetch-run`, sandbox control, approval and the pad's
own scripts, never `goalcup-verify`):

```
goalcup-verify check --json --known <known> --harness-files harness-files.json <local entry>
goalcup-verify check --json --server <fetched entry> --local <local entry> --known <known>
goalcup-verify secrets --known <known> <fetched entry> status.json goalcup.log upload.json
goalcup-verify secrets --known <plain> <local entry> <fetched entry>   # the no-shape value (section 5)
git -C <project> cat-file --batch-all-objects --batch | goalcup-verify secrets --known <known> -
goalcup-verify usage <fetched entry> --json --out usage
```

The `cat-file` line reads every object in the project's own git store in the box, which holds
everything the agent pushed; it runs only when the run had a push token. The pad clones nothing: no
outside API is called to verify (ledger T4). The known-values file holds `known-box` (the run
certificate key, ledger R3b) by then. The conductor copies the JSON out, removes the box, and
`check.py` maps it to the row.

`harness-files.json` is made in the box before the seal's wait ends: every file under the connector's
native roots (PRD-11 native rows) changed since the start marker, as `[{"path": "<source>:<relative
path>", "size": n}]` (PRD-08 section 1). The verifier never walks a home folder itself. A connector
with no native roots, or a harness that wrote no file there, gives an empty list; the local log check
is then `skip`.

The six checks. Each is PASS, FAIL or INCONCLUSIVE. **They read the proxy record** (ledger T4): PRD-08's
S6 report, PRD-09's S7 summary and PRD-07's `fetch.json` (S13). The harness's own session files add
one optional check, the **local log check** (PRD-08's `threads_match` and `wire_keeps_up`, with
`subagents_present` on the manifest), where the harness keeps them and PRD-09 reads them
(`readable: true`). With `readable: false` (`no_reader`, `snapshot_unreadable`, `no_native`) it is
`skip`, the row says `local_log: none`, and nothing else changes: it never blocks a PASS or a tier.
Where it runs and fails, it is a FAIL like any other.

| # | Check | PASS when | INCONCLUSIVE when | Catches |
|---|---|---|---|---|
| C1 | No missed call | No unexplained gap: `gap_has_end` `pass`, and every `named` loss sits in a gap with a start, an end and a reason (in a fault row, only the gaps section 3.1 expects). The history check (below) is `pass` or `skip`. The local log check, where it runs: `threads_match` `pass`, and `wire_keeps_up` `pass` or, for a totals log (`harness_log: "totals"`, Hermes), `skip` "no per-call times" with `threads_match` passing in the totals form (PRD-08 section 3.1). At least 2 model calls in the record; the case rows of section 2 hold. Dodger: `named` as section 2 says | the record holds fewer than 2 model calls | B1 |
| C2 | Every subagent recorded | usage `threads` has at least one thread with a parent, and every thread has wire calls or a native stream. Threads and parents come from the record (PRD-09's thread hints and the history check's chains, below), plus native streams where read; with a local log, `subagents_present` `pass` with the manifest too. Dodger: every `conn` to the pinned host is a named tunnel and no thread is claimed from it; with a local log, the native streams carry the threads | no thread with a parent (the agent did not spawn one; rerun once, then a prompt note) | B4 |
| C3 | No secret | 0 known-value hits in local parts, the server copy, `status.json`, `goalcup.log`, `upload.json`, the pad's output folder (after the pane replacement of section 4) and the project's git objects (leak scan copied from `matrix.sh:1267-1320`, read in the box; "push not tested" without a push token); the known values include the run certificate key (`known-box`, ledger R3b); 0 unexpected shape hits; every planted shaped value has at least one MASK. Hits of the no-shape value are `facts.plain_hits`, not a FAIL, in the default build (section 5) | the setup-check script never ran (section 5) | B8 |
| C4 | Record whole, server copy whole and readable | local `check`: no `fail`; `server_chunks_whole`, `server_equals_local`, `server_counts_match` all `pass` (from `fetch.json`); parts kept (start event `keep: true` and `upload.json` `keep: true`). A `skip` on `server_equals_local` is FAIL, not skip | `server = "off"` (row is "local pass" at best) | B2, B5, B6 |
| C5 | Usage stats present | usage on the server copy, from the wire calls (native tokens added where read): merged input plus output tokens above 0; every thread has a model and calls above 0; one row per subagent; `coverage.unattributed` 0 with PRD-09's meaning (only a hint-bearing call that lost its hint); `wire_only` calls are reported, never failed. Dodger: wire none, the summary says "not recorded on the wire"; native tokens present where a local log is read, and none claimed where there is none | the record holds fewer than 2 model calls | the usage reader silently reading nothing |
| C6 | Counts the participant saw | `status.json` in `mode: sealed`: `written.model_requests` equals the verifier's `counts.model_requests` and `fetch.json`'s `model_requests`; `written.off_ms` equals the verifier's `off_ms`; the number on the saved end screen is that same number; no `status.jsonl` sample has `written.model_requests` above the final recount | none | B3 |

**The history check** (ledger T4; worked out by PRD-09 from the record, ask 5). Most harnesses resend
the whole conversation with each call. So each call's request holds the earlier answers of its thread,
and each such answer must be in the record as a response of its own.

- **Match.** An earlier answer matches a recorded response by its tool call ids where it has any,
  else by its text with whitespace made even. Masked bytes match, since both sides passed the same
  masking. Thinking blocks are left out (providers drop or encrypt them). An answer from an earlier
  part of the run is matched against that part's entry.
- **Orphan.** An answer with no recorded response is a missed call. Inside a named gap it is part of
  that gap's stated loss. Outside every named gap it fails C1, named by thread, `sid` and time.
- **Skip.** A thread whose calls never resend an earlier answer (the harness keeps the history on the
  provider's side, for example by a previous-response id) is `skip` "no resent history". All threads
  skipped: the check is `skip`.
- **A hint, not a proof.** It sees a missed call only when a later call resends its answer. The last
  answer of a thread is out of its reach, and so is a harness that rewrites its history (risks).
- **Chains for C2.** A call that continues no earlier conversation starts a thread. A child's final
  answer, returned to another thread's resent conversation as a tool result, names that thread as its
  parent. Thread ids on the wire (Codex's `thread-id` header) come first.

A harness with no local log (Hermes run through its gateway, Amp, any harness before PRD-09 has its
reader; Hermes before `sqlite.rs` lands in the second slice) is judged on C1, C2 and C5 from the
record alone and can reach "verified" (section 9).

INCONCLUSIVE causes that are never goalcup's failure, decided first from the record's own `http_resp`
statuses (`O-harness-access.md:280-281`). After them, and before any check: the run must have done work.
The workspace has at least one commit since the start, and the record has at least one model request
or native call (the idea of `matrix.sh:2054-2150`). Otherwise the row is INCONCLUSIVE "the harness did
no work" (a first-run dialog ate the prompt, a login failed).

| Cause | Sign |
|---|---|
| `provider_down` | Most recorded model responses are 5xx (from the record only; the pad reads no status page, ledger T4) |
| `provider_refused` | 401 or 403 on the model host (a bad or unfunded key) |
| `sandbox_lost` | The box vanished or timed out before the seal |
| `harness_install` | The pinned version would not install |
| `harness_subagents_unavailable` | PRD-11's connector outlook says subagents need a daemon 1.0 does not start (OpenClaw's Gateway, appendix O H5.4), and no thread has a parent. C2 is INCONCLUSIVE and the tier reads "not verified: subagents need the Gateway" |

Row, `row.json` (sorted keys, no secret, no path outside the run folder):

```json
{"connector":"codex-cli","harness_version":"<pinned>","case":"direct","prompt":"full",
 "variant":[],"where":"sandbox","os":"linux-x86_64","image":"<snapshot id>",
 "goalcup":"1.0.0+<commit>","server":"staging",
 "run":"r…","entry":"e…","started":"…","minutes":20.1,"result":"PASS",
 "checks":{"C1":"PASS","C2":"PASS","C3":"PASS","C4":"PASS","C5":"PASS","C6":"PASS"},
 "why":[], "facts":{"wire_requests":0,"harness_calls":0,"harness_log":"per_call","local_log":"read",
  "history":{"state":"pass","answers":0,"orphans":0,"orphans_in_gaps":0},"threads":0,
  "subagents":0,"native_files":0,"gaps":{},"off_ms":0,"provider_errors":{},"secret_hits":0,
  "masks":{},"plain_hits":0,"screen_known":0,"canary":"not planted",
  "tokens":{"input":0,"cached":0,"output":0,"reasoning":0},
  "models":[],"raw_bytes":0,"compressed_bytes":0,"recorder_rss_peak_mb":0,"spend_usd_est":0,
  "priced_high":[],"first_live_alarm":null,"over_cap":false,"push":"tested"},
 "exec":{},
 "pad_trusted_at":"<commit of the last passing selftest>"}
```

`variant` lists the faults, `--exec` and `--pre` scripts (section 3.1), for example
`["kill-recorder@8","kill-recorder@10"]`; empty for a plain run. `exec` maps each script to its
watcher verdict (`exec/<script>.verdict`), for example `{"h2-parallel.sh":"PASS"}`. Without a local
log, `local_log` is `none`, `harness_calls` and `harness_log` are null, and `history` still holds. `goalcup` comes from the start event's `build` key (PRD-01). `why` holds one plain line
per non-PASS check, copied from the verifier's `why`. The row result is PASS when all six are PASS, FAIL
when any is FAIL, otherwise INCONCLUSIVE.

### 9. The table, the tier file and the release check

`scripts/pad/table` reads every `row.json` under `pad-out/` and writes three files:

1. `docs/plans/2026-10-30-v1.0/PAD-RESULTS.md`: a summary with one line per harness (tier, version, the
   three case results, the kill row for the core set, date), then one table per harness with one row per
   run (newest first), the six checks, the facts that matter (wire requests against harness calls,
   subagents, tokens, gaps, spend) and `why` for anything not PASS. Fault rows sit in a separate table
   per harness.
2. `connectors/tiers.toml`: the one tier source for the start line, the guide and README (PRD-00
   ownership table). Shape, as PRD-10 asks (T1): one table per connector id, sorted, same rows give the
   same bytes:

```toml
[codex-cli]
tier = "verified"            # "verified", "not verified" or "not supported"
version = "<pinned version>" # empty unless verified
reason = ""                  # one plain line unless verified, e.g. "no key", "C1 history orphan"
```

3. `docs/plans/2026-10-30-v1.0/pad-rows.jsonl`: every `row.json` as one sorted line (rows hold no secret
   and no path outside the run folder, section 8), committed with `PAD-RESULTS.md`.

In phase 0, `scripts/pad/table` with no rows writes the seed `connectors/tiers.toml`: one table per
connector id (16), `tier = "not verified"`, `version = ""`, `reason = "not yet run on the pad"`. The
integrator commits it with PRD-11's phase 0 connectors, so PRD-10's build-time embed has a file from
the start.

Tier rule:

- **verified on `<version>`**: on that version, from the release commit (below), with a trusted pad
  (section 12): the latest `direct`, `gateway` and `dodger` runs, all `full` mode with no fault step, are PASS. `gateway` counts only where a documented route exists; `n/a` does not block.
  Fault rows (every `cycle` row among them) and `micro` rows never give or take a tier.
- **not verified**: anything else, with the reason (`no key`, a failing check, `loop stopped after
  three fixes`). A missing local log reader is never a reason: the record is the proof (ledger T4).
- **not supported**: the harness ignores `HTTPS_PROXY` or refuses the run CA on its model host in the
  `direct` case, so the direct run looks like a dodger (tunnels with `tls_refused`, or nothing reaching the
  proxy while session files grow). Never hacked around.

`scripts/pad/table --release-check <commit>` (PRD-14 section 9, PRD-16) reads the committed
`pad-rows.jsonl`, never `pad-out/`. It exits 0 only when the nine core `full` rows (Claude Code, Codex,
Hermes in the three cases) and the Codex kill row are PASS with a trusted pad, the `h2-parallel.sh` and
`long-tunnel.sh` exec rows of PRD-03 acceptance 3 and 9 have a `PASS` verdict (PRD-00 done item 9), and
the commit those rows ran on has no difference to `<commit>` under `crates/`, `server/worker/src/`,
`connectors/`, `Cargo.lock`, `Cargo.toml` and `rust-toolchain.toml`, leaving out `connectors/tiers.toml`
(it is written after the rows, so it always differs). Otherwise exit 1 with the missing or differing
items named.

`connectors/tiers.toml` is committed by this PRD's lane after phase 5. PRD-10 embeds it at build time, so
the release binary is built once more with it; only the tier text differs from the binary the pad ran.

### 10. The pad image

`scripts/pad/image` builds one Vercel Sandbox snapshot: Ubuntu with `tmux`, `sqlite3`, `zstd`, `jq`,
`git`, `gh`, `iproute2`, Node and Python, plus every harness at its pinned version as the non-root user
(Claude Code refuses bypass mode as root, `O-harness-access.md:542`). No key, no goalcup build. In phase 0
it holds the three core harnesses only (PRD-00 phase 0 step 9); the other 12 are added before phase 5.
Each run starts from it, so a 20-minute run does not spend 5 minutes installing. The snapshot id is
written in every row. Rebuild it when a pinned version changes. The sandbox's `HOME` is `/vercel` and its
user is `ubuntu` with passwordless `sudo` (`2026-10-vercel-gateway.md:92`, `docs/research/VERCEL-COMPAT.md`
row "Root not required"); the pad uses `$HOME`, never a fixed path.

### 11. The contributor path

The pad is the one way to add a harness (owner decision). `skills/adding-a-capture-harness/SKILL.md` is
rewritten to these steps, about 60 lines:

1. Read the harness's current docs. Write `connectors/<id>.toml` from PRD-11's schema (one flat file,
   data only, no Rust).
2. Write `scripts/pad/harnesses/<id>.toml` (section 6).
3. Copy `scripts/pad/runs/micro.toml`, set `harness = "<id>"`, `key = "<your key name>"`,
   `connector = "connectors/<id>.toml"`, `where = "here"`, `prompt = "smoke"` (it keeps
   `server = "off"`), and run `scripts/pad/pad <that file>`. `connector` loads the new file into the
   shipped binary, so no rebuild is needed. Read the table and `report.txt`. Fix the connector until C1 to C3 and C6 pass and C4's
   local half passes (the record is the proof; no log reader is needed).
4. Optional: if the harness keeps session files and the row says `local_log: none`, attach the masked
   native streams of the run (`goalcup-verify dump --kind native <entry>`) to the pull request. A reader
   may then be written in PRD-09's module from that sample (PRD-09: "a reader for a session format is
   written only from a real pad sample"). It adds the local log check; it is not needed for a tier. A
   contributor may write it; it is a code change reviewed as such.
5. Open the pull request with `row.json`. A maintainer runs the pad on staging, `mode = "full"`,
   `case = "all"`. PASS gives the tier.

This PRD deletes `skills/adding-a-capture-harness/writing-a-decoder.md` (277 lines; decoders are cut,
PRD-08, and the usage reader rule replaces it) and
`skills/adding-a-capture-harness/templates/connector-harness.toml` (209 lines; replaced by a pointer to
PRD-11's example connector). `skills/adding-a-capture-harness/templates/harness-facts.md` stays, with a
"pad result" section added and its `crates/goalcup-protocol` citation (`:56`) removed. A new
`skills/adding-a-capture-harness/templates/pad-harness.toml` is the section 6 file.
`docs/HARNESS-ONBOARDING.md` is PRD-15's to shorten to a link.

### 12. The pad must fail before it is trusted

The pad is trusted at a commit only when `scripts/pad/selftest` turns every known fault, run 1's bugs
first (K2), into FAIL on the right check, and a clean run into PASS, and a reviewer has read its first
outputs by hand (below; ledger B9). `selftest` runs as soon as the offline controls exist (builder notes
step 2), at the end of phase 2 (PRD-16), and again after any change under `scripts/pad/`,
`crates/goalcup-verify/`, or a check threshold; the row field `pad_trusted_at` names the commit. A row
made by an untrusted pad says so and cannot give a tier.

**The first-output read.** A simple rig can hide a wrong check behind a tidy table, so a reviewer who did
not write the pad reads its first outputs by hand, once, and again after a change to `check.py`'s
mapping. The reviewer reads `selftest`'s output, then the first `micro` run's table, `row.json` and
`report.txt` and the first `cycle` run's, and checks each against the record itself
(`goalcup-verify dump` in a check box), not against the pad's own words:

- K2 fails C1 on the run 1 stop (the wire ends at 15:43:15Z with no gap), not on some other finding;
  K6, K9, K10 and K11 fail on the check and finding their row names.
- The `look` and `seal` lines' call counts equal a hand count of model requests in `dump`.
- The crash happened: `status.jsonl` shows a new recorder pid at the kill time, the record has the one
  `recorder_down` gap there, and calls were written after it. The `wait-recovery` line saw that, not
  just a live process.
- `keep-going` typed only when the pane and the wire were idle, and the pane shows it.
- C3 scanned the joined local parts, the server copy and the output folder (the known-values file held
  the harness key and `known-box`); a deliberately planted shaped value shows a MASK in `dump`.
- C4 compared the server copy with the kept local parts, not with a pruned folder.
- The rig's outside calls (it logs each command to `calls.log` in the run folder) are only the sandbox
  CLI, the test repo create and delete, the staging approval and `fetch-run.sh` (ledger T4).
- Cleanup left nothing: `sandbox list` shows no `pad-` box of that run, the test repo is gone, the trimmed
  folder holds only what section 3.2 keeps.

The reviewer writes one night-log line per finding, or one line "first-output read done at <commit>".
Only then is `pad_trusted_at` set.

Control ids are kept from revision 1 so other PRDs' references hold; K1, K3 and K4 are cut.

| Control | Input | Must give |
|---|---|---|
| K1 | Cut (the run 1 converter is cut, PRD-00). Run 1's real numbers come from PRD-16 A1 | |
| K2 run 1 shape | PRD-08's run 1 shape builder (`check_run1_shape.rs`), written as a part folder and as a fake fetched copy | C1 FAIL (`gap_has_end`; local log check `wire_keeps_up`, `threads_match`), C2 FAIL (`subagents_present`), C4 FAIL (`gap_has_end`, `streams_closed`, `server_counts_match`), C6 FAIL against a fake `status.json` whose `written.model_requests` is run 1's finish-line 1,736 |
| K3, K4 | Cut (the silent-drop hook is cut, PRD-00). The C1 and C2 mapping on a silent loss is proven by K2 | |
| K5 secret leak, real | Claude Code, `mode = "micro"`, `prompt = "smoke"`, `server = "staging"`, plus a value planted in a file outside the workspace that the setup script also prints, listed in the main known-values file. PRD-05 section 10: such a value is not masked | C3 FAIL with one known-value hit, in local parts and server copy |
| K6 server tamper | A copy of K7's fetched folder: one flipped byte in `rec.gcr.zst`; one middle chunk removed from `fetch.json` and its bytes from `rec.gcr.zst`; `model_requests` in `fetch.json` plus one | C4 FAIL for each |
| K7 clean | Claude Code, `mode = "micro"`, `prompt = "smoke"`, `server = "staging"`, direct, kept parts | PASS on all six |
| K8 pruned | K7 again with `keep = false` (PRD-06 U5's run may stand in when it runs on the same commit) | C4 FAIL ("parts pruned"), never PASS or skip |
| K9 count tamper | K7's `status.json` with `written` plus one | C6 FAIL |
| K10 usage gone | K7 with the usage summary emptied | C5 FAIL |
| K11 missed call (history) | PRD-09's history fixtures (synthetic, no secret, under `scripts/pad/fixtures/`): (a) a record that leaves out one response while later requests resend its answer, with no gap; (b) the same with a named gap over it; (c) a record whose calls never resend history; (d) a clean record with no native stream at all | (a) C1 FAIL with one orphan; (b) C1 PASS, the orphan counted in the gap; (c) history `skip`, C1 PASS; (d) local log check `skip`, all six PASS (the no-local-log path to "verified") |

K2, K6, K9, K10 and K11 cost nothing and run in seconds in a check box (a build box with no key, ledger
B1). K2's part folder and fake fetched copy are written by PRD-08's test, K11's by PRD-09's, in a build
box, and committed under `scripts/pad/fixtures/` (synthetic, no secret). K5, K7 and K8 are short real runs, about 1.5 USD in all.

### 13. The watcher loop

A watcher is an agent (PRD-16 schedules them; one per named cause, as many at once as there are causes,
each its own real-runs workflow of 2 agents, ledger B2 and B11). It acts on a row
that is FAIL. INCONCLUSIVE rows are rerun once with no code change; a second INCONCLUSIVE goes in the
table as is.

1. **Read.** `row.json`, `report.txt`, the verifier JSON, `watch/`, the panes, `goalcup.log`, then
   `goalcup-verify dump --sid <first failing sid>` on the fetched copy, in a check box (ledger B1;
   PRD-08 keeps the first failure's offset and `sid`).
2. **Name the cause.** The watcher names the cause in one line and finds the owning PRD. The signature
   `<connector>|<case>|<variant>|<check id>|<first finding code>` only groups rows:

   | Sign | Look first in |
   |---|---|
   | Wire has the calls, usage does not read them | PRD-09 |
   | Wire lacks the calls; GAPs or a `memory_guard` | PRD-02 |
   | Wire lacks the calls; `tunnel` or `tls_refused` on a listed host; `refusal_after_trust` | PRD-11 (CA variables, proxy setting), then PRD-03 |
   | Nothing reached the proxy while session files grew | PRD-11 (daemon to stop, setting that beats `HTTPS_PROXY`); else the harness is "not supported" |
   | A rollout or transcript file has no native stream | PRD-04, or PRD-11's rows |
   | Known value or shape in the record | PRD-05 |
   | `server_*` | PRD-06, then PRD-07 |
   | `inv*`, `gap_has_end`, `streams_closed` | PRD-01, PRD-02 |
   | A history orphan outside every gap | PRD-02 (a write lost), then PRD-03 (a stream not recorded), then PRD-09 (the match rule) |
   | A `recover` line (refused, wrong port, harness restarted) | PRD-10, then PRD-02 |
   | C6 | PRD-02 (counts), PRD-10 (text) |
   | A fault row's own line (gap length, restart, catch-up) | The fault's owner in section 3.1 |
   | The pad itself (a wrong manifest, a typed prompt split, a fault that did not fire) | PRD-12 |

3. **Reproduce cheap.** Copy the row's settings file, set `mode = "cycle"` (same harness, case and
   steps) and run it. If it does not reproduce, rerun the original settings once.
4. **Fix.** On the owning PRD's branch, as that PRD's builder, inside its owned paths (PRD-00 merge rule).
   Run that PRD's own tests in a build box.
5. **Rebuild.** `cargo build --release --locked -p goalcup -p goalcup-verify` in a build box as `ubuntu`
   (PRD-00 "Where the build runs"; PRD-14's build lock `target/.build-lock` applies inside the box). The
   conductor never compiles and the pad box never builds; the run copies the binaries out of the build
   box. A row is a release row only when its binaries came from an integration commit (PRD-16 merges the
   fix first).
6. **Rerun** the same settings file, same harness version. The same named cause again, whatever the signature:
   one failed fix. A different named cause starts a new loop.
7. **Stop** after three failed fixes on one cause (the cause the watcher names in step 2; the signature
   only groups rows): a night-log entry (PRD-16) with the three attempts,
   the row stays FAIL, the tier is "not verified". For the core set that is a release blocker in the
   end-of-build report.

Rules that never bend:

- A watcher never makes a check pass by changing the check, a tolerance or the pad's expectation, unless it
  shows with record evidence that the check was wrong. That change goes to the check's owner (PRD-08 or
  PRD-12), is written in the night log as "check changed", and `selftest` must still turn K2 and K5 to K11
  into their expected results before any row is trusted again.
- Never hack around a harness: no base URL from goalcup, no patched harness, no disabled pinning. A harness
  that will not use the proxy is "not supported".
- One cause, one watcher: a lock file per named cause in `pad-out/loops/`.
- When the loop ends, the watcher deletes the run folders it kept (section 3.2).

## Files it owns

Create (about 1,400 lines in all, against 6,208 src and 4,482 test lines of the tools it replaces):

| Path | What | Lines (est.) |
|---|---|---:|
| `scripts/pad/pad` | Conductor half: the settings file (mode defaults, unknown key refused), key files from the environment secrets, key file rules, push token rule, test repo create and delete, staging and loopback only (from `matrix.sh:253-258`, `:559-600`, no production setting), sandbox lifecycle, approval (`:839-887`), fetch, copy-in, the spend check, pane replacement, cleanup trap, the printed table, `calls.log`, ledger, queue | 320 |
| `scripts/pad/pad-run` | Box half: version check, project and push token, plants (canary only with the owner's yes), setup, `pre`, key route, tmux start, typing, the steps of section 1a with their waits and limits (`wait-calls`, `look`, `wait-recovery`, `keep-going`), watch, fault and exec schedule (with the `pad:recover` window), `known-box`, seal, upload wait, wipe, the section 8 commands | 330 |
| `scripts/pad/check.py` | Section 8: maps the S6, S7 and `fetch.json` JSON the box's commands wrote to C1 to C6, fault expectations, INCONCLUSIVE causes, `row.json`, `report.txt`, the known-value scan of the output folder, the ledger line. It never runs `goalcup-verify` itself | 320 |
| `scripts/pad/selftest` | Section 12 | 100 |
| `scripts/pad/table` | Section 9, including `--release-check` | 150 |
| `scripts/pad/image` | Section 10 | 60 |
| `scripts/pad/pad.toml` | The per-run cap, parallelism, staging address, rate lines per model | 30 |
| `scripts/pad/prompts/micro.txt`, `smoke.txt`, `full.txt`, `pad-setup-check.sh` | Section 5 | 25 |
| `scripts/pad/runs/micro.toml`, `cycle.toml`, `full.toml` | Example settings files (section 1a) | 15 |
| `scripts/pad/exec/h2-parallel.sh`, `refusal-curl.sh`, `long-tunnel.sh`, `time-left.sh`, `hint-local.sh`, `proxy-setting.sh`, `stale-gateway.sh` | Section 3.1, watcher and `--pre` commands other PRDs' proofs need | 70 |
| `scripts/pad/fixtures/` | K2's part folder and fake fetched copy, written by PRD-08's test; K11's history fixtures, written by PRD-09's (section 12) | generated |
| `scripts/pad/harnesses/<connector id>.toml` and `gateway/` snippets | Section 6, one per connector (16) | 16 x 15 |
| `scripts/pad/test_pad.py` | Automated tests below, few on purpose (ledger B10) | 90 |
| `docs/plans/2026-10-30-v1.0/PAD-RESULTS.md`, `pad-rows.jsonl` | Generated by `table` (section 9) | |
| `connectors/tiers.toml` | Generated by `table` (section 9), seeded in phase 0; the only path under `connectors/` this PRD writes | |
| `skills/adding-a-capture-harness/SKILL.md` (rewrite), `templates/pad-harness.toml` (new), `templates/harness-facts.md` (change) | Section 11 | 100 |

Change: `.gitignore`: add `/pad-out/` in the pad's first commit (phase 0), before any run writes there
(PRD-00 ownership row; PRD-13 removes the `spikes` lines).

Delete (this PRD, not PRD-13; PRD-00 ruling on consistency item 10):
`skills/adding-a-capture-harness/writing-a-decoder.md`,
`skills/adding-a-capture-harness/templates/connector-harness.toml`.

Must not touch: `crates/**` (PRD-01 to PRD-11), `server/**` and `scripts/fetch-run.sh` (PRD-07),
`connectors/**` other than `tiers.toml`, including `model-hosts.toml` (PRD-11; the dodger copy is made in
the run folder, never in the checkout), `scripts/gate.sh` and CI (PRD-14), `docs/*.md` (PRD-15),
`NIGHT-LOG.md` (PRD-16; watchers write it in their PRD-16 role), the old scripts (PRD-13 deletes them).

## Interfaces with other PRDs

| PRD | What crosses | Direction |
|---|---|---|
| PRD-00 | Done item 1 (direct, gateway and dodger, all `full`, Hermes per session as the local log check, dodger pass) and done item 9 (HTTP/2 and long-tunnel exec rows); owner question 3 (masking, default no); defaults for keys, spend, server, kept parts; seams S9, S13, S15, S16; the conductor and build boxes; the `.gitignore` ownership row | Used here |
| PRD-01 | `rec/` layout, `zstd -dc --long=27` on parts; the start event's `hosts` (with `kind` and `pinned`), `hosts_sha256`, `connector`, `connector_source`, `harness_version`, `keep`, `build`, `fd_limit` | Used here |
| PRD-02 | `status.json` (S4: mode, written, off, losing); `GOALCUP_KEEP_PARTS`; `rec/watchdog.pid`; proofs R1, R3, R4, R5, R7 run on the pad through faults (section 3.1); the kill case on Claude Code and Hermes `full` rows with R1's pass lines; the 2 s gap bound; the eager restart and crash-loop rule (ledger R3a: no `null_mode` gap in the kill rows); for `goalcup recover` (R3b), the run certificate key file (0600, run folder, kept for the run) and the recover entry (PRD-10 ask N9, open) | Used here; proofs owned there |
| PRD-03 | Section 12 per-case expectations; the dodger case is the `pinned` list copy through `--hosts`; acceptance 3, 7 and 9 run as `--exec` scripts, and the verdicts of 3 and 9 feed `--release-check`; the route-drop fault | Used here; proofs owned there |
| PRD-04 | The manifest rule for C2 (PRD-04 interfaces row PRD-12) | Used here |
| PRD-05 | The planted file shapes (section 10) and A1; the no-shape value as a stated loss in the default build; `sk_test_`, `whsec_` and the canary only with the owner's yes to PRD-00 question 3 | Used here |
| PRD-06 | `upload.json` (S9: `keep`, `final: sealed`, waits) for the upload wait and the watch; U3, U5 via the `cut-worker` fault; U8 via `kill-both`; the `freeze` stand-in | Used here; proofs owned there |
| PRD-07 | `fetch-run.sh --bucket goalcup-traces-staging` from the conductor (read only) and `--local`; S13 `rec.gcr.zst` and `fetch.json`; the staging test approval route `POST /v1/device/approve` | Used here |
| PRD-08 | S6 `check --json`, `--harness-files`, `--known`, `--server --local`, `secrets` (with `-` for the project's git objects), `gap_has_end` and `named` losses for C1; `threads_match`, `wire_keeps_up` and `subagents_present` as the optional local log check (ask 6), `check --live` (under `nice -n 19`), `dump`, all run in a box (PRD-08 section 1 "Where it runs"); the totals form and `skip` rules of section 3.1; dodger exit 1 from `listed_hosts_decrypted: named` (R4); `refusal_after_trust`; `check_run1_shape.rs` for K2 | Used here |
| PRD-09 | U1 and S7 via `usage --json --out`; the history check and the wire threads and parents of C2 in S7 (ask 5, ledger T4); `usage --json` on the open entry for the spend check (`models[].tokens`); `readable` and `reason`, `coverage` (`unattributed` with its narrow meaning, `wire_only` reported), `threads`, `host_kind`; no `--live`, no pricing inside goalcup; new readers only from pad samples (PRD-16 phase 5 reader step) | Used here |
| PRD-10 | `GOALCUP_SERVER`, `GOALCUP_HOME`, `GOALCUP_KEEP_PARTS`, `GOALCUP_NO_BROWSER`; `start --test --repo --prompt --connector --hosts` (S16) and `first-message.txt`; `continue`; `seal --yes`; `status --json`; contract words; `setup --harness <id> --yes` (S15); `goalcup recover` and the status line ending `Run: goalcup recover` (section 3.2 and 4, ledger R3b; acceptance 11 runs as the `kill-watchdog-recover` fault); reads `connectors/tiers.toml` in the T1 shape | Both |
| PRD-11 | Flat connector files and the launch plan; native roots for the manifest; `pinned` in `model-hosts.toml`; loading for `--connector` and `--hosts`; acceptance 1 to 6 run on the pad, 3 to 5 set up by `--pre` scripts (PRD-11 ask 6); the OpenClaw subagent outlook; OpenCode counted once with both versions | Used here |
| PRD-13 | Deletes the replaced scripts and their tests (below) once `selftest` passes | Done there |
| PRD-14 | `scripts/pad/test_pad.py` in `gate fast`; `scripts/pad/table --release-check <commit>` for `release.sh tag`; the build lock inside build boxes. The pad needs no `test-hooks` build | Both |
| PRD-15 | `docs/HARNESS-ONBOARDING.md` shrinks to a link to the skill; `docs/OPERATING.md` "The matrix" section points to the pad; the guide's tier table reads `connectors/tiers.toml` | Written there |
| PRD-16 | Schedules pad phases, the ladder's rungs (section 1a: `micro`, then `cycle` runs, then `full` waves, many in parallel) and watchers; the night log; the end-of-build report reads `PAD-RESULTS.md`; A1 to A4 run through the pad with settings files (`prompt` as a path, `minutes`, the `keep-going` and `kill-recorder@m` steps); the spend check (section 7) and the scribe's spend report lines, which read the ledger; the phase 5 reader step | Used there |
| PRD-17 | Real-run lessons RR1 to RR6 ride on pad rows (RR6 rides on P9, the owner's Mac run; Codex WebSocket deflate, Anthropic brotli); X7 is a pad rule (a row passes only if the agent worked) | Used there |

## What is cut

Deleted by PRD-13 after `selftest` passes on the integration branch; what the pad needs is copied first
(the functions named in "Today").

| Path | Src lines | Tests that go |
|---|---:|---|
| `scripts/matrix.sh` | 2,522 | `scripts/test_matrix.py` 1,517 |
| `scripts/linux-matrix.sh` | 876 | `scripts/test_linux_matrix.py` 1,219 |
| `scripts/linux-harness-smoke.sh` | 242 | none |
| `scripts/check-connector.sh` | 314 | `scripts/test_check_connector.py` 411 |
| `scripts/campaign.py`, `campaign.example.json` | 1,952 | `scripts/test_campaign.py` 1,335 |
| `scripts/kit_settle.py` | 302 | none (its first-run answers move to PRD-11's `no_prompts` data and `goalcup setup --yes`) |
| **Total (PRD-13)** | **6,208** | **4,482** |

Deleted by this PRD (section 11): `writing-a-decoder.md` 277 and `templates/connector-harness.toml` 209,
486 lines, no tests. `scripts/linux-check.sh` and its test are PRD-14's to delete, with
`.github/workflows/linux-x86_64.yml`.

## Acceptance

### Real-run proofs (first)

| # | Proof | Pass |
|---|---|---|
| P1 | **The pad fails before it passes.** `scripts/pad/selftest` on the integration commit, end of phase 2; the first-output read of section 12 | K2, K5, K6, K8, K9, K10 each FAIL on exactly the named checks; K7 PASS on all six; K11 gives its four named results; the reviewer's night-log line "first-output read done" names the commit |
| P2 | **Phase 1a, ladder rung 1.** As soon as the core slice records one request (PRD-00): Claude Code and Codex, `mode = "micro"`, `direct`, `server = "off"`, `where = "here"` inside pad boxes (builder notes step 2), never on the conductor | C1, C3 and C6 PASS from the record; C4's local half PASS; C2 INCONCLUSIVE as expected (the `micro` prompt asks for no subagent). The same settings with `prompt = "smoke"` give C2 PASS |
| P3 | **Core set, release.** Claude Code, Codex, Hermes: `direct`, `gateway` and `dodger`, all `full`; staging; trusted pad | Nine PASS rows on named versions (PRD-00 done item 1). Each `direct` row shows subagents above 1, tokens above 0, `provider_errors` listed, and the history check `pass` or `skip` with 0 orphans outside gaps. Hermes passes C1 from the record; its totals form is the local log check where `state.db` is read |
| P4 | **One system for gateways.** The three `gateway` rows of P3, plus Kilo's `gateway` `full` row (PRD-16 A4) | `ai-gateway.vercel.sh` is `listed: shared`, `kind: gateway` in every `conn`; no connector has a gateway-specific line; usage calls show `host_kind: gateway`; C1 PASS |
| P5 | **Parallel.** Shown by the `cycle` and `full` waves (as many boxes at once as the account allows, ledger B11), no separate run | No cross-talk: distinct runs and entries; each row's known values found nowhere in another run's record; no 429 caused by the pad's own queue rule being ignored |
| P6 | **Provider refusal is not our failure.** Hermes `smoke` with a DeepSeek key file holding a wrong key | Row INCONCLUSIVE `provider_refused`; C4 and C6 still PASS (the 401s are recorded) |
| P7 | **The table.** After phase 5 | `PAD-RESULTS.md` has 15 harnesses (OpenCode once), a tier and a reason for each, a named version for each "verified", and `pad_trusted_at` on every row; `connectors/tiers.toml` has 16 tables; `table --release-check <integration head>` exits 0 |
| P8 | **Contributor dry run.** After the merge to main (PRD-00 "Not in this build"): a builder who did not write the pad adds a harness with only the skill and the pad, a `micro` settings file with `where = "here"` | A row with C1 PASS, under one hour of work |
| P9 | **The Mac.** The owner runs `scripts/pad/pad` with `scripts/pad/runs/full.toml` changed to `harness = "claude-code-cli"`, `key = "claude"`, `case = "direct"`, `where = "here"`, on a Mac after the build | PASS, or a named failure the night log turns into a fix. Hermes on macOS reads the OS trust store (appendix O H3.3); goalcup never adds the run CA to a keychain; if Hermes cannot trust it there, Hermes on macOS is "not supported on macOS" with that reason |
| P10 | **Kill case on the core set.** Codex (PRD-02 R1, done item 2), Claude Code and Hermes: `direct`, `mode = "full"` with `steps = ["kill-recorder@8", "kill-recorder@10"]` | Each row PASS with the `kill-recorder` expectations of section 3.1. The Codex row is in `--release-check`; a Claude Code or Hermes FAIL is a named risk in the end-of-build report |
| P11 | **The other faults, once each** (phase 4, beside the core set's `full` wave, PRD-16 section 6, since full runs come at the end, ledger B3; shared with other rows' boxes): `freeze@6+10` on Codex `full`; `route-drop@6+1` on Codex `full`; `cut-worker@1+10` on Claude Code `cycle` with `minutes = 14` (U3), and the same with `keep = false` (U5); `fd-limit@0` on Codex with 20 subagents asked (R4); `kill-watchdog@5` on Claude Code (R5); `kill-both@5` on Claude Code (R7, and `+upload` for U8); the `--pre` rows of PRD-11 acceptance 3 to 5 | Each row PASS with its section 3.1 expectations; the owner PRD's remaining lines read by the watcher from `report.txt` |
| P12 | **Recover** (ledger R3b, PRD-10 acceptance 11). Codex `direct`, `mode = "full"`, `minutes = 10`, `steps = ["kill-watchdog-recover@3"]`, so the agent still works after the recover (PRD-10 names `smoke`; a 3-minute run may end before it). Phase 4, after PRD-02's recover entry lands | Row PASS with the `kill-watchdog-recover` expectations of section 3.1, judged from the record, `status.json` and the server copy only (ledger T4). Answers given before the recorder kill and resent after the recover match recorded responses or sit in the one gap |
| P13 | **No local log, verified** (ledger T4). The first harness that keeps no session file the pad reads (Amp when it has a key; else Codex with its native roots left out by a `connector` copy, `mode = "full"`, `direct`, once in phase 4; the copy makes it a check row, not a tier row) | All six PASS with `local_log: none`; C2 finds a thread with a parent from the record's chains; C5 tokens above 0 from the wire |
| P14 | **The rig walks by itself** (ledger B9). Codex `direct`, `mode = "cycle"`: an agent who did not write the pad writes only the settings file, runs `scripts/pad/pad <file>`, and reads the table | Every step of the `cycle` list has a line; the crash and `wait-recovery` lines match the record (section 12's read); no step was typed by the agent; the run ends with a table even when a step is forced to time out (a second run with `wait-calls 999`); `cleanup` leaves no box, no test repo and only the trimmed folder |
| P15 | **Cycle runs per pad** (ledger B3 rung 2). Every harness with a key, every case it has, `mode = "cycle"`, as soon as its connector lands, many in parallel | One row each; FAIL rows go to a watcher (section 13); `keep-going` exercised on at least the core set |

### Automated tests that guard them

Few on purpose (ledger B10). The real runs through the rig are the main proof, and `selftest`'s
controls K2, K6, K9, K10 and K11 already prove the check mapping on fixtures, so `test_pad.py` does not
repeat them. It guards only what a real run cannot show safely and what the ledger names: secrets in the
pad's own hands (point 4) and the never-stop expectations of the crash rows (ledger R3, R3a, R3b). No
test pins a table layout, a field order or a line of wording. A test that takes longer to change than
the code it guards is deleted or made simpler.

`scripts/pad/test_pad.py` (Python `unittest`, no network, no harness):

- Keys: a key file that is 0644, a symlink, or inside the checkout is refused; a push token that starts
  `gho_` or `ghp_` is refused; no key value appears in any argument the pad builds (a fake key and a
  recorded `subprocess` call list), including a `key_route = "env"` launch, whose value is only
  exported.
- The `exec` environment filter keeps only proxy and CA names from a fake `environ` that also holds a
  key.
- The pane replacement turns a known value in raw, base64 and percent form into `[known:<n>]`.
- Crash rows: a kill fixture with one closed `recorder_down` gap per kill is PASS; a `null_mode` gap or
  a gap over 2 s is FAIL. A recover fixture with a changed harness pid or port is FAIL.

Left to the real runs and the first-output read (section 12), not to unit tests: the queue, the spend
check, the INCONCLUSIVE causes (P6 is a real one), the table bytes, `--release-check` (the reviewer
compares its first output with `git diff` by hand), the outside-call list (`calls.log`), and the step
waits (P14).

## Risks and blind spots

| Risk | What we do |
|---|---|
| A harness's first-run screen changes and eats the prompt | The "did work" gate makes it INCONCLUSIVE, not a false PASS; pane captures show the screen; `pre_keys` and `ready_text` are data |
| The agent does not spawn a subagent, or refuses to print `.env` | C2 or C3 INCONCLUSIVE, rerun once; the prompt word is per harness. A run never passes on a missing exercise |
| False alarms in C1 (a harness writes a call twice, a log format changes) | PRD-08's tolerance and "harness above wire only" rule; P3 must pass before any other harness is judged; a check change follows the rule in section 13 |
| The history check flags an answer the harness made itself (a compaction summary sent as an answer, a trimmed or edited answer, an answer from a session the run did not record) | The orphan is named by thread, `sid` and time; the watcher shows it with record evidence and the match rule changes by section 13's rule, in PRD-09. Never a looser C1 without that evidence |
| The history check cannot see the last answer of a thread, or a stop for good near the end of a run | It is a hint, not a proof (ledger T4). In 1.0 one process holds the port, so a recorder that stops cuts the harness off and shows as a `recorder_down` gap, never a silent stop (PRD-02); `streams_closed` and the counts in C6 catch a record that ends early; K2 proves the run 1 shape fails; the local log check sees it where a log is kept |
| With no local log, a subagent whose every call was missed and whose answer never came back to its parent leaves no trace in the record | The prompt asks for at least five named subagents, so a run with fewer threads than that is read by the watcher; any loss on the wire also shows as a gap or an orphan in the child's own resent history; P13 shows the path on a real run |
| The pad itself has a bug that hides a loss | `selftest` before trust; `pad_trusted_at` on every row |
| A simple table hides a wrong check (a crash step that killed nothing, a count read from the wrong file) | The first-output read of section 12 compares the table with the record by hand before any row is trusted, and again after a mapping change |
| A step waits forever, or an agent drives steps by hand around a stuck rig | Every step has a limit and the rig goes on to seal, check and clean up (section 1a); a stuck rig is a pad bug for PRD-12's watcher row, never a reason to drive steps by hand |
| A rig that dies leaves a box with keys or a test repo | Cleanup runs from a trap; boxes are `--non-persistent` with a timeout; keys are wiped in the box before the checks; a kept repo is a night-log line (section 3.2) |
| A `cycle` crash at minute 3 lands before any stream is open | The kill waits up to 60 s for an open stream, else kills anyway and says so (section 3.1); `wait-recovery` still needs calls written after it |
| With K3 and K4 cut, no real run proves C1 and C2 catch a silent loss | K2 proves the mapping on run 1's shape; PRD-08's `check_run1_shape.rs` proves the checks; PRD-16 A1 repeats run 1's load on 1.0. A silent loss in 1.0 would need a design bug PRD-02 section 12 rules out; if one shows, a real FAIL row is the evidence |
| Only Codex, Claude Code and Hermes have log readers in 1.0 (PRD-09) | The record is the proof (ledger T4): the other 12 are judged from it, with the local log check `skip` and `local_log: none` in the table, and may be "verified". The PRD-16 phase 5 reader step may still add a reader from their sample, as an extra check |
| Hermes before `sqlite.rs` lands, or run through its gateway with no `state.db` the pad reads | Judged from the record; the local log check is `skip` (PRD-09 ask 2) |
| A dodger simulated by a pinned list is not a real dodger | A list entry marked `pinned` takes the tunnel path a real pinning client reaches after refusal learning, but skips the learning itself. That path, with its one refused call, is proven by PRD-03 acceptance 7 and `proxy_tls.rs`, not by the dodger rows. A real one (Amp) is run when an account exists; Cursor stays the documented case |
| Typing into a TUI through tmux differs from a person typing | tmux is a real terminal; the paste uses bracketed paste when the harness asked for it. If a harness has it off, a multi-line first message can submit early: the pad then joins lines with spaces as `matrix.sh:109-118` does, and the row says so. Which tmux format shows a pane's bracketed paste state is unverified; the builder checks `tmux list-panes -F` first |
| `route-drop` cuts the box's own control channel | The route comes back by a detached timer in the box; the conductor retries `sandbox copy` and the spend check after it. Whether `sandbox exec` and `sandbox copy` use the box's default route is unverified; the first route-drop run checks it with a 30 s drop |
| `SIGSTOP` of the whole tree also stops tmux children the pad relies on | Only the goalcup and harness tree is stopped (from the watchdog pid down), never tmux or `pad-run` |
| The `/etc/hosts` cut does not take (a name cache) | `getent hosts` before and after; a local listener that refuses or holds the connection stands in, never terminating TLS; else "not run: cut did not take" (PRD-06) |
| Vercel Sandbox concurrency and egress limits | Unverified for 6 at once; the queue default is a value in `pad.toml`. The 28 Oct runs showed direct egress and no TLS re-termination (`2026-10-vercel-gateway.md`, Round 2) |
| Keys outlive the run | `--non-persistent`, `sandbox remove`, wipe step, no key in the image or a build box; `GH_TOKEN` never in a box, only the push token; C3 scans the pad's own output |
| The agent sends `.env.local` to the model provider | It holds only the push token (Contents read and write on the throwaway owner) and fake values; the real harness key lives in the isolated home or the launch shell, as for a participant |
| Provider outage looks like 15 failures | `provider_down` from recorded statuses only (`O-harness-access.md:266-281`); no status page is read (ledger T4) |
| `goalcup recover` needs the run certificate key on disk for the whole run (ledger R3b) | Accepted by the owner: a participant who meddles with it breaks the rules anyway. The pad adds the key to `known-box`, so C3 fails if it ever reaches the record, the server copy or a log |
| The harness loses its terminal when the watchdog dies, so `recover` has nothing to keep recording | The `kill-watchdog-recover` row is INCONCLUSIVE "harness lost its terminal", status must show the `goalcup continue` line, and the watcher follows PRD-10 acceptance 11 |
| A runaway run (Claude Code with 20 subagents) passes 10 USD inside 20 minutes | The spend check every 5 minutes seals it at 10 USD; between checks it can overshoot by up to 5 minutes of spend, and an open entry reads to its last whole frame, so the check errs low by at most one frame; the row says `over_cap`. Provider budgets are the backstop where they exist (unverified per vendor); the pad reads no provider usage page (ledger T4); the owner's dashboards stay the owner's check |
| The spend check slows the harness | It runs under `nice -n 19` every 5 minutes, like `check --live`; `procs.jsonl` shows its cost |
| A "verified" version is not the one people run on 7 Nov | The row names the version; connectors turn auto-update off (PRD-11); PRD-16 reruns the core set on 5 Nov |
| The release binary embeds a newer `tiers.toml` than the binary the pad ran | Only the tier text differs; `--release-check` excludes exactly that one file and nothing else |
| Staging shared by parallel runs | Each run has its own run and entry; checks read by entry id; test runs stay in `goalcup-traces-staging` |
| Linux only in this build | P9 on the owner's Mac. Hermes on macOS reads the OS trust store (appendix O H3.3); goalcup never adds the run CA to a keychain; if Hermes cannot trust it there, Hermes on macOS is "not supported on macOS" with that reason |
| No push token from the owner | Rows run without `--repo`; C3 says "push not tested"; everything else is checked |

## Overridable defaults

| Default | Value | Who may change it |
|---|---|---|
| Run length, `full` | 20 minutes after the paste | Owner |
| Run length, `micro` | 3 minutes | PRD-16 |
| Run length, `cycle` | 8 minutes, crash at minute 3 | PRD-16 |
| Step limits | setup 5 min, start 3 min, `wait-calls` 4 min, `wait-recovery` 3 min, `fetch` 5 min (section 1a) | PRD-12 builder |
| Per-run cap | 10 USD (ledger T6, locked), checked every 5 minutes during the run and sealed there; a `smoke` run near 2 USD is an estimate, not a cap | Owner |
| Build spend report lines | 200 and 400 USD model, 75 and 150 USD sandboxes; no whole-build cap (PRD-00 owner question 8) | Owner |
| Rate per model | The vendor's listed rate on the day, cached input at its cached rate; a model with no line at the highest rate for that key | PRD-12 builder |
| Parallelism | No total cap of our own (ledger B11); 2 per provider key; up to 6 for DeepSeek until its 429 share passes 5% (PRD-00 R3-9) | PRD-16 |
| Sandbox | 4 vCPU, timeout run length plus 30 minutes, `--non-persistent` | PRD-16 |
| Ready delay, submit delay | 30 s, 2 s, per harness override | Harness file author |
| Upload wait after the seal | 10 minutes | PRD-06 after measuring |
| Rerun of INCONCLUSIVE | once | PRD-16 |
| Failed fixes per cause | 3 | Owner |
| Tier needs | `direct`, `gateway` and `dodger` `full` PASS, trusted pad | Owner |
| Fault defaults | kill at minutes 8 and 10; freeze 10 min; cut 10 min; route drop 1 min; recover: recorder killed 1 minute after the watchdog, `recover` 1 minute later | Owner PRD of each fault |
| Canary minute | 5, only with the owner's yes to PRD-00 question 3 | PRD-05 |
| Push token | none until the owner provides one: rows run without `--repo` | Owner |
| Harnesses that get `keep-going` in `cycle` | Those whose harness file says `keep_going = true`; the core set's files do | Harness file author |

## Builder notes

Order of work (PRD-00 slices). The rig comes first and stays simple (ledger B9): build `micro` mode end to
end before anything else, then `cycle`, then `full`. Cut a flag, a setting or a step before adding one.

1. **Phase 0:** `scripts/pad/image` with the three core harnesses (PRD-00 phase 0 step 9); `/pad-out/` in
   `.gitignore`; `scripts/pad/table` seeds `connectors/tiers.toml` (section 9) for the integrator to
   commit with PRD-11's phase 0 connectors.
2. **Core slice:** `check.py` and `test_pad.py` against fixture S6, S7 and `fetch.json` JSON (S6 stable by
   H+3, S7 and U1 by H+4, S13 by H+3). Then the offline controls K2, K6, K9, K10 and K11 in `selftest`. This is
   the part everything else trusts; the offline controls run in a check box. Then `pad-run` and `pad`
   in `micro` mode (`where = "here"`, `server = "off"`) for Claude Code and Codex: phase 1a needs it (P2),
   and ladder rung 1 starts there. Before
   the sandbox lifecycle lands, the watcher creates a pad box by hand (`sandbox create --snapshot <pad
   image> --non-persistent`), copies in the binaries, `scripts/pad/` and one harness key, and runs
   `pad` with a `micro` settings file inside it, then removes the box (ledger B3); never on the conductor. Copy the approval, allowlist and
   scrub functions from `matrix.sh` with their line references in a comment.
3. **Second slice:** sandbox lifecycle and cleanup (section 3.2), then `server = "local"`, then staging,
   approval and `fetch-run` (ladder rung 2); `cycle` mode with its steps and waits (section 1a);
   the three cases for the core set (harness files for Claude Code, Codex and Hermes first, after reading
   each vendor's current docs or `--help` of the pinned version, cited in the commit, ledger T7); the
   spend check; faults, `--exec` and `--pre` scripts (`kill-watchdog-recover` once PRD-02's recover
   entry lands); the real controls K5, K7, K8; the first-output read (section 12); mark the pad trusted
   at the end of phase 2. Climb rungs 3 to 9 as each core feature lands.
4. **Third slice:** `table`, `tiers.toml`, `pad-rows.jsonl` and `--release-check`, the ledger and the queue;
   `full` mode; then the other 13 harness files, each written after reading the vendor's current docs
   (ledger T7), each followed at once by its `cycle` runs (P15); the image gains their harnesses.
5. The skill and templates; delete the two skill files.

Traps from the audit:

- Codex reads text and Enter in one write as a paste: send Enter after a delay (`matrix.sh:120-122`).
  Codex's `$CODEX_HOME/.env` overwrites `HTTPS_PROXY`, `NO_PROXY` and `SSL_CERT_FILE`
  (`O-harness-access.md:127-128`, `:657`): the gateway key file may hold only the key line.
- Panes show `.env.local` and `env`: replace known values at each copy-out, before a pane is written
  to the run folder (section 4), or C3 fails on the pad's own output.
- Claude Code asks "do you trust this folder" in every new workspace (`matrix.sh:123-129`) and refuses
  bypass mode as root (`O-harness-access.md:542`). Run as `ubuntu`.
- Hermes: stop any gateway for the same home first (`O-harness-access.md:125-126`; PRD-11's `before`
  does it in 1.0); one-shot mode caps delegation at 2 children (`O-harness-access.md:713`), so run the
  chat TUI as people do.
- `sandbox exec` is not a terminal; goalcup refuses a pipe. Run goalcup inside tmux
  (`2026-10-vercel-gateway.md`, Round 2 findings).
- Never compare raw wire attempts with harness calls: warm-ups, interrupts and refused retries are wire
  requests (run report section 4). C1 reads PRD-08's comparison, which already excludes them.
- Never scan or check a pruned folder (B5): `GOALCUP_KEEP_PARTS=1` always, except K8 and U5; C4 fails
  without it.
- Never let the watch re-read from the start after a restart (B9): append-only files with times.
- `--exec` reads `/proc/<pid>/environ`, which holds the harness's keys: filter by name before anything
  is stored or passed, and test the filter.
- Run 1's observer had no live alarm; the pad records the first `check --live` failure time.
- `goalcup recover` refuses inside the harness's environment: run it in the `pad:recover` window, never
  in the harness's pane or with `/proc/<pid>/environ` values (PRD-10 section 3.2).
- Never call an outside API to verify (ledger T4): no provider usage or billing lookup, no status page,
  no clone. If a check seems to need one, the fact belongs in the record; ask its owner.

How a watcher proves it in a real run: run `scripts/pad/selftest` (its offline controls run in a check
box) and confirm K2 and K5 to K11 in its output; then copy `scripts/pad/runs/full.toml` with
`harness = "codex-cli"`, `key = "openai"`, `case = "direct"` and run `scripts/pad/pad <that file>`; read
the table it prints; during the
run `tail -f pad-out/codex-cli/direct/<stamp>/watch/status.jsonl`; after it, read `row.json` and confirm
`subagents` above 1, `history.orphans` 0 outside gaps, `wire_requests` within tolerance of
`harness_calls` where `local_log` is `read`, `secret_hits` 0, C4 PASS, and `pad_trusted_at` set; then `scripts/pad/table` and read the Codex table in `PAD-RESULTS.md` and its
table in `connectors/tiers.toml`.

## Asks (ruled)

PRD-00 revision 2 ruled on revision 1's nine asks of PRD-00 (accepted except the hook, cut, and the tier
copy, now this PRD's own file) and PRD-01 accepted both asks of PRD-01 (`pinned`, `hosts_sha256`, `keep`
in the start event). PRD-00 R3-11 accepted asks 1 to 4 as written; they are kept for the owning
PRDs' builders. Asks 5 and 6 are new in revision 4 (ledger T4, locked) and open; the replan places them.

1. **PRD-16:** drop "`pad-run` seals at 80% of the live estimate" from its spend table (live pricing is
   cut; section 7 here), drop "tiers copied from `pad-out/tiers.json`" (the file is
   `connectors/tiers.toml`, written here), and drop "the silent drop hook for selftest" from its PRD-02
   row. Its four flags and `--release-check` are in section 1 and section 9.
2. **PRD-14:** the hook registry row `GOALCUP_TEST_SILENT_DROP` and the "pad's hook build" lines go (the
   pad uses no hook build); `--release-check` excludes `connectors/tiers.toml` from its diff (section 9).
3. **PRD-03:** state the route-drop pass line for the proxy side (section 3.1 uses "every later `conn` to
   a listed host is `decrypt`, no `refusal_after_trust`").
4. **PRD-08:** a shape hit is `fail` "unless the pad lists it as expected": the pad has no such list in
   1.0 and passes none; please drop the clause or name the input. Done by PRD-00 R3-11; PRD-08 reports a
   glued shape as `warn` `glued_shape`.
5. **PRD-09 (open, ledger T4):** add the history check to S7 as `history {state: pass | fail | skip,
   reason, answers, orphans: [{thread, sid, at, in_gap}]}`, by the match rule of section 8, worked out
   after upload from the record (ledger R8), with no harness name. Give `threads` and `parent` from the
   record when no native stream is read: a thread hint on the wire first, else the resent-conversation
   chains, a child named by its final answer coming back to a parent as a tool result. Write the K11
   fixtures. Without this, C1 and C2 cannot judge a harness with no local log.
6. **PRD-08 (open, ledger T4):** C1 is now `gap_has_end`, the `named` losses and the history check;
   `threads_match`, `wire_keeps_up` and `subagents_present`'s native half are the optional local log
   check. With `readable: false` they are `skip` "no local log" and must not lower the exit code on
   their own. PRD-08's T4 bullet and PRD-12 interfaces row ("C1 = `wire_keeps_up` + `threads_match`")
   follow.

## Review answers

History: revision 2's answers to the three reviews of 30 Oct. Where a row differs from the body, the
ledger or the alignment answers below, those win (for example the build stop line, gateway and dodger
at `smoke`, and the flat rate are superseded by L5-02, L5-03 and G3-X11). The move of this section to
`REVIEW-HISTORY.md` is left to one later step (alignment X2-11). "Accepted" meant the change was in the
body at revision 2.

### Consistency check

| Item | Answer |
|---|---|
| Registry (seam numbers) | Accepted. Revision 1's S11, S12, S13 are now S14 (cut), S15 (`setup --yes`), S16 (`--connector`, `--hosts`); S13 is the fetched layout |
| 2 Defaults (night cap) | Accepted. 400 USD model and 150 USD sandboxes as the build stop line, from PRD-00 question 18 and PRD-16 (section 7) |
| 10 Paths with two owners | Accepted. This PRD deletes its two skill files itself; the cut table no longer lists them under PRD-13 |
| 11, 100 Connector layout | Accepted. Flat `connectors/<id>.toml`; contributor step 1 and section 1 changed |
| 17 Dodger exit code | Accepted with PRD-08 R4: exit 1, from `listed_hosts_decrypted: named`; `wire_keeps_up` `named` where it runs, `skip` for a totals harness |
| 57, 75 One override mechanism | Accepted. `goalcup start --hosts <copy>` (S16); no `GOALCUP_CONNECTORS_DIR` |
| 73, 74 `no_prompts`, `setup --yes` | Accepted. S15 replaces `kit_settle.py` in the box |
| 79 Fetched layout | Accepted. C4, C6 and K6 read `fetch.json` (S13) |
| 84 Rate-limit range | Accepted. B7 says 95 to 99, with the three sources; K1, which used 97 to 99, is cut |
| 86 `test-hooks` in the guard | Accepted as ruled by PRD-00; moot here, since the pad uses no hook build after K3 and K4 are cut |
| 93 Root and build host | Accepted. Section 13 step 5 rebuilds in a build box as `ubuntu`; the conductor never compiles |
| 96 Flags PRD-16 needs | Accepted. `--prompt-file`, `--minutes`, `--keep-going`, `--kill-recorder` in section 1; `--release-check` in section 9 |
| 97 Harness files count | Accepted. 16 pad harness files; OpenCode is one table row with both versions |
| 98 Tier file owner | Accepted. `scripts/pad/table` writes `connectors/tiers.toml`, this PRD's only path under `connectors/`, in PRD-10's T1 shape |
| 99 Test bucket | Accepted. `goalcup-traces-staging`; no separate bucket (PRD-07) |

### Review A

| Finding | Answer |
|---|---|
| 1 Refusal learning pad case with a delayed upstream | Rejected as a pad case: it needs a delay hook in product code, and PRD-03 covers it with `proxy_tls.rs` ("a plain close teaches nothing"). The pad's `route-drop` fault checks the real-world shape: no `refusal_after_trust` after a network cut |
| 4d Kill case per core harness | Accepted. P10: `--kill-recorder 8,10` on Codex, Claude Code and Hermes `full` |
| 12 Canary `sk_test_` at minute 5 | Partly. Added to `full` runs as a shape-rule check (PRD-05's offer); mid-run reading of new `.env` values is rejected by PRD-05, so a canary the agent never reads is "not exercised", not a fail |
| Coverage: Linux sleep | Accepted. `freeze@6+10` on Codex (P11), with PRD-06's pass line; it proves catch-up, not the clock (PRD-02) |
| Coverage: network change on the harness leg | Accepted. `route-drop@6+1` on Codex (P11) |
| Coverage: hours of many subagents before the 2-hour run | Routed to PRD-16 (A2, Claude Code with up to 20 subagents, 60 minutes). The pad provides `--minutes` and `--prompt-file` |
| 5, 6, 7, 8 and others | Not this PRD's; owned by PRD-02, PRD-03, PRD-10 and PRD-11 |

### Review B

| Item | Answer |
|---|---|
| C9 Fetched layout | Accepted (consistency 79) |
| C11, C12 Flat files, one override | Accepted |
| C13 `setup --yes` | Accepted (S15, owner question 8 default yes) |
| C15 `test-hooks` | Accepted as ruled; the pad needs no hook build |
| C16 Rebuild on the conductor | Accepted. Build boxes only |
| C25 Seam numbers | Accepted |
| A4 Phase 1a slice | Accepted. `pad-run --where here` is in the core slice (builder notes) |
| A8 Native readers for 12 harnesses | Accepted. C1 INCONCLUSIVE "no log reader" gives "not verified: no log reader"; a reader may come from a phase 5 sample (PRD-09). Superseded by ledger T4: C1 is judged from the record and a missing reader is no tier reason |
| A11 Dodger exit code | Partly. The observable pass lines are in section 2; exit 1 stays, because PRD-08 derives it from the named `pinned` tunnels, which every core harness has, not from `wire_keeps_up` |
| R1 Hermes cannot pass C1 | Accepted, option (a) per PRD-00: C1 passes by the totals form |
| B6 Real-run phase size | Accepted. Tier and done item need gateway and dodger at least `smoke` |
| N1 Run 1 converter | Accepted. K1 cut |
| N4 Live spend pricing | Accepted. No `prices.toml`, no seal at 80%; a flat-rate ledger after the run keeps a number for the build stop line, with an `over_cap` rule (review B's own blind spot on keys without a budget) |
| N5 Silent-drop hook, K3, K4 | Accepted. Cut; risk row added |
| T5 P5 and P8 | Accepted. P5 is shown by phase 4 waves; P8 after the merge to main |
| W1, W2 Commands the agent may not run | Accepted. `--exec` with four scripts, run in the harness's proxy and CA environment |
| W4 Unknown gateway by hint | Partly. PRD-11 owns the case; the pad passes its host-list copy with `--hosts` |
| W5 How to cut the Worker | Partly. PRD-06's `/etc/hosts` line first, a TCP forwarder if `getent` shows it did not take |
| W10 Kept parts | Accepted (already the rule; C4) |
| M8 Tier file | Accepted (consistency 98) |
| O6 Pad image in phase 0 | Accepted. Three core harnesses first |
| O7 Phase 1a with `--server off` | Accepted (P2) |
| Blind spot: provider budgets unverified | Accepted. Risk row and `over_cap` rule |

## Alignment answers

Revision 3 answers the three fix lists for this file (groups 1 to 3) from the alignment reviews. The
ledger wins; no locked item is changed and no scope is added past what the ledger, PRD-00 revision 4 or
another PRD's acceptance asks of the pad. "Applied" means the body now says it; "partly" says what was
taken; "rejected" says why. Fixes for other files are made by those files' owners.

### Group 1

| Id | Answer |
|---|---|
| L4-1 | Applied. Owner decisions, section 5, interfaces PRD-05, defaults: no `sk_test_`, `whsec_` or canary unless the ledger records the owner's yes to PRD-00 question 3. Also fixed the effect PRD-05 names: the no-shape value is a stated loss in the default build (its own known-values file, `facts.plain_hits`, no FAIL), and `env` output is masked by shape, not by exact value |
| L5-01 | Applied, merged with G3-X11. Section 7: `pad` runs `usage --json` on the open entry in the box every 5 minutes on every run, prices it per model and seals at 10 USD; `over_cap` and a night-log line. Risk row, watch table, defaults and interfaces follow. Same wording as PRD-00 "Not in this build" |
| L5-02/L6-09 | Applied. No whole-build cap and no `smoke` cap; report lines at 200 and 400 USD model and 75 and 150 USD sandboxes (PRD-00 owner question 8). Section 7, defaults, PRD-16 interface row |
| L6-05 | Applied, merged with G3-X1. Two-halves table, section 3 rows 0:24 to 0:28, section 7 known-values file, section 8: every `goalcup-verify` command runs in the pad box; the conductor fetches, copies in and reads the JSON. Selftest offline controls run in a check box (section 12); `--where here` never on the conductor |
| L3-3 | Applied. `--release-check` also needs `PASS` verdicts for the `h2-parallel.sh` and `long-tunnel.sh` exec rows (PRD-00 done item 9 names both); watchers write `exec/<script>.verdict`, carried in `row.json` `exec` |
| L5-03 | Applied, merged with G3-X13. Owner decisions, task-text note, tier rule, `--release-check`, P3, interfaces PRD-00, defaults: all core rows `full`; `smoke` for phase 1a, short build tests and watcher reproduction |
| L5-04 | Applied (PRD-12 part): risk row names the PRD-16 phase 5 reader step; PRD-09 interface row too |
| L5-07 | Applied, merged with G3-12d. Section 6: `key_route` (`login`, `file`, `env`) with `key_file`, `key_line`, `key_env`, for `direct` and `gateway`; section 3 step 0:01 and section 7 follow; a `test_pad.py` case checks an exported key is never in argv. Claude Code's example is `key_env`, because PRD-00 owner question 2 passes the participant's environment to the harness |
| L6-10 | Partly. Taken: the conductor creates the repo with `GH_TOKEN`, which never enters a box. Rejected: the deploy key. PRD-10 start step 2 needs a `GH_TOKEN=` line in `.env.local` and its push block pushes over HTTPS with that token, and step 3 needs a public repo; an SSH deploy key would not run the participant's path (point 7). G3-X12's fine-grained push token on a throwaway owner (the rule `matrix.sh:92-104` already enforces) is used instead |
| L7-01 | Applied. "Files it owns" Change line and section 1: `/pad-out/` in `.gitignore` in the phase 0 commit; builder step 1 |
| L7-02 | Applied. Section 6: `[direct]` `files` and `env`; provider, model and base URL settings are pad data, never connector data; launch facts stay PRD-11's |
| L5-08 | Applied. Builder step 3 |
| L5-12 | Applied. Section 7: `pad` writes each key from its environment secret into `~/.pad-keys/`, 0600, `umask 077`, no echo |
| L5-13 | Applied. INCONCLUSIVE cause `harness_subagents_unavailable`, signed by PRD-11's OpenClaw outlook |
| L6-18 | Applied. Section 13 steps 2, 6 and 7 and the lock rule: one named cause; the signature only groups rows |
| L7-04 | Applied. P9 and the Linux-only risk row |

### Group 2

| Id | Answer |
|---|---|
| P00-1 | Applied (see L5-02/L6-09) |
| 06-17 | Applied (PRD-12 part). Section 3.1 `cut-worker` and the risk row: the stand-in is a local listener that refuses or holds the connection and never terminates TLS; else "not run: cut did not take" |
| X2-07 | Rejected. Other PRDs name these faults with machine pass lines: PRD-06's sleep stand-in is the `freeze` row, PRD-03 and PRD-16 route the network-change case to `route-drop`, and PRD-02 R4, R5 and R7 are real-run proofs whose pass lines (seal within 35 s, `recovered` with `abandoned`, raised `fd_limit`) `check.py` reads. Moving them to `--exec` scripts would move the same work to a watcher's reading and lose the machine check; it is not less code across the PRD set |
| X2-11 | Partly. Moving "Review answers" to `REVIEW-HISTORY.md` needs a write to a second file, which this pass may not make, and many writers of one new file would race (as PRD-00 revision 4 answered). "Review answers" now opens with a line saying it is history and naming what it superseded; the asks are marked ruled. The move may run once, after every alignment pass, by one agent |

### Group 3

| Id | Answer |
|---|---|
| G3-X1 | Applied (see L6-05), with builder step 2's hand-made pad box for phase 1a, P2, the K2 fixtures under `scripts/pad/fixtures/`, and section 13 step 1's `dump` in a check box |
| G3-X11 | Applied (see L5-01): rate lines per model, `priced_high`, the spend check every 5 minutes. Run by `pad` on every run, not by a watcher on `full` rows only, as PRD-00 "Not in this build" now says |
| G3-12a | Applied. Section 4: known values in panes and the last screen become `[known:<n>]` at copy-out, counted in `facts.screen_known`; C3 scans after it; a test and a trap note |
| G3-X12 | Applied. Section 3 step 0:00 and 0:01, section 5, section 7, C3, tests, risks: the conductor creates `<pad owner>/gc-pad-<id>` with `GH_TOKEN`; the box gets only the push token as `GH_TOKEN=` in `.env.local`; `gho_` and `ghp_` refused. Until the owner provides the token, rows run without `--repo` and say "push not tested" |
| G3-12b | Applied. Section 9 and builder step 1: phase 0 seed of `connectors/tiers.toml` |
| G3-12c | Applied. Section 9 item 3 `pad-rows.jsonl`; `--release-check` reads it; files table and tests |
| G3-12d | Applied, merged with L5-07 |
| G3-X25 | Applied. Section 7 queue row, defaults, a queue test |
| G3-12e | Applied. P11 `--minutes 14` for U3 and U5; step 0:24 upload wait |
| G3-12f | Applied. `exit_keys` in section 6; `kill-watchdog` and `kill-both` rows |
| G3-X13 | Applied (see L5-03) |
| G3-X35 | Applied. `--pre <script>` in section 1 and 3.1, with `hint-local.sh`, `proxy-setting.sh`, `stale-gateway.sh` (and a `daemon` value in the harness file for Cline's hub and OpenCode 2's service, PRD-11 ask 6); P11 and files table |
| G3-12g | Applied. Section 2 and the risk row |
| G3-12h | Applied. Section 8: the INCONCLUSIVE causes first, then the did-work gate, then the checks |
| G3-12i | Rejected as superseded: L5-02 and P00-1 remove both rows, so there is nothing left to label |
| G3-12j | Applied. "Today" row, owner decisions, files table and the allowlist test: production always refused, no flag |
| G3-X42 | Applied. Step 0:03: approval key and member id, each piped from its own 0600 file |
| G3-12k | Applied. Owner questions cited by PRD-00 revision 4 numbers (3, 4, 7, 8) |
| G3-12l | Applied. The `linux-check.sh` row is gone; totals 6,208 and 4,482; PRD-14 deletes it |
| G3-X21 | Applied. Asks item 4 marked done by R3-11 (`warn` `glued_shape`) |
| G3-X58 | Applied. Section 13 |
| G3-12m | Applied. K2 row |
| G3-12n | Applied. C6 names `written.model_requests` and `written.off_ms` (PRD-02's `status.json` keys) |
| G3-12o | Applied. Section 1 `--build` default |
| G3-X37 | Applied. Section 2 dodger row: the launch block's `Host list from a file` line and the end screen, as PRD-10 acceptance 3 says |
| G3-X43 | Applied. `Cargo.toml` and `rust-toolchain.toml` in the compared paths, and a test |
| G3-X45 | Applied. Interfaces PRD-17 row |

### Ledger update (30 Oct)

| Id | Answer |
|---|---|
| R3a, R3b, T4 carried in | **T4:** the proxy record is the proof. C1 is now "No missed call": no unexplained gap plus the new history check (section 8: match rule, orphans, `skip` where a harness does not resend history, a hint not a proof); `threads_match`, `wire_keeps_up` and `subagents_present` on the manifest became the optional local log check, `skip` with no log. C2 and C5 read threads, parents and tokens from the record, native streams added where read. So a harness with no local log (Hermes through its gateway, Amp) can reach "verified"; "no log reader" is gone as an INCONCLUSIVE cause and a tier reason (sections 8, 9, 11, risks, P2, P3, P4, P8). No outside API is called to verify: the pushed-repo clone became a scan of the project's git objects in the box, `provider_down` no longer reads a status page, and the pad reads no provider usage page (sections 7, 8, risks, a no-outside-call test). New: control K11 (history fixtures, including a clean record with no native stream), P13 (a real no-local-log row), row facts `local_log` and `history`, a watcher row for orphans, asks 5 (PRD-09) and 6 (PRD-08). **R3b:** new fault `kill-watchdog-recover@m` (section 3.1): kill the watchdog, `goalcup recover` (expects "nothing to recover"), kill the recorder a minute later, `goalcup recover` again from a clean window; pass lines from PRD-10 acceptance 11 read from the record, plus P12, a fixture test, a defaults row and two risk rows. The run certificate key now on disk is added to `known-box`, so C3 proves it never leaves the box in the record, the server copy or a log. **R3a:** the `kill-recorder` rows now also expect no `null_mode` gap (kills 2 minutes apart are no crash loop); the crash loop stays PRD-02's proof. PRD-08's interfaces row for PRD-12, PRD-09's "no reader means C1 INCONCLUSIVE" lines and PRD-16's "not verified: no log reader" wording are those files' owners' to follow |

### Ledger update 2 (30 Oct)

| Id | Answer |
|---|---|
| B3, B9, B10, B11 carried in | **B9:** the pad is now the simple rig. One command, `scripts/pad/pad <settings file>`; the long flag list is gone, replaced by one small settings file (harness, key name, case, mode, and optional prompt, minutes, steps; an unknown key is a usage error) and a table mapping the flags other PRDs name to settings (section 1). Every step has a built-in wait and a limit, and a timed-out step never stops the rig (section 1a). The rig prints one plain table, one line per step and the result, also kept as `table.txt`. The server allowlist became "staging and loopback only, no setting", so production cannot be chosen. Section 12 keeps `selftest` (K2, run 1's shape, first) and adds the first-output read: a reviewer who did not write the pad checks the first `selftest`, `micro` and `cycle` outputs against the record by hand before `pad_trusted_at` is set; P1 and a risk row follow. **B3:** three modes, `micro`, `cycle` and `full` (section 1a), with the `cycle` steps listed (setup, start, wait-calls, look, kill-recorder, wait-recovery, keep-going where the harness file says so, seal, wait-upload, fetch, check, cleanup) and the ladder's rungs as settings changes (local first, then upload, subagents, recorder crash, watchdog kill and recover, keep going, gateway, dodger, HTTP/2). New `micro` prompt with no subagent; `cycle` uses the payments prompt sealed at 8 minutes. Only `full` rows give a tier. New section 3.2: cleanup after every run (box, keys, test repo deleted with `GH_TOKEN`, local files trimmed, kept only for a watcher's FAIL). New P14 (the rig walks by itself) and P15 (cycle runs per pad); P2, P9, P11, P13, section 11 and section 13 now use settings files. **B10:** `test_pad.py` shrinks from about 270 to about 90 lines: it keeps the key, `exec` filter and pane tests (point 4) and the crash and recover fixtures (never-stop rules); the mapping tests go (`selftest`'s controls prove the mapping), as do the queue, spend, INCONCLUSIVE, table and release-check tests, left to real runs and the first-output read. **B11:** no total sandbox cap of our own (it wins over PRD-00 R3-9's 6 in all); the per-key queue stays, because rate limits make rows INCONCLUSIVE; offline controls never queue; P5 and the PRD-16 interface row follow. Total new lines about 1,400 (was 1,500) |
| B3, B11 sweep (30 Oct) | Applied. P11's fault rows run in phase 4 beside the `full` wave, as PRD-16 section 6 says (full runs at the end). The watcher count is one per named cause with no cap of our own. The hand-made pad box of builder note 2 is removed after its run |

## Amendments

### 2026-10-30, replan after PRD-12 slice 2 (merge c7649890)

- **Where the slices landed.** Merge c7649890 (titled "slice 1") carries both slices: the branch head
  was 62bd0ed3, the slice 2 review fixes included. The later "slice 2 merge" logged as 15a6963b is the
  PRD-07 replan commit; the branch was already in (PRD-16 note).
- **Three review nits to apply in the next slice, all cuts** (reviewer B, third round):
  - B3-1: `PLANTED` in `check.py` becomes `(("openai_key",), ("github_token",), ("google_key",
    "generic_secret_assignment"), ("jwt",), ("private_key",), ("url_password",))`, and the comment says
    only that the Google line is masked under the shell rule, row 11, which wins over `google_key`, row
    13. By PRD-05 section 10, `openai_key` (row 1) and `github_token` (rows 3 and 4) win their overlap
    with row 11, so the shell rule alternative on those two is dead and would hide a broken OpenAI or
    GitHub shape rule behind an unrelated MASK.
  - B3-2: in `secrets()`, test `masks is None` first ("no MASK counts by rule in
    verify/secrets.json"), then the exercise test without the `(masks or {})` guard. Same result, the
    right why line: until PRD-08 adds `masks`, the cause is the seam, not "secret exercise not done",
    which would send a watcher to rerun the agent.
  - B3-3: one spelling per limit. `step_check` uses the `limit` it is passed (plus 120, as before)
    instead of recomputing `runner_limits(...)["verify"]`, and `step_cleanup` drops the two parameters
    it ignores.
- **C3 cannot pass yet.** Until PRD-08's `secrets --json` reports MASK spans by rule kind under
  `masks` (PRD-08 note), C3 is INCONCLUSIVE on every run, so P2's "C3 PASS" waits on PRD-08's secrets
  slice and on the masker (PRD-05), as well as on a `goalcup` build on integration. Neither is merged.
- **C1 with a history `skip`.** `check.py` lets a history `skip` pass C1, as section 8, K11 (c) and
  ledger T4 say. PRD-08's interfaces row line "C1 is INCONCLUSIVE only when `history_complete` and both
  optional checks are `skip`" is the stale one (PRD-08 note); the mapping stays here.
- **C3 depends on PRD-05's row order.** `PLANTED` holds today's S12 kind names and relies on rows 1, 3
  and 4 winning over row 11, and row 11 over row 13. A rename or reorder in PRD-05 is posted as a seam
  line and followed here (PRD-05 note).
- **Size, `wc -l` on integration at c7649890:** `pad` 419 (row 320), `pad-run` 373 (row 330),
  `check.py` 352 (row 320), `test_pad.py` 214 (row 90), `table` 84 (row 150), `image` 54 (row 60),
  `sbx.mjs` 133 (not in the files table; it is the sandbox lifecycle of section 3.2, so it counts
  against the pad's total), `pad.toml` 17, harness files 92, prompts and settings 34; 1,771 under
  `scripts/pad/` against "about 1,400". Still to land at their rows: `selftest` 100, `exec/` 70, the
  other 13 harness files (about 195) and the skill (100), plus the staging, approval, fetch, copy-in,
  spend, fault, keep-going, `pre`, ledger, queue and release-check steps in `pad`, `pad-run` and
  `table`. Before each later slice adds lines, cut what serves no point (the three nits above first);
  `test_pad.py` adds no mapping test (those are `selftest`'s controls, ledger B10), only point 4 and
  never-stop cases.
- **Python.** `pad` and `pad-run` need 3.11 (`tomllib`; the pad image installs `python3.11`);
  `check.py` and `table` run on 3.9. `test_pad.py` runs under 3.11 (PRD-14 note).

### 2026-10-30, replan after PRD-05 phase 0 slice (merge 82484521)

- **S12 row order confirmed** on integration: counting data lines from 0, `openai_key` 1,
  `github_token` 3 and 4, `jwt` 7, `private_key` 8, `generic_secret_assignment` 10 and 11,
  `url_password` 12, `google_key` 13, as C3 reads them.
- **A pad run on a stub build is not evidence.** With the S2 stub every MASK is `fail_closed`, so C3
  fails on every planted value. PRD-05 builder note 1 keeps real keys out of pad boxes until
  `start.mask_rules` has more than one entry (PRD-16 checks it); the stub's list is
  `["fail_closed"]`.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-12 slice 3 (merge 9e259b4d)

- **Gate.** `gate fast: PASS in 82s` on 9e259b4d. The gate does not run `test_pad.py` yet (PRD-14 note),
  so the 17 pad tests were green only in the fixer's build box at 9e9e384a.
- **B3-1 to B3-3 are in:** `PLANTED` with the shell rule only on the Google line, `masks is None` tested
  first in `secrets()`, `step_check` uses the `limit` it is passed, `step_cleanup` takes no unused
  parameters.
- **Four post-merge review nits (reviewer B) for the next slice, three of them cuts:**
  - B1: one helper, `env_without(hide=NEVER_IN_BOX)`, replaces the three copies of the env filter in
    `sbx()`, `box()` and `step_fetch()` (which passes `set(NEVER_IN_BOX) - set(FETCH)`). `box()` always
    passes `env=env_without()`; the `None if argv` branch goes, so `check.py` in a here run no longer
    gets the organizer's whole environment.
  - B2: delete `test_local_server_in_a_sandbox_is_refused`. It guards a refusal a later slice removes
    and none of the ledger B10 subjects. The other two `HereAndFetch` tests stay (point 4).
  - B3: `release_check` reads `sha + ":" + ROWS_REL`, not `"HEAD:" + ROWS_REL`, so its result depends on
    its argument alone. Until it lands, `--release-check <commit>` is right only from a checkout whose
    HEAD is `<commit>` (PRD-14 and PRD-16 notes).
  - B4: the size, below.
- **Fetch-run cleanup leans on PRD-07's folder name.** After a timeout `step_fetch` removes
  `.fetch-run.*` next to the fetched entry, because a killed `sh` skips its EXIT trap and leaves
  `pg_service.conf`, which holds the database connection (point 4). A rename of that work folder in
  `scripts/fetch-run.sh` is a seam line and is followed here the same day (PRD-07 note).
- **Size, `wc -l` at 9e259b4d, fixtures left out:** `pad` 500 (row 320), `pad-run` 411 (330),
  `check.py` 358 (320), `table` 192 (150), `test_pad.py` 287 (90), `image` 54 (60), `sbx.mjs` 133 (no
  row), `pad.toml` 17, prompts 16, runs 16, harness files 92 (3 of 16), `image.id` 1. In all 2,077
  under `scripts/pad/`, 306 more than at c7649890 (1,771), against PRD-00's "about 1,350 to 1,450 with
  its tests": 43% over the top of the row. Still to land at their rows: `selftest` 100, `exec/` 70 and
  13 harness files (about 195), which alone make about 2,440, before the spend, fault, keep-going,
  `pre`, ledger and queue steps.
- **Replan decision on the size (PRD-00 size rule, more than 10% over).** The lines past the row serve
  point 4 (the env filters, the approval key and member id read in the process from named env vars, the
  conductor's scan for its own secrets, fetch credentials kept out of the box, the work folder cleanup),
  point 5 (wait-upload on S9, fetch-run and the copy-in, `check --server`, `secrets` and `usage` on the
  fetched copy) and point 3 (a limit on every step; a timed-out step never stops the rig). This is not
  a raise of the row. Before the next slice adds a line to `pad`, `pad-run`, `check.py`, `table` or
  `test_pad.py`, it applies B1 and B2, and reviewer B names which `test_pad.py` tests fall outside the
  ledger B10 subjects (point 4, never-stop, PRD-12 acceptance) so they go; each later slice reports per
  file against the rows above. Whether the row itself rises stays the open owner question logged at
  12:53Z; no new one.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-11 slice 2 (merge b9fa50f8)

- A pad row run on an integration build between b9fa50f8 and PRD-11's hosts and launch merges is not
  evidence: `decide` tunnels every host (zero decrypted requests), no hint or `local_model` note, no
  home lock, no version, and `update_off` is not written. The 1a micro rungs wait for those two merges
  plus S8 `watchdog::run` and PRD-10's `start`.

### 2026-10-30, replan after PRD-14 slice 1 (merge 22fe5926)

- **`gate fast` now runs the pad's tests:** `python3.11 -m unittest discover -s scripts/pad -p
  'test_*.py'` (17 tests on 22fe5926), with python3.11 installed in a build box when no Python 3.11 or
  later is found. The earlier "the gate does not run `test_pad.py`" notes are closed. Any pad test
  that fails now blocks every lane's merge, so a pad slice runs `gate fast` before it asks for a merge.
- `release.sh` and `packaging/README.md` now name `scripts/pad/table --release-check <commit>` as the
  check after `gate full`; `tag` does not run it yet (PRD-14's next release slice). PRD-12's nit B3
  still decides whether that call is right from any checkout.

### 2026-10-30, replan after PRD-08 slice 1 (merge d10feef7)

`goalcup-verify` is on integration with `check --json` (S6) for the integrity checks. What
`check.py` reads, as merged (PRD-08 note):

- A `recorder_down` gap's cause and length: the `inv6` check's `facts.gaps`, a list of `{cause,
  start, end, ms}` (`start`, `end` UTC text). `counts.gaps` is a map from reason name to count.
  `check.py`'s fallback to `counts.gaps` keys is only for older output.
- `keep`: each S6 entry carries `keep` from the start event, so C4 can read it from the record
  instead of the settings.
- `--local`, `--known` and `--harness-files` are accepted and read nothing yet; the checks that need
  them, and `history_complete`, `wire_keeps_up`, `threads_match`, `subagents_present` and the
  `server_*` ids, are `skip` "not built in this slice", so C1 and C2 stay INCONCLUSIVE and C3 waits
  on `secrets --json` `masks`.
- `--live`: null mode is read from `status.json` `off.since` only; a live entry with nothing
  durable yet exits 1 with `inv1` `skip`; a part file that cannot be opened exits 3.
- Selftest's K2 still waits on PRD-08's run 1 shape builder (next slice, with U1).
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-14 slice 2 (merge 04886295)

- `scripts/release.sh tag` now runs `python3 scripts/pad/table --release-check <sha>` from a detached
  worktree at origin/main's sha, so nit B3 no longer makes the tag judge another commit's rows. B3 is
  still worth taking (a run by hand from any checkout), but the release no longer waits on it. `table`
  must stay 3.9-clean, since `release.sh` calls `python3`.
- `CODEOWNERS` now names `/scripts/pad/` (`@nembal`).
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-06 slice 1 (merge e6bd8a50)

- `upload.json` (S9) is on integration, but no sender is yet. Until PRD-06's send and beat slices
  merge, a pad run with a server ends with `upload.json` at `state: waiting` and `final: none`, so
  `wait-upload` times out. Rows that need the server copy (C4's server checks, U3, U5, U8) are then
  "not run: sender not merged", not FAIL. `server = "off"` runs are not affected. The `watch/upload.jsonl`
  reader can be built against the S9 type now (`goalcup::upload::state`).
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-08 slice 2 (merge 669f5d92)

`check --server <fetched> [--local <entry dir>]` now gives real server results (PRD-08 note). What
`check.py` reads for C4, C6 and K6, as merged:

- The three ids `server_chunks_whole`, `server_equals_local` and `server_counts_match` per entry, and
  the top-level `server` object `{chunks, whole, equals_local, db_match}`. `equals_local` and
  `db_match` are true only when every entry passes, false on any `fail`, else null. `whole` is true
  only when every entry passes.
- Under `--server`, a folder with no `fetch.json` gives all three `fail`. A bare `rec.gcr.zst`
  checked without `--server` skips them, so C4 always passes `--server` on a fetched copy.
- `fetch.json` `missing.why` set gives `server_chunks_whole` `fail` with `first.o` =
  `missing.offset`. `final` null alone gives `server_chunks_whole` `named` (exit 1) and
  `server_counts_match` `skip` "not sealed". C4 reads the open end as INCONCLUSIVE, not FAIL.
- `server_equals_local` is `skip` when the `--local` entry's first part is pruned or its `rec/` is
  empty. A `--local` path that never held a record exits 3, which is the box's mistake, not the
  run's.
- K6's three tampers map as follows. A flipped byte fails `server_equals_local`; L-FW9 proves it.
  A removed middle chunk fails `server_chunks_whole` on the offset or seq break. `model_requests`
  plus one fails `server_counts_match`; the check reads `model_requests` as the seal's `http_post`
  plus `ws_up_msgs`.
- K2 still waits on the run 1 shape builder (PRD-08's legs slice). U1's stub is on integration, so
  that slice need not wait for PRD-09.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-09 slice 2 (merge dab0ce1f)

- **`goalcup-verify usage` still exits 3** at dab0ce1f: the wire leg is read inside U1, but `run`, the
  native leg and S7 output come in PRD-09's next slices. Until they land, a pad run's spend check and
  C5 record `usage` as not read (the row says so); exit 3 is never priced as 0 USD or read as 0
  tokens. U1's `readable()` is a method now (PRD-09 B10); C1 reads it through PRD-08 as before.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-12 slice 4 (merge 69f355c6)

- **Gate.** `gate fast: PASS in 137s` on 69f355c6 (nextest 188 of 188, 16 pad tests). Branch head
  e598500c; all 10 non-merge commits in 00fac28d..69f355c6 carry both trailer lines. There was no real
  key run: the proof is fake-binary probes in build boxes.
- **Built:** `kill-recorder@m`, `wait-recovery`, `kill-watchdog-recover@m`, `keep-going`, the
  detached watch of section 4, the 5-minute spend check with `[rates]` in `pad.toml` and the seal at
  `cap_usd`, the `pad-out/spend.jsonl` ledger, and PRD-11's flat `connectors/<id>.toml` (top-level
  `home`, with `{home}` filled, and `native`). The other six faults (`kill-watchdog`, `kill-both`,
  `freeze`, `cut-worker`, `route-drop`, `fd-limit`), `exec` and `pre` still answer skip.
- **B1 to B3 of the slice 3 note are in.** The 13:25Z question on `test_pad.py`'s B10 ground is closed
  by the review rounds: the C3 mapping tests are gone. `test_known_value_hit_fails`,
  `test_conductor_scan_counts_its_own_secrets` and `test_create_timeout_still_removes_the_box` stay
  (point 4; the last one guards a box left holding keys).
- **One review nit for the next slice, a cut:** B6-1, drop the inline comment on
  `got["wait-recovery"] -= 30` in `runner_limits`; the docstring stays the one explanation.
- **Open-stream sign.** S4 has no open-stream count. `kill-recorder` waits up to 60 s for
  `last_request_at` within the last 10 s, then kills anyway, and its line says "N s after a written
  request" or "no request written yet", not section 3's "on an open stream". If PRD-02 adds a count to
  S4 (PRD-02 note), the pad reads it through `SEAM` in place of the sign.
- **wait-recovery** reads S4 `pid` (a new one), `mode` `recording`, `off.spans` (`recorder_down`
  spans on sid 0: one more than at the kill), `off.since` null, and `written.model_requests` past its
  value at the kill. It ends 30 s before its step's limit, so its own FAIL line is written before the
  step times out (B5-1).
- **A seal at the cap before a fault** makes each fault window holding it INCONCLUSIVE, never FAIL
  (review A-N3, A-N6); the record-only checks still run.
- **The cap cannot seal a run yet.** On integration `goalcup seal` and `goalcup recover` exit 3 "not
  built" (PRD-10 note). `seal-now` then writes a FAIL line and is tried again at each later check. So
  no run with a real key starts before PRD-10's `seal` merges (ledger T6), besides PRD-05's masker
  (PRD-16 note), and a `kill-watchdog-recover` row is not evidence before `recover` merges.
- **Spend ledger.** One line per run in `pad-out/spend.jsonl`: `at`, `connector`, `case`, `mode`,
  `key`, `run`, `entry`, `usd` (priced at the check step, null when usage was not read), `usd_live`
  (the last live estimate, null if never read), `over_cap`, `models`, `priced_high`, `box_minutes`.
  The scribe sums `usd`, else `usd_live`. A model with no rate line is priced at its key's highest
  listed rate. The rates were copied on 30 Oct (the commit cites the pages); `gemini-4-argon` doubles
  from 1 Dec, after the 6 November date, so no change is due in the build.
- **Pricing** reads S7 `models[].tokens` `input`, `cached`, `cache_write` and `output`; `reasoning`
  is never added, because `output` holds it (PRD-09 section 4). Until PRD-09's `usage` stops exiting
  3, every spend line has `usd` null and the run says usage was not read.
- **Size, `wc -l` at 69f355c6:** `pad` 610 (row 320), `pad-run` 658 (330), `check.py` 429 (320),
  `table` 192 (150), `test_pad.py` 308 (90), `image` 54 (60), `sbx.mjs` 133 (no row), `pad.toml` 31,
  prompts 16, runs 16, harness files 92, `image.id` 1. In all 2,540 under `scripts/pad/`, 463 more
  than at 9e259b4d (2,077), 75% over the top of PRD-00's "about 1,350 to 1,450". Still to land:
  `selftest` 100, `exec/` 70, 13 harness files (about 195), the skill 100, the six faults, `pre` and
  the queue. The slice 3 size decision stands. The lines added here serve point 3 (each fault step
  has its own limit, wait-recovery ends before its step, the spend check never fails a row and starts
  when start is tried) and point 4 (the watch keeps only the replaced pane, `recover` runs under
  `env -i`, the ledger holds names and numbers only). Whether the row rises stays the 12:53Z owner
  question; overage alone is reported, not held (integrator 11:20Z). The next slice applies B6-1
  first, and the six faults reuse `kill_rec`, `fault`, `at` and `recover` instead of new helpers.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-11 slice 4, part 1 (merge a96d9f71)

- Ten of 16 connector files are on integration; new: `copilot-cli`, `gemini-cli`, `openclaw-cli`,
  `opencode-cli`, `opencode2-cli`, `qwen-cli`. For the login in the box: `GEMINI_CLI_HOME` stands
  for the home folder, so Gemini's files sit under `{home}/.gemini`; `QWEN_HOME` is the `.qwen`
  folder itself; `OPENCLAW_HOME` holds `.openclaw`. The two OpenCode connectors run the same
  `opencode` program and are told apart only by `version`, so the box installs the matching major.
- PRD-11's facts sheets are rewritten from the pad's rows in PRD-11's last slice, so each row's
  record and `row.json` are what that slice reads. No change to the row shape is asked.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-08 slice 3 (merge 7a995b6e)

- **`secrets` output as merged** (PRD-08 note): always JSON on stdout, no `--json` flag. Keys:
  `verifier`, `result`, `exit`, `known_values`, `inputs` (`[{path, as, read}]`), `known_hits`
  (`[{form, in, sha12, n}]`), `shape_hits` and `glued_shape` (`[{kind, in, sha12, n}]`), `masks`
  (`{kind: span count}`), `mask_bytes` (`{fail_closed: bytes}`) and `big_masks` (`[{kind, in,
  len}]`). Exit 2 on a fail, 1 when an input was read only in part, 0 otherwise; 3 with no report when
  a path given or the known file cannot be read. `check --known` adds a `secrets` check per entry,
  which `check.py` already reads for C3.
- **Changes for the pad's next touch** (both cuts or one-word changes):
  - `check.py` `SECRET_HITS` adds `"big_masks"`: a non-`fail_closed` MASK span over 16 KiB makes the
    scan `fail` by itself, and `hits()` would read it as 0. `"hits"` is not a key PRD-08 writes and
    can go. `glued_shape` is a `warn` and stays out. The "Not fixed yet" comment above it is now
    fixed; the `exits.json` fallback stays for exit 3.
  - `pad-run`'s verify step drops `raw` (`goalcup.log`, `upload.json`) from the `secrets` argv:
    `secrets <entry>` reads every file of the entry except `rec/` itself, so naming them again
    doubles their `n`. The copied `status.json` is another path and stays.
  - Section 8's command list reads `goalcup-verify secrets --known <known> <fetched entry>
    status.json` for the same reason.
- **C3 is no longer held by PRD-08.** `verify/secrets.json` now carries `masks`, so `masks()` returns
  a map. Since the local entry and the fetched copy go into one report, their span counts are summed;
  C3 tests only for a non-zero count per planted kind, so the sum changes nothing. C3 still needs
  PRD-05's masker writing MASK records and a `goalcup` build on integration before it can PASS.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-08 slice 4 (merge 1fdaf2c3): reading the three honesty checks

- `listed_hosts_decrypted`, `conn_bytes` and `mask_failclosed` now report (PRD-08 note). C4's "no
  `fail`" already covers a silent listed tunnel (`listed_hosts_decrypted` `fail`) and a silent loss
  (`conn_bytes` `fail`, exit 2). C1 also reads `listed_hosts_decrypted`: `pass` in `direct` and
  `gateway` rows, `named` only where the row expects it (dodger, fault rows). A `warn` is not a
  FAIL; `facts.refusal_after_trust` is listed in the row.
- **`conn_bytes` `skip` is not a FAIL but is not evidence.** In a `dodger` row it is `skip` "no
  decrypted connection to judge", as expected. In a `direct` or `gateway` row a `skip` (facts
  `no_plain_tail` above 0) means a decrypted connection ended with no `plain_up` or `plain_down`:
  the row stays as it is and the night log gets one line naming PRD-03.
- `mask_failclosed` is `warn` at most and keeps exit 0. The row prints `facts.streams` and
  `facts.bytes` beside PRD-05 A1's `mask_failclosed_bytes`; neither lowers a PASS.
- No locked ledger item is touched and no owner question comes from this slice.
