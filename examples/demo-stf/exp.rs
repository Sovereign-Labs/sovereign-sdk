pub mod app {
    #[cfg(feature = "native")]
    use crate::runner_config::Config;
    use crate::runtime::Runtime;
    #[cfg(feature = "native")]
    pub use sov_modules_api::default_context::DefaultContext;
    pub use sov_modules_api::default_context::ZkDefaultContext;
    #[cfg(feature = "native")]
    pub use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
    use sov_modules_api::Context;
    #[cfg(feature = "native")]
    use sov_modules_api::RpcRunner;
    #[cfg(feature = "native")]
    use sov_modules_api::Spec;
    use sov_modules_stf_template::AppTemplate;
    pub use sov_modules_stf_template::Batch;
    use sov_modules_stf_template::SequencerOutcome;
    use sov_modules_stf_template::TxEffect;
    use sov_rollup_interface::services::stf_runner::StateTransitionRunner;
    #[cfg(feature = "native")]
    use sov_rollup_interface::stf::ProverConfig;
    use sov_rollup_interface::stf::ZkConfig;
    use sov_rollup_interface::zk::traits::Zkvm;
    #[cfg(feature = "native")]
    use sov_state::ProverStorage;
    use sov_state::Storage;
    use sov_state::ZkStorage;
    pub struct DemoAppRunner<C: Context, Vm: Zkvm> {
        pub stf: DemoApp<C, Vm>,
        pub batch_builder: Option<FiFoStrictBatchBuilder<Runtime<C>, C>>,
    }

    pub struct RpcStorage<C: Context> {
        pub storage: C::Storage,
    }

    impl BankRpcImpl<DefaultContext> for RpcStorage<DefaultContext> {
        fn get_working_set(
            &self,
        ) -> ::sov_state::WorkingSet<<DefaultContext as ::sov_modules_api::Spec>::Storage> {
            ::sov_state::WorkingSet::new(self.storage.clone())
        }
    }
    impl ElectionRpcImpl<DefaultContext> for RpcStorage<DefaultContext> {
        fn get_working_set(
            &self,
        ) -> ::sov_state::WorkingSet<<DefaultContext as ::sov_modules_api::Spec>::Storage> {
            ::sov_state::WorkingSet::new(self.storage.clone())
        }
    }
    impl ValueSetterRpcImpl<DefaultContext> for RpcStorage<DefaultContext> {
        fn get_working_set(
            &self,
        ) -> ::sov_state::WorkingSet<<DefaultContext as ::sov_modules_api::Spec>::Storage> {
            ::sov_state::WorkingSet::new(self.storage.clone())
        }
    }
    pub fn get_rpc_methods(
        storj: <DefaultContext as ::sov_modules_api::Spec>::Storage,
    ) -> jsonrpsee::RpcModule<()> {
        let mut module = jsonrpsee::RpcModule::new(());
        let r = RpcStorage {
            storage: storj.clone(),
        };
        module.merge(BankRpcServer::into_rpc(r.clone())).unwrap();
        module
            .merge(ElectionRpcServer::into_rpc(r.clone()))
            .unwrap();
        module
            .merge(ValueSetterRpcServer::into_rpc(r.clone()))
            .unwrap();
        module
    }

    #[cfg(feature = "native")]
    impl<Vm: Zkvm> RpcRunner for DemoAppRunner<DefaultContext, Vm> {
        type Context = DefaultContext;
        fn get_storage(&self) -> <Self::Context as Spec>::Storage {
            self.inner().current_storage.clone()
        }
    }
}
