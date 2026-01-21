# eth_feeHistory test cases

## Summary
- Returns historical gas information for a range of blocks, enabling fee estimation for EIP-1559 transactions.
- Response includes base fees, gas used ratios, and optionally reward percentiles.
- Introduced by EIP-1559; critical for wallet fee estimation UX.

## Parameters
- `blockCount`: Number of blocks to return (1-1024). Values > 1024 are capped.
- `newestBlock`: Block tag or hex number identifying the end of the range.
  - Supported tags: `latest`, `pending`, `finalized`, `safe`, `earliest`, or hex block number.
- `rewardPercentiles`: Optional array of floats (0-100), must be monotonically increasing.
  - If provided, response includes `reward` array with priority fee percentiles.
  - If omitted or empty, `reward` field is omitted from response.

## Response schema
| Field | Type | Length | Notes |
|-------|------|--------|-------|
| `oldestBlock` | uint | - | First block number in returned range |
| `baseFeePerGas` | uint[] | blockCount + 1 | Includes predicted next block fee |
| `gasUsedRatio` | float[] | blockCount | Each value in [0.0, 1.0] |
| `reward` | uint[][] | blockCount × len(percentiles) | Only if percentiles provided |
| `baseFeePerBlobGas` | uint[] | blockCount + 1 | EIP-4844; empty in this rollup |
| `blobGasUsedRatio` | float[] | blockCount | EIP-4844; empty in this rollup |

## Rollup-specific semantics (assumptions)
- `latest` and `pending` resolve to the same pending block (differs from Ethereum spec where `latest` = most recent sealed block).
- `finalized` and `safe` both resolve to the latest sealed block.
- `reward` always returns zeros (preferred sequencer model, no priority fee auction).
- Blob gas fields (`baseFeePerBlobGas`, `blobGasUsedRatio`) return empty arrays (EIP-4844 not implemented).
- Percentile validation allows `<=` (monotonically non-decreasing), not strictly `<`.

## Test cases

Priority legend: P0 = must-have correctness, P1 = high value, P2 = medium value, P3 = optional.

### Block tag semantics

| ID | Priority | Description |
|----|----------|-------------|
| TC01 | P0 | `latest` and `pending` return identical results (same oldest_block, same array lengths). |
| TC02 | P0 | `finalized` returns valid data for sealed blocks only. |
| TC03 | P0 | `safe` returns valid data (same behavior as `finalized` in this rollup). |
| TC04 | P1 | `earliest` with blockCount=2 returns oldest_block=0 (graceful underflow handling). |
| TC05 | P1 | Specific block number (e.g., `Number(7)`) returns correct oldest_block = 7 - blockCount + 1. |
| TC06 | P2 | `finalized` and `safe` return same results (both map to latest sealed). |

### Parameter validation

| ID | Priority | Description |
|----|----------|-------------|
| TC07 | P0 | `blockCount = 0` returns an error. |
| TC08 | P1 | `blockCount > 1024` (e.g., 2000) is silently capped to 1024. |
| TC09 | P2 | `blockCount = 1024` exactly works (boundary case). |
| TC10 | P0 | Percentile > 100 returns an error. |
| TC11 | P1 | Percentile < 0 returns an error (if client allows negative floats). |
| TC12 | P0 | Non-monotonic percentiles (e.g., [75, 50, 25]) returns an error. |
| TC13 | P2 | Duplicate percentiles (e.g., [25, 25, 75]) - verify behavior (spec unclear, implementation allows `<=`). |
| TC14 | P2 | Empty percentiles array `[]` omits `reward` field (same as omitted). |

### Response schema invariants

| ID | Priority | Description |
|----|----------|-------------|
| TC15 | P0 | `base_fee_per_gas.len() == gas_used_ratio.len() + 1` for any valid request. |
| TC16 | P0 | `gas_used_ratio.len() == blockCount` (or available blocks if fewer exist). |
| TC17 | P1 | `oldest_block == newestBlock - blockCount + 1` for normal ranges. |
| TC18 | P1 | `reward` array has dimensions `[blockCount][len(percentiles)]` when percentiles provided. |
| TC19 | P2 | `baseFeePerBlobGas` and `blobGasUsedRatio` are empty arrays (rollup-specific). |

### Value correctness

| ID | Priority | Description |
|----|----------|-------------|
| TC20 | P0 | All `gas_used_ratio` values are in range [0.0, 1.0]. |
| TC21 | P1 | For a block that includes at least one transaction, `gas_used_ratio` is > 0.0. |
| TC22 | P1 | `base_fee_per_gas` values are non-negative (u128, but verify no weird serialization). |
| TC23 | P1 | All `reward` values are zero (rollup-specific: no priority fees). |
| TC24 | P2 | `base_fee_per_gas[blockCount]` is the predicted next block fee (verify it exists and is reasonable). |

### Edge cases

| ID | Priority | Description |
|----|----------|-------------|
| TC25 | P1 | Request with blockCount=1 returns exactly 2 base fees and 1 gas ratio. |
| TC26 | P2 | Request spanning beyond earliest (e.g., blockCount=100 at block 5) gracefully returns available blocks. |
| TC27 | P3 | Request for future block number returns error or empty (verify behavior). |
| TC28 | P2 | Percentiles with fractional values (e.g., [10.5, 50.0, 90.5]) work correctly. |

### State and history scenarios (real-world usage)

These test realistic usage patterns with actual blockchain state changes.

| ID | Priority | Description |
|----|----------|-------------|
| TC29 | P0 | Empty blocks have `gas_used_ratio = 0.0`; verify multiple consecutive empty blocks. |
| TC30 | P0 | Block with transaction has `gas_used_ratio > 0.0`; deploy contract and verify ratio reflects gas consumed. |
| TC31 | P1 | Multiple transactions across blocks: send 3 txs in separate blocks, verify fee history shows distinct ratios for each. |
| TC32 | P1 | Fee history progression: query at block N, produce more blocks, query again - `oldest_block` should advance. |
| TC33 | P1 | Same block queried via `Number(N)` and via range ending at N returns consistent `gas_used_ratio` for that block. |
| TC34 | P2 | Heavy gas usage block: deploy large contract or run expensive computation, verify ratio approaches but doesn't exceed 1.0. |
| TC35 | P2 | Mixed history: sequence of [empty, tx, empty, tx, tx] blocks - verify ratios match pattern [0, >0, 0, >0, >0]. |
| TC36 | P1 | Base fee stability: across N blocks without extreme gas usage, `base_fee_per_gas` values should be relatively stable (no wild swings). |

#### Setup for state scenarios
- Use `SimpleStorage` contract for lightweight transactions
- Use `wait_for_next_blocks(1)` between transactions to ensure separate blocks
- Record block numbers when transactions are mined for targeted queries
- Use `pause_preferred_batches()` before final assertions

### Existing coverage (already tested)

For reference, these cases are already covered in `evm_fee_history.rs`:
- Basic call with Latest tag and empty percentiles
- Single block request (blockCount=1)
- Percentiles [25, 50, 75] returns zeros
- blockCount=0 error
- Specific block number
- Large blockCount capped to 1024

## Test dependencies

- Requires rollup with multiple sealed blocks (use `wait_for_next_blocks`)
- Pause sequencer before assertions to ensure deterministic state
- For TC21, ensure at least one block includes a transaction (a simple transfer is sufficient)
- **For state scenarios**: Deploy `SimpleStorage` contract, use `set_value()` for transactions
- Use `alloy_client` with `get_fee_history` method
- **For heavy gas tests**: May need contract with expensive operations (loops, storage writes)

## Implementation notes

- Run tests with: `SKIP_GUEST_BUILD=1 cargo nextest run -p sov-demo-rollup --test all_tests 'fee_history'`
- Follow existing test patterns in `evm_fee_history.rs`
- Assert invariants, not absolute values (anti-flakiness)
