use sov_modules_api::capabilities::{
    BatchFromUnregisteredSequencer, ChainState, GasEnforcer, SequencerRemuneration,
    TransactionAuthenticator, TransactionAuthorizer, UnregisteredAuthenticationError,
};
use sov_modules_api::*;
use tracing::{debug, warn};

use crate::sequencer_mode::common::{
    apply_batch_logs, apply_tx, create_tx_receipt, get_gas_used, BatchReceipt,
};
use crate::{ApplyTxResult, AuthTxOutput, Runtime, StateCheckpoint, TxReceiptContents};

#[allow(clippy::result_large_err)]
#[allow(clippy::too_many_arguments)]
pub fn process_unauthorized_tx<S: Spec, R: Runtime<S>>(
    runtime: &mut R,
    mut pre_exec_working_set: PreExecWorkingSet<S, StateCheckpoint<S>>,
    slot_gas: &S::Gas,
    validated_output: AuthTxOutput<S, R>,
    raw_tx: FullyBakedTx,
    sequencer_da_address: &<S::Da as DaSpec>::Address,
) -> (
    Result<ApplyTxResult<S>, TxProcessingError>,
    StateCheckpoint<S>,
    BasicGasMeter<S>,
) {
    pre_exec_working_set = pre_exec_working_set.commit();
    let (auth_tx, auth_data, message) = validated_output;

    let raw_tx_hash = auth_tx.raw_tx_hash;
    let tx = &auth_tx.authenticated_tx;

    let mut ctx = match runtime
        .transaction_authorizer()
        .resolve_unregistered_context(&auth_data, sequencer_da_address, &mut pre_exec_working_set)
    {
        Ok(ctx) => ctx,
        Err(e) => {
            let (scratchpad, pre_exec_gas_meter) = pre_exec_working_set.revert();
            return (
                Err(TxProcessingError::CannotResolveContext(e.to_string())),
                scratchpad.commit(),
                pre_exec_gas_meter,
            );
        }
    };

    // Check that the transaction isn't a duplicate
    if let Err(e) = runtime.transaction_authorizer().check_uniqueness(
        &auth_data,
        &ctx,
        &mut pre_exec_working_set,
    ) {
        let (scratchpad, pre_exec_gas_meter) = pre_exec_working_set.revert();
        return (
            Err(TxProcessingError::CheckUniquenessFailed(e.to_string())),
            scratchpad.commit(),
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
            Err(TxProcessingError::MarkTxAttemptedFailed(err.to_string())),
            scratchpad.commit(),
            pre_exec_gas_meter,
        );
    }

    let gas_price = pre_exec_working_set.gas_price().clone();
    // After this check, we are confident that the transaction sender can cover the costs of transaction processing.
    if let Err(err) =
        runtime
            .gas_enforcer()
            .try_reserve_gas(tx, &gas_price, &mut ctx, &mut pre_exec_working_set)
    {
        let (scratchpad, pre_exec_gas_meter) = pre_exec_working_set.revert();
        return (
            Err(TxProcessingError::CannotReserveGas(err.to_string())),
            scratchpad.commit(),
            pre_exec_gas_meter,
        );
    }

    let (scratchpad, pre_exec_gas_meter) = pre_exec_working_set.to_scratchpad_and_gas_meter();
    let gas_info = pre_exec_gas_meter.gas_info();

    // The transaction will execute until one of the following conditions is met:
    // 1. It consumes more funds than `tx.max_fee`.
    // 2. The `Gas::calculate_min(tx.gas_limit, slot_gas)` is exhausted.
    let working_set_gas_meter = tx.gas_meter(&gas_info.gas_price, slot_gas);

    let mut working_set = WorkingSet::create_working_set(scratchpad, tx, working_set_gas_meter);

    // Here we charge the gas for the transaction sig & pre-execution checks.
    if let Err(err) = working_set.charge_gas(&gas_info.gas_used) {
        let (mut scratchpad, transaction_consumption) = working_set.revert();

        // Refund the remaining gas to the sender.
        runtime.gas_enforcer().refund_remaining_gas(
            ctx.gas_refund_recipient(),
            &transaction_consumption.remaining_funds(),
            &mut scratchpad,
        );
        return (
            Err(TxProcessingError::OutOfGas(err.to_string())),
            scratchpad.commit(),
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

    let operating_mode = runtime.chain_state().operating_mode(&mut scratchpad);
    runtime.gas_enforcer().reward_prover(
        &transaction_consumption.base_fee_value(),
        operating_mode,
        &mut scratchpad,
    );

    let mut checkpoint = scratchpad.commit();
    let sequencer_reward = transaction_consumption.priority_fee();
    runtime.sequencer_remuneration().reward_sequencer_or_refund(
        sequencer_da_address,
        ctx.gas_refund_recipient(),
        sequencer_reward,
        &mut checkpoint,
    );

    (Ok(apply_tx), checkpoint, pre_exec_gas_meter)
}

#[allow(clippy::type_complexity)]
#[cfg_attr(feature = "bench", sov_modules_api::cycle_tracker)]
pub(crate) fn authenticate_unregistered_tx<S: Spec, R: Runtime<S>, I: StateProvider<S>>(
    batch: &BatchFromUnregisteredSequencer,
    pre_exec_working_set: &mut PreExecWorkingSet<S, I>,
) -> Result<AuthTxOutput<S, R>, UnregisteredAuthenticationError> {
    let (tx, auth_data, call) = R::Auth::authenticate_unregistered(batch, pre_exec_working_set)?;
    Ok((tx, auth_data, R::wrap_call(call)))
}

#[tracing::instrument(skip_all, name = "StfBlueprint::apply_batch")]
#[allow(clippy::too_many_arguments)]
#[cfg_attr(feature = "bench", sov_modules_api::cycle_tracker)]
pub(crate) fn apply_batch<S, RT>(
    runtime: &mut RT,
    checkpoint: StateCheckpoint<S>,
    slot_gas: &S::Gas,
    batch: BatchFromUnregisteredSequencer,
    blob_idx: usize,
    sequencer_da_address: &<S::Da as DaSpec>::Address,
    gas_price: &<S::Gas as Gas>::Price,
) -> (BatchReceipt<S>, StateCheckpoint<S>)
where
    S: Spec,
    RT: Runtime<S>,
{
    debug!(
        batch_id = hex::encode(batch.id),
        sequencer_da_address = %sequencer_da_address,
        ?gas_price,
        "Applying a batch"
    );

    let scratchpad = checkpoint.to_tx_scratchpad();

    debug!(
        batch_id = hex::encode(batch.id),
        "Verifying & executing transactions"
    );

    // A helper function to create a batch receipt.
    let early_return_batch_receipt =
        |tx_receipts: Vec<TransactionReceipt<S>>,
         ignored_tx_receipts: Vec<IgnoredTransactionReceipt<TxReceiptContents<S>>>,
         gas_used: <S as Spec>::Gas|
         -> BatchReceipt<S> {
            BatchReceipt {
                batch_hash: batch.id,
                tx_receipts,
                ignored_tx_receipts,
                inner: BatchSequencerReceipt {
                    da_address: sequencer_da_address.clone(),
                    gas_price: gas_price.clone(),
                    gas_used,
                    outcome: BatchSequencerOutcome {
                        rewards: Rewards {
                            accumulated_reward: Amount::ZERO,
                            accumulated_penalty: Amount::ZERO,
                        },
                    },
                },
            }
        };

    // Check: The slot gas is higher than the gas needed to validate the transaction.
    let max_unregistered_tx_check_costs = <S as GasSpec>::max_unregistered_tx_check_costs();
    if slot_gas.dim_is_less_or_eq(&max_unregistered_tx_check_costs) {
        // We don't consume gas for failed authentication of unregistered sequencer.
        let gas_used = S::Gas::zero();

        let ignored = IgnoredTransactionReceipt::<TxReceiptContents<S>> {
            ignored: IgnoredTxContents {
                gas_used: gas_used.clone(),
                index: 0,
            },
        };

        return (
            early_return_batch_receipt(Vec::new(), vec![ignored], gas_used),
            scratchpad.commit(),
        );
    }

    // We need to be cautious about potential DoS attacks. When receiving an emergency registration, we cannot immediately determine if the sequencer registration will succeed.
    // A malicious actor could exploit this mechanism to attack the rollup, for instance, by sending large transactions that are costly to deserialize from an address with no funds on the rollup.
    // To mitigate this, we initialize the gas meter with just enough gas to process a valid transaction. If the transaction is too big, we quickly run out of gas.
    // Additionally, we rate-limit (during blob selection) the number of forced registrations to further reduce the effectiveness of such attacks.
    let meter = BasicGasMeter::new_with_gas(max_unregistered_tx_check_costs, gas_price.clone());

    let mut pre_exec_working_set = scratchpad.to_pre_exec_working_set(meter);
    let authentication_result =
        authenticate_unregistered_tx::<S, RT, _>(&batch, &mut pre_exec_working_set);

    let validated_output = match authentication_result {
        Ok(auth_output) => auth_output,
        // Note that on failure no one pays for the gas used.
        // However, we still meter it so that it counts against the slot gas limit.
        Err(e) => {
            let (scratchpad, gas_meter) = pre_exec_working_set.to_scratchpad_and_gas_meter();
            let gas_info = gas_meter.gas_info();
            let gas_used = gas_info.gas_used;
            match e {
                UnregisteredAuthenticationError::FatalError(err, tx_hash) => {
                    let err_str = format!("Unregistered sequencer authentication failed: {err}");
                    warn!(error = ?err_str);
                    let skipped = SkippedTxContents {
                        error: TxProcessingError::AuthenticationFailed(err_str),
                        gas_used: gas_used.clone(),
                    };

                    return (
                        early_return_batch_receipt(
                            vec![create_tx_receipt(skipped, tx_hash, batch.tx.data.clone())],
                            Vec::new(),
                            gas_used,
                        ),
                        scratchpad.commit(),
                    );
                }
                UnregisteredAuthenticationError::OutOfGas(reason) => {
                    warn!(error = %reason, "Not enough gas to authenticate the batch");
                }
                UnregisteredAuthenticationError::InvalidAuthenticationDiscriminant => {
                    warn!("Invalid authentication discriminant");
                }
            }
            let ignored = IgnoredTransactionReceipt::<TxReceiptContents<S>> {
                ignored: IgnoredTxContents {
                    gas_used: gas_used.clone(),
                    index: 0,
                },
            };

            return (
                early_return_batch_receipt(Vec::new(), vec![ignored], gas_used),
                scratchpad.commit(),
            );
        }
    };

    let raw_tx_hash = validated_output.0.raw_tx_hash;

    let process_tx_result = process_unauthorized_tx(
        runtime,
        pre_exec_working_set,
        slot_gas,
        validated_output,
        batch.tx.clone(),
        sequencer_da_address,
    );

    let mut tx_receipts = Vec::new();
    let gas_used;
    let mut accumulated_reward = Amount::ZERO;

    let (tx_result, checkpoint, pre_exec_gas_meter) = process_tx_result;

    match tx_result {
        Err(error) => {
            // There is no one to charge for the pre-execution gas because the sequencer was not registered at the time of the error.
            // However, we deduct the gas from the slot gas meter.
            gas_used = pre_exec_gas_meter.gas_info().gas_used;
            let skipped = SkippedTxContents {
                error,
                gas_used: gas_used.clone(),
            };

            let tx_receipt = create_tx_receipt(skipped, raw_tx_hash, batch.tx.data.clone());
            tx_receipts.push(tx_receipt);
        }
        Ok(ApplyTxResult {
            transaction_consumption,
            receipt,
        }) => {
            // We reward sequencer only if the registration transaction is successful.
            if receipt.receipt.is_successful() {
                let sequencer_reward = transaction_consumption.priority_fee();
                accumulated_reward = accumulated_reward
                    .checked_add(sequencer_reward.0)
                    // SAFETY: The sequencer reward cannot exceed the total token supply, so this cannot overflow
                    .expect("AccumulatedReward overflow");
            }
            gas_used = get_gas_used(&receipt);
            tx_receipts.push(receipt);
        }
    }

    let batch_receipt = BatchReceipt {
        batch_hash: batch.id,
        tx_receipts,
        ignored_tx_receipts: vec![],
        inner: BatchSequencerReceipt {
            da_address: sequencer_da_address.clone(),
            gas_price: gas_price.clone(),
            gas_used: gas_used.clone(),

            outcome: BatchSequencerOutcome {
                rewards: Rewards {
                    accumulated_reward,
                    accumulated_penalty: Amount::ZERO,
                },
            },
        },
    };

    apply_batch_logs(&batch_receipt, blob_idx);

    (batch_receipt, checkpoint)
}
