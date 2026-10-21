//! Phase five and the seal: the recording proxy stopped, the entry sealed within the one finish
//! budget, and what became of the run said.

use super::forked::{Exited, Forked, release_and_say};
use super::proxy::{Proxied, proxy_admission_line};
use super::{StartOutcome, pause_at};
use crate::CliError;
use crate::commands::say;
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
        uploading,
        wrap_up,
        left_running,
        keeper,
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
        ..
    } = forked;
    // A run's hold on the terminal (its hangup absorbed, its log open) lasts to the end
    // of this function: the seal and the upload's wrap-up below must outlive a closed window.
    let _chat = chat;
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
            Some(cfg),
            out,
        ),
        (None, None) => Err(CliError::Internal(
            "the run's proxy was stopped and nothing finished the run".to_string(),
        )),
    };
    // The run is over when it was stopped for its end (its deadline, or a newer run replacing
    // it; a full disk stops the agent and leaves the run open), when its time is up by the count
    // of the server's last answer (never this machine's date), and always for a run with no
    // server. Otherwise the agent's exit
    // ended this segment only: the run stays open on the server until its deadline and
    // `goalcup continue` restarts it.
    let snapshot = keeper.snapshot();
    let ended_by_stop = snapshot.stopped_for_its_end();
    let on_server = record.server_url.is_some();
    let over = !on_server || crate::run::is_over(&record, Some(&snapshot), crate::run::now_unix());
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
            disk_stop: stopped == Some(crate::run::watch::DISK_STOP_REASON),
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
            format!("uploaded: entry {entry}, an earlier segment of this run, acknowledged.")
        } else {
            format!(
                "upload of entry {entry}, an earlier segment, is incomplete ({}). {how_to_finish}",
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
        out,
    );
}

/// How long the recording proxy's stop waits for connections to end on their own when the run is
/// stopped while the harness still runs (its deadline, the server's stop, a full disk, a
/// replacement). They do not end then: a model's streaming answer or a kept-alive WebSocket stays
/// open as long as the harness does. So the stop cuts them almost at once (each open stream
/// recorded `Interrupted`), and the seal, which ends the harness, follows within seconds instead of
/// after the whole drain share.
const RUNNING_HARNESS_PROXY_DRAIN: Duration = Duration::from_secs(2);

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
#[allow(clippy::too_many_arguments)]
pub(super) fn finish(
    client: &(dyn DaemonClient + Sync),
    entry_id: &str,
    proxied: Proxied,
    deadline: std::time::Instant,
    wrap_up: WrapUp,
    termination: Duration,
    harness_running: bool,
    reconnect: Option<&Path>,
    out: &mut dyn std::io::Write,
) -> Result<CaptureFinishReport, CliError> {
    let Proxied {
        proxy,
        sink,
        runtime,
        ..
    } = proxied;
    // A harness that still runs keeps its connections open: they would hold the stop, and the
    // seal that ends the harness, for the whole drain share. They are cut.
    let stopped = if harness_running {
        runtime.block_on(proxy.stop_within(RUNNING_HARNESS_PROXY_DRAIN))
    } else {
        runtime.block_on(proxy.stop())
    };
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
    for line in crate::proxy::outage::finish_lines(h.upstream_outages, &h.outage_windows) {
        let _ = say(out, &line);
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
