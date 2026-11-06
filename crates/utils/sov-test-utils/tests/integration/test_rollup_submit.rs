use sov_bank::utils::TokenHolder;
use sov_bank::{config_gas_token_id, Bank, Coins};
use sov_mock_da::BlockProducingConfig;
use sov_modules_api::{Amount, ApiTxEffect, EncodeCall, Runtime};
use sov_modules_stf_blueprint::GenesisParams;
use sov_stf_runner::processes::RollupProverConfig;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::test_rollup::{GenesisSource, RollupBuilder, TestRollup};
use sov_test_utils::{
    default_test_signed_transaction_with_nonce, generate_optimistic_runtime_with_kernel,
    RtAgnosticBlueprint, TestSpec, TestUser,
};

// The preferred sequencer requires a kernel that accepts soft confirmations.
generate_optimistic_runtime_with_kernel!(
    TestRuntime <=
    kernel_type: sov_kernels::soft_confirmations::SoftConfirmationsKernel<'a, S>,
    modules: []
);

type S = TestSpec;
type RT = TestRuntime<S>;
type TestBlueprint = RtAgnosticBlueprint<S, RT>;

async fn start_manual_rollup_with_user() -> (TestRollup<TestBlueprint>, TestUser<S>) {
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);
    let user = genesis_config.additional_accounts()[0].clone();
    let genesis_params = GenesisParams {
        runtime: <RT as Runtime<S>>::GenesisConfig::from_minimal_config(genesis_config.into()),
    };
    let test_rollup = RollupBuilder::<TestBlueprint>::new(
        GenesisSource::CustomParams(genesis_params),
        BlockProducingConfig::Manual,
        0,
    )
    .set_config(|c| {
        c.rollup_prover_config = RollupProverConfig::Disabled;
    })
    .set_da_config(|c| {
        c.sender_address = HighLevelOptimisticGenesisConfig::<S>::sequencer_da_addr();
    })
    .start()
    .await
    .unwrap();
    test_rollup.produce_enough_finalized_slots().await;
    (test_rollup, user)
}

fn transfer(to: &TestUser<S>, amount: u128) -> TestRuntimeCall<S> {
    <RT as EncodeCall<Bank<S>>>::to_decodable(sov_bank::CallMessage::Transfer {
        to: to.address(),
        coins: Coins {
            amount: Amount::new(amount),
            token_id: config_gas_token_id(),
        },
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn submit_and_wait_included_returns_receipt_and_events() {
    let (test_rollup, user) = start_manual_rollup_with_user().await;
    let recipient = TestUser::<S>::generate_with_default_balance();

    let included = test_rollup
        .submit_and_wait_included(&user, transfer(&recipient, 0x1234))
        .await
        .unwrap();

    assert!(
        matches!(included.receipt, ApiTxEffect::Successful { .. }),
        "Transfer should succeed, got {:?}",
        included.receipt
    );
    assert_eq!(
        included.events,
        vec![TestRuntimeEvent::Bank(
            sov_bank::event::Event::TokenTransferred {
                from: TokenHolder::User(user.address()),
                to: TokenHolder::User(recipient.address()),
                coins: Coins {
                    amount: Amount::new(0x1234),
                    token_id: config_gas_token_id(),
                },
                memo: None,
            }
        )]
    );
    test_rollup.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn submit_increments_nonce_per_user() {
    let (test_rollup, user) = start_manual_rollup_with_user().await;
    let recipient = TestUser::<S>::generate_with_default_balance();

    for amount in [0x1234, 0x5678] {
        test_rollup
            .submit_and_wait_included(&user, transfer(&recipient, amount))
            .await
            .unwrap();
    }

    let next_nonce = test_rollup
        .api_client()
        .get_next_nonce(&user.credential_id())
        .await
        .unwrap();
    assert_eq!(next_nonce, 2, "Each submitted tx should consume one nonce");
    test_rollup.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn submit_seeds_nonce_from_rollup() {
    let (test_rollup, user) = start_manual_rollup_with_user().await;
    let recipient = TestUser::<S>::generate_with_default_balance();
    let tx = default_test_signed_transaction_with_nonce::<RT, S>(
        user.private_key(),
        &transfer(&recipient, 0x1234),
        0,
        &<RT as Runtime<S>>::CHAIN_HASH,
    );
    test_rollup.send_tx_to_sequencer(&tx).await.unwrap();

    let included = test_rollup
        .submit_and_wait_included(&user, transfer(&recipient, 0x5678))
        .await
        .unwrap();

    assert!(
        matches!(included.receipt, ApiTxEffect::Successful { .. }),
        "A tx submitted after one sent outside `submit` should use the next nonce, got {:?}",
        included.receipt
    );
    test_rollup.shutdown().await.unwrap();
}
