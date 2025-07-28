use anyhow::Result;
use sov_modules_api::{GenesisState, Spec};

use super::Mailbox;
use crate::{HyperlaneAddress, Recipient};

impl<S: Spec, R: Recipient<S>> Mailbox<S, R>
where
    S::Address: HyperlaneAddress,
{
    /// Initializes module with the `admin` role.
    pub(crate) fn init_module(
        &mut self,
        _admin_config: &<Self as sov_modules_api::Module>::Config,
        _state: &mut impl GenesisState<S>,
    ) -> Result<()> {
        Ok(())
    }
}
