use std::net::SocketAddr;

use jsonrpsee::RpcModule;
use sov_db::ledger_db::{LedgerDB, SlotCommit};
use sov_modules_api::SlotData;
use sov_rollup_interface::da::{BlobReaderTrait, DaSpec};
use sov_rollup_interface::services::da::DaService;
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_rollup_interface::zk::Zkvm;
use tokio::sync::oneshot;
use tracing::{debug, info};

use crate::RunnerConfig;

type StateRoot<ST, Vm, Da> = <ST as StateTransitionFunction<Vm, Da>>::StateRoot;

type InitialState<ST, Vm, Da> = <ST as StateTransitionFunction<Vm, Da>>::InitialState;

/// Combines `DaService` with `StateTransitionFunction` and "runs" the rollup.
pub struct StateTransitionRunner<ST, Da, Vm>
where
    Da: DaService,
    Vm: Zkvm,
    ST: StateTransitionFunction<Vm, Da::Spec, Condition = <Da::Spec as DaSpec>::ValidityCondition>,
{
    start_height: u64,
    da_service: Da,
    app: ST,
    ledger_db: LedgerDB,
    state_root: StateRoot<ST, Vm, Da::Spec>,
    listen_address: SocketAddr,
}

impl<ST, Da, Vm> StateTransitionRunner<ST, Da, Vm>
where
    Da: DaService<Error = anyhow::Error> + Clone + Send + Sync + 'static,
    Vm: Zkvm,
    ST: StateTransitionFunction<Vm, Da::Spec, Condition = <Da::Spec as DaSpec>::ValidityCondition>,
{
    /// Creates a new `StateTransitionRunner` runner.
    pub fn new(
        runner_config: RunnerConfig,
        da_service: Da,
        ledger_db: LedgerDB,
        mut app: ST,
        should_init_chain: bool,
        genesis_config: InitialState<ST, Vm, Da::Spec>,
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
                Default::default(),
                filtered_block.header(),
                &filtered_block.validity_condition(),
                &mut blobs,
            );
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
