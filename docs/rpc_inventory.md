# RPC Inventory (Sovereign SDK EVM)

Assumptions (explicit):
- This inventory is derived from current RPC registration and handlers in `sov-evm` and `sov-ethereum`.
- "Implemented" means a handler is registered and returns data; "stubbed" means it returns "Method <name> not supported".
- Local-only methods are compiled behind the `local` feature flag.

## Implementation locations

- `crates/module-system/module-implementations/sov-evm/src/rpc/handlers.rs` (EVM RPC handlers)
- `crates/module-system/module-implementations/sov-evm/src/rpc/mod.rs` (block/tag resolution, pending state)
- `crates/module-system/module-implementations/sov-evm/src/rpc/fee_history.rs` (fee history)
- `crates/module-system/module-implementations/sov-evm/src/rpc/trace.rs` (debug tracing)
- `crates/full-node/sov-ethereum/src/lib.rs` (RPC registration + stubs)
- `crates/full-node/sov-ethereum/src/handlers/mod.rs` (tx submission + local signer)
- `crates/full-node/sov-ethereum/src/handlers/get_logs.rs` (log limits + cursor)
- `crates/full-node/sov-ethereum/src/handlers/subscribe.rs` (subscriptions)
- `crates/full-node/sov-ethereum/src/handlers/subscribe/params.rs` (subscription validation)

## Implemented: EVM module (`sov-evm`)

Source: `crates/module-system/module-implementations/sov-evm/src/rpc/handlers.rs`.

| Method | Status | Notes |
| --- | --- | --- |
| `net_version` | implemented | Returns chain id as string. |
| `eth_chainId` | implemented | Returns chain id. |
| `eth_getBlockByHash` | implemented | Block lookup by hash. |
| `eth_getBlockByNumber` | implemented | Supports block tags and EIP-1898 `blockHash` via `BlockId`. |
| `eth_getBalance` | implemented | Block-tagged state read. |
| `eth_getStorageAt` | implemented | Block-tagged state read. |
| `eth_getTransactionCount` | implemented | Returns uniqueness module nonce for address. |
| `eth_getCode` | implemented | Block-tagged state read. |
| `eth_feeHistory` | implemented | Reward percentiles accepted (zeroed); block_count capped at 1024. |
| `eth_getTransactionByHash` | implemented | Looks up sealed or pending txs in accessory state. |
| `eth_getBlockReceipts` | implemented | Receipts by block. |
| `eth_getTransactionReceipt` | implemented | Receipt by tx hash. |
| `eth_call` | implemented | Block-tagged call; `state_overrides` and `block_overrides` ignored. |
| `eth_blockNumber` | implemented | Returns latest sealed block number. |
| `eth_estimateGas` | implemented | Block-tagged estimation with safety margin. |
| `debug_traceBlockByNumber` | implemented | Geth tracing; only `callTracer` supported. |
| `debug_traceTransaction` | implemented | Geth tracing; only `callTracer` supported. |
| `web3_clientVersion` | implemented | `sov-evm/<version>`. |
| `web3_sha3` | implemented | Keccak-256. |
| `net_listening` | implemented | Always `true`. |
| `eth_maxPriorityFeePerGas` | implemented | Always `0`. |
| `eth_getBlockTransactionCountByNumber` | implemented | Tx count in block. |
| `eth_getBlockTransactionCountByHash` | implemented | Tx count in block. |

## Implemented: full node (`sov-ethereum`)

Source: `crates/full-node/sov-ethereum/src/lib.rs` and `crates/full-node/sov-ethereum/src/handlers`.

| Method | Status | Notes |
| --- | --- | --- |
| `eth_gasPrice` | implemented | Always `0`. |
| `eth_sendRawTransaction` | implemented | RLP tx submission. |
| `eth_sendRawTransactionSync` | implemented (custom) | Waits for receipt or timeout (max 2s). |
| `realtime_sendRawTransaction` | implemented (custom) | Waits for receipt. |
| `eth_getLogs` | implemented | Enforces response-size cap; suggests cursor method. |
| `eth_getLogsWithCursor` | implemented (custom) | Non-standard pagination. |
| `eth_subscribe` / `eth_unsubscribe` | implemented | Supports `logs` and `newHeads` only; log block-range options rejected except `pending..pending`; `newHeads` takes no params. |
| `eth_accounts` | implemented (local-only) | Requires `local` feature. |
| `eth_sendTransaction` | implemented (local-only) | Requires `local` feature. |

## Explicitly stubbed (method not supported)

Source: `crates/full-node/sov-ethereum/src/lib.rs`.

- `eth_protocolVersion`
- `eth_coinbase`
- `eth_mining`
- `eth_hashrate`
- `eth_getTransactionByBlockHashAndIndex`
- `eth_getTransactionByBlockNumberAndIndex`
- `eth_getUncleCountByBlockHash`
- `eth_getUncleCountByBlockNumber`
- `eth_getUncleByBlockHashAndIndex`
- `eth_getUncleByBlockNumberAndIndex`
- `eth_newFilter`
- `eth_newBlockFilter`
- `eth_newPendingTransactionFilter`
- `eth_uninstallFilter`
- `eth_getFilterChanges`
- `eth_getFilterLogs`
- `eth_sign`
- `eth_signTransaction`
- `eth_signTypedData`
- `eth_signTypedData_v1`
- `eth_signTypedData_v3`
- `eth_signTypedData_v4`
- `eth_getProof`
- `eth_createAccessList`
- `eth_syncing`
- `net_peerCount`
- `trace_block`
- `trace_call`
- `trace_filter`
- `trace_get`
- `trace_rawTransaction`
- `trace_replayBlockTransactions`
- `trace_replayTransaction`
- `trace_transaction`
- `txpool_content`
- `txpool_contentFrom`
- `txpool_inspect`
- `txpool_status`

## Behavior notes (block tags and overrides)

Source: `crates/module-system/module-implementations/sov-evm/src/rpc/mod.rs`.

- `latest` and `pending` are treated as the same tag when resolving blocks (both map to the pending head).
- `finalized` and `safe` map to the latest finalized block (may lag head when `finalization_blocks > 0`); `earliest` maps to the first stored block.
- State reads for `latest`/`pending` resolve via the current state accessor (no pending block replay).
- Methods accepting `BlockId` also accept EIP-1898 `{"blockHash": ..., "requireCanonical": ...}`; `requireCanonical` is ignored.

## Error codes (custom/non-standard)

Sources: `crates/full-node/sov-ethereum/src/lib.rs`, `crates/utils/sov-rpc-eth-types/src/eth_api_error.rs`.

- `-32004` method not supported (stubbed RPCs).
- `-32005` limit exceeded (`eth_getLogs` response cap).
- `-32003` transaction rejected (`eth_sendRawTransaction*`, `eth_sendTransaction`).
- `-32001` resource not found (unknown block/receipt).
- `4444` pruned history unavailable (EIP-4444).
- `4` timeout (`eth_sendRawTransactionSync`).
- `-32602` invalid params (generic input validation).
