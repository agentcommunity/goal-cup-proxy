//! The presentation can change without owning setup state or a harness process.

use crate::common::m2_cli_support::{OnPty, Rig};
use goalcup_cli::{
    prep_file,
    ui::{Choice, Menu, render, theme::Style},
    welcome,
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    process::{Command, Stdio},
};

fn connectors() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../connectors")
}

#[test]
fn plain_and_narrow_rendering_is_static_and_cannot_inject_terminal_controls() {
    let choices = [Choice {
        label: "Example\x1b[2J\nlabel".into(),
        hint: None,
    }];
    let menu = Menu {
        title: "Pick",
        subtitle: "An agent",
        choices: &choices,
    };
    let style = Style::plain(32, 12);
    let first = render(&menu, 0, 0, style);
    assert_eq!(first, render(&menu, 0, 7, style));
    assert!(!first.contains('\x1b'));
    assert!(first.lines().all(|line| line.len() < 32));
    assert!(first.contains("/goal-cup"));
}

#[test]
fn a_short_terminal_keeps_the_selected_choice_visible() {
    let choices: Vec<_> = (0..20)
        .map(|i| Choice {
            label: format!("Agent {i}"),
            hint: None,
        })
        .collect();
    let menu = Menu {
        title: "Pick",
        subtitle: "An agent",
        choices: &choices,
    };
    let frame = render(&menu, 19, 0, Style::plain(32, 10));
    assert!(frame.contains("> Agent 19"));
    assert!(frame.lines().count() <= 10);
}

#[test]
fn discovery_reports_executable_availability_without_saving_a_choice() {
    let rig = Rig::chat();
    let env = BTreeMap::from([("PATH".into(), rig.path())]);
    let choices = welcome::harness_choices(&rig.connectors, &env, true).unwrap();
    assert!(!choices.is_empty());
    assert!(choices.iter().all(|choice| choice.installed));
    let missing = welcome::harness_choices(
        &rig.connectors,
        &BTreeMap::from([("PATH".into(), "".into())]),
        true,
    )
    .unwrap();
    assert!(missing.iter().all(|choice| !choice.installed));
    assert!(prep_file::read_prep_lenient(&rig.cfg).harness.is_empty());
}

fn private_command(cfg: &std::path::Path, home: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_goalcup"));
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", home)
        .env("GOALCUP_CONFIG_DIR", cfg)
        .env("GOALCUP_CONNECTORS_DIR", connectors())
        .env("GOALCUP_NO_BROWSER", "1")
        .env("GOALCUP_NO_ANIMATION", "1")
        .env("LANG", "C.UTF-8")
        .env("TERM", "xterm-256color");
    command
}

#[test]
fn a_pipe_and_json_output_never_enter_the_picker() {
    let root = tempfile::tempdir().unwrap();
    let cfg = root.path().join("cfg");
    let output = private_command(&cfg, root.path())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(!output.stdout.contains(&27) && !output.stderr.contains(&27));
    assert!(!cfg.exists());
    let output = private_command(&cfg, root.path())
        .args(["prep", "--json"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let steps: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(steps.len(), 6);
    assert!(!output.stdout.contains(&27));
}

#[test]
fn cancelling_restores_terminal_modes_and_writes_no_setup() {
    let root = tempfile::tempdir().unwrap();
    let cfg = root.path().join("cfg");
    let mut command = Command::new("/bin/sh");
    command.args(["-c", "before=$(stty -g); \"$GOALCUP_BINARY\"; result=$?; after=$(stty -g); [ \"$before\" = \"$after\" ] && [ \"$result\" = 0 ]"])
        .env_clear().env("PATH", "/usr/bin:/bin").env("HOME", root.path())
        .env("GOALCUP_BINARY", env!("CARGO_BIN_EXE_goalcup"))
        .env("GOALCUP_CONFIG_DIR", &cfg).env("GOALCUP_CONNECTORS_DIR", connectors())
        .env("LANG", "C.UTF-8").env("TERM", "xterm-256color");
    let mut terminal = OnPty::spawn(command, None);
    terminal.wait_shown("Choose your harness");
    terminal.type_bytes("\x03");
    assert!(terminal.wait().success(), "{}", terminal.shown());
    assert!(!cfg.exists());
}

#[test]
fn an_arrow_key_can_open_the_guide_without_selecting_a_harness() {
    let root = tempfile::tempdir().unwrap();
    let cfg = root.path().join("cfg");
    let mut terminal = OnPty::spawn(private_command(&cfg, root.path()), None);
    terminal.wait_shown("Choose your harness");
    // All three bytes are sent together: buffered input must not hide the rest from poll.
    terminal.type_bytes("\x1b[A\r");
    assert!(terminal.wait().success(), "{}", terminal.shown());
    assert!(terminal.shown().contains(welcome::ADD_GUIDE));
    assert!(!cfg.exists());
}

#[test]
fn choosing_a_harness_uses_existing_prep_and_subsequent_setup_keeps_it() {
    let rig = Rig::chat_with_shared_run_home();
    let home = rig.root.join("private-home");
    std::fs::create_dir(&home).unwrap();
    let env = BTreeMap::from([("PATH".into(), rig.path())]);
    let expected = welcome::harness_choices(&rig.connectors, &env, true)
        .unwrap()
        .remove(0)
        .id;
    let command = || {
        let mut command = private_command(&rig.cfg, &home);
        command
            .args(["prep", "--dev"])
            .env("PATH", rig.path())
            .env("GOALCUP_CONNECTORS_DIR", &rig.connectors)
            .env(
                "GOALCUP_TEST_PREP",
                "ac=on,free=107374182400,hosts=ok,awake=ok",
            );
        command
    };
    let mut terminal = OnPty::spawn(command(), None);
    terminal.wait_shown("Choose your harness");
    terminal.type_bytes("\r");
    terminal.wait_shown("Press Enter to open");
    terminal.type_bytes("\x04");
    terminal.wait();
    assert_eq!(prep_file::read_prep_lenient(&rig.cfg).harness, expected);
    let mut terminal = OnPty::spawn(command(), None);
    terminal.wait_shown("Press Enter to open");
    assert!(!terminal.shown().contains("Choose your harness"));
    terminal.type_bytes("\x04");
    terminal.wait();
    assert_eq!(prep_file::read_prep_lenient(&rig.cfg).harness, expected);
}
