//! The one secret comparison both the daemon (control bearer, ingest token) and the CLI (proxy
//! credential) make. It lives here because neither may link the other (D3) and both link this crate.

/// Whether two secrets are equal, in time that does not depend on how much of the second
/// one was right.
///
/// A `==` on `&str` stops at the first differing byte, so the time it takes to refuse
/// tells a caller how long a prefix it guessed — and a 64-character hex token guessed one
/// character at a time is 1,024 tries rather than 16^64. The loop below runs over the
/// longer of the two and folds every byte, so only the length of what the caller sent
/// leaks, which the caller already knows.
pub fn constant_time_eq(presented: &[u8], expected: &[u8]) -> bool {
    let mut difference: u8 = 0;
    for i in 0..presented.len().max(expected.len()) {
        // Out of range on either side is a guaranteed difference: 0 against 1.
        let a = presented.get(i).copied().unwrap_or(0);
        let b = expected.get(i).copied().unwrap_or(1);
        difference |= a ^ b;
        // The optimiser is free to notice that a non-zero fold can never become zero again
        // and to break out of this loop early — which would put the timing back exactly
        // where the loop was written to take it from. `black_box` is the barrier that says
        // the accumulator is observed on every iteration.
        difference = std::hint::black_box(difference);
    }
    difference == 0 && presented.len() == expected.len()
}

#[cfg(test)]
mod tests {
    use super::constant_time_eq;

    #[test]
    fn constant_time_eq_compares_whole_inputs() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"secreT"));
        assert!(!constant_time_eq(b"secret", b"secret-longer"));
        assert!(!constant_time_eq(b"", b"x"));
        // A zero byte past the end of the shorter input still differs.
        assert!(!constant_time_eq(b"ab", b"ab\0"));
    }
}
