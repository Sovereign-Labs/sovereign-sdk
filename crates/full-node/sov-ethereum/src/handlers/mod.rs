use std::sync::Arc;

use alloy_primitives::{Bytes, B256, U256};

use jsonrpsee::types::{ErrorObjectOwned, Params};
use jsonrpsee::Extensions;
use sov_address::{EthereumAddress, FromVmAddress};

pub use sov_evm::EthereumAuthenticator;
use sov_evm::{Evm, RlpEvmTransaction};
use sov_modules_api::capabilities::HasKernel;
use sov_modules_api::{RawTx, Spec};
use sov_sequencer::Sequencer;

use crate::to_jsonrpsee_error_object;
use crate::Ethereum;

const ETH_RPC_ERROR: &str = "ETH_RPC_ERROR";

#[cfg(feature = "local")]
pub(crate) mod signer {
    use super::*;
    use alloy_eips::Encodable2718;
    use alloy_primitives::Address;
    use alloy_rpc_types::TransactionRequest;
    use reth_rpc_eth_types::EthApiError;
    use sov_evm::eth_api_into_rpc_error;
    use sov_modules_api::macros::config_value;

    pub async fn eth_accounts<S, Seq>(
        _: Params<'static>,
        ethereum: Arc<Ethereum<S, Seq>>,
        _: Extensions,
    ) -> Result<Vec<Address>, ErrorObjectOwned>
    where
        S: Spec,
        Seq: Sequencer<Spec = S>,
        S::Address: FromVmAddress<EthereumAddress>,
        Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
    {
        Ok(ethereum.eth_signer.signers())
    }

    pub async fn eth_send_transaction<S, Seq>(
        parameters: Params<'static>,
        ethereum: Arc<Ethereum<S, Seq>>,
        _: Extensions,
    ) -> Result<B256, ErrorObjectOwned>
    where
        S: Spec,
        Seq: Sequencer<Spec = S>,
        S::Address: FromVmAddress<EthereumAddress>,
        Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
    {
        let mut transaction_request: TransactionRequest = parameters.one().unwrap();

        let evm = Evm::<S>::default();

        // get from, return error if none
        let from = transaction_request
            .from
            .ok_or(to_jsonrpsee_error_object("No from address", ETH_RPC_ERROR))?;

        // return error if not in signers
        if !ethereum.eth_signer.signers().contains(&from) {
            return Err(to_jsonrpsee_error_object(
                "From address not in signers",
                ETH_RPC_ERROR,
            ));
        }

        let raw_evm_tx = {
            let mut state = ethereum.sequencer.api_state().default_api_state_accessor();

            // set nonce if none
            transaction_request.nonce.get_or_insert_with(|| {
                evm.get_transaction_count(from, None, &mut state)
                    .unwrap_or_default()
                    .to::<u64>()
            });

            let chain_id = evm
                .chain_id(&mut state)
                .expect("Failed to get chain id")
                .map(|id| id.to())
                .unwrap_or(config_value!("CHAIN_ID"));
            transaction_request.chain_id = Some(chain_id);

            let estimated_gas = evm.eth_estimate_gas(
                transaction_request.clone(),
                Some("pending".to_string()),
                &mut state,
            )?;
            transaction_request.gas = Some(estimated_gas.to::<u64>());

            let transaction = transaction_request
                .build_typed_tx()
                .map_err(|_| eth_api_into_rpc_error(EthApiError::TransactionConversionError))?;

            // sign transaction
            let signed_tx = ethereum
                .eth_signer
                .sign_transaction(transaction, from)
                .map_err(|e| to_jsonrpsee_error_object(e, ETH_RPC_ERROR))?;

            RlpEvmTransaction {
                rlp: signed_tx.encoded_2718(),
            }
        };
        let (tx_hash, raw_message) = ethereum
            .make_raw_tx(raw_evm_tx)
            .map_err(|e| to_jsonrpsee_error_object(e, ETH_RPC_ERROR))?;

        let tx = Seq::Rt::encode_with_ethereum_auth(RawTx::new(raw_message));

        ethereum.sequencer.accept_tx(tx).await.map_err(|e| {
            to_jsonrpsee_error_object(
                format!("{} - '{}' ({:?})", e.status, e.message, e.details),
                ETH_RPC_ERROR,
            )
        })?;

        Ok(tx_hash)
    }
}

pub async fn eth_gas_price<S, Seq>(
    _: Params<'static>,
    ethereum: Arc<Ethereum<S, Seq>>,
    _: Extensions,
) -> Result<U256, ErrorObjectOwned>
where
    S: Spec,
    Seq: Sequencer<Spec = S>,
    S::Address: FromVmAddress<EthereumAddress>,
    Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
{
    let price = {
        let mut state = ethereum.api_state_accessor();

        let suggested_tip = ethereum
            .gas_price_oracle
            .suggest_tip_cap(&mut state)
            .await
            .unwrap();

        let evm = Evm::<S>::default();
        let base_fee = U256::from(
            evm.get_block_by_number(None, None, &mut state)
                .unwrap()
                .unwrap()
                .header
                .base_fee_per_gas
                .unwrap_or_default(),
        );

        suggested_tip + base_fee
    };

    Ok(price)
}

pub async fn eth_send_raw_transaction<S, Seq>(
    parameters: Params<'static>,
    ethereum: Arc<Ethereum<S, Seq>>,
    _: Extensions,
) -> Result<B256, ErrorObjectOwned>
where
    S: Spec,
    Seq: Sequencer<Spec = S>,
    S::Address: FromVmAddress<EthereumAddress>,
    Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
{
    let data: Bytes = parameters.one().unwrap();

    let raw_evm_tx = RlpEvmTransaction { rlp: data.to_vec() };

    let (tx_hash, raw_message) = ethereum
        .make_raw_tx(raw_evm_tx)
        .map_err(|e| to_jsonrpsee_error_object(e, ETH_RPC_ERROR))?;

    let tx = Seq::Rt::encode_with_ethereum_auth(RawTx::new(raw_message));

    ethereum.sequencer.accept_tx(tx).await.map_err(|e| {
        to_jsonrpsee_error_object(
            format!("{} - '{}' ({:?})", e.status, e.message, e.details),
            ETH_RPC_ERROR,
        )
    })?;

    Ok(tx_hash)
}
