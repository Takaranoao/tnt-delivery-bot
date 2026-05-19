/// FNV-1a 64-bit hash. Deterministic across runs/processes.
pub fn fnv1a64(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Slot in 0..n for a token, de-synchronized by its stored random.
pub fn slot(token: &str, rand: i64, n: u64) -> u64 {
    debug_assert!(n != 0);
    (fnv1a64(token).wrapping_add(rand as u64)) % n
}

/// True on exactly one tick per n ticks for a given token.
pub fn is_due(tick: u64, token: &str, rand: i64, n: u64) -> bool {
    n != 0 && tick % n == slot(token, rand, n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_deterministic() {
        assert_eq!(fnv1a64("3abc128856"), fnv1a64("3abc128856"));
        assert_ne!(fnv1a64("a"), fnv1a64("b"));
    }

    #[test]
    fn slot_in_range() {
        for n in [1u64, 12, 60] {
            let s = slot("3abc128856", 42, n);
            assert!(s < n);
        }
    }

    #[test]
    fn due_exactly_once_per_period() {
        let n = 12;
        let token = "3abc128856";
        let rand = 12345;
        let hits: usize = (0..n).filter(|t| is_due(*t, token, rand, n)).count();
        assert_eq!(hits, 1, "exactly one due tick per period");
    }

    #[test]
    fn random_desynchronizes_tokens() {
        // Different rand values generally shift the slot.
        let a = slot("same-token", 1, 12);
        let b = slot("same-token", 7, 12);
        // Not a hard guarantee for all inputs, but must hold for these.
        assert_ne!(a, b);
    }
}
