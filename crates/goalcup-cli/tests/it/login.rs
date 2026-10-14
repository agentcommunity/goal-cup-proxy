//! `goalcup login`: the server and the account are stored at mode 0600 in the tracker's own
//! configuration directory. The device flow itself is `account.rs`'s; this file holds the
//! stored file, the address rules and the binary's side of the command.

use goalcup_cli::server::{ServerConfig, read_server_config, server_path, write_server_config};
use std::process::{Command, Stdio};

#[test]
fn the_server_and_account_are_stored_for_the_owner_alone() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("cfg");
    let config = ServerConfig {
        url: "https://ingest.example.org".into(),
        account: Some("ada".into()),
    };
    let path = write_server_config(&cfg, &config).unwrap();
    assert_eq!(read_server_config(&cfg).unwrap().unwrap(), config);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "server.json is mode {:o}", mode & 0o777);
    }
}

/// A `server.json` an older `goalcup login` wrote holds a pasted token. It is still read (so an
/// upgrade does not break `status`), the token is ignored, and the next write drops it.
#[test]
fn an_older_server_file_with_a_token_is_read_and_the_token_is_ignored() {
    let tmp = tempfile::tempdir().unwrap();
    let old_token = "gct_live_9f8e7d6c5b4a39281706f5e4d3c2b1a0";
    goalcup_cli::config::write_secret_file(
        &server_path(tmp.path()),
        format!(r#"{{"url":"https://ingest.example.org/","token":"{old_token}"}}"#).as_bytes(),
    )
    .unwrap();
    let stored = read_server_config(tmp.path()).unwrap().unwrap();
    assert_eq!(stored.url, "https://ingest.example.org");
    assert_eq!(stored.account, None);
    assert!(
        !format!("{stored:?}").contains(old_token),
        "the token is not carried"
    );
    write_server_config(tmp.path(), &stored).unwrap();
    let text = std::fs::read_to_string(server_path(tmp.path())).unwrap();
    assert!(!text.contains(old_token), "{text}");
}

#[test]
fn local_test_servers_and_paths_are_accepted_and_normalised() {
    use goalcup_cli::server::checked_server_url;
    for (given, stored) in [
        ("http://localhost:8787", "http://localhost:8787"),
        ("http://[::1]:8787/", "http://[::1]:8787"),
        (
            "HTTPS://Ingest.Example.org/Goal/Cup/",
            "https://ingest.example.org/Goal/Cup",
        ),
        ("http://127.0.0.1", "http://127.0.0.1"),
    ] {
        assert_eq!(checked_server_url(given).unwrap(), stored);
    }
    // A refusal never repeats a URL that carries a password.
    let refusal = checked_server_url("https://me:hunter2@ingest.example.org").unwrap_err();
    assert!(!refusal.to_string().contains("hunter2"), "{refusal}");
}

#[test]
fn a_bad_server_address_is_refused() {
    use goalcup_cli::server::checked_server_url;
    for server in [
        "ingest.example.org",
        "http://ingest.example.org",
        "https://",
        "https://user:pw@ingest.example.org",
        "https://ingest.example.org/?t=1",
        "https://ingest.example.org:80x",
        "http://127.0.0.1.evil.example",
        "http://[127.0.0.1]",
        "https://ingest.example.org:0",
        "https://ingest.example.org:99999",
        "https://a_b.example.org",
        "https://ingest.example.org/%2e%2e",
    ] {
        assert!(
            checked_server_url(server).is_err(),
            "{server:?} was accepted"
        );
    }
}

#[test]
fn a_corrupt_or_oversized_server_file_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    for bytes in [b"not json".to_vec(), vec![b' '; 64 * 1024]] {
        goalcup_cli::config::write_secret_file(&server_path(tmp.path()), &bytes).unwrap();
        let refusal = read_server_config(tmp.path()).unwrap_err();
        assert_eq!(refusal.exit_code(), 1, "{refusal}");
    }
}

#[cfg(unix)]
#[test]
fn a_server_file_others_can_read_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let config = ServerConfig {
        url: "https://ingest.example.org".into(),
        account: None,
    };
    let path = write_server_config(tmp.path(), &config).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let refusal = read_server_config(tmp.path()).unwrap_err();
    assert_eq!(refusal.exit_code(), 1);
}

/// The binary no longer reads a token: `goalcup login` is the browser approval. A bad address
/// is refused before anything is asked or written, and a good one that nothing answers writes
/// nothing, whatever stdin holds.
#[test]
fn the_login_command_approves_in_the_browser_and_reads_no_token() {
    use std::io::Write;
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("cfg");
    let out = Command::new(env!("CARGO_BIN_EXE_goalcup"))
        .args(["login", "--server", "http://ingest.example.org"])
        .env("GOALCUP_CONFIG_DIR", &cfg)
        .env("GOALCUP_TEST_NO_BROWSER", "1")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(!cfg.exists());

    // Nothing is listening: the approval cannot start, and a pasted line is not taken instead.
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", closed.local_addr().unwrap());
    drop(closed);
    let token = "gct_live_9f8e7d6c5b4a39281706f5e4d3c2b1a0";
    let mut child = Command::new(env!("CARGO_BIN_EXE_goalcup"))
        .args(["login", "--server", &url])
        .env("GOALCUP_CONFIG_DIR", &cfg)
        .env("GOALCUP_TEST_NO_BROWSER", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let _ = child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{token}\n").as_bytes());
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("could not be reached"), "{said}");
    assert!(!server_path(&cfg).exists(), "a failed login wrote a file");
}
