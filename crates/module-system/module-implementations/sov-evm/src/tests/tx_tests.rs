use std::str::FromStr;

use alloy_consensus::{EthereumTxEnvelope, Signed, TxEip1559};
use alloy_primitives::{Address, Signature, TxKind, U256};
use alloy_rpc_types::TransactionRequest;
use ethers_core::types::transaction::eip2718::TypedTransaction;
use ethers_core::types::{Bytes, Eip1559TransactionRequest};
use ethers_core::utils::rlp::Rlp;
use ethers_signers::{LocalWallet, Signer};
use reth_primitives::{Recovered, TransactionSigned};
use revm::context::{BlockEnv, TransactTo, TransactionType, TxEnv};
use sov_modules_api::macros::config_value;

use crate::evm::primitive_types::TransactionSignedAndRecovered;
use crate::helpers::prepare_call_env;
use crate::primitive_types::Block;

#[tokio::test(flavor = "multi_thread")]
async fn tx_rlp_encoding_test() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = "dcf2cbdd171a21c480aa7f53d77f31bb102282b3ff099c78e3118b37348c72f7"
        .parse::<LocalWallet>()?;
    let from_addr = wallet.address();
    let to_addr =
        ethers_core::types::Address::from_str("0x0aa7420c43b8c1a7b165d216948870c8ecfe1ee1")?;
    let data: Bytes = Bytes::from_str(
        "0x6ecd23060000000000000000000000000000000000000000000000000000000000000002",
    )?;

    let tx_request = Eip1559TransactionRequest::new()
        .from(from_addr)
        .chain_id(config_value!("CHAIN_ID"))
        .nonce(0u64)
        .max_priority_fee_per_gas(413047990155u64)
        .max_fee_per_gas(768658734568u64)
        .gas(184156u64)
        .to(to_addr)
        .value(2000000000000u64)
        .data(data);

    let tx = TypedTransaction::Eip1559(tx_request);

    let sig = wallet.sign_transaction(&tx).await?;
    sig.verify(tx.sighash(), wallet.address())?;

    let rlp_bytes = tx.rlp_signed(&sig);
    let rlp_encoded = Rlp::new(&rlp_bytes);

    let (decoded_tx, decoded_sig) = TypedTransaction::decode_signed(&rlp_encoded)?;
    decoded_sig.verify(decoded_tx.sighash(), wallet.address())?;

    assert_eq!(tx, decoded_tx);
    Ok(())
}

#[test]
fn tx_conversion() {
    let signer = Address::random();
    let tx = TransactionSignedAndRecovered {
        signer,
        signed_transaction: EthereumTxEnvelope::Eip1559(Signed::new_unchecked(
            TxEip1559::default(),
            Signature::test_signature(),
            Default::default(),
        )),
        block_number: 5u64,
    };

    let reth_tx: Recovered<TransactionSigned> = tx.into();

    assert_eq!(signer, reth_tx.signer());
}

// TODO: Needs more complex tests later
#[test]
fn prepare_call_env_conversion() {
    let from = Address::random();
    let to = Address::random();
    let request = TransactionRequest {
        from: Some(from),
        to: Some(TxKind::Call(to)),
        gas_price: Some(100),
        gas: Some(200),
        value: Some(U256::from(300u64)),
        nonce: Some(1),
        chain_id: Some(1),
        transaction_type: Some(2),
        ..Default::default()
    };

    let block_env = BlockEnv::default();

    let tx_env = prepare_call_env(&block_env, request).unwrap();
    let expected = TxEnv {
        tx_type: TransactionType::Eip1559.into(),
        caller: from,
        gas_price: 100,
        gas_limit: 200,
        kind: TransactTo::Call(to),
        value: U256::from(300u64),
        chain_id: Some(1),
        nonce: 1,
        ..Default::default()
    };

    assert_eq!(tx_env.caller, expected.caller);
    assert_eq!(tx_env.gas_limit, expected.gas_limit);
    assert_eq!(tx_env.gas_price, expected.gas_price);
    assert_eq!(tx_env.gas_priority_fee, expected.gas_priority_fee);
    assert_eq!(tx_env.kind.is_create(), expected.kind.is_create());
    assert_eq!(tx_env.value, expected.value);
    assert_eq!(tx_env.data, expected.data);
    assert_eq!(tx_env.chain_id, expected.chain_id);
    assert_eq!(tx_env.nonce, expected.nonce);
    assert_eq!(tx_env.access_list, expected.access_list);
}

#[test]
fn prepare_call_block_env() {
    let block = Block::default();
    let sealed_block = block.clone().seal();

    let block_env = BlockEnv::from(sealed_block);

    assert_eq!(block_env.number, block.header.number);
    assert_eq!(block_env.beneficiary, block.header.beneficiary);
    assert_eq!(block_env.timestamp, block.header.timestamp);
    assert_eq!(
        block_env.basefee,
        block.header.base_fee_per_gas.unwrap_or_default()
    );
    assert_eq!(block_env.gas_limit, block.header.gas_limit);
    assert_eq!(block_env.prevrandao, Some(block.header.mix_hash));
}
