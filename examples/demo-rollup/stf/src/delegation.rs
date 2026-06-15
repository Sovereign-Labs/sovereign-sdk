//! This is a technical only module to forward all necessary implementations to inner, non-authenticated Runtime.

use sov_address::{EthereumAddress, FromVmAddress};
use sov_hyperlane_integration::HyperlaneAddress;
use sov_modules_api::{prelude::*, Base58Address};
use sov_modules_api::{
    AuthenticatedTransactionData, BlockHooks, DispatchCall, EncodeCall, Genesis, GenesisState,
    ModuleError, ModuleId, ModuleInfo, NestedEnumUtils, RuntimeEventProcessor, Spec,
    StateCheckpoint, Storage, TxHooks, TxState, TypeErasedEvent,
};
use sov_rollup_interface::da::DaSpec;

use crate::runtime::Runtime;
use demo_stf_declaration::GenesisConfig;
use demo_stf_declaration::Runtime as RuntimeInner;
use demo_stf_declaration::RuntimeCall;

impl<S: Spec> Genesis for Runtime<S>
where
    S::Address: FromVmAddress<EthereumAddress> + FromVmAddress<Base58Address> + HyperlaneAddress,
{
    type Spec = S;
    type Config = GenesisConfig<S>;

    fn genesis(
        &mut self,
        genesis_rollup_header: &<<Self::Spec as Spec>::Da as DaSpec>::BlockHeader,
        config: &Self::Config,
        state: &mut impl GenesisState<Self::Spec>,
    ) -> Result<(), ModuleError> {
        self.0.genesis(genesis_rollup_header, config, state)
    }
}

impl<S: Spec> DispatchCall for Runtime<S>
where
    S::Address: FromVmAddress<EthereumAddress> + FromVmAddress<Base58Address> + HyperlaneAddress,
{
    type Spec = S;
    type Decodable = RuntimeCall<S>;

    fn encode(decodable: &Self::Decodable) -> Vec<u8> {
        RuntimeInner::<S>::encode(decodable)
    }

    fn dispatch_call<I: StateProvider<Self::Spec>>(
        &mut self,
        message: Self::Decodable,
        state: &mut WorkingSet<Self::Spec, I>,
        context: &Context<Self::Spec>,
    ) -> Result<(), ModuleError> {
        self.0.dispatch_call(message, state, context)
    }

    fn module_id(&self, message: &Self::Decodable) -> &ModuleId {
        self.0.module_id(message)
    }

    fn module_info(
        &self,
        discriminant: <Self::Decodable as NestedEnumUtils>::Discriminants,
    ) -> &dyn ModuleInfo<Spec = Self::Spec> {
        self.0.module_info(discriminant)
    }
}

impl<S: Spec> EncodeCall<sov_bank::Bank<S>> for Runtime<S>
where
    S::Address: FromVmAddress<EthereumAddress> + FromVmAddress<Base58Address> + HyperlaneAddress,
{
    fn encode_call(data: <sov_bank::Bank<S> as sov_modules_api::Module>::CallMessage) -> Vec<u8> {
        <RuntimeInner<S> as EncodeCall<sov_bank::Bank<S>>>::encode_call(data)
    }

    fn to_decodable(
        data: <sov_bank::Bank<S> as sov_modules_api::Module>::CallMessage,
    ) -> Self::Decodable {
        <RuntimeInner<S> as EncodeCall<sov_bank::Bank<S>>>::to_decodable(data)
    }
}

impl<S: Spec> EncodeCall<sov_test_modules::access_pattern::AccessPattern<S>> for Runtime<S>
where
    S::Address: FromVmAddress<EthereumAddress> + FromVmAddress<Base58Address> + HyperlaneAddress,
{
    fn encode_call(
        data: <sov_test_modules::access_pattern::AccessPattern<S> as sov_modules_api::Module>::CallMessage,
    ) -> Vec<u8> {
        <RuntimeInner<S> as EncodeCall<sov_test_modules::access_pattern::AccessPattern<S>>>::encode_call(data)
    }

    fn to_decodable(
        data: <sov_test_modules::access_pattern::AccessPattern<S> as sov_modules_api::Module>::CallMessage,
    ) -> Self::Decodable {
        <RuntimeInner<S> as EncodeCall<sov_test_modules::access_pattern::AccessPattern<S>>>::to_decodable(data)
    }
}

impl<S: Spec> EncodeCall<sov_synthetic_load::SyntheticLoad<S>> for Runtime<S>
where
    S::Address: FromVmAddress<EthereumAddress> + FromVmAddress<Base58Address> + HyperlaneAddress,
{
    fn encode_call(
        data: <sov_synthetic_load::SyntheticLoad<S> as sov_modules_api::Module>::CallMessage,
    ) -> Vec<u8> {
        <RuntimeInner<S> as EncodeCall<sov_synthetic_load::SyntheticLoad<S>>>::encode_call(data)
    }

    fn to_decodable(
        data: <sov_synthetic_load::SyntheticLoad<S> as sov_modules_api::Module>::CallMessage,
    ) -> Self::Decodable {
        <RuntimeInner<S> as EncodeCall<sov_synthetic_load::SyntheticLoad<S>>>::to_decodable(data)
    }
}

impl<S: Spec> BlockHooks for Runtime<S>
where
    S::Address: FromVmAddress<EthereumAddress> + FromVmAddress<Base58Address> + HyperlaneAddress,
{
    type Spec = S;

    fn begin_rollup_block_hook(
        &mut self,
        visible_hash: &<<Self::Spec as Spec>::Storage as Storage>::Root,
        state: &mut StateCheckpoint<Self::Spec>,
    ) {
        self.0.begin_rollup_block_hook(visible_hash, state);
    }

    fn end_rollup_block_hook(&mut self, state: &mut StateCheckpoint<Self::Spec>) {
        self.0.end_rollup_block_hook(state);
    }
}

impl<S: Spec> TxHooks for Runtime<S>
where
    S::Address: FromVmAddress<EthereumAddress> + FromVmAddress<Base58Address> + HyperlaneAddress,
{
    type Spec = S;

    fn pre_dispatch_tx_hook<T: TxState<Self::Spec>>(
        &mut self,
        tx: &AuthenticatedTransactionData<Self::Spec>,
        state: &mut T,
    ) -> anyhow::Result<()> {
        self.0.pre_dispatch_tx_hook(tx, state)
    }

    fn post_dispatch_tx_hook<T: TxState<Self::Spec>>(
        &mut self,
        tx: &AuthenticatedTransactionData<Self::Spec>,
        ctx: &Context<Self::Spec>,
        state: &mut T,
    ) -> anyhow::Result<()> {
        self.0.post_dispatch_tx_hook(tx, ctx, state)
    }
}

#[cfg(feature = "native")]
impl<S: Spec> sov_modules_api::FinalizeHook for Runtime<S>
where
    S::Address: FromVmAddress<EthereumAddress> + FromVmAddress<Base58Address> + HyperlaneAddress,
{
    type Spec = S;

    fn finalize_hook(
        &mut self,
        root_hash: &<<Self::Spec as Spec>::Storage as Storage>::Root,
        state: &mut impl sov_modules_api::AccessoryStateReaderAndWriter,
    ) {
        self.0.finalize_hook(root_hash, state);
    }
}

impl<S: Spec> RuntimeEventProcessor for Runtime<S>
where
    S::Address: FromVmAddress<EthereumAddress> + FromVmAddress<Base58Address> + HyperlaneAddress,
{
    type RuntimeEvent = demo_stf_declaration::RuntimeEvent<S>;

    fn convert_to_runtime_event(event: TypeErasedEvent) -> Option<Self::RuntimeEvent> {
        RuntimeInner::<S>::convert_to_runtime_event(event)
    }
}

#[cfg(feature = "native")]
impl<S: Spec> sov_modules_api::CliWallet for Runtime<S>
where
    S::Address: FromVmAddress<EthereumAddress> + FromVmAddress<Base58Address> + HyperlaneAddress,
{
    type CliStringRepr<T> = demo_stf_declaration::RuntimeMessage<T, S>;
}

#[cfg(feature = "native")]
impl<S: Spec> sov_modules_api::rest::HasRestApi<S> for Runtime<S>
where
    S::Address: FromVmAddress<EthereumAddress> + FromVmAddress<Base58Address> + HyperlaneAddress,
{
    fn rest_api(&self, state: sov_modules_api::rest::ApiState<S>) -> axum::Router<()> {
        self.0.rest_api(state)
    }

    fn openapi_spec(&self) -> Option<utoipa::openapi::OpenApi> {
        self.0.openapi_spec()
    }
}

#[cfg(feature = "native")]
impl<T, S> sov_modules_api::cli::CliFrontEnd<Runtime<S>>
    for demo_stf_declaration::RuntimeSubcommand<T, S>
where
    T: clap::Args,
    S: Spec + for<'de> serde::Deserialize<'de>,
    S::Address: FromVmAddress<EthereumAddress> + FromVmAddress<Base58Address> + HyperlaneAddress,
    demo_stf_declaration::RuntimeSubcommand<T, S>:
        sov_modules_api::cli::CliFrontEnd<RuntimeInner<S>>,
{
    type CliIntermediateRepr<U> =
        <demo_stf_declaration::RuntimeSubcommand<T, S> as sov_modules_api::cli::CliFrontEnd<
            RuntimeInner<S>,
        >>::CliIntermediateRepr<U>;
}
