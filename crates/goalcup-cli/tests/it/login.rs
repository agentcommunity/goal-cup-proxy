//! `goalcup login`: the token is stored at mode 0600 in the tracker's own configuration
//! directory, and never printed.

use goalcup_cli::commands::login::login_with;
use goalcup_cli::server::{ServerConfig, read_server_config, server_path, write_server_config};
use std::process::{Command, Stdio};

const TOKEN: &str = "gct_live_9f8e7d6c5b4a39281706f5e4d3c2b1a0";

fn login(cfg: &std::path::Path, server: &str, input: &str) -> (Result<(), String>, String) {
    let mut out = Vec::new();
    let result =
        login_with(cfg, server, &mut input.as_bytes(), &mut out).map_err(|e| e.to_string());
    (result, String::from_utf8(out).unwrap())
}

#[test]
fn a_login_stores_the_server_and_token_for_the_owner_alone_and_never_prints_the_token() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("cfg");
    let (result, said) = login(&cfg, "https://ingest.example.org/", &format!("  {TOKEN}\n"));
    result.unwrap();
    assert!(!said.contains(TOKEN), "the token was printed: {said}");
    assert!(said.contains("https://ingest.example.org"), "{said}");

    let stored = read_server_config(&cfg).unwrap().unwrap();
    assert_eq!(stored.url, "https://ingest.example.org");
    assert_eq!(stored.token, TOKEN);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(server_path(&cfg))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "server.json is mode {:o}", mode & 0o777);
    }
    assert!(
        !format!("{stored:?}").contains(TOKEN),
        "Debug prints the token"
    );

    // A second login replaces the first.
    let other = "gct_live_00000000000000000000000000000001";
    login(&cfg, "http://127.0.0.1:8787", &format!("{other}\n"))
        .0
        .unwrap();
    let stored = read_server_config(&cfg).unwrap().unwrap();
    assert_eq!(
        (stored.url.as_str(), stored.token.as_str()),
        ("http://127.0.0.1:8787", other)
    );
}

#[test]
fn a_bad_server_address_or_token_is_refused_and_nothing_is_written() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("cfg");
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
        let (result, _) = login(&cfg, server, &format!("{TOKEN}\n"));
        assert!(result.is_err(), "{server:?} was accepted");
    }
    let near_miss = &TOKEN[..TOKEN.len() - 1];
    for input in [
        "\n".to_string(),
        "short\n".to_string(),
        format!("{TOKEN} {TOKEN}\n"),
        format!("{}\n", "x".repeat(600)),
        format!("{near_miss}\u{e9}\n"),
    ] {
        let (result, said) = login(&cfg, "https://ingest.example.org", &input);
        let refusal = result.expect_err("a bad token was accepted");
        assert!(
            !refusal.contains(near_miss) && !said.contains(near_miss),
            "a refusal repeated the token: {refusal}"
        );
    }
    assert!(!server_path(&cfg).exists(), "a refused login wrote a file");
    assert!(read_server_config(&cfg).unwrap().is_none());
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
fn a_new_login_replaces_a_loose_file_with_one_for_the_owner_alone() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(server_path(tmp.path()), b"{}").unwrap();
    std::fs::set_permissions(
        server_path(tmp.path()),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    login(
        tmp.path(),
        "https://ingest.example.org",
        &format!("{TOKEN}\n"),
    )
    .0
    .unwrap();
    let mode = std::fs::metadata(server_path(tmp.path()))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o077, 0, "mode {:o}", mode & 0o777);
}

#[cfg(unix)]
#[test]
fn a_server_file_others_can_read_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let config = ServerConfig {
        url: "https://ingest.example.org".into(),
        token: TOKEN.into(),
    };
    let path = write_server_config(tmp.path(), &config).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let refusal = read_server_config(tmp.path()).unwrap_err();
    assert_eq!(refusal.exit_code(), 1);
    assert!(!refusal.to_string().contains(TOKEN));
}

/// The binary reads the token from piped stdin and prints neither it nor any part of it.
#[test]
fn the_login_command_reads_the_token_from_stdin() {
    use std::io::Write;
    let tmp = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_goalcup"))
        .args(["login", "--server", "https://ingest.example.org"])
        .env("GOALCUP_CONFIG_DIR", tmp.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{TOKEN}\n").as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let (stdout, stderr) = (
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(out.status.success(), "{stdout}{stderr}");
    assert!(!stdout.contains(&TOKEN[..12]) && !stderr.contains(&TOKEN[..12]));
    assert_eq!(
        read_server_config(tmp.path()).unwrap().unwrap().token,
        TOKEN
    );

    // An address refused before anything is read or written.
    let out = Command::new(env!("CARGO_BIN_EXE_goalcup"))
        .args(["login", "--server", "http://ingest.example.org"])
        .env("GOALCUP_CONFIG_DIR", tmp.path().join("other"))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(!tmp.path().join("other").exists());
}
