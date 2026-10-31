//! PRD-03 acceptance cases for the run CA and the TLS configs (section 4). PRD-17's
//! `lessons_tls` holds the AKI, constraint, file-mode and bundle cases; they are not repeated here.

use goalcup::connector::{Decision, HostList, Kind, Source};
use goalcup::pipeline::{Call, Calls, Mono, Recorder};
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
