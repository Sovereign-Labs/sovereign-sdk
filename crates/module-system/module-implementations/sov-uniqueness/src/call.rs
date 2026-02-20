use schemars::JsonSchema;
use sov_modules_api::macros::{serialize, UniversalWallet};
use sov_modules_api::{Context, CoreModuleError, CredentialId, Spec, TxState};
use strum::{EnumDiscriminants, EnumIs, VariantArray};

use crate::error::{PruneGenerationsError, PruneSelfGenerationsError};
use crate::Uniqueness;

/// The available call messages for the `sov-uniqueness` module.
#[derive(Debug, PartialEq, Eq, Clone, JsonSchema, EnumDiscriminants, EnumIs, UniversalWallet)]
#[serialize(Borsh, Serde)]
#[schemars(rename = "CallMessage")]
#[strum_discriminants(derive(VariantArray, EnumIs))]
#[serde(rename_all = "snake_case")]
pub enum CallMessage {
    /// Prune generation deduplication data for the specified credentials.
    /// Only callable by the chain state admin.
    PruneGenerations {
        /// The credential IDs whose generation data should be deleted.
        credential_ids: Vec<CredentialId>,
    },
    /// Prune the caller's own generation deduplication data.
    /// The caller must provide their credential ID, and the module verifies
    /// that it derives to the sender's address.
    PruneSelfGenerations {
        /// The caller's credential ID whose generation data should be deleted.
        credential_id: CredentialId,
    },
}

impl<S: Spec> Uniqueness<S> {
    /// Prune generation deduplication data for the specified credentials.
    ///
    /// Only the chain state admin is authorized to call this method.
    /// Each credential's entire generation entry is removed from state.
    pub(crate) fn prune_generations(
        &mut self,
        credential_ids: &[CredentialId],
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> Result<(), PruneGenerationsError> {
        let admin = self
            .chain_state
            .admin_address(state)
            .map_err(CoreModuleError::state_read)?
            .ok_or(PruneGenerationsError::NoAdmin)?;

        if context.sender() != &admin {
            return Err(PruneGenerationsError::NotAdmin {
                admin: admin.to_string(),
                sender: context.sender().to_string(),
            });
        }

        for credential_id in credential_ids {
            self.generations
                .delete(credential_id, state)
                .map_err(CoreModuleError::state_write)?;
        }

        Ok(())
    }

    /// Prune the caller's own generation deduplication data.
    ///
    /// The caller provides their credential ID, and the module verifies that
    /// `S::Address::from(credential_id) == context.sender()` before deleting.
    pub(crate) fn prune_self_generations(
        &mut self,
        credential_id: &CredentialId,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> Result<(), PruneSelfGenerationsError> {
        let expected_sender = S::Address::from(*credential_id);
        if *context.sender() != expected_sender {
            return Err(PruneSelfGenerationsError::NotOwner {
                credential_id: credential_id.to_string(),
                expected_sender: expected_sender.to_string(),
                actual_sender: context.sender().to_string(),
            });
        }

        self.generations
            .delete(credential_id, state)
            .map_err(CoreModuleError::state_write)?;

        Ok(())
    }
}
