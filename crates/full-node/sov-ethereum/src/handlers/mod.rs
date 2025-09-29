mod get_logs;
mod subscribe;
use alloy_primitives::{Bytes, B256};
use alloy_rpc_types::TransactionReceipt;
pub use get_logs::eth_get_logs;
use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::types::Params as JRpcParams;
use jsonrpsee::Extensions;
use sov_address::{EthereumAddress, FromVmAddress};
pub use sov_evm::EthereumAuthenticator;
#[cfg(feature = "local")]
use sov_evm::Evm;
use sov_evm::RlpEvmTransaction;
use sov_modules_api::capabilities::AuthorizationData;
use sov_modules_api::capabilities::HasKernel;
use sov_modules_api::capabilities::TransactionAuthenticator;
use sov_modules_api::capabilities::UniquenessData;
use sov_modules_api::Runtime;
use sov_modules_api::{RawTx, Spec};
use sov_sequencer::Sequencer;
use std::sync::Arc;
pub use subscribe::eth_subscribe;

use crate::to_jsonrpsee_error_object;
use crate::Ethereum;

const ETH_RPC_ERROR: &str = "ETH_RPC_ERROR";
/// Txs with nonce in the future of more than this threshold are rejected immediately. If the nonce is in the future but below the threshold, we'll buffer it
/// for a little while.
const FUTURE_NONCE_THRESHOLD: u64 = 100;
/// How long to wait between retries.
const SLEEP_DURATION_MS: u64 = 10;
/// The maximum number of times to fetch the nonce and retry.
const MAX_RETRIES: u32 = 10;
/// The maximum amount of time to buffer a tx with a future nonce. Provides an upper bound in case retry attempts are taking too long.
const MAX_BUFFER_DURATION_MS: u128 = 200;

async fn process_raw_transaction<S, Seq, T, F>(
    data: Bytes,
    ethereum: Arc<Ethereum<S, Seq>>,
    on_success: F,
) -> Result<T, ErrorObjectOwned>
where
    S: Spec,
    Seq: Sequencer<Spec = S>,
    S::Address: FromVmAddress<EthereumAddress>,
    Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
    F: Fn(B256, Arc<Ethereum<S, Seq>>) -> Result<T, ErrorObjectOwned>,
{
    let raw_evm_tx = RlpEvmTransaction { rlp: data.to_vec() };
    let (tx_hash, raw_message) = ethereum
        .make_raw_tx(raw_evm_tx)
        .map_err(|e| to_jsonrpsee_error_object(e, ETH_RPC_ERROR))?;

    // Authenticate the transaction so that we can get the credential ID and nonce.
    let tx = Seq::Rt::encode_with_ethereum_auth(RawTx::new(raw_message));
    let mut state = ethereum
        .sequencer
        .api_state()
        .default_api_state_accessor()
        .to_provable_reader();
    let (_decoded_tx, auth_data, _call) =
        <Seq::Rt as Runtime<S>>::Auth::authenticate(&tx, &mut state).map_err(|e| {
            to_jsonrpsee_error_object(format!("Authentication failed: {e}"), ETH_RPC_ERROR)
        })?;
    let mut state = state.api_state_accessor;
    let AuthorizationData {
        credential_id,
        uniqueness,
        ..
    } = auth_data;
    drop(auth_data); // Drop the authorization data because it's not `Send`, so we can't hold it across retries.
    let retries = if ethereum.buffer_raw_txs {
        MAX_RETRIES
    } else {
        0
    };
    let start = std::time::Instant::now();
    for _ in 0..retries {
        match uniqueness {
            UniquenessData::Nonce(nonce) => {
                let expected_nonce = sov_uniqueness::Uniqueness::<S>::default()
                    .nonce(&credential_id, &mut state)?
                    .unwrap_or_default();
                if nonce == expected_nonce {
                    ethereum.sequencer.accept_tx(tx).await.map_err(|e| {
                        to_jsonrpsee_error_object(
                            format!("{} - '{}' ({:?})", e.status, e.message, e.details),
                            ETH_RPC_ERROR,
                        )
                    })?;

                    return on_success(tx_hash, ethereum);
                } else if nonce < expected_nonce {
                    return Err(to_jsonrpsee_error_object(
                        format!("Nonce error: nonce {nonce} has already been used"),
                        ETH_RPC_ERROR,
                    ));
                } else if nonce > (expected_nonce + FUTURE_NONCE_THRESHOLD) {
                    return Err(to_jsonrpsee_error_object(
                        format!(
                            "Nonce error: Provided nonce {nonce} is in the future. Expected nonce is {expected_nonce}",
                        ),
                        ETH_RPC_ERROR,
                    ));
                }
            }
            _ => {
                return Err(to_jsonrpsee_error_object(
                    "Invalid uniqueness data",
                    ETH_RPC_ERROR,
                ));
            }
        }
        // tokio::time::sleep can have unreliable timing under load, so if the total time we've been retrying is too large we'll break the loop early.
        if start.elapsed().as_millis() > MAX_BUFFER_DURATION_MS {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(SLEEP_DURATION_MS)).await;
        state = ethereum.sequencer.api_state().default_api_state_accessor();
    }

    // Once we've exhausted all retries, make one "regular" attempt to accept the transaction.
    // This ensures that every tx is attempted at least once even if MAX_RETRIES is set to zero
    // and provides some protection against spuriously rejecting txs in case our view of the state was stale.
    ethereum.sequencer.accept_tx(tx).await.map_err(|e| {
        to_jsonrpsee_error_object(
            format!("{} - '{}' ({:?})", e.status, e.message, e.details),
            ETH_RPC_ERROR,
        )
    })?;

    on_success(tx_hash, ethereum)
}

#[cfg(feature = "local")]
pub(crate) mod signer {
    use super::*;
    use alloy_eips::Encodable2718;
    use alloy_primitives::Address;
    use alloy_rpc_types::TransactionRequest;
    use sov_evm::eth_api_into_rpc_error;
    use sov_modules_api::macros::config_value;
    use sov_rpc_eth_types::EthApiError;

    pub async fn eth_accounts<S, Seq>(
        _: JRpcParams<'static>,
        ethereum: Arc<Ethereum<S, Seq>>,
        _: Extensions,
    ) -> Result<Vec<Address>, ErrorObjectOwned>
    where
        S: Spec,
        Seq: Sequencer<Spec = S>,
        S::Address: FromVmAddress<EthereumAddress>,
        Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
    {
        Ok(ethereum.eth_signer.addresses())
    }

    pub async fn eth_send_transaction<S, Seq>(
        parameters: JRpcParams<'static>,
        ethereum: Arc<Ethereum<S, Seq>>,
        _: Extensions,
    ) -> Result<B256, ErrorObjectOwned>
    where
        S: Spec,
        Seq: Sequencer<Spec = S>,
        S::Address: FromVmAddress<EthereumAddress>,
        Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
    {
        let mut transaction_request: TransactionRequest = parameters.one()?;

        let evm = Evm::<S>::default();

        // get from, return error if none
        let from = transaction_request
            .from
            .ok_or(to_jsonrpsee_error_object("No from address", ETH_RPC_ERROR))?;

        // return error if not in signers
        if !ethereum.eth_signer.addresses().contains(&from) {
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
                .sign_transaction(transaction, &from)
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

pub async fn eth_send_raw_transaction<S, Seq>(
    parameters: JRpcParams<'static>,
    ethereum: Arc<Ethereum<S, Seq>>,
    _: Extensions,
) -> Result<B256, ErrorObjectOwned>
where
    S: Spec,
    Seq: Sequencer<Spec = S>,
    S::Address: FromVmAddress<EthereumAddress>,
    Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
{
    let data: Bytes = parameters.one()?;

    process_raw_transaction(data, ethereum, |tx_hash, _| Ok(tx_hash)).await
}

pub async fn realtime_send_raw_transaction<S, Seq>(
    parameters: JRpcParams<'static>,
    ethereum: Arc<Ethereum<S, Seq>>,
    _: Extensions,
) -> Result<Option<TransactionReceipt>, ErrorObjectOwned>
where
    S: Spec,
    Seq: Sequencer<Spec = S>,
    S::Address: FromVmAddress<EthereumAddress>,
    Seq::Rt: HasKernel<S> + EthereumAuthenticator<S> + Default + Send + Sync + 'static,
{
    let data: Bytes = parameters.one()?;

    process_raw_transaction(data, ethereum, |tx_hash, ethereum| {
        let evm = sov_evm::Evm::<S>::default();
        evm.get_transaction_receipt(
            tx_hash,
            &mut ethereum.sequencer.api_state().default_api_state_accessor(),
        )
    })
    .await
}
