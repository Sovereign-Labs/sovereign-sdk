use alloy::consensus::{SignableTransaction, TxEip1559, TxEnvelope};
use alloy::eips::Encodable2718;
use alloy::signers::local::PrivateKeySigner;
use alloy::signers::Signer;
use alloy_primitives::{hex, Address, Bytes, TxKind, B256, U256, U64};
use alloy_rpc_types_eth::{AccessListResult, TransactionRequest};
use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use serde::Serialize;
use sov_eth_client::SimpleStorageClient;

pub async fn tx_count(
    client: &SimpleStorageClient,
    address: Address,
    block: impl Serialize,
) -> anyhow::Result<u64> {
    let count: U64 = client
        .ws
        .request("eth_getTransactionCount", rpc_params![address, block])
        .await?;
    Ok(count.to::<u64>())
}

pub async fn raw_signed_eip1559(
    signer: &PrivateKeySigner,
    chain_id: u64,
    nonce: u64,
    gas_limit: u64,
    to: TxKind,
    value: U256,
    input: Bytes,
    max_fee_per_gas: u128,
    max_priority_fee_per_gas: u128,
) -> anyhow::Result<String> {
    let tx = TxEip1559 {
        chain_id,
        nonce,
        gas_limit,
        max_fee_per_gas,
        max_priority_fee_per_gas,
        to,
        value,
        input,
        access_list: Default::default(),
    };
    let sig = signer.sign_hash(&tx.signature_hash()).await?;
    let envelope = TxEnvelope::Eip1559(tx.into_signed(sig));
    Ok(format!("0x{}", hex::encode(envelope.encoded_2718())))
}

/// Estimates gas for `tx`, asserts the sender can afford it, and sets the gas limit.
/// The affordability check includes `tx.value` in the ceiling.
pub async fn estimate_gas_and_check_affordability(
    client: &SimpleStorageClient,
    tx: &mut TransactionRequest,
    max_fee_per_gas: u128,
    sender_balance: U256,
) -> anyhow::Result<()> {
    tx.gas = None;
    let estimated_gas_limit = client.eth_estimate_gas(tx.clone()).await;
    let tx_value = tx.value.unwrap_or(U256::ZERO);
    let ceiling = U256::from(estimated_gas_limit)
        .checked_mul(U256::from(max_fee_per_gas))
        .and_then(|cost| cost.checked_add(tx_value))
        .ok_or_else(|| anyhow::anyhow!("gas affordability ceiling overflow"))?;
    assert!(
        ceiling < sender_balance,
        "test precondition failed: gas ceiling {ceiling} must be below sender balance {sender_balance}"
    );
    tx.gas = Some(estimated_gas_limit);
    Ok(())
}

pub async fn finalized_block_number_and_hash(client: &SimpleStorageClient) -> (u64, B256) {
    let finalized_block = client
        .eth_get_block_by_number(Some("finalized".to_string()))
        .await;
    (finalized_block.header.number, finalized_block.header.hash)
}

#[derive(Debug)]
pub struct EndpointResults {
    pub estimate_gas: Result<U64, jsonrpsee::core::client::Error>,
    pub call: Result<String, jsonrpsee::core::client::Error>,
    pub create_access_list: Result<AccessListResult, String>,
    pub send_raw_tx: Result<B256, jsonrpsee::core::client::Error>,
}

/// Calls all 4 simulation/submission endpoints with the same request.
/// Returns individual results without any consistency assertions.
pub async fn call_all_endpoints(
    client: &SimpleStorageClient,
    request: &TransactionRequest,
    signer: &PrivateKeySigner,
) -> EndpointResults {
    let estimate_gas: Result<U64, _> = client
        .ws
        .request("eth_estimateGas", rpc_params![request, "latest"])
        .await;

    let call: Result<String, _> = client
        .ws
        .request("eth_call", rpc_params![request, "latest"])
        .await;

    let access_list_result: Result<AccessListResult, _> = client
        .ws
        .request("eth_createAccessList", rpc_params![request, "latest"])
        .await;

    let create_access_list: Result<AccessListResult, String> = match access_list_result {
        Ok(alr) if alr.error.is_some() => Err(alr.error.unwrap()),
        Ok(alr) => Ok(alr),
        Err(e) => Err(e.to_string()),
    };

    let chain_id: U64 = client
        .ws
        .request("eth_chainId", rpc_params![])
        .await
        .unwrap();

    let nonce = match request.nonce {
        Some(n) => n,
        None => tx_count(client, signer.address(), "latest").await.unwrap(),
    };

    let max_fee = match request.gas_price.or(request.max_fee_per_gas) {
        Some(fee) => fee,
        None => {
            let gas_price: U256 = client
                .ws
                .request("eth_gasPrice", rpc_params![])
                .await
                .unwrap();
            gas_price.to::<u128>()
        }
    };

    let gas_limit = match request.gas {
        Some(g) => g,
        None => match &estimate_gas {
            Ok(estimated) => estimated.to::<u64>(),
            Err(_) => 1_000_000,
        },
    };

    let raw_tx = raw_signed_eip1559(
        signer,
        chain_id.to::<u64>(),
        nonce,
        gas_limit,
        request.to.unwrap_or(TxKind::Create),
        request.value.unwrap_or(U256::ZERO),
        request.input.input.clone().unwrap_or_default(),
        max_fee,
        request.max_priority_fee_per_gas.unwrap_or(0),
    )
    .await
    .unwrap();

    let send_raw_tx: Result<B256, _> = client
        .ws
        .request("eth_sendRawTransaction", rpc_params![&raw_tx])
        .await;

    EndpointResults {
        estimate_gas,
        call,
        create_access_list,
        send_raw_tx,
    }
}
