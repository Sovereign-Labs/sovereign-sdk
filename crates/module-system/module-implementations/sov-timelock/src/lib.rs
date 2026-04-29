//! Timelock proposal management module.

#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

use std::fmt::{Display, Formatter};
use std::num::NonZeroU64;
use std::str::FromStr;

use borsh::{BorshDeserialize, BorshSerialize};
use schemars::JsonSchema;
use sov_modules_api::capabilities::{
    calculate_timelock_proposal_id_metered, ProposalId, TimelockCapability, TimelockError,
    TimelockPolicy, TimelockProposalData, TimelockProposalOutcome,
};
use sov_modules_api::macros::{serialize, UniversalWallet};
use sov_modules_api::{
    err_detail, Context, CoreModuleError, DaSpec, Error as ModuleError, ErrorContext, ErrorDetail,
    EventEmitter, GenesisState, Module, ModuleId, ModuleInfo, ModuleRestApi, Spec, StateMap,
    TxState,
};

/// Maximum number of pending proposals per address.
pub const MAX_PENDING_PROPOSALS_PER_ADDRESS: u32 = 128;

const PROPOSAL_KEY_SEPARATOR: &str = "/proposals/";

/// Storage key for a pending proposal.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    BorshDeserialize,
    BorshSerialize,
    serde::Serialize,
    serde::Deserialize,
    JsonSchema,
)]
#[serde(bound = "S: Spec", deny_unknown_fields)]
#[schemars(bound = "S::Address: ::schemars::JsonSchema", rename = "ProposalKey")]
pub struct ProposalKey<S: Spec>(
    /// Proposal owner.
    pub S::Address,
    /// Proposal id.
    pub ProposalId,
);

impl<S: Spec> ProposalKey<S> {
    fn new(address: S::Address, proposal_id: ProposalId) -> Self {
        Self(address, proposal_id)
    }
}

impl<S: Spec> Display for ProposalKey<S> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{PROPOSAL_KEY_SEPARATOR}{}", self.0, self.1)
    }
}

impl<S> FromStr for ProposalKey<S>
where
    S: Spec,
    S::Address: FromStr<Err: Into<Box<dyn std::error::Error + Send + Sync + 'static>>>,
{
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let Some((address, proposal_id)) = s.rsplit_once(PROPOSAL_KEY_SEPARATOR) else {
            anyhow::bail!(
                "{s} is not a proposal key: missing '{PROPOSAL_KEY_SEPARATOR}' separator"
            );
        };

        Ok(Self(
            S::Address::from_str(address)
                .map_err(|error| anyhow::Error::from_boxed(error.into()))?,
            ProposalId::from_str(proposal_id)?,
        ))
    }
}

/// Cancellation rules for proposals owned by an address.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    BorshDeserialize,
    BorshSerialize,
    serde::Serialize,
    serde::Deserialize,
    JsonSchema,
    UniversalWallet,
)]
#[serde(bound = "S: Spec", deny_unknown_fields)]
#[schemars(
    bound = "S::Address: ::schemars::JsonSchema",
    rename = "CancellationPolicy"
)]
pub struct CancellationPolicy<S: Spec> {
    /// Address allowed to cancel proposals for this account when the cancel call specifies an owner.
    pub authorized_canceller: S::Address,
    /// Delay before policy updates take effect.
    pub policy_change_timelock_seconds: u64,
}

/// Stored unlock condition for a pending proposal.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    BorshDeserialize,
    BorshSerialize,
    serde::Serialize,
    serde::Deserialize,
    JsonSchema,
)]
#[serde(bound = "S: Spec", deny_unknown_fields)]
#[schemars(
    bound = "S::Address: ::schemars::JsonSchema",
    rename = "UnlockCondition"
)]
pub struct UnlockCondition<S: Spec> {
    /// Unix timestamp at which the proposal becomes executable.
    pub executable_from: u64,
    /// Unix timestamp after which the proposal is expired.
    pub executable_until: u64,
    /// Cancellation policy snapshotted when the proposal was created.
    pub cancellation_policy: CancellationPolicy<S>,
}

/// A pending proposal and its unlock condition.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    BorshDeserialize,
    BorshSerialize,
    serde::Serialize,
    serde::Deserialize,
    JsonSchema,
)]
#[serde(bound = "S: Spec", deny_unknown_fields)]
#[schemars(
    bound = "S::Address: ::schemars::JsonSchema",
    rename = "PendingProposal"
)]
pub struct PendingProposal<S: Spec> {
    /// Full proposal data whose hash is used as the proposal id.
    pub proposal_data: TimelockProposalData,
    /// Condition that must hold before the proposal may execute.
    pub unlock_condition: UnlockCondition<S>,
}

/// Events emitted by the timelock module.
#[derive(Debug, PartialEq, Eq, Clone, JsonSchema)]
#[serialize(Borsh, Serde)]
#[serde(bound = "S: Spec", rename_all = "snake_case")]
#[schemars(bound = "S::Address: ::schemars::JsonSchema", rename = "Event")]
pub enum Event<S: Spec> {
    /// A proposal was registered and is pending unlock.
    ProposalRegistered {
        /// Proposal owner.
        address: S::Address,
        /// Proposal id.
        proposal_id: ProposalId,
        /// Full proposal data whose hash is the proposal id.
        proposal_data: TimelockProposalData,
        /// Unix timestamp at which the proposal becomes executable.
        executable_from: u64,
        /// Unix timestamp after which the proposal is expired.
        executable_until: u64,
    },
    /// A proposal was unlocked and consumed for execution.
    ProposalUnlocked {
        /// Proposal owner.
        address: S::Address,
        /// Proposal id.
        proposal_id: ProposalId,
    },
    /// A pending proposal was cancelled.
    ProposalCancelled {
        /// Proposal owner.
        address: S::Address,
        /// Proposal id.
        proposal_id: ProposalId,
        /// Address that cancelled the proposal.
        cancelled_by: S::Address,
    },
}

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
}

/// Errors returned by the timelock module.
#[derive(Debug, thiserror::Error, serde::Serialize)]
#[serde(
    tag = "error_code",
    rename_all = "snake_case",
    bound = "S::Address: serde::Serialize"
)]
pub enum Error<S: Spec> {
    /// A core module error occurred.
    #[error(transparent)]
    Core(#[from] CoreModuleError),
    /// A timelock semantic error occurred.
    #[error(transparent)]
    Timelock(#[from] TimelockError),
    /// The proposal already exists.
    #[error("Proposal {proposal_id} already exists for address {address}")]
    ProposalAlreadyExists {
        /// Proposal owner.
        address: S::Address,
        /// Existing proposal id.
        proposal_id: ProposalId,
    },
    /// The proposal owner already has the maximum number of pending proposals.
    #[error(
        "Address {address} already has the maximum of {max_pending_proposals} pending proposals"
    )]
    MaxPendingProposalsReached {
        /// Proposal owner.
        address: S::Address,
        /// Maximum pending proposals per address.
        max_pending_proposals: u32,
    },
    /// The proposal counter is inconsistent with the proposal map.
    #[error(
        "Proposal count for address {address} is inconsistent while deleting proposal {proposal_id}"
    )]
    ProposalCountUnderflow {
        /// Proposal owner.
        address: S::Address,
        /// Proposal id being deleted.
        proposal_id: ProposalId,
    },
    /// The sender is not authorized to cancel the proposal.
    #[error(
        "Address {sender} is not authorized to cancel proposal {proposal_id} for {address}; authorized canceller is {authorized_canceller}"
    )]
    UnauthorizedCanceller {
        /// Transaction sender.
        sender: S::Address,
        /// Proposal owner.
        address: S::Address,
        /// Address authorized by the proposal's cancellation policy.
        authorized_canceller: S::Address,
        /// Proposal id.
        proposal_id: ProposalId,
    },
    /// The current chain time is negative.
    #[error("Current chain time {current_time} is before the Unix epoch")]
    InvalidCurrentTime {
        /// Current chain time in seconds.
        current_time: i64,
    },
    /// Computing an unlock timestamp overflowed.
    #[error(
        "Timestamp overflow while computing unlock condition from current time {current_time}"
    )]
    TimestampOverflow {
        /// Current chain time in seconds.
        current_time: u64,
    },
}

impl<S> ErrorDetail for Error<S>
where
    S: Spec,
    S::Address: serde::Serialize,
{
    fn error_detail(&self) -> Result<ErrorContext, Box<dyn std::error::Error + Send + Sync>> {
        Ok(err_detail!(self))
    }
}

/// Timelock module.
#[derive(Clone, ModuleInfo, ModuleRestApi)]
pub struct Timelock<S: Spec> {
    /// The ID of the sov-timelock module.
    #[id]
    pub id: ModuleId,

    /// Pending proposals.
    #[state]
    pub proposals: StateMap<ProposalKey<S>, PendingProposal<S>>,

    /// Number of pending proposals per address.
    #[state]
    pub proposal_counts: StateMap<S::Address, u32>,

    /// Cancellation policy per address.
    #[state]
    pub policies: StateMap<S::Address, CancellationPolicy<S>>,

    /// Reference to the ChainState module.
    #[module]
    pub(crate) chain_state: sov_chain_state::ChainState<S>,

    #[phantom]
    phantom: std::marker::PhantomData<S>,
}

impl<S: Spec> Module for Timelock<S> {
    type Spec = S;

    type Config = ();

    type CallMessage = CallMessage<S>;

    type Event = Event<S>;

    type Error = ModuleError;

    fn genesis(
        &mut self,
        _genesis_rollup_header: &<<S as Spec>::Da as DaSpec>::BlockHeader,
        _config: &Self::Config,
        _state: &mut impl GenesisState<S>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn call(
        &mut self,
        msg: Self::CallMessage,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> Result<(), Self::Error> {
        match msg {
            CallMessage::CancelProposal {
                proposal_id,
                address,
            } => self
                .cancel_proposal(proposal_id, address, context, state)
                .map_err(ModuleError::from)?,
            CallMessage::ModifyCancellationPolicy { new_policy } => {
                self.modify_cancellation_policy(new_policy, context, state)?;
            }
        }
        Ok(())
    }
}

impl<S: Spec> TimelockCapability<S> for Timelock<S> {
    fn register_or_try_unlock_proposal(
        &mut self,
        address: &S::Address,
        proposal_data: TimelockProposalData,
        policy: TimelockPolicy,
        state: &mut impl TxState<S>,
    ) -> Result<TimelockProposalOutcome, ModuleError> {
        let proposal_id = calculate_timelock_proposal_id_metered::<_, S>(&proposal_data, state)
            .map_err(|error| CoreModuleError::Generic(anyhow::anyhow!(error)))
            .map_err(ModuleError::from)?;
        let key = ProposalKey::new(*address, proposal_id);
        if self
            .proposals
            .get(&key, state)
            .map_err(CoreModuleError::state_read)?
            .is_some()
        {
            self.try_unlock_proposal_inner(address, &proposal_id, state)
                .map_err(ModuleError::from)?;
            Ok(TimelockProposalOutcome::Unlocked)
        } else {
            self.register_proposal_inner(address, proposal_id, proposal_data, policy, state)
                .map_err(ModuleError::from)?;
            Ok(TimelockProposalOutcome::Registered)
        }
    }
}

impl<S: Spec> Timelock<S> {
    fn current_time_secs(&self, state: &mut impl TxState<S>) -> Result<u64, Error<S>> {
        let current_time = self
            .chain_state
            .get_time(state)
            .map_err(CoreModuleError::state_read)?
            .secs();

        current_time
            .try_into()
            .map_err(|_| Error::InvalidCurrentTime { current_time })
    }

    fn cancellation_policy(
        &self,
        address: &S::Address,
        state: &mut impl TxState<S>,
    ) -> Result<CancellationPolicy<S>, Error<S>> {
        Ok(self
            .policies
            .get(address, state)
            .map_err(CoreModuleError::state_read)?
            .unwrap_or(CancellationPolicy {
                authorized_canceller: *address,
                policy_change_timelock_seconds: 0,
            }))
    }

    fn unlock_condition(
        &self,
        address: &S::Address,
        policy: TimelockPolicy,
        state: &mut impl TxState<S>,
    ) -> Result<UnlockCondition<S>, Error<S>> {
        let current_time = self.current_time_secs(state)?;
        let executable_from = current_time
            .checked_add(policy.unlock_seconds_from_proposal.get())
            .ok_or(Error::TimestampOverflow { current_time })?;
        let executable_until = executable_from
            .checked_add(policy.expire_seconds_after_unlock())
            .ok_or(Error::TimestampOverflow { current_time })?;

        Ok(UnlockCondition {
            executable_from,
            executable_until,
            cancellation_policy: self.cancellation_policy(address, state)?,
        })
    }

    fn register_proposal_inner(
        &mut self,
        address: &S::Address,
        proposal_id: ProposalId,
        proposal_data: TimelockProposalData,
        policy: TimelockPolicy,
        state: &mut impl TxState<S>,
    ) -> Result<(), Error<S>> {
        let key = ProposalKey::new(*address, proposal_id);
        if self
            .proposals
            .get(&key, state)
            .map_err(CoreModuleError::state_read)?
            .is_some()
        {
            return Err(Error::ProposalAlreadyExists {
                address: *address,
                proposal_id,
            });
        }

        let unlock_condition = self.unlock_condition(address, policy, state)?;
        let pending_proposal = PendingProposal {
            proposal_data,
            unlock_condition,
        };
        self.add_proposal(&key, &pending_proposal, state)?;
        self.emit_event(
            state,
            Event::ProposalRegistered {
                address: *address,
                proposal_id,
                proposal_data: pending_proposal.proposal_data.clone(),
                executable_from: pending_proposal.unlock_condition.executable_from,
                executable_until: pending_proposal.unlock_condition.executable_until,
            },
        );
        Ok(())
    }

    fn try_unlock_proposal_inner(
        &mut self,
        address: &S::Address,
        proposal_id: &ProposalId,
        state: &mut impl TxState<S>,
    ) -> Result<(), Error<S>> {
        let key = ProposalKey::new(*address, *proposal_id);
        let Some(pending_proposal) = self
            .proposals
            .get(&key, state)
            .map_err(CoreModuleError::state_read)?
        else {
            return Err(TimelockError::ProposalNotFound.into());
        };

        let current_time = self.current_time_secs(state)?;
        if current_time < pending_proposal.unlock_condition.executable_from {
            return Err(TimelockError::ProposalLocked.into());
        }

        if current_time > pending_proposal.unlock_condition.executable_until {
            return Err(TimelockError::ProposalExpired.into());
        }

        self.delete_proposal(&key, state)?;
        self.emit_event(
            state,
            Event::ProposalUnlocked {
                address: *address,
                proposal_id: *proposal_id,
            },
        );

        Ok(())
    }

    fn cancel_proposal(
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

    fn add_proposal(
        &mut self,
        key: &ProposalKey<S>,
        pending_proposal: &PendingProposal<S>,
        state: &mut impl TxState<S>,
    ) -> Result<(), Error<S>> {
        let count = self
            .proposal_counts
            .get(&key.0, state)
            .map_err(CoreModuleError::state_read)?
            .unwrap_or_default();

        if count >= MAX_PENDING_PROPOSALS_PER_ADDRESS {
            return Err(Error::MaxPendingProposalsReached {
                address: key.0,
                max_pending_proposals: MAX_PENDING_PROPOSALS_PER_ADDRESS,
            });
        }

        self.proposals
            .set(key, pending_proposal, state)
            .map_err(CoreModuleError::state_write)?;
        self.proposal_counts
            .set(&key.0, &(count + 1), state)
            .map_err(CoreModuleError::state_write)?;

        Ok(())
    }

    fn delete_proposal(
        &mut self,
        key: &ProposalKey<S>,
        state: &mut impl TxState<S>,
    ) -> Result<(), Error<S>> {
        let Some(new_count) = self
            .proposal_counts
            .get(&key.0, state)
            .map_err(CoreModuleError::state_read)?
            .unwrap_or_default()
            .checked_sub(1)
        else {
            return Err(Error::ProposalCountUnderflow {
                address: key.0,
                proposal_id: key.1,
            });
        };

        self.proposals
            .delete(key, state)
            .map_err(CoreModuleError::state_write)?;
        if new_count == 0 {
            self.proposal_counts
                .delete(&key.0, state)
                .map_err(CoreModuleError::state_write)?;
        } else {
            self.proposal_counts
                .set(&key.0, &new_count, state)
                .map_err(CoreModuleError::state_write)?;
        }

        Ok(())
    }

    fn modify_cancellation_policy(
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
