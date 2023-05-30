use sov_state::WorkingSet;

use crate::{Context, Spec};

/// Transaction represents a deserialized RawTx.
#[derive(Debug, PartialEq, Eq, Clone, borsh::BorshDeserialize, borsh::BorshSerialize)]
pub struct Transaction<C: Context> {
    pub signature: C::Signature,
    pub pub_key: C::PublicKey,
    pub runtime_msg: Vec<u8>,
    pub nonce: u64,
}

impl<C: Context> Transaction<C> {
    #[allow(dead_code)]
    pub fn new(msg: Vec<u8>, pub_key: C::PublicKey, signature: C::Signature, nonce: u64) -> Self {
        Self {
            signature,
            runtime_msg: msg,
            pub_key,
            nonce,
        }
    }
}

pub trait ApplyBatchHooks {
    type Context: Context;

    /// runs just before a transaction is dispatched to an appropriate module.
    fn pre_dispatch_tx_hook(
        &self,
        tx: Transaction<Self::Context>,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<<Self::Context as Spec>::Address>;

    /// runs after the tx is dispatched to an appropriate module.
    fn post_dispatch_tx_hook(
        &self,
        pub_key: <Self::Context as Spec>::PublicKey,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    );

    /// runs at the beginning of apply_blob.
    fn enter_apply_blob(
        &self,
        sequencer: &[u8],
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<()>;

    /// runs at the end of apply_batch.
    fn exit_apply_blob(
        &self,
        amount: u64,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<()>;
}
