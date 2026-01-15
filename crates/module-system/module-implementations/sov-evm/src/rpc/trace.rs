use std::ops::DerefMut;

use alloy_eips::BlockId;
use alloy_eips::BlockNumberOrTag;
use alloy_primitives::Address;
use alloy_primitives::B256;
use alloy_rpc_types_trace::geth::{
    GethDebugBuiltInTracerType, GethDebugTracerType, GethDebugTracingOptions, GethTrace,
    TraceResult,
};
use revm::context::result::ExecResultAndState;
use revm::context::{BlockEnv, CfgEnv, TxEnv};
use revm_database_interface::TryDatabaseCommit;
use revm_inspectors::tracing::{TracingInspector, TracingInspectorConfig};
use sov_address::{EthereumAddress, FromVmAddress};
use sov_modules_api::{ApiStateAccessor, Spec};
use sov_rpc_eth_types::EthApiError;

use super::maybe_archival_state::MaybeArchivalState;
use crate::conversions::replay_tx_env;
use crate::db::EvmDb;
use crate::error::into_rpc_error;
use crate::evm::primitive_types::{MaybeSealedBlock, TxSignedAndRecovered};
use crate::executor::{get_cfg_env, inspect_with_precompiles, transact_commit};
use crate::sov_evm::SovPrecompiles;
use crate::Evm;

impl<S: Spec> Evm<S>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    /// Pre-load transactions from a block to avoid borrow conflicts
    pub(super) fn preload_block_transactions(
        &self,
        maybe_block: &MaybeSealedBlock,
        state: &mut ApiStateAccessor<S>,
    ) -> Result<Vec<TxSignedAndRecovered>, EthApiError> {
        maybe_block
            .tx_range()
            .map(|tx_idx| {
                self.transaction(tx_idx, state)
                    .ok_or_else(|| EthApiError::PrunedHistoryUnavailable)
            })
            .collect()
    }

    /// Setup execution environment for tracing (handles both pending and sealed blocks)
    /// Takes a block number, fetches it, and sets up everything needed for tracing
    pub(super) fn setup_trace_execution<'a>(
        &'a self,
        block_number: u64,
        state: &'a mut ApiStateAccessor<S>,
    ) -> Result<
        (
            MaybeArchivalState<'a, S>,
            Vec<TxSignedAndRecovered>,
            BlockEnv,
            CfgEnv,
        ),
        EthApiError,
    > {
        // Get the block - could be pending or sealed
        let maybe_block = self
            .get_maybe_sealed_block(block_number, state)
            .ok_or_else(|| EthApiError::HeaderNotFound(BlockId::number(block_number)))?;

        // Pre-load transactions to avoid borrow conflicts
        let transactions = self.preload_block_transactions(&maybe_block, state)?;

        let is_pending = matches!(maybe_block, MaybeSealedBlock::Pending(_));

        let mut maybe_archival_state: MaybeArchivalState<'a, S> = if is_pending {
            state.into()
        } else {
            let archival = self.archival_state_pre_block(maybe_block.number(), state)?;
            Box::new(archival).into()
        };
        let state = maybe_archival_state.deref_mut();

        let mut block_env = self.block_env(state)?;
        block_env.basefee = 0; // Set the base fee to zero for evm execution. Gas is paid for by the sov gas meter instead
        let cfg = self.cfg(state)?;
        let cfg_env = get_cfg_env(&block_env, &cfg, None);

        Ok((maybe_archival_state, transactions, block_env, cfg_env))
    }

    pub(super) fn trace_block_by_number(
        &self,
        block: BlockNumberOrTag,
        opts: GethDebugTracingOptions,
        state: &mut ApiStateAccessor<S>,
    ) -> Result<Vec<TraceResult>, EthApiError> {
        let block_number = self.resolve_block_number(block, state);

        // Setup execution environment (fetches block, preloads transactions, sets up state)
        let (mut state, txs_to_trace, block_env, cfg_env) =
            self.setup_trace_execution(block_number, state)?;

        // Load enabled precompiles before creating db (to avoid borrow conflicts)
        let enabled_custom_precompiles = self
            .get_enabled_sov_precompiles(state.deref_mut())
            .map_err(|e| EthApiError::other(into_rpc_error(e)))?;

        let mut evm_db = self.db(state.deref_mut());

        // Trace all transactions in the block
        let mut traces = vec![];

        for tx in txs_to_trace {
            let result = self.trace_transaction_inner(
                &block_env,
                replay_tx_env(&tx),
                cfg_env.clone(),
                &mut evm_db,
                &opts,
                &enabled_custom_precompiles,
            )?;
            traces.push(TraceResult::new_success(result, Some(*tx.hash())));
        }

        Ok(traces)
    }

    pub(super) fn trace_transaction(
        &self,
        tx_hash: B256,
        opts: GethDebugTracingOptions,
        state: &mut ApiStateAccessor<S>,
    ) -> Result<GethTrace, EthApiError> {
        // Get transaction - could be in pending_transactions or sealed blocks
        let tx_number = self
            .tx_index(&tx_hash, state)
            .ok_or(EthApiError::PrunedHistoryUnavailable)?;
        let traced_tx = self
            .transaction(tx_number, state)
            .ok_or(EthApiError::PrunedHistoryUnavailable)?;

        // Setup execution environment (fetches block, preloads transactions, sets up state)
        let (mut state, txs_to_replay, block_env, cfg_env) =
            self.setup_trace_execution(traced_tx.block_number, state)?;

        // Load enabled precompiles before creating db (to avoid borrow conflicts)
        let enabled_custom_precompiles = self
            .get_enabled_sov_precompiles(state.deref_mut())
            .map_err(|e| EthApiError::other(into_rpc_error(e)))?;

        let mut evm_db = self.db(state.deref_mut());

        // Replay previous transactions in the block
        for tx in txs_to_replay {
            // As soon as we reach target tx - exit the loop
            if *tx.signed_transaction.hash() == tx_hash {
                break;
            }

            // Create precompiles for each replay (moved into transact_commit)
            let precompiles = SovPrecompiles::new(enabled_custom_precompiles.clone(), &self.bank_module);
            transact_commit(
                &mut evm_db,
                &block_env,
                replay_tx_env(&tx),
                cfg_env.clone(),
                precompiles,
            )
            .map_err(EthApiError::from)?;
        }

        // Trace the target transaction
        self.trace_transaction_inner(
            &block_env,
            replay_tx_env(&traced_tx),
            cfg_env,
            &mut evm_db,
            &opts,
            &enabled_custom_precompiles,
        )
    }

    pub(super) fn trace_transaction_inner(
        &self,
        block_env: &BlockEnv,
        tx_env: TxEnv,
        cfg: CfgEnv,
        db: &mut EvmDb<ApiStateAccessor<S>, S>,
        opts: &GethDebugTracingOptions,
        enabled_custom_precompiles: &Vec<Address>,
    ) -> Result<GethTrace, EthApiError> {
        let GethDebugTracingOptions {
            tracer,
            tracer_config,
            ..
        } = opts;
        if let Some(tracer) = tracer {
            return match tracer {
                GethDebugTracerType::BuiltInTracer(GethDebugBuiltInTracerType::CallTracer) => {
                    let call_config = tracer_config
                        .clone()
                        .into_call_config()
                        .map_err(|_| EthApiError::InvalidTracerConfig)?;

                    let inspector_config =
                        TracingInspectorConfig::from_geth_call_config(&call_config);
                    let mut inspector = TracingInspector::new(inspector_config);

                    // Create precompile provider with sovereign state access
                    let precompiles =
                        SovPrecompiles::new(enabled_custom_precompiles.clone(), &self.bank_module);

                    let gas_limit = tx_env.gas_limit;
                    let ExecResultAndState { result, state } = inspect_with_precompiles(
                        &mut *db,
                        block_env,
                        tx_env,
                        cfg,
                        &mut inspector,
                        precompiles,
                    )?;
                    db.try_commit(state)?;

                    inspector.set_transaction_gas_limit(gas_limit);
                    let frame = inspector
                        .geth_builder()
                        .geth_call_traces(call_config, result.gas_used());

                    Ok(frame.into())
                }
                _ => Err(EthApiError::Unsupported("unsupported tracer")),
            };
        }
        Err(EthApiError::Unsupported("unsupported tracer"))
    }
}
