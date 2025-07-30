//! Example of EIP-712 signatures for Sovereign SDK rollup operations.

use alloy_primitives::{address, B256};
use alloy_sol_types::{eip712_domain, sol, SolStruct};
use anyhow::Result;
use k256::ecdsa::signature::hazmat::{PrehashSigner, PrehashVerifier};
use k256::ecdsa::SigningKey;

sol! {
    #[derive(Debug)]
    struct Coins {
        uint128 amount;
        bytes32 token_id;
    }
    #[derive(Debug)]
    struct Transfer {
        address to;
        Coins coins;
    }
}

fn main() -> Result<()> {
    let signing_key = SigningKey::from_slice(&hex::decode(
        "0000000000000000000000000000000000000000000000000000000000000001",
    )?)?;

    let domain = eip712_domain! {
        name: "CallMessage",
        version: "1",
        chain_id: 4321,
        verifying_contract: address!("0000000000000000000000000000000000000000"),
    };
    let msg = Transfer {
        to: address!("0000000000000000000000000000000000000000"),
        coins: Coins {
            amount: 1000,
            token_id: B256::ZERO,
        },
    };

    let hash = msg.eip712_signing_hash(&domain);
    let signature: k256::ecdsa::Signature = signing_key.sign_prehash(hash.as_slice())?;

    println!("Message: {msg:?}");
    println!("Hash: 0x{}", hex::encode(hash));
    println!("Signature: 0x{}", hex::encode(signature.to_bytes()));

    let verifying_key = signing_key.verifying_key();
    let is_valid = verifying_key
        .verify_prehash(hash.as_slice(), &signature)
        .is_ok();
    println!("Signature valid: {is_valid}");

    Ok(())
}
