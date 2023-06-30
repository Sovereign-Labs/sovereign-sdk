use borsh::{BorshDeserialize, BorshSerialize};
use sov_rollup_interface::zk::traits::{ValidityCondition, Zkvm};

/// A wrapper around a code commitment which implements borsh
#[derive(Clone, Debug)]
pub struct StoredCodeCommitment<Vm: Zkvm> {
    pub commitment: Vm::CodeCommitment,
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

/// A wrapper around a validity condition which implements borsh
#[derive(Clone, Debug)]
pub struct BorshValidityCondition<Cond: ValidityCondition> {
    pub condition: Cond,
}

impl<Cond: ValidityCondition> BorshSerialize for BorshValidityCondition<Cond> {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        bincode::serialize_into(writer, &self.condition)
            .expect("Serialization to vec is infallible");
        Ok(())
    }
}

impl<Cond: ValidityCondition> BorshDeserialize for BorshValidityCondition<Cond> {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let condition: Cond = bincode::deserialize_from(reader)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        Ok(Self { condition })
    }
}
