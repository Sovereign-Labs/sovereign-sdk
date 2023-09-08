//! Full-Node specific RPC methods.

use anyhow::Context;
use demo_stf::app::App;
use sov_celestia_adapter::verifier::address::CelestiaAddress;
use sov_db::ledger_db::LedgerDB;
#[cfg(feature = "experimental")]
use sov_ethereum::experimental::EthRpcConfig;
use sov_modules_stf_template::{SequencerOutcome, TxEffect};
use sov_rollup_interface::services::da::DaService;
use sov_rollup_interface::zk::Zkvm;
use sov_sequencer::get_sequencer_rpc;
use sov_stf_runner::get_ledger_rpc;

/// register sequencer rpc methods.
pub fn register_sequencer<Vm, Da>(
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

/// register ledger rpc methods.
pub fn register_ledger(
    ledger_db: LedgerDB,
    methods: &mut jsonrpsee::RpcModule<()>,
) -> Result<(), anyhow::Error> {
    let ledger_rpc = get_ledger_rpc::<SequencerOutcome<CelestiaAddress>, TxEffect>(ledger_db);
    methods
        .merge(ledger_rpc)
        .context("Failed to merge ledger RPC modules")
}

#[cfg(feature = "experimental")]
/// register ethereum methods.
pub fn register_ethereum<Da: DaService>(
    da_service: Da,
    eth_rpc_config: EthRpcConfig,
    methods: &mut jsonrpsee::RpcModule<()>,
) -> Result<(), anyhow::Error> {
    let ethereum_rpc = sov_ethereum::get_ethereum_rpc(da_service, eth_rpc_config);

    methods
        .merge(ethereum_rpc)
        .context("Failed to merge Ethereum RPC modules")
}
