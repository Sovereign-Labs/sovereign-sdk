use sov_bank::{get_token_address, Bank, BankConfig, CallMessage, Coins, TotalSupplyResponse};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::hooks::{ApplyBlobHooks, SlotHooks, TxHooks};
use sov_modules_api::transaction::Transaction;
use sov_modules_api::{Address, Context, Hasher, PublicKey, Spec};
use sov_modules_macros::{DefaultRuntime, DispatchCall, Genesis, MessageCodec};
use sov_modules_stf_template::{AppTemplate, SequencerOutcome};
use sov_rollup_interface::da::BlobReaderTrait;
use sov_rollup_interface::mocks::{MockZkvm, TestBlob, TestValidityCond};
use sov_state::{DefaultStorageSpec, ProverStorage, WorkingSet};

use crate::ChainState;

type C = DefaultContext;

#[derive(Genesis, DispatchCall, MessageCodec, DefaultRuntime)]
#[serialization(borsh::BorshDeserialize, borsh::BorshSerialize)]
pub(crate) struct TestRuntime<C: Context> {
    pub bank: sov_bank::Bank<C>,
    pub chain_state: ChainState<C, TestValidityCond>,
}

impl<C: Context> TxHooks for TestRuntime<C> {
    type Context = C;

    fn pre_dispatch_tx_hook(
        &self,
        tx: &Transaction<Self::Context>,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<<Self::Context as Spec>::Address> {
        Ok(tx.pub_key().to_address())
    }

    fn post_dispatch_tx_hook(
        &self,
        tx: &Transaction<Self::Context>,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

impl<C: Context> ApplyBlobHooks for TestRuntime<C> {
    type Context = C;
    type BlobResult = SequencerOutcome;

    fn begin_blob_hook(
        &self,
        blob: &mut impl BlobReaderTrait,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn end_blob_hook(
        &self,
        result: Self::BlobResult,
        working_set: &mut WorkingSet<<Self::Context as Spec>::Storage>,
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

pub(crate) fn create_demo_genesis_config<C: Context>() -> GenesisConfig<C> {
    let bank_config = BankConfig::<C> { tokens: vec![] };

    let token_address = sov_bank::get_genesis_token_address::<C>(
        &bank_config.tokens[0].token_name,
        bank_config.tokens[0].salt,
    );

    GenesisConfig::new(bank_config, ())
}
