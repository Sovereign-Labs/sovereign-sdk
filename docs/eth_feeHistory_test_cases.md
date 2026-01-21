# eth_feeHistory test cases

## Summary
- Returns historical gas information for a range of blocks, enabling fee estimation for EIP-1559 transactions.
- Response includes base fees, gas used ratios, and optionally reward percentiles.
- Introduced by EIP-1559; critical for wallet fee estimation UX.

## Parameters
- `blockCount`: Number of blocks to return (1-1024). Current implementation caps values > 1024 to 1024.
  - If the range would extend before genesis, the response contains fewer blocks (`oldestBlock = 0`).
- `newestBlock`: Block tag or hex number identifying the end of the range.
  - Supported tags: `latest`, `pending`, `finalized`, `safe`, `earliest`, or hex block number.
  - Spec: `latest` = most recent sealed block; `pending` = next block including mempool txs; `safe`/`finalized` may lag head.
- `rewardPercentiles`: Optional array of floats (0-100), must be monotonically increasing.
  - If provided, response includes `reward` array with priority fee percentiles.
  - If omitted or empty, `reward` field is omitted from response.

## Curl example

```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_feeHistory",
    "params": [4, "latest", [25, 50, 75]],
    "id": 1
  }'
```

Response:
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "oldestBlock": "0x5",
    "baseFeePerGas": ["0x7", "0x7", "0x7", "0x7", "0x7"],
    "gasUsedRatio": [0.0, 0.15, 0.0, 0.32],
    "reward": [
      ["0x0", "0x0", "0x0"],
      ["0x0", "0x0", "0x0"],
      ["0x0", "0x0", "0x0"],
      ["0x0", "0x0", "0x0"]
    ]
  }
}
```

## Response schema
Let `returnedBlockCount = number of blocks actually returned` (<= `blockCount`).
| Field | Type | Length | Notes |
|-------|------|--------|-------|
| `oldestBlock` | uint | - | First block number in returned range |
| `baseFeePerGas` | uint[] | returnedBlockCount + 1 | Includes predicted next block fee |
| `gasUsedRatio` | float[] | returnedBlockCount | Each value in [0.0, 1.0] |
| `reward` | uint[][] | returnedBlockCount × len(percentiles) | Only if percentiles provided |
| `baseFeePerBlobGas` | uint[] | returnedBlockCount + 1 | EIP-4844; empty in this rollup |
| `blobGasUsedRatio` | float[] | returnedBlockCount | EIP-4844; empty in this rollup |

## Implementation notes and deviations
Rollup-specific behavior (intended):
- `latest` resolves to `pending` (tooling compatibility).
- `finalized` and `safe` both resolve to the latest sealed block.
- `reward` values are zeros (preferred sequencer model, no priority fee auction).
- Blob gas fields (`baseFeePerBlobGas`, `blobGasUsedRatio`) return empty arrays (EIP-4844 not implemented).
- Percentile validation allows `<=` (monotonically non-decreasing).
- `blockCount > 1024` is capped to 1024 (no error).

Known deviations vs Ethereum L1 (bugs to track, based on code/tests):
- `baseFeePerGas` can drop to 0 after genesis (violates EIP-1559 min base fee).
- Genesis `baseFeePerGas` in `eth_feeHistory` does not match the block header.
- Empty `rewardPercentiles` returns `reward` as empty rows instead of omitting the field.
- `reward` row count is based on requested `blockCount`, not `returnedBlockCount`, when the range underflows.
- `pending` baseFeePerGas in `eth_feeHistory` does not match the pending block header.

## Test cases

Priority legend: P0 = must-have correctness, P1 = high value, P2 = medium value, P3 = optional.

### Block tag semantics

| ID | Priority | Description |
|----|----------|-------------|
| TC01 | P0 | `latest` and `pending` return identical results (all fields equal). |
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
| TC18 | P1 | `reward` array has dimensions `[returnedBlockCount][len(percentiles)]` when percentiles provided. |
| TC19 | P2 | `baseFeePerBlobGas` and `blobGasUsedRatio` are empty arrays (rollup-specific). |

### Value correctness

| ID | Priority | Description |
|----|----------|-------------|
| TC20 | P0 | All `gas_used_ratio` values are in range [0.0, 1.0]. |
| TC21 | P0 | `gas_used_ratio[i] == gas_used / gas_limit` for each returned block (match `eth_getBlockByNumber`). |
| TC22 | P0 | `base_fee_per_gas[i]` matches `eth_getBlockByNumber` baseFeePerGas for each returned block. |
| TC23 | P1 | `base_fee_per_gas[last]` matches baseFeePerGas of block `newestBlock + 1` when that block is sealed. |
| TC24 | P1 | `base_fee_per_gas[i] >= 1` for all blocks after genesis (EIP-1559 min base fee). |
| TC25 | P2 | All `reward` values are zero (rollup-specific: no priority fees). |

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
- Latest and pending return identical results (intentional semantics)
- Pending baseFeePerGas matches pending block header (KNOWN BUG)
- Reward row count matches returned block count when range underflows (KNOWN BUG)
- Fee history values match block headers for base fee and gas used ratio
- Earliest (genesis) fee history values match block headers (KNOWN BUG)
- Base fee >= 1 after genesis (KNOWN BUG)
- Predicted next base fee matches the next sealed block header

## Test dependencies

- Requires rollup with multiple sealed blocks (use `wait_for_next_blocks`)
- Pause sequencer before assertions to ensure deterministic state
- For TC21, ensure at least one block includes a transaction (a simple transfer is sufficient)
- **For state scenarios**: Deploy `SimpleStorage` contract, use `set_value()` for transactions
- Use `alloy_client` with `get_fee_history` method
- Use `eth_getBlockByNumber` for value-level cross-checks
- **For heavy gas tests**: May need contract with expensive operations (loops, storage writes)

## Implementation notes

- Run tests with: `SKIP_GUEST_BUILD=1 cargo nextest run -p sov-demo-rollup --test all_tests 'fee_history'`
- Follow existing test patterns in `evm_fee_history.rs`
- Assert absolute values when derived from canonical block headers; otherwise use invariants
