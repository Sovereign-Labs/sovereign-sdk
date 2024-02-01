use serde::{Deserialize, Serialize};
// no genesis for this module

/// Config for the NonFungibleToken module.
/// Sets admin and existing owners.
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct NonFungibleTokenConfig {}

#[cfg(test)]
mod tests {
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
}
