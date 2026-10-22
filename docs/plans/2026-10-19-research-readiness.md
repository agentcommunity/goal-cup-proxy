# Research readiness phases 1–2 implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task.

**Goal:** Make the demonstrated research path trustworthy and reusable across available real harness evidence, instead of stopping after one successful bounded run.

**Architecture:** Retain raw v5 capture and verdict behavior. Repair format-specific native authorship/session provenance through decoder-versioned fields; compare local/rebuilt artifacts using the same verifier/options through a harness-neutral offline helper. Actual remote retrieval remains the existing organizer `fetch-run.sh` path. Public evidence tables expose coverage and missing cells; private packets retain source references.

**Tech Stack:** Existing Rust verifier, Python standard-library replay orchestration/tests, existing staging retrieval script.

**Spec:** `docs/RESEARCH-TARGET.md` phases 1–2; `docs/LOCAL-RESEARCH-HANDOFF.md`; `docs/research/m2-report-format.md`; measured private native-child probe and independent review. The user explicitly requested continued autonomous coding toward phases 1–2; existing approved architecture and execution method remain binding.

## Global constraints

- No harness names or model rules in shared capture/core code; native-format interpretation stays in its decoder.
- No new v5 records, integrity/termination verdict meanings, decoder kind vocabulary, Decoder API version or report schema changes in this slice. Per-decoder fields and decoder version may change explicitly.
- Preserve ordinary CLI/exec authorship promises. Do not strengthen eligibility by inventing an ID requirement or editing fixtures to hide changed behavior.
- Source-session provenance labels the container, not every record's actor or freshness. Child/fork copied history and agent-authored assignment are not new human input. Keep all tool/model/history occurrences; no global deduplication.
- Preserve raw failures and retained unknowns. No raw capture or secret in Git; sanitized source fixtures require the repository redaction/review workflow.
- No new paid launch merely to fill time. Reuse real captures; Claude inference, silent model changes, production changes and deployment remain forbidden. Staging retrieval is read-only, explicitly scoped.
- Gate before commit, separate review, no concurrent gates sharing a target directory. Source fixes and evidence replay do not establish unmeasured harness/platform/endurance coverage.

## Review focus

- First own header versus copied parent header; `payload.id` versus parent-valued `payload.session_id`; malformed/ambiguous headers and loss before identity.
- Valid CLI markers, headless exec exclusions, ordinary resume, multiple/interleaved streams, actual child assignment and copied human text.
- Generic comparison of valid, incomplete, invalid, mismatched and metadata-different traces without turning identical failure into success.
- Output containment/exclusive creation/private permissions; same verifier/key options; no raw or credentials in process arguments or public output.
- Real remote retrieval versus artifact comparison, historical versus fresh evidence, capture-only versus decoded context, and model/cost/platform restrictions.

### Task 1: Repair native fork identity and authorship

**Owner:** focused Sol implementer; independent Sol review after completion.
**Files:** `crates/goalcup-verifier/src/capture_analysis/native/codex.rs`, narrowly required native framework hooks, native behavior/report tests and version-sensitive public report expectations, registry metadata and report-semantics docs.

- [x] Reproduce the measured own-child-header → copied parent header → copied prompt/child assignment misclassification in behavioral tests; preserve red output.
- [x] Freeze bounded own source identity/launch authority from the first header. Preserve ordinary interactive positive markers without adding an unrelated session-ID prerequisite. Do not promote child/ambiguous/lost-first-header sources through later CLI-looking headers.
- [x] Add optional decoder-versioned `fields.native_context` with explicit own/parent IDs, source role, header source reference and fork-history uncertainty where supplied. Do not claim per-record fresh/history classification when unknown. Keep unsupported records source-referenced.
- [x] Cover normal CLI/exec/resume, copied headers, ID precedence, malformed/oversize/missing identity, loss/gaps, interleaved streams, source references and unchanged per-source tool/history occurrences. Bump only the affected decoder version; inspect regenerated report expectations, leaving raw fixtures unchanged.
- [x] Rebuild and privately replay existing baseline, repeat and child capture. Child human markers must reduce from three to one with recorded parent/child provenance, without changing capture integrity or byte/head evidence. Obtain independent code/evidence review.

### Task 2: Provide a harness-neutral offline replay comparison

**Owner:** separate implementation agent; no overlap with Task 1 source ownership.
**Files:** new `scripts/research-replay.py`, `scripts/test_research_replay.py`, usage documentation owned with the helper.

**Interface:** `python3 scripts/research-replay.py --local TRACE --rebuilt TRACE --output NEW_DIR [--verifier PATH] [--dev-key-file PUBLIC_KEY_FILE]`.

- [x] Add real-verifier behavioral tests, using owned copies of existing corpus fixtures: matching valid traces; matching incomplete traces; two different valid chains; invalid evidence; report metadata differences; existing/inside-trace output refusal and private permissions. No tests that assert internal function shapes or doc wording.
- [x] Run the same verifier/options for both integrity and research reports, preserving leg exit codes, stderr and NDJSON in a fresh private directory outside both inputs. Record verifier identity/hash and supplied-key provenance without key-file contents.
- [x] Emit `comparison.json`: verified record/byte/head equality, observation-line count/hash equality, full-report equality, each leg's integrity/analysis/delivery results and aggregate exit. Observation comparison includes unknowns; metadata differences remain explicit. No deduplication, raw editing or fabricated retrieval proof.
- [x] Exit 0 only when both verifier legs exit 0 and chain identity plus observation replay agree; 1 for incomplete/degraded or mismatch, 2 for invalid evidence, 3 for invocation/process/report failure. Full-summary/delivery differences are qualified separately and never called remote-retrieval success. The helper compares supplied artifacts; actual fetch remains a separate, logged operation.
- [x] Verify against a fresh binary and actual Codex local/rebuilt artifacts. Keep the helper independent of harness/model/provider names and secure binding paths. Obtain separate review.

### Task 3: Establish the available cross-harness evidence matrix

**Owner:** read-only Luna inventory/review; root coordinates actual authorized retrieval and sanitized docs.
**Files:** `docs/research/coverage-matrix.md`, links from `TODO.md` and harness-facts index; private packets outside Git.

- [x] Inventory shipped harness versions/platforms and existing real captures, distinguishing model reply, no-login smoke, synthetic fixture and capture-only data. Never inspect auth-file values.
- [x] For suitable existing staging runs, retrieve actual indexed stored bytes using existing secure bindings with read-only SQL/staging R2 GETs; then run Task 2 comparison and Task 1's fresh decoder. Do not launch a different model or manufacture missing activity.
- [x] Independent reviewer samples context-visible model/tool/native/child evidence per tested harness. Record absent activity, retained undecoded data, capture gaps and unavailable access separately. Maintain a small public matrix without private run/entry/account identifiers.
- [x] Update phase status and reproducible commands honestly. Phase 1 closes only with reproducible reviewed evidence; phase 2 closes only for the explicitly tested harness/version/platform cells. Keep remaining cells open rather than claiming universal completion.
- [x] Prepare one controlled phase-3 interruption/resume or outage check from existing tests, with expected evidence/loss assertions and resource measurements. Further inference requires an approved model/cost path and a specific untested property; multi-hour runs remain deferred.
- [x] Run required fast gate and relevant script tests, independently review the net changes, commit reviewed source/docs, and report actual phase-1/2 state plus next work. Do not stop merely because the first subtask passes.

### Task 4: Decode the demonstrated thread WebSocket format

**Why now:** Actual stored-byte retrieval and source inspection established a real Amp conversation
format distinct from the supported provider protocols. All 297 measured message streams are
accounted for: complete JSON objects/array or literal ping/pong. This is the smallest useful
remaining decoding gap for the already available non-Codex capture. No new inference is needed.

**Owner:** focused fresh Sol implementer; independent review. Root owns later gates/replay.
**Files:** new format-specific verifier module and integration behavior tests; registry/module
registration; version-sensitive report/selection expectations; report semantics docs. No connector,
protocol, daemon, CLI policy or stable API/kind/schema changes.

- [x] Read the private source investigation at `results/phase2-existing/amp-decoder-investigation.md`
  under the existing private research root and inspect actual raw shapes there, keeping raw content
  private. It is evidence, not a specification to follow blindly: its proposed cross-frame global
  dedup and wire human_message mapping conflict with existing report contracts and are excluded.
- [x] Accept only WS client/server message streams with the exact observed `amp-thread-jsonrpc`
  hint. Reassemble source segments per message under existing parser/stream bounds; reject loss,
  redaction, malformed JSON, unsupported opcode/encoding and limits conservatively with source refs.
  No cross-stream history accumulator or global deduplication.
- [x] Decode complete server `message_added` snapshots only: assistant text/thinking content as
  source-qualified model_response, assistant tool_use as tool_call, context-visible user tool_result
  as tool_result, and safely projected supplied model/usage snapshots. Supply message/block IDs
  where present. Label application/thread evidence; do not claim a direct provider request or
  provider finish outcome. Preserve model-facing name; leases/executor notifications do not replace it.
- [x] Deltas, tool leases, executor results/progress, user send/echo, settings/environment, unsupported
  objects/array elements and future forms remain referenced unknowns. Heartbeat literals may be
  recognized structural unknowns. Wire user messages do not become native human_message. Do not
  manufacture model_request/outcome or exact hidden system/inference context. Complete snapshots
  avoid counting deltas as additional logical replies; repeated snapshots still remain per-source
  occurrences and are not globally deduplicated.
- [x] Decoder outputs follow existing field sanitation/bounds, no environment/account/tool-description
  metadata projected opportunistically. Report text/tool inputs/results use existing private-output
  sanitation policies. Version this decoder independently; measured tag names this actual sample's
  scope and synthetic-only describes forms not seen. Native metadata remains capture-only.
- [x] Add RED/GREEN behavioral tests for real-shape synthetic messages, multi-part framing, array
  items, assistant text/tool/usage/model and context tool results, nonmatching streams, malformed/lost/
  redacted/oversized data, bounds, interleaved messages, unsupported/delta/heartbeat forms and source
  refs. Construct safe fixtures; do not commit private raw bytes. Existing raw fixtures unchanged.
- [x] Update registry measurement wording for OpenAI Chat Completions to the now retrieved/replayed
  real Hermes HTTP/SSE/JSON sample, after independent evidence verification. Update only appropriate
  synthetic-only forms, leave unmeasured shapes explicit. Regenerate/inspect report expectations.
- [x] Root rebuilds and replays all four actual local/rebuilt pairs plus child, independently reviews
  Amp's decoded observations back to stored source, runs required checks and commits net reviewed work.
