//! Ethereum transfer generator.
use arbitrary::Arbitrary;
use reth_primitives::{Bytes, TransactionSigned, TxKind, U256};
use reth_rpc_types::{transaction::EIP1559TransactionRequest, AccessList, TypedTransactionRequest};
use revm::primitives::Address;
pub use secp256k1::SecretKey;

use crate::randomness::Randomness;
use crate::signer::Signer;

/// Transfer generator.
pub struct TransferGenerator {
    signer: Signer,
    randomness_manager: Randomness,
}

impl TransferGenerator {
    /// Set up a new transfer generator
    pub fn new(key: SecretKey, salt: u128) -> Self {
        TransferGenerator {
            signer: Signer::new(key),
            randomness_manager: Randomness::new(salt),
        }
    }

    /// Generate a transfer transaction.
    pub fn generate(&mut self, nonce: u64) -> TransactionSigned {
        let to = self.randomize_data();
        self.signed_tx(to, nonce)
    }

    fn randomize_data(&mut self) -> Address {
        for _ in 0..20 {
            if self.randomness_manager.has_enough() {
                let offset = self.randomness_manager.offset();
                let u = &mut arbitrary::Unstructured::new(
                    &self.randomness_manager.randomness[offset..],
                );

                if let Ok(to) = Address::arbitrary(u) {
                    self.randomness_manager.update_remaining(u.len());
                    return to;
                } else {
                    self.randomness_manager.increase_buffer_size();
                }
            }
            self.randomness_manager.re_randomize();
        }
        unreachable!("Could not get enough randomness to generate a transaction");
    }

    /// Creates the signed transfer tx
    fn signed_tx(&self, to: Address, nonce: u64) -> TransactionSigned {
        let request = TypedTransactionRequest::EIP1559(EIP1559TransactionRequest {
            chain_id: 4321,
            nonce,
            value: U256::from(1),
            input: Bytes::new(),
            max_priority_fee_per_gas: U256::ZERO,
            max_fee_per_gas: U256::from(1000),
            gas_limit: U256::from(u64::MAX),
            kind: TxKind::Call(to),
            access_list: AccessList::default(),
        });

        self.signer
            .sign_transaction(request)
            .expect("Could not sign")
    }
}
