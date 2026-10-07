//! The eight template variables, built from what the daemon returned.
//!
//! Env-free by construction: every value under test comes out of a `WorkspaceInfo` or a
//! `ResumeInfo` the fixture built, so nothing here reads a variable and this binary is not
//! the one env-mutating test (`tests/env_resolution.rs` still is).
#![cfg(feature = "v1-compat")]

mod support;

use goalcup_cli::vars;
use goalcup_protocol::template::render;
use std::path::PathBuf;

#[test]
fn every_launch_and_resume_template_in_every_shipped_manifest_renders() {
    let info = support::workspace_info();
    let v = vars::from_workspace(&info);
    for id in ["claude-code-cli", "codex-cli"] {
        let m = support::connector(id).manifest;
        for a in m.launch.args.iter().chain(m.resume.args.iter()) {
            let out = render(a, &v).unwrap_or_else(|e| panic!("{id} `{a}`: {e}"));
            assert!(
                !out.contains('{'),
                "{id}: `{a}` left a placeholder on the command line"
            );
        }
        for (k, val) in &m.launch.env {
            let out = render(val, &v).unwrap_or_else(|e| panic!("{id} env {k}: {e}"));
            assert!(!out.contains('{'), "{id}: env {k} left a placeholder");
            if k == "GOALCUP_RUN" {
                assert_eq!(
                    out, info.entry_id,
                    "{id}: run_id is the entry id, never empty"
                );
            }
        }
        if let Some(sl) = &m.launch.status_line {
            assert_eq!(
                render(&sl.command_template, &v).unwrap(),
                format!("cat {}", info.status_line_path)
            );
        }
        for p in &m.preflight.required_paths {
            assert!(!render(p, &v).unwrap().contains('{'));
        }
    }
}

/// Every schema-1 capture template — argument and environment value — renders against the
/// variables the CLI supplies. A capture argument may keep a literal brace (`{{` in the
/// template), so the check is that rendering succeeds and leaves no `{name}` behind.
#[test]
fn every_capture_template_in_every_shipped_manifest_renders() {
    let info = support::workspace_info();
    let v = vars::from_workspace(&info);
    for id in ["claude-code-cli", "codex-cli"] {
        let loaded = goalcup_cli::connector::load_harness(&support::connectors_dir(), id, false)
            .expect("the schema-1 harness loads");
        let c = &loaded.loaded.manifest.capture;
        for t in c.args.iter().chain(c.env.values()) {
            let out = render(t, &v).unwrap_or_else(|e| panic!("{id} `{t}`: {e}"));
            for name in vars::SUPPLIED {
                assert!(
                    !out.contains(&format!("{{{name}}}")),
                    "{id}: `{t}` left {{{name}}} unrendered"
                );
            }
        }
    }
}

#[test]
fn a_resume_renders_to_the_same_strings_as_the_original_launch() {
    let info = support::workspace_info();
    let resume = support::resume_info(&info);
    let a = vars::from_workspace(&info);
    let b = vars::from_resume(&resume, &info.entry_id).expect("the ids match");
    for name in vars::SUPPLIED {
        assert_eq!(
            a.get(name),
            b.get(name),
            "{name} differs between launch and resume: the halves of one entry would \
             be captured differently"
        );
    }
    assert_eq!(vars::SUPPLIED.len(), 8);
    // `run_id` has no field of its own; it is the entry id, in both builders.
    assert_eq!(a.get("run_id"), Some(info.entry_id.as_str()));
    assert_eq!(b.get("run_id"), a.get("run_id"));
    assert_eq!(a.get("run_id"), a.get("entry_id"));
    // The fixture carries a reserved port BECAUSE the daemon does. Asserting equality over a
    // pair of `None`s would pass while a real resume rendered `--port ""`.
    assert!(
        resume.server_port.is_some(),
        "a resume whose recipe names {{server_port}} and did not get one is the hole P4 closed"
    );
    assert_eq!(a.get("server_port"), b.get("server_port"));
    assert_ne!(
        a.get("server_port"),
        Some(""),
        "the equality above is over a real port, not over two empty strings"
    );
}

#[test]
fn a_read_only_resume_preview_renders_the_same_launch_variables_as_resume() {
    let info = support::workspace_info();
    let resume = support::resume_info(&info);
    let preview = goalcup_protocol::control::ResumePreviewInfo {
        entry_id: resume.entry_id.clone(),
        process_generation: resume.process_generation,
        process_handoff_phase: "registered".to_string(),
        session_id: resume.session_id.clone(),
        workspace: resume.workspace.clone(),
        artifact_root: resume.artifact_root.clone(),
        harness_home: resume.harness_home.clone(),
        status_line_path: resume.status_line_path.clone(),
        server_port: resume.server_port,
    };
    let normal = vars::from_resume(&resume, &info.entry_id).expect("the ids match");
    let read_only =
        vars::from_resume_preview(&preview, &info.entry_id).expect("the preview ids match");
    for name in vars::SUPPLIED {
        assert_eq!(normal.get(name), read_only.get(name), "{name} drifted");
    }
}

#[test]
fn the_eight_are_exactly_the_names_task_2_pinned() {
    // The same list `tests/shipped_connectors.rs` asserts every shipped manifest's variables
    // are drawn from — that binary reads `vars::SUPPLIED` itself, so this is the one place the
    // eight names are written out literally. Importing here too would leave nothing pinning
    // the contents: the whole suite would agree by construction with whatever `src/vars.rs`
    // happened to say.
    let mut supplied = vars::SUPPLIED.to_vec();
    supplied.sort_unstable();
    assert_eq!(
        supplied,
        [
            "artifact_root",
            "entry_id",
            "harness_home",
            "run_id",
            "server_port",
            "session_id",
            "status_line_path",
            "workspace",
        ]
    );
    let info = support::workspace_info();
    let v = vars::from_workspace(&info);
    for name in vars::SUPPLIED {
        assert!(v.get(name).is_some(), "{name} is listed and not supplied");
    }
}

#[test]
fn a_name_no_builder_supplies_is_a_refusal_that_names_it() {
    let info = support::workspace_info();
    let v = vars::from_workspace(&info);
    let e = match render("--home {home}", &v) {
        Ok(out) => panic!("an unsupplied token rendered instead of refusing: `{out}`"),
        Err(e) => e.to_string(),
    };
    assert!(
        e.contains("home"),
        "a refusal that does not name the token is one nobody can act on: {e}"
    );
    // `PATH_TOKENS` is the *path* language the daemon fills, not this one. A manifest that
    // reached a launch template for one of its own names fails here rather than launching.
    assert!(!vars::SUPPLIED.contains(&"config_dir"));
}

#[test]
fn this_crate_writes_nothing_into_the_workspace() {
    // This crate has no writer at all (one writer, the daemon). A source file that is
    // not there is skipped; `src/vars.rs` is required.
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut checked = 0;
    for f in ["src/vars.rs", "src/launch.rs", "src/seal.rs"] {
        let path = here.join(f);
        let Ok(s) = std::fs::read_to_string(&path) else {
            continue;
        };
        checked += 1;
        assert!(
            !s.contains("fs::write") && !s.contains("File::create"),
            "{f} must not write into a workspace (R3: one writer, the daemon)"
        );
    }
    assert!(checked >= 1, "src/vars.rs is this task's and must be there");
    assert!(here.join("src/vars.rs").is_file());
}

#[test]
fn a_server_mode_launch_without_a_reserved_port_renders_an_empty_string_and_task_8_refuses_it() {
    let mut info = support::workspace_info();
    info.server_port = None;
    let v = vars::from_workspace(&info);
    assert_eq!(v.get("server_port"), Some(""));
    // No shipped connector names `{server_port}`, so the recipe is given one.
    let mut m = support::connector("codex-cli").manifest;
    m.launch.args.push("{server_port}".to_string());
    let rendered: Vec<String> = m
        .launch
        .args
        .iter()
        .map(|a| render(a, &v).unwrap())
        .collect();
    assert!(
        rendered.iter().any(|a| a.is_empty()),
        "an unreserved port renders empty here; Task 8 is where it is refused"
    );
}

/// The same hole on the resume side: a port the daemon never reserved reaches the recipe as
/// `None`. This module does not refuse a missing port. It renders the empty string,
/// deliberately (see `build`), and the launcher is where a `Server`-mode argv carrying one is
/// refused. What this test pins is the input that refusal fires on — so it is named for
/// that.
#[test]
fn a_resume_with_no_reserved_port_renders_an_empty_port_for_task_8_to_refuse() {
    let info = support::workspace_info();
    let mut resume = support::resume_info(&info);
    resume.server_port = None;
    let v = vars::from_resume(&resume, &info.entry_id).expect("the ids match");
    assert_eq!(v.get("server_port"), Some(""));
}

/// `from_resume` prefers the caller's `entry_id` over the daemon's echo, which
/// is right for the variable and silent about the rest of the answer: the workspace, the
/// harness home and the receiver token in a mismatched `ResumeInfo` belong to the *other*
/// entry, so a resume of `A` would launch into `B`'s directories under `A`'s name.
#[test]
fn a_resume_answered_for_another_entry_is_refused_naming_both_ids() {
    let info = support::workspace_info();
    let mut resume = support::resume_info(&info);
    resume.entry_id = "entry-9999".to_string();
    let e = match vars::from_resume(&resume, &info.entry_id) {
        Ok(_) => panic!("a mismatched entry id built launch variables instead of refusing"),
        Err(e) => e,
    };
    assert_eq!(e.exit_code(), 1, "a mismatch is the participant's rung");
    let msg = e.to_string();
    assert!(
        msg.contains("entry-0000") && msg.contains("entry-9999"),
        "a refusal that names neither id is one nobody can act on: {msg}"
    );
    // Unchanged when they agree — the check is the mismatch, not the parameter.
    assert!(vars::from_resume(&support::resume_info(&info), &info.entry_id).is_ok());
}

/// Values are byte-exact: what the daemon spelled is what renders, and the
/// quoting a shell would need is the consumer's business, not this module's.
#[test]
fn a_workspace_path_with_a_space_renders_unchanged_and_unquoted() {
    let mut info = support::workspace_info();
    let spaced = format!("{}/my agent runs", info.workspace);
    info.workspace = spaced.clone();
    let v = vars::from_workspace(&info);
    assert_eq!(v.get("workspace"), Some(spaced.as_str()));
    let out = render("--cwd {workspace}", &v).expect("a supplied name renders");
    assert_eq!(
        out,
        format!("--cwd {spaced}"),
        "the value is passed through byte for byte: no quoting, no escaping, no trimming"
    );
    assert!(
        !out.contains('\\') && !out.contains('"') && !out.contains('\''),
        "this module must not quote — Task 8 hands argv to Command::arg, which needs none"
    );
    // And a resume of the same run renders the same bytes.
    let mut resume = support::resume_info(&info);
    resume.workspace = spaced.clone();
    let b = vars::from_resume(&resume, &info.entry_id).expect("the ids match");
    assert_eq!(b.get("workspace"), v.get("workspace"));
}
