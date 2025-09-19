#[cfg(test)]
mod tests {
    use crate::codec::BcsCodec;
    use crate::codec::{StateItemDecoder, StateItemEncoder};
    use alloy_primitives::{B256, U256};
    use revm::state::AccountInfo;
    use serde::{Deserialize, Serialize};

    #[derive(Deserialize, Serialize, Debug, PartialEq, Clone, Default)]
    pub struct DbAccount(pub AccountInfo);

    fn time_decode<T>(codec: &BcsCodec, encoded: &[u8], type_name: &str) -> T 
    where 
        T: for<'a> serde::Deserialize<'a>
    {
        let start = std::time::Instant::now();
        let result = codec.try_decode(encoded).unwrap();
        let duration = start.elapsed();
        
        println!("BCS DECODE: type={}, size={}B, duration={:?}", 
                 type_name, 
                 encoded.len(), 
                 duration);
        
        result
    }

    #[test]
    fn test_bcs_decode_timing_u64() {
        let codec = BcsCodec;
        let value: u64 = 42;
        
        let encoded = codec.encode(&value);
        let decoded: u64 = time_decode(&codec, &encoded, "u64");
        
        assert_eq!(decoded, value);
    }

    #[test]
    fn test_bcs_decode_timing_u256() {
        let codec = BcsCodec;
        let value = U256::from(12345678901234567890u64);
        
        let encoded = codec.encode(&value);
        let decoded: U256 = time_decode(&codec, &encoded, "U256");
        
        assert_eq!(decoded, value);
    }

    #[test]
    fn test_bcs_decode_timing_eoa() {
        let codec = BcsCodec;
        
        // Create EOA (Externally Owned Account) - no code
        let eoa_account = DbAccount(AccountInfo {
            balance: U256::ZERO,
            nonce: 42,
            code_hash: revm::primitives::KECCAK_EMPTY,
            code: None,
        });
        
        let encoded = codec.encode(&eoa_account);
        let decoded: DbAccount = time_decode(&codec, &encoded, "EOA DbAccount");
        
        assert_eq!(decoded, eoa_account);
    }

    #[test]
    fn test_bcs_decode_timing_contract() {
        let codec = BcsCodec;
        
        // Create Contract Account - has code hash
        let contract_account = DbAccount(AccountInfo {
            balance: U256::ZERO,
            nonce: 1,
            code_hash: B256::from_slice(&[0x12; 32]),
            code: None,
        });
        
        let encoded = codec.encode(&contract_account);
        let decoded: DbAccount = time_decode(&codec, &encoded, "Contract DbAccount");
        
        assert_eq!(decoded, contract_account);
    }
}