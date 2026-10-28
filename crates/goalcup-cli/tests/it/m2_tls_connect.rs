//! CONNECT admission, SNI inspection and the per-launch CA, over real loopback TCP and real
//! TLS on both legs.
//!
//! Admission takes no capture sink and emits no frames (the relay root builds
//! `RelayContext`), so "the sink receives nothing" holds by construction. What these tests
//! inspect instead is every surface admission does produce: the outcome and its
//! `Display`/`Debug`, the bytes written back to the client, and the bytes that reach the
//! upstream.
//!
//! `fixtures/m2-tls/openssl-clienthello-*.hex` are ClientHellos an OpenSSL 3.6.4 client
//! (Python `ssl`) sent to a loopback socket, one with SNI `localhost` and one with none. They
//! are an oracle for SNI parsing from a TLS implementation other than the rustls the recording
//! proxy and the other tests use. They hold only random bytes, ephemeral public keys and the name
//! `localhost`.

use crate::common::m2_tls_support;

use goalcup_cli::proxy::api::{CapturePath, CapturePolicy};
use goalcup_cli::proxy::connect::{
    Admission, AdmissionConfig, AdmissionConfigError, AdmissionFailure, ApprovedHost, FailureClass,
    MAX_CONNECT_HEAD_BYTES, PROXY_USERNAME, ProxyCredential, TunnelEnd, TunnelReason, Tunneled,
};
use goalcup_cli::proxy::tls::{CA_CERT_FILE_NAME, CA_VALIDITY, CaError, ProxyCa, UpstreamRoots};
use m2_tls_support::*;
use rustls::pki_types::CertificateDer;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

const H11: &[u8] = b"http/1.1";
const H2: &[u8] = b"h2";

fn policy() -> CapturePolicy {
    CapturePolicy {
        paths: vec![CapturePath {
            path: "/v1/responses".into(),
            prefix: false,
            protocol_hint: Some("openai-responses".into()),
        }],
    }
}

fn credential() -> ProxyCredential {
    ProxyCredential::from_secret(SECRET.into()).unwrap()
}

/// A per-launch CA for `approved` and an admission config over it.
fn config(
    approved: &[&str],
    auth: &[&str],
    roots: &UpstreamRoots,
) -> (ProxyCa, Arc<AdmissionConfig>) {
    let hosts: Vec<String> = approved.iter().map(|h| h.to_string()).collect();
    let ca = ProxyCa::generate(&hosts).unwrap();
    let config = AdmissionConfig::new(
        hosts
            .into_iter()
            .map(|host| ApprovedHost {
                host,
                policy: policy(),
            })
            .collect(),
        auth.iter().map(|h| h.to_string()).collect(),
        credential(),
        &ca,
        roots,
    )
    .unwrap();
    (ca, Arc::new(config))
}

fn roots(pki: &TestPki) -> UpstreamRoots {
    UpstreamRoots::from_certificates([pki.root()]).unwrap()
}

fn failed(outcome: Admission) -> AdmissionFailure {
    match outcome {
        Admission::Failed(failure) => failure,
        other => panic!("expected a failed admission, got {other:?}"),
    }
}

fn tunneled(outcome: Admission) -> Tunneled {
    match outcome {
        Admission::Tunneled(tunnel) => tunnel,
        other => panic!("expected a tunnel, got {other:?}"),
    }
}

/// A tunnel's host, port, reason and end: everything but its clock times.
fn facts(tunnel: &Tunneled) -> (&str, u16, TunnelReason, TunnelEnd) {
    assert!(tunnel.opened <= tunnel.closed);
    (&tunnel.host, tunnel.port, tunnel.reason, tunnel.end)
}

/// Every diagnostic surface of an outcome, as bytes.
fn diagnostics(outcome: &Admission) -> Vec<u8> {
    let mut text = format!("{outcome:?}");
    if let Admission::Failed(failure) = outcome {
        text.push_str(&format!(" {failure} {}", failure.class));
    }
    text.into_bytes()
}

fn assert_absent(what: &str, haystack: &[u8], sentinels: &[&str]) {
    for sentinel in sentinels {
        assert!(
            !contains(haystack, sentinel.as_bytes()),
            "{what} contains {sentinel:?}: {}",
            String::from_utf8_lossy(haystack)
        );
    }
}

/// Connects a real TLS client through the recording proxy to `target`, trusting only `ca`.
async fn tls_through(
    proxy: std::net::SocketAddr,
    target: &str,
    ca: &ProxyCa,
    alpn: &[&[u8]],
) -> std::io::Result<tokio_rustls::client::TlsStream<TcpStream>> {
    let tcp = open_tunnel(proxy, target).await;
    let connector = TlsConnector::from(client_config(&[ca.certificate_der().clone()], alpn, true));
    within(
        "client handshake",
        connector.connect(server_name("localhost"), tcp),
    )
    .await
}

fn der_extensions(der: &[u8]) -> x509_parser::certificate::X509Certificate<'_> {
    x509_parser::parse_x509_certificate(der).unwrap().1
}

fn key_ids(
    cert: &x509_parser::certificate::X509Certificate<'_>,
) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    use x509_parser::extensions::ParsedExtension;
    let (mut ski, mut aki) = (None, None);
    for ext in cert.extensions() {
        match ext.parsed_extension() {
            ParsedExtension::SubjectKeyIdentifier(id) => ski = Some(id.0.to_vec()),
            ParsedExtension::AuthorityKeyIdentifier(a) => {
                aki = a.key_identifier.as_ref().map(|id| id.0.to_vec())
            }
            _ => {}
        }
    }
    (ski, aki)
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

#[tokio::test]
async fn trusted_upstream_is_intercepted_with_a_per_launch_leaf() {
    let pki = TestPki::new("upstream test CA");
    let (chain, key) = pki.leaf(&["localhost"]);
    let mut upstream = TlsUpstream::start(chain, key, &[H11]).await;
    let (ca, config) = config(&["localhost"], &[], &roots(&pki));
    let proxy = serve_one(config).await;
    let target = format!("localhost:{}", upstream.addr.port());

    let (client, outcome) = tokio::join!(
        tls_through(proxy.addr, &target, &ca, &[H11]),
        proxy.outcome()
    );
    let Admission::Intercepted(intercepted) = outcome else {
        panic!("expected interception, got {outcome:?}");
    };
    assert_eq!(intercepted.host, "localhost");
    assert_eq!(intercepted.policy, policy());
    assert_eq!(intercepted.alpn.as_deref(), Some("http/1.1"));
    let mut client = client.expect("client trusting only the per-launch CA completes TLS");
    assert_eq!(client.get_ref().1.alpn_protocol(), Some(H11));

    // The leaf the harness saw: issued by the per-launch CA, for the host, with an AKI.
    let leaf = client.get_ref().1.peer_certificates().unwrap()[0].clone();
    let leaf = der_extensions(&leaf);
    let ca_cert = der_extensions(ca.certificate_der());
    let (ca_ski, _) = key_ids(&ca_cert);
    let (_, leaf_aki) = key_ids(&leaf);
    assert!(
        ca_ski.is_some() && leaf_aki == ca_ski,
        "leaf AKI must name the CA key"
    );
    let san = leaf.subject_alternative_name().unwrap().unwrap();
    assert_eq!(
        format!("{:?}", san.value.general_names),
        r#"[DNSName("localhost")]"#
    );
    assert!(
        leaf.extended_key_usage()
            .unwrap()
            .unwrap()
            .value
            .server_auth
    );
    assert!(!leaf.is_ca());
    let not_after = leaf.validity().not_after.timestamp();
    assert!(not_after <= unix_now() + CA_VALIDITY.as_secs() as i64 + 60);

    // The upstream leg was validated TLS to the same name, HTTP/1.1.
    let seen = upstream.next_seen().await;
    assert_eq!(seen.handshake, Ok(()));
    assert_eq!(seen.sni.as_deref(), Some("localhost"));
    assert_eq!(seen.alpn.as_deref(), Some(H11));

    let relay = relay(intercepted.pair);
    assert_eq!(echo_round_trip(&mut client, "ping\n").await, "echo:ping\n");
    drop(client);
    within("relay end", relay).await.unwrap();
    upstream.shutdown().await;
}

#[tokio::test]
async fn upstream_with_wrong_hostname_is_refused() {
    let pki = TestPki::new("upstream test CA");
    let (chain, key) = pki.leaf(&["wrong.test"]);
    let mut upstream = TlsUpstream::start(chain, key, &[H11]).await;
    let (ca, config) = config(&["localhost"], &[], &roots(&pki));
    let proxy = serve_one(config).await;
    let target = format!("localhost:{}", upstream.addr.port());

    let (client, outcome) = tokio::join!(
        tls_through(proxy.addr, &target, &ca, &[H11]),
        proxy.outcome()
    );
    assert_eq!(
        failed(outcome),
        AdmissionFailure {
            class: FailureClass::UpstreamTls,
            host: Some("localhost".into())
        }
    );
    assert!(
        client.is_err(),
        "no harness-facing handshake after a failed upstream check"
    );
    assert!(upstream.next_seen().await.handshake.is_err());
    upstream.shutdown().await;
}

#[tokio::test]
async fn untrusted_upstream_is_refused() {
    let trusted = TestPki::new("trusted test CA");
    let untrusted = TestPki::new("untrusted test CA");
    let (chain, key) = untrusted.leaf(&["localhost"]);
    let mut upstream = TlsUpstream::start(chain, key, &[H11]).await;
    let (ca, config) = config(&["localhost"], &[], &roots(&trusted));
    let proxy = serve_one(config).await;
    let target = format!("localhost:{}", upstream.addr.port());

    let (client, outcome) = tokio::join!(
        tls_through(proxy.addr, &target, &ca, &[H11]),
        proxy.outcome()
    );
    assert_eq!(failed(outcome).class, FailureClass::UpstreamTls);
    assert!(client.is_err());
    assert!(upstream.next_seen().await.handshake.is_err());
    upstream.shutdown().await;
}

#[tokio::test]
async fn refused_loopback_upstream_keeps_a_fixed_connect_failure_kind() {
    let reserved = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reserved.local_addr().unwrap().port();
    drop(reserved);
    let pki = TestPki::new("unused");
    // The default 30 s admission: a host that is not a provider's (the agent's own traffic) is
    // never retried, so its refusal is answered at once, as it always was.
    let (_ca, config) = config(&["api.test"], &[], &roots(&pki));
    let proxy = serve_one(config).await;
    let target = format!("127.0.0.1:{port}");
    let started = Instant::now();
    let mut client = TcpStream::connect(proxy.addr).await.unwrap();
    client
        .write_all(&connect_head(&target, Some(&good_auth())))
        .await
        .unwrap();
    let response = read_to_end(&mut client).await;
    let failure = failed(proxy.outcome().await);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "a host that is not a provider's was retried: {:?}",
        started.elapsed()
    );
    assert_eq!(
        failure.class,
        FailureClass::UpstreamConnect(goalcup_cli::proxy::connect::ConnectFailureKind::TcpRefused)
    );
    assert!(response.starts_with(b"HTTP/1.1 502 "));
}

/// A provider host (here an authentication host, whose failed connect is answered `502`) is
/// retried until the connect deadline: five sixths of a 1.2 s admission is a full second.
#[tokio::test]
async fn a_refused_provider_host_is_retried_before_its_502() {
    let reserved = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reserved.local_addr().unwrap().port();
    drop(reserved);
    let pki = TestPki::new("unused");
    let (_ca, config) = config(&["api.test"], &["localhost"], &roots(&pki));
    let config = Arc::new(
        Arc::try_unwrap(config)
            .unwrap()
            .with_timeouts(Duration::from_millis(1200), Duration::from_millis(300)),
    );
    let proxy = serve_one(config).await;
    let started = Instant::now();
    let mut client = TcpStream::connect(proxy.addr).await.unwrap();
    client
        .write_all(&connect_head(
            &format!("localhost:{port}"),
            Some(&good_auth()),
        ))
        .await
        .unwrap();
    let response = read_to_end(&mut client).await;
    let failure = failed(proxy.outcome().await);
    assert!(
        started.elapsed() >= Duration::from_millis(600),
        "a provider host was not retried: {:?}",
        started.elapsed()
    );
    assert!(matches!(failure.class, FailureClass::UpstreamConnect(_)));
    assert!(response.starts_with(b"HTTP/1.1 502 "));
}

#[tokio::test]
async fn connect_requires_the_proxy_credential() {
    let wrong = "WRONGSECRET-SENTINEL-7f3a";
    let pipelined = b"PIPELINED-PAYLOAD-SENTINEL";
    let cases: Vec<(&str, Vec<String>)> = vec![
        ("missing", vec![]),
        ("wrong secret", vec![basic(PROXY_USERNAME, wrong)]),
        ("wrong user", vec![basic("someone", SECRET)]),
        ("wrong scheme", vec![format!("Bearer {SECRET}")]),
        ("raw secret", vec![format!("Basic {SECRET}")]),
        ("repeated", vec![good_auth(), basic(PROXY_USERNAME, wrong)]),
    ];
    let pki = TestPki::new("unused");
    let (_ca, config) = config(&["api.test"], &[], &roots(&pki));
    for (case, headers) in cases {
        let recorder = Recorder::start(b"never".to_vec()).await;
        let proxy = serve_one(config.clone()).await;
        let target = format!("127.0.0.1:{}", recorder.addr.port());
        let mut head = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n");
        for value in &headers {
            head.push_str(&format!("Proxy-Authorization: {value}\r\n"));
        }
        head.push_str("\r\n");
        let mut bytes = head.into_bytes();
        bytes.extend_from_slice(pipelined);
        let mut tcp = TcpStream::connect(proxy.addr).await.unwrap();
        tcp.write_all(&bytes).await.unwrap();
        let response = read_to_end(&mut tcp).await;
        let outcome = proxy.outcome().await;
        let text = diagnostics(&outcome);
        assert_eq!(
            failed(outcome),
            AdmissionFailure {
                class: FailureClass::ProxyAuthRequired,
                host: None
            },
            "{case}"
        );
        assert!(response.starts_with(b"HTTP/1.1 407 "), "{case}");
        assert!(contains(&response, b"Proxy-Authenticate: Basic"), "{case}");
        let encoded_wrong = basic(PROXY_USERNAME, wrong);
        let encoded_good = good_auth();
        let sentinels = [
            SECRET,
            wrong,
            "PIPELINED",
            &encoded_wrong["Basic ".len()..],
            &encoded_good["Basic ".len()..],
        ];
        assert_absent(case, &text, &sentinels);
        assert_absent(case, &response, &sentinels);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            recorder.accepted(),
            0,
            "{case}: no upstream connection before auth"
        );
    }

    // Positive control on the same kind of target: the right credential (case-insensitive
    // scheme) reaches the upstream, and the pipelined bytes arrive there intact.
    let recorder = Recorder::start(b"reply".to_vec()).await;
    let proxy = serve_one(config.clone()).await;
    let target = format!("127.0.0.1:{}", recorder.addr.port());
    let value = good_auth().replacen("Basic", "bAsIc", 1);
    let mut bytes = connect_head(&target, Some(&value));
    bytes.extend_from_slice(pipelined);
    let mut tcp = TcpStream::connect(proxy.addr).await.unwrap();
    tcp.write_all(&bytes).await.unwrap();
    tcp.shutdown().await.unwrap();
    let (head, mut rest) = within("CONNECT response", read_head(&mut tcp)).await;
    assert!(head.starts_with(b"HTTP/1.1 200 "));
    rest.extend(read_to_end(&mut tcp).await);
    assert_eq!(rest, b"reply");
    assert_eq!(recorder.bytes().await, pipelined);
    assert_eq!(
        tunneled(proxy.outcome().await).reason,
        TunnelReason::NotApproved
    );
}

#[tokio::test]
async fn proxy_url_userinfo_is_the_credential_admission_accepts() {
    let pki = TestPki::new("unused");
    let (_ca, config) = config(&["api.test"], &[], &roots(&pki));
    let url = credential().proxy_url("127.0.0.1:9".parse().unwrap());
    let userinfo = url
        .strip_prefix("http://")
        .and_then(|rest| rest.split_once('@'))
        .map(|(userinfo, host)| {
            assert_eq!(host, "127.0.0.1:9");
            userinfo.to_owned()
        })
        .unwrap();
    let (user, secret) = userinfo.split_once(':').unwrap();
    let recorder = Recorder::start(Vec::new()).await;
    let proxy = serve_one(config).await;
    let mut tcp = TcpStream::connect(proxy.addr).await.unwrap();
    let target = format!("127.0.0.1:{}", recorder.addr.port());
    tcp.write_all(&connect_head(&target, Some(&basic(user, secret))))
        .await
        .unwrap();
    tcp.shutdown().await.unwrap();
    let (head, _) = within("CONNECT response", read_head(&mut tcp)).await;
    assert!(head.starts_with(b"HTTP/1.1 200 "));
    drop(tcp);
    assert_eq!(recorder.bytes().await, b"");
    assert!(matches!(proxy.outcome().await, Admission::Tunneled(_)));
}

#[tokio::test]
async fn split_connect_head_keeps_pipelined_tls_bytes() {
    // Interception: the head arrives in four writes and the ClientHello's first seven bytes
    // ride in the last one. A completed handshake proves every byte reached TLS once, in
    // order (the transcript hash covers them).
    let pki = TestPki::new("upstream test CA");
    let (chain, key) = pki.leaf(&["localhost"]);
    let upstream = TlsUpstream::start(chain, key, &[H11]).await;
    let (ca, config) = config(&["localhost"], &[], &roots(&pki));
    let proxy = serve_one(config.clone()).await;
    let head = connect_head(
        &format!("localhost:{}", upstream.addr.port()),
        Some(&good_auth()),
    );
    let plan = ShimPlan {
        head_splits: vec![9, 40, head.len() - 3],
        head,
        glue: 7,
        record_payload: None,
        write_piece: 4096,
    };
    let client_cfg = client_config(&[ca.certificate_der().clone()], &[H11], true);
    let ((client, shim), outcome) = tokio::join!(
        tls_via_shim(proxy.addr, plan, client_cfg, "localhost"),
        proxy.outcome()
    );
    let Admission::Intercepted(intercepted) = outcome else {
        panic!("expected interception, got {outcome:?}");
    };
    let mut client = client.expect("handshake over a pipelined CONNECT");
    let relay = relay(intercepted.pair);
    assert_eq!(
        echo_round_trip(&mut client, "pipelined\n").await,
        "echo:pipelined\n"
    );
    drop(client);
    within("relay end", relay).await.unwrap();
    within("shim end", shim).await.unwrap();
    upstream.shutdown().await;

    // Tunnel: the same pipelining with a ClientHello for another name; the upstream must see
    // every post-head byte exactly once and in order, and the client the upstream's reply.
    let recorder = Recorder::start(b"UPSTREAM-REPLY".to_vec()).await;
    let upstream_port = recorder.addr.port();
    let proxy = serve_one(config).await;
    let hello = client_hello(client_config(&[], &[H11], true), "other.test");
    let mut payload = hello.clone();
    payload.extend_from_slice(b"\x00TRAILING-OPAQUE\xff");
    let head = connect_head(
        &format!("localhost:{}", recorder.addr.port()),
        Some(&good_auth()),
    );
    let mut tcp = TcpStream::connect(proxy.addr).await.unwrap();
    tcp.set_nodelay(true).unwrap();
    write_in_pieces(&mut tcp, &head[..head.len() - 2], 11).await;
    let mut last = head[head.len() - 2..].to_vec();
    last.extend_from_slice(&payload[..10]);
    tcp.write_all(&last).await.unwrap();
    write_in_pieces(&mut tcp, &payload[10..], 97).await;
    tcp.shutdown().await.unwrap();
    let (response, mut rest) = within("CONNECT response", read_head(&mut tcp)).await;
    assert!(response.starts_with(b"HTTP/1.1 200 "));
    rest.extend(read_to_end(&mut tcp).await);
    assert_eq!(rest, b"UPSTREAM-REPLY");
    assert_eq!(recorder.bytes().await, payload);
    assert_eq!(
        facts(&tunneled(proxy.outcome().await)),
        (
            "localhost",
            upstream_port,
            TunnelReason::SniMismatch,
            TunnelEnd::Closed {
                client_to_upstream: payload.len() as u64,
                upstream_to_client: 14
            }
        )
    );
}

/// Sends `payload` through an authenticated CONNECT to `target`, half-closes, and returns
/// what came back after the `200` head.
async fn tunnel_round_trip(proxy: std::net::SocketAddr, target: &str, payload: &[u8]) -> Vec<u8> {
    let mut tcp = open_tunnel(proxy, target).await;
    write_in_pieces(&mut tcp, payload, 61).await;
    tcp.shutdown().await.unwrap();
    read_to_end(&mut tcp).await
}

#[tokio::test]
async fn auth_and_non_capture_hosts_are_opaque_tunnels() {
    let pki = TestPki::new("unused");
    let (_ca, config) = config(&["api.test"], &["localhost"], &roots(&pki));
    let mut payload = client_hello(client_config(&[], &[H11], true), "api.test");
    payload.extend_from_slice(b"Authorization: Bearer AUTH-SENTINEL-9d1e\r\n");
    payload.extend((0..=255u8).collect::<Vec<_>>());
    let reply = b"UPSTREAM-OPAQUE-\x00\xff-REPLY".to_vec();

    // An authentication host, even with a ClientHello naming an approved host, is still an
    // unchanged tunnel whose content is never read. It is reported as `SniMismatch`, a
    // fail-open tunnel the recording proxy chains as a loss: an approved host's traffic must not
    // leave uncaptured through an authentication host.
    let recorder = Recorder::start(reply.clone()).await;
    let upstream_port = recorder.addr.port();
    let proxy = serve_one(config.clone()).await;
    let target = format!("localhost:{upstream_port}");
    assert_eq!(
        tunnel_round_trip(proxy.addr, &target, &payload).await,
        reply
    );
    assert_eq!(recorder.bytes().await, payload);
    let outcome = proxy.outcome().await;
    assert_absent(
        "auth tunnel outcome",
        &diagnostics(&outcome),
        &["AUTH-SENTINEL", "Bearer", "api.test"],
    );
    assert_eq!(
        facts(&tunneled(outcome)),
        (
            "localhost",
            upstream_port,
            TunnelReason::SniMismatch,
            TunnelEnd::Closed {
                client_to_upstream: payload.len() as u64,
                upstream_to_client: reply.len() as u64
            }
        )
    );

    // A host that is neither approved nor an authentication host, whose ClientHello names a
    // host that is not approved either.
    let mut payload = client_hello(client_config(&[], &[H11], true), "other.test");
    payload.extend_from_slice(b"Authorization: Bearer AUTH-SENTINEL-9d1e\r\n");
    payload.extend((0..=255u8).collect::<Vec<_>>());
    let recorder = Recorder::start(reply.clone()).await;
    let proxy = serve_one(config).await;
    let target = format!("127.0.0.1:{}", recorder.addr.port());
    assert_eq!(
        tunnel_round_trip(proxy.addr, &target, &payload).await,
        reply
    );
    assert_eq!(recorder.bytes().await, payload);
    let outcome = proxy.outcome().await;
    assert_absent(
        "non-capture outcome",
        &diagnostics(&outcome),
        &["AUTH-SENTINEL", "Bearer"],
    );
    let tunnel = tunneled(outcome);
    assert_eq!(tunnel.host, "127.0.0.1");
    assert_eq!(tunnel.reason, TunnelReason::NotApproved);
}

#[tokio::test]
async fn fragmented_client_hello_decides_by_sni() {
    let pki = TestPki::new("upstream test CA");
    let (chain, key) = pki.leaf(&["localhost"]);
    let mut upstream = TlsUpstream::start(chain, key, &[H11]).await;
    let (ca, config) = config(&["localhost"], &[], &roots(&pki));

    // Matching SNI, ClientHello split into 64-byte records sent in 50-byte writes:
    // intercepted, and the handshake completes.
    let proxy = serve_one(config.clone()).await;
    let plan = ShimPlan {
        head: connect_head(
            &format!("localhost:{}", upstream.addr.port()),
            Some(&good_auth()),
        ),
        head_splits: vec![],
        glue: 0,
        record_payload: Some(64),
        write_piece: 50,
    };
    let client_cfg = client_config(&[ca.certificate_der().clone()], &[H11], true);
    let ((client, shim), outcome) = tokio::join!(
        tls_via_shim(proxy.addr, plan, client_cfg, "localhost"),
        proxy.outcome()
    );
    let Admission::Intercepted(intercepted) = outcome else {
        panic!("expected interception, got {outcome:?}");
    };
    let mut client = client.expect("handshake over a fragmented ClientHello");
    assert_eq!(upstream.next_seen().await.handshake, Ok(()));
    let relay = relay(intercepted.pair);
    assert_eq!(
        echo_round_trip(&mut client, "fragments\n").await,
        "echo:fragments\n"
    );
    drop(client);
    within("relay end", relay).await.unwrap();
    within("shim end", shim).await.unwrap();

    // Absent and non-matching SNI, from rustls and from OpenSSL: tunnelled, and the upstream
    // receives exactly the bytes the client sent, including those read during inspection.
    let fragmented = |hello: Vec<u8>| refragment(&hello, 40);
    let cases = [
        (
            "rustls, no SNI",
            fragmented(client_hello(client_config(&[], &[H11], false), "localhost")),
            TunnelReason::SniAbsent,
        ),
        (
            "rustls, other SNI",
            fragmented(client_hello(client_config(&[], &[H11], true), "other.test")),
            TunnelReason::SniMismatch,
        ),
        (
            "rustls, SNI with the approved name as a prefix",
            fragmented(client_hello(
                client_config(&[], &[H11], true),
                "localhost.test",
            )),
            TunnelReason::SniMismatch,
        ),
        (
            "openssl, no SNI",
            hex_fixture(include_str!(
                "../fixtures/m2-tls/openssl-clienthello-no-sni.hex"
            )),
            TunnelReason::SniAbsent,
        ),
        (
            "openssl, no SNI, fragmented",
            fragmented(hex_fixture(include_str!(
                "../fixtures/m2-tls/openssl-clienthello-no-sni.hex"
            ))),
            TunnelReason::SniAbsent,
        ),
    ];
    for (case, hello, reason) in cases {
        let recorder = Recorder::start(b"tunnel-reply".to_vec()).await;
        let proxy = serve_one(config.clone()).await;
        let target = format!("localhost:{}", recorder.addr.port());
        assert_eq!(
            tunnel_round_trip(proxy.addr, &target, &hello).await,
            b"tunnel-reply",
            "{case}"
        );
        assert_eq!(recorder.bytes().await, hello, "{case}");
        let tunnel = tunneled(proxy.outcome().await);
        assert_eq!(
            (tunnel.host.as_str(), tunnel.reason),
            ("localhost", reason),
            "{case}"
        );
    }

    // OpenSSL's ClientHello naming `localhost`, replayed raw: the SNI matches, so admission
    // validates the upstream and answers with its own TLS server flight instead of
    // tunnelling. The replaying client cannot finish the handshake, which fails client-side.
    let openssl = hex_fixture(include_str!(
        "../fixtures/m2-tls/openssl-clienthello-sni-localhost.hex"
    ));
    let proxy = serve_one(config.clone()).await;
    let mut tcp = open_tunnel(proxy.addr, &format!("localhost:{}", upstream.addr.port())).await;
    write_in_pieces(&mut tcp, &openssl, 300).await;
    let mut flight = [0u8; 3];
    within("server flight", tcp.read_exact(&mut flight))
        .await
        .unwrap();
    assert_eq!(
        flight,
        [0x16, 0x03, 0x03],
        "a TLS handshake record from the recording proxy"
    );
    drop(tcp);
    assert_eq!(
        failed(proxy.outcome().await),
        AdmissionFailure {
            class: FailureClass::ClientTls,
            host: Some("localhost".into())
        }
    );
    assert_eq!(upstream.next_seen().await.handshake, Ok(()));
    upstream.shutdown().await;
}

#[tokio::test]
async fn http1_alpn_policy_is_explicit() {
    let pki = TestPki::new("upstream test CA");
    let (chain, key) = pki.leaf(&["localhost"]);
    // An upstream that would prefer h2 if it were offered.
    let mut upstream = TlsUpstream::start(chain, key, &[H2, H11]).await;
    let (ca, config) = config(&["localhost"], &[], &roots(&pki));
    let target = format!("localhost:{}", upstream.addr.port());

    // Offered h2 and http/1.1: only http/1.1 is selected, on both legs.
    let proxy = serve_one(config.clone()).await;
    let (client, outcome) = tokio::join!(
        tls_through(proxy.addr, &target, &ca, &[H2, H11]),
        proxy.outcome()
    );
    let Admission::Intercepted(intercepted) = outcome else {
        panic!("expected interception, got {outcome:?}");
    };
    assert_eq!(intercepted.alpn.as_deref(), Some("http/1.1"));
    let mut client = client.unwrap();
    assert_eq!(client.get_ref().1.alpn_protocol(), Some(H11));
    assert_eq!(upstream.next_seen().await.alpn.as_deref(), Some(H11));
    let relay_task = relay(intercepted.pair);
    assert_eq!(echo_round_trip(&mut client, "h11\n").await, "echo:h11\n");
    drop(client);
    within("relay end", relay_task).await.unwrap();

    // No ALPN at all (WebSocket-style clients): accepted as HTTP/1.1, reported as None.
    let proxy = serve_one(config.clone()).await;
    let (client, outcome) =
        tokio::join!(tls_through(proxy.addr, &target, &ca, &[]), proxy.outcome());
    let Admission::Intercepted(intercepted) = outcome else {
        panic!("expected interception, got {outcome:?}");
    };
    assert_eq!(intercepted.alpn, None);
    let mut client = client.unwrap();
    assert_eq!(client.get_ref().1.alpn_protocol(), None);
    assert_eq!(upstream.next_seen().await.alpn.as_deref(), Some(H11));
    let relay_task = relay(intercepted.pair);
    assert_eq!(echo_round_trip(&mut client, "none\n").await, "echo:none\n");
    drop(client);
    within("relay end", relay_task).await.unwrap();

    // h2 only: refused with an explicit no_application_protocol alert.
    let proxy = serve_one(config.clone()).await;
    let (client, outcome) = tokio::join!(
        tls_through(proxy.addr, &target, &ca, &[H2]),
        proxy.outcome()
    );
    assert_eq!(failed(outcome).class, FailureClass::AlpnIncompatible);
    let error = client.unwrap_err();
    assert_eq!(
        error
            .get_ref()
            .and_then(|e| e.downcast_ref::<rustls::Error>()),
        Some(&rustls::Error::AlertReceived(
            rustls::AlertDescription::NoApplicationProtocol
        ))
    );
    assert_eq!(upstream.next_seen().await.handshake, Ok(()));
    upstream.shutdown().await;

    // An upstream that speaks only h2 refuses the recording proxy's http/1.1-only offer.
    let (chain, key) = pki.leaf(&["localhost"]);
    let mut h2_only = TlsUpstream::start(chain, key, &[H2]).await;
    let proxy = serve_one(config).await;
    let target = format!("localhost:{}", h2_only.addr.port());
    let (client, outcome) = tokio::join!(
        tls_through(proxy.addr, &target, &ca, &[H11]),
        proxy.outcome()
    );
    assert_eq!(failed(outcome).class, FailureClass::AlpnIncompatible);
    assert!(client.is_err());
    assert!(h2_only.next_seen().await.handshake.is_err());
    h2_only.shutdown().await;
}

#[tokio::test]
async fn oversized_and_malformed_connect_heads_are_refused_without_echo() {
    let pki = TestPki::new("unused");
    let (_ca, config) = config(&["localhost"], &[], &roots(&pki));
    let config = Arc::new(
        Arc::try_unwrap(config)
            .unwrap()
            .with_timeouts(Duration::from_secs(5), Duration::from_secs(1)),
    );
    let auth = good_auth();
    let sentinels = ["SENTINEL", SECRET, &auth["Basic ".len()..]];

    // Exactly the bound, no terminator, connection left open: refused at the bound, not
    // after waiting for more.
    let mut oversized =
        format!("CONNECT localhost:443 HTTP/1.1\r\nProxy-Authorization: {auth}\r\nX-SENTINEL: ")
            .into_bytes();
    oversized.resize(MAX_CONNECT_HEAD_BYTES, b'a');
    let proxy = serve_one(config.clone()).await;
    let mut tcp = TcpStream::connect(proxy.addr).await.unwrap();
    tcp.write_all(&oversized).await.unwrap();
    let response = read_to_end(&mut tcp).await;
    let outcome = proxy.outcome().await;
    assert_absent("oversized", &diagnostics(&outcome), &sentinels);
    assert_eq!(failed(outcome).class, FailureClass::HeadTooLarge);
    assert!(
        response.starts_with(b"HTTP/1.1 431 "),
        "{}",
        String::from_utf8_lossy(&response)
    );
    assert_absent("oversized response", &response, &sentinels);

    // A flood far past the bound is cut off at the bound too.
    let proxy = serve_one(config.clone()).await;
    let mut tcp = TcpStream::connect(proxy.addr).await.unwrap();
    let flood = tokio::spawn(async move {
        let chunk = vec![b'x'; 64 * 1024];
        for _ in 0..64 {
            if tcp.write_all(&chunk).await.is_err() {
                break;
            }
        }
    });
    let started = Instant::now();
    assert_eq!(
        failed(proxy.outcome().await).class,
        FailureClass::HeadTooLarge
    );
    assert!(started.elapsed() < Duration::from_secs(3));
    within("flood writer", flood).await.unwrap();

    let malformed: Vec<(&str, String, FailureClass, &str)> = vec![
        (
            "userinfo in target",
            format!(
                "CONNECT user:SENTINEL-USERINFO@localhost:443 HTTP/1.1\r\nProxy-Authorization: {auth}\r\n\r\n"
            ),
            FailureClass::Malformed,
            "400",
        ),
        (
            "path and query in target",
            format!(
                "CONNECT localhost:443/v1?key=SENTINEL-QUERY#frag HTTP/1.1\r\nProxy-Authorization: {auth}\r\n\r\n"
            ),
            FailureClass::Malformed,
            "400",
        ),
        (
            "absolute-form request",
            format!(
                "GET http://localhost/v1?token=SENTINEL-GET HTTP/1.1\r\nProxy-Authorization: {auth}\r\n\r\n"
            ),
            FailureClass::NotConnect,
            "405",
        ),
        (
            "header without colon",
            format!(
                "CONNECT localhost:443 HTTP/1.1\r\nProxy-Authorization: {auth}\r\nbroken SENTINEL-HEADER\r\n\r\n"
            ),
            FailureClass::Malformed,
            "400",
        ),
        (
            "folded header",
            format!(
                "CONNECT localhost:443 HTTP/1.1\r\nProxy-Authorization: {auth}\r\nX-A: b\r\n SENTINEL-FOLD\r\n\r\n"
            ),
            FailureClass::Malformed,
            "400",
        ),
        (
            "control byte in header",
            format!(
                "CONNECT localhost:443 HTTP/1.1\r\nProxy-Authorization: {auth}\r\nX-A: SENTINEL\x01CTL\r\n\r\n"
            ),
            FailureClass::Malformed,
            "400",
        ),
        (
            "unknown version",
            format!(
                "CONNECT localhost:443 HTTP/2.0\r\nProxy-Authorization: {auth}\r\nX-SENTINEL: 1\r\n\r\n"
            ),
            FailureClass::Malformed,
            "400",
        ),
        (
            "missing port",
            format!("CONNECT SENTINEL-HOST HTTP/1.1\r\nProxy-Authorization: {auth}\r\n\r\n"),
            FailureClass::Malformed,
            "400",
        ),
        (
            "port zero",
            format!("CONNECT sentinel.test:0 HTTP/1.1\r\nProxy-Authorization: {auth}\r\n\r\n"),
            FailureClass::Malformed,
            "400",
        ),
        (
            "extra request-line token",
            format!(
                "CONNECT localhost:443 HTTP/1.1 SENTINEL-EXTRA\r\nProxy-Authorization: {auth}\r\n\r\n"
            ),
            FailureClass::Malformed,
            "400",
        ),
    ];
    for (case, head, class, status) in malformed {
        let proxy = serve_one(config.clone()).await;
        let mut tcp = TcpStream::connect(proxy.addr).await.unwrap();
        tcp.write_all(head.as_bytes()).await.unwrap();
        let response = read_to_end(&mut tcp).await;
        let outcome = proxy.outcome().await;
        let text = diagnostics(&outcome);
        assert_eq!(
            failed(outcome),
            AdmissionFailure { class, host: None },
            "{case}"
        );
        assert!(
            response.starts_with(format!("HTTP/1.1 {status} ").as_bytes()),
            "{case}: {}",
            String::from_utf8_lossy(&response)
        );
        assert_absent(case, &text, &sentinels);
        assert_absent(case, &response, &sentinels);
        assert_absent(case, &text, &["sentinel"]);
    }
}

/// Every entry under `root`: relative path -> (mode, file contents or symlink target, or
/// `None` for a directory).
fn snapshot(root: &Path) -> BTreeMap<String, (u32, Option<Vec<u8>>)> {
    use std::os::unix::fs::PermissionsExt as _;
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let meta = std::fs::symlink_metadata(&path).unwrap();
            let rel = path.strip_prefix(root).unwrap().display().to_string();
            let content = if meta.is_dir() {
                stack.push(path.clone());
                None
            } else if meta.file_type().is_symlink() {
                Some(
                    std::fs::read_link(&path)
                        .unwrap()
                        .display()
                        .to_string()
                        .into_bytes(),
                )
            } else {
                Some(std::fs::read(&path).unwrap())
            };
            out.insert(rel, (meta.permissions().mode() & 0o7777, content));
        }
    }
    out
}

#[tokio::test]
async fn ca_certificate_is_private_and_only_the_scoped_home_changes() {
    use std::os::unix::fs::PermissionsExt as _;
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    for dir in [
        "home/.codex",
        "home/.claude",
        "config/app",
        "scoped",
        "trap",
    ] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    std::fs::write(
        root.join("home/.codex/config.toml"),
        "model = \"unchanged\"\n",
    )
    .unwrap();
    std::fs::write(root.join("home/.claude/settings.json"), "{}\n").unwrap();
    std::fs::write(root.join("config/app/trust.pem"), "unchanged\n").unwrap();
    std::os::unix::fs::symlink(
        "../home/.codex/config.toml",
        root.join("trap").join(CA_CERT_FILE_NAME),
    )
    .unwrap();
    let before = snapshot(root);

    let pki = TestPki::new("upstream test CA");
    let (ca, config) = config(&["localhost"], &[], &roots(&pki));
    let written = ca.write_certificate(&root.join("scoped")).unwrap();
    assert_eq!(written, root.join("scoped").join(CA_CERT_FILE_NAME));
    assert_eq!(
        ca.write_certificate(&root.join("scoped")),
        Err(CaError::Write(std::io::ErrorKind::AlreadyExists)),
        "an existing file is never replaced"
    );
    assert_eq!(
        ca.write_certificate(&root.join("trap")),
        Err(CaError::Write(std::io::ErrorKind::AlreadyExists)),
        "a planted symlink is never followed"
    );

    // A full interception with this CA writes nothing either.
    let (chain, key) = pki.leaf(&["localhost"]);
    let upstream = TlsUpstream::start(chain, key, &[H11]).await;
    let proxy = serve_one(config).await;
    let target = format!("localhost:{}", upstream.addr.port());
    let (client, outcome) = tokio::join!(
        tls_through(proxy.addr, &target, &ca, &[H11]),
        proxy.outcome()
    );
    assert!(matches!(outcome, Admission::Intercepted(_)));
    drop((client, outcome));
    upstream.shutdown().await;

    let mut after = snapshot(root);
    let (mode, content) = after
        .remove(&format!("scoped/{CA_CERT_FILE_NAME}"))
        .expect("the certificate file");
    assert_eq!(after, before, "nothing but the scoped certificate changed");
    assert_eq!(mode, 0o600);
    let pem = content.unwrap();
    assert_eq!(pem, ca.certificate_pem().as_bytes());
    let pem = String::from_utf8(pem).unwrap();
    assert_eq!(pem.matches("-----BEGIN CERTIFICATE-----").count(), 1);
    assert!(!pem.contains("PRIVATE KEY"));
    assert_eq!(
        std::fs::metadata(&written).unwrap().permissions().mode() & 0o777,
        0o600
    );

    // The CA is constrained to the approved hosts, cannot sign intermediates, is short-lived.
    let cert = der_extensions(ca.certificate_der());
    let constraints = cert.basic_constraints().unwrap().unwrap();
    assert!(constraints.value.ca);
    assert_eq!(constraints.value.path_len_constraint, Some(0));
    let names = cert.name_constraints().unwrap().unwrap();
    assert!(names.critical);
    assert_eq!(
        format!(
            "{:?}",
            names
                .value
                .permitted_subtrees
                .as_ref()
                .unwrap()
                .iter()
                .map(|s| &s.base)
                .collect::<Vec<_>>()
        ),
        r#"[DNSName("localhost")]"#
    );
    assert!(names.value.excluded_subtrees.is_none());
    let not_before = cert.validity().not_before.timestamp();
    let not_after = cert.validity().not_after.timestamp();
    let now = unix_now();
    assert!(not_before <= now && now - not_before <= 3600 + 60);
    assert!(not_after >= now && not_after - now <= CA_VALIDITY.as_secs() as i64 + 60);
    assert!(cert.key_usage().unwrap().unwrap().value.key_cert_sign());
}

#[tokio::test]
async fn cancelling_one_admission_closes_only_its_sockets() {
    let pki = TestPki::new("unused");
    let (_ca, config) = config(&["api.test"], &[], &roots(&pki));
    let mut recorder_a = Recorder::start(b"never-read".to_vec()).await;
    let mut recorder_b = Recorder::start(b"reply-b".to_vec()).await;
    let proxy_a = serve_one(config.clone()).await;
    let proxy_b = serve_one(config).await;
    let mut client_a = open_tunnel(
        proxy_a.addr,
        &format!("127.0.0.1:{}", recorder_a.addr.port()),
    )
    .await;
    let mut client_b = open_tunnel(
        proxy_b.addr,
        &format!("127.0.0.1:{}", recorder_b.addr.port()),
    )
    .await;
    client_a.write_all(b"a1").await.unwrap();
    client_b.write_all(b"b1").await.unwrap();
    recorder_a.wait_received(2).await;
    recorder_b.wait_received(2).await;

    proxy_a.cancel().await;
    // Both sockets the cancelled admission owned are closed ...
    assert_eq!(read_to_end(&mut client_a).await, b"");
    assert_eq!(recorder_a.bytes().await, b"a1");
    // ... and the other tunnel on the same configuration is untouched.
    client_b.write_all(b"b2").await.unwrap();
    client_b.shutdown().await.unwrap();
    assert_eq!(read_to_end(&mut client_b).await, b"reply-b");
    assert_eq!(recorder_b.bytes().await, b"b1b2");
    assert_eq!(
        tunneled(proxy_b.outcome().await).end,
        TunnelEnd::Closed {
            client_to_upstream: 4,
            upstream_to_client: 7
        }
    );
}

#[tokio::test]
async fn stalled_clients_are_bounded_by_the_admission_deadline() {
    let pki = TestPki::new("unused");
    let (_ca, config) = config(&["localhost"], &[], &roots(&pki));
    let config = Arc::new(
        Arc::try_unwrap(config)
            .unwrap()
            .with_timeouts(Duration::from_millis(300), Duration::from_millis(300)),
    );

    // Silent after connecting.
    let proxy = serve_one(config.clone()).await;
    let _silent = TcpStream::connect(proxy.addr).await.unwrap();
    let started = Instant::now();
    assert_eq!(
        failed(proxy.outcome().await),
        AdmissionFailure {
            class: FailureClass::Timeout,
            host: None
        }
    );
    assert!(started.elapsed() < Duration::from_secs(3));

    // Half a head.
    let proxy = serve_one(config.clone()).await;
    let mut partial = TcpStream::connect(proxy.addr).await.unwrap();
    partial
        .write_all(b"CONNECT localhost:443 HTTP/1.1\r\n")
        .await
        .unwrap();
    assert_eq!(failed(proxy.outcome().await).class, FailureClass::Timeout);

    // Authenticated CONNECT to an approved host, then no ClientHello.
    let proxy = serve_one(config).await;
    let _stalled = open_tunnel(proxy.addr, "localhost:443").await;
    assert_eq!(
        failed(proxy.outcome().await),
        AdmissionFailure {
            class: FailureClass::Timeout,
            host: Some("localhost".into())
        }
    );
}

#[test]
fn admission_config_validates_hosts_and_redacts_the_credential() {
    let roots = UpstreamRoots::from_certificates([TestPki::new("r").root()]).unwrap();
    let ca = ProxyCa::generate(&["api.test".into(), "chat.test".into()]).unwrap();
    let approved = |host: &str| ApprovedHost {
        host: host.into(),
        policy: policy(),
    };
    let build = |approved: Vec<ApprovedHost>, auth: Vec<&str>| {
        AdmissionConfig::new(
            approved,
            auth.into_iter().map(String::from).collect(),
            credential(),
            &ca,
            &roots,
        )
        .map(|_| ())
    };
    assert_eq!(build(vec![approved("api.test")], vec!["auth.test"]), Ok(()));
    assert_eq!(
        build(vec![approved("api.test")], vec!["api.test"]),
        Err(AdmissionConfigError::AmbiguousHost)
    );
    assert_eq!(
        build(vec![approved("api.test"), approved("api.test")], vec![]),
        Err(AdmissionConfigError::DuplicateHost)
    );
    assert_eq!(
        build(vec![], vec!["auth.test", "auth.test"]),
        Err(AdmissionConfigError::DuplicateHost)
    );
    assert_eq!(
        build(vec![approved("other.test")], vec![]),
        Err(AdmissionConfigError::Tls(CaError::HostNotPermitted))
    );
    for bad in [
        "API.test",
        "api.test:443",
        "https://api.test",
        "api.test.",
        "*.api.test",
        "",
        "127.0.0.1",
        "user@api.test",
        "-a.test",
        "a..test",
    ] {
        assert_eq!(
            build(vec![approved(bad)], vec![]),
            Err(AdmissionConfigError::InvalidHost),
            "approved {bad:?}"
        );
        assert_eq!(
            build(vec![], vec![bad]),
            Err(AdmissionConfigError::InvalidHost),
            "auth {bad:?}"
        );
    }
    assert_eq!(
        ProxyCa::generate(&[]).unwrap_err(),
        CaError::NoApprovedHosts
    );
    assert_eq!(
        ProxyCa::generate(&["Api.test".into()]).unwrap_err(),
        CaError::InvalidHost
    );
    assert_eq!(
        UpstreamRoots::from_certificates([CertificateDer::from(b"not a certificate".to_vec())])
            .unwrap_err(),
        CaError::InvalidRoot
    );

    for bad in [
        "short",
        "0123456789ABCDEF0123456789ABCDEF",
        "0123456789abcdef0123456789abcdeg",
        "0123456789abcdef0123456789abcdef0",
    ] {
        assert_eq!(
            ProxyCredential::from_secret(bad.into()).unwrap_err(),
            AdmissionConfigError::InvalidCredential
        );
    }
    let generated = ProxyCredential::generate();
    let secret = generated.expose_secret().to_owned();
    assert!(ProxyCredential::from_secret(secret.clone()).is_ok());
    assert_ne!(secret, ProxyCredential::generate().expose_secret());

    let config = AdmissionConfig::new(
        vec![approved("api.test")],
        vec!["auth.test".into()],
        generated,
        &ca,
        &roots,
    )
    .unwrap();
    let debug = format!("{config:?} {ca:?} {:?}", credential());
    assert_absent(
        "Debug",
        debug.as_bytes(),
        &[&secret, SECRET, "PRIVATE", "BEGIN"],
    );
    assert!(debug.contains("api.test") && debug.contains("auth.test"));
}
