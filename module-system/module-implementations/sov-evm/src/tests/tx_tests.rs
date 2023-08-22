use std::str::FromStr;

use ethers_core::abi::Address;
use ethers_core::types::transaction::eip2718::TypedTransaction;
use ethers_core::types::{Bytes, Eip1559TransactionRequest};
use ethers_core::utils::rlp::Rlp;
use ethers_signers::{LocalWallet, Signer};

#[tokio::test]
async fn tx_rlp_encoding_test() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = "dcf2cbdd171a21c480aa7f53d77f31bb102282b3ff099c78e3118b37348c72f7"
        .parse::<LocalWallet>()?;
    let from_addr = wallet.address();
    let to_addr = Address::from_str("0x0aa7420c43b8c1a7b165d216948870c8ecfe1ee1")?;
    let data: Bytes = Bytes::from_str(
        "0x6ecd23060000000000000000000000000000000000000000000000000000000000000002",
    )?;

    let tx_request = Eip1559TransactionRequest::new()
        .from(from_addr)
        .chain_id(1u64)
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
