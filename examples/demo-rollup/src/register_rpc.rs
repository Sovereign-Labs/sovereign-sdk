//! Full-Node specific RPC methods.

use anyhow::Context;
use demo_stf::App;
#[cfg(feature = "experimental")]
use sov_ethereum::experimental::EthRpcConfig;
use sov_rollup_interface::services::da::DaService;
use sov_rollup_interface::zk::Zkvm;
use sov_sequencer::get_sequencer_rpc;

// register sequencer rpc methods.
pub(crate) fn register_sequencer<Vm, Da>(
    da_service: Da,
    app: &mut App<Vm, Da::Spec>,
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

#[cfg(feature = "experimental")]
// register ethereum methods.
pub(crate) fn register_ethereum<C: sov_modules_api::Context, Da: DaService>(
    da_service: Da,
    eth_rpc_config: EthRpcConfig<C>,
    storage: C::Storage,
    methods: &mut jsonrpsee::RpcModule<()>,
) -> Result<(), anyhow::Error> {
    let ethereum_rpc = sov_ethereum::get_ethereum_rpc::<C, Da>(da_service, eth_rpc_config, storage);

    methods
        .merge(ethereum_rpc)
        .context("Failed to merge Ethereum RPC modules")
}
