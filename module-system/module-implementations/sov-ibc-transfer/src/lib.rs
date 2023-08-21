mod call;
pub mod context;
mod genesis;

#[cfg(test)]
mod tests;

#[cfg(feature = "native")]
mod query;

pub use call::CallMessage;
use context::TransferContext;
#[cfg(feature = "native")]
pub use query::Response;
use sov_modules_api::{Error, ModuleInfo};
use sov_state::WorkingSet;

pub struct ExampleModuleConfig {}

/// A new module:
/// - Must derive `ModuleInfo`
/// - Must contain `[address]` field
/// - Can contain any number of ` #[state]` or `[module]` fields
/// - Should derive `ModuleCallJsonSchema` if the "native" feature is enabled.
///   This is optional, and is only used to generate a JSON Schema for your
///   module's call messages (which is useful to develop clients, CLI tooling
///   etc.).
#[cfg_attr(feature = "native", derive(sov_modules_api::ModuleCallJsonSchema))]
#[derive(ModuleInfo, Clone)]
pub struct Transfer<C: sov_modules_api::Context> {
    /// Address of the module.
    #[address]
    pub address: C::Address,

    /// Reference to the Bank module.
    #[module]
    pub(crate) bank: sov_bank::Bank<C>,

    /// Keeps track of the address of each token we minted, indexed by token
    /// name.
    #[state]
    pub(crate) minted_tokens: sov_state::StateMap<String, C::Address>,
}

impl<C: sov_modules_api::Context> Transfer<C> {
    pub fn into_context<'ws>(
        self,
        working_set: &'ws mut WorkingSet<C::Storage>,
    ) -> TransferContext<'ws, C> {
        TransferContext::new(self, working_set)
    }
}

impl<C: sov_modules_api::Context> sov_modules_api::Module for Transfer<C> {
    type Context = C;

    type Config = ExampleModuleConfig;

    type CallMessage = call::CallMessage;

    fn genesis(
        &self,
        config: &Self::Config,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<(), Error> {
        // The initialization logic
        Ok(self.init_module(config, working_set)?)
    }

    fn call(
        &self,
        _msg: Self::CallMessage,
        _context: &Self::Context,
        _working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse, Error> {
        todo!()
    }
}

impl<C> core::fmt::Debug for Transfer<C>
where
    C: sov_modules_api::Context,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // FIXME: put real values here, or remove `Debug` requirement from router::Module
        f.debug_struct("Transfer")
            .field("address", &self.address)
            .finish()
    }
}
