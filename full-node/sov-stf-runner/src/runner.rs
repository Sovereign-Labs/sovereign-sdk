use std::net::SocketAddr;

use jsonrpsee::RpcModule;
use sov_db::ledger_db::{LedgerDB, SlotCommit};
use sov_modules_api::SlotData;
use sov_rollup_interface::da::{BlobReaderTrait, DaSpec};
use sov_rollup_interface::services::da::DaService;
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_rollup_interface::zk::ZkvmHost;
use tokio::sync::oneshot;
use tracing::{debug, info};

use crate::verifier::StateTransitionVerifier;
use crate::{RunnerConfig, StateTransitionData};

type StateRoot<ST, Vm, Da> = <ST as StateTransitionFunction<Vm, Da>>::StateRoot;

type InitialState<ST, Vm, Da> = <ST as StateTransitionFunction<Vm, Da>>::InitialState;

/// Combines `DaService` with `StateTransitionFunction` and "runs" the rollup.
pub struct StateTransitionRunner<ST, Da, Vm, V>
where
    Da: DaService,
    Vm: ZkvmHost,
    ST: StateTransitionFunction<Vm, Da::Spec, Condition = <Da::Spec as DaSpec>::ValidityCondition>,
    V: StateTransitionFunction<Vm::Guest, Da::Spec>,
{
    start_height: u64,
    da_service: Da,
    app: ST,
    ledger_db: LedgerDB,
    state_root: StateRoot<ST, Vm, Da::Spec>,
    listen_address: SocketAddr,
    prover: Option<Prover<V, Da, Vm>>,
}

/// Represents the possible modes of execution for a zkvm program
pub enum ProofGenConfig<ST, Da: DaService, Vm: ZkvmHost>
where
    ST: StateTransitionFunction<Vm::Guest, Da::Spec>,
{
    /// The simulator runs the rollup verifier logic without even emulating the zkvm
    Simulate(StateTransitionVerifier<ST, Da::Verifier, Vm::Guest>),
    /// The executor runs the rollup verification logic in the zkvm, but does not actually
    /// produce a zk proof
    Execute,
    /// The prover runs the rollup verification logic in the zkvm and produces a zk proof
    Prover,
}

/// A prover for the demo rollup. Consists of a VM and a config
pub struct Prover<ST, Da: DaService, Vm: ZkvmHost>
where
    ST: StateTransitionFunction<Vm::Guest, Da::Spec>,
{
    /// The Zkvm Host to use
    pub vm: Vm,
    /// The prover configuration
    pub config: ProofGenConfig<ST, Da, Vm>,
}

impl<ST, Da, Vm, V, Root, Witness> StateTransitionRunner<ST, Da, Vm, V>
where
    Da: DaService<Error = anyhow::Error> + Clone + Send + Sync + 'static,
    Vm: ZkvmHost,
    V: StateTransitionFunction<Vm::Guest, Da::Spec, StateRoot = Root, Witness = Witness>,
    ST: StateTransitionFunction<
        Vm,
        Da::Spec,
        StateRoot = Root,
        Condition = <Da::Spec as DaSpec>::ValidityCondition,
        Witness = Witness,
    >,
    Witness: Default,
    Root: Clone + AsRef<[u8]>,
{
    /// Creates a new `StateTransitionRunner`.
    ///
    /// If a previous state root is provided, uses that as the starting point
    /// for execution. Otherwise, initializes the chain using the provided
    /// genesis config.
    pub fn new(
        runner_config: RunnerConfig,
        da_service: Da,
        ledger_db: LedgerDB,
        mut app: ST,
        prev_state_root: Option<Root>,
        genesis_config: InitialState<ST, Vm, Da::Spec>,
        prover: Option<Prover<V, Da, Vm>>,
    ) -> Result<Self, anyhow::Error> {
        let rpc_config = runner_config.rpc_config;

        let prev_state_root = if let Some(prev_state_root) = prev_state_root {
            // Check if the rollup has previously been initialized
            debug!("Chain is already initialized. Skipping initialization.");
            prev_state_root
        } else {
            info!("No history detected. Initializing chain...");
            let genesis_root = app.init_chain(genesis_config);
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
            app,
            ledger_db,
            state_root: prev_state_root,
            listen_address,
            prover,
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

            let _server_handle = server.start(methods).unwrap();
            futures::future::pending::<()>().await;
        });
    }

    /// Runs the rollup.
    pub async fn run_in_process(&mut self) -> Result<(), anyhow::Error> {
        for height in self.start_height.. {
            debug!("Requesting data for height {}", height,);

            let filtered_block = self.da_service.get_finalized_at(height).await?;
            let mut blobs = self.da_service.extract_relevant_txs(&filtered_block);

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

            let slot_result = self.app.apply_slot(
                &self.state_root,
                Default::default(),
                filtered_block.header(),
                &filtered_block.validity_condition(),
                &mut blobs,
            );
            for receipt in slot_result.batch_receipts {
                data_to_commit.add_batch(receipt);
            }
            if let Some(Prover { vm, config }) = self.prover.as_mut() {
                let (inclusion_proof, completeness_proof) = self
                    .da_service
                    .get_extraction_proof(&filtered_block, &blobs)
                    .await;

                let transition_data: StateTransitionData<V, Da::Spec, Vm::Guest> =
                    StateTransitionData {
                        pre_state_root: self.state_root.clone(),
                        da_block_header: filtered_block.header().clone(),
                        inclusion_proof,
                        completeness_proof,
                        blobs,
                        state_transition_witness: slot_result.witness,
                    };
                vm.add_hint(transition_data);

                match config {
                    ProofGenConfig::Simulate(verifier) => {
                        verifier.run_block(vm.simulate_with_hints()).map_err(|e| {
                            anyhow::anyhow!("Guest execution must succeed but failed with {:?}", e)
                        })?;
                    }
                    ProofGenConfig::Execute => vm.run(false)?,
                    ProofGenConfig::Prover => vm.run(true)?,
                }
            }
            let next_state_root = slot_result.state_root;

            self.ledger_db.commit_slot(data_to_commit)?;
            self.state_root = next_state_root;
        }

        Ok(())
    }
}
