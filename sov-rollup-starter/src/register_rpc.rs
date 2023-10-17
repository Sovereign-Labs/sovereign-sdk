use anyhow::Context;
use sov_modules_api::Zkvm;
use sov_rollup_interface::services::da::DaService;
use sov_sequencer::get_sequencer_rpc;
use stf_starter::StfWithBuilder;

/// register sequencer rpc methods.
pub(crate) fn register_sequencer<Vm, Da>(
    da_service: Da,
    app: &mut StfWithBuilder<Vm, Da::Spec>,
    methods: &mut jsonrpsee::RpcModule<()>,
) -> Result<(), anyhow::Error>
where
    Da: DaService,
    Vm: Zkvm,
{
    let batch_builder = app.batch_builder.take().unwrap();
    let sequencer_rpc = get_sequencer_rpc(batch_builder, da_service);
    methods
        .merge(sequencer_rpc)
        .context("Failed to merge Txs RPC modules")
}
