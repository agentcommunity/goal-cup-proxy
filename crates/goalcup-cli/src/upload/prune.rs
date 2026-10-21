//! Which chain parts may be deleted from this machine: the ones the server acknowledged long
//! enough ago.
//!
//! A 168-hour run with many agents records more than a disk holds, so a part the server has
//! acknowledged is deleted [`PRUNE_AFTER`] later, or at once under pressure: the parts alone are
//! past the run's reservation ([`crate::run::reserve::pressure`]), or the disk is short of space
//! whatever the chain's size, under 5 GiB free beyond the reserve or the reserve drawn down to
//! nothing ([`crate::run::reserve::short_of_disk`]). Everything here is a judgement on
//! names and numbers; the run's watch tick ([`crate::run::watch`]) owns the clock and does the
//! deleting.
//!
//! What is never deleted:
//!
//! - the last part, which the daemon is writing;
//! - the part that holds the last acknowledged record (a part is deletable only when it ends
//!   *before* the acknowledged offset), so [`super::uploader::local_link_before`] always finds
//!   the record the server's position names, and the verifier keeps the link to the first
//!   surviving record;
//! - any part behind one that is kept: deletion only ever takes a prefix of the parts, so the
//!   ones that remain are contiguous (`chain_parts` refuses a gap).

use goalcup_protocol::export::{ChainPart, chain_parts};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long a part is kept after it was acknowledged.
pub const PRUNE_AFTER: Duration = Duration::from_secs(2 * 60 * 60);

/// When this process first saw each part wholly acknowledged, by `(entry, part start)`.
///
/// Monotonic instants, so a wall-clock jump cannot make a part look older than it is. It lives
/// in memory: after a restart it is empty and nothing is deleted for [`PRUNE_AFTER`] (unless
/// the disk is under pressure).
#[derive(Debug, Default)]
pub struct AckAges(HashMap<(String, u64), Instant>);

/// The parts of `entry` that may be deleted now, oldest first.
///
/// A part qualifies when it is not the last part and ends before `acked_offset` (the bytes the
/// server holds, logical), and either it was first seen acknowledged [`PRUNE_AFTER`] or more
/// before `now`, or `pressure` says the disk needs the space. The answer is a prefix of
/// `parts`: the first part that does not qualify ends it.
pub fn deletable(
    entry: &str,
    parts: &[ChainPart],
    acked_offset: u64,
    ages: &mut AckAges,
    now: Instant,
    pressure: bool,
) -> Vec<PathBuf> {
    // An entry with no parts listed says nothing about its parts' ages.
    if parts.is_empty() {
        return Vec::new();
    }
    // The acknowledged parts: a prefix, and never the last part.
    let acknowledged: Vec<&ChainPart> = parts
        .split_last()
        .map(|(_, earlier)| earlier)
        .unwrap_or_default()
        .iter()
        .take_while(|part| part.start.saturating_add(part.len) < acked_offset)
        .collect();
    // A part that is gone (deleted before) or not acknowledged any more forgets its age.
    ages.0
        .retain(|(e, start), _| e != entry || acknowledged.iter().any(|part| part.start == *start));
    // Every acknowledged part starts its clock when first seen, even one that waits behind a
    // younger part: it must not start again when the ones before it are gone.
    let aged: Vec<Instant> = acknowledged
        .iter()
        .map(|part| *ages.0.entry((entry.to_string(), part.start)).or_insert(now))
        .collect();
    acknowledged
        .into_iter()
        .zip(aged)
        .take_while(|(_, since)| pressure || now.saturating_duration_since(*since) >= PRUNE_AFTER)
        .map(|(part, _)| part.path.clone())
        .collect()
}

/// The parts in `trace_dir` ([`chain_parts`]), listed again when a part vanishes between the
/// listing and its measurement: the watch deletes acknowledged parts while the uploader reads
/// the directory. A trace directory that is itself missing is still `NotFound`.
pub fn parts_now(trace_dir: &Path) -> std::io::Result<Vec<ChainPart>> {
    let mut last = None;
    for _ in 0..3 {
        match chain_parts(trace_dir) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => last = Some(error),
            other => return other,
        }
    }
    Err(last.unwrap_or_else(|| std::io::ErrorKind::NotFound.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: Duration = Duration::from_secs(3600);

    /// Four parts of 100 bytes each; the last (start 300) is the live one.
    fn parts() -> Vec<ChainPart> {
        (0..4)
            .map(|i| ChainPart {
                start: i * 100,
                len: 100,
                path: PathBuf::from(format!("part-{i}")),
            })
            .collect()
    }

    fn names(paths: Vec<PathBuf>) -> Vec<String> {
        paths
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
    }

    #[test]
    fn only_parts_acknowledged_two_hours_ago_are_deleted() {
        let (mut ages, t0) = (AckAges::default(), Instant::now());
        let parts = parts();
        // First seen acknowledged now: nothing is old enough.
        assert!(deletable("e", &parts, 250, &mut ages, t0, false).is_empty());
        // One second short of two hours, still not.
        let almost = t0 + 2 * HOUR - Duration::from_secs(1);
        assert!(deletable("e", &parts, 250, &mut ages, almost, false).is_empty());
        // At two hours the two parts wholly below 250 go; part 2 (200..300) is not acknowledged
        // to its end.
        let due = t0 + 2 * HOUR;
        assert_eq!(
            names(deletable("e", &parts, 250, &mut ages, due, false)),
            ["part-0", "part-1"]
        );
        // Part 2 is acknowledged to its end only later, and has its own two hours from then.
        assert_eq!(
            names(deletable("e", &parts, 301, &mut ages, due, false)),
            ["part-0", "part-1"]
        );
        assert_eq!(
            names(deletable(
                "e",
                &parts,
                301,
                &mut ages,
                due + 2 * HOUR,
                false
            )),
            ["part-0", "part-1", "part-2"]
        );
    }

    #[test]
    fn the_part_holding_the_last_acknowledged_record_is_kept() {
        let (mut ages, t0) = (AckAges::default(), Instant::now());
        let parts = parts();
        deletable("e", &parts, 300, &mut ages, t0, false);
        // The server holds exactly the first three parts: part 2 ends at the acknowledged
        // offset, its last record is the acknowledged one, so it stays.
        assert_eq!(
            names(deletable("e", &parts, 300, &mut ages, t0 + 10 * HOUR, true)),
            ["part-0", "part-1"]
        );
        // Nothing acknowledged: nothing goes, at any age, under any pressure.
        assert!(deletable("e", &parts, 0, &mut ages, t0 + 20 * HOUR, true).is_empty());
        assert!(deletable("e", &parts, 100, &mut ages, t0 + 20 * HOUR, true).is_empty());
    }

    #[test]
    fn the_live_part_is_never_deleted() {
        let (mut ages, t0) = (AckAges::default(), Instant::now());
        let parts = parts();
        // The server holds everything and more, the parts are ancient, the disk is full.
        deletable("e", &parts, u64::MAX, &mut ages, t0, true);
        let gone = names(deletable(
            "e",
            &parts,
            u64::MAX,
            &mut ages,
            t0 + 99 * HOUR,
            true,
        ));
        assert_eq!(gone, ["part-0", "part-1", "part-2"]);
        // A chain of one part, and of none.
        let one = &parts[..1];
        assert!(deletable("e", one, u64::MAX, &mut ages, t0 + 99 * HOUR, true).is_empty());
        assert!(deletable("e", &[], u64::MAX, &mut ages, t0, true).is_empty());
    }

    #[test]
    fn pressure_deletes_young_acknowledged_parts() {
        let (mut ages, t0) = (AckAges::default(), Instant::now());
        let parts = parts();
        // The very first time they are seen, under pressure.
        assert_eq!(
            names(deletable("e", &parts, 250, &mut ages, t0, true)),
            ["part-0", "part-1"]
        );
        // Without pressure the same parts are young.
        let mut fresh = AckAges::default();
        assert!(deletable("e", &parts, 250, &mut fresh, t0, false).is_empty());
    }

    #[test]
    fn deletion_takes_a_prefix_so_no_gap_is_left() {
        let (mut ages, t0) = (AckAges::default(), Instant::now());
        let parts = parts();
        // Part 1 is old but part 0 is not (its age was forgotten when the server's position
        // moved back): part 1 must not go alone.
        ages.0.insert(("e".to_string(), 100), t0);
        assert!(deletable("e", &parts, 250, &mut ages, t0 + 2 * HOUR, false).is_empty());
        // Both have been seen acknowledged since: they go together, in order.
        assert_eq!(
            names(deletable("e", &parts, 250, &mut ages, t0 + 4 * HOUR, false)),
            ["part-0", "part-1"]
        );
    }

    #[test]
    fn a_part_keeps_its_clock_when_the_ones_before_it_are_gone() {
        let (mut ages, t0) = (AckAges::default(), Instant::now());
        let parts = parts();
        // Parts 0 and 1 are seen together; the later call finds both due at once.
        deletable("e", &parts, 250, &mut ages, t0, false);
        let due = t0 + 2 * HOUR;
        // Only part 0 is deleted (one at a time); part 1 is still due when asked again.
        let first = deletable("e", &parts, 250, &mut ages, due, false);
        assert_eq!(names(first), ["part-0", "part-1"]);
        let after = &parts[1..];
        assert_eq!(
            names(deletable("e", after, 250, &mut ages, due, false)),
            ["part-1"]
        );
    }

    #[test]
    fn ages_are_kept_for_each_entry_and_forgotten_with_the_part() {
        let (mut ages, t0) = (AckAges::default(), Instant::now());
        let parts = parts();
        deletable("a", &parts, 250, &mut ages, t0, false);
        // Entry `b` has its own clock.
        assert!(deletable("b", &parts, 250, &mut ages, t0 + 2 * HOUR, false).is_empty());
        assert_eq!(
            deletable("a", &parts, 250, &mut ages, t0 + 2 * HOUR, false).len(),
            2
        );
        // Once the first two are gone, their ages are forgotten.
        deletable("a", &parts[2..], 250, &mut ages, t0 + 3 * HOUR, false);
        assert!(!ages.0.contains_key(&("a".to_string(), 0)));
    }

    #[test]
    fn parts_are_listed_again_not_failed_when_the_directory_is_missing_only() {
        let dir = tempfile::tempdir().unwrap();
        let missing = parts_now(&dir.path().join("nope")).unwrap_err();
        assert_eq!(missing.kind(), std::io::ErrorKind::NotFound);
        std::fs::write(dir.path().join("chain.jsonl"), b"ab\n").unwrap();
        std::fs::write(dir.path().join("chain.00000000000000000003.jsonl"), b"c\n").unwrap();
        let listed = parts_now(dir.path()).unwrap();
        assert_eq!(
            listed.iter().map(|p| (p.start, p.len)).collect::<Vec<_>>(),
            [(0, 3), (3, 2)]
        );
    }
}
