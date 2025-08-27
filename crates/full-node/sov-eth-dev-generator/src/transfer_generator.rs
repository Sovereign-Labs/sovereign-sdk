//! Ethereum transfer generator.
use alloy_consensus::TxEip1559;
use alloy_consensus::TypedTransaction;
use alloy_primitives::{Address, Bytes, TxKind, U256};
use arbitrary::Arbitrary;
use reth_primitives::TransactionSigned;
use revm::context::transaction::AccessList;
use secp256k1::SecretKey;

use crate::randomness::Randomness;
use sov_eth_dev_signer::Signer;

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
        let request = TypedTransaction::Eip1559(TxEip1559 {
            chain_id: 4321,
            nonce,
            value: U256::from(1),
            input: Bytes::new(),
            max_priority_fee_per_gas: 0,
            max_fee_per_gas: 0,
            gas_limit: 0,
            to: TxKind::Call(to),
            access_list: AccessList::default(),
        });

        self.signer
            .sign_transaction(request)
            .expect("Could not sign")
    }
}
