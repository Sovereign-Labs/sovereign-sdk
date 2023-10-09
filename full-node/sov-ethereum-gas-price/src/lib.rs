#[cfg(feature = "experimental")]
pub use experimental::{get_ethereum_gas_price_rpc, EthereumGasPrice};
#[cfg(feature = "experimental")]
mod cache;
#[cfg(feature = "experimental")]
mod gas_oracle;
#[cfg(feature = "experimental")]
pub mod experimental {
    use jsonrpsee::types::ErrorObjectOwned;
    use jsonrpsee::RpcModule;
    use reth_primitives::U256;
    use sov_evm::Evm;
    use sov_modules_api::WorkingSet;

    use crate::gas_oracle::{GasPriceOracle};

    pub use crate::gas_oracle::GasPriceOracleConfig;

    pub fn get_ethereum_gas_price_rpc<C: sov_modules_api::Context>(
        gas_price_oracle_config: GasPriceOracleConfig,
        storage: C::Storage,
    ) -> RpcModule<EthereumGasPrice<C>> {
        let mut rpc = RpcModule::new(EthereumGasPrice::new(gas_price_oracle_config, storage));

        register_rpc_methods(&mut rpc).expect("Failed to register sequencer RPC methods");
        rpc
    }

    pub struct EthereumGasPrice<C: sov_modules_api::Context> {
        gas_price_oracle: GasPriceOracle<C>,
        storage: C::Storage,
    }

    impl<C: sov_modules_api::Context> EthereumGasPrice<C> {
        fn new(gas_price_oracle_config: GasPriceOracleConfig, storage: C::Storage) -> Self {
            let gas_price_oracle =
                GasPriceOracle::new(Evm::<C>::default(), gas_price_oracle_config);
            Self {
                gas_price_oracle,
                storage,
            }
        }
    }

    fn register_rpc_methods<C: sov_modules_api::Context>(
        rpc: &mut RpcModule<EthereumGasPrice<C>>,
    ) -> Result<(), jsonrpsee::core::Error> {
        rpc.register_async_method("eth_gasPrice", |_, ethereum| async move {
            let price = {
                let mut working_set = WorkingSet::<C>::new(ethereum.storage.clone());

                let suggested_tip = ethereum
                    .gas_price_oracle
                    .suggest_tip_cap(&mut working_set)
                    .await
                    .unwrap();

                let evm = Evm::<C>::default();
                let base_fee = evm
                    .get_block_by_number(None, None, &mut working_set)
                    .unwrap()
                    .unwrap()
                    .header
                    .base_fee_per_gas
                    .unwrap_or_default();

                suggested_tip + base_fee
            };

            Ok::<U256, ErrorObjectOwned>(price)
        })?;

        Ok(())
    }
}
