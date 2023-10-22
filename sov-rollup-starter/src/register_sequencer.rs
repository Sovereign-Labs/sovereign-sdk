use anyhow::Context;
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::Spec;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::services::da::DaService;
use sov_sequencer::batch_builder::FiFoStrictBatchBuilder;
use sov_sequencer::get_sequencer_rpc;
use sov_state::ProverStorage;
use stf_starter::Runtime;

/// register sequencer rpc methods.
pub(crate) fn register_sequencer<Da>(
    storage: &<DefaultContext as Spec>::Storage,
    da_service: Da,
    methods: &mut jsonrpsee::RpcModule<()>,
) -> Result<(), anyhow::Error>
where
    Da: DaService,
{
    let batch_builder = create_batch_builder::<<Da as DaService>::Spec>(storage.clone());
    let sequencer_rpc = get_sequencer_rpc(batch_builder, da_service);
    methods
        .merge(sequencer_rpc)
        .context("Failed to merge Txs RPC modules")
}

fn create_batch_builder<Da: DaSpec>(
    storage: ProverStorage<sov_state::DefaultStorageSpec>,
) -> FiFoStrictBatchBuilder<DefaultContext, Runtime<DefaultContext, Da>> {
    let batch_size_bytes = 1024 * 100; // max allowed batch size = 100 KB
    FiFoStrictBatchBuilder::new(
        batch_size_bytes,
        u32::MAX as usize,
        Runtime::default(),
        storage,
    )
}
