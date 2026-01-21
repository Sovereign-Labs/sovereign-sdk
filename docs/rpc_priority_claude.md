# Sovereign SDK EVM RPC Testing Priority

## Prioritization Criteria

Methods prioritized by:
1. **Wallet UX impact** - Methods called by MetaMask, Rainbow, etc.
2. **dApp dependency** - Methods used by ethers.js, viem, wagmi for contract interaction
3. **Block explorer usage** - Methods used by Etherscan-style explorers
4. **Developer tooling** - Methods used by Foundry, Hardhat for testing/deployment
5. **Block tag sensitivity** - Methods where latest/pending/finalized distinction matters

## Priority Tiers

### P0 - Critical (Breaks wallet/dApp connection)

| Method | Rationale |
|--------|-----------|
| `eth_chainId` | Network identification, wallet connection, tx signing |
| `eth_getBalance` | Wallet balance display, sufficient funds checks |
| `eth_call` | Contract reads, token balances (ERC-20 balanceOf), ENS resolution |
| `eth_sendRawTransaction` | Transaction submission |
| `eth_getTransactionReceipt` | Transaction confirmation, success/failure detection |

### P1 - High (Breaks common operations)

| Method | Rationale |
|--------|-----------|
| `eth_getBlockByNumber` | Block context, dApp state, explorer display |
| `eth_getTransactionCount` | Nonce management, transaction sequencing |
| `eth_estimateGas` | Gas estimation before sending, wallet UX |
| `eth_getLogs` | Event monitoring, indexing, dApp state sync |
| `eth_getCode` | Contract detection, proxy resolution, wallet contract checks |

### P2 - Medium (Impacts specific features)

| Method | Rationale |
|--------|-----------|
| `eth_blockNumber` | Current block height, polling |
| `eth_feeHistory` | EIP-1559 fee estimation |
| `eth_getStorageAt` | Direct storage reads, proxy slot inspection |
| `eth_getBlockByHash` | Block lookups by hash |
| `eth_getTransactionByHash` | Transaction details before confirmation |
| `eth_getBlockReceipts` | Bulk receipt fetching |

### P3 - Lower (Specialized use cases)

| Method | Rationale |
|--------|-----------|
| `debug_traceTransaction` | Debugging, transaction simulation |
| `debug_traceBlockByNumber` | Block-level debugging |
| `net_version`, `web3_*` | Compatibility checks |
| `eth_subscribe` | WebSocket real-time updates |
| `eth_getBlockTransactionCount*` | Transaction counts |

## Top 10 Endpoints to Test First

Based on P0/P1 methods with highest block tag sensitivity:

| Rank | Method | Key Test Focus |
|------|--------|----------------|
| 1 | `eth_call` | Block tag handling (latest vs pending vs finalized), state at different blocks |
| 2 | `eth_getTransactionReceipt` | Receipt schema, pending vs mined distinction |
| 3 | `eth_getBalance` | Balance at different block heights, pending state |
| 4 | `eth_getBlockByNumber` | Block schema, all tag types, pending block structure |
| 5 | `eth_getLogs` | Block range filtering, topic filtering, pagination |
| 6 | `eth_estimateGas` | Accuracy, gas margins, block context |
| 7 | `eth_getTransactionCount` | Pending nonce calculation, block tag handling |
| 8 | `eth_chainId` | Consistency, return format |
| 9 | `eth_getCode` | Code at different blocks, pending state |
| 10 | `eth_blockNumber` | Current height, monotonicity |

## Known Semantic Differences from Ethereum Spec

### 1. `latest` treated as `pending`

**Location**: `mod.rs:253-254`
```rust
BlockNumberOrTag::Latest | BlockNumberOrTag::Pending => PendingOrBlock::Pending,
```

**Impact**: Clients expecting `latest` to return the most recent sealed block will instead get a synthetic pending block. This may affect:
- Block explorers expecting sealed block data
- dApps checking for transaction inclusion in a specific block
- Clients using `latest` as a stable reference point

**Spec behavior**: `latest` = most recent sealed block, `pending` = next block being built

### 2. Gas price always returns 0

**Location**: `eth_maxPriorityFeePerGas` returns `U256::ZERO`

**Impact**: May confuse fee estimation logic in wallets/dApps

### 3. State/block overrides not implemented

**Location**: `eth_call` accepts but ignores `state_overrides` and `block_overrides` parameters

**Impact**: Advanced simulation features won't work

## Testing Strategy Recommendations

### Block Tag Coverage Matrix

For each block-tag-aware method, test:

| Scenario | latest | pending | finalized | safe | earliest | Number(n) |
|----------|--------|---------|-----------|------|----------|-----------|
| Empty state | | | | | | |
| After 1 block | | | | | | |
| After multiple blocks | | | | | | |
| With pending txs | | | | | | |
| Non-existent block | | | | | N/A | |

### EIP-1898 BlockId Coverage

For methods that accept `block_id`, also test the object form:
- `{"blockHash": <hash>, "requireCanonical": <bool>}` using a known canonical block hash.
- Validate the result matches the equivalent `Number(n)` / tag-based request.
- For unknown hashes, expect an error/null response (do not assert error shape).

Recommended methods for blockHash coverage:
- `eth_getBalance`, `eth_getStorageAt`, `eth_getTransactionCount`, `eth_getCode`
- `eth_call`, `eth_estimateGas`
- `eth_getBlockByNumber`, `eth_getBlockReceipts`, `eth_getBlockTransactionCountByNumber`
- `eth_getLogs` with `filter.blockHash`

### EIP-1898 BlockHash Test Cases

Setup (shared):
1. Record the latest sealed block before deployment as N0/H0.
2. Deploy `SimpleStorage`; record deploy block number/hash as N1/H1.
3. Call `set_value(1)` in its own block; record N2/H2.
4. Call `emitLogs()` (or equivalent log-emitting method) in its own block; record N3/H3.
5. Send a simple transfer to a second account in its own block; record N4/H4.
6. Pause sequencer before assertions.

| ID | Priority | Description |
|----|----------|-------------|
| BH01 | P0 | `eth_getBlockByNumber` with `{"blockHash": H2, "requireCanonical": true}` matches `eth_getBlockByHash(H2)` (number/hash/tx count). |
| BH02 | P0 | `eth_getBalance` for the transfer recipient at H4 matches `Number(N4)` and differs from `Number(N3)`; `requireCanonical: true/false` yield identical results. |
| BH03 | P0 | `eth_getTransactionCount` for sender at H4 matches `Number(N4)` and reflects the transfer; `requireCanonical` toggle yields identical results. |
| BH04 | P0 | `eth_getCode` for contract address at H1 is non-empty; at pre-deploy block hash (N0/H0) it is empty. |
| BH05 | P1 | `eth_getStorageAt` for slot 0 at H2 returns value `1`; at H1 returns `0` (or initial). |
| BH06 | P1 | `eth_call` for `get()` at H2 returns `1`; at H1 returns `0`; result matches the same calls with `Number(N2)`/`Number(N1)`. |
| BH07 | P1 | `eth_estimateGas` for `set_value(2)` at H2 matches estimate at `Number(N2)` and is non-zero. |
| BH08 | P1 | `eth_getBlockReceipts` with blockHash H3 returns the same receipt count as `Number(N3)` and includes the log tx receipt. |
| BH09 | P1 | `eth_getBlockTransactionCountByNumber` with blockHash H3 equals tx count from `eth_getBlockByHash(H3)`. |
| BH10 | P1 | `eth_getLogs` with `filter.blockHash = H3` returns only logs from H3; compare to range filter `[N3, N3]`. |
| BH11 | P2 | Unknown blockHash for blockId methods (e.g., `eth_getBalance`, `eth_getBlockByNumber`) returns an error; do not assert error shape. |

### Invariants to Assert

1. **Monotonicity**: `eth_blockNumber` never decreases within a session
2. **Consistency**: Data returned for a sealed block never changes
3. **Relationship**: `pending.number == latest_sealed.number + 1`
4. **Schema**: Response structure matches Ethereum JSON-RPC spec
5. **State consistency**: Balance/nonce/code changes are atomic per block

### Anti-Flakiness

- Use explicit block production controls (pause/resume)
- Assert invariants, not absolute values
- No wall-clock timing dependencies
- Deterministic transaction ordering
