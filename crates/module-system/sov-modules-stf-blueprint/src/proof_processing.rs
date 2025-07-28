use std::marker::PhantomData;

use sov_modules_api::capabilities::{ChainState, GasEnforcer, ProofProcessor};
use sov_modules_api::proof_metadata::{ProofType, SerializeProofWithDetails};
use sov_modules_api::transaction::AuthenticatedTransactionData;
use sov_modules_api::{
    Amount, BasicGasMeter, DaSpec, Gas, GasArray, GasMeter, GasSpec, InvalidProofError,
    MeteredBorshDeserialize, PreExecWorkingSet, ProofOutcome, ProofReceipt, ProofReceiptContents,
    Rewards, Spec, StateCheckpoint, StateProvider, TxScratchpad, WorkingSet,
};
use sov_state::{Storage, StorageProof};

use crate::Runtime;

const LOG_PREFIX: &str = "Returning early from the proof processing workflow";

// Proof processing workflow:
// 1. Check if the sequencer is bonded.
// 2. Check if the sequencer is registered.
// 3. Verify the proof via the `ProofProcessor` capability.
// 4. Return the proof receipt.
// If any of the steps fail, the proof processing workflow is aborted and returns a `ProofReceipt` with a `ProofOutcome::Invalid`` outcome.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(crate) fn process_proof<S, RT>(
    runtime: &mut RT,
    slot_gas: &S::Gas,
    blob_hash: [u8; 32],
    sequencer_da_address: &<S::Da as DaSpec>::Address,
    sequencer_rollup_address: &S::Address,
    sequencer_bond: Amount,
    gas_price: &<S::Gas as Gas>::Price,
    raw_proof: Vec<u8>,
    state: StateCheckpoint<S>,
) -> (ProcessProofOutput<S>, StateCheckpoint<S>)
where
    S: Spec,
    RT: Runtime<S>,
{
    let mut workflow = ProofProcessingWorkflow::new(runtime, blob_hash, sequencer_da_address);

    // Check if the sequencer is bonded, and create `pre_exec_working_set`.
    let mut pre_exec_working_set = match workflow.authorize_sequencer(
        slot_gas,
        sequencer_bond,
        gas_price,
        state.to_tx_scratchpad(),
    ) {
        WorkflowResult::Proceed(pre_exec_working_set) => pre_exec_working_set,
        WorkflowResult::EarlyReturn(out, scratchpad) => {
            tracing::debug!("{LOG_PREFIX}: unable to create pre execution working set");

            // If sequencer authorization failed we don't charge any gas.
            return (out, scratchpad.commit());
        }
    };

    // The `pre_exec_working_set` is initialized, indicating that the sequencer is bonded and we can begin charging gas.
    match SerializeProofWithDetails::<S>::deserialize(
        &mut raw_proof.as_slice(),
        &mut pre_exec_working_set,
    ) {
        Ok(proof_with_details) => {
            // Reserve gas for the proof verification.
            let mut working_set = match workflow.try_reserve_gas(
                slot_gas,
                sequencer_rollup_address,
                gas_price,
                proof_with_details.details.into(),
                pre_exec_working_set,
            ) {
                WorkflowResult::Proceed(working_set) => working_set,
                WorkflowResult::EarlyReturn(out, scratchpad) => {
                    tracing::debug!(
                        "{LOG_PREFIX}: unable to reserve gas for the proof verification"
                    );

                    tracing::info!(
                        sequencer = %sequencer_da_address,
                        gas_used = %out.gas_used,
                        "The sequencer paid for the transaction.",
                    );

                    let gas_value = out
                        .gas_used
                        .checked_value(gas_price)
                        // SAFETY: Unwrapping is safe here because `gas_used` comes from `BasicGasMeter``, which ensures overflow does not occur.
                        .expect("The gas value can't overflow");

                    let mut checkpoint = scratchpad.commit();
                    workflow.charge_sequencer_and_reward_prover(
                        gas_value,
                        sequencer_bond,
                        &mut checkpoint,
                    );

                    return (out, checkpoint);
                }
            };

            // `workflow.try_reserve_gas` succeeded, meaning that any charge will be deducted from the sequencer's balance in the bank module, rather than from the sequencer's bond.
            let receipt_contents = match proof_with_details.proof {
                ProofType::ZkAggregatedProof(proof) => runtime
                    .proof_processor()
                    .process_aggregated_proof(proof, sequencer_rollup_address, &mut working_set)
                    .map(|(pub_data, proof)| ProofReceiptContents::AggregateProof(pub_data, proof)),

                ProofType::OptimisticProofAttestation(proof) => runtime
                    .proof_processor()
                    .process_attestation(proof, sequencer_rollup_address, &mut working_set)
                    .map(ProofReceiptContents::Attestation),

                ProofType::OptimisticProofChallenge(proof, rollup_height) => runtime
                    .proof_processor()
                    .process_challenge(
                        proof,
                        rollup_height,
                        sequencer_rollup_address,
                        &mut working_set,
                    )
                    .map(ProofReceiptContents::BlockProof),
            };

            let (outcome, mut scratchpad, transaction_consumption) = match receipt_contents {
                Ok(receipt_contents) => {
                    let (scratchpad, transaction_consumption, _) = working_set.finalize();
                    (
                        ProofOutcome::Valid(receipt_contents),
                        scratchpad,
                        transaction_consumption,
                    )
                }
                Err(e) if e.is_not_revertable() => {
                    let (scratchpad, transaction_consumption, _) = working_set.finalize();
                    (
                        ProofOutcome::Invalid(e),
                        scratchpad,
                        transaction_consumption,
                    )
                }
                Err(e) => {
                    let (scratchpad, transaction_consumption) = working_set.revert();
                    (
                        ProofOutcome::Invalid(e),
                        scratchpad,
                        transaction_consumption,
                    )
                }
            };

            runtime.gas_enforcer().refund_remaining_gas(
                sequencer_rollup_address,
                &transaction_consumption.remaining_funds(),
                &mut scratchpad,
            );

            let operating_mode = runtime.chain_state().operating_mode(&mut scratchpad);

            runtime.gas_enforcer().reward_prover(
                &transaction_consumption.base_fee_value(),
                operating_mode,
                &mut scratchpad,
            );

            let sequencer_reward = Rewards {
                accumulated_reward: transaction_consumption.priority_fee().0,
                accumulated_penalty: Amount::ZERO,
            };
            let mut checkpoint = scratchpad.commit();
            runtime.gas_enforcer().return_escrowed_funds_to_sequencer(
                sequencer_bond,
                sequencer_reward,
                sequencer_da_address,
                &mut checkpoint,
            );

            let gas_used = transaction_consumption.base_fee().clone();
            (
                ProcessProofOutput {
                    proof_receipt: ProofReceipt {
                        blob_hash,
                        outcome,
                        gas_used: transaction_consumption.base_fee().as_ref().to_vec(),
                        gas_price: gas_price.as_ref().map(|amount| amount.0).to_vec(),
                    },
                    gas_used,
                },
                checkpoint,
            )
        }
        Err(e) => {
            // We could not deserialize the data from the DA. Penalize the sequencer and return early.
            tracing::debug!("{LOG_PREFIX}: unable to deserialize proof {:?}", e);

            let (state, gas_meter) = pre_exec_working_set.to_scratchpad_and_gas_meter();

            tracing::info!(
                sequencer = %sequencer_da_address,
                "The sequencer paid for the transaction.",
            );

            // SAFETY: We compute this value at the beginning of the function when we create the `pre_exec_working_set`. If that failed, we'll never reach this point.
            let max_tx_check_value = <S as GasSpec>::max_tx_check_costs()
                .checked_value(gas_price)
                .unwrap();

            let mut checkpoint = state.commit();

            workflow.charge_sequencer_and_reward_prover(
                gas_meter.gas_info().gas_value,
                max_tx_check_value,
                &mut checkpoint,
            );

            let gas_used = gas_meter.gas_info().gas_used;
            (
                ProcessProofOutput {
                    proof_receipt: invalid_proof_receipt::<S>(
                        blob_hash,
                        InvalidProofError::PreconditionNotMet(
                            "Sequencer penalized for invalid serialization".to_string(),
                        ),
                    ),
                    gas_used,
                },
                checkpoint,
            )
        }
    }
}

#[allow(clippy::type_complexity)]
pub(crate) struct ProcessProofOutput<S: Spec> {
    pub(crate) proof_receipt: ProofReceipt<
        S::Address,
        <S as Spec>::Da,
        <S::Storage as Storage>::Root,
        StorageProof<<S::Storage as Storage>::Proof>,
    >,

    pub(crate) gas_used: S::Gas,
}

// Decides if the proof processing workflow should continue or return early.
#[allow(clippy::large_enum_variant)]
enum WorkflowResult<Arg, S: Spec, I: StateProvider<S>> {
    // Proceed with the proof processing.
    Proceed(Arg),
    // Early return from the proof processing.
    EarlyReturn(ProcessProofOutput<S>, TxScratchpad<S, I>),
}

struct ProofProcessingWorkflow<'a, S: Spec, RT: Runtime<S>> {
    runtime: &'a mut RT,
    blob_hash: [u8; 32],
    sequencer_da_address: &'a <S::Da as DaSpec>::Address,
    _phantom: PhantomData<S>,
}

impl<'a, S, RT> ProofProcessingWorkflow<'a, S, RT>
where
    S: Spec,
    RT: Runtime<S>,
{
    fn new(
        runtime: &'a mut RT,
        blob_hash: [u8; 32],
        sequencer_da_address: &'a <S::Da as DaSpec>::Address,
    ) -> Self {
        Self {
            runtime,
            blob_hash,
            sequencer_da_address,
            _phantom: PhantomData,
        }
    }

    fn authorize_sequencer<I: StateProvider<S>>(
        &self,
        slot_gas: &S::Gas,
        sequencer_bond: Amount,
        gas_price: &<S::Gas as Gas>::Price,
        tx_scratchpad: TxScratchpad<S, I>,
    ) -> PreExecWorkingSetResult<S, I> {
        // CHECKS:
        // 1. `max_tx_check_costs` will not cause an overflow when converted to a token value.
        let max_tx_check_costs = <S as GasSpec>::max_tx_check_costs();
        let max_tx_check_value = match <S as GasSpec>::max_tx_check_costs().checked_value(gas_price)
        {
            Some(v) => v,
            None => {
                return WorkflowResult::EarlyReturn(
                    ProcessProofOutput {
                        proof_receipt: invalid_proof_receipt::<S>(
                            self.blob_hash,
                            InvalidProofError::PreconditionNotMet(
                                "Overflow: Unable to calculate gas value for max_tx_check_costs"
                                    .to_string(),
                            ),
                        ),
                        gas_used: S::Gas::zero(),
                    },
                    tx_scratchpad,
                );
            }
        };

        // 2. Check that the sequencer has enough bond to cover the max_tx_check_costs
        if max_tx_check_value > sequencer_bond {
            return WorkflowResult::EarlyReturn(
                ProcessProofOutput {
                    proof_receipt: invalid_proof_receipt::<S>(
                        self.blob_hash,
                        InvalidProofError::PreconditionNotMet(
                            "Sequencer bond is too low".to_string(),
                        ),
                    ),
                    gas_used: S::Gas::zero(),
                },
                tx_scratchpad,
            );
        }

        // 3. Check that the slot gas is higher than the gas needed to validate the transaction.
        if slot_gas.dim_is_less_or_eq(&max_tx_check_costs) {
            return WorkflowResult::EarlyReturn(
                ProcessProofOutput {
                    proof_receipt: invalid_proof_receipt::<S>(
                        self.blob_hash,
                        InvalidProofError::PreconditionNotMet("Slot run out of gas".to_string()),
                    ),
                    gas_used: S::Gas::zero(),
                },
                tx_scratchpad,
            );
        }

        let pre_exec_gas_meter =
            BasicGasMeter::<S>::new_with_gas(max_tx_check_costs, gas_price.clone());

        let mut pre_exec_working_set = tx_scratchpad.to_pre_exec_working_set(pre_exec_gas_meter);

        // This represents the cost incurred by the sequencer solely for accepting the proof. It includes the cost of:
        // - refund_remaining_gas
        // - reward_sequencer
        // etc
        pre_exec_working_set
            .charge_gas(&<S as GasSpec>::process_tx_pre_exec_checks_gas())
            // SAFETY: It is ok to expect here because `pre_exec_gas_meter` was initialized with `max_tx_check_costs` which is bigger than process_tx_pre_exec_checks_gas.
            .expect("The gas meter should be able to charge the pre-execution checks");

        WorkflowResult::Proceed(pre_exec_working_set)
    }

    fn try_reserve_gas<I: StateProvider<S>>(
        &mut self,
        slot_gas: &S::Gas,
        sequencer_rollup_address: &S::Address,
        gas_price: &<S::Gas as Gas>::Price,
        auth_tx: AuthenticatedTransactionData<S>,
        mut pre_exec_working_set: PreExecWorkingSet<S, I>,
    ) -> WorkflowResult<WorkingSet<S, I>, S, I> {
        pre_exec_working_set = pre_exec_working_set.commit();
        if let Err(e) = self.runtime.gas_enforcer().try_reserve_gas_for_proof(
            &auth_tx,
            gas_price,
            sequencer_rollup_address,
            &mut pre_exec_working_set,
        ) {
            let (scratchpad, gas_meter) = pre_exec_working_set.revert();
            return Self::make_early_return(
                self.blob_hash,
                scratchpad,
                e.to_string(),
                gas_meter.gas_info().gas_used,
            );
        }

        let (scratchpad, gas_meter) = pre_exec_working_set.to_scratchpad_and_gas_meter();
        let gas_info = gas_meter.gas_info();

        // The transaction will execute until one of the following conditions is met:
        // 1. It consumes more funds than `tx.max_fee`.
        // 2. The `Gas::calculate_min(tx.gas_limit, slot_gas)` is exhausted.
        let working_set_gas_meter = auth_tx.gas_meter(gas_price, slot_gas);

        let mut working_set =
            WorkingSet::create_working_set(scratchpad, &auth_tx, working_set_gas_meter);

        if let Err(err) = working_set.charge_gas(&gas_info.gas_used) {
            let (scratchpad, _transaction_consumption) = working_set.revert();

            return Self::make_early_return(
                self.blob_hash,
                scratchpad,
                err.to_string(),
                gas_info.gas_used,
            );
        }

        WorkflowResult::Proceed(working_set)
    }

    fn charge_sequencer_and_reward_prover(
        &mut self,
        max_auth_cost: Amount,
        reserved_gas_tokens: Amount,
        state: &mut StateCheckpoint<S>,
    ) {
        let sequencer_reward = Rewards {
            accumulated_reward: Amount::ZERO,
            accumulated_penalty: max_auth_cost,
        };
        self.runtime
            .gas_enforcer()
            .return_escrowed_funds_to_sequencer(
                reserved_gas_tokens,
                sequencer_reward,
                self.sequencer_da_address,
                state,
            );
    }

    fn make_early_return<I: StateProvider<S>>(
        blob_hash: [u8; 32],
        tx_scratchpad: TxScratchpad<S, I>,
        reason: String,
        gas_used: S::Gas,
    ) -> WorkflowResult<WorkingSet<S, I>, S, I> {
        WorkflowResult::EarlyReturn(
            ProcessProofOutput {
                proof_receipt: invalid_proof_receipt::<S>(
                    blob_hash,
                    InvalidProofError::PreconditionNotMet(reason),
                ),
                gas_used,
            },
            tx_scratchpad,
        )
    }
}

#[allow(clippy::type_complexity)]
fn invalid_proof_receipt<S: Spec>(
    blob_hash: [u8; 32],
    reason: InvalidProofError,
) -> ProofReceipt<
    S::Address,
    <S as Spec>::Da,
    <S::Storage as Storage>::Root,
    StorageProof<<S::Storage as Storage>::Proof>,
> {
    ProofReceipt {
        blob_hash,
        outcome: ProofOutcome::Invalid(reason),
        gas_used: S::Gas::zero().as_ref().to_vec(),
        gas_price: Vec::new(),
    }
}

type PreExecWorkingSetResult<S, I> = WorkflowResult<PreExecWorkingSet<S, I>, S, I>;
