//! This module implements the `ZkvmGuest` trait for the RISC0 VM.

#[cfg(target_os = "zkvm")]
use risc0_zkvm::guest::env;
use serde::de::DeserializeOwned;
use sov_rollup_interface::zk::ZkvmGuest;

#[cfg(target_os = "zkvm")]
impl ZkvmGuest for Risc0Guest {
    type Verifier = crate::Risc0Verifier;

    #[cfg(feature = "bincode")]
    fn read_from_host<T: DeserializeOwned>(&self) -> T {
        let mut len: u32 = 0;
        env::read_slice(std::slice::from_mut(&mut len));

        let mut bytes = vec![0u8; len as usize];
        env::read_slice(&mut bytes);

        bincode::deserialize(&bytes).unwrap()
    }

    #[cfg(not(feature = "bincode"))]
    fn read_from_host<T: DeserializeOwned>(&self) -> T {
        env::read()
    }

    fn commit<T: serde::Serialize>(&self, item: &T) {
        env::commit(item);
    }
}

#[cfg(not(target_os = "zkvm"))]
#[derive(Default)]
struct Hints {
    #[cfg(feature = "bincode")]
    values: std::io::Cursor<Vec<u8>>,
    #[cfg(not(feature = "bincode"))]
    values: Vec<u32>,
    #[cfg(not(feature = "bincode"))]
    position: usize,
}

#[cfg(not(target_os = "zkvm"))]
impl Hints {
    #[cfg(feature = "bincode")]
    pub fn with_hints(hints: Vec<u8>) -> Self {
        Hints {
            values: std::io::Cursor::new(hints),
        }
    }

    #[cfg(not(feature = "bincode"))]
    pub fn with_hints(hints: Vec<u32>) -> Self {
        Hints {
            values: hints,
            position: 0,
        }
    }
}

#[cfg(not(feature = "bincode"))]
#[cfg(not(target_os = "zkvm"))]
impl risc0_zkvm::serde::WordRead for Hints {
    fn read_words(&mut self, words: &mut [u32]) -> risc0_zkvm::serde::Result<()> {
        if let Some(slice) = self.values.get(self.position..self.position + words.len()) {
            words.copy_from_slice(slice);
            self.position += words.len();
            Ok(())
        } else {
            Err(risc0_zkvm::serde::Error::DeserializeUnexpectedEnd)
        }
    }

    fn read_padded_bytes(&mut self, bytes: &mut [u8]) -> risc0_zkvm::serde::Result<()> {
        use risc0_zkvm::align_up;
        use risc0_zkvm_platform::WORD_SIZE;

        let remaining_bytes: &[u8] = bytemuck::cast_slice(&self.values[self.position..]);
        if bytes.len() > remaining_bytes.len() {
            return Err(risc0_zkvm::serde::Error::DeserializeUnexpectedEnd);
        }
        bytes.copy_from_slice(&remaining_bytes[..bytes.len()]);
        self.position += align_up(bytes.len(), WORD_SIZE) / WORD_SIZE;
        Ok(())
    }
}

/// A guest for the RISC0 VM. When running in the Risc0 environment, this struct
/// implements the `ZkvmGuest` trait in terms of Risc0's env::read and env::commit functions.
/// When running in any other environment, the struct uses interior mutability to emulate
/// the same functionality.
#[derive(Default)]
pub struct Risc0Guest {
    #[cfg(not(target_os = "zkvm"))]
    hints: std::sync::Mutex<Hints>,
    #[cfg(not(target_os = "zkvm"))]
    commits: std::sync::Mutex<Vec<u32>>,
}

impl Risc0Guest {
    /// Constructs a new Risc0 Guest
    pub fn new() -> Self {
        Self::default()
    }

    /// Constructs a new Risc0 Guest with the provided hints.
    ///
    /// This function is only available outside Risc0's environment.
    #[cfg(not(target_os = "zkvm"))]
    #[cfg(not(feature = "bincode"))]
    pub fn with_hints(hints: Vec<u32>) -> Self {
        Self {
            hints: std::sync::Mutex::new(Hints::with_hints(hints)),
            commits: Default::default(),
        }
    }

    /// Constructs a new Risc0 Guest with the provided hints.
    ///
    /// This function is only available outside Risc0's environment.
    #[cfg(not(target_os = "zkvm"))]
    #[cfg(feature = "bincode")]
    pub fn with_hints(hints: Vec<u8>) -> Self {
        Self {
            hints: std::sync::Mutex::new(Hints::with_hints(hints)),
            commits: Default::default(),
        }
    }
}

#[cfg(not(target_os = "zkvm"))]
impl ZkvmGuest for Risc0Guest {
    type Verifier = crate::Risc0Verifier;

    #[cfg(feature = "bincode")]
    fn read_from_host<T: DeserializeOwned>(&self) -> T {
        use std::ops::DerefMut;

        let mut hints = self.hints.lock().unwrap();
        let hints = hints.deref_mut();

        bincode::deserialize_from::<_, T>(&mut hints.values).expect("Deserialization failed")
    }

    #[cfg(not(feature = "bincode"))]
    fn read_from_host<T: DeserializeOwned>(&self) -> T {
        use std::ops::DerefMut;

        use risc0_zkvm::serde::Deserializer;

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
