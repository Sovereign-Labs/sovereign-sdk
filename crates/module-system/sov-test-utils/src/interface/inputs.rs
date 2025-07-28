use std::collections::HashMap;

use derivative::Derivative;
use sov_mock_da::{MockAddress, MockBlob};
use sov_modules_api::capabilities::TransactionAuthenticator;
use sov_modules_api::transaction::{PriorityFeeBips, Transaction, TxDetails, UnsignedTransaction};
use sov_modules_api::{Amount, CryptoSpec, DispatchCall, FullyBakedTx, PrivateKey, RawTx, Spec};
use sov_rollup_interface::da::RelevantBlobs;

use crate::runtime::Runtime;

/// Defines the type of a message that can be sent to the runtime.
#[derive(Derivative)]
#[derivative(
    Clone(bound = "RT: Runtime<S>, S: Spec"),
    Debug(bound = "RT: Runtime<S>, S: Spec")
)]
pub enum TransactionType<RT: Runtime<S>, S: Spec> {
    /// A transaction which is pre-signed and pre-wrapped in the `<Runtime as TransactionAuthenticator>::Input` type.
    PreAuthenticated(FullyBakedTx),
    /// A pre-signed transaction. Ie, a transaction that has already been signed and formatted by the sender
    PreSigned(RawTx),
    /// A plain transaction. That is a transaction that has not been signed or encoded yet
    Plain {
        /// A plain call message to be sent.
        message: <RT as DispatchCall>::Decodable,
        /// The private key of the sender.
        key: <S::CryptoSpec as CryptoSpec>::PrivateKey,
        /// The details of the transaction.
        details: TxDetails<S>,
    },
}

impl<RT: Runtime<S>, S: Spec> TransactionType<RT, S> {
    /// Get a mutable reference to the [`TxDetails`] if self is [`TransactionType::Plain`].
    /// Otherwise returns [`None`].
    pub fn details_mut(&mut self) -> Option<&mut TxDetails<S>> {
        Some(match self {
            TransactionType::PreAuthenticated(_) | TransactionType::PreSigned { .. } => {
                return None
            }
            TransactionType::Plain { details, .. } => details,
        })
    }

    /// Override the details of the transaction. This method panics if called with [`TransactionType::PreSigned`].
    pub fn with_details(self, details: TxDetails<S>) -> Self {
        match self {
            TransactionType::Plain { message, key, .. } => TransactionType::Plain {
                message,
                key,
                details,
            },
            TransactionType::PreSigned(_) => {
                panic!("PreSigned transactions cannot specify custom details")
            }
            TransactionType::PreAuthenticated(_) => {
                panic!("PreAuthenticated transactions cannot specify custom details")
            }
        }
    }

    /// Set the chain ID of the transaction.
    pub fn with_chain_id(mut self, chain_id: u64) -> Self {
        if let Some(details) = self.details_mut() {
            details.chain_id = chain_id;
        }

        self
    }

    /// Set the max priority fee of the transaction.
    pub fn with_max_priority_fee_bips(mut self, max_priority_fee_bips: PriorityFeeBips) -> Self {
        if let Some(details) = self.details_mut() {
            details.max_priority_fee_bips = max_priority_fee_bips;
        }

        self
    }

    /// Set the max fee of the transaction.
    pub fn with_max_fee(mut self, max_fee: Amount) -> Self {
        if let Some(details) = self.details_mut() {
            details.max_fee = max_fee;
        }

        self
    }

    /// Set the gas limit of the transaction.
    pub fn with_gas_limit(mut self, gas_limit: Option<S::Gas>) -> Self {
        if let Some(details) = self.details_mut() {
            details.gas_limit = gas_limit;
        }

        self
    }

    /// Converts a [`TransactionType`] into a serialized authenticated transaction ready to be passed
    /// to the runtime.
    pub fn to_serialized_authenticated_tx(
        self,
        nonces: &mut HashMap<<S::CryptoSpec as CryptoSpec>::PublicKey, u64>,
    ) -> FullyBakedTx {
        match self {
            TransactionType::PreAuthenticated(data) => data,
            TransactionType::PreSigned(raw_tx) => RT::Auth::encode_with_standard_auth(raw_tx),
            TransactionType::Plain {
                message,
                key,
                details,
            } => RT::Auth::encode_with_standard_auth(Self::sign_and_serialize(
                message,
                key,
                &RT::CHAIN_HASH,
                details,
                nonces,
            )),
        }
    }

    /// Creates a [`TransactionType`] from an [`UnsignedTransaction`].
    pub fn pre_signed(
        unsigned_tx: UnsignedTransaction<RT, S>,
        key: &<S::CryptoSpec as CryptoSpec>::PrivateKey,
        chain_hash: &[u8; 32],
    ) -> Self {
        let tx = borsh::to_vec(&Transaction::new_signed_tx(key, chain_hash, unsigned_tx)).unwrap();
        Self::PreSigned(RawTx { data: tx })
    }

    /// Sign a message with the given key, generating a [`Transaction`].
    pub fn sign(
        msg: <RT as DispatchCall>::Decodable,
        key: <S::CryptoSpec as CryptoSpec>::PrivateKey,
        chain_hash: &[u8; 32],
        details: TxDetails<S>,
        nonces: &mut HashMap<<S::CryptoSpec as CryptoSpec>::PublicKey, u64>,
    ) -> Transaction<RT, S> {
        let pub_key = key.pub_key();
        let nonce = *nonces.get(&pub_key).unwrap_or(&0);
        nonces.insert(pub_key, nonce + 1);
        Transaction::<RT, S>::new_signed_tx(
            &key,
            chain_hash,
            UnsignedTransaction::new(
                msg,
                details.chain_id,
                details.max_priority_fee_bips,
                details.max_fee,
                nonce,
                details.gas_limit,
            ),
        )
    }

    /// Sign and borsh-serialize a message with the given key, generating a [`RawTx`]
    pub fn sign_and_serialize(
        msg: <RT as DispatchCall>::Decodable,
        key: <S::CryptoSpec as CryptoSpec>::PrivateKey,
        chain_hash: &[u8; 32],
        details: TxDetails<S>,
        nonces: &mut HashMap<<S::CryptoSpec as CryptoSpec>::PublicKey, u64>,
    ) -> RawTx {
        let tx = Self::sign(msg, key, chain_hash, details, nonces);

        RawTx {
            data: borsh::to_vec(&tx).unwrap(),
        }
    }
}

/// Defines the type of batch that can be sent to the runtime.
pub struct BatchType<RT: Runtime<S>, S: Spec>(pub Vec<TransactionType<RT, S>>);

impl<RT: Runtime<S>, S: Spec> From<Vec<TransactionType<RT, S>>> for BatchType<RT, S> {
    fn from(value: Vec<TransactionType<RT, S>>) -> Self {
        Self(value)
    }
}

/// Defines the proof that can be sent to the runtime.
pub struct ProofInput(pub Vec<u8>);

/// Input that can be executed in a slot ran by the test runtime.
#[derive(derive_more::From)]
pub enum SlotInput<RT: Runtime<S>, S: Spec> {
    /// Execute a transaction as input to a slot.
    Transaction(TransactionType<RT, S>),
    /// Execute a batch as input to a slot.
    Batch(BatchType<RT, S>),
    /// Execute a batch as input to a slot.
    Batches(Vec<BatchType<RT, S>>),
    /// Execute a proof as input to a slot.
    Proof(ProofInput),
    /// Execute pre-encoded blobs as input to a slot.
    Blobs(RelevantBlobs<MockBlob>),
}

/// Information about the sequencer to use in soft-confirmation mode
#[derive(Clone)]
pub enum SequencerInfo {
    /// This is a preferred sequencer
    Preferred {
        /// The number of visible slots to advance
        slots_to_advance: u8,
        /// The sequence number for this batch
        sequence_number: u64,
    },
    /// This is a regular sequencer
    Regular,
}

/// Information to build blobs in soft-confirmation mode
pub struct SoftConfirmationBlobInfo<RT: Runtime<S>, S: Spec> {
    /// The batch to be included in the blob
    pub batch_type: BatchType<RT, S>,
    /// The address of the sequencer
    pub sequencer_address: MockAddress,
    /// Additional information about the sequencer
    pub sequencer_info: SequencerInfo,
}
