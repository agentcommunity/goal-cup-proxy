//! The report shape every check renders into, and the one gate that reads a connector: the
//! directory pin. `super`'s `pub use` re-exports every item at one path.

use crate::connector::HarnessConnector;
use serde::Serialize;

/// The one spelling of the one door. `goalcup start --dev` is the place a human types it
/// (the V1 `goalcup doctor --verify-with-dev` was the other until D48), and the header of
/// [`super`] lists the four things it admits.
pub(crate) const DEV_DOOR: &str = "--dev";

/// What one check can say.
///
/// Three levels and not two, because "the harness is not installed on this machine" and
/// "the harness is installed and is the wrong version" are different answers and a
/// participant reading a `goalcup doctor` needs to tell them apart. `Warn` never fails a
/// report; [`PreflightReport::failed`] counts `Fail` alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Pass,
    Warn,
    Fail,
}

impl Level {
    /// The word this level renders as, padded to one width so a report's levels line up.
    pub(crate) fn word(self) -> &'static str {
        match self {
            Level::Pass => "PASS",
            Level::Warn => "WARN",
            Level::Fail => "FAIL",
        }
    }
}

/// One question, its answer, and what to do about the answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    /// Stable across releases and never translated: this is what a script greps for.
    pub id: String,
    pub level: Level,
    /// What was found, in the participant's own terms, naming the variable, the port, the
    /// digest or the version it is about.
    pub detail: String,
    /// The command or the change that makes this check stop saying what it just said.
    /// Empty only where there is nothing to do — a `Pass`.
    pub fix: String,
}

impl Check {
    /// One check as a participant reads it: the level, the id, the detail, and the fix
    /// indented under it where there is one to give.
    ///
    /// The one renderer. [`PreflightReport::render`] calls it for a connector's checks and
    /// `goalcup doctor` calls it for the daemon's, so the daemon's line and a connector's
    /// line cannot drift into two layouts that read as two reports.
    pub(crate) fn render(&self) -> String {
        let mut out = format!("  {}  {:<16}{}\n", self.level.word(), self.id, self.detail);
        if !self.fix.is_empty() {
            out.push_str(&format!("        fix: {}\n", self.fix));
        }
        out
    }

    pub(crate) fn new(id: &str, level: Level, detail: String, fix: &str) -> Check {
        Check {
            id: id.to_string(),
            level,
            detail,
            fix: fix.to_string(),
        }
    }

    pub(crate) fn pass(id: &str, detail: String) -> Check {
        Check::new(id, Level::Pass, detail, "")
    }
}

/// Every check for one connector, in the order they are asked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct PreflightReport {
    pub connector_id: String,
    pub checks: Vec<Check>,
}

impl PreflightReport {
    /// Whether anything here refuses the run. `Warn` does not; `Fail` does, and a harness
    /// that is not installed is a `Fail` (`harness_executable`). Any installed build passes:
    /// it is recorded, not pinned (D53).
    pub(crate) fn failed(&self) -> bool {
        self.checks.iter().any(|c| c.level == Level::Fail)
    }

    /// The report as a participant reads it: the connector, then [`Check::render`] for each
    /// of its checks.
    pub(crate) fn render(&self) -> String {
        let mut out = format!("{}\n", self.connector_id);
        for c in &self.checks {
            out.push_str(&c.render());
        }
        out
    }
}

/// The schema-1 capture connector directory pin, using the same one-door levels and wording.
///
/// A connector whose declared id is pinned never reaches the Dev door: an altered copy remains
/// a failure even when `--dev` was typed. The loader separately refuses `Origin::Dev` for any
/// directory whose digest is a shipped pin.
pub fn harness_pin_gate(c: &HarnessConnector, dev: bool) -> Check {
    let id = &c.loaded.manifest.id;
    let found = &c.loaded.connector_dir_sha256;
    match goalcup_protocol::pins::pinned_connector_dir_sha256(id) {
        Some(_)
            if goalcup_protocol::pins::is_pinned(
                goalcup_protocol::pins::CONNECTOR_DIR_SHA256S,
                id,
                found,
            ) =>
        {
            Check::pass(
                "manifest_pin",
                format!("`{id}`'s connector files are the ones this build pins ({found})."),
            )
        }
        Some(pinned) => Check::new(
            "manifest_pin",
            Level::Fail,
            format!(
                "`{id}`'s connector files are not the ones this build pins: they hash to \
                 {found}, and this build pins {pinned}. A pinned connector never receives \
                 {DEV_DOOR} relaxations."
            ),
            &format!("restore the shipped `connectors/{id}` directory."),
        ),
        None => {
            let what = format!(
                "no build pins `{id}` — its connector files ({found}) are hand-written, or \
                 from a release this build does not know. Nothing has agreed to the rules \
                 this run would be judged by."
            );
            dev_or_fail(what, dev)
        }
    }
}

/// The one door, applied to a connector this build does not pin.
fn dev_or_fail(what: String, dev: bool) -> Check {
    if dev {
        Check::new(
            "manifest_pin",
            Level::Warn,
            format!(
                "{what} {DEV_DOOR} was typed, and {DEV_DOOR} is the only thing that admits \
                 it. This run is a development run and its results are not a judged entry's."
            ),
            "",
        )
    } else {
        Check::new(
            "manifest_pin",
            Level::Fail,
            what,
            &format!(
                "restore the shipped `connectors/<id>/` directory, or pass {DEV_DOOR} and \
                 accept that the run is a development run."
            ),
        )
    }
}
