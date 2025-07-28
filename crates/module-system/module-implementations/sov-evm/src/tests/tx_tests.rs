use std::str::FromStr;

use alloy_primitives::TxKind;
use ethers_core::types::transaction::eip2718::TypedTransaction;
use ethers_core::types::{Bytes, Eip1559TransactionRequest};
use ethers_core::utils::rlp::Rlp;
use ethers_signers::{LocalWallet, Signer};
use reth_primitives::revm_primitives::{BlockEnv, TransactTo, TxEnv};
use reth_primitives::{Address, TransactionSignedEcRecovered, U256};
use reth_rpc_types::request::TransactionInput;
use reth_rpc_types::TransactionRequest;
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
        signed_transaction: reth_primitives::TransactionSigned::default(),
        block_number: 5u64,
    };

    let reth_tx: TransactionSignedEcRecovered = tx.into();

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
        max_fee_per_gas: None,
        max_priority_fee_per_gas: None,
        gas: Some(200),
        value: Some(U256::from(300u64)),
        input: TransactionInput::default(),
        nonce: Some(1),
        chain_id: Some(1),
        access_list: None,
        transaction_type: Some(2),
        blob_versioned_hashes: Default::default(),
        max_fee_per_blob_gas: None,
        ..Default::default()
    };

    let block_env = BlockEnv::default();

    let tx_env = prepare_call_env(&block_env, request).unwrap();
    let expected = TxEnv {
        caller: from,
        gas_price: U256::from(100u64),
        gas_limit: 200u64,
        gas_priority_fee: None,
        transact_to: TransactTo::Call(to),
        value: U256::from(300u64),
        data: Default::default(),
        chain_id: Some(1u64),
        nonce: Some(1u64),
        access_list: vec![],
        blob_hashes: vec![],
        max_fee_per_blob_gas: None,
        authorization_list: None,
    };

    assert_eq!(tx_env.caller, expected.caller);
    assert_eq!(tx_env.gas_limit, expected.gas_limit);
    assert_eq!(tx_env.gas_price, expected.gas_price);
    assert_eq!(tx_env.gas_priority_fee, expected.gas_priority_fee);
    assert_eq!(
        tx_env.transact_to.is_create(),
        expected.transact_to.is_create()
    );
    assert_eq!(tx_env.value, expected.value);
    assert_eq!(tx_env.data, expected.data);
    assert_eq!(tx_env.chain_id, expected.chain_id);
    assert_eq!(tx_env.nonce, expected.nonce);
    assert_eq!(tx_env.access_list, expected.access_list);
}

#[test]
fn prepare_call_block_env() {
    let block = Block {
        header: Default::default(),
        transactions: Default::default(),
    };

    let sealed_block = block.clone().seal();

    let block_env = BlockEnv::from(sealed_block);

    assert_eq!(block_env.number.to::<u64>(), block.header.number);
    assert_eq!(block_env.coinbase, block.header.beneficiary);
    assert_eq!(block_env.timestamp.to::<u64>(), block.header.timestamp);
    assert_eq!(
        block_env.basefee.to::<u64>(),
        block.header.base_fee_per_gas.unwrap_or_default()
    );
    assert_eq!(block_env.gas_limit.to::<u64>(), block.header.gas_limit);
    assert_eq!(block_env.prevrandao, Some(block.header.mix_hash));
}
