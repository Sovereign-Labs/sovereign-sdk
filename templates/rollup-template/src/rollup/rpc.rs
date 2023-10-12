use anyhow::Context;
use sov_db::ledger_db::LedgerDB;
use sov_modules_api::{DaSpec, Zkvm};
use sov_modules_stf_template::{SequencerOutcome, TxEffect};
use sov_rollup_interface::services::da::DaService;
use sov_sequencer::get_sequencer_rpc;
use sov_stf_runner::get_ledger_rpc;
use template_stf::StfWithBuilder;

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

/// register ledger rpc methods.
pub(crate) fn register_ledger<Da: DaService>(
    ledger_db: LedgerDB,
    methods: &mut jsonrpsee::RpcModule<()>,
) -> Result<(), anyhow::Error> {
    let ledger_rpc =
        get_ledger_rpc::<SequencerOutcome<<Da::Spec as DaSpec>::Address>, TxEffect>(ledger_db);
    methods
        .merge(ledger_rpc)
        .context("Failed to merge ledger RPC modules")
}
