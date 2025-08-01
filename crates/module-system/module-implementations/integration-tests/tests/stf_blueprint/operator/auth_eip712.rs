use sov_address::{EthereumAddress, EvmCryptoSpec};
use sov_mock_da::{MockBlob, MockDaSpec};
use sov_mock_zkvm::MockZkvm;
use sov_modules_api::capabilities::TransactionAuthenticator;
use sov_modules_api::configurable_spec::ConfigurableSpec;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::macros::config_value;
use sov_modules_api::transaction::{PriorityFeeBips, Transaction, UnsignedTransaction};
use sov_modules_api::{DispatchCall, FullyBakedTx, RawTx, Runtime, Spec};
use sov_rollup_interface::da::RelevantBlobs;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::{TestRunner, ValueSetter};
use sov_test_utils::{generate_optimistic_runtime, EncodeCall, TestUser, TEST_DEFAULT_MAX_FEE};
use sov_value_setter::CallMessage;

type TestSpec =
    ConfigurableSpec<MockDaSpec, MockZkvm, MockZkvm, EthereumAddress, Native, EvmCryptoSpec>;
type S = TestSpec;
generate_optimistic_runtime!(TestRuntime <= value_setter: ValueSetter<S>);
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

pub fn create_tx_valid<S: Spec, RT: Runtime<S>>(
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

    Transaction::<RT, S>::new_signed_tx(signer.private_key(), &RT::CHAIN_HASH, utx)
}

pub fn encode_message<S: Spec, RT: Runtime<S> + EncodeCall<ValueSetter<S>>>(
) -> <RT as DispatchCall>::Decodable {
    <RT as EncodeCall<ValueSetter<S>>>::to_decodable(CallMessage::SetValue {
        value: 8,
        gas: None,
    })
}

pub fn encode<S: Spec, RT: Runtime<S>>(tx: Transaction<RT, S>) -> FullyBakedTx {
    <RT as Runtime<S>>::Auth::encode_with_standard_auth(RawTx::new(borsh::to_vec(&tx).unwrap()))
}

#[test]
fn test_eip712() {
    let (mut runner, admin) = setup();

    let tx = create_tx_valid::<_, RT>(
        0,
        PriorityFeeBips::ZERO,
        &admin,
        config_value!("CHAIN_ID"),
        encode_message::<_, RT>(),
    );
    let serialized_tx = encode(tx);
    let txs: Vec<FullyBakedTx> = vec![serialized_tx];

    let blob = borsh::to_vec(&txs).unwrap();
    let blob = MockBlob::new_with_hash(blob, runner.config.sequencer_da_address);

    let blobs = RelevantBlobs {
        proof_blobs: Default::default(),
        batch_blobs: vec![blob],
    };

    let _ = runner.execute(blobs);
}
