use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use sov_modules_api::prelude::*;
use sov_modules_api::{Context, WorkingSet};

use crate::NonFungibleToken;

/// Config for the NonFungibleToken module.
/// Sets admin and existing owners.
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct NonFungibleTokenConfig<C: Context> {
    /// Admin of the NonFungibleToken module.
    pub admin: C::Address,
    /// Existing owners of the NonFungibleToken module.
    pub owners: Vec<(u64, C::Address)>,
}

impl<C: Context> NonFungibleToken<C> {
    pub(crate) fn init_module(
        &self,
        config: &<Self as sov_modules_api::Module>::Config,
        working_set: &mut WorkingSet<C>,
    ) -> Result<()> {
        self.admin.set(&config.admin, working_set);
        for (id, owner) in config.owners.iter() {
            if self.owners.get(id, working_set).is_some() {
                bail!("Token id {} already exists", id);
            }
            self.owners.set(id, owner, working_set);
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use sov_modules_api::default_context::DefaultContext;
    use sov_modules_api::utils::generate_address;
    use sov_modules_api::Spec;

    use super::NonFungibleTokenConfig;

    #[test]
    fn test_config_serialization() {
        let address: <DefaultContext as Spec>::Address =
            generate_address::<DefaultContext>("admin");
        let owner: <DefaultContext as Spec>::Address = generate_address::<DefaultContext>("owner");

        let config = NonFungibleTokenConfig::<DefaultContext> {
            admin: address,
            owners: vec![(0, owner)],
        };

        let data = r#"
        {
            "admin":"sov1335hded4gyzpt00fpz75mms4m7ck02wgw07yhw9grahj4dzg4yvqk63pml",
            "owners":[
                [0,"sov1fsgzj6t7udv8zhf6zj32mkqhcjcpv52yph5qsdcl0qt94jgdckqsczjm2y"]
            ]
        }"#;

        let parsed_config: NonFungibleTokenConfig<DefaultContext> =
            serde_json::from_str(data).unwrap();
        assert_eq!(config, parsed_config)
    }
}
