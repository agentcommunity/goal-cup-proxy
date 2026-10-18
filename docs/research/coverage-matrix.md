# Research validation coverage

Status: 19 October 2026. These are measured cells, not a statement that every shipped harness
has completed research validation. Raw recordings, account identifiers and detailed source
references remain private. The [research target](../RESEARCH-TARGET.md) defines acceptance;
the [local handoff](../LOCAL-RESEARCH-HANDOFF.md) defines the experiment limits.

Capture integrity, useful model activity, stored-byte retrieval and decoding are separate
results. A lifecycle PASS or signed upload ACK establishes neither a model reply nor retrieval.
A valid report with capture-only streams establishes neither transcript decoding nor coverage.

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

No fresh Linux, WSL2 or native Windows research run is claimed. Claude inference remains deferred.
Earlier error and incomplete samples are retained. The Hermes row was initially mistaken for
an invalid-key smoke; reviewing this exact retrieved capture found four HTTP 200 responses,
including complete SSE and JSON replies. That correction is source evidence, not an inference
from upload success. Decoder versions belong to each generated
report, alongside the verifier build; old reports are historical unless explicitly regenerated.

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
its reporting are reviewed. No further paid run is justified merely to fill time.


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
