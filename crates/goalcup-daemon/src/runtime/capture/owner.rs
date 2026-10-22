//! The entry owner thread: one redactor per live producer, draining the sink into the chain
//! writer on a flush and sync cadence.

use super::*;

/// The owner-driven safe-byte flush cadence: inside the 250 ms bound, with scheduling headroom.
pub(crate) const FLUSH_EVERY: Duration = Duration::from_millis(200);

/// The durable-frontier cadence: inside the 1 s bound, with scheduling headroom.
pub(crate) const SYNC_EVERY: Duration = Duration::from_millis(800);

/// Native bytes the finish may still read.
pub(super) const NATIVE_FINISH_BYTES: u64 = 64 << 20;

/// How often the owner polls the native adapter when it reported no deferred work.
pub(super) const NATIVE_EVERY: Duration = Duration::from_millis(100);

/// The least time between two native discovery-loss gaps while the entry runs; the finish
/// records any growth left over.
pub(super) const NATIVE_LOSS_EVERY: Duration = Duration::from_secs(60);

/// Items taken per drain call.
pub(super) const DRAIN_BATCH: usize = 256;

/// The owner thread's state: one redactor per live producer and the producers it lost.
pub(super) struct Owner {
    sink: Arc<OwnerSink>,
    writer: Arc<Mutex<ChainWriter>>,
    clock: IngestClock,
    extra_patterns: Vec<ExtraPattern>,
    /// The run's exact secret values, redacted last by every producer's redactor (D60).
    secrets: crate::redact::RunSecrets,
    ingest: Option<Arc<IngestEntry>>,
    native: Option<NativeCapture>,
    redactors: BTreeMap<String, StreamRedactor>,
    /// Producers the owner interrupted: nothing of theirs is appended afterwards and no
    /// ProducerClosed is ever written for them.
    lost: BTreeSet<String>,
    pub(super) last_flush: Instant,
    pub(super) last_sync: Instant,
    last_native: Option<Instant>,
    /// The native adapter's `lost_entries` the chain already holds a gap for, and when it was
    /// written.
    native_lost: u64,
    native_lost_at: Option<Instant>,
    /// Where a producer's loss notes its cause; the gap records only that it was lost.
    cycles: Arc<CycleCounters>,
}

impl Owner {
    pub(super) fn new(
        sink: Arc<OwnerSink>,
        writer: Arc<Mutex<ChainWriter>>,
        clock: IngestClock,
        extra_patterns: &[ExtraPattern],
        secrets: crate::redact::RunSecrets,
        ingest: Option<Arc<IngestEntry>>,
        native: Option<NativeCapture>,
    ) -> Owner {
        Owner {
            sink,
            writer,
            clock,
            extra_patterns: extra_patterns.to_vec(),
            secrets,
            ingest,
            native,
            redactors: BTreeMap::new(),
            lost: BTreeSet::new(),
            last_flush: Instant::now(),
            last_sync: Instant::now(),
            last_native: None,
            native_lost: 0,
            native_lost_at: None,
            cycles: Arc::new(CycleCounters::default()),
        }
    }

    /// Note the causes of producer losses on `cycles`, the daemon's own.
    pub(super) fn noting_to(mut self, cycles: Arc<CycleCounters>) -> Owner {
        self.cycles = cycles;
        self
    }

    pub(super) fn spawn(self, control: Arc<OwnerControl>) -> std::io::Result<JoinHandle<()>> {
        std::thread::Builder::new()
            .name("capture-owner".into())
            .spawn(move || self.run(&control))
    }

    /// Serve until asked to stop, then finish the native adapter, cut every producer still
    /// live, drain everything queued into the chain and sync. An exit that did not get that far
    /// (a panic) leaves `drained` unset.
    pub(super) fn run(mut self, control: &OwnerControl) {
        struct Exit<'a>(&'a OwnerControl);
        impl Drop for Exit<'_> {
            fn drop(&mut self) {
                self.0.exited.store(true, Ordering::Release);
            }
        }
        let _exit = Exit(control);
        while !control.stop.load(Ordering::Acquire) {
            #[cfg(test)]
            if control.fault.load(Ordering::Acquire) {
                panic!("a test stops the owner before it drains");
            }
            self.cycle();
        }
        #[cfg(any(test, feature = "test-hooks"))]
        while control.hold_drain.load(Ordering::Acquire) {
            std::thread::sleep(IDLE_WAIT);
        }
        if let Some(native) = &mut self.native {
            let finished = native.finish(NativeFinishBudget {
                max_bytes: NATIVE_FINISH_BYTES,
                max_duration_ns: control.finish_budget_ns.load(Ordering::Acquire),
            });
            if finished.discovery_capped {
                lock(&self.writer).streamless_gap(GapReason::LimitExceeded, &(self.clock)());
            }
            self.record_native_losses(true);
        }
        self.apply_losses();
        self.sink.close_intake();
        self.drain_all();
        self.flush_all();
        self.sync();
        control.drained.store(true, Ordering::Release);
    }

    /// One pass: drain the queue, apply the receiver's losses, and poll the native adapter.
    pub(super) fn cycle(&mut self) {
        let wait = FLUSH_EVERY
            .saturating_sub(self.last_flush.elapsed())
            .min(IDLE_WAIT);
        for item in self.sink.drain(DRAIN_BATCH, wait) {
            self.handle(item);
            self.tick();
        }
        self.apply_losses();
        let native_due = self
            .last_native
            .is_none_or(|last| last.elapsed() >= NATIVE_EVERY);
        if native_due {
            self.poll_native();
        }
        self.tick();
    }

    pub(super) fn poll_native(&mut self) {
        let Some(native) = &mut self.native else {
            return;
        };
        let report = native.poll();
        // Deferred work polls again on the next pass.
        self.last_native = (!report.more_work).then(Instant::now);
        self.record_native_losses(false);
    }

    /// A stream-less `LimitExceeded` gap when the native adapter's entries kept out for good
    /// first appear and each time their count grows: at most one per `NATIVE_LOSS_EVERY`
    /// while the entry runs, and whatever growth is left at the finish (D57). A gap record
    /// carries no count; the trace reads incomplete from the first.
    fn record_native_losses(&mut self, finishing: bool) {
        let Some(native) = &self.native else {
            return;
        };
        let lost = native.loss_report().lost_entries;
        let recent = self
            .native_lost_at
            .is_some_and(|at| at.elapsed() < NATIVE_LOSS_EVERY);
        if lost <= self.native_lost || (recent && !finishing) {
            return;
        }
        self.native_lost = lost;
        self.native_lost_at = Some(Instant::now());
        lock(&self.writer).streamless_gap(GapReason::LimitExceeded, &(self.clock)());
    }

    /// The receiver's losses, each applied after every item queued before it was taken:
    /// a loss is latched after its producer's last queued frame. The sink's own queued close
    /// is the one that becomes `ProducerClosed`, so a `Closed` outcome adds nothing.
    pub(super) fn apply_losses(&mut self) {
        let Some(ingest) = &self.ingest else {
            return;
        };
        let losses: Vec<ProducerLoss> = ingest
            .take_outcomes()
            .into_iter()
            .filter_map(|outcome| match outcome {
                ProducerOutcome::Lost(loss) => Some(loss),
                ProducerOutcome::Closed(_) => None,
            })
            .collect();
        if losses.is_empty() {
            return;
        }
        self.drain_all();
        for loss in losses {
            self.lose(&loss.producer, loss.reason);
        }
    }

    /// Flush every redactor, then sync, each when its cadence is due: a sync never runs ahead
    /// of the flush that is due with it.
    pub(super) fn tick(&mut self) {
        if self.last_flush.elapsed() >= FLUSH_EVERY {
            self.flush_all();
        }
        if self.last_sync.elapsed() >= SYNC_EVERY {
            self.sync();
        }
    }

    pub(super) fn drain_all(&mut self) {
        loop {
            let items = self.sink.drain(DRAIN_BATCH, Duration::ZERO);
            if items.is_empty() {
                return;
            }
            for item in items {
                self.handle(item);
                self.tick();
            }
        }
    }

    pub(super) fn append_all(&mut self, records: Vec<CaptureRecord>) {
        let now = (self.clock)();
        let mut writer = lock(&self.writer);
        for record in records {
            writer.append(record, &now);
        }
    }

    pub(super) fn handle(&mut self, item: QueueItem) {
        match item {
            QueueItem::Frame(frame) => self.frame(frame),
            QueueItem::Lost { producer, reason } => self.lose(&producer, reason),
            QueueItem::Closed {
                producer,
                final_seq,
            } => self.closed(producer, final_seq),
        }
    }

    pub(super) fn frame(&mut self, frame: CaptureFrame) {
        let producer = frame.stream.producer.clone();
        if self.lost.contains(&producer) {
            return;
        }
        if let FrameEvent::Recovery {
            previous_producer,
            reason,
        } = &frame.event
            && self.redactors.contains_key(previous_producer)
        {
            // The previous epoch's undecided state is interrupted and retained first.
            self.lose(&previous_producer.clone(), *reason);
        }
        if !self.redactors.contains_key(&producer) {
            match StreamRedactor::for_run(&self.extra_patterns, &self.secrets) {
                Ok(redactor) => {
                    self.redactors.insert(producer.clone(), redactor);
                }
                Err(error) => return self.lose_because(&producer, error),
            }
        }
        let pushed = self
            .redactors
            .get_mut(&producer)
            .map(|redactor| redactor.push(frame));
        match pushed {
            Some(Ok(records)) => self.append_all(records),
            Some(Err(error)) => self.lose_because(&producer, error),
            None => self.lose(&producer, GapReason::ProducerLost),
        }
    }

    /// [`Self::lose`] a producer as `ProducerLost`, noting `cause`, which the gap cannot carry.
    fn lose_because(&mut self, producer: &str, cause: impl std::fmt::Display) {
        self.cycles.note(&DaemonError::Refused(format!(
            "a capture producer was lost: {cause}"
        )));
        self.lose(producer, GapReason::ProducerLost);
    }

    /// Interrupt `producer`'s redactor (if live) and chain its loss, as a gap on each stream
    /// it left open or, when it left none, as one stream-less gap.
    pub(super) fn lose(&mut self, producer: &str, reason: GapReason) {
        if !self.lost.insert(producer.to_string()) {
            return;
        }
        let observed = (self.clock)();
        let mut records = match self.redactors.remove(producer) {
            Some(mut redactor) => redactor.interrupt(observed.clone(), reason),
            None => Vec::new(),
        };
        if !records
            .iter()
            .any(|record| matches!(record, CaptureRecord::Gap { .. }))
        {
            records.push(streamless_gap_record(reason, &observed));
        }
        self.append_all(records);
    }

    pub(super) fn closed(&mut self, producer: String, final_seq: Option<u64>) {
        if self.lost.contains(&producer) {
            return;
        }
        if let Some(mut redactor) = self.redactors.remove(&producer) {
            match redactor.flush((self.clock)()) {
                Ok(records) => self.append_all(records),
                Err(error) => {
                    self.redactors.insert(producer.clone(), redactor);
                    return self.lose_because(&producer, error);
                }
            }
        }
        let observed = (self.clock)();
        let record = CaptureRecord::ProducerClosed {
            producer,
            observed: observed.clone(),
            final_seq,
        };
        lock(&self.writer).append(record, &observed);
    }

    pub(super) fn flush_all(&mut self) {
        self.last_flush = Instant::now();
        let producers: Vec<String> = self.redactors.keys().cloned().collect();
        for producer in producers {
            let now = (self.clock)();
            let flushed = self
                .redactors
                .get_mut(&producer)
                .map(|redactor| redactor.flush(now));
            match flushed {
                Some(Ok(records)) => self.append_all(records),
                Some(Err(error)) => self.lose_because(&producer, error),
                None => self.lose(&producer, GapReason::ProducerLost),
            }
        }
    }

    pub(super) fn sync(&mut self) {
        self.last_sync = Instant::now();
        // A failed flush is latched by the writer and reported by status.
        let _ = lock(&self.writer).sync();
    }
}
