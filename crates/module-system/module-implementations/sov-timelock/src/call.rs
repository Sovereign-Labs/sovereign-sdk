use std::num::NonZeroU64;

use schemars::JsonSchema;
use sov_modules_api::capabilities::{
    ProposalId, TimelockCapability, TimelockError, TimelockPolicy, TimelockProposalData,
    TimelockProposalOutcome,
};
use sov_modules_api::macros::{serialize, UniversalWallet};
use sov_modules_api::{
    Context, CoreModuleError, Error as ModuleError, EventEmitter, SafeVec, Spec, TxState,
};

use crate::{
    CancellationPolicy, Error, Event, ProposalKey, Timelock, MAX_PROPOSAL_KEYS_PER_CLEANUP,
};

/// Calls supported by the timelock module.
#[derive(Debug, PartialEq, Eq, Clone, JsonSchema, UniversalWallet)]
#[serialize(Borsh, Serde)]
#[serde(bound = "S: Spec")]
#[schemars(bound = "S::Address: ::schemars::JsonSchema", rename = "CallMessage")]
#[serde(rename_all = "snake_case")]
pub enum CallMessage<S: Spec> {
    /// Cancel a pending proposal.
    CancelProposal {
        /// Proposal id to cancel.
        proposal_id: ProposalId,
        /// Owner address. If absent, the sender is treated as the owner.
        address: Option<S::Address>,
    },
    /// Modify the sender's cancellation policy.
    ModifyCancellationPolicy {
        /// New cancellation policy.
        new_policy: CancellationPolicy<S>,
    },
    /// Clean expired proposals.
    CleanExpiredProposals {
        /// Proposal keys to clean.
        proposal_keys: SafeVec<ProposalKey<S>, MAX_PROPOSAL_KEYS_PER_CLEANUP>,
    },
}

impl<S: Spec> Timelock<S> {
    pub(super) fn cancel_proposal(
        &mut self,
        proposal_id: ProposalId,
        address: Option<S::Address>,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> Result<(), Error<S>> {
        let address = address.unwrap_or(*context.sender());
        let key = ProposalKey::new(address, proposal_id);
        let Some(pending_proposal) = self
            .proposals
            .get(&key, state)
            .map_err(CoreModuleError::state_read)?
        else {
            return Err(TimelockError::ProposalNotFound.into());
        };

        if context.sender() != &address
            && context.sender()
                != &pending_proposal
                    .unlock_condition
                    .cancellation_policy
                    .authorized_canceller
        {
            return Err(Error::UnauthorizedCanceller {
                sender: *context.sender(),
                address,
                authorized_canceller: pending_proposal
                    .unlock_condition
                    .cancellation_policy
                    .authorized_canceller,
                proposal_id,
            });
        }

        self.delete_proposal(&key, state)?;
        self.emit_event(
            state,
            Event::ProposalCancelled {
                address,
                proposal_id,
                cancelled_by: *context.sender(),
            },
        );
        Ok(())
    }

    pub(super) fn clean_expired_proposals(
        &mut self,
        proposal_keys: SafeVec<ProposalKey<S>, MAX_PROPOSAL_KEYS_PER_CLEANUP>,
        state: &mut impl TxState<S>,
    ) -> Result<(), Error<S>> {
        let current_time = self.current_time_secs(state)?;
        for key in proposal_keys {
            self.clean_expired_proposal(&key, current_time, state)?;
        }

        Ok(())
    }

    fn clean_expired_proposal(
        &mut self,
        key: &ProposalKey<S>,
        current_time: u64,
        state: &mut impl TxState<S>,
    ) -> Result<(), Error<S>> {
        let Some(pending_proposal) = self
            .proposals
            .get(key, state)
            .map_err(CoreModuleError::state_read)?
        else {
            return Err(TimelockError::ProposalNotFound.into());
        };

        if current_time <= pending_proposal.unlock_condition.executable_until {
            return Err(Error::ProposalNotExpired {
                proposal_key: key.clone(),
                current_time,
                executable_until: pending_proposal.unlock_condition.executable_until,
            });
        }

        self.delete_proposal(key, state)?;
        self.emit_event(
            state,
            Event::ExpiredProposalCleaned {
                address: key.0,
                proposal_id: key.1,
            },
        );

        Ok(())
    }

    pub(super) fn modify_cancellation_policy(
        &mut self,
        new_policy: CancellationPolicy<S>,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> Result<(), ModuleError> {
        let current_policy = self
            .cancellation_policy(context.sender(), state)
            .map_err(ModuleError::from)?;
        let Some(policy_change_timelock_seconds) =
            NonZeroU64::new(current_policy.policy_change_timelock_seconds)
        else {
            self.policies
                .set(context.sender(), &new_policy, state)
                .map_err(CoreModuleError::state_write)
                .map_err(ModuleError::from)?;
            return Ok(());
        };

        let proposal_data = self
            .policy_update_proposal_data(&new_policy)
            .map_err(ModuleError::from)?;
        let policy = TimelockPolicy {
            unlock_seconds_from_proposal: policy_change_timelock_seconds,
            expire_seconds_after_unlock_override: None,
        };
        match self.register_or_try_unlock_proposal(
            context.sender(),
            proposal_data,
            policy,
            state,
        )? {
            TimelockProposalOutcome::Registered => {}
            TimelockProposalOutcome::Unlocked => {
                self.policies
                    .set(context.sender(), &new_policy, state)
                    .map_err(CoreModuleError::state_write)
                    .map_err(ModuleError::from)?;
            }
        }

        Ok(())
    }

    fn policy_update_proposal_data(
        &self,
        new_policy: &CancellationPolicy<S>,
    ) -> Result<TimelockProposalData, Error<S>> {
        let encoded_message = borsh::to_vec(&CallMessage::ModifyCancellationPolicy {
            new_policy: new_policy.clone(),
        })
        .map_err(|error| CoreModuleError::Generic(anyhow::anyhow!(error)))?;

        Ok(TimelockProposalData::custom_data(self.id, encoded_message))
    }
}
