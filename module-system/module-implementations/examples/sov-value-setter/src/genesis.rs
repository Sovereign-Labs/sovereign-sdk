use anyhow::Result;
use sov_modules_api::prelude::*;
use sov_modules_api::WorkingSet;

use super::ValueSetter;

/// Initial configuration for sov-value-setter module.
#[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq)]
pub struct ValueSetterConfig<C: sov_modules_api::Context> {
    /// Admin of the module.
    pub admin: C::Address,
}

impl<C: sov_modules_api::Context> ValueSetter<C> {
    /// Initializes module with the `admin` role.
    pub(crate) fn init_module(
        &self,
        admin_config: &<Self as sov_modules_api::Module>::Config,
        working_set: &mut WorkingSet<C>,
    ) -> Result<()> {
        self.admin.set(&admin_config.admin, working_set);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use sov_modules_api::default_context::DefaultContext;
    use sov_modules_api::Address;

    use crate::ValueSetterConfig;

    #[test]
    fn test_config_serialization() {
        let admin = Address::from([1; 32]);
        let config = ValueSetterConfig::<DefaultContext> { admin };

        let data = r#"
        {
            "admin":"sov1qyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqszqgpqyqs259tk3"
        }"#;

        let parsed_config: ValueSetterConfig<DefaultContext> = serde_json::from_str(data).unwrap();
        assert_eq!(parsed_config, config);
    }
}
