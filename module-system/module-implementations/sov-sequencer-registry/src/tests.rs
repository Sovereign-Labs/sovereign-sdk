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

    let seq_da_addreess = MockAddress::from_str("0000000000000000000000000000000000000000000000000000000000000000").unwrap()];
    
    let config = SequencerConfig::<DefaultContext, MockDaSpec> {
        seq_rollup_address,
        seq_da_address: seq_da_addreess,
        coins_to_lock: coins,
        is_preferred_sequencer: true,
    };

    let data = r#"{
        "seq_rollup_address":"sov1l6n2cku82yfqld30lanm2nfw43n2auc8clw7r5u5m6s7p8jrm4zqrr8r94",
        "seq_da_address":"0000000000000000000000000000000000000000000000000000000000000000",
        "coins_to_lock":{
            "amount":50,
            "token_address":"sov1zsnx7n2wjvtkr0ttscfgt06pjca3v2e6stxeu49qwynavmk7a8xqlxkkjp"
        },
        "is_preferred_sequencer":true
    }"#;

    let parsed_config: SequencerConfig::<DefaultContext, MockDaSpec> = serde_json::from_str(data).unwrap();
    assert_eq!(config, parsed_config)
}
