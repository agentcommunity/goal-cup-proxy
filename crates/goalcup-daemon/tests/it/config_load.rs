//! The daemon's config file: the measured defaults, the fields it refuses by name, and
//! the refusals `validate()` makes.

use goalcup_daemon::config::DaemonConfig;

fn base(extra: &str) -> String {
    format!(
        r#"
artifact_root = "/tmp/gc/artifacts"
connectors_dir = "/tmp/gc/connectors"
config_dir = "/tmp/gc/config"
workspace_parent = "/tmp/gc/workspaces"
control_addr = "127.0.0.1:7788"
{extra}
"#
    )
}

#[test]
fn the_cut_fields_are_gone() {
    let c = DaemonConfig::from_toml_str(&base("")).unwrap();
    c.validate().unwrap();
    // A config that sets either of these fields must not silently load,
    // and the refusal must name the field: "invalid config" sends a participant looking
    // at the wrong line of a file whose every other line is fine.
    // The first two were cut with the V1 transports; the rest are the V1 service, receipt and
    // cadence keys the daemon used to accept and ignore.
    for cut in [
        "unix_socket = \"/tmp/gc.sock\"",
        "foreign_scan_interval_ms = 30000",
        "upload_url = \"http://127.0.0.1:9000\"",
        "server_public_key = \"abababab\"",
        "receipt_binding = { mode = \"legacy\" }",
        "poll_interval_ms = 0",
        "disk_reserve_bytes = 0",
    ] {
        let field = cut.split(' ').next().unwrap();
        let e = DaemonConfig::from_toml_str(&base(cut))
            .expect_err("a cut field must be a load error, not a silently ignored one")
            .to_string();
        assert!(e.contains(field), "the refusal must name {field}: {e}");
    }
}

#[test]
fn a_non_loopback_control_address_is_refused() {
    let c =
        DaemonConfig::from_toml_str(&base("").replace("127.0.0.1:7788", "0.0.0.0:7788")).unwrap();
    let e = c.validate().unwrap_err().to_string();
    assert!(e.contains("loopback"), "{e}");
    // This refusal is what makes the unauthenticated status route safe.
}

#[test]
fn a_workspace_parent_beside_the_artifact_root_is_refused() {
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
