use std::str::FromStr;

use sov_bank::Coins;
use sov_modules_api::{default_context::DefaultContext, AddressBech32, Spec};
use sov_rollup_interface::mocks::{MockAddress, MockDaSpec};

use crate::SequencerConfig;

#[test]
fn test_config_serialization() {
    let seq_rollup_address: <DefaultContext as Spec>::Address =
        AddressBech32::from_str("sov1l6n2cku82yfqld30lanm2nfw43n2auc8clw7r5u5m6s7p8jrm4zqrr8r94")
            .unwrap()
            .into();

    let token_address: <DefaultContext as Spec>::Address =
        AddressBech32::from_str("sov1zsnx7n2wjvtkr0ttscfgt06pjca3v2e6stxeu49qwynavmk7a8xqlxkkjp")
            .unwrap()
            .into();

    let coins = Coins::<DefaultContext> {
        amount: 50,
        token_address,
    };

    /*
        let seq_da_addreess = MockAddress::from[[0u8; 32]];
        ///{"seq_rollup_address":"sov1l6n2cku82yfqld30lanm2nfw43n2auc8clw7r5u5m6s7p8jrm4zqrr8r94","seq_da_address":{"addr":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]},"coins_to_lock":{"amount":50,"token_address":"sov1zsnx7n2wjvtkr0ttscfgt06pjca3v2e6stxeu49qwynavmk7a8xqlxkkjp"},"is_preferred_sequencer":true}
        let config = SequencerConfig::<DefaultContext, MockDaSpec> {
            seq_rollup_address,
            seq_da_address: seq_da_addreess,
            coins_to_lock: coins,
            is_preferred_sequencer: true,
        };
    */

    /*


    let address: <DefaultContext as Spec>::Address =
        AddressBech32::from_str("sov1l6n2cku82yfqld30lanm2nfw43n2auc8clw7r5u5m6s7p8jrm4zqrr8r94")
            .unwrap()
            .into();

    let config = BankConfig::<DefaultContext> {
        tokens: vec![TokenConfig {
            token_name: "sov-demo-token".to_owned(),
            address_and_balances: vec![(address, 100000000)],
            authorized_minters: vec![address],
            salt: 0,
        }],
    };

    let data = r#"
    {
        "tokens":[
            {
                "token_name":"sov-demo-token",
                "address_and_balances":[["sov1l6n2cku82yfqld30lanm2nfw43n2auc8clw7r5u5m6s7p8jrm4zqrr8r94",100000000]],
                "authorized_minters":["sov1l6n2cku82yfqld30lanm2nfw43n2auc8clw7r5u5m6s7p8jrm4zqrr8r94"]
                ,"salt":0
            }
        ]
    }"#;

    let parsed_config: BankConfig<DefaultContext> = serde_json::from_str(data).unwrap();

    assert_eq!(config, parsed_config)*/
}
