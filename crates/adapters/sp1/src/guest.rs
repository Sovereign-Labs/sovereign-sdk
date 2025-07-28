//! Implementation of the SP1 guest for the Sovereign ZkvmGuest trait.

use sov_rollup_interface::zk::ZkvmGuest;

#[derive(Default)]
/// SP1 Guest implementation.
pub struct SP1Guest {
    #[cfg(not(target_os = "zkvm"))]
    hints: std::sync::Mutex<Vec<Vec<u8>>>,
    #[cfg(not(target_os = "zkvm"))]
    commits: std::sync::Mutex<Vec<u8>>,
}

#[cfg(target_os = "zkvm")]
impl ZkvmGuest for SP1Guest {
    type Verifier = crate::SP1Verifier;
    fn read_from_host<T: serde::de::DeserializeOwned>(&self) -> T {
        sp1_lib::io::read()
    }

    fn commit<T: serde::Serialize>(&self, item: &T) {
        sp1_lib::io::commit(item)
    }
}

impl SP1Guest {
    /// Constructs a new SP1 Guest
    pub fn new() -> Self {
        Self::default()
    }

    /// Constructs a new SP1 Guest with the provided hints.
    ///
    /// This function is only available outside of SP1's environment.
    #[cfg(not(target_os = "zkvm"))]
    pub fn with_hints(hints: Vec<Vec<u8>>) -> Self {
        Self {
            hints: std::sync::Mutex::new(hints),
            commits: Default::default(),
        }
    }
}

#[cfg(not(target_os = "zkvm"))]
impl ZkvmGuest for SP1Guest {
    type Verifier = crate::SP1Verifier;
    fn read_from_host<T: serde::de::DeserializeOwned>(&self) -> T {
        let mut hints = self.hints.lock().unwrap();
        let hint = hints.remove(0);

        bincode::deserialize(&hint).expect("Hints must be bincode serializable")
    }

    fn commit<T: serde::Serialize>(&self, item: &T) {
        self.commits.lock().unwrap().extend_from_slice(
            &bincode::serialize(item).expect("Hints must be bincode serializable"),
        );
    }
}
