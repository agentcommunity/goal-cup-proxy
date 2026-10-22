//! Restarts across session limits (docs/research/RESTARTS-DESIGN.md): `goalcup continue` after
//! a stop, with the chat-mode stand-in harness and the upload server's stand-in on loopback.

use crate::common::m2_cli_support::Rig;
use crate::common::upload_server::{ServerOptions, UploadServer};

/// A test build's disk sizes: the reservation, no margin, a 50 ms watch.
const DISK: (&str, &str) = ("GOALCUP_TEST_DISK", "4194304,0,50");
/// The uploader's cadence a test can watch, with a heartbeat every 100 ms.
const BEATING: (&str, &str) = (
    "GOALCUP_TEST_UPLOAD_POLICY",
    "200,25,100,400,10000,268435456,3000,100",
);

fn harness(rig: &Rig, name: &str) -> String {
    rig.harness_file(name)
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// The run's clone is gone when the participant continues: a sandbox whose work folder was on a
/// disk that did not survive the stop, or a folder moved away. The continue is refused before
/// anything changes, here or on the server, and names the folder; with the folder back in place
/// the next continue goes on in it. (Before, the missing folder was made again, empty, and the
/// agent went on in it with none of its work.)
#[test]
fn a_continue_whose_clone_is_gone_is_refused_and_changes_nothing() {
    let rig = Rig::chat();
    let server = UploadServer::start(&rig.root.join("server"), ServerOptions::default());
    rig.prepare_for(&server.url());
    let mut first = rig.start_on_pty(&[DISK, BEATING]);
    first.wait_shown("tui: ready");
    first.type_line("one");
    first.wait_shown("tui: noted one");
    first.type_line("/exit");
    assert!(first.wait().success(), "{}", first.shown());
    let run_id = server.run_ids()[0].clone();
    let workspace = goalcup_cli::run::read(&rig.cfg, &run_id).unwrap().workspace;
    let away = rig.root.join("moved-away");
    std::fs::rename(&workspace, &away).unwrap();
    let (agent, folders) = (harness(&rig, "tui.pid"), rig.run_dirs());

    let mut refused = rig.continue_on_pty(&[DISK, BEATING], None);
    let status = refused.wait();
    let said = refused.shown();
    assert_eq!(status.code(), Some(1), "{said}");
    assert!(said.contains(&workspace.display().to_string()), "{said}");
    assert!(said.contains("Nothing was changed"), "{said}");
    assert!(!workspace.exists(), "the folder was not made again");
    assert_eq!(
        server.entry_ids(&run_id).len(),
        1,
        "no segment was asked for"
    );
    assert_eq!(harness(&rig, "tui.pid"), agent, "no agent was started");
    assert_eq!(rig.run_dirs(), folders);

    std::fs::rename(&away, &workspace).unwrap();
    let mut again = rig.continue_on_pty(&[DISK, BEATING], None);
    again.wait_shown("tui: ready");
    again.type_line("/exit");
    assert!(again.wait().success(), "{}", again.shown());
    assert_eq!(server.entry_ids(&run_id).len(), 2);
    assert_eq!(rig.clones(), [workspace]);
}
