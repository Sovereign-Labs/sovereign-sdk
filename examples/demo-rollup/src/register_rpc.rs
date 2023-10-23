//! Full-Node specific RPC methods.

use anyhow::Context;
use demo_stf::App;
#[cfg(feature = "experimental")]
use sov_ethereum::experimental::EthRpcConfig;
use sov_rollup_interface::services::da::DaService;
use sov_rollup_interface::zk::Zkvm;
use sov_sequencer::get_sequencer_rpc;

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
