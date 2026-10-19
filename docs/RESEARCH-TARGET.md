# What do we want to achieve?

The tracer is a research instrument for running coding agents across harnesses and operating
systems. Preserve useful source evidence, make it flow reliably to private research output, and
state what was not observed. Goal Cup's contest is one use of that evidence. This target records
the maintainer's direction from the 18 October 2026 review; it is not a claim that all of it works
today. Start execution with the [local handoff](LOCAL-RESEARCH-HANDOFF.md).

## Agreed scope

- Record model interactions and declared native session artifacts. Preserve source bytes after
  bounded capture-time redaction; interpret them later through versioned decoders.
- Tool calls and results exposed to the agent's context are sufficient. We do not need separate
  instrumentation of npm, browsers, searches, subprocess internals, or full tool-output spill
  files. If the model received a truncated result, preserve that fact where exposed. A result
  appearing only in a subsequent request is still useful context evidence.
- Subagent conversations are valuable in their own right. Check their capture and parent links
  separately; a parent's summary does not establish collection of a child's conversation.
- Capture-time redaction stays bounded. More extensive personal-information and key cleaning
  belongs in the cleaned research export. Raw captures remain private, and known credential
  leakage still requires a fix. Reproducing the retrieval acceptance path uses an organizer/local
  operator's secure, read-only staging access. The retrieval command selects a named run; its
  credentials may permit broader staging reads unless the grants explicitly restrict them.
  Keep those credentials with the authorized custodian; researchers receive cleaned exports
  rather than direct database or bucket access.
- The tracer observes and never supplies encouragement, prompts, or automatic restarts. Native
  same-harness subagents and continuation configured before launch are allowed (D10, D37).
- Current implementation is Unix-only: macOS and Linux, with WSL2 as a separate Linux environment.
  Native Windows is a future platform target; a WSL2 result is not native Windows validation.

## What the review established

The architecture is suitable: raw capture is separate from decoding, gaps remain visible,
versioned observations reference source evidence, and storage/processing are bounded. Native
and wire legs have different coverage and neither erases a gap in the other.

The main uncertainty is whether useful evidence reliably reaches the research consumer. The
matrix validates lifecycle, integrity, and uploaded position. It does not reconstruct stored
bytes or establish activity-level completeness. The report's cross-leg `consistent` state only
checks presence of native activity and decoded wire requests, not exchange-by-exchange agreement.

Known native selectors omit some subagent artifacts, notably Claude's deeper transcripts. Full
tool-output spill collection is not a required fix. Proxy transport/encoding support has limits;
HTTP/2-only traffic can fail rather than merely go unrecorded. Streaming redaction has known
semantic-boundary leakage and over-redaction cases. Measure actual effects before widening scope.

Short macOS/Linux Codex and Hermes practices are recorded in [WP0](../TODO.md#wp0-measure).
Long-run measurements, several platform combinations, and cleaned research delivery remain
unfinished. Historical measurements are not fresh reproductions. See [architecture](ARCHITECTURE.md),
[accepted limitations](CHEAT-CHECKLIST.md), and [research report semantics](research/m2-report-format.md).

## 1. Establish one reliable end-to-end path

Flow: a real Codex run → native and proxy capture → local chain → staging upload → stored-byte
reconstruction → offline verification → decoded private research output → independent review.

The local agent starts with Codex Luna/high, one initial prompt, and at most 30 minutes of live
work. Claude waits until budget is available. Use the supplied [workload](research/codex-luna-30m-workload.txt)
to exercise installs, documentation, search where available, tool use, a local application,
meaningful validation, and native subagents. A short useful run is valid evidence; do not inject
continuation to manufacture duration.

Acceptance:

1. Record the tracer, harness, model/effort, OS/architecture, connector digest, and run/entry IDs.
2. Retrieve actual stored bytes using the organizer retrieval path. Matching upload position is
   useful but is not a substitute. Verify the rebuilt chain and compare verified record count,
   bytes, and head with the complete local chain where retained.
3. Reproduce decoding with the same verifier/options. Trace sampled observations back to source
   records: initial instruction, model reply, tool call/result, an install attempt, documentation
   lookup, search if available, and child conversation if delegation occurred. Distinguish
   absent activity, unobservable/excluded activity, recorded loss, and retained undecoded content.
4. An independent reviewer can reproduce retrieval and analysis without repairing raw artifacts.
   Workload-authored ledgers help locate events; they are not independent ground truth.

## 2. Make the path repeatable across harnesses

For each shipped harness/version, maintain a small evidence table: actual platform, wire format,
decoder/version, real sample, model/tool coverage, child-session coverage, and known omissions.
Reuse shared wire decoders where protocols match. Add a harness-specific decoder only for an
actual format difference. Unknown decoding must remain unknown, not zero activity.

The local agent repairs the smallest demonstrated flow/decoder defect, adds a behavioral test,
gets a separate review, then repeats the same bounded task. Preserve failed evidence. Do not
rewrite historical fixtures to hide failures or change existing PASS/exit meanings.

Acceptance: a real sample for each tested harness follows the same retrieval/replay procedure;
expected context events can be found in retained evidence and decoded output, or their limitations
are explicit. No model/harness switches are implied by this target; Claude testing is deferred.

## 3. Establish endurance and platform confidence

First repeat the bounded path until failures are understood. Within subsequent runs of at most
30 minutes, test one controlled interruption/resume or upload outage, then rotation/pruning and
reconstruction. Measure resource growth and time to recover, with explicit start/stop conditions.
Do not combine every fault into the first experiment. A manual seal is not a server-deadline test.

Repeat on locally available macOS/Linux environments, then real WSL2. Keep unavailable cells
unmeasured. Longer multi-hour runs are a later decision, not required for this preparation.

Acceptance: expected evidence survives or loss is accurately reported; complete reconstructed
data remains decodable; orphaned work is identified within stated lifecycle limits; resource
growth is measured. A clean hash chain alone is insufficient to establish these properties.

## Evidence and stopping rules

Keep separate results for workload execution, capture integrity, delivery position, reconstructed
bytes, decoding, research coverage, and cleanup. Any may succeed while another fails. Use
`not measured` or `blocked` rather than inferring success from another column.

The first useful milestone is one reproducible run with this report. Stop repetition when the
baseline and one repeat are understood, or a stable blocker needs account access, unavailable
capabilities, another model, or a scope decision. Leave a handoff that says exactly what to do
next. The [local handoff](LOCAL-RESEARCH-HANDOFF.md) describes the working loop and evidence packet.
