use sov_modules_api::{Amount, Spec};

use super::{AsUser, TestUser};

/// A representation of a prover to be used in tests.
#[derive(Debug, Clone)]
pub struct TestProver<S: Spec> {
    /// The [`TestUser`] info of the prover.
    pub user_info: TestUser<S>,
    /// The amount of tokens bonded by the prover.
    pub bond: Amount,
}

impl<S: Spec> AsUser<S> for TestProver<S> {
    fn as_user(&self) -> &TestUser<S> {
        &self.user_info
    }

    fn as_user_mut(&mut self) -> &mut TestUser<S> {
        &mut self.user_info
    }
}

impl<S: Spec> TestProver<S> {
    /// Generates a new prover with the given config.
    pub fn generate(config: TestProverConfig) -> Self {
        Self {
            user_info: TestUser::generate(config.additional_balance),
            bond: config.bond,
        }
    }
}

/// A configuration for a test prover.
pub struct TestProverConfig {
    /// Any additional (not bonded) balance that the bank should mint for the prover.
    pub additional_balance: Amount,
    /// The amount of tokens to bond at genesis. These tokens will be minted by the bank.
    pub bond: Amount,
}
