//! The uploader (PRD-06): sends every part file to the server in small chunks soon after it is
//! written, tells the server the written counts in a heartbeat, and frees local disk once the
//! server holds the bytes. It runs on two plain threads with a blocking client, never on the
//! recorder's runtime, and never ends the process.
//!
//! Present: the seams ([`ServerWord`] S10, [`spawn`], [`UploadCtx`], [`SealFacts`] S18,
//! `upload.json` S9), the timing rules ([`policy`]), the client, the chunk cut ([`chunk`]) and the
//! heartbeat ([`beat`]). The threads come in slice 3; until then [`spawn`] starts none and says
//! so plainly in `upload.json`.

pub mod beat;
pub mod chunk;
pub mod client;
pub mod policy;
pub mod state;

pub use policy::WrapUp;

use crate::cli::{self, ServerJson};
use crate::pipeline::{EntryTotals, Mono, Recorder};
use crate::watchdog::SealReason;
use serde::{Deserialize, Serialize};
use state::{LastError, State, UploadJson};
use std::path::PathBuf;
use std::time::SystemTime;
use tokio::sync::watch;

/// The last heartbeat answer (seam S10). The recorder forwards it to the watchdog as the `server`
/// control line; the watchdog resets its deadline timer from `seconds_left` and seals with reason
/// `server_stop` on `stop`. The beat publishes it with `send_replace`, which keeps the value while
/// no receiver exists, so a subscriber that comes later still sees the last word.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerWord {
    pub stop: bool,
    pub reason: Option<String>,
    pub seconds_left: Option<u64>,
    /// Boot time, taken before the heartbeat call, so a wall clock set back never lengthens the
    /// deadline (PRD-17 S7).
    pub asked_at: Mono,
}

/// What the uploader needs to start (seam S18).
pub struct UploadCtx {
    /// `$GOALCUP_HOME`, for the queue of older entries.
    pub home: PathBuf,
    /// `runs/<run>/`, with `server.json` and the run credential (PRD-10).
    pub run_dir: PathBuf,
    /// The entry folder: `upload.json`, `upload.lock`, `rec/`.
    pub entry_dir: PathBuf,
    pub run: String,
    pub entry: String,
    /// None in null mode.
    pub recorder: Option<Recorder>,
    /// `GOALCUP_KEEP_PARTS` from the start facts: nothing is pruned.
    pub keep: bool,
}

/// What the seal hands over (seam S18).
#[derive(Clone, Debug)]
pub struct SealFacts {
    pub reason: SealReason,
    pub workspace_head: Option<String>,
    pub totals: EntryTotals,
}

/// The running uploader.
pub struct UploadHandle {
    word: watch::Sender<Option<ServerWord>>,
}

/// Starts the uploader for the entry in `ctx`. Never panics and never fails: an unwritable folder
/// is ignored.
pub fn spawn(ctx: UploadCtx) -> UploadHandle {
    let (word, _) = watch::channel(None);
    let mut json = UploadJson::new(ctx.run, ctx.entry);
    let server = cli::read_json::<ServerJson>(&cli::server_json(&ctx.run_dir));
    match server.and_then(|s| s.server) {
        None => {
            json.state = State::Local;
            json.keep = true;
        }
        Some(url) => {
            json.server_url = Some(url);
            json.keep = ctx.keep;
            json.state = State::Waiting;
            json.last_error = Some(LastError::new(
                SystemTime::now(),
                policy::Class::Internal,
                None,
                "the sender is not in this build yet; nothing is sent",
            ));
        }
    }
    let _ = state::write(&ctx.entry_dir, &json);
    UploadHandle { word }
}

impl UploadHandle {
    /// The last heartbeat answer; None before the first one.
    pub fn server_word(&self) -> watch::Receiver<Option<ServerWord>> {
        self.word.subscribe()
    }

    /// The seal is durable: send what is left within `wrap_up`. Returns at once in this slice.
    pub fn finish(self, wrap_up: WrapUp, seal: SealFacts) {
        let _ = (wrap_up, seal);
    }
}
