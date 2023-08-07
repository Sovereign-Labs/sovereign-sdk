use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::hooks::{ApplyBlobHooks, SlotHooks, TxHooks};
use sov_modules_api::transaction::Transaction;
use sov_modules_api::{Context, PublicKey, Spec};
use sov_modules_macros::{DefaultRuntime, DispatchCall, Genesis, MessageCodec};
use sov_modules_stf_template::{AppTemplate, SequencerOutcome};
use sov_rollup_interface::da::BlobReaderTrait;
use sov_rollup_interface::mocks::{MockZkvm, TestBlob, TestValidityCond};
use sov_state::WorkingSet;
use sov_value_setter::{ValueSetter, ValueSetterConfig};

use crate::ChainState;

type C = DefaultContext;

#[derive(Genesis, DispatchCall, MessageCodec, DefaultRuntime)]
#[serialization(borsh::BorshDeserialize, borsh::BorshSerialize)]
pub(crate) struct TestRuntime<C: Context> {
    pub value_setter: ValueSetter<C>,
    pub chain_state: ChainState<C, TestValidityCond>,
}

impl<C: Context> TxHooks for TestRuntime<C> {
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

impl<C: Context> ApplyBlobHooks for TestRuntime<C> {
    type Context = C;
    type BlobResult = SequencerOutcome;

    fn begin_blob_hook(
        &self,
        _blob: &mut impl BlobReaderTrait,
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

impl SlotHooks<TestValidityCond> for TestRuntime<C> {
    type Context = C;

    fn begin_slot_hook(
        &self,
        slot_data: &impl sov_rollup_interface::services::da::SlotData<Condition = TestValidityCond>,
        working_set: &mut sov_state::WorkingSet<<Self::Context as sov_modules_api::Spec>::Storage>,
    ) -> anyhow::Result<()> {
        self.chain_state.begin_slot_hook(slot_data, working_set)
    }

    fn end_slot_hook(&self, new_state_root: [u8; 32]) -> anyhow::Result<()> {
        self.chain_state.end_slot_hook(new_state_root)
    }
}

pub(crate) fn create_demo_genesis_config<C: Context>(
    admin: <C as Spec>::Address,
) -> GenesisConfig<C> {
    let value_setter_config = ValueSetterConfig { admin };

    GenesisConfig::new(value_setter_config, ())
}

/// Clones the [`AppTemplate`]'s [`Storage`] and extract the underlying [`WorkingSet`]
pub(crate) fn get_working_set<C: Context>(
    app_template: &AppTemplate<
        C,
        TestRuntime<C>,
        MockZkvm,
        TestValidityCond,
        TestBlob<<DefaultContext as Spec>::Address>,
    >,
) -> WorkingSet<<C as Spec>::Storage> {
    WorkingSet::new(app_template.current_storage.clone())
}
