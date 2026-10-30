//! PRD-03 acceptance cases for the run CA and the TLS configs (section 4). PRD-17's
//! `lessons_tls` holds the AKI, constraint, file-mode and bundle cases; they are not repeated here.

use goalcup::proxy::ca::{RUN_CA_FILE, RunCa, certificates};
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
