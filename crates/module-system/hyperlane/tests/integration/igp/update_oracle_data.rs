use sov_bank::Amount;
use sov_hyperlane_integration::igp::{ExchangeRateAndGasPrice, RelayerWithDomainKey};
use sov_hyperlane_integration::{
    InterchainGasPaymaster, InterchainGasPaymasterCallMessage, InterchainGasPaymasterEvent,
};
use sov_modules_api::macros::config_value;
use sov_test_utils::{AsUser, TransactionTestCase};

use crate::runtime::{register_relayer_with_dummy_igp, setup, TestRuntimeEvent, RT, S};

#[test]
fn set_correct_update_oracle_data() {
    let (mut runner, _, relayer, ..) = setup();

    let domain = config_value!("HYPERLANE_BRIDGE_DOMAIN");

    let oracle_data_value = ExchangeRateAndGasPrice {
        gas_price: Amount(433232),
        token_exchange_rate: 1434323,
    };

    register_relayer_with_dummy_igp(&mut runner, &relayer, domain);

    let relayer_address = relayer.address();

    let igp = InterchainGasPaymaster::<S>::default();
    runner.execute_transaction(TransactionTestCase {
        input: relayer.create_plain_message::<RT, InterchainGasPaymaster<S>>(
            InterchainGasPaymasterCallMessage::UpdateOracleData {
                domain,
                oracle_data: oracle_data_value,
            },
        ),
        assert: Box::new(move |result, state| {
            assert!(result.events.iter().any(|event| {
                matches!(
                    event,
                    TestRuntimeEvent::InterchainGasPaymaster(
                        InterchainGasPaymasterEvent::OracleDataUpdated {
                            relayer,
                            domain,
                            oracle_data,
                        })
                    if *relayer == relayer_address && domain == domain
                    && oracle_data == &oracle_data_value
                )
            }));

            let key = RelayerWithDomainKey::new(relayer_address, domain);
            let state_domain_oracle = igp
                .domain_oracle_data
                .get(&key, state)
                .expect("domain oracle value retrieved");

            assert_eq!(state_domain_oracle, Some(oracle_data_value));
        }),
    });
}
