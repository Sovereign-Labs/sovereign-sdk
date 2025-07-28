use sov_hyperlane_integration::test_recipient::TestRecipient;
use sov_hyperlane_integration::warp::Warp;
use sov_hyperlane_integration::{
    EthAddress, HyperlaneAddress, InterchainGasPaymaster, Ism, Mailbox as RawMailbox,
    MerkleTreeHook, Recipient, StorageLocation,
};
use sov_modules_api::{
    Context, HexHash, HexString, Module, ModuleId, ModuleInfo, ModuleRestApi, Spec, TxState,
};
use sov_test_utils::generate_runtime;

pub type Mailbox<S> = RawMailbox<S, RoutingRecipient<S>>;

/// A module that can route messages between Warp and TestRecipient.
#[derive(Clone, ModuleInfo, ModuleRestApi)]
pub struct RoutingRecipient<S: Spec> {
    /// The ID of the module.
    #[id]
    pub id: ModuleId,

    #[phantom]
    phantom: std::marker::PhantomData<S>,
}

impl<S: Spec> Module for RoutingRecipient<S> {
    type Spec = S;
    type Config = ();
    type CallMessage = ();
    type Event = ();

    fn call(
        &mut self,
        _msg: Self::CallMessage,
        _context: &Context<Self::Spec>,
        _state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

impl<S: Spec> Recipient<S> for RoutingRecipient<S>
where
    S::Address: HyperlaneAddress,
{
    fn ism(&self, recipient: &HexHash, state: &mut impl TxState<S>) -> anyhow::Result<Option<Ism>> {
        if let Ok(Some(ism)) = Warp::<S>::default().ism(recipient, state) {
            return Ok(Some(ism));
        }
        TestRecipient::<S>::default().ism(recipient, state)
    }

    /// Handles an inbound message. Note that this deviates from more standard Hyperlane `handle` API because all messages
    /// are dispatched through this module regardless of their ultimate destination, so we need to explicitly pass the recipient as an argument.
    fn handle(
        &mut self,
        origin: u32,
        sender: HexHash,
        recipient: &HexHash,
        body: HexString,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        if Warp::<S>::default()
            .ism(recipient, state)
            .is_ok_and(|ism| ism.is_some())
        {
            Warp::<S>::default().handle(origin, sender, recipient, body, state)
        } else {
            TestRecipient::<S>::default().handle(origin, sender, recipient, body, state)
        }
    }

    fn handle_validator_announce(
        &self,
        validator_address: &EthAddress,
        storage_location: &StorageLocation,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        Warp::<S>::default().handle_validator_announce(
            validator_address,
            storage_location,
            state,
        )?;
        TestRecipient::<S>::default().handle_validator_announce(
            validator_address,
            storage_location,
            state,
        )
    }

    fn default_ism(&self, state: &mut impl TxState<S>) -> anyhow::Result<Option<Ism>> {
        // warp doesn't have default ism for sure, so we only try test recipient
        TestRecipient::<S>::default().default_ism(state)
    }
}

generate_runtime! {
    name: TestRuntime,
    modules: [mailbox: Mailbox<S>, merkle_tree_hook: MerkleTreeHook<S>, interchain_gas_paymaster: InterchainGasPaymaster<S>, routing_recipient: RoutingRecipient<S>, test_recipient: TestRecipient<S>, warp: Warp<S>],
    operating_mode: sov_modules_api::runtime::OperatingMode::Zk,
    minimal_genesis_config_type: sov_test_utils::runtime::genesis::zk::config::MinimalZkGenesisConfig<S>,
    runtime_trait_impl_bounds: [S::Address: HyperlaneAddress],
    kernel_type: sov_test_utils::runtime::SoftConfirmationsKernel<'a, S>,
    auth_type: sov_modules_api::capabilities::RollupAuthenticator<S, TestRuntime<S>>,
    auth_call_wrapper: |call| call,
}
