//! PRD-03 acceptance cases for the run CA and the TLS configs (section 4). PRD-17's
//! `lessons_tls` holds the AKI, constraint, file-mode and bundle cases; they are not repeated here.
//! The last cases drive the real `proxy::serve` against the fake upstream.

mod common;
mod proxy_fixtures;

use common::pki::{H2, HTTP11};
use common::recorder::{data_of, events, gaps, opens};
use common::upstream::{Script, Upstream};
use goalcup::connector::{Decision, HostList, Kind, Source};
use goalcup::pipeline::{Call, Calls, GapReason, Mono, Recorder, StreamKind};
use goalcup::proxy::ProxyHandle;
use goalcup::proxy::ca::RunCa;
use goalcup::proxy::connect::fingerprint_hex16;
use goalcup::proxy::hosts::{Hosts, REFUSAL_EXPIRY};
use goalcup::proxy::tls::{Offer, Tls, upstream_offer};
use proxy_fixtures::{client_trusting, conns, get, proxy, through};
use rustls::ClientConnection;
use rustls::pki_types::ServerName;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, duplex};
use tokio_rustls::{TlsAcceptor, TlsConnector};

fn name(host: &'static str) -> ServerName<'static> {
    ServerName::try_from(host).unwrap()
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

/// cr-proxy-3: a learned refusal is gone 10 minutes later on the test clock, so the pair tries our
/// leaf again; a second refusal of the pair holds for the rest of the boot. Each writes a note.
#[test]
fn a_second_refusal_holds_for_the_boot() {
    let (rec, calls) = Recorder::for_tests(1 << 30);
    let (hosts, fp, t) = (hosts(), [3u8; 32], Mono(42_000_000_000));
    let expiry = REFUSAL_EXPIRY.as_nanos() as u64;
    assert!(hosts.learn(&rec, t, "api.openai.com", &fp));
    assert!(hosts.refused(Mono(t.ns() + expiry - 1), "api.openai.com", &fp));
    let later = Mono(t.ns() + expiry);
    assert!(!hosts.refused(later, "api.openai.com", &fp));
    assert!(hosts.learn(&rec, later, "api.openai.com", &fp));
    let year = 365 * 86_400 * 1_000_000_000;
    assert!(hosts.refused(Mono(later.ns() + year), "api.openai.com", &fp));
    assert_eq!(ca_refused(&calls).len(), 2);
}

/// cr-proxy-3: a client that refuses our leaf by closing without an alert teaches the pair after
/// three closes (`SILENT_STRIKES`), with one note; a trusted handshake clears the count.
#[test]
fn three_silent_closes_teach_like_an_alert() {
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
fn proxy_to(up: &Upstream, trust: bool) -> (ProxyHandle, SocketAddr, Calls, RunCa, TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut env = vec![(
        "GOALCUP_TEST_UPSTREAM",
        format!("api.openai.com={}", up.addr()),
    )];
    if trust {
        let roots = up.pki().roots_file(dir.path()).display().to_string();
        env.push(("GOALCUP_TEST_EXTRA_ROOTS", roots));
    }
    let (h, addr, calls, ca) = proxy(HOST_FILE, &env);
    (h, addr, calls, ca, dir)
}

const TARGET: &str = "api.openai.com:443";

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
    let (_h, addr, calls, _, _dir) = proxy_to(&up, false);
    let own = TlsConnector::from(up.pki().client_config(&[HTTP11]));
    let mut t = own
        .connect(name("api.openai.com"), through(addr, TARGET).await)
        .await
        .unwrap();
    get(&mut t).await;
    drop(t);
    let (sid, meta, _) = conns(&calls, 1).await.remove(0);
    assert_eq!(
        (&meta["mode"], &meta["reason"]),
        (&json!("tunnel"), &json!("upstream_untrusted"))
    );
    assert_eq!(gaps_of(&calls, sid), [(GapReason::UpstreamUntrusted, None)]);
}

/// cr-proxy-1: an upstream signed by a CA in the participant's `SSL_CERT_FILE` (a corporate
/// inspection CA) is trusted, so the call is decrypted, not tunnelled.
#[tokio::test]
async fn an_upstream_signed_by_the_participants_ca_is_decrypted() {
    let up = Upstream::start(Script::ok(b"hi").with_tls(&["api.openai.com"], &[HTTP11]));
    let dir = tempfile::tempdir().unwrap();
    let roots = up.pki().roots_file(dir.path()).display().to_string();
    let env = [
        (
            "GOALCUP_TEST_UPSTREAM",
            format!("api.openai.com={}", up.addr()),
        ),
        ("SSL_CERT_FILE", roots),
    ];
    let (_h, addr, calls, ca) = proxy(HOST_FILE, &env);
    let c = TlsConnector::from(client_trusting(vec![ca.cert_der().clone()], &[b"http/1.1"]));
    let mut t = c
        .connect(name("api.openai.com"), through(addr, TARGET).await)
        .await
        .unwrap();
    get(&mut t).await;
    drop(t);
    assert_eq!(conns(&calls, 1).await[0].1["mode"], "decrypt");
}

/// GAP 14: an upstream that rejects the offered ALPN is an `alpn_other` tunnel.
#[tokio::test]
async fn an_upstream_rejecting_the_alpn_is_an_alpn_other_tunnel() {
    let up = Upstream::start(Script::ok(b"hi").with_tls(&["api.openai.com"], &[H2]));
    let (_h, addr, calls, _, _dir) = proxy_to(&up, true);
    let own = TlsConnector::from(up.pki().client_config(&[HTTP11]));
    let _ = own
        .connect(name("api.openai.com"), through(addr, TARGET).await)
        .await;
    let (sid, meta, _) = conns(&calls, 1).await.remove(0);
    assert_eq!(meta["reason"], "alpn_other");
    assert_eq!(gaps_of(&calls, sid), [(GapReason::ProtocolOther, None)]);
}

/// Refusal learning through `serve`: an `unknown_ca` alert writes one note and a `tls_refused`
/// GAP naming it; the same fingerprint then tunnels to the real upstream; another fingerprint
/// is still decrypted.
#[tokio::test]
async fn an_alert_teaches_its_fingerprint_and_no_other() {
    let up = Upstream::start(Script::ok(b"hi").with_tls(&["api.openai.com"], &[HTTP11]));
    let (_h, addr, calls, ca, _dir) = proxy_to(&up, true);
    let refusing = TlsConnector::from(up.pki().client_config(&[HTTP11]));
    let first = refusing
        .connect(name("api.openai.com"), through(addr, TARGET).await)
        .await;
    assert!(first.is_err(), "our leaf is refused");
    let (sid, meta, _) = conns(&calls, 1).await.remove(0);
    assert_eq!(
        (&meta["mode"], &meta["reason"]),
        (&json!("tunnel"), &json!("tls_refused"))
    );
    let alert = Some(json!({"alert": "unknown_ca"}));
    assert_eq!(gaps_of(&calls, sid), [(GapReason::TlsRefused, alert)]);
    assert_eq!(ca_refused(&calls).len(), 1);

    let mut t = refusing
        .connect(name("api.openai.com"), through(addr, TARGET).await)
        .await
        .unwrap();
    get(&mut t).await;
    drop(t);
    assert_eq!(conns(&calls, 2).await[1].1["reason"], "tls_refused");

    let trusting = TlsConnector::from(client_trusting(vec![ca.cert_der().clone()], &[]));
    let mut t = trusting
        .connect(name("api.openai.com"), through(addr, TARGET).await)
        .await
        .unwrap();
    get(&mut t).await;
    drop(t);
    assert_eq!(conns(&calls, 3).await[2].1["mode"], "decrypt");
    assert_eq!(ca_refused(&calls).len(), 1);
}

/// A plain close mid-handshake: GAP `tls_refused` with `{"alert": false}`, nothing learned.
#[tokio::test]
async fn a_close_mid_handshake_teaches_nothing() {
    let up = Upstream::start(Script::ok(b"hi").with_tls(&["api.openai.com"], &[HTTP11]));
    let (_h, addr, calls, ca, _dir) = proxy_to(&up, true);
    let mut hello = Vec::new();
    let config = client_trusting(vec![ca.cert_der().clone()], &[b"http/1.1"]);
    let mut tls = ClientConnection::new(config, name("api.openai.com")).unwrap();
    tls.write_tls(&mut hello).unwrap();
    let mut s = through(addr, TARGET).await;
    s.write_all(&hello).await.unwrap();
    drop(s);
    let (sid, ..) = conns(&calls, 1).await.remove(0);
    let alert = Some(json!({"alert": false}));
    assert_eq!(gaps_of(&calls, sid), [(GapReason::TlsRefused, alert)]);
    assert!(ca_refused(&calls).is_empty());
}

/// cr-proxy-3 through `serve`: three clients that read our flight and close without an alert teach
/// the pair with one note; the fourth connection of the pair tunnels.
#[tokio::test]
async fn silent_closes_after_our_leaf_are_learned() {
    let up = Upstream::start(Script::ok(b"hi").with_tls(&["api.openai.com"], &[HTTP11]));
    let (_h, addr, calls, ca, _dir) = proxy_to(&up, true);
    let config = client_trusting(vec![ca.cert_der().clone()], &[b"http/1.1"]);
    for n in 1..=4 {
        let mut hello = Vec::new();
        let mut tls = ClientConnection::new(config.clone(), name("api.openai.com")).unwrap();
        tls.write_tls(&mut hello).unwrap();
        let mut s = through(addr, TARGET).await;
        s.write_all(&hello).await.unwrap();
        let mut flight = [0u8; 1];
        s.read_exact(&mut flight).await.unwrap();
        drop(s);
        conns(&calls, n).await;
    }
    assert_eq!(ca_refused(&calls).len(), 1);
    assert_eq!(conns(&calls, 4).await[3].1["reason"], "tls_refused");
}

/// Acceptance 3, automated half: an `h2`-only client sends four requests in parallel on one
/// connection. Each is recorded with its stream id and its answer, the query is dropped (point
/// 4), and nothing is lost.
#[tokio::test]
async fn four_parallel_h2_requests_are_recorded_on_one_conn() {
    let up = Upstream::start(Script::ok(b"hi").with_tls(&["api.openai.com"], &[H2]));
    let (_h, addr, calls, ca, _dir) = proxy_to(&up, true);
    let trusting = TlsConnector::from(client_trusting(vec![ca.cert_der().clone()], &[b"h2"]));
    let t = trusting
        .connect(name("api.openai.com"), through(addr, TARGET).await)
        .await
        .unwrap();
    let (mut send, conn) = h2::client::handshake(t).await.unwrap();
    let driver = tokio::spawn(conn);
    let mut answers = Vec::new();
    for _ in 0..4 {
        send = send.ready().await.unwrap();
        let req = http::Request::get("https://api.openai.com/v1/models?key=v")
            .body(())
            .unwrap();
        answers.push(send.send_request(req, true).unwrap().0);
    }
    for answer in answers {
        let mut body = answer.await.unwrap().into_body();
        let mut got = Vec::new();
        while let Some(piece) = body.data().await {
            let piece = piece.unwrap();
            let _ = body.flow_control().release_capacity(piece.len());
            got.extend_from_slice(&piece);
        }
        assert_eq!(got, b"hi");
    }
    drop(send);
    driver.abort();
    let (sid, meta, _) = conns(&calls, 1).await.remove(0);
    assert_eq!(
        (&meta["mode"], &meta["alpn"]),
        (&json!("decrypt"), &json!("h2"))
    );
    let opened = opens(&calls);
    let of = |kind: StreamKind| (opened.iter()).filter(move |o| o.1 == kind && o.2["h2"].is_u64());
    let ids: BTreeSet<_> = of(StreamKind::HttpReq)
        .map(|o| o.2["h2"].as_u64())
        .collect();
    assert_eq!(ids.len(), 4, "four requests on four streams");
    for (_, _, meta) in of(StreamKind::HttpReq) {
        assert_eq!(meta["path"], "/v1/models");
        assert!(
            meta["headers"]
                .as_array()
                .unwrap()
                .contains(&json!([":path", "/v1/models"]))
        );
    }
    let answered: Vec<_> = of(StreamKind::HttpResp)
        .map(|o| data_of(&calls, o.0))
        .collect();
    assert_eq!(answered, vec![b"hi".to_vec(); 4]);
    let lost = |g: &Call| matches!(g, Call::Gap(_, GapReason::H2Unsupported, ..));
    assert!(!gaps(&calls).iter().any(lost));
    assert!(gaps_of(&calls, sid).is_empty());
    assert!(!events(&calls).iter().any(|e| e["what"] == "parse_stopped"));
}
