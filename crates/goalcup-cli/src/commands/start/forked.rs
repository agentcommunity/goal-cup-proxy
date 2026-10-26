//! A capture entry from its fork on, and the states it passes through after the gate.

use super::finish::{finish, finish_unlaunched};
use super::proxy::Proxied;
use crate::CliError;
use crate::commands::say;
use crate::entries::EntryRecord;
use crate::launch::LaunchPlan;
use crate::run::RunRecord;
use crate::run::reserve::RunReserve;
use crate::upload::policy::WrapUp;
use goalcup_protocol::control::{CaptureFinishReport, CaptureWorkspaceInfo, DaemonClient};
use goalcup_protocol::process::{ProcessHandle, termination_bound};
use std::path::Path;
use std::process::{Child, ExitStatus};

/// A capture entry from the moment its child is forked: from here the entry is sealed, never
/// abandoned, and this command's own output failing ends nothing. A released harness is waited
/// for and the entry sealed either way. A parked child that is never released never ran, so there
/// is nothing to wait for.
///
/// Every way out of the later phases that does not run to the harness's own end goes through
/// [`Forked::unlaunched`] or [`Forked::sealed_launch`], so a run that ends early is sealed the
/// same way.
pub(super) struct Forked<'a> {
    pub(super) client: &'a (dyn DaemonClient + Sync),
    pub(super) cfg: &'a Path,
    pub(super) info: CaptureWorkspaceInfo,
    /// The recording proxy; `None` once a deadline finish has stopped it.
    pub(super) proxied: Option<Proxied>,
    /// This machine's record of the entry, with no pid yet.
    pub(super) template: EntryRecord,
    /// For a run opened on a server: given back if the harness is never launched.
    pub(super) give_back: Option<crate::upload::GiveBack>,
    /// What the uploader needs, for a run opened on a server.
    pub(super) upload: Option<UploadStart>,
    /// This machine's record of the run.
    pub(super) record: RunRecord,
    /// The run's one disk reservation, held from the entry's creation to the run's end, until
    /// the watch tick takes it over when the harness is released.
    pub(super) reserve: Option<RunReserve>,
    /// The watch tick, which has the reservation from the harness's release to just before the
    /// reservation is released.
    pub(super) watching: Option<crate::run::watch::Watching>,
    /// A chat-mode run's hold on the terminal, from just before the release to the end of the
    /// run's wrap-up.
    pub(super) chat: Option<ChatTerminal>,
    /// The lock that keeps a second run off the shared harness home. It is let go at the seal:
    /// the upload that follows needs no harness home, and the next segment or run may start.
    pub(super) home_lock: Option<std::fs::File>,
    /// The counters of the earlier segments of a continued run, being closed and sent beside
    /// this one: the disk watch covers their entries too.
    pub(super) drained: Vec<crate::upload::uploader::DrainHandles>,
    /// The harness's display name, for what the participant reads.
    pub(super) harness_name: String,
    /// The manifest's first-screen hint, shown just before a chat-mode harness opens.
    pub(super) first_screen: Option<String>,
    /// A first launch, or a continue that picks up the last session or starts a new one.
    pub(super) opens: crate::participant::Opens,
    /// What the participant puts at the start of their prompt, for a real run's first launch
    /// ([`crate::participant::prompt_block`]); empty otherwise.
    pub(super) prompt: Vec<String>,
    /// The run's ids and folders, held back from the launch block and written to its
    /// `host.log` once it is open.
    pub(super) details: Vec<String>,
}

/// What a chat-mode run holds while the harness owns the terminal (D57): the terminal's signals
/// absorbed, the host quiet, and the terminal's modes as they were before the harness took it.
pub(super) struct ChatTerminal {
    shield: goalcup_platform::terminal::Shield,
    pub(super) quiet: crate::host_log::Quiet,
    modes: Option<goalcup_platform::terminal::Modes>,
}

impl ChatTerminal {
    /// Taken just before the release, once the launch line is out.
    pub(super) fn take(run_dir: &Path) -> Result<ChatTerminal, CliError> {
        let shield = goalcup_platform::terminal::Shield::take().map_err(|source| CliError::Io {
            path: "the terminal's signals".to_string(),
            source,
        })?;
        Ok(ChatTerminal {
            shield,
            quiet: crate::host_log::quiet(run_dir),
            modes: goalcup_platform::terminal::Modes::save(libc::STDIN_FILENO),
        })
    }

    /// The harness has exited: Ctrl-C, Ctrl-\ and Ctrl-Z are this command's own again, and it
    /// speaks again if the terminal is still there. A hangup stays absorbed until the wrap-up is
    /// done ([`Drop`]), so a window closed now does not cut the seal or the upload short.
    /// The terminal's modes are put back first, while the terminal is there: a harness ended in
    /// raw mode would leave the participant's shell typing blind.
    pub(super) fn harness_exited(&mut self) {
        self.shield.harness_exited();
        if self.quiet.harness_exited()
            && let Some(modes) = &self.modes
            && let Err(e) = modes.restore()
        {
            crate::host_log::say(&format!("the terminal's modes could not be put back ({e})"));
        }
    }
}

/// What the uploader of a run opened on a server starts from.
pub(super) struct UploadStart {
    pub(super) server: Option<crate::upload::http::ServerClient>,
    pub(super) record: crate::upload::UploadRecord,
    /// The start receipt's key, which every acknowledgement must verify under.
    pub(super) key: String,
    pub(super) policy: crate::upload::uploader::Policy,
    /// The server's count of the run's time left, for when the server cannot say stop.
    pub(super) local_deadline: Option<crate::upload::uploader::LocalDeadline>,
}

impl Forked<'_> {
    /// This machine's record of the entry, with the harness's pid once there is one.
    pub(super) fn record(&self, last_pid: Option<u32>) -> EntryRecord {
        EntryRecord {
            last_pid,
            ..self.template.clone()
        }
    }

    /// End a run whose harness never ran or was never told about: the entry is sealed within
    /// the one budget, and the run is marked unscanned. Returns the error that ended it.
    pub(super) fn unlaunched(mut self, error: CliError, out: &mut dyn std::io::Write) -> CliError {
        // A chat-mode launch that failed before its log opened still says its ids and folders,
        // for support.
        if !self.details.is_empty() {
            let _ = say(out, &crate::participant::details_heading(None));
            for line in self.details.drain(..) {
                let _ = say(out, &line);
            }
        }
        let keeps_reserve = self.keeps_reserve();
        // Whatever ended the launch, including a Ctrl-C, the seal below runs its full retries.
        crate::arming::close_window();
        if let Some(proxied) = self.proxied {
            finish_unlaunched(self.client, &self.info.entry_id, proxied, out);
        }
        if let Some(reserve) = self.reserve {
            end_reserve(reserve, keeps_reserve);
        }
        if let Some(give_back) = self.give_back {
            give_back.give();
        }
        error
    }

    /// What is left of the run by the server's answer, counted from it on the continuous clock;
    /// `None` for a run with no server.
    pub(super) fn time_left(&self) -> Option<crate::run::TimeLeft> {
        self.upload
            .as_ref()
            .and_then(|start| start.local_deadline)
            .map(|count| count.time_left())
    }

    /// Whether the run's reserve file outlives a launch that failed: a segment of a server run
    /// that is not over goes on to be continued, and its one reservation is the run's, not the
    /// segment's. A new run that is given back to the server, and a run that is over, take their
    /// reserve with them.
    fn keeps_reserve(&self) -> bool {
        // Over by the server's count, not this machine's date (WP11).
        let left = self.time_left();
        self.give_back.is_none()
            && self.record.server_url.is_some()
            && !crate::run::time_left::time_up(
                left.as_ref(),
                self.record.deadline_at.as_deref(),
                crate::run::now_unix(),
            )
    }

    /// The root ruling under D40: the entry was sealed while its harness was being launched, so
    /// this command ends the child it just released, says so, and answers the committed seal.
    pub(super) fn sealed_launch(
        mut self,
        child: Child,
        handed: &ProcessHandle,
        out: &mut dyn std::io::Write,
    ) -> CliError {
        let keeps_reserve = self.keeps_reserve();
        if let Some(reserve) = self.reserve.take() {
            end_reserve(reserve, keeps_reserve);
        }
        let Forked {
            client,
            info,
            proxied,
            give_back,
            mut chat,
            ..
        } = self;
        // The harness is ended at once and nothing was uploaded: the run goes back.
        if let Some(give_back) = give_back {
            give_back.give();
        }
        let end = crate::launch::end_released_child(child, handed);
        // Ended: the terminal is this command's again.
        if let Some(chat) = chat.as_mut() {
            chat.harness_exited();
        }
        let _ = say(out, &end.announcement(&info.entry_id));
        let deadline = std::time::Instant::now() + crate::finish_budget();
        if let Some(proxied) = proxied {
            let _ = finish(
                client,
                &info.entry_id,
                proxied,
                deadline,
                WrapUp::one_attempt(),
                termination_bound(0),
                false,
                None,
                out,
            );
        }
        // Worded from what was observed, never more: a native child that could not be checked is
        // not "nothing is running".
        CliError::Refused(end.refusal(&info.entry_id))
    }
}

/// The end of a reserve this command holds when a launch failed: released, or dropped with the
/// file kept (its lock goes with it) for the run's next segment.
fn end_reserve(reserve: RunReserve, keep: bool) {
    if keep {
        drop(reserve);
    } else {
        release_and_say(reserve);
    }
}

/// Give the run's reserve back to the disk, and say so when that fails: the file keeps its
/// space until a later start or `goalcup status` removes it.
pub(super) fn release_and_say(reserve: RunReserve) {
    if let Err(error) = reserve.release() {
        crate::host_log::say(&format!(
            "the run's disk reservation could not be released ({error}); it keeps its space \
             until a later goalcup command removes it"
        ));
    }
}

/// What the gate holds for the phases after the fork.
pub(super) struct Gate {
    pub(super) plan: LaunchPlan,
    /// What the harness reported for its `version_args`, or `unreported` (D53).
    pub(super) harness_version: String,
    pub(super) goal_stream: goalcup_protocol::capture::StreamKey,
    pub(super) gated: crate::launch::GatedSpawn,
}

/// A harness registered and released.
pub(super) struct Released<'a> {
    pub(super) forked: Forked<'a>,
    pub(super) handed: ProcessHandle,
    pub(super) child: Child,
    pub(super) spawned_at: goalcup_protocol::capture::CaptureTime,
    pub(super) goal_stream: goalcup_protocol::capture::StreamKey,
    pub(super) harness_version: String,
}

/// A harness that has exited.
pub(super) struct Exited<'a> {
    pub(super) forked: Forked<'a>,
    pub(super) status: Result<ExitStatus, CliError>,
    /// The finish a deadline made while the harness was still running, if one did.
    pub(super) finished_early: Option<Result<CaptureFinishReport, CliError>>,
    /// The uploader, for a run opened on a server.
    pub(super) uploading: Option<crate::upload::uploader::Running>,
    /// The wrap-up after the stop, which the seal and the upload's finish share.
    pub(super) wrap_up: WrapUp,
    /// Linux: the refusal the command ends with, after the seal, when the sweep left a process
    /// running or could not read `/proc` after it had signalled one.
    pub(super) left_running: Option<CliError>,
    /// The run's state file.
    pub(super) keeper: crate::run::StateKeeper,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::reserve::{DiskPolicy, reserve_path};

    /// The reserve of a run that goes on is dropped, not released, when a launch fails.
    #[test]
    fn a_failed_launch_keeps_the_runs_reserve_file_only_when_the_run_goes_on() {
        let tmp = tempfile::tempdir().unwrap();
        let policy = DiskPolicy::from_test_spec("4194304,0,50").unwrap();
        let path = reserve_path(tmp.path(), "r1");
        end_reserve(RunReserve::take(&path, &policy).unwrap(), true);
        assert!(path.exists(), "kept for the run's next segment");
        assert!(!crate::run::reserve::held_by_another(&path), "and unlocked");
        end_reserve(RunReserve::take(&path, &policy).unwrap(), false);
        assert!(!path.exists(), "released");
    }
}
