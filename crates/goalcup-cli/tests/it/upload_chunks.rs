//! The uploader's pieces that need no server: how a chunk is cut from the chain, and how long
//! a failure waits.

use goalcup_cli::upload::uploader::{Policy, Position, backoff, build_chunk};
use goalcup_protocol::chain::GENESIS;
use std::time::Duration;

/// A chain of `n` records, each line carrying only its link (as the envelope does, at the top
/// level) and some filler, and the hash each record leaves.
fn chain(n: u64) -> (Vec<u8>, Vec<String>) {
    let mut bytes = Vec::new();
    let mut hashes = Vec::new();
    let mut prev = GENESIS.to_string();
    for seq in 0..n {
        let hash = format!("{seq:064x}");
        bytes.extend_from_slice(
            format!(
                "{{\"seq\":{seq},\"prev\":\"{prev}\",\"hash\":\"{hash}\",\"body\":\"{}\"}}\n",
                "x".repeat(200)
            )
            .as_bytes(),
        );
        prev = hash.clone();
        hashes.push(hash);
    }
    (bytes, hashes)
}

#[test]
fn a_chunk_takes_whole_lines_from_the_servers_position_and_leaves_a_torn_tail() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut bytes, hashes) = chain(5);
    let whole = bytes.len();
    bytes.extend_from_slice(b"{\"seq\":5,\"prev\":\"");
    let path = tmp.path().join("chain.jsonl");
    std::fs::write(&path, &bytes).unwrap();
    let spool = tmp.path().join("spool.zst");

    let start = Position {
        offset: 0,
        next_seq: 0,
        head: GENESIS.to_string(),
    };
    let chunk = build_chunk(&path, &start, &spool).unwrap().unwrap();
    assert_eq!((chunk.first_seq, chunk.last_seq), (0, 4));
    assert_eq!(chunk.length, whole as u64);
    assert_eq!(chunk.head, hashes[4]);
    assert_eq!(chunk.prev, GENESIS);
    let body = std::fs::read(&spool).unwrap();
    assert_eq!(body.len() as u64, chunk.compressed);
    assert_eq!(zstd::decode_all(body.as_slice()).unwrap(), &bytes[..whole]);
    use sha2::Digest;
    assert_eq!(chunk.sha256, hex::encode(sha2::Sha256::digest(&body)));

    // From the middle: the server holds the first two records.
    let line = whole / 5;
    let middle = Position {
        offset: 2 * line as u64,
        next_seq: 2,
        head: hashes[1].clone(),
    };
    let chunk = build_chunk(&path, &middle, &spool).unwrap().unwrap();
    assert_eq!((chunk.first_seq, chunk.last_seq), (2, 4));

    // Caught up: nothing complete waits, and no spool is left.
    let end = Position {
        offset: whole as u64,
        next_seq: 5,
        head: hashes[4].clone(),
    };
    assert!(build_chunk(&path, &end, &spool).unwrap().is_none());
    assert!(!spool.exists());
}

#[test]
fn a_chain_that_does_not_continue_the_servers_copy_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let (bytes, hashes) = chain(3);
    let path = tmp.path().join("chain.jsonl");
    std::fs::write(&path, &bytes).unwrap();
    let line = (bytes.len() / 3) as u64;
    for (next_seq, head) in [(1, "f".repeat(64)), (2, hashes[1].clone())] {
        let wrong = Position {
            offset: line,
            next_seq,
            head,
        };
        let refused = build_chunk(&path, &wrong, &tmp.path().join("s.zst")).unwrap_err();
        assert!(
            refused.to_string().contains("does not continue"),
            "{refused}"
        );
    }
}

#[test]
fn a_failure_waits_longer_each_time_up_to_the_cap_and_never_less_than_asked() {
    let policy = Policy::default();
    let first = backoff(&policy, 1, None);
    assert!(first >= Duration::from_secs(60) && first <= Duration::from_secs(75));
    let fourth = backoff(&policy, 4, None);
    assert!(fourth >= Duration::from_secs(480) && fourth <= Duration::from_secs(600));
    for failures in [5, 9, 40] {
        let wait = backoff(&policy, failures, None);
        assert!(wait >= Duration::from_secs(900) && wait <= Duration::from_secs(1125));
    }
    assert_eq!(
        backoff(&policy, 1, Some(Duration::from_secs(3600))),
        Duration::from_secs(3600)
    );
}

/// A server's acknowledgement is kept only when the local record ending at the server's
/// offset is the one it names: this is how that record is found.
#[test]
fn the_record_ending_at_an_offset_is_found_and_nothing_else() {
    use goalcup_cli::upload::uploader::local_link_before;
    let tmp = tempfile::tempdir().unwrap();
    let (bytes, hashes) = chain(3);
    let path = tmp.path().join("chain.jsonl");
    std::fs::write(&path, &bytes).unwrap();
    let line = (bytes.len() / 3) as u64;
    assert_eq!(local_link_before(&path, line), Some((0, hashes[0].clone())));
    assert_eq!(
        local_link_before(&path, 3 * line),
        Some((2, hashes[2].clone()))
    );
    for nowhere in [0, line - 1, line + 5, 4 * line] {
        assert_eq!(local_link_before(&path, nowhere), None, "{nowhere}");
    }
}

/// The workspace repository's checked-out commit, from `.git` files alone.
#[test]
fn the_repo_head_is_read_from_the_git_files() {
    use goalcup_cli::upload::uploader::repo_head;
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    assert_eq!(repo_head(ws), None, "no repository");
    let git = ws.join(".git");
    std::fs::create_dir_all(git.join("refs/heads")).unwrap();
    let commit = "0123456789abcdef0123456789abcdef01234567";
    std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    assert_eq!(repo_head(ws), None, "nothing committed yet");
    std::fs::write(git.join("refs/heads/main"), format!("{commit}\n")).unwrap();
    assert_eq!(repo_head(ws).as_deref(), Some(commit));
    std::fs::remove_file(git.join("refs/heads/main")).unwrap();
    std::fs::write(
        git.join("packed-refs"),
        format!("# pack-refs with: peeled\n{commit} refs/heads/main\n"),
    )
    .unwrap();
    assert_eq!(repo_head(ws).as_deref(), Some(commit), "a packed branch");
    std::fs::write(git.join("HEAD"), format!("{}\n", commit.to_uppercase())).unwrap();
    assert_eq!(repo_head(ws).as_deref(), Some(commit), "a detached HEAD");
    std::fs::write(git.join("HEAD"), "ref: refs/../../etc/passwd\n").unwrap();
    assert_eq!(
        repo_head(ws),
        None,
        "a reference leaving .git is not followed"
    );
}

// ------------------------------------------------------------------ budgets with retries
//
// The uploader against the upload-server stand-in, through a forwarder that can answer a
// request itself: a status the stand-in never gives (a rejected chunk, a refused seal) for as
// many requests as a test says, then everything passes through. Backoffs are milliseconds.

mod retries {
    use super::chain;
    use crate::common::upload_server::{ServerOptions, UploadServer};
    use goalcup_cli::server::ServerConfig;
    use goalcup_cli::upload::api::OpenRunRequest;
    use goalcup_cli::upload::http::ServerClient;
    use goalcup_cli::upload::policy::{Policy, WrapUp};
    use goalcup_cli::upload::uploader::{Report, Uploader};
    use goalcup_cli::upload::{UploadRecord, fetch_receipt, open_run};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    const TOKEN: &str = "gct_test_0123456789abcdef";

    type Fault = Box<dyn FnMut(&str) -> Option<u16> + Send>;

    /// A forwarder in front of the stand-in. `fault` sees `"METHOD /path"` of each request and
    /// answers `Some(status)` to refuse it itself, `None` to pass it on.
    struct Front {
        url: String,
        seen: Arc<Mutex<Vec<String>>>,
        stop: Arc<AtomicBool>,
    }

    impl Front {
        fn start(target: String, fault: Fault) -> Front {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let seen = Arc::new(Mutex::new(Vec::new()));
            let stop = Arc::new(AtomicBool::new(false));
            let fault = Arc::new(Mutex::new(fault));
            let (seen_in, stop_in) = (seen.clone(), stop.clone());
            std::thread::spawn(move || {
                while !stop_in.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let (target, fault, seen) =
                                (target.clone(), fault.clone(), seen_in.clone());
                            std::thread::spawn(move || answer(stream, &target, &fault, &seen));
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(5)),
                    }
                }
            });
            Front { url, seen, stop }
        }

        fn count(&self, prefix: &str) -> usize {
            self.seen
                .lock()
                .unwrap()
                .iter()
                .filter(|line| line.starts_with(prefix))
                .count()
        }
    }

    impl Drop for Front {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
        }
    }

    fn answer(
        mut stream: TcpStream,
        target: &str,
        fault: &Mutex<Fault>,
        seen: &Mutex<Vec<String>>,
    ) {
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
            return;
        }
        let (mut headers, mut length) = (Vec::new(), 0usize);
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                let (name, value) = (name.trim().to_string(), value.trim().to_string());
                if name.eq_ignore_ascii_case("content-length") {
                    length = value.parse().unwrap();
                }
                headers.push((name, value));
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        let mut parts = request_line.split_whitespace();
        let (method, path) = (parts.next().unwrap(), parts.next().unwrap());
        let label = format!("{method} {path}");
        seen.lock().unwrap().push(label.clone());
        let injected = (fault.lock().unwrap())(&label);
        let (status, content_type, reply) = match injected {
            Some(status) => (
                status,
                "application/json".to_string(),
                br#"{"error":"injected","message":"refused by the test"}"#.to_vec(),
            ),
            None => {
                let agent = ureq::AgentBuilder::new()
                    .timeout(Duration::from_secs(10))
                    .build();
                let mut request = agent.request(method, &format!("{target}{path}"));
                for (name, value) in &headers {
                    let skip = ["host", "content-length", "connection", "transfer-encoding"];
                    if !skip.iter().any(|s| name.eq_ignore_ascii_case(s)) {
                        request = request.set(name, value);
                    }
                }
                let response = match request.send_bytes(&body) {
                    Ok(response) | Err(ureq::Error::Status(_, response)) => response,
                    Err(error) => panic!("the stand-in did not answer: {error}"),
                };
                let content_type = response
                    .header("Content-Type")
                    .unwrap_or("application/json")
                    .to_string();
                (
                    response.status(),
                    content_type,
                    response.into_string().unwrap().into_bytes(),
                )
            }
        };
        let head = format!(
            "HTTP/1.1 {status} X\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            reply.len()
        );
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(&reply);
    }

    /// A run opened on the stand-in, three chain records and a seal on disk, and an uploader
    /// whose server is `front`.
    struct Run {
        _dir: tempfile::TempDir,
        server: UploadServer,
        front: Front,
        record: UploadRecord,
        key: String,
        spool: std::path::PathBuf,
    }

    impl Run {
        fn new(fault: Fault) -> Run {
            let dir = tempfile::tempdir().unwrap();
            let server = UploadServer::start(
                &dir.path().join("server"),
                ServerOptions {
                    tokens: vec![(TOKEN.to_string(), "member-1".to_string())],
                    ..ServerOptions::default()
                },
            );
            let config = |url: String| ServerConfig {
                url,
                token: TOKEN.to_string(),
            };
            // The run is opened straight on the stand-in; only the uploader goes through the front.
            let direct = ServerClient::new(&config(server.url()));
            let request = OpenRunRequest {
                connector_id: "fake".into(),
                manifest_sha256: "a".repeat(64),
                connector_dir_sha256: "b".repeat(64),
                workspace: "/nowhere".into(),
                session_id: "00000000-0000-4000-8000-000000000001".into(),
                harness_version: "1".into(),
                tracer_version: "0.1.0".into(),
                repo_url: "https://github.com/goal-cup/demo".into(),
            };
            let opened = open_run(&direct, &request).unwrap();
            let receipt =
                fetch_receipt(&direct, &opened, &request, goalcup_protocol::chain::GENESIS)
                    .unwrap();
            let trace = dir.path().join("trace");
            std::fs::create_dir_all(&trace).unwrap();
            std::fs::write(trace.join("chain.jsonl"), chain(3).0).unwrap();
            std::fs::write(trace.join("seal.json"), "{\"sealed\":true}").unwrap();
            std::fs::write(trace.join("baseline.json"), "{\"baseline\":true}").unwrap();
            let front = Front::start(server.url(), fault);
            let record = UploadRecord {
                run_id: opened.run_id,
                entry_id: opened.entry_id,
                server_url: front.url.clone(),
                trace_dir: trace.display().to_string(),
                official_start: opened.official_start,
                deadline_at: opened.deadline_at,
                workspace: String::new(),
            };
            let spool = dir.path().join("uploads");
            std::fs::create_dir_all(&spool).unwrap();
            Run {
                _dir: dir,
                server,
                front,
                record,
                key: receipt.public_key,
                spool,
            }
        }

        fn uploader(&self) -> Uploader {
            let client = ServerClient::new(&ServerConfig {
                url: self.front.url.clone(),
                token: TOKEN.to_string(),
            });
            Uploader::resume(
                client,
                self.record.clone(),
                self.key.clone(),
                quick(),
                &self.spool,
            )
            .unwrap()
        }
    }

    /// Backoffs of 10 to 40 ms; nothing else matters to a finish.
    fn quick() -> Policy {
        Policy::from_test_spec("1000,10,10,40,5000,268435456,2000").unwrap()
    }

    fn is_chunk(label: &str) -> bool {
        label.starts_with("PUT /v1/runs/")
    }

    fn finish(run: &Run, wrap: Duration) -> (Report, Vec<String>, Duration) {
        let mut uploader = run.uploader();
        let mut said = Vec::new();
        let began = Instant::now();
        let report = uploader.finish(Some(WrapUp::after(wrap)), &mut |line| {
            said.push(line.to_string())
        });
        (report, said, began.elapsed())
    }

    #[test]
    fn a_rejected_chunk_is_retried_not_the_end() {
        // The first two chunks are refused as too large (413), the way a rule being deployed
        // might refuse them; the third is accepted.
        let mut refused = 0;
        let run = Run::new(Box::new(move |label| {
            (is_chunk(label) && {
                refused += 1;
                refused <= 2
            })
            .then_some(413)
        }));
        let (report, said, _) = finish(&run, Duration::from_secs(20));
        assert!(report.caught_up && report.sealed, "{report:?}");
        assert_eq!(
            run.front.count("PUT /v1/runs/"),
            3,
            "two refusals, then the chunk"
        );
        assert!(
            said.iter()
                .any(|line| line.contains("413") && line.contains("trying again")),
            "{said:?}"
        );
        let chunks = run.server.chunks(&run.record.run_id);
        assert_eq!(chunks.len(), 1);
        assert_eq!(
            run.server.status(&run.record.run_id).as_deref(),
            Some("sealed")
        );
    }

    #[test]
    fn finish_keeps_sending_after_the_seal_until_the_server_answers() {
        // The run is sealed on this machine (seal.json and baseline.json are on disk). The
        // server answers 503, then 400 to the final heartbeat and the seal, and only then
        // answers: the finish keeps asking through all of it.
        let mut asked = 0;
        let run = Run::new(Box::new(move |label| {
            // The uploader's own first look at where the server is (`resume`) is not counted.
            if label.starts_with("GET") {
                return None;
            }
            asked += 1;
            if asked <= 3 {
                Some(503)
            } else if label.ends_with("/heartbeat") && asked <= 5
                || label.ends_with("/seal") && asked <= 9
            {
                Some(400)
            } else {
                None
            }
        }));
        let (report, said, _) = finish(&run, Duration::from_secs(20));
        assert!(report.caught_up && report.sealed, "{report:?}\n{said:?}");
        assert!(
            run.server.seal(&run.record.run_id).is_some(),
            "the server holds the seal"
        );
        assert!(run.server.final_commit(&run.record.run_id).is_some());
        assert!(
            said.iter().any(|line| line.contains("400")),
            "the refused calls were said: {said:?}"
        );
    }

    #[test]
    fn a_401_ends_the_upload_at_once() {
        let run = Run::new(Box::new(|label| is_chunk(label).then_some(401)));
        let (report, _, took) = finish(&run, Duration::from_secs(20));
        assert!(!report.caught_up && !report.sealed, "{report:?}");
        assert!(
            report.why.as_deref().unwrap_or("").contains("401"),
            "{report:?}"
        );
        assert_eq!(run.front.count("PUT /v1/runs/"), 1, "asked once, not again");
        assert!(took < Duration::from_secs(5), "{took:?}");
    }

    #[test]
    fn a_finish_gives_up_when_its_wrap_up_has_passed() {
        let run = Run::new(Box::new(|label| (!label.starts_with("GET")).then_some(500)));
        let (report, said, took) = finish(&run, Duration::from_millis(600));
        assert!(!report.sealed, "{report:?}");
        assert!(
            report
                .why
                .as_deref()
                .unwrap_or("")
                .starts_with("the time left to finish ran out; the last answer was: ")
                && report.why.as_deref().unwrap_or("").contains("500"),
            "{report:?}"
        );
        assert!(
            said.len() >= 2,
            "it kept asking while the wrap-up lasted: {said:?}"
        );
        assert!(
            took < Duration::from_secs(5),
            "and stopped when it ran out: {took:?}"
        );
    }
}
