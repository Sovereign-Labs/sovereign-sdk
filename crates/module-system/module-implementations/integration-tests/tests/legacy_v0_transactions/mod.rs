//! Compatibility tests for V0 transactions in the pre-fork wire format.
//!
//! The fixtures in `v0_fixtures.json` were generated on the `v0` branch (the commit right before
//! the multisig and accounts hard fork, see the `source_*` fields in the file) by signing
//! `Bank::Transfer` calls with deterministic keys, and were checked to execute successfully there.
//! They target a runtime generated with `generate_optimistic_runtime!(LegacyV0FixtureRuntime <= )`,
//! whose module set (and therefore `RuntimeCall` encoding) and chain hash are identical on both
//! sides of the fork, so the same runtime is generated here and the raw bytes are replayed
//! unchanged.
//!
//! See `sov_modules_api::transaction::legacy_v0` for the compatibility shim under test.

use std::env;

use serde::Deserialize;
use sov_bank::{config_gas_token_id, Bank, CallMessage as BankCallMessage, Coins};
use sov_modules_api::capabilities::{self, UniquenessData};
use sov_modules_api::macros::config_value;
use sov_modules_api::transaction::legacy_v0::{self, DecodedTransaction};
use sov_modules_api::transaction::{
    PriorityFeeBips, Transaction, TxDetails, UnsignedTransaction, Version0,
};
use sov_modules_api::{
    Amount, CryptoSpec, DispatchCall, GasUnit, PrivateKey, PublicKey, RawTx, Runtime as _, Spec,
    TxEffect,
};
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{
    generate_optimistic_runtime, EncodeCall, TestSpec, TestUser, TransactionTestCase,
    TransactionType, TxProcessingError, TEST_DEFAULT_MAX_FEE, TEST_DEFAULT_MAX_PRIORITY_FEE,
    TEST_DEFAULT_USER_BALANCE,
};

generate_optimistic_runtime!(LegacyV0FixtureRuntime <= );

type S = TestSpec;
type RT = LegacyV0FixtureRuntime<S>;
type Key = <<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey;
type RuntimeCall = <RT as DispatchCall>::Decodable;

/// Generated on the `v0` branch; see the module docs.
const FIXTURES_JSON: &str = include_str!("v0_fixtures.json");

#[derive(Deserialize)]
struct Fixtures {
    chain_hash: String,
    chain_id: u64,
    recipient: Party,
    senders: Vec<Party>,
    transactions: Vec<FixtureTx>,
}

#[derive(Deserialize)]
struct Party {
    seed: u8,
    address: String,
    credential_id: Option<String>,
}

#[derive(Deserialize)]
struct FixtureTx {
    name: String,
    sender_seed: u8,
    uniqueness: UniquenessData,
    amount: String,
    max_priority_fee_bips: u64,
    max_fee: String,
    gas_limit: Option<[u64; 2]>,
    chain_id: u64,
    chain_hash: String,
    expect: Expectation,
    signing_payload: String,
    raw_tx: String,
}

/// How the post-fork rollup is expected to treat a fixture, without any chain hash overrides.
#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Expectation {
    Success,
    InvalidChainId,
    InvalidSignature,
}

impl Fixtures {
    fn load() -> Self {
        serde_json::from_str(FIXTURES_JSON).expect("v0_fixtures.json is valid")
    }

    fn by_name(&self, name: &str) -> &FixtureTx {
        self.transactions
            .iter()
            .find(|tx| tx.name == name)
            .unwrap_or_else(|| panic!("no fixture named {name}"))
    }

    fn successful(&self) -> impl Iterator<Item = &FixtureTx> {
        self.transactions
            .iter()
            .filter(|tx| tx.expect == Expectation::Success)
    }

    fn recipient(&self) -> <S as Spec>::Address {
        address_from_seed(self.recipient.seed)
    }

    fn expected_call(&self, tx: &FixtureTx) -> RuntimeCall {
        <RT as EncodeCall<Bank<S>>>::to_decodable(BankCallMessage::Transfer {
            to: self.recipient(),
            coins: Coins {
                amount: Amount::new(tx.amount()),
                token_id: config_gas_token_id(),
            },
        })
    }

    /// A fresh rollup whose genesis funds every fixture sender.
    fn runner(&self) -> TestRunner<RT, S> {
        let genesis_config = HighLevelOptimisticGenesisConfig::<S>::generate().add_accounts(
            self.senders
                .iter()
                .map(|sender| TestUser::new(key_from_seed(sender.seed), TEST_DEFAULT_USER_BALANCE))
                .collect(),
        );
        let genesis = GenesisConfig::from_minimal_config(genesis_config.into());
        TestRunner::<RT, S>::new_with_genesis(genesis.into_genesis_params(), RT::default())
    }
}

impl FixtureTx {
    fn raw_tx(&self) -> Vec<u8> {
        hex::decode(&self.raw_tx).unwrap()
    }

    fn signing_payload(&self) -> Vec<u8> {
        hex::decode(&self.signing_payload).unwrap()
    }

    fn chain_hash(&self) -> [u8; 32] {
        hex32(&self.chain_hash)
    }

    fn amount(&self) -> u128 {
        self.amount.parse().unwrap()
    }

    /// The details the fixture was built from. Legacy transactions carry the chain id in the
    /// field that now holds the chain hash fragment.
    fn details(&self) -> TxDetails<S> {
        TxDetails {
            max_priority_fee_bips: PriorityFeeBips::from(self.max_priority_fee_bips),
            max_fee: Amount::new(self.max_fee.parse().unwrap()),
            gas_limit: self.gas_limit.map(GasUnit::from),
            chain_hash_fragment: self.chain_id,
        }
    }

    fn input(&self) -> TransactionType<RT, S> {
        TransactionType::PreSigned(RawTx::new(self.raw_tx()))
    }
}

/// Deterministic ed25519 key: the 32 seed bytes are `seed_byte, seed_byte + 1, ...`. Must match
/// the derivation used by the fixture generator.
fn key_from_seed(seed_byte: u8) -> Key {
    let seed: Vec<u8> = (0..32u8).map(|i| seed_byte.wrapping_add(i)).collect();
    Key::try_from(seed).expect("32-byte ed25519 seed")
}

fn address_from_seed(seed_byte: u8) -> <S as Spec>::Address {
    key_from_seed(seed_byte).pub_key().credential_id().into()
}

fn hex32(hex_str: &str) -> [u8; 32] {
    hex::decode(hex_str)
        .unwrap()
        .try_into()
        .expect("32-byte hex string")
}

fn decode(raw_tx: &[u8]) -> DecodedTransaction<RT, S> {
    let mut reader = raw_tx;
    let decoded =
        legacy_v0::deserialize_transaction_reader::<RT, S, <S as Spec>::CryptoSpec, _>(&mut reader)
            .expect("transaction decodes");
    assert!(reader.is_empty(), "decoder left trailing bytes");
    decoded
}

fn decode_legacy(raw_tx: &[u8]) -> Version0<RT, S> {
    match decode(raw_tx) {
        DecodedTransaction::LegacyV0(tx) => tx,
        DecodedTransaction::Current(tx) => {
            panic!("expected a legacy V0 envelope, decoded a current-format transaction: {tx:?}")
        }
    }
}

fn assert_effect(name: &str, effect: &TxEffect<S>, expect: Expectation) {
    match expect {
        Expectation::Success => assert!(
            effect.is_successful(),
            "fixture {name} should execute, got {effect:?}"
        ),
        Expectation::InvalidChainId => {
            assert_authentication_failure(name, effect, "Invalid chain id");
        }
        Expectation::InvalidSignature => {
            assert_authentication_failure(name, effect, "Signature verification failed");
        }
    }
}

fn assert_authentication_failure(name: &str, effect: &TxEffect<S>, expected_reason: &str) {
    let TxEffect::Skipped(skipped) = effect else {
        panic!("fixture {name} should be skipped, got {effect:?}");
    };
    let TxProcessingError::AuthenticationFailed(reason) = &skipped.error else {
        panic!(
            "fixture {name} should fail authentication, got {:?}",
            skipped.error
        );
    };
    assert!(
        reason.contains(expected_reason),
        "fixture {name}: unexpected authentication failure: {reason}"
    );
}

/// Executes the fixture in its own slot and checks the outcome against `expect`.
fn execute_expecting(runner: &mut TestRunner<RT, S>, tx: &FixtureTx, expect: Expectation) {
    let name = tx.name.clone();
    runner.execute_transaction(TransactionTestCase {
        input: tx.input(),
        assert: Box::new(move |result, _state| assert_effect(&name, &result.tx_receipt, expect)),
    });
}

/// Guards against drift in the key derivation and runtime constants the fixtures rely on.
#[test]
fn fixture_metadata_matches_this_runtime() {
    let fixtures = Fixtures::load();

    assert_eq!(hex32(&fixtures.chain_hash), RT::CHAIN_HASH);
    assert_eq!(fixtures.chain_id, config_value!("CHAIN_ID"));
    assert_eq!(fixtures.recipient().to_string(), fixtures.recipient.address);
    for sender in &fixtures.senders {
        assert_eq!(address_from_seed(sender.seed).to_string(), sender.address);
        assert_eq!(
            key_from_seed(sender.seed)
                .pub_key()
                .credential_id()
                .to_string(),
            sender
                .credential_id
                .clone()
                .expect("senders list a credential id")
        );
    }
}

/// Every fixture decodes as a legacy V0 envelope carrying exactly the fields it was built from.
#[test]
fn fixtures_decode_as_legacy_v0_envelopes() {
    let fixtures = Fixtures::load();

    for tx in &fixtures.transactions {
        let decoded = decode_legacy(&tx.raw_tx());
        assert_eq!(decoded.address_override, None, "{}", tx.name);
        assert_eq!(
            decoded.pub_key,
            key_from_seed(tx.sender_seed).pub_key(),
            "{}",
            tx.name
        );
        assert_eq!(
            decoded.runtime_call,
            fixtures.expected_call(tx),
            "{}",
            tx.name
        );
        assert_eq!(decoded.uniqueness, tx.uniqueness, "{}", tx.name);
        assert_eq!(decoded.details, tx.details(), "{}", tx.name);
    }
}

/// The fixtures are not valid current-format transactions: the derived decoder rejects them, so
/// they only get in through the legacy shim.
#[test]
fn derived_decoder_rejects_legacy_fixtures() {
    let fixtures = Fixtures::load();

    for tx in &fixtures.transactions {
        let error = borsh::from_slice::<Transaction<RT, S>>(&tx.raw_tx())
            .err()
            .unwrap_or_else(|| panic!("{}: derived decoder accepted a legacy envelope", tx.name));
        // Borsh reports the truncated input as "Unexpected length of input".
        assert!(
            error.to_string().contains("Unexpected length of input"),
            "{}: legacy envelopes end where the derived decoder expects `address_override`, got: {error}",
            tx.name
        );
    }
}

/// The unmetered decode path used by the sequencer (`decode_serialized_tx`) also understands
/// legacy envelopes.
#[test]
fn fixtures_decode_through_decode_sov_tx() {
    let fixtures = Fixtures::load();

    for tx in &fixtures.transactions {
        let call = capabilities::decode_sov_tx::<S, RT>(&tx.raw_tx())
            .unwrap_or_else(|e| panic!("{}: {e}", tx.name));
        assert_eq!(call, fixtures.expected_call(tx), "{}", tx.name);
    }
}

/// Rebuilding each fixture on this branch with the legacy signing payload and envelope yields the
/// exact bytes produced by the `v0` branch (ed25519 signatures are deterministic), so the legacy
/// definitions here match the pre-fork ones byte for byte.
#[test]
fn legacy_encoding_reproduces_v0_fixture_bytes() {
    let fixtures = Fixtures::load();

    for tx in &fixtures.transactions {
        let key = key_from_seed(tx.sender_seed);
        let mut rebuilt = Version0::<RT, S> {
            // Placeholder, replaced once the signing payload is known.
            signature: key.sign(&[]),
            pub_key: key.pub_key(),
            runtime_call: fixtures.expected_call(tx),
            uniqueness: tx.uniqueness,
            details: tx.details(),
            address_override: None,
        };

        let payload =
            legacy_v0::legacy_signing_bytes(&Transaction::V0(rebuilt.clone()), &tx.chain_hash());
        assert_eq!(
            payload,
            tx.signing_payload(),
            "{}: legacy signing payload",
            tx.name
        );

        rebuilt.signature = key.sign(&payload);
        assert_eq!(
            legacy_v0::legacy_v0_envelope_bytes(&rebuilt),
            tx.raw_tx(),
            "{}: legacy envelope",
            tx.name
        );
    }
}

/// Legacy transactions are authenticated, authorized and executed by the post-fork STF.
#[test]
fn legacy_transactions_execute_and_transfer_funds() {
    let fixtures = Fixtures::load();
    let mut runner = fixtures.runner();

    let mut expected_balance = 0u128;
    for tx in fixtures.successful() {
        execute_expecting(&mut runner, tx, Expectation::Success);
        expected_balance += tx.amount();
    }

    let recipient = fixtures.recipient();
    runner.query_visible_state(|state| {
        let balance = Bank::<S>::default()
            .get_balance_of(&recipient, config_gas_token_id(), state)
            .unwrap();
        assert_eq!(
            balance,
            Some(Amount::new(expected_balance)),
            "recipient should have received every successful legacy transfer"
        );
    });
}

#[test]
fn legacy_transaction_with_wrong_chain_id_is_rejected() {
    let fixtures = Fixtures::load();
    let mut runner = fixtures.runner();

    execute_expecting(
        &mut runner,
        fixtures.by_name("wrong_chain_id"),
        Expectation::InvalidChainId,
    );
}

#[test]
fn legacy_transaction_with_unknown_chain_hash_is_rejected() {
    let fixtures = Fixtures::load();
    let mut runner = fixtures.runner();

    execute_expecting(
        &mut runner,
        fixtures.by_name("wrong_chain_hash"),
        Expectation::InvalidSignature,
    );
}

/// A legacy transaction signed over a chain hash that is only valid through a configured grace
/// period is accepted: like the pre-fork authenticator, the legacy path tries every chain hash
/// valid at the current height.
#[test]
fn legacy_transaction_signed_over_grace_period_chain_hash_is_accepted() {
    let fixtures = Fixtures::load();
    let tx = fixtures.by_name("grace_period_chain_hash");
    let mut runner = fixtures.runner();

    // Keep the runtime's own chain hash primary, but make the fixture's chain hash valid for the
    // first 1000 heights through a grace period.
    env::set_var(
        "SOV_TEST_CONST_OVERRIDE_CHAIN_HASH_OVERRIDES",
        format!(
            r#"[{{ start_height = 0, end_height = 0, chain_hash = "0x{}", grace_period = 1000 }}]"#,
            tx.chain_hash
        ),
    );
    execute_expecting(&mut runner, tx, Expectation::Success);
    env::remove_var("SOV_TEST_CONST_OVERRIDE_CHAIN_HASH_OVERRIDES");
}

/// Replaying a legacy transaction is caught by the uniqueness check like any other transaction.
#[test]
fn legacy_transaction_cannot_be_replayed() {
    let fixtures = Fixtures::load();
    let tx = fixtures.by_name("nonce_0_transfer");
    let mut runner = fixtures.runner();

    execute_expecting(&mut runner, tx, Expectation::Success);
    runner.execute_transaction(TransactionTestCase {
        input: tx.input(),
        assert: Box::new(|result, _state| {
            let TxEffect::Skipped(skipped) = &result.tx_receipt else {
                panic!(
                    "replayed legacy transaction should be skipped, got {:?}",
                    result.tx_receipt
                );
            };
            let TxProcessingError::CheckUniquenessFailed(_) = &skipped.error else {
                panic!(
                    "replay should fail the uniqueness check, got {:?}",
                    skipped.error
                );
            };
        }),
    });
}

/// Appending an `address_override` byte to a legacy envelope turns it into a current-format
/// envelope, which is authenticated with the current rules: its details field is then read as a
/// chain hash fragment, and a legacy chain id is not a fragment of any valid chain hash. (Even if
/// it were, the legacy signature does not cover the current signing payload.) The two encodings
/// are not interchangeable.
#[test]
fn legacy_envelope_with_appended_address_override_is_rejected() {
    let fixtures = Fixtures::load();
    let tx = fixtures.by_name("nonce_0_transfer");
    let mut raw_tx = tx.raw_tx();
    raw_tx.push(0); // `address_override = None` in the current encoding.

    let DecodedTransaction::Current(Transaction::V0(decoded)) = decode(&raw_tx) else {
        panic!("a V0 envelope with a trailing `address_override` should decode as current-format");
    };
    assert_eq!(decoded.address_override, None);

    let mut runner = fixtures.runner();
    runner.execute_transaction(TransactionTestCase {
        input: TransactionType::PreSigned(RawTx::new(raw_tx)),
        assert: Box::new(|result, _state| {
            assert_authentication_failure(
                "nonce_0_transfer with appended address_override",
                &result.tx_receipt,
                "Invalid chain hash fragment",
            );
        }),
    });
}

/// Conversely, truncating a current-format envelope so that it parses as a legacy one makes the
/// legacy rules apply: the chain hash fragment is then read as a chain id and rejected.
#[test]
fn current_envelope_truncated_to_legacy_is_rejected() {
    let fixtures = Fixtures::load();
    let tx = fixtures.by_name("nonce_0_transfer");
    let unsigned = UnsignedTransaction::<RT, S>::new(
        fixtures.expected_call(tx),
        RT::CHAIN_HASH,
        TEST_DEFAULT_MAX_PRIORITY_FEE,
        TEST_DEFAULT_MAX_FEE,
        UniquenessData::Nonce(0),
        None,
        None,
    );
    let signed = Transaction::<RT, S>::new_signed_tx(
        &key_from_seed(tx.sender_seed),
        &RT::CHAIN_HASH,
        unsigned,
    );
    let mut raw_tx = borsh::to_vec(&signed).unwrap();
    assert_eq!(
        raw_tx.pop(),
        Some(0),
        "a current envelope ends with `address_override = None`"
    );

    let DecodedTransaction::LegacyV0(_) = decode(&raw_tx) else {
        panic!("truncated current-format envelope should decode as legacy");
    };

    let mut runner = fixtures.runner();
    runner.execute_transaction(TransactionTestCase {
        input: TransactionType::PreSigned(RawTx::new(raw_tx)),
        assert: Box::new(|result, _state| {
            assert_authentication_failure(
                "truncated current-format envelope",
                &result.tx_receipt,
                "Invalid chain id",
            );
        }),
    });
}

/// The legacy-aware decoder accepts current-format envelopes, with or without an address override,
/// exactly like the derived decoder.
#[test]
fn current_format_envelopes_decode_unchanged() {
    let fixtures = Fixtures::load();
    let tx = fixtures.by_name("nonce_0_transfer");
    let key = key_from_seed(tx.sender_seed);

    for address_override in [None, Some(address_from_seed(0x60))] {
        let unsigned = UnsignedTransaction::<RT, S>::new(
            fixtures.expected_call(tx),
            RT::CHAIN_HASH,
            TEST_DEFAULT_MAX_PRIORITY_FEE,
            TEST_DEFAULT_MAX_FEE,
            UniquenessData::Nonce(0),
            None,
            address_override,
        );
        let signed = Transaction::<RT, S>::new_signed_tx(&key, &RT::CHAIN_HASH, unsigned);

        let DecodedTransaction::Current(decoded) = decode(&borsh::to_vec(&signed).unwrap()) else {
            panic!("current-format envelope decoded as legacy");
        };
        assert_eq!(decoded, signed);
    }
}

#[test]
fn invalid_address_override_tag_is_rejected() {
    let fixtures = Fixtures::load();
    let mut raw_tx = fixtures.by_name("nonce_0_transfer").raw_tx();
    raw_tx.push(2); // Neither `None` (0) nor `Some` (1).

    let error = legacy_v0::deserialize_transaction_reader::<RT, S, <S as Spec>::CryptoSpec, _>(
        &mut raw_tx.as_slice(),
    )
    .expect_err("an invalid Option tag should not decode");
    assert!(
        error
            .to_string()
            .contains("Invalid Option representation: 2"),
        "unexpected error: {error}"
    );
}

/// Current-format transactions from a sender that previously used the legacy encoding keep working
/// and share the same nonce sequence.
#[test]
fn current_format_transaction_follows_legacy_transactions() {
    let fixtures = Fixtures::load();
    let mut runner = fixtures.runner();

    for name in [
        "nonce_0_transfer",
        "nonce_1_transfer_with_gas_limit_and_priority_fee",
    ] {
        execute_expecting(&mut runner, fixtures.by_name(name), Expectation::Success);
    }

    let legacy_tx = fixtures.by_name("nonce_0_transfer");
    let unsigned = UnsignedTransaction::<RT, S>::new(
        fixtures.expected_call(legacy_tx),
        RT::CHAIN_HASH,
        TEST_DEFAULT_MAX_PRIORITY_FEE,
        TEST_DEFAULT_MAX_FEE,
        UniquenessData::Nonce(2),
        None,
        None,
    );
    runner.execute_transaction(TransactionTestCase {
        input: TransactionType::pre_signed(
            unsigned,
            &key_from_seed(legacy_tx.sender_seed),
            &RT::CHAIN_HASH,
        ),
        assert: Box::new(|result, _state| {
            assert!(
                result.tx_receipt.is_successful(),
                "current-format transaction with nonce 2 should execute, got {:?}",
                result.tx_receipt
            );
        }),
    });
}
