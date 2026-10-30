//! PRD-03 `proxy_tunnel.rs`: the relay pump (rules 1, 4, 8, section 5a, `quiet`) and tunnels
//! (section 9: counts, the listed-host GAP, the per-host `long_tunnel` flag, reset passing).

mod common;

use bytes::Bytes;
use common::recorder::gaps;
use common::{HANG, within_async};
use goalcup::connector::hosts::{Kind, Source};
use goalcup::pipeline::{Call, Calls, EndReason, GapReason, Mono, OpenFlags, Recorder, StreamKind};
use goalcup::proxy::pump::{self, Parser};
use goalcup::proxy::tunnel::{self, Flags, TunnelConn};
use goalcup::proxy::ws::Dir;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::os::fd::AsFd;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

/// A connected loopback pair: (test side, proxy side).
async fn pair() -> (TcpStream, TcpStream) {
    let (listener, addr) = common::ports::held();
    listener.set_nonblocking(true).unwrap();
    let listener = TcpListener::from_std(listener).unwrap();
    let a = TcpStream::connect(addr).await.unwrap();
    (a, listener.accept().await.unwrap().0)
}

/// The harness side, the upstream side, and the relay task between them.
struct Relay {
    client: TcpStream,
    upstream: TcpStream,
    done: JoinHandle<()>,
}

async fn tunnel(rec: &Recorder, flags: &Arc<Flags>, c: TunnelConn, cp: &'static [u8]) -> Relay {
    let ((client, cproxy), (upstream, uproxy)) = (pair().await, pair().await);
    let (rec, flags) = (rec.clone(), flags.clone());
    let done = tokio::spawn(async move {
        let (quiet, cp) = (AtomicU64::new(0), Bytes::from_static(cp));
        tunnel::run(
            &rec,
            &flags,
            &quiet,
            c,
            cproxy,
            cp,
            uproxy,
            Bytes::from_static(b"srv"),
        )
        .await;
    });
    let mut r = Relay {
        client,
        upstream,
        done,
    };
    let (mut up, mut down) = (vec![0; cp.len()], [0; 3]);
    r.upstream.read_exact(&mut up).await.unwrap();
    r.client.read_exact(&mut down).await.unwrap();
    assert_eq!(
        (&up[..], &down),
        (cp, b"srv"),
        "prefixes are replayed first"
    );
    r
}

async fn pumped(rec: &Recorder, parser: Option<Box<dyn Parser>>, quiet: Arc<AtomicU64>) -> Relay {
    let ((client, cproxy), (upstream, uproxy)) = (pair().await, pair().await);
    let rec = rec.clone();
    let done = tokio::spawn(async move {
        let mut conn = rec.open(StreamKind::Conn, 0, 0, 0, OpenFlags::NONE, json!({}));
        pump::run(&rec, &mut conn, cproxy, uproxy, parser, &quiet).await;
        conn.end(rec.now(), EndReason::Complete, None);
    });
    Relay {
        client,
        upstream,
        done,
    }
}

/// Sends `n` bytes from `from` and reads them all at `to`.
async fn pass(from: &mut TcpStream, to: &mut TcpStream, byte: u8, n: usize) -> Vec<u8> {
    let send = async { from.write_all(&vec![byte; n]).await.unwrap() };
    let mut got = vec![0; n];
    let (_, read) = tokio::join!(send, to.read_exact(&mut got));
    read.unwrap();
    got
}

/// Both sides close cleanly, then the relay ends.
async fn close(r: Relay) {
    let Relay {
        client,
        mut upstream,
        done,
    } = r;
    drop(client);
    assert_eq!(upstream.read(&mut [0; 64]).await.unwrap(), 0);
    drop(upstream);
    done.await.unwrap();
}

fn conn(host: &str, reason: &'static str, listed: Option<(Source, Kind)>) -> TunnelConn {
    let (host, fingerprint) = (host.to_owned(), None);
    TunnelConn {
        host,
        port: 443,
        reason,
        listed,
        fingerprint,
    }
}

fn notes(calls: &Calls, what: &str) -> Vec<Value> {
    let is = |ev: &Value| ev["what"] == what;
    let all = calls.all().into_iter();
    all.filter_map(|c| {
        if let Call::Event(ev) = c {
            is(&ev).then_some(ev)
        } else {
            None
        }
    })
    .collect()
}

fn end_tail(calls: &Calls) -> Value {
    let ends = calls.all().into_iter().filter_map(|c| match c {
        Call::End(_, _, _, _, tail) => tail,
        _ => None,
    });
    ends.last().expect("a conn END with a tail")
}

#[tokio::test]
async fn up_and_down_counts_are_exact_prefixes_included() {
    within_async(HANG, async {
        let (rec, calls) = Recorder::for_tests(1 << 30);
        let flags = Arc::new(Flags::default());
        let mut r = tunnel(
            &rec,
            &flags,
            conn("example.org", "not_listed", None),
            b"hello",
        )
        .await;
        pass(&mut r.client, &mut r.upstream, 1, 1000).await;
        pass(&mut r.upstream, &mut r.client, 2, 2000).await;
        close(r).await;
        let tail = end_tail(&calls);
        assert_eq!(
            (tail["up"].as_u64(), tail["down"].as_u64()),
            (Some(1005), Some(2003))
        );
        assert!(gaps(&calls).is_empty());
    })
    .await;
}

#[tokio::test]
async fn a_listed_tunnel_is_a_named_loss_and_sign_in_is_not() {
    within_async(HANG, async {
        let guard = json!({"guard": "client_hello", "limit": 65536});
        let listed = Some((Source::Shared, Kind::Provider));
        let cases = [
            ("pinned", listed, Some((GapReason::Pinned, None))),
            (
                "hello_too_big",
                listed,
                Some((GapReason::MemoryGuard, Some(guard))),
            ),
            ("sign_in", None, None),
        ];
        for (reason, listed, want) in cases {
            let (rec, calls) = Recorder::for_tests(1 << 30);
            let flags = Arc::new(Flags::default());
            let mut r = tunnel(&rec, &flags, conn("api.example", reason, listed), b"hi").await;
            pass(&mut r.client, &mut r.upstream, 1, 12).await;
            pass(&mut r.upstream, &mut r.client, 2, 23).await;
            close(r).await;
            let got = gaps(&calls).into_iter().map(|g| match g {
                Call::Gap(_, reason, start, end, _, lost, tail) => {
                    assert!(start <= end);
                    (reason, lost, tail)
                }
                _ => unreachable!(),
            });
            let got: Vec<_> = got.collect();
            // Up: 2 replayed + 12; down: 3 replayed + 23.
            let want: Vec<_> = want.into_iter().map(|(g, t)| (g, Some(40), t)).collect();
            assert_eq!(got, want, "{reason}");
        }
    })
    .await;
}

#[tokio::test]
async fn the_flag_fires_once_for_a_stream() {
    let (rec, calls) = Recorder::for_tests(1 << 30);
    let flags = Flags::default();
    let base = 7_000_000_000_000;
    let at = |s: u64| Mono(base + s * 1_000_000_000);
    let mut w = flags.watch("stream.example", at(0));
    for s in 0..60 {
        flags.observe(&rec, &mut w, at(s), Dir::Down, 300);
    }
    assert!(notes(&calls, "long_tunnel").is_empty(), "open under 60 s");
    let mut second = flags.watch("stream.example", at(60));
    for s in 60..200 {
        flags.observe(&rec, &mut w, at(s), Dir::Down, 300);
        flags.observe(&rec, &mut second, at(s), Dir::Down, 300);
    }
    let got = notes(&calls, "long_tunnel");
    assert_eq!(got.len(), 1);
    assert_eq!(
        (&got[0]["host"], &got[0]["why"]),
        (&json!("stream.example"), &json!("stream"))
    );
}

#[tokio::test]
async fn the_flag_fires_once_for_an_upload_and_never_for_a_download() {
    within_async(HANG, async {
        let (rec, calls) = Recorder::for_tests(1 << 30);
        let flags = Arc::new(Flags::default());
        for _ in 0..6 {
            let mut r = tunnel(&rec, &flags, conn("up.example", "not_listed", None), b"").await;
            pass(&mut r.client, &mut r.upstream, 1, 256 << 10).await;
            close(r).await;
        }
        let mut r = tunnel(&rec, &flags, conn("down.example", "not_listed", None), b"").await;
        pass(&mut r.upstream, &mut r.client, 2, 50 << 20).await;
        close(r).await;
        let got = notes(&calls, "long_tunnel");
        assert_eq!(got.len(), 1);
        assert_eq!(
            (&got[0]["host"], &got[0]["why"]),
            (&json!("up.example"), &json!("upload"))
        );
    })
    .await;
}

#[tokio::test]
async fn an_upstream_reset_reaches_the_client_as_a_reset_and_a_close_as_a_close() {
    within_async(HANG, async {
        let (rec, _calls) = Recorder::for_tests(1 << 30);
        let flags = Arc::new(Flags::default());
        let mut r = tunnel(&rec, &flags, conn("a.example", "not_listed", None), b"").await;
        pass(&mut r.client, &mut r.upstream, 1, 10).await;
        socket2::SockRef::from(&r.upstream)
            .set_linger(Some(Duration::ZERO))
            .unwrap();
        let Relay {
            mut client,
            upstream,
            done,
        } = r;
        drop(upstream);
        let mut buf = [0u8; 16];
        let err = client.read(&mut buf).await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::ConnectionReset);
        done.await.unwrap();

        let mut r = tunnel(&rec, &flags, conn("b.example", "not_listed", None), b"").await;
        pass(&mut r.upstream, &mut r.client, 2, 3).await;
        drop(r.upstream);
        assert_eq!(r.client.read(&mut buf).await.unwrap(), 0);
        drop(r.client);
        r.done.await.unwrap();
    })
    .await;
}

/// Peers (client, upstream test sides), feeds, feeds that found their chunk already waiting at
/// the peer (a non-blocking peek).
#[derive(Default)]
struct AtPeer(Arc<OnceLock<[std::net::TcpStream; 2]>>, Arc<AtomicUsize>, Arc<AtomicUsize>);

impl Parser for AtPeer {
    fn feed(&mut self, dir: Dir, _: Mono, bytes: &[u8]) -> Result<(), GapReason> {
        let peer = &self.0.get().unwrap()[usize::from(dir == Dir::Up)];
        let mut b = vec![0; bytes.len()];
        if peer.peek(&mut b).is_ok_and(|n| b[..n] == *bytes) {
            self.2.fetch_add(1, Ordering::SeqCst);
        }
        self.1.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn end(self: Box<Self>, _: Mono, _: EndReason, _: u64) {}
}

async fn fed(n: &AtomicUsize, k: usize) {
    while n.load(Ordering::SeqCst) < k {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

#[tokio::test]
async fn bytes_reach_the_peer_before_the_parser_and_sse_before_eof() {
    within_async(HANG, async {
        let (rec, _calls) = Recorder::for_tests(1 << 30);
        let parser = AtPeer::default();
        let (peers, feeds, ok) = (parser.0.clone(), parser.1.clone(), parser.2.clone());
        let mut r = pumped(&rec, Some(Box::new(parser)), Arc::default()).await;
        let dup = |s: &TcpStream| std::net::TcpStream::from(s.as_fd().try_clone_to_owned().unwrap());
        peers.set([dup(&r.client), dup(&r.upstream)]).unwrap();
        r.client.write_all(&[1; 100]).await.unwrap();
        fed(&feeds, 1).await;
        r.upstream.write_all(b"data: 1\n\n").await.unwrap();
        fed(&feeds, 2).await;
        let (mut sse, mut req) = ([0; 9], [0; 100]);
        r.client.read_exact(&mut sse).await.unwrap();
        r.upstream.read_exact(&mut req).await.unwrap();
        close(r).await;
        assert_eq!(ok.load(Ordering::SeqCst), 2);
    })
    .await;
}

/// Panics on every feed, or returns `decode_failed` on a chunk of `b'!'`; counts its feeds.
struct Faulty(bool, Arc<AtomicUsize>);

impl Parser for Faulty {
    fn feed(&mut self, _: Dir, _: Mono, bytes: &[u8]) -> Result<(), GapReason> {
        self.1.fetch_add(1, Ordering::SeqCst);
        assert!(!self.0, "a parser bug");
        if bytes[0] == b'!' {
            Err(GapReason::DecodeFailed)
        } else {
            Ok(())
        }
    }
    fn end(self: Box<Self>, _: Mono, _: EndReason, _: u64) {}
}

#[tokio::test]
async fn a_parse_fault_keeps_relaying_and_counts_what_follows() {
    within_async(HANG, async {
        for panics in [true, false] {
            let (rec, calls) = Recorder::for_tests(1 << 30);
            let fed = Arc::new(AtomicUsize::new(0));
            let parser = Box::new(Faulty(panics, fed.clone()));
            let mut r = pumped(&rec, Some(parser), Arc::default()).await;
            pass(&mut r.client, &mut r.upstream, b'a', 40).await;
            pass(&mut r.upstream, &mut r.client, b'b', 50).await;
            pass(&mut r.client, &mut r.upstream, b'!', 7).await;
            pass(&mut r.client, &mut r.upstream, b'c', 30).await;
            pass(&mut r.upstream, &mut r.client, b'd', 2000).await;
            close(r).await;
            let (want_fed, tail, lost) = match panics {
                true => (1, Some(json!({"panic": true})), 50 + 7 + 30 + 2000),
                false => (3, None, 30 + 2000),
            };
            assert_eq!(fed.load(Ordering::SeqCst), want_fed);
            let stopped = notes(&calls, "parse_stopped");
            assert_eq!(stopped.len(), 1);
            assert_eq!(stopped[0]["reason"], "decode_failed");
            let all = gaps(&calls);
            let [Call::Gap(sid, GapReason::DecodeFailed, _, _, _, got, got_tail)] = &all[..] else {
                panic!("one conn GAP decode_failed");
            };
            assert_eq!(
                (stopped[0]["conn"].as_u64(), *got, got_tail),
                (Some(*sid), Some(lost), &tail)
            );
        }
    })
    .await;
}

#[tokio::test]
async fn quiet_moves_with_relayed_bytes_not_with_an_idle_connection() {
    within_async(HANG, async {
        let (rec, _calls) = Recorder::for_tests(1 << 30);
        let quiet = Arc::new(AtomicU64::new(0));
        let mut r = pumped(&rec, None, quiet.clone()).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(quiet.load(Ordering::SeqCst), 0);
        let before = rec.now().ns();
        pass(&mut r.client, &mut r.upstream, 1, 10).await;
        while quiet.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        let moved = quiet.load(Ordering::SeqCst);
        assert!(moved >= before);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(quiet.load(Ordering::SeqCst), moved);
        close(r).await;
    })
    .await;
}
