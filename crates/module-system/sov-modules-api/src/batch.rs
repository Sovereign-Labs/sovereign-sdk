use std::sync::Arc;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_rollup_interface::da::DaSpec;

use crate::{
    Amount, Context, DispatchCall, Gas, Runtime, SlotGasMeter, Spec, StateCheckpoint,
    TransactionReceipt, TxScratchpad,
};

/// `FullyBakedTx` represents a serialized signed rollup transaction that has been encoded with
/// authentication information and is ready to be placed on the DA layer.
#[derive(
    PartialEq,
    Eq,
    Clone,
    BorshDeserialize,
    BorshSerialize,
    Serialize,
    Deserialize,
    derive_more::AsRef,
)]
pub struct FullyBakedTx {
    /// Serialized transaction.
    #[as_ref(forward)]
    pub data: Vec<u8>,
}

impl std::fmt::Debug for FullyBakedTx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FullyBakedTx")
            .field("data", &hex::encode(&self.data))
            .finish()
    }
}

impl FullyBakedTx {
    /// Construct a `FullyBakedTx` containing the given data
    #[must_use]
    pub fn new(data: Vec<u8>) -> Self {
        Self { data }
    }
}

/// `RawTx` represents a serialized signed rollup transaction. A `RawTx` needs to be encoded
/// with authentication information before being placed on the DA layer.
#[derive(
    PartialEq,
    Eq,
    Clone,
    BorshDeserialize,
    BorshSerialize,
    Serialize,
    Deserialize,
    derive_more::AsRef,
)]
pub struct RawTx {
    /// Serialized transaction.
    #[as_ref(forward)]
    pub data: Vec<u8>,
}

impl std::fmt::Debug for RawTx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RawTx")
            .field("data", &hex::encode(&self.data))
            .finish()
    }
}

impl RawTx {
    /// Construct a `RawTx` containing the given data
    #[must_use]
    pub fn new(data: Vec<u8>) -> Self {
        Self { data }
    }
}

/// A blob that has been selected for execution
pub struct SelectedBlob<S: Spec, B> {
    /// The blob data
    pub blob_data: BlobDataWithId<S, B>,
    /// The da address of the blob's sender
    pub sender: <<S as Spec>::Da as DaSpec>::Address,
    /// The tokens reserved for pre-execution checks from the sender's account.
    /// In principle, the location where these tokens are reserved is up to the implementation of the blob selector -
    /// in practice, this is always in the bank's balance at `self.bank.id()`
    pub reserved_gas_tokens: Option<Amount>,
}

impl<S: Spec, B> SelectedBlob<S, B> {
    /// See [`BlobDataWithId::map_batch`].
    pub fn map_batch<B1>(self, f: impl FnOnce(B) -> B1) -> SelectedBlob<S, B1> {
        SelectedBlob {
            blob_data: self.blob_data.map_batch(f),
            sender: self.sender,
            reserved_gas_tokens: self.reserved_gas_tokens,
        }
    }
}

/// The amount of tokens reserved for pre-execution checks *for a particular transaction* from the sender's account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequencerBondForTx {
    /// When the balance comes from the preferred sequencer, this amount is shared across all transactions in the batch.
    /// If one transaction fails, the reserved amount will decrease causing cascading failures.
    Preferred(Amount),
    /// When the balance comes from the standard sequencer, each transaction has its own separate reserved pool,
    /// so the failure of one transaction does not affect the reserved tokens for other transactions.
    Standard(Amount),
}

impl SequencerBondForTx {
    /// The amount of tokens available for pre-execution checks.
    pub fn amount(&self) -> Amount {
        match self {
            SequencerBondForTx::Preferred(amount) | SequencerBondForTx::Standard(amount) => *amount,
        }
    }
}

const ID_SIZE: usize = 32;

/// Contains raw transactions obtained from the DA blob.
/// A Batch with its ID.
#[derive(Clone, Debug, PartialEq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct BatchWithId<S: Spec> {
    /// Batch of transactions.
    pub batch: Arc<Vec<FullyBakedTx>>,
    /// The ID of the batch, carried over from the DA layer. This is the hash of the blob which contained the batch.
    pub id: [u8; ID_SIZE],
    /// The address of the sequencer that submitted the batch.
    pub sequencer_address: S::Address,
}

impl<S: Spec> BatchWithId<S> {
    /// Construct a new batch with the given ID.
    pub fn new(batch: Arc<Vec<FullyBakedTx>>, id: [u8; 32], sequencer_address: S::Address) -> Self {
        Self {
            batch,
            id,
            sequencer_address,
        }
    }

    /// The total size, in bytes, of all transactions and associated batch metadata.
    pub fn batch_with_id_size(&self) -> usize {
        let batch_size: usize = self.batch.iter().map(|tx| tx.data.len()).sum();
        batch_size + ID_SIZE + self.sequencer_address.as_ref().len()
    }
}

/// Contains blob data obtained from the DA.
#[derive(Debug, PartialEq, Clone, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlobData<S: Spec> {
    /// Batch of transactions.
    Batch((Arc<Vec<FullyBakedTx>>, S::Address)),
    /// Emergency Registration
    EmergencyRegistration(FullyBakedTx),
    /// Aggregated proof posted on the DA.
    Proof((Vec<u8>, S::Address)),
}

impl<S: Spec> BlobData<S> {
    /// Tag the blob with the given ID.
    pub fn with_id(self, id: [u8; 32]) -> BlobDataWithId<S, BatchWithId<S>> {
        match self {
            Self::Batch((batch, seq_addr)) => BlobDataWithId::Batch(BatchWithId {
                batch,
                id,
                sequencer_address: seq_addr,
            }),
            Self::Proof((proof, seq_addr)) => BlobDataWithId::Proof {
                proof,
                id,
                sequencer_address: seq_addr,
            },
            Self::EmergencyRegistration(tx) => BlobDataWithId::EmergencyRegistration { tx, id },
        }
    }
}

/// Contains blob data obtained from the DA.
/// N.B. The `B` type must store its own ID (e.g. `BlobWithId`) - `BlobDataWithId::Batch`
/// does not annotate the `B` with the ID again.
#[derive(Debug, PartialEq, Clone, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlobDataWithId<S: Spec, B> {
    /// Batch of transactions.
    Batch(B),
    /// Emergency Registration
    EmergencyRegistration {
        /// The registration transaction
        tx: FullyBakedTx,
        /// The id of the blob on the DA layer
        id: [u8; 32],
    },
    /// Aggregated proof posted on the DA.
    Proof {
        /// The proof
        proof: Vec<u8>,
        /// The id of the blob on the DA layer
        id: [u8; 32],
        /// The address of the sequencer that submitted the proof
        sequencer_address: S::Address,
    },
}

impl<S: Spec> BlobDataWithId<S, BatchWithId<S>> {
    /// The size of the blob in bytes.
    pub fn blob_size(&self) -> usize {
        match self {
            BlobDataWithId::Batch(b) => b.batch_with_id_size(),
            BlobDataWithId::Proof {
                proof,
                sequencer_address,
                ..
            } => proof.len() + 32 + sequencer_address.as_ref().len(),
            BlobDataWithId::EmergencyRegistration { tx, .. } => tx.data.len() + 32,
        }
    }

    /// Returns the ID of the blob.
    pub fn id(&self) -> [u8; 32] {
        match self {
            BlobDataWithId::Batch(b) => b.id,
            BlobDataWithId::Proof { id, .. } | BlobDataWithId::EmergencyRegistration { id, .. } => {
                *id
            }
        }
    }

    /// Returns true if the blob is an emergency registration.
    pub fn is_emergency_registration(&self) -> bool {
        matches!(self, BlobDataWithId::EmergencyRegistration { .. })
    }
}

/// The control flow to take after beginning to execute a tx.
pub enum TxControlFlow<R> {
    /// Continue processing the transaction using the provided output.
    ContinueProcessing(R),
    /// Ignore the tx, reverting any effects of its processing so far.
    IgnoreTx,
}

/// The provisional outcome for a sequencer after applying a single transaction
pub struct ProvisionalSequencerOutcome<S: Spec> {
    /// The sequencer's reward, in rollup tokens
    pub reward: Amount,
    /// The sequencer's penalty, in rollup tokens
    pub penalty: Amount,
    /// Whether the sequencer has run out of funds
    pub execution_status: MaybeExecuted<S>,
}

/// The reason a transaction was rejected by the sequencer due to insufficient funds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum OutOfFundsReason<S: Spec> {
    /// The gas calculation overflowed.
    #[error("Overflow: Unable to calculate gas value for max_tx_check_costs")]
    TxGasOverflow,
    /// The sequencer doesn't have sufficient bond to cover the cost of checking the transaction - either because the gas price is very high
    /// of the sequencer bond is too low.
    #[error("The sequencer did not have sufficient funds to cover tx authentication checks, sequencer bond is {sequencer_bond}, but the cost of checking the transaction is {max_tx_check_value}")]
    SequencerBondTooLow {
        #[allow(missing_docs)]
        sequencer_bond: Amount,
        #[allow(missing_docs)]
        max_tx_check_value: Amount,
    },
    /// The slot gas limit has been exhausted.
    #[error("The slot gas limit has been exhausted, max_tx_check_gas is {max_tx_check_gas}, remaining_slot_gas is {remaining_slot_gas}")]
    SlotGasLimitExhausted {
        #[allow(missing_docs)]
        max_tx_check_gas: <S as Spec>::Gas,
        #[allow(missing_docs)]
        remaining_slot_gas: <S as Spec>::Gas,
    },
    /// The sequencer ran out of gas.
    #[error("The sequencer did not have sufficient funds to cover tx authentication: {0}")]
    SequencerOutOfGasForAuthentication(String),
}

/// A transaction that may or may not have been executed
pub enum MaybeExecuted<S: Spec> {
    /// The execution result from the tx
    Executed(TransactionReceipt<S>),
    /// The transactions wasn't executed because the sequencer ran out of funds
    SequencerOutOfFunds(OutOfFundsReason<S>),
}

impl<S: Spec> ProvisionalSequencerOutcome<S> {
    /// A convenient constructor for provisionally penalizing the sequencer and indicating
    /// that the sequencer has run out of funds.
    #[must_use]
    pub fn out_of_funds(penalty: Amount, reason: OutOfFundsReason<S>) -> Self {
        Self {
            reward: Amount::ZERO,
            penalty,
            execution_status: MaybeExecuted::SequencerOutOfFunds(reason),
        }
    }

    /// A convenient constructor for provisionally penalizing the sequencer
    pub fn penalize(penalty: Amount, receipt: TransactionReceipt<S>) -> Self {
        Self {
            reward: Amount::ZERO,
            penalty,
            execution_status: MaybeExecuted::Executed(receipt),
        }
    }
    /// A convenient constructor for provisionally rewarding the sequencer
    pub fn reward(amount: Amount, receipt: TransactionReceipt<S>) -> Self {
        Self {
            reward: amount,
            penalty: Amount::ZERO,
            execution_status: MaybeExecuted::Executed(receipt),
        }
    }
}

/// Allows the node component which produces a batch to inject logic at two points in transaction
/// lifecycle. This is used by the sequencer to unwind failing transactions and to inspect
/// the set of state changes made by a transaction before committing.
pub trait InjectedControlFlow<S: Spec> {
    /// Runs after authentication but before the transaction executes
    fn pre_flight<RT: Runtime<S>>(
        &self,
        runtime: &RT,
        context: &Context<S>,
        call: &<RT as DispatchCall>::Decodable,
    ) -> TxControlFlow<()>;

    /// Runs after the transaction has executed.
    fn post_tx(
        &self,
        provisional_outcome: ProvisionalSequencerOutcome<S>,
        dirty_scratchpad: TxScratchpad<S, StateCheckpoint<S>>,
        slot_gas_meter_before_tx: &SlotGasMeter<S>,
        gas_used: &<S as Spec>::Gas,
    ) -> (StateCheckpoint<S>, TxControlFlow<TransactionReceipt<S>>);
}

/// A batch that can be processed incrementally
pub trait IncrementalBatch<S: Spec>: Iterator<Item = (FullyBakedTx, Self::ControlFlow)> {
    /// The post tx hook type used by this funciton
    type ControlFlow: InjectedControlFlow<S>;
    /// Returns an accurate lower bound on the remaining elements, if known.
    fn known_remaining_txs(&self) -> Option<usize>;

    /// The id of this blob on the DA layer, if known.
    fn id(&self) -> Option<[u8; 32]>;

    /// Runs just before the batch is applied.
    fn pre_flight(&mut self, state_checkpoint: &mut StateCheckpoint<S>);

    /// The address of the sequencer that submitted the batch.
    fn sequencer_address(&self) -> S::Address;
}

impl<S: Spec> InjectedControlFlow<S> for NoOpControlFlow {
    fn pre_flight<RT: Runtime<S>>(
        &self,
        _runtime: &RT,
        _context: &Context<S>,
        _call: &<RT as DispatchCall>::Decodable,
    ) -> TxControlFlow<()> {
        TxControlFlow::ContinueProcessing(())
    }

    fn post_tx(
        &self,
        provisional_outcome: ProvisionalSequencerOutcome<S>,
        dirty_scratchpad: TxScratchpad<S, StateCheckpoint<S>>,
        _slot_gas_meter_before_tx: &SlotGasMeter<S>,
        _gas_used: &<S as Spec>::Gas,
    ) -> (StateCheckpoint<S>, TxControlFlow<TransactionReceipt<S>>) {
        match provisional_outcome.execution_status {
            MaybeExecuted::Executed(receipt) => (
                dirty_scratchpad.commit(),
                TxControlFlow::ContinueProcessing(receipt),
            ),
            MaybeExecuted::SequencerOutOfFunds(_) => {
                (dirty_scratchpad.commit(), TxControlFlow::IgnoreTx)
            }
        }
    }
}

/// Control flow which does not alter a transaction's execution path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoOpControlFlow;

/// A Batch with its ID.
#[derive(Debug)]

pub struct IterableBatchWithId<S: Spec, CF: InjectedControlFlow<S>> {
    /// Batch of transactions.
    batch: Arc<Vec<FullyBakedTx>>,
    offset: usize,
    /// The ID of the batch, carried over from the DA layer. This is the hash of the blob which contained the batch.
    pub id: [u8; 32],
    /// The address of the sequencer that submitted the batch.
    pub sequencer_address: S::Address,
    /// Control flow.
    pub cf: CF,
}

impl<S: Spec, CF: InjectedControlFlow<S>> IterableBatchWithId<S, CF> {
    /// Remaining transactions in the batch.
    pub fn remaining(&self) -> usize {
        self.batch.len().saturating_sub(self.offset)
    }
}

impl<S: Spec, CF: InjectedControlFlow<S>> IterableBatchWithId<S, CF> {
    /// Create a new `IterableBatchWithId` from a `BatchWithId`.
    pub fn new(batch_with_id: BatchWithId<S>, cf: CF) -> Self {
        Self {
            batch: batch_with_id.batch,
            offset: 0,
            id: batch_with_id.id,
            sequencer_address: batch_with_id.sequencer_address,
            cf,
        }
    }
}

impl<S: Spec, CF: InjectedControlFlow<S> + Clone> Iterator for IterableBatchWithId<S, CF> {
    // TODO: Consider skipping this clone by adding a lifetime parameter
    type Item = (FullyBakedTx, CF);

    fn next(&mut self) -> Option<Self::Item> {
        let tx = self.batch.get(self.offset).cloned();
        if let Some(tx) = tx {
            self.offset += 1;
            Some((tx, self.cf.clone()))
        } else {
            None
        }
    }
}

impl<S: Spec, CF: InjectedControlFlow<S> + Clone> IncrementalBatch<S>
    for IterableBatchWithId<S, CF>
{
    type ControlFlow = CF;

    fn known_remaining_txs(&self) -> Option<usize> {
        Some(self.batch.len())
    }

    fn id(&self) -> Option<[u8; 32]> {
        Some(self.id)
    }

    fn pre_flight(&mut self, _state_checkpoint: &mut StateCheckpoint<S>) {
        // Do nothing
    }

    fn sequencer_address(&self) -> S::Address {
        self.sequencer_address.clone()
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
/// The reason a transaction was rejected by the sequencer
pub enum RejectReason {
    /// The sender of the tx must be a sequencer admin
    SenderMustBeAdmin,
    /// The sequencer ran out of funds to pay for gas.
    SequencerOutOfGas,
    /// The transaction did not result in a sufficient reward.
    InsufficientReward {
        /// The minimum reward, in rollup tokens
        expected: u128,
        /// The reward received, in rollup tokens
        found: u128,
    },
}

impl<S: Spec> BlobData<S> {
    /// Batch variant constructor.
    pub fn new_batch(txs: Arc<Vec<FullyBakedTx>>, sequencer_address: S::Address) -> Self {
        BlobData::Batch((txs, sequencer_address))
    }

    /// Emergency Registration variant constructor.
    pub fn new_emergency_registration(tx: FullyBakedTx) -> Self {
        BlobData::EmergencyRegistration(tx)
    }

    /// Proof variant constructor.
    pub fn new_proof(proof: Vec<u8>, sequencer_address: S::Address) -> Self {
        BlobData::Proof((proof, sequencer_address))
    }
}

impl<S: Spec, B> BlobDataWithId<S, B> {
    /// Convert the inner `Batch` type to another type, if applicable
    pub fn map_batch<Target>(self, f: impl FnOnce(B) -> Target) -> BlobDataWithId<S, Target> {
        match self {
            Self::Batch(b) => BlobDataWithId::Batch(f(b)),
            Self::Proof {
                proof,
                id,
                sequencer_address,
            } => BlobDataWithId::Proof {
                proof,
                id,
                sequencer_address,
            },
            Self::EmergencyRegistration { tx, id } => {
                BlobDataWithId::EmergencyRegistration { tx, id }
            }
        }
    }
}

/// The sequencer rewards.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Rewards {
    /// Rewards accumulated by the sequencer during the batch processing
    pub accumulated_reward: Amount,
    /// Penalties accumulated by the sequencer during the batch processing
    pub accumulated_penalty: Amount,
}

impl std::fmt::Display for Rewards {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Print the net reward. We do all calculations in u64 to avoid overflows and then add the correct sign.
        if self.accumulated_reward >= self.accumulated_penalty {
            let output = self
                .accumulated_reward
                .saturating_sub(self.accumulated_penalty);
            write!(f, "{output}")
        } else {
            let negative_reward = self
                .accumulated_penalty
                .saturating_sub(self.accumulated_reward);
            write!(f, "-{negative_reward}")
        }
    }
}

/// Outcome of batch execution.
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, derive_more::Display,
)]
#[display("{{ rewards: {} }}", rewards)]
#[serde(rename_all = "snake_case")]
pub struct BatchSequencerOutcome {
    /// Sequencer receives reward amount in defined token and can withdraw its deposit. The amount is net of any penalties.
    pub rewards: Rewards,
}

/// A receipt for a batch that was submitted by a sequencer to the DA layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, derive_more::Display)]
#[display(
    "{{ da_address: {}, gas_price: {}, gas_used: {}, outcome: {} }}",
    da_address,
    gas_price,
    gas_used,
    outcome
)]
#[serde(bound = "S: Spec")]
pub struct BatchSequencerReceipt<S: Spec> {
    /// The da address of the sequencer that submitted the batch.
    pub da_address: <<S as Spec>::Da as DaSpec>::Address,
    /// Gas price for a given batch.
    pub gas_price: <S::Gas as Gas>::Price,
    /// Gas used during the batch execution.
    pub gas_used: S::Gas,
    /// The sequencer outcome for this batch.
    pub outcome: BatchSequencerOutcome,
}

#[cfg(test)]
mod tests {
    use sov_mock_da::MockDaSpec;
    use sov_mock_zkvm::MockZkvm;

    use super::*;
    use crate::default_spec::DefaultSpec;
    use crate::execution_mode::Native;
    use crate::Address;
    type TestSpec = DefaultSpec<MockDaSpec, MockZkvm, MockZkvm, Native>;

    #[test]
    fn test_batch_with_id_size() {
        let batch = Arc::new(vec![
            FullyBakedTx::new(vec![1, 2, 3]), // size = 3
            FullyBakedTx::new(vec![1]),       // size = 1
        ]);
        let id = [11; 32]; // size = 32
        let sequencer_address = Address::new([22; 28]); // size = 28

        let manually_calulated_batch_size = 3 + 1 + 32 + 28;

        let batch_with_id = BatchWithId::<TestSpec>::new(batch, id, sequencer_address);

        assert_eq!(
            manually_calulated_batch_size,
            batch_with_id.batch_with_id_size()
        );
    }

    #[test]
    fn test_iterable_batch_with_id_remaining() {
        let batch = Arc::new(vec![FullyBakedTx::new(vec![]), FullyBakedTx::new(vec![])]);
        let batch_with_id = BatchWithId::<TestSpec>::new(batch, [0; 32], [0; 28].into());
        let mut batch_with_id = IterableBatchWithId::new(batch_with_id, NoOpControlFlow);
        assert_eq!(batch_with_id.remaining(), 2);
        batch_with_id.next();
        assert_eq!(batch_with_id.remaining(), 1);
        assert!(batch_with_id.next().is_some());
        assert_eq!(batch_with_id.remaining(), 0);
        assert!(batch_with_id.next().is_none());
        assert_eq!(batch_with_id.remaining(), 0);
        assert!(batch_with_id.next().is_none());
        assert_eq!(batch_with_id.remaining(), 0);
    }
}
