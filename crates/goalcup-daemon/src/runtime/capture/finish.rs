//! The local finish: its budget, stages, refusals and the latch an expired finish leaves, and the
//! owner stop it waits on.

use super::*;

/// No refusable finish stage begins with less than this left of the deadline. It covers the
/// last commit step (append `LocalFinish`, sync, `seal.json`, registry row), dominated by the
/// existing 1 s sync bound, and it doubles as the margin that lets the answer reach the CLI
/// before its transport timeout. It is a part of [`FINISH_COMMIT_RESERVE`], not another budget.
pub(super) const FINISH_RESPONSE_MARGIN: Duration = Duration::from_secs(1);

/// The finish reason a local capture finish records on its `LocalFinish` record. A local
/// capture has no signed deadline, so a finish is always one the participant's seal asked for.
pub(super) const LOCAL_FINISH_REASON: &str = "manual";

/// The stages of the v5 local finish that can wait or be refused, in order. A refusal names the
/// stage it stopped before; the token is the wire message of `finish_deadline`. The numbers are
/// those of CONTRACTS.md, C6 local finish budget amendment, ¶4. Stage 1 (drain ingest, which
/// expires as a cut with gaps) and stage 5 (carried gaps, which are appended and not waited on)
/// are never refused, so they have no variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FinishStage {
    /// Stage 0, a zero budget: refused before any finish step, admission still open.
    NotStarted,
    /// Stage 2: the blocking permit.
    BlockingPermit,
    /// Stage 3: the entry's run lock. Another finish owns the entry, so a refusal here latches
    /// nothing.
    RunLock,
    /// Stage 4: the owner stop.
    OwnerDrain,
    /// Stage 6: ending the registered process tree.
    Termination,
    /// Stage 7: the workspace walk into the seal manifest.
    WorkspaceWalk,
    /// Stage 8: `LocalFinish`, sync, `seal.json` and the registry row.
    LocalFinish,
}

impl FinishStage {
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) const ALL: [FinishStage; 7] = [
        FinishStage::NotStarted,
        FinishStage::BlockingPermit,
        FinishStage::RunLock,
        FinishStage::OwnerDrain,
        FinishStage::Termination,
        FinishStage::WorkspaceWalk,
        FinishStage::LocalFinish,
    ];

    /// The stage whose wire token is `token`; test seams name a stage by it.
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) fn from_token(token: &str) -> Option<FinishStage> {
        Self::ALL.into_iter().find(|stage| stage.token() == token)
    }

    pub(super) fn token(self) -> &'static str {
        match self {
            FinishStage::NotStarted => "not_started",
            FinishStage::BlockingPermit => "blocking_permit",
            FinishStage::RunLock => "run_lock",
            FinishStage::OwnerDrain => "owner_drain",
            FinishStage::Termination => "termination",
            FinishStage::WorkspaceWalk => "workspace_walk",
            FinishStage::LocalFinish => "local_finish",
        }
    }
}

/// One finish's share of the one local-finish budget, fixed when the daemon decodes the
/// request: the end of its drain share and its deadline. Termination time is added back at each
/// later stage (`excluded`), because it keeps TERMINATION.md's bound and is never cut.
#[derive(Debug, Clone, Copy)]
pub(super) struct FinishBudget {
    pub(super) drain_end: tokio::time::Instant,
    pub(super) deadline: tokio::time::Instant,
}

impl FinishBudget {
    /// `NotStarted` for a zero remainder. The drain share ends [`FINISH_COMMIT_RESERVE`]
    /// before the deadline, or now when less than the reserve remains.
    pub(super) fn from_request(request: &CaptureSealRequest) -> Result<FinishBudget, DaemonError> {
        let remaining = request.remaining_finish();
        if remaining.is_zero() {
            return Err(refused_at(FinishStage::NotStarted));
        }
        let now = tokio::time::Instant::now();
        let deadline = now + remaining;
        let drain_end = deadline
            .checked_sub(FINISH_COMMIT_RESERVE)
            .map_or(now, |end| end.max(now));
        Ok(FinishBudget {
            drain_end,
            deadline,
        })
    }

    /// The last instant a refusable stage may begin, given `excluded` termination time.
    pub(super) fn last_start(&self, excluded: Duration) -> tokio::time::Instant {
        let end = self.deadline + excluded;
        end.checked_sub(FINISH_RESPONSE_MARGIN)
            .unwrap_or_else(tokio::time::Instant::now)
    }

    /// Whether a refusable stage may begin now: more than [`FINISH_RESPONSE_MARGIN`] remains.
    pub(super) fn admits(&self, excluded: Duration) -> bool {
        tokio::time::Instant::now() < self.last_start(excluded)
    }

    /// What the native adapter may still spend: the rest of the drain share.
    pub(super) fn native_budget(&self) -> Duration {
        self.drain_end
            .saturating_duration_since(tokio::time::Instant::now())
    }
}

/// Call `attempt` until it answers `Some`, sleeping up to [`IDLE_WAIT`] between tries, and give
/// up with `None` once `deadline` has passed. Every finish deadline is a `tokio::time::Instant`,
/// the clock [`FinishBudget`] keeps.
pub(super) fn poll_until<T>(
    deadline: tokio::time::Instant,
    mut attempt: impl FnMut() -> Option<T>,
) -> Option<T> {
    loop {
        if let Some(done) = attempt() {
            return Some(done);
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return None;
        }
        std::thread::sleep(IDLE_WAIT.min(deadline - now));
    }
}

pub(super) fn refused_at(stage: FinishStage) -> DaemonError {
    DaemonError::FinishDeadline {
        stage: stage.token(),
    }
}

/// What an expired finish left for the next one to chain. Set by a refusal at stage 2, 4, 6, 7
/// or 8 (never 3: another finish owns the entry; see [`FinishStage`]), merged with what an earlier refusal left;
/// taken, and appended, only by a finish that reaches the proxy-counter stage; never cleared by
/// a refusal. Across a restart it is lost, and reopen's own stream-less `IngestLost` gap keeps a
/// registered or launched run incomplete.
#[derive(Debug)]
pub(super) struct ExpiredFinish {
    /// The launcher's proxy counters, when the expired attempt had them and had not chained them.
    pub(super) proxy_pending: Option<ProxyCounters>,
    /// The outcome an expired attempt had already bound at stage 6. A later finish reuses it
    /// only if it is `Confirmed`; see [`bind_termination`].
    pub(super) termination: Option<TerminationOutcomeV1>,
}

/// Whether a recorded outcome came from an attempt that signalled the harness: a seal that
/// finds the process gone afterwards must not read as "ended on its own".
pub(super) fn signalled(outcome: &TerminationOutcomeV1) -> bool {
    matches!(
        outcome,
        TerminationOutcomeV1::Unresolved {
            reason: TerminationFailureV1::ProcessStillRunning | TerminationFailureV1::RequestFailed,
            ..
        }
    )
}

/// What a finish binds at the termination stage, given what an expired attempt recorded
/// (CONTRACTS.md, H-10 4a ¶5). A recorded `Confirmed` is reused and `terminate` is not called:
/// the harness was ended and observed gone. Any other recorded outcome (parked, refused,
/// unresolved) may describe a harness that is running now, so the finish terminates again and
/// binds the new outcome, except that a new `AlreadyStopped` after an attempt that signalled
/// keeps the recorded outcome. Returns the outcome and whether `terminate` ran.
pub(super) fn bind_termination<E>(
    recorded: Option<TerminationOutcomeV1>,
    terminate: impl FnOnce() -> Result<TerminationOutcomeV1, E>,
) -> Result<(TerminationOutcomeV1, bool), E> {
    match recorded {
        Some(done @ TerminationOutcomeV1::Confirmed { .. }) => Ok((done, false)),
        recorded => {
            let fresh = terminate()?;
            Ok(match (recorded, fresh) {
                (Some(earlier), TerminationOutcomeV1::AlreadyStopped { .. })
                    if signalled(&earlier) =>
                {
                    (earlier, true)
                }
                (_, fresh) => (fresh, true),
            })
        }
    }
}

/// Armed for the async part of a finish, which axum drops when the caller's connection closes.
/// A dropped attempt has set `finishing` and closed admission, and the launcher's proxy counters
/// it carried exist nowhere else, so while armed its drop latches them exactly as a stage-2
/// refusal does: the next finish chains them and one stream-less `ShutdownDeadline` gap. The
/// blocking part, once spawned, always runs to completion and latches through its own paths, so
/// the guard is disarmed just before it starts. Raw facts only: "an attempt began and its caller
/// left" and the counters (D3); `expire` takes only `capture.expired`, a std mutex.
pub(super) struct AbandonGuard {
    capture: Arc<CaptureEntry>,
    proxy: Option<ProxyCounters>,
    armed: bool,
}

impl AbandonGuard {
    /// The attempt no longer needs the latch: the caller is handing the rest of the finish to a
    /// path that latches for itself, or has just latched.
    pub(super) fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for AbandonGuard {
    fn drop(&mut self) {
        if self.armed {
            self.capture
                .expire(FinishStage::BlockingPermit, self.proxy, None);
        }
    }
}

/// How the owner stop ended.
pub(super) enum OwnerStop {
    Exited,
    StillRunning,
}

impl Daemon {
    /// Finish a v5 entry locally, once. A retry answers the committed finish again.
    ///
    /// The whole finish runs inside the one budget the request grants (CONTRACTS.md, H-10 4a):
    /// the drain share ends [`FINISH_COMMIT_RESERVE`] before its deadline, and a stage that
    /// waits (the blocking permit, the entry's lock, the owner) or that would begin inside
    /// [`FINISH_RESPONSE_MARGIN`] of the deadline is refused with `finish_deadline`, writing no
    /// terminal. The harness's termination keeps its own bound and is outside the budget.
    pub(crate) async fn finish_capture(
        &self,
        entry_id: &str,
        request: &CaptureSealRequest,
    ) -> Result<CaptureFinishReport, DaemonError> {
        let open = self.captures().get(entry_id).cloned();
        let Some(capture) = open else {
            // A sealed entry's finish is on its row, which is written before the entry leaves
            // the map.
            return self
                .sealed_capture(entry_id)?
                .and_then(|(row, _)| row.capture?.finished)
                .ok_or_else(|| {
                    DaemonError::Missing(format!("no capture entry {entry_id} in this daemon"))
                });
        };
        // Stage 0: a zero budget is refused before anything is closed or flagged.
        let budget = FinishBudget::from_request(request)?;
        capture.finishing.store(true, Ordering::Release);
        let abandoned = AbandonGuard {
            capture: capture.clone(),
            proxy: request.proxy,
            armed: true,
        };
        // Stage 1: close admission and drain ingest until the end of the drain share. Expiry is
        // the existing cut with `ShutdownDeadline` gaps, never a refusal.
        if let Some(ingest) = &capture.ingest {
            ingest.close_admission();
            ingest.drain(budget.drain_end).await;
        }
        // Stage 2: the blocking permit.
        let permit = if budget.admits(Duration::ZERO) {
            tokio::time::timeout_at(
                budget.last_start(Duration::ZERO),
                self.inner.blocking.clone().acquire_owned(),
            )
            .await
            .ok()
        } else {
            None
        };
        let Some(permit) = permit else {
            // A refusal that latches below; the guard must not latch it twice.
            abandoned.disarm();
            return Err(capture.expire(FinishStage::BlockingPermit, request.proxy, None));
        };
        // From here the blocking task runs to completion even if this future is dropped.
        abandoned.disarm();
        let daemon = self.clone_handle();
        let request = request.clone();
        let entry_id = entry_id.to_string();
        tokio::task::spawn_blocking(move || {
            let finished = daemon.finish_capture_blocking(&entry_id, &capture, &request, budget);
            drop(permit);
            finished
        })
        .await
        .map_err(|error| {
            DaemonError::Refused(format!("the local finish did not complete: {error}"))
        })?
    }

    /// The committed finish of `entry_id`, read from its registry row: another finish holds the
    /// entry's lock and may have just written it.
    pub(super) fn committed_finish(&self, entry_id: &str) -> Option<CaptureFinishReport> {
        self.registry().get(entry_id).ok()??.capture?.finished
    }

    pub(super) fn finish_capture_blocking(
        &self,
        entry_id: &str,
        capture: &CaptureEntry,
        request: &CaptureSealRequest,
        budget: FinishBudget,
    ) -> Result<CaptureFinishReport, DaemonError> {
        // Stage 3: the entry's run lock, polled until the last instant a stage may begin. Another
        // finish holds it while it commits, and a committed finish is answered as committed.
        capture.pause_before(FinishStage::RunLock);
        let until = if budget.admits(Duration::ZERO) {
            budget.last_start(Duration::ZERO)
        } else {
            tokio::time::Instant::now()
        };
        let Some(run) = capture.lock_until(until) else {
            return match self.committed_finish(entry_id) {
                Some(done) => Ok(done),
                // Another finish owns the entry: this attempt latches nothing.
                None => Err(capture.expire(FinishStage::RunLock, None, None)),
            };
        };
        if let Some(done) = run.finished() {
            return Ok(done.clone());
        }
        let entry_id = run.entry.entry_id.clone();
        // The terminal is already chained (its registry write failed, or a daemon died between
        // the two): commit it, and append nothing after it.
        let chained = lock(&capture.chained_finish).clone();
        if let Some(chained) = chained {
            let report = CaptureFinishReport {
                entry_id: entry_id.clone(),
                trace_dir: run.entry.trace_dir.display().to_string(),
                termination: chained.termination,
                terminal_seq: Some(chained.seq),
                seal: if chained.seal_written {
                    LocalSealOutcome::SealWritten
                } else {
                    LocalSealOutcome::SealWriteFailed
                },
            };
            return self.commit_finish(capture, run, report, chained.sealed_at);
        }

        // Stage 4: stop the owner. Native finish gets the rest of the drain share; the owner's
        // exit is awaited until the last instant a stage may begin, and is never joined while it
        // runs. A slow owner is a refusal; its thread stays with the entry for the next finish.
        capture.pause_before(FinishStage::OwnerDrain);
        // A deferred native scope rides on this request. It is installed here, under the run
        // lock and before the owner is stopped, so the owner finishes it within the native
        // budget below (CONTRACTS.md, C6 local finish budget amendment, ¶9).
        capture.install_deferred_scope(request.native_scope.as_deref(), run.launched());
        let stopped = budget.admits(Duration::ZERO)
            && matches!(
                capture.stop_owner(
                    budget.native_budget(),
                    Some(budget.last_start(Duration::ZERO)),
                ),
                OwnerStop::Exited
            );
        if !stopped {
            return Err(capture.expire(FinishStage::OwnerDrain, request.proxy, None));
        }

        // Stage 5: the writer is uncontended now.
        let recorded = capture.chain_carried_gaps(request, run.launched());

        // Stage 6: end the registered process tree, outside the budget.
        capture.pause_before(FinishStage::Termination);
        if !budget.admits(Duration::ZERO) {
            return Err(capture.expire(FinishStage::Termination, None, recorded));
        }
        let row = self
            .registry()
            .get(&entry_id)?
            .ok_or_else(|| DaemonError::Missing(format!("entry {entry_id}")))?;
        let requested_at = self.finalization_timestamp("termination request")?;
        let began = std::time::Instant::now();
        let (termination, ran) = bind_termination(recorded, || {
            let (outcome, _) = match &row.last_process {
                // No signed walk budget binds a local capture: the module's own ceiling does.
                Some(handle) => super::super::termination::terminate_handle(
                    self,
                    handle,
                    usize::MAX,
                    &requested_at,
                )?,
                None => super::super::termination::unresolved_termination(
                    self,
                    &requested_at,
                    TerminationFailureV1::ProcessAuthorityUnavailable,
                    String::new(),
                )?,
            };
            Ok::<_, DaemonError>(outcome)
        })?;
        // The time this attempt spent ending the harness is added back to the deadline.
        let excluded = if ran { began.elapsed() } else { Duration::ZERO };

        // Stage 7: the workspace walk.
        capture.pause_before(FinishStage::WorkspaceWalk);
        if !budget.admits(excluded) {
            return Err(capture.expire(FinishStage::WorkspaceWalk, None, Some(termination)));
        }
        let sealed_at = now_rfc3339();
        let manifest = match workspace::list_workspace(&row.workspace, &entry_id, &sealed_at, &[]) {
            Ok(manifest) => manifest,
            Err(error) => {
                // The harness was ended above; a later finish must not read that as "ended on
                // its own".
                capture.keep_termination(termination);
                return Err(error);
            }
        };

        // Stage 8: the terminal. Once it begins it completes under the existing no-retry rule;
        // only its registry half (`commit_finish`) is retried as it stands.
        capture.pause_before(FinishStage::LocalFinish);
        if !budget.admits(excluded) {
            return Err(capture.expire(FinishStage::LocalFinish, None, Some(termination)));
        }
        let mut writer = lock(&capture.writer);
        let before = writer.export.head().map(|(seq, _)| seq);
        let written = writer.export.write_local_finish(
            &manifest,
            (capture.clock)(),
            LOCAL_FINISH_REASON,
            termination.clone(),
        );
        // A failed terminal write is reported as it stands, here and in every later status:
        // no retry and no further sync.
        let (seal, terminal_seq) = match written {
            Ok((_, link)) => {
                *lock(&capture.chained_finish) = Some(super::ChainedFinish {
                    seq: link.seq,
                    termination: termination.clone(),
                    sealed_at: sealed_at.clone(),
                    seal_written: true,
                });
                (LocalSealOutcome::SealWritten, Some(link.seq))
            }
            Err(_) => {
                let terminal_seq = writer
                    .export
                    .head()
                    .map(|(seq, _)| seq)
                    .filter(|seq| Some(*seq) != before);
                // A terminal chained without its `seal.json`: a retry commits the failure and
                // appends nothing after it.
                if let Some(seq) = terminal_seq {
                    *lock(&capture.chained_finish) = Some(super::ChainedFinish {
                        seq,
                        termination: termination.clone(),
                        sealed_at: sealed_at.clone(),
                        seal_written: false,
                    });
                }
                (LocalSealOutcome::SealWriteFailed, terminal_seq)
            }
        };
        drop(writer);
        let report = CaptureFinishReport {
            entry_id: entry_id.clone(),
            trace_dir: row.trace_dir.display().to_string(),
            termination,
            terminal_seq,
            seal,
        };
        self.commit_finish(capture, run, report, sealed_at)
    }

    /// The registry half of a finish: the row records the report, and a sealed entry leaves
    /// the capture map. Retried as it stands by the next finish when it fails.
    fn commit_finish(
        &self,
        capture: &CaptureEntry,
        mut run: MutexGuard<'_, CaptureRun>,
        report: CaptureFinishReport,
        sealed_at: String,
    ) -> Result<CaptureFinishReport, DaemonError> {
        let entry_id = report.entry_id.clone();
        let sealed_status = (report.seal == LocalSealOutcome::SealWritten)
            .then(|| capture.snapshot(Some(&report), run.launched()));
        run.entry = self.registry().update(&entry_id, |entry| {
            entry.sealed_at = Some(sealed_at);
            if let Some(capture) = entry.capture.as_mut() {
                capture.finished = Some(report.clone());
                capture.sealed_status = sealed_status.clone();
            }
        })?;
        if sealed_status.is_some() {
            // Its row now answers for it: the entry's ingest grant and chain descriptors go
            // with the last reference to it.
            drop(run);
            self.captures().remove(&entry_id);
        }
        Ok(report)
    }

    /// Test seam: occupy every blocking permit until the returned permit drops.
    #[cfg(any(test, feature = "test-hooks"))]
    pub async fn hold_blocking_permits(&self) -> tokio::sync::OwnedSemaphorePermit {
        let all = u32::try_from(super::super::daemon::MAX_IN_FLIGHT).unwrap_or(u32::MAX);
        self.inner
            .blocking
            .clone()
            .acquire_many_owned(all)
            .await
            .expect("the blocking semaphore stays open")
    }

    /// Test seam: park (or release) the entry's owner thread between its stop request and its
    /// drain.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn hold_capture_owner_drain(&self, entry_id: &str, hold: bool) {
        if let Some(capture) = self.captures().get(entry_id) {
            capture.owner.hold_drain.store(hold, Ordering::Release);
        }
    }

    /// Test seam: pause `delay` immediately before finish stage `stage` (its token) checks its
    /// admission, so a test can spend the budget deterministically.
    #[cfg(any(test, feature = "test-hooks"))]
    ///
    /// Panics on a token no stage has and on an entry that is not open: a seam that silently does
    /// nothing makes the test pass or fail for the wrong reason.
    pub fn delay_capture_finish_stage(&self, entry_id: &str, stage: &str, delay: Duration) {
        let stage =
            FinishStage::from_token(stage).unwrap_or_else(|| panic!("no finish stage {stage:?}"));
        let captures = self.captures();
        let capture = captures
            .get(entry_id)
            .unwrap_or_else(|| panic!("no open capture entry {entry_id}"));
        *lock(&capture.stage_delay) = Some((stage, delay));
    }

    /// Test seam: whether any finish attempt has begun on this open entry.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn capture_finishing(&self, entry_id: &str) -> Option<bool> {
        self.captures()
            .get(entry_id)
            .map(|capture| capture.finishing.load(Ordering::Acquire))
    }
}

impl CaptureEntry {
    /// Stop the owner: native finish within `native_budget`, remaining producers cut, the queue
    /// drained, flushed and synced.
    ///
    /// With `wait_until` the wait is bounded: the owner's exit is polled every [`IDLE_WAIT`] and
    /// the thread is joined only once it has exited, so a stuck owner is [`OwnerStop::StillRunning`]
    /// and its `JoinHandle` stays here for the next finish. Without it (the entry is being
    /// dropped) the join is unbounded, as it always was: the owner's post-stop work is bounded by
    /// the 8 MiB queue, and a held syscall there is not new.
    pub(super) fn stop_owner(
        &self,
        native_budget: Duration,
        wait_until: Option<tokio::time::Instant>,
    ) -> OwnerStop {
        let budget = u64::try_from(native_budget.as_nanos()).unwrap_or(u64::MAX);
        self.owner.finish_budget_ns.store(budget, Ordering::Release);
        self.owner.stop.store(true, Ordering::Release);
        if let Some(wait_until) = wait_until {
            let gone = poll_until(wait_until, || {
                (self.owner.exited.load(Ordering::Acquire) || lock(&self.thread).is_none())
                    .then_some(())
            });
            if gone.is_none() {
                return OwnerStop::StillRunning;
            }
        }
        if let Some(thread) = lock(&self.thread).take() {
            let _ = thread.join();
        }
        OwnerStop::Exited
    }

    /// The entry's run lock, polled every [`IDLE_WAIT`] until `until`. `None` when another
    /// holder kept it that long; a poisoned lock is served as it is everywhere else.
    pub(super) fn lock_until(
        &self,
        until: tokio::time::Instant,
    ) -> Option<MutexGuard<'_, CaptureRun>> {
        poll_until(until, || match self.run.try_lock() {
            Ok(guard) => Some(guard),
            Err(std::sync::TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => None,
        })
    }

    /// Refuse the finish before `stage` and leave what the next one needs. Every stage but the
    /// run lock sets (or merges into) the sticky [`ExpiredFinish`]: `proxy` are the launcher's
    /// counters this attempt did not chain, `termination` the outcome it had already bound. The
    /// run lock is another finish's entry, so a refusal there latches nothing.
    pub(super) fn expire(
        &self,
        stage: FinishStage,
        proxy: Option<ProxyCounters>,
        termination: Option<TerminationOutcomeV1>,
    ) -> DaemonError {
        if stage != FinishStage::RunLock {
            let mut slot = lock(&self.expired);
            let earlier = slot.take();
            *slot = Some(ExpiredFinish {
                proxy_pending: proxy.or(earlier.as_ref().and_then(|e| e.proxy_pending)),
                termination: termination.or(earlier.and_then(|e| e.termination)),
            });
        }
        refused_at(stage)
    }

    /// An outcome the finish bound and then could not commit for a reason that is not a
    /// refusal: kept so a later finish does not read the harness as having ended on its own.
    pub(super) fn keep_termination(&self, termination: TerminationOutcomeV1) {
        let mut slot = lock(&self.expired);
        let earlier = slot.take();
        *slot = Some(ExpiredFinish {
            proxy_pending: earlier.as_ref().and_then(|e| e.proxy_pending),
            termination: Some(termination),
        });
    }

    /// Test seam: the pause a test asked for just before `stage` checks its admission.
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) fn pause_before(&self, stage: FinishStage) {
        let delay = {
            let mut slot = lock(&self.stage_delay);
            match slot.take() {
                Some((delayed, delay)) if delayed == stage => Some(delay),
                other => {
                    *slot = other;
                    None
                }
            }
        };
        if let Some(delay) = delay {
            std::thread::sleep(delay);
        }
    }

    #[cfg(not(any(test, feature = "test-hooks")))]
    #[inline(always)]
    pub(super) fn pause_before(&self, _stage: FinishStage) {}

    /// Finish stage 5, with the writer uncontended: chain what this finish carries, in the order
    /// the chain reads it (CONTRACTS.md, C6 local finish budget amendment, ¶4 and ¶5). First the
    /// deferred-scope gap, if the entry still owes it. Then the launcher's proxy counters, this
    /// request's or an expired attempt's. Then one stream-less `ShutdownDeadline` gap if an
    /// earlier attempt expired: the harness ran between the two attempts with admission closed,
    /// so whatever it did then was not recorded. Takes the sticky [`ExpiredFinish`] and returns
    /// the termination outcome it had already bound, for stage 6.
    pub(super) fn chain_carried_gaps(
        &self,
        request: &CaptureSealRequest,
        launched: bool,
    ) -> Option<TerminationOutcomeV1> {
        let latched = lock(&self.expired).take();
        let recorded = latched
            .as_ref()
            .and_then(|latched| latched.termination.clone());
        let counters = request
            .proxy
            .or(latched.as_ref().and_then(|latched| latched.proxy_pending));
        // A deferred scope that no finish installed: the snapshot was never read, and one
        // stream-less gap says so.
        let deferred_gap = self.take_deferred_gap(launched);
        let mut writer = lock(&self.writer);
        if deferred_gap {
            writer.streamless_gap(GapReason::SourceChanged, &(self.clock)());
        }
        if let Some(counters) = &counters {
            writer.proxy_gaps(counters, &(self.clock)());
        }
        if latched.is_some() {
            writer.streamless_gap(GapReason::ShutdownDeadline, &(self.clock)());
        }
        recorded
    }
}
