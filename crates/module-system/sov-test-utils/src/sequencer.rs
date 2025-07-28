use std::net::SocketAddr;
use std::num::NonZero;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use sov_api_spec::Client;
use sov_db::ledger_db::LedgerDb;
use sov_db::schema::SchemaBatch;
use sov_db::storage_manager::NativeStorageManager;
use sov_mock_da::storable::service::StorableMockDaService;
use sov_mock_da::{MockAddress, MockBlock, MockDaSpec};
use sov_modules_api::{DaSyncState, Runtime, SlotData, Spec, SyncStatus};
use sov_modules_stf_blueprint::GenesisParams;
use sov_paymaster::{PaymasterConfig, SafeVec};
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_rollup_interface::storage::HierarchicalStorageManager;
use sov_rollup_interface::StateUpdateInfo;
use sov_sequencer::standard::{StdSequencer, StdSequencerConfig};
pub use sov_sequencer::test_stateless::TestStatelessSequencer;
use sov_sequencer::{SequencerApis, SequencerConfig};
use sov_state::{DefaultStorageSpec, ProverStorage};
use sov_stf_runner::query_state_update_info;
use sov_value_setter::ValueSetterConfig;
use tempfile::TempDir;
use tokio::sync::watch;

use crate::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use crate::runtime::{GenesisConfig, TestOptimisticRuntime};
use crate::{
    TestHasher, TestPrivateKey, TestSpec, TestStfBlueprint, TEST_MAX_BATCH_SIZE,
    TEST_MAX_CONCURRENT_BLOBS,
};

/// A `struct` that contains a sequencer and a copy of its running Axum
/// server, for use in tests. See [`TestSequencerSetup::new`] and
/// [`TestSequencerSetup::with_real_sequencer`].
pub struct TestSequencerSetup<Rt: Runtime<TestSpec>> {
    _dir: TempDir,
    /// The [`SequencerConfig`] used in this test.
    pub config: SequencerConfig<<TestSpec as Spec>::Address, StdSequencerConfig>,
    /// The DA service used by the sequencer.
    pub da_service: StorableMockDaService,
    /// What was passed to Sequencer::create.
    pub state_update_receiver: watch::Receiver<StateUpdateInfo<<TestSpec as Spec>::Storage>>,
    // Keep a reference to the state update sender used to create the sequencer
    // so it doesn't go out of scope and close the channel immediately.
    _state_update_sender: watch::Sender<StateUpdateInfo<<TestSpec as Spec>::Storage>>,
    /// The sequencer used in the test.
    pub sequencer: Arc<StdSequencer<TestSpec, Rt, StorableMockDaService>>,
    /// The admin private key used to create an external user account for transaction handling.
    pub admin_private_key: TestPrivateKey,
    /// The Axum server handle used to start the Axum server.
    pub axum_server_handle: axum_server::Handle,
    /// The Axum server address.
    pub axum_addr: SocketAddr,
    /// Handler for shutdown of sequencer
    pub shutdown_sender: watch::Sender<()>,
}

impl<Rt: Runtime<TestSpec>> Drop for TestSequencerSetup<Rt> {
    fn drop(&mut self) {
        // Error means that senders are already shut down.
        let _ = self.shutdown_sender.send(());
        self.axum_server_handle.shutdown();
    }
}

impl<Rt: Runtime<TestSpec>> TestSequencerSetup<Rt> {
    /// Like [`TestSequencerSetup::new`], but with a custom [`NativeStorageManager`].
    pub async fn with_storage_manager(
        dir: TempDir,
        da_service: StorableMockDaService,
        sequencer_config: StdSequencerConfig,
        register_admin: bool,
        mut storage_manager: NativeStorageManager<
            MockDaSpec,
            ProverStorage<DefaultStorageSpec<TestHasher>>,
        >,
    ) -> anyhow::Result<Self> {
        // Generate a genesis config, then overwrite the attester key/address with ones that
        // we know. We leave the other values untouched.
        let genesis_config =
            HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);

        let admin = genesis_config.additional_accounts()[0].clone();

        let value_setter_config = ValueSetterConfig {
            admin: admin.address(),
        };
        let paymaster_config = PaymasterConfig {
            payers: SafeVec::new(),
        };

        let runtime = TestOptimisticRuntime::<TestSpec>::default();

        // Run genesis registering the attester and sequencer we've generated.
        let genesis_config = GenesisConfig::from_minimal_config(
            genesis_config.into(),
            value_setter_config,
            paymaster_config,
        );
        let sequencer_rollup_address = genesis_config
            .sequencer_registry
            .sequencer_config
            .seq_rollup_address;

        let params = GenesisParams {
            runtime: genesis_config,
        };

        let stf = TestStfBlueprint::with_runtime(runtime);

        let genesis_block = MockBlock::default();
        let (stf_state, _ledger_state) =
            storage_manager.create_state_for(genesis_block.header())?;

        let (_genesis_root, stf_state) = stf.init_chain(&Default::default(), stf_state, params);
        storage_manager.save_change_set(genesis_block.header(), stf_state, SchemaBatch::new())?;
        storage_manager.finalize(&genesis_block.header)?;
        let (stf_state, ledger_state) =
            storage_manager.create_state_after(genesis_block.header())?;
        let ledger_db = LedgerDb::with_reader(ledger_state.clone())?;
        let api_ledger_db = LedgerDb::with_shared_notifications(&ledger_db);

        let (sync_status_sender, _) = watch::channel(SyncStatus::Syncing {
            synced_da_height: 0,
            target_da_height: 0,
        });

        let da_sync_state = Arc::new(DaSyncState {
            synced_da_height: AtomicU64::new(0),
            target_da_height: AtomicU64::new(0),
            sync_status_sender,
        });
        let admin_addresses = if register_admin {
            vec![admin.address()]
        } else {
            vec![]
        };

        let state_update_info = query_state_update_info(&ledger_db, stf_state).await?;

        let (state_update_sender, state_update_receiver) = watch::channel(state_update_info);
        let (shutdown_sender, mut shutdown_receiver) = watch::channel(());
        shutdown_receiver.mark_unchanged();

        let config = SequencerConfig {
            rollup_address: sequencer_rollup_address,
            admin_addresses,
            automatic_batch_production: true,
            max_allowed_node_distance_behind: 10,
            dropped_tx_ttl_secs: 0,
            sequencer_kind_config: sequencer_config,
            max_batch_size_bytes: TEST_MAX_BATCH_SIZE,
            max_concurrent_blobs: TEST_MAX_CONCURRENT_BLOBS,
            blob_processing_timeout_secs: 60,
        };

        let (sequencer, _) = StdSequencer::<TestSpec, Rt, StorableMockDaService>::create(
            da_service.clone(),
            state_update_receiver.clone(),
            da_sync_state,
            dir.path(),
            &config,
            ledger_db,
            api_ledger_db,
            shutdown_sender.clone(),
        )
        .await?;

        let (axum_addr, sequencer_axum_server) = {
            let router = SequencerApis::rest_api_server(sequencer.clone(), shutdown_receiver);
            let handle = axum_server::Handle::new();

            let handle1 = handle.clone();
            tokio::spawn(async move {
                axum_server::Server::bind(SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, 0)))
                    .handle(handle1)
                    .serve(router.into_make_service())
                    .await
                    .unwrap();
            });

            (handle.listening().await.unwrap(), handle)
        };

        Ok(Self {
            _dir: dir,
            state_update_receiver,
            _state_update_sender: state_update_sender,
            config,
            da_service,
            sequencer,
            admin_private_key: admin.private_key,
            axum_server_handle: sequencer_axum_server,
            axum_addr,
            shutdown_sender,
        })
    }

    /// Instantiates a new sequencer with a [`TestOptimisticRuntime`] and an empty
    /// [`StorableMockDaService`].
    ///
    /// The RPC and Axum servers for the newly generated sequencer are created
    /// on the fly, and their handles are stored inside a [`TestSequencerSetup`].
    /// This results in the automatic shutdown of the servers when the
    /// [`TestSequencerSetup`] is dropped.
    pub async fn new(
        dir: TempDir,
        da_service: StorableMockDaService,
        sequencer_config: StdSequencerConfig,
        register_admin: bool,
    ) -> anyhow::Result<Self> {
        let storage_manager = NativeStorageManager::<
            MockDaSpec,
            ProverStorage<DefaultStorageSpec<TestHasher>>,
        >::new(dir.path())?;

        Self::with_storage_manager(
            dir,
            da_service,
            sequencer_config,
            register_admin,
            storage_manager,
        )
        .await
    }

    /// Returns a [`Client`] REST handler for the sequencer.
    pub fn client(&self) -> Client {
        Client::new(&format!("http://{}", self.axum_addr))
    }
}

impl<Rt: Runtime<TestSpec>> TestSequencerSetup<Rt> {
    /// Like [`TestSequencerSetup::with_real_sequencer`], but allows to
    /// specify the maximum number of transactions in the mempool before
    /// eviction.
    pub async fn with_real_sequencer_and_mempool_max_txs_count(
        mempool_max_txs_count: NonZero<usize>,
    ) -> anyhow::Result<Self> {
        let dir = tempfile::tempdir()?;

        TestSequencerSetup::<Rt>::new(
            dir,
            StorableMockDaService::new_in_memory(MockAddress::new([172; 32]), 0).await,
            StdSequencerConfig {
                mempool_max_txs_count: Some(mempool_max_txs_count),
                max_batch_size_bytes: None,
            },
            true,
        )
        .await
    }

    /// Creates a new [`TestSequencerSetup`]. Instantiates a new [`TestOptimisticRuntime`], [`NativeStorageManager`], executes genesis
    /// and then builds a new [`StdSequencer`]. Instantiates an Axum server in a separate thread.
    pub async fn with_real_sequencer() -> anyhow::Result<Self> {
        Self::with_real_sequencer_and_mempool_max_txs_count(NonZero::new(usize::MAX).unwrap()).await
    }
}
