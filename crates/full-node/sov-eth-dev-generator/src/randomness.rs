use sov_transaction_generator::rng_utils::{get_random_bytes, randomize_buffer};

/// Manages randomness buffer for Arbitrary implementations
pub struct Randomness {
    pub randomness: Vec<u8>,
    pub remaining: usize,
    pub target_buffer_size: usize,
    pub salt: u128,
}

impl Randomness {
    pub fn new(salt: u128) -> Self {
        let randomness = get_random_bytes(100_000, salt);
        let remaining_randomness = randomness.len();
        Self {
            randomness,
            remaining: remaining_randomness,
            target_buffer_size: 100_000,
            salt,
        }
    }

    pub fn re_randomize(&mut self) {
        if self.randomness.len() < self.target_buffer_size {
            self.randomness = vec![0; self.target_buffer_size];
        }
        randomize_buffer(&mut self.randomness[..], self.salt);
        self.remaining = self.randomness.len();
        self.salt += 1;
    }

    pub fn offset(&self) -> usize {
        self.randomness.len() - self.remaining
    }

    pub fn has_enough(&self) -> bool {
        self.remaining > std::cmp::min(1000, self.target_buffer_size / 10)
    }

    pub fn increase_buffer_size(&mut self) {
        self.target_buffer_size *= 2;
    }

    pub fn update_remaining(&mut self, new_remaining: usize) {
        self.remaining = new_remaining;
    }
}
