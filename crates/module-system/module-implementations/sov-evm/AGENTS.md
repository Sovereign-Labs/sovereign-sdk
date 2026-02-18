# AGENTS.md - sov-evm

> See also: root `<repo>/AGENTS.md` for build commands, test runner, and general conventions.

## Crate Purpose

EVM execution module for Sovereign rollups. Stores EVM state, executes transactions, and serves JSON-RPC queries. The core type is `Evm<S>` which is generic over `Spec` (the Sovereign module specification trait). This crate owns all EVM state and is the authoritative source for block/tx/receipt data and gas estimation.

## Key Files

| File | Why it matters |
|------|----------------|
| `src/rpc/mod.rs` | **Hotspot**: block resolution, response assembly, synthetic blocks, fee computation. Changed in 8/10 audit PRs. |
| `src/rpc/handlers.rs` | Where new JSON-RPC methods are added (via `#[rpc_method]`). |
| `src/rpc/error.rs` | Error code mapping — pitfall zone (see PR #2378). |
| `src/helpers.rs` | Response builders (`prepare_call_env`, `from_recovered_with_block_context`) — pitfall zone for base fee bugs. |
| `src/hooks.rs` | Begin/end block hooks: block sealing, base fee update. |
| `src/evm/primitive_types.rs` | Core types including synthetic block hash encoding. |
| `src/state_access.rs` | State reads, `block_env` construction — source of truth for BASEFEE. |
| `src/evm/executor.rs` | REVM execution wrappers, `get_cfg_env`, gas rebate. |

## Sovereign EVM Semantics (DO NOT flag as bugs)

These behaviors are intentional and differ from standard Ethereum:

- `latest` and `pending` map to the same pending head block.
- Pending/soft-confirmed data may appear in block/tx/receipt/log responses with non-null block fields.
- `eth_blockNumber` tracks the pending head.
- `safe` and `finalized` resolve to the same finalized block (no distinct safe semantics).
- `eth_gasPrice` returns 0 (fees handled by rollup gas meter, not EVM).
- `eth_maxPriorityFeePerGas` returns 0.
- `eth_call`/`eth_estimateGas` run with `gas_price=0` and `disable_base_fee=true` in CfgEnv.
- `state_overrides` and `block_overrides` are accepted but ignored.
- Only EIP-1559 transactions are supported; non-1559 tx types are rejected at submission.
- Gas metering is "Rollup" mode by default: actual fees are charged by the Sovereign gas meter, not by EVM's built-in fee mechanism.
- Synthetic blocks provide instant-finality UX: each tx gets its own synthetic block before DA confirmation.

## Critical Invariants (MUST be maintained)

### Base fee consistency

The base fee must flow correctly through ALL surfaces:
- `block_env.basefee` (for the BASEFEE opcode, EIP-3198)
- `TransactionInfo.base_fee` (for tx response `effectiveGasPrice`)
- Receipt `effective_gas_price`
- `eth_feeHistory` response

Past bugs: PRs #2462, #2463, #2387, #2458.

### Cross-endpoint consistency

The same value (e.g., `effectiveGasPrice`, `gasUsed`, `blockHash`) must be identical across:
- `eth_getTransactionByHash`
- `eth_getTransactionReceipt`
- `eth_getBlockByNumber` (full txs mode)
- `eth_getBlockReceipts`

Past bug: PR #2462.

### Error codes

Must use standard JSON-RPC / EIP-1474 error codes. Never invent codes. See `sov-ethereum/src/lib.rs` for the canonical constants. Past bug: PR #2378.

### Response encoding

Quantity fields use `0x`-prefix with no leading zeros. DATA fields are fixed-width, zero-padded. Example: `eth_getStorageAt` returns 32-byte `B256` DATA, not stripped `U256`. Past bug: PR #2459.

### Activation heights

Behavior changes are gated behind configurable block heights in `<repo>/constants.toml` (e.g., `EVM_RECEIPT_ACTUAL_FEE_HEIGHT`). Historical data must remain consistent; new behavior activates only at/after the configured height.

### Gas estimation must error on revert

`eth_estimateGas` must return an RPC error (with revert data) when the call reverts, not a gas value. Past bug: PR #2459.

## Common Pitfalls (learned from audit)

| Don't | Do instead |
|-------|------------|
| Pass `None`/`0` for base_fee in response builders | Always propagate `block_env.basefee` from `get_block_env()` |
| Set `block_env.basefee = 0` to "disable" fees | Use `CfgEnv::disable_base_fee` flag — keeps BASEFEE opcode (EIP-3198) correct |
| Use `saturating_sub` in fee calculations | Use checked arithmetic; underflow means a bug, not a value to clamp |
| Compute `effectiveGasPrice` with EIP-1559 formula | Use `receipt_fees` accessory state for actual values (Sovereign gas meter charges differently) |
| Accept block identifiers as `String` | Use `BlockId` from alloy, which handles EIP-1898 `{blockHash:..}` objects |
| Add a new RPC method only in this crate | Also add a corresponding stub in `sov-ethereum/src/lib.rs` |

## Testing

```bash
# Integration tests (no rollup needed, fast)
cargo nextest run -p sov-evm <test_name>

# E2E tests (requires full rollup, slower)
cargo nextest run -p sov-demo-rollup <test_name>

# List all tests
cargo nextest run -p sov-evm --list
```

| Location | Purpose |
|----------|---------|
| `tests/integration/` | Module-level tests without full rollup |
| `<repo>/examples/demo-rollup/tests/evm/` | E2E tests — each file covers one RPC concern |
| `<repo>/docs/eth_*_test_cases.md` | Spec-first test case documents |
| `<repo>/crates/utils/sov-evm-test-utils/` | Test helpers and contracts |

**Patterns:**
- Write test case doc first (`<repo>/docs/eth_*_test_cases.md`), then implement.
- Mark unfixed bugs as `#[ignore]` with an explanation comment.
- Use sequencer pause/resume for determinism. Avoid wall-clock timing assertions.
- Always verify the same value across all endpoints that return it (cross-endpoint assertions).

## Configuration

See `<repo>/constants.toml` for all config values. EVM-specific keys: `EVM_BLOCK_PRUNING_THRESHOLD`, `EVM_GAS_METERING_MODE`, `EVM_MAX_FEE_CHECK_HEIGHT`, `EVM_RECEIPT_ACTUAL_FEE_HEIGHT`, `INITIAL_BASE_FEE_PER_GAS`. Note: `INITIAL_BASE_FEE_PER_GAS` is `[10, 10]` — 2-dimensional (compute, storage).

## Reference Docs

- `<repo>/docs/rpc_inventory.md` — Complete RPC method inventory with status
- `<repo>/docs/rpc_priority_sorted.md` — Prioritized testing plan
- `<repo>/docs/ethereum-rpc-compliance-findings.md` — Compliance audit findings
- [Ethereum Execution APIs](https://ethereum.github.io/execution-apis/) — Upstream spec
