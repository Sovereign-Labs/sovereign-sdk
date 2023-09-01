//! Full-Node specific RPC methods.

use anyhow::Context;
use celestia::verifier::address::CelestiaAddress;
use demo_stf::app::App;
use sov_db::ledger_db::LedgerDB;
use sov_modules_stf_template::{SequencerOutcome, TxEffect};
use sov_rollup_interface::services::da::DaService;
use sov_rollup_interface::zk::Zkvm;
use sov_sequencer::get_sequencer_rpc;
use sov_stf_runner::get_ledger_rpc;

#[cfg(feature = "experimental")]
const TX_SIGNER_PRIV_KEY_PATH: &str = "../test-data/keys/tx_signer_private_key.json";

/// register sequencer rpc methods.
pub fn register_sequencer<Vm, DA>(
    da_service: DA,
    app: &mut App<Vm, DA::Spec>,
    methods: &mut jsonrpsee::RpcModule<()>,
) -> Result<(), anyhow::Error>
where
    DA: DaService,
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
pub fn register_ethereum<DA: DaService>(
    da_service: DA,
    methods: &mut jsonrpsee::RpcModule<()>,
) -> Result<(), anyhow::Error> {
    use std::fs;

    let data = fs::read_to_string(TX_SIGNER_PRIV_KEY_PATH).context("Unable to read file")?;

    let hex_key: sov_cli::wallet_state::HexPrivateAndAddress =
        serde_json::from_str(&data).context("JSON does not have a correct format.")?;

    let key_and_address: sov_cli::wallet_state::PrivateKeyAndAddress<
        sov_modules_api::default_context::DefaultContext,
    > = hex_key
        .try_into()
        .expect("Failed to parse sequencer private key and address");

    let ethereum_rpc = sov_ethereum::get_ethereum_rpc(da_service, key_and_address.private_key);
    methods
        .merge(ethereum_rpc)
        .context("Failed to merge Ethereum RPC modules")
}
