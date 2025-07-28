use sov_modules_api::capabilities::{
    AuthenticationError, ChainState, GasEnforcer, SequencerAuthorization, TransactionAuthenticator,
    TransactionAuthorizer,
};
use sov_modules_api::transaction::TransactionConsumption;
use sov_modules_api::{
    Amount, BasicGasMeter, BatchSequencerOutcome, BatchSequencerReceipt, DaSpec, ExecutionContext,
    FullyBakedTx, Gas, GasArray, GasMeter, GasSpec, GetGasPrice, IgnoredTransactionReceipt,
    IncrementalBatch, InjectedControlFlow, PreExecWorkingSet, ProvisionalSequencerOutcome, Rewards,
    SequencerBondForTx, SlotGasMeter, Spec, StateCheckpoint, StateProvider, TransactionReceipt,
    TxControlFlow, TxScratchpad, WorkingSet, *,
};
use sov_rollup_interface::TxHash;
use tracing::{trace, warn};

pub use crate::sequencer_mode::common::PreExecError;
use crate::sequencer_mode::common::{
    apply_batch_logs, apply_tx, create_tx_receipt, get_gas_used, BatchReceipt,
};
use crate::{ApplyTxResult, AuthTxOutput, Runtime, TxReceiptContents};

type TxAndError = (TxProcessingError, FullyBakedTx);

/// Executes the entire transaction lifecycle.
///
/// The caller is responsible for penalizing the sequencer if this method returns an error. If the tx can be attempted,
/// this method must return Ok(()) and handle any sequencer rewards internally.
#[allow(clippy::result_large_err, clippy::too_many_arguments)]
#[cfg_attr(feature = "native", tracing::instrument(skip_all, name = "StfBlueprint::process_tx", fields(context = ?execution_context)))]
#[cfg_attr(feature = "bench", sov_modules_api::cycle_tracker)]
pub fn process_tx_and_reward_prover<S, R, I, C>(
    runtime: &mut R,
    pre_exec_working_set: PreExecWorkingSet<S, I>,
    slot_gas: &S::Gas,
    validated_output: AuthTxOutput<S, R>,
    raw_tx: FullyBakedTx,
    sequencer_da_address: &<S::Da as DaSpec>::Address,
    sequencer_rollup_address: S::Address,
    #[allow(unused_variables)] execution_context: ExecutionContext,
    injected_control_flow: &C,
    operating_mode: OperatingMode,
) -> (
    Result<ApplyTxResult<S>, TxAndError>,
    TxScratchpad<S, I>,
    BasicGasMeter<S>,
)
where
    S: Spec,
    R: Runtime<S>,
    I: StateProvider<S>,
    C: InjectedControlFlow<S>,
{
    #[cfg(feature = "native")]
    let visible_slot_number =
        sov_modules_api::VersionReader::current_visible_slot_number(&pre_exec_working_set);

    #[cfg(feature = "native")]
    let (start, discriminant) = {
        (
            std::time::Instant::now(),
            call_message_repr::<R>(&validated_output.2),
        )
    };

    let result = process_tx_and_reward_prover_inner(
        runtime,
        pre_exec_working_set,
        slot_gas,
        validated_output,
        raw_tx,
        sequencer_da_address,
        sequencer_rollup_address,
        injected_control_flow,
        operating_mode,
    );

    #[cfg(feature = "native")]
    track_transaction_metrics(
        &result.0,
        start.elapsed(),
        execution_context,
        visible_slot_number,
        sequencer_da_address,
        discriminant,
        &result.2,
    );

    result
}

#[cfg(feature = "native")]
fn track_transaction_metrics<S: Spec>(
    result: &Result<ApplyTxResult<S>, TxAndError>,
    execution_time: std::time::Duration,
    execution_context: ExecutionContext,
    visible_slot_number: sov_rollup_interface::common::VisibleSlotNumber,
    sequencer_address: &<S::Da as DaSpec>::Address,
    message_discriminant: String,
    basic_gas_meter: &BasicGasMeter<S>,
) {
    sov_metrics::track_metrics(|metrics_tracker| {
        let tx_effect = match result {
            Ok(tx_result) => sov_metrics::TransactionEffect::from(&tx_result.receipt.receipt),
            Err(_) => sov_metrics::TransactionEffect::Skipped,
        };

        let gas_used = basic_gas_meter.gas_info().gas_used;

        let transaction_metrics = sov_metrics::TransactionProcessingMetrics {
            execution_time,
            tx_effect,
            execution_context,
            visible_slot_number,
            sequencer_address: sequencer_address.to_string(),
            call_message: message_discriminant,
            gas_used: gas_used.as_ref().to_vec(),
        };

        metrics_tracker.submit(transaction_metrics);
    });
}

/// Actual processing of transaction.
///
/// The caller is responsible for penalizing the sequencer if this method returns an error. If the tx can be attempted,
/// this method must return Ok(()) and handle any sequencer rewards internally.
#[allow(clippy::result_large_err, clippy::too_many_arguments)]
fn process_tx_and_reward_prover_inner<S, R, I, C>(
    runtime: &mut R,
    mut pre_exec_working_set: PreExecWorkingSet<S, I>,
    slot_gas: &S::Gas,
    validated_output: AuthTxOutput<S, R>,
    raw_tx: FullyBakedTx,
    sequencer_da_address: &<S::Da as DaSpec>::Address,
    sequencer_rollup_address: S::Address,
    injected_control_flow: &C,
    operating_mode: OperatingMode,
) -> (
    Result<ApplyTxResult<S>, TxAndError>,
    TxScratchpad<S, I>,
    BasicGasMeter<S>,
)
where
    S: Spec,
    R: Runtime<S>,
    I: StateProvider<S>,
    C: InjectedControlFlow<S>,
{
    let (auth_tx, auth_data, message) = validated_output;

    let raw_tx_hash = auth_tx.raw_tx_hash;
    let tx = &auth_tx.authenticated_tx;

    let maybe_ctx = runtime.transaction_authorizer().resolve_context(
        &auth_data,
        sequencer_da_address,
        sequencer_rollup_address,
        &mut pre_exec_working_set,
    );

    let mut ctx = match maybe_ctx {
        Ok(ctx) => ctx,
        Err(err) => {
            let (scratchpad, pre_exec_gas_meter) = pre_exec_working_set.revert();
            return (
                Err((
                    TxProcessingError::CannotResolveContext(err.to_string()),
                    raw_tx,
                )),
                scratchpad,
                pre_exec_gas_meter,
            );
        }
    };

    match injected_control_flow.pre_flight(runtime, &ctx, &message) {
        TxControlFlow::ContinueProcessing(_) => {}
        TxControlFlow::IgnoreTx => {
            let (scratchpad, pre_exec_gas_meter) = pre_exec_working_set.revert();
            return (
                Err((TxProcessingError::RejectedByPreFlight, raw_tx)),
                scratchpad,
                pre_exec_gas_meter,
            );
        }
    }

    // Check that the transaction isn't a duplicate
    if let Err(err) = runtime.transaction_authorizer().check_uniqueness(
        &auth_data,
        &ctx,
        &mut pre_exec_working_set,
    ) {
        let (scratchpad, pre_exec_gas_meter) = pre_exec_working_set.revert();
        return (
            Err((
                TxProcessingError::CheckUniquenessFailed(err.to_string()),
                raw_tx,
            )),
            scratchpad,
            pre_exec_gas_meter,
        );
    }

    if let Err(err) = runtime.transaction_authorizer().mark_tx_attempted(
        &auth_data,
        sequencer_da_address,
        &mut pre_exec_working_set,
    ) {
        let (scratchpad, pre_exec_gas_meter) = pre_exec_working_set.revert();
        return (
            Err((
                TxProcessingError::MarkTxAttemptedFailed(err.to_string()),
                raw_tx,
            )),
            scratchpad,
            pre_exec_gas_meter,
        );
    }

    let gas_price = pre_exec_working_set.gas_price().clone();
    if let Err(err) =
        runtime
            .gas_enforcer()
            .try_reserve_gas(tx, &gas_price, &mut ctx, &mut pre_exec_working_set)
    {
        let (scratchpad, pre_exec_gas_meter) = pre_exec_working_set.revert();
        return (
            Err((TxProcessingError::CannotReserveGas(err.to_string()), raw_tx)),
            scratchpad,
            pre_exec_gas_meter,
        );
    }

    let (scratchpad, pre_exec_gas_meter) = pre_exec_working_set.to_scratchpad_and_gas_meter();

    // The transaction will execute until one of the following conditions is met:
    // 1. It consumes more funds than `tx.max_fee`.
    // 2. The `Gas::calculate_min(tx.gas_limit, slot_gas)` is exhausted.
    let working_set_gas_meter = tx.gas_meter(&pre_exec_gas_meter.gas_info().gas_price, slot_gas);
    let mut working_set = WorkingSet::create_working_set(scratchpad, tx, working_set_gas_meter);

    // Recover the authentication cost from the user.
    if let Err(err) = working_set.charge_gas(&pre_exec_gas_meter.gas_info().gas_used) {
        let (mut scratchpad, transaction_consumption) = working_set.revert();

        // Refund the remaining gas to the sender.
        runtime.gas_enforcer().refund_remaining_gas(
            ctx.gas_refund_recipient(),
            &transaction_consumption.remaining_funds(),
            &mut scratchpad,
        );

        return (
            Err((TxProcessingError::OutOfGas(err.to_string()), raw_tx)),
            scratchpad,
            pre_exec_gas_meter,
        );
    }

    // If the transaction is valid, execute it and apply the changes to the state.
    let (apply_tx, mut scratchpad) =
        apply_tx(runtime, &ctx, tx, raw_tx_hash, raw_tx, message, working_set);

    let transaction_consumption = &apply_tx.transaction_consumption;

    runtime.gas_enforcer().refund_remaining_gas(
        ctx.gas_refund_recipient(),
        &transaction_consumption.remaining_funds(),
        &mut scratchpad,
    );

    runtime.gas_enforcer().reward_prover(
        &transaction_consumption.base_fee_value(),
        operating_mode,
        &mut scratchpad,
    );

    (Ok(apply_tx), scratchpad, pre_exec_gas_meter)
}

#[cfg_attr(feature = "bench", sov_modules_api::cycle_tracker)]
#[cfg_attr(
    feature = "native",
    tracing::instrument(skip_all, name = "StfBlueprint::authenticate")
)]
fn deserialize_and_authenticate<S: Spec, R: Runtime<S>, I: StateProvider<S>>(
    tx: &FullyBakedTx,
    pre_exec_working_set: &mut PreExecWorkingSet<S, I>,
) -> Result<AuthTxOutput<S, R>, AuthenticationError> {
    let (tx, auth_data, call) = R::Auth::authenticate(tx, pre_exec_working_set)?;
    Ok((tx, auth_data, R::wrap_call(call)))
}

pub struct IncrementalBatchReceipt<S: Spec> {
    pub tx_receipts: Vec<TransactionReceipt<S>>,
    pub ignored_tx_receipts: Vec<IgnoredTransactionReceipt<TxReceiptContents<S>>>,
    pub inner: BatchSequencerReceipt<S>,
}

impl<S: Spec> IncrementalBatchReceipt<S> {
    pub fn finalize(self, id: [u8; 32]) -> BatchReceipt<S> {
        BatchReceipt {
            batch_hash: id,
            tx_receipts: self.tx_receipts,
            ignored_tx_receipts: self.ignored_tx_receipts,
            inner: self.inner,
        }
    }
}

#[tracing::instrument(skip_all, name = "StfBlueprint::apply_batch", fields(context = ?execution_context))]
#[allow(clippy::too_many_arguments)]
#[cfg_attr(feature = "bench", sov_modules_api::cycle_tracker)]
pub(crate) fn apply_batch<S, RT, B>(
    runtime: &mut RT,
    mut checkpoint: StateCheckpoint<S>,
    slot_gas_meter: &mut SlotGasMeter<S>,
    mut batch_with_id: B,
    blob_idx: usize,
    sequencer_da_address: &<S::Da as DaSpec>::Address,
    sequencer_bond: Amount,
    gas_price: &<S::Gas as Gas>::Price,
    execution_context: ExecutionContext,
) -> (IncrementalBatchReceipt<S>, StateCheckpoint<S>)
where
    S: Spec,
    RT: Runtime<S>,
    B: IncrementalBatch<S>,
{
    let span = if let Some(id) = batch_with_id.id() {
        tracing::info_span!("batch", batch_id = hex::encode(id)).entered()
    } else {
        tracing::info_span!("sequencer-batch").entered()
    };

    trace!(
        sequencer_da_address = %sequencer_da_address,
        ?gas_price,
        "Applying a batch"
    );

    batch_with_id.pre_flight(&mut checkpoint);

    // We require non-preferred sequencer to bond for their entire batch up front.
    // However, the *preferred* sequencer streams transactions, so it can't know the total number of transactions in advance.
    // Because of that, we allow the preferred sequencer to bond enough for only a single transaction and we do accounting in real time.
    let is_preferred_sequencer = runtime
        .sequencer_authorization()
        .is_preferred_sequencer(sequencer_da_address, &mut checkpoint);

    let operating_mode = runtime.chain_state().operating_mode(&mut checkpoint);

    trace!("Verifying & executing transactions");

    // Cost of the authentication for the entire batch.
    // It should include the costs of `authentication` and process_tx pre-execution checks.
    if !execution_context.is_sequencer() {
        assert!(
            batch_with_id.known_remaining_txs().is_some(),
            "Batch sizes are always known by the time the batch appears on the DA Layer"
        );
    }

    let mut tx_receipts = Vec::with_capacity(batch_with_id.known_remaining_txs().unwrap_or(128));
    let mut ignored_tx_receipts = Vec::default();

    let mut accumulated_reward = Amount::ZERO;
    let mut accumulated_penalty = Amount::ZERO;
    let sequencer_address = batch_with_id.sequencer_address();

    let mut sequencer_bond_per_tx = if is_preferred_sequencer {
        SequencerBondForTx::Preferred(sequencer_bond)
    } else {
        // Split the bond evenly across all the transactions in the batch.
        let divisor = batch_with_id
            .known_remaining_txs()
            .expect("Batch sizes from non-preferred sequencers are always known in advance")
            .max(1) as u128;
        let amount = sequencer_bond
            .checked_div(Amount::new(divisor))
            // SAFETY: We know that `divisor` is always greater than because we call ``.max(1)` immediately` above.
            .expect("Divison by zero");
        SequencerBondForTx::Standard(amount)
    };
    let initial_slot_gas_used = slot_gas_meter.total_gas_used();

    checkpoint.commit_revertable_storage_cache();

    let mut clean_scratchpad = checkpoint.to_tx_scratchpad();

    for (idx, (raw_tx, injected_control_flow)) in batch_with_id.enumerate() {
        // Authorize and process the transaction, handling sequencer rewards/penalties internally.
        // The caller is responsible for maintaining the global gas limit.
        let AuthAndProcessOutput {
            gas_used,
            scratchpad: dirty_scratchpad,
            outcome,
        } = auth_and_process_tx_and_incentivize_sequencer(
            runtime,
            clean_scratchpad,
            // Here we make sure that a tx can't use more gas that remaining gas in the slot gas meter.
            slot_gas_meter.remaining_slot_gas(sequencer_da_address),
            raw_tx,
            sequencer_da_address,
            sequencer_address.clone(),
            gas_price,
            execution_context,
            sequencer_bond_per_tx,
            idx,
            &injected_control_flow,
            operating_mode,
        );

        let provisional_outcome = match outcome {
            AuthAndProcessOutcome::IllegalSequencer { reason } => {
                tracing::warn!(%reason, "Transaction could not be attempted due to sequencer error. If this error persists, check that your sequencer has sufficient funds");
                ProvisionalSequencerOutcome::out_of_funds(
                    // SAFETY: `gas_used` is either Zero or comes from `BasicGasMeter`, which ensures overflow protection.
                    gas_used
                        .checked_value(gas_price)
                        .expect("gas_used value overflowed"),
                    reason,
                )
            }
            AuthAndProcessOutcome::Skipped {
                error,
                tx_hash,
                tx_body,
            } => {
                ProvisionalSequencerOutcome::penalize(
                    // SAFETY: `gas_used`  comes from `BasicGasMeter`, which ensures overflow protection.
                    gas_used
                        .checked_value(gas_price)
                        .expect("gas_used value overflowed"),
                    create_tx_receipt(
                        SkippedTxContents {
                            error,
                            gas_used: gas_used.clone(),
                        },
                        tx_hash,
                        tx_body.data,
                    ),
                )
            }
            AuthAndProcessOutcome::Applied {
                transaction_consumption,
                receipt,
            } => ProvisionalSequencerOutcome::reward(
                transaction_consumption.priority_fee().0,
                receipt,
            ),
        };

        let provisional_reward = provisional_outcome.reward;
        let provisional_penalty = provisional_outcome.penalty;
        let (mut new_checkpoint, outcome) = injected_control_flow.post_tx(
            provisional_outcome,
            dirty_scratchpad,
            slot_gas_meter,
            &gas_used,
        );
        match outcome {
            TxControlFlow::ContinueProcessing(receipt) => {
                new_checkpoint.commit_revertable_storage_cache();
                // SAFETY: It is safe to unwrap here because the total gas used is guaranteed to be less than the slot gas limit.
                slot_gas_meter
                    .charge_gas(&gas_used, sequencer_da_address)
                    .expect("Impossible happened: SlotGasMeter underflows when charging gas.");

                // SAFETY: This won't overflow because rewards/penalties cannot exceed `TOKEN::total_supply` value, which is of type u128.
                accumulated_reward = accumulated_reward
                    .checked_add(provisional_reward)
                    .expect("Total supply of gas token exceeded.");
                accumulated_penalty = accumulated_penalty
                    .checked_add(provisional_penalty)
                    .expect("Total supply of gas token exceeded");
                tx_receipts.push(receipt);
            }
            TxControlFlow::IgnoreTx => {
                if !execution_context.is_sequencer() {
                    new_checkpoint.commit_revertable_storage_cache();
                    // SAFETY: It is safe to unwrap here because the total gas used is guaranteed to be less than the slot gas limit.
                    slot_gas_meter
                        .charge_gas(&gas_used, sequencer_da_address)
                        .expect("Impossible happened: SlotGasMeter underflows when charging gas.");

                    // SAFETY: This won't overflow because rewards and penalties cannot exceed `TOKEN::total_supply`, which is of type `u128`.
                    // This is ensured as it's impossible to accumulate more funds than `TOKEN::total_supply`,
                    // since all rewards and penalties originate from user balances or the sequencer stake.
                    accumulated_reward = accumulated_reward
                        .checked_add(provisional_reward)
                        .expect("Total supply of gas token exceeded.");
                    accumulated_penalty = accumulated_penalty
                        .checked_add(provisional_penalty)
                        .expect("Total supply of gas token exceeded");
                    if is_preferred_sequencer {
                        // SAFETY: We've already charged this gas amount, so it can't overflow at this point.
                        // If we're penalizing the preferred sequencer, we need to account for that in the authorizing the next transaction.
                        sequencer_bond_per_tx = SequencerBondForTx::Preferred(
                            sequencer_bond_per_tx
                                .amount()
                                .saturating_sub(gas_used.checked_value(gas_price).unwrap()),
                        );
                    }

                    // In onchain mode, transactions that make the sequencer run out of gas are treated as "ignored".
                    // While they consume gas, their hashes cannot be computed, so they are not indexed in the database.
                    let ignored = IgnoredTransactionReceipt::<TxReceiptContents<S>> {
                        ignored: IgnoredTxContents {
                            gas_used,
                            index: idx,
                        },
                    };

                    ignored_tx_receipts.push(ignored);
                } else {
                    new_checkpoint.discard_revertable_storage_cache();
                }
                // If we *are* provisionally executing in the sequencer and we run out of funds, the transaction will not be added to the batch.
                // In that case, we need to undo the accounting for penalization of the sequencer.
            }
        }
        clean_scratchpad = new_checkpoint.to_tx_scratchpad();
    }

    let total_gas_used_in_batch = slot_gas_meter
        .total_gas_used()
        .checked_sub(&initial_slot_gas_used)
        // SAFETY: During batch execution, gas is consumed. This means that the total gas used after execution is always greater than before.
        .expect("initial_slot_gas_used can't be bigger than gas used after batch execution");

    let rewards = Rewards {
        accumulated_reward,
        accumulated_penalty,
    };
    // End of the transaction processing phase.
    let batch_receipt = IncrementalBatchReceipt {
        tx_receipts,
        ignored_tx_receipts,
        inner: BatchSequencerReceipt {
            da_address: sequencer_da_address.clone(),
            gas_price: gas_price.clone(),
            gas_used: total_gas_used_in_batch,
            outcome: BatchSequencerOutcome {
                rewards: rewards.clone(),
            },
        },
    };

    checkpoint = clean_scratchpad.commit();
    runtime.gas_enforcer().return_escrowed_funds_to_sequencer(
        sequencer_bond,
        rewards,
        sequencer_da_address,
        &mut checkpoint,
    );
    apply_batch_logs(&batch_receipt, blob_idx);
    span.exit();
    (batch_receipt, checkpoint)
}

enum AuthAndProcessOutcome<S: Spec> {
    /// The sequencer was not allowed to process this transaction
    IllegalSequencer { reason: OutOfFundsReason<S> },
    /// The transaction failed before execution started
    Skipped {
        error: TxProcessingError,
        tx_hash: TxHash,
        tx_body: FullyBakedTx,
    },
    /// The transaction failed
    Applied {
        transaction_consumption: TransactionConsumption<S::Gas>,
        receipt: TransactionReceipt<S>,
    },
}

struct AuthAndProcessOutput<S: Spec, I: StateProvider<S>> {
    /// the *total* gas used in the course of authentication and processing
    gas_used: <S as Spec>::Gas,
    scratchpad: TxScratchpad<S, I>,
    outcome: AuthAndProcessOutcome<S>,
}

fn penalize_sequencer<S: Spec, RT: Runtime<S>, I: StateProvider<S>>(
    runtime: &mut RT,
    auth_cost: Amount,
    sequencer_address: &S::Address,
    operating_mode: OperatingMode,
    tx_scratchpad: &mut TxScratchpad<S, I>,
) {
    runtime
        .gas_enforcer()
        .reward_prover_from_sequencer_balance(
            auth_cost,
            sequencer_address,
            operating_mode,
            tx_scratchpad,
        )
        // We ensure that the sequencer bond is at least `max_tx_check_value` so this should never fail.
        .expect("Sequencer should have enough funds to pay for the pre-execution checks");
}

/// Executes the authentication and processing of a transaction, and rewards/penalizes the sequencer
#[cfg_attr(feature = "bench", sov_modules_api::cycle_tracker)]
#[allow(clippy::too_many_arguments)]
fn auth_and_process_tx_and_incentivize_sequencer<S, RT, I, C>(
    runtime: &mut RT,
    scratchpad: TxScratchpad<S, I>,
    slot_gas: &S::Gas,
    raw_tx: FullyBakedTx,
    sequencer_da_address: &<S::Da as DaSpec>::Address,
    sequencer_rollup_address: S::Address,
    gas_price: &<S::Gas as Gas>::Price,
    execution_context: ExecutionContext,
    sequencer_bond: SequencerBondForTx,
    idx: usize,
    injected_control_flow: &C,
    operating_mode: OperatingMode,
) -> AuthAndProcessOutput<S, I>
where
    S: Spec,
    RT: Runtime<S>,
    I: StateProvider<S>,
    C: InjectedControlFlow<S>,
{
    // CHECKS:
    // 1. `max_tx_check_costs` will not cause an overflow when converted to a token value.
    let max_tx_check_costs = <S as GasSpec>::max_tx_check_costs();
    let max_tx_check_value = match max_tx_check_costs.checked_value(gas_price) {
        Some(v) => v,
        None => {
            return AuthAndProcessOutput {
                outcome: AuthAndProcessOutcome::IllegalSequencer {
                    reason: OutOfFundsReason::TxGasOverflow,
                },
                scratchpad,
                gas_used: <S as Spec>::Gas::zero(),
            }
        }
    };

    if sequencer_bond.amount() < max_tx_check_value {
        return AuthAndProcessOutput {
            outcome: AuthAndProcessOutcome::IllegalSequencer {
                reason: OutOfFundsReason::SequencerBondTooLow {
                    sequencer_bond: sequencer_bond.amount(),
                    max_tx_check_value,
                },
            },
            scratchpad,
            gas_used: <S as Spec>::Gas::zero(),
        };
    }

    // 3. The slot gas is higher than the gas needed to validate the transaction.
    if slot_gas.dim_is_less_or_eq(&max_tx_check_costs) {
        return AuthAndProcessOutput {
            outcome: AuthAndProcessOutcome::IllegalSequencer {
                reason: OutOfFundsReason::SlotGasLimitExhausted {
                    max_tx_check_gas: max_tx_check_costs,
                    remaining_slot_gas: slot_gas.clone(),
                },
            },
            scratchpad,
            gas_used: <S as Spec>::Gas::zero(),
        };
    }

    // In the conditions above, we ensured that both the sequencer bond and the remaining gas in the slot gas meter exceed `max_tx_check_costs`.
    // Initialize `pre_exec_gas_meter` with `max_tx_check_costs` gas.
    let pre_exec_gas_meter = BasicGasMeter::new_with_gas(max_tx_check_costs, gas_price.clone());

    let mut pre_exec_working_set: PreExecWorkingSet<S, _> =
        scratchpad.to_pre_exec_working_set(pre_exec_gas_meter);

    // Charge gas for all the checks in the `process_tx_and_reward_prover`.
    // SAFETY: We can unwrap here because, we asserted that max_tx_check_costs > process_tx_pre_exec_checks_gas.
    pre_exec_working_set
        .charge_gas(&<S as GasSpec>::process_tx_pre_exec_checks_gas())
        .expect("The gas meter should be able to charge the pre-execution checks");

    let authentication_result =
        deserialize_and_authenticate::<S, RT, I>(&raw_tx, &mut pre_exec_working_set);

    let validated_output = match authentication_result {
        Ok(auth_output) => auth_output,
        Err(pre_exec_error) => {
            let (mut scratchpad, pre_exec_gas_meter) =
                pre_exec_working_set.to_scratchpad_and_gas_meter();

            let gas_used_for_authentication = pre_exec_gas_meter.gas_info().gas_used;
            let funds_used_for_authentication = pre_exec_gas_meter.gas_info().gas_value;

            return match pre_exec_error {
                AuthenticationError::FatalError(err, tx_hash) => {
                    penalize_sequencer(
                        runtime,
                        funds_used_for_authentication,
                        &sequencer_rollup_address,
                        operating_mode,
                        &mut scratchpad,
                    );

                    AuthAndProcessOutput {
                        scratchpad,
                        gas_used: gas_used_for_authentication,
                        outcome: AuthAndProcessOutcome::Skipped {
                            error: TxProcessingError::AuthenticationFailed(err.to_string()),
                            tx_hash,
                            tx_body: raw_tx,
                        },
                    }
                }
                AuthenticationError::OutOfGas(e) => {
                    penalize_sequencer(
                        runtime,
                        funds_used_for_authentication,
                        &sequencer_rollup_address,
                        operating_mode,
                        &mut scratchpad,
                    );

                    AuthAndProcessOutput {
                        scratchpad,
                        gas_used: gas_used_for_authentication,
                        outcome: AuthAndProcessOutcome::IllegalSequencer {
                            reason: OutOfFundsReason::SequencerOutOfGasForAuthentication(e),
                        },
                    }
                }
            };
        }
    };

    // Begin the transaction processing phase.
    let raw_tx_hash = validated_output.0.raw_tx_hash;
    let span = tracing::info_span!("transaction", id = %raw_tx_hash, idx = %idx).entered();

    #[cfg(feature = "native")]
    assert_eq!(
        RT::Auth::compute_tx_hash(&raw_tx).ok(),
        Some(raw_tx_hash),
        "Sanity check failed. The transaction hash computed by the authenticator does not match the hash computed by the dedicated tx hash calculation utility method. This is a bug, please report it."
    );

    // Process the transaction and reward the sequencer if everything went well. Responsibility for
    // penalizing the sequencer if the transaction cannot be executed due to sequencer error is with the caller.
    let process_tx_result = process_tx_and_reward_prover(
        runtime,
        pre_exec_working_set,
        slot_gas,
        validated_output,
        raw_tx,
        sequencer_da_address,
        sequencer_rollup_address.clone(),
        execution_context,
        injected_control_flow,
        operating_mode,
    );

    span.exit();

    let (tx_result, mut scratchpad, pre_exec_gas_meter) = process_tx_result;

    match tx_result {
        Err((error, raw_tx)) => {
            penalize_sequencer(
                runtime,
                pre_exec_gas_meter.gas_info().gas_value,
                &sequencer_rollup_address,
                operating_mode,
                &mut scratchpad,
            );

            let gas_used = pre_exec_gas_meter.gas_info().gas_used;
            AuthAndProcessOutput {
                outcome: AuthAndProcessOutcome::Skipped {
                    error,
                    tx_hash: raw_tx_hash,
                    tx_body: raw_tx,
                },
                scratchpad,
                gas_used,
            }
        }
        Ok(ApplyTxResult {
            transaction_consumption,
            receipt,
        }) => {
            // The gas_used in the receipt is the sum of pre_exec_gas_meter.gas_used and the gas consumed during transaction execution.
            let gas_used = get_gas_used(&receipt);

            AuthAndProcessOutput {
                gas_used,
                scratchpad,
                outcome: AuthAndProcessOutcome::Applied {
                    receipt,
                    transaction_consumption,
                },
            }
        }
    }
}
