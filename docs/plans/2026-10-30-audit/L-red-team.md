# Lane L: red team. Unrecordable runs, day-3 breakage and cheats

Base: `/home/user/goal-cup-proxy` at 4b226de2. Read-only. Nothing was built or run. Line counts are
`wc -l` of src files (Rust src files also hold inline tests). "Inferred" means read from code, not
measured. "Unverified" means it could not be checked. Docs read in full for the cheat half:
`docs/CHEAT-CHECKLIST.md` (305 lines), `docs/TERMINATION.md` (262), `docs/RULES.md` (489), plus the
verifier, daemon, CLI and Worker code cited below.

The 2-hour run report (`docs/research/results/2026-10-30-two-hour-run.md`) does not exist on any
branch yet. Nothing in this appendix is checked against it, and nothing measured shows how stopping
or recording behaves in the Vercel Sandbox.

## Lane summary

This lane asked two questions. First: what makes a week-long run unrecordable, or breaks the harness
on day 3? Second: which cheats does the record expose, and do they stay exposed after the likely
cuts? On the first, three things in today's code can end recording for the rest of a week, or stop
the harness, with no warning to anyone during the run: the proxy's 128-stream table turns wire
capture off for good, a dead `goalcup start` leaves the harness pointing at a dead proxy port, and
the 33rd live subagent transcript waits for a native stream slot that never frees. Each has a small
guard. Several quieter paths (a harness or tool that ignores `HTTPS_PROXY`, an update that moves
files, TLS inspection, a new host) end in silence: a sealed run with no gap. On the second, the only
leg an agent cannot forge is the wire, and the only real anti-tamper fact today is the Worker's
append-only chunk store with a sha256 per chunk and a server `received_at`. The hash chain, the
seal, the research decoders, the termination proof, the byte-for-byte check and the email rule do
not carry any cheat control that is needed at event time. Cheat detection that matters is small,
lives on the server or at start, and ends in a human decision.

## Top findings

1. **Point A: stream table full ends wire capture for good.** Once 128 proxy streams are open, the
   emitter latches `Disabled` for the rest of the segment. The harness keeps running. Only a local
   counter says so (`crates/goalcup-cli/src/proxy/api.rs:449-454`, `:220-225`, `:706-710`). About
   1 line to make it recoverable.
2. **Point B: a dead `goalcup start` blocks the harness.** The harness keeps `HTTPS_PROXY` aimed at a
   dead port, so every model call is refused; Claude Code retries for about three hours
   (`proxy/mod.rs:1019-1022`, `:1078-1088`; `commands/start/proxy.rs:65-66`;
   `connectors/claude-code-cli/harness.toml:61-65`).
3. **Point C: subagent 33 onward is not read during the run.** 32 native streams, never closed while
   a file is live, plus a global 1,024-file map that is never pruned
   (`goalcup-daemon/src/native_capture.rs:276-282`, `:1555-1557`, `:280`, `:803-806`, `:1323`;
   `runtime/capture/owner.rs:145-147`).
4. **Silent bypass is the biggest blind spot.** A harness or tool that ignores `HTTPS_PROXY` yields a
   sealed run with no gap; the verifier exits 0. The only warnings are a local "no model calls
   recorded" after 10 minutes when the count is exactly zero, and verifier hint labels that never
   change the exit code (`run/mod.rs:257-258`, `:295-297`; `capture_analysis/report.rs:334-405`).
   A model-call count in each heartbeat plus a Worker flag is a few lines.
5. **The proxy does all TLS, decoding and WebSocket inflate on one worker thread**
   (`commands/start/proxy.rs:96-100`). Point 3 says never slow the harness. 1 line.
6. **Upstream trust is Mozilla roots only** (`proxy/tls.rs:22-24`, `:231-234`). Office TLS
   inspection or a new provider CA breaks every model call for the rest of the run. The start
   preflight is TCP only, so it would not catch this (`proxy/preflight.rs:1-13`).
7. **The server does not check the chain.** No verify job exists (`crates/goalcup-verifier/src/lib.rs:1-2`,
   `docs/OPERATING.md:310`, `docs/SERVER-DESIGN.md:159`). Append-only offsets plus `received_at`
   is what stops a rewrite of uploaded bytes (`server/worker/src/chunks.ts:66-70`, `:81-93`).
8. **No cheat control needed at event time depends on the chain, seal, decoders, termination proof,
   byte-for-byte check or email rule.** Decoders can be written after Friday, as long as bytes are
   stored raw minus secrets.
9. **Some member binding must survive a device-approval cut.** If the login credential can open runs,
   an agent that reads it can open a new run and replace the participant's run
   (`server/worker/src/runs.ts:130-135`).
10. **Four doc claims are false today** (section 7): an hourly verify job, a verifier second-harness
    report, in-run version sampling, and `assistance.explicit_records`.

## 1. The three run-enders

| # | What | Effect | Record shows | Where |
|---|---|---|---|---|
| A | The proxy's own stream table fills once (128 open streams) | Wire capture is off **for good** for the rest of the segment. The harness keeps running. | One local refusal counter. No gap is chained at that moment; the tail shows as loss at the finish (exact form unverified) | `crates/goalcup-cli/src/proxy/api.rs:449-454`, `:220-225`, `:706-710` |
| B | The `goalcup start` process dies, or the accept loop dies a sixth time | The harness keeps `HTTPS_PROXY` pointing at a dead port. Every model call is refused. The harness is blocked, not just unrecorded. | Silence for the calls never made. `recorder_lost` in local `state.json` only | `proxy/mod.rs:1019-1022`, `:1078-1088`; `commands/start/proxy.rs:65-66` |
| C | 33 or more live subagent transcripts (Claude Code, Codex) | Transcript 33 onward waits for a stream slot that never frees. Nothing of it is read during the run. | Silence during the run, one stream-less `LimitExceeded` gap at the finish that names no file | `goalcup-daemon/src/native_capture.rs:276-282`, `:1555-1557`; `runtime/capture/owner.rs:145-147` |

The smallest guard for each:

- **A:** treat "too many open streams" like a full queue: a recoverable `Failed`, not a terminal
  `Disabled`. One line in `latch`/`open`. Or raise the bound far above any real load.
- **B:** no restart cap (restart with backoff for ever), and make the proxy the last thing to die:
  ignore `SIGHUP`/`SIGPIPE` in `goalcup start` while the harness lives. A proxy outside the
  `goalcup start` process (inside the long-lived daemon) is the real fix, but it is not small.
- **C:** close an append-only stream that is caught up and quiet, and reopen it later (phase 2
  note 2 already designs this), or raise the 32 and the 1024 to numbers no run reaches.

## 2. How data flows, in one paragraph

`goalcup start` runs the recording proxy in its own process, on one tokio worker thread
(`commands/start/proxy.rs:96-100`). The harness gets `HTTPS_PROXY=http://goalcup:<secret>@127.0.0.1:<port>`
and a CA variable, and nothing else from the participant's shell except `PATH`, `TERM`,
`COLORTERM`, `LANG`, `SHELL` and `LC_*` (`launch.rs:333`, `:392-410`, `:855-860`). The proxy decrypts
only approved hosts; every other host is an opaque tunnel noted in a side file written at finish
(`proxy/tunnels.rs:1-8`). Decoded bodies go over loopback TCP to the daemon, which redacts and
chains them, and which also reads the harness's session files (`native_capture.rs`). The uploader
in `goalcup start` sends chain chunks to the Worker every 30 minutes and never stops the run on a
failure (`upload/uploader.rs:9-35`, `upload/policy.rs:79-90`). The Worker takes uploads until 24
hours after the deadline (`server/worker/src/db.ts:17`, `:39-46`; `runs.ts:109-113`). On the
server, each chunk must start at the exact offset, `prev` and `first_seq` the server holds
(`server/worker/src/chunks.ts:66-70`); R2 refuses bytes whose sha256 differs from the header
(`chunks.ts:6`, `chunks.ts:197-200`); and the server stamps each chunk with its own clock,
`received_at` (`chunks.ts:81-93`, `server/worker/src/db.ts:144`).

## 3. Unrecordable runs, scenario by scenario

Legend for "Record shows": **gap** = a chained statement of loss; **silence** = nothing in the
chain says anything is missing; **local only** = said in `state.json`/`host.log`, never uploaded;
**broken** = the harness itself fails.

### 3.1 A careless participant

| How it happens | What is lost | Record shows | Smallest guard |
|---|---|---|---|
| Closes the terminal or the SSH session drops. `SIGHUP` reaches both `goalcup start` and the harness (they share the process group, `launch.rs:45-60`). The daemon survives (`daemon/spawn.rs:16-17`, `setsid`) | The run stops. Uploads stop until `goalcup upload` or `continue` | Gap (unsealed entry, `IngestLost` on reopen) | Say "run inside tmux or screen" at start. Nothing in code |
| Only `goalcup start` dies (OOM, panic, `kill`), and the harness lives on (e.g. started under `nohup`, or it ignores `SIGHUP`) | All later calls, and the harness itself: the proxy port is dead, so every call is refused. Claude Code then retries for about three hours (`connectors/claude-code-cli/harness.toml:61-65`) | Broken + silence | Ignore `SIGHUP` in `goalcup start` while the harness runs; end the harness if the proxy is lost |
| Keeps the provider key or a gateway URL in a shell variable (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `ANTHROPIC_BASE_URL`) | Nothing reaches the harness: the launch builds the environment from an allowlist (`launch.rs:15-16`, `:37-43`, `:1473`). The harness asks for a login, or calls the default host without the gateway | Broken at start (visible), or the gateway's calls tunnel unread (silence) | Say it in `goalcup prep`. Keep the allowlist; it is what keeps a base URL out (D1) |
| Starts a second competition run on the same account (a teammate's laptop, a retry) | The first run is replaced and stopped (`server/worker/src/db.ts:438-440`, `db-local.ts:353-354`) | Gap: the heartbeat answers `replaced` | None needed. It is the one-agent rule. Make the start screen say it |
| Runs on a machine whose clock is more than one hour wrong | The proxy CA is not yet valid (`proxy/tls.rs:55-59`, `:119-120`). Every intercepted call fails TLS | Broken; `client_tls_failures` counter only | Check the clock against the Worker's `Date` header at start |

### 3.2 A hostile participant (recording side)

Most of this is already listed in `docs/CHEAT-CHECKLIST.md:29-37`, `:92-97`, `:130-145`. The cheat
side is in section 5. The new points:

| How it happens | What is lost | Record shows | Smallest guard |
|---|---|---|---|
| Puts `HTTPS_PROXY` or `NO_PROXY` in the `env` block of Claude Code's `settings.json` (Goal Cup's home, or the workspace's `.claude/settings.json`). Claude Code applies that block to its own process (unverified that it beats the launch value) | Every model call goes direct | Silence in the chain; tunnels list does not see it either (no CONNECT is made); verifier's `native_only` label is the only hint | At start and at each `continue`, refuse a settings file whose `env` names a `*_PROXY` or a CA variable. Small: the reader for `env.ANTHROPIC_BASE_URL` already exists (`connectors/claude-code-cli/harness.toml:40`) |
| The agent kills or `SIGSTOP`s `goalcup-daemon` | Wire and native capture for the rest of the segment. The proxy's sender gives up after 10 failed reconnects in a row (`proxy/ingest.rs:51-56`, `:104`) | One `IngestLost` gap on reopen, then silence | Accept: it is visible as a gap. Tell the participant in the heartbeat |
| The agent opens 1,024 tunnels through the proxy (a crawler, a load test) and keeps them open | New connections, the harness's model calls included, get `503` (`proxy/mod.rs:110-112`, `:994-997`) | Broken; counted in `refused_over_capacity` only | Never refuse an approved-host connection for capacity; cap only tunnels. Or no cap, with the fd limit raised to the hard maximum |
| Uses OpenClaw's own proxy setting (`OPENCLAW_PROXY_URL`, `proxy.proxyUrl`), which wins over `HTTPS_PROXY` (`git show origin/opencode-openclaw:docs/harnesses/openclaw-cli.md`, lines 74-75, 103) | All calls | Silence | Only matters if an OpenClaw connector ships; refuse that key then |

### 3.3 An auto-updating harness

Claude Code's connector sets `DISABLE_AUTOUPDATER=1` (`harness.toml:60`) and Codex gets
`check_for_update_on_startup=false` (`connectors/codex-cli/harness.toml:46`). A participant can
still update by hand between segments, and the build is recorded, not pinned (ARCHITECTURE section 4).
The version is probed once per launch (`crates/goalcup-cli/src/commands/start/phases.rs:273-281`),
so a change shows only between segments (cheat C13).

| How it happens | What is lost | Record shows | Smallest guard |
|---|---|---|---|
| The update moves the session or subagent files (new folder name, new depth) | The moved transcripts. The wire still records the calls | **Silence.** Every subagent row has `warn_missing = false` (`claude-code-cli/harness.toml:115-148`), and the native-silence warning fires only when no native file at all was read (`run/mod.rs:299-305`), which the main transcript prevents | Count model calls per conversation on the wire and compare with native files at finish; or one line at finish: "N subagent files read" |
| The update stops writing a session file at all (this already happened: Hermes 0.21.1, `CHEAT-CHECKLIST.md:62-63`) | The native leg | Silence (`warn_missing = false`, `connectors/hermes-cli/harness.toml:109`) | Same line at finish |
| The update sends a request head over 64 KiB (more beta flags, big cookies) | That connection goes opaque for the rest of its life (`proxy/http.rs:86`; mod.rs docs `:23-26`) | Gap (`DecodeFailed` loss) per connection | Raise `MAX_HEAD_BYTES` to 1 MiB; it guards nothing real |
| The update offers only `h2` in ALPN, or pins its certificate | Every intercepted call fails: the proxy offers only `http/1.1` (`proxy/tls.rs:27-28`, `:213`; `connect.rs:624-637`) | Broken; counter only | Nothing small. A thin proxy that speaks h2 is not small. Detect at start with a version probe and refuse loudly |
| The update calls a new host (a regional or CDN name) | Those calls | Silence in the chain; host and bytes only in the tunnels side file, written at finish (`tunnels.rs:1-8`) | Approve every host the provider table names for the harness, and chain one `Loss` for a tunnel whose host ends in a provider domain |

### 3.4 A provider change

| How it happens | What is lost | Record shows | Smallest guard |
|---|---|---|---|
| New content coding, or zstd with a window over 8 MiB (`proxy/http.rs:102`) | The body bytes. Only decoded bytes are kept; the encoded suffix is dropped (`websocket.rs:41-44`, `http.rs:1363-1378`) | Gap (`UnsupportedEncoding` / `DecodeFailed`) | Keep the raw bytes when decoding fails (record raw, decode later) |
| New CA chain not in the bundled Mozilla set (`proxy/tls.rs:22-24`, `:231-234`) | Every call | Broken | Add the system roots to the upstream store |
| Model API moves to a new path on an approved host | Nothing: every request on an opened host is recorded (Task 1, progress doc) | Recorded | None |
| New host | See 3.3 | Silence | See 3.3 |

### 3.5 A laptop in a bag

| How it happens | What is lost | Record shows | Smallest guard |
|---|---|---|---|
| Lid closed for hours. Processes freeze; open streams break on wake | The in-flight response tail | Gap-like: the stream ends `Interrupted` | None. Harness retries |
| Laptop moves to an office network with TLS inspection (a company entrant on day 3) | Every model call: the proxy checks upstream against Mozilla roots only, so the inspection certificate fails, though the harness alone would have trusted it. The start preflight is a TCP connect only (`proxy/preflight.rs:1-13`), so it would not have caught this at start either | **Broken** for the rest of the run | Upstream roots = Mozilla + system store |
| Asleep or offline past the deadline plus 24 hours | The unsent tail: the Worker answers `410` (`runs.ts:109-113`; `db.ts:17`) | Gap on the server (run `incomplete`) | Say the 24-hour rule at start and in the deadline notice. Or keep the close window longer; it costs nothing |
| A long sleep during the deadline | Wrap-up (one hour, counted through sleep, `uploader.rs:27-29`) may expire; the rest is left to `goalcup upload` by hand | Local only, until the participant acts | Have `goalcup status` and the next `goalcup` command run the upload |

### 3.6 A thousand subagents

| How it happens | What is lost | Record shows | Smallest guard |
|---|---|---|---|
| More than 32 live append-only transcripts (Claude Code subagents, Codex child rollouts). Streams never close while the file is live (`native_capture.rs:44-46`, `:1555-1557`; phase 2 notes, Note 2) | Every transcript past the 32nd, for as long as the 32 stay open, which is the whole segment | Silence during the run; one stream-less gap at finish (`owner.rs:145-147`) | Point C in section 1 |
| More than 1,024 matched files in all rows together (transcript + meta + every `tool-results/*` file); the map is global and never pruned (`native_capture.rs:280`, `:803-806`, `:1323`) | Every later file | One stream-less `LimitExceeded` gap when the count rises (`native_capture.rs:94-101`) | Raise to 20,000 (the hard ceiling, `:187`). Measure the poll's `stat` cost; a lifetime of `tool-results` files can pass 20,000 too |
| Native read rate: 256 KiB per poll, polls every 100 ms (`native_plan.rs:11`; `owner.rs:16`), about 2.5 MB/s at most (inferred) | Nothing during the run, but a backlog. At finish only 64 MiB more is read (`owner.rs:13`, `:128-135`) inside a 180 s budget (`goalcup-protocol/src/control.rs:708`) | Gap (`ShutdownDeadline`) per stream not reached | Read more per poll when there is backlog; give the finish no byte cap, only the time cap |
| Over 128 concurrent open wire streams (in-flight responses, WebSocket messages) | **All wire capture for the rest of the segment** (point A) | Local counter, then loss at finish | Point A in section 1 |
| Over 96 open wire streams while the native side holds 32 (the entry-wide 128 is shared, `goalcup-daemon/src/ingest.rs:1335-1336`) | The daemon drops the epoch; the proxy recovers at the next request (`ingest.rs:379`). Under steady load, quick losses in a row reach 10 and the sender gives up for good (`proxy/ingest.rs:51-56`; inferred, not measured) | Gaps, then silence | Raise the entry bound with the native one; count a `LimitExceeded` loss as no failure in the reconnect row |
| Over 128 captured connections (keep-alive pools: idle connections hold a place) | Each extra connection is relayed uncaptured for its whole life | Gap (`capture_table_full` loss) | Raise `MAX_OPEN_CONNECTIONS`; it is a bookkeeping table, not fds |
| All TLS (both legs), decoding and WebSocket inflate run on **one** worker thread (`commands/start/proxy.rs:96-100`) | Nothing lost, but each call waits behind every other. Point 3 says never slow the harness | Silence (latency is not recorded) | `worker_threads` = number of cores. One line |

### 3.7 A 40 GB session database

| How it happens | What is lost | Record shows | Smallest guard |
|---|---|---|---|
| Hermes `state.db` is a `final_snapshot` of a **shared** home that grows across all runs (`connectors/hermes-cli/harness.toml:111-121`). Copied only after exit, at most 64 MiB, inside 180 s | Everything past 64 MiB | Gap (`ShutdownDeadline`) | Do not copy shared databases. Use the wire for Hermes. If a copy is kept, copy it to a file beside the chain with its own hash, not through the redactor |
| A `snapshot` row is re-read in full every time the file changes (`native_capture.rs:47-49`). Hermes 0.19/0.20 rewrite `session_*.json` each turn (`hermes-cli/harness.toml:103-109`) | Nothing lost; the chain grows with the square of the session size (inferred: 1,000 turns of a file averaging 25 MB is 25 GB) | The run hits disk or the 64 GiB server cap (`server/worker/src/context.ts:12`), then `413` for ever, retried without end (`uploader.rs:20-24`) | Read a changed snapshot only when quiet, at most once a minute, or only at finish |
| A SQLite store under OpenCode or OpenClaw (branch `origin/opencode-openclaw`, both facts sheets) | All of it: schema 1 refuses SQLite (`native_capture.rs:123-124`) | Not applicable: no connector | Do not add those connectors before Friday |
| A run past 64 GiB compressed on the server | The tail | Local only: `413` is retried for ever, said after 3 repeats (`run/mod.rs:167`) | Keep the cap (owner), but chain or heartbeat it so the server knows |

### 3.8 A WebSocket that lives for days

Codex (`chatgpt.com`) and Amp (`ampcode.com`) use WebSockets. Each message is its own stream
(`websocket.rs:10-17`), so one long connection is not a problem in itself.

| How it happens | What is lost | Record shows | Smallest guard |
|---|---|---|---|
| A context-takeover deflate stream loses history once (one bad frame) | Every later compressed message on that connection, possibly for days (`websocket.rs:41-44`) | Gap per message (`DecodeFailed`) | Keep raw bytes when inflate fails |
| The accept loop dies and restarts | The open WebSocket is aborted (`proxy/mod.rs:1036-1040`); the harness must reconnect | Gap (`ShutdownDeadline` label for a restart cut) | Do not drop the old loop's tasks on restart |
| A tunnel (an MCP server over `wss`) stays open for days | Nothing, but it holds a place of the 1,024 (`mod.rs:48-51`) | n/a | As 3.2 |
| A WebSocket message capture is refused once (queue full) | That message; capture resumes at the next message | Gap and an index hole | None |

### 3.9 A model call made by a tool the agent installed

| How it happens | What is lost | Record shows | Smallest guard |
|---|---|---|---|
| The agent runs `claude -p` inside a Claude Code run. It inherits `CLAUDE_CONFIG_DIR`, `HTTPS_PROXY` and the CA | Nothing: wire and transcript (`*/*.jsonl`) are both read | Recorded (inferred) | None |
| A Python or curl script calls an **approved** host (`api.anthropic.com` in a Claude Code run, `api.openai.com` in a Codex run). It honours `HTTPS_PROXY`, is intercepted, and does not trust the proxy CA, because only `NODE_EXTRA_CA_CERTS` / `CODEX_CA_CERTIFICATE` is set (`claude-code-cli/harness.toml:82-84`, `codex-cli/harness.toml:78-80`; `CHEAT-CHECKLIST.md:38-43` says no shipped connector names a shared CA row) | The script fails with a certificate error. The agent then tends to unset the proxy or turn off checks. Unset: the call is unrecorded | **Broken**, then silence | Hand `SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE` and `CURL_CA_BUNDLE` the system roots plus the proxy CA for every connector (the code exists: `ca_bundle.rs`, `phases.rs:531`). Cost: WP12 (a call that skips the proxy still verifies), which `extend` mode already has |
| A script calls a host that is not approved (another provider) | The whole call | Silence in the chain; a row in the tunnels file | Approve the hosts of every provider in `providers.toml`, not only the harness's |
| A Node script on Node below 22.21 / 24.5: `fetch` ignores `HTTPS_PROXY` even with `NODE_USE_ENV_PROXY` ([Node.js enterprise network guide](https://nodejs.org/learn/http/enterprise-network-configuration), [humanwhocodes](https://humanwhocodes.com/blog/2026/03/proxying-fetch-requests-server-side-javascript)) | The call | Silence, not even a tunnel row | None small. Accept and say it |
| `codex exec` inside a Claude Code run: no `CODEX_HOME`, `HOME` is the run home | It fails (no login), or, if logged in there, tunnels unread | Broken or silence | None |

### 3.10 A harness that ignores `HTTPS_PROXY`

| How it happens | What is lost | Record shows | Smallest guard |
|---|---|---|---|
| Node fetch on an old Node, a harness with its own proxy setting (OpenClaw), a gateway daemon started outside the run (Hermes 0.22, `CHEAT-CHECKLIST.md:50-63`), or QUIC/HTTP3 (UDP, never sees a proxy; no shipped harness is known to use it, unverified) | Every model call | **Silence.** A sealed run with no gap: the verifier exits 0 (ARCHITECTURE section 1, "may be unobserved without a gap"). The only warning is "no model calls recorded" after 10 minutes, local only, and only when the count is exactly zero (`run/mod.rs:257-258`, `:295-297`; `CHEAT-CHECKLIST.md:56-61`) | Send the model-call count in every heartbeat, and have the Worker flag a run that has native activity and zero wire calls after 30 minutes. A few lines. The wire count already exists (`proxy/mod.rs:404-418`) |

The two part files describe this case a little differently. The unrecordable half says the local
10-minute warning is the only warning. The cheat half points out that the verifier also sets
`analysis.cross_leg.state = native_activity_without_wire_streams` and `analysis.evidence =
native_only` (`report.rs:334-405`). Both are true: those labels exist, but they are hints that never
change the exit code (`report.rs:231-238`, `:328-333`), they appear only after the run, and they need
a native leg to compare against.

### 3.11 The Worker is down

| How it happens | What is lost | Record shows | Smallest guard |
|---|---|---|---|
| Down during the 24-hour start window | Nobody can start: the open is required, 3 tries, 3 s apart (`upload/mod.rs:10-11`, `:44-45`) | Nothing: no run exists | A local start that registers later. Not small. Better: deploy freeze and a status page for 7 Nov |
| Down mid-run | Nothing: every byte stays on disk, retried 1 to 15 minutes (`uploader.rs:9-35`, `policy.rs:79-90`) | Local only (`UPLOAD_FAILING_AFTER` 30 min, `run/mod.rs:199`) | None |
| Down mid-run on a small disk | Acknowledged parts cannot be pruned. Under 1 GiB free beyond a 3 GiB reserve, the harness is frozen with `SIGSTOP` (`run/pause.rs:1-11`; `reserve.rs:20-35`) | **Broken (paused)**. A paused process also loses its open model streams | Cut the pause and the reserve: the owner decided "disk full, no special handling" |
| Down for more than 24 hours after a deadline | The tail (`410`) | Gap on the server | Longer close window |
| A bad deploy answers `400`/`422` | Nothing; retried for ever (`uploader.rs:20-24`) | Local only | None |

## 4. Recording risk under the likely cuts

| Cut | What it removes | What can now go wrong | Guard to keep |
|---|---|---|---|
| **Thin proxy** (relay and tee raw plaintext, decode later) | Most of `http.rs` (2,257), `websocket.rs` (1,080), decoders in the hot path; the opaque fallback, head and decode limits | (1) Raw bytes still hold secrets in headers; compressed bodies cannot be redacted on the fly, so redaction must move after decompression in the daemon. (2) If the tee buffers without a bound, a burst means memory growth and an OOM kill of `goalcup start`, which is point B (harness blocked). (3) h2-only and pinning still break | A fixed memory bound that drops with a chained `Loss`, never one that waits. Never let a capture error touch the relay |
| **No caps** (streams, connections, files) | Points A and C, the 503 refusal, the 1,024-file cap | The real limit becomes file descriptors. On `EMFILE` the accept loop pauses 10 ms and retries (`mod.rs:106-108`), so the harness's connection waits in the backlog: slow, not refused. macOS starts at 256 (`commands/start/proxy.rs:29-31`). Native polls `stat` every tracked file every 100 ms, so 20,000 files cost CPU on the participant's machine (inferred) | Raise `RLIMIT_NOFILE` to the hard maximum (already done up to 2,304, `proxy.rs:27`, `:33-61`). Poll quiet files rarely. Keep the server byte cap: it guards a real cost |
| **Simpler upload** (fewer acknowledgements, no prune, or upload at the end) | `acks.jsonl`, `prune.rs`, part of `lifecycle.rs` | Upload only at the end: a stolen or dead laptop on day 6 loses six days. No prune: a week-long chain stays on disk; with the disk pause cut, a full disk ends with the daemon's `StorageFull` gap (`writer.rs:294`) and the harness's own writes failing, which the owner accepted. A slower cadence also widens the not-uploaded window that a cheater can rewrite (section 6.1) | Keep the 30-minute chunk cadence (RULES.md:332 says every 30 minutes or sooner). It is the cheapest insurance in the code, for recording and for tamper evidence |
| **Fewer refusals at start** | Preflight, profile refusals | A run that cannot record starts anyway and looks fine (section 3.10). The start preflight is TCP only today, so it never caught TLS inspection anyway | Replace start refusals with one early check during the run: "first model call recorded" in the first heartbeat, said plainly to the participant |
| **Drop the restart cap** (owner question, phase 2 notes item 8) | `MAX_RECORDER_RESTARTS` | A loop that dies at once restarts in a tight loop | Backoff, no cap |

Tension between the halves: the thin-proxy cut says raw bytes still carry secrets and redaction must
move into the daemon after decompression. The cheat half says decoders can wait until after Friday
"as long as the bytes are stored raw (minus secrets)". Both hold only if redaction after
decompression lands before Friday. If it does not, either the thin proxy waits or the raw-bytes plan
leaks secrets (Point 4).

## 5. Cheats and what exposes each one today

### 5.1 Five facts that decide almost every row

1. **The server copy is append-only and timed. It does not check the chain.** A chunk must start at
   the exact offset, `prev` and `first_seq` the server holds (`server/worker/src/chunks.ts:66-70`).
   R2 refuses bytes whose sha256 differs from the header (`chunks.ts:6`, `chunks.ts:197-200`). The
   server stamps each chunk with its own clock, `received_at` (`chunks.ts:81-93`,
   `server/worker/src/db.ts:144`). The `Goalcup-Head` it signs is the client's claim. No verify job
   exists: `crates/goalcup-verifier/src/lib.rs:1-2` ("its verify job is not built yet"),
   `docs/OPERATING.md:310`, `docs/SERVER-DESIGN.md:159`. So today the hash chain protects nothing on
   the server side. The append-only store plus `received_at` is what stops a rewrite of uploaded bytes.
2. **The chain has no secret.** Whoever controls the machine can rewrite the not-yet-uploaded part
   and recompute every hash (`docs/CHEAT-CHECKLIST.md:15-22`). So against a deliberate cheat the
   chain adds nothing the server copy does not already give. It catches accidents (torn tail, a bad
   disk) and lets a third party verify a local copy offline with signed acks
   (`crates/goalcup-protocol/src/ack.rs:1-9`, `crates/goalcup-verifier/src/capture_analysis/reader.rs:908-964`).
3. **The agent can write everything local except the wire bytes.** It can rewrite its own session
   file, read `control.json`, call the daemon's loopback API and write the shared config home
   (`docs/CHEAT-CHECKLIST.md:29-37`). The wire bytes come over TLS that the proxy checks against the
   real host (`docs/CHEAT-CHECKLIST.md:84-86`). So the **wire leg is the only leg an agent cannot
   forge**. The native leg is useful for authorship markers, not as tamper evidence. This is also why
   the silent-bypass cases in 3.2, 3.9 and 3.10 matter most: they remove the one unforgeable leg.
4. **Judging happens after the run, from data on the server.** Judges get the prompt, disclosed
   context and the repository at the judged commit (`docs/RULES.md:448-450`). A human makes every
   disqualification decision (`docs/RULES.md:446-447`). So any decoder that only reads stored bytes
   can be written after Friday 6 Nov, as long as the bytes are stored raw (minus secrets).
5. **The verifier's cheat-related outputs are hints, not checks.** `assistance`,
   `analysis.evidence` and `analysis.cross_leg` never change the exit code
   (`crates/goalcup-verifier/src/capture_analysis/report.rs:231-238`, `report.rs:328-333`). The labels
   are "a hint for judges" (`docs/RULES.md:314`).

### 5.2 The cheat list

"Rule" is where RULES.md or CHEAT-CHECKLIST.md names it. "Today's evidence" gives the record data and
the code that reads or writes it.

| # | Cheat | Rule | Today's evidence (path:line) |
|---|---|---|---|
| C1 | Human types something other than "keep going" after the prompt (incl. `/model`, a permission answer, a key, a login) | RULES.md:98-105, 239-244 | Native transcript user turns with a typed marker: Claude `promptSource == "typed"` or `origin.kind == "human"` (`crates/goalcup-verifier/src/capture_analysis/native/claude_code.rs:41-90`); Codex `content_item_kinds` holds `user.text` (`native/codex.rs:205-253`). Counted as keep_going / other (`capture_analysis/stats.rs:636-643`, `844-860`, `978-1000`); `assistance.evidence` (`report.rs:231-238`, `279-283`). Wire: the same turn appears as a new user message in the next request body (no wire decoder classifies authorship: `capture_analysis/anthropic.rs:7`). Hermes and Amp: no native decoder, wire only. |
| C2 | Side-channel prompting: a person edits a file, issue, doc or env the agent reads; or sends through Codex thread tools, OpenCode `POST /session/:id/shell`, Hermes gateway/cron, Claude Remote Control | RULES.md:226-237 | A message that reaches the model shows as a user turn or tool result in the wire request body. A Codex/Claude native turn without the typed marker is listed as unmarked (`native/codex.rs:214-216`). External state changes are not in the record at all; only the agent's reads of them (tool results) are. |
| C3 | Seeded workspace or smuggled product (non-empty folder, repo with commits, product hidden in skills/memory/MCP) | RULES.md:136-160 | Prevented at start: folder must hold only `.gitignore`, `.DS_Store`, `.env*`, checked before and after approval (`crates/goalcup-cli/src/commands/start/project.rs:1-12`, `32-40`). Server checks the repo has no commit (`server/worker/src/runs.ts:336-347`, called at `runs.ts:232-236`). Baseline manifest of workspace and harness home in the chain at seq 0 (`reader.rs:984-1020`, `crates/goalcup-protocol/src/seal.rs:58-85` field `home`). Product hidden in context shows as text in the first request body and as tool reads. |
| C4 | Undisclosed prepared context (instruction files, skills, memory, MCP) | RULES.md:95-97, 161-174 | First request body of each conversation (system prompt, injected files, skill lists). Baseline home listing (names, sizes, digests: `seal.rs:72-75`). |
| C5 | Second harness (a sub-harness) or an outside AI agent doing the work | RULES.md:80-84, 394-398 | The tool call that starts it (`claude ...`) in the wire and native records. Its traffic, if it honours the inherited `HTTPS_PROXY`, is a tunnel to a host not approved: one row in `tunnels-<entry>.jsonl` with host, port, bytes and times (`crates/goalcup-cli/src/proxy/tunnels.rs:20-23`, `99-130`), uploaded beside the seal, outside the chain (`server/worker/src/runs.ts:546-548`). The verifier has **no** second-harness check, despite RULES.md:83-84 ("the verifier reports it"). An agent on another machine leaves only its commits on GitHub. Note 3.9: a nested `claude -p` in a Claude Code run is recorded on both legs (inferred). |
| C6 | Routing model work around the proxy (a tool that ignores `HTTPS_PROXY`, a gateway in workspace `.claude/settings.json`, a Codex profile provider, Bedrock, local ollama, a Hermes gateway started outside the run) | RULES.md:313, 378-379; CHEAT-CHECKLIST.md:38-63, 130-144 | `analysis.cross_leg.state = native_activity_without_wire_streams` and `analysis.evidence = native_only` (`report.rs:334-405`). Tunnel list rows for the gateway host (above). Local ollama: nothing (`NO_PROXY`, CHEAT-CHECKLIST.md:141-143). Hermes outside gateway: nothing (CHEAT-CHECKLIST.md:49-63). A `*_PROXY` key in a settings `env` block leaves no tunnel row either (3.2). |
| C7 | Capture-off flags and cloud or remote modes (`--ephemeral`, `--no-session-persistence`, `--pure`, Cursor `-p`, cloud handoff) | RULES.md:216-225 | Launch record holds `profile_id`, `harness_version`, `continuation` (`crates/goalcup-protocol/src/capture.rs:301-330`). The argv flags themselves are not a field found (unverified). Effects: no native file (evidence `wire_only`), or no wire (`none`). A cloud handoff shows as a tool or slash command in the transcript, then silence. |
| C8 | Tampering with the record: edit, truncate or fabricate the local trace; stay offline and upload a fabricated week late; agent writes junk with the run credential | CHEAT-CHECKLIST.md:15-28 | Chain link checks (`reader.rs:762-781`), receipt binds connector and head (`reader.rs:803-835`), signed acks with server time (`ack.rs:36-50`, `reader.rs:908-964`), deadline facts (`reader.rs:873-881`). Server side: append-only offsets (`chunks.ts:66-70`), `received_at` per chunk (`chunks.ts:81-93`), heartbeat silence notices (`server/worker/src/notices.ts:155-165`). The run credential uploads to one run only (CHEAT-CHECKLIST.md:23-28). |
| C9 | A person writes code, commits or pushes; or another account pushes | RULES.md:232-233, 416-422 | Server records the judged commit from GitHub's activity list, with `late_push` and `force_push` flags (`server/worker/src/github.ts:169-222`, flags at `github.ts:199-203`). The record holds the agent's own `git commit`/`git push` tool calls with times. Nothing compares the two automatically. The activity actor is not stored (`github.ts:75-100` keeps after, before, at, force only). |
| C10 | Work after the deadline, history rewrite | RULES.md:416-422 | Server judged commit by GitHub timestamp, `late_push`, `force_push` (`github.ts:169-222`). Records stamped after deadline counted as a hint (`reader.rs:873-881`). |
| C11 | Start outside the window, longer than 168 h, many runs and keep the best, seed one run from another | RULES.md:44-52, 79, 85 | Server: `official_start` from its own clock, deadline from config (`runs.ts:242-262`); database refuses outside the window (`server/worker/src/db.ts:179-184`). Replacement is recorded (`runs.ts:130-135`; the heartbeat answers `replaced`, 3.1). Seeding: only by comparing repositories. |
| C12 | Fake or wrapped harness, or an unpinned build that drops the proxy | CHEAT-CHECKLIST.md:92-103 | `Launch.harness_build` program and image sha256, observed by the daemon after exec (`capture.rs:348-367`, `crates/goalcup-daemon/src/runtime/capture/mod.rs:761-776`); `connector_dir_sha256` checked against the receipt (`reader.rs:812-835`); `harness_version` is self-reported (`crates/goalcup-cli/src/harness_version.rs:1-8`). |
| C13 | Harness updated mid-run | RULES.md:213-215 (a flag, not a DQ) | Version probed once per launch (`crates/goalcup-cli/src/commands/start/phases.rs:273-281`). No in-run sampling found, despite RULES.md:214-215 ("samples the version, records a change as an event"). A change shows only between segments (a new Launch record per `continue`; continue path not fully traced, unverified). Section 3.3 lists what an update can silently break. |
| C14 | Modified Goal Cup Proxy (fabricates or drops wire bytes) | RULES.md:377-379 | Nothing proves the recorder build. `tracer_version` is self-reported and only compared with a minimum (`runs.ts:228`, `server/worker/src/device.ts:55`). |
| C15 | Disk filled on purpose to hide work | RULES.md:319-323 | `Gap { reason: StorageFull }` (`capture.rs:191-202`, `295-300`). Owner decision: no special handling, harness stops. |
| C16 | Agent stops its own recording | RULES.md:114-118 | Prevented: `goalcup seal` refuses inside the harness (`crates/goalcup-cli/src/commands/seal.rs:44-56`). The control API is still reachable (CHEAT-CHECKLIST.md:29-37). Killing or stopping the daemon leaves one `IngestLost` gap, then silence (3.2). |
| C17 | Changing online services or artifacts after the deadline | RULES.md:436-437 | Outside the record entirely. |

Note: `CaptureRecord::Assistance` exists in the schema and the reader (`capture.rs:332-335`,
`reader.rs:1493`) but no daemon or CLI code writes it (only `writer.rs:94`, `387` match it, as a
pattern). So `assistance.explicit_records` is always 0 today.

### 5.3 Does each cheat stay exposed after each cut?

Key: **Same** = exposure unchanged. **Weaker** = still visible, less convenient or less provable.
**Lost** = no longer visible. **Better** = more visible. "n/a" = the cut never touched it.

| # | Cut seal or hash chain | Cut research decoders | Cut device approval | Cut termination confirm | Cut byte-for-byte check | Cut email masking |
|---|---|---|---|---|---|---|
| C1 extra human message | Same (bytes still on the server) | Weaker: no automatic count; a person must find user turns in raw bytes, or a small extractor written later | n/a | n/a | n/a | n/a |
| C2 side channel | Same | Weaker (unmarked-turn list gone) | n/a | n/a | n/a | n/a |
| C3 seeded product | Weaker only if the baseline goes too; start check and empty-repo check remain | Same (humans read it) | Weaker: the empty-repo check runs on open; it needs an authenticated open, not a browser click | n/a | n/a | n/a |
| C4 undisclosed context | Weaker only if the baseline home listing goes; the first request body remains | Same | n/a | n/a | n/a | n/a |
| C5 second harness or outside agent | Same: the tunnel list was never in the chain | Same: no decoder checks it today | n/a | Same: a leftover second harness after the stop has no proxy to call | n/a | Better: commit author emails in `git log` output stop being masked |
| C6 bypass | Same | Weaker: the `native_only` and `cross_leg` labels go; the tunnel list remains | n/a | n/a | n/a | n/a |
| C7 capture-off flags | Same | Weaker (evidence label gone) | n/a | n/a | n/a | n/a |
| C8 tampering | Weaker only for offline, third-party proof and accidental damage. Deliberate rewrite of the not-uploaded window is open today too. Uploaded bytes stay protected by append-only offsets and `received_at` | Same | Weaker if the run credential becomes a reusable login credential (see 6.3) | n/a | n/a | n/a |
| C9 human pushes | Same | Same | Weaker: the member binding must survive in some form | n/a | n/a | Better (author emails visible) |
| C10 late work | Same: judged on the server from GitHub | Same | n/a | Same: a process left running cannot change the judged commit | n/a | n/a |
| C11 window, replays | Same (server clock) | Same | Weaker: one browser approval per run is the human-presence step; without any account binding, Lost | n/a | n/a | n/a |
| C12 fake harness | Same if the build digests move to a plain start record | Same | n/a | n/a | n/a | n/a |
| C13 harness update | Same | Same | n/a | n/a | n/a | n/a |
| C14 modified proxy | Not exposed before or after | Not exposed | Not exposed | n/a | n/a | n/a |
| C15 disk full | Same if gap notes stay | Same | n/a | n/a | n/a | n/a |
| C16 agent stops recording | Same | Same | n/a | Same (the in-harness refusal is separate, `seal.rs:44-56`) | n/a | n/a |
| C17 post-deadline service change | Not in the record before or after | | | | | |

## 6. Why each cut is safe or not, cheat-wise

### 6.1 Seal and hash chain

- What the chain gives today: offline proof of a local copy, catching accidental damage, binding
  acks to positions (`reader.rs:762-781`, `908-964`). What it does not give: protection against the
  participant in the not-uploaded window (`docs/CHEAT-CHECKLIST.md:15-22`). The server does not
  check it (`lib.rs:1-2`).
- What the seal gives: an end-of-run workspace listing or Git commit plus dirty digest
  (`seal.rs:58-85`, `reader.rs:1074-1110`) and the termination outcome (`capture.rs:336-341`). The
  judged commit comes from GitHub on the server (`github.ts:169-222`), so the seal's commit is a
  local claim that judges do not need.
- **Verdict for cheats:** cut is safe if the server keeps append-only offsets, R2 sha256 per chunk
  and `received_at` per chunk. That is the real anti-tamper fact today.
- **What could go wrong:** (a) a run whose server copy is lost leaves only a local copy that no one
  can prove; (b) the not-uploaded window grows if upload cadence slows; keep uploads frequent
  (RULES.md:332 says every 30 minutes or sooner); (c) offline research users lose a self-checking
  format. (d) CHEAT-CHECKLIST.md:27-28 claims an hourly verify job that does not exist; fix that text
  whatever is decided.

### 6.2 Research decoders

- They turn bytes into hints: human-message counts, `cross_leg`, `evidence`, models
  (`report.rs:231-405`, `stats.rs:636-1000`, `native/claude_code.rs`, `native/codex.rs`). About
  7,125 src lines in `anthropic.rs`, `openai.rs`, `chat.rs`, `wire.rs`, `thread.rs`, `tracker.rs`,
  `lineage.rs` and `native/*` alone (wc -l, src). `reader.rs` (2,070 src lines) is the integrity
  reader, not a decoder.
- **Verdict for cheats:** safe to cut before Friday, because no cheat depends on a decoder running
  during the run. Every input stays on the server. Write a small extractor before judging, not
  before the event.
- **What could go wrong:** (a) with a week and thousands of subagents, nobody reads raw bytes by
  hand, so C1 and C6 go unseen unless the extractor exists by judging time; (b) the native
  authorship markers are harness-specific (`promptSource`, `content_item_kinds`) and change between
  versions; record the native files raw so a later extractor can still read them; (c) without a
  wire decoder, the "model calls happened outside the proxy" signal becomes "compare the transcript
  with the tunnel list by eye". The heartbeat model-call count and Worker flag from 3.10 would
  replace part of (c) at almost no cost.

### 6.3 Device approval

- Two approvals today: `goalcup login` once, then one per run (`crates/goalcup-cli/src/account/device.rs:1-8`,
  `server/worker/src/device.ts:1-10`). The run approval mints a credential for that run alone
  (CHEAT-CHECKLIST.md:23-28).
- Cheat value: it binds a run to a member (needed for C9, C11 and for Point 5's "by ID"), it lets the
  server apply the start window, the empty-repo check and the competition version at open
  (`runs.ts:211-262`), and it limits what an agent that reads the credential can do: upload to this
  one run.
- **Verdict:** some member binding is required; the second browser click per run is not a cheat
  control. Cutting the per-run click is acceptable if the run credential stays scoped to one run.
- **What could go wrong:** if the login credential itself can open runs, an agent that reads it
  (it runs as the participant) can open a new run and replace the participant's run
  (`runs.ts:130-135`). That is sabotage more than cheating, but it is a real hole. Keep the open
  behind a human step or keep the run credential separate from anything stored where the agent can
  read it. Also note from 3.11: the open is required to start at all, so the Worker must be up during
  the start window whatever approval flow is kept.

### 6.4 Termination confirm

- Read two ways. (a) The `Confirmed` proof machinery (`crates/goalcup-daemon/src/terminate/`, 2,498
  src lines; `runtime/termination.rs`, 285; `goalcup-cli/src/subreaper.rs`, 617;
  `released_child.rs`, 291). (b) The "ask first" prompt of `goalcup seal` (`seal.rs:118`).
- Cheat value: none found. A process left after the stop has no live proxy, so its model calls
  fail or bypass (that is C6, which the termination outcome never showed). Late pushes are judged by
  GitHub's timestamp on the server (`github.ts:169-222`). TERMINATION.md itself says `Confirmed`
  claims nothing outside the owned set (TERMINATION.md:16-19, 26, 189-195).
- **Verdict:** safe to cut for cheats. Keep a plain stop (SIGTERM to the group, grace, SIGKILL) and
  one line noting the result. Keep the in-harness refusal (`seal.rs:44-56`); it is about 10 lines.
- **What could go wrong:** a leftover process (Hermes tools run in their own sessions,
  TERMINATION.md:96-100) keeps using CPU or the network after the run on the participant's
  machine; a nuisance, not a cheat. The ordering matters for recording too: point B says the proxy
  should be the last thing to die while the harness lives, so a plain stop should end the harness
  group before the proxy. No measured Sandbox data exists (the 2-hour run report is missing).

### 6.5 Byte-for-byte check

- It was a test-time acceptance check that each subagent transcript on disk equals its chain copy
  (`docs/plans/2026-10-29-record-everything-plan.md:307`; failure noted at
  `docs/plans/2026-10-29-record-everything-progress.md:72`). No code for it was found in this repo
  (unverified; likely the external run kit). `scripts/matrix.sh:2208-2235` is a different check:
  local chain parts equal the server copy (an upload test, Point 5).
- Cheat value: none. The agent can rewrite the disk file anyway (fact 3), so equality with disk
  never proved honesty.
- **Verdict:** cut, as the owner decided. A cheaper check that does matter for recording is the
  "N subagent files read" line at finish from 3.3.

### 6.6 Email masking

- The `email` rule (`crates/goalcup-daemon/src/redact.rs:280-287`). It also covers URL passwords by
  accident (`docs/plans/2026-10-29-record-everything-phase2-notes.md:440`).
- Cheat value: negative. Masking hides commit author emails and outside-account emails in tool
  output, which are the clues for C5 and C9. Cutting it makes those cheats a little easier to see.
- **Verdict:** cut, after the URL userinfo rule lands (owner decision b).
- **What could go wrong:** participant and third-party emails sit in private raw traces; the
  research export must still clean them before anyone else gets them (RULES.md:369-374).

## 7. Cheats the record must still show after the cuts, and the minimum data

This table merges the cheat half's minimum-data list with the recording risks from sections 1 to 4
that would hide the same cheat. "Threat after cuts" names the cut or the run-ender that most weakens
it. C14 and C17 are not caught before or after (section 8), so they have no row.

| Cheat | Minimum data the record must keep | Where it lives after the cuts | Threat after cuts | Guard that keeps it visible |
|---|---|---|---|---|
| C1 extra human message | Every request body to the model service (user turns), and the harness's native session files raw (secrets removed only), with capture times | Uploaded wire and native streams | Decoder cut (no count); point A (wire off); point C (subagent transcripts unread) | Small extractor by judging time; fix points A and C |
| C2 side channel | Same as C1, plus tool results (what the agent read) | Wire request bodies | Point A; silent bypass (3.10) | Fix point A; heartbeat model-call count |
| C3 seeded product | Server empty-repo check at open; start folder check; first request body; tool reads | Worker `runs.ts:336-347`; CLI `project.rs`; wire | Device-approval cut if the open is no longer authenticated; seal cut if the baseline goes | Keep an authenticated open; keep the start folder check |
| C4 hidden context | First request body of each conversation; optionally a start listing of the harness home (names, sizes, sha256) as a plain uploaded file | Wire; optional start file | Seal cut drops the baseline home listing | Optional plain start file |
| C5 second harness or outside agent | Tool calls with times; the tunnel list (host, port, bytes, times); GitHub push list | Wire; `tunnels-<entry>.jsonl`; server GitHub read | Tunnel list is written only at finish (`tunnels.rs:1-8`); a dead `goalcup start` (point B) may never write it | Keep the tunnel list upload; store GitHub push list with actor |
| C6 bypass | Tunnel list; native session files (model turns the wire lacks); model-call count per heartbeat (new) | Same, plus heartbeat | Decoder cut removes `native_only` / `cross_leg`; settings `env` proxy override leaves no tunnel row (3.2) | Heartbeat count + Worker flag (3.10); refuse `*_PROXY` keys in settings `env` |
| C7 capture-off flags | Launch facts: connector id, harness version line, argv if available; native files present or not | One start record (moved out of the chain) | Seal or chain cut, if the Launch record is not moved | Plain start record with argv (argv unverified today) |
| C8 tampering | Append-only server store with sha256 per chunk and `received_at` per chunk; heartbeats with server time | Worker `chunks.ts`, `db.ts:144`, `notices.ts` | Slower upload cadence widens the rewritable window | Keep the 30-minute cadence; keep append-only offsets and R2 sha256 |
| C9 human pushes | GitHub push activity for the run window (sha, time, force, ideally actor) and the agent's recorded `git push` calls | Server GitHub read (today only the judged result and two flags are kept) | Device-approval cut without member binding | Keep member binding; store the push list (`github.ts:75-100` already reads it) |
| C10 late work | GitHub timestamps of pushes; deadline by server clock | Worker `github.ts`, `runs.ts` | None found | None needed |
| C11 window, replays | Member id, `official_start`, `deadline_at`, replacement time | Worker database | Device-approval cut without any account binding (Lost) | Keep member binding and the server clock |
| C12 fake harness | Program and image sha256 of the launched harness | One start record | Seal or chain cut, if the digests are not moved | Move `harness_build` to the start record |
| C13 harness update | Harness version per launch and per `continue` | One start record per segment | Only one probe per launch today; updates can also silently move files (3.3) | "N subagent files read" line at finish |
| C15 disk full | A gap note with time when a write fails | Gap record, kept | Cutting the disk pause and reserve (owner accepted) | Keep the `StorageFull` gap (`writer.rs:294`) |
| C16 agent stops recording | The in-harness seal refusal (prevention); an `IngestLost` gap when the daemon dies | CLI `seal.rs:44-56`; gap record | Termination cut, if the refusal goes with it | Keep the about 10-line refusal |

Two cheap adds worth an owner look (each is a few lines, not required by the seven points): store
the full GitHub push list for the run window, with the actor (`github.ts:75-100` already reads it and
drops it); and keep the harness argv in the start record if it is not already there (unverified).

### 7.1 Cheats humans judge from the record anyway (no code needed)

Each of these ends in a human decision (RULES.md:446-447). Code only helps find the spot.

- **C1 extra human messages:** whether a message is "keep going", who said it, and whether it
  steered the work. RULES.md:108-109 already says the detection is not proof and an organizer
  reviews. A grep over stored user turns is enough to find them.
- **C2 side-channel prompting:** no code can see a changed Linear issue or document. A human
  compares what the agent read with what the participant controls.
- **C3 and C4 smuggled product or hidden context:** a human reads the first request and the
  disclosed context and decides if it is "a prebuilt product" (RULES.md:159-160).
- **C5 outside agent or second harness:** a human reads the tool calls and the tunnel list and
  compares GitHub commits with the agent's recorded pushes.
- **C6 intent of a bypass:** whether routing around the proxy was deliberate is a judgement
  (RULES.md:313, 378-379).
- **C9 human code edits:** commits on GitHub that no recorded tool call explains.
- **C13 harness update:** already "a flag, not a disqualification; an organizer looks"
  (RULES.md:214-215).
- **C15 disk full:** "Judges decide each such gap case by case" (RULES.md:322-323).
- **C11 seeding one run from another:** a human compares repositories.
- **C17 post-deadline service changes:** outside the record; humans only.

The cheats that need code at event time, all on the server or at start, and all small: the
empty-repo check, the start-window and deadline clock, the judged commit with late and force flags,
the start folder check, the append-only chunk store with `received_at`, the tunnel list, and the
in-harness seal refusal. None of them is the chain, the seal, a decoder, the termination proof, the
byte-for-byte check or the email rule.

## 8. Cheats the record does not catch, before or after the cuts

- **A modified Goal Cup Proxy.** `tracer_version` is self-reported (`runs.ts:228`). A rebuilt proxy
  can drop or invent wire bytes. Only provider-side request ids could show it, and organizers have
  no access to them (unverified that any provider would confirm).
- **Model calls from a tool that ignores `HTTPS_PROXY`** (`ssh`, raw sockets, a command that unsets
  it, Node `fetch` below 22.21 / 24.5, QUIC) and **local ollama** under `NO_PROXY`
  (RULES.md:352-357, CHEAT-CHECKLIST.md:141-143; section 3.9, 3.10). Only the effects show: code
  with no model call behind it.
- **A Hermes gateway started outside the run** (CHEAT-CHECKLIST.md:49-63).
- **Work by a person on another machine** that is pushed under the agent's token or an SSH key the
  agent also uses (CHEAT-CHECKLIST.md:158-174). Only GitHub's push list shows it, and then only as
  a push without a matching recorded tool call.

## 9. Doc and code mismatches found (fix the text whatever is cut)

| Claim | Reality |
|---|---|
| CHEAT-CHECKLIST.md:27-28: "a verify job re-runs goalcup-verify over the stored chain every hour and at seal" | Not built: `crates/goalcup-verifier/src/lib.rs:1-2`, `docs/OPERATING.md:310`, `docs/SERVER-DESIGN.md:159` |
| RULES.md:83-84: a second harness's traffic "the verifier reports it" | No verifier check names a second harness; the only trace is the tunnel list outside the chain and the tool call |
| RULES.md:214-215: Goal Cup Proxy "samples the version, records a change as an event" | One probe per launch (`phases.rs:273-281`); no in-run sampling found |
| `assistance.explicit_records` in every research summary (`report.rs:231`, `279-283`) | No code writes `CaptureRecord::Assistance`; always 0 |

## 10. Ranked guards for Friday (smallest first, by risk removed)

1. `MAX_OPEN_STREAMS` exhaustion is recoverable, or the bound is far higher (point A). About 1 line plus a test.
2. Proxy runtime gets one worker per core (`commands/start/proxy.rs:98`). 1 line.
3. Upstream roots add the system store (3.4, 3.5). A few lines; `rustls-native-certs` or the platform verifier. Risk: a participant's own bad root is trusted; the harness already trusts it.
4. No restart cap, with backoff; `goalcup start` ignores `SIGHUP` while the harness lives (point B). Risk: an orphaned `goalcup` if the harness also ignores it; the daemon's termination still ends it.
5. Native: close quiet caught-up streams, raise the file cap (point C; phase 2 Note 2). Risk: repeated prefixes in the chain; dedupe on the server.
6. Hand the shared CA variables to every connector (3.9). Risk: WP12, already true for `extend`.
7. Model-call count in the heartbeat, and a Worker flag for "native activity, zero wire calls" (3.10). Risk: false alarms for a harness with no native leg; flag only when native is non-zero. Also restores much of the C6 signal lost with the decoder cut.
8. Cut the `SIGSTOP` disk pause and the run reserve (3.11). Removes `run/pause.rs` (360) and most of `run/reserve.rs` (956). Risk: a full disk now stops the harness by itself, which the owner accepted.
9. Raise `MAX_HEAD_BYTES`, keep raw bytes on decode failure (3.3, 3.4, 3.8).
10. Say "run in tmux or screen" and "upload within 24 hours of the deadline" on the start screen.

For cheats, keep at event time (section 7.1): the empty-repo check, the start-window and deadline
clock, the judged commit with late and force flags, the start folder check, the append-only chunk
store with `received_at`, the tunnel list, the in-harness seal refusal and some member binding.

## 11. What could go wrong with this analysis

- Point A's chain form at finish is inferred. How a `Disabled` emitter closes its producer was not
  traced, so "loss at finish" could be an `IngestLost` gap, a `ProducerLost` gap or open streams
  counted as unknown tail loss. Each is a gap at the end; none is a gap at the moment.
- Whether the daemon-side bound (shared 128) or the proxy-side bound (128) trips first depends on
  how many native streams are open. With Claude Code the main transcript is always open, so the
  daemon side usually trips first (recoverable). With Hermes or Codex with few files, the proxy
  side trips first (terminal).
- The Claude Code settings `env` override of `HTTPS_PROXY` is unverified.
- Throughput and CPU figures are read from constants, not measured.
- Whether the Launch record holds argv, and how `continue` records a new version, are unverified.
- The byte-for-byte check's code was not found in this repo (likely the external run kit, unverified).
- None of this is checked against the 2-hour run report, which does not exist yet.
