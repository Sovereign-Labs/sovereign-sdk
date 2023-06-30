use anyhow::Result;
use borsh::{BorshDeserialize, BorshSerialize};
use sov_rollup_interface::zk::traits::Zkvm;
use sov_state::WorkingSet;

use crate::ChainState;
pub struct Config<Vm: Zkvm> {
    /// A code commitment to be used for verifying proofs
    commitment_to_allowed_verifier_method: Vm::CodeCommitment,
}

/// A wrapper around a code commitment which implements borsh
#[derive(Clone, Debug)]
pub struct StoredCodeCommitment<Vm: Zkvm> {
    commitment: Vm::CodeCommitment,
}

impl<Vm: Zkvm> BorshSerialize for StoredCodeCommitment<Vm> {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        bincode::serialize_into(writer, &self.commitment)
            .expect("Serialization to vec is infallible");
        Ok(())
    }
}

impl<Vm: Zkvm> BorshDeserialize for StoredCodeCommitment<Vm> {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let commitment: Vm::CodeCommitment = bincode::deserialize_from(reader)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        Ok(Self { commitment })
    }
}

impl<C: sov_modules_api::Context> ChainState<C> {
    pub(crate) fn init_module(
        &self,
        _config: &<Self as sov_modules_api::Module>::Config,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        self.slot_height.set(&0, working_set);
        Ok(())
    }
}
