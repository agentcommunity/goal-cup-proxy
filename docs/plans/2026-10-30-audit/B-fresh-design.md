# Appendix B: Goal Cup Proxy designed fresh (Lane B)

Repo: /home/user/goal-cup-proxy, commit 4b226de2 (same as origin/main). Read-only work of 29 Oct 2026.
Nothing was built, run or tested. This appendix merges two independent fresh designs, B1 (lens: least
code) and B2 (lens: survive a week on a real laptop), and then compares both with the inventory of the
code that exists (Appendix A, `docs/plans/2026-10-30-audit/A-inventory.md`).

**Lane summary.** Two designers who read only the brief, `VISION.md` and `docs/RULES.md` (not the code)
each designed Goal Cup Proxy from a blank page. They arrived at almost the same shape: one recording
HTTPS proxy set through the harness's own environment (`HTTPS_PROXY` plus a run CA in the harness's CA
variable), decrypting only model hosts and logging every other connection as host, port, bytes and
times; a poller that walks the harness's session folder and opens, reads and closes each file, so there
is no subagent code and no cap; one masking function applied in memory before anything touches disk;
immutable compressed segments uploaded by idempotent `PUT`; a plain status screen; and a connector that
is 15 to 20 lines of data. Both estimate about 2,000 client source lines and about 2,500 to 3,800
source lines in all, against about 102,000 production source lines today (Appendix A, section A.2).
They differ on whether the proxy may change two headers (B1 yes, B2 never), on one process (B1) or a
supervisor plus a detached recorder (B2), on gzip or zstd, and on how session files travel. Compared
with the code, roughly 26,000 existing source lines sit in areas neither design has (separate-daemon
plumbing, termination proof, disk handling, research decoders, connector pins, event-rule commands).
The code also has some parts both designs skipped that do serve a point (terminal sharing for chat-mode
harnesses, reading the model host from the harness's own config, no-follow reads of session files).
Both designs add things the code lacks: a shared list of model hosts across providers, no caps of our
own, joined-view masking of secrets split across streamed deltas, masking of workspace `.env` values,
periodic SQLite snapshots during the run, and a sleep gap. Both designers warn that a rewrite in eight
days is the largest risk; B1 advises cutting the existing code toward this shape instead.

## Top findings

1. **Two independent designs converge.** Both chose an environment-set recording proxy plus a
   session-folder poller, with no per-harness client code and no caps (B1 sections 2 to 3, B2 sections
   3 to 4). That agreement is the strongest signal in this lane that the core of the current product
   (proxy in the CLI, native file capture) is the right core.
2. **Fresh size: about 2,000 client source lines.** B1: about 2,040 src and 1,320 test lines for the
   client, about 2,550 / 1,620 in all, about 3,800 / 2,400 after an honest 1.5 times. B2: about 2,070 /
   1,980 for the client, about 2,570 / 2,430 for the capture system plus about 400 src for run
   lifecycle on the server, plus or minus 50 percent. Both are estimates, not measurements.
3. **No subagent code is needed.** In-process subagents share the parent's network stack and
   child-process subagents inherit its environment, so every subagent call crosses the same proxy.
   Session files are just more files. Open, read, close per poll removes the file-handle limit, so the
   owner's "raise or remove 128, 32 and 1024" becomes "delete them".
4. **The main disagreement is header changes.** B1 forces `Accept-Encoding: gzip` and strips
   `Sec-WebSocket-Extensions` on model hosts to drop the brotli, zstd and `permessage-deflate` code.
   B2 never changes a forwarded byte and decodes all four encodings plus `permessage-deflate`. The
   existing code does what B2 does (`crates/goalcup-cli/src/proxy/http.rs`, `proxy/websocket.rs`).
5. **The main point 4 gain is joined-view masking.** Both designs mask a secret split across many
   streamed deltas by joining the delta text with a map back to raw byte ranges (about 80 lines in
   B2). `docs/RULES.md:289-298` says this is missing today.
6. **Neither design has a hash chain, signed acks, signed receipts or signed status.** Integrity is a
   sha256 per segment or pack plus idempotent `PUT`. Today about 1,000 lines plus the verifier reader
   serve tamper evidence, which is not one of the seven points (Appendix A, section A.9).
7. **Neither design has termination proof, a daemon control API, disk handling or research decoders.**
   A plain `killpg(SIGTERM)`, a grace period and `killpg(SIGKILL)` end the run in both.
8. **Both designs skipped parts that do serve a point.** Terminal sharing and host logging
   (`crates/goalcup-platform/src/terminal.rs`, 292 src; `crates/goalcup-cli/src/host_log.rs`, 253),
   reading the model host from the harness's own config (`crates/goalcup-cli/src/model_host.rs`, 893
   src) and no-follow session reads (`sourcecap`, 1,285 src) protect points 3, 1 and 4. A cut must keep
   small versions of them.
9. **Open harness questions are the same in both designs.** Bun (OpenCode) and the CA variable, Amp's
   proxy support, Hermes's `trust_env`, and Claude Code's background-agent supervisor are all
   **unverified** and must be measured before 6 November.
10. **Rewrite risk.** Both designs are in Go; the code is in Rust. Both designers say the line counts
    are the easy part. B1: "use this as the target shape and cut the existing code toward it, not to
    start a new repository" (B1 section 12).
11. **The 2-hour run report (`docs/research/results/2026-10-30-two-hour-run.md`) does not exist** on
    main or any of the 89 remote branches, per the brief. B2 could not use it for compression ratios
    (B2 section 15). Nothing in this appendix relies on it.

## B.0 How this was made

- Designers B1 and B2 read only the scratchpad brief, `VISION.md` and `docs/RULES.md`. They did not
  read the code, connectors, scripts or other docs, on purpose. Harness facts come from the web and
  are cited. Anything they could not check is marked **unverified**.
- The writer of this appendix merged the two parts, removed duplicates (constraints, harness evidence,
  sources), kept every citation, table row and number, and stated both views where the parts differ.
- The comparison (section B.6) reads Appendix A. Line counts there are Appendix A's, which split
  production src from inline tests.

### Line-count spot checks (wc -l, 29 Oct)

| File or set | Kind | Stated | wc -l | Result |
|---|---|---:|---:|---|
| `VISION.md` | doc | 44 | 44 | correct |
| `docs/RULES.md` | doc | 489 | 489 | correct |
| `crates/*/src` (all files) | src incl. inline tests | about 103k (brief) | 102,831 | correct |
| `crates/*/tests` (all files) | tests incl. fixtures | about 126k (brief) | 133,051 | brief counts 126,325 `.rs` and shell lines; the extra 6,726 are fixtures (Appendix A, A.1). Not wrong, but say "126k plus 6.7k fixtures" |
| `server/worker/src/*.ts` | src | 4,001 | 4,001 | correct |
| `server/worker/test` `.ts` | tests | 5,749 | 5,749 | correct |
| `scripts/` all files | src + tests + data | about 22k (brief) | 21,878 | correct; 10,393 of it is `test_*.py` tests |
| `crates/goalcup-cli/src/proxy/websocket.rs` | src + inline tests | 1,080 | 1,080 | correct |
| `crates/goalcup-daemon/src/redact.rs` | src + inline tests | 1,208 + 71 | 1,279 | correct |
| `crates/goalcup-daemon/src/redact/stream.rs` | src + inline tests | 2,945 + 1,063 | 4,008 | correct |
| `crates/goalcup-platform/src/terminal.rs` | src + inline tests | 292 + 190 | 482 | correct |
| `crates/goalcup-cli/src/model_host.rs` | src + inline tests | 893 + 647 | 1,540 | correct |
| **B1's "about 142k source lines the brief counts today"** | mixed | 142k | see note | **wrong label, fixed below.** 103k + 17k + 22k includes the Worker's tests, SQL and lock files and the scripts' 10,393 test lines. Production src is about 102,406 (Appendix A, A.2). B1's 3,800 lines is about 3.7 percent of that, not "under 3 percent". |

---

## B.1 Both designs at a glance

| Aspect | Design 1 (B1, least code) | Design 2 (B2, survives the week) |
|---|---|---|
| Processes | One binary, one process per part: proxy, watcher, recorder, uploader, and parent of the harness. No daemon, no socket. | Foreground `goalcup start` is a tiny supervisor; a detached `goalcup recorder` (nice +10) holds proxy, watcher, uploader and status; harness is a child of the supervisor. |
| Model-call capture | `HTTPS_PROXY` + run CA in the harness's CA variable; decrypt known model hosts; tunnel the rest | Same |
| Which hosts are decrypted | One list in the binary: union of model API hosts across connectors plus cloud wildcards; setup adds custom hosts | Connector suffixes + shared list of about 30 provider suffixes + hosts the participant names at setup; suffix match |
| Forwarded bytes | Two disclosed changes on decrypted hosts: `Accept-Encoding: gzip`, strip `Sec-WebSocket-Extensions` | Never changed: not a header, not a retry, not a timeout |
| Inner protocol | ALPN `http/1.1` only | ALPN `http/1.1` only |
| Body encodings | gzip only (request bodies decoded if gzip); other: headers stored, body not, gap | gzip, deflate, br, zstd decoded on the copy; unknown: body not stored, gap |
| WebSocket | Plain frames (no deflate negotiated), about 80-line parser | Unmask, join fragments, inflate `permessage-deflate` per direction |
| Tee | Copy into per-exchange memory buffer; record after exchange | Non-blocking tee buffer to a parser goroutine; overflow drops the rest of that connection with a gap |
| Large bodies | In memory; could spill past 64 MB (about 30 lines more) | Spill to temp file above 32 MiB |
| `NO_PROXY` | Cleared, so nothing bypasses by an inherited list | Loopback excluded; a local model on 127.0.0.1 is not recorded |
| Node `fetch` | Not mentioned | Always set `NODE_USE_ENV_PROXY=1` |
| CA | Per part, in memory only; key never on disk | One per machine, made at setup, key file mode 0600, never uploaded |
| Subagents | Wire plus one session poller; no subagent code | Same (two nets) |
| Session files | Poll every 10 s; new bytes to last newline as `file` records in the log | Walk root every 10 s, stat known files every 5 s; append mode or 1 MiB chunked snapshot with hash dedupe; slices go straight into packs |
| SQLite | Copy `.db` + `-wal` when changed, at most every 10 min and at seal, masked same-length | Raw read of `.db` + `-wal`, re-read if WAL salt changed (3 tries), at most every 15 min and at seal, 1 MiB chunks deduped |
| Masking output | `<redacted:KIND>` in text; same-length `*` in binary | Same-length `*` everywhere, plus a count per kind |
| Run secrets | `.env*` values of 8+ characters + live credential header values, in 5 encodings | Same |
| Split secrets | Joined delta text with a map back to raw spans | Same (about 80 lines) |
| Local format | One append-only JSONL log, gzip segments every 10 min or 32 MB | zstd wire segments (level 3, 128 MiB window) rotated at 256 MiB or 10 min, `events.jsonl`, `state.json`, one-pack `outbox/` |
| Upload | `PUT` segment by `run/part/seq` with `x-sha256`; heartbeat and seal endpoints | One `PUT` of a pack by `run/part/seq`; heartbeat is an empty pack every 5 min; final pack has `"final": true` |
| Server store | Object storage + small `segments` and `runs` tables | Object storage only + `runs/{run}/latest.json` |
| Rebuild | `goalcup admin rebuild` in the same binary, about 150 lines | Offline rebuild tool, about 250 src lines |
| Participant view | One paragraph, `goalcup status`, one summary | Same, with five `PROBLEM:` checks |
| Connector | About 15 lines of TOML or a Go struct | One TOML file, about 20 lines |
| Harness test | Practice run with a prompt asking for three subagents | `goalcup doctor --harness X` (about 100 lines) with a prompt asking for two subagents and one tool call |
| Sleep | `clock_jump` gap when wall and monotonic clocks differ by more than 5 s | `suspended {from, to}` when the wall clock jumps more than 60 s between 5 s ticks |
| Deadline stop | SIGTERM to process group, SIGKILL after 10 s, seal | SIGTERM to process group, SIGKILL after 30 s, seal |
| If goalcup dies | Linux: `Pdeathsig` kills harness. macOS: harness stuck (fail-closed) until `goalcup continue` | Supervisor restarts recorder on the same port with a `recorder_restart` gap |
| Language | Go, no third-party dependency; Rust 1.5 to 2 times the lines | Go, three packages (zstd, brotli, TOML); Rust about 1.3 times |
| Client size (src / tests) | about 2,040 / 1,320 | about 2,070 / 1,980 |
| All in (src / tests) | about 2,550 / 1,620; 1.5x: about 3,800 / 2,400 | about 2,570 / 2,430 plus about 400 src server lifecycle |

## B.2 Where the two designs disagree

| # | Topic | B1 view | B2 view | Note |
|---|---|---|---|---|
| 1 | Change forwarded headers? | Yes, two disclosed changes, to cut brotli, zstd, HTTP/2 and inflater code. Risk listed: a provider that requires compression fails (B1 failure 10). | Never. "A pipe does exactly what the client and server do." Accepts one zstd and one brotli package. | Existing code agrees with B2 (`README.md:150` "Bytes are relayed unchanged"). B2's choice costs a few hundred lines more; B1's choice adds a behaviour change that must be tested per harness. |
| 2 | One process or two | One. A separate process needs a pid file, health checks, a socket and restart logic. If the process dies, the part ends. | Two. The proxy is in the harness's data path; a separate recorder can be restarted on the same port within a second. Three would be too many. | B1's macOS failure (row 5) blocks the harness until `goalcup continue`; B2's row 6 blocks only after two independent crashes. |
| 3 | `NO_PROXY` | Cleared, including loopback, so a local model server is recorded. Risk: a corporate host that needed direct access now tunnels (B1 failure 16). | Loopback excluded; OpenCode's local server must bypass the proxy and Claude Code never proxies loopback WebSockets. A local model is a known gap (B2 failure 10). | B2's evidence suggests clearing loopback breaks OpenCode. Per-connector `no_proxy` (B2 connector field) is a middle path. |
| 4 | CA lifetime | New CA per part, key only in memory. | One CA per machine, key on disk 0600. | B1 is stronger for point 4; B2 avoids re-trust per part. |
| 5 | Session file transport | File chunks become `file` records inside the one log. | File slices go into packs directly from the harness's files; snapshots dedupe 1 MiB chunks. | B2 avoids copying session bytes twice on disk; B1 has one format. |
| 6 | SQLite | Copy pair when changed, every 10 min max. Torn copy is a known risk. | Raw read with WAL-salt check, 15 min max, chunk dedupe. Intermediate states between snapshots are lost (B2 failure 11). | Both better than today's single `final_snapshot` for Hermes (Appendix A, A.10 section 2). |
| 7 | Masking marker | Text `<redacted:KIND>`, binary same-length. | Same-length everywhere; leaks secret length, accepted. | Same-length keeps append cursors and chunk hashes stable. |
| 8 | Server index | Small `segments` and `runs` tables. | No rows per entry; object storage plus `latest.json`. | Existing Worker uses Supabase tables (Appendix A, A.10 section 1). |
| 9 | Harness check | A practice run. | A `doctor` command that runs the same check (about 100 lines). | |
| 10 | Workers body limit | 100 MB "on lower plans", not re-checked; 32 MB segments fit. | 100 MB on Free and Pro; 64 MiB packs. | Both cite https://developers.cloudflare.com/workers/platform/limits/ |
| 11 | Hermes files | Snapshot `state.db`; also `sessions/*.json`. | `state.db` (+ `-wal`) only, snapshot mode. | Existing connector reads both (`connectors/hermes-cli/harness.toml`, Appendix A). |
| 12 | Codex files | `sessions/**/*.jsonl`. | `sessions` plus `*.sqlite` at the home root. | |
| 13 | Version | `<cmd> --version` once per part in `meta`. | User agent on every call, plus `version` at part start. | |
| 14 | Agent-facing extras | `goalcup time-left` (about 15 lines) and seal refusal inside the agent are kept as rules. | Left out as event plumbing; about 60 lines if needed. | |

## B.3 Shared ground

### Constraints both designed against

| Constraint | Source |
|---|---|
| Seven points, no caps of our own, cut first | `VISION.md:26-44` |
| Week-long run, start window 7 Nov 00:00 UTC to 8 Nov 00:00 UTC, ready 6 Nov | `VISION.md:13-17`, `:43` |
| Proxy set for the harness process only; never change a base URL or app-wide trust or config | `docs/RULES.md:7-15` |
| Harness runs in its normal chat mode in the participant's own terminal | `docs/RULES.md:7-9` |
| Each harness gets its own home inside goalcup's config; goalcup never touches `~/.claude`, `~/.codex`, `~/.hermes` | `docs/RULES.md:274-288` |
| Parts: `goalcup continue` starts the next part of the same run | `docs/RULES.md:70-73` |
| Hard stop at 168 hours on the server clock | `docs/RULES.md:48-50`, `:326-327` |
| macOS, Linux, WSL2 | `docs/RULES.md:266-267` |
| Owner: disk full gets no special handling; file path is the ID; byte-for-byte file match not required | brief, lines 29-40 |
| Owner: masking keeps every secret rule, adds URL password rule, drops email and rate-limit rules | brief, lines 37-38 |

Items in RULES.md that are rules, not points (hard stop, `time-left`, git commit at seal, workspace
checks), B1 marks as "rules". They stay only where they are a few lines.

### Options weighed for capturing model calls (both designs)

| Option | Covers | Cost | Verdict (B1) | Verdict (B2) |
|---|---|---|---|---|
| A. Rewrite each harness's base URL to a local gateway | Only providers we map; API-key users only | Per harness, per provider | No. `RULES.md:14`: goalcup never changes a model base URL. Also breaks OAuth logins on some harnesses. | Rejected: changes harness config, misses OAuth paths. |
| B. `HTTPS_PROXY` in the harness's env plus a run CA in its CA variable, decrypt known model hosts | Every host the harness reaches through the proxy; every provider; every subagent that inherits env | One proxy, one data table | **Chosen.** | **Chosen.** |
| C. Read only session files | Only what the harness writes down | Decoder per harness (B1); zero network code (B2) | Not enough alone. `RULES.md:301-304`: some internal calls appear in no transcript. Kept as the second source. | Not enough alone. Harnesses truncate tool output, skip helper calls, and some keep threads in forms we cannot read (RULES "What the evidence covers"). |
| D. OS-level capture (eBPF uprobes on TLS libraries, macOS network extension) | Yes | Very large, per OS, needs root | No. Too much code, root, and it is app-wide. | (not weighed) |
| E. System proxy or trust store | Everything on the machine | Admin rights, privacy | (not weighed) | Rejected by the rules ("never ... any app-wide trust"). |

### Harness evidence from the web (merged, with every source)

| Harness | Proxy variables | CA variable | Sessions and subagents | Sources |
|---|---|---|---|---|
| Claude Code | Reads `HTTPS_PROXY`/`HTTP_PROXY`/`NO_PROXY`; no SOCKS; trusts bundled plus OS roots by default | `NODE_EXTRA_CA_CERTS` | `<config>/projects/<project>/<session>.jsonl`; subagents in `<session>/subagents/agent-<id>.jsonl` plus `agent-<id>.meta.json` naming the parent `toolUseId`; nested subagents sit flat beside their parent; workflow agents in `subagents/workflows/wf_<id>/` (third-party, **unverified** against official docs). Gzips request bodies by default (`CLAUDE_CODE_GZIP_REQUEST_BODIES`). Background agents run under a per-user supervisor that "inherits the environment of whichever shell starts it first". | https://code.claude.com/docs/en/network-config ; https://cdn.jsdelivr.net/npm/oh-my-opencode@4.18.2/dist/skills/coding-agent-sessions/references/claude.md (third-party); https://frederick-douglas-pearce.github.io/blog/2026/inside-the-subagent-trace-file/ |
| Codex CLI | Honours `HTTPS_PROXY` per third-party guides, **unverified** against upstream source or official docs | `CODEX_CA_CERTIFICATE`, then `SSL_CERT_FILE`, then system roots; since about v0.129 the same CA path covers WebSocket | `<CODEX_HOME>/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl`, one per thread, subagents included; parent link carried as thread metadata; a SQLite state file; spawned threads linked in a SQLite table (third-party, **unverified**). Talks to the Responses API over WebSocket by default in recent versions. A user reported 2,393 subagent rollout files from one resumed thread, about 731 GiB logical size (found via search, issue link **unverified**). | https://codex.danielvaughan.com/2026/05/08/codex-cli-tls-inspecting-proxies-custom-ca-certificates-enterprise/ ; https://codex.danielvaughan.com/2026/04/18/codex-cli-proxy-configuration-socks-http-corporate-networks/ ; https://learn.chatgpt.com/docs/config-file/environment-variables ; https://codex.danielvaughan.com/2026/04/29/codex-cli-rollout-files-session-recording-replay-audit-trails/ ; https://codex.danielvaughan.com/2026/06/01/codex-cli-session-history-local-search-rollout-format-knowledge-mining/ ; https://cdn.jsdelivr.net/npm/oh-my-opencode@4.18.2/dist/skills/coding-agent-sessions/references/codex.md ; https://luna-shi.github.io/blog/codex-source-dive-subagents/ ; https://developers.openai.com/codex/subagents ; https://github.com/openai/codex/issues/13041 ; https://codex.danielvaughan.com/2026/05/03/codex-cli-websocket-mode-responses-api-persistent-connections-agent-loop-performance/ |
| Hermes Agent (Python) | `httpx` honours `HTTPS_PROXY` when `trust_env` is on; that Hermes and every provider client keep it on is **unverified** | `SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE` (bundle = system roots + run CA) | SQLite `state.db` in WAL mode shared by all sessions, subagent rows tagged in `model_config`; `delegate_task` children run in the same process. A `state.db` corruption report exists. | https://www.python-httpx.org/environment_variables/ ; https://hermes-agent.nousresearch.com/docs/developer-guide/session-storage (official) ; https://gormes-docs.pages.dev/upstream-hermes/session-storage/ (third-party) ; https://skills.cat/skills/nousresearch/hermes-agent/hermes-agent-spawning (third-party) ; https://github.com/NousResearch/hermes-agent/issues/5563 |
| OpenCode (Bun) | Bun `fetch` honours `HTTPS_PROXY`; OpenCode docs list `HTTPS_PROXY`, `NO_PROXY`; its local server connection must bypass the proxy | OpenCode docs say `NODE_EXTRA_CA_CERTS` works, but the Sentry CLI notes Bun ignores it and `SSL_CERT_FILE`. **Unverified; measure before shipping.** | Newer builds: SQLite `opencode.db` with `session.parent_id` for subagents; older: JSON tree `storage/session`, `message`, `part` (third-party, **unverified** for current versions). A 24 GB session database was reported. | https://bun.com/guides/http/proxy ; https://bun.com/docs/guides/http/proxy ; https://opencode.ai/en/docs/network ; https://opencode.ai/docs/network/ ; https://github.com/getsentry/cli/pull/901 ; https://blog.devgenius.io/agent-session-persistence-in-opencode-9c3440448c06 ; https://forums.basehub.com/anomalyco/opencode/15 ; https://pkg.go.dev/github.com/mrf/agentwatch@v0.2.0/sources/opencode ; https://dev.to/whtsky/garbage-collecting-opencodes-24-gb-session-database-7b5 |
| Amp | No official statement found on `HTTPS_PROXY` or CA variables. **Unverified; measure.** | (**unverified**) | Threads appear to live on Amp's server (`amp threads export`); local files unclear, **unverified** | https://ampcode.com/manual ; https://www.developersdigest.tech/reference/amp/subcommands (third-party) |
| Plain Node `fetch` | Ignores `HTTPS_PROXY` unless `NODE_USE_ENV_PROXY=1` (Node 22.21+ / 24+) | | | https://nodejs.org/en/learn/http/enterprise-network-configuration |

Other web facts used: Go raises the soft `RLIMIT_NOFILE` to the hard limit at start on Unix
(https://go.dev/doc/go1.19), so the macOS soft default of 256 does not bite the recorder. Whether Go
restores it for children is **unverified**.

---

## B.4 Design 1 (B1): least code, in full

### B1.0 The answer in ten lines

1. One binary, `goalcup`. One process per part: it is the proxy, the file watcher, the recorder and the
   uploader, and it is the parent of the harness. No daemon, no socket, no second process.
2. Model calls are captured by a recording proxy set through the harness's own environment
   (`HTTPS_PROXY`, `HTTP_PROXY`) plus a per-run certificate authority (CA) passed through the
   harness's CA variable. Only known model hosts are decrypted. Everything else is tunnelled and
   logged as host, port, bytes and times.
3. Subagents need no subagent code. Every subagent's calls go through the same process environment,
   so the same proxy sees them. Their session files are picked up by one poller that walks the
   harness's home. The file path is the ID. No per-stream files, so no file-handle limit and no cap.
4. Everything goes into one append-only JSON Lines log per part, cut into gzip segments every
   10 minutes or 32 MB. One open file at a time.
5. One redaction function runs on every byte before it touches disk: header names, URL parts,
   key patterns, exact run secrets (with their encodings), and secrets split across streamed deltas.
6. Any failure to record becomes a `gap` record and a counter. Forwarding never waits on recording.
7. Upload is idempotent `PUT` of immutable segments keyed `run/part/seq`, plus a seal message
   that lists them. The server stores them in object storage and a small index. The story is
   rebuilt by sorting records by `(time, id)`.
8. The participant sees a plain paragraph before launch, `goalcup status` during the run, and the
   same summary after. The record is on their disk as `zcat`-readable files.
9. A new harness is a data entry of about 15 lines: command, home variable, CA variable, model hosts,
   session globs. No client decoder per harness.
10. Estimate: about 2,000 lines of client source, 450 of server source, about 1,300 lines of tests.
    With an honest 1.5x for what was missed: about 3,500 source lines in all.

### B1.1 The proxy (point 1)

Option B is the smallest thing that sees "any provider" without per-provider code. All four target
runtimes honour the standard variables (evidence in B.3).

- Listens on `127.0.0.1:<random port>`. The harness child gets `HTTPS_PROXY`, `HTTP_PROXY`,
  `https_proxy`, `http_proxy` set to it, and `NO_PROXY`/`no_proxy` set to empty, so nothing the
  harness reaches skips us by an inherited bypass list. Plain `http://` model servers (a local
  model server, for example) arrive as ordinary proxy requests and are recorded directly.
- `CONNECT host:port`:
  - If `host` is in the decrypt set: answer the client with a leaf certificate for `host`, signed by the
    run CA (made on the fly, cached in memory per host). Dial upstream with normal system trust.
    Advertise ALPN `http/1.1` only, so the inner protocol is always HTTP/1.1. This cuts all HTTP/2
    code.
  - Otherwise: tunnel bytes as they are and write one `tunnel` record at close (host, port, bytes
    each way, open and close times). Never decrypted.
  - If the client rejects our certificate (pinning, or a runtime that ignored the CA variable):
    note a `gap` of kind `tls_refused` with the host, and tunnel that host for the rest of the part.
    The harness retries and goes on. Recording that host is lost and says so.
- Inside a decrypted connection, for each HTTP/1.1 exchange: forward the request and response
  byte-for-byte, and tee a copy into an in-memory buffer for that exchange. When the exchange ends
  (or the connection drops), redact the buffered copy and write a `req` and a `resp` record. The
  harness never waits on the recorder: forwarding is the same `io.Copy` it would be without us.
- Two small header changes, disclosed to the participant, both on decrypted hosts only:
  - `Accept-Encoding` is set to `gzip` so every response we must redact is decodable with the
    standard library. Most model streams are not compressed anyway.
  - `Sec-WebSocket-Extensions` is removed from WebSocket upgrades, so no `permessage-deflate` is
    negotiated and frames are plain. That turns WebSocket capture into an 80-line frame parser
    instead of a stateful inflater.
- Request bodies the client compressed (Claude Code gzips request bodies by default) are decoded
  if gzip. Any other encoding: the body is not stored, a `gap` of kind `undecoded_body` is written,
  and the headers are stored. Storing undecoded bytes would skip redaction (point 4 beats point 1
  when they clash, and the gap says so).

**Which hosts are decrypted.** One list in the binary: the union of model API hosts across all
connectors, plus wildcards for the big clouds. Example: `api.anthropic.com`, `api.openai.com`,
`chatgpt.com`, `openrouter.ai`, `generativelanguage.googleapis.com`, `*-aiplatform.googleapis.com`,
`bedrock-runtime.*.amazonaws.com`, `*.openai.azure.com`, `api.mistral.ai`, `api.deepseek.com`,
`api.x.ai`, `api.groq.com`, `api.together.xyz`, `api.fireworks.ai`, `ampcode.com`. Exact hosts for
Hermes's default provider and OpenCode's hosted models are **unverified**. `goalcup setup` adds any
custom base URL host the participant names (for a self-hosted gateway).

Why not decrypt everything: it would break every tool the agent runs that pins or ignores our CA
(git, package managers, SDKs inside the product), which slows or blocks the harness (point 3), and it
would pull far more secrets through the recorder (point 4). A model call to a host not on the list
still leaves a `tunnel` record with its byte counts, so the server can flag "a lot of TLS to an
unknown API-looking host" as a probable missed provider.

**Why not a separate proxy process.** A separate process needs a pid file, health checks, a socket
for status and restart logic. In one process the proxy lives exactly as long as the part. If the
process dies, the part ends.

### B1.2 Subagents (point 2)

Two sources, no subagent-specific code in either.

1. **Wire.** In-process subagents (Claude Code's Task tool, Codex's `spawn_agent`, Hermes's
   `delegate_task`) share the parent's network stack; child-process subagents inherit its
   environment. Either way their calls go through our proxy. Each TCP connection gets its own
   goroutine and each exchange its own buffer. Nothing is counted against a limit.
2. **Session files.** One poller walks the harness's session globs every 10 seconds, `stat`s each file,
   and for any file that is new or larger than last seen, reads the new bytes up to the last newline,
   redacts them and writes a `file` record `{path, offset, data}`. It opens one file at a time and
   closes it at once, so a week with tens of thousands of subagent files never holds more than a
   handful of descriptors (macOS defaults to 256 per process, brief line 32). Offsets are saved in the
   run folder so `goalcup continue` does not resend old bytes. A file that shrank or whose first
   4 KB changed is read again from 0, with a `file_rewritten` note.

SQLite files (Hermes, OpenCode) cannot be tailed by offset. For a file matched as `sqlite` in the
connector, the poller copies the `.db` plus `-wal` when either changed, at most every 10 minutes and
at seal, redacts the copy with same-length masking, and writes it as a `snap` record. A copy taken
during a write may be torn; the WAL usually lets a reader recover. That is a known risk, not a gap
B1 can close without linking SQLite. Amp gets wire only.

Scale: the reported 2,393 Codex subagent rollout files (about 731 GiB logical, link **unverified**).
The poller handles count; size is the disk's and the server cap's problem, by owner decision.

### B1.3 Secrets (point 4)

One function, `redact(kind, bytes) -> bytes`, called on every header set, URL, body, WebSocket
payload, file chunk and snapshot before anything is written. Nothing reaches disk unredacted, and the
run CA key never touches disk (it lives in memory for the part; a new part gets a new CA).

| Layer | Rule | Applies to |
|---|---|---|
| Header names | value replaced by `[masked]` for `authorization`, `cookie`, `set-cookie`, `x-api-key`, and any name containing `key`, `token`, `secret`, `password`, `auth` (`RULES.md:338-340`) | req/resp headers, WebSocket upgrade |
| Header values | any `Bearer ...` or `Basic ...` masked whatever the header name | all headers |
| URLs | userinfo (including the password part) masked; query values whose name matches the header rule, `sig`, or contains `signature` masked (`RULES.md:341-343`) | request URLs, URLs inside bodies |
| Patterns | known key shapes: `sk-...`, `sk-ant-...`, `github_pat_...`, `ghp_`/`gho_`/`ghs_`, `AKIA...`, `xox[abpr]-...`, `AIza...`, JWTs `eyJ...\.eyJ...\.`, PEM private key blocks, `scheme://user:pass@` | all bodies, file chunks, snapshots |
| JSON keys | string value of keys like `access_token`, `refresh_token`, `id_token`, `api_key`, `apiKey`, `client_secret`, `password` | JSON bodies and JSONL file lines |
| Exact run secrets | every value of 8+ characters from the workspace `.env*` files at start, plus every credential header value seen live (kept in memory only); each also matched as base64 (3 alignments), base64url, URL-encoded and JSON-escaped | everything |
| Split across deltas | for streamed responses, the text from known delta fields (`text`, `delta`, `partial_json`, `arguments`, `thinking`) is joined with a map back to each delta's raw span; a pattern or exact match in the joined text masks every delta it touches | SSE bodies and WebSocket text frames |

Text gets `<redacted:KIND>`. Binary (SQLite snapshots) gets same-length `*` so the file stays
readable. Cost: about 250 lines. It buys point 4 in one place, testable with one corpus of fixtures.
The split-delta rule is the one piece the rules say is missing today (`RULES.md:289-298`).

### B1.4 Gaps without blocking (point 3)

- Forwarding is never behind recording. The tee copies into memory; the write happens after the
  exchange, in the connection's own goroutine, after the client already has every byte.
- Every recorder step returns an error instead of panicking; each connection handler has a
  `recover()`. On error, write a `gap` record (if the log still works) and bump a counter in
  `status.json` (which works even when the log does not, since it is rewritten whole).
- Gap kinds: `write_failed`, `tls_refused`, `undecoded_body`, `upstream_error`, `file_read_failed`,
  `file_rewritten`, `snapshot_failed`, `upload_failing`, `upload_rejected` (server said 413 or
  similar), `part_start`/`part_end` (so downtime between parts is visible), `clock_jump`.
- Disk full: by owner decision, nothing special. Writes fail, gaps are counted in memory and in the
  heartbeat, and the harness carries on until the disk stops it too.
- Upload failures never stop the harness: segments wait on disk and retry.

### B1.5 On-disk record format (point 5)

```
~/.goalcup/
  token                      device login (0600)
  homes/<harness>/           the harness's own home (rules), e.g. CLAUDE_CONFIG_DIR, CODEX_HOME
  runs/<run_id>/
    part-01/
      open.jsonl             current segment, plain text, appended
      00000001.jsonl.gz      closed segments, immutable
      00000002.jsonl.gz
      offsets.json           poller state (path -> size, head hash)
      uploaded.txt           seq numbers the server acked
    part-02/ ...
    status.json              rewritten every 10 s, read by `goalcup status`
    pid                      pid and pgid of the part, for `seal` and stale-part cleanup
```

Segment rotation: every 10 minutes or 32 MB, whichever first. At rotation `open.jsonl` is gzipped to
the next number, fsynced, and deleted. On start, a leftover `open.jsonl` from a crash becomes a
segment first. Only one file is ever open for writing.

One JSON object per line. Every record has `t` (wall clock, nanoseconds), `k` (kind) and `id`.

| `k` | Fields | ID |
|---|---|---|
| `meta` | run, part, harness, harness version (`<cmd> --version`), goalcup version, OS facts (os, version, arch, CPUs, memory, disk size/free, WSL2, container), server time at open, deadline | `p<part>` |
| `req` | conn, method, url, headers, body (`body` as UTF-8 text, or `body_b64`) | `p<part>.c<conn>.x<n>` |
| `resp` | status, headers, body, `partial` if the stream was cut | same as its `req` |
| `ws` | dir (`up`/`down`), opcode, payload | `p<part>.c<conn>.x<n>.f<m>` |
| `tunnel` | host, port, bytes up/down, open/close times, reason (`not_model_host`, `tls_refused`) | `p<part>.c<conn>` |
| `file` | path relative to harness home, offset, data | the path (owner decision) |
| `snap` | path, data_b64 (gzip of the masked copy), sizes | the path plus `@t` |
| `gap` | kind, detail, counts | `p<part>.g<n>` |
| `end` | reason (`exit`, `signal`, `deadline`, `seal`), exit code, git HEAD and a digest of `git status --porcelain` (rules) | `p<part>.end` |

Why JSON Lines and gzip: both are in every standard library, both are readable by the participant
with `zcat | less`, and gzip is streamable on the server.

### B1.6 Upload protocol and server (point 5)

Client to server, all over HTTPS with a per-run bearer token from the run approval:

| Call | Purpose | Idempotent by |
|---|---|---|
| `POST /device` then poll `/device/<code>` | `goalcup login`, once per computer | device code |
| `POST /runs` `{harness, practice, repo}` returns approval URL; poll until approved, returns `{run_id, part, deadline, server_time, upload_token}` | open run (start) or next part (continue) | approval id |
| `PUT /runs/<id>/parts/<n>/segments/<seq>` body = gzip bytes, header `x-sha256` | upload one segment | same sha: 200; different sha: 409 (logged as gap, never overwritten) |
| `POST /runs/<id>/heartbeat` `{part, counters, gaps}` every 5 minutes | liveness, gap counts, for emails | none needed |
| `POST /runs/<id>/parts/<n>/seal` `{last_seq, shas[]}` | says the part is complete | same body: 200 |

Uploader loop: every 30 seconds, send every closed segment not in `uploaded.txt`, oldest first, one at
a time, with backoff on failure. After the harness exits: rotate, upload, seal, retrying for up to one
hour (rules). Any later `goalcup` command first flushes what is still pending (rules:
`RULES.md:332-335`).

Server (a Cloudflare Worker, since that is what exists; any small HTTP service would do):

- Object storage key `runs/<run_id>/<part>/<seq:08>.jsonl.gz`. Written only if absent.
- One small table `segments(run, part, seq, sha256, bytes, received_at)` and `runs(id, account,
  harness, practice, opened_at, deadline, state, sealed_parts)`.
- Storage-cost guard: a per-run byte total is checked on each PUT. Over the limit the server answers
  413, the client keeps the segment on disk and writes `upload_rejected`. The limit's value is the
  owner's call (brief line 35). The client has no cap.
- Cloudflare Workers request bodies are limited to 100 MB on lower plans (not re-checked), so 32 MB
  segments fit.

Rebuild by ID and time (`goalcup admin rebuild <run>`, same binary, organizer only): list segments by
`(part, seq)`, stream each, and for each part emit records in log order (already time order within a
part). Merge parts by `t`, correcting each part's clock by `server_time - client time` from its `meta`.
Pair `req`/`resp`/`ws` by `id`. Join `file` chunks by path and offset into whole files. Output one
timeline JSONL plus a folder of rebuilt session files. About 150 lines.

### B1.7 What the participant sees (point 6)

Before launch, printed once, in plain words:

```
Goal Cup is recording this run (run r_8f3k, part 1).
It records: every request and reply between your harness and its model service, its session files
(including every subagent), the hosts it connects to (name, port, bytes, times only), its version,
and basic machine facts (OS, CPU, memory, disk; no names).
It never records: login headers, keys from your .env files, anything from other programs.
It changes two transport headers on model traffic (compression only) so secrets can be removed.
Your record is in ~/.goalcup/runs/r_8f3k and uploads every 10 minutes.
`goalcup status` in another terminal shows what has been recorded.
```

`goalcup status` (reads `status.json`; works from any terminal):

```
Run r_8f3k  part 1  running  52h10m elapsed  115h50m left
Model calls recorded   4,812   (api.anthropic.com 4,790, api.openai.com 22)
Session files          1,204   (1,190 subagent files)
Other hosts            37      (not decrypted; see list with --hosts)
Recorded               2.1 GB  uploaded 308 of 312 segments
Gaps                   3       tls_refused: registry.example.com (1), undecoded_body (2)
PROBLEM: last upload failed 14 min ago (network). Retrying. Your run is not affected.
```

After the harness exits, the same summary prints, plus where the files are and the `zcat` command
to read them. That is all of point 6: one paragraph, one status screen, one summary.

### B1.8 How a harness is added (point 7)

A connector is a struct in one Go file (or a TOML block; data either way):

```toml
[codex-cli]
command      = ["codex"]
version      = ["codex", "--version"]
home_env     = "CODEX_HOME"            # goalcup sets it to ~/.goalcup/homes/codex-cli
ca_env       = ["CODEX_CA_CERTIFICATE"]
model_hosts  = ["api.openai.com", "chatgpt.com"]
sessions     = ["sessions/**/*.jsonl"]
snapshots    = []                       # sqlite files to copy, if any
extra_env    = {}                       # e.g. CLAUDE_CODE_GZIP_REQUEST_BODIES = "0" if wanted
```

Steps to add one: write the entry; run `goalcup start --practice --harness <id>`; type a prompt that
asks the agent to start three subagents; check `goalcup status` shows model calls, more than one
session file and no `tls_refused`; ship. About 15 lines of data and one practice run. No client-side
decoder. Decoding chat files for review is a server or offline job and can come after Friday.

Initial table (values marked **unverified** are to be measured in a practice run before 6 Nov):

| Harness | home_env | ca_env | sessions |
|---|---|---|---|
| Claude Code | `CLAUDE_CONFIG_DIR` | `NODE_EXTRA_CA_CERTS` | `projects/**/*.jsonl`, `projects/**/*.meta.json` |
| Codex CLI | `CODEX_HOME` | `CODEX_CA_CERTIFICATE` | `sessions/**/*.jsonl` |
| Hermes Agent | `HERMES_HOME` (**unverified** name) | `SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE` (bundle = system roots + run CA) | snapshot `state.db`; `sessions/*.json` |
| OpenCode | `XDG_DATA_HOME`, `XDG_CONFIG_HOME` | `NODE_EXTRA_CA_CERTS` (**unverified** under Bun) | snapshot `opencode/opencode.db`; `opencode/storage/**/*.json` |
| Amp | (**unverified**) | `NODE_EXTRA_CA_CERTS` (**unverified**) | none; wire only |

For `SSL_CERT_FILE` style variables, which replace the trust store rather than add to it, goalcup
writes `ca-bundle.pem` = system roots + run CA in the run folder. The public CA cert is not a secret;
its key never leaves memory.

### B1.9 Process model and lifecycle

- `goalcup start` (and `continue`): approve in browser, open run or part, generate run CA in memory,
  start proxy on a random loopback port, start poller and uploader goroutines, write `meta`, print the
  paragraph, then `exec` the harness as a child in its own process group with the terminal attached
  (stdin, stdout, stderr inherited; no PTY layer, no input capture code). Wait for it.
- On Linux the child gets `Pdeathsig = SIGTERM`, so if goalcup dies the harness dies and the part
  ends cleanly. macOS has no such flag (see failure modes).
- Deadline (rules): a timer at the server's deadline, corrected by the start-time clock offset.
  At the deadline: SIGTERM to the process group, SIGKILL after 10 seconds, then seal.
- `goalcup seal` (rules): from another terminal, asks "stop the agent? y/N", then signals the pid in
  `runs/<id>/pid`. It refuses when `GOALCUP_RUN` is in its own environment (the harness child has it
  set), so the agent cannot seal itself.
- `goalcup time-left` (rules): reads `status.json`. About 15 lines.
- `goalcup continue` first kills any process group left in `pid` from a dead part, so no harness runs
  unrecorded.

### B1.10 What B1 deliberately leaves out

| Left out | Why it is safe to leave out | Point it would have served |
|---|---|---|
| Separate daemon, IPC socket, supervisor | One process does it; status via a file | none |
| PTY or terminal recording, input relay | Human messages reach the model and the session files; both are recorded | 1 (already met) |
| HTTP/2 inside the MITM | ALPN offers only HTTP/1.1; clients that demand h2 tunnel with a `tls_refused` gap | 1 (partly) |
| Brotli and zstd decoding | `Accept-Encoding: gzip` on model hosts; anything else is a gap | 1 (edge) |
| Per-harness decoders and verifier labels in the client | Raw records are enough to upload; decoding is review work | none of the seven |
| Per-stream or per-subagent output files | One log; no open-file limit, no caps | 2 (better without) |
| Byte-for-byte file matching | Owner dropped it | none |
| Disk-pressure freeze and resume | Owner: no special handling | 3 (arguably against it) |
| Workspace checks (empty folder, `.gitignore` covers `.env*`) | Rules, not points; the server checks the repo is empty. Keep the `.gitignore` check only if the owner wants it (about 40 lines) | 4 indirectly |
| Version sampling during the run | Recorded once per part in `meta`; a change between parts shows | none |
| Email, public board, judging | Server features, not the recorder | none |
| Native Windows, desktop apps | Rules exclude them | 7 (by rule) |
| Auto-update | Install script handles releases | none |
| Recording the harness's sign-in hosts | Never decrypted; tunnel records only | 4 |

### B1.11 Lines of code, honest estimate

Language: Go, because `net/http`, `crypto/tls`, `crypto/x509` (on-the-fly certificates),
`compress/gzip`, `encoding/json` and `os/exec` with process groups are all in the standard library.
No third-party dependency is needed. If the team stays in Rust, expect about 1.5 to 2 times these
source numbers (hyper, rustls, rcgen, tokio wiring), and a much larger test harness.

| Component | Point(s) | Src lines | Test lines |
|---|---|---|---|
| CLI entry, commands (`login start continue status seal upload time-left admin rebuild`) | 6, rules | 180 | 40 |
| Server client: device login, open run/part, heartbeat | 5 | 180 | 80 |
| Proxy: CONNECT, tunnel, MITM, leaf certs, HTTP/1.1 tee, plain HTTP proxying | 1, 2, 3 | 350 | 250 |
| WebSocket frame parser and recorder | 1 | 90 | 60 |
| Redaction (all layers, split-delta, exact values with encodings, same-length binary mask) | 4 | 250 | 400 (mostly fixtures) |
| Recorder: log, rotation, gzip, crash recovery of `open.jsonl` | 5, 3 | 120 | 80 |
| Session poller and SQLite snapshots | 2 | 150 | 100 |
| Uploader and seal | 5 | 120 | 100 |
| Status file, status screen, start paragraph, final summary | 6 | 140 | 40 |
| Child launch, process group, Pdeathsig, deadline stop, seal signal, git facts | 3, rules | 150 | 60 |
| Machine facts | 5 (rules list) | 60 | 10 |
| Connectors (data, 5 harnesses) | 7 | 100 | 20 |
| Rebuild (organizer) | 5 | 150 | 80 |
| **Client total** | | **about 2,040** | **about 1,320** |
| Server Worker: device auth (or reuse existing account auth), runs, segments, heartbeat, seal, size guard | 5 | 450 | 300 |
| Install script | 7 | 60 | 0 |
| **Grand total** | | **about 2,550** | **about 1,620** |

Honest correction: first estimates like this run low. Plan for about 1.5 times: about 3,800 source
lines and 2,400 test lines. **Corrected comparison:** B1 wrote "under 3 percent of the about 142k
source lines the brief counts today". The 142k mixes in tests, SQL and lock files (see B.0). Against
the about 102,406 production src lines in Appendix A (A.2), 2,550 is about 2.5 percent and 3,800 is
about 3.7 percent.

The real cost is not lines; it is that this is a rewrite with eight days left. B1's advice for Friday
is to use this as the target shape and cut the existing code toward it, not to start a new
repository. B1 did not read the existing code, so it could not say which parts already match (section
B.6 does that).

### B1.12 Failure modes of design 1

| # | Failure | Effect | Point hit | Mitigation in design, or residual |
|---|---|---|---|---|
| 1 | A harness or tool ignores `HTTPS_PROXY` (raw socket, `ssh`, a command that unsets it) | Calls not seen at all | 1 | Residual, same as rules today (`RULES.md:299-301`). Server compares session-file turns with wire calls later. |
| 2 | A runtime ignores our CA variable (Bun, possibly) | TLS refused on a model host; the harness retries through the proxy, which now tunnels that host | 1 | `tls_refused` gap; that harness's model calls are then tunnel-only. Must be measured per harness before shipping (OpenCode, Amp). |
| 3 | Model host not in the decrypt list (custom gateway, new provider) | Calls tunnelled, not recorded | 1 | `tunnel` record with bytes; setup asks for custom hosts; server flags large unknown tunnels. |
| 4 | Claude Code background agents run under a shared supervisor that inherits the env of whichever shell started it first (https://code.claude.com/docs/en/network-config, "Apply network settings to background agents") | Background subagents may bypass the proxy | 1, 2 | Could be set via the `env` block of the run home's `settings.json`, but that edits harness config, which the rules forbid for user config. Since the run home is goalcup's own, this may be allowed. **Owner decision; unverified whether the supervisor is per config dir.** |
| 5 | goalcup process dies on macOS (OOM, kill) | Harness keeps running, every model call fails with connection refused | 3 | Fail-closed by design: no unrecorded calls, but the harness is stuck until `goalcup continue`, which kills the orphan. Server emails after 20 minutes of silence (rules). |
| 6 | A secret is split in a way the delta-join misses (secret inside tool arguments streamed as escaped JSON inside JSON, or across two exchanges) | Secret recorded | 4 | Fixture corpus; same-key exact matching catches known run secrets in most encodings. Residual for unknown secrets, as today. |
| 7 | Over-redaction (an exact secret value that is a common word; a 9-character `.env` value like `localhost`) | Legit text masked | 1 | Minimum length 8 and skip values that are plain words or URLs without credentials; residual. |
| 8 | Very large response bodies held in memory until the exchange ends | Memory growth with many big parallel streams | 3 | Model responses are small; a 1 GB download from a decrypted host would sit in memory. Could stream to a temp file past 64 MB (about 30 lines more). Not a cap; a spill. |
| 9 | Long-lived WebSocket (Codex Responses over WS) | Frames are written per message, fine; but a split secret across frames on different messages is missed | 4 | Delta-join across frames of one connection; residual. |
| 10 | Stripping `Sec-WebSocket-Extensions` or forcing `Accept-Encoding: gzip` changes server behaviour or bandwidth | Slightly more bytes on the wire, a provider that requires compression fails | 3 | Unlikely; an `upstream_error` gap would show it. Must be tested per harness. |
| 11 | Torn SQLite snapshot | Native record for Hermes/OpenCode unreadable at that snapshot | 2 | Next snapshot and seal snapshot; wire still has every call. |
| 12 | Session files written outside the globs (a harness version moves them) | Native subagent files missed | 2 | Poll the whole home with `**/*` and a size filter instead of narrow globs; costs disk, saves code. B1 would start broad. |
| 13 | Disk fills | Writes fail, gaps counted, harness stops when it cannot write either | 3, 5 | Owner decision: no special handling. |
| 14 | Server per-run limit reached | Segments stay local, `upload_rejected` | 5 | Owner sets the limit; participant sees it in status. |
| 15 | Clock jumps (laptop sleep, NTP) | Out-of-order `t` within a part | 5 | `clock_jump` gap when wall clock moves against monotonic by more than 5 s; rebuild uses log order within a part, `t` only across parts. |
| 16 | `NO_PROXY` cleared breaks an internal corporate host that needs direct access | That host now goes through our tunnel; works unless the corp proxy is required | 7 | Chain to the participant's own upstream proxy if one was set (about 20 lines); otherwise residual. |
| 17 | Thousands of files every 10 s on a slow disk | Poll takes longer than its interval | 3 | Poll runs in its own goroutine and just starts late; never touches the harness. |
| 18 | Rewrite risk: building this in eight days | Not ready on 6 Nov | all | Use as target shape for cuts, not a fresh repo. |

### B1.13 Point-by-point check

| Point | Met by |
|---|---|
| 1 Record every model call | Env proxy + run CA + decrypt list; tunnel log for the rest; gaps for refusals |
| 2 Every subagent | Same proxy for all; one poller over the whole harness home; one log, no caps |
| 3 Never block or slow | Tee after forward; recover per connection; gaps not stops; one process with the harness as child |
| 4 Never record secrets | One redaction function before every write; CA key in memory only |
| 5 Upload everything, rebuild by ID and time | Immutable gzip JSONL segments, idempotent PUT, seal list, rebuild tool |
| 6 Tell the participant plainly | One paragraph, one status screen, one summary, readable local files |
| 7 Harnesses people use, as they use them | Normal chat mode in their terminal; a connector is 15 lines of data; tested with a prompt that asks for subagents |

---

## B.5 Design 2 (B2): survives a week on a real laptop, in full

Lens: one participant, one laptop (macOS, Linux or WSL2), one harness, 168 hours. The laptop sleeps,
changes Wi-Fi, the harness may update itself, the agent may start thousands of subagents, and the model
provider will have outages. The design must survive all of that with the least code.

### B2.0 The whole design in ten lines

1. `goalcup start` opens the run on the server, starts one background **recorder** process, then starts
   the harness as its own child, in the user's terminal, with a few extra environment variables.
2. The variables point the harness (and everything it starts) at the recorder's local HTTPS proxy and
   make it trust a local CA. Nothing app-wide or system-wide changes.
3. For model hosts, the proxy ends TLS on both sides and **copies bytes in a dumb pipe**. A separate
   goroutine parses the copy into exchange records. The pipe never waits for the parser.
4. For every other host, the proxy is a blind tunnel. It notes host, port, bytes and times only.
5. Secrets are masked in memory before anything touches disk. If a body cannot be masked (unknown
   encoding), it is not stored; a gap is noted instead.
6. The recorder polls the harness's own session folder every few seconds and picks up every new or
   grown file. Subagent files are just more files.
7. Every 60 seconds the recorder packs new bytes (wire records, session file growth, events) into one
   compressed **pack** and PUTs it to the server. One endpoint, idempotent by sequence number.
8. The server stores packs as-is in object storage. A rebuild tool replays packs in order to rebuild
   every file by path and every exchange by time.
9. `goalcup status` prints plain counts: calls, files, subagent files, bytes uploaded, gaps.
10. A new harness is one small data file: launch command, home variable, model host suffixes, session
    folder. No harness-specific code on the client.

### B2.1 Processes and data flow

```
terminal
  goalcup start  (foreground, tiny supervisor: opens run, restarts recorder, waits for harness)
     |-- harness (child, owns the TTY, env: HTTPS_PROXY + CA vars + home var)
     |      |-- subagents (threads or child processes: inherit env, same proxy)
     |      `-- tools (curl, git, npm...: inherit env)
     `-- goalcup recorder (detached daemon, nice +10)
            proxy :PORT  -> model hosts: TLS pipe + tee -> parser -> masker -> wire-NNN.jsonl.zst
                         -> other hosts: blind tunnel -> events.jsonl (host, port, bytes, times)
            watcher      -> polls harness session folder -> cursors
            uploader     -> builds pack from wire segments + file growth + events -> PUT -> server
            status       -> status.json (read by goalcup status)
server (Worker + object storage)
  PUT /v1/runs/{run}/parts/{part}/packs/{seq}  -> R2 runs/{run}/p{part}/{seq}.pack
  writes runs/{run}/latest.json (status, last seen) for email and status checks
rebuild tool (offline, organizer side)
  reads packs in seq order -> files by path, exchanges by time, gaps, connections
```

Why two processes and not one: the proxy is in the harness's data path. If it dies, the harness
cannot reach its model. A separate recorder lets the small foreground supervisor restart it on the
same port within a second and write a `recorder_restart` gap. Why not three: the uploader and watcher
are cheap goroutines with `recover()` around them; a bug there notes a gap and restarts the loop, it
does not take the proxy down.

Why the harness is a plain child and not wrapped in a PTY: the harness owns the real terminal, so
typing is never slowed and no terminal code is needed. Human messages are recorded where they reach
the model (the wire) and where the harness writes them (session files).

### B2.2 Point 1: model calls

Environment the recorder adds to the harness (and only the harness):

```
HTTPS_PROXY=http://127.0.0.1:PORT   https_proxy=...   HTTP_PROXY=...  http_proxy=...
NODE_USE_ENV_PROXY=1
NODE_EXTRA_CA_CERTS=<run>/ca.pem
SSL_CERT_FILE=<run>/bundle.pem        (system roots + our CA; SSL_CERT_FILE replaces, not adds)
REQUESTS_CA_BUNDLE=<run>/bundle.pem   CURL_CA_BUNDLE=<run>/bundle.pem
CODEX_CA_CERTIFICATE=<run>/bundle.pem
<HOME_VAR>=<goalcup home for this harness>   (CLAUDE_CONFIG_DIR, CODEX_HOME, HERMES_HOME, ...)
```

`NODE_USE_ENV_PROXY=1` is always set; it is harmless where unused. `bundle.pem` is built once per part
from the system PEM (`/etc/ssl/cert.pem` on macOS, `/etc/ssl/certs/ca-certificates.crt` on Debian-like
Linux, a short list of known paths) plus our CA. If no system PEM is found, the start says so plainly
and uses our CA plus a Mozilla bundle shipped in the binary.

The CA: one key pair per machine, made at setup, private key file mode 0600, never uploaded. Leaf
certs are minted per host on demand and cached in memory.

**Which hosts are opened.** Decrypt list = union of: the connector's model host suffixes (for example
`anthropic.com`, `openai.com`, `chatgpt.com`); a shared list of about 30 known model-provider host
suffixes (OpenRouter, Google generative API, Mistral, Groq, DeepSeek, xAI, Together, Fireworks,
Bedrock runtime, Vertex, Azure OpenAI and so on); any extra host the participant names at setup
(custom gateway), disclosed in the record. Suffix matching, not exact hosts, so a harness update that
moves to `api2.vendor.com` stays covered. Everything else is a blind tunnel: one `conn` event at close
(host, port, bytes up and down, open and close time). Never decrypted, never stored.

Why not decrypt everything: it would put git pushes, package downloads and every web page the agent
reads into the record (huge, and full of secrets), and it would break every tool that ignores our CA.

**The dumb pipe plus tee.** For a model host:

1. `CONNECT host:443` arrives. Dial upstream TCP first. If that fails, answer `502` to CONNECT, so the
   harness sees an ordinary network error and retries the way it always does. Write a `conn_error`
   event.
2. Accept the client TLS with our leaf cert, ALPN offering only `http/1.1`.
3. Dial upstream TLS, verified against the system roots, ALPN `http/1.1`.
4. Copy bytes both ways with `io.Copy`. Each direction also writes into a **non-blocking tee buffer**
   that feeds a parser goroutine. If the tee buffer is full, the tee drops the rest of that
   connection and writes a `gap: tee_overflow` event. The pipe never waits.
5. The parser reads the client copy with `http.ReadRequest` and the server copy with
   `http.ReadResponse(req)` in turn (HTTP/1.1 is in order). On `101 Switching Protocols` it switches
   to a WebSocket frame reader.
6. One wire record per finished exchange (or per WebSocket message) goes to the masker, then to the
   current wire segment.

The forwarded bytes are never changed. Not a header, not a retry, not a timeout. A failure in the
parser cannot affect the harness, it can only cost a record (and a gap says so). Why a pipe and not a
full HTTP reverse proxy: a reverse proxy decides retries, pooling, timeouts, header rewriting and idle
closes on its own. Each of those can slow or break a harness after sleep or a network change.

Bodies are held in memory until the exchange ends, spilling to a temp file above 32 MiB. That limit
guards memory, not bytes recorded: nothing is dropped at 32 MiB. A crash loses only exchanges in
flight, noted as a `recorder_restart` gap.

**Encodings and WebSocket.**

- `Content-Encoding` gzip, deflate, br and zstd are decoded on the recorded copy only, using the Go
  standard library plus one brotli and one zstd package. Claude Code gzips request bodies by default.
- Unknown encoding: the body is **not stored**, because we cannot mask it. A `gap: undecodable_body`
  event names host, path, size and encoding.
- WebSocket: Codex CLI talks to the Responses API over WebSocket by default in recent versions. The
  frame reader unmasks client frames, joins fragments, and inflates `permessage-deflate` with one
  long-lived inflater per direction (or one per message when `no_context_takeover` was agreed). The
  extension header is not stripped, so the pipe stays untouched.
- HTTP/2 and gRPC to the client are not offered. A client that insists on h2 fails the handshake.
  None of the five harnesses is known to require it (**unverified** for Amp).

**Plain HTTP and local models.** A plain `http://` request through the proxy is forwarded and recorded
the same way. Loopback traffic is normally excluded through `NO_PROXY` (OpenCode's docs say its local
server connection must bypass the proxy, and Claude Code never proxies loopback WebSockets). So **a
local model on 127.0.0.1 is not recorded on the wire.** It is a known gap.

What could go wrong: a harness ignores `HTTPS_PROXY` or pins certificates, so its model calls are
invisible on the wire (caught by the start check and by "no model calls seen"); a tool the agent runs
calls a model host with a TLS stack that ignores our CA (Go binaries on macOS use the Keychain, not
`SSL_CERT_FILE`, **unverified** for current Go), so that tool's call fails and the product work is
hurt (recorded as `client_tls_reject`); Bun-based harnesses may not read `NODE_EXTRA_CA_CERTS`, so they
fail TLS at once and the start check catches it before the run is approved.

### B2.3 Point 2: subagents

Two independent nets. Neither has any cap.

**Net 1, the wire.** In every harness B2 checked, subagents run in the same process (Claude Code
subagents, Codex agent threads, Hermes `delegate_task`) or as child processes that inherit the
environment. So every subagent model call passes the same proxy. A thousand subagents are a thousand
more exchanges. Subagent identity on the wire comes from what the harness puts in the request (system
prompt, headers); the rebuild tool works that out later.

**Net 2, the session folder.** Each connector names the harness's session root inside the goalcup
home for that harness. The watcher walks that root every 10 seconds and stats known files every 5
seconds. Every new or changed file is picked up, whatever its name or depth. The file path is the ID
(owner decision).

| Mode | When | What is sent |
|---|---|---|
| append | file grew and its first 4 KiB did not change | the new bytes since the last cursor, cut at the last newline so masking sees whole lines |
| snapshot | anything else: file shrank, head changed, SQLite (`*.db` + `-wal`), other JSON rewritten in place | the file split into fixed 1 MiB chunks; only chunks whose hash was not sent before travel |

SQLite without a SQLite library: read the main file and the `-wal` file raw, then read the WAL header
again. If its salt changed (a checkpoint ran during the read), read again (up to 3 times, then note a
gap and try next cycle). The rebuild tool opens the pair and runs an integrity check. A failed snapshot
is replaced by the next one. Because SQLite rewrites pages in place, fixed 1 MiB chunks dedupe well: an
hourly snapshot of a 2 GB database sends only the chunks that changed. Snapshots of a changed database
are taken at most once per 15 minutes, plus at seal. That interval guards upload volume, not what is
recorded: the WAL holds every committed change between snapshots only until a checkpoint, so a
snapshot can miss intermediate states.

Files deleted by the harness keep their last uploaded version. Nothing is ever deleted on the server
because a file vanished locally.

File handles: the watcher never keeps a file open. It opens, reads, closes. The proxy uses two
descriptors per connection. Go raises the soft `RLIMIT_NOFILE` to the hard limit at start, so the macOS
soft default of 256 does not bite the recorder.

What could go wrong: Claude Code background agents under a per-user supervisor may bypass the proxy and
write elsewhere (must be tested with a prompt that asks for background agents; if it bites, set the
proxy variables in the `env` block of the goalcup-owned settings file for that home, allowed by the
docs, and it is goalcup's home, not the user's); a harness that moves its session root after an
auto-update (status shows "no session files for 30 minutes while model calls continue" as a `PROBLEM:`
line); very many small appends (10,000 stats every 5 seconds is about 2,000 per second, well within a
laptop's reach).

### B2.4 Point 4: secrets

One rule above all: **mask in memory before writing; if we cannot mask, we do not store.**

One masker, used for wire records and session files alike. It is length-preserving: a match is
overwritten with `*` of the same length, and a counter per kind is kept. Same length means SQLite pages
stay valid, append cursors stay true, and 1 MiB chunk hashes stay stable for unchanged data. The kinds
and counts go into an event (`masked: {github_pat: 2}`).

Rules (data file, about 60 lines):

1. Header names: `authorization`, `proxy-authorization`, `cookie`, `set-cookie`, `x-api-key`, and any
   name containing `key`, `token`, `secret`, `password` or `auth` -> value masked.
2. `Bearer ` and `Basic ` values in any header.
3. URL userinfo, and query values whose name matches rule 1, `sig`, or contains `signature`.
4. URL password rule in bodies (`scheme://user:pass@host`) (owner decision).
5. Body patterns: every existing secret pattern (provider keys, GitHub tokens incl. `github_pat_`,
   cloud keys, private key blocks, and so on). Not email, not rate-limit fields (owner decision).
6. Run secrets: every value of 8 or more characters from `.env` and `.env.*` files in the workspace at
   start, plus the harness login tokens read from header values as they pass. Each is matched raw,
   base64, base64url, URL-encoded and JSON-escaped. Values live in memory only.

Split secrets in streamed replies: after a response ends, the masker builds a joined view of all JSON
string values in SSE `data:` lines (and WebSocket text messages), in order, keeping a map from each
joined offset back to the raw byte range. It runs the patterns on the joined view and masks the
matching raw ranges. This catches a key the model repeats in 30 small deltas. About 80 lines. It fails
on keys that cross a JSON escape, which secret alphabets almost never contain.

Session files: append mode cuts at a newline, so a JSONL line is masked whole. Snapshot mode masks in
1 MiB windows with a 512-byte overlap, before chunking.

What is never read or sent: login files in the harness home. Each connector lists exclude names
(`auth.json`, `.credentials.json`, `.env`, `secrets.json`, keychain links). The watcher only reads
under the session root, never the whole home.

What could go wrong: a secret in a format no pattern knows is recorded (same as today; raw records stay
private); a secret that spans two SQLite overflow pages in a snapshot is missed; length-preserving
masking leaks the secret's length (acceptable; say so in the rules); a false positive masks harmless
text (a long hex hash looks like a key; the count shows it).

### B2.5 Point 3: gaps without blocking

- The proxy forwards first and records second. The forwarding path has no lock, no disk write, no
  parse, no allocation per byte beyond `io.Copy` and a non-blocking channel send.
- Every recording step that can fail (parse, decode, mask, write, upload, scan) catches its own error
  and writes one `gap` event: `{t, kind, what, host|path, from, to, bytes_lost_estimate}`. Then it
  goes on.
- Every goroutine runs under `recover()`. A panic becomes a gap and a restart of that loop.
- The recorder process itself is restarted by `goalcup start` on the same port, with a
  `recorder_restart` gap. The harness sees a few refused connections and retries.
- Disk full (owner decision): no special handling. Writes fail, gaps pile up in memory (counted, not
  stored), the harness itself will stop soon after. No pause, no freeze.
- If events cannot be written either, the counters still reach the server in the next pack header.

Gap kinds (closed list, about 12): `tee_overflow`, `parse_error`, `undecodable_body`,
`client_tls_reject`, `upstream_tls_error`, `write_error`, `scan_error`, `snapshot_torn`,
`recorder_restart`, `suspended` (sleep), `upload_backlog` (informational), `clock_jump`.

Sleep: the status loop ticks every 5 seconds. If the wall clock moved more than 60 seconds between
ticks, it writes `suspended {from, to}`. TCP connections that died during sleep fail in the harness,
which retries through the same pipe.

### B2.6 On-disk record format

Folder per part: `$GOALCUP_HOME/runs/<run_id>/part-<n>/`

| File | Content | Life |
|---|---|---|
| `wire-000001.jsonl.zst` ... | one JSON line per exchange or WebSocket message, zstd (level 3, long window 128 MiB), flushed every 2 s, rotated at 256 MiB uncompressed or 10 min | deleted after the server confirms the pack that carries it |
| `events.jsonl` | lifecycle, `conn`, `gap`, `masked`, `suspended`, part start facts | append only, uploaded in append mode |
| `state.json` | next pack seq, cursors per file (size, head hash, offset), chunk hashes sent, written by temp + fsync + rename | until seal upload done |
| `outbox/<seq>.pack` | at most one pack waiting to be sent | deleted on 2xx |
| `ca.pem`, `bundle.pem` | public certs for the harness env | part life |

Wire record (one line):

```json
{"v":1,"id":"c17.3","conn":17,"n":3,"t0":"2026-11-07T00:01:02.123Z","t1":"2026-11-07T00:01:09.880Z",
 "host":"api.example-provider.com","port":443,"method":"POST","path":"/v1/messages",
 "status":200,"req_h":[["content-type","application/json"],["authorization","********"]],
 "res_h":[["content-type","text/event-stream"]],
 "req":"{...}","res":"event: ...","enc":{"req":"utf8","res":"utf8"},
 "masked":{"bearer":1},"err":null}
```

WebSocket message: `{"v":1,"id":"c22.ws.118","conn":22,"dir":"up","op":"text","t":"...","data":"..."}`.
Bodies are UTF-8 strings when valid, else base64 with `"enc":"b64"`.

Event line: `{"t":"...","kind":"conn","host":"github.com","port":443,"up":5123,"down":88231,"t0":"...","t1":"..."}`.

Paths in events and packs are relative to the harness home (`home:projects/.../agent-1.jsonl`) or the
workspace (`ws:...`). This keeps the user name out of IDs without losing order or identity.

Local disk use is only what has not been uploaded yet, plus the session files the harness keeps anyway.
A week with no network at all could grow the wire folder by tens of GB (**unverified**: no compression
ratio measured).

### B2.7 Point 5: upload, server storage, rebuild

```
PUT /v1/runs/{run}/parts/{part}/packs/{seq}
Authorization: Bearer <run upload token>          (issued when the run is approved)
Content-Type: application/x-goalcup-pack
X-Pack-Sha256: <hex>
body: pack (<= 64 MiB)
-> 201 created | 200 same seq and same sha already stored | 409 same seq, different sha
-> 413 run over storage limit | 401/403 token | 5xx retry
```

Pack layout: a 4-byte header length, a JSON header, then payload bytes.

```json
{"run":"r_abc","part":2,"seq":4411,"t":"...","final":false,
 "status":{"calls":18204,"ws_msgs":0,"files":2113,"gaps":1,"masked":7,"pending_bytes":0,
           "harness_ua":"<user agent seen last>","deadline_ok":true},
 "entries":[
   {"path":"wire-000123.jsonl.zst","kind":"append","off":0,"len":5242880,"at":0},
   {"path":"home:projects/x/s1/subagents/agent-9.jsonl","kind":"append","off":81920,"len":4121,"at":5242880,"z":true},
   {"path":"home:state.db","kind":"snap","size":2147483648,"chunks":["h1","h2","..."],"new":{"h7":[5247001,1048576]},"z":true},
   {"path":"events.jsonl","kind":"append","off":9911,"len":800,"at":6300000}]}
```

`at` is the payload offset; `z` means that slice is its own zstd frame. Wire segments are already
compressed, so they go in as they are.

Client rules:

- Build the next pack only when the outbox is empty. So at most one pack is waiting. Everything else
  stays where it already is. No queue to manage.
- Write the pack to `outbox/`, then save cursors and the new seq in `state.json`, then send. A crash
  at any point re-sends the same seq with the same bytes, which the server accepts as a duplicate.
- Send every 60 seconds when anything is new, at once when 64 MiB is pending, and an empty pack (header
  only) every 5 minutes as the heartbeat.
- Each request has a 10 minute timeout. Retries back off from 2 s to at most 60 s, forever. After
  sleep or a Wi-Fi change, the next try just works.
- The 64 MiB pack size guards the Workers request body limit of 100 MB on Free and Pro plans. It does
  not limit what is recorded: bigger backlogs just make more packs.

After the harness exits or the deadline passes, the recorder seals: one last scan, last snapshots, the
workspace git HEAD and a digest of uncommitted changes, then a pack with `"final": true`. It keeps
uploading for up to one hour, then exits. Any later `goalcup` command resumes from `state.json` and
`outbox/`.

**Server storage.** The Worker does four checks (token matches run, part is open or within 24 h after
the end, seq is a number, sha matches body while streaming) and stores the body unchanged at
`runs/{run}/p{part}/{seq:08}.pack` in object storage, with the server receive time as object metadata.
It then writes `runs/{run}/latest.json` with the pack header's `status` and the receive time. Emails
("nothing heard for 20 minutes", "no model calls for 3 hours") read `latest.json` from a cron. No
database rows per entry. The per-run byte limit (`RUN_MAX_COMPRESSED_BYTES`, 64 GiB today) guards
storage cost and says so in its 413 body. The owner picks the number.

**Rebuild by ID and time.** The rebuild tool (organizer side, offline) lists `runs/{run}/p*/` in
(part, seq) order and replays: `append` entries write `len` bytes at `off` into file `path`
(part-scoped), gaps in offsets reported, not hidden; `snap` entries assemble the file from chunks, a
chunk hash not in this pack is looked up in the map of earlier packs; wire segments are decompressed
and their records sorted by `t0`, then `conn`, then `n`; the timeline is wire records + events + file
appends (time = pack `t` for appends), each with client time and server receive time. IDs: run id,
part number, file path, exchange id `c<conn>.<n>`. Times: client UTC per record, plus the server
receive time per pack, so a wrong laptop clock is visible.

### B2.8 Point 6: what the participant sees

At start, before the harness opens:

```
Goal Cup is recording run r_abc, part 1. Deadline: Sat 14 Nov 2026 03:12 UTC.
We record: every request to model hosts (*.anthropic.com and 31 other model hosts) and its reply,
           every file the harness writes under ~/.goalcup/homes/claude-code-cli/projects,
           the address, port and size of every other connection (never its content).
We mask: login headers, keys and tokens, the values in your .env files. Details: goalcup show
We never: change your harness settings, send your agent anything, read your own ~/.claude.
```

`goalcup status` (any terminal, any time):

```
Run r_abc part 2, running 3 d 4 h, 4 d 20 h left.
Model calls recorded: 18,204 (last 12 s ago). Hosts: api.anthropic.com.
Session files recorded: 2,113 (2,090 under subagents/).
Other connections seen, not opened: github.com 2,113, registry.npmjs.org 400, pypi.org 37.
Uploaded: all but 0 B. Last upload 41 s ago.
Masked: 7 values (bearer 5, github_pat 2).
Gaps: 1. Tue 10 Nov 01:02 to 06:40, laptop asleep.
PROBLEM: none
```

`PROBLEM:` lines come from five checks only: no model call for 30 minutes while the harness process
is alive and using CPU; no upload success for 30 minutes; recorder restarted; a gap other than
`suspended`; a non-model host with more than 50 MB down that matches the vendor of the harness (a model
host we may be missing). At the end of the part the same block prints once more.

### B2.9 Point 7: how a harness is added

A connector is one TOML file. No code.

```toml
id = "claude-code-cli"
command = ["claude"]                      # chat mode, as people run it
home_env = "CLAUDE_CONFIG_DIR"            # where the harness keeps its state; goalcup owns this folder
session_root = "projects"                 # relative to home; watched recursively
exclude = [".credentials.json", "*.lock"]
model_hosts = ["anthropic.com", "claude.ai", "claude.com"]
version = ["claude", "--version"]
ca_env = ["NODE_EXTRA_CA_CERTS"]          # documented; the common set is always added
no_proxy = []                             # loopback is excluded by default
banned_flags = ["--no-session-persistence"]
```

Starting points for the other four (all need the test below before Friday):

| Harness | home_env | session_root | model_hosts | Notes |
|---|---|---|---|---|
| Codex CLI | `CODEX_HOME` | `sessions` plus `*.sqlite` at home root | `openai.com`, `chatgpt.com` | WebSocket by default; native binary |
| Hermes Agent | `HERMES_HOME` | `state.db` (+ `-wal`) | depends on provider set in its wizard, plus the shared list | SQLite snapshot mode only |
| OpenCode | `XDG_DATA_HOME` (+ `XDG_CONFIG_HOME`) | `opencode/` | per configured provider | loopback must bypass proxy; Bun CA handling **unverified** |
| Amp | **unverified** | **unverified** | `ampcode.com` (**unverified**) | threads may live server-side; then wire only |

Test to add or re-check a harness (also run by `goalcup doctor --harness X`, about 100 lines):
1. Launch it through the recorder with the participant's real login, in chat mode.
2. Type a prompt that asks for two subagents and one tool call.
3. Pass when: at least one decrypted exchange to a model host, at least one new session file, no
   `client_tls_reject` from the harness itself, and the status prints sane numbers.

Releases may ship new connectors during the cup; a connector is data, so the risk of a release is low.

### B2.10 Surviving the week

| Event | What happens | Code needed |
|---|---|---|
| Laptop sleeps 6 h | All processes freeze. On wake, open TCP connections are dead; harness retries through the pipe; status loop writes `suspended`. Upload resumes on next tick. | about 10 lines (clock jump check) |
| Wi-Fi change, VPN on/off | Upstream connections drop. Each new CONNECT dials fresh. No pooled upstream connections exist to go stale. | none |
| Provider outage, 429, 5xx | Recorded as ordinary exchanges with their status. Dial failures answer 502 to CONNECT, recorded as `conn_error`. | none |
| Server outage | Outbox holds one pack; data stays in place; retry forever, 60 s max backoff. | none |
| Harness auto-update | Running process unchanged. New version runs at next `goalcup continue`. Version is seen in every request's user agent and in the part start `version` output. New hosts under the same vendor suffix stay decrypted. New session layout under the same root is still watched. | none |
| Harness update moves state outside session root | Native capture stops; `PROBLEM:` line after 30 min. | covered by status check |
| Thousands of subagents | Wire: more exchanges, nothing per subagent. Files: more stats per cycle. No caps. | none |
| Many parallel streams | Two descriptors each; soft limit already raised. Memory: bodies in RAM up to 32 MiB each then spill. | about 15 lines (spill) |
| Recorder panic | Goroutine recovers; process crash is restarted by supervisor on the same port. | about 30 lines |
| Supervisor (`goalcup start`) dies | Harness and recorder keep running. Recorder sees harness pid gone and seals. | about 10 lines |
| Terminal closed | Harness dies (SIGHUP). Recorder seals. `goalcup continue` starts the next part. | none |
| Deadline | Recorder sends SIGTERM to the harness process group, SIGKILL after 30 s, seals. Server clock wins; local clock used only offline. | about 30 lines |
| Clock wrong on laptop | Server receive time per pack shows the skew. | none |
| Disk full | No special handling (owner). Gaps counted; harness stops by itself. | none |

### B2.11 What B2 deliberately leaves out

| Left out | Why it is safe to leave out | Risk |
|---|---|---|
| PTY wrapper and keystroke recording | Human messages reach the model and the session file; both are recorded. Commands typed into the harness (`/model`) appear in session files for Claude Code and Codex (**unverified** for the others). | A typed command that never reaches the model or a file is invisible. |
| Any client-side decoder per harness or provider | The rebuild/verifier side can decode later from raw records. Client stays harness-agnostic. | Status cannot say "12 subagents" for SQLite harnesses, only "file changed". |
| Pausing or freezing the agent on low disk | Owner decision. | A full disk loses data. |
| Version sampler | User agent header carries the version on every call. | A harness with a generic user agent: version only at part start. |
| Caps on subagents, streams, files, bytes | Vision rule. Only memory (spill at 32 MiB) and the Worker body size (64 MiB packs) are bounded, and both lose nothing. | Memory spikes with very many huge parallel requests. |
| inotify / FSEvents | Polling is simple, the same on all three systems, and survives sleep. | Up to 5 s delay; a file written and deleted inside 5 s is missed. |
| HTTP/2 and gRPC to the client | Not needed by known harnesses. | A future harness requiring h2 fails at the start check. |
| Byte-for-byte check of subagent files | Owner decision. | None for the seven points. |
| Server database per entry | Object storage plus `latest.json` is enough to rebuild and to email. | Ad hoc queries need the rebuild tool. |
| Decrypting non-model hosts | Keeps secrets and size down. | A model call to an unknown host is a blind tunnel (seen as host + bytes only). |
| System-wide proxy, desktop apps, native Windows | Rules exclude them. | None. |
| Agent-facing tools (`time-left`), seal refusal inside the agent | Event plumbing, not one of the seven points. Kept small if the event needs them (about 60 lines). | None for capture. |

### B2.12 Honest size estimate

Language: Go. One static binary per OS and architecture with plain `GOOS`/`GOARCH` builds, a mature TLS
and HTTP standard library, goroutines with `recover()`, and Go raises the file limit itself. Rust
would work too, at perhaps 1.3 times the lines; the design does not depend on the language.

| Component | Src lines | Test lines | Notes |
|---|---|---|---|
| CLI and supervisor (`login`, `start`, `continue`, `seal`, `status`, `show`, `upload`, `doctor`) | 450 | 300 | run open/approve uses the existing server auth flow; not counted beyond glue |
| Proxy: CONNECT, tunnel, MITM, CA and leaf certs, plain HTTP | 250 | 250 | |
| Tee buffer, HTTP/1.1 parser on the copy, WebSocket frames, permessage-deflate, body decoding | 330 | 400 | biggest correctness risk; most tests here |
| Masker (headers, URL, patterns, run secrets, joined stream view) | 250 | 350 | rules data file about 60 lines not counted |
| Wire segment writer (zstd, flush, rotate) | 80 | 60 | |
| Watcher (walk, stat, append cursors, SQLite raw snapshot, chunking) | 250 | 250 | |
| Uploader (pack build, outbox, state file, retry, heartbeat) | 280 | 250 | |
| Status, events, gaps, sleep check, deadline stop | 180 | 120 | |
| Connectors | 0 | 0 | 5 TOML files, about 20 lines each |
| **Client total** | **about 2,070** | **about 1,980** | |
| Worker: pack PUT, `latest.json`, status GET, cron emails | 250 | 250 | run open, approval, deadline and GitHub checks are event logic; estimate +400 src if rewritten |
| Rebuild tool (packs to files, wire timeline, gaps) | 250 | 200 | verifier rules and decoders are separate and not counted |
| **Capture system total** | **about 2,570** | **about 2,430** | plus about 400 src for run lifecycle on the server |

These are estimates from experience, not measurements; the real figure is expected within plus or
minus 50 percent. Third-party packages: one zstd, one brotli, one TOML. Nothing else.

### B2.13 Failure modes of design 2

| # | Failure | Likelihood | Effect | Seen by | Mitigation |
|---|---|---|---|---|---|
| 1 | Harness ignores `HTTPS_PROXY` (or a provider SDK inside it sets `trust_env=False`) | medium for Amp and Hermes plugins, low for others | no wire record; native only | start check, `PROBLEM:` no model calls | connector test before Friday; reverse-proxy base URL mode is a later add (about 60 lines) |
| 2 | Harness rejects our CA (Bun not reading `NODE_EXTRA_CA_CERTS`, cert pinning) | medium for OpenCode, unknown for Amp | harness cannot reach model: **blocks** | start check fails before approval | add `SSL_CERT_FILE`/`BUN_OPTIONS=--use-system-ca` per connector; test |
| 3 | Agent's own tool calls a model host with a stack that ignores our CA | medium (Go tools on macOS, Java) | that tool call fails; agent product work hurt | `client_tls_reject` events | document; participant can add the host to a per-run "tunnel only" list at setup, which is disclosed |
| 4 | Claude Code background agents run under a supervisor without our env | unknown | subagent calls and files missed | file count vs call count mismatch | test; set env in goalcup-owned settings file |
| 5 | Recorder process dies | low | harness gets refused connections for about 1 s; exchanges in flight lost | `recorder_restart` gap | supervisor restart on same port |
| 6 | Both supervisor and recorder die | very low | harness cannot reach model until `goalcup continue` | server email after 20 min silence | none cheaper; rules already allow continue |
| 7 | Parser misreads an odd HTTP/1.1 stream (pipelining, odd chunking) | low | that connection's later records lost | `parse_error` gap | pipe unaffected |
| 8 | Unknown content encoding | low | body not stored | `undecodable_body` gap | add decoder |
| 9 | Model call to a host not in the decrypt list | medium for custom gateways | call seen as host and bytes only | `conn` events, `PROBLEM:` big vendor host | participant names gateways at setup |
| 10 | Local model on loopback | low in this event | not on wire | none on wire; session files still record | connector may remove loopback from `NO_PROXY` where safe |
| 11 | SQLite snapshot misses states between snapshots | high for Hermes/OpenCode, by design | intermediate DB states lost; final state kept; wire has every call | none needed | wire is the primary record for those harnesses |
| 12 | Torn SQLite read three times in a row | low | that snapshot skipped | `snapshot_torn` gap | next cycle |
| 13 | Secret split across a JSON escape or a SQLite page boundary | low | secret recorded | verifier flag | private raw records; clean before sharing |
| 14 | Unknown secret format | medium | secret recorded | verifier flag | add pattern; same as today |
| 15 | Large backlog after days offline | low | local disk grows; long catch-up | `pending_bytes` in status | none; owner chose no disk handling |
| 16 | Compression ratio worse than hoped | unknown | upload volume high, may hit 64 GiB run limit | 413 from server, status line | measure on a long test run; tune segment size |
| 17 | File written and deleted inside one 5 s poll | low | file missed | none | acceptable; wire still has the calls |
| 18 | Harness update moves its session root | low within a week | native capture stops | `PROBLEM:` line after 30 min | watch broader root |
| 19 | Memory spike: many parallel huge requests | low | spill to temp files; more disk I/O | none | spill threshold |
| 20 | Wrong system CA bundle path on some Linux | low | tools inside the agent fail TLS | start check runs `curl` to a model host | built-in fallback bundle |
| 21 | Laptop clock wrong | low | client times off | server receive time | rebuild uses both |
| 22 | Two runs share a harness home | low | files mixed | lock file in home; second start refuses | none more |
| 23 | A harness that requires HTTP/2 to its model host | low today | harness cannot connect: **blocks** | start check | add h2 later only if needed |

Rows 2 and 23 are the only ones that can block the harness, and both are caught by the start check
before a competition run is approved. Row 6 blocks only after two independent crashes.

### B2.14 What must be tested before Friday 6 November

1. Each of the five harnesses through the proxy, with the participant's real login, chat mode, a
   prompt that asks for subagents (never forced by config). Pass rule in B2.9.
2. Codex over WebSocket with `permessage-deflate`: records decode and secrets are masked.
3. Claude Code with a prompt that asks for background agents (row 4).
4. A 24-hour run with a real lid-close sleep and a Wi-Fi switch, then a rebuild that matches.
5. Compression ratio of a long run's wire segments. Nothing in the estimates is measured. The 2-hour
   run report (`docs/research/results/2026-10-30-two-hour-run.md`) does not exist on any branch yet,
   per the brief, so it could not be used.
6. Open-file use with 200 parallel subagents on macOS.
7. Kill `-9` the recorder mid-stream: harness recovers, gap recorded, seq resumes.

---

## B.6 Comparison with the existing code (Appendix A)

Line counts below are production src from Appendix A unless marked. "D1" is design 1 (B1), "D2" is
design 2 (B2). "Present in" means the design has a part that does the same job, even if much smaller.

### B.6.1 Area by area

#### CLI proxy and launch (`crates/goalcup-cli`)

| Existing area (src lines, Appendix A) | D1 | D2 | Present in | What it suggests |
|---|---|---|---|---|
| CONNECT, SNI, intercept or tunnel, upstream TLS (`proxy/connect.rs` 1,281, `proxy/tls.rs` 295) | yes | yes | both | **Keep.** The core both designs chose. Existing ALPN is already `http/1.1` only (`proxy/tls.rs:213,275`), as in both designs. |
| Proxy lifecycle with a 1,024-connection cap (503) and 5 restarts (`proxy/mod.rs` 1,498) | yes, no cap | yes, no cap; supervisor restarts recorder | both (without caps) | **Change.** Remove `MAX_CONNECTION_TASKS` and `MAX_RECORDER_RESTARTS` (`proxy/mod.rs:110-121`, `:1022`): neither design has them, and both block the harness (point 3). |
| HTTP/1.1 relay, bytes unchanged, decode gzip/br/zstd/deflate (`proxy/http.rs` 2,052) | yes, gzip only via `Accept-Encoding` rewrite | yes, all four, bytes unchanged | both | **Keep, shrink.** Existing matches D2's rule. D2 budgets 330 lines for tee, parser, WebSocket and decoding together; 2,052 suggests heavy over-build, but read before cutting. |
| WebSocket relay with `permessage-deflate` (`proxy/websocket.rs` 1,019) | yes, extension stripped, about 90 lines | yes, inflater per direction | both | **Keep, shrink.** D1's header strip would delete the inflater but changes forwarded bytes. |
| Header masking and allowlisted metadata (`proxy/headers.rs` 267, `proxy/metadata.rs` 254) | yes | yes | both | **Keep, merge** into the one masker both designs have. |
| CaptureEmitter that orders frames without blocking relays (`proxy/api.rs` 824) | partial (per-exchange buffer) | partial (non-blocking tee + parser) | both, smaller | **Change.** Both designs record per finished exchange, not per frame stream. |
| IPC sink to the daemon's ingest port, gives up after 10 failures (`proxy/ingest.rs` 1,136) | no | no | neither | **Cut** if the recorder joins the proxy's process. D2 keeps two processes but puts proxy and recorder in the same one, so no IPC either. |
| Outage windows, keeps 32 (`proxy/outage.rs` 144) | gap `upstream_error` | `conn_error` event, 502 to CONNECT | both, as plain events | **Change** to one event per failure; drop the coalescing. |
| Provider reachability probe before start (`proxy/preflight.rs` 250) | no | yes (start check, `curl` to a model host) | D2 | **Keep small.** D2 relies on a start check to catch the only two blocking failures. |
| Tunnel list capped at 10,000 rows (`proxy/tunnels.rs` 192) | yes, one record per tunnel, no cap | yes, `conn` event per tunnel, no cap | both | **Keep, remove cap**, write per connection instead of a list at finish. |
| Model host read from the harness's own config, D62 (`model_host.rs` 893) | no (setup asks for custom hosts) | no (setup names gateways; suffix list) | neither | **Keep a small version.** See B.6.2 item 4. |
| CA bundle: proxy CA plus OS roots (`ca_bundle.rs` 207) | yes | yes | both | **Keep.** |
| Launch: argv and env from nothing, CA files, after-exit hooks (`launch.rs` 1,994, of which D40 spawn gate about 340) | yes, no gate | yes, no gate | both (no gate) | **Change.** Keep env building; cut the park-before-exec gate. |
| Shared harness homes, lock, repair modes (`home.rs` 700) | yes (homes under `~/.goalcup/homes`) | yes, plus a lock file | both | **Keep, shrink.** |
| Per-run home from seeds, used by Amp (`per_run_home.rs` 489) | no | no | neither | Point 7 for Amp only. Keep only if Amp ships; both designs mark Amp **unverified**. |
| Host log while a chat harness owns the terminal (`host_log.rs` 253) | no | no | neither | **Keep small.** See B.6.2 item 3. |
| `--version` of the harness (`harness_version.rs` 203) | yes (`meta`) | yes (part start + user agent) | both | **Keep, shrink** to a few lines. Appendix A called it no point; both designs record it. |
| WSL1/WSL2 and filesystem checks (`folder_gate.rs` 156) | records WSL2 and container in `meta` | no | D1 (facts only) | **Keep if cheap.** Supported platforms are a rule (`RULES.md:266-267`). |
| Config dir, `control.json`, `daemon.toml` (`config.rs` 426) | token file only | `$GOALCUP_HOME` | both, tiny | **Shrink** with the daemon. |
| Connector lookup and pin allowlist (`connector.rs` 133) | yes (data entry) | yes (TOML file) | both (no pins) | **Keep lookup, cut pins.** |

#### CLI commands, participant text, run state, upload

| Existing area | D1 | D2 | Present in | What it suggests |
|---|---|---|---|---|
| `login`, device approval (`commands/login.rs` 84, `account/device.rs` 371, `account/credential.rs` 176, `server.rs` 212) | yes | yes (existing auth flow) | both | **Keep.** |
| `start` orchestration and phases (`commands/start/*` about 4,800 total incl. project 363, refusals 229, server 546) | yes | yes | both | **Change.** Keep open, approve, launch, wait; cut project-folder rules, push lines, skill, reserve and pause wiring. |
| `continue` (`commands/continue_run.rs` 417) | yes, kills stale group first | yes | both | **Keep.** |
| `seal` (`commands/seal.rs` 586) | yes, y/N, refuses inside agent | yes | both | **Shrink.** D1's version signals a pid; no daemon round trip. |
| `status` with signed-status check (`commands/status.rs` 764) | yes (reads `status.json`) | yes, with five `PROBLEM:` checks | both | **Change.** Drop the signature check and the pinned key (`entries.rs` 60). |
| `upload` (`commands/upload.rs` 343) | yes | yes | both | **Keep.** |
| `entries` (`commands/entries.rs` 82) | no | no | neither | **Cut** (the run folder lists runs). |
| `doctor` incl. decoder Warn and SSH section (`commands/doctor.rs` 471) | no (practice run) | yes, about 100 lines | D2 | **Shrink** to D2's harness check. |
| `setup` / prep: machine checks (`commands/prep.rs` 1,301) | yes (custom hosts) | yes (gateways, CA at setup) | both, small | **Shrink.** |
| `mcp` time-left tool (`commands/mcp.rs` 273) and `time-left` (`commands/time_left.rs` 117, `run/time_left.rs` 260) | `time-left` about 15 lines | left out (about 60 if needed) | D1 (tiny) | **Cut the MCP server; keep at most a tiny `time-left`** (rule, not point). |
| Participant wording (`participant/*` 1,788) | one paragraph, status, summary | same | both | **Keep, shrink.** Both designs show point 6 in under 150 lines. |
| Run state, native-silence and no-model-calls notes (`run/mod.rs` 1,137) | `status.json` | `status.json` + `PROBLEM:` checks | both | **Keep, shrink.** The "no model calls" and "native silent" notes match D2's checks. |
| Disk: reserve file, SIGSTOP pause, vitals, disk parts of watch (`run/reserve.rs` 533, `run/pause.rs` 211, `vitals.rs` 158, about 330 mixed) | no | no | neither | **Cut** (owner decision). |
| Upload chunking, session, heartbeat, finish (`upload/*` 3,956) | segments + heartbeat + seal | packs + empty-pack heartbeat + final pack | both | **Keep, shrink.** Drop the local deadline count (110). |
| Prune acked chain parts (`upload/prune.rs` 173) | no (keeps segments, records `uploaded.txt`) | yes (wire segment deleted after confirm) | D2 | **Keep.** Deleting acked data is not disk handling; a week run needs it. |
| Daemon client, spawn and retry (`daemon/mod.rs` 724, `daemon/spawn.rs` 710, `retry.rs` 474) | no | no (supervisor spawns recorder, no client API) | neither | **Cut** with the daemon. |
| GitHub repo checks, repo question, push lines, SSH-key warning, skill install, arming (`account/github.rs` 466, `commands/run_repo.rs` 197, `push_lines.rs` 152, `ssh_keys.rs` 192, `skill.rs` 86, `arming.rs` 175) | no (server checks repo) | no (event logic on server, +400 if rewritten) | neither | **Cut from the client, or owner call.** Event rules, not points (Appendix A, A.7 blind spots). |
| Subreaper, released child, spawn gate (`subreaper.rs` 354, `released_child.rs` 291, about 340 in `launch.rs`) | no (`killpg`) | no (`killpg`) | neither | **Cut.** |

#### Daemon and platform

| Existing area | D1 | D2 | Present in | What it suggests |
|---|---|---|---|---|
| Separate-process plumbing: control API, ingest, registry, lifecycle, config, binary (5,214) | no | no (a detached recorder, but no control API, IPC or registry; status via file) | neither | **Cut or collapse.** D2 shows the one feature worth keeping from a second process (restart on the same port) costs about 30 lines. |
| Native session capture (`native_capture.rs` 1,632, `native_plan.rs` 311) with caps 1,024 files, 32 streams, 20,000, 128 fresh ids | yes, open-read-close, no caps | yes, open-read-close, no caps | both | **Keep, change.** Delete every cap; poll the whole session root. |
| Handle-rooted no-follow reads (`sourcecap` 1,285) and 32-handle budget (`handle_budget.rs` 79) | no | no (exclude list; reads only under session root) | neither | **Keep a small version** (refuse symlinks); cut the budget. See B.6.2 item 5. |
| Redaction: whole-record and streaming (`redact.rs` 1,208, `redact/stream.rs` 2,945) | yes, about 250 lines, after exchange | yes, about 250 lines, after exchange | both | **Change.** Both designs redact a buffered exchange, which avoids streaming recognizers; keep the existing test corpus as the bar (A.3 row 7: "a secret split across two frames is still caught"). |
| Run secrets in every encoding (`protocol/run_secret.rs` 376: proxy credential and GitHub token, at most 2) | yes, plus `.env` values | yes, plus `.env` values | both, wider | **Change:** add `.env` values and live credential headers. |
| Owner thread, bounded sink, writer with gaps (`owner.rs` 388, `sink.rs` 556, `writer.rs` 391) | one log writer | wire segment writer + events | both, simpler | **Shrink;** remove `MAX_LIVE_PRODUCERS` 128 and `MAX_PRODUCERS` 65,536. |
| Finish: 180 s budget, 7 stages, latch (`finish.rs` 763) | rotate, upload, seal, up to 1 h | final pack, up to 1 h | both, simpler | **Change** to one timeout (A.9 already says so). |
| Termination proof (`terminate/*`, `runtime/termination.rs` 2,428; `protocol/process.rs` 497) | no (`killpg`, 10 s) | no (`killpg`, 30 s) | neither | **Cut.** Risk: `setsid` or double-forked children escape the group. |
| Seal-time workspace walk and hash (`workspace.rs` 485) | no | no | neither | **Cut.** |
| Git commit and dirty digest at seal (`workspace_git.rs` 530) | yes (`end` record) | yes (seal) | both, tiny | **Shrink** to a few lines that run `git rev-parse` and hash `git status --porcelain`. |
| Harness build digest, process image (`harness_build.rs` 89, `process_image.rs` 40) | no | no | neither | **Cut.** |
| Status signed with the run key (`status.rs` 149) | status file | status file | both, unsigned | **Change:** drop the signature. |
| Sleep lock (`platform/awake.rs` 269) | no | no (records `suspended` instead) | neither | **Owner call.** See B.6.2 item 7. |
| Disk free and reserve (`disk.rs` 57 + 182) | free bytes in `meta` | no | D1 (fact only) | **Cut `ReserveFile`.** |
| Machine facts, AC power, container (`machine.rs` 288 no-point part) | yes (OS, CPU, memory, disk, WSL2, container) | no | D1 | **Keep a small facts block** if judging reads it; cut AC power. |
| Terminal sharing for chat TUIs (`terminal.rs` 292) | no | no | neither | **Keep.** See B.6.2 item 3. |
| Handle-rooted trace writes (`tracecap` 1,833) | plain files | plain files | neither | **Shrink** to plain writes with fsync; the agent cannot reach the run folder any more than today (D5 is a rule). |
| Clone rule (`clone.rs` 68) | no | no | neither | **Cut** (event rule). |

#### Protocol and verifier

| Existing area | D1 | D2 | Present in | What it suggests |
|---|---|---|---|---|
| v5 record types and bounds (`capture.rs` 744) | 9 record kinds | wire record + event kinds | both, simpler | **Change:** no freeze, no caps; keep `MAX_CAPTURE_QUEUE_BYTES` only if it guards memory. |
| Hash chain, envelope (`chain.rs` 45, `envelope.rs` 129) | no (sha per segment) | no (sha per pack) | neither | **Owner call.** Tamper evidence, not a point. |
| Trace writer with rotation (`export.rs` 1,872) | gzip segments, 10 min or 32 MB | zstd segments, 10 min or 256 MiB | both | **Keep, shrink** (drop legacy and test-only parts, about 300). |
| Control API types (`control.rs` 1,010) | no | no | neither | **Cut** with the daemon. |
| Server-signed acks and start receipt (`ack.rs` 95, `receipt.rs` 186; Worker `sign.ts` 116) | no (HTTP 200 per segment) | no (201/200 per pack) | neither | **Owner call.** Acks drive upload progress today; both designs use idempotent status codes instead. |
| Workspace manifest (`seal.rs` 140) | git facts only | git facts only | both, tiny | **Shrink.** |
| Connector schema, refusals, trust tables (`harness/*` 3,351, `capture_policy.rs` 1,077, `template.rs` 114, `manifest.rs` 49) | 15-line entry, no validator | 20-line TOML, no validator | both, tiny | **Change: shrink hard.** Keep the rule "a connector may not route around the proxy" (point 1). |
| Connector pins and folder digest (`pins.rs` 520, `connector_dir.rs` 215) | no | no | neither | **Cut.** |
| Sleep-counting clock (`clock.rs` 60) | `clock_jump` gap | `suspended` gap | both, as gaps | **Keep;** add the gap record. |
| Verifier integrity reader (2,408) | rebuild tool about 150 | rebuild tool about 250 | both, different job | **Change.** Both replace "check the chain" with "rebuild the story". Raise the reader's 128 caps in the same change, or cut it with the chain. |
| Verifier research decoders and reports (9,745) | no | no | neither | **Cut from the Friday build** (A.9 agrees); keep a 100 to 200 line call and file count for the test kit. |

#### Worker, scripts and the rest

| Existing area | D1 | D2 | Present in | What it suggests |
|---|---|---|---|---|
| Device flow and rate limits (`device.ts` 146, `ratelimit.ts` 139) | device flow yes; rate limit not mentioned | existing auth reused; rate limit not mentioned | both (flow) | **Keep both.** The rate limit guards a real cost and says so. |
| Runs open, continue, position, abandon, receipt (`runs.ts` 637) | `POST /runs` with approval, part number | run open "event logic", +400 if rewritten | both | **Keep, shrink.** |
| Chunk `PUT`, idempotent, size guard, R2 create-only (`chunks.ts` 228) | segment `PUT` | pack `PUT` | both | **Keep.** Already matches both designs' idea; the R2 key is already path-like (`chunks.ts:157`). |
| Supabase index (`db.ts` 485, `db-supabase.ts` 433, SQL mirror 4,468 data) | small `segments` and `runs` tables | none, `latest.json` | D1 | **Keep for Friday** (it exists and works); D2 shows it could shrink later. |
| Local test backend (`db-local.ts` 701) | no | no | neither | Keep for Friday (only local end-to-end upload test, A.10). |
| Heartbeat with vitals (`runs.ts` heartbeat) | yes | empty pack every 5 min | both | **Keep, drop disk vitals.** |
| Emails and cron (`mail.ts` 70, `notices.ts` 220) | silence email mentioned (rules) | cron reads `latest.json` | both | **Keep;** cut `disk_full`. |
| Judged commit (`github.ts` 223) | no | no (event logic) | neither | Owner call; after Friday (A.10 agrees). |
| `RUN_MAX_COMPRESSED_BYTES` 64 GiB | yes, 413, owner's value | yes, 413, owner's value | both | **Keep;** owner sets the number. |
| Rebuild from server (`scripts/fetch-run.sh` 349 + verifier) | `goalcup admin rebuild` | offline rebuild tool | both | **Change:** output a timeline and files, not only an integrity verdict. |
| Real-run test kit (`matrix.sh` 2,522, `linux-matrix.sh` 876, `campaign.py` 1,888, `kit_settle.py` 302) | one practice run per harness | `doctor` + seven tests (B2.14) | both, tiny | **Keep for Friday;** shrink later (A.10 agrees). |
| Research and disk scripts (`research-*`, `t25-disk-full.sh`) | no | no | neither | **Cut.** |
| Installer with a recording-run guard (`packaging/install.sh` 394) | install script about 60 | plain builds | both, smaller | **Keep;** the guard (about 90 lines) protects point 3. |
| Second independent redactor (`spikes/redact.py` 349) | no (one fixture corpus) | no | neither | **Keep** as a test oracle for point 4. |
| Tests: about 159,500 lines (A.1) | about 1,320 to 2,400 | about 1,980 to 2,430 | | Both designs plan about 1 test line per src line, against 1.6 today. The largest test cuts follow the code cuts (daemon IPC, termination, decoders, the Worker stand-in). |

### B.6.2 What the code has that both designs left out

Rough total of existing production src in areas neither design has (Appendix A numbers; buckets
overlap a little, so treat it as an estimate): separate-daemon plumbing 5,214; daemon client and retry
1,908; termination proof and process code about 3,910 (2,428 + 497 + 354 + 291 + about 340); disk
handling about 1,414; research decoders 9,745; pins and folder digest 735; event-rule commands about
2,280; workspace walk 485; build digest and process image 129. **About 25,800 src lines.** Add the
connector-schema shrink (about 2,000, A.9) and the large proxy files (B.6.1) for the rest of the gap
between about 102,000 today and about 2,500 to 3,800 fresh.

| # | Part the code has | Lines (src) | Serves a point? | Judgement |
|---|---|---:|---|---|
| 1 | Separate daemon with control API, ingest IPC, registry, lifecycle | 5,214 (+1,908 client side) | Carries 1, 2, 5, but only because of the split | **Over-engineering.** Both designs avoid it. D2 shows that the useful part (restart the recorder on the same port) costs about 30 lines. Risk: `status` and `seal` from another terminal must then work through a file and a pid, which both designs do. |
| 2 | Termination proof: process-tree walk, Darwin audit tokens, subreaper, released child, D40 spawn gate | about 3,910 | none (rules: 168 h stop) | **Over-engineering** for the seven points. `killpg` plus grace plus `SIGKILL` meets the rule. Risk: a `setsid` child keeps running after seal; B1 kills any stale group on `continue`. |
| 3 | Terminal sharing for chat TUIs (`terminal.rs` 292) and host log (`host_log.rs` 253) | 545 | 3, 7 | **Serves a point; both designs missed it.** If goalcup stays in the foreground process group while a TUI harness owns the terminal, a Ctrl-C or a stray goalcup message lands in the TUI. In B1's one-process design a Ctrl-C that kills goalcup kills the proxy, so the harness loses its model (point 3). Keep a small version: ignore SIGINT in goalcup, hand the foreground group to the harness, send goalcup's own messages to a log file. |
| 4 | Model host read from the harness's own config, D62 (`model_host.rs`) | 893 | 1, 4 | **Serves a point; both designs replaced it with a setup question.** A participant who set a gateway in `settings.json` or `config.toml` and forgets to name it at setup is recorded tunnel-only (B1 failure 3, B2 failure 9). Keep a smaller reader for the base-URL keys of the shipped harnesses. |
| 5 | Handle-rooted, no-follow reads of session files (`sourcecap` 1,285) | 1,285 | 2, 4 | **Partly serves a point.** Both designs read every file under the session root. An agent (or a tool) that plants a symlink from the session folder to `~/.ssh/id_ed25519` would have it uploaded, masked only if a pattern matches. Keep "refuse symlinks and devices" (a few lines with `lstat`/`O_NOFOLLOW`); the identity and mount machinery is over-engineering. |
| 6 | Hash chain, signed acks, signed receipt, signed status, pinned run key | about 1,000 (chain, envelope, ack, receipt, status signing, `entries.rs` pinning, `sign.ts`) | none (tamper evidence, README P2) | **Over-engineering for the seven points**, but it is the event's anti-tamper story. Owner call. Signed status and the pinned key are the weakest part (A.9: low value, the agent can already reach the daemon). |
| 7 | Sleep lock (`awake.rs`) | 269 | indirect 1 | **Not a point.** A sleeping laptop pauses harness and recorder together, so nothing goes unrecorded; both designs log a gap instead. It helps the participant's score, not the record. Cheap; owner call. |
| 8 | Disk reserve, pause, vitals | about 1,414 | none (owner dropped) | **Cut.** Both designs agree with the owner. |
| 9 | Event-rule commands (GitHub repo check, project folder, push lines, SSH warning, skill install, MCP time-left, arming) | about 2,280 | none (rules) | **Over-engineering for the points.** Some are rules (`RULES.md`). Move what must stay to the server (repo check) or keep a tiny `time-left` like D1. |
| 10 | Workspace walk and hash at seal | 485 | none | **Cut;** both designs keep only git HEAD and a dirty digest. |
| 11 | Research decoders and reports | 9,745 | none (measures 1, 2) | **Cut from the Friday build.** Both designs move decoding to review time. |
| 12 | Connector pins, folder digest, 1,580-line validator, 1,077-line trust tables | about 4,300 | partly 1 and 4 | **Mostly over-engineering.** Keep only the rule that no connector env routes around the proxy and no key sits in a seed. |
| 13 | Per-run seeded home for Amp | 489 | 7 (Amp only) | Keep only if Amp ships. |
| 14 | Outage-window coalescing | 144 | 3 | **Over-built;** one event per failure does the job. |
| 15 | Second independent redactor (`spikes/redact.py`) | 349 | 4 (testing) | **Serves a point as a test oracle.** Both designs left out an independent check. Keep it. |
| 16 | Installer guard that will not replace a recording goalcup | about 90 | 3 | **Serves a point.** Keep. |
| 17 | Hermes `launch_prep` that points provider `recorder` at the proxy (`connectors/hermes-cli/harness.toml`) | data | 1 | Both designs rely on `HTTPS_PROXY` alone for Hermes, with `trust_env` **unverified**. Keep the existing route until measured. |
| 18 | Large real-run test kit and 159,500 test lines | 5,588 src kit, 4,071 kit tests | 7 (testing) | Keep the kit for Friday; the test volume shrinks with the code. |

### B.6.3 What both designs have that the code lacks

| # | Both designs have | Today (Appendix A) | Point | Note and risk |
|---|---|---|---|---|
| 1 | A shared list of model-provider hosts (B1: one list plus cloud wildcards; B2: about 30 suffixes plus connector suffixes) | Decrypts only the hosts a connector declares, plus the host read from the harness config (`docs/DECISIONS.md:59` D1; `AGENTS.md:4`) | 1 "to any provider" | Closes "several providers ... the largest real gap" (`docs/plans/2026-10-29-record-everything-phase2-notes.md:482-484`). Risk: a tool that ignores the CA and calls a listed host fails (B2 failure 3). |
| 2 | No caps of our own: open, read, close per poll; no 128, 32, 1,024, 20,000 or 128 fresh-id limits; no 503; no restart limit | Caps in four crates that must move together (A.4) | 2, 3 | Must also change the verifier reader, or long runs read as invalid (`reader.rs:1567,1695`) if the reader stays. |
| 3 | Joined-view masking of a secret split across streamed deltas | Not reliably caught (`docs/RULES.md:292-294`) | 4 | B1: per connection; B2: per response, about 80 lines. Fails on keys across a JSON escape (B2). |
| 4 | Exact masking of workspace `.env` values (8+ characters) and live credential header values, in 5 encodings | Run secrets are the proxy credential and GitHub token only, at most 2 (`protocol/run_secret.rs`) | 4 | Risk: over-masking short common values (B1 failure 7). |
| 5 | URL password rule in bodies; no email rule; no Codex rate-limit rules | Email rule at `redact.rs:279-287`, Codex rows at `connectors/codex-cli/harness.toml:110-111`; URL password rule missing (A, finding 10) | 4 | This is the owner's option b. |
| 6 | SQLite snapshots during the run (B1 every 10 min max; B2 every 15 min max with chunk dedupe) | Hermes `state.db` copied once at the end (`final_snapshot`) | 2 | A hard death today loses Hermes subagent rows (A.10 section 2). Also opens a path for OpenCode, whose sessions are SQLite only (branch facts sheet). |
| 7 | Watch the whole session root, any name, any depth | Declared native rows with patterns; `warn_missing = false` on subagent rows | 2 | Survives a harness that renames files. Risk: more bytes; B2 excludes login files by name. |
| 8 | A sleep or clock-jump gap (`clock_jump`, `suspended`) | A sleep lock and a sleep-counting clock; a sleep gap record is not listed in Appendix A (**unverified**) | 3, 5, 6 | Plain participant line: "laptop asleep" (B2 status). |
| 9 | A rebuild tool that outputs a timeline and whole files | `fetch-run.sh` plus `goalcup-verify --capture-integrity` (integrity verdict); server verify job not built (`docs/SERVER-DESIGN.md:157-159`) | 5 | Both tools are still offline; neither design builds the server-side job either. |
| 10 | A "large unknown host" flag (B1: unknown API-looking host; B2: a non-model host over 50 MB matching the harness's vendor) | Tunnels are listed, not flagged (`proxy/tunnels.rs`) | 1, 6 | Cheap detector for a missed provider. |
| 11 | No hash chain; one sha256 per segment or pack | Hash chain plus signed acks | 5 | Simpler; loses tamper evidence (B.6.2 item 6). |
| 12 | Plain HTTP requests through the proxy recorded | Not stated in Appendix A (**unverified**) | 1 | Needs a local model server reachable through the proxy; B1 and B2 disagree on loopback (B.2 row 3). |
| 13 | A start or practice check that a harness actually trusts the CA before a competition run | Provider reachability probe (`proxy/preflight.rs`) checks TCP, not trust (A.7) | 3, 7 | The only two blocking failures in B2 (rows 2, 23) are caught here. |
| 14 | A per-kind masked count shown to the participant (B2) | Not in Appendix A (**unverified**) | 6 | "Masked: 7 values (bearer 5, github_pat 2)". |

### B.6.4 Risks of acting on this comparison

- **Neither designer read the code.** "Present in both" means the job is done, not that the existing
  file is the right size. Every "shrink" needs a reading of the file first.
- **Language.** Both designs are in Go; the code is Rust. B1 expects 1.5 to 2 times the lines in Rust,
  B2 about 1.3 times. A language change before 6 November is not on the table; use the designs as a
  target shape.
- **Rules versus points.** Termination, the GitHub checks, the workspace manifest and the signed
  receipt serve event rules in `docs/RULES.md`, not the seven points. Cutting them moves the rule to
  the server or drops it. That is an owner decision, not a design one.
- **Unmeasured harness facts.** OpenCode's CA handling under Bun, Amp's proxy support, Hermes's
  `trust_env` and the Claude Code background-agent supervisor are **unverified** in both designs. The
  existing connectors carry measured facts (for example Hermes's `launch_prep`) that a cut must not
  lose.
- **No measured long run.** The 2-hour run report does not exist. Compression ratios, memory under
  thousands of subagents and macOS file-handle use are unmeasured in both designs and in the code.

## B.7 Sources (merged, deduplicated)

- https://code.claude.com/docs/en/network-config (proxy, CA, background agent supervisor, gzip bodies)
- https://frederick-douglas-pearce.github.io/blog/2026/inside-the-subagent-trace-file/ (third-party)
- https://cdn.jsdelivr.net/npm/oh-my-opencode@4.18.2/dist/skills/coding-agent-sessions/references/claude.md (third-party)
- https://cdn.jsdelivr.net/npm/oh-my-opencode@4.18.2/dist/skills/coding-agent-sessions/references/codex.md (third-party)
- https://learn.chatgpt.com/docs/config-file/environment-variables (`CODEX_HOME`, `CODEX_SQLITE_HOME`, `CODEX_CA_CERTIFICATE`, `SSL_CERT_FILE`)
- https://codex.danielvaughan.com/2026/05/08/codex-cli-tls-inspecting-proxies-custom-ca-certificates-enterprise/ (third-party)
- https://codex.danielvaughan.com/2026/04/18/codex-cli-proxy-configuration-socks-http-corporate-networks/ (third-party)
- https://codex.danielvaughan.com/2026/04/29/codex-cli-rollout-files-session-recording-replay-audit-trails/ (third-party)
- https://codex.danielvaughan.com/2026/06/01/codex-cli-session-history-local-search-rollout-format-knowledge-mining/ (third-party)
- https://codex.danielvaughan.com/2026/05/03/codex-cli-websocket-mode-responses-api-persistent-connections-agent-loop-performance/ (third-party)
- https://luna-shi.github.io/blog/codex-source-dive-subagents/
- https://developers.openai.com/codex/subagents (official)
- https://github.com/openai/codex/issues/13041 (WebSocket fallback)
- https://www.python-httpx.org/environment_variables/
- https://hermes-agent.nousresearch.com/docs/developer-guide/session-storage (official)
- https://gormes-docs.pages.dev/upstream-hermes/session-storage/ (third-party)
- https://skills.cat/skills/nousresearch/hermes-agent/hermes-agent-spawning (third-party)
- https://github.com/NousResearch/hermes-agent/issues/5563 (state.db corruption report)
- https://bun.com/guides/http/proxy
- https://bun.com/docs/guides/http/proxy
- https://opencode.ai/en/docs/network
- https://opencode.ai/docs/network/
- https://github.com/getsentry/cli/pull/901 (Bun and `NODE_EXTRA_CA_CERTS`)
- https://blog.devgenius.io/agent-session-persistence-in-opencode-9c3440448c06
- https://forums.basehub.com/anomalyco/opencode/15
- https://pkg.go.dev/github.com/mrf/agentwatch@v0.2.0/sources/opencode (third-party)
- https://dev.to/whtsky/garbage-collecting-opencodes-24-gb-session-database-7b5 (third-party)
- https://nodejs.org/en/learn/http/enterprise-network-configuration (`NODE_USE_ENV_PROXY`)
- https://ampcode.com/manual (no proxy or storage facts found)
- https://www.developersdigest.tech/reference/amp/subcommands (third-party)
- https://developers.cloudflare.com/workers/platform/limits/ (100 MB body on Free/Pro; B1 did not re-check)
- https://go.dev/doc/go1.19 (`RLIMIT_NOFILE` raised)
