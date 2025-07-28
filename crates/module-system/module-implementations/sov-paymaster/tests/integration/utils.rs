use std::collections::HashMap;

use sov_modules_api::{Amount, CryptoSpec, PrivateKey, SafeVec, Spec};
use sov_paymaster::{PayeePolicy, PayerGenesisConfig, PaymasterConfig, PaymasterPolicyInitializer};
use sov_state::{DefaultStorageSpec, ProverStorage};
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::traits::MinimalGenesis;
use sov_test_utils::runtime::{
    Runtime, TestRunner, ValueSetter, ValueSetterCallMessage, ValueSetterConfig,
};
use sov_test_utils::{
    AsUser, EncodeCall, MockDaSpec, TestSequencer, TestUser, TransactionTestCase, TransactionType,
};

use crate::runtime::{GenesisConfig, PaymasterRuntime};

pub type S = sov_test_utils::TestSpec;
pub type RT = PaymasterRuntime<S>;

pub struct Setup {
    pub user: TestUser<S>,
    pub user_2: TestUser<S>,
    /// A user who is pre-registered as a payer for [`sequencer`]
    pub payer: TestUser<S>,
    /// The pre-registered sequencer
    pub sequencer: TestSequencer<S>,
    pub genesis_config: GenesisConfig<S>,
}

impl Setup {
    pub fn payer_setup(&mut self) -> &mut PayerGenesisConfig<S> {
        self.genesis_config.paymaster.payers.first_mut().unwrap()
    }
}

pub enum TxOutcome {
    Skipped,
    Executed,
    Reverted,
}

// Use a trait to circumvent the orphan rule and add `do_value_setter_tx` to TestRunner
pub trait DoValueSetterTx<S: Spec> {
    fn do_value_setter_tx(&mut self, user: &TestUser<S>, expected_outcome: TxOutcome);
    fn do_value_setter_tx_with_generation(
        &mut self,
        user: &TestUser<S>,
        generation: u64,
        expected_outcome: TxOutcome,
    );
}

impl<RT: Runtime<S>, S: Spec> DoValueSetterTx<S> for TestRunner<RT, S>
where
    RT: 'static + Runtime<S> + MinimalGenesis<S> + EncodeCall<ValueSetter<S>>,
    S: Spec<
        Storage = ProverStorage<
            DefaultStorageSpec<<<S as Spec>::CryptoSpec as CryptoSpec>::Hasher>,
        >,
        Da = MockDaSpec,
    >,
{
    fn do_value_setter_tx(&mut self, user: &TestUser<S>, expected_outcome: TxOutcome) {
        match expected_outcome {
            TxOutcome::Skipped => self.execute_skipped_transaction(TransactionTestCase {
                input: user.create_plain_message::<RT, ValueSetter<S>>(
                    ValueSetterCallMessage::SetValue {
                        value: 99,
                        gas: None,
                    },
                ),
                assert: Box::new(|_, _| {}),
            }),
            TxOutcome::Reverted => self.execute_transaction(TransactionTestCase {
                input: user.create_plain_message::<RT, ValueSetter<S>>(
                    ValueSetterCallMessage::AssertVisibleSlotNumber {
                        expected_visible_slot_number: 10_000_000,
                    },
                ),
                assert: Box::new(|_, _| {}),
            }),
            TxOutcome::Executed => self.execute_transaction(TransactionTestCase {
                input: user.create_plain_message::<RT, ValueSetter<S>>(
                    ValueSetterCallMessage::SetValue {
                        value: 99,
                        gas: None,
                    },
                ),
                assert: Box::new(|result, _state| {
                    assert!(!result.tx_receipt.is_skipped());
                }),
            }),
        };
    }

    fn do_value_setter_tx_with_generation(
        &mut self,
        user: &TestUser<S>,
        generation: u64,
        expected_outcome: TxOutcome,
    ) {
        match expected_outcome {
            TxOutcome::Skipped => {
                let input = user.create_plain_message::<RT, ValueSetter<S>>(
                    ValueSetterCallMessage::SetValue {
                        value: 99,
                        gas: None,
                    },
                );
                let input =
                    TransactionType::PreAuthenticated(input.to_serialized_authenticated_tx(
                        &mut HashMap::from([(user.private_key().pub_key(), generation)]),
                    ));
                self.execute_skipped_transaction(TransactionTestCase {
                    input,
                    assert: Box::new(|_, _| {}),
                })
            }
            TxOutcome::Reverted => {
                // Unused in any tests. Use do_value_setter_tx for reverted test cases.
                // Can be implemented later if it becoms needed for a specific setup
                unimplemented!();
            }
            TxOutcome::Executed => {
                let input = user.create_plain_message::<RT, ValueSetter<S>>(
                    ValueSetterCallMessage::SetValue {
                        value: 99,
                        gas: None,
                    },
                );
                let input =
                    TransactionType::PreAuthenticated(input.to_serialized_authenticated_tx(
                        &mut HashMap::from([(user.private_key().pub_key(), generation)]),
                    ));
                self.execute_transaction(TransactionTestCase {
                    input,
                    assert: Box::new(|result, _state| {
                        assert!(!result.tx_receipt.is_skipped());
                    }),
                })
            }
        };
    }
}

/// Setup a genesis config containing a sequencer, a pre-registered payer, and two additional users with the requested balance.
pub fn setup(user_balance: Amount) -> Setup {
    // Generate a genesis config
    let genesis_config = HighLevelOptimisticGenesisConfig::generate()
        .add_accounts_with_default_balance(1)
        .add_accounts_with_balance(2, user_balance);

    let sequencer = genesis_config.initial_sequencer.clone();
    let payer = genesis_config
        .additional_accounts()
        .first()
        .unwrap()
        .clone();
    let user = genesis_config.additional_accounts().get(1).unwrap().clone();
    let user_2 = genesis_config.additional_accounts().get(2).unwrap().clone();

    let genesis_config = GenesisConfig::from_minimal_config(
        genesis_config.into(),
        PaymasterConfig {
            payers: [PayerGenesisConfig {
                payer_address: payer.address(),
                policy: PaymasterPolicyInitializer {
                    default_payee_policy: PayeePolicy::Allow {
                        max_fee: None,
                        gas_limit: None,
                        max_gas_price: None,
                        transaction_limit: None,
                    },
                    payees: SafeVec::new(),
                    authorized_sequencers: sov_paymaster::AuthorizedSequencers::All,
                    authorized_updaters: [payer.address()].as_ref().try_into().unwrap(),
                },
                sequencers_to_register: [sequencer.da_address].as_ref().try_into().unwrap(),
            }]
            .as_ref()
            .try_into()
            .unwrap(),
        },
        ValueSetterConfig {
            admin: user.address(),
        },
    );

    Setup {
        payer,
        sequencer,
        user,
        user_2,
        genesis_config,
    }
}
