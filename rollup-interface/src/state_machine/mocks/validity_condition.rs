use std::marker::PhantomData;

use anyhow::Error;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sha2::Digest;

use crate::zk::{ValidityCondition, ValidityConditionChecker};

/// A trivial test validity condition structure that only contains a boolean
#[derive(
    Debug,
    BorshDeserialize,
    BorshSerialize,
    Serialize,
    Deserialize,
    PartialEq,
    Clone,
    Copy,
    Default,
    Eq,
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

#[derive(BorshDeserialize, BorshSerialize, Debug)]
/// A mock validity condition checker that always evaluate to cond
pub struct MockValidityCondChecker<Cond: ValidityCondition> {
    phantom: PhantomData<Cond>,
}

impl ValidityConditionChecker<MockValidityCond> for MockValidityCondChecker<MockValidityCond> {
    type Error = Error;

    fn check(&mut self, condition: &MockValidityCond) -> Result<(), Self::Error> {
        if condition.is_valid {
            Ok(())
        } else {
            Err(anyhow::format_err!("Invalid mock validity condition"))
        }
    }
}

impl<Cond: ValidityCondition> MockValidityCondChecker<Cond> {
    /// Creates new test validity condition
    pub fn new() -> Self {
        Self {
            phantom: Default::default(),
        }
    }
}

impl<Cond: ValidityCondition> Default for MockValidityCondChecker<Cond> {
    fn default() -> Self {
        Self::new()
    }
}
