use anyhow::Context as _;
use sov_db::ledger_db::LedgerDB;
use sov_modules_api::{Context, Spec};
use sov_modules_stf_blueprint::{Runtime as RuntimeTrait, SequencerOutcome, TxEffect};
use sov_rollup_interface::services::da::DaService;
use sov_sequencer::batch_builder::FiFoStrictBatchBuilder;

/// Register rollup's default rpc methods.
pub fn register_rpc<RT, C, Da>(
    storage: &<C as Spec>::Storage,
    ledger_db: &LedgerDB,
    da_service: &Da,
    sequencer: C::Address,
) -> Result<jsonrpsee::RpcModule<()>, anyhow::Error>
where
    RT: RuntimeTrait<C, <Da as DaService>::Spec> + Send + Sync + 'static,
    C: Context,
    Da: DaService + Clone,
{
    // runtime rpc.
    let mut rpc_methods = RT::rpc_methods(storage.clone());

    // ledger rpc.
    {
        rpc_methods.merge(sov_ledger_rpc::server::rpc_module::<
            LedgerDB,
            SequencerOutcome<<C as Spec>::Address>,
            TxEffect,
        >(ledger_db.clone())?)?;
    }

    // sequencer rpc.
    {
        let batch_builder = FiFoStrictBatchBuilder::new(
            1024 * 100,
            u32::MAX as usize,
            RT::default(),
            storage.clone(),
            sequencer,
        );

        let sequencer_rpc = sov_sequencer::get_sequencer_rpc(batch_builder, da_service.clone());
        rpc_methods
            .merge(sequencer_rpc)
            .context("Failed to merge Txs RPC modules")?;
    }

    Ok(rpc_methods)
}
