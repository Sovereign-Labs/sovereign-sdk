use std::collections::HashMap;

use sov_bank::Amount;
use sov_hyperlane_integration::igp::{DomainDefaultGas, DomainOracleData, ExchangeRateAndGasPrice};
use sov_modules_api::SafeVec;

pub mod post_mailbox_send;
pub mod set_relayer_config;
pub mod update_oracle_data;

// Specific implementations for your domain types
pub(crate) fn default_gas_hashmap_to_safe_vec<const MAX_SIZE: usize>(
    map: HashMap<u32, Amount>,
) -> SafeVec<DomainDefaultGas, MAX_SIZE> {
    let vec: Vec<DomainDefaultGas> = map
        .into_iter()
        .map(|(domain, default_gas)| DomainDefaultGas {
            domain,
            default_gas,
        })
        .collect();

    vec.try_into()
        .expect("default_gas hashmap to safevec succeeds")
}

pub(crate) fn oracle_data_hashmap_to_safe_vec<const MAX_SIZE: usize>(
    map: HashMap<u32, ExchangeRateAndGasPrice>,
) -> SafeVec<DomainOracleData, MAX_SIZE> {
    let vec: Vec<DomainOracleData> = map
        .into_iter()
        .map(|(domain, data_value)| DomainOracleData { domain, data_value })
        .collect();

    vec.try_into()
        .expect("default_gas hashmap to safevec succeeds")
}

pub(crate) struct IGPMetadata {
    pub destination_gas_limit: Amount,
}

impl IGPMetadata {
    pub(crate) fn serialize(&self) -> SafeVec<u8, 8192> {
        const GAS_LIMIT_OFFSET: usize = 34;

        let mut buf = vec![0_u8; 86];

        // Set gas limit (position 34-66)
        let gas_limit_bytes = self.destination_gas_limit.0.to_be_bytes();
        buf[GAS_LIMIT_OFFSET + 16..GAS_LIMIT_OFFSET + 32].copy_from_slice(&gas_limit_bytes);

        SafeVec::try_from(buf).expect("succeeds")
    }
}
