//! Support for `capture_failure_campaign.rs`: only what the storage and accounting campaign adds
//! to `m2_vertical_support` and to `capture_campaign_support` (the status waits, process helpers
//! and evidence `Row` both campaigns share). Nothing here edits or shadows those modules, and
//! nothing reaches the capture path except the real binaries and the filesystem.
//!
//! - the one filesystem fault the campaign injects: a second hard link to `chain.jsonl`, created
//!   and removed outside the trace directory ([`Alias`]);
//! - readers of the chain's lines and bytes ([`line_at`], [`bytes_through`]).

use super::m2_vertical_support::Chain;
use goalcup_protocol::envelope::Envelope;
use std::path::{Path, PathBuf};

/// A second hard link to the live `chain.jsonl`, held outside the trace directory. While it
/// exists the chain file has two names, which the append capability refuses at its next
/// validation; the capability never writes through an aliased file. Dropping it removes the
/// name, and so does [`Alias::remove`], once the test has seen the refusal.
pub struct Alias {
    path: PathBuf,
    present: bool,
}

impl Alias {
    pub fn create(chain: &Path, beside: &Path) -> Alias {
        let path = beside.join("chain-alias");
        std::fs::hard_link(chain, &path).expect("a hard link to the live chain");
        Alias {
            path,
            present: true,
        }
    }

    pub fn links(&self) -> u64 {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(&self.path).map_or(0, |m| m.nlink())
    }

    /// Remove the name. A test body calls this and wants it to succeed.
    pub fn remove(&mut self) {
        if self.present {
            std::fs::remove_file(&self.path).expect("the alias");
            self.present = false;
        }
    }
}

impl Drop for Alias {
    // Best effort and never a panic: a panic here while a test is already unwinding would abort
    // before `Vertical::drop` ends the daemon and the workload.
    fn drop(&mut self) {
        if self.present
            && let Err(error) = std::fs::remove_file(&self.path)
        {
            eprintln!(
                "could not remove the chain alias {}: {error}",
                self.path.display()
            );
        }
        self.present = false;
    }
}

/// The `link.hash` of chain line `seq`, and the chain bytes through the end of that line.
pub fn line_at(chain: &Chain, seq: u64) -> Option<(String, u64)> {
    let mut through = 0u64;
    for raw in chain.bytes.split_inclusive(|b| *b == b'\n') {
        through += raw.len() as u64;
        if !raw.ends_with(b"\n") {
            return None;
        }
        let envelope: Envelope = serde_json::from_slice(raw).ok()?;
        if envelope.link.seq == seq {
            return Some((envelope.link.hash, through));
        }
    }
    None
}

/// Chain bytes through the last complete line holding a record for which `pick` is true.
pub fn bytes_through(
    chain: &Chain,
    pick: impl Fn(&goalcup_protocol::capture::CaptureRecord) -> bool,
) -> u64 {
    let mut through = 0u64;
    let mut found = 0u64;
    for (raw, line) in chain
        .bytes
        .split_inclusive(|b| *b == b'\n')
        .zip(chain.lines.iter())
    {
        through += raw.len() as u64;
        if line.record.as_ref().is_some_and(&pick) {
            found = through;
        }
    }
    found
}
