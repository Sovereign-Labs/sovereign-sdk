use sov_modules_api::{CheckUniquenessError, CredentialId, Spec, StateAccessor, StateReader};
use sov_state::User;

use crate::Uniqueness;
impl<S: Spec> Uniqueness<S> {
    pub(crate) fn check_nonce_uniqueness(
        &self,
        credential_id: &CredentialId,
        transaction_nonce: u64,
        state: &mut impl StateReader<User>,
    ) -> Result<(), CheckUniquenessError> {
        let nonce = self
            .nonces
            .get(credential_id, state)
            .map_err(|e| CheckUniquenessError::Internal(e.to_string()))?
            .unwrap_or_default();

        if nonce != transaction_nonce {
            return Err(CheckUniquenessError::BadNonce {
                expected_nonce: nonce,
                provided_nonce: transaction_nonce,
            });
        }

        Ok(())
    }

    pub(crate) fn check_nonce_uniqueness_allow_nonconsecutive(
        &self,
        credential_id: &CredentialId,
        transaction_nonce: u64,
        state: &mut impl StateReader<User>,
    ) -> Result<(), CheckUniquenessError> {
        let nonce = self
            .nonces
            .get(credential_id, state)
            .map_err(|e| CheckUniquenessError::Internal(e.to_string()))?
            .unwrap_or_default();

        if nonce > transaction_nonce {
            return Err(CheckUniquenessError::NonceTooLow {
                minimum_nonce: nonce,
                provided_nonce: transaction_nonce,
            });
        }

        Ok(())
    }

    pub(crate) fn mark_nonce_tx_attempted(
        &mut self,
        credential_id: &CredentialId,
        state: &mut impl StateAccessor,
    ) -> anyhow::Result<()> {
        let nonce = self.nonces.get(credential_id, state)?.unwrap_or_default();

        let nonce = nonce.checked_add(1).expect("Maximum nonce value reached");

        self.nonces.set(credential_id, &nonce, state)?;

        Ok(())
    }
}
