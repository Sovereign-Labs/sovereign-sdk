//! Several utilities needed to generate message for testing.
//!
//! TODO: Add a doctest to describe how to generate messages in tests.

use std::num::NonZero;
use std::rc::Rc;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use sov_blob_storage::PreferredBatchData;
use sov_modules_api::capabilities::{TransactionAuthenticator, UniquenessData};
use sov_modules_api::macros::config_value;
use sov_modules_api::transaction::{PriorityFeeBips, Transaction, TxDetails, UnsignedTransaction};
use sov_modules_api::{Amount, CryptoSpec, EncodeCall, FullyBakedTx, Module, RawTx, Spec};
use sov_modules_stf_blueprint::Runtime;

use crate::{TEST_DEFAULT_GAS_LIMIT, TEST_DEFAULT_MAX_FEE, TEST_DEFAULT_MAX_PRIORITY_FEE};

/// Utilities for generating messages for the bank module.
pub mod bank;
/// Utilities for generating messages for the sequencer registry module.
pub mod sequencer_registry;
/// Utilities for generating messages for the value setter module.
pub mod value_setter;

/// A generic message object used to create transactions.
pub struct Message<S: Spec, Mod: Module> {
    /// The sender's private key.
    pub sender_key: Rc<<S::CryptoSpec as CryptoSpec>::PrivateKey>,
    /// The message content.
    pub content: Mod::CallMessage,
    /// Data related to fees and gas handling.
    pub details: TxDetails<S>,
    /// The message generation number.
    pub generation: u64,
}

impl<S: Spec, Mod: Module> Message<S, Mod> {
    fn new(
        sender_key: Rc<<S::CryptoSpec as CryptoSpec>::PrivateKey>,
        content: Mod::CallMessage,
        chain_id: u64,
        max_priority_fee_bips: PriorityFeeBips,
        max_fee: Amount,
        gas_limit: Option<S::Gas>,
        generation: u64,
    ) -> Self {
        Self {
            sender_key,
            content,
            details: TxDetails {
                chain_id,
                max_priority_fee_bips,
                max_fee,
                gas_limit,
            },
            generation,
        }
    }

    /// Converts a [`Message`] into a [`Transaction`] using the [`TxDetails`] provided by the [`Message`].
    pub fn to_tx<RT: EncodeCall<Mod> + Runtime<S>>(
        self,
    ) -> sov_modules_api::transaction::Transaction<RT, S> {
        Transaction::<RT, S>::new_signed_tx(
            &self.sender_key,
            &RT::CHAIN_HASH,
            UnsignedTransaction::new(
                <RT as EncodeCall<Mod>>::to_decodable(self.content),
                self.details.chain_id,
                self.details.max_priority_fee_bips,
                self.details.max_fee,
                UniquenessData::Generation(self.generation),
                self.details.gas_limit,
            ),
        )
    }
}

/// The execution mode of the blob sender. This is used to specify how to appropriately serialize blobs (
/// using the [`PreferredBatchData`] struct or a standard batch ).
pub enum BlobBuildingCtx {
    /// Standard execution mode
    Standard,
    /// Preferred execution mode (when the preferred sequencer is used)
    Preferred {
        /// The current sequence number to build the batch with
        /// We are using an atomic because we need to be able to increment the sequence number
        /// in a thread-safe way (a lot of integration tests are multi-threaded by default).
        curr_sequence_number: Arc<AtomicU64>,
    },
}

/// Trait used to generate messages from the DA layer to automate module testing
pub trait MessageGenerator {
    /// Module where the messages originate from.
    type Module: Module;

    /// Module spec
    type Spec: Spec;

    /// The default chain ID to use for the messages. Defaults to `constants.toml` constant.
    fn default_chain_id() -> u64 {
        config_value!("CHAIN_ID")
    }

    /// Generates a list of messages originating from the module using the provided transaction details.
    fn create_messages(
        &self,
        chain_id: u64,
        max_priority_fee_bips: PriorityFeeBips,
        max_fee: Amount,
        estimated_gas_usage: Option<<Self::Spec as Spec>::Gas>,
    ) -> Vec<Message<Self::Spec, Self::Module>>;

    /// Generates a list of messages originating from the module using default transaction details.
    /// Note: sets the gas usage to the default gas limit.
    fn create_default_messages(&self) -> Vec<Message<Self::Spec, Self::Module>> {
        self.create_messages(
            Self::default_chain_id(),
            TEST_DEFAULT_MAX_PRIORITY_FEE,
            TEST_DEFAULT_MAX_FEE,
            Some(<Self::Spec as Spec>::Gas::from(TEST_DEFAULT_GAS_LIMIT)),
        )
    }

    /// Generates a list of messages originating from the module using default transaction details and no gas usage.
    fn create_default_messages_without_gas_usage(&self) -> Vec<Message<Self::Spec, Self::Module>> {
        self.create_messages(
            Self::default_chain_id(),
            TEST_DEFAULT_MAX_PRIORITY_FEE,
            TEST_DEFAULT_MAX_FEE,
            None,
        )
    }

    /// Creates a vector of raw transactions from the module.
    fn create_default_encoded_txs<RT: Runtime<Self::Spec> + EncodeCall<Self::Module>>(
        &self,
    ) -> Vec<FullyBakedTx> {
        self.create_encoded_txs::<RT>(
            Self::default_chain_id(),
            TEST_DEFAULT_MAX_PRIORITY_FEE,
            TEST_DEFAULT_MAX_FEE,
            Some(<Self::Spec as Spec>::Gas::from(TEST_DEFAULT_GAS_LIMIT)),
        )
    }

    /// Generates a list of raw transactions originating from the module using default transaction details and no gas usage.
    fn create_default_encoded_txs_without_gas_usage<
        RT: Runtime<Self::Spec> + EncodeCall<Self::Module>,
    >(
        &self,
    ) -> Vec<FullyBakedTx> {
        self.create_encoded_txs::<RT>(
            Self::default_chain_id(),
            TEST_DEFAULT_MAX_PRIORITY_FEE,
            TEST_DEFAULT_MAX_FEE,
            None,
        )
    }

    /// Creates a vector of raw transactions from the module.
    fn create_encoded_txs<RT: Runtime<Self::Spec> + EncodeCall<Self::Module>>(
        &self,
        chain_id: u64,
        max_priority_fee_bips: PriorityFeeBips,
        max_fee: Amount,
        estimated_gas_usage: Option<<Self::Spec as Spec>::Gas>,
    ) -> Vec<FullyBakedTx> {
        let messages_iter = self
            .create_messages(
                chain_id,
                max_priority_fee_bips,
                max_fee,
                estimated_gas_usage,
            )
            .into_iter();
        let mut serialized_messages = Vec::default();
        for message in messages_iter {
            let tx = message.to_tx::<RT>();
            serialized_messages.push(RT::Auth::encode_with_standard_auth(RawTx::new(
                borsh::to_vec(&tx).unwrap(),
            )));
        }
        serialized_messages
    }

    /// Generates a list of blobs originating from the module using default transaction details.
    /// This function calls [`MessageGenerator::create_default_encoded_txs`].
    fn create_blobs<RT: Runtime<Self::Spec> + EncodeCall<Self::Module>>(
        &self,
        mode: &BlobBuildingCtx,
    ) -> Vec<u8> {
        let txs: Vec<FullyBakedTx> = self
            .create_default_encoded_txs::<RT>()
            .into_iter()
            .collect();

        match mode {
            BlobBuildingCtx::Standard => borsh::to_vec(&txs).unwrap(),
            BlobBuildingCtx::Preferred {
                curr_sequence_number,
            } => {
                let batch = PreferredBatchData {
                    data: txs.into(),
                    sequence_number: curr_sequence_number
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst),
                    visible_slots_to_advance: NonZero::new(1).unwrap(),
                };

                borsh::to_vec(&batch).unwrap()
            }
        }
    }
}
