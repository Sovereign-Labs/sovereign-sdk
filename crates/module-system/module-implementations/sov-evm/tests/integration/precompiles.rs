use crate::helpers::EvmAccount;

use alloy_consensus::{TxEip1559, TypedTransaction};
use alloy_eips::eip1559::MIN_PROTOCOL_BASE_FEE;
use alloy_eips::eip2718::Encodable2718;
use alloy_primitives::{address, Address, Bytes, TxKind, U256};
use sov_address::{EthereumAddress, FromVmAddress, MultiAddress, MultiAddressEvm};
use sov_bank::{config_gas_token_id, Amount, Coins};
use sov_eth_dev_signer::Signer;
use sov_evm::precompiles::{
    BankBalancePrecompile, EvmPrecompileEnv, EvmPrecompileSet, PrecompileResult,
    SequencingTimestampPrecompile, BANK_BALANCE_PRECOMPILE_ADDRESS,
    SEQUENCING_TIMESTAMP_PRECOMPILE_ADDRESS,
};
use sov_evm::{
    AccountData, ContractCreationPolicy, EthereumAuthenticator, Evm, EvmAuthenticatorInput,
    EvmChainSpec, EvmGenesisConfig, RlpEvmTransaction, SpecId,
};
use sov_evm_test_utils::{PrecompileTester, SolCall};
use sov_modules_api::capabilities::TransactionAuthenticator;
use sov_modules_api::configurable_spec::ConfigurableSpec;
use sov_modules_api::macros::config_value;
use sov_modules_api::sov_universal_wallet::schema::UniversalWallet;
use sov_modules_api::transaction::Transaction;
use sov_modules_api::{RawTx, Spec, TxState};
use sov_rollup_interface::da::Time;
use sov_rollup_interface::execution_mode::Native;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{
    generate_runtime, AsUser, MockDaSpec, MockZkvm, MockZkvmCryptoSpec, TestStorage, TestUser,
    TransactionTestCase, TransactionType, TEST_DEFAULT_USER_BALANCE,
};

type PrecompileTestSpec = ConfigurableSpec<
    MockDaSpec,
    MockZkvm,
    MockZkvm,
    MultiAddressEvm,
    Native,
    MockZkvmCryptoSpec,
    TestStorage,
>;

type S = PrecompileTestSpec;

const IDENTITY_PRECOMPILE: Address = address!("0000000000000000000000000000000000000004");
const TIMESTAMP_SECONDS: i64 = 1_234_567;
const TRANSFER_AMOUNT: Amount = Amount::new(123_456);

#[derive(Clone)]
struct CompositePrecompiles<S: Spec> {
    bank_balance: BankBalancePrecompile<S>,
    sequencing_timestamp: SequencingTimestampPrecompile<S>,
}

impl<S: Spec> Default for CompositePrecompiles<S> {
    fn default() -> Self {
        Self {
            bank_balance: BankBalancePrecompile::default(),
            sequencing_timestamp: SequencingTimestampPrecompile::default(),
        }
    }
}

impl<S> EvmPrecompileSet<S> for CompositePrecompiles<S>
where
    S: Spec,
    S::Address: FromVmAddress<EthereumAddress>,
{
    fn addresses(&self) -> impl Iterator<Item = Address> {
        self.bank_balance
            .addresses()
            .chain(self.sequencing_timestamp.addresses())
    }

    fn execute<ST: TxState<S>>(
        &self,
        address: Address,
        input: &[u8],
        gas_limit: u64,
        env: &mut EvmPrecompileEnv<'_, S, ST>,
    ) -> Option<PrecompileResult> {
        self.bank_balance
            .execute(address, input, gas_limit, env)
            .or_else(|| {
                self.sequencing_timestamp
                    .execute(address, input, gas_limit, env)
            })
    }
}

macro_rules! define_runtime {
    ($module:ident, $runtime:ident, $runtime_call:ident, $evm_ty:ty) => {
        mod $module {
            use super::*;

            type TestEvm<S> = $evm_ty;

            generate_runtime! {
                name: $runtime,
                modules: [evm: TestEvm<S>],
                operating_mode: sov_modules_api::runtime::OperatingMode::Zk,
                minimal_genesis_config_type: sov_test_utils::runtime::genesis::optimistic::MinimalOptimisticGenesisConfig<S>,
                runtime_trait_impl_bounds: [S::Address: FromVmAddress<EthereumAddress>],
                kernel_type: sov_kernels::basic::BasicKernel<'a, S>,
                auth_type: sov_evm::EvmAuthenticator<S, Self>,
                auth_call_wrapper: |call| match call {
                    EvmAuthenticatorInput::Evm(call) => $runtime_call::Evm(call),
                    EvmAuthenticatorInput::Standard(call) => call,
                },
            }

            impl<S: Spec> sov_evm::EthereumAuthenticator<S> for $runtime<S>
            where
                S::Address: FromVmAddress<EthereumAddress>,
                Transaction<Self, S>: UniversalWallet,
            {
                fn add_ethereum_auth(
                    tx: RawTx,
                ) -> <Self::Auth as TransactionAuthenticator<S>>::Input {
                    EvmAuthenticatorInput::Evm(tx)
                }
            }

            pub type RT = $runtime<super::S>;

            pub fn setup() -> (TestRunner<RT, super::S>, EvmAccount, EvmAccount, TestUser<super::S>)
            {
                let caller = EvmAccount::generate();
                let balance_holder = EvmAccount::generate();
                let genesis_config = HighLevelOptimisticGenesisConfig::generate()
                    .add_accounts_with_default_balance(1);
                let bank_sender = genesis_config
                    .additional_accounts()
                    .first()
                    .expect("setup should create a bank sender")
                    .clone();

                let evm_config = EvmGenesisConfig {
                    accounts: vec![
                        AccountData::empty_with_address(caller.address()),
                        AccountData::empty_with_address(balance_holder.address()),
                    ],
                    chain_spec: EvmChainSpec {
                        limit_contract_code_size: None,
                        coinbase: Address::ZERO,
                        block_gas_limit: 1_000_000_000,
                        tx_gas_limit: Some(30_000_000),
                        hardforks: vec![(0, SpecId::CANCUN)],
                    },
                    contract_creation_policy: ContractCreationPolicy::Everyone,
                    initial_base_fee: 0,
                    genesis_timestamp: 0,
                    admin: bank_sender.address(),
                };

                let mut genesis =
                    GenesisConfig::from_minimal_config(genesis_config.into(), evm_config);
                if let Some(config) = genesis.bank.gas_token_config.as_mut() {
                    config.address_and_balances.push((
                        MultiAddress::Vm(EthereumAddress::from(caller.address())),
                        TEST_DEFAULT_USER_BALANCE,
                    ));
                    config.address_and_balances.push((
                        MultiAddress::Vm(EthereumAddress::from(balance_holder.address())),
                        TEST_DEFAULT_USER_BALANCE,
                    ));
                }

                let runner =
                    TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default());
                (runner, caller, balance_holder, bank_sender)
            }
        }
    };
}

define_runtime!(default_runtime, DefaultRuntime, DefaultRuntimeCall, Evm<S>);
define_runtime!(
    bank_runtime,
    BankRuntime,
    BankRuntimeCall,
    Evm<S, BankBalancePrecompile<S>>
);
define_runtime!(
    timestamp_runtime,
    TimestampRuntime,
    TimestampRuntimeCall,
    Evm<S, SequencingTimestampPrecompile<S>>
);
define_runtime!(
    composite_runtime,
    CompositeRuntime,
    CompositeRuntimeCall,
    Evm<S, CompositePrecompiles<S>>
);

macro_rules! evm_tx {
    ($module:ident, $nonce:expr, $caller:expr, $to:expr, $input:expr, $gas_limit:expr) => {{
        let raw_tx = create_raw_evm_tx($nonce, &$caller, $to, $input, $gas_limit);
        TransactionType::<$module::RT, S>::PreAuthenticated($module::RT::encode_with_ethereum_auth(
            raw_tx,
        ))
    }};
}

macro_rules! deploy_tester {
    ($module:ident, $runner:expr, $caller:expr) => {{
        let tester = $caller.address().create(0);
        $runner.execute_transaction(TransactionTestCase {
            input: evm_tx!(
                $module,
                0,
                $caller,
                TxKind::Create,
                Bytes::from(PrecompileTester::BYTECODE.to_vec()),
                3_000_000
            ),
            assert: Box::new(|ctx, state| {
                assert!(ctx.tx_receipt.is_successful());
                let receipt = Evm::<S>::default()
                    .receipt(0, state)
                    .expect("deployment should have an EVM receipt");
                assert!(receipt.0.receipt.success);
            }),
        });
        tester
    }};
}

macro_rules! assert_precompile_result {
    (
        $module:ident,
        $runner:expr,
        $caller:expr,
        $tester:expr,
        $nonce:expr,
        $precompile:expr,
        $input:expr,
        $expected_output:expr
    ) => {{
        let call = PrecompileTester::assertPrecompileResultCall {
            precompile: $precompile,
            input: $input,
            expectedOutput: $expected_output,
        };
        let nonce = $nonce;

        $runner.execute_transaction(TransactionTestCase {
            input: evm_tx!(
                $module,
                nonce,
                $caller,
                TxKind::Call($tester),
                Bytes::from(call.abi_encode()),
                1_000_000
            ),
            assert: Box::new(move |ctx, state| {
                assert!(ctx.tx_receipt.is_successful());
                let receipt = Evm::<S>::default()
                    .receipt(nonce, state)
                    .expect("precompile assertion call should have an EVM receipt");
                assert!(
                    receipt.0.receipt.success,
                    "precompile assertion call reverted at EVM receipt index {nonce}"
                );
            }),
        });
    }};
}

macro_rules! assert_identity_precompile {
    ($module:ident, $runner:expr, $caller:expr, $tester:expr, $nonce:expr) => {{
        let input = Bytes::from_static(b"sov-identity");
        assert_precompile_result!(
            $module,
            $runner,
            $caller,
            $tester,
            $nonce,
            IDENTITY_PRECOMPILE,
            input.clone(),
            input
        );
    }};
}

macro_rules! assert_empty_precompile {
    ($module:ident, $runner:expr, $caller:expr, $tester:expr, $nonce:expr, $precompile:expr, $input:expr $(,)?) => {
        assert_precompile_result!(
            $module,
            $runner,
            $caller,
            $tester,
            $nonce,
            $precompile,
            $input,
            Bytes::new()
        );
    };
}

macro_rules! assert_bank_balance_precompile {
    ($module:ident, $runner:expr, $caller:expr, $tester:expr, $nonce:expr, $balance_holder:expr, $expected_balance:expr $(,)?) => {
        assert_precompile_result!(
            $module,
            $runner,
            $caller,
            $tester,
            $nonce,
            BANK_BALANCE_PRECOMPILE_ADDRESS,
            bank_input($balance_holder),
            u256_bytes(($expected_balance).0)
        );
    };
}

macro_rules! assert_timestamp_precompile {
    ($module:ident, $runner:expr, $caller:expr, $tester:expr, $nonce:expr) => {
        assert_precompile_result!(
            $module,
            $runner,
            $caller,
            $tester,
            $nonce,
            SEQUENCING_TIMESTAMP_PRECOMPILE_ADDRESS,
            Bytes::new(),
            u256_bytes((TIMESTAMP_SECONDS as u128) * 1_000_000_000)
        );
    };
}

#[test]
fn default_evm_keeps_eth_precompiles_and_custom_addresses_are_empty() {
    let (mut runner, caller, balance_holder, _) = default_runtime::setup();
    let tester = deploy_tester!(default_runtime, runner, caller);

    assert_identity_precompile!(default_runtime, runner, caller, tester, 1);
    assert_empty_precompile!(
        default_runtime,
        runner,
        caller,
        tester,
        2,
        BANK_BALANCE_PRECOMPILE_ADDRESS,
        bank_input(balance_holder.address()),
    );
    assert_empty_precompile!(
        default_runtime,
        runner,
        caller,
        tester,
        3,
        SEQUENCING_TIMESTAMP_PRECOMPILE_ADDRESS,
        Bytes::new(),
    );
}

#[test]
fn bank_precompile_runtime_enables_only_bank_precompile() {
    let (mut runner, caller, balance_holder, _) = bank_runtime::setup();
    let tester = deploy_tester!(bank_runtime, runner, caller);

    assert_identity_precompile!(bank_runtime, runner, caller, tester, 1);
    assert_bank_balance_precompile!(
        bank_runtime,
        runner,
        caller,
        tester,
        2,
        balance_holder.address(),
        TEST_DEFAULT_USER_BALANCE,
    );
    assert_empty_precompile!(
        bank_runtime,
        runner,
        caller,
        tester,
        3,
        SEQUENCING_TIMESTAMP_PRECOMPILE_ADDRESS,
        Bytes::new(),
    );
}

#[test]
fn timestamp_precompile_runtime_enables_only_timestamp_precompile() {
    let (mut runner, caller, balance_holder, _) = timestamp_runtime::setup();
    runner.config.freeze_time = Some(Time::from_secs(TIMESTAMP_SECONDS));
    let tester = deploy_tester!(timestamp_runtime, runner, caller);

    assert_identity_precompile!(timestamp_runtime, runner, caller, tester, 1);
    assert_timestamp_precompile!(timestamp_runtime, runner, caller, tester, 2);
    assert_empty_precompile!(
        timestamp_runtime,
        runner,
        caller,
        tester,
        3,
        BANK_BALANCE_PRECOMPILE_ADDRESS,
        bank_input(balance_holder.address()),
    );
}

#[test]
fn bank_precompile_reflects_bank_transfer() {
    let (mut runner, caller, balance_holder, bank_sender) = bank_runtime::setup();
    let tester = deploy_tester!(bank_runtime, runner, caller);

    assert_bank_balance_precompile!(
        bank_runtime,
        runner,
        caller,
        tester,
        1,
        balance_holder.address(),
        TEST_DEFAULT_USER_BALANCE,
    );

    let recipient =
        <S as Spec>::Address::from_vm_address(EthereumAddress::from(balance_holder.address()));
    runner.execute_transaction(TransactionTestCase {
        input: bank_sender.create_plain_message::<bank_runtime::RT, sov_bank::Bank<S>>(
            sov_bank::CallMessage::Transfer {
                to: recipient,
                coins: Coins {
                    amount: TRANSFER_AMOUNT,
                    token_id: config_gas_token_id(),
                },
            },
        ),
        assert: Box::new(|ctx, _state| {
            assert!(ctx.tx_receipt.is_successful());
        }),
    });

    assert_bank_balance_precompile!(
        bank_runtime,
        runner,
        caller,
        tester,
        2,
        balance_holder.address(),
        TEST_DEFAULT_USER_BALANCE
            .checked_add(TRANSFER_AMOUNT)
            .expect("test balance should not overflow"),
    );
}

#[test]
fn composite_precompile_runtime_includes_both_custom_precompiles() {
    let (mut runner, caller, balance_holder, _) = composite_runtime::setup();
    runner.config.freeze_time = Some(Time::from_secs(TIMESTAMP_SECONDS));
    let tester = deploy_tester!(composite_runtime, runner, caller);

    assert_identity_precompile!(composite_runtime, runner, caller, tester, 1);
    assert_bank_balance_precompile!(
        composite_runtime,
        runner,
        caller,
        tester,
        2,
        balance_holder.address(),
        TEST_DEFAULT_USER_BALANCE,
    );
    assert_timestamp_precompile!(composite_runtime, runner, caller, tester, 3);
}

fn create_raw_evm_tx(
    nonce: u64,
    caller: &EvmAccount,
    to: TxKind,
    input: Bytes,
    gas_limit: u64,
) -> RawTx {
    let tx = TxEip1559 {
        to,
        input,
        nonce,
        gas_limit,
        max_fee_per_gas: MIN_PROTOCOL_BASE_FEE as u128 * 2,
        chain_id: config_value!("CHAIN_ID"),
        ..Default::default()
    };

    let signer = Signer::new(caller.secret_key());
    let signed_tx = signer
        .sign_transaction(TypedTransaction::Eip1559(tx))
        .unwrap();
    let rlp = signed_tx.encoded_2718();

    RawTx {
        data: borsh::to_vec(&RlpEvmTransaction { rlp }).unwrap(),
    }
}

fn bank_input(address: Address) -> Bytes {
    Bytes::copy_from_slice(address.as_slice())
}

fn u256_bytes(value: u128) -> Bytes {
    Bytes::copy_from_slice(&U256::from(value).to_be_bytes::<32>())
}
