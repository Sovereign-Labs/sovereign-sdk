use schemars::JsonSchema;
use sov_modules_api::capabilities::{ProposalId, TimelockProposalData};
use sov_modules_api::macros::serialize;
use sov_modules_api::Spec;

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
    /// An expired proposal was cleaned.
    ExpiredProposalCleaned {
        /// Proposal owner.
        address: S::Address,
        /// Proposal id.
        proposal_id: ProposalId,
    },
}
