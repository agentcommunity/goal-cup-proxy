//! The seal (PRD-02 section 11). It rides the item channel: a first marker behind every wire
//! record offered before the stop, the native finish, then a final marker that carries the seal
//! event's facts. The writer answers each once it has written all before it, so "drained" is the
//! marker reaching the writer: no polling and no timer (a live writer drains at its own rate; one
//! that writes nothing for 60 s is the stall exit).

use super::item::{Item, Mark, SealEnd, Sealed};
use super::status::{self, AfterSeal, Board, CutFiles, Outlived, Status};
use super::{Mono, Recorder, Sid, counts};
use crate::cli::log::{self, Fact};
use crate::mask::Masker;
use crate::native::NativeReader;
use crate::upload::{SealFacts, UploadHandle, WrapUp, policy::Policy};
use crate::watchdog::{Exit, HarnessFacts, SealReason};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

const S: u64 = 1_000_000_000;
/// The grace on a harness exit: no relay byte for 5 s, at most 30 s after the `exit` line.
const QUIET_NS: u64 = 5 * S;
const GRACE_NS: u64 = 30 * S;
/// The native finish's time guard at the end of the run window (`deadline`, `server_stop`,
/// `signal`); none on `harness_exit` and `seal_command`.
const NATIVE_GUARD: Duration = Duration::from_secs(180);
/// Each `git` call's budget.
const GIT_NS: u64 = 10 * S;

/// What the seal works with. `run` is called once, by the control thread.
pub(crate) struct Sealer {
    pub rec: Recorder,
    pub board: Option<Board>,
    /// The status thread: stopped once the seal is written, so from then on only the seal writes
    /// `status.json` (once, then each second for `after_seal`).
    pub status: Option<JoinHandle<()>>,
    pub entry_dir: PathBuf,
    pub workspace: Option<PathBuf>,
    /// The proxy's `quiet_since()`: the `Recorder::now()` of the last byte any relay moved.
    pub quiet_since: Option<Box<dyn Fn() -> Mono + Send>>,
    pub native: Option<NativeReader>,
    pub uploader: Option<UploadHandle>,
}

/// Why and how the seal came.
pub(crate) struct SealAsk {
    pub reason: SealReason,
    pub server_reason: Option<String>,
    pub harness: Option<HarnessFacts>,
    pub exit: Option<Exit>,
    /// When the `exit` line came, for the grace.
    pub exit_at: Option<Mono>,
    pub outlived: Option<Outlived>,
}

impl Sealer {
    /// Steps 1 to 7 of section 11. `on_durable` runs once the seal's LINK is synced and published
    /// and `rec/run-ca.key` is gone (it sends `sealed`); the uploader's wrap-up comes after it.
    pub(crate) fn run(self, ask: SealAsk, on_durable: impl FnOnce()) -> SealFacts {
        let Sealer {
            rec,
            board,
            status,
            entry_dir,
            workspace,
            quiet_since,
            native,
            uploader,
        } = self;
        let reason = ask.reason;
        if let (SealReason::HarnessExit, Some(quiet)) = (reason, &quiet_since) {
            let from = ask.exit_at.unwrap_or_else(|| rec.now());
            loop {
                let now = rec.now().ns();
                let quiet_for = now.saturating_sub(quiet().ns());
                if quiet_for >= QUIET_NS || now.saturating_sub(from.ns()) >= GRACE_NS {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        rec.stop_wire();
        let stop = rec.now();
        mark(&rec, None);

        let exited = reason == SealReason::HarnessExit || ask.exit.is_some();
        let guard = matches!(
            reason,
            SealReason::Deadline | SealReason::ServerStop | SealReason::Signal
        )
        .then_some(NATIVE_GUARD);
        let sealing = |on: bool| {
            if let Some(b) = &board {
                b.set(|s| s.sealing = on.then(|| "reading session files".into()));
            }
        };
        sealing(guard.is_none() && native.is_some());
        let fin = native.map(|n| n.finish(exited, guard)).unwrap_or_default();
        sealing(false);
        if let Some(o) = &ask.outlived {
            let note =
                json!({"ev": "note", "what": "outlived", "count": o.count, "programs": o.programs});
            rec.event(note);
        }
        let ws = workspace.as_deref();
        let head = ws.and_then(|w| git(w, &["rev-parse", "HEAD"]));
        let head = head.and_then(|b| Some(String::from_utf8(b).ok()?.trim().to_string()));
        let dirty = ws.and_then(|w| git(w, &["status", "--porcelain", "-z"]));
        let dirty = dirty.map(|b| hex::encode(Sha256::digest(b)));

        rec.stop_all();
        rec.close_open_gaps(rec.now());
        let harness = match (exited, &ask.harness) {
            (true, _) => "exited",
            (false, Some(_)) => "running",
            (false, None) => "unknown",
        };
        let mut event = json!({
            "ev": "seal", "reason": reason, "exit": ask.exit, "harness": harness,
            "workspace_head": head, "dirty": dirty,
        });
        if let Some(r) = &ask.server_reason {
            event["server_reason"] = r.as_str().into();
        }
        let (after_bytes, conns) = rec.after_stop();
        let seen: Vec<Sid> = conns.iter().map(|c| c.0).collect();
        let end = SealEnd {
            reason,
            workspace_head: head.clone(),
            event,
            stop,
            after_bytes,
            conns,
            tail: None,
            masker: None,
        };
        let Some(Sealed { totals, masker }) = mark(&rec, Some(Box::new(end))) else {
            // The writer is gone: the process is already ending with 70, and the next boot
            // recovers the entry.
            return SealFacts {
                reason,
                workspace_head: head,
                totals: Default::default(),
            };
        };

        remove_key(&entry_dir);
        if after_bytes > 0 {
            log::line(&entry_dir, "after_stop", &[("bytes", Fact::N(after_bytes))]);
        }
        // The sealed `status.json` is on disk before `sealed`, so the end screen reads it whole.
        // The status thread stops first: one writer from here on, and no stall exit after the
        // seal (the watchdog restarts nothing once `sealed` is sent).
        if let Some(b) = &board {
            b.stop();
            if let Some(t) = status {
                let _ = t.join();
            }
            b.set(|s| {
                let (files, bytes) = (fin.cut_files, fin.cut_bytes);
                s.cut_files = Some(CutFiles { files, bytes });
                s.outlived = ask.outlived.clone();
                s.after_seal = Some(AfterSeal::default());
            });
            status::write(&rec, &entry_dir, b);
        }
        on_durable();
        let late = Late {
            rec: rec.clone(),
            masker,
            at_seal: after_bytes,
            seen,
        };
        if let Some(b) = &board {
            let (rec, dir, b) = (rec.clone(), entry_dir.clone(), b.clone());
            late.clone().keep(move |a| {
                b.set(|s| s.after_seal = Some(a));
                status::write(&rec, &dir, &b);
            });
        }
        let facts = SealFacts {
            reason,
            workspace_head: head,
            totals,
        };
        wrap_up(&rec, &entry_dir, uploader, &facts, &late);
        facts
    }
}

/// The uploader's wrap-up (section 11 step 7), then what reached the wire after the seal (step
/// 6), which no record holds, to the log: its bytes and its masked hosts.
fn wrap_up(
    rec: &Recorder,
    entry_dir: &Path,
    uploader: Option<UploadHandle>,
    facts: &SealFacts,
    late: &Late,
) {
    if let Some(u) = uploader {
        u.finish(
            WrapUp::starting(rec.now(), &Policy::default()),
            facts.clone(),
        );
    }
    let a = late.now();
    if a.bytes > 0 {
        let mut line = vec![("bytes", Fact::N(a.bytes))];
        for h in &a.hosts {
            // A host the masker changed may no longer have host syntax: it is named masked.
            let host = log::Host::checked(h).map_or(Fact::Word("masked"), Fact::Host);
            line.push(("host", host));
        }
        log::line(entry_dir, "after_seal", &line);
    }
}

/// Removes `rec/run-ca.key` and syncs `rec/` (ledger R3b): at the seal, and on a boot that finds
/// the entry sealed by a recorder that died before it could.
pub(super) fn remove_key(entry_dir: &Path) {
    let rec_dir = entry_dir.join("rec");
    let _ = std::fs::remove_file(rec_dir.join(crate::watchdog::KEY_FILE));
    let _ = File::open(&rec_dir).and_then(|d| d.sync_all());
}

/// On an entry sealed before this boot: every byte this boot relays is after the seal, so from
/// boot on `after_seal` in the sealed `status.json` (what earlier boots put there, plus this
/// boot's) is kept current each second while the recorder lives. Returns what the wrap-up logs.
pub(super) fn keep_sealed(rec: &Recorder, entry_dir: &Path, masker: Option<Masker>) -> Late {
    let late = Late::sealed(rec, masker);
    let before = Status::read(entry_dir).and_then(|s| s.after_seal);
    let before = before.unwrap_or_default();
    let dir = entry_dir.to_path_buf();
    late.clone().keep(move |a| {
        let mut hosts = [&before.hosts[..], &a.hosts[..]].concat();
        hosts.sort_unstable();
        hosts.dedup();
        let bytes = before.bytes + a.bytes;
        sealed_status(&dir, |s| s.after_seal = Some(AfterSeal { bytes, hosts }));
    });
    late
}

/// The wrap-up on an entry sealed before this boot: the seal's reason and head from the sealed
/// `status.json` (else `reason`), and the totals counted from the files.
pub(super) fn wrap_up_sealed(
    rec: &Recorder,
    entry_dir: &Path,
    reason: SealReason,
    uploader: Option<UploadHandle>,
    late: &Late,
) {
    let seal = Status::read(entry_dir).and_then(|s| s.seal);
    let facts = SealFacts {
        reason: seal.as_ref().map_or(reason, |s| s.reason),
        workspace_head: seal.and_then(|s| s.workspace_head),
        totals: counts::entry_totals(entry_dir),
    };
    wrap_up(rec, entry_dir, uploader, &facts, late);
}

/// The sealed `status.json` with `f` applied, the seal's own fields kept: a boot on an entry
/// sealed before it names itself there, and its `after_seal` bytes.
pub(super) fn sealed_status(entry_dir: &Path, f: impl FnOnce(&mut Status)) {
    if let Some(mut s) = Status::read(entry_dir) {
        f(&mut s);
        let _ = s.write(entry_dir);
    }
}

/// Sends a seal marker and waits for the writer's answer. None for the first marker, or when the
/// pipeline is gone (or sends nothing).
fn mark(rec: &Recorder, last: Option<Box<SealEnd>>) -> Option<Sealed> {
    let (reply, answer) = mpsc::channel();
    rec.send(Item::Mark(Mark { last, reply }));
    answer.recv().ok().flatten()
}

/// What reached the wire after the seal, which no record holds: the bytes relayed after
/// `at_seal` (the count when the seal's tail was taken, just before its final marker), and the
/// hosts of connections opened after the stop that carried bytes and are not in `seen` (named at
/// the seal), masked.
#[derive(Clone)]
pub(super) struct Late {
    rec: Recorder,
    masker: Option<Masker>,
    at_seal: u64,
    /// Sorted, as `after_stop` returns it.
    seen: Vec<Sid>,
}

impl Late {
    /// On an entry sealed before this boot: every byte and host of this boot is late.
    pub(super) fn sealed(rec: &Recorder, masker: Option<Masker>) -> Late {
        Late {
            rec: rec.clone(),
            masker,
            at_seal: 0,
            seen: Vec::new(),
        }
    }

    /// The count and the hosts now, one name per host however many connections went to it. A
    /// masker panic, or no masker, stars the hosts and keeps the count.
    fn now(&self) -> AfterSeal {
        let (bytes, conns) = self.rec.after_stop();
        let new = conns
            .into_iter()
            .filter(|c| self.seen.binary_search(&c.0).is_err());
        let mut hosts = Value::from(new.filter_map(|c| c.1).collect::<Vec<_>>());
        let masked = (self.masker.as_ref())
            .map(|m| catch_unwind(AssertUnwindSafe(|| m.mask_meta(&mut hosts))).is_ok());
        if masked != Some(true) {
            hosts = star(&hosts);
        }
        let mut hosts: Vec<String> = serde_json::from_value(hosts).unwrap_or_default();
        hosts.sort_unstable();
        hosts.dedup();
        let bytes = bytes.saturating_sub(self.at_seal);
        AfterSeal { bytes, hosts }
    }

    /// Hands `put` the count and hosts each second while the process lives.
    fn keep(self, put: impl Fn(AfterSeal) + Send + 'static) {
        let run = move || {
            loop {
                std::thread::sleep(Duration::from_secs(1));
                put(self.now());
            }
        };
        let _ = std::thread::Builder::new()
            .name("goalcup-after-seal".into())
            .spawn(run);
    }
}

/// Every string of a list of hosts as stars of its length.
fn star(hosts: &Value) -> Value {
    let all = hosts.as_array().into_iter().flatten();
    let stars = all.map(|h| "*".repeat(h.as_str().map_or(0, str::len)));
    Value::from(stars.collect::<Vec<_>>())
}

/// `git <args>` in `dir` with no optional lock (it never takes the workspace's index lock), its
/// output, or None on failure or after 10 s of boot time.
pub(super) fn git(dir: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let mut child = Command::new("git")
        .arg("--no-optional-locks")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut out = child.stdout.take()?;
    let read = std::thread::spawn(move || {
        let mut b = Vec::new();
        out.read_to_end(&mut b).map(|_| b)
    });
    let until = Mono::now().ns() + GIT_NS;
    loop {
        match child.try_wait() {
            Ok(Some(st)) if st.success() => return read.join().ok()?.ok(),
            Ok(None) if Mono::now().ns() < until => std::thread::sleep(Duration::from_millis(20)),
            Ok(Some(_)) => return None,
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

#[cfg(feature = "test-hooks")]
impl Recorder {
    /// The seal for tests over `real_for_tests`: no status board, native reader, uploader,
    /// workspace or grace. Returns the sealed entry totals.
    pub fn seal_for_tests(&self, entry_dir: &Path, reason: SealReason) -> super::Counts {
        let sealer = Sealer {
            rec: self.clone(),
            board: None,
            status: None,
            entry_dir: entry_dir.to_path_buf(),
            workspace: None,
            quiet_since: None,
            native: None,
            uploader: None,
        };
        let ask = SealAsk {
            reason,
            server_reason: None,
            harness: None,
            exit: None,
            exit_at: None,
            outlived: None,
        };
        sealer.run(ask, || {}).totals.counts
    }

    /// The seal's wire stop alone, so a test can offer calls between the stop and the seal.
    pub fn stop_wire_for_tests(&self) {
        self.stop_wire();
    }
}
