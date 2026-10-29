# PRD-03: The recording proxy

Id: PRD-03. Version: goalcup 1.0. Date: 29 October 2026. Base: main at 4b226de2.
Index: [PRD-00](PRD-00-index.md). Format: [PRD-01](PRD-01-record-format.md). Owner of `crates/goalcup/src/proxy/`
and `crates/goalcup/tests/proxy_*.rs`.

## Goal

One loopback proxy for every harness: decrypt and record byte for byte every connection to a listed
model host, relay every other connection untouched as a recorded tunnel, and never refuse or slow a call.

## Points served

1 (every model call, any provider, gateways included), 2 (subagent traffic shares the same connections and
hosts), 3 (relay first, no caps, every failure falls back to a working tunnel), 5 (every connection is a
record), 7 (HTTP/2, WebSocket, gateways, the harnesses people use). Point 4 through plaintext: every body is
decoded before PRD-05 masks it.

## Owner decisions it carries out

- **One system for every harness, no patchwork** (BRIEF3, 29 Oct night). One proxy for every connection.
  One shared host list (providers, gateways, cloud model services, vendor backends), plus hosts the
  participant registers, plus hosts the harness config names as a hint. Those are decrypted and recorded
  byte for byte. Everything else is relayed untouched and recorded as a tunnel. A long streaming tunnel to an
  unknown host is flagged. Never decrypt every host.
- **HTTP/2 is supported** (Cursor can force it). Pinned certificates fall back to a recorded tunnel with a
  named gap.
- **Proxy failure:** the recorder restarts on the same port fast, writes a gap for the dead time and never
  blocks the harness (PRD-02 owns the watchdog; this PRD makes the proxy restartable).
- **Option B, one process.** The proxy calls the recorder handle in its own process. No IPC, no producers,
  no epochs, no failure budget.
- **One recording per WebSocket direction**, raw bytes, light framing (PRD-01).
- **No caps of our own.** Only stated memory guards; past one, a gap, and recording goes on.
- **Fresh core:** copy the proven tricky parts (CA and TLS setup, HTTP/1.1 parsing, brotli, gzip, deflate,
  zstd, WebSocket deflate), delete the rest with the old crates.
- Never set a model base URL (`AGENTS.md:81`, D1, D36). The only wire path is `HTTPS_PROXY`.

## Answer to the owner's gateway question

Yes. A gateway is just another HTTPS host. Through `HTTPS_PROXY` the harness sends `CONNECT
ai-gateway.vercel.sh:443`, exactly as it sends `CONNECT api.deepseek.com:443` with a direct key. The proxy
sees a host name and nothing else, so "with a gateway" and "without a gateway" are one code path. The only
fact needed is that the gateway's host is on the list. Kilo under Vercel AI Gateway is solved the moment
`ai-gateway.vercel.sh` is on the shared list (it is, section 3). What differs between gateways is inside the
bytes (model names like `vendor/model`, extra usage fields, SSE keep-alive comments), and the recorder
never reads that; PRD-09 does, later.

Why a list and not "decrypt everything": decryption only works for a client that trusts the run CA. The
harness does (we hand it the CA). The tools the agent runs (git, curl with its own bundle, boto3 with
`certifi`, Go and Java programs, gRPC with its own roots) often do not, and would fail (point 3). The list
keeps decryption to hosts that carry model traffic. Everything else still leaves a record (host, bytes,
times), and a long stream to an unlisted host is flagged, so a missed model host is visible, not silent.

Dodgers are not a second system. A harness that ignores `HTTPS_PROXY` never reaches us at all: the native
reader (PRD-04) and an honest "no model requests written" (PRD-10) are all we can give it. A client that
pins or refuses our CA reaches us and gets a recorded tunnel with a named gap. A vendor backend (Amp,
Cursor) is a listed host like any other: its own protocol is recorded byte for byte, and the provider call
it makes on its servers is out of reach by design.

## Today

| What | Where | Fact |
|---|---|---|
| Proxy module | `crates/goalcup-cli/src/proxy/` (12 files) | 10,561 lines: 9,212 src, 1,349 inline tests. Integration tests `m2_*` 12,131 lines plus 4,027 support lines (`docs/plans/2026-10-30-audit/C-module-audits.md` section 3.2) |
| HTTP/1.1 only, both legs | `proxy/tls.rs:24-31`, `:213`, `:275`; refusal `proxy/connect.rs:624-637` | An `h2`-only client gets a `no_application_protocol` alert and no service. Cursor's server can force HTTP/2 (`O-harness-access.md` H7.3) |
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
    server first or not TLS  -> tunnel
    ClientHello              -> read SNI, ALPN list, fingerprint
       decide(SNI or CONNECT host)
         not listed, sign-in host        -> tunnel (conn record)
         listed, pinned in the list      -> tunnel + GAP pinned
         listed, learned refusal         -> tunnel + GAP tls_refused
         listed                          -> TLS upstream with the client's ALPN offer
            upstream TLS fails           -> new TCP, tunnel + GAP (upstream_untrusted)
            serve the client a leaf from the run CA, ALPN = what upstream chose
               client refuses the leaf   -> learn (host, fingerprint), GAP tls_refused, close
               ALPN h2                   -> pump + passive HTTP/2 parser
               ALPN http/1.1 or none     -> pump + passive HTTP/1.1 parser (WebSocket on 101)
```

Rules that hold everywhere:

1. **Relay first, record second.** Each direction is a pump: read a chunk, write it to the other side
   unchanged, then hand the same bytes to that connection's parser. The parser runs inline after the write
   and calls PRD-02's recorder handle (seam S1), which never waits. Nothing on the forwarding path waits for
   recording. Bytes are never re-encoded: both TLS legs carry the same plaintext, so even HTTP/2 frames
   and flow control pass through untouched.
2. **Record everything on a decrypted connection.** No path filter, no Host check, no method filter, no
   per-exchange selection. The old Host-mismatch fallback (`http.rs:11-16`) and evidence paths
   (`connectors/codex-cli/harness.toml:121-130`) go.
3. **Never refuse a connection the harness makes.** No connection cap, no stream cap, no admission
   deadline after the CONNECT head, no failure budget. Every TLS or protocol failure falls back to a
   working tunnel or keeps relaying unparsed. The only answers that are not `200` are for broken CONNECT
   requests (`400`, `431`), a non-CONNECT request (`405`, see Overridable defaults), and an upstream that
   cannot be reached at all (`502`, the same failure the harness would see without us).
4. **A parser bug never touches the relay.** Each parser call runs inside `catch_unwind`. A panic ends
   recording for that connection with a GAP (`decode_failed`, tail `{"panic":true}`) and the pump goes on.
   The workspace keeps `panic = "unwind"` (PRD-14).
5. **No proxy credential.** `HTTPS_PROXY` becomes `http://127.0.0.1:<port>`. The credential guarded the
   listener against other local users (`connect.rs:30-40`), but it gives nothing a local attacker can use
   (the CA key never leaves memory) and it costs a run secret the masker must hunt in every encoding
   (D60, `docs/DECISIONS.md:510-526`) plus a `407` for any client that drops proxy userinfo. Cut.
6. **Multi-thread runtime**, default worker count (today one thread, `commands/start/proxy.rs:98`).
7. **Raise the open-file soft limit** to the hard limit at recorder start (copy
   `commands/start/proxy.rs:33-60` without the 1024-connection target). An `EMFILE` on accept pauses
   10 ms and retries (`proxy/mod.rs` `ACCEPT_RETRY_PAUSE`); connections wait in the kernel backlog, never
   refused.

### 2. Admission

- **CONNECT head:** copy `connect.rs:728-797` (head read, request line, `host:port` and `[ipv6]:port`,
  normalization `:840-881`) without the credential check. Head guard 8 KiB (`:71`), read deadline 30 s.
- **Upstream TCP:** copy `connect_host` and `connect_addresses` (`connect.rs:962-1041`): each round
  re-resolves, IPv4 first, `TCP_NODELAY`, keepalive. Retry until 25 s when the CONNECT target is listed
  (a short outage costs a slower answer, not a `502`); one round otherwise. No idle timeout anywhere: SSE
  and WebSocket streams idle legitimately (`connect.rs:960-961`).
- **First bytes:** after `200`, race the client's first bytes against the server's. Server first means
  a tunnel (`reason: not_tls`). This fixes the 30 s hang for server-speaks-first protocols.
- **ClientHello:** copy `peek_client_hello` and `parse_client_hello` (`connect.rs:1042-1213`; it already
  joins a ClientHello split across TLS records). Extend it to return the ALPN list and a **fingerprint**:
  SHA-256 of the legacy version, the cipher suites sorted, the extension types sorted, the supported groups
  sorted, all with GREASE values removed (RFC 8701). Sorting makes it stable for TLS stacks that shuffle
  extensions. The bytes read are replayed into whichever path is chosen (`PrefixedIo`, `connect.rs:1215-1280`).
  Guard 64 KiB; past it, a tunnel.
- **The deciding name is the SNI.** When there is no SNI, the CONNECT host decides. A listed SNI behind an
  IP literal or another name is decrypted and recorded (the upstream TLS check uses the SNI name), so the
  old `SniMismatch` loss class goes. An unlisted SNI behind a listed CONNECT host is a tunnel.

### 3. The one host list (consumed, not owned)

PRD-11 owns `connectors/model-hosts.toml` and builds a `HostList` (seam S3) from three sources: shared,
participant (`goalcup setup --record-host`, PRD-10), and config (hosts the harness config names, read by
PRD-11 from today's `model_host.rs` logic). The proxy asks one question per connection:
`hosts.decide(name) -> Listed { source, pinned } | SignIn | NotListed`.

Matching rules the proxy relies on (PRD-11 implements them in S3):

- An entry is an exact name (`api.openai.com`), a subdomain pattern (`*.openai.azure.com`), or a pattern
  with one `*` inside one label (`bedrock-runtime.*.amazonaws.com`, `*-aiplatform.googleapis.com`).
- `never` entries (sign-in hosts: `auth.openai.com`, `platform.claude.com`, and the like) win over every
  pattern (point 4; today `auth_hosts`, `connectors/codex-cli/harness.toml:32`).
- `pinned = true` marks a host whose clients are known to pin: always a tunnel with GAP `pinned`.
- The list is fixed for an entry. A host added mid-run takes effect at the next `goalcup continue`,
  because the run CA is minted for the list (section 4).

Starting list this PRD asks PRD-11 to ship (facts from appendices E and O; PRD-11 verifies each row):

| Kind | Hosts |
|---|---|
| Providers | `api.anthropic.com`, `api.openai.com`, `chatgpt.com` (Codex's ChatGPT backend), `api.deepseek.com`, `api.moonshot.ai`, `api.moonshot.cn`, `api.kimi.com`, `api.z.ai`, `open.bigmodel.cn`, `api.groq.com`, `api.together.ai`, `api.together.xyz`, `api.fireworks.ai`, `generativelanguage.googleapis.com` |
| Gateways | `ai-gateway.vercel.sh`, `openrouter.ai`, `gateway.ai.cloudflare.com`, `api.kilo.ai`, `ai-gateway.kilo.ai` |
| Cloud model services | `bedrock-runtime.*.amazonaws.com`, `bedrock-mantle.*.api.aws`, `*-aiplatform.googleapis.com`, `aiplatform.googleapis.com`, `aiplatform.us.rep.googleapis.com`, `aiplatform.eu.rep.googleapis.com`, `*.openai.azure.com`, `*.services.ai.azure.com` |
| Vendor backends | `ampcode.com`, `api.factory.ai`, `api2.cursor.sh`, `*.api5.cursor.sh` (Cursor stays out of the 15; the row documents the dodger case) |
| Never decrypt | `auth.openai.com`, `platform.claude.com` (unverified additions: each cloud's token and sign-in hosts) |

Left off on purpose: `api.cloudflare.com` (it also carries the agent's own deploys; a Workers AI user
registers it), self-hosted LiteLLM and any custom base URL (the config hint or a registration covers them).

### 4. The run CA and TLS

Copy `proxy/tls.rs:100-215` and `crates/goalcup-cli/src/ca_bundle.rs` (207 lines) into
`src/proxy/ca.rs` and `src/proxy/tls.rs`, with five changes:

1. **The CA outlives a recorder boot.** Today the key dies with the process (`tls.rs:5-14`). A restarted
   recorder with a new CA would be refused by every harness, which trusts the CA file it was launched with.
   So the watchdog mints the CA once per entry (`ca::RunCa::mint(&HostList)`), writes `run-ca.pem` (the CA
   alone) and `bundle.pem` (OS roots plus the run CA, `ca_bundle.rs` logic) as 0600 files, and passes the
   key and certificate to each recorder boot over an inherited pipe (seam S8; see Asks). The key is never
   written to disk, logged or put in argv. `RunCa::import(bytes)` rebuilds it in the recorder.
2. **Name constraints from the list.** Permitted subtrees are the wildcard-free suffix of each entry
   (`openai.azure.com` for `*.openai.azure.com`, `amazonaws.com` for `bedrock-runtime.*.amazonaws.com`),
   so a leaked key still cannot sign for an unrelated name. Keep `pathLen 0`, 8 days validity (`tls.rs:55`),
   one hour of clock skew (`:59`), and the Authority Key Identifier on leaves (`:200`).
3. **Leaves on demand.** One leaf per SNI, minted at first use and cached for the boot, as a rustls
   `CertifiedKey`. A pattern entry can match many names, so leaves cannot all be made at start.
4. **ALPN is mirrored, not forced.** Upstream is offered the client's own ALPN list, filtered to `h2` and
   `http/1.1`. Whatever upstream chooses (or none) is the only protocol offered back to the client. Both
   legs then speak the same protocol and bytes pass unchanged. A harness behaves as it would without us
   (point 7). If the client offered ALPN but neither `h2` nor `http/1.1`, the connection is a tunnel.
5. **Upstream roots are the Mozilla set plus the OS store** (the same OS read as the bundle). That covers
   a corporate inspection CA or a private gateway CA the machine already trusts. If upstream validation
   still fails, or upstream asks for a client certificate, the proxy opens a fresh TCP connection and
   tunnels: the harness then validates the real certificate itself, so trust is never lowered.

**Refusal learning.** If the client aborts the handshake after our leaf was sent (an alert such as
`unknown_ca`, `bad_certificate`, `certificate_unknown`, or a close before `Finished`), the proxy records
`(host, fingerprint)` in a per-boot table, writes a GAP `tls_refused` for that connection and a `note`
`ca_refused` (host and fingerprint prefix, never a secret), and closes. Every later connection with the same
pair is tunnelled with a GAP `tls_refused`. Other fingerprints to the same host are still decrypted, so the
harness keeps being recorded while, say, a `boto3` test script falls back after one failed call. Pinned
clients look the same on the wire, so learned refusals are always `tls_refused`; `pinned` is only for
list entries marked so.

### 5. HTTP/1.1 (passive parser)

Copy the framing from `proxy/http.rs:706-1265`: head scan, fields, request and response heads, `Framing`
(length, chunked, close-delimited), `Chunked`, chunk-size lines, pipelining order, `HEAD`, `204` and
`304`, 1xx. Copy `websocket_accept_key` (`http.rs:73-82`). Rewrite the relay as passive:

- The request side opens one `http_req` stream per request (meta: method, path with query, `ver: "1.1"`,
  headers in wire order, `coding`). The response side opens `http_resp` with `req` = the request's sid
  (meta: status, headers, `coding`, `decoded`). Headers go raw into meta in memory; PRD-02 calls PRD-05's
  `mask_meta` before anything is written.
- 1xx heads become a `note` event (`informational_response`) with status and headers.
- Bodies are de-chunked and decoded (section 7), then sent as DATA in pieces of at most 256 KiB with
  decoded offsets. Chunked trailers go in the END tail (see Asks).
- **Upgrade:** a request with `Upgrade: websocket` keeps forwarding; the request side only buffers its
  copy of any later client bytes until the response side sees the answer. A `101` with the right
  `Sec-WebSocket-Accept` switches both sides to the WebSocket parser, which replays the buffered copy
  first. Any other answer replays the copy into the HTTP parser. The copy buffer has a 1 MiB memory guard;
  past it, a GAP `memory_guard` and that direction stops parsing for the connection (it still relays).
- Framing that cannot be parsed: GAP `decode_failed` on the open stream, then the connection relays
  unparsed to its end. No loss is silent.

### 6. HTTP/2 (passive parser, new code)

Full support as an observer, because both legs carry the same frames:

- Per direction: the 24-byte client preface, then frames (9-byte header). `HEADERS` plus `CONTINUATION`
  feed one HPACK decoder per direction; it follows the sender's table size updates. `DATA` payloads, with
  padding stripped, go to the stream's decoder. `END_STREAM` ends a body; a second `HEADERS` with
  `END_STREAM` is trailers. `RST_STREAM` ends both streams `interrupted` with the error code in the END
  tail. `GOAWAY` ends streams above its last stream id. `PRIORITY`, `PING`, `SETTINGS`, `WINDOW_UPDATE` and
  unknown frame types are skipped by length (RFC 9113 section 4.1). `PUSH_PROMISE` opens an `http_req`
  from the promised header block and an `http_resp` on the promised stream.
- Streams: one `http_req` per client stream, one `http_resp` per server response, meta `ver: "2"`, `h2`
  stream id, headers including pseudo-headers in wire order (see Asks).
- Extended CONNECT with `:protocol websocket` (RFC 8441): the stream's DATA payloads feed the WebSocket
  parser, giving `ws_up` and `ws_down` with `req` = that request.
- gRPC and Connect bodies are recorded as raw bodies. A message-level coding header (`grpc-encoding`,
  `connect-content-encoding`) other than `identity` is a GAP `unsupported_encoding` for that body, so no
  compressed byte reaches disk (D14).
- What a passive observer cannot follow (an HPACK error, a table past the 1 MiB memory guard, a frame
  over the 16 MiB maximum): GAP `h2_unsupported` on every open stream of the connection, a `note`, and the
  rest of the connection relays unparsed. Never a refused connection.
- HPACK: one decoder crate added in the scaffold (first choice `loona-hpack`, the maintained fork of
  `hpack`; check the version on crates.io first, unverified). If no crate is fit, write a decoder against
  the RFC 7541 appendix C vectors (about 500 lines with the Huffman table).

### 7. Body decoding

Copy `proxy/http.rs:1514-2022` into `src/proxy/decode.rs`: gzip (with `x-gzip`), deflate (zlib and raw),
brotli (`brotli-decompressor`), zstd. At most one coding; anything else is a GAP `unsupported_encoding`
and no body bytes (`http.rs:18-24`). A coding that breaks part way keeps the decoded prefix, writes a GAP
`decode_failed` and ends the stream `decode_error`. Two changes:

- No per-read decode budget (`MAX_DECODED_PER_READ`, `http.rs:97`): decoded output goes out in
  32 KiB pieces as it is produced, and PRD-02's memory guards decide. Deflate expands at most about
  1032 to 1, so the work stays bounded by the input.
- The zstd window limit stays at 8 MiB for HTTP `zstd` (RFC 9659 section 3, `http.rs:102`). Codex encodes
  its request bodies with zstd level 3 (`E-providers.md` E8.1).

### 8. WebSocket (passive parser)

Copy `proxy/websocket.rs` negotiation and framing: `negotiate` and `permessage_deflate` (`:195-265`), the
framer (`:324-420`), unmasking and the per-direction inflater with context takeover (`:553-1000`), the
`DEFLATE_TAIL` (`:80`). Change the output to PRD-01's shape:

- One `ws_up` and one `ws_down` stream per upgraded connection, opened when the `101` is seen, meta
  `deflate`. Every message is one or more DATA records with the opcode in the flags and the FIN flag on its
  last piece. Text, binary, close, ping and pong are all recorded (today control frames are dropped,
  `websocket.rs:16-18`).
- Payloads are unmasked and inflated. Under `no_context_takeover` the inflater resets per message.
  A broken frame or lost history is a GAP `decode_failed` for that message (unknown loss, `lost` 0); the
  stream goes on with the next message. Framing is always tracked, so relaying never stops.
- The END tail holds the close code when a close frame was seen.

### 9. Tunnels

Every connection that is not decrypted is a `conn` stream with `mode: "tunnel"` and a `reason`, opened when
the decision is made and ended at close with the tail `{"up": n, "down": n}`. Bytes are copied with
`copy_bidirectional` (`connect.rs:672-690`), no parse.

- To a listed host (pinned, refused, upstream untrusted, h2 not followed): also a GAP on the `conn`
  stream at close, `lost` = up plus down plus one (see Asks).
- **Long streaming tunnel flag:** a tunnel to an unlisted host that has been open 60 s and has received
  downstream bytes in at least 10 different seconds gets one `note` event `{"what":"long_tunnel",
  "conn":sid,"host":...}`. PRD-10 turns it into one status line: "<host> streamed data for N minutes and was
  not recorded. If it is a model service, run `goalcup setup --record-host <host>` and `goalcup continue`."
- Every tunnel is in the record when it opens, so a crash loses none (today they are written only at
  finish, `tunnels.rs`), and there is no row cap (`tunnels.rs:20`).

### 10. Restart

The recorder process may die (a bug, an out-of-memory kill). PRD-02's watchdog holds the listen socket and
starts the next boot at once (seam S8). What the proxy does:

- `proxy::serve(listener, ctx)` accepts on the inherited socket; it binds nothing itself.
- In-flight connections of the dead boot are cut by the kernel. The harness sees a reset and retries,
  as it would after any network drop. New connections wait in the backlog for the restart (listen backlog
  4096, set by the watchdog).
- The new boot imports the same CA, so the harness keeps trusting it. Leaves and the refusal table are
  rebuilt on demand (one more failed handshake per refusing pair per boot, at most).
- The dead time is the `recorder_down` gap PRD-02 writes. The proxy writes nothing for it.

### 11. Memory

The proxy holds per connection two read buffers (16 KiB), the TLS states, one HPACK table per direction,
an inflater window per WebSocket direction (32 KiB), and a decoder per open body (a brotli window can reach
16 MiB). Stated guards, each writing a GAP `memory_guard` with the guard name and going on: ClientHello
64 KiB, HTTP head 64 KiB, upgrade copy 1 MiB, HPACK table 1 MiB. Everything after the parser is PRD-02's
queue and its guards. There is no connection, stream or byte cap.

### 12. What a pad run sees per harness case

| Case | `conn` meta | Records | Participant sees (PRD-10) |
|---|---|---|---|
| Direct key (Claude Code to `api.anthropic.com`) | `decrypt`, `listed: shared`, `alpn` | `http_req`, `http_resp` decoded (brotli) | model requests written |
| Gateway (Kilo or Claude Code to `ai-gateway.vercel.sh`; OpenRouter) | `decrypt`, `listed: shared` | same as direct | same |
| Custom base URL (config names `llm.example.com`) | `decrypt`, `listed: config` | same | same |
| Vendor backend (Amp to `ampcode.com`) | `decrypt`, `listed: shared` | `ws_up`, `ws_down` of Amp's JSON-RPC; no provider call | "vendor backend: the model call runs on the vendor's servers" |
| Pinned or refusing client | `tunnel`, `reason: pinned` or `tls_refused` | GAP per connection, bytes each way | "not recorded: <host> refused the recorder" |
| Ignores `HTTPS_PROXY` | nothing reaches the proxy | session files only (PRD-04) | "no model requests written" alarm |

## Files it owns

Create (all under `crates/goalcup/`):

| Path | What | Estimate, src lines |
|---|---|---|
| `src/proxy/mod.rs` | `serve(listener, ProxyCtx)`, accept loop, open-file limit, per-connection task, panic guard | 200 |
| `src/proxy/connect.rs` | CONNECT head, upstream TCP with retry, first-bytes race, ClientHello (SNI, ALPN, fingerprint), decision | 650 (about 450 copied) |
| `src/proxy/ca.rs` | `RunCa` mint, export, import, subtrees from the list, leaf cache, `run-ca.pem` and `bundle.pem` | 400 (about 330 copied) |
| `src/proxy/tls.rs` | upstream client config per ALPN list, harness-facing server config per chosen ALPN, refusal detection | 200 (about 120 copied) |
| `src/proxy/hosts.rs` | `decide()` over S3 plus the per-boot refusal table | 80 |
| `src/proxy/pump.rs` | verbatim pump per direction that feeds a parser after each write | 120 |
| `src/proxy/tunnel.rs` | byte tunnel, conn record, long-tunnel watch | 120 |
| `src/proxy/http1.rs` | passive HTTP/1.1 parser | 900 (about 600 copied) |
| `src/proxy/h2.rs` | passive HTTP/2 parser | 650 |
| `src/proxy/ws.rs` | passive WebSocket parser | 600 (about 500 copied) |
| `src/proxy/decode.rs` | gzip, deflate, brotli, zstd | 500 (copied) |
| **Total** | | **about 4,400** (PRD-00 budget 4,000; the 400 over is HTTP/2, which the budget did not have) |

Tests: `tests/proxy_connect.rs`, `tests/proxy_tls.rs`, `tests/proxy_http1.rs`, `tests/proxy_h2.rs`,
`tests/proxy_ws.rs`, `tests/proxy_tunnel.rs`, `tests/proxy_restart.rs`, and fixtures in
`tests/proxy_fixtures/` (a Codex WebSocket deflate capture, an Anthropic brotli SSE body, a zstd request
body, an h2 session made with `curl --http2`). They use PRD-14's `tests/common/` fake upstream and a
recorder stub that collects records in memory.

Must not touch: `src/pipeline/` and `src/watchdog.rs` (PRD-02), `src/mask/` (PRD-05), `src/connector/` and
`connectors/**` including `model-hosts.toml` (PRD-11), `src/cli/` (PRD-10), `crates/goalcup-format/`
(PRD-01), the old crates (PRD-13 deletes them).

## Interfaces with other PRDs

| PRD | What this PRD uses or gives | Seam |
|---|---|---|
| PRD-01 | Stream kinds `conn`, `http_req`, `http_resp`, `ws_up`, `ws_down`; meta keys; gap reasons 3, 4, 6, 7, 11 and `memory_guard`; `note` values `long_tunnel`, `ca_refused`, `pinned_host`, `informational_response`, `unsupported_coding` | F1 through S1 |
| PRD-02 | Uses `Recorder::open(kind, conn, req, meta)`, `StreamHandle::{data, ws, end, gap}`, `Recorder::event`, the clock. Gives `proxy::serve(listener, ProxyCtx)`. The watchdog calls `proxy::ca::RunCa::mint` and `write_files`, and pipes `export()` to each boot; the recorder calls `RunCa::import` | S1, S8 |
| PRD-05 | Meta holds raw headers and path in memory; PRD-02 masks them with `mask_meta` before writing. Copy the name rules from `proxy/headers.rs:20-46` and the start-line, value and query rules (`headers.rs:153-240`). With the proxy credential gone, the `proxy_credential` rule and its run-secret forms (D60) have nothing to find | S2 (via PRD-02) |
| PRD-11 | Uses `HostList::decide(name)` and `HostList::ca_subtrees()`. Asks PRD-11 to set `HTTPS_PROXY` and `https_proxy` to `http://127.0.0.1:<port>`, `NO_PROXY=127.0.0.1,localhost,::1`, `NODE_USE_ENV_PROXY=1`, extend-mode CA variables to `run-ca.pem` and replace-mode ones (`SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE`) to `bundle.pem`. Replace-mode variables must get the bundle now: tunnels go through the proxy and the client checks the real certificate (today a harness's own variable gets the CA alone, `ca_bundle.rs:17-19`) | S3 |
| PRD-10 | Status lines for `long_tunnel`, `ca_refused`, `pinned`, vendor backend, and the `--record-host` input | records |
| PRD-12 | The dodger case for Claude Code, Codex and Hermes: the pad copies `model-hosts.toml` and marks the harness's model host `pinned = true`. No code seam. Pass means the harness still works, every connection to that host is a tunnel with GAP `pinned`, and the participant is told | data |
| PRD-14 | `tests/common/` fake upstream that speaks TLS (test roots passed in `ProxyCtx.extra_roots`, a plain field), HTTP/1.1, HTTP/2 and WebSocket | test support |
| PRD-17 | Lessons this PRD must pass: Codex WebSocket deflate with context takeover, Anthropic brotli SSE, Codex zstd request bodies, ClientHello split over records, leaf AKI for Python 3.13, SSE prefix reaches the client before EOF (`m2_http_relay.rs:162`), form-encoded refresh-token bodies decoded for PRD-05 | `tests/lessons_*.rs` |
| PRD-13 | Deletes `crates/goalcup-cli/src/proxy/` after PRD-17's lessons pass | |

## What is cut

From `crates/goalcup-cli/src/proxy/` (lines with inline tests, `wc -l`):

| File | Lines | Fate |
|---|---:|---|
| `api.rs` (emitter, epochs, recovery, loss dedupe, stream limit) | 824 | deleted |
| `ingest.rs` (IPC sender, failure budget, queue lanes) | 1,177 | deleted |
| `mod.rs` (lifecycle, restart supervisor, 45 counters, capture table, 503) | 1,532 | deleted; about 200 new lines in `proxy/mod.rs` |
| `outage.rs` (outage windows) | 316 | deleted |
| `tunnels.rs` (side list, 10,000 rows) | 285 | deleted; tunnels are records |
| `preflight.rs` (TCP probe before launch) | 420 | deleted; the harness shows its own network errors, and a run with no model requests says so |
| `metadata.rs` (allowlist, hints, labels) | 407 | deleted; meta is the masked head |
| `headers.rs` (header streams, masking) | 403 | deleted; PRD-05 copies the name and query rules |
| `connect.rs` | 1,565 | about 450 copied (head parse, authority, upstream connect, ClientHello, `PrefixedIo`); credential, failure classes, deadlines, `refuse` table deleted |
| `tls.rs` | 295 | about 200 copied (CA, leaf, roots), ALPN policy rewritten |
| `http.rs` | 2,257 | about 1,100 copied (framing, decoders); the active relay, selection, Host check, budget deleted |
| `websocket.rs` | 1,080 | about 500 copied (negotiation, framer, unmask, inflate); per-message streams, indexes, recovery deleted |
| **Total** | **10,561** | about 2,250 copied, about 8,300 deleted |

Also cut: `crates/goalcup-cli/src/commands/start/proxy.rs` (437; about 30 lines copied for the open-file
limit), and `ca_bundle.rs` (207) moves into `proxy/ca.rs`.

Tests that go (with the old crate, PRD-13): `m2_proxy_integration.rs` (3,025), `m2_http_relay.rs`
(2,249), `m2_websocket_relay.rs` (2,074), `m2_tls_connect.rs` (1,275), `m2_capture_sender.rs` (1,991),
`m2_emitter.rs` (1,517), and their support (4,027). Behaviours inverted on purpose:
`excess_connections_are_refused_predictably` (`m2_proxy_integration.rs:1846-1900`),
`http1_alpn_policy_is_explicit` (`m2_tls_connect.rs:726`), the restart-cap tests (`:2920-3025`) and the
failure-budget tests (`m2_capture_sender.rs:1123-1288`).

## Acceptance

Real-run proofs first (on the pad, PRD-12, Linux sandbox, real keys). A watcher runs
`goalcup-verify check` and `goalcup-verify dump` (PRD-08) on each entry.

1. **Direct key, HTTP/1.1, brotli.** Claude Code with one subagent. Pass: every `conn` to
   `api.anthropic.com` is `decrypt`; every `http_resp` with a `br` coding has `decoded: true` and its DATA
   parses as SSE; POSTs to `/v1/messages` equal the assistant turns in the session files (PRD-12's check);
   zero `unsupported_encoding`, zero `tls_refused` on a listed host.
2. **Direct key, WebSocket with deflate.** Codex with one subagent. Pass: one `ws_up` and one `ws_down` per
   WebSocket connection; every `ws_up` text message parses as JSON; the count of `response.create` messages
   equals Codex's own turn count within the rate-limit retries; zero `decode_failed`.
3. **HTTP/2.** Codex's HTTP fallback (it offers `h2`) or `curl --http2` run by the agent against a listed
   host. Pass: a `conn` with `alpn: "h2"`, `http_req` and `http_resp` with `h2` stream ids, decoded bodies,
   and a concurrent-streams case with at least 4 streams on one connection.
4. **Gateway.** Kilo through Vercel AI Gateway, and one harness through OpenRouter. Pass: the gateway host
   is `listed: shared` with no config hint needed; wire POSTs equal the harness's own calls; the gateway key
   header is masked.
5. **Vendor backend.** Amp. Pass: `ampcode.com` is decrypted; the thread WebSocket is recorded in both
   directions; no `api.deepseek.com` connection appears (expected); the status says "vendor backend".
6. **Dodger case for the core set** (pinned by the pad's list copy). Pass: the harness finishes its task;
   every connection to its model host is a tunnel with GAP `pinned`; nothing claims "recording works".
7. **Refusal fallback.** The agent runs
   `curl --cacert /etc/ssl/certs/ca-certificates.crt https://api.openai.com/v1/models` twice. Pass: the
   first fails with one GAP `tls_refused` and one `ca_refused` note; the second gets a `401` through a tunnel;
   the harness's own calls to the same host stay `decrypt`.
8. **Restart.** The kill test of PRD-01 acceptance 3 (Codex, `kill -9` twice). Pass: after each restart
   the harness's new connections are `decrypt` (the CA survived), not `tls_refused`; the harness finishes.
9. **Long tunnel.** The agent runs `curl -N https://stream.wikimedia.org/v2/stream/recentchange` for
   90 s. Pass: one `long_tunnel` note for that host, no other effect.
10. **Load.** Codex with 20 or more subagents for 30 minutes. Pass: zero proxy GAPs of any kind on listed
    hosts; no `503` or refused connection; wire POSTs plus `response.create` messages match Codex's record
    per 10-minute bucket (the run 1 comparison).

Automated tests that guard them (fake upstream, recorder stub):

- `proxy_connect.rs`: SNI decides, no SNI uses the CONNECT host, a listed SNI behind an IP literal is
  decrypted, a server-first protocol tunnels with no wait, 2,000 concurrent connections are all served
  (no cap), a non-CONNECT request gets `405`.
- `proxy_tls.rs`: the ALPN mirror matrix (h2 plus h1 against h2, against h1, against none); upstream
  certificate failure falls back to a tunnel with a GAP; a refusing client teaches the table and a second
  fingerprint is still decrypted; a CA exported and imported signs leaves the first boot's CA file trusts.
- `proxy_http1.rs`: each coding decodes to the plaintext; an unknown coding writes no body byte; chunked
  trailers reach the END tail; pipelining pairs responses; `HEAD` and `304` have no body; the SSE prefix
  reaches the client before EOF; a parser panic leaves the relay running.
- `proxy_h2.rs`: CONTINUATION, HPACK dynamic table and size updates, padding, trailers, `RST_STREAM`,
  `GOAWAY`, extended CONNECT WebSocket, `grpc-encoding: gzip` gives a GAP, an HPACK error gives
  `h2_unsupported` and the relay goes on byte for byte.
- `proxy_ws.rs`: the Codex fixture inflates to the recorded JSON; takeover and no-takeover; a fragmented
  message is one message with FIN on its last piece; control frames recorded; client masking removed.
- `proxy_tunnel.rs`: up and down counts are exact; the long-tunnel note fires once.
- `proxy_restart.rs`: `serve` on an inherited listener after the first boot is dropped; a queued connection
  is served by the second boot with the same CA.

## Risks and blind spots

| Risk | What could go wrong | What we do |
|---|---|---|
| ALPN mirroring changes the protocol harnesses use through us | Codex and others move from HTTP/1.1 to HTTP/2 on `api.openai.com`; the new HTTP/2 parser is then on the hot path of the core set | It matches how they run without us (point 7). Parser panics never touch the relay; HTTP/2 is acceptance 3 and 10. Fallback if the core set fails on h2: offer only `http/1.1` upstream (one line, an overridable default) |
| First refused handshake per (host, fingerprint) per boot | That one call fails for a tool, or for a harness that refuses our CA (for example Hermes on Bedrock through `boto3`, which trusts only `certifi`, `E-providers.md` E6.5) | Accepted. Most clients retry; the second call tunnels. Hosts known to pin go in the list as `pinned` |
| Wider decryption records the agent's own calls to listed hosts | A test suite calling `api.openai.com` with the bundle shows up as model requests; Amp's `/api/internal` (e-mail, secret fingerprints, `connectors/amp-cli/harness.toml:105-109`) is now recorded | Masking still applies (point 4). PRD-09 tells calls apart by headers and thread hints. PRD-12's wire-against-log check allows extra wire calls and reports them |
| Name constraints use wide suffixes for cloud patterns | `amazonaws.com` as a permitted subtree weakens the leaked-key guard for AWS names | The key never leaves memory or the watchdog pipe; only processes we launch trust the CA |
| CA handed over a pipe | A bug in the hand-off makes every restart refuse all hosts | Acceptance 8 and `proxy_restart.rs` prove it |
| Brotli windows | Many concurrent brotli bodies can hold up to 16 MiB each | Measured in acceptance 10 (RSS); PRD-02's guards cover queues, not decoders |
| ECH, HTTP/3, UDP | A client with Encrypted Client Hello shows its public outer name; QUIC and WebRTC never reach a CONNECT proxy | Tunnel by outer name; not supported, stated in PRD-15's guide |
| No credential | Another local user can route traffic through the proxy into this run | Loopback only; it cannot read anything out. Single-user sandboxes and laptops are the target |
| `405` for non-CONNECT | A client that sends `http://` requests to `HTTPS_PROXY` is refused | goalcup never sets `HTTP_PROXY`, so no harness should; the pad counts `405`s |
| HPACK crate quality | An unmaintained or panicking decoder | Panic guard; RFC 7541 vectors in `proxy_h2.rs`; own decoder as the fallback |
| Unverified facts | Bun `fetch` and HTTP/2 through a proxy; Cursor trusting `NODE_EXTRA_CA_CERTS` on its HTTP/2 path; the hosts marked unverified in section 3 | Each is a pad row, not an assumption |

## Overridable defaults

| Default | Value | Who may change it |
|---|---|---|
| Upstream connect retry for listed targets | until 25 s, pause 250 ms doubling to 2 s (`connect.rs:86-89`) | PRD-03 |
| ALPN | mirrored (`h2` or `http/1.1`) | PRD-03, to `http/1.1` only if HTTP/2 fails the core set |
| Long tunnel | open 60 s and downstream bytes in 10 different seconds | PRD-10 with PRD-03 |
| Memory guards | ClientHello 64 KiB, HTTP head 64 KiB, upgrade copy 1 MiB, HPACK table 1 MiB | PRD-03 |
| Non-CONNECT requests | `405` | Owner (BRIEF3 says never refuse; this refuses a request type no harness sends) |
| Proxy credential | none | Owner |
| Refusal table | per boot, in memory | PRD-03 |

## Asks of PRD-00 and PRD-01

PRD-00:

1. **S8 must carry the run CA.** The watchdog mints the CA (calling `proxy::ca`), writes the PEM files before
   it starts the harness, and passes the key and certificate to every recorder boot over an inherited pipe.
   Without this, the first restart breaks every decrypted host, because the harness trusts the old CA file.
   S8 today names only the socket, the boot number and the previous exit status.
2. **S1 needs three details:** `end(reason, tail)` with an optional JSON tail (the `conn` up and down counts,
   trailers, close codes, `RST_STREAM` codes); `gap(reason, start, end, off, lost, tail)` with the stream
   offset and lost bytes; and the caller owns DATA offsets for both `data` and `ws` (the proxy knows decoded
   offsets and holes).
3. **S3 needs** `decide(name)` with the pattern rules of section 3 (exact, subdomain, one `*` in a label,
   `never` wins, `pinned`) and `ca_subtrees()`.
4. **Scaffold dependencies (PRD-02's `Cargo.toml`):** `rustls` (ring), `tokio-rustls`, `rcgen`,
   `webpki-roots`, `flate2`, `brotli-decompressor`, `zstd`, `sha1`, `sha2`, `base64`, `socket2`, `libc`,
   and one HPACK decoder (section 6). The locked versions today: rustls 0.23.43, tokio-rustls 0.26.5,
   rcgen 0.14.10, webpki-roots 1.0.9, brotli-decompressor 5.0.3, flate2 1.1.10, zstd 0.13.3, socket2 0.6.5
   (`Cargo.lock`).
5. **The dodger paragraph** ("One system for every harness") calls Amp a dodger. Under this design Amp
   and Cursor are vendor backends: listed, decrypted, recorded byte for byte, with the provider call out of
   reach. "Dodger" should mean only a client that ignores `HTTPS_PROXY` or refuses our CA. HTTP/2 no
   longer blocks Cursor.
6. **The size budget** for the proxy should be about 4,400 lines, not 4,000 (HTTP/2 was not in it).

PRD-01:

1. **GAP on a `conn` stream.** Say that a GAP whose `sid` is a `conn` stream (no DATA) is allowed, with
   `off` 0 and `lost` = wire bytes both ways plus one, and that invariant 4 does not apply to `conn`
   streams.
2. **One more gap and tunnel reason:** `upstream_untrusted` (a listed host whose server certificate fails
   the recorder's check, or that asks for a client certificate; the connection is tunnelled). Proposed
   GAP reason 12.
3. **Tunnel reasons** `sign_in` (a `never` host) and `alpn_other` (the client offered only protocols we do
   not parse), beside `not_listed`, `tls_refused`, `pinned`, `not_tls`, `h2_unsupported`.
4. **END tail keys** for `http_req` and `http_resp`: `trailers` (masked header list), `wire` (bytes on the
   wire before decoding, since `seen` counts decoded bytes), `h2_error` (an `RST_STREAM` or `GOAWAY` code);
   for `ws_up` and `ws_down`: `close_code`.
5. **HTTP/2 headers:** say that `headers` holds the pseudo-headers (`:method`, `:path`, `:authority`,
   `:status`) in wire order, so nothing is dropped.
6. **`conn` meta `tls`:** allow `null` for a listed host reached in plain text after `CONNECT` (it is parsed
   like a decrypted connection) and add `fingerprint` (first 16 hex of the ClientHello fingerprint) so a
   `ca_refused` note can be matched to its tunnels.

## Builder notes

Order of work:

1. `connect.rs`, `ca.rs`, `tls.rs`, `hosts.rs`, `tunnel.rs`, `mod.rs` with a stub parser that records
   nothing: every connection relays. Prove "never refuse" first (2,000 connections, server-first, refusal
   learning, restart with an imported CA).
2. `pump.rs`, `decode.rs` and `http1.rs` against S1 stubs. This is the first piece for the early real test
   (PRD-00 phase 1a): Claude Code direct key.
3. `ws.rs`. Second early real test: Codex direct key.
4. `h2.rs`. Then the gateway and vendor runs.

Traps from the audit and the old code:

- Write to the peer before parsing, every time. Run 1's loss came from the hand-off, not the relay; keep
  the relay free of anything that can wait.
- `ServerConfig` carries the ALPN list, so build one per (host, chosen ALPN) and cache the three variants;
  do not build one per connection.
- rustls with the ring provider does not do post-quantum key exchange; a client that offers both a hybrid
  and a classic share still works. Check a client that sends only a hybrid share (unverified).
- A ClientHello can span TLS records (`connect.rs:1090-1120` handles it); keep that.
- Leaves need the Authority Key Identifier (`tls.rs:17-19`, Python 3.13).
- WebSocket: under context takeover a lost history can never be reset mid-connection
  (`websocket.rs:43-46`); record later compressed messages as gaps.
- Do not reuse the old `[masked]` marker or mask in the proxy. Raw meta goes to PRD-02; masking is PRD-05's.
- Never log a header value, a body byte, the CA key or a fingerprint with a host in the same line as a
  secret. Errors carry fixed text (today's rule, `tls.rs:62-63`).
- `catch_unwind` needs `UnwindSafe`; wrap the parser state in `AssertUnwindSafe` and drop that
  connection's parser after a panic.

How a watcher proves it in a real run: after each pad run, `goalcup-verify dump <entry> | jq` for
`k=="open" and skind=="conn"`; every listed host must show `mode: "decrypt"` except the cases in
acceptance 6 and 7; count `gap` records by reason (any proxy reason on a listed host outside those cases
is a failure); compare wire POSTs plus `ws_up` text messages with the harness's own log (PRD-12); grep the
dump for `long_tunnel` and read each host by eye.
