use crate::utils::encode_call;
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use sov_api_spec::types as api_types;
use sov_full_node_configs::sequencer::Limits;
use sov_full_node_configs::sequencer::SovRateLimiterConfig;
use sov_mock_da::BlockProducingConfig;
use sov_mock_zkvm::crypto::private_key::Ed25519PrivateKey;
use sov_modules_api::CryptoSpec;
use sov_modules_api::DispatchCall;
use sov_modules_api::PrivateKey;
use sov_modules_api::RawTx;
use sov_modules_api::Spec;
use sov_modules_stf_blueprint::Runtime;
use sov_sequencer::rest_api::AcceptTx;
use sov_sequencer::SequencerKindConfig;
use sov_test_utils::generate_operator_runtime_with_kernel;
use sov_test_utils::runtime::genesis::operator::HighLevelOperatorGenesisConfig;
use sov_test_utils::runtime::GenesisParams;
use sov_test_utils::runtime::SoftConfirmationsKernel;
use sov_test_utils::test_rollup::GenesisSource;
use sov_test_utils::test_rollup::RollupBuilder;
use sov_test_utils::test_rollup::StoragePath;
use sov_test_utils::test_rollup::TestRollup;
use sov_test_utils::RtAgnosticBlueprint;
use sov_test_utils::TestSpec;
use sov_test_utils::TestUser;
use sov_test_utils::TEST_DEFAULT_USER_BALANCE;
use sov_value_setter::ValueSetter;
use sov_value_setter::ValueSetterConfig;
use std::sync::Arc;
use std::time::Duration;

generate_operator_runtime_with_kernel!(kernel_type: SoftConfirmationsKernel<'a, S>, TestRuntime <= value_setter: ValueSetter<S>);

type RT = TestRuntime<TestSpec>;
type TestBlueprint = RtAgnosticBlueprint<TestSpec, RT>;

async fn create_test_rollup(
    rate_limiter_config: SovRateLimiterConfig<<TestSpec as Spec>::Address>,
) -> (TestRollup<TestBlueprint>, TestUser<TestSpec>) {
    let reward_user = TestUser::<TestSpec>::generate(TEST_DEFAULT_USER_BALANCE);

    let genesis_config =
        HighLevelOperatorGenesisConfig::<TestSpec>::generate_with_additional_accounts(
            1,
            reward_user,
        );

    let admin = genesis_config.additional_accounts()[0].clone();
    let rt_genesis_config = <RT as Runtime<TestSpec>>::GenesisConfig::from_minimal_config(
        genesis_config.into(),
        ValueSetterConfig {
            admin: admin.address(),
        },
    );

    let genesis_params = GenesisParams {
        runtime: rt_genesis_config.clone(),
    };

    let dir = Arc::new(tempfile::tempdir().unwrap());

    let seq_da_address = genesis_params
        .runtime
        .sequencer_registry
        .sequencer_config
        .seq_da_address;

    let builder = RollupBuilder::<RtAgnosticBlueprint<TestSpec, RT>>::new(
        GenesisSource::CustomParams(genesis_params),
        BlockProducingConfig::Manual,
        0,
    )
    .set_config(|c| {
        c.storage = StoragePath::Tmp(dir);
        c.max_concurrent_blobs = 64;
        if let SequencerKindConfig::Preferred(ref mut config) = &mut c.sequencer_config {
            config.num_cache_warmup_workers = 0;
            config.batch_execution_time_limit_millis = 3000;
            config.rate_limiter = Some(rate_limiter_config);
        }
    })
    .set_da_config(|c| c.sender_address = seq_da_address)
    .set_persistent_da()
    .with_preferred_seq_recovery_strategy(sov_sequencer::preferred::RecoveryStrategy::TryToSave);

    (builder.start().await.unwrap(), admin)
}

#[tokio::test(flavor = "multi_thread")]
async fn test_rate_limiting() {
    let sov_config = SovRateLimiterConfig {
        default_limits: Limits {
            resources_per_bucket: 5,
            refill_rate: 100,
        },
        max_requests_per_second: 1_000_000,
        address_custom_limits: Vec::default(),
        ip_custom_limits: Vec::default(),
    };

    let (test_rollup, admin) = create_test_rollup(sov_config).await;
    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let client = test_rollup.api_client().clone();

    // Send tx that takes 200ms but the limit for each sender is 3000*0.5% = 15ms
    let tx = tx_set_value_and_sleep(&admin.private_key, 0, 99, 200);

    // The transactions is accepted but the sender has been rate-limited after it is executed.
    client
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&tx),
        })
        .await
        .unwrap();

    // Another transactions fails.
    let tx: RawTx = tx_set_value_and_sleep(&admin.private_key, 1, 100, 1);
    let err = client
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&tx),
        })
        .await
        .unwrap_err();

    let err_str = err.to_string();
    assert!(err_str.contains("The sender was rate-limited by the sequencer:"));

    // Unfortunately, the only way to test this is by waiting.
    // The rate limits recover proportionally to the time elapsed since the last request.
    // Wait long enough for the limits to reset.
    tokio::time::sleep(Duration::from_millis(2000)).await;

    client
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&tx),
        })
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_correct_ip() {
    let sov_config = SovRateLimiterConfig {
        default_limits: Limits {
            resources_per_bucket: 5,
            refill_rate: 0,
        },
        max_requests_per_second: 0,
        address_custom_limits: Vec::default(),
        ip_custom_limits: Vec::default(),
    };

    let (test_rollup, admin) = create_test_rollup(sov_config).await;
    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await.unwrap();

    let base_url = test_rollup.client.base_url.clone();
    let client = test_rollup.api_client().clone();

    let url = format!("{base_url}/sequencer/txs");
    let x_forwarded_for = "123.123.123.123";

    // Send first tx.
    {
        let tx: RawTx = tx_set_value_and_sleep(&admin.private_key, 0, 100, 0);
        let request = AcceptTx {
            body: sov_sequencer::rest_api::Base64Blob { blob: tx.data },
        };

        client
            .client()
            .post(&url)
            .json(&request)
            .header("X-forwarded-for", x_forwarded_for)
            .send()
            .await
            .unwrap();
    }

    // Send another tx with the same x_forwarded_for ip but diffrent sender address.
    {
        let private_key = <<TestSpec as Spec>::CryptoSpec as CryptoSpec>::PrivateKey::generate();
        let tx: RawTx = tx_set_value_and_sleep(&private_key, 1, 100, 0);

        let request = AcceptTx {
            body: sov_sequencer::rest_api::Base64Blob { blob: tx.data },
        };

        let resp = client
            .client()
            .post(&url)
            .json(&request)
            // The header should be case insensitive.
            .header("x-forwarded-for", x_forwarded_for)
            .send()
            .await
            .unwrap();

        let err = resp.bytes().await.unwrap();
        let err_str = std::str::from_utf8(&err).unwrap().to_string();

        println!("err_str: {err_str}");
        // Check that the correct IP was rate limmited.
        assert!(err_str
            .contains("The sender was rate-limited by the sequencer: Resource limit exceeded for IP: 123.123.123.123"));
    }
}

fn tx_set_value_and_sleep(
    key: &Ed25519PrivateKey,
    generation: u64,
    value_to_set: u64,
    sleep_millis: u64,
) -> RawTx {
    let msg = <TestRuntime<TestSpec> as DispatchCall>::Decodable::ValueSetter(
        sov_value_setter::CallMessage::SetValueAndSleep {
            value: value_to_set as u32,
            sleep_millis,
        },
    );
    encode_call::<RT>(key, generation, &msg)
}
