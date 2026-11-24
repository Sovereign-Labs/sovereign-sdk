use alloy_consensus::{TxEip1559, TypedTransaction};
use alloy_eips::eip1559::MIN_PROTOCOL_BASE_FEE;
use alloy_primitives::{TxKind, U256};
use sov_evm::{RlpEvmTransaction, TransactionSigned};
use sov_modules_api::capabilities::{take_auth_cache_stats, AuthCacheStats};
use sov_modules_api::macros::config_value;

use crate::helpers::EvmAccount;

/// A signed EIP-1559 transfer, as raw authenticator input and as the decoded transaction.
fn signed_transfer(from: &EvmAccount) -> (Vec<u8>, TransactionSigned) {
    let tx = TxEip1559 {
        to: TxKind::Call(EvmAccount::generate().address()),
        value: U256::from(0x1234u64),
        gas_limit: 1_000_000,
        max_fee_per_gas: MIN_PROTOCOL_BASE_FEE as u128 * 2,
        chain_id: config_value!("CHAIN_ID"),
        ..Default::default()
    };
    let (rlp, signed_tx): (RlpEvmTransaction, TransactionSigned) =
        from.sign(TypedTransaction::Eip1559(tx));
    (borsh::to_vec(&rlp).unwrap(), signed_tx)
}

#[test]
fn prewarm_signature_cache_caches_signer_recovery() {
    let (raw_tx, signed_tx) = signed_transfer(&EvmAccount::generate());
    sov_evm::prewarm_signature_cache(&raw_tx);
    take_auth_cache_stats();

    sov_evm::recover_signer_cached(&signed_tx).unwrap();

    assert_eq!(
        take_auth_cache_stats(),
        AuthCacheStats {
            signature_hits: 1,
            ..Default::default()
        },
        "The prewarm should have cached the signer recovery"
    );
}

#[test]
fn recover_signer_cached_returns_the_signer() {
    let account = EvmAccount::generate();
    let (raw_tx, signed_tx) = signed_transfer(&account);
    sov_evm::prewarm_signature_cache(&raw_tx);

    assert_eq!(
        sov_evm::recover_signer_cached(&signed_tx).unwrap(),
        account.address()
    );
}
