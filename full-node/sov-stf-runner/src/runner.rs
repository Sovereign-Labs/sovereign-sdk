use std::net::SocketAddr;
use std::time::Duration;

use jsonrpsee::RpcModule;
use sov_db::ledger_db::{LedgerDB, SlotCommit};
use sov_rollup_interface::da::{BlobReaderTrait, BlockHeaderTrait, DaSpec};
use sov_rollup_interface::services::da::{DaService, SlotData};
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_rollup_interface::storage::StorageManager;
use sov_rollup_interface::zk::ZkvmHost;
use tokio::sync::oneshot;
use tracing::{debug, info};

use crate::verifier::StateTransitionVerifier;
use crate::{ProofSubmissionStatus, ProverService, RunnerConfig, StateTransitionData};
type StateRoot<ST, Vm, Da> = <ST as StateTransitionFunction<Vm, Da>>::StateRoot;
type InitialState<ST, Vm, Da> = <ST as StateTransitionFunction<Vm, Da>>::GenesisParams;

/// Combines `DaService` with `StateTransitionFunction` and "runs" the rollup.
pub struct StateTransitionRunner<Stf, Sm, Da, Vm, Ps>
where
    Da: DaService,
    Vm: ZkvmHost,
    Sm: StorageManager,
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

impl<Stf, Sm, Da, Vm, Ps> StateTransitionRunner<Stf, Sm, Da, Vm, Ps>
where
    Da: DaService<Error = anyhow::Error> + Clone + Send + Sync + 'static,
    Vm: ZkvmHost,
    Sm: StorageManager,
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
        storage_manager: Sm,
        prev_state_root: Option<StateRoot<Stf, Vm, Da::Spec>>,
        genesis_config: InitialState<Stf, Vm, Da::Spec>,
        prover_service: Ps,
    ) -> Result<Self, anyhow::Error> {
        let rpc_config = runner_config.rpc_config;

        let prev_state_root = if let Some(prev_state_root) = prev_state_root {
            // Check if the rollup has previously been initialized
            debug!("Chain is already initialized. Skipping initialization.");
            prev_state_root
        } else {
            info!("No history detected. Initializing chain...");
            let genesis_state = storage_manager.get_native_storage();
            let (genesis_root, _) = stf.init_chain(genesis_state, genesis_config);
            info!(
                "Chain initialization is done. Genesis root: 0x{}",
                hex::encode(genesis_root.as_ref())
            );
            genesis_root
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
        for height in self.start_height.. {
            debug!("Requesting data for height {}", height,);

            loop {
                match self.da_service.get_last_finalized_block_header().await {
                    Ok(header) => {
                        tracing::trace!("Last finalized height={}", header.height());
                        if header.height() >= height {
                            break;
                        }
                    }
                    Err(err) => {
                        tracing::info!("Error receiving last finalized block header: {:?}", err);
                    }
                }

                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            // At this point we sure that the block request is finalized

            // Assumes we are on chains with instant finality
            let filtered_block = self.da_service.get_block_at(height).await?;

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

            let pre_state = self.storage_manager.get_native_storage();
            let slot_result = self.stf.apply_slot(
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
                    pre_state_root: self.state_root.clone(),
                    da_block_header: filtered_block.header().clone(),
                    inclusion_proof,
                    completeness_proof,
                    blobs,
                    state_transition_witness: slot_result.witness,
                };

            // Create ZKP proof.
            {
                let header_hash = transition_data.da_block_header.hash();
                self.prover_service.submit_witness(transition_data).await;
                // TODO(#1185): This section will be moved and called upon block finalization once we have fork management ready.
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
                        // TODO(#1185): Add timeout handling.
                        Ok(ProofSubmissionStatus::ProofGenerationInProgress) => {
                            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await
                        }
                        // TODO(#1185): Add handling for DA submission errors.
                        Err(e) => panic!("{:?}", e),
                    }
                }
            }
            let next_state_root = slot_result.state_root;

            self.ledger_db.commit_slot(data_to_commit)?;
            self.state_root = next_state_root;
        }

        Ok(())
    }
}
