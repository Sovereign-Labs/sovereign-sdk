use alloy_consensus::{TxEip1559, TxEip2930, TxLegacy, TypedTransaction};
use alloy_eips::eip2718::Encodable2718;
use alloy_primitives::TxKind;
use alloy_rpc_types::TransactionRequest;
use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::RpcModule;
use reth_rpc_eth_types::EthApiError;
use sov_address::{EthereumAddress, FromVmAddress};
use sov_evm::{eth_api_into_rpc_error, EthereumAuthenticator, Evm, RlpEvmTransaction};
use sov_modules_api::capabilities::HasKernel;
use sov_modules_api::macros::config_value;
use sov_modules_api::{ApiStateAccessor, RawTx, Spec};
use sov_sequencer::Sequencer;

use crate::{to_jsonrpsee_error_object, Ethereum, ETH_RPC_ERROR};

fn config_chain_id() -> u64 {
    config_value!("CHAIN_ID")
}

pub fn register_signer_rpc_methods<S, Seq>(
    rpc: &mut RpcModule<Ethereum<S, Seq>>,
) -> Result<(), jsonrpsee::core::client::Error>
where
    S: Spec,
    Seq: Sequencer<Spec = S>,
    S::Address: FromVmAddress<EthereumAddress>,
    Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
{
    rpc.register_async_method("eth_accounts", |_parameters, ethereum, _| async move {
        Ok::<_, ErrorObjectOwned>(ethereum.eth_signer.signers())
    })?;
    rpc.register_async_method(
        "eth_sendTransaction",
        |parameters, ethereum, _| async move {
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
                if transaction_request.nonce.is_none() {
                    let nonce = evm
                        .get_transaction_count(from, None, &mut state)
                        .unwrap_or_default();

                    transaction_request.nonce = Some(nonce.to());
                }

                let transaction =
                    to_typed_transaction_request(transaction_request, &evm, &mut state)?;

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

            Ok::<_, ErrorObjectOwned>(tx_hash)
        },
    )?;
    Ok(())
}

fn to_typed_transaction_request<S: sov_modules_api::Spec>(
    transaction_request: TransactionRequest,
    evm: &Evm<S>,
    state: &mut ApiStateAccessor<S>,
) -> Result<TypedTransaction, ErrorObjectOwned>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    let chain_id = evm
        .chain_id(state)
        .expect("Failed to get chain id")
        .map(|id| id.to())
        .unwrap_or(config_chain_id());

    let estimated_gas = evm.eth_estimate_gas(
        transaction_request.clone(),
        Some("pending".to_string()),
        state,
    )?;

    let gas_limit = estimated_gas.to::<u64>();

    let transaction = build_tx(transaction_request, chain_id, gas_limit)?;

    Ok(transaction)
}

fn build_tx(
    transaction_request: TransactionRequest,
    chain_id: u64,
    gas_limit: u64,
) -> Result<TypedTransaction, ErrorObjectOwned> {
    let TransactionRequest {
        to,
        gas_price,
        max_fee_per_gas,
        max_priority_fee_per_gas,
        value,
        input: data,
        nonce,
        mut access_list,
        max_fee_per_blob_gas,
        blob_versioned_hashes,
        sidecar,
        ..
    } = transaction_request;

    let transaction = match (
        gas_price,
        max_fee_per_gas,
        access_list.take(),
        max_fee_per_blob_gas,
        blob_versioned_hashes,
        sidecar,
    ) {
        // legacy transaction
        // gas price required
        (Some(_), None, None, None, None, None) => TypedTransaction::Legacy(TxLegacy {
            nonce: nonce.unwrap_or_default(),
            gas_price: gas_price.unwrap_or_default(),
            gas_limit,
            value: value.unwrap_or_default(),
            input: data.into_input().unwrap_or_default(),
            to: to.unwrap_or(TxKind::Create),
            chain_id: Some(chain_id),
        }),
        // EIP2930
        // if only access_list is set, and no eip1599 fees
        (_, None, Some(access_list), None, None, None) => TypedTransaction::Eip2930(TxEip2930 {
            nonce: nonce.unwrap_or_default(),
            gas_price: gas_price.unwrap_or_default(),
            gas_limit,
            value: value.unwrap_or_default(),
            input: data.into_input().unwrap_or_default(),
            to: to.unwrap_or(TxKind::Create),
            chain_id,
            access_list,
        }),
        // EIP1559
        // if 4844 fields missing
        // gas_price, max_fee_per_gas, access_list,
        // max_fee_per_blob_gas, blob_versioned_hashes,
        // sidecar,
        (None, _, _, None, None, None) => {
            // Empty fields fall back to the canonical transaction schema.
            TypedTransaction::Eip1559(TxEip1559 {
                nonce: nonce.unwrap_or_default(),
                max_fee_per_gas: max_fee_per_gas.unwrap_or_default(),
                max_priority_fee_per_gas: max_priority_fee_per_gas.unwrap_or_default(),
                gas_limit,
                value: value.unwrap_or_default(),
                input: data.into_input().unwrap_or_default(),
                to: to.unwrap_or(TxKind::Create),
                chain_id,
                access_list: access_list.unwrap_or_default(),
            })
        }
        // EIP-4844
        (None, _, _, Some(_), Some(_), Some(_)) => {
            return Err(eth_api_into_rpc_error(EthApiError::Unsupported(
                "EIP-4844 is not supported",
            )))
        }
        _ => {
            return Err(sov_evm::eth_api_into_rpc_error(
                EthApiError::ConflictingFeeFieldsInRequest,
            ))
        }
    };
    Ok(transaction)
}
