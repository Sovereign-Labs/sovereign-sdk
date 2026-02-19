# AGENTS.md - sov-evm

> See repository `AGENTS.md` for global rules. This file adds crate-specific guidance only.

## Mission

`sov-evm` is the source of truth for EVM execution state and EVM JSON-RPC state queries. Most production RPC correctness bugs originate here, especially around block/tag resolution, fee context, and cross-endpoint consistency.

## Ownership Boundary

- Own here: EVM state, block/tx/receipt/log assembly, `eth_call`, `eth_estimateGas`, `eth_feeHistory`, tracing.
- Do not own here: transport concerns, RPC method stubs, WebSocket plumbing, sequencer submission UX wrappers. Those belong to `crates/full-node/sov-ethereum`.

## Shared RPC Semantics (Intentional)

These are by-design repo semantics. Do not flag as bugs unless a concrete tooling workflow breaks.

- `latest` and `pending` resolve to the same pending head view for block/tag resolution.
- Pending/soft-confirmed data may appear in block/tx/receipt/log responses.
- `safe` and `finalized` both resolve to the latest finalized block.
- `eth_getTransactionCount(..., "latest")` may include pending-inclusive nonce effects.
- `eth_getLogs` with `toBlock: "latest"` may include pending-head logs.
- `newHeads` may surface synthetic/pending-head style headers.
- `eth_blockNumber` returns latest sealed block number, not synthetic pending height.
- `eth_gasPrice` returns current `block_env.basefee`.
- `eth_maxPriorityFeePerGas` returns `0`.
- `eth_call` accepts `state_overrides`/`block_overrides` but currently ignores them.
- EIP-1898 `requireCanonical` is accepted but effectively a no-op in no-reorg semantics.

## High-Risk Hotspots

| Path | Why it matters |
| --- | --- |
| `src/rpc/mod.rs` | Block/tag resolution, synthetic block cache, tx/receipt assembly, fee linkage |
| `src/rpc/handlers.rs` | Public JSON-RPC method behavior and parameter types |
| `src/rpc/fee_history.rs` | Fee history shape, validation, and base-fee progression |
| `src/helpers.rs` | Tx response construction and `effectiveGasPrice` derivation paths |
| `src/state_access.rs` | Historical/pending state reads and block-env lookup |
| `src/evm/executor.rs` | EVM cfg/env flags (`disable_base_fee`, call environment) |
| `src/rpc/trace.rs` | Debug trace parity with block/context resolution |

## Critical Invariants

### 1. Fee/BaseFee coherence

If you touch fee context, validate all of these together:

- Pending/sealed block header `base_fee_per_gas`
- Call/trace `BlockEnv.basefee`
- Tx response `effectiveGasPrice`
- Receipt `effective_gas_price`
- `eth_feeHistory` base-fee series

### 2. Cross-endpoint value consistency

For the same tx/block, values must agree across:

- `eth_getTransactionByHash`
- `eth_getTransactionReceipt`
- `eth_getBlockByNumber`/`eth_getBlockByHash` (full tx mode)
- `eth_getBlockReceipts`

### 3. Block selector consistency

- Prefer `BlockId` over ad-hoc string parsing.
- Keep `BlockId::Hash` and `BlockId::Number` paths behaviorally aligned.
- Ensure historical lookup and synthetic lookup errors are deterministic and consistent.

### 4. Encoding correctness

- QUANTITY: `0x` prefixed, no leading zeros.
- DATA: fixed-width where required (for example `eth_getStorageAt` as 32-byte data).
- Nullability must match tooling expectations for each endpoint.

### 5. Determinism and replay safety

- Do not introduce non-deterministic state access in module/core logic.
- Avoid hidden behavior drift between native and proof-relevant paths.

## Failure Pattern Matrix (PR Lessons)

| Pattern | Repeated in PR(s) | Typical root cause | What to verify before merge |
| --- | --- | --- | --- |
| Error-code drift | `#2378`, `#2459` | Invalid params / rejected tx / not-found mapped inconsistently | Invalid-param, revert, and not-found paths return stable expected code families |
| Block-id regression | `#2379`, `#2391` | Using tag strings instead of `BlockId`/EIP-1898 paths | All relevant methods accept and correctly route `BlockId` variants |
| Missing endpoint consistency | `#2381`, `#2391` | New methods added but behavior not aligned with existing fields | New endpoint values match existing tx/receipt/block data contracts |
| Fee history and pending surface drift | `#2387` | Base-fee or pending range assumptions diverge across endpoints | `eth_feeHistory`, block headers, and tx/receipt fee fields stay coherent |
| Nonce/receipt lifecycle mismatch | `#2395`, `#2458` | Pending/soft-confirmed state not reflected consistently | Nonce, tx lookup, and receipt lookup agree during pending-to-sealed transitions |
| Estimation/call behavior mismatch | `#2459` | Revert and gas estimation paths do not mirror real execution constraints | `eth_estimateGas` errors on revert with revert payload; no success value on revert |
| Effective-gas-price mismatch | `#2462` | Fee context dropped while building tx response | `effectiveGasPrice` aligns across tx object and receipt for same tx |
| BASEFEE opcode mismatch | `#2463` | `BlockEnv.basefee` set inconsistently in call/trace contexts | `BASEFEE` opcode result matches block header base fee in same context |

## Change Workflow

1. Identify whether the change is state/query logic (`sov-evm`) or wrapper/transport (`sov-ethereum`).
2. For any RPC field change, enumerate every endpoint that returns the same conceptual value.
3. Update behavior docs if semantics changed: `docs/rpc_inventory.md` and relevant `docs/eth_*_test_cases.md`.
4. Add or update tests before merge for at least one pending and one sealed scenario.

## Pre-Merge Checklist

1. If touching fees/base fee, validate all fee surfaces in one run.
2. If touching tx/receipt fields, cross-check all four endpoint families listed above.
3. If touching block tags/block id, test `earliest/latest/pending/safe/finalized/number/hash` selectors.
4. If touching `eth_call` or `eth_estimateGas`, test success, revert, and halt paths.
5. If touching response encoding, verify QUANTITY vs DATA shape for affected fields.
6. If adding or changing a method contract, confirm `sov-ethereum` stub/registration expectations remain correct.

## Minimal Audit Notes (P0/P1)

When doing static production-readiness sweeps, prioritize:

1. Wallet send flow: nonce, estimation, submission, receipt polling.
2. SDK decoding stability: ethers/viem/web3 parsing of tx/receipt/block/log shapes.
3. Internal coherence: same tx hash gives non-contradictory fields across endpoints.
4. Gas UX safety: no impossible fee combinations that mislead fee selection logic.

## Fast Commands

```bash
SKIP_GUEST_BUILD=1 cargo nextest run -p sov-evm
SKIP_GUEST_BUILD=1 cargo nextest run -p sov-demo-rollup evm_rpc
SKIP_GUEST_BUILD=1 cargo nextest run -p sov-demo-rollup evm_fee_history
SKIP_GUEST_BUILD=1 cargo nextest run -p sov-demo-rollup evm_effective_gas_price
SKIP_GUEST_BUILD=1 cargo nextest run -p sov-demo-rollup evm_basefee
```
