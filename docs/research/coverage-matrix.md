# Research validation coverage

Status: 22 October 2026. The first table is the 19 October batch; the 22 October local test day is
in its own table below it. These are measured cells, not a statement that every shipped harness
has completed research validation. Raw recordings, account identifiers and detailed source
references remain private. The [research target](../RESEARCH-TARGET.md) defines acceptance;
the [local handoff](../LOCAL-RESEARCH-HANDOFF.md) defines the experiment limits.

Capture integrity, useful model activity, stored-byte retrieval and decoding are separate
results. A lifecycle PASS or signed upload ACK establishes neither a model reply nor retrieval.
A valid report with capture-only streams establishes neither transcript decoding nor coverage.

**19 October batch**

| Harness | Measured platform/version | Activity evidence | Wire / native interpretation | Actual stored-byte replay | Child coverage |
|---|---|---|---|---|---|
| Codex CLI | macOS arm64 / 0.156.1 | Real model replies and tool pairs; supplied bounded workload | OpenAI Responses / `codex-native` | Baseline and repeat retrieved; identical local/rebuilt chain and reports | Separate local-only full-context child probe; inherited context recovered independently |
| Amp CLI | macOS / 0.0.1792209635… | Historical assistant text and one successful tool result located in raw frames | Thread JSON-RPC complete snapshots decoded; native metadata capture-only | Retrieved and verified valid; identical chain and reports | Not measured |
| Claude Code | macOS / 2.1.299 | Historical quota/error sample; no successful model reply established | Anthropic Messages / `claude-code-native` | Retrieved; equal local/rebuilt reports, both incomplete (3 gaps) | Not measured; deeper native transcript selectors remain a known omission |
| Hermes CLI | macOS / actual run version unreported; connector 0.19.0 | Historical successful model replies and tool exchanges, independently source-sampled | OpenAI Chat Completions / native snapshot capture-only | Retrieved and verified valid; identical chain and reports | Not measured |
| Codex CLI | Linux arm64 / 0.153.0 | Historical no-login or uncertain-authentication smoke | OpenAI Responses / `codex-native` | Not established | Not measured |
| Hermes CLI | Linux arm64 / actual run version unreported | Historical small exchange; successful inference unproven | OpenAI Chat Completions / native snapshot capture-only | Not established | Not measured |

Fresh macOS replays used `codex-native` 2, `claude-code-native` 2.1.259-2.1.283,
`openai-responses` 2, `anthropic-messages` 3, `openai-chat-completions` 2 and
`amp-thread-jsonrpc` 1. Each report records the verifier build and options. Linux rows have not
been regenerated or retrieved in this batch: exact staging run metadata was unavailable.

Staging signatures were checked with a supplied staging public key retained with the private
packets. Reports preserve `key_source: supplied` and `server_key_unpinned`; this is not a claim
of production-key anchoring.

No fresh Linux, WSL2 or native Windows research run was claimed in the 19 October batch. Claude
inference was deferred then; the 22 October table below changes that for Claude Code. WSL2 and
native Windows have still not run.
Earlier error and incomplete samples are retained. The Hermes row was initially mistaken for
an invalid-key smoke; reviewing this exact retrieved capture found four HTTP 200 responses,
including complete SSE and JSON replies. That correction is source evidence, not an inference
from upload success. Decoder versions belong to each generated
report, alongside the verifier build; old reports are historical unless explicitly regenerated.

## 22 October local test day

Staging only, on the owner's Mac (arm64) and Docker on it. "Linux" below means Docker on the Mac,
not a real Linux host. Source and aggregates: the [results](results/2026-10-22-local-test-day.md).
Each run signed in once; the cheapest model settings were used (Codex gpt-5.6-luna at low, Claude
Code haiku with thinking off, Amp mode low, Hermes DeepSeek deepseek-v4-flash). "Server copy equal"
means the stored upload equals the local chain. Raw traces stay private.

| Harness | Platform / version | Run | Integrity and server copy | Agent work and notes |
|---|---|---|---|---|
| Claude Code | macOS / 2.1.299 | 10 min practice | Before brotli decoding: incomplete, 2 gaps (replies not decoded). After: valid, 0 gaps, server copy equal | Before: a start-up dialog took the prompt, no work. After: 3 commits, 25 tests, time-left answered, seal refused |
| Claude Code | Linux / 2.1.283 | 10 min practice | Before: incomplete, 2 gaps. After brotli decoding: 0 gaps, server copy equal | First rerun: no answer (a permission prompt nobody could answer). After the no-prompt kit settings: 58 tool uses, 6 commits, haiku only, time-left answered, seal refused |
| Codex CLI | macOS / 0.156.1 | 10 min practice | PASS, valid, server copy equal | Worked, 0 commits: the default sandbox makes `.git` read-only and Codex waited 8.7 of 10 minutes for an approval |
| Codex CLI | macOS / 0.156.1 | 120 min, subagents, no git | Valid, 0 gaps; 14,720 records / 16.4 MB | 2 subagent rollouts, attributed; no approval stall without git |
| Codex CLI | Linux / 0.153.0 | 10 min practice | PASS, valid, server copy equal | First run did no work (prompt split by the pty). After fixes: sandbox cannot start in Docker; after the no-prompt settings, 2 commits, every turn gpt-5.6-luna low. `goalcup time-left` was `not found` in Codex's login shell; fixed (plan v5), not rerun |
| Amp CLI | macOS / 0.0.1792209635-g24a9fe | 10 min practice | PASS, valid, server copy equal | 19 commits, time-left answered, seal refused |
| Amp CLI | macOS / same | 120 min, subagents | Valid, 0 gaps; 11,740 records / 9.9 MB | Subagents ran on the parent thread with `parentToolUseId`, on claude-opus-5-5 while the thread was in mode low |
| Amp CLI | Linux / same | 10 min practice | PASS, valid, server copy equal, but no agent work (welcome screens took the prompt) | Welcome state set in the kit afterwards |
| Amp CLI | Linux / same | 120 min, build task, network cut 10 min | Valid, 0 gaps; 21,115 records / 20.9 MB | Prompt received; the acknowledgement covered the pre-cut tail 9.4 minutes after the cut |
| Hermes CLI | macOS / 0.19.0 | 10 min practice | PASS, valid, server copy equal | Worked, time-left answered, seal refused |
| Hermes CLI | macOS / 0.19.0 | 120 min, subagents | Valid, 0 gaps; 2,751 records / 55.9 MB | 2 `delegate_task` children, on the wire only, unattributed |
| Hermes CLI | macOS, 9 GiB APFS image / 0.19.0 | 120 min, build task, D65 disk drill | Valid, 0 gaps; 2,176 records / 52.1 MB | D65 PASS: paused below 1 GiB within 35 s, resumed above 3 GiB within 24 s; a pause held through the deadline and the run still sealed |
| Hermes CLI | Linux / 0.19.0 | 10 min practice | PASS, valid, server copy equal | 44 tests, time-left answered, seal refused |
| Hermes CLI | Linux / 0.19.0 | 120 min, build task, network cut 10 min, pause 20 min | Valid, 0 gaps; 2,650 records / 73.5 MB | Heartbeat failures 1 to 2 during the cut, 0 after; run state survived the pause; agent idle after about 7 minutes |

What the table adds and what it does not:

- Claude Code now has a captured, valid, gap-free run with real model replies on both surfaces.
  The fix was brotli (and zstd) decoding in the recording proxy: `api.anthropic.com` answers
  `content-encoding: br`, and each undecoded reply had been an `unsupported_encoding` gap.
  In a second round (about 02:00 UTC) both Claude Code cells stalled, on an expired login
  refresh and on an auto-mode dialog; both are fixed in the product or kit and not rerun.
- Every long run reached its deadline, sealed and uploaded, and the local tail part equals the
  server copy's tail. After about 30 minutes acknowledged parts are pruned locally by design, so
  the server copies were rebuilt with `scripts/fetch-run.sh` and verified instead. Daemon and CLI
  memory and open files stayed flat. All of this is 2-hour evidence; agents finished their
  workloads in 5 to 7 minutes and idled, so it does not show busy-run growth.
- Decoding, retrieval and child coverage were not regenerated for these runs in the results file.
  Subagent attribution is a decoder change (89703603: Amp `parentToolUseId` and `Task` results,
  Codex messages and `spawn_agent`), not a measured replay of the 22 October copies. Hermes
  children stay wire-only until a native decoder exists.
- Size, measured by `scripts/research-measure.py` on the 120-minute server copies and five 18 to 19
  October Codex traces: Codex 8.2 MB/h, Amp 4.9 to 10.4 MB/h, Hermes 26 to 37 MB/h (over 95% wire).
  Linear 168 h projections: about 1.4 GB, 0.8 to 1.75 GB and 4.4 to 6.2 GB; about 130 MB, 80 to 170
  MB and 225 to 445 MB with zstd. They come from idle 2-hour runs, so a busy run can be larger.
- Not run on 22 October: WSL2 and native Windows (no Windows machine), test runs, full-flow cells
  and the judged-commit check (no GitHub token), runs beyond 2 hours, a real Linux host, x86_64.

## Bounded Codex result

The failed baseline exposed a shutdown drain ordering defect. The reviewed shared fix terminates
the harness while capture remains live, then drains; it does not introduce harness-specific core
policy. The repeat recorded 53,129 records / 47,425,207 bytes with zero capture gaps, verified
locally and after actual staging retrieval. Decoded reports were byte-identical. All 22 wire
requests used the requested Luna model at high effort. Work lasted 175.376 seconds; there was
no continuation prompt and no requirement to manufacture a 30-minute task.

Termination remains `Unresolved(ProcessAuthorityUnavailable)`. A scoped process check found no
residual workload process, which does not prove containment. The 194 retained unknown observations
are decoder classifications, including telemetry and unclassified native record forms; they are
not 194 lost messages. Removing those classifications is not an acceptance requirement.

The separate child probe captured 2,549 records / 4,451,190 bytes with zero gaps. A blind reviewer
recovered a held-out inherited context note exactly, plus child identity, parent link, assignment,
four tool pairs and final reply. The requested child calculation failed. Recording coverage
passed; workload success did not. This local-only probe adds no staging-retrieval claim.

## Procedure and next boundary

Use the same offline verifier and options on the retained local trace and the separately retrieved
stored-byte trace. Preserve retrieval logs and failures. Compare chain identity and every emitted
observation, including unknowns, then independently check selected observations against their source
parts. Record absent activity, retained undecoded content and capture loss separately.

Phase 1 has a successful bounded staging repeat and a separate child-recording proof. Fork-aware
authorship reporting now preserves separate parent/child identities and counts one positively
marked parent input, rather than promoting copied history and child assignment to new human
input. Fresh replay preserves the original chain evidence. Phase 2 adds a real Hermes wire
activity path: 61 records / 494,111 bytes, zero gaps, 30 decoded observations and identical
local/retrieved reports. Its native snapshot remains capture-only. Amp adds a verified
storage/replay path: 963 records / 962,353 bytes, zero gaps. The new thread decoder emits
seven decoded facts: one assistant text response, one model-facing tool call, one context-visible
tool result and two supplied model/usage snapshots. It retains 302 unknown observations, including
heartbeats and unsupported application events. Local and retrieved reports match exactly.
Deltas and executor echoes are not counted as additional replies or tool calls; repeated complete
snapshots remain separate source occurrences. Hidden server-side provider requests/system prompts
are not reconstructed. Native metadata remains capture-only. Historical retrieval can extend
pipeline evidence without another inference call. It cannot create missing successful activity.
The historical Claude no-reply capture was also retrieved and replayed: 120 records / 329,889
bytes, three gaps (one shutdown deadline and two unsupported encoding), and 26 observations.
Both legs preserve exit 1 with identical chain and reports and an ACK through terminal. There
are model requests but no decoded model response. This establishes reproducible failed evidence,
not a successful Claude run or new inference.

Phase 3 real-harness interruption/endurance measurements remain open until the shorter path and
its reporting are reviewed. No further paid run is justified merely to fill time. On 22 October the
2-hour step ran with network cut, pause and disk drills (table above); 8-hour and 24-hour runs did not.


## Controlled upload-outage check

The existing unpaid `upload_e2e::a_server_that_goes_away_mid_run_gets_everything_once_it_is_back`
passed on 19 October after fresh binaries were built. It stops the loopback upload endpoint
for 600 ms after its first chunk, restarts the same store/port, reconstructs the stand-in's
stored bytes and checks exact equality with the local chain and seal/baseline documents,
valid signed evidence, final-record ACK and no duplicated ACKs.

```sh
cargo build --offline --workspace
cargo nextest run --offline -p goalcup-cli --test full --test-threads 1 \
  -E 'test(/^upload_e2e::a_server_that_goes_away_mid_run_gets_everything_once_it_is_back$/)'
```

The test itself took 1.613 seconds; the invocation including incremental test compilation took
9.10 seconds. Command accounting reported 776,978,432 bytes maximum RSS, including compilation;
that is not a recorder memory-growth or whole-process-tree measurement. This is a real OS
fixture process and local protocol stand-in, not real-harness endurance, remote R2, or agent
interruption/resume. Those phase-3 measurements remain open.
