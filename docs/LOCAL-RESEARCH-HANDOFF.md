# Local research validation handoff

Read [the target](RESEARCH-TARGET.md) and [AGENTS.md](../AGENTS.md) first. This is an execution
handoff for a local agent with a terminal, Codex login, and organizer staging access. The cloud
prepared documents, a workload, and retrieval tests; it did not launch a paid run, deploy a
server, or establish fresh platform coverage.

## Mission and authority

Make one real run flow through collection, upload, actual stored-byte retrieval, decoding, and
independent review. Then fix demonstrated defects and repeat. Use Codex Luna at high reasoning,
including native same-harness children where supported. Do not run Claude while its budget is
unavailable or silently substitute a more expensive model. Each workload has at most 25 minutes
of work plus a finish margin; request a stop at 25 minutes, aiming to finish within 30 minutes.
Inspect termination instead of treating a timer or successful command as proof that all work stopped.
Upload, retrieval, and analysis may finish afterward without model inference.

The supervising local agent may use several subagents for source review, decoder analysis,
experiment review, and documentation, with separate file ownership. They are outside the
measured workload and must not enter its capture home, prompt its harness, or edit its workspace
while it runs. Only one measured Codex run uses a given isolated home at a time. Inside that run,
native Codex subagents may work on the application. Before launch, check agent-role settings and
overrides enforce the selected Luna model/high effort for children too; do not select a stronger
reviewer model. If that cannot be enforced, skip in-run delegation and record the limitation.

Fixes in this checkout are authorized; run appropriate checks and obtain a separate review.
Staging test runs and bounded workload installs are intended. Do not deploy, modify production,
delete historical evidence, change signing trust, or widen capture/privacy policy to get a pass.
Never copy authentication files or print credentials/configuration dumps. Keep research captures
private; server export cleaning is still unfinished.

## 0. Inventory and prepare locally

Start in this tracer checkout. Use absolute paths once terminals change directory:

```sh
export TRACER_REPO="$(git rev-parse --show-toplevel)"
export GOALCUP_CONFIG_DIR="$HOME/goalcup-research-night/config"
export RESEARCH_WORK_ROOT="$HOME/goalcup-research-night/work"
export RESEARCH_LAUNCH_ROOT="$HOME/goalcup-research-night/launch"
export RESEARCH_RESULTS="$HOME/goalcup-research-night/results"
umask 077
mkdir -p "$GOALCUP_CONFIG_DIR" "$RESEARCH_WORK_ROOT" "$RESEARCH_LAUNCH_ROOT" "$RESEARCH_RESULTS"
```

Use an existing dedicated logged-in test home if one is already available; do not clone its auth
into a second home. Record paths privately. The work and launch folders must be outside Git
repositories and evidence must remain outside the measured workspace. Artifact storage needs
APFS, ext4, or XFS with the tracer's required disk reservation; overlay/tmpfs is refused. WSL2
paths must be inside Linux, not `/mnt/c`. Keep the machine awake and allow the exact rebuilt
binary in any per-app firewall.

Check `git status`, `rustup show`, `cargo --version`, `codex --version`, `python3 --version`, and
availability of `node`, `npm`, `zstd`, `psql`, and `curl`. Check `codex --help`, `codex features
list`, and, if supported, `codex debug models --bundled`. Those commands require no paid model
turn. Record the exact native Codex executable on PATH; avoid accidentally measuring an old
wrapper or another checkout's tracer binary. The connector records builds rather than pinning
the harness version.

Build fresh binaries before testing. Fetch locked dependencies once if needed, then:

```sh
cd "$TRACER_REPO"
cargo build --offline --workspace
export PATH="$TRACER_REPO/target/debug:$PATH"
export GOALCUP_CONNECTORS_DIR="$TRACER_REPO/connectors"
sh scripts/gate.sh fast
```

If Cargo/dependencies or the supported filesystem are unavailable, fix that setup or report the
blocker. An existing prebuilt binary is useful for diagnosis but cannot certify this checkout.
The full release gate also needs Worker dependencies; see [the gate](../README.md#the-gate).

## 1. Select the model and stage the run

Default to `gpt-5.6-luna`, high reasoning: this is the Luna generation used by the existing
repository measurements. The cloud's installed Codex also advertised `gpt-6-luna`; that is not
permission to change generation or price silently. Confirm the selected Luna slug and `high`
are supported by the actual local installation/account. If not, stop paid launches and report
the available Luna options. Verify actual parent and child model/effort afterward, not just config.

Use staging explicitly and a disposable empty public repository owned by the test operator:

```sh
goalcup login --server https://goalcup-ingest-staging.b-9a5.workers.dev
goalcup prep --harness codex-cli --repo https://github.com/OWNER/EMPTY-RESEARCH-REPO --work-root "$RESEARCH_WORK_ROOT"
```

Replace the repository placeholder. Follow prep's interactive sign-in and token prompts; never
put token values in arguments or chat. The account must be authorized as a staging test operator.
Inspect prep's actual output: it also reports practice readiness. The server start path requires
valid prepared repository/work-root state; do not forge it. If prep requests a short practice,
complete that separately, with a small useful prompt and the same Luna settings.

Prep opens Codex in the tracer-owned home. Exit that setup session before editing its config.
The persistent path is `$GOALCUP_CONFIG_DIR/capture-homes/codex-cli/config.toml`. Merge these
participant-owned settings there, preserving unrelated tables and avoiding duplicate keys:

```toml
model = "gpt-5.6-luna"
model_reasoning_effort = "high"
web_search = "live"

[features]
multi_agent = true
```

These are isolated participant settings, not connector edits. Do not change `~/.codex`, model
base URLs, pinned connectors, or provider routing. Use the installed Codex's supported controls
to choose approvals/sandbox settings that allow this disposable workspace's installs, localhost
server, and public network lookups. Otherwise the run can stall for approvals. Confirm syntax
against the installed version and record only a nonsecret settings allowlist. The cloud verified
search/multi-agent capability on Codex 0.159.2; older local versions may differ. The complete
config block has not been strictly validated as a live setup.

The existing `scripts/linux-matrix.sh` forces Luna/low before each run. Do not reuse it unchanged
for this high-reasoning experiment. Its earlier measurements remain valid for their own settings.

Run `goalcup doctor --harness codex-cli`. Prepare two terminals (or a supervisor-controlled PTY
with a real child wait and an independently scheduled stop). Paste the [workload](research/codex-luna-30m-workload.txt)
once into the measured harness, not into the supervising agent's own instruction stream.

## 2. Run for up to 30 minutes, with no continuation injection

Normal `--practice` runs last ten minutes. `matrix.sh --timeout` only changes waiting time.
For this experiment, use an organizer `--test` run with a manual local cutoff:

```sh
cd "$RESEARCH_LAUNCH_ROOT"
goalcup start --harness codex-cli --test
```

Approve the staging run when asked. Once Codex is ready, establish the watchdog below before
submitting the workload once, and record submission time with the supervisor's monotonic clock.
Do not rely on the model checking its clock. Poll status in short intervals so interruption
remains responsive. Record idle versus active duration honestly.
The model targets 25 minutes and may finish earlier. An idle chat UI is not continuing work.

From the second terminal, using the same config/binaries/connectors:

```sh
goalcup entries --json
goalcup status ENTRY_ID
```

Resolve the exact entry ID from the dedicated run's entries and record its run ID separately.
`seal`, `status`, and explicit `upload` take an **entry ID**; `continue` and `--check-copy` take
a **run ID**. Never guess IDs or seal some other active run. Read only the relevant run record
under `$GOALCUP_CONFIG_DIR/runs/RUN_ID/run.json` to locate workspace and per-entry `trace_dir`.

Set `ENTRY_ID` to that exact entry and start this seal-only watchdog in the second terminal:

```sh
python3 "$TRACER_REPO/scripts/research-watch.py" --config "$GOALCUP_CONFIG_DIR" --entry "$ENTRY_ID" --seconds 1500 --goalcup "$TRACER_REPO/target/debug/goalcup" >"$RESEARCH_RESULTS/$ENTRY_ID-watch.log" 2>&1 &
WATCH_PID=$!
```

The timer starts when the script starts, so start it immediately before submitting the prompt.
Its 25-minute deadline leaves up to five minutes for the existing seal/termination path. If it
starts late, subtract elapsed prompt time from 1500 seconds. Keep it supervised and inspect the
log/exit code; a cancelled or failed watchdog may leave the harness running. It sends no input,
never restarts an agent, and invokes only `seal` for the explicit entry/config. The seal call has
a 300-second timeout, not a guarantee of process death. Keep the machine awake.

At an earlier task completion, or if the watchdog fails, end the measured entry manually:

```sh
goalcup seal ENTRY_ID
```

Let the original `start` process finish its bounded wrap-up. If needed, run `goalcup upload
ENTRY_ID` afterward. A manual seal verifies a bounded research run; it does not demonstrate the
server-deadline stop. Do not force-kill the tracer as the normal stopping mechanism.
If you seal early, cancel the still-waiting watchdog and collect its exit status. Read the sealed
termination outcome and check known workload child/server processes; a successful seal command
does not imply `Confirmed` process-tree termination.

The supervisor must not send `keep going`, relaunch a coding harness inside the workload, or
edit the workload during recording. If a turn stops early, capture that outcome. Inspect why,
adjust only the next run's initial task or prelaunch native settings, and try again. No promise
is made that a single prompt guarantees 30 minutes. D37 permits native continuation configured
before launch; use it only if its actual semantics are understood and its settings are recorded.

## 3. Retrieve, decode, and review the real evidence

For each run create a private results directory named by run ID. Preserve failures and original
traces. Keep a supervisor run sheet with versions, allowlisted settings, start/stop times, IDs,
activity versus idle time, and paths. Do not record secrets or full environment dumps.

First establish `STAGING_PUBLIC_KEY` from the known staging Ed25519 public key and validate its
provenance. It is public verification material, never the private signing seed. Set `LOCAL_TRACE`
and `RUN_ID` to this run's actual values. Then verify local integrity and compare upload position:

```sh
goalcup-verify --capture-integrity "$LOCAL_TRACE" --dev-key "$STAGING_PUBLIC_KEY"
sh "$TRACER_REPO/scripts/matrix.sh" --check-copy "$GOALCUP_CONFIG_DIR" "$RUN_ID"
```

The matrix has a supplied-key fallback. Use the same key/options for local and retrieved data.

Position agreement is not retrieval. With organizer read-only database and staging R2 access
configured through the existing secure environment, fetch the actual stored chunks:

```sh
sh "$TRACER_REPO/scripts/fetch-run.sh" --bucket goalcup-traces-staging "$RUN_ID" "$RESEARCH_RESULTS/$RUN_ID/rebuilt"
```

See [fetch-run.sh](../scripts/fetch-run.sh) for dependencies and credential handling. It can also
use `--index` and `--store` for an existing private mirror; that validates reconstruction, not a
fresh remote read. The bucket defaults to production, so keep the explicit staging bucket.
If organizer read access is missing, report `reconstruction: blocked`; upload-position success
does not complete phase 1. Do not request broader researcher access or add a public download API.

For every reconstructed entry, verify integrity with the staging public key. Set `REBUILT_ENTRY`
to its actual directory and `ENTRY_ID` to that entry. Create a fresh report outside the trace:

```sh
ENTRY_REPORT="$RESEARCH_RESULTS/$RUN_ID/$ENTRY_ID.ndjson"
goalcup-verify --research-json "$REBUILT_ENTRY" --dev-key "$STAGING_PUBLIC_KEY" --output "$ENTRY_REPORT"
```

The output must end in `.ndjson` and must not already exist; use a new name for each replay.
Set these paths explicitly under the private result folder. Reconcile complete local and rebuilt
validated record count, bytes, and head. Rotation produces ordered chain parts; do not hash only
the first local file or concatenate filenames lexically. Local pruning may make the local report
incomplete while the complete server reconstruction verifies. Compare replayed observations
under identical verifier/options; explain intentional capture/report bounds rather than treating
the NDJSON as a complete transcript. Never edit raw chains to get matching reports.

Inspect actual parent and child `turn_context` model/effort in retained native evidence. List
only model/effort and event references, not entire raw messages. Compare workload artifacts and
the assistant-authored `research-evidence/ledger.md` with source records and decoder output.
The ledger is a locator, not proof. Have an independent reviewer sample actual tool activity and
context from the trace. Sample an install attempt if needed, docs lookup, local app interaction,
tests and any resulting fix, model response, and web search/delegation if they actually happened.
Mark unavailable or unexercised activities explicitly. Do not invent a failing test or reinstall
working dependencies to fill a checklist; absence is not automatically a capture failure.

Write one result packet, with a row per sampled event and separate outcomes:

| Stage | Required evidence |
|---|---|
| Workload | Actual activity, early finish/stall/failure, artifacts, durations |
| Capture | Native/wire source references, integrity, gaps and scope omissions |
| Delivery | Final position/head and signed acknowledgement verification |
| Reconstruction | Actual stored-byte retrieval, verified head/count/bytes |
| Decoding | Verifier/decoder versions, observations and undecoded/truncated content |
| Coverage | Context-visible expected events found/missing/excluded/not exercised |
| Cleanup | Harness and workload servers/children stopped, known residuals |

Preserve only sanitized fixtures in Git after the normal redaction/review process. Private raw
traces, credentials, complete run homes, and result packets stay outside this checkout.

## The overnight working loop

1. Run one bounded baseline and review the packet. Distinguish task failure, capture failure,
   decoding failure, access blocker, and expected scope limitation.
2. Give reviewer subagents specific evidence/source questions. Give a fixing agent one demonstrated
   defect and separate file ownership. Add a regression test that checks the behavioral promise.
3. Rebuild, run the relevant test and the fast gate, then obtain independent code review. Run the
   full gate before release-level claims. Do not run concurrent gates sharing a target directory.
4. Repeat the same workload once on the same platform to establish repeatability. Native child
   capture gets its own check; independent full tool-output collection stays out of scope.
5. After that path is reliable, choose one interruption/resume or upload-outage experiment within
   another 30-minute window. Use `goalcup continue RUN_ID` for a genuinely stopped harness, not
   as an automatic encouragement loop. Do not start another inference run merely to fill time.
6. Expand only to locally available platforms/harnesses with approved Luna cost. Defer Claude,
   unavailable Windows machines, and multi-hour inference runs. Record unmeasured cells plainly.

Stop paid repetition after a baseline and one successful repeat unless a specific failed property
justifies another bounded run. Use remaining time for source fixes, tests, review, replay, and a
clear handoff. Stop on persistent auth/quota, unavailable staging/retrieval access, or unsupported
Luna settings; keep evidence and report the exact blocker instead of silently changing models.

## Cloud preparation validation

The cloud environment had no `cargo`; the Rust gate could not run. Focused retrieval tests use an
existing prebuilt verifier and therefore need a fresh local rerun. The preparation fixes positive
retrieval fixtures to obey D63, keeps contradictory historical corpus evidence intact, and makes
credential-error assertions portable across shells. No runtime retrieval policy changed.
The seal-only watchdog has behavioral tests that do not wait or launch a harness:

```sh
cd "$TRACER_REPO"
python3 -m unittest discover -s scripts -p 'test_research_watch.py'
python3 -m unittest discover -s scripts -p 'test_matrix.py'
python3 -m unittest discover -s scripts -p 'test_fetch_run.py'
```

Local completion means a reproducible private result packet and a reviewed fix, if needed—not
an assertion of universal research completeness. Update [TODO](../TODO.md) and the per-harness
facts sheet with measured versions/results, retaining older checkpoints as historical evidence.
