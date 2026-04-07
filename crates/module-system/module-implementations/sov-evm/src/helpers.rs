use alloy_consensus::transaction::Recovered;
use alloy_primitives::BlockNumber;
use alloy_primitives::TxKind;
use alloy_primitives::B256;
use alloy_rpc_types::{TransactionInfo, TransactionRequest};
use revm::context::{BlockEnv, TransactionType, TxEnv};
use sov_rpc_eth_types::EthResult;

use crate::evm::primitive_types::TransactionSigned;

/// Builds a [`TxEnv`] for `eth_call`, `eth_createAccessList`, and `eth_estimateGas` simulations.
///
/// # Affordability / balance checks
///
/// The desired behaviour (matching geth, the reference client) is:
///
/// | Scenario | Expected behaviour |
/// |---|---|
/// | Fee fields **omitted** (default) | No affordability check. Fields default to zero, so revm's balance formula `gas_limit * gas_price + value <= balance` evaluates to `0 <= balance` — always true. |
/// | Fee fields **explicitly provided** (`gasPrice > 0` or `maxFeePerGas > 0`) | Enforce `gas_limit * gas_price + value <= balance`. Geth returns `ErrInsufficientFunds` on failure. |
/// | `value > 0` (regardless of fee fields) | All clients verify the caller holds at least `value`. This is a transfer-affordability check, not a fee check. |
///
/// ## How clients implement this
///
/// **Geth** — `CallDefaults()` zeros `gasPrice`/`maxFeePerGas`/`maxPriorityFeePerGas`
/// when omitted. `applyMessage()` sets `NoBaseFee: true` and zeroes `basefee` when
/// `gasPrice == 0`. The `buyGas()` balance check then trivially passes. When the
/// caller explicitly provides `gasPrice > 0`, the value is preserved and the check
/// runs against the caller's balance.
/// - <https://github.com/ethereum/go-ethereum/blob/master/internal/ethapi/transaction_args.go> (`CallDefaults`, `ToMessage`)
/// - <https://github.com/ethereum/go-ethereum/blob/master/internal/ethapi/api.go> (`doCall`, `AccessList`)
///
/// **Reth** — sets `disable_base_fee`, `disable_eip3607`, `disable_fee_charge` but
/// does **not** use revm's `disable_balance_check`. When `gasPrice > 0` it caps the
/// gas limit via `caller_gas_allowance()` instead of failing outright.
/// - <https://github.com/paradigmxyz/reth/blob/main/crates/rpc/rpc-eth-api/src/helpers/call.rs>
///
/// **Revm** — offers `CfgEnv::disable_balance_check` which skips
/// `ensure_enough_balance()` entirely and inflates the caller balance. Neither geth
/// nor reth use this flag; they rely on zeroed fees instead.
/// - <https://github.com/bluealloy/revm/blob/main/crates/handler/src/pre_execution.rs>
///
/// The **Ethereum JSON-RPC spec** (`execution-apis`) is silent on balance checking;
/// all fields in `GenericTransaction` are optional.
/// - <https://github.com/ethereum/execution-apis>
///
/// ## Current divergence from geth
///
/// This function **unconditionally sets `gas_price = 0`**, ignoring any user-supplied
/// `gasPrice` / `maxFeePerGas`. This means a caller who explicitly sets `gasPrice > 0`
/// in an `eth_call` request will **not** see an `InsufficientFunds` error, unlike geth.
/// This is intentional for the rollup metering model (fees are charged at the rollup
/// layer, not through the EVM's gas accounting), but it diverges from geth and may
/// surprise tooling that expects the balance check when fee fields are provided.
///
/// **TODO**: preserve user-supplied fee fields and enforce the balance check to match
/// geth semantics. Track in a follow-up PR.
// `pub(crate)` for test access.
pub(crate) fn prepare_call_env(
    block_env: &BlockEnv,
    request: TransactionRequest,
    tx_gas_limit: Option<u64>,
) -> EthResult<TxEnv> {
    let TransactionRequest {
        from,
        to,
        gas,
        value,
        input,
        nonce,
        access_list,
        chain_id,
        ..
    } = request;

    let gas_limit = gas.unwrap_or_else(|| simulation_gas_limit(block_env.gas_limit, tx_gas_limit));

    let env = TxEnv {
        tx_type: TransactionType::Eip1559.into(),
        gas_limit,
        nonce: nonce.unwrap_or_default(),
        caller: from.unwrap_or_default(),
        // Hardcoded to zero: makes revm's balance check (`gas_limit * gas_price + value`)
        // trivially pass. User-supplied gasPrice / maxFeePerGas is intentionally ignored
        // because sovereign-sdk charges fees via rollup metering, not EVM gas accounting.
        // DIVERGENCE: geth preserves user-supplied non-zero gasPrice and enforces the
        // balance check against it. See the function-level doc comment for details.
        gas_price: 0,
        gas_priority_fee: None,
        kind: to.unwrap_or(TxKind::Create),
        value: value.unwrap_or_default(),
        data: input.try_into_unique_input()?.unwrap_or_default(),
        chain_id,
        access_list: access_list.unwrap_or_default(),
        // Default values
        blob_hashes: vec![],
        max_fee_per_blob_gas: 0,
        authorization_list: vec![],
    };

    Ok(env)
}

fn simulation_gas_limit(block_gas_limit: u64, tx_gas_limit: Option<u64>) -> u64 {
    block_gas_limit.min(tx_gas_limit.unwrap_or(block_gas_limit))
}

/// Builds an RPC transaction from a recovered signed transaction with block context.
pub(crate) fn from_recovered_with_block_context(
    tx: Recovered<TransactionSigned>,
    block_hash: Option<B256>,
    block_number: BlockNumber,
    tx_index: u64,
    base_fee: Option<u64>,
) -> alloy_rpc_types::Transaction {
    let tx_info = TransactionInfo {
        base_fee,
        block_hash,
        block_number: Some(block_number),
        index: Some(tx_index),
        // Default value, because hash is in the tx.
        hash: None,
    };
    alloy_rpc_types::Transaction::from_transaction(tx.convert(), tx_info)
}

#[cfg(test)]
mod tests {
    use alloy_consensus::{EthereumTxEnvelope, Signed, TxEip1559};
    use alloy_primitives::Signature;
    use alloy_primitives::{Address, B256, U256};
    use revm::context::TransactTo;

    use super::*;

    // TODO: Needs more complex tests later
    #[test]
    fn prepare_call_env_conversion() {
        let from = Address::random();
        let to = Address::random();
        let request = TransactionRequest {
            from: Some(from),
            to: Some(TxKind::Call(to)),
            gas_price: Some(100),
            gas: Some(200),
            value: Some(U256::from(300u64)),
            nonce: Some(1),
            chain_id: Some(1),
            transaction_type: Some(2),
            ..Default::default()
        };

        let block_env = BlockEnv::default();

        let tx_env = prepare_call_env(&block_env, request, None).unwrap();
        let expected = TxEnv {
            tx_type: TransactionType::Eip1559.into(),
            caller: from,
            gas_price: 0,
            gas_limit: 200,
            kind: TransactTo::Call(to),
            value: U256::from(300u64),
            chain_id: Some(1),
            nonce: 1,
            ..Default::default()
        };

        assert_eq!(tx_env.caller, expected.caller);
        assert_eq!(tx_env.gas_limit, expected.gas_limit);
        assert_eq!(tx_env.gas_price, expected.gas_price);
        assert_eq!(tx_env.gas_priority_fee, expected.gas_priority_fee);
        assert_eq!(tx_env.kind.is_create(), expected.kind.is_create());
        assert_eq!(tx_env.value, expected.value);
        assert_eq!(tx_env.data, expected.data);
        assert_eq!(tx_env.chain_id, expected.chain_id);
        assert_eq!(tx_env.nonce, expected.nonce);
        assert_eq!(tx_env.access_list, expected.access_list);
    }

    #[test]
    fn prepare_call_env_omitted_gas_uses_tx_gas_limit() {
        let block_env = BlockEnv {
            gas_limit: 1_000_000_000,
            ..Default::default()
        };

        let request = TransactionRequest::default();

        // When tx_gas_limit is provided, omitted gas falls back to it
        let tx_env = prepare_call_env(&block_env, request.clone(), Some(30_000_000)).unwrap();
        assert_eq!(
            tx_env.gas_limit, 30_000_000,
            "should use tx_gas_limit when gas is omitted"
        );

        let lower_block_limit = BlockEnv {
            gas_limit: 500_000,
            ..Default::default()
        };
        let tx_env =
            prepare_call_env(&lower_block_limit, request.clone(), Some(30_000_000)).unwrap();
        assert_eq!(
            tx_env.gas_limit, 500_000,
            "should respect a lower block gas limit when gas is omitted"
        );

        // When tx_gas_limit is None, omitted gas falls back to block_env.gas_limit
        let tx_env = prepare_call_env(&block_env, request, None).unwrap();
        assert_eq!(
            tx_env.gas_limit, 1_000_000_000,
            "should fall back to block gas limit when tx_gas_limit is None"
        );
    }

    #[test]
    fn prepare_call_env_explicit_gas_bypasses_simulation_limit() {
        let block_env = BlockEnv {
            gas_limit: 1_000_000_000,
            ..Default::default()
        };

        let explicit_gas = 50_000;
        let request = TransactionRequest {
            gas: Some(explicit_gas),
            ..Default::default()
        };

        // Explicit gas should be used as-is, ignoring both tx_gas_limit and block_gas_limit
        let tx_env = prepare_call_env(&block_env, request, Some(30_000_000)).unwrap();
        assert_eq!(
            tx_env.gas_limit, explicit_gas,
            "explicit gas in request should bypass simulation_gas_limit"
        );
    }

    #[test]
    fn from_recovered_with_block_context_uses_base_fee_for_effective_gas_price() {
        let tx = TxEip1559 {
            max_fee_per_gas: 100,
            max_priority_fee_per_gas: 2,
            ..Default::default()
        };
        let recovered = Recovered::new_unchecked(
            EthereumTxEnvelope::Eip1559(Signed::new_unchecked(
                tx,
                Signature::test_signature(),
                Default::default(),
            )),
            Address::ZERO,
        );

        let with_base_fee =
            from_recovered_with_block_context(recovered.clone(), Some(B256::ZERO), 1, 0, Some(10));
        // min(max_priority_fee_per_gas, max_fee_per_gas - base_fee) + base_fee
        assert_eq!(with_base_fee.effective_gas_price, Some(12));

        let without_base_fee =
            from_recovered_with_block_context(recovered, Some(B256::ZERO), 1, 0, None);
        // Fallback behavior when base fee is unavailable.
        assert_eq!(without_base_fee.effective_gas_price, Some(100));
    }
}
