//! The soft memory guard (PRD-02 section 3): its size, and the one admission rule.

/// The soft guard's ceiling, and its size when memory cannot be read.
pub(crate) const CEILING: u64 = 512 << 20;
/// Its floor, on a machine with little memory.
const FLOOR: u64 = 128 << 20;

/// The soft guard in bytes: 512 MiB, or one eighth of physical memory if that is smaller, at
/// least 128 MiB. 512 MiB when memory cannot be read. Under `test-hooks` only,
/// `GOALCUP_TEST_GUARD_MB` (MiB) sets it.
pub(crate) fn soft_guard() -> u64 {
    #[cfg(feature = "test-hooks")]
    if let Some(mb) = std::env::var("GOALCUP_TEST_GUARD_MB")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
    {
        return mb << 20;
    }
    match physical_memory() {
        Some(ram) => (ram / 8).clamp(FLOOR, CEILING),
        None => CEILING,
    }
}

fn physical_memory() -> Option<u64> {
    // SAFETY: sysconf only reads a system value.
    let pages = unsafe { libc::sysconf(libc::_SC_PHYS_PAGES) };
    // SAFETY: as above.
    let size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if pages <= 0 || size <= 0 {
        return None;
    }
    (pages as u64).checked_mul(size as u64)
}

/// Whether a DATA offer of `len` held bytes is admitted with `held` bytes held under guard
/// `soft`. A stream not in a gap admits up to the guard; a stream in a gap admits only under three
/// quarters of it, so one stream does not flap into many tiny gaps. Either way `held` never goes
/// past the guard, so an offer larger than the guard is dropped behind a GAP and the wire alone
/// never reaches the stall mark (2 x soft). OPEN, END, GAP and EVENT never ask.
pub(crate) fn admits(held: u64, len: u64, in_gap: bool, soft: u64) -> bool {
    held.saturating_add(len) <= soft && (!in_gap || held < low_mark(soft))
}

/// Three quarters of the soft guard: where a stream in a gap admits again, and what `room()`
/// answers against, so native reading never pushes the wire into the band where it drops.
pub(crate) fn low_mark(soft: u64) -> u64 {
    soft / 4 * 3
}
