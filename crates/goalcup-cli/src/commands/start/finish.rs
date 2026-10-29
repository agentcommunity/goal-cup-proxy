//! Phase five and the seal: the recording proxy stopped, the entry sealed within the one finish
//! budget, and what became of the run said.

use super::forked::{Exited, Forked, release_and_say};
use super::proxy::{Proxied, proxy_admission_line};
use super::{StartOutcome, pause_at};
use crate::CliError;
use crate::commands::say;
use crate::proxy::tunnels;
use crate::run::NoModelCalls;
use crate::run::reserve::artifact_root;
use crate::upload::policy::WrapUp;
use goalcup_protocol::control::{CaptureFinishReport, DaemonClient, ProxyCounters};
use goalcup_protocol::process::termination_bound;
use std::path::Path;
use std::process::ExitStatus;
use std::time::Duration;

/// Phase five: stop the recording proxy and seal within the one finish budget, and say what became
/// of the run.
pub(super) fn finish_run(
    exited: Exited<'_>,
    draining: Vec<crate::upload::uploader::Draining>,
    out: &mut dyn std::io::Write,
) -> Result<StartOutcome, CliError> {
    pause_at("capture-finish-started");
    let Exited {
        forked,
        status,
        finished_early,
        helper_left,
        uploading,
        wrap_up,
        left_running,
        keeper,
        silence,
    } = exited;
    let Forked {
        client,
        cfg,
        info,
        proxied,
        watching,
        chat,
        record,
        home_lock,
        harness_name,
        model_calls_before,
        ..
    } = forked;
    // A run's hold on the terminal (its hangup absorbed, its log open) lasts to the end
    // of this function: the seal and the upload's wrap-up below must outlive a closed window.
    let _chat = chat;
    // The recording proxy's live count of model calls, which keeps its last value once the proxy
    // stops: the tick's verdict can be up to a tick old, and a call recorded since overrides it.
    let calls = proxied
        .as_ref()
        .map(|proxied| proxied.proxy.generation_requests());
    let deadline = std::time::Instant::now() + crate::finish_budget();
    let finished = match (finished_early, proxied) {
        (Some(finished), _) => finished,
        (None, Some(proxied)) => finish(
            client,
            &info.entry_id,
            proxied,
            deadline,
            wrap_up,
            termination_bound(0),
            false,
            // What the harness left running when its after-exit commands failed (D72) would hold
            // the proxy's connections open until its own idle exit: the stop cuts them, and says
            // so if it cut any. What reached the proxy is sealed and uploaded as always.
            helper_left.then_some(harness_name.as_str()),
            Some(cfg),
            out,
        ),
        (None, None) => Err(CliError::Internal(
            "the run's proxy was stopped and nothing finished the run".to_string(),
        )),
    };
    // The run is over when it was stopped for its end (its deadline, or a newer run replacing
    // it), when its time is up by the count of the server's last answer (never this machine's
    // date), and always for a run with no server. Otherwise the agent's exit ended this segment
    // only: the run stays open on the server until its deadline and `goalcup continue` restarts
    // it.
    let snapshot = keeper.snapshot();
    let ended_by_stop = snapshot.stopped_for_its_end();
    let on_server = record.server_url.is_some();
    let over = !on_server || crate::run::is_over(&record, Some(&snapshot), crate::run::now_unix());
    // The model calls this part of the run recorded: the recorder's count, or, once a deadline
    // finish stopped the recorder, the count the harness's exit kept (this part's share of it).
    let recorded = match &calls {
        Some(calls) => calls.load(std::sync::atomic::Ordering::SeqCst),
        None => snapshot
            .model_calls
            .unwrap_or(0)
            .saturating_sub(model_calls_before.unwrap_or(0)),
    };
    // `goalcup continue` can restart the run: a run on a server that is not over. Only such a
    // run keeps its reserve past this host.
    let run_goes_on = crate::run::can_continue(cfg, &record, true, over);
    // The entry is sealed and the run goes on: the reserve is handed over before anything else
    // is let go. This host's watch stops managing it and its lock is dropped, the file kept, so
    // that a `goalcup continue` takes the same file at once instead of finding it held for as
    // long as this host's wrap-up lasts (an hour at most). The upload that follows needs none
    // of it, and the next segment's watch prunes this entry's acknowledged parts.
    let mut watching = watching;
    if run_goes_on {
        drop(watching.take().and_then(crate::run::watch::Watching::end));
        // The agent is gone and the tick that kept its time-left file has ended: the next
        // segment writes its own, and none can have started before the home is let go below.
        crate::run::time_left::remove_file(cfg, &record.run_id);
    }
    // The shared harness home is free for the next segment or run, and the upload that follows
    // needs none of it.
    drop(home_lock);
    // How the harness ended is the participant's to see: the exit code of a run that was recorded
    // is not this command's own (`main.rs`), so it is said here.
    if let Ok(status) = &status {
        let _ = say(out, &format!("harness exited: {}.", describe_exit(status)));
    }
    // What happened, and what to do now, said at the seal and before the wrap-up upload: from
    // here `goalcup continue` works (the home and the run's reserve were let go above), so a
    // participant need not wait up to the wrap-up's hour to read it. `goalcup continue` is advised
    // only when it can restart the run; otherwise what is true is said.
    let how = match &status {
        Ok(status) => describe_exit(status),
        Err(error) => error.to_string(),
    };
    let until = |deadline_at: &str| {
        crate::participant::end_words(
            deadline_at,
            crate::participant::seconds_left(
                snapshot.time_left.as_ref(),
                Some(deadline_at),
                crate::run::now_unix(),
            ),
        )
    };
    let stopped = snapshot.stopped.as_deref();
    use crate::participant::Ending;
    let ending = match (on_server, run_goes_on, over, record.deadline_at.as_deref()) {
        (false, ..) => None,
        (true, true, _, Some(deadline_at)) => Some(Ending::Open {
            run_id: &record.run_id,
            how: &how,
            until: until(deadline_at),
        }),
        (true, false, true, _) if ended_by_stop => Some(match stopped {
            Some(crate::upload::uploader::REPLACED_REASON) => Ending::Replaced,
            _ => Ending::TimeUp {
                practice: record.practice,
            },
        }),
        (true, false, true, _) => Some(Ending::Over { how: &how }),
        (true, false, false, Some(deadline_at)) => Some(Ending::NoRestart {
            how: &how,
            until: until(deadline_at),
        }),
        _ => None,
    };
    if let Some(ending) = &ending {
        for line in crate::participant::ending_block(&harness_name, ending) {
            let _ = say(out, &line);
        }
    }
    // The tick's last verdict, repeated for a participant whose agent has closed: a run that
    // recorded no model call is told so at its end too, local runs included.
    let restart = if run_goes_on {
        crate::participant::Restart::Continue {
            run_id: &record.run_id,
        }
    } else if over {
        crate::participant::Restart::Over
    } else {
        crate::participant::Restart::Cannot
    };
    // Said on every run that recorded no model call, whether or not the watch tick saw it silent.
    let silent = snapshot.no_model_calls.unwrap_or(silence);
    for line in no_model_calls_block(silent, recorded > 0, &harness_name, restart) {
        let _ = say(out, &line);
    }
    // After the seal: the rest of the chain and the seal itself, within the finish budget.
    // A local run has nothing to upload, so there is nothing of it left to send.
    let mut sent_whole = !on_server;
    let mut server_ended = false;
    let mut upload_report = None;
    // The wrap-up can take minutes: the participant is told to wait for it.
    if uploading.is_some() || !draining.is_empty() {
        let _ = say(out, "");
        let _ = say(out, crate::participant::UPLOADING_REST);
    }
    if let Some(running) = uploading {
        let report = running.finish(wrap_up);
        // Everything this entry has is on the server, and the seal; and the server may say the
        // run itself is sealed or incomplete.
        sent_whole = report.caught_up && report.sealed;
        server_ended = report.run_ended;
        let line = match &report {
            crate::upload::uploader::Report {
                caught_up: true,
                sealed: true,
                ..
            } => format!(
                "uploaded: {} bytes, acknowledged, and the seal.",
                report.offset
            ),
            _ => format!(
                "upload incomplete ({}): the run is complete on this machine. Run `goalcup \
                 upload {}` to finish sending it.",
                report.why.as_deref().unwrap_or("the seal was not sent"),
                info.entry_id
            ),
        };
        let _ = say(out, &line);
        upload_report = Some(report);
    }
    // The earlier segments of a continued run were being closed beside this one: what became of
    // each is said, and none is left unsent for a run that cannot be continued.
    for drain in draining {
        let entry = drain.entry_id().to_string();
        let how_to_finish = drain.how_to_finish();
        let report = drain.wait(wrap_up);
        sent_whole &= report.caught_up;
        let line = if report.caught_up {
            format!("uploaded: part {entry}, an earlier part of this run, acknowledged.")
        } else {
            format!(
                "upload of part {entry}, an earlier part of this run, is incomplete ({}). {how_to_finish}",
                report.why.as_deref().unwrap_or("the upload did not finish")
            )
        };
        let _ = say(out, &line);
    }
    // Last: whether the rest is on the server, and what that leaves to do.
    if let Some(ending) = &ending {
        for line in crate::participant::after_upload(ending, sent_whole, server_ended) {
            let _ = say(out, &line);
        }
    }
    // The tick ends here, just before the reservation goes (or is left to the next start): it
    // hands the reservation back even if it died on the way.
    let reserve = watching.and_then(crate::run::watch::Watching::end);
    // No agent of this run is left to read its time-left file. A run that goes on had it removed
    // at the seal; its next segment may be writing it by now.
    if !run_goes_on {
        crate::run::time_left::remove_file(cfg, &record.run_id);
    }
    // The host's last word on the run's state, now that nothing else writes it: what became of
    // the upload is what `goalcup status` tells from here on.
    // Not written over a next segment's: its host may have started while this one sent.
    keeper.update_while_current(|state| {
        state.ended = true;
        if let Some(report) = &upload_report {
            state.upload.stopped = false;
            state.upload.problem = if report.caught_up && report.sealed {
                None
            } else {
                Some(
                    report
                        .why
                        .clone()
                        .unwrap_or_else(|| "the seal was not sent".to_string()),
                )
            };
        }
    });
    // The reserve is released when the run has ended (its deadline, a stop for its end, or the
    // server's word that it is sealed or incomplete), whatever became of its upload; or when the
    // run goes on but `goalcup continue` cannot restart it and all of it is on the server. A run
    // `continue` can restart keeps it. A reserve already handed over (the lock dropped, the
    // file kept) for a run the server now calls over is removed by the same rule.
    let release = over || server_ended || (!run_goes_on && sent_whole);
    match reserve {
        Some(reserve) if release => release_and_say(reserve),
        Some(_) => {}
        None if server_ended => {
            crate::run::reserve::release_if_done(
                cfg,
                &artifact_root(cfg),
                &record.run_id,
                true,
                crate::run::now_unix(),
            );
        }
        None => {}
    }
    // Last of the wrap-up: the chain parts the server acknowledged of every entry that is sealed
    // and sent go from this machine (the last part, the seal and the acknowledgements stay).
    crate::upload::prune::clean_finished_runs(cfg);
    finished?;
    if let Some(refusal) = left_running {
        return Err(refusal);
    }
    Ok(StartOutcome {
        entry_id: info.entry_id.clone(),
        exit_status: Some(status?),
    })
}

/// A harness's end in words: its exit code, or the signal that ended it.
pub(super) fn describe_exit(status: &ExitStatus) -> String {
    if let Some(code) = status.code() {
        return format!("exit code {code}");
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return format!("ended by signal {signal}");
        }
    }
    status.to_string()
}

/// Seal an entry whose harness never ran (or was never told about), within the one budget.
pub(super) fn finish_unlaunched(
    client: &(dyn DaemonClient + Sync),
    entry_id: &str,
    proxied: Proxied,
    out: &mut dyn std::io::Write,
) {
    let deadline = std::time::Instant::now() + crate::finish_budget();
    let _ = finish(
        client,
        entry_id,
        proxied,
        deadline,
        WrapUp::one_attempt(),
        termination_bound(0),
        false,
        None,
        None,
        out,
    );
}

/// How long the recording proxy's stop waits for connections to end on their own when the run is
/// stopped while the harness still runs (its deadline, the server's stop, a full disk, a
/// replacement). They do not end then: a model's streaming answer or a kept-alive WebSocket stays
/// open as long as the harness does. So the stop cuts them almost at once (each open stream
/// recorded `Interrupted`), and the seal, which ends the harness, follows within seconds instead of
/// after the whole drain share. The same bound applies when the harness exited but its connector's
/// after-exit commands (D72) failed, so what it left running may hold its connections open
/// (`finish`'s `helper_left`).
pub(super) const RUNNING_HARNESS_PROXY_DRAIN: Duration = Duration::from_secs(2);

/// How long the sender may take to end its connection, before the seal, when the harness still
/// runs and the recording proxy could not queue its producer's close. Without it the daemon's
/// ingest drain would wait for a producer still live until its drain share ends; a closed
/// connection is a settled (lost) producer, an honest tail-loss gap, and the drain returns.
const RUNNING_HARNESS_SINK_SHUTDOWN: Duration = Duration::from_secs(1);

/// Stop the recording proxy, seal with its counters, then let the sender drain, and say how the run
/// ended, all within `deadline` (the one local-finish budget, taken by the caller immediately
/// before this call); nothing here waits on the harness.
///
/// A seal that did not take (the daemon busy, restarting, or past its budget) is asked again with
/// growing waits until `wrap_up` has run out ([`crate::retry::retry_capture_seal_until`]): the
/// recording proxy is stopped once, and each later ask has a whole local-finish budget of its own.
/// A `WrapUp::one_attempt()` asks once.
///
/// The one deadline of a capture run's finish is `finish_budget()` (`LOCAL_CAPTURE_FINISH_BUDGET`),
/// taken when the harness has exited and immediately before the recording proxy's stop: nothing
/// runs between the two. The recording proxy's stop gets its drain share (the budget less
/// `FINISH_COMMIT_RESERVE` and `GIT_READ_FLOOR`), the daemon's seal its own request's remainder,
/// and the sender's drain what is left of the same deadline (CONTRACTS.md, "C6 local finish
/// budget (H-10 4a)").
///
/// `harness_running` is true when the run is stopped while the harness still runs (its
/// deadline, the server's stop, a full disk, a replacement): the recording proxy's stop then waits
/// [`RUNNING_HARNESS_PROXY_DRAIN`] at most, because the harness's open connections would not end
/// before the seal ends the harness. If the recording proxy could not queue its producer's close,
/// the sender is ended too before the seal ([`RUNNING_HARNESS_SINK_SHUTDOWN`]).
///
/// Once the recorder is stopped, a run that recorded model calls says so in one line, with the
/// hosts those calls went to as the recorder observed them ([`recording_works_line`]). A run whose
/// harness never ran records no calls, so it says nothing of the recording.
///
/// `helper_left` names the harness when it exited but its connector's after-exit commands (D72)
/// failed: what it left running (Hermes's gateway) may hold the proxy's connections open, so the
/// stop is bounded by [`RUNNING_HARNESS_PROXY_DRAIN`] too, and the participant is told in one line
/// only if the stop actually cut a connection. The sender is not cut early then: the harness has
/// exited and the seal does not wait on it.
#[allow(clippy::too_many_arguments)]
pub(super) fn finish(
    client: &(dyn DaemonClient + Sync),
    entry_id: &str,
    proxied: Proxied,
    deadline: std::time::Instant,
    wrap_up: WrapUp,
    termination: Duration,
    harness_running: bool,
    helper_left: Option<&str>,
    reconnect: Option<&Path>,
    out: &mut dyn std::io::Write,
) -> Result<CaptureFinishReport, CliError> {
    let Proxied {
        proxy,
        sink,
        runtime,
        tunnels_file,
        ..
    } = proxied;
    // The recorder's count is read once it is stopped: the stop may settle a call still in flight.
    let model_calls = proxy.generation_requests();
    // Read here as well, while the handle is still whole: the stop consumes it, and the hosts are
    // read once the recorder is stopped, from this shared set.
    let generation_hosts = proxy.generation_hosts_shared();
    // A harness that still runs keeps its connections open: they would hold the stop, and the
    // seal that ends the harness, for the whole drain share. They are cut.
    let stopped = if harness_running || helper_left.is_some() {
        runtime.block_on(proxy.stop_within(RUNNING_HARNESS_PROXY_DRAIN))
    } else {
        runtime.block_on(proxy.stop())
    };
    // Said once the stop is done, and only when it cut something: a failed stop with nothing
    // left open (Hermes 0.19.0 has no `-y` on `gateway stop`) cut nothing, and says nothing.
    if let Some(harness) = helper_left
        && !stopped.drained
    {
        let _ = say(out, &crate::participant::helper_not_stopped(harness));
    }
    pause_at("capture-finish-proxy-stopped");
    // A producer the recording proxy did not close stays live in the daemon's ingest drain until
    // its share ends. With the harness still running, the sender's connection is ended now instead
    // (the shutdown cut by the bound aborts it), so the seal is not held. The later shutdown below
    // then reports from what it finds.
    if harness_running && stopped.producer_close != crate::proxy::ProducerClose::Queued {
        runtime.block_on(async {
            let _ = tokio::time::timeout(RUNNING_HARNESS_SINK_SHUTDOWN, sink.shutdown()).await;
        });
    }
    let h = &stopped.health;
    let a = &h.admission_failures;
    // Diagnostics are fixed counters only. A failed stdout write must not prevent sealing.
    let _ = say(out, &proxy_admission_line(h, a));
    // The recording, in one line, when the recorder saw model calls. None is said here: the end of
    // `goalcup start` says so in its no-model-calls block.
    let calls = model_calls.load(std::sync::atomic::Ordering::SeqCst);
    if calls > 0 {
        let hosts = observed_hosts(&generation_hosts);
        let _ = say(out, &recording_works_line(calls, &hosts));
    }
    for line in crate::proxy::outage::finish_lines(h.upstream_outages, &h.outage_windows) {
        let _ = say(out, &line);
    }
    // The sealed-connection list (D74), written before the seal so the upload that follows it
    // finds the file. A list that cannot be written costs only the list: the seal goes on.
    if let Some(line) = tunnels::summary_line(entry_id, &h.tunnels, h.tunnels_dropped) {
        let _ = say(out, &line);
    }
    if let Err(e) = tunnels::write(&tunnels_file, entry_id, &h.tunnels, h.tunnels_dropped) {
        crate::host_log::log_only(&format!("the sealed-connection list was not written: {e}"));
    }
    let counters = ProxyCounters {
        tunneled_fail_open: h.tunneled_fail_open,
        opaque_fallbacks: h.opaque_fallbacks,
        capture_table_full: h.capture_table_full,
        cut_unsettled: h.cut_unsettled,
        unterminated: stopped.unterminated as u64,
        sink_refusals: h.sink_refusals,
    };
    // After the harness exited, the launcher's own root is reaped and the daemon's termination
    // answers `AlreadyStopped` after one `ps`: `termination_bound(0)`. At a deadline the harness
    // is still running, and the bound gives it its grace.
    let sealed = crate::commands::seal::seal_locally_until(
        client,
        entry_id,
        Some(counters),
        deadline,
        wrap_up,
        termination,
        reconnect,
    );
    // A committed seal stays committed: the report is this command's own output, and its
    // failing (a closed pipe, a quit pager) ends nothing. It is said on stderr, if that is
    // still there, so the trace's path is not lost with it.
    if let Ok(report) = &sealed
        && let Err(e) = crate::commands::seal::report_local_finish(entry_id, report, out)
    {
        crate::host_log::say(&format!(
            "entry {entry_id} is sealed, but its report could not be written to stdout ({e}). \
             The trace is {}.",
            report.trace_dir
        ));
    }
    // Cut at the deadline, the sender ends its connection with the rest unwritten, as its own
    // bound would. When the seal returned after the deadline, it is cut at once.
    runtime.block_on(async {
        let _ = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), sink.shutdown())
            .await;
    });
    sealed
}

/// The line that says the recording works, said at the finish of a run whose recorder saw
/// `calls` model calls, from `hosts` (the hosts those calls went to, as observed). Without a host
/// it names no host.
pub(super) fn recording_works_line(calls: u64, hosts: &[String]) -> String {
    if hosts.is_empty() {
        format!("Recording works: {calls} model call(s) recorded.")
    } else {
        format!(
            "Recording works: {calls} model call(s) recorded from {}.",
            hosts.join(", ")
        )
    }
}

/// The hosts the recorder observed the counted model calls go to, in order. A set that cannot be
/// read gives none, and the line then names no host rather than a guess.
fn observed_hosts(hosts: &std::sync::Mutex<std::collections::BTreeSet<String>>) -> Vec<String> {
    hosts
        .lock()
        .map(|hosts| hosts.iter().cloned().collect())
        .unwrap_or_default()
}

/// The lines that say, at the end of `goalcup start`, that no model call was recorded in this
/// part of the run, in the past tense (the harness has exited): none when `recorded` says a model
/// call was. `silent` is the watch tick's last verdict when the tick called the harness silent,
/// else how long the harness ran and whether its connections failed the TLS handshake.
fn no_model_calls_block(
    silent: NoModelCalls,
    recorded: bool,
    harness: &str,
    restart: crate::participant::Restart<'_>,
) -> Vec<String> {
    if recorded {
        return Vec::new();
    }
    let line = crate::participant::problem_no_model_calls(
        harness,
        silent.minutes,
        silent.tls_failing,
        restart,
        false,
    );
    vec![
        String::new(),
        format!("{} {line}", crate::participant::PROBLEM),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::participant::{NO_MODEL_CALLS_MARK, Restart};
    use crate::run::{NoModelCalls, RunState};

    /// The end-of-run block says no model call was recorded whether or not the watch tick called
    /// the harness silent, in the past tense (nothing to type, nothing to exit). It says nothing
    /// when a model call was recorded in this part of the run.
    #[test]
    fn the_end_of_run_says_no_model_calls_whether_or_not_the_tick_saw_it() {
        let run_id = "r0123456789abcdef";
        let unflagged = NoModelCalls {
            minutes: 0,
            tls_failing: false,
        };
        let lines =
            no_model_calls_block(unflagged, false, "Some Agent", Restart::Continue { run_id });
        let line = lines.last().expect("a line");
        assert!(line.starts_with("PROBLEM:"), "{lines:?}");
        assert!(line.contains(NO_MODEL_CALLS_MARK), "{lines:?}");
        assert!(line.contains("less than a minute"), "{lines:?}");
        assert!(
            line.contains(&format!("goalcup continue {run_id}")),
            "{lines:?}"
        );
        assert!(!line.contains("type it"), "{lines:?}");
        assert!(!line.contains("Exit"), "{lines:?}");

        let mut state = RunState::launched("e1", Ok(()));
        state.no_model_calls = Some(NoModelCalls {
            minutes: 4,
            tls_failing: true,
        });
        let silent = state.no_model_calls.expect("the tick's verdict");
        let line = no_model_calls_block(silent, false, "Some Agent", Restart::Over)
            .pop()
            .expect("a line");
        assert!(line.contains("4 minutes"), "{line}");
        assert!(line.contains("does not trust"), "{line}");
        assert!(
            no_model_calls_block(silent, true, "Some Agent", Restart::Over).is_empty(),
            "a call recorded in this part of the run says nothing of silence"
        );
    }

    /// The recording's one line names the count and the hosts the calls were recorded from.
    #[test]
    fn the_recording_line_names_the_calls_and_their_hosts() {
        assert_eq!(
            recording_works_line(2, &["models.example.test".to_string()]),
            "Recording works: 2 model call(s) recorded from models.example.test."
        );
        let two = ["chatgpt.com".to_string(), "api.openai.com".to_string()];
        assert_eq!(
            recording_works_line(1, &two),
            "Recording works: 1 model call(s) recorded from chatgpt.com, api.openai.com."
        );
        assert_eq!(
            recording_works_line(3, &[]),
            "Recording works: 3 model call(s) recorded."
        );
    }
}
