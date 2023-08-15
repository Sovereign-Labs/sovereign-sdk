use sov_chain_state::{ChainState, ChainStateConfig};
use sov_modules_api::capabilities::{BlobRefOrOwned, BlobSelector};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::hooks::{ApplyBlobHooks, SlotHooks, TxHooks};
use sov_modules_api::transaction::Transaction;
use sov_modules_api::{Context, PublicKey, Spec};
use sov_modules_macros::{DefaultRuntime, DispatchCall, Genesis, MessageCodec};
use sov_modules_stf_template::{AppTemplate, Runtime, SequencerOutcome};
use sov_rollup_interface::da::BlobReaderTrait;
use sov_rollup_interface::mocks::{MockZkvm, TestBlob};
use sov_rollup_interface::zk::ValidityCondition;
use sov_state::WorkingSet;
use sov_value_setter::{ValueSetter, ValueSetterConfig};

#[derive(Genesis, DispatchCall, MessageCodec, DefaultRuntime)]
#[serialization(borsh::BorshDeserialize, borsh::BorshSerialize)]
pub(crate) struct TestRuntime<C: Context, Cond: ValidityCondition> {
    pub value_setter: ValueSetter<C>,
    pub chain_state: ChainState<C, Cond>,
}

impl<C: Context, Cond: ValidityCondition> TxHooks for TestRuntime<C, Cond> {
    type Context = C;

    fn pre_dispatch_tx_hook(
        &self,
        tx: &Transaction<Self::Context>,
        _working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<<Self::Context as Spec>::Address> {
        Ok(tx.pub_key().to_address())
    }

    fn post_dispatch_tx_hook(
        &self,
        _tx: &Transaction<Self::Context>,
        _working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

impl<C: Context, Cond: ValidityCondition, B: BlobReaderTrait> ApplyBlobHooks<B>
    for TestRuntime<C, Cond>
{
    type Context = C;
    type BlobResult = SequencerOutcome<B::Address>;

    fn begin_blob_hook(
        &self,
        _blob: &mut B,
        _working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn end_blob_hook(
        &self,
        _result: Self::BlobResult,
        _working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

impl<C: Context, Cond: ValidityCondition> SlotHooks<Cond> for TestRuntime<C, Cond> {
    type Context = C;

    fn begin_slot_hook(
        &self,
        slot_data: &impl sov_rollup_interface::services::da::SlotData<Cond = Cond>,
        working_set: &mut sov_state::WorkingSet<<Self::Context as sov_modules_api::Spec>::Storage>,
    ) {
        self.chain_state.begin_slot_hook(slot_data, working_set)
    }

    fn end_slot_hook(&self, _working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>) {}
}

impl<C: Context, Cond: ValidityCondition> BlobSelector for TestRuntime<C, Cond> {
    type Context = C;

    fn get_blobs_for_this_slot<'a, I, B>(
        &self,
        current_blobs: I,
        _working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<Vec<BlobRefOrOwned<'a, B>>>
    where
        B: BlobReaderTrait,
        I: IntoIterator<Item = &'a mut B>,
    {
        Ok(current_blobs.into_iter().map(Into::into).collect())
    }
}

impl<C: Context, Cond: ValidityCondition, B: BlobReaderTrait> Runtime<C, Cond, B>
    for TestRuntime<C, Cond>
{
}

pub(crate) fn create_demo_genesis_config<C: Context, Cond: ValidityCondition>(
    admin: <C as Spec>::Address,
) -> GenesisConfig<C, Cond> {
    let value_setter_config = ValueSetterConfig { admin };
    let chain_state_config = ChainStateConfig {
        initial_slot_height: 0,
    };
    GenesisConfig::new(value_setter_config, chain_state_config)
}

/// Clones the [`AppTemplate`]'s [`Storage`] and extract the underlying [`WorkingSet`]
pub(crate) fn get_working_set<C: Context, Cond: ValidityCondition>(
    app_template: &AppTemplate<
        C,
        Cond,
        MockZkvm,
        TestRuntime<C, Cond>,
        TestBlob<<DefaultContext as Spec>::Address>,
    >,
) -> WorkingSet<<C as Spec>::Storage> {
    WorkingSet::new(app_template.current_storage.clone())
}
