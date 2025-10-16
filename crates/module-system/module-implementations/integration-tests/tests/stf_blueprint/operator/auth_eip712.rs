use sov_accounts::{Accounts, CallMessage as AccountsCallMessage};
use sov_address::{EthereumAddress, EvmCryptoSpec};
use sov_eip712_auth::{
    Eip712Authenticator, Eip712AuthenticatorInput, Eip712AuthenticatorTrait, SchemaProvider,
};
use sov_mock_da::{MockBlob, MockDaSpec};
use sov_mock_zkvm::MockZkvm;
use sov_modules_api::capabilities::{TransactionAuthenticator, UniquenessData};
use sov_modules_api::configurable_spec::ConfigurableSpec;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::macros::config_value;
use sov_modules_api::transaction::PubKeyAndSignature;
use sov_modules_api::transaction::TxDetails;
use sov_modules_api::transaction::{PriorityFeeBips, Transaction, UnsignedTransaction};
use sov_modules_api::CryptoSpec;
use sov_modules_api::Multisig;
use sov_modules_api::SkippedTxContents;
use sov_modules_api::{FullyBakedTx, PrivateKey, RawTx, Runtime, Spec, SuccessfulTxContents};
use sov_rollup_interface::da::RelevantBlobs;
use sov_rollup_interface::stf::{TxEffect, TxReceiptContents};
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::{TestRunner, ValueSetter};
use sov_test_utils::TransactionTestCase;
use sov_test_utils::{generate_runtime, EncodeCall, TestUser, TEST_DEFAULT_MAX_FEE};
use sov_value_setter::CallMessage;

type TestSpec =
    ConfigurableSpec<MockDaSpec, MockZkvm, MockZkvm, EthereumAddress, Native, EvmCryptoSpec>;
type S = TestSpec;

type TestPrivateKey = <<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey;

// The Eip712Authenticator requires access to the UniversalWallet schema, which must be generated
// from a runtime. But the authenticator must be passed to the runtime to construct it.
//
// Normally this is handled with build-scripts; for this test, we instead create a dummy schema and
// build a dummy runtime with that, and we can then build the real schema from the RuntimeCall in
// this runtime (which will be identical to the real runtime).
mod schema_generation {
    use sov_modules_api::runtime::get_runtime_schema;
    use sov_modules_api::sov_universal_wallet::schema::Schema;
    use std::sync::OnceLock;

    use super::{
        generate_runtime, Eip712Authenticator, EvmCryptoSpec, SchemaProvider, ValueSetter, S,
    };

    /// Dummy schema provider for the schema-generation runtime
    pub struct DummySchemaProvider;
    impl SchemaProvider for DummySchemaProvider {
        const SCHEMA_BORSH: &'static [u8] = &[];

        // This should never be called since we only use this runtime for schema generation
        fn get_schema() -> &'static Schema {
            panic!("DummySchemaProvider::get_schema() should never be called")
        }
    }

    // Runtime used only for generating the schema
    generate_runtime! {
        name: SchemaGenRuntime,
        modules: [value_setter: ValueSetter<S>],
        operating_mode: sov_modules_api::OperatingMode::Optimistic,
        minimal_genesis_config_type: sov_test_utils::runtime::genesis::optimistic::MinimalOptimisticGenesisConfig<S>,
        runtime_trait_impl_bounds: [S: ::sov_modules_api::Spec<CryptoSpec = EvmCryptoSpec>],
        kernel_type: sov_kernels::basic::BasicKernel<'a, S>,
        auth_type: Eip712Authenticator<S, SchemaGenRuntime<S>, DummySchemaProvider>,
        auth_call_wrapper: |call| call,
    }

    /// Get the test runtime schema, generating it once and caching it
    pub fn get_test_schema() -> &'static Schema {
        static SCHEMA: OnceLock<Schema> = OnceLock::new();

        SCHEMA.get_or_init(|| {
            get_runtime_schema::<S, SchemaGenRuntime<S>>()
                .expect("Failed to generate test runtime schema")
        })
    }
}

/// The real schema provider for tests that uses the runtime-generated schema
pub struct TestSchemaProvider;
impl SchemaProvider for TestSchemaProvider {
    const SCHEMA_BORSH: &'static [u8] = &[]; // Not used since we override get_schema()

    /// Override the default implementation to use our runtime-generated schema
    fn get_schema() -> &'static sov_modules_api::sov_universal_wallet::schema::Schema {
        schema_generation::get_test_schema()
    }
}

generate_runtime! {
    name: TestRuntime,
    modules: [value_setter: ValueSetter<S>],
    operating_mode: sov_modules_api::OperatingMode::Optimistic,
    minimal_genesis_config_type: sov_test_utils::runtime::genesis::optimistic::MinimalOptimisticGenesisConfig<S>,
    runtime_trait_impl_bounds: [S: ::sov_modules_api::Spec<CryptoSpec = EvmCryptoSpec>],
    kernel_type: sov_kernels::basic::BasicKernel<'a, S>,
    auth_type: Eip712Authenticator<S, TestRuntime<S>, TestSchemaProvider>,
    auth_call_wrapper: |call| call,
}

impl Eip712AuthenticatorTrait<S> for TestRuntime<S> {
    fn add_eip712_auth(tx: RawTx) -> <Self::Auth as TransactionAuthenticator<S>>::Input {
        Eip712AuthenticatorInput::Eip712(tx)
    }
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

pub fn create_utx<S: Spec, RT: Runtime<S>>(message: RT::Decodable) -> UnsignedTransaction<RT, S> {
    let details = TxDetails {
        max_priority_fee_bips: PriorityFeeBips::ZERO,
        max_fee: TEST_DEFAULT_MAX_FEE,
        gas_limit: None,
        chain_id: config_value!("CHAIN_ID"),
    };
    UnsignedTransaction::new_with_details(message, UniquenessData::Generation(0), details)
}

pub fn sign_utx_in_place<S: Spec, RT: Runtime<S>>(
    utx: &UnsignedTransaction<RT, S>,
    private_key: &<S::CryptoSpec as CryptoSpec>::PrivateKey,
) -> <S::CryptoSpec as CryptoSpec>::Signature {
    let schema = TestSchemaProvider::get_schema();

    let transaction_type_index = schema
        .rollup_expected_index(
            sov_modules_api::sov_universal_wallet::schema::RollupRoots::UnsignedTransaction,
        )
        .unwrap();

    let utx_bytes = borsh::to_vec(&utx).expect("Failed to serialize unsigned transaction");
    let eip712_signing_data = schema
        .eip712_signing_digest(transaction_type_index, &utx_bytes)
        .expect("Failed to calculate EIP712 hash");

    private_key.sign(&eip712_signing_data)
}

pub fn sign_utx<S: Spec, RT: Runtime<S>>(
    utx: UnsignedTransaction<RT, S>,
    signer: &TestUser<S>,
) -> Transaction<RT, S> {
    let signature = sign_utx_in_place(&utx, signer.private_key());
    utx.to_signed_tx(signer.private_key().pub_key(), signature)
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

pub fn encode<S: Spec, RT: Runtime<S> + Eip712AuthenticatorTrait<S>>(
    tx: Transaction<RT, S>,
) -> FullyBakedTx {
    let raw_tx = RawTx::new(borsh::to_vec(&tx).unwrap());
    <RT as Eip712AuthenticatorTrait<S>>::encode_with_eip712_auth(raw_tx)
}

fn execute_tx(
    runner: &mut TestRunner<RT, S>,
    tx: Transaction<RT, S>,
) -> TxEffect<
    impl TxReceiptContents<Successful = SuccessfulTxContents<S>, Skipped = SkippedTxContents<S>>,
> {
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
    let (mut runner, admin) = setup();
    let call = encode_message::<_, RT>();
    let tx = create_tx::<_, RT>(call, &admin);

    let receipt = execute_tx(&mut runner, tx);
    let TxEffect::Successful(SuccessfulTxContents { .. }) = receipt else {
        panic!("Expected transaction to succeed, got: {receipt:?}");
    };
}

#[test]
fn test_multisig_signature_verification() {
    use sov_test_utils::AsUser;
    let (mut runner, admin) = setup();

    // First, create and register a multisig
    let multisig_keys = vec![
        TestPrivateKey::generate(),
        TestPrivateKey::generate(),
        TestPrivateKey::generate(),
    ];

    // Create the multisig and register it
    let multisig = Multisig::new(2, multisig_keys.iter().map(|k| k.pub_key()).collect());
    let multisig_credential_id =
        multisig.credential_id::<<<S as Spec>::CryptoSpec as CryptoSpec>::Hasher>();
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Accounts<S>>(
            AccountsCallMessage::InsertCredentialId(multisig_credential_id),
        ),
        assert: Box::new(move |result, _state| {
            assert!(result.tx_receipt.is_successful());
        }),
    });

    // Create a multisig transaction
    let utx = create_utx::<S, RT>(encode_message::<_, RT>());
    let mut signatures = Vec::new();
    for key in multisig_keys.iter() {
        signatures.push(sign_utx_in_place(&utx, key));
    }
    // Generate a signature from a random private key that's not part of the multisig. We'll use this in some of the test cases.
    let random_private_key = TestPrivateKey::generate();
    let random_signature = sign_utx_in_place(&utx, &random_private_key);
    let tx = utx.to_multisig_tx(multisig);

    // Helper functions to assert the expected behavior of the transaction
    let assert_tx_success = |tx: Transaction<RT, S>, runner: &mut TestRunner<RT, S>| {
        let receipt = execute_tx(runner, tx);
        let TxEffect::Successful(SuccessfulTxContents { .. }) = receipt else {
            panic!("Expected transaction to succeed, got: {receipt:?}");
        };
    };
    let assert_tx_skipped =
        |tx: Transaction<RT, S>, runner: &mut TestRunner<RT, S>, reason: &'static str| {
            let receipt = execute_tx(runner, tx);
            let TxEffect::Skipped(SkippedTxContents { error, .. }) = receipt else {
                panic!("Expected transaction to be skipped, got: {receipt:?}");
            };

            assert!(
                error.to_string().contains(reason),
                "Expected error to contain {reason}, got: {error}"
            );
        };

    // A transaction with three valid signatures should succeed
    let tx_with_three_sigs = {
        let mut tx = tx.clone();
        for (key, signature) in multisig_keys.iter().zip(signatures.iter()) {
            tx.add_signature(signature.clone(), key.pub_key()).unwrap();
        }
        Transaction::<RT, S>::from(tx)
    };
    assert_tx_success(tx_with_three_sigs, &mut runner);

    // A transaction with only two valid signatures should succeed
    let tx_with_two_sigs = {
        let mut tx = tx.clone();
        for (key, signature) in multisig_keys.iter().zip(signatures.iter().take(2)) {
            tx.add_signature(signature.clone(), key.pub_key()).unwrap();
        }
        Transaction::<RT, S>::from(tx)
    };
    assert_tx_success(tx_with_two_sigs, &mut runner);

    // A transaction with only one valid signature should be skipped, since this is a 2/3 multisig.
    let tx_with_one_sig = {
        let mut tx = tx.clone();
        for (key, signature) in multisig_keys.iter().zip(signatures.iter().take(1)) {
            tx.add_signature(signature.clone(), key.pub_key()).unwrap();
        }
        Transaction::<RT, S>::from(tx)
    };
    assert_tx_skipped(
        tx_with_one_sig,
        &mut runner,
        "Not enough valid signatures. Required: 2, Got: 1",
    );

    // A transaction where one of the signatures isn't from this account should be skipped, since this changes the computed credential ID.
    let tx_with_random_sig = {
        let mut tx = tx.clone();
        tx.add_signature(signatures[0].clone(), multisig_keys[0].pub_key())
            .unwrap();
        tx.signatures
            .try_push(PubKeyAndSignature {
                signature: random_signature.clone(),
                pub_key: random_private_key.pub_key(),
            })
            .unwrap();
        Transaction::<RT, S>::from(tx)
    };
    // Since the random signature is not part of the multisig, this changes the computed credential ID yielding a gas error. If we were to add a paymaster,
    // The tx would succeed on a different account. In that case, this test case would need refinement to distinguish between the two cases.
    assert_tx_skipped(
        tx_with_random_sig,
        &mut runner,
        "Insufficient balance to pay for the transaction gas",
    );

    // A transaction with a duplicate signature should be skipped
    let tx_with_duplicate_sig = {
        let mut tx = tx.clone();
        tx.add_signature(signatures[0].clone(), multisig_keys[0].pub_key())
            .unwrap();
        tx.signatures
            .try_push(PubKeyAndSignature {
                signature: signatures[0].clone(),
                pub_key: multisig_keys[0].pub_key(),
            })
            .unwrap();
        Transaction::<RT, S>::from(tx)
    };
    assert_tx_skipped(
        tx_with_duplicate_sig,
        &mut runner,
        "is not part of the multisig or has already signed",
    );

    // A transaction with a bad signature should be skipped
    let tx_with_bad_sig = {
        let mut tx = tx.clone();
        tx.add_signature(signatures[0].clone(), multisig_keys[0].pub_key())
            .unwrap();
        tx.add_signature(
            multisig_keys[1].sign(&[1, 2, 3]),
            multisig_keys[1].pub_key(),
        )
        .unwrap();
        Transaction::<RT, S>::from(tx)
    };
    assert_tx_skipped(tx_with_bad_sig, &mut runner, "signature error");
}
