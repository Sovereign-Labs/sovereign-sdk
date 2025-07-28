//! Project level utilities that are used for testing the different crates of Sovereign SDK.
//!
//! WARNING: This crate is **NOT** intended to be used in production code. This is a testing utility crate.

#![deny(missing_docs)]

use std::sync::Arc;

pub use evm::simple_smart_contract::SimpleStorageContract;
pub use generators::MessageGenerator;
pub use interface::*;
pub use rt_agnostic_blueprint::RtAgnosticBlueprint;
use serde::{Deserialize, Serialize};
pub use sov_db::schema::SchemaBatch;
pub use sov_mock_da::verifier::MockDaSpec;
use sov_mock_da::BlockProducingConfig;
pub use sov_mock_zkvm::{MockZkvm, MockZkvmCryptoSpec};
use sov_modules_api::configurable_spec::ConfigurableSpec;
use sov_modules_api::default_spec::DefaultSpec;
use sov_modules_api::macros::config_value;
use sov_modules_api::transaction::{
    PriorityFeeBips, Transaction, TransactionCallable, TxDetails, UnsignedTransaction,
};
use sov_modules_api::{Address, Amount, BasicGasMeter, CryptoSpec, Gas, GasArray, Spec};
pub use sov_modules_api::{EncodeCall, TxProcessingError, TxReceiptContents};
pub use sov_modules_rollup_blueprint::logging::initialize_logging;
pub use sov_modules_stf_blueprint::get_gas_used;
use sov_modules_stf_blueprint::{BatchReceipt, StfBlueprint};
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::execution_mode::{Native, Zk};
use sov_state::nomt::prover_storage::NomtProverStorage;
pub use sov_state::ProverStorage;
use sov_state::{DefaultStorageSpec, StateAccesses, Storage};
pub use {sov_bank, sov_paymaster, sov_rollup_apis, sov_universal_wallet};

mod evm;

mod rt_agnostic_blueprint;

/// Utilities for recording logs.
pub mod logging;

/// End-to-end rollup node testing utilities.
pub mod test_rollup;

/// Utilities for generating test data.
pub mod generators;

/// Utilities for writing integration tests against ledger APIs (both Rust API and REST APIs).
pub mod ledger_db;

/// Utilities for testing the runtime.
pub mod runtime;

/// Utilities for testing the sequencer.
pub mod sequencer;
/// Utilities for testing that require [`ProverStorage`].
pub mod storage;

/// Utilities that specify an interface for testing.
pub mod interface;

/// The default test crypto spec type.
pub type TestCryptoSpec = MockZkvmCryptoSpec;
/// The default hasher type. This is the hasher type
/// ([`sov_rollup_interface::reexports::digest::Digest`]) defined by the
/// [`TestCryptoSpec`].
pub type TestHasher = <MockZkvmCryptoSpec as CryptoSpec>::Hasher;
/// The default storage spec type. Uses a [`TestHasher`] for hashing.
pub type TestStorageSpec = DefaultStorageSpec<TestHasher>;
/// The default test spec. Uses a [`MockZkvm`] for both inner and outer vm verification.
/// Uses [`MockZkvmCryptoSpec`] for cryptographic primitives.
pub type TestSpec = DefaultSpec<MockDaSpec, MockZkvm, MockZkvm, Native>;
/// Shortcut to [`sov_mock_da::MockHash`];
pub type TestSlotHash = <MockDaSpec as DaSpec>::SlotHash;
/// The default test spec for NOMT. Uses a [`MockZkvm`] for both inner and outer vm verification.
/// Uses [`MockZkvmCryptoSpec`] for cryptographic primitives.
pub type TestNomtSpec = ConfigurableSpec<
    MockDaSpec,
    MockZkvm,
    MockZkvm,
    Address,
    Native,
    MockZkvmCryptoSpec,
    NomtProverStorage<TestStorageSpec, TestSlotHash>,
>;
/// The default test spec for ZK. Uses a [`MockZkvm`] for both inner and outer vm verification.
pub type ZkTestSpec = DefaultSpec<MockDaSpec, MockZkvm, MockZkvm, Zk>;
/// The default address type. This is the [`sov_modules_api::BasicAddress`] type defined by the [`TestSpec`].
pub type TestAddress = <TestSpec as Spec>::Address;
/// The default private key type. This is the [`sov_rollup_interface::crypto::PrivateKey`] type defined by the [`TestSpec`].
pub type TestPrivateKey = <TestCryptoSpec as CryptoSpec>::PrivateKey;
/// The default public key type. This is the [`sov_rollup_interface::crypto::PublicKey`] type defined by the [`TestCryptoSpec`].
pub type TestPublicKey = <TestCryptoSpec as CryptoSpec>::PublicKey;
/// The default signature type. This is the [`sov_rollup_interface::crypto::Signature`] type defined by the [`TestCryptoSpec`].
pub type TestSignature = <TestCryptoSpec as CryptoSpec>::Signature;

/// The default STF blueprint type. Uses [`MockDaSpec`] for DA and [`sov_kernels::basic::BasicKernel`] for kernel.
pub type TestStfBlueprint<RT, S> = StfBlueprint<S, RT>;
/// The default [`sov_db::storage_manager::NativeStorageManager`], that can be used with [`ProverStorage`] and [`TestStorageSpec`].
pub type TestStorageManager =
    sov_db::storage_manager::NativeStorageManager<MockDaSpec, ProverStorage<TestStorageSpec>>;
// --- Blessed test parameters ---

// Blessed gas parameters

/// The maximum batch size that the preferred sequencer can create.
pub const TEST_MAX_BATCH_SIZE: usize = 1024 * 1024;

/// If a blob is not processed within this time, it will shut down the sequencer.
pub const TEST_BLOB_PROCESSING_TIMEOUT: u64 = 60;

/// The maximum number of concurrent blobs.
pub const TEST_MAX_CONCURRENT_BLOBS: usize = 16;

/// The default max fee to set for a transaction. This should be enough to be able to execute most standard transactions for the test rollup.
pub const TEST_DEFAULT_MAX_FEE: Amount = Amount::new(100_000_000_000);
/// The default gas limit to set for a transaction. This is an optional parameter.
/// This value should be high enough to be able to execute most standard transactions for the test rollup.
pub const TEST_DEFAULT_GAS_LIMIT: [u64; 2] = [1_000_000_000, 1_000_000_000];
/// The number of blocks required to finalize a block.
pub const TEST_FINALIZATION_BLOCKS: u32 = 3;
/// The default amount of tokens that should be staked by a user (prover, sequencer, etc.). This value is roughly equal to the
/// max fee for a transaction because sequencers need to pre-emptively pay for all transactions' pre-execution checks using their stake.
pub const TEST_DEFAULT_USER_STAKE: [u64; 2] = [100_000_000_000, 100_000_000_000];
/// Minimum sequencer bond.
pub const TEST_MIN_SEQ_BOND: Amount = Amount::new(1000);
/// The default amount of tokens that should be in the user's bank account. This amount should always be higher than [`TEST_DEFAULT_MAX_FEE`] and
/// [`TEST_DEFAULT_USER_STAKE`]. This value is set so that the user can send a dozen transactions without having to refill its bank account.
pub const TEST_DEFAULT_USER_BALANCE: Amount = Amount::new(1_000_000_000_000_000);
/// The default max priority fee to set for a transaction. We are setting this value to zero to avoid having to do
/// priority fee accounting in the tests. If a test needs to test sequencer rewards, it should set the transaction priority fee
/// to a non-zero value.
pub const TEST_DEFAULT_MAX_PRIORITY_FEE: PriorityFeeBips = PriorityFeeBips::from_percentage(0);

// --- End Blessed gas parameters (used for testing) ---

// Blessed rollup constants
// Constants used in the genesis configuration of the test runtime

// --- Attester incentives constants ---
/// The default max attested height at the genesis of the rollup. This is the height that contains the highest attestation
/// for the rollup. This value is set to zero in tests because the rollup always starts at zeroth height.
pub const TEST_MAX_ATTESTED_HEIGHT: SlotNumber = SlotNumber::GENESIS;
/// The default finalized height of the light client. This value should always be below the [`TEST_MAX_ATTESTED_HEIGHT`].
/// This value is set to zero in tests because the rollup always starts at zeroth height. This value should be manually
/// updated for now because light clients are not yet implemented.
pub const TEST_LIGHT_CLIENT_FINALIZED_HEIGHT: SlotNumber = SlotNumber::GENESIS;
/// The default rollup finality period. Used by the [`sov_attester_incentives::AttesterIncentives`] module to determine the
/// range of heights that are eligible for attestations.
pub const TEST_ROLLUP_FINALITY_PERIOD: u64 = 5;
/// The default name to use for the gas token.
pub const TEST_GAS_TOKEN_NAME: &str = "TestGasToken";

/// Default [`sov_stf_runner::ProofManagerConfig::prover_address`] value in tests.
pub const TEST_DEFAULT_PROVER_ADDRESS: &str =
    "sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf";

/// Default [`sov_sequencer::SequencerConfig::rollup_address`] value in tests.
pub const TEST_DEFAULT_SEQUENCER_ADDRESS: &str =
    "sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf";

/// Default wait time value for different [`sov_mock_da::BlockProducingConfig`] value in tests.
pub const TEST_DEFAULT_MOCK_DA_BLOCK_TIME_MS: u64 = 100;
/// Default [`BlockProducingConfig`] for tests that need periodic block producing.
pub const TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING: BlockProducingConfig =
    BlockProducingConfig::Periodic {
        block_time_ms: TEST_DEFAULT_MOCK_DA_BLOCK_TIME_MS,
    };
/// Default [`BlockProducingConfig`] for tests that need on batch submit variant
pub const TEST_DEFAULT_MOCK_DA_ON_SUBMIT: BlockProducingConfig =
    BlockProducingConfig::OnBatchSubmit {
        block_wait_timeout_ms: Some(TEST_DEFAULT_MOCK_DA_BLOCK_TIME_MS),
    };
/// Default [`BlockProducingConfig`] for tests that need on any submit variant
pub const TEST_DEFAULT_MOCK_DA_ON_ANY_SUBMIT: BlockProducingConfig =
    BlockProducingConfig::OnAnySubmit {
        block_wait_timeout_ms: Some(TEST_DEFAULT_MOCK_DA_BLOCK_TIME_MS),
    };

/// Generates a default [`TxDetails`] for testing.
pub fn default_test_tx_details<S: Spec>() -> TxDetails<S> {
    TxDetails {
        max_priority_fee_bips: TEST_DEFAULT_MAX_PRIORITY_FEE,
        max_fee: TEST_DEFAULT_MAX_FEE,
        gas_limit: None,
        chain_id: config_value!("CHAIN_ID"),
    }
}

/// Creates signed transaction with default test parameters from serializable RuntimeCallMessage.
pub fn default_test_signed_transaction<T: TransactionCallable, S: Spec>(
    key: &<<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey,
    msg: &T::Call,
    nonce: u64,
    chain_hash: &[u8; 32],
) -> Transaction<T, S> {
    let tx_details = default_test_tx_details::<S>();
    test_signed_transaction(key, msg, nonce, chain_hash, tx_details)
}

/// Creates signed transaction.
pub fn test_signed_transaction<T: TransactionCallable, S: Spec>(
    key: &<<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey,
    msg: &T::Call,
    nonce: u64,
    chain_hash: &[u8; 32],
    tx_details: TxDetails<S>,
) -> Transaction<T, S> {
    Transaction::<T, S>::new_signed_tx(
        key,
        chain_hash,
        UnsignedTransaction::new(
            msg.clone(),
            tx_details.chain_id,
            tx_details.max_priority_fee_bips,
            tx_details.max_fee,
            nonce,
            tx_details.gas_limit,
        ),
    )
}

/// An implementation of [`sov_rollup_interface::stf::TxReceiptContents`] for testing. TestTxReceiptContents uses
/// a `u32` as the receipt contents.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct TestTxReceiptContents;

impl sov_rollup_interface::stf::TxReceiptContents for TestTxReceiptContents {
    type Skipped = u32;
    type Reverted = u32;
    type Successful = u32;
    type Ignored = u32;
}

/// Keep reference to [`Amount`] for using in tests.
#[derive(Clone)]
pub struct AtomicAmount {
    num: Arc<std::sync::Mutex<Amount>>,
}

impl AtomicAmount {
    /// Create a new [`AtomicAmount`]
    pub fn new(amount: Amount) -> Self {
        Self {
            num: Arc::new(std::sync::Mutex::new(amount)),
        }
    }

    /// Get the current value of the [`AtomicAmount`]
    pub fn get(&self) -> Amount {
        *self.num.lock().unwrap()
    }

    /// Add a value to the [`AtomicAmount`]
    pub fn add(&self, value: Amount) {
        let mut amount = self.num.lock().unwrap();
        *amount = amount
            .checked_add(value)
            .expect("Too big amount for addition");
    }

    /// Subtract a value from the [`AtomicAmount`]
    pub fn sub(&self, value: Amount) {
        let mut amount = self.num.lock().unwrap();
        *amount = amount
            .checked_sub(value)
            .expect("Insufficient amount for subtraction");
    }
}

/// BasicGasMeter for tests.
pub fn new_test_gas_meter<S: Spec>() -> BasicGasMeter<S> {
    BasicGasMeter::new_with_gas(<S::Gas as Gas>::max(), <S::Gas as Gas>::Price::ZEROED)
}

/// BasicGasMeter for tests.
pub fn new_test_gas_meter_with_price<S: Spec>(
    gas_price: <S::Gas as Gas>::Price,
) -> BasicGasMeter<S> {
    BasicGasMeter::new_with_gas(<S::Gas as Gas>::max(), gas_price)
}

/// Serializes a value to JSON and validates it based on its
/// [`schemars::JsonSchema`] rules.
#[allow(clippy::result_large_err)]
pub fn validate_schema<T>(item: &T) -> Result<(), jsonschema::error::ValidationErrorKind>
where
    T: schemars::JsonSchema + serde::Serialize,
{
    let schema = serde_json::to_value(schemars::schema_for!(T)).unwrap();
    let json = serde_json::to_value(item).unwrap();

    jsonschema::validate(&schema, &json).map_err(|e| e.kind)
}

/// Validate all the storage accesses in a particular cache log,
/// returning the new state root and change set after applying all writes.
/// This function is equivalent to calling:
/// `self.compute_state_update` & `self.materialize_changes`
pub fn validate_and_materialize<ST: Storage>(
    storage: ST,
    state_accesses: StateAccesses,
    witness: &ST::Witness,
    prev_state_root: ST::Root,
) -> anyhow::Result<(ST::Root, ST::ChangeSet)> {
    let (root_hash, node_batch) =
        storage.compute_state_update(state_accesses, witness, prev_state_root)?;

    let change_set = storage.materialize_changes(node_batch);
    Ok((root_hash, change_set))
}
