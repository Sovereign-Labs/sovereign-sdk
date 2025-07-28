use sov_modules_api::{CredentialId, Spec, StateAccessor, StateWriter};
use sov_state::User;

use crate::{Account, Accounts};

impl<S: Spec> Accounts<S> {
    /// Resolve the sender's public key to an address.
    /// If the sender is not registered, but a fallback address if provided, immediately registers
    /// the credential to the fallback and then returns it.
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
                    addr: default_address.clone(),
                };
                self.accounts.set(credential_id, &new_account, state)?;

                Ok(default_address.clone())
            }
        }
    }
}
