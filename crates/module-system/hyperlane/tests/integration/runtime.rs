use std::collections::HashMap;

use secp256k1::rand::rngs::OsRng;
use secp256k1::{Message, Secp256k1, SecretKey};
use sha3::Keccak256;
use sov_bank::Amount;
use sov_hyperlane_integration::igp::ExchangeRateAndGasPrice;
use sov_hyperlane_integration::test_recipient::{
    CallMessage as RecipientCallMessage, TestRecipient,
};
use sov_hyperlane_integration::{
    EthAddress, HyperlaneAddress, InterchainGasPaymaster, InterchainGasPaymasterCallMessage, Ism,
    Mailbox as RawMailbox, MerkleTreeHook, ValidatorSignature,
};
use sov_modules_api::gas::GasArray;
use sov_modules_api::{BasicGasMeter, Gas, HexHash, HexString, Spec};
use sov_test_utils::runtime::genesis::zk::config::HighLevelZkGenesisConfig;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{generate_runtime, AsUser, TestSpec, TestUser, TransactionTestCase};

use crate::igp::{default_gas_hashmap_to_safe_vec, oracle_data_hashmap_to_safe_vec};

pub type Mailbox<S> = RawMailbox<S, TestRecipient<S>>;
pub type S = TestSpec;
pub type RT = TestRuntime<S>;

generate_runtime! {
    name: TestRuntime,
    modules: [mailbox: Mailbox<S>, test_recipient: TestRecipient<S>, merkle_tree_hook: MerkleTreeHook<S>, interchain_gas_paymaster: InterchainGasPaymaster<S>],
    operating_mode: sov_modules_api::runtime::OperatingMode::Zk,
    minimal_genesis_config_type: sov_test_utils::runtime::genesis::zk::config::MinimalZkGenesisConfig<S>,
    runtime_trait_impl_bounds: [S::Address: HyperlaneAddress],
    kernel_type: sov_test_utils::runtime::BasicKernel<'a, S>,
    auth_type: sov_modules_api::capabilities::RollupAuthenticator<S, TestRuntime<S>>,
    auth_call_wrapper: |call| call,
}

#[allow(clippy::type_complexity)]
pub fn setup() -> (
    TestRunner<TestRuntime<S>, S>,
    TestUser<S>,
    TestUser<S>,
    TestUser<S>,
    TestUser<S>,
    TestUser<S>,
) {
    let genesis_config = HighLevelZkGenesisConfig::generate_with_additional_accounts(5);

    let admin_account = genesis_config.additional_accounts()[0].clone();
    let extra_acccount = genesis_config.additional_accounts()[1].clone();
    let relayer_account = genesis_config.additional_accounts()[2].clone();
    let beneficiary_account = genesis_config.additional_accounts()[4].clone();
    let user_account = genesis_config.additional_accounts()[3].clone();

    let genesis = GenesisConfig::from_minimal_config(genesis_config.into(), (), (), (), ());

    (
        TestRunner::new_with_genesis(genesis.into_genesis_params(), Default::default()),
        admin_account,
        extra_acccount,
        relayer_account,
        beneficiary_account,
        user_account,
    )
}

pub fn register_recipient(
    runner: &mut TestRunner<RT, S>,
    user: &TestUser<S>,
    recipient_address: HexHash,
) {
    register_recipient_with_ism(runner, user, recipient_address, Ism::AlwaysTrust);
}

pub fn register_relayer_with_dummy_igp(
    runner: &mut TestRunner<RT, S>,
    relayer: &TestUser<S>,
    domain: u32,
) {
    let domain_oracle_data = HashMap::from([(
        domain,
        ExchangeRateAndGasPrice {
            gas_price: Amount(100),
            token_exchange_rate: 100,
        },
    )]);
    let domain_default_gas = HashMap::from([(domain, Amount(100))]);
    register_relayer_with_igp(
        runner,
        relayer,
        domain_oracle_data,
        domain_default_gas,
        Amount(100),
        None,
    );
}

pub fn register_relayer_with_igp(
    runner: &mut TestRunner<RT, S>,
    relayer: &TestUser<S>,
    domain_oracle_data: HashMap<u32, ExchangeRateAndGasPrice>,
    domain_default_gas: HashMap<u32, Amount>,
    default_gas: Amount,
    beneficiary: Option<<S as Spec>::Address>,
) {
    runner.execute_transaction(TransactionTestCase {
        input: relayer.create_plain_message::<RT, InterchainGasPaymaster<S>>(
            InterchainGasPaymasterCallMessage::SetRelayerConfig {
                domain_oracle_data: oracle_data_hashmap_to_safe_vec(domain_oracle_data.clone()),
                domain_default_gas: default_gas_hashmap_to_safe_vec(domain_default_gas.clone()),
                default_gas,
                beneficiary,
            },
        ),
        assert: Box::new(|result, _| {
            assert!(
                result.tx_receipt.is_successful(),
                "IGP set relayer config was not done successfully"
            );
        }),
    });
}

pub fn register_recipient_with_ism(
    runner: &mut TestRunner<RT, S>,
    user: &TestUser<S>,
    recipient_address: HexHash,
    ism: Ism,
) {
    runner.execute_transaction(TransactionTestCase {
        input: user.create_plain_message::<RT, TestRecipient<S>>(RecipientCallMessage::Register {
            address: recipient_address,
            ism,
        }),
        assert: Box::new(|result, _| {
            assert!(
                result.tx_receipt.is_successful(),
                "Recipient was not registered successfully"
            );
        }),
    });
}

pub fn set_default_ism(runner: &mut TestRunner<RT, S>, user: &TestUser<S>, ism: Ism) {
    runner.execute_transaction(TransactionTestCase {
        input: user.create_plain_message::<RT, TestRecipient<S>>(
            RecipientCallMessage::SetDefaultIsm { ism },
        ),
        assert: Box::new(|result, _| {
            assert!(
                result.tx_receipt.is_successful(),
                "ISM was not set successfully"
            );
        }),
    });
}

fn eth_address_from_secp_public_key(pub_key: secp256k1::PublicKey) -> EthAddress {
    use sha3::Digest;
    let pubkey_bytes: [u8; 65] = pub_key.serialize_uncompressed();
    // The first byte is the compression flag, which we don't care about.
    // https://stackoverflow.com/questions/66383584/how-to-extract-uncompressed-public-key-from-secp256k1
    let bytes = &pubkey_bytes[1..];

    let hash: [u8; 32] = Keccak256::digest(bytes).into();
    // truncate first 12 bytes
    HexString(hash[12..].try_into().expect("Must be exactly 20 bytes"))
}

pub fn random_validator() -> (SecretKey, EthAddress) {
    let secp = Secp256k1::new();
    let (secret_key, public_key) = secp.generate_keypair(&mut OsRng);
    let address = eth_address_from_secp_public_key(public_key);
    (secret_key, address)
}

pub fn sign(digest: [u8; 32], sk: &SecretKey) -> ValidatorSignature {
    let secp = Secp256k1::new();
    let signature = secp.sign_ecdsa_recoverable(&Message::from_digest(digest), sk);
    let (recovery_id, sig_bytes) = signature.serialize_compact();

    let mut bytes = [0u8; 65];
    bytes[..64].copy_from_slice(&sig_bytes);
    bytes[64] = recovery_id.to_i32() as u8;
    HexString(bytes)
}

pub fn unlimited_gas_meter() -> BasicGasMeter<S> {
    BasicGasMeter::new_with_funds_and_gas(
        Amount::MAX,
        <<S as Spec>::Gas as Gas>::max(),
        <<S as Spec>::Gas as Gas>::Price::ZEROED,
    )
}
