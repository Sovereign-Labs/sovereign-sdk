use std::str::FromStr;

use alloy_consensus::{SignableTransaction, TxEip1559, TxEnvelope};
use alloy_primitives::{hex, Address, Bytes, TxKind, U256};
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use sov_modules_api::macros::config_value;

#[tokio::test(flavor = "multi_thread")]
async fn tx_rlp_encoding_test() -> Result<(), Box<dyn std::error::Error>> {
    let signer: PrivateKeySigner =
        "dcf2cbdd171a21c480aa7f53d77f31bb102282b3ff099c78e3118b37348c72f7".parse()?;
    let from_addr = signer.address();
    let to_addr = Address::from_str("0x0aa7420c43b8c1a7b165d216948870c8ecfe1ee1")?;
    let data: Bytes =
        hex!("6ecd23060000000000000000000000000000000000000000000000000000000000000002").into();

    let tx = TxEip1559 {
        chain_id: config_value!("CHAIN_ID"),
        nonce: 0,
        gas_limit: 184156,
        max_fee_per_gas: 768658734568,
        max_priority_fee_per_gas: 413047990155,
        to: TxKind::Call(to_addr),
        value: U256::from(2000000000000u64),
        input: data,
        access_list: Default::default(),
    };

    // Sign the transaction
    let sig = signer.sign_hash_sync(&tx.signature_hash())?;

    // Verify signature
    assert_eq!(
        sig.recover_address_from_prehash(&tx.signature_hash())?,
        from_addr
    );

    // Encode as signed transaction envelope and decode
    let signed_tx = tx.into_signed(sig);
    let tx_envelope = TxEnvelope::Eip1559(signed_tx);
    let encoded = alloy_rlp::encode(&tx_envelope);

    let decoded: TxEnvelope = alloy_rlp::decode_exact(&encoded)?;

    // Verify decoded transaction matches
    assert_eq!(tx_envelope, decoded);
    Ok(())
}
