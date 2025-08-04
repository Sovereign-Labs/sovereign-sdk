use alloy_primitives::address;
use alloy_sol_types::{eip712_domain, Eip712Domain, SolStruct};
use sov_address::{EthereumAddress, EvmCryptoSpec};
use sov_evm::Eip712Authenticator;
use sov_mock_da::{MockBlob, MockDaSpec};
use sov_mock_zkvm::MockZkvm;
use sov_modules_api::capabilities::TransactionAuthenticator;
use sov_modules_api::configurable_spec::ConfigurableSpec;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::macros::config_value;
use sov_modules_api::transaction::TxDetails;
use sov_modules_api::transaction::{PriorityFeeBips, Transaction, UnsignedTransaction};
use sov_modules_api::{FullyBakedTx, PrivateKey, RawTx, Runtime, Spec, SuccessfulTxContents};
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
    name: "Transaction",
    version: "1",
    chain_id: 4321,
    verifying_contract: address!("0000000000000000000000000000000000000000"),
};

pub mod sol_struct {
    use alloy_sol_types::sol;

    sol! {
        #[derive(Debug)]
        struct TxDetails {
            uint64 chain_id;
        }
    }
}

pub fn create_utx<S: Spec, RT: Runtime<S>>(message: RT::Decodable) -> UnsignedTransaction<RT, S> {
    let details = TxDetails {
        max_priority_fee_bips: PriorityFeeBips::ZERO,
        max_fee: TEST_DEFAULT_MAX_FEE,
        gas_limit: None,
        chain_id: config_value!("CHAIN_ID"),
    };
    UnsignedTransaction::new_with_details(message, 0, details)
}

pub fn sign_utx<S: Spec, RT: Runtime<S>>(
    utx: UnsignedTransaction<RT, S>,
    signer: &TestUser<S>,
) -> Transaction<RT, S> {
    let hash = utx.details.as_sol_struct().eip712_signing_hash(&DOMAIN);
    let pk = signer.private_key();
    let signature = pk.sign(hash.as_slice());
    utx.to_signed_tx(pk.pub_key(), signature)
}

pub fn create_tx<S: Spec, RT: Runtime<S>>(
    message: RT::Decodable,
    signer: &TestUser<S>,
) -> Transaction<RT, S> {
    let utx = create_utx::<S, RT>(message);
    sign_utx::<S, RT>(utx, signer)
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
) -> TxEffect<impl TxReceiptContents<Successful = SuccessfulTxContents<S>>> {
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
fn correct_signature_is_accepted() {
    let (runner, admin) = setup();
    let call = encode_message::<_, RT>();
    let tx = create_tx::<_, RT>(call, &admin);

    let receipt = execute_tx(runner, tx);
    let TxEffect::Successful(SuccessfulTxContents { .. }) = receipt else {
        panic!("Expected transaction to succeed, got: {receipt:?}");
    };
}
