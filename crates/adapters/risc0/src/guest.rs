//! This module implements the `ZkvmGuest` trait for the RISC0 VM.

#[cfg(target_os = "zkvm")]
use risc0_zkvm::guest::env;
use serde::de::DeserializeOwned;
use sov_rollup_interface::zk::ZkvmGuest;

#[cfg(not(target_os = "zkvm"))]
pub(crate) mod hint_serde {
    use serde::de::DeserializeOwned;
    use serde::Serialize;

    /// Reader for bincode-serialized host hints.
    #[cfg_attr(not(feature = "bincode"), allow(dead_code))]
    #[derive(Default)]
    pub(crate) struct BincodeHints {
        values: std::io::Cursor<Vec<u8>>,
    }

    #[cfg_attr(not(feature = "bincode"), allow(dead_code))]
    impl BincodeHints {
        /// Creates a bincode hint reader from serialized bytes.
        pub(crate) fn new(hints: Vec<u8>) -> Self {
            Self {
                values: std::io::Cursor::new(hints),
            }
        }
    }

    /// Reader for RISC0 word-serialized host hints.
    #[cfg_attr(feature = "bincode", allow(dead_code))]
    #[derive(Default)]
    pub(crate) struct Risc0SerdeHints {
        values: Vec<u32>,
        position: usize,
    }

    #[cfg_attr(feature = "bincode", allow(dead_code))]
    impl Risc0SerdeHints {
        /// Creates a RISC0 serde hint reader from serialized words.
        pub(crate) fn new(hints: Vec<u32>) -> Self {
            Self {
                values: hints,
                position: 0,
            }
        }
    }

    impl risc0_zkvm::serde::WordRead for Risc0SerdeHints {
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

    /// Appends a bincode-serialized hint.
    #[cfg_attr(not(feature = "bincode"), allow(dead_code))]
    pub(crate) fn write_bincode_hint<T: Serialize>(env: &mut Vec<u8>, item: &T) {
        bincode::serialize_into(env, item).expect("Risc0 hint serialization is infallible");
    }

    /// Reads a bincode-serialized hint.
    #[cfg_attr(not(feature = "bincode"), allow(dead_code))]
    pub(crate) fn read_bincode_hint<T: DeserializeOwned>(hints: &mut BincodeHints) -> T {
        bincode::deserialize_from::<_, T>(&mut hints.values).expect("Deserialization failed")
    }

    /// Appends a RISC0 word-serialized hint.
    #[cfg_attr(feature = "bincode", allow(dead_code))]
    pub(crate) fn write_risc0_serde_hint<T: Serialize>(env: &mut Vec<u32>, item: &T) {
        let mut serializer = risc0_zkvm::serde::Serializer::new(env);
        item.serialize(&mut serializer)
            .expect("Risc0 hint serialization is infallible");
    }

    /// Reads a RISC0 word-serialized hint.
    #[cfg_attr(feature = "bincode", allow(dead_code))]
    pub(crate) fn read_risc0_serde_hint<T: DeserializeOwned>(hints: &mut Risc0SerdeHints) -> T {
        use risc0_zkvm::serde::Deserializer;

        T::deserialize(&mut Deserializer::new(hints)).unwrap()
    }
}

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
#[cfg(feature = "bincode")]
type Hints = hint_serde::BincodeHints;

#[cfg(not(target_os = "zkvm"))]
#[cfg(not(feature = "bincode"))]
type Hints = hint_serde::Risc0SerdeHints;

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
            hints: std::sync::Mutex::new(Hints::new(hints)),
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
            hints: std::sync::Mutex::new(Hints::new(hints)),
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

        hint_serde::read_bincode_hint(hints)
    }

    #[cfg(not(feature = "bincode"))]
    fn read_from_host<T: DeserializeOwned>(&self) -> T {
        use std::ops::DerefMut;

        let mut hints = self.hints.lock().unwrap();
        let hints = hints.deref_mut();
        hint_serde::read_risc0_serde_hint(hints)
    }

    fn commit<T: serde::Serialize>(&self, item: &T) {
        self.commits.lock().unwrap().extend_from_slice(
            &risc0_zkvm::serde::to_vec(item).expect("Serialization to vec is infallible"),
        );
    }
}

#[cfg(test)]
mod tests {
    use serde::de::DeserializeOwned;
    use serde::{Deserialize, Serialize};

    use super::hint_serde::{
        read_bincode_hint, read_risc0_serde_hint, write_bincode_hint, write_risc0_serde_hint,
        BincodeHints, Risc0SerdeHints,
    };

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct TestStruct {
        ints: Vec<i32>,
        string: String,
    }

    fn test_hints() -> Vec<TestStruct> {
        vec![
            TestStruct {
                ints: vec![1, 2, 3, 4, 5],
                string: "hello".to_string(),
            },
            TestStruct {
                ints: vec![10, -20, 30, 49, 50],
                string: "hello B".to_string(),
            },
        ]
    }

    fn check_round_trip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(
        hints: &[T],
    ) {
        let mut bincode_env = Vec::new();
        let mut risc0_env = Vec::new();

        for hint in hints {
            write_bincode_hint(&mut bincode_env, hint);
            write_risc0_serde_hint(&mut risc0_env, hint);
        }

        let mut bincode_reader = BincodeHints::new(bincode_env);
        let mut risc0_reader = Risc0SerdeHints::new(risc0_env);

        for hint in hints {
            assert_eq!(hint, &read_bincode_hint(&mut bincode_reader));
            assert_eq!(hint, &read_risc0_serde_hint(&mut risc0_reader));
        }
    }

    #[test]
    fn both_hint_serializers_round_trip_structs() {
        check_round_trip(&test_hints());
    }

    #[test]
    fn both_hint_serializers_round_trip_hex_types() {
        use sov_rollup_interface::common::{HexHash, HexString};

        check_round_trip(&[HexString::new(vec![0, 1, 2, 255])]);

        let hashes: [HexHash; 2] = [HexString::new([7; 32]), HexString::new([9; 32])];
        check_round_trip(&hashes);
    }
}
