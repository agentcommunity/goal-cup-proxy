# Fix list for PRD-03-proxy.md (triage group 2)

Source reviews: align-X1-run1, align-X2-scope, align-X3-builder, align-PRD-00 to align-PRD-06.
The ledger (OWNER-DECISIONS.md) wins. An id shared by several fix lists carries the same change text in each;
make in this file only the part that names this file. Items are ordered by severity, then id.

Counts: 0 blocker, 9 major, 14 minor, 23 in all.

## G2-11 (major)
- Sources: X3-14, P03-3
- Section and lines here: Acceptance 10 (583-586); Risks row 'A connection that stops parsing' (627); Overridable defaults ALPN (645)
- Same id also in: PRD-08-verifier-checks
- Ledger item or rule: PRD-16 acceptance plan; T6 (10 USD per run); acceptance a watcher cannot run
- Change:
  PRD-03 acceptance 10: "**Load** (phase 6, PRD-16 A1 and A2, as PRD-16 runs them under the 10 USD cap). Pass on each: zero proxy GAPs of any kind on listed hosts, `memory_guard` included (PRD-16 X3); no `503` or refused connection; PRD-16 X1 holds per 10-minute bucket; recorder RSS reported. Connections cut by A2's planned kill are PRD-02's `recorder_down`, not a proxy GAP." Lines 586 and 645: "The ALPN default stays `PreferH1` in 1.0; no mirrored load run is scheduled (G2-25)." Make the risks row at 627 say the same.
  PRD-08 R6: point to the same runs (PRD-16 A1 and A2), not "A2 Codex".

## G2-23 (major)
- Sources: P03-1, X2-04 (part); P02-17 dropped as superseded
- Section and lines here: Section 7 'Decoder memory is guarded' (356-362); section 11 (437-444); Overridable defaults; Interfaces PRD-02 row (498); `proxy_http1.rs` and acceptance wording
- Same id also in: PRD-02-recorder-core
- Ledger item or rule: Ledger R5 (one stated guard), L2; PRD-00 R3-1
- Change:
  PRD-03 section 7, replace the counter paragraph: "Before a coded body is decoded, call `Recorder::room()`; if the window (`1 << WBITS` for brotli, the zstd frame window up to 8 MiB, 32 KiB for gzip and deflate) fits, hold `Recorder::reserve(window)` until the body ends. If it does not fit, the body is not decoded: GAP `memory_guard` with `{"guard":"decoder_window","limit":<room at that moment>}` (lost as G2-26)." Drop "256 MiB process-wide" from sections 7 and 11 and from Overridable defaults; add `Recorder::reserve` and `room` to the PRD-02 interfaces row; change "with the decoder guard set low" to "with `for_tests(room)` small".
  PRD-02 keeps rule 7, `reserve` and ask 4. Risks "Memory": "decoder windows sit inside the 512 MiB guard through `reserve`".

## G2-24 (major)
- Sources: P03-2, P02-4, P03-12, X2-04 (part)
- Section and lines here: Section 10 `ProxyHandle` (418-430); line 498; line 616; builder note 3 (663); Asks 3 (663-664); `proxy_restart.rs` (615-617)
- Same id also in: PRD-02-recorder-core
- Ledger item or rule: Ledger R2 (seal timing), L2 (no unused surface); lesson P8
- Change:
  PRD-03 section 10: `ProxyHandle` offers one accessor, `quiet_since() -> Mono`: the `Recorder::now()` of the last byte any relay moved. Open but silent connections do not count. Delete `idle()`, `stop_accepting()` and `open_conns()` from the text, line 498, line 616, builder note 3 and asks 3. `proxy_restart.rs`: test that a silent open connection does not delay `quiet_since`.
  PRD-02 step 1: "The quiet time comes from `ProxyHandle::quiet_since()`: the grace ends when it is 5 s old, or 30 s after the `exit` line." `after_stop` hosts stay from the writer's table.

## G2-26 (major)
- Sources: P01-8, P03-10
- Section and lines here: Section 7 (360-361)
- Same id also in: PRD-01-record-format
- Ledger item or rule: Ledger R4 (lost bytes where known), point 6
- Change:
  PRD-01 section 3, after the GAP table: "`lost` is counted in the stream's offset space (decoded bytes for HTTP bodies, inflated bytes for WebSocket). When that size is unknown, as for `unsupported_encoding` and a body not decoded under `decoder_window`, `lost` is 0 and the tail gives `{"wire": <encoded bytes>}`."
  PRD-03 line 361: "`lost` 0, tail `{"guard":"decoder_window","limit":<room>,"wire":n}`".

## G2-27 (major)
- Sources: P01-6, P03-18
- Section and lines here: Asks after revision 2 (653-667)
- Same id also in: PRD-01-record-format
- Ledger item or rule: PRD-00 ruling R3-4 and R3-5; builder misread
- Change:
  PRD-01 line 465 list: add `outlived`, `native_missing`, `parse_stopped`. After it: "`long_tunnel` facts: `host`, `why` (`stream` or `upload`), `conns`, `up`, `down`, `secs`. `parse_stopped`: `conn`, `reason`. `native_missing`: `row`. `outlived`: `count`, `programs`." Line 375: `"tunnel_hosts":{"github.com":{"conns":12,"up":40960,"down":2097152,"listed":null}}`; line 384: "a listed host's value adds `listed` (the GAP reason name)". Line 214 "Written by": "PRD-02, PRD-04". Line 776: add "GAP 10 `cut_at_seal`, note `native_missing`".
  PRD-03 asks 1 and 2: mark "ruled by PRD-00 R3-4 and R3-5; PRD-01 copies them (P01-6)".

## P03-4 (major)
- Sources: P03-4
- Section and lines here: Section 2 ClientHello (193); section 11 (442); section 9 (386-398)
- Same id also in: PRD-01-record-format, PRD-08-verifier-checks
- Ledger item or rule: Ledger R4 (named loss), point 6
- Change:
  PRD-03 section 2, after "Guard 64 KiB (`client_hello`); past it, a tunnel" add: "The CONNECT host decides. The `conn` gets the new tunnel reason `hello_too_big`; on a listed host its close GAP is `memory_guard` with `{"guard":"client_hello","limit":65536}`, `off` 0, `lost` = `up` + `down` + 1."
  PRD-01: add `hello_too_big` to the tunnel reasons and GAP 1 to the listed-tunnel GAP list. PRD-08: count it as `named`.

## P03-5 (major)
- Sources: P03-5
- Section and lines here: Section 9 tunnel reasons (388) and listed GAP list (395); section 6 (328-338)
- Same id also in: PRD-00-index, PRD-01-record-format
- Ledger item or rule: Ledger H6; builder misread (a decrypted connection cannot fall back to a tunnel)
- Change:
  PRD-03 section 9: remove `h2_unsupported` from the tunnel reason list and "11 `h2_unsupported`" from the listed-tunnel GAP list. Section 6, add: "`h2_unsupported` is never a tunnel reason: the connection is already decrypted, so it keeps relaying unparsed (5a)."
  PRD-00 third slice: "anything else is GAP `h2_unsupported` and the connection relays unparsed (PRD-03 5a)". PRD-01: drop `h2_unsupported` from the `conn` `reason` list.

## P03-6 (major)
- Sources: P03-6
- Section and lines here: Files it owns, tests paragraph (478-480)
- Ledger item or rule: Point 4, ledger M1, AGENTS.md hard rule on committed fixtures
- Change:
  Replace with: "a Codex-shaped WebSocket deflate fixture: take the inflated, masked `ws_up` and `ws_down` messages of a phase 1a record, re-deflate them with context takeover in the test, and commit only the masked plaintext source. It is reviewed by eye before commit (AGENTS.md). No raw wire capture is ever written to disk."

## P03-7 (major)
- Sources: P03-7
- Section and lines here: Acceptance 1 (547-548), 2 (551-552), 4 (562-563)
- Ledger item or rule: Ledger T4 (one set of checks), T8 (Hermes must pass)
- Change:
  In acceptance 1, 2 and 4 replace the comparison clause with "PRD-12 check C1 passes (PRD-08 `threads_match` and `wire_keeps_up`; Hermes by the per-session form, owner question 7)". Keep the proxy lines (decrypt, alpn, decoded, zero GAPs).

## G2-25 (minor)
- Sources: P03-13, X2-04 (part), X3-14 (part)
- Section and lines here: Section 4 item 4 (249-252); `proxy_tls.rs` (594-595); lines 586, 645; files table (457-475)
- Same id also in: PRD-00-index
- Ledger item or rule: Ledger L2 (less code); H6 is met by `PreferH1` serving h2-only clients
- Change:
  PRD-03 section 4 item 4: replace "A switch `Alpn::Mirror` ... with a night-log line." with "Mirroring the client's full list is not built in 1.0; `PreferH1` is the only ALPN mode. It may be added after the build once HTTP/2 rows pass." `proxy_tls.rs`: the matrix under `PreferH1` only. Files table total "about 4,400" becomes "about 4,250" (with G2-24); PRD-00 Proxy row the same.

## P03-11 (minor)
- Sources: P03-11
- Section and lines here: Section 4 item 4 (243, 252-256); `proxy_tls.rs` matrix
- Ledger item or rule: Point 6, R4 (named reason)
- Change:
  Add to section 4 item 4: "If upstream rejects the offered ALPN (alert `no_application_protocol`, or it picks none for an `h2`-only client), open a fresh TCP connection and tunnel with `reason: "alpn_other"` and GAP `protocol_other`." Add the case to the `proxy_tls.rs` matrix.

## P03-14 (minor)
- Sources: P03-14 (decided here: keep, state it)
- Section and lines here: Section 2 upstream TCP (181-183); Overridable defaults (644); Risks
- Ledger item or rule: Ledger R2 (never slow); point 3; a default the fix may decide
- Change:
  Keep the retry and add a Risks row: "A listed host outage holds the harness's CONNECT up to 25 s instead of failing fast; accepted because one slow answer beats a failed call (PRD-17 C6). Unlisted targets are not retried." Record the choice in PRD-15's decisions.

## P03-15 (minor)
- Sources: P03-15
- Section and lines here: Section 1 rule 3 (148-151)
- Same id also in: PRD-01-record-format
- Ledger item or rule: Point 5, R4
- Change:
  PRD-03: "Each non-`200` answer writes one `note` `connect_refused` `{host, status}` (host only when the head parsed), at most one per host and status per minute." PRD-01: add the note name.

## P03-16 (minor)
- Sources: P03-16
- Section and lines here: Section 3 (212-213)
- Ledger item or rule: Clarity (one list, H2)
- Change:
  Replace the examples with "`never` entries (PRD-11 section 4, plus goalcup's own server added in code)".

## P03-17 (minor)
- Sources: P03-17
- Section and lines here: Acceptance 4 (561-562); Builder notes (703-704); Risks 'Unverified facts' (638)
- Ledger item or rule: Ledger T7, clarity
- Change:
  "Kilo through Vercel AI Gateway by the route PRD-11 and PRD-12 cite (recheck the docs on the day, T7)".

## P03-19 (minor)
- Sources: P03-19
- Section and lines here: Section 5a item 2 (309-310); Asks 3 (663)
- Same id also in: PRD-02-recorder-core
- Ledger item or rule: Point 6
- Change:
  PRD-03 Asks 3, add: "PRD-02: set status `losing` while a `parse_stopped` connection is open, and clear it at that connection's close GAP." PRD-02 section 6: the same rule.

## P03-20 (minor)
- Sources: P03-20
- Section and lines here: Builder notes (671-681)
- Ledger item or rule: Ledger B1
- Change:
  Add: "Build and run every test here in a build box (Vercel Sandbox, ledger B1); the cloud session only writes and reviews code."

## P03-21 (minor)
- Sources: P03-21
- Section and lines here: Section 2 ClientHello (193)
- Ledger item or rule: Clarity
- Change:
  Add "(today 32 KiB, `connect.rs:75`; doubled for post-quantum key shares, PRD-17 C4)".

## P03-22 (minor)
- Sources: P03-22
- Section and lines here: Section 4 refusal learning (259-274); acceptance 7 (573-576)
- Same id also in: PRD-08-verifier-checks
- Ledger item or rule: Provable acceptance; point 6
- Change:
  PRD-03: "one `ca_refused` note each time a pair is learned"; acceptance 7's third call adds "and a second `ca_refused` note". The no-alert close gets GAP tail `{"alert": false}`. PRD-08: skip `refusal_after_trust` for a GAP with `alert: false`.

## P03-8 (minor)
- Sources: P03-8
- Section and lines here: Acceptance 1 (546-547)
- Ledger item or rule: Provable acceptance
- Change:
  "its DATA parses as SSE when `content-type` is `text/event-stream`, else as JSON".

## P03-9 (minor)
- Sources: P03-9
- Section and lines here: Interfaces PRD-01 row (497); section 7 (346-347)
- Same id also in: PRD-01-record-format
- Ledger item or rule: Ledger R4, clarity
- Change:
  PRD-03 section 7: "GAP `unsupported_encoding` with tail `{"codings": [...]}` (each token at most 32 bytes, at most 4 tokens)." Remove `unsupported_coding` from the PRD-01 row. PRD-01: give GAP 3 the `codings` tail key.

## P05-21 (minor)
- Sources: P05-21
- Section and lines here: Interfaces PRD-05 row (499)
- Same id also in: PRD-05-masking
- Ledger item or rule: Less code; clarity
- Change:
  PRD-03 row: "PRD-05 copies the header rules into `src/mask/meta.rs`; PRD-03 copies none." PRD-05 lines 488 and 502: "PRD-11's new Codex connector file has no rate-limit, plan or credit rows and its comment says headers are recorded".

## X2-11 (minor)
- Sources: X2-11
- Section and lines here: 'Review answers' section and fully ruled 'Asks' sections
- Same id also in: PRD-00-index, PRD-01-record-format, PRD-02-recorder-core, PRD-04-native-capture, PRD-05-masking, PRD-06-uploader-compression, PRD-07-worker-server, PRD-08-verifier-checks, PRD-09-usage-research-stats, PRD-10-cli-participant, PRD-11-connectors-harnesses, PRD-12-test-pad, PRD-13-cuts, PRD-14-tests-gate, PRD-15-docs-decisions, PRD-16-orchestration-acceptance, PRD-17-lessons-old-code
- Ledger item or rule: Ledger L2; point 6 (plain, true text). Apply last, after every other fix, so line numbers in the fix lists stay valid
- Change:
  Move every "Review answers" section and every fully ruled "Asks" section into one file `docs/plans/2026-10-30-v1.0/REVIEW-HISTORY.md`, verbatim, one heading per PRD. Before moving, copy any ruling still binding into the PRD body. Each PRD keeps one line at its end: "Review history: REVIEW-HISTORY.md, section PRD-NN." The replan keeps new asks in the Amendments note only.
