//! Shared helper for spinning up an external mock DA service over RPC.
//!
//! Used by integration tests that need a DA layer running outside the rollup
//! process so multiple rollups can share it (replica/leader, restart, …).

use std::net::SocketAddr;

use sov_mock_da::storable::rpc::start_server;
use sov_mock_da::storable::StorableMockDaService;
use sov_mock_da::{BlockProducingConfig, MockAddress, MockDaConfig};
use sov_shutdown::SecondaryShutdownController;

/// Mock-DA sequencer address. Must match
/// `examples/test-data/genesis/integration-tests/sequencer_registry.json`,
/// otherwise batches are rejected.
pub const TEST_SEQ_DA_ADDRESS: MockAddress = MockAddress::new([0; 32]);

/// Resources owned by [`start_external_mock_da`]. Call `shutdown.shutdown()` to
/// stop the DA service's background tasks.
pub struct ExternalDa {
    pub service: StorableMockDaService,
    pub shutdown: SecondaryShutdownController,
    pub addr: SocketAddr,
}

/// Starts a [`StorableMockDaService`] backed by an in-memory SQLite DB and
/// exposes it over RPC on `127.0.0.1:0`.
pub async fn start_external_mock_da(
    block_producing: BlockProducingConfig,
) -> anyhow::Result<ExternalDa> {
    let shutdown = SecondaryShutdownController::new();
    let mut config = MockDaConfig::instant_with_sender(TEST_SEQ_DA_ADDRESS);
    config.block_producing = block_producing;
    let service = StorableMockDaService::from_config(config, &shutdown).await;
    let addr = start_server(service.clone(), "127.0.0.1", 0).await?;
    Ok(ExternalDa {
        service,
        shutdown,
        addr,
    })
}
