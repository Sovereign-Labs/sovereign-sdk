use sov_chain_state::ChainState;
use sov_modules_api::hooks::{ApplyBlobHooks, FinalizeHook, SlotHooks, TxHooks};
use sov_modules_api::macros::DefaultRuntime;
use sov_modules_api::transaction::Transaction;
use sov_modules_api::{
    AccessoryWorkingSet, BlobReaderTrait, Context, DaSpec, DispatchCall, Genesis, MessageCodec,
    PublicKey, Spec,
};
use sov_modules_stf_blueprint::{Runtime, RuntimeTxHook, SequencerOutcome};
use sov_state::Storage;
use sov_value_setter::ValueSetter;

#[derive(Genesis, DispatchCall, MessageCodec, DefaultRuntime)]
#[serialization(borsh::BorshDeserialize, borsh::BorshSerialize)]
pub(crate) struct TestRuntime<C: Context> {
    pub value_setter: ValueSetter<C>,
}

#[derive(Default)]
pub(crate) struct TestKernel<C: Context, Da: DaSpec> {
    pub _chain_state: ChainState<C, Da>,
}

impl<C: Context> TxHooks for TestRuntime<C> {
    type Context = C;
    type PreArg = RuntimeTxHook<C>;
    type PreResult = C;

    fn pre_dispatch_tx_hook(
        &self,
        tx: &Transaction<Self::Context>,
        _working_set: &mut sov_modules_api::WorkingSet<C>,
        arg: &RuntimeTxHook<C>,
    ) -> anyhow::Result<C> {
        let RuntimeTxHook { height, sequencer } = arg;
        let sender = tx.pub_key().to_address();
        let sequencer = sequencer.to_address();

        Ok(C::new(sender, sequencer, *height))
    }

    fn post_dispatch_tx_hook(
        &self,
        _tx: &Transaction<Self::Context>,
        _ctx: &C,
        _working_set: &mut sov_modules_api::WorkingSet<C>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

impl<C: Context, B: BlobReaderTrait> ApplyBlobHooks<B> for TestRuntime<C> {
    type Context = C;
    type BlobResult = SequencerOutcome<<B as BlobReaderTrait>::Address>;

    fn begin_blob_hook(
        &self,
        _blob: &mut B,
        _working_set: &mut sov_modules_api::WorkingSet<C>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn end_blob_hook(
        &self,
        _result: Self::BlobResult,
        _working_set: &mut sov_modules_api::WorkingSet<C>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

impl<C: Context, Da: DaSpec> SlotHooks<Da> for TestRuntime<C> {
    type Context = C;

    fn begin_slot_hook(
        &self,
        _slot_header: &Da::BlockHeader,
        _validity_condition: &Da::ValidityCondition,
        _pre_state_root: &<<Self::Context as Spec>::Storage as Storage>::Root,
        _working_set: &mut sov_modules_api::WorkingSet<C>,
    ) {
    }

    fn end_slot_hook(&self, _working_set: &mut sov_modules_api::WorkingSet<C>) {}
}

impl<C: Context, Da: sov_modules_api::DaSpec> FinalizeHook<Da> for TestRuntime<C> {
    type Context = C;

    fn finalize_hook(
        &self,
        _root_hash: &<<Self::Context as Spec>::Storage as Storage>::Root,
        _accesorry_working_set: &mut AccessoryWorkingSet<C>,
    ) {
    }
}

impl<C: Context, Da: DaSpec> Runtime<C, Da> for TestRuntime<C> {
    type GenesisConfig = GenesisConfig<C>;

    fn rpc_methods(_storage: <C as Spec>::Storage) -> jsonrpsee::RpcModule<()> {
        todo!()
    }

    type GenesisPaths = ();

    fn genesis_config(
        _genesis_paths: &Self::GenesisPaths,
    ) -> Result<Self::GenesisConfig, anyhow::Error> {
        todo!()
    }
}
