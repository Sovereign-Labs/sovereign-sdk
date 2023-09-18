#[cfg(not(target_os = "zkvm"))]
use std::ops::DerefMut;

#[cfg(target_os = "zkvm")]
use risc0_zkvm::guest::env;
#[cfg(not(target_os = "zkvm"))]
use risc0_zkvm::serde::{Deserializer, WordRead};
use sov_rollup_interface::zk::{Zkvm, ZkvmGuest};

use crate::Risc0MethodId;

#[cfg(target_os = "zkvm")]
impl ZkvmGuest for Risc0Guest {
    fn read_from_host<T: serde::de::DeserializeOwned>(&self) -> T {
        env::read()
    }

    fn commit<T: serde::Serialize>(&self, item: &T) {
        env::commit(item);
    }
}

#[cfg(not(target_os = "zkvm"))]
#[derive(Default)]
struct Hints {
    values: Vec<u32>,
    position: usize,
}

#[cfg(not(target_os = "zkvm"))]
impl Hints {
    pub fn with_hints(hints: Vec<u32>) -> Self {
        Hints {
            values: hints,
            position: 0,
        }
    }
    pub fn remaining(&self) -> usize {
        self.values.len() - self.position
    }
}

#[cfg(not(target_os = "zkvm"))]
impl WordRead for Hints {
    fn read_words(&mut self, words: &mut [u32]) -> risc0_zkvm::serde::Result<()> {
        if self.remaining() < words.len() {
            return Err(risc0_zkvm::serde::Error::DeserializeUnexpectedEnd);
        }
        words.copy_from_slice(&self.values[self.position..self.position + words.len()]);
        self.position += words.len();
        Ok(())
    }

    fn read_padded_bytes(&mut self, bytes: &mut [u8]) -> risc0_zkvm::serde::Result<()> {
        let remaining_bytes: &[u8] = bytemuck::cast_slice(&self.values[self.position..]);
        if bytes.len() > remaining_bytes.len() {
            return Err(risc0_zkvm::serde::Error::DeserializeUnexpectedEnd);
        }
        bytes.copy_from_slice(&remaining_bytes[..bytes.len()]);
        self.position += bytes.len() / std::mem::size_of::<u32>();
        Ok(())
    }
}

#[derive(Default)]
pub struct Risc0Guest {
    #[cfg(not(target_os = "zkvm"))]
    hints: std::sync::Mutex<Hints>,
    #[cfg(not(target_os = "zkvm"))]
    commits: std::sync::Mutex<Vec<u32>>,
}

impl Risc0Guest {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(not(target_os = "zkvm"))]
    pub fn with_hints(hints: Vec<u32>) -> Self {
        Self {
            hints: std::sync::Mutex::new(Hints::with_hints(hints)),
            commits: Default::default(),
        }
    }
}

#[cfg(not(target_os = "zkvm"))]
impl ZkvmGuest for Risc0Guest {
    fn read_from_host<T: serde::de::DeserializeOwned>(&self) -> T {
        let mut hints = self.hints.lock().unwrap();
        let mut hints = hints.deref_mut();
        T::deserialize(&mut Deserializer::new(&mut hints)).unwrap()
    }

    fn commit<T: serde::Serialize>(&self, item: &T) {
        self.commits.lock().unwrap().extend_from_slice(
            &risc0_zkvm::serde::to_vec(item).expect("Serialization to vec is infallible"),
        );
    }
}

impl Zkvm for Risc0Guest {
    type CodeCommitment = Risc0MethodId;

    type Error = anyhow::Error;

    fn verify<'a>(
        _serialized_proof: &'a [u8],
        _code_commitment: &Self::CodeCommitment,
    ) -> Result<&'a [u8], Self::Error> {
        // Implement this method once risc0 supports recursion: issue #633
        todo!("Implement once risc0 supports recursion: https://github.com/Sovereign-Labs/sovereign-sdk/issues/633")
    }

    fn verify_and_extract_output<
        Add: sov_rollup_interface::RollupAddress,
        Da: sov_rollup_interface::da::DaSpec,
    >(
        _serialized_proof: &[u8],
        _code_commitment: &Self::CodeCommitment,
    ) -> Result<sov_rollup_interface::zk::StateTransition<Da, Add>, Self::Error> {
        todo!()
    }
}
