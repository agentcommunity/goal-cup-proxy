# PRD-03: The recording proxy

Id: PRD-03. Version: goalcup 1.0. Base: main at 4b226de2. Index: [PRD-00](PRD-00-index.md).
Format: [PRD-01](PRD-01-record-format.md). Owner of `crates/goalcup/src/proxy/`,
`crates/goalcup/tests/proxy_*.rs` and `crates/goalcup/tests/proxy_fixtures/`.
Revision 2, 30 October 2026: acts on every PRD-03 item of the consistency check and reviews A and B, and
follows PRD-00 and PRD-01 revision 2 (section "Review answers" at the end).
Revision 3, 30 October 2026: alignment fixes against the ledger (section "Alignment answers" at the end).

## Goal

One loopback proxy for every harness: decrypt and record byte for byte every connection to a listed
model host, relay every other connection untouched as a recorded tunnel, and never refuse or slow a call.

## Points served

1 (every model call, any provider, gateways included), 2 (subagent traffic shares the same connections and
hosts), 3 (relay first, no caps, every failure falls back to a working tunnel), 5 (every connection is a
record), 7 (HTTP/2, WebSocket, gateways, the harnesses people use). Point 4 through plaintext: every body is
decoded before PRD-05 masks it, and no compressed or encoded body byte ever reaches disk.

## Owner decisions it carries out

- **One system for every harness, no patchwork** (ledger H1 to H4). One proxy for every connection.
  One shared host list (providers, gateways, cloud model services, vendor backends), plus hosts the
  participant registers, plus hosts the harness config names as a hint. Those are decrypted and recorded
  byte for byte. Everything else is relayed untouched and recorded as a tunnel. A long streaming tunnel to
  an unknown host is flagged. Never decrypt every host.
- **The owner's 30 Oct message:** a gateway is an HTTPS proxy hop to a different API address; "strong
  pads for HTTPS proxy and gateway, and dodgers get to use the current way". This PRD makes the first two
  one code path and gives dodgers the light path (section "Answer to the owner's gateway question").
- **HTTP/2 is supported** (ledger H6). Pinned certificates fall back to a recorded tunnel with a named
  gap (H5).
- **Proxy failure** (R3): the recorder restarts on the same port fast, writes a gap for the dead time and
  never blocks the harness. PRD-02 owns the watchdog; this PRD makes the proxy restartable and hands
  PRD-02 what it needs for a graceful hand-over.
- **Option B, one process** (L5). The proxy calls the recorder handle in its own process. No IPC, no
  producers, no epochs, no failure budget.
- **One recording per WebSocket direction**, raw bytes, light framing (F1, PRD-01).
- **No caps of our own** (R5). Only stated memory guards; past one, a gap, and recording goes on.
- **No silent loss** (R4): a connection that stops parsing leaves a gap from the fault to its close.
- **Fresh core** (L3): copy the proven tricky parts (CA and TLS setup, HTTP/1.1 parsing, brotli, gzip,
  deflate, zstd, WebSocket deflate), delete the rest with the old crates.
- Never set a model base URL (`AGENTS.md:81`, D1, D36). The only wire path is `HTTPS_PROXY`.

## Answer to the owner's gateway question

"When it's a gateway, isn't it still an HTTPS proxy, just a different API endpoint?" **Yes.** Through
`HTTPS_PROXY` the harness sends `CONNECT ai-gateway.vercel.sh:443`, exactly as it sends
`CONNECT api.deepseek.com:443` with a direct key. The proxy sees a host name and nothing else, so "with a
gateway" and "without a gateway" are one code path. The only fact needed is that the gateway's host is on
the list. Kilo under Vercel AI Gateway is solved the moment `ai-gateway.vercel.sh` is on the shared list
(it is: PRD-11 section 4). A gateway that is not on the list is covered by the participant registering it
(`goalcup setup --record-host`) or by the harness config naming it (a hint). What differs between gateways
is inside the bytes (model names like `vendor/model`, extra usage fields, SSE keep-alive comments); the
recorder never reads that, PRD-09 does after upload. The gateway's own call to the provider runs on the
gateway's servers, so the bytes the agent saw are the bytes we have.

Why a list and not "decrypt everything": decryption only works for a client that trusts the run CA. The
harness does (we hand it the CA). The tools the agent runs (git, curl with its own bundle, boto3 with
`certifi`, Go and Java programs, gRPC with its own roots) often do not, and would fail (point 3). The list
keeps decryption to hosts that carry model traffic. Everything else still leaves a record (host, bytes,
times), and a host that looks like model traffic is flagged once (section 9), so a missed model host is
visible, not silent.

Dodgers are not a second system, and the owner is right that the lighter core already covers them. A
harness that ignores `HTTPS_PROXY` never reaches us: the native reader (PRD-04) and an honest "no model
requests written" (PRD-10) are what it gets. A client that pins or refuses our CA reaches us and gets a
recorded tunnel with a named gap. A vendor backend (Amp, Cursor) is a listed host like any other: its own
protocol is recorded byte for byte, and the provider call it makes on its servers is out of reach by design.

So the pad's strong cases are `direct` and `gateway`, with the same proofs (acceptance 1, 2 and 4: the
three core harnesses through Vercel AI Gateway, whose key is provided), and the `dodger` case proves the
light path (acceptance 6).

## Today

| What | Where | Fact |
|---|---|---|
| Proxy module | `crates/goalcup-cli/src/proxy/` (12 files) | 10,561 lines: 9,212 src, 1,349 inline tests. Integration tests `m2_*` 12,131 lines plus 4,027 support lines (`docs/plans/2026-10-30-audit/C-module-audits.md` section 3.2) |
| HTTP/1.1 only, both legs | `proxy/tls.rs:24-31`, `:213`, `:275`; refusal `proxy/connect.rs:624-637` | An `h2`-only client gets a `no_application_protocol` alert and no service. Cursor's server can force HTTP/2 (`O-harness-access.md` H7.3) |
| Upstream first, then the client | `proxy/connect.rs:610-633` | Upstream TLS is checked before the harness handshake, so a bad upstream certificate is never laundered into a trusted leaf (PRD-17 T3). Kept |
| Client refuses our CA | `proxy/connect.rs:628-633` | `Failed(ClientTls)`, the connection is dropped. Every retry fails the same way. No fallback |
| Upstream certificate fails our check | `proxy/connect.rs:622`; roots `proxy/tls.rs:231-236` (Mozilla set only) | `Failed(UpstreamTls)`. A gateway behind a private or corporate CA blocks the harness (`E-providers.md` E7, last row) |
| SNI and CONNECT host must agree | `proxy/connect.rs:528-579` | A listed SNI behind another authority is a fail-open tunnel loss; no SNI is a tunnel loss |
| Server-speaks-first protocols | `proxy/connect.rs:16-17` | The tunnel waits for client bytes until the 30 s admission deadline, then fails |
| Connection cap | `proxy/mod.rs:112` (`MAX_CONNECTION_TASKS = 1024`), refusal `:991-997` | The 1,025th live connection gets a `503` |
| Capture table | `proxy/mod.rs:1445-1470`, `MAX_OPEN_CONNECTIONS = 128` | Connections past 128 are relayed uncaptured |
| Stream limit ends capture | `proxy/api.rs:451-453` | The 129th open stream disables the epoch for the launch |
| Failure budget | `proxy/ingest.rs:51-56`, `:104` (`MAX_CONNECTION_FAILURES = 10`) | Ended run 1's recording at 15:43:15 (run report, "Root cause") |
| Restart supervisor | `proxy/mod.rs:1022` (`MAX_RECORDER_RESTARTS = 5`) | Restarts only the accept loop, not a dead process |
| Host check inside a connection | `proxy/http.rs:11-16` | A request whose Host names another host turns the connection opaque |
| Decode bound | `proxy/http.rs:93-97` (`MAX_DECODED_PER_READ` = 8 MiB queue), zstd window `:102` | Past it, a `LimitExceeded` gap |
| WebSocket | `proxy/websocket.rs:10-23` | One stream per message (152,728 server messages in run 1), control frames never recorded |
| Tunnel side list | `proxy/tunnels.rs:20` (`MAX_TUNNEL_NOTES = 10,000`) | Written only at finish, outside the chain, capped |
| Header masking | `proxy/headers.rs:23-30` | By name parts `key`, `token`, `secret`, `password`, `auth`, plus cookies and Bearer or Basic values |
| One runtime thread | `crates/goalcup-cli/src/commands/start/proxy.rs:98` | All TLS, relays and decoders share one thread |
| Host rule | `commands/start/proxy.rs:153-188`; `model_host.rs:1-25` | Connector defaults plus the one address the config names (D74). No shared list |
| CA | `proxy/tls.rs:100-147` | Per launch, key in memory only, name constrained (`:131`), 8 days (`:55`), AKI on leaves (`:200`, Python 3.13 needs it) |
| Upstream keepalive | `proxy/connect.rs:91-98`, `:1030-1037` | 60 s idle, 15 s interval, 4 probes. Kept |

Run 1 facts this PRD answers: the proxy relayed every call but recorded none after 15:43:15 (failure budget);
every WebSocket server message was its own stream (418 MB of overhead); no model host was ever missing
from the list, but the sealed-host list had no model host while model traffic went unrecorded
(`run-report` measure 3). Codex used WebSocket with permessage-deflate and HTTP POST fallback, both on
`api.openai.com`.

## Design

### 1. Shape

```
harness --TCP--> 127.0.0.1:<port> (socket held by the watchdog, PRD-02)
  read CONNECT head
  connect TCP to the CONNECT target (retried for a listed target)
  write "200 Connection established"
  wait for the first bytes from either side (no deadline)
    server first                     -> tunnel (not_tls)
    plain HTTP request, listed host  -> HTTP/1.1 parser, conn tls null
    other non-TLS bytes              -> tunnel (not_tls)
    ClientHello past 64 KiB          -> tunnel (hello_too_big), + GAP memory_guard if the CONNECT host is listed
    ClientHello                      -> read SNI, ALPN list, fingerprint
       decide(SNI, else CONNECT host)
         Never (sign-in host)               -> tunnel (sign_in)
         NotListed                          -> tunnel (not_listed)
         Listed, pinned                     -> tunnel + GAP pinned
         Listed, learned refusal            -> tunnel + GAP tls_refused
         Listed, no h2 or http/1.1 offered  -> tunnel + GAP protocol_other (alpn_other)
         Listed                             -> TLS upstream, ALPN by the rule in section 4
            upstream TLS fails              -> new TCP, tunnel + GAP upstream_untrusted
            serve the client a leaf from the run CA, ALPN = what upstream chose
               client alert on our leaf     -> learn (host, fingerprint), GAP tls_refused, close
               client closes, no alert      -> GAP tls_refused {"alert": false}, close, nothing learned
               ALPN h2                      -> pump + passive HTTP/2 parser
               ALPN http/1.1 or none        -> pump + passive HTTP/1.1 parser (WebSocket on 101)
```

Rules that hold everywhere:

1. **Relay first, record second.** Each direction is a pump: read a chunk, write it to the other side
   unchanged, then hand the same bytes to that connection's parser. The parser runs inline after the write
   (decoding stays on the relay, PRD-02 section 2 rule 5) and calls PRD-02's recorder handle (seam S1),
   which never waits. Nothing on the forwarding path waits for recording. Bytes are never re-encoded: both
   TLS legs carry the same plaintext, so even HTTP/2 frames and flow control pass through untouched.
2. **Record everything on a decrypted connection.** No path filter, no Host check, no method filter, no
   per-exchange selection. The old Host-mismatch fallback (`http.rs:11-16`) and evidence paths
   (`connectors/codex-cli/harness.toml:121-130`) go.
3. **Never refuse a connection the harness makes.** No connection cap, no stream cap, no admission
   deadline after the CONNECT head, no failure budget. Every TLS or protocol failure falls back to a
   working tunnel or keeps relaying unparsed. The only answers that are not `200` are for broken CONNECT
   requests (`400`, `431`), a non-CONNECT request (`405`, see Overridable defaults), and an upstream that
   cannot be reached at all (`502`, the same failure the harness would see without us). Each non-`200`
   answer writes one `note` `connect_refused` `{host, status}` (host only when the head parsed), at most
   one per host and status per minute; no `conn` stream, since no connection exists.
4. **A parser bug never touches the relay.** Each parser call runs inside `catch_unwind`. A panic ends
   parsing for that connection as in section 5a (GAP `decode_failed`, tail `{"panic":true}`) and the pump
   goes on. The workspace keeps `panic = "unwind"` (PRD-14).
5. **No proxy credential.** `HTTPS_PROXY` is `http://127.0.0.1:<port>` (PRD-00 S8, PRD-11). The
   credential guarded the listener against other local users (`connect.rs:30-40`), but it gives nothing a
   local attacker can use (the CA key sits only in memory and in one 0600 file in a 0700 folder, ledger R3b) and it cost a run secret the masker had to hunt
   in every encoding (D60, `docs/DECISIONS.md:510-526`) plus a `407` for any client that drops proxy
   userinfo. Cut everywhere.
6. **Relay threads: 2 by default** (PRD-00 ruling; PRD-02 section 1 does its memory and CPU math on it).
   Masking, writing and the native reader run elsewhere, so the relay threads only do TLS, framing and
   decoding. An overridable default, raised if acceptance 10 shows relay lag.
7. **Wait, never refuse, on file descriptors.** The watchdog raises the open-file limit (PRD-02
   section 10). Before each `accept` the loop asks `pipeline::fds::may_accept()`; while it is false it
   waits 10 ms and asks again. An `EMFILE` from `accept` itself is the fallback: pause 10 ms, retry, never
   end the loop (PRD-17 C11). New connections wait in the kernel backlog.
8. **An upstream failure reaches the harness as a failure.** When the upstream leg errors (reset,
   keepalive timeout, TLS error mid-stream), the proxy closes the harness leg with a TCP reset
   (`SO_LINGER` 0), not a clean close, so the harness sees what it would see without us and retries at
   once (review A finding 13). A clean upstream close is passed on as a clean close.
9. **Every timer uses `Recorder::now()`** (boot time, counts a suspend), checked on the relay's own
   events or a 1 s tick: the long-tunnel windows and the refusal expiry (PRD-00 rules of the shape).

### 2. Admission

- **CONNECT head:** copy `connect.rs:728-797` (head read, request line, `host:port` and `[ipv6]:port`,
  normalization `:840-881`) without the credential check. Head guard 8 KiB (`:71`), read deadline 30 s.
  Bytes past the blank line (a client that sends its ClientHello with the head) are replayed into the
  chosen path (PRD-17 C1). A broken head is never echoed (C2).
- **Upstream TCP:** copy `connect_host` and `connect_addresses` (`connect.rs:962-1041`): each round
  re-resolves, IPv4 first, `TCP_NODELAY`, keepalive 60 s / 15 s / 4. Retry until 25 s when the CONNECT
  target is listed (a short outage costs a slower answer, not a `502`; the cost is a Risks row); one
  round otherwise, because retrying the agent's own traffic changes the run being measured (C6). No idle
  timeout anywhere: SSE and WebSocket streams idle legitimately (`connect.rs:960-961`). No `TCP_USER_TIMEOUT`: the harness's
  own socket would wait the same kernel time without us (rule 8 passes the failure on when it comes).
- **First bytes:** after `200`, race the client's first bytes against the server's. Server first means
  a tunnel (`reason: not_tls`). This fixes the 30 s hang for server-speaks-first protocols.
- **ClientHello:** copy `peek_client_hello` and `parse_client_hello` (`connect.rs:1042-1213`; it already
  joins a ClientHello split across TLS records). Extend it to return the ALPN list and a **fingerprint**:
  SHA-256 of the legacy version, the cipher suites sorted, the extension types sorted, the supported groups
  sorted, all with GREASE values removed (RFC 8701). Sorting makes it stable for TLS stacks that shuffle
  extensions. The bytes read are replayed into whichever path is chosen (`PrefixedIo`, `connect.rs:1215-1280`).
  Guard 64 KiB (`client_hello`; today 32 KiB, `connect.rs:75`, doubled for post-quantum key shares,
  PRD-17 C4); past it, a tunnel and the CONNECT host decides. The `conn` gets tunnel reason
  `hello_too_big`; on a listed host its close GAP is `memory_guard` with
  `{"guard":"client_hello","limit":65536}`, `off` 0, `lost` = `up` + `down` + 1 (PRD-01 section 4).
  `conn` meta `fingerprint` holds the first 16 hex.
- **The deciding name is the SNI.** When there is no SNI, the CONNECT host decides. A listed SNI behind an
  IP literal or another name is decrypted and recorded, and the upstream TLS check uses the SNI name, so a
  local fake server can never pose as a listed host (it fails the check and becomes a tunnel, PRD-17
  C10). The old `SniMismatch` loss class goes. An unlisted SNI behind a listed CONNECT host is a tunnel.

### 3. The one host list (consumed, not owned)

PRD-11 owns `connectors/model-hosts.toml` and its contents (PRD-11 section 4 is the one list; this PRD
keeps no copy) and builds a `HostList` (seam S3) from three sources: shared, participant (`goalcup setup
--record-host`, PRD-10), and config (hints from config files and the connector's named base-URL variables
only). The proxy asks one question per connection:
`hosts.decide(name) -> Listed { source, kind, pinned } | Never | NotListed` (PRD-00 S3). `source` and
`kind` go into `conn` meta `listed` and `kind`.

Matching rules the proxy relies on (PRD-11 implements them in S3; `connector_hosts.rs` tests them):

- An entry is an exact name (`api.openai.com`), a subdomain pattern (`*.openai.azure.com`), or a pattern
  with one `*` inside one label (`bedrock-runtime.*.amazonaws.com`, `*-aiplatform.googleapis.com`).
- `never` entries (PRD-11 section 4, plus goalcup's own server added in code) win over every pattern
  and every source (point 4).
- `pinned = true` marks a host whose clients are known to pin: always a tunnel with GAP `pinned`. Empty
  at ship; the pad's dodger case sets it in its own list copy.
- The list is fixed for an entry. A host added mid-run takes effect at the next `goalcup continue`,
  because the run CA is minted for the list (section 4).

### 4. The run CA and TLS

Copy `proxy/tls.rs:100-215` and `crates/goalcup-cli/src/ca_bundle.rs` (207 lines) into
`src/proxy/ca.rs` and `src/proxy/tls.rs`, with these changes:

1. **The CA outlives a recorder boot.** Today the key dies with the process (`tls.rs:5-14`). A restarted
   recorder with a new CA would be refused by every harness, which trusts the CA file it was launched with.
   So the watchdog mints the CA once per entry with `ca::RunCa::mint(&HostList)` and calls
   `RunCa::write_files(entry_dir)`, which writes `run-ca.pem` (the CA certificate alone) and `bundle.pem`
   (OS roots plus the run CA, run CA first, `ca_bundle.rs` logic) as 0600 files in
   `$GOALCUP_HOME/runs/<run>/<entry>/`. `RunCa::export()` gives the certificate and key as PEM for the
   first control line (PRD-00 S8, fd 4); `RunCa::import(pem)` rebuilds it in each recorder boot. The key
   has exactly one file, PRD-02's `rec/run-ca.key` (0600, created new, removed at the seal), so `goalcup
   recover` can reload the same CA after the watchdog died (ledger R3b). It is never logged or put in
   argv. PRD-02 turns core dumps off before it reads that line (PRD-00 rules of the shape), so the key
   cannot reach a core file.
2. **Name constraints from the list.** Permitted subtrees come from `HostList::ca_subtrees()`: the
   wildcard-free suffix of each entry (`openai.azure.com` for `*.openai.azure.com`, `amazonaws.com` for
   `bedrock-runtime.*.amazonaws.com`), so a leaked key still cannot sign for an unrelated name. Keep
   `pathLen 0`, 8 days validity (`tls.rs:55`), one hour of clock skew (`:59`), and the Authority Key
   Identifier on leaves (`:200`).
3. **Leaves on demand.** One leaf per SNI, minted at first use and cached for the boot, as a rustls
   `CertifiedKey`. A pattern entry can match many names, so leaves cannot all be made at start.
4. **ALPN: prefer HTTP/1.1, keep HTTP/2 for clients that need it.** Upstream is offered a list built
   from the client's own offer, filtered to `h2` and `http/1.1`:
   - client offers `http/1.1` (with or without `h2`): offer upstream `http/1.1` only;
   - client offers only `h2`: offer upstream `h2` only;
   - client offers no ALPN (WebSocket clients often do): offer none, parse as HTTP/1.1 (PRD-17 T7);
   - client offers ALPN with neither: tunnel, GAP `protocol_other`, `reason: alpn_other`.
   Whatever upstream chooses (or none) is the only protocol offered back to the client. Both legs speak the
   same protocol and bytes pass unchanged. This keeps the new HTTP/2 parser off the core set's main path
   (all three ran on HTTP/1.1 in run 1 and the audit) while an `h2`-only client, such as Cursor's, is
   served and recorded (PRD-00 "ALPN on the release path"). Mirroring the client's full list is not
   built in 1.0; `PreferH1` is the only ALPN mode. It may be added after the build once HTTP/2 rows pass.
   If upstream rejects the offered ALPN (alert `no_application_protocol`, or it picks none for an
   `h2`-only client), open a fresh TCP connection and tunnel with `reason: "alpn_other"` and GAP
   `protocol_other`.
5. **Upstream roots are the Mozilla set plus the OS store** (the same OS read as the bundle). That covers
   a corporate inspection CA or a private gateway CA the machine already trusts (PRD-17 T9). If upstream
   validation still fails, or upstream asks for a client certificate, the proxy opens a fresh TCP
   connection and tunnels (GAP `upstream_untrusted`): the harness then validates the real certificate
   itself, so trust is never lowered (PRD-17 T3).
6. **The crypto provider is named** (ring), so rustls never panics on two providers (PRD-17 T8).

**Refusal learning** (second slice; in the core slice a refused handshake is a GAP `tls_refused` and a
close, nothing learned). Review A finding 1 showed that learning from any early close can tunnel the
harness itself for days. The rules:

- **Learn only from an explicit alert** on our leaf: `unknown_ca`, `bad_certificate`,
  `certificate_unknown`, `unsupported_certificate`, `access_denied` (rustls reports
  `AlertReceived(..)`). A plain close, a reset or a timeout during the handshake teaches nothing: it is
  what a harness does when its own timer fires during a rate-limit burst, a Wi-Fi change or a wake from
  sleep. That connection gets GAP `tls_refused` with tail `{"alert": false}` and closes; the next one
  is decrypted again.
- **A trusted set beats the refusal table.** A `(host, fingerprint)` pair that completed a handshake
  with our leaf in this boot is never learned as refusing. The harness connects first and often, so its
  own pair is trusted within seconds of start and after each restart.
- **A learned refusal expires after 10 minutes.** The next connection of that pair tries our leaf again.
  At most one failed call per refusing pair per 10 minutes.
- On learning: one `note` `ca_refused` (host and fingerprint prefix, never a secret) each time a pair
  is learned, so again after each expiry. Every later connection of the pair, until expiry, is a tunnel
  with GAP `tls_refused`.
- Both tables live in memory, per boot. Not persisted: after a restart the harness reconnects first (its
  connections were reset) and is trusted again at once, and the expiry bounds any other case.
- Pinned clients look the same on the wire, so learned refusals are always `tls_refused`; `pinned` is
  only for list entries marked so.

### 5. HTTP/1.1 (passive parser)

Copy the framing from `proxy/http.rs:706-1265`: head scan, fields, request and response heads, `Framing`
(length, chunked, close-delimited), `Chunked`, chunk-size lines, pipelining order, `HEAD`, `204` and
`304`, 1xx. Copy `websocket_accept_key` (`http.rs:73-82`). Rewrite the relay as passive:

- The request side opens one `http_req` stream per request (meta: method, path without query, `ver:
  "1.1"`, headers in wire order, `coding` as a list or null). The proxy drops the query and fragment
  when it builds the meta, as 0.7 does (copy `safe_target`, `metadata.rs:75-106`; PRD-01 `http_req`
  meta). The response side opens `http_resp` with `req` = the
  request's sid (meta: status, headers, `coding`, `decoded`). Headers go raw into meta in memory; PRD-02
  calls PRD-05's `mask_meta` before anything is written.
- 1xx heads become a `note` event (`informational_response`) with status and headers.
- **Head guard** (`http_head`, 64 KiB). Past it, the parser stops storing fields but keeps scanning the
  field lines for the ones framing needs (`content-length`, `transfer-encoding`, `content-encoding`,
  `connection`, `upgrade`), opens the stream with the fields stored so far, and writes on it GAP
  `memory_guard` with `off` 0, `lost` 0 and tail `{"guard":"http_head","limit":65536,"head":<bytes not
  stored>}`. Then it goes on with the body and later requests. It never stops parsing (ledger R5).
- Bodies are de-chunked and decoded (section 7), then sent as DATA pieces through the handle (the handle
  owns offsets, PRD-00 S1). END tails: `wire` (body bytes on the wire before decoding), `trailers`
  (chunked trailers, masked like headers), `unsent` on `http_req` when the upstream closed before the
  client's bytes could be forwarded.
- **Upgrade:** a request with `Upgrade: websocket` keeps forwarding; the request side only buffers its
  copy of any later client bytes until the response side sees the answer. A `101` with the right
  `Sec-WebSocket-Accept` switches both sides to the WebSocket parser, which replays the buffered copy
  first. Any other answer replays the copy into the HTTP parser. The copy has a 1 MiB memory guard
  (`upgrade_copy`); past it, parsing stops for the connection (5a).
- Framing that cannot be parsed: parsing stops for the connection (5a).

#### 5a. When a decrypted connection stops parsing

This is review A finding 2 and PRD-01 section 4 ("A decrypted connection that stops parsing"). It applies
to HTTP/1.1 framing faults, the upgrade copy guard, a parser panic, and the HTTP/2 cases of section 6.

1. The open stream gets its own GAP (`decode_failed`, `memory_guard` or `h2_unsupported`) and ends
   `decode_error`.
2. At the fault, one `note` `parse_stopped` `{conn, reason}` is written (PRD-01 note list). PRD-02 sets
   status `losing` while a `parse_stopped` connection is open and clears it at that connection's close
   GAP (Asks), so the participant sees the loss while it happens.
3. The connection keeps relaying unparsed. The proxy counts every plaintext byte relayed after the fault.
4. At close, the `conn` stream gets one GAP with the same reason, start = the fault time, end = the close,
   `off` 0, `lost` = plaintext bytes relayed after the fault + 1, so every request made later on that
   connection sits inside a named loss and counts in lost bytes. Then the `conn` END.

### 6. HTTP/2 (passive parser, new code, third slice)

Both legs carry the same frames, so the parser only observes. It is narrow on purpose (review B B1):

- Per direction: the 24-byte client preface, then frames (9-byte header). `HEADERS` plus `CONTINUATION`
  feed one HPACK decoder per direction; it follows the sender's table size updates. `DATA` payloads, with
  padding stripped, go to the stream's decoder. `END_STREAM` ends a body; a second `HEADERS` with
  `END_STREAM` is trailers (END tail `trailers`). `RST_STREAM` ends both streams `interrupted` with the
  code in the END tail (`h2_error`). `GOAWAY` ends streams above its last stream id. `PRIORITY`, `PING`,
  `SETTINGS`, `WINDOW_UPDATE` and unknown frame types are skipped by length (RFC 9113 section 4.1).
- Streams: one `http_req` per client stream, one `http_resp` per server response, meta `ver: "2"`, `h2`
  stream id, headers including pseudo-headers in wire order (PRD-01 section 4); `:path` has its query
  dropped like `path` (section 5).
- **Not followed in 1.0, and never silent:**
  - `PUSH_PROMISE` (clients normally disable push): parsing stops for the connection (5a,
    `h2_unsupported`).
  - Extended CONNECT (`:protocol`, RFC 8441): that stream's `http_req` gets GAP `h2_unsupported` with
    `lost` = its DATA bytes + 1 at its end, and no DATA bytes are recorded for it (they may be
    compressed WebSocket frames). The rest of the connection keeps parsing.
  - A message-level coding (`grpc-encoding`, `connect-content-encoding`) other than `identity`: GAP
    `unsupported_encoding` for that body (tail as in section 7) and no body bytes, so no compressed byte
    reaches disk (D14). Five lines; it stays because point 4 needs it.
  - An HPACK error, a table past the 1 MiB guard (`hpack_table`), or a frame over the 16 MiB maximum:
    parsing stops for the connection (5a). Never a refused connection.
- `h2_unsupported` is never a tunnel reason: the connection is already decrypted, so it keeps relaying
  unparsed (5a).
- HPACK: one decoder crate added in the scaffold (first choice `loona-hpack`, the maintained fork of
  `hpack`; check the version on crates.io first, unverified). If no crate is fit, write a decoder against
  the RFC 7541 appendix C vectors (about 500 lines with the Huffman table).

### 7. Body decoding

Copy `proxy/http.rs:1514-2022` into `src/proxy/decode.rs`: gzip (with `x-gzip`), deflate (zlib and raw),
brotli (`brotli-decompressor`), zstd. At most one content coding; more than one, or an unknown one, is a
GAP `unsupported_encoding` with tail `{"codings": [...]}` (each token at most 32 bytes, at most 4 tokens),
`lost` 0 and tail key `wire` (encoded bytes, PRD-01 section 3), and no body bytes (`http.rs:18-24`,
PRD-17 D5). A coding that
breaks part way keeps the decoded prefix, writes a GAP `decode_failed` and ends the stream `decode_error`.
Changes:

- No per-read decode budget (`MAX_DECODED_PER_READ`, `http.rs:97`): decoded output goes out in
  32 KiB pieces as it is produced, and PRD-02's `held_bytes` guard decides. Deflate expands at most about
  1032 to 1, so the work stays bounded by the input (PRD-17 D7).
- The zstd window limit stays at 8 MiB for HTTP `zstd` (RFC 9659 section 3, `http.rs:102`). Codex encodes
  its request bodies with zstd level 3 (`E-providers.md` E8.1).
- **Decoder memory is guarded** (review A finding 10) inside PRD-02's one memory guard, with no counter
  of its own. Before a coded body is decoded, call `Recorder::room()`; if the window (`1 << WBITS` for
  brotli, read from the first one or two bytes of the body, RFC 7932 section 9.1; the zstd frame window,
  at most 8 MiB; 32 KiB for gzip and deflate) fits, hold `Recorder::reserve(window)` until the body
  ends. If it does not fit, the body is not decoded: its stream gets GAP `memory_guard` with `lost` 0
  and tail `{"guard":"decoder_window","limit":<room at that moment>,"wire":<encoded bytes>}`, and the
  relay goes on. Run 1 had about 40 open bodies at once; at brotli's default 4 MiB window that is
  160 MiB, inside `room()` (the windows Anthropic sends are unverified, Risks).

### 8. WebSocket (passive parser)

Copy `proxy/websocket.rs` negotiation and framing: `negotiate` and `permessage_deflate` (`:195-265`), the
framer (`:324-420`), unmasking and the per-direction inflater with context takeover (`:553-1000`), the
`DEFLATE_TAIL` (`:80`). Change the output to PRD-01's shape:

- One `ws_up` and one `ws_down` stream per upgraded connection, opened when the `101` is seen, meta
  `deflate`. Every message goes out through `StreamHandle::ws(t, opcode, fin, bytes)`, one or more pieces,
  `fin` on its last. Text, binary, close, ping and pong are all recorded (today control frames are
  dropped, `websocket.rs:16-18`). A control frame inside a fragmented message keeps the message whole
  (PRD-17 W8).
- `fin` is also the masking boundary: PRD-02 passes it to `StreamMasker::boundary()`, so a fail-closed
  mask span never crosses a message (PRD-00 S1 and S2, review A finding 3).
- Payloads are unmasked and inflated. Under `no_context_takeover` the inflater resets per message.
  A broken frame or lost history is a GAP `decode_failed` for that message (unknown loss, `lost` 0); the
  stream goes on with the next message. Under context takeover a lost history cannot be reset
  mid-connection (`websocket.rs:43-46`), so every later compressed message of that direction is a GAP too.
  Framing is always tracked, so relaying never stops.
- END tail: `close_code` when a close frame was seen; `unsent` on `ws_up` as for `http_req`.

### 9. Tunnels

Every connection that is not decrypted is a `conn` stream with `mode: "tunnel"` and a `reason` (one of
PRD-01's list: `not_listed`, `sign_in`, `tls_refused`, `pinned`, `upstream_untrusted`, `alpn_other`,
`not_tls`, `hello_too_big`), opened when the decision is made and ended at close. Bytes are copied with
`copy_bidirectional` (`connect.rs:672-690`), no parse.

- END tail for every `conn`: `up` and `down`, bytes each way on the TCP wire (TLS records included).
  A decrypted `conn` also has `plain_up` and `plain_down` (PRD-01 section 3), which PRD-08's
  `conn_bytes` compares with its streams.
- **A tunnel to a listed host is a loss.** At close its `conn` stream gets one GAP with the matching
  reason (1 `memory_guard` with `{"guard":"client_hello","limit":65536}` for `hello_too_big`,
  6 `tls_refused`, 7 `pinned`, 13 `upstream_untrusted`, 14 `protocol_other` for `alpn_other` and
  `not_tls`), `off` 0, `lost` = `up` + `down` + 1 (PRD-01
  section 4). A `sign_in` tunnel is no loss (that host is never decrypted by design, point 4) and gets
  no GAP.
- **The unlisted model host flag, per host** (review A finding 6). For each unlisted host the proxy
  keeps, over a sliding 10 minutes, the seconds in which any tunnel to it received bytes and the bytes
  sent up. One `note` `long_tunnel` per host per boot when either:
  1. one tunnel has been open 60 s and received bytes in at least 10 different seconds (a stream), or
  2. tunnels to the host sent at least 1 MiB up within 10 minutes (model requests carry whole contexts;
     downloads mostly come down).
  Tail keys: `host`, `why` (`stream` or `upload`), `conns`, `up`, `down`, `secs`. PRD-10 turns it into one
  status line asking the participant to register the host if it is a model service. A `git push` can
  trip rule 2; the note says "if it is a model service", so one extra line is the accepted cost. A short,
  small, non-streaming call to an unlisted host is still only a tunnel record; PRD-10's live
  wire-against-native line is the second net for that.
- Every tunnel is in the record when it opens, so a crash loses none (today they are written only at
  finish, `tunnels.rs`), and there is no row cap (`tunnels.rs:20`).

### 10. Restart and hand-over

The recorder process may die (a bug, an out-of-memory kill) or be replaced on purpose (null mode to
normal, PRD-02 section 8). PRD-02's watchdog holds the listen socket and starts the next boot (seam S8).
What the proxy does:

- `proxy::serve(listener, ProxyCtx { recorder, hosts, ca, extra_roots }) -> ProxyHandle` accepts on the
  inherited socket; it binds nothing itself. Listen backlog 4,096, set by the watchdog.
- `ProxyHandle` offers one accessor, `quiet_since() -> Mono`: the `Recorder::now()` of the last byte any
  relay moved (review A finding 8). Open but silent connections do not count, so a connection held open
  and idle never delays the seal (PRD-17 P8). PRD-02 uses it for the grace after a harness-exit seal
  (PRD-02 seal step 1); GAP 15 `after_stop` takes its conns and hosts from the writer's table, not from
  the proxy. There is no graceful hand-over (PRD-02 section 9.1 weighed and dropped it).
- On a crash, in-flight connections of the dead boot are cut by the kernel. The harness sees a reset and
  retries, as it would after any network drop. New connections wait in the backlog.
- The new boot imports the same CA, so the harness keeps trusting it. Leaves, the trusted set and the
  refusal table are rebuilt on demand.
- The dead time is the `recorder_down` gap PRD-02 writes. The proxy writes nothing for it.

### 11. Memory

The proxy holds per connection two read buffers (16 KiB), the TLS states, one HPACK table per direction,
an inflater window per WebSocket direction (32 KiB), and a decoder per open body. Stated guards, each
writing a GAP `memory_guard` with `{"guard": <name>, "limit": <bytes>}` (PRD-01 section 3 names) and going
on: `client_hello` 64 KiB (then a tunnel, `hello_too_big`), `http_head` 64 KiB (the head's later fields
are not stored; parsing goes on, section 5), `upgrade_copy` 1 MiB (5a), `hpack_table` 1 MiB (5a),
`decoder_window` (a body whose window does not fit `Recorder::room()`, section 7). Each stops parsing at
most for the one connection that passed it. `upgrade_copy` and `hpack_table` keep the 5a stop because
they are rare (more than 1 MiB sent before a `101` answer; a table 256 times HTTP/2's 4 KiB default) and
following on would cost a second code path. Everything after the parser is PRD-02's `held_bytes` guard,
and decoder windows sit inside it through `reserve`. There is no connection, stream or byte cap.

### 12. What a pad run sees per harness case

| Case | `conn` meta | Records | Participant sees (PRD-10) |
|---|---|---|---|
| Direct key (Claude Code to `api.anthropic.com`) | `decrypt`, `listed: shared`, `kind: provider`, `alpn: http/1.1` | `http_req`, `http_resp` decoded (brotli) | model requests written |
| Gateway (Claude Code, Codex, Hermes or Kilo to `ai-gateway.vercel.sh`) | `decrypt`, `listed: shared`, `kind: gateway` | same as direct | same |
| Custom base URL (config names `llm.example.com`) | `decrypt`, `listed: config`, `kind: other` | same | same |
| Vendor backend (Amp to `ampcode.com`) | `decrypt`, `listed: shared`, `kind: vendor` | `ws_up`, `ws_down` of Amp's protocol; no provider call | "vendor backend: the model call runs on the vendor's servers" |
| Pinned or refusing client | `tunnel`, `reason: pinned` or `tls_refused` | GAP per connection, bytes each way | "not recorded: <host> refused the recorder" (keyed off the GAP) |
| Ignores `HTTPS_PROXY` | nothing reaches the proxy | session files only (PRD-04) | "no model requests written" alarm |

## Files it owns

Create (all under `crates/goalcup/`):

| Path | What | Estimate, src lines |
|---|---|---|
| `src/proxy/mod.rs` | `serve`, `ProxyCtx`, `ProxyHandle::quiet_since`, accept loop with `may_accept`, per-connection task, panic guard, test hooks | 240 |
| `src/proxy/connect.rs` | CONNECT head, upstream TCP with retry, first-bytes race, ClientHello (SNI, ALPN, fingerprint), decision, reset on upstream error | 650 (about 450 copied) |
| `src/proxy/ca.rs` | `RunCa` mint, export, import, `write_files`, subtrees from the list, leaf cache, bundle | 400 (about 330 copied) |
| `src/proxy/tls.rs` | upstream client config per ALPN offer, harness-facing server config per chosen ALPN, alert classification | 220 (about 120 copied) |
| `src/proxy/hosts.rs` | `decide()` over S3 plus the per-boot trusted set and refusal table with expiry | 120 |
| `src/proxy/pump.rs` | verbatim pump per direction that feeds a parser after each write; byte counters for 5a | 120 |
| `src/proxy/tunnel.rs` | byte tunnel, `conn` record, listed-host GAP, per-host flag | 160 |
| `src/proxy/http1.rs` | passive HTTP/1.1 parser, upgrade copy, 5a | 900 (about 600 copied) |
| `src/proxy/h2.rs` | narrow passive HTTP/2 parser | 450 |
| `src/proxy/ws.rs` | passive WebSocket parser | 600 (about 500 copied) |
| `src/proxy/decode.rs` | gzip, deflate, brotli, zstd, decoder window check through `room` and `reserve` | 540 (about 500 copied) |
| **Total** | | **about 4,250** (PRD-00 size row). HTTP/2 shrank by 200; the refusal rules, 5a, the decoder check and the per-host flag took most of it back; no `Alpn::Mirror` and a one-accessor `ProxyHandle` saved about 150 |

Tests (about 2,000 lines, PRD-14's table): `tests/proxy_connect.rs`, `tests/proxy_tls.rs`,
`tests/proxy_http1.rs`, `tests/proxy_h2.rs`, `tests/proxy_ws.rs`, `tests/proxy_tunnel.rs`,
`tests/proxy_restart.rs`, and fixtures in `tests/proxy_fixtures/` (an Anthropic brotli SSE body, a zstd
request body, an h2 session made with `curl --http2`, the RFC WebSocket examples, and a Codex-shaped
WebSocket deflate fixture: take the inflated, masked `ws_up` and `ws_down` messages of a phase 1a record,
re-deflate them with context takeover in the test, and commit only the masked plaintext source. It is
reviewed by eye before commit (AGENTS.md). No raw wire capture is ever written to disk; PRD-17 risk
"fixtures are partly synthetic"). They use
PRD-14's `tests/common/` fake upstream (a private helper until it lands, removed at phase 3) and
`Recorder::for_tests(room)`.

Test hooks (PRD-14 registry, under `#[cfg(feature = "test-hooks")]` in `src/proxy/mod.rs`):
`GOALCUP_TEST_UPSTREAM` (`host=127.0.0.1:port` pairs: where the proxy connects for a name) and
`GOALCUP_TEST_EXTRA_ROOTS` (a PEM file added to `ProxyCtx.extra_roots`), both read once in `serve`.

Must not touch: `src/pipeline/` and `src/watchdog/` (PRD-02), `src/mask/` (PRD-05), `src/connector/` and
`connectors/**` including `model-hosts.toml` (PRD-11), `src/cli/` and `src/main.rs` (PRD-10),
`crates/goalcup-format/` (PRD-01), the old crates (PRD-13 deletes them).

## Interfaces with other PRDs

| PRD | What this PRD uses or gives | Seam |
|---|---|---|
| PRD-00 | S1, S3, S8 final shapes; the `ProxyCtx` ruling; 2 relay threads; ALPN on the release path; slices (core: CONNECT, CA, HTTP/1.1, decode, WebSocket, tunnels; second: refusal learning, restart with the CA hand-over; third: HTTP/2) | |
| PRD-01 | Stream kinds `conn`, `http_req`, `http_resp`, `ws_up`, `ws_down`; meta keys (`kind`, `fingerprint`, `tls` null, pseudo-headers, `coding` list, `path` without query); tunnel reason `hello_too_big`; END tails `up`, `down`, `plain_up`, `plain_down`, `wire`, `trailers`, `h2_error`, `close_code`, `unsent`; GAP reasons 1, 3 (tail `codings`), 4, 6, 7, 11 (never a tunnel reason), 13, 14, the `lost` 0 plus `wire` rule, and the `conn` GAP rules (tunnels to listed hosts, a connection that stops parsing); guard names `client_hello`, `http_head`, `upgrade_copy`, `hpack_table`, `decoder_window`; notes `long_tunnel`, `ca_refused`, `informational_response`, `connect_refused`, `parse_stopped` | F1 through S1 |
| PRD-02 | Uses `Recorder::open(kind, conn, req, 0, flags, meta)`, `StreamHandle::{data, ws, gap, end}` (the handle owns offsets; `gap(reason, start, lost, tail)` reports a loss when it ends), `Recorder::event`, `Recorder::now`, `Recorder::room` and `Recorder::reserve` (decoder windows), `pipeline::fds::may_accept()`. Gives `proxy::serve(listener, ProxyCtx { recorder, hosts, ca, extra_roots }) -> ProxyHandle` with `quiet_since()`. Asks PRD-02 to show `losing` while a `parse_stopped` connection is open. The watchdog calls `RunCa::mint`, `write_files` and `export` and sends the PEM on the first control line; each boot calls `RunCa::import`. PRD-02 turns core dumps off and installs the fixed-text panic hook before reading that line | S1, S8 |
| PRD-05 | Meta holds raw headers and the query-free path in memory; PRD-02 masks them with `mask_meta` before writing. PRD-05 copies the header rules into `src/mask/meta.rs`; PRD-03 copies none. WebSocket `fin` reaches `StreamMasker::boundary()` through PRD-02. No proxy credential, so no credential rule | S2 (via PRD-02) |
| PRD-08 | Every named loss on a listed host's `conn` (GAP 1 for `hello_too_big`, 6, 7, 13, 14 for tunnels; 11 and the other 5a reasons for a connection that stopped parsing) counts as `named` in `listed_hosts_decrypted` and `wire_keeps_up` (PRD-00 ruling). With 5a, every unparsed byte on a decrypted connection sits inside a `conn` GAP, so `conn_bytes` may fail (not only warn) on bytes outside streams and outside such a GAP; PRD-08 and PRD-16 decide when | records |
| PRD-10 | Status lines for `long_tunnel` (both `why` values), `ca_refused`, GAP `pinned` and `tls_refused` (no `pinned_host` note exists), `parse_stopped` ("recording of one connection to <host> stopped; its traffic still passes"), vendor backend; the `--record-host` input | records |
| PRD-11 | Uses `HostList::decide(name) -> Listed { source, kind, pinned } \| Never \| NotListed` and `ca_subtrees()`; the list itself is PRD-11 section 4. PRD-11 sets `HTTPS_PROXY` and `https_proxy` to `http://127.0.0.1:<port>`, `NO_PROXY=127.0.0.1,localhost,::1`, `NODE_USE_ENV_PROXY=1`, extend-mode CA variables to `run-ca.pem` and replace-mode ones (`SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE`) to `bundle.pem` (tunnels go through the proxy and the client checks the real certificate). PRD-03 owns the code that writes both PEM files; PRD-02's watchdog calls it | S3 |
| PRD-12 | The dodger case passes a copy of `model-hosts.toml` with the harness's model host `pinned = true` through `goalcup start --hosts <file>` (S16). The gateway case uses Vercel AI Gateway for the core set. Watchers run the acceptance 3, 7 and 9 commands themselves in the pad box | S16 |
| PRD-14 | `tests/common/` fake upstream (TLS with test roots via `ProxyCtx.extra_roots`, HTTP/1.1, HTTP/2, WebSocket), `Recorder::for_tests`, the two `GOALCUP_TEST_` hooks above, `panic = "unwind"` | test support |
| PRD-17 | Lessons this PRD must pass: C1 to C11, T2 to T9, H4 to H7, H11, H12, D1, D5, D7, W1, W8, W9, W11, P2 (`tests/lessons_*.rs` and the `proxy_*` guards PRD-17 names) | `tests/lessons_*.rs` |
| PRD-13 | Deletes `crates/goalcup-cli/src/proxy/` after PRD-17's deletion gate passes | |

## What is cut

From `crates/goalcup-cli/src/proxy/` (lines with inline tests, `wc -l`):

| File | Lines | Fate |
|---|---:|---|
| `api.rs` (emitter, epochs, recovery, loss dedupe, stream limit) | 824 | deleted |
| `ingest.rs` (IPC sender, failure budget, queue lanes) | 1,177 | deleted |
| `mod.rs` (lifecycle, restart supervisor, 45 counters, capture table, 503) | 1,532 | deleted; about 240 new lines in `proxy/mod.rs` |
| `outage.rs` (outage windows) | 316 | deleted |
| `tunnels.rs` (side list, 10,000 rows) | 285 | deleted; tunnels are records |
| `preflight.rs` (TCP probe before launch) | 420 | deleted; the harness shows its own network errors, and a run with no model requests says so |
| `metadata.rs` (allowlist, hints, labels) | 407 | deleted except `safe_target` (query and fragment drop, about 30 lines, into `http1.rs`); meta is the masked head |
| `headers.rs` (header streams, masking) | 403 | deleted; PRD-05 copies the header rules |
| `connect.rs` | 1,565 | about 450 copied (head parse, authority, upstream connect, ClientHello, `PrefixedIo`); credential, failure classes, deadlines, `refuse` table deleted |
| `tls.rs` | 295 | about 200 copied (CA, leaf, roots), ALPN policy rewritten |
| `http.rs` | 2,257 | about 1,100 copied (framing, decoders); the active relay, selection, Host check, budget deleted |
| `websocket.rs` | 1,080 | about 500 copied (negotiation, framer, unmask, inflate); per-message streams, indexes, recovery deleted |
| **Total** | **10,561** | about 2,250 copied, about 8,300 deleted |

Also cut: `crates/goalcup-cli/src/commands/start/proxy.rs` (437; the open-file raise moves to PRD-02's
watchdog), and `ca_bundle.rs` (207) moves into `proxy/ca.rs`.

Tests that go (with the old crate, PRD-13): `m2_proxy_integration.rs` (3,025), `m2_http_relay.rs`
(2,249), `m2_websocket_relay.rs` (2,074), `m2_tls_connect.rs` (1,275), `m2_capture_sender.rs` (1,991),
`m2_emitter.rs` (1,517), and their support (4,027). Behaviours inverted on purpose:
`excess_connections_are_refused_predictably` (`m2_proxy_integration.rs:1846-1900`),
`http1_alpn_policy_is_explicit` (`m2_tls_connect.rs:726`), the restart-cap tests (`:2920-3025`) and the
failure-budget tests (`m2_capture_sender.rs:1123-1288`).

## Acceptance

Real-run proofs first (on the pad, PRD-12, Linux sandbox, real keys, staging Worker). A watcher runs
`goalcup-verify check` and `goalcup-verify dump` (PRD-08) on each entry. Where a proof needs a command
the agent may not run, the watcher runs it itself in the pad box, in the harness's environment
(`HTTPS_PROXY` and the CA variables), during the run (review B W1, W2).

1. **Direct key, HTTP/1.1, brotli** (core slice, phase 1a). Claude Code with one subagent. Pass: every
   `conn` to `api.anthropic.com` is `decrypt` with `alpn: "http/1.1"`; every `http_resp` with a `br`
   coding has `decoded: true` and its DATA parses as SSE when `content-type` is `text/event-stream`,
   else as JSON; PRD-12 check C1 passes (PRD-08 `threads_match` and `wire_keeps_up`); zero
   `unsupported_encoding`, zero `tls_refused` on a listed host.
2. **Direct key, WebSocket with deflate** (core slice). Codex with one subagent. Pass: one `ws_up` and
   one `ws_down` per WebSocket connection; every `ws_up` text message parses as JSON; PRD-12 check C1
   passes (PRD-08 `threads_match` and `wire_keeps_up`); zero `decode_failed`; Codex's HTTP fallback, if
   used, shows `alpn: "http/1.1"`.
3. **HTTP/2** (third slice). During a Codex pad run the watcher runs an `h2`-only client against a
   listed host: `curl --http2-prior-knowledge --parallel --parallel-max 4` with four URLs to
   `https://api.openai.com/v1/models` (check with `-v` that curl offers only `h2` over TLS; unverified;
   else `nghttp`). Pass: one `conn` with `alpn: "h2"`, four `http_req` and four `http_resp` with `h2`
   stream ids on it, decoded bodies, zero `h2_unsupported`.
4. **Gateway** (second slice; the strong case the owner asked for). The pad's `gateway` case for Claude
   Code, Codex and Hermes through Vercel AI Gateway (key provided), as Vercel documents each route
   (PRD-12 section 2), and Kilo through Vercel AI Gateway by the route PRD-11 and PRD-12 cite (recheck
   the docs on the day, T7). Pass for each: `ai-gateway.vercel.sh` is `decrypt`, `listed: shared`,
   `kind: gateway`, with no participant or config entry needed; PRD-12 check C1 passes (PRD-08
   `threads_match` and `wire_keeps_up`; Hermes by the per-session totals form, PRD-08 section 3.1,
   PRD-00 done item 1); the gateway key header is masked; zero GAPs on that host. OpenRouter: **not run: no key**
   (ledger T2); it ships "not verified" and runs as written when a key exists.
5. **Vendor backend.** Amp: **not run: no key** (ledger T2); ships "not verified". When an account
   exists, pass: `ampcode.com` is decrypted; the thread WebSocket is recorded in both directions; no
   provider connection appears (expected); the status says "vendor backend".
6. **Dodger case for the core set** (second slice; pinned through the pad's `--hosts` copy). Pass: the
   harness finishes its task; every connection to its model host is a tunnel with GAP `pinned`; zero
   decrypted requests to it; nothing claims "recording works" (PRD-00 done item 1).
7. **Refusal fallback** (second slice). During a Claude Code pad run the watcher runs
   `curl --cacert /etc/ssl/certs/ca-certificates.crt https://api.openai.com/v1/models` twice, then once
   more after 11 minutes. Pass: the first fails with one GAP `tls_refused` and one `ca_refused` note; the
   second gets a `401` through a tunnel; the third fails once more (expiry) and is learned again, with a
   second `ca_refused` note; the
   harness's own calls to listed hosts stay `decrypt` throughout.
8. **Restart** (second slice). The kill test of PRD-02 R1 (Codex, `kill -9` twice). Pass: after each
   restart the harness's new connections are `decrypt` (the CA survived), not `tls_refused`; the harness
   finishes.
9. **Long tunnel** (core slice). The watcher runs `curl -N https://stream.wikimedia.org/v2/stream/recentchange`
   for 90 s in the pad box. Pass: one `long_tunnel` note with `why: "stream"` for that host, no other
   effect, no GAP.
10. **Load** (phase 6, PRD-16 A1 and A2, as PRD-16 runs them under the 10 USD cap). Pass on each: zero
    proxy GAPs of any kind on listed hosts, `memory_guard` included (PRD-16 X3); no `503` or refused
    connection; PRD-16 X1 holds per 10-minute bucket; recorder RSS reported. Connections cut by A2's
    planned kill are PRD-02's `recorder_down`, not a proxy GAP. The ALPN default stays `PreferH1` in 1.0;
    no mirrored load run is scheduled.

Automated tests that guard them (fake upstream, `Recorder::for_tests`):

- `proxy_connect.rs`: SNI decides, no SNI uses the CONNECT host, a listed SNI behind an IP literal is
  decrypted, a server-first protocol tunnels with no wait, 2,000 concurrent connections are all served
  (no cap), `may_accept()` false makes accepts wait and never refuse, a non-CONNECT request gets `405`
  and one `connect_refused` note, an upstream reset reaches the client as a reset, a ClientHello past
  64 KiB to a listed CONNECT host tunnels with reason `hello_too_big` and its GAP 1 `memory_guard`.
- `proxy_tls.rs`: the ALPN matrix (client h2+h1, h1, h2 only, none, other) against upstream h2, h1 and
  none, under `PreferH1` (the only mode), including an upstream that rejects the offered ALPN (a fresh
  TCP connection, a tunnel with `alpn_other` and GAP `protocol_other`); upstream certificate failure
  falls back to a tunnel with GAP `upstream_untrusted`; an `unknown_ca` alert teaches the table, writes
  one `ca_refused` note, and a second fingerprint is still decrypted; a plain close teaches nothing and
  its GAP has `{"alert": false}`; a pair in the trusted set is never learned; a
  learned refusal expires after 10 minutes on the test clock; a CA exported and imported signs leaves the
  first boot's `run-ca.pem` trusts.
- `proxy_http1.rs`: each coding decodes to the plaintext; an unknown coding writes no body byte; chunked
  trailers reach the END tail; pipelining pairs responses; `HEAD` and `304` have no body; the SSE prefix
  reaches the client before EOF; a parser panic leaves the relay running; **after a framing fault, a
  second request on the same connection sits inside a `conn` GAP whose `lost` equals the bytes relayed
  after the fault plus one, and a `parse_stopped` note exists**; with `Recorder::for_tests(room)` small,
  the next `br` body gets GAP `memory_guard` `decoder_window` with `lost` 0 and a `wire` tail, and the
  client still gets every byte; a head past 64 KiB gets GAP `memory_guard` `http_head`, its body and the
  next request on the connection are still recorded; a request target's query never reaches the meta.
- `proxy_h2.rs`: CONTINUATION, HPACK dynamic table and size updates, padding, trailers, `RST_STREAM`,
  `GOAWAY`; `grpc-encoding: gzip` gives a GAP and no body bytes; an extended CONNECT stream gets its GAP
  and no DATA while other streams keep parsing; `PUSH_PROMISE` and an HPACK error give `h2_unsupported`,
  the 5a `conn` GAP, and the relay goes on byte for byte.
- `proxy_ws.rs`: the Codex fixture inflates to the recorded JSON; takeover and no-takeover; a fragmented
  message is one message with `fin` on its last piece; control frames recorded; client masking removed.
- `proxy_tunnel.rs`: `up` and `down` counts are exact; a listed-host tunnel gets its GAP with `lost` =
  `up` + `down` + 1; a `sign_in` tunnel gets none; the per-host flag fires once for a stream and once
  for 1 MiB sent up across short tunnels, and not for a 50 MiB download.
- `proxy_restart.rs`: `serve` on an inherited listener after the first boot is dropped; a queued
  connection is served by the second boot with the same CA; `quiet_since` moves with relayed bytes and a
  silent open connection does not delay it.

## Risks and blind spots

| Risk | What could go wrong | What we do |
|---|---|---|
| Preferring HTTP/1.1 changes what an h2-capable client uses | A client that would use h2 without us uses HTTP/1.1 through us, so one more connection per parallel call | All three core harnesses ran on HTTP/1.1 through 0.7 and in run 1; point 7 is met as long as the harness works. The ALPN default stays `PreferH1` in 1.0; mirroring is not built and no mirrored load run is scheduled |
| A cut handshake is not learned | A client that refuses our CA by closing without an alert fails on every call, never falling back | Every common TLS stack (OpenSSL, BoringSSL, Go, Java, rustls) sends an alert on an untrusted chain. The GAP `tls_refused` names each failure; acceptance 7 checks curl |
| A tool shares the harness's TLS fingerprint and refuses | With the trusted set, that tool's calls keep failing once per call, since its pair is never learned | Accepted: point 1 (the harness stays recorded) over one tool's calls. The GAP and note name it; the participant can run that tool without the CA variables |
| Refusal expiry | A refusing tool fails one call every 10 minutes | Accepted; most clients retry. Overridable default |
| A connection that stops parsing loses the rest of its requests | Long-lived pool connections may carry many later calls | Named and counted (5a); status says "losing" from the `parse_stopped` note; acceptance 1, 2 and 10 require zero such GAPs on listed hosts |
| Per-host flag noise | A big `git push` or an upload to an unlisted host gives one note | One line per host per boot, worded "if it is a model service" (PRD-10) |
| Wider decryption records the agent's own calls to listed hosts | A test suite calling `api.openai.com` with the bundle shows up as model requests; Amp's `/api/internal` (e-mail, secret fingerprints, `connectors/amp-cli/harness.toml:105-109`) is recorded | Masking still applies (point 4). PRD-09 tells calls apart by headers and thread hints. PRD-12's wire-against-log check allows extra wire calls and reports them |
| Name constraints use wide suffixes for cloud patterns | `amazonaws.com` as a permitted subtree weakens the leaked-key guard for AWS names | The key lives only in memory, the control socket and one 0600 file in a 0700 folder until the seal (`rec/run-ca.key`, ledger R3b); only processes we launch trust the CA |
| CA handed over the control line | A bug in the hand-off makes every restart refuse all hosts | Acceptance 8 and `proxy_restart.rs` prove it |
| Decoder guard drops a body | When a body's window does not fit `Recorder::room()`, that coded body is a GAP, not a record | Stated guard with a name; decoder windows sit inside PRD-02's 512 MiB guard through `reserve`; acceptance 10 requires zero such GAPs |
| Upstream connect retry | A listed host outage holds the harness's CONNECT up to 25 s instead of failing fast | Accepted: one slow answer beats a failed call (PRD-17 C6). Unlisted targets are not retried. Overridable default |
| Upstream reset passed on as a reset | A harness that treated our clean close as "done" now sees an error | That is what it sees without us (point 7); it retries |
| ECH, HTTP/3, UDP | A client with Encrypted Client Hello shows its public outer name; QUIC and WebRTC never reach a CONNECT proxy | Tunnel by outer name; not supported, stated in PRD-15's guide |
| No credential | Another local user can route traffic through the proxy into this run | Loopback only; it cannot read anything out. Single-user sandboxes and laptops are the target |
| `405` for non-CONNECT | A client that sends `http://` requests to `HTTPS_PROXY` is refused | goalcup never sets `HTTP_PROXY`, so no harness should; the pad counts `405`s. A local model server on plain HTTP is reached past the proxy and named by PRD-11 and PRD-10 (`local_model`) |
| HPACK crate quality | An unmaintained or panicking decoder | Panic guard; RFC 7541 vectors in `proxy_h2.rs`; own decoder as the fallback |
| Unverified facts | Bun `fetch` and HTTP/2 through a proxy; whether `curl --http2-prior-knowledge` offers only `h2` over TLS; Kilo's Vercel AI Gateway route (PRD-11 and PRD-12 cite one; recheck on the day, T7); brotli window sizes Anthropic sends; a client that sends only a post-quantum key share (rustls with ring does not do hybrid key exchange) | Each is a pad row or a watcher check, not an assumption |

## Overridable defaults

| Default | Value | Who may change it |
|---|---|---|
| Upstream connect retry for listed targets | until 25 s, pause 250 ms doubling to 2 s (`connect.rs:86-89`) | PRD-03 |
| ALPN | `PreferH1` (h2 only for an h2-only client); the only mode in 1.0, no mirrored load run is scheduled | Owner, after the build |
| Relay threads | 2 | PRD-03, if acceptance 10 shows relay lag |
| Long tunnel, per host | one tunnel open 60 s with bytes in 10 different seconds, or 1 MiB sent up in 10 minutes | PRD-10 with PRD-03 |
| Refusal learning | explicit alert only; trusted set wins; expiry 10 minutes; per boot | PRD-03 |
| Memory guards | `client_hello` 64 KiB, `http_head` 64 KiB, `upgrade_copy` 1 MiB, `hpack_table` 1 MiB, `decoder_window` = `Recorder::room()` at that moment (no number of its own) | PRD-03 |
| Non-CONNECT requests | `405` | Owner (the ledger says never refuse; this refuses a request type no harness sends) |
| Proxy credential | none | Owner |

## Asks after revision 2

PRD-00 and PRD-01 revision 2 took this PRD's twelve first-round asks. What is left:

1. **PRD-01:** add `parse_stopped` to the note list (tail `{conn, reason}`), and `why`, `conns`, `up`,
   `down`, `secs` as `long_tunnel` facts. Ruled by PRD-00 R3-4; PRD-01 copies them (P01-6). Done.
2. **PRD-00:** the risk row and review-A answer for refusal learning say "a trusted set kept in the
   entry folder" and "earlier in the entry". This PRD keeps both tables per boot, with a 10-minute expiry
   (section 4, reason there). Ruled by PRD-00 R3-5. Done.
3. **PRD-02:** take the after-seal grace's quiet time from `ProxyHandle::quiet_since()`; read the CA
   line only after core dumps are off; set status `losing` while a `parse_stopped` connection is open,
   and clear it at that connection's close GAP.
4. **PRD-08:** `conn_bytes` may become `fail` for unaccounted bytes outside any `conn` GAP (5a).
5. **PRD-10:** words for `parse_stopped` and for `long_tunnel` with `why: "upload"`; key the pinned line
   off GAP `pinned`, since `pinned_host` was dropped (PRD-01).

## Builder notes

Order of work, by PRD-00's slices:

1. **Core slice.** `connect.rs`, `ca.rs` (mint, export, import, files), `tls.rs` with `PreferH1`,
   `hosts.rs` without learning, `tunnel.rs`, `mod.rs` with a stub parser that records nothing: every
   connection relays. Prove "never refuse" first (2,000 connections, `may_accept`, server-first). Then
   `pump.rs`, `decode.rs` (with the decoder guard) and `http1.rs` with 5a against S1 stubs: the first
   piece for the early real test (phase 1a, acceptance 1, Claude Code direct key). Then `ws.rs`: the
   second early test (acceptance 2, Codex). The per-host flag lands with `tunnel.rs`.
2. **Second slice.** Refusal learning with the trusted set and expiry; `ProxyHandle::quiet_since` and
   the restart test with an imported CA; the gateway and dodger rows.
3. **Third slice.** `h2.rs`, then acceptance 3. ALPN stays `PreferH1`.

Build and run every test here in a build box (Vercel Sandbox, ledger B1); the cloud session only writes
and reviews code.

Traps from the audit and the old code:

- Write to the peer before parsing, every time. Run 1's loss came from the hand-off, not the relay; keep
  the relay free of anything that can wait.
- Check upstream TLS before answering the client's handshake (`connect.rs:610-633`); the fallback after
  a failed upstream check is a fresh TCP connection, never the half-used one.
- `ServerConfig` carries the ALPN list, so build one per chosen ALPN (h2, http/1.1, none) and share it;
  leaves come from a `ResolvesServerCert` that reads the leaf cache. Do not build a config per connection.
- Classify the client's handshake error: `AlertReceived(UnknownCA | BadCertificate | CertificateUnknown |
  UnsupportedCertificate | AccessDenied)` learns; `UnexpectedEof`, a reset or a timeout does not.
- A ClientHello can span TLS records (`connect.rs:1090-1120` handles it); keep that.
- Leaves need the Authority Key Identifier (`tls.rs:17-19`, Python 3.13).
- WebSocket: under context takeover a lost history can never be reset mid-connection
  (`websocket.rs:43-46`); record later compressed messages as gaps.
- Do not reuse the old `[masked]` marker or mask in the proxy. Raw meta goes to PRD-02; masking is PRD-05's.
- Never log a header value, a body byte, the CA key or a fingerprint with a host in the same line as a
  secret. Errors carry fixed text through `cli::log::line` (S11; today's rule, `tls.rs:62-63`).
- `catch_unwind` needs `UnwindSafe`; wrap the parser state in `AssertUnwindSafe` and drop that
  connection's parser after a panic (5a).
- Set `SO_LINGER` 0 on the harness leg only when the upstream leg failed; a clean close stays clean.
- Every implementer checks the vendor's current docs before a harness-specific assumption (T7): Kilo
  through Vercel AI Gateway by the route PRD-11 and PRD-12 cite, curl's ALPN with
  `--http2-prior-knowledge`, the HPACK crate.

How a watcher proves it in a real run: after each pad run, `goalcup-verify dump <entry> | jq` for
`k=="open" and skind=="conn"`; every listed host must show `mode: "decrypt"` except the cases in
acceptance 6 and 7; count `gap` records by reason (any proxy reason on a listed host outside those cases
is a failure); grep for `parse_stopped` (zero expected); where the harness keeps a log, compare wire POSTs
plus `ws_up` text messages with it (PRD-12's optional local log check; the record is the proof, ledger T4); read each `long_tunnel` host by eye.

## Review answers

History of revision 2. Where an answer here disagrees with the body or the "Alignment answers" section
(`ProxyHandle::{stop_accepting, idle, open_conns}`, the 256 MiB decoder counter, the `Mirror` switch,
the masked Codex capture), the body and "Alignment answers" win.

Sources: CC = `prd-consistency.md` item; A = review A finding; B = review B item; U = the owner's
relayed message of 30 Oct. Verdicts: accepted, partly accepted, rejected, not this PRD.

| Item | Ask | Verdict and what changed |
|---|---|---|
| U | Gateway is still an HTTPS proxy hop; one system; strong pads for HTTPS proxy and gateway; dodgers use the current way | Accepted. Gateway answer rewritten around the owner's words; acceptance 4 now runs the three core harnesses through Vercel AI Gateway (key provided) plus Kilo; dodgers keep the light path (acceptance 6) |
| CC 3, B C1 | S1 shape | Accepted. PRD-02's shape (handle owns offsets, `gap(reason, start, lost, tail)` called when the loss ends). Interfaces row and 5a use it; the old ask 2 is withdrawn |
| CC 6, B C5 | S8 carries the CA | Accepted. CA as PEM on the first control line (fd 4), not a separate pipe; `export`/`import` in section 4 |
| CC 9 | `proxy_fixtures/` owner | Accepted. Owned here; a masked real Codex capture added |
| CC 12 | `src/watchdog/` folder | Accepted. Must-not-touch line updated |
| CC 24, B verdict 2 | Size | Accepted. Stays about 4,400: HTTP/2 narrowed by 200, the review fixes add about 200 |
| CC 32, 33, 35, B C2, C3 | Tunnel reasons, `conn` GAP, END tails, meta | Accepted as PRD-01 ruled: GAP 13 `upstream_untrusted`, 14 `protocol_other` for `alpn_other` and `not_tls`; `up`/`down` on the TCP wire plus `plain_up`/`plain_down`; `unsent` added |
| CC 36 | `pinned_host` note | Accepted PRD-01's ruling: no such note; PRD-10 keys off GAP `pinned` (Asks 5) |
| CC 41, B C4 | Proxy credential | Accepted. Cut everywhere (rule 5) |
| CC 42, CC 108 | Open-file raise owned twice | Accepted. PRD-02 raises; rule 7 now uses `fds::may_accept()` with `EMFILE` retry as fallback |
| CC 43 | `serve` signature | Accepted. `serve(listener, ProxyCtx { recorder, hosts, ca, extra_roots }) -> ProxyHandle` |
| CC 44 | Relay thread count | Accepted. 2 by default (rule 6), overridable |
| CC 50 | Kill-test gap length | Not this PRD (PRD-02 owns the number); acceptance 8 checks only that the CA survives |
| CC 52 | Listed host tunnelled for reasons PRD-08 does not accept | Accepted. Interfaces row PRD-08 lists GAP 6, 7, 11, 13, 14 as `named` (PRD-00 ruling) |
| CC 53, B W3 | Acceptance with no key | Accepted. OpenRouter and Amp marked "not run: no key"; acceptance 4 keeps its strong part on Vercel AI Gateway |
| CC 54 | `decide()` return | Accepted. `Listed { source, kind, pinned } \| Never \| NotListed` (PRD-00 S3 naming) |
| CC 55 | Host list duplication | Accepted. Section 3 keeps only the matching rules and points to PRD-11 section 4 |
| CC 56 | Folder for the PEM files | Accepted. `$GOALCUP_HOME/runs/<run>/<entry>/`; PRD-03 code, PRD-02's watchdog calls it |
| CC 57, B C12 | Dodger seam | Accepted. `goalcup start --hosts <file>` (S16) |
| CC 58 | Guard names | Accepted. Five names in section 11, matching PRD-01 |
| A 1 | Refusal learning can tunnel the harness | Partly accepted. Explicit alert only, trusted set, 10-minute expiry, tests. Not persisted to the entry folder (per boot is enough after a restart; Asks 2). The pad case with a delay hook is rejected in favour of `proxy_tls.rs` (no hook in product code for it) |
| A 2 | Parser loss on a reused connection | Accepted (a) as 5a plus a `parse_stopped` note; (b) passed to PRD-08; (c) ALPN `PreferH1`. Fault-injection tests added |
| A 3 | Fail-closed mask across WebSocket messages | Accepted the proxy part: `fin` reaches `StreamMasker::boundary()` via PRD-02. Masking is PRD-05's |
| A 4 | Recorder exit resets every connection | Partly accepted. `ProxyHandle::{stop_accepting, idle}` for a graceful hand-over; hard guard, `oom_score_adj` and the pad kill cases are PRD-02 and PRD-12 |
| A 5 | Core dump holds raw bytes and the CA key | Accepted as an interface: PRD-02 turns dumps off before the CA line is read |
| A 6 | Unlisted model host is a silent tunnel | Partly accepted. Per-host flag with a stream rule and a 1 MiB upload rule. The reviewer's "2 MiB down or 60 s open" rule rejected: package downloads and git would trip it every run. The live wire-against-native line is PRD-10's |
| A 7 | Local model server | Not this PRD (PRD-11 and PRD-10 `local_model`); noted in the `405` risk row |
| A 8 | Processes that outlive the harness | Partly: `ProxyHandle::open_conns` and `idle` give PRD-02 the data for GAP 15 `after_stop`; subreaper and port reuse are PRD-02's |
| A 10 | Decoder memory outside the guard | Accepted. `decoder_window` guard, 256 MiB process-wide, counted from declared windows (section 7) |
| A 13 | Half-open upstream | Partly accepted. Upstream failure reaches the harness as a reset (rule 8). `TCP_USER_TIMEOUT` 120 s rejected: a timer of our own that the harness would not have without us |
| A 14 | Which clock | Accepted. Rule 9: proxy timers use `Recorder::now()` |
| A coverage | Fault injection on reused connections; network change | Accepted the first (tests). Network change: the `proxy_tls.rs` "plain close teaches nothing" test covers the proxy side; the route-drop pad case is PRD-12's |
| B A6 | ALPN on the release path | Accepted. `PreferH1` default, `Mirror` switch after its pad rows |
| B B1 | Narrow HTTP/2 | Accepted, with one exception: the `grpc-encoding` check stays (five lines) because without it compressed bytes reach disk unmasked (point 4). Push and RFC 8441 are named gaps, not features |
| B N10 | Refusal learning in the second slice | Accepted. Core slice: GAP and close, nothing learned |
| B W1, W2 | Watcher runs the commands | Accepted. Acceptance 3, 7 and 9 are run by the watcher in the pad box |
| B M3 | `tests/common/` not ready | Accepted. A private helper until it lands, removed at phase 3 |
| B T6 | RR6 real run for host hints | Not this PRD (PRD-17); hint parsing is PRD-11's |

## Alignment answers

Revision 3 against OWNER-DECISIONS.md (the ledger wins). Fix ids are from the alignment fix lists.

| Fix | What was done |
|---|---|
| L4-5 | Applied. `path` and HTTP/2 `:path` drop the query and fragment, as 0.7 does (sections 5 and 6); `safe_target` is copied; a test checks no query reaches the meta; the PRD-05 row and the cut table no longer copy query rules (ledger M1, point 4) |
| L2-14 | Rejected. It conflicts with P03-14, which group 2 decided after it, and with PRD-17 C6, whose lesson test `listed_refusal_is_retried` keeps the retry. The retry never delays a working call; its cost is now a Risks row. Recording it in PRD-15's decisions is PRD-15's part |
| L2-15 | Applied. `http_head` no longer stops parsing: the parser keeps scanning framing fields, stores no more, writes GAP `memory_guard` (`lost` 0, tail `head`) and goes on (section 5, section 11, a test). `upgrade_copy` and `hpack_table` keep the 5a stop, stated as rare in section 11 (ledger R5) |
| L2-16 | Partly. Applied with P03-4's names, as PRD-01 already did: tunnel reason `hello_too_big`, close GAP 1 `memory_guard` on a listed host with `lost` = `up` + `down` + 1. `memory_guard` is not used as a tunnel reason, since PRD-01 keeps GAP reasons and `conn` reasons apart |
| G2-11 | Applied. Acceptance 10 runs on PRD-16 A1 and A2 under the 10 USD cap with X1 and X3; the ALPN default stays `PreferH1`, with no mirrored run (acceptance 10, Risks, Overridable defaults). PRD-08's part is PRD-08's |
| G2-23 | Applied. The process-wide 256 MiB counter is gone: a decoder checks `Recorder::room()` and holds `Recorder::reserve(window)` (section 7, section 11, Overridable defaults, PRD-02 row, test with `for_tests(room)` small) |
| G2-24 | Applied. `ProxyHandle` has only `quiet_since() -> Mono`; `stop_accepting`, `idle` and `open_conns` are gone from section 10, the PRD-02 row, the builder notes, Asks 3 and `proxy_restart.rs` |
| G2-25 | Applied. `Alpn::Mirror` is not built; `PreferH1` is the only mode; the `proxy_tls.rs` matrix runs under it; size about 4,250 (ledger L2) |
| G2-26 | Applied. A body not decoded under `decoder_window`, and an `unsupported_encoding` body, get `lost` 0 and a `wire` tail (section 7) |
| G2-27 | Applied. Asks 1 and 2 are marked ruled by PRD-00 R3-4 and R3-5 |
| P03-4 | Applied (section 1 diagram, section 2, section 9, `proxy_connect.rs`, PRD-01 and PRD-08 rows). PRD-01 and PRD-08's parts are theirs |
| P03-5 | Applied. `h2_unsupported` is out of the tunnel reasons and the listed-tunnel GAP list; section 6 says it is never a tunnel reason (ledger H6) |
| P03-6 | Applied. The Codex fixture is re-deflated in the test from masked plaintext of a phase 1a record; no raw wire capture is written; reviewed by eye (point 4, ledger M1, AGENTS.md) |
| P03-7 | Applied. Acceptance 1, 2 and 4 use PRD-12 check C1 (PRD-08 `threads_match` and `wire_keeps_up`; Hermes by the per-session totals form). The Hermes reference names PRD-08 section 3.1 and PRD-00 done item 1, since PRD-12 cites a different owner question number than the fix list |
| P03-8 | Applied in acceptance 1 |
| P03-9 | Applied. GAP 3 tail `codings` with its bounds; `unsupported_coding` removed from the PRD-01 row |
| P03-11 | Applied. Upstream ALPN rejection gives a fresh TCP connection, a tunnel with `alpn_other` and GAP `protocol_other` (section 4, `proxy_tls.rs`) |
| P03-14 | Applied. The listed-target retry stays; a Risks row states the cost |
| P03-15 | Applied. Every non-`200` answer writes one `connect_refused` note, at most one per host and status per minute (rule 3, test) |
| P03-16 | Applied in section 3 |
| P03-17 | Applied in acceptance 4, Risks "Unverified facts" and the builder notes |
| P03-19 | Applied in 5a item 2, the PRD-02 row and Asks 3 |
| P03-20 | Applied in the builder notes (ledger B1) |
| P03-21 | Applied in section 2 (checked: `connect.rs:75` is 32 KiB today) |
| P03-22 | Applied. One `ca_refused` note each time a pair is learned; acceptance 7's third call adds a second note; a no-alert close has GAP tail `{"alert": false}` (section 1 diagram, section 4, `proxy_tls.rs`). PRD-08's part is PRD-08's |
| P05-21 | Applied in the PRD-05 row |
| X2-11 | Partly. Moving the "Review answers" and ruled "Asks" into `REVIEW-HISTORY.md` needs a write to a second file, which this pass may not make; it is left to the one pass that owns that file. Here the binding rulings are already in the body, Asks 1 and 2 are marked done, and the "Review answers" section opens with a line saying the body and these answers win |

## Amendments

### 2026-10-30, replan after PRD-17 phase 0 slice (merge 1423f8f9)

- **Rule 2's guard is in the lessons lane, not here** (lessons log 10:51Z): PRD-17 H11, W11 and G1
  name "PRD-03 rule 2 tests", and this PRD's test list has none, so PRD-17 writes
  `lessons_http1::every_request_on_a_decrypted_host_is_recorded` (a gateway-prefixed path, a Host
  mismatch and a non-POST method) and `lessons_ws::an_upgrade_on_any_path_is_recorded`. Do not add a
  duplicate rule 2 case. Both drive the real `proxy::serve` against PRD-14's `common::upstream`, so
  a filter by path, method or Host on a decrypted host fails a lessons test, which blocks PRD-13's cut
  (PRD-17 gate check 2). `proxy::serve` and `common::upstream` keep their PRD-00 "Seams" shapes so
  those tests can be written against them.

### 2026-10-30, replan after PRD-10 slice 1 (merge d55030ee)

- **S11 is on integration** (`goalcup::cli::log::{line, Fact, Host}`). A host enters the log only as
  `Fact::Host(Host::checked(s)?)` where `s` is the connection authority the proxy already parsed or a
  host from the host list; a request path, query, header value or other traffic text is never a
  `Fact::Host` or `Fact::Path`. When `Host::checked` returns `None`, log a fixed `Fact::Word` in its
  place, never the text. Accept and socket errors (EMFILE included) go in as `Fact::Io(kind)`, counts as
  `Fact::N`. After S3 lands, PRD-10 adds `From<connector::hosts::Host>` for hosts from the list.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-11 slice 2 (merge b9fa50f8)

- **S3 is on integration.** `HostList::decide(&self, &str) -> Decision` (`Listed { source, kind,
  pinned }`, `Never`, `NotListed`) and `HostList::ca_subtrees() -> Vec<String>` have their final
  signatures, so slice 2 (serve, hosts, tunnel, pump, http1) builds against them now. Both are stubs
  until PRD-11's hosts slice: every name is `NotListed` and there are no subtrees. A proxy test that
  needs a `Listed` or `Never` answer waits for that merge; nothing in `src/proxy/` copies the
  matching meanwhile.

### 2026-10-30, replan after core slice 1 (merge 0e411074)

- **Section 7 as built.** Brotli is counted per allocation, not as `1 << WBITS` read from the header:
  brotli-decompressor 5.0.3 allocates the whole declared window at the first meta-block, so a flushed
  (SSE) body counts its full window from the first push (about 4.24 MB at WBITS 22, 16.8 MB at 24). A
  body is refused only before its first piece went out; a body already decoding is never refused. A
  refused body's allocation is made and dropped at once, a transient of up to 16 MiB per worker
  thread outside `held` (`BrotliHeap` doc). Zstd counts its frame window plus 512 KiB
  (`ZSTD_STREAM_OVERHEAD`). Section 7's "40 bodies is 160 MiB" holds at WBITS 22; at WBITS 24
  `room()` at the default soft guard (384 MiB, less what is held) fits about 23 open bodies, under
  run 1's about 40. The window Anthropic sends stays the Risks row's pad check. Wire-proxy's 14:47Z
  owner question (acceptance 10 holds for about 40 concurrent coded bodies, not 200) touches no
  locked item: R5 holds, the lever is PRD-02's soft default, measured by PRD-16 X3 and PRD-02 R8.
  The proxy adds no cap and no uncounted decoder for it.
- **Slice 2 carries three nits from the last review** (owned files): delete the last block of
  `a_flushed_brotli_body_counts_its_whole_window` in `tests/proxy_decode.rs` (room `1 << 20`), which
  repeats the first block of `a_window_over_room_is_not_decoded`, and drop "and refuses a body whose
  window does not fit" from its doc (B14, R2); rewrap the `Zstd` struct doc in `decode.rs` (lines
  674 to 678, one line at 127 columns) to the file's 100 (B15). The `BrotliHeap` doc's claim that
  PRD-02's margin holds the refusal transient is true only when the soft guard is at least
  64 MiB times the worker threads (the margin is a quarter of soft); say that bound instead.
- **R1 is declined for 1.0:** single-segment zstd frames are over-counted by up to about 384 KiB per
  body. It only makes the guard more cautious, touches no record byte, and would add lines to the
  file most over its estimate. Reopen only if A1 to A3 show a `decoder_window` GAP on a zstd body.
  A17 (refuse brotli allocations) stays skipped: the crate's Huffman table sites use an empty
  slice unchecked and could panic.
- **Size (decision line 15:16Z in the wire-proxy log).** Slice 1 is 2,873 src lines against its
  files' 2,410 estimate (+19%): `decode.rs` 810 against 540, `ws.rs` 735 against 600, `ca.rs` 439
  against 400, `connect.rs` 673 against 650, `tls.rs` 203 against 220, `mod.rs` 13. The extra in
  `decode.rs` serves point 3 (decoder memory counted as allocated, so the guard writes a GAP where
  the process once grew unseen: the breaker measured 3.2 GB allocated against 384 MiB counted) and in
  `ws.rs` point 1 (every control frame recorded, the data message kept whole); both accepted. The
  4,250 row stays the ceiling: `mod.rs`, `hosts.rs`, `pump.rs`, `tunnel.rs`, `http1.rs` and `h2.rs`
  get at most 1,802 lines together (4,250 plus 10%, less 2,873), against their 1,990 estimate. The
  builder cuts what serves no point first; past 1,802 needs its own decision line naming the point.
  Tests: 1,029 lines in 4 of the 7 planned files against the 2,000 row; `proxy_http1.rs`,
  `proxy_h2.rs`, `proxy_tunnel.rs` and `proxy_restart.rs` share the 971 left (fixtures not counted).
- **On integration now:** `proxy::{ca, tls, connect, decode, ws}` with `RunCa`, `Tls`,
  `BodyDecoder`, `Step`, `parse_codings`, `WsConn`, `PrefixedIo`. Slice 2 builds `serve`, `hosts`,
  `pump`, `tunnel` and `http1` on them; `http1` feeds `BodyDecoder` and records what each `Step` says.
- No locked ledger item is touched. No new owner question comes from this slice.

### 2026-10-30, replan after PRD-14 slice 4 (merge ec0d79d8)

- **`proxy_connect` joins PRD-14's `heavy` group in the PRD-03 merge that brings its 2,000
  concurrent connections case,** not before: `proxy_connect.rs` is on integration with ClientHello
  parses only, which need no group. That merge carries one edit to `.config/nextest.toml`: add
  `proxy_connect` to the alternation of the existing override, which reads
  `filter = 'binary(/^native_walk$/)'` at ec0d79d8 (so `binary(/^(native_walk|proxy_connect)$/)`).
  Under merge rule 1 that one edit is the seam change the PRD-16 note after PRD-17 phase 0 allows.
  `proxy_restart.rs` does the same in the merge that brings it.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-08 slice 4 (merge 1fdaf2c3): what `conn_bytes` now demands of the proxy

PRD-08's `conn_bytes` is built (PRD-08 note). It judges only what the record shows, so the proxy's
slice that writes decrypted connections must meet four points or the check is blind or fails a good
run:

- **Every decrypted `conn` END carries `plain_up` and `plain_down`.** One decrypted connection with
  no tail turns the whole run's `conn_bytes` into `skip`, which hides a real loss.
- **Every HTTP stream END carries `wire`** (the body bytes on the wire; the verifier falls back to
  `seen` if absent). Heads are counted from the OPEN `headers`.
- **A parser fault on a reused decrypted connection writes a `conn` GAP** (PRD-01 section 4, `lost`
  = unparsed bytes + 1). A GAP on an HTTP request or response stream does not cover the other
  requests: an unparsed request with no `conn` GAP leaves its bytes unaccounted and `conn_bytes`
  fails (exit 2) above 1 MiB and 50% of the direction.
- **A WebSocket stream GAP carries its real `lost`** (+1 by invariant 11). `lost` 0 (size unknown)
  waives the check for the whole connection, so use it only when the size is unknown.

No locked ledger item is touched and no owner question comes from this note.

### 2026-10-30, replan after PRD-12 slice 5 (merge 9e71bde0): the pad's scripts for acceptance 3, 7 and 9

- `scripts/pad/exec/` holds the watcher commands: `h2-parallel.sh` (acceptance 3: four `--http2-prior-knowledge`
  requests in parallel to `api.openai.com`, curl 8.10.0 or later), `refusal-curl.sh` (acceptance 7:
  a client trusting only the system CAs, two calls, a 660 s sleep, a third call) and `long-tunnel.sh`
  (acceptance 9: 90 s on `stream.wikimedia.org`, a non-model host). The record decides the pass, not
  the script's exit.
- **What the pad reads from the record for acceptance 7:** the only loss a `refusal-curl.sh` row may
  name is `tls_refused`, and the verifier exit may be 1 for it alone; anything else named fails the
  row. A `tls_refused` GAP with `{"alert": false}` stays the case PRD-08 skips for `refusal_after_trust`.
  A dodger row expects `pinned` tunnels with no `tls_refused` of its own.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-31, conductor seam decision: `serve` for PRD-02 slice A (08:40Z)

PRD-02's next slice (`boot.rs`) calls `serve`, so **`serve` must be on integration before PRD-02 slice A merges**. Shape: `proxy::serve(listener: std::net::TcpListener, ctx: ProxyCtx { recorder, hosts, ca }) -> ProxyHandle` is a plain `fn` that builds and owns its 2-worker tokio runtime, spawns the accept loop and returns at once; `ProxyHandle::quiet_since() -> Mono`. `extra_roots` leaves the context: the test hook is read once inside `serve`. If slice 2's review runs long, land a compiling `serve` in a seam-only merge first (PRD-00: stubs first) so PRD-02 can build against it.

### 2026-10-31, replan after PRD-03 slice 2 (merge 47835216): pump reason, F6 test, size

- **Pump doc and `reason` (nits A-N1, N-eofcut).** `Pumped.reason` is `Complete` when the client half-closed first and the upstream then cut (`Eof` then `Cut`), and `Interrupted` (through `Dropped`) when the upstream cut ends the relay first. No byte is lost and no GAP is involved either way: a close-delimited body still ends interrupted through `down_cut`. The serve slice, which touches this block anyway, rewords the doc to say that, or maps any `ld == Cut` to `Interrupted`; either is fine, pick the smaller. Do not add code for it elsewhere.
- **`Pumped.upstream` and the two derived values (Q1).** Decide in the serve slice: if `serve` also just drops the upstream leg, drop the field; `streams` and `reason` can be one `match`. Taste only.
- **F6 mid-upload test (A-N2).** The serve slice (or the next `proxy_tunnel` touch) adds one case: a Content-Length body cut short by an upstream cut, expecting `conn` and `http_req` END interrupted and the `unsent` tail on `http_req`. One case, no stress rerun.
- **R3 note, down side.** Server bytes held when the harness aborts are recorded in `http_resp` / `ws_down`; there is no down-side `unsent` marker. Decision: none is added. The bytes are in the record, the `conn` END is interrupted, and `conn_bytes` counts them, so nothing is hidden; PRD-01's `unsent` stays up-side only.
- **Size.** `wc -l` on integration at 47835216: mod 17, hosts 113, pump 324, tunnel 190, http1 1,090, ca 439, connect 673, decode 812, tls 203, ws 750; total 4,611 against the 4,250 row (+8.5%, under the +10% line of 4,675). `h2.rs` (ceiling 1,802 in this PRD's own estimate) and `serve` are still to land and will cross +10%; the builder of each says in its merge request which point the extra lines serve (PRD-00 rule), and first cuts what serves none. No cut is on offer today.
- **Seam.** The `serve` rule in the conductor note above stands: `serve` drops the harness leg, never shuts it down. No locked ledger item is touched and no owner question comes from this slice.
