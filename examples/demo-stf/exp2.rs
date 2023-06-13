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
    pub type ZkAppRunner<Vm> = DemoAppRunner<ZkDefaultContext, Vm>;
    use crate::batch_builder::FiFoStrictBatchBuilder;
    #[cfg(feature = "native")]
    use sov_bank::query::{BankRpcImpl, BankRpcServer};
    #[cfg(feature = "native")]
    use sov_election::query::{ElectionRpcImpl, ElectionRpcServer};
    #[cfg(feature = "native")]
    use sov_modules_macros::expose_rpc;
    #[cfg(feature = "native")]
    use sov_value_setter::query::{ValueSetterRpcImpl, ValueSetterRpcServer};
    #[cfg(feature = "native")]
    pub type NativeAppRunner<Vm> = DemoAppRunner<DefaultContext, Vm>;
    pub type DemoApp<C, Vm> = AppTemplate<C, Runtime<C>, Vm>;
    /// Batch receipt type used by the demo app. We export this type so that it's easily accessible to the full node.
    pub type DemoBatchReceipt = SequencerOutcome;
    /// Tx receipt type used by the demo app. We export this type so that it's easily accessible to the full node.
    pub type DemoTxReceipt = TxEffect;
    #[cfg(feature = "native")]
    #[cfg(feature = "native")]
    #[cfg(feature = "native")]
    #[cfg(feature = "native")]
    impl<Vm: Zkvm> StateTransitionRunner<ProverConfig, Vm> for DemoAppRunner<DefaultContext, Vm> {
        type RuntimeConfig = Config;
        type Inner = DemoApp<DefaultContext, Vm>;
        type BatchBuilder = FiFoStrictBatchBuilder<Runtime<DefaultContext>, DefaultContext>;
        type Error = anyhow::Error;
        fn new(runtime_config: Self::RuntimeConfig) -> Self {
            let storage = ProverStorage::with_config(runtime_config.storage)
                .expect("Failed to open prover storage");
            let app = AppTemplate::new(storage.clone(), Runtime::default());
            let batch_size_bytes = 1024 * 100;
            let batch_builder = FiFoStrictBatchBuilder::new(
                batch_size_bytes,
                u32::MAX as usize,
                Runtime::default(),
                storage,
            );
            Self {
                stf: app,
                batch_builder: Some(batch_builder),
            }
        }
        fn inner(&self) -> &Self::Inner {
            &self.stf
        }
        fn inner_mut(&mut self) -> &mut Self::Inner {
            &mut self.stf
        }
        fn take_batch_builder(&mut self) -> Result<Self::BatchBuilder, Self::Error> {
            self.batch_builder
                .take()
                .ok_or(::anyhow::__private::must_use({
                    let error = ::anyhow::__private::format_err(format_args!(
                        "Batch builder already taken"
                    ));
                    error
                }))
        }
    }
    pub struct RpcStorage<C: Context> {
        pub storage: C::Storage,
    }
    #[automatically_derived]
    impl<C: ::core::clone::Clone + Context> ::core::clone::Clone for RpcStorage<C>
    where
        C::Storage: ::core::clone::Clone,
    {
        #[inline]
        fn clone(&self) -> RpcStorage<C> {
            RpcStorage {
                storage: ::core::clone::Clone::clone(&self.storage),
            }
        }
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
    impl<Vm: Zkvm> StateTransitionRunner<ZkConfig, Vm> for DemoAppRunner<ZkDefaultContext, Vm> {
        type RuntimeConfig = [u8; 32];
        type Inner = DemoApp<ZkDefaultContext, Vm>;
        type BatchBuilder = FiFoStrictBatchBuilder<Runtime<ZkDefaultContext>, ZkDefaultContext>;
        type Error = anyhow::Error;
        fn new(runtime_config: Self::RuntimeConfig) -> Self {
            let storage =
                ZkStorage::with_config(runtime_config).expect("Failed to open zk storage");
            let app: AppTemplate<ZkDefaultContext, Runtime<ZkDefaultContext>, Vm> =
                AppTemplate::new(storage.clone(), Runtime::default());
            let batch_size_bytes = 1024 * 100;
            let batch_builder = FiFoStrictBatchBuilder::new(
                batch_size_bytes,
                u32::MAX as usize,
                Runtime::default(),
                storage,
            );
            Self {
                stf: app,
                batch_builder: Some(batch_builder),
            }
        }
        fn inner(&self) -> &Self::Inner {
            &self.stf
        }
        fn inner_mut(&mut self) -> &mut Self::Inner {
            &mut self.stf
        }
        fn take_batch_builder(&mut self) -> Result<Self::BatchBuilder, Self::Error> {
            self.batch_builder
                .take()
                .ok_or(::anyhow::__private::must_use({
                    let error = ::anyhow::__private::format_err(format_args!(
                        "Batch builder already taken"
                    ));
                    error
                }))
        }
    }
    #[cfg(feature = "native")]
    impl<Vm: Zkvm> RpcRunner for DemoAppRunner<DefaultContext, Vm> {
        type Context = DefaultContext;
        fn get_storage(&self) -> <Self::Context as Spec>::Storage {
            self.inner().current_storage.clone()
        }
    }
}
