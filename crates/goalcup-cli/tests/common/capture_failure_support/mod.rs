//! Support for `capture_failure_campaign.rs`: only what the storage and accounting campaign adds
//! to `m2_vertical_support` and to `capture_campaign_support` (the status waits, process helpers
//! and evidence `Row` both campaigns share). Nothing here edits or shadows those modules, and
//! nothing reaches the capture path except the real binaries and the filesystem.
//!
//! - the one filesystem fault the campaign injects: a second hard link to `chain.jsonl`, created
//!   and removed outside the trace directory ([`Alias`]);
//! - readers of the chain's lines and bytes ([`line_at`], [`launch_baseline_bytes`]).

use super::m2_vertical_support::Chain;
use goalcup_protocol::capture::{CaptureRecord, CaptureSource, StreamKey};
use goalcup_protocol::envelope::{Envelope, EventBody};
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

/// The bytes of what the run itself writes before the harness runs: the Baseline link, the goal
/// stream (its `Open`, segments and `End`, and the goal producer's `ProducerClosed`) and the
/// `Launch` record. Summed line by line, not taken as a chain prefix: the daemon appends `Launch`
/// only after it has hashed the harness binaries (D53), and the harness is already running and
/// chaining its own traffic and native-file records by then. How many land before `Launch` is a
/// race (a few KiB on macOS, tens to over a hundred KiB on a Linux debug build hashing a 6.6 MB
/// interpreter), and none of it is baseline.
pub fn launch_baseline_bytes(chain: &Chain) -> u64 {
    let envelopes: Vec<(u64, Envelope)> = chain
        .bytes
        .split_inclusive(|b| *b == b'\n')
        .filter(|raw| raw.ends_with(b"\n"))
        .map(|raw| {
            let envelope = serde_json::from_slice(raw).expect("a complete chain line");
            (raw.len() as u64, envelope)
        })
        .collect();
    let goal: Option<StreamKey> = envelopes.iter().find_map(|(_, e)| match &e.body {
        EventBody::Capture {
            record:
                CaptureRecord::Open {
                    stream,
                    source: CaptureSource::Goal,
                    ..
                },
        } => Some(stream.clone()),
        _ => None,
    });
    envelopes
        .iter()
        .filter(|(_, e)| match &e.body {
            EventBody::Baseline { .. } => true,
            EventBody::Capture { record } => match record {
                CaptureRecord::Launch { .. } => true,
                CaptureRecord::Open { stream, .. }
                | CaptureRecord::Segment { stream, .. }
                | CaptureRecord::End { stream, .. } => goal.as_ref() == Some(stream),
                CaptureRecord::ProducerClosed { producer, .. } => {
                    goal.as_ref().is_some_and(|g| &g.producer == producer)
                }
                _ => false,
            },
            _ => false,
        })
        .map(|(len, _)| len)
        .sum()
}
