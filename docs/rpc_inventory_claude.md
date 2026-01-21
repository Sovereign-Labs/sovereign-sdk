# Sovereign SDK EVM RPC Implementation Inventory

## Overview

This document catalogs all JSON-RPC methods implemented in the Sovereign SDK EVM module, their parameters, block tag handling behavior, and implementation locations.

## Implementation Files

| File | Purpose |
|------|---------|
| `crates/module-system/module-implementations/sov-evm/src/rpc/handlers.rs` | RPC method handlers (25 methods) |
| `crates/module-system/module-implementations/sov-evm/src/rpc/mod.rs` | Block resolution, state access, pending block |
| `crates/module-system/module-implementations/sov-evm/src/rpc/fee_history.rs` | Fee history calculation |
| `crates/module-system/module-implementations/sov-evm/src/rpc/trace.rs` | Debug tracing |
| `crates/full-node/sov-ethereum/src/lib.rs` | RPC registration, tx submission |
| `crates/full-node/sov-ethereum/src/handlers/get_logs.rs` | Log filtering with pagination |
| `crates/full-node/sov-ethereum/src/handlers/subscribe.rs` | WebSocket subscriptions |

## Implemented Methods

### ETH Namespace (18 methods)

| Method | Parameters | Block Tag Support | Location (handlers.rs) |
|--------|------------|-------------------|------------------------|
| `eth_chainId` | none | N/A | L56-65 |
| `eth_blockNumber` | none | N/A | L312-317 |
| `eth_getBlockByHash` | block_hash, details | N/A (hash lookup) | L68-94 |
| `eth_getBlockByNumber` | block_id, details | Yes | L97-111 |
| `eth_getBalance` | address, block_id | Yes | L114-137 |
| `eth_getStorageAt` | address, index, block_id | Yes | L140-158 |
| `eth_getTransactionCount` | address, block_id | Yes (pending-aware) | L161-196 |
| `eth_getCode` | address, block_id | Yes | L199-209 |
| `eth_call` | request, block_id, state_overrides*, block_overrides* | Yes | L293-309 |
| `eth_estimateGas` | request, block_id | Yes | L321-381 |
| `eth_feeHistory` | block_count, newest_block, reward_percentiles | Yes | L216-239 |
| `eth_getTransactionByHash` | hash | N/A (includes pending) | L242-258 |
| `eth_getTransactionReceipt` | hash | N/A | L277-289 |
| `eth_getBlockReceipts` | block_id | Yes | L261-274 |
| `eth_getBlockTransactionCountByHash` | block_hash | N/A | L494-519 |
| `eth_getBlockTransactionCountByNumber` | block_id | Yes | L477-490 |
| `eth_maxPriorityFeePerGas` | none | N/A (returns 0) | L462-473 |
| `eth_syncing` | none | N/A (returns error) | L454-458 |

*Note: state_overrides and block_overrides are accepted but not implemented (parameters ignored)

### NET Namespace (3 methods)

| Method | Parameters | Behavior | Location |
|--------|------------|----------|----------|
| `net_version` | none | Returns chain ID as string | L46-53 |
| `net_listening` | none | Always returns true | L435-440 |
| `net_peerCount` | none | Returns "method not supported" error | L444-448 |

### WEB3 Namespace (2 methods)

| Method | Parameters | Location |
|--------|------------|----------|
| `web3_clientVersion` | none | L414-421 |
| `web3_sha3` | data | L425-429 |

### DEBUG Namespace (2 methods)

| Method | Parameters | Location |
|--------|------------|----------|
| `debug_traceBlockByNumber` | block, opts | L384-396 |
| `debug_traceTransaction` | tx_hash, opts | L399-408 |

### Full Node Methods (sov-ethereum)

| Method | Parameters | Location |
|--------|------------|----------|
| `eth_gasPrice` | none | lib.rs |
| `eth_sendRawTransaction` | raw_tx | handlers/mod.rs |
| `eth_sendRawTransactionSync` | raw_tx, timeout | handlers/mod.rs |
| `realtime_sendRawTransaction` | raw_tx | handlers/mod.rs (custom) |
| `eth_getLogs` | filter | handlers/get_logs.rs |
| `eth_getLogsWithCursor` | filter, cursor | handlers/get_logs.rs (custom) |
| `eth_subscribe` | subscription_type, filter | handlers/subscribe.rs |
| `eth_unsubscribe` | subscription_id | handlers/subscribe.rs (via rpc registration) |
| `eth_accounts` | none | handlers/mod.rs (dev only) |
| `eth_sendTransaction` | tx | handlers/mod.rs (dev only) |

## Block Tag Handling

Implementation in `mod.rs:245-297`:

| Tag | Resolution | Notes |
|-----|-----------|-------|
| `latest` | Pending block | **Differs from spec**: treated same as "pending" |
| `pending` | Pending block (latest + 1) | Synthetic block with pending txs |
| `finalized` | Latest sealed block | |
| `safe` | Latest sealed block | |
| `earliest` | First available block | |
| `Number(n)` | Specific sealed block | |

### Key Code Paths

**Block tag to block number** (`block_tag_to_pending_or_block`, L245-260):
```rust
match block {
    BlockNumberOrTag::Earliest => PendingOrBlock::Number(*block_numbers.start()),
    BlockNumberOrTag::Latest | BlockNumberOrTag::Pending => PendingOrBlock::Pending,
    BlockNumberOrTag::Finalized | BlockNumberOrTag::Safe => PendingOrBlock::Number(*block_numbers.end()),
    BlockNumberOrTag::Number(number) => PendingOrBlock::Number(number),
}
```

**Pending nonce handling** (`get_transaction_count`, L161-196):
- When block_id is latest/pending, scans pending transactions for highest nonce
- Returns max(state_nonce, pending_nonce + 1)

**Pending transaction lookup** (`get_transaction_by_hash`, L242-258):
- First checks sealed transactions, then falls back to pending pool

### BlockId / EIP-1898 Handling

Methods that accept `block_id` (`BlockId`) also accept EIP-1898 object form:
- `{"blockHash": <hash>, "requireCanonical": <bool>}` is supported.
- `requireCanonical` is currently ignored; only `blockHash` is used to resolve a block number.
- Unknown hashes return `HeaderNotFound` via `block_hash_to_number` lookup.

## Not Implemented Methods

| Category | Methods |
|----------|---------|
| Uncle methods | `eth_getUncleCountByBlockHash/Number`, `eth_getUncleByBlock*AndIndex` |
| Transaction by index | `eth_getTransactionByBlockHashAndIndex`, `eth_getTransactionByBlockNumberAndIndex` |
| Filter lifecycle | `eth_newFilter`, `eth_newBlockFilter`, `eth_newPendingTransactionFilter`, `eth_uninstallFilter`, `eth_getFilterChanges`, `eth_getFilterLogs` |
| Signing | `eth_sign`, `eth_signTransaction`, `eth_signTypedData*` |
| Proofs | `eth_getProof` |
| Access lists | `eth_createAccessList` |
| Mining | `eth_coinbase`, `eth_mining`, `eth_hashrate`, `eth_protocolVersion` |
| Tracing (OpenEthereum/Parity) | `trace_block`, `trace_call`, `trace_filter`, `trace_get`, `trace_rawTransaction`, `trace_replayBlockTransactions`, `trace_replayTransaction`, `trace_transaction` |
| Txpool | `txpool_content`, `txpool_contentFrom`, `txpool_inspect`, `txpool_status` |

## Error Codes

| Code | Meaning | Usage |
|------|---------|-------|
| -32001 | Resource not found | Block pruned, receipt not found |
| -32003 | Transaction rejected | Invalid auth, invalid nonce |
| -32004 | Method not supported | `eth_syncing`, `net_peerCount` |
| -32005 | Limit exceeded | Response size, max logs |
| -32600 | Invalid request | Invalid parameters |
| 4 | Timeout | `eth_sendRawTransactionSync` timeout |
