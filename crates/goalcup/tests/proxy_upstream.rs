//! cr-proxy-2: the upstream leg goes through the participant's own proxy (`HTTPS_PROXY`), with a
//! CONNECT of our own; a refusing proxy is a `502` and one note at once, a failing one is retried;
//! a test upstream never goes through it.

mod common;
mod proxy_fixtures;

use common::pki::HTTP11;
use common::recorder::events;
use common::upstream::{Script, Upstream};
use common::{HANG, within_async};
use goalcup::net::Egress;
use goalcup::pipeline::Calls;
use proxy_fixtures::{HOSTS, client_trusting, connect, conns, get, through};
use rustls::pki_types::ServerName;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_rustls::TlsConnector;

const TARGET: &str = "api.openai.com:443";

fn name(host: &'static str) -> ServerName<'static> {
    ServerName::try_from(host).unwrap()
}

/// [`proxy_fixtures::proxy`] with the machine's own proxy variables cleared first, so only the
/// test's `env` decides (a CI or sandbox may export `https_proxy`, which wins over `HTTPS_PROXY`).
fn proxy(
    env: &[(&str, String)],
) -> (
    goalcup::proxy::ProxyHandle,
    SocketAddr,
    Calls,
    goalcup::proxy::ca::RunCa,
) {
    for k in [
        "https_proxy",
        "HTTPS_PROXY",
        "all_proxy",
        "ALL_PROXY",
        "no_proxy",
        "NO_PROXY",
    ] {
        // SAFETY: nextest runs each test in its own process; this runs before `serve` reads them.
        unsafe { std::env::remove_var(k) };
    }
    proxy_fixtures::proxy(HOSTS, env)
}

/// A participant's proxy: for each connection reads the head to the blank line and keeps it,
/// then writes `answer`, or `200` and pipes both ways to `to`.
fn fake_proxy(
    answer: Option<&'static [u8]>,
    to: Option<SocketAddr>,
) -> (SocketAddr, Arc<Mutex<Vec<String>>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let at = l.local_addr().unwrap();
    let heads = Arc::new(Mutex::new(Vec::new()));
    let seen = heads.clone();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { return };
            let mut head = Vec::new();
            let mut b = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                match s.read(&mut b) {
                    Ok(1) => head.push(b[0]),
                    _ => break,
                }
            }
            seen.lock()
                .unwrap()
                .push(String::from_utf8_lossy(&head).into_owned());
            match (answer, to) {
                (Some(answer), _) => {
                    let _ = s.write_all(answer);
                }
                (None, Some(to)) => {
                    let up = TcpStream::connect(to).unwrap();
                    s.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                        .unwrap();
                    let (mut a, mut b) = (s.try_clone().unwrap(), up.try_clone().unwrap());
                    let (mut s, mut up) = (s, up);
                    std::thread::spawn(move || std::io::copy(&mut a, &mut b));
                    std::thread::spawn(move || std::io::copy(&mut up, &mut s));
                }
                (None, None) => {}
            }
        }
    });
    (at, heads)
}

fn notes(calls: &Calls, what: &str) -> Vec<Value> {
    (events(calls).into_iter())
        .filter(|v| v["what"] == what)
        .collect()
}

/// cr-proxy-2: a model call goes through the participant's proxy with its credentials, and is
/// still decrypted and recorded.
#[tokio::test]
async fn model_calls_go_through_the_participants_proxy() {
    let up = Upstream::start(Script::ok(b"hi").with_tls(&["api.openai.com"], &[HTTP11]));
    let (fake, heads) = fake_proxy(None, Some(up.addr()));
    let dir = tempfile::tempdir().unwrap();
    let env = [
        ("HTTPS_PROXY", format!("http://u:s3cret@{fake}")),
        (
            "GOALCUP_TEST_EXTRA_ROOTS",
            up.pki().roots_file(dir.path()).display().to_string(),
        ),
    ];
    let (_h, addr, calls, ca) = proxy(&env);
    let c = TlsConnector::from(client_trusting(vec![ca.cert_der().clone()], &[b"http/1.1"]));
    within_async(HANG, async {
        let mut t = (c.connect(name("api.openai.com"), through(addr, TARGET).await))
            .await
            .unwrap();
        get(&mut t).await;
    })
    .await;
    assert_eq!(
        *heads.lock().unwrap(),
        [
            "CONNECT api.openai.com:443 HTTP/1.1\r\nHost: api.openai.com:443\r\nProxy-Authorization: Basic dTpzM2NyZXQ=\r\n\r\n"
        ]
    );
    let got = conns(&calls, 1).await;
    assert_eq!(got[0].1["mode"], "decrypt");
    let all = format!("{:?}", events(&calls));
    assert!(
        !all.contains("s3cret") && !all.contains("dTpzM2NyZXQ="),
        "{all}"
    );
}

/// Review focus 4: a proxy that refuses (407) gives the harness a `502` and one note at once:
/// a 4xx is final, never retried.
#[tokio::test]
async fn a_proxy_that_refuses_is_a_502_with_one_note() {
    let (fake, heads) = fake_proxy(
        Some(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n"),
        None,
    );
    let (_h, addr, calls, _) = proxy(&[("HTTPS_PROXY", format!("http://{fake}"))]);
    let code = within_async(Duration::from_secs(5), connect(addr, TARGET))
        .await
        .1;
    assert_eq!(code, 502);
    assert_eq!(heads.lock().unwrap().len(), 1);
    let notes = notes(&calls, "connect_refused");
    assert_eq!(
        (notes.len(), &notes[0]["status"], &notes[0]["host"]),
        (1, &json!(502), &json!("api.openai.com"))
    );
}

/// A `GOALCUP_TEST_UPSTREAM` target is dialled directly, even with a (dead) proxy set.
#[tokio::test]
async fn a_test_upstream_never_goes_through_the_proxy() {
    let up = Upstream::start(Script::ok(b"hi").with_tls(&["api.openai.com"], &[HTTP11]));
    let (closed_guard, closed) = common::ports::refusing();
    let dir = tempfile::tempdir().unwrap();
    let env = [
        ("HTTPS_PROXY", format!("http://{closed}")),
        (
            "GOALCUP_TEST_UPSTREAM",
            format!("api.openai.com={}", up.addr()),
        ),
        (
            "GOALCUP_TEST_EXTRA_ROOTS",
            up.pki().roots_file(dir.path()).display().to_string(),
        ),
    ];
    let (_h, addr, calls, ca) = proxy(&env);
    let c = TlsConnector::from(client_trusting(vec![ca.cert_der().clone()], &[b"http/1.1"]));
    within_async(HANG, async {
        let mut t = (c.connect(name("api.openai.com"), through(addr, TARGET).await))
            .await
            .unwrap();
        get(&mut t).await;
    })
    .await;
    assert_eq!(conns(&calls, 1).await[0].1["mode"], "decrypt");
    drop(closed_guard);
}

/// A proxy's 5xx (it could not reach the target now) is retried for a listed host, like a dial
/// failure.
#[tokio::test]
async fn a_proxy_5xx_is_retried() {
    let (fake, heads) = fake_proxy(Some(b"HTTP/1.1 503 Service Unavailable\r\n\r\n"), None);
    let (_h, addr, _calls, _) = proxy(&[("HTTPS_PROXY", format!("http://{fake}"))]);
    let asking = tokio::spawn(connect(addr, TARGET));
    within_async(Duration::from_secs(5), async {
        while heads.lock().unwrap().len() < 2 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    asking.abort();
}

/// An IPv6 target is bracketed in the CONNECT line and the Host header.
#[test]
fn an_ipv6_target_is_bracketed_in_the_connect_head() {
    let vars = BTreeMap::from([("HTTPS_PROXY".to_owned(), "http://p:3128".to_owned())]);
    let e = Egress::from_vars(&vars, vec![]);
    assert_eq!(
        e.proxy_for("2001:db8::1")
            .unwrap()
            .connect_head("2001:db8::1", 443),
        "CONNECT [2001:db8::1]:443 HTTP/1.1\r\nHost: [2001:db8::1]:443\r\n\r\n"
    );
}
