//! PRD-03 `proxy_tunnel.rs`: the relay pump (rules 1, 4, 8, section 5a, `quiet`) and tunnels
//! (section 9: counts, the listed-host GAP, the per-host `long_tunnel` flag, reset passing).

mod common;

use bytes::Bytes;
use common::recorder::gaps;
use common::{HANG, within_async};
use goalcup::connector::hosts::{Kind, Source};
use goalcup::pipeline::{
    Call, Calls, EndReason, GapReason, Mono, OpenFlags, Recorder, Sid, StreamKind,
};
use goalcup::proxy::h2::H2;
use goalcup::proxy::http1::Http1;
use goalcup::proxy::pump::{self, Parser, Stop};
use goalcup::proxy::tunnel::{self, Flags, TunnelConn};
use goalcup::proxy::ws::Dir;
use serde_json::{Value, json};
use std::future::Future;
use std::os::fd::AsFd;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, DuplexStream};
use tokio::io::{Join, ReadBuf, ReadHalf, WriteHalf, duplex};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
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

/// Both loopback pairs and a task relaying between their proxy sides.
async fn spawn<F>(relay: impl FnOnce(TcpStream, TcpStream) -> F) -> Relay
where
    F: Future<Output = ()> + Send + 'static,
{
    let ((client, cproxy), (upstream, uproxy)) = (pair().await, pair().await);
    let done = tokio::spawn(relay(cproxy, uproxy));
    Relay {
        client,
        upstream,
        done,
    }
}

async fn tunnel(rec: &Recorder, flags: &Arc<Flags>, c: TunnelConn) -> Relay {
    let (rec, flags) = (rec.clone(), flags.clone());
    let cp = c.client_prefix.clone();
    let mut r = spawn(|cproxy, uproxy| async move {
        let quiet = AtomicU64::new(0);
        tunnel::run(&rec, &flags, &quiet, c, cproxy, uproxy).await;
    })
    .await;
    let (mut up, mut down) = (vec![0; cp.len()], [0; 3]);
    r.upstream.read_exact(&mut up).await.unwrap();
    r.client.read_exact(&mut down).await.unwrap();
    assert_eq!(
        (&up[..], &down),
        (&cp[..], b"srv"),
        "prefixes are replayed first"
    );
    r
}

/// A pump over a `conn` stream; `parser` is made from the conn's sid.
async fn pumped<P>(rec: &Recorder, parser: P, quiet: Arc<AtomicU64>) -> Relay
where
    P: FnOnce(Sid) -> Option<Box<dyn Parser>> + Send + 'static,
{
    let rec = rec.clone();
    spawn(|cproxy, uproxy| async move {
        let mut conn = rec.open(StreamKind::Conn, 0, 0, 0, OpenFlags::NONE, json!({}));
        let (legs, none) = ((cproxy, uproxy), (Bytes::new(), Bytes::new()));
        let parser = parser(conn.sid());
        pump::relay(&rec, &mut conn, legs, none, parser, &quiet, None).await;
        conn.end(rec.now(), EndReason::Complete, None);
    })
    .await
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

fn conn(host: &str, reason: &'static str, listed: Option<(Source, Kind)>, cp: &[u8]) -> TunnelConn {
    let (host, fingerprint) = (host.to_owned(), None);
    TunnelConn {
        host,
        port: 443,
        reason,
        listed,
        fingerprint,
        client_prefix: Bytes::copy_from_slice(cp),
        upstream_prefix: Bytes::from_static(b"srv"),
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
            conn("example.org", "not_listed", None, b"hello"),
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
            let mut r = tunnel(&rec, &flags, conn("api.example", reason, listed, b"hi")).await;
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

#[test]
fn the_flag_fires_once_for_a_stream() {
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
            let mut r = tunnel(&rec, &flags, conn("up.example", "not_listed", None, b"")).await;
            pass(&mut r.client, &mut r.upstream, 1, 256 << 10).await;
            close(r).await;
        }
        let mut r = tunnel(&rec, &flags, conn("down.example", "not_listed", None, b"")).await;
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
        let mut r = tunnel(&rec, &flags, conn("a.example", "not_listed", None, b"")).await;
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

        let mut r = tunnel(&rec, &flags, conn("b.example", "not_listed", None, b"")).await;
        pass(&mut r.upstream, &mut r.client, 2, 3).await;
        drop(r.upstream);
        assert_eq!(r.client.read(&mut buf).await.unwrap(), 0);
        drop(r.client);
        r.done.await.unwrap();
    })
    .await;
}

/// Waits until the parser was fed `k` bytes in all.
async fn fed(copies: &Mutex<[Vec<u8>; 2]>, k: usize) {
    while copies.lock().unwrap().iter().map(Vec::len).sum::<usize>() < k {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

#[tokio::test]
async fn bytes_reach_the_peer_before_the_parser_and_sse_before_eof() {
    within_async(HANG, async {
        let (rec, _calls) = Recorder::for_tests(1 << 30);
        let parser = Copies::default();
        let (copies, peers, ok) = (parser.0.clone(), parser.1.clone(), parser.2.clone());
        let mut r = pumped(&rec, |_| Some(Box::new(parser)), Arc::default()).await;
        let dup =
            |s: &TcpStream| std::net::TcpStream::from(s.as_fd().try_clone_to_owned().unwrap());
        peers.set([dup(&r.client), dup(&r.upstream)]).unwrap();
        r.client.write_all(&[1; 100]).await.unwrap();
        fed(&copies, 100).await;
        r.upstream.write_all(b"data: 1\n\n").await.unwrap();
        fed(&copies, 109).await;
        let (mut sse, mut req) = ([0; 9], [0; 100]);
        r.client.read_exact(&mut sse).await.unwrap();
        r.upstream.read_exact(&mut req).await.unwrap();
        // The parser's copies hold the sockets open: end both sides through them.
        for peer in peers.get().unwrap() {
            peer.shutdown(std::net::Shutdown::Write).unwrap();
        }
        close(r).await;
        assert_eq!(ok.load(Ordering::SeqCst), 2);
    })
    .await;
}

/// How [`Faulty`] fails: a panic on every feed, `decode_failed` on a chunk of `b'!'`, or bytes
/// still held at the close.
#[derive(Clone, Copy, PartialEq)]
enum Fails {
    Panic,
    Bang,
    AtClose,
}

/// A fake parser that holds every byte it is fed unrecorded and fails as told.
struct Faulty(Fails, Arc<AtomicUsize>, u64);

impl Parser for Faulty {
    fn feed(&mut self, _: Dir, t: Mono, bytes: &[u8]) -> Result<(), Stop> {
        self.1.fetch_add(1, Ordering::SeqCst);
        self.2 = bytes.len() as u64;
        assert!(self.0 != Fails::Panic, "a parser bug");
        let reason = GapReason::DecodeFailed;
        let held = self.2;
        match bytes[0] == b'!' && self.0 == Fails::Bang {
            true => Err(Stop {
                reason,
                tail: None,
                held,
                since: t,
                at_close: false,
            }),
            false => Ok(()),
        }
    }
    fn end(self: Box<Self>, t: Mono, _: EndReason, _: bool, _: &[u8]) -> Option<Stop> {
        (self.0 == Fails::AtClose).then_some(Stop {
            reason: GapReason::DecodeFailed,
            tail: None,
            held: 5,
            since: t,
            at_close: true,
        })
    }
    fn panicked(self: Box<Self>, t: Mono) -> Stop {
        Stop::panic(self.2, t)
    }
}

/// 5a on the relay: the note, and one `conn` GAP whose `lost` is what the parser held plus every
/// byte relayed after the fault; the faulting chunk counts whole. Bytes held at the close get the
/// GAP alone.
#[tokio::test]
async fn a_parse_fault_keeps_relaying_and_counts_what_follows() {
    within_async(HANG, async {
        for fails in [Fails::Panic, Fails::Bang, Fails::AtClose] {
            let (rec, calls) = Recorder::for_tests(1 << 30);
            let fed = Arc::new(AtomicUsize::new(0));
            let parser = Box::new(Faulty(fails, fed.clone(), 0));
            let mut r = pumped(&rec, |_| Some(parser), Arc::default()).await;
            pass(&mut r.client, &mut r.upstream, b'a', 40).await;
            pass(&mut r.upstream, &mut r.client, b'b', 50).await;
            pass(&mut r.client, &mut r.upstream, b'!', 7).await;
            pass(&mut r.client, &mut r.upstream, b'c', 30).await;
            pass(&mut r.upstream, &mut r.client, b'd', 2000).await;
            close(r).await;
            let (want_fed, tail, lost) = match fails {
                Fails::Panic => (1, Some(json!({"panic": true})), 40 + 50 + 7 + 30 + 2000),
                Fails::Bang => (3, None, 7 + 30 + 2000),
                Fails::AtClose => (5, None, 5),
            };
            assert_eq!(fed.load(Ordering::SeqCst), want_fed);
            // Bytes only held at the close: parsing never stopped, so no note.
            let stopped = notes(&calls, "parse_stopped");
            assert_eq!(stopped.len(), usize::from(fails != Fails::AtClose));
            let all = gaps(&calls);
            let [Call::Gap(sid, GapReason::DecodeFailed, _, _, _, got, got_tail)] = &all[..] else {
                panic!("one conn GAP decode_failed");
            };
            assert_eq!((*got, got_tail), (Some(lost), &tail));
            for note in stopped {
                let conn = Some(*sid);
                assert_eq!(
                    (note["conn"].as_u64(), &note["reason"]),
                    (conn, &json!("decode_failed"))
                );
            }
        }
    })
    .await;
}

/// PRD-03 acceptance (`proxy_http1.rs` list, relay side): after a framing fault, a later request
/// on the connection sits inside the `conn` GAP, whose `lost` is every byte relayed after the
/// fault, the faulting chunk whole; a `parse_stopped` note exists.
#[tokio::test]
async fn a_later_request_sits_inside_the_conn_gap() {
    within_async(HANG, async {
        let (rec, calls) = Recorder::for_tests(1 << 30);
        let r2 = rec.clone();
        let mut r = pumped(
            &rec,
            move |conn| Some(Box::new(Http1::new(&r2, conn))),
            Arc::default(),
        )
        .await;
        let send = |b: &[u8]| b.to_vec();
        let get = send(b"GET /a HTTP/1.1\r\n\r\n");
        let ok = send(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
        let mut bad = send(b"BAD\r\n\r\nPOST /c HTTP/1.1\r\nContent-Length: 8000\r\n\r\n");
        bad.extend_from_slice(&[b'x'; 8000]);
        let answer = send(b"HTTP/1.1 201 Created\r\nContent-Length: 0\r\n\r\n");
        for (up, bytes) in [(true, &get), (false, &ok), (true, &bad), (false, &answer)] {
            let (from, to) = match up {
                true => (&mut r.client, &mut r.upstream),
                false => (&mut r.upstream, &mut r.client),
            };
            from.write_all(bytes).await.unwrap();
            let mut got = vec![0; bytes.len()];
            to.read_exact(&mut got).await.unwrap();
        }
        close(r).await;
        let reqs = calls
            .all()
            .into_iter()
            .filter(|c| matches!(c, Call::Open(_, StreamKind::HttpReq, ..)));
        assert_eq!(reqs.count(), 1);
        assert_eq!(notes(&calls, "parse_stopped").len(), 1);
        let all = gaps(&calls);
        let [Call::Gap(_, GapReason::DecodeFailed, _, _, _, lost, None)] = &all[..] else {
            panic!("one conn GAP decode_failed");
        };
        assert_eq!(*lost, Some((bad.len() + answer.len()) as u64));
    })
    .await;
}

/// One HTTP/2 frame: the 9-byte header, then the payload.
fn h2_frame(ty: u8, flags: u8, sid: u32, payload: &[u8]) -> Vec<u8> {
    let n = (payload.len() as u32).to_be_bytes();
    [&n[1..], &[ty, flags], &sid.to_be_bytes(), payload].concat()
}

/// A HEADERS frame with `END_HEADERS` (and `END_STREAM` when `end`) whose block holds literal
/// fields without indexing (RFC 7541 section 6.2.2).
fn h2_head(sid: u32, end: bool, fields: &[(&str, &str)]) -> Vec<u8> {
    let mut block = Vec::new();
    for (n, v) in fields {
        block.extend([0, n.len() as u8].iter().chain(n.as_bytes()));
        block.extend([v.len() as u8].iter().chain(v.as_bytes()));
    }
    h2_frame(1, 0x4 | u8::from(end), sid, &block)
}

/// PRD-03 `proxy_h2.rs` list, relay side: `PUSH_PROMISE` and an HPACK error stop parsing an h2
/// connection. The open streams get GAP `h2_unsupported`, a `parse_stopped` note is written, the
/// relay goes on byte for byte, and the `conn` GAP's `lost` is every byte relayed from the
/// faulting frame on.
#[tokio::test]
async fn an_h2_stop_names_the_rest_and_relays_it_whole() {
    let api = [(":scheme", "https"), (":authority", "api.example.com")];
    let req = |sid, method, end| {
        let f = [&[(":method", method)][..], &api[..], &[(":path", "/v1/m")][..]].concat();
        h2_head(sid, end, &f)
    };
    let settings = h2_frame(4, 0, 0, &[]);
    let start = [&b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n"[..], &settings].concat();
    let open = [start, req(1, "POST", false), h2_frame(0, 0, 1, b"part")].concat();
    let answer = [settings.clone(), h2_head(1, false, &[(":status", "200")])].concat();
    let promise = [0, 0, 0, 2, 0x82, 0x84];
    let push = (false, h2_frame(5, 0x4, 1, &promise));
    // Index 62 with an empty dynamic table.
    let bad_index = (true, h2_frame(1, 0x5, 3, &[0xbe]));
    let later = [h2_frame(0, 1, 1, b"rest"), req(5, "GET", true)].concat();
    for (fault_up, fault) in [push, bad_index] {
        within_async(HANG, async {
            let (rec, calls) = Recorder::for_tests(1 << 30);
            let r2 = rec.clone();
            let mut r = pumped(
                &rec,
                move |conn| Some(Box::new(H2::new(&r2, conn))),
                Arc::default(),
            )
            .await;
            let tail = b"server bytes after the fault".to_vec();
            let steps = [
                (true, &open),
                (false, &answer),
                (fault_up, &fault),
                (true, &later),
                (false, &tail),
            ];
            for (up, bytes) in steps {
                let (from, to) = match up {
                    true => (&mut r.client, &mut r.upstream),
                    false => (&mut r.upstream, &mut r.client),
                };
                from.write_all(bytes).await.unwrap();
                let mut got = vec![0; bytes.len()];
                to.read_exact(&mut got).await.unwrap();
                assert_eq!(&got, bytes, "relayed byte for byte");
            }
            close(r).await;
            let all = calls.all();
            let conn = all.iter().find_map(|c| match c {
                Call::Open(sid, StreamKind::Conn, ..) => Some(*sid),
                _ => None,
            });
            let streams: Vec<Sid> = all
                .iter()
                .filter_map(|c| match c {
                    Call::Open(sid, StreamKind::HttpReq | StreamKind::HttpResp, ..) => Some(*sid),
                    _ => None,
                })
                .collect();
            assert_eq!(streams.len(), 2, "stream 1's request and response, nothing later");
            let lost = (fault.len() + later.len() + tail.len()) as u64;
            let mut want: Vec<_> = streams
                .iter()
                .map(|s| (*s, GapReason::H2Unsupported, None))
                .collect();
            want.push((conn.unwrap(), GapReason::H2Unsupported, Some(lost)));
            let got: Vec<_> = gaps(&calls)
                .into_iter()
                .filter_map(|c| match c {
                    Call::Gap(sid, reason, _, _, _, lost, _) => Some((sid, reason, lost)),
                    _ => None,
                })
                .collect();
            assert_eq!(got, want);
            let stopped = notes(&calls, "parse_stopped");
            let [note] = &stopped[..] else {
                panic!("one parse_stopped note");
            };
            assert_eq!(
                (note["conn"].as_u64(), &note["reason"]),
                (conn, &json!("h2_unsupported"))
            );
        })
        .await;
    }
}

#[tokio::test]
async fn quiet_moves_with_relayed_bytes_not_with_an_idle_connection() {
    within_async(HANG, async {
        let (rec, _calls) = Recorder::for_tests(1 << 30);
        let quiet = Arc::new(AtomicU64::new(0));
        let mut r = pumped(&rec, |_| None, quiet.clone()).await;
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

/// A proxy-side read half whose clean EOF comes out as `UnexpectedEof` when `.1`: a TLS peer
/// gone without close_notify, as tokio-rustls reports it.
struct Cuts(ReadHalf<DuplexStream>, bool);

impl AsyncRead for Cuts {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let r = Pin::new(&mut self.0).poll_read(cx, buf);
        if self.1 && matches!(r, Poll::Ready(Ok(()))) && buf.filled().len() == before {
            return Poll::Ready(Err(std::io::ErrorKind::UnexpectedEof.into()));
        }
        r
    }
}

fn cuts(side: DuplexStream, cut: bool) -> Join<Cuts, WriteHalf<DuplexStream>> {
    let (r, w) = tokio::io::split(side);
    tokio::io::join(Cuts(r, cut), w)
}

/// Relays `up`, then `down`, through [`Http1`]; the upstream closes, then the client, and the
/// `cut` leg closes without close_notify. A client's cut reaches the harness as a close; an
/// upstream's does not, while the relay's caller still holds the client leg (rule 8), and the
/// connection ends `interrupted` since the harness leg is dropped unfinished. Each
/// stream's kind and END reason, by kind, and the GAPs.
async fn cut_close(up: &[u8], down: &[u8], cut: Dir) -> (Vec<(StreamKind, EndReason)>, Vec<Call>) {
    let (rec, calls) = Recorder::for_tests(1 << 30);
    let ((mut client, cp), (mut upstream, up_)) = (duplex(1 << 16), duplex(1 << 16));
    let legs = (cuts(cp, cut == Dir::Up), cuts(up_, cut == Dir::Down));
    let r = rec.clone();
    let relay = tokio::spawn(async move {
        let mut conn = r.open(StreamKind::Conn, 0, 0, 0, OpenFlags::NONE, json!({}));
        let parser: Box<dyn Parser> = Box::new(Http1::new(&r, conn.sid()));
        let (none, quiet) = ((Bytes::new(), Bytes::new()), AtomicU64::new(0));
        let p = pump::relay(&r, &mut conn, legs, none, Some(parser), &quiet, None).await;
        conn.end(r.now(), p.reason, None);
        assert!(!p.upstream_failed, "a cut is not reset (rule 8)");
        p.client
    });
    client.write_all(up).await.unwrap();
    let mut got = vec![0; up.len()];
    upstream.read_exact(&mut got).await.unwrap();
    upstream.write_all(down).await.unwrap();
    drop(upstream);
    let mut seen = vec![0; down.len()];
    client.read_exact(&mut seen).await.unwrap();
    assert_eq!((&got[..], &seen[..]), (up, down), "verbatim");
    let mut b = [0; 1];
    if cut == Dir::Down {
        let held = relay.await.unwrap();
        let wait = tokio::time::timeout(Duration::from_millis(50), client.read(&mut b));
        assert!(wait.await.is_err(), "no clean close for an upstream cut");
        drop(held);
    } else {
        assert_eq!(client.read(&mut b).await.unwrap(), 0, "the close passed on");
        drop(client);
        relay.await.unwrap();
    }
    let all = calls.all();
    let kind = |sid| {
        all.iter().find_map(|c| match c {
            Call::Open(s, k, ..) if *s == sid => Some(*k),
            _ => None,
        })
    };
    let ends = all.iter().filter_map(|c| match c {
        Call::End(sid, _, reason, ..) => Some((kind(*sid).unwrap(), *reason)),
        _ => None,
    });
    (by_kind(ends.collect()), gaps(&calls))
}

/// A client's cut is passed on as a close; an upstream's leaves the harness leg unfinished.
fn conn_end(cut: Dir) -> EndReason {
    match cut {
        Dir::Up => EndReason::Complete,
        Dir::Down => EndReason::Interrupted,
    }
}

fn by_kind(mut ends: Vec<(StreamKind, EndReason)>) -> Vec<(StreamKind, EndReason)> {
    ends.sort_by_key(|e| e.0.to_u8());
    ends
}

#[tokio::test]
async fn only_an_upstream_cut_leaves_a_close_delimited_reply_unproven() {
    within_async(HANG, async {
        use EndReason::{Complete, Interrupted};
        use StreamKind::{Conn, HttpReq, HttpResp, WsDown, WsUp};
        let get = b"GET /events HTTP/1.1\r\nHost: a.example\r\n\r\n";
        let sse = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\ndata: 1\n\n";
        for (cut, resp) in [(Dir::Up, Complete), (Dir::Down, Interrupted)] {
            let (ends, gaps) = cut_close(get, sse, cut).await;
            let want = vec![(HttpReq, Complete), (HttpResp, resp), (Conn, conn_end(cut))];
            assert_eq!(ends, by_kind(want));
            assert!(gaps.is_empty());
        }
        // A whole WebSocket session, a Close each way: whole whichever leg is cut.
        let mut up =
            b"GET /ws HTTP/1.1\r\nHost: a.example\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
            Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
                .to_vec();
        // Text "hi", then Close 1000, masked with a zero key.
        up.extend(b"\x81\x82\0\0\0\0hi\x88\x82\0\0\0\0\x03\xe8");
        let mut down =
            b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
            Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n\r\n"
                .to_vec();
        down.extend(b"\x81\x02yo\x88\x02\x03\xe8");
        for cut in [Dir::Up, Dir::Down] {
            let (ends, gaps) = cut_close(&up, &down, cut).await;
            let mut whole = [HttpReq, WsUp, WsDown].map(|k| (k, Complete)).to_vec();
            whole.push((Conn, conn_end(cut)));
            assert_eq!(ends, by_kind(whole));
            assert!(gaps.is_empty());
        }
    })
    .await;
}

/// A parser that keeps a copy of every byte fed (up, down) and, once the peers (client, upstream
/// test sides) are set, counts the feeds that found their chunk already waiting at the peer (a
/// non-blocking peek). The last field keeps the bytes `end` gets as never forwarded.
#[derive(Default)]
struct Copies(
    Arc<Mutex<[Vec<u8>; 2]>>,
    Arc<OnceLock<[std::net::TcpStream; 2]>>,
    Arc<AtomicUsize>,
    Arc<Mutex<Vec<u8>>>,
);

impl Parser for Copies {
    fn feed(&mut self, dir: Dir, _: Mono, bytes: &[u8]) -> Result<(), Stop> {
        if let Some(peers) = self.1.get() {
            let mut b = vec![0; bytes.len()];
            let peer = &peers[usize::from(dir == Dir::Up)];
            if peer.peek(&mut b).is_ok_and(|n| b[..n] == *bytes) {
                self.2.fetch_add(1, Ordering::SeqCst);
            }
        }
        self.0.lock().unwrap()[usize::from(dir == Dir::Down)].extend_from_slice(bytes);
        Ok(())
    }
    fn end(self: Box<Self>, _: Mono, _: EndReason, _: bool, unsent: &[u8]) -> Option<Stop> {
        self.3.lock().unwrap().extend_from_slice(unsent);
        None
    }
    fn panicked(self: Box<Self>, t: Mono) -> Stop {
        Stop::panic(0, t)
    }
}

/// Point 1: one leg's error ends the relay while the other leg is mid-write. The part of the
/// chunk its writer took reached the peer, so it is counted and parsed; the rest of a client
/// chunk goes to the parser's `end` as never forwarded, the rest of a server chunk is parsed
/// too (model output we hold), and no GAP names a loss.
#[tokio::test]
async fn a_chunk_cut_mid_write_by_the_other_legs_error_is_recorded_as_far_as_it_went() {
    within_async(HANG, async {
        let (rec, calls) = Recorder::for_tests(1 << 30);
        // The client side takes 1 KiB unread, so an 8 KiB reply stops mid-write.
        let ((mut client, cp), (mut upstream, up)) = (duplex(1024), duplex(1 << 16));
        let parser = Copies::default();
        let (copies, unsent) = (parser.0.clone(), parser.3.clone());
        let r = rec.clone();
        let relay = tokio::spawn(async move {
            let mut conn = r.open(StreamKind::Conn, 0, 0, 0, OpenFlags::NONE, json!({}));
            let (none, quiet) = ((Bytes::new(), Bytes::new()), AtomicU64::new(0));
            let parser: Box<dyn Parser> = Box::new(parser);
            let p = pump::relay(&r, &mut conn, (cp, up), none, Some(parser), &quiet, None).await;
            conn.end(r.now(), p.reason, None);
            (p.plain_up, p.plain_down, p.reason, p.unwritten)
        });
        upstream.write_all(&[7; 8192]).await.unwrap();
        let mut first = [0; 1];
        client.read_exact(&mut first).await.unwrap();
        // The upstream goes away, and the client's next byte cannot reach it.
        drop(upstream);
        client.write_all(b"x").await.unwrap();
        let (plain_up, plain_down, reason, unwritten) = relay.await.unwrap();
        let mut rest = Vec::new();
        client.read_to_end(&mut rest).await.unwrap();
        let got = 1 + rest.len();
        assert!(got < 8192, "the reply was cut mid-write");
        assert_eq!((plain_up, plain_down), (0, got as u64));
        // Both rests, for a tunnel's GAP: the client's byte and the reply's.
        assert_eq!(unwritten, [1, 8192 - got as u64]);
        assert_eq!(*unsent.lock().unwrap(), b"x");
        let copies = copies.lock().unwrap();
        assert_eq!((copies[0].len(), &copies[1]), (0, &vec![7; 8192]));
        assert_eq!(reason, EndReason::Interrupted);
        assert!(gaps(&calls).is_empty());
    })
    .await;
}

/// An upstream read half that waits until told, then fails with `.1`: a reset, or
/// `UnexpectedEof` for a TLS peer gone without close_notify.
struct ResetOn(oneshot::Receiver<()>, std::io::ErrorKind);

impl AsyncRead for ResetOn {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        _: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let kind = self.1;
        Pin::new(&mut self.0).poll(cx).map(|_| Err(kind.into()))
    }
}

/// An upstream write half that takes this many more bytes, then stays pending, as a TCP peer
/// that stopped reading does.
struct Stalls(Arc<AtomicUsize>);

impl AsyncWrite for Stalls {
    fn poll_write(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let n = buf.len().min(self.0.load(Ordering::SeqCst));
        self.0.fetch_sub(n, Ordering::SeqCst);
        if n == 0 {
            Poll::Pending
        } else {
            Poll::Ready(Ok(n))
        }
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

/// What [`reset_mid_upload`] saw: plaintext bytes forwarded up, each http_req's END reason and
/// tail `wire` and `unsent`, and every GAP's `lost`.
#[derive(Debug, PartialEq)]
struct Upload {
    plain_up: u64,
    reqs: Vec<(EndReason, u64, Option<u64>)>,
    lost: Vec<Option<u64>>,
}

/// PRD-03 section 5: the upstream read fails with `kind` (a reset by default) while the
/// client's chunk is held mid-write by backpressure after `took` bytes.
async fn reset_mid_upload(chunk: &[u8], took: usize) -> Upload {
    cut_mid_upload(chunk, took, std::io::ErrorKind::ConnectionReset).await
}

async fn cut_mid_upload(chunk: &[u8], took: usize, kind: std::io::ErrorKind) -> Upload {
    let (rec, calls) = Recorder::for_tests(1 << 30);
    let (reset, left) = (oneshot::channel(), Arc::new(AtomicUsize::new(took)));
    let upstream = tokio::io::join(ResetOn(reset.1, kind), Stalls(left.clone()));
    let (mut client, cp) = duplex(1 << 16);
    let r = rec.clone();
    let relay = tokio::spawn(async move {
        let mut conn = r.open(StreamKind::Conn, 0, 0, 0, OpenFlags::NONE, json!({}));
        let parser: Box<dyn Parser> = Box::new(Http1::new(&r, conn.sid()));
        let (none, quiet) = ((Bytes::new(), Bytes::new()), AtomicU64::new(0));
        let legs = (cp, upstream);
        let p = pump::relay(&r, &mut conn, legs, none, Some(parser), &quiet, None).await;
        conn.end(r.now(), p.reason, None);
        assert_eq!(p.reason, EndReason::Interrupted);
        p.plain_up
    });
    client.write_all(chunk).await.unwrap();
    while left.load(Ordering::SeqCst) > 0 {
        tokio::task::yield_now().await;
    }
    reset.0.send(()).unwrap();
    let plain_up = relay.await.unwrap();
    let all = calls.all();
    let reqs: Vec<Sid> = all
        .iter()
        .filter_map(|c| match c {
            Call::Open(sid, StreamKind::HttpReq, ..) => Some(*sid),
            _ => None,
        })
        .collect();
    let reqs = reqs.iter().filter_map(|req| {
        all.iter().find_map(|c| match c {
            Call::End(sid, _, reason, _, tail) if sid == req => {
                let t = tail.as_ref().unwrap();
                Some((*reason, t["wire"].as_u64().unwrap(), t["unsent"].as_u64()))
            }
            _ => None,
        })
    });
    let lost = gaps(&calls).into_iter().filter_map(|g| match g {
        Call::Gap(_, _, _, _, _, lost, _) => Some(lost),
        _ => None,
    });
    Upload {
        plain_up,
        reqs: reqs.collect(),
        lost: lost.collect(),
    }
}

/// The client bytes read and never forwarded are recorded, and each http_req counts its own
/// among them, head included, as END tail `unsent`: the rest of a body, a head never forwarded,
/// a pipelined request. None is left out, so no conn GAP.
#[tokio::test]
async fn an_upstream_reset_mid_upload_counts_the_rest_of_the_chunk_unsent() {
    use EndReason::Complete;
    within_async(HANG, async {
        let mut post =
            b"POST /up HTTP/1.1\r\nHost: a.example\r\nContent-Length: 5000\r\n\r\n".to_vec();
        let head = post.len();
        post.extend([b'u'; 5000]);
        let all = post.len() as u64;
        let sent = head as u64 + 1000;
        let up = reset_mid_upload(&post, head + 1000).await;
        let want = |plain_up, reqs| Upload {
            plain_up,
            reqs,
            lost: vec![],
        };
        assert_eq!(up, want(sent, vec![(Complete, 5000, Some(4000))]));
        // A head cut short: the request opens from its never forwarded rest.
        let up = reset_mid_upload(&post, 20).await;
        assert_eq!(up, want(20, vec![(Complete, 5000, Some(all - 20))]));
        // A pipelined request behind the body is its own.
        let next = b"GET /next HTTP/1.1\r\nHost: a.example\r\n\r\n";
        post.extend(next);
        let up = reset_mid_upload(&post, head + 1000).await;
        let reqs = vec![
            (Complete, 5000, Some(4000)),
            (Complete, 0, Some(next.len() as u64)),
        ];
        assert_eq!(up, want(sent, reqs));
    })
    .await;
}

/// F6: a Content-Length body the client sent only in part, then the upstream cut (no
/// close_notify) or reset before the rest. The request is not whole, so it ends `interrupted`;
/// the part read and never forwarded is its `unsent`, and nothing is a loss, so no conn GAP.
#[tokio::test]
async fn an_upstream_cut_mid_upload_ends_the_request_interrupted_with_its_unsent() {
    use EndReason::Interrupted;
    use std::io::ErrorKind::{ConnectionReset, UnexpectedEof};
    within_async(HANG, async {
        let mut post =
            b"POST /up HTTP/1.1\r\nHost: a.example\r\nContent-Length: 5000\r\n\r\n".to_vec();
        let head = post.len();
        // 2000 of the 5000 body bytes; the upstream takes the head and 1000 of them.
        post.extend([b'u'; 2000]);
        for kind in [UnexpectedEof, ConnectionReset] {
            let up = cut_mid_upload(&post, head + 1000, kind).await;
            let want = Upload {
                plain_up: head as u64 + 1000,
                reqs: vec![(Interrupted, 2000, Some(1000))],
                lost: vec![],
            };
            assert_eq!(up, want, "{kind:?}");
        }
    })
    .await;
}
