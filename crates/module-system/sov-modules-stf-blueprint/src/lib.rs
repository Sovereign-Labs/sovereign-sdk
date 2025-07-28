#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
mod stf_blueprint;

use sequencer_mode::{registered, unregistered};
use sov_metrics::{save_elapsed, start_timer};
#[cfg(all(feature = "gas-constant-estimation", feature = "native"))]
use sov_modules_api::track_gas_constants_usage;
#[cfg(feature = "native")]
use sov_modules_api::{capabilities::RollupHeight, AccessoryDelta};
use sov_modules_api::{
    BatchSequencerReceipt, GasArray, GasSpec, HexHash, IncrementalBatch, InjectedControlFlow,
    KernelStateAccessor, NoOpControlFlow, SelectedBlob, TransactionReceipt, VersionReader,
};
#[cfg(feature = "native")]
use sov_state::{SlotValue, StateAccesses};
mod proof_processing;
use sov_modules_api::{PrivilegedKernelAccessor, SlotGasMeter};
use sov_rollup_interface::stf::ProofReceipt;
mod sequencer_mode;
use sov_modules_api::{IterableBatchWithId, TxReceiptContents};
#[cfg(feature = "test-utils")]
mod utils;
/// We export the `apply_tx` function to use inside the simulation endpoints.
pub use sequencer_mode::apply_tx;
pub use sequencer_mode::common::{get_gas_used, AuthTxOutput, BatchReceipt, ValidatedAuthOutput};
pub use sequencer_mode::registered::{process_tx_and_reward_prover, PreExecError};
use sov_modules_api::capabilities::{
    BatchFromUnregisteredSequencer, BlobSelector, BlobSelectorOutput, BlockGasInfo, ChainState,
    HasKernel, Kernel, SequencerRemuneration,
};
use sov_modules_api::hooks::BlockHooks;
use sov_modules_api::transaction::TransactionConsumption;
pub use sov_modules_api::{BatchWithId, BlobData, Runtime};
use sov_modules_api::{
    BlobDataWithId, DaSpec, ExecutionContext, Gas, Genesis, Spec, StateCheckpoint,
};
#[cfg(feature = "native")]
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::da::RelevantBlobIters;
use sov_rollup_interface::stf::{ApplySlotOutput, StateTransitionFunction};
#[cfg(feature = "native")]
use sov_state::storage::StateUpdate;
use sov_state::{Storage, StorageProof};
pub use stf_blueprint::StfBlueprint;
use tracing::trace;

#[cfg(feature = "native")]
type MaterializedUpdate<S> = (
    <S as Storage>::Root,
    <S as Storage>::Witness,
    <S as Storage>::ChangeSet,
);

/// The result of applying a transaction to the state.
/// This is the value returned when [`process_tx_and_reward_prover`] succeeds.
/// It contains the new transaction checkpoint, transaction receipt and the amount of gas tokens that the sequencer should be rewarded.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(bound = "S: Spec")]
pub struct ApplyTxResult<S: Spec> {
    /// Gas consumption.
    pub transaction_consumption: TransactionConsumption<S::Gas>,
    /// The transaction receipt.
    pub receipt: TransactionReceipt<S>,
}

/// Genesis parameters for a blueprint
#[derive(Clone)]
pub struct GenesisParams<RuntimeConfig> {
    /// The runtime genesis parameters
    pub runtime: RuntimeConfig,
}

impl<S, RT> StfBlueprint<S, RT>
where
    S: Spec,
    RT: Runtime<S>,
{
    /// Produces the final [`AccessoryDelta`] for the slot without fully computing
    /// the new state root and changeset.
    ///
    /// This is used by the sequencer that only requires the accessory delta thus avoiding
    /// relatively expensive computation on the "hot" mutex locking path inside the sequencer.
    #[cfg(feature = "native")]
    #[tracing::instrument(skip_all, name = "StfBlueprint::materialize_accessory_state")]
    pub fn materialize_accessory_state(
        &self,
        runtime: &mut RT,
        checkpoint: StateCheckpoint<S>,
    ) -> (
        AccessoryDelta<S::Storage>,
        StateAccesses,
        <S::Storage as Storage>::Witness,
    ) {
        let rollup_height = checkpoint.rollup_height_to_access();
        let (accesses, mut accessory_delta, witness) = checkpoint.freeze();
        let next_visible_hash =
            Self::next_visible_root(runtime, &mut accessory_delta, rollup_height);

        tracing::trace_span!("runtime_finalize_hook").in_scope(|| {
            runtime.finalize_hook(&next_visible_hash, &mut accessory_delta);
        });

        (accessory_delta, accesses, witness)
    }

    /// Compute the new state root and change set after running a batch.
    ///  
    /// This method is quite complicated because it invokes the `finalize_hook` using the visible hash that will become available
    /// for the *next* rollup block.
    #[cfg_attr(feature = "bench", sov_modules_api::cycle_tracker)]
    #[cfg(feature = "native")]
    #[tracing::instrument(skip_all, name = "StfBlueprint::materialize_slot")]
    pub fn materialize_slot(
        &self,
        runtime: &mut RT,
        create_rollup_block: bool,
        mut checkpoint: StateCheckpoint<S>,
        prev_state_root: <S::Storage as Storage>::Root,
    ) -> MaterializedUpdate<S::Storage> {
        let rollup_height = checkpoint.rollup_height_to_access();
        let mut changes = None;
        // This is a debug feature to capture the state changes for a given rollup block. This mirrors logic in `RollupBlockExecutor` -
        // if you change this, be sure to modify that as well.
        let capture_state_changes = std::env::var("SOV_DEBUG_STATE_ROOT_COMPUTATION").is_ok();
        if create_rollup_block && capture_state_changes {
            let mut found = checkpoint.changes().changes;
            found.sort_by_key(|((key, _), _)| key.clone());
            changes = Some(found);
        }
        let (next_root_hash, mut state_update, mut accessory_delta, witness, storage) =
            checkpoint.materialize_update(prev_state_root);

        // Special case: at genesis, we save the genesis root to the accessory state here. This ensure's it's available even before
        // the next slot causes `synchronize_chain` to be called.
        if rollup_height == RollupHeight::GENESIS
            && runtime
                .chain_state()
                .genesis_root(&mut accessory_delta)
                .is_none()
        {
            runtime
                .chain_state()
                .save_genesis_root(&mut accessory_delta, &next_root_hash);
        }

        // Run the finalize hook if necessary
        if create_rollup_block {
            if capture_state_changes {
                use sov_state::StateRoot;
                let file_name = format!(
                    "{}-{}.txt",
                    rollup_height,
                    hex::encode(next_root_hash.namespace_root(sov_state::ProvableNamespace::User))
                );
                match std::fs::File::create(&file_name) {
                    Ok(mut out_file) => {
                        for ((key, namespace), value) in changes.expect("State changes must have been captured if capture_state_changes is true") {
                            use std::io::Write;
                            if namespace == sov_state::Namespace::User {
                                if let Err(e) = out_file.write_all(
                                    format!(
                                        "{} => {}\n",
                                        key,
                                        SlotValue::debug_show(value.as_ref())
                                    )
                                    .as_bytes(),
                                ) {
                                    tracing::error!("Error while writing state root to file for debugging {}: {}", &file_name, e);
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!(
                            "Failed to create file for state root debugging {}: {}",
                            &file_name,
                            e
                        );
                    }
                }
            }
            let next_visible_hash =
                Self::next_visible_root(runtime, &mut accessory_delta, rollup_height);

            runtime.finalize_hook(&next_visible_hash, &mut accessory_delta);
        }
        state_update.add_accessory_items(accessory_delta.freeze());
        let change_set = storage.materialize_changes(state_update);
        (next_root_hash, witness, change_set)
    }

    #[cfg_attr(feature = "bench", sov_modules_api::cycle_tracker)]
    #[cfg(not(feature = "native"))]
    fn materialize_slot(
        &self,
        _create_rollup_block: bool,
        checkpoint: StateCheckpoint<S>,
        prev_state_root: <S::Storage as Storage>::Root,
    ) -> (
        <S::Storage as Storage>::Root,
        <S::Storage as Storage>::Witness,
        <S::Storage as Storage>::ChangeSet,
    ) {
        use sov_state::Witness;

        Self::check_state_root_delay();

        let (next_root_hash, state_update, _, witness, storage) =
            checkpoint.materialize_update(prev_state_root);

        let change_set = storage.materialize_changes(state_update);
        assert!(witness.is_empty(), "Non-native execution must completely consume the witness! The prover may be malicious, or this may be a bug.");

        (next_root_hash, witness, change_set)
    }

    fn check_state_root_delay() -> u64 {
        let state_root_delay_blocks =
            sov_modules_api::macros::config_value!("STATE_ROOT_DELAY_BLOCKS");
        if state_root_delay_blocks == 0 {
            tracing::error!("Setting state root delay blocks to 0 is currently unsupported. If you need state roots with no delay, please contact the SDK maintainers.");
            panic!("STATE_ROOT_DELAY_BLOCKS is set to 0.");
        }

        state_root_delay_blocks
    }

    /// Compute the next visible hash.
    ///
    /// We have a special case at genesis, where we need to explicitly fetch the genesis root from the accessory state because
    /// the `synchronize_chain` method (which populates state root information in the accessory state) is not called until after
    /// the genesis invocation of `materialize_slot`.
    #[cfg(feature = "native")]
    fn next_visible_root(
        runtime: &mut RT,
        accessory_delta: &mut AccessoryDelta<S::Storage>,
        rollup_height: RollupHeight,
    ) -> <S::Storage as Storage>::Root {
        if rollup_height.saturating_sub(Self::check_state_root_delay()) == RollupHeight::GENESIS {
            runtime
                .chain_state()
                .genesis_root(accessory_delta).expect("genesis root must be set on first iteration of `materialize_slot`. This is a bug - please report it")
        } else {
            runtime.chain_state().visible_hash_with_accessory_state(rollup_height.saturating_add(1), accessory_delta)
                .unwrap_or_else(|| panic!("next visible hash must be known in advance, but was unable to get it for rollup height {rollup_height}. This is a bug - please report it"))
        }
    }
}

impl<S, RT> StateTransitionFunction<S::InnerZkvm, S::OuterZkvm, S::Da> for StfBlueprint<S, RT>
where
    S: Spec,
    RT: Runtime<S>,
    RT: HasKernel<S>,
{
    type StateRoot = <S::Storage as Storage>::Root;

    type Address = S::Address;

    type GasPrice = <S::Gas as Gas>::Price;

    type GenesisParams = GenesisParams<<RT as Genesis>::Config>;
    type PreState = S::Storage;
    type ChangeSet = <S::Storage as Storage>::ChangeSet;

    type TxReceiptContents = TxReceiptContents<S>;

    type BatchReceiptContents = BatchSequencerReceipt<S>;

    type StorageProof = StorageProof<<S::Storage as Storage>::Proof>;

    type Witness = <S::Storage as Storage>::Witness;

    fn init_chain(
        &self,
        genesis_rollup_header: &<S::Da as DaSpec>::BlockHeader,
        pre_state: Self::PreState,
        params: Self::GenesisParams,
    ) -> (Self::StateRoot, Self::ChangeSet) {
        let mut runtime = RT::default();
        // Sanity checks.
        assert!(<S as GasSpec>::process_tx_pre_exec_checks_gas()
            .dim_is_less_than(&<S as GasSpec>::max_tx_check_costs()), "Gas misconfiguration: PROCESS_TX_PRE_EXEC_GAS must be less than MAX_SEQUENCER_EXEC_GAS_PER_TX");
        let mut state_checkpoint = StateCheckpoint::new(pre_state, &runtime.kernel());

        let mut genesis_accessor =
            state_checkpoint.to_genesis_state_accessor::<RT>(&params.runtime);

        if let Err(e) = runtime.genesis(
            genesis_rollup_header,
            &params.runtime,
            &mut genesis_accessor,
        ) {
            tracing::error!(error = %e, "Runtime initialization must succeed");
            panic!("Runtime initialization must succeed {e}");
        }

        #[cfg(feature = "native")]
        let (genesis_hash, _, change_set) = self.materialize_slot(
            &mut runtime,
            true,
            state_checkpoint,
            <S::Storage as Storage>::PRE_GENESIS_ROOT,
        );
        #[cfg(not(feature = "native"))]
        let (genesis_hash, _, change_set) = self.materialize_slot(
            true,
            state_checkpoint,
            <S::Storage as Storage>::PRE_GENESIS_ROOT,
        );

        (genesis_hash, change_set)
    }

    /// Run a state transition using the STF blueprint.
    ///
    /// ## How it Works
    ///
    /// Ths Sovereign SDK invokes this function exactly once for each block produced on the DA layer. A "slot" is a block on the DA layer.
    /// Reorgs on the underlying DA chain are handled externally by the Sovereign SDK, and it's the job of this function to implement a "pure"
    /// state transition from its inputs to its outputs.
    ///
    /// This *implementation* of `apply_slot` has two key units of transition: a "slot", which causes bookkeeping changes to the rollup state that are not *primarily*
    /// intended to be user facing, and a "rollup block", a state transition in user space which involves processing some batches of transactions. Every single DA layer block
    /// causes a "slot" to be processed, and each slot contains either zero or one "rollup block".
    ///
    /// Since we're buidling "sovereign" rollups which don't rely on external smart contracts, the rollup has to keep track of all the data that appears on the DA layer in order
    /// to enforce censorship resistance. But, we still want sequencers to be able to give out "soft-confirmations" *before* transactions are finalized on the DA layer. This
    /// requires that we have some mechanism to prevent minor changes on the DA layer from impacting the outcome of transactions. We do this by partitioning the state
    /// into two spaces. "Kernel" state contains an exact record of all the DA layer data from the moment it appears on the DA layer, while "User" state contains the
    /// the state created by transactions. During transaction processing, all user state is visible, but access to "kernel" state is restricted to data older than some
    /// "visible" rollup height. This "visible" height is set by the "preferred" sequencer, and corresponds to the height of the latest DA layer block that the preferred
    /// sequencer had seen at the time he built each batch of transactions (assuming the preferred sequencer is honest). For security, we constrain the "visible" height
    /// to be no more than some constant ("DEFERRED_SLOTS_COUNT") behind the "true" slot number.
    ///
    /// ## Divergences Between Native and Non-Native Execution
    ///
    /// The native and non-native execution paths diverge in the `apply_slot` method in only a small handfull of places. These divergences need to be carefully
    /// audited when making changes to this code, because all reads or writes to state must be done in exactly the same order in both execution paths in order to
    /// generate the correct witness.  (Exception: Accessory state may be read or written anywhere in native code without a corresponding access in non-native code).
    ///
    /// - Metrics are not tracked or emitted in non-native mode. (Note: The vast majority of #[cfg] gates in this module are related to metrics tracking)
    /// - Events are not emitted in non-native mode.
    /// - The `FinalizeHook` is not invoked in non-native mode
    /// - The return type of `materialize_slot` is different in native and non-native mode
    #[cfg_attr(
        feature = "native",
        tracing::instrument(
            name = "StfBlueprint::apply_slot",
            skip_all
            fields(context = ?execution_context, da_height = slot_header.height())
        )
    )]
    #[cfg_attr(feature = "bench", sov_modules_api::cycle_tracker(pre_state_root))]
    #[cfg_attr(
        all(feature = "gas-constant-estimation", feature = "native"),
        track_gas_constants_usage(pre_state_root)
    )]
    fn apply_slot(
        &self,
        pre_state_root: &Self::StateRoot,
        pre_state: Self::PreState,
        witness: Self::Witness,
        slot_header: &<S::Da as DaSpec>::BlockHeader,
        relevant_blobs: RelevantBlobIters<&mut [<S::Da as DaSpec>::BlobTransaction]>,
        execution_context: ExecutionContext,
    ) -> ApplySlotOutput<S::InnerZkvm, S::OuterZkvm, S::Da, Self> {
        self.apply_slot_with_control_flow(
            pre_state_root,
            pre_state,
            witness,
            slot_header,
            relevant_blobs,
            execution_context,
            NoOpControlFlow,
        )
    }
}

impl<S, RT> StfBlueprint<S, RT>
where
    S: Spec,
    RT: Runtime<S>,
    RT: HasKernel<S>,
{
    #[cfg_attr(feature = "bench", sov_modules_api::cycle_tracker)]
    fn select_and_validate_blobs<CF: InjectedControlFlow<S> + Clone>(
        &self,
        runtime: &mut RT,
        relevant_blobs: RelevantBlobIters<&mut [<S::Da as DaSpec>::BlobTransaction]>,
        kernel: &mut KernelStateAccessor<S>,
        cf: CF,
    ) -> (
        BlobSelectorOutput<SelectedBlob<S, IterableBatchWithId<S, CF>>>,
        Vec<HexHash>,
    ) {
        runtime
            .blob_selector()
            .get_blobs_for_this_slot(relevant_blobs, kernel, cf)
            .expect("blob selection must succeed, probably serialization failed")
    }
}

impl<S, RT> StfBlueprint<S, RT>
where
    S: Spec,
    RT: Runtime<S>,
    RT: HasKernel<S>,
{
    /// Run a state transition using the STF blueprint.
    // Similar to `apply_slot`, but enables the injection of a custom `InjectedControlFlow`.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_slot_with_control_flow<CF: InjectedControlFlow<S> + Clone>(
        &self,
        pre_state_root: &<S::Storage as Storage>::Root,
        pre_state: S::Storage,
        witness: <S::Storage as Storage>::Witness,
        slot_header: &<S::Da as DaSpec>::BlockHeader,
        relevant_blobs: RelevantBlobIters<&mut [<S::Da as DaSpec>::BlobTransaction]>,
        execution_context: ExecutionContext,
        cf: CF,
    ) -> ApplySlotOutput<S::InnerZkvm, S::OuterZkvm, S::Da, Self> {
        let mut runtime = RT::default();
        // Sanity check that gas limits are set correctly. This is already checked at genesis, but we check again in case
        // Someone modifies the code after genesis.
        assert!(<S as GasSpec>::process_tx_pre_exec_checks_gas()
            .dim_is_less_than(&<S as GasSpec>::max_tx_check_costs()), "Gas misconfiguration: PROCESS_TX_PRE_EXEC_GAS must be less than MAX_SEQUENCER_EXEC_GAS_PER_TX");

        start_timer!(start_slot);

        let mut state = StateCheckpoint::with_witness(pre_state, witness, &runtime.kernel());
        // First, we bootstrap the kernel from the previous state. The
        // `true_slot_number`, will *always* be stale because it's leftover from the
        // previous slot.
        let mut kernel_with_stale_heights = runtime.kernel().accessor(&mut state);

        // `visible_slot_number`, and `rollup_height` may or may not be stale. If we don't produce a rollup block,
        // during this slot, then the visible slot number and rollup height will not progress, so the old values are still accurate.
        let old_true_slot_number = kernel_with_stale_heights.true_slot_number();
        let old_visible_slot_number = kernel_with_stale_heights.visible_slot_number();
        let old_rollup_height = kernel_with_stale_heights.rollup_height_to_access();

        // WARNING: The true slot number gets updated in the
        // `ChainState::synchronize_chain` method. The visible slot number gets
        // updated in the `ChainState::increment_rollup_height` method.
        //
        // Be careful to respect the call order: the `ChainState` hooks MUST
        // be called before the `BlobStorage`'s, which MUST be called before
        // the `Runtime`'s slot hooks.
        runtime.chain_state().synchronize_chain(
            slot_header,
            pre_state_root,
            &mut kernel_with_stale_heights,
        );

        let mut kernel_with_partially_stale_heights = kernel_with_stale_heights;
        assert_ne!(
            kernel_with_partially_stale_heights.true_slot_number(),
            old_true_slot_number,
            "Sanity check failed (the true slot number didn't progress as expected), this is a bug and should be reported."
        );

        tracing::trace!("Selecting blobs");
        let (blob_selector_output, discarded_blobs) = self.select_and_validate_blobs(
            &mut runtime,
            relevant_blobs,
            &mut kernel_with_partially_stale_heights,
            cf,
        );
        tracing::trace!("Done selecting blobs");

        // The blob selector *must* not mutate the visible slot number or rollup height internally. instead, it must return an output
        // indicating whether a rollup block should be created and, if so, what the new visible slot number should be.
        assert_eq!(
            kernel_with_partially_stale_heights.visible_slot_number(),
            old_visible_slot_number,
            "Sanity check failed (the visible slot number progressed when it shouldn't have), this is a bug and should be reported."
        );
        assert_eq!(
            kernel_with_partially_stale_heights.rollup_height_to_access(),
            old_rollup_height,
            "Sanity check failed (the rollup height progressed when it shouldn't have), this is a bug and should be reported."
        );

        if blob_selector_output.creates_rollup_block() {
            let visible_slot_number = kernel_with_partially_stale_heights
                .visible_slot_number()
                .advance(blob_selector_output.visible_slot_number_increase);
            assert!(visible_slot_number.as_true() <= kernel_with_partially_stale_heights.true_slot_number(), "Visible slot number must be less than or equal to the true slot number. This is a bug, please report it.");

            // "Increment rollup height" updates the rollup state to reflect the new rollup block and visible slot numbers and modifies the accessor's cached heights.
            runtime.chain_state().increment_rollup_height(
                &mut kernel_with_partially_stale_heights,
                visible_slot_number,
            );

            // All heights have been updated.
            assert_ne!(
                kernel_with_partially_stale_heights.visible_slot_number(),
                old_visible_slot_number,
                "Sanity check failed (the visible slot number didn't progress as expected), this is a bug and should be reported."
            );
            assert_ne!(
                kernel_with_partially_stale_heights.rollup_height_to_access(),
                old_rollup_height,
                "Sanity check failed (the rollup height didn't progress as expected), this is a bug and should be reported."
            );
        } else {
            // Defensive programming; if we don't create a rollup block, we aren't allowed to execute any blobs.
            // We panic if this invariant is violated, beccause in this case the rollup block hooks will not be executed correctly leading
            // To potentially inconsistent state.
            assert!(
                blob_selector_output.selected_blobs.is_empty(),
                "Sanity check failed: no rollup block was created but blobs were selected for processing. This is a bug and should be reported."
            );
        }

        let mut kernel = kernel_with_partially_stale_heights;
        let new_rollup_height = kernel.rollup_height_to_access();

        // Compute the state root to show to transactions during execution.
        let visible_hash = runtime
            .chain_state()
            .visible_hash_for(new_rollup_height, &mut kernel)
            .expect("The current visible hash should be possible to compute at this point because the chain-state should have synchronized. This is a bug. Please report it.");

        save_elapsed!(blob_selection_time SINCE start_slot);

        let create_rollup_block = blob_selector_output.creates_rollup_block();

        let (total_gas, proof_receipts, batch_receipts, mut state) = self
            .apply_batches_in_user_space(
                &mut runtime,
                blob_selector_output,
                state,
                execution_context,
                visible_hash,
            );

        let mut kernel_state_accessor = runtime.kernel().accessor(&mut state);

        runtime
            .chain_state()
            .finalize_chain_state(&total_gas, &mut kernel_state_accessor);

        let rollup_height = state.rollup_height_to_access();
        let (state_root, witness, change_set) = {
            // We can't use `if cfg!` here because `materialize_slot` returns different types in native and non-native mode.
            // So we structure this code to make it obvious that we're handling both cases.
            #[cfg(not(feature = "native"))]
            {
                self.materialize_slot(create_rollup_block, state, pre_state_root.clone())
            }
            #[cfg(feature = "native")]
            {
                let slot_finalization_start = std::time::Instant::now();
                let visible_slot_number = state.current_visible_slot_number();

                // Note the call to materialize slot mixed in with metrics operations here.
                let (state_root, witness, change_set) = self.materialize_slot(
                    &mut runtime,
                    create_rollup_block,
                    state,
                    pre_state_root.clone(),
                );

                let slot_finalization_time = slot_finalization_start.elapsed();
                sov_metrics::track_metrics(|tracker| {
                    tracker.submit(sov_metrics::SlotProcessingMetrics {
                        blobs_selection_time: blob_selection_time,
                        slot_finalization_time,
                        da_height: slot_header.height(),
                        execution_context,
                        visible_slot_number,
                        gas_used: total_gas.as_ref().to_vec(),
                    });
                });
                (state_root, witness, change_set)
            }
        };

        ApplySlotOutput::<S::InnerZkvm, S::OuterZkvm, S::Da, Self> {
            state_root,
            change_set,
            proof_receipts,
            batch_receipts,
            discarded_blobs,
            witness,
            rollup_height,
        }
    }

    /// Run the provided sequence of batches, updating the user-space rollup state as we go.
    /// Batches can inject control flow, which will be respected by the runner.
    ///
    /// ## DOS and Censorship Resistance
    /// Responsibility for censorship resistance
    /// and DOS protection is *shared* between the blob selector and this method. The blob selector is responsible
    /// for ensuring that the costs of deserializing and (if applicable) storing *blobs* is paid for by someone,
    /// and for ensuring some level of fairness in selection of blobs to pass to the rollup. Specifically, the blob selector
    /// should be careful to ensure that actors other than the preferred sequencer can get their blobs selected for execution.
    ///
    /// This method is responsible for apportioning *execution resources* (i.e. gas) between different actors. It should
    /// ensure that the preferred sequencer cannot use all available block-space in order to censor other actors, and that
    /// all execution is paid for by someone.
    ///
    /// ## Assumptions
    /// This method assumes that the underlying DA layer provides a reasonable degree of fairness in ordering, so that
    /// executing blobs in FIFO order is not significantly worse than for censorship resistance than executing them in
    /// any other order.
    ///
    #[allow(clippy::type_complexity)]
    #[cfg_attr(feature = "bench", sov_modules_api::cycle_tracker(visible_hash))]
    #[cfg_attr(
        all(feature = "gas-constant-estimation", feature = "native"),
        track_gas_constants_usage(visible_hash)
    )]
    #[tracing::instrument(skip_all, fields(context=?execution_context), level = "debug")]
    pub fn apply_batches_in_user_space<B: IncrementalBatch<S>>(
        &self,
        runtime: &mut RT,
        blob_selector_output: BlobSelectorOutput<SelectedBlob<S, B>>,
        mut state: StateCheckpoint<S>,
        execution_context: ExecutionContext,
        visible_hash: <<S as Spec>::Storage as Storage>::Root,
    ) -> (
        <S as Spec>::Gas,
        Vec<
            ProofReceipt<
                <S as Spec>::Address,
                <S as Spec>::Da,
                <<S as Spec>::Storage as Storage>::Root,
                StorageProof<<<S as Spec>::Storage as Storage>::Proof>,
            >,
        >,
        Vec<BatchReceipt<S>>,
        StateCheckpoint<S>,
    ) {
        let creates_rollup_block = blob_selector_output.creates_rollup_block();

        // Note: The gas price should be computed after all the capabilities involving the [`KernelStateAccessor`] to have the
        // most recent version of the visible rollup height.
        let gas_price = runtime.chain_state().base_fee_per_gas(&mut state).expect("The base fee per gas for the current slot should be known at this point! This is a bug. Please report it");
        let block_gas_limit = runtime.chain_state().block_gas_limit(&mut state).expect("The slot gas limit for the current slot should be known at this point! This is a bug. Please report it");

        let preferred_sequencer = runtime
            .sequencer_remuneration()
            .preferred_sequencer(&mut state);

        // The slot gas meter differentiates gas usage between preferred and standard transaction batches/proofs.
        // It ensures that preferred transactions cannot consume the entire slot gas limit, preventing the preferred sequencer
        // from censoring other types of transactions, such as standard transactions or emergency registrations.
        let mut slot_gas_meter =
            SlotGasMeter::<S>::new(block_gas_limit.clone(), preferred_sequencer);

        trace!(
            blob_count = blob_selector_output.selected_blobs.len(),
            "Selected batch(es) for execution in current slot"
        );

        // We run [`SlotHooks::begin_rollup_block_hook`] if the visible height is updated. This is to ensure that we have the
        // following invariant: the `user_space` root only updates when the `visible_slot_height` gets increased.
        // If not enforced, this may break soft-confirmations because it will not be possible to deterministically
        // predict the user space state when executing priority blobs.
        start_timer!(begin_slot_start);
        if creates_rollup_block {
            BlockHooks::begin_rollup_block_hook(runtime, &visible_hash, &mut state);
        }
        save_elapsed!(begin_block_hook_time SINCE begin_slot_start);

        let mut proof_receipts = Vec::new();
        let mut batch_receipts = Vec::new();

        start_timer!(blob_processing_start);

        for (
            blob_idx,
            SelectedBlob {
                blob_data,
                sender,
                reserved_gas_tokens,
            },
        ) in blob_selector_output.selected_blobs.into_iter().enumerate()
        {
            match blob_data {
                BlobDataWithId::Batch(batch) => {
                    start_timer!(start_batch_processing);
                    let batch_id = batch.id();
                    let sequencer_bond = reserved_gas_tokens
                        .expect("Batches from registered sequencers must have reserved gas tokens");
                    let (batch_receipt, next_checkpoint) = registered::apply_batch::<S, RT, B>(
                        runtime,
                        state,
                        &mut slot_gas_meter,
                        batch,
                        blob_idx,
                        &sender,
                        sequencer_bond,
                        &gas_price,
                        execution_context,
                    );

                    // Metrics section
                    #[cfg(feature = "native")]
                    {
                        save_elapsed!(processing_time SINCE start_batch_processing);
                        let transactions_count = batch_receipt.tx_receipts.len();
                        let ignored_transactions_count = batch_receipt.tx_receipts.len();

                        sov_metrics::track_metrics(|tracker| {
                            tracker.submit(sov_metrics::BatchMetrics {
                                processing_time,
                                transactions_count,
                                ignored_transactions_count,
                            });
                        });
                    };

                    batch_receipts.push(batch_receipt.finalize(batch_id.unwrap_or([0u8; 32])));
                    state = next_checkpoint;
                }
                BlobDataWithId::EmergencyRegistration { tx, id } => {
                    let slot_gas = slot_gas_meter.remaining_slot_gas(&sender);
                    assert!(reserved_gas_tokens.is_none(), "Emergency registration transactions come from unknown sequencers, so gas cannot be reserved. This is a bug.");
                    let (batch_receipt, next_checkpoint) = unregistered::apply_batch::<S, RT>(
                        runtime,
                        state,
                        slot_gas,
                        BatchFromUnregisteredSequencer { tx, id },
                        blob_idx,
                        &sender,
                        &gas_price,
                    );

                    let gas_used = &batch_receipt.inner.gas_used;

                    // SAFETY: Within `unregistered::apply_batch`, we always ensure tx gas meter is initialized with less than the remaining gas in the slot gas meter.
                    slot_gas_meter
                        .charge_gas(gas_used, &sender)
                        .expect("The slot gas meter should be able to charge the gas");

                    batch_receipts.push(batch_receipt);
                    state = next_checkpoint;
                }
                BlobDataWithId::Proof {
                    proof,
                    id,
                    sequencer_address,
                } => {
                    let slot_gas = slot_gas_meter.remaining_slot_gas(&sender);
                    let sequencer_bond = reserved_gas_tokens
                        .expect("Proofs always come from registered sequencers and must have reserved gas tokens");
                    let (receipt, next_checkpoint, gas_used) = self.process_proof(
                        runtime,
                        id,
                        slot_gas,
                        &sender,
                        &sequencer_address,
                        sequencer_bond,
                        &gas_price,
                        proof,
                        state,
                    );

                    // SAFETY: Within `process_proof`, we always ensure the pre execution and tx gas meters are initialized with less than the remaining gas in the slot gas meter.
                    slot_gas_meter
                        .charge_gas(&gas_used, &sender)
                        .expect("The slot gas meter should be able to charge the gas");

                    state = next_checkpoint;
                    proof_receipts.push(receipt);
                }
            }
        }

        save_elapsed!(blob_processing_time SINCE blob_processing_start);
        start_timer!(end_slot_hooks_start);

        // Note that we run the end-slot hooks even in non-native mode, which is why this can't
        // be a single "native" block
        if creates_rollup_block {
            BlockHooks::end_rollup_block_hook(runtime, &mut state);
            let mut block_gas_info = BlockGasInfo::new(block_gas_limit, gas_price);
            block_gas_info.update_gas_used(slot_gas_meter.total_gas_used());
            let rollup_height = state.rollup_height_to_access();
            runtime
                .kernel()
                .record_gas_usage(&mut state, block_gas_info, rollup_height);
        }
        save_elapsed!(end_block_hook_time SINCE end_slot_hooks_start);
        #[cfg(feature = "native")]
        {
            let total_gas = slot_gas_meter.total_gas_used().as_ref().to_vec();
            sov_metrics::track_metrics(|tracker| {
                tracker.submit(sov_metrics::UserSpaceSlotProcessingMetrics {
                    begin_block_hook_time,
                    blobs_processing_time: blob_processing_time,
                    visible_slot_number: state.current_visible_slot_number(),
                    execution_context,
                    end_block_hook_time,
                    gas_used: total_gas,
                });
            });
        }

        (
            slot_gas_meter.total_gas_used(),
            proof_receipts,
            batch_receipts,
            state,
        )
    }
}
