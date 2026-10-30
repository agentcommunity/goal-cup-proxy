//! PRD-06 "Automated tests": the backoff waits, the 120 s cap, `Retry-After`, the reset on
//! progress (run 1's lag bug) and the clock case. `policy.rs` takes `now` as an argument, so the
//! clock case needs no test hook. The every-other-PUT case drives the real client against a
//! small fake Worker of its own, as the sender does: progress on a stored chunk, a failure on a
//! refused one.

use goalcup::pipeline::Mono;
use goalcup::upload::client::{ChunkHeaders, ServerClient, Token};
use goalcup::upload::policy::{Backoff, Cadence, Policy, Timer, after, backoff, jitter};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;

const S: fn(u64) -> Duration = Duration::from_secs;

#[test]
fn waits_double_from_2_s_and_never_pass_120_s() {
    let p = Policy::default();
    let waits: Vec<u64> = (1..=8).map(|n| backoff(&p, n, None, 1.0).as_secs()).collect();
    assert_eq!(waits, [2, 4, 8, 16, 32, 64, 120, 120]);

    for step in 0..=10 {
        let j = 0.5 + f64::from(step) * 0.05;
        for n in 1..=40 {
            let nominal = backoff(&p, n, None, 1.0);
            let wait = backoff(&p, n, None, j);
            assert!(wait <= S(120), "failure {n}, jitter {j}: {wait:?}");
            assert!(wait >= nominal / 2, "failure {n}, jitter {j}: {wait:?}");
        }
    }
    for _ in 0..1000 {
        let j = jitter();
        assert!((0.5..=1.0).contains(&j), "{j}");
    }
    let mut b = Backoff::new(&p);
    for _ in 0..40 {
        assert!(b.fail(Mono(0), None) <= S(120));
    }
}

#[test]
fn retry_after_is_honoured_up_to_the_cap() {
    let p = Policy::default();
    for j in [0.5, 0.75, 1.0] {
        assert_eq!(backoff(&p, 1, Some(S(900)), j), S(120));
        assert_eq!(backoff(&p, 1, Some(S(10)), j), S(10));
    }
}

#[test]
fn progress_resets_the_wait() {
    let p = Policy::default();
    let mut b = Backoff::new(&p);
    for _ in 0..10 {
        b.fail(Mono(0), None);
    }
    assert!(b.fail(Mono(0), None) >= S(60));
    b.progress();
    assert!(b.due(Mono(0)));
    let wait = b.fail(Mono(0), None);
    assert!(wait <= S(2), "{wait:?}");
    assert_eq!(b.failures, 1);
}

#[test]
fn a_clock_jump_makes_the_round_and_the_beat_due_on_the_next_tick() {
    for spread_first in [false, true] {
        let p = Policy { spread_first, ..Policy::default() };
        let t0 = Mono(1_000_000_000_000);
        let mut round = Cadence::new(&p);
        let mut beat = Timer::beat(&p);
        assert!(round.due(t0, 1));
        assert!(beat.due(t0));
        round.started(t0);
        beat.started(t0);
        // The second round and beat, a full interval after.
        let t1 = after(t0, S(300));
        assert!(round.due(t1, 1));
        assert!(beat.due(t1));
        round.started(t1);
        beat.started(t1);
        assert!(!round.due(after(t1, S(1)), 1));
        assert!(!beat.due(after(t1, S(1))));
        // A size trigger is due at once.
        assert!(round.due(after(t1, S(1)), p.trigger_bytes));
        // `now` moves 10 minutes ahead in one step (a laptop wakes).
        let woke = after(t1, S(600));
        assert!(round.due(woke, 1));
        assert!(beat.due(woke));
        let mut b = Backoff::new(&p);
        b.fail(t1, Some(S(120)));
        assert!(b.due(woke));
    }
}

/// A fake Worker that answers every other PUT with `503` (no `Retry-After`) and the rest with a
/// stored chunk. It reads each body whole, as the real Worker does once its checks pass.
fn flaky_worker(puts: usize) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for n in 0..puts {
            let Ok((mut stream, _)) = listener.accept() else { return };
            let mut seen = Vec::new();
            let mut buf = [0u8; 4096];
            let end = loop {
                if let Some(i) = seen.windows(4).position(|w| w == b"\r\n\r\n") {
                    break i + 4;
                }
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(k) => seen.extend_from_slice(&buf[..k]),
                }
            };
            let head = String::from_utf8_lossy(&seen[..end]).to_ascii_lowercase();
            let length: usize = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0);
            while seen.len() < end + length {
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(k) => seen.extend_from_slice(&buf[..k]),
                }
            }
            let (status, body) = if n % 2 == 0 {
                ("503 Service Unavailable", r#"{"error":"maintenance"}"#.to_string())
            } else {
                let at = (n as u64).div_ceil(2) * 10;
                let head = "a".repeat(64);
                ("200 OK", format!(r#"{{"offset":{at},"next_seq":{n},"head":"{head}"}}"#))
            };
            let answer = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(answer.as_bytes());
        }
    });
    port
}

#[test]
fn a_worker_failing_every_other_put_never_makes_a_wait_above_4_s() {
    let puts = 20;
    let port = flaky_worker(puts);
    let token = Token::new(format!("gcr_{}", "0".repeat(64)));
    let client = ServerClient::new(&format!("http://127.0.0.1:{port}"), Some(token));
    let p = Policy::default();
    let mut b = Backoff::new(&p);
    let (mut stored, mut failed) = (0, 0);
    for n in 0..puts {
        let h = ChunkHeaders {
            entry_id: "e0123456789abcdef".into(),
            offset: stored * 10,
            raw_len: 10,
            first_seq: stored,
            last_seq: stored,
            head: "a".repeat(64),
            prev: "0".repeat(64),
            sha256: "c".repeat(64),
        };
        let body = [0u8; 8];
        match client.put_chunk("r0123456789abcdef", &h, &body[..], body.len() as u64) {
            Ok(answer) => {
                assert!(answer.offset > h.offset, "put {n}: {answer:?}");
                b.progress();
                stored += 1;
            }
            Err(e) => {
                let wait = b.fail(Mono(0), e.retry_after().map(Duration::from_secs));
                assert!(wait <= Duration::from_secs(4), "put {n}: {e}, wait {wait:?}");
                failed += 1;
            }
        }
    }
    assert_eq!((stored, failed), (10, 10));
}
