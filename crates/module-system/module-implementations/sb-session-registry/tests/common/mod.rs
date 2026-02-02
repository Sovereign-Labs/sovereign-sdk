// Mock DEX module used for testing session enforcement APIs.

mod test_dex {
    use anyhow::Result;
    use schemars::JsonSchema;
    use sov_modules_api::macros::{serialize, UniversalWallet};
    use sov_modules_api::{
        Context, GenesisState, Module, ModuleId, ModuleInfo, ModuleRestApi, Spec, TxState,
    };

    use sb_session_registry::SessionRegistry;

    #[derive(Clone, Debug, PartialEq, Eq)]
    #[serialize(Serde)]
    pub struct DexConfig {}

    #[derive(Debug, Clone, PartialEq, Eq, JsonSchema, UniversalWallet)]
    #[serialize(Borsh, Serde)]
    #[schemars(bound = "S: Spec", rename = "DexCallMessage")]
    #[serde(rename_all = "snake_case")]
    pub enum DexCallMessage<S: Spec> {
        EnforceSessionActive { wallet: S::Address },
        EnforceSessionPresent { wallet: S::Address },
    }

    #[derive(Clone, ModuleInfo, ModuleRestApi)]
    pub struct TestDex<S: Spec> {
        #[id]
        pub id: ModuleId,

        #[module]
        pub session_registry: SessionRegistry<S>,
    }

    impl<S: Spec> Module for TestDex<S> {
        type Spec = S;
        type Config = DexConfig;
        type CallMessage = DexCallMessage<S>;
        type Event = ();
        type Error = anyhow::Error;

        fn genesis(
            &mut self,
            _header: &<S::Da as sov_modules_api::DaSpec>::BlockHeader,
            _config: &Self::Config,
            _state: &mut impl GenesisState<S>,
        ) -> Result<()> {
            Ok(())
        }

        fn call(
            &mut self,
            msg: Self::CallMessage,
            _ctx: &Context<S>,
            state: &mut impl TxState<S>,
        ) -> Result<()> {
            match msg {
                DexCallMessage::EnforceSessionActive { wallet } => {
                    self.session_registry.enforce_session_active(&wallet, state)
                }
                DexCallMessage::EnforceSessionPresent { wallet } => self
                    .session_registry
                    .enforce_session_present(&wallet, state),
            }
        }
    }
}

pub use test_dex::{DexCallMessage, DexConfig, TestDex};
