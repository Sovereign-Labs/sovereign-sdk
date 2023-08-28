#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

mod batch_builder;
mod config;
use std::marker::PhantomData;
use std::net::SocketAddr;

use borsh::{BorshDeserialize, BorshSerialize};
pub use config::RpcConfig;
mod ledger_rpc;
pub use batch_builder::FiFoStrictBatchBuilder;
pub use config::{from_toml_path, RollupConfig, RunnerConfig, StorageConfig};
use jsonrpsee::RpcModule;
pub use ledger_rpc::get_ledger_rpc;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sov_db::ledger_db::{LedgerDB, SlotCommit};
use sov_modules_stf_template::AppTemplate;
use sov_rollup_interface::da::{BlobReaderTrait, DaSpec, DaVerifier};
use sov_rollup_interface::services::da::{DaService, SlotData};
use sov_rollup_interface::stf::{FromConfig, StateTransitionFunction, ZkMode};
use sov_rollup_interface::zk::{ZkSystem, Zkvm, ZkvmGuest, ZkvmHost};
use sov_state::{Storage, ZkStorage};
use tokio::sync::oneshot;
use tracing::{debug, info};

type StateRoot<ST, Vm, DA> = <ST as StateTransitionFunction<
    Vm,
    <<DA as DaService>::Spec as DaSpec>::BlobTransaction,
>>::StateRoot;

type InitialState<ST, Vm, DA> = <ST as StateTransitionFunction<
    Vm,
    <<DA as DaService>::Spec as DaSpec>::BlobTransaction,
>>::InitialState;

/// Verifies a state transition
pub struct StateTransitionVerifier<ST, Da, Vm>
where
    Da: DaVerifier,
    Vm: ZkSystem,
    ST: StateTransitionFunction<Vm, Da::Spec>,
{
    app: ST,
    da_verifier: Da,
    vm_guest: Vm::Guest,
}
impl<ST, Da, Vm> StateTransitionVerifier<ST, Da, Vm>
where
    Da: DaVerifier,
    Vm: ZkSystem,
    ST: StateTransitionFunction<Vm, Da::Spec>,
{
    /// Create a [`StateTransitionVerifier`]
    pub fn new(app: ST, da_verifier: Da, vm_guest: Vm::Guest) -> Self {
        Self {
            app,
            da_verifier,
            vm_guest,
        }
    }

    /// Verify the next block
    pub fn run_block(&mut self) -> Result<(), Da::Error> {
        let mut data: StateTransitionData<ST, Da::Spec, Vm> = self.vm_guest.read_from_host();
        let validity_condition = self.da_verifier.verify_relevant_tx_list(
            &data.da_block_header,
            &data.blobs,
            data.inclusion_proof,
            data.completeness_proof,
        )?;

        let result = self.app.apply_slot(
            data.state_transition_witness,
            &data.da_block_header,
            &validity_condition,
            &mut data.blobs,
        );

        Ok(())
    }
}

/// Combines `DaService` with `StateTransitionFunction` and "runs" the rollup.
pub struct StateTransitionRunner<ST, DA, Vm>
where
    DA: DaService,
    Vm: ZkSystem,
    ST: StateTransitionFunction<Vm, DA::Spec>,
{
    start_height: u64,
    da_service: DA,
    app: ST,
    ledger_db: LedgerDB,
    state_root: ST::StateRoot,
    listen_address: SocketAddr,
    verifier: Option<StateTransitionVerifier<ST, DA::Verifier, Vm>>,
}

impl<ST, DA, Vm, StateRoot> StateTransitionRunner<ST, DA, Vm>
where
    DA: DaService<Error = anyhow::Error> + Clone + Send + Sync + 'static,
    Vm: ZkSystem,
    ST: StateTransitionFunction<Vm, DA::Spec> + FromConfig<ZkMode, Config = StateRoot>,
    StateRoot: Clone + Into<[u8; 32]>,
{
    /// Creates a new `StateTransitionRunner` runner.
    pub fn new(
        runner_config: RunnerConfig,
        da_service: DA,
        ledger_db: LedgerDB,
        mut app: ST,
        should_init_chain: bool,
        genesis_config: ST::InitialState,
        verifier: Option<StateTransitionVerifier<ST, DA::Verifier, Vm>>,
    ) -> Result<Self, anyhow::Error> {
        let rpc_config = runner_config.rpc_config;

        let prev_state_root = {
            // Check if the rollup has previously been initialized
            if should_init_chain {
                info!("No history detected. Initializing chain...");
                let ret_hash = app.init_chain(genesis_config);
                info!("Chain initialization is done.");
                ret_hash
            } else {
                debug!("Chain is already initialized. Skipping initialization.");
                app.get_current_state_root()?
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
            app,
            ledger_db,
            state_root: prev_state_root,
            listen_address,
            verifier,
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
    pub async fn run(&mut self, prover: Option<&Vm::Host>) -> Result<(), anyhow::Error> {
        // FIXME!
        let prover = prover.map(|p| (p, self.verifier.as_ref().unwrap()));

        for height in self.start_height.. {
            debug!("Requesting data for height {}", height,);

            let filtered_block = self.da_service.get_finalized_at(height).await?;
            let (mut blobs, validity_condition) =
                self.da_service.extract_relevant_txs(&filtered_block);

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
                Default::default(),
                filtered_block.header(),
                &validity_condition,
                &mut blobs,
            );
            for receipt in slot_result.batch_receipts {
                data_to_commit.add_batch(receipt);
            }
            let next_state_root: ST::StateRoot = slot_result.state_root;
            if let Some((prover_instance, ref guest)) = prover {
                let (inclusion_proof, completeness_proof) = self
                    .da_service
                    .get_extraction_proof(&filtered_block, &blobs)
                    .await;

                prover_instance.write_to_guest(&next_state_root);
                prover_instance.write_to_guest(filtered_block.header());
                prover_instance.write_to_guest(&inclusion_proof);
                prover_instance.write_to_guest(&completeness_proof);
                prover_instance.write_to_guest(&blobs);
                prover_instance.write_to_guest(slot_result.witness);
            }

            self.ledger_db.commit_slot(data_to_commit)?;
            self.state_root = next_state_root;
        }

        Ok(())
    }
}

#[derive(Serialize, BorshDeserialize, BorshSerialize, Deserialize)]
// Prevent serde from generating spurious trait bounds. The correct serde bounds are already enforced by the
// StateTransitionFunction, DA, and Zkvm traits.
#[serde(bound = "")]
/// Data required to verify a state transition.
pub struct StateTransitionData<ST: StateTransitionFunction<Vm, DA>, DA: DaSpec, Vm>
where
    Vm: ZkSystem,
{
    /// The state root before the state transition
    pub pre_state_root: ST::StateRoot,
    /// The header of the da block that is being processed
    pub da_block_header: DA::BlockHeader,
    /// The proof of inclusion for all blobs
    pub inclusion_proof: DA::InclusionMultiProof,
    /// The proof that the provided set of blobs is complete
    pub completeness_proof: DA::CompletenessProof,
    /// The blobs that are being processed
    pub blobs: Vec<<DA as DaSpec>::BlobTransaction>,
    /// The witness for the state transition
    pub state_transition_witness: ST::Witness,
}
