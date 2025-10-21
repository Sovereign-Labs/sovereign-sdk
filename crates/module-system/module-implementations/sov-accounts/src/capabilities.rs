use sov_modules_api::{CredentialId, Spec, StateAccessor};

use crate::Accounts;

impl<S: Spec> Accounts<S> {
    /// Resolve the sender's public key to an address.
    /// If the sender is not registered, but a fallback address if provided, immediately registers
    /// the credential to the fallback and then returns it.
    pub fn resolve_sender_address_and_authorize<ST: StateAccessor>(
        &mut self,
        default_address: &S::Address,
        requested_address: &Option<S::Address>,
        credential_id: &CredentialId,
        state: &mut ST,
    ) -> anyhow::Result<S::Address> {
        // A request is allowed if
        // 1. The requested address is the default address
        // 2. No address was requested (so the default address is used)
        // 3. The signing credential has been specfiically authorized for this address
        match requested_address {
            Some(requested_address) => {
                // Case 1: The requested address is the default address
                if requested_address == default_address {
                    return Ok(default_address.clone());
                } else if let Some(account) = self.accounts.get(requested_address, state)? {
                    // Case 2: The requested address is not the default address
                    if account.allowed_credentials.contains(credential_id) {
                        return Ok(requested_address.clone());
                    }
                    // fall through to the error   
                }
                // Fall through to the error
            }
            // Case 3: No address was requested (so the default address is used)
            None => return Ok(default_address.clone()),
        }
        // Default: The requested address is not allowed to access the account
        anyhow::bail!(
            "Credential {} is not allowed to access address {:?}",
            credential_id,
            requested_address
        );
    }
}
