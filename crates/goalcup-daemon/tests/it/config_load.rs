//! The daemon's config file: the measured defaults, the two fields it refuses by name, and
//! the three refusals `validate()` makes.

use goalcup_daemon::config::{DaemonConfig, ReceiptBindingPolicy};

fn base(extra: &str) -> String {
    format!(
        r#"
artifact_root = "/tmp/gc/artifacts"
connectors_dir = "/tmp/gc/connectors"
config_dir = "/tmp/gc/config"
workspace_parent = "/tmp/gc/workspaces"
control_addr = "127.0.0.1:7788"
upload_url = "http://127.0.0.1:9000"
server_public_key = "{}"
{extra}
"#,
        "ab".repeat(32)
    )
}

#[test]
fn local_capture_config_needs_no_service_pair() {
    let local = base("")
        .lines()
        .filter(|line| !line.starts_with("upload_url") && !line.starts_with("server_public_key"))
        .collect::<Vec<_>>()
        .join("\n");
    for text in [
        local.clone(),
        format!("{local}\nupload_url = \"\"\nserver_public_key = \"\"\n"),
    ] {
        let config = DaemonConfig::from_toml_str(&text).expect("a local config parses");
        config.validate().expect("local capture needs no service");
        assert!(config.upload_url.is_empty());
        assert!(config.server_public_key.is_empty());
    }
}

#[test]
fn a_service_pair_must_be_supplied_together() {
    let local = base("")
        .lines()
        .filter(|line| !line.starts_with("upload_url") && !line.starts_with("server_public_key"))
        .collect::<Vec<_>>()
        .join("\n");
    for field in [
        "upload_url = \"http://127.0.0.1:9000\"".to_string(),
        format!("server_public_key = \"{}\"", "ab".repeat(32)),
    ] {
        let config = DaemonConfig::from_toml_str(&format!("{local}\n{field}\n"))
            .expect("a partial pair parses before validation");
        let error = config
            .validate()
            .expect_err("a partial pair refuses")
            .to_string();
        assert!(
            error.contains("upload_url") && error.contains("server_public_key"),
            "{error}"
        );
    }
}

#[test]
fn the_defaults_are_the_measured_ones_and_the_cut_fields_are_gone() {
    let c = DaemonConfig::from_toml_str(&base("")).unwrap();
    c.validate().unwrap();
    assert_eq!(
        c.poll_interval_ms, 250,
        "spike 5 kept up with 2,371 records at 0.25 s"
    );
    assert_eq!(c.heartbeat_seconds, 60);
    assert_eq!(c.clock_skew_seconds, 120.0);
    assert_eq!(c.disk_reserve_bytes, 256 << 20);
    // A config that sets either of these fields must not silently load,
    // and the refusal must name the field: "invalid config" sends a participant looking
    // at the wrong line of a file whose every other line is fine.
    for cut in [
        "unix_socket = \"/tmp/gc.sock\"",
        "foreign_scan_interval_ms = 30000",
    ] {
        let field = cut.split(' ').next().unwrap();
        let e = DaemonConfig::from_toml_str(&base(cut))
            .expect_err("a cut field must be a load error, not a silently ignored one")
            .to_string();
        assert!(e.contains(field), "the refusal must name {field}: {e}");
    }
}

#[test]
fn a_valid_new_event_receipt_binding_section_loads_and_validates() {
    let config = DaemonConfig::from_toml_str(&base(
        r#"
[receipt_binding]
mode = "new_event"
questionnaire_version = "2026.09-v1"
"#,
    ))
    .expect("the closed new-event receipt policy parses");
    config.validate().expect("the configured version is valid");
    assert_eq!(
        config.receipt_binding,
        ReceiptBindingPolicy::NewEvent {
            questionnaire_version: "2026.09-v1".to_string()
        }
    );
}

#[test]
fn receipt_binding_defaults_legacy_and_refuses_partial_or_invalid_modes() {
    let legacy = DaemonConfig::from_toml_str(&base("")).expect("old config parses");
    assert_eq!(legacy.receipt_binding, ReceiptBindingPolicy::Legacy);

    for invalid in [
        "[receipt_binding]\nmode = \"legacy\"\nquestionnaire_version = \"2026.09-v1\"\n",
        "[receipt_binding]\nmode = \"new_event\"\n",
        "[receipt_binding]\nmode = \"future\"\n",
    ] {
        DaemonConfig::from_toml_str(&base(invalid))
            .expect_err("the closed receipt policy must refuse this shape");
    }

    let invalid = DaemonConfig::from_toml_str(&base(
        "[receipt_binding]\nmode = \"new_event\"\nquestionnaire_version = \"contains spaces\"\n",
    ))
    .expect("shape parses before semantic validation");
    let error = invalid
        .validate()
        .expect_err("the shared questionnaire grammar is enforced")
        .to_string();
    assert!(
        error.contains("receipt_binding.questionnaire_version"),
        "{error}"
    );
}

#[test]
fn a_non_loopback_control_address_is_refused() {
    let c =
        DaemonConfig::from_toml_str(&base("").replace("127.0.0.1:7788", "0.0.0.0:7788")).unwrap();
    let e = c.validate().unwrap_err().to_string();
    assert!(e.contains("loopback"), "{e}");
    // This refusal is what makes the three unauthenticated read routes safe.
}

#[test]
fn a_bad_server_key_and_a_workspace_parent_beside_the_artifact_root_are_both_refused() {
    let bad_key = DaemonConfig::from_toml_str(&base("").replace(&"ab".repeat(32), "nope")).unwrap();
    assert!(
        bad_key
            .validate()
            .unwrap_err()
            .to_string()
            .contains("server_public_key"),
        "an unparseable key would make every acknowledgement silently unverifiable"
    );
    // Agents enumerate the parent directory on their first tool call.
    let adjacent =
        DaemonConfig::from_toml_str(&base("").replace("/tmp/gc/workspaces", "/tmp/gc")).unwrap();
    assert!(adjacent.validate().is_err());
}

/// Both roots, spelled by the caller. The nesting refusal is about a *pair* of paths, so
/// the cases below need to move both of them, not just the one `base()` hard-codes.
fn with_roots(artifact_root: &str, workspace_parent: &str) -> String {
    base("")
        .replace("/tmp/gc/artifacts", artifact_root)
        .replace("/tmp/gc/workspaces", workspace_parent)
}

fn nesting_refusal(artifact_root: &str, workspace_parent: &str) -> String {
    let c = DaemonConfig::from_toml_str(&with_roots(artifact_root, workspace_parent))
        .expect("the pair parses; it is validate() that owes the refusal");
    let e = c
        .validate()
        .expect_err("an entangled workspace_parent and artifact_root must be refused (R3)")
        .to_string();
    assert!(e.contains("nested"), "{e}");
    e
}

/// D5: the agent must never see its own evidence. A `..` in either root is not a
/// spelling quirk — it is the shortest way to write a workspace parent that lands inside
/// the artifact root while passing a component-wise `starts_with` on the raw strings.
#[test]
fn a_dot_dot_that_walks_the_workspace_parent_into_the_artifact_root_is_refused() {
    nesting_refusal("/tmp/gc/artifacts", "/tmp/other/../gc/artifacts/ws");
    // And the same trick spelled in the artifact root instead.
    nesting_refusal("/tmp/gc/x/../workspaces/evidence", "/tmp/gc/workspaces");
}

/// The reverse direction of the same entanglement: the artifact root sitting *inside*
/// the workspace parent. `nests()` is symmetric and this is the half no test covered.
#[test]
fn an_artifact_root_inside_the_workspace_parent_is_refused() {
    nesting_refusal("/tmp/gc/workspaces/artifacts", "/tmp/gc/workspaces");
}

/// Lexical normalisation alone cannot see this one: neither path contains a `..` and
/// neither is a prefix of the other as written. Only resolving the symlink shows that
/// the agent would be enumerating the artifact root.
#[cfg(unix)]
#[test]
fn a_symlinked_workspace_parent_pointing_inside_the_artifact_root_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let artifacts = dir.path().join("artifacts");
    std::fs::create_dir_all(artifacts.join("inside")).unwrap();
    let link = dir.path().join("workspaces");
    std::os::unix::fs::symlink(artifacts.join("inside"), &link).unwrap();

    nesting_refusal(artifacts.to_str().unwrap(), link.to_str().unwrap());
}

/// A `..` that crosses a symlink does not mean what it reads as. With
/// `link -> artifacts/inner`, the kernel resolves `link/../ws` to `artifacts/ws` — inside
/// the artifact root — while any pass that collapses the `..` before consulting the
/// filesystem computes `link`'s *parent* instead and waves the pair through. This is the
/// case that forces the resolution to canonicalise first and normalise afterwards.
#[cfg(unix)]
#[test]
fn a_dot_dot_crossing_a_symlink_into_the_artifact_root_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let artifacts = dir.path().join("artifacts");
    std::fs::create_dir_all(artifacts.join("inner")).unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(artifacts.join("inner"), &link).unwrap();

    let workspace_parent = link.join("..").join("ws");
    nesting_refusal(
        artifacts.to_str().unwrap(),
        workspace_parent.to_str().unwrap(),
    );
}

/// A root resolved against the daemon's working directory writes somewhere different
/// depending on who started it, so every one of the four is refused by name.
#[test]
fn a_relative_root_is_refused_by_name() {
    for field in [
        "artifact_root",
        "connectors_dir",
        "config_dir",
        "workspace_parent",
    ] {
        let absolute = match field {
            "artifact_root" => "/tmp/gc/artifacts",
            "connectors_dir" => "/tmp/gc/connectors",
            "config_dir" => "/tmp/gc/config",
            _ => "/tmp/gc/workspaces",
        };
        let c = DaemonConfig::from_toml_str(&base("").replace(absolute, "gc/relative")).unwrap();
        let e = c
            .validate()
            .expect_err("a relative root must be refused")
            .to_string();
        assert!(e.contains(field), "the refusal must name {field}: {e}");
        assert!(e.contains("absolute"), "{e}");
    }
}

/// A zero poll interval is a tailer that spins without waiting; a zero chunk maximum is
/// an uploader whose chunks carry nothing and never drain the buffer. Both are configs
/// that load cleanly and then do no work, so both are refused by name.
#[test]
fn a_zero_cadence_or_chunk_maximum_is_refused_by_name() {
    for (field, extra) in [
        ("poll_interval_ms", "poll_interval_ms = 0"),
        ("upload_chunk_max", "upload_chunk_max = 0"),
    ] {
        let e = DaemonConfig::from_toml_str(&base(extra))
            .unwrap()
            .validate()
            .expect_err("zero must be refused")
            .to_string();
        assert!(e.contains(field), "the refusal must name {field}: {e}");
    }
}

/// The uploader speaks HTTP. A `file://` or bare-host `upload_url` would be a chunk
/// destination the signing path cannot reach, discovered only once a run is under way.
#[test]
fn an_upload_url_that_is_not_http_is_refused_by_name() {
    for bad in ["file:///tmp/gc/uploads", "127.0.0.1:9000", ""] {
        let c =
            DaemonConfig::from_toml_str(&base("").replace("http://127.0.0.1:9000", bad)).unwrap();
        let e = c
            .validate()
            .expect_err("a non-http upload_url must be refused")
            .to_string();
        assert!(
            e.contains("upload_url"),
            "the refusal must name upload_url: {e}"
        );
    }
}

/// The `[event]` section is optional, complete, readable, and in order.
///
/// The daemon signs these three fields with the run key, and an agent is told
/// that it may act on them. So the file is where they are checked: a date this build cannot
/// read would be signed and handed to a reader that cannot read it either, and a start
/// window that closed before the round opened is a window no entry could start in — both of
/// which a participant should learn from `goalcup-daemon --config` refusing at startup
/// rather than from a status page days later.
#[test]
fn the_optional_event_section_is_absent_complete_and_in_order() {
    // Absent is the normal case: no round configured, and the status document falls back to
    // the entry's own timeline while saying that it has.
    let none = DaemonConfig::from_toml_str(&base("")).unwrap();
    assert!(none.event.is_none());

    let round = "[event]\n\
                 t0 = \"2026-10-01T00:00:00Z\"\n\
                 start_window_closes_at = \"2026-10-08T00:00:00Z\"\n\
                 rules_version = \"rules-2026-10-01\"\n";
    let c = DaemonConfig::from_toml_str(&base(round)).unwrap();
    c.validate().unwrap();
    let event = c.event.expect("the section loaded");
    assert_eq!(event.t0, "2026-10-01T00:00:00Z");
    assert_eq!(event.start_window_closes_at, "2026-10-08T00:00:00Z");
    assert_eq!(event.rules_version, "rules-2026-10-01");

    // All three together, or none: two of the three would mix a configured date with an
    // invented one under one `event_status_source`, which is a document nobody can act on.
    for missing in ["t0", "start_window_closes_at", "rules_version"] {
        let partial: String = round
            .lines()
            .filter(|l| !l.starts_with(missing))
            .collect::<Vec<_>>()
            .join("\n");
        let e = DaemonConfig::from_toml_str(&base(&partial))
            .expect_err("a half-configured round must not load")
            .to_string();
        assert!(e.contains(missing), "the refusal must name {missing}: {e}");
    }

    // A date this build cannot read. `parse_z` is the reader every comparison of these
    // strings goes through, so what it refuses is exactly what a reader of the document
    // could not have used.
    let e = DaemonConfig::from_toml_str(&base(
        &round.replace("2026-10-01T00:00:00Z", "1 October 2026"),
    ))
    .unwrap()
    .validate()
    .expect_err("an unreadable instant is refused")
    .to_string();
    assert!(e.contains("t0") && e.contains("1 October 2026"), "{e}");

    // And the order. A window that closes before it opens.
    let e = DaemonConfig::from_toml_str(&base(
        &round.replace("2026-10-08T00:00:00Z", "2026-09-01T00:00:00Z"),
    ))
    .unwrap()
    .validate()
    .expect_err("a start window that closed before t0 is refused")
    .to_string();
    assert!(
        e.contains("t0") && e.contains("start_window_closes_at"),
        "{e}"
    );
}
