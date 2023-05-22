#[cfg(target_os = "zkvm")]
use risc0_zkvm::guest::env;
use sov_rollup_interface::zk::traits::{Zkvm, ZkvmGuest};

use crate::Risc0MethodId;

pub struct Risc0Guest;

#[cfg(target_os = "zkvm")]
impl ZkvmGuest for Risc0Guest {
    fn read_from_host<T: serde::de::DeserializeOwned>(&self) -> T {
        env::read()
    }
}

#[cfg(not(target_os = "zkvm"))]
impl ZkvmGuest for Risc0Guest {
    fn read_from_host<T: serde::de::DeserializeOwned>(&self) -> T {
        unimplemented!("This method should only be called in zkvm mode")
    }
}

impl Zkvm for Risc0Guest {
    type CodeCommitment = Risc0MethodId;

    type Error = anyhow::Error;

    fn verify<'a>(
        _serialized_proof: &'a [u8],
        _code_commitment: &Self::CodeCommitment,
    ) -> Result<&'a [u8], Self::Error> {
        // Implement this method once risc0 supports recursion
        todo!()
    }
}
