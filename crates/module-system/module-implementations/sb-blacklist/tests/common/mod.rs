// Mock DEX module used for testing blacklist enforcement APIs.

mod test_dex {
    use anyhow::Result;
    use schemars::JsonSchema;
    use sov_modules_api::macros::{serialize, UniversalWallet};
    use sov_modules_api::{
        Context, GenesisState, Module, ModuleId, ModuleInfo, ModuleRestApi, Spec,
        TxState,
    };

    use sb_blacklist::Blacklist;

    #[derive(Clone, Debug, PartialEq, Eq)]
    #[serialize(Serde)]
    pub struct DexConfig {}

    #[derive(Debug, Clone, PartialEq, Eq, JsonSchema, UniversalWallet)]
    #[serialize(Borsh, Serde)]
    #[schemars(bound = "S: Spec", rename = "DexCallMessage")]
    #[serde(rename_all = "snake_case")]
    pub enum DexCallMessage<S: Spec> {
        EnforceNotBlacklisted { wallet: S::Address },
    }

    #[derive(Clone, ModuleInfo, ModuleRestApi)]
    pub struct TestDex<S: Spec> {
        #[id]
        pub id: ModuleId,

        #[module]
        pub blacklist: Blacklist<S>,
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
                DexCallMessage::EnforceNotBlacklisted { wallet } => {
                    self.blacklist
                        .enforce_not_blacklisted(&wallet, state)
                }
            }
        }
    }
}

pub use test_dex::{DexCallMessage, DexConfig, TestDex};
