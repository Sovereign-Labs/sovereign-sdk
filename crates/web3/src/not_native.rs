use serde::{Deserialize, Serialize};
use sov_universal_wallet::schema::Schema;

pub const DEFAULT_MAX_PRIORITY_FEE_BIPS: u64 = 0;
pub const DEFAULT_MAX_FEE: u128 = 100000000;
pub const DEFAULT_GAS_LIMIT: Option<Vec<u64>> = None;

pub fn default_uniqueness() -> UniquenessData {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_secs();
    UniquenessData::Generation(now)
}

pub trait AsRuntimeCallJson {
    fn as_runtime_call_json(&self) -> serde_json::Value;
}

pub trait ChainSchema {
    type Error: std::error::Error;

    fn schema() -> Result<Schema, Self::Error>;
}

pub trait Signer {
    fn sign(&self, message: &[u8]) -> Vec<u8>;
    fn public_key(&self) -> Vec<u8>;
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UniquenessData {
    /// Nonce-based uniqueness: an account's transactions must have a unique and consecutive nonces
    Nonce(u64),
    /// Generation-based uniqueness: the last `PAST_TRANSACTION_GENERATION` generations are cached.
    /// Transactions older than this buffer are invalid, transactions falling within it or with a
    /// higher generation are valid but must have a unique hash within their generation
    Generation(u64),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TxDetails {
    max_priority_fee_bips: u64,
    max_fee: u128,
    gas_limit: Option<Vec<u64>>,
    chain_id: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UnsignedTransaction {
    runtime_call: serde_json::Value,
    uniqueness: UniquenessData, // optional so it can be filled in later
    tx_details: TxDetails,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Transaction {
    runtime_call: serde_json::Value,
    uniqueness: UniquenessData,
    tx_details: TxDetails,
    pub_key: String,
    signature: String,
}

#[derive(Debug)]
pub enum TransactionBuilderError {
    PrivateKeyInvalid,
    ChainHashFailed,
}

pub struct TransactionBuilder {
    call: Box<dyn AsRuntimeCallJson>,
    chain_id: u64,
    uniqueness: Option<UniquenessData>,
    priority_fee_bips: Option<u64>,
    max_fee: Option<u128>,
    gas_limit: Option<Option<Vec<u64>>>,
}

impl TransactionBuilder {
    pub fn new<M: AsRuntimeCallJson + 'static>(call: M, chain_id: u64) -> Self {
        Self {
            call: Box::new(call),
            chain_id,
            uniqueness: None,
            priority_fee_bips: None,
            max_fee: None,
            gas_limit: None,
        }
    }

    pub fn uniqueness(mut self, uniqueness: UniquenessData) -> Self {
        self.uniqueness = Some(uniqueness);
        self
    }

    pub fn gas_limit(mut self, gas_limit: Option<Vec<u64>>) -> Self {
        self.gas_limit = Some(gas_limit);
        self
    }

    pub fn build(self) -> Result<UnsignedTransaction, TransactionBuilderError> {
        let priority_fee = self
            .priority_fee_bips
            .unwrap_or(DEFAULT_MAX_PRIORITY_FEE_BIPS);
        let max_fee = self.max_fee.unwrap_or(DEFAULT_MAX_FEE);
        let gas_limit = self.gas_limit.unwrap_or(None);
        let uniqueness = self.uniqueness.unwrap_or_else(default_uniqueness);

        todo!()
    }

    // todo
    // fn build_and_sign()
}
