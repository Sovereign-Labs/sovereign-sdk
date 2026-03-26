//! Unit tests for the WindowV2 fixed-size bitfield nonce approach.
//!
//! These tests verify correctness of the improved window nonce implementation
//! that uses a fixed [u64; 16] array (1024 bits) instead of Vec<u8>.

use borsh::{BorshDeserialize, BorshSerialize};

const WINDOW_BITS: usize = 1024;
const WINDOW_U64S: usize = WINDOW_BITS / 64;
const ADVANCE_THRESHOLD: usize = WINDOW_BITS * 3 / 4;
const ADVANCE_TARGET: usize = WINDOW_BITS / 2;

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
struct WindowV2State {
    start: u64,
    bits: [u64; WINDOW_U64S],
}

impl Default for WindowV2State {
    fn default() -> Self {
        Self {
            start: 0,
            bits: [0u64; WINDOW_U64S],
        }
    }
}

impl WindowV2State {
    #[inline]
    fn check(&self, nonce: u64) -> Result<(), &'static str> {
        if nonce < self.start {
            return Err("nonce too old");
        }
        let delta_u64 = nonce - self.start;
        if delta_u64 >= WINDOW_BITS as u64 {
            return Err("nonce too far ahead");
        }
        let delta = delta_u64 as usize;
        let word_idx = delta / 64;
        let bit_idx = delta % 64;
        if self.bits[word_idx] & (1u64 << bit_idx) != 0 {
            return Err("duplicate nonce");
        }
        Ok(())
    }

    #[inline]
    fn mark(&mut self, nonce: u64) -> Result<(), &'static str> {
        if nonce < self.start {
            return Err("nonce too old");
        }
        let delta_u64 = nonce - self.start;
        if delta_u64 >= WINDOW_BITS as u64 {
            return Err("nonce too far ahead");
        }
        let mut delta = delta_u64 as usize;

        if delta >= ADVANCE_THRESHOLD {
            let advance = delta - ADVANCE_TARGET;
            self.shift_right(advance);
            self.start = self
                .start
                .checked_add(advance as u64)
                .ok_or("nonce space exhausted")?;
            delta = (nonce - self.start) as usize;
        }

        let word_idx = delta / 64;
        let bit_idx = delta % 64;
        if self.bits[word_idx] & (1u64 << bit_idx) != 0 {
            return Err("duplicate nonce");
        }
        self.bits[word_idx] |= 1u64 << bit_idx;
        Ok(())
    }

    #[inline]
    fn shift_right(&mut self, n: usize) {
        if n == 0 {
            return;
        }
        if n >= WINDOW_BITS {
            self.bits = [0u64; WINDOW_U64S];
            return;
        }
        let word_shift = n / 64;
        let bit_shift = n % 64;
        if bit_shift == 0 {
            for i in 0..WINDOW_U64S {
                let src = i + word_shift;
                self.bits[i] = if src < WINDOW_U64S { self.bits[src] } else { 0 };
            }
        } else {
            for i in 0..WINDOW_U64S {
                let src = i + word_shift;
                let lo = if src < WINDOW_U64S { self.bits[src] } else { 0 };
                let hi = if src + 1 < WINDOW_U64S {
                    self.bits[src + 1]
                } else {
                    0
                };
                self.bits[i] = (lo >> bit_shift) | (hi << (64 - bit_shift));
            }
        }
    }
}

#[test]
fn fresh_account_first_nonce() {
    let mut state = WindowV2State::default();
    assert!(state.check(0).is_ok());
    state.mark(0).unwrap();
    assert_eq!(state.check(0), Err("duplicate nonce"));
}

#[test]
fn sequential_nonces_0_to_100() {
    let mut state = WindowV2State::default();
    for i in 0u64..100 {
        assert!(state.check(i).is_ok(), "nonce {i} should be valid");
        state.mark(i).unwrap();
    }
    for i in 0u64..100 {
        assert_eq!(
            state.check(i),
            Err("duplicate nonce"),
            "nonce {i} should be duplicate"
        );
    }
}

#[test]
fn out_of_order_within_window() {
    let mut state = WindowV2State::default();
    let nonces = [50u64, 10, 90, 30, 70, 5, 99, 1];
    for &n in &nonces {
        assert!(state.check(n).is_ok(), "nonce {n} should be valid");
        state.mark(n).unwrap();
    }
    for &n in &nonces {
        assert_eq!(
            state.check(n),
            Err("duplicate nonce"),
            "nonce {n} should be duplicate"
        );
    }
    assert!(state.check(0).is_ok());
    assert!(state.check(2).is_ok());
    assert!(state.check(51).is_ok());
}

#[test]
fn nonce_too_old() {
    let mut state = WindowV2State::default();
    state.start = 100;
    assert_eq!(state.check(99), Err("nonce too old"));
    assert_eq!(state.check(0), Err("nonce too old"));
    assert!(state.check(100).is_ok());
}

#[test]
fn nonce_too_far_ahead() {
    let state = WindowV2State::default();
    assert_eq!(state.check(WINDOW_BITS as u64), Err("nonce too far ahead"));
    assert_eq!(
        state.check(WINDOW_BITS as u64 + 1000),
        Err("nonce too far ahead")
    );
    assert!(state.check(WINDOW_BITS as u64 - 1).is_ok());
}

#[test]
fn window_auto_advance() {
    let mut state = WindowV2State::default();
    for i in 0u64..10 {
        state.mark(i).unwrap();
    }
    assert_eq!(state.start, 0);

    let high_nonce = ADVANCE_THRESHOLD as u64 + 10;
    state.mark(high_nonce).unwrap();

    assert!(state.start > 0, "window should have advanced");
    assert_eq!(state.check(0), Err("nonce too old"));
    assert_eq!(state.check(high_nonce), Err("duplicate nonce"));
    assert!(state.check(high_nonce + 1).is_ok());
}

#[test]
fn full_window_saturation() {
    let mut state = WindowV2State::default();
    for i in 0u64..(WINDOW_BITS as u64) {
        state.mark(i).unwrap();
    }
    // After filling nonces 0..1024, the window has auto-advanced.
    // All marked nonces still within the window should be duplicates.
    for i in 0u64..(WINDOW_BITS as u64) {
        if i >= state.start {
            let delta = (i - state.start) as usize;
            if delta < WINDOW_BITS {
                assert_eq!(
                    state.check(i),
                    Err("duplicate nonce"),
                    "nonce {i} should be duplicate after saturation (start={})",
                    state.start
                );
            }
        }
    }
    // Nonces beyond the marked range should be valid (not yet used)
    let next_unused = WINDOW_BITS as u64;
    if next_unused >= state.start && (next_unused - state.start) < WINDOW_BITS as u64 {
        assert!(
            state.check(next_unused).is_ok(),
            "nonce {next_unused} was never marked, should be valid"
        );
    }
}

#[test]
fn shift_right_basic() {
    let mut state = WindowV2State::default();
    state.bits[0] = 1;
    state.shift_right(1);
    assert_eq!(state.bits[0], 0);

    let mut state = WindowV2State::default();
    state.bits[0] = 2;
    state.shift_right(1);
    assert_eq!(state.bits[0], 1);
}

#[test]
fn shift_right_cross_word_boundary() {
    let mut state = WindowV2State::default();
    state.bits[1] = 1;
    state.shift_right(1);
    assert_eq!(state.bits[0], 1u64 << 63);
    assert_eq!(state.bits[1], 0);
}

#[test]
fn shift_right_exact_word() {
    let mut state = WindowV2State::default();
    state.bits[1] = 0xDEAD_BEEF_CAFE_BABE;
    state.shift_right(64);
    assert_eq!(state.bits[0], 0xDEAD_BEEF_CAFE_BABE);
    assert_eq!(state.bits[1], 0);
}

#[test]
fn shift_right_full_window() {
    let mut state = WindowV2State::default();
    state.bits = [0xFFFF_FFFF_FFFF_FFFF; WINDOW_U64S];
    state.shift_right(WINDOW_BITS);
    assert_eq!(state.bits, [0u64; WINDOW_U64S]);
}

#[test]
fn shift_right_zero() {
    let mut state = WindowV2State::default();
    state.bits[0] = 42;
    state.bits[5] = 99;
    let original = state.clone();
    state.shift_right(0);
    assert_eq!(state, original);
}

#[test]
fn shift_right_multi_word_with_bit_offset() {
    let mut state = WindowV2State::default();
    state.bits[3] = 1;
    state.shift_right(65);
    assert_eq!(state.bits[1], 1u64 << 63);
    assert_eq!(state.bits[3], 0);
}

#[test]
fn borsh_roundtrip() {
    let mut state = WindowV2State::default();
    for i in [0u64, 5, 10, 100, 500, 1023] {
        state.mark(i).unwrap();
    }
    let bytes = borsh::to_vec(&state).unwrap();
    let deserialized: WindowV2State = BorshDeserialize::try_from_slice(&bytes).unwrap();
    assert_eq!(state, deserialized);
}

#[test]
fn borsh_serialized_size_is_constant() {
    let empty = WindowV2State::default();
    let empty_size = borsh::to_vec(&empty).unwrap().len();

    let mut full = WindowV2State::default();
    for i in 0..WINDOW_BITS as u64 {
        full.mark(i).unwrap();
    }
    let full_size = borsh::to_vec(&full).unwrap().len();

    assert_eq!(empty_size, full_size, "serialized size must be constant");
    assert_eq!(empty_size, 8 + 128, "expected 136 bytes (u64 + [u64; 16])");
}

#[test]
fn large_nonce_values() {
    let mut state = WindowV2State {
        start: u64::MAX - WINDOW_BITS as u64,
        bits: [0u64; WINDOW_U64S],
    };
    let nonce = u64::MAX - 1;
    assert!(state.check(nonce).is_ok());
    state.mark(nonce).unwrap();
    assert_eq!(state.check(nonce), Err("duplicate nonce"));
}

#[test]
fn dos_resistance_no_unbounded_growth() {
    let state = WindowV2State::default();
    assert_eq!(state.check(1_000_000), Err("nonce too far ahead"));
    assert_eq!(state.check(u64::MAX), Err("nonce too far ahead"));
}

#[test]
fn window_advance_preserves_recent_nonces() {
    let mut state = WindowV2State::default();
    for i in 400u64..512 {
        state.mark(i).unwrap();
    }
    state.mark(900).unwrap();

    for i in 400u64..512 {
        if i >= state.start {
            assert_eq!(
                state.check(i),
                Err("duplicate nonce"),
                "nonce {i} should still be tracked after advance (start={})",
                state.start
            );
        }
    }
}

#[test]
fn mark_beyond_window_rejects() {
    let mut state = WindowV2State::default();
    state.mark(0).unwrap();
    // mark() now enforces the same policy as check(): nonces beyond
    // the window are rejected, preventing attackers from forcing
    // window advancement to expire replay protection bits.
    assert_eq!(
        state.mark(WINDOW_BITS as u64 + 100),
        Err("nonce too far ahead")
    );
    // Window should NOT have advanced
    assert_eq!(state.start, 0);
}

#[test]
fn replay_after_advance_is_prevented() {
    let mut state = WindowV2State::default();
    // Mark nonce 5
    state.mark(5).unwrap();
    assert_eq!(state.check(5), Err("duplicate nonce"));

    // Use a nonce in the upper quarter to trigger auto-advance
    let high = ADVANCE_THRESHOLD as u64 + 10;
    state.mark(high).unwrap();

    // Nonce 5 should now be "too old", NOT "available for replay"
    assert_eq!(state.check(5), Err("nonce too old"));
    // The high nonce should be marked
    assert_eq!(state.check(high), Err("duplicate nonce"));
}

#[test]
fn advance_near_u64_max_uses_checked_add() {
    let mut state = WindowV2State {
        start: u64::MAX - WINDOW_BITS as u64 + 1,
        bits: [0u64; WINDOW_U64S],
    };
    // Nonce in upper quarter should trigger advance
    let nonce = u64::MAX - 1;
    let delta = (nonce - state.start) as usize;
    assert!(delta >= ADVANCE_THRESHOLD, "should trigger advance");
    // This should succeed (advance fits within u64)
    state.mark(nonce).unwrap();
    assert_eq!(state.check(nonce), Err("duplicate nonce"));
}

#[test]
fn storage_size_comparison_table() {
    use std::collections::{BTreeMap, HashSet};

    fn make_tx_hash(seed: u64) -> [u8; 32] {
        let mut hash = [0u8; 32];
        hash[..8].copy_from_slice(&seed.to_le_bytes());
        hash
    }

    #[derive(Default, BorshSerialize)]
    struct GenState {
        buckets: BTreeMap<u64, HashSet<[u8; 32]>>,
    }

    #[derive(Default, BorshSerialize)]
    struct V1State {
        start: u64,
        bits: Vec<u8>,
    }

    eprintln!("\n=== Storage Size Comparison (Borsh serialized bytes per account) ===");
    eprintln!(
        "| {:>8} | {:>10} | {:>12} | {:>12} | {:>12} |",
        "Tx Count", "Nonce", "Generation", "Window v1", "Window v2"
    );
    eprintln!("|----------|------------|--------------|--------------|--------------|");

    for &tx_count in &[0u64, 10, 100, 500, 1000, 1700] {
        let nonce_size = borsh::to_vec(&tx_count).unwrap().len();

        let mut gen = GenState::default();
        for i in 0..tx_count.min(1700) {
            gen.buckets
                .entry(i % 5000)
                .or_default()
                .insert(make_tx_hash(i));
        }
        let gen_size = borsh::to_vec(&gen).unwrap().len();

        let mut v1 = V1State::default();
        for i in 0..tx_count {
            let delta = (i - v1.start) as usize;
            v1.bits.resize((delta / 8 + 1).max(v1.bits.len()), 0);
            v1.bits[delta / 8] |= 1 << (delta % 8);
        }
        let v1_size = borsh::to_vec(&v1).unwrap().len();

        let mut v2 = WindowV2State::default();
        for i in 0..tx_count.min(WINDOW_BITS as u64) {
            v2.mark(i).unwrap();
        }
        let v2_size = borsh::to_vec(&v2).unwrap().len();

        eprintln!(
            "| {:>8} | {:>8} B | {:>10} B | {:>10} B | {:>10} B |",
            tx_count, nonce_size, gen_size, v1_size, v2_size
        );
    }
}
