# E02: early unattended objective and sustained capture load

> **Historical (2026-10-02).** E02 ran on the E01 binary through the in-process sink; its Part B verdict was FAIL (§8.1). Current: [shipped-path Part B](m2-synthetic-vertical.md#full-part-b-result--one-synthetic-run-on-frozen-3c); index: [README](README.md).

> **What this is:** E02 on branch `m2/e02-unattended`. It holds the pre-registration: the
> literal commands, the candidate, the agent objective and its external checks (Part A), and
> the deterministic 30-minute recorder load with its byte schedule (Part B). The inputs (goal,
> criteria, checker, corpus, load, sampler, driver) froze at `edebac2`, and the plan text at
> `7689ea5`, both before any live row. Later edits to §1–§6 are status, the LIVE GO approval,
> the post-hoc §4.3 tolerance change (§8.1) and the fix-round-1 disclosures. It also records two plumbing runs
> (a driver dry run with E01's fake harness and a 180-second compressed load, both loopback
> only), **the two Part A rows** (§7) and **the 30-minute Part B load** (§8), all measured on
> 2026-09-28.
>
> **Candidate.** Every measurement here ran on the **E01 candidate binary**: runner code
> `94e540e`, sha256 `495adae7…ee0ab1`, used in place. **It is not the current tree.** Since
> E02 was based (`c85d0dd`), m2/wire has merged T03R, T04b, T06, T12, T14, T19, T20, T21,
> T18-F2 and CP2 (at `d40248a` when Part B ran). None of those changes is in the measured
> binary. Physical reservation, the IPC sender and receiver, connector capture profiles, the
> ported extra redaction patterns and the decoders were therefore not exercised. Rerun the
> measurements on a new candidate to cover them (T27).
>
> **What it is not:** two short Part A rows are not evidence of two-hour autonomy. Part B is
> recorder stress, not agent autonomy. It is not an installed or reserved pilot.
> - Storage is **experimental, unreserved quota-only accounting**: `physical_reservation=false`,
>   `reserved_bytes=0`. The quota is an admission ceiling, never a reservation.
> - The anchor is `local_only`. No receipt, ACK, devserver or upload exists.
> - Part A and Part B are separate measurements. Neither stands in for the other (packet E02).

## 1. Status

| Item | State | Evidence (git-ignored `spikes/m2-first-release/out/early/e02/`) |
|---|---|---|
| Part A, Codex CLI row | **ran** after the user's LIVE GO: natural exit in 97.8 s; objective **`partial`** (every catalogue check exact; 2 unit tests where the frozen minimum is 3); literal verify **0** (§7) | `codex-20260928T072736Z` |
| Part A, Claude Code row | **ran** after the Codex row: natural exit in 284.8 s; objective **`complete`**; literal verify **0** (§7) | `claude-20260928T072954Z` |
| Part B, 30-minute load | **ran** in the orchestrator's quiet window: load self-check ok; **frozen cross-check FAIL** (exit 1, 2 of 6 checks, both from one lost SSE response misattributed to +899 s; §8.1); **corrected cross-check ok** (match tolerance 1 s → 50 ms, chosen after the run); literal verify **1** as designed (the deliberate gap) | `load-20260928T101817Z` |
| Driver dry run (fake harness, seeded, loopback) | ran on `edebac2`, clean tree: runner exit **0**, literal `goalcup-verify` **0** | `dry-20260928T071711Z` |
| Part B plumbing smoke (`load-smoke`, 180 s, loopback) | ran on `edebac2`, clean tree: cross-check **ok**; runner and verify exit **1**, as designed (the deliberate gap) | `load-smoke-20260928T071734Z` |

## 2. Candidate, files and locations

- **Runner:** the reviewed E01 binary, used in place and never rebuilt, because its hash is the
  one the host's network filter allows (E01 §8):
  `<worktree>/target/release/examples/m2_capture_experiment`,
  sha256 `495adae7181497145e0a214358818ec96930a0b19b586d74499d642cf0ee0ab1`, built from code
  `94e540e0304847b340c17b2af1a5145dd7e5df2f` (passed as `--recorder-revision`).
  `goalcup-verify` beside it: sha256 `6df96af721edc85b18dd9e7698bb0a72d701ca5359be87d069a736267f31bd89`.
  The driver refuses (exit 4) any other runner or verifier. The runner interface is E00 §6 plus
  E01 §2, unchanged. **E02 changes no crate code.**
- **E02 files.** The lease (CONTRACTS Wave 2, R18) names `spikes/m2-first-release/early-run.sh`.
  With the dispatch's leave to pick a directory, every E02 script lives in
  **`spikes/m2-first-release/e02/`** instead:

  | File | Role |
  |---|---|
  | `early-run.sh` | the driver: one bounded run per invocation (`dry`, `codex`, `claude`, `load`, `load-smoke`) |
  | `early-corpus.py` | the deterministic seed workspace and the frozen task criteria; `selfcheck` |
  | `early-check.py` | Part A's external output check against the criteria |
  | `early-load.py` | Part B's workload: the byte schedule; `plan`, `selfcheck`, `run` |
  | `early-sample.py` | the external resource sampler (R19) and its summary |
  | `early-crosscheck.py` | Part B's independent check: sent = forwarded = retained outside the gap |
  | `early-analyze.py` | Part A row analysis per §3.6 (written after the Codex row, committed after both; the driver never runs it; it only reads evidence) |

  Also `docs/qualification/m2-early-goal.txt` (the goal) and
  `docs/qualification/m2-early-criteria.json` (the task criteria, generated by
  `early-corpus.py criteria --goal docs/qualification/m2-early-goal.txt`).
- **Toolchain** (R18): `/usr/bin/python3`, Python 3.9.6, sha256
  `b8763cf250e607a778bb4603cecb5b90338814d0a3dfcba0d57b1de242f610e9`, standard library only.
  The same interpreter runs the agent's task and every E02 script.
- **Where a run lives.** Each run gets a private root `/Users/Shared/goalcup-m2-e02/<run>` (0700,
  `umask 077`, outside every repository, not under `/tmp` or `$TMPDIR`; E01 §3.3). It holds the
  runner's `--out` (`out/`), the seed (`seed/`) and any pin home. The driver refuses to start
  unless `/Users/Shared/goalcup-m2-e02` is empty. When every check has run, it renames the run
  root to `<evidence>/run-root`. So a later run's workspace never has an earlier run, or an
  earlier solution, beside it; this holds only inside the run root, and a file an agent writes
  elsewhere, such as the Claude row's `/tmp/test.db` (§7.7), outlives the run. The driver leaves the empty
  shared parent in place; it was removed by hand after each session of runs. Driver evidence is in the git-ignored
  `spikes/m2-first-release/out/early/e02/<run>/` (`.gitignore:12 spikes/*/out/`): `identity.txt`
  (binary, script, harness and host identity; exits), `command.txt` (the literal argv),
  `samples.jsonl`, verifier, `facts`, `inspect`, side-effect and check outputs.

## 3. Part A: the agent objective

### 3.1 The objective (the same for both harnesses)

- **Goal:** `docs/qualification/m2-early-goal.txt`, 842 bytes, sha256
  `bc9de96c619a2f414934a8469553b1e09cd94dcdafd3a7a36e6f9fe1bfed153f`. It is delivered once,
  automatically, as the argv element after `--`, and is retained only as the Goal stream. In
  short, it asks the agent to work alone and to build the catalogue that `SPEC.md` specifies,
  with Python 3.9 stdlib only and no network. The deliverables are `catalog.py`
  (`ingest`/`query`/`export`), tests under `tests/` for malformed, duplicate and large inputs,
  and `LIMITATIONS.md`. The agent should run `check_public.py` itself, leave `./corpus`
  unchanged, ask nothing and wait for nothing, and finish with a summary.
- **Seed workspace** (`early-corpus.py seed`, deterministic: own splitmix64 PRNG, fixed mtimes):
  110 files, 8 610 195 bytes, tree sha256
  `11627784e90017b00e736e8c5b703773bc3ca7ba2b270c07a87d51020d41ef9c`. The driver refuses a seed
  with any other hash.
  - `SPEC.md` (3 846 B, spec id `e02-catalogue-spec`): the exact CLI, input formats, rejection
    reasons and their order, source links, the token rule (`[a-z0-9]+` after lower-casing),
    query semantics (AND over distinct tokens, score, order, `--limit`) and the export format.
  - `check_public.py` (13 608 B): the public acceptance checks. It checks ingest, 10 public
    queries, the export count and head, and the missing-database exit. It prints
    `e02-public-check` lines.
  - `corpus/` (108 files):
    - 90 Markdown articles, plus 5 more in `articles/archive/`, one of them with CRLF line ends;
    - four JSONL files with 120 records;
    - one large accepted document (`misc/big.md`, 3 151 599 B), which has its own vocabulary
      so that it decides only the query about it;
    - 15 planted rejections: `missing_field` 5, `duplicate_id` 3, `empty` 2,
      `malformed_json` 2, and one each of `too_large` (`misc/huge.md`, 5 250 915 B),
      `invalid_utf8` and `unsupported_type`.

    216 documents are accepted.
- **Held-out checks** are fixed in `m2-early-criteria.json` (sha256 `5d29fbce…9df464b0`). They
  are never in the seed, and run only after the harness has stopped:
  - 20 held-out queries: six single terms; four pairs; one triple; tie order by id; hyphen
    splitting; a digits token; the large document's last line; a term only in the too-large
    file (→ `[]`); a term only in a rejected duplicate record (→ `[]`); case and punctuation
    (`Lantern,`); and `--limit 3` and the default limit of 10 on common terms.
  - the full export: 216 rows, canonical sha256 `9f342d7e…87e88828`.
- **Why the answers can be trusted:** they are computed twice and must agree
  (`early-corpus.py selfcheck`). One computation is the generator's own document model: tokens
  are recorded as they are emitted, and acceptance is decided on structured data. The other is
  a reference reader that parses the written bytes by `SPEC.md`.

### 3.2 Seeding without a runner change (R18)

The runner creates `{out}/workspace` empty. The workload is therefore

```text
/bin/sh -c 'cp -Rp "$E02_SEED"/. ./ && exec "$0" "$@"' <harness> <recipe args> -- {goal}
```

with `--workload-env E02_SEED=<run root>/seed`.
- `exec` keeps the PID, the session leadership and stdout, so Codex's late-bound scope still
  works: in the dry run it was bound in 254 ms.
- The runner's `--pin-sha256`/`--pin-version-output` would now pin `/bin/sh`, so they are not
  passed. **The driver checks the harness pins itself** before anything starts:
  - Codex: sha256 `a29d9e86…d7eaa22`, and `--version` = `codex-cli 0.153.0` (scoped
    environment, an empty pin home, no proxy);
  - Claude: sha256 `d8cb1e5c…5651d21e`, with no version run, as in E01.

  The pins go to `identity.txt`. `summary.json` `launch.program` is `/bin/sh`.
- The seed's hash is taken again after the run (`seed_unchanged`). The check also records
  whether the workspace's `corpus/` still equals the seed's.

### 3.3 Predeclared settings

| | Codex CLI 0.153.0 | Claude Code 2.1.283 |
|---|---|---|
| Model / effort (D38) | `-m gpt-5.6-luna`, `model_reasoning_effort="low"` | `--model haiku --effort low` |
| Launch | `codex exec --json`, one turn; `approval_policy="never"`; `-s workspace-write` (tool commands have no network); `--skip-git-repo-check`; process-scoped `projects` trust override | `claude -p`, one turn; `--permission-mode acceptEdits --permission-prompts none`; **`--allowedTools Bash`**; `--output-format json`; MCP off (`--strict-mcp-config`, empty config, two env switches) |
| Continuation / subagents | none: `exec` ends with its turn; no subagent configured | none: `-p` ends with its turn. Claude's built-in subagent tool is not disabled; its transcripts are a native source (`required=false`) |
| HOME | fresh empty per-run dir | fresh per-run dir holding only `Library/Keychains` → the user's (D38) |
| Harness home | shared isolated `CODEX_HOME=$GOALCUP_HARNESSES/home/codex` (D39/R10) | shared isolated `CLAUDE_CONFIG_DIR=$GOALCUP_HARNESSES/home/claude` |
| Captured / auth host | `chatgpt.com:/backend-api/codex/responses` / `auth.openai.com` | `api.anthropic.com:/v1/messages` / `platform.claude.com` |
| Native sources | rollout, late-bound from `thread.started:thread_id` | transcript (required), subagents and tool results (not required), scoped by the pre-assigned session ID |
| CA variable | `CODEX_CA_CERTIFICATE` | `NODE_EXTRA_CA_CERTS` |

Common: `--quota-bytes 1073741824`, `--require-isolated-workspace`, `--dispatch-deadline-secs 120`,
`--workload-timeout-secs 7200`, and an environment built from nothing (E01 §4.2 plus `E02_SEED`).
No base URL and no provider variable are set.

**One deliberate change from the E01 recipe.** Claude gains `--allowedTools Bash`. Without it,
acceptEdits denies every shell command, so Claude could write `catalog.py` but could never run it,
`check_public.py` or its tests; Codex can. T02 measured this setting with the keychain-link
HOME (`cl-r1-bash-homelink-125352`): 0 attributable app-wide writes, and no user shell rc in the
snapshot. The costs are these:
- Claude's Bash is not OS-sandboxed, unlike Codex's `workspace-write`.
- Its tool processes inherit `HTTPS_PROXY`, so they could tunnel out through the proxy. The
  proxy counts that in `tunneled_not_approved`.

**Approved by the user at LIVE GO (2026-09-28)** for E02. It is a recipe change from E01 and
T02 §8. The side effects it measured in the Claude row are in §7.7.

### 3.4 Stop criteria and time limits

- **Natural end:** the harness exits after its one turn.
- **Horizon:** 7 200 s (`--workload-timeout-secs`). Then SIGTERM to the group and every
  tracked descendant, 10 s, SIGKILL, 5 s (E01 §2). The horizon is an observation bound, not a
  promise of two hours of work. A five-minute completion is recorded as five minutes.
- **Dispatch deadline:** the workload is stopped if no approved-host connection has been
  intercepted 120 s after spawn. For Codex, a `thread.started` not seen within 120 s is a
  `SourceChanged` gap.
- **Never during a measured row:** later input, approvals, keep-going, login repair,
  continuation, workspace edits or rescue.
- **Every row counts.** Natural completion, failure and waiting are all outcomes. Every row,
  negative ones included, is kept. A rerun after an environmental negative is the
  orchestrator's decision, recorded (the E01 precedent).
- **Order:** Codex, then Claude, never concurrent, on the same candidate and conditions.
  Worst case per row is 7 200 s plus about 10 s of stop grace, plus the post-run checks. The
  agent's own tests are bounded at 900 s, ingest at 600 s, each query at 60 s and the export
  at 300 s.

### 3.5 External check and outcome classes (frozen in the criteria)

`early-check.py` runs after the trace is sealed.
- It copies the workspace (symlinks as links), runs the agent's `catalog.py` against the
  **pristine** seed corpus with a fresh database, and runs `python3 -m unittest discover -s
  tests`.
- Every command runs with `PATH=/usr/bin:/bin` and `HOME`/`TMPDIR` under the check directory, and
  its process group is killed at the timeout.
- It scores ingest (exact accepted count and rejection set), the 10 public and 20 held-out queries
  (exact JSON), the export (canonical sha256), the missing-database exit 2, the tests (exit 0,
  at least 3 run) and `LIMITATIONS.md` (at least 200 B).

| Outcome | Rule |
|---|---|
| `complete` | ingest exact; public 10/10; held-out ≥ 18/20; export exact; tests exit 0 with ≥ 3 tests; `LIMITATIONS.md` ≥ 200 B |
| `partial` | `catalog.py` exists and ingest exits 0, but not complete |
| `none` | no `catalog.py`, or ingest does not exit 0 |

The outcome is reported separately from the recorder's verdict. An honestly captured agent
failure does not fail the recorder.

### 3.6 What each row records (research observations, E02 packet A)

- Model(s) observed (wire and native).
- Tool calls by name, from native, cross-checked on the wire.
- Errors: tool errors, API errors and retries, `permission_denials`.
- Context events: any context-compaction record in either native transcript.
- Subagent use.
- Token counts reported by the harness.
- Stop reason and wall duration.
- Pre-registered inspectability (E01 §5, with `--call-token catalog.py --output-token
  e02-catalogue-spec`).
- Retained vs source bytes and audit kinds per stream; any replacement that runs to the end of
  a body is a known limitation, not a pass. Codex `encrypted_content` of 16 KiB or more is
  expected in long runs (preflight-wave2).
- Integrity, gaps, side effects (E01 §4.4 roots plus `~/Library/Keychains` metadata; never
  contents) and the sampler series.

**Active, idle and unknown intervals** are defined now, to be computed after the run. The run
is cut into 60 s windows from spawn to the workload's end.
- *Active:* the window holds at least one captured record of any source.
- *Idle:* no record, and the harness tree's sampled CPU is under 1 %.
- *Unknown:* the window has no sample, or overlaps a gap.

### 3.7 Before LIVE GO (outside the measured rows)

1. Maintainer confirms the Claude `--allowedTools Bash` setting (§3.3).
2. Network filter: the runner binary above is already allowed, and E01 reached `chatgpt.com`,
   `api.anthropic.com` and `platform.claude.com` through it. The first attempt to each new
   destination failed in E01 (§7.3 there). Confirm the rules, including `auth.openai.com`,
   before the measured rows.
3. Isolated auth readiness, in the recipe environment with no proxy. Run these once. No login
   or repair is attempted.
   ```sh
   . <repo>/harnesses/env.sh
   H=$(mktemp -d); env -i HOME="$H" PATH=/usr/bin:/bin:/usr/sbin:/sbin CODEX_HOME="$GOALCUP_HARNESSES/home/codex" \
     "$GOALCUP_HARNESSES/node/lib/node_modules/@openai/codex/node_modules/@openai/codex-darwin-arm64/vendor/aarch64-apple-darwin/bin/codex" login status </dev/null
   H2=$(mktemp -d); mkdir "$H2/Library"; ln -s "$HOME/Library/Keychains" "$H2/Library/Keychains"
   env -i HOME="$H2" USER="$USER" LOGNAME="$LOGNAME" SHELL=/bin/zsh TMPDIR="$TMPDIR" PATH=/usr/bin:/bin:/usr/sbin:/sbin \
     CLAUDE_CONFIG_DIR="$GOALCUP_HARNESSES/home/claude" DISABLE_AUTOUPDATER=1 \
     "$HOME/.local/share/claude/versions/2.1.283" auth status </dev/null
   ```
4. `/Users/Shared/goalcup-m2-e02` is empty. The driver checks this.

### 3.8 Commands

```sh
cd <worktree>
. <repo>/harnesses/env.sh
spikes/m2-first-release/e02/early-run.sh codex     # row 1; wait for it to finish
spikes/m2-first-release/e02/early-run.sh claude    # row 2
```

## 4. Part B: the deterministic recorder load

### 4.1 Schedule (`early-load.py plan --schedule full`, sha256 of the printed plan `c6848013…57adcf34`)

The schedule has six 300 s phases, dense and sparse alternating (D S D S D S), 1 800 s in
all. Payload bytes are counted in both directions on captured paths.

| Part | Per unit | Units | Bytes |
|---|---|---|---|
| Dense, HTTP/SSE half | a 16 KiB echo (16 KiB up + 16 KiB down); an SSE response of 8 events × 4 096 B (`events=8&size=4072&interval_ms=100`) | 900 ticks (1 s) | 900 × 65 536 |
| Dense, WS half | two 16 KiB client text messages (4 KiB frames, a ping after the first frame), echoed in 4 KiB frames (`frag=4096`) | 900 ticks | 900 × 65 536 |
| Sparse | sixteen interleaved SSE streams, offsets 0.625 s apart, each receiving one 256-byte event every 10 s (3 requests × 10 events per phase) | 3 phases × 16 × 30 events | 368 640 |
| Native | four 1 024-byte JSONL lines per second, appended to the native source | 1 800 s | 7 372 800 |
| **Total logical payload** | | | **125 706 240 B = 119.8828125 MiB** (dense 117 964 800 = 112.5 MiB) |

The following are declared and carry no scheduled payload:
- an auth-host request at the start (tunnelled);
- the stall trigger (§4.2).

Protocol, JSON, base64 and audit overhead are extra and are measured.

**Renewal and overlap.** Connections are renewed every 10 echo requests and every 15 SSE
requests. WS sessions are renewed every 30 ticks. Every lane connection is also renewed at the
end of its phase. All lanes run concurrently, so HTTP streams overlap.

**Secret sentinel.** A synthetic `sk-ant-…` value is built at run time and never written to
source or to the report. Sizes never change. It appears:
- in the echo body of dense ticks 30, 90, … (15 times), split across two request chunks 50 ms
  apart;
- in the first WS message of the same ticks (15 times), across the 4 096-byte frame boundary,
  in both directions;
- in one native line per minute (30 times), written in two halves 300 ms apart.

### 4.2 The deliberate stall (minute 12)

At 720 s, inside the second dense phase, the load sends one empty `POST /v1/bulk?size=0` on its
own connection. The runner's `--stall-on-open-path /v1/bulk --stall-max-ms 60000` stops the
owner draining at that Open. It holds until the sink refuses a producer with `QueueFull`, then
drains again. At 128 KiB/s the dense traffic fills the 8 MiB encoded queue by itself. The smoke
held **15 522 ms**. Required:
- the release is `queue_full`, not `timeout`;
- a Recovery naming the lost producer;
- permanent gap records;
- clean capture after the Recovery;
- connection IDs re-observed under the fresh producer;
- the load's own byte checks prove that forwarding went on throughout.

### 4.3 What the cross-check compares (`early-crosscheck.py OUT EV`)

1. **Sent = forwarded, exactly.** The load's report is compared with the endpoint's
   independent counters for echo, SSE, WS, bulk and auth, in requests, events, messages and
   bytes.
2. **Sent = retained outside the injected gap.**
   - Every stream in the chain that ended `complete` with no gap must equal a sent body or WS
     message, by sha256.
   - A sentinel item needs instead: the sent source length, an `anthropic_key` audit span, and
     no sentinel bytes retained.
   - The matched stream's Open must lie within the item's send interval, up to a tolerance.
     Every dense SSE body is identical (the endpoint's events are deterministic), so time
     decides which one is missing. The frozen tolerance was 1 s. It was changed **post hoc** to
     50 ms after the 30-minute run, whose frozen verdict was FAIL (§8.1).
   - Unmatched sent items and unclean streams must lie in the gap window, [trigger − 2 s, last
     Recovery + 2 s]. Anything else is unexplained and fails the check.
3. **Native:** line by line against the source file; sentinel lines must be redacted and every
   other line equal.
4. **No leak:** no sentinel byte and no proxy-credential shape in any decoded segment, or in any
   other file of the run or its evidence.
5. **Reported, not pass/fail:** the runner's measured peaks:
   - `max_flush_interval_ms` and `max_sync_interval_ms`. These are owner **cadence** maxima
     (flush-to-flush and sync-to-sync), not the safe-byte latency the 250 ms target is about.
     Fix round 1 added a post-hoc latency measurement from chain timestamps (§7.6, §8.3);
   - sink high-water;
   - recorder peak RSS;
   - chain size against the 512 MiB quota.

   Their prospective targets are flush ≤ 250 ms, sync ≤ 1 s, queue ≤ 8 MiB and recorder RSS
   ≤ 256 MiB. They are targets, and nothing here asserts them as results.

The literal `goalcup-verify --capture-integrity` exit is expected to be **1**: `incomplete`,
from the deliberate gap.

### 4.4 Runner arguments (abridged; each run's `command.txt` holds the literal argv)

```text
--quota-bytes 536870912 --profile-id e02-controlled-load --harness-version 'e02-early-load-1 full'
--capture api.e00.localhost:/v1/{echo,sse,ws}:exact:e00-{echo,sse,ws}   (three rules)
--capture api.e00.localhost:/v1/bulk:exact --auth-host auth.e00.localhost
--upstream-roots endpoint --controlled-endpoint
--native-source 'name=e02,root={native_root},pattern=e02-native.jsonl,form=append'
--ca-env E02_PROXY_CA --stall-on-open-path /v1/bulk --stall-max-ms 60000
--workload-timeout-secs 2100 --workload-env PATH=/usr/bin:/bin --workload-env-inherit TMPDIR
-- /usr/bin/python3 spikes/m2-first-release/e02/early-load.py run --schedule full
   --endpoint {endpoint} --auth-endpoint {auth_endpoint} --upstream-ca {upstream_ca}
   --native-path {native_root}/e02-native.jsonl --test-id {test_id} --report {out}/load-report.json
```

### 4.5 The sampler (R19)

`early-sample.py watch` runs beside the runner, read-only, every 1 s for Part B and every 2 s
for Part A. Each sample records:
- the runner's RSS and CPU (from `ps` cumulative CPU time);
- the workload tree's process count, RSS and CPU;
- the runner's own `health.json`: sink queued payload bytes and items, high-water, QueueFull
  refusals, durable storage, proxy counters;
- the chain size and the load average.

`summarize` gives peaks, the least-squares growth per hour of RSS and chain size, and per-phase
means and maxima. For Part B, times are measured from the load's own start. Full timing
distributions are deferred as an E00-owned change (R19). The flush and sync figures are the
runner's maxima only.

### 4.6 Command, duration and host

```sh
cd <worktree>
spikes/m2-first-release/e02/early-run.sh load
```

- **Duration:** the load takes 1 800 s. The last sparse responses close about 9.4 s later: the
  smoke took 189.4 s for its 180 s plan. Setup and the post-run checks add a few minutes.
  Budget **35 minutes**.
- **Quiet host:** no cargo gates, no builds, no `hdiutil`/disk-image probes (the T03R volume
  step, P5) and no live Part A row may run in the window. `load` mode refuses (exit 4) when a
  `cargo`, `rustc`, `hdiutil`, `cc` or `ld` process is running. The smoke mode does not check.
- **Expected size (a projection, not a result):** the smoke's chain was 1.51× its logical
  payload (18 932 020 B for 12 570 624 B). At that ratio the full run writes about 190 MB, 35 %
  of the 512 MiB quota. If real expansion reaches the quota, the failure is kept and the budget
  is revised explicitly (packet).

## 5. Verification done while preparing (not measurements)

**Self-checks.**
- `early-corpus.py selfcheck docs/qualification/m2-early-criteria.json`: **ok**. The seed is
  deterministic across two generations. The model and the reference reader agree on all 216
  documents, the 15 rejections and all 30 queries. The frozen criteria equal the generator's.
- `early-load.py selfcheck`: **ok**. It checks:
  - the plan arithmetic (the full total 125 706 240 B, 256-byte sparse events, a 4 096-byte
    dense SSE event built independently of the formula);
  - that the filler has no run longer than 9 characters and no `@`;
  - the sentinel's shape and its placement inside the chunk, frame and native splits;
  - that 1 024-byte native lines stay exact;
  - a round trip of masked, fragmented WS messages with an interleaved ping, and unmasked
    frames of all three length forms;
  - chunked decoding;
  - that the proxy credential stays out of `repr`.

**Mutation checks.** Evidence is in `target/e02-evidence/` of this worktree; nothing there is
committed.

| Target | Mutants | Result |
|---|---|---|
| Corpus self-check | reader keeps the last duplicate; no lower-casing; processing by line before path; titles not stripped (CRLF); `--limit` ignored | **5/5 killed** |
| Output check | reference solution (not committed) | `complete`: public 10/10, held-out 20/20, export exact |
| | ties ordered by id descending | public 1/10, held-out 4/20 → `partial` |
| | CRLF title not stripped | public 10/10 but held-out 19/20 and export wrong → `partial` (the held-out set sees what the public set does not) |
| | duplicate keeps the last | ingest wrong → `partial` |
| Part B cross-check (copies of a smoke run) | one WS stream's segments dropped outside the window; an endpoint counter off by one; a sentinel written into `health.json`; one native byte changed | **4/4 detected** |

**Driver dry run `dry-20260928T071711Z`.** Candidate `edebac2`, clean tree. It ran E01's
`fake_harness.sh` (sha256 `f3c161c6…fb125c`, `fake-harness 0.153.0-e01`) through the controlled
endpoint, seeded by the §3.2 wrapper with the frozen goal and seed.

| Measured | Value |
|---|---|
| Runner / literal verify | exit **0** / **0**: `valid`, sealed, 20 records, 0 gaps, `local_only` |
| Seeding | workspace = seed (`SPEC.md`, `check_public.py`, `corpus/`) plus the fake's `result.txt`; seed unchanged after the run; workspace corpus unmodified |
| Late scope through `exec` | bound 254 ms after spawn; first approved-host interception 254 ms |
| Workload | exit 0 in 163 ms. Its TERM-ignoring helper was tracked, then killed after the 10 s grace: **`Confirmed`**, requested 07:17:14.374Z, confirmed 07:17:24.406Z |
| `inspect` (E01 tokens for the fake) | tool call and output **wire+native**; retained = source for goal (842 B), native (482 B), request and response (241 B each) |
| Output check | `none` (no `catalog.py`, as expected of the fake) |
| Side effects | `config.toml` hash unchanged. All 4 created paths and 1 of 9 modified paths are attributable, all in the fake home. The other 8 are this measuring session's Claude Code transcripts and the Claude desktop app's cache: host churn (E01 §7.7) |
| Resources | runner peak RSS 18 022 400 B; owner flush/sync cadence maxima 234 / 834 ms |

**Part B smoke `load-smoke-20260928T071734Z`.** Candidate `edebac2`, clean tree, schedule
`smoke`: 30 s phases and the stall at 72 s. This is plumbing on a loaded host: 9 `cargo`,
`rustc` or `hdiutil` processes at the start, and a 1-minute load average up to 7.57. **It is
not a measurement.**

| Measured | Value |
|---|---|
| Load self-check | **ok**: every exchange byte-checked; 12 570 624 B, equal to the plan; 0 errors; tick lateness at most 12 ms |
| Sent = forwarded | **equal** in all 12 endpoint counters |
| Sent = retained outside the gap | **equal**. Matched: 228 + 1 sentinel HTTP requests, 227 + 1 responses, 179 + 1 WS client messages, 179 + 1 server messages. Missing, in the window: 1 SSE response in flight at the refusal. Unexplained: 0 |
| Stall and recovery | held 15 522 ms, released by `queue_full`. 1 QueueFull refusal, 1 Recovery, 1 `queue_full` stream gap. 3 connection IDs re-observed under the fresh producer, all 3 first opened under the lost one. 304 items sent after the window, none missing |
| Native | 1 stream, 737 280 source B; of the file's 720 lines, 717 equal and 3 sentinel lines redacted, 0 differ |
| Leak scan | 0 sentinel and 0 credential hits in the chain; 0 in 28 other files |
| Runner (reported) | flush cadence 238 ms, sync cadence 834 ms; sink high-water 8 247 696 B payload + 512 B control; peak RSS 21 659 648 B; chain 18 932 020 B of 536 870 912 (reserved 0) |
| Integrity | runner and literal verify exit **1**: `incomplete`, sealed, 3 022 records, 1 gap, `producer_recovered`; verify took 0.07 s and 4 816 896 B RSS |
| Sampler | 189 samples, largest gap 1.086 s; queued payload peaked at 7 745 174 B in the stall phase, and at most 125 446 B in every other phase |

An earlier pre-commit smoke (`load-smoke-20260928T071015Z`, kept) exposed a cross-check bug:
matching by digest alone mis-attributed the one lost SSE response to the run's last tick,
because every dense SSE body is identical. Matching became time-aware (§4.3.2) before the
commit.

## 6. Limits and open items

- **Held-out answers are in the repository**, outside the workspace, and the agent is not told
  of them. Codex's sandbox can read the whole disk, and Claude's Bash is unsandboxed. Deliberate
  roaming is not prevented; the per-run root prevents only the accidental sibling discovery.
- **`E02_SEED` is visible to the harness** and its tools. It names the pristine seed, which
  holds nothing the workspace does not.
- **Claude's `--allowedTools Bash`** (§3.3) is the one recipe change from E01. The user approved
  it at LIVE GO; §7.7 lists what it measurably touched.
- **Harness pins are the driver's, not the runner's.** They are in `identity.txt`, not in
  `summary.json` `launch` (which names `/bin/sh`) or the Launch record.
- **Sampler tree:** descendants by parent chain at each sample. A process re-parented to launchd
  (as in the fake's own-session helper) is not in the tree series; the runner's tracker lists
  it. CPU % comes from cumulative CPU time, and a process that exited between samples takes
  its time with it (clamped at 0).
- **Part B is recorder stress, not agent autonomy.** It has a Python 3.9 / LibreSSL 2.8.3
  client, not a harness TLS stack. It cannot prove refresh or auth boundaries, which stay
  untested.
- **The output check runs agent-written code on this host**: after the run, bounded, with an
  environment built from nothing and no credentials.
- **n = 1 row per harness** is planned. Timing distributions beyond the runner's maxima are
  deferred (R19); §7.6 and §8.3 add a post-hoc latency distribution from chain timestamps.
- **The side-effect listing roots exclude `/tmp` and `$TMPDIR`.** A host write there is not
  seen, and one happened (§7.7, `/tmp/test.db`). For T24/T27, add `/tmp` and `$TMPDIR` as
  listing roots (or a `find -newer` sweep), or give each run its own `TMPDIR`.

## 7. Part A results (2026-09-28)

Measured on this host in the runs named. Numbers come from each run's `summary.json`, the
literal verifier, `inspect.json`, `check/check.json`, the side-effect diff, the sampler, and
`analysis.json`. `analysis.json` is written by `early-analyze.py`, which counts retained bytes
per the §3.6 definitions. Evidence is in the git-ignored
`spikes/m2-first-release/out/early/e02/<run>/`.

### 7.1 Procedure

- **Go-ahead.** The user gave LIVE GO through the orchestrator on 2026-09-28, and approved
  Claude's `--allowedTools Bash`.
- **Runner.** The same runner, sha256 `495adae7…ee0ab1`, never rebuilt.
- **Scripts.** The driver and scripts were at `7689ea5`, with the same sha256 as `edebac2`.
  - The Claude row's `e02_status_lines=1` is the then-untracked `early-analyze.py`. The driver
    does not run it; it was committed afterwards.
- **Isolated auth readiness** (`authcheck-20260928T072726Z`) ran outside the rows, in the
  recipe environment with no proxy:
  - Codex `login status`: exit 0, "Logged in using ChatGPT".
  - Claude `auth status`: exit 0, `loggedIn: true`, `claude.ai`, `max`.
  - No login or repair was attempted. The Codex auth home was empty afterwards, and the
    Claude one held only its keychain link.
- **Rows, serialized, with no input, approval or rescue:**
  - Codex: 07:27:38–07:29:24Z;
  - then Claude: 07:29:55–07:34:44Z (driver start to runner finish).
- **Network.** Neither row failed for a network reason: 0 admission failures in both.
- **Host.** Other lanes were building. At the start, 8 and 7 `cargo`/`rustc`/`hdiutil`
  processes were running, and the 1-minute load average was 7.2 to 12. Every resource figure
  below comes from a loaded host.
- **Relocation** into `<run>/run-root` was a rename on one APFS volume (both paths on device
  16777234). It was verified afterwards:
  - the literal `goalcup-verify` re-run on the relocated trace printed output byte-identical
    to the pre-move run, exit 0;
  - the chain's last record equals the durable head in `summary.json` (Codex seq 11986
    `5d6a63fc…`, Claude seq 1284 `7c03dd70…`);
  - a sha256 manifest is in `<run>/run-root.sha256` (234 and 319 files).

  `/Users/Shared/goalcup-m2-e02` was then removed by hand: the driver renames the per-run
  root but leaves the empty shared parent.
- **Native originals.** Copies of both harnesses' session files are in `<run>/native/`, each
  copy's sha256 equal to its source.

### 7.2 Objective outcome (external check, frozen criteria)

| | Codex CLI 0.153.0 (`codex-20260928T072736Z`) | Claude Code 2.1.283 (`claude-20260928T072954Z`) |
|---|---|---|
| **Outcome** | **`partial`** | **`complete`** |
| Ingest (216 accepted, 15 rejections) | exact | exact |
| Public queries | 10/10 | 10/10 |
| Held-out queries | **20/20** | **20/20** |
| Export (216 rows, canonical sha256) | exact | exact |
| Missing database → exit 2 | yes | yes |
| Agent's tests | exit 0, **2 tests**. The frozen minimum is 3, which alone makes the row `partial`. The goal never stated that minimum (or the runner); it lives only in the criteria. The two tests cover all three named input types: `test_malformed_duplicate_and_large` asserts `malformed_json`, `duplicate_id`, `empty` and `too_large`, and `test_query_and_export` covers query and export | exit 0, 32 tests |
| `LIMITATIONS.md` | 634 B | 4 429 B |
| Workspace corpus / seed | unmodified / unchanged | unmodified / unchanged |
| Agent's final claim | "Unit tests: 2/2", "Public checks: 13/13", "Nothing remains undone" | a summary of the three commands, the tests and the limitations |

### 7.3 Behaviour

| | Codex | Claude |
|---|---|---|
| Model / effort | configured `gpt-5.6-luna` low. Native `turn_context` `gpt-5.6-luna` / `low`; on the wire, `"model":"gpt-5.6-luna"` 8 times in the 8 client messages, and no other model | configured `haiku` low. `claude-haiku-4-5-20251001` in all 95 native assistant records, and 42 times in the 42 wire requests (no other model). Effort is not observable natively (T02) |
| Wall time / stop | **97.8 s**, natural exit 0 | **284.8 s**, natural exit 0; stdout `success`, `end_turn`, 44 turns, API time 259.2 s |
| Tool calls (native) | 6 × `exec` (code mode), 6 outputs; 7 reasoning items, 7 messages | 43: `Bash` 23, `Edit` 9, `Read` 8, `Write` 3. 43 results, **4 with `is_error`**. `permission_denials` 0 |
| Errors, retries, compaction | no native record kind naming error, retry or compaction | none named so; the 4 tool errors above |
| Subagents | none | none (no subagent or tool-result files) |
| Tokens (harness-reported) | input 135 446 (114 944 cached), output 4 245 (286 reasoning) | input 338, cache write 73 468, cache read 2 651 346, output 26 524 (4 858 thinking). The harness reports a notional `total_cost_usd` of 0.545 on a subscription |
| Descendants tracked | 41: `Python` 29, `codex` 4, `zsh` 2, `sandbox-exec` 2, `python3` 2, `codex-code-mode` 1, `xcodebuild` 1. All gone → `AlreadyStopped` | 155: `Python` 118, `zsh` 14, `security` 4, `python3.9` 4, `head` 4, `tail` 3, others 8 (including the seeding `sh` and `cp`). All gone → `AlreadyStopped` |
| Active / idle / unknown (60 s windows) | 2 / 0 / 0 | 5 / 0 / 0 |

A five-minute completion is not two-hour autonomy evidence (packet). Both agents finished
their one turn well inside the horizon and stopped on their own.

### 7.4 Capture and integrity

| | Codex | Claude |
|---|---|---|
| Literal `goalcup-verify --capture-integrity` | exit **0**: `valid`, `local_only`, sealed, 11 987 records, 11 488 632 B, 0 gaps, no torn tail | exit **0**: `valid`, `local_only`, sealed, 1 285 records, 3 703 770 B, 0 gaps, no torn tail |
| Goal dispatch / native scope | first interception 759 ms after spawn; late scope bound from `thread.started` in 1 769 ms, through the `exec` wrapper | first interception 1 027 ms; pre-assigned session |
| Proxy | 20 accepted, 19 intercepted, 1 not-approved tunnel, 0 auth tunnels, **0 admission failures**, 0 × `407`. 19 closed interrupted (routine, E01 §7.2). Clean drain, no capture loss, 0 transport failures | 31 accepted, 18 intercepted, **13 not-approved tunnels**, 0 auth tunnels, 0 admission failures. 14 closed complete, 4 interrupted. Clean drain, no capture loss, 0 transport failures |
| Stream-less losses | none | none |
| Storage | 11 488 632 B of 1 GiB, **reserved 0** | 3 703 770 B, reserved 0 |

The proxy does not name tunnel hosts (E01 §9). So Claude's 13 not-approved tunnels, against 1
in E01's Claude pass, cannot be attributed: harness telemetry or the agent's own Bash could
each explain them. No approved or auth host was involved.

### 7.5 Retained vs source, and inspectability (E01 §5 method)

Tokens: `--call-token catalog.py --output-token e02-catalogue-spec`.

| Row | Source | Streams | Source B | Retained B | Audit |
|---|---|---|---|---|---|
| Codex | goal | 1 | 842 | 842 | — |
| Codex | native | 1 | 207 471 | 207 471 | —. **Byte-identical** to the rollout file |
| Codex | wire request/response (the WS upgrade) | 1 / 1 | 0 / 0 | 0 / 0 | — |
| Codex | `ws_client` | 8 | 97 898 | 97 898 | — |
| Codex | `ws_server` | 3 960 | 1 668 794 | 1 668 769 | `huggingface_token` × 1, 53 source B, not to the end |
| Claude | goal | 1 | 842 | 842 | — |
| Claude | native | 1 | 839 449 | 839 439 | `email` × 4 (74 source B). Differs from the file only on the 2 lines holding the account e-mail (E01 §7.2) |
| Claude | `wire_request` | 42 | 233 730 | 233 725 | `email` × 2 (37 source B) in one request |
| Claude | `wire_response` | 42 | 981 809 | 981 809 | — |

- **Tool-field inspectability:** tool call **wire+native** and tool output **wire+native** in
  both rows. No stream was replaced to its end.
- **Known limitation, not a pass:** the Codex `huggingface_token` span is a false positive
  inside a reasoning item's `encrypted_content`. It is a base64-like blob in a
  `response.output_item.added` server message (WS message 3 884, 1 662 B). This is the
  E00-I1 class: a replacement started inside a non-secret encoded value. It removed 53 bytes of
  an opaque blob and no inspectable field. No replacement ran to the end of a body, so the
  fail-closed path for long encoded values (E00 §7) did not occur in this row.

### 7.6 Resources (runner maxima and external samples; the targets are prospective)

| | Codex | Claude |
|---|---|---|
| Runner peak RSS (`getrusage`) | 19 529 728 B | 19 742 720 B |
| Owner flush cadence / sync cadence: worst flush-to-flush and sync-to-sync interval (a proxy, not the latency target) | 220 ms / 831 ms | **267 ms** / 834 ms |
| Sink high-water (encoded payload) | 225 235 B | 261 397 B |
| Sampled queue, max queued payload | 1 802 B | 783 B |
| Sampled runner CPU, max | 5.5 % | 3.0 % |
| Sampled harness-tree RSS, max | 185 073 664 B | 338 264 064 B |
| Largest reaped descendant peak RSS | 206 356 480 B | 338 870 272 B |
| Sampled harness-tree CPU, max | 47.9 % | 18.4 % |
| Verifier | 0.08 s, 5 144 576 B RSS | 0.21 s, 5 652 480 B RSS |
| Samples | 49 at 2 s, largest gap 2.013 s | 143 at 2 s, largest gap 2.018 s |

**The 267 ms is owner flush cadence, not the target.** `max_flush_interval_ms` is the longest
time between two starts of the owner's `flush_all`, whether or not any byte was waiting. The
250 ms target (spec §9, T15 §A.2) is about **safe-byte latency**: how long after capture a
byte is appended to the chain. The cadence went over 250 ms once in the Claude row, on a host
with a 1-minute load average up to 11.5.

**Post hoc (fix round 1, not pre-registered): safe-byte latency from chain timestamps.**
- **Method.** For every Segment record except the goal:
  - oldest-byte age = the envelope's `recorded_at` (the owner's clock when it appended the
    record) − `segment.first.wall` (capture of its first byte);
  - last-byte age = `recorded_at` − `segment.last.wall`.

  The minimum age, 0.04 ms, shows the two clocks are consistent. The script and its outputs are
  in the ignored `spikes/m2-first-release/out/early/e02/post-hoc/` (`latency.py`,
  `latency-<run>.json`).

| Row | Segments | Oldest byte p50 / p99 / p99.9 / max | Last byte p50 / p99 / max | Over 250 ms |
|---|---|---|---|---|
| Codex | 3 997 | 0.2 / 132.5 / 168.6 / **199.8 ms** | 0.2 / 132.5 / 199.8 ms | 0 |
| Claude | 1 069 | 183.4 / 207.0 / 221.7 / **226.8 ms** | 14.6 / 188.0 / 210.2 ms | 0 |

Claude has one more segment, a native one with an e-mail audit span; its age was 196.6 ms.

**The safe-byte latency target was met in both rows.** No byte waited more than 250 ms, so the
cadence overshoot delayed no byte past the target. Every other
figure is within its target: sync cadence ≤ 1 s, queue ≤ 8 MiB, recorder RSS ≤ 256 MiB. This
is one sample per row on a loaded host.

### 7.7 Side effects (metadata only, §3.6 roots)

| | Codex | Claude (recipe with `--allowedTools Bash`) |
|---|---|---|
| `harnesses/home/codex/config.toml` sha256 | unchanged, `cc20ba96…` | unchanged |
| Attributable to the run | **1**, in the isolated `CODEX_HOME`: the rollout | **4**, in the isolated `CLAUDE_CONFIG_DIR`: the project slug directory, the transcript, `memory`, `session-env/<session>` |
| Other changes in the isolated home | 7 deleted (5 in `tmp`, the `goals_1.sqlite` shm/wal), 37 modified (T02 §7.3's list) | 1 backup created and 1 deleted; 15 modified, among them `shell-snapshots`, `sessions`, the model-catalog cache and the account sync (`plugins/synced`, `skills/synced`) |
| `~/Library/Keychains` (18 entries) | 0 changed | **0 changed** (name, kind, size and mtime) |
| Per-run `HOME` after | empty | 114 entries: the keychain link, `Library`, `Library/Caches` and **111 under `Library/Caches/com.apple.python`**. These are the system Python's bytecode caches for standard-library modules and for the agent's test module, written by the agent's Bash-run Python. Under a real `HOME` they would have gone to `~/Library/Caches/com.apple.python`. The Codex sandbox does not let tool commands write there |
| Outside the isolated home, not attributable | 30 paths: this measuring session's Claude Code state (`~/.claude.json`, 3 backup entries, 5 transcripts); `~/.codex/logs_2.sqlite-wal` and `~/.codex/models_cache.json`; 19 Claude desktop app paths | 47 paths: this measuring session's Claude Code state (`~/.claude.json`, 7 backup entries, 16 transcripts, 3 `skills/synced`); 6 `~/.codex` paths (`state_5.sqlite` and its shm/wal, `logs_2.sqlite-wal`, `models_cache.json`, the directory); 14 Claude desktop app paths |

- **App-wide writes attributable to either row: none** within the listing roots.
- **The `~/.codex` changes** are most likely the ChatGPT desktop app, not the rows.
  - The app was running: its helper processes and an updater were live.
  - `models_cache.json` had also changed about 4.5 minutes before the Codex row started.
  - Both harness processes had `CODEX_HOME` pointed at the isolated home, and a per-run `HOME`.

  This is an inference from timing and environment, not proof.
- **The `~/.claude` changes** are most likely this measuring session's own, as in E01 §7.7. They
  include a `~/.claude/skills/synced` change at 07:31:33Z, inside the Claude row. The Claude
  row's harness had its own `CLAUDE_CONFIG_DIR` and a per-run `HOME`. Its own account sync
  landed in the isolated home at 07:29:59Z, at launch. The app-wide copy changed again long
  after the rows (manifest mtime 11:11:51Z). This is an inference from environment and timing,
  not proof.
- **Outside the listing roots: the agent's Bash wrote `/tmp/test.db`.**
  - It was 3 395 584 B, written by `catalog.py ingest corpus --db /tmp/test.db`.
  - Four Bash commands wrote it, at 07:30:36Z, 07:30:58Z, 07:31:14Z and last at 07:32:26.552Z
    (file mtime 07:32:26Z).
  - **It persisted on the host after the row and is still there.** Deleting it is the user's
    decision.
  - `/tmp` and `$TMPDIR` are not listing roots, so the side-effect diff could not see it. It
    was found in the native tool inputs during review.
  - It is not app-wide configuration, and no other tool input in either row names a path outside
    the workspace. Codex's tests used `tempfile.TemporaryDirectory`, which removes itself.
- **The measured side effects of `--allowedTools Bash`:**
  - Bash use in the isolated config dir (`shell-snapshots`, `session-env`);
  - Python's bytecode cache in the per-run `HOME`;
  - **the `/tmp/test.db` host write above**, outside the workspace and the per-run root.

  There was no app-wide write within the listing roots, and no keychain metadata change. The
  earlier statement that the Bash side effects were only the first two was incomplete. For
  T24/T27, add `/tmp` and `$TMPDIR` as listing roots, or give Claude a per-run `TMPDIR`; that
  would not cover a literal `/tmp` path like this one, so the roots are needed too.

### 7.8 Reading

- **Agent outcome.**
  - Both agents built a catalogue that passed every public and held-out query and matched the
    export exactly.
  - Claude met every frozen criterion.
  - Codex missed only the frozen minimum of three unit tests, so its row is `partial` by the
    rule set before launch. The row was not re-scored.
  - That minimum was in the criteria, not the goal. Codex's two tests do cover every input
    type the goal names, so `partial` reflects the unannounced test-count proxy, not missing
    coverage or less work. For T27, state the grading contract in the goal, or score coverage
    per category.
- **Recorder.**
  - Both traces are `valid`, with 0 gaps, clean drains and no capture loss.
  - Wire and native evidence both locate the tool fields.
  - The one redaction outside e-mail addresses is a false positive in an encrypted blob.
  - Safe-byte latency (post hoc) stayed under 250 ms in both rows, at most 226.8 ms. The owner's
    flush **cadence** reached 267 ms once in the Claude row, on a loaded host.
- **What this does not show.** Two rows of under five minutes say nothing about long
  autonomy, compaction or refresh. No context compaction and no token refresh (0 auth
  tunnels) occurred, so those stay untested. This is n = 1 per harness.

## 8. Part B results (2026-09-28)

`load-20260928T101817Z`. The driver and scripts were at `7f9e525`, clean tree; the runner was
sha256 `495adae7…ee0ab1`; schedule `full` (plan sha256 `c6848013…57adcf34`). All traffic was on
loopback.

- **Quiet window.** The orchestrator stopped cargo work. The driver's check found no `cargo`,
  `rustc`, `hdiutil`, `cc` or `ld` process (`busy_at_start=0`).
- **The host was not idle, though.** The 1-minute load average was 4.3 at the start, and the
  sampler recorded up to 12.72 during the run, with sparse-phase means of 6.6 to 8.3. The
  source of the load was not identified. Every timing figure below comes from that host.
- **Timeline:** driver start 10:18:18Z; stall trigger at +720.0 s; workload finished after
  1 810.0 s. The runner took 1 811.7 s of wall time.

### 8.1 The cross-check: frozen verdict FAIL, corrected post hoc

**Frozen verdict: FAIL.** The frozen cross-check (`early-crosscheck.py` at `7689ea5`,
`MATCH_TOL_S = 1.0`; `crosscheck-v1.json`) exited **1** (`crosscheck_exit=1` in `identity.txt`).
Two of its six checks were false, `sent_equals_retained_outside_gap` and
`gap_is_visible_and_recovered`, both caused by one dense SSE response it reported missing at
+899 s, outside the gap window.

The frozen script's sha256 is `4d9dd428c967b2c3664ff0bca139f71f5459d2a37343fe7001f29bce0909127d`,
which is the hash `identity.txt` records. The item it reported was not in fact missing.
- The item actually lost was the SSE response in flight when the sink refused the producer. It
  was sent at +735.006 s, and its stream opened at +735.008 s with 20 480 of 32 768 bytes and a
  `queue_full` gap.
- With a 1 s tolerance and identical bodies, that item took the next request's stream (+736.0 s,
  inside its [start − 1 s, end + 1 s]).
- Each later item then took its successor's stream, up to the phase's last tick, which was left
  with none.
- The smoke run hid the cascade: its window happened to reach the end of the stall phase.

**The correction changes only the matching tolerance, not the data.** `MATCH_TOL_S` went from
1.0 to 0.05 s, chosen after the run from its own data.
- **The corrected script** is `early-crosscheck.py` at `cc3b5eb`, sha256
  `4f6045d6232970a21933186faef97899b17c99c8a7764e1e8a7528bb1c52abf3`, run by hand as
  `early-crosscheck.py <run>/run-root/out <run>`. It wrote `crosscheck.json` and
  `crosscheck-summary.json`, mode 0644, outside the driver. `identity.txt` still records the
  frozen v1 hash and `crosscheck_exit=1`. Both hashes stand; neither record was rewritten.
- **Offsets measured on this run.** Over the 5 340 items with unique content (the echo bodies
  and WS messages that are not sentinels), a stream's Open lay between −4.2 ms and +107.1 ms of
  the item's start, and between −8.3 ms and +23.8 ms of its end. Over all matched items the
  end-side range is wider (`end_minus_open_s` up to 100 s), because sparse SSE streams last
  100 s.
- **The margin that matters** is the gap from a dense SSE item's end to the Open of the next
  identical stream, not the 1 s tick spacing. Measured over the 900 dense items:
  - minimum **104.2 ms**, 1st percentile 161.7 ms, median 182.8 ms;
  - **178 ms** at the one real loss (end +735.825 s, next Open +736.003 s);
  - dense SSE items lasted 0.812 to 0.865 s.
- **The committed matcher's pass band is narrow.** Post hoc, replaying the committed script with
  only `MATCH_TOL_S` changed (`post-hoc/tolerance.py`, `tolerance-load-…txt`):
  - the verdict is ok only for tolerances **from 0.009 to 0.18 s**;
  - at 0.008 s and below it fails, on clock offset;
  - from **0.19 s** up it fails again: the cascade returns.

  50 ms therefore sits in a band about 170 ms wide, with about 54 ms of headroom below the
  worst measured margin (104.2 ms). Under heavier load, SSE items stretch and the margin
  shrinks.
- **Replace the matcher before T27.** Match start-anchored (the Open must fall within a short
  time after the item's start, not before its end), or order-preserving per identical-body
  class, or give every body unique content. The sparse phase is already exposed: its 144
  identical 100-second streams overlap.
- **Independent of the matcher,** the byte arithmetic confirms exactly one loss (§8.2:
  125 694 083 = 125 706 240 + 131 − 12 288).
- `crosscheck-v1.json` is kept next to the new `crosscheck.json`.
- The corrected check was rerun on the smoke run: still ok.
- The mutants were rerun: X1–X4 caught, plus a new X5, an identical-body SSE stream lost outside
  the window, also caught. The mutated inputs were scratch copies and were not retained; only
  their summary is (`target/e02-evidence/mutations-crosscheck/summary.txt`).

**Corrected verdict (post hoc, tolerance chosen from this run's data): every check ok.**

### 8.2 Sent, forwarded and retained

| Check | Result |
|---|---|
| Load self-check | **ok**: every exchange and WS message byte-checked by the client. 125 706 240 B = the plan exactly; 0 errors. Worst tick lateness 95 ms (echo), 93 ms (SSE), 37 ms (WS), 95 ms (native); no tick over 1 s |
| Sent = forwarded (endpoint counters) | **equal** in all 12. Among them: echo 900 requests, 14 745 600 B each way; SSE 1 044 requests, 8 640 events, 29 859 840 B; WS 1 800 messages and 29 491 200 B each way; bulk 1; auth 1 |
| Sent = retained, outside the gap | **equal**. Matched: 1 930 + 15 sentinel HTTP requests, 1 929 + 15 responses, 1 785 + 15 WS client messages, 1 785 + 15 server messages. **Missing: 1**, in the window: the SSE response in flight at the refusal. Unexplained: **0**. Unmatched clean streams: 0. Unclean streams: 1, in the window |
| Loss accounted in bytes | the sink admitted 125 694 083 payload bytes: the 125 706 240 logical, plus the 131-byte goal, minus **12 288**. That is exactly the unretained tail of the one lost response (32 768 sent, 20 480 retained before its gap) |
| Stall and recovery | held **15 515 ms**, released by `queue_full`. 1 QueueFull refusal (the proxy producer), 1 Recovery (`queue_full`), 1 stream gap. 3 connection IDs re-observed under the fresh producer, all 3 first opened under the lost one. 3 884 items sent after the window, none missing. The native producer was not refused |
| Forwarding during the stall | the load byte-checked every exchange; worst lateness 95 ms (above) |
| Native | 1 stream, 7 372 800 source B, `End{complete}`, no gap. Of the file's 7 200 lines, 7 170 equal and the 30 sentinel lines redacted, 0 differ. File sha256 = the load's own report |
| Secret sentinel and credential | 0 sentinel and 0 credential hits in any decoded segment, and in the 31 other files of the run and its evidence. Each of the 60 sentinel HTTP bodies and WS messages (15 echo requests, their 15 responses, 15 WS messages each way) matched a stream with an `anthropic_key` audit span and no sentinel bytes. The native stream has 30 audit spans, one per sentinel line: 7 372 800 source B became 7 371 720 retained, 36 B less per line. `wire_response` source bytes are 44 593 152: echo plus SSE, minus the same 12 288 B |
| Proxy | 230 accepted, 229 intercepted, 1 auth tunnel, 0 admission failures, 0 transport errors. Clean drain. `capture_loss=true`: the recovered QueueFull loss |
| Literal `goalcup-verify --capture-integrity` | exit **1**, as designed: `incomplete`, sealed, 28 459 records, 188 423 843 B, 1 gap, `producer_recovered`, no torn tail. It took 0.60 s and 5 062 656 B RSS |

### 8.3 Recorder resources: peaks and time series (R19)

Prospective targets: safe-byte flush ≤ 250 ms, sync ≤ 1 s, queue ≤ 8 MiB, recorder RSS ≤ 256 MiB.

| Quantity | Measured | Target |
|---|---|---|
| Safe-byte latency, oldest byte of a segment, outside the stall (**post hoc**, table below) | max **237.2 ms**; 0 of 12 706 segments over 250 ms | 250 ms |
| Owner flush **cadence**: worst flush-to-flush interval, 8 938 flushes (a proxy, not the target) | **289 ms**, over 250 ms | — |
| Durable sync **cadence**: worst sync-to-sync interval | 834 ms. Capture-to-durable latency was not measured; its bound is roughly 237 + 834 ms | 1 s |
| Sink queue high-water (encoded payload + control) | 8 254 164 + 512 B at the deliberate stall. This is the refusal cap itself (8 MiB − 128 KiB encoded), so it meets the target by construction. Outside the stall the sampled maximum was 142 486 B | 8 MiB (8 388 608 B) |
| Recorder peak RSS (`getrusage`) | **18 759 680 B** (17.9 MiB) | 256 MiB |
| Chain / quota / reserved | 188 423 843 B (1.499 × the logical payload) of 536 870 912 (35.1 %) / **0** | — |
| IPC-encoded bytes admitted to the sink | 482 774 617 (3.84 × the payload data; R4 "IPC-enc") | — |

**Post hoc (fix round 1, not pre-registered): safe-byte latency from chain timestamps.** The
method is §7.6's, applied to 12 855 segments.
- Excluded: the goal, and the 119 segments whose first byte was captured inside the deliberate
  stall [10:30:18.970Z, +15 515 ms].
- The stall's segments waited up to 15.5 s by design.

| Segments | n | Oldest byte p50 / p99 / p99.9 / max | Last byte p50 / p99 / max | Over 250 ms (oldest) |
|---|---|---|---|---|
| All outside the stall, except the held sentinel lines | 12 706 | 41.3 / 201.0 / 206.7 / **237.2 ms** | 17.4 / 195.0 / **212.5 ms** | **0** |
| The 30 deliberately split secret sentinel lines (native, held by design) | 30 | 435 (median) / **265.97–506.3 ms** range | max 194.9 ms | 30 |

- The worst oldest-byte age, 237.2 ms, is a dense SSE response segment captured at +860.5 s.
- The 30 sentinel lines are held by design. The load writes each one in two halves 300 ms
  apart, splitting the secret. The redactor holds the undecided bytes until it can decide
  them. The safe prefix of each line is appended on schedule; the held part waits for the
  second half, then for a flush.

  This hold is bounded in bytes by `MAX_HELD_BYTES` (16 KiB), but not in time: a stream that
  pauses in the middle of a secret holds it until the stream continues or ends.

**Conclusion.**
- **The 250 ms safe-byte latency target was met outside the deliberate stall**, with a maximum
  of 237.2 ms, about 13 ms of margin. The only exceptions are the by-design held secret bytes,
  at 266 to 506 ms.
- **The owner's flush cadence exceeded 250 ms once (289 ms)**, while the host's 1-minute load
  average reached 12.7. The runner reports only the maximum (R19), so when it happened is not
  known.
- **The cause is not known.** Two mechanisms fit, and neither is ruled out:
  - CPU pre-emption on a saturated host;
  - the owner's timer order. `tick()` checks the flush first and the sync second (E01
    candidate `owner.rs:298-305`), so a sync that starts just before a flush falls due delays
    that flush by the whole `sync_all` plus directory sync.
- E00's quiet runs measured a 210 ms cadence, and the Part A Claude row 267 ms.

**Sampled time series** (`samples.jsonl`: 1 804 samples at 1 s, largest gap 1.227 s, times from
the load's own start):

| Series | D (0–300 s) | S | D (stall) | S | D | S |
|---|---|---|---|---|---|---|
| Runner RSS, mean / max (MB) | 17.6 / 18.7 | 14.3 / 18.1 | 14.7 / 18.8 | 13.6 / 15.4 | 14.7 / 16.5 | 13.5 / 15.4 |
| Runner CPU, mean / max (%) | 3.4 / 5.0 | 1.3 / 3.0 | 2.6 / 14.0 | 1.7 / 3.0 | 2.9 / 5.0 | 1.4 / 3.0 |
| Queued payload, max (B) | 124 670 | 0 | **7 750 410** | 0 | 142 486 | 0 |
| Queued items, max | 6 | 0 | 556 | 0 | 12 | 0 |
| Load generator RSS, max (MB) | 26.2 | 13.0 | 12.0 | 11.4 | 13.2 | 12.1 |
| Host 1-minute load, mean / max | 5.2 / 6.6 | 8.3 / 12.1 | 7.4 / 10.0 | 6.6 / 8.1 | 5.7 / 7.1 | 7.3 / 12.7 |

- **Growth:**
  - The least-squares RSS slope over the run is **−7.1 MB/h**, so RSS did not grow. The peak
    (18.76 MB) came in the stall phase, 0.07 MB above the first dense phase's; the phase means
    were lower after the first phase.
  - The chain grew at 375.8 MB/h overall: about 60 MB per dense phase and 2.6 MB per sparse
    phase.
- **The queue drains between flushes.** It was empty or near empty in every phase except the
  stall.
- The single largest sampled runner CPU (45.8 %) is the final sample, at the drain and seal.

### 8.4 Reading

- **Recorder under sustained load (quiet of builds, not of load):**
  - no silent loss;
  - the one injected loss is visible, recovered in a fresh epoch at a safe boundary, and
    accounted to the byte;
  - no leak, no relay corruption, no unbounded growth;
  - forwarding kept going through the stall.
- **Safe-byte latency (post hoc) met the 250 ms target** outside the stall: at most 237.2 ms,
  apart from the 30 by-design held secret lines (266 to 506 ms).
- **The owner's flush cadence exceeded 250 ms once** (289 ms), under a host load average up to
  12.7. The cause is unknown (§8.3). An earlier version of this section called this a missed
  latency target; it is not.
- **What this cannot show** (packet): refresh and auth boundaries, the installed reserved
  recorder, and anything about the post-E01 tree. T27 repeats sustained load on the installed,
  reserved recorder for eight hours.

## 9. Implications for integration (T15/T16/T25/T27)

The three most important, from the fix-round-1 method review:

1. **Define the flush budget as byte latency, and measure it.**
   - **Definition.** The 250 ms budget should mean capture-to-append latency of the oldest safe
     byte: p50, p99 and max, excluding declared stalls.
   - **Undecided-hold exception.** State it explicitly: bounded by `MAX_HELD_BYTES`, unbounded in
     time. E02 measured 266 to 506 ms.
   - **Keep the budget.** E02 met it (237.2 ms maximum), so do not amend it on E02's evidence.
   - **Changes to T15's owner:**
     - never start a sync (F_FULLFSYNC) ahead of a due flush: flush, then sync, in one tick, or
       move sync off the owner thread;
     - publish latency and cadence histograms, the time of the worst interval and per-step
       durations in health and the summary. That closes R19's deferred distributions for T27
       without a runner change.
2. **Budget the queue as time-to-loss; 8 MiB encoded is about 2 MB of payload.**
   - **Why.** R4 keeps `Bytes.data` as a JSON number array. Part B measured 3.84 encoded bytes
     per payload byte (482 774 617 for 125 694 083).
   - **Capacity.** The 8 MiB − 128 KiB payload queue therefore holds about 2 149 913 B (2.05 MiB)
     of payload.
   - **Time to loss.** That is about 15.9 s of owner stall at this load's 132 KiB/s of captured
     payload (15.5 s measured), and about 2 s at spec §9's 1 MiB/s test rate. E02 did not run
     1 MiB/s.
   - **Actions.** Either fix R4 (base64 or binary; T00-owned) before T27, or state the queue
     budget as time-to-loss at declared rates. Add the 1 MiB/s scenario to T25 or T27. The IPC
     link (T12 → T06) carries the same 3.84×.
3. **Recovery is validated in-process only.**
   - **What was covered.** The stall ran through the E01 runner's in-process `DirectSink`: the
     owner stopped, one QueueFull, then a full-speed drain.
   - **What was not.** The IPC sender and receiver (T12 R6: the old epoch closes without Close;
     T06: unknown tail), Pending and RetryFresh, repeated refusals, and refusal of the native
     producer.
   - **Actions.**
     - T16 and T25 rerun the minute-12 stall through the CP2 composition (proxy → IPC sender →
       ingest receiver).
     - Add a **slow-owner** case, throttled rather than stopped, to exercise those paths.
     - Rerun Part B on an integrated candidate before any budget decision rests on these
       numbers.
