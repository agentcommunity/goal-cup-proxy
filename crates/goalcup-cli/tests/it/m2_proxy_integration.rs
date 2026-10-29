//! The recording proxy root composed from the real leaves — admission and the per-launch CA,
//! `relay_http`, `relay_websocket` — over a real loopback listener, real TLS on both legs, and
//! a bounded direct `CaptureSink` whose worker validates what it retains.
//!
//! Every client trusts only what it must: an intercepted client trusts only the recording proxy's
//! per-launch CA, and a tunnelled one only the synthetic upstream's test CA. Nothing here is
//! mocked between the listener and the sink, and nothing leaves loopback.

use crate::common::m2_proxy_support;
use crate::common::m2_tls_support::{client_config, client_hello};

use goalcup_cli::proxy::api::CaptureEmitter;
use goalcup_cli::proxy::headers::HEADERS_HINT;
use goalcup_cli::proxy::tls::CA_CERT_FILE_NAME;
use goalcup_cli::proxy::{MAX_CONNECTION_TASKS, ProducerClose, StartError};
use goalcup_protocol::capture::{
    CaptureSource, ConnectionEvent, ConnectionOutcome, EndReason, EpochAdmission, FrameEvent,
    GapReason, MAX_OPEN_CONNECTIONS, MAX_OPEN_STREAMS, SubmitResult, is_capture_id,
};
use m2_proxy_support::*;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// One chunk of a chunked body.
fn chunk(data: &[u8]) -> Vec<u8> {
    let mut out = format!("{:x}\r\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\r\n");
    out
}

/// A fully clean run: a normal drain, no capture loss, no failed connection.
#[track_caller]
fn assert_all_clean(report: &goalcup_cli::proxy::ProxyStopReport) {
    assert!(
        report.is_clean_drain() && !report.capture_loss() && report.transport_failures() == 0,
        "{report:?}"
    );
}

/// A header stream (`http-head-v1`): the masked head of an exchange, beside its body stream.
fn is_head(view: &StreamView) -> bool {
    view.meta.protocol_hint.as_deref() == Some(HEADERS_HINT)
}

/// The chain's streams without the header streams: the bodies, which these tests count.
fn bodies(frames: &[goalcup_protocol::capture::CaptureFrame]) -> Vec<StreamView> {
    streams(frames)
        .into_iter()
        .filter(|v| !is_head(v))
        .collect()
}

fn stream_with<'a>(views: &'a [StreamView], marker: &[u8]) -> &'a StreamView {
    let found: Vec<&StreamView> = views
        .iter()
        .filter(|v| contains(&v.bytes, marker))
        .collect();
    assert_eq!(
        found.len(),
        1,
        "{:?} is in {} streams",
        String::from_utf8_lossy(marker),
        found.len()
    );
    found[0]
}

/// Lets this process hold `needed` descriptors, up to its hard limit. A test that keeps a
/// hundred relayed connections open holds four sockets for each, past macOS's default soft
/// limit of 256.
fn allow_descriptors(needed: u64) {
    #[cfg(unix)]
    {
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: `limit` is a valid, writable `rlimit` for both calls.
        unsafe {
            if libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) == 0 && limit.rlim_cur < needed {
                limit.rlim_cur = needed.min(limit.rlim_max);
                libc::setrlimit(libc::RLIMIT_NOFILE, &limit);
            }
        }
    }
}

fn begins(calls: &[Call]) -> Vec<(String, EpochAdmission)> {
    calls
        .iter()
        .filter_map(|c| match c {
            Call::Begin { producer, answer } => Some((producer.clone(), *answer)),
            _ => None,
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn startup_admits_configured_producer_before_emitter() {
    let clock = clock_start();
    // The emitter's constructor assumes an admitted epoch: it asks the sink nothing.
    let idle = DirectSink::new(QUEUE_BYTES);
    let emitter = CaptureEmitter::new(PRODUCER.to_string(), idle.clone());
    assert!(
        idle.calls().is_empty(),
        "CaptureEmitter::new called the sink"
    );
    drop(emitter);

    let pki = TestPki::new("upstream test CA");
    let mut upstream = Upstream::start(&pki, &["localhost"]).await;
    let target = format!("localhost:{}", upstream.addr.port());

    // Pending, then Ready: the same ID is polled, and no connection is served before Ready.
    let sink = DirectSink::new(QUEUE_BYTES);
    sink.hold_admission(true);
    let dir = tempfile::tempdir().unwrap();
    let listener = loopback_listener();
    let addr = listener.local_addr().unwrap();
    let config = proxy_config(listener, dir.path(), &RigConfig::new(&pki), sink.clone());
    let starting = tokio::spawn(goalcup_cli::proxy::start(config));
    let mut early = TcpStream::connect(addr).await.unwrap();
    early.write_all(&connect_head(&target)).await.unwrap();
    let mut byte = [0u8; 1];
    assert!(
        tokio::time::timeout(Duration::from_millis(300), early.read(&mut byte))
            .await
            .is_err(),
        "a connection was served before the producer was admitted"
    );
    eventually("pending polls", || begins(&sink.calls()).len() >= 3).await;
    assert!(
        sink.calls().iter().all(|call| matches!(
            call,
            Call::Begin { producer, answer: EpochAdmission::Pending } if producer == PRODUCER
        )),
        "something other than a Pending poll of the configured ID: {:?}",
        sink.calls()
    );
    assert!(!starting.is_finished(), "start returned while Pending");
    sink.hold_admission(false);
    let handle = within("start", starting)
        .await
        .unwrap()
        .expect("the recording proxy starts once admitted");
    assert_eq!(handle.producer(), PRODUCER);
    assert_eq!(handle.health().producers_admitted, 1);

    // The CA file is a certificate only, owner-only, in the given directory.
    assert_eq!(handle.ca_certificate(), dir.path().join(CA_CERT_FILE_NAME));
    let pem = std::fs::read_to_string(handle.ca_certificate()).unwrap();
    assert!(pem.contains("BEGIN CERTIFICATE") && !pem.contains("PRIVATE KEY"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(handle.ca_certificate())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    // The early client is served now, through the one emitter.
    let (head, rest) = within("early CONNECT answer", read_head(&mut early)).await;
    assert!(head.starts_with(b"HTTP/1.1 200 ") && rest.is_empty());
    let mut client = tls_over(early, &[handle.ca_certificate_der().clone()], "localhost").await;
    let mut up = upstream.accept().await.tls;
    exchange(&mut client, &mut up, "/v1/responses", b"{\"n\":1}", b"{}").await;
    close_both(client, up).await;
    let report = within("stop", handle.stop()).await;
    assert_all_clean(&report);
    sink.settle().await;

    let calls = sink.calls();
    let first_submit = calls
        .iter()
        .position(|c| matches!(c, Call::Submit { .. }))
        .expect("the exchange was submitted");
    let last_begin = calls
        .iter()
        .rposition(|c| matches!(c, Call::Begin { .. }))
        .unwrap();
    assert!(
        last_begin < first_submit,
        "a frame preceded Ready: {calls:?}"
    );
    assert!(matches!(
        &calls[last_begin],
        Call::Begin { producer, answer: EpochAdmission::Ready } if producer == PRODUCER
    ));
    assert!(matches!(
        &calls[first_submit],
        Call::Submit { producer, seq: 0, .. } if producer == PRODUCER
    ));
    let frames = sink.frames();
    check_frames(&frames);
    check_clock(&frames, clock, clock_now());
    assert!(frames.iter().all(|f| f.stream.producer == PRODUCER));
    assert_eq!(
        sink.retained().last(),
        Some(&Retained::ProducerClosed {
            producer: PRODUCER.into(),
            final_seq: Some(frames.last().unwrap().seq),
        })
    );

    // RetryFresh, then Ready: a fresh ID that the sink, the emitter and the handle agree on.
    let sink = DirectSink::new(QUEUE_BYTES);
    sink.script([EpochAdmission::RetryFresh]);
    let mut rig = Rig::start(RigConfig::new(&pki), sink.clone()).await;
    let fresh = rig.handle().producer().to_string();
    assert!(is_capture_id(&fresh) && fresh != PRODUCER, "{fresh}");
    assert_eq!(
        begins(&sink.calls()),
        vec![
            (PRODUCER.to_string(), EpochAdmission::RetryFresh),
            (fresh.clone(), EpochAdmission::Ready),
        ]
    );
    let mut client = rig.client(upstream.addr.port()).await;
    let mut up = upstream.accept().await.tls;
    exchange(&mut client, &mut up, "/v1/responses", b"{\"n\":2}", b"{}").await;
    close_both(client, up).await;
    rig.wait_closed(1).await;
    assert_eq!(rig.stop().await.producer_close, ProducerClose::Queued);
    sink.settle().await;
    let frames = sink.frames();
    assert!(!frames.is_empty());
    assert!(frames.iter().all(|f| f.stream.producer == fresh));
    assert!(sink.calls().iter().all(|c| match c {
        Call::Submit { producer, .. } | Call::Close { producer, .. } => producer == &fresh,
        Call::Begin { .. } => true,
    }));

    // Disabled: startup fails before any frame; the listener and the CA file are gone.
    let sink = DirectSink::new(QUEUE_BYTES);
    sink.script([EpochAdmission::Disabled(GapReason::StorageFull)]);
    let dir = tempfile::tempdir().unwrap();
    let listener = loopback_listener();
    let addr = listener.local_addr().unwrap();
    let config = proxy_config(listener, dir.path(), &RigConfig::new(&pki), sink.clone());
    let refused = within("start", goalcup_cli::proxy::start(config)).await;
    assert_eq!(
        refused.unwrap_err(),
        StartError::CaptureDisabled(GapReason::StorageFull)
    );
    assert!(!dir.path().join(CA_CERT_FILE_NAME).exists());
    assert!(
        TcpStream::connect(addr).await.is_err(),
        "listener still open"
    );
    assert_eq!(sink.calls().len(), 1);

    // Pending past the startup deadline: refused at the deadline, same ID, no busy loop.
    let sink = DirectSink::new(QUEUE_BYTES);
    sink.hold_admission(true);
    let dir = tempfile::tempdir().unwrap();
    let mut short = RigConfig::new(&pki);
    short.startup = Duration::from_millis(300);
    let config = proxy_config(loopback_listener(), dir.path(), &short, sink.clone());
    let started = Instant::now();
    let refused = within("start", goalcup_cli::proxy::start(config)).await;
    let elapsed = started.elapsed();
    assert_eq!(refused.unwrap_err(), StartError::StartupDeadline);
    assert!(
        elapsed >= Duration::from_millis(300) && elapsed < Duration::from_millis(800),
        "{elapsed:?}"
    );
    let polls = begins(&sink.calls());
    assert!(
        (3..20).contains(&polls.len()),
        "{} polls in 300 ms",
        polls.len()
    );
    assert!(
        polls
            .iter()
            .all(|(p, a)| p == PRODUCER && *a == EpochAdmission::Pending)
    );
    assert!(!dir.path().join(CA_CERT_FILE_NAME).exists());
    assert!(
        !sink
            .calls()
            .iter()
            .any(|c| matches!(c, Call::Submit { .. }))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sse_prefix_reaches_client_before_upstream_finishes() {
    let clock = clock_start();
    let pki = TestPki::new("upstream test CA");
    let mut upstream = Upstream::start(&pki, &["localhost"]).await;
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut rig = Rig::start(RigConfig::new(&pki), sink.clone()).await;

    let mut client = rig.client(upstream.addr.port()).await;
    let conn = upstream.accept().await;
    assert_eq!(conn.sni.as_deref(), Some("localhost"));
    let mut up = conn.tls;

    let body = br#"{"model":"m","input":"hello","stream":true}"#;
    let request = post(
        "/v1/responses?key=SENTINEL-QUERY",
        body,
        "Authorization: Bearer SENTINEL-HEADER-TOKEN\r\n",
    );
    send(&mut client, &request).await;
    let (head, got) = read_request(&mut up).await;
    assert_eq!([head, got].concat(), request);

    let response_head = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\nTransfer-Encoding: chunked\r\n\r\n";
    let first = chunk(b"data: one\n\n");
    send(&mut up, &[&response_head[..], &first].concat()).await;
    let mut seen = Vec::new();
    read_until(&mut client, &mut seen, b"data: one\n\n").await;
    // The upstream has not finished the body (its next chunk and terminator are unwritten),
    // and the direct sink already holds the first event on a still-open response stream.
    sink.settle().await;
    let views = bodies(&sink.frames());
    let response = views
        .iter()
        .find(|v| v.source == CaptureSource::WireResponse)
        .expect("response stream open before the body finished");
    assert_eq!(response.bytes, b"data: one\n\n");
    assert_eq!(response.end, None);

    let rest = [chunk(b"data: two\n\n"), b"0\r\n\r\n".to_vec()].concat();
    send(&mut up, &rest).await;
    let expected = [&response_head[..], &first, &rest].concat();
    read_at_least(&mut client, &mut seen, expected.len()).await;
    assert_eq!(
        seen, expected,
        "the client saw other bytes than the upstream wrote"
    );
    close_both(client, up).await;
    rig.wait_closed(1).await;
    let report = rig.stop().await;
    assert_all_clean(&report);
    sink.settle().await;

    eprintln!("evidence sse trace:\n{}", trace(&sink.retained()));
    let frames = sink.frames();
    check_frames(&frames);
    check_clock(&frames, clock, clock_now());
    let controls = controls(&frames);
    assert_eq!(controls.len(), 2, "{controls:?}");
    let connection = controls[0].connection.clone();
    assert!(is_capture_id(&connection));
    assert_eq!(
        controls[0].event,
        ConnectionEvent::Open {
            alpn: Some("http/1.1".into()),
            already_open: false
        }
    );
    assert_eq!(controls[1].connection, connection);
    assert_eq!(
        controls[1].event,
        ConnectionEvent::Close {
            outcome: ConnectionOutcome::Complete
        }
    );
    let views = bodies(&frames);
    assert_eq!(views.len(), 2);
    let (req, resp) = (&views[0], &views[1]);
    assert_eq!(req.source, CaptureSource::WireRequest);
    assert_eq!(req.connection(), connection);
    assert_eq!(req.exchange(), 1);
    assert_eq!(req.meta.method.as_deref(), Some("POST"));
    assert_eq!(req.meta.host.as_deref(), Some("localhost"));
    assert_eq!(req.meta.path.as_deref(), Some("/v1/responses"));
    assert_eq!(req.meta.protocol_hint.as_deref(), Some("openai-responses"));
    assert_eq!(req.bytes, body);
    assert_eq!(req.end, Some(EndReason::Complete));
    assert_eq!(resp.source, CaptureSource::WireResponse);
    assert_eq!(
        (resp.connection(), resp.exchange()),
        (connection.as_str(), 1)
    );
    assert_eq!(resp.meta.status, Some(200));
    assert_eq!(resp.meta.content_type.as_deref(), Some("text/event-stream"));
    assert_eq!(resp.bytes, b"data: one\n\ndata: two\n\n");
    assert_eq!(resp.end, Some(EndReason::Complete));
    assert!(controls[0].at < req.open_at && resp.end_at.unwrap() < controls[1].at);
    assert_eq!(
        sink.retained().last(),
        Some(&Retained::ProducerClosed {
            producer: PRODUCER.into(),
            final_seq: Some(frames.last().unwrap().seq),
        })
    );
    let retained = sink.retained();
    for sentinel in ["SENTINEL-HEADER-TOKEN", "SENTINEL-QUERY", SECRET] {
        assert!(
            !retained_contains(&retained, sentinel.as_bytes()),
            "{sentinel}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rejected_websocket_upgrade_stays_http_capture() {
    let clock = clock_start();
    let pki = TestPki::new("upstream test CA");
    let mut upstream = Upstream::start(&pki, &["localhost"]).await;
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut rig = Rig::start(RigConfig::new(&pki), sink.clone()).await;
    let mut client = rig.client(upstream.addr.port()).await;
    let mut up = upstream.accept().await.tls;

    let upgrade = ws_upgrade_request("/v1/realtime");
    send(&mut client, &upgrade).await;
    let (head, rest) = within("upgrade request", read_head(&mut up)).await;
    assert_eq!((head, rest), (upgrade, Vec::new()));
    let refusal = b"HTTP/1.1 426 Upgrade Required\r\nContent-Type: text/plain\r\nContent-Length: 11\r\n\r\nupgrade-now";
    send(&mut up, refusal).await;
    let mut seen = Vec::new();
    read_at_least(&mut client, &mut seen, refusal.len()).await;
    assert_eq!(seen, refusal);
    // The connection is still HTTP.
    exchange(&mut client, &mut up, "/v1/responses", b"after-426", b"{}").await;
    close_both(client, up).await;
    rig.wait_closed(1).await;
    assert_all_clean(&rig.stop().await);
    sink.settle().await;

    let frames = sink.frames();
    check_frames(&frames);
    check_clock(&frames, clock, clock_now());
    assert!(
        !controls(&frames)
            .iter()
            .any(|c| c.event == ConnectionEvent::UpgradedWebSocket)
    );
    let views = bodies(&frames);
    assert!(views.iter().all(|v| matches!(
        v.source,
        CaptureSource::WireRequest | CaptureSource::WireResponse
    )));
    let rejected = views
        .iter()
        .find(|v| v.source == CaptureSource::WireResponse && v.exchange() == 1)
        .unwrap();
    assert_eq!(rejected.meta.status, Some(426));
    assert_eq!(rejected.bytes, b"upgrade-now");
    assert_eq!(rejected.end, Some(EndReason::Complete));
    let later = stream_with(&views, b"after-426");
    assert_eq!(later.exchange(), 2);
    assert_eq!(later.connection(), rejected.connection());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unknown_protocol_hint_is_captured_without_a_decoder() {
    let clock = clock_start();
    let pki = TestPki::new("upstream test CA");
    let mut upstream = Upstream::start(&pki, &["localhost"]).await;
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut rig = Rig::start(RigConfig::new(&pki), sink.clone()).await;
    let mut client = rig.client(upstream.addr.port()).await;
    let mut up = upstream.accept().await.tls;
    exchange(
        &mut client,
        &mut up,
        "/v1/unknown",
        b"\x00opaque-request\xff",
        b"\x01opaque-response\xfe",
    )
    .await;
    close_both(client, up).await;
    rig.wait_closed(1).await;
    assert_all_clean(&rig.stop().await);
    sink.settle().await;
    let frames = sink.frames();
    check_frames(&frames);
    check_clock(&frames, clock, clock_now());
    let views = bodies(&frames);
    assert_eq!(views.len(), 2);
    for view in &views {
        assert_eq!(
            view.meta.protocol_hint.as_deref(),
            Some("vendor-unknown-v9")
        );
        assert_eq!(view.end, Some(EndReason::Complete));
    }
    assert_eq!(views[0].bytes, b"\x00opaque-request\xff");
    assert_eq!(views[1].bytes, b"\x01opaque-response\xfe");
}

/// One `GET path` on a keep-alive connection, answered with `reply`; both ends are checked byte
/// for byte.
async fn get_exchange(
    client: &mut ClientTls,
    upstream: &mut UpstreamTls,
    path: &str,
    reply: &[u8],
) {
    let request = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").into_bytes();
    send(client, &request).await;
    let (head, got) = read_request(upstream).await;
    assert_eq!([head, got].concat(), request, "upstream saw other bytes");
    let response = ok_response("application/json", reply);
    send(upstream, &response).await;
    let mut seen = Vec::new();
    read_at_least(client, &mut seen, response.len()).await;
    assert_eq!(seen, response, "client saw other bytes");
}

/// Every request on an opened host is recorded, whatever its path and method: the host policy
/// only picks the decoder hint. The policy is the production rule for a URL with no path
/// (`ModelHost::approved`), not a single-path test policy. So `POST /v1/messages` gets its
/// hint from the `/v1` wire row, `GET /v1/models` is recorded with no hint through the `/v1/`
/// base-prefix row, and `POST /x/other`, outside every row, is recorded with no hint too.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_request_on_an_opened_host_is_recorded() {
    let clock = clock_start();
    let pki = TestPki::new("upstream test CA");
    let mut upstream = Upstream::start(&pki, &["localhost"]).await;
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut config = RigConfig::new(&pki);
    config.approved = vec![
        goalcup_cli::model_host::ModelHost {
            host: "localhost".into(),
            port: 443,
            base_path: String::new(),
            from: "test config".into(),
            edit: None,
        }
        .approved(),
    ];
    let mut rig = Rig::start(config, sink.clone()).await;
    let mut client = rig.client(upstream.addr.port()).await;
    let mut up = upstream.accept().await.tls;
    exchange(
        &mut client,
        &mut up,
        "/v1/messages",
        b"{\"messages\":[]}",
        b"{\"id\":\"m\"}",
    )
    .await;
    get_exchange(&mut client, &mut up, "/v1/models", b"{\"data\":[]}").await;
    exchange(
        &mut client,
        &mut up,
        "/x/other",
        b"outside-every-row",
        b"{\"ok\":true}",
    )
    .await;
    close_both(client, up).await;
    rig.wait_closed(1).await;
    let recorded = rig
        .handle()
        .recorded_requests()
        .load(std::sync::atomic::Ordering::SeqCst);
    let model_calls = rig
        .handle()
        .generation_requests()
        .load(std::sync::atomic::Ordering::SeqCst);
    assert_all_clean(&rig.stop().await);
    sink.settle().await;

    let frames = sink.frames();
    check_frames(&frames);
    check_clock(&frames, clock, clock_now());
    let views = bodies(&frames);
    let requests: Vec<&StreamView> = views
        .iter()
        .filter(|v| v.source == CaptureSource::WireRequest)
        .collect();
    let responses = views
        .iter()
        .filter(|v| v.source == CaptureSource::WireResponse)
        .count();
    assert_eq!((requests.len(), responses), (3, 3), "{views:?}");
    assert_eq!(recorded, 3);
    // Only the hinted POST is a model call: the GET model list and the POST outside every
    // row are not.
    assert_eq!(model_calls, 1);
    let paths: Vec<&str> = requests
        .iter()
        .map(|v| v.meta.path.as_deref().expect("a retained path"))
        .collect();
    assert_eq!(paths, ["/v1/messages", "/v1/models", "/x/other"]);
    let hints: Vec<Option<&str>> = requests
        .iter()
        .map(|v| v.meta.protocol_hint.as_deref())
        .collect();
    assert_eq!(hints, [Some("anthropic-messages"), None, None]);
    let exchanges: Vec<u64> = requests.iter().map(|v| v.exchange()).collect();
    assert_eq!(exchanges, [1, 2, 3]);
    assert_eq!(requests[2].bytes, b"outside-every-row");
}

/// A refresh-token request to a token endpoint on an opened host is recorded like any other
/// request: the proxy hands its body over unchanged. Masking happens in the daemon's redactor,
/// before a segment is written; that is pinned on decoded segment bytes in the daemon's
/// `capture_redaction` suite. Sign-in hosts stay sealed (auth hosts), which is a separate rule.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_token_refresh_on_an_opened_host_is_recorded() {
    let clock = clock_start();
    let pki = TestPki::new("upstream test CA");
    let mut upstream = Upstream::start(&pki, &["localhost"]).await;
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut rig = Rig::start(RigConfig::new(&pki), sink.clone()).await;
    let mut client = rig.client(upstream.addr.port()).await;
    let mut up = upstream.accept().await.tls;
    let body = br#"{"grant_type":"refresh_token","refresh_token":"rt-0123456789abcdefghij"}"#;
    exchange(
        &mut client,
        &mut up,
        "/oauth/token",
        body,
        br#"{"access_token":"at-0123456789abcdefghij"}"#,
    )
    .await;
    close_both(client, up).await;
    rig.wait_closed(1).await;
    let model_calls = rig
        .handle()
        .generation_requests()
        .load(std::sync::atomic::Ordering::SeqCst);
    assert_all_clean(&rig.stop().await);
    sink.settle().await;

    let frames = sink.frames();
    check_frames(&frames);
    check_clock(&frames, clock, clock_now());
    assert_eq!(model_calls, 0, "a token refresh is no model call");
    let views = bodies(&frames);
    let refresh = views
        .iter()
        .find(|v| v.source == CaptureSource::WireRequest)
        .expect("the refresh request is recorded");
    assert_eq!(refresh.meta.path.as_deref(), Some("/oauth/token"));
    assert_eq!(refresh.meta.protocol_hint, None);
    assert_eq!(refresh.bytes, body);
}

/// Every recorded exchange on an opened host has one request and one response header stream: its
/// masked head, under the header hint, with no method. Two exchanges give four, the credentials
/// are masked in them, and no credential value reaches any frame. The model-call count is the
/// same as without them: only the hinted POST counts.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_recorded_exchange_has_header_streams() {
    let clock = clock_start();
    let pki = TestPki::new("upstream test CA");
    let mut upstream = Upstream::start(&pki, &["localhost"]).await;
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut config = RigConfig::new(&pki);
    config.approved = vec![
        goalcup_cli::model_host::ModelHost {
            host: "localhost".into(),
            port: 443,
            base_path: String::new(),
            from: "test config".into(),
            edit: None,
        }
        .approved(),
    ];
    let mut rig = Rig::start(config, sink.clone()).await;
    let mut client = rig.client(upstream.addr.port()).await;
    let mut up = upstream.accept().await.tls;
    let extra = "Authorization: Bearer SECRET-bearer-0123\r\nx-api-key: SECRET-key-4567\r\nUser-Agent: example-agent/2.1.312\r\n";
    let request = post(
        "/v1/messages?key=SECRET-query-89",
        b"{\"messages\":[]}",
        extra,
    );
    send(&mut client, &request).await;
    let (head, got) = read_request(&mut up).await;
    assert_eq!([head, got].concat(), request, "upstream saw other bytes");
    let response = ok_response("application/json", b"{\"id\":\"m\"}");
    send(&mut up, &response).await;
    let mut seen = Vec::new();
    read_at_least(&mut client, &mut seen, response.len()).await;
    assert_eq!(seen, response, "client saw other bytes");
    get_exchange(&mut client, &mut up, "/v1/models", b"{\"data\":[]}").await;
    close_both(client, up).await;
    rig.wait_closed(1).await;
    let model_calls = rig
        .handle()
        .generation_requests()
        .load(std::sync::atomic::Ordering::SeqCst);
    assert_all_clean(&rig.stop().await);
    sink.settle().await;

    let frames = sink.frames();
    check_frames(&frames);
    check_clock(&frames, clock, clock_now());
    assert_eq!(model_calls, 1, "a header stream is never a model call");
    let views = streams(&frames);
    let heads: Vec<&StreamView> = views
        .iter()
        .filter(|v| v.meta.protocol_hint.as_deref() == Some(HEADERS_HINT))
        .collect();
    let request_heads = heads
        .iter()
        .filter(|v| v.source == CaptureSource::WireRequest)
        .count();
    let response_heads = heads
        .iter()
        .filter(|v| v.source == CaptureSource::WireResponse)
        .count();
    assert_eq!((request_heads, response_heads), (2, 2), "{views:?}");
    for head in &heads {
        assert_eq!(head.meta.method, None, "a header stream has no method");
        assert_eq!(head.end, Some(EndReason::Complete));
        assert!(head.gaps.is_empty(), "{:?}", head.gaps);
    }
    let first = heads
        .iter()
        .find(|v| v.source == CaptureSource::WireRequest && v.exchange() == 1)
        .expect("the first request has a head stream");
    let text = String::from_utf8(first.bytes.clone()).expect("a masked head is text");
    assert!(
        text.starts_with("POST /v1/messages?key=[masked] HTTP/1.1\r\n"),
        "{text}"
    );
    assert!(text.contains("Authorization: [masked]\r\n"), "{text}");
    assert!(text.contains("x-api-key: [masked]\r\n"), "{text}");
    assert!(
        text.contains("User-Agent: example-agent/2.1.312\r\n"),
        "{text}"
    );
    // The decoded frames hold the masked heads and no credential value, in any stream.
    assert!(!retained_contains(&sink.retained(), b"SECRET"));
    let body = views
        .iter()
        .find(|v| {
            v.source == CaptureSource::WireRequest
                && v.meta.protocol_hint.as_deref() == Some("anthropic-messages")
        })
        .expect("the model request body is recorded");
    assert_eq!(body.bytes, b"{\"messages\":[]}");
}

/// An exchange the recorder cannot retain (a method over the metadata limit) is still relayed
/// unchanged and leaves one `LimitExceeded` loss in the chain: it never vanishes silently. The
/// connection carries on, and the next exchange is recorded under its own exchange number.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_exchange_the_recorder_cannot_retain_is_a_loss() {
    let clock = clock_start();
    let pki = TestPki::new("upstream test CA");
    let mut upstream = Upstream::start(&pki, &["localhost"]).await;
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut rig = Rig::start(RigConfig::new(&pki), sink.clone()).await;
    let mut client = rig.client(upstream.addr.port()).await;
    let mut up = upstream.accept().await.tls;
    let method = "M".repeat(65);
    let request = format!(
        "{method} /v1/responses HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2\r\n\r\nhi"
    )
    .into_bytes();
    send(&mut client, &request).await;
    let (head, got) = read_request(&mut up).await;
    assert_eq!([head, got].concat(), request, "upstream saw other bytes");
    let response = ok_response("text/plain", b"ok");
    send(&mut up, &response).await;
    let mut seen = Vec::new();
    read_at_least(&mut client, &mut seen, response.len()).await;
    assert_eq!(seen, response, "client saw other bytes");
    exchange(
        &mut client,
        &mut up,
        "/v1/responses",
        b"{\"after\":1}",
        b"{}",
    )
    .await;
    close_both(client, up).await;
    rig.wait_closed(1).await;
    let handle = rig.handle();
    let model = handle
        .model_requests()
        .load(std::sync::atomic::Ordering::SeqCst);
    let recorded = handle
        .recorded_requests()
        .load(std::sync::atomic::Ordering::SeqCst);
    let report = rig.stop().await;
    assert!(
        report.is_clean_drain() && report.transport_failures() == 0,
        "{report:?}"
    );
    sink.settle().await;

    let frames = sink.frames();
    check_frames(&frames);
    check_clock(&frames, clock, clock_now());
    assert_eq!((model, recorded), (2, 1));
    assert_eq!(
        losses(&frames)
            .into_iter()
            .map(|(_, r)| r)
            .collect::<Vec<_>>(),
        vec![GapReason::LimitExceeded]
    );
    let views = bodies(&frames);
    assert_eq!(views.len(), 2, "{views:?}");
    assert!(views.iter().all(|v| v.exchange() == 2));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn direct_sink_recovery_preserves_relay() {
    let clock = clock_start();
    let pki = TestPki::new("upstream test CA");
    let mut upstream = Upstream::start(&pki, &["localhost"]).await;
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut rig = Rig::start(RigConfig::new(&pki), sink.clone()).await;
    let mut client = rig.client(upstream.addr.port()).await;
    let mut up = upstream.accept().await.tls;

    exchange(&mut client, &mut up, "/v1/responses", b"before-loss", b"{}").await;

    // The sink refuses everything; the relay keeps every byte flowing both ways.
    sink.set_pressure(true);
    let body = [&b"DURING-LOSS"[..], &vec![b'q'; 300 * 1024]].concat();
    let reply = [&b"DURING-LOSS-REPLY"[..], &vec![b'r'; 300 * 1024]].concat();
    exchange(&mut client, &mut up, "/v1/responses", &body, &reply).await;
    assert!(sink.calls().iter().any(|c| matches!(
        c,
        Call::Submit {
            answer: SubmitResult::Disabled(GapReason::QueueFull),
            ..
        }
    )));

    // Readiness returns; the next request boundary recovers under a fresh producer.
    sink.set_pressure(false);
    exchange(
        &mut client,
        &mut up,
        "/v1/responses",
        b"after-recovery",
        b"{}",
    )
    .await;
    close_both(client, up).await;
    rig.wait_closed(1).await;
    let report = rig.stop().await;
    assert_eq!(report.producer_close, ProducerClose::Queued);
    assert!(
        report.is_clean_drain() && report.capture_loss(),
        "{report:?}"
    );
    assert_eq!(report.transport_failures(), 0);
    assert!(report.health.sink_refusals > 0);
    assert_eq!(report.health.producers_admitted, 2);
    sink.settle().await;

    let retained = sink.retained();
    eprintln!("evidence recovery trace:\n{}", trace(&retained));
    let frames = sink.frames();
    check_frames(&frames);
    check_clock(&frames, clock, clock_now());
    let recoveries = recoveries(&frames);
    assert_eq!(recoveries.len(), 1, "{recoveries:?}");
    let (at, previous, fresh, reason) = recoveries[0].clone();
    assert_eq!(
        (previous.as_str(), reason),
        (PRODUCER, GapReason::QueueFull)
    );
    assert!(is_capture_id(&fresh) && fresh != PRODUCER);
    // The sticky gap of the lost producer is retained, before the recovery, and stays.
    let gap_at = retained
        .iter()
        .position(|r| {
            *r == Retained::Gap {
                producer: PRODUCER.into(),
                reason: GapReason::QueueFull,
            }
        })
        .expect("sticky gap retained");
    let recovery_at = retained
        .iter()
        .position(|r| matches!(r, Retained::Frame(f) if f.seq == 0 && f.stream.producer == fresh))
        .unwrap();
    assert!(gap_at < recovery_at);
    // The surviving connection is re-observed before any new payload, same identity.
    let controls = controls(&frames);
    let original = controls[0].connection.clone();
    let reobserved = controls
        .iter()
        .find(|c| c.producer == fresh)
        .expect("re-observation");
    assert_eq!(reobserved.at, at + 1);
    assert_eq!(reobserved.connection, original);
    assert_eq!(
        reobserved.event,
        ConnectionEvent::Open {
            alpn: Some("http/1.1".into()),
            already_open: true
        }
    );
    let views = streams(&frames);
    let after = stream_with(&views, b"after-recovery");
    assert_eq!(after.key.producer, fresh);
    assert_eq!(
        (after.connection(), after.exchange()),
        (original.as_str(), 3)
    );
    assert!(after.open_at > reobserved.at);
    let before = stream_with(&views, b"before-loss");
    assert_eq!(
        (before.key.producer.as_str(), before.exchange()),
        (PRODUCER, 1)
    );
    // Nothing observed during the loss is replayed, and no exchange-2 stream exists in the
    // new epoch.
    assert!(!retained_contains(&retained, b"DURING-LOSS"));
    assert!(
        !views
            .iter()
            .any(|v| v.key.producer == fresh && v.exchange() == 2)
    );
    // Only the recovered producer closes; the lost one never does.
    assert!(retained.contains(&Retained::ProducerClosed {
        producer: fresh.clone(),
        final_seq: Some(frames.last().unwrap().seq),
    }));
    assert!(!retained.iter().any(|r| matches!(
        r,
        Retained::ProducerClosed { producer, .. } if producer == PRODUCER
    )));
    assert!(
        !retained
            .iter()
            .any(|r| matches!(r, Retained::Invalid { .. }))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn clean_producer_close_follows_connection_drain() {
    let clock = clock_start();
    let pki = TestPki::new("upstream test CA");
    let mut upstream = Upstream::start(&pki, &["localhost"]).await;

    // Ordered: the connection drains, then the producer closes; the worker validates it.
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut rig = Rig::start(RigConfig::new(&pki), sink.clone()).await;
    let mut client = rig.client(upstream.addr.port()).await;
    let mut up = upstream.accept().await.tls;
    exchange(&mut client, &mut up, "/v1/responses", b"ordered", b"{}").await;
    close_both(client, up).await;
    rig.wait_closed(1).await;
    let report = rig.stop().await;
    eprintln!("evidence ordered close: {report:?}");
    assert!(report.drained);
    assert_all_clean(&report);
    sink.settle().await;
    let retained = sink.retained();
    let frames = sink.frames();
    check_frames(&frames);
    check_clock(&frames, clock, clock_now());
    let close = controls(&frames).pop().unwrap();
    assert_eq!(
        close.event,
        ConnectionEvent::Close {
            outcome: ConnectionOutcome::Complete
        }
    );
    assert_eq!(
        close.at,
        frames.len() - 1,
        "the connection close is the last frame"
    );
    assert_eq!(
        retained.last(),
        Some(&Retained::ProducerClosed {
            producer: PRODUCER.into(),
            final_seq: Some(frames.last().unwrap().seq),
        })
    );

    // Abrupt task loss: the handle is dropped, never stopped. Every socket goes, the
    // connection's lifecycle is still recorded, and no close is ever asked for.
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut rig = Rig::start(RigConfig::new(&pki), sink.clone()).await;
    let mut client = rig.client(upstream.addr.port()).await;
    let mut up = upstream.accept().await.tls;
    exchange(&mut client, &mut up, "/v1/responses", b"abrupt", b"{}").await;
    drop(rig.handle.take());
    let (_, _) = read_to_end(&mut up).await;
    let (_, _) = read_to_end(&mut client).await;
    eventually("the cut connection records its close", || {
        controls(&sink.frames()).len() == 2
    })
    .await;
    sink.settle().await;
    let frames = sink.frames();
    check_frames(&frames);
    check_clock(&frames, clock, clock_now());
    assert_eq!(
        controls(&frames)[1].event,
        ConnectionEvent::Close {
            outcome: ConnectionOutcome::Interrupted
        }
    );
    assert!(!sink.calls().iter().any(|c| matches!(c, Call::Close { .. })));
    assert!(
        !sink
            .retained()
            .iter()
            .any(|r| matches!(r, Retained::ProducerClosed { .. }))
    );

    // Queued is not retained: with the worker stalled the stop still reports `Queued` (the
    // sink accepted it), yet no ProducerClosed exists until the worker validates it.
    let sink = DirectSink::new(QUEUE_BYTES);
    sink.block_worker(true);
    let mut rig = Rig::start(RigConfig::new(&pki), sink.clone()).await;
    let mut client = rig.client(upstream.addr.port()).await;
    let mut up = upstream.accept().await.tls;
    exchange(&mut client, &mut up, "/v1/responses", b"stalled", b"{}").await;
    close_both(client, up).await;
    rig.wait_closed(1).await;
    assert_eq!(rig.stop().await.producer_close, ProducerClose::Queued);
    assert!(sink.retained().is_empty());
    assert!(sink.queued() > 0);
    sink.block_worker(false);
    sink.settle().await;
    assert!(matches!(
        sink.retained().last(),
        Some(Retained::ProducerClosed { producer, final_seq: Some(_) }) if producer == PRODUCER
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn auth_host_and_unknown_sni_tunnels_stay_opaque() {
    let pki = TestPki::new("upstream test CA");
    let mut upstream = Upstream::start(&pki, &["localhost", "other.test"]).await;
    let port = upstream.addr.port();
    let request = post(
        "/v1/responses",
        b"SENTINEL-TUNNEL-BODY",
        "Authorization: Bearer SENTINEL-TUNNEL-TOKEN\r\n",
    );
    let reply = ok_response("application/json", b"SENTINEL-TUNNEL-REPLY");
    // A tunnelled client talks TLS to the upstream itself: it trusts only the test CA.
    async fn through(client: &mut ClientTls, up: &mut UpstreamTls, request: &[u8], reply: &[u8]) {
        send(client, request).await;
        let (head, body) = read_request(up).await;
        assert_eq!([head, body].concat(), request, "tunnel changed the request");
        send(up, reply).await;
        let mut seen = Vec::new();
        read_at_least(client, &mut seen, reply.len()).await;
        assert_eq!(seen, reply, "tunnel changed the reply");
    }

    // An authentication host stays opaque even beside an approved host.
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut config = RigConfig::new(&pki);
    config.approved = vec![approved("api.test", policy(&[("/", true, None)]))];
    config.auth_hosts = vec!["localhost".into()];
    let mut rig = Rig::start(config, sink.clone()).await;
    let tcp = connect_via(rig.addr, &format!("localhost:{port}")).await;
    let mut client = tls_over(tcp, &[pki.root()], "localhost").await;
    let mut up = upstream.accept().await.tls;
    through(&mut client, &mut up, &request, &reply).await;
    close_both(client, up).await;
    let handle = rig.handle();
    eventually("the tunnel is reported", || {
        handle.health().tunneled_auth_host == 1
    })
    .await;
    let report = rig.stop().await;
    assert_eq!(report.health.intercepted, 0);
    assert_eq!(report.producer_close, ProducerClose::Queued);
    sink.settle().await;
    assert_eq!(
        sink.retained(),
        vec![Retained::ProducerClosed {
            producer: PRODUCER.into(),
            final_seq: None
        }],
        "an opaque tunnel produced capture records"
    );

    // An approved host whose ClientHello names another server, and a host that is not
    // approved: both tunnel unchanged, capture nothing, and are reported.
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut config = RigConfig::new(&pki);
    config.approved = vec![approved("localhost", policy(&[("/", true, None)]))];
    let mut rig = Rig::start(config, sink.clone()).await;
    // A host that is not approved is opaque by design, not a loss: nothing is chained for it.
    // Its ClientHello names a host that is not approved either; one naming an approved host is
    // a bypass (`a_tunnel_to_another_authority_naming_an_approved_host_is_a_loss`).
    let tcp = connect_via(rig.addr, &format!("127.0.0.1:{port}")).await;
    let mut client = tls_over(tcp, &[pki.root()], "other.test").await;
    let mut up = upstream.accept().await.tls;
    through(&mut client, &mut up, &request, &reply).await;
    close_both(client, up).await;
    let handle = rig.handle();
    eventually("the not-approved tunnel is reported", || {
        handle.health().tunneled_not_approved == 1
    })
    .await;
    sink.settle().await;
    assert!(
        sink.frames().is_empty(),
        "a not-approved tunnel is not a capture loss"
    );
    // The ClientHello names another server: an uncaptured relay of an approved host. It is a
    // loss, and it is the one frame this producer has.
    let tcp = connect_via(rig.addr, &format!("localhost:{port}")).await;
    let mut client = tls_over(tcp, &[pki.root()], "other.test").await;
    let conn = upstream.accept().await;
    assert_eq!(conn.sni.as_deref(), Some("other.test"));
    let mut up = conn.tls;
    through(&mut client, &mut up, &request, &reply).await;
    close_both(client, up).await;
    eventually("the fail-open tunnel is reported", || {
        handle.health().tunneled_fail_open == 1
    })
    .await;
    let report = rig.stop().await;
    assert_eq!(report.health.intercepted, 0);
    assert!(report.capture_loss(), "a fail-open tunnel is capture loss");
    sink.settle().await;
    let frames = sink.frames();
    check_frames(&frames);
    assert_eq!(
        losses(&frames),
        vec![(0, GapReason::DecodeFailed)],
        "the fail-open tunnel is the producer's one stream-zero loss, first"
    );
    assert_eq!(frames[0].stream.stream, 0);
    assert_eq!(
        sink.retained().last(),
        Some(&Retained::ProducerClosed {
            producer: PRODUCER.into(),
            final_seq: Some(0)
        }),
        "the clean close attests the loss frame"
    );
    for sentinel in [
        "SENTINEL-TUNNEL-BODY",
        "SENTINEL-TUNNEL-TOKEN",
        "SENTINEL-TUNNEL-REPLY",
        SECRET,
    ] {
        assert!(!retained_contains(&sink.retained(), sentinel.as_bytes()));
    }
}

/// A CONNECT to another authority (here an IP literal; an authentication host takes the same
/// path) whose ClientHello names an approved host carries that host's traffic uncaptured. It is
/// a fail-open tunnel and a loss, never an opaque by-design one.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tunnel_to_another_authority_naming_an_approved_host_is_a_loss() {
    let pki = TestPki::new("upstream test CA");
    let mut upstream = Upstream::start(&pki, &["localhost"]).await;
    let port = upstream.addr.port();
    let request = post("/v1/responses", b"SENTINEL-BYPASS-BODY", "");
    let reply = ok_response("application/json", b"SENTINEL-BYPASS-REPLY");
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut rig = Rig::start(RigConfig::new(&pki), sink.clone()).await;
    let tcp = connect_via(rig.addr, &format!("127.0.0.1:{port}")).await;
    let mut client = tls_over(tcp, &[pki.root()], "localhost").await;
    let mut up = upstream.accept().await.tls;
    send(&mut client, &request).await;
    let (head, body) = read_request(&mut up).await;
    assert_eq!([head, body].concat(), request);
    send(&mut up, &reply).await;
    let mut seen = Vec::new();
    read_at_least(&mut client, &mut seen, reply.len()).await;
    close_both(client, up).await;
    let handle = rig.handle();
    eventually("the fail-open tunnel is reported", || {
        handle.health().tunneled_fail_open == 1
    })
    .await;
    let report = rig.stop().await;
    assert_eq!(report.health.tunneled_not_approved, 0, "{report:?}");
    assert!(report.capture_loss(), "{report:?}");
    sink.settle().await;
    assert_eq!(
        losses(&sink.frames()),
        vec![(0, GapReason::DecodeFailed)],
        "the bypass is chained as the producer's one loss"
    );
    assert!(!retained_contains(
        &sink.retained(),
        b"SENTINEL-BYPASS-BODY"
    ));
}

/// A request inside an intercepted connection whose Host names another host is not relayed
/// uncaptured as if it were ordinary traffic: the connection falls back to opaque, a loss.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_request_for_another_host_inside_an_intercepted_connection_is_a_loss() {
    let pki = TestPki::new("upstream test CA");
    let mut upstream = Upstream::start(&pki, &["localhost"]).await;
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut rig = Rig::start(RigConfig::new(&pki), sink.clone()).await;
    let mut client = rig.client(upstream.addr.port()).await;
    let mut up = upstream.accept().await.tls;
    let request = b"POST /v1/responses HTTP/1.1\r\nHost: other.test\r\n\
        Content-Type: application/json\r\nContent-Length: 18\r\n\r\nSENTINEL-HOST-BODY"
        .to_vec();
    send(&mut client, &request).await;
    let mut got = Vec::new();
    read_at_least(&mut up, &mut got, request.len()).await;
    assert_eq!(got, request, "the bytes are still relayed unchanged");
    let response = ok_response("application/json", b"{}");
    send(&mut up, &response).await;
    let mut seen = Vec::new();
    read_at_least(&mut client, &mut seen, response.len()).await;
    close_both(client, up).await;
    rig.wait_closed(1).await;
    let report = rig.stop().await;
    sink.settle().await;
    assert_eq!(report.health.opaque_fallbacks, 1, "{report:?}");
    assert!(report.capture_loss(), "{report:?}");
    assert!(
        streams(&sink.frames()).is_empty(),
        "nothing of it was captured"
    );
    assert_eq!(
        losses(&sink.frames())
            .into_iter()
            .map(|(_, reason)| reason)
            .collect::<Vec<_>>(),
        vec![GapReason::DecodeFailed]
    );
}

/// H-16 (CONTRACTS.md, proxy loss in the stream): a fail-open tunnel runs inside admission until both sides close, so its
/// loss is chained when the tunnel is decided, not when it ends. Held open here, the frame is
/// already in the sink while the tunnel still carries bytes, and the counter moved with it. A
/// task cut or panicking mid-relay therefore still leaves its loss in the stream.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_fail_open_tunnel_is_chained_when_it_begins_not_when_it_ends() {
    let pki = TestPki::new("upstream test CA");
    let mut upstream = Upstream::start(&pki, &["localhost", "other.test"]).await;
    let port = upstream.addr.port();
    let request = post("/v1/responses", b"HELD-TUNNEL-BODY", "");
    let reply = ok_response("application/json", b"HELD-TUNNEL-REPLY");

    // SNI naming another server.
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut config = RigConfig::new(&pki);
    config.approved = vec![approved("localhost", policy(&[("/", true, None)]))];
    let mut rig = Rig::start(config, sink.clone()).await;
    let tcp = connect_via(rig.addr, &format!("localhost:{port}")).await;
    let mut client = tls_over(tcp, &[pki.root()], "other.test").await;
    let mut up = upstream.accept().await.tls;
    eventually("the loss is chained while the tunnel is open", || {
        sink.frames()
            .iter()
            .any(|f| matches!(f.event, FrameEvent::Loss { .. }))
    })
    .await;
    assert_eq!(
        rig.handle().health().tunneled_fail_open,
        1,
        "the tunnel has not ended: the counter moved with the frame, at the decision"
    );
    // And the tunnel is live and unchanged after the loss was recorded.
    send(&mut client, &request).await;
    let (head, body) = read_request(&mut up).await;
    assert_eq!([head, body].concat(), request);
    send(&mut up, &reply).await;
    let mut seen = Vec::new();
    read_at_least(&mut client, &mut seen, reply.len()).await;
    assert_eq!(seen, reply);
    close_both(client, up).await;
    let report = rig.stop().await;
    assert_eq!(report.health.tunneled_fail_open, 1);
    assert_eq!(report.health.fail_open_sni_mismatch, 1, "{report:?}");
    assert_eq!(report.producer_close, ProducerClose::Queued);
    sink.settle().await;
    let frames = sink.frames();
    check_frames(&frames);
    assert_eq!(losses(&frames), vec![(0, GapReason::DecodeFailed)]);
    assert_eq!(
        sink.retained().last(),
        Some(&Retained::ProducerClosed {
            producer: PRODUCER.into(),
            final_seq: Some(0)
        })
    );

    // The first bytes are not TLS: decided from the first read, with the tunnel still open.
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut config = RigConfig::new(&pki);
    config.approved = vec![approved("localhost", policy(&[("/", true, None)]))];
    let mut rig = Rig::start(config, sink.clone()).await;
    let mut tcp = connect_via(rig.addr, &format!("localhost:{port}")).await;
    tcp.write_all(b"NOT-TLS\r\n\r\n").await.unwrap();
    eventually(
        "the loss of a non-TLS tunnel is chained while it is open",
        || {
            sink.frames()
                .iter()
                .any(|f| matches!(f.event, FrameEvent::Loss { .. }))
        },
    )
    .await;
    assert_eq!(rig.handle().health().tunneled_fail_open, 1);
    drop(tcp);
    let report = rig.stop().await;
    assert_eq!(report.health.tunneled_fail_open, 1, "{report:?}");
    assert_eq!(report.health.fail_open_not_tls, 1, "{report:?}");
    sink.settle().await;
    assert_eq!(
        losses(&sink.frames())
            .into_iter()
            .map(|(_, reason)| reason)
            .collect::<Vec<_>>(),
        vec![GapReason::DecodeFailed]
    );
}

/// H-16: a connection cut while still unsettled (here in admission, before it relayed a byte)
/// is counted and chained as a `ShutdownDeadline` loss ahead of the producer's close. The close
/// is conservative for such a connection, which may not have been a tunnel at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_connection_cut_unsettled_is_a_loss_before_the_close() {
    let pki = TestPki::new("upstream test CA");
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut config = RigConfig::new(&pki);
    config.shutdown = Duration::from_millis(400);
    let mut rig = Rig::start(config, sink.clone()).await;
    let mut silent = TcpStream::connect(rig.addr).await.unwrap();
    let handle = rig.handle();
    eventually("the connection is in admission", || {
        handle.health().unsettled == 1
    })
    .await;
    let report = rig.stop().await;
    assert_eq!(report.health.cut_unsettled, 1, "{report:?}");
    assert_eq!(report.producer_close, ProducerClose::Queued);
    assert!(report.capture_loss());
    let _ = read_to_end(&mut silent).await;
    sink.settle().await;
    let frames = sink.frames();
    check_frames(&frames);
    assert_eq!(losses(&frames), vec![(0, GapReason::ShutdownDeadline)]);
    assert_eq!(
        sink.retained().last(),
        Some(&Retained::ProducerClosed {
            producer: PRODUCER.into(),
            final_seq: Some(0)
        }),
        "the close attests the loss it follows"
    );
}

/// A fail-open tunnel still open at stop is settled at its decision, so it is no unsettled cut,
/// but it is still counted: the report reads lossy and the seal request's counters carry it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_fail_open_tunnel_open_at_stop_is_still_capture_loss() {
    let pki = TestPki::new("upstream test CA");
    let mut upstream = Upstream::start(&pki, &["localhost", "other.test"]).await;
    let port = upstream.addr.port();
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut config = RigConfig::new(&pki);
    config.approved = vec![approved("localhost", policy(&[("/", true, None)]))];
    config.shutdown = Duration::from_millis(400);
    let mut rig = Rig::start(config, sink.clone()).await;
    let tcp = connect_via(rig.addr, &format!("localhost:{port}")).await;
    let mut client = tls_over(tcp, &[pki.root()], "other.test").await;
    let mut up = upstream.accept().await.tls;
    let handle = rig.handle();
    eventually("the tunnel is open", || handle.health().unsettled == 1).await;
    let report = rig.stop().await;
    assert_eq!(report.health.cut_unsettled, 0, "{report:?}");
    assert_eq!(report.health.tunneled_fail_open, 1, "{report:?}");
    assert!(report.capture_loss(), "{report:?}");
    let _ = read_to_end(&mut client).await;
    let _ = read_to_end(&mut up).await;
    sink.settle().await;
    assert_eq!(losses(&sink.frames()), vec![(0, GapReason::DecodeFailed)]);
}

/// Fix plan v10 U1: a client that closes after the `200` for an approved host, before its
/// ClientHello is complete (no byte at all, or part of a record header), sent nothing of that
/// host's traffic. It is a client failure: no upstream connection, no fail-open, no loss. A
/// ClientHello that does arrive but cannot be parsed stays fail-open, a loss, with its reason.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_client_that_closes_before_its_client_hello_is_no_loss() {
    let pki = TestPki::new("upstream test CA");
    let upstream = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    upstream.set_nonblocking(true).unwrap();
    let port = upstream.local_addr().unwrap().port();
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut config = RigConfig::new(&pki);
    config.approved = vec![approved("localhost", policy(&[("/", true, None)]))];
    let mut rig = Rig::start(config, sink.clone()).await;
    let handle = rig.handle();

    let silent = connect_via(rig.addr, &format!("localhost:{port}")).await;
    drop(silent);
    let mut partial = connect_via(rig.addr, &format!("localhost:{port}")).await;
    partial.write_all(&[0x16, 0x03, 0x01, 0x00]).await.unwrap();
    drop(partial);
    eventually("both closes are client failures", || {
        handle.health().admission_failures.client_io == 2
    })
    .await;
    let health = handle.health();
    assert_eq!(health.tunneled_fail_open, 0, "{health:?}");
    assert_eq!(health.admission_failed, 2, "{health:?}");
    assert_eq!(
        upstream.accept().map(|_| ()).unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock,
        "no upstream connection was opened for a client that sent no ClientHello"
    );
    sink.settle().await;
    assert!(
        losses(&sink.frames()).is_empty(),
        "a client close is no loss"
    );

    // A complete handshake record that is not a ClientHello: still fail-open, by its reason.
    let mut garbage = connect_via(rig.addr, &format!("localhost:{port}")).await;
    garbage
        .write_all(&[0x16, 0x03, 0x01, 0x00, 0x04, 0x02, 0x00, 0x00, 0x00])
        .await
        .unwrap();
    eventually("the unparsable ClientHello is a fail-open tunnel", || {
        handle.health().fail_open_unparsed == 1
    })
    .await;
    let (tunnelled, _) = upstream.accept().expect("the tunnel connected upstream");
    drop(tunnelled);
    drop(garbage);
    let report = rig.stop().await;
    assert_eq!(report.health.tunneled_fail_open, 1, "{report:?}");
    assert_eq!(report.health.fail_open_unparsed, 1, "{report:?}");
    assert_eq!(report.health.admission_failures.client_io, 2, "{report:?}");
    assert!(report.capture_loss(), "{report:?}");
    sink.settle().await;
    assert_eq!(losses(&sink.frames()), vec![(0, GapReason::DecodeFailed)]);
}

/// Fix plan v10: a fail-open tunnel is counted by its reason, so `host.log` says why the loss
/// happened. A complete ClientHello with no server name, to an approved host, is relayed
/// uncaptured and counted as `fail_open_no_sni`, and only there (review P4 L4).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_client_hello_without_a_server_name_counts_as_fail_open_no_sni() {
    let pki = TestPki::new("upstream test CA");
    let upstream = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    upstream.set_nonblocking(true).unwrap();
    let port = upstream.local_addr().unwrap().port();
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut config = RigConfig::new(&pki);
    config.approved = vec![approved("localhost", policy(&[("/", true, None)]))];
    let mut rig = Rig::start(config, sink.clone()).await;
    let handle = rig.handle();

    // rustls with SNI off: a real, complete ClientHello that names no server.
    let hello = client_hello(client_config(&[], &[], false), "localhost");
    let mut client = connect_via(rig.addr, &format!("localhost:{port}")).await;
    client.write_all(&hello).await.unwrap();
    eventually("the ClientHello is decided", || {
        handle.health().tunneled_fail_open == 1
    })
    .await;
    let (tunnelled, _) = upstream.accept().expect("the tunnel connected upstream");
    drop(tunnelled);
    drop(client);
    let report = rig.stop().await;
    let health = &report.health;
    assert_eq!(health.tunneled_fail_open, 1, "{report:?}");
    assert_eq!(
        (
            health.fail_open_no_sni,
            health.fail_open_sni_mismatch,
            health.fail_open_not_tls,
            health.fail_open_unparsed,
        ),
        (1, 0, 0, 0),
        "{report:?}"
    );
    assert!(report.capture_loss(), "{report:?}");
    sink.settle().await;
    assert_eq!(losses(&sink.frames()), vec![(0, GapReason::DecodeFailed)]);
}

/// Tunnels that are opaque by design (an authentication host, a host that is not approved),
/// still held open at stop by a process that outlived the harness, were decided when their
/// ClientHello arrived: cutting them is not a loss, and nothing is chained. Each is still noted in
/// the sealed-connection list, with its bytes unknown.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_open_tunnel_opaque_by_design_cut_at_stop_is_not_a_loss() {
    let pki = TestPki::new("upstream test CA");
    let mut upstream = Upstream::start(&pki, &["localhost", "other.test"]).await;
    let port = upstream.addr.port();
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut config = RigConfig::new(&pki);
    config.approved = vec![approved("api.test", policy(&[("/", true, None)]))];
    config.auth_hosts = vec!["localhost".into()];
    config.shutdown = Duration::from_millis(400);
    let mut rig = Rig::start(config, sink.clone()).await;
    let tcp = connect_via(rig.addr, &format!("localhost:{port}")).await;
    let mut auth = tls_over(tcp, &[pki.root()], "localhost").await;
    let mut auth_up = upstream.accept().await.tls;
    let tcp = connect_via(rig.addr, &format!("127.0.0.1:{port}")).await;
    let mut other = tls_over(tcp, &[pki.root()], "other.test").await;
    let mut other_up = upstream.accept().await.tls;
    let handle = rig.handle();
    eventually("both tunnels are open", || handle.health().unsettled == 2).await;
    let report = rig.stop().await;
    assert_eq!(report.health.cut_unsettled, 0, "{report:?}");
    assert_eq!(report.producer_close, ProducerClose::Queued);
    assert!(!report.capture_loss(), "{report:?}");
    // Both are still in the sealed-connection list, cut: how many bytes passed is not known.
    let mut cut: Vec<_> = report
        .health
        .tunnels
        .iter()
        .map(|note| (note.host.as_str(), note.port, note.reason, note.up_bytes))
        .collect();
    cut.sort();
    assert_eq!(
        cut,
        [
            ("127.0.0.1", port, "not_model_host", None),
            ("localhost", port, "sign_in", None)
        ]
    );
    let _ = read_to_end(&mut auth).await;
    let _ = read_to_end(&mut auth_up).await;
    let _ = read_to_end(&mut other).await;
    let _ = read_to_end(&mut other_up).await;
    sink.settle().await;
    assert!(losses(&sink.frames()).is_empty(), "nothing is chained");
    assert_eq!(
        sink.retained(),
        vec![Retained::ProducerClosed {
            producer: PRODUCER.into(),
            final_seq: None
        }]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stop_with_active_stream_and_blocked_sink_meets_one_deadline() {
    let clock = clock_start();
    let pki = TestPki::new("upstream test CA");
    let mut upstream = Upstream::start(&pki, &["localhost", "other.test"]).await;
    let port = upstream.addr.port();
    let sink = DirectSink::new(QUEUE_BYTES);
    sink.block_worker(true);
    let mut config = RigConfig::new(&pki);
    config.shutdown = Duration::from_millis(600);
    let mut rig = Rig::start(config, sink.clone()).await;

    // An intercepted SSE stream, mid-body.
    let mut client = rig.client(port).await;
    let mut up = upstream.accept().await.tls;
    send(&mut client, &post("/v1/responses", b"{}", "")).await;
    let _ = read_request(&mut up).await;
    let head =
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n";
    send(&mut up, &[&head[..], &chunk(b"data: partial\n\n")].concat()).await;
    let mut seen = Vec::new();
    read_until(&mut client, &mut seen, b"data: partial").await;
    // A connection still in admission, and an open opaque tunnel.
    let mut silent = TcpStream::connect(rig.addr).await.unwrap();
    let tcp = connect_via(rig.addr, &format!("127.0.0.1:{port}")).await;
    let mut tunnel = tls_over(tcp, &[pki.root()], "other.test").await;
    let mut tunnel_up = upstream.accept().await.tls;
    let handle = rig.handle();
    eventually("three live connections", || {
        let h = handle.health();
        h.intercepted_open == 1 && h.unsettled == 2
    })
    .await;

    let started = Instant::now();
    let report = rig.stop().await;
    let elapsed = started.elapsed();
    eprintln!("evidence bounded stop (deadline 600 ms): {elapsed:?} {report:?}");
    // One deadline, measured from the call: no nested drain gets a timeout of its own.
    assert!(
        elapsed < Duration::from_millis(600 + 150),
        "stop took {elapsed:?} for a 600 ms deadline"
    );
    assert!(!report.drained && report.unterminated == 0, "{report:?}");
    assert_eq!(report.health.cut_intercepted, 1);
    // The open tunnel is not a host that is approved: it was decided, and cut without a loss.
    assert_eq!(report.health.cut_unsettled, 1);
    assert_eq!(report.health.closed_interrupted, 1);
    assert_eq!(
        (report.health.active, report.health.unsettled),
        (0, 0),
        "{report:?}"
    );
    assert_eq!(report.producer_close, ProducerClose::Queued);
    // Cuts are the drain's story, not transport failures; the cut admission is possible
    // capture loss (admission cannot say yet whether it would have been a fail-open tunnel).
    assert!(
        !report.is_clean_drain() && report.capture_loss(),
        "{report:?}"
    );
    assert_eq!(report.transport_failures(), 0);
    // Every socket the recording proxy held is gone.
    for (what, ended) in [
        ("client", read_to_end(&mut client).await),
        ("upstream", read_to_end(&mut up).await),
        ("silent", read_to_end(&mut silent).await),
        ("tunnel client", read_to_end(&mut tunnel).await),
        ("tunnel upstream", read_to_end(&mut tunnel_up).await),
    ] {
        assert!(!contains(&ended.0, b"data: late"), "{what}");
    }

    // The close was queued, not retained: nothing is validated while the worker is stalled.
    assert!(sink.retained().is_empty());
    sink.block_worker(false);
    sink.settle().await;
    eprintln!(
        "evidence cut-at-deadline trace:\n{}",
        trace(&sink.retained())
    );
    let frames = sink.frames();
    check_frames(&frames);
    check_clock(&frames, clock, clock_now());
    assert_eq!(
        losses(&frames)
            .into_iter()
            .map(|(_, reason)| reason)
            .collect::<Vec<_>>(),
        vec![GapReason::ShutdownDeadline],
        "the cut admission is the one chained loss"
    );
    let views = bodies(&frames);
    let response = views
        .iter()
        .find(|v| v.source == CaptureSource::WireResponse)
        .unwrap();
    assert_eq!(response.bytes, b"data: partial\n\n");
    assert_eq!(response.end, Some(EndReason::Interrupted));
    let close = controls(&frames).pop().unwrap();
    assert_eq!(
        close.event,
        ConnectionEvent::Close {
            outcome: ConnectionOutcome::Interrupted
        }
    );
    assert!(response.end_at.unwrap() < close.at);
    assert_eq!(
        sink.retained().last(),
        Some(&Retained::ProducerClosed {
            producer: PRODUCER.into(),
            final_seq: Some(frames.last().unwrap().seq),
        })
    );

    // An idle keep-alive connection (no stream open) is cut too, and its close is retained
    // before the producer's.
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut config = RigConfig::new(&pki);
    config.shutdown = Duration::from_millis(400);
    let mut rig = Rig::start(config, sink.clone()).await;
    let mut client = rig.client(port).await;
    let mut up = upstream.accept().await.tls;
    exchange(&mut client, &mut up, "/v1/responses", b"idle-after", b"{}").await;
    let report = rig.stop().await;
    assert_eq!(report.producer_close, ProducerClose::Queued);
    assert_eq!(
        (
            report.health.cut_intercepted,
            report.health.control_refusals
        ),
        (1, 0)
    );
    sink.settle().await;
    let retained = sink.retained();
    let n = retained.len();
    assert!(
        matches!(
            &retained[n - 2],
            Retained::Frame(f) if matches!(
                &f.event,
                goalcup_protocol::capture::FrameEvent::Connection {
                    event: ConnectionEvent::Close { outcome: ConnectionOutcome::Interrupted },
                    ..
                }
            )
        ),
        "{:?}",
        retained[n - 2]
    );
    assert!(matches!(&retained[n - 1], Retained::ProducerClosed { .. }));
    drop((client, up));

    // With the sink refusing at the cut, the tail and the close are refused, and said so.
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut config = RigConfig::new(&pki);
    config.shutdown = Duration::from_millis(400);
    let mut rig = Rig::start(config, sink.clone()).await;
    let mut client = rig.client(port).await;
    let mut up = upstream.accept().await.tls;
    send(&mut client, &post("/v1/responses", b"{}", "")).await;
    let _ = read_request(&mut up).await;
    send(&mut up, &[&head[..], &chunk(b"data: partial\n\n")].concat()).await;
    let mut seen = Vec::new();
    read_until(&mut client, &mut seen, b"data: partial").await;
    sink.set_pressure(true);
    let report = rig.stop().await;
    assert_eq!(
        report.producer_close,
        ProducerClose::Refused(GapReason::QueueFull)
    );
    assert!(report.capture_loss() && report.health.control_refusals >= 1);
    sink.settle().await;
    assert!(
        !sink
            .retained()
            .iter()
            .any(|r| matches!(r, Retained::ProducerClosed { .. }))
    );
    assert!(sink.retained().contains(&Retained::Gap {
        producer: PRODUCER.into(),
        reason: GapReason::QueueFull
    }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn connection_local_tls_error_keeps_listener_and_other_identities() {
    let clock = clock_start();
    let pki = TestPki::new("upstream test CA");
    let mut upstream = Upstream::start(&pki, &["localhost"]).await;
    let port = upstream.addr.port();
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut rig = Rig::start(RigConfig::new(&pki), sink.clone()).await;

    let mut a = rig.client(port).await;
    let mut a_up = upstream.accept().await.tls;
    exchange(&mut a, &mut a_up, "/v1/responses", b"marker-a-1", b"{}").await;

    // B breaks its own TLS mid-connection: a forged application-data record.
    let mut b = rig.client(port).await;
    let mut b_up = upstream.accept().await.tls;
    exchange(&mut b, &mut b_up, "/v1/responses", b"marker-b-1", b"{}").await;
    let forged = [&[0x17u8, 0x03, 0x03, 0x00, 0x20][..], &[0x5a; 32]].concat();
    b.get_mut().0.write_all(&forged).await.unwrap();
    let _ = read_to_end(&mut b_up).await;

    // E ends inside a request body: the relay's UnexpectedEof closes it Interrupted.
    let mut e = rig.client(port).await;
    let mut e_up = upstream.accept().await.tls;
    let cut = post("/v1/responses", &[b'x'; 100], "");
    send(&mut e, &cut[..cut.len() - 90]).await;
    let mut got = Vec::new();
    read_until(&mut e_up, &mut got, b"\r\n\r\n").await;
    e.shutdown().await.unwrap();
    let _ = read_to_end(&mut e_up).await;

    // C fails the harness-facing handshake: its client does not trust the recording proxy's CA.
    let tcp = connect_via(rig.addr, &format!("localhost:{port}")).await;
    assert!(try_tls_over(tcp, &[pki.root()], "localhost").await.is_err());
    let _c_up = upstream.accept().await;

    let handle = rig.handle();
    eventually("B, C and E are settled", || {
        let h = handle.health();
        h.closed_transport_error == 1 && h.admission_failed == 1 && h.closed_interrupted == 1
    })
    .await;

    // A is untouched, and the listener still admits D.
    exchange(&mut a, &mut a_up, "/v1/responses", b"marker-a-2", b"{}").await;
    let mut d = rig.client(port).await;
    let mut d_up = upstream.accept().await.tls;
    exchange(&mut d, &mut d_up, "/v1/responses", b"marker-d-1", b"{}").await;
    close_both(a, a_up).await;
    close_both(d, d_up).await;
    rig.wait_closed(4).await;
    let report = rig.stop().await;
    assert_eq!(report.health.task_panics, 0);
    assert_eq!(report.health.closed_complete, 2);
    assert_eq!(report.health.closed_transport_error, 1);
    assert_eq!(report.health.closed_interrupted, 1);
    assert_eq!(report.producer_close, ProducerClose::Queued);
    assert!(report.is_clean_drain() && !report.capture_loss());
    assert_eq!(
        report.transport_failures(),
        2,
        "B's TLS error and C's admission"
    );
    sink.settle().await;

    let frames = sink.frames();
    check_frames(&frames);
    check_clock(&frames, clock, clock_now());
    let views = streams(&frames);
    let conn_a = stream_with(&views, b"marker-a-1").connection().to_string();
    let conn_b = stream_with(&views, b"marker-b-1").connection().to_string();
    let conn_d = stream_with(&views, b"marker-d-1").connection().to_string();
    assert!(conn_a != conn_b && conn_b != conn_d && conn_a != conn_d);
    let a2 = stream_with(&views, b"marker-a-2");
    assert_eq!((a2.connection(), a2.exchange()), (conn_a.as_str(), 2));
    let closes: Vec<(String, ConnectionEvent)> = controls(&frames)
        .into_iter()
        .filter(|c| matches!(c.event, ConnectionEvent::Close { .. }))
        .map(|c| (c.connection, c.event))
        .collect();
    let close_of = |conn: &str| {
        closes
            .iter()
            .find(|(c, _)| c == conn)
            .map(|(_, e)| e.clone())
            .unwrap()
    };
    assert_eq!(
        close_of(&conn_b),
        ConnectionEvent::Close {
            outcome: ConnectionOutcome::TransportError
        }
    );
    for conn in [&conn_a, &conn_d] {
        assert_eq!(
            close_of(conn),
            ConnectionEvent::Close {
                outcome: ConnectionOutcome::Complete
            }
        );
    }
    let cut_body = views
        .iter()
        .find(|v| v.source == CaptureSource::WireRequest && v.bytes == [b'x'; 10])
        .expect("the cut request body");
    assert_eq!(cut_body.end, Some(EndReason::Interrupted));
    assert_eq!(
        close_of(cut_body.connection()),
        ConnectionEvent::Close {
            outcome: ConnectionOutcome::Interrupted
        }
    );
    assert_eq!(
        closes.len(),
        4,
        "the failed handshake opened no capture connection"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn excess_connections_are_refused_predictably() {
    let pki = TestPki::new("upstream test CA");
    let mut upstream = Upstream::start(&pki, &["localhost"]).await;
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut config = RigConfig::new(&pki);
    config.max_connections = 2;
    let mut rig = Rig::start(config, sink.clone()).await;

    // Two connections hold both slots: one intercepted, one still in admission.
    let mut held = rig.client(upstream.addr.port()).await;
    let mut held_up = upstream.accept().await.tls;
    let mut silent = TcpStream::connect(rig.addr).await.unwrap();
    let handle = rig.handle();
    eventually("both slots taken", || handle.health().active == 2).await;

    // A third is answered 503 and closed at once, without admission or upstream contact.
    let mut third = TcpStream::connect(rig.addr).await.unwrap();
    let (answer, _) = read_to_end(&mut third).await;
    assert!(
        answer.is_empty() || answer.starts_with(b"HTTP/1.1 503 "),
        "{}",
        String::from_utf8_lossy(&answer)
    );
    eventually("refusal counted", || {
        handle.health().refused_over_capacity == 1
    })
    .await;
    assert!(upstream.try_accept().is_none());
    assert_eq!(handle.health().active, 2);

    // A freed slot admits the next connection.
    silent.shutdown().await.unwrap();
    drop(silent);
    eventually("slot freed", || handle.health().active == 1).await;
    let mut next = rig.client(upstream.addr.port()).await;
    let mut next_up = upstream.accept().await.tls;
    exchange(
        &mut next,
        &mut next_up,
        "/v1/responses",
        b"after-slot",
        b"{}",
    )
    .await;
    exchange(&mut held, &mut held_up, "/v1/responses", b"held", b"{}").await;
    close_both(next, next_up).await;
    close_both(held, held_up).await;
    rig.wait_closed(2).await;
    let report = rig.stop().await;
    assert_eq!(report.health.refused_over_capacity, 1);
    assert_eq!(report.health.accepted, 4);
    assert_eq!(report.producer_close, ProducerClose::Queued);
}

/// Opens an intercepted connection whose relay falls back to opaque bytes: a selected request
/// whose head has no end within the relay's 64 KiB head bound. The rest of the connection is
/// relayed as bytes, unchanged, both ways.
async fn fall_back_to_opaque(rig: &Rig, upstream: &mut Upstream) -> (ClientTls, UpstreamTls) {
    let mut client = rig.client(upstream.addr.port()).await;
    let mut up = upstream.accept().await.tls;
    let mut request = b"POST /v1/responses HTTP/1.1\r\nHost: localhost\r\nX-Big: ".to_vec();
    request.extend(vec![b'a'; 70 * 1024]);
    request.extend(b"\r\nContent-Length: 13\r\n\r\nopaque-marker");
    send(&mut client, &request).await;
    let mut got = Vec::new();
    read_at_least(&mut up, &mut got, request.len()).await;
    assert_eq!(got, request);
    let response = ok_response("application/json", b"{}");
    send(&mut up, &response).await;
    let mut seen = Vec::new();
    read_at_least(&mut client, &mut seen, response.len()).await;
    assert_eq!(seen, response);
    (client, up)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn opaque_fallback_is_capture_loss() {
    let pki = TestPki::new("upstream test CA");
    let mut upstream = Upstream::start(&pki, &["localhost"]).await;
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut rig = Rig::start(RigConfig::new(&pki), sink.clone()).await;
    let (client, up) = fall_back_to_opaque(&rig, &mut upstream).await;
    close_both(client, up).await;
    rig.wait_closed(1).await;
    let report = rig.stop().await;
    sink.settle().await;

    // The relay itself went well; only its capture did not.
    assert!(
        report.is_clean_drain() && report.transport_failures() == 0,
        "{report:?}"
    );
    assert_eq!(report.health.closed_complete, 1);
    assert!(streams(&sink.frames()).is_empty());
    assert!(report.capture_loss(), "{report:?}");
    assert_eq!(report.health.opaque_fallbacks, 1);
    // The fallback is in the stream, ahead of the clean close that would otherwise hide it.
    let frames = sink.frames();
    check_frames(&frames);
    assert_eq!(
        losses(&frames)
            .into_iter()
            .map(|(_, reason)| reason)
            .collect::<Vec<_>>(),
        vec![GapReason::DecodeFailed]
    );
    let last = frames.last().unwrap().seq;
    assert_eq!(
        sink.retained().last(),
        Some(&Retained::ProducerClosed {
            producer: PRODUCER.into(),
            final_seq: Some(last)
        })
    );
}

/// A fallback is counted when it happens, not only at a clean end: both peers going away
/// without a TLS `close_notify`, as an exiting harness routinely does, still counts it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn opaque_fallback_is_counted_however_the_connection_ends() {
    let pki = TestPki::new("upstream test CA");
    let mut upstream = Upstream::start(&pki, &["localhost"]).await;
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut rig = Rig::start(RigConfig::new(&pki), sink.clone()).await;
    let (client, up) = fall_back_to_opaque(&rig, &mut upstream).await;
    drop(client);
    drop(up);
    rig.wait_closed(1).await;
    let report = rig.stop().await;
    assert_eq!(report.health.closed_interrupted, 1, "{report:?}");
    assert_eq!(report.health.opaque_fallbacks, 1, "{report:?}");
    assert!(report.capture_loss(), "{report:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn start_accepts_more_tasks_than_capture_connections() {
    let pki = TestPki::new("upstream test CA");
    for max_connections in [MAX_OPEN_CONNECTIONS + 1, MAX_CONNECTION_TASKS as usize] {
        let mut config = RigConfig::new(&pki);
        config.max_connections = max_connections;
        let mut rig = Rig::try_start(config, DirectSink::new(QUEUE_BYTES))
            .await
            .expect("a task limit above the capture table starts");
        assert_all_clean(&rig.stop().await);
    }
}

/// The harness's subprocesses share the recording proxy, so intercepted connections can outnumber
/// the emitter's connection table. One past it is relayed uncaptured and counted; it neither
/// refuses the agent nor ends capture for the connections in the table.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn intercepted_connections_past_the_capture_table_fail_open() {
    allow_descriptors(4096);
    let pki = TestPki::new("upstream test CA");
    let mut upstream = Upstream::start(&pki, &["localhost"]).await;
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut config = RigConfig::new(&pki);
    config.max_connections = MAX_OPEN_CONNECTIONS + 8;
    let mut rig = Rig::start(config, sink.clone()).await;
    let mut held = Vec::new();
    for _ in 0..MAX_OPEN_CONNECTIONS {
        let client = rig.client(upstream.addr.port()).await;
        held.push((client, upstream.accept().await.tls));
    }
    let handle = rig.handle();
    eventually("the capture table is full", || {
        handle.health().intercepted_open == MAX_OPEN_CONNECTIONS as u64
    })
    .await;

    let mut extra = rig.client(upstream.addr.port()).await;
    let mut extra_up = upstream.accept().await.tls;
    exchange(
        &mut extra,
        &mut extra_up,
        "/v1/responses",
        b"past-the-table",
        b"{}",
    )
    .await;
    // The loss is in the stream before the uncaptured connection has even ended.
    eventually("the table-full loss is chained", || {
        sink.frames()
            .iter()
            .any(|f| matches!(f.event, FrameEvent::Loss { .. }))
    })
    .await;
    close_both(extra, extra_up).await;
    let health = handle.health();
    assert_eq!(health.capture_table_full, 1);
    assert_eq!(health.refused_over_capacity, 0);
    assert_eq!(health.intercepted, MAX_OPEN_CONNECTIONS as u64 + 1);
    assert_eq!(health.intercepted_open, MAX_OPEN_CONNECTIONS as u64);

    let (client, up) = &mut held[0];
    exchange(client, up, "/v1/responses", b"still-captured", b"{}").await;
    for (client, up) in held {
        close_both(client, up).await;
    }
    rig.wait_closed(MAX_OPEN_CONNECTIONS as u64).await;
    let report = rig.stop().await;
    sink.settle().await;

    let frames = sink.frames();
    let views = streams(&frames);
    assert_eq!(
        stream_with(&views, b"still-captured").end,
        Some(EndReason::Complete)
    );
    assert!(!views.iter().any(|v| contains(&v.bytes, b"past-the-table")));
    let opens = controls(&frames)
        .into_iter()
        .filter(|c| matches!(c.event, ConnectionEvent::Open { .. }))
        .count();
    assert_eq!(opens, MAX_OPEN_CONNECTIONS);
    assert_eq!(
        losses(&frames)
            .into_iter()
            .map(|(_, reason)| reason)
            .collect::<Vec<_>>(),
        vec![GapReason::LimitExceeded],
        "one loss for the connection relayed uncaptured"
    );
    check_frames(&frames);
    assert_eq!(report.producer_close, ProducerClose::Queued);
    assert!(report.capture_loss(), "{report:?}");
}

/// Exhausting the open-stream bound ends capture inside the emitter, where the sink never
/// sees it; the recording proxy's health still counts it as a refusal.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_stream_limit_refusal_is_counted() {
    allow_descriptors(4096);
    let pki = TestPki::new("upstream test CA");
    let mut upstream = Upstream::start(&pki, &["localhost"]).await;
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut config = RigConfig::new(&pki);
    config.max_connections = MAX_OPEN_STREAMS;
    let mut rig = Rig::start(config, sink.clone()).await;

    // Each connection holds two payload streams open: a request with half its body sent, and
    // the response its upstream began before the request finished.
    let request_head =
        b"POST /v1/responses HTTP/1.1\r\nHost: localhost\r\nContent-Length: 8\r\n\r\nhalf";
    let response_head = b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\n\r\nhalf";
    let mut held = Vec::new();
    for _ in 0..MAX_OPEN_STREAMS / 2 {
        let mut client = rig.client(upstream.addr.port()).await;
        let mut up = upstream.accept().await.tls;
        send(&mut client, request_head).await;
        read_at_least(&mut up, &mut Vec::new(), request_head.len()).await;
        send(&mut up, response_head).await;
        read_at_least(&mut client, &mut Vec::new(), response_head.len()).await;
        held.push((client, up));
    }
    let handle = rig.handle();
    assert_eq!(handle.health().sink_refusals, 0);

    // One stream more is refused by the emitter itself; the relay goes on.
    let mut extra = rig.client(upstream.addr.port()).await;
    let mut extra_up = upstream.accept().await.tls;
    exchange(
        &mut extra,
        &mut extra_up,
        "/v1/responses",
        b"one-more",
        b"{}",
    )
    .await;
    assert_eq!(handle.health().sink_refusals, 1);

    close_both(extra, extra_up).await;
    for (mut client, mut up) in held {
        send(&mut client, b"done").await;
        read_at_least(&mut up, &mut Vec::new(), 4).await;
        send(&mut up, b"done").await;
        read_at_least(&mut client, &mut Vec::new(), 4).await;
        close_both(client, up).await;
    }
    rig.wait_closed(MAX_OPEN_STREAMS as u64 / 2 + 1).await;
    let report = rig.stop().await;
    assert_eq!(report.health.sink_refusals, 1);
    assert!(report.capture_loss(), "{report:?}");
    assert!(
        sink.calls().iter().all(|call| !matches!(
            call,
            Call::Submit {
                answer: SubmitResult::Disabled(_),
                ..
            }
        )),
        "the sink itself refused nothing"
    );
    sink.settle().await;
    assert!(
        !streams(&sink.frames())
            .iter()
            .any(|v| contains(&v.bytes, b"one-more"))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn start_refuses_invalid_configuration_before_touching_the_sink() {
    let pki = TestPki::new("upstream test CA");
    type Case = (&'static str, Box<dyn Fn(&mut RigConfig)>, StartError);
    let cases: Vec<Case> = vec![
        (
            "producer",
            Box::new(|c| c.producer = "not-an-id".into()),
            StartError::InvalidProducer,
        ),
        (
            "startup deadline",
            Box::new(|c| c.startup = Duration::ZERO),
            StartError::InvalidLimits,
        ),
        (
            "shutdown deadline",
            Box::new(|c| c.shutdown = Duration::ZERO),
            StartError::InvalidLimits,
        ),
        (
            "no connections",
            Box::new(|c| c.max_connections = 0),
            StartError::InvalidLimits,
        ),
        (
            "too many connections",
            Box::new(|c| c.max_connections = MAX_CONNECTION_TASKS as usize + 1),
            StartError::InvalidLimits,
        ),
        (
            "no approved host",
            Box::new(|c| c.approved.clear()),
            StartError::Ca(goalcup_cli::proxy::tls::CaError::NoApprovedHosts),
        ),
    ];
    for (what, edit, expected) in cases {
        let sink = DirectSink::new(QUEUE_BYTES);
        let dir = tempfile::tempdir().unwrap();
        let mut config = RigConfig::new(&pki);
        edit(&mut config);
        let config = proxy_config(loopback_listener(), dir.path(), &config, sink.clone());
        let debug = format!("{config:?}");
        assert!(
            !debug.contains(SECRET),
            "{what}: Debug shows the credential"
        );
        let refused = within("start", goalcup_cli::proxy::start(config)).await;
        assert_eq!(refused.unwrap_err(), expected, "{what}");
        assert!(sink.calls().is_empty(), "{what}: the sink was asked");
        assert!(!dir.path().join(CA_CERT_FILE_NAME).exists(), "{what}");
    }
    // A listener not bound to a loopback address is refused before the CA or the sink. The
    // v4-mapped form of 127.0.0.1 binds without leaving loopback, and the check (on the
    // address as bound, `IpAddr::is_loopback`) conservatively refuses it.
    let sink = DirectSink::new(QUEUE_BYTES);
    let dir = tempfile::tempdir().unwrap();
    let mapped = std::net::TcpListener::bind("[::ffff:127.0.0.1]:0").unwrap();
    assert!(!mapped.local_addr().unwrap().ip().is_loopback());
    let config = proxy_config(mapped, dir.path(), &RigConfig::new(&pki), sink.clone());
    let refused = within("start", goalcup_cli::proxy::start(config)).await;
    assert_eq!(refused.unwrap_err(), StartError::NotLoopback);
    assert!(sink.calls().is_empty());
    assert!(!dir.path().join(CA_CERT_FILE_NAME).exists());

    // An existing certificate file is never replaced, and the sink is not asked.
    let sink = DirectSink::new(QUEUE_BYTES);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(CA_CERT_FILE_NAME), b"planted").unwrap();
    let config = proxy_config(
        loopback_listener(),
        dir.path(),
        &RigConfig::new(&pki),
        sink.clone(),
    );
    let refused = within("start", goalcup_cli::proxy::start(config)).await;
    assert_eq!(
        refused.unwrap_err(),
        StartError::Ca(goalcup_cli::proxy::tls::CaError::Write(
            std::io::ErrorKind::AlreadyExists
        ))
    );
    assert!(sink.calls().is_empty());
    assert_eq!(
        std::fs::read(dir.path().join(CA_CERT_FILE_NAME)).unwrap(),
        b"planted"
    );
}

#[test]
fn stop_never_closes_the_producer_past_a_stuck_task() {
    // The recording proxy gets its own runtime: the sink below blocks one of its workers inside
    // `try_submit`, which can starve that runtime's own timers; the owner stops it from here.
    let proxy_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let owner_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    owner_rt.block_on(async {
        let pki = TestPki::new("upstream test CA");
        let mut upstream = Upstream::start(&pki, &["localhost"]).await;
        let sink = DirectSink::new(QUEUE_BYTES);
        let dir = tempfile::tempdir().unwrap();
        let mut config = RigConfig::new(&pki);
        config.shutdown = Duration::from_secs(1);
        let config = proxy_config(loopback_listener(), dir.path(), &config, sink.clone());
        let handle = within("start", proxy_rt.spawn(goalcup_cli::proxy::start(config)))
            .await
            .unwrap()
            .unwrap();
        let tcp = connect_via(
            handle.local_addr(),
            &format!("localhost:{}", upstream.addr.port()),
        )
        .await;
        let mut client = tls_over(tcp, &[handle.ca_certificate_der().clone()], "localhost").await;
        let mut up = upstream.accept().await.tls;

        eventually("the connection is relayed", || {
            handle.health().intercepted_open == 1
        })
        .await;
        // A sink that breaks the submission contract and blocks inside `try_submit`: the relay
        // task is stuck in synchronous code (holding the emitter's lock), so cancellation
        // cannot end it.
        sink.stall_submits(true);
        send(&mut client, &post("/v1/responses", b"stuck", "")).await;
        eventually("the relay is stuck in the sink", || sink.stalled() == 1).await;
        // Released well after the deadline, from another thread, so a stop that waited on
        // the stuck task (or on the emitter lock it holds) would overrun visibly.
        let release = {
            let sink = sink.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(3));
                sink.stall_submits(false);
            })
        };
        let started = Instant::now();
        let report = within("stop", handle.stop()).await;
        let elapsed = started.elapsed();
        eprintln!("evidence stuck-task stop (deadline 1 s): {elapsed:?} {report:?}");
        // The one deadline holds even past a task that never ends: a nested timeout started
        // at the cut would return about 750 ms late.
        assert!(
            elapsed >= Duration::from_millis(900) && elapsed < Duration::from_millis(1000 + 150),
            "stop took {elapsed:?} for a 1 s deadline"
        );
        assert_eq!(report.unterminated, 1, "{report:?}");
        assert_eq!(
            report.health.intercepted_open, 1,
            "the stuck task is the relay"
        );
        assert_eq!(report.producer_close, ProducerClose::NotAttempted);
        assert!(!report.is_clean_drain() && report.capture_loss());
        release.join().unwrap();
        let _ = read_to_end(&mut up).await;
        let _ = read_to_end(&mut client).await;
        sink.settle().await;
        assert!(
            !sink.calls().iter().any(|c| matches!(c, Call::Close { .. })),
            "a close was asked for although a task never ended"
        );
        assert!(
            !sink
                .retained()
                .iter()
                .any(|r| matches!(r, Retained::ProducerClosed { .. }))
        );
    });
    proxy_rt.shutdown_timeout(Duration::from_secs(5));
}

/// Reads WebSocket frames from `stream` into `buf` until one complete frame is buffered;
/// returns it and removes its bytes from `buf`.
async fn next_ws_frame<S: tokio::io::AsyncRead + Unpin>(
    stream: &mut S,
    buf: &mut Vec<u8>,
) -> WsFrame {
    let mut chunk = vec![0u8; 16 * 1024];
    within("next WebSocket frame", async {
        loop {
            if let Some((frame, used)) = parse_ws_frame(buf) {
                buf.drain(..used);
                return frame;
            }
            let n = stream.read(&mut chunk).await.expect("read failed");
            assert!(n > 0, "EOF inside a WebSocket frame");
            buf.extend_from_slice(&chunk[..n]);
        }
    })
    .await
}

/// A synthetic provider: HTTP requests are answered `reply:<body>` on keep-alive; a
/// WebSocket upgrade is accepted and every text message echoed `echo:<payload>` until the
/// closing handshake.
async fn serve_echo(mut up: UpstreamTls) {
    loop {
        let (head, mut body) = read_head(&mut up).await;
        if head.is_empty() {
            let _ = up.shutdown().await;
            return;
        }
        if contains(&head, b"Upgrade: websocket") {
            send(&mut up, &ws_101()).await;
            let mut buf = body;
            loop {
                let frame = next_ws_frame(&mut up, &mut buf).await;
                match frame.opcode {
                    0x1 => {
                        let echo = [&b"echo:"[..], &frame.payload].concat();
                        send(&mut up, &ws_server_text(&echo)).await;
                    }
                    0x8 => {
                        send(&mut up, &ws_server_close()).await;
                        let (rest, _) = read_to_end(&mut up).await;
                        assert!(rest.is_empty() && buf.is_empty());
                        let _ = up.shutdown().await;
                        return;
                    }
                    other => panic!("unexpected opcode {other}"),
                }
            }
        }
        let length = content_length(&head);
        read_at_least(&mut up, &mut body, length).await;
        let reply = [&b"reply:"[..], &body].concat();
        send(&mut up, &ok_response("application/json", &reply)).await;
    }
}

/// Upgrades an intercepted client connection at `/v1/realtime`.
async fn upgrade(client: &mut ClientTls) -> Vec<u8> {
    send(client, &ws_upgrade_request("/v1/realtime")).await;
    let (head, rest) = within("101", read_head(client)).await;
    assert_eq!(head, ws_101());
    rest
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn websocket_101_and_first_data_share_one_write() {
    let clock = clock_start();
    let pki = TestPki::new("upstream test CA");
    let mut upstream = Upstream::start(&pki, &["localhost"]).await;
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut rig = Rig::start(RigConfig::new(&pki), sink.clone()).await;
    let mut client = rig.client(upstream.addr.port()).await;
    let mut up = upstream.accept().await.tls;

    // The client's upgrade request and its first frame travel in one write, and so do the
    // upstream's 101 and its first frame: both prefixes cross the HTTP-to-WebSocket relay
    // hand-off.
    let request = ws_upgrade_request("/v1/realtime");
    let client_first = ws_client_text(b"client-first");
    send(
        &mut client,
        &[request.clone(), client_first.clone()].concat(),
    )
    .await;
    let (head, early) = within("upgrade head", read_head(&mut up)).await;
    assert_eq!(head, request);
    let server_first = ws_server_text(b"server-first");
    send(&mut up, &[ws_101(), server_first.clone()].concat()).await;
    let (head, mut after) = within("101 head", read_head(&mut client)).await;
    assert_eq!(head, ws_101());
    read_at_least(&mut client, &mut after, server_first.len()).await;
    assert_eq!(
        after, server_first,
        "the server prefix was lost or duplicated"
    );
    let mut got = early;
    read_at_least(&mut up, &mut got, client_first.len()).await;
    assert_eq!(
        got, client_first,
        "the client prefix was lost or duplicated"
    );

    let client_second = ws_client_text(b"client-second");
    send(&mut client, &client_second).await;
    let mut got = Vec::new();
    read_at_least(&mut up, &mut got, client_second.len()).await;
    assert_eq!(got, client_second);
    let server_second = ws_server_text(b"server-second");
    send(&mut up, &server_second).await;
    let mut got = Vec::new();
    read_at_least(&mut client, &mut got, server_second.len()).await;
    assert_eq!(got, server_second);
    // The closing handshake, then both ends close.
    send(&mut client, &ws_client_close()).await;
    let mut got = Vec::new();
    read_at_least(&mut up, &mut got, ws_client_close().len()).await;
    assert_eq!(got, ws_client_close());
    send(&mut up, &ws_server_close()).await;
    let mut got = Vec::new();
    read_at_least(&mut client, &mut got, ws_server_close().len()).await;
    assert_eq!(got, ws_server_close());
    close_both(client, up).await;
    rig.wait_closed(1).await;
    let report = rig.stop().await;
    assert_all_clean(&report);
    sink.settle().await;

    eprintln!("evidence websocket trace:\n{}", trace(&sink.retained()));
    let frames = sink.frames();
    check_frames(&frames);
    check_clock(&frames, clock, clock_now());
    let controls = controls(&frames);
    let events: Vec<ConnectionEvent> = controls.iter().map(|c| c.event.clone()).collect();
    assert_eq!(
        events,
        vec![
            ConnectionEvent::Open {
                alpn: Some("http/1.1".into()),
                already_open: false
            },
            ConnectionEvent::UpgradedWebSocket,
            ConnectionEvent::Close {
                outcome: ConnectionOutcome::Complete
            },
        ]
    );
    let connection = controls[0].connection.clone();
    let views = bodies(&frames);
    let handshake: Vec<&StreamView> = views
        .iter()
        .filter(|v| {
            matches!(
                v.source,
                CaptureSource::WireRequest | CaptureSource::WireResponse
            )
        })
        .collect();
    assert_eq!(handshake.len(), 2);
    assert_eq!(handshake[1].meta.status, Some(101));
    for view in &handshake {
        assert_eq!(
            (view.connection(), view.exchange()),
            (connection.as_str(), 1)
        );
        assert!(
            view.end_at.unwrap() < controls[1].at,
            "upgrade recorded before the 101 ended"
        );
    }
    let messages: Vec<&StreamView> = views
        .iter()
        .filter(|v| matches!(v.source, CaptureSource::WsClient | CaptureSource::WsServer))
        .collect();
    assert_eq!(messages.len(), 4);
    for view in &messages {
        assert!(
            view.open_at > controls[1].at,
            "a message before UpgradedWebSocket"
        );
        assert_eq!(
            (view.connection(), view.exchange()),
            (connection.as_str(), 1)
        );
        assert_eq!(view.meta.path.as_deref(), Some("/v1/realtime"));
        assert_eq!(view.meta.protocol_hint.as_deref(), Some("openai-realtime"));
        assert_eq!(view.end, Some(EndReason::Complete));
    }
    let all: Vec<u8> = messages.iter().flat_map(|v| v.bytes.clone()).collect();
    for text in [
        "client-first",
        "server-first",
        "client-second",
        "server-second",
    ] {
        assert_eq!(
            count(&all, text.as_bytes()),
            1,
            "{text} captured other than once"
        );
    }
    let index = |marker: &[u8], source| {
        let view = stream_with(&views, marker);
        assert_eq!(view.source, source);
        view.message_index().unwrap()
    };
    let indexes = [
        index(b"client-first", CaptureSource::WsClient),
        index(b"server-first", CaptureSource::WsServer),
        index(b"client-second", CaptureSource::WsClient),
        index(b"server-second", CaptureSource::WsServer),
    ];
    assert!(
        indexes[0] < indexes[2] && indexes[1] < indexes[3],
        "{indexes:?}"
    );
    let mut sorted = indexes.to_vec();
    sorted.sort();
    assert_eq!(sorted, vec![0, 1, 2, 3]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_http_and_websocket_connections_share_one_emitter() {
    let clock = clock_start();
    const HTTP_CONNS: usize = 6;
    const WS_CONNS: usize = 3;
    const ROUNDS: usize = 4;
    let pki = TestPki::new("upstream test CA");
    let mut upstream = Upstream::start(&pki, &["localhost"]).await;
    let port = upstream.addr.port();
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut rig = Rig::start(RigConfig::new(&pki), sink.clone()).await;
    let server = tokio::spawn(async move {
        let mut providers = tokio::task::JoinSet::new();
        for _ in 0..HTTP_CONNS + WS_CONNS {
            providers.spawn(serve_echo(upstream.accept().await.tls));
        }
        while let Some(done) = providers.join_next().await {
            done.unwrap();
        }
    });

    let mut clients = tokio::task::JoinSet::new();
    for i in 0..HTTP_CONNS {
        let mut client = rig.client(port).await;
        clients.spawn(async move {
            for j in 0..ROUNDS {
                let body = format!("http-{i}-round-{j}").into_bytes();
                send(&mut client, &post("/v1/responses", &body, "")).await;
                let expected = ok_response("application/json", &[&b"reply:"[..], &body].concat());
                let mut got = Vec::new();
                read_at_least(&mut client, &mut got, expected.len()).await;
                assert_eq!(got, expected);
            }
            client.shutdown().await.unwrap();
            let (rest, _) = read_to_end(&mut client).await;
            assert!(rest.is_empty());
        });
    }
    for k in 0..WS_CONNS {
        let mut client = rig.client(port).await;
        clients.spawn(async move {
            let mut buf = upgrade(&mut client).await;
            for j in 0..ROUNDS {
                let text = format!("ws-{k}-msg-{j}").into_bytes();
                send(&mut client, &ws_client_text(&text)).await;
                let frame = next_ws_frame(&mut client, &mut buf).await;
                assert_eq!(frame.payload, [&b"echo:"[..], &text].concat());
            }
            send(&mut client, &ws_client_close()).await;
            let frame = next_ws_frame(&mut client, &mut buf).await;
            assert_eq!(frame.opcode, 0x8);
            client.shutdown().await.unwrap();
            let (rest, _) = read_to_end(&mut client).await;
            assert!(rest.is_empty() && buf.is_empty());
        });
    }
    while let Some(done) = clients.join_next().await {
        done.unwrap();
    }
    within("providers", server).await.unwrap();
    rig.wait_closed((HTTP_CONNS + WS_CONNS) as u64).await;
    let report = rig.stop().await;
    assert_all_clean(&report);
    sink.settle().await;

    let frames = sink.frames();
    check_frames(&frames);
    check_clock(&frames, clock, clock_now());
    assert!(frames.iter().all(|f| f.stream.producer == PRODUCER));
    let keys: std::collections::BTreeSet<_> =
        streams(&frames).iter().map(|v| v.key.clone()).collect();
    let views = streams(&frames);
    assert_eq!(keys.len(), views.len(), "a stream ID was reused");
    let controls = controls(&frames);
    let mut connections = std::collections::BTreeSet::new();
    for i in 0..HTTP_CONNS {
        let mut conn = None;
        for j in 0..ROUNDS {
            let body = format!("http-{i}-round-{j}").into_bytes();
            let reply = [&b"reply:"[..], &body].concat();
            let req: Vec<&StreamView> = views
                .iter()
                .filter(|v| v.source == CaptureSource::WireRequest && v.bytes == body)
                .collect();
            let resp: Vec<&StreamView> = views
                .iter()
                .filter(|v| v.source == CaptureSource::WireResponse && v.bytes == reply)
                .collect();
            assert_eq!((req.len(), resp.len()), (1, 1), "http-{i}-round-{j}");
            let conn = conn.get_or_insert_with(|| req[0].connection().to_string());
            assert_eq!(
                (req[0].connection(), req[0].exchange()),
                (conn.as_str(), j as u64 + 1)
            );
            assert_eq!(
                (resp[0].connection(), resp[0].exchange()),
                (conn.as_str(), j as u64 + 1)
            );
            // Every client uses the same path, so labels are equal across connections.
            assert_eq!(req[0].meta.label, "POST /v1/responses");
        }
        assert!(
            connections.insert(conn.unwrap()),
            "two clients share a connection"
        );
    }
    for k in 0..WS_CONNS {
        let sent: Vec<&StreamView> = (0..ROUNDS)
            .map(|j| {
                let text = format!("ws-{k}-msg-{j}").into_bytes();
                let found: Vec<&StreamView> = views
                    .iter()
                    .filter(|v| v.source == CaptureSource::WsClient && v.bytes == text)
                    .collect();
                assert_eq!(found.len(), 1);
                found[0]
            })
            .collect();
        let echoed: Vec<&StreamView> = (0..ROUNDS)
            .map(|j| {
                let text = format!("echo:ws-{k}-msg-{j}").into_bytes();
                let found: Vec<&StreamView> = views
                    .iter()
                    .filter(|v| v.source == CaptureSource::WsServer && v.bytes == text)
                    .collect();
                assert_eq!(found.len(), 1);
                found[0]
            })
            .collect();
        let conn = sent[0].connection().to_string();
        assert!(
            connections.insert(conn.clone()),
            "two clients share a connection"
        );
        let mut indexes = std::collections::BTreeSet::new();
        for j in 0..ROUNDS {
            for view in [sent[j], echoed[j]] {
                assert_eq!((view.connection(), view.exchange()), (conn.as_str(), 1));
                assert!(
                    indexes.insert(view.message_index().unwrap()),
                    "index reused"
                );
            }
            if j > 0 {
                assert!(sent[j].message_index() > sent[j - 1].message_index());
                assert!(echoed[j].message_index() > echoed[j - 1].message_index());
                assert!(sent[j].open_at > sent[j - 1].end_at.unwrap());
            }
            assert!(echoed[j].message_index() > sent[j].message_index());
        }
        assert!(
            controls
                .iter()
                .any(|c| c.connection == conn && c.event == ConnectionEvent::UpgradedWebSocket)
        );
    }
    // Every connection opened once and closed Complete; equal labels never merged two.
    for conn in &connections {
        let own: Vec<&ControlView> = controls.iter().filter(|c| &c.connection == conn).collect();
        assert!(matches!(
            own.first().unwrap().event,
            ConnectionEvent::Open {
                already_open: false,
                ..
            }
        ));
        assert_eq!(
            own.last().unwrap().event,
            ConnectionEvent::Close {
                outcome: ConnectionOutcome::Complete
            }
        );
    }
    assert_eq!(connections.len(), HTTP_CONNS + WS_CONNS);
    assert_eq!(
        controls
            .iter()
            .filter(|c| matches!(c.event, ConnectionEvent::Open { .. }))
            .count(),
        HTTP_CONNS + WS_CONNS
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn connection_close_between_messages_is_retained() {
    let clock = clock_start();
    let pki = TestPki::new("upstream test CA");
    let mut upstream = Upstream::start(&pki, &["localhost"]).await;
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut rig = Rig::start(RigConfig::new(&pki), sink.clone()).await;

    // One complete message each way, then the TCP/TLS connection closes with no closing
    // handshake: no payload stream is open, and the lifecycle still says how it ended.
    let mut client = rig.client(upstream.addr.port()).await;
    let mut up = upstream.accept().await.tls;
    send(&mut client, &ws_upgrade_request("/v1/realtime")).await;
    let (head, _) = within("upgrade head", read_head(&mut up)).await;
    assert_eq!(head, ws_upgrade_request("/v1/realtime"));
    send(&mut up, &ws_101()).await;
    let (head, mut buf) = within("101", read_head(&mut client)).await;
    assert_eq!(head, ws_101());
    send(&mut client, &ws_client_text(b"only-client-message")).await;
    let mut up_buf = Vec::new();
    let frame = next_ws_frame(&mut up, &mut up_buf).await;
    assert_eq!(frame.payload, b"only-client-message");
    send(&mut up, &ws_server_text(b"only-server-message")).await;
    let frame = next_ws_frame(&mut client, &mut buf).await;
    assert_eq!(frame.payload, b"only-server-message");
    // Both messages are complete at the sink before anything closes.
    sink.settle().await;
    eventually("both messages ended", || {
        streams(&sink.frames())
            .iter()
            .filter(|v| matches!(v.source, CaptureSource::WsClient | CaptureSource::WsServer))
            .filter(|v| v.end == Some(EndReason::Complete))
            .count()
            == 2
    })
    .await;
    close_both(client, up).await;
    rig.wait_closed(1).await;
    let report = rig.stop().await;
    assert_eq!(report.producer_close, ProducerClose::Queued);
    assert_eq!(report.health.closed_interrupted, 1);
    sink.settle().await;

    let frames = sink.frames();
    check_frames(&frames);
    check_clock(&frames, clock, clock_now());
    let close = controls(&frames).pop().unwrap();
    assert_eq!(
        close.event,
        ConnectionEvent::Close {
            outcome: ConnectionOutcome::Interrupted
        }
    );
    let views = streams(&frames);
    for view in &views {
        assert!(
            view.end_at.unwrap() < close.at,
            "a stream was open at the close"
        );
        assert_eq!(view.end, Some(EndReason::Complete));
    }
    assert!(matches!(
        sink.retained().last(),
        Some(Retained::ProducerClosed { producer, final_seq: Some(_) }) if producer == PRODUCER
    ));
}

/// The three report questions stay apart, and loss that shutdown could not classify is never
/// reported as no loss.
#[test]
fn stop_report_never_calls_unclassified_cuts_clean() {
    use goalcup_cli::proxy::{ProxyHealth, ProxyStopReport};
    let clean = ProxyStopReport {
        drained: true,
        accept_loop_lost: false,
        unterminated: 0,
        producer_close: ProducerClose::Queued,
        health: ProxyHealth::default(),
    };
    assert_all_clean(&clean);
    let with = |edit: &dyn Fn(&mut ProxyStopReport)| {
        let mut report = clean.clone();
        edit(&mut report);
        report
    };
    let lossy: Vec<(&str, ProxyStopReport)> = vec![
        ("cut_unsettled", with(&|r| r.health.cut_unsettled = 1)),
        ("unterminated", with(&|r| r.unterminated = 1)),
        ("accept_loop_lost", with(&|r| r.accept_loop_lost = true)),
        (
            "tunneled_fail_open",
            with(&|r| r.health.tunneled_fail_open = 1),
        ),
        (
            "capture_table_full",
            with(&|r| r.health.capture_table_full = 1),
        ),
        ("opaque_fallbacks", with(&|r| r.health.opaque_fallbacks = 1)),
        ("sink_refusals", with(&|r| r.health.sink_refusals = 1)),
        ("control_refusals", with(&|r| r.health.control_refusals = 1)),
        (
            "refused close",
            with(&|r| r.producer_close = ProducerClose::Refused(GapReason::QueueFull)),
        ),
    ];
    for (what, report) in lossy {
        assert!(
            report.capture_loss(),
            "{what} is not reported as capture loss"
        );
    }
    // Loss alone does not make the drain unclean, and a drain alone is not a transport
    // failure: each predicate answers its own question.
    assert!(with(&|r| r.health.sink_refusals = 1).is_clean_drain());
    assert_eq!(with(&|r| r.drained = false).transport_failures(), 0);
    assert!(!with(&|r| r.drained = false).capture_loss());
    assert_eq!(
        with(&|r| r.health.closed_transport_error = 1).transport_failures(),
        1
    );
    assert!(!with(&|r| r.health.closed_transport_error = 1).capture_loss());
}

/// Connections to a provider host refused while it is dark are one outage window, not one per
/// connection; a dead host that is not a provider's (the agent's own traffic) is not an outage;
/// and the next admission that reaches a provider host closes the window.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_upstream_outage_is_one_window_closed_by_the_next_reach() {
    let pki = TestPki::new("upstream test CA");
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut config = RigConfig::new(&pki);
    // An authentication host is a provider host whose failed connect is answered `502`.
    config.approved = vec![approved("api.test", policy(&[("/", true, None)]))];
    config.auth_hosts = vec!["localhost".into()];
    let mut rig = Rig::start(config, sink.clone()).await;
    let dark = {
        let reserved = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        reserved.local_addr().unwrap().port()
    };
    let refused = |addr, target: String| async move {
        let mut tcp = TcpStream::connect(addr).await.unwrap();
        tcp.write_all(&connect_head(&target)).await.unwrap();
        let (answer, _) = read_to_end(&mut tcp).await;
        assert!(answer.starts_with(b"HTTP/1.1 502 "), "{answer:?}");
    };
    tokio::join!(
        refused(rig.addr, format!("localhost:{dark}")),
        refused(rig.addr, format!("localhost:{dark}")),
        refused(rig.addr, format!("127.0.0.1:{dark}")),
    );
    let handle = rig.handle();
    eventually("all three failures counted", || {
        handle.health().admission_failures.upstream_connect == 3
    })
    .await;
    let dark_health = handle.health();
    assert_eq!(dark_health.upstream_outages, 1);
    assert_eq!(dark_health.outage_windows.len(), 1);
    let open = dark_health.outage_windows[0].clone();
    assert_eq!(
        open.failures, 2,
        "the host that is not a provider's counted"
    );
    assert!(open.first_failure <= open.last_failure);
    assert_eq!(open.recovered, None);

    // The provider host is back: the next tunnel to it closes the window when it is decided,
    // which is once the client's first bytes arrive (the recording proxy reads the server name in
    // them).
    let upstream = Upstream::start(&pki, &["localhost"]).await;
    let mut tunnel = connect_via(rig.addr, &format!("localhost:{}", upstream.addr.port())).await;
    tunnel.write_all(b"not a ClientHello").await.unwrap();
    eventually("the window closes", || {
        handle.health().outage_windows[0].recovered.is_some()
    })
    .await;
    drop(tunnel);
    let report = rig.stop().await;
    assert_eq!(report.health.upstream_outages, 1);
    let closed = &report.health.outage_windows[0];
    assert_eq!(closed.failures, 2);
    assert_eq!(closed.first_failure, open.first_failure);
    let recovered = closed.recovered.as_deref().expect("the window closed");
    assert!(recovered >= closed.last_failure.as_str());
}

/// A dead accept loop is restarted on the same port: the address the harness already has serves a
/// new connection, and its request is recorded. The test kills the loop; the supervisor binds the
/// port again and runs a new loop over the same shared state.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_dead_accept_loop_is_restarted_on_the_same_port() {
    let clock = clock_start();
    let pki = TestPki::new("upstream test CA");
    let mut upstream = Upstream::start(&pki, &["localhost"]).await;
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut rig = Rig::start(RigConfig::new(&pki), sink.clone()).await;
    let addr = rig.addr;
    rig.handle().kill_accept_for_test();
    eventually("the recorder restarts", || {
        rig.handle().health().recorder_restarts == 1
    })
    .await;
    assert_eq!(rig.handle().local_addr(), addr);
    assert!(!rig.handle().health().recorder_lost);
    let mut client = rig.client(upstream.addr.port()).await;
    let mut up = upstream.accept().await.tls;
    exchange(
        &mut client,
        &mut up,
        "/v1/messages",
        b"{\"messages\":[]}",
        b"{\"id\":\"m\"}",
    )
    .await;
    close_both(client, up).await;
    rig.wait_closed(1).await;
    let recorded = rig
        .handle()
        .recorded_requests()
        .load(std::sync::atomic::Ordering::SeqCst);
    assert_all_clean(&rig.stop().await);
    sink.settle().await;

    let frames = sink.frames();
    check_frames(&frames);
    check_clock(&frames, clock, clock_now());
    assert_eq!(recorded, 1);
    let requests = bodies(&frames)
        .iter()
        .filter(|v| v.source == CaptureSource::WireRequest)
        .count();
    assert_eq!(requests, 1);
}

/// The recorder is restarted at most five times in a run. The sixth death is not restarted: the
/// port is no longer served, and the run reports the recorder lost.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_recorder_is_restarted_at_most_five_times() {
    let pki = TestPki::new("upstream test CA");
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut rig = Rig::start(RigConfig::new(&pki), sink.clone()).await;
    for restarts in 1..=5u64 {
        rig.handle().kill_accept_for_test();
        eventually("a restart", || {
            rig.handle().health().recorder_restarts == restarts
        })
        .await;
    }
    rig.handle().kill_accept_for_test();
    eventually("the sixth death is reported", || {
        rig.handle().health().recorder_lost
    })
    .await;
    let health = rig.handle().health();
    assert_eq!(health.recorder_restarts, 5);
    assert!(health.recorder_lost);
    assert!(
        TcpStream::connect(rig.addr).await.is_err(),
        "the port is still served after the sixth death"
    );
    let report = rig.stop().await;
    assert!(report.accept_loop_lost, "{report:?}");
    assert_eq!(report.health.recorder_restarts, 5);
}

/// The restart budget is the run's, not the launch's: a launch that continues a run which already
/// restarted its recorder four times has one restart left, and the next death is the one past the
/// limit. The count it starts from shows in its health.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_continued_launch_keeps_the_runs_restart_count() {
    let pki = TestPki::new("upstream test CA");
    let sink = DirectSink::new(QUEUE_BYTES);
    let mut config = RigConfig::new(&pki);
    config.recorder_restarts_before = 4;
    let mut rig = Rig::start(config, sink.clone()).await;
    assert_eq!(rig.handle().health().recorder_restarts, 4);
    rig.handle().kill_accept_for_test();
    eventually("the last restart of the run", || {
        rig.handle().health().recorder_restarts == 5
    })
    .await;
    assert!(!rig.handle().health().recorder_lost);
    rig.handle().kill_accept_for_test();
    eventually("the next death is past the run's limit", || {
        rig.handle().health().recorder_lost
    })
    .await;
    assert_eq!(rig.handle().health().recorder_restarts, 5);
    assert!(
        TcpStream::connect(rig.addr).await.is_err(),
        "the port is still served after the run's limit"
    );
    let report = rig.stop().await;
    assert!(report.accept_loop_lost, "{report:?}");
    assert_eq!(report.health.recorder_restarts, 5);
}
