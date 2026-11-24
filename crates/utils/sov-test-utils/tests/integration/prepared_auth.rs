//! Checks that authenticating a transaction that was prepared ahead of time is indistinguishable
//! from authenticating it directly: same result, same gas meter state.

use sov_modules_api::capabilities::UniquenessData;
use sov_modules_api::capabilities::{
    take_auth_cache_stats, AuthCacheStats, AuthenticationError, HasKernel, TransactionAuthenticator,
};
use sov_modules_api::transaction::{Transaction, UnsignedTransaction};
use sov_modules_api::Multisig;
use sov_modules_api::{
    Amount, BasicGasMeter, CryptoSpec, FullyBakedTx, Gas, GasArray, PrivateKey, RawTx, Runtime,
    Spec, StateCheckpoint, TxHash,
};
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::storage::ForklessStorageManager;
use sov_test_utils::{default_test_tx_details, AsUser, TestUser};
use sov_value_setter::ValueSetter;

use crate::helpers::{setup, RT, S};

type Auth = <RT as sov_modules_stf_blueprint::Runtime<S>>::Auth;
type TestGas = <S as Spec>::Gas;
type GasPrice = <TestGas as Gas>::Price;

/// The outcome of one authentication: its result, and the gas meter it left behind.
type Outcome = (Result<TxHash, AuthenticationError>, String);

fn set_value_call() -> <RT as sov_modules_api::DispatchCall>::Decodable {
    <RT as sov_test_utils::EncodeCall<ValueSetter<S>>>::to_decodable(
        sov_value_setter::CallMessage::SetValue {
            value: 0x1234,
            gas: None,
        },
    )
}

/// A valid V0 transaction, serialized the way a batch carries it.
fn signed_v0_tx(admin: &TestUser<S>) -> FullyBakedTx {
    admin
        .create_plain_message::<RT, ValueSetter<S>>(sov_value_setter::CallMessage::SetValue {
            value: 0x1234,
            gas: None,
        })
        .to_serialized_authenticated_tx(&mut Default::default())
}

/// A valid V1 transaction signed by two of three multisig keys.
fn signed_v1_tx() -> FullyBakedTx {
    let keys: Vec<_> = (0..3)
        .map(|_| <<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey::generate())
        .collect();
    let multisig = Multisig::new(2, keys.iter().map(|key| key.pub_key()).collect());
    let mut tx = UnsignedTransaction::<RT, S>::new_with_details(
        set_value_call(),
        UniquenessData::Generation(0),
        default_test_tx_details::<S>(),
        None,
    )
    .to_multisig_tx(multisig);
    tx.sign(&keys[0], &RT::CHAIN_HASH).unwrap();
    tx.sign(&keys[1], &RT::CHAIN_HASH).unwrap();
    let raw_tx = borsh::to_vec(&Transaction::<RT, S>::from(tx)).unwrap();
    <Auth as TransactionAuthenticator<S>>::encode_with_standard_auth(RawTx { data: raw_tx })
}

fn gas_price() -> GasPrice {
    GasPrice::from([Amount::new(3), Amount::new(5)])
}

/// Authenticates `tx` with a gas meter that holds `gas` and, if given, `funds`. Returns the
/// result and the meter.
fn authenticate_with_meter(
    runner: &TestRunner<RT, S>,
    tx: &FullyBakedTx,
    gas: TestGas,
    funds: Option<Amount>,
) -> (Result<TxHash, AuthenticationError>, BasicGasMeter<S>) {
    let meter = match funds {
        Some(funds) => BasicGasMeter::<S>::new_with_funds_and_gas(funds, gas, gas_price()),
        None => BasicGasMeter::<S>::new_with_gas(gas, gas_price()),
    };
    let storage = runner.storage_manager().create_prover_storage();
    let checkpoint = StateCheckpoint::<S>::new(storage, &RT::default().kernel());
    let mut state =
        sov_modules_api::StateProvider::to_tx_scratchpad(checkpoint).to_pre_exec_working_set(meter);

    let result = <Auth as TransactionAuthenticator<S>>::authenticate(tx, &mut state)
        .map(|(tx_and_hash, _, _)| tx_and_hash.raw_tx_hash);
    let (_, meter) = state.to_scratchpad_and_gas_meter();
    (result, meter)
}

fn authenticate(
    runner: &TestRunner<RT, S>,
    tx: &FullyBakedTx,
    gas: TestGas,
    funds: Option<Amount>,
) -> Outcome {
    let (result, meter) = authenticate_with_meter(runner, tx, gas, funds);
    (result, format!("{meter:?}"))
}

/// Gas limits and funds from nothing up to slightly more than authenticating `tx` costs, so that
/// authentication fails at each of its charges and also succeeds.
fn limits(runner: &TestRunner<RT, S>, tx: &FullyBakedTx) -> Vec<(TestGas, Option<Amount>)> {
    const STEPS: u64 = 400;
    let (result, meter) = authenticate_with_meter(runner, tx, TestGas::MAX, None);
    assert!(result.is_ok(), "The transaction must be valid: {result:?}");
    let cost = meter.gas_info().gas_used;
    let ceiling: [u64; 2] = [cost.as_ref()[0] + 1, cost.as_ref()[1] + 1];

    let mut limits = Vec::new();
    for step in 0..=STEPS {
        let scaled =
            |dimension: u64| (u128::from(dimension) * u128::from(step) / u128::from(STEPS)) as u64;
        // Both dimensions tight, then each dimension tight on its own.
        limits.push((
            TestGas::from([scaled(ceiling[0]), scaled(ceiling[1])]),
            None,
        ));
        limits.push((TestGas::from([scaled(ceiling[0]), ceiling[1]]), None));
        limits.push((TestGas::from([ceiling[0], scaled(ceiling[1])]), None));
        // Enough gas, but tight funds.
        let full_value = TestGas::from(ceiling).value(gas_price());
        let funds = full_value
            .checked_mul(Amount::new(u128::from(step)))
            .unwrap()
            .checked_div(Amount::new(u128::from(STEPS)))
            .unwrap();
        limits.push((TestGas::from(ceiling), Some(funds)));
    }
    limits
}

/// Authenticates `tx` under every limit, first directly and then from its prepared form.
fn direct_and_prepared_outcomes(
    runner: &TestRunner<RT, S>,
    tx: &FullyBakedTx,
) -> (Vec<Outcome>, Vec<Outcome>, AuthCacheStats) {
    let limits = limits(runner, tx);
    let run = |limits: &[(TestGas, Option<Amount>)]| -> Vec<Outcome> {
        limits
            .iter()
            .map(|(gas, funds)| authenticate(runner, tx, *gas, *funds))
            .collect()
    };

    let direct = run(&limits);
    <Auth as TransactionAuthenticator<S>>::prewarm_authentication(tx, 0);
    take_auth_cache_stats();
    let prepared = run(&limits);
    (direct, prepared, take_auth_cache_stats())
}

#[test]
fn prepared_v0_tx_authenticates_like_a_direct_one() {
    let (admin, runner) = setup();
    let tx = signed_v0_tx(&admin);

    let (direct, prepared, _) = direct_and_prepared_outcomes(&runner, &tx);

    assert_eq!(prepared, direct);
}

#[test]
fn prepared_v1_tx_authenticates_like_a_direct_one() {
    let (_admin, runner) = setup();
    let tx = signed_v1_tx();

    let (direct, prepared, _) = direct_and_prepared_outcomes(&runner, &tx);

    assert_eq!(prepared, direct);
}

#[test]
fn limits_cover_failure_and_success() {
    let (admin, runner) = setup();
    let tx = signed_v0_tx(&admin);

    let (direct, _, _) = direct_and_prepared_outcomes(&runner, &tx);
    let distinct_failures: std::collections::BTreeSet<&String> = direct
        .iter()
        .filter(|(result, _)| result.is_err())
        .map(|(_, meter)| meter)
        .collect();
    let successes = direct.iter().filter(|(result, _)| result.is_ok()).count();

    assert!(
        distinct_failures.len() > 100 && successes > 0,
        "The limits should leave the meter in many different failed states and also succeed, \
         got {} distinct failures and {successes} successes",
        distinct_failures.len()
    );
}

#[test]
fn prepared_tx_is_used_by_authenticate() {
    let (admin, runner) = setup();
    let tx = signed_v0_tx(&admin);
    <Auth as TransactionAuthenticator<S>>::prewarm_authentication(&tx, 0);
    take_auth_cache_stats();

    let (result, _) = authenticate(&runner, &tx, TestGas::MAX, None);
    result.unwrap();

    assert_eq!(
        take_auth_cache_stats(),
        AuthCacheStats {
            prepared_hits: 1,
            ..Default::default()
        },
        "Authentication should use the prepared transaction and skip the signature lookup"
    );
}

#[test]
fn malformed_tx_is_not_prepared() {
    let (admin, runner) = setup();
    let mut tx = signed_v0_tx(&admin);
    tx.data.truncate(tx.data.len() / 2);
    <Auth as TransactionAuthenticator<S>>::prewarm_authentication(&tx, 0);
    take_auth_cache_stats();

    let (result, _) = authenticate(&runner, &tx, TestGas::MAX, None);
    result.unwrap_err();

    assert_eq!(
        take_auth_cache_stats().prepared_hits,
        0,
        "A transaction that cannot be decoded should not be prepared"
    );
}
