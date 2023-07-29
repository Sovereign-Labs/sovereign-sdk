use std::marker::PhantomData;

use borsh::BorshDeserialize;
use sov_modules_api::hooks::{ApplyBlobHooks, TxHooks};
use sov_modules_api::{Context, DispatchCall, Genesis};
use sov_rollup_interface::da::CountedBufReader;
use sov_rollup_interface::stf::BatchReceipt;
use sov_rollup_interface::zk::ValidityCondition;
use sov_rollup_interface::Buf;
use sov_state::StateCheckpoint;
use tracing::{debug, error};

use crate::tx_verifier::{verify_txs_stateless, TransactionAndRawHash};
use crate::{Batch, SequencerOutcome, SlashingReason, TxEffect};

pub struct AppTemplate<C: Context, RT, Vm, Cond> {
    pub current_storage: C::Storage,
    pub runtime: RT,
    pub(crate) checkpoint: Option<StateCheckpoint<C::Storage>>,
    phantom_vm: PhantomData<Vm>,
    phantom_cond: PhantomData<Cond>,
}

#[derive(Debug)]
pub enum ApplyBatchError {
    /// Contains batch hash
    Ignored([u8; 32]),
    Slashed {
        /// Contains batch hash
        hash: [u8; 32],
        reason: SlashingReason,
        sequencer_da_address: Vec<u8>,
    },
}

impl From<ApplyBatchError> for BatchReceipt<SequencerOutcome, TxEffect> {
    fn from(value: ApplyBatchError) -> Self {
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

impl<C: Context, RT, Vm, Cond: ValidityCondition> AppTemplate<C, RT, Vm, Cond>
where
    RT: DispatchCall<Context = C>
        + Genesis<Context = C>
        + TxHooks<Context = C>
        + ApplyBlobHooks<Context = C, BlobResult = SequencerOutcome>,
{
    pub fn new(storage: C::Storage, runtime: RT) -> Self {
        Self {
            runtime,
            current_storage: storage,
            checkpoint: None,
            phantom_vm: PhantomData::<Vm>::default(),
            phantom_cond: PhantomData::<Cond>::default(),
        }
    }

    // Do all stateless checks and data formatting, that can be results in sequencer slashing
    pub(crate) fn pre_process_batch(
        &self,
        blob_data: &mut CountedBufReader<impl Buf>,
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

    // Attempt to deserialize batch, error results in sequencer slashing.
    fn deserialize_batch(
        &self,
        blob_data: &mut CountedBufReader<impl Buf>,
    ) -> Result<Batch, SlashingReason> {
        match Batch::deserialize_reader(blob_data) {
            Ok(batch) => Ok(batch),
            Err(e) => {
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
    fn verify_txs_stateless(
        &self,
        batch: Batch,
    ) -> Result<Vec<TransactionAndRawHash<C>>, SlashingReason> {
        match verify_txs_stateless(batch.take_transactions()) {
            Ok(txs) => Ok(txs),
            Err(e) => {
                error!("Stateless verification error - the sequencer included a transaction which was known to be invalid. {}\n", e);
                Err(SlashingReason::StatelessVerificationFailed)
            }
        }
    }

    // Checks that runtime message can be decoded from transaction.
    // If a single message cannot be decoded, sequencer is slashed
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
