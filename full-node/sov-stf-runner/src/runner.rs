use std::collections::VecDeque;
use std::net::SocketAddr;

use jsonrpsee::RpcModule;
use sov_db::ledger_db::{LedgerDB, SlotCommit};
use sov_rollup_interface::da::{BlobReaderTrait, BlockHeaderTrait, DaSpec};
use sov_rollup_interface::services::da::{DaService, SlotData};
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_rollup_interface::storage::HierarchicalStorageManager;
use sov_rollup_interface::zk::{StateTransitionData, Zkvm, ZkvmHost};
use tokio::sync::oneshot;
use tracing::{debug, info};

use crate::verifier::StateTransitionVerifier;
use crate::{ProofSubmissionStatus, ProverService, RunnerConfig};

type StateRoot<ST, Vm, Da> = <ST as StateTransitionFunction<Vm, Da>>::StateRoot;
type GenesisParams<ST, Vm, Da> = <ST as StateTransitionFunction<Vm, Da>>::GenesisParams;

/// Combines `DaService` with `StateTransitionFunction` and "runs" the rollup.
pub struct StateTransitionRunner<Stf, Sm, Da, Vm, Ps>
where
    Da: DaService,
    Vm: ZkvmHost,
    Sm: HierarchicalStorageManager<Da::Spec>,
    Stf: StateTransitionFunction<Vm, Da::Spec, Condition = <Da::Spec as DaSpec>::ValidityCondition>,
    Ps: ProverService,
{
    start_height: u64,
    da_service: Da,
    stf: Stf,
    storage_manager: Sm,
    ledger_db: LedgerDB,
    state_root: StateRoot<Stf, Vm, Da::Spec>,
    listen_address: SocketAddr,
    prover_service: Ps,
}

/// Represents the possible modes of execution for a zkVM program
pub enum ProofGenConfig<Stf, Da: DaService, Vm: ZkvmHost>
where
    Stf: StateTransitionFunction<Vm::Guest, Da::Spec>,
{
    /// Skips proving.
    Skip,
    /// The simulator runs the rollup verifier logic without even emulating the zkVM
    Simulate(StateTransitionVerifier<Stf, Da::Verifier, Vm::Guest>),
    /// The executor runs the rollup verification logic in the zkVM, but does not actually
    /// produce a zk proof
    Execute,
    /// The prover runs the rollup verification logic in the zkVM and produces a zk proof
    Prover,
}

/// How [`StateTransitionRunner`] is initialized
pub enum InitVariant<Stf: StateTransitionFunction<Vm, Da>, Vm: Zkvm, Da: DaSpec> {
    /// From give state root
    Initialized(Stf::StateRoot),
    /// From empty state root
    Genesis {
        /// Genesis block header should be finalized at init moment
        block_header: Da::BlockHeader,
        /// Genesis params for Stf::init
        genesis_params: GenesisParams<Stf, Vm, Da>,
    },
}

impl<Stf, Sm, Da, Vm, Ps> StateTransitionRunner<Stf, Sm, Da, Vm, Ps>
where
    Da: DaService<Error = anyhow::Error> + Clone + Send + Sync + 'static,
    Vm: ZkvmHost,
    Sm: HierarchicalStorageManager<Da::Spec>,
    Stf: StateTransitionFunction<
        Vm,
        Da::Spec,
        Condition = <Da::Spec as DaSpec>::ValidityCondition,
        PreState = Sm::NativeStorage,
        ChangeSet = Sm::NativeChangeSet,
    >,

    Ps: ProverService<StateRoot = Stf::StateRoot, Witness = Stf::Witness, DaService = Da>,
{
    /// Creates a new `StateTransitionRunner`.
    ///
    /// If a previous state root is provided, uses that as the starting point
    /// for execution. Otherwise, initializes the chain using the provided
    /// genesis config.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        runner_config: RunnerConfig,
        da_service: Da,
        ledger_db: LedgerDB,
        stf: Stf,
        mut storage_manager: Sm,
        init_variant: InitVariant<Stf, Vm, Da::Spec>,
        prover_service: Ps,
    ) -> Result<Self, anyhow::Error> {
        let rpc_config = runner_config.rpc_config;

        let prev_state_root = match init_variant {
            InitVariant::Initialized(state_root) => {
                debug!("Chain is already initialized. Skipping initialization.");
                state_root
            }
            InitVariant::Genesis {
                block_header,
                genesis_params: params,
            } => {
                info!(
                    "No history detected. Initializing chain on block_header={:?}...",
                    block_header
                );
                let storage = storage_manager.create_storage_on(&block_header)?;
                let (genesis_root, initialized_storage) = stf.init_chain(storage, params);
                storage_manager.save_change_set(&block_header, initialized_storage)?;
                storage_manager.finalize(&block_header)?;
                info!(
                    "Chain initialization is done. Genesis root: 0x{}",
                    hex::encode(genesis_root.as_ref()),
                );
                genesis_root
            }
        };

        let listen_address = SocketAddr::new(rpc_config.bind_host.parse()?, rpc_config.bind_port);

        // Start the main rollup loop
        let item_numbers = ledger_db.get_next_items_numbers();
        let last_slot_processed_before_shutdown = item_numbers.slot_number - 1;
        let start_height = runner_config.start_height + last_slot_processed_before_shutdown;

        Ok(Self {
            start_height,
            da_service,
            stf,
            storage_manager,
            ledger_db,
            state_root: prev_state_root,
            listen_address,
            prover_service,
        })
    }

    /// Starts a RPC server with provided rpc methods.
    pub async fn start_rpc_server(
        &self,
        methods: RpcModule<()>,
        channel: Option<oneshot::Sender<SocketAddr>>,
    ) {
        let listen_address = self.listen_address;
        let _handle = tokio::spawn(async move {
            let server = jsonrpsee::server::ServerBuilder::default()
                .build([listen_address].as_ref())
                .await
                .unwrap();

            let bound_address = server.local_addr().unwrap();
            if let Some(channel) = channel {
                channel.send(bound_address).unwrap();
            }
            info!("Starting RPC server at {} ", &bound_address);

            let _server_handle = server.start(methods);
            futures::future::pending::<()>().await;
        });
    }

    /// Runs the rollup.
    pub async fn run_in_process(&mut self) -> Result<(), anyhow::Error> {
        let mut seen_block_headers: VecDeque<<Da::Spec as DaSpec>::BlockHeader> = VecDeque::new();
        let mut seen_receipts: VecDeque<_> = VecDeque::new();
        let mut height = self.start_height;
        loop {
            debug!("Requesting data for height {}", height);
            let mut filtered_block = self.da_service.get_block_at(height).await?;

            // Checking if reorg happened or not.
            if let Some(prev_block_header) = seen_block_headers.back() {
                if prev_block_header.hash() != filtered_block.header().prev_hash() {
                    tracing::warn!("Block at height={} does not belong in current chain. Chain has forked. Traversing backwards", height);
                    while let Some(seen_block_header) = seen_block_headers.pop_back() {
                        seen_receipts.pop_back();
                        let block = self
                            .da_service
                            .get_block_at(seen_block_header.height())
                            .await?;
                        if block.header().prev_hash() == seen_block_header.prev_hash() {
                            height = seen_block_header.height();
                            filtered_block = block;
                            break;
                        }
                    }
                    tracing::info!("Resuming execution on height={}", height);
                }
            }

            let mut blobs = self.da_service.extract_relevant_blobs(&filtered_block);

            info!(
                "Extracted {} relevant blobs at height {}: {:?}",
                blobs.len(),
                height,
                blobs
                    .iter()
                    .map(|b| format!(
                        "sequencer={} blob_hash=0x{}",
                        b.sender(),
                        hex::encode(b.hash())
                    ))
                    .collect::<Vec<_>>()
            );

            let mut data_to_commit = SlotCommit::new(filtered_block.clone());

            let pre_state = self
                .storage_manager
                .create_storage_on(filtered_block.header())?;
            let slot_result = self.stf.apply_slot(
                // TODO(https://github.com/Sovereign-Labs/sovereign-sdk/issues/1247): incorrect pre-state root in case of re-org
                &self.state_root,
                pre_state,
                Default::default(),
                filtered_block.header(),
                &filtered_block.validity_condition(),
                &mut blobs,
            );

            for receipt in slot_result.batch_receipts {
                data_to_commit.add_batch(receipt);
            }

            let (inclusion_proof, completeness_proof) = self
                .da_service
                .get_extraction_proof(&filtered_block, &blobs)
                .await;

            let transition_data: StateTransitionData<Stf::StateRoot, Stf::Witness, Da::Spec> =
                StateTransitionData {
                    // TODO(https://github.com/Sovereign-Labs/sovereign-sdk/issues/1247): incorrect pre-state root in case of re-org
                    initial_state_root: self.state_root.clone(),
                    final_state_root: slot_result.state_root.clone(),
                    da_block_header: filtered_block.header().clone(),
                    inclusion_proof,
                    completeness_proof,
                    blobs,
                    state_transition_witness: slot_result.witness,
                };

            self.storage_manager
                .save_change_set(filtered_block.header(), slot_result.change_set)?;

            // ----------------
            // Create ZK proof.
            {
                let header_hash = transition_data.da_block_header.hash();
                self.prover_service.submit_witness(transition_data).await;
                // TODO(https://github.com/Sovereign-Labs/sovereign-sdk/issues/1185):
                //   This section will be moved and called upon block finalization once we have fork management ready.
                self.prover_service
                    .prove(header_hash.clone())
                    .await
                    .expect("The proof creation should succeed");

                loop {
                    let status = self
                        .prover_service
                        .send_proof_to_da(header_hash.clone())
                        .await;

                    match status {
                        Ok(ProofSubmissionStatus::Success) => {
                            break;
                        }
                        // TODO(https://github.com/Sovereign-Labs/sovereign-sdk/issues/1185): Add timeout handling.
                        Ok(ProofSubmissionStatus::ProofGenerationInProgress) => {
                            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await
                        }
                        // TODO(https://github.com/Sovereign-Labs/sovereign-sdk/issues/1185): Add handling for DA submission errors.
                        Err(e) => panic!("{:?}", e),
                    }
                }
            }
            let next_state_root = slot_result.state_root;

            seen_receipts.push_back(data_to_commit);

            self.state_root = next_state_root;
            seen_block_headers.push_back(filtered_block.header().clone());
            height += 1;

            // ----------------
            // Finalization. Done after seen block for proper handling of instant finality
            // Can be moved to another thread to improve throughput
            let last_finalized = self.da_service.get_last_finalized_block_header().await?;
            // For safety we finalize blocks one by one
            tracing::info!(
                "Last finalized header height is {}, ",
                last_finalized.height()
            );
            // Checking all seen blocks, in case if there was delay in getting last finalized header.
            while let Some(earliest_seen_header) = seen_block_headers.front() {
                tracing::debug!(
                    "Checking seen header height={}",
                    earliest_seen_header.height()
                );
                if earliest_seen_header.height() <= last_finalized.height() {
                    tracing::debug!(
                        "Finalizing seen header height={}",
                        earliest_seen_header.height()
                    );
                    self.storage_manager.finalize(earliest_seen_header)?;
                    seen_block_headers.pop_front();
                    let receipts = seen_receipts.pop_front().unwrap();
                    self.ledger_db.commit_slot(receipts)?;
                    continue;
                }

                break;
            }
        }
    }

    /// Allows to read current state root
    pub fn get_state_root(&self) -> &Stf::StateRoot {
        &self.state_root
    }
}
