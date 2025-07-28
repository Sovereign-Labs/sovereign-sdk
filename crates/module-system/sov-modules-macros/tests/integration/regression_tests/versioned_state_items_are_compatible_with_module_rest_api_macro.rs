//! Regression for <https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/1727>.
//!
//! See also <https://github.com/Sovereign-Labs/sovereign-sdk-wip/pull/1824>.

use sov_modules_api::{Module, ModuleId, ModuleInfo, ModuleRestApi, Spec, VersionedStateValue};

#[derive(Clone, ModuleInfo, ModuleRestApi)]
struct TestModule<S: Spec> {
    #[id]
    id: ModuleId,

    #[allow(unused)]
    #[rest_api(include)]
    #[state]
    state_value: VersionedStateValue<u64>,

    #[phantom]
    phantom: std::marker::PhantomData<S>,
}

impl<S: Spec> Module for TestModule<S> {
    type Spec = S;
    type Config = ();
    type CallMessage = ();
    type Event = ();

    fn call(
        &mut self,
        _message: Self::CallMessage,
        _context: &sov_modules_api::Context<Self::Spec>,
        _state: &mut impl sov_modules_api::TxState<Self::Spec>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}
