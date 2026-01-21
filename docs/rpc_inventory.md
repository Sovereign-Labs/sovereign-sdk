# RPC Inventory (Sovereign SDK EVM)

Assumptions (explicit):
- This inventory is derived from current RPC registration and handlers in `sov-evm` and `sov-ethereum`.
- "Implemented" means a handler is registered and returns data; "stubbed" means it returns "Method not supported".
- Local-only methods are compiled behind the `local` feature flag.

## Implemented: EVM module (`sov-evm`)

Source: `crates/module-system/module-implementations/sov-evm/src/rpc/handlers.rs`.

| Method | Status | Notes |
| --- | --- | --- |
| `net_version` | implemented | Returns chain id as string. |
| `eth_chainId` | implemented | Returns chain id. |
| `eth_getBlockByHash` | implemented | Block lookup by hash. |
| `eth_getBlockByNumber` | implemented | Supports block tags and EIP-1898 `blockHash` object via `BlockId`. |
| `eth_getBalance` | implemented | Block-tagged state read. |
| `eth_getStorageAt` | implemented | Block-tagged state read. |
| `eth_getTransactionCount` | implemented | Counts pending txs for `latest`/`pending` tags. |
| `eth_getCode` | implemented | Block-tagged state read. |
| `eth_feeHistory` | implemented | Base fee history; reward percentiles supported. |
| `eth_getTransactionByHash` | implemented | Includes pending transaction lookup. |
| `eth_getBlockReceipts` | implemented | Receipts by block. |
| `eth_getTransactionReceipt` | implemented | Receipt by tx hash. |
| `eth_call` | implemented | Block-tagged call. |
| `eth_blockNumber` | implemented | Returns current head block number. |
| `eth_estimateGas` | implemented | Block-tagged estimation. |
| `debug_traceBlockByNumber` | implemented | Geth tracing options. |
| `debug_traceTransaction` | implemented | Geth tracing options. |
| `web3_clientVersion` | implemented | `sov-evm/<version>`. |
| `web3_sha3` | implemented | Keccak-256. |
| `net_listening` | implemented | Always `true`. |
| `eth_maxPriorityFeePerGas` | implemented | Always `0`. |
| `eth_getBlockTransactionCountByNumber` | implemented | Tx count in block. |
| `eth_getBlockTransactionCountByHash` | implemented | Tx count in block. |
| `net_peerCount` | stubbed | Returns "Method not supported". |
| `eth_syncing` | stubbed | Returns "Method not supported". |

## Implemented: full node (`sov-ethereum`)

Source: `crates/full-node/sov-ethereum/src/lib.rs` and `crates/full-node/sov-ethereum/src/handlers`.

| Method | Status | Notes |
| --- | --- | --- |
| `eth_gasPrice` | implemented | Always `0`. |
| `eth_sendRawTransaction` | implemented | RLP tx submission. |
| `eth_sendRawTransactionSync` | implemented (custom) | Non-standard; waits for receipt or timeout. |
| `realtime_sendRawTransaction` | implemented (custom) | Non-standard; waits for receipt. |
| `eth_getLogs` | implemented | Enforces response-size cap; suggests cursor method. |
| `eth_getLogsWithCursor` | implemented (custom) | Non-standard pagination. |
| `eth_subscribe` / `eth_unsubscribe` | implemented | Supports `logs` and `newHeads` only; log block-range options rejected except `pending..pending`. |
| `eth_accounts` | implemented (local-only) | Requires `local` feature. |
| `eth_sendTransaction` | implemented (local-only) | Requires `local` feature. |

## Explicitly stubbed (method not supported)

Source: `crates/full-node/sov-ethereum/src/lib.rs`.

- `eth_blobBaseFee`
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

## Notes relevant to block-tag semantics (for test planning)

Source: `crates/module-system/module-implementations/sov-evm/src/rpc/mod.rs`.

- `latest` and `pending` are treated as the same tag when resolving blocks and state (intended to avoid Foundry issues).
- `finalized` and `safe` map to the current head block.
