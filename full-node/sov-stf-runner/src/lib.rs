#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

mod batch_builder;
mod config;
use std::marker::PhantomData;
use std::net::SocketAddr;

use anyhow::Context;
use borsh::{BorshDeserialize, BorshSerialize};
use celestia::{BlobIteratorWithSender, BlobWithSender};
pub use config::RpcConfig;
mod ledger_rpc;
pub use batch_builder::FiFoStrictBatchBuilder;
pub use config::{from_toml_path, RollupConfig, RunnerConfig, StorageConfig};
use jsonrpsee::RpcModule;
pub use ledger_rpc::get_ledger_rpc;
use serde::{Deserialize, Serialize};
use sov_db::ledger_db::{LedgerDB, SlotCommit};
use sov_rollup_interface::da::{BlobReaderTrait, DaSpec, DaVerifier};
use sov_rollup_interface::services::da::{DaService, SlotData};
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_rollup_interface::zk::{ProofSystem, ZkVerifier, ZkvmGuest, ZkvmHost};
use tokio::sync::oneshot;
use tracing::{debug, info};

/// Verifies a state transition
pub struct StateTransitionVerifier<ST, Da, Zk>
where
    Da: DaVerifier,
    Zk: ZkVerifier,
    ST: StateTransitionFunction<Zk, Da::Spec>,
{
    app: ST,
    da_verifier: Da,
    phantom: PhantomData<Zk>,
}
impl<ST, Da, Zk> StateTransitionVerifier<ST, Da, Zk>
where
    Da: DaVerifier,
    Zk: ZkvmGuest,
    ST: StateTransitionFunction<Zk, Da::Spec>,
{
    /// Create a [`StateTransitionVerifier`]
    pub fn new(app: ST, da_verifier: Da) -> Self {
        Self {
            app,
            da_verifier,
            phantom: Default::default(),
        }
    }

    /// Verify the next block
    pub fn run_block(&mut self, zkvm: Zk) -> Result<ST::StateRoot, Da::Error> {
        let mut data: StateTransitionData<ST, Da::Spec, Zk> = zkvm.read_from_host();
        for blob in data.blobs.iter() {
            println!("blob before validation: {:?}", blob);
        }
        let validity_condition = self.da_verifier.verify_relevant_tx_list(
            &data.da_block_header,
            &data.blobs,
            data.inclusion_proof,
            data.completeness_proof,
        )?;

        for blob in data.blobs.iter() {
            println!("blob after validation: {:?}", blob);
        }

        let result = self.app.apply_slot(
            data.state_transition_witness,
            &data.da_block_header,
            &validity_condition,
            &mut data.blobs,
        );

        Ok(result.state_root)
    }
}

/// Combines `DaService` with `StateTransitionFunction` and "runs" the rollup.
pub struct StateTransitionRunner<ST, DA, Zk, V>
where
    DA: DaService,
    Zk: ZkvmHost,
    ST: StateTransitionFunction<Zk, DA::Spec>,
    V: StateTransitionFunction<Zk::Guest, DA::Spec>,
{
    start_height: u64,
    da_service: DA,
    app: ST,
    ledger_db: LedgerDB,
    state_root: ST::StateRoot,
    listen_address: SocketAddr,
    #[allow(clippy::type_complexity)]
    verifier: Option<(Zk, StateTransitionVerifier<V, DA::Verifier, Zk::Guest>)>,
}

impl<ST, DA, Zk, V, Root, Witness> StateTransitionRunner<ST, DA, Zk, V>
where
    DA: DaService<Error = anyhow::Error> + Clone + Send + Sync + 'static,
    Zk: ZkvmHost,
    ST: StateTransitionFunction<Zk, DA::Spec, StateRoot = Root, Witness = Witness>,
    V: StateTransitionFunction<Zk::Guest, DA::Spec, StateRoot = Root, Witness = Witness>,
    Witness: Default,
    Root: Clone,
{
    /// Creates a new `StateTransitionRunner` runner.
    #[allow(clippy::type_complexity)]
    pub fn new(
        runner_config: RunnerConfig,
        da_service: DA,
        ledger_db: LedgerDB,
        mut app: ST,
        should_init_chain: bool,
        genesis_config: ST::InitialState,
        verifier: Option<(Zk, StateTransitionVerifier<V, DA::Verifier, Zk::Guest>)>,
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
    pub async fn run(&mut self) -> Result<(), anyhow::Error> {
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
            if let Some((host, verifier)) = self.verifier.as_mut() {
                let (inclusion_proof, completeness_proof) = self
                    .da_service
                    .get_extraction_proof(&filtered_block, &blobs)
                    .await;

                let transition_data: StateTransitionData<V, DA::Spec, Zk::Guest> =
                    StateTransitionData {
                        pre_state_root: self.state_root.clone(),
                        da_block_header: filtered_block.header().clone(),
                        inclusion_proof,
                        completeness_proof,
                        blobs,
                        state_transition_witness: slot_result.witness,
                    };
                host.add_hint(transition_data);

                verifier
                    .run_block(host.simulate_with_hints())
                    .map_err(|e| {
                        anyhow::anyhow!("Guest execution must succeed but failed with {:?}", e)
                    })?;
            }

            self.ledger_db.commit_slot(data_to_commit)?;
            self.state_root = slot_result.state_root;
        }

        Ok(())
    }
}

#[derive(Serialize, BorshDeserialize, BorshSerialize, Deserialize)]
// Prevent serde from generating spurious trait bounds. The correct serde bounds are already enforced by the
// StateTransitionFunction, DA, and Zkvm traits.
#[serde(bound = "")]
/// Data required to verify a state transition.
pub struct StateTransitionData<ST: StateTransitionFunction<Zk, DA>, DA: DaSpec, Zk>
where
    Zk: ZkVerifier,
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
