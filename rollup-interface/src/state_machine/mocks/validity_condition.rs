use std::marker::PhantomData;

use anyhow::Error;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sha2::Digest;

use crate::zk::{ValidityCondition, ValidityConditionChecker};

/// A trivial test validity condition structure that only contains a boolean
#[derive(
    Debug, BorshDeserialize, BorshSerialize, Serialize, Deserialize, PartialEq, Clone, Copy, Default,
)]
pub struct MockValidityCond {
    /// The associated validity condition field. If it is true, the validity condition is verified
    pub is_valid: bool,
}

impl ValidityCondition for MockValidityCond {
    type Error = Error;
    fn combine<H: Digest>(&self, rhs: Self) -> Result<Self, Self::Error> {
        Ok(MockValidityCond {
            is_valid: self.is_valid & rhs.is_valid,
        })
    }
}

#[derive(Debug, BorshDeserialize, BorshSerialize)]
/// A mock validity condition checker that always evaluate to cond
pub struct TestValidityCondChecker<MockValidityCond> {
    phantom: PhantomData<MockValidityCond>,
}

impl ValidityConditionChecker<MockValidityCond> for TestValidityCondChecker<MockValidityCond> {
    type Error = Error;

    fn check(&mut self, condition: &MockValidityCond) -> Result<(), Self::Error> {
        if condition.is_valid {
            Ok(())
        } else {
            Err(anyhow::format_err!("Invalid mock validity condition"))
        }
    }
}
