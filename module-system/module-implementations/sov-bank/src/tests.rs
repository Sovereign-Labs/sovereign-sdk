use std::str::FromStr;

use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::{AddressBech32, Spec};

use crate::{BankConfig, TokenConfig};

#[test]
fn test_config_serialization() {
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

    assert_eq!(config, parsed_config)
}
