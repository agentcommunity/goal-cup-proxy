//! The uploader: the entry's chain to the server in chunks, each acknowledged.
//!
//! The chain is one or more part files (`chain.jsonl`, then `chain.<start>.jsonl`; D56), and
//! every offset here is logical: the byte in the whole chain, wherever its part starts. A
//! chunk never crosses the end of a part. Parts the server acknowledged long ago may be
//! deleted ([`super::prune`]); the uploader only ever reads from the server's offset on.
//!
//! It reads the chain the daemon writes and never touches it: recording never waits on the
//! network, and a run whose server is away for a day loses nothing, because every byte is
//! still on disk and is sent when the server is back. What it sends and when
//! (docs/UPLOAD-API.md, and docs/SERVER-DESIGN.md "The run, step by step", step 5):
//!
//! - **What:** whole lines from where the server is (`offset`, `next_seq`, `head`), each
//!   chunk one zstd frame of at most [`MAX_CHUNK_BYTES`] compressed, built in a spool file
//!   beside the trace and streamed from it, so a chunk is never held in memory.
//! - **When:** at once when the run launches, then every [`Policy::interval`] (thirty
//!   minutes; the first interval after the opening chunk is a random share of it, so that
//!   runs started together do not send together), or sooner once [`Policy::eager_bytes`] of
//!   chain is waiting, and everything at the end.
//! - **Failures:** no failure ends a run's upload, and none stops the run (D64). No answer, a
//!   server error, a rate limit, the maintenance switch, a rejected chunk (`400`, `413`, `422`),
//!   `401`, `404`, `410` or any other answer is asked again after a backoff that starts at a
//!   minute and doubles to fifteen, with jitter, never sooner than the server's `Retry-After`:
//!   a bad deploy is fixed on the server, and the run must still be recording when it is. A
//!   `409` means the server is somewhere else: its position is read again and the upload
//!   continues from there. A local chain that does not continue the server's copy, and an
//!   acknowledgement that does not verify, are asked again the same way, the server's position
//!   read again first. A local error (the disk full, a part briefly missing, `acks.jsonl` not
//!   writable) is this machine's trouble of the moment: it is said and asked again after the
//!   same backoff. A `4xx` that repeats [`crate::run::REFUSALS_SAID_AFTER`] times is said once
//!   and kept for `goalcup status` until the server answers normally. After the stop the same
//!   asking goes on for the wrap-up ([`Policy::wrap_up`], an hour, counted through machine
//!   sleep): the final heartbeat, the rest of the chain and the seal. What is still unsent then
//!   is `goalcup upload`'s. Only that command, run by hand, gives up on `401`, `404` and `410`
//!   and on the two disagreements above ([`ServerError::ends_manual_upload`]), and says why.
//! - **Acknowledgements:** each one is checked (this entry, the chunk's last `seq` and head,
//!   a signature under the start receipt's key) and appended to the trace's `acks.jsonl`,
//!   where `goalcup-verify` reads it.

mod chunk;
mod finish;
mod heartbeat;
mod lifecycle;
mod local;
mod session;

pub use super::policy::{Policy, WrapUp, backoff};
pub use chunk::{Chunk, MAX_CHUNK_BYTES, Position, build_chunk, waiting_bytes};
pub use finish::Report;
pub use heartbeat::{EndedGate, LocalDeadline, REPLACED_REASON, stop_reason};
pub use lifecycle::{Closing, DrainHandles, Draining, Running, begin, drain, finish_advice};
pub use local::{
    is_sent, local_end, local_link_before, lock_entry, lock_path, note_repo_head,
    note_repo_head_unless_noted, repo_head,
};
pub use session::Uploader;

#[cfg(test)]
use super::UploadRecord;
use crate::CliError;
use std::path::Path;

fn io_error(path: &Path) -> impl Fn(std::io::Error) -> CliError + '_ {
    move |source| CliError::Io {
        path: path.display().to_string(),
        source,
    }
}

#[cfg(test)]
fn record_in(dir: &Path) -> UploadRecord {
    UploadRecord {
        run_id: "r0123456789abcdef".into(),
        entry_id: "e1".into(),
        server_url: String::new(),
        trace_dir: dir.join("trace").display().to_string(),
        official_start: String::new(),
        deadline_at: String::new(),
        workspace: String::new(),
    }
}
