# PRD-17: Lessons from the old code

Id: PRD-17. Version: goalcup 1.0. Date: 30 October 2026. Base: main at 4b226de2.
Index: [PRD-00](PRD-00-index.md). Format: [PRD-01](PRD-01-record-format.md). Owner of
`docs/plans/2026-10-30-v1.0/LESSONS.md`, `crates/goalcup/tests/lessons_*.rs` and
`crates/goalcup/tests/lessons_fixtures/`.

Revision 2, 30 October 2026: answers the consistency check, review A and review B, and follows PRD-00
and PRD-01 revision 2. Every item and its answer is in "Review answers".

Revision 3, 30 October 2026: the alignment pass against the ledger. Adds sections 2.14 to 2.17 (format
and recovery, verifier, start and status, every DECISIONS entry), gives every row a guard code, renames
the harness facts HF1 to HF8, and fixes the gate's run place and merge rule. Every fix and its answer is
in "Alignment answers".

## Goal

Before the old crates are deleted, write down every edge case the old code learned the hard way, where
it handled it, and the case the fresh core must pass. No lesson is lost because a file was deleted.

## Points served

1 (every model call decodes: brotli, zstd, WebSocket deflate, chunked bodies, odd ClientHellos), 2 (one
stream per WebSocket direction, no native cap), 3 (every failure falls back to a working relay; restart on
the same port), 4 (secrets split across pieces, form-encoded tokens, header and URL masking), 5 (upload
retries and the server's idempotent store), 6 (counts at write, no false "recording works"), 7 (harness
facts: Hermes gateway, Codex wrapper, Node proxy switch, macOS keychain and file limits).

## Owner decisions it carries out

- **Fresh core, proven parts copied** (ledger L3, L4; BRIEF3, 29 Oct night): "Before deleting old crates,
  one agent extracts a lessons list from the old code, tests and DECISIONS.md ... and that list becomes
  acceptance cases. Old crates are deleted in the same build."
- **Option B, one process** (ledger L5). Lessons about the daemon hand-off (IPC, producers, epochs,
  failure budget) are kept only as "never again" rules (section 2.11), not as code.
- **No caps of our own** (ledger R3, R5). Old tests that pin a cap (1,024 connections, 128 open streams,
  32 native slots, 5 restarts, 10 failures) are listed as inverted on purpose (section 4), never ported.
- **Masking option b** (ledger M1). The email rule and the Codex rate-limit rules go; their old tests are
  listed so the loss is a decision, not an accident (K12, K16).
- **HTTP/2, gateways, one host list** (ledger H1, H2, H4, H6). Old lessons about forcing HTTP/1.1, the
  Host check and the SNI-mismatch loss are inverted (section 4). Old lessons about URL authorities and
  loopback addresses now guard the one host list's inputs (section 2.7).
- **One system for every harness** (ledger H1, H5, H7; BRIEF3, 29 Oct night; the owner's gateway question
  of 30 Oct). A gateway is one more listed HTTPS host. The old code's gateway trouble came from per-path
  rules and an environment built from nothing, not from the gateway; section 2.13 keeps those lessons.
- **Build in sandboxes, in three slices** (ledger B1, B3; PRD-00 build order). Lessons tests run in a
  build box as `ubuntu`, and each one merges only once the slice that builds its module is on
  integration and the test passes there.
- **B2, B2a.** The lessons lane plans, builds in worktrees, and each merge request gets two independent
  reviewers. When the replan agent amends another PRD's test, it also updates LESSONS.md's guard column
  or writes a night-log line for the lane.
- **AGENTS.md "one test per behaviour".** Each lesson has one guard per layer: the owning PRD's test
  where it names one, a lessons test only where no owner test drives the whole path (section 1).

## Today

| Source read | Where | Size (`wc -l`) |
|---|---|---|
| Recording proxy | `crates/goalcup-cli/src/proxy/*.rs` | 10,561 (src with inline tests) |
| Proxy tests | `crates/goalcup-cli/tests/it/m2_proxy_integration.rs`, `m2_http_relay.rs`, `m2_websocket_relay.rs`, `m2_tls_connect.rs` | 3,025, 2,249, 2,074, 1,275 (tests) |
| Redaction | `crates/goalcup-daemon/src/redact.rs`, `redact/stream.rs` | 1,279, 4,008 (src) |
| Redaction tests | `crates/goalcup-daemon/tests/it/redaction.rs`, `capture_redaction.rs`, `capture_redaction_extra.rs`, `capture_redaction_run_secrets.rs` | 2,629, 4,745, 1,113, 379 (tests) |
| Native capture | `crates/goalcup-daemon/src/native_capture.rs`, `crates/goalcup-platform/src/sourcecap.rs`, `sourcecap/unix.rs` | 1,632, 616, 669 (src) |
| Native tests | `crates/goalcup-daemon/tests/it/capture_native.rs` | 2,664 (tests) |
| Host and address rules | `crates/goalcup-cli/src/model_host.rs`, `proxy/preflight.rs`, `crates/goalcup-protocol/src/capture_policy.rs` | 1,540, 420, about 1,100 (src) |
| CA bundle, terminal, launch | `crates/goalcup-cli/src/ca_bundle.rs`, `crates/goalcup-platform/src/terminal.rs`, `crates/goalcup-cli/src/launch.rs`, `home.rs` | 207, 482, and parts (src) |
| Upload and server | `crates/goalcup-cli/src/upload/**`, `tests/it/upload_*.rs`, `server/worker/test/*.ts` | parts |
| Writer, finish and queue | `crates/goalcup-daemon/src/runtime/capture/writer.rs`, `finish.rs`, `sink.rs`; tests `crates/goalcup-daemon/tests/it/capture_lifecycle.rs`, `capture_ingest.rs` | 391, 763, 556 (src); 3,529, 3,095 (tests) |
| Chain, seal, parts and status types | `crates/goalcup-protocol/src/chain.rs`, `seal.rs`, `export.rs`, `capture_status.rs` | 45, 140, 2,017, 87 (src) |
| Verifier | `crates/goalcup-verifier/src/**`, `tests/**` (`m2_capture_integrity.rs`, `m2_decoder_v1_quiet_panic.rs`, `v5_corpus.rs`) | 13,488 (src), 20,724 (tests with fixtures code) |
| Start, continue and status | `crates/goalcup-cli/src/commands/start/*.rs`, `continue_run.rs`, `status.rs` | 6,130 in all (494 and 814 for the last two) |
| Decisions | `docs/DECISIONS.md` | 1,162 |
| Measured runs | `docs/research/results/2026-10-22-fix-plan*.md`, `2026-10-24-real-machine-round.md`, `2026-10-26-mac-retest.md`, `2026-10-vercel-gateway.md`, run 1's report (`docs/research/results/2026-10-30-two-hour-run.md` on `origin/smart/tender-clarke-qupne3`) | |
| Fix commits | `git log -i --grep=fix` over the proxy, redaction and native paths | 23 commits; the ones with a lesson are cited |

The old code stays readable at main `4b226de2` and tag `v0.7.0` after PRD-13 deletes it (PRD-13, "The
old code is never lost"). The old crates leave the workspace in phase 0 (`exclude`, PRD-16) and are read
from disk until the cut.

## Design

### 1. What a lesson is, and how it is kept

Each lesson is one row: an ID, the case (what goes wrong without the rule), where the old code handles it
(path:line) and its old test, the 1.0 acceptance case, the code fate and the guard owner.

**Acceptance codes.** `A:` an automated test, named `file::test`, or a test file for a file-level
guard. `R:` a real-run check on the pad (PRD-12) or a named real run. `D:` a data or docs file and the
key or section that holds the lesson. `X:` the owner decision or PRD that dropped or inverted the case
(fate `X` or `I` with no test left). A row may have more than one. Sections 2.9 to 2.16 use the same
codes: their 1.0 guard column is the `A:` guard, the `R:` pad row, the `D:` file and key, or the `X:`
decision, and the owner PRD stands last in that cell.

**Lesson ids.** C, T, H, D, W, K, N, P, F, S, R and G are the section letters of 2.1 to 2.11 and 2.13;
HF (harness facts, 2.12), FW (format and writer, 2.14), VR (verifier, 2.15) and SC (start and status,
2.16) are two letters, so the new ids never read as a fate code (`X`, `V`). Outside this file a lesson id is always written
with its prefix, `PRD-17 H2`; LESSONS.md's ID column uses `L-H2`. The ledger and other PRDs reuse
letters such as H2, R3 and T1 with other meanings.

**One guard per lesson.** If the owning PRD already names a matching test in its own acceptance, that
test is the guard and PRD-17 writes nothing. Otherwise PRD-17 writes the guard in
`crates/goalcup/tests/lessons_*.rs`. A row names two guards only when they test different layers (a
unit in the owning PRD and the whole path through `proxy::serve` here, as in C10 and K15). No lesson is
tested twice at the same layer.

**How a lessons test is written** (PRD-14 section 4). It runs the real `proxy::serve` with
`Recorder::for_tests` and `common::upstream`, and asserts on records: decoded DATA, GAP reasons, END
reasons and tails. It waits on an event, never on a sleep, and has no time bound below 5 s; a
`within(30 s)` only guards a hang. One stated exception: C7 holds the tunnel idle for 120 s by design,
`#[ignore]`, full tier (PRD-14 section 4). Speed and memory figures are reported, never a pass line.

H10 and K15 need the mask worker, because `Recorder::for_tests` shows the calls before masking. They
run `proxy::serve` against a real recorder in a temp entry folder (`GOALCUP_SERVER=off`), through
PRD-14's `common::recorder::real(dir)` over PRD-02's `Recorder::real_for_tests(dir, masker)`, and read
the part file with `goalcup_format::read`; every other lessons test uses `for_tests`.

**Code fate codes.**

| Code | Meaning |
|---|---|
| V | Copy verbatim: only renames, imports and the call into the new seam change |
| E | Copy with named edits |
| W | Rewrite: the behaviour is kept, the code is new (passive parser, new format, one process) |
| I | Inverted on purpose: the old behaviour is now wrong; the old test must not be ported |
| D | Data or docs only (connector data, participant text) |
| X | Dropped by an owner decision; the loss is stated |

**Path prefixes** used in the tables: `cli/` = `crates/goalcup-cli/src/`, `cli-it/` =
`crates/goalcup-cli/tests/it/`, `px/` = `crates/goalcup-cli/src/proxy/`, `dm/` =
`crates/goalcup-daemon/src/`, `dm-it/` = `crates/goalcup-daemon/tests/it/`, `pol/` =
`crates/goalcup-protocol/src/capture_policy.rs`, `plat/` = `crates/goalcup-platform/src/`. New test
files are under `crates/goalcup/tests/`.

**LESSONS.md** is a status table only: ID, guard, status (`open`, `green`, `real-run pass`,
`dropped: <decision>`), and the commit that turned it green. This PRD is the one list; LESSONS.md does not
copy the rows. Its guard column is the one map from lesson to test: when an owner renames a guard, the
PRD-17 lane edits that column. That is a LESSONS.md edit, not a new test and not a gate failure.

### 2. The lessons list

#### 2.1 CONNECT and admission (C)

| ID | Case | Old code; old test | 1.0 acceptance | Fate | Guard |
|---|---|---|---|---|---|
| C1 | A client sends the CONNECT head and its ClientHello in one write, without waiting for `200`. Bytes past the blank line must be replayed into the chosen path, never dropped | `px/connect.rs:512-518`, `:728-754`; `cli-it/m2_tls_connect.rs:449` | A: `lessons_connect::connect_and_client_hello_in_one_write_are_decrypted`; and the same with the head split over 1-byte reads | V | PRD-17 |
| C2 | A broken CONNECT head gets a fixed answer (`400`, `431` at 8 KiB, `405` for a non-CONNECT method) and is never echoed: it may hold a credential. Control bytes in a header value are a `400` | `px/connect.rs:692-717`, `:756-801`, bound `:72`; `cli-it/m2_tls_connect.rs:803` | A: `lessons_connect::bad_heads_get_fixed_answers_without_echo` (no input byte in any answer, log or record) | E (no `407`) | PRD-17 |
| C3 | Authority forms: `host:port`, `[v6]:port`, upper case, one trailing dot, `_` in a label; port 0, userinfo, path and query refused | `px/connect.rs:840-881`; inline test `px/connect.rs:1540` | A: the inline test, copied as `lessons_connect::authority_forms` | V | PRD-17 |
| C4 | A ClientHello split across TLS records and across TCP reads. Post-quantum key shares make ClientHellos larger than one segment. The bytes read are replayed (`PrefixedIo`) | `px/connect.rs:1042-1213`, `:1215-1280`; inline `:1487`; `cli-it/m2_tls_connect.rs:602`; fixtures `crates/goalcup-cli/tests/fixtures/m2-tls/*.hex` | A: `lessons_connect::client_hello_in_two_records_and_one_byte_reads_decides_by_sni`, using the copied OpenSSL fixtures and a 2 KiB synthetic hello | E (adds ALPN and fingerprint, PRD-03) | PRD-17 |
| C5 | A client that closes before its ClientHello is complete is no loss and opens no upstream connection (fix commit `92236c8e`) | `px/connect.rs:571-576`; `cli-it/m2_proxy_integration.rs:1385` | A: `lessons_connect::close_before_hello_opens_no_upstream` (fake upstream sees 0 connections; one `conn` record, no GAP) | W | PRD-17 |
| C6 | Upstream connect: each round re-resolves, IPv4 first, `TCP_NODELAY`, keepalive 60 s / 15 s / 4. Retry only for listed hosts: the agent's own traffic (pip, npm, git) gets one round, because retrying changes the run being measured. 1.0 adds one rule (PRD-03 rule 8): an upstream failure reaches the harness as a TCP reset, not a clean close | `px/connect.rs:962-1041` (`connect_host`, `connect_addresses`, keepalive); `cli-it/m2_tls_connect.rs:273`, `:306` | A: `lessons_connect::unlisted_refusal_is_one_round` (the fake upstream counts one connect, the client gets `502`) and `lessons_connect::listed_refusal_is_retried` (refused once, accepted on the next round, the client's call goes through). No stopwatch. The reset rule: PRD-03 `proxy_connect.rs` | V | PRD-17 |
| C7 | No idle timeout after admission: SSE and WebSocket streams idle for minutes between turns | `px/connect.rs:960-961` | R: RR1 (Codex WebSockets idle between turns and stay open). A: `lessons_connect::idle_tunnel_survives_120s` (full tier only) | V | PRD-17 |
| C8 | One broken handshake or one cancelled connection closes only its own two sockets; the listener and the other connections go on | `cli-it/m2_tls_connect.rs:1088`; `cli-it/m2_proxy_integration.rs:1727` | A: `lessons_connect::one_bad_handshake_leaves_others_relaying` | W | PRD-17 |
| C9 | A server-speaks-first protocol hung 30 s waiting for client bytes | `px/connect.rs:17-18` | A: PRD-03 `proxy_connect.rs` ("server-first protocol tunnels with no wait") | I (wait removed) | PRD-03 |
| C10 | A listed SNI inside a tunnel to another authority was a fail-open loss. In 1.0 it is decrypted with the upstream check made against the SNI name, so a local fake server can never pose as a listed host | `px/connect.rs:528-545`; `cli-it/m2_proxy_integration.rs:1153` | A: PRD-03 `proxy_connect.rs` (listed SNI behind an IP literal is decrypted) plus `lessons_connect::listed_sni_to_a_local_fake_server_is_never_trusted` (tunnel plus GAP, the client sees the fake's own certificate) | I, then W | PRD-03, PRD-17 |
| C11 | `accept` failing with `EMFILE` must pause and retry, never end the loop | `px/mod.rs:108`, `:1007-1011` | A: PRD-03 `proxy_connect.rs` (2,000 concurrent connections) and PRD-02 `watchdog_rlimit.rs` | V | PRD-03 |
| C12 | The listener is loopback only | `px/mod.rs:249-251` | A: PRD-02 `watchdog_restart.rs` asserts `127.0.0.1` | V | PRD-02 |

#### 2.2 TLS and the run CA (T)

| ID | Case | Old code; old test | 1.0 acceptance | Fate | Guard |
|---|---|---|---|---|---|
| T1 | Leaves carry the Authority Key Identifier. Python 3.13 refuses a leaf without it, so every Python harness and tool would fail | `px/tls.rs:17-20`, `:200`; `cli-it/m2_tls_connect.rs:164` (`key_ids`, leaf AKI equals CA SKI) | A: `lessons_tls::leaf_aki_names_the_ca_key`. R: RR3 (Hermes is Python; zero `tls_refused` on its listed host; the row records the version of the Python that runs Hermes, `$(dirname $(readlink -f $(command -v hermes)))/python --version` or the interpreter in the Hermes shebang) | V | PRD-17 |
| T2 | The CA: `pathLen 0`, critical name constraints, one hour back-dated, 8 days valid, leaves with one DNS SAN and serverAuth. A leaked key cannot sign an unrelated name | `px/tls.rs:100-147`, `:169-215`, constants `:52-59`; `cli-it/m2_tls_connect.rs:164` | A: `lessons_tls::ca_constraints_validity_and_leaf_shape` (subtrees come from the host list, PRD-03 change 2) | E | PRD-17 |
| T3 | Upstream TLS is validated before the harness handshake. A bad upstream certificate is never laundered into a trusted leaf | `px/connect.rs:602-622`; `cli-it/m2_tls_connect.rs:225`, `:253` | A: PRD-03 `proxy_tls.rs` (upstream failure falls back to a tunnel with a GAP) plus `lessons_tls::untrusted_upstream_reaches_client_unchanged` (the client sees the upstream's own certificate) | W (fallback instead of refusal) | PRD-03, PRD-17 |
| T4 | CA files are created new, mode 0600, and an existing file or link at the path is refused, never followed. The key has one file, `rec/run-ca.key`, created the same way and removed at the seal, so `goalcup recover` can reload the same CA (ledger R3b) | `px/tls.rs:158-178`; `cli-it/m2_tls_connect.rs:986` | A: `lessons_tls::ca_files_are_new_0600_and_never_follow_a_link`, no `PRIVATE KEY` block in the entry folder outside `rec/run-ca.key`, and none at all after the seal | V | PRD-17 |
| T5 | A shared CA variable (`SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE`) replaces the roots of every tool child. It gets OS roots plus the run CA: macOS from the System Roots keychain, Linux from the first of five bundle paths. Only `CERTIFICATE` blocks that parse are kept, re-encoded from DER, so a private key in a source bundle never reaches the file | `cli/ca_bundle.rs:1-207` (`LINUX_BUNDLES` `:68-74`, `certificates` `:124-150`); `cli-it/ca_bundle.rs` (88) | A: copy `cli-it/ca_bundle.rs` as `lessons_tls::bundle_*`. R: RR5 (git and npm through tunnels work with the bundle set) | V (`handed_file` edited per PRD-03) | PRD-17 |
| T6 | `SSL_CERT_DIR`, `CURL_CA_BUNDLE` and `NODE_TLS_REJECT_UNAUTHORIZED` are never set (D43 ruling 3) | `pol/` `PINNED_ENV` `:145`, `TRUST_ONLY` `:151` | A: PRD-11 `connector_launch.rs` | D | PRD-11 |
| T7 | ALPN: a client offering no ALPN is HTTP/1.1 (WebSocket clients often offer none). Never offer the client a protocol the upstream did not choose | `px/tls.rs:24-31`, `:281-283`; `cli-it/m2_tls_connect.rs:726` (h2-only refused) | A: PRD-03 `proxy_tls.rs` ALPN mirror matrix | I (h2 refusal) and W | PRD-03 |
| T8 | The crypto provider is named explicitly (ring). rustls panics at first use when two providers are compiled in and no default is installed | `px/tls.rs:292-295` | A: any `proxy_tls.rs` test runs in a binary with the scaffold's features | V | PRD-03 |
| T9 | An egress that re-signs TLS (a corporate inspection proxy, a cloud session's egress proxy) blocked every run: the recorder trusted the Mozilla roots only and has no upstream proxy | `px/tls.rs:231-236`; `docs/research/results/2026-10-vercel-gateway.md:150-162` | D: `docs/PARTICIPANT-GUIDE.md` known limits (PRD-15): an egress that needs its own proxy is unsupported. PRD-03 adds the OS store to upstream roots. R: none overnight (Vercel Sandbox has direct egress, same file `:12-14`) | E, D | PRD-03, PRD-15 |

#### 2.3 HTTP/1.1 framing (H)

| ID | Case | Old code; old test | 1.0 acceptance | Fate | Guard |
|---|---|---|---|---|---|
| H1 | Leading blank lines before a head are skipped; bare LF line ends are accepted and kept | `px/http.rs:776-803`, `:943-947`; inline `px/http.rs:2119`; `px/headers.rs:396` | A: inline test copied as `lessons_http1::head_scan_skips_blank_lines_and_takes_bare_lf` | V | PRD-17 |
| H2 | Obsolete line folding, a bad header name, a non-ASCII or conflicting `Content-Length`, or a bad `Transfer-Encoding` cannot be framed. In 0.7 the exchange got one GAP and the connection went opaque. In 1.0 parsing stops for the connection by PRD-03 section 5a: the open stream gets its GAP, a `parse_stopped` note is written, and at close the `conn` stream gets one GAP whose `lost` covers every byte relayed after the fault, so a later request on that connection is never silent (review A finding 2). `Content-Length: 5, 5` is accepted | `px/http.rs:828-859`; inline `:2133` | A: `lessons_http1::unframeable_heads_relay_on_inside_a_named_loss` (each old bad head, then a second request: the client gets both answers byte for byte; the second request sits inside the 5a `conn` GAP). The generic fault case is PRD-03 `proxy_http1.rs` | V (parse), W (fallback) | PRD-17 |
| H3 | Transfer codings other than `chunked` join the codings to undo (`Transfer-Encoding: gzip, chunked`). A content coding plus a transfer coding is two codings, so it is D5's GAP (checked: `transfer()` appends both, `for_codings` takes at most one, `px/http.rs:1546-1557`) | `px/http.rs:901-916` | A: `lessons_http1::transfer_gzip_then_chunked_decodes` (meta `coding` is `["gzip"]`, PRD-01 accepted the list) | V | PRD-17 |
| H4 | Chunked: a size line up to 4 KiB with extensions, at most 16 hex digits after leading zeros, a bare LF after data, trailers to a blank line within 64 KiB, and a body split at every byte ends exactly | `px/http.rs:1155-1265`; inline `:2100`, `:2109`; `cli-it/m2_http_relay.rs:405`, `:446` | A: `lessons_http1::chunked_split_at_every_byte_ends_exactly` (copied) and `::chunk_size_rules`. Trailers to the END tail: PRD-03 `proxy_http1.rs` | V | PRD-17 |
| H5 | No body for `HEAD`, 1xx, 204 and 304; a 2xx to an inner CONNECT is not parsed; a response with neither length nor chunked runs to close | `px/http.rs:1040-1056`; `cli-it/m2_http_relay.rs:249` | A: PRD-03 `proxy_http1.rs` (`HEAD`, 304) plus `lessons_http1::framing_matrix` (204, 1xx, read-to-close) | V | PRD-03, PRD-17 |
| H6 | Pipelined requests pair with their responses in order; a `100 Continue` before the final answer keeps the pairing | `px/http.rs:90`; `cli-it/m2_http_relay.rs:1301` | A: PRD-03 `proxy_http1.rs` (pipelining) plus `lessons_http1::informational_then_final_keeps_the_pair` | W | PRD-03, PRD-17 |
| H7 | The SSE prefix reaches the client before the upstream finishes (relay first) | `cli-it/m2_http_relay.rs:162`; `cli-it/m2_proxy_integration.rs:292` | A: PRD-03 `proxy_http1.rs` | W | PRD-03 |
| H8 | EOF inside a body ends the stream `interrupted`, never `complete`. A request written after the upstream closed cleanly is not forwarded | `cli-it/m2_http_relay.rs:1353`, `:1395` | A: `lessons_http1::eof_inside_a_body_is_interrupted` and `::request_after_upstream_close_is_marked_unsent` (END tail `unsent`, PRD-01 section 3) | W | PRD-17 |
| H9 | A head over 64 KiB is not parsed; the connection relays on and the loss is named (PRD-03 section 5a, as H2) | `px/http.rs:86`; `cli-it/m2_http_relay.rs:2067`, `:2124` | A: `lessons_http1::head_over_64k_relays_inside_a_named_loss` | W | PRD-17 |
| H10 | Credentials reach the upstream byte for byte and never the record | `cli-it/m2_http_relay.rs:934` | A: `lessons_http1::authorization_forwarded_unchanged_and_starred_in_meta` | W | PRD-17 |
| H11 | Every request on a decrypted host is recorded, whatever its path or method: token refreshes, `/models`, telemetry (fix commit `2652b2d4` had to stop counting them as calls) | `cli-it/m2_proxy_integration.rs:517`, `:603`; `cli-it/m2_http_relay.rs:1459` | A: PRD-03 rule 2 tests. Counting is PRD-02 and PRD-10 (POSTs and client messages only) | W | PRD-03 |
| H12 | A Host or absolute-form authority naming another host turned the connection opaque | `px/http.rs:11-16`; `cli-it/m2_http_relay.rs:1821`; `cli-it/m2_proxy_integration.rs:1194` | X: PRD-03 records every request on a decrypted connection; not ported | I | PRD-03 |

#### 2.4 Body decoding (D)

| ID | Case | Old code; old test | 1.0 acceptance | Fate | Guard |
|---|---|---|---|---|---|
| D1 | The Anthropic API answers `content-encoding: br` whenever the request offers `br`. Before brotli decoding every Claude Code reply was an `unsupported_encoding` gap (measured 22 Oct) | `px/http.rs:1868-1966` (strict state, trailing bytes after the end are invalid `:1952-1957`); `docs/research/results/2026-10-22-fix-plan-v2.md:15-24`; `cli-it/m2_http_relay.rs:622`, `:720` (decodes to whole 32 KiB pieces with no gap); fixture `crates/goalcup-cli/tests/fixtures/m2-http/sse-body.br` (made with `brotli -q 11`, not a real capture) | A: PRD-03 `proxy_http1.rs` (each coding) plus `lessons_decode::brotli_whole_32k_pieces_and_trailing_bytes`. R: RR2 | V | PRD-03, PRD-17 |
| D2 | zstd: several frames in a row; a frame that ends exactly when the output buffer fills is done (review P4 M1); window at most 2^23 (RFC 9659). Codex zstd-encodes HTTP request bodies with a ChatGPT sign-in (`E-providers.md:191`); run 1 used an API key and had none | `px/http.rs:100-102`, `:1968-2021`, P4 M1 `:2006-2015`; `cli-it/m2_http_relay.rs:622` (two frames), `:798` (cut after whole pieces is still a gap) | A: `lessons_decode::zstd_frame_ending_on_the_buffer_edge_is_complete`, `::zstd_cut_after_whole_pieces_is_a_gap`. R: only if a pad run uses a ChatGPT sign-in; otherwise "not shown in a real run" in LESSONS.md | V | PRD-17 |
| D3 | gzip: several members in a row, CRC and size checked per member, header fields `FEXTRA`, `FNAME`, `FCOMMENT`, `FHCRC`, reserved flags refused, header up to 64 KiB; `x-gzip` is gzip | `px/http.rs:1643-1769`, `:1546-1557`; `cli-it/m2_http_relay.rs:539`, bad CRC `:595-605` | A: `lessons_decode::gzip_two_members_and_a_bad_crc` | V | PRD-17 |
| D4 | HTTP `deflate` is zlib or raw, told apart by the first two bytes, and the first byte may arrive alone | `px/http.rs:1771-1850` (check `:1806`) | A: `lessons_decode::raw_and_zlib_deflate_split_after_byte_one` | V | PRD-17 |
| D5 | `identity` is ignored; more than one coding, or an unknown one, writes no body byte and a GAP names the codings | `px/http.rs:1546-1557`, `:2023-2050`; `cli-it/m2_http_relay.rs:850` | A: PRD-03 `proxy_http1.rs` | V | PRD-03 |
| D6 | A coding that breaks part way keeps the decoded prefix (output counted before the error is read), then GAP `decode_failed` and END `decode_error` | `px/http.rs:1603-1641`; `cli-it/m2_http_relay.rs:504` | A: `lessons_decode::corrupt_tail_keeps_the_prefix` | V | PRD-17 |
| D7 | A decompression bomb: 1 KiB that inflates to 1 GiB must not stall the relay or exhaust memory. The old 8 MiB per-read budget goes (PRD-03 section 7); decoding runs after the forward write, in 32 KiB pieces, PRD-02's `held_bytes` guard writes the gap, and PRD-03's `decoder_window` guard bounds open decoder windows (review A finding 10) | `px/http.rs:93-97`; `cli-it/m2_http_relay.rs:2039` | A: `lessons_decode::bomb_relays_on_in_bounded_pieces` (`for_tests` with a small room: the client gets every coded byte; no offered piece is over 32 KiB; the record holds a decoded prefix and a `memory_guard` GAP; RSS is reported, not asserted) | W | PRD-17 |
| D8 | Decoding is done on the copy only; the harness gets the coded bytes it asked for | `cli-it/m2_http_relay.rs:539` | A: `lessons_decode.rs` (every test in it checks that the fake client got the coded bytes) | W | PRD-17 |

#### 2.5 WebSocket and upgrade (W)

| ID | Case | Old code; old test | 1.0 acceptance | Fate | Guard |
|---|---|---|---|---|---|
| W1 | Codex uses one WebSocket with permessage-deflate and context takeover. One inflater per direction per connection; append `00 00 ff ff` at each message end (RFC 7692 section 7.2.2); reset only for a sender that negotiated no takeover | `px/websocket.rs:80`, `:749-774`, `:776-812`; `cli-it/m2_websocket_relay.rs:504`; `docs/harnesses/codex-cli.md:149`; `E-providers.md:478` | A: PRD-03 `proxy_ws.rs` (Codex fixture, takeover and no takeover). R: RR1 | V | PRD-03 |
| W2 | Parameters come from the `101`'s header, which may span several header lines. Only RFC 7692 parameters, each at most once, window bits 8 to 15, quoted values allowed. Anything else is relayed unchanged and recorded as a loss (0.7: one `unsupported_encoding` stream per direction, then nothing, `px/websocket.rs:33-34`, `:647-651`) | `px/websocket.rs:193-265`, `px/http.rs:1071-1074`; inline `px/websocket.rs:1025`; `cli-it/m2_websocket_relay.rs:698` | A: inline test copied as `lessons_ws::negotiation_accepts_only_rfc7692`, plus `::unknown_extension_relays_inside_a_named_loss` (no payload byte recorded as decoded; every later byte of each direction sits in a named GAP, per message or the 5a `conn` GAP) | V | PRD-17 |
| W3 | A fixed 15-bit inflate window decodes any smaller negotiated window | `px/websocket.rs:28-29` | A: `lessons_ws::server_max_window_bits_10_decodes` | V | PRD-17 |
| W4 | Under takeover the inflater must be fed even when a message's record is lost, or every later message is garbage | `px/websocket.rs:622-631`; `cli-it/m2_websocket_relay.rs:1596` | A: `lessons_ws::guard_drop_keeps_the_next_message_decodable` (inflation runs in the proxy before PRD-02's queue, so a guard drop never touches history) | V plus a rule | PRD-17 |
| W5 | A final DEFLATE block (BFINAL) or a decode error under takeover loses the history for good: every later compressed message in that direction is a GAP, never garbage | `px/websocket.rs:791-800`, `:814-827`; `cli-it/m2_websocket_relay.rs:1368`, `:611` | A: `lessons_ws::bfinal_ends_takeover_history` | V | PRD-17 |
| W6 | Frame headers and mask keys split across 1-byte reads; non-minimal lengths accepted | `px/websocket.rs:349-420`; `cli-it/m2_websocket_relay.rs:316` | A: `lessons_ws::one_byte_reads_decode_the_same` | V | PRD-17 |
| W7 | Frame validity per direction: RSV2 and RSV3 clear, known opcode, the client masks and the server does not, control frames have FIN, at most 125 bytes and no RSV1, continuation order, length top bit clear. 0.7: a violation is a GAP and that direction stops parsing. 1.0 (PRD-03 section 8): framing is tracked by length, so a violation is a GAP `decode_failed` for that message and the next message parses; where the length cannot be trusted (top bit set), parsing stops for the connection (5a). The relay goes on in every case | `px/websocket.rs:673-689`, `:898-920`; `cli-it/m2_websocket_relay.rs:776` | A: `lessons_ws::violations_are_named_and_relaying_goes_on` (each violation: the client and server get every byte; the message's bytes sit in a GAP; a following valid message is recorded, or for the bad length, sits in the 5a GAP) | V (checks), W (after a violation) | PRD-17 |
| W8 | Control frames interleave inside a fragmented message without breaking it. 1.0 records them with their opcode (PRD-03 change) | `cli-it/m2_websocket_relay.rs:431` | A: PRD-03 `proxy_ws.rs` (control frames recorded) | W | PRD-03 |
| W9 | The `101` and the first server frame arrive in one read; client frames may follow the upgrade request at once. Bytes past each handshake go to the WebSocket parser exactly once | `cli-it/m2_proxy_integration.rs:2392`; `cli-it/m2_http_relay.rs:1593`; `cli-it/m2_websocket_relay.rs:165` | A: `lessons_ws::upgrade_101_and_first_frame_in_one_write` | W (passive replay, PRD-03 section 5) | PRD-17 |
| W10 | Only a `101` whose `Sec-WebSocket-Accept` matches the key of a valid `GET` HTTP/1.1, version 13, upgrade is a WebSocket. A rejected upgrade stays HTTP. A bad `101` is parsed no further: 5a, so the bytes after it sit in a named loss | `px/http.rs:73-82`, `:999-1016`, `:1058-1069`; inline `:2081`; `cli-it/m2_http_relay.rs:1719`, `:1775`; `cli-it/m2_proxy_integration.rs:405` | A: `lessons_ws::rejected_upgrade_stays_http` and `::bad_accept_is_a_named_loss_not_a_websocket` | V | PRD-17 |
| W11 | An upgrade on any path of a decrypted host is recorded (`/v1/realtime`, `/ws/telemetry`) | `cli-it/m2_http_relay.rs:1687` | A: PRD-03 rule 2 | W | PRD-03 |
| W12 | A long message is written before its final frame and memory stays flat (run 1 client messages up to 1.5 MB) | `cli-it/m2_websocket_relay.rs:371`, `:1920` | A: `lessons_ws::a_long_message_is_offered_before_its_last_frame` (a 64 MiB message in 1 MiB frames: the first `ws` piece reaches `for_tests` before the test sends the last frame; event order, not a memory figure) | W | PRD-17 |
| W13 | An end mid-frame or mid-message is `interrupted`, never `complete`; the close code goes to the END tail | `cli-it/m2_websocket_relay.rs:1020` | A: `lessons_ws::eof_mid_message_is_interrupted` | V | PRD-17 |
| W14 | Rate-limit retries move Codex from WebSocket to HTTP POST on the same host. Run 1 counted 95 to 99 rate-limited calls (counts, not a share: 95 calls in measure 9; about 97 `error` events and 99 `response.failed` messages in the reconcile) | run 1 report, measure 9 and the reconcile notes | R: RR1 (both paths decode when they appear; with ALPN `PreferH1` the POSTs stay on HTTP/1.1, PRD-03 section 4) | D | PRD-12 |

#### 2.6 Secrets (K). Guards live in PRD-05

PRD-05 copies the rules and the stream masker. The rows below make sure each hard case has a named guard.

| ID | Case | Old code; old test | 1.0 acceptance | Fate | Guard |
|---|---|---|---|---|---|
| K1 | A secret split across pushes, DATA pieces or WebSocket messages is one secret; output never depends on chunking | `dm/redact/stream.rs:61-66`; `dm-it/capture_redaction.rs:35`, `:1539`; `dm-it/capture_redaction_run_secrets.rs:117`, `:152` | A: PRD-05 `mask_stream.rs` (every split point) | E | PRD-05 |
| K2 | Form-encoded OAuth refresh bodies: `refresh_token=1%2F%2F0gAb...` (percent-encoded) and `0.AXoA...` (dotted). A path after `--token=` is left alone | `dm/redact.rs:265-279`; `dm-it/redaction.rs:541`, `:551`, `:562`; `dm-it/capture_redaction.rs:2272`; `cli-it/m2_proxy_integration.rs:603` | A: PRD-05 `mask_rules.rs` with these three cases named. R: K2 cannot be forced in a pad run; automated only | V | PRD-05 |
| K3 | JSON-escaped secrets up to seven backslashes deep, and JSON inside a JSON string (tool output) | `dm/redact.rs:250-263`; `dm-it/capture_redaction.rs:3544` | A: PRD-05 `mask_stream.rs` | V | PRD-05 |
| K4 | PEM blocks bounded at 15,872 bytes; Claude Code's Read tool puts a line number and an arrow before every line; a lone BEGIN is bounded; a claim inside the block is merged | `dm/redact.rs:125`, `:245`; `dm-it/capture_redaction.rs:3637`, `:3482`, `:3890`; `dm-it/redaction.rs:2146-2445` | A: PRD-05 `mask_corpus.rs` and `mask_stream.rs` | V | PRD-05 |
| K5 | Codex `encrypted_content` blobs: run 1 had 634 `openai_key` and 777 `huggingface_token` spans, almost all inside the blobs | run 1 report, "2. Secrets"; `dm-it/capture_redaction.rs:3430` | A: PRD-05 `mask_rules.rs` (20 KiB base64url run intact). R: PRD-05 A2 on RR1 | E (left guard) | PRD-05 |
| K6 | Header masking: names with `key`, `token`, `secret`, `password`, `auth`, plus `cookie` and `set-cookie`; `Bearer` or `Basic` in any header; URL userinfo up to the last `@`; query names percent-decoded (`%6Bey`), `sig`, `signature`; `://` after `?` is query; a bare CR masks the whole line; a folded line of a masked header is masked (fix commit `0a4d848e`). HTTP/2: `:path` gets the query rule. No name is exempt, rate-limit headers included: they are masked when the name matches (PRD-05 section 7, PRD-00 owner question 11) | `px/headers.rs:20-75`, `:83-140`, `:186-270`; tests `px/headers.rs:277-402` | A: PRD-05 `mask_meta.rs` (rewritten from those tests). R: PRD-05 A3 | W (same-length `*`) | PRD-05 |
| K7 | Masked-key forms such as `sk-ab...wxyz` | `dm/redact.rs:209-216`; `dm-it/redaction.rs:998-1231` | A: PRD-05 `mask_rules.rs` | V | PRD-05 |
| K8 | Invalid UTF-8 around a secret; binary input; a key with invalid UTF-8 inside | `dm-it/redaction.rs:375`; `dm-it/capture_redaction.rs:2544`, `:3782` | A: PRD-05 `mask_stream.rs` | V | PRD-05 |
| K9 | Fail closed at the hold bound; an interrupt or gap never flushes a secret prefix | `dm/redact/stream.rs:125`; `dm-it/capture_redaction.rs:1112`, `:2806`, `:3397` | A: PRD-05 `mask_stream.rs` (fail-closed reproducers) | V | PRD-05 |
| K10 | Claude Code's billing header also appears inside the system prompt body | `connectors/claude-code-cli/harness.toml:155-159`; `dm-it/redaction.rs:352` | A: PRD-05 `mask_corpus.rs` | D | PRD-05, PRD-11 |
| K11 | Amp's WebSocket carries a `wsToken` in JSON | `connectors/amp-cli/harness.toml:102` | A: PRD-05 `mask_rules.rs` | D | PRD-05, PRD-11 |
| K12 | The email rule is dropped. It also caught a token used as the user of a URL with no password (`https://<token>@host`); `url_password` needs a colon. The old suite's URL credentials (81 and 190 characters) both have a colon and stay masked | `dm/redact.rs:280-287`; `dm-it/capture_redaction.rs:3580-3607` | A: PRD-05 `mask_rules.rs` keeps both old cases. The no-colon form is a stated loss (Risks) | X | PRD-05 |
| K13 | The run secret in every encoding (D60) goes with the proxy credential, cut everywhere (PRD-00, PRD-03 rule 5). PRD-05 still copies the encoding code of `run_secret.rs` for exact values | `crates/goalcup-protocol/src/run_secret.rs`; `dm-it/capture_redaction_run_secrets.rs` | X: no credential exists (PRD-00, PRD-03 rule 5) | X | none |
| K14 | Errors and log lines carry fixed text: no host path, header, body or key is formatted into one. A panic message is the same risk (review A finding 5) | `px/tls.rs:62-80`; `px/http.rs:46-51`; `px/connect.rs:29-40` | A: PRD-02 `pipeline_secure.rs` (a panic with a marker writes file and line, never the marker). R: PRD-12 C3 scans `goalcup.log` (S11), `status.json` and `upload.json` on every pad run | W | PRD-02, PRD-12 |
| K15 | A fail-closed mask window must end with the WebSocket message. In 0.7 each message was its own stream, so a window ended with the message; in 1.0 a direction is one stream, and Codex's compact JSON has no raw `\n`, so one overflow could star out hours of messages while every check stays green (review A finding 3) | `dm/redact/stream.rs:76-80` (window runs to `End` or `Gap` when no `\n` comes) | A: PRD-05 `mask_stream.rs` (the masker ends the window at `boundary()`) plus `lessons_ws::fail_closed_ends_at_message_end` (through `proxy::serve`: a 20 KiB undecided candidate in message 1 of `ws_up`, then 10 plain messages; message 2 onward is unmasked, the span is named `fail_closed`). R: RR1 reports `mask_failclosed_bytes` | W | PRD-05, PRD-17 |
| K16 | The Codex `rate_limits`, `plan_type`, `credits_remaining` and `credits` rules are dropped (ledger M1): those bodies are recorded readable. Rate-limit headers are not exempt from the header name rule (K6) | `connectors/codex-cli/harness.toml:110-113`; `dm-it/redaction.rs:752-766`, `:812-847`, `:930-953` | A: PRD-05 `mask_rules.rs` (a Codex `rate_limits` object comes through byte for byte) | X (M1) | PRD-05 |

#### 2.7 Host names and addresses (N)

These rules guarded D62's model address. In 1.0 they guard every input to the one host list: the shared
file, `goalcup setup --record-host` (PRD-10) and host hints from config files and the connector's named
`hint_env` variables (PRD-11). One function checks all three: `connector::hosts::check(name, source) ->
Result<Host, Skipped>` (PRD-00 S3). A hint that fails is skipped, never a refusal; one that points at
this computer becomes a `local_model` note (PRD-11, review A finding 7); `--record-host` refuses.

| ID | Case | Old code; old test | 1.0 acceptance | Fate | Guard |
|---|---|---|---|---|---|
| N1 | A backslash, space or control byte in a URL authority. A WHATWG parser reads `\` as `/`, so `https://api.example.com\@evil.com` reaches one host while a naive split checks another. Refused whole, userinfo included (fix commit `e21af7d7`) | `cli/model_host.rs:836-845`, reason `:498-502`; test `:1311-1330` | A: `lessons_hosts::authority_with_backslash_or_control_is_never_a_host` (hint skipped with a note; `--record-host` refused) | V | PRD-17 |
| N2 | `0.0.0.0`, `::`, `::ffff:127.x.y.z` and `::ffff:0.0.0.0` reach this computer; a mapped public address does not. Compare after `to_canonical()` | `px/preflight.rs:169-174`; test `:393-417` | A: `lessons_hosts::unspecified_and_mapped_loopback_reach_this_computer` (`Skipped` says "this computer", so a hint gives `local_model`) | V | PRD-17 |
| N3 | Loopback by name: `localhost`, `*.localhost`, `localhost.localdomain`; an address spelled as a name (`127.1`, `0x7f.1`, `127.0.0.01`); a single-label name | `pol/` `local_host_problem` `:1016-1049` | A: `lessons_hosts::loopback_names_and_numeric_labels_are_refused` | V | PRD-17 |
| N4 | Host shape: lowercase letters, digits and `-` in labels of 1 to 63, at most 253, no IPv4 literal, no trailing dot; the userinfo is dropped before anything can log it | `pol/` `is_capture_host` `:1001-1015`; `cli/model_host.rs:852-866`; test `:1146` | A: `lessons_hosts::host_shape` | V | PRD-17 |
| N5 | Goal Cup's own server (`agentcommunity.org` and subdomains) and any name containing `goalcup` is never a model host, so the uploader's own traffic is never decrypted | `pol/` `:120-138` | A: `lessons_hosts::the_event_server_is_never_listed` | V | PRD-17 |
| N6 | A name that resolves only to this computer (N2 forms included) is skipped, and as a hint it is a `local_model` note. DNS is asked once at start, so a later DNS change is not seen | `cli/model_host.rs:509-537`; `px/preflight.rs:176-190` | A: `lessons_hosts::a_name_resolving_only_here_is_skipped` (stub resolver) | E | PRD-17 |
| N7 | The config reader: one regular file, never a link (`O_NOFOLLOW`), at most 1 MiB, UTF-8 with the BOM stripped, JSON or TOML that parses, YAML by a small block lookup; one `via` indirection (Codex `model_providers.*.base_url` via `model_provider`); Claude Code keeps its base URL at `env.ANTHROPIC_BASE_URL` in `settings.json` (D74) | `cli/model_host.rs:560-760` | A: PRD-11 `connector_hints.rs` | E | PRD-11 |
| N8 | `NO_PROXY` holds the loopback names, so a local model server is never proxied and never recorded. 1.0 names it: a hint that points here writes a `local_model` note and PRD-10 one status line (review A finding 7) | `cli/launch.rs:319-320`, `:858-859` | A: PRD-11 `connector_launch.rs` (`127.0.0.1,localhost,::1`) and `connector_hints.rs` (a loopback value gives one `local_model` note) | D | PRD-11 |
| N9 | Path rules (`%2e`, dot segments, quotes, non-ASCII) mattered when paths selected what was recorded. In 1.0 a decrypted host records every path | `pol/` `unclean_path` `:1071-1076` | X: no use left; a decrypted host records every path (ledger H2, PRD-03 rule 2) | X (no use left) | none |

#### 2.8 Process, terminal, restart and environment (P)

| ID | Case | Old code; old test | 1.0 acceptance | Fate | Guard |
|---|---|---|---|---|---|
| P1 | Restart on the same port. The old supervisor rebound the port up to 5 times; a connection made while nothing listened was refused and never recorded | `px/mod.rs:1019-1112`, the window `:1040-1043`; `cli-it/m2_proxy_integration.rs:2920`, `:2967` | A: PRD-02 `watchdog_restart.rs` (a plain `kill -9`, no hook: the port is unchanged and a connection made during the restart is served). R: RR4 | I (cap), W (the watchdog holds the socket) | PRD-02 |
| P2 | A restarted recorder must serve leaves from the same CA, or every harness call fails TLS. The CA now travels on the first control line (PRD-00 S8) | `px/tls.rs:5-14` (the key died with the process) | A: PRD-03 `proxy_restart.rs` (a queued connection is served by the second boot with the same CA) and PRD-02 `watchdog_restart.rs` (a decrypted connection after the restart verifies against the same `run-ca.pem`). R: RR4 | W | PRD-02, PRD-03 |
| P3 | The harness shares the terminal's foreground group, so Ctrl-C, Ctrl-\, Ctrl-Z and a closed window reach goalcup too. Catch them, never ignore them: `SIG_IGN` survives `exec`, so a harness started under it is born deaf to Ctrl-C. Ignore only `SIGTTOU`, and only after the fork. Give INT, QUIT and TSTP back when the harness exits; keep HUP until the wrap-up ends | `plat/terminal.rs:1-128` (`Shield`) | A: `lessons_launch::ctrl_c_reaches_the_harness_and_the_watchdog_lives` (pty) | V | PRD-17 |
| P4 | A TUI must never get a process group of its own: its first read from the terminal raises `SIGTTIN` and it hangs before drawing a frame | `cli/launch.rs:45-80` | A: `lessons_launch::a_tui_child_reads_the_terminal` (pty; the child is in the foreground group) | V (rule) | PRD-17 |
| P5 | Put the terminal's modes back after a harness that left it raw; continue a harness that stopped itself | `plat/terminal.rs:129-157` (`Modes`), `:257-292` (`wait_resuming`) | A: `lessons_launch::modes_restored_after_a_raw_exit` | V | PRD-17 |
| P6 | Open files: launchd starts a process at 256 on macOS. Raise the recorder's soft limit to the hard limit, but on macOS clamp it to `OPEN_MAX` (10,240): `setrlimit` with an infinite `rlim_cur` for `RLIMIT_NOFILE` fails with `EINVAL` there (macOS `getrlimit(2)` compatibility note; unverified on a Mac in this build). Give the harness only `max(soft, 4096)`, never the hard limit: in Docker the hard limit is often 1,048,576 and tools that loop over every descriptor or use `select()` suffer (review A finding 11). The old code asked only for 2,304 | `cli/commands/start/proxy.rs:25-60` | A: PRD-02 `watchdog_rlimit.rs` (recorder to the hard limit, harness to 4,096, macOS branch). R: PRD-02 R4 (Linux) and the owner's Mac run (RR6) | E | PRD-02 |
| P7 | Codex's npm wrapper hides the native binary; signalling the launcher does not stop Codex (D22). The watchdog owns ending the harness (PRD-00 S8) | `docs/DECISIONS.md` D22; `connectors/codex-cli/harness.toml` termination row | A: PRD-02 `watchdog_end.rs` (a child and a detached grandchild get SIGTERM then SIGKILL; the watchdog and recorder are never signalled) | D | PRD-02 |
| P8 | A seal must not wait for a connection the harness or its helper holds open: a real Codex run held one until the reserve (D56), and Hermes's gateway holds them up to its 600 s idle exit (D72) | D56 "Time"; D72 0.7.1 bullets | A: PRD-02 `pipeline_seal.rs` (open streams end `cut_at_seal`; a held-open idle connection does not delay the seal past the grace; bytes after the stop are one `after_stop` GAP). R: PRD-11 acceptance 5 (Hermes gateway) | W | PRD-02 |
| P9 | An environment built from nothing drops a key the harness reads from its environment: Codex with `env_key` for Vercel AI Gateway failed before any call | `docs/research/results/2026-10-vercel-gateway.md:203-208` | A: PRD-11 `connector_launch.rs` (the participant's key variables pass). R: the pad's gateway case | D | PRD-11 |
| P10 | Node needs `NODE_USE_ENV_PROXY=1` to send `fetch` through `HTTPS_PROXY` (D59); `NODE_EXTRA_CA_CERTS` is read only at process start | D59; `cli/launch.rs:40-41` | A: PRD-11 `connector_launch.rs` | D | PRD-11 |
| P11 | Codex runs every tool command as `/bin/sh -lc`. On Debian a login shell resets `PATH`, so with an empty run `HOME` the agent loses `~/.local/bin` (and `goalcup time-left`). The run home gets a `.profile` that puts the handed `PATH` back, written beside and renamed over, never following a link | `cli/home.rs:591-620`; `cli/launch.rs:337`; `docs/research/results/2026-10-22-fix-plan-v5.md:9-30` | A: PRD-11 `connector_launch.rs` (`sh -lc 'command -v goalcup'` finds it). R: PRD-10 acceptance 7 (`sh -lc 'goalcup time-left'` in the harness's `HOME`) | V | PRD-11 |
| P12 | goalcup's lock beside a harness home must use a name no harness uses. `<connector>.lock` was exactly the lock Claude Code takes for its token refresh, so a start with an expired token stalled | `cli/home.rs:105-113`; `docs/research/results/2026-10-22-fix-plan-v6.md:1-25` | A: PRD-11 `connector_launch.rs` (lock name ends `.goalcup.lock`) | V | PRD-11 |
| P13 | macOS keychain: a scratch `HOME` hides the login keychain, so `gh` logins and Claude subscription logins are not found (real-machine round R2; D38 links `Library/Keychains`) | `docs/research/results/2026-10-24-real-machine-round.md:40-52`; D38 | R: the owner's Mac pad run (PRD-12 P9) | D | PRD-11 |
| P14 | The approval may take minutes. A 30 s idle exit before launch abandoned 8 test runs (real-machine round R1). Nothing before the launch may time out sooner than the approval code (15 min) | `docs/research/results/2026-10-24-real-machine-round.md:19-38` | A: PRD-10 `cli_commands.rs` (no timer under 15 min before launch) | W | PRD-10 |
| P15 | A stop meets one stated bound even with a stuck connection task. PRD-02 owns the bound at a deadline seal: drain 60 s, then native finish up to 180 s (PRD-00 ruling on consistency item 49); the upload wrap-up runs after the seal and never delays it | `cli-it/m2_proxy_integration.rs:1543`, `:2237` | A: PRD-02 `pipeline_seal.rs` | W | PRD-02 |

#### 2.9 Session files (F). Guards live in PRD-04

| ID | Case | Old code; old test | 1.0 acceptance | Guard |
|---|---|---|---|---|
| F1 | Admit a file by the later of mtime and ctime, in whole seconds: an agent can set mtime back, not ctime (D57) | `dm/native_capture.rs:23-30`; `dm-it/capture_native.rs:2261`, `:2296`, `:2320` | A: PRD-04 `native_walk.rs` (backdated mtime with a new ctime is taken; a file a second before the start is skipped); PRD-04 keeps the rule for every row | PRD-04 |
| F2 | Truncate then append never reuses offsets; a replacement is a new generation. Truncate then regrow past the cursor between two polls is not detectable (stated) | `dm/native_capture.rs:55-61`; `dm-it/capture_native.rs:1321`, `:1278` | A: PRD-04 `native_tail.rs` | PRD-04 |
| F3 | No link followed at any level; a FIFO is never opened (the read would hang); no mount crossing | `plat/sourcecap/unix.rs:94-98`, `:185-190`, `:221-226`; `dm-it/capture_native.rs:631`, `:686`, `:2059` | A: PRD-04 `native_safety.rs` | PRD-04 |
| F4 | A partial last line is kept; a live EOF is not an end | `dm-it/capture_native.rs:566`, `:1238`, `:1710` | A: PRD-04 `native_tail.rs` | PRD-04 |
| F5 | Hermes rewrites its snapshot atomically during a read (a `source_changed` gap); `request_dump_*` is never matched (D61) | `dm-it/capture_native.rs:1962`, `:1906` | A: PRD-04 `native_snapshot.rs`, `native_safety.rs` | PRD-04 |
| F6 | 0.7 read a final snapshot only after a confirmed harness exit and kept nothing otherwise; the queue had to be drained between read passes or a few MiB latched the old queue (fix commit `cd3e541c`). 1.0 inverts the first half on purpose: a seal copy while the harness runs is taken and marked `harness_exited: false` with a `final_snapshot_live` note, and SQLite sets are also copied every 60 minutes when changed (review A finding 9). The second half becomes `room()`: native reads wait, nothing latches | `dm-it/capture_native.rs:2553`, `:2600` | A: PRD-04 `native_snapshot.rs` (seal copy while running is marked; periodic copy), `native_room.rs` (room 0 reads nothing and loses nothing) | PRD-04 |
| F7 | 32 native slots for 43 rollout files lost about 11 transcripts in run 1 | run 1 report, root cause 5 | R: PRD-04 acceptance 2 (Codex with subagents: one native stream per rollout file) | PRD-04 |
| F8 | Hermes compaction in place keeps the session id with fewer messages and stops the JSON snapshot file; the 0.7 connector turns it off (`connectors/hermes-cli/harness.toml:49-56`, D61). Dropped by PRD-11 for 1.0: 1.0 reads `state.db`, not the JSON snapshot. Unverified whether in-place compaction also rewrites `state.db` rows; PRD-04's hourly copies keep earlier states if it does | D61 | X: PRD-11 (1.0 reads `state.db`); none in code. R: the Hermes `full` pad row (PRD-12 P3); a loss there brings the switch back as data | PRD-11 |

#### 2.10 Upload and server (S). Guards live in PRD-06 and PRD-07

| ID | Case | Old code; old test | 1.0 acceptance | Guard |
|---|---|---|---|---|
| S1 | A retried chunk is answered from the stored ack without its body; a dropped reply leaves the chunk stored and the retry harmless | `server/worker/test/integration.test.ts:292`; `cli-it/upload_server.rs:514`, `:920` | A: PRD-07 `server/worker/test/integration.test.ts` (kept case, today `:292`); A: PRD-06 `upload_resume.rs` (the fake Worker commits a chunk and drops the answer) | PRD-06, PRD-07 |
| S2 | A body that does not match `Goalcup-Sha256` is `422` and nothing is kept; header and size checks come before the body | `server/worker/test/integration.test.ts:320`; `cli-it/upload_server.rs:589` | A: PRD-07 `integration.test.ts` (kept case, today `:320`) | PRD-07 |
| S3 | Past the deadline the server takes nothing but the final heartbeat (`503` with `Retry-After`) | `server/worker/test/integration.test.ts:638` | A: PRD-07 `integration.test.ts` (kept case, today `:638`; `503 awaiting_final`, PRD-07 section 3); PRD-06 waits | PRD-07 |
| S4 | No ordinary heartbeat after, or while a lost final is asked again | `cli/upload/uploader/heartbeat.rs:15-20`; `cli-it/upload_chunks.rs:1563`, `:1599` | A: PRD-06 `upload_beat.rs` | PRD-06 |
| S5 | A `409` re-reads the server's position and moves only to a position the local stream confirms | `cli-it/upload_chunks.rs:901`, `:924` | A: PRD-06 `upload_resume.rs` (a `409` with no forward move counts as a failure) | PRD-06 |
| S6 | No server answer stops the run (D64) | `cli-it/upload_chunks.rs:975` | R: PRD-06 U3 (network cut) | PRD-06 |
| S7 | A clock set back a day must not lengthen a segment | `cli-it/upload_chunks.rs:1261` | A: PRD-06 `upload_beat.rs::clock_set_back_does_not_lengthen_a_segment`; A: PRD-10 `cli_start.rs` (the deadline comes from the server's `seconds_left` on the boot clock; a wall clock set back 24 h does not change it) | PRD-06, PRD-10 |
| S8 | Run 1: 264 MB chunks and an 18-minute backoff left 7% sent at t+61 min; pruning at the seal removed the local copy before anyone verified it | run 1 report, findings 6 and 7 | R: PRD-06 U2 and U5 | PRD-06 |
| S9 | A heartbeat lost in a network cut waited for the next 5-minute beat (real-machine round F7) | `docs/research/results/2026-10-24-real-machine-round.md:111-112` | R: PRD-06 U3 | PRD-06 |
| S10 | A lost reply to the open is the same run (no second run) | `cli-it/upload_server_segments.rs:602` | A: PRD-07 Worker test under `server/worker/test/` (a repeated open answers the same run; the lane writes the exact name in LESSONS.md, or asks PRD-07 for it in the night log) | PRD-07 |

#### 2.11 Run 1's cause chain: never again (R)

| ID | Rule | Old code | 1.0 guard |
|---|---|---|---|
| R1 | A drop under load never counts toward "the recorder is dead" | `px/ingest.rs:51-56`, `:104` (`MAX_CONNECTION_FAILURES`) | A: PRD-02 `pipeline_no_latch.rs` (PRD-02) |
| R2 | A memory guard per stream with a gap, not one 8 MiB queue that drops a whole producer | `dm/runtime/capture/sink.rs` (`MAX_CAPTURE_QUEUE_BYTES`) | A: PRD-02 `pipeline_guard.rs` and `pipeline_stall.rs` (PRD-02) |
| R3 | Every loss is a GAP with a start and an end, including bytes relayed after a parser stop (PRD-03 section 5a) and after the seal (`after_stop`) | run 1: 23 gaps, all with one time and `lost_bytes` null | A: PRD-08 `check_invariants.rs` (invariant 12 `gap_has_end`: a GAP with start equal to end fails); A: PRD-03 `proxy_http1.rs` fault case; H2, H9, W2, W7, W10 here (PRD-08, PRD-03) |
| R4 | Count at write; count POSTs and client text and binary messages, never selections or interrupts | `record_generation_request` (run 1 report, root cause 4) | R: PRD-02 R3 (the seal's `model_requests` equals the verifier's recount) and PRD-10 acceptance 1 (PRD-02, PRD-10) |
| R5 | One stream per WebSocket direction (152,728 server-message streams in run 1) | `px/websocket.rs:691-716` | A: PRD-01 `golden.rs`; A: PRD-03 `proxy_ws.rs` (PRD-01, PRD-03) |
| R6 | A decrypted connection that stopped recording was in no list: the sealed-host list showed no model host while model traffic went unrecorded | run 1 report, measure 3 | A: PRD-03 `proxy_connect.rs` (a `conn` record at open) (PRD-03) |
| R7 | "`intercepted=` above 0" is not proof of a model record | `docs/research/results/2026-10-vercel-gateway.md:23-25` | R: PRD-12 C1 (no unexplained gap and the history check, from the record; the harness's own log as an optional extra, ledger T4) (PRD-12) |
| R8 | Codex sends a `thread-id` header on every POST: subagent attribution needs it readable, not masked | `docs/plans/2026-10-30-audit/M-two-hour-run.md:947`, `:1046` | A: PRD-05 `mask_meta.rs` (the name has no credential part, so the value stays readable); R: RR1 (PRD-05, PRD-09 reads it) |

#### 2.12 Harness facts from DECISIONS and the measured runs (HF). Data, PRD-11

| ID | Fact | Source | 1.0 guard |
|---|---|---|---|
| HF1 | Hermes 0.22 runs every model call in a gateway process per `HERMES_HOME` that keeps the environment of whoever started it. A stale gateway sent every call around the proxy while status said "Problems: none" | D72; `docs/research/results/2026-10-26-mac-retest.md:28-30` | R: PRD-11 acceptance 5 (a stale Hermes gateway left running) (PRD-11) |
| HF2 | Hermes on macOS ignores `HERMES_CA_BUNDLE` and trusts a `providers` entry's `ssl_ca_cert`; its native Anthropic client ignores `ssl_ca_cert` | D72 | D: `connectors/hermes-cli.toml` (the CA entry PRD-11 writes); R: the owner's Mac run, RR6 (PRD-11) |
| HF3 | `hermes chat --continue` exits 1 with no session; 0.19.0 has no `gateway stop -y` (exit 2) | D72, D73 | D: `connectors/hermes-cli.toml` (`plan.before`, `plan.after`); A: PRD-02 `watchdog_end.rs` (the watchdog runs `plan.before` and `plan.after`; a failed helper command is logged and the run goes on) (PRD-11, PRD-02) |
| HF4 | Codex refresh tokens rotate: two runs on one Codex home race the refresh, so runs on a shared home are serialised (D39) | D39 | A: PRD-10 `cli_commands.rs::second_start_on_one_home_refuses` (`home_busy`) (PRD-10) |
| HF5 | Codex `-c` overrides force its embedded mode (startup warning in run 1) | run 1 report, measure 9 | A: PRD-11 `connector_launch.rs::codex_gets_no_dash_c_override`; D: `connectors/codex-cli.toml` `args` (PRD-11) |
| HF6 | Claude Code writes its session file at startup, before any message, so a session file alone does not show a conversation to resume | D73 | A: PRD-10 `cli_continue.rs` (the resume rule uses written counts; a quick non-zero exit after a resume opens a new session on the next `continue`); D: PRD-10 section 3.1 step 3 (PRD-10) |
| HF7 | A typed prompt was swallowed by a first-run dialog, a welcome screen or a split paste, and the row still passed | `docs/research/results/2026-10-22-fix-plan-v2.md:11-28` | R: PRD-12 pass rule (a row passes only if the agent worked) (PRD-12) |
| HF8 | A sign-in host (OAuth token endpoint) is never decrypted; under a replace-mode CA its refresh failed (D62). PRD-03 gives replace-mode variables the full bundle | D62; `connectors/codex-cli/harness.toml:32` | A: PRD-11 `connector_hosts.rs` (`never` wins) (PRD-11) |

#### 2.13 Gateways and the sandbox (G). Measured on 28 Oct

A gateway is an HTTPS model endpoint with another host name and sometimes another path. The measured
gateway runs failed only where 0.7 had per-host or per-path rules, or built the environment from
nothing. 1.0's one host list (gateways listed with `kind: gateway`) and "record every path" (H11) remove
both, so these rows add no new code, only guards that already exist.

| ID | Fact | Source | 1.0 guard |
|---|---|---|---|
| G1 | Gateways use their own path prefixes (`/claude-code/v1/messages`, `/codex/v1/responses`). 0.7's decoder took only the exact `/v1/messages`, so 19 recorded calls were labelled undecoded | `docs/research/results/2026-10-vercel-gateway.md:56-61` | Recording: H11's guard (PRD-03 rule 2 tests). Usage: R: PRD-12 P4's `gateway` rows (usage calls read with `host_kind: gateway`; the shape is picked by path suffix and body, never by host; `usage_shapes.rs` is cut, ledger B10) (PRD-03, PRD-09) |
| G2 | A gateway host on no list was tunnelled (`intercepted=20`, zero wire model requests) | same file `:23-25`, `:127`, `:139` | D: `connectors/model-hosts.toml` (`ai-gateway.vercel.sh`, OpenRouter and Cloudflare AI Gateway as `kind: gateway`); R: PRD-12 P4 (`listed: shared` with no participant or config entry) (PRD-11, PRD-12) |
| G3 | Codex with `env_key` for the gateway failed before any call: the key variable was dropped | same file `:203-208` | A: P9's guard, PRD-11 `connector_launch.rs` (PRD-11) |
| G4 | Hermes with a bare `provider: custom` gateway entry sends no key: the calls are recorded and answered `401`. Its built-in `ai-gateway` provider works | same file `:14-17`, `:125-126` | R: PRD-12 gateway case config (Hermes uses `ai-gateway`); a provider refusal is not our failure (PRD-12 P6) (PRD-12) |
| G5 | A Vercel Sandbox `stop` keeps a disk snapshot for 30 days, keys included; `sandbox exec` is not a terminal, so `goalcup start` must run in `tmux` | same file `:108-113` | R: PRD-12 pad boxes are `--non-persistent` and run in `tmux` (PRD-12) |

#### 2.14 Format, writer and recovery (FW). Guards live in PRD-01, PRD-02 and PRD-08

0.7 wrote a JSON-lines hash chain through a separate daemon; 1.0 writes PRD-01's part files from one
process. These rows keep the case, not the code: every one is fate W (rewritten) unless its cell says
`X:`.

| ID | Case | Old code; old test | 1.0 acceptance | Guard |
|---|---|---|---|---|
| FW1 | A close or a loss is never what the memory guard drops. 0.7 kept a separate reserve for closes and loss markers, so a full payload queue never blocked a close or hid a loss, and the writer kept a protected allowance for them | `dm/runtime/capture/sink.rs:6-15`, `:118-122`; `writer.rs:19-26` | A: PRD-02 `pipeline_guard.rs` (OPEN is always admitted; only offered DATA pieces drop, and every drop is a GAP on its own stream) | PRD-02 |
| FW2 | Only a synced point counts as written: an appended record not yet synced is not durable (`capture_status.rs:12-13`), and a failed flush or part rotation stops the writer (`writer.rs:203-205`). 1.0: the durable index length marks it, recovery truncates to it, and a write error ends the recorder with exit 74 and a `recorder_down` GAP that names it (PRD-02 section 5) | `crates/goalcup-protocol/src/capture_status.rs:12-13`; `dm/runtime/capture/writer.rs:203-205` | A: PRD-01 `recovery.rs` (a part file cut at a random byte recovers to the last index line) | PRD-01 |
| FW3 | Part files are contiguous and named by where they start; a part missing between two others is damage; when the first parts were pruned locally, only the server copy can be judged whole (0.7 `chain_pruned_locally`) | `crates/goalcup-protocol/src/export.rs:78-116`; `crates/goalcup-verifier/src/capture_analysis/reader.rs:286-290`, `:504` | A: PRD-08 `server_compare.rs` (a missing middle chunk fails); PRD-08 exit 1 for pruned local parts (section 1; the lane asks PRD-08 for the named case) | PRD-08 |
| FW4 | A record of an unknown kind or version is reported as that, never as damage or a parse failure, so a newer writer's record never reads as a broken entry (D54) | `export.rs:347-356`, `:478-480` | A: PRD-08 `check_invariants.rs` (an unknown record kind gives its id and exit; GAP reason 8 gives `warn`, not a crash) | PRD-08 |
| FW5 | A crash inside the seal must leave neither an entry that reads unsealed for ever nor one sealed twice: the next command commits it once (0.7: a terminal written without its `seal.json`; a seal whose registry write was lost) | `dm-it/capture_lifecycle.rs:2210`, `:2486`, `:2547` | A: PRD-01 `recovery.rs` (a dead entry gets one recovery frame; a part file already closed by `R` takes a second recovery frame, never a second seal) | PRD-01 |
| FW6 | Nothing is appended after the seal. 0.7 refused a late frame after the local finish; 1.0 writes bytes relayed after the stop as one `after_stop` GAP before the seal, and nothing follows the seal's LINK | `dm-it/capture_lifecycle.rs:1994` | A: PRD-02 `pipeline_seal.rs` (bytes after the stop are one `after_stop` GAP); A: PRD-08 `check_invariants.rs` | PRD-02, PRD-08 |
| FW7 | A last line cut by a crash is not a whole record: the reader keeps it as a torn tail, never as data, and a resume refuses a chain whose length changed under it | `dm-it/capture_lifecycle.rs:2614`; `export.rs:969-982` | A: PRD-08 `check_invariants.rs` (a torn tail); A: PRD-01 `recovery.rs` | PRD-01, PRD-08 |
| FW8 | A dead boot's counters must reach the record. 0.7 latched an expired finish's proxy counters for the next finish to chain (`finish.rs:167-171`), and its proxy counters became gaps only at the seal (`writer.rs:185-188`). 1.0 writes each loss as a GAP when it happens (R3) and puts the dead boot's COUNT in the recovery frame (PRD-01 section 12 step 3) | `dm/runtime/capture/finish.rs:167-171`; `writer.rs:185-188`; `dm-it/capture_lifecycle.rs:1821` | A: PRD-01 `recovery.rs` (the recovery frame's COUNT equals a recount of its boot); R: RR4 | PRD-01 |
| FW9 | The flush refused a chain file that had gained a second name or been replaced, and the reader never followed a replaced file (link, hard link, not a regular file) | `export.rs:1868-1872`; `reader.rs:378` | A: PRD-08 `server_compare.rs` (the server copy is the judge: one flipped byte fails) | PRD-08 |

#### 2.15 Verifier (VR). Guards live in PRD-08

| ID | Case | Old code; old test | 1.0 acceptance | Guard |
|---|---|---|---|---|
| VR1 | The reader holds one record at a time and checks a length before it decodes or allocates, so a damaged or hostile file cannot exhaust memory | `crates/goalcup-verifier/src/capture_analysis/reader.rs:11-20`, `:146` | A: PRD-08 `check_invariants.rs` (a torn tail whose length runs past the end of the file is exit 2 for a sealed entry and allocates nothing of that length) | PRD-08 |
| VR2 | A panic inside a decoder printed nothing: the default hook writes the payload, which may hold record text, to stderr (contract S7, review N4) | `crates/goalcup-verifier/tests/m2_decoder_v1_quiet_panic.rs`; `capture_analysis/session.rs` (`quiet_panic_hook`) | A: PRD-08 and PRD-09 (a panic inside a check or inside `usage` prints file and line, never record text; the lane asks both owners for the named test in the night log) | PRD-08, PRD-09 |
| VR3 | Evidence problems are findings, never an error exit: exit 3 is only "no usable report" (input unreadable, bad command line) | `reader.rs:266-301`; `crates/goalcup-verifier/src/bin/goalcup-verify.rs:296-310` | A: PRD-08 `check_invariants.rs` (each mutation gives its id and exit 1 or 2, never 3) | PRD-08 |
| VR4 | The exit judges the record, not the run (D45): an END `interrupted` by the client and a stream no usage shape reads are research facts and never lower exit 0; only a loss does | D45; `crates/goalcup-verifier/tests/it/v5_corpus.rs` | A: PRD-08 `check_named.rs` (a clean record with an `interrupted` END exits 0; the lane asks PRD-08 for the case) | PRD-08 |

#### 2.16 Start, continue and status (SC). Guards live in PRD-10

| ID | Case | Old code; old test | 1.0 acceptance | Guard |
|---|---|---|---|---|
| SC1 | The project folder and goalcup's own folders must not hold each other, checked on the spelled and the resolved paths, so an agent that lists its folder's parent never sees the run's record (D5) | D5; `dm-it/capture_lifecycle.rs:2583` | A: PRD-10 `cli_folder.rs` (a project folder inside `$GOALCUP_HOME`, or one that holds it, also through a link, is refused; the lane asks PRD-10 for the case) | PRD-10 |
| SC2 | A failure before the launch leaves nothing behind: a Ctrl-C, a denial, an expired code or an absent server ends the start with one sentence and no run open | `cli/commands/start/mod.rs:22-37`, `:58-64`; `dm-it/capture_lifecycle.rs:1053` | A: PRD-10 `cli_folder.rs` (a refused start ends with "Nothing was created." and writes nothing). X: the abandon route is cut (PRD-07); a failed competition start is replaced by the next start | PRD-10, PRD-07 |
| SC3 | While the harness owns the terminal, goalcup writes nothing to it; every message goes to the log, and goalcup speaks again only after the harness exits | `cli/commands/start/mod.rs:10-19` | A: `lessons_launch::nothing_is_written_to_the_terminal_while_the_harness_runs` (pty: between the launch block and the harness's exit the pty carries only the harness's bytes) | PRD-17 |
| SC4 | A part ends; the run does not. When the harness exits before the deadline, the entry is sealed and uploaded and `goalcup continue` opens the next part | `cli/commands/start/mod.rs:39-56` | A: PRD-10 `cli_continue.rs`; R: PRD-10 acceptance 10 (continue resumes) | PRD-10 |
| SC5 | A sleep lock that cannot be taken never refuses or stops a run; status says it | `cli/commands/start/mod.rs:141-143` | R: PRD-10 acceptance 6 (the sleep line is a warning; no `PROBLEM:` line for sleep) | PRD-10 |
| SC6 | A seal that failed never reads as finished | `dm-it/capture_lifecycle.rs:2176` | A: PRD-10 `cli_finish.rs` (a `status.json` with no seal never prints the finished line; the lane asks PRD-10 for the case) | PRD-10 |
| SC7 | `goalcup status` trusted only a status document signed with the key `start` pinned, and refused one about another entry, because the status route was open to any local process | `cli/commands/status.rs:1-41` | X: signing and the control port are cut (ledger L5, L6); 1.0 status reads three local files and never talks to the recorder (PRD-10 section 4) | PRD-10 |

#### 2.17 DECISIONS.md, entry by entry

Every entry of `docs/DECISIONS.md` is either a lesson above or has no edge case left for 1.0, with the
reason.

| D | What it settled | Lesson, or "no edge case: why" |
|---|---|---|
| D1 | No base URL; a loopback proxy reached through the harness's `HTTPS_PROXY`; only listed hosts decrypted | Hard rule kept (AGENTS.md); HF8, T6, section 2.7 |
| D2 | Two legs, native and wire; neither erases a gap in the other | Section 2.9; R3 |
| D3 | Record raw, interpret later | No edge case: a design rule (ledger R8), kept by PRD-02 |
| D4 | One daemon at a time; one entry per workspace | HF4 (one run per isolated home); the daemon is gone (ledger L5) |
| D5 | The agent never sees its evidence | SC1 |
| D7 | Start flow (superseded) | No edge case: superseded by D37 and D57 |
| D9 | Gap policy: loss is stated, never hidden | R3 (ledger R4) |
| D10 | One harness per entry, checked by review | No edge case: a judging rule; the record stays raw (ledger R8) |
| D11 | The seal records the workspace | X: `seal.json` is gone (PRD-01); the start event carries `workspace_head` |
| D12 | Versions (superseded) | No edge case: superseded by D53 |
| D14 | Redaction at capture, before disk | K1, K9 (ledger M1) |
| D16 | The server fixes the clock | S7 |
| D22 | Kill the process tree; the Codex npm wrapper hides the binary | P7 |
| D23 | Harness tiers (retired) | No edge case: retired |
| D24 | The control API is authenticated | X: the control API is gone (ledger L5) |
| D25 | Status is for humans, never agent context | No edge case: a rule AGENTS.md keeps; D66 is P11 |
| D26 | Usage derived by the verifier; plan and credit fields stripped before publishing | K16 (the Codex plan and credit rules are dropped by M1); publishing is outside this build |
| D30 | `Confirmed` termination bound | X: termination proof is cut (ledger L6); P7 keeps ending the harness |
| D36 | Credential headers stripped; sign-in hosts never decrypted; decoded is raw | K6, HF8, D8 |
| D37 | Autonomous runs and research evidence | No edge case: an event rule (`docs/RULES.md`) |
| D38 | Fresh per-run HOME with a link to `Library/Keychains` | P13 |
| D39 | Extra patterns; launch template; shared Codex home serialised | HF4, K10, K11 |
| D40 | Finish keeps the termination grace; drain share and reserve | P8, P15 |
| D41 | Hermes CA trust and key fingerprint | HF2; the key fingerprint clause is superseded by D61 (no key forwarded) |
| D42 | Data-only harnesses, open harness PRs | No edge case: a process rule (ledger H7) |
| D43 | Harness contract v1; CA rule generalised | T6 (ruling 3); the rest is superseded or withdrawn |
| D44 | OpenCode's SQLite rows, read-only, after the run | F6 (SQLite sets copied hourly and at the seal) |
| D45 | The integrity exit judges the record, not the run | VR4 |
| D46 | Both local halves bound to the same connector bytes | X: no daemon (ledger L5) and no pins (ledger L6) |
| D47 | No DCO sign-off | No edge case: a contribution rule |
| D48 | The V1 path leaves the tree | No edge case: already gone |
| D49, D50 | `AMP_API_KEY`; `stdout_json` identity (superseded) | No edge case: superseded by D60 and D61 |
| D51 | A connector's argv and env are data; generic deny rules | X: deny lists are cut (ledger L6); the no-base-URL hard rule stays (PRD-11) |
| D52 | Shipped connectors set no model, effort, sandbox, permission or MCP limit | No edge case: connector data rule (PRD-11, ledger T5) |
| D53 | Record the harness build | No edge case: the start event carries `harness_version` (PRD-01) |
| D54 | Protocol v5 only; an unknown version is a version | FW4 |
| D55 | The chain rotates into part files | FW2, FW3 |
| D56 | The seal of a clone; the walk budget | X: `seal.json` is gone (PRD-01); `workspace_head` in the start event |
| D57 | Chat mode; every session file since the start | F1 |
| D58 | Producer IDs never run out | X: no producers (ledger L5); section 4 |
| D59 | `NODE_USE_ENV_PROXY`; a shared run home | P10 |
| D60 | The run's own secret by exact value | K13 |
| D61 | Hermes in chat mode from its own home; no key forwarded | HF3, P9 |
| D62 | The model host the harness's config names | Section 2.7 (N7), HF8 |
| D63 | A signed receipt describes its Launch | X: signing is cut (ledger L6) |
| D64 | No server error stops a run | S6 |
| D65 | Disk full: pause the agent | X: no disk-full handling (ledger L6, section 7) |
| D66 | `goalcup time-left`, pull only | P11 |
| D67 | Recommend no permission prompts; the participant sets them | HF7; PRD-10 setup writes `no_prompts` only with the participant's yes |
| D68 | The connector contract locked at schema 1 | No edge case: no backward compatibility (ledger L7) |
| D69 | goalcup holds no keys; push with the participant's login (partly superseded) | No edge case beyond D71 and D73 |
| D70 | The release is the list; old versions refused | No edge case: PRD-10 acceptance 8 (version floor) and PRD-07 keep it |
| D71 | The project folder is the workspace; the 30 s idle timer ran out at the approval | P14, SC1 |
| D72 | Hermes trust entry and a fresh gateway | HF1, HF2, HF3 |
| D73 | The push lines are never lost; `continue` resumes with no session file | HF6, SC4; the push block is PRD-10 `cli_push.rs` |
| D74 | The model host is the connector's default or the one address the config names | Section 2.7, G1 to G5 |

### 3. Copied verbatim versus rewritten

Function-level ledger for the parts the lessons depend on. PRD-03, PRD-05, PRD-02 and PRD-11 do the copying;
this table tells them which ranges must arrive unchanged.

| Old range | Lines | Fate | Into | Edits allowed |
|---|---:|---|---|---|
| `px/connect.rs:728-754` (`read_connect_head`), `:840-881` (authority, host and SNI normalising) | 69 | V | `src/proxy/connect.rs` | none |
| `px/connect.rs:962-1041` (`connect_host`, `connect_addresses`, `set_upstream_keepalive`) | 80 | V | `src/proxy/connect.rs` | the 25 s retry bound for listed targets replaces `connect_reserve` and `connect_upstream` (`:884-924`), which go |
| `px/connect.rs:1042-1213` (ClientHello peek and parse, `Reader`) | 172 | E | `src/proxy/connect.rs` | return the ALPN list and the fingerprint; guard 64 KiB |
| `px/connect.rs:1215-1280` (`PrefixedIo`) | 66 | V | `src/proxy/connect.rs` | none |
| `px/connect.rs:692-726` (fixed answers) | 35 | E | `src/proxy/connect.rs` | drop `407`; `parse_connect_head` (`:756-801`) loses its credential check (PRD-03 rule 5) |
| `px/tls.rs:100-215` (CA, leaf) | 116 | E | `src/proxy/ca.rs` | subtrees from the list; export and import; per-SNI leaf cache; ALPN set per connection |
| `px/tls.rs:216-294` (roots, provider) | 79 | E | `src/proxy/tls.rs` | add the OS store |
| `cli/ca_bundle.rs:1-207` | 207 | V | `src/proxy/ca.rs` | `handed_file` per PRD-03 |
| `px/http.rs:73-82`, `:776-1265` (accept key, head scan, fields, heads, framing, chunked) | 500 | V | `src/proxy/http1.rs` | `Fields` also keeps every header in wire order for meta |
| `px/http.rs:1514-2050` (decoders) | 537 | E | `src/proxy/decode.rs` | remove the per-read budget (`MAX_DECODED_PER_READ`) |
| `px/websocket.rs:193-265`, `:323-420`, `:673-689`, `:718-747`, `:776-827` (negotiation, framer, validity, unmask, inflate) | 297 | V | `src/proxy/ws.rs` | none |
| `px/websocket.rs:749-774` (`frame_end`) | 26 | E | `src/proxy/ws.rs` | output to `ws_up` or `ws_down` DATA with flags |
| Inline tests: `px/connect.rs:1487`, `:1540`; `px/http.rs:2081-2135`; `px/websocket.rs:1025` | about 200 | V | `tests/lessons_*.rs` | none |
| `px/headers.rs:20-75`, `:83-140`, `:186-270` (rules) | about 200 | W | `src/mask/` (PRD-05) | same rules, same-length `*` over JSON pairs |
| `dm/redact.rs`, `dm/redact/stream.rs` | 5,287 | E | `src/mask/` (PRD-05) | PRD-05 sections 2 to 6 |
| `pol/` `:120-138`, `:1001-1049`; `px/preflight.rs:169-192`; `cli/model_host.rs:836-893` | about 160 | V | `src/connector/hosts.rs` (PRD-11) | one entry point for all three host sources |
| `cli/model_host.rs:560-760` (file reader, YAML lookup) | about 200 | E | `src/connector/` (PRD-11) | `*` over keys per PRD-11 |
| `plat/terminal.rs:27-292` (constants and handler, `Shield`, `Modes`, `EchoOff`, `wait_resuming`) | 266 | V | `src/watchdog/shield.rs` (PRD-02) | none. Revision 1 said `:60-292`; PRD-02 checked that `Shield` needs the constants and handler at `:27-52` (`Shield` itself starts at `:53`). PRD-02 and PRD-13 now use `:27-292` |
| `cli/commands/start/proxy.rs:33-60` (open-file limit) | 28 | E | `src/watchdog/spawn.rs` (PRD-02) | recorder to the hard limit with the macOS clamp, harness to `max(soft, 4096)` (P6) |
| `cli/home.rs:591-620` (`.profile`), `:105-113` (lock name) | about 45 | V | `src/connector/` (PRD-11) | none |

Rewritten (behaviour kept, code new): the active relays in `px/http.rs` and `px/websocket.rs` become
passive parsers fed after each forward write (PRD-03); header streams become masked OPEN meta (PRD-01);
the tunnel side list becomes `conn` records at open; the restart supervisor becomes the watchdog's held
socket (PRD-02); counts move to the writer; a connection that stops parsing leaves one `conn` GAP to its
close (PRD-03 section 5a) instead of going opaque.

### 4. Inverted on purpose: old tests that must not be ported

| Old test | Lines | Why it is now wrong |
|---|---|---|
| `cli-it/m2_proxy_integration.rs:1846` `excess_connections_are_refused_predictably`, `:1997`, `:2077` | about 300 | No connection or stream cap |
| `cli-it/m2_proxy_integration.rs:2967`, `:2999` (restart cap) | about 60 | No restart cap |
| `cli-it/m2_tls_connect.rs:726` `http1_alpn_policy_is_explicit` | about 75 | ALPN is mirrored; HTTP/2 is parsed |
| `cli-it/m2_tls_connect.rs:339`, `:420`, `:1173` (proxy credential) | about 160 | The credential is cut (PRD-03 point 5) |
| `cli-it/m2_tls_connect.rs:1129` (admission deadline) | about 45 | No deadline after the CONNECT head |
| `cli-it/m2_http_relay.rs:1821`; `cli-it/m2_proxy_integration.rs:1153`, `:1194` | about 160 | Host check and SNI mismatch loss are gone |
| `cli-it/m2_http_relay.rs:1459` (only hinted paths count) | about 70 | Counts are POSTs and client messages |
| `cli-it/m2_websocket_relay.rs` message-index tests (`:1189`, `:1268`, `:1409`, `:1468`, `:1549`) | about 400 | One stream per direction; no message indexes or epochs |
| `cli-it/m2_capture_sender.rs` (1,991), `m2_emitter.rs` (1,517), `m2_ipc_compose.rs` (1,606) | 5,114 | No IPC, producers, epochs or failure budget |
| `cli/tests/full/t25_disk_full.rs` (1,173) | 1,173 | Disk full gets no special handling |
| `dm/tests/full/process_kill.rs` (1,285), `dm-it/terminate_sim.rs` (2,714) | 3,999 | Termination proof is cut |

## Files it owns

Create:

- `docs/plans/2026-10-30-v1.0/LESSONS.md`: the status table (section 1). About 170 lines (sections 2.1 to 2.16;
  section 2.17 maps DECISIONS to these rows and adds none).
- `crates/goalcup/tests/lessons_connect.rs`, `lessons_tls.rs`, `lessons_http1.rs`, `lessons_decode.rs`,
  `lessons_ws.rs`, `lessons_hosts.rs`, `lessons_launch.rs`. About 1,300 lines in all (estimate, inside
  PRD-14's 1,500 row), including about 200 lines of copied inline tests. Revision 1's
  `lessons_restart.rs` is cut: PRD-02's `watchdog_restart.rs` already tests the same promise.
  `lessons_launch.rs` spawns the watchdog under a pty made by the test itself (`openpty`; the child
  gets it as its controlling terminal), so it runs in a build box under `sandbox exec`, which is not a
  terminal. It goes in PRD-14's `heavy` group (`.config/nextest.toml`, a line PRD-14 adds on this PRD's
  request).
- `crates/goalcup/tests/lessons_fixtures/`: copies of `crates/goalcup-cli/tests/fixtures/m2-tls/*.hex`,
  `m2-http/sse-body.br` and `m2-websocket/rfc-examples.json` (about 12 KiB). If PRD-03's
  `tests/proxy_fixtures/` already holds a file, the lessons tests read it from there instead.

Must not touch: anything under `crates/goalcup/src/` (the module owners), `tests/common/` (PRD-14),
other PRDs' test files, `connectors/**`, `server/**`, the other PRDs, and the old crates (read only
until PRD-13 deletes them). Until `tests/common/` lands, a lessons file may keep a private helper,
removed at phase 3 (PRD-00 phase 0 step 8).

## Interfaces with other PRDs

| PRD | What this PRD needs or gives |
|---|---|
| PRD-01 | Gives FW2, FW5, FW7, FW8 and R5; PRD-01's `recovery.rs` and `golden.rs` are their guards. H10 and K15 read their part file with `goalcup_format::read` |
| PRD-02 | Gives P1, P2, P3 to P8, P15, K14, R1, R2, R4, C12, FW1, FW6 and HF3 as cases; PRD-02's tests are the guards for P1, P2, P6, P7, P8, P15, FW1, FW6. Needs the watchdog runnable from a test under a pty (P3 to P5, SC3); no test hook (plain `kill -9`). Needs a test constructor for the real pipeline (masker and writer) in a temp folder: `Recorder::real_for_tests(dir, masker)` (H10, K15). Copies `terminal.rs:27-292` (section 3). Calls `StreamMasker::boundary()` at each `fin` (K15) |
| PRD-03 | Gives sections 2.1 to 2.5. Needs `proxy::serve(listener, ProxyCtx { recorder, hosts, ca, extra_roots }) -> ProxyHandle` public (PRD-00 ruling), and the ranges in section 3 copied with only the listed edits. Section 5a is the fallback H2, H9, W2, W7 and W10 assert. The open-file raise is PRD-02's; PRD-03 only waits on `fds::may_accept()` (C11) |
| PRD-04 | Gives section 2.9; PRD-04's named tests are the guards |
| PRD-05 | Gives section 2.6; PRD-05's tests are the guards, except K15, which also has a whole-path lessons test. K12, K13 and K16 are stated drops; R8 is PRD-05's `mask_meta.rs` |
| PRD-06, PRD-07 | Give section 2.10; their tests are the guards. S7's segment side is PRD-06's `upload_beat.rs::clock_set_back_does_not_lengthen_a_segment`. S10 needs a named Worker case (a repeated open answers the same run): the lane names it in LESSONS.md, or asks PRD-07 in the night log |
| PRD-08 | H10 and K15 write a part file; their test also runs `goalcup-verify check` on it and asserts exit 0. No other lessons test writes one. Gives FW3, FW4, FW6, FW7, FW9, R3 and section 2.15. The lane asks PRD-08 in the night log to name four cases in its tests: a part folder whose first parts are gone exits 1 (FW3); a panic inside a check prints no record text (VR2); every mutation exits 1 or 2, never 3 (VR3); a clean record with an `interrupted` END exits 0 (VR4) |
| PRD-09 | G1: the usage reader picks the shape by path suffix, so gateway paths are read (PRD-12 P4's `gateway` rows; no `usage_shapes.rs`, ledger B10). VR2: a panic inside `usage` prints no record text (`usage_run1.rs`) |
| PRD-10 | P14, S7 (`cli_start.rs`), HF4 (`cli_commands.rs::second_start_on_one_home_refuses`), HF6 (`cli_continue.rs`), P11 (acceptance 7), section 2.16; the `local_model` status line (N8). `--record-host` calls PRD-11's one host check, so N1 to N6 hold for it. The lane asks PRD-10 in the night log to name SC1 in `cli_folder.rs` and SC6 in `cli_finish.rs` |
| PRD-11 | Section 2.7, the D rows of 2.8, HF1 to HF8 in 2.12 (HF5 guarded by `connector_launch.rs::codex_gets_no_dash_c_override`), and G2, G3. Uses `connector::hosts::check(name: &str, source: Source) -> Result<Host, Skipped>` (PRD-00 S3) for the shared file, `record-hosts.txt`, `--record-host` and hints, so `lessons_hosts.rs` tests one entry point. F8 dropped by PRD-11 (accepted here) |
| PRD-12 | Real-run lessons ride on pad rows (RR1 to RR6 below); HF7 is a pad rule; R7 is C1; G4, G5 are pad facts |
| PRD-13 | Deletes the old crates after the deletion gate in Acceptance. Its ledger uses `terminal.rs:27-292` (fixed) |
| PRD-14 | `tests/common/`: `common::upstream` (scripted TLS upstream, HTTP/1.1, WebSocket, held handshakes, `ProxyCtx.extra_roots`), `Recorder::for_tests(room)`, `common::home::guard()`, and a `common::recorder::real(dir)` helper that starts the pipeline with masking and returns the entry folder (H10, K15). `lessons_*.rs` run in the fast tier, `idle_tunnel_survives_120s` in the full tier, `lessons_launch` in the `heavy` group. Time rules of section 4 apply |
| PRD-15 | T9 (egress that re-signs TLS is unsupported) and K12 (no-colon URL token) go into the participant guide's known limits. `2026-10-vercel-gateway.md` is cited by T9, P9 and G1 to G5; history docs are kept in this build (PRD-00) |
| PRD-16 | `LESSONS.md` by H+3; the PRD-17 lane runs the deletion gate (Acceptance) at phase 2, before PRD-13 merges. If a check fails, the night log names each failing row, its lesson and its owning PRD. The owner fixes it on its branch and the lane runs the gate again after each fix merges, until it is green or `REPORT_AT`. The cut follows as soon as it is green; if it is not green at `REPORT_AT`, line 1 of `MORNING.md` reads "Not ready: old crates not deleted (ledger L3), deletion gate <state>" (PRD-16 section 5) |

## What is cut

This PRD deletes nothing. It lets PRD-13 delete the old crates by turning their edge cases into named
guards. The old tests that end with no lesson are in section 4 (about 11,600 lines). Every other old test
behind a lesson ends once its guard is green.

## Acceptance

Real-run proofs first. Each rides on a pad run that already happens (PRD-12); the lessons watcher adds no
run of its own.

| # | Proof | Lessons | Pass |
|---|---|---|---|
| RR1 | Codex, direct key, the pad prompt with subagents (PRD-12 P3) | W1, W4, W14, C7, K5, K15, R5, R8 | At least one `ws_up` and `ws_down` pair with `deflate: true`; every `ws_up` text message that ended whole and holds no `fail_closed` span parses as JSON (messages with a `fail_closed` span are counted and reported, not failed); zero `decode_failed`, zero `unsupported_encoding` and zero `parse_stopped` on listed hosts; any HTTP POST fallback decodes; `thread-id` readable in meta; PRD-05 A2 passes; `mask_failclosed_bytes` reported and no `fail_closed` span crosses a message |
| RR2 | Claude Code, direct key (PRD-12 P3) | D1, K6, K10 | Every `http_resp` with coding `br` has `decoded: true` and its DATA, outside `fail_closed` spans, parses as SSE events; zero `unsupported_encoding`; every `x-api-key` and `authorization` value is all `*` |
| RR3 | Hermes, direct key (PRD-12 P3) | T1, HF1 | Zero `tls_refused` on the listed host; written model requests above 0; the row records the version of the Python that runs Hermes (`$(dirname $(readlink -f $(command -v hermes)))/python --version`, or the interpreter in the Hermes shebang); Python 3.13 or later makes T1 a real check, earlier marks T1 "not shown in a real run" |
| RR4 | The kill test (PRD-01 acceptance 3, PRD-02 R1) | P1, P2 | The harness log has no `connection refused` and no certificate error after either kill; every new `conn` to a listed host after a restart is `decrypt` |
| RR5 | Any core-set pad run where the agent installs packages and uses git (the payments prompt does) | T5, C6 | npm, pip and git traffic are tunnels with `not_listed`; no tool result in the session files or the recorded model traffic holds a certificate error (`self-signed`, `unable to get local issuer`, `CERTIFICATE_VERIFY_FAILED`) |
| RR6 | The owner's Mac pad run (PRD-12 P9) | P6, P13, HF2 | The recorder's soft limit is raised (no `EINVAL` in the log); the Claude login is found; Hermes, if run, records |

Revision 1's hint smoke (Codex with `https://0.0.0.0:8443/v1` and a backslash authority) is cut: it spent
a real run on a pure function. `lessons_hosts.rs` guards N1 to N6, and PRD-11 acceptance 3 already runs
Codex with a config hint and a loopback value through the same `hosts::check`.

Automated, the gate for PRD-13's deletion (phase 2, run by the PRD-17 lane):

1. **A list check, not a name freeze.** Every row in sections 2.1 to 2.16 has an `A:`, `R:`, `D:` or
   `X:` entry in LESSONS.md's guard column. Every `A:` name of the form `file::test` appears in
   `cargo nextest list --workspace`; any other `A:` names a test file that exists (a Rust test file, or
   a Worker test under `server/worker/test/`); every `D:` file exists; every `X:` names a decision. A
   guard an owner renamed is fixed by the lane with a LESSONS.md edit and a night-log line; only a
   lesson with no guard at all fails the check. Run in a build box as `ubuntu`, with check 2.
2. All `lessons_*.rs` tests and every `A:` guard in LESSONS.md pass on integration in `gate fast`, in a build
   box as `ubuntu` (`idle_tunnel_survives_120s` in `gate full`).
3. Every `R:` row is `real-run pass`, or LESSONS.md says which pad row will show it and the night log
   names any that failed. Real-run rows do not block the deletion (the old code stays at `v0.7.0`); they
   block the release through PRD-12.
4. Every row with fate `X` or `I` names the owner decision or PRD that made it so.

## Risks and blind spots

- **The list may still miss a case.** Every old crate's source tree and tests were searched for their
  rules (module and doc comments, `D` citations, named tests, 23 fix commits), and every DECISIONS entry
  is mapped in section 2.17; not every one of the 103,000 lines was read. A case a pad run finds later is
  added as a row: the watcher checks the old code at `v0.7.0` first.
- **Fixtures are partly synthetic.** `sse-body.br` was made with the reference encoder, not taken from the
  Anthropic API; the WebSocket fixtures are RFC examples. Real proof is RR1 and RR2. PRD-03 should add a
  masked real Codex WebSocket capture from the phase 1a run.
- **The passive design changes failure modes.** The old relay paused the request side during an upgrade;
  the new one keeps forwarding and buffers a copy (PRD-03 section 5). W9 and W10 test the copy, not the
  pause. A client that streams a lot before the `101` hits the 1 MiB copy guard.
- **HTTP/2 has no old lessons.** Every HTTP/2 case is new code with only PRD-03's tests behind it.
  PRD-03 keeps it narrow (no push, no RFC 8441, no gRPC body) and ALPN prefers HTTP/1.1 until the
  HTTP/2 pad rows pass, so the core set runs on the HTTP/1.1 path these lessons guard.
- **The 1.0 fallbacks differ from 0.7's.** H2, H9, W2, W7 and W10 assert PRD-03's section 5a, not the
  old "one gap, then opaque". If PRD-03 changes 5a, these five tests change with it; the promise they
  hold (every later byte inside a named loss) does not.
- **Lost secret coverage by decision.** With the email rule gone, `https://<token>@host` with no colon is
  masked only if the token has a known prefix (K12). With the proxy credential gone, any local process can
  use the listener (D37 already assumed a single-user machine).
- **macOS lessons are not tested overnight.** P6, P13 and HF2 rest on the owner's Mac run. The P6 claim
  about `EINVAL` comes from Apple's `getrlimit(2)` page, unverified here.
- **D2 may never show in a pad run.** Codex zstd-encodes request bodies only with a ChatGPT sign-in; the pad
  uses an OpenAI key.
- **A request not forwarded** (H8) is recorded but did not reach the provider. PRD-01 now has the END
  tail `unsent`; if PRD-08 or PRD-12 forget to subtract it, a harness retry counts twice.
- **Guards owned by other PRDs** may be renamed or weakened during the build. The phase 2 list check
  finds a missing name; it cannot see a guard that still exists but no longer asserts the case. The
  lane reads each owner guard's assertions once before marking its row `green`.
- **Fewer lessons tests means more trust in other PRDs.** Revision 2 moved P1 and P2 fully to PRD-02 and
  PRD-03 tests to avoid testing one promise twice. If those PRDs slip to a later slice, the deletion
  gate waits for them.

## Overridable defaults

| Default | Value | Who may change it |
|---|---|---|
| Where a guard lives | The owning PRD's test when it names one, else `lessons_*.rs` | PRD-17 with the owning PRD |
| Real-run rows block deletion | No; they block release through PRD-12 | PRD-16 |
| Long idle test | 120 s, full tier only | PRD-14 |
| A renamed guard | A LESSONS.md edit by the PRD-17 lane, never a gate failure | PRD-17 |
| A hint that fails N1 to N6 | Skipped with a log line, never a refusal; `--record-host` refuses | PRD-10, PRD-11 |

## Builder notes

Order of work:

1. **Phase 0 (from H+0, read only on the old code; LESSONS.md by H+3, PRD-16).** Write LESSONS.md from
   section 2: every ID with its guard and `open`. Copy the fixtures and the inline tests named in section 3
   into `lessons_*.rs` behind the module they need. Keep them on `v1.0/prd-17-lessons`. `LESSONS.md`
   (every row `open`) merges alone by H+3. A lessons file, or a single test, merges only when it passes
   on integration in a build box; until then it stays on the branch and its row stays `open`. Never
   merge a failing or `#[ignore]` lessons test (C7's full-tier test merges once it passes in `gate full`).
2. **Phase 1, slice by slice.** Core slice: `lessons_hosts.rs` first (no network, pure functions, needs
   only PRD-11's `hosts::check`), then `lessons_decode.rs` and `lessons_http1.rs` against
   `common::upstream`, then `lessons_ws.rs` (K15 needs PRD-05's `boundary()`), `lessons_connect.rs` and
   `lessons_tls.rs`. Second slice: `lessons_launch.rs` once the watchdog runs. HTTP/2 has no lessons
   file. Mark each row `green` with its commit.
3. **Phase 1a.** Read the first real runs with RR1 and RR2's checks. A failure is a row: name the lesson,
   the PRD and the record.
4. **Phase 2.** Run the four gate checks in Acceptance; post the result in the night log; PRD-13 merges only
   after it. Checks 1 and 2 run in one build box as `ubuntu` on the integration head; the lane only reads
   their output. Check 1 is a short script, for example: take the guard column of LESSONS.md, keep names
   of the form `file::test`, and grep each in `cargo nextest list --workspace --message-format oneline`;
   test that every other `A:` and `D:` file exists. If a check fails, the night log names each failing
   row, its lesson and its owning PRD. The owner fixes it on its branch and the lane runs the gate again
   after each fix merges, until it is green or `REPORT_AT`. The cut follows as soon as it is green; if it
   is not green at `REPORT_AT`, line 1 of `MORNING.md` reads "Not ready: old crates not deleted (ledger
   L3), deletion gate <state>".

Traps from the old code and the audit:

- Feed the inflater even when a record is dropped (W4). If the WebSocket parser ever moves behind PRD-02's
  queue, every Codex message after the first drop is garbage.
- Do not count zstd done by "output buffer not full" alone (D2, review P4 M1).
- Read deflate parameters from the `101`, not the request (W2).
- Never put a raw input byte into an error, a log line or a gap tail (C2, K14).
- While the harness owns the terminal, write nothing to it (SC3).
- Python 3.13 needs the AKI (T1); rcgen does not add it by default (`use_authority_key_identifier_extension`).
- `SIG_IGN` survives `exec` (P3). Catch the terminal's signals; never ignore them.
- On macOS, clamp `RLIMIT_NOFILE` before `setrlimit` (P6).
- A lessons test checks records, not internals: run the real `proxy::serve`, a fake upstream and the record
  collector, and assert on decoded DATA, GAP reasons and END reasons ("test the promise, not the shape").
- No stopwatch below 5 s and no sleep (PRD-14). C6 counts connect rounds; W12 checks event order.
- The old inline tests copied as lessons tests used old types; keep their inputs and expected outcomes,
  rewrite only the calls.

How a watcher proves it in a real run: after each core-set pad run, run `goalcup-verify dump` on the entry
and check RR1 to RR5's conditions with `jq` over the dump (for example, every `open` with `skind`
`http_resp` and `coding` `br` has `decoded` true). Write the result per row in LESSONS.md.

## Asks of PRD-00 and PRD-01

Revision 1's six asks are ruled: PRD-01 added the END tail `unsent` and made `coding` a list (PRD-01
review answers, CC 33 and 34); PRD-00 took done item 3, the four-check deletion gate, the
`lessons_fixtures/` path and the CA on S8's first control line (PRD-00 "PRD-17 asks 1 to 4: Accepted").

Revision 2's one ask of PRD-00 is ruled too (PRD-00 R3-7, done): the registry row reads "95 to 99
rate-limited calls", a count. It stays below as history; nothing here is open. New asks of other PRDs
(sections 2.14 to 2.16) go through the night log, as the Interfaces rows say.

1. **Rate-limit registry row (ruled, R3-7).** "Numbers and names" says "95 to 99% of responses". The run report gives
   counts, not a share: 95 calls failed with `rate_limit_exceeded` (measure 9), about 97 `error` events
   and 99 `response.failed` messages (reconcile notes). Make it "95 to 99 rate-limited calls".

## Review answers

This section is history from revision 2. The body above, the ledger and "Alignment answers" win over it.
It may move to `REVIEW-HISTORY.md` in one later step by one agent (alignment X2-11). Ids X1 to X8 in it
are now HF1 to HF8.

Sources: the consistency check (CC), review A (A), review B (B), and the owner's relayed gateway
question. Rulings already made in PRD-00 and PRD-01 revision 2 are followed, not reopened. "Checked"
means checked in the old code or the run report.

| Item | What it asked | Answer and what changed here |
|---|---|---|
| CC 6 | S8 must carry the run CA | Accepted by PRD-00 (first control line). P2 now names it; asks closed |
| CC 9 | Owner of `tests/lessons_fixtures/` | Accepted. Header and "Files it owns" |
| CC 13 | Old crates during phases 0 to 3 | Followed: old crates leave the workspace in phase 0 and are read from disk (Today) |
| CC 15, 93 | Builds in sandboxes, as `ubuntu` | Accepted. Deletion gate check 2 runs in a build box as `ubuntu` |
| CC 18 | Done item 3 wording | Ruled by PRD-00 as asked; asks section closed |
| CC 33, 34 | `unsent` tail, `coding` as a list | Ruled by PRD-01. H3 and H8 now cite them, no longer as asks |
| CC 41, 66 | Proxy credential cut | Accepted. K13 hedge removed (no credential, guard "none"); section 3 drops the credential check from `parse_connect_head` |
| CC 42, 108 | P6 against PRD-03 rule 7 | Resolved: PRD-02 raises the limit, PRD-03 waits on `fds::may_accept()`. Interfaces rows PRD-02 and PRD-03 updated |
| CC 43 | `serve` signature | Accepted. Interfaces row PRD-03 uses `serve(listener, ProxyCtx { recorder, hosts, ca, extra_roots })` |
| CC 47 | Helpers and the harness end have no owner | Ruled: the watchdog (PRD-02). P7 and X3 now name `watchdog_end.rs` |
| CC 49 | P8 wants "one deadline" | Accepted as ruled: P15 states PRD-02's bound (drain 60 s, native finish 180 s) |
| CC 84 | One rate-limit range "95 to 99%" | Partly. One range, but as counts: checked in the run report, 95 failed calls (measure 9), about 97 `error` events and 99 `response.failed`; none is a share of responses. W14 says so; new ask to PRD-00 |
| CC 88 | Copy ledger `terminal.rs` range | Accepted with PRD-02's correction, checked: constants and handler are at `:27-52`, `Shield` starts at `:53`, so the copy is `:27-292` (266 lines, not 233). P5 cite narrowed to `Modes` `:129-157` |
| CC 102 | History docs other PRDs cite | Resolved by PRD-00 (kept in this build). `2026-10-vercel-gateway.md` is now cited by G1 to G5 too |
| CC 109 | `hosts::check` missing from PRD-11 | Done by PRD-11 and PRD-00 S3. Section 2.7 and Interfaces use the exact signature, `Result<Host, Skipped>` |
| A 2 | Parser loss on a reused connection is silent | Accepted for the lessons: H2, H9, W2, W7, W10 now assert PRD-03 section 5a (every later byte inside a named loss); R3 widened; RR1 requires zero `parse_stopped` on listed hosts |
| A 3 | Lessons case for fail-closed masking on one WebSocket direction | Accepted. New row K15 (checked `stream.rs:76-80`), whole-path test `lessons_ws::fail_closed_ends_at_message_end`, PRD-05 keeps the unit guard, RR1 reports `mask_failclosed_bytes` |
| A 5 | Panic text and core dumps | Partly here: K14 now also names PRD-02 `pipeline_secure.rs`; the code is PRD-02's |
| A 7 | Local model server never named | Accepted for the lessons: N2, N6, N8 and the section 2.7 intro say a hint that points here is a `local_model` note (PRD-11) |
| A 9 | SQLite subagents only at the seal | F6 rewritten: the 0.7 rule "read only after a confirmed exit" is inverted on purpose by PRD-04 (marked live copy, hourly copies) |
| A 10 | Decoder memory outside the guard | D7 names PRD-03's `decoder_window` guard |
| A 11 | Harness gets the hard file limit | Accepted for P6: recorder to the hard limit (macOS clamp), harness to `max(soft, 4096)`, guarded by `watchdog_rlimit.rs` |
| A 13 | Half-open upstream (C6) | Routed: PRD-03 took the reset on upstream failure and rejected `TCP_USER_TIMEOUT`. C6 notes rule 8; its guard is PRD-03's |
| A 1, 4, 6, 8, 12, 14 to 16 | Refusal learning, restarts, unlisted hosts, outliving helpers, mid-run secrets, clocks, outages, auto-update | No old lesson behind them; owned by PRD-02, 03, 05, 06, 10, 11. No change here |
| B A12 | Name check as a hard gate is brittle | Accepted. Gate check 1 is a list check over LESSONS.md's guard column run by this lane; a renamed guard is a LESSONS.md edit. Added a risk: a guard can exist and stop asserting the case, so the lane reads each owner guard once |
| B T6 | Revision 1's RR6 spends a real run on a pure function | Accepted. Cut; `lessons_hosts.rs` and PRD-11 acceptance 3 cover it. The Mac row, RR7 in revision 1, is now RR6, and the watcher adds no run of its own |
| B B1, A6 | Narrow HTTP/2, prefer HTTP/1.1 | Followed. Risks row says the core set runs on the HTTP/1.1 path these lessons guard; W14 notes `PreferH1` |
| B T2 | No hook-based proofs | Followed. No lessons test needs a hook; P1 moved to PRD-02's plain `kill -9` test |
| B M3 | `tests/common/` may land late | Accepted. A private helper until it lands, removed at phase 3 |
| B A10, M6 | Amendments file, night-log per lane | PRD-00 ruled (no `AMENDMENTS.md`; PRD-16 may split the log). Nothing to change here |
| PRD-14 section 4 (time rules) | No stopwatch under 5 s | Applied: C6 counts connect rounds, D7 asserts piece size and a GAP, W12 asserts event order; RSS only reported |
| Own check: C6 range | `:884-1040` mixed copied and cut code | Fixed, checked: copy `:962-1041`; `connect_reserve` and `connect_upstream` (`:884-924`) go |
| Own check: H3 | Content plus transfer coding | Checked `transfer()` (`http.rs:901-916`) and `for_codings` (`:1546-1557`): two codings are D5's GAP. Row says so |
| Own check: duplicate test | `lessons_restart.rs` tested what `watchdog_restart.rs` tests | Cut (less code, one guard per layer). About 100 lines fewer |
| PRD-11 F8 drop | Hermes compaction switch dropped | Accepted as X, with the unverified `state.db` point and a pad-row trigger to bring it back |
| Owner's gateway question | Is a gateway just an HTTPS endpoint, one system for all? | Yes, and the old code's own runs show it: new section 2.13 (G1 to G5). Every gateway failure on 28 Oct came from per-path rules, an unlisted host or a dropped key variable, not from the gateway. 1.0's one list and "record every path" cover them with guards that already exist. Dodgers use the same pipeline; their loss is named (PRD-03, PRD-12) |

## Alignment answers

Fix lists g1, g2 and g3 of the alignment pass, checked against the ledger, the PRDs they touch and the
old code. Each id with what was done here.

| Fix | What was done |
|---|---|
| F2 | Applied. Today has rows for the writer, finish and queue, the chain, seal, parts and status types, the verifier, and start, continue and status. New sections 2.14 (FW1 to FW9, guards in PRD-01, PRD-02, PRD-08), 2.15 (VR1 to VR4, PRD-08) and 2.16 (SC1 to SC7, PRD-10), and 2.17, one line per DECISIONS entry (a lesson id, or "no edge case" with the reason). The risk is reworded to say only what is true: the old trees were searched for their rules and every DECISIONS entry is mapped, not every line read; a case a pad run finds later becomes a row |
| F3 | Applied, with the codes of G3-17a. Every row in 2.9 to 2.13 has `A:`, `R:`, `D:` or `X:`. HF4 is `cli_commands.rs::second_start_on_one_home_refuses` (PRD-10); S7 is PRD-06 `upload_beat.rs::clock_set_back_does_not_lengthen_a_segment` plus PRD-10 `cli_start.rs` (two layers); HF5 is PRD-11 `connector_launch.rs::codex_gets_no_dash_c_override`. HF6: the fix named "PRD-10 section 11", which is the version section; the rule sits in PRD-10 section 3.1 step 3 and is tested by `cli_continue.rs`, so the row says that. Gate check 1 rewritten |
| F4 | Applied. X1 to X8 are HF1 to HF8 here, in RR3, RR6, the risks and the Interfaces rows; gate check 4 reads "Every row with fate `X` or `I`". PRD-10 and PRD-11 already cite HF. PRD-12's interfaces row (line 702) still says "X7"; that file's fixer owns it |
| P02-30 | Applied. R2 names `pipeline_guard.rs` and `pipeline_stall.rs` |
| X2-11 | Partly. This pass edits only this file, and many writers of one new file would race (as PRD-00 and the other PRDs answered), so `REVIEW-HISTORY.md` is not written here. "Review answers" now opens with a line saying it is history and that the body, the ledger and these answers win; the one revision 2 ask is marked ruled (R3-7). Every binding ruling is in the body. The move may run once, after every alignment pass, by one agent |
| G3-17a | Applied (with F3). Section 1 defines `A:`, `R:`, `D:` and `X:` for every table; T9, H12, K13, N9 and D8 got the code form too; check 1 needs one entry per row and checks `A:` names and `D:` files |
| G3-17b | Applied. Checks 1 and 2 run in one build box as `ubuntu` on the integration head (Builder notes step 4, Acceptance check 1); the lane only reads their output (ledger B1) |
| G3-17c | Applied. Builder notes step 1 and the sandboxes bullet: `LESSONS.md` merges alone by H+3; a lessons test merges only once it passes on integration in a build box. One stated exception, so the two rules do not clash: C7's full-tier test is `#[ignore]` by PRD-14's rule and merges once it passes in `gate full` |
| G3-X32 | Applied. Section 1 says H10 and K15 run through PRD-14's `common::recorder::real(dir)` over PRD-02's `Recorder::real_for_tests(dir, masker)` (both PRDs now name them) and read the part with `goalcup_format::read`; Interfaces rows PRD-02 and PRD-14 say the same |
| G3-17d | Applied. RR1 counts and reports messages with a `fail_closed` span instead of failing them; RR2 parses DATA outside `fail_closed` spans |
| G3-X6 | Applied. Builder notes step 4 and the Interfaces PRD-16 row: name each failing row, fix on the owner's branch, rerun after each fix until green or `REPORT_AT`; the `MORNING.md` line uses PRD-16 section 5's exact words ("... deletion gate <state>") |
| G3-17e | Partly. K16 is added and the masking bullet says (K12, K16). The fix's K6 text ("a name with `ratelimit` is not masked") and its `x-ratelimit-*` guard are rejected: PRD-05 section 7 now masks rate-limit headers whose name matches the name rule (PRD-00 owner question 11, default no exemption). K6 says that instead, and K16's guard is PRD-05's `mask_rules.rs` case that a Codex `rate_limits` object comes through byte for byte |
| G3-17f | Applied. Section 1 "Lesson ids": outside this file `PRD-17 H2`, in LESSONS.md `L-H2` |
| G3-17g | Applied. Each owner-decision bullet carries its ledger ids; a B2, B2a bullet is added |
| G3-17h | Applied. Section 1 states C7's 120 s idle hold as the one exception, `#[ignore]`, full tier (PRD-14 section 4) |
| G3-X33 | Applied. Interfaces PRD-08 row; PRD-01 and PRD-08 already say the same |
| G3-17i | Applied in T1 and RR3: the Python that runs Hermes, found through the `hermes` path or its shebang |
| G3-17j | Applied in RR5: tool results in the session files or the recorded traffic, with the three error texts |
| G3-17k | Partly, checked in the code at `4b226de2`. Applied: C9 `px/connect.rs:17-18`, T6 `PINNED_ENV` `:145`, P12 and the section 3 row `cli/home.rs:105-113` (the doc comment starts at 105, the function ends at 113; the fix said 106). Rejected: T2's `px/tls.rs:52-59` is right (`CA_VALIDITY` is line 55, `CA_CLOCK_SKEW` line 59), and H9's `px/http.rs:86` is right (`MAX_HEAD_BYTES` is line 86) |
| G3-17l | Applied. "Files it owns": the pty is made by the test (`openpty`, controlling terminal), so it runs under `sandbox exec` |
| G3-X39 | Applied. P11's real-run proof is PRD-10 acceptance 7 (`sh -lc 'goalcup time-left'` in the harness's `HOME`) |
| B10 sweep (30 Oct) | Applied. G1's usage guard is PRD-12 P4's `gateway` rows (R:), since PRD-14 cut `usage_shapes.rs`; VR2's usage half is a case in `usage_run1.rs` |

## Amendments

### 2026-10-30, replan after PRD-07 slice 1 (merge 4dcc1e7b)

- **Answer to the S10 question (lessons log, 10:45Z):** the Worker tests for "a repeated open answers
  the same run" are `server/worker/test/device.test.ts:156` ("repeat open with the same session_id
  returns the same run; another session_id is 409") and `server/worker/test/lifecycle.test.ts:222`
  ("a lost-reply repeat of the open is the same run, also after continue"), on integration at 4dcc1e7b.
  Both stay through PRD-07 step 3. `integration.test.ts:185` needs a receipt and goes with that route.

### 2026-10-30, replan after PRD-17 phase 0 slice (merge 1423f8f9)

- **Review nits A7 and B7 go into the lane's next `LESSONS.md` edit** (they came from the fix review
  and are not in 1423f8f9). In L-H11 and L-W11 the guard parentheses read `(PRD-03 rule 2 ...)`,
  which by the legend makes PRD-03 the owner, while the Owner column says PRD-17. Drop the
  parentheses in those two cells, as every other `lessons_` guard already has none (L-C1); the
  `lessons_` prefix names the owner and the rule 2 source stays in the 10:51Z decision line. Any
  status or gate script reads the owner from the Owner column, never from guard parentheses. Dropping
  the Owner column (PRD-17 section 1 lists only ID, guard, status, commit) is optional, not required.
- **H11, W11 and G1's recording half are this lane's tests now** (lessons log 10:51Z): PRD-03 has no
  rule 2 case, so `lessons_http1::every_request_on_a_decrypted_host_is_recorded` (a gateway-prefixed
  path, a Host mismatch, a non-POST method) and `lessons_ws::an_upgrade_on_any_path_is_recorded` are
  written here and count in the lessons row of the size budget. Section 2's "PRD-03 rule 2 tests" for
  H11, W11 and G1 reads as these two. PRD-03 writes no duplicate (PRD-03 note).
- **L-S10 is no longer pending:** the names are in the earlier note above (`device.test.ts:156`,
  `lifecycle.test.ts:222`). The next `LESSONS.md` edit puts them in and leaves 6 `(name pending)`
  rows (FW3, VR2, VR3, VR4, SC1, SC6), each now asked in its owner's Amendments (PRD-08, PRD-10).
- **HF2 answered from PRD-11's own text:** 1.0 writes no Hermes CA entry and no `HERMES_CA_BUNDLE`;
  PRD-11's macOS risk row and its answer L5-10 leave Hermes "not verified on macOS" (ledger T9, Linux
  first). L-HF2's guard stays `R: RR6` (Hermes, if run on the owner's Mac). Section 2's `D:
  connectors/hermes-cli.toml` entry for HF2 names no key and is not used. Check 1 holds, since the row
  has a guard.
- **The PRD-14 heavy-group ask is answered, with a change:** `.config/nextest.toml` on integration
  names `lessons_launch` only in a comment. Group `heavy` starts empty because nextest refuses a
  `binary()` filter that matches no binary, so the merge that brings `lessons_launch.rs` adds it to
  the heavy override (see the PRD-16 note for how that one line passes merge rule 1).
- **Size report** (`wc -l` on integration at 1423f8f9): merged PRD-17 code is 0 lines (no
  `tests/lessons_*.rs`, no `tests/lessons_fixtures/` yet) against the shared "Lessons (PRD-17), test
  support, `rule_code`, `e2e_run` (PRD-14)" tests row of 3,000; nothing else of that row is on
  integration yet. `LESSONS.md` is 197 lines of docs, outside the row. Its 154 rows match the 154 ids
  of sections 2.1 to 2.16.

### 2026-10-30, replan after PRD-17 upkeep slice 1 (merge 646737c6)

- **One guard name form, for the next `LESSONS.md` edit** (review A-1). A `file::test` entry is
  written without `.rs` (`recovery::a_sealed_entry_takes_nothing_more`, `upload_beat::...`), as the
  `lessons_` rows already are; a file-level entry keeps `.rs` (`recovery.rs (PRD-01)`). Fix L-FW2,
  L-FW5, L-FW8 and L-S7 to that form. The legend says Rust tests live under `crates/*/tests/`, not
  `crates/goalcup/tests/` only: `recovery.rs` is in `crates/goalcup-format/tests/`.
- **Phase 2 check 1 follows that form.** For a `file::test` entry it splits on `::` and looks for a
  line of `cargo nextest list --workspace --message-format oneline` whose binary id ends in
  `::<file>` and whose test name is `<test>`; for a file-level entry it tests that
  `crates/*/tests/<file>` or `server/worker/test/<file>` exists. A literal grep of `recovery.rs::...`
  would report a guard as missing when it is not.
- **L-FW5 names the test that asserts its first half** (review A-2): add
  `recovery::a_part_file_cut_at_any_byte_recovers_to_its_last_index_line` (one recovery frame per
  dead boot, `gaps.len() == 1`) beside the second-frame and sealed-entry tests.
- **The Commit cell names the commit the build box proved** when the guarded crate changed between
  the merge that brought the test and the proof (review A-3). For L-FW2, L-FW5 and L-FW8, `f5b0a6a2`
  stands: nothing in `goalcup-format` changed up to the proof at `15df7cf3`.
- **L-S10 gives files and owner only** (review B1): `A: server/worker/test/device.test.ts (PRD-07);
  A: server/worker/test/lifecycle.test.ts (PRD-07)`. The parenthesised text looked like an exact test
  title and was cut short; the exact titles stay in the 4dcc1e7b note above.
- **L-FW2 is green on its recovery half only** (review B2). The exit-74 half of FW2 (a write error
  ends the recorder with exit 74 and a `recorder_down` GAP, PRD-02 section 5) has no named PRD-02
  test; PRD-02's note asks for one. When it is named, the lane adds it to L-FW2. No lessons test is
  written for it.
- **Size report** (`wc -l` on integration at 646737c6): merged PRD-17 code is still 0 lines (no
  `tests/lessons_*.rs`, no `tests/lessons_fixtures/`) against the shared 3,000-line tests row.
  `LESSONS.md` stays 197 lines of docs (6 rows changed), outside the row.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-17 upkeep slice 2 (merge ae970618)

- **The next `LESSONS.md` edit trims the legend** (review B-4, nit). Cut the clause "the line above
  says where Rust and Worker test files live" from "One name form", and the example "such as
  `recovery.rs` in `crates/goalcup-format/tests/`" from "Guard codes"; the `recovery.rs (PRD-01)`
  example in "One name form" already shows both. Nothing else in the legend changes.
- **L-S1's Worker case has a new title on the PRD-07 lane** (upload-server log 11:47Z): "a retry of
  the same chunk is 200 with the very same stored answer, body unread" (`integration.test.ts`, step 3
  removes `ack`). The L-S1 cell needs no edit, since it names the file only. The L-S2, L-S3 and L-S10
  titles are unchanged on `v1.0/prd-07-worker`.
- **Phase 2 check 1 also reads the titles for file-level Worker entries.** A bare file name passes
  check 1 even if the guarded case left the file. For L-S1, L-S2, L-S3 and L-S10 the lane confirms,
  at the gate, that each title last written in the night log is still a test title in that file
  (exact string match), and logs a renamed title as a renamed guard. The `green` mark at `b474329d`
  stands until then; the gate box re-proves the Worker tests on the gate commit.
- **Size report** (`wc -l` on integration at ae970618): merged PRD-17 code is still 0 lines (no
  `tests/lessons_*.rs`, no `tests/lessons_fixtures/`) against the shared 3,000-line tests row.
  `LESSONS.md` is 203 lines of docs, outside the row; 6 rows green (L-FW2, L-FW5, L-FW8 at
  `f5b0a6a2`; L-S2, L-S3, L-S10 at `b474329d`), 6 rows still `(name pending)`.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-07 slice 2 (merge 2f4bc6a0)

- **L-S1's new title is on integration**: `server/worker/test/integration.test.ts:250` "a retry of the
  same chunk is 200 with the very same stored answer, body unread" (the upkeep 3 seam line at 12:11Z saw
  the old title at 5ec35393). Check 1 matches the new title. L-S2 (`integration.test.ts:278`), L-S3
  (`:542`) and L-S10 (`device.test.ts:150`, `lifecycle.test.ts:201`) keep their titles; only line
  numbers moved. The gate proved them on 2f4bc6a0: Worker `npm test` 253 pass, 0 fail.

### 2026-10-30, replan after PRD-10 slice 1 (merge d55030ee)

- **SC1 is load-bearing for a relative `GOALCUP_HOME`** (PRD-10 review A8). `cli::home()` makes a
  relative `GOALCUP_HOME` absolute against the current folder, so `GOALCUP_HOME=gc` started in the
  project folder becomes `<project>/gc`; only SC1's "a project folder that holds `$GOALCUP_HOME`"
  refusal stops it. When PRD-10 names the SC1 case in `cli_folder.rs` (core slice), the lane checks
  that a relative `GOALCUP_HOME` is one of its inputs before it marks L-SC1 green.
- SC1 and SC6 stay `(name pending)`: slice 1 wrote neither `cli_folder.rs` nor `cli_finish.rs`.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-11 slice 2 (merge b9fa50f8)

- `connector::hosts::check(name, Source) -> Result<Host, Skipped>` is on integration with its final
  signature, but as a stub (lowercase and label shape only). `lessons_hosts.rs` waits for PRD-11's hosts
  slice, which copies the full check (section 3); a lesson test against the stub would pass or fail for
  the wrong reason. `lessons_launch.rs` waits for the launch slice (lock, `.profile`) and S8.

### 2026-10-30, replan after PRD-08 slice 1 (merge d10feef7)

Names for `LESSONS.md` (the lessons lane writes them; judge 13:06Z):

- L-VR3: `crates/goalcup-verify/tests/check_invariants.rs`
  `vr3_every_mutation_fails_its_check_with_exit_2` (21 rows; each asserts its own check fails and exit
  2). The 12:49Z name is superseded.
- L-VR4: `crates/goalcup-verify/tests/check_named.rs` `clean_record_with_interrupted_end_exits_0`.
- L-FW3: the local half (`parts_pruned` named, exit 1) is built but has no test yet; PRD-08's next
  slice names it. L-VR2: the panic hook is built (file and line, exit 3), its test comes with the
  server slice (PRD-08 note). Both stay `(name pending)`.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-10 slice 2 (merge 56495dd6)

- **L-SC6 is named:** `crates/goalcup/tests/cli_finish.rs` `unsealed_status_never_reads_as_finished`.
  A `status.json` with no seal never yields the end screen's finished block, and status never
  prints the sealed state line. The lessons lane replaces its `(name pending)`.
- L-SC1 stays pending. `cli/folder.rs` and `cli_folder.rs` need neither S3 nor S8, so they may come
  in PRD-10's next slice, ahead of `start` (PRD-10 note). The check on the relative `GOALCUP_HOME`
  input from the slice 1 note still applies.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-14 slice 2 (merge 04886295)

- **`common::upstream` is on integration** (04886295) for TLS with a chosen ALPN, HTTP/1.1 (whole,
  chunked, SSE, codings), WebSocket with permessage-deflate, held handshakes and answers and close
  after N bytes. PRD-03's `decode`, `ws`, `tls` and `connect` leaf modules reached integration in
  0e411074, so `lessons_decode.rs` can land now; `lessons_http1.rs` waits only on `proxy::serve`
  (PRD-03 slice 2). HTTP/2 cases wait on PRD-14 slice 3; a TLS that chooses `h2`
  panics the fake's task today.
- `common::recorder::real` (H10, K15) is not on integration; it comes in PRD-14 slice 3 after PRD-02's
  `Recorder::real_for_tests`.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-03 core slice 1 (merge 0e411074)

- **`lessons_decode.rs` need not wait for `http1`.** `proxy::decode::{parse_codings, BodyDecoder,
  Step}` is on integration and driven alone over `Recorder::for_tests(room)`, as PRD-03's
  `tests/proxy_decode.rs` does. D1, D2, D3, D4 and D6 can be written against it now; D7's bomb test
  checks a `memory_guard` GAP in the record, so it waits for `http1` (PRD-03 slice 2). Reuse
  `tests/proxy_fixtures/anthropic-sse.br` and `sse-flushed.br` rather than adding copies, and do not
  repeat PRD-03's cases (each coding decodes, an unknown coding writes no body byte, a window over
  `room()` is not decoded).
- **`lessons_tls.rs` T1, T2, T4 and T5** can use `proxy::ca::RunCa` (`mint`, `leaf`, `write_files`,
  `export`, `import`) and `proxy::ca::certificates` now. T3 and every `lessons_connect` row drive
  `proxy::serve`, which comes in PRD-03 slice 2.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-08 slice 2 (merge 669f5d92)

Names for `LESSONS.md` (the lessons lane writes them; judge 14:56Z, 15:13Z):

- L-FW3 (local and server halves, plus a wrong `--local` path exiting 3):
  `crates/goalcup-verify/tests/server_compare.rs`
  `fw3_pruned_entry_is_named_and_server_copy_stands_in`. This replaces the `(name pending)`.
- L-FW9: `crates/goalcup-verify/tests/server_compare.rs` `fw9_one_flipped_byte_fails_the_copy`.
- L-VR2: `crates/goalcup-verify/tests/server_compare.rs`
  `vr2_seal_without_reason_or_totals_is_reported`, plus the `("inv5", m_mask_at_top)` row of
  `check_invariants.rs` `vr3_every_mutation_fails_its_check_with_exit_2`. Both inputs exited 3 before
  the fix. The panic hook itself (file and line, exit 3) is recorded as a review check, because no
  test input reaches it.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-11 slice 3 (merge c14d63b8)

- **`hosts::check` is the full rule set on integration**, so `lessons_hosts.rs` and the lessons
  fixtures can land against it. Facts the guards meet: N2 forms (`0.0.0.0`, `::`, mapped loopback)
  come back `Skipped { local: true }`, as do private and link-local addresses; N3 numeric spellings
  (`127.1`, `0x7f.1`) are refused with `local` false (PRD-11 note, known), so N3's guard asserts the
  refusal only. N6 uses `hosts::resolves_only_here(&[IpAddr])` (public), so the stub-resolver guard
  calls it directly. N1's userinfo and backslash handling is `hints::authority` (crate-private);
  its guard goes through `plan` with a hint file, as `connector_hints.rs` does. HF1 to HF8 rows whose
  guard is `connector_launch.rs` (lock, version probe, typed settings, never a link) are on
  integration at c14d63b8 and can turn green after a box run on the integration head.

### 2026-10-30, replan after PRD-10 slice 3 (merge f66ce4a5)

- **SC1 is named and green:** `crates/goalcup/tests/cli_folder.rs`
  `project_folder_and_goalcup_home_never_hold_each_other`. It refuses a project inside
  `$GOALCUP_HOME` and one that holds it, also through a link and with `..` in the spelled path; its
  inputs include the relative `GOALCUP_HOME=gc` of the A8 note, so the check the lane was asked to
  make is in the case. The "side by side" part of the rule has no input of its own in the case. Row 2.16 SC1 can point at it.
- **SC2's folder case is named and green:** `cli_folder.rs::refused_start_creates_nothing` (the
  refusal ends "Nothing was created.", the project's listing of names, sizes and times is unchanged,
  and neither `GOALCUP_HOME` nor its parent exists). SC2's other cases (Ctrl-C, a denial, an expired code, an absent
  server) wait for PRD-10's `approve.rs`. For the start slice the PRD-10 note fixes the order
  `connector::load`, `folder::check`, then `connector::plan`, because `plan` creates the isolated
  home, its lock file and the `update_off` rows; without that order this case would fail.
- PRD-10's SC row in this PRD's acceptance table (P14, S7, HF4, HF6 still to come) is unchanged.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-11 slice 4, part 1 (merge a96d9f71)

- **P11's A guard is on integration:** `connector_launch.rs`
  `with_home_swapped_a_login_shell_finds_goalcup` (PATH reset to `/usr/bin:/bin` inside `sh -lc`,
  then `command -v goalcup` finds it; goalcup's own `.profile` is replaced, any other is kept). It
  passed in the merge's gate box, so the P11 row can turn green with commit a96d9f71. The R guard
  (PRD-10 acceptance 7) only has a real target once a connector sets `HOME` (Amp, Droid, PRD-11
  slice 5).
- No locked ledger item is touched and no owner question comes from this slice.
