#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

use std::collections::HashMap;

use arbitrary::Arbitrary;
use reth_primitives::{
    sign_message, Bytes, Transaction, TransactionSigned, TxEip1559, TxEip2930, TxEip4844, TxKind, TxLegacy, U256
};
use reth_rpc_types::{transaction::EIP1559TransactionRequest, AccessList, TypedTransactionRequest};
use revm::primitives::{Address, B256};
use secp256k1::{rand::{RngCore, SeedableRng}, PublicKey};
pub use secp256k1::{SecretKey};
use sha2::Digest;


/// Ethereum transaction signer.
#[derive(Clone)]
pub struct DevSigner {
    signers: HashMap<Address, SecretKey>,
}

/// Signature error.
#[derive(Debug, thiserror::Error)]
pub enum SignError {
    /// Error occurred while trying to sign data.
    #[error("Could not sign")]
    CouldNotSign,
    /// Signer for a requested account is not found.
    #[error("Unknown account")]
    NoAccount,
    /// TypedData has an invalid format.
    #[error("Given typed data is not valid")]
    TypedData,
    /// Invalid transaction request in `sign_transaction`.
    #[error("invalid transaction request")]
    InvalidTransactionRequest,
    /// No chain id
    #[error("No chain id")]
    NoChainId,
}

impl DevSigner {
    /// Creates a new DevSigner.
    pub fn new(secret_keys: Vec<SecretKey>) -> Self {
        let mut signers = HashMap::with_capacity(secret_keys.len());

        for sk in secret_keys {
            let public_key = PublicKey::from_secret_key(secp256k1::SECP256K1, &sk);
            let address = reth_primitives::public_key_to_address(public_key);

            signers.insert(address, sk);
        }

        Self { signers }
    }

    /// Signs an ethereum transaction.
    pub fn sign_transaction(
        &self,
        request: TypedTransactionRequest,
        address: Address,
    ) -> Result<TransactionSigned, SignError> {
        let transaction =
            to_primitive_transaction(request).ok_or(SignError::InvalidTransactionRequest)?;
        let tx_signature_hash = transaction.signature_hash();
        let signer = self.signers.get(&address).ok_or(SignError::NoAccount)?;

        let signature = sign_message(B256::from_slice(signer.as_ref()), tx_signature_hash)
            .map_err(|_| SignError::CouldNotSign)?;

        Ok(TransactionSigned::from_transaction_and_signature(
            transaction,
            signature,
        ))
    }

    /// List of signers.
    pub fn signers(&self) -> Vec<Address> {
        self.signers.keys().copied().collect()
    }


}

/// Transfer generator.
pub struct TransferGenerator {
    key: SecretKey,
    randomness: Vec<u8>,
    remaining_randomness: usize,
    target_buffer_size: usize,
    salt: u128,
}


/// Get a Vec of `num` bytes, seeded by `num` and  a salt value
pub fn get_random_bytes(num: usize, salt: u128) -> Vec<u8> {
    let mut output = vec![0; num];
    randomize_buffer(&mut output, salt);
    output
}

/// Randomize the given buffer. The rng is seeded from the buffer's length and the salt
pub fn randomize_buffer(buffer: &mut [u8], salt: u128) {
    // First, use the hash of a sha256 string to get a high quality rng. (Seeding yourself is hard because you need a high hamming weight!)
    let input = format!("{}|{}", buffer.len(), salt);
    let salt_hashed = sha2::Sha256::digest(input);
    let mut rng = rand_chacha::ChaChaRng::from_seed(salt_hashed.into());
    rng.fill_bytes(buffer);
}

/// Setup generation with the given params
pub fn setup_harness(rng_salt: u128, key: SecretKey) -> TransferGenerator {
    let random_bytes: Vec<u8> = get_random_bytes(100_000, rng_salt);
    let u = &mut arbitrary::Unstructured::new(&random_bytes[..]);
    let remaining_randomness = u.len();
    TransferGenerator {
        randomness: random_bytes,
        remaining_randomness,
        key,
        target_buffer_size: 100_000,
        salt: rng_salt,
    }
}

impl TransferGenerator {
    /// Set up a new transfer generator
    pub fn new(key: SecretKey, salt: u128) -> Self {
        setup_harness(salt, key)
    }


    /// Generate a transfer transaction.
    pub fn generate(
        &mut self,
        nonce: u64,
    ) -> TransactionSigned {
        for _ in 0..20 {
            if self.has_enough_randomness() {
                let u =
                    &mut arbitrary::Unstructured::new(&self.randomness[self.randomness_offset()..]);

                if let Ok(output) = self.generate_min_transfer(nonce, u) {
                    self.remaining_randomness = u.len();
                    return output;
                } else {
                    self.target_buffer_size *= 2;
                }
            }
            self.re_randomize();
        }
        unreachable!("Could not get enough randomness to generate a transaction");
    }

    fn re_randomize(&mut self) {
        if self.randomness.len() < self.target_buffer_size {
            self.randomness = vec![0; self.target_buffer_size];
        }
        randomize_buffer(&mut self.randomness[..], self.salt);
        self.remaining_randomness = self.randomness.len();
        self.salt += 1;
    }

    fn randomness_offset(&self) -> usize {
        self.randomness.len() - self.remaining_randomness
    }

    fn has_enough_randomness(&self) -> bool {
        self.remaining_randomness > std::cmp::min(1000, self.target_buffer_size / 10)
    }

    /// Generate a transfer transaction.
    fn generate_min_transfer(&self, nonce: u64, u: &mut arbitrary::Unstructured<'_>) -> Result<TransactionSigned, arbitrary::Error> {

        let to: [u8;20] = Arbitrary::arbitrary(u)?;
        let value = 1;
        let request = TypedTransactionRequest::EIP1559(EIP1559TransactionRequest {
            chain_id: 4321,
            nonce,
            value: U256::from_limbs([value, 0, 0, 0]),
            input: Bytes::new(),
            max_priority_fee_per_gas: U256::ZERO,
            max_fee_per_gas: U256::from_limbs([1000, 0, 0, 0]),
            gas_limit: U256::from_limbs([u64::MAX, 0, 0, 0]),
            kind: TxKind::Call(Address::from_slice(&to)),
            access_list: AccessList::default(),
        });

        let transaction =
            to_primitive_transaction(request).expect("Invalid transaction request");
        let tx_signature_hash = transaction.signature_hash();
        let signer = self.key;

        let signature = sign_message(B256::from_slice(signer.as_ref()), tx_signature_hash)
            .expect("Could not sign");

        Ok(TransactionSigned::from_transaction_and_signature(
            transaction,
            signature,
        ))

    }
}

/// Converts a typed transaction request into a primitive transaction.
///
/// Returns `None` if any of the following are true:
/// - `nonce` is greater than [`u64::MAX`]
/// - `gas_limit` is greater than [`u64::MAX`]
/// - `value` is greater than [`u128::MAX`]
///   Copy from [`reth_rpc_types_compat::transaction::to_primitive_transaction`]
fn to_primitive_transaction(tx_request: TypedTransactionRequest) -> Option<Transaction> {
    Some(match tx_request {
        TypedTransactionRequest::Legacy(tx) => Transaction::Legacy(TxLegacy {
            chain_id: tx.chain_id,
            nonce: tx.nonce,
            gas_price: tx.gas_price.to(),
            gas_limit: tx.gas_limit.try_into().ok()?,
            to: tx.kind,
            value: tx.value,
            input: tx.input,
        }),
        TypedTransactionRequest::EIP2930(tx) => Transaction::Eip2930(TxEip2930 {
            chain_id: tx.chain_id,
            nonce: tx.nonce,
            gas_price: tx.gas_price.to(),
            gas_limit: tx.gas_limit.try_into().ok()?,
            to: tx.kind,
            value: tx.value,
            input: tx.input,
            access_list: tx.access_list,
        }),
        TypedTransactionRequest::EIP1559(tx) => Transaction::Eip1559(TxEip1559 {
            chain_id: tx.chain_id,
            nonce: tx.nonce,
            max_fee_per_gas: tx.max_fee_per_gas.to(),
            gas_limit: tx.gas_limit.try_into().ok()?,
            to: tx.kind,
            value: tx.value,
            input: tx.input,
            access_list: tx.access_list,
            max_priority_fee_per_gas: tx.max_priority_fee_per_gas.to(),
        }),
        TypedTransactionRequest::EIP4844(tx) => Transaction::Eip4844(TxEip4844 {
            chain_id: tx.chain_id,
            nonce: tx.nonce,
            gas_limit: tx.gas_limit.to(),
            max_fee_per_gas: tx.max_fee_per_gas.to(),
            max_priority_fee_per_gas: tx.max_priority_fee_per_gas.to(),
            placeholder: None,
            to: tx.to,
            value: tx.value,
            access_list: tx.access_list,
            blob_versioned_hashes: tx.blob_versioned_hashes,
            max_fee_per_blob_gas: tx.max_fee_per_blob_gas.to(),
            input: tx.input,
        }),
    })
}
