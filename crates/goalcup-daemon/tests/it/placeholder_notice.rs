//! The daemon says so when its server key is the published placeholder.
//!
//! The placeholder's seed is a constant in this repository, so every receipt and
//! acknowledgement under it can be forged and a run anchored to it is anchored to nothing.
//! The daemon does not refuse (that would break every development run), but it prints one
//! line on stderr at startup, and prints nothing for any other key.
//!
//! The binary cases never start a daemon: the control port is occupied before the process
//! runs, so `Daemon::start` refuses, and the process exits by itself after the startup
//! notice, which is what these read.

use goalcup_daemon::config::DaemonConfig;
use goalcup_protocol::receipt::PLACEHOLDER_SERVER_PUBLIC_KEY_HEX;

/// The marker the notice carries, so a test can count it without pinning its whole wording.
const MARKER: &str = "placeholder";

fn config_text(root: &std::path::Path, control_addr: &str, key: &str) -> String {
    format!(
        r#"
artifact_root = "{r}/artifacts"
connectors_dir = "{r}/connectors"
config_dir = "{r}/config"
workspace_parent = "{r}/workspaces"
control_addr = "{control_addr}"
upload_url = "http://127.0.0.1:1"
server_public_key = "{key}"
"#,
        r = root.display()
    )
}

#[test]
fn the_notice_names_the_placeholder_key_and_no_other() {
    let root = std::path::Path::new("/tmp/gc-placeholder-notice");
    for key in [
        PLACEHOLDER_SERVER_PUBLIC_KEY_HEX.to_string(),
        PLACEHOLDER_SERVER_PUBLIC_KEY_HEX.to_ascii_uppercase(),
    ] {
        let config = DaemonConfig::from_toml_str(&config_text(root, "127.0.0.1:0", &key)).unwrap();
        let notice = config
            .placeholder_key_notice()
            .expect("the placeholder key, in either case, is noticed");
        assert!(notice.contains(MARKER), "{notice}");
        assert!(!notice.contains('\n'), "one line: {notice:?}");
    }
    for key in ["ab".repeat(32), String::new()] {
        let config = DaemonConfig::from_toml_str(&config_text(root, "127.0.0.1:0", &key)).unwrap();
        assert_eq!(config.placeholder_key_notice(), None, "key {key:?}");
    }
}

/// Run `goalcup-daemon --config` against a config whose control port is already taken, and
/// return stderr. The process refuses and exits on its own; it is never signalled.
fn stderr_of_a_refused_start(key: &str) -> String {
    let root = tempfile::tempdir().expect("tempdir");
    let taken = std::net::TcpListener::bind("127.0.0.1:0").expect("occupy a port");
    let addr = taken.local_addr().expect("addr").to_string();
    let config = root.path().join("daemon.toml");
    std::fs::write(&config, config_text(root.path(), &addr, key)).expect("write the config");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_goalcup-daemon"))
        .arg("--config")
        .arg(&config)
        .output()
        .expect("the binary runs");
    assert!(
        !out.status.success(),
        "the occupied port must refuse the start: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    drop(taken);
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn the_placeholder_key_produces_exactly_one_notice_line_at_startup() {
    let stderr = stderr_of_a_refused_start(PLACEHOLDER_SERVER_PUBLIC_KEY_HEX);
    let notices: Vec<&str> = stderr
        .lines()
        .filter(|l| l.contains(MARKER) && l.starts_with("goalcup-daemon:"))
        .collect();
    assert_eq!(notices.len(), 1, "stderr was: {stderr}");
    assert!(
        notices[0].contains("forge"),
        "the line says what the placeholder costs: {}",
        notices[0]
    );
}
