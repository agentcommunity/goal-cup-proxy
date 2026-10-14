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
