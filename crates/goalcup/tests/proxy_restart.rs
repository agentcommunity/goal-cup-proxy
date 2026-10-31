//! PRD-03 section 10: a second boot serves the listener the watchdog holds, with the same CA, and
//! `quiet_since` follows relayed bytes only.

mod common;
mod proxy_fixtures;

use common::pki::HTTP11;
use common::upstream::{Script, Upstream};
use common::{HANG, within_async};
use goalcup::proxy::ca::{RUN_CA_FILE, RunCa, certificates};
use proxy_fixtures::{HOSTS, client_trusting, get, serve_on, status, through};
use rustls::pki_types::{CertificateDer, ServerName};
use std::time::Duration;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;

/// A connection queued in the backlog while no boot runs is served by the next boot, whose
/// imported CA signs a leaf the first boot's `run-ca.pem` trusts.
#[tokio::test]
async fn the_next_boot_serves_the_backlog_with_the_same_ca() {
    let up = Upstream::start(Script::ok(b"hi").with_tls(&["api.openai.com"], &[HTTP11]));
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: nextest runs each test in its own process; set before either `serve` reads them.
    unsafe {
        std::env::set_var(
            "GOALCUP_TEST_UPSTREAM",
            format!("api.openai.com={}", up.addr()),
        );
        std::env::set_var("GOALCUP_TEST_EXTRA_ROOTS", up.pki().roots_file(dir.path()));
    }
    let (listener, addr) = common::ports::held();
    let first = RunCa::mint(&["openai.com".to_string()]).unwrap();
    first.write_files(dir.path()).unwrap();
    drop(serve_on(&listener, HOSTS, first.clone()));

    let mut queued = TcpStream::connect(addr).await.unwrap();
    let head = b"CONNECT api.openai.com:443 HTTP/1.1\r\nHost: api.openai.com\r\n\r\n";
    queued.write_all(head).await.unwrap();
    let second = RunCa::import(&first.export()).unwrap();
    let _second = serve_on(&listener, HOSTS, second);
    assert_eq!(within_async(HANG, status(&mut queued)).await, 200);

    let pem = std::fs::read(dir.path().join(RUN_CA_FILE)).unwrap();
    let roots = certificates(&pem).into_iter().map(CertificateDer::from);
    let connector = tokio_rustls::TlsConnector::from(client_trusting(roots.collect(), &[HTTP11]));
    let name = ServerName::try_from("api.openai.com").unwrap();
    let mut t = connector
        .connect(name, queued)
        .await
        .expect("the leaf verifies");
    get(&mut t).await;
}

/// `quiet_since` moves when a relay moves bytes; an open connection that relays nothing leaves it.
#[tokio::test]
async fn quiet_since_moves_with_relayed_bytes_only() {
    let (echo, target) = common::ports::held();
    std::thread::spawn(move || {
        for s in echo.incoming().flatten() {
            std::thread::spawn(move || {
                let mut r = s;
                let mut w = r.try_clone().unwrap();
                let _ = std::io::copy(&mut r, &mut w);
            });
        }
    });
    let (listener, addr) = common::ports::held();
    let ca = RunCa::mint(&["openai.com".to_string()]).unwrap();
    let (h, _calls) = serve_on(&listener, HOSTS, ca);
    let before = h.quiet_since();
    let mut busy = through(addr, &target.to_string()).await;
    busy.write_all(b"ping").await.unwrap();
    let mut back = [0u8; 4];
    busy.read_exact(&mut back).await.unwrap();
    // The relay counts a chunk just after writing it on: wait until it moved and holds still.
    let moved = within_async(HANG, async {
        loop {
            let seen = h.quiet_since();
            tokio::time::sleep(Duration::from_millis(50)).await;
            if seen > before && h.quiet_since() == seen {
                return seen;
            }
        }
    })
    .await;

    // A second connection that relays nothing, given a moment to move bytes it never has.
    let _idle = through(addr, &target.to_string()).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        h.quiet_since(),
        moved,
        "an open silent connection is not traffic"
    );
}
