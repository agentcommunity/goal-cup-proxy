//! PRD-03 acceptance cases for the run CA and the TLS configs (section 4). PRD-17's
//! `lessons_tls` holds the AKI, constraint, file-mode and bundle cases; they are not repeated here.
//! The last cases drive the real `proxy::serve` against the fake upstream.

mod common;

use common::pki::{H2, HTTP11};
use common::recorder::{ends, events, gaps, opens};
use common::upstream::{Script, Upstream};
use common::{HANG, within_async};
use goalcup::connector::{Decision, HostList, Kind, Source};
use goalcup::pipeline::{Call, Calls, GapReason, Mono, Recorder, StreamKind};
use goalcup::proxy::{ProxyCtx, ProxyHandle, serve};
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use goalcup::proxy::ca::{RUN_CA_FILE, RunCa, certificates};
use goalcup::proxy::connect::fingerprint_hex16;
use goalcup::proxy::hosts::{Hosts, REFUSAL_EXPIRY};
use goalcup::proxy::tls::{Offer, Refusal, Tls, classify_client_handshake, upstream_offer};
use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{ClientConfig, ClientConnection, RootCertStore};
use std::sync::Arc;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, duplex};
use tokio_rustls::{TlsAcceptor, TlsConnector};

fn client_trusting(roots: Vec<CertificateDer<'static>>, alpn: &[&[u8]]) -> Arc<ClientConfig> {
    let mut store = RootCertStore::empty();
    for root in roots {
        store.add(root).unwrap();
    }
    client_with(store, alpn)
}

fn client_with(store: RootCertStore, alpn: &[&[u8]]) -> Arc<ClientConfig> {
    let mut config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(store)
            .with_no_client_auth();
    config.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
    Arc::new(config)
}

fn name(host: &'static str) -> ServerName<'static> {
    ServerName::try_from(host).unwrap()
}

/// Acceptance 8 guard: a CA exported and imported in a fresh boot signs leaves the first boot's
/// `run-ca.pem` trusts, and HTTP/1.1 is what both sides speak.
#[tokio::test]
async fn an_imported_ca_signs_leaves_the_first_boots_run_ca_trusts() {
    let dir = tempfile::tempdir().unwrap();
    let pem = {
        let first = RunCa::mint(&["openai.com".to_string()]).unwrap();
        first.write_files(dir.path()).unwrap();
        first.export()
    };
    let run_ca = std::fs::read(dir.path().join(RUN_CA_FILE)).unwrap();
    let roots = certificates(&run_ca)
        .into_iter()
        .map(CertificateDer::from)
        .collect();

    let second = RunCa::import(&pem).unwrap();
    let tls = Tls::new(&second, Vec::new());
    let acceptor = TlsAcceptor::from(tls.server(Some(b"http/1.1")));
    let connector = TlsConnector::from(client_trusting(roots, &[b"h2", b"http/1.1"]));

    let (client_io, server_io) = duplex(64 * 1024);
    let server = tokio::spawn(async move {
        let mut stream = acceptor.accept(server_io).await.unwrap();
        let alpn = stream.get_ref().1.alpn_protocol().map(<[u8]>::to_vec);
        let mut buf = [0u8; 4];
        stream.read_exact(&mut buf).await.unwrap();
        stream.write_all(b"pong").await.unwrap();
        stream.flush().await.unwrap();
        (alpn, buf)
    });
    let mut stream = connector
        .connect(name("api.openai.com"), client_io)
        .await
        .unwrap();
    assert_eq!(stream.get_ref().1.alpn_protocol(), Some(&b"http/1.1"[..]));
    stream.write_all(b"ping").await.unwrap();
    stream.flush().await.unwrap();
    let mut buf = [0u8; 4];
    stream.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"pong");
    let (alpn, got) = server.await.unwrap();
    assert_eq!(alpn.as_deref(), Some(&b"http/1.1"[..]));
    assert_eq!(&got, b"ping");
}

/// Section 2: a ClientHello without SNI gets the leaf of the CONNECT host, and the client that
/// trusts the run CA accepts it.
#[tokio::test]
async fn no_sni_gets_the_connect_hosts_leaf() {
    let ca = RunCa::mint(&["openai.com".to_string()]).unwrap();
    let tls = Tls::new(&ca, Vec::new());
    let config = tls.server_for(Some(b"http/1.1"), "api.openai.com").unwrap();
    let acceptor = TlsAcceptor::from(config);
    let mut client = (*client_trusting(vec![ca.cert_der().clone()], &[b"http/1.1"])).clone();
    client.enable_sni = false;
    let connector = TlsConnector::from(Arc::new(client));
    let (client_io, server_io) = duplex(64 * 1024);
    let (server, client) = tokio::join!(
        acceptor.accept(server_io),
        connector.connect(name("api.openai.com"), client_io)
    );
    let server = server.expect("the server handshake");
    assert_eq!(server.get_ref().1.server_name(), None, "no SNI was sent");
    let client = client.expect("the client trusts the CONNECT host's leaf");
    assert_eq!(client.get_ref().1.alpn_protocol(), Some(&b"http/1.1"[..]));
}

/// Section 4 point 4, `PreferH1`: what upstream is offered for each client offer.
#[test]
fn the_alpn_offer_follows_prefer_h1() {
    let ca = RunCa::mint(&["openai.com".to_string()]).unwrap();
    let tls = Tls::new(&ca, Vec::new());
    type Case = (
        &'static [&'static str],
        Offer,
        Option<&'static [&'static str]>,
    );
    let cases: [Case; 5] = [
        (&["h2", "http/1.1"], Offer::Http1, Some(&["http/1.1"][..])),
        (&["http/1.1"], Offer::Http1, Some(&["http/1.1"][..])),
        (&["h2"], Offer::H2, Some(&["h2"][..])),
        (&[], Offer::None, Some(&[][..])),
        (&["spdy/3.1", "acme-tls/1"], Offer::Other, None),
    ];
    let bytes = |list: &[&str]| {
        list.iter()
            .map(|p| p.as_bytes().to_vec())
            .collect::<Vec<_>>()
    };
    for (client, offer, upstream) in cases {
        assert_eq!(upstream_offer(&bytes(client)), offer, "{client:?}");
        let offered = tls.client(offer).map(|c| c.alpn_protocols.clone());
        let expected = upstream.map(bytes);
        assert_eq!(offered, expected, "{client:?}");
    }
}

/// A client that trusts only the public roots (as a pinned or dodging client does) refuses our
/// leaf with an alert, the one thing refusal learning may use.
#[tokio::test]
async fn an_untrusting_client_refuses_with_an_alert() {
    let ca = RunCa::mint(&["openai.com".to_string()]).unwrap();
    let tls = Tls::new(&ca, Vec::new());
    let acceptor = TlsAcceptor::from(tls.server(Some(b"http/1.1")));
    let mut public = RootCertStore::empty();
    public.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let connector = TlsConnector::from(client_with(public, &[b"http/1.1"]));
    let (client_io, server_io) = duplex(64 * 1024);
    let (server, client) = tokio::join!(
        acceptor.accept(server_io),
        connector.connect(name("api.openai.com"), client_io)
    );
    assert!(client.is_err());
    let Err(err) = server else {
        panic!("the server handshake succeeded")
    };
    assert_eq!(
        classify_client_handshake(&err),
        Refusal::Alert("unknown_ca"),
        "{err:?}"
    );
}

/// A client that goes away mid-handshake sent no alert: nothing to learn from.
#[tokio::test]
async fn a_close_mid_handshake_is_no_alert() {
    let ca = RunCa::mint(&["openai.com".to_string()]).unwrap();
    let tls = Tls::new(&ca, Vec::new());
    let acceptor = TlsAcceptor::from(tls.server(Some(b"http/1.1")));
    let mut hello = Vec::new();
    ClientConnection::new(
        client_trusting(vec![ca.cert_der().clone()], &[b"http/1.1"]),
        name("api.openai.com"),
    )
    .unwrap()
    .write_tls(&mut hello)
    .unwrap();
    let (mut client_io, server_io) = duplex(64 * 1024);
    client_io.write_all(&hello).await.unwrap();
    drop(client_io);
    let Err(err) = acceptor.accept(server_io).await else {
        panic!("the server handshake succeeded")
    };
    assert_eq!(classify_client_handshake(&err), Refusal::NoAlert, "{err:?}");
}

const HOST_FILE: &str = r#"
provider = ["api.openai.com", "*.openai.azure.com"]
never = ["login.openai.azure.com"]
"#;

fn hosts() -> Hosts {
    Hosts::new(HostList::from_file(HOST_FILE.as_bytes(), "hosts.toml").unwrap())
}

/// The `ca_refused` notes written so far.
fn ca_refused(calls: &Calls) -> Vec<serde_json::Value> {
    (calls.all().into_iter())
        .filter_map(|c| match c {
            Call::Event(ev) if ev["what"] == "ca_refused" => Some(ev),
            _ => None,
        })
        .collect()
}

/// Section 2: the SNI decides, else the CONNECT host.
#[test]
fn the_sni_is_the_deciding_name() {
    let hosts = hosts();
    let listed = Decision::Listed {
        source: Source::Shared,
        kind: Kind::Provider,
        pinned: false,
    };
    assert_eq!(
        hosts.decide(Some("api.openai.com"), "127.0.0.1"),
        ("api.openai.com", listed)
    );
    assert_eq!(
        hosts.decide(None, "api.openai.com"),
        ("api.openai.com", listed)
    );
    assert_eq!(
        hosts.decide(Some("example.com"), "api.openai.com"),
        ("example.com", Decision::NotListed)
    );
}

/// Acceptance 7: an alert teaches the pair and writes one note; a second fingerprint of the same
/// host and a pair never learned still get our leaf.
#[test]
fn an_alert_teaches_one_pair_and_writes_one_note() {
    let (rec, calls) = Recorder::for_tests(1 << 30);
    let hosts = hosts();
    let (fp, other) = ([7u8; 32], [8u8; 32]);
    let t = Mono(5_000_000_000);
    assert!(!hosts.refused(t, "api.openai.com", &fp), "never learned");
    assert!(hosts.learn(&rec, t, "api.openai.com", &fp));
    assert!(
        hosts.learn(&rec, t, "api.openai.com", &fp),
        "a second handshake in flight"
    );
    assert!(hosts.refused(t, "api.openai.com", &fp));
    assert!(!hosts.refused(t, "api.openai.com", &other));
    assert!(!hosts.refused(t, "chatgpt.com", &fp));
    let notes = ca_refused(&calls);
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0]["ev"], "note");
    assert_eq!(notes[0]["host"], "api.openai.com");
    assert_eq!(notes[0]["fingerprint"], fingerprint_hex16(&fp));
}

/// Acceptance 7: a pair that completed a handshake with our leaf in this boot is never learned.
#[test]
fn a_trusted_pair_is_never_learned() {
    let (rec, calls) = Recorder::for_tests(1 << 30);
    let hosts = hosts();
    let fp = [9u8; 32];
    let t = Mono(1);
    hosts.trust("api.openai.com", &fp);
    assert!(!hosts.learn(&rec, t, "api.openai.com", &fp));
    assert!(!hosts.refused(t, "api.openai.com", &fp));
    assert!(calls.all().is_empty(), "no note for a trusted pair");
}

/// Acceptance 7: a learned refusal is gone 10 minutes later on the test clock, and learning the
/// pair again writes a second note.
#[test]
fn a_learned_refusal_expires_after_ten_minutes() {
    let (rec, calls) = Recorder::for_tests(1 << 30);
    let hosts = hosts();
    let fp = [3u8; 32];
    let t = Mono(42_000_000_000);
    let expiry = REFUSAL_EXPIRY.as_nanos() as u64;
    assert!(hosts.learn(&rec, t, "api.openai.com", &fp));
    assert!(hosts.refused(Mono(t.ns() + expiry - 1), "api.openai.com", &fp));
    let later = Mono(t.ns() + expiry);
    assert!(!hosts.refused(later, "api.openai.com", &fp));
    assert!(hosts.learn(&rec, later, "api.openai.com", &fp));
    assert!(hosts.refused(later, "api.openai.com", &fp));
    assert_eq!(ca_refused(&calls).len(), 2);
}

/// Rule 8 over real TLS: the upstream ends a close-delimited reply without close_notify. The
/// relay passes no clean close on; its caller drops the harness leg, and the harness reads the
/// bytes that came, then `UnexpectedEof`, as it would without us.
#[tokio::test]
async fn an_upstream_tls_cut_reaches_the_harness_as_a_cut() {
    use bytes::Bytes;
    use goalcup::pipeline::{OpenFlags, StreamKind};
    use goalcup::proxy::pump;
    use std::sync::atomic::AtomicU64;

    let ca = RunCa::mint(&["openai.com".to_string()]).unwrap();
    let tls = Tls::new(&ca, Vec::new());
    let trusting = || TlsConnector::from(client_trusting(vec![ca.cert_der().clone()], &[]));
    let reply = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\ndata: 1\n\n";
    let ((harness_io, down_io), (up_io, server_io)) = (duplex(1 << 16), duplex(1 << 16));
    let acceptor = TlsAcceptor::from(tls.server(None));
    let server = tokio::spawn({
        let acceptor = acceptor.clone();
        async move {
            let mut s = acceptor.accept(server_io).await.unwrap();
            let mut head = [0u8; 4];
            s.read_exact(&mut head).await.unwrap();
            s.write_all(reply).await.unwrap();
            s.flush().await.unwrap();
            // Gone without close_notify.
            drop(s.into_inner().0);
        }
    });
    let (rec, _calls) = Recorder::for_tests(1 << 30);
    let up = trusting();
    let relay = tokio::spawn(async move {
        let client = acceptor.accept(down_io).await.unwrap();
        let upstream = up.connect(name("api.openai.com"), up_io).await.unwrap();
        let (kind, flags) = (StreamKind::Conn, OpenFlags::NONE);
        let mut conn = rec.open(kind, 0, 0, 0, flags, serde_json::json!({}));
        let (legs, none) = ((client, upstream), (Bytes::new(), Bytes::new()));
        let quiet = AtomicU64::new(0);
        let p = pump::relay(&rec, &mut conn, legs, none, None, &quiet, None).await;
        conn.end(rec.now(), p.reason, None);
        assert!(!p.upstream_failed, "a cut is not a reset");
        drop(p.client);
    });
    let mut harness = trusting()
        .connect(name("api.openai.com"), harness_io)
        .await
        .unwrap();
    harness.write_all(b"GET ").await.unwrap();
    harness.flush().await.unwrap();
    let mut got = Vec::new();
    let err = harness.read_to_end(&mut got).await.unwrap_err();
    assert_eq!(got, reply, "verbatim");
    let eof = std::io::ErrorKind::UnexpectedEof;
    assert_eq!(err.kind(), eof, "no clean close");
    server.await.unwrap();
    relay.await.unwrap();
}

/// The real `serve` with `api.openai.com` sent to `up`, trusting its test CA when `trust`.
fn proxy(up: &Upstream, trust: bool) -> (ProxyHandle, SocketAddr, Calls, RunCa, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: nextest runs each test in its own process; `serve` reads these once, below.
    unsafe {
        std::env::set_var("GOALCUP_TEST_UPSTREAM", format!("api.openai.com={}", up.addr()));
        if trust {
            std::env::set_var("GOALCUP_TEST_EXTRA_ROOTS", up.pki().roots_file(dir.path()));
        }
    }
    let (listener, addr) = common::ports::held();
    let (recorder, calls) = Recorder::for_tests(1 << 30);
    let ca = RunCa::mint(&["openai.com".to_string()]).unwrap();
    let hosts = HostList::from_file(HOST_FILE.as_bytes(), "hosts.toml").unwrap();
    let ctx = ProxyCtx {
        recorder,
        hosts,
        ca: ca.clone(),
    };
    (serve(listener, ctx), addr, calls, ca, dir)
}

/// A CONNECT to `api.openai.com:443` answered `200`.
async fn through(proxy: SocketAddr) -> TcpStream {
    let mut s = TcpStream::connect(proxy).await.unwrap();
    s.write_all(b"CONNECT api.openai.com:443 HTTP/1.1\r\nHost: api.openai.com\r\n\r\n")
        .await
        .unwrap();
    let mut head = [0u8; 39];
    s.read_exact(&mut head).await.unwrap();
    assert_eq!(&head, b"HTTP/1.1 200 Connection established\r\n\r\n");
    s
}

/// One request; reads until the fake upstream's `hi`.
async fn get<S: AsyncRead + AsyncWrite + Unpin>(t: &mut S) {
    t.write_all(b"GET /v1/models HTTP/1.1\r\nHost: api.openai.com\r\n\r\n")
        .await
        .unwrap();
    let mut got = Vec::new();
    while !got.ends_with(b"hi") {
        let mut b = [0u8; 256];
        let n = t.read(&mut b).await.unwrap();
        assert!(n > 0, "the response ended early");
        got.extend_from_slice(&b[..n]);
    }
}

/// Waits for `n` conn ENDs; gives (sid, OPEN meta) per ended conn in open order.
async fn conns(calls: &Calls, n: usize) -> Vec<(u64, Value)> {
    within_async(HANG, async {
        loop {
            let ended: Vec<_> = ends(calls).into_iter().map(|e| e.0).collect();
            let got: Vec<_> = (opens(calls).into_iter())
                .filter(|o| o.1 == StreamKind::Conn && ended.contains(&o.0))
                .map(|o| (o.0, o.2))
                .collect();
            if got.len() >= n {
                return got;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
}

fn gaps_of(calls: &Calls, sid: u64) -> Vec<(GapReason, Option<Value>)> {
    (gaps(calls).into_iter())
        .filter_map(|g| match g {
            Call::Gap(s, reason, .., tail) if s == sid => Some((reason, tail)),
            _ => None,
        })
        .collect()
}

/// GAP 13: an upstream certificate our roots refuse is a tunnel, and the harness meets the
/// upstream's own certificate (trust is never lowered).
#[tokio::test]
async fn an_untrusted_upstream_is_a_tunnel_to_its_own_certificate() {
    let up = Upstream::start(Script::ok(b"hi").with_tls(&["api.openai.com"], &[HTTP11]));
    let (_h, addr, calls, _, _dir) = proxy(&up, false);
    let own = TlsConnector::from(up.pki().client_config(&[HTTP11]));
    let mut t = own.connect(name("api.openai.com"), through(addr).await).await.unwrap();
    get(&mut t).await;
    drop(t);
    let (sid, meta) = conns(&calls, 1).await.remove(0);
    assert_eq!((&meta["mode"], &meta["reason"]), (&json!("tunnel"), &json!("upstream_untrusted")));
    assert_eq!(gaps_of(&calls, sid), [(GapReason::UpstreamUntrusted, None)]);
}

/// GAP 14: an upstream that rejects the offered ALPN is an `alpn_other` tunnel.
#[tokio::test]
async fn an_upstream_rejecting_the_alpn_is_an_alpn_other_tunnel() {
    let up = Upstream::start(Script::ok(b"hi").with_tls(&["api.openai.com"], &[H2]));
    let (_h, addr, calls, _, _dir) = proxy(&up, true);
    let own = TlsConnector::from(up.pki().client_config(&[HTTP11]));
    let _ = own.connect(name("api.openai.com"), through(addr).await).await;
    let (sid, meta) = conns(&calls, 1).await.remove(0);
    assert_eq!(meta["reason"], "alpn_other");
    assert_eq!(gaps_of(&calls, sid), [(GapReason::ProtocolOther, None)]);
}

/// Refusal learning through `serve`: an `unknown_ca` alert writes one note and a `tls_refused`
/// GAP naming it; the same fingerprint then tunnels to the real upstream; another fingerprint
/// is still decrypted.
#[tokio::test]
async fn an_alert_teaches_its_fingerprint_and_no_other() {
    let up = Upstream::start(Script::ok(b"hi").with_tls(&["api.openai.com"], &[HTTP11]));
    let (_h, addr, calls, ca, _dir) = proxy(&up, true);
    let refusing = TlsConnector::from(up.pki().client_config(&[HTTP11]));
    let first = refusing.connect(name("api.openai.com"), through(addr).await).await;
    assert!(first.is_err(), "our leaf is refused");
    let (sid, meta) = conns(&calls, 1).await.remove(0);
    assert_eq!((&meta["mode"], &meta["reason"]), (&json!("tunnel"), &json!("tls_refused")));
    let alert = Some(json!({"alert": "unknown_ca"}));
    assert_eq!(gaps_of(&calls, sid), [(GapReason::TlsRefused, alert)]);
    assert_eq!(ca_refused(&calls).len(), 1);

    let mut t = refusing.connect(name("api.openai.com"), through(addr).await).await.unwrap();
    get(&mut t).await;
    drop(t);
    assert_eq!(conns(&calls, 2).await[1].1["reason"], "tls_refused");

    let trusting = TlsConnector::from(client_trusting(vec![ca.cert_der().clone()], &[]));
    let mut t = trusting.connect(name("api.openai.com"), through(addr).await).await.unwrap();
    get(&mut t).await;
    drop(t);
    assert_eq!(conns(&calls, 3).await[2].1["mode"], "decrypt");
    assert_eq!(ca_refused(&calls).len(), 1);
}

/// A plain close mid-handshake: GAP `tls_refused` with `{"alert": false}`, nothing learned.
#[tokio::test]
async fn a_close_mid_handshake_teaches_nothing() {
    let up = Upstream::start(Script::ok(b"hi").with_tls(&["api.openai.com"], &[HTTP11]));
    let (_h, addr, calls, ca, _dir) = proxy(&up, true);
    let mut hello = Vec::new();
    let config = client_trusting(vec![ca.cert_der().clone()], &[b"http/1.1"]);
    let mut tls = ClientConnection::new(config, name("api.openai.com")).unwrap();
    tls.write_tls(&mut hello).unwrap();
    let mut s = through(addr).await;
    s.write_all(&hello).await.unwrap();
    drop(s);
    let (sid, _) = conns(&calls, 1).await.remove(0);
    let alert = Some(json!({"alert": false}));
    assert_eq!(gaps_of(&calls, sid), [(GapReason::TlsRefused, alert)]);
    assert!(ca_refused(&calls).is_empty());
}

/// An `h2`-only client until the HTTP/2 parser lands: decrypted with `alpn` `h2`, every byte
/// relayed, and the loss named (5a `h2_unsupported`), never silent.
#[tokio::test]
async fn an_h2_only_client_is_relayed_and_its_loss_named() {
    let up = Upstream::start(Script::ok(b"hi").with_tls(&["api.openai.com"], &[H2]));
    let (_h, addr, calls, ca, _dir) = proxy(&up, true);
    let trusting = TlsConnector::from(client_trusting(vec![ca.cert_der().clone()], &[b"h2"]));
    let t = trusting.connect(name("api.openai.com"), through(addr).await).await.unwrap();
    let (send, conn) = h2::client::handshake(t).await.unwrap();
    let driver = tokio::spawn(conn);
    let mut send = send.ready().await.unwrap();
    let req = http::Request::get("https://api.openai.com/v1/models").body(()).unwrap();
    let mut body = send.send_request(req, true).unwrap().0.await.unwrap().into_body();
    let mut got = Vec::new();
    while let Some(piece) = body.data().await {
        let piece = piece.unwrap();
        let _ = body.flow_control().release_capacity(piece.len());
        got.extend_from_slice(&piece);
    }
    assert_eq!(got, b"hi");
    driver.abort();
    let (sid, meta) = conns(&calls, 1).await.remove(0);
    assert_eq!((&meta["mode"], &meta["alpn"]), (&json!("decrypt"), &json!("h2")));
    assert_eq!(gaps_of(&calls, sid), [(GapReason::H2Unsupported, None)]);
    let stopped = json!({"ev": "note", "what": "parse_stopped", "conn": sid, "reason": "h2_unsupported"});
    assert!(events(&calls).contains(&stopped));
}
