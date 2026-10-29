# Record everything: where we are (status at the end of the build)

29 Oct 2026. Companion to the [spec](2026-10-29-record-everything-spec.md),
the [plan](2026-10-29-record-everything-plan.md) and the
[progress ledger](2026-10-29-record-everything-progress.md). This file answers one question:
has the system got healthier, and what is still open. Part 1 is plain English. Part 2 is the
technical explainer for the record.

## Part 1: plain English

**Short answer.** Yes. Most of the holes the audit found are closed and tested, and the live
runs show the fixes working on real harnesses. Three things are not finished, and one of them
(long runs) is the real test we have not run yet.

**What is fixed (live runs show it, or tests prove it):**

- Calls on a host Goal Cup opens were partly unrecorded. Now every call on that host is
  recorded, whatever the path.
- A crafted address could hide every call. Such addresses are now refused at start.
- Request and response headers were dropped. Now they are kept, with credentials masked.
- Claude's subagent transcripts were missing from the trace. The live Claude run now has the
  subagent transcript in the trace.
- Codex users with an API key were not recorded. The live Codex run recorded 11 calls on the
  API host.
- A gateway user could be refused for a host they never use. Now only the host they need is
  checked. The rest gives a warning.
- If the recorder died mid-run, recording stopped. Now it restarts on the same port.
- The upload trusted any file named as the tunnel list. It now checks the file.
- The "recording works" signal counted things that were not model calls. It now counts model
  calls only.
- Hermes's state database was not in the trace. It is now copied once at the end of the run.
- Participants were not told what is recorded. The text and docs now say it.

**What is still open:**

- **Core, needs a decision from you:**
  1. The Hermes version we ran cannot start subagents (its delegation tool fails every time).
     Hermes runs therefore have no subagent transcript. Fix: a Hermes version that works, or
     accept it for now.
  2. Long runs with many subagents can lose transcripts. The cap is about 32 open subagent
     files at once. Past that, the rest are lost to one gap record. Fix: raise the cap or reuse
     slots. This is design work we have not started. It matters for the 168-hour run.
  3. The redactor masks some things the spec says to keep. It masks email addresses and some
     rate-limit blocks inside the chain, so the subagent copy is not byte-for-byte the file on
     disk. Fix: your call on whether those rules stay.
  4. Credentials under an ordinary header name are masked by name and scheme only. A credential
     hidden under a plain name such as `X-Session-Handle` would be stored. Fix: value masking.
     Your call, because it is a privacy trade-off.
- **Edge cases (rare, known, mitigated by a note or a guard):**
  - A crash can lose the connections open at that moment. Connections that arrive during the
    short rebind window are refused with no record. The chain has no gap reason for this yet
    (the format is frozen).
  - After five restarts in one run, the recorder refuses calls for the rest of the run. Five is
    a board decision, not from the spec.
  - The "on this computer" check runs once at start. A name that changes its answer later is
    not caught.
  - The "recording works" line lists the hosts we configured, not the hosts that got traffic.
  - Some tests fail only in this sandbox (root user, no IPv6). One proxy test failed once and
    then passed. These are test-environment problems, not product bugs.
- **Not verified yet:**
  - No run has lasted longer than about 5 minutes. Nothing is known past two hours.
  - The gateway run and live negative runs (base path refused, host blocked, proxy killed
    mid-run) are not done live. Their focused tests pass.
  - The "decoded calls" count moves from run to run (5 to 11). We think that is the model's
    behaviour in a short run, but we have not proven it.
  - The Codex subagent check could not compare the trace with the file on disk, because the
    child file is not archived by the kit.

**Is the system healthier than before?** For what this build set out to do, yes. The known
holes are closed or narrowed, and the live runs show more recorded, not less. It is not yet
proven for long runs, and it is not yet safe to ship without the Worker deploy and the release
order below.

**Before anyone ships this:**

1. Deploy the Worker before any client from `main` seals (the tunnel-list check needs it).
2. Ship the new Claude and Hermes connector forms with the client that understands them.
   Older clients refuse them.
3. Decide the four core items above.
4. Run the long runs (24 and 168 hours) with the same checks.

---

## Part 2: technical explainer (for the record)

### 2.1 What changed, by component

**Proxy (`crates/goalcup-cli/src/proxy/`)**

- `http.rs` `select()`: every exchange on an approved host gets a capture. Path rules pick only
  the protocol hint. Exchanges that cannot be kept (a method over 64 bytes) write a
  `LimitExceeded` gap. A POST counts as a model call only if a host-policy row names its path
  (`ExchangeMeta::counts_as_model_call` in `metadata.rs`; WebSocket uses the same helper).
- `headers.rs`: `masked_head` keeps the start line and every header, with values replaced by
  `[masked]` when the name is `authorization`, `proxy-authorization`, `cookie`, `set-cookie`,
  `x-api-key`, `api-key`, `x-goog-api-key`, or contains `key`, `token`, `secret`, `password`,
  `auth` (case-insensitive). `mask_value` also masks URL userinfo, `Bearer`/`Basic` credentials
  in any header, and credential-named query values (percent-decoded names). Header streams use
  hint `http-head-v1`, have no method, and are gated like the body stream.
- `mod.rs` supervisor: `supervise_accept` restarts a dead accept loop on the same address (at
  most 5 per run; the count carries over `goalcup continue` via `recorder_restarts_before`).
  Each restart writes a host-log line and a run-state problem. An unpolled connection dropped by
  a restart writes a `CutUnsettled` gap. Known limits: live connections are lost on a panic
  (their `JoinSet` is dropped); connections in the rebind window are refused with no record.
- `api.rs` `ProxyLoss`: `CutUnsettled` maps to `ShutdownDeadline` (nearest existing reason; v5
  is frozen, so no new reason was added).

**Refusals and start (`model_host.rs`, `commands/start/`, `commands/prep.rs`)**

- Refused when recording is impossible: unclean path (`protocol::capture_policy::unclean_path`,
  which covers dot segments, `%2e`, backslash and WHATWG-sensitive bytes), an authority with a
  backslash, space or control byte, Goal Cup's own event server, `agentcommunity.org` and its
  subdomains (`TRACKER_HOSTS`), plain http, loopback, and names that resolve only to this
  computer (`preflight::resolves_only_to_loopback`: loopback, `0.0.0.0`, `::`, mapped loopback;
  a 10 s bounded lookup).
- `provider_hosts`: the one place that decides hard versus warn. With a model service, that
  service is the only hard host. Without one, only the first profile host is hard, so a ChatGPT
  login is not blocked by the API host. Setup and start both call it.
- `refuse_if_on_this_computer` runs at start, continue and setup. The file-based refusal carries
  an `EditSetting` (key, full path, fallback), so it says what to edit.

**Upload and Worker (`upload/uploader/finish.rs`, `server/worker/src/runs.ts`)**

- Client `read_tunnels`: `O_NOFOLLOW`, regular file named `tunnels-<entry>.jsonl`, at most
  8 MiB, first line parsed as JSON with `format` and `entry_id` compared as fields (the Rust
  writer sorts keys, so byte comparison would reject real lists).
- Worker `takeDocument` runs the same check after the size check, so repeat seals are covered.

**Native capture (`crates/goalcup-protocol/src/harness/`, `crates/goalcup-daemon/src/`)**

- `MAX_NATIVE_ROWS = 8` equals the daemon's `MAX_NATIVE_SOURCES`. Each row needs a unique name
  and a distinct (root, pattern). Cross-field rules run per row. `native_plan` returns one spec
  per row.
- Claude rows: `subagents` (`projects/*/*/subagents/agent-*.jsonl`, append-only, hint
  `claude-code-jsonl`), `subagent-meta`, `tool-results`, and two workflow-subagent rows. The
  direct shape is observed in one real run; the wider patterns come from community write-ups.
- Limits (`native_capture.rs`): 32 open append-only streams (`MAX_OPEN_STREAMS / 4`), 1024
  tracked files across all rows. An append-only file keeps its stream until finish. Files that
  never get a slot become one streamless `LimitExceeded` gap at finish.
- `final_snapshot` form: never polled during the run. Read once at finish, only when the harness
  exit is `Confirmed` or `AlreadyStopped` (not `Unresolved`). The finish pass drains the capture
  queue between reads (`finish_draining`), so a copy past about 8 MiB is whole. Budget
  `NATIVE_FINISH_BYTES` 64 MiB; over it writes the existing gap. The Hermes state row uses it.
  `has_native_session` ignores these rows (D73); `listed_home` drops only the files the pattern
  names.

**Redaction (`crates/goalcup-daemon/src/redact.rs`, `redact/stream.rs`)**

- Form-encoded refresh tokens (`refresh_token=...` with a `%XX` escape or a dotted value,
  16-unit floor) are masked. JSON keys such as `accessToken` match. Escaped JSON values are
  masked whole. Spaced code (`let token = entry.keys.x`) is left alone.
- Existing rules also mask email addresses and Codex rate-limit blocks. That is the cause of the
  byte-for-byte gap for subagent transcripts (Part 1, open item 3).
- Known residual: no value-based masking for credentials under ordinary header names.

**Verifier (`crates/goalcup-verifier/src/capture_analysis/`)**

- `is_artifact_hint` (in `goalcup-protocol/src/capture.rs`) is the one rule that excludes header
  and tunnel streams from model-call counts. The evidence label uses the model-request count.
  `cross_leg.wire_streams` keeps its old meaning; `cross_leg.wire_model_request_streams` is new.
  `RESEARCH_SCHEMA_VERSION` is still 5 (owner decision).

**Participant text (`commands/start/finish.rs`, `proxy/tunnels.rs`)**

- "Recording works: N model call(s) recorded from <configured hosts>." when N > 0. The no-calls
  block always prints when N is 0. The sealed-hosts line reads
  `sealed hosts (passed through unread): ...`, with `size unknown` for all-cut hosts.

**Docs**

- `RULES.md`, `PARTICIPANT-GUIDE.md`, `CONNECTOR-SCHEMA.md`, `OPERATING.md`,
  `server/worker/README.md`, `CONTRACTS.md`, `UPLOAD-API.md` and the harness facts sheets are
  updated. Masking wording says headers and query values are masked by name and scheme, and
  bodies by pattern, with `<redacted:KIND>` markers. Echoed keys are not reliably caught.

### 2.2 Measured evidence (29 Oct, main at the end of the build)

Kit: one short practice session per harness, through the local dev Worker, with the test keys.
Before-numbers are from the start of the build (`scratchpad/kit/before.md`).

| Run | Decoded model calls | Wire streams | Evidence | Subagent | Leak |
|---|---|---|---|---|---|
| Claude Code, small model, thinking off (before) | 10 | 20 | wire_and_native | 0 in chain, 1 on disk | none |
| Claude Code (end of build) | 9 | 88 | wire_and_native | 1 on disk, 1 in chain, 2 conversations | none |
| Codex 0.153.0, low effort (before) | 10 | 20 | wire_and_native | 0 | none |
| Codex 0.153.0 (end of build) | 11 | 48 | wire_and_native | spawn_agent ran; child in chain | none |
| Hermes 0.19.0 (before) | 6 | 14 | wire_only | 0 | none |
| Hermes 0.19.0 (end of build) | 5 | 24 | wire_only | 0; delegation fails | none |

All end-of-build runs: integrity valid (anchor signed), sealed, `server_key_unpinned` as the only
finding (the dev Worker key is not pinned).

Not measured: the gateway run, live negative runs, runs longer than about 5 minutes.

### 2.3 Known tooling gaps (not product bugs)

- The kit summary counts subagents from a path shape the chain does not use. It reads 0 when a
  subagent ran. Fix before the next measured run.
- The kit does not archive the child rollout from disk, so the Codex byte comparison cannot be
  done from the archive.
- One auxiliary request in the Codex run has no identified purpose.
- Some tests need a non-root user and IPv6 (sandbox limits).

### 2.4 Gate and CI

- Each PR passed `sh scripts/gate.sh fast` in the sandbox (Linux, Rust 1.99.0, 8 vCPU), in
  about 150 to 310 seconds, and the hosted `check` job.
- Worker tests (`npm test` in `server/worker`) ran when `server/` changed.

### 2.5 Decisions recorded for the owner

See Part 1, open items 1 to 4, and the owner list in the progress ledger (Task 14 notes). These
are not made in code. Each is a single change once decided.
