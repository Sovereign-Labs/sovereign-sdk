//! This module implements the core logic of the rollup.
//! To add new functionality to your rollup:
//!   1. Add a new module dependency to your `Cargo.toml` file
//!   2. Add the module to the `Runtime` below
//!   3. Update `genesis.json` with any additional data required by your new module
#[cfg(feature = "native")]
pub use sov_accounts::{AccountsRpcImpl, AccountsRpcServer};
#[cfg(feature = "native")]
pub use sov_bank::{BankRpcImpl, BankRpcServer};
use sov_modules_api::capabilities::{BlobRefOrOwned, BlobSelector};
use sov_modules_api::default_context::ZkDefaultContext;
use sov_modules_api::macros::DefaultRuntime;
use sov_modules_api::{Context, DaSpec, DispatchCall, Genesis, MessageCodec, Zkvm};
use sov_modules_stf_template::AppTemplate;
use sov_rollup_interface::da::DaVerifier;
#[cfg(feature = "native")]
pub use sov_sequencer_registry::{SequencerRegistryRpcImpl, SequencerRegistryRpcServer};
use sov_state::ZkStorage;
use sov_stf_runner::verifier::StateTransitionVerifier;

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
            .map(BlobRefOrOwned::Ref)
            .collect())
    }
}

/// Create the zk version of the STF.
pub fn zk_stf<Vm: Zkvm, Da: DaSpec>(
) -> AppTemplate<ZkDefaultContext, Da, Vm, Runtime<ZkDefaultContext, Da>> {
    let storage = ZkStorage::new();
    AppTemplate::new(storage, Runtime::default())
}

/// A verifier for the rollup
pub type RollupVerifier<DA, Zk> = StateTransitionVerifier<
    AppTemplate<
        ZkDefaultContext,
        <DA as DaVerifier>::Spec,
        Zk,
        Runtime<ZkDefaultContext, <DA as DaVerifier>::Spec>,
    >,
    DA,
    Zk,
>;
