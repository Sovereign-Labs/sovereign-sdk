use sov_db::ledger_db::LedgerDB;
use sov_mock_da::{
    MockAddress, MockBlockHeader, MockDaConfig, MockDaService, MockDaSpec, MockDaVerifier,
    MockValidityCond,
};
use sov_mock_zkvm::MockZkvm;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::storage::HierarchicalStorageManager;
use sov_stf_runner::mock::MockStf;
use sov_stf_runner::{
    InitVariant, ParallelProverService, RollupConfig, RollupProverConfig, RpcConfig, RunnerConfig,
    StateTransitionRunner, StorageConfig,
};

struct MockStorageManager<Da: DaSpec> {
    phantom_data: std::marker::PhantomData<Da>,
}

impl<Da: DaSpec> Default for MockStorageManager<Da> {
    fn default() -> Self {
        Self {
            phantom_data: std::marker::PhantomData,
        }
    }
}

impl<Da: DaSpec> HierarchicalStorageManager<Da> for MockStorageManager<Da> {
    type NativeStorage = ();
    type NativeChangeSet = ();

    fn create_storage_on(
        &mut self,
        _block_header: &Da::BlockHeader,
    ) -> anyhow::Result<Self::NativeStorage> {
        Ok(())
    }

    fn create_finalized_storage(&mut self) -> anyhow::Result<Self::NativeStorage> {
        Ok(())
    }

    fn save_change_set(
        &mut self,
        _block_header: &Da::BlockHeader,
        _change_set: Self::NativeChangeSet,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn finalize(&mut self, _block_header: &Da::BlockHeader) -> anyhow::Result<()> {
        Ok(())
    }
}

type Stf = MockStf<MockValidityCond>;
#[tokio::test]
async fn init_and_restart() {
    let tmpdir = tempfile::tempdir().unwrap();
    let address = MockAddress::new([11u8; 32]);
    let rollup_config = RollupConfig::<MockDaConfig> {
        storage: StorageConfig {
            path: tmpdir.path().to_path_buf(),
        },
        runner: RunnerConfig {
            start_height: 1,
            rpc_config: RpcConfig {
                bind_host: "127.0.0.1".to_string(),
                bind_port: 0,
            },
        },
        da: MockDaConfig {
            sender_address: address,
        },
    };

    let da_service = MockDaService::new(address);

    let ledger_db = LedgerDB::with_path(tmpdir.path()).unwrap();

    let stf = Stf::default();

    let storage_manager = MockStorageManager::<MockDaSpec>::default();

    let init_variant: InitVariant<MockStf<MockValidityCond>, MockZkvm, MockDaSpec> =
        InitVariant::Genesis {
            genesis_block_header: MockBlockHeader::from_height(0),
        };

    let genesis_config = ();

    let vm = MockZkvm::default();
    let verifier = MockDaVerifier::default();

    let prover_config = RollupProverConfig::Prove;

    let prover_service =
        ParallelProverService::new(vm, stf.clone(), verifier, prover_config, (), 1);

    // TODO: Extend test, probably with different STF
    let _runner = StateTransitionRunner::new(
        rollup_config.runner,
        da_service,
        ledger_db,
        stf,
        storage_manager,
        init_variant,
        genesis_config,
        prover_service,
    )
    .unwrap();
}
