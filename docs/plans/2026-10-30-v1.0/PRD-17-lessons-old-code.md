# PRD-17: Lessons from the old code

Id: PRD-17. Version: goalcup 1.0. Date: 30 October 2026. Base: main at 4b226de2.
Index: [PRD-00](PRD-00-index.md). Format: [PRD-01](PRD-01-record-format.md). Owner of
`docs/plans/2026-10-30-v1.0/LESSONS.md`, `crates/goalcup/tests/lessons_*.rs` and
`crates/goalcup/tests/lessons_fixtures/`.

Revision 2, 30 October 2026: answers the consistency check, review A and review B, and follows PRD-00
and PRD-01 revision 2. Every item and its answer is in the last section.

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

- **Fresh core, proven parts copied** (BRIEF3, 29 Oct night): "Before deleting old crates, one agent
  extracts a lessons list from the old code, tests and DECISIONS.md ... and that list becomes acceptance
  cases. Old crates are deleted in the same build."
- **Option B, one process.** Lessons about the daemon hand-off (IPC, producers, epochs, failure budget) are
  kept only as "never again" rules (section 2.11), not as code.
- **No caps of our own.** Old tests that pin a cap (1,024 connections, 128 open streams, 32 native slots,
  5 restarts, 10 failures) are listed as inverted on purpose (section 4), never ported.
- **Masking option b.** The email rule and the Codex rate-limit rules go; their old tests are listed so the
  loss is a decision, not an accident (K12).
- **HTTP/2, gateways, one host list.** Old lessons about forcing HTTP/1.1, the Host check and the
  SNI-mismatch loss are inverted (section 4). Old lessons about URL authorities and loopback addresses now
  guard the one host list's inputs (section 2.7).
- **One system for every harness** (BRIEF3, 29 Oct night; the owner's gateway question of 30 Oct). A
  gateway is one more listed HTTPS host. The old code's gateway trouble came from per-path rules and an
  environment built from nothing, not from the gateway; section 2.13 keeps those lessons.
- **Build in sandboxes, in three slices** (BRIEF3, 30 Oct 02:00; PRD-00 build order). Lessons tests run
  in a build box as `ubuntu`, and each one waits for the slice that builds its module.
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

**Acceptance codes.** `A:` an automated test, named `file::test`. `R:` a real-run check on the pad
(PRD-12) or a named real run. A row may have both.

**One guard per lesson.** If the owning PRD already names a matching test in its own acceptance, that
test is the guard and PRD-17 writes nothing. Otherwise PRD-17 writes the guard in
`crates/goalcup/tests/lessons_*.rs`. A row names two guards only when they test different layers (a
unit in the owning PRD and the whole path through `proxy::serve` here, as in C10 and K15). No lesson is
tested twice at the same layer.

**How a lessons test is written** (PRD-14 section 4). It runs the real `proxy::serve` with
`Recorder::for_tests` and `common::upstream`, and asserts on records: decoded DATA, GAP reasons, END
reasons and tails. It waits on an event, never on a sleep, and has no time bound below 5 s; a
`within(30 s)` only guards a hang. Speed and memory figures are reported, never a pass line.

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
| C9 | A server-speaks-first protocol hung 30 s waiting for client bytes | `px/connect.rs:16-17` | A: PRD-03 `proxy_connect.rs` ("server-first protocol tunnels with no wait") | I (wait removed) | PRD-03 |
| C10 | A listed SNI inside a tunnel to another authority was a fail-open loss. In 1.0 it is decrypted with the upstream check made against the SNI name, so a local fake server can never pose as a listed host | `px/connect.rs:528-545`; `cli-it/m2_proxy_integration.rs:1153` | A: PRD-03 `proxy_connect.rs` (listed SNI behind an IP literal is decrypted) plus `lessons_connect::listed_sni_to_a_local_fake_server_is_never_trusted` (tunnel plus GAP, the client sees the fake's own certificate) | I, then W | PRD-03, PRD-17 |
| C11 | `accept` failing with `EMFILE` must pause and retry, never end the loop | `px/mod.rs:108`, `:1007-1011` | A: PRD-03 `proxy_connect.rs` (2,000 concurrent connections) and PRD-02 `watchdog_rlimit.rs` | V | PRD-03 |
| C12 | The listener is loopback only | `px/mod.rs:249-251` | A: PRD-02 `watchdog_restart.rs` asserts `127.0.0.1` | V | PRD-02 |

#### 2.2 TLS and the run CA (T)

| ID | Case | Old code; old test | 1.0 acceptance | Fate | Guard |
|---|---|---|---|---|---|
| T1 | Leaves carry the Authority Key Identifier. Python 3.13 refuses a leaf without it, so every Python harness and tool would fail | `px/tls.rs:17-20`, `:200`; `cli-it/m2_tls_connect.rs:164` (`key_ids`, leaf AKI equals CA SKI) | A: `lessons_tls::leaf_aki_names_the_ca_key`. R: RR3 (Hermes is Python; zero `tls_refused` on its listed host; the sandbox's Python version is in the row) | V | PRD-17 |
| T2 | The CA: `pathLen 0`, critical name constraints, one hour back-dated, 8 days valid, leaves with one DNS SAN and serverAuth. A leaked key cannot sign an unrelated name | `px/tls.rs:100-147`, `:169-215`, constants `:52-59`; `cli-it/m2_tls_connect.rs:164` | A: `lessons_tls::ca_constraints_validity_and_leaf_shape` (subtrees come from the host list, PRD-03 change 2) | E | PRD-17 |
| T3 | Upstream TLS is validated before the harness handshake. A bad upstream certificate is never laundered into a trusted leaf | `px/connect.rs:602-622`; `cli-it/m2_tls_connect.rs:225`, `:253` | A: PRD-03 `proxy_tls.rs` (upstream failure falls back to a tunnel with a GAP) plus `lessons_tls::untrusted_upstream_reaches_client_unchanged` (the client sees the upstream's own certificate) | W (fallback instead of refusal) | PRD-03, PRD-17 |
| T4 | CA files are created new, mode 0600, and an existing file or link at the path is refused, never followed. The key never touches disk | `px/tls.rs:158-178`; `cli-it/m2_tls_connect.rs:986` | A: `lessons_tls::ca_files_are_new_0600_and_never_follow_a_link`, and no `PRIVATE KEY` block in the entry folder | V | PRD-17 |
| T5 | A shared CA variable (`SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE`) replaces the roots of every tool child. It gets OS roots plus the run CA: macOS from the System Roots keychain, Linux from the first of five bundle paths. Only `CERTIFICATE` blocks that parse are kept, re-encoded from DER, so a private key in a source bundle never reaches the file | `cli/ca_bundle.rs:1-207` (`LINUX_BUNDLES` `:68-74`, `certificates` `:124-150`); `cli-it/ca_bundle.rs` (88) | A: copy `cli-it/ca_bundle.rs` as `lessons_tls::bundle_*`. R: RR5 (git and npm through tunnels work with the bundle set) | V (`handed_file` edited per PRD-03) | PRD-17 |
| T6 | `SSL_CERT_DIR`, `CURL_CA_BUNDLE` and `NODE_TLS_REJECT_UNAUTHORIZED` are never set (D43 ruling 3) | `pol/` `PINNED_ENV` `:146`, `TRUST_ONLY` `:151` | A: PRD-11 `connector_launch.rs` | D | PRD-11 |
| T7 | ALPN: a client offering no ALPN is HTTP/1.1 (WebSocket clients often offer none). Never offer the client a protocol the upstream did not choose | `px/tls.rs:24-31`, `:281-283`; `cli-it/m2_tls_connect.rs:726` (h2-only refused) | A: PRD-03 `proxy_tls.rs` ALPN mirror matrix | I (h2 refusal) and W | PRD-03 |
| T8 | The crypto provider is named explicitly (ring). rustls panics at first use when two providers are compiled in and no default is installed | `px/tls.rs:292-295` | A: any `proxy_tls.rs` test runs in a binary with the scaffold's features | V | PRD-03 |
| T9 | An egress that re-signs TLS (a corporate inspection proxy, a cloud session's egress proxy) blocked every run: the recorder trusted the Mozilla roots only and has no upstream proxy | `px/tls.rs:231-236`; `docs/research/results/2026-10-vercel-gateway.md:150-162` | R: none overnight (Vercel Sandbox has direct egress, same file `:12-14`). PRD-03 adds the OS store to upstream roots; an egress that needs its own proxy stays unsupported and is said in the docs | E, D | PRD-03, PRD-15 |

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
| H12 | A Host or absolute-form authority naming another host turned the connection opaque | `px/http.rs:11-16`; `cli-it/m2_http_relay.rs:1821`; `cli-it/m2_proxy_integration.rs:1194` | Not ported. PRD-03 records everything on a decrypted connection | I | PRD-03 |

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
| D8 | Decoding is done on the copy only; the harness gets the coded bytes it asked for | `cli-it/m2_http_relay.rs:539` | A: covered by every D test (the fake client checks the coded bytes) | W | PRD-17 |

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
| K6 | Header masking: names with `key`, `token`, `secret`, `password`, `auth`, plus `cookie` and `set-cookie`; `Bearer` or `Basic` in any header; URL userinfo up to the last `@`; query names percent-decoded (`%6Bey`), `sig`, `signature`; `://` after `?` is query; a bare CR masks the whole line; a folded line of a masked header is masked (fix commit `0a4d848e`). HTTP/2: `:path` gets the query rule | `px/headers.rs:20-75`, `:83-140`, `:186-270`; tests `px/headers.rs:277-402` | A: PRD-05 `mask_meta.rs` (rewritten from those tests). R: PRD-05 A3 | W (same-length `*`) | PRD-05 |
| K7 | Masked-key forms such as `sk-ab...wxyz` | `dm/redact.rs:209-216`; `dm-it/redaction.rs:998-1231` | A: PRD-05 `mask_rules.rs` | V | PRD-05 |
| K8 | Invalid UTF-8 around a secret; binary input; a key with invalid UTF-8 inside | `dm-it/redaction.rs:375`; `dm-it/capture_redaction.rs:2544`, `:3782` | A: PRD-05 `mask_stream.rs` | V | PRD-05 |
| K9 | Fail closed at the hold bound; an interrupt or gap never flushes a secret prefix | `dm/redact/stream.rs:125`; `dm-it/capture_redaction.rs:1112`, `:2806`, `:3397` | A: PRD-05 `mask_stream.rs` (fail-closed reproducers) | V | PRD-05 |
| K10 | Claude Code's billing header also appears inside the system prompt body | `connectors/claude-code-cli/harness.toml:155-159`; `dm-it/redaction.rs:352` | A: PRD-05 `mask_corpus.rs` | D | PRD-05, PRD-11 |
| K11 | Amp's WebSocket carries a `wsToken` in JSON | `connectors/amp-cli/harness.toml:102` | A: PRD-05 `mask_rules.rs` | D | PRD-05, PRD-11 |
| K12 | The email rule is dropped. It also caught a token used as the user of a URL with no password (`https://<token>@host`); `url_password` needs a colon. The old suite's URL credentials (81 and 190 characters) both have a colon and stay masked | `dm/redact.rs:280-287`; `dm-it/capture_redaction.rs:3580-3607` | A: PRD-05 `mask_rules.rs` keeps both old cases. The no-colon form is a stated loss (Risks) | X | PRD-05 |
| K13 | The run secret in every encoding (D60) goes with the proxy credential, cut everywhere (PRD-00, PRD-03 rule 5). PRD-05 still copies the encoding code of `run_secret.rs` for exact values | `crates/goalcup-protocol/src/run_secret.rs`; `dm-it/capture_redaction_run_secrets.rs` | None: no credential exists | X | none |
| K14 | Errors and log lines carry fixed text: no host path, header, body or key is formatted into one. A panic message is the same risk (review A finding 5) | `px/tls.rs:62-80`; `px/http.rs:46-51`; `px/connect.rs:29-40` | A: PRD-02 `pipeline_secure.rs` (a panic with a marker writes file and line, never the marker). R: PRD-12 C3 scans `goalcup.log` (S11), `status.json` and `upload.json` on every pad run | W | PRD-02, PRD-12 |
| K15 | A fail-closed mask window must end with the WebSocket message. In 0.7 each message was its own stream, so a window ended with the message; in 1.0 a direction is one stream, and Codex's compact JSON has no raw `\n`, so one overflow could star out hours of messages while every check stays green (review A finding 3) | `dm/redact/stream.rs:76-80` (window runs to `End` or `Gap` when no `\n` comes) | A: PRD-05 `mask_stream.rs` (the masker ends the window at `boundary()`) plus `lessons_ws::fail_closed_ends_at_message_end` (through `proxy::serve`: a 20 KiB undecided candidate in message 1 of `ws_up`, then 10 plain messages; message 2 onward is unmasked, the span is named `fail_closed`). R: RR1 reports `mask_failclosed_bytes` | W | PRD-05, PRD-17 |

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
| N9 | Path rules (`%2e`, dot segments, quotes, non-ASCII) mattered when paths selected what was recorded. In 1.0 a decrypted host records every path | `pol/` `unclean_path` `:1071-1076` | Not ported | X (no use left) | none |

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
| P11 | Codex runs every tool command as `/bin/sh -lc`. On Debian a login shell resets `PATH`, so with an empty run `HOME` the agent loses `~/.local/bin` (and `goalcup time-left`). The run home gets a `.profile` that puts the handed `PATH` back, written beside and renamed over, never following a link | `cli/home.rs:591-620`; `cli/launch.rs:337`; `docs/research/results/2026-10-22-fix-plan-v5.md:9-30` | A: PRD-11 `connector_launch.rs` (`sh -lc 'command -v goalcup'` finds it). R: PRD-10 acceptance 8 | V | PRD-11 |
| P12 | goalcup's lock beside a harness home must use a name no harness uses. `<connector>.lock` was exactly the lock Claude Code takes for its token refresh, so a start with an expired token stalled | `cli/home.rs:100-112`; `docs/research/results/2026-10-22-fix-plan-v6.md:1-25` | A: PRD-11 `connector_launch.rs` (lock name ends `.goalcup.lock`) | V | PRD-11 |
| P13 | macOS keychain: a scratch `HOME` hides the login keychain, so `gh` logins and Claude subscription logins are not found (real-machine round R2; D38 links `Library/Keychains`) | `docs/research/results/2026-10-24-real-machine-round.md:40-52`; D38 | R: the owner's Mac pad run (PRD-12 P9) | D | PRD-11 |
| P14 | The approval may take minutes. A 30 s idle exit before launch abandoned 8 test runs (real-machine round R1). Nothing before the launch may time out sooner than the approval code (15 min) | `docs/research/results/2026-10-24-real-machine-round.md:19-38` | A: PRD-10 `cli_commands.rs` (no timer under 15 min before launch) | W | PRD-10 |
| P15 | A stop meets one stated bound even with a stuck connection task. PRD-02 owns the bound at a deadline seal: drain 60 s, then native finish up to 180 s (PRD-00 ruling on consistency item 49); the upload wrap-up runs after the seal and never delays it | `cli-it/m2_proxy_integration.rs:1543`, `:2237` | A: PRD-02 `pipeline_seal.rs` | W | PRD-02 |

#### 2.9 Session files (F). Guards live in PRD-04

| ID | Case | Old code; old test | 1.0 acceptance | Guard |
|---|---|---|---|---|
| F1 | Admit a file by the later of mtime and ctime, in whole seconds: an agent can set mtime back, not ctime (D57) | `dm/native_capture.rs:23-30`; `dm-it/capture_native.rs:2261`, `:2296`, `:2320` | PRD-04 keeps the rule for every row; `native_walk.rs` (backdated mtime with a new ctime is taken; a file a second before the start is skipped) | PRD-04 |
| F2 | Truncate then append never reuses offsets; a replacement is a new generation. Truncate then regrow past the cursor between two polls is not detectable (stated) | `dm/native_capture.rs:55-61`; `dm-it/capture_native.rs:1321`, `:1278` | PRD-04 `native_tail.rs` | PRD-04 |
| F3 | No link followed at any level; a FIFO is never opened (the read would hang); no mount crossing | `plat/sourcecap/unix.rs:94-98`, `:185-190`, `:221-226`; `dm-it/capture_native.rs:631`, `:686`, `:2059` | PRD-04 `native_safety.rs` | PRD-04 |
| F4 | A partial last line is kept; a live EOF is not an end | `dm-it/capture_native.rs:566`, `:1238`, `:1710` | PRD-04 `native_tail.rs` | PRD-04 |
| F5 | Hermes rewrites its snapshot atomically during a read (a `source_changed` gap); `request_dump_*` is never matched (D61) | `dm-it/capture_native.rs:1962`, `:1906` | PRD-04 `native_snapshot.rs`, `native_safety.rs` | PRD-04 |
| F6 | 0.7 read a final snapshot only after a confirmed harness exit and kept nothing otherwise; the queue had to be drained between read passes or a few MiB latched the old queue (fix commit `cd3e541c`). 1.0 inverts the first half on purpose: a seal copy while the harness runs is taken and marked `harness_exited: false` with a `final_snapshot_live` note, and SQLite sets are also copied every 60 minutes when changed (review A finding 9). The second half becomes `room()`: native reads wait, nothing latches | `dm-it/capture_native.rs:2553`, `:2600` | PRD-04 `native_snapshot.rs` (seal copy while running is marked; periodic copy), `native_room.rs` (room 0 reads nothing and loses nothing) | PRD-04 |
| F7 | 32 native slots for 43 rollout files lost about 11 transcripts in run 1 | run 1 report, root cause 5 | PRD-04 acceptance 2 (one stream per rollout file) | PRD-04 |
| F8 | Hermes compaction in place keeps the session id with fewer messages and stops the JSON snapshot file; the 0.7 connector turns it off (`connectors/hermes-cli/harness.toml:49-56`, D61). Dropped by PRD-11 for 1.0: 1.0 reads `state.db`, not the JSON snapshot. Unverified whether in-place compaction also rewrites `state.db` rows; PRD-04's hourly copies keep earlier states if it does | D61 | X: none in code. R: the Hermes `full` pad row (PRD-12 P3); a loss there brings the switch back as data | PRD-11 |

#### 2.10 Upload and server (S). Guards live in PRD-06 and PRD-07

| ID | Case | Old code; old test | 1.0 acceptance | Guard |
|---|---|---|---|---|
| S1 | A retried chunk is answered from the stored ack without its body; a dropped reply leaves the chunk stored and the retry harmless | `server/worker/test/integration.test.ts:292`; `cli-it/upload_server.rs:514`, `:920` | PRD-07 keeps the Worker tests; PRD-06 `upload_resume.rs` | PRD-06, PRD-07 |
| S2 | A body that does not match `Goalcup-Sha256` is `422` and nothing is kept; header and size checks come before the body | `server/worker/test/integration.test.ts:320`; `cli-it/upload_server.rs:589` | PRD-07 Worker tests | PRD-07 |
| S3 | Past the deadline the server takes nothing but the final heartbeat (`503` with `Retry-After`) | `server/worker/test/integration.test.ts:638` | PRD-07 Worker tests; PRD-06 waits | PRD-07 |
| S4 | No ordinary heartbeat after, or while a lost final is asked again | `cli/upload/uploader/heartbeat.rs:15-20`; `cli-it/upload_chunks.rs:1563`, `:1599` | PRD-06 `upload_beat.rs` | PRD-06 |
| S5 | A `409` re-reads the server's position and moves only to a position the local stream confirms | `cli-it/upload_chunks.rs:901`, `:924` | PRD-06 `upload_resume.rs` | PRD-06 |
| S6 | No server answer stops the run (D64) | `cli-it/upload_chunks.rs:975` | PRD-06 U3 | PRD-06 |
| S7 | A clock set back a day must not lengthen a segment | `cli-it/upload_chunks.rs:1261` | PRD-10 or PRD-06, wherever the local deadline lives | PRD-10 |
| S8 | Run 1: 264 MB chunks and an 18-minute backoff left 7% sent at t+61 min; pruning at the seal removed the local copy before anyone verified it | run 1 report, findings 6 and 7 | PRD-06 U2 and U5 | PRD-06 |
| S9 | A heartbeat lost in a network cut waited for the next 5-minute beat (real-machine round F7) | `docs/research/results/2026-10-24-real-machine-round.md:111-112` | PRD-06 U3 | PRD-06 |
| S10 | A lost reply to the open is the same run (no second run) | `cli-it/upload_server_segments.rs:602` | PRD-07 Worker tests | PRD-07 |

#### 2.11 Run 1's cause chain: never again (R)

| ID | Rule | Old code | 1.0 guard |
|---|---|---|---|
| R1 | A drop under load never counts toward "the recorder is dead" | `px/ingest.rs:51-56`, `:104` (`MAX_CONNECTION_FAILURES`) | PRD-02 `pipeline_no_latch.rs` |
| R2 | A memory guard per stream with a gap, not one 8 MiB queue that drops a whole producer | `dm/runtime/capture/sink.rs` (`MAX_CAPTURE_QUEUE_BYTES`) | PRD-02 `pipeline_guard.rs`, R2 |
| R3 | Every loss is a GAP with a start and an end, including bytes relayed after a parser stop (PRD-03 section 5a) and after the seal (`after_stop`) | run 1: 23 gaps, all with one time and `lost_bytes` null | PRD-08 invariants; PRD-03 `proxy_http1.rs` fault case; H2, H9, W2, W7, W10 here |
| R4 | Count at write; count POSTs and client text and binary messages, never selections or interrupts | `record_generation_request` (run 1 report, root cause 4) | PRD-02 R3, PRD-10 acceptance 1 |
| R5 | One stream per WebSocket direction (152,728 server-message streams in run 1) | `px/websocket.rs:691-716` | PRD-01 golden and PRD-03 `proxy_ws.rs` |
| R6 | A decrypted connection that stopped recording was in no list: the sealed-host list showed no model host while model traffic went unrecorded | run 1 report, measure 3 | PRD-03 (a `conn` record at open) |
| R7 | "`intercepted=` above 0" is not proof of a model record | `docs/research/results/2026-10-vercel-gateway.md:23-25` | PRD-12 checks written requests against the harness's own log |
| R8 | Codex sends a `thread-id` header on every POST: subagent attribution needs it readable, not masked | `docs/plans/2026-10-30-audit/M-two-hour-run.md:947`, `:1046` | PRD-05 `mask_meta.rs` (name has no credential part), PRD-09 |

#### 2.12 Harness facts from DECISIONS and the measured runs (X). Data, PRD-11

| ID | Fact | Source | 1.0 guard |
|---|---|---|---|
| X1 | Hermes 0.22 runs every model call in a gateway process per `HERMES_HOME` that keeps the environment of whoever started it. A stale gateway sent every call around the proxy while status said "Problems: none" | D72; `docs/research/results/2026-10-26-mac-retest.md:28-30` | PRD-11 acceptance 5 |
| X2 | Hermes on macOS ignores `HERMES_CA_BUNDLE` and trusts a `providers` entry's `ssl_ca_cert`; its native Anthropic client ignores `ssl_ca_cert` | D72 | PRD-11 data; the owner's Mac run |
| X3 | `hermes chat --continue` exits 1 with no session; 0.19.0 has no `gateway stop -y` (exit 2) | D72, D73 | PRD-11 data; the watchdog runs `plan.before` and `plan.after` (PRD-02 `watchdog_end.rs`); a failed helper command is logged and the run goes on |
| X4 | Codex refresh tokens rotate: two runs on one Codex home race the refresh, so runs on a shared home are serialised (D39) | D39 | PRD-10 or PRD-11: one run per isolated home at a time |
| X5 | Codex `-c` overrides force its embedded mode (startup warning in run 1) | run 1 report, measure 9 | PRD-11 connector data |
| X6 | Claude Code writes its session file at startup, before any message | D73 | PRD-10 participant text |
| X7 | A typed prompt was swallowed by a first-run dialog, a welcome screen or a split paste, and the row still passed | `docs/research/results/2026-10-22-fix-plan-v2.md:11-28` | PRD-12 (a row passes only if the agent worked) |
| X8 | A sign-in host (OAuth token endpoint) is never decrypted; under a replace-mode CA its refresh failed (D62). PRD-03 gives replace-mode variables the full bundle | D62; `connectors/codex-cli/harness.toml:32` | PRD-11 `connector_hosts.rs` (`never` wins) |

#### 2.13 Gateways and the sandbox (G). Measured on 28 Oct

A gateway is an HTTPS model endpoint with another host name and sometimes another path. The measured
gateway runs failed only where 0.7 had per-host or per-path rules, or built the environment from
nothing. 1.0's one host list (gateways listed with `kind: gateway`) and "record every path" (H11) remove
both, so these rows add no new code, only guards that already exist.

| ID | Fact | Source | 1.0 guard |
|---|---|---|---|
| G1 | Gateways use their own path prefixes (`/claude-code/v1/messages`, `/codex/v1/responses`). 0.7's decoder took only the exact `/v1/messages`, so 19 recorded calls were labelled undecoded | `docs/research/results/2026-10-vercel-gateway.md:56-61` | Recording: H11 (every path on a decrypted host). Usage: PRD-09 picks the shape by path suffix and body, never by host |
| G2 | A gateway host on no list was tunnelled (`intercepted=20`, zero wire model requests) | same file `:23-25`, `:127`, `:139` | PRD-11 `model-hosts.toml` lists `ai-gateway.vercel.sh`, OpenRouter and Cloudflare AI Gateway as `kind: gateway`; PRD-12 P4 checks `listed: shared` with no participant or config entry |
| G3 | Codex with `env_key` for the gateway failed before any call: the key variable was dropped | same file `:203-208` | P9 (PRD-11 `connector_launch.rs`) |
| G4 | Hermes with a bare `provider: custom` gateway entry sends no key: the calls are recorded and answered `401`. Its built-in `ai-gateway` provider works | same file `:14-17`, `:125-126` | PRD-12 gateway case config (Hermes uses `ai-gateway`); a provider refusal is not our failure (PRD-12 P6) |
| G5 | A Vercel Sandbox `stop` keeps a disk snapshot for 30 days, keys included; `sandbox exec` is not a terminal, so `goalcup start` must run in `tmux` | same file `:108-113` | PRD-12 pad boxes are `--non-persistent` and run in `tmux` |

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
| `cli/home.rs:591-620` (`.profile`), `:100-112` (lock name) | about 45 | V | `src/connector/` (PRD-11) | none |

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

- `docs/plans/2026-10-30-v1.0/LESSONS.md`: the status table (section 1). About 130 lines.
- `crates/goalcup/tests/lessons_connect.rs`, `lessons_tls.rs`, `lessons_http1.rs`, `lessons_decode.rs`,
  `lessons_ws.rs`, `lessons_hosts.rs`, `lessons_launch.rs`. About 1,300 lines in all (estimate, inside
  PRD-14's 1,500 row), including about 200 lines of copied inline tests. Revision 1's
  `lessons_restart.rs` is cut: PRD-02's `watchdog_restart.rs` already tests the same promise.
  `lessons_launch.rs` spawns the watchdog under a pty, so it goes in PRD-14's `heavy` group
  (`.config/nextest.toml`, a line PRD-14 adds on this PRD's request).
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
| PRD-02 | Gives P1, P2, P3 to P8, P15, K14, R1, R2, R4, C12 as cases; PRD-02's tests are the guards for P1, P2, P6, P7, P8, P15. Needs the watchdog runnable from a test under a pty (P3 to P5); no test hook (plain `kill -9`). Copies `terminal.rs:27-292` (section 3). Calls `StreamMasker::boundary()` at each `fin` (K15) |
| PRD-03 | Gives sections 2.1 to 2.5. Needs `proxy::serve(listener, ProxyCtx { recorder, hosts, ca, extra_roots }) -> ProxyHandle` public (PRD-00 ruling), and the ranges in section 3 copied with only the listed edits. Section 5a is the fallback H2, H9, W2, W7 and W10 assert. The open-file raise is PRD-02's; PRD-03 only waits on `fds::may_accept()` (C11) |
| PRD-04 | Gives section 2.9; PRD-04's named tests are the guards |
| PRD-05 | Gives section 2.6; PRD-05's tests are the guards, except K15, which also has a whole-path lessons test. K12 and K13 are stated drops |
| PRD-06, PRD-07 | Give section 2.10; their tests are the guards |
| PRD-08 | Lesson records go through `goalcup-verify check` where a test writes a part file |
| PRD-09 | G1: the usage reader picks the shape by path suffix, so gateway paths are read |
| PRD-10 | P14, S7, X6; the `local_model` status line (N8). `--record-host` calls PRD-11's one host check, so N1 to N6 hold for it |
| PRD-11 | Section 2.7, the D rows of 2.8 and 2.12, and G2, G3. Uses `connector::hosts::check(name: &str, source: Source) -> Result<Host, Skipped>` (PRD-00 S3) for the shared file, `record-hosts.txt`, `--record-host` and hints, so `lessons_hosts.rs` tests one entry point. F8 dropped by PRD-11 (accepted here) |
| PRD-12 | Real-run lessons ride on pad rows (RR1 to RR6 below); X7 is a pad rule; G4, G5 are pad facts |
| PRD-13 | Deletes the old crates after the deletion gate in Acceptance. Its ledger uses `terminal.rs:27-292` (fixed) |
| PRD-14 | `tests/common/`: `common::upstream` (scripted TLS upstream, HTTP/1.1, WebSocket, held handshakes, `ProxyCtx.extra_roots`), `Recorder::for_tests(room)`, `common::home::guard()`. `lessons_*.rs` run in the fast tier, `idle_tunnel_survives_120s` in the full tier, `lessons_launch` in the `heavy` group. Time rules of section 4 apply |
| PRD-15 | T9 (egress that re-signs TLS is unsupported) and K12 (no-colon URL token) go into the participant guide's known limits. `2026-10-vercel-gateway.md` is cited by T9, P9 and G1 to G5; history docs are kept in this build (PRD-00) |
| PRD-16 | `LESSONS.md` by H+3; the PRD-17 lane runs the deletion gate (Acceptance) at phase 2, before PRD-13 merges |

## What is cut

This PRD deletes nothing. It lets PRD-13 delete the old crates by turning their edge cases into named
guards. The old tests that end with no lesson are in section 4 (about 11,600 lines). Every other old test
behind a lesson ends once its guard is green.

## Acceptance

Real-run proofs first. Each rides on a pad run that already happens (PRD-12); the lessons watcher adds no
run of its own.

| # | Proof | Lessons | Pass |
|---|---|---|---|
| RR1 | Codex, direct key, the pad prompt with subagents (PRD-12 P3) | W1, W4, W14, C7, K5, K15, R5, R8 | At least one `ws_up` and `ws_down` pair with `deflate: true`; every `ws_up` text message that ended whole parses as JSON; zero `decode_failed`, zero `unsupported_encoding` and zero `parse_stopped` on listed hosts; any HTTP POST fallback decodes; `thread-id` readable in meta; PRD-05 A2 passes; `mask_failclosed_bytes` reported and no `fail_closed` span crosses a message |
| RR2 | Claude Code, direct key (PRD-12 P3) | D1, K6, K10 | Every `http_resp` with coding `br` has `decoded: true` and its DATA parses as SSE events; zero `unsupported_encoding`; every `x-api-key` and `authorization` value is all `*` |
| RR3 | Hermes, direct key (PRD-12 P3) | T1, X1 | Zero `tls_refused` on the listed host; written model requests above 0; the row records the sandbox's Python version (Python 3.13 or later makes T1 a real check; earlier marks T1 "not shown in a real run") |
| RR4 | The kill test (PRD-01 acceptance 3, PRD-02 R1) | P1, P2 | The harness log has no `connection refused` and no certificate error after either kill; every new `conn` to a listed host after a restart is `decrypt` |
| RR5 | Any core-set pad run where the agent installs packages and uses git (the payments prompt does) | T5, C6 | npm, pip and git traffic are tunnels with `not_listed`; the harness log shows no certificate error from a tool |
| RR6 | The owner's Mac pad run (PRD-12 P9) | P6, P13, X2 | The recorder's soft limit is raised (no `EINVAL` in the log); the Claude login is found; Hermes, if run, records |

Revision 1's hint smoke (Codex with `https://0.0.0.0:8443/v1` and a backslash authority) is cut: it spent
a real run on a pure function. `lessons_hosts.rs` guards N1 to N6, and PRD-11 acceptance 3 already runs
Codex with a config hint and a loopback value through the same `hosts::check`.

Automated, the gate for PRD-13's deletion (phase 2, run by the PRD-17 lane):

1. **A list check, not a name freeze.** Every row in section 2 with an `A:` guard has a guard in
   LESSONS.md's guard column, and each name there appears in `cargo nextest list --workspace` (or
   the file exists, for a file-level guard). A guard an owner renamed is fixed by the lane with a
   LESSONS.md edit and a night-log line; only a lesson with no test at all fails the check.
2. All `lessons_*.rs` tests and every guard in LESSONS.md pass on integration in `gate fast`, in a build
   box as `ubuntu` (`idle_tunnel_survives_120s` in `gate full`).
3. Every `R:` row is `real-run pass`, or LESSONS.md says which pad row will show it and the night log
   names any that failed. Real-run rows do not block the deletion (the old code stays at `v0.7.0`); they
   block the release through PRD-12.
4. Every `X` and `I` row names the owner decision or PRD that made it so.

## Risks and blind spots

- **The list is a sample.** It comes from the tests, DECISIONS, the measured runs and 23 fix commits, not
  from every line of 103,000. Mitigation: the old code stays at `v0.7.0`; a watcher who finds a new failure
  in a pad run checks the old code first and adds a row.
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
- **macOS lessons are not tested overnight.** P6, P13 and X2 rest on the owner's Mac run. The P6 claim
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
   into `lessons_*.rs` behind the module they need; they compile once the scaffold stubs exist (PRD-02).
2. **Phase 1, slice by slice.** Core slice: `lessons_hosts.rs` first (no network, pure functions, needs
   only PRD-11's `hosts::check`), then `lessons_decode.rs` and `lessons_http1.rs` against
   `common::upstream`, then `lessons_ws.rs` (K15 needs PRD-05's `boundary()`), `lessons_connect.rs` and
   `lessons_tls.rs`. Second slice: `lessons_launch.rs` once the watchdog runs. HTTP/2 has no lessons
   file. Mark each row `green` with its commit.
3. **Phase 1a.** Read the first real runs with RR1 and RR2's checks. A failure is a row: name the lesson,
   the PRD and the record.
4. **Phase 2.** Run the four gate checks in Acceptance; post the result in the night log; PRD-13 merges only
   after it. Check 1 is a short script in the lane, for example: take the guard column of LESSONS.md, keep
   names of the form `file::test`, and grep each in `cargo nextest list --workspace --message-format
   oneline`.

Traps from the old code and the audit:

- Feed the inflater even when a record is dropped (W4). If the WebSocket parser ever moves behind PRD-02's
  queue, every Codex message after the first drop is garbage.
- Do not count zstd done by "output buffer not full" alone (D2, review P4 M1).
- Read deflate parameters from the `101`, not the request (W2).
- Never put a raw input byte into an error, a log line or a gap tail (C2, K14).
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

One new ask of PRD-00:

1. **Rate-limit registry row.** "Numbers and names" says "95 to 99% of responses". The run report gives
   counts, not a share: 95 calls failed with `rate_limit_exceeded` (measure 9), about 97 `error` events
   and 99 `response.failed` messages (reconcile notes). Make it "95 to 99 rate-limited calls".

## Review answers

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
