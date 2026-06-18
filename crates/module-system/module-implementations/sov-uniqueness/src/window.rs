use borsh::{BorshDeserialize, BorshSerialize};
use serde::Serialize;
use sov_modules_api::{macros::config_value, CredentialId, Spec, StateAccessor, StateReader};
use sov_state::User;
use std::collections::VecDeque;

const PAST_TRANSACTIONS_WINDOW: u64 = {
    let window = config_value!("PAST_TRANSACTIONS_WINDOW");
    assert!(
        window > 7u64,
        "PAST_TRANSACTIONS_WINDOW must be at least  8"
    );
    assert!(
        window < 2u64.pow(16),
        "PAST_TRANSACTIONS_WINDOW must be less than 65536"
    );
    window
};

/// A window of seen nonces.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Default, Serialize)]
pub struct Window {
    /// The nonce at which the window starts. Always a multiple of 8 so that entries in the bits array stay aligned.
    /// (Otherwise, we would have to iterate the array and shift each entry when we adjust the window)
    start_nonce: u64,
    /// A bitmap representing PAST_TRANSACTIONS_WINDOW nonces above (and including) the start nonce.
    /// Each entry contains a 1 if that nonce has been seen, 0 otherwise.
    ///
    /// For example, if the start nonce is 10 then index `0` represents nonce `10`.
    bits: BitMap,
}

impl Window {
    /// A test-only constructor that allows creating a window from a tuple of start nonce and bits.
    pub fn test_only_from_tuple((start_nonce, bits): (u64, Vec<u8>)) -> Self {
        Self {
            start_nonce,
            bits: BitMap(VecDeque::from(bits)),
        }
    }
}

// Cases:
// nonce = 8, start = 0, PAST_TRANSACTIONS_WINDOW = 7; unaligned = r. Round up to 8
// nonce = 8, start = 0, PAST_TRANSACTIONS_WINDOW = 8; unaligned = 0. Stay the same
impl Window {
    /// Adds a nonce to the set of seen nonces, adjusting the window if necessary.
    ///
    /// Safety: will panic if `nonce` < `self.start_nonce`.
    fn add_nonce(&mut self, nonce: u64) {
        let old_start_nonce = self.start_nonce;

        // The current window is start_nonce..(start_nonce + PAST_TRANSACTIONS_WINDOW);
        // That means the max offset we can have without adjusting the window is PAST_TRANSACTIONS_WINDOW - 1.
        let new_start_nonce = if nonce >= self.start_nonce.saturating_add(PAST_TRANSACTIONS_WINDOW)
        {
            // In this branch, we're adjusting the window.
            let unaligned_start_nonce = nonce - (PAST_TRANSACTIONS_WINDOW - 1);
            // Safety: PAST_TRANSACTIONS_WINDOW is greater than 7, so unaligned_start_nonce + 7 <= u64::MAX
            let new_start_nonce = (unaligned_start_nonce + 7) & 0xFFFF_FFFF_FFFF_FFF8; // round up to multiple of 8

            // Drop the bytes that are no longer in the window.
            let bytes_to_drop = (new_start_nonce - old_start_nonce) / 8;
            let bytes_to_drop = std::cmp::min(self.bits.0.len() as u64, bytes_to_drop);
            self.bits.0.drain(0..bytes_to_drop as usize);
            new_start_nonce
        } else {
            old_start_nonce
        };

        // Safety: new_start_nonce is always less than or equal to nonce, so nonce - new_start_nonce is non-negative.
        let offset = nonce
            .checked_sub(new_start_nonce)
            .expect("nonce - new_start_nonce is negative. This is a bug.");
        self.bits.set_bit(
            offset
                .try_into()
                .expect("offset is too large. This is a bug."),
        );
        self.start_nonce = new_start_nonce;
    }

    /// Checks if a nonce has been seen.
    fn has_seen_nonce(&self, nonce: u64) -> bool {
        // If the nonce is below the window, assume we've seen it.
        let Some(offset) = nonce.checked_sub(self.start_nonce) else {
            return true;
        };

        // If the nonce is more than usize::MAX above the start nonce, we definitely haven't seen it.
        let Ok(offset) = offset.try_into() else {
            return false;
        };
        // Otherwise, check the bitmap
        self.bits.get_bit(offset)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Default, Serialize)]
pub struct BitMap(VecDeque<u8>);

impl BitMap {
    /// Returns the bit at the given offset
    fn get_bit(&self, index: usize) -> bool {
        let byte_index = index / 8;
        let bit_index = index % 8;
        let Some(&byte) = self.0.get(byte_index) else {
            return false;
        };
        byte & (1 << bit_index) != 0
    }

    /// Sets the bit at the given offset.
    fn set_bit(&mut self, index: usize) {
        assert!(
            index < PAST_TRANSACTIONS_WINDOW as usize,
            "Index out of bounds: {index} > {PAST_TRANSACTIONS_WINDOW}"
        );
        // Resize if necessessary. This is only required if the index is currently out of bounds.
        if index >= self.0.len() * 8 {
            self.0.resize((index / 8) + 1, 0);
        }
        let byte_index = index / 8;
        let bit_index = index % 8;
        self.0[byte_index] |= 1 << bit_index;
    }
}

use crate::Uniqueness;
impl<S: Spec> Uniqueness<S> {
    pub(crate) fn check_window_uniqueness(
        &self,
        credential_id: &CredentialId,
        nonce: u64,
        state: &mut impl StateReader<User>,
    ) -> anyhow::Result<()> {
        let window = self.window.get(credential_id, state)?.unwrap_or_default();
        let start = window.start_nonce;

        anyhow::ensure!(
	    nonce >= start,
	    "Tx outdated for credential id: {credential_id}, expected at least: {start}, but found: {nonce}");

        anyhow::ensure!(
            !window.has_seen_nonce(nonce),
            "Tx duplicate for credential id: {credential_id}, with nonce: {nonce}"
        );

        Ok(())
    }

    pub(crate) fn mark_window_tx_attempted(
        &mut self,
        credential_id: &CredentialId,
        nonce: u64,
        state: &mut impl StateAccessor,
    ) -> anyhow::Result<()> {
        let mut window = self.window.get(credential_id, state)?.unwrap_or_default();

        assert!(nonce >= window.start_nonce, "Tx is being marked as attempted despite having a consumed nonce {nonce}. This is a bug.");

        window.add_nonce(nonce);

        self.window.set(credential_id, &window, state)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use crate::window::{BitMap, Window, PAST_TRANSACTIONS_WINDOW};

    #[test]
    fn test_get_bit() {
        let bitmap = BitMap(VecDeque::from([0b10101010, 0b10]));

        for i in 0..=10 {
            assert_eq!(bitmap.get_bit(i), i % 2 == 1);
        }
        for i in 11..32 {
            assert!(!bitmap.get_bit(i));
        }
    }

    // Make a permutation of the keys 0..256
    fn permute_key(i: usize) -> usize {
        (i * 37 + 3) % 256
    }

    #[test]
    fn test_get_and_set() {
        let mut bitmap = BitMap(VecDeque::new());

        for i in 0..256 {
            let key = permute_key(i);
            bitmap.set_bit(key);

            // Check that all set keys are set
            for i in 0..i {
                let key = permute_key(i);
                assert!(bitmap.get_bit(key));
            }
            // Check that all unset keys are still unset
            for j in (i + 1)..256 {
                let key = permute_key(j);
                assert!(!bitmap.get_bit(key));
            }
        }
    }

    #[test]
    fn test_adjust_window() {
        let mut window = Window::default();

        assert!(!window.has_seen_nonce(0));

        window.add_nonce(0);
        assert_eq!(window.start_nonce, 0);
        assert!(window.has_seen_nonce(0));
        assert!(!window.has_seen_nonce(1));

        window.add_nonce(PAST_TRANSACTIONS_WINDOW - 1);
        assert_eq!(window.start_nonce, 0);
        assert!(window.has_seen_nonce(PAST_TRANSACTIONS_WINDOW - 1));
        assert!(!window.has_seen_nonce(1));
        assert!(window.has_seen_nonce(0));
        assert!(!window.has_seen_nonce(PAST_TRANSACTIONS_WINDOW));

        window.add_nonce(PAST_TRANSACTIONS_WINDOW);
        assert_eq!(window.start_nonce, 8);
        assert!(window.has_seen_nonce(1)); // Below the window, so we've "seen" it
        assert!(window.has_seen_nonce(7)); // Below the window, so we've "seen" it
        assert!(!window.has_seen_nonce(8)); // In the window and unseen.
        assert!(window.has_seen_nonce(PAST_TRANSACTIONS_WINDOW - 1)); // Ir the window and see
        assert!(window.has_seen_nonce(PAST_TRANSACTIONS_WINDOW)); // In the window and see

        window.add_nonce(43);

        window.add_nonce(PAST_TRANSACTIONS_WINDOW + 32);

        assert!(window.has_seen_nonce(43));
        assert!(window.has_seen_nonce(PAST_TRANSACTIONS_WINDOW + 32));
        assert!(!window.has_seen_nonce(44));

        window.add_nonce(u64::MAX - 1);
        assert!(window.has_seen_nonce(u64::MAX - 1));
        assert!(!window.has_seen_nonce(u64::MAX));

        let mut window = Window::default();
        window.add_nonce(u64::MAX);
        assert!(window.has_seen_nonce(u64::MAX));
        assert!(!window.has_seen_nonce(u64::MAX - 1));
    }
}
