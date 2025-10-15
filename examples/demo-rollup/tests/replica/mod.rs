use crate::test_helpers::build_transfer_token_tx;
use crate::test_helpers::test_genesis_source;
use sov_bank::config_gas_token_id;
use sov_demo_rollup::ExternalMockDemoRollup;
use sov_mock_da::storable::rpc::start_server;
use sov_mock_da::storable::rpc::MockDaClientConfig;
use sov_mock_da::storable::StorableMockDaService;
use sov_mock_da::MockAddress;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::CryptoSpec;
use sov_modules_api::OperatingMode;
use sov_modules_api::PrivateKey;
use sov_modules_api::PublicKey;
use sov_modules_api::Spec;
use sov_modules_rollup_blueprint::RollupBlueprint;
use sov_test_utils::test_rollup::read_private_key;
use sov_test_utils::test_rollup::RollupBuilder;
use sov_test_utils::test_rollup::TestRollup;

type S = <ExternalMockDemoRollup<Native> as RollupBlueprint<Native>>::Spec;

fn random_address() -> <S as Spec>::Address {
    let pk = <<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey::generate();
    pk.pub_key().credential_id().into()
}

#[tokio::test(flavor = "multi_thread")]
async fn test_replica() {
    let genesis = test_genesis_source(OperatingMode::Operator);
    let (da_service, _shutdown_sender) =
        StorableMockDaService::new_in_memory_periodic(1000, MockAddress::new([0; 32])).await;

    let addr = start_server(da_service.clone(), "127.0.0.1", 0)
        .await
        .unwrap();

    let key_and_address = read_private_key::<
        <ExternalMockDemoRollup<Native> as RollupBlueprint<Native>>::Spec,
    >("tx_signer_private_key.json");

    let test_rollup: TestRollup<ExternalMockDemoRollup<Native>> =
        RollupBuilder::new_with_external_da(
            genesis,
            MockDaClientConfig {
                url: format!("http://{addr}"),
            },
        )
        .start_test_rollup()
        .await
        .unwrap();

    let token_id = config_gas_token_id();

    da_service.wait_for_height(10).await.unwrap();

    let receiver_addr: <S as Spec>::Address = random_address();

    let tx = build_transfer_token_tx::<S>(
        &key_and_address.private_key,
        token_id,
        receiver_addr,
        100,
        0,
    );

    test_rollup
        .client
        .client
        .send_txs_to_sequencer(&[tx])
        .await
        .unwrap();

    let receiver_balance = test_rollup
        .client
        .get_balance::<S>(&receiver_addr, &token_id, None)
        .await
        .unwrap();

    assert_eq!(receiver_balance.0, 100);
}
