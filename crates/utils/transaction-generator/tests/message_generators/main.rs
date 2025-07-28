use std::sync::Arc;

use sov_bank::Bank;
use sov_modules_api::capabilities::config_chain_id;
use sov_modules_api::prelude::arbitrary::{self};
use sov_modules_api::transaction::TxDetails;
use sov_modules_api::{Amount, DispatchCall, EncodeCall, Runtime};
use sov_paymaster::{
    PayeePolicy, PayerGenesisConfig, Paymaster, PaymasterConfig, PaymasterPolicyInitializer,
    SafeVec,
};
use sov_test_modules::access_pattern::{
    AccessPattern, AccessPatternDiscriminants, AccessPatternGenesisConfig,
};
use sov_test_utils::runtime::genesis::optimistic::{
    HighLevelOptimisticGenesisConfig, MinimalOptimisticGenesisConfig,
};
use sov_test_utils::runtime::{ValueSetter, ValueSetterConfig};
use sov_test_utils::{
    generate_runtime, TestAttester, TestSequencer, TestSpec as S, TestUser, TransactionType,
    TEST_DEFAULT_MAX_FEE, TEST_DEFAULT_MAX_PRIORITY_FEE,
};
use sov_transaction_generator::generators::access_pattern::{
    AccessPatternHarness, AccessPatternMessageGenerator,
};
use sov_transaction_generator::generators::bank::harness_interface::BankHarness;
use sov_transaction_generator::generators::bank::BankMessageGenerator;
use sov_transaction_generator::generators::basic::{
    BasicAccessPatternHarness, BasicBankHarness, BasicCallMessageFactory, BasicChangeLogEntry,
    BasicModuleRef, BasicTag, BasicValueSetterHarness,
};
use sov_transaction_generator::generators::value_setter::{
    ValueSetterHarness, ValueSetterMessageGenerator,
};
use sov_transaction_generator::interface::rng_utils::{get_random_bytes, randomize_buffer};
use sov_transaction_generator::interface::{MessageValidity, Percent};
use sov_transaction_generator::{Distribution, GeneratedMessage, State};
use sov_value_setter::CallMessageDiscriminants as ValueSetterDiscriminants;

mod bank;
mod basic;
mod transactions;

const USER_BALANCE: Amount = Amount::new(1_000_000_000_000);
const MAX_VEC_LEN_VALUE_SETTER: usize = 1000;
const MAXIMUM_WRITE_DATA_LENGTH: usize = 100;
const MAXIMUM_WRITE_BEGIN_INDEX: u64 = 1000;
const MAXIMUM_WRITE_SIZE: u64 = 100;
const MAXIMUM_HOOKS_OPS: u64 = 10;

generate_runtime! {
    name: TestRuntime,
    modules: [paymaster: Paymaster<S>, value_setter: ValueSetter<S>, access_pattern: AccessPattern<S>],
    operating_mode: sov_modules_api::runtime::OperatingMode::Optimistic,
    minimal_genesis_config_type: MinimalOptimisticGenesisConfig<S>,
    gas_enforcer: paymaster: Paymaster<S>,
    runtime_trait_impl_bounds: [],
    kernel_type: sov_kernels::basic::BasicKernel<'a, S>,
    auth_type: sov_modules_api::capabilities::RollupAuthenticator<S, Self>,
    auth_call_wrapper: |call| call,
}

type RT = TestRuntime<S>;
type GeneratorOutput = GeneratedMessage<S, TestRuntimeCall<S>, BasicChangeLogEntry<S>>;

pub const BUFFER_SIZE: usize = 100_000;
// The minimum randomness needed to guarantee successful transaction generation
pub const SAFE_MIN_RANDOMNESS: usize = 1_000;

#[derive(Clone, Debug)]
enum ModulesToUse {
    Bank,
    ValueSetter,
    AccessPattern,
}

impl ModulesToUse {
    /// Builds dynamic module reference
    pub fn select<
        R: Runtime<S>
            + EncodeCall<Bank<S>>
            + EncodeCall<ValueSetter<S>>
            + EncodeCall<AccessPattern<S>>,
    >(
        &self,
        bank_harness: BasicBankHarness<S, R>,
        value_setter_harness: BasicValueSetterHarness<S, R>,
        access_pattern_harness: BasicAccessPatternHarness<S, R>,
    ) -> BasicModuleRef<S, R> {
        match self {
            ModulesToUse::Bank => {
                let module: BasicModuleRef<S, R> = Arc::new(bank_harness);
                module
            }
            ModulesToUse::ValueSetter => {
                let module: BasicModuleRef<S, R> = Arc::new(value_setter_harness);
                module
            }
            ModulesToUse::AccessPattern => {
                let module: BasicModuleRef<S, R> = Arc::new(access_pattern_harness);
                module
            }
        }
    }
}

struct NumTxsExecuted {
    num_bank_txs: u64,
    num_access_pattern_txs: u64,
    num_value_setter_txs: u64,
}

pub fn plain_tx_with_default_details<R: Runtime<S>>(
    gen_output: &GeneratedMessage<S, <R as DispatchCall>::Decodable, BasicChangeLogEntry<S>>,
) -> TransactionType<R, S> {
    TransactionType::Plain {
        message: gen_output.message.clone(),
        key: gen_output.sender.clone(),
        details: TxDetails {
            max_priority_fee_bips: TEST_DEFAULT_MAX_PRIORITY_FEE,
            max_fee: TEST_DEFAULT_MAX_FEE,
            gas_limit: None,
            chain_id: config_chain_id(),
        },
    }
}

pub struct TestGenerator<R: Runtime<S>> {
    generator: BasicCallMessageFactory<S, R>,

    bank_harness: BasicBankHarness<S, R>,
    value_setter_harness: BasicValueSetterHarness<S, R>,
    access_pattern_harness: BasicAccessPatternHarness<S, R>,

    state: State<S, BasicTag>,
    randomness: Vec<u8>,
    remaining_randomness: usize,
    target_buffer_size: usize,
    salt: u128,
    initial_transactions:
        Vec<GeneratedMessage<S, <R as DispatchCall>::Decodable, BasicChangeLogEntry<S>>>,
}

impl<R: Runtime<S>> TestGenerator<R> {
    pub fn generate(
        &mut self,
        modules_distribution: &Distribution<BasicModuleRef<S, R>>,
        validity: MessageValidity,
    ) -> GeneratedMessage<S, <R as DispatchCall>::Decodable, BasicChangeLogEntry<S>> {
        for _ in 0..20 {
            if self.has_enough_randomness() {
                let u =
                    &mut arbitrary::Unstructured::new(&self.randomness[self.randomness_offset()..]);

                if let Ok(output) = self.generator.generate_call_message(
                    modules_distribution,
                    u,
                    &mut self.state,
                    validity,
                ) {
                    self.remaining_randomness = u.len();
                    return output;
                } else {
                    self.target_buffer_size *= 2;
                }
            }
            self.re_randomize();
        }
        unreachable!("Could not get enough randomness to generate a transaction");
    }

    fn re_randomize(&mut self) {
        if self.randomness.len() < self.target_buffer_size {
            self.randomness = vec![0; self.target_buffer_size];
        }
        randomize_buffer(&mut self.randomness[..], self.salt);
        self.remaining_randomness = self.randomness.len();
        self.salt += 1;
    }

    fn randomness_offset(&self) -> usize {
        self.randomness.len() - self.remaining_randomness
    }

    fn has_enough_randomness(&self) -> bool {
        self.remaining_randomness > std::cmp::min(SAFE_MIN_RANDOMNESS, self.target_buffer_size / 10)
    }
}

// Setup generation with the given params
fn setup_harness<
    R: Runtime<S>
        + EncodeCall<Bank<S>>
        + EncodeCall<ValueSetter<S>>
        + EncodeCall<AccessPattern<S>>
        + Clone,
>(
    address_creation_rate: Percent,
    admin: &TestUser<S>,
    max_value_setter_vec_len: usize,
    maximum_write_data_length: usize,
    maximum_write_begin_index: u64,
    maximum_write_size: u64,
    maximum_hooks_ops: u64,
    modules_distribution: &Distribution<ModulesToUse>,
) -> TestGenerator<R> {
    use sov_bank::CallMessageDiscriminants::*;

    let bank_harness = BankHarness::new(BankMessageGenerator::<S>::new(
        Distribution::with_equiprobable_values(vec![Transfer, Freeze, Burn, Mint, CreateToken]),
        address_creation_rate,
    ));

    let value_setter_harness = ValueSetterHarness::new(ValueSetterMessageGenerator::<S>::new(
        Distribution::with_equiprobable_values(vec![
            ValueSetterDiscriminants::SetValue,
            ValueSetterDiscriminants::SetManyValues,
        ]),
        sov_transaction_generator::generators::value_setter::ValueSetterGeneratorOptions {
            maximum_vec_length: max_value_setter_vec_len,
            min_and_max_number_of_individual_state_operations: (0, 0),
            min_and_max_number_of_new_values_for_heavy_state: (0, 0),
            min_and_max_number_of_iterations_for_cpu_heavy_operation: (0, 0),
            max_heavy_state_size: 0,
        },
        admin.private_key.clone(),
    ));

    let access_pattern_harness =
        AccessPatternHarness::new(AccessPatternMessageGenerator::<S>::new(
            Distribution::with_equiprobable_values(vec![
                AccessPatternDiscriminants::WriteCells,
                AccessPatternDiscriminants::WriteCustom,
                AccessPatternDiscriminants::ReadCells,
                AccessPatternDiscriminants::DeleteCells,
                // TODO(@theochap): fix set hook log production
                // AccessPatternDiscriminants::SetHook,
                AccessPatternDiscriminants::HashBytes,
                AccessPatternDiscriminants::HashCustom,
                AccessPatternDiscriminants::StoreSignature,
                AccessPatternDiscriminants::VerifySignature,
                AccessPatternDiscriminants::VerifyCustomSignature,
                AccessPatternDiscriminants::StoreSerializedString,
                AccessPatternDiscriminants::DeserializeBytesAsString,
                AccessPatternDiscriminants::DeserializeCustomString,
                AccessPatternDiscriminants::UpdateAdmin,
            ]),
            maximum_write_data_length,
            maximum_write_begin_index,
            maximum_write_size,
            maximum_hooks_ops,
            admin.private_key.clone(),
        ));

    let modules: Vec<BasicModuleRef<S, R>> = modules_distribution
        .inner()
        .iter()
        .map(|(_, module_to_use)| {
            module_to_use.select::<R>(
                bank_harness.clone(),
                value_setter_harness.clone(),
                access_pattern_harness.clone(),
            )
        })
        .collect();

    let factory = BasicCallMessageFactory::<S, R>::new();

    // Synchronizes the state with the value setter module
    let mut state: State<S, BasicTag> = State::new();

    let random_bytes: Vec<u8> = get_random_bytes(100_000, 0);
    let u = &mut arbitrary::Unstructured::new(&random_bytes[..]);
    let initial_txs = factory
        .generate_setup_messages(&modules, u, &mut state)
        .expect("Failed to generate setup messages");
    let remaining_randomness = u.len();
    TestGenerator {
        randomness: random_bytes,
        bank_harness,
        value_setter_harness,
        access_pattern_harness,
        remaining_randomness,
        generator: factory,
        state,
        target_buffer_size: BUFFER_SIZE,
        salt: 1,
        initial_transactions: initial_txs,
    }
}

pub struct Setup {
    pub user: TestUser<S>,
    /// A user who is pre-registered as a payer for [`sequencer`]
    pub payer: TestUser<S>,
    /// The pre-registered sequencer
    pub sequencer: TestSequencer<S>,
    /// The pre-registered attester
    pub attester: TestAttester<S>,
    /// The admin user of [`ValueSetter`] module
    pub value_setter_admin: TestUser<S>,
    #[allow(missing_docs)]
    pub genesis_config: GenesisConfig<S>,
}

fn setup_roles_and_config(user_balance: Amount) -> Setup {
    let mut genesis_config = HighLevelOptimisticGenesisConfig::generate()
        .add_accounts_with_default_balance(2)
        .add_accounts_with_balance(2, user_balance);

    let quarter_max = Amount::MAX.checked_div(Amount::new(4)).unwrap();
    genesis_config
        .initial_attester
        .user_info
        .available_gas_balance = quarter_max;
    genesis_config.initial_attester.bond = quarter_max;
    genesis_config.initial_sequencer.bond = quarter_max;
    genesis_config
        .initial_sequencer
        .user_info
        .available_gas_balance = quarter_max;

    let sequencer = genesis_config.initial_sequencer.clone();
    let attester = genesis_config.initial_attester.clone();
    let payer = genesis_config
        .additional_accounts()
        .first()
        .unwrap()
        .clone();
    let admin = genesis_config.additional_accounts().get(1).unwrap().clone();
    let user = genesis_config.additional_accounts().get(2).unwrap().clone();
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
            admin: admin.address(),
        },
        AccessPatternGenesisConfig {
            admin: admin.address(),
        },
    );
    Setup {
        payer,
        sequencer,
        attester,
        user,
        genesis_config,
        value_setter_admin: admin,
    }
}
