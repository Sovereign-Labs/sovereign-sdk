use std::collections::HashMap;

use sov_bank::Amount;
use sov_hyperlane_integration::igp::{ExchangeRateAndGasPrice, RelayerWithDomainKey};
use sov_hyperlane_integration::{
    InterchainGasPaymaster, InterchainGasPaymasterCallMessage, InterchainGasPaymasterEvent,
};
use sov_modules_api::macros::config_value;
use sov_test_utils::{AsUser, TransactionTestCase};

use super::{default_gas_hashmap_to_safe_vec, oracle_data_hashmap_to_safe_vec};
use crate::runtime::{setup, TestRuntimeEvent, RT, S};

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
    let domain_default_gas_assert = domain_default_gas.clone();
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
