use sov_modules_api::capabilities::UniquenessData;
use sov_modules_api::{CredentialId, Spec, StateAccessor, StateReader, TxHash};
use sov_state::User;

use crate::Uniqueness;

impl<S: Spec> Uniqueness<S> {
    /// Checks the provided uniqueness number.
    /// ## Note
    /// This method should perform all the checks required to ensure that the execution of
    /// [`Self::mark_tx_attempted`] will succeed.
    ///
    /// # Errors
    /// May return an error if state access fails (e.g if we run out of gas) or if an overflow occurs (in the `check_generation_uniqueness` case).
    pub fn check_uniqueness(
        &self,
        credential_id: &CredentialId,
        transaction_uniqueness: UniquenessData,
        transaction_hash: TxHash,
        state: &mut impl StateReader<User>,
    ) -> anyhow::Result<()> {
        match transaction_uniqueness {
            UniquenessData::Nonce(nonce) => {
                self.check_nonce_uniqueness(credential_id, nonce, state)
            }
            UniquenessData::Generation(generation) => {
                self.check_generation_uniqueness(credential_id, generation, transaction_hash, state)
            }
        }
    }

    /// Marks a transaction as attempted, ensuring that future attempts at execution will fail.
    ///
    /// # Errors
    /// May return an error if state access fails (e.g if we run out of gas) or if an overflow occurs (in the `check_generation_uniqueness` case).
    pub fn mark_tx_attempted(
        &mut self,
        credential_id: &CredentialId,
        transaction_generation: UniquenessData,
        transaction_hash: TxHash,
        state: &mut impl StateAccessor,
    ) -> anyhow::Result<()> {
        match transaction_generation {
            UniquenessData::Nonce(_) => self.mark_nonce_tx_attempted(credential_id, state),
            UniquenessData::Generation(generation) => self.mark_generational_tx_attempted(
                credential_id,
                generation,
                transaction_hash,
                state,
            ),
        }
    }
}
