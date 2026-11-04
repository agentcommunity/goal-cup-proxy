# Lane B: Egress: upstream proxy and trust roots, proxy hardening

> **Owner descope (4 Nov 2026), overrides this file:** no egress layer. B.1-B.4 and B.9 are dropped
> (participants rarely sit behind company proxies). B.8 is instead a refusal: `goalcup setup` and
> `goalcup start` stop with one plain line when `HTTPS_PROXY`/`https_proxy`/`ALL_PROXY`/`all_proxy`
> is set (`proxy::proxy_network`). B.5, B.6 and B.7 stand as written. Review focus 1-3 and 5 and
> the `crate::net` contract below no longer apply.
**Wave:** 1   **Depends on:** none   **Branch:** fix/net
**Covers findings:** arch-x2, cr-proxy-1, cr-proxy-2, cr-upload-server-3, cr-upload-server-9, cr-proxy-3, cr-proxy-4, cr-proxy-5, cr-proxy-x1 (dropped)
**Owns files:** Create `crates/goalcup/src/net.rs`, `crates/goalcup/tests/net_egress.rs`, `crates/goalcup/tests/proxy_upstream.rs`. Modify `crates/goalcup/src/proxy/{ca,connect,hosts,mod,tls}.rs`, `crates/goalcup/src/upload/client.rs`, `crates/goalcup/tests/proxy_tls.rs`, `crates/goalcup/tests/lessons_tls.rs`.
**Shared files (only these parts):** `src/lib.rs` (one line `pub mod net;` after `pub mod mask;`, line 18); `src/pipeline/mod.rs` the `ProxyCtx { .. }` literal at 846-850 (one field `null,`); `tests/proxy_fixtures/mod.rs` the `ProxyCtx` literal at 40-44 (one field `null: false,`) plus one new fn `serve_null`; `src/cli/start.rs` one line after 292 (`lines.push(text::upload_to(..))`); `src/cli/setup.rs` one array item in `machine()` at 238-243; `tests/cli_setup.rs` one helper `setup_env` beside `setup` (61-70) and one test. `connector/launch.rs` 19-31: **no edit** (the harness still gets goalcup's proxy and CA files; the recorder now carries the participant's originals).
**Produces (for other lanes):** `crate::net` (final contract, replaces the brief's sketch):
```rust
pub const INSIDE_RUN: &str = "GOALCUP_TIME_LEFT_FILE"; // set only in a harness goalcup launched
pub struct Egress { /* private: proxy, no_proxy, os, own, unusable */ }   // Debug never shows a password
pub struct Upstream { pub host: String, pub port: u16, pub from: &'static str, /* private auth */ }
impl Egress {
    pub fn load() -> &'static Egress;                                   // once per process (OnceLock)
    pub fn from_vars(vars: &BTreeMap<String, String>, os: Vec<Vec<u8>>) -> Egress; // pure
    pub fn proxy_for(&self, host: &str) -> Option<&Upstream>;
    pub fn roots(&self) -> rustls::RootCertStore;   // webpki + os + own
    pub fn bundle(&self) -> Vec<Vec<u8>>;           // os then own, DER, deduped, order kept
    pub fn own(&self) -> &[Vec<u8>];                // the participant's CA files, DER
    pub fn line(&self) -> Option<String>;           // one plain line for start and setup
}
impl Upstream {
    pub fn connect_head(&self, host: &str, port: u16) -> String;
    pub fn ureq(&self) -> ureq::Proxy;
}
pub fn admin_trusted(plist: &str) -> std::collections::HashSet<String>; // upper-case SHA-1 hex
pub(crate) fn os_store() -> Vec<Vec<u8>>;
```
`ServerClient::new(base, token)` keeps its signature (callers in approve.rs, continue_run.rs, setup.rs, upload/mod.rs do not move); new `ServerClient::with_egress(base, token, &Egress)`. `Tls::new(&RunCa, Vec<CertificateDer>)` and `RunCa::write_files(&Path)` keep their signatures.
**Consumes:** none

## Dropped
- cr-proxy-x1: not egress; it is a masker change (cross-delta known-value matching in `mask/values.rs`/`mask/stream.rs`, not owned here) to a loss the owner accepted in PRD-05. The point-6 half (tell participants) belongs to the docs lane: one sentence in PARTICIPANT-GUIDE that a key the model echoes in a streamed answer may be stored split across deltas. Owner question if code is wanted before the event.
- Line drift, not a drop: cr-proxy-4 cites `proxy/mod.rs:289/320`; today the upstream handshake is `mod.rs:314` and the client accept `mod.rs:353`. cr-upload-server-3 cites `client.rs:221`; `try_proxy_from_env(false)` is at 228, the webpki-only roots at 213-214.

## Review focus
1. A proxy password (`http://u:s3cret@host`) never reaches Debug, `line()`, `goalcup.log` or a `ServerError` text (point 4). Tests: B.1 step 1, B.4 step 1.
2. The run token goes to a proxy only inside a CONNECT tunnel, never in a plaintext request; an `http://` server (loopback only) is never proxied. Test: B.4 step 1.
3. Inside a harness goalcup launched, `HTTPS_PROXY` is goalcup's own loopback recorder: a nested `goalcup` must not chain into it (loop, token through the recorder). Test: B.1 step 1 (`INSIDE_RUN`).
4. A participant proxy that refuses (407, close) gives the harness a 502 and one `connect_refused` note, as without goalcup. Test: B.3 step 1.
5. A System.keychain certificate the admin set to Deny, or never trusted (this Mac holds 3 untrusted Apple certs there, admin trust list empty), is not added as a root. Test: B.1 step 1 (`admin_trusted`).

Test commands follow `scripts/gate.sh:80` (`cargo nextest run --locked --workspace`), narrowed. PATH needs `~/.cargo/bin` first on the Mac.

### Task B.1: `net.rs`, the egress contract   (M, PARALLEL-OK)
**Files:** Create `crates/goalcup/src/net.rs`; Modify `crates/goalcup/src/lib.rs:18` (+1 line); Test `crates/goalcup/tests/net_egress.rs`
**Interfaces:** Produces the `net` contract above.
- [ ] Step 1: Write the failing tests in `tests/net_egress.rs` (`vars(&[(k, v)]) -> BTreeMap` helper; `A40 = "A".repeat(40)`, `B40 = "B".repeat(40)`):
```rust
#[test] fn the_participants_proxy_is_used_and_its_password_kept_out_of_sight() {
    let e = Egress::from_vars(&vars(&[("HTTPS_PROXY", "http://u:s3cret@proxy.corp:8080/")]), vec![]);
    let up = e.proxy_for("api.openai.com").unwrap();
    assert_eq!((up.host.as_str(), up.port, up.from), ("proxy.corp", 8080, "HTTPS_PROXY"));
    assert_eq!(up.connect_head("api.openai.com", 443),
        "CONNECT api.openai.com:443 HTTP/1.1\r\nHost: api.openai.com:443\r\nProxy-Authorization: Basic dTpzM2NyZXQ=\r\n\r\n");
    assert_eq!(e.line().unwrap(),
        "goalcup sends model calls and uploads through your proxy proxy.corp:8080 (HTTPS_PROXY).");
    assert!(!format!("{e:?}{up:?}").contains("s3cret"));
}
#[test] fn proxy_variables_are_read_in_curls_order() {
    let at = |p: &[(&str, &str)]| Egress::from_vars(&vars(p), vec![])
        .proxy_for("api.openai.com").map(|u| (u.host.clone(), u.port));
    assert_eq!(at(&[("HTTPS_PROXY", "http://a:1"), ("https_proxy", "http://b:2")]), Some(("b".into(), 2)));
    assert_eq!(at(&[("ALL_PROXY", "c:3")]), Some(("c".into(), 3)));
    assert_eq!(at(&[("HTTPS_PROXY", "http://d")]), Some(("d".into(), 80)));
    assert_eq!(at(&[("HTTP_PROXY", "http://e:4")]), None); // https traffic only
    assert_eq!(at(&[("HTTPS_PROXY", "")]), None);
}
#[test] fn a_proxy_goalcup_cannot_speak_is_named_and_skipped() {
    for url in ["socks5://s:1080", "https://p:443"] {
        let e = Egress::from_vars(&vars(&[("HTTPS_PROXY", url)]), vec![]);
        assert!(e.proxy_for("api.openai.com").is_none());
        assert_eq!(e.line().unwrap(),
            "goalcup cannot use the proxy in HTTPS_PROXY (only http:// proxies work), so it connects directly.");
    }
}
#[test] fn no_proxy_loopback_and_a_goalcup_harness_go_direct() {
    let e = Egress::from_vars(&vars(&[("HTTPS_PROXY", "http://p:3128"),
        ("NO_PROXY", ".corp.example, internal.host,*.svc")]), vec![]);
    for h in ["git.corp.example", "corp.example", "internal.host", "a.svc", "127.0.0.1", "localhost", "::1"] {
        assert!(e.proxy_for(h).is_none(), "{h}");
    }
    assert!(e.proxy_for("api.openai.com").is_some());
    assert!(e.proxy_for("notinternal.host").is_some());
    let star = Egress::from_vars(&vars(&[("HTTPS_PROXY", "http://p:3128"), ("no_proxy", "*")]), vec![]);
    assert!(star.proxy_for("api.openai.com").is_none());
    let dir = tempfile::tempdir().unwrap();
    let pem = dir.path().join("a.pem");
    std::fs::write(&pem, RunCa::mint(&["example.org".into()]).unwrap().export().cert).unwrap();
    let inside = Egress::from_vars(&vars(&[("HTTPS_PROXY", "http://127.0.0.1:40123"),
        (INSIDE_RUN, "/x/upload.json"), ("SSL_CERT_FILE", pem.to_str().unwrap())]), vec![]);
    assert!(inside.proxy_for("api.openai.com").is_none());
    assert!(inside.own().is_empty());
    assert_eq!(inside.line(), None);
}
#[test] fn the_participants_ca_files_join_the_roots_once() {
    let (dir, ca, os) = (tempfile::tempdir().unwrap(), RunCa::mint(&["example.org".into()]).unwrap(),
                         RunCa::mint(&["example.net".into()]).unwrap());
    let pem = dir.path().join("a.pem");
    std::fs::write(&pem, ca.export().cert).unwrap();
    let p = pem.to_str().unwrap();
    let e = Egress::from_vars(&vars(&[("SSL_CERT_FILE", p), ("NODE_EXTRA_CA_CERTS", p),
        ("REQUESTS_CA_BUNDLE", "/no/such/file")]), vec![os.cert_der().to_vec()]);
    assert_eq!(e.own(), [ca.cert_der().to_vec()]);
    assert_eq!(e.bundle(), [os.cert_der().to_vec(), ca.cert_der().to_vec()]);
    assert_eq!(e.roots().len(), webpki_roots::TLS_SERVER_ROOTS.len() + 2);
}
#[test] fn only_admin_trusted_system_keychain_certificates_count() {
    let plist = format!("<plist version=\"1.0\"><dict><key>trustList</key><dict>\
        <key>{A40}</key><dict><key>trustSettings</key><array/></dict>\
        <key>{B40}</key><dict><key>trustSettings</key><array><dict>\
        <key>kSecTrustSettingsResult</key><integer>3</integer></dict></array></dict>\
        </dict><key>trustVersion</key><integer>1</integer></dict></plist>");
    assert_eq!(admin_trusted(&plist), HashSet::from([A40.clone()]));
    let empty = "<plist version=\"1.0\"><dict><key>trustList</key><dict/>\
        <key>trustVersion</key><integer>1</integer></dict></plist>";
    assert!(admin_trusted(empty).is_empty());
}
```
- [ ] Step 2: Run, expect FAIL (compile error, `goalcup::net` does not exist): `cargo nextest run -p goalcup --test net_egress`
- [ ] Step 3: Implement `src/net.rs` with the contract above. Pinned:
  - Proxy vars in order `https_proxy, HTTPS_PROXY, all_proxy, ALL_PROXY`, first non-empty wins; `HTTP_PROXY` never read. Accepted: `http://[user[:pass]@]host[:port][/]` or bare `host[:port]`; port default 80; user and pass percent-decoded. Any other scheme: no proxy, `unusable = Some(var)`.
  - NO_PROXY from `no_proxy` then `NO_PROXY`: comma list, trimmed, lower-case, leading `*.` or `.` stripped; `*` = all direct; host matches when equal or ending in `.` + entry. Loopback (`localhost`, `127.0.0.0/8`, `::1`) always direct.
  - `INSIDE_RUN` present: no proxy and no `own` files (those variables are goalcup's own there).
  - `own`: certificates (`ca::certificates`) of `SSL_CERT_FILE`, `NODE_EXTRA_CA_CERTS`, `REQUESTS_CA_BUNDLE` in that order, a missing or unreadable file skipped, deduped by DER.
  - `connect_head`: exactly the step-1 string; `Proxy-Authorization: Basic <base64(user:pass)>` only with a user.
  - Debug of both types: `Upstream(host:port)` only.
  - `os_store()` (moved from `ca.rs:350-382` with `LINUX_BUNDLES` and `MACOS_ROOTS_KEYCHAIN`): Linux unchanged (first bundle holding a certificate). macOS: `security find-certificate -a -p /System/Library/Keychains/SystemRootCertificates.keychain`, plus each certificate of `security find-certificate -a -p /Library/Keychains/System.keychain` whose upper-case SHA-1 (`sha1` crate, already a dependency) is in `admin_trusted(<security trust-settings-export -d FILE>)`, FILE created new (O_EXCL) in `std::env::temp_dir()` as `goalcup-trust-<pid>.plist`, removed after the read. Every `security` call `env_clear`, stdin/stderr null, as today.
  - `admin_trusted`: every `<key>` of 40 hex digits inside `trustList`, except one whose dict holds `kSecTrustSettingsResult` `3` (Deny).
  - `load()`: `OnceLock`, `from_vars(&cli::env_vars(), os_store())`.
- [ ] Step 4: Run, expect PASS: `cargo nextest run -p goalcup --test net_egress`
- [ ] Step 5: Commit: `git add crates/goalcup/src/net.rs crates/goalcup/src/lib.rs crates/goalcup/tests/net_egress.rs; git commit -m "net: one egress for proxy and trust roots (arch-x2)"`

### Task B.2: run CA files and upstream roots from Egress   (S)
**Files:** Modify `src/proxy/ca.rs` 44-61 (drop the moved consts), 186-195 (`write_files`), 350-382 (removed, moved to net), 426-439 (`bundle_pem` takes DER list); `src/proxy/tls.rs` 66-73. Test `tests/lessons_tls.rs`, `tests/proxy_tls.rs`.
**Interfaces:** Consumes `Egress::load().bundle()`, `.own()`, `.roots()`.
- [ ] Step 1: Write the failing tests:
```rust
// lessons_tls.rs (its own process under nextest; env set before the first Egress::load)
#[test] fn the_ca_files_carry_the_participants_ca() {
    let other = RunCa::mint(&["example.org".into()]).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let theirs = dir.path().join("corp.pem");
    std::fs::write(&theirs, other.export().cert).unwrap();
    unsafe { std::env::set_var("SSL_CERT_FILE", &theirs) };
    let ca = RunCa::mint(&subtrees()).unwrap();
    let out = tempfile::tempdir().unwrap();
    ca.write_files(out.path()).unwrap();
    let run_ca = certificates(&std::fs::read(out.path().join(RUN_CA_FILE)).unwrap());
    assert_eq!(run_ca, [ca.cert_der().to_vec(), other.cert_der().to_vec()]);
    let bundle = certificates(&std::fs::read(out.path().join(BUNDLE_FILE)).unwrap());
    assert_eq!(bundle[0], ca.cert_der().to_vec());
    assert_eq!(bundle.iter().filter(|d| **d == other.cert_der().to_vec()).count(), 1);
    assert!(bundle.len() >= 22);
}
// proxy_tls.rs
#[tokio::test] async fn an_upstream_signed_by_the_participants_ca_is_decrypted() {
    let up = Upstream::start(Script::ok(b"hi").with_tls(&["api.openai.com"], &[HTTP11]));
    let dir = tempfile::tempdir().unwrap();
    let roots = up.pki().roots_file(dir.path()).display().to_string();
    let env = [("GOALCUP_TEST_UPSTREAM", format!("api.openai.com={}", up.addr())), ("SSL_CERT_FILE", roots)];
    let (_h, addr, calls, ca) = proxy(HOST_FILE, &env);
    let c = TlsConnector::from(client_trusting(vec![ca.cert_der().clone()], &[b"http/1.1"]));
    let mut t = c.connect(name("api.openai.com"), through(addr, TARGET).await).await.unwrap();
    get(&mut t).await; drop(t);
    assert_eq!(conns(&calls, 1).await[0].1["mode"], "decrypt");
}
```
- [ ] Step 2: Run, expect FAIL (`run_ca` has one certificate; the second test sees `upstream_untrusted`): `cargo nextest run -p goalcup --test lessons_tls the_ca_files_carry_the_participants_ca` and `cargo nextest run -p goalcup --test proxy_tls an_upstream_signed_by_the_participants_ca_is_decrypted`
- [ ] Step 3: Implement. `write_files`: `run-ca.pem` = run CA then `own()`; `bundle.pem` = run CA then `bundle()`; still `NoSystemRoots` when the OS part is empty (`Egress` exposes it through `bundle().len() > own().len()`; keep the existing refusal). `Tls::new`: `let mut roots = net::Egress::load().roots(); roots.add_parsable_certificates(extra_roots);`. Delete `os_roots()`. Update the `ca.rs` and `tls.rs` module docs ("OS store" becomes "the egress roots").
- [ ] Step 4: Run, expect PASS: `cargo nextest run -p goalcup --test lessons_tls --test proxy_tls --test proxy_restart --test watchdog_restart --test watchdog_recover`
- [ ] Step 5: Commit: `git add crates/goalcup/src/proxy/ca.rs crates/goalcup/src/proxy/tls.rs crates/goalcup/tests/lessons_tls.rs crates/goalcup/tests/proxy_tls.rs; git commit -m "proxy: corp roots and the participant's CA files in upstream trust and the CA files (cr-proxy-1)"`

### Task B.3: the upstream leg through the participant's proxy   (M)
**Files:** Modify `src/proxy/connect.rs` 234-276 (`connect_upstream`), `src/proxy/mod.rs` 91-101 (`Boot.egress: &'static Egress`), 112-122, 246, 458. Create `tests/proxy_upstream.rs` (`mod common; mod proxy_fixtures;`).
**Interfaces:** `pub async fn connect_upstream(rec: &Recorder, name: &str, port: u16, listed: bool, test_override: Option<SocketAddr>, via: Option<&net::Upstream>) -> io::Result<TcpStream>`; `pub const PROXY_ANSWER_DEADLINE: Duration = HEAD_DEADLINE;` (30 s).
- [ ] Step 1: Write the failing tests. Helper `fake_proxy(answer: Option<&'static [u8]>, to: Option<SocketAddr>) -> (SocketAddr, Arc<Mutex<Vec<String>>>)`: a thread that accepts, reads to `\r\n\r\n`, stores the head, writes `answer` (or `HTTP/1.1 200 Connection established\r\n\r\n` and pipes both ways to `to`).
```rust
#[tokio::test] async fn model_calls_go_through_the_participants_proxy() {
    let up = Upstream::start(Script::ok(b"hi").with_tls(&["api.openai.com"], &[HTTP11]));
    let (fake, heads) = fake_proxy(None, Some(up.addr()));
    let dir = tempfile::tempdir().unwrap();
    let env = [("HTTPS_PROXY", format!("http://u:s3cret@{fake}")),
               ("GOALCUP_TEST_EXTRA_ROOTS", up.pki().roots_file(dir.path()).display().to_string())];
    let (_h, addr, calls, ca) = proxy(HOSTS, &env);
    let c = TlsConnector::from(client_trusting(vec![ca.cert_der().clone()], &[b"http/1.1"]));
    let mut t = c.connect(name("api.openai.com"), through(addr, "api.openai.com:443").await).await.unwrap();
    assert_eq!(get(&mut t).await, b"hi"); drop(t);
    assert_eq!(*heads.lock().unwrap(), ["CONNECT api.openai.com:443 HTTP/1.1\r\nHost: api.openai.com:443\r\nProxy-Authorization: Basic dTpzM2NyZXQ=\r\n\r\n"]);
    assert_eq!(conns(&calls, 1).await[0].1["mode"], "decrypt");
}
#[tokio::test] async fn a_proxy_that_refuses_is_a_502_with_one_note() {
    let (fake, _) = fake_proxy(Some(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n"), None);
    let (_h, addr, calls, _) = proxy(HOSTS, &[("HTTPS_PROXY", format!("http://{fake}"))]);
    assert_eq!(status(&mut connect_head(addr, "api.openai.com:443").await).await, 502);
    let notes = events(&calls, "connect_refused");
    assert_eq!((notes.len(), &notes[0]["status"], &notes[0]["host"]), (1, &json!(502), &json!("api.openai.com")));
}
#[tokio::test] async fn a_test_upstream_never_goes_through_the_proxy() {
    let up = Upstream::start(Script::ok(b"hi").with_tls(&["api.openai.com"], &[HTTP11]));
    let (closed_guard, closed) = common::ports::refusing();
    let dir = tempfile::tempdir().unwrap();
    let env = [("HTTPS_PROXY", format!("http://{closed}")),
               ("GOALCUP_TEST_UPSTREAM", format!("api.openai.com={}", up.addr())),
               ("GOALCUP_TEST_EXTRA_ROOTS", up.pki().roots_file(dir.path()).display().to_string())];
    let (_h, addr, calls, ca) = proxy(HOSTS, &env);
    // same GET as the first test; then:
    assert_eq!(conns(&calls, 1).await[0].1["mode"], "decrypt");
    drop(closed_guard);
}
```
(`status`, `connect_head`, `events`: reuse the helpers of `proxy_connect.rs`/`proxy_fixtures` where they exist; add them to this file otherwise.)
- [ ] Step 2: Run, expect FAIL (the proxy dials `api.openai.com` itself, the fake sees no head): `cargo nextest run -p goalcup --test proxy_upstream`
- [ ] Step 3: Implement. In each round: `test_override` set → that address, direct; else `via` set → `connect_addresses(lookup(via.host, via.port))`, write `via.connect_head(name, port)`, read the answer one byte at a time up to `HEAD_GUARD` bytes within `PROXY_ANSWER_DEADLINE` (no tunnel byte consumed), any `HTTP/1.x 2xx` → the stream, else `Err`; else direct as today. Retry rule unchanged. Callers pass `target.is_none().then(|| b.egress.proxy_for(host)).flatten()`. `serve` sets `egress: net::Egress::load()`.
- [ ] Step 4: Run, expect PASS: `cargo nextest run -p goalcup --test proxy_upstream --test proxy_connect --test proxy_tls`
- [ ] Step 5: Commit: `git add crates/goalcup/src/proxy/connect.rs crates/goalcup/src/proxy/mod.rs crates/goalcup/tests/proxy_upstream.rs; git commit -m "proxy: CONNECT upstream through the participant's proxy (cr-proxy-2)"`

### Task B.4: ServerClient through Egress, PUT time scaled to the chunk   (M)
**Files:** Modify `src/upload/client.rs` 1-22 (consts, module doc), 190-244 (struct, `new`), 289-323 (`put_chunk`). Test `tests/net_egress.rs`.
**Interfaces:** `pub fn with_egress(base: &str, token: Option<Token>, egress: &Egress) -> ServerClient`; `new` = `with_egress(base, token, Egress::load())`; `pub fn put_budget(content_length: u64) -> Duration`; `const MIN_UPLINK: u64 = 8 * 1024;` (bytes/s).
- [ ] Step 1: Write the failing tests:
```rust
#[test] fn the_token_reaches_a_proxy_only_inside_connect() {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let fake = l.local_addr().unwrap();
    let seen = std::thread::spawn(move || {   // reads the head, refuses, returns what it read
        let (mut s, _) = l.accept().unwrap();
        let head = read_to_blank_line(&mut s);
        s.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n").unwrap();
        head
    });
    let e = Egress::from_vars(&vars(&[("HTTPS_PROXY", &format!("http://u:s3cret@{fake}"))]), vec![]);
    let c = ServerClient::with_egress("https://ingest.example", Some(Token::new("gcr_secret")), &e);
    let err = c.json::<serde_json::Value, ()>("GET", "/v1/health", None).unwrap_err();
    let head = seen.join().unwrap();
    assert!(head.starts_with("CONNECT ingest.example:443 HTTP/1.1\r\n"), "{head}");
    assert!(head.contains("Proxy-Authorization: basic dTpzM2NyZXQ="), "{head}");
    assert!(!head.contains("gcr_secret") && !head.contains("Bearer"));
    assert!(matches!(&err, ServerError::Transport(t) if !t.contains("s3cret")), "{err}");
}
#[test] fn an_http_server_is_never_proxied() {
    let (server, addr) = answer_once(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}");
    let e = Egress::from_vars(&vars(&[("HTTPS_PROXY", "http://127.0.0.1:1"), ("HTTP_PROXY", "http://127.0.0.1:1")]), vec![]);
    let c = ServerClient::with_egress(&format!("http://{addr}"), None, &e);
    assert_eq!(c.json::<serde_json::Value, ()>("GET", "/v1/health", None).unwrap(), json!({}));
    server.join().unwrap();
}
#[test] fn a_chunk_gets_time_for_a_slow_uplink() {
    assert_eq!(put_budget(0), Duration::from_secs(600));
    assert_eq!(put_budget(1 << 20), Duration::from_secs(600));
    assert_eq!(put_budget(32 << 20), Duration::from_secs(4096));
}
```
- [ ] Step 2: Run, expect FAIL (no `with_egress`/`put_budget`): `cargo nextest run -p goalcup --test net_egress`
- [ ] Step 3: Implement. `ServerClient { base, token, tls: Option<Arc<ClientConfig>>, proxy: Option<ureq::Proxy> }`; roots `egress.roots()`; proxy only when `base` starts with `https://` and `egress.proxy_for(host)` is some (`Upstream::ureq()`); `fn agent(&self, whole: Duration) -> ureq::Agent` built per call with today's settings (`redirects(0)`, idle 0, `try_proxy_from_env(false)`, the timeouts), `.proxy(..)` when set. `json` uses `agent(CALL_TIMEOUT)`, `put_chunk` uses `agent(put_budget(content_length))`. `put_budget = max(PUT_TIMEOUT, content_length / MIN_UPLINK s)`. Replace the comment at 227 with "The token goes to a proxy only inside a CONNECT tunnel, and only the participant's (net::Egress)". No chunk shrinking (chunk.rs is not this lane's; at 8 KiB/s a 32 MiB chunk now fits).
- [ ] Step 4: Run, expect PASS: `cargo nextest run -p goalcup --test net_egress --test cli_approve --test upload_live --test upload_worker --test upload_manual`
- [ ] Step 5: Commit: `git add crates/goalcup/src/upload/client.rs crates/goalcup/tests/net_egress.rs; git commit -m "upload: the participant's proxy and roots for the server client, PUT time by chunk size (cr-upload-server-3, -9)"`

### Task B.5: refusal learning that does not re-fail every 10 minutes   (S, PARALLEL-OK)
**Files:** Modify `src/proxy/hosts.rs` 1-19 (doc, consts), 30-35 (`Tables`), 57-104; `src/proxy/mod.rs` 356-363. Test `tests/proxy_tls.rs` 164-183 (rewrite) and new tests.
**Interfaces:** `pub const SILENT_STRIKES: u32 = 3;` `pub fn learn_silent(&self, rec: &Recorder, now: Mono, host: &str, fp: &[u8; 32]) -> bool`. `learn` keeps its signature.
- [ ] Step 1: Write the failing tests (replace `a_learned_refusal_expires_after_ten_minutes`):
```rust
#[test] fn a_second_refusal_holds_for_the_boot() {
    let (rec, calls) = Recorder::for_tests(1 << 30);
    let (hosts, fp, t) = (hosts(), [3u8; 32], Mono(42_000_000_000));
    let expiry = REFUSAL_EXPIRY.as_nanos() as u64;
    assert!(hosts.learn(&rec, t, "api.openai.com", &fp));
    assert!(hosts.refused(Mono(t.ns() + expiry - 1), "api.openai.com", &fp));
    let later = Mono(t.ns() + expiry);
    assert!(!hosts.refused(later, "api.openai.com", &fp));
    assert!(hosts.learn(&rec, later, "api.openai.com", &fp));
    assert!(hosts.refused(Mono(later.ns() + 365 * 86_400 * 1_000_000_000), "api.openai.com", &fp));
    assert_eq!(ca_refused(&calls).len(), 2);
}
#[test] fn three_silent_closes_teach_like_an_alert() {
    let (rec, calls) = Recorder::for_tests(1 << 30);
    let (hosts, fp, t) = (hosts(), [4u8; 32], Mono(1));
    assert!(!hosts.learn_silent(&rec, t, "api.openai.com", &fp));
    assert!(!hosts.learn_silent(&rec, t, "api.openai.com", &fp));
    assert!(!hosts.refused(t, "api.openai.com", &fp));
    assert!(hosts.learn_silent(&rec, t, "api.openai.com", &fp));
    assert!(hosts.refused(t, "api.openai.com", &fp));
    assert_eq!(ca_refused(&calls).len(), 1);
    let other = [5u8; 32];
    hosts.learn_silent(&rec, t, "api.openai.com", &other);
    hosts.trust("api.openai.com", &other);
    assert!(!hosts.learn_silent(&rec, t, "api.openai.com", &other));
    assert!(!hosts.learn_silent(&rec, t, "api.openai.com", &other));
}
#[tokio::test] async fn silent_closes_after_our_leaf_are_learned() {
    // proxy_to(&up, true); three times: CONNECT, write a ClientHello (as in
    // a_close_mid_handshake_teaches_nothing), read until at least 1 byte of our flight, drop.
    // Then: ca_refused(&calls).len() == 1 and a fourth connection's conn has reason "tls_refused".
}
```
- [ ] Step 2: Run, expect FAIL (`learn_silent` missing; the year-later `refused` is false): `cargo nextest run -p goalcup --test proxy_tls`
- [ ] Step 3: Implement. `Tables.refused: HashMap<Pair, (Option<Mono>, u32)>` (learned-at, `None` = for the boot; strikes) and `silent: HashMap<Pair, u32>`; `learn`: strikes + 1, second strike → `None`; `learn_silent`: count + 1, at `SILENT_STRIKES` reset and `learn`; `trust` clears both. In `mod.rs` 362, `Refusal::NoAlert` calls `learn_silent` only when `wire[1] > 0` (our flight went out). `a_close_mid_handshake_teaches_nothing` stays green (one close).
- [ ] Step 4: Run, expect PASS: `cargo nextest run -p goalcup --test proxy_tls`
- [ ] Step 5: Commit: `git add crates/goalcup/src/proxy/hosts.rs crates/goalcup/src/proxy/mod.rs crates/goalcup/tests/proxy_tls.rs; git commit -m "proxy: a second refusal holds for the boot, silent refusals are learned (cr-proxy-3)"`

### Task B.6: handshake deadlines on both legs   (S)
**Files:** Modify `src/proxy/tls.rs` (one const), `src/proxy/mod.rs` 189-205 (`test_hooks` returns a `Hooks { upstream, roots, handshake }` struct), 312-316, 353. Test `tests/proxy_tls.rs`.
**Interfaces:** `pub const HANDSHAKE_DEADLINE: Duration = Duration::from_secs(60);` test hook `GOALCUP_TEST_HANDSHAKE_MS` (test-hooks feature only).
- [ ] Step 1: Write the failing tests:
```rust
#[tokio::test] async fn a_stalled_upstream_handshake_becomes_a_tunnel() {
    let (staller, at) = common::ports::held();     // accepts into the backlog, never answers
    let env = [("GOALCUP_TEST_UPSTREAM", format!("api.openai.com={at}")), ("GOALCUP_TEST_HANDSHAKE_MS", "200".into())];
    let (_h, addr, calls, ca) = proxy(HOST_FILE, &env);
    let c = TlsConnector::from(client_trusting(vec![ca.cert_der().clone()], &[b"http/1.1"]));
    let s = through(addr, TARGET).await;
    tokio::spawn(async move { let _ = c.connect(name("api.openai.com"), s).await; });
    let (_, meta, _) = within_async(Duration::from_secs(5), conns(&calls, 1)).await.remove(0);
    assert_eq!(meta["reason"], "upstream_untrusted");
    drop(staller);
}
#[tokio::test] async fn a_stalled_client_handshake_ends_and_teaches_nothing() {
    // proxy_to(&up, true) plus GOALCUP_TEST_HANDSHAKE_MS=200; write only the ClientHello and
    // keep the socket open. Within 5 s: GAP tls_refused with tail {"alert": false}, ca_refused empty.
}
```
- [ ] Step 2: Run, expect FAIL (both hang to the 5 s bound): `cargo nextest run -p goalcup --test proxy_tls stalled`
- [ ] Step 3: Implement: wrap `c.connect(server_name, up)` (mod.rs:314) and `acceptor.accept(..)` (mod.rs:353) in `tokio::time::timeout(hooks.handshake, ..)`. Upstream timeout → `UpstreamFail::Untrusted` path (fresh tunnel, the harness meets the server itself). Client timeout → the `tls_refused` `{"alert": false}` lost tunnel with no `learn`/`learn_silent`.
- [ ] Step 4: Run, expect PASS: `cargo nextest run -p goalcup --test proxy_tls`
- [ ] Step 5: Commit: `git add crates/goalcup/src/proxy/tls.rs crates/goalcup/src/proxy/mod.rs crates/goalcup/tests/proxy_tls.rs; git commit -m "proxy: 60 s handshake deadline on both legs (cr-proxy-4)"`

### Task B.7: null mode relays without a parser   (S)
**Files:** Modify `src/proxy/mod.rs` 54-59 (`ProxyCtx.null: bool`), 91-101, 417-421; shared `src/pipeline/mod.rs` 846-850 (`null,`), `tests/proxy_fixtures/mod.rs` 40-44 (`null: false,`) plus `pub fn serve_null(listener: &TcpListener, hosts: &str, ca: RunCa) -> (ProxyHandle, Calls)`. Test `tests/proxy_tls.rs`.
- [ ] Step 1: Write the failing test:
```rust
#[tokio::test] async fn null_mode_relays_without_parsing() {
    // env GOALCUP_TEST_UPSTREAM + GOALCUP_TEST_EXTRA_ROOTS as proxy_to(&up, true) sets them,
    // then serve_null(&listener, HOST_FILE, ca.clone()); one decrypted GET answers "hi".
    assert!(opens(&calls).iter().all(|(_, kind, _)| *kind == StreamKind::Conn), "a parser ran");
}
```
- [ ] Step 2: Run, expect FAIL (an HTTP stream is opened): `cargo nextest run -p goalcup --test proxy_tls null_mode_relays_without_parsing`
- [ ] Step 3: Implement: `Boot.null` from `ctx.null`; in `decrypt` the parser is `None` when `b.null`.
- [ ] Step 4: Run, expect PASS: `cargo nextest run -p goalcup --test proxy_tls --test watchdog_null --test upload_null`
- [ ] Step 5: Commit: `git add crates/goalcup/src/proxy/mod.rs crates/goalcup/src/pipeline/mod.rs crates/goalcup/tests/proxy_fixtures/mod.rs crates/goalcup/tests/proxy_tls.rs; git commit -m "proxy: no parser in null mode (cr-proxy-5)"`

### Task B.8: tell the participant which proxy goalcup uses   (S)
**Files:** shared `src/cli/start.rs` after 292: `lines.extend(crate::net::Egress::load().line());`; shared `src/cli/setup.rs` 238-243: add `crate::net::Egress::load().line(),` to the `lines` array. Test `tests/cli_setup.rs` (helper `setup_env(home, path, args, envs: &[(&str, &str)])`, a copy of `setup` with extra `.env`).
- [ ] Step 1: Write the failing test:
```rust
#[test] fn setup_names_the_participants_proxy() {
    let _guard = guard();
    let (home, c) = (TempHome::new(), connector());
    let fake = Fake::new(&c.program);
    let (out, all) = setup_env(&home, &fake.path(), &["--harness", &c.id, "--yes"],
        &[("HTTPS_PROXY", "http://u:s3cret@proxy.corp:8080")]);
    assert_eq!(out.status.code(), Some(0), "{all}");
    assert!(all.contains("through your proxy proxy.corp:8080 (HTTPS_PROXY)."), "{all}");
    assert!(!all.contains("s3cret"), "{all}");
}
```
- [ ] Step 2: Run, expect FAIL (no proxy line): `cargo nextest run -p goalcup --test cli_setup setup_names_the_participants_proxy`
- [ ] Step 3: Implement the two lines above.
- [ ] Step 4: Run, expect PASS: `cargo nextest run -p goalcup --test cli_setup --test cli_start`
- [ ] Step 5: Commit: `git add crates/goalcup/src/cli/start.rs crates/goalcup/src/cli/setup.rs crates/goalcup/tests/cli_setup.rs; git commit -m "cli: start and setup name the proxy goalcup uses (point 6)"`

### Task B.9: one real harness behind a TLS-inspecting proxy   (S, PAD)
**Files:** none (evidence only).
- [ ] Step 1: On the owner's Mac, with `mitmdump --listen-port 8899` running (owner question 3), in a practice folder: `HTTPS_PROXY=http://127.0.0.1:8899 SSL_CERT_FILE=$HOME/.mitmproxy/mitmproxy-ca-cert.pem NODE_EXTRA_CA_CERTS=$HOME/.mitmproxy/mitmproxy-ca-cert.pem goalcup start --practice` with claude-code, one prompt, `goalcup finish`.
- [ ] Step 2: Expect: start prints the proxy line; the mitmdump log shows `CONNECT api.anthropic.com:443` from the recorder; `goalcup-verify` on the entry shows decrypted `conn`s with requests (not `upstream_untrusted` tunnels) and native rows; the upload reached the server. Record native+wire counts in the lane PR, not just exit codes.
- [ ] Step 3: Commit: none (evidence goes in the PR body).

## Owner questions
1. macOS system proxy settings (System Settings, PAC) are not read; only the env variables people export for their harnesses. Enough for the event?
2. Login-keychain roots a user trusted by hand (not admin/MDM) are not added; only `/Library/Keychains/System.keychain` with admin trust. Add the user domain too?
3. B.9 needs mitmproxy on the Mac (`brew install mitmproxy`). OK to install, or test with a plain CONNECT proxy and only cover cr-proxy-2 live?
4. cr-proxy-x1: docs disclosure only (docs lane), or plan a masker task?
