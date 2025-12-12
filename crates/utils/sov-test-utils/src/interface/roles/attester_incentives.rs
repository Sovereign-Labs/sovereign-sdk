use sov_modules_api::{Amount, Spec};

use super::{AsUser, TestUser};

/// A test attester.
#[derive(Debug, Clone)]
pub struct TestAttester<S: Spec> {
    /// The [`TestUser`] info of the attester.
    pub user_info: TestUser<S>,
    /// The amount of tokens bonded by the attester.
    pub bond: Amount,
    /// The next rollup height at which the attester is supposed to attest.
    pub slot_to_attest: u64,
}

/// The configuration to generate an attester.
pub struct TestAttesterConfig {
    /// The amount of tokens to bond at genesis.
    pub bond: Amount,
    /// Any additional (not bonded) balance that the bank should mint for the attester.
    pub free_balance: Amount,
}

impl<S: Spec> TestAttester<S> {
    /// Returns the rollup height at which the attester is supposed to attest.
    pub fn slot_to_attest(&self) -> u64 {
        self.slot_to_attest
    }

    /// Slashes the attester, making it unbonded.
    pub fn slash(&mut self) {
        self.bond = Amount::ZERO;
    }

    /// Generates a new attester with the given configuration.
    pub fn generate(config: TestAttesterConfig) -> Self {
        Self {
            user_info: TestUser::generate(config.free_balance),
            bond: config.bond,
            slot_to_attest: 1,
        }
    }
}

impl<S: Spec> AsUser<S> for TestAttester<S> {
    fn as_user(&self) -> &TestUser<S> {
        &self.user_info
    }

    fn as_user_mut(&mut self) -> &mut TestUser<S> {
        &mut self.user_info
    }
}

/// An unbonded test challenger.
#[derive(Debug, Clone)]
pub struct TestChallenger<S: Spec> {
    /// The [`TestUser`] info of the challenger.
    pub user_info: TestUser<S>,
}

impl<S: Spec> TestChallenger<S> {
    /// Generates a new challenger with the given balance.
    pub fn generate(balance: Amount) -> Self {
        Self {
            user_info: TestUser::generate(balance),
        }
    }
}

impl<S: Spec> AsUser<S> for TestChallenger<S> {
    fn as_user(&self) -> &TestUser<S> {
        &self.user_info
    }

    fn as_user_mut(&mut self) -> &mut TestUser<S> {
        &mut self.user_info
    }
}

/// A bonded test challenger.
#[derive(Debug, Clone)]
pub struct BondedTestChallenger<S: Spec> {
    /// The [`TestUser`] info of the challenger.
    pub user_info: TestUser<S>,
    /// The amount of tokens bonded by the challenger.
    pub bond: Amount,
}

impl<S: Spec> AsUser<S> for BondedTestChallenger<S> {
    fn as_user(&self) -> &TestUser<S> {
        &self.user_info
    }

    fn as_user_mut(&mut self) -> &mut TestUser<S> {
        &mut self.user_info
    }
}

impl<S: Spec> BondedTestChallenger<S> {
    /// Creates a new bonded challenger from a challenger and a bond amount. The bond amount is subtracted from the challenger's free balance.
    pub fn from_challenger(challenger: TestChallenger<S>, bond: Amount) -> Self {
        assert!(bond <= challenger.user_info.available_gas_balance);

        Self {
            user_info: TestUser {
                private_key: challenger.user_info.private_key().clone(),
                available_gas_balance: challenger
                    .user_info
                    .available_gas_balance
                    .checked_sub(bond)
                    .unwrap(),
                token_balances: Vec::new(),
                custom_credential_id: None,
            },
            bond,
        }
    }
}
