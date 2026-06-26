//! Rollup-level `ecrecover` benchmark.
//!
//! Deploys the `EcrecoverBatch` contract and times the STF `simulate` of a `recoverBatch(N)`
//! transaction — N **distinct** ECDSA signatures verified via the `ecrecover` precompile
//! (`0x01`) — through the full sov-evm execution path (EVM interpreter + inspector + sov
//! wrapper + state-root computation). `simulate` runs apply_slot **without** committing to the
//! on-disk NOMT DB (the test `SimpleStorageManager` / `NomtProverStorage` commits per slot,
//! which is slow on macOS — ~50ms/slot — and irrelevant to the EVM cost we want to isolate;
//! cf. `NonCommitingStorageManager`, `sov-test-utils/src/storage.rs:256`). The **slope of time
//! vs N is the per-ecrecover cost at the rollup level**, which is what dominated relay resync
//! (settlement txns verify ~14–40 sigs each).
//!
//! Showing the signal: run on the current build (native enables `revm/secp256k1`, the C
//! libsecp256k1 backend) and again with that one-line feature reverted (pure-Rust k256):
//!
//!   cargo bench -p sov-evm --bench ecrecover
//!   git stash   # drop the `revm/secp256k1` line in sov-evm/Cargo.toml, then bench again
//!
//! The slope drops ~5× (measured raw: k256 ≈117µs vs libsecp256k1 ≈24µs per ecrecover).

use alloy_consensus::crypto::secp256k1::public_key_to_address;
use alloy_consensus::{TxEip1559, TypedTransaction};
use alloy_eips::eip1559::MIN_PROTOCOL_BASE_FEE;
use alloy_primitives::{Address, Bytes, TxKind, B256};
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use k256::ecdsa::{RecoveryId, Signature, SigningKey};
use secp256k1::{PublicKey, SecretKey};
use sov_address::{EthereumAddress, FromVmAddress, MultiAddress, MultiAddressEvm};
use sov_eth_dev_signer::Signer;
use sov_evm::{
    AccountData, ContractCreationPolicy, EthereumAuthenticator, Evm, EvmAuthenticatorInput,
    EvmChainSpec, EvmGenesisConfig, RlpEvmTransaction, SpecId, TransactionSigned,
};
use sov_evm_test_utils::EcrecoverBatch;
use sov_modules_api::capabilities::TransactionAuthenticator;
use sov_modules_api::configurable_spec::ConfigurableSpec;
use sov_modules_api::macros::config_value;
use sov_modules_api::sov_universal_wallet::schema::UniversalWallet;
use sov_modules_api::transaction::Transaction;
use sov_modules_api::{RawTx, Spec};
use sov_rollup_interface::execution_mode::Native;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{
    generate_runtime, MockDaSpec, MockZkvm, MockZkvmCryptoSpec, TestStorage, TransactionTestCase,
    TransactionType, TEST_DEFAULT_USER_BALANCE,
};

// ---------------------------------------------------------------------------
// Self-contained EVM harness (mirrors sov-evm/tests/integration/{runtime,helpers}.rs).
// ---------------------------------------------------------------------------

type EvmTestSpec = ConfigurableSpec<
    MockDaSpec,
    MockZkvm,
    MockZkvm,
    MultiAddressEvm,
    Native,
    MockZkvmCryptoSpec,
    TestStorage,
>;

generate_runtime! {
    name: TestRuntime,
    modules: [evm: Evm<S>],
    operating_mode: OperatingMode::Zk,
    minimal_genesis_config_type: sov_test_utils::runtime::genesis::optimistic::MinimalOptimisticGenesisConfig<S>,
    runtime_trait_impl_bounds: [S::Address: FromVmAddress<EthereumAddress>],
    kernel_type: sov_kernels::basic::BasicKernel<'a, S>,
    auth_type: sov_evm::EvmAuthenticator<S, Self>,
    auth_call_wrapper: |call| match call {
        EvmAuthenticatorInput::Evm(call) => TestRuntimeCall::Evm(call),
        EvmAuthenticatorInput::Standard(call) => call,
    },
}

impl<S: Spec> sov_evm::EthereumAuthenticator<S> for TestRuntime<S>
where
    S::Address: FromVmAddress<EthereumAddress>,
    Transaction<Self, S>: UniversalWallet,
{
    fn add_ethereum_auth(tx: RawTx) -> <Self::Auth as TransactionAuthenticator<S>>::Input {
        EvmAuthenticatorInput::Evm(tx)
    }
}

type S = EvmTestSpec;
type RT = TestRuntime<EvmTestSpec>;

/// An EVM account that can sign transactions (mirror of the integration-test helper).
struct EvmAccount(SecretKey);

impl EvmAccount {
    fn from_seed(seed: u8) -> Self {
        Self(SecretKey::from_slice(&[seed; 32]).expect("valid secret key"))
    }

    fn address(&self) -> Address {
        public_key_to_address(PublicKey::from_secret_key(secp256k1::SECP256K1, &self.0))
    }

    fn sign(&self, tx: TypedTransaction) -> RlpEvmTransaction {
        use alloy_eips::eip2718::Encodable2718;
        let signer = Signer::new(self.0);
        let signed: TransactionSigned = signer.sign_transaction(tx).unwrap();
        RlpEvmTransaction {
            rlp: signed.encoded_2718(),
        }
    }
}

/// Genesis with one funded EVM account.
fn setup() -> (TestRunner<RT, S>, EvmAccount) {
    let account = EvmAccount::from_seed(0x11);

    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);
    let admin = genesis_config
        .additional_accounts()
        .first()
        .unwrap()
        .clone();

    let evm_config = EvmGenesisConfig {
        accounts: vec![AccountData::empty_with_address(account.address())],
        chain_spec: EvmChainSpec {
            limit_contract_code_size: None,
            coinbase: Address::ZERO,
            block_gas_limit: 1_000_000_000,
            tx_gas_limit: Some(30_000_000),
            hardforks: vec![(0, SpecId::CANCUN)],
        },
        contract_creation_policy: ContractCreationPolicy::Everyone,
        enabled_custom_precompiles: Default::default(),
        initial_base_fee: 0,
        genesis_timestamp: 0,
        admin: admin.address(),
    };

    let mut genesis = GenesisConfig::from_minimal_config(genesis_config.into(), evm_config);
    if let Some(c) = genesis.bank.gas_token_config.as_mut() {
        c.address_and_balances.push((
            MultiAddress::Vm(EthereumAddress::from(account.address())),
            TEST_DEFAULT_USER_BALANCE,
        ));
    }

    let runner =
        TestRunner::new_with_genesis(genesis.into_genesis_params(), TestRuntime::default());
    (runner, account)
}

fn build_tx(account: &EvmAccount, tx: TxEip1559) -> TransactionType<RT, S> {
    let tx = TxEip1559 {
        gas_limit: 30_000_000,
        max_fee_per_gas: MIN_PROTOCOL_BASE_FEE as u128 * 2,
        chain_id: config_value!("CHAIN_ID"),
        ..tx
    };
    let signed = account.sign(TypedTransaction::Eip1559(tx));
    let raw_tx = RawTx {
        data: borsh::to_vec(&signed).unwrap(),
    };
    TransactionType::PreAuthenticated(RT::encode_with_ethereum_auth(raw_tx))
}

fn deploy_tx(account: &EvmAccount, bytecode: Bytes) -> TransactionType<RT, S> {
    build_tx(
        account,
        TxEip1559 {
            input: bytecode,
            nonce: 0,
            ..Default::default()
        },
    )
}

fn call_tx(account: &EvmAccount, nonce: u64, to: Address, data: Bytes) -> TransactionType<RT, S> {
    build_tx(
        account,
        TxEip1559 {
            to: TxKind::Call(to),
            input: data,
            nonce,
            ..Default::default()
        },
    )
}

fn exec(runner: &mut TestRunner<RT, S>, tx: TransactionType<RT, S>) {
    runner.execute_transaction(TransactionTestCase {
        input: tx,
        assert: Box::new(|_, _| {}),
    });
}

// ---------------------------------------------------------------------------
// N distinct valid signatures for the ecrecover precompile.
// ---------------------------------------------------------------------------

fn distinct_signatures(n: usize) -> (Vec<B256>, Vec<u8>, Vec<B256>, Vec<B256>) {
    let sk = SigningKey::from_slice(&[0x4d; 32]).unwrap();
    let (mut hs, mut vs, mut rs, mut ss) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for i in 0..n {
        // Distinct digest per signature -> distinct (r, s).
        let mut digest = [0u8; 32];
        digest[..8].copy_from_slice(&((i as u64) + 1).to_be_bytes());
        let (sig, recid): (Signature, RecoveryId) = sk.sign_prehash_recoverable(&digest).unwrap();
        let b = sig.to_bytes();
        hs.push(B256::from(digest));
        vs.push(recid.to_byte() + 27);
        rs.push(B256::from_slice(&b[..32]));
        ss.push(B256::from_slice(&b[32..]));
    }
    (hs, vs, rs, ss)
}

// ---------------------------------------------------------------------------
// Benchmark: execute a recoverBatch(N) tx, swept over N.
// ---------------------------------------------------------------------------

const SIG_COUNTS: &[usize] = &[1, 20, 100, 500];

fn bench_rollup_ecrecover(c: &mut Criterion) {
    let mut group = c.benchmark_group("rollup_ecrecover");
    for &n in SIG_COUNTS {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |bencher, &n| {
            let (mut runner, account) = setup();
            let contract = EcrecoverBatch::default();
            let contract_addr = account.address().create(0);
            exec(&mut runner, deploy_tx(&account, contract.byte_code()));

            let (h, v, r, s) = distinct_signatures(n);
            let calldata = contract.recover_batch(h, v, r, s);
            let tx = call_tx(&account, 1, contract_addr, calldata);

            // `simulate` runs the full STF apply_slot (auth + EVM execution incl. the
            // ecrecover precompile + state-root computation) but does NOT commit to the
            // on-disk NOMT DB, isolating the rollup EVM cost from the ~50ms/slot NOMT
            // disk-commit floor (TestStorage = NomtProverStorage in a tempdir). The
            // contract is already committed (deploy above); simulate does not advance the
            // nonce, so the same tx is re-simulated each iteration.
            bencher.iter(|| {
                black_box(runner.simulate(tx.clone()));
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_rollup_ecrecover);
criterion_main!(benches);
