use sov_modules_api::{CredentialId, Spec, StateAccessor, StateReader, StateWriter};
use sov_state::User;

use crate::{Account, Accounts};

impl<S: Spec> Accounts<S> {
    /// Resolve the sender's public key to an address.
    ///
    /// If the credential is already registered (by a prior `InsertCredentialId`
    /// or prior auto-registration), returns the stored address.
    ///
    /// Otherwise immediately auto-registers the credential to `default_address`
    /// (typically `Address::from(credential_id)`) and returns that. Note that
    /// this auto-registration produces a **different** address than
    /// [`CallMessage::InsertCredentialId`](crate::CallMessage::InsertCredentialId)
    /// would — the latter derives `hash(credential_id || registering_sender)`
    /// via [`derive_address_for_new_credential`](crate::derive_address_for_new_credential).
    /// Callers that need a specific on-chain address for a credential (e.g. a
    /// multisig) must pre-register via `InsertCredentialId` before the
    /// credential's first tx.
    pub fn resolve_sender_address<ST: StateAccessor>(
        &mut self,
        default_address: &S::Address,
        credential_id: &CredentialId,
        state: &mut ST,
    ) -> Result<S::Address, <ST as StateWriter<User>>::Error> {
        let maybe_address = self.accounts.get(credential_id, state)?.map(|a| a.addr);

        match maybe_address {
            Some(address) => Ok(address),
            None => {
                // 1. Add the credential -> account mapping
                let new_account = Account {
                    addr: *default_address,
                };
                self.accounts.set(credential_id, &new_account, state)?;

                Ok(*default_address)
            }
        }
    }

    /// Resolve the sender's public key to an address.
    pub fn resolve_sender_address_read_only<ST: StateReader<User>>(
        &self,
        default_address: &S::Address,
        credential_id: &CredentialId,
        state: &mut ST,
    ) -> Result<S::Address, ST::Error> {
        let maybe_address = self.accounts.get(credential_id, state)?.map(|a| a.addr);
        match maybe_address {
            Some(address) => Ok(address),
            None => Ok(*default_address),
        }
    }
}
