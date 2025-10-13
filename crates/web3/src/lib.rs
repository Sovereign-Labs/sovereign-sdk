use sov_modules_api::{
    capabilities::{config_chain_id, UniquenessData},
    transaction::{PriorityFeeBips, Transaction, UnsignedTransaction},
    Amount, CallMessage, CryptoSpec, RuntimeDiscriminant, Spec, UnmanagedRuntimeCall,
};

#[derive(Debug)]
pub enum TransactionBuilderError {
    PrivateKeyInvalid,
    ChainHashFailed,
}

pub trait ChainHash {
    type Error: std::error::Error;

    fn chain_hash() -> Result<[u8; 32], Self::Error>;
}

pub const DEFAULT_MAX_PRIORITY_FEE_BIPS: PriorityFeeBips = PriorityFeeBips::ZERO;
pub const DEFAULT_MAX_FEE: Amount = Amount(100000000);
pub const DEFAULT_GAS_LIMIT: Option<Vec<u64>> = None;

pub fn default_uniqueness() -> UniquenessData {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_secs();
    UniquenessData::Generation(now)
}

pub struct TransactionBuilder<S: Spec, M: CallMessage + RuntimeDiscriminant> {
    call: M,
    uniqueness: Option<UniquenessData>,
    priority_fee_bips: Option<PriorityFeeBips>,
    max_fee: Option<Amount>,
    gas_limit: Option<Option<S::Gas>>,
}

impl<S: Spec, M: CallMessage + RuntimeDiscriminant> TransactionBuilder<S, M> {
    pub fn for_call(call: M) -> Self {
        Self {
            call,
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

    pub fn gas_limit(mut self, gas_limit: Option<S::Gas>) -> Self {
        self.gas_limit = Some(gas_limit);
        self
    }

    pub fn build(
        self,
    ) -> Result<UnsignedTransaction<UnmanagedRuntimeCall<M>, S>, TransactionBuilderError> {
        let priority_fee = self
            .priority_fee_bips
            .unwrap_or(DEFAULT_MAX_PRIORITY_FEE_BIPS);
        let max_fee = self.max_fee.unwrap_or(DEFAULT_MAX_FEE);
        let gas_limit = self.gas_limit.unwrap_or(None);
        let uniqueness = self.uniqueness.unwrap_or_else(default_uniqueness);

        Ok(UnsignedTransaction::new(
            self.call,
            config_chain_id(),
            priority_fee,
            max_fee,
            uniqueness,
            gas_limit,
        ))
    }

    pub fn build_and_sign<C: ChainHash>(
        self,
        private_key: &<S::CryptoSpec as CryptoSpec>::PrivateKey,
    ) -> Result<Transaction<UnmanagedRuntimeCall<M>, S>, TransactionBuilderError> {
        let chain_hash = C::chain_hash().map_err(|_| TransactionBuilderError::ChainHashFailed)?;
        Ok(self.build()?.sign(private_key, &chain_hash))
    }
}
