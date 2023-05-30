use crate::{Context, Spec};
use sov_state::WorkingSet;

// A Transaction object that is compatible with the module-system/sov-default-stf;
#[derive(Debug, PartialEq, Eq, Clone, borsh::BorshDeserialize, borsh::BorshSerialize)]
pub struct Transaction<C: Context> {
    signature: C::Signature,
    pub_key: C::PublicKey,
    runtime_msg: Vec<u8>,
    nonce: u64,
}

impl<C: Context> Transaction<C> {
    pub fn new(msg: Vec<u8>, pub_key: C::PublicKey, signature: C::Signature, nonce: u64) -> Self {
        Self {
            signature,
            runtime_msg: msg,
            pub_key,
            nonce,
        }
    }

    pub fn signature(&self) -> &C::Signature {
        &self.signature
    }

    pub fn pub_key(&self) -> &C::PublicKey {
        &self.pub_key
    }

    pub fn runtime_msg(&self) -> &[u8] {
        &self.runtime_msg
    }

    pub fn nonce(&self) -> u64 {
        self.nonce
    }
}

/// Hooks that execute within the `StateTransitionFunction::apply_blob` function for each processed transaction.
pub trait ApplyBlobTxHooks {
    type Context: Context;

    /// Runs just before a transaction is dispatched to an appropriate module.
    fn pre_dispatch_tx_hook(
        &self,
        tx: Transaction<Self::Context>,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<<Self::Context as Spec>::Address>;

    /// Runs after the tx is dispatched to an appropriate module.
    fn post_dispatch_tx_hook(
        &self,
        tx: &Transaction<Self::Context>,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<()>;
}

// Hooks related to the Sequencer functionality.
// In essence, the sequencer locks a bond at the beginning of the
// `StateTransitionFunction::apply_blob`, and is rewarded once a blob of transactions is processed.
pub trait ApplyBlobSequencerHooks {
    type Context: Context;
    /// Runs at the beginning of apply_blob, locks the sequencer bond.
    fn lock_sequencer_bond(
        &self,
        sequencer: &[u8],
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<()>;

    /// Executes at the end of apply_blob and rewards the sequencer. This method is not invoked if the sequencer has been slashed.
    fn reward_sequencer(
        &self,
        amount: u64,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<()>;
}
