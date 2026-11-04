# cr-proxy notes (proxy/, pipeline/, mask/) -- draft, verdict pending

## Findings (top first)
- important: macOS upstream roots = SystemRootCertificates.keychain only (proxy/ca.rs:61,352-366); admin/MDM roots in /Library/Keychains/System.keychain missing. Behind TLS-inspecting corp network: every listed call -> upstream_untrusted tunnel (unrecorded), AND harness then fails TLS because launch.rs:28-30 removed the participant's own NODE_EXTRA_CA_CERTS/SSL_CERT_FILE/REQUESTS_CA_BUNDLE (bundle.pem = same SystemRoots + run CA). Harness blocked. Fix: add System.keychain roots + append participant's original CA files to upstream roots and bundle.pem.
- important: participant's HTTPS_PROXY/HTTP_PROXY/ALL_PROXY removed (connector/launch.rs:19-27) and connect_upstream dials direct (proxy/connect.rs:261-296), no upstream-proxy chaining. Proxy-only network => every CONNECT 502 => harness blocked. Fix: carry the original proxy to the recorder and CONNECT through it, or refuse at start with a plain message.
- important: refusal learning re-probes our leaf every 10 min (proxy/hosts.rs:19 REFUSAL_EXPIRY) and learns only on explicit alerts (proxy/mod.rs:323-333). A client that can never trust the run CA (Go on macOS ignores SSL_CERT_FILE -> crush-cli; docs/harnesses/crush-cli.md:44 covers Linux only) fails one real model call per (host,fingerprint) every 10 min for 168 h; a client that closes without alert fails every call. Fix: second refusal of a pair => refused for the boot (or backoff), and learn repeated no-alert closes right after our Certificate.
- minor: upstream TLS handshake (proxy/mod.rs:289) and client accept (mod.rs:320) have no deadline; an upstream that stalls mid-handshake pins the task + 2 fds (Admitted) forever after the harness gave up. Fix: bound the upstream handshake (e.g. 60 s) and race it with client EOF.
- minor: null mode still runs the HTTP/h2 parsers and body decoders on the 2 relay threads (pipeline/mod.rs:832-851 -> proxy/mod.rs:390-394) though nothing is recorded. Fix: parser None when recorder has no tx.
- minor: body decoding (gzip/br/zstd) runs inline on the relay task between chunks (pump.rs:164-176 feed after write); with RELAY_THREADS=2 many concurrent decoding streams delay relays. Acceptable, note only.

## Strengths
- relay-first then parse, parser behind catch_unwind; masking off the relay on a low-priority thread; unbounded item channel behind one held-bytes guard => relay never waits on disk/masker.
- secrets: headers by name (key/token/secret/password/auth, cookies), query stripped from :path and HTTP/1 target, exact values in 15 forms, fail-closed hold, mask panic => GAP not raw bytes, masker build failure => die (null mode), no core dumps, panic hook never prints payload.
- CA: name-constrained, pathLen 0, key never in Debug, removed at seal; leaf per SNI; upstream-first so bad upstream certs are never laundered.

## Re other lanes
- arch-10 agree, confirmed: pipeline/mod.rs:995 null recorder returns on Restart -> ProxyHandle drop (proxy/mod.rs:79-88 shutdown_background) cuts every open relay. Same for arch-9 (any recorder exit). Fix as arch says: stop accepting, keep relaying open conns until they close (tunnel-style, no parser) before exit; null mode needs no parser at all (my minor).
- cr-watchdog respawn-forever: agree from proxy side: the listen socket stays in the watchdog, so harness CONNECTs sit in the backlog with nothing serving them (harness hangs, not fails).
## Inbox: none received.
## Verdict: With fixes (3 important: corp CA roots/env, upstream proxy chaining, 10-min refusal re-probe). Hot path design and masking are strong.

## Answers (cross-talk round)
- inbox-cr-proxy.md: no file, no questions received.
- cr-watchdog confirmed (cr-watchdog.md:17) nothing carries participant HTTPS_PROXY / CA env and start prints no warning: cr-proxy-1/2 stand as important.

## Re other lanes (round 2, checked in code)
- cr-upload-server-3 agree important, same root as cr-proxy-1/2: upload/client.rs:213-214 webpki roots only, :228 try_proxy_from_env(false). On a proxy-only or TLS-inspecting network sign-in/open/upload fail before the harness runs. One shared fix: read participant proxy + extra CA files once at start, feed both the recorder upstream (connect.rs) and ServerClient (token may go through a CONNECT tunnel: TLS is end to end).
- arch-1 agree (already noted): pipeline/mod.rs:995 null recorder returns -> ProxyHandle drop cuts relays.
- cr-format-verify-1 severity-down to minor: labels come from model-hosts.toml compiled into the recorder; a cheater who can relabel a conn can also not record it, and parts upload live. Real value of the fix is catching hosts added to model-hosts.toml after the recorder build. honesty.rs:258 already unions conn and start-event labels.
- cr-format-verify-11 agree minor (defence in depth): rec/ is created 0700 and chmod'ed (watchdog/mod.rs:933-939), so 0644 part files are not readable by other users in place.
- cr-scripts C3 / cr-format-verify C3 confirm: recorder masks a known secret only when contiguous; mask/rules.rs:565 and verify history.rs:38 state it is not masked across SSE deltas; pad check.py:242-256 turns those hits into PASS. See cr-proxy-x1.

## New
- cr-proxy-x1 important: a known secret (from .env) that the model echoes in a streamed answer is stored raw, split across SSE delta events (JSON framing between pieces defeats the contiguous masker). Owner-accepted "stated loss" in PRD-05, but not told to participants (no mention in PARTICIPANT-GUIDE/RULES/README) and uploaded. Points 4 and 6. Fix: for text/event-stream bodies, also run known-value matching over the joined decoded delta strings per stream and star the matching pieces; at minimum disclose it in the guide.
