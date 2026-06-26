use alloy_primitives::{Bytes, B256};
use alloy_sol_types::{sol, SolCall};

sol!(
    #[sol(
        rpc,
        all_derives = true,
        bytecode = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/contracts/artifacts/", "EcrecoverBatch.bin")))]
    EcrecoverBatchContract,
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/contracts/artifacts/",
        "EcrecoverBatch.abi"
    )
);

/// Wrapper for the `EcrecoverBatch` benchmark contract (offline encoding).
///
/// The contract loops over arrays of `(hash, v, r, s)` and calls the `ecrecover`
/// precompile (`0x01`) for each — used to measure signature-verification cost in
/// the rollup EVM.
pub struct EcrecoverBatch {
    bytecode: Bytes,
}

impl Default for EcrecoverBatch {
    fn default() -> Self {
        // Use the bytecode already embedded via the sol! macro.
        Self {
            bytecode: Bytes::from(EcrecoverBatchContract::BYTECODE.to_vec()),
        }
    }
}

impl EcrecoverBatch {
    /// Deploy bytecode.
    pub fn byte_code(&self) -> Bytes {
        self.bytecode.clone()
    }

    /// Encode a `recoverBatch(h, v, r, s)` call. The four arrays must have equal
    /// length `n`; the contract recovers `n` signatures.
    pub fn recover_batch(&self, h: Vec<B256>, v: Vec<u8>, r: Vec<B256>, s: Vec<B256>) -> Bytes {
        let call = EcrecoverBatchContract::recoverBatchCall { h, v, r, s };
        Bytes::from(call.abi_encode())
    }
}
