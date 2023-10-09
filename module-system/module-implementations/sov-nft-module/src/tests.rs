use crate::NonFungibleTokenConfig;

#[test]
fn test_config_serialization() {
    let config = NonFungibleTokenConfig {};

    let data = r#"
    {
    
    }"#;

    let parsed_config: NonFungibleTokenConfig = serde_json::from_str(data).unwrap();
    assert_eq!(config, parsed_config)
}
