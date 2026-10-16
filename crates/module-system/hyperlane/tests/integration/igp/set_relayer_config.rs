use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use sov_bank::Amount;
use sov_hyperlane_integration::igp::{
    DomainDefaultGas, DomainOracleData, ExchangeRateAndGasPrice, RelayerWithDomainKey,
};
use sov_hyperlane_integration::{
    InterchainGasPaymaster, InterchainGasPaymasterCallMessage, InterchainGasPaymasterEvent,
};
use sov_modules_api::macros::config_value;
use sov_modules_api::TxEffect;
use sov_test_utils::runtime::genesis::zk::config::HighLevelZkGenesisConfig;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{AsUser, TransactionTestCase};

use super::{default_gas_hashmap_to_safe_vec, oracle_data_hashmap_to_safe_vec};
use crate::runtime::{setup, GenesisConfig, TestRuntimeEvent, RT, S};

#[test]
fn set_correct_relayer_config() {
    let (mut runner, _, relayer, beneficiary_address, ..) = setup();

    let domain = config_value!("HYPERLANE_BRIDGE_DOMAIN");

    let domain_oracle_data = HashMap::from([(
        domain,
        ExchangeRateAndGasPrice {
            gas_price: Amount(321544),
            token_exchange_rate: 10_000_000_000,
        },
    )]);

    let domain_default_gas = HashMap::from([(domain, Amount(4234324))]);
    let domain_default_gas_assert: BTreeMap<_, _> =
        domain_default_gas.clone().into_iter().collect();
    let expected_default_gas = Amount(20234);
    let beneficiary_address = Some(beneficiary_address.address());
    let relayer_address = relayer.address();

    let igp = InterchainGasPaymaster::<S>::default();
    runner.execute_transaction(TransactionTestCase {
        input: relayer.create_plain_message::<RT, InterchainGasPaymaster<S>>(
            InterchainGasPaymasterCallMessage::SetRelayerConfig {
                domain_oracle_data: oracle_data_hashmap_to_safe_vec(domain_oracle_data.clone()),
                domain_default_gas: default_gas_hashmap_to_safe_vec(domain_default_gas.clone()),
                default_gas: expected_default_gas,
                beneficiary: beneficiary_address,
            },
        ),
        assert: Box::new(move |result, state| {
            assert!(result.events.iter().any(|event| {
                matches!(
                    event,
                    TestRuntimeEvent::InterchainGasPaymaster(
                        InterchainGasPaymasterEvent::RelayerConfigSet {
                            relayer,
                            domain_custom_gas,
                            default_gas,
                            beneficiary
                        })
                    if *relayer == relayer_address && domain_custom_gas == &domain_default_gas_assert
                    && *default_gas == expected_default_gas && beneficiary == &beneficiary_address
                )
            }));

            let key = RelayerWithDomainKey::new(relayer_address, domain);
            let state_domain_oracle = igp
                .domain_oracle_data
                .get(&key, state)
                .expect("domain oracle value retrieved");

            assert_eq!(
                state_domain_oracle,
                Some(domain_oracle_data[&domain])
            );

            let state_domain_default_gas = igp
                .domain_default_gas
                .get(&key, state)
                .expect("domain default gas retrieved");

            assert_eq!(
                state_domain_default_gas,
                Some(domain_default_gas[&domain])
            );

            let state_default_gas = igp
                .relayer_default_gas
                .get(&relayer_address, state)
                .expect("default gas retrieved");

            assert_eq!(state_default_gas, Some(expected_default_gas));

            let state_beneficiary = igp
                .beneficiary
                .get(&relayer_address, state)
                .expect("beneficiary retrieved");

            assert_eq!(state_beneficiary, Some(beneficiary_address));
        }),
    });
}

/// Executes the same rejected `SetRelayerConfig` on fresh runners from one genesis, and returns
/// the distinct amounts of gas charged.
///
/// A rejected config bails at the first invalid domain, after a metered write for every valid
/// domain visited before it. The domains must be visited in a deterministic order, otherwise the
/// gas charged (and thus the state root) differs between nodes. Every new `HashMap` gets its own
/// hasher seed, so a hash-ordered implementation would visit the invalid domain at a different
/// position in different runs.
fn distinct_gas_of_rejected_relayer_config(
    domain_oracle_data: Vec<DomainOracleData>,
    domain_default_gas: Vec<DomainDefaultGas>,
) -> BTreeSet<Vec<u64>> {
    const RUNS: usize = 8;

    let genesis_config = HighLevelZkGenesisConfig::generate_with_additional_accounts(3);
    let relayer = genesis_config.additional_accounts()[2].clone();
    let genesis = GenesisConfig::from_minimal_config(genesis_config.into(), (), (), (), ());

    let gas_used = Arc::new(Mutex::new(BTreeSet::new()));
    for _ in 0..RUNS {
        let mut runner: TestRunner<RT, S> =
            TestRunner::new_with_genesis(genesis.clone().into_genesis_params(), Default::default());
        let gas_used = gas_used.clone();
        runner.execute_transaction(TransactionTestCase {
            input: relayer.create_plain_message::<RT, InterchainGasPaymaster<S>>(
                InterchainGasPaymasterCallMessage::SetRelayerConfig {
                    domain_oracle_data: domain_oracle_data
                        .clone()
                        .try_into()
                        .expect("domain count is within limit"),
                    domain_default_gas: domain_default_gas
                        .clone()
                        .try_into()
                        .expect("domain count is within limit"),
                    default_gas: Amount(20234),
                    beneficiary: None,
                },
            ),
            assert: Box::new(move |result, _state| {
                let TxEffect::Reverted(contents) = &result.tx_receipt else {
                    panic!("Relayer config with an invalid domain must be rejected");
                };
                gas_used
                    .lock()
                    .unwrap()
                    .insert(contents.gas_used.as_ref().to_vec());
            }),
        });
    }

    let gas_used = gas_used.lock().unwrap().clone();
    gas_used
}

const VALID_DOMAINS: u32 = 16;

#[test]
fn rejected_oracle_data_charges_deterministic_gas() {
    let valid_data = ExchangeRateAndGasPrice {
        gas_price: Amount(321544),
        token_exchange_rate: 10_000_000_000,
    };
    let invalid_data = ExchangeRateAndGasPrice {
        gas_price: Amount::ZERO,
        token_exchange_rate: 10_000_000_000,
    };
    let domain_oracle_data = (0..VALID_DOMAINS)
        .map(|domain| (1000 + domain, valid_data))
        .chain(std::iter::once((1000 + VALID_DOMAINS, invalid_data)))
        .map(|(domain, data_value)| DomainOracleData { domain, data_value })
        .collect();

    let gas_used = distinct_gas_of_rejected_relayer_config(domain_oracle_data, Vec::new());

    assert_eq!(
        gas_used.len(),
        1,
        "Identical tx on identical state must charge identical gas: {gas_used:?}"
    );
}

#[test]
fn rejected_domain_default_gas_charges_deterministic_gas() {
    let domain_default_gas = (0..VALID_DOMAINS)
        .map(|domain| (1000 + domain, Amount(4234324)))
        .chain(std::iter::once((1000 + VALID_DOMAINS, Amount::ZERO)))
        .map(|(domain, default_gas)| DomainDefaultGas {
            domain,
            default_gas,
        })
        .collect();

    let gas_used = distinct_gas_of_rejected_relayer_config(Vec::new(), domain_default_gas);

    assert_eq!(
        gas_used.len(),
        1,
        "Identical tx on identical state must charge identical gas: {gas_used:?}"
    );
}
