//! The wrap-up of one entry's upload: its end committed in the final heartbeat, the rest of the
//! chain, then the seal; and the report of how it went.

use super::heartbeat::{Beat, heartbeat};
use super::local::noted_repo_head;
use super::session::{Attempt, left, past, ran_out};
use super::{Uploader, WrapUp, backoff};
use crate::upload::api::{SealRequest, StatusAnswer};
use crate::upload::http::ServerError;
use goalcup_protocol::export::{BASELINE_FILE, SEAL_FILE};
use std::io::Read;
use std::path::Path;
use std::sync::PoisonError;
use std::time::Duration;

/// How an upload ended, for the participant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// Every complete line of the chain is on the server, acknowledged.
    pub caught_up: bool,
    /// The server has the run's `seal.json` and `baseline.json`.
    pub sealed: bool,
    /// Why not, when either is false.
    pub why: Option<String>,
    /// Bytes of the chain the server holds.
    pub offset: u64,
    /// The server's answer to the seal says the run itself is over (sealed or incomplete).
    pub run_ended: bool,
}

impl Report {
    pub(super) fn not(why: impl Into<String>, offset: u64) -> Report {
        Report {
            caught_up: false,
            sealed: false,
            why: Some(why.into()),
            offset,
            run_ended: false,
        }
    }
}

impl Uploader {
    /// Send everything and then the seal, asking again after each failure that does not end
    /// the upload, all within the wrap-up `until` (or for as long as it takes, with `None`).
    /// `say` hears each wait.
    ///
    /// The seal's documents are read before the chain is drained: the daemon writes them
    /// after the chain's last record, so the drain that follows sends every record they seal,
    /// and the server is never sealed on a copy missing its tail.
    pub fn finish(&mut self, until: Option<WrapUp>, say: &mut dyn FnMut(&str)) -> Report {
        let documents = self.seal_documents();
        // A sealed run commits its end first: after the deadline, the server accepts nothing
        // past it, and nothing at all until it is committed. So does a segment closed from its
        // tail, which has the end to commit and no seal to send.
        if (documents.is_some() || self.close_unsealed)
            && let Err(why) = self.commit_end(until, say)
        {
            return Report::not(why, self.pos.offset);
        }
        let mut failures = 0;
        let mut last = None::<String>;
        loop {
            match self.attempt(until) {
                Attempt::CaughtUp => break,
                Attempt::RanOut => {
                    return Report::not(ran_out(last.as_deref()), self.pos.offset);
                }
                Attempt::Stop(why) => return Report::not(why, self.pos.offset),
                Attempt::Retry(after, why) => {
                    failures += 1;
                    let wait = backoff(&self.policy, failures, after);
                    if past(until, wait) {
                        return Report::not(ran_out(Some(&why)), self.pos.offset);
                    }
                    say(&format!(
                        "upload: {why}; trying again in {}s",
                        wait.as_secs().max(1)
                    ));
                    last = Some(why);
                    std::thread::sleep(wait);
                }
            }
        }
        let caught_up = |sealed, why, run_ended| Report {
            caught_up: true,
            sealed,
            why,
            offset: self.pos.offset,
            run_ended,
        };
        let Some(documents) = documents else {
            // A segment closed from its tail has no seal to send: it is done when its chain,
            // up to its committed end, is.
            if self.close_unsealed {
                self.note_sent();
            }
            return caught_up(false, None, false);
        };
        let mut failures = 0;
        let mut last = None::<String>;
        loop {
            let Some(timeout) = left(until, crate::upload::http::CALL_TIMEOUT) else {
                return caught_up(false, Some(ran_out(last.as_deref())), false);
            };
            match self.seal(&documents, timeout) {
                Ok(run_ended) => {
                    self.note_answered();
                    self.mark_ok();
                    self.note_sent();
                    return caught_up(true, None, run_ended);
                }
                Err(error) if !self.gives_up(&error) => {
                    self.note_refused(&error);
                    failures += 1;
                    let wait = backoff(
                        &self.policy,
                        failures,
                        error.retry_after().map(Duration::from_secs),
                    );
                    if past(until, wait) {
                        return caught_up(false, Some(ran_out(Some(&error.to_string()))), false);
                    }
                    say(&format!(
                        "upload: the seal was not stored ({error}); trying again in {}s",
                        wait.as_secs().max(1)
                    ));
                    last = Some(error.to_string());
                    std::thread::sleep(wait);
                }
                Err(error) => return caught_up(false, Some(error.to_string()), false),
            }
        }
    }

    /// The final heartbeat: commit the sealed chain's last record as the run's end, asking
    /// again after each failure that does not end the upload (for a run's own upload, every
    /// failure: D64), within the wrap-up `until`. A
    /// chain that cannot be read just now is a failure of that kind too: it is read again after
    /// the same backoff, not taken for a chain with no whole record.
    fn commit_end(
        &mut self,
        until: Option<WrapUp>,
        say: &mut dyn FnMut(&str),
    ) -> Result<(), String> {
        let mut failures = 0;
        let mut last = None::<String>;
        loop {
            let repo_head = noted_repo_head(&self.spool, &self.record.entry_id);
            // Under the lock the run's ordinary heartbeats take, so none is in flight behind
            // this one, and none is sent once this one has gone. The time left is measured
            // after the lock is taken: an ordinary heartbeat in flight holds it for as long as
            // it takes, and must not come out of this call's time.
            let (retry_after, why) = {
                let mut ended = self.ended.lock().unwrap_or_else(PoisonError::into_inner);
                let Some(timeout) = left(until, crate::upload::http::CALL_TIMEOUT) else {
                    return Err(ran_out(last.as_deref()));
                };
                let beat = heartbeat(
                    &self.server,
                    &self.record,
                    true,
                    repo_head,
                    self.vitals.as_ref(),
                    timeout,
                );
                // Once it has been sent, whatever the answer, the run's end is the final
                // heartbeat's to commit: no ordinary one follows it.
                if matches!(beat, Beat::Answer(_)) {
                    *ended = true;
                }
                match beat {
                    Beat::Empty => return Err("the chain holds no whole record".to_string()),
                    Beat::Answer(Ok(_)) => {
                        self.note_answered();
                        return Ok(());
                    }
                    Beat::Answer(Err(error)) if self.gives_up(&error) => {
                        return Err(error.to_string());
                    }
                    Beat::Answer(Err(error)) => {
                        self.note_refused(&error);
                        (
                            error.retry_after().map(Duration::from_secs),
                            format!("the final heartbeat was not stored ({error})"),
                        )
                    }
                    Beat::Unreadable(error) => (
                        None,
                        format!("the final heartbeat could not read the chain ({error})"),
                    ),
                }
            };
            failures += 1;
            let wait = backoff(&self.policy, failures, retry_after);
            if past(until, wait) {
                return Err(ran_out(Some(&why)));
            }
            say(&format!(
                "upload: {why}; trying again in {}s",
                wait.as_secs().max(1)
            ));
            last = Some(why);
            std::thread::sleep(wait);
        }
    }

    /// The trace's `seal.json` and `baseline.json`, once both exist.
    fn seal_documents(&self) -> Option<(String, String)> {
        let trace = Path::new(&self.record.trace_dir);
        Some((
            read_document(&trace.join(SEAL_FILE))?,
            read_document(&trace.join(BASELINE_FILE))?,
        ))
    }

    /// `POST /v1/runs/{id}/seal` with the two documents: the run is over.
    /// Answers whether the server says the run itself is over.
    fn seal(&self, documents: &(String, String), timeout: Duration) -> Result<bool, ServerError> {
        let request = seal_request(&self.record, documents);
        self.server
            .json_timed::<StatusAnswer>(
                "POST",
                &format!("/v1/runs/{}/seal", self.record.run_id),
                Some(&request),
                timeout,
            )
            .map(|answer| answer.run_ended())
    }
}

/// The seal's body: the two documents, and the sealed-connection list when the finish wrote one
/// (D74). A list that is missing, empty, unreadable or over the bound is left out: the seal never
/// waits on it.
fn seal_request(record: &crate::upload::UploadRecord, documents: &(String, String)) -> SealRequest {
    SealRequest {
        entry_id: record.entry_id.clone(),
        seal_json: documents.0.clone(),
        baseline_json: documents.1.clone(),
        tunnels_jsonl: record
            .tunnels_file
            .as_deref()
            .and_then(|file| read_document(Path::new(file))),
    }
}

/// The largest document sent with the seal: the verifier's own bound on `seal.json`.
const MAX_DOCUMENT_BYTES: u64 = 16 * 1024 * 1024;

/// A non-empty document of at most [`MAX_DOCUMENT_BYTES`], or `None`.
fn read_document(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut text = String::new();
    file.take(MAX_DOCUMENT_BYTES + 1)
        .read_to_string(&mut text)
        .ok()?;
    (!text.is_empty() && text.len() as u64 <= MAX_DOCUMENT_BYTES).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The finish wrote the list: it goes with the seal, verbatim. No list (or an older record,
    /// which names none): the body is the one an older server has always taken.
    #[test]
    fn the_seal_carries_the_tunnel_list_when_it_exists() {
        let dir = tempfile::tempdir().unwrap();
        let documents = ("{\"seal\":1}".to_string(), "{\"baseline\":1}".to_string());
        let list = dir.path().join("tunnels-e1.jsonl");
        let text = "{\"format\":\"goalcup-tunnels-v1\",\"entry_id\":\"e1\"}\n{\"dropped\":0}\n";
        std::fs::write(&list, text).unwrap();
        let record = crate::upload::UploadRecord {
            tunnels_file: Some(list.display().to_string()),
            ..super::super::record_in(dir.path())
        };
        assert_eq!(
            seal_request(&record, &documents).tunnels_jsonl.as_deref(),
            Some(text)
        );

        std::fs::remove_file(&list).unwrap();
        let request = seal_request(&record, &documents);
        assert_eq!(request.tunnels_jsonl, None);
        let body = serde_json::to_value(&request).unwrap();
        assert!(body.get("tunnels_jsonl").is_none(), "{body}");

        let mut old = serde_json::to_value(super::super::record_in(dir.path())).unwrap();
        old.as_object_mut().unwrap().remove("tunnels_file");
        let old: crate::upload::UploadRecord = serde_json::from_value(old).unwrap();
        assert_eq!(old.tunnels_file, None);
    }
}
