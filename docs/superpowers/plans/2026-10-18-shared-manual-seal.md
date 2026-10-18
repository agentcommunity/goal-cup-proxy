# Shared manual-seal lifecycle implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task.

**Goal:** A manual seal stops the owned harness soon enough for its existing recording proxy to close normally, without manufacturing shutdown loss by waiting on a live harness.

**Architecture:** The daemon remains the sole terminator. Serialize termination before ingest/owner drain under the entry run lock, keeping capture live during termination. Exclude the actual termination duration from both capture deadline boundaries and retain the outcome across retries. No harness-specific core branch or wire-record change.

**Tech Stack:** Existing Rust daemon/CLI, synthetic lifecycle fixtures, offline cargo gate.

**Spec:** `docs/CONTRACTS.md` C6; `docs/DECISIONS.md` D22, D30, D40; `docs/TERMINATION.md`; `docs/RESEARCH-TARGET.md`.

## Global constraints

- The tracer records and never supplies prompts or continuation.
- No harness names in core code; harness-specific behavior belongs in connector data or versioned decoders.
- The daemon never links the verifier; the CLI never links the daemon.
- Preserve v5 records and existing verifier verdict meanings. No gap suppression or fabricated termination confirmation.
- Termination grace remains outside the capture finish budget. No new paid run until the synthetic regression, rebuild, gate, and independent review pass.
- Preserve failed raw evidence privately. No production or deployment changes.

## Review focus

- Existing live producers must finish after harness termination; stalled producers must still produce explicit loss.
- Concurrent launcher/external finish calls must share one committed terminal and avoid double termination.
- A finish refusal after termination must retain the real outcome across retry, especially Confirmed.
- Native final writes and final workspace state must remain captured after termination.
- Capture deadline expiry must not truncate the connector's termination grace or misstate ownership certainty.

### Task 1: Correct the shared stop/drain dependency

**Files:** `crates/goalcup-daemon/src/runtime/capture/finish.rs`, narrowly required capture state helpers, existing CLI synthetic vertical/finish-budget tests, `docs/CONTRACTS.md`, `docs/DECISIONS.md`, and architecture/termination documentation where stage order is described.

- [x] Add a behavioral regression around the existing streaming synthetic harness: establish actual WebSocket traffic, externally seal without releasing the harness, require timely owned-harness stop and producer closure with no manufactured shutdown-deadline gap. Run it and preserve the expected failure before implementation.
- [x] Move daemon-owned termination ahead of ingest/owner drain, under the existing entry lock. Keep source capture running until termination finishes; shift both capture budget boundaries by actual termination time. Persist/latch outcome through refusals and concurrent finishes without widening process authority.
- [x] Add meaningful retry/budget/concurrency regression coverage where existing tests do not already prove these promises. Preserve the existing explicit-loss cases and all P1-P7 tests.
- [x] Amend the documented stage order and rationale. Record unresolved macOS termination separately; add bounded, sanitized guard diagnostics only if needed to identify the failed authority check. Do not claim escaped orphan cleanup from an empty targeted-process list.
- [x] Run focused lifecycle, finish-budget, native-tail, recovery and termination tests; self-review. Leave changes uncommitted for independent review and root's full fast gate.

### Task 2: Validate and repeat the same bounded research workload

- [x] Independently review the net implementation and tests; repair demonstrated findings through the implementer.
- [x] Rebuild all workspace binaries and run `sh scripts/gate.sh fast` using installed Rust 1.99. Run relevant Python checks if their implementation changes.
- [ ] Run the supplied workload once more with the same model/effort/platform and seal-only watchdog. Seal early on actual task completion, no continuation input. Record idle/work/finish timing separately.
- [ ] Retrieve real indexed staging bytes, verify and decode under the same supplied staging public key/options, reconcile local/rebuilt count, bytes, head and observations, and independently sample context-visible activity.
- [ ] Update TODO and the harness facts sheet with measured outcomes and limitations, preserving historical checkpoints. Commit reviewed source/docs only after required gate; no push or deployment.
