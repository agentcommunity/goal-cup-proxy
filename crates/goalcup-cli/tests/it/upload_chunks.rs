//! The uploader's pieces that need no server: how a chunk is cut from the chain, and how long
//! a failure waits.

use goalcup_cli::upload::uploader::{Policy, Position, backoff, build_chunk};
use goalcup_protocol::chain::GENESIS;
use goalcup_protocol::export::{ChainPart, chain_parts, part_file_name};
use std::path::Path;
use std::time::Duration;

/// The chain parts in `dir`, as the uploader lists them.
fn parts_in(dir: &Path) -> Vec<ChainPart> {
    chain_parts(dir).unwrap()
}

/// Write `bytes` as the parts of a chain in `dir`, cut at the byte offsets `cuts`.
fn write_parts(dir: &Path, bytes: &[u8], cuts: &[usize]) {
    let mut start = 0;
    for end in cuts.iter().copied().chain([bytes.len()]) {
        std::fs::write(dir.join(part_file_name(start as u64)), &bytes[start..end]).unwrap();
        start = end;
    }
}

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
    let chunk = build_chunk(&parts_in(tmp.path()), &start, &spool)
        .unwrap()
        .unwrap();
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
    let chunk = build_chunk(&parts_in(tmp.path()), &middle, &spool)
        .unwrap()
        .unwrap();
    assert_eq!((chunk.first_seq, chunk.last_seq), (2, 4));

    // Caught up: nothing complete waits, and no spool is left.
    let end = Position {
        offset: whole as u64,
        next_seq: 5,
        head: hashes[4].clone(),
    };
    assert!(
        build_chunk(&parts_in(tmp.path()), &end, &spool)
            .unwrap()
            .is_none()
    );
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
        let refused =
            build_chunk(&parts_in(tmp.path()), &wrong, &tmp.path().join("s.zst")).unwrap_err();
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
    let parts = parts_in(tmp.path());
    assert_eq!(
        local_link_before(&parts, line),
        Some((0, hashes[0].clone()))
    );
    assert_eq!(
        local_link_before(&parts, 3 * line),
        Some((2, hashes[2].clone()))
    );
    for nowhere in [0, line - 1, line + 5, 4 * line] {
        assert_eq!(local_link_before(&parts, nowhere), None, "{nowhere}");
    }
}

/// The same chain in three parts, the first two later deleted: the record ending at an offset
/// is found in whichever surviving part holds it, at its logical offset.
#[test]
fn the_record_ending_at_an_offset_is_found_in_its_part() {
    use goalcup_cli::upload::uploader::local_link_before;
    let tmp = tempfile::tempdir().unwrap();
    let (bytes, hashes) = chain(6);
    let line = bytes.len() / 6;
    write_parts(tmp.path(), &bytes, &[2 * line, 4 * line]);
    let parts = parts_in(tmp.path());
    // On a part boundary the record belongs to the part that ends there.
    for (record, end) in [(0, 1), (1, 2), (2, 3), (3, 4), (4, 5), (5, 6)] {
        assert_eq!(
            local_link_before(&parts, end * line as u64),
            Some((record, hashes[record as usize].clone())),
            "record {record}"
        );
    }
    std::fs::remove_file(tmp.path().join(part_file_name(0))).unwrap();
    let parts = parts_in(tmp.path());
    assert_eq!(
        local_link_before(&parts, line as u64),
        None,
        "that part is gone"
    );
    assert_eq!(
        local_link_before(&parts, 3 * line as u64),
        Some((2, hashes[2].clone()))
    );
}

#[test]
fn a_chunk_stops_at_a_part_end() {
    let tmp = tempfile::tempdir().unwrap();
    let (bytes, hashes) = chain(6);
    let line = bytes.len() / 6;
    write_parts(tmp.path(), &bytes, &[2 * line, 5 * line]);
    let spool = tmp.path().join("spool.zst");
    let parts = parts_in(tmp.path());

    // From the start: the first part's two records, though four more wait.
    let mut pos = Position {
        offset: 0,
        next_seq: 0,
        head: GENESIS.to_string(),
    };
    let chunk = build_chunk(&parts, &pos, &spool).unwrap().unwrap();
    assert_eq!((chunk.offset, chunk.length), (0, 2 * line as u64));
    assert_eq!((chunk.first_seq, chunk.last_seq), (0, 1));
    assert_eq!(
        zstd::decode_all(std::fs::read(&spool).unwrap().as_slice()).unwrap(),
        &bytes[..2 * line]
    );

    // The next chunk starts at the logical offset where the part ended, in the next part.
    pos = Position {
        offset: chunk.offset + chunk.length,
        next_seq: chunk.last_seq + 1,
        head: chunk.head.clone(),
    };
    let chunk = build_chunk(&parts, &pos, &spool).unwrap().unwrap();
    assert_eq!((chunk.first_seq, chunk.last_seq), (2, 4));
    assert_eq!(chunk.length, 3 * line as u64);
    assert_eq!(chunk.prev, hashes[1]);

    // From the middle of a part, still not past its end.
    let middle = Position {
        offset: 3 * line as u64,
        next_seq: 3,
        head: hashes[2].clone(),
    };
    let chunk = build_chunk(&parts, &middle, &spool).unwrap().unwrap();
    assert_eq!((chunk.first_seq, chunk.last_seq), (3, 4));

    // The last part, then caught up.
    let last = Position {
        offset: 5 * line as u64,
        next_seq: 5,
        head: hashes[4].clone(),
    };
    let chunk = build_chunk(&parts, &last, &spool).unwrap().unwrap();
    assert_eq!((chunk.first_seq, chunk.last_seq), (5, 5));
    let end = Position {
        offset: 6 * line as u64,
        next_seq: 6,
        head: hashes[5].clone(),
    };
    assert!(build_chunk(&parts, &end, &spool).unwrap().is_none());
    assert!(!spool.exists());
}

#[test]
fn upload_continues_from_the_server_offset_after_parts_were_deleted() {
    let tmp = tempfile::tempdir().unwrap();
    let (bytes, hashes) = chain(6);
    let line = bytes.len() / 6;
    write_parts(tmp.path(), &bytes, &[2 * line, 4 * line]);
    // The first part was acknowledged and deleted.
    std::fs::remove_file(tmp.path().join(part_file_name(0))).unwrap();
    let spool = tmp.path().join("spool.zst");
    let parts = parts_in(tmp.path());
    assert_eq!(parts[0].start, 2 * line as u64);

    // The server holds the first three records: the chunk is cut from the second part, at the
    // logical offset.
    let at = Position {
        offset: 3 * line as u64,
        next_seq: 3,
        head: hashes[2].clone(),
    };
    let chunk = build_chunk(&parts, &at, &spool).unwrap().unwrap();
    assert_eq!(chunk.offset, 3 * line as u64);
    assert_eq!((chunk.first_seq, chunk.last_seq), (3, 3));

    // The server is behind what was deleted: nothing is sent across the gap, and the sentence
    // says why.
    let behind = Position {
        offset: line as u64,
        next_seq: 1,
        head: hashes[0].clone(),
    };
    std::fs::remove_file(&spool).unwrap();
    let refused = build_chunk(&parts, &behind, &spool)
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("already deleted from this machine"),
        "{refused}"
    );
    assert!(!spool.exists());

    // Waiting bytes are logical too.
    use goalcup_cli::upload::uploader::waiting_bytes;
    assert_eq!(waiting_bytes(&parts, 3 * line as u64), 3 * line as u64);
    assert_eq!(waiting_bytes(&parts, 6 * line as u64), 0);
    assert_eq!(waiting_bytes(&[], 0), 0);
}

#[test]
fn a_closed_part_without_a_whole_record_is_an_error_not_a_stall() {
    let tmp = tempfile::tempdir().unwrap();
    let (bytes, hashes) = chain(3);
    let line = bytes.len() / 3;
    // Part 0 ends in a torn line (it cannot be, for a part the daemon closed); a later part
    // exists.
    std::fs::write(tmp.path().join(part_file_name(0)), &bytes[..line + 10]).unwrap();
    std::fs::write(
        tmp.path().join(part_file_name(line as u64 + 10)),
        &bytes[line + 10..],
    )
    .unwrap();
    let at = Position {
        offset: line as u64,
        next_seq: 1,
        head: hashes[0].clone(),
    };
    let error = build_chunk(&parts_in(tmp.path()), &at, &tmp.path().join("s.zst"))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("closed but holds no whole record"),
        "{error}"
    );
    // The same torn tail in the last (live) part just waits.
    std::fs::remove_file(tmp.path().join(part_file_name(line as u64 + 10))).unwrap();
    assert!(
        build_chunk(&parts_in(tmp.path()), &at, &tmp.path().join("s.zst"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn local_end_reads_the_last_part() {
    use goalcup_cli::upload::uploader::local_end;
    let tmp = tempfile::tempdir().unwrap();
    let (mut bytes, hashes) = chain(6);
    let line = bytes.len() / 6;
    let whole = bytes.len();
    write_parts(tmp.path(), &bytes, &[2 * line, 4 * line]);
    assert_eq!(
        local_end(&parts_in(tmp.path())),
        Some((whole as u64, 5, hashes[5].clone()))
    );
    // A torn tail in the last part is not a record.
    bytes.extend_from_slice(b"{\"seq\":6,");
    write_parts(tmp.path(), &bytes, &[2 * line, 4 * line]);
    assert_eq!(
        local_end(&parts_in(tmp.path())),
        Some((whole as u64, 5, hashes[5].clone()))
    );
    // A part just opened by a rotation holds nothing yet: the part before it answers.
    std::fs::write(tmp.path().join(part_file_name(whole as u64)), b"").unwrap();
    std::fs::write(
        tmp.path().join(part_file_name(4 * line as u64)),
        &bytes[4 * line..whole],
    )
    .unwrap();
    assert_eq!(
        local_end(&parts_in(tmp.path())),
        Some((whole as u64, 5, hashes[5].clone()))
    );
    // With the earlier parts deleted, offsets stay logical.
    std::fs::remove_file(tmp.path().join(part_file_name(0))).unwrap();
    std::fs::remove_file(tmp.path().join(part_file_name(2 * line as u64))).unwrap();
    assert_eq!(
        local_end(&parts_in(tmp.path())),
        Some((whole as u64, 5, hashes[5].clone()))
    );
    assert_eq!(local_end(&[]), None);
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
        /// When set, the position the server reports (`GET /v1/runs/{id}`) is rewritten to this
        /// `(offset, next_seq, head)`: a server that is wrong, or lying.
        lie: Arc<Mutex<Option<(u64, u64, String)>>>,
        /// When set, every heartbeat answer is rewritten to `stop: true` with this `reason`
        /// (`null` for none): a server that says the run is over, as W2's does for a run a
        /// newer start replaced.
        stop_reason: Arc<Mutex<Option<Option<String>>>>,
    }

    impl Front {
        fn start(target: String, fault: Fault) -> Front {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let seen = Arc::new(Mutex::new(Vec::new()));
            let stop = Arc::new(AtomicBool::new(false));
            let fault = Arc::new(Mutex::new(fault));
            let lie = Arc::new(Mutex::new(None));
            let stop_reason = Arc::new(Mutex::new(None));
            let (seen_in, stop_in, lie_in, reason_in) =
                (seen.clone(), stop.clone(), lie.clone(), stop_reason.clone());
            std::thread::spawn(move || {
                while !stop_in.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let (target, fault, seen, lie, reason) = (
                                target.clone(),
                                fault.clone(),
                                seen_in.clone(),
                                lie_in.clone(),
                                reason_in.clone(),
                            );
                            std::thread::spawn(move || {
                                answer(stream, &target, &fault, &seen, &lie, &reason)
                            });
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(5)),
                    }
                }
            });
            Front {
                url,
                seen,
                stop,
                lie,
                stop_reason,
            }
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
        lie: &Mutex<Option<(u64, u64, String)>>,
        stop_reason: &Mutex<Option<Option<String>>>,
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
                let status = response.status();
                let mut reply = response.into_string().unwrap().into_bytes();
                if method == "GET"
                    && path.matches('/').count() == 3
                    && let Some((offset, next_seq, head)) = lie.lock().unwrap().clone()
                {
                    let mut json: serde_json::Value = serde_json::from_slice(&reply).unwrap();
                    json["offset"] = offset.into();
                    json["next_seq"] = next_seq.into();
                    json["head"] = head.into();
                    reply = serde_json::to_vec(&json).unwrap();
                }
                if method == "POST"
                    && path.ends_with("/heartbeat")
                    && status == 200
                    && let Some(reason) = stop_reason.lock().unwrap().clone()
                {
                    let mut json: serde_json::Value = serde_json::from_slice(&reply).unwrap();
                    json["stop"] = true.into();
                    json["reason"] = reason.into();
                    reply = serde_json::to_vec(&json).unwrap();
                }
                (status, content_type, reply)
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
                account: None,
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
                account: None,
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
    fn a_chain_in_parts_is_uploaded_and_the_upload_goes_on_after_the_first_part_is_deleted() {
        let run = Run::new(Box::new(|_| None));
        let trace = std::path::PathBuf::from(&run.record.trace_dir);
        let (bytes, hashes) = chain(3);
        let line = bytes.len() / 3;
        // Not sealed yet, and two records in the first part.
        std::fs::remove_file(trace.join("seal.json")).unwrap();
        std::fs::write(trace.join("chain.jsonl"), &bytes[..2 * line]).unwrap();
        let (report, said, _) = finish(&run, Duration::from_secs(20));
        assert!(report.caught_up, "{report:?}\n{said:?}");
        assert_eq!(report.offset, 2 * line as u64);

        // The daemon rotated and recorded one more record; the first part was acknowledged and
        // is deleted. A new uploader (after a restart) resumes at the server's offset.
        std::fs::write(
            trace.join(goalcup_protocol::export::part_file_name(2 * line as u64)),
            &bytes[2 * line..],
        )
        .unwrap();
        std::fs::remove_file(trace.join("chain.jsonl")).unwrap();
        let (report, said, _) = finish(&run, Duration::from_secs(20));
        assert!(report.caught_up, "{report:?}\n{said:?}");
        assert_eq!(report.offset, 3 * line as u64);
        let chunks = run.server.chunks(&run.record.run_id);
        assert_eq!(chunks.len(), 2, "one chunk for each part");
        assert_eq!(chunks[1].offset, 2 * line as u64);
        assert_eq!(chunks[1].head, hashes[2]);
    }

    /// A run whose chain is two records in `chain.jsonl` and one in a later part, unsealed.
    fn run_in_two_parts(fault: Fault) -> (Run, usize) {
        let run = Run::new(fault);
        let trace = std::path::PathBuf::from(&run.record.trace_dir);
        let (bytes, _) = chain(3);
        let line = bytes.len() / 3;
        std::fs::remove_file(trace.join("seal.json")).unwrap();
        std::fs::write(trace.join("chain.jsonl"), &bytes[..2 * line]).unwrap();
        std::fs::write(
            trace.join(goalcup_protocol::export::part_file_name(2 * line as u64)),
            &bytes[2 * line..],
        )
        .unwrap();
        (run, line)
    }

    fn sweep_far_ahead(run: &Run, acknowledged: u64) -> usize {
        use goalcup_cli::run::{EntryRef, watch::Pruner};
        let entry = EntryRef {
            entry_id: run.record.entry_id.clone(),
            trace_dir: std::path::PathBuf::from(&run.record.trace_dir),
        };
        let mut pruner = Pruner::default();
        let t0 = Instant::now();
        pruner.sweep(
            std::slice::from_ref(&entry),
            &|_| acknowledged,
            t0,
            &mut |_| {},
        );
        pruner.sweep(
            std::slice::from_ref(&entry),
            &|_| acknowledged,
            t0 + Duration::from_secs(3 * 3600),
            &mut |_| {},
        );
        goalcup_protocol::export::chain_parts(&entry.trace_dir)
            .unwrap()
            .len()
    }

    #[test]
    fn a_server_position_the_local_chain_does_not_confirm_is_not_an_acknowledgement() {
        // Resume against a server that says it holds far more than exists, with a foreign head.
        let (run, _) = run_in_two_parts(Box::new(|_| None));
        *run.front.lie.lock().unwrap() = Some((10_000_000, 99_999, "f".repeat(64)));
        let uploader = run.uploader();
        assert_eq!(uploader.acknowledged(), 0);
        assert_eq!(
            sweep_far_ahead(&run, uploader.acknowledged()),
            2,
            "nothing is deleted behind a number the server only claimed"
        );
        drop(uploader);

        // An honest server: a signed acknowledgement raises the offset, and only that.
        *run.front.lie.lock().unwrap() = None;
        let mut uploader = run.uploader();
        let first = uploader.finish(Some(WrapUp::after(Duration::from_secs(20))), &mut |_| {});
        assert!(first.caught_up, "{first:?}");
        assert_eq!(uploader.acknowledged(), first.offset);
        assert!(first.offset > 0);
    }

    #[test]
    fn a_409_reread_moves_the_acknowledged_offset_only_to_a_confirmed_position() {
        // The first chunk is stored but its reply is lost: the retry gets a 409, and the
        // re-read reports a position the local chain confirms, which is then the offset.
        let (run, line) = run_in_two_parts(Box::new(|_| None));
        run.server.drop_next_chunk_reply();
        let mut uploader = run.uploader();
        let report = uploader.finish(Some(WrapUp::after(Duration::from_secs(20))), &mut |_| {});
        assert!(report.caught_up, "{report:?}");
        assert_eq!(uploader.acknowledged(), 3 * line as u64);

        // Now the same 409 path, but the re-read reports a far offset and a foreign head: the
        // uploader stops, and the offset it has verified does not move.
        let (run, line) = run_in_two_parts(Box::new(|label| {
            // Every chunk is answered 409 by the front itself.
            label.starts_with("PUT /v1/runs/").then_some(409)
        }));
        let mut uploader = run.uploader();
        *run.front.lie.lock().unwrap() = Some((line as u64, 1, "f".repeat(64)));
        let report = uploader.finish(Some(WrapUp::after(Duration::from_secs(20))), &mut |_| {});
        assert!(!report.caught_up, "{report:?}");
        assert_eq!(uploader.acknowledged(), 0);
        assert_eq!(sweep_far_ahead(&run, uploader.acknowledged()), 2);
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

    // ------------------------------------------------------------- what the heartbeat says

    use goalcup_cli::run::{Recording, Vitals};
    use goalcup_cli::upload::uploader::begin;
    use std::sync::atomic::AtomicU64;

    /// A heartbeat every 50 ms, a chunk interval and a poll long enough to stay out of the way.
    fn beating() -> Policy {
        Policy::from_test_spec("60000,20,10,40,5000,268435456,2000,50").unwrap()
    }

    fn start_beating(
        run: &Run,
        vitals: Vitals,
    ) -> (
        goalcup_cli::upload::uploader::Running,
        std::sync::mpsc::Receiver<String>,
    ) {
        let client = ServerClient::new(&ServerConfig {
            url: run.front.url.clone(),
            account: None,
            token: TOKEN.to_string(),
        });
        let (stop, stopped) = std::sync::mpsc::channel();
        let running = begin(
            client,
            run.record.clone(),
            run.key.clone(),
            beating(),
            run.spool.clone(),
            None,
            stop,
            vitals,
        )
        .unwrap();
        (running, stopped)
    }

    fn wait_for<T>(what: &str, mut found: impl FnMut() -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(found) = found() {
                return found;
            }
            assert!(Instant::now() < deadline, "{what} never happened");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn the_heartbeat_carries_recording_and_model_requests() {
        let run = Run::new(Box::new(|_| None));
        let counter = Arc::new(AtomicU64::new(7));
        let vitals = Vitals::counting(counter.clone());
        vitals.set_recording(Recording::Gaps);
        let (running, _stopped) = start_beating(&run, vitals.clone());
        let id = run.record.run_id.clone();
        let said = |recording: &str, requests: u64| {
            let heartbeats = run.server.heartbeats(&id);
            heartbeats
                .iter()
                .any(|h| {
                    !h.final_
                        && h.recording.as_deref() == Some(recording)
                        && h.model_requests == Some(requests)
                })
                .then_some(())
        };
        wait_for("a heartbeat saying gaps and 7", || said("gaps", 7));
        // What the tick and the proxy learn later is what the next heartbeat says.
        vitals.set_recording(Recording::Ok);
        counter.store(10, Ordering::SeqCst);
        wait_for("a heartbeat saying ok and 10", || said("ok", 10));
        drop(running);
    }

    #[test]
    fn a_replaced_answer_stops_the_run_with_its_reason() {
        let run = Run::new(Box::new(|_| None));
        let (running, stopped) = start_beating(&run, Vitals::default());
        // Beating along, and the server says nothing about stopping.
        wait_for("a first heartbeat", || {
            (run.front.count("POST /v1/runs/") >= 1).then_some(())
        });
        assert!(stopped.try_recv().is_err(), "stopped without a reason");
        *run.front.stop_reason.lock().unwrap() = Some(Some("replaced".to_string()));
        let reason = stopped
            .recv_timeout(Duration::from_secs(20))
            .expect("the run was not stopped");
        assert_eq!(reason, "a newer run on your account replaced this one");
        drop(running);

        // The same answer with the reason `deadline`, or none at all (an older server), is the
        // deadline sentence.
        for reason in [Some("deadline".to_string()), None] {
            let run = Run::new(Box::new(|_| None));
            *run.front.stop_reason.lock().unwrap() = Some(reason);
            let (running, stopped) = start_beating(&run, Vitals::default());
            let said = stopped
                .recv_timeout(Duration::from_secs(20))
                .expect("the run was not stopped");
            assert!(
                said.starts_with("the server says the run is over (its deadline was"),
                "{said}"
            );
            drop(running);
        }
    }
}
