use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::RpcModule;
use reth_primitives::{TxKind, U256};
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
            let mut transaction_request: reth_rpc_types::TransactionRequest =
                parameters.one().unwrap();

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
                    rlp: signed_tx.envelope_encoded().to_vec(),
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
    transaction_request: reth_rpc_types::TransactionRequest,
    evm: &Evm<S>,
    state: &mut ApiStateAccessor<S>,
) -> Result<reth_rpc_types::TypedTransactionRequest, ErrorObjectOwned>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    let chain_id = evm
        .chain_id(state)
        .expect("Failed to get chain id")
        .map(|id| id.to())
        .unwrap_or(config_chain_id());

    let gas_price = transaction_request.gas_price.unwrap_or_default();

    if transaction_request.from.is_none() {
        return Err(to_jsonrpsee_error_object("No from address", ETH_RPC_ERROR));
    }

    let estimated_gas = evm.eth_estimate_gas(
        reth_rpc_types::TransactionRequest {
            from: transaction_request.from,
            to: transaction_request.to,
            gas: transaction_request.gas,
            gas_price: Some(gas_price),
            max_fee_per_gas: None,
            value: transaction_request.value,
            input: transaction_request.input.clone(),
            nonce: transaction_request.nonce,
            chain_id: Some(chain_id),
            access_list: transaction_request.access_list.clone(),
            max_priority_fee_per_gas: None,
            transaction_type: None,
            blob_versioned_hashes: None,
            max_fee_per_blob_gas: None,
            ..Default::default()
        },
        Some("pending".to_string()),
        state,
    )?;

    let gas_limit = estimated_gas.to::<U256>();

    let reth_rpc_types::TransactionRequest {
        to,
        gas_price,
        max_fee_per_gas,
        max_priority_fee_per_gas,
        gas,
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
        (Some(_), None, None, None, None, None) => {
            Some(reth_rpc_types::TypedTransactionRequest::Legacy(
                reth_rpc_types::transaction::LegacyTransactionRequest {
                    nonce: nonce.unwrap_or_default(),
                    gas_price: U256::from(gas_price.unwrap_or_default()),
                    gas_limit: U256::from(gas.unwrap_or_default()),
                    value: value.unwrap_or_default(),
                    input: data.into_input().unwrap_or_default(),
                    kind: to.unwrap_or(TxKind::Create),
                    chain_id: None,
                },
            ))
        }
        // EIP2930
        // if only access_list is set, and no eip1599 fees
        (_, None, Some(access_list), None, None, None) => {
            Some(reth_rpc_types::TypedTransactionRequest::EIP2930(
                reth_rpc_types::transaction::EIP2930TransactionRequest {
                    nonce: nonce.unwrap_or_default(),
                    gas_price: U256::from(gas_price.unwrap_or_default()),
                    gas_limit: U256::from(gas.unwrap_or_default()),
                    value: value.unwrap_or_default(),
                    input: data.into_input().unwrap_or_default(),
                    kind: to.unwrap_or(TxKind::Create),
                    chain_id: config_value!("CHAIN_ID"),
                    access_list,
                },
            ))
        }
        // EIP1559
        // if 4844 fields missing
        // gas_price, max_fee_per_gas, access_list,
        // max_fee_per_blob_gas, blob_versioned_hashes,
        // sidecar,
        (None, _, _, None, None, None) => {
            // Empty fields fall back to the canonical transaction schema.
            Some(reth_rpc_types::TypedTransactionRequest::EIP1559(
                reth_rpc_types::transaction::EIP1559TransactionRequest {
                    nonce: nonce.unwrap_or_default(),
                    max_fee_per_gas: U256::from(max_fee_per_gas.unwrap_or_default()),
                    max_priority_fee_per_gas: U256::from(
                        max_priority_fee_per_gas.unwrap_or_default(),
                    ),
                    gas_limit: U256::from(gas.unwrap_or_default()),
                    value: value.unwrap_or_default(),
                    input: data.into_input().unwrap_or_default(),
                    kind: to.unwrap_or(TxKind::Create),
                    chain_id: config_value!("CHAIN_ID"),
                    access_list: access_list.unwrap_or_default(),
                },
            ))
        }
        // EIP-4844
        (None, _, _, Some(_), Some(_), Some(_)) => {
            return Err(eth_api_into_rpc_error(EthApiError::Unsupported(
                "EIP-4844 is not supported",
            )))
        }
        _ => None,
    };

    Ok(match transaction {
        Some(reth_rpc_types::TypedTransactionRequest::Legacy(mut m)) => {
            m.chain_id = Some(chain_id);
            m.gas_limit = gas_limit;
            m.gas_price = U256::from(gas_price.unwrap_or_default());

            reth_rpc_types::TypedTransactionRequest::Legacy(m)
        }
        Some(reth_rpc_types::TypedTransactionRequest::EIP2930(mut m)) => {
            m.chain_id = chain_id;
            m.gas_limit = gas_limit;
            m.gas_price = U256::from(gas_price.unwrap_or_default());

            reth_rpc_types::TypedTransactionRequest::EIP2930(m)
        }
        Some(reth_rpc_types::TypedTransactionRequest::EIP1559(mut m)) => {
            m.chain_id = chain_id;
            m.gas_limit = gas_limit;
            m.max_fee_per_gas = U256::from(max_fee_per_gas.unwrap_or_default());

            reth_rpc_types::TypedTransactionRequest::EIP1559(m)
        }
        Some(reth_rpc_types::TypedTransactionRequest::EIP4844(_)) => {
            return Err(sov_evm::eth_api_into_rpc_error(EthApiError::Unsupported(
                "EIP-4844 is not supported",
            )))
        }
        None => {
            return Err(sov_evm::eth_api_into_rpc_error(
                EthApiError::ConflictingFeeFieldsInRequest,
            ))
        }
    })
}
