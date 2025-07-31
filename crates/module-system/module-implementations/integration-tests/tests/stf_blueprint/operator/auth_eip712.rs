use alloy_primitives::address;
use alloy_sol_types::{eip712_domain, sol, Eip712Domain, SolStruct};
use sov_address::{EthereumAddress, EvmCryptoSpec};
use sov_evm::Eip712Authenticator;
use sov_mock_da::{MockBlob, MockDaSpec};
use sov_mock_zkvm::MockZkvm;
use sov_modules_api::capabilities::TransactionAuthenticator;
use sov_modules_api::configurable_spec::ConfigurableSpec;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::macros::config_value;
use sov_modules_api::transaction::{PriorityFeeBips, Transaction, UnsignedTransaction};
use sov_modules_api::{FullyBakedTx, PrivateKey, RawTx, Runtime, Spec};
use sov_rollup_interface::da::RelevantBlobs;
use sov_rollup_interface::stf::{TxEffect, TxReceiptContents};
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::{TestRunner, ValueSetter};
use sov_test_utils::{generate_runtime, EncodeCall, TestUser, TEST_DEFAULT_MAX_FEE};
use sov_value_setter::CallMessage;

type TestSpec =
    ConfigurableSpec<MockDaSpec, MockZkvm, MockZkvm, EthereumAddress, Native, EvmCryptoSpec>;
type S = TestSpec;
generate_runtime! {
    name: TestRuntime,
    modules: [value_setter: ValueSetter<S>],
    operating_mode: sov_modules_api::OperatingMode::Optimistic,
    minimal_genesis_config_type: sov_test_utils::runtime::genesis::optimistic::MinimalOptimisticGenesisConfig<S>,
    runtime_trait_impl_bounds: [S: ::sov_modules_api::Spec<CryptoSpec = EvmCryptoSpec>],
    kernel_type: sov_kernels::basic::BasicKernel<'a, S>,
    auth_type: Eip712Authenticator<S, TestRuntime<S>>,
    auth_call_wrapper: |call| call,
}
type RT = TestRuntime<S>;

fn setup() -> (TestRunner<RT, S>, TestUser<S>) {
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(2);

    let accounts = genesis_config.additional_accounts();
    let admin = accounts.first().unwrap().clone();

    let module_config = sov_value_setter::ValueSetterConfig {
        admin: admin.address(),
    };

    let genesis = GenesisConfig::from_minimal_config(genesis_config.clone().into(), module_config);
    let runner = TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default());
    (runner, admin)
}

const DOMAIN: Eip712Domain = eip712_domain! {
    name: "CallMessage",
    version: "1",
    chain_id: 4321,
    verifying_contract: address!("0000000000000000000000000000000000000000"),
};

sol! {
    #[derive(Debug)]
    struct TxDetails {
        uint64 chain_id;
    }
}

pub fn create_tx<S: Spec, RT: Runtime<S>>(
    nonce: u64,
    max_priority_fee_bips: PriorityFeeBips,
    signer: &TestUser<S>,
    chain_id: u64,
    message: RT::Decodable,
) -> Transaction<RT, S> {
    let utx = UnsignedTransaction::new(
        message,
        chain_id,
        max_priority_fee_bips,
        TEST_DEFAULT_MAX_FEE,
        nonce,
        None,
    );

    let tx_details = TxDetails { chain_id };
    let hash = tx_details.eip712_signing_hash(&DOMAIN);
    let pk = signer.private_key();

    let signature = pk.sign(hash.as_slice());
    let signed_tx = utx.to_signed_tx(pk.pub_key(), signature);
    signed_tx
}

pub fn encode_message<S: Spec, RT: Runtime<S> + EncodeCall<ValueSetter<S>>>() -> RT::Decodable {
    let msg = CallMessage::SetValue {
        value: 0,
        gas: None,
    };
    RT::to_decodable(msg)
}

pub fn encode<S: Spec, RT: Runtime<S>>(tx: Transaction<RT, S>) -> FullyBakedTx {
    let raw_tx = RawTx::new(borsh::to_vec(&tx).unwrap());
    RT::Auth::encode_with_standard_auth(raw_tx)
}

fn execute_tx(
    mut runner: TestRunner<RT, S>,
    tx: Transaction<RT, S>,
) -> TxEffect<impl TxReceiptContents> {
    let serialized_tx = encode(tx);
    let txs: Vec<FullyBakedTx> = vec![serialized_tx];
    let blob = borsh::to_vec(&txs).unwrap();
    let blob = MockBlob::new_with_hash(blob, runner.config.sequencer_da_address);

    let blobs = RelevantBlobs {
        proof_blobs: Default::default(),
        batch_blobs: vec![blob],
    };

    let (receipts, _) = runner.execute(blobs);
    let receipt = receipts.last_tx_receipt().receipt.clone();
    receipt
}

#[test]
fn test_eip712() {
    let (runner, admin) = setup();
    let tx = create_tx::<_, RT>(
        0,
        PriorityFeeBips::ZERO,
        &admin,
        config_value!("CHAIN_ID"),
        encode_message::<_, RT>(),
    );

    let receipt = execute_tx(runner, tx);
    dbg!(receipt);
}
