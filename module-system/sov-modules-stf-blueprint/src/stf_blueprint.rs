use std::marker::PhantomData;

use borsh::BorshDeserialize;
use sov_modules_api::runtime::capabilities::KernelSlotHooks;
use sov_modules_api::{
    BasicAddress, BlobReaderTrait, Context, DaSpec, DispatchCall, GasUnit, StateCheckpoint,
};
use sov_modules_core::WorkingSet;
use sov_rollup_interface::stf::{BatchReceipt, TransactionReceipt};
use tracing::{debug, error};

use crate::tx_verifier::{verify_txs_stateless, TransactionAndRawHash};
use crate::{Batch, Runtime, RuntimeTxHook, SequencerOutcome, SlashingReason, TxEffect};

type ApplyBatchResult<T, A> = Result<T, ApplyBatchError<A>>;

#[allow(type_alias_bounds)]
type ApplyBatch<Da: DaSpec> = ApplyBatchResult<
    BatchReceipt<SequencerOutcome<<Da::BlobTransaction as BlobReaderTrait>::Address>, TxEffect>,
    <Da::BlobTransaction as BlobReaderTrait>::Address,
>;
#[cfg(all(target_os = "zkvm", feature = "bench"))]
use sov_zk_cycle_macros::cycle_tracker;

/// An implementation of the
/// [`StateTransitionFunction`](sov_rollup_interface::stf::StateTransitionFunction)
/// that is specifically designed to work with the module-system.
pub struct StfBlueprint<C: Context, Da: DaSpec, Vm, RT: Runtime<C, Da>, K: KernelSlotHooks<C, Da>> {
    /// State storage used by the rollup.
    /// The runtime includes all the modules that the rollup supports.
    pub(crate) runtime: RT,
    pub(crate) kernel: K,
    phantom_context: PhantomData<C>,
    phantom_vm: PhantomData<Vm>,
    phantom_da: PhantomData<Da>,
}

pub(crate) enum ApplyBatchError<A: BasicAddress> {
    // Contains batch hash
    Ignored([u8; 32]),
    Slashed {
        // Contains batch hash
        hash: [u8; 32],
        reason: SlashingReason,
        sequencer_da_address: A,
    },
}

impl<A: BasicAddress> From<ApplyBatchError<A>> for BatchReceipt<SequencerOutcome<A>, TxEffect> {
    fn from(value: ApplyBatchError<A>) -> Self {
        match value {
            ApplyBatchError::Ignored(hash) => BatchReceipt {
                batch_hash: hash,
                tx_receipts: Vec::new(),
                inner: SequencerOutcome::Ignored,
            },
            ApplyBatchError::Slashed {
                hash,
                reason,
                sequencer_da_address,
            } => BatchReceipt {
                batch_hash: hash,
                tx_receipts: Vec::new(),
                inner: SequencerOutcome::Slashed {
                    reason,
                    sequencer_da_address,
                },
            },
        }
    }
}

impl<C, Vm, Da, RT, K> Default for StfBlueprint<C, Da, Vm, RT, K>
where
    C: Context,
    Da: DaSpec,
    RT: Runtime<C, Da>,
    K: KernelSlotHooks<C, Da>,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<C, Vm, Da, RT, K> StfBlueprint<C, Da, Vm, RT, K>
where
    C: Context,
    Da: DaSpec,
    RT: Runtime<C, Da>,
    K: KernelSlotHooks<C, Da>,
{
    /// [`StfBlueprint`] constructor.
    pub fn new() -> Self {
        Self {
            runtime: RT::default(),
            kernel: K::default(),
            phantom_context: PhantomData,
            phantom_vm: PhantomData,
            phantom_da: PhantomData,
        }
    }

    #[cfg_attr(all(target_os = "zkvm", feature = "bench"), cycle_tracker)]
    pub(crate) fn apply_blob(
        &self,
        checkpoint: StateCheckpoint<C>,
        blob: &mut Da::BlobTransaction,
    ) -> (ApplyBatch<Da>, StateCheckpoint<C>) {
        debug!(
            "Applying batch from sequencer: 0x{}",
            hex::encode(blob.sender())
        );

        let mut batch_workspace = checkpoint.to_revertable();

        // ApplyBlobHook: begin
        if let Err(e) = self.runtime.begin_blob_hook(blob, &mut batch_workspace) {
            error!(
                "Error: The batch was rejected by the 'begin_blob_hook' hook. Skipping batch without slashing the sequencer: {}",
                e
            );

            return (
                Err(ApplyBatchError::Ignored(blob.hash())),
                batch_workspace.revert(),
            );
        }

        // Write changes from begin_blob_hook
        batch_workspace = batch_workspace.checkpoint().to_revertable();

        // TODO: don't ignore these events: https://github.com/Sovereign-Labs/sovereign/issues/350
        let _ = batch_workspace.take_events();

        let (txs, messages) = match self.pre_process_batch(blob) {
            Ok((txs, messages)) => (txs, messages),
            Err(reason) => {
                // Explicitly revert on slashing, even though nothing has changed in pre_process.
                let mut batch_workspace = batch_workspace.checkpoint().to_revertable();
                let sequencer_da_address = blob.sender();
                let sequencer_outcome = SequencerOutcome::Slashed {
                    reason,
                    sequencer_da_address: sequencer_da_address.clone(),
                };
                let checkpoint = match self
                    .runtime
                    .end_blob_hook(sequencer_outcome, &mut batch_workspace)
                {
                    Ok(()) => {
                        // TODO: will be covered in https://github.com/Sovereign-Labs/sovereign-sdk/issues/421
                        batch_workspace.checkpoint()
                    }
                    Err(e) => {
                        error!("End blob hook failed: {}", e);
                        batch_workspace.revert()
                    }
                };

                return (
                    Err(ApplyBatchError::Slashed {
                        hash: blob.hash(),
                        reason,
                        sequencer_da_address,
                    }),
                    checkpoint,
                );
            }
        };

        // Sanity check after pre processing
        assert_eq!(
            txs.len(),
            messages.len(),
            "Error in preprocessing batch, there should be same number of txs and messages"
        );

        // TODO fetch gas price from chain state
        let gas_elastic_price = [0, 0];
        let mut sequencer_reward = 0u64;

        let mut tx_receipts = Vec::with_capacity(txs.len());

        let mut batch_workspace = self.apply_txs(
            txs,
            messages,
            &gas_elastic_price,
            &mut tx_receipts,
            batch_workspace,
            &mut sequencer_reward,
        );

        // TODO: calculate the amount based of gas and fees
        let sequencer_outcome = SequencerOutcome::Rewarded(sequencer_reward);

        if let Err(e) = self
            .runtime
            .end_blob_hook(sequencer_outcome.clone(), &mut batch_workspace)
        {
            // TODO: will be covered in https://github.com/Sovereign-Labs/sovereign-sdk/issues/421
            error!("Failed on `end_blob_hook`: {}", e);
        };

        (
            Ok(BatchReceipt {
                batch_hash: blob.hash(),
                tx_receipts,
                inner: sequencer_outcome,
            }),
            batch_workspace.checkpoint(),
        )
    }

    // Do all stateless checks and data formatting, that can be results in sequencer slashing
    #[cfg_attr(all(target_os = "zkvm", feature = "bench"), cycle_tracker)]
    fn pre_process_batch(
        &self,
        blob_data: &mut impl BlobReaderTrait,
    ) -> Result<
        (
            Vec<TransactionAndRawHash<C>>,
            Vec<<RT as DispatchCall>::Decodable>,
        ),
        SlashingReason,
    > {
        let batch = self.deserialize_batch(blob_data)?;
        debug!("Deserialized batch with {} txs", batch.txs.len());

        // Run the stateless verification, since it is stateless we don't commit.
        let txs = self.verify_txs_stateless(batch)?;

        let messages = self.decode_txs(&txs)?;

        Ok((txs, messages))
    }

    #[cfg_attr(all(target_os = "zkvm", feature = "bench"), cycle_tracker)]
    fn apply_txs(
        &self,
        txs: Vec<TransactionAndRawHash<C>>,
        messages: Vec<<RT as DispatchCall>::Decodable>,
        gas_elastic_price: &[u64],
        tx_receipts: &mut Vec<TransactionReceipt<TxEffect>>,
        mut batch_workspace: WorkingSet<C>,
        sequencer_reward: &mut u64,
    ) -> WorkingSet<C> {
        // Dispatching transactions
        for (TransactionAndRawHash { tx, raw_tx_hash }, msg) in
            txs.into_iter().zip(messages.into_iter())
        {
            // Update the working set gas meter with the available funds
            let gas_price = C::GasUnit::from_arbitrary_dimensions(gas_elastic_price);
            let gas_limit = tx.gas_limit();
            let gas_tip = tx.gas_tip();
            batch_workspace.set_gas(gas_limit, gas_price);

            // Pre dispatch hook
            // TODO set the sequencer pubkey
            let hook = RuntimeTxHook {
                height: 1,
                sequencer: tx.pub_key().clone(),
            };
            let ctx = match self
                .runtime
                .pre_dispatch_tx_hook(&tx, &mut batch_workspace, &hook)
            {
                Ok(verified_tx) => verified_tx,
                Err(e) => {
                    // Don't revert any state changes made by the pre_dispatch_hook even if the Tx is rejected.
                    // For example nonce for the relevant account is incremented.
                    error!("Stateful verification error - the sequencer included an invalid transaction: {}", e);
                    let gas_used = batch_workspace.gas_used().to_dimensions();
                    let receipt = TransactionReceipt {
                        tx_hash: raw_tx_hash,
                        body_to_save: None,
                        events: batch_workspace.take_events(),
                        receipt: TxEffect::Reverted,
                        gas_used,
                    };

                    tx_receipts.push(receipt);
                    continue;
                }
            };

            // Commit changes after pre_dispatch_tx_hook
            batch_workspace = batch_workspace.checkpoint().to_revertable();

            let tx_result = self.runtime.dispatch_call(msg, &mut batch_workspace, &ctx);

            let remaining_gas = batch_workspace.gas_remaining_funds();
            let gas_reward = gas_limit
                .saturating_add(gas_tip)
                .saturating_sub(remaining_gas);

            *sequencer_reward = sequencer_reward.saturating_add(gas_reward);
            debug!(
                "Tx {} sequencer reward: {}",
                hex::encode(raw_tx_hash),
                gas_reward
            );

            let events = batch_workspace.take_events();
            let tx_effect = match tx_result {
                Ok(_) => TxEffect::Successful,
                Err(e) => {
                    error!(
                        "Tx 0x{} was reverted error: {}",
                        hex::encode(raw_tx_hash),
                        e
                    );
                    // The transaction causing invalid state transition is reverted
                    // but we don't slash and we continue processing remaining transactions.
                    batch_workspace = batch_workspace.revert().to_revertable();
                    TxEffect::Reverted
                }
            };
            debug!("Tx {} effect: {:?}", hex::encode(raw_tx_hash), tx_effect);

            let gas_used = batch_workspace.gas_used().to_dimensions();
            let receipt = TransactionReceipt {
                tx_hash: raw_tx_hash,
                body_to_save: None,
                events,
                receipt: tx_effect,
                gas_used,
            };

            tx_receipts.push(receipt);
            // We commit after events have been extracted into receipt.
            batch_workspace = batch_workspace.checkpoint().to_revertable();

            // TODO: `panic` will be covered in https://github.com/Sovereign-Labs/sovereign-sdk/issues/421
            self.runtime
                .post_dispatch_tx_hook(&tx, &ctx, &mut batch_workspace)
                .expect("inconsistent state: error in post_dispatch_tx_hook");
        }

        batch_workspace
    }

    // Attempt to deserialize batch, error results in sequencer slashing.
    #[cfg_attr(all(target_os = "zkvm", feature = "bench"), cycle_tracker)]
    fn deserialize_batch(
        &self,
        blob_data: &mut impl BlobReaderTrait,
    ) -> Result<Batch, SlashingReason> {
        match Batch::try_from_slice(data_for_deserialization(blob_data)) {
            Ok(batch) => Ok(batch),
            Err(e) => {
                assert_eq!(blob_data.verified_data().len(), blob_data.total_len(), "Batch deserialization failed and some data was not provided. The prover might be malicious");
                // If the deserialization fails, we need to make sure it's not because the prover was malicious and left
                // out some relevant data! Make that check here. If the data is missing, panic.
                error!(
                    "Unable to deserialize batch provided by the sequencer {}",
                    e
                );
                Err(SlashingReason::InvalidBatchEncoding)
            }
        }
    }

    // Stateless verification of transaction, such as signature check
    // Single malformed transaction results in sequencer slashing.
    #[cfg_attr(all(target_os = "zkvm", feature = "bench"), cycle_tracker)]
    fn verify_txs_stateless(
        &self,
        batch: Batch,
    ) -> Result<Vec<TransactionAndRawHash<C>>, SlashingReason> {
        match verify_txs_stateless(batch.txs) {
            Ok(txs) => Ok(txs),
            Err(e) => {
                error!("Stateless verification error - the sequencer included a transaction which was known to be invalid. {}\n", e);
                Err(SlashingReason::StatelessVerificationFailed)
            }
        }
    }

    // Checks that runtime message can be decoded from transaction.
    // If a single message cannot be decoded, sequencer is slashed
    #[cfg_attr(all(target_os = "zkvm", feature = "bench"), cycle_tracker)]
    fn decode_txs(
        &self,
        txs: &[TransactionAndRawHash<C>],
    ) -> Result<Vec<<RT as DispatchCall>::Decodable>, SlashingReason> {
        let mut decoded_messages = Vec::with_capacity(txs.len());
        for TransactionAndRawHash { tx, raw_tx_hash } in txs {
            match RT::decode_call(tx.runtime_msg()) {
                Ok(msg) => decoded_messages.push(msg),
                Err(e) => {
                    error!("Tx 0x{} decoding error: {}", hex::encode(raw_tx_hash), e);
                    return Err(SlashingReason::InvalidTransactionEncoding);
                }
            }
        }
        Ok(decoded_messages)
    }
}

#[cfg(feature = "native")]
fn data_for_deserialization(blob: &mut impl BlobReaderTrait) -> &[u8] {
    blob.full_data()
}

#[cfg(not(feature = "native"))]
fn data_for_deserialization(blob: &mut impl BlobReaderTrait) -> &[u8] {
    blob.verified_data()
}
