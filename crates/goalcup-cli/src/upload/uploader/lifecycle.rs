//! The upload's lifecycle beside a run: the threads `goalcup start` begins (heartbeats and
//! uploading), the finish it asks for, and the drain of an earlier segment.

use super::heartbeat::watch_deadline;
use super::session::{Attempt, STOPPING, note_refusal, past, ran_out};
use super::{EndedGate, LocalDeadline, Policy, Report, Uploader, WrapUp, backoff, lock_entry};
use crate::CliError;
use crate::run::Vitals;
use crate::upload::UploadRecord;
use crate::upload::http::ServerClient;
use crate::upload::policy::Cadence;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// The uploader running beside `goalcup start`, on a thread of its own.
pub struct Running {
    entry_id: String,
    /// `None` once dropped: the upload thread then sees the run's end.
    finish: Option<std::sync::mpsc::Sender<WrapUp>>,
    /// The flag that keeps the background upload from starting anything more.
    quit: std::sync::Arc<AtomicBool>,
    report: std::sync::mpsc::Receiver<Report>,
    last_ok: std::sync::Arc<AtomicU64>,
    acked_offset: std::sync::Arc<AtomicU64>,
}

impl Drop for Running {
    /// A `Running` dropped without a finish (the run ended some other way) stops its upload
    /// thread rather than leaving it to retry in the background: nothing more is started, and
    /// the thread gets a moment to say it has returned.
    fn drop(&mut self) {
        self.quit.store(true, Ordering::SeqCst);
        // Its sender gone, the upload thread sees the end of the run and returns.
        drop(self.finish.take());
        let _ = self.report.recv_timeout(Duration::from_secs(2));
    }
}

/// How long past the wrap-up `goalcup start` waits for the uploader's own last word.
const FINISH_GRACE: Duration = Duration::from_secs(10);

/// How often the wait for the uploader's last word looks at the continuous clock again.
const FINISH_SLICE: Duration = Duration::from_secs(5);

/// Start uploading `record` beside the run: the first chunk at once, then on [`Policy`]'s
/// cadence. Its waits and failures are said on stderr, prefixed `goalcup: upload:`; nothing it
/// does can stop or slow the recording.
///
/// `stop` is the run's one stop channel: when the server says the run is over (its deadline,
/// or a newer run replacing this one), or this machine's count of the run's length runs out,
/// the reason is sent on it once. `vitals` is what the heartbeats say about the recording, and
/// where the uploader leaves its own trouble for `goalcup status`.
#[allow(clippy::too_many_arguments)]
pub fn begin(
    server: ServerClient,
    record: UploadRecord,
    receipt_key: String,
    policy: Policy,
    spool_dir: PathBuf,
    local_deadline: Option<LocalDeadline>,
    stop: std::sync::mpsc::Sender<String>,
    vitals: Vitals,
) -> Result<Running, CliError> {
    begin_spawning(
        &spawn_thread,
        server,
        record,
        receipt_key,
        policy,
        spool_dir,
        local_deadline,
        stop,
        vitals,
    )
}

/// A thread named `name` running `work`, detached.
type Spawn<'a> = &'a dyn Fn(&str, Box<dyn FnOnce() + Send>) -> std::io::Result<()>;

fn spawn_thread(name: &str, work: Box<dyn FnOnce() + Send>) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name(name.into())
        .spawn(work)
        .map(|_| ())
}

/// [`begin`], starting its threads with `spawn`. The heartbeat thread starts first, and a
/// thread that cannot start is an error of `begin`: a run whose heartbeats and deadline watch
/// cannot run is not begun, because nothing would tell the server it was alive, or stop it at
/// its deadline.
#[allow(clippy::too_many_arguments)]
fn begin_spawning(
    spawn: Spawn<'_>,
    server: ServerClient,
    record: UploadRecord,
    receipt_key: String,
    policy: Policy,
    spool_dir: PathBuf,
    local_deadline: Option<LocalDeadline>,
    stop: std::sync::mpsc::Sender<String>,
    vitals: Vitals,
) -> Result<Running, CliError> {
    let lock = lock_entry(&spool_dir, &record.entry_id)?;
    let entry_id = record.entry_id.clone();
    let (finish, finishing) = std::sync::mpsc::channel::<WrapUp>();
    let (reported, report) = std::sync::mpsc::channel::<Report>();
    let last_ok = std::sync::Arc::new(AtomicU64::new(crate::run::now_unix()));
    let acked_offset = std::sync::Arc::new(AtomicU64::new(0));
    let shared = (last_ok.clone(), acked_offset.clone());
    // Heartbeats and the deadline run on a thread of their own, so a slow chunk upload never
    // delays a stop. Once the run must stop, uploading waits for the finish.
    let over = std::sync::Arc::new(AtomicBool::new(false));
    let done = std::sync::Arc::new(AtomicBool::new(false));
    let ended = EndedGate::default();
    let spawn_error = |what: &str| {
        let what = what.to_string();
        move |source| CliError::Io {
            path: format!("the {what} thread"),
            source,
        }
    };
    {
        let (server, record, vitals) = (server.clone(), record.clone(), vitals.clone());
        let (over, done, ended) = (over.clone(), done.clone(), ended.clone());
        spawn(
            "goalcup-heartbeat",
            Box::new(move || {
                watch_deadline(
                    &server,
                    &record,
                    policy,
                    local_deadline,
                    &vitals,
                    &stop,
                    &over,
                    &done,
                    &ended,
                )
            }),
        )
        .map_err(spawn_error("heartbeat"))?;
    }
    let quit = over.clone();
    let beside = Beside {
        finishing,
        over,
        ended,
        shared,
        vitals,
    };
    let heartbeats_done = done.clone();
    spawn(
        "goalcup-upload",
        Box::new(move || {
            let _lock = lock;
            let _done_on_return = SetOnDrop(done);
            let ended = run(server, record, receipt_key, policy, &spool_dir, beside);
            let _ = reported.send(ended);
        }),
    )
    .map_err(|source| {
        // The heartbeat thread is already running: it is told to end, as the upload thread
        // would have told it.
        heartbeats_done.store(true, Ordering::SeqCst);
        spawn_error("upload")(source)
    })?;
    Ok(Running {
        entry_id,
        finish: Some(finish),
        quit,
        report,
        last_ok,
        acked_offset,
    })
}

/// What the upload thread hears from, and says to, `goalcup start`.
struct Beside {
    /// The run is sealed: finish within this wrap-up.
    finishing: std::sync::mpsc::Receiver<WrapUp>,
    /// Set by the heartbeat thread once the run must stop.
    over: std::sync::Arc<AtomicBool>,
    /// Set when the final heartbeat is stored, after which no ordinary one is sent.
    ended: EndedGate,
    /// The counters [`Running::last_ok`] and [`Running::acked_offset`] read.
    shared: (std::sync::Arc<AtomicU64>, std::sync::Arc<AtomicU64>),
    vitals: Vitals,
}

/// Sets its flag when dropped: the watcher thread's signal that the uploader has returned.
struct SetOnDrop(std::sync::Arc<AtomicBool>);

impl Drop for SetOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

impl Running {
    /// The unix second of the last time this upload worked: it starts at the run's launch, and
    /// is set again by each acknowledged chunk, by the seal, and whenever there is nothing
    /// left to send. The run's disk watch calls uploads failing when it is half an hour old.
    pub fn last_ok(&self) -> std::sync::Arc<AtomicU64> {
        self.last_ok.clone()
    }

    /// The entry this upload sends.
    pub fn entry_id(&self) -> &str {
        &self.entry_id
    }

    /// Bytes of the chain the server has acknowledged.
    pub fn acked_offset(&self) -> std::sync::Arc<AtomicU64> {
        self.acked_offset.clone()
    }

    /// The run is sealed: send the rest and the seal, asking again after every failure,
    /// whatever the answer (D64), until `wrap_up` has run out, and say how it went. Returns by
    /// then (and a short grace) whatever the server does: an upload still under way is left
    /// to `goalcup upload`.
    ///
    /// The wait is counted on the continuous clock, in slices: a machine that sleeps through
    /// the wrap-up does not owe it again on waking.
    pub fn finish(self, wrap_up: WrapUp) -> Report {
        if let Some(finish) = &self.finish {
            let _ = finish.send(wrap_up);
        }
        let last_word = WrapUp::after(wrap_up.remaining() + FINISH_GRACE);
        loop {
            let left = last_word.remaining();
            if left.is_zero() {
                return Report::not("the upload did not finish in time", 0);
            }
            match self.report.recv_timeout(left.min(FINISH_SLICE)) {
                Ok(report) => return report,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Report::not("the upload thread ended without a report", 0);
                }
            }
        }
    }
}

/// How the end of an entry that is no longer recording is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Closing {
    /// The entry is sealed locally: its end, the rest of its chain, then the seal.
    Sealed,
    /// The entry could not be sealed (the host died, the machine rebooted, the chain's tail is
    /// torn): its end is the last whole record of its chain, committed in a final heartbeat, and
    /// the chain is sent up to it. No seal goes; the segment is incomplete, and says so.
    FromTail,
}

/// An entry that is no longer recording, being closed and sent on a thread of its own
/// ([`drain`]), beside the segment that came after it.
pub struct Draining {
    entry_id: String,
    closing: Closing,
    last_ok: std::sync::Arc<AtomicU64>,
    acked_offset: std::sync::Arc<AtomicU64>,
    report: std::sync::mpsc::Receiver<Report>,
}

/// A run's own uploader for `record` (D64): the server's position asked for again after every
/// failure, whatever the answer, until `until` has run out, when `Err` is the report. Each wait
/// is said, prefixed `label`; a refusal is counted in `vitals` when there are any.
#[allow(clippy::too_many_arguments)]
fn resume_for_run(
    server: &ServerClient,
    record: &UploadRecord,
    key: &str,
    policy: Policy,
    spool_dir: &Path,
    until: WrapUp,
    vitals: Option<&Vitals>,
    label: &str,
) -> Result<Uploader, Report> {
    let mut failures = 0u32;
    loop {
        match Uploader::resume(
            server.clone(),
            record.clone(),
            key.to_string(),
            policy,
            spool_dir,
        ) {
            Ok(mut up) => {
                up.persist = true;
                up.vitals = vitals.cloned();
                up.note_answered();
                return Ok(up);
            }
            Err(error) => {
                note_refusal(vitals, &error);
                failures += 1;
                let wait = backoff(
                    &policy,
                    failures,
                    error.retry_after().map(Duration::from_secs),
                );
                if past(Some(until), wait) {
                    return Err(Report::not(ran_out(Some(&error.to_string())), 0));
                }
                crate::host_log::say(&format!(
                    "{label}: {error}; trying again in {}s",
                    wait.as_secs().max(1)
                ));
                std::thread::sleep(wait);
            }
        }
    }
}

/// Close `record`'s entry on the server on a thread of its own: its final heartbeat, the rest of
/// its chain and (when it is [`Closing::Sealed`]) its seal, asking again after every failure,
/// whatever the answer (D64), until `wrap_up` has run out. It holds the entry's upload lock
/// ([`lock_entry`]) until it is done, so a refusal here means another goalcup uploads the entry.
pub fn drain(
    server: ServerClient,
    record: UploadRecord,
    receipt_key: String,
    policy: Policy,
    spool_dir: PathBuf,
    closing: Closing,
    wrap_up: WrapUp,
) -> Result<Draining, CliError> {
    let lock = lock_entry(&spool_dir, &record.entry_id)?;
    let entry_id = record.entry_id.clone();
    let (reported, report) = std::sync::mpsc::channel::<Report>();
    let last_ok = std::sync::Arc::new(AtomicU64::new(crate::run::now_unix()));
    let acked_offset = std::sync::Arc::new(AtomicU64::new(0));
    let shared = (last_ok.clone(), acked_offset.clone());
    std::thread::Builder::new()
        .name("goalcup-drain".into())
        .spawn(move || {
            let _lock = lock;
            let mut say = crate::host_log::say;
            let label = format!("upload of entry {}", record.entry_id);
            let ended = match resume_for_run(
                &server,
                &record,
                &receipt_key,
                policy,
                &spool_dir,
                wrap_up,
                None,
                &label,
            ) {
                Ok(mut up) => {
                    up.share(&shared.0, &shared.1);
                    up.close_unsealed = closing == Closing::FromTail;
                    up.finish(Some(wrap_up), &mut say)
                }
                Err(report) => report,
            };
            let _ = reported.send(ended);
        })
        .map_err(|source| CliError::Io {
            path: "the upload thread".to_string(),
            source,
        })?;
    Ok(Draining {
        entry_id,
        closing,
        last_ok,
        acked_offset,
        report,
    })
}

/// What to do about an earlier segment's unfinished upload. One sealed by its host is finished
/// by `goalcup upload`; one closed from its tail has an end `goalcup upload` cannot commit, and
/// the next `goalcup continue` closes and sends it again.
pub fn finish_advice(closing: Closing, entry_id: &str) -> String {
    match closing {
        Closing::Sealed => format!("Run `goalcup upload {entry_id}` to finish sending it."),
        Closing::FromTail => {
            "Run `goalcup continue` (it finishes the earlier segment's upload).".to_string()
        }
    }
}

/// What the disk watch reads of an entry that is being drained: the same two counters as of a
/// recording entry's upload ([`Running::last_ok`], [`Running::acked_offset`]).
#[derive(Debug, Clone)]
pub struct DrainHandles {
    pub entry_id: String,
    pub last_ok: std::sync::Arc<AtomicU64>,
    pub acked_offset: std::sync::Arc<AtomicU64>,
}

impl Draining {
    /// The entry this thread sends.
    pub fn entry_id(&self) -> &str {
        &self.entry_id
    }

    /// How the entry was closed: a segment closed from its tail cannot be finished by
    /// `goalcup upload`, only by a `goalcup continue`.
    pub fn closing(&self) -> Closing {
        self.closing
    }

    /// What to tell the participant to do about this entry's upload when it did not finish.
    pub fn how_to_finish(&self) -> String {
        finish_advice(self.closing, &self.entry_id)
    }

    /// The counters the disk watch reads, for the watch to cover this entry too.
    pub fn handles(&self) -> DrainHandles {
        DrainHandles {
            entry_id: self.entry_id.clone(),
            last_ok: self.last_ok.clone(),
            acked_offset: self.acked_offset.clone(),
        }
    }

    /// Wait for the thread's report for as long as `until` leaves (and a short grace). The
    /// thread has a wrap-up of its own and stops by it; a report that is not there by then is
    /// "did not finish", and what is unsent is `goalcup upload`'s.
    pub fn wait(self, until: WrapUp) -> Report {
        let last_word = WrapUp::after(until.remaining() + FINISH_GRACE);
        loop {
            let left = last_word.remaining();
            if left.is_zero() {
                return Report::not("the upload did not finish in time", 0);
            }
            match self.report.recv_timeout(left.min(FINISH_SLICE)) {
                Ok(report) => return report,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Report::not("the upload thread ended without a report", 0);
                }
            }
        }
    }
}

/// The upload thread beside a recording run. Nothing ends it but the run's end (D64): every
/// failure, whatever the server answered, is asked again after a backoff, and the run goes on
/// recording meanwhile. The finish then sends the rest within the wrap-up.
fn run(
    server: ServerClient,
    record: UploadRecord,
    key: String,
    policy: Policy,
    spool_dir: &Path,
    beside: Beside,
) -> Report {
    use std::sync::mpsc::RecvTimeoutError;
    let Beside {
        finishing,
        over,
        ended,
        shared,
        vitals,
    } = beside;
    let mut say = crate::host_log::say;
    let mut uploader: Option<Uploader> = None;
    let mut failures = 0u32;
    let mut wait_until = Instant::now();
    let mut next_due = Instant::now();
    let mut cadence = Cadence::new(&policy);
    loop {
        let now = Instant::now();
        if !over.load(Ordering::SeqCst) && now >= wait_until {
            if uploader.is_none() {
                match Uploader::resume(
                    server.clone(),
                    record.clone(),
                    key.clone(),
                    policy,
                    spool_dir,
                ) {
                    Ok(mut resumed) => {
                        resumed.persist = true;
                        resumed.halt = Some(over.clone());
                        resumed.share(&shared.0, &shared.1);
                        resumed.vitals = Some(vitals.clone());
                        resumed.ended = ended.clone();
                        resumed.note_answered();
                        uploader = Some(resumed);
                    }
                    Err(error) => {
                        note_refusal(Some(&vitals), &error);
                        failures += 1;
                        let wait = backoff(
                            &policy,
                            failures,
                            error.retry_after().map(Duration::from_secs),
                        );
                        say(&format!(
                            "upload: {error}; trying again in {}s",
                            wait.as_secs().max(1)
                        ));
                        vitals.upload_failed(&error.to_string());
                        wait_until = now + wait;
                    }
                }
            }
            // Nothing waiting is an upload with nothing to complain of.
            if let Some(up) = uploader.as_ref()
                && up.waiting() == 0
                // An acknowledgement still to be written is trouble, not an idle upload.
                && up.owed_ack.is_none()
            {
                up.mark_ok();
                vitals.upload_ok();
            }
            if let Some(up) = uploader.as_mut()
                && (now >= next_due || up.waiting() >= policy.eager_bytes)
            {
                let retry = match up.attempt(None) {
                    Attempt::CaughtUp => {
                        failures = 0;
                        next_due = cadence.next_due(now);
                        vitals.upload_ok();
                        None
                    }
                    Attempt::Retry(after, why) => Some((after, why)),
                    // A run's upload asks again instead (`persist`); never the end of it.
                    Attempt::Stop(why) => Some((None, why)),
                    // Only a finish has a wrap-up to run out; a background attempt has none.
                    Attempt::RanOut => None,
                };
                if let Some((after, why)) = retry {
                    failures += 1;
                    let wait = backoff(&policy, failures, after);
                    say(&format!(
                        "upload: {why}; trying again in {}s",
                        wait.as_secs().max(1)
                    ));
                    if why != STOPPING {
                        vitals.upload_failed(&why);
                    }
                    wait_until = now + wait;
                }
            }
        }
        let offset = || uploader.as_ref().map(|u| u.pos.offset).unwrap_or(0);
        match finishing.recv_timeout(policy.poll) {
            Err(RecvTimeoutError::Timeout) => continue,
            // `goalcup start` ended without a finish (a refusal after the launch): stop here.
            Err(RecvTimeoutError::Disconnected) => {
                return Report::not("the run ended before its upload finished", offset());
            }
            Ok(wrap_up) => {
                let mut up = match uploader {
                    Some(up) => up,
                    // Never reached during the run: asked again through the wrap-up.
                    None => match resume_for_run(
                        &server,
                        &record,
                        &key,
                        policy,
                        spool_dir,
                        wrap_up,
                        Some(&vitals),
                        "upload",
                    ) {
                        Ok(mut up) => {
                            up.share(&shared.0, &shared.1);
                            up.ended = ended.clone();
                            up
                        }
                        Err(report) => return report,
                    },
                };
                return up.finish(Some(wrap_up), &mut say);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::record_in;
    use super::*;
    use std::sync::Mutex;

    /// A segment closed from its tail cannot be finished by `goalcup upload`; the advice for
    /// it is `goalcup continue`, which closes and sends it again.
    #[test]
    fn a_tail_closed_segment_is_finished_by_continue_not_upload() {
        let sealed = finish_advice(Closing::Sealed, "e1");
        assert!(sealed.contains("goalcup upload e1"), "{sealed}");
        let tail = finish_advice(Closing::FromTail, "e1");
        assert!(tail.contains("goalcup continue"), "{tail}");
        assert!(!tail.contains("goalcup upload"), "{tail}");
    }

    /// A heartbeat thread that cannot start is an error of `begin`, and the upload thread is
    /// never started behind it.
    #[test]
    fn a_heartbeat_thread_that_cannot_start_is_an_error_of_begin() {
        let dir = tempfile::tempdir().unwrap();
        let spawned = Mutex::new(Vec::<String>::new());
        let spawn = |name: &str, _work: Box<dyn FnOnce() + Send>| {
            spawned.lock().unwrap().push(name.to_string());
            Err(std::io::Error::other("no threads left"))
        };
        let (stop, _stopped) = std::sync::mpsc::channel();
        let result = begin_spawning(
            &spawn,
            ServerClient::anonymous("http://127.0.0.1:1"),
            record_in(dir.path()),
            "key".into(),
            Policy::default(),
            dir.path().join("uploads"),
            None,
            stop,
            Vitals::default(),
        );
        match result {
            Err(CliError::Io { path, source }) => {
                assert!(path.contains("heartbeat"), "{path}");
                assert!(source.to_string().contains("no threads left"), "{source}");
            }
            Err(other) => panic!("a different error: {other}"),
            Ok(_) => panic!("begin went on without its heartbeat thread"),
        }
        assert_eq!(*spawned.lock().unwrap(), ["goalcup-heartbeat"]);
    }

    /// An upload thread that cannot start is an error too, and the heartbeat thread that is
    /// already running is told to end rather than left beating for a run that never began.
    #[test]
    fn an_upload_thread_that_cannot_start_ends_the_heartbeat_thread_it_leaves() {
        let dir = tempfile::tempdir().unwrap();
        let heartbeat = Mutex::new(None);
        let spawn = |name: &str, work: Box<dyn FnOnce() + Send>| {
            if name == "goalcup-heartbeat" {
                *heartbeat.lock().unwrap() = Some(std::thread::spawn(work));
                Ok(())
            } else {
                Err(std::io::Error::other("no threads left"))
            }
        };
        let (stop, _stopped) = std::sync::mpsc::channel();
        let result = begin_spawning(
            &spawn,
            ServerClient::anonymous("http://127.0.0.1:1"),
            record_in(dir.path()),
            "key".into(),
            Policy::default(),
            dir.path().join("uploads"),
            None,
            stop,
            Vitals::default(),
        );
        assert!(
            matches!(result, Err(CliError::Io { ref path, .. }) if path.contains("upload")),
            "begin refused"
        );
        // Joins only if the thread was told to end.
        heartbeat.lock().unwrap().take().unwrap().join().unwrap();
    }
}
