#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

mod batch_builder;
mod config;
pub use config::RpcConfig;
mod runner_config;
use std::net::SocketAddr;
mod ledger_rpc;
pub use batch_builder::FiFoStrictBatchBuilder;
pub use config::RollupConfig;
use jsonrpsee::RpcModule;
pub use ledger_rpc::get_ledger_rpc;
pub use runner_config::{from_toml_path, Config, StorageConfig};
use sov_db::ledger_db::{LedgerDB, SlotCommit};
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::services::da::DaService;
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_rollup_interface::zk::Zkvm;
use tracing::{debug, info};

type StateRoot<ST, Vm, DA> = <ST as StateTransitionFunction<
    Vm,
    <<DA as DaService>::Spec as DaSpec>::BlobTransaction,
>>::StateRoot;

type InitialState<ST, Vm, DA> = <ST as StateTransitionFunction<
    Vm,
    <<DA as DaService>::Spec as DaSpec>::BlobTransaction,
>>::InitialState;

/// Combines `DaService` with `StateTransitionFunction` and "runs" the rollup.
pub struct StateTransitionRunner<ST, DA, Vm>
where
    DA: DaService,
    Vm: Zkvm,
    ST: StateTransitionFunction<Vm, <<DA as DaService>::Spec as DaSpec>::BlobTransaction>,
{
    start_height: u64,
    da_service: DA,
    app: ST,
    ledger_db: LedgerDB,
    state_root: StateRoot<ST, Vm, DA>,
    listen_address: SocketAddr,
}

impl<ST, DA, Vm> StateTransitionRunner<ST, DA, Vm>
where
    DA: DaService<Error = anyhow::Error> + Clone + Send + Sync + 'static,
    Vm: Zkvm,
    ST: StateTransitionFunction<Vm, <<DA as DaService>::Spec as DaSpec>::BlobTransaction>,
{
    /// Creates a new `StateTransitionRunner` runner.
    pub fn new(
        rollup_config: RollupConfig,
        da_service: DA,
        ledger_db: LedgerDB,
        mut app: ST,
        should_init_chain: bool,
        genesis_config: InitialState<ST, Vm, DA>,
    ) -> Result<Self, anyhow::Error> {
        let rpc_config = rollup_config.rpc_config;

        let prev_state_root = {
            // Check if the rollup has previously been initialized
            if should_init_chain {
                info!("No history detected. Initializing chain...");
                app.init_chain(genesis_config);
                info!("Chain initialization is done.");
            } else {
                debug!("Chain is already initialized. Skipping initialization.");
            }

            let res = app.apply_slot(Default::default(), []);
            // HACK: Tell the rollup that you're running an empty DA layer block so that it will return the latest state root.
            // This will be removed shortly.
            res.state_root
        };

        let listen_address = SocketAddr::new(rpc_config.bind_host.parse()?, rpc_config.bind_port);

        // Start the main rollup loop
        let item_numbers = ledger_db.get_next_items_numbers();
        let last_slot_processed_before_shutdown = item_numbers.slot_number - 1;
        let start_height = rollup_config.start_height + last_slot_processed_before_shutdown;

        Ok(Self {
            start_height,
            da_service,
            app,
            ledger_db,
            state_root: prev_state_root,
            listen_address,
        })
    }

    /// Starts an rpc server with provided rpc methods.
    pub async fn start_rpc_server(&self, methods: RpcModule<()>) {
        let listen_address = self.listen_address;
        let _handle = tokio::spawn(async move {
            let server = jsonrpsee::server::ServerBuilder::default()
                .build([listen_address].as_ref())
                .await
                .unwrap();

            info!("Starting RPC server at {} ", server.local_addr().unwrap());
            let _server_handle = server.start(methods).unwrap();
            futures::future::pending::<()>().await;
        });
    }

    /// Runs the rollup.
    pub async fn run(&mut self) -> Result<(), anyhow::Error> {
        for height in self.start_height.. {
            info!("Requesting data for height {}", height,);

            let filtered_block = self.da_service.get_finalized_at(height).await?;

            let mut blobs = self.da_service.extract_relevant_txs(&filtered_block);

            info!(
                "Extracted {} relevant blobs at height {}",
                blobs.len(),
                height
            );

            let mut data_to_commit = SlotCommit::new(filtered_block.clone());

            let slot_result = self.app.apply_slot(Default::default(), &mut blobs);
            for receipt in slot_result.batch_receipts {
                data_to_commit.add_batch(receipt);
            }
            let next_state_root = slot_result.state_root;

            self.ledger_db.commit_slot(data_to_commit)?;
            self.state_root = next_state_root;
        }

        Ok(())
    }
}
