// Smoke tests for the integrated demo-rollup EVM wiring. The full EVM test suite
// lives in `crates/full-node/sov-ethereum/tests/integration/`. The two tests kept
// here exercise the realistic, full-runtime path (with bank, paymaster, hyperlane,
// etc.) to guard against integration regressions that the sov-ethereum-only test
// runtime cannot catch.

mod evm_paymaster_balance_check;
mod evm_rpc;
pub(crate) mod evm_test_helper;
