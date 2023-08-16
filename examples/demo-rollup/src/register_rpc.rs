//! TODO
use std::sync::Arc;

use anyhow::Context;
use demo_stf::app::App;
use jupiter::verifier::address::CelestiaAddress;
use risc0_adapter::host::Risc0Verifier;
use sov_db::ledger_db::LedgerDB;
use sov_modules_stf_template::{SequencerOutcome, TxEffect};
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::services::da::DaService;
use sov_sequencer::get_sequencer_rpc;
use sov_stf_runner::get_ledger_rpc;

///
pub fn register_sequencer<DA>(
    da_service: DA,
    demo_runner: &mut App<
        Risc0Verifier,
        <DA::Spec as DaSpec>::ValidityCondition,
        <DA::Spec as DaSpec>::BlobTransaction,
    >,
    methods: &mut jsonrpsee::RpcModule<()>,
) -> Result<(), anyhow::Error>
where
    DA: DaService<Error = anyhow::Error> + Send + Sync + 'static,
{
    let batch_builder = demo_runner.batch_builder.take().unwrap();
    let sequencer_rpc = get_sequencer_rpc(batch_builder, Arc::new(da_service));
    methods
        .merge(sequencer_rpc)
        .context("Failed to merge Txs RPC modules")
}

///
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
pub fn register_ethereum(
    da_config: DaServiceConfig,
    methods: &mut jsonrpsee::RpcModule<()>,
) -> Result<(), anyhow::Error> {
    use std::fs;

    let data = fs::read_to_string(TX_SIGNER_PRIV_KEY_PATH).context("Unable to read file")?;

    let hex_key: HexKey =
        serde_json::from_str(&data).context("JSON does not have correct format.")?;

    let tx_signer_private_key = DefaultPrivateKey::from_hex(&hex_key.hex_priv_key).unwrap();

    let ethereum_rpc = get_ethereum_rpc(da_config, tx_signer_private_key);
    methods
        .merge(ethereum_rpc)
        .context("Failed to merge Ethereum RPC modules")
}
