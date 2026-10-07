//! `goalcup auth` — the device grant, and the `0600` rule.
//!
//! The flow prints what the participant types into their own browser and never the grant;
//! the grant lands at `0600` and reloads; a server that never authorizes is a refusal with
//! the URL repeated and nothing stored.
//!
//! The transport half decides what crosses the network: which URLs this command will talk to
//! at all, that TLS is actually linked, that a redirect does not carry the device code away,
//! and that a server which omits `interval` does not get asked back-to-back.
#![cfg(feature = "v1-compat")]

mod support;

use goalcup_cli::commands::auth::{AuthOptions, load_grant, logout, poll_interval, run};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// A grant server that answers exactly two routes, then stops: `POST /device` returns a
/// code, `POST /token` returns `authorization_pending` `pending_rounds` times and then a
/// grant. Enough to prove the flow and the storage.
fn mock_grant_server(pending_rounds: usize) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let mut left = pending_rounds;
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let body = if line.contains("/device") {
                r#"{"device_code":"dc","user_code":"WXYZ-1234","verification_uri":"https://example.invalid/device","interval":0}"#.to_string()
            } else if left > 0 {
                left -= 1;
                r#"{"error":"authorization_pending"}"#.to_string()
            } else {
                r#"{"grant_token":"g-abc","subject":"u1","email_verified":true,"scopes":["entry:create"],"expires_at":"2026-12-31T00:00:00Z"}"#.to_string()
            };
            let mut headers = String::new();
            loop {
                let mut h = String::new();
                reader.read_line(&mut h).unwrap();
                if h.trim().is_empty() {
                    break;
                }
                headers.push_str(&h);
            }
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(resp.as_bytes()).unwrap();
        }
    });
    format!("http://{addr}")
}

/// The same two routes, except `POST /token` answers `token` every time — a server that
/// has made up its mind, which is the half [`mock_grant_server`] cannot express.
fn mock_answering(token: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let body = if line.contains("/device") {
                r#"{"device_code":"dc","user_code":"WXYZ-1234","verification_uri":"https://example.invalid/device","interval":0}"#
            } else {
                token
            };
            loop {
                let mut h = String::new();
                reader.read_line(&mut h).unwrap();
                if h.trim().is_empty() {
                    break;
                }
            }
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(resp.as_bytes()).unwrap();
        }
    });
    format!("http://{addr}")
}

fn opts(server: String) -> AuthOptions {
    AuthOptions {
        server,
        open_browser: false,
        poll_limit: 8,
        // The shipped ten minutes. A test that wants the deadline itself sets a tiny one at
        // the call site that is about it.
        poll_deadline: goalcup_cli::commands::auth::POLL_DEADLINE,
    }
}

#[test]
fn the_participant_reads_the_code_in_their_own_browser_and_the_grant_lands_at_0600() {
    let tmp = tempfile::tempdir().unwrap();
    let server = mock_grant_server(2);
    let mut out = Vec::new();
    let grant = run(tmp.path(), &opts(server.clone()), &mut out).unwrap();
    assert_eq!(grant.grant_token, "g-abc");
    assert!(grant.email_verified);
    let printed = String::from_utf8(out).unwrap();
    assert!(
        printed.contains("WXYZ-1234"),
        "the user code is shown, never typed by the CLI"
    );
    assert!(printed.contains("https://example.invalid/device"));
    assert!(
        !printed.contains("g-abc"),
        "the grant itself is never printed"
    );

    let path = goalcup_cli::config::grant_path(tmp.path());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let reloaded = load_grant(tmp.path()).unwrap().unwrap();
    assert_eq!(reloaded.grant_token, grant.grant_token);
    assert!(logout(tmp.path()).unwrap());
    assert!(load_grant(tmp.path()).unwrap().is_none());
}

#[test]
fn a_server_that_never_authorizes_ends_as_a_refusal_with_the_url_repeated() {
    let tmp = tempfile::tempdir().unwrap();
    let server = mock_grant_server(usize::MAX);
    let mut out = Vec::new();
    let err = run(tmp.path(), &opts(server), &mut out).unwrap_err();
    assert_eq!(
        err.exit_code(),
        1,
        "a participant can fix this by finishing in the browser"
    );
    assert!(
        format!("{err}").contains("example.invalid"),
        "say where to go, again: {err}"
    );
    assert!(
        load_grant(tmp.path()).unwrap().is_none(),
        "a failed auth stores nothing"
    );
}

#[test]
fn no_grant_is_stored_outside_the_config_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let server = mock_grant_server(0);
    run(tmp.path(), &opts(server), &mut Vec::new()).unwrap();
    let written: Vec<_> = std::fs::read_dir(tmp.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(written, vec!["grant.json".to_string()]);
}

/// The tests above read `run`'s writer. This one reads the **binary's** two
/// streams, which is what a participant and their terminal scrollback actually see.
#[test]
fn the_shipped_binary_prints_the_code_and_puts_the_grant_on_neither_stream() {
    let cfg = support::empty_cfg();
    let server = mock_grant_server(1);
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_goalcup"))
        .args(["auth", "--no-open"])
        .env_clear()
        .envs(support::clean_env())
        .env("GOALCUP_CONFIG_DIR", &cfg)
        .env("GOALCUP_SERVER", &server)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        out.status.success(),
        "`goalcup auth` against a server that authorizes: {:?}, {stderr}",
        out.status.code()
    );
    assert!(
        stdout.contains("WXYZ-1234"),
        "the code is shown: {stdout:?}"
    );
    assert!(stdout.contains("https://example.invalid/device"));
    assert!(!stdout.contains("g-abc"), "not on stdout: {stdout:?}");
    assert!(!stderr.contains("g-abc"), "not on stderr: {stderr:?}");
    assert!(
        stderr.is_empty(),
        "a run that worked says nothing: {stderr:?}"
    );

    let grant = load_grant(&cfg).unwrap().unwrap();
    assert_eq!(grant.grant_token, "g-abc");
    assert_eq!(grant.scopes, vec!["entry:create".to_string()]);
    assert!(!grant.stored_at.is_empty(), "the file dates itself");
    assert!(
        !format!("{grant:?}").contains("g-abc"),
        "a `{{:?}}` of the grant hands over the token: {grant:?}"
    );
}

/// A server that says no is rung 1 — the participant starts again — and it says no in the
/// `ErrorBody` shape every route in this tree answers with.
#[test]
fn a_server_that_refuses_the_code_outright_is_rung_one_and_stores_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let server = mock_answering(r#"{"error":{"code":"expired_token","message":"it expired"}}"#);
    let err = run(tmp.path(), &opts(server), &mut Vec::new()).unwrap_err();
    assert_eq!(err.exit_code(), 1, "start again: {err}");
    let said = format!("{err}");
    assert!(
        said.contains("expired_token") && said.contains("it expired"),
        "{said}"
    );
    assert!(
        said.contains("example.invalid"),
        "say where to go, again: {said}"
    );
    assert!(load_grant(tmp.path()).unwrap().is_none());
}

/// A body that is neither a grant nor an error is the CLI failing to do its job — rung 3,
/// never rung 1, because there is nothing a participant does differently next time.
#[test]
fn a_grant_the_cli_cannot_read_is_rung_three_and_stores_nothing() {
    for body in [r#"{"grant_token":"g-abc"}"#, r#"{"whatever":true}"#] {
        let tmp = tempfile::tempdir().unwrap();
        let err = run(tmp.path(), &opts(mock_answering(body)), &mut Vec::new()).unwrap_err();
        assert_eq!(
            err.exit_code(),
            3,
            "{body} left under the wrong rung: {err}"
        );
        assert!(
            !format!("{err}").contains("g-abc"),
            "a half-formed grant is not quoted back: {err}"
        );
        assert!(load_grant(tmp.path()).unwrap().is_none());
    }
}

// ------------------------------------------------------------------ the transport

/// A server that counts what reaches it and answers the flow straight through.
///
/// The counter is the point. "This URL was refused before any request was made" is not a
/// claim an error message can carry on its own — a refusal after a failed connection reads
/// much the same — so the tests below assert against a real listener that never saw a
/// byte.
fn counting_server() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&seen);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            counter.fetch_add(1, Ordering::SeqCst);
            let mut stream = stream.unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let body = if line.contains("/device") {
                r#"{"device_code":"dc","user_code":"WXYZ-1234","verification_uri":"https://example.invalid/device","interval":0}"#
            } else {
                r#"{"grant_token":"g-abc","subject":"u1","email_verified":true,"scopes":[],"expires_at":"2026-12-31T00:00:00Z"}"#
            };
            loop {
                let mut h = String::new();
                reader.read_line(&mut h).unwrap();
                if h.trim().is_empty() {
                    break;
                }
            }
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(resp.as_bytes()).unwrap();
        }
    });
    (format!("http://{addr}"), seen)
}

/// A server whose very first answer is a `302` to somewhere else.
fn mock_redirecting() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            loop {
                let mut h = String::new();
                reader.read_line(&mut h).unwrap();
                if h.trim().is_empty() {
                    break;
                }
            }
            let resp = "HTTP/1.1 302 Found\r\nLocation: http://elsewhere.invalid/v0/device\r\nContent-Length: 0\r\n\r\n";
            stream.write_all(resp.as_bytes()).unwrap();
        }
    });
    format!("http://{addr}")
}

/// A loopback port nothing is listening on: bound to learn the number, then dropped.
fn closed_loopback_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

/// A plain `http://` URL to anything but this machine would put the device code and the
/// grant on the wire in the clear. It is refused **before the first request**, and the
/// message says which scheme is the problem.
#[test]
fn a_non_loopback_http_url_is_refused_naming_the_scheme_before_any_request() {
    let tmp = tempfile::tempdir().unwrap();
    let (_url, seen) = counting_server();
    let err = run(
        tmp.path(),
        &opts("http://example.invalid".to_string()),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert_eq!(err.exit_code(), 1, "a different --server fixes it: {err}");
    let said = format!("{err}");
    assert!(said.contains("http://"), "name the scheme: {said}");
    assert!(
        !said.contains("did not answer"),
        "this is a refusal, not a connection that failed: {said}"
    );
    assert_eq!(
        seen.load(Ordering::SeqCst),
        0,
        "nothing was sent anywhere: {said}"
    );
    assert!(load_grant(tmp.path()).unwrap().is_none());
}

/// Loopback `http://` is the development server and every mock in this file, and it stays
/// allowed — the refusal above is about leaving the machine, not about the scheme alone.
#[test]
fn loopback_http_is_still_allowed() {
    let tmp = tempfile::tempdir().unwrap();
    let (url, seen) = counting_server();
    let grant = run(tmp.path(), &opts(url), &mut Vec::new()).unwrap();
    assert_eq!(grant.grant_token, "g-abc");
    assert!(seen.load(Ordering::SeqCst) >= 2, "both routes were reached");
}

/// The TLS backend is linked. Without ureq's `tls` feature an `https://` URL never reaches
/// a socket at all: ureq answers "unknown scheme", which this command would have reported
/// as a server that did not answer. A closed loopback port must therefore fail with a
/// **connection** error.
#[test]
fn an_https_url_reaches_the_transport_rather_than_an_unknown_scheme() {
    let tmp = tempfile::tempdir().unwrap();
    let port = closed_loopback_port();
    let err = run(
        tmp.path(),
        &opts(format!("https://127.0.0.1:{port}")),
        &mut Vec::new(),
    )
    .unwrap_err();
    let said = format!("{err}").to_ascii_lowercase();
    assert!(
        !said.contains("unknown scheme"),
        "ureq's `tls` feature is not enabled, so this build cannot speak https at all: \
         {said}"
    );
    assert!(
        said.contains("did not answer"),
        "a closed port is a connection that failed: {said}"
    );
}

/// A scheme this command does not speak is named rather than handed to the transport.
#[test]
fn a_scheme_this_command_does_not_speak_is_refused_naming_it() {
    let tmp = tempfile::tempdir().unwrap();
    for server in ["ftp://example.invalid", "example.invalid"] {
        let err = run(tmp.path(), &opts(server.to_string()), &mut Vec::new()).unwrap_err();
        assert_eq!(err.exit_code(), 1, "{server}: {err}");
        assert!(load_grant(tmp.path()).unwrap().is_none());
    }
}

/// The device code rides in a request header, and a followed redirect would have handed it
/// to whatever host the `Location` named. A `3xx` is a refusal naming the server instead.
#[test]
fn a_redirect_is_refused_and_the_device_code_is_not_re_sent() {
    let tmp = tempfile::tempdir().unwrap();
    let server = mock_redirecting();
    let err = run(tmp.path(), &opts(server.clone()), &mut Vec::new()).unwrap_err();
    assert_eq!(err.exit_code(), 1, "point --server somewhere else: {err}");
    let said = format!("{err}");
    assert!(said.contains(&server), "name the server: {said}");
    assert!(said.contains("302"), "name what it answered: {said}");
    assert!(
        !said.contains("elsewhere.invalid"),
        "the host it wanted to send the code to is not where the participant is pointed: \
         {said}"
    );
    assert!(load_grant(tmp.path()).unwrap().is_none());
}

/// `--poll-limit 0` asks this command to never ask, and then to report that the code was
/// never entered. It is a malformed flag, refused before any request.
#[test]
fn a_poll_limit_of_zero_is_refused_before_any_request() {
    let tmp = tempfile::tempdir().unwrap();
    let (url, seen) = counting_server();
    let mut opts = opts(url);
    opts.poll_limit = 0;
    let err = run(tmp.path(), &opts, &mut Vec::new()).unwrap_err();
    assert_eq!(
        err.exit_code(),
        1,
        "a different --poll-limit fixes it: {err}"
    );
    assert!(format!("{err}").contains("--poll-limit"), "{err}");
    assert_eq!(seen.load(Ordering::SeqCst), 0, "nothing was asked");
}

/// The interval the loop actually waits, case by case.
///
/// An absent `interval` is five seconds and not `0`: a `0` would send `poll_limit` requests
/// back-to-back and the ten-minute window `DEFAULT_POLL_LIMIT` is counted for would not exist.
/// A `0` is honoured only from a server on this machine — which is what keeps the mocks above
/// instant without letting anything across the network set the pace.
#[test]
fn an_absent_interval_is_five_seconds_and_a_zero_is_loopback_only() {
    let five = Duration::from_secs(5);
    for loopback in [true, false] {
        assert_eq!(
            poll_interval(None, loopback),
            five,
            "an absent interval is the default, not zero (loopback={loopback})"
        );
    }
    assert_eq!(
        poll_interval(Some(0), false),
        five,
        "a zero from across the network is treated as unsaid"
    );
    assert_eq!(
        poll_interval(Some(0), true),
        Duration::ZERO,
        "a zero from this machine is the mock and the dev server"
    );
    for loopback in [true, false] {
        assert_eq!(poll_interval(Some(7), loopback), Duration::from_secs(7));
        assert_eq!(
            poll_interval(Some(3600), loopback),
            Duration::from_secs(30),
            "a server asking for an hour between polls is capped"
        );
    }
}

/// A device-flow server that names `uri` as the page to visit, and answers
/// `authorization_pending` — with `slow_down` where asked — for ever.
///
/// Two things at once because they are the same server: the verification URI is the string
/// this command hands a browser, and the poll is what a `slow_down` stretches.
fn mock_pending_forever(uri: &'static str, slow_down: bool) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = match stream {
                Ok(stream) => stream,
                Err(_) => return,
            };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let body = if line.contains("/device") {
                format!(
                    "{{\"device_code\":\"dc\",\"user_code\":\"WXYZ-1234\",\
                     \"verification_uri\":\"{uri}\",\"interval\":0}}"
                )
            } else if slow_down {
                r#"{"error":"slow_down"}"#.to_string()
            } else {
                r#"{"error":"authorization_pending"}"#.to_string()
            };
            loop {
                let mut h = String::new();
                if reader.read_line(&mut h).is_err() || h.trim().is_empty() {
                    break;
                }
            }
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(resp.as_bytes());
        }
    });
    format!("http://{addr}")
}

/// The `verification_uri` came off the network, and it is the string this command hands to a
/// browser. It is checked before it is printed, let alone opened.
///
/// `check_server_url` vets the SERVER before the socket; this checks the URI the response
/// body asks the browser to visit, because a typo'd or hostile server needs one answer to
/// invoke a local scheme handler. `https://` to somewhere that is not this machine is the
/// real case; the loopback `http://` shape is the development server's, which is what every
/// other test in this file runs against.
#[test]
fn a_verification_uri_this_command_will_not_open_is_refused_before_it_is_printed() {
    for bad in [
        "file:///etc/passwd",
        "vscode://install?extension=evil",
        "http://evil.example/device",
        // Loopback without a port is not the dev shape either: a dev server listens on one.
        "http://127.0.0.1/device",
        "https://127.0.0.1:8443/device",
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let server = mock_pending_forever(Box::leak(bad.to_string().into_boxed_str()), false);
        let mut out = Vec::new();
        let err = run(tmp.path(), &opts(server), &mut out).expect_err("{bad} must be refused");
        assert_eq!(err.exit_code(), 1, "{bad}");
        assert!(
            format!("{err}").contains(bad),
            "the refusal names it: {err}"
        );
        assert!(
            !String::from_utf8(out).unwrap().contains(bad),
            "and it never reached the participant's screen as the page to visit: {bad}"
        );
        assert!(
            load_grant(tmp.path()).unwrap().is_none(),
            "and nothing was stored: {bad}"
        );
    }

    // The two shapes that are allowed, reached through the same door: the real server, and
    // the loopback development one this crate's own tests run against.
    for good in [
        "https://goalcup.example/device",
        "http://127.0.0.1:8123/device",
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let server = mock_pending_forever(Box::leak(good.to_string().into_boxed_str()), false);
        let mut o = opts(server);
        o.poll_limit = 1;
        let mut out = Vec::new();
        let err = run(tmp.path(), &o, &mut out).expect_err("this server never authorizes");
        assert!(
            format!("{err}").contains("the code was never entered"),
            "the URI was accepted and the flow ran to its own end: {err}"
        );
        assert!(
            String::from_utf8(out).unwrap().contains(good),
            "and the participant was told where to go: {good}"
        );
    }
}

/// The device-grant poll is bounded by a wall clock and not only by a request count, so
/// repeated `slow_down` cannot stretch it.
///
/// `DEFAULT_POLL_LIMIT` is 140 requests; a server answering `slow_down` walks the wait to
/// `MAX_INTERVAL_SECONDS` and holds the terminal about seventy minutes on a code the same
/// file documents as living about ten.
#[test]
fn repeated_slow_down_cannot_stretch_the_poll_past_its_deadline() {
    let tmp = tempfile::tempdir().unwrap();
    let server = mock_pending_forever("https://example.invalid/device", true);
    let mut o = opts(server);
    // A budget no wall clock would ever spend, so the only thing that can end this is the
    // deadline — which is the whole assertion.
    o.poll_limit = 100_000;
    o.poll_deadline = Duration::from_millis(300);
    let began = std::time::Instant::now();
    let err = run(tmp.path(), &o, &mut Vec::new()).expect_err("the deadline ends it");
    assert_eq!(err.exit_code(), 1);
    let said = format!("{err}");
    assert!(
        said.contains("as long as a device code lives"),
        "the refusal is the deadline's and not the budget's: {said}"
    );
    assert!(
        began.elapsed() < Duration::from_secs(30),
        "and it came back at the deadline rather than at the ceiling of 100000 polls"
    );
    assert!(
        load_grant(tmp.path()).unwrap().is_none(),
        "nothing was stored"
    );
}
