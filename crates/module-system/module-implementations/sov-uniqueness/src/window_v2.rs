use borsh::{BorshDeserialize, BorshSerialize};
use sov_modules_api::{CredentialId, Spec, StateAccessor, StateReader};
use sov_state::User;

use crate::Uniqueness;

/// Default window size: 16 u64 words = 1024 bits = 128 bytes.
/// Supports tracking 1024 concurrent in-flight nonces per account.
pub(crate) const DEFAULT_WINDOW_WORDS: usize = 16;

/// Stack-allocated, fixed-size sliding window for nonce uniqueness tracking.
///
/// `N` is the number of `u64` words in the bitfield. The window tracks
/// `N * 64` nonces. The recommended default is [`DEFAULT_WINDOW_WORDS`] (16),
/// giving a 1024-nonce window in 136 bytes of storage.
///
/// The window accepts nonces in `[start, start + N*64)`. Nonces below `start`
/// are rejected as "too old"; nonces at or above `start + N*64` are rejected
/// as "too far ahead". When a nonce lands in the upper quarter, the window
/// automatically slides forward to re-center it at the midpoint.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub(crate) struct WindowNonceState<const N: usize = DEFAULT_WINDOW_WORDS> {
    /// The lowest nonce tracked by the window.
    start: u64,
    /// Fixed-size bitfield. Bit `i` represents nonce `start + i`.
    bits: [u64; N],
}

impl<const N: usize> Default for WindowNonceState<N> {
    fn default() -> Self {
        Self {
            start: 0,
            bits: [0u64; N],
        }
    }
}

impl<const N: usize> WindowNonceState<N> {
    const WINDOW_BITS: usize = N * 64;
    const ADVANCE_THRESHOLD: usize = Self::WINDOW_BITS * 3 / 4;
    const ADVANCE_TARGET: usize = Self::WINDOW_BITS / 2;

    /// Returns the start of the current window (lowest accepted nonce).
    #[inline]
    pub(crate) fn start(&self) -> u64 {
        self.start
    }

    /// Check if a nonce is valid (not duplicate, not too old, not too far ahead).
    /// This is the read-only validation path — no state mutation.
    #[inline]
    pub(crate) fn check(&self, nonce: u64) -> anyhow::Result<()> {
        anyhow::ensure!(
            nonce >= self.start,
            "Tx outdated: expected nonce >= {}, got {nonce}",
            self.start,
        );
        // Compare as u64 BEFORE casting to usize to prevent truncation on 32-bit targets.
        let delta_u64 = nonce - self.start;
        anyhow::ensure!(
            delta_u64 < Self::WINDOW_BITS as u64,
            "Tx nonce too far ahead: max accepted is {}, got {nonce}",
            self.start + Self::WINDOW_BITS as u64 - 1,
        );
        let delta = delta_u64 as usize; // safe: verified < WINDOW_BITS
        let word_idx = delta / 64;
        let bit_idx = delta % 64;
        anyhow::ensure!(
            self.bits[word_idx] & (1u64 << bit_idx) == 0,
            "Tx duplicate for nonce: {nonce}"
        );
        Ok(())
    }

    /// Mark a nonce as used. Enforces the same acceptance policy as [`Self::check`]:
    /// nonces outside `[start, start + N*64)` are always rejected.
    #[inline]
    pub(crate) fn mark(&mut self, nonce: u64) -> anyhow::Result<()> {
        anyhow::ensure!(
            nonce >= self.start,
            "Tx outdated: expected nonce >= {}, got {nonce}",
            self.start,
        );
        let delta_u64 = nonce - self.start;
        anyhow::ensure!(
            delta_u64 < Self::WINDOW_BITS as u64,
            "Tx nonce too far ahead: max accepted is {}, got {nonce}",
            self.start + Self::WINDOW_BITS as u64 - 1,
        );
        let mut delta = delta_u64 as usize;

        // Auto-advance when nonce is in the upper quarter of the window.
        // This slides the window forward to maintain headroom for future nonces
        // while preserving ~N*32 bits of lookback for recent nonces.
        if delta >= Self::ADVANCE_THRESHOLD {
            let advance = delta - Self::ADVANCE_TARGET;
            self.shift_right(advance);
            self.start = self.start.checked_add(advance as u64).ok_or_else(|| {
                anyhow::anyhow!(
                    "Nonce space exhausted for window starting at {}",
                    self.start
                )
            })?;
            delta = (nonce - self.start) as usize;
        }

        let word_idx = delta / 64;
        let bit_idx = delta % 64;
        anyhow::ensure!(
            self.bits[word_idx] & (1u64 << bit_idx) == 0,
            "Tx duplicate for nonce: {nonce}",
        );
        self.bits[word_idx] |= 1u64 << bit_idx;
        Ok(())
    }

    /// Shift the entire bitfield right by `n` bits (drops the lowest `n` bits,
    /// advancing the window). O(N) with no allocation.
    ///
    /// Forward iteration is safe because we always read from `src >= i`,
    /// so source words are never overwritten before they are read.
    #[inline]
    fn shift_right(&mut self, n: usize) {
        if n == 0 {
            return;
        }
        if n >= Self::WINDOW_BITS {
            self.bits = [0u64; N];
            return;
        }

        let word_shift = n / 64;
        let bit_shift = n % 64;

        if bit_shift == 0 {
            for i in 0..N {
                let src = i + word_shift;
                self.bits[i] = if src < N { self.bits[src] } else { 0 };
            }
        } else {
            for i in 0..N {
                let src = i + word_shift;
                let lo = if src < N { self.bits[src] } else { 0 };
                let hi = if src + 1 < N { self.bits[src + 1] } else { 0 };
                self.bits[i] = (lo >> bit_shift) | (hi << (64 - bit_shift));
            }
        }
    }
}

impl<S: Spec> Uniqueness<S> {
    pub(crate) fn check_window_v2_uniqueness(
        &self,
        credential_id: &CredentialId,
        nonce: u64,
        state: &mut impl StateReader<User>,
    ) -> anyhow::Result<()> {
        let window = self
            .window_v2
            .get(credential_id, state)?
            .unwrap_or_default();
        window.check(nonce)
    }

    pub(crate) fn mark_window_v2_tx_attempted(
        &mut self,
        credential_id: &CredentialId,
        nonce: u64,
        state: &mut impl StateAccessor,
    ) -> anyhow::Result<()> {
        let mut window = self
            .window_v2
            .get(credential_id, state)?
            .unwrap_or_default();
        window.mark(nonce)?;
        self.window_v2.set(credential_id, &window, state)?;
        Ok(())
    }
}
