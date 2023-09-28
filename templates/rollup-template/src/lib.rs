#[cfg(feature = "native")]
pub use sov_accounts::{AccountsRpcImpl, AccountsRpcServer};
#[cfg(feature = "native")]
pub use sov_bank::{BankRpcImpl, BankRpcServer};
use sov_modules_api::{
    capabilities::{BlobRefOrOwned, BlobSelector},
    macros::DefaultRuntime,
    Context, DaSpec, DispatchCall, Genesis, MessageCodec,
};
#[cfg(feature = "native")]
pub use sov_sequencer_registry::{SequencerRegistryRpcImpl, SequencerRegistryRpcServer};

#[cfg(feature = "native")]
mod builder;

mod hooks;
#[cfg(feature = "native")]
pub mod rollup;

#[cfg(feature = "native")]
mod rpc;

#[cfg_attr(
    feature = "native",
    derive(sov_modules_api::macros::CliWallet),
    sov_modules_api::macros::expose_rpc
)]
#[derive(Genesis, DispatchCall, MessageCodec, DefaultRuntime)]
#[serialization(borsh::BorshDeserialize, borsh::BorshSerialize)]
#[cfg_attr(
    feature = "native",
    serialization(serde::Serialize, serde::Deserialize)
)]
pub struct Runtime<C: Context, Da: DaSpec> {
    pub accounts: sov_accounts::Accounts<C>,
    pub bank: sov_bank::Bank<C>,
    pub sequencer_registry: sov_sequencer_registry::SequencerRegistry<C, Da>,
}

impl<C, Da> sov_modules_stf_template::Runtime<C, Da> for Runtime<C, Da>
where
    C: Context,
    Da: DaSpec,
{
}

// Select which blobs will be executed in this slot. In this implementation simply execute all
// available blobs in the order they appeared on the DA layer
impl<C: Context, Da: DaSpec> BlobSelector<Da> for Runtime<C, Da> {
    type Context = C;
    fn get_blobs_for_this_slot<'a, I>(
        &self,
        current_blobs: I,
        _working_set: &mut sov_modules_api::WorkingSet<C>,
    ) -> anyhow::Result<Vec<BlobRefOrOwned<'a, Da::BlobTransaction>>>
    where
        I: IntoIterator<Item = &'a mut Da::BlobTransaction>,
    {
        Ok(current_blobs
            .into_iter()
            .map(|blob| BlobRefOrOwned::Ref(blob))
            .collect())
    }
}
