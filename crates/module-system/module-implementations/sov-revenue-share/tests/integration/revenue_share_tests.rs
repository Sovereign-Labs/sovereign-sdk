use schemars::JsonSchema;
use sov_address::EthereumAddress;
use sov_bank::{Amount, Bank, IntoPayable, TokenId};
use sov_mock_da::{MockAddress, MockDaSpec};
use sov_mock_zkvm::MockZkvm;
use sov_modules_api::configurable_spec::ConfigurableSpec;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::macros::{serialize, UniversalWallet};
use sov_modules_api::{Context, CryptoSpec, Module, ModuleInfo, Spec};
use sov_revenue_share::{CallMessage as RevenueShareCallMessage, RevenueShare};
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{generate_optimistic_runtime, AsUser, TestUser, TransactionTestCase};

use crate::test_helpers::TestCryptoSpec;

type TestSpec =
    ConfigurableSpec<MockDaSpec, MockZkvm, MockZkvm, EthereumAddress, Native, TestCryptoSpec>;

type S = TestSpec;

// Generate a runtime that includes revenue share module and the test helper
generate_optimistic_runtime!(
    TestRevenueShareRuntime <=
    revenue_share_module: RevenueShare<S>,
    test_helper: TestHelper<S>
);

type RT = TestRevenueShareRuntime<S>;

#[derive(Clone, ModuleInfo)]
pub(crate) struct TestHelper<S: Spec> {
    #[id]
    pub id: sov_modules_api::ModuleId,

    #[module]
    pub revenue_share: RevenueShare<S>,
}

#[derive(Clone, Debug, PartialEq, Eq, JsonSchema, UniversalWallet)]
#[serialize(Borsh, Serde)]
#[schemars(bound = "S: Spec", rename = "TestHelperMessages")]
pub enum TestHelperCallMessage<S: Spec> {
    PayRevenueShare {
        token_id: TokenId,
        amount: Amount,
        from: S::Address,
    },
    ComputeAndPayRevenueShare {
        token_id: TokenId,
        total_revenue: Amount,
        from: S::Address,
    },
    CheckIsPreferredSequencer,
}

impl<S: Spec> Module for TestHelper<S> {
    type Spec = S;
    type Config = ();
    type CallMessage = TestHelperCallMessage<S>;
    type Event = ();

    fn call(
        &mut self,
        msg: Self::CallMessage,
        context: &sov_modules_api::Context<S>,
        state: &mut impl sov_modules_api::TxState<S>,
    ) -> anyhow::Result<()> {
        match msg {
            TestHelperCallMessage::PayRevenueShare {
                token_id,
                amount,
                from,
            } => self
                .revenue_share
                .pay_revenue_share(&from, token_id, amount, state),
            TestHelperCallMessage::ComputeAndPayRevenueShare {
                token_id,
                total_revenue,
                from,
            } => self.revenue_share.compute_and_pay_revenue_share(
                &from,
                token_id,
                total_revenue,
                state,
            ),
            TestHelperCallMessage::CheckIsPreferredSequencer => {
                let is_preferred = self.revenue_share.is_preferred_sequencer(context, state);
                if is_preferred {
                    Ok(())
                } else {
                    anyhow::bail!("Not preferred sequencer")
                }
            }
        }
    }
}

struct TestSetup {
    admin_user: TestUser<S>,
    regular_user: TestUser<S>,
    operator_user: TestUser<S>,
    token_id: TokenId,
}

fn setup() -> (TestSetup, TestRunner<RT, S>) {
    // Generate genesis config with accounts
    let mut genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(2);

    let regular_user = genesis_config
        .additional_accounts()
        .first()
        .unwrap()
        .clone();
    let operator_user = genesis_config.additional_accounts().get(1).unwrap().clone();

    let priv_key = serde_json::from_str::<<TestCryptoSpec as CryptoSpec>::PrivateKey>(
        r#""0d87c12ea7c12024b3f70a26d735874608f17c8bce2b48e6fe87389310191264""#,
    )
    .unwrap();
    let admin_user = TestUser::<TestSpec>::new(priv_key, sov_test_utils::TEST_DEFAULT_USER_BALANCE);

    genesis_config
        .additional_accounts_mut()
        .push(admin_user.clone());

    let token_id = sov_bank::config_gas_token_id();

    // Build the runtime-specific genesis config with revenue share config
    let minimal_config: sov_test_utils::runtime::genesis::optimistic::MinimalOptimisticGenesisConfig<S> = genesis_config.into();

    let genesis_config = GenesisConfig::from_minimal_config(minimal_config, (), ());
    let runner = TestRunner::new_with_genesis(genesis_config.into_genesis_params(), RT::default());

    let test_setup = TestSetup {
        admin_user,
        regular_user,
        operator_user,
        token_id,
    };

    (test_setup, runner)
}

#[test]
fn test_activate_deactivate_revenue_share() {
    let (setup, mut runner) = setup();

    // Try to activate as non-admin (should fail)
    runner.execute_transaction(TransactionTestCase {
        input: setup
            .regular_user
            .create_plain_message::<RT, sov_revenue_share::RevenueShare<S>>(
                RevenueShareCallMessage::ActivateRevenueShare,
            ),
        assert: Box::new(|result, _state| {
            assert!(result.tx_receipt.is_reverted());
        }),
    });

    // Activate as admin (should succeed)
    runner.execute_transaction(TransactionTestCase {
        input: setup
            .admin_user
            .create_plain_message::<RT, sov_revenue_share::RevenueShare<S>>(
                RevenueShareCallMessage::ActivateRevenueShare,
            ),
        assert: Box::new(|result, _state| {
            assert!(result.tx_receipt.is_successful());
        }),
    });

    // Check that revenue share is active
    runner.query_state(|state| {
        let is_active = runner
            .runtime()
            .revenue_share_module
            .is_active
            .get(state)
            .unwrap()
            .unwrap();
        assert!(is_active);
    });

    // Deactivate as non-admin (should fail)
    runner.execute_transaction(TransactionTestCase {
        input: setup
            .regular_user
            .create_plain_message::<RT, sov_revenue_share::RevenueShare<S>>(
                RevenueShareCallMessage::DeactivateRevenueShare,
            ),
        assert: Box::new(|result, _state| {
            assert!(result.tx_receipt.is_reverted());
        }),
    });

    // Deactivate as admin (should succeed)
    runner.execute_transaction(TransactionTestCase {
        input: setup
            .admin_user
            .create_plain_message::<RT, sov_revenue_share::RevenueShare<S>>(
                RevenueShareCallMessage::DeactivateRevenueShare,
            ),
        assert: Box::new(|result, _state| {
            assert!(result.tx_receipt.is_successful());
        }),
    });

    // Check that revenue share is inactive
    runner.query_state(|state| {
        let is_active = runner
            .runtime()
            .revenue_share_module
            .is_active
            .get(state)
            .unwrap()
            .unwrap();
        assert!(!is_active);
    });
}

#[test]
fn test_update_revenue_percentage() {
    let (setup, mut runner) = setup();

    // Try to increase percentage as admin (should fail)
    runner.execute_transaction(TransactionTestCase {
        input: setup
            .admin_user
            .create_plain_message::<RT, sov_revenue_share::RevenueShare<S>>(
                RevenueShareCallMessage::LowerRevenuePercentage {
                    percentage_in_basis_points: 20000,
                },
            ),
        assert: Box::new(|result, _state| {
            assert!(result.tx_receipt.is_reverted());
        }),
    });

    // Decrease percentage to 5% (500 basis points) as admin (should succeed)
    runner.execute_transaction(TransactionTestCase {
        input: setup
            .admin_user
            .create_plain_message::<RT, sov_revenue_share::RevenueShare<S>>(
                RevenueShareCallMessage::LowerRevenuePercentage {
                    percentage_in_basis_points: 500,
                },
            ),
        assert: Box::new(|result, _state| {
            assert!(result.tx_receipt.is_successful());
        }),
    });

    // Check that revenue share percentage is updated
    runner.query_state(|state| {
        let percentage = runner
            .runtime()
            .revenue_share_module
            .revenue_share_percentage_bps
            .get(state)
            .unwrap()
            .unwrap();

        assert_eq!(percentage, 500);
    });

    // Try to decrease percentage from a non-admin account (should fail)
    runner.execute_transaction(TransactionTestCase {
        input: setup
            .regular_user
            .create_plain_message::<RT, sov_revenue_share::RevenueShare<S>>(
                RevenueShareCallMessage::LowerRevenuePercentage {
                    percentage_in_basis_points: 100,
                },
            ),
        assert: Box::new(|result, _state| {
            assert!(result.tx_receipt.is_reverted());
        }),
    });
}

#[test]
fn test_update_sovereign_admin() {
    let (setup, mut runner) = setup();

    // Try to update admin as non-admin (should fail)
    runner.execute_transaction(TransactionTestCase {
        input: setup
            .regular_user
            .create_plain_message::<RT, sov_revenue_share::RevenueShare<S>>(
                RevenueShareCallMessage::UpdateSovereignAdmin {
                    new_admin: setup.regular_user.address(),
                },
            ),
        assert: Box::new(|result, _state| {
            assert!(result.tx_receipt.is_reverted());
        }),
    });

    // Update admin (should succeed)
    runner.execute_transaction(TransactionTestCase {
        input: setup
            .admin_user
            .create_plain_message::<RT, sov_revenue_share::RevenueShare<S>>(
                RevenueShareCallMessage::UpdateSovereignAdmin {
                    new_admin: setup.regular_user.address(),
                },
            ),
        assert: Box::new(|result, _state| {
            assert!(result.tx_receipt.is_successful());
        }),
    });

    // Check that the admin is updated
    runner.query_state(|state| {
        let admin = runner
            .runtime()
            .revenue_share_module
            .sovereign_admin_override
            .get(state)
            .unwrap()
            .unwrap();
        assert_eq!(admin, setup.regular_user.address());
    });

    // Old admin should no longer work
    runner.execute_transaction(TransactionTestCase {
        input: setup
            .admin_user
            .create_plain_message::<RT, sov_revenue_share::RevenueShare<S>>(
                RevenueShareCallMessage::ActivateRevenueShare,
            ),
        assert: Box::new(|result, _state| {
            assert!(result.tx_receipt.is_reverted());
        }),
    });

    // New admin should work
    runner.execute_transaction(TransactionTestCase {
        input: setup
            .regular_user
            .create_plain_message::<RT, sov_revenue_share::RevenueShare<S>>(
                RevenueShareCallMessage::ActivateRevenueShare,
            ),
        assert: Box::new(|result, _state| {
            assert!(result.tx_receipt.is_successful());
        }),
    });
}

#[test]
fn test_pay_revenue_share() {
    let (setup, mut runner) = setup();

    // Get initial user balance
    let user_balance_start = runner.query_state(|state| {
        let bank = Bank::<S>::default();
        bank.get_balance_of(&setup.regular_user.address(), setup.token_id, state)
            .unwrap()
            .unwrap()
    });

    // Calculate the amount to send (revenue share percentage * user balance)
    let send_amount = Amount::new(10000);

    // Try to pay share revenue (should succeed but not send any revenue as revenue share is inactive)
    runner.execute_transaction(TransactionTestCase {
        input: setup.admin_user.create_plain_message::<RT, TestHelper<S>>(
            TestHelperCallMessage::PayRevenueShare {
                token_id: setup.token_id,
                amount: send_amount,
                from: setup.regular_user.address(),
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(result.tx_receipt.is_successful());
        }),
    });

    // Check that user balance hasn't changed
    runner.query_state(|state| {
        let bank = Bank::<S>::default();
        let user_balance = bank
            .get_balance_of(&setup.regular_user.address(), setup.token_id, state)
            .unwrap()
            .unwrap();

        assert_eq!(user_balance, user_balance_start);
    });

    // Activate revenue share
    runner.execute(
        setup
            .admin_user
            .create_plain_message::<RT, sov_revenue_share::RevenueShare<S>>(
                RevenueShareCallMessage::ActivateRevenueShare,
            ),
    );

    // Pay revenue (should succeed and send revenue)
    runner.execute_transaction(TransactionTestCase {
        input: setup.admin_user.create_plain_message::<RT, TestHelper<S>>(
            TestHelperCallMessage::PayRevenueShare {
                token_id: setup.token_id,
                amount: send_amount,
                from: setup.regular_user.address(),
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(result.tx_receipt.is_successful());
        }),
    });

    // Check that user balance has changed
    runner.query_state(|state| {
        let bank = Bank::<S>::default();
        let user_balance = bank
            .get_balance_of(&setup.regular_user.address(), setup.token_id, state)
            .unwrap()
            .unwrap();

        let revenue_share_module = RevenueShare::<S>::default();
        let revenue_share_balance = bank
            .get_balance_of(
                revenue_share_module.id().to_payable(),
                setup.token_id,
                state,
            )
            .unwrap()
            .unwrap();

        assert_eq!(user_balance, user_balance_start.saturating_sub(send_amount));
        assert_eq!(send_amount, revenue_share_balance);
    });
}

#[test]
fn test_compute_and_pay_revenue_share() {
    let (setup, mut runner) = setup();

    // Get initial revenue share percentage and realized revenue
    // (we're assuming the regular user account contains all realized revenue)
    let (revenue_share_percentage_bps, realized_revenue) = runner.query_state(|state| {
        let revenue_share = RevenueShare::<S>::default();
        let rev_share_percentage_bps = revenue_share.get_revenue_share_percentage_bps(state);

        let bank = Bank::<S>::default();
        let realized_revenue = bank
            .get_balance_of(&setup.regular_user.address(), setup.token_id, state)
            .unwrap()
            .unwrap();

        (rev_share_percentage_bps, realized_revenue)
    });

    // Calculate the amount that should be sent (revenue share percentage * realized revenue)
    let amount_to_pay = realized_revenue
        .saturating_mul(Amount::new(revenue_share_percentage_bps as u128))
        .saturating_div(Amount::new(10000));

    // Try share revenue (should succeed but not send any revenue as revenue share is inactive)
    runner.execute_transaction(TransactionTestCase {
        input: setup.admin_user.create_plain_message::<RT, TestHelper<S>>(
            TestHelperCallMessage::ComputeAndPayRevenueShare {
                token_id: setup.token_id,
                total_revenue: realized_revenue,
                from: setup.regular_user.address(),
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(result.tx_receipt.is_successful());
        }),
    });

    // Check that the stored revenue hasn't changed
    runner.query_state(|state| {
        let bank = Bank::<S>::default();
        let realized_revenue_after_attempting_to_pay = bank
            .get_balance_of(&setup.regular_user.address(), setup.token_id, state)
            .unwrap()
            .unwrap();

        assert_eq!(realized_revenue, realized_revenue_after_attempting_to_pay);
    });

    // Activate revenue share
    runner.execute(
        setup
            .admin_user
            .create_plain_message::<RT, sov_revenue_share::RevenueShare<S>>(
                RevenueShareCallMessage::ActivateRevenueShare,
            ),
    );

    // Share revenue (should succeed and send revenue)
    runner.execute_transaction(TransactionTestCase {
        input: setup.admin_user.create_plain_message::<RT, TestHelper<S>>(
            TestHelperCallMessage::ComputeAndPayRevenueShare {
                token_id: setup.token_id,
                total_revenue: realized_revenue,
                from: setup.regular_user.address(),
            },
        ),
        assert: Box::new(|result, _state| {
            assert!(result.tx_receipt.is_successful());
        }),
    });

    // Check that user balance has changed and revenue share balance has increased
    runner.query_state(|state| {
        let bank = Bank::<S>::default();
        let realized_revenue_after_paying = bank
            .get_balance_of(&setup.regular_user.address(), setup.token_id, state)
            .unwrap()
            .unwrap();

        let revenue_share_module = RevenueShare::<S>::default();
        let revenue_share_balance = bank
            .get_balance_of(
                revenue_share_module.id().to_payable(),
                setup.token_id,
                state,
            )
            .unwrap()
            .unwrap();

        assert_eq!(
            realized_revenue_after_paying,
            realized_revenue.saturating_sub(amount_to_pay)
        );
        assert_eq!(amount_to_pay, revenue_share_balance);
    });
}

#[test]
fn test_withdraw_rewards() {
    let (setup, mut runner) = setup();

    // Activate revenue share
    runner.execute(
        setup
            .admin_user
            .create_plain_message::<RT, sov_revenue_share::RevenueShare<S>>(
                RevenueShareCallMessage::ActivateRevenueShare,
            ),
    );

    // Make operator sovereign admin
    runner.execute(
        setup
            .admin_user
            .create_plain_message::<RT, sov_revenue_share::RevenueShare<S>>(
                RevenueShareCallMessage::UpdateSovereignAdmin {
                    new_admin: setup.operator_user.address(),
                },
            ),
    );

    // Get operator balance
    let operator_balance = runner.query_state(|state| {
        let bank = Bank::<S>::default();
        bank.get_balance_of(&setup.operator_user.address(), setup.token_id, state)
            .unwrap()
            .unwrap()
    });

    // Pay revenue share (send revenue to revenue share module)
    let revenue_share = Amount::new(10000);
    runner.execute(setup.admin_user.create_plain_message::<RT, TestHelper<S>>(
        TestHelperCallMessage::PayRevenueShare {
            token_id: setup.token_id,
            amount: revenue_share,
            from: setup.regular_user.address(),
        },
    ));

    // Try to withdraw rewards to a non-admin account (should fail)
    runner.execute_transaction(TransactionTestCase {
        input: setup
            .regular_user
            .create_plain_message::<RT, sov_revenue_share::RevenueShare<S>>(
                RevenueShareCallMessage::WithdrawRewards {
                    token_id: setup.token_id,
                },
            ),
        assert: Box::new(|result, _state| {
            assert!(result.tx_receipt.is_reverted());
        }),
    });

    // Try to withdraw rewards to the admin account (should succeed)
    runner.execute_transaction(TransactionTestCase {
        input: setup
            .operator_user
            .create_plain_message::<RT, sov_revenue_share::RevenueShare<S>>(
                RevenueShareCallMessage::WithdrawRewards {
                    token_id: setup.token_id,
                },
            ),
        assert: Box::new(move |result, state| {
            assert!(result.tx_receipt.is_successful());
            let expected_operator_balance = operator_balance
                .saturating_sub(result.gas_value_used)
                .saturating_add(revenue_share);
            let bank = Bank::<S>::default();

            let actual_operator_balance = bank
                .get_balance_of(&setup.operator_user.address(), setup.token_id, state)
                .unwrap()
                .unwrap();
            assert_eq!(actual_operator_balance, expected_operator_balance);
        }),
    });
}

#[test]
fn test_is_preferred_sequencer() {
    use std::str::FromStr;

    use sov_modules_api::transaction::Credentials;

    let (setup, mut runner) = setup();

    // Default sequencer should be the preferred sequencer in test setup
    runner.execute_transaction(TransactionTestCase {
        input: setup
            .regular_user
            .create_plain_message::<RT, TestHelper<S>>(
                TestHelperCallMessage::CheckIsPreferredSequencer,
            ),
        assert: Box::new(|result, _state| {
            // Should succeed because default test runner uses preferred sequencer
            assert!(result.tx_receipt.is_successful());
        }),
    });

    // Test with a random address that's neither preferred nor registered
    runner.query_state(|state| {
        let revenue_share = RevenueShare::<S>::default();

        // Create a mock address that is not the preferred sequencer
        let random_address =
            EthereumAddress::from_str("0x71334bf1710D12c9f689cC819476fA589F08C64C").unwrap();
        let random_da_address = MockAddress::from([99; 32]);
        let ctx = Context::<S>::new(
            random_address,
            Credentials::default(),
            random_address,
            random_da_address,
        );

        let is_preferred = revenue_share.is_preferred_sequencer(&ctx, state);
        assert!(
            !is_preferred,
            "Random address should not be preferred sequencer"
        );
    });
}
