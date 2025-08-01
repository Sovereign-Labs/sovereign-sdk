use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::num::NonZero;

use derive_more::derive::Display;
use reqwest::Client;
pub use sov_accounts::Accounts;
pub use sov_attester_incentives::{
    AttesterIncentives, AttesterIncentivesConfig, CallMessage as AttesterCallMessage,
};
pub use sov_bank::{
    config_gas_token_id, Bank, BankConfig, CallMessage as BankCallMessage, Coins, IntoPayable,
    Payable, TokenConfig, TokenId,
};
pub use sov_blob_storage::BlobStorage;
use sov_blob_storage::PreferredBatchData;
pub use sov_capabilities::StandardProvenRollupCapabilities;
pub use sov_chain_state::{ChainState, ChainStateConfig};
use sov_db::storage_manager::NativeChangeSet;
pub use sov_kernels::basic::BasicKernel;
pub use sov_kernels::soft_confirmations::SoftConfirmationsKernel;
use sov_mock_da::{MockAddress, MockBlob, MockBlockHeader, MockDaSpec};
use sov_modules_api::capabilities::{ChainState as _, Kernel, RollupHeight};
use sov_modules_api::da::Time;
use sov_modules_api::prelude::utoipa::openapi::OpenApi;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::rest::{ApiState, HasRestApi};
use sov_modules_api::{
    Amount, ApiStateAccessor, ApplySlotOutput, BlobReaderTrait, CryptoSpec, DaSpec, EncodeCall,
    Error, Gas, Genesis, InfallibleStateAccessor, Module, PrivateKey, Spec, StateCheckpoint,
    TransactionReceipt, TxEffect, VersionReader, VisibleSlotNumber, *,
};
use sov_modules_stf_blueprint::{get_gas_used, StfBlueprint};
pub use sov_modules_stf_blueprint::{GenesisParams, Runtime};
pub use sov_operator_incentives::{OperatorIncentives, OperatorIncentivesConfig};
pub use sov_paymaster::Paymaster;
pub use sov_prover_incentives::{ProverIncentives, ProverIncentivesConfig};
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::da::RelevantBlobs;
use sov_rollup_interface::stf::{ExecutionContext, StateTransitionFunction};
pub use sov_sequencer_registry::{self, SequencerRegistry, SequencerRegistryConfig};
use sov_state::{DefaultStorageSpec, Storage, StorageProof};
pub use sov_uniqueness::Uniqueness;
pub use sov_value_setter::{
    CallMessage as ValueSetterCallMessage, Event as ValueSetterEvent, ValueSetter,
    ValueSetterConfig,
};
use tokio::sync::watch::{self};
pub use {
    sov_accounts, sov_attester_incentives, sov_bank, sov_blob_storage, sov_paymaster,
    sov_value_setter,
};

use crate::storage::{ForklessStorageManager, SimpleStorageManager};
use crate::{
    generate_optimistic_runtime, validate_and_materialize, Arc, BatchAssertContext, BatchReceipt,
    BatchTestCase, BatchType, ProofAssertContext, ProofTestCase, SequencerInfo, SlotInput,
    SoftConfirmationBlobInfo, TestStfBlueprint, TransactionAssertContext, TransactionTestCase,
    TransactionType,
};

pub(crate) mod macros;

generate_optimistic_runtime!(TestOptimisticRuntime <= value_setter: ValueSetter<S>, paymaster: Paymaster<S>);

/// Utilities for generating genesis configs.
pub mod genesis;

/// Traits used to define interfaces for the runtime.
pub mod traits;
use traits::MinimalGenesis;

type NoncesMap<S> = HashMap<<<S as Spec>::CryptoSpec as CryptoSpec>::PublicKey, u64>;

/// Metadata about a blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobInfo {
    /// The size of the blob.
    pub size: usize,
}

/// Metadata about the blobs in a slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelevantBlobInfo {
    /// Metadata about the proof blobs.
    pub proof_blobs: Vec<BlobInfo>,
    /// Metadata about the batch blobs.
    pub batch_blobs: Vec<BlobInfo>,
}

impl RelevantBlobInfo {
    /// Creates a new `RelevantBlobInfo` from a `RelevantBlobs` struct.
    pub fn from_blobs(blobs: &RelevantBlobs<MockBlob>) -> Self {
        Self {
            proof_blobs: blobs
                .proof_blobs
                .iter()
                .map(|blob| BlobInfo {
                    size: blob.total_len(),
                })
                .collect(),
            batch_blobs: blobs
                .batch_blobs
                .iter()
                .map(|blob| BlobInfo {
                    size: blob.total_len(),
                })
                .collect(),
        }
    }
}

/// Defines a slot receipt. A slot receipt is a list of [`BatchReceipt`]s and a block header.
pub struct SlotReceipt<S: Spec> {
    #[allow(missing_docs)]
    pub discarded_blobs: Vec<HexHash>,
    #[allow(missing_docs)]
    pub batch_receipts: Vec<BatchReceipt<S>>,
    #[allow(missing_docs, clippy::type_complexity)]
    pub proof_receipts: Vec<
        ProofReceipt<
            S::Address,
            S::Da,
            <S::Storage as Storage>::Root,
            StorageProof<<S::Storage as Storage>::Proof>,
        >,
    >,
    #[allow(missing_docs)]
    pub state_root: <S::Storage as Storage>::Root,
}

impl<S: Spec> SlotReceipt<S> {
    /// Returns the last batch receipt in the slot receipt.
    pub fn last_batch_receipt(&self) -> &BatchReceipt<S> {
        self.batch_receipts.last().unwrap()
    }

    /// Returns the last transaction receipt in the last batch receipt of the slot receipt.
    pub fn last_tx_receipt(&self) -> &TransactionReceipt<S> {
        self.last_batch_receipt().tx_receipts.last().unwrap()
    }

    /// Returns the batch receipts contained in the slot receipt.
    pub fn batch_receipts(&self) -> &[BatchReceipt<S>] {
        &self.batch_receipts
    }
}

/// TestRunner specific configuration values.
pub struct RunnerConfig<Da: DaSpec> {
    /// The sequencers DA address used as the address of the sender of a blob.
    pub sequencer_da_address: Da::Address,
    /// All blocks produced by the runner will be at the time provided.
    /// This is useful if your tests are dependent on timestamps.
    pub freeze_time: Option<Time>,
    /// The address to bind the rest API server on.
    ///
    /// ## Note
    /// By default we are using the port 0 to ensure that the allocated port is not taken by another process.
    pub axum_addr: SocketAddr,
}

/// Stateful test runner that can be used to run and accumulate slot results for a given runtime.
pub struct TestRunner<
    RT: Runtime<S>,
    S: Spec,
    Sm: ForklessStorageManager = SimpleStorageManager<
        DefaultStorageSpec<<<S as Spec>::CryptoSpec as CryptoSpec>::Hasher>,
    >,
> {
    stf: StfBlueprint<S, RT>,
    nonces: HashMap<<S::CryptoSpec as CryptoSpec>::PublicKey, u64>,
    slot_receipts: Vec<SlotReceipt<S>>,
    state_root: <S::Storage as Storage>::Root,
    storage_manager: Sm,
    /// A channel to send the storage over. This should be subscribed to the same channel as [`Self::checkpoint_receiver`].
    checkpoint_sender: watch::Sender<StateCheckpoint<S>>,
    /// The corresponding receiving end of the channel.
    checkpoint_receiver: watch::Receiver<StateCheckpoint<S>>,
    axum_server: axum_server::Handle,
    /// Test runner configuration.
    pub config: RunnerConfig<S::Da>,
}

impl<RT: Runtime<S>, S: Spec, Sm: ForklessStorageManager> Drop for TestRunner<RT, S, Sm> {
    fn drop(&mut self) {
        self.axum_server.shutdown();
    }
}

/// The output of the apply slot function that uses the test spec and da spec.
pub type TestApplySlotOutput<RT, S> = ApplySlotOutput<
    <S as Spec>::InnerZkvm,
    <S as Spec>::OuterZkvm,
    <S as Spec>::Da,
    TestStfBlueprint<RT, S>,
>;

/// The output of the runner
pub struct RunnerOutput<S: Spec> {
    /// The slot receipt emitted at the end of the slot execution
    pub receipt: SlotReceipt<S>,
    /// The change set containing the delta of the state after the slot execution
    pub change_set: NativeChangeSet,
    /// The root of the state after the slot execution
    pub root: <<S as Spec>::Storage as Storage>::Root,
}

/// The default format of the data returned by the rest API GET requests to the module data.
#[derive(serde::Deserialize, Debug, PartialEq)]
pub struct ApiGetStateData<T> {
    /// The value returned by the REST API.
    pub value: Option<T>,
}

/// A wrapper type to specify the path of an API endpoint.
#[derive(Debug, Clone, PartialEq, Display)]
pub struct ApiPath(pub String);

impl ApiPath {
    /// Returns the default path for API queries to the given module.
    pub fn query_module(module_name: &str) -> Self {
        let module_name = if !module_name.starts_with('/') {
            format!("/{module_name}")
        } else {
            module_name.to_string()
        };

        ApiPath(format!("/modules{module_name}"))
    }

    /// Adds a custom path to the API endpoint.
    pub fn with_custom_api_path(self, path: &str) -> Self {
        let path = if !path.starts_with('/') {
            format!("/{path}")
        } else {
            path.to_string()
        };

        ApiPath(format!("{self}{path}"))
    }

    /// Returns the default path for the API of the given module and state key.
    pub fn with_default_state_path(self, state_key: &str) -> Self {
        let state_key = if !state_key.starts_with('/') {
            format!("/{state_key}")
        } else {
            state_key.to_string()
        };

        ApiPath(format!("{self}/state{state_key}"))
    }

    /// Adds an item number to the path.
    pub fn get_item_number(self, item: u64) -> Self {
        ApiPath(format!("{self}/items/{item}"))
    }

    /// Adds a rollup height to the path.
    pub fn with_rollup_height(self, height: u64) -> Self {
        ApiPath(format!("{self}?rollup_height={height}"))
    }
}

impl<RT, S, Sm> TestRunner<RT, S, Sm>
where
    RT: Runtime<S> + MinimalGenesis<S>,
    Sm: ForklessStorageManager,
    S: Spec<Storage = Sm::Storage, Da = MockDaSpec>,
    <S::Storage as Storage>::ChangeSet: Clone,
    <S::Storage as Storage>::Root: Clone,
{
    /// Returns the runtime of the test runner.
    pub fn runtime(&self) -> &RT {
        self.stf.runtime()
    }

    /// Returns a reference to the storage manager of the test runner.
    pub fn storage_manager(&self) -> &Sm {
        &self.storage_manager
    }

    /// Returns the state root of the previous slot.
    /// Since genesis is always ran when constructing the runner, there will always be a previous slot when executing new slots.
    pub fn state_root(&self) -> &<S::Storage as Storage>::Root {
        &self.state_root
    }

    /// Returns the current "true" rollup height.
    ///
    /// ## Note (soft-confirmations)
    /// This value may be different from the value that would be returned by the [`ApiStateAccessor::current_visible_slot_number`] method inside
    /// [`TestRunner::query_visible_state`].
    pub fn true_slot_number(&self) -> SlotNumber {
        SlotNumber::new(self.slot_receipts.len() as u64)
    }

    /// Returns the current visible slot number accessible from the transaction context.
    pub fn visible_slot_number(&self) -> VisibleSlotNumber {
        self.query_visible_state(|state| state.current_visible_slot_number())
    }

    /// A simple helper function to get the balance of a given address in the gas token currency with an [`InfallibleStateAccessor`].
    /// This can be used to check the balance of an address in closures.
    pub fn bank_gas_balance(
        address: &S::Address,
        state: &mut impl InfallibleStateAccessor,
    ) -> Option<Amount> {
        sov_bank::Bank::<S>::default()
            .get_balance_of(address, config_gas_token_id(), state)
            .unwrap_infallible()
    }

    /// A simple helper function to get the the staked balance of a sequencer.
    pub fn get_sequencer_staking_balance(
        sequencer: &<S::Da as DaSpec>::Address,
        state: &mut ApiStateAccessor<S>,
    ) -> Option<Amount> {
        sov_sequencer_registry::SequencerRegistry::<S>::default()
            .get_sender_balance_via_api(sequencer, state)
    }

    /// Returns the slot receipts accumulated by the state runner
    pub fn receipts(&self) -> &Vec<SlotReceipt<S>> {
        &self.slot_receipts
    }

    /// Returns a reference to the nonces used by the state runner
    pub fn nonces(&self) -> &HashMap<<S::CryptoSpec as CryptoSpec>::PublicKey, u64> {
        &self.nonces
    }

    /// Returns a mutable reference to the nonces used by the state runner
    pub fn nonces_mut(&mut self) -> &mut HashMap<<S::CryptoSpec as CryptoSpec>::PublicKey, u64> {
        &mut self.nonces
    }

    /// Returns the state of the rollup at the visible height.
    fn visible_state(&self) -> ApiStateAccessor<S> {
        let stf_state = self.storage_manager.create_prover_storage();
        let mut runtime = RT::default();
        let kernel = runtime.kernel();

        let mut state_checkpoint = StateCheckpoint::<S>::new(stf_state.clone(), &kernel);

        let base_fee_per_gas = RT::default()
            .chain_state()
            .base_fee_per_gas(&mut state_checkpoint).expect("Impossible to get the base fee per gas for the current slot. This is a bug. Please report it");

        ApiStateAccessor::<S>::new_with_price_and_heights(
            &state_checkpoint,
            RT::default().kernel_with_slot_mapping(),
            state_checkpoint.rollup_height_to_access(),
            state_checkpoint.current_visible_slot_number(),
            base_fee_per_gas,
        ).unwrap_or_else(|_| panic!("ApiStateAccessor creation failed but the requested block height {} or visible height {} is accessible. This is a bug. Please report it.", state_checkpoint.rollup_height_to_access(), state_checkpoint.current_visible_slot_number()))
    }

    /// Returns the state of the rollup at the most recent version of the rollup.
    fn state_at_true_height(&self) -> ApiStateAccessor<S> {
        let stf_state = self.storage_manager.create_prover_storage();
        let mut runtime = RT::default();
        let kernel = runtime.kernel();

        let mut state_checkpoint = StateCheckpoint::<S>::new(stf_state.clone(), &kernel);

        let base_fee_per_gas = RT::default()
            .chain_state()
            .base_fee_per_gas(&mut state_checkpoint).expect("Impossible to get the base fee per gas for the current slot. This is a bug. Please report it");

        ApiStateAccessor::<S>::new_with_price_and_slot_number_dangerous(
            &state_checkpoint,
            RT::default().kernel_with_slot_mapping(),
            self.true_slot_number(),
            base_fee_per_gas,
        ).unwrap_or_else(|_| panic!("ApiStateAccessor creation failed but the requested true height {} is accessible. This is a bug. Please report it.", self.true_slot_number()))
    }

    /// Queries the state of the rollup. Calls the given closure with an [`ApiStateAccessor`] and returns the result.
    /// This method does not commit any changes to the state, it simply queries the state and discards the changes
    /// like what would happen by sending RPC/REST requests.
    ///
    /// ## Note
    /// We are using a closure here to ensure that we are accessing the most recent state of the rollup.
    /// Simply returning the [`ApiStateAccessor`] would not be sufficient because the state may be updated while
    /// the [`ApiStateAccessor`] still exists.
    pub fn query_visible_state<Output>(
        &self,
        query: impl FnOnce(&mut ApiStateAccessor<S>) -> Output,
    ) -> Output {
        query(&mut self.visible_state())
    }

    /// This method queries the state of the rollup at the latest _true_ height.
    ///
    /// ## Note
    /// This method is mostly useful in the `soft-confirmations` context. In that case, the _true_ height may be higher than the
    /// current _visible_ height from the default [`TestRunner::query_visible_state`] method. This is because the execution of
    /// blocks from non-preferred sequencers may be deferred to later.
    pub fn query_state<Output>(
        &self,
        query: impl FnOnce(&mut ApiStateAccessor<S>) -> Output,
    ) -> Output {
        query(&mut self.state_at_true_height())
    }

    /// Queries the state of the rollup at the given height. This is essentially the same thing as [`TestRunner::query_visible_state`]
    /// followed by [`ApiStateAccessor::get_archival_state`].
    pub fn query_state_at_height<Output>(
        &self,
        height: RollupHeight,
        query: impl FnOnce(&mut ApiStateAccessor<S>) -> Output,
    ) -> Option<Output> {
        let mut current_state = self
            .state_at_true_height()
            .get_archival_state(height)
            .ok()?;
        Some(query(&mut current_state))
    }

    /// TODO(@theochap): A temporary solution until `https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/1192` is resolved.
    /// Updates the state of the rollup by committing the changes of the given closure.
    pub fn __apply_to_state(&mut self, query: impl FnOnce(&mut StateCheckpoint<S>)) {
        let stf_state = self.storage_manager.create_prover_storage();

        let mut runtime = RT::default();

        let mut state = StateCheckpoint::<S>::new(stf_state.clone(), &runtime.kernel());

        let mut kernel_state = runtime.kernel().accessor(&mut state);

        // We need to synchronize the chain state with a mock kernel state to have a correct state view.
        runtime.chain_state().synchronize_chain(
            &Default::default(),
            &self.state_root,
            &mut kernel_state,
        );

        kernel_state.update_visible_slot_number(kernel_state.visible_slot_number().advance(1));

        query(&mut state);

        let (reads_writes, _, witness) = state.freeze();

        let (new_state_root, change_set) =
            validate_and_materialize(stf_state, reads_writes, &witness, self.state_root.clone())
                .unwrap();

        self.storage_manager
            .commit_change_set(change_set, new_state_root.clone());
        self.state_root = new_state_root;
        self.synchronize_storage_channel();
    }

    /// Sends the current storage over the [`Self::checkpoint_sender`].
    fn synchronize_storage_channel(&mut self) {
        let storage = self.storage_manager.create_prover_storage();
        self.checkpoint_sender
            .send(StateCheckpoint::new(storage, &RT::default().kernel()))
            .expect("Failed to send storage, the storage channel is closed. This is a bug. Please report it.");
    }

    /// Builds a new test runner and runs genesis.
    pub fn new_with_genesis(
        genesis_config: GenesisParams<<RT as Genesis>::Config>,
        _runtime: RT,
    ) -> Self {
        // Use the runtime to create an STF blueprint
        let stf = StfBlueprint::<S, RT>::new();

        // ----- Setup and run genesis ---------
        let mut storage_manager = Sm::new_in_tempdir();

        let sequencer_da_address =
            <RT as MinimalGenesis<S>>::sequencer_registry_config(&genesis_config.runtime)
                .sequencer_config
                .seq_da_address;

        let stf_state = storage_manager.create_prover_storage();

        let (sender, receiver) = watch::channel(StateCheckpoint::new(
            stf_state.clone(),
            &RT::default().kernel(),
        ));

        let (state_root, change_set) =
            stf.init_chain(&Default::default(), stf_state, genesis_config);

        storage_manager.commit_change_set(change_set, state_root.clone());

        // ----- End genesis ---------

        let config = RunnerConfig {
            sequencer_da_address,
            freeze_time: None,
            axum_addr: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        };

        let mut runner = Self {
            nonces: HashMap::new(),
            slot_receipts: Vec::new(),
            state_root,
            storage_manager,
            stf,
            axum_server: Default::default(),
            checkpoint_sender: sender,
            checkpoint_receiver: receiver,
            config,
        };

        runner.synchronize_storage_channel();

        runner
    }

    fn next_header(&mut self) -> MockBlockHeader {
        let height = self
            .true_slot_number()
            .checked_add(1)
            .expect("Slot number overflow")
            .get();
        if let Some(timestamp) = &self.config.freeze_time {
            MockBlockHeader::new(height, timestamp.clone())
        } else {
            MockBlockHeader::from_height(height)
        }
    }

    fn txs_to_blobs(
        txs: Vec<TransactionType<RT, S>>,
        sequencer: <MockDaSpec as DaSpec>::Address,
        nonces: &mut HashMap<<S::CryptoSpec as CryptoSpec>::PublicKey, u64>,
    ) -> (RelevantBlobs<MockBlob>, RelevantBlobInfo) {
        Self::batches_to_blobs(vec![(BatchType(txs), sequencer)], nonces)
    }

    /// Builds [`RelevantBlobs`] from a list of [`BatchType`]s.
    ///
    /// Note: This should be used with a [`BasicKernel`] implementation.
    pub fn batches_to_blobs(
        batches: Vec<(BatchType<RT, S>, MockAddress)>,
        nonces: &mut HashMap<<S::CryptoSpec as CryptoSpec>::PublicKey, u64>,
    ) -> (RelevantBlobs<MockBlob>, RelevantBlobInfo) {
        let blobs = batches
            .into_iter()
            .map(|(batch, sequencer)| {
                let txns = batch
                    .0
                    .into_iter()
                    .map(|tx| tx.to_serialized_authenticated_tx(nonces))
                    .collect::<Vec<_>>();
                MockBlob::new_with_hash(borsh::to_vec(&txns).unwrap(), sequencer)
            })
            .collect::<Vec<_>>();

        let blobs = RelevantBlobs {
            batch_blobs: blobs,
            proof_blobs: vec![],
        };
        let info = RelevantBlobInfo::from_blobs(&blobs);
        (blobs, info)
    }

    /// Builds [`RelevantBlobs`] from a list of [`SoftConfirmationBlobInfo`]s.
    ///
    /// To be used in soft-confirmation mode, ie with a [`sov_kernels::soft_confirmations::SoftConfirmationsKernel`] implementation.
    pub fn soft_confirmation_batches_to_blobs(
        batches: Vec<SoftConfirmationBlobInfo<RT, S>>,
        nonces: &mut HashMap<<S::CryptoSpec as CryptoSpec>::PublicKey, u64>,
    ) -> RelevantBlobs<MockBlob> {
        let blobs = batches
            .into_iter()
            .map(
                |SoftConfirmationBlobInfo {
                     batch_type: batch,
                     sequencer_address,
                     sequencer_info,
                 }| {
                    let raw_txns = batch
                        .0
                        .into_iter()
                        .map(|tx| tx.to_serialized_authenticated_tx(nonces))
                        .collect::<Vec<_>>();

                    let serialized_batch = match sequencer_info {
                        SequencerInfo::Preferred {
                            slots_to_advance,
                            sequence_number,
                        } => borsh::to_vec(&PreferredBatchData {
                            sequence_number,
                            data: raw_txns.into(),
                            visible_slots_to_advance: NonZero::new(slots_to_advance).unwrap(),
                        })
                        .unwrap(),
                        SequencerInfo::Regular => borsh::to_vec(&raw_txns).unwrap(),
                    };

                    MockBlob::new_with_hash(serialized_batch, sequencer_address)
                },
            )
            .collect::<Vec<_>>();

        RelevantBlobs {
            batch_blobs: blobs,
            proof_blobs: vec![],
        }
    }

    /// Simulates execution of the provided input without committing to the updated state.
    /// This is useful to retrieve non-deterministic outcomes associated with execution such as
    /// dynamic gas prices.
    pub fn simulate<T: Into<SlotInput<RT, S>>>(
        &mut self,
        input: T,
    ) -> (TestApplySlotOutput<RT, S>, RelevantBlobInfo, NoncesMap<S>) {
        self.simulate_with_control_flow(input, ExecutionContext::Node, NoOpControlFlow)
    }

    ///  Simulates execution of the provided input without committing to the updated state,
    ///  using custom `InjectedControlFlow`.
    fn simulate_with_control_flow<T: Into<SlotInput<RT, S>>, CF: InjectedControlFlow<S> + Clone>(
        &mut self,
        input: T,
        execution_context: ExecutionContext,
        cf: CF,
    ) -> (TestApplySlotOutput<RT, S>, RelevantBlobInfo, NoncesMap<S>) {
        let block_header = self.next_header();
        let stf_state = self.storage_manager.create_prover_storage();
        let slot_input: SlotInput<RT, S> = input.into();
        let sequencer = self.config.sequencer_da_address;
        let mut nonces = self.nonces.clone();

        let (mut blobs, blob_info) = match slot_input {
            SlotInput::Transaction(tx) => Self::txs_to_blobs(vec![tx], sequencer, &mut nonces),
            SlotInput::Batch(batch) => Self::txs_to_blobs(batch.0, sequencer, &mut nonces),
            SlotInput::Batches(batches) => {
                let batches = batches
                    .into_iter()
                    .map(|batch| (batch, sequencer))
                    .collect();
                Self::batches_to_blobs(batches, &mut nonces)
            }
            SlotInput::Proof(proof) => {
                let blob = MockBlob::new_with_hash(proof.0, sequencer);
                let blobs = RelevantBlobs {
                    batch_blobs: vec![],
                    proof_blobs: vec![blob],
                };
                let info = RelevantBlobInfo::from_blobs(&blobs);

                (blobs, info)
            }
            SlotInput::Blobs(blobs) => {
                let info = RelevantBlobInfo::from_blobs(&blobs);
                (blobs, info)
            }
        };
        (
            self.stf.apply_slot_with_control_flow(
                self.state_root(),
                stf_state.clone(),
                Default::default(),
                &block_header,
                blobs.as_iters(),
                execution_context,
                cf,
            ),
            blob_info,
            nonces,
        )
    }

    /// Executes the provided input and commits the state updates.
    /// This is useful for executing setup transactions that aren't test cases.
    pub fn execute<T: Into<SlotInput<RT, S>>>(
        &mut self,
        input: T,
    ) -> (SlotReceipt<S>, RelevantBlobInfo) {
        let (result, blob_info, nonces) = self.simulate::<T>(input);
        let (result, slot_receipt) = split_apply_slot_output::<S, RT>(result);

        self.commit_apply_slot_output(result, nonces);

        (slot_receipt, blob_info)
    }

    /// Executes the provided input as sequencer.
    pub fn execute_as_sequencer<T: Into<SlotInput<RT, S>>>(
        &mut self,
        input: T,
    ) -> (SlotReceipt<S>, RelevantBlobInfo) {
        let (result, blob_info, nonces) = self.simulate_with_control_flow::<T, _>(
            input,
            ExecutionContext::Sequencer,
            SeqControlFlow,
        );
        let (result, slot_receipt) = split_apply_slot_output::<S, RT>(result);
        self.commit_apply_slot_output(result, nonces);
        (slot_receipt, blob_info)
    }

    fn commit_apply_slot_output(
        &mut self,
        output: TestApplySlotOutput<RT, S>,
        nonces: NoncesMap<S>,
    ) {
        self.storage_manager
            .commit_change_set(output.change_set, output.state_root.clone());
        self.synchronize_storage_channel();
        self.state_root = output.state_root.clone();
        self.slot_receipts.push(SlotReceipt {
            discarded_blobs: output.discarded_blobs.clone(),
            batch_receipts: output.batch_receipts.clone(),
            proof_receipts: output.proof_receipts.clone(),
            state_root: output.state_root,
        });
        self.nonces = nonces;
    }

    /// Advance the rollup `slots_to_advance` slots without executing
    /// any transactions.
    pub fn advance_slots(&mut self, slots_to_advance: usize) -> &mut Self {
        for _ in 0..slots_to_advance {
            let block_header = self.next_header();
            let stf_state = self.storage_manager.create_prover_storage();
            let mut blobs = RelevantBlobs {
                proof_blobs: vec![],
                batch_blobs: vec![],
            };
            let result = self.stf.apply_slot(
                self.state_root(),
                stf_state.clone(),
                Default::default(),
                &block_header,
                blobs.as_iters(),
                ExecutionContext::Node,
            );
            self.commit_apply_slot_output(result, self.nonces.clone());
        }
        self
    }

    /// Execute a [`TransactionTestCase`] against the current state of the test runtime.
    ///
    /// Under the hood this will execute a slot with a single batch containing a single
    /// transaction.
    pub fn execute_transaction(
        &mut self,
        transaction_test: TransactionTestCase<RT, S>,
    ) -> &mut Self {
        let (result, blob_metadata) = self.execute(transaction_test.input);
        let batch_receipt = result.batch_receipts[0].clone();
        let blob_info = blob_metadata.batch_blobs[0].clone();
        let tx_receipt = batch_receipt.tx_receipts[0].clone();
        let gas_used = get_gas_used(&tx_receipt);
        let gas_price = batch_receipt.inner.gas_price.clone();

        let ctx = TransactionAssertContext::from_receipt::<MockDaSpec>(
            tx_receipt,
            blob_info,
            gas_used.value(&gas_price),
        );
        (transaction_test.assert)(ctx, &mut self.visible_state());
        self
    }

    /// Send a transaction which should be skipped. Asserts that the tx is indeed skipped.
    /// Does not increment the sender nonce.
    pub fn execute_skipped_transaction(
        &mut self,
        mut transaction_test: TransactionTestCase<RT, S>,
    ) -> &mut Self {
        // Wrap the test assertion in an assertion that the tx was skipped
        transaction_test.assert = Box::new(|ctx, state| {
            assert!(
                ctx.tx_receipt.is_skipped(),
                "Transaction was expected to be skipped but was executed"
            );
            (transaction_test.assert)(ctx, state);
        });

        // If we're incrementing a nonce, check which one.
        let pubkey_for_nonce_to_decrement = match &transaction_test.input {
            TransactionType::Plain { key, .. } => Some(key.pub_key()),
            _ => None,
        };
        // Execute the tx and reset the nonce if necessary
        self.execute_transaction(transaction_test);
        if let Some(nonce) = pubkey_for_nonce_to_decrement {
            if let Some(n) = self.nonces.get_mut(&nonce) {
                *n -= 1;
            }
        }

        self
    }

    /// Execute a BatchTestCase against the current state of the runtime.
    ///
    /// Under the hood this will execute a slot with the provided batch.
    pub fn execute_batch(&mut self, batch_test: BatchTestCase<RT, S>) -> &mut Self {
        let (result, _) = self.execute(batch_test.input);
        let ctx = BatchAssertContext {
            sender_da_address: self.config.sequencer_da_address,
            batch_receipt: result.batch_receipts.first().cloned(),
        };
        (batch_test.assert)(ctx, &mut self.visible_state());
        self
    }

    /// Execute a ProofTestCase against the current state of the runtime.
    ///
    /// This will submit a slot containing a single proof blob.
    pub fn execute_proof<M: Module>(&mut self, proof_test: ProofTestCase<S>) -> &mut Self
    where
        RT: EncodeCall<M>,
    {
        let (result, _) = self.execute(proof_test.input);
        let proof_receipt = result.proof_receipts.first().cloned();

        let gas_value_used = if let Some(proof_receipt) = &proof_receipt {
            let gas_used = <S as Spec>::Gas::try_from(proof_receipt.gas_used.clone()).unwrap_or_else(
                |_|
                    panic!(
                        "Impossible to convert gas used {:?} to a gas unit {}. This is a bug - the batch receipt should always contain the correct number of gas dimensions. Please report this bug",
                        proof_receipt.gas_used,
                        std::any::type_name::<S::Gas>()
                    )
            );
            let gas_price = <<S as Spec>::Gas as sov_modules_api::Gas>::Price::try_from(
                proof_receipt.gas_price.iter().map(|raw| Amount::new(*raw)).collect::<Vec<_>>(),
            )
                .unwrap_or_else(
                    |_|
                        panic!(
                            "Impossible to convert gas used {:?} to a gas unit {}. This is a bug - the batch receipt should always contain the correct number of gas dimensions. Please report this bug",
                            proof_receipt.gas_used,
                            std::any::type_name::<<S::Gas as Gas>::Price>()
                        )
                );

            gas_used.value(&gas_price)
        } else {
            Amount::ZERO
        };

        let ctx = ProofAssertContext {
            proof_receipt,
            gas_value_used,
        };
        (proof_test.assert)(ctx, &mut self.visible_state());

        self
    }
}

// Extracts batch and proof receipts from apply slot output.
// Note: taking original errors is done to prevent loss of context of errors. This brake tests.
fn split_apply_slot_output<S: Spec, RT: Runtime<S>>(
    mut result: TestApplySlotOutput<RT, S>,
) -> (TestApplySlotOutput<RT, S>, SlotReceipt<S>) {
    let original_discarded_blobs = std::mem::take(&mut result.discarded_blobs.clone());
    let original_batch_receipts = std::mem::take(&mut result.batch_receipts);
    let original_proof_receipts = std::mem::take(&mut result.proof_receipts.clone());
    result.discarded_blobs = original_discarded_blobs.clone();
    result.batch_receipts = original_batch_receipts.clone();
    result.proof_receipts = original_proof_receipts.clone();
    let slot_receipt = SlotReceipt {
        discarded_blobs: original_discarded_blobs,
        batch_receipts: original_batch_receipts,
        proof_receipts: original_proof_receipts,
        state_root: result.state_root.clone(),
    };

    (result, slot_receipt)
}

impl<RT, S, Sm> TestRunner<RT, S, Sm>
where
    RT: Runtime<S> + MinimalGenesis<S> + HasRestApi<S>,
    S: Spec<Da = MockDaSpec>,
    <S::Storage as Storage>::ChangeSet: Clone,
    Sm: ForklessStorageManager<Storage = S::Storage>,
{
    /// Sets up a REST-api server for frameworks whose runtime that implements [`HasRestApi`].
    /// Returns a client to use to query the server.
    pub async fn setup_rest_api_server(&mut self) -> Client {
        let state = ApiState::build(
            Arc::new(()),
            self.checkpoint_receiver.clone(),
            self.runtime().kernel_with_slot_mapping(),
            None,
        );

        let router = self.runtime().rest_api(state);

        let (axum_addr, axum_server) = {
            let handle = axum_server::Handle::new();
            let axum_addr = self.axum_addr();

            let handle_cloned = handle.clone();
            tokio::spawn(async move {
                axum_server::Server::bind(axum_addr)
                    .handle(handle_cloned)
                    .serve(router.into_make_service())
                    .await
                    .unwrap();
            });

            let axum_addr = handle.listening().await.unwrap();

            (axum_addr, handle)
        };

        self.config.axum_addr = axum_addr;
        self.axum_server = axum_server;

        Client::new()
    }

    /// Returns the OpenAPI spec for the runtime.
    pub fn open_api_spec(&self) -> Option<OpenApi>
    where
        RT: HasRestApi<S>,
    {
        self.runtime().openapi_spec()
    }

    /// Returns a vector of all available paths in the OpenAPI spec associated with the runtime.
    pub fn available_paths(&self) -> Vec<ApiPath> {
        self.open_api_spec()
            .unwrap()
            .paths
            .paths
            .keys()
            .cloned()
            .map(ApiPath)
            .collect()
    }

    /// Returns the combined address and port of the REST API server.
    pub fn axum_addr(&self) -> SocketAddr {
        self.config.axum_addr
    }

    /// Returns the base path for the API.
    pub fn base_path(&self) -> String {
        self.open_api_spec()
            .unwrap()
            .servers
            .unwrap()
            .first()
            .unwrap()
            .url
            .replace("localhost:12346", self.axum_addr().to_string().as_str())
    }

    /// Sends a GET request to the API at the given path.
    ///
    /// ## Note
    /// Paths can be obtained from the openAPI spec (returned by the method [`Self::open_api_spec`]).
    pub async fn query_api(&self, path: &ApiPath, client: &Client) -> reqwest::Response {
        let base_path = self.base_path();

        let url = format!("{base_path}{path}");

        client
            .get(&url)
            .send()
            .await
            .expect("Failed querying router")
    }

    /// Sends a GET request to the API at the given path and returns the deserialized response to the expected format.
    pub async fn query_api_response<T: serde::de::DeserializeOwned>(
        &self,
        path: &ApiPath,
        client: &Client,
    ) -> T {
        self.query_api(path, client)
            .await
            .json()
            .await
            .expect("Impossible to deserialize the response to the expected format")
    }
}

/// Assert that a transaction reverted for the expected reason.
pub fn assert_tx_reverted_with_reason<S: Spec>(result: TxEffect<S>, reason: anyhow::Error) {
    if let TxEffect::Reverted(contents) = result {
        assert_eq!(
            &contents.reason,
            &Error::ModuleError(reason),
            "The transaction should have reverted because instead the outcome was {contents:?}"
        );
    } else {
        panic!(
            "The transaction should have reverted because {reason}, instead the outcome was {result:?}"
        );
    }
}

// This replicate logic from `AsyncBatchResponder`.
// And all modifications mede there shold be replicated.
#[derive(Clone)]
struct SeqControlFlow;

impl<S: Spec> InjectedControlFlow<S> for SeqControlFlow {
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
        let ProvisionalSequencerOutcome {
            execution_status, ..
        } = provisional_outcome;
        let MaybeExecuted::Executed(receipt) = execution_status else {
            return (dirty_scratchpad.revert(), TxControlFlow::IgnoreTx);
        };

        if !receipt.receipt.is_successful() {
            let _ = dirty_scratchpad.tx_changes();
            return (dirty_scratchpad.revert(), TxControlFlow::IgnoreTx);
        }

        (
            dirty_scratchpad.commit(),
            TxControlFlow::ContinueProcessing(receipt),
        )
    }
}
