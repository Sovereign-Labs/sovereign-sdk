use sov_chain_state::{ChainState, ChainStateConfig};
use sov_modules_api::capabilities::{BlobRefOrOwned, BlobSelector};
use sov_modules_api::hooks::{ApplyBlobHooks, SlotHooks, TxHooks};
use sov_modules_api::macros::DefaultRuntime;
use sov_modules_api::transaction::Transaction;
use sov_modules_api::{
    BlobReaderTrait, Context, DaSpec, DispatchCall, Genesis, MessageCodec, PublicKey, Spec,
};
use sov_modules_stf_template::{AppTemplate, Runtime, SequencerOutcome};
use sov_rollup_interface::mocks::MockZkvm;
use sov_value_setter::{ValueSetter, ValueSetterConfig};

#[derive(Genesis, DispatchCall, MessageCodec, DefaultRuntime)]
#[serialization(borsh::BorshDeserialize, borsh::BorshSerialize)]
pub(crate) struct TestRuntime<C: Context, Da: DaSpec> {
    pub value_setter: ValueSetter<C>,
    pub chain_state: ChainState<C, Da>,
}

impl<C: Context, Da: DaSpec> TxHooks for TestRuntime<C, Da> {
    type Context = C;

    fn pre_dispatch_tx_hook(
        &self,
        tx: &Transaction<Self::Context>,
        _working_set: &mut sov_state::WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<<Self::Context as Spec>::Address> {
        Ok(tx.pub_key().to_address())
    }

    fn post_dispatch_tx_hook(
        &self,
        _tx: &Transaction<Self::Context>,
        _working_set: &mut sov_state::WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

impl<C: Context, Da: DaSpec> ApplyBlobHooks<Da::BlobTransaction> for TestRuntime<C, Da> {
    type Context = C;
    type BlobResult =
        SequencerOutcome<<<Da as DaSpec>::BlobTransaction as BlobReaderTrait>::Address>;

    fn begin_blob_hook(
        &self,
        _blob: &mut Da::BlobTransaction,
        _working_set: &mut sov_state::WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn end_blob_hook(
        &self,
        _result: Self::BlobResult,
        _working_set: &mut sov_state::WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

impl<C: Context, Da: DaSpec> SlotHooks<Da> for TestRuntime<C, Da> {
    type Context = C;

    fn begin_slot_hook(
        &self,
        slot_header: &Da::BlockHeader,
        validity_condition: &Da::ValidityCondition,
        working_set: &mut sov_state::WorkingSet<<Self::Context as Spec>::Storage>,
    ) {
        self.chain_state
            .begin_slot_hook(slot_header, validity_condition, working_set)
    }

    fn end_slot_hook(
        &self,
        _working_set: &mut sov_state::WorkingSet<<Self::Context as Spec>::Storage>,
    ) {
    }

    fn finalize_slot_hook(
        &self,
        _root_hash: [u8; 32],
        _accesorry_working_set: &mut sov_state::AccessoryWorkingSet<
            <Self::Context as Spec>::Storage,
        >,
    ) {
    }
}

impl<C, Da> BlobSelector<Da> for TestRuntime<C, Da>
where
    C: Context,
    Da: DaSpec,
{
    type Context = C;

    fn get_blobs_for_this_slot<'a, I>(
        &self,
        current_blobs: I,
        _working_set: &mut sov_state::WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<Vec<BlobRefOrOwned<'a, Da::BlobTransaction>>>
    where
        I: IntoIterator<Item = &'a mut Da::BlobTransaction>,
    {
        Ok(current_blobs.into_iter().map(Into::into).collect())
    }
}

impl<C: Context, Da: DaSpec> Runtime<C, Da> for TestRuntime<C, Da> {}

pub(crate) fn create_demo_genesis_config<C: Context, Da: DaSpec>(
    admin: <C as Spec>::Address,
) -> GenesisConfig<C, Da> {
    let value_setter_config = ValueSetterConfig { admin };
    let chain_state_config = ChainStateConfig {
        initial_slot_height: 0,
    };
    GenesisConfig::new(value_setter_config, chain_state_config)
}

/// Clones the [`AppTemplate`]'s [`Storage`] and extract the underlying [`WorkingSet`]
pub(crate) fn get_working_set<C: Context, Da: DaSpec>(
    app_template: &AppTemplate<C, Da, MockZkvm, TestRuntime<C, Da>>,
) -> sov_state::WorkingSet<<C as Spec>::Storage>
where
{
    sov_state::WorkingSet::new(app_template.current_storage.clone())
}
